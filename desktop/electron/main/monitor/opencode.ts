/**
 * OpenCode 监控。从 desktop/src-tauri/src/monitor/opencode.rs 迁移。
 *
 * v2 没有可轮询的忙闲端点，状态完全由 SSE 事件驱动：
 * `session.execution.started` / `succeeded` / `failed`。同时兼容 v1 的事件形状。
 */

import { opencodeBasicHeader, serverBaseUrl, type ServerConfig } from "../model";
import type { AgentEvent, ProtocolState } from "../protocol/state";
import { MonitorCancelled, PinTracker, type CancelToken, type MonitorContext } from "./manager";

const MAX_SSE_BUFFER = 1 << 20; // 1 MiB：服务端不按行发送时兜底
/**
 * 建连超时。注意它**不覆盖读流阶段**——opencode 空闲时 15 秒才发一次心跳，
 * 用同一个值去掐流会导致每 15 秒掉线一次。
 */
const CONNECT_TIMEOUT_MS = 10_000;

function authHeaders(server: ServerConfig): Record<string, string> {
  // v2 起服务端用 HTTP Basic（用户名固定 opencode），不是 Bearer。
  const header = opencodeBasicHeader(server);
  return header ? { Authorization: header } : {};
}

/** 取原始文本；拿不到返回 null，不抛——展示信息缺失不该让服务显示离线。 */
async function fetchTextOrNull(
  server: ServerConfig,
  url: string,
  token: { isCancelled: boolean },
): Promise<string | null> {
  if (token.isCancelled) return null;
  try {
    const response = await fetch(url, {
      headers: authHeaders(server),
      signal: AbortSignal.timeout(10_000),
    });
    if (!response.ok) return null;
    return await response.text();
  } catch {
    return null;
  }
}

function eventsUrl(server: ServerConfig): string {
  return new URL("/api/event", serverBaseUrl(server)).toString();
}

/**
 * 清掉待交互标记。
 *
 * 两条路径都清：一是 `form.replied`（已观测到）；二是 agent 重新动起手来——
 * 只有拿到答案它才会继续干活，所以"又开始调用工具"就等价于问题已解决。
 * 第二条是为 `form.cancelled` 兜底：那个事件名没观测到，不去猜，但若只依赖
 * `form.replied`，被取消的提问会让"等待回答"永久挂着。
 */
function clearPending(state: ProtocolState, sessionId: string): void {
  const activity = state.activities.get(sessionId);
  if (!activity || activity.pending === null) return;
  activity.pending = null;
  state.applyEffectiveBusy(sessionId);
}

function updateBusy(state: ProtocolState, sessionId: string, busy: boolean): void {
  if (busy) state.rawBusy.add(sessionId);
  else state.rawBusy.delete(sessionId);
  state.applyEffectiveBusy(sessionId);
}

function buildEvent(
  serverId: string,
  sessionId: string,
  kind: AgentEvent["kind"],
  eventKey: string,
  state: ProtocolState,
): AgentEvent {
  return {
    serverId,
    sessionId,
    sessionTitle: state.titles.get(sessionId) ?? null,
    kind,
    eventKey,
    body: null,
    occurredAt: new Date(),
  };
}

/**
 * 工具调用的活动行：`名字 · 命令预览`。
 *
 * 事件名与负载都来自真机观测（见 /tmp/opencode/observed-events.log），不是照抄文档：
 * opencode 把工具名放在 `session.tool.input.started` 的 `name`，把参数放在
 * `session.tool.input.ended` 的 `text`（一段 JSON 字符串，命令在 `command` 键）。
 */
function applyToolActivity(
  sessionId: string,
  properties: Record<string, unknown>,
  state: ProtocolState,
  phase: "started" | "ended",
): void {
  const callId = typeof properties.id === "string" ? properties.id : "";
  const activity = state.activityFor(sessionId);

  if (phase === "started") {
    const name =
      typeof properties.name === "string" && properties.name.length > 0
        ? properties.name
        : "工具";
    if (callId.length === 0) {
      activity.display = name;
      return;
    }
    activity.currentTool = { id: callId, name };
    activity.display = name;
    return;
  }

  // ended：text 是一段 JSON，形如 {"command": "..."} / {"filePath": "..."}
  const text = typeof properties.text === "string" ? properties.text : "";
  const command = text.length > 0 ? extractCommand(text) : null;
  if (callId.length > 0 && command) activity.toolCommands.set(callId, command);
  const name = activity.currentTool?.name;
  activity.currentTool = null;
  if (name) activity.display = command ? `${name} · ${command}` : name;
}

