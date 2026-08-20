//! mesh 面板命令：peer 发现与配置拉取/推送。
//! 复用 core 的 mesh_client/discovery（明文 HTTP，仅限 tailnet 内自家端点）。

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::State;

use agentpocket_core::discovery::{self, MeshPeer, MESH_PORT};
use agentpocket_core::host;
use agentpocket_core::mesh_client;

use crate::commands::{preview_from_content, AppState, CommandError, ImportPreview};
use crate::model::MeshPeerEntry;

/// peer 在线探测（单次 /info 请求）超时。
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
/// 拉取/推送 HTTP 请求超时。
const MESH_HTTP_TIMEOUT: Duration = Duration::from_secs(5);

/// 前端展示用：一个 mesh peer 及其在线状态。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshPeerView {
    pub name: String,
    pub host: String,
    pub version: Option<String>,
    pub online: bool,
    pub manual: bool,
}

/// 推送到对端后的新增/更新计数（对端 /config 应答）。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushCounts {
    pub added: u64,
    pub updated: u64,
}

// 三条命令均为 async：网络 IO 跑在 Tauri 线程池，避免阻塞 UI 主线程。
#[tauri::command]
pub async fn discover_mesh_peers(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<MeshPeerView>, CommandError> {
    Ok(discover_peers(&state))
}

#[tauri::command]
pub async fn mesh_pull(
    state: State<'_, Arc<AppState>>,
    host: String,
) -> Result<ImportPreview, CommandError> {
    mesh_pull_at(&state, &host, MESH_PORT)
}

#[tauri::command]
pub async fn mesh_push(
    state: State<'_, Arc<AppState>>,
    host: String,
) -> Result<PushCounts, CommandError> {
    mesh_push_at(&state, &host, MESH_PORT)
}

/// 发现 peer：tailscale 在线设备 + 手动 peer 合并探测，未应答的手动 peer
/// 补 online:false 行（对端 daemon 可能暂时离线，仍需展示以便推送配置）。
fn discover_peers(state: &AppState) -> Vec<MeshPeerView> {
    let manual = manual_peers(state);
    let mut candidates = discovery::tailscale_candidates(discovery::find_tailscale_binary().as_deref());
    candidates.extend(manual.iter().map(|peer| (peer.host.clone(), peer.name.clone())));
    let probed = discovery::probe_candidates(&candidates, PROBE_TIMEOUT);
    merge_views(&manual, probed)
}

fn manual_peers(state: &AppState) -> Vec<MeshPeerEntry> {
    state
        .config
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .settings
        .mesh_peers
        .clone()
}

/// 合并探测结果与手动 peer 列表：在线结果映射 online:true，host 命中手动
/// 列表的标 manual:true；未应答的手动 peer 补 online:false 行（version 为
/// None）；按 host 去重（在线优先，probe_candidates 已保证在线侧唯一），
/// 结果按 name 排序。
fn merge_views(manual: &[MeshPeerEntry], probed: Vec<MeshPeer>) -> Vec<MeshPeerView> {
    let manual_hosts: HashSet<&str> = manual.iter().map(|p| p.host.as_str()).collect();
    let online_hosts: HashSet<String> = probed.iter().map(|p| p.host.clone()).collect();
    let mut views: Vec<MeshPeerView> = probed
        .into_iter()
        .map(|peer| {
            let is_manual = manual_hosts.contains(peer.host.as_str());
            MeshPeerView {
                name: peer.name,
                host: peer.host,
                version: peer.version,
                online: true,
                manual: is_manual,
            }
        })
        .collect();

    for entry in manual {
        if !online_hosts.contains(&entry.host) {
            views.push(MeshPeerView {
                name: entry.name.clone(),
                host: entry.host.clone(),
                version: None,
                online: false,
                manual: true,
            });
        }
    }

    views.sort_by(|a, b| a.name.cmp(&b.name));
    views
}

/// 从对端拉取配置并注册导入预览。port 参数便于测试注入 mock 端点。
pub(crate) fn mesh_pull_at(
    state: &Arc<AppState>,
    host: &str,
    port: u16,
) -> Result<ImportPreview, CommandError> {
    let response = mesh_client::get(host, port, "/config", &[], MESH_HTTP_TIMEOUT)?;
    if response.status != 200 {
        return Err(CommandError::Mesh(format!(
            "对方返回 HTTP {}：{}",
            response.status, response.body
        )));
    }
    preview_from_content(state, &response.body)
}

/// 把当前配置推送到对端并解析新增/更新计数。port 参数便于测试注入 mock 端点。
pub(crate) fn mesh_push_at(
    state: &Arc<AppState>,
    host: &str,
    port: u16,
) -> Result<PushCounts, CommandError> {
    let text = {
        let config = state
            .config
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.store.export_text(&config)?
    };
    let source = host::hostname();
    let response = mesh_client::post(
        host,
        port,
        "/config",
        &[("X-AgentPocket-Source", source.as_str())],
        &text,
        MESH_HTTP_TIMEOUT,
    )?;
    if response.status != 200 {
        return Err(CommandError::Mesh(format!(
            "对方返回 HTTP {}：{}",
            response.status, response.body
        )));
    }
    // 应答必须携带 added/updated 两个整数；缺失按错误处理，不显示 null。
    serde_json::from_str::<PushCounts>(&response.body)
        .map_err(|e| CommandError::Mesh(format!("推送应答解析失败：{e}")))
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

    /// 启动 mock 对端：对所有请求应答固定状态码和 body，返回监听端口。
    fn spawn_mock(status: u16, body: String) -> u16 {
        let server = tiny_http::Server::http(("127.0.0.1", 0)).unwrap();
        let port = match server.server_addr() {
            tiny_http::ListenAddr::IP(addr) => addr.port(),
            other => panic!("unexpected listen addr: {other:?}"),
        };
        std::thread::spawn(move || {
            for request in server.incoming_requests() {
                let _ = request.respond(
                    tiny_http::Response::from_string(body.clone())
                        .with_status_code(tiny_http::StatusCode(status)),
                );
            }
        });
        port
    }

    #[test]
    fn mesh_pull_registers_preview_and_returns_counts() {
        let state = sample_state();
        let body = r#"{"schema":1,"servers":[{"id":"m1","name":"Peer-A","host":"100.64.0.9","port":3080,"token":"peer-token","backend":"kimi"}]}"#;
        let port = spawn_mock(200, body.to_string());

        let preview = mesh_pull_at(&state, "127.0.0.1", port).unwrap();

        assert_eq!(preview.valid_count, 1);
        assert!(preview.invalid.is_empty());
        // preview 已注册，后续 apply_import 可直接使用。
        assert!(state
            .import_previews
            .lock()
            .unwrap()
            .contains_key(&preview.import_id));
    }

    #[test]
    fn mesh_pull_non_200_returns_error() {
        let state = sample_state();
        let port = spawn_mock(400, "没有可导入的有效服务器".to_string());

        let error = mesh_pull_at(&state, "127.0.0.1", port).unwrap_err();

        assert!(matches!(
            error,
            CommandError::Mesh(ref message) if message.contains("HTTP 400")
        ));
        assert!(error.to_string().contains("没有可导入的有效服务器"));
    }

    #[test]
    fn mesh_push_sends_source_header_and_parses_counts() {
        let state = sample_state();
        // mock 把收到的 X-AgentPocket-Source 头回显进 body，同时记入断言变量。
        let received: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let received_for_mock = Arc::clone(&received);
        let server = tiny_http::Server::http(("127.0.0.1", 0)).unwrap();
        let port = match server.server_addr() {
            tiny_http::ListenAddr::IP(addr) => addr.port(),
            other => panic!("unexpected listen addr: {other:?}"),
        };
        std::thread::spawn(move || {
            for request in server.incoming_requests() {
                let source = request.headers().iter().find(|header| {
                    header
                        .field
                        .as_str()
                        .as_str()
                        .eq_ignore_ascii_case("X-AgentPocket-Source")
                });
                let echo = source.map(|header| header.value.as_str().to_string());
                *received_for_mock.lock().unwrap() = echo.clone();
                // 应答携带 echo 字段：PushCounts 解析应忽略多余字段。
                let body = format!(r#"{{"echo":{},"added":2,"updated":0}}"#, serde_json::json!(echo));
                let _ = request.respond(
                    tiny_http::Response::from_string(body)
                        .with_status_code(tiny_http::StatusCode(200)),
                );
            }
        });

        let counts = mesh_push_at(&state, "127.0.0.1", port).unwrap();

        assert_eq!(counts, PushCounts { added: 2, updated: 0 });
        // 推送请求携带本机主机名作为来源标识。
        assert_eq!(
            received.lock().unwrap().as_deref(),
            Some(host::hostname().as_str())
        );
    }

    #[test]
    fn discover_merges_online_and_offline_manual_peers() {
        // 在线 peer：mock /info 应答身份 JSON（带版本）。
        let info = r#"{"app":"agentpocket","version":"2.8.0","name":"live-host"}"#;
        let online_port = spawn_mock(200, info.to_string());
        // 离线 peer：占用一个端口后立刻关闭，确保连接被拒绝。
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let dead_port = listener.local_addr().unwrap().port();
        drop(listener);

        // 与 probe_candidates 相同的探测逻辑（端口可注入）：在线命中、死端口落空。
        let probed: Vec<MeshPeer> = [
            discovery::probe_peer("127.0.0.1", online_port, "fallback-live", PROBE_TIMEOUT),
            discovery::probe_peer("127.0.0.1", dead_port, "fallback-dead", PROBE_TIMEOUT),
        ]
        .into_iter()
        .flatten()
        .collect();
        assert_eq!(probed.len(), 1);

        // 手动 peer：两台同名主机（考验 host 去重）+ 一台指向离线地址。
        let manual = vec![
            MeshPeerEntry {
                name: "桌面机".to_string(),
                host: "127.0.0.1".to_string(),
            },
            MeshPeerEntry {
                name: "重复登记".to_string(),
                host: "127.0.0.1".to_string(),
            },
            MeshPeerEntry {
                name: "离线机".to_string(),
                host: "100.64.0.99".to_string(),
            },
        ];

        let views = merge_views(&manual, probed);

        assert_eq!(
            views,
            vec![
                // 在线行优先（host 去重后仅一行），带 /info 上报的版本。
                MeshPeerView {
                    name: "live-host".to_string(),
                    host: "127.0.0.1".to_string(),
                    version: Some("2.8.0".to_string()),
                    online: true,
                    manual: true,
                },
                // 未应答的手动 peer 补 online:false 行，version 为 None。
                MeshPeerView {
                    name: "离线机".to_string(),
                    host: "100.64.0.99".to_string(),
                    version: None,
                    online: false,
                    manual: true,
                },
            ]
        );
    }
}
