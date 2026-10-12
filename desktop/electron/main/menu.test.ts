/**
 * 菜单模板结构。
 *
 * 标题栏删掉后，原生菜单成了导入/导出/手机配对/设置/置顶的唯一入口。少配一项
 * 就是功能凭空消失，而界面上看不出任何异样——这里逐项断言。
 *
 * 另外，「设置」曾经漏配过：动作类型里有、渲染进程也处理了，却没有任何菜单项
 * 发得出来。所以下面不按标签文字找，而是遍历所有点击回调实际发出的动作名。
 */

import { describe, expect, it } from "vitest";

import { menuTemplate, type MenuAction } from "./menu-template";

interface Probe {
  /** 菜单里所有点击回调实际发出的动作名。 */
  sent: MenuAction[];
  opened: string[];
  template: ReturnType<typeof menuTemplate>;
}

function probe(isMac = false): Probe {
  const sent: MenuAction[] = [];
  const opened: string[] = [];
  const template = menuTemplate({
    send: (action) => sent.push(action),
    isMac,
    appName: "AgentPocket",
    openExternal: (url) => opened.push(url),
  });
  return { sent, opened, template };
}

/** 点开模板里每一项的 click，收集发出的动作。 */
function fireAll(template: ReturnType<typeof menuTemplate>, into: MenuAction[]): void {
  for (const item of template) {
    for (const child of (item.submenu ?? []) as { click?: () => void }[]) {
      child.click?.();
    }
  }
  expect(into.length).toBeGreaterThan(0);
}

/** 展开成一行行的标签，方便断言。 */
function labels(template: ReturnType<typeof menuTemplate>): string[] {
  const out: string[] = [];
  for (const item of template) {
    out.push(String(item.label ?? ""));
    for (const child of (item.submenu ?? []) as { label?: string }[]) {
      if (child.label) out.push(`  ${child.label}`);
    }
  }
  return out;
}

describe("原生菜单栏模板", () => {
  it("每个动作都有菜单项能发得出来", () => {
    const { sent, template } = probe();
    fireAll(template, sent);

    // 这五个是渲染进程接的动作。少任何一个，界面上就少一个入口且无任何提示。
    const expected: MenuAction[] = [
      "import",
      "export",
      "phone-pair",
      "settings",
      "toggle-sidebar",
      "toggle-always-on-top",
    ];
    for (const action of expected) {
      expect(sent, `菜单里没有能发出 "${action}" 的项`).toContain(action);
    }
  });

  it("导入、导出、手机配对、设置都在「文件」下", () => {
    const { template } = probe();
    const file = template.find((m) => m.label === "文件");
    const fileLabels = (file?.submenu ?? []).map((c) => (c as { label?: string }).label ?? "");
    expect(fileLabels).toContain("导入服务连接…");
    expect(fileLabels).toContain("导出服务连接…");
    expect(fileLabels).toContain("手机配对…");
    expect(fileLabels).toContain("设置");
  });

  it("侧边栏折叠与窗口置顶在「视图」下", () => {
    const { template } = probe();
    const view = template.find((m) => m.label === "视图");
    const viewLabels = (view?.submenu ?? []).map((c) => (c as { label?: string }).label ?? "");
    expect(viewLabels).toContain("折叠/展开侧边栏");
    expect(viewLabels).toContain("窗口置顶");
  });

  it("没有配置任何快捷键", () => {
    // 用户明确要求停用快捷键：菜单项与系统默认编辑快捷键并存时，
    // 焦点落在输入框里也会抢键。逐项断言，防止以后顺手加回来。
    const { template } = probe();
    const accelerators: unknown[] = [];
    for (const item of template) {
      if (item.accelerator) accelerators.push(item.accelerator);
      for (const child of (item.submenu ?? []) as { accelerator?: string }[]) {
        if (child.accelerator) accelerators.push(child.accelerator);
      }
    }
    expect(accelerators).toEqual([]);
  });

  it("项目主页走外部浏览器，不在应用内开新窗", () => {
    const { opened, template } = probe();
    const help = template.find((m) => m.label === "帮助");
    const item = (help?.submenu ?? [])[0] as { label?: string; click?: () => void };
    expect(item.label).toBe("项目主页");
    item.click?.();
    expect(opened).toEqual(["https://github.com/npu-chenlin/AgentPocket"]);
  });

  it("非 macOS 不显示应用菜单，退出放在「文件」下", () => {
    const { template } = probe(false);
    expect(labels(template)).not.toContain("AgentPocket");
    const file = template.find((m) => m.label === "文件");
    const roles = (file?.submenu ?? []).map((c) => (c as { role?: string }).role ?? "");
    expect(roles).toContain("quit");
  });

  it("macOS 有应用菜单，最小化在「窗口」下", () => {
    const { template } = probe(true);
    expect(template[0]?.label).toBe("AgentPocket");
    const win = template.find((m) => m.label === "窗口");
    const roles = (win?.submenu ?? []).map((c) => (c as { role?: string }).role ?? "");
    expect(roles).toContain("minimize");
  });

  it("顶层菜单顺序：文件、编辑、视图、窗口、帮助", () => {
    const { template } = probe();
    const top = template
      .map((m) => String(m.label ?? ""))
      // macOS 的应用菜单没有 label 语义的差异，这里只看中文那五个。
      .filter((l) => l !== "AgentPocket");
    expect(top).toEqual(["文件", "编辑", "视图", "窗口", "帮助"]);
  });
});