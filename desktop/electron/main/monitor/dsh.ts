/**
 * dsh（DeepSeek Harness）监控 —— 0.2 版。
 *
 * 0.1 的模型是"一条全局下游流收所有事件"。0.2 拆成三条，这里按需组合：
 *
 *   1. 启动时 `session/list` 拉一次会话集合与 running 标志
 *   2. 常连 `$events`：会话增删、running 变化、agent 失败、审批/提问请求
 *   3. 对每个**非子代理**会话开一条 `session/follow`，收 turn/start、turn/end
 *
 * 为什么第 3 条要按会话开：0.2 没有会话级事件的全局流。只订阅 running 变化
 * 能撑起"活跃会话"列表，但拿不到 turn/end 的结束原因（完成还是失败）。
 *
 * 两条注意：
 *   - 跟随一个冷会话会把它**激活**（服务端 `promote()`），所以只跟随列表里
 *     出现的、非子代理的会话，不预先扫全部。
 *   - `$events` 的审批/提问是 waterfall，**必须回一条**，否则服务端一直等，
 *     真人那边的审批就卡住了。这里只回 `next`，把决定权留给 Web UI。
 */

import { randomUUID } from "node:crypto";

import { dshCookieHeaders } from "../dsh-auth";
import { serverBaseUrl, type ServerConfig } from "../model";
import {
  dshWaterfallReply,
  parseEventsFrame,
  parseEventsReady,
  parseFollowFrame,
  parseSessionList,
  waterfallEvent,
  type DshSession,
} from "../protocol/dsh";
import { PinTracker, type MonitorContext } from "./manager";

/** follow 连接的上限：会话再多也不该把文件描述符吃光。 */
const MAX_FOLLOWS = 24;
/** `$events` 挂了要重连，重连前重新校准一次列表。 */
const LIST_REFRESH_MS = 60_000;

function wsUrl(server: ServerConfig, path: string): string {
  const base = serverBaseUrl(server);
  const scheme = base.protocol === "https:" ? "wss" : "ws";
  return `${scheme}://${base.host}/${path}`;
}

interface RpcOptions {
  method: string;
  path: string;
  args: Record<string, unknown>;
}

async function rpc(
  server: ServerConfig,
  { method, path, args }: RpcOptions,
): Promise<string> {
  const headers = dshCookieHeaders(server);
  const response = await fetch(new URL(`/api/${path}`, serverBaseUrl(server)), {
    method: "POST",
    headers: { "Content-Type": "application/json", ...headers },
    body: JSON.stringify({
      type: "client-request",
      rpcId: randomUUID(),
      method,
      payload: { args },
    }),
    signal: AbortSignal.timeout(10_000),
  });
  if (response.status === 401) {
    // 配置里的密钥不对（dsh 重装换了密钥等），症状就是 401。
    throw new Error("dsh 认证失败：检查该服务连接里填的密钥");
  }
  if (!response.ok) {
    throw new Error(`${method} 请求失败：HTTP ${response.status}`);
  }
  return response.text();
}

/** session/list 的参数名是 `_request`（描述符里的 wire 名），不是 `request`。 */
async function fetchSessions(
  server: ServerConfig,
  context: MonitorContext,
): Promise<DshSession[]> {
  const body = await rpc(server, {
    method: "session/list",
    path: "session/list",
    args: { _request: {} },
  });
  return parseSessionList(body, context.state);
}

/**
 * 打开一条 mux 连接。Cookie 必须走 headers——浏览器的 WebSocket 构造器没有这个
 * 选项（只有子协议），而这里跑在 Electron 主进程的 Node 上，它是支持的。
 *
 * DOM 的类型定义不认识这个选项，所以显式声明一次；运行时已实测生效。
 */
interface NodeWebSocketInit {
  headers?: Record<string, string>;
}

function openSocket(url: string, init: NodeWebSocketInit): WebSocket {
  const ctor = WebSocket as unknown as new (url: string, init: NodeWebSocketInit) => WebSocket;
  return new ctor(url, init);
}

async function openMux(server: ServerConfig): Promise<WebSocket> {
  const headers = dshCookieHeaders(server);
  const socket = openSocket(wsUrl(server, "api/remote.mux"), { headers });
  await waitForOpen(socket);
  return socket;
}
/** mux 上的第 0 帧必须是 open，之后服务端才会推 item。 */
function subscribe(socket: WebSocket, streamId: string, endpoint: string, args: object): void {
  socket.send(JSON.stringify({ type: "open", streamId, endpoint, payload: { args } }));
}

function waitForOpen(socket: WebSocket): Promise<void> {
  return new Promise((resolve, reject) => {
    socket.addEventListener("open", () => resolve(), { once: true });
    socket.addEventListener("error", () => reject(new Error("WebSocket 连接失败")), {
      once: true,
    });
  });
}

