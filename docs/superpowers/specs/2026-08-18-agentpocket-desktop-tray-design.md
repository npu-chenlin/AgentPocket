# AgentPocket Desktop Tray 设计

- 日期：2026-08-18
- 状态：已批准
- 首发平台：Linux
- 技术栈：Tauri 2 + Vite + TypeScript 原生 DOM + Rust
- 仓库位置：现有 AgentPocket 仓库的 `desktop/` 子目录

## 1. 背景与目标

AgentPocket Android 已能连接多台 Kimi Code Web / DeepSeek Harness（dsh）服务器，监听服务器在线状态、忙碌会话数和任务事件，并在手机上发送完成、失败、审批与提问通知。

桌面端首版定位为轻量的 **Agent Tray / Control Center**，不是 Android WebView 容器的桌面移植，也不重建 Kimi/dsh 的完整会话 UI。它常驻系统托盘，统一监控多台服务器，发送高价值系统通知，并用系统浏览器一键打开对应服务器或会话。

## 2. 范围

### 2.1 首版包含

- 多服务器增删改与当前服务器选择
- Kimi / dsh 后端自动探测和手动选择
- 每台服务器独立后台 monitor
- 在线/离线与忙碌会话数
- 完成、失败/中断、等待审批、等待回答四类系统通知
- 系统托盘菜单与一键浏览器打开
- 启动时隐藏、关闭窗口后继续常驻、显式退出
- 开机启动
- 与 Android 兼容的服务器 JSON 导入/导出
- Linux AppImage 和 `.deb` 构建

### 2.2 首版不包含

- 内嵌 Kimi/dsh 网页
- 自建会话列表、消息流或工具卡片 UI
- 通知内审批/回答
- agent workflow / DAG / 集群编排
- 自动云同步或 GitHub/Gitee 配置同步
- 工具调用、增量文本、工具失败等中间过程通知
- Windows/macOS 安装包正式发布（代码保持可移植，后续单独验证）

## 3. 技术选型

采用 **Tauri 2 + Vite + TypeScript 原生 DOM + Rust 后端**。

- Tauri 提供小体积桌面壳、托盘、系统通知、开机启动、shell opener 和应用数据目录。
- Rust 后端承载长期 WebSocket 连接、协议解析、状态聚合、配置持久化和系统集成，避免把关键监听逻辑绑定到设置窗口 WebView 的生命周期。
- 设置界面规模较小，首版使用 TypeScript 原生 DOM，不引 React/Vue/Svelte；未来完整控制中心需要复杂状态 UI 时再评估框架。
- 不把 monitor 放在 TypeScript/WebView 中，以保证窗口隐藏或关闭后监听仍可靠运行。

## 4. 目录与模块

```text
desktop/
├── package.json
├── vite.config.ts
├── index.html
├── src/
│   ├── main.ts                 # Tauri invoke/event 入口
│   ├── server-list.ts          # 列表与表单渲染
│   ├── import-export.ts        # 导入/导出交互
│   └── styles.css
└── src-tauri/
    ├── Cargo.toml
    ├── tauri.conf.json
    ├── capabilities/
    └── src/
        ├── lib.rs              # 应用状态、commands、生命周期
        ├── model.rs            # Server、状态、统一事件模型
        ├── config.rs           # 配置读取/原子写入/导入导出
        ├── monitor/
        │   ├── mod.rs          # trait、manager、重连生命周期
        │   ├── kimi.rs         # Kimi HTTP + WebSocket 协议
        │   └── dsh.rs          # dsh HTTP RPC + events.mux
        ├── notification.rs     # 四类通知、去重、点击目标
        ├── tray.rs             # 托盘菜单与状态刷新
        └── opener.rs           # 浏览器 URL / 会话深链
```

模块边界：

- `monitor` 只把后端协议归一化为统一状态/事件，不直接操作 UI 或通知。
- `notification` 只消费统一事件，不解析 Kimi/dsh 原始帧。
- `tray` 只消费当前状态快照并处理菜单动作。
- `config` 不依赖 Tauri UI，可独立单元测试。
- TypeScript 前端只能通过白名单 commands 读写配置和触发动作，不直接访问文件系统或 token。

## 5. 统一数据模型

### 5.1 服务器配置

与 Android 的核心字段兼容：

```json
{
  "id": "uuid",
  "name": "工作站",
  "host": "100.95.189.73",
  "port": 3080,
  "token": "",
  "backend": "dsh"
}
```

约束：

