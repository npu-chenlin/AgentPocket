# AgentPocket Desktop Tray Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在现有 AgentPocket 仓库的 `desktop/` 下构建 Linux 优先的 Tauri 2 托盘应用，管理并监听多台 Kimi/dsh 服务器，发送四类高价值通知，并通过系统浏览器一键打开服务器。

**Architecture:** 设置界面使用 Vite + TypeScript 原生 DOM；Rust 后端拥有配置、Kimi/dsh 协议 monitor、通知去重、系统托盘和生命周期。每台服务器一个可取消的异步 monitor，原始后端帧归一化为统一 `ServerStatus` 和 `AgentEvent`，再由托盘和通知消费。

**Tech Stack:** Tauri 2、Rust stable、Tokio、Reqwest、tokio-tungstenite、Serde、Vite、TypeScript、Vitest；Tauri 官方 notification/autostart/opener/dialog 插件。

## Global Constraints

- 首发平台为 Linux；必须产出并验证 AppImage 与 `.deb`。
- 使用 Tauri 2 + Vite + TypeScript 原生 DOM + Rust，不引 React/Vue/Svelte。
- 代码位于现有仓库 `desktop/`；Android `app/` 逻辑不得回归。
- 首版只通知 Completed、Failed、ApprovalRequired、QuestionRequired；不得通知工具调用、增量文本或普通工具失败。
- 不内嵌 Kimi/dsh 页面；只允许系统浏览器打开由已保存服务器配置构造的 `http/https` URL。
- 配置兼容 Android 的 `id/name/host/port/token/backend` 字段；首版只做手动导入/导出，不做云同步。
- token 不得出现在日志、列表状态或前端常规快照中；Unix 配置文件权限为 `0600`。
- 单台服务器失败不得影响其他 monitor；退出时最多等待 3 秒清理。
- 实施中的每次 `git commit` 都属于独立 git mutation，执行者必须在实际执行时向用户单独确认；本计划中的 commit 命令仅给出建议，不代表预授权。
- Rust 当前未安装；安装 Rust 或系统 Tauri Linux 依赖会写入工作目录外/可能需要 sudo，执行前必须获得用户确认。

---

## File Map

### Frontend

- `desktop/package.json` — npm scripts 与 JS 依赖。
- `desktop/vite.config.ts`、`desktop/tsconfig.json`、`desktop/index.html` — Vite/Tauri 入口。
- `desktop/src/model.ts` — 前端 DTO 与 command 名称。
- `desktop/src/validation.ts` — 可单测的表单校验。
- `desktop/src/server-list.ts` — 服务器列表、编辑表单和设置渲染。
- `desktop/src/import-export.ts` — 文件选择、导入预览、合并/替换、导出警告。
- `desktop/src/main.ts` — invoke/event 接线与刷新。
- `desktop/src/styles.css` — 轻量设置 UI。
- `desktop/src/*.test.ts` — Vitest 单元测试。

### Rust

- `desktop/src-tauri/Cargo.toml` — Tauri 与运行时依赖。
- `desktop/src-tauri/tauri.conf.json` — 窗口、bundle、托盘资源和 Linux target。
- `desktop/src-tauri/capabilities/default.json` — 最小前端权限。
- `desktop/src-tauri/src/main.rs` — 桌面二进制入口。
- `desktop/src-tauri/src/lib.rs` — Builder、managed state、commands、生命周期。
- `desktop/src-tauri/src/model.rs` — 配置、脱敏视图、状态、事件 DTO。
- `desktop/src-tauri/src/config.rs` — 校验、schema 兼容、原子持久化、备份、导入合并。
- `desktop/src-tauri/src/protocol/mod.rs` — 通用协议解析结果。
- `desktop/src-tauri/src/protocol/dsh.rs`、`kimi.rs` — 脱敏帧到统一状态/事件的纯函数。
- `desktop/src-tauri/src/monitor/mod.rs` — manager、task 生命周期、重连退避。
- `desktop/src-tauri/src/monitor/dsh.rs`、`kimi.rs` — HTTP/WebSocket 连接。
- `desktop/src-tauri/src/notification.rs` — TTL 去重和四类系统通知。
- `desktop/src-tauri/src/opener.rs` — 安全 URL 构造与打开。
- `desktop/src-tauri/src/tray.rs` — 动态托盘菜单、窗口 hide/show、显式退出。
- `desktop/src-tauri/src/commands.rs` — 前端白名单 commands、导入预览缓存。
- `desktop/src-tauri/tests/fixtures/*.json` — Kimi/dsh 脱敏事件 fixture。

### Repository

- `.gitignore` — 忽略 `desktop/node_modules/`、`desktop/dist/`、`desktop/src-tauri/target/`。
- `README.md` — 桌面端开发/构建/使用说明（实现完成后更新）。

---

### Task 1: Bootstrap Tauri 2 + Vanilla TypeScript

