# AgentPocket Mesh 守护进程设计（Phase 1）

- 日期：2026-08-20（v3 修订：聚焦独立守护进程 + 自动更新；GUI/手机端延后）
- 状态：已批准
- 产物：`agentpocket` 无头二进制（Linux x86_64 musl 静态）+ 一键安装脚本
- 原则：**GUI 与 Android 本阶段一行不改**

## 1. 背景与目标

AgentPocket 的配置（kimi/dsh 服务器列表）目前只在 GUI 桌面端与手机之间通过一次性扫码同步流动。目标（Phase 1）：交付一个**独立守护进程**，让任意机器（无头服务器、或与 GUI 共存的桌面机）成为 mesh 节点，配置可发现、可拉取、可推送，并支持**自动更新**与一键安装自启动。

核心原则（承继已批准决策）：

- 无鉴权：信任 tailnet。
- 网络边界即权限边界：mesh 端点只放行回环 + `100.64.0.0/10`。
- 无开关：daemon 启动即 mesh 节点。
- 数据流动由人触发（pull/push 命令），无自动 gossip。
- 同一 tailnet 内桌面/手机本可直连所有 kimi/dsh，监控通知仍由 GUI/手机承担——daemon 不做监控、不做通知、不写事件日志（journal 流方案已否决）。

## 2. 范围

### 2.1 Phase 1 包含

- `core` 共享 crate 抽取：`model.rs` + `config.rs`（配置模型/存取/导入导出/备份），GUI 以 `pub use` 转发引用，行为零变化
- `daemon` crate：mesh HTTP 端点（48720）、tailscale 发现、pull/push 客户端、自动更新
- 命令面：`serve` / `peers` / `pull` / `push` / `status` / `update` / `version`
- 一键安装脚本 `scripts/install.sh`：下载 + systemd 安装 + 自启动
- 与 GUI 同机共存：共享 `~/.local/share/com.local.agentpocket.desktop/config.json`，端口无冲突

### 2.2 Phase 2（延后，设计保留）

- GUI 的 Mesh 同步面板（peer 列表、拉取/推送按钮、扫码引导二维码）
- Android peer 管理（扫码/手动添加、拉取/推送、`agentpocket://peer` scheme）
- 删除 GUI 旧一次性扫码同步服务器

### 2.3 不包含（non-goals）

- 单一万能二进制（Tauri 链接 webkit2gtk，无头机器无法运行；两产物一核心）
- mDNS / 局域网发现；IPv6 tailnet（只监听 IPv4）
- 鉴权、配对、确认弹窗；自动 gossip 同步
- daemon 侧监控、事件流、通知（webhook/ntfy/journal）
- GUI 附加远程 daemon 的 attach 模式（Clash dashboard 形态；将来需要时给 daemon 加控制 API 即可，本设计不留实现但也不堵路）
- Windows/macOS 无头产物
- 手机作为服务端

## 3. 架构

```text
repo/
├── core/            # 共享 crate（无 GUI 依赖）：model、config
├── daemon/          # agentpocket 二进制
│   ├── mesh.rs          # HTTP 端点 + is_peer_allowed（tiny_http）
│   ├── discovery.rs     # tailscale status --json 解析 + 并发探测
│   ├── client.rs        # mesh HTTP 客户端（std TcpStream，无 TLS）
│   ├── update.rs        # 自动更新（ureq + rustls，仅此模块用 HTTPS）
│   └── main.rs          # 命令行入口
├── desktop/         # GUI（仅 src-tauri/{model,config}.rs 改为 pub use core 转发）
├── app/             # Android（不动）
└── scripts/install.sh
```

- **core 抽取最小化**：只抽 `model.rs`/`config.rs`（纯 std+serde，无 tauri 依赖）。`desktop/src-tauri` 加 path 依赖并留转发 shim，现有测试与行为不变。daemon 与 GUI 必须共享同一套配置格式与合并逻辑，杜绝复制分叉。
- **mesh 客户端无 TLS**：tailnet 内全部是 100.x 上的明文 HTTP，`std::net::TcpStream` 手写最小 HTTP 即可，musl 静态零系统依赖。仅自动更新走 GitHub API 需 HTTPS，用 `ureq`（rustls，musl 友好）。
- **配置路径**：daemon 与 GUI 同为 Linux XDG `~/.local/share/com.local.agentpocket.desktop/`（GUI 的 Tauri identifier 数据目录）；同机同用户共存时读同一份 `config.json`。
- 无 workspace 改造：`daemon/`、`core/`、`desktop/src-tauri/` 各自独立 crate，path 依赖互联。

## 4. mesh 端点（daemon::mesh）

- `tiny_http` 常驻监听 `0.0.0.0:48720`（IPv4），随 `serve` 进程生命周期。
- 每请求先过 `is_peer_allowed(remote_addr)`：允许 `127.0.0.0/8`、`100.64.0.0/10`；其余 403，不读 body。
- 端口被占：启动失败给出明确报错（"48720 已被占用"），进程退出；不与 GUI 冲突（GUI 旧同步用随机端口）。

| 方法 | 路径 | 行为 |
|---|---|---|
| GET | `/info` | 200，`{"app":"agentpocket","version":"…","name":"<主机名>"}` |
| GET | `/config` | 200，统一交换格式（`export_text`） |
| POST | `/config` | 自动合并后持久化，200 `{"added":K,"updated":M}` |
| * | 其他 | 404 |

POST 语义（自动合并 = `ImportMode::Merge`）：同 ID 覆盖、新 ID 追加、`activeId` 不变；写前走 5 份备份轮换；来源取 `X-AgentPocket-Source` 头（推送方主机名，纯标注），合并结果写一行 stdout 日志（供 journal 排查，不做通知）；body 非法 → 400 不落盘。

