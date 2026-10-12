/**
 * 内嵌 Agent 视图：用 WebContentsView 把 Agent 页面放进主窗口内容区。
 *
 * 这是当初选 Electron 的原因——Tauri 在 Linux 上无法给子 webview 定位
 * （tauri-apps/tauri#10420），Electron 的 `WebContentsView.setBounds` 是官方支持的。
 *
 * 每个服务一个 view，切换时只改可见性不重建，页面状态因此得以保留。
 */

import { BrowserWindow, WebContentsView, shell } from "electron";

import { installAuth, sessionForServer } from "./auth";
import type { ServerConfig } from "./model";
import { buildServerUrl } from "./server-url";

export interface AgentBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

interface Entry {
  view: WebContentsView;
  serverId: string;
}

const views = new Map<string, Entry>();
let activeId: string | null = null;
let bounds: AgentBounds = { x: 0, y: 0, width: 1, height: 1 };
/** 弹窗打开时收起视图：它是原生控件，层级在 DOM 之上。 */
let overlaid = false;

function urlFor(server: ServerConfig, sessionId: string | null): string {
  return buildServerUrl(server, sessionId).toString();
}

/**
 * 打开某个服务/会话。已存在就只导航并聚焦，避免重建丢掉页面状态。
 */
export async function openServer(
  window: BrowserWindow,
  server: ServerConfig,
  sessionId: string | null,
): Promise<void> {
  const url = urlFor(server, sessionId);

  const existing = views.get(server.id);
  if (existing) {
    await existing.view.webContents.loadURL(url);
    activeId = server.id;
    refreshVisibility();
    return;
  }

  // 每个服务用自己的 session，cookie 不与其他服务串台，
  // 凭据也只注到这个 session 里（见 auth.ts）。
  const viewSession = sessionForServer(server.id);
  installAuth(viewSession, server);

  const view = new WebContentsView({
    webPreferences: {
      session: viewSession,
      // Agent 页面是外部站点，不能拿到我们的 preload。
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
    },
  });

  // 只允许在当前 origin 内跳转，站外交给系统浏览器。
  const origin = new URL(url).origin;
  view.webContents.setWindowOpenHandler(({ url: target }) => {
    if (new URL(target).origin !== origin) openExternal(target);
    // 一律拒绝开新窗口：新窗口拿不到会话上下文。
    return { action: "deny" };
  });
  view.webContents.on("will-navigate", (event, target) => {
    if (new URL(target).origin !== origin) event.preventDefault();
  });

  await view.webContents.loadURL(url);

  views.set(server.id, { view, serverId: server.id });
  window.contentView.addChildView(view);
  applyBounds();
  activeId = server.id;
  refreshVisibility();
}

/** 前端上报内容区矩形。CSS 决定布局，原生只跟随。 */
export function setBounds(next: AgentBounds): void {
  bounds = next;
  applyBounds();
}

function applyBounds(): void {
  for (const { view } of views.values()) {
    view.setBounds(bounds);
  }
}

export function setOverlaid(next: boolean): void {
  overlaid = next;
  refreshVisibility();
}

function refreshVisibility(): void {
  for (const [id, entry] of views) {
    entry.view.setVisible(id === activeId && !overlaid);
  }
}

/** 删除服务时关掉它的视图。凭据变了也走这里，否则会一直用旧 cookie。 */
export function closeServer(window: BrowserWindow, serverId: string): void {
  const entry = views.get(serverId);
  if (!entry) return;
  window.contentView.removeChildView(entry.view);
  entry.view.webContents.close();
  views.delete(serverId);
  if (activeId === serverId) activeId = null;
  refreshVisibility();
}

/** 销毁全部视图：进程退出前调用。 */
export function closeAll(window: BrowserWindow): void {
  for (const id of [...views.keys()]) closeServer(window, id);
}

/**
 * 打开外部链接到系统浏览器。
 *
 * Electron 出于安全不会自动跳转外部站点，必须显式用 shell 打开。这里只放行
 * http/https，免得页面被诱导去执行本地协议。
 */
export function openExternal(target: string): void {
  let url: URL;
  try {
    url = new URL(target);
  } catch {
    throw new Error(`不是合法 URL：${target}`);
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw new Error(`拒绝打开非 http(s) 链接：${url.protocol}`);
  }
  void shell.openExternal(url.toString());
}
