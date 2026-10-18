/**
 *
 * 行为要求逐条对齐：哪些事件产生通知、忙碌怎么算、活动行显示什么，
 * 都直接影响用户看到的"正在做什么"和提醒时机。
 */

import type { AgentEvent, AgentEventKind, ProtocolState } from "./state";

/** 命令预览：取首个非空行并截断，避免把整段脚本塞进界面。 */
function commandPreview(inputText: string): string {
  const firstLine =
    inputText
      .split(/\r?\n/)
      .map((line) => line.trim())
      .find((line) => line.length > 0) ?? "";
  const MAX = 80;
  // 按码点而不是 UTF-16 码元计数，否则含 emoji 的命令会算错长度。
  // Rust 版按字节截断，这里按字符——两者都只为"够短"，按字符对界面更友好。
  const characters = [...firstLine];
  return characters.length > MAX ? `${characters.slice(0, MAX).join("")}…` : firstLine;
}

const SUBAGENT_TOOLS = new Set(["Agent", "AgentSwarm", "Task"]);

function displayName(name: string): string {
  return SUBAGENT_TOOLS.has(name) ? "子代理" : name;
}

function toolDisplay(name: string, command: string | null | undefined): string {
  return command ? `${name} · ${command}` : name;
}

function str(value: unknown): string {
  return typeof value === "string" ? value : "";
}

function bool(value: unknown): boolean {
  return value === true;
}

function applyPhase(sessionId: string, phase: Record<string, unknown>, state: ProtocolState): void {
  const activity = state.activityFor(sessionId);
  switch (str(phase.kind)) {
    case "tool_call": {
      const toolCallId = str(phase.toolCallId);
      const name = displayName(str(phase.name) || "工具");
      const command = activity.toolCommands.get(toolCallId);
      activity.currentTool = { id: toolCallId, name };
      activity.display = toolDisplay(name, command);
      break;
    }
    case "streaming":
    case "running": {
      activity.currentTool = null;
      activity.display = str(phase.stream) === "assistant" ? "输出中" : "思考中";
      break;
    }
    case "ended": {
      activity.currentTool = null;
      break;
    }
    default:
      break;
  }
}

function handleToolCallStarted(
  sessionId: string,
  payload: Record<string, unknown>,
  state: ProtocolState,
): void {
  const toolCallId = str(payload.toolCallId);
  if (toolCallId.length === 0) return;

  const name = str(payload.name);
  // 命令文本有三个来源，逐个回退。
  const display = payload.display as Record<string, unknown> | undefined;
  const args = payload.args as Record<string, unknown> | undefined;
  const command =
    str(display?.command) || str(args?.command) || str(args?.description);

  const preview = commandPreview(command);
  const activity = state.activityFor(sessionId);
  if (preview.length > 0) {
    activity.toolCommands.set(toolCallId, preview);
  }

  // 相位事件可能先于工具事件到达，这里补一次刷新。
  if (activity.currentTool?.id === toolCallId) {
    const resolved = name.length > 0 ? displayName(name) : activity.currentTool.name;
    activity.display = toolDisplay(resolved, preview.length > 0 ? preview : null);
  }
}

function handleProtocolEvent(
  serverId: string,
  message: Record<string, unknown>,
  now: Date,
  state: ProtocolState,
): AgentEvent[] {
  const eventType = str(message.type);
  const sessionId = str(message.session_id) || null;
  const payload = message.payload as Record<string, unknown> | undefined;
  if (!payload || typeof payload !== "object") return [];

  const events: AgentEvent[] = [];

  switch (eventType) {
    case "event.session.status_changed": {
      const status = str(payload.status) || "idle";
      const previous = str(payload.previous_status);
      const wasActive = previous.length > 0 && previous !== "idle";
      const isActive = status !== "idle";
      // 通知不在这里发：完成/失败走 prompt.*，审批/回答走 work_changed，
      // 单一通路避免一次事件触发两次提醒。
      if (sessionId && wasActive !== isActive) {
        if (isActive) state.mainTurnInactive.delete(sessionId);
        else state.mainTurnInactive.add(sessionId);
        state.applyEffectiveBusy(sessionId);
      }
      break;
    }

    case "event.session.work_changed": {
      const busy = bool(payload.busy);
      const pending = str(payload.pending_interaction) || "none";
      if (!sessionId) break;

      const previousPending = state.activities.get(sessionId)?.pending ?? null;
      if (busy) state.rawBusy.add(sessionId);
      else state.rawBusy.delete(sessionId);

      if (typeof payload.main_turn_active === "boolean") {
        if (payload.main_turn_active) state.mainTurnInactive.delete(sessionId);
        else state.mainTurnInactive.add(sessionId);
      }
      state.applyEffectiveBusy(sessionId);

      // 待审批/待回答在活动行优先展示，none 时回落相位文本。
      if (state.busy.has(sessionId)) {
        state.activityFor(sessionId).pending =
          pending === "approval" || pending === "question" ? pending : null;
      }

      // 只在状态跃迁时提醒一次，重复推送不刷屏。
      if (previousPending !== pending) {
        const kind: AgentEventKind | null =
          pending === "approval"
            ? "approvalRequired"
            : pending === "question"
              ? "questionRequired"
              : null;
        if (kind) {
          events.push(buildEvent(serverId, sessionId, kind, eventKey(message, pending), now, state));
        }
      }
      break;
    }

    case "event.session.created": {
      const session = payload.session as Record<string, unknown> | undefined;
      const id = str(session?.id);
      if (id.length === 0) break;
      const meta = session?.meta as Record<string, unknown> | undefined;
      const raw = str(meta?.title);
      state.titles.set(id, raw.length === 0 || raw === "null" ? "New Session" : raw);
      state.rawBusy.add(id);
      state.mainTurnInactive.delete(id);
      state.applyEffectiveBusy(id);
      break;
    }

    default:
      break;
  }

  return events;
}

