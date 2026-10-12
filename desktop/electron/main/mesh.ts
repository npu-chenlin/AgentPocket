/**
 * mesh：管理同一 Tailnet 内其它 AgentPocket 节点。
 * 从 desktop/src-tauri/src/mesh.rs 迁移。
 *
 * 节点之间靠 daemon 在 MESH_PORT 上暴露的 HTTP 接口通信，这里只做客户端。
 */

import { hostname } from "node:os";

import { MESH_PORT, discover, type MeshPeer } from "./discovery";
import * as kimiConfig from "./kimi-config";

const MESH_HTTP_TIMEOUT_MS = 5_000;
/** 重启 kimi web：带活跃会话时停止阶段可能要几十秒。 */
const RESTART_TIMEOUT_MS = 150_000;
const UPGRADE_TIMEOUT_MS = 660_000;

export interface MeshPeerView {
  name: string;
  host: string;
  version: string | null;
  online: boolean;
  manual: boolean;
  kimiVersion: string | null;
  webActive: boolean;
  webPort: number | null;
}

export interface KimiSyncResult {
  bytes: number;
  /** 覆盖前是否备份了旧文件（仅 pull 有意义）。 */
  backedUp: boolean;
}

interface MeshResponse {
  status: number;
  body: string;
}

async function meshGet(
  host: string,
  path: string,
  headers: Record<string, string>,
  timeoutMs: number,
): Promise<MeshResponse> {
  const response = await fetch(`http://${host}:${MESH_PORT}${path}`, {
    headers,
    signal: AbortSignal.timeout(timeoutMs),
  });
  return { status: response.status, body: await response.text() };
}

async function meshPost(
  host: string,
  path: string,
  headers: Record<string, string>,
  body: string,
  timeoutMs: number,
): Promise<MeshResponse> {
  const response = await fetch(`http://${host}:${MESH_PORT}${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json", ...headers },
    body,
    signal: AbortSignal.timeout(timeoutMs),
  });
  return { status: response.status, body: await response.text() };
}

/** <home>/.kimi-code/config.toml。 */
export function kimiConfigPath(): string {
  return kimiConfig.configPath();
}

/**
 * 补齐展示用的 Kimi 状态。
 *
 * 发现阶段只确认了"这是个节点"，这里再问一次 /kimi-info 拿 Kimi Code CLI
 * 与 kimi web 的状态。拿不到不算失败——节点可能压根没装 Kimi。
 */
async function decorate(peer: MeshPeer): Promise<MeshPeerView> {
  const base: MeshPeerView = {
    ...peer,
    online: true,
    kimiVersion: null,
    webActive: false,
    webPort: null,
  };
  try {
    const response = await meshGet(peer.host, "/kimi-info", {}, MESH_HTTP_TIMEOUT_MS);
    if (response.status !== 200) return base;
    const value = JSON.parse(response.body) as {
      kimi_version?: string;
      web_active?: boolean;
      web_port?: number;
    };
    return {
      ...base,
      kimiVersion: value.kimi_version ?? null,
      webActive: value.web_active === true,
      webPort: typeof value.web_port === "number" ? value.web_port : null,
    };
  } catch {
    // 没装 Kimi 或端点不可用，保留已知的在线状态即可。
    return base;
  }
}

/** 发现节点并补齐状态：Tailscale 在线设备 + peers.json 里的手动节点。 */
export async function discoverPeers(configDir: string): Promise<MeshPeerView[]> {
  const peers = await discover(configDir);
  return Promise.all(peers.map(decorate));
}

/** 从对端拉取 config.toml。写入前会校验语法，避免把坏配置落到本地。 */
export async function meshPull(host: string): Promise<KimiSyncResult> {
  const response = await meshGet(host, "/kimi-config", {}, MESH_HTTP_TIMEOUT_MS);
  if (response.status !== 200) {
    throw new Error(`对方返回 HTTP ${response.status}：${response.body}`);
  }
  const { backedUp } = kimiConfig.write(response.body);
  return { bytes: Buffer.byteLength(response.body), backedUp };
}

/** 把本地 config.toml 推送给对端。 */
export async function meshPush(host: string): Promise<KimiSyncResult> {
  const text = kimiConfig.read();
  const response = await meshPost(
    host,
    "/kimi-config",
    { "X-AgentPocket-Source": hostname() },
    text,
    MESH_HTTP_TIMEOUT_MS,
  );
  if (response.status !== 200) {
    throw new Error(`对方返回 HTTP ${response.status}：${response.body}`);
  }
  return { bytes: Buffer.byteLength(text), backedUp: false };
}

/** 升级对端的 Kimi Code CLI。耗时可达数分钟。 */
export async function meshKimiUpgrade(host: string): Promise<string> {
  const response = await meshPost(host, "/kimi-upgrade", {}, "{}", UPGRADE_TIMEOUT_MS);
  if (response.status !== 200) {
    throw new Error(`对方返回 HTTP ${response.status}：${response.body}`);
  }
  let value: { before?: string; after?: string };
  try {
    value = JSON.parse(response.body) as { before?: string; after?: string };
  } catch (error) {
    throw new Error(`升级应答解析失败：${(error as Error).message}`);
  }
  if (value.before && value.after) {
    return value.before === value.after
      ? `已是最新：${value.after}`
      : `升级完成：${value.before} -> ${value.after}`;
  }
  if (value.after) return `安装完成：${value.after}`;
  return "安装脚本已执行，但未能确认版本";
}

/** 重启对端的 kimi web。有活跃会话时对方会拒绝，force 可强制。 */
export async function meshKimiWebRestart(host: string, force: boolean): Promise<string> {
  const response = await meshPost(
    host,
    "/kimi-web-restart",
    {},
    JSON.stringify({ force }),
    RESTART_TIMEOUT_MS,
  );
  if (response.status !== 200) {
    throw new Error(response.body.trim() || `对方返回 HTTP ${response.status}`);
  }
  return response.body;
}