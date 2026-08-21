//! pull/push 命令的业务逻辑：同步 ~/.kimi-code/config.toml。

use std::path::Path;
use std::time::Duration;

use agentpocket_core::kimi_config;
use agentpocket_core::mesh_client::{self as client, ClientError};

use crate::mesh::MESH_PORT;

const TIMEOUT: Duration = Duration::from_secs(5);

/// CLI 入口：拉取对端 config.toml 覆盖本地（dry_run 只预览）。
pub fn run_pull(home: &Path, host: &str, dry_run: bool) -> Result<String, String> {
    pull_via(home, host, MESH_PORT, dry_run)
}

/// CLI 入口：把本地 config.toml 推送给对端。
pub fn run_push(home: &Path, host: &str) -> Result<String, String> {
    push_via(home, host, MESH_PORT)
}

/// 实际实现；端口单列是为了测试能用 mesh::start(…, 0) 的随机端口起对端做端到端。
fn pull_via(home: &Path, host: &str, port: u16, dry_run: bool) -> Result<String, String> {
    let response = client::get(host, port, "/kimi-config", &[], TIMEOUT)
        .map_err(|e| e.to_string())?;
    if response.status != 200 {
        return Err(format!("对方返回 HTTP {}：{}", response.status, response.body));
    }
    let local = kimi_config::read(home).ok();
    if dry_run {
        return Ok(match &local {
            Some(current) if *current == response.body => {
                format!("[dry-run] 远端 config.toml {} 字节，与本地一致", response.body.len())
            }
            Some(current) => format!(
                "[dry-run] 远端 config.toml {} 字节，本地 {} 字节，将被覆盖",
                response.body.len(),
                current.len()
            ),
            None => format!(
                "[dry-run] 远端 config.toml {} 字节，本地不存在，将新建",
                response.body.len()
            ),
        });
    }
    kimi_config::write(home, &response.body)?;
    Ok(match local {
        Some(_) => format!(
            "已覆盖：远端 config.toml（{} 字节），旧文件已备份为 config.toml.bak",
            response.body.len()
        ),
        None => format!("已新建：远端 config.toml（{} 字节）", response.body.len()),
    })
}

fn push_via(home: &Path, host: &str, port: u16) -> Result<String, String> {
    let text = kimi_config::read(home)?;
    let hostname = agentpocket_core::host::hostname();
    let response = client::post(
        host,
        port,
        "/kimi-config",
        &[("X-AgentPocket-Source", hostname.as_str())],
        &text,
        TIMEOUT,
    )
    .map_err(|e: ClientError| e.to_string())?;
    if response.status != 200 {
        return Err(format!("对方返回 HTTP {}：{}", response.status, response.body));
    }
    Ok(format!("对方已接收 config.toml（{} 字节）", text.len()))
}

/// 远端升级走安装脚本，下载+执行放宽到 11 分钟（端点侧无额外超时）。
const UPGRADE_TIMEOUT: Duration = Duration::from_secs(660);

pub fn kimi_local_status(home: &Path) -> Result<String, String> {
    let (installed, version) = crate::kimi::info(home);
    Ok(if installed {
        format!("Kimi Code CLI 已安装：{}", version.as_deref().unwrap_or("未知版本"))
    } else {
        "Kimi Code CLI 未安装（agentpocket kimi --upgrade 安装）".to_string()
    })
}

pub fn kimi_local_upgrade(home: &Path) -> Result<String, String> {
    println!("执行官方安装脚本（下载 + 校验，可能需要数分钟）…");
    let outcome = crate::kimi::install_or_upgrade(home)?;
    Ok(format_upgrade_message(outcome.before.as_deref(), outcome.after.as_deref()))
}

pub fn kimi_remote_status(host: &str) -> Result<String, String> {
    kimi_remote_status_via(host, MESH_PORT)
}

fn kimi_remote_status_via(host: &str, port: u16) -> Result<String, String> {
    let response = client::get(host, port, "/kimi-info", &[], TIMEOUT)
        .map_err(|e| e.to_string())?;
    if response.status != 200 {
        return Err(format!("对方返回 HTTP {}：{}", response.status, response.body));
    }
    let value: serde_json::Value =
        serde_json::from_str(&response.body).map_err(|e| e.to_string())?;
    Ok(if value["installed"].as_bool().unwrap_or(false) {
        format!(
            "对方 Kimi Code CLI：{}",
            value["version"].as_str().unwrap_or("未知版本")
        )
    } else {
        format!("对方未安装 Kimi Code CLI（agentpocket kimi {host} --upgrade 安装）")
    })
}

pub fn kimi_remote_upgrade(host: &str) -> Result<String, String> {
    kimi_remote_upgrade_via(host, MESH_PORT)
}

