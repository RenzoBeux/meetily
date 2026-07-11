use chrono::Utc;
use sqlx::{migrate::MigrateDatabase, Result, Sqlite, SqlitePool, Transaction};
use std::fs;
use std::path::Path;
use tauri::Manager;

#[derive(Clone)]
pub struct DatabaseManager {
    pool: SqlitePool,
}

impl DatabaseManager {
    pub async fn new(tauri_db_path: &str, backend_db_path: &str) -> Result<Self> {
        if let Some(parent_dir) = Path::new(tauri_db_path).parent() {
            if !parent_dir.exists() {
                fs::create_dir_all(parent_dir).map_err(|e| sqlx::Error::Io(e))?;
            }
        }

        if !Path::new(tauri_db_path).exists() {
            if Path::new(backend_db_path).exists() {
                log::info!(
                    "Copying database from {} to {}",
                    backend_db_path,
                    tauri_db_path
                );
                fs::copy(backend_db_path, tauri_db_path).map_err(|e| sqlx::Error::Io(e))?;
            } else {
                log::info!("Creating database at {}", tauri_db_path);
                Sqlite::create_database(tauri_db_path).await?;
            }
        }

        let pool = SqlitePool::connect(tauri_db_path).await?;

        sqlx::migrate!("./migrations").run(&pool).await?;

        Ok(DatabaseManager { pool })
    }

    // NOTE: So for the first time users they needs to start the application
    // after they can just delete the existing .sqlite file and then copy the existing .db file to
    // the current app dir, So the system detects legacy db and copy it and starts with that data
    // (Newly created .sqlite with the copied content from .db)
    pub async fn new_from_app_handle(app_handle: &tauri::AppHandle) -> Result<Self> {
        // Resolve the app's data directory
        let app_data_dir = app_handle
            .path()
            .app_data_dir()
            .expect("failed to get app data dir");
        if !app_data_dir.exists() {
            fs::create_dir_all(&app_data_dir).map_err(|e| sqlx::Error::Io(e))?;
        }

        // Define database paths
        let tauri_db_path = app_data_dir
            .join("meeting_minutes.sqlite")
            .to_string_lossy()
            .to_string();
        // Legacy backend DB path (for auto-migration if exists)
        let backend_db_path = app_data_dir
            .join("meeting_minutes.db")
            .to_string_lossy()
            .to_string();

        // WAL file paths for defensive cleanup
        let wal_path = app_data_dir.join("meeting_minutes.sqlite-wal");
        let shm_path = app_data_dir.join("meeting_minutes.sqlite-shm");

        log::info!("Tauri DB path: {}", tauri_db_path);
        log::info!("Legacy backend DB path: {}", backend_db_path);

        // Try to open database with defensive WAL handling
        match Self::new(&tauri_db_path, &backend_db_path).await {
            Ok(db_manager) => {
                log::info!("Database opened successfully");
                Ok(db_manager)
            }
            Err(e) => {
                // Check if error is due to corrupted WAL file
                let error_msg = e.to_string();
                if error_msg.contains("malformed") || error_msg.contains("corrupt") {
                    log::warn!("Database appears corrupted, likely due to orphaned WAL file. Attempting recovery...");
                    log::warn!("Error details: {}", error_msg);

                    // QUARANTINE, don't delete. This branch fires exactly after a crash —
                    // the case where the -wal may hold the newest committed meetings.
                    // Deleting it destroyed that data with no undo. Instead we snapshot the
                    // main DB and rename the wal/shm aside so recovery is still possible.
                    let ts = Utc::now().format("%Y%m%d-%H%M%S");
                    let main_path = Path::new(&tauri_db_path);
                    if main_path.exists() {
                        let backup = format!("{}.corrupt-{}.bak", tauri_db_path, ts);
                        match fs::copy(main_path, &backup) {
                            Ok(_) => log::warn!("Backed up main DB before recovery: {}", backup),
                            Err(e) => log::warn!("Failed to back up main DB before recovery: {}", e),
                        }
                    }
                    if wal_path.exists() {
                        let quarantined = wal_path.with_extension(format!("sqlite-wal.corrupt-{}.bak", ts));
                        match fs::rename(&wal_path, &quarantined) {
                            Ok(_) => log::warn!("Quarantined WAL file to: {:?}", quarantined),
                            Err(e) => log::warn!("Failed to quarantine WAL file: {}", e),
                        }
                    }
                    if shm_path.exists() {
                        let quarantined = shm_path.with_extension(format!("sqlite-shm.corrupt-{}.bak", ts));
                        match fs::rename(&shm_path, &quarantined) {
                            Ok(_) => log::warn!("Quarantined SHM file to: {:?}", quarantined),
                            Err(e) => log::warn!("Failed to quarantine SHM file: {}", e),
                        }
                    }

                    // Retry connection after quarantining WAL files
                    log::info!("Retrying database connection after WAL quarantine...");
                    match Self::new(&tauri_db_path, &backend_db_path).await {
                        Ok(db_manager) => {
                            log::info!("Database opened successfully after WAL recovery");
                            Ok(db_manager)
                        }
                        Err(retry_err) => {
                            log::error!("Database connection failed even after WAL cleanup: {}", retry_err);
                            Err(retry_err)
                        }
                    }
                } else {
                    // Not a WAL-related error, propagate original error
                    log::error!("Database connection failed: {}", error_msg);
                    Err(e)
                }
            }
        }
    }

