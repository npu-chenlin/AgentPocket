# AgentPocket Mesh 同步设计

- 日期：2026-08-20
- 状态：已批准
- 涉及端：桌面端（Linux 优先，代码保持可移植）+ Android
- 技术栈：沿用现状（Tauri 2 + tiny_http + Rust；Android Java）

## 1. 背景与目标

桌面端已有一套**一次性扫码同步**：随机端口 + UUID token + 10 分钟 TTL 的临时 HTTP 服务器，手机扫码后 GET/POST `/config`。它的定位是"两台设备之间的临时握手"，无法支撑多设备之间的常态配置流动。

目标：把配置同步升级为 **mesh 模式**——信任网络内的每个桌面端都是对等节点，可被发现、可被拉取（load）、可被推送（push）。手机端保持纯客户端。

核心原则：

- **无鉴权**：信任网络本身，不设 token、不设配对。
- **网络边界即权限边界**：mesh 端点只接受来自 Tailscale 网段（及本机回环）的请求。tailnet 之内人人可信，之外一律 403。
- **无开关**：不设"Server 模式"设置项。桌面端启动即成为 mesh 节点，暴露面由网络边界约束，不需要用户管理。
- **数据流动由人触发**：不做自动 gossip/后台同步，所有 load/push 都是用户点按钮。

## 2. 范围

### 2.1 包含

- 桌面端常驻 mesh HTTP 端点（固定端口 48720），tailnet 网段访问控制
- 桌面端 peer 发现：`tailscale status --json` + 端口探测；手动添加 peer
- 拉取（pull）：从 peer 拉配置 → 现有导入预览流程（合并/替换二选一）
- 推送（push）：把本机配置推给 peer → 对方**自动合并** + 系统通知
- 手机端 peer 管理：扫码/手动添加、保存 peer、拉取/推送
- 删除旧的一次性扫码同步服务器（随机端口 + token + TTL 那套）

### 2.2 不包含（non-goals）

- mDNS / 局域网自动发现（用户环境以 Tailscale 为中心；将来真有需求再加）
- 任何形式的鉴权、配对、确认弹窗（信任网假设，见 §8）
- 自动 gossip 同步、配置变更自动传播
- 手机作为服务端（Android 后台保不住监听服务；需要前台服务 + 常驻通知，代价不成比例）
- 桌面主动推送到手机（手机配置是从桌面同步来的副本，无人需要往手机推）
- IPv6 tailnet 地址支持（端点只监听 IPv4；所有已知流程均使用 100.x IPv4）
- peer 列表本身跨设备同步（发现机制会重新找到它们）

## 3. 桌面端 mesh 端点

### 3.1 监听与访问控制

- `tiny_http` 常驻监听 `0.0.0.0:48720`（IPv4），随应用启动，随应用退出。
- 每个请求先过 `is_peer_allowed(remote_addr)`：
  - 允许：`127.0.0.0/8`（本机调试）、`100.64.0.0/10`（Tailscale CGNAT IPv4）
  - 其余：403，不读 body
- 端口被占（其他进程）：端点启动失败，**不阻塞应用启动**，同步面板显示错误状态。
- 效果：mesh 端点在 tailnet 内全员可达；在咖啡店 WiFi 等不可信网络上端口虽开但拿不到任何数据，配置中的 token 永不出 tailnet。

### 3.2 端点定义

| 方法 | 路径 | 行为 |
|---|---|---|
| GET | `/info` | 200，`{"app":"agentpocket","version":"2.8.0","name":"<主机名>"}`。探测握手用。 |
| GET | `/config` | 200，统一交换格式（现有 `export_text`：schema + activeId + servers）。 |
| POST | `/config` | body 为交换格式。自动合并后持久化，返回 200 `{"added":K,"updated":M}`。 |
| * | 其他 | 404 |

POST 语义（自动合并，即现有 `ImportMode::Merge`）：

- 同 ID 服务器覆盖，新 ID 追加，`activeId` 不变。
- 写入前走现有备份机制（`backup_primary`，5 份轮换）——误推可回滚。
- 成功后发系统通知：`AgentPocket：从 <来源> 收到配置（新增 K / 更新 M 台服务器）`。
- 来源取请求头 `X-AgentPocket-Source`（推送方主机名，纯标注无鉴权），缺失时回退显示远端 IP。
- body 非法（解析失败 / 0 台有效服务器）→ 400，不落盘、不通知。

### 3.3 删除项

旧 `sync.rs` 的一次性同步服务器整体删除：`start_sync_server` / `stop_sync_server` 命令、TTL、token 校验、`SyncInfo`/`SyncOption` 结构。**保留并复用** `enumerate_candidates()`（IP 候选枚举）与 `render_qr_svg()`（peer 二维码渲染）。

## 4. 桌面端发现

### 4.1 tailscale status 发现

