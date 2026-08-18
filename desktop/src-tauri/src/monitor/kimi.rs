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

    // 1. Fetch baseline session list.
    fetch_baseline(client, server, base.clone(), state, token).await?;
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

    // 3. Send client_hello with subscriptions and cursors.
    let session_ids: Vec<String> = state.titles.keys().cloned().collect();
    let hello = client_hello(&session_ids);
    ws_stream
        .send(Message::Text(hello.to_string().into()))
        .await
        .map_err(|e| MonitorError::Ws(e.to_string()))?;

    read_loop(client, ws_stream, server, update_tx, state, token).await
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
    client: &reqwest::Client,
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
                        match handle_message(&text, client, server, state, &mut ws_stream, token).await {
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
            fetch_baseline(client, server, base, state, token)
                .await
                .map_err(|e| MonitorError::Protocol(e.to_string()))?;
            let ids: Vec<String> = state.titles.keys().cloned().collect();
            let sub = subscribe(&ids);
            ws_stream
                .send(Message::Text(sub.to_string().into()))
                .await
                .map_err(|e| MonitorError::Ws(e.to_string()))?;
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
