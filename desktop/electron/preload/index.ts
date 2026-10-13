import { contextBridge, ipcRenderer } from "electron";

import type { AppView } from "../main/commands";
import type { ImportMode, ImportPreview, PinToggleResult } from "../../src/model";
import type { WorkspaceSnapshot, TabStatus } from "../../shared/workspace";
import type { Backend, DesktopSettings, ServerConfig, ServerForEdit } from "../main/model";
import type { KimiSyncResult, MeshPeerView } from "../main/mesh";
import { MENU_ACTION_CHANNEL, type MenuAction } from "../main/menu";
import type { BallState } from "../main/ball-ipc";
import type { SyncInfo } from "../main/sync";

/** 原生视图占据的矩形，坐标系与渲染进程视口一致。 */
export interface WebviewRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/**
 * 渲染进程可见的全部接口。
 *
 * 白名单式暴露：渲染进程拿不到 ipcRenderer 本体，只能调这里列出的命令，
 * Agent 页面那个 WebContentsView 更是什么都拿不到。
 */
const api = {
  getAppView: (): Promise<AppView> => ipcRenderer.invoke("app:get-view"),
  getServerForEdit: (id: string): Promise<ServerForEdit> =>
    ipcRenderer.invoke("server:for-edit", id),
  saveServer: (server: ServerConfig): Promise<AppView> => ipcRenderer.invoke("server:save", server),
  deleteServer: (id: string): Promise<AppView> => ipcRenderer.invoke("server:delete", id),
  probeBackend: (server: ServerConfig): Promise<Backend> => ipcRenderer.invoke("server:probe", server),
  reconnectAll: (): Promise<void> => ipcRenderer.invoke("server:reconnect-all"),
  syncWorkspace: (snapshot: WorkspaceSnapshot): Promise<void> => ipcRenderer.invoke("workspace:sync", snapshot),
  retryTab: (id: string): Promise<void> => ipcRenderer.invoke("workspace:retry", id),
  /** 拖拽标签悬停内容区时，在网页上方显示半透明投放指示；null 表示隐藏。 */
  setDropIndicator: (rect: WebviewRect | null): Promise<void> => ipcRenderer.invoke("workspace:drop-indicator", rect),
  /** 模态遮挡前抓取可见网页的快照，用于虚化背景。 */
  captureViews: (): Promise<Record<string, string>> => ipcRenderer.invoke("workspace:capture"),
  onTabStatus: (callback: (status: TabStatus) => void): (() => void) => {
    const listener = (_event: unknown, status: TabStatus) => callback(status);
    ipcRenderer.on("workspace:status", listener);
    return () => ipcRenderer.removeListener("workspace:status", listener);
  },
  onTabFocus: (callback: (id: string) => void): (() => void) => {
    const listener = (_event: unknown, id: string) => callback(id);
    ipcRenderer.on("workspace:focus", listener);
    return () => ipcRenderer.removeListener("workspace:focus", listener);
  },
  setAlwaysOnTop: (pinned: boolean): Promise<void> => ipcRenderer.invoke("window:set-always-on-top", pinned),
  toggleSessionPin: (serverId: string, sessionId: string): Promise<PinToggleResult> =>
    ipcRenderer.invoke("pin:toggle", serverId, sessionId),
  updateSettings: (settings: DesktopSettings): Promise<AppView> => ipcRenderer.invoke("settings:update", settings),
  previewImportText: (content: string): Promise<ImportPreview> => ipcRenderer.invoke("import:preview-text", content),
  applyImport: (id: string, mode: ImportMode): Promise<AppView> => ipcRenderer.invoke("import:apply", id, mode),
  exportConfigText: (): Promise<string> => ipcRenderer.invoke("export:text"),
  chooseExportPath: (): Promise<string | null> => ipcRenderer.invoke("dialog:choose-export-path"),
  exportConfig: (path: string): Promise<void> => ipcRenderer.invoke("export:to", path),
  openExternal: (url: string): Promise<void> => ipcRenderer.invoke("shell:open-external", url),

  discoverMeshPeers: (): Promise<MeshPeerView[]> => ipcRenderer.invoke("mesh:discover"),
  meshPull: (host: string): Promise<KimiSyncResult> => ipcRenderer.invoke("mesh:pull", host),
  meshPush: (host: string): Promise<KimiSyncResult> => ipcRenderer.invoke("mesh:push", host),
  meshKimiUpgrade: (host: string): Promise<string> => ipcRenderer.invoke("mesh:upgrade", host),
  meshKimiWebRestart: (host: string, force: boolean): Promise<string> =>
    ipcRenderer.invoke("mesh:restart-web", host, force),

  startSyncServer: (): Promise<SyncInfo> => ipcRenderer.invoke("sync:start"),
  stopSyncServer: (): Promise<void> => ipcRenderer.invoke("sync:stop"),

  onStateChanged: (callback: (view: AppView) => void): (() => void) => {
    const handler = (_event: unknown, view: AppView): void => callback(view);
    ipcRenderer.on("app:state", handler);
    return () => ipcRenderer.removeListener("app:state", handler);
  },
  onAgentEvent: (callback: (event: unknown) => void): (() => void) => {
    const handler = (_event: unknown, payload: unknown): void => callback(payload);
    ipcRenderer.on("app:event", handler);
    return () => ipcRenderer.removeListener("app:event", handler);
  },

  /**
   * 订阅原生菜单动作。菜单项本身在主进程，这里只收动作名并交给渲染进程执行，
   * 避免同一份业务逻辑在两边各写一遍。
   */
  onMenuAction: (callback: (action: MenuAction) => void): (() => void) => {
    const handler = (_event: unknown, action: MenuAction): void => callback(action);
    ipcRenderer.on(MENU_ACTION_CHANNEL, handler);
    return () => ipcRenderer.removeListener(MENU_ACTION_CHANNEL, handler);
  },

  /** 悬浮球窗口点了一下，请求开合面板。 */
  onBallToggle: (callback: () => void): (() => void) => {
    const handler = (): void => callback();
    ipcRenderer.on("ball:toggle-request", handler);
    return () => ipcRenderer.removeListener("ball:toggle-request", handler);
  },

  /** 把表情状态推给悬浮球窗口；它看不到服务状态，只能由主窗口算好推过去。 */
  publishBallState: (state: BallState): Promise<void> =>
    ipcRenderer.invoke("ball:publish-state", state),
};

export type AgentPocketApi = typeof api;

contextBridge.exposeInMainWorld("agentpocket", api);