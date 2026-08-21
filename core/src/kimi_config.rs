//! ~/.kimi-code/config.toml 同步：单文件、整体替换，覆盖前备份为 config.toml.bak。

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// 当前进程的 $HOME（daemon 以 systemd 用户运行，HOME 由 unit 指定）。
pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

/// <home>/.kimi-code/config.toml。
pub fn config_path(home: &Path) -> PathBuf {
    home.join(".kimi-code").join("config.toml")
}

pub fn read(home: &Path) -> Result<String, String> {
    let path = config_path(home);
    std::fs::read_to_string(&path)
        .map_err(|e| format!("读取 {} 失败：{e}", path.display()))
}

/// 覆盖写入；已有文件先备份为同目录 config.toml.bak（仅保留一代）。
pub fn write(home: &Path, content: &str) -> Result<(), String> {
    let path = config_path(home);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建 {} 失败：{e}", parent.display()))?;
    }
    if path.exists() {
        let backup = path.with_extension("toml.bak");
        std::fs::copy(&path, &backup)
            .map_err(|e| format!("备份 {} 失败：{e}", path.display()))?;
    }
    std::fs::write(&path, content).map_err(|e| format!("写入 {} 失败：{e}", path.display()))?;
    // config.toml 含 API key，收紧为属主可读写
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_reads_back_and_backs_up() {
        let home = tempfile::tempdir().unwrap();
        write(home.path(), "model = \"k2\"\n").unwrap();
        assert_eq!(read(home.path()).unwrap(), "model = \"k2\"\n");
        let mode = std::fs::metadata(config_path(home.path())).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);

        write(home.path(), "model = \"k3\"\n").unwrap();
        let backup = home.path().join(".kimi-code/config.toml.bak");
        assert_eq!(std::fs::read_to_string(backup).unwrap(), "model = \"k2\"\n");
        assert_eq!(read(home.path()).unwrap(), "model = \"k3\"\n");
    }

    #[test]
    fn read_missing_is_error() {
        let home = tempfile::tempdir().unwrap();
        assert!(read(home.path()).is_err());
    }
}
