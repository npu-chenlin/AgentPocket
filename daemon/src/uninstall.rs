use std::process::Command;

use crate::paths;

const BIN_PATH: &str = "/usr/local/bin/agentpocket";
const SERVICE_PATH: &str = "/etc/systemd/system/agentpocket.service";

/// 停止并移除 systemd 服务与二进制；配置目录保留。
pub fn run() {
    if !is_root() {
        eprintln!("请用 sudo 运行：sudo agentpocket uninstall");
        std::process::exit(1);
    }
    let _ = Command::new("systemctl").args(["stop", "agentpocket"]).status();
    let _ = Command::new("systemctl").args(["disable", "agentpocket"]).status();
    let _ = std::fs::remove_file(SERVICE_PATH);
    let _ = Command::new("systemctl").args(["daemon-reload"]).status();
    // Linux 允许删除正在运行的二进制，进程退出后文件才真正释放
    let _ = std::fs::remove_file(BIN_PATH);
    println!(
        "已卸载 agentpocket（配置目录 {} 保留，可手动删除）",
        paths::default_config_dir().display()
    );
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
