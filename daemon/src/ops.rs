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
    let response =
        client::get(host, port, "/kimi-config", &[], TIMEOUT).map_err(|e| e.to_string())?;
    if response.status != 200 {
        return Err(format!(
            "对方返回 HTTP {}：{}",
            response.status, response.body
        ));
    }
    let local = kimi_config::read(home).ok();
    if dry_run {
        return Ok(match &local {
            Some(current) if *current == response.body => {
                format!(
                    "[dry-run] 远端 config.toml {} 字节，与本地一致",
                    response.body.len()
                )
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
        return Err(format!(
            "对方返回 HTTP {}：{}",
            response.status, response.body
        ));
    }
    Ok(format!("对方已接收 config.toml（{} 字节）", text.len()))
}

/// 远端升级走安装脚本，下载+执行放宽到 11 分钟（端点侧无额外超时）。
const UPGRADE_TIMEOUT: Duration = Duration::from_secs(660);

pub fn kimi_local_status(home: &Path) -> Result<String, String> {
    let state = crate::kimi::detect(home);
    Ok(format_kimi_state(
        state.official.as_deref(),
        state
            .npm
            .iter()
            .map(|n| (n.prefix.display().to_string(), n.version.clone())),
    ))
}

pub fn kimi_local_upgrade(home: &Path) -> Result<String, String> {
    println!("归一化到官方版（如有 npm 安装先移除，再执行官方安装脚本，可能需要数分钟）…");
    let outcome = crate::kimi::ensure_official(home)?;
    Ok(format_ensure_message(
        outcome.before.as_deref(),
        outcome.after.as_deref(),
        &outcome
            .npm_removed
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>(),
        &outcome.npm_failed,
    ))
}

pub fn kimi_remote_status(host: &str) -> Result<String, String> {
    kimi_remote_status_via(host, MESH_PORT)
}

fn kimi_remote_status_via(host: &str, port: u16) -> Result<String, String> {
    let response =
        client::get(host, port, "/kimi-info", &[], TIMEOUT).map_err(|e| e.to_string())?;
    if response.status != 200 {
        return Err(format!(
            "对方返回 HTTP {}：{}",
            response.status, response.body
        ));
    }
    let value: serde_json::Value =
        serde_json::from_str(&response.body).map_err(|e| e.to_string())?;
    let npm = value["npm"]
        .as_array()
        .map(|arr| {
            arr.iter().map(|n| {
                (
                    n["prefix"].as_str().unwrap_or("-").to_string(),
                    n["version"].as_str().map(String::from),
                )
            })
        })
        .into_iter()
        .flatten();
    Ok(format!(
        "对方{}",
        format_kimi_state(value["version"].as_str(), npm)
    ))
}

pub fn kimi_remote_upgrade(host: &str) -> Result<String, String> {
    kimi_remote_upgrade_via(host, MESH_PORT)
}

fn kimi_remote_upgrade_via(host: &str, port: u16) -> Result<String, String> {
    println!("已请求对方归一化到官方版（移除 npm 安装 + 官方安装脚本，可能需要数分钟）…");
    let response = client::post(host, port, "/kimi-upgrade", &[], "", UPGRADE_TIMEOUT)
        .map_err(|e: ClientError| e.to_string())?;
    if response.status != 200 {
        return Err(format!(
            "对方返回 HTTP {}：{}",
            response.status, response.body
        ));
    }
    let value: serde_json::Value =
        serde_json::from_str(&response.body).map_err(|e| e.to_string())?;
    let removed: Vec<String> = value["npmRemoved"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|p| p.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let failed: Vec<String> = value["npmFailed"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|p| p.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    Ok(format!(
        "对方{}",
        format_ensure_message(
            value["before"].as_str(),
            value["after"].as_str(),
            &removed,
            &failed
        )
    ))
}

pub fn kimi_web_enable(config_dir: &Path, home: &Path, port: u16) -> Result<String, String> {
    crate::kimi_web::enable(home, port)?;
    let registered = crate::kimi_web::register_server_entry(config_dir, port)?;
    Ok(format!(
        "kimi web 服务已生成并启动（端口 {port}）；{registered}"
    ))
}

pub fn kimi_web_restart(home: &Path, force: bool) -> Result<String, String> {
    crate::kimi_web::restart_guarded(home, force)?;
    Ok("kimi web 服务已重启".to_string())
}

pub fn kimi_web_disable(home: &Path) -> Result<String, String> {
    crate::kimi_web::disable(home)?;
    Ok("kimi web 服务已停止并移除".to_string())
}

pub fn kimi_web_status(home: &Path) -> Result<String, String> {
    let status = crate::kimi_web::status(home);
    Ok(if status.installed {
        format!(
            "kimi web 服务：{}（端口 {}）",
            if status.active {
                "运行中"
            } else {
                "未运行"
            },
            status.port
        )
    } else {
        "kimi web 服务未生成（agentpocket kimi-web enable 生成）".to_string()
    })
}

pub fn kimi_remote_web_restart(host: &str, force: bool) -> Result<String, String> {
    kimi_remote_web_restart_via(host, MESH_PORT, force)
}

fn kimi_remote_web_restart_via(host: &str, port: u16, force: bool) -> Result<String, String> {
    let body = serde_json::json!({ "force": force }).to_string();
    let response = client::post(
        host,
        port,
        "/kimi-web-restart",
        &[],
        &body,
        Duration::from_secs(120),
    )
    .map_err(|e: ClientError| e.to_string())?;
    if response.status != 200 {
        // 端点拒绝时 body 携带原因（如活跃会话保护），原样透传
        return Err(response.body);
    }
    Ok("对方 kimi web 服务已重启".to_string())
}

/// 状态视图：官方版版本 + npm 安装清单（两者可能并存）。
fn format_kimi_state(
    official: Option<&str>,
    npm: impl Iterator<Item = (String, Option<String>)>,
) -> String {
    let mut lines = match official {
        Some(v) => vec![format!("Kimi Code CLI 官方版：{v}")],
        None => vec!["Kimi Code CLI 官方版未安装".to_string()],
    };
    let mut npm_count = 0;
    for (prefix, version) in npm {
        npm_count += 1;
        lines.push(format!(
            "检测到 npm 安装：{prefix}（{}）",
            version.as_deref().unwrap_or("未知版本")
        ));
    }
    if npm_count > 0 {
        lines.push("存在 npm 安装，--upgrade 会先移除再装官方版".to_string());
    } else if official.is_none() {
        lines.push("agentpocket kimi --upgrade 安装".to_string());
    }
    lines.join("\n")
}

fn format_ensure_message(
    before: Option<&str>,
    after: Option<&str>,
    npm_removed: &[String],
    npm_failed: &[String],
) -> String {
    let mut parts = Vec::new();
    if !npm_removed.is_empty() {
        parts.push(format!("已移除 npm 安装：{}", npm_removed.join("、")));
    }
    for failure in npm_failed {
        parts.push(format!("npm 安装移除失败：{failure}"));
    }
    parts.push(match (before, after) {
        (Some(b), Some(a)) if b == a => format!("已是最新：{a}"),
        (Some(b), Some(a)) => format!("升级完成：{b} -> {a}"),
        (None, Some(a)) => format!("安装完成：{a}"),
        (_, None) => "安装脚本已执行，但未能确认版本，请手动检查".to_string(),
    });
    parts.join("；")
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
    fn ensure_message_variants() {
        assert_eq!(
            format_ensure_message(Some("0.37.0"), Some("0.38.0"), &[], &[]),
            "升级完成：0.37.0 -> 0.38.0"
        );
        assert_eq!(
            format_ensure_message(
                Some("0.38.0"),
                Some("0.38.0"),
                &["/home/u/.nvm/versions/node/v22.22.0".to_string()],
                &[]
            ),
            "已移除 npm 安装：/home/u/.nvm/versions/node/v22.22.0；已是最新：0.38.0"
        );
        assert_eq!(
            format_ensure_message(None, Some("0.38.0"), &[], &[]),
            "安装完成：0.38.0"
        );
    }

    #[test]
    fn kimi_state_lists_npm_conflict() {
        let message = format_kimi_state(
            Some("0.38.0"),
            [("p1".to_string(), Some("0.37.2".to_string()))].into_iter(),
        );
        assert!(message.contains("官方版：0.38.0"));
        assert!(message.contains("检测到 npm 安装：p1（0.37.2）"));
        assert!(message.contains("--upgrade"));
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
