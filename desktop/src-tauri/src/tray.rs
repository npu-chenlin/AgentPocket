use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use tauri::menu::{CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime};

use crate::commands::{mutate_config, AppState};
use crate::model::{AppConfig, Backend, ServerStatus};
use crate::opener::open_saved_server;

const TRAY_ID: &str = "main-tray";

/// Pure model of a tray menu entry.
///
/// The controller turns this plan into Tauri menu items. Keeping the plan
/// separate makes the ordering, labels and icon policy testable without a
/// running app.
#[derive(Clone, Debug, PartialEq)]
pub enum TrayItemKind {
    Server {
        id: String,
        name: String,
        backend: Backend,
        connected: bool,
    },
    OpenActive {
        enabled: bool,
    },
    Manage,
    Reconnect {
        running_count: u32,
    },
    Autostart {
        checked: bool,
    },
    Quit,
    Separator,
}

impl TrayItemKind {
    /// Stable ID used for both the plan and the built menu item.
    pub fn id(&self) -> String {
        match self {
            TrayItemKind::Server { id, .. } => format!("server:{id}"),
            TrayItemKind::OpenActive { .. } => "open-active".to_string(),
            TrayItemKind::Manage => "manage".to_string(),
            TrayItemKind::Reconnect { .. } => "reconnect".to_string(),
            TrayItemKind::Autostart { .. } => "autostart".to_string(),
            TrayItemKind::Quit => "quit".to_string(),
            TrayItemKind::Separator => String::new(),
        }
    }

    /// Human-readable label for the entry.
    pub fn text(&self, _config: &AppConfig, _statuses: &HashMap<String, ServerStatus>) -> String {
        match self {
            TrayItemKind::Server { name, .. } => name.clone(),
            TrayItemKind::OpenActive { enabled } => {
                if *enabled {
                    "打开当前服务".to_string()
                } else {
                    "打开当前服务 (未选择)".to_string()
                }
            }
            TrayItemKind::Manage => "管理窗口".to_string(),
            TrayItemKind::Reconnect { running_count } => {
                if *running_count == 0 {
                    "重连全部".to_string()
                } else {
                    format!("重连全部 ({running_count} 个运行中)")
                }
            }
            TrayItemKind::Autostart { .. } => "开机启动".to_string(),
            TrayItemKind::Quit => "退出".to_string(),
            TrayItemKind::Separator => String::new(),
        }
    }
}

/// Build a deterministic menu plan from the current config and statuses.
///
/// Order: one icon item per server, then open-active, manage, reconnect,
/// autostart and quit.
pub fn build_menu_plan(
    config: &AppConfig,
    statuses: &HashMap<String, ServerStatus>,
) -> Vec<TrayItemKind> {
    let mut plan = Vec::new();

    for server in &config.servers {
        let connected = statuses
            .get(&server.id)
            .map(|s| s.connected)
            .unwrap_or(false);
        plan.push(TrayItemKind::Server {
            id: server.id.clone(),
            name: server.name.clone(),
            backend: server.backend,
            connected,
        });
    }

    let active_enabled = config
        .active_id
        .as_ref()
        .map(|id| config.servers.iter().any(|s| &s.id == id))
        .unwrap_or(false);
    plan.push(TrayItemKind::OpenActive {
        enabled: active_enabled,
    });
    plan.push(TrayItemKind::Manage);

    let running_count = statuses.values().map(|s| s.active_count).sum();
    plan.push(TrayItemKind::Reconnect { running_count });

    plan.push(TrayItemKind::Autostart {
        checked: config.settings.autostart,
    });
    plan.push(TrayItemKind::Quit);

    plan
}

/// Tooltip summarizing connected servers and running tasks.
pub fn tooltip(statuses: &HashMap<String, ServerStatus>, server_count: usize) -> String {
    let connected = statuses.values().filter(|s| s.connected).count();
    let running = statuses.values().map(|s| s.active_count).sum::<u32>();
    format!("已连接 {connected}/{server_count} 台，运行 {running} 个任务")
}

/// Controller that owns the single tray icon and rebuilds its menu on demand.
pub struct TrayController<R: Runtime> {
    app_handle: AppHandle<R>,
}

impl<R: Runtime> Clone for TrayController<R> {
    fn clone(&self) -> Self {
        Self {
            app_handle: self.app_handle.clone(),
        }
    }
}

