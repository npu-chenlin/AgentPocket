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
        Backend::Opencode => {
            // opencode v2 的会话页路由为 /server/{base64url(serverUrl)}/session/{id}；
            // 未指定会话时回退根路径。
            if let Some(id) = session_id.filter(|s| !s.is_empty()) {
                let key = opencode_server_key(&url);
                url.set_path(&format!("/server/{key}/session/{id}"));
            } else {
                url.set_path("/");
            }
        }
    }

    Ok(url)
}

/// opencode v2 前端用于标识服务器的 key：`base64url(服务器根 URL)`。
/// 前端把整个 origin（含 scheme/host/port，无尾部斜杠）编码进路由段。
pub fn opencode_server_key(base: &Url) -> String {
    let origin = format!(
        "{}://{}:{}",
        base.scheme(),
        base.host_str().unwrap_or_default(),
        base.port_or_known_default().unwrap_or_default()
    );
    encode_base64url(origin.as_bytes())
}

/// URL-safe base64（无填充），与 opencode web 前端 `cn()` 的编码一致。
fn encode_base64url(input: &[u8]) -> String {
    use std::fmt::Write;
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        let _ = write!(out, "{}", CHARS[(n >> 18) as usize & 63] as char);
        let _ = write!(out, "{}", CHARS[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            let _ = write!(out, "{}", CHARS[(n >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            let _ = write!(out, "{}", CHARS[n as usize & 63] as char);
        }
    }
    out
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

    #[test]
    fn opencode_root_url() {
        let url = build_server_url(&server("", Backend::Opencode), None).unwrap();
        assert_eq!(url.as_str(), "http://100.64.0.2:3080/");
    }

    #[test]
    fn opencode_session_url_encodes_server_origin() {
        // v2 前端一致：/server/{base64url(origin)}/session/{id}。
        let url = build_server_url(
            &server("opencode:secret", Backend::Opencode),
            Some("ses_abc"),
        )
        .unwrap();
        assert_eq!(
            url.as_str(),
            "http://100.64.0.2:3080/server/aHR0cDovLzEwMC42NC4wLjI6MzA4MA/session/ses_abc"
        );
    }

    #[test]
    fn opencode_server_key_matches_frontend_encoding() {
        // 真实 v2 前端：base64url("http://127.0.0.1:4096") == aHR0cDovLzEyNy4wLjAuMTo0MDk2
        let base = Url::parse("http://127.0.0.1:4096/").unwrap();
        assert_eq!(
            opencode_server_key(&base),
            "aHR0cDovLzEyNy4wLjAuMTo0MDk2"
        );
    }

    #[test]
    fn opencode_session_url_ignores_credentials_in_token() {
        // token 现在只是 user:pass，不再参与路由构造。
        let mut srv = server("bob:hunter2", Backend::Opencode);
        srv.host = "127.0.0.1".to_string();
        srv.port = 4096;
        let url = build_server_url(&srv, Some("ses_1")).unwrap();
        assert_eq!(
            url.as_str(),
            "http://127.0.0.1:4096/server/aHR0cDovLzEyNy4wLjAuMTo0MDk2/session/ses_1"
        );
    }
}
