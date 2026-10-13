/**
 * IPC 命令表。对应 Rust 侧 src-tauri/src/commands.rs。
 *
 * 渲染进程只能通过 preload 暴露的 invoke 调到这里；所有错误都原样抛回，
 * 不做静默兜底。
 */

import { BrowserWindow, Notification, dialog, ipcMain, shell } from "electron";

import * as agentView from "./agent-view";
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

export interface ImportPreview {
  importId: string;
  validCount: number;
  invalidCount: number;
  servers: ServerSummary[];
  issues: string[];
}

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
      statuses: Object.fromEntries(this.statuses),
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
    ipcMain.handle(channel, async (_event, ...args) => handler(...(args as never[])));
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

  handle<void>("server:save", async (draft: ServerConfig) => {
    validateServer(draft);
    const name = draft.name.trim();
    if (name.length === 0) throw new Error("名称不能为空");

    const saved = await state.store.update((config) => {
      const existing = config.servers.findIndex((item) => item.id === draft.id);
      if (existing >= 0) config.servers[existing] = { ...draft, name };
      else config.servers.push({ ...draft, name });
      if (!config.activeId) config.activeId = draft.id;
    });
    // 凭据变了必须重建视图，否则会一直用旧 cookie。
    agentView.closeServer(window, draft.id);
    await state.publish(saved);
  });

  handle<void>("server:delete", async (id: string) => {
    agentView.closeServer(window, id);
    state.pinned.removeServer(id);
    const saved = await state.store.update((config) => {
      config.servers = config.servers.filter((item) => item.id !== id);
      if (config.activeId === id) config.activeId = config.servers[0]?.id ?? null;
    });
    await state.publish(saved);
  });

  handle<Backend>("server:probe", (draft: ServerConfig) =>
    probeBackend(draft).catch((error: ProbeError) => {
      throw new Error(error.message);
    }),
  );

  handle<void>("server:reconnect-all", () => {
    state.monitor.reconnectAll();
  });

  handle<void>("agent:open", async (id: string, sessionId: string | null) => {
    const config = await state.store.load();
    const server = config.servers.find((item) => item.id === id);
    if (!server) throw new Error(`找不到服务连接：${id}`);
    await agentView.openServer(window, server, sessionId);
  });

  handle<void>("agent:set-bounds", (rect: agentView.AgentBounds) => {
    agentView.setBounds(rect);
  });

  handle<void>("agent:set-overlaid", (overlaid: boolean) => {
    agentView.setOverlaid(overlaid);
  });

  handle<void>("window:set-always-on-top", (pinned: boolean) => {
    window.setAlwaysOnTop(pinned);
    tray.refresh();
  });

  handle<boolean>("pin:toggle", (serverId: string, sessionId: string) => {
    const changed = state.pinned.toggle(serverId, sessionId);
    void state.publish();
    return changed;
  });

  handle<void>("settings:update", async (settings: DesktopSettings) => {
    const saved = await state.store.update((config) => {
      config.settings = settings;
    });
    tray.refresh();
    await state.publish(saved);
  });

  handle<ImportPreviewData>("import:preview-text", (content: string) =>
    state.store.previewImportText(content),
  );

  handle<void>("import:apply", async (data: ImportPreviewData, mode: ImportMode) => {
    const saved = await state.store.applyImportAndSave(data, mode);
    await state.publish(saved);
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
}