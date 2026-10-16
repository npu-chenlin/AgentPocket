import { createDockview, themeLight, type DockviewApi, type IDockviewPanel, type SerializedDockview } from "dockview";
import { validTab, type WorkspaceSnapshot, type WorkspaceTab, type TabStatus } from "../../shared/workspace";
import { createWorkspaceHeader } from "./workspace-header";

export const LAYOUT_KEY = "agentpocket.dockview.v1";
interface SavedWorkspace { version: 1; tabs: WorkspaceTab[]; layout: SerializedDockview }

export function parseWorkspace(raw: string): SavedWorkspace {
  if (raw.length > 500_000) throw new Error("布局过大");
  const value = JSON.parse(raw) as SavedWorkspace;
  if (value.version !== 1 || !Array.isArray(value.tabs) || value.tabs.length > 100 || !value.tabs.every(validTab)
    || new Set(value.tabs.map(t => t.id)).size !== value.tabs.length) throw new Error("标签数据损坏");
  const layout = value.layout;
  if (!layout?.grid || !layout.panels || layout.popoutGroups?.length || layout.floatingGroups?.length || layout.edgeGroups)
    throw new Error("布局数据损坏");
  const ids = new Set(value.tabs.map(t => t.id));
  if (Object.keys(layout.panels).length !== ids.size) throw new Error("布局标签不匹配");
  for (const [id, panel] of Object.entries(layout.panels)) {
    if (!ids.has(id) || panel.id !== id || panel.contentComponent !== "agent" || panel.tabComponent)
      throw new Error("布局组件不匹配");
    delete panel.params;
  }
  let nodes = 0;
  const seen = new Set<string>();
  function check(node: any, depth: number): void {
    if (!node || depth > 30 || ++nodes > 300 || !Number.isFinite(node.size) || node.size < 0)
      throw new Error("布局结构损坏");
    if (node.type === "branch" && Array.isArray(node.data)) node.data.forEach((n: unknown) => check(n, depth + 1));
    else if (node.type === "leaf" && Array.isArray(node.data?.views)) {
      for (const id of node.data.views) {
        if (!ids.has(id) || seen.has(id)) throw new Error("布局标签重复");
        seen.add(id);
      }
    } else throw new Error("布局节点损坏");
  }
  check(layout.grid.root, 0);
  if (seen.size !== ids.size) throw new Error("布局缺少标签");
  return value;
}

export interface WorkspaceBridge {
  syncWorkspace: (snapshot: WorkspaceSnapshot) => Promise<void>;
  retryTab: (id: string) => Promise<void>;
  setDropIndicator: (rect: { x: number; y: number; width: number; height: number } | null) => Promise<void>;
  onTabStatus: (callback: (status: TabStatus) => void) => () => void;
  onTabFocus: (callback: (id: string) => void) => () => void;
}
export class WorkspaceController {
  readonly dock: DockviewApi;
  private tabs = new Map<string, WorkspaceTab>();
  private elements = new Map<string, HTMLElement>();
  private statuses = new Map<string, TabStatus>();
  private overlays = new Set<string>();
  private revision = 0;
  private frame = 0;
  private ready = false;
  private disposed = false;
  private cleanup: (() => void)[] = [];
  private observer: ResizeObserver;
  private closeHeaderMenu?: () => void;

