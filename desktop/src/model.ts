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

export interface ServerStatus {
  connected: boolean;
  activeCount: number;
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
  sourceKind: "full" | "android";
}

export type ImportMode = "merge" | "replace";
export type ExportFormat = "full" | "android";

export const commands = {
  getAppView: "get_app_view",
  getServerForEdit: "get_server_for_edit",
  saveServer: "save_server",
  deleteServer: "delete_server",
  setActiveServer: "set_active_server",
  updateSettings: "update_settings",
  probeBackend: "probe_backend",
  reconnectAll: "reconnect_all",
  openServer: "open_server",
  previewImport: "preview_import",
  previewImportText: "preview_import_text",
  applyImport: "apply_import",
  exportConfig: "export_config",
  exportConfigText: "export_config_text",
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
