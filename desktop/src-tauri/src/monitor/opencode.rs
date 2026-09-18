use std::time::Duration;

use chrono::Utc;
use futures_util::StreamExt;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio::time::{interval, timeout};
use tokio_util::sync::CancellationToken;

use crate::model::{AgentEvent, AgentEventKind, ServerConfig};
use crate::monitor::{
    cancellable_request, cancellable_sleep, emit_events, send_status, MonitorUpdate,
    PinnedSessions, PinTracker, ReconnectBackoff,
};
use crate::protocol::{build_event, ProtocolState};

const EVENT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const STATUS_POLL_INTERVAL: Duration = Duration::from_secs(2);

pub async fn run(
    server: ServerConfig,
    update_tx: mpsc::Sender<MonitorUpdate>,
    token: CancellationToken,
    pinned: PinnedSessions,
) {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            send_status(
                &update_tx,
                &server.id,
                false,
                &ProtocolState::default(),
                Some(e.to_string()),
                &PinnedSessions::default(),
            );
            return;
        }
    };

    let mut backoff = ReconnectBackoff::default();
    let mut state = ProtocolState::default();
    let mut pins = PinTracker::new(pinned);

    loop {
        if token.is_cancelled() {
            break;
        }

        match poll_and_events(&client, &server, &update_tx, &mut state, &token, &mut pins).await {
            Ok(()) => {
                backoff.reset();
            }
            Err(e) => {
                send_status(
                    &update_tx,
                    &server.id,
                    false,
                    &state,
                    Some(e.to_string()),
                    &pins.pinned,
                );
            }
        }

        let delay = backoff.next_delay();
        cancellable_sleep(delay, &token).await;
        if token.is_cancelled() {
            break;
        }
    }

    send_status(&update_tx, &server.id, false, &state, None, &pins.pinned);
}

async fn poll_and_events(
    client: &reqwest::Client,
    server: &ServerConfig,
    update_tx: &mpsc::Sender<MonitorUpdate>,
    state: &mut ProtocolState,
    token: &CancellationToken,
    pins: &mut PinTracker,
) -> Result<(), MonitorError> {
    // 初始基线：会话列表 + 状态。
    fetch_baseline(client, server, state, token).await?;
    pins.emit_finished(update_tx, &server.id, state).await;
    send_status(update_tx, &server.id, true, state, None, &pins.pinned);

    // 事件流长连接。
    let events_url = events_url(server)?;
    let stream_fut = async {
        let mut req = client.get(events_url).header("Accept", "text/event-stream");
        if !server.opencode_token().is_empty() {
            req = req.header("Authorization", format!("Bearer {}", server.opencode_token()));
        }
        req.send().await
    };
    let resp = tokio::select! {
        () = token.cancelled() => return Err(MonitorError::Event("cancelled".to_string())),
        r = timeout(EVENT_CONNECT_TIMEOUT, stream_fut) =>
            r.map_err(|_| MonitorError::Event("event stream connect timed out".to_string()))?
            .map_err(|e| MonitorError::Event(e.to_string()))?,
    };
    if !resp.status().is_success() {
        return Err(MonitorError::Event(format!("HTTP {}", resp.status())));
    }
    let mut stream = resp.bytes_stream();

    // 状态轮询与事件流并行：轮询负责忙闲，事件流负责过滤掉自己产生的噪声、
    // 识别结果事件。
    let mut poll = interval(STATUS_POLL_INTERVAL);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            chunk = stream.next() => {
                match chunk {
                    Some(Ok(bytes)) => {
                        let text = String::from_utf8_lossy(&bytes).to_string();
                        let events = handle_sse(server, &text, state);
                        emit_events(update_tx, events).await;
                    }
                    Some(Err(e)) => return Err(MonitorError::Event(e.to_string())),
                    None => return Err(MonitorError::Event("event stream closed".to_string())),
                }
            }
            _ = poll.tick() => {
                let statuses = fetch_status(client, server, token).await?;
                for (session_id, busy) in statuses {
                    update_busy(state, &session_id, busy);
                }
                pins.emit_finished(update_tx, &server.id, state).await;
                send_status(update_tx, &server.id, true, state, None, &pins.pinned);
            }
            () = token.cancelled() => {
                return Err(MonitorError::Event("cancelled".to_string()));
            }
        }
    }
}

fn update_busy(state: &mut ProtocolState, session_id: &str, busy: bool) {
    if busy {
        state.raw_busy.insert(session_id.to_string());
    } else {
        state.raw_busy.remove(session_id);
    }
    state.apply_effective_busy(session_id);
}

