/**
 * IPC 命令表。对应 Rust 侧 src-tauri/src/commands.rs。
 *
 * 渲染进程只能通过 preload 暴露的 invoke 调到这里；所有错误都原样抛回，
 * 不做静默兜底。
 */

import { BrowserWindow, app, dialog, ipcMain, shell } from "electron";
import { randomUUID } from "node:crypto";
import type { WorkspaceSnapshot } from "../../shared/workspace";
import type { PinToggleResult } from "../../src/model";
import { isolatedMode } from "./runtime";
import * as agentView from "./agent-view";
import { ballWindowRef, dragBall, ballSnapshot, publishBallState } from "./ball-window";
import { setDropIndicator, type DropIndicatorRect } from "./drop-overlay";
import type { BallDrag, BallState, BallSnapshot } from "./ball-ipc";
import { ConfigStore, type ImportMode, type ImportPreviewData } from "./config-store";
import {
  toForEdit,
  toSummary,
  validateServer,
  type AppConfig,
  type Backend,
  type DesktopSettings,
  type ServerConfig,
  type ServerForEdit,
  type ServerSummary,
} from "./model";
import { MonitorManager, PinnedSessions, type ServerStatusView } from "./monitor/manager";
import {
  discoverPeers,
  meshKimiUpgrade,
  meshKimiWebRestart,
  meshPull,
  meshPush,
  type KimiSyncResult,
  type MeshPeerView,
} from "./mesh";
import { startServer, stopServer, type SyncInfo } from "./sync";
import { buildServerUrl } from "./server-url";
import type { AgentEvent } from "./protocol/state";
import { TrayController } from "./tray";
import { ProbeError, probeBackend } from "./probe";

/** 状态每次变化都自增，前端据此判断是否需要重绘。 */
let revision = 0;

export interface AppView {
  revision: number;
  settings: DesktopSettings;
  servers: ServerSummary[];
  activeId: string | null;
  statuses: Record<string, ServerStatusView>;
}

import type { ImportPreview } from "../../src/model";

export class AppState {
  readonly store: ConfigStore;
  readonly pinned = new PinnedSessions();
  readonly monitor: MonitorManager;
  private statuses = new Map<string, ServerStatusView>();
  private config: AppConfig | null = null;

  constructor(
    store: ConfigStore,
    private readonly onChange: (view: AppView) => void,
    private readonly onEvent: (event: AgentEvent) => void,
  ) {
    this.store = store;
    // 监控任务已经把会话列表汇总好了，这里只负责存下来并广播。
    this.monitor = new MonitorManager(this.pinned, (serverId, update) => {
      if (update.kind === "status") this.statuses.set(serverId, update.status);
      else this.onEvent(update.event);
      if (this.config) this.onChange(this.buildView());
    });
  }

  peekStatus(serverId: string): ServerStatusView | undefined {
    return this.statuses.get(serverId);
  }

  /** 当前视图快照，供 IPC 直接返回。 */
  peekView(): AppView {
    return this.buildView();
  }

  async start(): Promise<AppConfig> {
    const config = await this.store.load();
    this.config = config;
    this.monitor.syncServers(config.servers);
    this.onChange(this.buildView());
    return config;
  }

  /** 把配置变化广播给渲染进程，并让监控任务跟上。 */
  async publish(config?: AppConfig): Promise<void> {
    this.config = config ?? (await this.store.load());
    this.monitor.syncServers(this.config.servers);
    this.onChange(this.buildView());
  }

  private buildView(): AppView {
    const config = this.config;
    if (!config) {
      throw new Error("AppState 尚未加载配置");
    }
    revision += 1;
    // 服务被删掉后，残留状态要一并清掉，否则列表里会挂着一个幽灵条目。
    const live = new Set(config.servers.map((server) => server.id));
    for (const id of [...this.statuses.keys()]) {
      if (!live.has(id)) this.statuses.delete(id);
    }
    return {
      revision,
      settings: config.settings,
      servers: config.servers.map(toSummary),
      activeId: config.activeId,
      statuses: Object.fromEntries([...this.statuses].map(([id, status]) => [id, {
        ...status, sessions: status.sessions.map(session => ({ ...session,
          pinned: this.pinned.has(id, session.id), done: session.done && this.pinned.has(id, session.id) })),
      }])), 
    };
  }
}

function nowIso(): string {
  return new Date().toISOString();
}

function emptyStatus(error: string | null): ServerStatusView {
  return {
    connected: false,
    activeCount: 0,
    sessions: [],
    serverVersion: null,
    lastCheckedAt: nowIso(),
    error,
  };
}

