//! pull/push 命令的业务逻辑。

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use agentpocket_core::config::{ConfigStore, ImportMode};

use crate::client::{self, ClientError};
use crate::mesh::MESH_PORT;

const TIMEOUT: Duration = Duration::from_secs(5);

pub enum PullMode {
    Merge,
    Replace,
    DryRun,
}

/// CLI 入口：对端 mesh 端点固定监听 MESH_PORT。
pub fn run_pull(config_dir: &Path, host: &str, mode: PullMode) -> Result<String, String> {
    pull_via(config_dir, host, MESH_PORT, mode)
}

pub fn run_push(config_dir: &Path, host: &str) -> Result<String, String> {
    push_via(config_dir, host, MESH_PORT)
}

/// 实际实现；端口单列是为了测试能用 mesh::start(…, 0) 的随机端口起对端做端到端。
fn pull_via(config_dir: &Path, host: &str, port: u16, mode: PullMode) -> Result<String, String> {
    let response = client::get(host, port, "/config", &[], TIMEOUT)
        .map_err(|e| e.to_string())?;
    if response.status != 200 {
        return Err(format!("对方返回 HTTP {}：{}", response.status, response.body));
    }

    let store = ConfigStore::new(config_dir.to_path_buf());
    let data = store.preview_import_text(&response.body).map_err(|e| e.to_string())?;
    let preview = format!(
        "有效 {} 台 / 无效 {} 台：{}",
        data.preview.valid_servers,
        data.preview.invalid_servers,
        data.preview
            .servers
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join("、")
    );
    if matches!(mode, PullMode::DryRun) {
        return Ok(format!("[dry-run] {preview}"));
    }

    let current = store
        .load()
        .map_err(|e| e.to_string())?
        .config;
    // 与 mesh 端点 POST 同一口径：preview.servers 即 apply_import 会并入的有效集合，
    // 以此在对端合并前算出本地视角的新增/更新计数。
    let old_ids: HashSet<&str> = current.servers.iter().map(|s| s.id.as_str()).collect();
    let added = data
        .preview
        .servers
        .iter()
        .filter(|s| !old_ids.contains(s.id.as_str()))
        .count();
    let updated = data.preview.servers.len().saturating_sub(added);
    let mode_name = match mode {
        PullMode::Replace => ImportMode::Replace,
        _ => ImportMode::Merge,
    };
    let merged = store
        .apply_import(&current, data, mode_name)
        .map_err(|e| e.to_string())?;
    store.save(&merged).map_err(|e| e.to_string())?;
    Ok(match mode {
        PullMode::Replace => format!("已替换：{preview}"),
        _ => format!("已合并：新增 {added} / 更新 {updated} 台服务器"),
    })
}

fn push_via(config_dir: &Path, host: &str, port: u16) -> Result<String, String> {
    let store = ConfigStore::new(config_dir.to_path_buf());
    let current = store.load().map_err(|e| e.to_string())?.config;
    let text = store.export_text(&current).map_err(|e| e.to_string())?;

    let hostname = crate::paths::hostname();
    let response = client::post(
        host,
        port,
        "/config",
        &[("X-AgentPocket-Source", hostname.as_str())],
        &text,
        TIMEOUT,
    )
    .map_err(|e: ClientError| e.to_string())?;
    if response.status != 200 {
        return Err(format!("对方返回 HTTP {}：{}", response.status, response.body));
    }
    let counts: serde_json::Value =
        serde_json::from_str(&response.body).map_err(|e| e.to_string())?;
    Ok(format!(
        "对方已合并：新增 {} / 更新 {} 台服务器",
        counts["added"], counts["updated"]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentpocket_core::config::ConfigStore;
    use agentpocket_core::model::{AppConfig, Backend, ServerConfig};

    fn seed(dir: &std::path::Path, id: &str, name: &str) {
        let store = ConfigStore::new(dir.to_path_buf());
        store
            .save(&AppConfig {
                servers: vec![ServerConfig::new(id, name, "100.64.0.2", 3080, "t", Backend::Dsh)],
                ..Default::default()
            })
            .unwrap();
    }

    #[test]
    fn pull_merges_remote_into_local() {
        let remote_dir = tempfile::tempdir().unwrap();
        let local_dir = tempfile::tempdir().unwrap();
        seed(remote_dir.path(), "r1", "Remote");
        seed(local_dir.path(), "l1", "Local");
        let handle = crate::mesh::start(
            crate::mesh::MeshContext {
                config_dir: remote_dir.path().to_path_buf(),
                version: "t",
                hostname: "remote-host".to_string(),
            },
            0,
        )
        .unwrap();

        let message = pull_via(local_dir.path(), "127.0.0.1", handle.port, PullMode::Merge).unwrap();

        assert!(message.contains("新增 1"));
        let outcome = ConfigStore::new(local_dir.path().to_path_buf()).load().unwrap();
        assert_eq!(outcome.config.servers.len(), 2);
        handle.stop();
    }

    #[test]
    fn pull_dry_run_does_not_touch_disk() {
        let remote_dir = tempfile::tempdir().unwrap();
        let local_dir = tempfile::tempdir().unwrap();
        seed(remote_dir.path(), "r1", "Remote");
        let handle = crate::mesh::start(
            crate::mesh::MeshContext {
                config_dir: remote_dir.path().to_path_buf(),
                version: "t",
                hostname: "remote-host".to_string(),
            },
            0,
        )
        .unwrap();

        pull_via(local_dir.path(), "127.0.0.1", handle.port, PullMode::DryRun).unwrap();

        let outcome = ConfigStore::new(local_dir.path().to_path_buf()).load().unwrap();
        assert!(outcome.config.servers.is_empty());
        handle.stop();
    }

    #[test]
    fn push_reports_peer_merge_counts() {
        let remote_dir = tempfile::tempdir().unwrap();
        let local_dir = tempfile::tempdir().unwrap();
        seed(remote_dir.path(), "r1", "Remote");
        seed(local_dir.path(), "l1", "Local");
        let handle = crate::mesh::start(
            crate::mesh::MeshContext {
                config_dir: remote_dir.path().to_path_buf(),
                version: "t",
                hostname: "remote-host".to_string(),
            },
            0,
        )
        .unwrap();

        let message = push_via(local_dir.path(), "127.0.0.1", handle.port).unwrap();

        assert!(message.contains("新增 1"));
        handle.stop();
    }
}
