use log::info;
use tauri::{AppHandle, Emitter, Manager};

use super::manager::DatabaseManager;
use crate::state::AppState;

/// Initialize database on app startup
/// Handles first launch detection and conditional initialization
pub async fn initialize_database_on_startup(app: &AppHandle) -> Result<(), String> {
    // Check if this is the first launch (no database exists yet)
    let is_first_launch = DatabaseManager::is_first_launch(app)
        .await
        .map_err(|e| format!("Failed to check first launch status: {}", e))?;

    if is_first_launch {
        info!("First launch detected - will notify window when ready");

        // Delay event emission to ensure window is ready and React listeners are registered
        let app_handle = app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
            app_handle
                .emit("first-launch-detected", ())
                .expect("Failed to emit first-launch-detected event");
            info!("Emitted first-launch-detected after delay");
        });
    } else {
        // Normal flow - initialize database immediately
        let db_manager = DatabaseManager::new_from_app_handle(app)
            .await
            .map_err(|e| format!("Failed to initialize database manager: {}", e))?;

        // Rotating startup backup (best-effort; keep the last 5 snapshots). Uses
        // VACUUM INTO, which is consistent even with an active WAL. This is the DB's
        // safety net: a corrupt-on-next-launch DB can be restored from here.
        if let Ok(app_data_dir) = app.path().app_data_dir() {
            db_manager
                .backup_to_dir(&app_data_dir.join("backups"), 5)
                .await;
        }

        app.manage(AppState { db_manager });
        info!("Database initialized successfully");
    }

    Ok(())
}
