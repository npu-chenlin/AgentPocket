//! Kimi Code CLI 探测与安装/升级。
//! 升级语义 = 归一化到官方安装脚本版：先卸载探测到的 npm 全局安装，再执行官方脚本。

use std::path::{Path, PathBuf};
use std::process::Command;

/// 官方安装脚本（免 Node，自动校验 checksum 并把 kimi 放入 PATH）。
pub const INSTALL_SCRIPT: &str = "curl -fsSL https://code.kimi.com/kimi-code/install.sh | bash";
/// npm 包名。
const NPM_PKG: &str = "@moonshot-ai/kimi-code";
/// npm 全局包相对前缀的路径。
const NPM_PKG_REL: &str = "lib/node_modules/@moonshot-ai/kimi-code";

/// 官方布局下的 kimi 可执行文件（不做 PATH 兜底，与 daemon 的 HOME 视角一致）。
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

#[derive(Debug, Default)]
pub struct NpmInstall {
    pub prefix: PathBuf,
    pub version: Option<String>,
}

/// 探测 npm 全局安装：nvm 管理的各 node 版本 + 常见系统前缀。
pub fn npm_installs(home: &Path) -> Vec<NpmInstall> {
    let mut prefixes: Vec<PathBuf> = Vec::new();
    let nvm_root = home.join(".nvm/versions/node");
    if let Ok(entries) = std::fs::read_dir(&nvm_root) {
        let mut versions: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        versions.sort();
        prefixes.extend(versions);
    }
    prefixes.push(home.join(".local"));
    prefixes.push(PathBuf::from("/usr/local"));
    prefixes.push(PathBuf::from("/usr"));

    prefixes
        .into_iter()
        .filter_map(|prefix| {
            let pkg = prefix.join(NPM_PKG_REL);
            if !pkg.is_dir() {
                return None;
            }
            let version = std::fs::read_to_string(pkg.join("package.json"))
                .ok()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
                .and_then(|v| v["version"].as_str().map(String::from));
            Some(NpmInstall { prefix, version })
        })
        .collect()
}

/// 卸载全部 npm 安装（各用其前缀自带的 npm）；返回（成功前缀，失败描述）。
pub fn uninstall_npm(home: &Path) -> (Vec<PathBuf>, Vec<String>) {
    let mut removed = Vec::new();
    let mut failed = Vec::new();
    for install in npm_installs(home) {
        let npm = install.prefix.join("bin/npm");
        let result = Command::new(&npm)
            .args(["uninstall", "-g", NPM_PKG])
            .env("HOME", home)
            .status();
        match result {
            Ok(status) if status.success() => removed.push(install.prefix),
            Ok(status) => failed.push(format!("{}：npm 退出码 {}", install.prefix.display(), status)),
            Err(e) => failed.push(format!("{}：{e}", install.prefix.display())),
        }
    }
    (removed, failed)
}

/// 本机 Kimi Code CLI 全貌：官方版版本 + npm 安装列表。
#[derive(Debug, Default)]
pub struct KimiState {
    pub official: Option<String>,
    pub npm: Vec<NpmInstall>,
}

pub fn detect(home: &Path) -> KimiState {
    KimiState {
        official: binary_path(home).and_then(|bin| version(&bin)),
        npm: npm_installs(home),
    }
}

#[derive(Debug)]
pub struct EnsureOutcome {
    pub before: Option<String>,
    pub after: Option<String>,
    pub npm_removed: Vec<PathBuf>,
    pub npm_failed: Vec<String>,
}

/// 归一化到官方版：卸载 npm 安装 → 执行官方安装脚本（未装即装，已装即升级）。
pub fn ensure_official(home: &Path) -> Result<EnsureOutcome, String> {
    let before = binary_path(home).and_then(|bin| version(&bin));
    let (npm_removed, npm_failed) = uninstall_npm(home);
    let output = Command::new("bash")
        .arg("-c")
        .arg(INSTALL_SCRIPT)
        .env("HOME", home)
        .current_dir(home)
        .output()
        .map_err(|e| format!("启动安装脚本失败：{e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: Vec<&str> = stderr.lines().rev().take(5).collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        return Err(format!("安装脚本退出码 {}：\n{}", output.status, tail.join("\n")));
    }
    let after = binary_path(home).and_then(|bin| version(&bin));
    Ok(EnsureOutcome { before, after, npm_removed, npm_failed })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// 写一个可执行的假 kimi 脚本，echo 固定版本。
    fn fake_official(home: &Path, version: &str) -> PathBuf {
        let dir = home.join(".kimi-code/bin");
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("kimi");
        std::fs::write(&bin, format!("#!/bin/sh\necho {version}\n")).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        bin
    }

    /// 伪造一个 nvm 前缀下的 npm 全局安装（package.json + 记录参数的假 npm）。
    fn fake_npm_install(home: &Path, node_ver: &str, pkg_ver: &str) -> PathBuf {
        let prefix = home.join(format!(".nvm/versions/node/{node_ver}"));
        let pkg = prefix.join(NPM_PKG_REL);
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(
            pkg.join("package.json"),
            format!(r#"{{"name":"{NPM_PKG}","version":"{pkg_ver}"}}"#),
        )
        .unwrap();
        let log = home.join(format!("npm-{node_ver}.log"));
        let npm = prefix.join("bin/npm");
        std::fs::create_dir_all(prefix.join("bin")).unwrap();
        std::fs::write(
            &npm,
            format!("#!/bin/sh\necho \"$@\" >> {}\nexit 0\n", log.display()),
        )
        .unwrap();
        std::fs::set_permissions(&npm, std::fs::Permissions::from_mode(0o755)).unwrap();
        prefix
    }

    #[test]
    fn binary_path_prefers_bundled_layout() {
        let home = tempfile::tempdir().unwrap();
        assert!(binary_path(home.path()).is_none());
        let bin = fake_official(home.path(), "0.99.0");
        assert_eq!(binary_path(home.path()), Some(bin));
    }

    #[test]
    fn version_reads_first_line() {
        let home = tempfile::tempdir().unwrap();
        let bin = fake_official(home.path(), "0.99.0");
        assert_eq!(version(&bin).as_deref(), Some("0.99.0"));
    }

    #[test]
    fn detect_reports_official_and_npm_sources() {
        let home = tempfile::tempdir().unwrap();
        fake_official(home.path(), "0.38.0");
        fake_npm_install(home.path(), "v22.22.0", "0.37.2");

        let state = detect(home.path());

        assert_eq!(state.official.as_deref(), Some("0.38.0"));
        assert_eq!(state.npm.len(), 1);
        assert_eq!(state.npm[0].version.as_deref(), Some("0.37.2"));
    }

    #[test]
    fn detect_missing_reports_empty() {
        let home = tempfile::tempdir().unwrap();
        let state = detect(home.path());
        assert!(state.official.is_none());
        assert!(state.npm.is_empty());
    }

    #[test]
    fn uninstall_npm_invokes_prefix_npm() {
        let home = tempfile::tempdir().unwrap();
        let prefix = fake_npm_install(home.path(), "v22.22.0", "0.37.2");

        let (removed, failed) = uninstall_npm(home.path());

        assert_eq!(removed, vec![prefix.clone()]);
        assert!(failed.is_empty());
        let log = std::fs::read_to_string(home.path().join("npm-v22.22.0.log")).unwrap();
        assert!(log.contains(&format!("uninstall -g {NPM_PKG}")));
        // 卸载动作只是调 npm；目录仍由 npm 自己清理，这里不模拟。
        let _ = prefix;
    }
}
