# AgentPocket Mesh 守护进程（Phase 1）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 交付独立 `agentpocket` 无头守护进程：mesh 端点（48720，tailnet 边界）+ tailscale 发现 + pull/push + status + 自动更新 + 一键安装脚本；GUI 与 Android 零行为变化。

**Architecture:** 从 `desktop/src-tauri` 抽出 `model.rs`/`config.rs` 成共享 crate `core/`（GUI 留 `pub use` 转发 shim）；新建 `daemon/` crate（tiny_http 端点 + std TcpStream 手写 HTTP 客户端 + clap CLI），仅自动更新用 ureq(rustls) 走 HTTPS。三个 crate 各自独立、path 依赖互联，不建 workspace。

**Tech Stack:** Rust 2021、tiny_http 0.12、serde/serde_json、clap 4（derive）、semver 1、ureq 2（rustls）、musl 静态编译（x86_64-unknown-linux-musl）。

**Spec:** `docs/superpowers/specs/2026-08-20-mesh-sync-design.md`

## Global Constraints

- **GUI 行为零变化**：`desktop/` 只允许两处改动——`src-tauri/src/model.rs`、`src-tauri/src/config.rs` 变为 `pub use agentpocket_core::…::*;` 转发 shim，`src-tauri/Cargo.toml` 增加 `agentpocket-core = { path = "../../core" }`。`app/`（Android）不动。
- mesh 端点：监听 `0.0.0.0:48720`（IPv4）；`is_peer_allowed` 只放行 `127.0.0.0/8` 与 `100.64.0.0/10`（`o[0]==100 && 64..=127.contains(o[1])`）；IPv6 一律拒绝；其余 403 不读 body。
- 交换格式沿用 schema 1（`export_text`/`preview_import_text`/`apply_import`/`ImportMode::Merge`），不改 core 公开 API 语义。
- 版本：`core` 与 `daemon` crate 均 `2.8.0`；GUI 保持 `2.7.0` 不动。
- 测试基线：`desktop/src-tauri` 现有 83 个测试必须保持全绿。
- 所有 cargo 命令前先执行：
  ```bash
  source "$HOME/.cargo/env"
  export CARGO_REGISTRIES_CRATES_IO_INDEX='sparse+https://rsproxy.cn/index/'
  export CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse
  ```
- git 一律 `rtk` 前缀（`rtk git add` / `rtk git commit`）；commit 信息中文、`feat(daemon):`/`refactor:`/`chore:` 前缀。
- daemon 代码注释用中文，风格对齐现有 `sync.rs`（模块顶注释 + 简短行内注释）。

---

### Task 1: 抽取 core crate（model + config）

**Files:**
- Create: `core/Cargo.toml`、`core/src/lib.rs`
- Move: `desktop/src-tauri/src/model.rs` → `core/src/model.rs`；`desktop/src-tauri/src/config.rs` → `core/src/config.rs`（用 `git mv` 保留历史）
- Modify: `desktop/src-tauri/Cargo.toml`（加 path 依赖）
- Replace: `desktop/src-tauri/src/model.rs`、`desktop/src-tauri/src/config.rs`（shim 重新创建）

**Interfaces:**
- Consumes: 无（首个任务）
- Produces: crate `agentpocket-core`，模块 `agentpocket_core::model`（`AppConfig`/`ServerConfig`/`Backend`/`DesktopSettings` 等）与 `agentpocket_core::config`（`ConfigStore::new(PathBuf)`、`load() -> LoadOutcome`、`save(&AppConfig)`、`preview_import_text(&str) -> ImportPreviewData`、`apply_import(&AppConfig, ImportPreviewData, ImportMode) -> AppConfig`、`export_text(&AppConfig) -> String`、`ImportMode::{Merge,Replace}`）。后续所有任务通过这些签名使用。

- [ ] **Step 1: 创建 core crate 骨架**

`core/Cargo.toml`：

```toml
[package]
name = "agentpocket-core"
version = "2.8.0"
edition = "2021"

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
url = "2"
uuid = { version = "1", features = ["v4"] }
chrono = { version = "0.4", features = ["serde"] }

[dev-dependencies]
tempfile = "3"
```

（版本对齐 `desktop/src-tauri/Cargo.toml` 现有依赖。）

`core/src/lib.rs`：

```rust
pub mod config;
pub mod model;
```

- [ ] **Step 2: git mv 两个模块**

```bash
cd /home/user/progs/KimiCodeWebApp
git mv desktop/src-tauri/src/model.rs core/src/model.rs
git mv desktop/src-tauri/src/config.rs core/src/config.rs
```

`config.rs` 内的 `use crate::model::{…}` 在 core crate 里同样成立（同 crate），无需改动。`model.rs` 中 `default_schema` 是 `pub(crate)`，仅被同文件 serde 属性使用，无需放开。

- [ ] **Step 3: GUI 侧留转发 shim**

`desktop/src-tauri/Cargo.toml` 的 `[dependencies]` 增加：

```toml
agentpocket-core = { path = "../../core" }
```

新建 `desktop/src-tauri/src/model.rs`：

```rust
pub use agentpocket_core::model::*;
```

新建 `desktop/src-tauri/src/config.rs`：

```rust
pub use agentpocket_core::config::*;
```

（`lib.rs` 的 `pub mod model; pub mod config;` 不变；`crate::model::X` / `crate::config::X` 全部经 re-export 继续可用。）

- [ ] **Step 4: 验证 GUI 测试基线不被破坏**

```bash
cd /home/user/progs/KimiCodeWebApp/desktop/src-tauri && cargo test --offline
```

Expected: 与抽取前相同的 83 个测试全部 PASS（测试随文件搬进 core，同时 src-tauri 里引用 `crate::config` 的测试经 shim 编译通过）。

- [ ] **Step 5: core crate 自身测试**

```bash
cd /home/user/progs/KimiCodeWebApp/core && cargo test
```

Expected: 搬入的 model/config 测试全部 PASS。

- [ ] **Step 6: Commit**

```bash
cd /home/user/progs/KimiCodeWebApp
rtk git add core desktop/src-tauri
rtk git commit -m "refactor: 抽取 model/config 为 agentpocket-core 共享 crate（GUI 行为不变）"
```

---

### Task 2: daemon crate 脚手架 + mesh 端点（/info、访问控制、serve/version 命令）

**Files:**
- Create: `daemon/Cargo.toml`、`daemon/src/main.rs`、`daemon/src/mesh.rs`、`daemon/src/paths.rs`
- Test: `daemon/src/mesh.rs`（`#[cfg(test)]` 内嵌，风格对齐 `sync.rs`）

**Interfaces:**
- Consumes: 无（不依赖 core 的端点行为）
- Produces:
  - `mesh::MESH_PORT: u16 = 48720`
  - `mesh::is_peer_allowed(&SocketAddr) -> bool`
  - `mesh::MeshContext { config_dir: PathBuf, version: &'static str, hostname: String }`
  - `mesh::start(MeshContext, port: u16) -> Result<MeshHandle, MeshError>`（port 0 = 随机端口；`MeshHandle { port, … }`，`stop(self)`、`wait(self)`）
  - `paths::default_config_dir() -> PathBuf`（XDG 解析，后续任务共用）
  - `paths::hostname() -> String`
  - CLI：`agentpocket serve`、`agentpocket version`（后续任务再加其余子命令）

- [ ] **Step 1: daemon crate 骨架**

`daemon/Cargo.toml`：

```toml
[package]
name = "agentpocket"
version = "2.8.0"
edition = "2021"

[dependencies]
agentpocket-core = { path = "../core" }
tiny_http = "0.12"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
clap = { version = "4", features = ["derive"] }
uuid = { version = "1", features = ["v4"] }

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 2: 写失败测试（is_peer_allowed 矩阵 + /info + 404）**

`daemon/src/mesh.rs` 先写测试骨架（实现部分先空着使编译失败即可）：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;

    fn ctx(dir: &std::path::Path) -> MeshContext {
        MeshContext {
            config_dir: dir.to_path_buf(),
            version: "2.8.0-test",
            hostname: "test-host".to_string(),
        }
    }

    fn raw_request(port: u16, request: &str) -> (u16, String) {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        let text = String::from_utf8_lossy(&response).into_owned();
        let status = text.split_whitespace().nth(1)
            .and_then(|c| c.parse::<u16>().ok()).expect("status code");
        let body = text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
        (status, body)
    }

    #[test]
    fn is_peer_allowed_matrix() {
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};
        let sa = |a: u8, b: u8, c: u8, d: u8| SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(a, b, c, d)), 48720);
        assert!(is_peer_allowed(&sa(127, 0, 0, 1)));
        assert!(is_peer_allowed(&sa(100, 64, 0, 2)));
        assert!(is_peer_allowed(&sa(100, 127, 255, 254)));
        assert!(!is_peer_allowed(&sa(100, 128, 0, 1)));
        assert!(!is_peer_allowed(&sa(192, 168, 1, 10)));
        assert!(!is_peer_allowed(&sa(8, 8, 8, 8)));
        let v6 = "[fd7a:115c:a1e0::1]:48720".parse::<SocketAddr>().unwrap();
        assert!(!is_peer_allowed(&v6));
    }

    #[test]
    fn info_endpoint_answers_with_identity() {
        let dir = tempfile::tempdir().unwrap();
        let handle = start(ctx(dir.path()), 0).unwrap();
        let (status, body) = raw_request(
            handle.port,
            "GET /info HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        );
        assert_eq!(status, 200);
        let value: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(value["app"], "agentpocket");
        assert_eq!(value["version"], "2.8.0-test");
        assert_eq!(value["name"], "test-host");
        handle.stop();
    }

    #[test]
    fn unknown_path_returns_404() {
        let dir = tempfile::tempdir().unwrap();
        let handle = start(ctx(dir.path()), 0).unwrap();
        let (status, _) = raw_request(
            handle.port,
            "GET /nope HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        );
        assert_eq!(status, 404);
        handle.stop();
    }
}
```

