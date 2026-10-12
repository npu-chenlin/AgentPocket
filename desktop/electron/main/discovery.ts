/**
 * 节点发现。从 core/src/discovery.rs 迁移。
 *
 * 候选来源两个：Tailscale 在线设备（`tailscale status --json`）和手写的
 * peers.json。两条都只是"候选"，还要逐个探测 MESH_PORT 的 /info 端点，
 * 应答里 app 必须是 agentpocket 才算命中——否则同一网段随便一个开着的
 * 端口都会被误认成节点。
 */

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";

export const MESH_PORT = 48720;

const PROBE_TIMEOUT_MS = 3_000;

export interface MeshPeer {
  name: string;
  host: string;
  version: string | null;
  manual: boolean;
}

interface PeerFile {
  peers: { name: string; host: string }[];
}

/** CLI 查找顺序：PATH → 常见安装路径。找不到就退化为"只有手动 peer"。 */
export function findTailscaleBinary(): string | null {
  try {
    execFileSync("tailscale", ["version"], { stdio: "ignore" });
    return "tailscale";
  } catch {
    // PATH 里没有，继续试固定安装位置。
  }
  for (const path of ["/usr/bin/tailscale", "/usr/local/bin/tailscale"]) {
    if (existsSync(path)) return path;
  }
  return null;
}

/**
 * 解析 tailscale status --json：只留在线且带 IPv4 的 peer。
 *
 * 字段名是 tailscale 自己定的（Peer / HostName / TailscaleIPs / Online），
 * 不要按 JS 习惯改名——对不上就会一个节点都发现不了。顶层 "Self" 不是
 * map 的值，自然被排除。
 */
export function parseOnlinePeers(json: string): { name: string; ip: string }[] {
  let parsed: { Peer?: Record<string, unknown> };
  try {
    parsed = JSON.parse(json) as { Peer?: Record<string, unknown> };
  } catch {
    // tailscale 换了输出格式时不该让整个发现崩掉，当作没发现即可。
    return [];
  }
  const peers = parsed.Peer;
  if (!peers || typeof peers !== "object") return [];

  const result: { name: string; ip: string }[] = [];
  for (const entry of Object.values(peers)) {
    const record = entry as {
      HostName?: string;
      TailscaleIPs?: unknown;
      Online?: boolean;
    };
    if (record.Online !== true) continue;
    if (typeof record.HostName !== "string") continue;
    if (!Array.isArray(record.TailscaleIPs)) continue;
    // 优先 IPv4：mesh 端口与大多数内网服务都走 IPv4。
    const ipv4 = record.TailscaleIPs.find(
      (ip) => typeof ip === "string" && /^\d{1,3}(\.\d{1,3}){3}$/.test(ip),
    );
    if (typeof ipv4 !== "string") continue;
    result.push({ name: record.HostName, ip: ipv4 });
  }
  result.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
  return result;
}

/** 探测单个 host；/info 应答 app==agentpocket 才算命中。 */
export async function probePeer(
  host: string,
  fallbackName: string,
  timeoutMs = PROBE_TIMEOUT_MS,
): Promise<MeshPeer | null> {
  let body: string;
  try {
    const response = await fetch(`http://${host}:${MESH_PORT}/info`, {
      signal: AbortSignal.timeout(timeoutMs),
    });
    if (!response.ok) return null;
    body = await response.text();
  } catch {
    // 离线节点本来就该被安静跳过，这不是错误。
    return null;
  }

  let value: { app?: string; name?: string; version?: string };
  try {
    value = JSON.parse(body) as typeof value;
  } catch {
    return null;
  }
  if (value.app !== "agentpocket") return null;

  return {
    name: value.name ?? fallbackName,
    host,
    version: value.version ?? null,
    manual: false,
  };
}

/** Tailscale 在线设备候选，返回 (ip, 备用名)。 */
export function tailscaleCandidates(binary: string | null): [string, string][] {
  if (!binary) return [];
  let output: string;
  try {
    output = execFileSync(binary, ["status", "--json"], {
      encoding: "utf8",
      // tailscale 可能往 stderr 写提示噪音，只要 stdout。
      stdio: ["ignore", "pipe", "ignore"],
    });
  } catch {
    return [];
  }
  return parseOnlinePeers(output).map(({ name, ip }) => [ip, name]);
}

export function peersFilePath(configDir: string): string {
  return join(configDir, "peers.json");
}

export function loadManualPeers(configDir: string): MeshPeer[] {
  const path = peersFilePath(configDir);
  if (!existsSync(path)) return [];
  let file: PeerFile;
  try {
    file = JSON.parse(readFileSync(path, "utf8")) as PeerFile;
  } catch {
    // peers.json 是手写维护的，语法错不该让整个发现失败。
    return [];
  }
  if (!Array.isArray(file.peers)) return [];
  return file.peers
    .filter((peer) => typeof peer.host === "string" && peer.host.length > 0)
    .map((peer) => ({
      name: peer.name ?? "",
      host: peer.host,
      version: null,
      manual: true,
    }));
}

/** 并发探测候选，按 host 去重、按 name 排序。 */
export async function probeCandidates(
  candidates: [string, string][],
  timeoutMs = PROBE_TIMEOUT_MS,
): Promise<MeshPeer[]> {
  const probed = await Promise.all(
    candidates.map(([host, name]) => probePeer(host, name, timeoutMs)),
  );
  const seen = new Set<string>();
  const peers: MeshPeer[] = [];
  for (const peer of probed) {
    if (peer && !seen.has(peer.host)) {
      seen.add(peer.host);
      peers.push(peer);
    }
  }
  peers.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
  return peers;
}

/** 完整发现：Tailscale 在线设备 + 手动 peer。 */
export async function discover(configDir: string): Promise<MeshPeer[]> {
  const candidates = tailscaleCandidates(findTailscaleBinary());
  for (const peer of loadManualPeers(configDir)) {
    candidates.push([peer.host, peer.name]);
  }
  return probeCandidates(candidates);
}