use std::path::PathBuf;
use std::process::Command;

use crate::paths;

const BIN_PATH: &str = "/usr/local/bin/agentpocket";
const SERVICE_PATH: &str = "/etc/systemd/system/agentpocket.service";
const COMPLETION_PATH: &str = "/usr/share/bash-completion/completions/agentpocket";

/// 停止并移除 systemd 服务与二进制；配置目录保留。
pub fn run() {
    if !is_root() {
        eprintln!("请用 sudo 运行：sudo agentpocket uninstall");
        std::process::exit(1);
    }
    // 先于删除 unit 读取服务用户的 HOME，否则 sudo 下会误报 /root
    let config_dir = service_config_dir();
    let _ = Command::new("systemctl").args(["stop", "agentpocket"]).status();
    let _ = Command::new("systemctl").args(["disable", "agentpocket"]).status();
    let _ = std::fs::remove_file(SERVICE_PATH);
    let _ = Command::new("systemctl").args(["daemon-reload"]).status();
    // Linux 允许删除正在运行的二进制，进程退出后文件才真正释放
    let _ = std::fs::remove_file(BIN_PATH);
    let _ = std::fs::remove_file(COMPLETION_PATH);
    println!(
        "已卸载 agentpocket（配置目录 {} 保留，可手动删除）",
        config_dir.display()
    );
}

/// 从 unit 的 Environment=HOME= 还原服务运行用户的数据目录；读不到时退回当前进程默认值。
fn service_config_dir() -> PathBuf {
    let home = std::fs::read_to_string(SERVICE_PATH).ok().and_then(|content| {
        content.lines().find_map(|line| {
            line.strip_prefix("Environment=HOME=")
                .map(|h| h.trim().to_string())
        })
    });
    match home {
        Some(home) => PathBuf::from(home).join(".local/share/com.local.agentpocket.desktop"),
        None => paths::default_config_dir(),
    }
}

fn is_root() -> bool {
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim() == "0")
        .unwrap_or(false)
}