**Files:**
- Create: `desktop/package.json`
- Create: `desktop/package-lock.json`
- Create: `desktop/vite.config.ts`
- Create: `desktop/tsconfig.json`
- Create: `desktop/index.html`
- Create: `desktop/src/main.ts`
- Create: `desktop/src/styles.css`
- Create: `desktop/src-tauri/Cargo.toml`
- Create: `desktop/src-tauri/build.rs`
- Create: `desktop/src-tauri/tauri.conf.json`
- Create: `desktop/src-tauri/capabilities/default.json`
- Create: `desktop/src-tauri/src/main.rs`
- Create: `desktop/src-tauri/src/lib.rs`
- Modify: `.gitignore`

**Interfaces:**
- Produces: `desktop` npm workspace with `npm run dev`, `npm run test`, `npm run build`, `npm run tauri`.
- Produces: `agentpocket_desktop_lib::run()` Tauri entrypoint.

- [ ] **Step 1: Confirm/install prerequisites**

Run:

```bash
export PATH="$HOME/.nvm/versions/node/v22.22.0/bin:$PATH"
node --version
npm --version
rustc --version
cargo --version
pkg-config --modversion webkit2gtk-4.1
pkg-config --modversion appindicator3-0.1 || pkg-config --modversion ayatana-appindicator3-0.1
```

Expected: Node `v22.22.0`; Rust stable and Linux WebKit/AppIndicator packages available. If Rust/system packages are absent, stop and ask approval before installing outside the repository. For Ubuntu 22.04 the expected user-run system package command is:

```bash
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

Rust installation requires separate approval before running rustup.

- [ ] **Step 2: Create the minimal package manifests**

`desktop/package.json`:

```json
{
  "name": "agentpocket-desktop",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc --noEmit && vite build",
    "test": "vitest run",
    "test:watch": "vitest",
    "tauri": "tauri"
  },
  "dependencies": {
    "@tauri-apps/api": "^2.0.0",
    "@tauri-apps/plugin-dialog": "^2.0.0"
  },
  "devDependencies": {
    "@tauri-apps/cli": "^2.0.0",
    "typescript": "^5.7.0",
    "vite": "^7.0.0",
    "vitest": "^3.0.0"
  }
}
```

`desktop/src-tauri/Cargo.toml` must declare:

```toml
[package]
name = "agentpocket-desktop"
version = "0.1.0"
edition = "2021"

[lib]
name = "agentpocket_desktop_lib"
crate-type = ["staticlib", "cdylib", "rlib"]

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
tauri = { version = "2", features = ["tray-icon"] }
tauri-plugin-autostart = "2"
tauri-plugin-dialog = "2"
tauri-plugin-notification = "2"
tauri-plugin-opener = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "sync", "time"] }
```

- [ ] **Step 3: Create minimal Tauri/Vite entrypoints**

`main.rs`:

```rust
fn main() {
    agentpocket_desktop_lib::run();
}
```

`lib.rs`:

```rust
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running AgentPocket Desktop");
}
```

`src/main.ts` renders `<h1>AgentPocket</h1>` and imports `styles.css`. Configure one `main` window at 760×620, `visible: true` in development, title `AgentPocket`.

- [ ] **Step 4: Install locked JS dependencies**

Run:

```bash
cd /home/user/progs/KimiCodeWebApp/desktop
export PATH="$HOME/.nvm/versions/node/v22.22.0/bin:$PATH"
npm install
```

Expected: `package-lock.json` created; no unresolved peer dependency.

- [ ] **Step 5: Run baseline checks**

Run:

```bash
cd /home/user/progs/KimiCodeWebApp/desktop
npm run build
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: both exit 0.

- [ ] **Step 6: Update `.gitignore`**

Append exactly:

```gitignore
/desktop/node_modules/
/desktop/dist/
/desktop/src-tauri/target/
```

- [ ] **Step 7: Commit after explicit user confirmation**

```bash
git add .gitignore desktop
git commit -m "feat(desktop): bootstrap Tauri tray application"
```

---

### Task 2: Configuration Model, Validation, Atomic Storage, Import/Export Core

**Files:**
- Create: `desktop/src-tauri/src/model.rs`
- Create: `desktop/src-tauri/src/config.rs`
- Modify: `desktop/src-tauri/src/lib.rs`
- Modify: `desktop/src-tauri/Cargo.toml`
- Test: inline `#[cfg(test)]` modules in `model.rs` and `config.rs`

**Interfaces:**
- Produces: `Backend::{Kimi,Dsh}`; `ServerConfig`; `DesktopSettings`; `AppConfig`; `ServerSummary`.
- Produces: `ConfigStore::{load,save,preview_import,apply_import,export}`.
- Produces: `ImportMode::{Merge,Replace}`, `ImportPreview`, `ExportFormat::{Full,Android}`.

- [ ] **Step 1: Write failing model validation tests**

Add tests for these exact cases:

