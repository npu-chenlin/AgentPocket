/**
 * Kimi 监控。从 desktop/src-tauri/src/monitor/kimi.rs 迁移。
 *
 * 流程：拉版本 → 拉会话基线 → 给忙碌会话取详情做种子 → 订阅 WebSocket 收实时事件。
 * 细分状态（主回合是否活跃、后台任务数、待交互）全靠推送，轮询只做启动校准。
 */

import { randomUUID } from "node:crypto";

import { serverBaseUrl, type ServerConfig } from "../model";
import { parseFrame } from "../protocol/kimi";
import { PinTracker, type MonitorContext } from "./manager";

const DEFAULT_TITLE = "点击查看 Kimi 会话";

function wsUrl(server: ServerConfig): string {
  const base = serverBaseUrl(server);
  const scheme = base.protocol === "https:" ? "wss" : "ws";
  return `${scheme}://${base.host}/api/v1/ws`;
}

/**
 * Kimi 把 Bearer 令牌放在 WebSocket 子协议名里（`kimi-code.bearer.<token>`）。
 * Node 的全局 WebSocket 第二参数只接受子协议名，正好对得上。
 */
function wsProtocol(server: ServerConfig): string | string[] | undefined {
  return server.token ? `kimi-code.bearer.${server.token}` : undefined;
}

function authHeaders(server: ServerConfig): Record<string, string> {
  return server.token ? { Authorization: `Bearer ${server.token}` } : {};
}

interface SessionDetail {
  busy: boolean;
  mainTurnActive: boolean;
  pending: string | null;
}

/** 会话详情种子。字段缺失时 `mainTurnActive` 缺省视为活跃。 */
export function parseSessionDetail(text: string): SessionDetail | null {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return null;
  }
  const data = (value as { data?: Record<string, unknown> })?.data;
  if (!data) return null;

  const pendingRaw = typeof data.pending_interaction === "string" ? data.pending_interaction : null;
  return {
    busy: data.busy === true,
    mainTurnActive: typeof data.main_turn_active === "boolean" ? data.main_turn_active : true,
    pending: pendingRaw === "approval" || pendingRaw === "question" ? pendingRaw : null,
  };
}

/** 统计运行中的后台任务，决定"等后台"还是"已完成"。 */
export function countRunningTasks(text: string): number {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return 0;
  }
  const data = (value as { data?: unknown })?.data;
  const items = Array.isArray(data)
    ? data
    : Array.isArray((data as { items?: unknown })?.items)
      ? ((data as { items: unknown[] }).items)
      : null;
  if (!items) return 0;
  return items.filter(
    (task) => (task as { status?: string })?.status === "running",
  ).length;
}

async function fetchJson(
  server: ServerConfig,
  url: string,
  token: { isCancelled: boolean },
): Promise<unknown> {
  if (token.isCancelled) throw new Error("cancelled");
  const response = await fetch(url, {
    headers: authHeaders(server),
    signal: AbortSignal.timeout(10_000),
  });
  if (!response.ok) {
    throw new Error(`HTTP ${response.status} ${url}`);
  }
  return response.json();
}

async function fetchTextOrNull(
  server: ServerConfig,
  url: string,
  token: { isCancelled: boolean },
): Promise<string | null> {
  if (token.isCancelled) return null;
  try {
    const response = await fetch(url, {
      headers: authHeaders(server),
      signal: AbortSignal.timeout(10_000),
    });
    if (!response.ok) return null;
    return await response.text();
  } catch {
    return null;
  }
}

/**
 * 从 `/api/v1/meta` 的响应里取版本号。
 *
 * 字段是 `server_version` 不是 `version`。写错不会报错，只会静默变成 null，
 * 看起来像"这个服务没提供版本信息"——所以这条单独导出并测。
 */
export function parseServerVersion(json: string): string | null {
  const value = JSON.parse(json) as { data?: { server_version?: unknown } };
  const version = value.data?.server_version;
  return typeof version === "string" ? version : null;
}

/**
 * 版本号只是展示信息，拿不到不影响监控。
 */
