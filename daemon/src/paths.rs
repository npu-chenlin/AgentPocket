//! 配置目录解析（与 GUI 共享 XDG 路径）。

use std::path::PathBuf;

/// ~/.local/share/com.local.agentpocket.desktop（遵守 XDG_DATA_HOME）。
/// GUI 桌面端（Tauri identifier）与 daemon 在同机同用户下读写同一份 config.json，
/// 故目录名与 GUI 实际数据目录保持一致。
pub fn default_config_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME").unwrap_or_default();
            PathBuf::from(home).join(".local/share")
        });
    base.join("com.local.agentpocket.desktop")
}
