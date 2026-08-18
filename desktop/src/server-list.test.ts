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
      "dsh-1": { connected: true, activeCount: 2, lastCheckedAt: null, error: null },
      "kimi-1": { connected: false, activeCount: 0, lastCheckedAt: null, error: "secret-token" },
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

  it("shows active badge and running count", () => {
    const html = renderServerList(viewFixture());
    expect(html).toContain("badge--active");
    expect(html).toContain("2 个任务运行中");
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
});
