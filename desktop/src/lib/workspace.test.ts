import { afterEach, describe, expect, it, vi } from "vitest";
import { WorkspaceController, parseWorkspace, LAYOUT_KEY, type WorkspaceBridge } from "./workspace-controller";
const controllers: WorkspaceController[] = [];
function setup(onOpen = vi.fn()) {
  const host = document.createElement("div"); document.body.append(host);
  const bridge: WorkspaceBridge = { syncWorkspace: vi.fn(async () => {}), retryTab: vi.fn(async () => {}),
    setDropIndicator: vi.fn(async () => {}), captureViews: vi.fn(async () => ({})),
    onTabStatus: vi.fn(() => () => {}), onTabFocus: vi.fn(() => () => {}) };
  const errors = vi.fn();
  const controller = new WorkspaceController(host, bridge, vi.fn(), errors, onOpen);
  controller.dock.layout(1000, 600);
  controllers.push(controller);
  const snapshot = () => vi.mocked(bridge.syncWorkspace).mock.calls.at(-1)?.[0];
  return { controller, bridge, errors, host, onOpen, snapshot };
}
function button(root: ParentNode, label: string) {
  return root.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!;
}
function menuItem(label: string) {
  return [...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(item => item.textContent === label)!;
}
afterEach(() => { controllers.splice(0).forEach(c => c.dispose()); });
describe("Dockview 适配器（非原生端到端）", () => {
  it("普通打开复用，独立标签 UUID 不同，会话独立", () => {
    const { controller: c } = setup(); c.restore([{ id: "s1" }]);
    c.open("s1", null, "A"); const first = c.dock.activePanel!.id;
    c.open("s1", null, "A"); expect(c.dock.panels).toHaveLength(1);
    c.open("s1", null, "A", "duplicate"); expect(c.dock.panels).toHaveLength(2);
    expect(c.dock.activePanel!.id).not.toBe(first);
    c.open("s1", "session", "Session"); expect(c.dock.panels).toHaveLength(3);
  });
  it("右下组合由库布局，移动保留 ID，关闭最后一个和零服务清理", () => {
    const { controller: c } = setup(); c.restore([{ id: "s1" }]);
    c.open("s1", null, "A"); c.duplicate("right"); c.duplicate("below");
    expect(c.dock.groups).toHaveLength(3);
    const panel = c.dock.panels[0]; const id = panel.id;
    panel.api.moveTo({ group: c.dock.groups[2], position: "center" });
    expect(c.dock.getPanel(id)).toBe(panel);
    c.prune([]); expect(c.dock.panels).toHaveLength(0);
  });
  it("保存恢复实际 Dockview 序列化及 UUID", async () => {
    const { controller: c, bridge } = setup(); c.restore([{ id: "s1" }]);
    c.open("s1", null, "A"); c.duplicate("right");
    await vi.waitFor(() => expect(bridge.syncWorkspace).toHaveBeenCalled());
    const saved = localStorage.getItem(LAYOUT_KEY)!;
    const parsed = parseWorkspace(saved); expect(parsed.tabs).toHaveLength(2);
    c.dispose(); controllers.splice(controllers.indexOf(c), 1);
    const { controller: next } = setup(); next.restore([{ id: "s1" }]);
    expect(next.dock.panels.map(p => p.id)).toEqual(parsed.tabs.map(t => t.id));
    expect(next.dock.groups).toHaveLength(2);
  });
  it.each(["向右分屏", "向下分屏", "新建独立标签"])("组内操作 %s 使用该组当前标签而非全局焦点", async label => {
    const { controller: c, host, snapshot } = setup();
    c.restore([{ id: "s1" }, { id: "s2" }]);
    c.open("s1", "a", "A");
    const source = c.dock.activePanel!.group;
    c.open("s1", "b", "B", "duplicate");
    const sourcePanel = c.dock.activePanel!;
    c.open("s2", "other", "Other", "right");
    const otherGroup = c.dock.activePanel!.group;
    const sourceElement = host.querySelector(`[data-tab-id="${sourcePanel.id}"]`)!.closest(".dv-groupview")!;
    const addPanel = vi.spyOn(c.dock, "addPanel");
    if (label === "向右分屏") button(sourceElement, label).click();
    else { button(sourceElement, "更多操作").click(); menuItem(label).click(); }
    expect(addPanel.mock.calls.at(-1)?.[0].position).toEqual({ referencePanel: sourcePanel,
      direction: label === "向右分屏" ? "right" : label === "向下分屏" ? "below" : "within" });
    const copy = c.dock.activePanel!;
    expect(copy.id).not.toBe(sourcePanel.id);
    expect(copy.title).toBe("B");
    expect(otherGroup.panels).toHaveLength(1);
    expect(c.dock.groups).toHaveLength(label === "新建独立标签" ? 2 : 3);
    if (label === "新建独立标签") expect(copy.group).toBe(source);
    await vi.waitFor(() => expect(snapshot()?.tabs.find(tab => tab.id === copy.id)).toMatchObject({ serverId: "s1", sessionId: "b" }));
  });
  it.each(["Escape", "outside", "pointercancel", "blur", "resize", "toggle", "remove", "dispose"])("菜单 %s 关闭释放遮挡且保留其他原因", async reason => {
    const { controller: c, host, snapshot } = setup(); c.restore([{ id: "s1" }]); c.open("s1", null, "A");
    c.setOverlay("modal", true); c.setOverlay("interaction", true);
    const more = button(host, "更多操作");
    more.click();
    expect(snapshot()?.overlays).toEqual(["modal", "interaction", "header-menu"]);
    expect(more.getAttribute("aria-expanded")).toBe("true");
    if (reason === "Escape") window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    else if (reason === "outside") document.body.dispatchEvent(new Event("pointerdown", { bubbles: true }));
    else if (reason === "toggle") more.click();
    else if (reason === "remove") c.dock.removePanel(c.dock.activePanel!);
    else if (reason === "dispose") { c.dispose(); controllers.splice(controllers.indexOf(c), 1); }
    else window.dispatchEvent(new Event(reason));
    expect(document.querySelector('[role="menu"]')).toBeNull();
    expect(more.getAttribute("aria-expanded")).toBe("false");
    await vi.waitFor(() => {
      expect(snapshot()?.overlays).not.toContain("header-menu");
      expect(snapshot()?.overlays).toContain("modal");
    });
    if (!["Escape", "pointercancel", "blur"].includes(reason)) expect(snapshot()?.overlays).toContain("interaction");
    if (reason === "Escape") expect(document.activeElement).toBe(more);
  });
  it("切换组菜单只保留一个，服务入口回调和键盘关闭正常", async () => {
    const { controller: c, host, onOpen, snapshot } = setup(); c.restore([{ id: "s1" }]);
    c.open("s1", null, "A"); c.duplicate("right");
    const controls = [...host.querySelectorAll<HTMLButtonElement>('[aria-label="更多操作"]')];
    controls[0].click(); controls[1].click();
    expect(document.querySelectorAll('[role="menu"]')).toHaveLength(1);
    expect(controls[0].getAttribute("aria-expanded")).toBe("false");
    expect(snapshot()?.overlays).toContain("header-menu");
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "End" }));
    expect(document.activeElement).toBe(menuItem("打开服务连接"));
    menuItem("打开服务连接").click();
    expect(onOpen).toHaveBeenCalledOnce();
    await vi.waitFor(() => expect(snapshot()?.overlays).toEqual([]));
    controls[1].dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    expect(document.activeElement).toBe(menuItem("向下分屏"));
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab" }));
    await vi.waitFor(() => expect(snapshot()?.overlays).toEqual([]));
  });
  it("标签栏按钮不触发组拖拽或 interaction 遮挡", () => {
    const { controller: c, host, snapshot } = setup(); c.restore([{ id: "s1" }]); c.open("s1", null, "A");
    const drag = vi.fn(); const subscription = c.dock.onWillDragGroup(drag);
    const more = button(host, "更多操作");
    more.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true })); more.click();
    expect(drag).not.toHaveBeenCalled();
    expect(snapshot()?.overlays).toEqual(["header-menu"]);
    subscription.dispose();
  });
  it("遮挡时抓取快照做虚化背景，解除后恢复占位", async () => {
    const { controller: c, bridge, host } = setup(); c.restore([{ id: "s1" }]); c.open("s1", null, "A");
    const id = c.dock.activePanel!.id;
    vi.mocked(bridge.captureViews).mockResolvedValue({ [id]: "data:image/png;base64,xyz" });
    c.setOverlay("modal", true);
    expect(bridge.captureViews).toHaveBeenCalled();
    await vi.waitFor(() => expect(host.querySelector(".agent-cover")).not.toBeNull());
    expect(host.querySelector(".agent-content p")).toBeNull();
    c.setOverlay("modal", false);
    expect(host.querySelector(".agent-cover")).toBeNull();
    expect(host.querySelector(".agent-content p")).not.toBeNull();
  });
  it("损坏布局回退且通知", () => {
    localStorage.setItem(LAYOUT_KEY, '{"version":99}');
    const { controller: c, errors } = setup(); c.restore([]);
    expect(c.dock.panels).toHaveLength(0); expect(errors).toHaveBeenCalled();
  });
  it.each(["null", "{}", "[]", '{"version":1,"tabs":[]}'])("拒绝损坏布局 %s", raw => {
    expect(() => parseWorkspace(raw)).toThrow();
  });
});