/** 读一帧文本；连接关闭返回 null。 */
function nextText(socket: WebSocket): Promise<string | null> {
  return new Promise((resolve, reject) => {
    const cleanup = (): void => {
      socket.removeEventListener("message", onMessage);
      socket.removeEventListener("close", onClose);
      socket.removeEventListener("error", onError);
    };
    const onMessage = (event: MessageEvent): void => {
      cleanup();
      // dsh 只发文本帧；二进制帧（不应出现）当空帧丢掉。
      resolve(typeof event.data === "string" ? event.data : "");
    };
    const onClose = (): void => {
      cleanup();
      resolve(null);
    };
    const onError = (): void => {
      cleanup();
      reject(new Error("WebSocket 错误"));
    };
    socket.addEventListener("message", onMessage);
    socket.addEventListener("close", onClose);
    socket.addEventListener("error", onError);
  });
}

/**
 * 单会话事件订阅。断线由外层重连；这里只管把帧喂给协议层。
 */
async function followSession(
  server: ServerConfig,
  sessionId: string,
  context: MonitorContext,
  onDone: (sessionId: string) => void,
): Promise<void> {
  const socket = await openMux(server);
  const streamId = `follow-${sessionId}`;
  try {
    subscribe(socket, streamId, "session/follow", {
      request: { address: { kind: "session", sessionId } },
    });
    for (;;) {
      if (context.token.isCancelled) break;
      const text = await nextText(socket);
      if (text === null) break;
      if (text === "") continue;
      for (const event of parseFollowFrame(server.id, sessionId, text, context.state)) {
        context.emitEvent(event);
      }
      context.emitStatus({ connected: true, error: null });
    }
  } finally {
    socket.close();
    onDone(sessionId);
  }
}

/**
 * 全局事件订阅：会话集合与 running 变化、审批/提问。
 *
 * 返回时连接已断（给外层做退避重连）。
 */
async function runEvents(server: ServerConfig, context: MonitorContext): Promise<void> {
  const socket = await openMux(server);
  context.emitStatus({ connected: true, error: null });
  let clientId: string | null = null;
  try {
    subscribe(socket, "events", "$events", {});
    for (;;) {
      if (context.token.isCancelled) break;
      const text = await nextText(socket);
      if (text === null) break;
      if (text === "") continue;

      const ready = parseReady(text);
      if (ready !== null) {
        clientId = ready;
        continue;
      }

      const { events, waterfalls } = parseEventsFrame(server.id, text, context.state);
      for (const event of events) context.emitEvent(event);
      for (const waterfall of waterfalls) {
        const event = waterfallEvent(server.id, waterfall, context.state);
        context.emitEvent(event);
        // 必须应答，否则服务端一直等，真人侧审批被我们拖住。
        if (clientId !== null) {
          socket.send(dshWaterfallReply(clientId, waterfall));
        }
      }
      context.emitStatus({ connected: true, error: null });
    }
  } finally {
    socket.close();
  }
}

/** ready 帧里取 clientId；不是 ready 就返回 null。 */
function parseReady(text: string): string | null {
  try {
    const value = JSON.parse(text) as unknown;
    if (typeof value !== "object" || value === null) return null;
    const frame = value as Record<string, unknown>;
    if (frame.type !== "item") return null;
    const inner = frame.value;
    if (typeof inner !== "object" || inner === null) return null;
    return parseEventsReady(inner as Record<string, unknown>);
  } catch {
    return null;
  }
}

/**
 * 一轮监控：拉列表 → 订阅全局事件 → 给每个会话开 follow。
 *
 * 全局事件流断开就整轮结束（外层退避重连），因为 running 状态已经不可信了。
 */
async function runOnce(context: MonitorContext): Promise<void> {
  const { server, state, token } = context;

  const sessions = await fetchSessions(server, context);
  const pins = new PinTracker(context.pinned);
  pins.emitFinished(context);

  // 只跟随非子代理会话；跟随会激活冷会话，所以别扫无关的。
  const followable = sessions
    .filter((s) => !s.subagent)
    .slice(0, MAX_FOLLOWS)
    .map((s) => s.sessionId);

  const follows = new Map<string, Promise<void>>();
  const startFollow = (sessionId: string): void => {
    if (follows.has(sessionId)) return;
    const promise = followSession(server, sessionId, context, (id) => follows.delete(id)).catch(
      (error: unknown) => {
        follows.delete(sessionId);
        if (!token.isCancelled) {
          context.emitStatus({
            connected: true,
            error: `会话 ${sessionId} 订阅失败：${error instanceof Error ? error.message : String(error)}`,
          });
        }
      },
    );
    follows.set(sessionId, promise);
  };
  for (const sessionId of followable) startFollow(sessionId);

  const refresh = setInterval(() => {
    void fetchSessions(server, context)
      .then((latest) => {
        for (const session of latest) {
          if (!session.subagent) startFollow(session.sessionId);
        }
      })
      .catch(() => {
        // 列表刷新失败不影响已建立的订阅；下一轮或重连时再校正。
      });
  }, LIST_REFRESH_MS);

  try {
    await runEvents(server, context);
  } finally {
    clearInterval(refresh);
    // 全局流断了，逐会话的订阅也没有意义了——一并收掉，等重连重建。
    await Promise.allSettled([...follows.values()]);
  }
}

export async function runDsh(context: MonitorContext): Promise<void> {
  await runOnce(context);
}