- [ ] **Step 3: 跑测试确认失败**

```bash
cd /home/user/progs/KimiCodeWebApp/daemon && cargo test
```

Expected: 编译失败（`start`/`is_peer_allowed`/`MeshContext` 未定义）。

- [ ] **Step 4: 实现 mesh.rs**

```rust
//! mesh HTTP 端点：固定端口、tailnet 访问围栏、/info 握手。
//! /config 的 GET/POST 在本模块由后续提交补齐（见 Task 3）。

use std::io::Read as _;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use tiny_http::{Method, Request, Response, Server, StatusCode};

/// mesh 固定监听端口。
pub const MESH_PORT: u16 = 48720;
/// recv 轮询间隔，保证 stop 信号能被及时检查。
const POLL_INTERVAL: Duration = Duration::from_millis(200);

#[derive(Debug)]
pub enum MeshError {
    Bind(String),
}

impl std::fmt::Display for MeshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MeshError::Bind(e) => write!(f, "mesh 端点绑定失败（端口被占用？）：{e}"),
        }
    }
}

/// 端点运行上下文：配置目录（core ConfigStore 用）+ 自报身份。
pub struct MeshContext {
    pub config_dir: PathBuf,
    pub version: &'static str,
    pub hostname: String,
}

/// 只放行回环与 Tailscale CGNAT 网段（100.64.0.0/10）；网络边界即权限边界。
pub fn is_peer_allowed(addr: &SocketAddr) -> bool {
    match addr.ip() {
        std::net::IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback() || (o[0] == 100 && (64..=127).contains(&o[1]))
        }
        std::net::IpAddr::V6(_) => false,
    }
}

pub struct MeshHandle {
    pub port: u16,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl MeshHandle {
    pub fn stop(mut self) {
        self.shutdown();
    }

    /// 阻塞直到服务线程退出（serve 命令用）。
    pub fn wait(mut self) {
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for MeshHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// 启动端点。port 传 0 表示随机端口（测试用）。
pub fn start(ctx: MeshContext, port: u16) -> Result<MeshHandle, MeshError> {
    let server = Server::http(("0.0.0.0", port)).map_err(|e| MeshError::Bind(e.to_string()))?;
    let bound = match server.server_addr() {
        tiny_http::ListenAddr::IP(addr) => addr.port(),
        other => return Err(MeshError::Bind(format!("unexpected listen addr: {other:?}"))),
    };

    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_thread = Arc::clone(&stop);
    let ctx = Arc::new(ctx);
    let join = std::thread::spawn(move || {
        loop {
            if stop_for_thread.load(Ordering::SeqCst) {
                break;
            }
            match server.recv_timeout(POLL_INTERVAL) {
                Ok(Some(request)) => handle_request(request, &ctx),
                Ok(None) => continue,
                Err(_) => break,
            }
        }
    });

    Ok(MeshHandle { port: bound, stop, join: Some(join) })
}

fn handle_request(mut request: Request, ctx: &MeshContext) {
    let allowed = request.remote_addr().map(is_peer_allowed).unwrap_or(false);
    if !allowed {
        let _ = request.respond(Response::empty(StatusCode(403)));
        return;
    }
    let path = request.url().split('?').next().unwrap_or("/").to_string();
    match (*request.method(), path.as_str()) {
        (Method::Get, "/info") => {
            let body = serde_json::json!({
                "app": "agentpocket",
                "version": ctx.version,
                "name": ctx.hostname,
            });
            let _ = request.respond(Response::from_string(body.to_string())
                .with_status_code(StatusCode(200))
                .with_header(json_header()));
        }
        _ => {
            let _ = request.respond(Response::empty(StatusCode(404)));
        }
    }
}

fn json_header() -> tiny_http::Header {
    tiny_http::Header::from_bytes("Content-Type", "application/json; charset=utf-8")
        .expect("static header is valid")
}
```

`daemon/src/paths.rs`：

```rust
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
```

`daemon/src/main.rs`：

```rust
mod mesh;
mod paths;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "agentpocket", about = "AgentPocket mesh 守护进程")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 前台运行：mesh 端点 + 自动更新（systemd 拉起此命令）
    Serve,
    /// 打印版本
    Version,
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Serve => {
            let ctx = mesh::MeshContext {
                config_dir: paths::default_config_dir(),
                version: env!("CARGO_PKG_VERSION"),
                hostname: paths::hostname(),
            };
            match mesh::start(ctx, mesh::MESH_PORT) {
                Ok(handle) => {
                    println!(
                        "[mesh] 监听 0.0.0.0:{}（仅 tailnet/回环可达），配置目录 {}",
                        mesh::MESH_PORT,
                        paths::default_config_dir().display()
                    );
                    handle.wait();
                }
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        Command::Version => println!("{}", env!("CARGO_PKG_VERSION")),
    }
}
```

- [ ] **Step 5: 跑测试确认通过**

```bash
cd /home/user/progs/KimiCodeWebApp/daemon && cargo test
```

Expected: 3 个测试 PASS。

- [ ] **Step 6: Commit**

```bash
cd /home/user/progs/KimiCodeWebApp
rtk git add daemon
rtk git commit -m "feat(daemon): crate 脚手架与 mesh 端点（/info 握手 + tailnet 访问围栏）"
```

---

### Task 3: mesh /config GET + POST（自动合并、备份、来源日志）

**Files:**
- Modify: `daemon/src/mesh.rs`（`handle_request` 增加 `/config` 分支 + 测试）

**Interfaces:**
- Consumes: `agentpocket_core::config::{ConfigStore, ImportMode}`（Task 1）
- Produces: `GET /config` → 200 交换格式文本；`POST /config`（body = 交换格式，头 `X-AgentPocket-Source` 可选）→ 200 `{"added":K,"updated":M}`；非法 body → 400。Task 4/5 的集成测试直接以 HTTP 调用这两个端点。

- [ ] **Step 1: 写失败测试（追加到 mesh.rs tests 模块）**

```rust
    use agentpocket_core::model::{Backend, ServerConfig};

    fn seed_config(dir: &std::path::Path) {
        let store = agentpocket_core::config::ConfigStore::new(dir.to_path_buf());
        let config = agentpocket_core::model::AppConfig {
            active_id: Some("s1".to_string()),
            servers: vec![ServerConfig::new(
                "s1", "Old", "100.64.0.2", 3080, "tok", Backend::Dsh,
            )],
            ..Default::default()
        };
        store.save(&config).unwrap();
    }

    #[test]
    fn get_config_returns_exchange_format() {
        let dir = tempfile::tempdir().unwrap();
        seed_config(dir.path());
        let handle = start(ctx(dir.path()), 0).unwrap();

        let (status, body) = raw_request(
            handle.port,
            "GET /config HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        );

        assert_eq!(status, 200);
        let value: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(value["schema"], 1);
        assert_eq!(value["servers"][0]["name"], "Old");
        assert!(value.get("settings").is_none());
        handle.stop();
    }

    #[test]
    fn post_config_merges_and_counts_and_backs_up() {
        let dir = tempfile::tempdir().unwrap();
        seed_config(dir.path());
        let handle = start(ctx(dir.path()), 0).unwrap();

        let body = r#"{"schema":1,"servers":[
            {"id":"s1","name":"Renamed","host":"100.64.0.2","port":3080,"token":"tok","backend":"dsh"},
            {"id":"s2","name":"New","host":"100.64.0.3","port":58627,"backend":"kimi"}]}"#;
        let (status, resp_body) = raw_request(
            handle.port,
            &format!(
                "POST /config HTTP/1.1\r\nHost: localhost\r\nX-AgentPocket-Source: peer-a\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            ),
        );

        assert_eq!(status, 200);
        let counts: serde_json::Value = serde_json::from_str(&resp_body).unwrap();
        assert_eq!(counts["added"], 1);
        assert_eq!(counts["updated"], 1);

        // 落盘结果：合并后的两台 + activeId 保持。
        let outcome = agentpocket_core::config::ConfigStore::new(dir.path().to_path_buf())
            .load().unwrap();
        assert_eq!(outcome.config.servers.len(), 2);
        assert_eq!(outcome.config.servers[0].name, "Renamed");
        assert_eq!(outcome.config.active_id.as_deref(), Some("s1"));
        // 导入前自动备份已生成。
        assert!(dir.path().join("backups").is_dir());
        handle.stop();
    }

    #[test]
    fn post_garbage_returns_400_without_touching_disk() {
        let dir = tempfile::tempdir().unwrap();
        seed_config(dir.path());
        let handle = start(ctx(dir.path()), 0).unwrap();

        let (status, _) = raw_request(
            handle.port,
            "POST /config HTTP/1.1\r\nHost: localhost\r\nContent-Length: 15\r\nConnection: close\r\n\r\nnot json at all",
        );

        assert_eq!(status, 400);
        let outcome = agentpocket_core::config::ConfigStore::new(dir.path().to_path_buf())
            .load().unwrap();
        assert_eq!(outcome.config.servers.len(), 1);
        handle.stop();
    }
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cd /home/user/progs/KimiCodeWebApp/daemon && cargo test
```

