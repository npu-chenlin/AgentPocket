/**
 * 托盘。从 desktop/src-tauri/src/tray.rs 迁移。
 *
 * 托盘是这类应用的常驻入口：管理窗口、重连、开机启动、退出都在这里。
 * 菜单保持短——每个服务的条目放管理窗口，托盘只留全局动作。
 */

import { BrowserWindow, Menu, Tray, app, nativeImage } from "electron";
import { join } from "node:path";

import type { ServerStatusView } from "./monitor/manager";
import type { DesktopSettings } from "./model";

/** 汇总在线数与运行中任务数，tooltip 和菜单文案都用它。 */
export function traySummary(statuses: Record<string, ServerStatusView>): {
  connected: number;
  running: number;
} {
  let connected = 0;
  let running = 0;
  for (const status of Object.values(statuses)) {
    if (status.connected) connected += 1;
    running += status.activeCount;
  }
  return { connected, running };
}

export function tooltipText(
  statuses: Record<string, ServerStatusView>,
  serverCount: number,
): string {
  const { connected, running } = traySummary(statuses);
  return `已连接 ${connected}/${serverCount} 台，运行 ${running} 个任务`;
}

/** 有任务在跑时文案要带上数量，否则用户不知道该不该点。 */
export function reconnectLabel(running: number): string {
  return running === 0 ? "重连全部" : `重连全部（${running} 个运行中）`;
}

export interface TrayActions {
  onShowWindow: () => void;
  onReconnectAll: () => void;
  onToggleAutostart: (value: boolean) => Promise<DesktopSettings>;
  onQuit: () => void;
  /** 当前状态快照，用于生成 tooltip。 */
  getSnapshot: () => {
    settings: DesktopSettings;
    statuses: Record<string, ServerStatusView>;
    serverCount: number;
  };
}

export class TrayController {
  private tray: Tray | null = null;

  constructor(
    private readonly window: BrowserWindow,
    private readonly actions: TrayActions,
  ) {}

  install(): void {
    // macOS 菜单栏要用纯黑模板图，系统才能按主题反色；
    // 其余平台用带底色的彩色图，浅色面板下才有对比度。
    const file =
      process.platform === "darwin" ? "trayTemplate.png" : "tray.png";
    const image = nativeImage.createFromPath(join(__dirname, "../../resources", file));
    this.tray = new Tray(image.isEmpty() ? nativeImage.createEmpty() : image);
    this.tray.on("click", () => this.actions.onShowWindow());
    this.refresh();
  }

  /** 状态或设置变化后重建 tooltip 与菜单。 */
  refresh(): void {
    if (!this.tray) return;
    const { settings, statuses, serverCount } = this.actions.getSnapshot();
    const { running } = traySummary(statuses);

    this.tray.setToolTip(tooltipText(statuses, serverCount));
    this.tray.setContextMenu(
      Menu.buildFromTemplate([
        { label: "管理窗口", click: () => this.actions.onShowWindow() },
        { label: reconnectLabel(running), click: () => this.actions.onReconnectAll() },
        { type: "separator" },
        // 不用 checkbox：原生菜单会为勾选框预留整列空白，普通项更紧凑
        {
          label: settings.autostart ? "开机启动：开" : "开机启动：关",
          click: () => {
            void this.actions.onToggleAutostart(!settings.autostart);
          },
        },
        { type: "separator" },
        { label: "退出", click: () => this.actions.onQuit() },
      ]),
    );
  }

  destroy(): void {
    this.tray?.destroy();
    this.tray = null;
  }
}

/** 应用级单实例：第二次启动只把已有窗口叫到前面。 */
export function enforceSingleInstance(onSecondLaunch: () => void): boolean {
  if (!app.requestSingleInstanceLock()) return false;
  app.on("second-instance", onSecondLaunch);
  return true;
}