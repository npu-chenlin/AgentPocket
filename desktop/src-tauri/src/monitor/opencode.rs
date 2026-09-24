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
/// 普通 JSON 请求的超时；SSE 长连接不使用这个总超时。
const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// 兜底心跳：即使 SSE 静默，也定期把已知状态推给 UI（不请求服务端）。
const STATUS_PUSH_INTERVAL: Duration = Duration::from_secs(10);
/// SSE 行缓冲上限，防止服务端不按行发送时无限增长。
const MAX_SSE_BUFFER: usize = 1 << 20;

/// 给 opencode 请求挂上认证头。v2 起服务端用 HTTP Basic（用户名固定 `opencode`，
/// 密码来自 `OPENCODE_SERVER_PASSWORD`），token 字段承载 `user:pass`。
fn authorize(req: reqwest::RequestBuilder, server: &ServerConfig) -> reqwest::RequestBuilder {
    match server.opencode_basic_header() {
        Some(header) => req.header("Authorization", header),
        None => req,
    }
}

pub async fn run(
    server: ServerConfig,
    update_tx: mpsc::Sender<MonitorUpdate>,
    token: CancellationToken,
    pinned: PinnedSessions,
) {
    let client = match reqwest::Client::builder()
        // 不能给整个 client 设置 timeout：reqwest 会把它应用到响应体读取，
        // opencode v2 空闲时 SSE 可能约 15 秒才发一次 heartbeat，届时会被误判断线。
        // 普通 JSON 请求在 get_json 中单独套 HTTP_REQUEST_TIMEOUT。
        .connect_timeout(Duration::from_secs(10))
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
    // 初始基线：会话列表（填充标题缓存）。
    fetch_baseline(client, server, state, token).await?;
    pins.emit_finished(update_tx, &server.id, state).await;
    send_status(update_tx, &server.id, true, state, None, &pins.pinned);

    // 事件流长连接。
    let events_url = events_url(server)?;
    let stream_fut = async {
        let req = authorize(
            client.get(events_url).header("Accept", "text/event-stream"),
            server,
        );
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

    // v2 没有可轮询的忙闲端点：状态完全由 SSE 事件驱动
    // （`session.execution.started` / `session.execution.succeeded`）。
    // 心跳只负责把已知状态重新推给 UI，并检测置顶会话完成。
    let mut heartbeat = interval(STATUS_PUSH_INTERVAL);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // 跨 chunk 行缓冲：TCP 分片可能把一行 `data:` 切断。
    let mut buffer = String::new();

    loop {
        tokio::select! {
            chunk = stream.next() => {
                match chunk {
                    Some(Ok(bytes)) => {
                        buffer.push_str(&String::from_utf8_lossy(&bytes));
                        if buffer.len() > MAX_SSE_BUFFER {
                            // 服务端不按行发送时避免无限增长。
                            buffer.clear();
                        } else if let Some(idx) = buffer.rfind('\n') {
                            let ready: String = buffer.drain(..=idx).collect();
                            let events = handle_sse(server, &ready, state);
                            emit_events(update_tx, events).await;
                            pins.emit_finished(update_tx, &server.id, state).await;
                            send_status(update_tx, &server.id, true, state, None, &pins.pinned);
                        }
                    }
                    Some(Err(e)) => return Err(MonitorError::Event(e.to_string())),
                    None => return Err(MonitorError::Event("event stream closed".to_string())),
                }
            }
            _ = heartbeat.tick() => {
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
/// opencode v2 的事件信封为
/// `{"id","created","type","data":{...},"location":{"directory":...}}`，
/// 会话标识在 `data.sessionID`。v1 的 `{"properties":{...}}` 形状仍兼容。
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
        // v2 把负载放在 `data`；更早的包装用 `payload`；v1 则直接平铺。
        let inner = json
            .get("data")
            .or_else(|| json.get("payload"))
            .filter(|v| v.is_object())
            .cloned()
            .unwrap_or_else(|| json.clone());
        let event_type = inner
            .get("type")
            .or_else(|| json.get("type"))
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
        let props = inner
            .get("properties")
            .filter(|v| v.is_object())
            .cloned()
            .unwrap_or_else(|| inner.clone());
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
            // v2：一次执行回合的开始与结束。
            "session.execution.started" => {
                if let Some(id) = session_id {
                    update_busy(state, &id, true);
                }
            }
            "session.execution.succeeded" | "session.execution.failed" => {
                if let Some(id) = session_id {
                    update_busy(state, &id, false);
                }
            }
            // v1 兼容。
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

/// 会话列表基线：填充标题缓存。v2 返回 `{"data":[...],"cursor":{...}}`。
async fn fetch_baseline(
    client: &reqwest::Client,
    server: &ServerConfig,
    state: &mut ProtocolState,
    token: &CancellationToken,
) -> Result<(), MonitorError> {
    let mut titles: Vec<(String, String)> = Vec::new();
    let mut cursor: Option<String> = None;
    // 最多翻 20 页，避免异常服务端导致无限循环。
    for _ in 0..20 {
        let mut path = String::from("/api/session?limit=200");
        if let Some(c) = &cursor {
            path.push_str("&cursor=");
            path.push_str(c);
        }
        let resp = get_json(client, server, &path, token).await?;
        let value: Value = serde_json::from_str(&resp)
            .map_err(|e| MonitorError::Protocol(format!("invalid JSON: {}", e)))?;
        let items = value
            .get("data")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                MonitorError::Protocol("expecting {data:[...]} session envelope".to_string())
            })?;
        for item in items {
            let Some(id) = item.get("id").and_then(|v| v.as_str()) else {
                continue;
            };
            let title = item
                .get("title")
                .and_then(|v| v.as_str())
                .filter(|t| !t.is_empty())
                .unwrap_or("OpenCode 会话");
            titles.push((id.to_string(), title.to_string()));
        }
        cursor = value
            .pointer("/cursor/next")
            .and_then(|v| v.as_str())
            .filter(|c| !c.is_empty())
            .map(String::from);
        if cursor.is_none() {
            break;
        }
    }
    // 收集完整后再替换，避免清空后只填到第一页。
    state.titles = titles.into_iter().collect();
    state.baseline_complete = true;
    Ok(())
}

fn events_url(server: &ServerConfig) -> Result<url::Url, MonitorError> {
    let mut base = server.base_url().map_err(MonitorError::Config)?;
    base.set_path("/api/event");
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
    let req = authorize(
        client.get(url).header("Accept", "application/json"),
        server,
    );
    let request = async {
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
        Ok::<String, MonitorError>(text)
    };
    timeout(HTTP_REQUEST_TIMEOUT, request)
        .await
        .map_err(|_| MonitorError::Http("request timed out".to_string()))?
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
    fn v2_data_envelope_parses_session_created() {
        // v2 信封：负载在 data 内，properties 形状保持不变。
        let data = r#"data: {"id":"evt_1","type":"session.created","data":{"sessionID":"ses_1","info":{"id":"ses_1","title":"写代码"}}}"#;
        let (_, state) = apply(&test_server(""), data);
        assert_eq!(state.titles.get("ses_1").map(|s| s.as_str()), Some("写代码"));
    }

    #[test]
    fn v2_nested_payload_envelope_still_parses() {
        // 兼容更早的 `{payload:{...v1...}}` 包装。
        let data = r#"data: {"directory":"/p","project":"prj","payload":{"id":"evt_1","type":"session.created","properties":{"sessionID":"ses_1","info":{"id":"ses_1","title":"写代码"}}}}"#;
        let (_, state) = apply(&test_server(""), data);
        assert_eq!(state.titles.get("ses_1").map(|s| s.as_str()), Some("写代码"));
    }

    #[test]
    fn durable_suffix_normalized() {
        let data = r#"data: {"id":"evt_1","type":"session.created.5","data":{"sessionID":"ses_1","info":{"id":"ses_1","title":"写代码"}}}"#;
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
        let data = r#"data: {"id":"evt_2","type":"session.deleted","data":{"sessionID":"ses_1"}}"#;
        let events = handle_sse(&server, data, &mut state);
        assert!(events.is_empty());
        assert!(!state.titles.contains_key("ses_1"));
        assert!(!state.raw_busy.contains("ses_1"));
        assert!(!state.busy.contains("ses_1"));
    }

    #[test]
    fn v2_execution_started_marks_busy() {
        let server = test_server("");
        let mut state = ProtocolState::default();
        state.titles.insert("ses_1".into(), "写代码".into());
        let data = r#"data: {"id":"evt_3","type":"session.execution.started","data":{"sessionID":"ses_1"}}"#;
        handle_sse(&server, data, &mut state);
        assert!(state.busy.contains("ses_1"));
        assert!(state.raw_busy.contains("ses_1"));
    }

    #[test]
    fn v2_execution_succeeded_clears_busy() {
        let server = test_server("");
        let mut state = ProtocolState::default();
        state.titles.insert("ses_1".into(), "写代码".into());
        state.raw_busy.insert("ses_1".into());
        state.busy.insert("ses_1".into());
        let data = r#"data: {"id":"evt_4","type":"session.execution.succeeded","data":{"sessionID":"ses_1"}}"#;
        handle_sse(&server, data, &mut state);
        assert!(!state.busy.contains("ses_1"));
        assert!(!state.raw_busy.contains("ses_1"));
    }

    #[test]
    fn v2_execution_failed_clears_busy() {
        let server = test_server("");
        let mut state = ProtocolState::default();
        state.raw_busy.insert("ses_1".into());
        state.busy.insert("ses_1".into());
        let data = r#"data: {"id":"evt_5","type":"session.execution.failed","data":{"sessionID":"ses_1"}}"#;
        handle_sse(&server, data, &mut state);
        assert!(!state.busy.contains("ses_1"));
    }

    #[test]
    fn v1_session_status_marks_busy() {
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
        let data = r#"data: {"id":"evt_4","type":"session.error","data":{"sessionID":"ses_1"}}"#;
        let (events, state) = apply(&test_server(""), data);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, AgentEventKind::Failed);
        assert_eq!(events[0].session_id.as_deref(), Some("ses_1"));
        let _ = state;
    }

    #[test]
    fn real_v2_execution_sequence_captured_from_server() {
        // 实测 v2 事件样本：started 后 succeeded 对应一次完整回合。
        let started = r#"data: {"id":"evt_0d2711843001VRtmJjVQBodo27","created":1790237022275,"type":"session.execution.started","data":{"sessionID":"ses_f2d999e91ffeDMQ4ckFCeAl0ge"},"durable":{"aggregateID":"ses_f2d999e91ffeDMQ4ckFCeAl0ge"}}"#;
        let succeeded = r#"data: {"id":"evt_0d271291f001toSXxqMlQrSqQh","created":1790237026591,"type":"session.execution.succeeded","data":{"sessionID":"ses_f2d999e91ffeDMQ4ckFCeAl0ge"},"durable":{"aggregateID":"ses_f2d999e91ffeDMQ4ckFCeAl0ge"}}"#;
        let server = test_server("opencode:opencode");
        let mut state = ProtocolState::default();
        handle_sse(&server, started, &mut state);
        assert!(state.busy.contains("ses_f2d999e91ffeDMQ4ckFCeAl0ge"));
        handle_sse(&server, succeeded, &mut state);
        assert!(!state.busy.contains("ses_f2d999e91ffeDMQ4ckFCeAl0ge"));
    }

    #[test]
    fn non_session_events_ignored() {
        let data = r#"data: {"id":"evt_5","type":"session.text.delta","data":{"sessionID":"ses_1"}}"#;
        let (events, state) = apply(&test_server(""), data);
        assert!(events.is_empty());
        assert!(state.titles.is_empty());
        assert!(state.busy.is_empty());
    }

    #[test]
    fn server_connected_event_ignored() {
        let data = r#"data: {"id":"evt_0","type":"server.connected","data":{}}"#;
        let (events, state) = apply(&test_server(""), data);
        assert!(events.is_empty());
        assert!(state.busy.is_empty());
    }

    #[test]
    fn events_url_uses_api_prefix() {
        let url = events_url(&test_server("")).unwrap();
        assert_eq!(url.as_str(), "http://127.0.0.1:4096/api/event");
    }

    #[test]
    fn authorization_header_is_basic_not_bearer() {
        let req = authorize(
            reqwest::Client::new()
                .get("http://127.0.0.1:4096/api/config"),
            &test_server("opencode:opencode"),
        )
        .build()
        .unwrap();
        assert_eq!(
            req.headers()
                .get("Authorization")
                .unwrap()
                .to_str()
                .unwrap(),
            "Basic b3BlbmNvZGU6b3BlbmNvZGU="
        );
    }

    #[test]
    fn no_authorization_header_without_credentials() {
        let req = authorize(
            reqwest::Client::new()
                .get("http://127.0.0.1:4096/api/config"),
            &test_server(""),
        )
        .build()
        .unwrap();
        assert!(req.headers().get("Authorization").is_none());
    }
}