/** 从工具参数里取一行可展示的命令；取不到就返回 null，不硬凑。 */
export function extractCommand(text: string): string | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return null;
  }
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) return null;
  const record = parsed as Record<string, unknown>;
  // 按可读性排序：命令最能说明"在干什么"。
  for (const key of ["command", "cmd", "script", "filePath", "path", "pattern", "description"]) {
    const value = record[key];
    if (typeof value === "string" && value.trim().length > 0) {
      return value.trim().replace(/\s+/g, " ").slice(0, 80);
    }
  }
  return null;
}

/** 会话标题：新建/更新时同步。 */
function applySessionTitle(
  sessionId: string,
  properties: Record<string, unknown>,
  state: ProtocolState,
): void {
  // created 事件把整个会话对象放在 info 里，id 也只在这里。
  const info = properties.info as Record<string, unknown> | undefined;
  if (info && typeof info.id === "string" && info.id.length > 0) {
    const title =
      typeof info.title === "string" && info.title.length > 0 ? info.title : "OpenCode 会话";
    state.titles.set(info.id, title);
    return;
  }
  if (sessionId.length === 0) return;
  const title =
    typeof properties.title === "string" && properties.title.length > 0
      ? properties.title
      : "OpenCode 会话";
  state.titles.set(sessionId, title);
}

/**
 * 解析一段 SSE 文本（可能含多行），更新状态并返回需要发出的事件。
 *
 * v2 信封是 `{id, created, type, location, data:{...}}`，会话标识在 `data.sessionID`；
 * v1 的 `{properties:{...}}` 形状仍要兼容。
 */
