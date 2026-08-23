# AgentPocket

AgentPocket 是一个通过 Tailscale 连接 Coding Agent 的多端工具：用 Android 或桌面端访问远程 Agent 服务、关注任务状态，并在多台节点之间维护 Kimi Code 环境。

AgentPocket 不运行 Coding Agent 本身，也不替代 Kimi Code 或 DeepSeek Harness；它负责连接、提醒、配置同步和节点维护。

## 核心用户旅程

1. 在运行 Kimi Code 或 DeepSeek Harness Web 的电脑上安装并登录 Tailscale。
2. 启动 Agent 服务，将它绑定到手机可访问的地址。
3. 在 Android 或 Desktop 中添加一个“服务连接”（主机、端口、后端和可选 token）。
4. 从手机或桌面打开 Agent 服务中的会话；Agent 任务完成、失败、等待回答或等待审批时，查看通知并回到对应会话。
5. 有多台电脑时，在节点管理中发现节点，并按需分发 Kimi 配置。

## 功能地图

| 功能域 | 解决的问题 | 包含能力 |
| --- | --- | --- |
| 使用 Agent | 随时进入远程 Coding Agent | 打开 Kimi/dsh、切换服务连接、进入活跃会话 |
| 关注任务 | 任务有结果或需要介入时及时回来 | 后台监听、在线与活动状态、完成/失败/回答/审批通知 |
| 管理连接 | 减少重复填写地址和凭据 | 服务连接增删改、后端识别、导入导出、手机配对 |
| 管理节点 | 维护多台运行 Kimi 的机器 | 节点发现、Kimi 配置分发、CLI 升级、Kimi Web 管理 |
| 应用维护 | 让各端可靠常驻和更新 | 托盘、开机启动、应用更新、daemon 安装与自更新 |

## 三个组件

| 组件 | 适合谁 | 主要能力 |
| --- | --- | --- |
| Android | 需要移动访问的人 | 内嵌 Agent Web、多服务连接、后台状态监听、任务通知、悬浮入口、扫码同步服务连接 |
| Desktop | 需要在电脑上常驻管理的人 | 系统托盘、状态监控、通知、浏览器跳转、服务连接导入导出、手机配对、节点管理 |
| Daemon | 管理无显示器或多台 Linux 机器的人 | 节点发现、Kimi 配置分发、Kimi CLI 管理、Kimi Web 生命周期、自更新 |

术语和边界见 [概念与术语](docs/concepts.md)，各端具体能力见 [功能矩阵](docs/feature-matrix.md)。

## 快速开始

### 前提

电脑和 Android 手机需要安装 [Tailscale](https://tailscale.com/download)，并登录同一个 Tailnet。电脑上还需要安装并启动一种 Agent 服务。

Kimi Code（macOS / Linux）：

```shell
curl -fsSL https://code.kimi.com/kimi-code/install.sh | bash
kimi web --dangerous-bypass-auth --host 0.0.0.0 --port 58627
```

`--dangerous-bypass-auth` 会关闭 Agent 服务自己的访问认证，只应在可信的 Tailscale 网络中使用。

DeepSeek Harness（需要 Node.js 18+）：

```shell
npx -y @deepseek-ai/dsh web --trusted-host <Tailscale IP>
```

国内网络可使用 npm 镜像：

```shell
npx -y --registry=https://registry.npmmirror.com @deepseek-ai/dsh web --trusted-host <Tailscale IP>
```

dsh 默认只监听 `127.0.0.1`。若手机不能直接访问，请在电脑上将 Tailscale 地址转发到本地端口，例如：

```shell
socat TCP-LISTEN:3080,bind=<Tailscale IP>,reuseaddr,fork TCP:127.0.0.1:3080
```

在 AgentPocket 中添加服务连接，例如 Kimi 使用 `http://<Tailscale IP>:58627`，dsh 使用 `http://<Tailscale IP>:3080`。

### 安装与构建

预构建安装包可从 [GitHub Releases](https://github.com/npu-chenlin/AgentPocket/releases) 下载。Android 的使用和构建说明见 [Android](docs/android.md)；Desktop 的安装与构建见 [Desktop](docs/desktop.md)；节点守护进程见 [Daemon](docs/daemon.md)。

## 配置同步的两个含义

- **手机配对 / 服务连接同步**：Android 与 Desktop 通过二维码互传保存的服务连接列表。导出文件可能包含 token，不要公开分享。
- **节点间 Kimi 配置分发**：Daemon 或 Desktop 节点面板在 Tailnet 内拉取/推送 `~/.kimi-code/config.toml`。它不等于手机配对，也不会同步 Android 的服务连接列表。

## 安全边界

- AgentPocket 是非官方客户端，与 Moonshot AI/Kimi、DeepSeek 官方无隶属关系。
- Tailscale 只解决网络可达性；AgentPocket 不替用户配置防火墙、端口转发或可信网络。
- Kimi 的 `--dangerous-bypass-auth`、Daemon 的节点端点和 Kimi 配置分发依赖 Tailnet 的信任边界。节点间 mesh 端点使用明文 HTTP 且无独立鉴权，只在可信 Tailnet 中使用。
- 服务连接导出、桌面配置和二维码可能包含访问凭据，请按密钥处理；不要提交到公开仓库或发送给不可信的人。
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
