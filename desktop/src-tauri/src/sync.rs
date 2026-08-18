use std::io::Read as _;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tiny_http::{Method, Request, Response, Server, StatusCode};
use uuid::Uuid;

use crate::commands::{preview_from_content, AppState, CommandError, ImportPreview};
use crate::config::ExportFormat;

/// 同步服务器自动停止时限：二维码过期后不再接受手机请求。
const SYNC_SERVER_TTL: Duration = Duration::from_secs(10 * 60);
/// recv 轮询间隔，保证停止信号和超时能被及时检查。
const POLL_INTERVAL: Duration = Duration::from_millis(200);
/// POST body 上限，防止异常客户端撑爆内存。
const MAX_BODY_BYTES: u64 = 1024 * 1024;

/// 前端展示用：一个候选对外地址及其预先渲染好的 URL 和二维码 SVG。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncOption {
    pub address: String,
    pub label: String,
    pub url: String,
    pub qr_svg: String,
}

/// 前端展示用：默认选中的地址 + 全部候选地址。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncInfo {
    pub selected: String,
    pub options: Vec<SyncOption>,
}

/// 运行中的同步服务器句柄。Drop 时会发停止信号并 join 线程。
pub struct SyncServerHandle {
    options: Vec<SyncOption>,
    port: u16,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl SyncServerHandle {
    pub fn url(&self) -> &str {
        &self.options[0].url
    }

    pub fn options(&self) -> &[SyncOption] {
        &self.options
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for SyncServerHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// 启动一次性同步服务器。回调只依赖普通闭包而非 AppHandle，便于单测。
pub fn start_server(
    state: Arc<AppState>,
    on_fetched: Box<dyn Fn() + Send>,
    on_received: Box<dyn Fn(ImportPreview) + Send>,
) -> Result<SyncServerHandle, CommandError> {
    let server = Server::http(("0.0.0.0", 0))
        .map_err(|e| CommandError::Sync(format!("failed to bind sync server: {e}")))?;
    let port = match server.server_addr() {
        tiny_http::ListenAddr::IP(addr) => addr.port(),
        other => {
            return Err(CommandError::Sync(format!(
                "unexpected sync server address: {other:?}"
            )))
        }
    };

    let token = Uuid::new_v4();
    let options = enumerate_candidates()
        .into_iter()
        .map(|(ip, label)| {
            let url = format!("agentpocket://sync?host={ip}&port={port}&token={token}");
            let qr_svg = render_qr_svg(&url)?;
            Ok(SyncOption {
                address: ip.to_string(),
                label: label.to_string(),
                url,
                qr_svg,
            })
        })
        .collect::<Result<Vec<_>, CommandError>>()?;

    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_thread = Arc::clone(&stop);
    let join = std::thread::spawn(move || {
        run_server(server, state, token, on_fetched, on_received, stop_for_thread);
    });

    Ok(SyncServerHandle {
        options,
        port,
        stop,
        join: Some(join),
    })
}

fn run_server(
    server: Server,
    state: Arc<AppState>,
    token: Uuid,
    on_fetched: Box<dyn Fn() + Send>,
    on_received: Box<dyn Fn(ImportPreview) + Send>,
    stop: Arc<AtomicBool>,
) {
    let deadline = Instant::now() + SYNC_SERVER_TTL;
    loop {
        if stop.load(Ordering::SeqCst) || Instant::now() >= deadline {
            break;
        }
        match server.recv_timeout(POLL_INTERVAL) {
            Ok(Some(request)) => handle_request(request, &state, &token, &on_fetched, &on_received),
            Ok(None) => continue,
            Err(_) => break,
        }
    }
}

fn handle_request(
    mut request: Request,
    state: &Arc<AppState>,
    token: &Uuid,
    on_fetched: &(dyn Fn() + Send),
    on_received: &(dyn Fn(ImportPreview) + Send),
) {
    let (path, query_token) = split_url(request.url());
    if path != "/config" {
        let _ = request.respond(Response::empty(StatusCode(404)));
        return;
    }
    let token_str = token.to_string();
    let header_ok = request.headers().iter().any(|header| {
        header.field.to_string().eq_ignore_ascii_case("X-Sync-Token")
            && header.value.as_str() == token_str
    });
    let query_ok = query_token.as_deref() == Some(token_str.as_str());
    if !header_ok && !query_ok {
        let _ = request.respond(Response::empty(StatusCode(403)));
        return;
    }

    match *request.method() {
        Method::Get => {
            let config = state
                .config
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match state.store.export_text(&config, ExportFormat::Android) {
                Ok(text) => {
                    let header = tiny_http::Header::from_bytes(
                        "Content-Type",
                        "application/json; charset=utf-8",
                    )
                    .expect("static header is valid");
                    let _ = request.respond(
                        Response::from_string(text)
                            .with_status_code(StatusCode(200))
                            .with_header(header),
                    );
                    on_fetched();
                }
                Err(error) => {
                    let _ = request.respond(
                        Response::from_string(error.to_string())
                            .with_status_code(StatusCode(500)),
                    );
                }
            }
        }
        Method::Post => {
            let mut body = String::new();
            let read_result = request
                .as_reader()
                .take(MAX_BODY_BYTES)
                .read_to_string(&mut body);
            match read_result {
                Ok(_) => match preview_from_content(state, &body) {
                    Ok(preview) => {
                        on_received(preview);
                        let _ = request.respond(Response::empty(StatusCode(202)));
                    }
                    Err(error) => {
                        let _ = request.respond(
                            Response::from_string(error.to_string())
                                .with_status_code(StatusCode(400)),
                        );
                    }
                },
                Err(_) => {
                    let _ = request.respond(Response::empty(StatusCode(400)));
                }
            }
        }
        _ => {
            let _ = request.respond(Response::empty(StatusCode(405)));
        }
    }
}

/// 把 request.url()（形如 `/config?token=…`）拆成路径和 token 查询参数。
fn split_url(url: &str) -> (&str, Option<String>) {
    let (path, query) = url.split_once('?').unwrap_or((url, ""));
    let token = url::form_urlencoded::parse(query.as_bytes())
        .find(|(key, _)| key == "token")
        .map(|(_, value)| value.into_owned());
    (path, token)
}

/// 枚举全部候选对外地址并分类排序：Tailscale 在前，局域网其次；只有没有
/// 其他地址时才包含回环地址（仅供调试）。
fn enumerate_candidates() -> Vec<(Ipv4Addr, &'static str)> {
    let entries = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|interface| match interface.addr {
            if_addrs::IfAddr::V4(v4) => Some((interface.name, v4.ip)),
            _ => None,
        })
        .collect();
    classify_ips(entries)
}

/// 对 (接口名, IPv4) 列表分类排序。分类规则：
/// - 100.64.0.0/10 网段或 tailscale 接口名 → Tailscale
/// - 其他非回环 IPv4 → 局域网
/// - 回环 127.0.0.1 → 仅在没有其他地址时返回
fn classify_ips(entries: Vec<(String, Ipv4Addr)>) -> Vec<(Ipv4Addr, &'static str)> {
    let mut tailscale = Vec::new();
    let mut lan = Vec::new();
    for (name, ip) in entries {
        if ip.is_loopback() {
            continue;
        }
        let is_tailscale = name.to_ascii_lowercase().contains("tailscale")
            || (ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1]));
        if is_tailscale {
            tailscale.push((ip, "Tailscale(推荐)"));
        } else {
            lan.push((ip, "局域网"));
        }
    }
    tailscale.extend(lan);
    if tailscale.is_empty() {
        tailscale.push((Ipv4Addr::LOCALHOST, "本机(仅调试)"));
    }
    tailscale
}