- `backend` 只能是 `kimi` 或 `dsh`。
- `host` 不含协议、路径、空格或端口。
- `port` 范围 1–65535。
- `token` 可为空（dsh 通常为空）。
- `id` 使用 UUID；导入数据缺失 id 时生成新 id。

桌面端配置文件另有外层结构：

```json
{
  "schema": 1,
  "activeId": "uuid",
  "servers": [],
  "settings": {
    "startHidden": true,
    "autostart": false,
    "notifications": true
  }
}
```

Android 导出的裸服务器数组也可导入；桌面导出时默认输出上述带 schema 的完整结构，并提供“仅服务器列表”兼容导出。

### 5.2 服务器状态

```rust
struct ServerStatus {
    connected: bool,
    active_count: u32,
    last_checked_at: Option<DateTime<Utc>>,
    error: Option<String>,
}
```

### 5.3 统一事件

```rust
enum AgentEventKind {
    Completed,
    Failed,
    ApprovalRequired,
    QuestionRequired,
}

struct AgentEvent {
    server_id: String,
    session_id: Option<String>,
    session_title: Option<String>,
    kind: AgentEventKind,
    event_key: String,
    body: Option<String>,
    occurred_at: DateTime<Utc>,
}
```

## 6. Monitor 架构

每台服务器有一个独立 monitor task，由 `MonitorManager` 管理。配置变更时只重启受影响的 monitor；删除服务器时取消对应 task。

统一接口：

```rust
trait ServerMonitor {
    async fn run(
        self,
        state_tx: watch::Sender<ServerStatus>,
        event_tx: mpsc::Sender<AgentEvent>,
        cancel: CancellationToken,
    );
}
```

实际实现可因 async trait ergonomics 使用 boxed future 或普通异步函数，但外部行为保持一致。

### 6.1 dsh monitor

- `POST /api/session.list` 获取会话标题和 `running` baseline。
- 连接 `ws(s)://host:port/api/events.mux`，只消费 `server-request` 下行帧。
- 解析：
  - `session/event`：`turn/start`、`turn/end`、`session/title`
  - `session/projection`：title
  - `approval/requested`
  - `question/requested`
- 不解析工具调用和增量文本。
- dsh 无 token 鉴权；依赖其 Host 信任围栏与用户已建立的网络路径。

### 6.2 Kimi monitor

- 复用 Android 已验证的 Kimi HTTP session list 与 WebSocket 协议语义。
- token 通过 `Sec-WebSocket-Protocol: kimi-code.bearer.{token}` 传递（有 token 时）。
- 解析协议事件和 agent 事件，归一化 complete/failed/approval/question。

### 6.3 重连

- 断线后指数退避：1s、2s、4s、8s、16s、30s，之后固定 30s。
- 成功连接后重置为 1s。
- 网络恢复、配置保存或“重新连接全部”时立即取消等待并重新连接。
- 单台服务器的连接失败不影响其他 monitor。
- 进程退出时向所有 task 发 cancellation，最长等待 3 秒后结束运行时。

## 7. 托盘行为

托盘菜单结构：

```text
AgentPocket
──────────────
● 工作站       2 个任务运行中
● 家庭服务器   在线
○ 备用机       离线
──────────────
打开当前服务器
管理服务器…
重新连接全部
开机启动       ✓
退出
```

- 服务器菜单项图标使用后端品牌图标：Kimi 小蓝球、dsh 鲸鱼；离线时统一灰色，不增加绿色状态点。
- 点击服务器：用系统默认浏览器打开该服务器根地址。
- 双击托盘图标：打开当前服务器；若没有当前服务器则打开管理窗口。
- “管理服务器…”显示/聚焦唯一设置窗口。
- 窗口关闭事件改为 hide，不退出进程。
- “退出”设置显式退出标志，停止 monitor 后终止应用。
- 托盘 tooltip 汇总：`已连接 X/Y 台，运行 N 个任务`。

## 8. 设置窗口

设置窗口包括：

1. 服务器列表
   - 后端 logo、名称、`host:port`
   - 在线/离线、忙碌数量
   - 当前服务器标识、编辑、删除
2. 添加/编辑表单
   - 名称、host、port、backend、token
   - “自动识别”按钮；识别失败时保留用户手选值
3. 操作
   - 添加服务器、重新连接
   - JSON 导入、完整导出、Android 兼容导出
4. 设置
   - 开机启动
   - 启动时隐藏（默认 true）
   - 系统通知（默认 true）

自动探测顺序：

1. 请求 dsh `POST /api/agentPreset.list`，收到 `server-response` 则识别 dsh。
2. 请求 Kimi `GET /api/v2/sessions`，非 404 且响应可识别则识别 Kimi。
3. 两者均失败时显示失败原因并允许手选，不阻止保存。

