//! peer 发现：tailscale status --json 解析 + 端口探测 + 手动 peer 合并。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;

use crate::client;
use crate::mesh::MESH_PORT;

#[derive(Clone, Debug, PartialEq)]
pub struct MeshPeer {
    pub name: String,
    pub host: String,
    pub version: Option<String>,
    pub manual: bool,
}

/// 只声明 Peer 字段；serde 默认忽略未知字段，顶层 "Self" 不映射即被排除。
#[derive(Deserialize)]
struct StatusJson {
    #[serde(rename = "Peer")]
    peers: Option<std::collections::HashMap<String, PeerEntry>>,
}

#[derive(Deserialize)]
struct PeerEntry {
    #[serde(rename = "HostName")]
    host_name: String,
    #[serde(rename = "TailscaleIPs")]
    ips: Option<Vec<String>>,
    #[serde(rename = "Online")]
    online: Option<bool>,
}

/// CLI 查找顺序：PATH → 常见安装路径。找不到返回 None（发现退化为仅手动 peer）。
pub fn find_tailscale_binary() -> Option<PathBuf> {
    if Command::new("tailscale")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        return Some(PathBuf::from("tailscale"));
    }
    ["/usr/bin/tailscale", "/usr/local/bin/tailscale"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
}

/// 解析 tailscale status --json：保留在线且带 IPv4 的 peer，返回 (主机名, IPv4)。
pub fn parse_online_peers(json: &str) -> Vec<(String, String)> {
    let parsed: StatusJson = serde_json::from_str(json).unwrap_or(StatusJson { peers: None });
    let Some(peers) = parsed.peers else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for entry in peers.into_values() {
        if !entry.online.unwrap_or(false) {
            continue;
        }
        let Some(ips) = entry.ips else { continue };
        let Some(ipv4) = ips.iter().find(|ip| ip.parse::<std::net::Ipv4Addr>().is_ok()) else {
            continue;
        };
        result.push((entry.host_name, ipv4.clone()));
    }
    result.sort();
    result
}

