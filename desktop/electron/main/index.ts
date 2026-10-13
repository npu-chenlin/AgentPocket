import { BrowserWindow, app } from "electron";
import { homedir } from "node:os";
import { join } from "node:path";
import { isolatedMode, isolatedConfigDir } from "./runtime";
import * as agentView from "./agent-view";
import { createBallWindow } from "./ball-window";
import { closeDropOverlay, createDropOverlay } from "./drop-overlay";
import { AppState, registerCommands, type AppView } from "./commands";
import { ConfigStore } from "./config-store";
import { Notifier } from "./notification";
import { defaultSettings, type DesktopSettings } from "./model";
import { installMenu } from "./menu";
import { TrayController, enforceSingleInstance } from "./tray";

let mainWindow: BrowserWindow | null = null;
function showMain(): void {
  if (!mainWindow || mainWindow.isDestroyed()) return;
  if (mainWindow.isMinimized()) mainWindow.restore();
  mainWindow.show();
  mainWindow.focus();
}

async function start(): Promise<void> {
  const dir = isolatedConfigDir ?? process.env.AGENTPOCKET_CONFIG_DIR
    ?? join(homedir(), ".local", "share", "com.local.agentpocket.desktop");
  const store = ConfigStore.open(dir);
  const window = new BrowserWindow({ width: 1100, height: 720, minWidth: 720, minHeight: 480,
    show: false, title: "AgentPocket", autoHideMenuBar: false,
    icon: join(__dirname, "../../resources/icon.png"),
    webPreferences: { preload: join(__dirname, "../preload/index.mjs"),
      contextIsolation: true, nodeIntegration: false, sandbox: false },
  });
  mainWindow = window;
  // 隔离模式（e2e/预览）：真实系统鼠标会命中坐标处最上层窗口，桌面上其他置顶窗口
  // （如旧版悬浮球、置顶应用）盖住测试窗口就会吃掉点击，这里用最高层级保证可测。
  if (isolatedMode) window.setAlwaysOnTop(true, "screen-saver");
  window.on("close", event => {
    if (!app.isQuitting) { event.preventDefault(); window.hide(); }
  });
  window.webContents.setWindowOpenHandler(() => ({ action: "deny" }));
  window.webContents.on("will-navigate", event => event.preventDefault());
  const notifier = new Notifier(new Date());
  let latest: AppView | null = null;
  const state = new AppState(store, view => {
    latest = view;
    if (!window.isDestroyed()) window.webContents.send("app:state", view);
  }, event => {
    if (window.isDestroyed()) return;
    window.webContents.send("app:event", event);
    const server = latest?.servers.find(s => s.id === event.serverId);
    if (server && !isolatedMode) notifier.notify(event, server, latest?.settings.notifications ?? true);
  });
  const tray = new TrayController(window, {
    onShowWindow: showMain,
    onReconnectAll: () => state.monitor.reconnectAll(),
    onToggleAutostart: async value => {
      if (!isolatedMode) app.setLoginItemSettings({ openAtLogin: value });
      const config = await store.load();
      const next: DesktopSettings = { ...config.settings, autostart: value };
      await store.save({ ...config, settings: next });
      await state.publish();
      tray.refresh();
      return next;
    },
    onQuit: () => app.quit(),
    getSnapshot: () => ({ settings: latest?.settings ?? defaultSettings(),
      statuses: latest?.statuses ?? {}, serverCount: latest?.servers.length ?? 0 }),
  });
  app.on("before-quit", () => {
    app.isQuitting = true;
    closeDropOverlay();
    agentView.closeAll(window);
    state.monitor.shutdown();
  });
  const config = await state.start();
  registerCommands(window, state, tray, dir);
  installMenu(window);
  if (!isolatedMode) {
    tray.install();
    app.setLoginItemSettings({ openAtLogin: config.settings.autostart });
  }
  if (process.env.ELECTRON_RENDERER_URL && !isolatedMode) await window.loadURL(process.env.ELECTRON_RENDERER_URL);
  else await window.loadFile(join(__dirname, "../renderer/index.html"));
  try { createBallWindow(window); }
  catch (error) {
    console.error("悬浮球初始化失败", error);
    window.webContents.send("app:event", { "ball-error": String(error) });
  }
  try { createDropOverlay(window); }
  catch (error) { console.error("投放指示层初始化失败", error); }
  if (!process.argv.includes("--hidden") && (isolatedMode || !app.isPackaged || !config.settings.startHidden)) showMain();
  app.on("activate", showMain);
}

if (!isolatedMode && !enforceSingleInstance(showMain)) app.quit();
else {
  app.isQuitting = false;
  void app.whenReady().then(start).catch(error => { console.error("应用启动失败", error); app.exit(1); });
  app.on("window-all-closed", () => {});
}
declare global { namespace Electron { interface App { isQuitting: boolean } } }
