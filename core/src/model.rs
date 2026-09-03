use std::collections::HashMap;

use chrono::{DateTime, Utc};
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

/// 手动登记的 mesh peer（存于 DesktopSettings，GUI 侧维护）。
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshPeerEntry {
    pub name: String,
    pub host: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopSettings {
    pub start_hidden: bool,
    pub autostart: bool,
    pub notifications: bool,
    /// 手动 peer 列表；旧配置无此字段时反序列化为空（字段级 serde default）。
    #[serde(default)]
    pub mesh_peers: Vec<MeshPeerEntry>,
}

impl Default for DesktopSettings {
    fn default() -> Self {
        Self {
            start_hidden: true,
            autostart: false,
            notifications: true,
            mesh_peers: Vec::new(),
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

/// 正在运行的会话摘要（忙碌会话 + 已置顶的完成会话；标题缺失时用占位文案）。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    /// 当前正在执行的活动，如 "Bash · git push"；未知时为 None。
    pub activity: Option<String>,
    /// 用户置顶的会话：完成后仍留在列表中提醒介入。
    pub pinned: bool,
    /// 置顶会话已全部完成（主回合与后台任务都结束），等待用户介入。
    pub done: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
    pub connected: bool,
    pub active_count: u32,
    /// 正在运行的会话列表，按标题排序以保证稳定。
    pub sessions: Vec<SessionSummary>,
    /// 服务端版本号（kimi 从 meta 接口获取；dsh 暂无此信息）。
    pub server_version: Option<String>,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AgentEventKind {
    Completed,
    Failed,
    ApprovalRequired,
    QuestionRequired,
    /// 置顶会话真正跑完（主回合与后台任务都结束），提醒用户介入收尾。
    PinnedFinished,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentEvent {
    pub server_id: String,
    pub session_id: Option<String>,
    pub session_title: Option<String>,
    pub kind: AgentEventKind,
    pub event_key: String,
    pub body: Option<String>,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("host must not contain a scheme, path, whitespace, or port")]
    InvalidHost,
    #[error("port must be greater than zero")]
    InvalidPort,
}

/// Redacted view of the application state sent to the frontend.
///
/// Tokens are intentionally excluded; use [`ServerForEdit`] or
/// [`crate::opener::open_saved_server`] when token access is required.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppView {
    pub revision: u64,
    pub settings: DesktopSettings,
    pub servers: Vec<ServerSummary>,
    pub active_id: Option<String>,
    pub statuses: HashMap<String, ServerStatus>,
}

/// Server configuration for the edit form, including the saved token.
///
/// This DTO is only returned for the exact server the user asked to edit;
/// listing APIs must use [`ServerSummary`] instead.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerForEdit {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub backend: Backend,
    pub token: String,
}

impl From<&ServerConfig> for ServerForEdit {
    fn from(server: &ServerConfig) -> Self {
        Self {
            id: server.id.clone(),
            name: server.name.clone(),
            host: server.host.clone(),
            port: server.port,
            backend: server.backend,
            token: server.token.clone(),
        }
    }
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