    /// Check if this is the first launch (sqlite database doesn't exist yet)
    pub async fn is_first_launch(app_handle: &tauri::AppHandle) -> Result<bool> {
        let app_data_dir = app_handle
            .path()
            .app_data_dir()
            .expect("failed to get app data dir");

        let tauri_db_path = app_data_dir.join("meeting_minutes.sqlite");

        Ok(!tauri_db_path.exists())
    }

    /// Import a legacy database from the specified path and initialize
    pub async fn import_legacy_database(
        app_handle: &tauri::AppHandle,
        legacy_db_path: &str,
    ) -> Result<Self> {
        let app_data_dir = app_handle
            .path()
            .app_data_dir()
            .expect("failed to get app data dir");

        if !app_data_dir.exists() {
            fs::create_dir_all(&app_data_dir).map_err(|e| sqlx::Error::Io(e))?;
        }

        // Copy legacy database to app data directory as meeting_minutes.db
        let target_legacy_path = app_data_dir.join("meeting_minutes.db");

        // Guard against a same-path copy. Onboarding auto-detects the legacy DB at
        // exactly this target path, so legacy_db_path == target_legacy_path is common.
        // std::fs::copy truncates the destination before reading, so copying a file
        // onto itself zeroes it (data loss on macOS/Linux). If they resolve to the same
        // file, the legacy DB is already in place — skip the copy and just initialize.
        let same_file = fs::canonicalize(legacy_db_path)
            .ok()
            .zip(fs::canonicalize(&target_legacy_path).ok())
            .map(|(a, b)| a == b)
            .unwrap_or(false);

        if same_file {
            log::info!(
                "Legacy DB is already at the target path ({}); skipping self-copy",
                target_legacy_path.display()
            );
        } else {
            log::info!(
                "Copying legacy database from {} to {}",
                legacy_db_path,
                target_legacy_path.display()
            );
            fs::copy(legacy_db_path, &target_legacy_path).map_err(|e| sqlx::Error::Io(e))?;
        }

        // Now use the standard initialization which will detect and migrate the legacy db
        Self::new_from_app_handle(app_handle).await
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Snapshot the live database to a rotating backup via `VACUUM INTO`, keeping the
    /// newest `keep` snapshots. `VACUUM INTO` produces a consistent copy even with an
    /// active WAL, so this is safe to run at startup. Best-effort: any failure is
    /// logged and swallowed so a backup problem never blocks the app.
    pub async fn backup_to_dir(&self, backups_dir: &Path, keep: usize) {
        if let Err(e) = fs::create_dir_all(backups_dir) {
            log::warn!("Failed to create DB backups dir {:?}: {}", backups_dir, e);
            return;
        }

        let ts = Utc::now().format("%Y%m%d-%H%M%S");
        let dest = backups_dir.join(format!("meeting_minutes-{}.sqlite", ts));
        // SQLite string literal: escape single quotes by doubling them.
        let dest_sql = dest.to_string_lossy().replace('\'', "''");

        match sqlx::query(&format!("VACUUM INTO '{}'", dest_sql))
            .execute(&self.pool)
            .await
        {
            Ok(_) => log::info!("Database backup created: {:?}", dest),
            Err(e) => {
                log::warn!("Database backup (VACUUM INTO) failed: {}", e);
                return;
            }
        }

        // Prune oldest snapshots beyond `keep`.
        let mut snapshots: Vec<_> = match fs::read_dir(backups_dir) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.starts_with("meeting_minutes-") && n.ends_with(".sqlite"))
                        .unwrap_or(false)
                })
                .collect(),
            Err(e) => {
                log::warn!("Failed to list DB backups for pruning: {}", e);
                return;
            }
        };
        // Timestamped names sort chronologically; oldest first.
        snapshots.sort();
        if snapshots.len() > keep {
            for old in &snapshots[..snapshots.len() - keep] {
                match fs::remove_file(old) {
                    Ok(_) => log::info!("Pruned old DB backup: {:?}", old),
                    Err(e) => log::warn!("Failed to prune old DB backup {:?}: {}", old, e),
                }
            }
        }
    }

    pub async fn with_transaction<T, F, Fut>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&mut Transaction<'_, Sqlite>) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let mut tx = self.pool.begin().await?;
        let result = f(&mut tx).await;

        match result {
            Ok(val) => {
                tx.commit().await?;
                Ok(val)
            }
            Err(err) => {
                tx.rollback().await?;
                Err(err)
            }
        }
    }

    /// Cleanup database connection and checkpoint WAL
    /// This should be called on application shutdown to ensure:
    /// - All WAL changes are written to the main database file
    /// - The .wal and .shm files are deleted
    /// - Connection pool is gracefully closed
    pub async fn cleanup(&self) -> Result<()> {
        log::info!("Starting database cleanup...");

        // Force checkpoint of WAL to main database file and remove WAL file
        // TRUNCATE mode: checkpoints all pages AND deletes the WAL file
        match sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
            .execute(&self.pool)
            .await
        {
            Ok(_) => log::info!("WAL checkpoint completed successfully"),
            Err(e) => log::warn!("WAL checkpoint failed (non-fatal): {}", e),
        }

        // Close the connection pool gracefully
        self.pool.close().await;
        log::info!("Database connection pool closed");

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    #[tokio::test]
    async fn backup_to_dir_creates_snapshot_and_prunes_old() {
        let dir = std::env::temp_dir().join("murmur_backup_to_dir_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        // Use a file-backed source DB (VACUUM INTO copies real pages).
        let src_db = dir.join("source.sqlite");
        let src_url = format!("sqlite://{}?mode=rwc", src_db.to_string_lossy().replace('\\', "/"));
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(&src_url)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE t (x INTEGER)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO t (x) VALUES (1)").execute(&pool).await.unwrap();
        let mgr = DatabaseManager { pool };

        // Seed older snapshots whose timestamped names sort before today's real one.
        for name in [
            "meeting_minutes-20250101-000001.sqlite",
            "meeting_minutes-20250101-000002.sqlite",
            "meeting_minutes-20250101-000003.sqlite",
        ] {
            fs::write(dir.join(name), b"old").unwrap();
        }

        // Creates one real VACUUM INTO snapshot, then prunes to keep the 2 newest.
        mgr.backup_to_dir(&dir, 2).await;

        let mut remaining: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| (e.file_name().to_string_lossy().to_string(), e.metadata().map(|m| m.len()).unwrap_or(0)))
            .filter(|(n, _)| n.starts_with("meeting_minutes-") && n.ends_with(".sqlite"))
            .collect();
        remaining.sort();
        eprintln!("remaining snapshots: {:?}", remaining);

        assert_eq!(remaining.len(), 2, "should keep exactly `keep` snapshots");
        // The freshly created backup (today's date) must survive pruning and be a
        // valid, non-empty SQLite file (not one of the 3-byte "old" seeds).
        let (newest_name, newest_len) = remaining.iter().max_by(|a, b| a.0.cmp(&b.0)).unwrap();
        assert!(
            *newest_len > 3,
            "newest snapshot {} should be a real DB copy, was {} bytes",
            newest_name,
            newest_len
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
