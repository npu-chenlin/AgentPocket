import { describe, expect, it } from "vitest";
import type { AppView } from "./model";
import { renderServerList } from "./server-list";

function viewFixture(): AppView {
  return {
    revision: 3,
    activeId: "dsh-1",
    settings: { autostart: false, startHidden: true, notifications: true },
    servers: [
      { id: "dsh-1", name: "Work", host: "100.64.0.2", port: 3080, backend: "dsh" },
      { id: "kimi-1", name: "Home", host: "kimi.local", port: 8080, backend: "kimi" },
    ],
    statuses: {
      "dsh-1": {
        connected: true,
        activeCount: 2,
        sessions: [
          { id: "sess-a", title: "重构登录模块" },
          { id: "sess-b", title: "修复 <b> 注入" },
        ],
        lastCheckedAt: null,
        error: null,
      },
      "kimi-1": {
        connected: false,
        activeCount: 0,
        sessions: [],
        lastCheckedAt: null,
        error: "secret-token",
      },
    },
  };
}

describe("renderServerList", () => {
  it("chooses backend-specific logos", () => {
    const html = renderServerList(viewFixture());
    expect(html).toContain('data-backend="dsh"');
    expect(html).toContain('data-backend="kimi"');
    expect(html).toContain("backend-logo--dsh");
    expect(html).toContain("backend-logo--kimi");
  });

  it("shows offline state in gray without a green status dot", () => {
    const html = renderServerList(viewFixture());
    expect(html).toContain("server-card--offline");
    expect(html).toContain("backend-logo--offline");
    expect(html).not.toContain("green-dot");
  });

  it("shows running count for online servers", () => {
    const html = renderServerList(viewFixture());
    expect(html).toContain("2 个任务运行中");
  });

  it("does not expose the active selector or badge", () => {
    const html = renderServerList(viewFixture());
    expect(html).not.toContain("badge--active");
    expect(html).not.toContain("设为当前");
    expect(html).not.toContain("data-action=\"activate\"");
  });

  it("never serializes token-like error details", () => {
    const html = renderServerList(viewFixture());
    expect(html).not.toContain("secret-token");
    expect(html).toContain("连接异常");
  });

  it("escapes server fields", () => {
    const view = viewFixture();
    view.servers[0].name = '<img src=x onerror="boom">';
    const html = renderServerList(view);
    expect(html).not.toContain("<img");
    expect(html).toContain("&lt;img");
  });

  it("shows collapsed sessions toggle for online servers with busy sessions", () => {
    const html = renderServerList(viewFixture());
    expect(html).toContain("toggle-sessions");
    expect(html).toContain("2 个运行中会话");
    expect(html).not.toContain("session-row");
    expect(html).not.toContain("重构登录模块");
  });

  it("lists session titles only for expanded servers", () => {
    const html = renderServerList(viewFixture(), new Set(["dsh-1"]));
    expect(html).toContain("重构登录模块");
    expect(html).toContain('data-action="open-session"');
    expect(html).toContain('data-session-id="sess-a"');
  });

  it("escapes session titles", () => {
    const html = renderServerList(viewFixture(), new Set(["dsh-1"]));
    expect(html).not.toContain("<b>");
    expect(html).toContain("&lt;b&gt;");
  });

  it("hides sessions panel for offline servers", () => {
    const view = viewFixture();
    view.statuses["kimi-1"] = {
      connected: false,
      activeCount: 1,
      sessions: [{ id: "s", title: "离线会话" }],
      lastCheckedAt: null,
      error: null,
    };
    const html = renderServerList(view, new Set(["kimi-1"]));
    expect(html).not.toContain("离线会话");
  });
});
