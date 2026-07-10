// Tauri commands exposing diarization status and runtime acceleration to the
// frontend Settings panel, plus `rediarize_meeting` for re-running diarization
// on past meetings (the live post-recording flow is in
// `recording_manager::run_post_recording_diarization`).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::anyhow;
use log::{info, warn};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

use crate::audio::decoder::decode_audio_file;
use crate::audio::recording_saver::TranscriptSegment;
use crate::audio::retranscription::find_audio_file;
use crate::database::repositories::meeting::MeetingsRepository;
use crate::database::repositories::setting::SettingsRepository;
use crate::database::repositories::transcript::TranscriptsRepository;
use crate::diarization::aligner::{assign_speakers, unique_speakers};
use crate::diarization::engine::{detect_gpu, mask_ranges, DiarizationEngine};
use crate::diarization::models::{ensure_models, status, ModelsStatus};
use crate::diarization::remote::{encode_wav_pcm16_mono, PyannoteClient};
use crate::state::AppState;

/// Cache of the most-recently-decoded meeting audio as 16 kHz mono f32 samples.
/// The "name speakers" step previews several short clips per meeting; decoding
/// the whole file once and slicing from memory keeps every play after the first
/// instant. Holds a single meeting at a time; cleared via
/// `clear_audio_clip_cache` when the naming dialog closes.
static CLIP_AUDIO_CACHE: Mutex<Option<(String, Arc<Vec<f32>>)>> = Mutex::new(None);

/// Which diarization backend to run. Local (sherpa-onnx, on-device) is the
/// default; LocalPro runs pyannote community-1 in a Python sidecar (fully
/// local, needs a Hugging Face token for the gated model); PyannoteCloud
/// uploads mic-masked audio to the pyannoteAI API (model "precision-2") and
/// requires an API key in transcript settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiarProvider {
    Local,
    LocalPro,
    PyannoteCloud,
}

fn parse_provider(provider: Option<&str>) -> Result<DiarProvider, String> {
    match provider.unwrap_or("local") {
        "local" => Ok(DiarProvider::Local),
        "local-pro" => Ok(DiarProvider::LocalPro),
        "pyannote" => Ok(DiarProvider::PyannoteCloud),
        other => Err(format!("Unknown diarization provider: {other}")),
    }
}

/// Decode the meeting audio, silence the mic ("You") ranges, and encode as a
/// mono 16 kHz PCM16 WAV — the shared input for both non-sherpa providers.
async fn prepare_masked_wav(
    audio_path: &std::path::Path,
    mic_ranges: &[(f64, f64)],
) -> anyhow::Result<Vec<u8>> {
    let audio = audio_path.to_path_buf();
    let ranges = mic_ranges.to_vec();
    tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
        let decoded = decode_audio_file(&audio)
            .map_err(|e| anyhow!("Failed to decode audio for diarization: {e}"))?;
        let mut samples = decoded.to_whisper_format();
        mask_ranges(&mut samples, &ranges, 16_000);
        Ok(encode_wav_pcm16_mono(&samples, 16_000))
    })
    .await
    .map_err(|e| anyhow!("Audio preparation task panicked: {e}"))?
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeAcceleration {
    pub whisper: &'static str,
    pub diarization: &'static str,
}

fn whisper_provider() -> &'static str {
    if cfg!(target_os = "macos") {
        if cfg!(feature = "metal") {
            "metal"
        } else if cfg!(feature = "coreml") {
            "coreml"
        } else {
            "cpu"
        }
    } else if cfg!(feature = "cuda") {
        "cuda"
    } else if cfg!(feature = "vulkan") {
        "vulkan"
    } else if cfg!(feature = "hipblas") {
        "rocm"
    } else {
        "cpu"
    }
}

fn diarization_provider() -> &'static str {
    if detect_gpu() {
        "cuda"
    } else {
        "cpu"
    }
}