function handleAgentEvent(
  serverId: string,
  message: Record<string, unknown>,
  now: Date,
  state: ProtocolState,
): AgentEvent[] {
  const eventType = str(message.type);
  const payload = message.payload as Record<string, unknown> | undefined;
  if (!payload || typeof payload !== "object") return [];

  const kind: AgentEventKind | null =
    eventType === "prompt.completed"
      ? "completed"
      : eventType === "prompt.aborted"
        ? "failed"
        : null;
  if (!kind) return [];

  const prefix = kind === "completed" ? "prompt-complete" : "prompt-aborted";
  const promptId = str(payload.promptId);
  const key = promptId.length > 0 ? `${prefix}:${promptId}` : eventKey(message, prefix);
  const sessionId = str(message.session_id) || null;
  return [buildEvent(serverId, sessionId, kind, key, now, state)];
}

/**
 * 事件去重键。
 *
 * 用 epoch + seq 而不是时间戳：同一帧被重发时键相同，通知不会重复。
 */
function eventKey(message: Record<string, unknown>, prefix: string): string {
  const epoch = str(message.epoch);
  const seq = message.seq;
  if (typeof seq === "number" && Number.isInteger(seq) && seq >= 0) {
    return `${prefix}:${epoch}:${seq}`;
  }
  return `${prefix}:${epoch}`;
}

function buildEvent(
  serverId: string,
  sessionId: string | null,
  kind: AgentEventKind,
  eventKey: string,
  now: Date,
  state: ProtocolState,
): AgentEvent {
  const title = sessionId ? (state.titles.get(sessionId) ?? null) : null;
  return {
    serverId,
    sessionId,
    sessionTitle: title,
    kind,
    eventKey,
    body: null,
    occurredAt: now,
  };
}

/** 解析一帧服务端推送，返回需要提醒用户的事件。 */
export function parseFrame(
  serverId: string,
  text: string,
  now: Date,
  state: ProtocolState,
): AgentEvent[] {
  // 解析失败要暴露出来：静默丢弃协议帧只会让问题变得无从查起。
  let message: unknown;
  try {
    message = JSON.parse(text);
  } catch (error) {
    throw new Error(`kimi frame is not valid JSON: ${(error as Error).message}`);
  }
  if (typeof message !== "object" || message === null || Array.isArray(message)) {
    throw new Error("kimi frame must be a JSON object");
  }
  const frame = message as Record<string, unknown>;
  const eventType = str(frame.type);

  // 只关心主 agent：子代理的回合事件一律忽略。
  const payload = frame.payload as Record<string, unknown> | undefined;
  const agentId = str(payload?.agentId);
  if (agentId.length > 0 && agentId !== "main") return [];

  switch (eventType) {
    case "server_hello":
    case "subscribe_ack":
    case "ack":
    case "ping":
    case "resync_required":
    case "error":
    case "prompt.submitted":
      return [];

    case "agent.status.updated": {
      const sessionId = str(frame.session_id);
      const phase = payload?.phase as Record<string, unknown> | undefined;
      if (sessionId.length > 0 && phase) applyPhase(sessionId, phase, state);
      return [];
    }

    case "tool.call.started": {
      const sessionId = str(frame.session_id);
      if (sessionId.length > 0 && payload) handleToolCallStarted(sessionId, payload, state);
      return [];
    }

    case "turn.ended": {
      // 轮次结束清命令缓存，防止无界增长。
      const sessionId = str(frame.session_id);
      if (sessionId.length > 0) {
        state.activities.get(sessionId)?.toolCommands.clear();
      }
      return [];
    }

    case "background.task.started":
    case "background.task.terminated": {
      const sessionId = str(frame.session_id);
      if (sessionId.length === 0) return [];
      const delta = eventType === "background.task.started" ? 1 : -1;
      const next = Math.max((state.bgRunning.get(sessionId) ?? 0) + delta, 0);
      if (next === 0) state.bgRunning.delete(sessionId);
      else state.bgRunning.set(sessionId, next);
      state.applyEffectiveBusy(sessionId);
      return [];
    }

    case "prompt.completed":
    case "prompt.aborted":
      return handleAgentEvent(serverId, frame, now, state);

    default:
      if (eventType.startsWith("event.session.")) {
        return handleProtocolEvent(serverId, frame, now, state);
      }
      return [];
  }
}