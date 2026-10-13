import { test, expect, _electron as electron, type ElectronApplication, type Page } from "@playwright/test";
import { createServer, type Server } from "node:http";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve, join } from "node:path";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
const exec = promisify(execFile);
async function mouse(from: { x: number; y: number }, to?: { x: number; y: number }, isBall = false) {
  const points = await app.evaluate(({ BrowserWindow, screen }, { from, to, isBall }) => {
    const window = BrowserWindow.getAllWindows().find(w => w.webContents.getURL().endsWith(isBall ? "ball.html" : "index.html"))!;
    // 真实系统鼠标会命中坐标处最上层窗口：桌面其他应用盖住测试窗口时点击会落到别处，
    // 每次操作前先把目标窗口抬到最前。
    if (window.isMinimized()) window.restore();
    window.show();
    window.moveTop();
    const box = window.getContentBounds();
    return [from, to ?? from].map(p => screen.dipToScreenPoint({ x: Math.round(box.x + p.x), y: Math.round(box.y + p.y) }));
  }, { from, to, isBall });
  await exec("powershell.exe", ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", resolve("e2e/mouse.ps1"),
    "-FromX", String(points[0].x), "-FromY", String(points[0].y), "-ToX", String(points[1].x), "-ToY", String(points[1].y), ...(to ? ["-Drag"] : [])]);
}
async function nativeClick(locator: import("@playwright/test").Locator, isBall = false) {
  const box = await locator.boundingBox();
  if (!box) throw Error("鼠标目标不可见");
  await mouse({ x: box.x + box.width / 2, y: box.y + box.height / 2 }, undefined, isBall);
}
async function saved() { return main.evaluate(() => JSON.parse(localStorage.getItem("agentpocket.dockview.v1")!)); }
async function groupCount() { return main.locator(".dv-groupview").count(); }
function activeGroup() { return main.locator(".dv-groupview.dv-active-group"); }
async function menuAction(name: string) {
  await nativeClick(activeGroup().getByRole("button", { name: "更多操作", exact: true }));
  await expect(main.getByRole("menu", { name: "标签组操作" })).toBeVisible();
  await expect.poll(async () => (await nativeViews()).some(v => v.visible)).toBe(false);
  await nativeClick(main.getByRole("menuitem", { name, exact: true }));
  await expect(main.getByRole("menu")).toHaveCount(0);
}
async function expectHeaderControls() {
  for (const group of await main.locator(".dv-groupview").all()) {
    const tab = await group.locator(".dv-tab.dv-active-tab").boundingBox();
    const controls = await group.locator(".workspace-header-actions").boundingBox();
    if (!tab || !controls) throw Error("标签栏操作不可见");
    expect(Math.abs(tab.y + tab.height / 2 - controls.y - controls.height / 2)).toBeLessThanOrEqual(1);
    expect(controls.x).toBeGreaterThanOrEqual(tab.x + tab.width);
    await expect(group.locator(".workspace-header-actions button")).toHaveCount(2);
    const split = await group.getByRole("button", { name: "向右分屏", exact: true }).boundingBox();
    expect(split!.height).toBeGreaterThanOrEqual(24);
    expect(split!.height).toBeLessThanOrEqual(28);
  }
}

test.describe.configure({ mode: "serial" });
const errors: string[] = [];
let server: Server;
let port: number;
let loads = 0;
let slow = false;
let fail = false;
let directory: string;
let app: ElectronApplication;
let main: Page;
let ball: Page;
const executablePath = process.env.AGENTPOCKET_E2E_EXE;

