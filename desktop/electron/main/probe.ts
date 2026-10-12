/**
 * 后端探测：给一个还没保存的服务连接判断它跑的是哪种 Agent。
 * 从 desktop/src-tauri/src/monitor/mod.rs 的 probe_backend 迁移。
 */

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

async function getOk(server: ServerConfig, path: string, headers: HeadersInit): Promise<boolean> {
  try {
    const url = new URL(path, serverBaseUrl(server));
    const response = await fetch(url, { headers, signal: AbortSignal.timeout(5000) });
    return response.ok;
  } catch (error) {
    throw new Error(error instanceof Error ? error.message : String(error));
  }
}

async function probeDsh(server: ServerConfig): Promise<string | null> {
  try {
    const response = await fetch(new URL("/api/session.list", serverBaseUrl(server)), {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        type: "client-request",
        rpcId: "probe",
        method: "session.list",
        payload: {},
      }),
      signal: AbortSignal.timeout(5000),
    });
    return response.ok ? null : `HTTP ${response.status}`;
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
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