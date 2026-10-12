import { render, screen } from "@testing-library/svelte";
import { describe, expect, it, vi } from "vitest";
import Sidebar from "./Sidebar.svelte";
import type { AppView, ServerStatus, ServerSummary } from "../model";

function server(id: string, name: string, backend: ServerSummary["backend"]): ServerSummary {
  return { id, name, host: "100.64.0.2", port: 4096, backend };
}

function status(overrides: Partial<ServerStatus> = {}): ServerStatus {
  return {
    connected: true,
    activeCount: 0,
    sessions: [],
    serverVersion: null,
    lastCheckedAt: null,
    error: null,
    ...overrides,
  };
}

function renderSidebar(overrides: Record<string, unknown> = {}) {
  const servers = [server("s1", "工作站", "opencode"), server("s2", "宿舍", "kimi")];
  const props = {
    servers,
    statuses: { s1: status(), s2: status({ connected: false }) } as Record<string, ServerStatus>,
    expanded: new Set<string>(),
    selectedId: null,
    selectedSessionId: null,
    onSelectServer: vi.fn(),
    onSelectSession: vi.fn(),
    onToggleExpand: vi.fn(),
    onTogglePin: vi.fn(),
    onEdit: vi.fn(),
    onDelete: vi.fn(),
    onAdd: vi.fn(),
    onToggleCollapsed: vi.fn(),
    collapsed: false,
    ...overrides,
  };
  return { props, ...render(Sidebar, props) };
}

