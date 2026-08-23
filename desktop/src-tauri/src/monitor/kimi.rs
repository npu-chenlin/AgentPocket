use std::time::Duration;

use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::protocol::{Message, WebSocketConfig};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tokio_util::sync::CancellationToken;

use crate::model::{AgentEvent, ServerConfig};
use crate::monitor::{
    cancellable_request, cancellable_sleep, emit_events, send_status, MonitorUpdate,
    ReconnectBackoff,
};
use crate::protocol::kimi::parse_frame;
use crate::protocol::ProtocolState;

pub async fn run(
    server: ServerConfig,
    update_tx: mpsc::Sender<MonitorUpdate>,
    token: CancellationToken,
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
            );
            return;
        }
    };

    let mut backoff = ReconnectBackoff::default();
    let mut state = ProtocolState::default();

    loop {
        if token.is_cancelled() {
            break;
        }

        match run_once(&client, &server, &update_tx, &mut state, &token).await {
            Ok(()) => {
                backoff.reset();
            }
            Err(e) => {
                send_status(&update_tx, &server.id, false, &state, Some(e.to_string()));
            }
        }

        let delay = backoff.next_delay();
        cancellable_sleep(delay, &token).await;
        if token.is_cancelled() {
            break;
        }
    }

    send_status(&update_tx, &server.id, false, &state, None);
}

async fn run_once(
    client: &reqwest::Client,
    server: &ServerConfig,
    update_tx: &mpsc::Sender<MonitorUpdate>,
    state: &mut ProtocolState,
    token: &CancellationToken,
) -> Result<(), MonitorError> {
    let base = server.base_url().map_err(MonitorError::Config)?;

    // 0. Fetch server version from meta (best-effort, never fatal).
    fetch_version(client, server, base.clone(), state, token).await;

    // 1. Fetch baseline session list.
    fetch_baseline(client, server, base.clone(), state, token).await?;
    // 启动种子：对忙碌会话取一次详情（含 main_turn_active / 后台任务数），
    // 之后的细分状态全靠推送。
    seed_busy_sessions(client, server, base.clone(), state, token).await;
    send_status(update_tx, &server.id, false, state, None);

    // 2. Connect WebSocket with optional subprotocol header.
    let ws_url = ws_url(server)?;
    let (mut ws_stream, response) = connect_ws(&ws_url, server)
        .await
        .map_err(|e| MonitorError::Ws(e.to_string()))?;

    // Preserve server-selected subprotocol if any.
    let selected_protocol = response
        .headers()
        .get("Sec-WebSocket-Protocol")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let _ = selected_protocol;

    send_status(update_tx, &server.id, true, state, None);

    // 3. 只用基础订阅：相位/工具/任务事件都由它推送，不再订阅 transcript（省流量）。
    let session_ids: Vec<String> = state.titles.keys().cloned().collect();
    let hello = client_hello(&session_ids);
    ws_stream
        .send(Message::Text(hello.to_string().into()))
        .await
        .map_err(|e| MonitorError::Ws(e.to_string()))?;
    state.subscribed = session_ids.into_iter().collect();

    read_loop(client, ws_stream, server, update_tx, state, token).await
}

async fn fetch_version(
    client: &reqwest::Client,
    server: &ServerConfig,
    base: url::Url,
    state: &mut ProtocolState,
    token: &CancellationToken,
) {
    let Ok(meta_url) = base.join("/api/v1/meta") else {
        return;
    };
    let mut req = client.get(meta_url);
    if !server.token.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", server.token));
    }
    let Ok(resp) = cancellable_request(async { req.send().await.map_err(|e| e.to_string()) }, token)
        .await
    else {
        return;
    };
    if !resp.status().is_success() {
        return;
    }
    let Ok(text) = resp.text().await else {
        return;
    };
    let version: Option<String> = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| {
            v.get("data")
                .and_then(|d| d.get("server_version"))
                .and_then(|s| s.as_str())
                .map(String::from)
        })
        .filter(|v| !v.is_empty());
    if version.is_some() {
        state.server_version = version;
    }
}

