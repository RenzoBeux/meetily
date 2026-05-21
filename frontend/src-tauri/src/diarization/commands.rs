// Tauri commands exposing diarization status and runtime acceleration to the
// frontend Settings panel, plus `rediarize_meeting` for re-running diarization
// on past meetings (the live post-recording flow is in
// `recording_manager::run_post_recording_diarization`).

use std::path::PathBuf;

use anyhow::anyhow;
use log::{info, warn};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

use crate::audio::recording_saver::TranscriptSegment;
use crate::audio::retranscription::find_audio_file;
use crate::database::repositories::meeting::MeetingsRepository;
use crate::database::repositories::transcript::TranscriptsRepository;
use crate::diarization::aligner::{assign_speakers, unique_speakers};
use crate::diarization::engine::{detect_gpu, DiarizationEngine};
use crate::diarization::models::{ensure_models, status, ModelsStatus};
use crate::state::AppState;

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
) -> Result<RediarizationResult, String> {
    let pool = state.db_manager.pool().clone();
    let result = run_rediarization(app.clone(), &pool, &meeting_id).await;

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

    let paths = ensure_models(&app)
        .await
        .map_err(|e| anyhow!("Diarization models unavailable: {e}"))?;

    let _ = app.emit(
        "diarization-progress",
        serde_json::json!({"status": "running", "meeting_id": meeting_id}),
    );

    let audio_for_blocking = audio_path.clone();
    let diar = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        let engine = DiarizationEngine::new(&paths)?;
        engine.run_on_file(&audio_for_blocking)
    })
    .await
    .map_err(|e| anyhow!("Diarization task panicked: {e}"))??;

    let _ = app.emit(
        "diarization-progress",
        serde_json::json!({"status": "aligning", "meeting_id": meeting_id, "segments": diar.len()}),
    );

    // Load the existing transcripts to align against. We use
    // `get_meeting_transcripts_paginated` with a large limit to avoid pulling
    // the meeting body twice.
    let (rows, _total) =
        MeetingsRepository::get_meeting_transcripts_paginated(pool, meeting_id, 100_000, 0)
            .await
            .map_err(|e| anyhow!("Failed to load transcripts: {e}"))?;
    info!(
        "Re-diarization for {}: {} transcript rows loaded",
        meeting_id,
        rows.len()
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
