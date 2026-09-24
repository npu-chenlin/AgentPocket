# AgentPocket

[![Release](https://img.shields.io/github/v/release/npu-chenlin/AgentPocket?label=release&sort=semver)](https://github.com/npu-chenlin/AgentPocket/releases)
[![License](https://img.shields.io/github/license/npu-chenlin/AgentPocket)](LICENSE)
[![Stars](https://img.shields.io/github/stars/npu-chenlin/AgentPocket?style=flat&label=stars)](https://github.com/npu-chenlin/AgentPocket)
[![Platform](https://img.shields.io/badge/platform-Android%20%7C%20Desktop%20%7C%20Daemon-informational)](docs/feature-matrix.md)

把装在你电脑上的 Coding Agent 装进口袋 —— 通过 Tailscale 用手机或桌面端随时进入远程 Agent 服务、关注任务状态，并在多台节点之间维护 Kimi Code 环境。

AgentPocket 不运行 Coding Agent 本身，也不替代 Kimi Code、DeepSeek Harness 或 OpenCode；它只负责「连接、提醒、配置同步和节点维护」这一层。

## 特性一览

| | 特性 | 说明 |
| --- | --- | --- |
| 手机形态 | 内嵌 Agent Web | 在 Android 应用里直接打开 Kimi / dsh / OpenCode，不必切浏览器 |
| 任务警觉 | 后台监听与通知 | 任务完成、失败、等待回答或等待审批时推送，点击回到对应会话 |
| 多服务器 | 服务连接管理 | 保存多套地址与凭据，识别后端类型，一键切换、扫码同步 |
| 多节点 | 节点管理 | 发现 Tailnet 内的电脑，分发 Kimi 配置、升级 CLI、管理 Kimi Web |
| 常驻 | 各端可靠运行 | 托盘常驻、开机自启、应用更新、daemon 自更新 |

支持的后端：**Kimi Code**、**DeepSeek Harness（dsh）**、**OpenCode**。

## 界面速览

| 手机端 | 桌面端 |
| --- | --- |
| ![手机端](docs/demo.jpg) | ![桌面端](docs/desktop.jpg) |

## 架构概览

```
Android / Desktop / Daemon
        │  连接（HTTP/SSE）      ┌────────────┐   拉起会话    ┌─────────────┐
        ├──────────────────────► │ AgentPocket │ ───────────► │ 你的电脑      │
        │                       │   连接层     │   配置同步   │ Kimi/dsh/OC │
        ◄────────────────────── │             │ ◄─────────── │ 节点发现/分发 │
         状态 / 通知            └────────────┘              └─────────────┘
                                        通过 Tailscale（同一 Tailnet）
```

各端角色：

| 组件 | 适合谁 | 主要能力 |
| --- | --- | --- |
| Android | 需要移动访问的人 | 内嵌 Agent Web、多服务连接、后台状态监听、任务通知、悬浮入口、扫码同步服务连接 |
| Desktop | 需要在电脑上常驻管理的人 | 系统托盘、状态监控、通知、浏览器跳转、服务连接导入导出、手机配对、节点管理 |
| Daemon | 管理无显示器或多台 Linux 机器的人 | 节点发现、Kimi 配置分发、Kimi CLI 管理、Kimi Web 生命周期、自更新 |

## 快速开始

### 前提

电脑和 Android 手机安装 [Tailscale](https://tailscale.com/download) 并登录到同一个 Tailnet。电脑上再启动一种 Agent 服务。

**Kimi Code**（macOS / Linux）：

```shell
curl -fsSL https://code.kimi.com/kimi-code/install.sh | bash
kimi web --dangerous-bypass-auth --host 0.0.0.0 --port 58627
```

`--dangerous-bypass-auth` 会关闭 Agent 服务自带的访问认证，只应在可信的 Tailscale 网络中使用。

**DeepSeek Harness**（需要 Node.js 18+）：

```shell
npx -y @deepseek-ai/dsh web --trusted-host <Tailscale IP>
```

网络受限时使用 npm 镜像：

```shell
npx -y --registry=https://registry.npmmirror.com @deepseek-ai/dsh web --trusted-host <Tailscale IP>
```

dsh 默认只监听 `127.0.0.1`。若手机不能直接访问，在电脑上将 Tailscale 地址转发到本地端口：

```shell
socat TCP-LISTEN:3080,bind=<Tailscale IP>,reuseaddr,fork TCP:127.0.0.1:3080
```

**OpenCode**（使用 [OpenCode](https://opencode.ai) 安装，需要 Node.js 18+）：

```shell
OPENCODE_SERVER_PASSWORD='你的密码' opencode serve --hostname 0.0.0.0 --port 4096
```

OpenCode 默认只监听 `127.0.0.1`；`--hostname 0.0.0.0` 让它在 Tailscale 地址上监听。
v2 起 Web 与 API 都要求登录：未设 `OPENCODE_SERVER_PASSWORD` 时服务不设防，设了之后
客户端必须用 **HTTP Basic** 认证，用户名固定为 `opencode`（`OPENCODE_SERVER_USERNAME`
当前被服务端忽略）。

### 添加服务连接

在 AgentPocket 中添加服务连接：

| 后端 | 地址示例 | token 字段 |
| --- | --- | --- |
| Kimi | `http://<Tailscale IP>:58627` | 可选访问令牌 |
| dsh | `http://<Tailscale IP>:3080` | 留空 |
| OpenCode | `http://<Tailscale IP>:4096` | 登录密码，或 `用户名:密码`（默认用户 `opencode`） |

> **OpenCode 认证**：服务端的密码是 `OPENCODE_SERVER_PASSWORD`，在 AgentPocket 的 token
> 字段填同一个密码即可（填 `opencode:密码` 或只填密码都行）。凭据由各端以 HTTP Basic
> 头发出，手机端由 WebView 的认证回调自动应答，不会弹登录框。
>
> 若 OpenCode 经 LAN/Tailscale IP 访问时侧栏会话列表为空，这是 OpenCode Web 前端的已知问题（按访问来源分键存储项目注册表）。AgentPocket 加载首页后会播种项目注册表并让 OpenCode 前端自行渲染项目与会话，远端打开即可看到历史会话，无需手动处理。

### 安装与构建

预构建安装包从 [GitHub Releases](https://github.com/npu-chenlin/AgentPocket/releases) 下载。

- Android：使用与构建见 [docs/android.md](docs/android.md)
- Desktop：安装与构建见 [docs/desktop.md](docs/desktop.md)
- Daemon：节点守护进程见 [docs/daemon.md](docs/daemon.md)

## 能力矩阵

| 功能 | Android | Desktop | Daemon |
| --- | :---: | :---: | :---: |
| 打开 Kimi / dsh / OpenCode Web | 支持 | 支持 | — |
| 服务连接管理 | 支持 | 支持 | 读取共享配置 |
| 自动识别后端 | 支持 | 支持 | 探测支持 |
| 任务完成/失败/等待通知 | 支持 | 支持 | — |
| 手机配对 / 连接同步 | 扫码 | 二维码 | — |
| 节点发现与 Kimi 配置分发 | — | 支持 | `pull`/`push` |

完整矩阵见 [docs/feature-matrix.md](docs/feature-matrix.md)。

## 配置同步的两个含义

- **手机配对 / 服务连接同步**：Android 与 Desktop 通过二维码互传保存的服务连接列表。导出文件可能包含 token，不要公开分享。
- **节点间 Kimi 配置分发**：Daemon 或 Desktop 节点面板在 Tailnet 内拉取/推送 `~/.kimi-code/config.toml`。它不等于手机配对，也不会同步 Android 的服务连接列表。

## 常见问题

- **连接失败/找不到服务？** 先确认电脑和手机在同一 Tailnet、服务地址用 `<Tailscale IP>` 而非 `127.0.0.1`。
- **dsh 手机打不开？** dsh 默认监听本机，先用 `socat` 转发，见上方快速开始。
- **OpenCode 连不上 / 一直转圈？** v2 起必须登录：确认服务端设了 `OPENCODE_SERVER_PASSWORD`，且 AgentPocket 的 token 字段填了同一个密码。
- **OpenCode 没有历史会话？** 用新版 AgentPocket 打开首页即可，项目注册表会自动播种。
- **token 会被同步吗？** 会，服务连接导出/二维码包含凭据，请按密钥对待。

## 安全边界

- AgentPocket 是非官方客户端，与 Moonshot AI/Kimi、DeepSeek、OpenCode 官方无隶属关系。
- Tailscale 只解决网络可达性；AgentPocket 不替用户配置防火墙、端口转发或可信网络。
- Kimi 的 `--dangerous-bypass-auth`、Daemon 的节点端点和 Kimi 配置分发依赖 Tailnet 的信任边界。节点间 mesh 端点使用明文 HTTP 且无独立鉴权，只在可信 Tailnet 中使用。
- 服务连接导出、桌面配置和二维码可能包含访问凭据（含 OpenCode 登录密码），请按密钥处理；不要提交到公开仓库或发送给不可信的人。
- OpenCode v2 的 Web/API 默认需要登录；AgentPocket 会带上 Basic 凭据访问被信任的服务地址，凭据不会发给其它站点。
- Daemon 安装脚本会安装 systemd 服务并使用 sudo；使用前请检查脚本内容、发布资产和目标机器。
- Windows 与 macOS Desktop 尚未进行正式发布验证。

## 文档

- [概念与术语](docs/concepts.md)：节点、Agent 服务、服务连接、会话、任务/回合和两种同步
- [Android](docs/android.md)：移动端使用、通知、悬浮入口、扫码和构建
- [Desktop](docs/desktop.md)：托盘、状态监控、服务连接管理、手机配对、节点面板和构建
- [Daemon](docs/daemon.md)：Linux 节点安装、命令、Kimi 管理和安全边界
- [功能矩阵](docs/feature-matrix.md)：Android、Desktop、Daemon 的能力边界

## License

[MIT](LICENSE)