async fn fetch_baseline(
    client: &reqwest::Client,
    server: &ServerConfig,
    base: url::Url,
    state: &mut ProtocolState,
    token: &CancellationToken,
) -> Result<(), MonitorError> {
    let list_url = build_sessions_url(base)?;
    let mut req = client.get(list_url);
    if !server.token.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", server.token));
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
    let items = value
        .get("data")
        .and_then(|d| d.get("items"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| MonitorError::Protocol("missing data.items".to_string()))?;

    state.titles.clear();
    state.busy.clear();
    state.raw_busy.clear();
    state.main_turn_inactive.clear();
    state.bg_running.clear();
    state.activities.clear();

    for item in items {
        let id = item
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| MonitorError::Protocol("missing session id".to_string()))?;
        let meta = item.get("meta").and_then(|v| v.as_object());
        let title = meta
            .and_then(|m| m.get("title"))
            .and_then(|v| v.as_str())
            .unwrap_or("点击查看 Kimi 会话");
        let title = if title.is_empty() || title == "null" {
            "点击查看 Kimi 会话"
        } else {
            title
        };
        state.titles.insert(id.to_string(), title.to_string());

        let status = item
            .get("activity")
            .and_then(|a| a.get("status"))
            .and_then(|v| v.as_str())
            .unwrap_or("idle");
        if status != "idle" {
            // 会话列表只有粗粒度状态；主回合活跃性默认视为活跃，由种子校准细化。
            state.raw_busy.insert(id.to_string());
            state.apply_effective_busy(id);
        }
    }

    state.baseline_complete = true;
    Ok(())
}

/// 启动种子：对每个忙碌会话取一次详情（含 main_turn_active），之后的状态全靠推送。
async fn seed_busy_sessions(
    client: &reqwest::Client,
    server: &ServerConfig,
    base: url::Url,
    state: &mut ProtocolState,
    token: &CancellationToken,
) {
    let ids: Vec<String> = state.busy.iter().cloned().collect();
    for id in ids {
        let Some(url) = base.join(&format!("/api/v1/sessions/{}", id)).ok() else {
            continue;
        };
        let Some(text) = fetch_text(client, server, url, token).await else {
            continue;
        };
        let Some(detail) = parse_session_detail(&text) else {
            continue;
        };
        if detail.busy {
            state.raw_busy.insert(id.clone());
        } else {
            state.raw_busy.remove(&id);
        }
        if detail.main_turn_active {
            state.main_turn_inactive.remove(&id);
        } else {
            state.main_turn_inactive.insert(id.clone());
        }
        if !detail.main_turn_active {
            // 主回合已结束：取一次后台任务数作为事件计数的初值。
            if let Ok(tasks_url) = base.join(&format!("/api/v1/sessions/{}/tasks", id)) {
                if let Some(text) = fetch_text(client, server, tasks_url, token).await {
                    let running = count_running_tasks(&text);
                    if running > 0 {
                        state.bg_running.insert(id.clone(), running);
                    } else {
                        state.bg_running.remove(&id);
                    }
                }
            }
        }
        state.apply_effective_busy(&id);
        // 种子阶段同步待交互状态，活动行直接显示「等待审批/等待回答」。
        if state.busy.contains(&id) {
            state.activities.entry(id.clone()).or_default().pending = detail.pending.clone();
        }
    }
}

async fn fetch_text(
    client: &reqwest::Client,
    server: &ServerConfig,
    url: url::Url,
    token: &CancellationToken,
) -> Option<String> {
    let mut req = client.get(url);
    if !server.token.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", server.token));
    }
    let resp = cancellable_request(async { req.send().await.map_err(|e| e.to_string()) }, token)
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.text().await.ok()
}

/// 会话详情种子：服务器侧忙碌、主回合活跃性与待交互状态。
#[derive(Debug, PartialEq)]
struct SessionDetail {
    busy: bool,
    main_turn_active: bool,
    pending: Option<String>,
}