/// 探测单个 host 的 mesh 端点；/info 应答 app==agentpocket 才算命中。
pub fn probe_peer(host: &str, port: u16, fallback_name: &str, timeout: Duration) -> Option<MeshPeer> {
    let response = client::get(host, port, "/info", &[], timeout).ok()?;
    if response.status != 200 {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(&response.body).ok()?;
    if value.get("app").and_then(|a| a.as_str()) != Some("agentpocket") {
        return None;
    }
    Some(MeshPeer {
        name: value
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or(fallback_name)
            .to_string(),
        host: host.to_string(),
        version: value
            .get("version")
            .and_then(|v| v.as_str())
            .map(String::from),
        manual: false,
    })
}

/// 完整发现：tailscale 在线设备 + 手动 peer，并发探测后按 host 去重。
pub fn discover(config_dir: &Path, tailscale: Option<&Path>, timeout: Duration) -> Vec<MeshPeer> {
    let mut candidates: Vec<(String, String)> = Vec::new(); // (host, 备用名)
    if let Some(binary) = tailscale {
        if let Ok(output) = Command::new(binary).args(["status", "--json"]).output() {
            candidates.extend(
                parse_online_peers(&String::from_utf8_lossy(&output.stdout))
                    .into_iter()
                    .map(|(name, ip)| (ip, name)),
            );
        }
    }
    let manual = load_manual_peers(config_dir);
    candidates.extend(manual.iter().map(|p| (p.host.clone(), p.name.clone())));

    let mut seen: HashSet<String> = HashSet::new();
    let mut peers: Vec<MeshPeer> = Vec::new();
    std::thread::scope(|scope| {
        let handles: Vec<_> = candidates
            .iter()
            .map(|(host, name)| {
                scope.spawn(move || probe_peer(host, MESH_PORT, name, timeout))
            })
            .collect();
        for handle in handles {
            let Some(peer) = handle.join().ok().flatten() else { continue };
            if seen.insert(peer.host.clone()) {
                peers.push(peer);
            }
        }
    });
    peers.sort_by(|a, b| a.name.cmp(&b.name));
    peers
}

#[derive(serde::Deserialize, serde::Serialize)]
struct PeerFile {
    peers: Vec<PeerEntryFile>,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct PeerEntryFile {
    name: String,
    host: String,
}

fn peer_file_path(config_dir: &Path) -> PathBuf {
    config_dir.join("peers.json")
}

pub fn load_manual_peers(config_dir: &Path) -> Vec<MeshPeer> {
    std::fs::read_to_string(peer_file_path(config_dir))
        .ok()
        .and_then(|text| serde_json::from_str::<PeerFile>(&text).ok())
        .map(|file| {
            file.peers
                .into_iter()
                .map(|p| MeshPeer {
                    name: p.name,
                    host: p.host,
                    version: None,
                    manual: true,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 预留写入接口：暂无 add-peer 子命令，peers.json 目前手写维护。
#[allow(dead_code)]
pub fn save_manual_peers(config_dir: &Path, peers: &[MeshPeer]) -> std::io::Result<()> {
    std::fs::create_dir_all(config_dir)?;
    let file = PeerFile {
        peers: peers
            .iter()
            .map(|p| PeerEntryFile {
                name: p.name.clone(),
                host: p.host.clone(),
            })
            .collect(),
    };
    std::fs::write(
        peer_file_path(config_dir),
        serde_json::to_string_pretty(&file).unwrap(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const FIXTURE: &str = r#"{
      "Self": {"HostName": "self-host", "TailscaleIPs": ["100.64.0.1"], "Online": true},
      "Peer": {
        "k1": {"HostName": "peer-a", "TailscaleIPs": ["100.64.0.2", "fd7a:115c:a1e0::1"], "Online": true},
        "k2": {"HostName": "peer-b", "TailscaleIPs": ["100.64.0.3"], "Online": false},
        "k3": {"HostName": "peer-c", "Online": true}
      }
    }"#;

    #[test]
    fn parse_keeps_online_peers_with_ipv4_only() {
        let peers = parse_online_peers(FIXTURE);
        assert_eq!(peers, vec![("peer-a".to_string(), "100.64.0.2".to_string())]);
    }

    #[test]
    fn manual_peers_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let peers = vec![MeshPeer {
            name: "桌面机".to_string(),
            host: "100.64.0.9".to_string(),
            version: None,
            manual: true,
        }];
        save_manual_peers(dir.path(), &peers).unwrap();
        let loaded = load_manual_peers(dir.path());
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].host, "100.64.0.9");
        assert_eq!(loaded[0].name, "桌面机");
    }

    #[test]
    fn manual_peers_file_missing_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_manual_peers(dir.path()).is_empty());
    }

    #[test]
    fn discover_finds_live_mesh_endpoint_via_manual_peer() {
        let dir = tempfile::tempdir().unwrap();
        let handle = crate::mesh::start(
            crate::mesh::MeshContext {
                config_dir: dir.path().to_path_buf(),
                version: "9.9.9",
                hostname: "live-host".to_string(),
            },
            0,
        )
        .unwrap();
        // 手动 peer 指向本机端点（端口由环境注入，测试里直接探测）。
        let peer = probe_peer("127.0.0.1", handle.port, "live-host", Duration::from_secs(3));
        let peer = peer.expect("probe succeeds");
        assert_eq!(peer.name, "live-host");
        assert_eq!(peer.version.as_deref(), Some("9.9.9"));
        handle.stop();
    }

    #[test]
    fn probe_dead_port_returns_none() {
        // 绑一个端口再关掉，确保连接被拒绝。
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        assert!(probe_peer("127.0.0.1", port, "dead", Duration::from_secs(1)).is_none());
    }

    #[test]
    fn probe_foreign_http_service_returns_none() {
        // 48720 上跑了个"别的" HTTP 服务（应答不是 agentpocket /info）。
        let server = tiny_http::Server::http(("127.0.0.1", 0)).unwrap();
        let port = match server.server_addr() {
            tiny_http::ListenAddr::IP(addr) => addr.port(),
            other => panic!("unexpected listen addr: {other:?}"),
        };
        std::thread::spawn(move || {
            for request in server.incoming_requests() {
                let _ = request.respond(tiny_http::Response::from_string(
                    r#"{"app":"something-else"}"#,
                ));
            }
        });
        assert!(probe_peer("127.0.0.1", port, "foreign", Duration::from_secs(3)).is_none());
    }
}
