import { describe, expect, it } from "vitest";

import { handleSse } from "./opencode";
import {
  dshWaterfallReply,
  parseEventsFrame,
  parseEventsReady,
  parseFollowFrame,
  parseSessionList,
  waterfallEvent,
} from "../protocol/dsh";
import { ProtocolState } from "../protocol/state";

function listBody(items: unknown[]): string {
  return JSON.stringify({
    result: { ok: true, value: { items } },
  });
}

function itemFrame(value: unknown): string {
  return JSON.stringify({ type: "item", streamId: "s", value });
}

describe("dsh 会话列表", () => {
  it("填入标题、忙碌状态与子代理标记", () => {
    const state = new ProtocolState();
    const sessions = parseSessionList(
      listBody([
        { sessionId: "sess-a1", running: true, projections: { values: { title: "A" } } },
        { sessionId: "sess-a2", running: false, projections: { values: { title: "B" } } },
        { sessionId: "sess-sub", running: true, origin: "subagent", projections: { values: { title: "子" } } },
      ]),
      state,
    );
    expect(state.titles.get("sess-a1")).toBe("A");
    expect(state.titles.get("sess-a2")).toBe("B");
    expect(state.busy.has("sess-a1")).toBe(true);
    expect(state.busy.has("sess-a2")).toBe(false);
    expect(state.subagentSessions.has("sess-sub")).toBe(true);
    expect(state.baselineComplete).toBe(true);
    expect(sessions.map((s) => s.sessionId)).toEqual(["sess-a1", "sess-a2", "sess-sub"]);
    expect(sessions[2].subagent).toBe(true);
  });

  it("标题为字面量 null 时不留标题", () => {
    const state = new ProtocolState();
    parseSessionList(
      listBody([{ sessionId: "s", running: false, projections: { values: { title: null } } }]),
      state,
    );
    expect(state.titles.has("s")).toBe(false);
  });

  it("重新拉取会清掉上一轮的会话", () => {
    const state = new ProtocolState();
    parseSessionList(listBody([{ sessionId: "old", running: true }]), state);
    parseSessionList(listBody([{ sessionId: "new", running: false }]), state);
    expect(state.busy.has("old")).toBe(false);
    expect(state.busy.has("new")).toBe(false);
  });

  it("ok=false 时报错", () => {
    expect(() =>
      parseSessionList(JSON.stringify({ result: { ok: false } }), new ProtocolState()),
    ).toThrow(/ok=false/);
  });

  it("结构不符时报错而不是当成空列表", () => {
    expect(() => parseSessionList("{}", new ProtocolState())).toThrow(/缺少 result/);
    expect(() =>
      parseSessionList(JSON.stringify({ result: { ok: true, value: {} } }), new ProtocolState()),
    ).toThrow(/value.items/);
  });

  it("非法 JSON 报错", () => {
    expect(() => parseSessionList("not json", new ProtocolState())).toThrow(/JSON/);
  });
});

