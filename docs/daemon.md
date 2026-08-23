# Daemon（节点守护进程）

Daemon 是运行在 Linux 节点上的 AgentPocket 服务。它让 Desktop 能发现节点、分发 Kimi Code 配置，并远程维护 Kimi CLI 与 Kimi Web。

## 安装

安装脚本从 GitHub 最新 Release 下载对应架构的二进制，安装 systemd 服务并设置开机启动：

```shell
curl -fsSL https://raw.githubusercontent.com/npu-chenlin/AgentPocket/main/scripts/install.sh | sudo bash
```

脚本支持 `x86_64` 和 `aarch64/arm64`。交互式安装时可选择安装/升级 Kimi Code CLI 并生成 Kimi Web 服务。

安装前应检查脚本和 Release 资产；脚本会使用 sudo 写入 `/usr/local/bin/agentpocket`、systemd 服务和 Bash 补全文件。

## 常用命令

以下命令通常以安装 daemon 的服务用户运行；需要时使用 `sudo -u <服务用户>`，不要混用不同用户的 HOME。

| 命令 | 作用 |
| --- | --- |
| `agentpocket peers` | 发现 Tailnet 内的 AgentPocket 节点 |
| `agentpocket status` | 一次性探测本机配置的 Agent 服务 |
| `agentpocket pull <节点>` | 拉取远端 `~/.kimi-code/config.toml` 覆盖本地；旧文件备份为 `.bak` |
| `agentpocket pull <节点> --dry-run` | 预览拉取结果，不写入文件 |
| `agentpocket push <节点>` | 将本机 Kimi 配置发送给目标节点 |
| `agentpocket kimi [节点]` | 查询本机或目标节点的 Kimi CLI 版本 |
| `agentpocket kimi [节点] --upgrade` | 安装/升级 Kimi CLI |
| `agentpocket kimi-web status` | 查看 Kimi Web 状态 |
| `agentpocket kimi-web enable` | 启动并注册 Kimi Web 服务连接 |
| `agentpocket kimi-web restart [--force]` | 重启；有活跃会话时默认拒绝，`--force` 忽略保护 |
| `agentpocket kimi-web disable` | 停止并移除 Kimi Web 服务 |
| `agentpocket update` | 手动检查并更新 daemon |
| `sudo agentpocket uninstall` | 停止并移除 daemon 与服务；配置目录保留 |
| `agentpocket completions <shell>` | 输出 bash/zsh/fish 等补全脚本 |

## 配置与共享

Daemon 与同机同用户的 Desktop 共用服务连接配置目录 `~/.local/share/com.local.agentpocket.desktop/config.json`。Kimi Code 配置位于 `~/.kimi-code/config.toml`，两者是不同文件，也对应两种不同的同步功能。

## 安全边界

- Daemon 节点端点监听 `0.0.0.0`，但只允许 Tailnet/回环访问；节点间通信为明文 HTTP，无独立 token 鉴权。
- 因此必须把 Tailnet 视为完整信任边界，不要把 mesh 端口暴露到公网或不可信局域网。
- `pull` 会覆盖本地 Kimi 配置，但会先保留 `config.toml.bak`；执行前可用 `--dry-run` 预览。
- Kimi Web 的管理模式可能监听 `0.0.0.0` 并关闭服务认证，仅应在可信 Tailnet 中启用。
- `kimi --upgrade` 会探测并清理 npm 安装，再执行官方安装流程；升级前确认目标节点和当前服务用户。
