use std::sync::atomic::Ordering;
use std::sync::Arc;

use chrono::Utc;
use tauri::{Listener, Manager};
use tokio::sync::mpsc;

use crate::commands::{run_monitor_coordinator, AppState};
use crate::config::{ConfigStore, LoadOutcome};
use crate::model::AppConfig;
use crate::monitor::MonitorManager;
use crate::tray::TrayController;

pub mod commands;
pub mod config;
pub mod mesh;
pub mod model;
pub mod monitor;
pub mod notification;
pub mod opener;
pub mod protocol;
pub mod sync;
pub mod tray;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 单实例：重复启动时把已有窗口唤到前台，而不是再开一个进程。
        // 需在其他插件之前注册。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
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
            app.manage(Arc::clone(&state));

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

            // 启动时按已加载的配置拉起所有监控任务；否则状态表一直为空，
            // 界面上所有服务器都会显示“离线”，直到首次手动重连或改配置。
            let state_for_initial_sync = Arc::clone(&state);
            tauri::async_runtime::spawn(async move {
                let _ = commands::sync_monitors(&state_for_initial_sync).await;
            });

            // Install the tray icon before showing the main window so the tray
            // is available even when the window is hidden on startup.
            let controller = TrayController::install(app, Arc::clone(&state))?;

            // Listen for rebuild requests from commands/monitor updates.
            let state_for_rebuild = Arc::clone(&state);
            app.listen("tray-rebuild-requested", move |_| {
                let _ = controller.rebuild(Arc::clone(&state_for_rebuild));
            });

            // Close-to-hide behavior: closing the window only hides it unless
            // the user explicitly chose Quit from the tray menu.
            if let Some(window) = app.get_webview_window("main") {
                let state_for_close = Arc::clone(&state);
                let window_for_close = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        if !state_for_close.explicit_exit.load(Ordering::SeqCst) {
                            api.prevent_close();
                            let _ = window_for_close.hide();
                        }
                    }
                });
            }

            // Startup visibility: respect startHidden in production and the
            // --hidden argument in all modes. The window defaults to hidden in
            // tauri.conf.json so we control visibility explicitly here.
            let start_hidden = state
                .config
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .settings
                .start_hidden;
            let hidden_by_arg = std::env::args().any(|arg| arg == "--hidden");
            let show_on_startup = !hidden_by_arg && (tauri::is_dev() || !start_hidden);
            if let Some(window) = app.get_webview_window("main") {
                if show_on_startup {
                    let _ = window.show();
                }
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_view,
            commands::get_server_for_edit,
            commands::save_server,
            commands::delete_server,
            commands::update_settings,
            commands::probe_backend,
            commands::reconnect_all,
            commands::open_server,
            commands::set_always_on_top,
            commands::preview_import_text,
            commands::apply_import,
            commands::export_config,
            commands::export_config_text,
            sync::start_sync_server,
            sync::stop_sync_server,
            mesh::discover_mesh_peers,
            mesh::mesh_pull,
            mesh::mesh_push,
        ])
        .run(tauri::generate_context!())
        .expect("error while running AgentPocket Desktop");
}
