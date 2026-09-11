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
| `agentpocket mcp` | 前台运行 stdio MCP server（MCP 客户端拉起的入口，一般不用手敲） |
| `agentpocket mcp install` / `status` | 把本机 agentpocket 登记进 Kimi Code 的 `~/.kimi-code/mcp.json`；查看登记状态 |

## 配置与共享

Daemon 与同机同用户的 Desktop 共用服务连接配置目录 `~/.local/share/com.local.agentpocket.desktop/config.json`。Kimi Code 配置位于 `~/.kimi-code/config.toml`，两者是不同文件，也对应两种不同的同步功能。

## MCP 工具（把机群暴露给 Agent）

`agentpocket mcp` 在前台运行一个 stdio MCP server，让 Kimi Code 这类 MCP 客户端能够**在其他服务器的 Kimi 上创建会话并下发任务**。它读本机共享配置里的地址与 token，**直连目标的 Kimi Web API**，不经过 mesh 端点，因此不新增未鉴权访问面。

它属于**客户端侧**能力，要跑在 Kimi Code 所在的那台机器上。获取这个二进制有两条路：桌面端装 `deb` 就已经带上（`/usr/bin/agentpocket`）；节点侧用 `scripts/install.sh` 装（`/usr/local/bin/agentpocket`）。

接入本机 Kimi Code，直接跑：

```shell
agentpocket mcp install     # 写 ~/.kimi-code/mcp.json；幂等，改动前备份为 .bak
agentpocket mcp status      # 查登记状态、以及登记的路径是否还在
```

它会写入下面这样的条目（`command` 用安装后的绝对路径，避免 PATH 差异）。想手写也行：

```json
{"mcpServers":{"agentpocket":{
  "command":"/usr/bin/agentpocket","args":["mcp"],
  "toolTimeoutMs":300000,
  "env":{"AGENTPOCKET_MCP_MAX_WAIT_SECONDS":"240"}}}}
```

三个工具：

| 工具 | 作用 |
| --- | --- |
| `list_targets` | 只读本地配置，列出可派发的目标（不发网络请求） |
| `ask_remote` | 在目标服务器创建会话并下发 prompt；未在等待预算内完成则返回 `session_id` 供后续查询 |
| `check_remote` | 查询某个已派发会话的状态与输出摘要 |

环境变量：

| 变量 | 作用 |
| --- | --- |
| `AGENTPOCKET_MCP_ALLOW` | 逗号分隔的服务器 id/name 白名单；不设则放行全部已配置的 Kimi 服务器 |
| `AGENTPOCKET_MCP_MAX_WAIT_SECONDS` | 单次等待上限，默认 45 秒；要更久需与 mcp.json 的 `toolTimeoutMs` 配对使用 |
| `AGENTPOCKET_MCP_RECORD_TTL_HOURS` | 关联记录的保留时长，默认 6 小时 |
| `AGENTPOCKET_MCP_MODEL` | 默认模型（形如 `provider/name`）；不设时在目标服务器的可用模型里自动挑 |

远端模型必须落到一个具体值上：**API 建出来的会话不会自动绑定模型**（建会话时传 `agent_config.model` 也无效，会话里仍然是空），不指定的话远端回合会以 `[model.not_configured] Model not set` 秒失败。`ask_remote` 的 `model` 省略时，先看 `AGENTPOCKET_MCP_MODEL`，再去目标服务器的 `GET /api/v1/models` 里自动挑一个（优先 Kimi Code 托管模型），选中的值回显在返回的 `model` 字段里。想知道某台能选什么，用 `list_targets` 带 `server` 参数。

每次 `ask_remote` 会在配置目录追加一条 `remote-calls.json` 记录（服务器、会话 id、任务摘要、过期时间），仅供本地审计，权限 0600。

摘要里 `status` 的取值：`running`（在跑，带 `session_id`，可用 `check_remote` 继续问）、`idle`（出结果了，`excerpt` 是最后一段输出）、`failed`（远端回合失败，`reason` 给出原因）、`awaiting_approval` / `awaiting_question`（在等人，需要人打开 `url` 处理）。

状态判定有一处刻意的保守：**没有证据就不宣称完成**。远端刚收到 prompt、还没被领取时 `busy` 同样是 false，此时宁可继续报 `running`，也不把"还没开始"报成"已完成"。判定失败会顺带把远端原因带出来，例如 `[model.not_configured] Model not set`（说明那台没配模型）。

刻意不提供的能力：不做 `list_sessions`（避免各服务器的会话状态互相推来推去）、不代答远端审批/提问、不取消远端任务、不订阅 transcript、不支持 dsh 后端。远端会话卡在审批上时，工具只报 `awaiting_approval`，需要人打开返回的链接处理。

## 安全边界

- Daemon 节点端点监听 `0.0.0.0`，但只允许 Tailnet/回环访问；节点间通信为明文 HTTP，无独立 token 鉴权。
- 因此必须把 Tailnet 视为完整信任边界，不要把 mesh 端口暴露到公网或不可信局域网。
- `pull` 会覆盖本地 Kimi 配置，但会先保留 `config.toml.bak`；执行前可用 `--dry-run` 预览。
- Kimi Web 的管理模式可能监听 `0.0.0.0` 并关闭服务认证，仅应在可信 Tailnet 中启用。
- `kimi --upgrade` 会探测并清理 npm 安装，再执行官方安装流程；升级前确认目标节点和当前服务用户。
- `agentpocket mcp` 会把本机保存的服务器 token 交给 MCP 客户端所在的那个主机进程，等于让它能代表你在这些服务器上建会话、发任务。只应在信任该客户端（例如本机的 Kimi Code）时启用；`AGENTPOCKET_MCP_ALLOW` 可把范围收紧到指定服务器。
