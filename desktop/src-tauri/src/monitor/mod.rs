pub mod dsh;
pub mod kimi;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::json;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;

use crate::model::{AgentEvent, AgentEventKind, Backend, ServerConfig, ServerStatus, SessionSummary};
use crate::protocol::{build_event, ProtocolState};

/// 用户置顶的会话集合，键为 "server_id|session_id"。
/// 由 AppState（命令层读写）与各监控任务（只读）共享。
pub type PinnedSessions = Arc<RwLock<HashSet<String>>>;

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
    pinned: PinnedSessions,
}

struct MonitorTask {
    config_fingerprint: String,
    config: ServerConfig,
    token: CancellationToken,
    handle: JoinHandle<()>,
}

impl MonitorManager {
    pub fn new(update_tx: mpsc::Sender<MonitorUpdate>, pinned: PinnedSessions) -> Self {
        Self {
            tasks: HashMap::new(),
            update_tx,
            pinned,
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
                    let pinned = self.pinned.clone();
                    let server = server.clone();
                    let config = server.clone();
                    let handle = tokio::spawn(async move {
                        run_single_server(server, update_tx, child_token, pinned).await;
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
            let pinned = self.pinned.clone();
            let config_for_task = config.clone();
            let handle = tokio::spawn(async move {
                run_single_server(config_for_task, update_tx, child_token, pinned).await;
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
            let mut handle = task.handle;
            if timeout(timeout_duration, &mut handle).await.is_err() {
                // Dropping a JoinHandle detaches the task. Abort explicitly so
                // a timed-out monitor cannot keep reconnecting after a config
                // change or be duplicated by the replacement task.
                handle.abort();
                let _ = handle.await;
            }
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
    pinned: PinnedSessions,
) {
    match server.backend {
        Backend::Dsh => dsh::run(server, update_tx, token, pinned).await,
        Backend::Kimi => kimi::run(server, update_tx, token, pinned).await,
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
    pinned: &PinnedSessions,
) {
    let pinned = pinned_read(pinned);
    let pin_key = |session_id: &str| format!("{}|{}", server_id, session_id);
    let title_of = |id: &String| {
        state
            .titles
            .get(id)
            .filter(|title| !title.is_empty())
            .cloned()
            .unwrap_or_else(|| "会话".to_string())
    };
    let mut sessions: Vec<SessionSummary> = state
        .busy
        .iter()
        .map(|id| SessionSummary {
            id: id.clone(),
            title: title_of(id),
            activity: state.activity_text(id),
            pinned: pinned.contains(&pin_key(id)),
            done: false,
        })
        .collect();
    // 置顶但已不忙碌的会话：仍以完成行留在列表中提醒用户介入（对齐手机端语义）。
    for key in pinned.iter() {
        let Some((key_server, session_id)) = key.split_once('|') else {
            continue;
        };
        if key_server != server_id || state.busy.contains(session_id) {
            continue;
        }
        let session_id = session_id.to_string();
        sessions.push(SessionSummary {
            title: title_of(&session_id),
            activity: Some("已完成，等你介入".to_string()),
            id: session_id,
            pinned: true,
            done: true,
        });
    }
    // 排序：置顶运行中 → 置顶已完成 → 其余；同级按标题排序，
    // 保证稳定，状态比较才不会因顺序抖动误触发重绘。
    sessions.sort_by(|a, b| {
        session_rank(a)
            .cmp(&session_rank(b))
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.id.cmp(&b.id))
    });
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

/// 置顶运行中 0 → 置顶已完成 1 → 其余 2（与手机端 sessionRank 一致）。
fn session_rank(session: &SessionSummary) -> u8 {
    if !session.pinned {
        2
    } else if session.done {
        1
    } else {
        0
    }
}

/// 置顶会话「忙碌 -> 空闲」跃迁检测：每次状态变化后调用。
/// prev_busy 由监控任务持有、跨重连保留，保证一次跃迁只发一个事件；
/// 取消置顶的会话会被移出跟踪，不再补发。
pub(crate) fn pinned_finished_events(
    server_id: &str,
    state: &ProtocolState,
    pinned: &HashSet<String>,
    prev_busy: &mut HashSet<String>,
    now: DateTime<Utc>,
) -> Vec<AgentEvent> {
    let prefix = format!("{}|", server_id);
    prev_busy.retain(|id| pinned.contains(&format!("{}{}", prefix, id)));
    let mut events = Vec::new();
    for key in pinned.iter().filter(|key| key.starts_with(&prefix)) {
        let session_id = &key[prefix.len()..];
        if state.busy.contains(session_id) {
            prev_busy.insert(session_id.to_string());
        } else if prev_busy.remove(session_id) {
            events.push(build_event(
                server_id,
                Some(session_id.to_string()),
                AgentEventKind::PinnedFinished,
                format!("pinned-done-{}", session_id),
                now,
                state,
            ));
        }
    }
    events
}

pub(crate) async fn emit_events(update_tx: &mpsc::Sender<MonitorUpdate>, events: Vec<AgentEvent>) {
    for event in events {
        let _ = update_tx.try_send(MonitorUpdate::Event(event));
    }
}

/// 监控任务持有的置顶跟踪器：共享置顶集合 + 本服务器会话的忙碌快照。
/// prev_busy 跨重连保留，保证「完成」跃迁只通知一次。
pub(crate) struct PinTracker {
    pinned: PinnedSessions,
    prev_busy: HashSet<String>,
}

impl PinTracker {
    pub(crate) fn new(pinned: PinnedSessions) -> Self {
        Self {
            pinned,
            prev_busy: HashSet::new(),
        }
    }

    /// 检测置顶会话的完成跃迁并发出事件；每次状态刷新后调用。
    pub(crate) async fn emit_finished(
        &mut self,
        update_tx: &mpsc::Sender<MonitorUpdate>,
        server_id: &str,
        state: &ProtocolState,
    ) {
        let events = {
            let guard = pinned_read(&self.pinned);
            pinned_finished_events(server_id, state, &guard, &mut self.prev_busy, Utc::now())
        };
        emit_events(update_tx, events).await;
    }
}

/// 读共享置顶集合；锁中毒时取回内部值（只读场景无需因中毒失败）。
pub(crate) fn pinned_read(
    pinned: &PinnedSessions,
) -> std::sync::RwLockReadGuard<'_, HashSet<String>> {
    pinned.read().unwrap_or_else(|poisoned| poisoned.into_inner())
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
    use crate::model::AgentEventKind;
    use std::collections::HashSet;

    fn pinned_set(keys: &[&str]) -> HashSet<String> {
        keys.iter().map(|k| k.to_string()).collect()
    }

    fn shared_pins(keys: &[&str]) -> PinnedSessions {
        Arc::new(RwLock::new(pinned_set(keys)))
    }

    fn state_with(busy: &[&str], titles: &[(&str, &str)]) -> ProtocolState {
        let mut state = ProtocolState::default();
        for (id, title) in titles {
            state.titles.insert(id.to_string(), title.to_string());
        }
        for id in busy {
            state.busy.insert(id.to_string());
        }
        state
    }

    fn recv_status(rx: &mut mpsc::Receiver<MonitorUpdate>) -> ServerStatus {
        match rx.try_recv() {
            Ok(MonitorUpdate::Status { status, .. }) => status,
            other => panic!("expected status update, got {:?}", other.is_ok()),
        }
    }

    #[test]
    fn send_status_marks_busy_pinned_session() {
        let (tx, mut rx) = mpsc::channel(4);
        let state = state_with(&["a"], &[("a", "写代码")]);
        let pinned = shared_pins(&["srv|a"]);

        send_status(&tx, "srv", true, &state, None, &pinned);

        let status = recv_status(&mut rx);
        assert_eq!(status.active_count, 1);
        assert_eq!(status.sessions.len(), 1);
        assert!(status.sessions[0].pinned);
        assert!(!status.sessions[0].done);
    }

    #[test]
    fn send_status_keeps_pinned_idle_session_as_done() {
        let (tx, mut rx) = mpsc::channel(4);
        // 会话 b 已不忙碌但被置顶：仍以 done 行留在列表里提醒介入。
        let state = state_with(&["a"], &[("a", "写代码"), ("b", "跑测试")]);
        let pinned = shared_pins(&["srv|b"]);

        send_status(&tx, "srv", true, &state, None, &pinned);

        let status = recv_status(&mut rx);
        // active_count 只统计真正忙碌的会话。
        assert_eq!(status.active_count, 1);
        assert_eq!(status.sessions.len(), 2);
        let done_row = status.sessions.iter().find(|s| s.id == "b").unwrap();
        assert!(done_row.pinned);
        assert!(done_row.done);
        assert_eq!(done_row.activity.as_deref(), Some("已完成，等你介入"));
    }

    #[test]
    fn send_status_orders_pinned_running_then_done_then_rest() {
        let (tx, mut rx) = mpsc::channel(4);
        let state = state_with(
            &["busy-pinned", "busy-plain"],
            &[
                ("busy-pinned", "甲"),
                ("busy-plain", "乙"),
                ("done-pinned", "丙"),
            ],
        );
        let pinned = shared_pins(&["srv|busy-pinned", "srv|done-pinned"]);

        send_status(&tx, "srv", true, &state, None, &pinned);

        let status = recv_status(&mut rx);
        let order: Vec<&str> = status.sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(order, vec!["busy-pinned", "done-pinned", "busy-plain"]);
    }

    #[test]
    fn pinned_finished_events_fire_once_on_busy_to_idle_transition() {
        let pinned = pinned_set(&["srv|a"]);
        let mut prev_busy = HashSet::new();
        let now = Utc::now();

        // 忙碌中：只记录，不发事件。
        let state = state_with(&["a"], &[("a", "写代码")]);
        let events = pinned_finished_events("srv", &state, &pinned, &mut prev_busy, now);
        assert!(events.is_empty());

        // 忙碌 -> 空闲：发一次 PinnedFinished，带上会话标题。
        let state = state_with(&[], &[("a", "写代码")]);
        let events = pinned_finished_events("srv", &state, &pinned, &mut prev_busy, now);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, AgentEventKind::PinnedFinished);
        assert_eq!(events[0].session_id.as_deref(), Some("a"));
        assert_eq!(events[0].session_title.as_deref(), Some("写代码"));

        // 再次空闲：不重复发。
        let events = pinned_finished_events("srv", &state, &pinned, &mut prev_busy, now);
        assert!(events.is_empty());
    }

    #[test]
    fn pinned_finished_events_ignore_unpinned_and_unpin_drops_tracking() {
        let pinned = pinned_set(&[]);
        let mut prev_busy = HashSet::new();
        let now = Utc::now();

        // 未置顶会话的忙碌->空闲不产事件。
        let state = state_with(&["a"], &[("a", "写代码")]);
        assert!(pinned_finished_events("srv", &state, &pinned, &mut prev_busy, now).is_empty());
        let state = state_with(&[], &[("a", "写代码")]);
        assert!(pinned_finished_events("srv", &state, &pinned, &mut prev_busy, now).is_empty());

        // 先置顶并记录忙碌，完成后取消置顶：不再补发事件，跟踪状态被清除。
        let pinned = pinned_set(&["srv|a"]);
        let state = state_with(&["a"], &[("a", "写代码")]);
        assert!(pinned_finished_events("srv", &state, &pinned, &mut prev_busy, now).is_empty());
        let pinned = pinned_set(&[]);
        let state = state_with(&[], &[("a", "写代码")]);
        assert!(pinned_finished_events("srv", &state, &pinned, &mut prev_busy, now).is_empty());
        assert!(prev_busy.is_empty());
    }

    // 其他服务器的置顶 key 不影响本服务器的跃迁检测。
    #[test]
    fn pinned_finished_events_scoped_to_server() {
        let pinned = pinned_set(&["other|a"]);
        let mut prev_busy = HashSet::new();
        let now = Utc::now();

        let state = state_with(&["a"], &[("a", "写代码")]);
        assert!(pinned_finished_events("srv", &state, &pinned, &mut prev_busy, now).is_empty());
        assert!(prev_busy.is_empty());
    }

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
        let mut manager = MonitorManager::new(tx, PinnedSessions::default());

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
        let mut manager = MonitorManager::new(tx, PinnedSessions::default());

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
        let _task = tokio::spawn(async move {
            dsh::run(server, update_tx, task_token, PinnedSessions::default()).await
        });

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
        let _task = tokio::spawn(async move {
            kimi::run(server, update_tx, task_token, PinnedSessions::default()).await
        });

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
