# GUI Mesh 面板（Phase 2 桌面部分）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development or executing-plans. Steps use checkbox tracking.

**Goal:** 桌面端设置对话框新增 Mesh 同步区块：peer 列表（tailscale 发现 + 手动）+ 拉取/推送 + 手动添加；复用现有导入预览流程与 update_settings 通道。

**Architecture:** 把 daemon 的 `client.rs`/`discovery.rs` 上移到 core（GUI 与 daemon 共享，符合规格 §3 的 core 扩容路线）；GUI 新增 3 个 tauri command（discover/pull/push）+ `DesktopSettings.meshPeers`；前端独立渲染模块 `mesh-panel.ts`（纯函数可测）。**不动**：旧扫码同步（仍是手机唯一同步通道）、daemon 的 mesh 服务端、主窗口布局。

**Tech Stack:** 既有栈（Tauri 2 / TS 原生 DOM / Rust / vitest）。

## Global Constraints

- 手机端不做 peer 管理；GUI 不加二维码引导；daemon 行为零变化（仅模块导入路径调整，测试语义不变）。
- 手动 peer 存 `DesktopSettings.meshPeers`（字段级 `#[serde(default)]`，旧配置兼容）；交换格式 schema 不变。
- 端口恒 48720；mesh 通信复用 core::mesh_client（明文 HTTP）。
- cargo 前置 env（rsproxy 镜像）；git 一律 `rtk`；中文注释。
- 测试基线随 Task A 变动：core 28→38（+client 4 +discovery 6，其中 2 个由真实端点改 tiny_http mock）、daemon 28→18；GUI Rust 55→+新增；前端 25→+新增。

## Task A: core 扩容——mesh_client/discovery/host 上移

- `git mv daemon/src/client.rs core/src/mesh_client.rs`；`git mv daemon/src/discovery.rs core/src/discovery.rs`
- `core/src/lib.rs`：`pub mod mesh_client; pub mod discovery; pub mod host;`
- `core/src/host.rs`：`pub fn hostname() -> String`（逻辑自 daemon/paths.rs 移入）；daemon paths.rs 删 hostname，daemon 调用点（main.rs、ops.rs）改 `agentpocket_core::host::hostname()`
- `core/src/discovery.rs` 拆分：`tailscale_candidates(Option<&Path>) -> Vec<(ip,name)>`、`probe_candidates(&[(ip,name)], timeout) -> Vec<MeshPeer>`（并发+去重+排序）、`discover(config_dir, tailscale, timeout)` = 两者组合 + peers.json 手动项；daemon 调用不变
- 原 client/discovery 测试中 `crate::mesh::start` 的 2 处改为 core 内 tiny_http mock `/info`（core dev-deps 加 tiny_http）；daemon/src/main.rs 删 `mod client; mod discovery;`，ops.rs/status.rs `use crate::client;` → `use agentpocket_core::mesh_client as client;`，main.rs `use agentpocket_core::discovery;`
- 验证：core `cargo test`（38）、daemon `cargo test`（18）、desktop/src-tauri `cargo test --offline`（55）
- Commit：`refactor: mesh_client/discovery/host 上移 core，GUI 与 daemon 共享`

## Task B: GUI Rust——mesh 命令 + 设置模型

- `core/src/model.rs`：`DesktopSettings` 加 `#[serde(default)] pub mesh_peers: Vec<MeshPeerEntry>`；`MeshPeerEntry { name: String, host: String }`（camelCase，serde default 空向量；Default impl 同步）
- `desktop/src-tauri/src/mesh.rs`（新）：
  - `discover_mesh_peers(state) -> Result<Vec<MeshPeerView>, CommandError>`（sync）：`tailscale_candidates` + `settings.mesh_peers` 合并 → `probe_candidates` → 在线 MeshPeer + 未应答的手动 peer（标 `online:false`，按 host 去重在线优先）。`MeshPeerView { name, host, version: Option<String>, online: bool, manual: bool }`（camelCase 序列化）
  - `mesh_pull(state, host) -> Result<ImportPreview, CommandError>`（sync）：`mesh_client::get(host, 48720, "/config", 5s)` → 非 200 报错 → `preview_from_content(state, &body)`
  - `mesh_push(state, host) -> Result<PushCounts, CommandError>`（sync）：读当前配置 `export_text` → `mesh_client::post`（`X-AgentPocket-Source: core::host::hostname()`）→ `PushCounts { added, updated }`
- `commands.rs`/`lib.rs`：注册三个命令（`mesh::discover_mesh_peers` 等）；CommandError 复用/新增变体
- `desktop/src/model.ts`：DesktopSettings 加 `meshPeers: MeshPeerEntry[]`；commands 常量 + `MeshPeerView`/`PushCounts` 类型
- 测试（src-tauri，tempdir + tiny_http mock 对端）：mesh_pull 注册 preview 且返回 valid_count；mesh_push 带 source 头且解析计数；discover 合并在线/离线手动 peer 去重
- Commit：`feat(desktop): mesh peer 发现与拉取/推送命令（设置模型扩容）`

## Task C: GUI 前端——设置对话框 Mesh 区块

- `desktop/src/mesh-panel.ts`（新，纯函数）：`renderMeshSection(peers: MeshPeerView[], busy: boolean): string`——列表行（名称/host/版本/在线点），每行 `data-mesh-host` + 拉取/推送按钮；手动 peer 离线显示"离线"；空态文案；发现中态。HTML 转义沿用 server-list.ts 惯例
- `main.ts`：设置对话框 `.settings-body` 追加 Mesh 区块（打开时 invoke discover；刷新按钮；手动添加表单 host+备注名 → 探测 `/info` 验证由后端 discover 承担：保存即 `update_settings` 追加 meshPeers 后刷新列表）；拉取 → `invoke meshPull` → `pendingImport` + `openImportConfirmation()`（复用）；推送 → `invoke meshPush` → `setMessage` toast
- `desktop/src/mesh-panel.test.ts`：渲染断言（在线行/离线手动行/去重/空态/转义/不含 token），~5 用例
- 验证：`npm test` 全绿 + `cargo test --offline`（src-tauri）；`npm run tauri build` 出包冒烟
- Commit：`feat(desktop): 设置对话框 Mesh 同步区块（peer 列表 + 拉取/推送）`
