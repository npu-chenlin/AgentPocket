//! mesh 面板命令：peer 发现与 ~/.kimi-code/config.toml 拉取/推送。
//! 复用 core 的 mesh_client/discovery/kimi_config（明文 HTTP，仅限 tailnet 内自家端点）。

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::State;

use agentpocket_core::discovery::{self, MeshPeer, MESH_PORT};
use agentpocket_core::host;
use agentpocket_core::kimi_config;
use agentpocket_core::mesh_client;

use crate::commands::{AppState, CommandError};
use crate::model::MeshPeerEntry;

/// peer 在线探测（单次 /info 请求）超时。
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);
/// 拉取/推送 HTTP 请求超时。
const MESH_HTTP_TIMEOUT: Duration = Duration::from_secs(5);
/// 重启 kimi web 的超时：带活跃会话时 systemd 停止阶段可能要几十秒。
const RESTART_TIMEOUT: Duration = Duration::from_secs(150);

/// 前端展示用：一个 mesh peer 及其在线状态、Kimi Code CLI / kimi web 状态。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshPeerView {
    pub name: String,
    pub host: String,
    pub version: Option<String>,
    pub online: bool,
    pub manual: bool,
    pub kimi_version: Option<String>,
    pub web_active: bool,
    pub web_port: Option<u16>,
}

/// config.toml 同步完成回执。
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KimiSyncResult {
    pub bytes: usize,
    /// 覆盖时是否生成了旧文件备份（仅 pull 有意义）。
    pub backed_up: bool,
}

// 三条命令均为 async：网络 IO 跑在 Tauri 线程池，避免阻塞 UI 主线程。
#[tauri::command]
pub async fn discover_mesh_peers(
    state: State<'_, Arc<AppState>>,
) -> Result<Vec<MeshPeerView>, CommandError> {
    discover_peers(&state).await
}

#[tauri::command]
pub async fn mesh_pull(host: String) -> Result<KimiSyncResult, CommandError> {
    let home = kimi_config::home_dir();
    tokio::task::spawn_blocking(move || mesh_pull_at(&home, &host, MESH_PORT))
        .await
        .map_err(|error| CommandError::Mesh(format!("mesh pull task failed: {error}")))?
}

#[tauri::command]
pub async fn mesh_push(host: String) -> Result<KimiSyncResult, CommandError> {
    let home = kimi_config::home_dir();
    tokio::task::spawn_blocking(move || mesh_push_at(&home, &host, MESH_PORT))
        .await
        .map_err(|error| CommandError::Mesh(format!("mesh push task failed: {error}")))?
}

/// 远端 Kimi Code CLI 升级（daemon /kimi-upgrade，含 npm 归一化；耗时放宽到 11 分钟）。
#[tauri::command]
pub async fn mesh_kimi_upgrade(host: String) -> Result<String, CommandError> {
    let response = tokio::task::spawn_blocking(move || {
        mesh_client::post(
            &host,
            MESH_PORT,
            "/kimi-upgrade",
            &[],
            "",
            Duration::from_secs(660),
        )
    })
    .await
    .map_err(|error| CommandError::Mesh(format!("mesh upgrade task failed: {error}")))??;
    if response.status != 200 {
        return Err(CommandError::Mesh(format!(
            "对方返回 HTTP {}：{}",
            response.status, response.body
        )));
    }
    let value: serde_json::Value = serde_json::from_str(&response.body)
        .map_err(|e| CommandError::Mesh(format!("升级应答解析失败：{e}")))?;
    Ok(match (value["before"].as_str(), value["after"].as_str()) {
        (Some(b), Some(a)) if b == a => format!("已是最新：{a}"),
        (Some(b), Some(a)) => format!("升级完成：{b} -> {a}"),
        (None, Some(a)) => format!("安装完成：{a}"),
        (_, None) => "安装脚本已执行，但未能确认版本".to_string(),
    })
}

