//! User-facing database backup/restore commands (Settings → Data).
//!
//! Backup (VACUUM INTO snapshot) and listing are safe. Restore replaces the live DB
//! with a chosen snapshot: it checkpoints + closes the pool, moves the current DB
//! aside as a timestamped safety copy (NON-destructive — the pure `restore_db_file`
//! rolls the move back if the copy fails), then relaunches. The file-swap is unit
//! tested; the pool-close + `app.restart()` still need a real running app to verify.

use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, Runtime};

use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct BackupEntry {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub modified_rfc3339: Option<String>,
}

fn backups_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to resolve app data dir: {e}"))?
        .join("backups");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create backups dir: {e}"))?;
    Ok(dir)
}

fn list_backup_entries(dir: &Path) -> Vec<BackupEntry> {
    let mut entries = Vec::new();
    if let Ok(read) = std::fs::read_dir(dir) {
        for e in read.flatten() {
            let path = e.path();
            if path.extension().and_then(|s| s.to_str()) != Some("sqlite") {
                continue;
            }
            let meta = e.metadata().ok();
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let modified = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());
            entries.push(BackupEntry {
                name: e.file_name().to_string_lossy().to_string(),
                path: path.to_string_lossy().to_string(),
                size_bytes: size,
                modified_rfc3339: modified,
            });
        }
    }
    // Timestamped names sort chronologically; newest first.
    entries.sort_by(|a, b| b.name.cmp(&a.name));
    entries
}

/// Create a fresh rotating DB snapshot (VACUUM INTO, WAL-consistent) and return
/// the newest backup entry.
#[tauri::command]
pub async fn db_backup_now<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
) -> Result<BackupEntry, String> {
    let dir = backups_dir(&app)?;
    // backup_to_dir is best-effort (logs on failure); confirm via the listing.
    state.db_manager.backup_to_dir(&dir, 20).await;
    list_backup_entries(&dir)
        .into_iter()
        .next()
        .ok_or_else(|| "Backup failed: no snapshot was created (see logs)".to_string())
}

/// List existing DB backups, newest first.
#[tauri::command]
pub async fn db_list_backups<R: Runtime>(app: AppHandle<R>) -> Result<Vec<BackupEntry>, String> {
    let dir = backups_dir(&app)?;
    Ok(list_backup_entries(&dir))
}

/// Pure, app-free file swap for restore (unit-testable): validate `name` is a plain
/// `.sqlite` filename inside `dir` (no path traversal), move the current DB aside as a
/// timestamped safety copy, drop the old WAL/SHM sidecars, and copy the snapshot into
/// place. On copy failure the move is rolled back so the current DB is left intact.
/// Returns the path of the safety copy. The snapshot is a VACUUM INTO file (self-
/// contained), so no sidecars are copied.
fn restore_db_file(dir: &Path, db_path: &Path, name: &str, ts: &str) -> Result<PathBuf, String> {
    // Reject anything that isn't a single plain filename (blocks `../` traversal).
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || Path::new(name).components().count() != 1
    {
        return Err("Invalid backup name".to_string());
    }
    let source = dir.join(name);
    if source.extension().and_then(|s| s.to_str()) != Some("sqlite") || !source.exists() {
        return Err(format!("Backup '{name}' not found"));
    }

    // Move the current DB aside as a safety copy (do not delete the user's data).
    let aside = db_path.with_file_name(format!("meeting_minutes.pre-restore-{ts}.sqlite"));
    if db_path.exists() {
        std::fs::rename(db_path, &aside)
            .map_err(|e| format!("Failed to move current DB aside: {e}"))?;
    }

    // Remove any residual sidecars of the old DB — they must NOT be applied to the
    // restored snapshot (that would corrupt it).
    for suffix in ["-wal", "-shm"] {
        let mut p = db_path.as_os_str().to_owned();
        p.push(suffix);
        let _ = std::fs::remove_file(PathBuf::from(p));
    }

    // Copy the chosen snapshot into place; roll the move back on failure.
    if let Err(e) = std::fs::copy(&source, db_path) {
        let _ = std::fs::rename(&aside, db_path);
        return Err(format!("Failed to copy backup into place: {e}"));
    }
    Ok(aside)
}

/// Restore the database from a chosen backup snapshot, then relaunch the app.
///
/// DESTRUCTIVE (but reversible): the current DB is moved aside as
/// `meeting_minutes.pre-restore-<ts>.sqlite` before the snapshot is copied in, so a
/// bad restore can be undone by hand. Requires an app relaunch to re-init everything.
#[tauri::command]
pub async fn db_restore_backup<R: Runtime>(
    app: AppHandle<R>,
    state: tauri::State<'_, AppState>,
    name: String,
) -> Result<(), String> {
    let dir = backups_dir(&app)?;
    let db_path = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("Failed to resolve app data dir: {e}"))?
        .join("meeting_minutes.sqlite");

    // Flush the WAL into the main file and close the pool so the DB is unlocked
    // (required on Windows) and the safety copy is complete.
    let pool = state.db_manager.pool();
    let _ = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)").execute(pool).await;
    pool.close().await;

    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let aside = restore_db_file(&dir, &db_path, &name, &ts)?;
    log::info!(
        "Restored DB from backup '{}' (previous DB kept at {}); relaunching",
        name,
        aside.display()
    );

    // Relaunch against the restored DB (diverges on success).
    app.restart();
    #[allow(unreachable_code)]
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_swaps_db_and_keeps_safety_copy() {
        let base = std::env::temp_dir().join("murmur_restore_file_test");
        let _ = std::fs::remove_dir_all(&base);
        let dir = base.join("backups");
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = base.join("meeting_minutes.sqlite");
        std::fs::write(&db_path, b"CURRENT").unwrap();
        std::fs::write(base.join("meeting_minutes.sqlite-wal"), b"stale-wal").unwrap();
        std::fs::write(dir.join("backup-2026.sqlite"), b"SNAPSHOT").unwrap();

        let aside = restore_db_file(&dir, &db_path, "backup-2026.sqlite", "20260713-000000").unwrap();
        assert_eq!(std::fs::read(&db_path).unwrap(), b"SNAPSHOT", "db replaced by snapshot");
        assert_eq!(std::fs::read(&aside).unwrap(), b"CURRENT", "old db kept as safety copy");
        assert!(
            !base.join("meeting_minutes.sqlite-wal").exists(),
            "stale wal removed so it can't corrupt the restored db"
        );

        // Path traversal and missing/non-sqlite names are rejected.
        assert!(restore_db_file(&dir, &db_path, "../evil.sqlite", "x").is_err());
        assert!(restore_db_file(&dir, &db_path, "nope.sqlite", "x").is_err());
        assert!(restore_db_file(&dir, &db_path, "backup-2026.txt", "x").is_err());
        let _ = std::fs::remove_dir_all(&base);
    }
}
