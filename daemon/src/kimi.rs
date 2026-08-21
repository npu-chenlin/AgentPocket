//! Kimi Code CLI 探测与安装/升级（官方安装脚本，可重复执行即升级）。

use std::path::{Path, PathBuf};
use std::process::Command;

/// 官方安装脚本（免 Node，自动校验 checksum 并把 kimi 放入 PATH）。
pub const INSTALL_SCRIPT: &str = "curl -fsSL https://code.kimi.com/kimi-code/install.sh | bash";

/// 按 HOME 布局找 kimi 可执行文件：官方安装脚本装出的二进制位于数据目录，
/// 与 daemon 的 HOME 视角一致，故不做 PATH 兜底。
pub fn binary_path(home: &Path) -> Option<PathBuf> {
    let bundled = home.join(".kimi-code/bin/kimi");
    if bundled.is_file() {
        return Some(bundled);
    }
    let local_bin = home.join(".local/bin/kimi");
    if local_bin.is_file() {
        return Some(local_bin);
    }
    None
}

/// 运行 `kimi --version` 取首行版本号。
pub fn version(bin: &Path) -> Option<String> {
    let output = Command::new(bin).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
}

#[derive(Debug)]
pub struct UpgradeOutcome {
    pub before: Option<String>,
    pub after: Option<String>,
}

/// 执行官方安装脚本（未安装即安装，已安装即升级）。
pub fn install_or_upgrade(home: &Path) -> Result<UpgradeOutcome, String> {
    let before = binary_path(home).and_then(|bin| version(&bin));
    let output = Command::new("bash")
        .arg("-c")
        .arg(INSTALL_SCRIPT)
        .env("HOME", home)
        .current_dir(home)
        .output()
        .map_err(|e| format!("启动安装脚本失败：{e}"))?;
    if !output.status.success() {
        let tail = String::from_utf8_lossy(&output.stderr);
        let tail: String = tail.lines().rev().take(5).collect::<Vec<_>>().into_iter()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!("安装脚本退出码 {}：\n{}", output.status, tail));
    }
    // 安装后 PATH 可能新增了 kimi，但数据目录内置二进制布局最稳定，先按布局找
    let after = binary_path(home).and_then(|bin| version(&bin));
    Ok(UpgradeOutcome { before, after })
}

/// 供测试注入的带超时本地版本探测（mesh GET /kimi-info 用）。
pub fn info(home: &Path) -> (bool, Option<String>) {
    match binary_path(home) {
        Some(bin) => (true, version(&bin)),
        None => (false, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// 写一个可执行的假 kimi 脚本，echo 固定版本。
    fn fake_kimi(home: &Path, version: &str) -> PathBuf {
        let dir = home.join(".kimi-code/bin");
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("kimi");
        std::fs::write(&bin, format!("#!/bin/sh\necho {version}\n")).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        bin
    }

    #[test]
    fn binary_path_prefers_bundled_layout() {
        let home = tempfile::tempdir().unwrap();
        assert!(binary_path(home.path()).is_none());
        let bin = fake_kimi(home.path(), "0.99.0");
        assert_eq!(binary_path(home.path()), Some(bin));
    }

    #[test]
    fn version_reads_first_line() {
        let home = tempfile::tempdir().unwrap();
        let bin = fake_kimi(home.path(), "0.99.0");
        assert_eq!(version(&bin).as_deref(), Some("0.99.0"));
        assert!(info(home.path()).0);
    }

    #[test]
    fn missing_kimi_reports_not_installed() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(info(home.path()), (false, None));
    }
}
