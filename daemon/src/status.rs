//! 一次性探测已配置服务器（在线/版本/忙碌会话数）。
//! REST 语义与 GUI monitor 一致：kimi /api/v1/meta + /api/v2/sessions；dsh /api/session.list。

use std::time::Duration;

use agentpocket_core::model::{Backend, ServerConfig};

use crate::client;

#[derive(Debug)]
pub struct ServerProbe {
    pub name: String,
    pub backend: Backend,
    pub online: bool,
    pub version: Option<String>,
    pub busy: usize,
    pub error: Option<String>,
}

pub fn probe_server(server: &ServerConfig, timeout: Duration) -> ServerProbe {
    let result = match server.backend {
        Backend::Kimi => probe_kimi(server, timeout),
        Backend::Dsh => probe_dsh(server, timeout),
    };
    match result {
        Ok((version, busy)) => ServerProbe {
            name: server.name.clone(),
            backend: server.backend,
            online: true,
            version,
            busy,
            error: None,
        },
        Err(error) => ServerProbe {
            name: server.name.clone(),
            backend: server.backend,
            online: false,
            version: None,
            busy: 0,
            error: Some(error),
        },
    }
}

/// Bearer 头只在 token 非空时携带（与 GUI monitor 行为一致）。
fn bearer(server: &ServerConfig) -> Vec<(&'static str, String)> {
    if server.token.is_empty() {
        Vec::new()
    } else {
        vec![("Authorization", format!("Bearer {}", server.token))]
    }
}

fn probe_kimi(
    server: &ServerConfig,
    timeout: Duration,
) -> Result<(Option<String>, usize), String> {
    let auth = bearer(server);
    let auth_refs: Vec<(&str, &str)> = auth.iter().map(|(k, v)| (*k, v.as_str())).collect();

    let meta = client::get(
        &server.host,
        server.port,
        "/api/v1/meta",
        &auth_refs,
        timeout,
    )
    .map_err(|e| e.to_string())?;
    if meta.status != 200 {
        return Err(format!("meta HTTP {}", meta.status));
    }
    let version = serde_json::from_str::<serde_json::Value>(&meta.body)
        .ok()
        .and_then(|v| {
            v.pointer("/data/server_version")
                .and_then(|s| s.as_str())
                .map(String::from)
        })
        .filter(|v| !v.is_empty());

    let sessions = client::get(
        &server.host,
        server.port,
        "/api/v2/sessions?meta.archived=false&page_size=100",
        &auth_refs,
        timeout,
    )
    .map_err(|e| e.to_string())?;
    if sessions.status != 200 {
        return Err(format!("sessions HTTP {}", sessions.status));
    }
    let value: serde_json::Value =
        serde_json::from_str(&sessions.body).map_err(|e| e.to_string())?;
    let items = value
        .pointer("/data/items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let busy = items
        .iter()
        .filter(|item| {
            item.pointer("/activity/status")
                .and_then(|s| s.as_str())
                .map(|s| s != "idle")
                .unwrap_or(false)
        })
        .count();
    Ok((version, busy))
}

fn probe_dsh(server: &ServerConfig, timeout: Duration) -> Result<(Option<String>, usize), String> {
    let body = serde_json::json!({
        "type": "client-request",
        "rpcId": uuid::Uuid::new_v4().to_string(),
        "method": "session.list",
        "payload": {},
    })
    .to_string();
    let response = crate::client::post(
        &server.host,
        server.port,
        "/api/session.list",
        &[],
        &body,
        timeout,
    )
    .map_err(|e| e.to_string())?;
    if response.status != 200 {
        return Err(format!("session.list HTTP {}", response.status));
    }
    let value: serde_json::Value =
        serde_json::from_str(&response.body).map_err(|e| e.to_string())?;
    if !value
        .pointer("/result/ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return Err("session.list 返回 ok=false".to_string());
    }
    let busy = value
        .pointer("/result/value/items")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter(|item| {
                    item.get("running")
                        .and_then(|r| r.as_bool())
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0);
    Ok((None, busy))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentpocket_core::model::{Backend, ServerConfig};
    use std::time::Duration;
    use tiny_http::{Header, Method, Response, Server};

    const TIMEOUT: Duration = Duration::from_secs(3);

    fn spawn_mock(port: u16, kimi: bool) {
        let server = Server::http(("127.0.0.1", port)).unwrap();
        std::thread::spawn(move || {
            for request in server.incoming_requests() {
                let json = Header::from_bytes("Content-Type", "application/json").unwrap();
                match (kimi, request.method(), request.url().split('?').next().unwrap_or("")) {
                    (true, Method::Get, "/api/v1/meta") => {
                        let _ = request.respond(Response::from_string(
                            r#"{"code":0,"data":{"server_version":"0.36.0"}}"#,
                        ).with_header(json));
                    }
                    (true, Method::Get, "/api/v2/sessions") => {
                        let _ = request.respond(Response::from_string(
                            r#"{"code":0,"data":{"items":[
                                {"id":"a","meta":{"title":"t1"},"activity":{"status":"running"}},
                                {"id":"b","meta":{"title":"t2"},"activity":{"status":"idle"}}]}}"#,
                        ).with_header(json));
                    }
                    (false, Method::Post, "/api/session.list") => {
                        let _ = request.respond(Response::from_string(
                            r#"{"result":{"ok":true,"value":{"items":[
                                {"sessionId":"a","running":true},
                                {"sessionId":"b","running":false}]}}}"#,
                        ).with_header(json));
                    }
                    _ => {
                        let _ = request.respond(Response::from_string("{}").with_status_code(404));
                    }
                }
            }
        });
    }

    fn free_port() -> u16 {
        std::net::TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    #[test]
    fn probes_kimi_version_and_busy_count() {
        let port = free_port();
        spawn_mock(port, true);
        let server = ServerConfig::new("k1", "Kimi", "127.0.0.1", port, "tok", Backend::Kimi);

        let probe = probe_server(&server, TIMEOUT);

        assert!(probe.online);
        assert_eq!(probe.version.as_deref(), Some("0.36.0"));
        assert_eq!(probe.busy, 1);
    }

    #[test]
    fn probes_dsh_busy_count() {
        let port = free_port();
        spawn_mock(port, false);
        let server = ServerConfig::new("d1", "Dsh", "127.0.0.1", port, "", Backend::Dsh);

        let probe = probe_server(&server, TIMEOUT);

        assert!(probe.online);
        assert_eq!(probe.busy, 1);
        assert!(probe.version.is_none());
    }

    #[test]
    fn dead_server_reports_offline() {
        let port = free_port();
        let server = ServerConfig::new("x", "Dead", "127.0.0.1", port, "", Backend::Kimi);

        let probe = probe_server(&server, Duration::from_millis(500));

        assert!(!probe.online);
        assert!(probe.error.is_some());
    }
}
