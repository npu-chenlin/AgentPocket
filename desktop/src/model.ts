export type Backend = "kimi" | "dsh";

export interface DesktopSettings {
  startHidden: boolean;
  autostart: boolean;
  notifications: boolean;
}

export interface ServerSummary {
  id: string;
  name: string;
  host: string;
  port: number;
  backend: Backend;
}

export interface SessionSummary {
  id: string;
  title: string;
}

export interface ServerStatus {
  connected: boolean;
  activeCount: number;
  /** 正在运行的会话列表（仅忙碌会话）。 */
  sessions: SessionSummary[];
  /** 服务端版本号（kimi 提供；dsh 为 null）。 */
  serverVersion: string | null;
  lastCheckedAt: string | null;
  error: string | null;
}

export interface AppView {
  revision: number;
  settings: DesktopSettings;
  servers: ServerSummary[];
  activeId: string | null;
  statuses: Record<string, ServerStatus>;
}

export interface ServerDraft {
  id: string;
  name: string;
  host: string;
  port: number;
  token: string;
  backend: Backend;
}

export type ServerForEdit = ServerDraft;

export interface ImportIssue {
  index: number;
  reason: string;
}

export interface ImportPreview {
  importId: string;
  validCount: number;
  invalid: ImportIssue[];
}

export type ImportMode = "merge" | "replace";

export interface SyncOption {
  address: string;
  label: string;
  url: string;
  qrSvg: string;
}

export interface SyncInfo {
  selected: string;
  options: SyncOption[];
}

export const commands = {
  getAppView: "get_app_view",
  getServerForEdit: "get_server_for_edit",
  saveServer: "save_server",
  deleteServer: "delete_server",
  updateSettings: "update_settings",
  probeBackend: "probe_backend",
  reconnectAll: "reconnect_all",
  openServer: "open_server",
  setAlwaysOnTop: "set_always_on_top",
  previewImportText: "preview_import_text",
  applyImport: "apply_import",
  exportConfig: "export_config",
  exportConfigText: "export_config_text",
  startSyncServer: "start_sync_server",
  stopSyncServer: "stop_sync_server",
} as const;

export function emptyServerDraft(): ServerDraft {
  return {
    id: "",
    name: "",
    host: "",
    port: 3080,
    token: "",
    backend: "dsh",
  };
}
