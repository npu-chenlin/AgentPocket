import type { AgentPocketApi } from "../../electron/preload";

/**
 * 把任意异常转成可展示的短文案。
 *
 * Electron 的 IPC 错误会带上 "Error invoking remote method ..." 前缀，
 * 对用户没有意义，剥掉。
 */
export function errorMessage(error: unknown): string {
  if (typeof error === "string") return stripInvokePrefix(error);
  if (error instanceof Error) return stripInvokePrefix(error.message);
  try {
    return stripInvokePrefix(JSON.stringify(error));
  } catch {
    return "发生未知错误";
  }
}

const INVOKE_PREFIX = /^Error invoking remote method '[^']+':\s*(Error:\s*)?/;

function stripInvokePrefix(text: string): string {
  return text.replace(INVOKE_PREFIX, "");
}

/**
 * IPC 桥。窗口 `agentpocket` 由 preload 注入；不在 Electron 里时抛错，
 * 而不是静默返回默认值——那样问题会被藏到很后面才暴露。
 */
function bridge(): AgentPocketApi {
  const api = (globalThis as { agentpocket?: AgentPocketApi }).agentpocket;
  if (!api) {
    throw new Error("当前不在 Electron 环境中运行：preload 未注入 agentpocket");
  }
  return api;
}

export function getAppView() {
  return bridge().getAppView();
}

export function getServerForEdit(id: string) {
  return bridge().getServerForEdit(id);
}

export function saveServer(server: import("../model").ServerDraft) {
  return bridge().saveServer(server as Parameters<AgentPocketApi["saveServer"]>[0]);
}

export function deleteServer(id: string) {
  return bridge().deleteServer(id);
}

export function probeBackend(server: import("../model").ServerDraft) {
  return bridge().probeBackend(server as Parameters<AgentPocketApi["probeBackend"]>[0]);
}

export function reconnectAll() {
  return bridge().reconnectAll();
}

/** 打开服务/会话到主窗口内的 Agent 视图。 */
export function openServer(id: string, sessionId: string | null) {
  return bridge().openServer(id, sessionId);
}

/** 上报内容区矩形，Agent 视图据此定位。 */
export function setWebviewBounds(rect: {
  x: number;
  y: number;
  width: number;
  height: number;
}) {
  return bridge().setWebviewBounds(rect);
}

/** 弹窗打开时收起 Agent 视图；它是原生控件，层级在 DOM 之上。 */
export function setWebviewOverlaid(overlaid: boolean) {
  return bridge().setWebviewOverlaid(overlaid);
}

export function setAlwaysOnTop(pinned: boolean) {
  return bridge().setAlwaysOnTop(pinned);
}

export function toggleSessionPin(id: string, sessionId: string) {
  return bridge().toggleSessionPin(id, sessionId);
}

export function updateSettings(settings: import("../model").DesktopSettings) {
  return bridge().updateSettings(settings);
}

export function previewImportText(content: string) {
  return bridge().previewImportText(content);
}

export function applyImport(data: unknown, mode: "merge" | "replace") {
  return bridge().applyImport(data as never, mode);
}

export function exportConfigText() {
  return bridge().exportConfigText();
}

export function exportConfig(path: string) {
  return bridge().exportConfig(path);
}

export function openExternal(url: string) {
  return bridge().openExternal(url);
}

export function discoverMeshPeers() {
  return bridge().discoverMeshPeers();
}

export function meshPull(host: string) {
  return bridge().meshPull(host);
}

export function meshPush(host: string) {
  return bridge().meshPush(host);
}

export function meshKimiUpgrade(host: string) {
  return bridge().meshKimiUpgrade(host);
}

export function meshKimiWebRestart(host: string, force: boolean) {
  return bridge().meshKimiWebRestart(host, force);
}

export function startSyncServer() {
  return bridge().startSyncServer();
}

export function stopSyncServer() {
  return bridge().stopSyncServer();
}

/** 订阅主进程推送的应用状态，返回取消订阅函数。 */
export function onStateChanged(callback: Parameters<AgentPocketApi["onStateChanged"]>[0]) {
  return bridge().onStateChanged(callback);
}

/** 订阅主进程推送的 Agent 事件。 */
export function onAgentEvent(callback: (payload: unknown) => void) {
  return bridge().onAgentEvent(callback);
}