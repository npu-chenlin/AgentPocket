/**
 * dsh（DeepSeek Harness）Web 认证。
 *
 * dsh 0.2 起页面与 `/api` 都要浏览器会话 cookie，没带一律 401。签发 cookie 的是
 * dsh 数据目录里那份**持久签名密钥**（`~/.dsh/.credentials.yaml`），客户端拿不到，
 * 也不该拿——复刻它的 HMAC 等于把 dsh 的内部签名格式抄一份到自己家里。
 *
 * dsh 自己提供了获取 cookie 的正规入口：启动行里的 launch token。
 *
 *     GET http://host:port/?token=<launch token>   →   303 + Set-Cookie: dsh-auth-…=v1.…
 *
 * 两者的寿命不同，这是整个设计的关键：
 *   - launch token 每次启动都重新生成，只在**当前进程**有效；
 *   - 换来的 cookie 由持久密钥签发，30 天有效，**dsh 重启后依然有效**（实测）。
 *
 * 所以客户端只需要把 cookie 存下来、当普通 cookie 用：不需要签名密钥，不需要复刻
 * HMAC，也就没有"密钥填错 / 密钥被清空 / 密钥丢了"这一整类失败模式。cookie 过期
 * 或服务地址变了（cookie 名绑定 authority）时，重新粘贴一次启动行即可。
 */

import { serverBaseUrl, type ServerConfig } from "./model";

/** cookie 有效期（天）。与 dsh 自身 `cookieMaxAgeDays` 的默认值一致。 */
export const COOKIE_MAX_AGE_DAYS = 30;

export interface DshCookie {
  /** cookie 名，形如 `dsh-auth-<base64url(sha256(authority))>`。 */
  readonly name: string;
  /** cookie 值，形如 `v1.<body>.<signature>`。 */
  readonly value: string;
  /** `name=value` 原样串，直接放进 Cookie 头。 */
  readonly header: string;
}

/**
 * 解析配置里存的 dsh cookie（`name=value` 或裸 `v1.…` 值）。
 *
 * 名字从 cookie 自己带的那半取，不再本地推导：dsh 已经把绑定 authority 的名字算好
 * 写进 Set-Cookie 了，客户端照抄即可。地址变了名字就不匹配，服务端会拒绝——这正是
 * 我们想要的行为（换地址本来就该重新取一次）。
 */
export function parseDshCookie(stored: string | null | undefined): DshCookie | null {
  const raw = (stored ?? "").trim();
  if (raw.length === 0) return null;

  // 容忍整条 Set-Cookie 被存进来：属性（Path/HttpOnly/…）在第一个分号之后，丢掉。
  const pair = raw.split(";")[0]?.trim() ?? "";
  // 裸值（只存了 v1.…）：没有名字，无法当 Cookie 头发出去。
  const separator = pair.indexOf("=");
  if (separator <= 0) return null;

  const name = pair.slice(0, separator).trim();
  const value = pair.slice(separator + 1).trim();
  // 名字必须是 dsh 自己那种；值必须是三段式。挡在本地，早于发请求。
  if (!/^dsh-auth-[A-Za-z0-9_-]+$/.test(name)) return null;
  if (!/^v1\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$/.test(value)) return null;
  return { name, value, header: `${name}=${value}` };
}

/** 这个服务当前可用的 cookie 头；配置里没有（或不像）cookie 时返回 null。 */
export function dshCookie(server: ServerConfig): DshCookie | null {
  if (server.backend !== "dsh") return null;
  return parseDshCookie(server.token);
}

/** 取 Cookie 头值；拿不到时返回空对象，让请求照常发出并如实报 401。 */
export function dshCookieHeaders(server: ServerConfig): Record<string, string> {
  const cookie = dshCookie(server);
  return cookie === null ? {} : { Cookie: cookie.header };
}

/** 从粘贴的启动行 / URL 里取出 launch token（`?token=…`）。 */
export function extractLaunchToken(text: string | null | undefined): string | null {
  const raw = (text ?? "").trim();
  if (raw.length === 0) return null;
  const match = raw.match(/[?&]token=([A-Za-z0-9_-]+)/);
  return match === null ? null : match[1];
}

/** 从响应头里取 Set-Cookie 的 `name=value`（丢掉属性）。 */
function firstCookiePair(response: Response): string | null {
  const values =
    typeof response.headers.getSetCookie === "function"
      ? response.headers.getSetCookie()
      : [response.headers.get("set-cookie") ?? ""];
  for (const value of values) {
    const pair = value.split(";")[0]?.trim() ?? "";
    if (pair.length > 0) return pair;
  }
  return null;
}

/**
 * 用 launch token 换一个会话 cookie。
 *
 * 走 `redirect: "manual"`：dsh 用 303 + Set-Cookie 完成这次交换，跟着跳转没有意义。
 * 换不到（token 过期、地址不对、后面根本不是 dsh）就返回 null，由调用方决定怎么提示。
 */
export async function exchangeLaunchToken(
  server: ServerConfig,
  launchToken: string,
  timeoutMs = 5000,
): Promise<DshCookie | null> {
  const url = new URL("/", serverBaseUrl(server));
  url.searchParams.set("token", launchToken);
  try {
    const response = await fetch(url, {
      redirect: "manual",
      signal: AbortSignal.timeout(timeoutMs),
    });
    const pair = firstCookiePair(response);
    return pair === null ? null : parseDshCookie(pair);
  } catch {
    return null;
  }
}

/**
 * 把用户粘进来的东西换成可存的 cookie。
 *
 * 两种输入都收：完整启动行（含 `?token=`，说明这次要现场交换），以及已经拿到的
 * `name=value` cookie（直接校验后返回）。
 */
export async function resolveDshCredential(
  server: ServerConfig,
  pasted: string,
): Promise<DshCookie | null> {
  const existing = parseDshCookie(pasted);
  if (existing !== null) return existing;
  const launchToken = extractLaunchToken(pasted);
  if (launchToken === null) return null;
  return exchangeLaunchToken(server, launchToken);
}
