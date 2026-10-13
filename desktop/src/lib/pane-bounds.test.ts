import { describe, expect, test } from "vitest";
import { clipBounds, validateSnapshot, type WorkspaceSnapshot } from "../../shared/workspace";
const tab = { id: "tab-a", serverId: "s1", sessionId: null, title: "A" };
const snapshot = (): WorkspaceSnapshot => ({ revision: 1, tabs: [tab], visible: {}, activeId: tab.id, overlays: [] });
describe("原子工作区契约", () => {
  test("同服务两个标签独立", () => {
    const value = snapshot();
    value.tabs.push({ ...tab, id: "tab-b" });
    value.visible = { "tab-a": { x: 0, y: 0, width: 100, height: 50 }, "tab-b": { x: 100, y: 0, width: 100, height: 50 } };
    expect(() => validateSnapshot(value)).not.toThrow();
  });
  test.each([NaN, Infinity, -Infinity])("拒绝非有限坐标 %s", x => {
    const value = snapshot(); value.visible[tab.id] = { x, y: 0, width: 1, height: 1 };
    expect(() => validateSnapshot(value)).toThrow();
  });
  test("拒绝重复标签和悬空引用", () => {
    const value = snapshot(); value.tabs.push(tab);
    expect(() => validateSnapshot(value)).toThrow();
    expect(() => validateSnapshot({ ...snapshot(), activeId: "absent" })).toThrow();
  });
  test("按缩放转 DIP、取整并裁剪", () => {
    expect(clipBounds({ x: -5.2, y: 8.3, width: 100.7, height: 90 }, 80, 100, 1.25))
      .toEqual({ x: 0, y: 10, width: 80, height: 90 });
  });
  test("屏幕外矩形变成零尺寸", () => {
    expect(clipBounds({ x: 120, y: 150, width: 50, height: 50 }, 100, 100)).toEqual({ x: 100, y: 100, width: 0, height: 0 });
  });
});