async function launch() {
  const env = { ...process.env };
  delete env.ELECTRON_RUN_AS_NODE;
  delete env.AGENTPOCKET_CONFIG_DIR;
  delete env.ELECTRON_RENDERER_URL;
  app = await electron.launch({ executablePath, args: [
    ...(executablePath ? [] : [resolve("out/main/index.js")]), `--agentpocket-isolated=${directory}`,
    ...(process.env.AGENTPOCKET_E2E_SCALE ? [`--force-device-scale-factor=${process.env.AGENTPOCKET_E2E_SCALE}`] : []),
  ], env });
  await expect.poll(() => app.windows().length).toBeGreaterThanOrEqual(2);
  main = app.windows().find(p => p.url().endsWith("index.html"))!;
  ball = app.windows().find(p => p.url().endsWith("ball.html"))!;
  main.on("pageerror", error => errors.push(error.message));
  ball.on("pageerror", error => errors.push(error.message));
  await expect(main.locator(".dock-host")).toBeVisible();
  await expect(main.locator(".workspace-toolbar")).toHaveCount(0);
  await expect(main.getByRole("button", { name: "服务连接 / 悬浮球面板" })).toHaveCount(0);
  await expect(ball.getByRole("button", { name: "打开悬浮球面板" })).toBeVisible();
}
async function nativeViews() {
  return app.evaluate(({ BrowserWindow }) => {
    const window = BrowserWindow.getAllWindows().find(w => w.getTitle() === "AgentPocket")!;
    return window.contentView.children.map((view: any) => ({ id: view.webContents?.id, bounds: view.getBounds(), visible: view.getVisible() }));
  });
}
async function openPanel() {
  await ball.getByRole("button", { name: "打开悬浮球面板" }).click();
  await expect(main.getByRole("dialog", { name: "悬浮球面板" })).toBeVisible();
}
async function openService() {
  await openPanel();
  await main.locator(".launcher__main").filter({ hasText: "Local Agent" }).click();
  await expect.poll(async () => (await nativeViews()).filter(v => v.visible).length).toBe(1);
}

test.beforeAll(async () => {
  directory = await mkdtemp(join(tmpdir(), "agentpocket-e2e-"));
  server = createServer((req, res) => {
    if (req.url?.startsWith("/api")) { res.writeHead(404); res.end(); return; }
    if (req.url?.includes("favicon")) { res.writeHead(204); res.end(); return; }
    if (fail) { req.socket.destroy(); return; }
    const number = ++loads;
    const respond = () => {
      res.setHeader("Content-Type", "text/html; charset=utf-8");
      res.end(`<!doctype html><html><body style="margin:20px;background:#eef4ff;font:18px sans-serif"><h1>Local test Agent</h1><p id="load">Load ${number}</p><input aria-label="Draft" placeholder="Draft"><button onclick="this.textContent=Number(this.textContent)+1">0</button></body></html>`);
    };
    if (slow) setTimeout(respond, 1500); else respond();
  });
  await new Promise<void>(r => server.listen(0, "127.0.0.1", r));
  port = (server.address() as any).port;
  await mkdir(join(directory, "config"));
  await writeFile(join(directory, "config", "config.json"), JSON.stringify({ schema: 1, activeId: "local",
    servers: [{ id: "local", name: "Local Agent", host: "127.0.0.1", port, token: "", backend: "dsh" }],
    settings: { startHidden: false, autostart: false, notifications: false, meshPeers: [] } }));
});
test.afterAll(async () => {
  await app?.close();
  server.closeAllConnections(); await new Promise<void>(r => server.close(() => r()));
  await rm(directory, { recursive: true, force: true });
});

