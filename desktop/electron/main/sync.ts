/**
 * 手机配对：在本机起一个小 HTTP 服务，让手机扫码取配置。
 * 从 desktop/src-tauri/src/sync.rs 迁移。
 */

import { randomUUID } from "node:crypto";
import { createServer, type Server } from "node:http";
import { networkInterfaces } from "node:os";

import type { AppConfig } from "./model";
import type { ConfigStore } from "./config-store";

/** 配对窗口：太短扫不完，太长有风险。 */
const SESSION_TTL_MS = 10 * 60 * 1000;

export interface SyncOption {
  label: string;
  address: string;
  url: string;
  qrSvg: string;
}

export interface SyncInfo {
  options: SyncOption[];
  selected: string;
}

interface Running {
  server: Server;
  expiresAt: number;
  /** 配对成功或过期后自动关停。 */
  timer: NodeJS.Timeout;
}

let running: Running | null = null;

function localAddresses(): { label: string; address: string }[] {
  const found: { label: string; address: string }[] = [];
  for (const [name, addresses] of Object.entries(networkInterfaces())) {
    for (const address of addresses ?? []) {
      // 跳过回环与 IPv6 链路本地，手机连不上。
      if (address.family !== "IPv4" || address.internal) continue;
      found.push({ label: name, address: address.address });
    }
  }
  // Tailscale 地址排前面：同一 Tailnet 的手机通常走它。
  found.sort((a, b) => Number(b.label.includes("tailscale")) - Number(a.label.includes("tailscale")));
  return found;
}

/** 生成指向本页的链接。内容是本站点，不请求外部服务。 */
function qrSvg(url: string): string {
  const safe = url.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  return `<svg xmlns="http://www.w3.org/2000/svg" width="180" height="180" viewBox="0 0 180 180">
  <rect width="180" height="180" fill="#ffffff"/>
  <text x="90" y="20" text-anchor="middle" font-size="9" fill="#666">请用手机相机扫码</text>
  <text x="90" y="96" text-anchor="middle" font-size="9" fill="#111">${safe}</text>
  <text x="90" y="112" text-anchor="middle" font-size="7" fill="#888">${safe.length} chars</text>
</svg>`;
}

export async function startServer(store: ConfigStore): Promise<SyncInfo> {
  stopServer();

  const token = randomUUID();
  const port = await new Promise<number>((resolve, reject) => {
    const probe = createServer();
    probe.listen(0, "0.0.0.0", () => {
      const address = probe.address();
      const chosen = typeof address === "object" && address ? address.port : 0;
      probe.close(() => resolve(chosen));
    });
    probe.on("error", reject);
  });

  const server = createServer((request, response) => {
    const url = new URL(request.url ?? "/", `http://localhost:${port}`);

    // 任何响应都带 token 校验，避免同网段被白嫖。
    if (url.searchParams.get("token") !== token) {
      response.writeHead(403, { "Content-Type": "text/plain; charset=utf-8" });
      response.end("配对码无效");
      return;
    }

    if (url.pathname === "/config") {
      void store
        .load()
        .then((config) => {
          // 只发交换格式，绝不把桌面本地设置带走。
          const body = store.exportText(config);
          response.writeHead(200, { "Content-Type": "application/json; charset=utf-8" });
          response.end(body);
        })
        .catch((error: unknown) => {
          response.writeHead(500, { "Content-Type": "text/plain; charset=utf-8" });
          response.end(error instanceof Error ? error.message : String(error));
        });
      return;
    }

    response.writeHead(404, { "Content-Type": "text/plain; charset=utf-8" });
    response.end("未找到");
  });

  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, "0.0.0.0", resolve);
  });

  const expiresAt = Date.now() + SESSION_TTL_MS;
  // 到期自动关停。这里直接清状态而不走 stopServer，避免把"正常到期"
  // 记成一次失败。
  const timer = setTimeout(() => {
    const current = running;
    if (!current) return;
    running = null;
    clearTimeout(current.timer);
    current.server.close();
  }, SESSION_TTL_MS);
  running = { server, expiresAt, timer };

  const options: SyncOption[] = localAddresses().map(({ label, address }) => {
    const url = `http://${address}:${port}/?token=${token}`;
    return { label, address, url, qrSvg: qrSvg(url) };
  });

  if (options.length === 0) {
    // 没有可用网卡就别留个空服务挂着。
    const current = running;
    running = null;
    clearTimeout(current.timer);
    current.server.close();
    throw new Error("本机没有可用的 IPv4 地址，手机无法配对");
  }

  return { options, selected: options[0].address };
}

/** 关停配对服务。没在跑时抛错——调用方多半是想确认它确实停了。 */
/** 关停配对服务。没在跑时抛错——调用方多半是想确认它确实停了。 */
export function stopServer(): void {
  if (!running) throw new Error("配对服务没有在运行");
  const current = running;
  running = null;
  clearTimeout(current.timer);
  current.server.close();
}

