/**
 * 已保存服务器的 URL 构造。
 *
 * 直接对应 core/src/server_url.rs，包括它对 token 百分号编码和 opencode
 * 会话路由的处理——这两处必须和 daemon / Android 端保持一致，改动要同步。
 */

import { serverBaseUrl, type ServerConfig } from "./model";

/**
 * RFC 3986 unreserved 之外的字符全部百分号编码。
 *
 * 不能用 `encodeURIComponent`：它不编码 `! * ' ( )`，与 Rust 侧
 * `NON_ALPHANUMERIC` 去掉 `- . _ ~` 的行为不一致。
 */
function percentEncodeToken(value: string): string {
  return value.replace(/[^A-Za-z0-9\-._~]/g, (character) => {
    const bytes = Buffer.from(character, "utf8");
    let out = "";
    for (const byte of bytes) {
      out += `%${byte.toString(16).toUpperCase().padStart(2, "0")}`;
    }
    return out;
  });
}

/** URL-safe base64（无填充），与 opencode web 前端 `cn()` 的编码一致。 */
export function encodeBase64Url(input: string): string {
  return Buffer.from(input, "utf8")
    .toString("base64")
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
}

/**
 * opencode v2 前端用于标识服务器的 key：`base64url(服务器根 URL)`。
 *
 * 编码的是整个 origin（含 scheme/host/port，无尾部斜杠）。
 */
export function opencodeServerKey(base: URL): string {
  const origin = `${base.protocol}//${base.hostname}:${base.port || "80"}`;
  return encodeBase64Url(origin);
}

/**
 * 构造指向已保存服务器的地址。
 *
 * Kimi 的访问令牌放在 `#token=` 片段里（前端消费，不走网络）；dsh 没有令牌；
 * OpenCode 的会话路由是 `/server/{base64url(origin)}/session/{id}`。
 */
export function buildServerUrl(server: ServerConfig, sessionId?: string | null): URL {
  const url = serverBaseUrl(server);
  const session = sessionId && sessionId.length > 0 ? sessionId : null;

  switch (server.backend) {
    case "dsh": {
      // DeepSeek Harness 没有稳定的会话深链，直接打开根路径。
      url.pathname = "/";
      break;
    }
    case "kimi": {
      url.pathname = session ? `/sessions/${session}` : "/";
      if (server.token) {
        url.hash = `#token=${percentEncodeToken(server.token)}`;
      }
      break;
    }
    case "opencode": {
      if (session) {
        const key = opencodeServerKey(url);
        url.pathname = `/server/${key}/session/${session}`;
      } else {
        url.pathname = "/";
      }
      break;
    }
  }

  return url;
}