test("真实产品入口、两个 preload、标签与原生网页", async () => {
  await launch();
  expect(await ball.evaluate(() => typeof (window as any).agentpocket)).toBe("undefined");
  expect(await ball.evaluate(() => typeof (window as any).agentpocketBall.getState)).toBe("function");
  await main.getByRole("button", { name: "打开服务连接", exact: true }).click();
  await expect(main.getByRole("dialog", { name: "悬浮球面板" })).toBeVisible();
  await main.locator(".launcher__main").filter({ hasText: "Local Agent" }).click();
  await expect.poll(async () => (await nativeViews()).filter(v => v.visible).length).toBe(1);
  await expect.poll(() => app.context().pages().filter(p => p.url().startsWith(`http://127.0.0.1:${port}`)).length).toBe(1);
  const agent = app.context().pages().find(p => p.url().startsWith(`http://127.0.0.1:${port}`))!;
  await expect(agent.getByRole("heading", { name: "Local test Agent" })).toBeVisible();
  await agent.getByRole("textbox", { name: "Draft" }).fill("first tab survives");
  const initialLoads = loads;
  await menuAction("新建独立标签");
  await expect.poll(async () => (await nativeViews()).length).toBe(2);
  await expect.poll(() => app.context().pages().filter(p => p.url().startsWith(`http://127.0.0.1:${port}`)).length).toBe(2);
  const other = app.context().pages().filter(p => p.url().startsWith(`http://127.0.0.1:${port}`)).find(p => p !== agent)!;
  await expect(other.getByRole("textbox", { name: "Draft" })).toHaveValue("");
  await other.getByRole("textbox", { name: "Draft" }).fill("second independent");
  await main.locator(".dv-tab").first().click();
  await expect(agent.getByRole("textbox", { name: "Draft" })).toHaveValue("first tab survives");
  expect(loads).toBe(initialLoads + 1);
  await openService();
  expect((await nativeViews()).length).toBe(2);
  expect(loads).toBe(initialLoads + 1);
  await expectHeaderControls();
  await nativeClick(activeGroup().getByRole("button", { name: "向右分屏", exact: true }));
  await expect.poll(async () => (await nativeViews()).filter(v => v.visible).length).toBe(2);
  await menuAction("向下分屏");
  await expect.poll(async () => (await nativeViews()).filter(v => v.visible).length).toBe(3);
  await expectHeaderControls();
  const loadsBeforeMenu = loads;
  await nativeClick(activeGroup().getByRole("button", { name: "更多操作", exact: true }));
  await expect.poll(async () => (await nativeViews()).some(v => v.visible)).toBe(false);
  const menuBounds = await main.getByRole("menu", { name: "标签组操作" }).boundingBox();
  const viewport = await main.evaluate(() => ({ width: innerWidth, height: innerHeight }));
  expect(menuBounds!.x).toBeGreaterThanOrEqual(0);
  expect(menuBounds!.y).toBeGreaterThanOrEqual(0);
  expect(menuBounds!.x + menuBounds!.width).toBeLessThanOrEqual(viewport.width);
  expect(menuBounds!.y + menuBounds!.height).toBeLessThanOrEqual(viewport.height);
  await main.screenshot({ path: "test-results/workspace-header-menu.png" });
  await main.keyboard.press("Escape");
  await expect(main.getByRole("menu")).toHaveCount(0);
  await expect.poll(async () => (await nativeViews()).filter(v => v.visible).length).toBe(3);
  await menuAction("打开服务连接");
  await expect(main.getByRole("dialog", { name: "悬浮球面板" })).toBeVisible();
  await expect.poll(async () => (await nativeViews()).some(v => v.visible)).toBe(false);
  await main.getByRole("dialog", { name: "悬浮球面板" }).getByRole("button", { name: "关闭", exact: true }).click();
  await expect.poll(async () => (await nativeViews()).filter(v => v.visible).length).toBe(3);
  expect(loads).toBe(loadsBeforeMenu);
  const idsBeforeDrag = (await saved()).tabs.map((t: any) => t.id).sort();
  const loadsBeforeDrag = loads;
  const source = await main.locator(".dv-tab").nth(1).boundingBox();
  const target = await main.locator(".agent-content").last().boundingBox();
  if (!source || !target) throw Error("拖拽目标不存在");
  await mouse({ x: source.x + 30, y: source.y + 15 }, { x: target.x + target.width - 30, y: target.y + target.height / 2 });
  await expect.poll(groupCount).toBe(4);
  expect((await saved()).tabs.map((t: any) => t.id).sort()).toEqual(idsBeforeDrag);
  expect(loads).toBe(loadsBeforeDrag);
  await expect.poll(async () => (await nativeViews()).filter(v => v.visible).length).toBe(4);
  const sash = main.locator(".dv-grid-view .dv-horizontal > .dv-sash-container > .dv-sash.dv-enabled").first();
  const sashBox = await sash.boundingBox();
  if (!sashBox) throw Error("分隔条不存在");
  const beforeResize = (await nativeViews()).map(v => v.bounds.width);
  await mouse({ x: sashBox.x + sashBox.width / 2, y: sashBox.y + sashBox.height / 4 },
    { x: sashBox.x + 75, y: sashBox.y + sashBox.height / 4 });
  await expect.poll(async () => (await nativeViews()).map(v => v.bounds.width)).not.toEqual(beforeResize);
  const gridBeforeGroupDrag = (await saved()).layout.grid;
  const groupHandle = await main.locator(".dv-groupview").last().locator(".dv-void-container").boundingBox();
  const firstContent = await main.locator(".agent-content").first().boundingBox();
  if (!groupHandle || !firstContent) throw Error("组拖拽目标不存在");
  await mouse({ x: groupHandle.x + groupHandle.width / 2, y: groupHandle.y + 16 },
    { x: firstContent.x + firstContent.width / 2, y: firstContent.y + firstContent.height - 40 });
  await expect.poll(async () => (await saved()).layout.grid).not.toEqual(gridBeforeGroupDrag);
  expect((await saved()).tabs.map((t: any) => t.id).sort()).toEqual(idsBeforeDrag);
  await expect.poll(groupCount).toBe(4);
  expect(loads).toBe(loadsBeforeDrag);
  await nativeClick(ball.getByRole("button", { name: "打开悬浮球面板" }), true);
  await expect(main.getByRole("dialog", { name: "悬浮球面板" })).toBeVisible();
  await expect.poll(async () => (await nativeViews()).some(v => v.visible)).toBe(false);
  await main.getByRole("dialog", { name: "悬浮球面板" }).getByRole("button", { name: "关闭", exact: true }).click();
  await expect.poll(async () => (await nativeViews()).filter(v => v.visible).length).toBe(4);
  const ballBefore = await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().find(w => w.webContents.getURL().endsWith("ball.html"))!.getBounds());
  await mouse({ x: 36, y: 36 }, { x: -600, y: 170 }, true);
  const ballAfter = await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().find(w => w.webContents.getURL().endsWith("ball.html"))!.getBounds());
  expect(ballAfter.x).not.toBe(ballBefore.x);
  expect(ballAfter.y).not.toBe(ballBefore.y);
  expect(await main.getByRole("dialog", { name: "悬浮球面板" }).count()).toBe(0);
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().find(w => w.getTitle() === "AgentPocket")!.hide());
  // 只关心主窗口和悬浮球：投放指示窗按需显示，平时保持隐藏
  await expect.poll(() => app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()
    .filter(w => { const u = w.webContents.getURL(); return u.endsWith("index.html") || u.endsWith("ball.html"); })
    .every(w => !w.isVisible()))).toBe(true);
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().find(w => w.getTitle() === "AgentPocket")!.show());
  await expect.poll(() => app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()
    .filter(w => { const u = w.webContents.getURL(); return u.endsWith("index.html") || u.endsWith("ball.html"); })
    .every(w => w.isVisible()))).toBe(true);
  await agent.screenshot({ path: "test-results/agent.png" });
  await ball.screenshot({ path: "test-results/ball.png", omitBackground: true });
  await main.screenshot({ path: "test-results/workspace.png" });
  expect(errors).toEqual([]);
});

