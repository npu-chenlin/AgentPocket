export interface WorkspaceTab {
  id: string;
  serverId: string;
  sessionId: string | null;
  title: string;
}

export interface AgentBounds { x: number; y: number; width: number; height: number }
export interface WorkspaceSnapshot {
  revision: number;
  tabs: WorkspaceTab[];
  visible: Record<string, AgentBounds>;
  activeId: string | null;
  overlays: string[];
}
export interface TabStatus {
  tabId: string;
  state: "loading" | "ready" | "error";
  error?: string;
}

export function validTab(value: unknown): value is WorkspaceTab {
  if (!value || typeof value !== "object") return false;
  const tab = value as WorkspaceTab;
  return typeof tab.id === "string" && /^[\w-]{1,80}$/.test(tab.id)
    && typeof tab.serverId === "string" && tab.serverId.length > 0 && tab.serverId.length <= 200
    && (tab.sessionId === null || (typeof tab.sessionId === "string" && tab.sessionId.length <= 1000))
    && typeof tab.title === "string" && tab.title.length <= 300;
}

export function validateSnapshot(value: WorkspaceSnapshot): void {
  if (!value || !Number.isSafeInteger(value.revision) || value.revision < 0
    || !Array.isArray(value.tabs) || value.tabs.length > 100 || !value.tabs.every(validTab)
    || new Set(value.tabs.map(t => t.id)).size !== value.tabs.length
    || !Array.isArray(value.overlays) || value.overlays.length > 10
    || !value.overlays.every(s => typeof s === "string" && s.length < 80)
    || !value.visible || typeof value.visible !== "object") throw new Error("无效工作区快照");
  const ids = new Set(value.tabs.map(t => t.id));
  if (value.activeId !== null && !ids.has(value.activeId)) throw new Error("无效活动标签");
  for (const [id, rect] of Object.entries(value.visible)) {
    if (!ids.has(id) || !rect || ![rect.x, rect.y, rect.width, rect.height].every(Number.isFinite)
      || rect.width < 0 || rect.height < 0) throw new Error("无效面板矩形");
  }
}

export function clipBounds(rect: AgentBounds, width: number, height: number, zoom = 1): AgentBounds {
  const x = Math.max(0, Math.min(width, Math.round(rect.x * zoom)));
  const y = Math.max(0, Math.min(height, Math.round(rect.y * zoom)));
  return { x, y, width: Math.max(0, Math.min(width, Math.round((rect.x + rect.width) * zoom)) - x),
    height: Math.max(0, Math.min(height, Math.round((rect.y + rect.height) * zoom)) - y) };
}