Expected: 新增 3 个测试 FAIL（/config 目前走 404），原有 3 个 PASS。

- [ ] **Step 3: 实现 /config 分支**

在 `handle_request` 的 match 中 `/info` 分支后追加（`use std::collections::HashSet;` 与 `MAX_BODY_BYTES` 常量放模块顶部）：

```rust
        (Method::Get, "/config") => {
            let store = agentpocket_core::config::ConfigStore::new(ctx.config_dir.clone());
            let current = match store.load() {
                Ok(outcome) => outcome.config,
                Err(_) => agentpocket_core::model::AppConfig::default(),
            };
            match store.export_text(&current) {
                Ok(text) => {
                    let _ = request.respond(Response::from_string(text)
                        .with_status_code(StatusCode(200))
                        .with_header(json_header()));
                }
                Err(e) => {
                    let _ = request.respond(Response::from_string(e.to_string())
                        .with_status_code(StatusCode(500)));
                }
            }
        }
        (Method::Post, "/config") => {
            let source = request
                .headers()
                .iter()
                .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case("X-AgentPocket-Source"))
                .map(|h| h.value.as_str().to_string())
                .or_else(|| {
                    request.remote_addr().map(|a| a.ip().to_string())
                })
                .unwrap_or_else(|| "未知来源".to_string());

            let mut body = String::new();
            let read_ok = request
                .as_reader()
                .take(MAX_BODY_BYTES)
                .read_to_string(&mut body)
                .is_ok();
            if !read_ok {
                let _ = request.respond(Response::empty(StatusCode(400)));
                return;
            }

            let store = agentpocket_core::config::ConfigStore::new(ctx.config_dir.clone());
            let current = match store.load() {
                Ok(outcome) => outcome.config,
                Err(_) => agentpocket_core::model::AppConfig::default(),
            };
            let result = (|| -> Result<(usize, usize), String> {
                let data = store
                    .preview_import_text(&body)
                    .map_err(|e| e.to_string())?;
                if data.config.servers.is_empty() {
                    return Err("没有可导入的有效服务器".to_string());
                }
                let old_ids: HashSet<&str> =
                    current.servers.iter().map(|s| s.id.as_str()).collect();
                let added = data
                    .config
                    .servers
                    .iter()
                    .filter(|s| !old_ids.contains(s.id.as_str()))
                    .count();
                let updated = data.config.servers.len().saturating_sub(added);
                let merged = store
                    .apply_import(&current, data, agentpocket_core::config::ImportMode::Merge)
                    .map_err(|e| e.to_string())?;
                store.save(&merged).map_err(|e| e.to_string())?;
                Ok((added, updated))
            })();

            match result {
                Ok((added, updated)) => {
                    println!(
                        "[mesh] 从 {source} 收到配置：新增 {added} / 更新 {updated} 台服务器"
                    );
                    let body = serde_json::json!({"added": added, "updated": updated});
                    let _ = request.respond(Response::from_string(body.to_string())
                        .with_status_code(StatusCode(200))
                        .with_header(json_header()));
                }
                Err(message) => {
                    let _ = request.respond(Response::from_string(message)
                        .with_status_code(StatusCode(400)));
                }
            }
        }
```

（`tiny_http 0.12` 的 `HeaderField::as_str()` 返回 `&AsciiStr`，再 `.as_str()` 得 `&str`——如果该 API 不匹配，用 `h.field.to_string().eq_ignore_ascii_case("x-agentpocket-source")` 替代，与 `sync.rs:160` 现有写法一致。）

- [ ] **Step 4: 跑测试确认通过**

```bash
cd /home/user/progs/KimiCodeWebApp/daemon && cargo test
```

Expected: 6 个测试全部 PASS。

- [ ] **Step 5: Commit**

```bash
cd /home/user/progs/KimiCodeWebApp
rtk git add daemon
rtk git commit -m "feat(daemon): mesh /config 拉取与自动合并推送（含备份与来源日志）"
```

---

### Task 4: mesh HTTP 客户端 + pull/push 命令

**Files:**
- Create: `daemon/src/client.rs`、`daemon/src/ops.rs`
- Modify: `daemon/src/main.rs`（加 `Pull`/`Push` 子命令）
- Test: `client.rs`、`ops.rs` 内嵌测试

**Interfaces:**
- Consumes: Task 2 的 `mesh::start`（测试对端）、Task 3 的 `/config` 端点
- Produces:
  - `client::request(method, host, port, path, headers: &[(&str, &str)], body: Option<&str>, timeout) -> Result<ClientResponse, ClientError>`，`ClientResponse { status: u16, body: String }`
  - `client::get(host, port, path, headers, timeout)` / `client::post(host, port, path, headers, body, timeout)` 便捷封装
  - `ops::PullMode { Merge, Replace, DryRun }`
  - `ops::run_pull(config_dir: &Path, host: &str, mode: PullMode) -> Result<String, String>`（返回给人看的结果文案）
  - `ops::run_push(config_dir: &Path, host: &str) -> Result<String, String>`
  - Task 5/6 复用 `client::get/post`。

- [ ] **Step 1: 写失败测试**

`client.rs` 测试（解析逻辑 + 对本地 mesh 端点的真实请求）：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const TIMEOUT: Duration = Duration::from_secs(3);

    #[test]
    fn parses_status_line_and_content_length_body() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 5\r\n\r\nhelloextra-bytes-from-keepalive";
        let response = parse_response(raw).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, "hello");
    }

    #[test]
    fn parses_body_without_content_length_up_to_eof() {
        let raw = b"HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\nnope";
        let response = parse_response(raw).unwrap();
        assert_eq!(response.status, 404);
        assert_eq!(response.body, "nope");
    }

    #[test]
    fn rejects_garbage_response() {
        assert!(parse_response(b"not http").is_err());
    }

    #[test]
    fn get_info_against_live_mesh_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let handle = crate::mesh::start(
            crate::mesh::MeshContext {
                config_dir: dir.path().to_path_buf(),
                version: "t",
                hostname: "h",
            },
            0,
        )
        .unwrap();
        let response = get("127.0.0.1", handle.port, "/info", &[], TIMEOUT).unwrap();
        assert_eq!(response.status, 200);
        assert!(response.body.contains("agentpocket"));
        handle.stop();
    }
}
```

`ops.rs` 测试（端到端：本地起端点，pull/push 两个临时目录互拉）：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use agentpocket_core::config::ConfigStore;
    use agentpocket_core::model::{AppConfig, Backend, ServerConfig};

    fn seed(dir: &std::path::Path, id: &str, name: &str) {
        let store = ConfigStore::new(dir.to_path_buf());
        store
            .save(&AppConfig {
                servers: vec![ServerConfig::new(id, name, "100.64.0.2", 3080, "t", Backend::Dsh)],
                ..Default::default()
            })
            .unwrap();
    }

    #[test]
    fn pull_merges_remote_into_local() {
        let remote_dir = tempfile::tempdir().unwrap();
        let local_dir = tempfile::tempdir().unwrap();
        seed(remote_dir.path(), "r1", "Remote");
        seed(local_dir.path(), "l1", "Local");
        let handle = crate::mesh::start(
            crate::mesh::MeshContext {
                config_dir: remote_dir.path().to_path_buf(),
                version: "t",
                hostname: "remote-host",
            },
            0,
        )
        .unwrap();

        let message = run_pull(local_dir.path(), "127.0.0.1", PullMode::Merge).unwrap();

        assert!(message.contains("新增 1"));
        let outcome = ConfigStore::new(local_dir.path().to_path_buf()).load().unwrap();
        assert_eq!(outcome.config.servers.len(), 2);
        handle.stop();
    }

    #[test]
    fn pull_dry_run_does_not_touch_disk() {
        let remote_dir = tempfile::tempdir().unwrap();
        let local_dir = tempfile::tempdir().unwrap();
        seed(remote_dir.path(), "r1", "Remote");
        let handle = crate::mesh::start(
            crate::mesh::MeshContext {
                config_dir: remote_dir.path().to_path_buf(),
                version: "t",
                hostname: "remote-host",
            },
            0,
        )
        .unwrap();

        run_pull(local_dir.path(), "127.0.0.1", PullMode::DryRun).unwrap();

        let outcome = ConfigStore::new(local_dir.path().to_path_buf()).load().unwrap();
        assert!(outcome.config.servers.is_empty());
        handle.stop();
    }

    #[test]
    fn push_reports_peer_merge_counts() {
        let remote_dir = tempfile::tempdir().unwrap();
        let local_dir = tempfile::tempdir().unwrap();
        seed(remote_dir.path(), "r1", "Remote");
        seed(local_dir.path(), "l1", "Local");
        let handle = crate::mesh::start(
            crate::mesh::MeshContext {
                config_dir: remote_dir.path().to_path_buf(),
                version: "t",
                hostname: "remote-host",
            },
            0,
        )
        .unwrap();

        let message = run_push(local_dir.path(), "127.0.0.1").unwrap();

        assert!(message.contains("新增 1"));
        handle.stop();
    }
}
```