```rust
#[test]
fn validates_server_fields() {
    let ok = ServerConfig::new("id", "Work", "100.64.0.2", 3080, "", Backend::Dsh);
    assert!(ok.validate().is_ok());
    assert!(ServerConfig::new("id", "Work", "http://host", 3080, "", Backend::Dsh).validate().is_err());
    assert!(ServerConfig::new("id", "Work", "host:3080", 3080, "", Backend::Dsh).validate().is_err());
    assert!(ServerConfig::new("id", "Work", "host", 0, "", Backend::Dsh).validate().is_err());
}

#[test]
fn summary_never_exposes_token() {
    let server = ServerConfig::new("id", "Work", "host", 3080, "secret", Backend::Dsh);
    let json = serde_json::to_string(&ServerSummary::from(&server)).unwrap();
    assert!(!json.contains("secret"));
    assert!(!json.contains("token"));
}
```

- [ ] **Step 2: Run tests and confirm failure**

Run:

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml model::tests -- --nocapture
```

Expected: FAIL because `model` types do not exist.

- [ ] **Step 3: Implement exact config DTOs**

Use `#[serde(rename_all = "camelCase")]` for JS-facing/config fields. Defaults:

```rust
impl Default for DesktopSettings {
    fn default() -> Self {
        Self { start_hidden: true, autostart: false, notifications: true }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self { schema: 1, active_id: None, servers: Vec::new(), settings: DesktopSettings::default() }
    }
}
```

`ServerConfig::base_url()` must return a parsed `url::Url`; validation rejects schemes/path/whitespace/colon in host and port 0. Add `url = "2"`, `uuid = { version = "1", features = ["v4"] }`, `chrono = { version = "0.4", features = ["serde"] }`.

- [ ] **Step 4: Write failing storage/import tests**

Use `tempfile::tempdir()` and cover:

```rust
#[test]
fn loads_android_array_and_assigns_missing_ids() { /* array with name/host/port/token/backend */ }

#[test]
fn merge_updates_matching_id_and_appends_new_server() { /* assert stable activeId */ }

#[test]
fn replace_rejects_zero_valid_servers() { /* malformed-only import */ }

#[test]
fn save_round_trips_and_creates_mode_0600_on_unix() { /* metadata.permissions().mode() & 0o777 */ }

#[test]
fn corrupted_primary_recovers_latest_backup_without_overwriting_primary() { /* assert bad bytes remain */ }

#[test]
fn backup_rotation_keeps_five_files() { /* perform six saves/imports */ }
```

- [ ] **Step 5: Run storage tests and confirm failure**

Run:

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml config::tests -- --nocapture
```

Expected: FAIL because `ConfigStore` does not exist.

- [ ] **Step 6: Implement `ConfigStore`**

Required signatures:

```rust
pub struct ConfigStore { app_dir: PathBuf, config_path: PathBuf }

impl ConfigStore {
    pub fn new(app_dir: PathBuf) -> Self;
    pub fn load(&self) -> Result<LoadOutcome, ConfigError>;
    pub fn save(&self, config: &AppConfig) -> Result<(), ConfigError>;
    pub fn preview_import(&self, path: &Path) -> Result<ImportPreviewData, ConfigError>;
    pub fn apply_import(&self, current: &AppConfig, data: ImportPreviewData, mode: ImportMode) -> Result<AppConfig, ConfigError>;
    pub fn export(&self, config: &AppConfig, path: &Path, format: ExportFormat) -> Result<(), ConfigError>;
}
```

Atomic save sequence: create app dir → serialize pretty JSON → write `config.json.tmp` → `sync_all()` → Unix `set_permissions(0o600)` → rename to `config.json`. Before import application, copy primary to `backups/config-YYYYMMDD-HHMMSS.json`, sort by filename and remove oldest beyond five. Do not overwrite corrupted primary during recovery.

- [ ] **Step 7: Run all config/model tests**

Run:

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml model::tests config::tests -- --nocapture
```

Expected: PASS.

- [ ] **Step 8: Commit after explicit user confirmation**

```bash
git add desktop/src-tauri
git commit -m "feat(desktop): add compatible atomic server configuration"
```

---

### Task 3: Pure Kimi/dsh Protocol Normalization

**Files:**
- Create: `desktop/src-tauri/src/protocol/mod.rs`
- Create: `desktop/src-tauri/src/protocol/dsh.rs`
- Create: `desktop/src-tauri/src/protocol/kimi.rs`
- Create: `desktop/src-tauri/tests/fixtures/dsh-session-list.json`
- Create: `desktop/src-tauri/tests/fixtures/dsh-events.json`
- Create: `desktop/src-tauri/tests/fixtures/kimi-events.json`
- Modify: `desktop/src-tauri/src/model.rs`
- Modify: `desktop/src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `ServerStatus`, `AgentEvent`, `AgentEventKind`, `ServerConfig` from Task 2.
- Produces: `ProtocolState`; `dsh::parse_session_list`, `dsh::parse_frame`, `kimi::parse_frame`.

- [ ] **Step 1: Add unified event/status types**

Define:

```rust
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
    pub connected: bool,
    pub active_count: u32,
    pub last_checked_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AgentEventKind { Completed, Failed, ApprovalRequired, QuestionRequired }