test("关闭释放、慢加载竞态、失败重试和原生焦点", async () => {
  const before = (await nativeViews()).length;
  slow = true;
  await menuAction("新建独立标签");
  await expect.poll(async () => (await nativeViews()).length).toBe(before + 1);
  await main.locator(".dv-active-group .dv-tab.dv-active-tab .dv-default-tab-action").click();
  await expect.poll(async () => (await nativeViews()).length).toBe(before);
  await main.waitForTimeout(1800);
  expect((await nativeViews()).length).toBe(before);
  slow = false;
  fail = true;
  await menuAction("新建独立标签");
  await expect(main.getByRole("button", { name: "重试", exact: true })).toBeVisible();
  fail = false;
  const viewsBeforeRetry = await nativeViews();
  await main.getByRole("button", { name: "重试", exact: true }).click();
  await expect(main.getByRole("button", { name: "重试", exact: true })).toHaveCount(0);
  expect((await nativeViews()).map(v => v.id)).toEqual(viewsBeforeRetry.map(v => v.id));
  const first = (await nativeViews())[0];
  await mouse({ x: first.bounds.x + 35, y: first.bounds.y + 35 });
  const firstTab = (await saved()).tabs[0].id;
  await expect.poll(async () => (await saved()).layout.activeGroup).toBe("1");
  expect((await saved()).layout.panels[firstTab]).toBeTruthy();
  await expect.poll(() => app.context().pages().filter(p => p.url().startsWith(`http://127.0.0.1:${port}`)).length).toBeGreaterThanOrEqual(1);
  const remote = app.context().pages().find(p => p.url().startsWith(`http://127.0.0.1:${port}`))!;
  expect(await remote.evaluate(() => typeof (window as any).agentpocket)).toBe("undefined");
  expect(await remote.evaluate(() => typeof (window as any).require)).toBe("undefined");
  await app.evaluate(({ BrowserWindow }) => {
    const host = BrowserWindow.getAllWindows().find(w => w.getTitle() === "AgentPocket")!;
    (host.contentView.children[0] as any).webContents.forcefullyCrashRenderer();
  });
  await expect(main.getByRole("button", { name: "重试", exact: true })).toBeVisible();
  await main.getByRole("button", { name: "重试", exact: true }).click();
  await expect(main.getByRole("button", { name: "重试", exact: true })).toHaveCount(0);
  await expect.poll(async () => (await nativeViews()).filter(v => v.visible).length).toBe(4);
  expect(errors).toEqual([]);
});