（pull/push 测试连 `127.0.0.1`——回环在允许列表里。）

- [ ] **Step 2: 跑测试确认失败**

```bash
cd /home/user/progs/KimiCodeWebApp/daemon && cargo test
```

Expected: 编译失败（`client`/`ops` 模块未建）。

- [ ] **Step 3: 实现 client.rs**

```rust
//! mesh HTTP 客户端：std TcpStream 手写最小 HTTP/1.1，无 TLS 依赖。
//! 只与 tailnet 内 100.x 上的自家端点通信，明文足够，musl 静态零系统依赖。

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

#[derive(Debug)]
pub enum ClientError {
    Io(String),
    Parse(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Io(e) => write!(f, "网络错误：{e}"),
            ClientError::Parse(e) => write!(f, "响应解析失败：{e}"),
        }
    }
}

pub struct ClientResponse {
    pub status: u16,
    pub body: String,
}

pub fn get(
    host: &str,
    port: u16,
    path: &str,
    headers: &[(&str, &str)],
    timeout: Duration,
) -> Result<ClientResponse, ClientError> {
    request("GET", host, port, path, headers, None, timeout)
}

pub fn post(
    host: &str,
    port: u16,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
    timeout: Duration,
) -> Result<ClientResponse, ClientError> {
    request("POST", host, port, path, headers, Some(body), timeout)
}

pub fn request(
    method: &str,
    host: &str,
    port: u16,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
    timeout: Duration,
) -> Result<ClientResponse, ClientError> {
    let addr = resolve(host, port)?;
    let mut stream = TcpStream::connect_timeout(&addr, timeout)
        .map_err(|e| ClientError::Io(e.to_string()))?;
    stream.set_read_timeout(Some(timeout)).map_err(|e| ClientError::Io(e.to_string()))?;
    stream.set_write_timeout(Some(timeout)).map_err(|e| ClientError::Io(e.to_string()))?;

    let mut raw = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n"
    );
    for (key, value) in headers {
        raw.push_str(&format!("{key}: {value}\r\n"));
    }
    if let Some(body) = body {
        raw.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    raw.push_str("\r\n");
    if let Some(body) = body {
        raw.push_str(body);
    }
    stream.write_all(raw.as_bytes()).map_err(|e| ClientError::Io(e.to_string()))?;

    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).map_err(|e| ClientError::Io(e.to_string()))?;
    parse_response(&bytes)
}

fn resolve(host: &str, port: u16) -> Result<SocketAddr, ClientError> {
    (host, port)
        .to_socket_addrs()
        .map_err(|e| ClientError::Io(e.to_string()))?
        .find(|addr| addr.is_ipv4())
        .ok_or_else(|| ClientError::Io(format!("无法解析主机 {host}")))
}

/// 解析响应：状态行 + Content-Length 或 EOF 截断的 body。不支持 chunked（自家端点不用）。
pub fn parse_response(bytes: &[u8]) -> Result<ClientResponse, ClientError> {
    let text = String::from_utf8_lossy(bytes);
    let (head, body) = text
        .split_once("\r\n\r\n")
        .ok_or_else(|| ClientError::Parse("响应缺少头部分隔".to_string()))?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| ClientError::Parse("状态行无效".to_string()))?;

    let content_length = head.lines().skip(1).find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim().eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
    });
    let body = match content_length {
        Some(length) => body
            .chars()
            .take(length)
            .collect::<String>(),
        None => body.to_string(),
    };
    Ok(ClientResponse { status, body })
}
```

- [ ] **Step 4: 实现 ops.rs**

```rust
//! pull/push 命令的业务逻辑。

use std::path::Path;
use std::time::Duration;

use agentpocket_core::config::{ConfigStore, ImportMode};

use crate::client::{self, ClientError};
use crate::mesh::MESH_PORT;

const TIMEOUT: Duration = Duration::from_secs(5);

pub enum PullMode {
    Merge,
    Replace,
    DryRun,
}

pub fn run_pull(config_dir: &Path, host: &str, mode: PullMode) -> Result<String, String> {
    let response = client::get(host, MESH_PORT, "/config", &[], TIMEOUT)
        .map_err(|e| e.to_string())?;
    if response.status != 200 {
        return Err(format!("对方返回 HTTP {}：{}", response.status, response.body));
    }

    let store = ConfigStore::new(config_dir.to_path_buf());
    let data = store.preview_import_text(&response.body).map_err(|e| e.to_string())?;
    let preview = format!(
        "有效 {} 台 / 无效 {} 台：{}",
        data.preview.valid_servers,
        data.preview.invalid_servers,
        data.preview
            .servers
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join("、")
    );
    if matches!(mode, PullMode::DryRun) {
        return Ok(format!("[dry-run] {preview}"));
    }

    let current = store
        .load()
        .map_err(|e| e.to_string())?
        .config;
    let mode_name = match mode {
        PullMode::Replace => ImportMode::Replace,
        _ => ImportMode::Merge,
    };
    let merged = store
        .apply_import(&current, data, mode_name)
        .map_err(|e| e.to_string())?;
    store.save(&merged).map_err(|e| e.to_string())?;
    Ok(match mode {
        PullMode::Replace => format!("已替换：{preview}"),
        _ => format!("已合并：{preview}"),
    })
}

pub fn run_push(config_dir: &Path, host: &str) -> Result<String, String> {
    let store = ConfigStore::new(config_dir.to_path_buf());
    let current = store.load().map_err(|e| e.to_string())?.config;
    let text = store.export_text(&current).map_err(|e| e.to_string())?;

    let hostname = crate::paths::hostname();
    let response = client::post(
        host,
        MESH_PORT,
        "/config",
        &[("X-AgentPocket-Source", hostname.as_str())],
        &text,
        TIMEOUT,
    )
    .map_err(|e: ClientError| e.to_string())?;
    if response.status != 200 {
        return Err(format!("对方返回 HTTP {}：{}", response.status, response.body));
    }
    let counts: serde_json::Value =
        serde_json::from_str(&response.body).map_err(|e| e.to_string())?;
    Ok(format!(
        "对方已合并：新增 {} / 更新 {} 台服务器",
        counts["added"], counts["updated"]
    ))
}
```

`main.rs` 增加 `mod client; mod ops;` 与子命令：

```rust
    /// 从 peer 拉取配置（默认合并）
    Pull {
        host: String,
        /// 用拉取结果替换本地服务器列表
        #[arg(long)]
        replace: bool,
        /// 只打印预览，不落盘
        #[arg(long)]
        dry_run: bool,
    },
    /// 把本地配置推送给 peer
    Push { host: String },
```

match 分支：

```rust
        Command::Pull { host, replace, dry_run } => {
            let mode = if dry_run {
                ops::PullMode::DryRun
            } else if replace {
                ops::PullMode::Replace
            } else {
                ops::PullMode::Merge
            };
            match ops::run_pull(&paths::default_config_dir(), &host, mode) {
                Ok(message) => println!("{message}"),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        Command::Push { host } => {
            match ops::run_push(&paths::default_config_dir(), &host) {
                Ok(message) => println!("{message}"),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
```

- [ ] **Step 5: 跑测试确认通过**

```bash
cd /home/user/progs/KimiCodeWebApp/daemon && cargo test
```

Expected: 全部 PASS（Task 2/3 的 6 个 + 本任务 7 个）。

- [ ] **Step 6: Commit**

```bash
cd /home/user/progs/KimiCodeWebApp
rtk git add daemon
rtk git commit -m "feat(daemon): mesh 客户端与 pull/push 命令"
```

---

### Task 5: tailscale 发现 + peers.json + peers 命令

