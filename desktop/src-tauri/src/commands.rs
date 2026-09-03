use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::config::{ConfigError, ConfigStore, ImportMode, ImportPreviewData};
use crate::model::{
    AppConfig, AppView, Backend, DesktopSettings, ServerConfig, ServerForEdit, ServerStatus,
    ServerSummary, ValidationError,
};
use crate::monitor::{probe_backend as probe, MonitorManager, MonitorUpdate, PinnedSessions, ProbeError};
use crate::notification::NotificationCoordinator;
use crate::opener::{open_saved_server, OpenerError};

const IMPORT_PREVIEW_TTL: Duration = Duration::from_secs(10 * 60);

/// Shared application state managed by Tauri.
///
/// All commands interact with this state. Config mutations follow the
/// transaction rule: clone → validate/apply → persist atomically → replace
/// in-memory → update autostart if changed → sync monitors → emit redacted
/// `app-state-changed` → request tray rebuild.
pub struct AppState {
    pub config: RwLock<AppConfig>,
    pub statuses: RwLock<HashMap<String, ServerStatus>>,
    pub store: ConfigStore,
    pub monitors: tokio::sync::Mutex<MonitorManager>,
    /// Serializes config read-modify-write transactions, including import and
    /// autostart side effects. Without this, two commands can both start from
    /// the same snapshot and overwrite one another.
    pub config_mutation: tokio::sync::Mutex<()>,
    pub import_previews: Mutex<HashMap<Uuid, PendingImport>>,
    pub sync_server: Mutex<Option<crate::sync::SyncServerHandle>>,
    /// 用户置顶的会话集合（"server_id|session_id"），与监控任务共享。
    pub pinned: PinnedSessions,
    pub explicit_exit: AtomicBool,
    pub revision: AtomicU64,
}

impl AppState {
    pub fn new(
        config: AppConfig,
        store: ConfigStore,
        monitors: MonitorManager,
        pinned: PinnedSessions,
    ) -> Self {
        Self {
            config: RwLock::new(config),
            statuses: RwLock::new(HashMap::new()),
            store,
            monitors: tokio::sync::Mutex::new(monitors),
            config_mutation: tokio::sync::Mutex::new(()),
            import_previews: Mutex::new(HashMap::new()),
            sync_server: Mutex::new(None),
            pinned,
            explicit_exit: AtomicBool::new(false),
            revision: AtomicU64::new(0),
        }
    }
}

/// Token-bearing import data cached in Rust. The frontend only ever sees the
/// associated [`ImportPreview`] response.
#[derive(Clone)]
pub struct PendingImport {
    pub data: ImportPreviewData,
    pub created_at: Instant,
}

/// Errors returned by Tauri commands. Kept serializable so the frontend can
/// display them.
#[derive(Debug, thiserror::Error, Serialize)]
pub enum CommandError {
    #[error("config error: {0}")]
    Config(String),
    #[error("validation error: {0}")]
    Validation(String),
    #[error("server not found")]
    ServerNotFound,
    #[error("import preview not found")]
    ImportPreviewNotFound,
    #[error("no valid servers to import")]
    NoValidServers,
    #[error("opener error: {0}")]
    Opener(String),
    #[error("probe error: dsh={dsh:?}, kimi={kimi:?}")]
    Probe {
        dsh: Option<String>,
        kimi: Option<String>,
    },
    #[error("monitor error: {0}")]
    Monitor(String),
    #[error("failed to update autostart: {0}")]
    Autostart(String),
    #[error("invalid import mode")]
    InvalidImportMode,
    #[error("invalid import id")]
    InvalidImportId,
    #[error("window error: {0}")]
    Window(String),
    #[error("sync error: {0}")]
    Sync(String),
    // 前端 toast 已带“拉取失败：/推送失败：”前缀，此处不再叠加第二层。
    #[error("{0}")]
    Mesh(String),
}

impl From<ConfigError> for CommandError {
    fn from(error: ConfigError) -> Self {
        CommandError::Config(error.to_string())
    }
}

impl From<ValidationError> for CommandError {
    fn from(error: ValidationError) -> Self {
        CommandError::Validation(error.to_string())
    }
}

impl From<OpenerError> for CommandError {
    fn from(error: OpenerError) -> Self {
        CommandError::Opener(error.to_string())
    }
}

impl From<ProbeError> for CommandError {
    fn from(error: ProbeError) -> Self {
        CommandError::Probe {
            dsh: error.dsh,
            kimi: error.kimi,
        }
    }
}