async function fetchVersion(
  server: ServerConfig,
  base: URL,
  context: MonitorContext,
): Promise<void> {
  const text = await fetchTextOrNull(server, new URL("/api/v1/meta", base).toString(), context.token);
  if (text === null) return;
  let version: string | null;
  try {
    version = parseServerVersion(text);
  } catch (error) {
    throw new Error(`meta 返回的不是合法 JSON: ${(error as Error).message}`);
  }
  context.state.serverVersion = version;
}

/** 拉会话基线，重置本地状态。解析失败要报错，不能当成"没有会话"。 */
async function fetchBaseline(
  server: ServerConfig,
  base: URL,
  context: MonitorContext,
): Promise<void> {
  const url = new URL("/api/v2/sessions", base);
  url.searchParams.append("meta.archived", "false");
  url.searchParams.append("page_size", "100");

  const value = (await fetchJson(server, url.toString(), context.token)) as {
    data?: { items?: unknown[] };
  };
  const items = value?.data?.items;
  if (!Array.isArray(items)) {
    throw new Error("会话列表缺少 data.items");
  }

  const { state } = context;
  state.titles.clear();
  state.busy.clear();
  state.rawBusy.clear();
  state.mainTurnInactive.clear();
  state.bgRunning.clear();
  state.activities.clear();

  for (const raw of items) {
    const item = raw as Record<string, unknown>;
    const id = typeof item.id === "string" ? item.id : null;
    if (id === null) throw new Error("会话条目缺少 id");

    const meta = item.meta as Record<string, unknown> | undefined;
    const rawTitle = typeof meta?.title === "string" ? meta.title : "";
    state.titles.set(id, rawTitle.length === 0 || rawTitle === "null" ? DEFAULT_TITLE : rawTitle);

    const activity = item.activity as Record<string, unknown> | undefined;
    const status = typeof activity?.status === "string" ? activity.status : "idle";
    if (status !== "idle") {
      // 列表只有粗粒度状态；主回合活跃性先默认活跃，由种子校准。
      state.rawBusy.add(id);
      state.applyEffectiveBusy(id);
    }
  }

  state.baselineComplete = true;
}

/** 对忙碌会话取一次详情，校准主回合活跃性与后台任务数。 */
async function seedBusySessions(
  server: ServerConfig,
  base: URL,
  context: MonitorContext,
): Promise<void> {
  const { state } = context;
  for (const id of [...state.busy]) {
    const text = await fetchTextOrNull(
      server,
      new URL(`/api/v1/sessions/${id}`, base).toString(),
      context.token,
    );
    if (text === null) continue;

    const detail = parseSessionDetail(text);
    if (!detail) continue;

    if (detail.busy) state.rawBusy.add(id);
    else state.rawBusy.delete(id);

    if (detail.mainTurnActive) state.mainTurnInactive.delete(id);
    else state.mainTurnInactive.add(id);

    if (!detail.mainTurnActive) {
      // 主回合已结束：取一次后台任务数作为事件计数的初值。
      const tasks = await fetchTextOrNull(
        server,
        new URL(`/api/v1/sessions/${id}/tasks`, base).toString(),
        context.token,
      );
      if (tasks !== null) {
        const running = countRunningTasks(tasks);
        if (running > 0) state.bgRunning.set(id, running);
        else state.bgRunning.delete(id);
      }
    }

    state.applyEffectiveBusy(id);
    if (state.busy.has(id)) {
      state.activityFor(id).pending = detail.pending;
    }
  }
}

function clientHello(sessionIds: string[]): string {
  return JSON.stringify({
    type: "client_hello",
    id: randomUUID(),
    payload: {
      client_id: "agentpocket-desktop",
      subscriptions: sessionIds,
      cursors: {},
    },
  });
}

function subscribe(sessionIds: string[]): string {
  return JSON.stringify({
    type: "subscribe",
    id: randomUUID(),
    payload: { session_ids: sessionIds, cursors: {} },
  });
}

