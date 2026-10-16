/**
 * opencode 事件解析。
 *
 * 这里的负载全部取自真机观测（/tmp/opencode/observed-events.log，27 种事件类型），
 * 不是照文档编的。openapi 的 causeSchema 只枚举了 Fail/Die/Interrupt 三种错误态，
 * 信封字段它写 `event` 而实测发 `type`——那份 schema 不可信，真机数据才可信。
 */

import { describe, expect, it } from "vitest";

import { extractCommand, finishedSessions, handleSse } from "./opencode";
import { ProtocolState } from "../protocol/state";

function sse(payload: unknown): string {
  return `data: ${JSON.stringify(payload)}\n`;
}

describe("opencode 事件解析（真机负载）", () => {
  it("session.execution.started / succeeded 驱动忙碌", () => {
    const state = new ProtocolState();
    handleSse("srv", sse({ type: "session.execution.started", data: { sessionID: "ses_1" } }), state);
    expect(state.busy.has("ses_1")).toBe(true);
    handleSse("srv", sse({ type: "session.execution.succeeded", data: { sessionID: "ses_1" } }), state);
    expect(state.busy.has("ses_1")).toBe(false);
  });

  it("工具开始与结束产出「工具名 · 命令」活动行", () => {
    const state = new ProtocolState();
    // 这两条的形状直接来自真机捕获。
    handleSse(
      "srv",
      sse({
        created: 1791804418933,
        type: "session.tool.input.started",
        data: { sessionID: "ses_1", assistantMessageID: "msg_1", id: "call_1", name: "bash" },
      }),
      state,
    );
    expect(state.activityFor("ses_1").display).toBe("bash");

    handleSse(
      "srv",
      sse({
        type: "session.tool.input.ended",
        data: {
          sessionID: "ses_1",
          id: "call_1",
          text: '{"command":"cd /tmp && ls"}',
        },
      }),
      state,
    );
    expect(state.activityFor("ses_1").display).toBe("bash · cd /tmp && ls");
    expect(state.activityFor("ses_1").currentTool).toBeNull();
  });

  it("工具结束事件清掉 currentTool", () => {
    const state = new ProtocolState();
    handleSse(
      "srv",
      sse({ type: "session.tool.input.started", data: { sessionID: "s", id: "c1", name: "read" } }),
      state,
    );
    expect(state.activityFor("s").currentTool).not.toBeNull();
    handleSse(
      "srv",
      sse({ type: "session.tool.success", data: { sessionID: "s", id: "c1" } }),
      state,
    );
    expect(state.activityFor("s").currentTool).toBeNull();
  });

  it("参数不是 JSON 时不编造命令，只显示工具名", () => {
    const state = new ProtocolState();
    handleSse(
      "srv",
      sse({ type: "session.tool.input.started", data: { sessionID: "s", id: "c1", name: "grep" } }),
      state,
    );
    handleSse(
      "srv",
      sse({ type: "session.tool.input.ended", data: { sessionID: "s", id: "c1", text: "不是JSON" } }),
      state,
    );
    expect(state.activityFor("s").display).toBe("grep");
  });

  it("session.created 用 info 里的标题", () => {
    const state = new ProtocolState();
    handleSse(
      "srv",
      sse({ type: "session.created", data: { info: { id: "ses_9", title: "重构登录" } } }),
      state,
    );
    expect(state.titles.get("ses_9")).toBe("重构登录");
  });

  it("session.deleted 清掉该会话全部状态（含活动）", () => {
    const state = new ProtocolState();
    handleSse("srv", sse({ type: "session.created", data: { info: { id: "s1", title: "T" } } }), state);
    handleSse("srv", sse({ type: "session.execution.started", data: { sessionID: "s1" } }), state);
    handleSse(
      "srv",
      sse({ type: "session.tool.input.started", data: { sessionID: "s1", id: "c", name: "bash" } }),
      state,
    );
    handleSse("srv", sse({ type: "session.deleted", data: { sessionID: "s1" } }), state);
    expect(state.titles.has("s1")).toBe(false);
    expect(state.busy.has("s1")).toBe(false);
    expect(state.activities.has("s1")).toBe(false);
  });

  it("信封判别字段是 event 时也认（openapi 写的就是 event）", () => {
    const state = new ProtocolState();
    handleSse("srv", sse({ id: "1", event: "session.execution.started", data: { sessionID: "s1" } }), state);
    expect(state.busy.has("s1")).toBe(true);
  });

  it("durable 事件带 .N 尾缀也归一化", () => {
    const state = new ProtocolState();
    handleSse("srv", sse({ type: "session.created.1", data: { info: { id: "s1", title: "T" } } }), state);
    expect(state.titles.get("s1")).toBe("T");
  });

  // form 是 opencode 表达「需要用户输入」的机制。下面的负载逐字取自真机捕获。
  it("form.created(kind=question) 产出提问事件并挂上待回答", () => {
    const state = new ProtocolState();
    const events = handleSse(
      "srv",
      sse({
        id: "evt_1",
        created: 1791805207069,
        type: "form.created",
        data: {
          form: {
            id: "frm_1",
            sessionID: "ses_q",
            title: "Questions",
            metadata: { kind: "question", tool: { messageID: "msg_1", id: "call_1" } },
            fields: [
              {
                key: "q0",
                title: "审批测试",
                description: "请从 A / B 中选择一项：",
                type: "string",
                options: [
                  { value: "选项 A", label: "选项 A" },
                  { value: "选项 B", label: "选项 B" },
                ],
                custom: true,
              },
            ],
          },
        },
      }),
      state,
    );
    expect(events).toHaveLength(1);
    expect(events[0].kind).toBe("questionRequired");
    // 去重键要用 form id，否则同一个 form 的多次推送会重复通知。
    expect(events[0].eventKey).toBe("opencode-form:frm_1");
    expect(state.activityFor("ses_q").pending).toBe("question");
    expect(state.activityText("ses_q")).toBe("等待回答");
  });

  it("真实时序：执行中提问，活动行仍是等待回答", () => {
    const state = new ProtocolState();
    handleSse("srv", sse({ type: "session.execution.started", data: { sessionID: "ses_q" } }), state);
    handleSse(
      "srv",
      sse({
        type: "session.tool.input.started",
        data: { sessionID: "ses_q", id: "c1", name: "ask" },
      }),
      state,
    );
    handleSse(
      "srv",
      sse({ type: "form.created", data: { form: { id: "frm_3", sessionID: "ses_q", metadata: { kind: "question" } } } }),
      state,
    );
    expect(state.busy.has("ses_q")).toBe(true);
    // 待回答优先于工具名——这正是 activityText 的既有优先级。
    expect(state.activityText("ses_q")).toBe("等待回答");
  });

  it("form.replied 清掉待回答", () => {
    const state = new ProtocolState();
    state.activityFor("ses_q").pending = "question";
    handleSse(
      "srv",
      sse({
        id: "evt_2",
        created: 1791805285004,
        type: "form.replied",
        data: { id: "frm_1", sessionID: "ses_q", answer: { q0: "选项 A" } },
      }),
      state,
    );
    expect(state.activities.get("ses_q")?.pending ?? null).toBeNull();
  });

  it("form.cancelled 事件名未观测，但 agent 重新动手也会清掉待回答", () => {
    const state = new ProtocolState();
    state.activityFor("ses_q").pending = "question";
    // 提问被取消后 agent 继续干活——不能因为没见过 form.cancelled 就永远挂着"等待回答"。
    handleSse(
      "srv",
      sse({ type: "session.tool.input.started", data: { sessionID: "ses_q", id: "c1", name: "bash" } }),
      state,
    );
    expect(state.activities.get("ses_q")?.pending ?? null).toBeNull();
  });

  it("kind 不是 question 的 form 不当作提问（不误报）", () => {
    const state = new ProtocolState();
    const events = handleSse(
      "srv",
      sse({ type: "form.created", data: { form: { id: "frm_2", sessionID: "ses_x", metadata: { kind: "other" } } } }),
      state,
    );
    expect(events).toEqual([]);
    expect(state.activities.get("ses_x")?.pending ?? null).toBeNull();
  });

  it("form.created 缺 sessionID 或 metadata 时不崩也不误报", () => {
    const state = new ProtocolState();
    expect(handleSse("srv", sse({ type: "form.created", data: {} }), state)).toEqual([]);
    expect(handleSse("srv", sse({ type: "form.created", data: { form: { id: "f", metadata: { kind: "question" } } } }), state)).toEqual([]);
  });

  it("session.error 产出失败事件", () => {
    const state = new ProtocolState();
    const events = handleSse("srv", sse({ type: "session.error", data: { sessionID: "s1" } }), state);
    expect(events).toHaveLength(1);
    expect(events[0].kind).toBe("failed");
  });

  it("shell.* / usage / text / reasoning 等事件不影响忙碌也不报错", () => {
    const state = new ProtocolState();
    const noise = [
      { type: "shell.created", data: { info: { id: "sh_1", status: "running", command: "ls" } } },
      { type: "shell.exited", data: { id: "sh_1", exit: 0, status: "exited" } },
      { type: "shell.deleted", data: { id: "sh_1" } },
      { type: "session.usage.updated", data: { sessionID: "s1", cost: 0, tokens: { input: 1 } } },
      { type: "session.text.started", data: { sessionID: "s1", ordinal: 0 } },
      { type: "session.text.delta", data: { sessionID: "s1", delta: "你好" } },
      { type: "session.text.ended", data: { sessionID: "s1", text: "你好" } },
      { type: "session.reasoning.started", data: { sessionID: "s1", ordinal: 0 } },
      { type: "session.reasoning.delta", data: { sessionID: "s1", delta: "thinking" } },
      { type: "session.reasoning.ended", data: { sessionID: "s1", text: "thinking" } },
      { type: "session.step.started", data: { sessionID: "s1", agent: "build" } },
      { type: "session.step.streamed", data: { sessionID: "s1" } },
      { type: "session.step.ended", data: { sessionID: "s1", finish: "tool-calls" } },
      { type: "session.tool.called", data: { sessionID: "s1", id: "c", input: {} } },
      { type: "session.tool.progress", data: { sessionID: "s1", id: "c" } },
      { type: "session.inbox.enqueued", data: { sessionID: "s1" } },
      { type: "session.inbox.delivered", data: { sessionID: "s1" } },
      { type: "project.updated", data: { id: "p1" } },
      { type: "model.updated", data: {} },
      { type: "provider.updated", data: {} },
      { type: "server.connected", data: {} },
    ];
    for (const payload of noise) {
      expect(handleSse("srv", sse(payload), state)).toEqual([]);
    }
    expect(state.busy.size).toBe(0);
  });

  it("v1 properties 形状仍兼容", () => {
    const state = new ProtocolState();
    handleSse(
      "srv",
      sse({ type: "session.status", properties: { sessionID: "s1", status: { type: "busy" } } }),
      state,
    );
    expect(state.busy.has("s1")).toBe(true);
  });

  it("v1 的终态不再被当成忙碌", () => {
    // 以前这里是 `status.type !== "idle"`，于是 failed/aborted 这类终态也被算成在跑。
    // 活跃列表只代表"仍在工作"，先用 busy 把会话点亮，再用终态把它清掉。
    for (const terminal of ["failed", "aborted", "interrupted", "something_new"]) {
      const state = new ProtocolState();
      handleSse(
        "srv",
        sse({ type: "session.status", properties: { sessionID: "s1", status: { type: "running" } } }),
        state,
      );
      expect(state.busy.has("s1")).toBe(true);
      handleSse(
        "srv",
        sse({ type: "session.status", properties: { sessionID: "s1", status: { type: terminal } } }),
        state,
      );
      expect(state.busy.has("s1"), `终态 ${terminal} 应清掉忙碌`).toBe(false);
    }
  });
});

