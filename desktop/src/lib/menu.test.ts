/**
 * 原生菜单栏 → 界面的接线。
 *
 * 导入/导出/手机配对/设置从标题栏搬进了菜单栏，这条链路断了的话，菜单点了
 * 什么都不会发生，而界面本身看不出任何异常——所以单独锁住。
 */

import { render, screen, waitFor } from "@testing-library/svelte";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AppView } from "../../electron/main/commands";

function appView(): AppView {
  return {
    revision: 1,
    settings: { startHidden: true, autostart: true, notifications: true, meshPeers: [] },
    servers: [{ id: "s1", name: "工作站", host: "100.64.0.2", port: 4096, backend: "kimi" }],
    activeId: null,
    statuses: {},
  };
}

/** 记录主进程发来的菜单动作，好让测试能触发它。 */
function stubApi() {
  let menuHandler: ((action: string) => void) | null = null;
  const api: Record<string, unknown> = {
    getAppView: vi.fn(async () => appView()),
    syncWorkspace: vi.fn(async () => {}),
    retryTab: vi.fn(async () => {}),
    setDropIndicator: vi.fn(async () => {}),
    captureViews: vi.fn(async () => ({})),
    onTabStatus: vi.fn(() => () => {}),
    onTabFocus: vi.fn(() => () => {}),
    discoverMeshPeers: vi.fn(async () => []),
    setAlwaysOnTop: vi.fn(async () => {}),
    exportConfigText: vi.fn(async () => "{}"),
    startSyncServer: vi.fn(async () => {
      throw new Error("测试里不真的起配对服务");
    }),
    onStateChanged: vi.fn(() => () => {}),
    onAgentEvent: vi.fn(() => () => {}),
    onBallToggle: vi.fn(() => () => {}),
    publishBallState: vi.fn(async () => {}),
    onMenuAction: vi.fn((cb: (action: string) => void) => {
      menuHandler = cb;
      return () => {
        menuHandler = null;
      };
    }),
  };
  return {
    api,
    /** 模拟用户在菜单栏点了某一项。 */
    fireMenu: (action: string) => {
      if (!menuHandler) throw new Error("还没订阅菜单动作");
      menuHandler(action);
    },
    hasHandler: () => menuHandler !== null,
  };
}

describe("原生菜单栏", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("标题栏不再有导入/导出/手机配对/设置按钮", async () => {
    const { api } = stubApi();
    (globalThis as { agentpocket?: unknown }).agentpocket = api;
    const { default: App } = await import("./App.svelte");
    render(App);
    await screen.findByText("选一个服务打开");

    // 这些都搬进菜单了，界面上不该再有
    expect(screen.queryByRole("button", { name: "导入" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "导出" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "手机配对" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "设置" })).not.toBeInTheDocument();
  });

  it("挂载时订阅菜单动作", async () => {
    const { api, hasHandler } = stubApi();
    (globalThis as { agentpocket?: unknown }).agentpocket = api;
    const { default: App } = await import("./App.svelte");
    render(App);
    await screen.findByText("选一个服务打开");
    expect(hasHandler()).toBe(true);
  });

  it("菜单「导入」打开粘贴导入对话框", async () => {
    const { api, fireMenu } = stubApi();
    (globalThis as { agentpocket?: unknown }).agentpocket = api;
    const { default: App } = await import("./App.svelte");
    render(App);
    await screen.findByText("选一个服务打开");

    fireMenu("import");
    await screen.findByRole("dialog");
  });

  it("菜单「设置」打开设置对话框", async () => {
    const { api, fireMenu } = stubApi();
    (globalThis as { agentpocket?: unknown }).agentpocket = api;
    const { default: App } = await import("./App.svelte");
    render(App);
    await screen.findByText("选一个服务打开");

    fireMenu("settings");
    await waitFor(() => {
      const dialogs = document.querySelectorAll("dialog.modal[open]");
      expect(dialogs.length).toBeGreaterThan(0);
    });
  });

  it("菜单「导出」走导出流程", async () => {
    const { api, fireMenu } = stubApi();
    (globalThis as { agentpocket?: unknown }).agentpocket = api;
    const { default: App } = await import("./App.svelte");
    render(App);
    await screen.findByText("选一个服务打开");

    fireMenu("export");
    await waitFor(() => {
      // 导出要挑路径，桩里没实现，会走到错误提示——说明确实被调用了
      expect(screen.getByRole("dialog")).toBeInTheDocument();
    });
  });

  it("菜单「显示/隐藏悬浮球面板」开合悬浮球面板", async () => {
    const { api, fireMenu } = stubApi();
    (globalThis as { agentpocket?: unknown }).agentpocket = api;
    const { default: App } = await import("./App.svelte");
    render(App);
    await screen.findByText("选一个服务打开");

    expect(screen.queryByRole("dialog", { name: "悬浮球面板" })).not.toBeInTheDocument();
    fireMenu("toggle-launcher");
    await screen.findByRole("dialog", { name: "悬浮球面板" });

    fireMenu("toggle-launcher");
    await waitFor(() => {
      expect(screen.queryByRole("dialog", { name: "悬浮球面板" })).not.toBeInTheDocument();
    });
  });

  it("菜单「窗口置顶」请求主进程置顶", async () => {
    const { api, fireMenu } = stubApi();
    (globalThis as { agentpocket?: unknown }).agentpocket = api;
    const { default: App } = await import("./App.svelte");
    render(App);
    await screen.findByText("选一个服务打开");

    // 标题栏已删，置顶只剩菜单这一个入口：点了必须真的调主进程。
    fireMenu("toggle-always-on-top");
    await waitFor(() => {
      expect(api.setAlwaysOnTop).toHaveBeenCalledWith(true);
    });
    // 再点一次取消
    fireMenu("toggle-always-on-top");
    await waitFor(() => {
      expect(api.setAlwaysOnTop).toHaveBeenLastCalledWith(false);
    });
  });

  it("认不出的菜单动作不会把界面弄坏", async () => {
    const { api, fireMenu } = stubApi();
    (globalThis as { agentpocket?: unknown }).agentpocket = api;
    const { default: App } = await import("./App.svelte");
    const { container } = render(App);
    await screen.findByText("选一个服务打开");

    expect(() => fireMenu("future-action-from-newer-menu")).not.toThrow();
    // 只看本次渲染的容器：同文件里前面的用例可能还留着异步收尾，
    // 断言全局 DOM 会受牵连。
    expect(container.querySelector(".workspace")).not.toBeNull();
  });
});