/// 解析一段 SSE 文本（可能包含多行），提取事件并更新内部状态。
/// 返回需要发出的 AgentEvent。
///
/// opencode 事件流支持两种信封：
/// - v1 `/event`：`{"id","type","properties"}`
/// - v2 `/global/event`：`{"directory","project","payload":{...v1 形状...}}`
/// 会话字段都在 `properties` 里（v1/v2 一致）。
fn handle_sse(
    server: &ServerConfig,
    text: &str,
    state: &mut ProtocolState,
) -> Vec<AgentEvent> {
    let mut events = Vec::new();
    for line in text.lines() {
        let Some(payload) = line.strip_prefix("data:") else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<Value>(payload.trim()) else {
            continue;
        };
        let inner = json.get("payload").cloned().unwrap_or(json);
        let event_type = inner
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        // durable 事件带 `.N` 尾缀（如 session.created.1），归一化处理。
        let event_type = if let Some((base, suffix)) = event_type.rsplit_once('.') {
            if suffix.chars().all(|c| c.is_ascii_digit()) {
                base
            } else {
                event_type
            }
        } else {
            event_type
        };
        let props = inner.get("properties").cloned().unwrap_or_default();
        let session_id = props
            .get("sessionID")
            .or_else(|| props.get("session_id"))
            .and_then(|v| v.as_str())
            .map(String::from);
        match event_type {
            "session.updated" | "session.created" => {
                if let Some(info) = props.get("info") {
                    if let Some(id) = info.get("id").and_then(|v| v.as_str()) {
                        let title = info
                            .get("title")
                            .and_then(|v| v.as_str())
                            .unwrap_or("OpenCode 会话");
                        state.titles.insert(id.to_string(), title.to_string());
                    }
                } else if let Some(id) = session_id {
                    let title = inner
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("OpenCode 会话");
                    state.titles.insert(id, title.to_string());
                }
            }
            "session.deleted" => {
                if let Some(id) = session_id {
                    state.titles.remove(&id);
                    state.raw_busy.remove(&id);
                    state.busy.remove(&id);
                }
            }
            "session.status" => {
                if let (Some(id), Some(status)) = (session_id, props.get("status")) {
                    let busy = status.get("type").and_then(|v| v.as_str()) != Some("idle");
                    update_busy(state, &id, busy);
                }
            }
            "session.idle" => {
                if let Some(id) = session_id {
                    state.raw_busy.remove(&id);
                    state.apply_effective_busy(&id);
                }
            }
            "session.error" => {
                if let Some(id) = session_id.as_ref() {
                    events.push(build_event(
                        &server.id,
                        Some(id.clone()),
                        AgentEventKind::Failed,
                        format!("opencode-session-error-{}", id),
                        Utc::now(),
                        state,
                    ));
                }
            }
            _ => {}
        }
    }
    events
}

/// 会话列表基线：填充标题缓存。
async fn fetch_baseline(
    client: &reqwest::Client,
    server: &ServerConfig,
    state: &mut ProtocolState,
    token: &CancellationToken,
) -> Result<(), MonitorError> {
    let resp = get_json(client, server, "/session", token).await?;
    let value: Value = serde_json::from_str(&resp)
        .map_err(|e| MonitorError::Protocol(format!("invalid JSON: {}", e)))?;
    let items = value.as_array().ok_or_else(|| {
        MonitorError::Protocol("expecting session list array".to_string())
    })?;
    state.titles.clear();
    for item in items {
        let Some(id) = item.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("OpenCode 会话");
        state.titles.insert(id.to_string(), title.to_string());
    }
    state.baseline_complete = true;
    Ok(())
}

/// 轮询会话状态：GET /session/status?directory=<dir>，返回 id -> busy。
async fn fetch_status(
    client: &reqwest::Client,
    server: &ServerConfig,
    token: &CancellationToken,
) -> Result<Vec<(String, bool)>, MonitorError> {
    let mut url = server.base_url().map_err(MonitorError::Config)?;
    url.set_path("/session/status");
    url.query_pairs_mut().append_pair(
        "directory",
        &agentpocket_core::server_url::opencode_directory(server),
    );
    let mut req = client.get(url).header("Accept", "application/json");
    if !server.opencode_token().is_empty() {
        req = req.header("Authorization", format!("Bearer {}", server.opencode_token()));
    }
    let resp = cancellable_request(async { req.send().await.map_err(|e| e.to_string()) }, token)
        .await
        .map_err(MonitorError::Http)?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| MonitorError::Http(e.to_string()))?;
    if !status.is_success() {
        return Err(MonitorError::Http(format!("HTTP {}", status)));
    }
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| MonitorError::Protocol(format!("invalid JSON: {}", e)))?;
    let Some(map) = value.as_object() else {
        return Ok(Vec::new());
    };
    let mut result = Vec::new();
    for (id, status) in map {
        let busy = status.get("type").and_then(|v| v.as_str()) != Some("idle");
        result.push((id.clone(), busy));
    }
    Ok(result)
}

fn events_url(server: &ServerConfig) -> Result<url::Url, MonitorError> {
    let mut base = server.base_url().map_err(MonitorError::Config)?;
    base.set_path("/global/event");
    Ok(base)
}

