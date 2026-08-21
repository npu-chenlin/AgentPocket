//! mesh HTTP 端点：固定端口、tailnet 访问围栏、/info 握手、/config 拉取与合并推送、
//! /kimi-config 同步 ~/.kimi-code/config.toml。

use std::collections::HashSet;
use std::io::Read as _;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use tiny_http::{Method, Request, Response, Server, StatusCode};

/// mesh 固定监听端口（单一来源：core discovery，GUI 探测与 daemon 端点共用）。
pub use agentpocket_core::discovery::MESH_PORT;
/// recv 轮询间隔，保证 stop 信号能被及时检查。
const POLL_INTERVAL: Duration = Duration::from_millis(200);
/// POST body 大小上限（1 MiB），防止对端超大请求拖垮守护进程。
const MAX_BODY_BYTES: u64 = 1024 * 1024;

#[derive(Debug)]
pub enum MeshError {
    Bind(String),
}

impl std::fmt::Display for MeshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MeshError::Bind(e) => write!(f, "mesh 端点绑定失败（端口被占用？）：{e}"),
        }
    }
}

/// 端点运行上下文：配置目录（core ConfigStore 用）+ kimi config.toml 所在 HOME + 自报身份。
pub struct MeshContext {
    pub config_dir: PathBuf,
    pub kimi_home: PathBuf,
    pub version: &'static str,
    pub hostname: String,
}

/// 只放行回环与 Tailscale CGNAT 网段（100.64.0.0/10）；网络边界即权限边界。
pub fn is_peer_allowed(addr: &SocketAddr) -> bool {
    match addr.ip() {
        std::net::IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback() || (o[0] == 100 && (64..=127).contains(&o[1]))
        }
        std::net::IpAddr::V6(_) => false,
    }
}

