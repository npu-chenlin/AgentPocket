//! 主机名解析（daemon 与 GUI 共享）。

/// 读取本机主机名：/etc/hostname → $HOSTNAME → 兜底 "agentpocket"。
pub fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "agentpocket".to_string())
}
