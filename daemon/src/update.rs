//! 自动更新：GitHub releases 检查 → 下载资产 → 原子替换自身 → systemd 重启。
//! 仅本模块走 HTTPS（ureq + rustls），mesh 链路保持 std 明文 HTTP。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use semver::Version;
use serde::Deserialize;

pub const REPO: &str = "npu-chenlin/AgentPocket";
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const INITIAL_DELAY: Duration = Duration::from_secs(10);

#[derive(Debug)]
pub enum UpdateError {
    Network(String),
    Parse(String),
    Io(String),
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdateError::Network(e) => write!(f, "更新检查网络错误：{e}"),
            UpdateError::Parse(e) => write!(f, "更新检查解析错误：{e}"),
            UpdateError::Io(e) => write!(f, "更新写入失败：{e}"),
        }
    }
}

#[derive(Debug)]
pub struct ReleaseInfo {
    pub version: Version,
    pub asset_url: String,
}

pub fn arch_asset_name() -> String {
    format!("agentpocket-{}-linux-musl", std::env::consts::ARCH)
}

pub fn parse_tag_version(tag: &str) -> Result<Version, UpdateError> {
    tag.trim_start_matches('v')
        .parse()
        .map_err(|e: semver::Error| UpdateError::Parse(format!("{tag}: {e}")))
}

#[derive(Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<ReleaseAsset>,
}

/// 查询最新 release；仅当远端版本严格大于 current 且带本架构资产时返回 Some。
pub fn fetch_latest(
    api_base: &str,
    current: &Version,
    timeout: Duration,
) -> Result<Option<ReleaseInfo>, UpdateError> {
    let url = format!("{api_base}/repos/{REPO}/releases/latest");
    let body = http_get_string(&url, timeout)?;
    let release: GithubRelease =
        serde_json::from_str(&body).map_err(|e| UpdateError::Parse(e.to_string()))?;
    let version = parse_tag_version(&release.tag_name)?;
    if version <= *current {
        return Ok(None);
    }
    let wanted = arch_asset_name();
    let asset_url = release
        .assets
        .into_iter()
        .find(|asset| asset.name == wanted)
        .map(|asset| asset.browser_download_url);
    Ok(asset_url.map(|asset_url| ReleaseInfo { version, asset_url }))
}

/// 下载资产并原子替换 self_path（写临时文件 → fsync → chmod 755 → rename）。
pub fn download_and_replace(
    url: &str,
    self_path: &Path,
    timeout: Duration,
) -> Result<(), UpdateError> {
    let bytes = http_get_bytes(url, timeout)?;
    if bytes.is_empty() {
        return Err(UpdateError::Network("资产内容为空".to_string()));
    }
    let tmp_path = self_path.with_extension("new");
    {
        use std::io::Write as _;
        let mut file = std::fs::File::create(&tmp_path)
            .map_err(|e| UpdateError::Io(e.to_string()))?;
        file.write_all(&bytes).map_err(|e| UpdateError::Io(e.to_string()))?;
        file.sync_all().map_err(|e| UpdateError::Io(e.to_string()))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| UpdateError::Io(e.to_string()))?;
    }
    std::fs::rename(&tmp_path, self_path).map_err(|e| UpdateError::Io(e.to_string()))
}

/// 在 systemd 环境下重启服务；返回是否尝试了重启。
pub fn restart_systemd() -> bool {
    let under_systemd = std::env::var_os("INVOCATION_ID").is_some()
        || Path::new("/run/systemd/system").exists();
    if !under_systemd {
        return false;
    }
    std::process::Command::new("systemctl")
        .args(["restart", "agentpocket"])
        .status()
        .is_ok_and(|status| status.success())
}

fn self_path() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// 执行一轮检查+更新；返回描述文案（serve 循环与 update 命令共用）。
pub fn check_and_apply(api_base: &str, timeout: Duration) -> Result<String, UpdateError> {
    let current = Version::parse(env!("CARGO_PKG_VERSION"))
        .expect("crate version is valid semver");
    let Some(release) = fetch_latest(api_base, &current, timeout)? else {
        return Ok("已是最新版本".to_string());
    };
    let Some(path) = self_path() else {
        return Err(UpdateError::Io("无法定位自身路径".to_string()));
    };
    let message = format!("更新到 {}：", release.version);
    download_and_replace(&release.asset_url, &path, timeout)?;
    if restart_systemd() {
        Ok(format!("{message}已替换并重启服务"))
    } else {
        Ok(format!("{message}已替换，请手动重启"))
    }
}