**Files:**
- Create: `daemon/src/discovery.rs`
- Modify: `daemon/src/main.rs`（加 `Peers` 子命令）
- Test: `discovery.rs` 内嵌测试

**Interfaces:**
- Consumes: Task 4 的 `client::get`、Task 2 的 `mesh::start`/`MESH_PORT`
- Produces:
  - `discovery::MeshPeer { name: String, host: String, version: Option<String>, manual: bool }`
  - `discovery::find_tailscale_binary() -> Option<PathBuf>`
  - `discovery::parse_online_peers(json: &str) -> Vec<(String, String)>`（hostname, ipv4）
  - `discovery::load_manual_peers(config_dir) -> Vec<MeshPeer>`、`save_manual_peers(config_dir, &[MeshPeer])`
  - `discovery::discover(config_dir: &Path, tailscale: Option<&Path>, timeout: Duration) -> Vec<MeshPeer>`（tailscale 设备 + 手动 peer 探测合并，按 host 去重，在线的带版本）

- [ ] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const FIXTURE: &str = r#"{
      "Self": {"HostName": "self-host", "TailscaleIPs": ["100.64.0.1"], "Online": true},
      "Peer": {
        "k1": {"HostName": "peer-a", "TailscaleIPs": ["100.64.0.2", "fd7a:115c:a1e0::1"], "Online": true},
        "k2": {"HostName": "peer-b", "TailscaleIPs": ["100.64.0.3"], "Online": false},
        "k3": {"HostName": "peer-c", "Online": true}
      }
    }"#;

    #[test]
    fn parse_keeps_online_peers_with_ipv4_only() {
        let peers = parse_online_peers(FIXTURE);
        assert_eq!(peers, vec![("peer-a".to_string(), "100.64.0.2".to_string())]);
    }

    #[test]
    fn manual_peers_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let peers = vec![MeshPeer {
            name: "桌面机".to_string(),
            host: "100.64.0.9".to_string(),
            version: None,
            manual: true,
        }];
        save_manual_peers(dir.path(), &peers).unwrap();
        let loaded = load_manual_peers(dir.path());
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].host, "100.64.0.9");
        assert_eq!(loaded[0].name, "桌面机");
    }

    #[test]
    fn manual_peers_file_missing_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_manual_peers(dir.path()).is_empty());
    }

    #[test]
    fn discover_finds_live_mesh_endpoint_via_manual_peer() {
        let dir = tempfile::tempdir().unwrap();
        let handle = crate::mesh::start(
            crate::mesh::MeshContext {
                config_dir: dir.path().to_path_buf(),
                version: "9.9.9",
                hostname: "live-host",
            },
            0,
        )
        .unwrap();
        // 手动 peer 指向本机端点（端口由环境注入，测试里直接探测）。
        let peer = probe_peer("127.0.0.1", handle.port, "live-host", Duration::from_secs(3));
        let peer = peer.expect("probe succeeds");
        assert_eq!(peer.name, "live-host");
        assert_eq!(peer.version.as_deref(), Some("9.9.9"));
        handle.stop();
    }

    #[test]
    fn probe_dead_port_returns_none() {
        // 绑一个端口再关掉，确保连接被拒绝。
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        assert!(probe_peer("127.0.0.1", port, "dead", Duration::from_secs(1)).is_none());
    }

    #[test]
    fn probe_foreign_http_service_returns_none() {
        // 48720 上跑了个"别的" HTTP 服务（应答不是 agentpocket /info）。
        let server = tiny_http::Server::http(("127.0.0.1", 0)).unwrap();
        let port = match server.server_addr() {
            tiny_http::ListenAddr::IP(addr) => addr.port(),
            other => panic!("unexpected listen addr: {other:?}"),
        };
        std::thread::spawn(move || {
            for request in server.incoming_requests() {
                let _ = request.respond(tiny_http::Response::from_string(
                    r#"{"app":"something-else"}"#,
                ));
            }
        });
        assert!(probe_peer("127.0.0.1", port, "foreign", Duration::from_secs(3)).is_none());
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cd /home/user/progs/KimiCodeWebApp/daemon && cargo test
```

Expected: 编译失败（`discovery` 模块未建）。

- [ ] **Step 3: 实现 discovery.rs**

```rust
//! peer 发现：tailscale status --json 解析 + 端口探测 + 手动 peer 合并。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;

use crate::client;
use crate::mesh::MESH_PORT;

#[derive(Clone, Debug, PartialEq)]
pub struct MeshPeer {
    pub name: String,
    pub host: String,
    pub version: Option<String>,
    pub manual: bool,
}

#[derive(Deserialize)]
struct StatusJson {
    #[serde(rename = "Peer")]
    peers: Option<std::collections::HashMap<String, PeerEntry>>,
}

#[derive(Deserialize)]
struct PeerEntry {
    #[serde(rename = "HostName")]
    host_name: String,
    #[serde(rename = "TailscaleIPs")]
    ips: Option<Vec<String>>,
    #[serde(rename = "Online")]
    online: Option<bool>,
}

/// CLI 查找顺序：PATH → 常见安装路径。找不到返回 None（发现退化为仅手动 peer）。
pub fn find_tailscale_binary() -> Option<PathBuf> {
    if Command::new("tailscale")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success())
    {
        return Some(PathBuf::from("tailscale"));
    }
    ["/usr/bin/tailscale", "/usr/local/bin/tailscale"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
}

/// 解析 tailscale status --json：保留在线且带 IPv4 的 peer，返回 (主机名, IPv4)。
pub fn parse_online_peers(json: &str) -> Vec<(String, String)> {
    let parsed: StatusJson = serde_json::from_str(json).unwrap_or(StatusJson { peers: None });
    let Some(peers) = parsed.peers else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for entry in peers.into_values() {
        if !entry.online.unwrap_or(false) {
            continue;
        }
        let Some(ips) = entry.ips else { continue };
        let Some(ipv4) = ips.iter().find(|ip| ip.parse::<std::net::Ipv4Addr>().is_ok()) else {
            continue;
        };
        result.push((entry.host_name, ipv4.clone()));
    }
    result.sort();
    result
}

/// 探测单个 host 的 mesh 端点；/info 应答 app==agentpocket 才算命中。
pub fn probe_peer(host: &str, port: u16, fallback_name: &str, timeout: Duration) -> Option<MeshPeer> {
    let response = client::get(host, port, "/info", &[], timeout).ok()?;
    if response.status != 200 {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(&response.body).ok()?;
    if value.get("app").and_then(|a| a.as_str()) != Some("agentpocket") {
        return None;
    }
    Some(MeshPeer {
        name: value
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or(fallback_name)
            .to_string(),
        host: host.to_string(),
        version: value
            .get("version")
            .and_then(|v| v.as_str())
            .map(String::from),
        manual: false,
    })
}

/// 完整发现：tailscale 在线设备 + 手动 peer，并发探测后按 host 去重。
pub fn discover(config_dir: &Path, tailscale: Option<&Path>, timeout: Duration) -> Vec<MeshPeer> {
    let mut candidates: Vec<(String, String)> = Vec::new(); // (host, 备用名)
    if let Some(binary) = tailscale {
        if let Ok(output) = Command::new(binary).args(["status", "--json"]).output() {
            candidates.extend(
                parse_online_peers(&String::from_utf8_lossy(&output.stdout))
                    .into_iter()
                    .map(|(name, ip)| (ip, name)),
            );
        }
    }
    let manual = load_manual_peers(config_dir);
    candidates.extend(manual.iter().map(|p| (p.host.clone(), p.name.clone())));

    let mut seen: HashSet<String> = HashSet::new();
    let mut peers: Vec<MeshPeer> = Vec::new();
    std::thread::scope(|scope| {
        let handles: Vec<_> = candidates
            .iter()
            .map(|(host, name)| {
                scope.spawn(move || probe_peer(host, MESH_PORT, name, timeout))
            })
            .collect();
        for handle in handles {
            let Some(peer) = handle.join().ok().flatten() else { continue };
            if seen.insert(peer.host.clone()) {
                peers.push(peer);
            }
        }
    });
    peers.sort_by(|a, b| a.name.cmp(&b.name));
    peers
}

#[derive(serde::Deserialize, serde::Serialize)]
struct PeerFile {
    peers: Vec<PeerEntryFile>,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct PeerEntryFile {
    name: String,
    host: String,
}

fn peer_file_path(config_dir: &Path) -> PathBuf {
    config_dir.join("peers.json")
}

pub fn load_manual_peers(config_dir: &Path) -> Vec<MeshPeer> {
    std::fs::read_to_string(peer_file_path(config_dir))
        .ok()
        .and_then(|text| serde_json::from_str::<PeerFile>(&text).ok())
        .map(|file| {
            file.peers
                .into_iter()
                .map(|p| MeshPeer {
                    name: p.name,
                    host: p.host,
                    version: None,
                    manual: true,
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn save_manual_peers(config_dir: &Path, peers: &[MeshPeer]) -> std::io::Result<()> {
    std::fs::create_dir_all(config_dir)?;
    let file = PeerFile {
        peers: peers
            .iter()
            .map(|p| PeerEntryFile {
                name: p.name.clone(),
                host: p.host.clone(),
            })
            .collect(),
    };
    std::fs::write(
        peer_file_path(config_dir),
        serde_json::to_string_pretty(&file).unwrap(),
    )
}
```

`main.rs` 增加 `mod discovery;` 与 `Peers` 子命令（并在文件顶部补 `use std::time::Duration;`，后续 Task 6/7 的分支也用它）：

```rust
    /// 发现并列出 mesh peer
    Peers,
```

```rust
        Command::Peers => {
            let tailscale = discovery::find_tailscale_binary();
            if tailscale.is_none() {
                eprintln!("未找到 tailscale CLI，仅探测手动 peer");
            }
            let peers = discovery::discover(
                &paths::default_config_dir(),
                tailscale.as_deref(),
                Duration::from_secs(3),
            );
            if peers.is_empty() {
                println!("未发现 AgentPocket peer");
            }
            for peer in peers {
                println!(
                    "{}  {}  {}",
                    peer.name,
                    peer.host,
                    peer.version.as_deref().unwrap_or("-")
                );
            }
        }
```

- [ ] **Step 4: 跑测试确认通过**

```bash
cd /home/user/progs/KimiCodeWebApp/daemon && cargo test
```

Expected: 全部 PASS。

- [ ] **Step 5: 手工冒烟（连真实 tailnet 验证发现）**

```bash
cd /home/user/progs/KimiCodeWebApp/daemon && cargo build && ./target/debug/agentpocket peers
```

Expected: 输出可能为空（tailnet 里还没有别的 daemon）但无报错；本机另开终端 `./target/debug/agentpocket serve` 后 `tailscale status` 里本机 peer 是否出现由 tailscale Self 排除逻辑决定（本机不算 peer，属预期）。

- [ ] **Step 6: Commit**

```bash
cd /home/user/progs/KimiCodeWebApp
rtk git add daemon
rtk git commit -m "feat(daemon): tailscale peer 发现与手动 peer 存储"
```

---

### Task 6: status 命令（一次性探测已配置服务器）

**Files:**
- Create: `daemon/src/status.rs`
- Modify: `daemon/src/main.rs`（加 `Status` 子命令）
- Test: `status.rs` 内嵌测试（tiny_http 起 mock kimi/dsh 服务）

**Interfaces:**
- Consumes: Task 4 的 `client::{get, post}`；`agentpocket_core::model::{ServerConfig, Backend}`
- Produces: `status::probe_server(server: &ServerConfig, timeout: Duration) -> ServerProbe`，`ServerProbe { name, backend, online: bool, version: Option<String>, busy: usize, error: Option<String> }`

- [ ] **Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use agentpocket_core::model::{Backend, ServerConfig};
    use std::time::Duration;
    use tiny_http::{Header, Method, Response, Server};

    const TIMEOUT: Duration = Duration::from_secs(3);

    fn spawn_mock(port: u16, kimi: bool) {
        let server = Server::http(("127.0.0.1", port)).unwrap();
        std::thread::spawn(move || {
            for request in server.incoming_requests() {
                let json = Header::from_bytes("Content-Type", "application/json").unwrap();
                match (kimi, request.method(), request.url().split('?').next().unwrap_or("")) {
                    (true, Method::Get, "/api/v1/meta") => {
                        let _ = request.respond(Response::from_string(
                            r#"{"code":0,"data":{"server_version":"0.36.0"}}"#,
                        ).with_header(json));
                    }
                    (true, Method::Get, "/api/v2/sessions") => {
                        let _ = request.respond(Response::from_string(
                            r#"{"code":0,"data":{"items":[
                                {"id":"a","meta":{"title":"t1"},"activity":{"status":"running"}},
                                {"id":"b","meta":{"title":"t2"},"activity":{"status":"idle"}}]}}"#,
                        ).with_header(json));
                    }
                    (false, Method::Post, "/api/session.list") => {
                        let _ = request.respond(Response::from_string(
                            r#"{"result":{"ok":true,"value":{"items":[
                                {"sessionId":"a","running":true},
                                {"sessionId":"b","running":false}]}}}"#,
                        ).with_header(json));
                    }
                    _ => {
                        let _ = request.respond(Response::from_string("{}").with_status_code(404));
                    }
                }
            }
        });
    }

    fn free_port() -> u16 {
        std::net::TcpListener::bind(("127.0.0.1", 0))
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    #[test]
    fn probes_kimi_version_and_busy_count() {
        let port = free_port();
        spawn_mock(port, true);
        let server = ServerConfig::new("k1", "Kimi", "127.0.0.1", port, "tok", Backend::Kimi);

        let probe = probe_server(&server, TIMEOUT);

        assert!(probe.online);
        assert_eq!(probe.version.as_deref(), Some("0.36.0"));
        assert_eq!(probe.busy, 1);
    }

    #[test]
    fn probes_dsh_busy_count() {
        let port = free_port();
        spawn_mock(port, false);
        let server = ServerConfig::new("d1", "Dsh", "127.0.0.1", port, "", Backend::Dsh);

        let probe = probe_server(&server, TIMEOUT);

        assert!(probe.online);
        assert_eq!(probe.busy, 1);
        assert!(probe.version.is_none());
    }

    #[test]
    fn dead_server_reports_offline() {
        let port = free_port();
        let server = ServerConfig::new("x", "Dead", "127.0.0.1", port, "", Backend::Kimi);

        let probe = probe_server(&server, Duration::from_millis(500));

        assert!(!probe.online);
        assert!(probe.error.is_some());
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

```bash
cd /home/user/progs/KimiCodeWebApp/daemon && cargo test
```

Expected: 编译失败（`status` 模块未建）。

- [ ] **Step 3: 实现 status.rs**

```rust
//! 一次性探测已配置服务器（在线/版本/忙碌会话数）。
//! REST 语义与 GUI monitor 一致：kimi /api/v1/meta + /api/v2/sessions；dsh /api/session.list。

use std::time::Duration;

use agentpocket_core::model::{Backend, ServerConfig};

use crate::client;

#[derive(Debug)]
pub struct ServerProbe {
    pub name: String,
    pub backend: Backend,
    pub online: bool,
    pub version: Option<String>,
    pub busy: usize,
    pub error: Option<String>,
}

pub fn probe_server(server: &ServerConfig, timeout: Duration) -> ServerProbe {
    let result = match server.backend {
        Backend::Kimi => probe_kimi(server, timeout),
        Backend::Dsh => probe_dsh(server, timeout),
    };
    match result {
        Ok((version, busy)) => ServerProbe {
            name: server.name.clone(),
            backend: server.backend,
            online: true,
            version,
            busy,
            error: None,
        },
        Err(error) => ServerProbe {
            name: server.name.clone(),
            backend: server.backend,
            online: false,
            version: None,
            busy: 0,
            error: Some(error),
        },
    }
}

fn bearer(server: &ServerConfig) -> Vec<(&'static str, String)> {
    if server.token.is_empty() {
        Vec::new()
    } else {
        vec![("Authorization", format!("Bearer {}", server.token))]
    }
}

fn probe_kimi(
    server: &ServerConfig,
    timeout: Duration,
) -> Result<(Option<String>, usize), String> {
    let auth = bearer(server);
    let auth_refs: Vec<(&str, &str)> = auth.iter().map(|(k, v)| (*k, v.as_str())).collect();

    let meta = client::get(
        &server.host,
        server.port,
        "/api/v1/meta",
        &auth_refs,
        timeout,
    )
    .map_err(|e| e.to_string())?;
    if meta.status != 200 {
        return Err(format!("meta HTTP {}", meta.status));
    }
    let version = serde_json::from_str::<serde_json::Value>(&meta.body)
        .ok()
        .and_then(|v| {
            v.pointer("/data/server_version")
                .and_then(|s| s.as_str())
                .map(String::from)
        })
        .filter(|v| !v.is_empty());

    let sessions = client::get(
        &server.host,
        server.port,
        "/api/v2/sessions?meta.archived=false&page_size=100",
        &auth_refs,
        timeout,
    )
    .map_err(|e| e.to_string())?;
    if sessions.status != 200 {
        return Err(format!("sessions HTTP {}", sessions.status));
    }
    let value: serde_json::Value =
        serde_json::from_str(&sessions.body).map_err(|e| e.to_string())?;
    let items = value
        .pointer("/data/items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let busy = items
        .iter()
        .filter(|item| {
            item.pointer("/activity/status")
                .and_then(|s| s.as_str())
                .map(|s| s != "idle")
                .unwrap_or(false)
        })
        .count();
    Ok((version, busy))
}

fn probe_dsh(server: &ServerConfig, timeout: Duration) -> Result<(Option<String>, usize), String> {
    let body = serde_json::json!({
        "type": "client-request",
        "rpcId": uuid::Uuid::new_v4().to_string(),
        "method": "session.list",
        "payload": {},
    })
    .to_string();
    let response = crate::client::post(
        &server.host,
        server.port,
        "/api/session.list",
        &[],
        &body,
        timeout,
    )
    .map_err(|e| e.to_string())?;
    if response.status != 200 {
        return Err(format!("session.list HTTP {}", response.status));
    }
    let value: serde_json::Value =
        serde_json::from_str(&response.body).map_err(|e| e.to_string())?;
    if !value
        .pointer("/result/ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return Err("session.list 返回 ok=false".to_string());
    }
    let busy = value
        .pointer("/result/value/items")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter(|item| {
                    item.get("running")
                        .and_then(|r| r.as_bool())
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0);
    Ok((None, busy))
}
```

`main.rs` 增加 `mod status;` 与 `Status` 子命令（并发探测全部已配置服务器）：

```rust
    /// 一次性探测已配置服务器状态
    Status,
```

```rust
        Command::Status => {
            let config_dir = paths::default_config_dir();
            let outcome = match agentpocket_core::config::ConfigStore::new(config_dir).load() {
                Ok(outcome) => outcome,
                Err(e) => {
                    eprintln!("读取配置失败：{e}");
                    std::process::exit(1);
                }
            };
            if outcome.config.servers.is_empty() {
                println!("尚未配置任何服务器（可 agentpocket pull <host> 先同步配置）");
            }
            std::thread::scope(|scope| {
                let handles: Vec<_> = outcome
                    .config
                    .servers
                    .iter()
                    .map(|server| {
                        scope.spawn(move || {
                            status::probe_server(server, Duration::from_secs(5))
                        })
                    })
                    .collect();
                for handle in handles {
                    let probe = handle.join().expect("probe thread");
                    if probe.online {
                        println!(
                            "{}  {}  在线 {}  {} 个活跃会话",
                            probe.name,
                            match probe.backend {
                                agentpocket_core::model::Backend::Kimi => "kimi",
                                agentpocket_core::model::Backend::Dsh => "dsh",
                            },
                            probe.version.as_deref().unwrap_or("-"),
                            probe.busy
                        );
                    } else {
                        println!(
                            "{}  {}  离线（{}）",
                            probe.name,
                            match probe.backend {
                                agentpocket_core::model::Backend::Kimi => "kimi",
                                agentpocket_core::model::Backend::Dsh => "dsh",
                            },
                            probe.error.as_deref().unwrap_or("未知错误")
                        );
                    }
                }
            });
        }
```

- [ ] **Step 4: 跑测试确认通过**

```bash
cd /home/user/progs/KimiCodeWebApp/daemon && cargo test
```

Expected: 全部 PASS。

- [ ] **Step 5: 手工冒烟（本机 58627 有真实 kimi 服务）**

```bash
cd /home/user/progs/KimiCodeWebApp/daemon && cargo build
# 先把本机现有配置目录直接用起来（GUI 的配置就在默认路径）：
./target/debug/agentpocket status
```

Expected: 列出本机配置里的服务器及在线状态（127.0.0.1 与 100.x 都在放行范围，kimi 能出版本号）。

- [ ] **Step 6: Commit**

```bash
cd /home/user/progs/KimiCodeWebApp
rtk git add daemon
rtk git commit -m "feat(daemon): status 命令一次性探测服务器在线/版本/忙碌会话"
```

---

### Task 7: 自动更新（update.rs + update 命令）

**Files:**
- Create: `daemon/src/update.rs`
- Modify: `daemon/Cargo.toml`（加 `semver`、`ureq`）、`daemon/src/main.rs`（加 `Update` 子命令）
- Test: `update.rs` 内嵌测试（本地 HTTP mock release）

**Interfaces:**
- Consumes: 无外部任务依赖
- Produces:
  - `update::arch_asset_name() -> String`（`agentpocket-{ARCH}-linux-musl`）
  - `update::fetch_latest(api_base: &str, current: &Version, timeout) -> Result<Option<ReleaseInfo>, UpdateError>`（仅当远端版本严格更新时返回 Some）
  - `update::download_and_replace(url: &str, self_path: &Path, timeout) -> Result<(), UpdateError>`（原子替换）
  - `update::restart_systemd() -> bool`
  - `update::spawn_update_loop()`（serve 内 10s 后首查，之后每 24h）
  - `ReleaseInfo { version: semver::Version, asset_url: String }`

- [ ] **Step 1: Cargo.toml 加依赖**

```toml
semver = "1"
ureq = { version = "2", default-features = false, features = ["tls"] }
```

（ureq 2 + rustls，musl 静态友好；仅本模块使用 HTTPS。）

- [ ] **Step 2: 写失败测试**

```rust
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
            for request in server.incoming_requests() {
                let body = serde_json::json!({
                    "tag_name": "v2.9.0",
                    "assets": [
                        {"name": "agentpocket-x86_64-linux-musl", "browser_download_url": "http://127.0.0.1:0/binary"},
                        {"name": "app.apk", "browser_download_url": "http://127.0.0.1:0/apk"}
                    ]
                })
                .to_string();
                let _ = request.respond(tiny_http::Response::from_string(body));
            }
        });

        let base = format!("http://127.0.0.1:{port}");
        let current = semver::Version::new(2, 8, 0);
        let release = fetch_latest(&base, current, TIMEOUT).unwrap().expect("newer release");
        assert_eq!(release.version, semver::Version::new(2, 9, 0));
        assert!(release.asset_url.contains("musl"));

        // 当前已是 2.9.0 → 不更新。
        let same = semver::Version::new(2, 9, 0);
        assert!(fetch_latest(&base, same, TIMEOUT).unwrap().is_none());
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
```

- [ ] **Step 3: 跑测试确认失败**

```bash
cd /home/user/progs/KimiCodeWebApp/daemon && cargo test
```

Expected: 编译失败（`update` 模块未建）。

- [ ] **Step 4: 实现 update.rs**

```rust
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
```

`main.rs` 增加 `mod update;` 与 `Update` 子命令，并在 `Serve` 分支 `handle.wait()` 之前调用 `update::spawn_update_loop();`：

```rust
    /// 手动检查并更新
    Update,
```

```rust
        Command::Update => {
            match update::check_and_apply("https://api.github.com", Duration::from_secs(60)) {
                Ok(message) => println!("{message}"),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
```

- [ ] **Step 5: 跑测试确认通过**

```bash
cd /home/user/progs/KimiCodeWebApp/daemon && cargo test
```

Expected: 全部 PASS。

- [ ] **Step 6: Commit**

```bash
cd /home/user/progs/KimiCodeWebApp
rtk git add daemon
rtk git commit -m "feat(daemon): GitHub releases 自动更新（semver 防降级 + 原子替换 + systemd 重启）"
```

---

### Task 8: 端口占用路径 + serve 冒烟

**Files:**
- Modify: `daemon/src/mesh.rs`（补端口占用测试与错误信息）

**Interfaces:**
- Consumes: Task 2 的 `mesh::start`
- Produces: `mesh::start` 对占用端口返回 `MeshError::Bind`，`serve` 命令以非零码退出（已在 Task 2 实现，本任务补测试覆盖）。

- [ ] **Step 1: 写失败测试（追加到 mesh.rs tests）**

```rust
    #[test]
    fn start_on_occupied_port_returns_bind_error() {
        let listener = std::net::TcpListener::bind(("0.0.0.0", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let dir = tempfile::tempdir().unwrap();

        let error = start(ctx(dir.path()), port).unwrap_err();

        assert!(matches!(error, MeshError::Bind(_)));
        assert!(error.to_string().contains("端口被占用"));
        drop(listener);
    }
```

注意：`tiny_http` 绑定 `0.0.0.0` 与已有 `TcpListener` 绑 `0.0.0.0` 同端口会冲突；若 Linux 上出现 `SO_REUSEADDR` 导致绑定成功的意外，改为先 `let server = Server::http(("0.0.0.0", port))` 占住端口再测（tiny_http 不设 SO_REUSEADDR，正常应当冲突）。

- [ ] **Step 2: 跑测试**

```bash
cd /home/user/progs/KimiCodeWebApp/daemon && cargo test
```

Expected: 全部 PASS（含新测试）。

- [ ] **Step 3: serve 冒烟（真实前台运行 + 手动 pull/push 回环）**

```bash
cd /home/user/progs/KimiCodeWebApp/daemon && cargo build
XDG_DATA_HOME=$(mktemp -d) ./target/debug/agentpocket serve &
sleep 1
# /info 应答（用 curl）
curl -s http://127.0.0.1:48720/info
kill %1
```

Expected: `/info` 返回 `{"app":"agentpocket",...}`；杀进程后端口释放。

- [ ] **Step 4: Commit**

```bash
cd /home/user/progs/KimiCodeWebApp
rtk git add daemon
rtk git commit -m "test(daemon): mesh 端口占用错误路径"
```

---

### Task 9: install.sh + musl 构建 + README + .gitignore

**Files:**
- Create: `scripts/install.sh`（可执行）、`scripts/build-daemon.sh`（可执行）
- Modify: `README.md`（新增 daemon 章节）、`.gitignore`（加 `dist/`）
- Test: `sh -n` 语法检查 + 本机以 `--uninstall`/临时目录方式演练（不真装本机服务；安装演练仅在用户确认后执行）

**Interfaces:**
- Consumes: Task 7 的资产命名 `agentpocket-<arch>-linux-musl`
- Produces: 一键安装入口 `curl -fsSL https://raw.githubusercontent.com/npu-chenlin/AgentPocket/main/scripts/install.sh | sudo bash`；release 资产由 `scripts/build-daemon.sh` 产出。

- [ ] **Step 1: 写 scripts/install.sh**

```sh
#!/bin/sh
# AgentPocket mesh 守护进程一键安装：下载二进制 + systemd 服务 + 自启动。
# 用法：curl -fsSL https://raw.githubusercontent.com/npu-chenlin/AgentPocket/main/scripts/install.sh | sudo bash
# 卸载：同命令加 --uninstall
set -eu

REPO="npu-chenlin/AgentPocket"
BIN_PATH="/usr/local/bin/agentpocket"
SERVICE_PATH="/etc/systemd/system/agentpocket.service"

if [ "${1:-}" = "--uninstall" ]; then
    systemctl stop agentpocket 2>/dev/null || true
    systemctl disable agentpocket 2>/dev/null || true
    rm -f "$SERVICE_PATH" "$BIN_PATH"
    systemctl daemon-reload
    echo "已卸载 agentpocket（配置目录 ~/.local/share/AgentPocket 保留）"
    exit 0
fi

if [ "$(id -u)" -ne 0 ]; then
    echo "请用 sudo 运行（或 curl … | sudo bash）" >&2
    exit 1
fi

case "$(uname -m)" in
    x86_64) ARCH="x86_64" ;;
    aarch64|arm64) ARCH="aarch64" ;;
    *) echo "不支持的架构：$(uname -m)" >&2; exit 1 ;;
esac
ASSET="agentpocket-${ARCH}-linux-musl"

LATEST_URL="https://api.github.com/repos/${REPO}/releases/latest"
DOWNLOAD_URL="$(curl -fsSL "$LATEST_URL" | tr ',' '\n' | grep -o "https://[^\"]*${ASSET}" | head -n 1)"
if [ -z "$DOWNLOAD_URL" ]; then
    echo "未在最新 release 找到资产 ${ASSET}，请到 https://github.com/${REPO}/releases 手动下载" >&2
    exit 1
fi

echo "下载 ${DOWNLOAD_URL} …"
curl -fsSL -o "$BIN_PATH" "$DOWNLOAD_URL"
chmod 755 "$BIN_PATH"

RUN_USER="${SUDO_USER:-root}"
HOME_DIR="$(getent passwd "$RUN_USER" | cut -d: -f6)"
[ -n "$HOME_DIR" ] || { HOME_DIR="/root"; RUN_USER="root"; }

cat > "$SERVICE_PATH" <<EOF
[Unit]
Description=AgentPocket mesh daemon
After=network-online.target
Wants=network-online.target

[Service]
User=${RUN_USER}
Environment=HOME=${HOME_DIR}
ExecStart=${BIN_PATH} serve
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl enable --now agentpocket

echo "安装完成："
echo "  状态    systemctl status agentpocket"
echo "  日志    journalctl -u agentpocket -f"
echo "  发现    sudo -u ${RUN_USER} ${BIN_PATH} peers"
echo "  同步    sudo -u ${RUN_USER} ${BIN_PATH} pull <桌面机IP或MagicDNS名>"
```

- [ ] **Step 2: 写 scripts/build-daemon.sh**

```sh
#!/bin/sh
# 构建 musl 静态二进制并复制到 dist/（资产名与 update.rs 的 arch_asset_name 一致）。
set -eu
cd "$(dirname "$0")/.."

export CARGO_REGISTRIES_CRATES_IO_INDEX='sparse+https://rsproxy.cn/index/'
export CARGO_REGISTRIES_CRATES_IO_PROTOCOL=sparse

ARCH="${1:-x86_64}"
TARGET="${ARCH}-unknown-linux-musl"

rustup target add "$TARGET"
cargo build --release --target "$TARGET" --manifest-path daemon/Cargo.toml

mkdir -p dist
cp "daemon/target/${TARGET}/release/agentpocket" "dist/agentpocket-${ARCH}-linux-musl"
echo "产物：dist/agentpocket-${ARCH}-linux-musl"
```

（`--manifest-path` 而非 `-p`：仓库根没有 workspace，daemon 有自己的 target 目录，与 desktop 现状一致。）

- [ ] **Step 3: 语法检查与权限**

```bash
cd /home/user/progs/KimiCodeWebApp
sh -n scripts/install.sh && sh -n scripts/build-daemon.sh
chmod +x scripts/install.sh scripts/build-daemon.sh
```

Expected: 无输出（语法 OK）。

- [ ] **Step 4: musl 构建验证（需要 musl-tools；若未安装：`printf 'cvte\n' | sudo -S apt-get install -y musl-tools`，用户已授权）**

```bash
cd /home/user/progs/KimiCodeWebApp && ./scripts/build-daemon.sh
file dist/agentpocket-x86_64-linux-musl
```

Expected: `statically linked`（或 `pie executable … static-pie`）；`dist/agentpocket-x86_64-linux-musl --version` 输出 `2.8.0`。

- [ ] **Step 5: README 增加 daemon 章节（用户视角，追加在现有 README 的桌面端章节之后）**

````markdown
## 服务器端（mesh 守护进程）

在没有显示器的服务器上一键安装 AgentPocket 守护进程，让配置在所有机器间流动：

```bash
curl -fsSL https://raw.githubusercontent.com/npu-chenlin/AgentPocket/main/scripts/install.sh | sudo bash
```

安装后自动启动并开机自启。常用命令（`sudo agentpocket …`）：

| 命令 | 作用 |
|---|---|
| `agentpocket peers` | 发现 tailnet 内的 AgentPocket 节点 |
| `agentpocket pull <IP或MagicDNS名>` | 从某节点拉取配置（`--replace` 替换 / `--dry-run` 预览） |
| `agentpocket push <IP或MagicDNS名>` | 把本机配置推送给某节点（对方自动合并） |
| `agentpocket status` | 查看本机所配服务器的在线/版本/活跃会话 |
| `agentpocket update` | 手动检查更新（日常每 24 小时自动检查并自更新） |

守护进程与桌面端共用 `~/.local/share/AgentPocket/config.json`，同机安装互不冲突。

mesh 端点仅在你的 Tailscale 网络内可达（Tailscale 网段之外一律拒绝）；节点间无鉴权，信任边界即你的 tailnet。
````

（按 README 现有格式微调标题层级；只写用户视角，不写实现细节。）

- [ ] **Step 6: .gitignore 加 `dist/`，跑全量测试**

```bash
cd /home/user/progs/KimiCodeWebApp
echo "dist/" >> .gitignore
cd daemon && cargo test && cd ../core && cargo test && cd ../desktop/src-tauri && cargo test --offline && cd ../..
```

Expected: daemon、core、GUI 三处测试全部 PASS。

- [ ] **Step 7: Commit**

```bash
cd /home/user/progs/KimiCodeWebApp
rtk git add scripts README.md .gitignore
rtk git commit -m "feat(daemon): 一键安装脚本、musl 构建脚本与 README 章节"
```

---

## 任务依赖

Task 1（core）→ Task 2（骨架/mesh）→ Task 3（/config）→ Task 4（client/pull/push）→ Task 5（discovery，依赖 4 的 client）→ Task 6（status，依赖 4 的 client）→ Task 7（update，独立可并行）→ Task 8（占用路径冒烟）→ Task 9（安装/构建/文档）。
Task 7 只依赖 Task 2 的 crate 存在，可与 Task 5/6 并行。

## 验收清单（对照 spec）

- [ ] `agentpocket serve` 前台守护：48720 + tailnet 围栏 + 自动更新循环（spec §4/§7）
- [ ] `/info` `/config` GET/POST 端点与自动合并、备份、来源标注（spec §4.2）
- [ ] `peers`/`pull`/`push`/`status`/`update`/`version` 命令（spec §6）
- [ ] tailscale status 解析 + 探测 + peers.json 手动 peer（spec §5）
- [ ] 自动更新：semver 防降级、原子替换、systemd 重启（spec §7）
- [ ] install.sh：下载 + unit（SUDO_USER + HOME）+ enable --now + --uninstall（spec §8）
- [ ] musl 静态资产命名 `agentpocket-<arch>-linux-musl`（spec §12）
- [ ] GUI 83 测试全绿、Android 无改动（Global Constraints）
