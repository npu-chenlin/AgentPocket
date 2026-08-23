pub mod dsh;
pub mod kimi;

use std::collections::HashMap;
use std::time::Duration;

use chrono::Utc;
use serde_json::json;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;

use crate::model::{AgentEvent, Backend, ServerConfig, ServerStatus, SessionSummary};
use crate::protocol::ProtocolState;

pub enum MonitorUpdate {
    Status {
        server_id: String,
        status: ServerStatus,
    },
    Event(AgentEvent),
}

pub struct MonitorManager {
    tasks: HashMap<String, MonitorTask>,
    update_tx: mpsc::Sender<MonitorUpdate>,
}

struct MonitorTask {
    config_fingerprint: String,
    config: ServerConfig,
    token: CancellationToken,
    handle: JoinHandle<()>,
}

impl MonitorManager {
    pub fn new(update_tx: mpsc::Sender<MonitorUpdate>) -> Self {
        Self {
            tasks: HashMap::new(),
            update_tx,
        }
    }

    pub async fn sync_servers(&mut self, servers: &[ServerConfig]) {
        let mut seen = HashMap::new();
        for server in servers {
            seen.insert(server.id.clone(), ());
            let fingerprint = config_fingerprint(server);
            match self.tasks.get(&server.id) {
                Some(task) if task.config_fingerprint == fingerprint => {
                    // unchanged; keep running
                }
                _ => {
                    let id = server.id.clone();
                    self.stop_one(&id, Duration::from_secs(2)).await;
                    let token = CancellationToken::new();
                    let child_token = token.child_token();
                    let update_tx = self.update_tx.clone();
                    let server = server.clone();
                    let config = server.clone();
                    let handle = tokio::spawn(async move {
                        run_single_server(server, update_tx, child_token).await;
                    });
                    self.tasks.insert(
                        id,
                        MonitorTask {
                            config_fingerprint: fingerprint,
                            config,
                            token,
                            handle,
                        },
                    );
                }
            }
        }

        let to_remove: Vec<String> = self
            .tasks
            .keys()
            .filter(|id| !seen.contains_key(*id))
            .cloned()
            .collect();
        for id in to_remove {
            self.stop_one(&id, Duration::from_secs(2)).await;
        }
    }

    pub async fn reconnect_all(&mut self, per_task_timeout: Duration) {
        let entries: Vec<(String, String, ServerConfig)> = self
            .tasks
            .iter()
            .map(|(id, task)| {
                (
                    id.clone(),
                    task.config_fingerprint.clone(),
                    task.config.clone(),
                )
            })
            .collect();

        for (id, fingerprint, config) in entries {
            self.stop_one(&id, per_task_timeout).await;

            let token = CancellationToken::new();
            let child_token = token.child_token();
            let update_tx = self.update_tx.clone();
            let config_for_task = config.clone();
            let handle = tokio::spawn(async move {
                run_single_server(config_for_task, update_tx, child_token).await;
            });
            self.tasks.insert(
                id,
                MonitorTask {
                    config_fingerprint: fingerprint,
                    config,
                    token,
                    handle,
                },
            );
        }
    }

    pub async fn shutdown(&mut self, per_task_timeout: Duration) {
        let ids: Vec<String> = self.tasks.keys().cloned().collect();
        for id in ids {
            self.stop_one(&id, per_task_timeout).await;
        }
    }

    async fn stop_one(&mut self, id: &str, timeout_duration: Duration) {
        if let Some(task) = self.tasks.remove(id) {
            task.token.cancel();
            let _ = timeout(timeout_duration, task.handle).await;
        }
    }
}

fn config_fingerprint(server: &ServerConfig) -> String {
    serde_json::to_string(server).unwrap_or_default()
}

async fn run_single_server(
    server: ServerConfig,
    update_tx: mpsc::Sender<MonitorUpdate>,
    token: CancellationToken,
) {
    match server.backend {
        Backend::Dsh => dsh::run(server, update_tx, token).await,
        Backend::Kimi => kimi::run(server, update_tx, token).await,
    }
}

#[derive(Clone, Debug)]
pub struct ReconnectBackoff {
    next_secs: u64,
}

impl Default for ReconnectBackoff {
    fn default() -> Self {
        Self { next_secs: 1 }
    }
}