export function handleSse(
  serverId: string,
  text: string,
  state: ProtocolState,
): AgentEvent[] {
  const events: AgentEvent[] = [];

  for (const line of text.split(/\r?\n/)) {
    if (!line.startsWith("data:")) continue;
    let json: unknown;
    try {
      json = JSON.parse(line.slice("data:".length).trim());
    } catch {
      continue;
    }
    if (typeof json !== "object" || json === null) continue;
    const envelope = json as Record<string, unknown>;

    // v2 把负载放在 data，更早的包装用 payload，v1 直接平铺。
    const candidate =
      (typeof envelope.data === "object" && envelope.data !== null
        ? (envelope.data as Record<string, unknown>)
        : null) ??
      (typeof envelope.payload === "object" && envelope.payload !== null
        ? (envelope.payload as Record<string, unknown>)
        : null) ??
      envelope;

    const rawType =
      (typeof candidate.type === "string" && candidate.type) ||
      // openapi 的 SSE schema 把判别字段写作 `event`，实测发的是 `type`。
      // 两个都认，省得服务端切换字段名时整片功能静默失效。
      (typeof envelope.event === "string" && envelope.event) ||
      (typeof envelope.type === "string" && envelope.type) ||
      "";
    // durable 事件带 .N 尾缀（如 session.created.1），归一化掉。
    const dot = rawType.lastIndexOf(".");
    const type =
      dot > 0 && /^\d+$/.test(rawType.slice(dot + 1)) ? rawType.slice(0, dot) : rawType;

    const properties =
      typeof candidate.properties === "object" && candidate.properties !== null
        ? (candidate.properties as Record<string, unknown>)
        : candidate;

    const sessionIdRaw = properties.sessionID ?? properties.session_id;
    const sessionId = typeof sessionIdRaw === "string" ? sessionIdRaw : null;

    switch (type) {
      case "session.updated":
      case "session.created": {
        // created 的会话 id 在 data.info.id 上，信封里没有 sessionID，
        // 所以这里不能拿 sessionId 当前置条件。
        applySessionTitle(sessionId ?? "", properties, state);
        break;
      }

      case "session.deleted": {
        if (sessionId) {
          state.titles.delete(sessionId);
          state.rawBusy.delete(sessionId);
          state.busy.delete(sessionId);
          state.activities.delete(sessionId);
        }
        break;
      }

      case "session.execution.started": {
        if (sessionId) updateBusy(state, sessionId, true);
        break;
      }

      case "session.execution.succeeded":
      case "session.execution.failed": {
        if (sessionId) updateBusy(state, sessionId, false);
        break;
      }

      // 活动行：工具名与命令预览。事件名取自真机观测（observed-events.log）。
      case "session.tool.input.started": {
        // agent 又动手了，说明之前那个提问已经有结果，见 clearPending 的说明。
        if (sessionId) {
          clearPending(state, sessionId);
          applyToolActivity(sessionId, properties, state, "started");
        }
        break;
      }

      case "session.tool.input.ended": {
        if (sessionId) applyToolActivity(sessionId, properties, state, "ended");
        break;
      }

      // 工具收尾：清掉 currentTool，显示行留给下一次事件覆盖。
      case "session.tool.success":
      case "session.tool.failed": {
        if (sessionId) state.activityFor(sessionId).currentTool = null;
        break;
      }

      // v1 兼容。
      case "session.status": {
        const status = properties.status as Record<string, unknown> | undefined;
        if (sessionId && status) {
          // 白名单活跃态，而不是"排除 idle"：status.type 是开放集合，
          // 用排除法会把 failed / aborted 这类终态也当成在跑。
          updateBusy(state, sessionId, status.type === "running" || status.type === "busy");
        }
        break;
      }

      case "session.idle": {
        if (sessionId) updateBusy(state, sessionId, false);
        break;
      }

      // 提问：opencode 用 form 机制表达「需要用户输入」。
      //
      // 会话 id 在 data.form.sessionID 上，不在 data.sessionID，所以不能复用上面的
      // sessionId。事件名与负载来自真机观测（form.created / form.replied）。
      case "form.created": {
        const form = properties.form as Record<string, unknown> | undefined;
        const metadata = form?.metadata as { kind?: unknown } | undefined;
        const formSession = typeof form?.sessionID === "string" ? form.sessionID : null;
        if (formSession && metadata?.kind === "question") {
          const formId = typeof form?.id === "string" ? form.id : "unknown";
          // 顺序要紧：applyEffectiveBusy 在会话不忙时会 activities.delete()，
          // 先设 pending 会被它抹掉。所以先算忙碌，再落 pending。
          state.applyEffectiveBusy(formSession);
          state.activityFor(formSession).pending = "question";
          events.push(
            buildEvent(serverId, formSession, "questionRequired", `opencode-form:${formId}`, state),
          );
        }
        break;
      }

      case "form.replied": {
        const formSession = typeof properties.sessionID === "string" ? properties.sessionID : null;
        if (formSession) clearPending(state, formSession);
        break;
      }

      /*
       * 这里没有处理审批（approvalRequired），是有意为之，不是漏了。
       *
       * opencode 的审批与提问是两套机制：审批走 Permission.Request（id 以 "per"
       * 开头，字段 action / resources / message），提问走 Form.Detail。真实服务
       * 上观测到的只有 form.*，没有承载 permission 的事件——该服务权限全开，
       * Permission.Effect 的 "ask" 分支不会触发。
       *
       * 若将来开启审批，这里不会有通知。补之前需要先重新采集事件名，别照 openapi
       * 猜（那份 schema 的信封字段都与实测不符）。详见
       * opencode-event-observations.md。
       */

      case "session.error": {
        if (sessionId) {
          events.push(
            buildEvent(serverId, sessionId, "failed", `opencode-session-error-${sessionId}`, state),
          );
        }
        break;
      }

      default:
        break;
    }
  }

  return events;
}

/** 会话列表基线：填充标题缓存。v2 返回 `{data:[...], cursor:{...}}`，翻页最多 20 次。 */
/**
 * 从 `/api/info` 的响应里取版本号。真实形状：`{version, pid, urls, paths, capabilities}`。
 */
export function parseServerVersion(json: string): string | null {
  const value = JSON.parse(json) as { version?: unknown };
  return typeof value.version === "string" ? value.version : null;
}

/**
 * 版本号。拿不到不影响监控：版本只是展示信息，断网之类的原因不该让服务显示离线。
 */
async function fetchVersion(
  server: ServerConfig,
  base: URL,
  context: MonitorContext,
): Promise<void> {
  const text = await fetchTextOrNull(server, new URL("/api/info", base).toString(), context.token);
  if (text === null) return;
  let version: string | null;
  try {
    version = parseServerVersion(text);
  } catch (error) {
    throw new Error(`info 返回的不是合法 JSON: ${(error as Error).message}`);
  }
  context.state.serverVersion = version;
}

/**
 * 服务端当前正在跑的会话（权威来源）。
 *
 * 这是纯事件驱动的一个补丁，也是必须的：SSE 只在状态**变化**时推事件。如果一个
 * 会话在我们连上之前就已经在跑，它永远不会发出 `session.execution.started`，
 * 于是监控会漏掉它——表现为"明明有活跃会话，列表里却不显示"。重连时同样会漏：
 * 断线期间开始的任务补不回来。
 *
 * `/api/session/active` 返回 `{data: {"ses_xxx": {"type": "running"}}}`，
 * 每次（重）连时对齐一次，事件只负责之后的增量更新。
 *
 * 返回 `null` 表示该端点不存在（老版本服务端），此时退回纯事件模式。其它错误
 * 一律抛出——那是真出问题，不能藏。
 */
