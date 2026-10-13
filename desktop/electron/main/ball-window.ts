import { BrowserWindow, screen, app } from "electron";
import { join } from "node:path";
import { readFileSync, writeFileSync, mkdirSync, renameSync } from "node:fs";
import type { BallPlacement, BallState, BallSnapshot, BallDrag } from "./ball-ipc";

const SIZE = 72;
let ballWindow: BrowserWindow | null = null;
let mainWindow: BrowserWindow | null = null;
let placement: BallPlacement = { side: "right", offsetY: 120 };
let snapshot: BallSnapshot = { revision: 0, face: "idle", open: false, spin: 0 };
let drag: { offset: { x: number; y: number }; original: { x: number; y: number } } | null = null;
let ready = false;

function position(): { x: number; y: number } {
  const box = mainWindow!.getBounds();
  return clamp({ x: placement.side === "right" ? box.x + box.width - SIZE : box.x, y: box.y + placement.offsetY });
}
function clamp(point: { x: number; y: number }): { x: number; y: number } {
  const work = screen.getDisplayNearestPoint(point).workArea;
  return { x: Math.round(Math.min(Math.max(point.x, work.x), work.x + work.width - SIZE)),
    y: Math.round(Math.min(Math.max(point.y, work.y), work.y + work.height - SIZE)) };
}
function follow(): void {
  if (!mainWindow || !ballWindow || ballWindow.isDestroyed() || drag) return;
  const at = position();
  ballWindow.setBounds({ ...at, width: SIZE, height: SIZE });
}
function visibility(): void {
  if (!ballWindow || ballWindow.isDestroyed()) return;
  if (ready && mainWindow?.isVisible() && !mainWindow.isMinimized()) { follow(); ballWindow.showInactive(); }
  else { dragBall("cancel"); ballWindow.hide(); }
}
function uiPath(): string { return join(app.getPath("userData"), "ui-state.json"); }

export function createBallWindow(host: BrowserWindow): BrowserWindow {
  mainWindow = host;
  try {
    const saved = JSON.parse(readFileSync(uiPath(), "utf8"));
    if (["left", "right"].includes(saved.ball?.side) && Number.isFinite(saved.ball?.offsetY)) placement = saved.ball;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") console.error("悬浮球位置恢复失败", error);
  }
  const at = position();
  const window = new BrowserWindow({ width: SIZE, height: SIZE, ...at,
    frame: false, thickFrame: false, useContentSize: true, transparent: true,
    skipTaskbar: true, resizable: false, movable: false,
    minimizable: false, maximizable: false, fullscreenable: false, focusable: false,
    parent: host, hasShadow: false, show: false,
    webPreferences: { preload: join(__dirname, "../preload/ball.mjs"),
      contextIsolation: true, nodeIntegration: false, sandbox: false },
  });
  ballWindow = window;
  ready = false;
  window.webContents.on("did-finish-load", () => { ready = true; visibility(); });
  const listeners: [string, () => void][] = [
    ["move", follow], ["resize", follow], ["show", visibility], ["hide", visibility],
    ["minimize", visibility], ["restore", visibility],
  ];
  listeners.forEach(([event, callback]) => host.on(event as "show", callback));
  host.once("closed", () => { if (!window.isDestroyed()) window.destroy(); });
  window.once("closed", () => {
    listeners.forEach(([event, callback]) => host.removeListener(event as "show", callback));
    screen.removeListener("display-metrics-changed", follow);
    if (ballWindow === window) ballWindow = null;
  });
  screen.on("display-metrics-changed", follow);
  const loading = process.env.ELECTRON_RENDERER_URL
    ? window.loadURL(new URL("ball.html", process.env.ELECTRON_RENDERER_URL + "/").toString())
    : window.loadFile(join(__dirname, "../renderer/ball.html"));
  void loading.catch(error => {
    console.error("悬浮球加载失败", error);
    host.webContents.send("app:event", { "ball-error": String(error) });
  });
  return window;
}

export function dragBall(action: BallDrag): void {
  if (!ballWindow || !mainWindow) return;
  if (action === "start") {
    const cursor = screen.getCursorScreenPoint();
    const [x, y] = ballWindow.getPosition();
    drag = { offset: { x: cursor.x - x, y: cursor.y - y }, original: { x, y } };
    return;
  }
  if (!drag) return;
  if (action === "cancel") {
    ballWindow.setPosition(drag.original.x, drag.original.y);
    drag = null;
    return;
  }
  const cursor = screen.getCursorScreenPoint();
  const at = clamp({ x: cursor.x - drag.offset.x, y: cursor.y - drag.offset.y });
  ballWindow.setBounds({ ...at, width: SIZE, height: SIZE });
  if (action === "end") {
    const box = mainWindow.getBounds();
    placement = { side: at.x + SIZE / 2 < box.x + box.width / 2 ? "left" : "right", offsetY: at.y - box.y };
    drag = null;
    follow();
    mkdirSync(app.getPath("userData"), { recursive: true });
    writeFileSync(uiPath() + ".tmp", JSON.stringify({ version: 1, ball: placement }));
    renameSync(uiPath() + ".tmp", uiPath());
  }
}
export function publishBallState(state: BallState): void {
  if (!state || typeof state.face !== "string" || typeof state.open !== "boolean" || !Number.isSafeInteger(state.spin))
    throw new Error("无效悬浮球状态");
  snapshot = { ...state, revision: snapshot.revision + 1 };
  if (ballWindow && !ballWindow.isDestroyed()) ballWindow.webContents.send("ball:state", snapshot);
}
export function ballSnapshot(): BallSnapshot { return snapshot; }
export function ballWindowRef(): BrowserWindow | null { return ballWindow; }
