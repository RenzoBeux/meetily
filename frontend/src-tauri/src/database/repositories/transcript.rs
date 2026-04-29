use crate::api::{TranscriptSearchResult, TranscriptSegment};
use chrono::Utc;
use sqlx::{Connection, Error as SqlxError, SqlitePool};
use std::collections::HashMap;
use tracing::{error, info};
use uuid::Uuid;

pub struct TranscriptsRepository;

impl TranscriptsRepository {
    /// Saves a new meeting and its associated transcript segments.
    /// This function uses a transaction to ensure that either both the meeting
    /// and all its transcripts are saved, or none of them are.
    pub async fn save_transcript(
        pool: &SqlitePool,
        meeting_title: &str,
        transcripts: &[TranscriptSegment],
        folder_path: Option<String>,
    ) -> Result<String, SqlxError> {
        let meeting_id = format!("meeting-{}", Uuid::new_v4());

        let mut conn = pool.acquire().await?;
        let mut transaction = conn.begin().await?;

        let now = Utc::now();

        // 1. Create the new meeting
        let result = sqlx::query(
            "INSERT INTO meetings (id, title, created_at, updated_at, folder_path) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&meeting_id)
        .bind(meeting_title)
        .bind(now)
        .bind(now)
        .bind(&folder_path)
        .execute(&mut *transaction)
        .await;

        if let Err(e) = result {
            error!("Failed to create meeting '{}': {}", meeting_title, e);
            transaction.rollback().await?;
            return Err(e);
        }

        info!("Successfully created meeting with id: {}", meeting_id);

        // 2. Save each transcript segment with audio timing fields and speaker tag
        for segment in transcripts {
            let transcript_id = format!("transcript-{}", Uuid::new_v4());
            let result = sqlx::query(
                "INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time, audio_end_time, duration, speaker)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
            )
            .bind(&transcript_id)
            .bind(&meeting_id)
            .bind(&segment.text)
            .bind(&segment.timestamp)
            .bind(segment.audio_start_time)
            .bind(segment.audio_end_time)
            .bind(segment.duration)
            .bind(&segment.speaker)
            .execute(&mut *transaction)
            .await;

            if let Err(e) = result {
                error!(
                    "Failed to save transcript segment for meeting {}: {}",
                    meeting_id, e
                );
                transaction.rollback().await?;
                return Err(e);
            }
        }

        info!(
            "Successfully saved {} transcript segments for meeting {}",
            transcripts.len(),
            meeting_id
        );

        // Commit the transaction
        transaction.commit().await?;

        Ok(meeting_id)
    }

    /// Bulk-update the speaker tag for a set of transcript IDs.
    /// Used after diarization to overwrite the source-faithful tags
    /// ("mic"/"system") with per-speaker IDs.
    pub async fn update_speakers(
        pool: &SqlitePool,
        speaker_map: &HashMap<String, String>,
    ) -> Result<u64, SqlxError> {
        if speaker_map.is_empty() {
            return Ok(0);
        }

        let mut conn = pool.acquire().await?;
        let mut transaction = conn.begin().await?;
        let mut updated: u64 = 0;

        for (transcript_id, speaker) in speaker_map {
            let result = sqlx::query("UPDATE transcripts SET speaker = ? WHERE id = ?")
                .bind(speaker)
                .bind(transcript_id)
                .execute(&mut *transaction)
                .await?;
            updated += result.rows_affected();
        }

        transaction.commit().await?;
        info!("Updated speaker tags on {} transcripts", updated);
        Ok(updated)
    }

    /// Update the speaker tag on a single transcript segment. Used when the
    /// user manually reassigns one chunk to a different speaker.
    pub async fn update_speaker_for_transcript(
        pool: &SqlitePool,
        transcript_id: &str,
        speaker: &str,
    ) -> Result<bool, SqlxError> {
        let result = sqlx::query("UPDATE transcripts SET speaker = ? WHERE id = ?")
            .bind(speaker)
            .bind(transcript_id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Rename a speaker across all transcripts in a meeting. Used when the
    /// user gives a real name to a diarization-assigned ID like "speaker_1".
    pub async fn rename_speaker_in_meeting(
        pool: &SqlitePool,
        meeting_id: &str,
        old_speaker: &str,
        new_speaker: &str,
    ) -> Result<u64, SqlxError> {
        let result = sqlx::query(
            "UPDATE transcripts SET speaker = ? WHERE meeting_id = ? AND speaker = ?",
        )
        .bind(new_speaker)
        .bind(meeting_id)
        .bind(old_speaker)
        .execute(pool)
        .await?;
        info!(
            "Renamed speaker '{}' -> '{}' on {} transcripts in meeting {}",
            old_speaker, new_speaker, result.rows_affected(), meeting_id
        );
        Ok(result.rows_affected())
    }

    /// Searches for a query string within the transcripts.
    /// It returns a list of matching transcripts with context.
    pub async fn search_transcripts(
        pool: &SqlitePool,
        query: &str,
    ) -> Result<Vec<TranscriptSearchResult>, SqlxError> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        let search_query = format!("%{}%", query.to_lowercase());

        let rows = sqlx::query_as::<_, (String, String, String, String)>(
            "SELECT m.id, m.title, t.transcript, t.timestamp
             FROM meetings m
             JOIN transcripts t ON m.id = t.meeting_id
             WHERE LOWER(t.transcript) LIKE ?",
        )
        .bind(&search_query)
        .fetch_all(pool)
        .await?;

        let results = rows
            .into_iter()
            .map(|(id, title, transcript, timestamp)| {
                let match_context = Self::get_match_context(&transcript, query);
                TranscriptSearchResult {
                    id,
                    title,
                    match_context,
                    timestamp,
                }
            })
            .collect();

        Ok(results)
    }

    /// Helper function to extract a snippet of text around the first match of a query.
    fn get_match_context(transcript: &str, query: &str) -> String {
        let transcript_lower = transcript.to_lowercase();
        let query_lower = query.to_lowercase();

        match transcript_lower.find(&query_lower) {
            Some(match_index) => {
                let start_index = match_index.saturating_sub(100);
                let end_index = (match_index + query.len() + 100).min(transcript.len());

                let mut context = String::new();
                if start_index > 0 {
                    context.push_str("...");
                }
                context.push_str(&transcript[start_index..end_index]);
                if end_index < transcript.len() {
                    context.push_str("...");
                }
                context
            }
            None => transcript.chars().take(200).collect(), // Fallback to the start of the transcript
        }
    }
}
