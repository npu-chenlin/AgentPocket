# Session Relay 设计（@别名 跨会话/跨服务器对话）

日期：2026-09-08
状态：设计已获用户口头确认，待实施

## 1. 背景与目标

参考 cc-connect 的 relay 模式（聊天平台做总线、agent 是 bot、@提及路由），
在 AgentPocket 体系内实现：**在任意 kimi session 中 `@别名` 另一个 session，
两个 agent 可以相互调用与对话，包括跨服务器**。

已验证的技术前提（2026-09-08，本机 kimi server 0.41.0）：

- `POST /api/v1/sessions/{id}/prompts` 官方支持程序化投递 prompt；
  目标 session 正忙时服务端自动排队（active/queued），另有 `prompts:steer`。
- 本机回环免鉴权可直接调用该 API（以不存在 session 试探，返回 40401
  "session does not exist"，说明请求已通过鉴权层）。
- `GET /api/v1/sessions/{id}/prompts` 可观察 active prompt 状态，足以检测
  回合结束；`GET /messages` 可拉取最终 assistant 回复。
- kimi 支持 MCP（capabilities.mcp=true，本机已挂 amap/context7 等，其中
  amap 为 streamable HTTP transport，可作 daemon MCP 端点的格式参考）。
- daemon mesh 已有 tailscale 100.x 网段白名单互信的 HTTP 端点
  （/info、/kimi-info、/kimi-web-enable、/kimi-web-restart 等）。

## 2. 非目标（YAGNI）

- 群聊 UI（cc-connect 式聚合界面）
- @时自动新建 session
- 文件/附件转传
- 独立 GUI 别名管理页（仅在现有会话下拉菜单挂一个"设为 @别名"动作）

## 3. 架构（方案 A：daemon 路由 + MCP 工具）

```
session A (任意机器)                session B (另一台机器)
    │ 用户写 "@gpu/refine ..."          │
    ▼                                  │
agent 调用 MCP 工具 relay_ask            │
    │                                  ▼
本机 daemon ──(本机session? 直接POST prompts API)
    │                                  │
    └──(跨机?)──mesh HTTP──▶ 对端 daemon ──▶ POST 对端 kimi API
                                   │
                     B 回合完成(B侧daemon轮询检测)
                                   │
              wait=true  ──发起端长轮询取结果，阻塞返回给 A 的工具调用
              wait=false ──B侧 daemon 回调 A 侧，注入回复触发 A 新一轮
```

核心原则：

- **token 不出机**：每台 daemon 只访问本机 kimi；跨机仅走 mesh 白名单信任。
- **不依赖 GUI 在线**：路由完全在 daemon；手机端零改动即可看到对话效果。
- **无 pending 时零请求**：不做常驻订阅，只有存在待完成投递时才轮询目标。

## 4. 组件设计

### 4.1 daemon `relay.rs` 模块

- **别名表** `relay.toml`（daemon 数据目录
  `~/.local/share/com.local.agentpocket.desktop/`，与 GUI config.json 同目录）：
  `refine -> { server: "local" | "<peer名>", session_id, note }`。
  本机别名可省 server 前缀；跨机写 `@gpu/refine`。
  CLI 维护：`agentpocket relay add/list/rm`。

  **交付原则：用户只在 web 对话中说自然语言，不接触 CLI。** 别名表是
  可选的覆盖层（改名/固定标题/消歧），不是必经注册步骤：

  - **零注册默认**：`relay_ask` 的 target 解析顺序为——别名表精确命中 →
    本机 session 标题子串匹配 → （P2）peer 标题匹配；唯一命中即投递，
    歧义时把候选（标题+短id+状态）返回给 agent 转问用户。
  - **会话内注册**：MCP 工具 `relay_register(alias)`（见 4.2），用户在
    目标会话里说"把你注册成 @xxx"，agent 调工具完成。
  - **AgentPocket 菜单**：桌面/手机"活跃会话"下拉菜单加"设为 @别名"
    （复用现有会话菜单，调 daemon 同一接口）。
  - CLI 仅面向服务器脚本场景保留。session_id 获取（CLI/内部路径）：
    `--session` 接受完整 URL（`http://host:58627/sessions/session_xxx`）
    或裸 `session_xxx`；`--latest` 取最近活跃 session；`--title <子串>`
    模糊匹配；add 均校验存在并回显标题。