#[derive(Clone, Debug, PartialEq)]
pub struct AgentEvent {
    pub server_id: String,
    pub session_id: Option<String>,
    pub session_title: Option<String>,
    pub kind: AgentEventKind,
    pub event_key: String,
    pub body: Option<String>,
    pub occurred_at: DateTime<Utc>,
}
```

`ProtocolState` owns `titles: HashMap<String,String>`, `busy: HashSet<String>`, and `baseline_complete: bool`.

- [ ] **Step 2: Write dsh fixture tests first**

Fixtures must be copied from the known Android protocol shapes but replace tokens, hostnames, titles and content with neutral values. Tests assert:

```rust
let mut state = ProtocolState::default();
parse_session_list(include_str!("../../tests/fixtures/dsh-session-list.json"), &mut state).unwrap();
assert_eq!(state.busy.len(), 1);
assert!(state.baseline_complete);

let events = parse_lines(include_str!("../../tests/fixtures/dsh-events.json"));
assert_eq!(collect_kinds(events, &mut state), vec![
    AgentEventKind::ApprovalRequired,
    AgentEventKind::QuestionRequired,
    AgentEventKind::Completed,
    AgentEventKind::Failed,
]);
```

Include ignored frames for `tool/call`, `assistant/chunk`, `session/queue`, unknown method, and non-`server-request`; assert they yield no `AgentEvent`.

- [ ] **Step 3: Run dsh tests and confirm failure**

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml protocol::dsh -- --nocapture
```

Expected: FAIL because parser is missing.

- [ ] **Step 4: Implement dsh pure parser**

Signatures:

```rust
pub fn parse_session_list(body: &str, state: &mut ProtocolState) -> Result<(), ProtocolError>;
pub fn parse_frame(server_id: &str, text: &str, now: DateTime<Utc>, state: &mut ProtocolState) -> Result<Vec<AgentEvent>, ProtocolError>;
```

Rules must match the approved spec and existing Android `DshServerMonitor`: `turn/start`, `turn/end`, title projection, approval/question; only `turn/end.reason.kind` completed produces Completed, while error/aborted/blocked/max-tokens/interrupted produces Failed. `event_key` uses approvalId/question rpcId/`turn-end:{seq}`.

- [ ] **Step 5: Write Kimi fixture tests first**

Cover `event.session.status_changed`, `event.session.work_changed`, `event.session.created`, `prompt.completed`, `prompt.aborted`, and duplicate frames. Verify active count never goes negative and title cache updates.

- [ ] **Step 6: Implement Kimi pure parser**

```rust
pub fn parse_frame(server_id: &str, text: &str, now: DateTime<Utc>, state: &mut ProtocolState) -> Result<Vec<AgentEvent>, ProtocolError>;
```

Port semantics from `app/src/main/java/com/local/kimiapp/KimiServerMonitor.java`, not guessed field names. Unknown events return `Ok(vec![])`.

- [ ] **Step 7: Run protocol tests**

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml protocol -- --nocapture
```

Expected: PASS; no fixture contains real credentials or message content.

- [ ] **Step 8: Commit after explicit user confirmation**

```bash
git add desktop/src-tauri/src/protocol desktop/src-tauri/src/model.rs desktop/src-tauri/tests/fixtures
git commit -m "feat(desktop): normalize Kimi and dsh monitor events"
```

---

### Task 4: Monitor Runtime, Per-Server Isolation, Detection, Reconnection

**Files:**
- Create: `desktop/src-tauri/src/monitor/mod.rs`
- Create: `desktop/src-tauri/src/monitor/dsh.rs`
- Create: `desktop/src-tauri/src/monitor/kimi.rs`
- Modify: `desktop/src-tauri/src/lib.rs`
- Modify: `desktop/src-tauri/Cargo.toml`
- Test: inline modules plus mock HTTP/WebSocket integration tests

**Interfaces:**
- Consumes: `ServerConfig`, `ServerStatus`, `AgentEvent`, protocol parsers.
- Produces: `MonitorManager::{sync_servers,reconnect_all,shutdown}`; `probe_backend`.
- Emits: `MonitorUpdate::{Status,Event}` over `mpsc`.

- [ ] **Step 1: Write retry/isolation tests**

Tests:

```rust
#[test]
fn backoff_sequence_caps_and_resets() {
    let mut b = ReconnectBackoff::default();
    assert_eq!([b.next(), b.next(), b.next(), b.next(), b.next(), b.next()],
               [1, 2, 4, 8, 16, 30].map(Duration::from_secs));
    assert_eq!(b.next(), Duration::from_secs(30));
    b.reset();
    assert_eq!(b.next(), Duration::from_secs(1));
}

#[tokio::test]
async fn removing_one_server_cancels_only_its_task() { /* fake monitor factory + cancellation probes */ }

#[tokio::test]
async fn changing_one_server_restarts_only_that_server() { /* compare generations */ }
```

- [ ] **Step 2: Run tests and confirm failure**

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml monitor::tests -- --nocapture
```

Expected: FAIL.