/// 远端重启 kimi web 服务（daemon /kimi-web-restart）。
/// 有活跃会话时 daemon 会拒绝；force=true 强制重启（GUI 需二次确认后才传）。
#[tauri::command]
pub async fn mesh_kimi_web_restart(host: String, force: bool) -> Result<String, CommandError> {
    let body = serde_json::json!({ "force": force }).to_string();
    let response = tokio::task::spawn_blocking(move || {
        mesh_client::post(
            &host,
            MESH_PORT,
            "/kimi-web-restart",
            &[],
            &body,
            RESTART_TIMEOUT,
        )
    })
    .await
    .map_err(|error| CommandError::Mesh(format!("mesh restart task failed: {error}")))??;
    if response.status != 200 {
        // daemon 拒绝原因（如活跃会话保护）原样透传给前端
        return Err(CommandError::Mesh(response.body));
    }
    Ok("对方 kimi web 服务已重启".to_string())
}

/// 发现 peer：tailscale 在线设备 + 手动 peer 合并探测，未应答的手动 peer
/// 补 online:false 行（对端 daemon 可能暂时离线，仍需展示以便推送配置）。
/// 在线 peer 追加一次 /kimi-info 探测，补齐 Kimi CLI 版本与 web 服务状态。
async fn discover_peers(state: &AppState) -> Result<Vec<MeshPeerView>, CommandError> {
    let manual = manual_peers(state);
    let views = tokio::task::spawn_blocking(move || {
        let mut candidates =
            discovery::tailscale_candidates(discovery::find_tailscale_binary().as_deref());
        candidates.extend(
            manual
                .iter()
                .map(|peer| (peer.host.clone(), peer.name.clone())),
        );
        let probed = discovery::probe_candidates(&candidates, PROBE_TIMEOUT);
        merge_views(&manual, probed)
    })
    .await
    .map_err(|error| CommandError::Mesh(format!("peer discovery task failed: {error}")))?;

    // /kimi-info is a synchronous core client. Probe a bounded batch at a time
    // so one slow peer cannot occupy the async runtime or create an unbounded
    // number of worker threads.
    let mut views = views;
    let online_indices: Vec<usize> = views
        .iter()
        .enumerate()
        .filter_map(|(index, view)| view.online.then_some(index))
        .collect();
    for batch in online_indices.chunks(4) {
        let jobs: Vec<_> = batch
            .iter()
            .map(|&index| {
                let host = views[index].host.clone();
                tokio::task::spawn_blocking(move || (index, fetch_kimi_info(&host)))
            })
            .collect();
        for job in jobs {
            let (index, (kimi_version, web_active, web_port)) = job
                .await
                .map_err(|error| CommandError::Mesh(format!("peer info task failed: {error}")))?;
            views[index].kimi_version = kimi_version;
            views[index].web_active = web_active;
            views[index].web_port = web_port;
        }
    }
    Ok(views)
}

fn fetch_kimi_info(host: &str) -> (Option<String>, bool, Option<u16>) {
    let Ok(response) = mesh_client::get(host, MESH_PORT, "/kimi-info", &[], PROBE_TIMEOUT) else {
        return (None, false, None);
    };
    if response.status != 200 {
        return (None, false, None);
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&response.body) else {
        return (None, false, None);
    };
    let web = &value["web"];
    (
        value["version"].as_str().map(String::from),
        web["active"].as_bool().unwrap_or(false),
        web["port"].as_u64().and_then(|p| u16::try_from(p).ok()),
    )
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
                kimi_version: None,
                web_active: false,
                web_port: None,
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
                kimi_version: None,
                web_active: false,
                web_port: None,
            });
        }
    }

    views.sort_by(|a, b| a.name.cmp(&b.name));
    views
}

/// 从对端拉取 config.toml 覆盖本地（旧文件备份为 .bak）。port 参数便于测试注入 mock 端点。
pub(crate) fn mesh_pull_at(
    home: &Path,
    host: &str,
    port: u16,
) -> Result<KimiSyncResult, CommandError> {
    let response = mesh_client::get(host, port, "/kimi-config", &[], MESH_HTTP_TIMEOUT)?;
    if response.status != 200 {
        return Err(CommandError::Mesh(format!(
            "对方返回 HTTP {}：{}",
            response.status, response.body
        )));
    }
    let backed_up = kimi_config::config_path(home).exists();
    kimi_config::write(home, &response.body).map_err(CommandError::Mesh)?;
    Ok(KimiSyncResult {
        bytes: response.body.len(),
        backed_up,
    })
}

