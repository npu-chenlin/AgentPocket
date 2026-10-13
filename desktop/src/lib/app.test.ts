import { render, screen, waitFor } from "@testing-library/svelte";
import { describe, expect, it } from "vitest";
import App from "./App.svelte";
import { stubBridge } from "./test-bridge";

describe("App 组件契约", () => {
  it("独立球不在主 DOM，空布局有可见服务入口", async () => {
    stubBridge(); const { container } = render(App);
    await screen.findByText("选一个服务打开");
    expect(container.querySelector(".ball")).toBeNull();
    expect(container.querySelector(".workspace-toolbar")).toBeNull();
    expect(screen.queryByRole("button", { name: "服务连接 / 悬浮球面板" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "打开服务连接" })).toBeInTheDocument();
  });
  it("面板打开服务生成 UUID 快照而不是按服务去重的视图命令", async () => {
    const { api } = stubBridge(); render(App);
    await screen.findByText("选一个服务打开");
    screen.getByRole("button", { name: "打开服务连接" }).click();
    (await screen.findByText("工作站")).click();
    await waitFor(() => expect(api.syncWorkspace.mock.calls.at(-1)?.[0].tabs).toHaveLength(1));
    const snapshot = api.syncWorkspace.mock.calls.at(-1)![0];
    expect(snapshot.tabs[0].serverId).toBe("s1");
    expect(snapshot.tabs[0].id).not.toBe("s1");
    expect(Object.keys(snapshot.visible)).toEqual([snapshot.tabs[0].id]);
  });
});
