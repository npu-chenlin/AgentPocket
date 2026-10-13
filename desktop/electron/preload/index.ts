import { contextBridge, ipcRenderer } from "electron";

import type { AppView } from "../main/commands";
import type { ImportMode, ImportPreviewData } from "../main/config-store";
import type { Backend, DesktopSettings, ServerConfig, ServerForEdit } from "../main/model";
import type { KimiSyncResult, MeshPeerView } from "../main/mesh";
import { MENU_ACTION_CHANNEL, type MenuAction } from "../main/menu";
import type { SyncInfo } from "../main/sync";

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
  saveServer: (server: ServerConfig): Promise<void> => ipcRenderer.invoke("server:save", server),
  deleteServer: (id: string): Promise<void> => ipcRenderer.invoke("server:delete", id),
  probeBackend: (server: ServerConfig): Promise<Backend> =>
    ipcRenderer.invoke("server:probe", server),
  reconnectAll: (): Promise<void> => ipcRenderer.invoke("server:reconnect-all"),

  openServer: (id: string, sessionId: string | null): Promise<void> =>
    ipcRenderer.invoke("agent:open", id, sessionId),
  setWebviewBounds: (rect: { x: number; y: number; width: number; height: number }): Promise<void> =>
    ipcRenderer.invoke("agent:set-bounds", rect),
  setWebviewOverlaid: (overlaid: boolean): Promise<void> =>
    ipcRenderer.invoke("agent:set-overlaid", overlaid),

  setAlwaysOnTop: (pinned: boolean): Promise<void> =>
    ipcRenderer.invoke("window:set-always-on-top", pinned),
  toggleSessionPin: (serverId: string, sessionId: string): Promise<boolean> =>
    ipcRenderer.invoke("pin:toggle", serverId, sessionId),

  updateSettings: (settings: DesktopSettings): Promise<void> =>
    ipcRenderer.invoke("settings:update", settings),
  previewImportText: (content: string): Promise<ImportPreviewData> =>
    ipcRenderer.invoke("import:preview-text", content),
  applyImport: (data: ImportPreviewData, mode: ImportMode): Promise<void> =>
    ipcRenderer.invoke("import:apply", data, mode),
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
};

export type AgentPocketApi = typeof api;

contextBridge.exposeInMainWorld("agentpocket", api);