## 9. 导入/导出

### 导入

- 支持 schema 1 完整结构与 Android 裸数组。
- 导入前验证字段；无效条目逐条列出，不静默吞掉。
- 用户选择：
  - **合并**：按 id 更新；无 id 或 id 未命中则新增。相同 id 的导入项覆盖本地核心字段。
  - **替换**：使用有效导入项替换全部服务器；若有效项为空则拒绝替换。
- 导入前将现有配置复制为带时间戳备份（最多保留 5 份）。

### 导出

- 完整导出包含 token，必须弹出明确警告：“文件包含服务器访问凭据，请勿公开分享”。
- Android 兼容导出只输出服务器数组，字段仍包含 token；同样警告。
- 不提供“自动上传 GitHub/Gitee”。

## 10. 通知

仅发送四类高价值事件：

- 回合完成
- 回合失败/中断
- 等待审批
- 等待回答

通知内容：

- 标题：`Kimi Code · 任务完成` 或 `DeepSeek Harness · 等待审批`。
- 正文：`[服务器名称] 会话标题/问题摘要`。
- 不通知中间工具调用、token 增量、普通工具失败。

去重：

- key 为 `server_id + event_key`。
- 使用容量有限的 LRU/TTL 缓存；默认 30 分钟过期，避免重连 baseline 重复通知，也防止无限增长。
- 仅对 monitor 启动后出现的新事件通知；连接时收到的 baseline 不通知。

点击：

- 能构造会话 URL 时打开对应会话。
- dsh 无稳定深链时打开服务器首页。
- Linux 桌面通知后端若不支持点击回调，保证通知可见，托盘仍提供一键打开。

## 11. 配置、安全与日志

- 配置保存在 Tauri 应用数据目录，不放仓库或工作目录。
- 写入使用同目录临时文件、`fsync`（平台支持时）和 atomic rename。
- token 首版随配置保存，文件权限在 Unix 上设为 `0600`；Windows/macOS 使用应用数据目录权限。后续可迁移到系统 keyring。
- 日志只记录 server id/name、状态、HTTP 状态码和协议事件类型；不得记录 token、完整 URL query、消息正文或原始帧。
- 前端不会收到 token，除非用户进入单台服务器编辑流程；列表命令返回脱敏模型。
- Tauri capabilities 采用最小权限；前端无任意 shell、任意文件系统或任意 URL 打开权限。
- opener 只允许由已保存服务器配置构造的 `http/https` URL。

## 12. 错误处理

- 单台服务器错误显示在该服务器状态，不弹出全局阻塞窗口。
- 保存配置失败时不更新内存态，返回可读错误。
- 导入部分失败时展示“有效 N 条 / 无效 M 条”与原因，用户确认后才应用有效数据。
- monitor 收到无法识别的事件只写 debug 日志，不断开连接。
- 配置文件损坏时尝试读取最近备份；仍失败则启动空配置并提示用户，不覆盖损坏原文件。
- 通知发送失败不影响 monitor 或托盘状态。

## 13. 测试

### Rust 单元测试

- 配置 schema 读取与 Android 裸数组兼容
- 导入合并/替换、无效字段、备份轮转
- 原子写入失败路径
- Kimi/dsh 脱敏 fixture 的状态和事件归一化
- baseline 抑制、事件去重 TTL
- 指数退避序列与成功重置
- opener URL 约束

### TypeScript 测试

- host/port/backend 表单校验
- 服务器状态渲染
- 导入结果与错误展示
- token 列表脱敏

### Linux 集成验证

- 托盘显示、菜单刷新、窗口隐藏/恢复
- 双击托盘与系统浏览器打开
- 完成/失败/审批/提问通知
- 开机启动开关
- 多服务器断线隔离与手动重连
- AppImage 和 `.deb` 安装、运行与卸载

## 14. 完成标准

首版完成必须满足：

1. Linux 启动后默认常驻托盘，关闭设置窗口不会退出。
2. 可管理多台 Kimi/dsh 服务器，并准确显示在线/离线和忙碌数量。
3. 四类通知可靠、去重，不产生工具级噪音。
4. 托盘点击能用系统浏览器打开目标服务器；支持当前服务器。
5. 配置与 Android 字段兼容，可手动导入/导出，配置写入具备原子性和备份。
6. 单台服务器故障不影响其他服务器；应用和 monitor 能干净退出。
7. Rust/TypeScript 测试通过，Linux AppImage 与 `.deb` 构建成功。
