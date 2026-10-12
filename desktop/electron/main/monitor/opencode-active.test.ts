/**
 * 活跃会话的对齐逻辑。
 *
 * 这组测试针对的是纯事件驱动的根本缺陷：SSE 只在状态**变化**时推事件，所以在我们
 * 连上之前就已经在跑的会话永远发不出 `session.execution.started`，必须靠权威端点
 * `/api/session/active` 补齐。
 */

import { createServer, type Server } from "node:http";
import { afterEach, describe, expect, it } from "vitest";

import { fetchActiveSessions, seedBusyFromActive } from "./opencode";
import { ProtocolState } from "../protocol/state";
import type { ServerConfig } from "../model";

const base: ServerConfig = {
  id: "s1",
  name: "本地",
  host: "127.0.0.1",
  port: 0,
  backend: "opencode",
  token: "pw",
};

let running: Server | null = null;

afterEach(async () => {
  if (running) {
    running.closeAllConnections();
    await new Promise<void>((resolve) => running!.close(() => resolve()));
    running = null;
  }
});

async function serve(body: string, status = 200): Promise<ServerConfig> {
  const http = createServer((req, res) => {
    if (new URL(req.url ?? "/", "http://x").pathname !== "/api/session/active") {
      res.writeHead(404);
      res.end();
      return;
    }
    res.writeHead(status, { "Content-Type": "application/json" });
    res.end(body);
  });
  await new Promise<void>((resolve) => http.listen(0, "127.0.0.1", resolve));
  const address = http.address();
  if (typeof address !== "object" || address === null) throw new Error("拿不到端口");
  running = http;
  return { ...base, port: address.port };
}

describe("/api/session/active", () => {
  it("取出 type=running 的会话", async () => {
    const config = await serve(
      JSON.stringify({
        data: {
          ses_a: { type: "running" },
          ses_b: { type: "running" },
        },
      }),
    );
    expect(await fetchActiveSessions(config)).toEqual(new Set(["ses_a", "ses_b"]));
  });

  it("空列表表示没有活跃会话", async () => {
    const config = await serve(JSON.stringify({ data: {} }));
    expect(await fetchActiveSessions(config)).toEqual(new Set());
  });

  it("认不出的 type 不算忙碌", async () => {
    // 服务端将来加了新状态时，宁可漏报也不要误报成"在跑"。
    const config = await serve(
      JSON.stringify({ data: { ses_a: { type: "paused" }, ses_b: { type: "running" } } }),
    );
    expect(await fetchActiveSessions(config)).toEqual(new Set(["ses_b"]));
  });

  it("端点不存在时返回 null（退回纯事件模式，不算错误）", async () => {
    const config = await serve("not found", 404);
    expect(await fetchActiveSessions(config)).toBeNull();
  });

  it("501 同样视为不支持", async () => {
    const config = await serve("nope", 501);
    expect(await fetchActiveSessions(config)).toBeNull();
  });

  it("500 抛出并带状态码——真问题不能吞", async () => {
    const config = await serve("boom", 500);
    await expect(fetchActiveSessions(config)).rejects.toThrow(/HTTP 500/);
  });

  it("响应结构不符时报错，不能当成没有活跃会话", async () => {
    const config = await serve(JSON.stringify({ result: {} }));
    await expect(fetchActiveSessions(config)).rejects.toThrow(/缺少 data/);
  });

  it("非法 JSON 报错", async () => {
    const config = await serve("<html>502</html>");
    await expect(fetchActiveSessions(config)).rejects.toThrow(/JSON/);
  });
});

describe("用权威列表覆盖忙碌", () => {
  it("把已在跑的会话补成忙碌", () => {
    const state = new ProtocolState();
    state.titles.set("ses_a", "已经在跑");
    seedBusyFromActive(state, new Set(["ses_a"]));
    expect(state.busy.has("ses_a")).toBe(true);
  });

  it("断线期间跑完的会话会被清掉——必须整体替换而非合并", () => {
    const state = new ProtocolState();
    // 上一次连接推断出的忙碌：任务其实已经在断线期间结束了。
    state.rawBusy.add("ses_done");
    state.applyEffectiveBusy("ses_done");
    expect(state.busy.has("ses_done")).toBe(true);

    seedBusyFromActive(state, new Set());
    expect(state.busy.has("ses_done")).toBe(false);
  });

  it("列表换了对象也能正确跟随", () => {
    const state = new ProtocolState();
    seedBusyFromActive(state, new Set(["ses_a", "ses_b"]));
    seedBusyFromActive(state, new Set(["ses_b", "ses_c"]));
    expect([...state.busy].sort()).toEqual(["ses_b", "ses_c"]);
  });
});