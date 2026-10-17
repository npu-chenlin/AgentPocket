import { BrowserWindow, WebContentsView, shell } from "electron";
import { prepareAuth, sessionForServer } from "./auth";
import type { ServerConfig } from "./model";
import { buildServerUrl } from "./server-url";
import { clipBounds, validateSnapshot, type WorkspaceSnapshot, type WorkspaceTab, type TabStatus } from "../../shared/workspace";

interface Entry {
  view: WebContentsView;
  tab: WorkspaceTab;
  fingerprint: string;
  state: TabStatus["state"];
  url: string;
}
const views = new Map<string, Entry>();
let latest: WorkspaceSnapshot | null = null;
let generation = 0;

function publish(window: BrowserWindow, entry: Entry, state: TabStatus["state"], error?: string): void {
  if (views.get(entry.tab.id) !== entry || window.isDestroyed()) return;
  entry.state = state;
  window.webContents.send("workspace:status", { tabId: entry.tab.id, state, error } satisfies TabStatus);
  place(window);
}

function place(window: BrowserWindow): void {
  const [width, height] = window.getContentSize();
  for (const entry of views.values()) {
    const rect = latest?.visible[entry.tab.id];
    const bounds = rect && clipBounds(rect, width, height, window.webContents.getZoomFactor());
    if (bounds) entry.view.setBounds(bounds);
    entry.view.setVisible(Boolean(bounds && bounds.width > 0 && bounds.height > 0
      && !latest?.overlays.length && entry.state === "ready"));
  }
}

function closeTab(window: BrowserWindow, id: string): void {
  const entry = views.get(id);
  if (!entry) return;
  views.delete(id);
  window.contentView.removeChildView(entry.view);
  entry.view.webContents.close();
}

/**
 * 加载一个窗格。先备好凭据再导航——dsh 要等 daemon 代签的 cookie 种进罐里，
 * 否则第一帧是 401 页面，而 dsh 不会自己跳回来。
 */
async function load(window: BrowserWindow, entry: Entry, server: ServerConfig): Promise<void> {
  publish(window, entry, "loading");
  try {
    await prepareAuth(sessionForServer(server.id), server);
  } catch (error) {
    publish(window, entry, "error", error instanceof Error ? error.message : String(error));
    return;
  }
  if (views.get(entry.tab.id) !== entry || window.isDestroyed()) return;
  void entry.view.webContents.loadURL(entry.url).then(
    () => publish(window, entry, "ready"),
    (error: Error) => publish(window, entry, "error", error.message),
  );
}

export async function syncWorkspace(window: BrowserWindow, snapshot: WorkspaceSnapshot,
  getServers: () => Promise<ServerConfig[]>): Promise<void> {
  validateSnapshot(snapshot);
  if (latest && snapshot.revision <= latest.revision) return;
  latest = snapshot;
  const epoch = ++generation;
  for (const id of views.keys()) if (!snapshot.tabs.some(t => t.id === id)) closeTab(window, id);
  place(window);
  const servers = await getServers();
  if (epoch !== generation || window.isDestroyed()) return;
  reconcile(window, servers);
}

function reconcile(window: BrowserWindow, servers: ServerConfig[]): void {
  for (const tab of latest?.tabs ?? []) {
    const server = servers.find(s => s.id === tab.serverId);
    if (!server) { closeTab(window, tab.id); continue; }
    const fingerprint = JSON.stringify(server);
    const previous = views.get(tab.id);
    if (previous && previous.fingerprint === fingerprint
      && previous.tab.sessionId === tab.sessionId && previous.tab.serverId === tab.serverId) continue;
    closeTab(window, tab.id);
    const session = sessionForServer(server.id);
    const view = new WebContentsView({ webPreferences: {
      session, contextIsolation: true, nodeIntegration: false, sandbox: true,
    } });
    view.setVisible(false);
    const url = buildServerUrl(server, tab.sessionId).toString();
    const origin = new URL(url).origin;
    view.webContents.setWindowOpenHandler(({ url: target }) => {
      if (/^https?:/.test(target)) openExternal(target);
      return { action: "deny" };
    });
    view.webContents.on("will-navigate", (event, target) => {
      if (new URL(target).origin !== origin) event.preventDefault();
    });
    const entry: Entry = { view, tab, fingerprint, state: "loading", url };
    views.set(tab.id, entry);
    window.contentView.addChildView(view);
    view.webContents.on("focus", () => {
      if (views.get(tab.id) === entry) window.webContents.send("workspace:focus", tab.id);
    });
    view.webContents.on("render-process-gone", (_event, details) => publish(window, entry, "error", details.reason));
    view.webContents.on("did-start-loading", () => publish(window, entry, "loading"));
    view.webContents.on("did-stop-loading", () => {
      if (entry.state !== "error") publish(window, entry, "ready");
    });
    view.webContents.on("did-fail-load", (_event, code, description, _url, mainFrame) => {
      if (mainFrame && code !== -3) publish(window, entry, "error", description);
    });
    void load(window, entry, server);
  }
  place(window);
}

export function refreshServers(window: BrowserWindow, servers: ServerConfig[]): void {
  ++generation;
  reconcile(window, servers);
}
export function retryTab(window: BrowserWindow, id: string, servers: ServerConfig[]): void {
  const entry = views.get(id);
  const server = servers.find(s => s.id === entry?.tab.serverId);
  if (entry && server) void load(window, entry, server);
}

export function closeServer(window: BrowserWindow, serverId: string): void {
  for (const [id, entry] of views) if (entry.tab.serverId === serverId) closeTab(window, id);
}
export function closeAll(window: BrowserWindow): void {
  ++generation;
  latest = null;
  for (const id of views.keys()) closeTab(window, id);
}
export function openExternal(target: string): void {
  const url = new URL(target);
  if (url.protocol !== "http:" && url.protocol !== "https:") throw new Error("拒绝打开非 http(s) 链接");
  void shell.openExternal(url.toString());
}