async function runOnce(context: MonitorContext): Promise<void> {
  const { server, state, token } = context;
  const base = serverBaseUrl(server);

  await fetchVersion(server, base, context);
  await fetchBaseline(server, base, context);
  await seedBusySessions(server, base, context);

  const pins = new PinTracker(context.pinned);
  pins.emitFinished(context);
  context.emitStatus({ connected: false, error: null });

  const socket = new WebSocket(wsUrl(server), wsProtocol(server));
  await waitForOpen(socket, token);
  context.emitStatus({ connected: true, error: null });

  const sessionIds = [...state.titles.keys()];
  socket.send(clientHello(sessionIds));
  state.subscribed.clear();
  for (const id of sessionIds) state.subscribed.add(id);

  // 定期 ping，避免服务端/中间设备因空闲断开。
  // 定期重发心跳，防止服务端/中间设备因空闲断开。
  // Node 的全局 WebSocket 没有 ping()，发一个应用层 ping 帧即可。
  const keepalive = setInterval(() => {
    if (socket.readyState === WebSocket.OPEN) socket.send(JSON.stringify({ type: "ping" }));
  }, 20_000);

  try {
    for (;;) {
      if (token.isCancelled) break;
      const message = await nextText(socket, token);
      if (message === null) break; // 连接关闭

      await handleMessage(message, server, base, context, socket);
      pins.emitFinished(context);

      // 新出现的会话补进订阅。
      const fresh = [...state.titles.keys()].filter((id) => !state.subscribed.has(id));
      if (fresh.length > 0) {
        socket.send(subscribe(fresh));
        for (const id of fresh) state.subscribed.add(id);
      }
      context.emitStatus({ connected: true, error: null });
    }
  } finally {
    clearInterval(keepalive);
    socket.close();
  }
}

function waitForOpen(socket: WebSocket, token: { isCancelled: boolean }): Promise<void> {
  return new Promise((resolve, reject) => {
    if (token.isCancelled) {
      reject(new Error("cancelled"));
      return;
    }
    socket.addEventListener("open", () => resolve(), { once: true });
    socket.addEventListener("error", () => reject(new Error("WebSocket 连接失败")), {
      once: true,
    });
  });
}

/** 等下一条文本帧；连接关闭返回 null。 */
function nextText(socket: WebSocket, token: { isCancelled: boolean }): Promise<string | null> {
  return new Promise((resolve, reject) => {
    if (token.isCancelled) {
      reject(new Error("cancelled"));
      return;
    }
    const cleanup = () => {
      socket.removeEventListener("message", onMessage);
      socket.removeEventListener("close", onClose);
      socket.removeEventListener("error", onError);
    };
    const onMessage = (event: MessageEvent) => {
      if (typeof event.data === "string") {
        cleanup();
        resolve(event.data);
      }
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
}

async function handleMessage(
  text: string,
  server: ServerConfig,
  base: URL,
  context: MonitorContext,
  socket: WebSocket,
): Promise<void> {
  let message: Record<string, unknown>;
  try {
    message = JSON.parse(text) as Record<string, unknown>;
  } catch (error) {
    throw new Error(`WebSocket 帧不是合法 JSON: ${(error as Error).message}`);
  }

  const type = typeof message.type === "string" ? message.type : "";
  const payload = message.payload as Record<string, unknown> | undefined;

  switch (type) {
    case "ping": {
      const nonce = payload?.nonce ?? randomUUID();
      socket.send(JSON.stringify({ type: "pong", payload: { nonce } }));
      return;
    }
    case "resync_required": {
      // 服务端要求重新同步：重拉基线并重建订阅。
      await fetchBaseline(server, base, context);
      await seedBusySessions(server, base, context);
      const ids = [...context.state.titles.keys()];
      socket.send(subscribe(ids));
      context.state.subscribed.clear();
      for (const id of ids) context.state.subscribed.add(id);
      return;
    }
    default: {
      for (const event of parseFrame(server.id, text, new Date(), context.state)) {
        context.emitEvent(event);
      }
    }
  }
}

export async function runKimi(context: MonitorContext): Promise<void> {
  await runOnce(context);
}