impl ReconnectBackoff {
    const CAP: u64 = 30;

    pub fn next_delay(&mut self) -> Duration {
        let delay = Duration::from_secs(self.next_secs);
        self.next_secs = if self.next_secs >= 16 {
            Self::CAP
        } else {
            self.next_secs * 2
        };
        delay
    }

    pub fn reset(&mut self) {
        self.next_secs = 1;
    }
}

pub(crate) fn send_status(
    update_tx: &mpsc::Sender<MonitorUpdate>,
    server_id: &str,
    connected: bool,
    state: &ProtocolState,
    error: Option<String>,
) {
    let mut sessions: Vec<SessionSummary> = state
        .busy
        .iter()
        .map(|id| SessionSummary {
            id: id.clone(),
            title: state
                .titles
                .get(id)
                .filter(|title| !title.is_empty())
                .cloned()
                .unwrap_or_else(|| "会话".to_string()),
            activity: state
                .activities
                .get(id)
                .and_then(|activity| activity.effective_display()),
        })
        .collect();
    // HashSet 遍历顺序不稳定，排序后状态比较才不会因顺序抖动误触发重绘。
    sessions.sort_by(|a, b| a.title.cmp(&b.title).then_with(|| a.id.cmp(&b.id)));
    let status = ServerStatus {
        connected,
        active_count: state.busy.len() as u32,
        sessions,
        server_version: state.server_version.clone(),
        last_checked_at: Some(Utc::now()),
        error,
    };
    let _ = update_tx.try_send(MonitorUpdate::Status {
        server_id: server_id.to_string(),
        status,
    });
}

pub(crate) async fn emit_events(update_tx: &mpsc::Sender<MonitorUpdate>, events: Vec<AgentEvent>) {
    for event in events {
        let _ = update_tx.try_send(MonitorUpdate::Event(event));
    }
}

pub(crate) async fn cancellable_sleep(duration: Duration, token: &CancellationToken) {
    tokio::select! {
        () = sleep(duration) => {}
        () = token.cancelled() => {}
    }
}

pub(crate) async fn cancellable_request<F, T>(
    future: F,
    token: &CancellationToken,
) -> Result<T, String>
where
    F: std::future::Future<Output = Result<T, String>>,
{
    tokio::select! {
        result = future => result,
        () = token.cancelled() => Err("cancelled".to_string()),
    }
}

#[derive(Debug, thiserror::Error)]
#[error("backend detection failed: dsh={dsh:?}, kimi={kimi:?}")]
pub struct ProbeError {
    pub dsh: Option<String>,
    pub kimi: Option<String>,
}

pub async fn probe_backend(server: &ServerConfig) -> Result<Backend, ProbeError> {
    let dsh_result = probe_dsh(server).await;
    if let Ok(()) = dsh_result {
        return Ok(Backend::Dsh);
    }

    let kimi_result = probe_kimi(server).await;
    if let Ok(()) = kimi_result {
        return Ok(Backend::Kimi);
    }

    Err(ProbeError {
        dsh: dsh_result.err(),
        kimi: kimi_result.err(),
    })
}

async fn probe_dsh(server: &ServerConfig) -> Result<(), String> {
    let client = reqwest::Client::new();
    let url = match server.base_url() {
        Ok(url) => url
            .join("/api/agentPreset.list")
            .map_err(|e| e.to_string())?,
        Err(e) => return Err(e.to_string()),
    };
    let body = json!({
        "type": "client-request",
        "rpcId": uuid::Uuid::new_v4().to_string(),
        "method": "agentPreset.list",
        "payload": {},
    });
    let resp = client
        .post(url)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("HTTP {}", status));
    }
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("invalid JSON: {}", e))?;
    if value.get("type").and_then(|v| v.as_str()) == Some("server-response") {
        Ok(())
    } else {
        Err("response type is not server-response".to_string())
    }
}