describe("dsh $events 全局流", () => {
  it("ready 帧取 clientId", () => {
    const frame = { type: "ready", clientId: "c-1", host: { home: "/home/x" } };
    expect(parseEventsReady(frame)).toBe("c-1");
    expect(parseEventsReady({ type: "emit" })).toBeNull();
  });

  it("api-session/status 驱动忙碌状态", () => {
    const state = new ProtocolState();
    const frame = (running: boolean) =>
      itemFrame({ type: "emit", event: "api-session/status", args: ["s1", running] });
    parseEventsFrame("srv", frame(true), state);
    expect(state.busy.has("s1")).toBe(true);
    parseEventsFrame("srv", frame(false), state);
    expect(state.busy.has("s1")).toBe(false);
  });

  it("api-session/added 是 upsert，重复投递不炸", () => {
    const state = new ProtocolState();
    const added = itemFrame({
      type: "emit",
      event: "api-session/added",
      args: [{ sessionId: "s1", running: false, projections: { values: { title: "T" } } }],
    });
    parseEventsFrame("srv", added, state);
    parseEventsFrame("srv", added, state);
    expect(state.titles.get("s1")).toBe("T");
    expect(state.busy.has("s1")).toBe(false);
  });

  it("api-session/added 带 origin=subagent 时标记为子代理", () => {
    const state = new ProtocolState();
    parseEventsFrame(
      "srv",
      itemFrame({
        type: "emit",
        event: "api-session/added",
        args: [{ sessionId: "sub", running: true, origin: "subagent" }],
      }),
      state,
    );
    expect(state.subagentSessions.has("sub")).toBe(true);
  });

  it("api-session/removed 清掉该会话状态", () => {
    const state = new ProtocolState();
    parseEventsFrame(
      "srv",
      itemFrame({ type: "emit", event: "api-session/added", args: [{ sessionId: "s1", running: true, projections: { values: { title: "T" } } }] }),
      state,
    );
    parseEventsFrame("srv", itemFrame({ type: "emit", event: "api-session/removed", args: ["s1"] }), state);
    expect(state.busy.has("s1")).toBe(false);
    expect(state.titles.has("s1")).toBe(false);
  });

  it("api-session/error 产出失败事件并带出原因", () => {
    const state = new ProtocolState();
    const { events } = parseEventsFrame(
      "srv",
      itemFrame({
        type: "emit",
        event: "api-session/error",
        args: ["s1", "llm-deepseek: no API key"],
      }),
      state,
    );
    expect(events).toHaveLength(1);
    expect(events[0].kind).toBe("failed");
    expect(events[0].body).toContain("no API key");
    expect(state.busy.has("s1")).toBe(false);
  });

  it("waterfall 帧解析出审批与提问，agentId 即会话 id", () => {
    const state = new ProtocolState();
    const approval = parseEventsFrame(
      "srv",
      itemFrame({
        type: "waterfall",
        event: "approval/request",
        eventId: "e1",
        agentId: "sess-1",
        request: { toolName: "bash" },
      }),
      state,
    );
    expect(approval.waterfalls).toEqual([
      { eventId: "e1", event: "approval/request", sessionId: "sess-1" },
    ]);
    expect(approval.events).toHaveLength(0);

    const event = waterfallEvent("srv", approval.waterfalls[0], state);
    expect(event.kind).toBe("approvalRequired");
    expect(event.sessionId).toBe("sess-1");
    expect(event.eventKey).toBe("approval:e1");

    const question = parseEventsFrame(
      "srv",
      itemFrame({ type: "waterfall", event: "user-questions/request", eventId: "e2", agentId: "s" }),
      state,
    );
    expect(waterfallEvent("srv", question.waterfalls[0], state).kind).toBe("questionRequired");
  });

  it("waterfall 缺 eventId 时仍给出可用键", () => {
    const state = new ProtocolState();
    const { waterfalls } = parseEventsFrame(
      "srv",
      itemFrame({ type: "waterfall", event: "approval/request" }),
      state,
    );
    expect(waterfalls).toHaveLength(1);
    expect(waterfalls[0].eventId.length).toBeGreaterThan(0);
  });

  it("waterfall 应答是 $events/result + next，绝不抢占", () => {
    const reply = JSON.parse(
      dshWaterfallReply("c-1", { eventId: "e1", event: "approval/request", sessionId: "s" }),
    );
    expect(reply.method).toBe("$events/result");
    expect(reply.payload.args.clientId).toBe("c-1");
    expect(reply.payload.args.eventId).toBe("e1");
    // next = 不认领，交回 Web UI 处理。
    expect(reply.payload.args.outcome).toEqual({ kind: "next" });
  });

  it("error / end 帧忽略，非法 JSON 报错而不是当空帧", () => {
    const state = new ProtocolState();
    expect(parseEventsFrame("srv", '{"type":"error","error":{"code":"x"}}', state).events).toEqual([]);
    expect(parseEventsFrame("srv", '{"type":"end","streamId":"s"}', state).events).toEqual([]);
    // 非法 JSON 是服务端出错，抛出去让外层重连，别静默当成空帧。
    expect(() => parseEventsFrame("srv", "not json", state)).toThrow(/JSON/);
  });
});

