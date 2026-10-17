/**
 * dsh（DeepSeek Harness）Web 认证。
 *
 * dsh 0.2 起给 Web UI 和 `/api` 都加了浏览器会话 cookie：没带 cookie 一律 401
 * （`dsh web authentication required; reopen the URL printed by dsh web.`）。
 *
 * 真正管用的凭据是 dsh 数据目录里那把**持久**签名密钥
 * （`~/.dsh/.credentials.yaml` 的 `client-connection/browser-session`），
 * 那份文件只在缺失时才生成、从不轮换。每次启动都变的 launch token 只打印在服务
 * 进程日志里，客户端拿不到，因此没有可用性。
 *
 * 所以做法与 Kimi 的访问令牌一致：把密钥填进服务连接的 token 字段，客户端自己
 * 签发 cookie。密钥落在客户端配置里——客户端本来就是可信边界。
 *
 * 与 dsh 的 `BrowserAuth.encodeCookie` 逐字段对齐：
 *   名字  `dsh-auth-<base64url(sha256(authority))>`
 *   值    `v1.<base64url(json)>.<base64url(hmac-sha256(secret, body))>`
 *   载荷  `{version:1,authority,issuedAt,expiresAt}`，毫秒，**键序不能变**——
 *         HMAC 覆盖的是 JSON.stringify 出来的那个字节串。
 *   authority = `host:port`，cookie 按它绑定，所以同一台 dsh 在不同地址下登录不通用。
 */

import { createHash, createHmac } from "node:crypto";

import type { ServerConfig } from "./model";

/** cookie 有效期（天）。与 dsh 自身 `cookieMaxAgeDays` 的默认值一致。 */
const COOKIE_MAX_AGE_DAYS = 30;

export interface DshCookie {
  /** `name=value` 原样串，直接放进 Cookie 头。 */
  readonly header: string;
  readonly expiresAt: number;
}

/** dsh 的 cookie 绑定 `host:port`。 */
export function dshAuthority(server: ServerConfig): string {
  return `${server.host}:${server.port}`;
}

function base64Url(input: Buffer | string): string {
  return Buffer.from(input).toString("base64")
    .replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** 严格解码：dsh 只写 32 字节的 base64url，长度 %4==1 在 base64 里不可能出现。 */
function decodeSecret(secret: string): Buffer | null {
  const value = secret.trim();
  if (value.length === 0 || value.length % 4 === 1) return null;
  if (!/^[A-Za-z0-9_-]+$/.test(value)) return null;
  const bytes = Buffer.from(value.replace(/-/g, "+").replace(/_/g, "/"), "base64");
  // 回编一次确认没有多余字符被静默丢弃。
  return base64Url(bytes) === value ? bytes : null;
}

/** cookie 名：`dsh-auth-<base64url(sha256(authority))>`。 */
export function dshCookieName(authority: string): string {
  return `dsh-auth-${base64Url(createHash("sha256").update(authority).digest())}`;
}

/**
 * 用密钥签发一个会话 cookie。密钥不合法时返回 null（调用方照常发请求，如实报 401）。
 */
export function mintDshCookie(
  secret: string,
  authority: string,
  nowMs: number = Date.now(),
): DshCookie | null {
  const key = decodeSecret(secret);
  if (key === null) return null;

  const expiresAt = nowMs + COOKIE_MAX_AGE_DAYS * 24 * 60 * 60 * 1000;
  // 键序固定：签的是这个字符串本身。
  const payload = JSON.stringify({
    version: 1,
    authority,
    issuedAt: nowMs,
    expiresAt,
  });
  const body = base64Url(payload);
  const signature = base64Url(createHmac("sha256", key).update(body).digest());
  return { header: `${dshCookieName(authority)}=v1.${body}.${signature}`, expiresAt };
}

/**
 * 这个服务当前可用的 cookie。
 *
 * 每次调用重新签（HMAC 很便宜），好处是配置一改立刻生效，不必管缓存失效。
 */
export function dshCookie(server: ServerConfig): DshCookie | null {
  if (server.backend !== "dsh") return null;
  return mintDshCookie(server.token, dshAuthority(server));
}

/** 取 Cookie 头值；拿不到密钥时返回空对象，让请求照常发出并如实报 401。 */
export function dshCookieHeaders(server: ServerConfig): Record<string, string> {
  const cookie = dshCookie(server);
  return cookie === null ? {} : { Cookie: cookie.header };
}
