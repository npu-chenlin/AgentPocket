import { render, screen } from "@testing-library/svelte";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AppView } from "../../electron/main/commands";

/**
 * 组件测试里的 Electron 环境替身。
 *
 * 只桩掉组件真正会调用的方法；其余保持 undefined，这样"调了不该调的方法"
 * 会立刻暴露，而不是悄悄通过。
 */
function stubApi(overrides: Partial<Record<string, unknown>> = {}): Record<string, unknown> {
  return {
    getAppView: vi.fn(async (): Promise<AppView> => ({
      revision: 1,
      settings: { startHidden: true, autostart: true, notifications: true, meshPeers: [] },
      servers: [
        { id: "s1", name: "工作站", host: "100.64.0.2", port: 4096, backend: "opencode" },
      ],
      activeId: "s1",
      statuses: {
        s1: {
          connected: true,
          activeCount: 1,
          sessions: [
            { id: "a", title: "重构登录", activity: null, pinned: false, done: false },
          ],
          serverVersion: null,
          lastCheckedAt: null,
          error: null,
        },
      },
    })),
    setWebviewBounds: vi.fn(async () => {}),
    setWebviewOverlaid: vi.fn(async () => {}),
    openServer: vi.fn(async () => {}),
    onStateChanged: vi.fn(() => () => {}),
    onAgentEvent: vi.fn(() => () => {}),
    onMenuAction: vi.fn(() => () => {}),
    ...overrides,
  };
}

describe("App", () => {
  beforeEach(() => {
    (globalThis as { agentpocket?: unknown }).agentpocket = stubApi();
  });

  it("挂载后渲染外壳与服务列表", async () => {
    const { default: App } = await import("./App.svelte");
    render(App);
    // 应用名已移入原生菜单栏，界面上不再出现——这里确认服务列表挂出来了。
    expect(await screen.findByText("工作站")).toBeInTheDocument();
    expect(screen.queryByText("AgentPocket")).not.toBeInTheDocument();
  });

  it("没有标题栏，窗口级操作全在菜单里", async () => {
    const { default: App } = await import("./App.svelte");
    const { container } = render(App);
    await screen.findByText("工作站");
    // 置顶移进「视图」菜单后，标题栏连同它一起删了。
    expect(container.querySelector(".app-header")).toBeNull();
    expect(screen.queryByRole("button", { name: "窗口置顶" })).not.toBeInTheDocument();
    // 侧边栏与内容区占满整个窗口
    expect(container.querySelector(".sidebar")).not.toBeNull();
    expect(container.querySelector(".stage")).not.toBeNull();
  });

  it("启动后把内容区矩形上报给主进程", async () => {
    const api = stubApi();
    (globalThis as { agentpocket?: unknown }).agentpocket = api;
    const { default: App } = await import("./App.svelte");
    render(App);
    await screen.findByText("工作站");
    expect(api.setWebviewBounds).toHaveBeenCalled();
  });
});