impl<R: Runtime> TrayController<R> {
    /// Create the tray icon, register its event handlers and set the initial menu.
    pub fn install(app: &tauri::App<R>, state: Arc<AppState>) -> Result<Self, tauri::Error> {
        let app_handle = app.handle().clone();
        let controller = Self { app_handle };

        let menu = controller.build_menu(&state)?;
        let tooltip = tooltip_text(&state);

        TrayIconBuilder::with_id(TRAY_ID)
            .icon(tray_icon_image())
            .menu(&menu)
            .tooltip(&tooltip)
            .show_menu_on_left_click(false)
            .on_menu_event({
                let state = Arc::clone(&state);
                move |app, event| handle_menu_event(app, &state, event.id.0.clone())
            })
            .on_tray_icon_event({
                let state = Arc::clone(&state);
                move |tray, event| handle_tray_icon_event(tray, &state, event)
            })
            .build(&controller.app_handle)?;

        Ok(controller)
    }

    /// Replace the tray menu and tooltip from the current application state.
    ///
    /// The old menu and its items are dropped, so handles are not leaked.
    pub fn rebuild(&self, state: Arc<AppState>) -> Result<(), tauri::Error> {
        if let Some(tray) = self.app_handle.tray_by_id(TRAY_ID) {
            let menu = self.build_menu(&state)?;
            tray.set_menu(Some(menu))?;
            tray.set_tooltip(Some(tooltip_text(&state)))?;
        }
        Ok(())
    }

    /// Update the tray tooltip directly.
    pub fn set_tooltip(&self, tooltip: &str) -> Result<(), tauri::Error> {
        if let Some(tray) = self.app_handle.tray_by_id(TRAY_ID) {
            tray.set_tooltip(Some(tooltip))?;
        }
        Ok(())
    }

    fn build_menu(&self, state: &AppState) -> Result<tauri::menu::Menu<R>, tauri::Error> {
        let (config, statuses) = read_state(state);
        let plan = build_menu_plan(&config, &statuses);

        let mut builder = MenuBuilder::new(&self.app_handle);
        for item in plan {
            builder = match item {
                TrayItemKind::Server {
                    id,
                    name,
                    backend,
                    connected,
                } => {
                    let icon = server_icon(backend, connected);
                    builder.icon(format!("server:{id}"), name, icon)
                }
                TrayItemKind::OpenActive { enabled } => {
                    let label = TrayItemKind::OpenActive { enabled }.text(&config, &statuses);
                    MenuItemBuilder::with_id("open-active", label)
                        .enabled(enabled)
                        .build(&self.app_handle)
                        .map(|i| builder.item(&i))?
                }
                TrayItemKind::Manage => {
                    let label = TrayItemKind::Manage.text(&config, &statuses);
                    builder.text("manage", label)
                }
                TrayItemKind::Reconnect { running_count } => {
                    let label = TrayItemKind::Reconnect { running_count }.text(&config, &statuses);
                    builder.text("reconnect", label)
                }
                TrayItemKind::Autostart { checked } => {
                    let item = CheckMenuItemBuilder::with_id("autostart", "开机启动")
                        .checked(checked)
                        .build(&self.app_handle)?;
                    builder.item(&item)
                }
                TrayItemKind::Quit => builder.text("quit", "退出"),
                TrayItemKind::Separator => builder.separator(),
            };
        }

        builder.build()
    }
}

fn read_state(state: &AppState) -> (AppConfig, HashMap<String, ServerStatus>) {
    let config = state
        .config
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let statuses = state
        .statuses
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    (config, statuses)
}

fn tooltip_text(state: &AppState) -> String {
    let (config, statuses) = read_state(state);
    tooltip(&statuses, config.servers.len())
}

fn server_icon(backend: Backend, connected: bool) -> tauri::image::Image<'static> {
    tauri::image::Image::from_bytes(server_icon_bytes(backend, connected))
        .expect("embedded server icon decodes")
}

fn server_icon_bytes(backend: Backend, connected: bool) -> &'static [u8] {
    match (backend, connected) {
        (Backend::Kimi, true) => include_bytes!("../icons/backend-kimi.png"),
        (Backend::Dsh, true) => include_bytes!("../icons/backend-dsh.png"),
        (_, false) => include_bytes!("../icons/backend-offline.png"),
    }
}

fn tray_icon_image() -> tauri::image::Image<'static> {
    tauri::image::Image::from_bytes(include_bytes!("../icons/backend-offline.png"))
        .expect("embedded tray icon decodes")
}

