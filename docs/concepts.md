# 概念与术语

AgentPocket 同时涉及远程 Web 服务和运行这些服务的机器。下面的术语用于区分它们。

| 术语 | 定义 | 示例 |
| --- | --- | --- |
| 节点 | 一台安装 AgentPocket daemon、可被发现和管理的物理机或虚拟机 | 一台 Linux 工作站或无显示器服务器 |
| Agent 服务 | 节点上运行、提供 Web 界面的 Coding Agent 服务 | Kimi Code Web、DeepSeek Harness Web |
| 服务连接 | Android/Desktop 保存的一条连接记录，包含名称、主机、端口、后端和可选 token | `100.64.0.2:58627` 的 Kimi 记录 |
| 会话 | Agent 服务中的一段对话或工作区上下文 | Kimi 的一个 session、dsh 的一个 session |
| 任务 / 回合 | 用户发起的一次执行，以及它的完成、失败、等待回答或等待审批状态 | 一次代码修改请求 |
| 节点间 Kimi 配置 | 节点用户目录中的 `~/.kimi-code/config.toml` | 模型、供应商等 Kimi Code 配置 |

## 两种同步

### 手机配对 / 服务连接同步

Android 和 Desktop 在同一可达网络中，通过二维码互传服务连接列表。它解决的是“另一端如何知道 Agent 服务地址和凭据”，不负责复制 Kimi Code 配置，也不要求安装 daemon。

### 节点间 Kimi 配置分发

Daemon 提供节点端点，Desktop 可以发现节点并拉取或推送 `~/.kimi-code/config.toml`。它解决的是“多台机器如何使用相同的 Kimi 配置”，不负责同步 Android/Desktop 的服务连接列表。

## 边界

- Agent 服务由 Kimi Code 或 dsh 提供；AgentPocket 只是客户端、监控器和节点运维工具。
- 节点可以运行 Agent 服务，但“节点”和“Agent 服务”不是同一个对象。
- 一个节点可以有多个 Agent 服务；一个客户端也可以保存多个服务连接。
- 状态监控展示 Agent 服务报告的在线状态和活跃会话，不等同于对节点操作系统健康状况的完整监控。
