//! 把本机 agentpocket 登记进 Kimi Code 的 `~/.kimi-code/mcp.json`。
//!
//! 与 mcp.rs 的分工：mcp.rs 是那个被 MCP 客户端拉起的 stdio server，本模块只管写/
//! 查这份**客户端配置**，供 `agentpocket mcp install|status` 使用。
//!
//! 写之前先把现有内容备份成 `mcp.json.bak`，并且只在能完整解析现有内容时才动手——
//! 别人的 MCP 配置不能被我们覆盖掉。

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use agentpocket_core::config::{atomic_write, ConfigStore};
use agentpocket_core::kimi_config;
use agentpocket_core::model::Backend;

/// 写进 mcp.json 的默认参数：单次等待上限 4 分钟。要真正等到这么久，还得配合
/// 这里的 `toolTimeoutMs`（客户端工具调用超时）一起生效。
const TOOL_TIMEOUT_MS: u64 = 300_000;
const MAX_WAIT_SECONDS: &str = "240";

/// Kimi Code 的 MCP 配置文件位置。
pub fn mcp_json_path() -> PathBuf {
    kimi_config::home_dir().join("mcp.json")
}

/// 纯函数：把 agentpocket 条目合并进现有内容，保留其它 server 与未知字段。
/// 现有内容不是合法 JSON、或结构不对时拒绝，绝不覆盖别人的配置。
pub fn merge_entry(existing: Option<&str>, cli: &Path) -> Result<String, String> {
    let mut root: Value = match existing.map(str::trim).filter(|s| !s.is_empty()) {
        Some(text) => serde_json::from_str(text)
            .map_err(|e| format!("现有 mcp.json 不是合法 JSON，未改动：{e}"))?,
        None => json!({}),
    };
    let root_obj = root
        .as_object_mut()
        .ok_or_else(|| "现有 mcp.json 顶层不是对象，未改动".to_string())?;
    let servers = root_obj
        .entry("mcpServers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| "现有 mcp.json 的 mcpServers 不是对象，未改动".to_string())?;
    servers.insert(
        "agentpocket".to_string(),
        json!({
            "command": cli.to_string_lossy(),
            "args": ["mcp"],
            "toolTimeoutMs": TOOL_TIMEOUT_MS,
            "env": { "AGENTPOCKET_MCP_MAX_WAIT_SECONDS": MAX_WAIT_SECONDS },
        }),
    );
    let mut text = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    text.push('\n');
    Ok(text)
}

/// 写入指定路径的 mcp.json：先备份现有内容为 `.bak`，再原子替换（0600）。
/// 调用方负责传绝对路径（CLI 传的是 `current_exe()`）；拆出 `_at` 是为了能在临时
/// 目录里测试，不去碰真实的家目录。
pub fn install_at(cli: &Path, path: &Path) -> Result<String, String> {
    let existing = std::fs::read_to_string(path).ok();
    let text = merge_entry(existing.as_deref(), &cli)?;
    let backup = path.with_extension("json.bak");
    if let Some(old) = existing.as_deref() {
        atomic_write(&backup, old.as_bytes()).map_err(|e| format!("备份失败：{e}"))?;
    }
    atomic_write(path, text.as_bytes()).map_err(|e| format!("写入失败：{e}"))?;

    let mut out = format!(
        "已登记：{}\n  command = {}\n  args    = [\"mcp\"]",
        path.display(),
        cli.display()
    );
    if existing.is_some() {
        out.push_str(&format!("\n  旧内容已备份为 {}", backup.display()));
    }
    Ok(out)
}

/// 查看登记状态。`_at` 同上，便于测试。
pub fn status_at(cli: &Path, path: &Path) -> Result<String, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(_) => {
            return Ok(format!(
                "未登记：{} 不存在。\n执行 `agentpocket mcp install` 接入。",
                path.display()
            ))
        }
    };
    let root: Value =
        serde_json::from_str(&text).map_err(|e| format!("{} 不是合法 JSON：{e}", path.display()))?;
    let Some(entry) = root.pointer("/mcpServers/agentpocket") else {
        return Ok(format!(
            "未登记：{} 里没有 mcpServers.agentpocket。\n执行 `agentpocket mcp install` 接入。",
            path.display()
        ));
    };
    let cmd = entry.get("command").and_then(|v| v.as_str()).unwrap_or("");
    let described = if cmd.is_empty() {
        "缺少 command 字段，建议重新执行 `agentpocket mcp install`".to_string()
    } else if Path::new(cmd).is_file() {
        format!("{cmd}（存在）")
    } else {
        format!("{cmd}（路径不存在，重新执行 `agentpocket mcp install` 修正）")
    };
    Ok(format!(
        "已登记于 {}\n  command = {described}\n  当前运行的是 {}",
        path.display(),
        cli.display()
    ))
}

