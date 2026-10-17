/**
 * dsh（DeepSeek Harness）协议解析 —— 0.2 版。
 *
 * 0.1 那套"一条全局下游流，帧里带 session/event、approval/requested"的模型在
 * 0.2 已经不存在了。现在分三条：
 *
 *   POST /api/session/list  会话集合 + running 标志（启动时拉一次）
 *   ws  $events             全局：会话增删、running 变化、审批/提问请求
 *   ws  session/follow      单会话：turn/start、turn/end（每个会话一条连接）
 *
 * 审批与提问是 waterfall 帧，**必须回一条**——这不是可选的订阅确认，而是应答
 * 通道：所有连着的客户端都不回，服务端会一直挂着等（无人应答最终 fail closed）。
 * 监控只旁观，一律回 `{kind:"next"}` 让 Web UI 去处理。
 */

import { randomUUID } from "node:crypto";

import type { AgentEvent, AgentEventKind, ProtocolState } from "./state";

function buildEvent(
  serverId: string,
  sessionId: string | null,
  kind: AgentEventKind,
  eventKey: string,
  state: ProtocolState,
  body: string | null = null,
): AgentEvent {
  return {
    serverId,
    sessionId,
    sessionTitle: sessionId ? (state.titles.get(sessionId) ?? null) : null,
    kind,
    eventKey,
    body,
    occurredAt: new Date(),
  };
}