describe("Sidebar", () => {
  it("列出所有服务连接并标注后端", () => {
    renderSidebar();
    expect(screen.getByText("工作站")).toBeInTheDocument();
    expect(screen.getByText("宿舍")).toBeInTheDocument();
    expect(screen.getByLabelText("OpenCode")).toBeInTheDocument();
    expect(screen.getByLabelText("Kimi")).toBeInTheDocument();
  });

  it("区分在线与离线状态", () => {
    renderSidebar();
    expect(screen.getByText("在线")).toBeInTheDocument();
    expect(screen.getByText("离线")).toBeInTheDocument();
  });

  it("有版本号时展示 v 前缀形式", () => {
    renderSidebar({
      statuses: {
        s1: status({ serverVersion: "2.1.1" }),
        s2: status({ connected: false, serverVersion: "2.0.26" }),
      },
    });
    expect(screen.getByText("v2.1.1")).toBeInTheDocument();
    expect(screen.getByText("v2.0.26")).toBeInTheDocument();
  });

  it("版本号缺失时不留空位", () => {
    renderSidebar({ statuses: { s1: status({ serverVersion: null }), s2: status({ connected: false }) } });
    expect(screen.queryByText(/^v/)).not.toBeInTheDocument();
  });

  it("没有服务时给出添加引导", () => {
    renderSidebar({ servers: [], statuses: {} });
    expect(screen.getByText("还没有服务连接")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "添加第一个" })).toBeInTheDocument();
  });

  describe("折叠态", () => {
    it("隐藏服务名与元信息，但保留可点击的导航", () => {
      renderSidebar({ collapsed: true, onToggleCollapsed: vi.fn() });
      // 文字信息全部让位给宽度
      expect(screen.queryByText("工作站")).not.toBeInTheDocument();
      expect(screen.queryByText("在线")).not.toBeInTheDocument();
      // 但仍能通过 aria-label 点开
      expect(screen.getByRole("button", { name: "打开 工作站" })).toBeInTheDocument();
    });

    it("折叠态点击图标仍回调 onSelectServer", () => {
      const { props } = renderSidebar({ collapsed: true, onToggleCollapsed: vi.fn() });
      screen.getByRole("button", { name: "打开 工作站" }).click();
      expect(props.onSelectServer).toHaveBeenCalledWith("s1");
    });

    it("折叠态不显示会话列表与底部操作", () => {
      const withSession = {
        expanded: new Set(["s1"]),
        collapsed: true,
        onToggleCollapsed: vi.fn(),
        statuses: {
          s1: status({ sessions: [{ id: "a", title: "重构登录", activity: null, pinned: false, done: false }] }),
          s2: status({ connected: false }),
        },
      };
      renderSidebar(withSession);
      expect(screen.queryByText("重构登录")).not.toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "编辑" })).not.toBeInTheDocument();
    });

    it("折叠态用角标显示运行中任务数——不能因为折叠把信息丢了", () => {
      renderSidebar({
        collapsed: true,
        onToggleCollapsed: vi.fn(),
        statuses: { s1: status({ activeCount: 3 }), s2: status({ connected: false }) },
      });
      expect(screen.getByLabelText("3 个运行中任务")).toBeInTheDocument();
    });

    it("提供展开按钮并回调", () => {
      const { props } = renderSidebar({ collapsed: true, onToggleCollapsed: vi.fn() });
      screen.getByRole("button", { name: "展开侧边栏" }).click();
      expect(props.onToggleCollapsed).toHaveBeenCalled();
    });

    it("展开态提供折叠按钮", () => {
      const { props } = renderSidebar({ collapsed: false, onToggleCollapsed: vi.fn() });
      screen.getByRole("button", { name: "折叠侧边栏" }).click();
      expect(props.onToggleCollapsed).toHaveBeenCalled();
    });
  });

  it("点击服务名回调 onSelectServer", async () => {
    const { props } = renderSidebar();
    screen.getByText("工作站").click();
    expect(props.onSelectServer).toHaveBeenCalledWith("s1");
  });

  it("展开后显示会话并带上 serverId/sessionId", () => {
    const withSession = status({
      activeCount: 1,
      sessions: [
        {
          id: "ses_1",
          title: "修复登录",
          activity: "Bash · cargo test",
          pinned: false,
          done: false,
        },
      ],
    });
    const { props } = renderSidebar({
      statuses: { s1: withSession, s2: status({ connected: false }) },
      expanded: new Set(["s1"]),
    });
    expect(screen.getByText("修复登录")).toBeInTheDocument();
    expect(screen.getByText("Bash · cargo test")).toBeInTheDocument();

    screen.getByText("修复登录").click();
    expect(props.onSelectSession).toHaveBeenCalledWith("s1", "ses_1");
  });

  it("未展开时不渲染会话", () => {
    const withSession = status({
      sessions: [{ id: "ses_1", title: "隐藏会话", activity: null, pinned: false, done: false }],
    });
    renderSidebar({ statuses: { s1: withSession, s2: status({ connected: false }) } });
    expect(screen.queryByText("隐藏会话")).not.toBeInTheDocument();
  });

  it("离线服务即使有会话也不展开", () => {
    const offline = status({ connected: false, sessions: [] });
    renderSidebar({
      statuses: { s1: offline, s2: offline },
      expanded: new Set(["s1"]),
    });
    expect(screen.queryByRole("button", { name: /展开 .* 的会话/ })).not.toBeInTheDocument();
  });

  it("置顶按钮回传 id 与 sessionId", () => {
    const withSession = status({
      sessions: [{ id: "ses_9", title: "置顶我", activity: null, pinned: true, done: false }],
    });
    const { props } = renderSidebar({
      statuses: { s1: withSession, s2: status({ connected: false }) },
      expanded: new Set(["s1"]),
    });
    screen.getByLabelText("置顶 置顶我").click();
    expect(props.onTogglePin).toHaveBeenCalledWith("s1", "ses_9");
  });

  it("标记当前选中的服务与展开按钮的 aria-expanded", () => {
    const withSession = status({
      sessions: [{ id: "ses_1", title: "任务", activity: null, pinned: false, done: false }],
    });
    renderSidebar({
      statuses: { s1: withSession, s2: status({ connected: false }) },
      expanded: new Set(["s1"]),
      selectedId: "s1",
    });
    expect(screen.getByRole("button", { name: "展开 工作站 的会话" })).toHaveAttribute(
      "aria-expanded",
      "true",
    );
    expect(screen.getByRole("button", { name: "工作站 在线" })).toHaveAttribute(
      "aria-current",
      "true",
    );
  });

  it("选中服务后才显示编辑/删除", () => {
    const { unmount } = renderSidebar();
    expect(screen.queryByRole("button", { name: "编辑" })).not.toBeInTheDocument();
    unmount();

    const { props } = renderSidebar({ selectedId: "s1" });
    screen.getByRole("button", { name: "编辑" }).click();
    expect(props.onEdit).toHaveBeenCalledWith("s1");
  });

  it("添加按钮回调 onAdd", () => {
    const { props } = renderSidebar();
    screen.getByLabelText("添加服务连接").click();
    expect(props.onAdd).toHaveBeenCalled();
  });

  it("展开按钮回调 onToggleExpand", () => {
    const withSession = status({
      sessions: [{ id: "ses_1", title: "任务", activity: null, pinned: false, done: false }],
    });
    const { props } = renderSidebar({ statuses: { s1: withSession, s2: status({ connected: false }) } });
    screen.getByRole("button", { name: "展开 工作站 的会话" }).click();
    expect(props.onToggleExpand).toHaveBeenCalledWith("s1");
  });

  it("忙碌任务数显示在侧边栏", () => {
    const busy = status({ activeCount: 3 });
    renderSidebar({ statuses: { s1: busy, s2: status({ connected: false }) } });
    expect(screen.getByText("3 个任务")).toBeInTheDocument();
  });
});