## 5. 发现（daemon::discovery）

1. `tailscale status --json` 解析 `Self`/`Peer`；过滤 `Online == true`，排除 `Self`。
2. 并发探测各 peer IPv4 `48720/info`：连接超时 400ms，总预算 ~3s。
3. `app == "agentpocket"` 即 mesh 节点，其余自然过滤。

CLI 查找顺序：`PATH` → Linux `/usr/bin/tailscale`、`/usr/local/bin/tailscale`（首版仅 Linux）。找不到时 `peers` 输出警告并只列手动 peer。

手动 peer：`~/.local/share/com.local.agentpocket.desktop/peers.json`，格式 `{"peers":[{"name":"…","host":"…"}]}`（host 为 100.x IP 或 MagicDNS 名），与发现列表按 host 去重。

## 6. 命令面

| 命令 | 行为 |
|---|---|
| `agentpocket serve` | 前台守护：mesh 端点 + 自动更新循环。systemd unit 拉起此命令。 |
| `agentpocket peers` | 发现并列出 peer（名称/host/版本/在线）。 |
| `agentpocket pull <host>` | 拉取并应用：默认合并；`--replace` 替换；`--dry-run` 只打印预览。 |
| `agentpocket push <host>` | 推送本机配置（带 `X-AgentPocket-Source`）。 |
| `agentpocket status` | 一次性探测已配置服务器：在线/版本/忙碌会话数（依赖 protocol 模块抽取的可行性，见 §12 备注）。 |
| `agentpocket update` | 手动触发更新检查并自更新。 |
| `agentpocket version` | 打印版本。 |

- pull/push 的 host 可用 100.x IP 或 MagicDNS 名；端口恒 48720。

## 7. 自动更新（daemon::update）

- 检查时机：`serve` 启动后 + 每 24h 一次；`update` 命令手动触发。
- 流程：`GET https://api.github.com/repos/npu-chenlin/AgentPocket/releases/latest` → `tag_name` 解析 semver → 比当前新则下载资产 `agentpocket-<arch>-linux-musl`（arch 映射 x86_64/aarch64）→ 写同目录临时文件 + fsync + chmod 755 → 原子 rename 覆盖自身路径 → systemd 环境下 `systemctl restart agentpocket`，非 systemd 打印"请手动重启"。
- 失败处理：任何失败（网络/校验/权限，如手动以非 root 运行无权覆盖 /usr/local/bin）仅记日志，下一周期重试；绝不让更新逻辑拖垮 mesh 服务。
- 无配置项：默认开启（需求本身即"支持自动更新"），不做开关。
- 版本比较：semver 严格大于才更新（防降级）。

## 8. 一键安装（scripts/install.sh）

用法：`curl -fsSL https://raw.githubusercontent.com/npu-chenlin/AgentPocket/main/scripts/install.sh | sudo bash`

- 依赖：curl + POSIX sh；探测架构（x86_64/aarch64）。
- 从 GitHub latest release 下载对应 musl 二进制 → `/usr/local/bin/agentpocket`。
- 写 `/etc/systemd/system/agentpocket.service`：`ExecStart=/usr/local/bin/agentpocket serve`；`User=` 取 `SUDO_USER`（无则 root），`Environment=HOME=` 指向该用户 home（保证配置路径与手动 CLI 一致）；`Restart=on-failure`。
- `daemon-reload` + `enable --now`；打印后续提示（`agentpocket peers` / `agentpocket pull <桌面host>`）。
- `--uninstall`：停用并删除 unit 与二进制，保留配置目录。
- 下载失败打印直链供手动处理。

## 9. 边界情况

- **与 GUI 同机共存**：端口无冲突（48720 vs GUI 旧同步随机端口）；`config.json` 共享，daemon 收到推送落盘后 GUI 需重启（或下次加载）才能看到，可接受。
- 两个 daemon 同机竞争 48720：后启动者报错退出。
- tailscale 在线但探测超时（非 AgentPocket 机器/防火墙）：不进列表，无噪音。
- 新服务器冷启动（主用例）：一键脚本装好 → `agentpocket pull <桌面host>` → 配置完成。
- 自动更新中断电/崩溃：rename 原子性保证旧新二选一完整存在；systemd 拉起恢复。

## 10. 安全模型（明确接受）

- mesh 端点无鉴权：tailnet 内任何设备可读全量配置（含 kimi/dsh token）、可推送合并。
- 防线是网络边界：`100.64.0.0/10` 之外一律 403。
- 自动合并只增不删 + 5 份备份，最坏情况可回滚。
- 自动更新仅信任本仓库 GitHub releases，semver 防降级。
- README 如实陈述：mesh 端点仅在你的 Tailscale 网络内可达。

## 11. 测试

- core 抽取后：GUI 现有 Rust 测试全部保持绿（shim 转发不改行为）。
- daemon：`is_peer_allowed` 矩阵；`/info`/`GET`/`POST` 与 403/404/400；合并计数/备份/`X-AgentPocket-Source` 日志；tailscale JSON fixture；探测过滤（agentpocket 应答/异物/超时）；client 手写 HTTP 解析；semver 比较与更新流程（mock release JSON + 本地文件替换路径，不真连 GitHub）；命令行参数与输出。

## 12. 版本、发布与备注

- 版本 2.8.0 起；release 新增资产 `agentpocket-x86_64-linux-musl`（aarch64 可选）。musl 静态编译（`x86_64-unknown-linux-musl`）。
- `status` 命令依赖 `desktop/src-tauri` 的 protocol REST 客户端抽取：若模块耦合 tauri 则本期降级为仅 kimi `/api/v1/meta` + sessions 计数的手写实现，不强行抽取（Phase 2 随 core 扩大顺带解决）。