fn render_qr_svg(content: &str) -> Result<String, CommandError> {
    let code = qrcode::QrCode::new(content.as_bytes())
        .map_err(|e| CommandError::Sync(format!("failed to build QR code: {e}")))?;
    Ok(code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(200, 200)
        .quiet_zone(true)
        .build())
}

// ------------------------------------------------------------------
// Commands
// ------------------------------------------------------------------

#[tauri::command]
pub fn start_sync_server(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
) -> Result<SyncInfo, CommandError> {
    // 已有运行中的同步服务器时先停旧的，避免旧 token 继续有效。
    {
        let mut guard = state
            .sync_server
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(old) = guard.take() {
            old.stop();
        }
    }

    let app_for_fetched = app.clone();
    let app_for_received = app.clone();
    let handle = start_server(
        Arc::clone(&state),
        Box::new(move || {
            let _ = app_for_fetched.emit("phone-config-fetched", ());
        }),
        Box::new(move |preview| {
            if let Some(window) = app_for_received.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
            let _ = app_for_received.emit("phone-config-received", preview);
        }),
    )?;

    let info = SyncInfo {
        selected: handle.options()[0].address.clone(),
        options: handle.options().to_vec(),
    };

    let mut guard = state
        .sync_server
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = Some(handle);
    Ok(info)
}

