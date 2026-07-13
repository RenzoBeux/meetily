//! User-facing database backup commands (Settings → Data).
//!
//! Restore is intentionally NOT implemented here yet: replacing the live DB file
//! requires closing the pool, overwriting `meeting_minutes.sqlite` + sidecars, and
//! relaunching the app — a destructive flow that needs a real running app to
//! verify safely. Backup (VACUUM INTO snapshot) and listing are safe and covered.

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
