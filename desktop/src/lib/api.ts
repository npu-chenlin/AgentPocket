import type { AgentPocketApi, WebviewRect } from "../../electron/preload";

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

export const syncWorkspace: AgentPocketApi["syncWorkspace"] = snapshot => bridge().syncWorkspace(snapshot);
export const retryTab: AgentPocketApi["retryTab"] = id => bridge().retryTab(id);
export const setDropIndicator: AgentPocketApi["setDropIndicator"] = rect => bridge().setDropIndicator(rect);
export const onTabStatus: AgentPocketApi["onTabStatus"] = callback => bridge().onTabStatus(callback);
export const onTabFocus: AgentPocketApi["onTabFocus"] = callback => bridge().onTabFocus(callback);

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

export function chooseExportPath() {
  return bridge().chooseExportPath();
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

/**
 * 订阅原生菜单动作。
 *
 * 导入/导出/手机配对/设置已移进应用菜单栏，这里只收动作名，由调用方执行——
 * 业务逻辑留在渲染进程，不在两边各写一遍。
 */
export function onMenuAction(callback: Parameters<AgentPocketApi["onMenuAction"]>[0]) {
  return bridge().onMenuAction(callback);
}

/** 订阅悬浮球窗口发来的"开合面板"请求。 */
export function onBallToggle(callback: () => void) {
  return bridge().onBallToggle(callback);
}

/** 把表情状态推给悬浮球窗口。 */
export function publishBallState(state: import("../../electron/main/ball-ipc").BallState) {
  return bridge().publishBallState(state);
}