  constructor(private host: HTMLElement, private bridge: WorkspaceBridge,
    private change: (tabs: WorkspaceTab[], activeId: string | null) => void,
    private error: (message: string) => void, onOpen: () => void = () => {}) {
    this.observer = new ResizeObserver(() => this.schedule());
    this.dock = createDockview(host, {
      theme: themeLight,
      disableFloatingGroups: true,
      dndStrategy: "pointer",
      createRightHeaderActionComponent: group => createWorkspaceHeader(group, {
        duplicate: (mode, panel) => this.duplicate(mode, panel),
        open: onOpen,
        menuChanged: close => {
          if (close) this.closeHeaderMenu?.();
          this.closeHeaderMenu = close;
          this.setOverlay("header-menu", !!close);
        },
      }),
      createComponent: ({ id }) => {
        const element = document.createElement("div");
        element.className = "agent-content";
        element.dataset.tabId = id;
        element.style.cssText = "width:100%;height:100%;display:grid;place-content:center;text-align:center";
        this.elements.set(id, element);
        this.observer.observe(element);
        this.renderStatus(id);
        return { element, init: () => {}, dispose: () => {
          this.observer.unobserve(element);
          if (this.elements.get(id) === element) this.elements.delete(id);
        } };
      },
    });
    const listen = (event: (cb: any) => { dispose(): void }, cb: (...args: any[]) => void) => {
      const subscription = event(cb);
      this.cleanup.push(() => subscription.dispose());
    };
    listen(this.dock.onDidLayoutChange, () => this.schedule());
    listen(this.dock.onDidActivePanelChange, () => this.schedule());
    // 拖拽标签/组时网页全程可见（VS Code 行为：只有标签影子跟随光标）。
    // Dockview 的投放指示是 DOM，会被原生网页盖住，所以把指示区域的位置
    // 实时同步给主进程，由透明指示窗画在网页上面。
    let dragging = false;
    let indicatorFrame = 0;
    let lastIndicator = "";
    const reportIndicator = () => {
      indicatorFrame = 0;
      const zone = dragging ? this.host.querySelector(".dv-drop-target-selection") : null;
      const rect = zone?.getBoundingClientRect();
      const next = rect && rect.width > 0 && rect.height > 0
        ? { x: Math.round(rect.x), y: Math.round(rect.y),
            width: Math.round(rect.width), height: Math.round(rect.height) }
        : null;
      const key = JSON.stringify(next);
      if (key !== lastIndicator) {
        lastIndicator = key;
        void this.bridge.setDropIndicator(next).catch(error => this.error(String(error)));
      }
      if (dragging) indicatorFrame = requestAnimationFrame(reportIndicator);
    };
    const beginDrag = () => {
      if (dragging) return;
      dragging = true;
      indicatorFrame = requestAnimationFrame(reportIndicator);
    };
    listen(this.dock.onWillDragPanel, beginDrag);
    listen(this.dock.onWillDragGroup, beginDrag);
    const finish = () => {
      if (!dragging) return;
      dragging = false;
      cancelAnimationFrame(indicatorFrame);
      if (lastIndicator !== "null") {
        lastIndicator = "null";
        void this.bridge.setDropIndicator(null).catch(error => this.error(String(error)));
      }
    };
    this.cleanup.push(() => { dragging = false; cancelAnimationFrame(indicatorFrame); });
    this.cleanup.push(bridge.onTabFocus(id => this.dock.getPanel(id)?.api.setActive()));
    this.cleanup.push(bridge.onTabStatus(status => {
      this.statuses.set(status.tabId, status);
      this.renderStatus(status.tabId);
    }));
    for (const event of ["pointerup", "pointercancel", "dragend", "drop", "blur"] as const) {
      window.addEventListener(event, finish);
      this.cleanup.push(() => window.removeEventListener(event, finish));
    }
    const key = (event: KeyboardEvent) => { if (event.key === "Escape") finish(); };
    window.addEventListener("keydown", key);
    this.cleanup.push(() => window.removeEventListener("keydown", key));
  }

  restore(servers: { id: string }[]): void {
    if (this.ready) return;
    const raw = localStorage.getItem(LAYOUT_KEY);
    if (raw) {
      try {
        const saved = parseWorkspace(raw);
        this.tabs = new Map(saved.tabs.map(tab => [tab.id, tab]));
        this.dock.fromJSON(saved.layout);
      } catch (error) {
        this.dock.clear();
        this.tabs.clear();
        this.error(`布局恢复失败，已回退空工作区：${String(error)}`);
      }
    }
    this.ready = true;
    this.prune(servers);
    this.schedule();
  }

  prune(servers: { id: string }[]): void {
    if (!this.ready) return;
    const live = new Set(servers.map(s => s.id));
    for (const panel of [...this.dock.panels]) {
      if (!live.has(this.tabs.get(panel.id)?.serverId ?? "")) this.dock.removePanel(panel);
    }
    this.schedule();
  }

