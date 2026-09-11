//! 裸执行 `agentpocket`（不带任何子命令）时拉起桌面端。
//!
//! deb 把 CLI 与 GUI 装在同一个包里，所以桌面上这个路径是已知的；节点上没有 GUI，
//! 这时退回打印帮助——不该因为缺一个 GUI 就报错，节点侧的用法完全不受影响。

use std::path::{Path, PathBuf};

use clap::CommandFactory;

use crate::Cli;

/// GUI 可执行文件名。deb 里装的是前者；后者是给将来改名留的兼容名。
const GUI_NAMES: &[&str] = &["agentpocket-desktop", "agentpocket-gui"];

/// 先找与自身同目录的（deb 装出来的布局就是这样），再按 PATH 找。
fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            out.extend(GUI_NAMES.iter().map(|name| dir.join(name)));
        }
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&paths) {
            out.extend(GUI_NAMES.iter().map(|name| dir.join(name)));
        }
    }
    out
}

/// 取第一个真实存在的候选（纯函数，便于测试）。
fn pick(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|p| p.is_file()).cloned()
}

/// 拉起桌面端；没装就打印帮助。
pub fn launch_or_help() -> Result<(), String> {
    let Some(gui) = pick(&candidates()) else {
        Cli::command().print_help().map_err(|e| e.to_string())?;
        println!();
        return Ok(());
    };
    spawn_detached(&gui)?;
    println!("已启动桌面端：{}", gui.display());
    Ok(())
}

/// 用 setsid 脱离当前进程组，免得在终端里 Ctrl-C 时把托盘应用一起带走；
/// 没有 setsid 就退化成普通 spawn。
fn spawn_detached(gui: &Path) -> Result<(), String> {
    let via_setsid = std::process::Command::new("setsid")
        .arg(gui)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if via_setsid.is_ok() {
        return Ok(());
    }
    std::process::Command::new(gui)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("启动桌面端失败：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_returns_first_existing_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("agentpocket-desktop");
        let present = dir.path().join("agentpocket-gui");
        std::fs::write(&present, b"x").unwrap();

        assert_eq!(
            pick(&[missing.clone(), present.clone()]),
            Some(present.clone())
        );
        assert_eq!(pick(&[missing]), None);
        assert_eq!(pick(&[]), None);
    }

    #[test]
    fn candidates_include_sibling_of_current_exe() {
        let list = candidates();
        let exe = std::env::current_exe().unwrap();
        let dir = exe.parent().unwrap();
        assert!(list.contains(&dir.join("agentpocket-desktop")), "{list:?}");
    }
}
