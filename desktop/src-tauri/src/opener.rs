use url::Url;

use crate::model::{AppConfig, Backend, ServerConfig, ValidationError};

/// Build a URL that points to a saved server, mirroring the Android
/// `MainActivity.loadConfiguredUrl` route construction. Tokens are never
/// included in the resulting URL.
pub fn build_server_url(
    server: &ServerConfig,
    session_id: Option<&str>,
) -> Result<Url, ValidationError> {
    let mut url = server.base_url()?;

    match server.backend {
        Backend::Dsh => {
            // DeepSeek Harness has no stable session deep-link; open the root.
            url.set_path("/");
        }
        Backend::Kimi => {
            if let Some(id) = session_id.filter(|s| !s.is_empty()) {
                url.set_path(&format!("/sessions/{}", id));
            } else {
                url.set_path("/");
            }
        }
    }

    Ok(url)
}

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
    fn opens_only_url_derived_from_saved_server() {
        let server = ServerConfig::new("s1", "Work", "100.64.0.2", 3080, "secret", Backend::Dsh);
        assert_eq!(
            build_server_url(&server, None).unwrap().as_str(),
            "http://100.64.0.2:3080/"
        );

        assert!(validate_external_url("file:///etc/passwd").is_err());
        assert!(validate_external_url("javascript:alert(1)").is_err());
    }

    #[test]
    fn kimi_root_url_has_trailing_slash() {
        let server = ServerConfig::new("s1", "Work", "100.64.0.2", 3080, "secret", Backend::Kimi);
        let url = build_server_url(&server, None).unwrap();
        assert_eq!(url.as_str(), "http://100.64.0.2:3080/");
    }

    #[test]
    fn kimi_session_url_includes_session_path() {
        let server = ServerConfig::new("s1", "Work", "100.64.0.2", 3080, "secret", Backend::Kimi);
        let url = build_server_url(&server, Some("sess-123")).unwrap();
        assert_eq!(url.as_str(), "http://100.64.0.2:3080/sessions/sess-123");
    }

    #[test]
    fn kimi_empty_session_id_opens_root() {
        let server = ServerConfig::new("s1", "Work", "100.64.0.2", 3080, "secret", Backend::Kimi);
        let url = build_server_url(&server, Some("")).unwrap();
        assert_eq!(url.as_str(), "http://100.64.0.2:3080/");
    }

    #[test]
    fn dsh_always_uses_root_url() {
        let server = ServerConfig::new("s1", "Work", "100.64.0.2", 3080, "secret", Backend::Dsh);
        let with_session = build_server_url(&server, Some("sess-123")).unwrap();
        let without_session = build_server_url(&server, None).unwrap();
        assert_eq!(with_session.as_str(), "http://100.64.0.2:3080/");
        assert_eq!(without_session.as_str(), "http://100.64.0.2:3080/");
    }

    #[test]
    fn server_url_never_exposes_token() {
        let server = ServerConfig::new(
            "s1",
            "Work",
            "100.64.0.2",
            3080,
            "super-secret",
            Backend::Kimi,
        );
        let url = build_server_url(&server, Some("sess")).unwrap();
        assert!(!url.as_str().contains("super-secret"));
        assert!(!url.as_str().contains("token"));
    }

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
            "http://100.64.0.2:3080/sessions/sess"
        );
        assert!(config.servers.iter().find(|s| s.id == "missing").is_none());
    }
}
