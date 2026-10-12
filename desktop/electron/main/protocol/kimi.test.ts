import { describe, expect, it } from "vitest";

import { parseFrame } from "./kimi";
import { ProtocolState } from "./state";

function frame(body: Record<string, unknown>): string {
  return JSON.stringify(body);
}

function parse(state: ProtocolState, body: Record<string, unknown>) {
  return parseFrame("s1", frame(body), new Date("2026-01-01T00:00:00Z"), state);
}

describe("子代理事件被忽略", () => {
  it("agentId 不是 main 的一律丢弃", () => {
    const state = new ProtocolState();
    const events = parse(state, {
      type: "prompt.completed",
      session_id: "sess-1",
      payload: { agentId: "sub-1", promptId: "p1" },
      epoch: "e1",
      seq: 1,
    });
    expect(events).toHaveLength(0);
  });

  it("主 agent 的完成事件正常产出", () => {
    const state = new ProtocolState();
    const events = parse(state, {
      type: "prompt.completed",
      session_id: "sess-1",
      payload: { agentId: "main", promptId: "p1" },
      epoch: "e1",
      seq: 1,
    });
    expect(events).toHaveLength(1);
    expect(events[0].kind).toBe("completed");
    expect(events[0].sessionId).toBe("sess-1");
  });
});

describe("完成与失败", () => {
  it("prompt.aborted 记为失败", () => {
    const state = new ProtocolState();
    const events = parse(state, {
      type: "prompt.aborted",
      session_id: "sess-1",
      payload: { promptId: "p9" },
      epoch: "e1",
      seq: 2,
    });
    expect(events[0].kind).toBe("failed");
    expect(events[0].eventKey).toBe("prompt-aborted:p9");
  });

  it("缺 payload 时不产出事件", () => {
    const state = new ProtocolState();
    expect(parse(state, { type: "prompt.completed", session_id: "s" })).toHaveLength(0);
  });
});

describe("去重键", () => {
  it("同一 epoch+seq 产生相同键", () => {
    const state = new ProtocolState();
    const body = {
      type: "prompt.completed",
      session_id: "s",
      payload: {},
      epoch: "e7",
      seq: 12,
    };
    expect(parse(state, body)[0].eventKey).toBe(
      parse(state, { ...body })[0].eventKey,
    );
  });

  it("缺 seq 时退回只用 epoch", () => {
    const state = new ProtocolState();
    const events = parse(state, {
      type: "prompt.completed",
      session_id: "s",
      payload: {},
      epoch: "e7",
    });
    expect(events[0].eventKey).toBe("prompt-complete:e7");
  });
});

describe("审批与提问只在跃迁时提醒", () => {
  const workChanged = (pending: string) => ({
    type: "event.session.work_changed",
    session_id: "sess-1",
    payload: { busy: true, pending_interaction: pending, main_turn_active: true },
    epoch: "e1",
    seq: 1,
  });

  it("首次进入审批态提醒一次", () => {
    const state = new ProtocolState();
    const events = parse(state, workChanged("approval"));
    expect(events).toHaveLength(1);
    expect(events[0].kind).toBe("approvalRequired");
  });

  it("重复推送不重复提醒", () => {
    const state = new ProtocolState();
    parse(state, workChanged("approval"));
    expect(parse(state, workChanged("approval"))).toHaveLength(0);
  });

  it("从审批切到提问再提醒", () => {
    const state = new ProtocolState();
    parse(state, workChanged("approval"));
    const events = parse(state, workChanged("question"));
    expect(events).toHaveLength(1);
    expect(events[0].kind).toBe("questionRequired");
  });

  it("回到 none 不提醒", () => {
    const state = new ProtocolState();
    parse(state, workChanged("approval"));
    expect(parse(state, workChanged("none"))).toHaveLength(0);
  });
});

