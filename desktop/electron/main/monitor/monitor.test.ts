import { describe, expect, it } from "vitest";

import { handleSse } from "./opencode";
import { parseFrame, parseSessionList } from "../protocol/dsh";
import { ProtocolState } from "../protocol/state";

describe("dsh 会话列表", () => {
  const body = JSON.stringify({
    result: {
      ok: true,
      value: {
        items: [
          { sessionId: "sess-a1", running: true, projections: { values: { title: "A" } } },
          { sessionId: "sess-a2", running: false, projections: { values: { title: "B" } } },
        ],
      },
    },
  });

  it("填入标题与忙碌状态", () => {
    const state = new ProtocolState();
    parseSessionList(body, state);
    expect(state.titles.get("sess-a1")).toBe("A");
    expect(state.titles.get("sess-a2")).toBe("B");
    expect(state.busy.has("sess-a1")).toBe(true);
    expect(state.busy.has("sess-a2")).toBe(false);
    expect(state.baselineComplete).toBe(true);
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

describe("dsh 事件帧", () => {
  function run(lines: string[]): { state: ProtocolState; kinds: string[] } {
    const state = new ProtocolState();
    parseSessionList(
      JSON.stringify({ result: { ok: true, value: { items: [] } } }),
      state,
    );
    const kinds: string[] = [];
    for (const line of lines) {
      for (const event of parseFrame("srv", line, state)) kinds.push(event.kind);
    }
    return { state, kinds };
  }

  it("审批、提问、完成、失败分别产出对应事件", () => {
    const { kinds } = run([
      '{"type":"server-request","method":"approval/requested","rpcId":"r1","payload":{"sessionId":"s","approvalId":"a1"}}',
      '{"type":"server-request","method":"question/requested","rpcId":"r2","payload":{"sessionId":"s"}}',
      '{"type":"server-request","method":"session/event","payload":{"sessionId":"s","event":{"type":"turn/end","seq":1,"data":{"reason":{"kind":"completed"}}}}}',
      '{"type":"server-request","method":"session/event","payload":{"sessionId":"s","event":{"type":"turn/end","seq":2,"data":{"reason":{"kind":"error"}}}}}',
    ]);
    expect(kinds).toEqual(["approvalRequired", "questionRequired", "completed", "failed"]);
  });

  it("turn/start 与 turn/end 改变忙碌状态", () => {
    const { state } = run([
      '{"type":"server-request","method":"session/event","payload":{"sessionId":"s","event":{"type":"turn/start"}}}',
    ]);
    expect(state.busy.has("s")).toBe(true);
    const after = run([
      '{"type":"server-request","method":"session/event","payload":{"sessionId":"s","event":{"type":"turn/start"}}}',
      '{"type":"server-request","method":"session/event","payload":{"sessionId":"s","event":{"type":"turn/end","seq":1,"data":{"reason":{"kind":"completed"}}}}}',
    ]);
    expect(after.state.busy.has("s")).toBe(false);
  });

  it("未知原因不产出事件但仍清除忙碌", () => {
    const { state, kinds } = run([
      '{"type":"server-request","method":"session/event","payload":{"sessionId":"s","event":{"type":"turn/start"}}}',
      '{"type":"server-request","method":"session/event","payload":{"sessionId":"s","event":{"type":"turn/end","seq":1,"data":{"reason":{"kind":"weird"}}}}}',
    ]);
    expect(kinds).toEqual([]);
    expect(state.busy.has("s")).toBe(false);
  });

  it("turn/end 缺 seq 时用随机键，保证去重键唯一", () => {
    const state = new ProtocolState();
    const frame = (seq?: number) =>
      `{"type":"server-request","method":"session/event","payload":{"sessionId":"s","event":{"type":"turn/end"${
        seq === undefined ? "" : `,"seq":${seq}`
      },"data":{"reason":{"kind":"completed"}}}}}`;
    const a = parseFrame("srv", frame(), state)[0].eventKey;
    const b = parseFrame("srv", frame(), state)[0].eventKey;
    expect(a).not.toBe(b);
    // 有 seq 时同值同键。
    expect(parseFrame("srv", frame(5), state)[0].eventKey).toBe(
      parseFrame("srv", frame(5), state)[0].eventKey,
    );
  });

  it("标题投影更新标题", () => {
    const { state } = run([
      '{"type":"server-request","method":"session/projection","payload":{"sessionId":"s","key":"title","value":"新标题"}}',
    ]);
    expect(state.titles.get("s")).toBe("新标题");
  });

  it("非 server-request 帧忽略", () => {
    const { kinds } = run([
      '{"type":"server-response","rpcId":"r5","payload":{"sessionId":"s"}}',
      '{"type":"server-request","method":"tool/call","payload":{"sessionId":"s"}}',
      '{"type":"server-request","method":"unknown/method","payload":{"sessionId":"s"}}',
    ]);
    expect(kinds).toEqual([]);
  });

  it("非法帧报错", () => {
    expect(() => parseFrame("srv", "not json", new ProtocolState())).toThrow(/JSON/);
    expect(() => parseFrame("srv", "[1]", new ProtocolState())).toThrow(/JSON 对象/);
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