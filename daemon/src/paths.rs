//! 配置目录与主机名解析（与 GUI 共享 XDG 路径）。

use std::path::PathBuf;

/// ~/.local/share/AgentPocket（遵守 XDG_DATA_HOME）。
/// GUI 桌面端与 daemon 在同机同用户下读写同一份 config.json。
pub fn default_config_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME").unwrap_or_default();
            PathBuf::from(home).join(".local/share")
        });
    base.join("AgentPocket")
}

pub fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "agentpocket".to_string())
}
