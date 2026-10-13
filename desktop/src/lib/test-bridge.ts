import { vi } from "vitest";
import type { AppView } from "../model";
export function testView(): AppView {
  return { revision: 1, settings: { startHidden: false, autostart: false, notifications: false, meshPeers: [] },
    servers: [{ id: "s1", name: "工作站", host: "127.0.0.1", port: 4096, backend: "kimi" }], activeId: null,
    statuses: { s1: { connected: true, activeCount: 1, sessions: [{ id: "session-a", title: "重构登录", activity: null, pinned: false, done: false }],
      serverVersion: null, lastCheckedAt: null, error: null } } };
}
export function stubBridge() {
  let menu: (action: string) => void = () => {};
  let status: (state: any) => void = () => {};
  const api = { getAppView: vi.fn(async () => testView()), syncWorkspace: vi.fn(async (_snapshot: any) => {}),
    retryTab: vi.fn(async () => {}), setDropIndicator: vi.fn(async () => {}),
    captureViews: vi.fn(async () => ({})), onTabFocus: vi.fn(() => () => {}),
    onTabStatus: vi.fn((cb: (state: any) => void) => { status = cb; return () => {}; }),
    onStateChanged: vi.fn(() => () => {}), onAgentEvent: vi.fn(() => () => {}),
    onBallToggle: vi.fn(() => () => {}), publishBallState: vi.fn(async () => {}),
    onMenuAction: vi.fn((cb: (action: string) => void) => { menu = cb; return () => {}; }),
    exportConfigText: vi.fn(async () => "{}"), discoverMeshPeers: vi.fn(async () => []),
    setAlwaysOnTop: vi.fn(async () => {}), startSyncServer: vi.fn(async () => { throw Error("测试未开启配对"); }),
    toggleSessionPin: vi.fn(async () => ({ pinned: true, view: testView() })),
  };
  Object.assign(globalThis, { agentpocket: api });
  return { api, fireMenu: (action: string) => menu(action), fireStatus: (state: any) => status(state) };
}