function asObject(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function asArray(value: unknown): unknown[] | null {
  return Array.isArray(value) ? value : null;
}

function asString(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}

/** 会话标题：projections.values.title 非空才算，服务端会给字面量 "null"。 */
function titleOf(item: Record<string, unknown> | null): string | null {
  const title = asObject(asObject(item?.projections)?.values)?.title;
  return typeof title === "string" && title.length > 0 && title !== "null" ? title : null;
}

/** 会话列表里的一行。 */
export interface DshSession {
  sessionId: string;
  running: boolean;
  /** 子代理会话（`origin === "subagent"`），不单独订阅。 */
  subagent: boolean;
  title: string | null;
}

/** 解析 `session/list` 的 RPC 响应。 */
export function parseSessionList(body: string, state: ProtocolState): DshSession[] {
  let value: unknown;
  try {
    value = JSON.parse(body);
  } catch (error) {
    throw new Error(`session/list 响应不是合法 JSON: ${(error as Error).message}`);
  }
  const result = asObject((value as Record<string, unknown>)?.result);
  if (!result) throw new Error("session/list 响应缺少 result");
  if (result.ok !== true) throw new Error("session/list 返回 ok=false");

  const items = asArray(asObject(result.value)?.items);
  if (!items) throw new Error("session/list 响应缺少 value.items");

  const sessions: DshSession[] = [];
  state.titles.clear();
  state.busy.clear();
  state.subagentSessions.clear();

  for (const raw of items) {
    const item = asObject(raw);
    const sessionId = asString(item?.sessionId);
    if (sessionId === null) throw new Error("会话条目缺少 sessionId");

    const subagent = item?.origin === "subagent";
    const title = titleOf(item);
    if (title !== null) state.titles.set(sessionId, title);
    if (subagent) state.subagentSessions.add(sessionId);
    if (item?.running === true) state.busy.add(sessionId);

    sessions.push({ sessionId, running: item?.running === true, subagent, title });
  }

  state.baselineComplete = true;
  return sessions;
}

/** `$events` 的 waterfall 帧：需要客户端应答的那种。 */
export interface DshWaterfall {
  /** 应答时回传的 eventId。 */
  eventId: string;
  /** `approval/request` 或 `user-questions/request`。 */
  event: string;
  /**
   * 事件归属的会话。
   *
   * 帧里这个字段叫 `agentId`，但 dsh 的 agent 就是以 session id 为键创建的
   * （`agent/status` 直接拿 `agent.id` 当 sessionId 发），所以它等于会话 id。
   */
  sessionId: string | null;
}

/** `$events` 首帧里的 clientId，应答 waterfall 时必须带上。 */
export function parseEventsReady(frame: Record<string, unknown>): string | null {
  if (frame.type !== "ready") return null;
  return asString(frame.clientId);
}

/**
 * 解析 `$events` 帧。
 *
 * 返回 `{ events, waterfalls }`：事件推给上层，waterfall 由监控循环立刻回
 * `next`（见 `dshWaterfallReply`）。
 */
export function parseEventsFrame(
  serverId: string,
  text: string,
  state: ProtocolState,
): { events: AgentEvent[]; waterfalls: DshWaterfall[] } {
  const frame = parseOuter(text);
  if (frame === null) return { events: [], waterfalls: [] };
  // 只有 item 帧带 value；error/end 帧对监控没有信息量。
  if (frame.type !== "item") return { events: [], waterfalls: [] };
  const value = asObject(frame.value);
  if (!value) return { events: [], waterfalls: [] };

  const type = asString(value.type);
  if (type === "waterfall") return { events: [], waterfalls: [parseWaterfallFrame(value)] };
  if (type !== "emit") return { events: [], waterfalls: [] };

  return { events: handleEmit(serverId, value, state), waterfalls: [] };
}

function parseWaterfallFrame(value: Record<string, unknown>): DshWaterfall {
  return {
    eventId: asString(value.eventId) ?? randomUUID(),
    event: asString(value.event) ?? "",
    sessionId: asString(value.agentId),
  };
}

function handleEmit(
  serverId: string,
  value: Record<string, unknown>,
  state: ProtocolState,
): AgentEvent[] {
  const event = asString(value.event) ?? "";
  const args = asArray(value.args) ?? [];
  const sessionId = asString(args[0]);

  switch (event) {
    case "api-session/status": {
      // args = [sessionId, running]
      if (sessionId === null) return [];
      if (args[1] === true) state.busy.add(sessionId);
      else state.busy.delete(sessionId);
      return [];
    }

    case "api-session/added": {
      // 新会话会出现两次（session/created 与 agent/created 各一次），按 id upsert。
      const summary = asObject(args[0]);
      const id = asString(summary?.sessionId);
      if (id === null) return [];
      if (summary?.origin === "subagent") state.subagentSessions.add(id);
      const title = titleOf(summary);
      if (title !== null) state.titles.set(id, title);
      if (summary?.running === true) state.busy.add(id);
      return [];
    }

    case "api-session/removed": {
      if (sessionId === null) return [];
      state.busy.delete(sessionId);
      state.titles.delete(sessionId);
      state.subagentSessions.delete(sessionId);
      return [];
    }

    case "api-session/error": {
      // 回合之外的 agent 失败：没有 turn/end 可依，只有这条。
      if (sessionId === null) return [];
      state.busy.delete(sessionId);
      const message = asString(args[1]);
      return [
        buildEvent(serverId, sessionId, "failed", `agent-error:${sessionId}`, state, message),
      ];
    }

    default:
      return [];
  }
}

/**
 * 解析 `session/follow` 帧。
 *
 * 每帧只属于打开它的那条连接，帧里不带 sessionId——由调用方告诉我们对哪个
 * 会话，别按帧内容去猜。
 */
export function parseFollowFrame(
  serverId: string,
  sessionId: string,
  text: string,
  state: ProtocolState,
): AgentEvent[] {
  const frame = parseOuter(text);
  if (frame === null || frame.type !== "item") return [];
  const value = asObject(frame.value);
  if (!value) return [];

  switch (asString(value.type)) {
    case "snapshot": {
      // 打开时回放一段窗口（默认最近 50 条），里面的 turn/end 是历史，不该通知。
      // 标题之类的元信息可以采纳。
      return [];
    }
    case "event": {
      const event = asObject(value.event);
      return event ? handleSessionEvent(serverId, sessionId, event, state) : [];
    }
    default:
      return [];
  }
}

function parseOuter(text: string): Record<string, unknown> | null {
  let message: unknown;
  try {
    message = JSON.parse(text);
  } catch (error) {
    throw new Error(`dsh 帧不是合法 JSON: ${(error as Error).message}`);
  }
  return asObject(message);
}

function handleSessionEvent(
  serverId: string,
  sessionId: string,
  event: Record<string, unknown>,
  state: ProtocolState,
): AgentEvent[] {
  const type = asString(event.type) ?? "";
  const seq = typeof event.seq === "number" ? event.seq : -1;

  switch (type) {
    case "turn/start": {
      state.busy.add(sessionId);
      return [];
    }

    case "turn/end": {
      state.busy.delete(sessionId);
      const data = asObject(event.data);
      const reason = asObject(data?.reason);
      const kind = asString(reason?.kind);
      const mapped: AgentEventKind | null =
        kind === "completed"
          ? "completed"
          : kind !== null &&
              ["error", "aborted", "blocked", "max-tokens", "interrupted", "forked"].includes(kind)
            ? "failed"
            : null;
      if (mapped === null) return [];

      // 失败时把模型的报错带出来——"任务失败"四个字对排查没有帮助。
      const body = mapped === "failed" ? failureBody(reason) : null;
      const key = seq >= 0 ? `turn-end:${seq}` : `turn-end:${randomUUID()}`;
      return [buildEvent(serverId, sessionId, mapped, key, state, body)];
    }

    case "session/title": {
      const title = asObject(event.data)?.title;
      if (typeof title === "string" && title.length > 0 && title !== "null") {
        state.titles.set(sessionId, title);
      }
      return [];
    }

    default:
      return [];
  }
}

/** 从 `turn/end` 的 reason 里取一句可读的失败原因。 */
export function failureBody(reason: Record<string, unknown> | null): string | null {
  const kind = asString(reason?.kind) ?? "failed";
  if (kind !== "error") return kind;
  const message = asString(asObject(reason?.error)?.message);
  return message ?? kind;
}

/** waterfall 的应答：监控只旁观，回 `next` 把决定权交回 Web UI。 */
export function dshWaterfallReply(clientId: string, waterfall: DshWaterfall): string {
  return JSON.stringify({
    type: "client-request",
    rpcId: randomUUID(),
    method: "$events/result",
    payload: {
      args: { clientId, eventId: waterfall.eventId, outcome: { kind: "next" } },
    },
  });
}

/** waterfall 帧转成一条通知事件。 */
export function waterfallEvent(
  serverId: string,
  waterfall: DshWaterfall,
  state: ProtocolState,
): AgentEvent {
  const approval = waterfall.event === "approval/request";
  return buildEvent(
    serverId,
    waterfall.sessionId,
    approval ? "approvalRequired" : "questionRequired",
    `${approval ? "approval" : "question"}:${waterfall.eventId}`,
    state,
  );
}
