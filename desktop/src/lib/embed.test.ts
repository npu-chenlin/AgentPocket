/**
 * 验收路径的回归测试：点服务 / 点会话 → 调 openServer → 上报内容区矩形。
 *
 * 这些正是当初从 Tauri 换到 Electron 要解决的事（把 Agent 页面嵌进主窗口并
 * 精确定位），所以用测试锁住：谁把这条链路改坏了，测试会红，而不是等到手动
 * 点一遍才发现。
 */

import { render, screen, waitFor } from "@testing-library/svelte";
import { describe, expect, it, vi } from "vitest";

import type { AppView } from "../../electron/main/commands";

function appView(): AppView {
  return {
    revision: 1,
    settings: { startHidden: true, autostart: true, notifications: true, meshPeers: [] },
    servers: [{ id: "s1", name: "工作站", host: "100.64.0.2", port: 4096, backend: "kimi" }],
    activeId: null,
    statuses: {
      s1: {
        connected: true,
        activeCount: 1,
        sessions: [
          {
            id: "sess-a",
            title: "重构登录",
            activity: "主 agent 已完成 · 等 1 个后台任务",
            pinned: false,
            done: false,
          },
        ],
        serverVersion: "1.2.3",
        lastCheckedAt: null,
        error: null,
      },
    },
  };
}

/** 只桩掉组件真正会调的接口；其余保持 undefined，让误用立刻暴露。 */
function stubApi(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    getAppView: vi.fn(async () => appView()),
    openServer: vi.fn(async () => {}),
    setWebviewBounds: vi.fn(async () => {}),
    setWebviewOverlaid: vi.fn(async () => {}),
    setAlwaysOnTop: vi.fn(async () => {}),
    toggleSessionPin: vi.fn(async () => true),
    onStateChanged: vi.fn(() => () => {}),
    onAgentEvent: vi.fn(() => () => {}),
    onMenuAction: vi.fn(() => () => {}),
    ...overrides,
  };
}

async function mount(api = stubApi()) {
  (globalThis as { agentpocket?: unknown }).agentpocket = api;
  const { default: App } = await import("./App.svelte");
  render(App);
  await screen.findByText("工作站");
  return api as { openServer: ReturnType<typeof vi.fn>; setWebviewBounds: ReturnType<typeof vi.fn> };
}

describe("内嵌 Agent 视图的接线", () => {
  // 不重置模块：resetModules 会让 Svelte 丢掉 effect 上下文（effect_orphan）。
  // 桥是在调用时才从 globalThis 读的，所以每个用例重新挂新的替身就够了。

  it("点服务名就打开该服务（会话为 null）", async () => {
    const api = await mount();

    screen.getByText("工作站").click();

    await waitFor(() => {
      expect(api.openServer).toHaveBeenCalledWith("s1", null);
    });
  });

  it("展开后点会话，带上会话 id 打开", async () => {
    const api = await mount();

    // 展开按钮只在"已连接且有会话"时出现。
    await screen.findByLabelText("展开 工作站 的会话").then((btn) => btn.click());
    screen.getByText("重构登录").click();

    await waitFor(() => {
      expect(api.openServer).toHaveBeenCalledWith("s1", "sess-a");
    });
  });

  it("启动后把内容区矩形上报给主进程，供视图定位", async () => {
    const api = await mount();

    await waitFor(() => {
      expect(api.setWebviewBounds).toHaveBeenCalled();
    });
    const rect = api.setWebviewBounds.mock.calls[0][0] as {
      x: number;
      y: number;
      width: number;
      height: number;
    };
    // 四个分量都必须是有限数：NaN 传进 setBounds 会让原生视图定位到乱处，
    // 而且不报错，只是"看起来没反应"——很难查。
    //
    // 宽度本身没法在这里断言：jsdom 不做布局，getBoundingClientRect() 恒返回
    // 全 0。真实的非零矩形由真机验证（Agent 视图确实铺满了内容区）。
    for (const value of [rect.x, rect.y, rect.width, rect.height]) {
      expect(Number.isFinite(value)).toBe(true);
    }
    expect(typeof rect.width).toBe("number");
    expect(typeof rect.height).toBe("number");
  });

  it("打开服务失败时把真实原因告诉用户，而不是静默吞掉", async () => {
    const api = await mount(
      stubApi({
        openServer: vi.fn(async () => {
          throw new Error(
            "Error invoking remote method 'agent:open': Error: 找不到服务连接：s1",
          );
        }),
      }),
    );

    screen.getByText("工作站").click();

    // Electron 的 IPC 前缀被剥掉，只留真实原因。
    await screen.findByText("打开失败：找不到服务连接：s1");
    expect(api.openServer).toHaveBeenCalled();
  });
});