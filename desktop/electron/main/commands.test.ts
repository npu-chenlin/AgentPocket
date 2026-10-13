import { beforeEach, describe, expect, it, vi } from "vitest";
const mocks = vi.hoisted(() => ({ handlers: new Map<string, (...args: any[]) => Promise<unknown>>(),
  ball: { webContents: { mainFrame: {} } }, publish: vi.fn(), peek: vi.fn(() => ({ revision: 1 })) }));
vi.mock("electron", () => ({ ipcMain: { handle: (name: string, fn: any) => mocks.handlers.set(name, fn) },
  app: {}, BrowserWindow: class {}, Notification: class {}, dialog: {}, shell: {}, Menu: {}, Tray: class {}, nativeImage: {} }));
vi.mock("./runtime", () => ({ isolatedMode: true }));
vi.mock("./ball-window", () => ({ ballWindowRef: () => mocks.ball, ballSnapshot: () => ({ revision: 3 }), dragBall: vi.fn(), publishBallState: vi.fn() }));
import { registerCommands } from "./commands";
const main = { webContents: { mainFrame: {}, on: vi.fn() } };
const event = (sender: any, senderFrame = sender.mainFrame) => ({ sender, senderFrame });
beforeEach(() => {
  mocks.handlers.clear();
  registerCommands(main as never, { publish: mocks.publish, peekView: mocks.peek } as never, {} as never, "isolated");
});
describe("IPC 身份与 frame 边界", () => {
  it("主窗口顶层 frame 才能读取配置", async () => {
    const handler = mocks.handlers.get("app:get-view")!;
    await expect(handler(event(main.webContents))).resolves.toEqual({ revision: 1 });
    await expect(handler(event(main.webContents, {}))).rejects.toThrow("sender denied");
    await expect(handler(event(mocks.ball.webContents))).rejects.toThrow("sender denied");
    await expect(handler(event({ mainFrame: {} }))).rejects.toThrow("sender denied");
  });
  it("球只允许专有动作，不能假冒 main 发布状态或操作工作区", async () => {
    await expect(mocks.handlers.get("ball:get-state")!(event(mocks.ball.webContents))).resolves.toEqual({ revision: 3 });
    for (const channel of ["workspace:sync", "server:for-edit", "server:delete", "ball:publish-state", "export:text"]) {
      await expect(mocks.handlers.get(channel)!(event(mocks.ball.webContents))).rejects.toThrow("sender denied");
    }
    await expect(mocks.handlers.get("ball:get-state")!(event(main.webContents))).rejects.toThrow("sender denied");
    await expect(mocks.handlers.get("ball:get-state")!(event(mocks.ball.webContents, {}))).rejects.toThrow("sender denied");
  });
  it("隔离启动拒绝外部节点发现与配对副作用", async () => {
    await expect(mocks.handlers.get("mesh:discover")!(event(main.webContents))).resolves.toEqual([]);
    await expect(mocks.handlers.get("mesh:push")!(event(main.webContents), "remote")).rejects.toThrow("隔离模式");
    await expect(mocks.handlers.get("sync:start")!(event(main.webContents))).rejects.toThrow("隔离模式");
  });
});
