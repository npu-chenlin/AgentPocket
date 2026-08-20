//! mesh HTTP 端点：固定端口、tailnet 访问围栏、/info 握手。
//! /config 的 GET/POST 在本模块由后续提交补齐（见 Task 3）。

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use tiny_http::{Method, Request, Response, Server, StatusCode};

/// mesh 固定监听端口。
pub const MESH_PORT: u16 = 48720;
/// recv 轮询间隔，保证 stop 信号能被及时检查。
const POLL_INTERVAL: Duration = Duration::from_millis(200);

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

/// 端点运行上下文：配置目录（core ConfigStore 用）+ 自报身份。
pub struct MeshContext {
    /// Task 3 接入 core ConfigStore 后开始读取。
    #[allow(dead_code)]
    pub config_dir: PathBuf,
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
                Err(_) => break,
            }
        }
    });

    Ok(MeshHandle { port: bound, stop, join: Some(join) })
}

fn handle_request(request: Request, ctx: &MeshContext) {
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
        _ => {
            let _ = request.respond(Response::empty(StatusCode(404)));
        }
    }
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

    fn ctx(dir: &std::path::Path) -> MeshContext {
        MeshContext {
            config_dir: dir.to_path_buf(),
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
}
