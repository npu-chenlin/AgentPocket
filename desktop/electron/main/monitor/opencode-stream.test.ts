/**
 * 事件流的超时语义。
 *
 * 这组测试是为了锁住一条踩过的坑：把连接超时挂在 fetch 上会连带掐掉读流。
 * opencode 空闲时 15 秒才发一次 `: heartbeat`，超时与心跳同频时表现为每 15 秒
 * 掉线一次（而且重连间隙的忙闲事件全丢，表现为"正在运行的会话不显示"）。
 *
 * 用本地 HTTP 服务模拟慢心跳，用很小的超时值把同样的场景压缩到毫秒级。
 */

import { createServer, type Server } from "node:http";
import { afterEach, describe, expect, it } from "vitest";

import { openEventStream } from "./opencode";
import { CancelToken } from "./manager";
import type { ServerConfig } from "../model";

const server: ServerConfig = {
  id: "s1",
  name: "本地",
  host: "127.0.0.1",
  port: 0, // 由测试服务实际监听端口填回
  backend: "opencode",
  token: "pw",
};

let running: Server | null = null;

/** 结束所有挂着的连接。SSE 响应永不结束，不主动断开会一直占着端口。 */
async function shutdown(http: Server): Promise<void> {
  http.closeAllConnections();
  await new Promise<void>((resolve) => http.close(() => resolve()));
}

afterEach(async () => {
  if (running) {
    await shutdown(running);
    running = null;
  }
});

/**
 * 起一个 SSE 服务。
 *
 * `headerDelayMs`：多久之后才发响应头（模拟建连慢）。
 * `heartbeatMs`：响应头之后每隔多久发一行心跳（0 表示发完头就沉默）。
 */
async function startSse(options: {
  headerDelayMs?: number;
  heartbeatMs?: number;
  status?: number;
}): Promise<{ config: ServerConfig; close: () => Promise<void> }> {
  const { headerDelayMs = 0, heartbeatMs = 0, status = 200 } = options;

  const http = createServer((req, res) => {
    setTimeout(() => {
      res.writeHead(status, { "Content-Type": "text/event-stream" });
      res.flushHeaders();
      if (heartbeatMs > 0) {
        const timer = setInterval(() => res.write(": heartbeat\n\n"), heartbeatMs);
        res.on("close", () => clearInterval(timer));
      }
    }, headerDelayMs);
  });

  await new Promise<void>((resolve) => http.listen(0, "127.0.0.1", resolve));
  const address = http.address();
  if (typeof address !== "object" || address === null) throw new Error("拿不到监听端口");
  running = http;

  return {
    config: { ...server, port: address.port },
    close: () => shutdown(http),
  };
}

describe("opencode 事件流超时", () => {
  it("响应头迟迟不来才超时", async () => {
    const { config } = await startSse({ headerDelayMs: 400 });
    const token = new CancelToken();
    await expect(openEventStream(config, token, 100)).rejects.toThrow(/连接超时/);
  });

  it("响应头已到之后，慢心跳不会把流掐断", async () => {
    // 超时 80ms，心跳 250ms：流必须活过心跳间隔，否则就是每 15 秒掉线的翻版。
    const { config } = await startSse({ heartbeatMs: 250 });
    const token = new CancelToken();
    const { response, dispose } = await openEventStream(config, token, 80);
    try {
      const reader = response.body!.getReader();
      const decoder = new TextDecoder();
      // 等到心跳真的到达——若流被超时掐断，这里会抛 AbortError。
      const { value } = await reader.read();
      expect(decoder.decode(value)).toContain("heartbeat");
      await reader.cancel();
    } finally {
      dispose();
    }
  });

  it("建连后服务端彻底沉默也不误判断线（直到被取消）", async () => {
    // 心跳间隔比超时长得多，正是线上遇到的形态。
    const { config } = await startSse({ heartbeatMs: 0 });
    const token = new CancelToken();
    const { response, dispose } = await openEventStream(config, token, 50);
    try {
      const reader = response.body!.getReader();
      let aborted = false;
      // 超过超时值之后仍能挂住读，说明没有被静默 abort。
      const read = reader.read().then(
        () => "data",
        () => {
          aborted = true;
          return "aborted";
        },
      );
      await new Promise((resolve) => setTimeout(resolve, 200));
      expect(aborted).toBe(false);
      void read;
      await reader.cancel().catch(() => {});
    } finally {
      dispose();
    }
  });

  it("取消信号能立刻打断读流", async () => {
    const { config } = await startSse({ heartbeatMs: 0 });
    const token = new CancelToken();
    const { response, dispose } = await openEventStream(config, token, 5000);
    const reader = response.body!.getReader();
    const read = reader.read().then(
      () => "resolved",
      () => "rejected",
    );
    token.cancel();
    expect(await read).toBe("rejected");
    dispose();
  });

  it("非 2xx 直接报错并带上状态码", async () => {
    const { config } = await startSse({ status: 503 });
    const token = new CancelToken();
    await expect(openEventStream(config, token, 2000)).rejects.toThrow(/HTTP 503/);
  });
});