/// 把本地 config.toml 推送到对端。port 参数便于测试注入 mock 端点。
pub(crate) fn mesh_push_at(
    home: &Path,
    host: &str,
    port: u16,
) -> Result<KimiSyncResult, CommandError> {
    let text = kimi_config::read(home).map_err(CommandError::Mesh)?;
    let source = host::hostname();
    let response = mesh_client::post(
        host,
        port,
        "/kimi-config",
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
    Ok(KimiSyncResult {
        bytes: text.len(),
        backed_up: false,
    })
}

// ------------------------------------------------------------------
// Tests
// ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::tempdir;

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

    fn seed_kimi_config(home: &Path, content: &str) {
        std::fs::create_dir_all(home.join(".kimi-code")).unwrap();
        std::fs::write(home.join(".kimi-code/config.toml"), content).unwrap();
    }

    #[test]
    fn mesh_pull_writes_remote_config_and_backs_up() {
        let home = tempdir().unwrap();
        seed_kimi_config(home.path(), "model = \"old\"\n");
        let port = spawn_mock(200, "model = \"remote\"\n".to_string());

        let result = mesh_pull_at(home.path(), "127.0.0.1", port).unwrap();

        assert_eq!(result.bytes, "model = \"remote\"\n".len());
        assert!(result.backed_up);
        assert_eq!(
            std::fs::read_to_string(home.path().join(".kimi-code/config.toml")).unwrap(),
            "model = \"remote\"\n"
        );
        assert_eq!(
            std::fs::read_to_string(home.path().join(".kimi-code/config.toml.bak")).unwrap(),
            "model = \"old\"\n"
        );
    }

    #[test]
    fn mesh_pull_non_200_returns_error() {
        let home = tempdir().unwrap();
        let port = spawn_mock(404, "读取失败".to_string());

        let error = mesh_pull_at(home.path(), "127.0.0.1", port).unwrap_err();

        assert!(matches!(
            error,
            CommandError::Mesh(ref message) if message.contains("HTTP 404")
        ));
    }

    #[test]
    fn mesh_push_sends_local_config_with_source_header() {
        let home = tempdir().unwrap();
        seed_kimi_config(home.path(), "model = \"local\"\n");
        // mock 记录收到的 body 与 X-AgentPocket-Source 头。
        let received: Arc<Mutex<(String, Option<String>)>> =
            Arc::new(Mutex::new((String::new(), None)));
        let received_for_mock = Arc::clone(&received);
        let server = tiny_http::Server::http(("127.0.0.1", 0)).unwrap();
        let port = match server.server_addr() {
            tiny_http::ListenAddr::IP(addr) => addr.port(),
            other => panic!("unexpected listen addr: {other:?}"),
        };
        std::thread::spawn(move || {
            for mut request in server.incoming_requests() {
                let mut body = String::new();
                let _ = request.as_reader().read_to_string(&mut body);
                let source = request
                    .headers()
                    .iter()
                    .find(|header| {
                        header
                            .field
                            .as_str()
                            .as_str()
                            .eq_ignore_ascii_case("X-AgentPocket-Source")
                    })
                    .map(|header| header.value.as_str().to_string());
                *received_for_mock.lock().unwrap() = (body, source);
                let _ = request.respond(
                    tiny_http::Response::from_string(r#"{"bytes":15}"#)
                        .with_status_code(tiny_http::StatusCode(200)),
                );
            }
        });

        let result = mesh_push_at(home.path(), "127.0.0.1", port).unwrap();

        assert_eq!(result.bytes, "model = \"local\"\n".len());
        let (body, source) = received.lock().unwrap().clone();
        assert_eq!(body, "model = \"local\"\n");
        assert_eq!(source.as_deref(), Some(host::hostname().as_str()));
    }

    #[test]
    fn discover_merges_online_and_offline_manual_peers() {
        // 在线 peer：mock /info 应答身份 JSON（带版本）。
        let info = r#"{"app":"agentpocket","version":"2.10.0","name":"live-host"}"#;
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
                    version: Some("2.10.0".to_string()),
                    online: true,
                    manual: true,
                    kimi_version: None,
                    web_active: false,
                    web_port: None,
                },
                // 未应答的手动 peer 补 online:false 行，version 为 None。
                MeshPeerView {
                    name: "离线机".to_string(),
                    host: "100.64.0.99".to_string(),
                    version: None,
                    online: false,
                    manual: true,
                    kimi_version: None,
                    web_active: false,
                    web_port: None,
                },
            ]
        );
    }
}