describe("dsh session/follow 单会话流", () => {
  it("turn/start 置忙，turn/end completed 置闲并产出完成", () => {
    const state = new ProtocolState();
    parseFollowFrame("srv", "s1", itemFrame({ type: "event", event: { type: "turn/start", seq: 1, data: { turn: 1 } } }), state);
    expect(state.busy.has("s1")).toBe(true);

    const events = parseFollowFrame(
      "srv",
      "s1",
      itemFrame({ type: "event", event: { type: "turn/end", seq: 2, data: { turn: 1, reason: { kind: "completed" } } } }),
      state,
    );
    expect(state.busy.has("s1")).toBe(false);
    expect(events).toHaveLength(1);
    expect(events[0].kind).toBe("completed");
    expect(events[0].eventKey).toBe("turn-end:2");
    expect(events[0].body).toBeNull();
  });

  it("turn/end error 判为失败并带出模型报错", () => {
    const state = new ProtocolState();
    const events = parseFollowFrame(
      "srv",
      "s1",
      itemFrame({
        type: "event",
        event: {
          type: "turn/end",
          seq: 18,
          data: { turn: 1, reason: { kind: "error", error: { message: "no API key", code: "MISSING_CREDENTIAL" } } },
        },
      }),
      state,
    );
    expect(events[0].kind).toBe("failed");
    expect(events[0].body).toBe("no API key");
  });

  it("aborted / blocked / max-tokens 都算失败", () => {
    for (const kind of ["aborted", "blocked", "max-tokens"]) {
      const state = new ProtocolState();
      const events = parseFollowFrame(
        "srv",
        "s1",
        itemFrame({ type: "event", event: { type: "turn/end", seq: 1, data: { reason: { kind } } } }),
        state,
      );
      expect(events[0]?.kind, kind).toBe("failed");
    }
  });

  it("未知 reason 不产出事件但仍清忙碌", () => {
    const state = new ProtocolState();
    parseFollowFrame("srv", "s1", itemFrame({ type: "event", event: { type: "turn/start", seq: 1 } }), state);
    const events = parseFollowFrame(
      "srv",
      "s1",
      itemFrame({ type: "event", event: { type: "turn/end", seq: 2, data: { reason: { kind: "weird" } } } }),
      state,
    );
    expect(events).toEqual([]);
    expect(state.busy.has("s1")).toBe(false);
  });

  it("snapshot 回放的历史不产出事件", () => {
    const state = new ProtocolState();
    const events = parseFollowFrame(
      "srv",
      "s1",
      itemFrame({
        type: "snapshot",
        cursor: 2,
        records: [
          { type: "event", event: { type: "turn/end", seq: 2, data: { reason: { kind: "completed" } } } },
        ],
      }),
      state,
    );
    expect(events).toEqual([]);
  });

  it("session/title 更新标题", () => {
    const state = new ProtocolState();
    parseFollowFrame(
      "srv",
      "s1",
      itemFrame({ type: "event", event: { type: "session/title", seq: 1, data: { title: "新标题" } } }),
      state,
    );
    expect(state.titles.get("s1")).toBe("新标题");
  });

  it("缺 seq 时用随机键，保证去重唯一", () => {
    const state = new ProtocolState();
    const frame = (seq?: number) =>
      itemFrame({
        type: "event",
        event: { type: "turn/end", ...(seq === undefined ? {} : { seq }), data: { reason: { kind: "completed" } } },
      });
    const a = parseFollowFrame("srv", "s1", frame(), state)[0].eventKey;
    const b = parseFollowFrame("srv", "s1", frame(), state)[0].eventKey;
    expect(a).not.toBe(b);
    expect(parseFollowFrame("srv", "s1", frame(5), state)[0].eventKey).toBe(
      parseFollowFrame("srv", "s1", frame(5), state)[0].eventKey,
    );
  });
});

