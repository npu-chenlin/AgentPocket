/**
 * dsh（DeepSeek Harness）协议解析。
 * 从 desktop/src-tauri/src/protocol/dsh.rs 迁移。
 */

import { randomUUID } from "node:crypto";

import type { AgentEvent, AgentEventKind, ProtocolState } from "./state";

function buildEvent(
  serverId: string,
  sessionId: string | null,
  kind: AgentEventKind,
  eventKey: string,
  state: ProtocolState,
): AgentEvent {
  return {
    serverId,
    sessionId,
    sessionTitle: sessionId ? (state.titles.get(sessionId) ?? null) : null,
    kind,
    eventKey,
    body: null,
    occurredAt: new Date(),
  };
}

/** 服务端偶尔不带 rpcId/approvalId，退化成一个随机 id，保证键非空。 */
function orUuid(value: unknown): string {
  return typeof value === "string" && value.length > 0 ? value : randomUUID();
}

function asObject(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

/** 解析 session.list 的 RPC 响应，重置标题与忙碌状态。 */
export function parseSessionList(body: string, state: ProtocolState): void {
  let value: unknown;
  try {
    value = JSON.parse(body);
  } catch (error) {
    throw new Error(`session.list 响应不是合法 JSON: ${(error as Error).message}`);
  }
  const result = asObject((value as Record<string, unknown>)?.result);
  if (!result) throw new Error("session.list 响应缺少 result");
  if (result.ok !== true) throw new Error("session.list 返回 ok=false");

  const items = asObject(result.value)?.items;
  if (!Array.isArray(items)) throw new Error("session.list 响应缺少 value.items");

  state.titles.clear();
  state.busy.clear();

  for (const raw of items) {
    const item = asObject(raw);
    const sessionId = typeof item?.sessionId === "string" ? item.sessionId : null;
    if (sessionId === null) throw new Error("会话条目缺少 sessionId");

    if (item?.running === true) state.busy.add(sessionId);

    const title = asObject(asObject(item?.projections)?.values)?.title;
    if (typeof title === "string" && title.length > 0 && title !== "null") {
      state.titles.set(sessionId, title);
    }
  }

  state.baselineComplete = true;
}

/** 解析一帧服务端推送。 */
export function parseFrame(
  serverId: string,
  text: string,
  state: ProtocolState,
): AgentEvent[] {
  let message: unknown;
  try {
    message = JSON.parse(text);
  } catch (error) {
    throw new Error(`dsh 帧不是合法 JSON: ${(error as Error).message}`);
  }
  const frame = asObject(message);
  if (!frame) throw new Error("dsh 帧必须是 JSON 对象");
  if (frame.type !== "server-request") return [];

  const method = typeof frame.method === "string" ? frame.method : "";
  const payload = asObject(frame.payload);
  if (!payload) return [];

  const sessionId = typeof payload.sessionId === "string" ? payload.sessionId : null;

  switch (method) {
    case "session/event": {
      const event = asObject(payload.event);
      return event ? handleSessionEvent(serverId, sessionId, event, state) : [];
    }

    case "session/projection": {
      if (payload.key === "title" && sessionId && typeof payload.value === "string") {
        const title = payload.value;
        if (title.length > 0 && title !== "null") state.titles.set(sessionId, title);
      }
      return [];
    }

    case "approval/requested":
      return [
        buildEvent(
          serverId,
          sessionId,
          "approvalRequired",
          `approval:${orUuid(payload.approvalId)}`,
          state,
        ),
      ];

    case "question/requested":
      return [
        buildEvent(
          serverId,
          sessionId,
          "questionRequired",
          `question:${orUuid(frame.rpcId)}`,
          state,
        ),
      ];

    case "session/subscribed":
    case "session/queue":
    case "approval/resolved":
    case "question/resolved":
      return [];

    default:
      return [];
  }
}

function handleSessionEvent(
  serverId: string,
  sessionId: string | null,
  event: Record<string, unknown>,
  state: ProtocolState,
): AgentEvent[] {
  const type = typeof event.type === "string" ? event.type : "";

  switch (type) {
    case "turn/start": {
      if (sessionId) state.busy.add(sessionId);
      return [];
    }

    case "turn/end": {
      if (sessionId) state.busy.delete(sessionId);
      const reason = asObject(asObject(event.data)?.reason)?.kind;
      const kind =
        typeof reason === "string"
          ? reason
          : "";
      const mapped: AgentEventKind | null =
        kind === "completed"
          ? "completed"
          : ["error", "aborted", "blocked", "max-tokens", "interrupted"].includes(kind)
            ? "failed"
            : null;
      if (!mapped) return [];

      const seq = typeof event.seq === "number" ? event.seq : -1;
      const key = seq >= 0 ? `turn-end:${seq}` : `turn-end:${randomUUID()}`;
      return [buildEvent(serverId, sessionId, mapped, key, state)];
    }

    case "session/title": {
      const title = asObject(event.data)?.title;
      if (sessionId && typeof title === "string" && title.length > 0 && title !== "null") {
        state.titles.set(sessionId, title);
      }
      return [];
    }

    default:
      return [];
  }
}