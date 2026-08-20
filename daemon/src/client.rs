//! mesh HTTP 客户端：std TcpStream 手写最小 HTTP/1.1，无 TLS 依赖。
//! 只与 tailnet 内 100.x 上的自家端点通信，明文足够，musl 静态零系统依赖。

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

#[derive(Debug)]
pub enum ClientError {
    Io(String),
    Parse(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Io(e) => write!(f, "网络错误：{e}"),
            ClientError::Parse(e) => write!(f, "响应解析失败：{e}"),
        }
    }
}

pub struct ClientResponse {
    pub status: u16,
    pub body: String,
}

pub fn get(
    host: &str,
    port: u16,
    path: &str,
    headers: &[(&str, &str)],
    timeout: Duration,
) -> Result<ClientResponse, ClientError> {
    request("GET", host, port, path, headers, None, timeout)
}

pub fn post(
    host: &str,
    port: u16,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
    timeout: Duration,
) -> Result<ClientResponse, ClientError> {
    request("POST", host, port, path, headers, Some(body), timeout)
}

pub fn request(
    method: &str,
    host: &str,
    port: u16,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
    timeout: Duration,
) -> Result<ClientResponse, ClientError> {
    let addr = resolve(host, port)?;
    let mut stream = TcpStream::connect_timeout(&addr, timeout)
        .map_err(|e| ClientError::Io(e.to_string()))?;
    stream.set_read_timeout(Some(timeout)).map_err(|e| ClientError::Io(e.to_string()))?;
    stream.set_write_timeout(Some(timeout)).map_err(|e| ClientError::Io(e.to_string()))?;

    let mut raw = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n"
    );
    for (key, value) in headers {
        raw.push_str(&format!("{key}: {value}\r\n"));
    }
    if let Some(body) = body {
        raw.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    raw.push_str("\r\n");
    if let Some(body) = body {
        raw.push_str(body);
    }
    stream.write_all(raw.as_bytes()).map_err(|e| ClientError::Io(e.to_string()))?;

    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).map_err(|e| ClientError::Io(e.to_string()))?;
    parse_response(&bytes)
}

fn resolve(host: &str, port: u16) -> Result<SocketAddr, ClientError> {
    (host, port)
        .to_socket_addrs()
        .map_err(|e| ClientError::Io(e.to_string()))?
        .find(|addr| addr.is_ipv4())
        .ok_or_else(|| ClientError::Io(format!("无法解析主机 {host}")))
}

/// 解析响应：状态行 + Content-Length 或 EOF 截断的 body。不支持 chunked（自家端点不用）。
pub fn parse_response(bytes: &[u8]) -> Result<ClientResponse, ClientError> {
    let text = String::from_utf8_lossy(bytes);
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| ClientError::Parse("响应缺少头部分隔".to_string()))?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| ClientError::Parse("状态行无效".to_string()))?;

    let content_length = head.lines().skip(1).find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim().eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
    });
    let body = match content_length {
        Some(length) => body
            .chars()
            .take(length)
            .collect::<String>(),
        None => body.to_string(),
    };
    Ok(ClientResponse { status, body })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const TIMEOUT: Duration = Duration::from_secs(3);

    #[test]
    fn parses_status_line_and_content_length_body() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 5\r\n\r\nhelloextra-bytes-from-keepalive";
        let response = parse_response(raw).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, "hello");
    }

    #[test]
    fn parses_body_without_content_length_up_to_eof() {
        let raw = b"HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\nnope";
        let response = parse_response(raw).unwrap();
        assert_eq!(response.status, 404);
        assert_eq!(response.body, "nope");
    }

    #[test]
    fn rejects_garbage_response() {
        assert!(parse_response(b"not http").is_err());
    }

    #[test]
    fn get_info_against_live_mesh_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let handle = crate::mesh::start(
            crate::mesh::MeshContext {
                config_dir: dir.path().to_path_buf(),
                version: "t",
                hostname: "h".to_string(),
            },
            0,
        )
        .unwrap();
        let response = get("127.0.0.1", handle.port, "/info", &[], TIMEOUT).unwrap();
        assert_eq!(response.status, 200);
        assert!(response.body.contains("agentpocket"));
        handle.stop();
    }
}
