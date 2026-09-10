use agentpocket_core::server_url::build_server_url;
use url::Url;

use crate::model::{AppConfig, ValidationError};

/// Validate that an externally supplied URL is safe to open.
/// Only `http:` and `https:` schemes are accepted; dangerous schemes such as
/// `file:`, `javascript:`, and `data:` are rejected.
pub fn validate_external_url(url: &str) -> Result<Url, OpenerError> {
    let parsed = Url::parse(url).map_err(|_| OpenerError::InvalidUrl)?;
    match parsed.scheme() {
        "http" | "https" => Ok(parsed),
        _ => Err(OpenerError::ForbiddenScheme),
    }
}

/// Open the URL for a saved server identified by `server_id`.
/// Callers cannot pass arbitrary URLs; the URL is derived from the stored
/// `ServerConfig` and an optional session id.
pub fn open_saved_server<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    config: &AppConfig,
    server_id: &str,
    session_id: Option<&str>,
) -> Result<(), OpenerError> {
    let server = config
        .servers
        .iter()
        .find(|s| s.id == server_id)
        .ok_or(OpenerError::ServerNotFound)?;
    let url = build_server_url(server, session_id)?;

    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(url.as_str(), None::<&str>)
        .map_err(|e| OpenerError::OpenFailed(e.to_string()))
}

#[derive(Debug, thiserror::Error)]
pub enum OpenerError {
    #[error("invalid URL")]
    InvalidUrl,
    #[error("forbidden URL scheme")]
    ForbiddenScheme,
    #[error("server not found")]
    ServerNotFound,
    #[error("failed to open URL: {0}")]
    OpenFailed(String),
    #[error(transparent)]
    Config(#[from] ValidationError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Backend, ServerConfig};

    #[test]
    fn validate_external_url_accepts_http_and_https() {
        assert!(validate_external_url("http://example.com/").is_ok());
        assert!(validate_external_url("https://example.com/path").is_ok());
    }

    #[test]
    fn validate_external_url_rejects_dangerous_schemes() {
        assert!(validate_external_url("file:///etc/passwd").is_err());
        assert!(validate_external_url("javascript:alert(1)").is_err());
        assert!(validate_external_url("data:text/html,<script>alert(1)</script>").is_err());
        assert!(validate_external_url("ftp://example.com/").is_err());
    }

    #[test]
    fn open_saved_server_looks_up_config_by_id() {
        let server = ServerConfig::new("s1", "Work", "100.64.0.2", 3080, "secret", Backend::Kimi);
        let config = AppConfig {
            schema: 1,
            active_id: Some("s1".to_string()),
            servers: vec![server],
            settings: Default::default(),
        };

        // We cannot call open_saved_server without a real Tauri app handle,
        // but we can verify the lookup logic by checking build_server_url on
        // the resolved server.
        let resolved = config.servers.iter().find(|s| s.id == "s1").unwrap();
        assert_eq!(
            build_server_url(resolved, Some("sess")).unwrap().as_str(),
            "http://100.64.0.2:3080/sessions/sess#token=secret"
        );
        assert!(config.servers.iter().find(|s| s.id == "missing").is_none());
    }
}
