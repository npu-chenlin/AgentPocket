//! kimi web 服务管理：生成并管理用户级 systemd 单元 kimi-web.service。
//! 单元模板与既有服务器实践一致：监听 0.0.0.0、tailnet 内免 token、崩溃自恢复。

use std::path::{Path, PathBuf};
use std::process::Command;

pub const UNIT_NAME: &str = "kimi-web.service";
pub const DEFAULT_PORT: u16 = 58627;

pub fn unit_path(home: &Path) -> PathBuf {
    home.join(".config/systemd/user").join(UNIT_NAME)
}

/// 单元模板：home/port 参数化，flags 与手工部署的既有单元保持一致。
pub fn render_unit(home: &Path, port: u16) -> String {
    let kimi = home.join(".kimi-code/bin/kimi");
    format!(
        "[Unit]\n\
         Description=Kimi web server (AgentPocket managed)\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={kimi} web --port {port} --host 0.0.0.0 --allow-remote-terminals --dangerous-bypass-auth --no-open\n\
         WorkingDirectory={home}\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        kimi = kimi.display(),
        home = home.display(),
    )
}

fn uid() -> Option<String> {
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

/// systemctl --user：系统服务进程里没有用户总线环境，缺 XDG_RUNTIME_DIR 时按 uid 补齐。
fn systemctl_user(home: &Path, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("systemctl");
    cmd.arg("--user").args(args).env("HOME", home);
    if std::env::var_os("XDG_RUNTIME_DIR").is_none() {
        if let Some(uid) = uid() {
            cmd.env("XDG_RUNTIME_DIR", format!("/run/user/{uid}"));
        }
    }
    let output = cmd.output().map_err(|e| format!("执行 systemctl 失败：{e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.status.success() {
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(format!("systemctl {} 失败：{}", args.join(" "), stderr))
    }
}

#[derive(Debug, Default)]
pub struct KimiWebStatus {
    pub installed: bool,
    pub active: bool,
    pub port: u16,
}

pub fn status(home: &Path) -> KimiWebStatus {
    let installed = unit_path(home).is_file();
    let active = installed
        && systemctl_user(home, &["is-active", UNIT_NAME])
            .map(|s| s.trim() == "active")
            .unwrap_or(false);
    let port = std::fs::read_to_string(unit_path(home))
        .ok()
        .and_then(|content| parse_port(&content))
        .unwrap_or(DEFAULT_PORT);
    KimiWebStatus { installed, active, port }
}

/// 从单元 ExecStart 解析 --port；缺省按默认端口。
pub fn parse_port(unit: &str) -> Option<u16> {
    let line = unit.lines().find(|l| l.contains("ExecStart="))?;
    let mut parts = line.split_whitespace();
    while let Some(part) = parts.next() {
        if part == "--port" {
            return parts.next().and_then(|p| p.parse().ok());
        }
    }
    None
}

pub fn enable(home: &Path, port: u16) -> Result<(), String> {
    let kimi = home.join(".kimi-code/bin/kimi");
    if !kimi.is_file() {
        return Err("未安装 Kimi Code CLI（先 agentpocket kimi --upgrade）".to_string());
    }
    let path = unit_path(home);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, render_unit(home, port)).map_err(|e| e.to_string())?;
    systemctl_user(home, &["daemon-reload"])?;
    systemctl_user(home, &["enable", "--now", UNIT_NAME]).map(|_| ())
}

pub fn restart(home: &Path) -> Result<(), String> {
    if !unit_path(home).is_file() {
        return Err("kimi web 服务未生成（先 agentpocket kimi-web enable）".to_string());
    }
    systemctl_user(home, &["restart", UNIT_NAME]).map(|_| ())
}

/// 本机 kimi web 当前活跃会话数（服务异常或未安装时为 0）。
pub fn busy_sessions(home: &Path) -> usize {
    let status = status(home);
    if !status.installed {
        return 0;
    }
    let server = agentpocket_core::model::ServerConfig::new(
        "kimi-web",
        "kimi-web",
        "127.0.0.1",
        status.port,
        "",
        agentpocket_core::model::Backend::Kimi,
    );
    let probe = crate::status::probe_server(&server, std::time::Duration::from_secs(5));
    if probe.online { probe.busy } else { 0 }
}

/// 带会话保护的重启：有活跃会话时非 force 拒绝，避免打断正在运行的任务。
pub fn restart_guarded(home: &Path, force: bool) -> Result<(), String> {
    if !force {
        let busy = busy_sessions(home);
        if busy > 0 {
            return Err(format!(
                "当前有 {busy} 个会话运行中，重启会中断它们；确认后用 --force（或 GUI 二次确认）强制重启"
            ));
        }
    }
    restart(home)
}

pub fn disable(home: &Path) -> Result<(), String> {
    if !unit_path(home).is_file() {
        return Err("kimi web 服务未生成".to_string());
    }
    let _ = systemctl_user(home, &["disable", "--now", UNIT_NAME]);
    std::fs::remove_file(unit_path(home)).map_err(|e| e.to_string())?;
    systemctl_user(home, &["daemon-reload"]).map(|_| ())
}

/// 把 kimi web 注册进 AgentPocket 服务器列表（host 取 tailscale IP，无则回环；
/// bypass-auth 模式无 token）。返回描述文案。
pub fn register_server_entry(config_dir: &Path, port: u16) -> Result<String, String> {
    let host = tailscale_self_ip().unwrap_or_else(|| "127.0.0.1".to_string());
    let name = agentpocket_core::host::hostname();
    let store = agentpocket_core::config::ConfigStore::new(config_dir.to_path_buf());
    let mut updated = false;
    store
        .update(|current| {
            if let Some(server) = current
                .servers
                .iter_mut()
                .find(|s| s.host == host && s.port == port)
            {
                server.name = name.clone();
                server.backend = agentpocket_core::model::Backend::Kimi;
                updated = true;
                return;
            }
            let id = uuid::Uuid::new_v4().to_string();
            current.servers.push(agentpocket_core::model::ServerConfig::new(
                &id,
                &name,
                &host,
                port,
                "",
                agentpocket_core::model::Backend::Kimi,
            ));
        })
        .map_err(|e| e.to_string())?;
    if updated {
        return Ok(format!("已更新 Agent 服务连接 {name}（{host}:{port}）"));
    }
    Ok(format!("已注册 Agent 服务连接 {name}（{host}:{port}）"))
}

fn tailscale_self_ip() -> Option<String> {
    let bin = agentpocket_core::discovery::find_tailscale_binary()?;
    let output = Command::new(bin).args(["ip", "-4"]).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_unit_matches_deployed_convention() {
        let unit = render_unit(Path::new("/home/u"), 59123);
        assert!(unit.contains(
            "ExecStart=/home/u/.kimi-code/bin/kimi web --port 59123 --host 0.0.0.0 \
             --allow-remote-terminals --dangerous-bypass-auth --no-open"
        ));
        assert!(unit.contains("WorkingDirectory=/home/u"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("WantedBy=default.target"));
    }

    #[test]
    fn parse_port_reads_exec_start() {
        let unit = render_unit(Path::new("/home/u"), 59123);
        assert_eq!(parse_port(&unit), Some(59123));
        assert_eq!(parse_port("[Service]\nExecStart=kimi web\n"), None);
    }

    #[test]
    fn status_missing_reports_not_installed() {
        let home = tempfile::tempdir().unwrap();
        let status = status(home.path());
        assert!(!status.installed);
        assert!(!status.active);
        assert_eq!(status.port, DEFAULT_PORT);
    }

    #[test]
    fn enable_without_kimi_binary_fails() {
        let home = tempfile::tempdir().unwrap();
        let error = enable(home.path(), DEFAULT_PORT).unwrap_err();
        assert!(error.contains("未安装"));
    }
}