describe("忙碌计算", () => {
  it("主回合结束但仍有后台任务时保持忙碌", () => {
    const state = new ProtocolState();
    parse(state, { type: "background.task.started", session_id: "sess-1" });
    parse(state, {
      type: "event.session.work_changed",
      session_id: "sess-1",
      payload: { busy: true, pending_interaction: "none", main_turn_active: false },
    });
    expect(state.busy.has("sess-1")).toBe(true);
    expect(state.activityText("sess-1")).toBe("主 agent 已完成 · 等 1 个后台任务");
  });

  it("后台任务清空后转为空闲", () => {
    const state = new ProtocolState();
    parse(state, { type: "background.task.started", session_id: "sess-1" });
    parse(state, {
      type: "event.session.work_changed",
      session_id: "sess-1",
      payload: { busy: true, pending_interaction: "none", main_turn_active: true },
    });
    parse(state, {
      type: "event.session.work_changed",
      session_id: "sess-1",
      payload: { busy: true, pending_interaction: "none", main_turn_active: false },
    });
    parse(state, { type: "background.task.terminated", session_id: "sess-1" });
    expect(state.busy.has("sess-1")).toBe(false);
  });

  it("服务器报空闲即清除忙碌", () => {
    const state = new ProtocolState();
    parse(state, {
      type: "event.session.work_changed",
      session_id: "sess-1",
      payload: { busy: true, pending_interaction: "none", main_turn_active: true },
    });
    expect(state.busy.has("sess-1")).toBe(true);
    parse(state, {
      type: "event.session.work_changed",
      session_id: "sess-1",
      payload: { busy: false, pending_interaction: "none", main_turn_active: true },
    });
    expect(state.busy.has("sess-1")).toBe(false);
  });
});

describe("活动行", () => {
  it("待审批优先于相位文本", () => {
    const state = new ProtocolState();
    parse(state, {
      type: "agent.status.updated",
      session_id: "sess-1",
      payload: { phase: { kind: "running", stream: "assistant" } },
    });
    expect(state.activityText("sess-1")).toBe("输出中");

    parse(state, {
      type: "event.session.work_changed",
      session_id: "sess-1",
      payload: { busy: true, pending_interaction: "approval", main_turn_active: true },
    });
    expect(state.activityText("sess-1")).toBe("等待审批");
  });

  it("工具调用显示 工具名 · 命令", () => {
    const state = new ProtocolState();
    parse(state, {
      type: "tool.call.started",
      session_id: "sess-1",
      payload: { toolCallId: "tc1", name: "Bash", display: { command: "git push\nsecond line" } },
    });
    parse(state, {
      type: "agent.status.updated",
      session_id: "sess-1",
      payload: { phase: { kind: "tool_call", toolCallId: "tc1", name: "Bash" } },
    });
    expect(state.activityText("sess-1")).toBe("Bash · git push");
  });

  it("命令来源依次回退 args.command / args.description", () => {
    const state = new ProtocolState();
    parse(state, {
      type: "tool.call.started",
      session_id: "s",
      payload: { toolCallId: "tc", name: "Bash", args: { description: "跑测试" } },
    });
    parse(state, {
      type: "agent.status.updated",
      session_id: "s",
      payload: { phase: { kind: "tool_call", toolCallId: "tc", name: "Bash" } },
    });
    expect(state.activityText("s")).toBe("Bash · 跑测试");
  });

  it("子代理类工具显示为 子代理", () => {
    const state = new ProtocolState();
    parse(state, {
      type: "agent.status.updated",
      session_id: "s",
      payload: { phase: { kind: "tool_call", toolCallId: "none", name: "Task" } },
    });
    expect(state.activityText("s")).toBe("子代理");
  });

  it("相位先于工具到达时能补上命令", () => {
    const state = new ProtocolState();
    // 相位先到，此时还没有命令缓存。
    parse(state, {
      type: "agent.status.updated",
      session_id: "s",
      payload: { phase: { kind: "tool_call", toolCallId: "tc", name: "Bash" } },
    });
    expect(state.activityText("s")).toBe("Bash");
    // 工具事件后到，应刷新为带命令的文本。
    parse(state, {
      type: "tool.call.started",
      session_id: "s",
      payload: { toolCallId: "tc", name: "Bash", display: { command: "ls" } },
    });
    expect(state.activityText("s")).toBe("Bash · ls");
  });

  it("轮次结束清空命令缓存", () => {
    const state = new ProtocolState();
    parse(state, {
      type: "tool.call.started",
      session_id: "s",
      payload: { toolCallId: "tc", name: "Bash", display: { command: "ls" } },
    });
    parse(state, { type: "turn.ended", session_id: "s" });
    expect(state.activities.get("s")?.toolCommands.size ?? 0).toBe(0);
  });
});