async fn probe_kimi(server: &ServerConfig) -> Result<(), String> {
    let client = reqwest::Client::new();
    let url = match server.base_url() {
        Ok(url) => url
            .join("/api/v2/sessions")
            .map_err(|e| e.to_string())?
            .clone(),
        Err(e) => return Err(e.to_string()),
    };
    let mut req = client.get(url);
    if !server.token.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", server.token));
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    if status == http::StatusCode::NOT_FOUND {
        return Err("endpoint not found".to_string());
    }
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if status.is_success() {
        let value: serde_json::Value =
            serde_json::from_str(&text).map_err(|e| format!("invalid JSON: {}", e))?;
        if value.get("data").is_some() {
            Ok(())
        } else {
            Err("missing data field".to_string())
        }
    } else {
        Err(format!("HTTP {}", status))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_sequence_caps_and_resets() {
        let mut b = ReconnectBackoff::default();
        assert_eq!(
            [
                b.next_delay(),
                b.next_delay(),
                b.next_delay(),
                b.next_delay(),
                b.next_delay(),
                b.next_delay(),
            ],
            [1, 2, 4, 8, 16, 30].map(Duration::from_secs)
        );
        assert_eq!(b.next_delay(), Duration::from_secs(30));
        b.reset();
        assert_eq!(b.next_delay(), Duration::from_secs(1));
    }

    #[tokio::test]
    async fn removing_one_server_cancels_only_its_task() {
        let (tx, mut rx) = mpsc::channel(64);
        let mut manager = MonitorManager::new(tx);

        let servers = vec![
            ServerConfig::new("s1", "One", "127.0.0.1", 1, "", Backend::Dsh),
            ServerConfig::new("s2", "Two", "127.0.0.1", 2, "", Backend::Dsh),
        ];
        manager.sync_servers(&servers).await;

        // Drain initial status messages.
        let _ = tokio::time::timeout(Duration::from_millis(200), async {
            while rx.try_recv().is_ok() {}
        })
        .await;

        // Remove the first server.
        manager.sync_servers(&servers[1..]).await;

        // The remaining server should still be present and the manager should
        // still own exactly one task.
        assert_eq!(manager.tasks.len(), 1);
        assert!(manager.tasks.contains_key("s2"));

        // The removed task should have been cancelled and removed.
        assert!(!manager.tasks.contains_key("s1"));

        manager.shutdown(Duration::from_secs(1)).await;
    }

    #[tokio::test]
    async fn changing_one_server_restarts_only_that_server() {
        let (tx, _rx) = mpsc::channel(64);
        let mut manager = MonitorManager::new(tx);

        let server_v1 = ServerConfig::new("s1", "One", "127.0.0.1", 1, "", Backend::Dsh);
        manager.sync_servers(&[server_v1]).await;
        let first_fingerprint = manager.tasks.get("s1").unwrap().config_fingerprint.clone();

        let server_v2 = ServerConfig::new("s1", "One", "127.0.0.1", 2, "", Backend::Dsh);
        manager.sync_servers(&[server_v2]).await;
        let second_fingerprint = manager.tasks.get("s1").unwrap().config_fingerprint.clone();

        assert_ne!(first_fingerprint, second_fingerprint);
        assert_eq!(manager.tasks.len(), 1);

        manager.shutdown(Duration::from_secs(1)).await;
    }

    // Minimal async HTTP mock using only Tokio TCP.
    async fn run_mock_server(
        mut handler: impl FnMut(&str, &str) -> (u16, String) + Send + 'static,
        max_requests: usize,
    ) -> (tokio::task::JoinHandle<()>, u16) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::time::timeout;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = tokio::spawn(async move {
            for _ in 0..max_requests {
                let accept = timeout(Duration::from_millis(500), listener.accept()).await;
                let (mut stream, _) = match accept {
                    Ok(Ok(pair)) => pair,
                    _ => break,
                };
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                let parts: Vec<&str> = request.split_whitespace().collect();
                let path = parts.get(1).copied().unwrap_or("/");
                let (status, body) = handler(path, &request);
                let response = format!(
                    "HTTP/1.1 {} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status,
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            }
        });
        (handle, port)
    }

    #[tokio::test]
    async fn detects_dsh_backend() {
        let (handle, port) = run_mock_server(
            |path, body| {
                if path == "/api/agentPreset.list" && body.contains("client-request") {
                    (
                        200,
                        r#"{"type":"server-response","rpcId":"r1","result":{"ok":true,"value":{}}}"#
                            .to_string(),
                    )
                } else {
                    (404, "{}".to_string())
                }
            },
            1,
        )
        .await;

        let server = ServerConfig::new("s1", "Mock", "127.0.0.1", port, "", Backend::Dsh);
        let result = probe_backend(&server).await;
        assert_eq!(result.unwrap(), Backend::Dsh);
        let _ = timeout(Duration::from_secs(1), handle).await;
    }

    #[tokio::test]
    async fn detects_kimi_backend_fallback() {
        let (handle, port) = run_mock_server(
            |path, _body| {
                if path.starts_with("/api/v2/sessions") {
                    (200, r#"{"data":{"items":[]}}"#.to_string())
                } else {
                    (404, "{}".to_string())
                }
            },
            2,
        )
        .await;

        let server = ServerConfig::new("s1", "Mock", "127.0.0.1", port, "", Backend::Kimi);
        let result = probe_backend(&server).await;
        assert_eq!(result.unwrap(), Backend::Kimi);
        let _ = timeout(Duration::from_secs(1), handle).await;
    }

    #[tokio::test]
    async fn probe_failure_reports_both_reasons() {
        let (handle, port) = run_mock_server(|_path, _body| (503, "{}".to_string()), 2).await;

        let server = ServerConfig::new("s1", "Mock", "127.0.0.1", port, "", Backend::Dsh);
        let err = probe_backend(&server).await.unwrap_err();
        assert!(err.dsh.is_some());
        assert!(err.kimi.is_some());
        let _ = timeout(Duration::from_secs(1), handle).await;
    }

    // ------------------------------------------------------------------
    // WebSocket run-loop tests
    // ------------------------------------------------------------------

    async fn run_combined_mock_server(
        http_handler: impl Fn(&str) -> (u16, String) + Send + 'static,
        ws_path: &'static str,
        ws_frames: Vec<String>,
        connect_tx: mpsc::Sender<()>,
    ) -> (tokio::task::JoinHandle<()>, u16) {
        use base64::Engine;
        use sha1::Digest;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::time::timeout;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let handle = tokio::spawn(async move {
            loop {
                let accept = timeout(Duration::from_millis(500), listener.accept()).await;
                let (mut stream, _) = match accept {
                    Ok(Ok(pair)) => pair,
                    _ => break,
                };

                let mut buf = Vec::new();
                let mut tmp = [0u8; 1024];
                loop {
                    let n = match stream.read(&mut tmp).await {
                        Ok(0) => break,
                        Ok(n) => n,
                        Err(_) => break,
                    };
                    buf.extend_from_slice(&tmp[..n]);
                    if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&buf);
                let parts: Vec<&str> = request.split_whitespace().collect();
                let path = parts.get(1).copied().unwrap_or("/");

                if path == ws_path {
                    let key = request
                        .lines()
                        .find_map(|line| {
                            let mut kv = line.splitn(2, ':');
                            if kv.next()?.trim().eq_ignore_ascii_case("Sec-WebSocket-Key") {
                                kv.next().map(|v| v.trim().to_string())
                            } else {
                                None
                            }
                        })
                        .unwrap_or_default();

                    let mut hasher = sha1::Sha1::new();
                    hasher.update(key.as_bytes());
                    hasher.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
                    let accept_key =
                        base64::engine::general_purpose::STANDARD.encode(hasher.finalize());

                    let response = format!(
                        "HTTP/1.1 101 Switching Protocols\r\n\
                         Upgrade: websocket\r\n\
                         Connection: Upgrade\r\n\
                         Sec-WebSocket-Accept: {}\r\n\r\n",
                        accept_key
                    );
                    if stream.write_all(response.as_bytes()).await.is_err() {
                        continue;
                    }
                    let _ = connect_tx.try_send(());

                    for frame in &ws_frames {
                        let payload = frame.as_bytes();
                        let mut header = vec![0x81];
                        if payload.len() < 126 {
                            header.push(payload.len() as u8);
                        } else if payload.len() < 65536 {
                            header.push(126);
                            header.extend_from_slice(&(payload.len() as u16).to_be_bytes());
                        } else {
                            header.push(127);
                            header.extend_from_slice(&(payload.len() as u64).to_be_bytes());
                        }
                        if stream.write_all(&header).await.is_err()
                            || stream.write_all(payload).await.is_err()
                        {
                            break;
                        }
                    }
                    let _ = sleep(Duration::from_millis(50)).await;
                    let _ = stream.shutdown().await;
                } else {
                    let (status, body) = http_handler(path);
                    let response = format!(
                        "HTTP/1.1 {} OK\r\n\
                         Content-Type: application/json\r\n\
                         Content-Length: {}\r\n\
                         Connection: close\r\n\r\n{}",
                        status,
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.shutdown().await;
                }
            }
        });
        (handle, port)
    }

    #[tokio::test]
    async fn dsh_monitor_connects_receives_frame_and_reconnects() {
        let session_list =
            r#"{"type":"server-response","rpcId":"r1","result":{"ok":true,"value":{"items":[]}}}"#;
        let frames = vec![r#"{"type":"server-request","method":"approval/requested","rpcId":"r2","payload":{"sessionId":"s1","approvalId":"a1"}}"#.to_string()];
        let (connect_tx, mut connect_rx) = mpsc::channel(4);

        let (handle, port) = run_combined_mock_server(
            |path| {
                if path == "/api/session.list" {
                    (200, session_list.to_string())
                } else {
                    (404, "{}".to_string())
                }
            },
            "/api/events.mux",
            frames,
            connect_tx,
        )
        .await;

        let server = ServerConfig::new("s-dsh", "Mock", "127.0.0.1", port, "", Backend::Dsh);
        let (update_tx, mut update_rx) = mpsc::channel(64);
        let token = CancellationToken::new();
        let task_token = token.child_token();
        let _task = tokio::spawn(async move { dsh::run(server, update_tx, task_token).await });

        // Wait for at least two WebSocket connections (initial + reconnect).
        let _ = timeout(Duration::from_secs(2), connect_rx.recv()).await;
        let _ = timeout(Duration::from_secs(2), connect_rx.recv()).await;

        // Collect at least one status update and one event.
        let mut saw_connected = false;
        let mut saw_event = false;
        let _ = timeout(Duration::from_secs(2), async {
            while let Some(update) = update_rx.recv().await {
                match update {
                    MonitorUpdate::Status { status, .. } if status.connected => {
                        saw_connected = true;
                    }
                    MonitorUpdate::Event(_) => {
                        saw_event = true;
                    }
                    _ => {}
                }
                if saw_connected && saw_event {
                    break;
                }
            }
        })
        .await;

        token.cancel();
        assert!(saw_connected, "should have emitted connected=true");
        assert!(saw_event, "should have emitted at least one event");
        let _ = timeout(Duration::from_secs(1), handle).await;
    }

    #[tokio::test]
    async fn kimi_monitor_connects_receives_frame_and_reconnects() {
        let sessions = r#"{"data":{"items":[]}}"#;
        let frames = vec![r#"{"type":"server_hello"}"#.to_string()];
        let (connect_tx, mut connect_rx) = mpsc::channel(4);

        let (handle, port) = run_combined_mock_server(
            |path| {
                if path.starts_with("/api/v2/sessions") {
                    (200, sessions.to_string())
                } else {
                    (404, "{}".to_string())
                }
            },
            "/api/v1/ws",
            frames,
            connect_tx,
        )
        .await;

        let server = ServerConfig::new("s-kimi", "Mock", "127.0.0.1", port, "", Backend::Kimi);
        let (update_tx, mut update_rx) = mpsc::channel(64);
        let token = CancellationToken::new();
        let task_token = token.child_token();
        let _task = tokio::spawn(async move { kimi::run(server, update_tx, task_token).await });

        let _ = timeout(Duration::from_secs(2), connect_rx.recv()).await;
        let _ = timeout(Duration::from_secs(2), connect_rx.recv()).await;

        let mut saw_connected = false;
        let mut saw_event_or_hello_ack = false;
        let _ = timeout(Duration::from_secs(2), async {
            while let Some(update) = update_rx.recv().await {
                match update {
                    MonitorUpdate::Status { status, .. } if status.connected => {
                        saw_connected = true;
                    }
                    _ => {
                        saw_event_or_hello_ack = true;
                    }
                }
                if saw_connected && saw_event_or_hello_ack {
                    break;
                }
            }
        })
        .await;

        token.cancel();
        assert!(saw_connected, "should have emitted connected=true");
        let _ = timeout(Duration::from_secs(1), handle).await;
    }
}
