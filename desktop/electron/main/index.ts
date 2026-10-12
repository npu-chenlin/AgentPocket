import { BrowserWindow, app } from "electron";
import { homedir } from "node:os";
import { join } from "node:path";

import * as agentView from "./agent-view";
import { AppState, registerCommands, type AppView } from "./commands";
import { ConfigStore } from "./config-store";
import { Notifier } from "./notification";
import { defaultSettings, type DesktopSettings, type ServerSummary } from "./model";
import { installMenu } from "./menu";
import { TrayController, enforceSingleInstance } from "./tray";

/**
 * 配置目录沿用旧版路径，这样迁移后用户已有的服务连接直接可用，不用重新录入。
 * Electron 默认的 userData 是另一处路径。
 */
function configDir(): string {
  return (
    process.env.AGENTPOCKET_CONFIG_DIR ??
    join(homedir(), ".local", "share", "com.local.agentpocket.desktop")
  );
}

function createWindow(): BrowserWindow {
  const window = new BrowserWindow({
    width: 1100,
    height: 720,
    minWidth: 720,
    minHeight: 480,
    // 默认隐藏，由配置决定何时显示，避免开机弹窗。
    show: false,
    title: "AgentPocket",
    // 菜单栏常驻：导入/导出/手机配对/设置都收在里面了，标题栏只留品牌与窗口操作。
    // 代价是 Linux 上多占约 25px 垂直空间。
    autoHideMenuBar: false,
    webPreferences: {
      preload: join(__dirname, "../preload/index.mjs"),
      contextIsolation: true,
      nodeIntegration: false,
      // preload 需要 require，给渲染进程开 sandbox 会挡住它。
      sandbox: false,
    },
  });

  if (process.env.ELECTRON_RENDERER_URL) {
    void window.loadURL(process.env.ELECTRON_RENDERER_URL);
  } else {
    void window.loadFile(join(__dirname, "../renderer/index.html"));
  }

  window.on("close", (event) => {
    // 点叉只收起到托盘，除非用户明确点了"退出"。
    if (!app.isQuitting) {
      event.preventDefault();
      window.hide();
    }
  });

  // 窗口重新显示时，Agent 视图要跟着回来，否则恢复后是一片空白。
  window.on("show", () => agentView.setOverlaid(false));

  return window;
}

if (!enforceSingleInstance(() => {
  const [existing] = BrowserWindow.getAllWindows();
  existing?.show();
  existing?.focus();
})) {
  app.quit();
} else {
  void app.whenReady().then(async () => {
    app.isQuitting = false;

    const window = createWindow();
    const dir = configDir();
    const store = ConfigStore.open(dir);

    // 通知的时间基准：应用启动时刻。启动瞬间拉回来的服务端基线都是
    // "已经发生过"的事件，拿它们通知用户等于报陈年旧账。
    const notifier = new Notifier(new Date());

    let latestView: AppView | null = null;
    let servers = new Map<string, ServerSummary>();

    const push = (view: AppView): void => {
      latestView = view;
      servers = new Map(view.servers.map((server) => [server.id, server]));
      // 视图是 AppState 汇总好的，原样转发即可；在这里再拼一次会把
      // statuses 和 activeId 抹掉。
      window.webContents.send("app:state", view);
    };

    const state = new AppState(store, push, (event) => {
      window.webContents.send("app:event", event);
      const server = servers.get(event.serverId);
      if (!server) {
        // 服务已被删除，事件无处可指。记一笔而不是编一个名字出来。
        console.error(`收到未知服务 ${event.serverId} 的事件，已忽略：`, event.kind);
        return;
      }
      try {
        notifier.notify(event, server, latestView?.settings.notifications ?? true);
      } catch (error) {
        // 通知失败不该拖垮监控，但要让用户知道为什么没弹通知。
        console.error("发送通知失败：", error);
      }
    });

    const tray = new TrayController(window, {
      onShowWindow: () => {
        window.show();
        window.focus();
      },
      onReconnectAll: () => state.monitor.reconnectAll(),
      onToggleAutostart: async (value) => {
        app.setLoginItemSettings({ openAtLogin: value });
        const config = await store.load();
        const next: DesktopSettings = { ...config.settings, autostart: value };
        await store.save({ ...config, settings: next });
        tray.refresh();
        return next;
      },
      onQuit: () => {
        app.isQuitting = true;
        agentView.closeAll(window);
        state.monitor.shutdown();
        app.quit();
      },
      getSnapshot: () => ({
        settings: latestView?.settings ?? defaultSettings(),
        statuses: latestView?.statuses ?? {},
        serverCount: latestView?.servers.length ?? 0,
      }),
    });

    registerCommands(window, state, tray, dir);
    installMenu(window);
    tray.install();

    const config = await state.start();
    app.setLoginItemSettings({ openAtLogin: config.settings.autostart });
    tray.refresh();

    const hidden = process.argv.includes("--hidden");
    if (!hidden && (app.isPackaged === false || !config.settings.startHidden)) {
      window.show();
    }

    app.on("activate", () => {
      const [existing] = BrowserWindow.getAllWindows();
      if (!existing) createWindow();
      else {
        existing.show();
        existing.focus();
      }
    });
  });

  // 托盘常驻应用不因窗口全关而退出。
  app.on("window-all-closed", () => {});
}

declare global {
  namespace Electron {
    interface App {
      isQuitting: boolean;
    }
  }
}