describe("opencode SSE", () => {
  function sse(line: string): string {
    return `data: ${line}\n`;
  }

  it("v2 data 信封：会话创建取标题", () => {
    const state = new ProtocolState();
    handleSse("srv", sse('{"id":"1","type":"session.created","data":{"info":{"id":"s1","title":"重构"}}}'), state);
    expect(state.titles.get("s1")).toBe("重构");
  });

  it("v2 execution 事件驱动忙碌", () => {
    const state = new ProtocolState();
    handleSse("srv", sse('{"type":"session.execution.started","data":{"sessionID":"s1"}}'), state);
    expect(state.busy.has("s1")).toBe(true);
    handleSse("srv", sse('{"type":"session.execution.succeeded","data":{"sessionID":"s1"}}'), state);
    expect(state.busy.has("s1")).toBe(false);
  });

  it("execution.failed 也清忙碌", () => {
    const state = new ProtocolState();
    handleSse("srv", sse('{"type":"session.execution.started","data":{"sessionID":"s1"}}'), state);
    handleSse("srv", sse('{"type":"session.execution.failed","data":{"sessionID":"s1"}}'), state);
    expect(state.busy.has("s1")).toBe(false);
  });

  it("durable 事件带 .N 尾缀也归一化处理", () => {
    const state = new ProtocolState();
    handleSse("srv", sse('{"type":"session.created.1","data":{"info":{"id":"s1","title":"T"}}}'), state);
    expect(state.titles.get("s1")).toBe("T");
  });

  it("非数字后缀不被剥掉", () => {
    const state = new ProtocolState();
    handleSse("srv", sse('{"type":"session.created.beta","data":{"sessionID":"s1"}}'), state);
    expect(state.titles.has("s1")).toBe(false);
  });

  it("信封判别字段是 event 时也认（openapi 写的就是 event）", () => {
    const state = new ProtocolState();
    handleSse("srv", sse('{"id":"1","event":"session.execution.started","data":{"sessionID":"s1"}}'), state);
    expect(state.busy.has("s1")).toBe(true);
    handleSse("srv", sse('{"id":"2","event":"session.execution.succeeded","data":{"sessionID":"s1"}}'), state);
    expect(state.busy.has("s1")).toBe(false);
  });

  it("v1 properties 形状仍兼容", () => {
    const state = new ProtocolState();
    handleSse("srv", sse('{"type":"session.status","properties":{"sessionID":"s1","status":{"type":"busy"}}}'), state);
    expect(state.busy.has("s1")).toBe(true);
  });

  it("session.error 产出失败事件", () => {
    const state = new ProtocolState();
    const events = handleSse("srv", sse('{"type":"session.error","data":{"sessionID":"s1"}}'), state);
    expect(events).toHaveLength(1);
    expect(events[0].kind).toBe("failed");
    expect(events[0].eventKey).toBe("opencode-session-error-s1");
  });

  it("session.deleted 清掉该会话全部状态", () => {
    const state = new ProtocolState();
    handleSse("srv", sse('{"type":"session.created","data":{"info":{"id":"s1","title":"T"}}}'), state);
    handleSse("srv", sse('{"type":"session.execution.started","data":{"sessionID":"s1"}}'), state);
    handleSse("srv", sse('{"type":"session.deleted","data":{"sessionID":"s1"}}'), state);
    expect(state.titles.has("s1")).toBe(false);
    expect(state.busy.has("s1")).toBe(false);
  });

  it("非会话事件忽略", () => {
    const state = new ProtocolState();
    handleSse("srv", sse('{"type":"server.connected","data":{"foo":1}}'), state);
    expect(state.titles.size).toBe(0);
    expect(state.busy.size).toBe(0);
  });

  it("坏 JSON 的 data 行跳过，不影响后续行", () => {
    const state = new ProtocolState();
    handleSse("srv", "data: {坏掉\n" + sse('{"type":"session.created","data":{"info":{"id":"s1","title":"T"}}}'), state);
    expect(state.titles.get("s1")).toBe("T");
  });
});