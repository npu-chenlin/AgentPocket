use std::time::Duration;

use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tokio_util::sync::CancellationToken;

use crate::model::ServerConfig;
use crate::monitor::{
    cancellable_request, cancellable_sleep, emit_events, send_status, MonitorUpdate,
    ReconnectBackoff,
};
use crate::protocol::dsh::{parse_frame, parse_session_list};
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

    // 1. Fetch session list via HTTP RPC.
    let list_url = base.join("/api/session.list").map_err(MonitorError::Url)?;
    let body = json!({
        "type": "client-request",
        "rpcId": uuid::Uuid::new_v4().to_string(),
        "method": "session.list",
        "payload": {},
    });

    let resp = cancellable_request(
        async {
            client
                .post(list_url)
                .json(&body)
                .send()
                .await
                .map_err(|e| e.to_string())
        },
        token,
    )
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
    parse_session_list(&text, state).map_err(|e| MonitorError::Protocol(e.to_string()))?;
    send_status(update_tx, &server.id, false, state, None);

    // 2. Connect WebSocket.
    let ws_url = ws_url(server)?;
    let (ws_stream, _) = connect_async(ws_url.to_string())
        .await
        .map_err(|e| MonitorError::Ws(e.to_string()))?;

    send_status(update_tx, &server.id, true, state, None);
    read_loop(ws_stream, server, update_tx, state, token).await
}

async fn read_loop(
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
                        let now = Utc::now();
                        match parse_frame(&server.id, &text, now, state) {
                            Ok(events) => emit_events(update_tx, events).await,
                            Err(e) => {
                                return Err(MonitorError::Protocol(e.to_string()));
                            }
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
                    Some(Ok(Message::Binary(_))) => {
                        // dsh streams are text-only; ignore binary frames.
                    }
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

fn ws_url(server: &ServerConfig) -> Result<url::Url, MonitorError> {
    let base = server.base_url().map_err(MonitorError::Config)?;
    let scheme = match base.scheme() {
        "https" => "wss",
        _ => "ws",
    };
    let mut url = base.clone();
    url.set_scheme(scheme).map_err(|_| MonitorError::UrlParse)?;
    url.set_path("/api/events.mux");
    Ok(url)
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
