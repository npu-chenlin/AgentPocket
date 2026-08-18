# AgentPocket 细粒度任务通知设计

- 日期：2026-08-18
- 状态：已批准（2026-08-18）
- 相关代码：`app/src/main/java/com/local/kimiapp/`

## 背景与目标

AgentPocket 已通过 `ServerMonitor` 常驻连接 dsh 的 `/api/events.mux`（WebSocket 纯下行流），但目前只消费 4 类信号（回合完成 / 回合失败 / 审批请求 / 提问请求），用于健康监测与系统通知。

本设计目标：在现有 events.mux 对接基础上**加深事件消费粒度**，把工具调用、增量内容、步骤推进等信息聚合成**一条实时更新的系统通知**，让用户在手机锁屏/后台时能随时看到"电脑上的 agent 正在干什么"；工具失败则单独即时提醒。**不改变现有完成/失败/审批/提问通知行为。**

## 现状（已存在）

- `DshServerMonitor` 连接 `ws://…/api/events.mux`，解析帧：`session/event`（turn/start、turn/end、session/title）、`session/projection`（title）、`approval/requested`、`question/requested`。
- `KimiServerMonitor` 对等：`event.session.status_changed`（complete/approval/question/aborted）、`event.session.work_changed`、`event.session.created/updated/deleted`、agent 事件 `prompt.completed/prompt.aborted`。
- 通知由 `KeepAliveService.postTask()` 发出（TASK_CHANNEL，IMPORTANCE_HIGH），点击经 `EXTRA_SERVER_ID` + `EXTRA_SESSION_ID` 直达会话；App 在前台（`MainActivity.isVisible`）时不打扰。

## 事件格式依据

dsh `session/event` 帧的 `event.type` 取值（实测 + 参考 DeepSeek-phone-harness 的 `dsh-utils.js` 归一化）：

| 类型 | 关键字段 |
|---|---|
| `assistant/chunk` | `data.chunk.type` ∈ {`text-delta`(text), `reasoning-delta`(text), `tool-call-delta`(name), `block-start`, `usage`} |
| `assistant/message` | `data.message.content[].text` |
| `user/message` | `data.content[].text` |
| `tool/call` | `data.name`、`data.arguments`（JSON 字符串）、`callId` |
| `tool/result` | `message.content[]`（双层 tool-result 块）、`isError`、callId 在 `message.source.callId` 或内层 |
| `step/start` / `step/end` | `data.step`（编号） |
| `turn/end` | `data.reason.kind` ∈ {completed, error, aborted, blocked, max-tokens, interrupted} |

## 设计

### 1. 会话活动状态（`DshServerMonitor` 内新增）

每会话维护一个 `SessionActivity` 对象：

```java
static final class SessionActivity {
    String currentTool;      // 最近 tool/call 的工具名
    String toolSummary;      // 参数摘要（截断 ~60 字符）
    StringBuilder deltaText; // 本回合 assistant 增量文本缓存（截断 ~200 字符）
    String lastToolError;    // 最近工具失败信息（用于失败通知）
    int step;                // 当前步骤编号
    boolean inTurn;          // 回合进行中（turn/start 置 true，turn/end 置 false）
}
```

存储在 `Map<String, SessionActivity>`（sessionId → activity），随 `session.list` 刷新清理。

活动状态变化时通过 `MonitorHost.onActivityChanged` 上报**只读快照**（`currentTool / toolSummary / deltaText / step / inTurn / lastToolError`），由 `KeepAliveService` 决定是否更新聚合通知；monitor 不直接接触通知。

### 2. 事件解析扩展（`handleSessionEvent`）

在现有 `switch (type)` 上新增分支：

- `assistant/chunk`：
  - `text-delta` → 追加 `deltaText`（超 200 字符截断），触发聚合通知刷新
  - `tool-call-delta` → `currentTool = chunk.name`，触发刷新
  - `reasoning-delta` → 标记"思考中"（`currentTool = null` 时正文显示"正在思考…"）
