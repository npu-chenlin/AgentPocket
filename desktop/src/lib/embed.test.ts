import { render, screen, waitFor } from "@testing-library/svelte";
import { describe, expect, it } from "vitest";
import App from "./App.svelte";
import { stubBridge } from "./test-bridge";

describe("工作区 IPC 组件契约（实际鼠标和原生视图在 e2e）", () => {
  it("会话目标、同服务副本与单一原子布局", async () => {
    const { api } = stubBridge(); render(App);
    await screen.findByText("选一个服务打开");
    screen.getByRole("button", { name: "打开服务连接" }).click();
    (await screen.findByRole("tab", { name: /活跃会话/ })).click();
    (await screen.findByText("重构登录")).click();
    await waitFor(() => expect(api.syncWorkspace.mock.calls.at(-1)?.[0].tabs[0].sessionId).toBe("session-a"));
    screen.getByRole("button", { name: "更多操作" }).click();
    screen.getByRole("menuitem", { name: "新建独立标签" }).click();
    await waitFor(() => expect(api.syncWorkspace.mock.calls.at(-1)?.[0].tabs).toHaveLength(2));
    const snapshot = api.syncWorkspace.mock.calls.at(-1)![0];
    expect(new Set(snapshot.tabs.map((t: any) => t.id)).size).toBe(2);
    expect(Object.keys(snapshot.visible)).toHaveLength(1);
  });
  it("加载错误保留标签并提供重试", async () => {
    const { api, fireStatus } = stubBridge(); render(App);
    await screen.findByText("选一个服务打开");
    screen.getByRole("button", { name: "打开服务连接" }).click();
    (await screen.findByText("工作站")).click();
    await waitFor(() => expect(api.syncWorkspace.mock.calls.at(-1)?.[0].tabs).toHaveLength(1));
    const id = api.syncWorkspace.mock.calls.at(-1)![0].tabs[0].id;
    fireStatus({ tabId: id, state: "error", error: "连接失败" });
    await screen.findByText("加载失败：连接失败");
    screen.getByRole("button", { name: "重试" }).click();
    expect(api.retryTab).toHaveBeenCalledWith(id);
  });
});
