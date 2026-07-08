// Tauri commands exposing diarization status and runtime acceleration to the
// frontend Settings panel, plus `rediarize_meeting` for re-running diarization
// on past meetings (the live post-recording flow is in
// `recording_manager::run_post_recording_diarization`).

use std::path::PathBuf;

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

    assign_speakers(&mut shims, &diar);

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
    })
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
