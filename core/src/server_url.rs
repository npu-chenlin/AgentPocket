//! 已保存服务器的 URL 构造。桌面端点击会话时打开的目标地址，与 daemon 侧
//! 返回给 MCP 调用方的会话链接，共用这一份逻辑。

use percent_encoding::{percent_encode, AsciiSet, NON_ALPHANUMERIC};
use url::Url;

use crate::model::{Backend, ServerConfig, ValidationError};

/// 保留 RFC 3986 unreserved 字符（字母数字与 - . _ ~），与 Android 端
/// `Uri.encode` 的行为一致。
const TOKEN_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Build a URL that points to a saved server, mirroring the Android
/// `MainActivity.loadConfiguredUrl` route construction. Kimi servers carry
/// their access token as a `#token=` fragment so the browser session
/// authenticates automatically (the fragment is consumed by the frontend and
/// never sent to the server over the wire); dsh has no token auth.
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
            if !server.token.is_empty() {
                let encoded = percent_encode(server.token.as_bytes(), TOKEN_ENCODE_SET);
                url.set_fragment(Some(&format!("token={encoded}")));
            }
        }
    }

    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(token: &str, backend: Backend) -> ServerConfig {
        ServerConfig::new("s1", "Work", "100.64.0.2", 3080, token, backend)
    }

    #[test]
    fn kimi_root_url_has_trailing_slash() {
        let url = build_server_url(&server("", Backend::Kimi), None).unwrap();
        assert_eq!(url.as_str(), "http://100.64.0.2:3080/");
    }

    #[test]
    fn kimi_session_url_includes_session_path() {
        let url = build_server_url(&server("", Backend::Kimi), Some("sess-123")).unwrap();
        assert_eq!(url.as_str(), "http://100.64.0.2:3080/sessions/sess-123");
    }

    #[test]
    fn kimi_empty_session_id_opens_root() {
        let url = build_server_url(&server("", Backend::Kimi), Some("")).unwrap();
        assert_eq!(url.as_str(), "http://100.64.0.2:3080/");
    }

    #[test]
    fn dsh_always_uses_root_url() {
        let with_session = build_server_url(&server("secret", Backend::Dsh), Some("sess-123")).unwrap();
        let without_session = build_server_url(&server("secret", Backend::Dsh), None).unwrap();
        assert_eq!(with_session.as_str(), "http://100.64.0.2:3080/");
        assert_eq!(without_session.as_str(), "http://100.64.0.2:3080/");
    }

    #[test]
    fn kimi_url_carries_token_fragment() {
        let url = build_server_url(&server("super-secret", Backend::Kimi), Some("sess")).unwrap();
        assert_eq!(
            url.as_str(),
            "http://100.64.0.2:3080/sessions/sess#token=super-secret"
        );

        // 特殊字符按百分号编码处理，与 Android 的 Uri.encode 行为一致。
        let special = build_server_url(&server("a b+c/d", Backend::Kimi), None).unwrap();
        assert_eq!(special.as_str(), "http://100.64.0.2:3080/#token=a%20b%2Bc%2Fd");
    }

    #[test]
    fn dsh_url_never_carries_token() {
        let url = build_server_url(&server("secret", Backend::Dsh), None).unwrap();
        assert_eq!(url.as_str(), "http://100.64.0.2:3080/");
    }
}