/// serve 内的后台更新线程：10s 后首查，之后每 24h。
pub fn spawn_update_loop() -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || loop {
        std::thread::sleep(INITIAL_DELAY);
        match check_and_apply("https://api.github.com", Duration::from_secs(60)) {
            Ok(message) => println!("[update] {message}"),
            Err(e) => eprintln!("[update] {e}"),
        }
        std::thread::sleep(CHECK_INTERVAL - INITIAL_DELAY);
    })
}

fn http_get_string(url: &str, timeout: Duration) -> Result<String, UpdateError> {
    let response = ureq::AgentBuilder::new()
        .timeout(timeout)
        .build()
        .get(url)
        .call()
        .map_err(|e| UpdateError::Network(e.to_string()))?;
    response
        .into_string()
        .map_err(|e| UpdateError::Network(e.to_string()))
}

fn http_get_bytes(url: &str, timeout: Duration) -> Result<Vec<u8>, UpdateError> {
    let response = ureq::AgentBuilder::new()
        .timeout(timeout)
        .build()
        .get(url)
        .call()
        .map_err(|e| UpdateError::Network(e.to_string()))?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| UpdateError::Network(e.to_string()))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const TIMEOUT: Duration = Duration::from_secs(3);

    #[test]
    fn parses_v_prefixed_tag() {
        assert_eq!(parse_tag_version("v2.8.1").unwrap(), semver::Version::new(2, 8, 1));
        assert!(parse_tag_version("not-a-version").is_err());
    }

    #[test]
    fn asset_name_matches_arch() {
        let name = arch_asset_name();
        assert!(name.starts_with("agentpocket-"));
        assert!(name.ends_with("-linux-musl"));
    }

    #[test]
    fn fetch_latest_picks_asset_and_ignores_older() {
        // 本地 mock GitHub API：发布 v2.9.0，带两个资产。
        let server = tiny_http::Server::http(("127.0.0.1", 0)).unwrap();
        let port = match server.server_addr() {
            tiny_http::ListenAddr::IP(addr) => addr.port(),
            other => panic!("unexpected listen addr: {other:?}"),
        };
        std::thread::spawn(move || {
            // 资产名按当前架构生成，URL 中带上资产名以便断言选中了 musl 资产。
            let wanted = arch_asset_name();
            let wanted_url = format!("http://127.0.0.1:0/{wanted}");
            for request in server.incoming_requests() {
                let body = serde_json::json!({
                    "tag_name": "v2.9.0",
                    "assets": [
                        {"name": wanted, "browser_download_url": wanted_url},
                        {"name": "app.apk", "browser_download_url": "http://127.0.0.1:0/apk"}
                    ]
                })
                .to_string();
                let _ = request.respond(tiny_http::Response::from_string(body));
            }
        });

        let base = format!("http://127.0.0.1:{port}");
        let current = semver::Version::new(2, 8, 0);
        let release = fetch_latest(&base, &current, TIMEOUT).unwrap().expect("newer release");
        assert_eq!(release.version, semver::Version::new(2, 9, 0));
        assert!(release.asset_url.contains("musl"));

        // 当前已是 2.9.0 → 不更新。
        let same = semver::Version::new(2, 9, 0);
        assert!(fetch_latest(&base, &same, TIMEOUT).unwrap().is_none());
    }

    #[test]
    fn download_and_replace_swaps_file_atomically() {
        // mock 资产服务器返回新二进制内容。
        let server = tiny_http::Server::http(("127.0.0.1", 0)).unwrap();
        let port = match server.server_addr() {
            tiny_http::ListenAddr::IP(addr) => addr.port(),
            other => panic!("unexpected listen addr: {other:?}"),
        };
        std::thread::spawn(move || {
            for request in server.incoming_requests() {
                let _ = request.respond(tiny_http::Response::from_string("new binary bytes"));
            }
        });

        let dir = tempfile::tempdir().unwrap();
        let self_path = dir.path().join("agentpocket");
        std::fs::write(&self_path, b"old").unwrap();

        download_and_replace(
            &format!("http://127.0.0.1:{port}/asset"),
            &self_path,
            TIMEOUT,
        )
        .unwrap();

        assert_eq!(std::fs::read(&self_path).unwrap(), b"new binary bytes");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&self_path).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111);
        }
    }
}