/// `agentpocket mcp install`：登记并顺带报告本机能派发多少台服务器。
pub fn install(cli: &Path) -> Result<String, String> {
    let mut out = install_at(cli, &mcp_json_path())?;
    out.push('\n');
    out.push_str(&servers_hint());
    out.push_str("\n新开一个 Kimi Code 会话才生效（工具清单在会话启动时加载）。");
    Ok(out)
}

/// `agentpocket mcp status`。
pub fn status(cli: &Path) -> Result<String, String> {
    status_at(cli, &mcp_json_path())
}

/// 「装了 MCP 但还没配服务连接」是很常见的新手状态，直接点出来。
fn servers_hint() -> String {
    match ConfigStore::new(crate::paths::default_config_dir()).load() {
        Ok(outcome) => {
            let kimi = outcome
                .config
                .servers
                .iter()
                .filter(|s| s.backend == Backend::Kimi)
                .count();
            if kimi == 0 {
                "当前没有可派发的 Kimi 服务连接：先在桌面端添加，或到节点上执行 `agentpocket kimi-web enable`。".to_string()
            } else {
                format!("当前可派发的 Kimi 服务器：{kimi} 台。")
            }
        }
        Err(e) => format!("读取本机服务连接失败：{e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli() -> PathBuf {
        PathBuf::from("/usr/bin/agentpocket")
    }

    #[test]
    fn creates_file_when_missing() {
        let text = merge_entry(None, &cli()).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["mcpServers"]["agentpocket"]["command"], "/usr/bin/agentpocket");
        assert_eq!(v["mcpServers"]["agentpocket"]["args"], json!(["mcp"]));
        assert_eq!(
            v["mcpServers"]["agentpocket"]["env"]["AGENTPOCKET_MCP_MAX_WAIT_SECONDS"],
            "240"
        );
    }

    #[test]
    fn preserves_other_servers_and_unknown_keys() {
        let existing = r#"{
          "mcpServers": { "other": { "command": "other-bin" } },
          "somethingElse": 1
        }"#;
        let text = merge_entry(Some(existing), &cli()).unwrap();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["mcpServers"]["other"]["command"], "other-bin");
        assert_eq!(v["mcpServers"]["agentpocket"]["command"], "/usr/bin/agentpocket");
        assert_eq!(v["somethingElse"], 1);
    }

    #[test]
    fn merge_is_idempotent() {
        let once = merge_entry(None, &cli()).unwrap();
        let twice = merge_entry(Some(&once), &cli()).unwrap();
        assert_eq!(once, twice);
    }

    #[test]
    fn rejects_malformed_or_wrong_shape() {
        let err = merge_entry(Some("{ not json"), &cli()).unwrap_err();
        assert!(err.contains("未改动"), "{err}");
        let err = merge_entry(Some("[1, 2]"), &cli()).unwrap_err();
        assert!(err.contains("顶层不是对象"), "{err}");
        let err = merge_entry(Some(r#"{"mcpServers": []}"#), &cli()).unwrap_err();
        assert!(err.contains("不是对象"), "{err}");
    }

    #[test]
    fn install_writes_backup_and_file_with_0600() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        std::fs::write(&path, r#"{"mcpServers":{"other":{"command":"x"}}}"#).unwrap();

        let msg = install_at(&cli(), &path).unwrap();
        assert!(msg.contains("已登记"), "{msg}");
        assert!(msg.contains("备份"), "{msg}");

        let written: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["mcpServers"]["other"]["command"], "x");
        assert_eq!(written["mcpServers"]["agentpocket"]["args"], json!(["mcp"]));

        let backup = dir.path().join("mcp.json.bak");
        assert!(backup.is_file(), "应生成 .bak");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "写入文件应为 0600");
        }
    }

    #[test]
    fn install_refuses_to_clobber_unparsable_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        std::fs::write(&path, "{ broken").unwrap();
        assert!(install_at(&cli(), &path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ broken");
    }

    #[test]
    fn status_reports_missing_then_registered() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mcp.json");
        let msg = status_at(&cli(), &path).unwrap();
        assert!(msg.contains("未登记"), "{msg}");

        install_at(&cli(), &path).unwrap();
        let msg = status_at(&cli(), &path).unwrap();
        assert!(msg.contains("已登记"), "{msg}");
        assert!(msg.contains("/usr/bin/agentpocket"), "{msg}");
    }
}