#[tauri::command]
pub async fn diarization_models_status<R: Runtime>(
    app: AppHandle<R>,
) -> Result<ModelsStatus, String> {
    status(&app).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_runtime_acceleration() -> RuntimeAcceleration {
    RuntimeAcceleration {
        whisper: whisper_provider(),
        diarization: diarization_provider(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RediarizationResult {
    pub meeting_id: String,
    pub speakers: usize,
    pub segments_updated: usize,
    /// Distinct diarization clusters that actually landed on transcript rows.
    /// When the user requested N speakers and this comes back lower, some
    /// cluster's speech never cleared the alignment threshold — the UI warns
    /// and points at the leftover "Others" bucket.
    pub matched_speakers: usize,
    /// Rows still carrying the generic "system" tag after alignment
    /// (unattributed speech, rendered as "Others").
    pub leftover_segments: usize,
}

/// Re-run speaker diarization on a previously saved meeting.
///
/// The DB row order in `transcripts` is canonical for past meetings
/// (`transcripts.json` may not exist for older recordings). We load the
/// existing rows, build adapter `TranscriptSegment`s for the aligner, run the
/// pipeline against the meeting's audio file, then UPDATE the `speaker` column
/// per row. Progress is streamed to the dialog via `diarization-progress`
/// events with a `meeting_id` discriminator so the live-recording toast in
/// `TranscriptContext` knows to ignore them.
#[tauri::command]
pub async fn rediarize_meeting<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    num_speakers: Option<u32>,
    provider: Option<String>,
) -> Result<RediarizationResult, String> {
    let provider = parse_provider(provider.as_deref())?;
    let pool = state.db_manager.pool().clone();
    let result = run_rediarization(app.clone(), &pool, &meeting_id, num_speakers, provider).await;

    match result {
        Ok(r) => {
            let _ = app.emit(
                "diarization-progress",
                serde_json::json!({
                    "status": "done",
                    "meeting_id": meeting_id,
                    "speakers": r.speakers,
                    "matched_speakers": r.matched_speakers,
                    "leftover_segments": r.leftover_segments,
                    "segments_updated": r.segments_updated,
                }),
            );
            Ok(r)
        }
        Err(e) => {
            let reason = e.to_string();
            warn!("Re-diarization for {} failed: {}", meeting_id, reason);
            let _ = app.emit(
                "diarization-progress",
                serde_json::json!({
                    "status": "error",
                    "meeting_id": meeting_id,
                    "reason": reason,
                }),
            );
            Err(reason)
        }
    }
}

async fn run_rediarization<R: Runtime>(
    app: AppHandle<R>,
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
    num_speakers: Option<u32>,
    provider: DiarProvider,
) -> anyhow::Result<RediarizationResult> {
    let _ = app.emit(
        "diarization-progress",
        serde_json::json!({"status": "starting", "meeting_id": meeting_id}),
    );

    let meeting = MeetingsRepository::get_meeting_metadata(pool, meeting_id)
        .await
        .map_err(|e| anyhow!("Failed to load meeting: {e}"))?
        .ok_or_else(|| anyhow!("Meeting not found: {meeting_id}"))?;

    let folder = meeting
        .folder_path
        .as_deref()
        .ok_or_else(|| anyhow!("Meeting has no folder_path; cannot locate audio"))?;
    let folder_pb = PathBuf::from(folder);
    if !folder_pb.exists() {
        return Err(anyhow!("Meeting folder does not exist: {}", folder));
    }
    let audio_path = find_audio_file(&folder_pb)
        .map_err(|e| anyhow!("Could not locate audio in {}: {e}", folder))?;
    info!(
        "Re-diarization for {}: audio = {}",
        meeting_id,
        audio_path.display()
    );

    // Load the existing transcripts up front: we need the "mic"-tagged ranges
    // to mask the local user out of clustering, plus the rows themselves to
    // align against afterwards. `get_meeting_transcripts_paginated` with a
    // large limit avoids pulling the meeting body twice.
    let (rows, _total) =
        MeetingsRepository::get_meeting_transcripts_paginated(pool, meeting_id, 100_000, 0)
            .await
            .map_err(|e| anyhow!("Failed to load transcripts: {e}"))?;
    info!(
        "Re-diarization for {}: {} transcript rows loaded",
        meeting_id,
        rows.len()
    );

    // Silence local mic regions before clustering so we only diarize "them".
    // The mic tag survives diarization (the aligner never overwrites it), so it
    // is still present even when re-diarizing an already-diarized meeting.
    let mic_ranges: Vec<(f64, f64)> = rows
        .iter()
        .filter(|r| r.speaker.as_deref() == Some("mic"))
        .filter_map(|r| Some((r.audio_start_time?, r.audio_end_time?)))
        .collect();

    let _ = app.emit(
        "diarization-progress",
        serde_json::json!({"status": "running", "meeting_id": meeting_id}),
    );

    let diar = match provider {
        DiarProvider::Local => {
            // Local models are only needed (and downloaded) on this path.
            let paths = ensure_models(&app)
                .await
                .map_err(|e| anyhow!("Diarization models unavailable: {e}"))?;
            let audio_for_blocking = audio_path.clone();
            let forced_clusters = num_speakers.map(|n| n as i32);
            tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
                let engine = DiarizationEngine::new(&paths, forced_clusters)?;
                engine.run_on_file_excluding(&audio_for_blocking, &mic_ranges)
            })
            .await
            .map_err(|e| anyhow!("Diarization task panicked: {e}"))??
        }
        DiarProvider::LocalPro => {
            let hf_token = SettingsRepository::get_transcript_api_key(pool, "huggingface")
                .await
                .map_err(|e| anyhow!("Failed to read Hugging Face token: {e}"))?
                .filter(|k| !k.trim().is_empty())
                .ok_or_else(|| {
                    anyhow!("Hugging Face token not configured. Add it in Settings → Transcript.")
                })?;

            let wav_bytes = prepare_masked_wav(&audio_path, &mic_ranges).await?;
            let wav_path =
                std::env::temp_dir().join(format!("meetily-diar-{}.wav", uuid::Uuid::new_v4()));
            tokio::fs::write(&wav_path, &wav_bytes)
                .await
                .map_err(|e| anyhow!("Failed to write temp WAV: {e}"))?;

            let progress_app = app.clone();
            let progress_meeting_id = meeting_id.to_string();
            let result = crate::diarization::localpro::run_local_pro(
                &app,
                &wav_path,
                num_speakers,
                &hf_token,
                &move |stage| {
                    let _ = progress_app.emit(
                        "diarization-progress",
                        serde_json::json!({"status": stage, "meeting_id": progress_meeting_id}),
                    );
                },
            )
            .await;

            if let Err(e) = tokio::fs::remove_file(&wav_path).await {
                warn!("Failed to remove temp WAV {}: {e}", wav_path.display());
            }
            result?
        }
        DiarProvider::PyannoteCloud => {
            let api_key = SettingsRepository::get_transcript_api_key(pool, "pyannote")
                .await
                .map_err(|e| anyhow!("Failed to read pyannoteAI API key: {e}"))?
                .filter(|k| !k.trim().is_empty())
                .ok_or_else(|| {
                    anyhow!("pyannoteAI API key not configured. Add it in Settings → Transcript.")
                })?;

            // Masking mirrors the local path: the clusterer (cloud or
            // on-device) only ever sees "them" audio.
            let wav_bytes = prepare_masked_wav(&audio_path, &mic_ranges).await?;

            let object_key = format!("meetily/{}-{}.wav", meeting_id, uuid::Uuid::new_v4());
            let client = PyannoteClient::new(api_key)?;
            let progress_app = app.clone();
            let progress_meeting_id = meeting_id.to_string();
            client
                .diarize(wav_bytes, &object_key, num_speakers, &move |stage| {
                    let _ = progress_app.emit(
                        "diarization-progress",
                        serde_json::json!({"status": stage, "meeting_id": progress_meeting_id}),
                    );
                })
                .await?
        }
    };

    let _ = app.emit(
        "diarization-progress",
        serde_json::json!({"status": "aligning", "meeting_id": meeting_id, "segments": diar.len()}),
    );

    // Build aligner shims. Only audio_start_time / audio_end_time / speaker
    // matter to assign_speakers — the rest get placeholder defaults.
    let mut shims: Vec<TranscriptSegment> = Vec::with_capacity(rows.len());
    let mut row_ids: Vec<String> = Vec::with_capacity(rows.len());
    for row in &rows {
        let (Some(start), Some(end)) = (row.audio_start_time, row.audio_end_time) else {
            // Skip rows without timing info; we can't align them.
            continue;
        };
        shims.push(TranscriptSegment {
            id: row.id.clone(),
            text: String::new(),
            audio_start_time: start,
            audio_end_time: end,
            duration: row.duration.unwrap_or(end - start),
            display_time: String::new(),
            confidence: 1.0,
            sequence_id: 0,
            speaker: row.speaker.clone().unwrap_or_default(),
        });
        row_ids.push(row.id.clone());
    }

    if shims.is_empty() {
        return Err(anyhow!(
            "Meeting has no transcript rows with timing info to align"
        ));
    }

    let matched_speakers = assign_speakers(&mut shims, &diar);
    let leftover_segments = shims.iter().filter(|s| s.speaker == "system").count();

    // Build the (id, Option<String>) update list. Empty speaker → write NULL.
    let updates: Vec<(String, Option<String>)> = shims
        .iter()
        .map(|s| {
            (
                s.id.clone(),
                if s.speaker.is_empty() {
                    None
                } else {
                    Some(s.speaker.clone())
                },
            )
        })
        .collect();

    let segments_updated = TranscriptsRepository::update_speakers(pool, &updates)
        .await
        .map_err(|e| anyhow!("Failed to persist diarized speakers: {e}"))?;

    let speakers = unique_speakers(&shims).len();

    // Mirror the live flow: emit the full updated segment list so any open
    // page that listens to `transcript-rediarized` refreshes in place.
    let _ = app.emit("transcript-rediarized", &shims);

    Ok(RediarizationResult {
        meeting_id: meeting_id.to_string(),
        speakers,
        segments_updated,
        matched_speakers,
        leftover_segments,
    })
}

/// One playable preview range for a speaker in the naming step.
#[derive(Debug, Clone, Serialize)]
pub struct SpeakerClip {
    /// Seconds from the recording start.
    pub start: f64,
    pub end: f64,
    /// Transcript text of the underlying segment (a hint for the user).
    pub text: String,
}

/// One diarized speaker plus representative audio clips to preview in the
/// "name speakers" step. Clips are ranked by *cleanness* — no time-overlap
/// with other speakers' rows, comfortable length — not by raw length: the
/// longest segment is exactly the one most likely to contain crosstalk.
#[derive(Debug, Clone, Serialize)]
pub struct SpeakerSample {
    /// Raw speaker tag as stored (e.g. "speaker_1").
    pub speaker: String,
    /// Preview ranges, cleanest first (up to `MAX_CLIPS_PER_SPEAKER`).
    pub clips: Vec<SpeakerClip>,
    pub segment_count: usize,
    pub total_seconds: f64,
}

/// How desirable a transcript row is as a voice-identification clip.
fn clip_score(len: f64, crosstalk_secs: f64, same_speaker_neighbors: u32) -> f64 {
    // Purity dominates: a clip someone else talks over is a bad sample no
    // matter how long it is.
    let purity = 1.0 - (crosstalk_secs / len).min(1.0);
    // Comfortable listening length: ramp up to ~4s, flat to 12s, then decay
    // (very long rows tend to hide crosstalk the transcript didn't mark).
    let length = if len < 4.0 {
        len / 4.0
    } else if len <= 12.0 {
        1.0
    } else {
        12.0 / len
    };
    // Rows flanked by the same speaker sit safely inside one person's turn.
    purity * 3.0 + length + 0.15 * f64::from(same_speaker_neighbors)
}

/// List the distinct non-"mic" speakers in a meeting, each with a few clean
/// audio snippet ranges for playback in the naming step. The local user's mic
/// stream ("You") is excluded — only the diarized remote voices need naming.
/// Sorted by total speaking time descending, except the leftover "system"
/// bucket (unattributed speech, shown as "Others") which always sorts last.
#[tauri::command]
pub async fn list_speaker_samples(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<Vec<SpeakerSample>, String> {
    /// Cap the preview clip so a very long segment doesn't play for minutes.
    const MAX_CLIP_SECONDS: f64 = 15.0;
    /// How many alternative clips to offer per speaker in the naming UI.
    const MAX_CLIPS_PER_SPEAKER: usize = 5;

    let pool = state.db_manager.pool().clone();
    let (rows, _total) =
        MeetingsRepository::get_meeting_transcripts_paginated(&pool, &meeting_id, 100_000, 0)
            .await
            .map_err(|e| format!("Failed to load transcripts: {e}"))?;
    info!(
        "list_speaker_samples for {}: scanning {} transcript rows",
        meeting_id,
        rows.len()
    );

    // Every timed row takes part in the crosstalk check — including "mic":
    // the preview clips are cut from the mixed recording, so the user's own
    // voice over a segment makes it a bad sample too.
    struct Timed<'a> {
        speaker: &'a str,
        start: f64,
        end: f64,
        text: &'a str,
    }
    let mut timed: Vec<Timed> = rows
        .iter()
        .filter_map(|row| {
            let speaker = row.speaker.as_deref().filter(|s| !s.trim().is_empty())?;
            let (start, end) = (row.audio_start_time?, row.audio_end_time?);
            (end > start).then_some(Timed {
                speaker,
                start,
                end,
                text: row.transcript.as_str(),
            })
        })
        .collect();
    timed.sort_by(|a, b| {
        a.start
            .partial_cmp(&b.start)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Seconds of each row that rows of a *different* speaker overlap. With
    // rows start-sorted, any overlapping pair (i < j) has timed[j].start <
    // timed[i].end, so the inner scan stops at the first non-overlapping row.
    let mut crosstalk = vec![0.0f64; timed.len()];
    for i in 0..timed.len() {
        for j in (i + 1)..timed.len() {
            if timed[j].start >= timed[i].end {
                break;
            }
            if timed[i].speaker == timed[j].speaker {
                continue;
            }
            let overlap = timed[i].end.min(timed[j].end) - timed[j].start;
            if overlap > 0.0 {
                crosstalk[i] += overlap;
                crosstalk[j] += overlap;
            }
        }
    }

    struct Acc {
        segment_count: usize,
        total_seconds: f64,
        /// (score, index into `timed`) of every candidate row.
        candidates: Vec<(f64, usize)>,
    }

    let mut map: std::collections::HashMap<&str, Acc> = std::collections::HashMap::new();
    let mut order: Vec<&str> = Vec::new();

    for (i, row) in timed.iter().enumerate() {
        if row.speaker == "mic" {
            continue;
        }
        let len = row.end - row.start;
        let neighbors = u32::from(i > 0 && timed[i - 1].speaker == row.speaker)
            + u32::from(timed.get(i + 1).is_some_and(|n| n.speaker == row.speaker));
        let score = clip_score(len, crosstalk[i], neighbors);
        let acc = map.entry(row.speaker).or_insert_with(|| {
            order.push(row.speaker);
            Acc {
                segment_count: 0,
                total_seconds: 0.0,
                candidates: Vec::new(),
            }
        });
        acc.segment_count += 1;
        acc.total_seconds += len;
        acc.candidates.push((score, i));
    }

    let mut samples: Vec<SpeakerSample> = order
        .into_iter()
        .map(|speaker| {
            let mut acc = map.remove(speaker).expect("speaker present in map");
            acc.candidates.sort_by(|a, b| {
                b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal)
            });
            let clips = acc
                .candidates
                .into_iter()
                .take(MAX_CLIPS_PER_SPEAKER)
                .map(|(_, i)| SpeakerClip {
                    start: timed[i].start,
                    end: timed[i].end.min(timed[i].start + MAX_CLIP_SECONDS),
                    text: timed[i].text.to_string(),
                })
                .collect();
            SpeakerSample {
                speaker: speaker.to_string(),
                clips,
                segment_count: acc.segment_count,
                total_seconds: acc.total_seconds,
            }
        })
        .collect();

    samples.sort_by(|a, b| {
        let a_leftover = a.speaker == "system";
        let b_leftover = b.speaker == "system";
        a_leftover.cmp(&b_leftover).then(
            b.total_seconds
                .partial_cmp(&a.total_seconds)
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });

    info!(
        "list_speaker_samples for {}: {} nameable speaker(s): {:?}",
        meeting_id,
        samples.len(),
        samples.iter().map(|s| &s.speaker).collect::<Vec<_>>()
    );

    Ok(samples)
}

/// Decode the whole of a meeting's audio to 16 kHz mono f32 and cache it (see
/// `CLIP_AUDIO_CACHE`), reusing the cache when it already holds this meeting.
/// The full decode is the expensive part of a clip preview, so both the play
/// command and the prewarm command funnel through here to pay it at most once.
async fn cached_meeting_samples(
    pool: &sqlx::SqlitePool,
    meeting_id: &str,
) -> Result<Arc<Vec<f32>>, String> {
    // Reuse the cached decode when it is for this same meeting.
    let cached = {
        let guard = CLIP_AUDIO_CACHE.lock().unwrap();
        match &*guard {
            Some((id, samples)) if id == meeting_id => Some(samples.clone()),
            _ => None,
        }
    };
    if let Some(s) = cached {
        return Ok(s);
    }

    let meeting = MeetingsRepository::get_meeting_metadata(pool, meeting_id)
        .await
        .map_err(|e| format!("Failed to load meeting: {e}"))?
        .ok_or_else(|| format!("Meeting not found: {meeting_id}"))?;
    let folder = meeting
        .folder_path
        .filter(|p| !p.trim().is_empty())
        .ok_or_else(|| "Meeting has no folder_path; cannot locate audio".to_string())?;
    let audio_path = find_audio_file(&PathBuf::from(&folder))
        .map_err(|e| format!("Could not locate audio in {folder}: {e}"))?;

    let decoded = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<f32>> {
        let decoded = decode_audio_file(&audio_path)
            .map_err(|e| anyhow!("Failed to decode audio: {e}"))?;
        Ok(decoded.to_whisper_format())
    })
    .await
    .map_err(|e| format!("Audio decode task panicked: {e}"))?
    .map_err(|e| e.to_string())?;

    let arc = Arc::new(decoded);
    let mut guard = CLIP_AUDIO_CACHE.lock().unwrap();
    *guard = Some((meeting_id.to_string(), arc.clone()));
    Ok(arc)
}

/// Warm `CLIP_AUDIO_CACHE` for a meeting so the first preview play in the
/// naming step is instant instead of paying the full-file decode on click.
/// Called fire-and-forget by the frontend when the naming step opens; any
/// error is harmless (the play itself re-attempts and surfaces it).
#[tauri::command]
pub async fn prewarm_audio_clip_cache(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<(), String> {
    let pool = state.db_manager.pool().clone();
    info!("prewarm_audio_clip_cache for {meeting_id}");
    cached_meeting_samples(&pool, &meeting_id).await?;
    Ok(())
}

/// Decode a slice of a meeting's audio and return it as a 16 kHz mono PCM16
/// WAV, for previewing a speaker's voice. `start`/`end` are seconds from the
/// recording start. The full decode is cached per meeting (see
/// `CLIP_AUDIO_CACHE`) so only the first clip pays the decode cost; the WAV is
/// returned over the raw binary IPC channel (not a JSON number array) so the
/// bytes don't get serialized element-by-element.
#[tauri::command]
pub async fn get_audio_clip(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    start: f64,
    end: f64,
) -> Result<tauri::ipc::Response, String> {
    const SAMPLE_RATE: usize = 16_000;
    let pool = state.db_manager.pool().clone();
    info!(
        "get_audio_clip for {}: requesting {:.1}s..{:.1}s",
        meeting_id, start, end
    );

    let samples = cached_meeting_samples(&pool, &meeting_id).await?;

    let total = samples.len();
    let start_idx = ((start.max(0.0) * SAMPLE_RATE as f64) as usize).min(total);
    let end_idx = ((end.max(0.0) * SAMPLE_RATE as f64) as usize).clamp(start_idx, total);
    let wav = encode_wav_pcm16_mono(&samples[start_idx..end_idx], SAMPLE_RATE as u32);
    Ok(tauri::ipc::Response::new(wav))
}

/// Rename all segments of one speaker tag in a meeting to a human label.
/// Returns the number of rows updated. Empty target names are rejected.
#[tauri::command]
pub async fn rename_meeting_speaker(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    from_speaker: String,
    to_speaker: String,
) -> Result<usize, String> {
    let to = to_speaker.trim();
    if to.is_empty() {
        return Err("New speaker name is empty".to_string());
    }
    let pool = state.db_manager.pool().clone();
    TranscriptsRepository::rename_speaker(&pool, &meeting_id, &from_speaker, to)
        .await
        .map_err(|e| format!("Failed to rename speaker: {e}"))
}

/// Drop the cached decoded audio (see `CLIP_AUDIO_CACHE`). Called when the
/// naming dialog closes so a long meeting's samples don't linger in memory.
#[tauri::command]
pub fn clear_audio_clip_cache() {
    if let Ok(mut guard) = CLIP_AUDIO_CACHE.lock() {
        *guard = None;
    }
}

/// Whether this meeting has any mic-tagged ("You") transcript segments.
///
/// When true, diarization masks the local user's audio out of clustering, so
/// the "Number of speakers" field should exclude the user (count only the
/// others). When false (e.g. a mono mix with no source split), the user's own
/// voice is clustered like anyone else and must be counted too.
#[tauri::command]
pub async fn meeting_has_mic_channel(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<bool, String> {
    let pool = state.db_manager.pool().clone();
    let row: (i64,) = sqlx::query_as(
        "SELECT EXISTS(SELECT 1 FROM transcripts WHERE meeting_id = ? AND speaker = 'mic')",
    )
    .bind(&meeting_id)
    .fetch_one(&pool)
    .await
    .map_err(|e| format!("Failed to check mic channel: {e}"))?;
    Ok(row.0 != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_provider_defaults_to_local() {
        assert_eq!(parse_provider(None).unwrap(), DiarProvider::Local);
    }

    #[test]
    fn parse_provider_accepts_known_values() {
        assert_eq!(parse_provider(Some("local")).unwrap(), DiarProvider::Local);
        assert_eq!(
            parse_provider(Some("local-pro")).unwrap(),
            DiarProvider::LocalPro
        );
        assert_eq!(
            parse_provider(Some("pyannote")).unwrap(),
            DiarProvider::PyannoteCloud
        );
    }

    #[test]
    fn parse_provider_rejects_unknown_values() {
        assert!(parse_provider(Some("skynet")).is_err());
    }
}