export async function fetchActiveSessions(server: ServerConfig): Promise<Set<string> | null> {
  const url = new URL("/api/session/active", serverBaseUrl(server));
  const response = await fetch(url.toString(), {
    headers: { ...authHeaders(server), Accept: "application/json" },
    signal: AbortSignal.timeout(10_000),
  });
  // 404/501 = 服务端没这个能力，与"请求失败"是两回事，不能当错误处理。
  if (response.status === 404 || response.status === 501) {
    console.warn(
      `[monitor] ${server.id} 的服务端没有 /api/session/active，` +
        `忙碌状态只能靠事件驱动：连上之前就已开始的会话不会被发现。`,
    );
    return null;
  }
  if (!response.ok) {
    throw new Error(`活跃会话请求失败：HTTP ${response.status} ${url.pathname}`);
  }

  let value: unknown;
  try {
    value = await response.json();
  } catch (error) {
    throw new Error(`活跃会话响应不是合法 JSON：${(error as Error).message}`);
  }
  const data = (value as { data?: unknown }).data;
  if (data === null || typeof data !== "object" || Array.isArray(data)) {
    throw new Error("活跃会话响应缺少 data 对象");
  }

  const running = new Set<string>();
  for (const [sessionId, entry] of Object.entries(data as Record<string, unknown>)) {
    if (sessionId.length === 0) continue;
    // schema 里 type 目前只有 "running"；认不出的值宁可不当忙碌。
    if ((entry as { type?: unknown } | null)?.type === "running") running.add(sessionId);
  }
  return running;
}

/**
 * 用权威列表覆盖忙碌状态。
 *
 * 必须整体替换而不是合并：上一次连接推断出的忙碌可能已经过期（任务在断线期间
 * 跑完了），合并会让它永远卡在"忙碌"。
 */
export function seedBusyFromActive(state: ProtocolState, active: Set<string>): void {
  const affected = new Set<string>([...state.rawBusy, ...active]);
  state.rawBusy.clear();
  for (const id of active) state.rawBusy.add(id);
  for (const id of affected) state.applyEffectiveBusy(id);
}

/**
 * 会话列表里哪些会话已经"有结论"（agent 不再工作）。
 *
 * opencode 给已结束的会话带 `outcome`（实测取值 `succeeded` / `interrupted`）。
 * 活跃列表只代表"仍在工作"，所以这些会话的忙碌状态必须清掉——否则一旦断线
 * 就再没人清它：终态不会再发 `session.execution.*` 事件，本地推断的忙碌会
 * 永远挂着。
 */
export function finishedSessions(items: unknown[]): Set<string> {
  const finished = new Set<string>();
  for (const raw of items) {
    const item = raw as Record<string, unknown>;
    if (typeof item?.id !== "string" || item.id.length === 0) continue;
    const outcome = item.outcome;
    if (typeof outcome === "string" && outcome.length > 0) finished.add(item.id);
  }
  return finished;
}

async function fetchBaseline(server: ServerConfig, context: MonitorContext): Promise<void> {
  const base = serverBaseUrl(server);
  const titles = new Map<string, string>();
  const finished = new Set<string>();
  let cursor: string | null = null;

  for (let page = 0; page < 20; page += 1) {
    const url = new URL("/api/session", base);
    url.searchParams.set("limit", "200");
    if (cursor) url.searchParams.set("cursor", cursor);

    const response = await fetch(url.toString(), {
      headers: { ...authHeaders(server), Accept: "application/json" },
      signal: AbortSignal.timeout(10_000),
    });
    if (!response.ok) throw new Error(`HTTP ${response.status} ${url.pathname}`);
    const text = await response.text();

    let value: unknown;
    try {
      value = JSON.parse(text);
    } catch (error) {
      throw new Error(`会话列表不是合法 JSON: ${(error as Error).message}`);
    }
    const envelope = value as { data?: unknown; cursor?: { next?: unknown } };
    if (!Array.isArray(envelope.data)) {
      throw new Error("会话列表信封应为 {data:[...]}");
    }

    for (const raw of envelope.data) {
      const item = raw as Record<string, unknown>;
      if (typeof item.id !== "string") continue;
      const title =
        typeof item.title === "string" && item.title.length > 0 ? item.title : "OpenCode 会话";
      titles.set(item.id, title);
    }
    for (const id of finishedSessions(envelope.data)) finished.add(id);

    const next = envelope.cursor?.next;
    cursor = typeof next === "string" && next.length > 0 ? next : null;
    if (!cursor) break;
  }

  // 收齐再整体替换，避免清空后只填到第一页。
  context.state.titles.clear();
  for (const [id, title] of titles) context.state.titles.set(id, title);
  for (const id of finished) {
    context.state.rawBusy.delete(id);
    context.state.applyEffectiveBusy(id);
  }
  context.state.baselineComplete = true;
}