test("工作区和球位置跨重启恢复、模态遮挡及窗口最小化", async () => {
  const before = await saved();
  const position = await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().find(w => w.webContents.getURL().endsWith("ball.html"))!.getBounds());
  await app.close();
  await launch();
  await expect.poll(async () => (await nativeViews()).length).toBe(before.tabs.length);
  await expect.poll(async () => (await nativeViews()).filter(v => v.visible).length).toBe(4);
  expect((await saved()).tabs.map((t: any) => t.id).sort()).toEqual(before.tabs.map((t: any) => t.id).sort());
  const integerLayout = (value: unknown) => JSON.parse(JSON.stringify(value, (_key, item) => typeof item === "number" ? Math.round(item) : item));
  expect(integerLayout((await saved()).layout.grid)).toEqual(integerLayout(before.layout.grid));
  const restored = await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().find(w => w.webContents.getURL().endsWith("ball.html"))!.getBounds());
  expect(restored).toEqual(position);
  const beforeNew = (await saved()).tabs.map((t: any) => t.id);
  await menuAction("新建独立标签");
  await expect.poll(async () => (await saved()).tabs.length).toBe(beforeNew.length + 1);
  expect(new Set((await saved()).tabs.map((t: any) => t.id)).size).toBe(beforeNew.length + 1);
  await openPanel();
  await expect.poll(async () => (await nativeViews()).some(v => v.visible)).toBe(false);
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().find(w => w.getTitle() === "AgentPocket")!.hide());
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().find(w => w.getTitle() === "AgentPocket")!.show());
  expect((await nativeViews()).some(v => v.visible)).toBe(false);
  await main.getByRole("dialog", { name: "悬浮球面板" }).getByRole("button", { name: "关闭", exact: true }).click();
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().find(w => w.getTitle() === "AgentPocket")!.minimize());
  await expect.poll(() => app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().find(w => w.webContents.getURL().endsWith("ball.html"))!.isVisible())).toBe(false);
  await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().find(w => w.getTitle() === "AgentPocket")!.restore());
  await expect.poll(() => app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows().find(w => w.webContents.getURL().endsWith("ball.html"))!.isVisible())).toBe(true);
  const snapshot = await ball.evaluate(() => (window as any).agentpocketBall.getState());
  expect(snapshot.revision).toBeGreaterThan(0);
  expect(typeof snapshot.spin).toBe("number");
  expect(errors).toEqual([]);
});

