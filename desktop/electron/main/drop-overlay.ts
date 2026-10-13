import { BrowserWindow, screen } from "electron";

/**
 * 分屏投放指示层：一个透明、穿透鼠标的子窗口。
 *
 * 标签内容是原生 WebContentsView，层级在渲染层之上，DOM 画不出盖在网页上的
 * 半透明遮罩；这里复用悬浮球验证过的透明子窗口方案，把 VS Code 式的
 * 半透明分屏指示画在网页上面，网页本身不用隐藏。
 */
const INDICATOR_HTML = `data:text/html;charset=utf-8,${encodeURIComponent(`<!doctype html>
<html><head><style>
  html, body { margin: 0; background: transparent; overflow: hidden; }
  .zone { box-sizing: border-box; width: 100vw; height: 100vh; border-radius: 6px;
    border: 2px solid rgba(34, 130, 244, .65); background: rgba(34, 130, 244, .14); }
</style></head><body><div class="zone"></div></body></html>`)}`;

export interface DropIndicatorRect { x: number; y: number; width: number; height: number }

let overlay: BrowserWindow | null = null;

export function createDropOverlay(host: BrowserWindow): void {
  const window = new BrowserWindow({ width: 10, height: 10,
    frame: false, thickFrame: false, useContentSize: true, transparent: true,
    parent: host, skipTaskbar: true, resizable: false, movable: false,
    minimizable: false, maximizable: false, fullscreenable: false, focusable: false,
    hasShadow: false, show: false,
    webPreferences: { contextIsolation: true, nodeIntegration: false, sandbox: true },
  });
  overlay = window;
  window.setIgnoreMouseEvents(true);
  void window.loadURL(INDICATOR_HTML).catch(error => console.error("投放指示层加载失败", error));
  host.once("closed", () => { if (!window.isDestroyed()) window.destroy(); });
  window.once("closed", () => { if (overlay === window) overlay = null; });
}

export function setDropIndicator(host: BrowserWindow, rect: DropIndicatorRect | null): void {
  if (!overlay || overlay.isDestroyed() || host.isDestroyed()) return;
  const valid = rect && [rect.x, rect.y, rect.width, rect.height].every(Number.isFinite)
    && rect.width > 0 && rect.height > 0;
  if (!valid) { overlay.hide(); return; }
  // 渲染进程给的是视口 CSS 像素，换算成屏幕物理坐标
  const box = host.getContentBounds();
  const topLeft = screen.dipToScreenPoint({ x: Math.round(box.x + rect.x), y: Math.round(box.y + rect.y) });
  const bottomRight = screen.dipToScreenPoint({ x: Math.round(box.x + rect.x + rect.width), y: Math.round(box.y + rect.y + rect.height) });
  overlay.setBounds({ x: topLeft.x, y: topLeft.y,
    width: Math.max(1, bottomRight.x - topLeft.x), height: Math.max(1, bottomRight.y - topLeft.y) });
  overlay.moveTop();
  overlay.showInactive();
}

export function closeDropOverlay(): void {
  if (overlay && !overlay.isDestroyed()) overlay.destroy();
  overlay = null;
}