async fn get_json(
    client: &reqwest::Client,
    server: &ServerConfig,
    path: &str,
    token: &CancellationToken,
) -> Result<String, MonitorError> {
    let url = server
        .base_url()
        .map_err(MonitorError::Config)?
        .join(path)
        .map_err(MonitorError::Url)?;
    let mut req = client.get(url).header("Accept", "application/json");
    if !server.opencode_token().is_empty() {
        req = req.header("Authorization", format!("Bearer {}", server.opencode_token()));
    }
    let resp = cancellable_request(async { req.send().await.map_err(|e| e.to_string()) }, token)
        .await
        .map_err(MonitorError::Http)?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| MonitorError::Http(e.to_string()))?;
    if !status.is_success() {
        return Err(MonitorError::Http(format!("HTTP {}", status)));
    }
    Ok(text)
}

#[derive(Debug, thiserror::Error)]
enum MonitorError {
    #[error("config error: {0}")]
    Config(#[source] crate::model::ValidationError),
    #[error("url error: {0}")]
    Url(#[source] url::ParseError),
    #[error("http error: {0}")]
    Http(String),
    #[error("event stream error: {0}")]
    Event(String),
    #[error("protocol error: {0}")]
    Protocol(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_server(token: &str) -> ServerConfig {
        ServerConfig::new(
            "oc",
            "OpenCode",
            "127.0.0.1",
            4096,
            token,
            crate::model::Backend::Opencode,
        )
    }

    fn apply(server: &ServerConfig, lines: &str) -> (Vec<AgentEvent>, ProtocolState) {
        let mut state = ProtocolState::default();
        let events = handle_sse(server, lines, &mut state);
        (events, state)
    }

    #[test]
    fn v1_envelope_parses_session_created() {
        let data = r#"data: {"id":"evt_1","type":"session.created","properties":{"sessionID":"ses_1","info":{"id":"ses_1","title":"写代码"}}}"#;
        let (_, state) = apply(&test_server(""), data);
        assert_eq!(state.titles.get("ses_1").map(|s| s.as_str()), Some("写代码"));
    }

    #[test]
    fn v2_envelope_parses_session_created() {
        let data = r#"data: {"directory":"/p","project":"prj","payload":{"id":"evt_1","type":"session.created","properties":{"sessionID":"ses_1","info":{"id":"ses_1","title":"写代码"}}}}"#;
        let (_, state) = apply(&test_server(""), data);
        assert_eq!(state.titles.get("ses_1").map(|s| s.as_str()), Some("写代码"));
    }

    #[test]
    fn durable_suffix_normalized() {
        let data = r#"data: {"id":"evt_1","type":"session.created.5","properties":{"sessionID":"ses_1","info":{"id":"ses_1","title":"写代码"}}}"#;
        let (_, state) = apply(&test_server(""), data);
        assert_eq!(state.titles.get("ses_1").map(|s| s.as_str()), Some("写代码"));
    }

    #[test]
    fn session_deleted_removes_state() {
        let server = test_server("");
        let mut state = ProtocolState::default();
        state.titles.insert("ses_1".into(), "写代码".into());
        state.raw_busy.insert("ses_1".into());
        state.busy.insert("ses_1".into());
        let data = r#"data: {"id":"evt_2","type":"session.deleted","properties":{"sessionID":"ses_1"}}"#;
        let events = handle_sse(&server, data, &mut state);
        assert!(events.is_empty());
        assert!(!state.titles.contains_key("ses_1"));
        assert!(!state.raw_busy.contains("ses_1"));
        assert!(!state.busy.contains("ses_1"));
    }

    #[test]
    fn session_status_marks_busy() {
        let server = test_server("");
        let mut state = ProtocolState::default();
        state.titles.insert("ses_1".into(), "写代码".into());
        let data = r#"data: {"id":"evt_3","type":"session.status","properties":{"sessionID":"ses_1","status":{"type":"busy"}}}"#;
        handle_sse(&server, data, &mut state);
        assert!(state.busy.contains("ses_1"));
        assert!(state.raw_busy.contains("ses_1"));
    }

    #[test]
    fn session_error_produces_failed_event() {
        let data = r#"data: {"id":"evt_4","type":"session.error","properties":{"sessionID":"ses_1"}}"#;
        let (events, state) = apply(&test_server(""), data);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, AgentEventKind::Failed);
        assert_eq!(events[0].session_id.as_deref(), Some("ses_1"));
        let _ = state;
    }

    #[test]
    fn non_session_events_ignored() {
        let data = r#"data: {"id":"evt_5","type":"message.part.updated","properties":{"sessionID":"ses_1"}}"#;
        let (events, state) = apply(&test_server(""), data);
        assert!(events.is_empty());
        assert!(state.titles.is_empty());
        assert!(state.busy.is_empty());
    }
}