#[tauri::command]
pub fn stop_sync_server(state: State<'_, Arc<AppState>>) -> Result<(), CommandError> {
    let mut guard = state
        .sync_server
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(handle) = guard.take() {
        handle.stop();
    }
    Ok(())
}

// ------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigStore;
    use crate::model::{AppConfig, Backend, ServerConfig};
    use crate::monitor::MonitorManager;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::sync::Mutex;
    use tempfile::tempdir;
    use tokio::sync::mpsc;

    fn sample_state() -> Arc<AppState> {
        let config = AppConfig {
            active_id: Some("s1".to_string()),
            servers: vec![ServerConfig::new(
                "s1",
                "Work",
                "100.64.0.2",
                3080,
                "secret-token",
                Backend::Dsh,
            )],
            ..AppConfig::default()
        };
        let dir = tempdir().unwrap();
        let store = ConfigStore::new(dir.path().to_path_buf());
        let (tx, _) = mpsc::channel(4);
        Arc::new(AppState::new(config, store, MonitorManager::new(tx)))
    }

    /// 发送一个原始 HTTP 请求，返回 (状态码, body)。
    fn raw_request(port: u16, request: &str) -> (u16, String) {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        let text = String::from_utf8_lossy(&response).into_owned();
        let status = text
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse::<u16>().ok())
            .expect("response has a status code");
        let body = text
            .split_once("\r\n\r\n")
            .map(|(_, body)| body.to_string())
            .unwrap_or_default();
        (status, body)
    }

    fn token_from_url(url: &str) -> &str {
        url.split("token=").nth(1).expect("url contains token")
    }

    #[test]
    fn get_returns_android_array_and_invokes_on_fetched() {
        let state = sample_state();
        let fetched = Arc::new(Mutex::new(0_usize));
        let fetched_for_callback = Arc::clone(&fetched);
        let handle = start_server(
            state,
            Box::new(move || {
                *fetched_for_callback.lock().unwrap() += 1;
            }),
            Box::new(|_| panic!("on_received must not fire for GET")),
        )
        .unwrap();
        let token = token_from_url(handle.url());

        let (status, body) = raw_request(
            handle.port(),
            &format!("GET /config?token={token} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"),
        );

        assert_eq!(status, 200);
        let servers: serde_json::Value = serde_json::from_str(&body).unwrap();
        let array = servers.as_array().expect("body is a JSON array");
        assert_eq!(array.len(), 1);
        assert_eq!(array[0]["name"], "Work");
        assert_eq!(array[0]["host"], "100.64.0.2");
        assert_eq!(*fetched.lock().unwrap(), 1);
    }

    #[test]
    fn get_with_bad_token_returns_403() {
        let state = sample_state();
        let handle = start_server(state, Box::new(|| {}), Box::new(|_| {})).unwrap();

        let (status, _) = raw_request(
            handle.port(),
            "GET /config?token=wrong-token HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        );
        assert_eq!(status, 403);

        let (status, _) = raw_request(
            handle.port(),
            "GET /config HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        );
        assert_eq!(status, 403);
    }

    #[test]
    fn post_valid_array_triggers_on_received_with_preview() {
        let state = sample_state();
        let received = Arc::new(Mutex::new(None));
        let received_for_callback = Arc::clone(&received);
        let handle = start_server(
            Arc::clone(&state),
            Box::new(|| {}),
            Box::new(move |preview| {
                *received_for_callback.lock().unwrap() = Some(preview);
            }),
        )
        .unwrap();
        let token = token_from_url(handle.url());

        let body = r#"[{"id":"p1","name":"Phone","host":"100.64.0.9","port":3080,"token":"t","backend":"kimi"},{"name":"Bad","host":"http://host","port":0,"backend":"dsh"}]"#;
        let (status, _) = raw_request(
            handle.port(),
            &format!(
                "POST /config HTTP/1.1\r\nHost: localhost\r\nX-Sync-Token: {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ),
        );

        assert_eq!(status, 202);
        let preview = received.lock().unwrap().clone().expect("preview received");
        assert_eq!(preview.valid_count, 1);
        assert_eq!(preview.invalid.len(), 1);
        // preview 已登记到 AppState，可供 apply_import 使用。
        assert!(state
            .import_previews
            .lock()
            .unwrap()
            .contains_key(&preview.import_id));
    }

    #[test]
    fn post_garbage_body_returns_400() {
        let state = sample_state();
        let handle = start_server(
            state,
            Box::new(|| {}),
            Box::new(|_| panic!("on_received must not fire for invalid body")),
        )
        .unwrap();
        let token = token_from_url(handle.url());

        let body = "not json at all";
        let (status, _) = raw_request(
            handle.port(),
            &format!(
                "POST /config?token={token} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ),
        );

        assert_eq!(status, 400);
    }

    #[test]
    fn classify_ips_sorts_tailscale_before_lan_and_skips_loopback() {
        let candidates = classify_ips(vec![
            ("eth0".to_string(), Ipv4Addr::new(192, 168, 1, 10)),
            ("lo".to_string(), Ipv4Addr::LOCALHOST),
            ("tailscale0".to_string(), Ipv4Addr::new(100, 64, 0, 2)),
            ("wlan0".to_string(), Ipv4Addr::new(10, 0, 0, 5)),
        ]);

        assert_eq!(
            candidates,
            vec![
                (Ipv4Addr::new(100, 64, 0, 2), "Tailscale(推荐)"),
                (Ipv4Addr::new(192, 168, 1, 10), "局域网"),
                (Ipv4Addr::new(10, 0, 0, 5), "局域网"),
            ]
        );
    }

    #[test]
    fn classify_ips_recognizes_cgnat_range_without_tailscale_name() {
        let candidates = classify_ips(vec![
            ("eth0".to_string(), Ipv4Addr::new(100, 127, 255, 254)),
            ("eth1".to_string(), Ipv4Addr::new(100, 128, 0, 1)),
        ]);

        assert_eq!(
            candidates,
            vec![
                (Ipv4Addr::new(100, 127, 255, 254), "Tailscale(推荐)"),
                (Ipv4Addr::new(100, 128, 0, 1), "局域网"),
            ]
        );
    }

    #[test]
    fn classify_ips_falls_back_to_loopback_only_when_no_other_addresses() {
        let candidates = classify_ips(vec![("lo".to_string(), Ipv4Addr::LOCALHOST)]);
        assert_eq!(candidates, vec![(Ipv4Addr::LOCALHOST, "本机(仅调试)")]);

        let candidates = classify_ips(Vec::new());
        assert_eq!(candidates, vec![(Ipv4Addr::LOCALHOST, "本机(仅调试)")]);
    }

    #[test]
    fn handle_options_carry_matching_url_and_qr_for_each_candidate() {
        let state = sample_state();
        let handle = start_server(state, Box::new(|| {}), Box::new(|_| {})).unwrap();

        assert!(!handle.options().is_empty());
        let token = token_from_url(handle.url()).to_string();
        for option in handle.options() {
            assert!(option.url.starts_with("agentpocket://sync?"));
            assert!(option.url.contains(&format!("host={}", option.address)));
            assert!(option.url.contains(&format!("port={}", handle.port())));
            assert!(option.url.contains(&format!("token={token}")));
            assert!(option.qr_svg.contains("<svg"));
            assert!(!option.label.is_empty());
        }
        // url() 返回默认选中（第一个）候选的地址。
        assert!(handle.url().contains(&format!("host={}", handle.options()[0].address)));
    }
}
