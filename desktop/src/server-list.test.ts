import { describe, expect, it } from "vitest";
import type { AppView } from "./model";
import { renderServerList } from "./server-list";

function viewFixture(): AppView {
  return {
    revision: 3,
    activeId: "dsh-1",
    settings: { autostart: false, startHidden: true, notifications: true, meshPeers: [] },
    servers: [
      { id: "dsh-1", name: "Work", host: "100.64.0.2", port: 3080, backend: "dsh" },
      { id: "kimi-1", name: "Home", host: "kimi.local", port: 8080, backend: "kimi" },
    ],
    statuses: {
      "dsh-1": {
        connected: true,
        activeCount: 2,
        sessions: [
          { id: "sess-a", title: "重构登录模块", activity: "Bash · npm test", pinned: false, done: false },
          { id: "sess-b", title: "修复 <b> 注入", activity: null, pinned: false, done: false },
        ],
        serverVersion: null,
        lastCheckedAt: null,
        error: null,
      },
      "kimi-1": {
        connected: false,
        activeCount: 0,
        sessions: [],
        serverVersion: null,
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

  it("surfaces escaped error details as a tooltip on the offline status", () => {
    const view = viewFixture();
    view.statuses["kimi-1"].error = 'dial "tcp" <b> timeout';
    const html = renderServerList(view);
    expect(html).toContain("连接异常");
    expect(html).toContain('title="dial &quot;tcp&quot; &lt;b&gt; timeout"');
    expect(html).not.toContain("<b> timeout");
  });

  it("omits the status tooltip when there is no error", () => {
    const view = viewFixture();
    view.statuses["kimi-1"].error = null;
    const html = renderServerList(view);
    expect(html).toContain("server-status--online");
    expect(html).not.toContain("server-status\" title=");
    expect(html).not.toContain("secret-token");
  });

  it("escapes server fields", () => {
    const view = viewFixture();
    view.servers[0].name = '<img src=x onerror="boom">';
    const html = renderServerList(view);
    expect(html).not.toContain("<img");
    expect(html).toContain("&lt;img");
  });

  it("makes the status row a sessions toggle when busy, collapsed by default", () => {
    const html = renderServerList(viewFixture());
    expect(html).toContain('data-action="toggle-sessions"');
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

  it("shows escaped activity text for expanded busy sessions", () => {
    const view = viewFixture();
    view.statuses["dsh-1"].sessions[0].activity = 'Bash · rm -rf "x"';
    const html = renderServerList(view, new Set(["dsh-1"]));
    expect(html).toContain("session-activity");
    expect(html).toContain("Bash · rm -rf &quot;x&quot;");
    expect(html).not.toContain('rm -rf "x"');
  });

  it("hides sessions panel for offline servers", () => {
    const view = viewFixture();
    view.statuses["kimi-1"] = {
      connected: false,
      activeCount: 1,
      sessions: [{ id: "s", title: "离线会话", activity: null, pinned: false, done: false }],
      serverVersion: null,
      lastCheckedAt: null,
      error: null,
    };
    const html = renderServerList(view, new Set(["kimi-1"]));
    expect(html).not.toContain("离线会话");
  });

  it("shows the kimi server version when reported", () => {
    const view = viewFixture();
    view.statuses["kimi-1"] = {
      connected: true,
      activeCount: 0,
      sessions: [],
      serverVersion: "0.36.0",
      lastCheckedAt: null,
      error: null,
    };
    const html = renderServerList(view);
    expect(html).toContain("v0.36.0");
  });

  it("omits the version segment when the server reports none", () => {
    const html = renderServerList(viewFixture());
    expect(html).not.toContain("server-version");
  });

  it("renders a pin toggle for every session row", () => {
    const html = renderServerList(viewFixture(), new Set(["dsh-1"]));
    expect(html).toContain('data-action="toggle-pin"');
    expect(html).toContain('aria-label="置顶 重构登录模块"');
  });

  it("marks pinned rows as active pins", () => {
    const view = viewFixture();
    view.statuses["dsh-1"].sessions[0].pinned = true;
    const html = renderServerList(view, new Set(["dsh-1"]));
    expect(html).toContain('aria-pressed="true"');
    expect(html).toContain("session-row--pinned");
  });

  it("shows a done mark instead of a spinner for pinned finished sessions", () => {
    const view = viewFixture();
    view.statuses["dsh-1"].sessions[1] = {
      id: "sess-b",
      title: "修复 <b> 注入",
      activity: "已完成，等你介入",
      pinned: true,
      done: true,
    };
    const html = renderServerList(view, new Set(["dsh-1"]));
    expect(html).toContain("session-row--done");
    expect(html).toContain("已完成，等你介入");
    const doneRow = html.split("sess-b")[1]?.split("</li>")[0] ?? "";
    expect(doneRow).not.toContain("session-spinner");
    expect(doneRow).toContain("session-done-mark");
  });
});
