/**
 * Agent 页面的凭据注入。
 *
 * 旧版（Tauri）靠一个 522 行的本机回环 HTTP 代理，在转发时补上 Authorization
 * 头。换成 Electron 后不需要代理：Chromium 的 webRequest 管线能直接改写请求头，
 * 导航、XHR、fetch 一视同仁。
 *
 * 为什么不用 app.on('login')：那只在主框架导航收到 401 挑战时触发。而 OpenCode
 * v2 的页面本身是静态的（能正常加载），真正 401 的是它随后发起的 /api/* 请求。
 * XHR/fetch 的 401 不会走到 login 事件，所以只注册 login 的话症状恰好是
 * "页面打开了但一直卡在 Connect to a server"。
 */

import { session as electronSession, type Session } from "electron";

import { opencodeBasicHeader, serverBaseUrl, type ServerConfig } from "./model";

/**
 * 每个服务一个独立的 session。
 *
 * 不能共用默认 session：cookie 会互相串台——登录了 A 服务器的 Kimi，
 * 打开 B 服务器时会被当成已登录。partition 名带上服务 id 做隔离，
 * 加 persist 前缀让登录状态跨重启保留，免得每次启动都要重新登录。
 */
export function sessionForServer(serverId: string): Session {
  return electronSession.fromPartition(`persist:server-${serverId}`);
}

/**
 * 这个 origin 的请求要不要注入、注入什么。
 *
 * 拆成纯函数是为了能测：真正注册 webRequest handler 需要 Electron 运行时，
 * 而这里的判断规则（origin 匹配、不覆盖已有凭据）正是容易改坏的地方。
 */
export function shouldInject(
  url: string,
  origin: string,
  header: string | null,
  existing: string | undefined,
): string | undefined {
  // 前缀比较要求结尾的 "/"，否则 http://host:40961 会匹配上 4096 的 origin。
  if (!url.startsWith(`${origin}/`)) return undefined;
  // 服务端已经给了凭据就别覆盖，否则用户改密码后会被这里的旧值钉住。
  if (existing !== undefined) return undefined;
  return header ?? undefined;
}

/**
 * 给这个 session 装上凭据注入。
 *
 * 只对配置里那个 origin 生效，别的一概不碰——否则同一个 session 里任何意外的
 * 跨域请求都会带上凭据出去。
 */
export function installAuth(session: Session, server: ServerConfig): void {
  const header = opencodeBasicHeader(server);
  if (server.backend !== "opencode" || header === null) {
    // Kimi 的令牌走 URL 片段由前端消费，dsh 没有令牌，两者都不需要注入。
    return;
  }
  const origin = serverBaseUrl(server).origin;

  session.webRequest.onBeforeSendHeaders({ urls: [`${origin}/*`] }, (details, callback) => {
    const existing = details.requestHeaders.Authorization ?? details.requestHeaders.authorization;
    const value = shouldInject(details.url, origin, header, existing);
    callback({
      requestHeaders:
        value === undefined
          ? details.requestHeaders
          : { ...details.requestHeaders, Authorization: value },
    });
  });
}