test("真实 IPC 保存、编辑、置顶、导入替换、删除全部服务", async () => {
  const data = await main.evaluate(async port => {
    const api = (window as any).agentpocket;
    const view = await api.saveServer({ id: "", name: "Second Agent", host: "127.0.0.1", port, token: "", backend: "dsh" });
    const id = view.servers.find((s: any) => s.name === "Second Agent").id;
    const edit = await api.getServerForEdit(id);
    const updated = await api.saveServer({ ...edit, name: "Edited Agent" });
    const settings = await api.updateSettings({ ...updated.settings, notifications: true });
    const pinned = await api.toggleSessionPin(id, "sample");
    const unpinned = await api.toggleSessionPin(id, "sample");
    return { id, view, edit, updated, settings, pinned, unpinned };
  }, port);
  expect(data.id).not.toBe("");
  expect(data.updated.servers.some((s: any) => s.name === "Edited Agent")).toBe(true);
  expect(data.settings.settings.notifications).toBe(true);
  expect(data.pinned.pinned).toBe(true);
  expect(data.unpinned.pinned).toBe(false);
  expect(data.pinned.view.servers).toHaveLength(2);
  await openPanel();
  await main.locator(".launcher__main").filter({ hasText: "Edited Agent" }).click();
  await expect.poll(async () => (await saved()).tabs.some((t: any) => t.serverId === data.id)).toBe(true);
  const imported = await main.evaluate(async port => {
    const api = (window as any).agentpocket;
    const preview = await api.previewImportText(JSON.stringify({ schema: 1, servers: [
      { id: "imported", name: "Imported", host: "127.0.0.1", port, token: "", backend: "dsh" } ] }));
    return { preview, view: await api.applyImport(preview.importId, "replace") };
  }, port);
  expect(imported.preview.validCount).toBe(1);
  expect(imported.view.servers).toHaveLength(1);
  await expect.poll(async () => (await nativeViews()).length).toBe(0);
  await openPanel();
  await main.locator(".launcher__main").filter({ hasText: "Imported" }).click();
  await expect.poll(async () => (await nativeViews()).filter(v => v.visible).length).toBe(1);
  const result = await main.evaluate(() => (window as any).agentpocket.deleteServer("imported"));
  expect(result.servers).toHaveLength(0);
  await expect.poll(async () => (await nativeViews()).length).toBe(0);
  await expect(main.getByText("还没有服务连接", { exact: true })).toBeVisible();
  expect(errors).toEqual([]);
});

test("悬浮球透明眼睛画布、同心及深浅背景截图", async () => {
  await main.evaluate(() => (window as any).agentpocket.publishBallState({ face: "idle", open: true, spin: 0 }));
  await expect(ball.getByRole("button")).toHaveAttribute("aria-expanded", "true");
  const result = await ball.evaluate(() => {
    const canvas = document.querySelector("canvas")!;
    const body = document.querySelector(".ball__body")!;
    const pixels = canvas.getContext("2d")!.getImageData(0, 0, canvas.width, canvas.height).data;
    let opaque = 0;
    let colored = 0;
    for (let i = 0; i < pixels.length; i += 4) {
      if (!pixels[i + 3]) continue;
      opaque++;
      if (pixels[i] !== pixels[i + 1] || pixels[i] !== pixels[i + 2]) colored++;
    }
    return { opaque, colored, cornerAlpha: pixels[3], dpr: devicePixelRatio, width: canvas.width,
      canvas: canvas.getBoundingClientRect().toJSON(), body: body.getBoundingClientRect().toJSON() };
  });
  expect(result.opaque).toBeGreaterThan(0);
  expect(result.colored).toBe(0);
  expect(result.cornerAlpha).toBe(0);
  expect(result.width).toBe(Math.round(56 * result.dpr));
  expect(result.canvas).toEqual(result.body);
  await ball.screenshot({ path: "test-results/ball-transparent.png", omitBackground: true });
  await ball.evaluate(() => { document.body.style.background = "#f5f7fa"; });
  await ball.screenshot({ path: "test-results/ball-light.png" });
  await ball.evaluate(() => { document.body.style.background = "#101827"; });
  await ball.screenshot({ path: "test-results/ball-dark.png" });
  await ball.evaluate(() => { document.body.style.background = "transparent"; });
  expect(errors).toEqual([]);
});
