/**
 * 后端探测：给一个还没保存的服务连接判断它跑的是哪种 Agent。
 * 从 desktop/src-tauri/src/monitor/mod.rs 的 probe_backend 迁移。
 *
 * dsh 0.2 起 `/api` 要求浏览器会话 cookie，而探测发生在服务还没保存、也就还没
 * 法取 cookie 的时候。所以 dsh 改用**免鉴权**指纹：`/manifest.webmanifest` 返回
 * dsh 自己的 JSON（`name: "DeepSeek Harness"`），kimi 上是 404、opencode 上是
 * 一段 HTML，互不混淆。带 cookie 时再补一次 `/api/session/list` 确认。
 */

import { dshCookieHeaders } from "./dsh-auth";
import { serverBaseUrl, type Backend, type ServerConfig } from "./model";

export class ProbeError extends Error {
  constructor(
    message: string,
    readonly detail: {
      dsh: string | null;
      kimi: string | null;
      opencode: string | null;
    },
  ) {
    super(message);
    this.name = "ProbeError";
  }
}

/** dsh 对未认证请求的 401 正文明文，作为 manifest 指纹之外的兜底。 */
const DSH_UNAUTHENTICATED_MARKER = "dsh web authentication required";

/** `/manifest.webmanifest` 是否是 dsh 的清单。kimi 404、opencode 返回 HTML。 */
export function isDshManifest(status: number, body: string): boolean {
  if (status !== 200) return false;
  try {
    const value = JSON.parse(body) as { name?: unknown; short_name?: unknown };
    return value.name === "DeepSeek Harness" || value.short_name === "DSH";
  } catch {
    return false;
  }
}

/** 认证挑战：返回 401 且正文是 dsh 那句话。 */
export function isDshUnauthenticated(status: number, body: string): boolean {
  return status === 401 && body.includes(DSH_UNAUTHENTICATED_MARKER);
}

async function probeDsh(server: ServerConfig): Promise<string | null> {
  const base = serverBaseUrl(server);
  try {
    const response = await fetch(new URL("/manifest.webmanifest", base), {
      signal: AbortSignal.timeout(5000),
    });
    const body = await response.text();
    if (isDshManifest(response.status, body)) return null;
    // 401 说明这个地址后面确实是 dsh，只是我们还没带 cookie。
    if (isDshUnauthenticated(response.status, body)) return null;
    return `HTTP ${response.status}`;
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
}

/** 带 cookie 的权威确认：能拉到会话列表才算数。 */
export async function probeDshAuthenticated(server: ServerConfig): Promise<boolean> {
  try {
    const headers = dshCookieHeaders(server);
    if (!("Cookie" in headers)) return false;
    const response = await fetch(new URL("/api/session/list", serverBaseUrl(server)), {
      method: "POST",
      headers: { "Content-Type": "application/json", ...headers },
      body: JSON.stringify({
        type: "client-request",
        rpcId: "probe",
        method: "session/list",
        payload: { args: { _request: {} } },
      }),
      signal: AbortSignal.timeout(5000),
    });
    return response.ok;
  } catch {
    return false;
  }
}

async function probeKimi(server: ServerConfig): Promise<string | null> {
  try {
    const headers: Record<string, string> = server.token
      ? { Authorization: `Bearer ${server.token}` }
      : {};
    const response = await fetch(new URL("/api/v2/sessions", serverBaseUrl(server)), {
      headers,
      signal: AbortSignal.timeout(5000),
    });
    return response.ok ? null : `HTTP ${response.status}`;
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
}

async function probeOpencode(server: ServerConfig): Promise<string | null> {
  try {
    const raw = server.token.trim();
    if (raw.length === 0) return "缺少凭据";
    const colon = raw.indexOf(":");
    const user = colon < 0 ? "opencode" : raw.slice(0, colon).trim();
    const pass = colon < 0 ? raw : raw.slice(colon + 1);
    const response = await fetch(new URL("/api/session?limit=1", serverBaseUrl(server)), {
      headers: {
        Authorization: `Basic ${Buffer.from(`${user}:${pass}`, "utf8").toString("base64")}`,
        Accept: "application/json",
      },
      signal: AbortSignal.timeout(5000),
    });
    return response.ok ? null : `HTTP ${response.status}`;
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
}

/** 三种后端都探一遍，先命中的就是它。 */
export async function probeBackend(server: ServerConfig): Promise<Backend> {
  const [dsh, kimi, opencode] = await Promise.all([
    probeDsh(server),
    probeKimi(server),
    probeOpencode(server),
  ]);

  if (dsh === null) return "dsh";
  if (kimi === null) return "kimi";
  if (opencode === null) return "opencode";

  throw new ProbeError(
    "无法识别 Agent 类型：三种后端的探测都没通过",
    { dsh, kimi, opencode },
  );
}