- [ ] **Step 3: Implement `MonitorManager` lifecycle**

Use:

```rust
pub enum MonitorUpdate {
    Status { server_id: String, status: ServerStatus },
    Event(AgentEvent),
}

pub struct MonitorManager {
    tasks: HashMap<String, MonitorTask>,
    update_tx: mpsc::Sender<MonitorUpdate>,
}

impl MonitorManager {
    pub async fn sync_servers(&mut self, servers: &[ServerConfig]);
    pub async fn reconnect_all(&mut self);
    pub async fn shutdown(&mut self, timeout: Duration);
}
```

A `MonitorTask` holds a config fingerprint, `CancellationToken`, and `JoinHandle`. Add `tokio-util = { version = "0.7", features = ["rt"] }`.

- [ ] **Step 4: Write backend detection tests with a local mock server**

Use a minimal Tokio TCP HTTP mock (no production dependency). Assert:

- dsh probe receives `POST /api/agentPreset.list` with client-request envelope and recognizes `server-response`.
- Kimi fallback requests `GET /api/v2/sessions` and recognizes non-404 JSON.
- both fail returns a structured error containing both probe reasons while preserving user selection in command layer.

- [ ] **Step 5: Implement HTTP/WebSocket connection loops**

Add dependencies:

```toml
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
tokio-tungstenite = { version = "0.27", features = ["rustls-tls-native-roots"] }
futures-util = "0.3"
http = "1"
```

Required behavior:

- Dsh: POST `session.list`, then `connect_async` to `/api/events.mux`, update `connected=true`, parse text frames, ignore binary, respond to ping per library, reconnect on close/error.
- Kimi: fetch baseline per existing Android behavior; WebSocket request includes `Sec-WebSocket-Protocol` only when token non-empty; preserve server-selected subprotocol handling.
- Never log request headers, token, full frames or message bodies.
- Race sleep against cancellation with `tokio::select!`.

- [ ] **Step 6: Run monitor tests**

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml monitor -- --nocapture
```

Expected: PASS; tests finish without orphan tasks.

- [ ] **Step 7: Commit after explicit user confirmation**

```bash
git add desktop/src-tauri
git commit -m "feat(desktop): add isolated Kimi and dsh monitors"
```

---

### Task 5: Notification Deduplication and Safe Browser Opening

**Files:**
- Create: `desktop/src-tauri/src/notification.rs`
- Create: `desktop/src-tauri/src/opener.rs`
- Modify: `desktop/src-tauri/src/lib.rs`
- Modify: `desktop/src-tauri/Cargo.toml`
- Test: inline unit tests

**Interfaces:**
- Consumes: `AgentEvent`, `ServerConfig`, settings.
- Produces: `NotificationCoordinator::handle_event`; `build_server_url`; `open_saved_server`.

- [ ] **Step 1: Write notification policy tests**

```rust
#[test]
fn duplicate_event_is_suppressed_for_thirty_minutes() { /* fake clock */ }

#[test]
fn same_event_key_on_different_servers_is_not_duplicate() { /* server id in key */ }

#[test]
fn expired_entries_are_removed_and_capacity_is_bounded() { /* 30 min + max 1024 */ }

#[test]
fn baseline_events_are_not_sent() { /* observed_at <= monitor_started_at */ }

#[test]
fn only_four_kinds_map_to_notifications() { /* exhaustive current enum */ }
```

- [ ] **Step 2: Implement testable policy separate from Tauri plugin**

```rust
pub struct NotificationPolicy {
    seen: HashMap<String, DateTime<Utc>>,
    ttl: Duration,
    capacity: usize,
}

impl NotificationPolicy {
    pub fn should_send(&mut self, event: &AgentEvent, monitor_started_at: DateTime<Utc>, now: DateTime<Utc>) -> bool;
}
```

`NotificationCoordinator` uses `tauri_plugin_notification::NotificationExt` only after policy passes and `settings.notifications == true`. Map titles exactly:

- Kimi + Completed → `Kimi Code · 任务完成`
- dsh + Completed → `DeepSeek Harness · 任务完成`
- corresponding `任务失败`、`等待审批`、`待回答`

- [ ] **Step 3: Write opener tests**

```rust
#[test]
fn opens_only_url_derived_from_saved_server() {
    assert_eq!(build_server_url(&server, None).unwrap().as_str(), "http://100.64.0.2:3080/");
    assert!(validate_external_url("file:///etc/passwd").is_err());
    assert!(validate_external_url("javascript:alert(1)").is_err());
}
```

For Kimi session URL, first copy the exact route construction from Android `MainActivity.loadConfiguredUrl`; for dsh always use root URL unless a stable route is verified by fixture/integration test.

- [ ] **Step 4: Implement opener through Rust plugin API**

`open_saved_server(app, config, server_id, session_id)` looks up the server by id; callers cannot submit arbitrary URL. Use `tauri_plugin_opener::OpenerExt` from Rust.

- [ ] **Step 5: Run tests**

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml notification opener -- --nocapture
```

Expected: PASS.

