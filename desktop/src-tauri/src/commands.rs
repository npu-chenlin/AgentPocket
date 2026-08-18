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

use crate::config::{ConfigError, ConfigStore, ExportFormat, ImportMode, ImportPreviewData};
use crate::model::{
    AppConfig, AppView, Backend, DesktopSettings, ServerConfig, ServerForEdit, ServerStatus,
    ServerSummary, ValidationError,
};
use crate::monitor::{probe_backend as probe, MonitorManager, MonitorUpdate, ProbeError};
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
    pub import_previews: Mutex<HashMap<Uuid, PendingImport>>,
    pub explicit_exit: AtomicBool,
    pub revision: AtomicU64,
}

impl AppState {
    pub fn new(config: AppConfig, store: ConfigStore, monitors: MonitorManager) -> Self {
        Self {
            config: RwLock::new(config),
            statuses: RwLock::new(HashMap::new()),
            store,
            monitors: tokio::sync::Mutex::new(monitors),
            import_previews: Mutex::new(HashMap::new()),
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
    #[error("invalid export format")]
    InvalidExportFormat,
    #[error("invalid import id")]
    InvalidImportId,
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

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormatArg {
    Full,
    Android,
}

impl From<ExportFormatArg> for ExportFormat {
    fn from(arg: ExportFormatArg) -> Self {
        match arg {
            ExportFormatArg::Full => ExportFormat::Full,
            ExportFormatArg::Android => ExportFormat::Android,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportSourceKind {
    Full,
    Android,
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
    pub source_kind: ImportSourceKind,
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

    mutate_config(&state, &app, |config| {
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
    mutate_config(&state, &app, |config| {
        config.servers.retain(|s| s.id != id);
        if config.active_id.as_deref() == Some(&id) {
            config.active_id = config.servers.first().map(|s| s.id.clone());
        }
    })
    .await
}

#[tauri::command]
pub async fn set_active_server(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    id: Option<String>,
) -> Result<AppView, CommandError> {
    mutate_config(&state, &app, |config| {
        config.active_id = id
            .clone()
            .filter(|active_id| config.servers.iter().any(|server| &server.id == active_id));
    })
    .await
}

#[tauri::command]
pub async fn update_settings(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    settings: DesktopSettings,
) -> Result<AppView, CommandError> {
    mutate_config(&state, &app, |config| {
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
pub async fn preview_import(
    state: State<'_, Arc<AppState>>,
    path: String,
) -> Result<ImportPreview, CommandError> {
    preview_import_inner(&state, Path::new(&path)).await
}

#[tauri::command]
pub async fn apply_import(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    import_id: String,
    mode: ImportModeArg,
) -> Result<AppView, CommandError> {
    let import_id = Uuid::parse_str(&import_id).map_err(|_| CommandError::InvalidImportId)?;
    let data = remove_preview(&state, import_id)?;

    let current = {
        let config = state
            .config
            .read()
            .map_err(|_| CommandError::Config("config lock poisoned".to_string()))?;
        config.clone()
    };

    let new_config = state.store.apply_import(&current, data, mode.into())?;
    state.store.save(&new_config)?;

    {
        let mut config = state
            .config
            .write()
            .map_err(|_| CommandError::Config("config lock poisoned".to_string()))?;
        *config = new_config.clone();
    }

    if current.settings.autostart != new_config.settings.autostart {
        update_autostart(&app, new_config.settings.autostart).await?;
    }

    sync_monitors(&state).await?;

    state.revision.fetch_add(1, Ordering::SeqCst);
    let view = build_app_view(&state);
    emit_app_state_changed(&app, &view);
    request_tray_rebuild(&app);
    Ok(view)
}

#[tauri::command]
pub fn export_config(
    state: State<'_, Arc<AppState>>,
    path: String,
    format: ExportFormatArg,
) -> Result<(), CommandError> {
    let config = state
        .config
        .read()
        .map_err(|_| CommandError::Config("config lock poisoned".to_string()))?;
    state
        .store
        .export(&config, Path::new(&path), format.into())?;
    Ok(())
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
    F: FnOnce(&mut AppConfig),
{
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

    // Step 3: persist atomically before mutating memory.
    state.store.save(&new_config)?;

    // Step 4: replace in-memory config.
    {
        let mut config = state
            .config
            .write()
            .map_err(|_| CommandError::Config("config lock poisoned".to_string()))?;
        *config = new_config.clone();
    }

    // Step 5: update autostart if the setting changed.
    if old_config.settings.autostart != new_config.settings.autostart {
        update_autostart(app, new_config.settings.autostart).await?;
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

async fn sync_monitors(state: &Arc<AppState>) -> Result<(), CommandError> {
    let servers = {
        let config = state
            .config
            .read()
            .map_err(|_| CommandError::Monitor("config lock poisoned".to_string()))?;
        config.servers.clone()
    };
    let mut monitors = state.monitors.lock().await;
    monitors.sync_servers(&servers).await;
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

async fn preview_import_inner(
    state: &Arc<AppState>,
    path: &Path,
) -> Result<ImportPreview, CommandError> {
    let data = state.store.preview_import(path)?;
    let (source_kind, invalid) = analyze_import_file(path)?;

    expire_old_previews(
        &mut state
            .import_previews
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    );

    let import_id = Uuid::new_v4();
    state
        .import_previews
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(
            import_id,
            PendingImport {
                data,
                created_at: Instant::now(),
            },
        );

    Ok(ImportPreview {
        import_id,
        valid_count: state
            .import_previews
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&import_id)
            .map(|pending| pending.data.preview.valid_servers)
            .unwrap_or(0),
        invalid,
        source_kind,
    })
}

fn analyze_import_file(path: &Path) -> Result<(ImportSourceKind, Vec<ImportIssue>), CommandError> {
    let bytes = std::fs::read(path).map_err(ConfigError::Io)?;
    let value: Value = serde_json::from_slice(&bytes).map_err(ConfigError::Json)?;

    match value {
        Value::Array(values) => Ok((ImportSourceKind::Android, collect_issues(values))),
        Value::Object(_) => {
            let servers = value
                .get("servers")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            Ok((ImportSourceKind::Full, collect_issues(servers)))
        }
        _ => Err(ConfigError::UnsupportedFormat.into()),
    }
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
                if let Ok(mut statuses) = state.statuses.write() {
                    statuses.insert(server_id, status);
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
    use std::fs;
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
        Arc::new(AppState::new(config, store, MonitorManager::new(tx)))
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
            MonitorManager::new(tx),
        ));

        let import_path = dir.path().join("import.json");
        fs::write(
            &import_path,
            r#"[{"id":"a","name":"A","host":"host","port":3080,"token":"top-secret","backend":"dsh"},{"name":"Bad","host":"http://host","port":0,"backend":"dsh"}]"#,
        )
        .unwrap();

        let preview = preview_import_inner(&state, &import_path).await.unwrap();
        assert_eq!(preview.valid_count, 1);
        assert_eq!(preview.invalid.len(), 1);
        assert_eq!(preview.source_kind, ImportSourceKind::Android);

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
            MonitorManager::new(tx),
        ));

        let import_path = dir.path().join("import.json");
        fs::write(
            &import_path,
            r#"[{"id":"a","name":"A","host":"host","port":3080,"token":"top-secret","backend":"dsh"}]"#,
        )
        .unwrap();

        let preview = preview_import_inner(&state, &import_path).await.unwrap();
        let import_id = preview.import_id;

        // apply_import needs an AppHandle; we cannot create one in a unit test,
        // so exercise the core logic directly.
        let data = remove_preview(&state, import_id).unwrap();
        let current = state.config.read().unwrap().clone();
        let new_config = state
            .store
            .apply_import(&current, data, ImportMode::Merge)
            .unwrap();
        state.store.save(&new_config).unwrap();
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
}