fn handle_menu_event<R: Runtime>(app: &AppHandle<R>, state: &Arc<AppState>, id: String) {
    match id.as_str() {
        id_str if id_str.starts_with("server:") => {
            let server_id = &id_str["server:".len()..];
            let config = state
                .config
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            let _ = open_saved_server(app, &config, server_id, None);
        }
        "open-active" => {
            let config = state
                .config
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone();
            if let Some(active_id) = &config.active_id {
                let _ = open_saved_server(app, &config, active_id, None);
            }
        }
        "manage" => {
            let _ = show_main_window(app);
        }
        "reconnect" => {
            let state = Arc::clone(state);
            tauri::async_runtime::spawn(async move {
                let mut monitors = state.monitors.lock().await;
                monitors.reconnect_all(Duration::from_secs(3)).await;
            });
        }
        "autostart" => {
            let app = app.clone();
            let state = Arc::clone(state);
            tauri::async_runtime::spawn(async move {
                let current = state
                    .config
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .settings
                    .autostart;
                let _ = mutate_config(&state, &app, |config| {
                    config.settings.autostart = !current;
                })
                .await;
            });
        }
        "quit" => {
            state.explicit_exit.store(true, Ordering::SeqCst);
            let app = app.clone();
            let state = Arc::clone(state);
            tauri::async_runtime::spawn(async move {
                let mut monitors = state.monitors.lock().await;
                monitors.shutdown(Duration::from_secs(3)).await;
                drop(monitors);
                app.exit(0);
            });
        }
        _ => {}
    }
}

fn handle_tray_icon_event<R: Runtime>(
    tray: &tauri::tray::TrayIcon<R>,
    state: &Arc<AppState>,
    event: TrayIconEvent,
) {
    if let TrayIconEvent::DoubleClick {
        button: MouseButton::Left,
        ..
    } = event
    {
        let config = state
            .config
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let app = tray.app_handle();
        if let Some(active_id) = &config.active_id {
            let _ = open_saved_server(app, &config, active_id, None);
        } else {
            let _ = show_main_window(app);
        }
    }
}

fn show_main_window<R: Runtime>(app: &AppHandle<R>) -> Result<(), tauri::Error> {
    if let Some(window) = app.get_webview_window("main") {
        window.show()?;
        window.unminimize()?;
        window.set_focus()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AppConfig, Backend, ServerConfig, ServerStatus};

    fn sample_config() -> AppConfig {
        AppConfig {
            active_id: Some("s2".to_string()),
            servers: vec![
                ServerConfig::new("s1", "Work", "host1", 3080, "t", Backend::Dsh),
                ServerConfig::new("s2", "Home", "host2", 3081, "t", Backend::Kimi),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn menu_orders_servers_then_actions() {
        let config = sample_config();
        let statuses = HashMap::new();
        let plan = build_menu_plan(&config, &statuses);

        let ids: Vec<String> = plan.iter().map(TrayItemKind::id).collect();
        assert_eq!(
            ids,
            vec![
                "server:s1",
                "server:s2",
                "open-active",
                "manage",
                "reconnect",
                "autostart",
                "quit",
            ]
        );
    }

    #[test]
    fn offline_server_uses_gray_icon_and_running_count_changes_label() {
        let mut config = AppConfig::default();
        config.servers.push(ServerConfig::new(
            "s1",
            "Work",
            "host",
            3080,
            "t",
            Backend::Kimi,
        ));

        let mut statuses = HashMap::new();
        statuses.insert(
            "s1".to_string(),
            ServerStatus {
                connected: false,
                ..Default::default()
            },
        );

        let plan = build_menu_plan(&config, &statuses);
        let server_item = plan
            .iter()
            .find(|item| item.id() == "server:s1")
            .expect("server item exists");
        assert_eq!(
            *server_item,
            TrayItemKind::Server {
                id: "s1".to_string(),
                name: "Work".to_string(),
                backend: Backend::Kimi,
                connected: false,
            }
        );

        statuses.insert(
            "s1".to_string(),
            ServerStatus {
                connected: true,
                active_count: 4,
                ..Default::default()
            },
        );
        let plan = build_menu_plan(&config, &statuses);
        let reconnect = plan
            .iter()
            .find(|item| item.id() == "reconnect")
            .expect("reconnect item exists");
        assert!(reconnect.text(&config, &statuses).contains('4'));
    }

    #[test]
    fn tooltip_summarizes_connected_and_active_counts() {
        let mut statuses = HashMap::new();
        statuses.insert(
            "s1".to_string(),
            ServerStatus {
                connected: true,
                active_count: 3,
                ..Default::default()
            },
        );
        statuses.insert(
            "s2".to_string(),
            ServerStatus {
                connected: true,
                active_count: 1,
                ..Default::default()
            },
        );
        statuses.insert(
            "s3".to_string(),
            ServerStatus {
                connected: false,
                active_count: 0,
                ..Default::default()
            },
        );

        assert_eq!(tooltip(&statuses, 3), "已连接 2/3 台，运行 4 个任务");
    }
}