impl From<agentpocket_core::mesh_client::ClientError> for CommandError {
    fn from(error: agentpocket_core::mesh_client::ClientError) -> Self {
        CommandError::Mesh(error.to_string())
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportModeArg {
    Merge,
    Replace,
}

impl From<ImportModeArg> for ImportMode {
    fn from(arg: ImportModeArg) -> Self {
        match arg {
            ImportModeArg::Merge => ImportMode::Merge,
            ImportModeArg::Replace => ImportMode::Replace,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportIssue {
    pub index: usize,
    pub reason: String,
}

/// Frontend-facing import preview. Contains no tokens.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub import_id: Uuid,
    pub valid_count: usize,
    pub invalid: Vec<ImportIssue>,
}

// ------------------------------------------------------------------
// Commands
// ------------------------------------------------------------------

#[tauri::command]
pub fn get_app_view(state: State<'_, Arc<AppState>>) -> Result<AppView, CommandError> {
    Ok(build_app_view(&state))
}

#[tauri::command]
pub fn get_server_for_edit(
    state: State<'_, Arc<AppState>>,
    id: String,
) -> Result<ServerForEdit, CommandError> {
    get_server_for_edit_inner(&state, &id)
}

#[tauri::command]
pub async fn save_server(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    mut server: ServerConfig,
) -> Result<AppView, CommandError> {
    if server.id.is_empty() {
        server.id = Uuid::new_v4().to_string();
    }
    server.validate()?;

    mutate_config(&state, &app, move |config| {
        if let Some(index) = config.servers.iter().position(|s| s.id == server.id) {
            config.servers[index] = server.clone();
        } else {
            config.servers.push(server.clone());
        }
    })
    .await
}

#[tauri::command]
pub async fn delete_server(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    id: String,
) -> Result<AppView, CommandError> {
    let purge_id = id.clone();
    let result = mutate_config(&state, &app, move |config| {
        config.servers.retain(|s| s.id != id);
        if config.active_id.as_deref() == Some(&id) {
            config.active_id = config.servers.first().map(|s| s.id.clone());
        }
    })
    .await;
    if result.is_ok() {
        purge_server_pins(&state, &purge_id);
    }
    result
}

/// 服务器被删除后，清掉它名下的置顶会话，避免残留 key 永远留在共享集合里。
fn purge_server_pins(state: &AppState, server_id: &str) {
    let prefix = format!("{}|", server_id);
    state
        .pinned
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .retain(|key| !key.starts_with(&prefix));
}

/// 置顶切换的结果：新的置顶状态 + 最新视图（前端立即重绘并提示）。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PinToggleResult {
    pub pinned: bool,
    pub view: AppView,
}

#[tauri::command]
pub fn toggle_session_pin(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
    session_id: String,
) -> Result<PinToggleResult, CommandError> {
    let pinned = toggle_pin_inner(&state, &id, &session_id);
    state.revision.fetch_add(1, Ordering::SeqCst);
    let view = build_app_view(&state);
    emit_app_state_changed(&app, &view);
    request_tray_rebuild(&app);
    Ok(PinToggleResult { pinned, view })
}

/// 切换置顶并即时修补状态表，不必等下一帧 monitor 上报：
/// 置顶忙碌会话打标；取消置顶时已完成行直接移除、运行中行去掉标记。
pub(crate) fn toggle_pin_inner(state: &AppState, server_id: &str, session_id: &str) -> bool {
    let key = format!("{}|{}", server_id, session_id);
    let now_pinned = {
        let mut pinned = state
            .pinned
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if pinned.contains(&key) {
            pinned.remove(&key);
            false
        } else {
            pinned.insert(key);
            true
        }
    };
    let mut statuses = state
        .statuses
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(status) = statuses.get_mut(server_id) {
        if now_pinned {
            for session in &mut status.sessions {
                if session.id == session_id {
                    session.pinned = true;
                }
            }
        } else {
            status
                .sessions
                .retain(|session| !(session.id == session_id && session.done));
            for session in &mut status.sessions {
                if session.id == session_id {
                    session.pinned = false;
                }
            }
        }
    }
    now_pinned
}

#[tauri::command]
pub async fn update_settings(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    settings: DesktopSettings,
) -> Result<AppView, CommandError> {
    mutate_config(&state, &app, move |config| {
        config.settings = settings.clone();
    })
    .await
}

#[tauri::command]
pub async fn probe_backend(server: ServerConfig) -> Result<String, CommandError> {
    server.validate()?;
    let backend = probe(&server).await?;
    Ok(match backend {
        Backend::Kimi => "kimi".to_string(),
        Backend::Dsh => "dsh".to_string(),
    })
}

#[tauri::command]
pub async fn reconnect_all(state: State<'_, Arc<AppState>>) -> Result<(), CommandError> {
    let servers = {
        let config = state
            .config
            .read()
            .map_err(|_| CommandError::Monitor("config lock poisoned".to_string()))?;
        config.servers.clone()
    };
    let mut monitors = state.monitors.lock().await;
    monitors.sync_servers(&servers).await;
    monitors.reconnect_all(Duration::from_secs(3)).await;
    Ok(())
}

#[tauri::command]
pub fn open_server(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    id: String,
    session_id: Option<String>,
) -> Result<(), CommandError> {
    let config = state
        .config
        .read()
        .map_err(|_| CommandError::Config("config lock poisoned".to_string()))?;
    open_saved_server(&app, &config, &id, session_id.as_deref())?;
    Ok(())
}

#[tauri::command]
pub fn set_always_on_top(app: AppHandle, pinned: bool) -> Result<(), CommandError> {
    use tauri::Manager;

    let window = app
        .get_webview_window("main")
        .ok_or_else(|| CommandError::Window("main window not found".to_string()))?;
    window
        .set_always_on_top(pinned)
        .map_err(|e| CommandError::Window(e.to_string()))?;
    Ok(())
}

#[tauri::command]
pub fn preview_import_text(
    state: State<'_, Arc<AppState>>,
    content: String,
) -> Result<ImportPreview, CommandError> {
    preview_from_content(&state, &content)
}

#[tauri::command]
pub async fn apply_import(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    import_id: String,
    mode: ImportModeArg,
) -> Result<AppView, CommandError> {
    let _mutation = state.config_mutation.lock().await;
    let import_id = Uuid::parse_str(&import_id).map_err(|_| CommandError::InvalidImportId)?;
    let data = remove_preview(&state, import_id)?;

    let store = state.store.clone();
    let mode = mode.into();
    let new_config = tokio::task::spawn_blocking(move || store.apply_import_and_save(data, mode))
        .await
        .map_err(|error| CommandError::Config(format!("config import task failed: {error}")))??;

    {
        let mut config = state
            .config
            .write()
            .map_err(|_| CommandError::Config("config lock poisoned".to_string()))?;
        *config = new_config.clone();
    }

    sync_monitors(&state).await?;

    state.revision.fetch_add(1, Ordering::SeqCst);
    let view = build_app_view(&state);
    emit_app_state_changed(&app, &view);
    request_tray_rebuild(&app);
    Ok(view)
}

#[tauri::command]
pub fn export_config(state: State<'_, Arc<AppState>>, path: String) -> Result<(), CommandError> {
    let config = state
        .config
        .read()
        .map_err(|_| CommandError::Config("config lock poisoned".to_string()))?;
    state.store.export(&config, Path::new(&path))?;
    Ok(())
}

#[tauri::command]
pub fn export_config_text(state: State<'_, Arc<AppState>>) -> Result<String, CommandError> {
    let config = state
        .config
        .read()
        .map_err(|_| CommandError::Config("config lock poisoned".to_string()))?;
    Ok(state.store.export_text(&config)?)
}

// ------------------------------------------------------------------
// Helpers
// ------------------------------------------------------------------

pub fn build_app_view(state: &AppState) -> AppView {
    let config = state
        .config
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let statuses = state
        .statuses
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let statuses: HashMap<String, ServerStatus> = statuses
        .iter()
        .filter(|(id, _)| config.servers.iter().any(|server| &server.id == *id))
        .map(|(id, status)| (id.clone(), status.clone()))
        .collect();
    AppView {
        revision: state.revision.load(Ordering::SeqCst),
        settings: config.settings.clone(),
        servers: config.servers.iter().map(ServerSummary::from).collect(),
        active_id: config.active_id.clone(),
        statuses: statuses.clone(),
    }
}

fn get_server_for_edit_inner(state: &AppState, id: &str) -> Result<ServerForEdit, CommandError> {
    let config = state
        .config
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    config
        .servers
        .iter()
        .find(|s| s.id == id)
        .map(ServerForEdit::from)
        .ok_or(CommandError::ServerNotFound)
}

pub(crate) async fn mutate_config<R, F>(
    state: &Arc<AppState>,
    app: &AppHandle<R>,
    mutate: F,
) -> Result<AppView, CommandError>
where
    R: tauri::Runtime,
    F: Fn(&mut AppConfig) + Send + 'static,
{
    let _mutation = state.config_mutation.lock().await;
    let old_config = {
        let config = state
            .config
            .read()
            .map_err(|_| CommandError::Config("config lock poisoned".to_string()))?;
        config.clone()
    };

    let mut new_config = old_config.clone();
    mutate(&mut new_config);
    validate_config(&new_config)?;

    // Apply the external side effect first. If it fails, neither the file nor
    // in-memory config changes. If persistence fails afterwards, compensate by
    // restoring the previous autostart state.
    if old_config.settings.autostart != new_config.settings.autostart {
        update_autostart(app, new_config.settings.autostart).await?;
    }

    let store = state.store.clone();
    let persisted = tokio::task::spawn_blocking(move || store.update(mutate))
        .await
        .map_err(|error| CommandError::Config(format!("config update task failed: {error}")))?;
    let new_config = match persisted {
        Ok(config) => config,
        Err(error) => {
            if old_config.settings.autostart != new_config.settings.autostart {
                let _ = update_autostart(app, old_config.settings.autostart).await;
            }
            return Err(error.into());
        }
    };

    {
        let mut config = state
            .config
            .write()
            .map_err(|_| CommandError::Config("config lock poisoned".to_string()))?;
        *config = new_config.clone();
    }

    // Step 6: sync affected monitors.
    sync_monitors(state).await?;

    // Step 7: emit redacted app-state-changed.
    state.revision.fetch_add(1, Ordering::SeqCst);
    let view = build_app_view(state);
    emit_app_state_changed(app, &view);

    // Step 8: request tray rebuild.
    request_tray_rebuild(app);

    Ok(view)
}

fn validate_config(config: &AppConfig) -> Result<(), CommandError> {
    for server in &config.servers {
        server.validate()?;
    }
    if let Some(active_id) = &config.active_id {
        if !config.servers.iter().any(|s| &s.id == active_id) {
            return Err(CommandError::Validation(
                "active server does not exist".to_string(),
            ));
        }
    }
    Ok(())
}

async fn update_autostart<R: tauri::Runtime>(
    app: &AppHandle<R>,
    enabled: bool,
) -> Result<(), CommandError> {
    use tauri_plugin_autostart::ManagerExt;

    let autolaunch = app.autolaunch();
    let is_enabled = autolaunch
        .is_enabled()
        .map_err(|e| CommandError::Autostart(e.to_string()))?;

    if enabled && !is_enabled {
        autolaunch
            .enable()
            .map_err(|e| CommandError::Autostart(e.to_string()))?;
    } else if !enabled && is_enabled {
        autolaunch
            .disable()
            .map_err(|e| CommandError::Autostart(e.to_string()))?;
    }
    Ok(())
}

pub(crate) async fn sync_monitors(state: &Arc<AppState>) -> Result<(), CommandError> {
    let servers = {
        let config = state
            .config
            .read()
            .map_err(|_| CommandError::Monitor("config lock poisoned".to_string()))?;
        config.servers.clone()
    };
    let mut monitors = state.monitors.lock().await;
    monitors.sync_servers(&servers).await;
    drop(monitors);
    let server_ids: std::collections::HashSet<&str> =
        servers.iter().map(|server| server.id.as_str()).collect();
    state
        .statuses
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .retain(|id, _| server_ids.contains(id.as_str()));
    Ok(())
}

fn emit_app_state_changed<R: tauri::Runtime>(app: &AppHandle<R>, view: &AppView) {
    let _ = app.emit("app-state-changed", view);
}

fn request_tray_rebuild<R: tauri::Runtime>(app: &AppHandle<R>) {
    // Tray management is implemented in a later task; emit a request that the
    // tray module can listen to once it lands.
    let _ = app.emit("tray-rebuild-requested", ());
}

pub(crate) fn preview_from_content(
    state: &Arc<AppState>,
    content: &str,
) -> Result<ImportPreview, CommandError> {
    let data = state.store.preview_import_text(content)?;
    let invalid = analyze_import_content(content)?;
    Ok(cache_preview(state, data, invalid))
}

fn cache_preview(
    state: &Arc<AppState>,
    data: ImportPreviewData,
    invalid: Vec<ImportIssue>,
) -> ImportPreview {
    let import_id = Uuid::new_v4();
    let valid_count = data.preview.valid_servers;
    {
        let mut previews = state
            .import_previews
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        expire_old_previews(&mut previews);
        previews.insert(
            import_id,
            PendingImport {
                data,
                created_at: Instant::now(),
            },
        );
    }
    ImportPreview {
        import_id,
        valid_count,
        invalid,
    }
}

fn analyze_import_content(content: &str) -> Result<Vec<ImportIssue>, CommandError> {
    let value: Value = serde_json::from_str(content).map_err(ConfigError::Json)?;

    let servers = value
        .get("servers")
        .and_then(Value::as_array)
        .cloned()
        .ok_or(ConfigError::UnsupportedFormat)?;
    Ok(collect_issues(servers))
}

fn collect_issues(values: Vec<Value>) -> Vec<ImportIssue> {
    values
        .into_iter()
        .enumerate()
        .filter_map(
            |(index, value)| match serde_json::from_value::<ServerConfig>(value) {
                Ok(server) => server.validate().err().map(|e| ImportIssue {
                    index,
                    reason: e.to_string(),
                }),
                Err(error) => Some(ImportIssue {
                    index,
                    reason: error.to_string(),
                }),
            },
        )
        .collect()
}

fn remove_preview(
    state: &Arc<AppState>,
    import_id: Uuid,
) -> Result<ImportPreviewData, CommandError> {
    let mut previews = state
        .import_previews
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    expire_old_previews(&mut previews);
    previews
        .remove(&import_id)
        .map(|pending| pending.data)
        .ok_or(CommandError::ImportPreviewNotFound)
}

fn expire_old_previews(previews: &mut HashMap<Uuid, PendingImport>) {
    let cutoff = Instant::now() - IMPORT_PREVIEW_TTL;
    previews.retain(|_, pending| pending.created_at > cutoff);
}

/// 比较两次服务器状态是否“实质相同”，忽略仅时间戳刷新的差异。
fn same_status(a: &ServerStatus, b: &ServerStatus) -> bool {
    a.connected == b.connected
        && a.active_count == b.active_count
        && a.sessions == b.sessions
        && a.server_version == b.server_version
        && a.error == b.error
}

// ------------------------------------------------------------------
// Monitor coordinator
// ------------------------------------------------------------------

pub async fn run_monitor_coordinator(
    state: Arc<AppState>,
    app: AppHandle,
    mut update_rx: mpsc::Receiver<MonitorUpdate>,
    monitor_started_at: DateTime<Utc>,
) {
    let mut coordinator = NotificationCoordinator::new();

    while let Some(update) = update_rx.recv().await {
        match update {
            MonitorUpdate::Status { server_id, status } => {
                let is_configured = state
                    .config
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .servers
                    .iter()
                    .any(|server| server.id == server_id);
                if !is_configured {
                    continue;
                }
                let changed = {
                    let mut statuses = state
                        .statuses
                        .write()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    let changed = statuses
                        .get(&server_id)
                        .map(|existing| !same_status(existing, &status))
                        .unwrap_or(true);
                    statuses.insert(server_id, status);
                    changed
                };
                // 状态没有实质变化（如仅 last_checked_at 刷新）时跳过重绘，
                // 避免高频 monitor 上报导致界面闪烁。
                if !changed {
                    continue;
                }
                state.revision.fetch_add(1, Ordering::SeqCst);
                let view = build_app_view(&state);
                emit_app_state_changed(&app, &view);
                request_tray_rebuild(&app);
            }
            MonitorUpdate::Event(event) => {
                let (server, settings) = {
                    let config = state
                        .config
                        .read()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    let server = config
                        .servers
                        .iter()
                        .find(|s| s.id == event.server_id)
                        .cloned();
                    let settings = config.settings.clone();
                    (server, settings)
                };

                if let Some(server) = server {
                    let now = Utc::now();
                    if let Some(pending) = coordinator.handle_event(
                        &event,
                        &server,
                        monitor_started_at,
                        now,
                        &settings,
                    ) {
                        let _ = coordinator.show(&app, pending);
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Backend;
    use tempfile::tempdir;

    fn sample_state() -> Arc<AppState> {
        let config = AppConfig {
            active_id: Some("s1".to_string()),
            servers: vec![
                ServerConfig::new(
                    "s1",
                    "Work",
                    "100.64.0.2",
                    3080,
                    "secret-token",
                    Backend::Dsh,
                ),
                ServerConfig::new(
                    "s2",
                    "Home",
                    "100.64.0.3",
                    3081,
                    "other-token",
                    Backend::Kimi,
                ),
            ],
            ..AppConfig::default()
        };
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().to_path_buf());
        let (tx, _) = mpsc::channel(4);
        Arc::new(AppState::new(
            config,
            store,
            MonitorManager::new(tx, PinnedSessions::default()),
            PinnedSessions::default(),
        ))
    }

    #[test]
    fn app_view_serializes_without_token() {
        let state = sample_state();
        let view = build_app_view(&state);
        let json = serde_json::to_string(&view).unwrap();

        assert_eq!(view.revision, 0);
        assert!(!json.contains("secret-token"));
        assert!(!json.contains("other-token"));
        assert!(json.contains("100.64.0.2"));
        assert!(json.contains("Work"));
    }

    #[test]
    fn app_view_revision_is_monotonic() {
        let state = sample_state();
        let first = build_app_view(&state);
        state.revision.fetch_add(1, Ordering::SeqCst);
        let second = build_app_view(&state);

        assert!(second.revision > first.revision);
    }

    #[test]
    fn app_view_omits_status_for_deleted_server() {
        let state = sample_state();
        state.statuses.write().unwrap().insert(
            "deleted".to_string(),
            ServerStatus {
                connected: true,
                active_count: 99,
                ..Default::default()
            },
        );

        let view = build_app_view(&state);
        assert!(!view.statuses.contains_key("deleted"));
    }

    #[test]
    fn server_for_edit_returns_token_only_for_exact_id() {
        let state = sample_state();

        let edit = get_server_for_edit_inner(&state, "s1").unwrap();
        assert_eq!(edit.token, "secret-token");

        let edit2 = get_server_for_edit_inner(&state, "s2").unwrap();
        assert_eq!(edit2.token, "other-token");

        assert!(get_server_for_edit_inner(&state, "missing").is_err());
    }

    #[tokio::test]
    async fn preview_import_hides_tokens() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().to_path_buf());
        let (tx, _) = mpsc::channel(4);
        let state = Arc::new(AppState::new(
            AppConfig::default(),
            store,
            MonitorManager::new(tx, PinnedSessions::default()),
            PinnedSessions::default(),
        ));

        let content = r#"{"schema":1,"servers":[{"id":"a","name":"A","host":"host","port":3080,"token":"top-secret","backend":"dsh"},{"name":"Bad","host":"http://host","port":0,"backend":"dsh"}]}"#;

        let preview = preview_from_content(&state, content).unwrap();
        assert_eq!(preview.valid_count, 1);
        assert_eq!(preview.invalid.len(), 1);

        // Response JSON must not contain the token.
        let json = serde_json::to_string(&preview).unwrap();
        assert!(!json.contains("top-secret"));
        assert!(!json.contains("token"));

        // The cached data must still contain the token for later application.
        let data = remove_preview(&state, preview.import_id).unwrap();
        let current = state.config.read().unwrap().clone();
        let new_config = state
            .store
            .apply_import(&current, data, ImportMode::Merge)
            .unwrap();
        assert_eq!(new_config.servers[0].token, "top-secret");
    }

    #[tokio::test]
    async fn apply_import_updates_config_and_removes_preview() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().to_path_buf());
        let (tx, _) = mpsc::channel(4);
        let state = Arc::new(AppState::new(
            AppConfig::default(),
            store,
            MonitorManager::new(tx, PinnedSessions::default()),
            PinnedSessions::default(),
        ));

        let content = r#"{"schema":1,"servers":[{"id":"a","name":"A","host":"host","port":3080,"token":"top-secret","backend":"dsh"}]}"#;

        let preview = preview_from_content(&state, content).unwrap();
        let import_id = preview.import_id;

        // apply_import needs an AppHandle; we cannot create one in a unit test,
        // so exercise the core logic directly.
        let data = remove_preview(&state, import_id).unwrap();
        let new_config = state
            .store
            .apply_import_and_save(data, ImportMode::Merge)
            .unwrap();
        *state.config.write().unwrap() = new_config;

        assert_eq!(state.config.read().unwrap().servers.len(), 1);
        assert_eq!(state.config.read().unwrap().servers[0].token, "top-secret");
        assert!(state
            .import_previews
            .lock()
            .unwrap()
            .get(&import_id)
            .is_none());
    }

    #[test]
    fn config_mutation_rejects_invalid_active_id() {
        let config = AppConfig {
            active_id: Some("missing".to_string()),
            servers: vec![ServerConfig::new(
                "s1",
                "Work",
                "100.64.0.2",
                3080,
                "token",
                Backend::Dsh,
            )],
            ..AppConfig::default()
        };
        assert!(validate_config(&config).is_err());
    }

    #[test]
    fn toggle_pin_marks_busy_session_and_unpin_removes_done_row() {
        let state = sample_state();
        state.statuses.write().unwrap().insert(
            "s1".to_string(),
            ServerStatus {
                connected: true,
                active_count: 2,
                sessions: vec![
                    crate::model::SessionSummary {
                        id: "run".to_string(),
                        title: "运行中".to_string(),
                        activity: None,
                        pinned: false,
                        done: false,
                    },
                    crate::model::SessionSummary {
                        id: "fin".to_string(),
                        title: "已完成".to_string(),
                        activity: Some("已完成，等你介入".to_string()),
                        pinned: true,
                        done: true,
                    },
                ],
                ..Default::default()
            },
        );
        // "fin" 的置顶标记来自 monitor；共享集合里先补上，模拟已置顶状态。
        state
            .pinned
            .write()
            .unwrap()
            .insert("s1|fin".to_string());

        // 置顶运行中的会话：打标并保留在列表。
        assert!(toggle_pin_inner(&state, "s1", "run"));
        // 读锁守卫不能经 let 引用绑定延长生命周期，否则同线程后续
        // toggle_pin_inner 拿写锁会自死锁；断言内联成语句即随语句释放。
        assert!(
            state.statuses.read().unwrap()["s1"]
                .sessions
                .iter()
                .find(|s| s.id == "run")
                .unwrap()
                .pinned
        );

        // 取消已完成会话的置顶：该行直接从列表移除。
        assert!(!toggle_pin_inner(&state, "s1", "fin"));
        assert!(
            state.statuses.read().unwrap()["s1"]
                .sessions
                .iter()
                .all(|s| s.id != "fin")
        );

        // 取消运行中会话的置顶：只去标记，行保留。
        assert!(!toggle_pin_inner(&state, "s1", "run"));
        assert!(
            !state.statuses.read().unwrap()["s1"]
                .sessions
                .iter()
                .find(|s| s.id == "run")
                .unwrap()
                .pinned
        );
        assert!(state.pinned.read().unwrap().is_empty());
    }

    #[test]
    fn purge_server_pins_removes_only_that_servers_keys() {
        let state = sample_state();
        {
            let mut pinned = state.pinned.write().unwrap();
            pinned.insert("s1|a".to_string());
            pinned.insert("s1|b".to_string());
            pinned.insert("s2|c".to_string());
        }
        purge_server_pins(&state, "s1");
        let pinned = state.pinned.read().unwrap();
        assert_eq!(pinned.len(), 1);
        assert!(pinned.contains("s2|c"));
    }

    #[test]
    fn same_status_ignores_last_checked_at_refresh() {
        let base = ServerStatus {
            connected: true,
            active_count: 2,
            sessions: Vec::new(),
            server_version: None,
            last_checked_at: Some(Utc::now()),
            error: None,
        };
        let refreshed = ServerStatus {
            last_checked_at: Some(Utc::now() + chrono::Duration::seconds(9)),
            ..base.clone()
        };
        assert!(same_status(&base, &refreshed));

        let busier = ServerStatus {
            active_count: 3,
            last_checked_at: Some(Utc::now()),
            ..base.clone()
        };
        assert!(!same_status(&base, &busier));

        let with_session = ServerStatus {
            sessions: vec![crate::model::SessionSummary {
                id: "s1".to_string(),
                title: "会话".to_string(),
                activity: None,
                pinned: false,
                done: false,
            }],
            ..base.clone()
        };
        assert!(!same_status(&base, &with_session));

        let errored = ServerStatus {
            error: Some("boom".to_string()),
            ..base.clone()
        };
        assert!(!same_status(&base, &errored));

        let upgraded = ServerStatus {
            server_version: Some("0.36.0".to_string()),
            ..base.clone()
        };
        assert!(!same_status(&base, &upgraded));
    }
}
