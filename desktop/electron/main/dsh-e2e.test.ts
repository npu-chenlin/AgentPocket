/**
 * @vitest-environment node
 *
 * 必须跑在 Node 里：这里要连真的 dsh，而 Cookie 只能走 Node WebSocket 的
 * `headers` 选项——jsdom 那个 WebSocket 构造函数没有这个参数（只有子协议），
 * 会直接抛 "Invalid Sec-WebSocket-Protocol value"。
 */

import { describe, expect, it } from "vitest";

import { dshCookie, dshCookieHeaders, exchangeLaunchToken, extractLaunchToken } from "./dsh-auth";
import { probeBackend, probeDshAuthenticated } from "./probe";
import type { ServerConfig } from "./model";
import { parseFollowFrame, parseSessionList } from "./protocol/dsh";
import { ProtocolState } from "./protocol/state";

/**
 * 对着**真** dsh（0.2.0-rc.2）跑一遍完整链路。
 *
 * 只在环境里给了地址时才跑：`DSH_E2E=host:port npx vitest run dsh-e2e`。
 * 默认跳过，免得 CI 上没起 dsh 就红。
 *
 * 凭据走 dsh 自己的正规入口：把启动行（或其中的 launch token，用 DSH_LAUNCH 传）换成一个
 * 会话 cookie。客户端不碰 `~/.dsh/.credentials.yaml`，也不复刻它的 HMAC 签名。
 */
const target = process.env.DSH_E2E ?? "";
const [host, port] = target.split(":");
const launchToken = process.env.DSH_LAUNCH ?? "";

const server: ServerConfig = {
  id: "e2e",
  name: "dsh",
  host,
  port: Number(port ?? 0),
  token: "",
  backend: "dsh",
};

/** 连上之前先把 launch token 换成 cookie 存进 server.token，后续用例共用。 */
async function ensureCookie(): Promise<void> {
  if (server.token !== "") return;
  if (launchToken === "") throw new Error("需要 DSH_LAUNCH=<启动行或 launch token>");
  const token = extractLaunchToken(launchToken) ?? launchToken;
  const cookie = await exchangeLaunchToken(server, token);
  if (cookie === null) throw new Error("launch token 换不到 cookie（可能已过期）");
  server.token = cookie.header;
}

/** 与 monitor/dsh.ts 的 openMux 同一套：Cookie 走 headers。 */
async function openMux(config: ServerConfig): Promise<WebSocket> {
  const headers = dshCookieHeaders(config);
  const ctor = WebSocket as unknown as new (
    url: string,
    init: { headers: Record<string, string> },
  ) => WebSocket;
  const socket = new ctor(`ws://${config.host}:${config.port}/api/remote.mux`, { headers });
  await new Promise<void>((resolve, reject) => {
    socket.addEventListener("open", () => resolve(), { once: true });
    socket.addEventListener("error", () => reject(new Error("WebSocket 连接失败")), { once: true });
  });
  return socket;
}

async function fetchSessions(): Promise<ProtocolState> {
  const headers = dshCookieHeaders(server);
  const response = await fetch(`http://${server.host}:${server.port}/api/session/list`, {
    method: "POST",
    headers: { "Content-Type": "application/json", ...headers },
    body: JSON.stringify({
      type: "client-request",
      rpcId: "e2e",
      method: "session/list",
      payload: { args: { _request: {} } },
    }),
  });
  expect(response.ok).toBe(true);
  const state = new ProtocolState();
  parseSessionList(await response.text(), state);
  return state;
}

function firstFrame(socket: WebSocket): Promise<string> {
  return new Promise((resolve, reject) => {
    socket.addEventListener("message", (event) => resolve(String(event.data)));
    socket.addEventListener("error", () => reject(new Error("ws error")));
    setTimeout(() => reject(new Error("没有收到帧")), 8000);
  });
}

describe.skipIf(!target)("dsh 0.2.0 端到端", () => {
  it("launch token 能换出可用 cookie", async () => {
    await ensureCookie();
    const cookie = dshCookie(server);
    expect(cookie, "换到的 cookie 解析不出来").not.toBeNull();
    expect(cookie!.header).toMatch(/^dsh-auth-[A-Za-z0-9_-]+=v1\./);
  });

  it("manifest 指纹把它认成 dsh", async () => {
    await expect(probeBackend(server)).resolves.toBe("dsh");
  });

  it("带 cookie 能拉到会话列表", async () => {
    await ensureCookie();
    await expect(probeDshAuthenticated(server)).resolves.toBe(true);
  });

  it("session/list 解析出会话，$events 能订阅", async () => {
    await ensureCookie();
    const state = await fetchSessions();
    expect(state.baselineComplete).toBe(true);

    const socket = await openMux(server);
    try {
      socket.send(JSON.stringify({ type: "open", streamId: "e2e", endpoint: "$events", payload: { args: {} } }));
      const parsed = JSON.parse(await firstFrame(socket)) as { value?: { type?: string; clientId?: string } };
      expect(parsed.value?.type).toBe("ready");
      expect(typeof parsed.value?.clientId).toBe("string");
    } finally {
      socket.close();
    }
  });

  it("跟随一个会话能收到快照，且快照不产出事件", async () => {
    await ensureCookie();
    const state = await fetchSessions();
    // 只跟随非子代理会话：监控本身就不跟随子代理（跟随一个子代理不会得到 snapshot），
    // 所以测试也必须按同一规则挑，否则挑中子代理时会对着真实服务误报失败。
    const session = [...state.titles.keys()].find((id) => !state.subagentSessions.has(id));
    if (session === undefined) return;

    const socket = await openMux(server);
    try {
      socket.send(JSON.stringify({
        type: "open",
        streamId: "follow",
        endpoint: "session/follow",
        payload: { args: { request: { address: { kind: "session", sessionId: session } } } },
      }));
      const frame = await firstFrame(socket);
      const parsed = JSON.parse(frame) as { value?: { type?: string } };
      expect(parsed.value?.type).toBe("snapshot");
      // 快照回放的历史不产出事件（否则重连会刷一堆旧账）。
      expect(parseFollowFrame("e2e", session, frame, state)).toEqual([]);
    } finally {
      socket.close();
    }
  });
});
