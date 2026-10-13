/**
 * 应用菜单的模板。
 *
 * 刻意和 Electron 解耦：`Menu.buildFromTemplate` 只在 Electron 运行时里可用，
 * vitest 拿不到，所以结构单独放一个纯函数文件，测试才能直接断言菜单长什么样。
 *
 * 菜单项本身不实现业务：点击后只发一个动作名给渲染进程，由那边调用已有的
 * 函数。逻辑留在渲染进程，避免同一件事在两边各写一遍。
 */

import type { MenuItemConstructorOptions } from "electron";

export type MenuAction =
  | "import"
  | "export"
  | "phone-pair"
  | "settings"
  | "toggle-launcher"
  | "toggle-always-on-top";

/** 渲染进程收到这个 IPC 后执行对应动作。 */
export const MENU_ACTION_CHANNEL = "menu:action";

export interface MenuDeps {
  /** 把动作名发给渲染进程。 */
  send: (action: MenuAction) => void;
  /** 是否 macOS：应用菜单的位置、最小化/关闭的归属都随之不同。 */
  isMac: boolean;
  /** 应用名，macOS 应用菜单的标题。 */
  appName: string;
  /** 打开外部链接。 */
  openExternal: (url: string) => void;
}

const HOMEPAGE = "https://github.com/npu-chenlin/AgentPocket";

/** 构造菜单模板。纯数据加点击回调，不读任何 Electron 状态。 */
export function menuTemplate(deps: MenuDeps): MenuItemConstructorOptions[] {
  const { send, isMac, appName, openExternal } = deps;

  return [
    ...(isMac
      ? ([
          {
            label: appName,
            submenu: [
              { role: "about" },
              { type: "separator" },
              { role: "services" },
              { type: "separator" },
              { role: "hide" },
              { role: "hideOthers" },
              { role: "unhide" },
              { type: "separator" },
              { role: "quit" },
            ],
          },
        ] as MenuItemConstructorOptions[])
      : []),
    {
      label: "文件",
      submenu: [
        { label: "导入服务连接…", click: () => send("import") },
        { label: "导出服务连接…", click: () => send("export") },
        { label: "手机配对…", click: () => send("phone-pair") },
        { type: "separator" },
        { label: "设置", click: () => send("settings") },
        { type: "separator" },
        isMac ? { role: "close", label: "关闭窗口" } : { role: "quit", label: "退出" },
      ],
    },
    {
      label: "视图",
      submenu: [
        { label: "显示/隐藏悬浮球面板", click: () => send("toggle-launcher") },
        { label: "窗口置顶", click: () => send("toggle-always-on-top") },
        { type: "separator" },
        { role: "reload", label: "重新加载" },
        { role: "forceReload", label: "强制重新加载" },
        { role: "toggleDevTools", label: "开发者工具" },
        { type: "separator" },
        { role: "resetZoom", label: "实际大小" },
        { role: "zoomIn", label: "放大" },
        { role: "zoomOut", label: "缩小" },
        { type: "separator" },
        { role: "togglefullscreen", label: "全屏" },
      ],
    },
    {
      label: "窗口",
      submenu: [
        { role: "minimize", label: "最小化" },
        ...(isMac
          ? ([
              { role: "zoom", label: "缩放" },
              { type: "separator" },
              { role: "front", label: "前置全部窗口" },
            ] as MenuItemConstructorOptions[])
          : ([{ role: "close", label: "关闭" }] as MenuItemConstructorOptions[])),
      ],
    },
    {
      label: "帮助",
      submenu: [
        // 系统原生「关于」面板：自动带上 package.json 里的版本号
        { role: "about", label: "关于" },
        { type: "separator" },
        { label: "项目主页", click: () => openExternal(HOMEPAGE) },
      ],
    },
  ];
}