- [ ] **Step 6: Commit after explicit user confirmation**

```bash
git add desktop/src-tauri/src/notification.rs desktop/src-tauri/src/opener.rs desktop/src-tauri/src/lib.rs
git commit -m "feat(desktop): add high-value notifications and safe opener"
```

---

### Task 6: Tauri App State, Commands, Import Preview Cache

**Files:**
- Create: `desktop/src-tauri/src/commands.rs`
- Modify: `desktop/src-tauri/src/lib.rs`
- Modify: `desktop/src-tauri/src/model.rs`
- Modify: `desktop/src-tauri/capabilities/default.json`
- Test: `commands.rs` unit tests

**Interfaces:**
- Consumes: ConfigStore, MonitorManager, ServerStatus, probe, opener.
- Produces commands: `get_app_view`, `get_server_for_edit`, `save_server`, `delete_server`, `set_active_server`, `update_settings`, `probe_backend`, `reconnect_all`, `open_server`, `preview_import`, `apply_import`, `export_config`.
- Emits frontend event: `app-state-changed` with redacted `AppView`.

- [ ] **Step 1: Define managed state and redacted DTO tests**

```rust
pub struct AppState {
    pub config: RwLock<AppConfig>,
    pub statuses: RwLock<HashMap<String, ServerStatus>>,
    pub store: ConfigStore,
    pub monitors: Mutex<MonitorManager>,
    pub import_previews: Mutex<HashMap<Uuid, PendingImport>>,
    pub explicit_exit: AtomicBool,
}
```

Tests verify `get_app_view` serialization contains no token and `get_server_for_edit(id)` returns token only for that exact saved id.

- [ ] **Step 2: Implement mutation transaction rule**

All config mutations follow:

1. clone current config;
2. validate/apply to clone;
3. persist clone atomically;
4. replace in-memory config;
5. update autostart if setting changed;
6. sync affected monitors;
7. emit redacted `app-state-changed`;
8. request tray rebuild.

If step 3 fails, do not mutate memory or monitors.

- [ ] **Step 3: Implement opaque import preview flow**

```rust
#[derive(Serialize)]
pub struct ImportPreview {
    pub import_id: Uuid,
    pub valid_count: usize,
    pub invalid: Vec<ImportIssue>,
    pub source_kind: ImportSourceKind,
}
```

`preview_import(path)` stores parsed token-bearing data only in Rust under `import_id`, returns counts/issues without tokens. `apply_import(import_id, mode)` removes the pending entry and expires previews older than 10 minutes. Frontend never receives imported tokens.

- [ ] **Step 4: Register commands and minimal capabilities**

`lib.rs` invokes all commands with `tauri::generate_handler![]`. Capabilities grant only core window/event plus dialog file selection required by frontend; do not grant shell or filesystem plugins. Autostart/notification/opener are called from Rust and need no broad frontend capability.

- [ ] **Step 5: Connect monitor updates to state**

Spawn one coordinator task in setup:

- `MonitorUpdate::Status` updates `statuses`, emits `app-state-changed`, refreshes tray.
- `MonitorUpdate::Event` calls NotificationCoordinator.
- No lock is held across `.await` or Tauri plugin calls.

- [ ] **Step 6: Run tests and Clippy**

```bash
cargo test --manifest-path desktop/src-tauri/Cargo.toml commands -- --nocapture
cargo clippy --manifest-path desktop/src-tauri/Cargo.toml --all-targets -- -D warnings
```

Expected: PASS / no warnings.

- [ ] **Step 7: Commit after explicit user confirmation**

```bash
git add desktop/src-tauri
git commit -m "feat(desktop): expose secure desktop control commands"
```

---

### Task 7: Dynamic Tray, Window Lifecycle, Autostart, Graceful Exit

**Files:**
- Create: `desktop/src-tauri/src/tray.rs`
- Modify: `desktop/src-tauri/src/lib.rs`
- Modify: `desktop/src-tauri/tauri.conf.json`
- Create: `desktop/src-tauri/icons/backend-kimi.png`
- Create: `desktop/src-tauri/icons/backend-dsh.png`
- Create: `desktop/src-tauri/icons/backend-offline.png`
- Test: pure tray model tests in `tray.rs`

**Interfaces:**
- Consumes: redacted config/status snapshot, safe opener, AppState.
- Produces: `TrayController::{install,rebuild,set_tooltip}`; lifecycle callbacks.

- [ ] **Step 1: Write pure tray menu model tests**

```rust
#[test]
fn menu_orders_servers_then_actions() { /* exact item ids server:<uuid>, open-active, manage, reconnect, autostart, quit */ }

#[test]
fn offline_server_uses_gray_icon_and_running_count_changes_label() { /* no green dot */ }

#[test]
fn tooltip_summarizes_connected_and_active_counts() {
    assert_eq!(tooltip(&statuses, 3), "已连接 2/3 台，运行 4 个任务");
}
```

- [ ] **Step 2: Implement dynamic menu rebuild**