fn parse_session_detail(text: &str) -> Option<SessionDetail> {
    let value: Value = serde_json::from_str(text).ok()?;
    let data = value.get("data")?;
    let busy = data.get("busy").and_then(|v| v.as_bool()).unwrap_or(false);
    let main_turn_active = data
        .get("main_turn_active")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let pending = data
        .get("pending_interaction")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|p| p == "approval" || p == "question");
    Some(SessionDetail {
        busy,
        main_turn_active,
        pending,
    })
}

/// 统计运行中的后台任务，决定「等后台」还是「已完成」。
fn count_running_tasks(text: &str) -> u32 {
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return 0;
    };
    let items = value
        .pointer("/data/items")
        .and_then(|v| v.as_array())
        .or_else(|| value.get("data").and_then(|v| v.as_array()));
    items
        .map(|arr| {
            arr.iter()
                .filter(|task| task.get("status").and_then(|v| v.as_str()) == Some("running"))
                .count() as u32
        })
        .unwrap_or(0)
}

fn build_sessions_url(base: url::Url) -> Result<url::Url, MonitorError> {
    let mut url = base.join("/api/v2/sessions").map_err(MonitorError::Url)?;
    url.query_pairs_mut()
        .append_pair("meta.archived", "false")
        .append_pair("page_size", "100");
    Ok(url)
}

async fn read_loop(
    client: &reqwest::Client,
    mut ws_stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
    server: &ServerConfig,
    update_tx: &mpsc::Sender<MonitorUpdate>,
    state: &mut ProtocolState,
    token: &CancellationToken,
) -> Result<(), MonitorError> {
    // 定期主动发送 ping，避免服务端/中间设备因空闲断开连接。
    let mut keepalive = tokio::time::interval(Duration::from_secs(20));
    keepalive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            msg = ws_stream.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        match handle_message(&text, client, server, state, &mut ws_stream, token).await {
                            Ok(events) => {
                                emit_events(update_tx, events).await;
                            }
                            Err(e) => return Err(e),
                        }
                        // 新出现的会话（如 event.session.created）补进基础订阅。
                        let new_ids: Vec<String> = state
                            .titles
                            .keys()
                            .filter(|id| !state.subscribed.contains(*id))
                            .cloned()
                            .collect();
                        if !new_ids.is_empty() {
                            ws_stream
                                .send(Message::Text(subscribe(&new_ids).to_string().into()))
                                .await
                                .map_err(|e| MonitorError::Ws(e.to_string()))?;
                            state.subscribed.extend(new_ids);
                        }
                        send_status(update_tx, &server.id, true, state, None);
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        return Ok(());
                    }
                    Some(Ok(Message::Ping(_))) => {
                        // tokio-tungstenite answers pong automatically.
                    }
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Binary(_))) => {}
                    Some(Ok(Message::Frame(_))) => {}
                    Some(Err(e)) => return Err(MonitorError::Ws(e.to_string())),
                }
            }
            _ = keepalive.tick() => {
                if ws_stream
                    .send(Message::Ping(Vec::<u8>::new().into()))
                    .await
                    .is_err()
                {
                    return Err(MonitorError::Ws("keepalive ping failed".to_string()));
                }
            }
            () = token.cancelled() => {
                let _ = ws_stream.close(None).await;
                return Ok(());
            }
        }
    }
}