- **pending 表**（同目录 `relay-pending.json`）：每条投递记录
  `{delivery_id, from_peer, from_session, to_alias, to_session, wait, hops,
  created_at, status}`；daemon 重启后恢复继续轮询。
- **完成检测**：存在 pending 时，2s 间隔轮询目标 `GET /prompts`，
  active prompt 从 running 消失即完成；再拉最后一条 assistant 消息为回复。

### 4.2 MCP 工具（daemon 监听 127.0.0.1 streamable HTTP MCP 端点）

- `relay_list()`：按解析顺序列出可 @ 的目标（别名 + 未绑定的会话标题），
  及各自状态（idle / 忙碌 / peer 离线）。
- `relay_ask(target, text, wait=true, timeout_secs=600)`：
  - 工具描述中写明约定：**用户消息出现 `@别名` 时，把内容通过本工具转发**。
  - `wait` 由 agent 自行决定（与 Bash 工具 `run_in_background` 同一心智模型）。
  - target 即 4.1 的解析顺序；歧义时返回候选列表。
- `relay_register(alias)`：注册**调用方 session 自身**——daemon 以最近活跃
  session（`main_turn_active=true` 优先，`updated_at` 最新）判定调用方，
  因工具调用发生时调用方会话必然正在写入。
- MCP 注册写入 kimi config（具体配置格式在实现首日对齐，参考本机
  amap-maps-streamableHTTP 的 http transport）。MCP 的自描述性使 agent
  从工具列表自行发现 relay 能力，无需系统提示注入。

### 4.3 mesh 端点扩展（`mesh.rs`）

两段式，避免长阻塞占用 tiny_http 线程：

- `POST /relay/deliver`：投递到本机 kimi，立即返回 `delivery_id`。请求体
  携带回址（发起端 peer 名），供异步回投使用——无状态，不依赖对端 peer 表。
- `GET /relay/wait?delivery_id=&timeout=`：30s 长轮询，B 完成即返回最终回复。
- `POST /relay/callback`：异步模式下 B 侧 daemon 主动回调 deliver 携带的
  回址，由 A 侧 daemon 把 B 的回复注入 A 的 session。

## 5. 关键行为

- **同步（wait=true）**：工具阻塞至 B 回合完成，返回 B 最终回复；
  默认超时 10 分钟，超时返回"B 仍在运行（delivery_id=xx），可等待回投"。
- **异步（wait=false）**：立即返回回执；B 完成后回调注入 A，注入格式：
  `[relay] @gpu/refine 回复了你（delivery_id=xx）：\n\n<最终回复>`，
  触发 A 新一轮，agent 自然接手。
- **目标正忙**：kimi 自动排队，照常等待。
- **B 等待人工审批**：超时后明确告知 A "B 在等人工审批"，不无限挂起。
- **循环防护**：
  - 投递内嵌 hop 计数，`hop >= 3` 拒绝投递并回错误；
  - 同一对会话同时仅允许 1 条 in-flight；
  - 注入的回投 prompt 带 `[relay]` 前缀，agent 可识别这不是人类输入。

## 6. 错误处理

| 场景 | 行为 |
|------|------|
| target 无命中 | `relay_ask` 报错并返回 `relay_list` 摘要，提示 agent 转问用户 |
| target 歧义（多标题命中） | 返回候选列表（标题+短id+状态）由 agent 转问用户 |
| 对端 daemon 不可达 | 返回 "peer 离线" |
| 目标 session 不存在 | 返回 kimi 原始错误码与信息 |
| daemon 重启 | pending 表持久化，恢复后继续轮询 |
| 超时 | 见上文；delivery_id 保留供后续回投 |

## 7. 测试策略

- 单元：别名解析（前缀省略规则）、hop 判定、pending 状态机、注入格式。
- 集成：本机 58627 起两个真实 session，互 @ 全链路（同步/异步/排队/超时）。
- 手动：真实跨机（gpu 服务器）验证 mesh 两段式。

## 8. 分期

- **P1（本机）**：relay.rs + 别名 CLI + MCP 工具 + 本机投递/回投，本机两
  session 互 @ 跑通。
- **P2（跨机）**：mesh 三端点 + hop 防护 + peer 离线处理，真实双机验证。

## 9. 实现时需确认的开放项

- kimi config 注册 streamable HTTP MCP server 的准确配置段格式。
- `GET /prompts` 在 queued 状态下的返回细节（决定轮询状态机的转移条件）。