Use Tauri 2 Rust APIs documented in `tauri::menu` and `tauri::tray`: `MenuBuilder`, `IconMenuItemBuilder`, `CheckMenuItemBuilder`, `TrayIconBuilder`, `on_menu_event`, `on_tray_icon_event`. Keep one tray id `main-tray`; replace its menu on state changes rather than leaking menu item handles.

- [ ] **Step 3: Implement click behavior**

- `server:<id>` / `open-active` → `open_saved_server`.
- `manage` → `show()`, `unminimize()`, `set_focus()` main window.
- `reconnect` → spawn async `MonitorManager::reconnect_all()`.
- `autostart` → official autostart plugin `enable()/disable()`, then persist setting transactionally.
- `quit` → set `explicit_exit=true`, run shutdown with 3s timeout, then `app.exit(0)`.
- Tray `DoubleClick` left button opens active server; if absent, show management window. Single click must not launch browser.

- [ ] **Step 4: Implement window close-to-hide**

Register `WindowEvent::CloseRequested`; when `explicit_exit == false`, call `api.prevent_close()` and `window.hide()`. Set `skipTaskbar` only while hidden if platform behavior is stable; do not add Linux-only window APIs.

- [ ] **Step 5: Configure startup visibility**

In setup, read persisted config before showing main window. If `startHidden == true` outside development, hide immediately; development keeps window visible unless launched with `--hidden`. Autostart plugin is configured with `--hidden`.

- [ ] **Step 6: Verify tray manually on Linux**

Run:

```bash
cd desktop
npm run tauri dev
```

Verify: tray visible; manage shows one window; close hides; double-click opens active server; autostart check updates; quit ends process and monitor tasks. Capture issues in implementation notes, not a new unsolicited document.

- [ ] **Step 7: Commit after explicit user confirmation**

```bash
git add desktop/src-tauri
git commit -m "feat(desktop): add dynamic tray and background lifecycle"
```

---

### Task 8: Vanilla TypeScript Settings UI and Import/Export UX

**Files:**
- Create: `desktop/src/model.ts`
- Create: `desktop/src/validation.ts`
- Create: `desktop/src/validation.test.ts`
- Create: `desktop/src/server-list.ts`
- Create: `desktop/src/server-list.test.ts`
- Create: `desktop/src/import-export.ts`
- Create: `desktop/src/import-export.test.ts`
- Modify: `desktop/src/main.ts`
- Modify: `desktop/src/styles.css`
- Modify: `desktop/index.html`

**Interfaces:**
- Consumes Tauri commands/events from Task 6.
- Produces: accessible server management UI; no direct token exposure in list.

- [ ] **Step 1: Write validation tests**

```ts
import { describe, expect, it } from 'vitest';
import { validateServerDraft } from './validation';

describe('validateServerDraft', () => {
  it('accepts a valid dsh server', () => {
    expect(validateServerDraft({ name: 'Work', host: '100.64.0.2', port: 3080, token: '', backend: 'dsh' })).toEqual({});
  });
  it.each(['http://host', 'host:3080', 'host/path', 'bad host'])('rejects host %s', host => {
    expect(validateServerDraft({ name: 'Work', host, port: 3080, token: '', backend: 'dsh' }).host).toBeTruthy();
  });
  it.each([0, 65536, NaN])('rejects port %s', port => {
    expect(validateServerDraft({ name: 'Work', host: 'host', port, token: '', backend: 'dsh' }).port).toBeTruthy();
  });
});
```

- [ ] **Step 2: Implement frontend models and validation**

Define exact command DTOs mirroring Rust camelCase fields. `ServerSummary` contains no token. `ServerEdit` contains token only after `get_server_for_edit`.

- [ ] **Step 3: Write list rendering tests**

Use pure functions returning DOM nodes/HTML from passed state (no global invoke). Assert:

- dsh/Kimi logo choice;
- offline class is gray and no green dot exists;
- active badge and running count;
- serialized list HTML never contains known token.

If DOM environment is needed, add `jsdom` as dev dependency and `environment: 'jsdom'` in Vitest config.

- [ ] **Step 4: Implement server list and edit dialog**

Required UX:

- cards with logo/name/address/status/current/edit/delete;
- add/edit modal with name/host/port/backend/token;
- probe button calls `probe_backend` and updates backend on success; error does not clear form;
- deletion confirmation; at least zero servers is allowed on desktop;
- save errors rendered inline, not `alert()` only.

- [ ] **Step 5: Implement settings controls**

Three controls call `update_settings`: autostart, startHidden, notifications. Disable the changed control during invoke and roll back UI on error.

- [ ] **Step 6: Implement import/export with explicit credential warning**

Use official dialog plugin only to choose paths. Flow:

- Import → open file → `preview_import(path)` → show valid/invalid counts → user chooses merge or replace → `apply_import(importId, mode)`.
- Full export / Android export → modal text exactly `文件包含服务器访问凭据，请勿公开分享` → save path → `export_config(path, format)`.
- Never render imported token values.

- [ ] **Step 7: Wire state event and prevent stale renders**

`main.ts`:

