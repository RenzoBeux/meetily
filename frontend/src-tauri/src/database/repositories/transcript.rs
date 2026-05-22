use crate::api::{TranscriptSearchResult, TranscriptSegment};
use chrono::Utc;
use sqlx::{Connection, Error as SqlxError, SqlitePool};
use tracing::{error, info};
use uuid::Uuid;

/// Row payload for inserting the tail half of a segment after a split.
/// The caller computes timestamps; this struct just transports them to the
/// repo so the SQL stays in one place.
#[derive(Debug, Clone)]
pub struct NewSegmentRow {
    pub id: String,
    pub meeting_id: String,
    pub text: String,
    pub timestamp: String,
    pub audio_start_time: Option<f64>,
    pub audio_end_time: Option<f64>,
    pub duration: Option<f64>,
    pub speaker: Option<String>,
}

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

    /// Bulk-update the `speaker` column for a set of transcript rows by id.
    /// All updates run in a single transaction so a failure mid-write rolls
    /// back cleanly. Used by post-recording diarization (re-runs on past
    /// meetings) — typical batch size is one row per transcript segment, so
    /// per-row UPDATE is fine.
    pub async fn update_speakers(
        pool: &SqlitePool,
        updates: &[(String, Option<String>)],
    ) -> Result<usize, SqlxError> {
        if updates.is_empty() {
            return Ok(0);
        }

        let mut conn = pool.acquire().await?;
        let mut transaction = conn.begin().await?;

        let mut affected: usize = 0;
        for (id, speaker) in updates {
            let result = sqlx::query("UPDATE transcripts SET speaker = ? WHERE id = ?")
                .bind(speaker)
                .bind(id)
                .execute(&mut *transaction)
                .await;

            match result {
                Ok(res) => affected += res.rows_affected() as usize,
                Err(e) => {
                    error!("Failed to update speaker for transcript {}: {}", id, e);
                    transaction.rollback().await?;
                    return Err(e);
                }
            }
        }

        transaction.commit().await?;
        info!(
            "Updated speaker on {} of {} transcript rows",
            affected,
            updates.len()
        );
        Ok(affected)
    }

    /// Update the `transcript` (text) column for a single segment.
    /// Returns true if a row matched the id.
    pub async fn update_segment_text(
        pool: &SqlitePool,
        segment_id: &str,
        new_text: &str,
    ) -> Result<bool, SqlxError> {
        let result = sqlx::query("UPDATE transcripts SET transcript = ? WHERE id = ?")
            .bind(new_text)
            .bind(segment_id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Bulk-delete transcript rows by id in a single transaction.
    /// Returns the number of rows actually removed.
    pub async fn delete_segments(
        pool: &SqlitePool,
        segment_ids: &[String],
    ) -> Result<usize, SqlxError> {
        if segment_ids.is_empty() {
            return Ok(0);
        }

        let mut conn = pool.acquire().await?;
        let mut transaction = conn.begin().await?;

        let mut affected: usize = 0;
        for id in segment_ids {
            let result = sqlx::query("DELETE FROM transcripts WHERE id = ?")
                .bind(id)
                .execute(&mut *transaction)
                .await;

            match result {
                Ok(res) => affected += res.rows_affected() as usize,
                Err(e) => {
                    error!("Failed to delete transcript {}: {}", id, e);
                    transaction.rollback().await?;
                    return Err(e);
                }
            }
        }

        transaction.commit().await?;
        info!(
            "Deleted {} of {} transcript rows",
            affected,
            segment_ids.len()
        );
        Ok(affected)
    }

    /// Merge: keep one segment row (update its text/end/duration/speaker) and
    /// delete the others, all atomically. The caller computes the merged
    /// values from the source rows.
    pub async fn merge_segments(
        pool: &SqlitePool,
        keeper_id: &str,
        merged_text: &str,
        audio_end_time: f64,
        duration: f64,
        speaker: Option<&str>,
        deleted_ids: &[String],
    ) -> Result<(), SqlxError> {
        let mut conn = pool.acquire().await?;
        let mut transaction = conn.begin().await?;

        let update = sqlx::query(
            "UPDATE transcripts
             SET transcript = ?, audio_end_time = ?, duration = ?, speaker = ?
             WHERE id = ?",
        )
        .bind(merged_text)
        .bind(audio_end_time)
        .bind(duration)
        .bind(speaker)
        .bind(keeper_id)
        .execute(&mut *transaction)
        .await;

        if let Err(e) = update {
            error!("Failed to update merge keeper {}: {}", keeper_id, e);
            transaction.rollback().await?;
            return Err(e);
        }

        for id in deleted_ids {
            let result = sqlx::query("DELETE FROM transcripts WHERE id = ?")
                .bind(id)
                .execute(&mut *transaction)
                .await;
            if let Err(e) = result {
                error!("Failed to delete merge source {}: {}", id, e);
                transaction.rollback().await?;
                return Err(e);
            }
        }

        transaction.commit().await?;
        info!(
            "Merged {} rows into keeper {}",
            deleted_ids.len() + 1,
            keeper_id
        );
        Ok(())
    }

    /// Split: update the source row to hold only the head, then insert a new
    /// row for the tail. The caller computes interpolated timestamps.
    pub async fn split_segment(
        pool: &SqlitePool,
        source_id: &str,
        head_text: &str,
        head_end_time: f64,
        head_duration: f64,
        tail: &NewSegmentRow,
    ) -> Result<(), SqlxError> {
        let mut conn = pool.acquire().await?;
        let mut transaction = conn.begin().await?;

        let update = sqlx::query(
            "UPDATE transcripts
             SET transcript = ?, audio_end_time = ?, duration = ?
             WHERE id = ?",
        )
        .bind(head_text)
        .bind(head_end_time)
        .bind(head_duration)
        .bind(source_id)
        .execute(&mut *transaction)
        .await;

        if let Err(e) = update {
            error!("Failed to update split source {}: {}", source_id, e);
            transaction.rollback().await?;
            return Err(e);
        }

        let insert = sqlx::query(
            "INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time, audio_end_time, duration, speaker)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(&tail.id)
        .bind(&tail.meeting_id)
        .bind(&tail.text)
        .bind(&tail.timestamp)
        .bind(tail.audio_start_time)
        .bind(tail.audio_end_time)
        .bind(tail.duration)
        .bind(&tail.speaker)
        .execute(&mut *transaction)
        .await;

        if let Err(e) = insert {
            error!("Failed to insert split tail for source {}: {}", source_id, e);
            transaction.rollback().await?;
            return Err(e);
        }

        transaction.commit().await?;
        info!("Split segment {} into head + tail {}", source_id, tail.id);
        Ok(())
    }

    /// Bulk insert transcript rows with explicit ids. Used by undo to restore
    /// segments previously deleted or merged away. Idempotent against
    /// already-present rows via INSERT OR IGNORE.
    pub async fn bulk_insert_segments(
        pool: &SqlitePool,
        rows: &[NewSegmentRow],
    ) -> Result<usize, SqlxError> {
        if rows.is_empty() {
            return Ok(0);
        }

        let mut conn = pool.acquire().await?;
        let mut transaction = conn.begin().await?;

        let mut affected: usize = 0;
        for row in rows {
            let result = sqlx::query(
                "INSERT OR IGNORE INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time, audio_end_time, duration, speaker)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
            )
            .bind(&row.id)
            .bind(&row.meeting_id)
            .bind(&row.text)
            .bind(&row.timestamp)
            .bind(row.audio_start_time)
            .bind(row.audio_end_time)
            .bind(row.duration)
            .bind(&row.speaker)
            .execute(&mut *transaction)
            .await;

            match result {
                Ok(res) => affected += res.rows_affected() as usize,
                Err(e) => {
                    error!("Failed to insert segment {}: {}", row.id, e);
                    transaction.rollback().await?;
                    return Err(e);
                }
            }
        }

        transaction.commit().await?;
        info!("Inserted {} of {} segments", affected, rows.len());
        Ok(affected)
    }

    /// Update text + audio bounds + duration on a single segment.
    /// Used by undo of `split` to restore the source row's pre-split state.
    pub async fn update_segment_bounds(
        pool: &SqlitePool,
        segment_id: &str,
        new_text: &str,
        audio_end_time: f64,
        duration: f64,
    ) -> Result<bool, SqlxError> {
        let result = sqlx::query(
            "UPDATE transcripts
             SET transcript = ?, audio_end_time = ?, duration = ?
             WHERE id = ?",
        )
        .bind(new_text)
        .bind(audio_end_time)
        .bind(duration)
        .bind(segment_id)
        .execute(pool)
        .await?;
        Ok(result.rows_affected() > 0)
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
