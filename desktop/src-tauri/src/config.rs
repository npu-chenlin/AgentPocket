use crate::model::{
    default_schema, AppConfig, Backend, DesktopSettings, ServerConfig, ServerSummary,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImportMode {
    Merge,
    Replace,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub valid_servers: usize,
    pub invalid_servers: usize,
    pub servers: Vec<ServerSummary>,
}

#[derive(Clone, Debug)]
pub struct ImportPreviewData {
    pub preview: ImportPreview,
    config: AppConfig,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportFormat {
    Full,
    Android,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoadOutcome {
    pub config: AppConfig,
    pub recovered_from_backup: Option<PathBuf>,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("configuration I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("configuration JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("configuration must be an object or an array")]
    UnsupportedFormat,
    #[error("unsupported configuration schema {0}; expected schema 1")]
    UnsupportedSchema(u32),
    #[error("import contains no valid servers")]
    NoValidServers,
    #[error("primary configuration is corrupt and no valid backup exists")]
    NoValidBackup,
    #[error("server configuration is invalid")]
    InvalidServer,
}

pub struct ConfigStore {
    app_dir: PathBuf,
    config_path: PathBuf,
}

impl ConfigStore {
    pub fn new(app_dir: PathBuf) -> Self {
        let config_path = app_dir.join("config.json");
        Self {
            app_dir,
            config_path,
        }
    }

    pub fn load(&self) -> Result<LoadOutcome, ConfigError> {
        if !self.config_path.exists() {
            return Ok(LoadOutcome {
                config: AppConfig::default(),
                recovered_from_backup: None,
            });
        }

        match fs::read(&self.config_path)
            .map_err(ConfigError::from)
            .and_then(|bytes| parse_config(&bytes)?.into_loadable_config())
        {
            Ok(config) => Ok(LoadOutcome {
                config,
                recovered_from_backup: None,
            }),
            Err(_) => self.load_latest_backup(),
        }
    }

    pub fn save(&self, config: &AppConfig) -> Result<(), ConfigError> {
        validate_schema(config.schema)?;
        if config
            .servers
            .iter()
            .any(|server| server.validate().is_err())
        {
            return Err(ConfigError::InvalidServer);
        }
        fs::create_dir_all(&self.app_dir)?;
        let bytes = serde_json::to_vec_pretty(config)?;
        atomic_write(&self.config_path, &bytes)
    }

    pub fn preview_import(&self, path: &Path) -> Result<ImportPreviewData, ConfigError> {
        let bytes = fs::read(path)?;
        let parsed = parse_config(&bytes)?;
        Ok(import_preview_from_parsed(parsed))
    }

    /// Parse an in-memory import payload (pasted JSON) without touching the
    /// filesystem, so the frontend can offer a copy/paste import flow.
    pub fn preview_import_text(&self, content: &str) -> Result<ImportPreviewData, ConfigError> {
        let parsed = parse_config(content.as_bytes())?;
        Ok(import_preview_from_parsed(parsed))
    }

    pub fn apply_import(
        &self,
        current: &AppConfig,
        data: ImportPreviewData,
        mode: ImportMode,
    ) -> Result<AppConfig, ConfigError> {
        if data.config.servers.is_empty() {
            return Err(ConfigError::NoValidServers);
        }
        self.backup_primary()?;

        match mode {
            ImportMode::Merge => {
                let mut merged = current.clone();
                for imported in data.config.servers {
                    if let Some(index) = merged
                        .servers
                        .iter()
                        .position(|server| server.id == imported.id)
                    {
                        merged.servers[index] = imported;
                    } else {
                        merged.servers.push(imported);
                    }
                }
                Ok(merged)
            }
            ImportMode::Replace => {
                let mut replacement = data.config;
                if replacement.active_id.as_ref().is_none_or(|active_id| {
                    !replacement
                        .servers
                        .iter()
                        .any(|server| &server.id == active_id)
                }) {
                    replacement.active_id =
                        replacement.servers.first().map(|server| server.id.clone());
                }
                Ok(replacement)
            }
        }
    }

    pub fn export(
        &self,
        config: &AppConfig,
        path: &Path,
        format: ExportFormat,
    ) -> Result<(), ConfigError> {
        let text = self.export_text(config, format)?;
        atomic_write(path, text.as_bytes())
    }

    /// Serialize the config (or the Android-compatible server list) to a
    /// pretty JSON string so the frontend can show it for copy/paste export.
    pub fn export_text(
        &self,
        config: &AppConfig,
        format: ExportFormat,
    ) -> Result<String, ConfigError> {
        validate_schema(config.schema)?;
        match format {
            ExportFormat::Full => Ok(serde_json::to_string_pretty(config)?),
            ExportFormat::Android => {
                let servers: Vec<AndroidServerRef<'_>> =
                    config.servers.iter().map(AndroidServerRef::from).collect();
                Ok(serde_json::to_string_pretty(&servers)?)
            }
        }
    }

    fn load_latest_backup(&self) -> Result<LoadOutcome, ConfigError> {
        let backup_dir = self.app_dir.join("backups");
        let paths = backup_paths(&backup_dir)?;
        for path in paths.into_iter().rev() {
            if let Ok(bytes) = fs::read(&path) {
                if let Ok(config) =
                    parse_config(&bytes).and_then(ParsedConfig::into_loadable_config)
                {
                    return Ok(LoadOutcome {
                        config,
                        recovered_from_backup: Some(path),
                    });
                }
            }
        }
        Err(ConfigError::NoValidBackup)
    }

    fn backup_primary(&self) -> Result<(), ConfigError> {
        if !self.config_path.exists() {
            return Ok(());
        }
        let backup_dir = self.app_dir.join("backups");
        fs::create_dir_all(&backup_dir)?;

        let backup_path = next_backup_path(&backup_dir, Utc::now());
        fs::copy(&self.config_path, backup_path)?;

        let paths = backup_paths(&backup_dir)?;
        let remove_count = paths.len().saturating_sub(5);
        for path in paths.into_iter().take(remove_count) {
            fs::remove_file(path)?;
        }
        Ok(())
    }
}

fn import_preview_from_parsed(parsed: ParsedConfig) -> ImportPreviewData {
    let servers = parsed
        .config
        .servers
        .iter()
        .map(ServerSummary::from)
        .collect();
    ImportPreviewData {
        preview: ImportPreview {
            valid_servers: parsed.config.servers.len(),
            invalid_servers: parsed.invalid_servers,
            servers,
        },
        config: parsed.config,
    }
}

fn next_backup_path(backup_dir: &Path, now: DateTime<Utc>) -> PathBuf {
    let timestamp = now.format("%Y%m%d-%H%M%S");
    (0_u32..)
        .map(|suffix| {
            if suffix == 0 {
                backup_dir.join(format!("config-{timestamp}.json"))
            } else {
                backup_dir.join(format!("config-{timestamp}-{suffix:03}.json"))
            }
        })
        .find(|path| !path.exists())
        .expect("an unused backup suffix must exist")
}

struct ParsedConfig {
    config: AppConfig,
    invalid_servers: usize,
    server_entries: usize,
}

impl ParsedConfig {
    fn into_loadable_config(self) -> Result<AppConfig, ConfigError> {
        if self.server_entries > 0 && self.config.servers.is_empty() {
            Err(ConfigError::NoValidServers)
        } else {
            Ok(self.config)
        }
    }
}

fn validate_schema(schema: u32) -> Result<(), ConfigError> {
    if schema == 1 {
        Ok(())
    } else {
        Err(ConfigError::UnsupportedSchema(schema))
    }
}

fn parse_config(bytes: &[u8]) -> Result<ParsedConfig, ConfigError> {
    let value: Value = serde_json::from_slice(bytes)?;
    match value {
        Value::Array(values) => {
            let server_entries = values.len();
            let (servers, invalid_servers) = parse_servers(values);
            Ok(ParsedConfig {
                config: AppConfig {
                    active_id: servers.first().map(|server| server.id.clone()),
                    servers,
                    ..AppConfig::default()
                },
                invalid_servers,
                server_entries,
            })
        }
        Value::Object(_) => {
            let full: FullConfig = serde_json::from_value(value)?;
            validate_schema(full.schema)?;
            let server_entries = full.servers.len();
            let (servers, invalid_servers) = parse_servers(full.servers);
            let active_id = full
                .active_id
                .filter(|active_id| servers.iter().any(|server| &server.id == active_id));
            Ok(ParsedConfig {
                config: AppConfig {
                    schema: full.schema,
                    active_id,
                    servers,
                    settings: full.settings,
                },
                invalid_servers,
                server_entries,
            })
        }
        _ => Err(ConfigError::UnsupportedFormat),
    }
}

fn parse_servers(values: Vec<Value>) -> (Vec<ServerConfig>, usize) {
    let total = values.len();
    let servers = values
        .into_iter()
        .filter_map(|value| serde_json::from_value::<AndroidServer>(value).ok())
        .map(ServerConfig::from)
        .filter(|server| server.validate().is_ok())
        .collect::<Vec<_>>();
    let invalid = total - servers.len();
    (servers, invalid)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FullConfig {
    #[serde(default = "default_schema")]
    schema: u32,
    active_id: Option<String>,
    servers: Vec<Value>,
    #[serde(default)]
    settings: DesktopSettings,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AndroidServer {
    #[serde(default)]
    id: String,
    name: String,
    host: String,
    port: u16,
    #[serde(default)]
    token: String,
    backend: Backend,
}

impl From<AndroidServer> for ServerConfig {
    fn from(server: AndroidServer) -> Self {
        ServerConfig::new(
            if server.id.is_empty() {
                Uuid::new_v4().to_string()
            } else {
                server.id
            },
            server.name,
            server.host,
            server.port,
            server.token,
            server.backend,
        )
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AndroidServerRef<'a> {
    id: &'a str,
    name: &'a str,
    host: &'a str,
    port: u16,
    token: &'a str,
    backend: Backend,
}

impl<'a> From<&'a ServerConfig> for AndroidServerRef<'a> {
    fn from(server: &'a ServerConfig) -> Self {
        Self {
            id: &server.id,
            name: &server.name,
            host: &server.host,
            port: server.port,
            token: &server.token,
            backend: server.backend,
        }
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp_path = path.with_file_name(format!(
        "{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("config.json")
    ));
    let mut file = File::create(&tmp_path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600))?;
    }
    replace_file(&tmp_path, path)?;
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;

    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
    }

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();

    // MoveFileExW can replace an existing file on Windows. The temp file is in
    // the same directory to avoid cross-volume moves. Atomicity still depends
    // on the destination filesystem honoring same-volume rename semantics.
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn backup_sort_key(name: &str) -> (String, u32) {
    let stem = name
        .strip_prefix("config-")
        .and_then(|stem| stem.strip_suffix(".json"))
        .unwrap_or(name);
    // Plain backups are named `config-YYYYMMDD-HHMMSS.json` (15-char timestamp
    // stem); same-second collisions append a monotonic `-NNN` suffix. Lexically
    // `-001` sorts before `.`, so a raw string comparison would rank the plain
    // (oldest) name *after* its suffixed siblings. Treat the plain name as
    // suffix 0 instead so filename order matches creation order.
    if stem.len() > 15 {
        if let Some(suffix) = stem[15..].strip_prefix('-') {
            if let Ok(suffix) = suffix.parse::<u32>() {
                return (stem[..15].to_string(), suffix);
            }
        }
    }
    (stem.to_string(), 0)
}

fn backup_paths(backup_dir: &Path) -> Result<Vec<PathBuf>, ConfigError> {
    if !backup_dir.exists() {
        return Ok(Vec::new());
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(backup_dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if let Some(name) = name
            .to_str()
            .filter(|name| name.starts_with("config-") && name.ends_with(".json"))
        {
            entries.push((
                entry.metadata()?.modified()?,
                name.to_string(),
                entry.path(),
            ));
        }
    }
    entries.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| backup_sort_key(&left.1).cmp(&backup_sort_key(&right.1)))
    });
    Ok(entries.into_iter().map(|(_, _, path)| path).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AppConfig, Backend, ServerConfig};
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    fn server(id: &str, name: &str) -> ServerConfig {
        ServerConfig::new(id, name, "host", 3080, "secret", Backend::Dsh)
    }

    #[test]
    fn loads_android_array_and_assigns_missing_ids() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("config.json"),
            r#"[{"name":"Work","host":"100.64.0.2","port":3080,"token":"secret","backend":"dsh"}]"#,
        )
        .unwrap();

        let outcome = ConfigStore::new(dir.path().to_path_buf()).load().unwrap();

        assert_eq!(outcome.config.servers.len(), 1);
        assert!(!outcome.config.servers[0].id.is_empty());
        assert_eq!(outcome.config.servers[0].name, "Work");
    }

    #[test]
    fn full_import_keeps_valid_servers_and_counts_malformed_items() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().to_path_buf());
        let import_path = dir.path().join("full-import.json");
        fs::write(
            &import_path,
            r#"{"schema":1,"activeId":"valid","servers":[{"id":"valid","name":"Work","host":"host","port":3080,"token":"","backend":"dsh"},{"id":"bad","name":"Bad","host":"host","port":"not-a-port","backend":"dsh"}],"settings":{"startHidden":true,"autostart":false,"notifications":true}}"#,
        )
        .unwrap();

        let data = store.preview_import(&import_path).unwrap();

        assert_eq!(data.preview.valid_servers, 1);
        assert_eq!(data.preview.invalid_servers, 1);
        assert_eq!(data.preview.servers[0].id, "valid");
    }

    #[test]
    fn full_import_missing_backend_counts_invalid_and_keeps_valid_for_replace() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().to_path_buf());
        let import_path = dir.path().join("full-mixed.json");
        fs::write(
            &import_path,
            r#"{"schema":1,"activeId":"valid","servers":[{"id":"valid","name":"Work","host":"host","port":3080,"token":"","backend":"dsh"},{"id":"broken","name":"Broken","host":"host","port":3080}],"settings":{"startHidden":true,"autostart":false,"notifications":true}}"#,
        )
        .unwrap();

        let data = store.preview_import(&import_path).unwrap();

        assert_eq!(data.preview.valid_servers, 1);
        assert_eq!(data.preview.invalid_servers, 1);
        let replaced = store
            .apply_import(&AppConfig::default(), data, ImportMode::Replace)
            .unwrap();
        assert_eq!(replaced.servers.len(), 1);
        assert_eq!(replaced.servers[0].id, "valid");
    }

    #[test]
    fn full_import_missing_schema_defaults_to_one() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().to_path_buf());
        let import_path = dir.path().join("missing-schema.json");
        fs::write(
            &import_path,
            r#"{"activeId":null,"servers":[],"settings":{"startHidden":true,"autostart":false,"notifications":true}}"#,
        )
        .unwrap();

        let data = store.preview_import(&import_path).unwrap();

        assert_eq!(data.config.schema, 1);
    }

    #[test]
    fn full_import_rejects_schema_zero() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().to_path_buf());
        let import_path = dir.path().join("schema-zero.json");
        fs::write(
            &import_path,
            r#"{"schema":0,"activeId":null,"servers":[],"settings":{"startHidden":true,"autostart":false,"notifications":true}}"#,
        )
        .unwrap();

        assert!(store.preview_import(&import_path).is_err());
    }

    #[test]
    fn full_import_rejects_unsupported_schema() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().to_path_buf());
        let import_path = dir.path().join("future-schema.json");
        fs::write(
            &import_path,
            r#"{"schema":2,"activeId":null,"servers":[],"settings":{"startHidden":true,"autostart":false,"notifications":true}}"#,
        )
        .unwrap();

        assert!(store.preview_import(&import_path).is_err());
    }

    #[test]
    fn save_rejects_unsupported_schema_without_overwriting_existing() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().to_path_buf());
        let original = AppConfig {
            servers: vec![server("id", "Original")],
            ..AppConfig::default()
        };
        store.save(&original).unwrap();
        let unsupported = AppConfig {
            schema: 2,
            servers: vec![server("id", "Unsupported")],
            ..AppConfig::default()
        };

        assert!(store.save(&unsupported).is_err());
        assert_eq!(store.load().unwrap().config, original);
    }

    #[test]
    fn export_rejects_unsupported_schema_without_overwriting_existing() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().to_path_buf());
        let export_path = dir.path().join("export.json");
        fs::write(&export_path, b"original bytes").unwrap();
        let unsupported = AppConfig {
            schema: 2,
            servers: vec![server("id", "Unsupported")],
            ..AppConfig::default()
        };

        assert!(store
            .export(&unsupported, &export_path, ExportFormat::Full)
            .is_err());
        assert_eq!(fs::read(export_path).unwrap(), b"original bytes");
    }

    #[test]
    fn merge_updates_matching_id_and_appends_new_server() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().to_path_buf());
        let current = AppConfig {
            active_id: Some("stable".into()),
            servers: vec![server("stable", "Old"), server("untouched", "Keep")],
            ..AppConfig::default()
        };
        let import_path = dir.path().join("import.json");
        fs::write(
            &import_path,
            r#"[{"id":"stable","name":"Updated","host":"host","port":3080,"token":"new","backend":"dsh"},{"id":"new","name":"New","host":"new-host","port":3081,"token":"","backend":"kimi"}]"#,
        )
        .unwrap();
        let data = store.preview_import(&import_path).unwrap();

        let merged = store
            .apply_import(&current, data, ImportMode::Merge)
            .unwrap();

        assert_eq!(merged.active_id.as_deref(), Some("stable"));
        assert_eq!(merged.servers.len(), 3);
        assert_eq!(merged.servers[0].name, "Updated");
        assert_eq!(merged.servers[1].name, "Keep");
        assert_eq!(merged.servers[2].id, "new");
    }

    #[test]
    fn replace_rejects_zero_valid_servers() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().to_path_buf());
        let import_path = dir.path().join("bad-import.json");
        fs::write(
            &import_path,
            r#"[{"name":"Bad","host":"http://host","port":0,"backend":"dsh"}]"#,
        )
        .unwrap();
        let data = store.preview_import(&import_path).unwrap();

        assert_eq!(data.preview.valid_servers, 0);
        assert!(store
            .apply_import(&AppConfig::default(), data, ImportMode::Replace)
            .is_err());
    }

    #[test]
    fn save_round_trips_and_creates_mode_0600_on_unix() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().to_path_buf());
        let config = AppConfig {
            active_id: Some("id".into()),
            servers: vec![server("id", "Work")],
            ..AppConfig::default()
        };

        store.save(&config).unwrap();
        let loaded = store.load().unwrap();

        assert_eq!(loaded.config, config);
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(dir.path().join("config.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn save_replaces_existing_config_on_second_write() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().to_path_buf());
        let first = AppConfig {
            servers: vec![server("id", "First")],
            ..AppConfig::default()
        };
        let second = AppConfig {
            servers: vec![server("id", "Second")],
            ..AppConfig::default()
        };

        store.save(&first).unwrap();
        store.save(&second).unwrap();

        assert_eq!(store.load().unwrap().config, second);
    }

    #[test]
    fn export_replaces_existing_file_on_second_write() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().to_path_buf());
        let path = dir.path().join("export.json");
        let first = AppConfig {
            servers: vec![server("id", "First")],
            ..AppConfig::default()
        };
        let second = AppConfig {
            servers: vec![server("id", "Second")],
            ..AppConfig::default()
        };

        store.export(&first, &path, ExportFormat::Full).unwrap();
        store.export(&second, &path, ExportFormat::Full).unwrap();
        let exported: AppConfig = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();

        assert_eq!(exported, second);
    }

    #[test]
    fn replace_file_replaces_existing_destination() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source.tmp");
        let destination = dir.path().join("destination.json");
        fs::write(&source, b"new").unwrap();
        fs::write(&destination, b"old").unwrap();

        replace_file(&source, &destination).unwrap();

        assert_eq!(fs::read(destination).unwrap(), b"new");
        assert!(!source.exists());
    }

    #[test]
    fn corrupted_primary_recovers_latest_backup_without_overwriting_primary() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().to_path_buf());
        let config = AppConfig {
            servers: vec![server("id", "Recovered")],
            ..AppConfig::default()
        };
        store.save(&config).unwrap();
        let import_path = dir.path().join("import.json");
        fs::write(
            &import_path,
            r#"[{"name":"Other","host":"host","port":3080,"backend":"dsh"}]"#,
        )
        .unwrap();
        let data = store.preview_import(&import_path).unwrap();
        store
            .apply_import(&config, data, ImportMode::Merge)
            .unwrap();
        fs::write(dir.path().join("config.json"), b"bad bytes").unwrap();

        let loaded = store.load().unwrap();

        assert_eq!(loaded.config.servers[0].name, "Recovered");
        assert!(loaded.recovered_from_backup.is_some());
        assert_eq!(
            fs::read(dir.path().join("config.json")).unwrap(),
            b"bad bytes"
        );
    }

    #[test]
    fn all_invalid_primary_servers_recover_latest_backup() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().to_path_buf());
        let recoverable = AppConfig {
            servers: vec![server("id", "Recovered")],
            ..AppConfig::default()
        };
        store.save(&recoverable).unwrap();
        let import_path = dir.path().join("import.json");
        fs::write(
            &import_path,
            r#"[{"name":"Other","host":"host","port":3080,"backend":"dsh"}]"#,
        )
        .unwrap();
        let data = store.preview_import(&import_path).unwrap();
        store
            .apply_import(&recoverable, data, ImportMode::Merge)
            .unwrap();
        fs::write(
            dir.path().join("config.json"),
            r#"{"schema":1,"activeId":null,"servers":[{"id":"bad","name":"Bad","host":"http://host","port":3080,"backend":"dsh"}],"settings":{"startHidden":true,"autostart":false,"notifications":true}}"#,
        )
        .unwrap();

        let loaded = store.load().unwrap();

        assert_eq!(loaded.config, recoverable);
        assert!(loaded.recovered_from_backup.is_some());
    }

    #[test]
    fn empty_primary_config_loads_without_backup_recovery() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().to_path_buf());
        fs::write(
            dir.path().join("config.json"),
            r#"{"schema":1,"activeId":null,"servers":[],"settings":{"startHidden":true,"autostart":false,"notifications":true}}"#,
        )
        .unwrap();

        let loaded = store.load().unwrap();

        assert!(loaded.config.servers.is_empty());
        assert!(loaded.recovered_from_backup.is_none());
    }

    #[test]
    fn backup_name_collision_uses_monotonic_suffix() {
        let dir = tempdir().unwrap();
        let now = chrono::DateTime::parse_from_rfc3339("2026-08-18T10:50:49Z")
            .unwrap()
            .with_timezone(&Utc);
        let first = next_backup_path(dir.path(), now);
        fs::write(&first, b"first").unwrap();

        let second = next_backup_path(dir.path(), now);

        assert_eq!(first.file_name().unwrap(), "config-20260818-105049.json");
        assert_eq!(
            second.file_name().unwrap(),
            "config-20260818-105049-001.json"
        );
    }

    #[test]
    fn same_second_backups_sort_plain_name_before_suffixes() {
        let dir = tempdir().unwrap();
        let now = chrono::DateTime::parse_from_rfc3339("2026-08-18T10:50:49Z")
            .unwrap()
            .with_timezone(&Utc);
        let first = next_backup_path(dir.path(), now);
        fs::write(&first, b"first").unwrap();
        let second = next_backup_path(dir.path(), now);
        fs::write(&second, b"second").unwrap();
        let third = next_backup_path(dir.path(), now);
        fs::write(&third, b"third").unwrap();

        // Force identical mtimes so only filename order can disambiguate:
        // creation order is plain, -001, -002.
        let epoch = std::time::UNIX_EPOCH;
        for path in [&first, &second, &third] {
            fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(epoch))
                .unwrap();
        }

        let paths = backup_paths(dir.path()).unwrap();

        assert_eq!(
            paths
                .iter()
                .map(|path| path.file_name().unwrap().to_str().unwrap())
                .collect::<Vec<_>>(),
            vec![
                "config-20260818-105049.json",
                "config-20260818-105049-001.json",
                "config-20260818-105049-002.json",
            ]
        );
    }

    #[test]
    fn backup_rotation_uses_modified_time_before_filename() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().to_path_buf());
        let config = AppConfig {
            servers: vec![server("id", "Work")],
            ..AppConfig::default()
        };
        store.save(&config).unwrap();
        let backup_dir = dir.path().join("backups");
        fs::create_dir_all(&backup_dir).unwrap();
        let old_future_named = backup_dir.join("config-99999999-999999.json");
        fs::write(&old_future_named, b"old").unwrap();
        fs::File::options()
            .write(true)
            .open(&old_future_named)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH))
            .unwrap();
        for index in 0..4 {
            fs::write(
                backup_dir.join(format!("config-20200101-00000{index}.json")),
                b"newer",
            )
            .unwrap();
        }
        let import_path = dir.path().join("import.json");
        fs::write(
            &import_path,
            r#"[{"name":"Other","host":"host","port":3080,"backend":"dsh"}]"#,
        )
        .unwrap();

        let data = store.preview_import(&import_path).unwrap();
        store
            .apply_import(&config, data, ImportMode::Merge)
            .unwrap();

        assert!(!old_future_named.exists());
        assert_eq!(fs::read_dir(backup_dir).unwrap().count(), 5);
    }

    #[test]
    fn backup_rotation_keeps_five_files() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().to_path_buf());
        let config = AppConfig {
            servers: vec![server("id", "Work")],
            ..AppConfig::default()
        };
        store.save(&config).unwrap();
        let import_path = dir.path().join("import.json");
        fs::write(
            &import_path,
            r#"[{"name":"Other","host":"host","port":3080,"backend":"dsh"}]"#,
        )
        .unwrap();

        for _ in 0..6 {
            let data = store.preview_import(&import_path).unwrap();
            store
                .apply_import(&config, data, ImportMode::Merge)
                .unwrap();
        }

        assert_eq!(fs::read_dir(dir.path().join("backups")).unwrap().count(), 5);
    }

    #[test]
    fn android_export_preserves_server_ids() {
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().to_path_buf());
        let path = dir.path().join("android.json");
        let config = AppConfig {
            servers: vec![server("stable-id", "Work")],
            ..AppConfig::default()
        };

        store.export(&config, &path, ExportFormat::Android).unwrap();
        let exported: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();

        assert_eq!(exported[0]["id"], "stable-id");
    }

    #[test]
    fn preview_import_text_parses_pasted_json() {
        let store = ConfigStore::new(tempdir().unwrap().path().to_path_buf());
        let data = store
            .preview_import_text(
                r#"[{"id":"s1","name":"Work","host":"host","port":3080,"token":"secret","backend":"dsh"}]"#,
            )
            .unwrap();

        assert_eq!(data.preview.valid_servers, 1);
        assert_eq!(data.preview.invalid_servers, 0);
        assert_eq!(data.preview.servers[0].name, "Work");
    }

    #[test]
    fn preview_import_text_counts_malformed_items() {
        let store = ConfigStore::new(tempdir().unwrap().path().to_path_buf());
        let data = store
            .preview_import_text(
                r#"{"schema":1,"servers":[{"id":"ok","name":"Work","host":"host","port":3080,"token":"","backend":"kimi"},{"id":"bad","name":"Bad","port":"oops","backend":"dsh"}]}"#,
            )
            .unwrap();

        assert_eq!(data.preview.valid_servers, 1);
        assert_eq!(data.preview.invalid_servers, 1);
    }

    #[test]
    fn export_text_serializes_both_formats_without_writing() {
        let store = ConfigStore::new(tempdir().unwrap().path().to_path_buf());
        let config = AppConfig {
            servers: vec![server("stable-id", "Work")],
            ..AppConfig::default()
        };

        let full: serde_json::Value =
            serde_json::from_str(&store.export_text(&config, ExportFormat::Full).unwrap()).unwrap();
        assert_eq!(full["servers"][0]["id"], "stable-id");
        assert_eq!(full["schema"], 1);

        let android: serde_json::Value =
            serde_json::from_str(&store.export_text(&config, ExportFormat::Android).unwrap())
                .unwrap();
        assert_eq!(android[0]["id"], "stable-id");
    }
}