fn kimi_remote_upgrade_via(host: &str, port: u16) -> Result<String, String> {
    println!("已请求对方执行安装脚本（下载 + 校验，可能需要数分钟）…");
    let response = client::post(host, port, "/kimi-upgrade", &[], "", UPGRADE_TIMEOUT)
        .map_err(|e: ClientError| e.to_string())?;
    if response.status != 200 {
        return Err(format!("对方返回 HTTP {}：{}", response.status, response.body));
    }
    let value: serde_json::Value =
        serde_json::from_str(&response.body).map_err(|e| e.to_string())?;
    let before = value["before"].as_str();
    let after = value["after"].as_str();
    Ok(format!(
        "对方{}",
        format_upgrade_message(before, after)
    ))
}

fn format_upgrade_message(before: Option<&str>, after: Option<&str>) -> String {
    match (before, after) {
        (Some(b), Some(a)) if b == a => format!("已是最新：{a}"),
        (Some(b), Some(a)) => format!("升级完成：{b} -> {a}"),
        (None, Some(a)) => format!("安装完成：{a}"),
        (_, None) => "安装脚本已执行，但未能确认版本，请手动检查".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed_kimi_config(dir: &std::path::Path, content: &str) {
        std::fs::create_dir_all(dir.join(".kimi-code")).unwrap();
        std::fs::write(dir.join(".kimi-code/config.toml"), content).unwrap();
    }

    fn start_remote(dir: &std::path::Path) -> crate::mesh::MeshHandle {
        crate::mesh::start(
            crate::mesh::MeshContext {
                config_dir: dir.to_path_buf(),
                kimi_home: dir.to_path_buf(),
                version: "t",
                hostname: "remote-host".to_string(),
            },
            0,
        )
        .unwrap()
    }

    #[test]
    fn pull_replaces_local_and_backs_up() {
        let remote_dir = tempfile::tempdir().unwrap();
        let local_dir = tempfile::tempdir().unwrap();
        seed_kimi_config(remote_dir.path(), "model = \"remote\"\n");
        seed_kimi_config(local_dir.path(), "model = \"local\"\n");
        let handle = start_remote(remote_dir.path());

        let message = pull_via(local_dir.path(), "127.0.0.1", handle.port, false).unwrap();

        assert!(message.contains("已覆盖"));
        assert_eq!(
            std::fs::read_to_string(local_dir.path().join(".kimi-code/config.toml")).unwrap(),
            "model = \"remote\"\n"
        );
        assert_eq!(
            std::fs::read_to_string(local_dir.path().join(".kimi-code/config.toml.bak")).unwrap(),
            "model = \"local\"\n"
        );
        handle.stop();
    }

    #[test]
    fn pull_dry_run_does_not_touch_disk() {
        let remote_dir = tempfile::tempdir().unwrap();
        let local_dir = tempfile::tempdir().unwrap();
        seed_kimi_config(remote_dir.path(), "model = \"remote\"\n");
        seed_kimi_config(local_dir.path(), "model = \"local\"\n");
        let handle = start_remote(remote_dir.path());

        let message = pull_via(local_dir.path(), "127.0.0.1", handle.port, true).unwrap();

        assert!(message.contains("[dry-run]"));
        assert_eq!(
            std::fs::read_to_string(local_dir.path().join(".kimi-code/config.toml")).unwrap(),
            "model = \"local\"\n"
        );
        handle.stop();
    }

    #[test]
    fn kimi_remote_status_reads_peer_info() {
        use std::os::unix::fs::PermissionsExt;
        let remote_dir = tempfile::tempdir().unwrap();
        let bin_dir = remote_dir.path().join(".kimi-code/bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let bin = bin_dir.join("kimi");
        std::fs::write(&bin, "#!/bin/sh\necho 0.99.0\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let handle = start_remote(remote_dir.path());

        let message = kimi_remote_status_via("127.0.0.1", handle.port).unwrap();

        assert!(message.contains("0.99.0"));
        handle.stop();
    }

    #[test]
    fn upgrade_message_variants() {
        assert_eq!(
            format_upgrade_message(Some("0.37.0"), Some("0.38.0")),
            "升级完成：0.37.0 -> 0.38.0"
        );
        assert_eq!(
            format_upgrade_message(Some("0.38.0"), Some("0.38.0")),
            "已是最新：0.38.0"
        );
        assert_eq!(format_upgrade_message(None, Some("0.38.0")), "安装完成：0.38.0");
    }

    #[test]
    fn push_sends_local_file() {
        let remote_dir = tempfile::tempdir().unwrap();
        let local_dir = tempfile::tempdir().unwrap();
        seed_kimi_config(local_dir.path(), "model = \"local\"\n");
        let handle = start_remote(remote_dir.path());

        let message = push_via(local_dir.path(), "127.0.0.1", handle.port).unwrap();

        assert!(message.contains("已接收"));
        assert_eq!(
            std::fs::read_to_string(remote_dir.path().join(".kimi-code/config.toml")).unwrap(),
            "model = \"local\"\n"
        );
        handle.stop();
    }
}
