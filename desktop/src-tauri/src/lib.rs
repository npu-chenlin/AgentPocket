use std::sync::Arc;

use chrono::Utc;
use tauri::Manager;
use tokio::sync::mpsc;

use crate::commands::{run_monitor_coordinator, AppState};
use crate::config::{ConfigStore, LoadOutcome};
use crate::model::AppConfig;
use crate::monitor::MonitorManager;

pub mod commands;
pub mod config;
pub mod model;
pub mod monitor;
pub mod notification;
pub mod opener;
pub mod protocol;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None::<Vec<&'static str>>,
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let app_dir = app
                .path()
                .app_data_dir()
                .expect("failed to resolve app data directory");
            let store = ConfigStore::new(app_dir);
            let loaded = store.load().unwrap_or_else(|_| LoadOutcome {
                config: AppConfig::default(),
                recovered_from_backup: None,
            });

            let (update_tx, update_rx) = mpsc::channel(256);
            let monitors = MonitorManager::new(update_tx);
            let state = Arc::new(AppState::new(loaded.config, store, monitors));
            let state_for_coordinator = Arc::clone(&state);
            app.manage(state);

            let app_handle = app.handle().clone();
            let monitor_started_at = Utc::now();
            tauri::async_runtime::spawn(async move {
                run_monitor_coordinator(
                    state_for_coordinator,
                    app_handle,
                    update_rx,
                    monitor_started_at,
                )
                .await;
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_view,
            commands::get_server_for_edit,
            commands::save_server,
            commands::delete_server,
            commands::set_active_server,
            commands::update_settings,
            commands::probe_backend,
            commands::reconnect_all,
            commands::open_server,
            commands::preview_import,
            commands::apply_import,
            commands::export_config,
        ])
        .run(tauri::generate_context!())
        .expect("error while running AgentPocket Desktop");
}
