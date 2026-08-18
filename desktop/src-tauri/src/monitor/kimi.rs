use std::time::Duration;

use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::protocol::{Message, WebSocketConfig},
};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tokio_util::sync::CancellationToken;

use crate::model::{AgentEvent, ServerConfig};
use crate::monitor::{
    cancellable_sleep, emit_events, send_status, MonitorUpdate, ReconnectBackoff,
};
use crate::protocol::kimi::parse_frame;
use crate::protocol::ProtocolState;

pub async fn run(
    server: ServerConfig,
    update_tx: mpsc::Sender<MonitorUpdate>,
    token: CancellationToken,
) {
    let mut backoff = ReconnectBackoff::default();
    let mut state = ProtocolState::default();

    loop {
        if token.is_cancelled() {
            break;
        }

        match run_once(&server, &update_tx, &mut state, &token).await {
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
    server: &ServerConfig,
    update_tx: &mpsc::Sender<MonitorUpdate>,
    state: &mut ProtocolState,
    token: &CancellationToken,
) -> Result<(), MonitorError> {
    let base = server.base_url().map_err(MonitorError::Config)?;

    // 1. Fetch baseline session list.
    fetch_baseline(server, base.clone(), state).await?;
    send_status(update_tx, &server.id, false, state, None);

    // 2. Connect WebSocket with optional subprotocol header.
    let ws_url = ws_url(server)?;
    let mut request = http::Request::builder().uri(ws_url.to_string());
    if !server.token.is_empty() {
        request = request.header(
            "Sec-WebSocket-Protocol",
            format!("kimi-code.bearer.{}", server.token),
        );
    }
    let request = request
        .body(())
        .map_err(|e| MonitorError::Http(format!("request build: {}", e)))?;

    let config = WebSocketConfig::default();
    let (mut ws_stream, response) = connect_async_with_config(request, Some(config), false)
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

    // 3. Send client_hello with subscriptions and cursors.
    let session_ids: Vec<String> = state.titles.keys().cloned().collect();
    let hello = client_hello(&session_ids);
    ws_stream
        .send(Message::Text(hello.to_string().into()))
        .await
        .map_err(|e| MonitorError::Ws(e.to_string()))?;

    read_loop(ws_stream, server, update_tx, state, token).await
}

async fn fetch_baseline(
    server: &ServerConfig,
    base: url::Url,
    state: &mut ProtocolState,
) -> Result<(), MonitorError> {
    let list_url = build_sessions_url(base)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| MonitorError::Http(e.to_string()))?;
    let mut req = client.get(list_url);
    if !server.token.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", server.token));
    }

    let resp = req
        .send()
        .await
        .map_err(|e| MonitorError::Http(e.to_string()))?;
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
            state.busy.insert(id.to_string());
        }
    }

    state.baseline_complete = true;
    Ok(())
}

fn build_sessions_url(base: url::Url) -> Result<url::Url, MonitorError> {
    let mut url = base.join("/api/v2/sessions").map_err(MonitorError::Url)?;
    url.query_pairs_mut()
        .append_pair("meta.archived", "false")
        .append_pair("page_size", "100");
    Ok(url)
}

async fn read_loop(
    mut ws_stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
    server: &ServerConfig,
    update_tx: &mpsc::Sender<MonitorUpdate>,
    state: &mut ProtocolState,
    token: &CancellationToken,
) -> Result<(), MonitorError> {
    loop {
        tokio::select! {
            msg = ws_stream.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        match handle_message(&text, server, update_tx, state, &mut ws_stream).await {
                            Ok(events) => {
                                emit_events(update_tx, events).await;
                            }
                            Err(e) => return Err(e),
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
            () = token.cancelled() => {
                let _ = ws_stream.close(None).await;
                return Ok(());
            }
        }
    }
}

async fn handle_message(
    text: &str,
    server: &ServerConfig,
    _update_tx: &mpsc::Sender<MonitorUpdate>,
    state: &mut ProtocolState,
    ws_stream: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
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
            if let Ok(base) = server.base_url() {
                let _ = fetch_baseline(server, base, state).await;
                let ids: Vec<String> = state.titles.keys().cloned().collect();
                let sub = subscribe(&ids);
                ws_stream
                    .send(Message::Text(sub.to_string().into()))
                    .await
                    .map_err(|e| MonitorError::Ws(e.to_string()))?;
            }
            Ok(Vec::new())
        }
        _ => parse_frame(&server.id, text, Utc::now(), state)
            .map_err(|e| MonitorError::Protocol(e.to_string())),
    }
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
