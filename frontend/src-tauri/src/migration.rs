// One-time startup migration from the legacy "Meetily" branding to "Murmur".
//
// Renaming the bundle identifier and the manual data folders moved every
// on-disk location the app reads; without this, an upgraded install would
// look like a first launch (empty DB, re-downloaded models). Runs at the very
// top of `setup()` — before the database, model engines, or notification
// manager touch their directories. Every step is best-effort and non-fatal:
// a fresh install simply finds nothing to migrate.

use std::fs;
use std::path::Path;

use tauri::{AppHandle, Manager, Runtime};

const LEGACY_IDENTIFIER: &str = "com.meetily.ai";

pub fn migrate_legacy_brand_dirs<R: Runtime>(app: &AppHandle<R>) {
    // 1. App data dir (SQLite DB, tauri-plugin-store preferences):
    //    <data>/com.meetily.ai[.dev] -> <data>/com.murmur.app[.dev]
    if let Ok(new_dir) = app.path().app_data_dir() {
        if let Some(parent) = new_dir.parent() {
            let is_dev = new_dir
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".dev"));
            let legacy_name = if is_dev {
                format!("{LEGACY_IDENTIFIER}.dev")
            } else {
                LEGACY_IDENTIFIER.to_string()
            };
            rename_if_needed(&parent.join(legacy_name), &new_dir);
        }
    }

    // 2. Manual models/templates root: <data>/Meetily -> <data>/Murmur
    if let Some(data_dir) = dirs::data_dir() {
        rename_if_needed(&data_dir.join("Meetily"), &data_dir.join("Murmur"));
    }

    // 3. Notification config root: <config>/meetily -> <config>/murmur.
    //    On Windows, config_dir == data_dir and the filesystem is
    //    case-insensitive, so step 2 already covered this and the guards
    //    make it a no-op; on macOS/Linux it is a distinct directory.
    if let Some(config_dir) = dirs::config_dir() {
        rename_if_needed(&config_dir.join("meetily"), &config_dir.join("murmur"));
    }
}

/// Rename `old` to `new` when `old` has data and `new` doesn't. A `new` that
/// exists but is an empty directory (e.g. pre-created by a plugin) is removed
/// so the rename can proceed.
fn rename_if_needed(old: &Path, new: &Path) {
    if !old.exists() || old == new {
        return;
    }
    if new.exists() {
        let new_is_empty_dir = fs::read_dir(new).map(|mut d| d.next().is_none()).unwrap_or(false);
        if !new_is_empty_dir {
            return;
        }
        if let Err(e) = fs::remove_dir(new) {
            log::warn!(
                "Legacy data migration: could not clear empty {}: {}",
                new.display(),
                e
            );
            return;
        }
    }
    match fs::rename(old, new) {
        Ok(()) => log::info!(
            "Migrated legacy data dir {} -> {}",
            old.display(),
            new.display()
        ),
        Err(e) => log::warn!(
            "Could not migrate legacy data dir {} -> {}: {} (the app will start fresh; \
             move the folder manually to recover old data)",
            old.display(),
            new.display(),
            e
        ),
    }
}