describe("opencode 已有结论的会话", () => {
  it("带 outcome 的会话视为已结束", () => {
    const finished = finishedSessions([
      { id: "ses_a", outcome: "succeeded", title: "A" },
      { id: "ses_b", outcome: "interrupted", title: "B" },
      { id: "ses_c", outcome: null, title: "C" },
      { id: "ses_d", title: "D" },
      { id: "", outcome: "succeeded" },
    ]);
    expect([...finished].sort()).toEqual(["ses_a", "ses_b"]);
  });

  it("列表里没有 outcome 就不清忙碌", () => {
    expect(finishedSessions([{ id: "ses_x" }, { id: "ses_y", outcome: "" }]).size).toBe(0);
  });
});

describe("命令提取", () => {
  it("取 command", () => {
    expect(extractCommand('{"command":"npm test"}')).toBe("npm test");
  });

  it("没有 command 时依次尝试其它键", () => {
    expect(extractCommand('{"filePath":"/a/b.ts"}')).toBe("/a/b.ts");
    expect(extractCommand('{"pattern":"TODO"}')).toBe("TODO");
  });

  it("压掉换行并截断到 80 字符", () => {
    const long = "x".repeat(300);
    expect(extractCommand(JSON.stringify({ command: long }))).toHaveLength(80);
    expect(extractCommand('{"command":"a\\n  b"}')).toBe("a b");
  });

  it("非对象、非 JSON、空值都返回 null", () => {
    expect(extractCommand("不是JSON")).toBeNull();
    expect(extractCommand("[1,2]")).toBeNull();
    expect(extractCommand("{}")).toBeNull();
    expect(extractCommand('{"command":"   "}')).toBeNull();
  });
});