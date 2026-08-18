use crate::model::{AppConfig, Backend, ServerConfig, ServerSummary};
use chrono::{Duration, Utc};
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
            .and_then(|bytes| parse_config(&bytes).map(|parsed| parsed.config))
        {
            Ok(config) => Ok(LoadOutcome {
                config,
                recovered_from_backup: None,
            }),
            Err(_) => self.load_latest_backup(),
        }
    }

    pub fn save(&self, config: &AppConfig) -> Result<(), ConfigError> {
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
        let servers = parsed
            .config
            .servers
            .iter()
            .map(ServerSummary::from)
            .collect();
        Ok(ImportPreviewData {
            preview: ImportPreview {
                valid_servers: parsed.config.servers.len(),
                invalid_servers: parsed.invalid_servers,
                servers,
            },
            config: parsed.config,
        })
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
        let bytes = match format {
            ExportFormat::Full => serde_json::to_vec_pretty(config)?,
            ExportFormat::Android => {
                let servers: Vec<AndroidServerRef<'_>> =
                    config.servers.iter().map(AndroidServerRef::from).collect();
                serde_json::to_vec_pretty(&servers)?
            }
        };
        atomic_write(path, &bytes)
    }

    fn load_latest_backup(&self) -> Result<LoadOutcome, ConfigError> {
        let backup_dir = self.app_dir.join("backups");
        let mut paths = backup_paths(&backup_dir)?;
        paths.sort();
        for path in paths.into_iter().rev() {
            if let Ok(bytes) = fs::read(&path) {
                if let Ok(parsed) = parse_config(&bytes) {
                    return Ok(LoadOutcome {
                        config: parsed.config,
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

        let now = Utc::now();
        let backup_path = (0_i64..)
            .map(|offset| {
                let timestamp = now + Duration::seconds(offset);
                backup_dir.join(format!("config-{}.json", timestamp.format("%Y%m%d-%H%M%S")))
            })
            .find(|path| !path.exists())
            .expect("an unused backup timestamp must exist");
        fs::copy(&self.config_path, backup_path)?;

        let mut paths = backup_paths(&backup_dir)?;
        paths.sort();
        let remove_count = paths.len().saturating_sub(5);
        for path in paths.into_iter().take(remove_count) {
            fs::remove_file(path)?;
        }
        Ok(())
    }
}

struct ParsedConfig {
    config: AppConfig,
    invalid_servers: usize,
}

fn parse_config(bytes: &[u8]) -> Result<ParsedConfig, ConfigError> {
    let value: Value = serde_json::from_slice(bytes)?;
    match value {
        Value::Array(values) => {
            let (servers, invalid_servers) = parse_servers(values);
            Ok(ParsedConfig {
                config: AppConfig {
                    active_id: servers.first().map(|server| server.id.clone()),
                    servers,
                    ..AppConfig::default()
                },
                invalid_servers,
            })
        }
        Value::Object(_) => {
            let mut config: AppConfig = serde_json::from_value(value)?;
            let original_count = config.servers.len();
            config.servers.retain(|server| server.validate().is_ok());
            for server in &mut config.servers {
                if server.id.is_empty() {
                    server.id = Uuid::new_v4().to_string();
                }
            }
            if config.active_id.as_ref().is_some_and(|active_id| {
                !config.servers.iter().any(|server| &server.id == active_id)
            }) {
                config.active_id = None;
            }
            Ok(ParsedConfig {
                invalid_servers: original_count - config.servers.len(),
                config,
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
    fs::rename(&tmp_path, path)?;
    Ok(())
}

fn backup_paths(backup_dir: &Path) -> Result<Vec<PathBuf>, ConfigError> {
    if !backup_dir.exists() {
        return Ok(Vec::new());
    }
    let paths = fs::read_dir(backup_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("config-") && name.ends_with(".json"))
        })
        .collect();
    Ok(paths)
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
}