#[derive(Debug)]
pub struct MeshHandle {
    /// 测试与后续 CLI 命令读取实际绑定端口。
    #[allow(dead_code)]
    pub port: u16,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl MeshHandle {
    /// 测试用：请求停机并等待线程退出（serve 命令走 wait）。
    #[allow(dead_code)]
    pub fn stop(mut self) {
        self.shutdown();
    }

    /// 阻塞直到服务线程退出（serve 命令用）。
    pub fn wait(mut self) {
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for MeshHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// 启动端点。port 传 0 表示随机端口（测试用）。
pub fn start(ctx: MeshContext, port: u16) -> Result<MeshHandle, MeshError> {
    let server = Server::http(("0.0.0.0", port)).map_err(|e| MeshError::Bind(e.to_string()))?;
    let bound = match server.server_addr() {
        tiny_http::ListenAddr::IP(addr) => addr.port(),
        other => return Err(MeshError::Bind(format!("unexpected listen addr: {other:?}"))),
    };

    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_thread = Arc::clone(&stop);
    let ctx = Arc::new(ctx);
    let join = std::thread::spawn(move || {
        loop {
            if stop_for_thread.load(Ordering::SeqCst) {
                break;
            }
            match server.recv_timeout(POLL_INTERVAL) {
                Ok(Some(request)) => handle_request(request, &ctx),
                Ok(None) => continue,
                Err(e) => {
                    eprintln!("[mesh] 接收错误：{e}");
                    std::process::exit(1);
                }
            }
        }
    });

    Ok(MeshHandle { port: bound, stop, join: Some(join) })
}

fn handle_request(mut request: Request, ctx: &MeshContext) {
    let allowed = request.remote_addr().map(is_peer_allowed).unwrap_or(false);
    if !allowed {
        let _ = request.respond(Response::empty(StatusCode(403)));
        return;
    }
    let path = request.url().split('?').next().unwrap_or("/").to_string();
    match (request.method(), path.as_str()) {
        (Method::Get, "/info") => {
            let body = serde_json::json!({
                "app": "agentpocket",
                "version": ctx.version,
                "name": ctx.hostname,
            });
            let _ = request.respond(Response::from_string(body.to_string())
                .with_status_code(StatusCode(200))
                .with_header(json_header()));
        }
        (Method::Get, "/config") => {
            let store = agentpocket_core::config::ConfigStore::new(ctx.config_dir.clone());
            let current = match store.load() {
                Ok(outcome) => outcome.config,
                Err(_) => agentpocket_core::model::AppConfig::default(),
            };
            match store.export_text(&current) {
                Ok(text) => {
                    let _ = request.respond(Response::from_string(text)
                        .with_status_code(StatusCode(200))
                        .with_header(json_header()));
                }
                Err(e) => {
                    let _ = request.respond(Response::from_string(e.to_string())
                        .with_status_code(StatusCode(500)));
                }
            }
        }
        (Method::Post, "/config") => {
            let source = request
                .headers()
                .iter()
                .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case("X-AgentPocket-Source"))
                .map(|h| h.value.as_str().to_string())
                .or_else(|| {
                    request.remote_addr().map(|a| a.ip().to_string())
                })
                .unwrap_or_else(|| "未知来源".to_string());

            let mut body = String::new();
            let read_ok = request
                .as_reader()
                .take(MAX_BODY_BYTES)
                .read_to_string(&mut body)
                .is_ok();
            if !read_ok {
                let _ = request.respond(Response::empty(StatusCode(400)));
                return;
            }

            let store = agentpocket_core::config::ConfigStore::new(ctx.config_dir.clone());
            let current = match store.load() {
                Ok(outcome) => outcome.config,
                Err(_) => agentpocket_core::model::AppConfig::default(),
            };
            let result = (|| -> Result<(usize, usize), String> {
                let data = store
                    .preview_import_text(&body)
                    .map_err(|e| e.to_string())?;
                // ImportPreviewData.config 为 core 私有字段，导入 ID 经公开的
                // preview.servers（即全部有效服务器，与 apply_import 合并的集合一致）。
                let imported: Vec<&str> = data
                    .preview
                    .servers
                    .iter()
                    .map(|s| s.id.as_str())
                    .collect();
                if imported.is_empty() {
                    return Err("没有可导入的有效服务器".to_string());
                }
                let old_ids: HashSet<&str> =
                    current.servers.iter().map(|s| s.id.as_str()).collect();
                let added = imported
                    .iter()
                    .filter(|id| !old_ids.contains(*id))
                    .count();
                let updated = imported.len().saturating_sub(added);
                let merged = store
                    .apply_import(&current, data, agentpocket_core::config::ImportMode::Merge)
                    .map_err(|e| e.to_string())?;
                store.save(&merged).map_err(|e| e.to_string())?;
                Ok((added, updated))
            })();

            match result {
                Ok((added, updated)) => {
                    println!(
                        "[mesh] 从 {source} 收到配置：新增 {added} / 更新 {updated} 台服务器"
                    );
                    let body = serde_json::json!({"added": added, "updated": updated});
                    let _ = request.respond(Response::from_string(body.to_string())
                        .with_status_code(StatusCode(200))
                        .with_header(json_header()));
                }
                Err(message) => {
                    let _ = request.respond(Response::from_string(message)
                        .with_status_code(StatusCode(400)));
                }
            }
        }
        (Method::Get, "/kimi-config") => match crate::kimi_config::read(&ctx.kimi_home) {
            Ok(text) => {
                let _ = request.respond(Response::from_string(text)
                    .with_status_code(StatusCode(200))
                    .with_header(text_header()));
            }
            Err(message) => {
                let _ = request.respond(Response::from_string(message)
                    .with_status_code(StatusCode(404)));
            }
        },
        (Method::Post, "/kimi-config") => {
            let source = request
                .headers()
                .iter()
                .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case("X-AgentPocket-Source"))
                .map(|h| h.value.as_str().to_string())
                .or_else(|| {
                    request.remote_addr().map(|a| a.ip().to_string())
                })
                .unwrap_or_else(|| "未知来源".to_string());

            let mut body = String::new();
            let read_ok = request
                .as_reader()
                .take(MAX_BODY_BYTES)
                .read_to_string(&mut body)
                .is_ok();
            if !read_ok || body.is_empty() {
                let _ = request.respond(Response::empty(StatusCode(400)));
                return;
            }

            match crate::kimi_config::write(&ctx.kimi_home, &body) {
                Ok(()) => {
                    println!("[mesh] 从 {source} 收到 kimi config.toml（{} 字节）", body.len());
                    let resp = serde_json::json!({"bytes": body.len()});
                    let _ = request.respond(Response::from_string(resp.to_string())
                        .with_status_code(StatusCode(200))
                        .with_header(json_header()));
                }
                Err(message) => {
                    let _ = request.respond(Response::from_string(message)
                        .with_status_code(StatusCode(500)));
                }
            }
        }
        _ => {
            let _ = request.respond(Response::empty(StatusCode(404)));
        }
    }
}

fn text_header() -> tiny_http::Header {
    tiny_http::Header::from_bytes("Content-Type", "text/plain; charset=utf-8")
        .expect("static header is valid")
}

fn json_header() -> tiny_http::Header {
    tiny_http::Header::from_bytes("Content-Type", "application/json; charset=utf-8")
        .expect("static header is valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;

    /// 两个目录共用同一 tempdir：agentpocket config.json 与 .kimi-code/config.toml 不冲突。
    fn ctx(dir: &std::path::Path) -> MeshContext {
        MeshContext {
            config_dir: dir.to_path_buf(),
            kimi_home: dir.to_path_buf(),
            version: "2.8.0-test",
            hostname: "test-host".to_string(),
        }
    }

    fn raw_request(port: u16, request: &str) -> (u16, String) {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        let text = String::from_utf8_lossy(&response).into_owned();
        let status = text.split_whitespace().nth(1)
            .and_then(|c| c.parse::<u16>().ok()).expect("status code");
        let body = text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
        (status, body)
    }

    #[test]
    fn is_peer_allowed_matrix() {
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};
        let sa = |a: u8, b: u8, c: u8, d: u8| SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(a, b, c, d)), 48720);
        assert!(is_peer_allowed(&sa(127, 0, 0, 1)));
        assert!(is_peer_allowed(&sa(100, 64, 0, 2)));
        assert!(is_peer_allowed(&sa(100, 127, 255, 254)));
        assert!(!is_peer_allowed(&sa(100, 128, 0, 1)));
        assert!(!is_peer_allowed(&sa(192, 168, 1, 10)));
        assert!(!is_peer_allowed(&sa(8, 8, 8, 8)));
        let v6 = "[fd7a:115c:a1e0::1]:48720".parse::<SocketAddr>().unwrap();
        assert!(!is_peer_allowed(&v6));
    }

    #[test]
    fn info_endpoint_answers_with_identity() {
        let dir = tempfile::tempdir().unwrap();
        let handle = start(ctx(dir.path()), 0).unwrap();
        let (status, body) = raw_request(
            handle.port,
            "GET /info HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        );
        assert_eq!(status, 200);
        let value: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(value["app"], "agentpocket");
        assert_eq!(value["version"], "2.8.0-test");
        assert_eq!(value["name"], "test-host");
        handle.stop();
    }

    #[test]
    fn unknown_path_returns_404() {
        let dir = tempfile::tempdir().unwrap();
        let handle = start(ctx(dir.path()), 0).unwrap();
        let (status, _) = raw_request(
            handle.port,
            "GET /nope HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        );
        assert_eq!(status, 404);
        handle.stop();
    }

    use agentpocket_core::model::{Backend, ServerConfig};

    fn seed_config(dir: &std::path::Path) {
        let store = agentpocket_core::config::ConfigStore::new(dir.to_path_buf());
        let config = agentpocket_core::model::AppConfig {
            active_id: Some("s1".to_string()),
            servers: vec![ServerConfig::new(
                "s1", "Old", "100.64.0.2", 3080, "tok", Backend::Dsh,
            )],
            ..Default::default()
        };
        store.save(&config).unwrap();
    }

    #[test]
    fn get_config_returns_exchange_format() {
        let dir = tempfile::tempdir().unwrap();
        seed_config(dir.path());
        let handle = start(ctx(dir.path()), 0).unwrap();

        let (status, body) = raw_request(
            handle.port,
            "GET /config HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        );

        assert_eq!(status, 200);
        let value: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(value["schema"], 1);
        assert_eq!(value["servers"][0]["name"], "Old");
        assert!(value.get("settings").is_none());
        handle.stop();
    }

    #[test]
    fn post_config_merges_and_counts_and_backs_up() {
        let dir = tempfile::tempdir().unwrap();
        seed_config(dir.path());
        let handle = start(ctx(dir.path()), 0).unwrap();

        let body = r#"{"schema":1,"servers":[
            {"id":"s1","name":"Renamed","host":"100.64.0.2","port":3080,"token":"tok","backend":"dsh"},
            {"id":"s2","name":"New","host":"100.64.0.3","port":58627,"backend":"kimi"}]}"#;
        let (status, resp_body) = raw_request(
            handle.port,
            &format!(
                "POST /config HTTP/1.1\r\nHost: localhost\r\nX-AgentPocket-Source: peer-a\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ),
        );

        assert_eq!(status, 200);
        let counts: serde_json::Value = serde_json::from_str(&resp_body).unwrap();
        assert_eq!(counts["added"], 1);
        assert_eq!(counts["updated"], 1);

        // 落盘结果：合并后的两台 + activeId 保持。
        let outcome = agentpocket_core::config::ConfigStore::new(dir.path().to_path_buf())
            .load().unwrap();
        assert_eq!(outcome.config.servers.len(), 2);
        assert_eq!(outcome.config.servers[0].name, "Renamed");
        assert_eq!(outcome.config.active_id.as_deref(), Some("s1"));
        // 导入前自动备份已生成。
        assert!(dir.path().join("backups").is_dir());
        handle.stop();
    }

    #[test]
    fn post_garbage_returns_400_without_touching_disk() {
        let dir = tempfile::tempdir().unwrap();
        seed_config(dir.path());
        let handle = start(ctx(dir.path()), 0).unwrap();

        let (status, _) = raw_request(
            handle.port,
            "POST /config HTTP/1.1\r\nHost: localhost\r\nContent-Length: 15\r\nConnection: close\r\n\r\nnot json at all",
        );

        assert_eq!(status, 400);
        let outcome = agentpocket_core::config::ConfigStore::new(dir.path().to_path_buf())
            .load().unwrap();
        assert_eq!(outcome.config.servers.len(), 1);
        handle.stop();
    }

    #[test]
    fn get_kimi_config_returns_file_or_404() {
        let dir = tempfile::tempdir().unwrap();
        let handle = start(ctx(dir.path()), 0).unwrap();
        let (status, _) = raw_request(
            handle.port,
            "GET /kimi-config HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        );
        assert_eq!(status, 404);

        std::fs::create_dir_all(dir.path().join(".kimi-code")).unwrap();
        std::fs::write(dir.path().join(".kimi-code/config.toml"), "model = \"k2\"\n").unwrap();
        let (status, body) = raw_request(
            handle.port,
            "GET /kimi-config HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        );
        assert_eq!(status, 200);
        assert_eq!(body, "model = \"k2\"\n");
        handle.stop();
    }

    #[test]
    fn post_kimi_config_writes_and_backs_up() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".kimi-code")).unwrap();
        std::fs::write(dir.path().join(".kimi-code/config.toml"), "old\n").unwrap();
        let handle = start(ctx(dir.path()), 0).unwrap();

        let body = "model = \"k3\"\n";
        let (status, resp) = raw_request(
            handle.port,
            &format!(
                "POST /kimi-config HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ),
        );
        assert_eq!(status, 200);
        let value: serde_json::Value = serde_json::from_str(&resp).unwrap();
        assert_eq!(value["bytes"], body.len());
        assert_eq!(
            std::fs::read_to_string(dir.path().join(".kimi-code/config.toml")).unwrap(),
            "model = \"k3\"\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join(".kimi-code/config.toml.bak")).unwrap(),
            "old\n"
        );
        handle.stop();
    }

    #[test]
    fn start_on_occupied_port_returns_bind_error() {
        let listener = std::net::TcpListener::bind(("0.0.0.0", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let dir = tempfile::tempdir().unwrap();

        let error = start(ctx(dir.path()), port).unwrap_err();

        assert!(matches!(error, MeshError::Bind(_)));
        assert!(error.to_string().contains("端口被占用"));
        drop(listener);
    }
}