  open(serverId: string, sessionId: string | null, title: string, mode: "reuse" | "duplicate" | "right" | "below" = "reuse",
    reference = this.dock.activePanel): void {
    const existing = this.dock.panels.find(p => {
      const tab = this.tabs.get(p.id);
      return tab?.serverId === serverId && tab.sessionId === sessionId;
    });
    if (mode === "reuse" && existing) { existing.api.setActive(); this.schedule(); return; }
    const tab = { id: crypto.randomUUID(), serverId, sessionId, title: title.slice(0, 300) };
    this.tabs.set(tab.id, tab);
    this.dock.addPanel({ id: tab.id, title: tab.title, component: "agent",
      position: reference ? { referencePanel: reference,
        direction: mode === "right" ? "right" : mode === "below" ? "below" : "within" } : undefined });
    this.schedule();
  }

  duplicate(mode: "duplicate" | "right" | "below", panel: IDockviewPanel | undefined = this.dock.activePanel): void {
    const tab = this.tabs.get(panel?.id ?? "");
    if (tab) this.open(tab.serverId, tab.sessionId, tab.title, mode, panel);
  }

  setOverlay(reason: string, active: boolean): void {
    if (active === this.overlays.has(reason)) return;
    if (active) this.overlays.add(reason); else this.overlays.delete(reason);
    // 遮挡只做两件事：把遮罩状态同步给界面（CSS 画一层半透明黑），
    // 以及通过 flush 把 overlays 告诉主进程去隐藏原生视图。
    // 遮罩是同步画出来的，主进程隐藏原生网页后底下立刻就是它，不会露白。
    for (const id of this.elements.keys()) this.renderStatus(id);
    this.flush();
  }

  private renderStatus(id: string): void {
    const element = this.elements.get(id);
    if (!element) return;
    element.replaceChildren();
    // 遮挡期间不填任何东西：遮罩由 CSS 画在窗格之上，底下是黑而非白，
    // 所以不需要截图做背景，也就没有"异步等图"的空窗期。
    if (this.overlays.size) return;
    const status = this.statuses.get(id);
    const text = document.createElement("p");
    text.textContent = status?.state === "error" ? `加载失败：${status.error ?? "页面已停止"}`
      : status?.state === "ready" ? "调整布局中，网页状态已保留" : "正在连接…";
    element.append(text);
    if (status?.state === "error") {
      const button = document.createElement("button");
      button.textContent = "重试";
      button.onclick = () => void this.bridge.retryTab(id).catch(e => this.error(String(e)));
      element.append(button);
    }
  }

  private schedule(): void {
    if (!this.frame && !this.disposed) this.frame = requestAnimationFrame(() => { this.frame = 0; this.flush(); });
  }

  private flush(): void {
    if (!this.ready || this.disposed) return;
    const tabs = this.dock.panels.map(p => this.tabs.get(p.id)!).filter(Boolean);
    const live = new Set(tabs.map(t => t.id));
    for (const id of this.tabs.keys()) if (!live.has(id)) { this.tabs.delete(id); this.statuses.delete(id); }
    const visible: WorkspaceSnapshot["visible"] = {};
    for (const group of this.dock.groups) {
      const id = group.activePanel?.id;
      const element = id && this.elements.get(id);
      if (!id || !element) continue;
      const rect = element.getBoundingClientRect();
      // Native children must leave the sash hit-area accessible to the renderer.
      visible[id] = { x: rect.x + 4, y: rect.y + 4,
        width: Math.max(0, rect.width - 8), height: Math.max(0, rect.height - 8) };
    }
    const activeId = this.dock.activePanel?.id ?? null;
    this.change(tabs, activeId);
    void this.bridge.syncWorkspace({ revision: ++this.revision, tabs, visible, activeId,
      overlays: [...this.overlays] }).catch(e => this.error(String(e)));
    try { localStorage.setItem(LAYOUT_KEY, JSON.stringify({ version: 1, tabs, layout: this.dock.toJSON() })); }
    catch { this.error("无法保存工作区布局"); }
  }

  dispose(): void {
    if (this.closeHeaderMenu) { this.closeHeaderMenu(); this.flush(); }
    this.disposed = true;
    cancelAnimationFrame(this.frame);
    this.cleanup.forEach(fn => fn());
    void this.bridge.setDropIndicator(null).catch(() => {});
    this.observer.disconnect();
    this.dock.dispose();
  }
}