describe("会话创建", () => {
  it("取 meta.title，缺失时用占位", () => {
    const state = new ProtocolState();
    parse(state, {
      type: "event.session.created",
      session_id: "x",
      payload: { session: { id: "sess-9", meta: { title: "重构登录" } } },
    });
    expect(state.titles.get("sess-9")).toBe("重构登录");
    expect(state.busy.has("sess-9")).toBe(true);
  });

  it("标题为字符串 null 时回落占位", () => {
    const state = new ProtocolState();
    parse(state, {
      type: "event.session.created",
      payload: { session: { id: "sess-9", meta: { title: "null" } } },
    });
    expect(state.titles.get("sess-9")).toBe("New Session");
  });
});

describe("解析失败要暴露", () => {
  it("非法 JSON 直接抛错", () => {
    const state = new ProtocolState();
    expect(() => parseFrame("s1", "{ 坏掉", new Date(), state)).toThrow(/not valid JSON/);
  });

  it("非对象帧直接抛错", () => {
    const state = new ProtocolState();
    expect(() => parseFrame("s1", "[1,2]", new Date(), state)).toThrow(/must be a JSON object/);
  });
});

describe("心跳等噪声帧", () => {
  it("不产出事件也不改变状态", () => {
    const state = new ProtocolState();
    for (const type of ["ping", "ack", "subscribe_ack", "server_hello", "resync_required", "error"]) {
      expect(parse(state, { type, session_id: "s" })).toHaveLength(0);
    }
    expect(state.busy.size).toBe(0);
  });
});
describe("命令预览截断", () => {
  it("超长命令截断并加省略号", () => {
    const state = new ProtocolState();
    parse(state, {
      type: "tool.call.started",
      session_id: "s",
      payload: { toolCallId: "tc", name: "Bash", display: { command: "x".repeat(200) } },
    });
    parse(state, {
      type: "agent.status.updated",
      session_id: "s",
      payload: { phase: { kind: "tool_call", toolCallId: "tc", name: "Bash" } },
    });
    expect(state.activityText("s")).toBe(`Bash · ${"x".repeat(80)}…`);
  });

  it("含 emoji 的命令按码点计数，不算错长度", () => {
    const state = new ProtocolState();
    const command = "🚀".repeat(90); // 90 个码点，但 UTF-16 长度是 180
    parse(state, {
      type: "tool.call.started",
      session_id: "s",
      payload: { toolCallId: "tc", name: "Bash", display: { command } },
    });
    parse(state, {
      type: "agent.status.updated",
      session_id: "s",
      payload: { phase: { kind: "tool_call", toolCallId: "tc", name: "Bash" } },
    });
    const text = state.activityText("s")!;
    // 恰好 80 个码点 + 省略号；按 UTF-16 误判会多截或少截。
    expect([...text.slice("Bash · ".length)]).toHaveLength(81);
  });

  it("只取首个非空行", () => {
    const state = new ProtocolState();
    parse(state, {
      type: "tool.call.started",
      session_id: "s",
      payload: {
        toolCallId: "tc",
        name: "Bash",
        display: { command: "\n\n  echo hi  \n第二行" },
      },
    });
    parse(state, {
      type: "agent.status.updated",
      session_id: "s",
      payload: { phase: { kind: "tool_call", toolCallId: "tc", name: "Bash" } },
    });
    expect(state.activityText("s")).toBe("Bash · echo hi");
  });
});