async fn handle_message(
    text: &str,
    client: &reqwest::Client,
    server: &ServerConfig,
    state: &mut ProtocolState,
    ws_stream: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    token: &CancellationToken,
) -> Result<Vec<AgentEvent>, MonitorError> {
    let msg: Value = serde_json::from_str(text)
        .map_err(|e| MonitorError::Protocol(format!("invalid JSON: {}", e)))?;
    let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or_default();

    match msg_type {
        "ping" => {
            let nonce = msg
                .get("payload")
                .and_then(|p| p.get("nonce"))
                .cloned()
                .unwrap_or_else(|| json!(uuid::Uuid::new_v4().to_string()));
            let pong = json!({
                "type": "pong",
                "payload": { "nonce": nonce },
            });
            ws_stream
                .send(Message::Text(pong.to_string().into()))
                .await
                .map_err(|e| MonitorError::Ws(e.to_string()))?;
            Ok(Vec::new())
        }
        "resync_required" => {
            let base = server.base_url().map_err(MonitorError::Config)?;
            fetch_baseline(client, server, base.clone(), state, token)
                .await
                .map_err(|e| MonitorError::Protocol(e.to_string()))?;
            seed_busy_sessions(client, server, base, state, token).await;
            let ids: Vec<String> = state.titles.keys().cloned().collect();
            let sub = subscribe(&ids);
            ws_stream
                .send(Message::Text(sub.to_string().into()))
                .await
                .map_err(|e| MonitorError::Ws(e.to_string()))?;
            state.subscribed = ids.into_iter().collect();
            Ok(Vec::new())
        }
        _ => parse_frame(&server.id, text, Utc::now(), state)
            .map_err(|e| MonitorError::Protocol(e.to_string())),
    }
}

async fn connect_ws(
    ws_url: &url::Url,
    server: &ServerConfig,
) -> Result<
    (
        WebSocketStream<MaybeTlsStream<TcpStream>>,
        http::Response<Option<Vec<u8>>>,
    ),
    tokio_tungstenite::tungstenite::Error,
> {
    use base64::Engine;

    let host = ws_url.host_str().unwrap_or("localhost");
    let port = ws_url
        .port_or_known_default()
        .unwrap_or(if ws_url.scheme() == "wss" { 443 } else { 80 });

    let stream = TcpStream::connect((host, port)).await?;
    let stream: MaybeTlsStream<TcpStream> = if ws_url.scheme() == "wss" {
        let cert_result = rustls_native_certs::load_native_certs();
        if !cert_result.errors.is_empty() {
            let msg = cert_result
                .errors
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(tokio_tungstenite::tungstenite::Error::Io(
                std::io::Error::other(msg),
            ));
        }
        let mut root_store = tokio_rustls::rustls::RootCertStore::empty();
        root_store.add_parsable_certificates(cert_result.certs);
        let config = tokio_rustls::rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth();
        let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(config));
        let server_name =
            tokio_rustls::rustls::pki_types::ServerName::try_from(host.to_string())
                .map_err(|e| tokio_tungstenite::tungstenite::Error::Io(std::io::Error::other(e)))?;
        MaybeTlsStream::Rustls(connector.connect(server_name, stream).await?)
    } else {
        MaybeTlsStream::Plain(stream)
    };

    let key = base64::engine::general_purpose::STANDARD.encode(uuid::Uuid::new_v4().as_bytes());
    let mut request = http::Request::builder()
        .method("GET")
        .uri(ws_url.as_str())
        .header("Host", format!("{}:{}", host, port))
        .header("Upgrade", "websocket")
        .header("Connection", "Upgrade")
        .header("Sec-WebSocket-Key", key)
        .header("Sec-WebSocket-Version", "13");
    if !server.token.is_empty() {
        request = request.header(
            "Sec-WebSocket-Protocol",
            format!("kimi-code.bearer.{}", server.token),
        );
    }
    let request = request.body(()).expect("valid request");

    let config = WebSocketConfig::default();
    tokio_tungstenite::client_async_with_config(request, stream, Some(config)).await
}

fn ws_url(server: &ServerConfig) -> Result<url::Url, MonitorError> {
    let base = server.base_url().map_err(MonitorError::Config)?;
    let scheme = match base.scheme() {
        "https" => "wss",
        _ => "ws",
    };
    let mut url = base.clone();
    url.set_scheme(scheme).map_err(|_| MonitorError::UrlParse)?;
    url.set_path("/api/v1/ws");
    Ok(url)
}

