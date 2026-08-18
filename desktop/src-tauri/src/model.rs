use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Kimi,
    Dsh,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfig {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    #[serde(default)]
    pub token: String,
    pub backend: Backend,
}

impl ServerConfig {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        host: impl Into<String>,
        port: u16,
        token: impl Into<String>,
        backend: Backend,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            host: host.into(),
            port,
            token: token.into(),
            backend,
        }
    }

    pub fn base_url(&self) -> Result<Url, ValidationError> {
        self.validate()?;
        Url::parse(&format!("http://{}:{}", self.host, self.port))
            .map_err(|_| ValidationError::InvalidHost)
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.port == 0 {
            return Err(ValidationError::InvalidPort);
        }
        if self.host.is_empty()
            || self.host.chars().any(char::is_whitespace)
            || self.host.contains(':')
            || self.host.contains('/')
            || self.host.contains("//")
        {
            return Err(ValidationError::InvalidHost);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopSettings {
    pub start_hidden: bool,
    pub autostart: bool,
    pub notifications: bool,
}

impl Default for DesktopSettings {
    fn default() -> Self {
        Self {
            start_hidden: true,
            autostart: false,
            notifications: true,
        }
    }
}

pub(crate) fn default_schema() -> u32 {
    1
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    #[serde(default = "default_schema")]
    pub schema: u32,
    pub active_id: Option<String>,
    pub servers: Vec<ServerConfig>,
    #[serde(default)]
    pub settings: DesktopSettings,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema: 1,
            active_id: None,
            servers: Vec::new(),
            settings: DesktopSettings::default(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerSummary {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub backend: Backend,
}

impl From<&ServerConfig> for ServerSummary {
    fn from(server: &ServerConfig) -> Self {
        Self {
            id: server.id.clone(),
            name: server.name.clone(),
            host: server.host.clone(),
            port: server.port,
            backend: server.backend,
        }
    }
}

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("host must not contain a scheme, path, whitespace, or port")]
    InvalidHost,
    #[error("port must be greater than zero")]
    InvalidPort,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_server_fields() {
        let ok = ServerConfig::new("id", "Work", "100.64.0.2", 3080, "", Backend::Dsh);
        assert!(ok.validate().is_ok());
        assert!(
            ServerConfig::new("id", "Work", "http://host", 3080, "", Backend::Dsh)
                .validate()
                .is_err()
        );
        assert!(
            ServerConfig::new("id", "Work", "host:3080", 3080, "", Backend::Dsh)
                .validate()
                .is_err()
        );
        assert!(ServerConfig::new("id", "Work", "host", 0, "", Backend::Dsh)
            .validate()
            .is_err());
    }

    #[test]
    fn app_config_missing_schema_deserializes_as_one() {
        let config: AppConfig = serde_json::from_str(
            r#"{"activeId":null,"servers":[],"settings":{"startHidden":true,"autostart":false,"notifications":true}}"#,
        )
        .unwrap();

        assert_eq!(config.schema, 1);
    }

    #[test]
    fn summary_never_exposes_token() {
        let server = ServerConfig::new("id", "Work", "host", 3080, "secret", Backend::Dsh);
        let json = serde_json::to_string(&ServerSummary::from(&server)).unwrap();
        assert!(!json.contains("secret"));
        assert!(!json.contains("token"));
    }
}