1. register `listen<AppView>('app-state-changed', ...)` before first fetch;
2. call `get_app_view`;
3. render latest monotonically increasing `revision` only;
4. store unlisten callback for HMR cleanup.

Add `revision: u64` to Rust AppView if not already present.

- [ ] **Step 8: Run frontend checks**

```bash
cd desktop
npm run test
npm run build
```

Expected: all Vitest tests pass; TypeScript has no errors.

- [ ] **Step 9: Commit after explicit user confirmation**

```bash
git add desktop/src desktop/index.html desktop/package*.json desktop/vite.config.ts desktop/tsconfig.json
git commit -m "feat(desktop): add server management settings UI"
```

---

### Task 9: End-to-End Integration, Linux Bundles, README

**Files:**
- Modify: `desktop/src-tauri/tauri.conf.json`
- Create: `desktop/src-tauri/icons/32x32.png`
- Create: `desktop/src-tauri/icons/128x128.png`
- Create: `desktop/src-tauri/icons/128x128@2x.png`
- Create: `desktop/src-tauri/icons/icon.png`
- Modify: `README.md`
- Modify: `.github/workflows/desktop-linux.yml` only if repository already uses GitHub Actions and user approves adding CI; otherwise document local commands only.

**Interfaces:**
- Consumes all previous tasks.
- Produces: tested AppImage and `.deb`; user-facing desktop instructions.

- [ ] **Step 1: Add production icon assets and bundle config**

Generate desktop app icons from the existing AgentPocket launcher/logo source, preserving brand. Configure bundle identifier `com.local.agentpocket.desktop`, product name `AgentPocket`, targets `appimage` and `deb`, and include tray/backend resources.

- [ ] **Step 2: Run full automated verification**

```bash
cd /home/user/progs/KimiCodeWebApp/desktop
export PATH="$HOME/.nvm/versions/node/v22.22.0/bin:$PATH"
npm ci
npm run test
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --all-targets
npm run tauri build -- --bundles appimage,deb
```

Expected: all exit 0; bundle paths printed under `desktop/src-tauri/target/release/bundle/{appimage,deb}/`.

- [ ] **Step 3: Perform real Kimi/dsh smoke test**

With known test servers:

1. add one Kimi and one dsh server;
2. verify status and active counts;
3. background window;
4. trigger completed, failed/interrupted, approval, question events;
5. verify exactly one appropriate notification per event and no tool-level notifications;
6. disconnect one server and confirm the other stays connected;
7. click tray items and verify browser target;
8. reconnect all;
9. quit and confirm no AgentPocket process remains.

Do not log/capture real token or prompt content in artifacts.

- [ ] **Step 4: Verify import/export and recovery**

- Export Android-compatible array, inspect field names (without printing token to console), re-import with merge.
- Import replacement with valid entries and verify backup count ≤ 5.
- Corrupt a copied test config in a temporary app-data directory and verify backup recovery; do not corrupt the real user config.

- [ ] **Step 5: Verify packages**

Install `.deb` using the normal desktop package workflow only after user confirmation; launch from application menu, validate tray/notification/autostart, uninstall, then run AppImage. If installation requires sudo, ask separately before doing it.

- [ ] **Step 6: Update README**

Add a `Desktop (Linux)` section covering:

- Tauri tray scope and four notification types;
- `desktop/` development prerequisites;
- `npm ci`, `npm run tauri dev`, `npm run tauri build -- --bundles appimage,deb`;
- AppImage/`.deb` output locations;
- JSON import/export credential warning;
- dsh/Kimi server networking remains the user's responsibility;
- Windows/macOS not yet release-validated.

Do not document internal auto-detection implementation details or app-internal behavior beyond user actions.

- [ ] **Step 7: Inspect final diff and working tree**

```bash
cd /home/user/progs/KimiCodeWebApp
rtk git diff --check
rtk git status --short
rtk git diff --stat
```

Expected: only intended desktop files, README, `.gitignore`, spec and plan are changed/untracked; no `node_modules`, `target`, tokens, local config, keystore or build artifacts.

- [ ] **Step 8: Commit after explicit user confirmation**

```bash
git add .gitignore README.md desktop docs/superpowers/specs/2026-08-18-agentpocket-desktop-tray-design.md docs/superpowers/plans/2026-08-18-agentpocket-desktop-tray.md
git commit -m "feat: add AgentPocket Linux desktop tray app"
```

- [ ] **Step 9: Push/release only with fresh explicit confirmation**

Do not push or create a GitHub Release implicitly. If the user requests it, first show the final commit and bundle checksums, then ask confirmation for that outward-facing action.

---

## Implementation Checkpoints

After Tasks 1–3: review API/data-model correctness before any live network monitor.

After Tasks 4–6: run a protocol/monitor security review—token redaction, cancellation, lock scope, arbitrary URL/path rejection.

After Tasks 7–8: review tray/window UX and ensure closing the window never kills monitors.

After Task 9: run verification-before-completion; no success claim until automated checks, real-server smoke tests and bundle builds have evidence.
