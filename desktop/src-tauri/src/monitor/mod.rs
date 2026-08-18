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

use crate::model::{AgentEvent, Backend, ServerConfig, ServerStatus};
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
                    self.stop_one(&id).await;
                    let token = CancellationToken::new();
                    let child_token = token.child_token();
                    let update_tx = self.update_tx.clone();
                    let server = server.clone();
                    let handle = tokio::spawn(async move {
                        run_single_server(server, update_tx, child_token).await;
                    });
                    self.tasks.insert(
                        id,
                        MonitorTask {
                            config_fingerprint: fingerprint,
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
            self.stop_one(&id).await;
        }
    }

    pub async fn reconnect_all(&mut self) {
        let entries: Vec<(String, String)> = self
            .tasks
            .iter()
            .map(|(id, task)| (id.clone(), task.config_fingerprint.clone()))
            .collect();

        for (id, fingerprint) in entries {
            let server = match self.resolve_config(&id, &fingerprint) {
                Some(s) => s,
                None => continue,
            };
            self.stop_one(&id).await;

            let token = CancellationToken::new();
            let child_token = token.child_token();
            let update_tx = self.update_tx.clone();
            let handle = tokio::spawn(async move {
                run_single_server(server, update_tx, child_token).await;
            });
            self.tasks.insert(
                id,
                MonitorTask {
                    config_fingerprint: fingerprint,
                    token,
                    handle,
                },
            );
        }
    }

    pub async fn shutdown(&mut self, per_task_timeout: Duration) {
        let ids: Vec<String> = self.tasks.keys().cloned().collect();
        for id in ids {
            if let Some(task) = self.tasks.get(&id) {
                task.token.cancel();
            }
            if let Some(mut task) = self.tasks.remove(&id) {
                let _ = timeout(per_task_timeout, &mut task.handle).await;
            }
        }
    }

    async fn stop_one(&mut self, id: &str) {
        if let Some(task) = self.tasks.remove(id) {
            task.token.cancel();
            let _ = timeout(Duration::from_secs(2), task.handle).await;
        }
    }

    fn resolve_config(&self, id: &str, fingerprint: &str) -> Option<ServerConfig> {
        // The fingerprint encodes the full config, so we can reconstruct enough
        // to restart with the same parameters. Deserialize it back.
        serde_json::from_str::<ServerConfig>(fingerprint)
            .ok()
            .filter(|s| s.id == id)
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
    let status = ServerStatus {
        connected,
        active_count: state.busy.len() as u32,
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
}