/** 注册全部 IPC 命令。 */
export function registerCommands(
  window: BrowserWindow,
  state: AppState,
  tray: TrayController,
  appDir: string,
): void {
  const handle = <T>(
    channel: string,
    handler: (...args: never[]) => Promise<T> | T,
  ): void => {
    ipcMain.handle(channel, async (event, ...args) => {
      const ballOnly = ["ball:toggle", "ball:drag", "ball:get-state"].includes(channel);
      const allowed = ballOnly ? ballWindowRef()?.webContents : window.webContents;
      if (!allowed || event.sender !== allowed || event.senderFrame !== allowed.mainFrame) {
        throw new Error("IPC sender denied");
      }
      if (isolatedMode && (channel.startsWith("mesh:") || channel.startsWith("sync:"))) {
        if (channel === "mesh:discover") return [];
        throw new Error("隔离模式禁止外部节点及配对操作");
      }
      return handler(...(args as never[]));
    });
  };

  handle<AppView>("app:get-view", async () => {
    await state.publish();
    // publish 已经把视图推给渲染进程了；这里再取一份，保证首次调用也能拿到数据。
    return state.peekView();
  });

  handle<ServerForEdit>("server:for-edit", async (id: string) => {
    const config = await state.store.load();
    const server = config.servers.find((item) => item.id === id);
    if (!server) throw new Error(`找不到服务连接：${id}`);
    return toForEdit(server);
  });

  handle<AppView>("server:save", async (draft: ServerConfig) => {
    validateServer(draft);
    const name = draft.name.trim();
    if (!name) throw new Error("名称不能为空");
    const server = { ...draft, name, id: draft.id || randomUUID() };
    const saved = await state.store.update(config => {
      const index = config.servers.findIndex(item => item.id === server.id);
      if (index >= 0) config.servers[index] = server; else config.servers.push(server);
      if (!config.activeId) config.activeId = server.id;
    });
    agentView.refreshServers(window, saved.servers);
    await state.publish(saved);
    return state.peekView();
  });

  handle<AppView>("server:delete", async (id: string) => {
    state.pinned.removeServer(id);
    const saved = await state.store.update(config => {
      config.servers = config.servers.filter(item => item.id !== id);
      if (config.activeId === id) config.activeId = config.servers[0]?.id ?? null;
    });
    agentView.refreshServers(window, saved.servers);
    await state.publish(saved);
    return state.peekView();
  });

  handle<Backend>("server:probe", (draft: ServerConfig) => probeBackend(draft));
  handle<void>("server:reconnect-all", () => state.monitor.reconnectAll());
  handle<void>("workspace:sync", (snapshot: WorkspaceSnapshot) =>
    agentView.syncWorkspace(window, snapshot, async () => (await state.store.load()).servers));
  handle<void>("workspace:retry", async (id: string) =>
    agentView.retryTab(window, id, (await state.store.load()).servers));
  handle<void>("workspace:drop-indicator", (rect: DropIndicatorRect | null) => setDropIndicator(window, rect));
  window.webContents.on("did-start-navigation", (_event, _url, inPlace, mainFrame) => {
    if (mainFrame && !inPlace) agentView.closeAll(window);
  });

  handle<void>("window:set-always-on-top", (pinned: boolean) => {
    window.setAlwaysOnTop(pinned);
    tray.refresh();
  });

  handle<PinToggleResult>("pin:toggle", async (serverId: string, sessionId: string) => {
    state.pinned.toggle(serverId, sessionId);
    const pinned = state.pinned.has(serverId, sessionId);
    await state.publish();
    return { pinned, view: state.peekView() };
  });

  handle<AppView>("settings:update", async (settings: DesktopSettings) => {
    const saved = await state.store.update(config => { config.settings = settings; });
    if (!isolatedMode) app.setLoginItemSettings({ openAtLogin: settings.autostart });
    tray.refresh();
    await state.publish(saved);
    return state.peekView();
  });

  const previews = new Map<string, ImportPreviewData>();
  handle<ImportPreview>("import:preview-text", async (content: string) => {
    const data = await state.store.previewImportText(content);
    const importId = randomUUID();
    previews.clear();
    previews.set(importId, data);
    return { importId, validCount: data.preview.validServers,
      invalid: data.issues ?? [] };
  });

  handle<AppView>("import:apply", async (id: string, mode: ImportMode) => {
    const data = previews.get(id);
    if (!data || !["merge", "replace"].includes(mode)) throw new Error("导入预览已失效");
    const saved = await state.store.applyImportAndSave(data, mode);
    previews.delete(id);
    agentView.refreshServers(window, saved.servers);
    await state.publish(saved);
    return state.peekView();
  });

  handle<string>("export:text", () => state.store.load().then((config) => state.store.exportText(config)));

  handle<string | null>("dialog:choose-export-path", async () => {
    const result = await dialog.showSaveDialog(window, {
      title: "导出配置",
      defaultPath: "agentpocket-config.json",
      filters: [{ name: "JSON 配置", extensions: ["json"] }],
    });
    return result.canceled ? null : (result.filePath ?? null);
  });

  handle<void>("export:to", async (path: string) => {
    const config = await state.store.load();
    await state.store.exportTo(config, path);
  });

  handle<void>("shell:open-external", (url: string) => {
    // 只放行 http/https，避免被诱导执行本地协议。
    const parsed = new URL(url);
    if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
      throw new Error(`拒绝打开非 http(s) 链接：${parsed.protocol}`);
    }
    void shell.openExternal(parsed.toString());
  });

  handle<string>("agent:preview-url", async (id: string, sessionId: string | null) => {
    const config = await state.store.load();
    const server = config.servers.find((item) => item.id === id);
    if (!server) throw new Error(`找不到服务连接：${id}`);
    return buildServerUrl(server, sessionId).toString();
  });

  handle<MeshPeerView[]>("mesh:discover", () => discoverPeers(appDir));

  handle<KimiSyncResult>("mesh:pull", (host: string) => meshPull(host));
  handle<KimiSyncResult>("mesh:push", (host: string) => meshPush(host));
  handle<string>("mesh:upgrade", (host: string) => meshKimiUpgrade(host));
  handle<string>("mesh:restart-web", (host: string, force: boolean) =>
    meshKimiWebRestart(host, force),
  );

  handle<SyncInfo>("sync:start", () => startServer(state.store));
  handle<void>("sync:stop", () => stopServer());

  handle<void>("ball:toggle", () => {
    if (window.isMinimized()) window.restore();
    window.show();
    window.focus();
    window.webContents.send("ball:toggle-request");
  });
  handle<void>("ball:drag", (action: BallDrag) => {
    if (!["start", "move", "end", "cancel"].includes(action)) throw new Error("无效拖动动作");
    dragBall(action);
  });
  handle<BallSnapshot>("ball:get-state", () => ballSnapshot());
  handle<void>("ball:publish-state", (state: BallState) => publishBallState(state));
}