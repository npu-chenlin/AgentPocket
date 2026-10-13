export type { Backend, DesktopSettings, MeshPeerEntry, ServerSummary, ServerForEdit } from "../electron/main/model";
export type { MeshPeerView, KimiSyncResult } from "../electron/main/mesh";
export type { SessionSummary, ServerStatusView as ServerStatus } from "../electron/main/monitor/manager";
export type { SyncInfo, SyncOption } from "../electron/main/sync";
export type { ImportMode } from "../electron/main/config-store";
export type { AppView } from "../electron/main/commands";
import type { AppView } from "../electron/main/commands";
import type { ServerConfig } from "../electron/main/model";
export type ServerDraft = ServerConfig;
export interface PinToggleResult { pinned: boolean; view: AppView }
export interface ImportIssue { index: number; reason: string }
export interface ImportPreview { importId: string; validCount: number; invalid: ImportIssue[] }
export function emptyServerDraft(): ServerDraft {
  return { id: "", name: "", host: "", port: 3080, token: "", backend: "dsh" };
}