fn client_hello(session_ids: &[String]) -> Value {
    json!({
        "type": "client_hello",
        "id": uuid::Uuid::new_v4().to_string(),
        "payload": {
            "client_id": "agentpocket-desktop",
            "subscriptions": session_ids,
            "cursors": {},
        },
    })
}

fn subscribe(session_ids: &[String]) -> Value {
    json!({
        "type": "subscribe",
        "id": uuid::Uuid::new_v4().to_string(),
        "payload": {
            "session_ids": session_ids,
            "cursors": {},
        },
    })
}

#[derive(Debug, thiserror::Error)]
enum MonitorError {
    #[error("config error: {0}")]
    Config(#[source] crate::model::ValidationError),
    #[error("url error: {0}")]
    Url(#[source] url::ParseError),
    #[error("url parse error")]
    UrlParse,
    #[error("http error: {0}")]
    Http(String),
    #[error("websocket error: {0}")]
    Ws(String),
    #[error("protocol error: {0}")]
    Protocol(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Backend, ServerConfig};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn parse_session_detail_extracts_busy_and_main_turn() {
        let text = r#"{"data":{"busy":true,"main_turn_active":false,"pending_interaction":"approval"}}"#;
        let detail = parse_session_detail(text).unwrap();
        assert!(detail.busy);
        assert!(!detail.main_turn_active);
        assert_eq!(detail.pending.as_deref(), Some("approval"));
        // main_turn_active 缺省视为活跃；pending 为 none 时记 None。
        let missing = r#"{"data":{"busy":true,"pending_interaction":"none"}}"#;
        let detail = parse_session_detail(missing).unwrap();
        assert!(detail.busy);
        assert!(detail.main_turn_active);
        assert_eq!(detail.pending, None);
        assert_eq!(parse_session_detail("{}"), None);
        assert_eq!(parse_session_detail("not json"), None);
    }

    #[test]
    fn count_running_tasks_counts_only_running() {
        let text = r#"{"data":{"items":[{"status":"running"},{"status":"completed"},{"status":"running"}]}}"#;
        assert_eq!(count_running_tasks(text), 2);
        // 兼容 data 直接是数组的老格式。
        let arr = r#"{"data":[{"status":"running"},{"status":"failed"}]}"#;
        assert_eq!(count_running_tasks(arr), 1);
        assert_eq!(count_running_tasks("not json"), 0);
    }

    #[test]
    fn ws_url_selects_ws_for_http_base_url() {
        let server = ServerConfig::new("s1", "S", "127.0.0.1", 3080, "", Backend::Kimi);
        let ws = ws_url(&server).unwrap();
        assert_eq!(ws.scheme(), "ws");
        assert_eq!(ws.path(), "/api/v1/ws");
    }

    #[tokio::test]
    async fn connect_ws_selects_tls_path_for_wss() {
        // Spin up a plain TCP server so the TLS handshake fails deterministically.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let server_task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 64];
            let _ = stream.read(&mut buf).await;
            // Respond with plain HTTP, which causes the TLS client to fail handshake.
            let _ = stream
                .write_all(b"HTTP/1.1 400 Not WebSocket\r\n\r\n")
                .await;
        });

        let server = ServerConfig::new("s1", "S", "127.0.0.1", port, "token", Backend::Kimi);
        let mut url = server.base_url().unwrap();
        url.set_scheme("wss").unwrap();
        url.set_path("/api/v1/ws");

        let err = connect_ws(&url, &server).await.unwrap_err();
        let err_str = err.to_string();
        // Rustls TLS handshake errors indicate the TLS path was selected.
        assert!(
            err_str.contains("tls")
                || err_str.contains("TLS")
                || err_str.contains("handshake")
                || err_str.contains("InvalidContentType")
                || err_str.contains("corrupt message"),
            "expected TLS handshake error, got: {}",
            err_str
        );

        let _ = tokio::time::timeout(Duration::from_secs(1), server_task).await;
    }
}