1. 执行 `tailscale status --json`，解析 `Self` 与 `Peer`。
2. 过滤：`Online == true`，排除 `Self`。
3. 并发探测各 peer 的 IPv4 `48720/info`：TCP 连接超时 400ms，整体预算约 1.5s，线程并发。
4. `app == "agentpocket"` 的即为本 mesh 节点；其余（手机、普通机器）自然被过滤。

CLI 查找顺序：`PATH` 中的 `tailscale` → macOS `/Applications/Tailscale.app/Contents/MacOS/Tailscale` → Windows `C:\Program Files\Tailscale\tailscale.exe` → Linux `/usr/bin/tailscale`、`/usr/local/bin/tailscale`。找不到时发现列表只含手动 peer，面板提示"未找到 tailscale CLI"。

### 4.2 手动 peer

- 存于桌面本地设置（`DesktopSettings`，不进交换格式）：`meshPeers: [{name, host}]`，端口恒为 48720 不存储。
- host 可以是 100.x IP 或 MagicDNS 主机名。
- 与发现的 peer 按 host 去重，合并显示。

### 4.3 触发时机

同步面板打开时发现一次；面板可见期间每 30s 轮询刷新；面板内有手动刷新按钮。

## 5. 桌面端 UI

设置对话框新增 **Mesh 同步** 区块（替换原扫码同步区块）：

- peer 列表：名称（主机名）、地址、版本号（来自 `/info`）、在线状态点。每行两个动作：**拉取**、**推送**。
- 拉取：`GET /config` → 打开现有导入预览（选合并/替换），用户确认后应用。
- 推送：`POST /config`（带 `X-AgentPocket-Source`）→ 结果 toast（成功显示对方合并统计，失败显示错误）。
- 手动添加：输入 host（+可选备注名）→ 探测 `/info` 验证；探测失败时提示但**允许保存**（对方可能暂未运行，保存后列表显示离线）。备注名留空时用 `/info` 返回的主机名。
- **扫码引导**：面板展示一个二维码，编码 `agentpocket://peer?host=<ip>&port=48720`（复用 `enumerate_candidates` 选 Tailscale IP 优先 + `render_qr_svg`）。无 token、无 TTL——它只是把地址递给手机。
- 主窗口不加任何东西。

## 6. Android 端

- 同步入口改为 peer 管理界面：
  - 添加：扫二维码（`agentpocket://peer?host=&port=`，intent filter 更新；保留对旧 `agentpocket://sync` 的兼容识别但按新语义处理）或手动输入 host。
  - peer 持久化：与现有服务器配置同级的本地存储，格式 `{"peers":[{"name":"…","host":"…"}]}`。
- 每个 peer：拉取（`GET /config` → 手机现有导入预览流程）、推送（`POST /config` + `X-AgentPocket-Source`）、在线探测（`/info`）。显示名取 `/info` 的 `name`，探测不到时显示 host。
- 删除旧扫码同步里的 token 逻辑（`SyncClient` 改为无 token 的固定端点客户端）。

## 7. 边界情况

- **推送与用户操作并发**：推送在配置写锁内完成读-合并-写；用户侧导入预览确认晚于推送到达时，按当时最新配置再应用。可接受，不做合并仲裁。
- **推送到达时接收方正忙**：tiny_http 单线程逐请求处理，天然串行。
- **tailscale 显示在线但 mesh 探测超时**（对方未跑 AgentPocket / 端口被防火墙挡）：不出现在 peer 列表，无报错噪音。
- **同机两个 AgentPocket**：已被单实例机制排除。
- **两台桌面经同一台手机间接同步**：不做中继，各自直连。

## 8. 安全模型（明确接受）

- mesh 端点**无鉴权**：tailnet 内任何设备可读全量配置（含 kimi/dsh token）、可推送合并。
- 防线是网络边界：`100.64.0.0/10` 之外一律 403。tailnet 成员资格由 Tailscale 账号管理，等价于"tailnet 内互信"。
- 自动合并只增不删（覆盖同 ID），且有 5 份配置备份；最坏情况是配置被塞入不需要的服务器，可手工删除或回滚。
- README 如实陈述：**mesh 端点仅在你的 Tailscale 网络内可达**；不含"请勿在不可信网络开启"之类的开关话术（因为没有开关）。

## 9. 测试

- Rust 单测：`is_peer_allowed` 矩阵（回环/CGNAT/局域网/公网）；`/info` 握手；GET /config 放行与 403；POST 合并语义（added/updated 计数、备份生成、通知回调触发、非法 body 400）；tailscale JSON 解析（fixture）；探测过滤（mock server 应答 agentpocket/应答异物/不应答）；端口占用错误路径。
- 前端：Mesh 面板渲染与动作事件（沿用 vitest）。
- Android：peer 存取 round-trip、SyncClient 无 token 请求（沿用现有测试基建，若无则跟随现状）。

## 10. 版本

两端统一升至 2.8.0（Android versionCode 39）。发版流程沿用现有 release 习惯。
