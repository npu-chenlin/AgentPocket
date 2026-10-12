/**
 * 把菜单模板装到应用上。
 *
 * 模板本身在 menu-template.ts——那里不依赖 Electron 运行时，测试可以直接
 * 断言结构（Menu.buildFromTemplate 在 vitest 里调不到）。
 */

import { Menu, app, shell, type BrowserWindow } from "electron";

import { MENU_ACTION_CHANNEL, menuTemplate } from "./menu-template";

export { MENU_ACTION_CHANNEL, menuTemplate, type MenuAction } from "./menu-template";

export function buildMenu(window: BrowserWindow): Menu {
  return Menu.buildFromTemplate(
    menuTemplate({
      send: (action) => window.webContents.send(MENU_ACTION_CHANNEL, action),
      isMac: process.platform === "darwin",
      appName: app.name,
      openExternal: (url) => void shell.openExternal(url),
    }),
  );
}

/** 安装应用菜单。 */
export function installMenu(window: BrowserWindow): void {
  Menu.setApplicationMenu(buildMenu(window));
}