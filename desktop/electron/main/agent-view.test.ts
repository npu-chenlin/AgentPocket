import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import { EventEmitter } from "node:events";
import type { BrowserWindow } from "electron";
import type { WorkspaceSnapshot } from "../../shared/workspace";
const mock = vi.hoisted(() => ({ views: [] as any[], loaders: [] as (() => Promise<void>)[] }));
vi.mock("electron", () => ({ shell: { openExternal: vi.fn() }, WebContentsView: class {
  setVisible = vi.fn(); setBounds = vi.fn(); webContents: any;
  constructor() {
    this.webContents = Object.assign(new EventEmitter(), { loadURL: vi.fn(() => (mock.loaders.shift() ?? (() => Promise.resolve()))()),
      close: vi.fn(), setWindowOpenHandler: vi.fn() }); mock.views.push(this);
  }
} }));
vi.mock("./auth", () => ({ installAuth: vi.fn(), sessionForServer: vi.fn(id => `session:${id}`) }));
import { syncWorkspace, closeAll, retryTab, refreshServers } from "./agent-view";
const host = { contentView: { addChildView: vi.fn(), removeChildView: vi.fn() }, webContents: { send: vi.fn(), getZoomFactor: () => 1 },
  getContentSize: () => [800, 600], isDestroyed: () => false } as unknown as BrowserWindow;
const server = { id: "s", name: "Sample", host: "127.0.0.1", port: 3080, backend: "dsh" as const, token: "" };
const tab = { id: "a", serverId: "s", sessionId: null, title: "A" };
const snap = (revision: number, tabs = [tab]): WorkspaceSnapshot => ({ revision, tabs,
  visible: Object.fromEntries(tabs.map(t => [t.id, { x: 10.2, y: 40.8, width: 300.1, height: 200.4 }])), activeId: tabs[0]?.id ?? null, overlays: [] });
const servers = async () => [server];
beforeEach(() => { mock.views.length = 0; mock.loaders.length = 0; vi.clearAllMocks(); });
afterEach(() => closeAll(host));
describe("WebContentsView lifecycle", () => {
  it("每标签一份实例，布局/切换不重新加载，坐标整数化", async () => {
    await syncWorkspace(host, snap(1, [tab, { ...tab, id: "b" }]), servers);
    expect(mock.views).toHaveLength(2);
    await syncWorkspace(host, { ...snap(2, [tab, { ...tab, id: "b" }]), visible: {} }, servers);
    for (const view of mock.views) {
      expect(view.webContents.loadURL).toHaveBeenCalledTimes(1);
      expect(view.setVisible).toHaveBeenLastCalledWith(false);
      expect(view.setBounds).toHaveBeenCalledWith({ x: 10, y: 41, width: 300, height: 200 });
    }
  });
  it("早到布局、晚到配置以最新 revision 为准", async () => {
    let resolve!: (value: typeof server[]) => void;
    const loading = syncWorkspace(host, snap(1), () => new Promise(r => { resolve = r; }));
    await syncWorkspace(host, snap(2, []), servers);
    resolve([server]); await loading;
    expect(mock.views).toHaveLength(0);
    await syncWorkspace(host, snap(1), servers);
    expect(mock.views).toHaveLength(0);
  });
  it("关闭后加载完成不会复活；关闭确实释放", async () => {
    let resolve!: () => void;
    mock.loaders.push(() => new Promise(r => { resolve = r; }));
    await syncWorkspace(host, snap(1), servers);
    const view = mock.views[0];
    await syncWorkspace(host, snap(2, []), servers);
    resolve(); await Promise.resolve();
    expect(view.webContents.close).toHaveBeenCalledOnce();
    expect(view.setVisible).not.toHaveBeenCalledWith(true);
  });
  it("失败保留标签，重试加载同一实例，凭据更新重建实例", async () => {
    mock.loaders.push(() => Promise.reject(Error("offline")));
    await syncWorkspace(host, snap(1), servers); await Promise.resolve();
    expect(host.webContents.send).toHaveBeenCalledWith("workspace:status", { tabId: "a", state: "error", error: "offline" });
    retryTab(host, "a"); await Promise.resolve();
    expect(mock.views[0].webContents.loadURL).toHaveBeenCalledTimes(2);
    refreshServers(host, [{ ...server, token: "synthetic-new" }]);
    expect(mock.views).toHaveLength(2);
    expect(mock.views[0].webContents.close).toHaveBeenCalledOnce();
  });
  it("模态遮挡与原生 focus", async () => {
    await syncWorkspace(host, { ...snap(1), overlays: ["modal", "interaction"] }, servers);
    await Promise.resolve();
    expect(mock.views[0].setVisible).toHaveBeenLastCalledWith(false);
    mock.views[0].webContents.emit("focus");
    expect(host.webContents.send).toHaveBeenCalledWith("workspace:focus", "a");
    await syncWorkspace(host, snap(2), servers);
    expect(mock.views[0].setVisible).toHaveBeenLastCalledWith(true);
  });
});