/**
 * 打开事件流。
 *
 * 超时只管「建连」，不管「读流」——这是 opencode 的一个硬约束：空闲时 SSE 每 15 秒
 * 才发一次 `: heartbeat`。若把 15 秒超时挂在 fetch 上，流会在下一个心跳到达前被 abort，
 * 表现为每 15 秒掉线一次，且重连间隙的忙闲事件全部丢失。
 * （Rust 版在 monitor/opencode.rs 里专门留了注释警告过这件事。）
 *
 * 所以这里手动控制：拿到响应头就撤销超时，之后只由取消信号驱动退出。
 *
 * `connectTimeoutMs` 可注入，便于测试用小值复现"慢心跳"场景。
 */
export async function openEventStream(
  server: ServerConfig,
  token: CancelToken,
  connectTimeoutMs = CONNECT_TIMEOUT_MS,
): Promise<{ response: Response; dispose: () => void }> {
  const controller = new AbortController();
  const abort = (reason: Error): void => {
    if (!controller.signal.aborted) controller.abort(reason);
  };
  const onCancel = (): void => abort(new MonitorCancelled());

  const connectTimer = setTimeout(
    () => abort(new Error(`事件流连接超时（${connectTimeoutMs / 1000} 秒内没有响应头）`)),
    connectTimeoutMs,
  );
  token.watch(onCancel);

  try {
    const response = await fetch(eventsUrl(server), {
      headers: { ...authHeaders(server), Accept: "text/event-stream" },
      signal: controller.signal,
    });
    if (!response.ok || !response.body) {
      throw new Error(`事件流连接失败：HTTP ${response.status}`);
    }
    // 响应头已到，连接超时使命完成；但取消订阅要留着，退出时靠它打断读。
    return { response, dispose: () => token.unwatch(onCancel) };
  } catch (error) {
    if (controller.signal.aborted) throw controller.signal.reason;
    throw error;
  } finally {
    clearTimeout(connectTimer);
  }
}

async function runOnce(context: MonitorContext): Promise<void> {
  const { server, state, token } = context;

  await fetchBaseline(server, context);
  // 版本与忙碌都取权威来源；失败按"没有该信息"处理，不影响监控。
  await fetchVersion(server, serverBaseUrl(server), context);
  const active = await fetchActiveSessions(server);
  if (active !== null) seedBusyFromActive(state, active);
  const pins = new PinTracker(context.pinned);
  pins.emitFinished(context);
  context.emitStatus({ connected: true, error: null });

  const { response, dispose } = await openEventStream(server, token);
  const reader = response.body!.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  // 心跳只负责把已知状态重新推给 UI，并检测置顶会话完成。
  let lastPush = Date.now();
  const PUSH_INTERVAL = 3_000;

  try {
    for (;;) {
      if (token.isCancelled) return;

      // 先看是否该做心跳推送，再决定要不要阻塞读。
      if (Date.now() - lastPush >= PUSH_INTERVAL) {
        pins.emitFinished(context);
        context.emitStatus({ connected: true, error: null });
        lastPush = Date.now();
      }

      const { value, done } = await reader.read();
      if (done) throw new Error("事件流已关闭");
      buffer += decoder.decode(value, { stream: true });

      if (buffer.length > MAX_SSE_BUFFER) {
        // 服务端不按行发送时避免无限增长。
        buffer = "";
        continue;
      }
      const index = buffer.lastIndexOf("\n");
      if (index < 0) continue;
      // TCP 分片可能把一行 data: 切断，所以按最后一个换行切。
      const ready = buffer.slice(0, index + 1);
      buffer = buffer.slice(index + 1);

      for (const event of handleSse(server.id, ready, state)) {
        context.emitEvent(event);
      }
      pins.emitFinished(context);
      context.emitStatus({ connected: true, error: null });
      lastPush = Date.now();
    }
  } finally {
    dispose();
    await reader.cancel().catch(() => {});
  }
}

export async function runOpencode(context: MonitorContext): Promise<void> {
  await runOnce(context);
}