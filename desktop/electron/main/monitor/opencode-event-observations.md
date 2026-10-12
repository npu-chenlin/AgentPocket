# opencode v2 事件类型观测清单

这份清单是对**真实运行的服务**订阅 `/api/event` 录下来的，不是从 openapi 抄的
——openapi 不可信：它把 SSE 信封的判别字段写作 `event`，实测发的是 `type`；
`x-effect-stream.causeSchema` 只枚举了 `Fail` / `Die` / `Interrupt` 三种错误态，
不是完整事件联合。

采集时间：2026-10-12，服务 `pilab opencode`（`100.115.54.102:4096`）。
采集方式：订阅 SSE，按事件类型去重，逐字记录完整 JSON。
覆盖的场景：一次常规任务（含工具调用、shell、报错重试）+ 一次 `form` 提问与答复。

## 已被 monitor/opencode.ts 处理

| 事件 | 用途 |
| --- | --- |
| `session.execution.started` | 置为忙碌 |
| `session.execution.succeeded` / `.failed` | 清除忙碌 |
| `session.tool.input.started` | 活动行：工具名 |
| `session.tool.input.ended` | 活动行：拼上命令预览 |
| `session.tool.success` / `.failed` | 清掉 currentTool |
| `session.created` / `.updated` | 会话标题 |
| `session.deleted` | 清掉该会话全部状态 |
| `form.created`（`metadata.kind === "question"`） | 产出「待回答」事件 |
| `form.replied` | 清除待回答 |

`session.execution.*` 的负载是 `{sessionID}`；`form.*` 的会话 id 在
`data.form.sessionID` 上（`form.replied` 才在 `data.sessionID`）。

## 已观测到但有意不处理

噪声类，对功能无影响，解析时静默跳过：

`server.connected`、`session.usage.updated`、`session.text.started` / `.delta` /
`.ended`、`session.reasoning.started` / `.delta` / `.ended`、`session.step.started`
/ `.streamed` / `.ended`、`session.tool.called` / `.progress`、`session.inbox.enqueued`
/ `.delivered`、`session.agent.selected`、`shell.created` / `.exited` / `.deleted`、
`project.updated`、`model.updated`、`provider.updated`、`command.updated`、
`agent.updated`、`skill.updated`、`plugin.updated`、`reference.updated`、
`websearch.updated`、`integration.updated`、`models-dev.refreshed`

## 已知缺口

以下四条在代码里也有对应注释，改动前先看这份清单。

- **审批（`approvalRequired`）未接**。opencode 的审批走 `Permission.Request`
  （`id` 以 `per` 开头，字段 `action` / `resources` / `message`），与提问的
  `Form.Detail` 是两套机制，回复分别是 `once`/`always`/`reject` 与 `Form.Reply.answer`。
  本次采集**没有观测到**承载 permission 的事件——该服务权限全开，
  `Permission.Effect` 的 `ask` 分支不会触发。将来若开启审批，需要重新采集
  事件名后再补，当前不会有通知。
- **`session.step.ended`（`finish: "tool-calls"`）未映射**。Kimi 有
  「主 agent 已完成 · 等 N 个后台任务」的分层语义，opencode 的 step 粒度不同，
  没有证据支持直接类比，故未实现。
- **`shell.*` 未映射为后台任务**。`shell.created` 带 `status: "running"` 与
  `command`，语义上像 Kimi 的后台任务，但未经证实。
- **重连后活动行会丢**。忙碌能用 `/api/session/active` 重建，活动行不能
  （没有权威端点）。Kimi 也有同样特性。