- `tool/call`：`currentTool = data.name`；`toolSummary = 参数摘要`（从 `arguments` JSON 提取路径/命令字段，截断 60 字符）；触发刷新
- `tool/result`：若 `isError == true` → **立即单独发"工具失败"通知**（不节流），tag = `toolerr:{sessionId}:{callId}`（防重复）；结果文本进 `lastToolError`（截断）
- `step/start`：`step = data.step`，触发刷新

### 3. 聚合通知（`KeepAliveService` 新增）

- **tag**：`activity:{sessionId}`
- **channel**：复用 `TASK_CHANNEL`
- **触发**：App 在后台（`!MainActivity.isVisible`）且会话回合进行中
- **内容模板**（`setStyle(BigTextStyle)`）：
  - 第一行：`[服务器名] 正在执行 {toolName}`（无工具时"正在生成回复…"/"正在思考…"）
  - 第二行：`{deltaText 或 toolSummary}`（截断 60 字符）
- **节流**：`Handler.postDelayed` 合并更新，两次刷新间隔 ≥ 1 秒；回合结束（turn/end）时**立即**执行最终更新并替换（不走节流）
- **生命周期**：
  - turn/start（或首个 tool/call）：创建聚合通知（仅后台）
  - 回合中：节流更新
  - turn/end：**取消聚合通知**（`notificationManager.cancel("activity:" + sessionId)`），随后走现有 `notifyTurnFinished` / `maybeNotify` 发完成/失败通知——避免"聚合通知"与"完成通知"双份打扰
  - App 回前台（`MainActivity.onResume`）：取消该 server 下所有 `activity:*` 聚合通知；`isVisible` 变化时由现有逻辑衔接
- **点击**：复用 `postTask` 的 Intent 构造（`EXTRA_SERVER_ID` + `EXTRA_SESSION_ID`），直达对应会话

### 4. 失败单独提醒

- 触发：`tool/result` 且 `isError == true`
- tag：`toolerr:{sessionId}:{callId}`（同一工具失败不重复刷）
- 内容：`[服务器名] 工具执行失败：{toolName}` + 错误摘要（截断 120 字符）
- 仅后台（`!MainActivity.isVisible`）时发

### 5. 悬浮球 / 表情链路

不动。`publishEvent("complete"/"aborted"/...)` 与 `last_event` 状态保持现有行为。

## 边界与限制

- **细粒度通知仅对 dsh 后端生效**。kimi 后端事件协议不同（`event.session.*` 无工具级/增量事件），保持现有完成/失败/审批/提问通知不变。`KimiServerMonitor` 不新增解析。
- `tool/result` 的双层 `message.content[].content[]` 结构与 `callId` 位置以真机实测为准（参考 phone-harness 归一化实现，需在真实 dsh 会话中验证）。
- 聚合通知仅在 App 后台创建，前台静默（延续现有策略）。
- 节流 1s 为默认值，可在实现时按体验微调。

## 改动清单

| 文件 | 改动 |
|---|---|
| `DshServerMonitor.java` | 新增 `SessionActivity` 状态类与 map；`handleSessionEvent` 新增 `assistant/chunk`、`tool/call`、`tool/result`、`step/start` 分支；activity 变化时回调 `host.onActivityChanged(serverId, sessionId, snapshot)` |
| `KeepAliveService.java` | 新增聚合通知管理器（创建/节流更新/取消）、`postToolError`；实现 `MonitorHost.onActivityChanged` |
| `ServerMonitor.java` | `MonitorHost` 接口新增 `onActivityChanged(String serverId, String sessionId, SessionActivitySnapshot)` 默认空实现 |
| `MainActivity.java` | `onResume` 时通知 Service 取消聚合通知（小改） |

无新增依赖（okhttp 已存在）。

## 验证

1. 构建 release 并安装到手机（`JAVA_HOME=/home/user/software/jdk17` + gradle `:app:assembleRelease` + `adb install -r`）
2. 连真实 dsh，发起一个会调用工具（如文件编辑/命令执行）的任务：
   - 后台时：看到聚合通知逐步更新（工具名 → 增量文本 → 完成）
   - 制造一次工具失败：确认单独失败通知出现且不重复
   - 回前台：聚合通知消失
   - 点聚合通知：直达对应会话
3. kimi 后端回归：现有通知行为不变
