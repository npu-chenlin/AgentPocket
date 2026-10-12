/**
 * dsh（DeepSeek Harness）监控。
 * 从 desktop/src-tauri/src/monitor/dsh.rs 迁移。
 *
 * 流程：HTTP RPC 拉会话列表 → 订阅 WebSocket 收事件。
 */

import { randomUUID } from "node:crypto";

import { serverBaseUrl, type ServerConfig } from "../model";
import { parseFrame, parseSessionList } from "../protocol/dsh";
import { PinTracker, type MonitorContext } from "./manager";

function wsUrl(server: ServerConfig): string {
  const base = serverBaseUrl(server);
  const scheme = base.protocol === "https:" ? "wss" : "ws";
  return `${scheme}://${base.host}/api/events.mux`;
}

/** dsh 用 JSON-RPC over HTTP，方法名带斜杠。 */
async function fetchSessionList(server: ServerConfig, context: MonitorContext): Promise<void> {
  const url = new URL("/api/session.list", serverBaseUrl(server));
  const response = await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      type: "client-request",
      rpcId: randomUUID(),
      method: "session.list",
      payload: {},
    }),
    signal: AbortSignal.timeout(10_000),
  });
  if (!response.ok) {
    throw new Error(`session.list 请求失败：HTTP ${response.status}`);
  }
  parseSessionList(await response.text(), context.state);
}

async function runOnce(context: MonitorContext): Promise<void> {
  const { server, state, token } = context;

  await fetchSessionList(server, context);
  const pins = new PinTracker(context.pinned);
  // 重连间隙里跑完的置顶会话，在列表校准后补一次完成跃迁检测。
  pins.emitFinished(context);
  context.emitStatus({ connected: false, error: null });

  const socket = new WebSocket(wsUrl(server));
  await new Promise<void>((resolve, reject) => {
    if (token.isCancelled) {
      reject(new Error("cancelled"));
      return;
    }
    socket.addEventListener("open", () => resolve(), { once: true });
    socket.addEventListener("error", () => reject(new Error("WebSocket 连接失败")), {
      once: true,
    });
  });
  context.emitStatus({ connected: true, error: null });

  // Node 的全局 WebSocket 没有 ping()，dsh 也没有应用层心跳，
  // 只能靠 TCP 层保活——这里不发任何东西，定时器仅用于让取消检查不被优化掉。
  const keepalive = setInterval(() => {
    if (token.isCancelled) clearInterval(keepalive);
  }, 20_000);

  try {
    for (;;) {
      if (token.isCancelled) break;
      const message = await new Promise<string | null>((resolve, reject) => {
        const cleanup = () => {
          socket.removeEventListener("message", onMessage);
          socket.removeEventListener("close", onClose);
          socket.removeEventListener("error", onError);
        };
        const onMessage = (event: MessageEvent) => {
          cleanup();
          // dsh 只发文本帧，二进制直接忽略。
          resolve(typeof event.data === "string" ? event.data : null);
        };
        const onClose = () => {
          cleanup();
          resolve(null);
        };
        const onError = () => {
          cleanup();
          reject(new Error("WebSocket 错误"));
        };
        socket.addEventListener("message", onMessage);
        socket.addEventListener("close", onClose);
        socket.addEventListener("error", onError);
      });
      if (message === null) break;

      for (const event of parseFrame(server.id, message, state)) {
        context.emitEvent(event);
      }
      pins.emitFinished(context);
      context.emitStatus({ connected: true, error: null });
    }
  } finally {
    clearInterval(keepalive);
    socket.close();
  }
}

export async function runDsh(context: MonitorContext): Promise<void> {
  await runOnce(context);
}