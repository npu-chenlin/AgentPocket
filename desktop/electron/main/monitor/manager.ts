/**
 * 监控管理器。从 desktop/src-tauri/src/monitor/mod.rs 迁移。
 *
 * 每个服务一个长驻任务，配置变了就重启任务；断线按指数退避重连。
 * 状态与事件统一走回调推给上层，不在这里吞错误。
 */

import type { ServerConfig } from "../model";
import { ProtocolState, type AgentEvent } from "../protocol/state";
import { runDsh } from "./dsh";
import { runKimi } from "./kimi";
import { runOpencode } from "./opencode";

export type MonitorUpdate =
  | { kind: "status"; serverId: string; status: ServerStatusView }
  | { kind: "event"; event: AgentEvent };

export interface SessionSummary {
  id: string;
  title: string;
  activity: string | null;
  pinned: boolean;
  done: boolean;
}

export interface ServerStatusView {
  connected: boolean;
  activeCount: number;
  sessions: SessionSummary[];
  serverVersion: string | null;
  lastCheckedAt: string | null;
  error: string | null;
}

/** 用户置顶的会话集合，键为 `server_id|session_id`。由命令层写、监控任务读。 */
export class PinnedSessions {
  private readonly keys = new Set<string>();

  static key(serverId: string, sessionId: string): string {
    return `${serverId}|${sessionId}`;
  }

  has(serverId: string, sessionId: string): boolean {
    return this.keys.has(PinnedSessions.key(serverId, sessionId));
  }

  list(): string[] {
    return [...this.keys];
  }

  /** 返回是否发生变化，供调用方决定要不要推状态。 */
  toggle(serverId: string, sessionId: string): boolean {
    const key = PinnedSessions.key(serverId, sessionId);
    if (this.keys.has(key)) {
      this.keys.delete(key);
      return true;
    }
    this.keys.add(key);
    return true;
  }

  removeServer(serverId: string): void {
    const prefix = `${serverId}|`;
    for (const key of [...this.keys]) {
      if (key.startsWith(prefix)) this.keys.delete(key);
    }
  }
}

/** 取消信号：任务用它退出轮询与等待。 */
export class CancelToken {
  private cancelled = false;
  private readonly listeners = new Set<() => void>();

  cancel(): void {
    if (this.cancelled) return;
    this.cancelled = true;
    for (const listener of this.listeners) listener();
    this.listeners.clear();
  }

  get isCancelled(): boolean {
    return this.cancelled;
  }

  /** 订阅取消。已取消时立刻回调，省得调用方自己判一次。 */
  watch(listener: () => void): () => void {
    if (this.cancelled) {
      listener();
      return () => {};
    }
    this.listeners.add(listener);
    return () => this.unwatch(listener);
  }

  unwatch(listener: () => void): void {
    this.listeners.delete(listener);
  }

  /** 可被取消打断的等待。 */
  sleep(ms: number): Promise<void> {
    if (this.cancelled) return Promise.resolve();
    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        this.unwatch(onCancel);
        resolve();
      }, ms);
      const onCancel = () => {
        clearTimeout(timer);
        resolve();
      };
      this.listeners.add(onCancel);
    });
  }
}

/** 任务是被主动取消的，不是出错。上层据此区分"离线"与"正在关闭"。 */
export class MonitorCancelled extends Error {
  constructor() {
    super("cancelled");
    this.name = "MonitorCancelled";
  }
}

/** 断线重连退避：1s → 2s → 4s … 封顶 30s。 */
export class ReconnectBackoff {
  private nextSecs = 1;

  nextDelay(): number {
    const delay = this.nextSecs;
    this.nextSecs = this.nextSecs >= 16 ? 30 : this.nextSecs * 2;
    return delay * 1000;
  }

  reset(): void {
    this.nextSecs = 1;
  }
}

export interface MonitorContext {
  server: ServerConfig;
  state: ProtocolState;
  pinned: PinnedSessions;
  token: CancelToken;
  /** 上报连接状态；会话列表由 `buildStatus` 从协议状态汇总，调用方不必自己拼。 */
  emitStatus: (status: { connected: boolean; error: string | null }) => void;
  emitEvent: (event: AgentEvent) => void;
}

/** 置顶完成跃迁跟踪：prevBusy 跨重连保留，保证"完成"只通知一次。 */
export class PinTracker {
  private readonly prevBusy = new Set<string>();

  constructor(private readonly pinned: PinnedSessions) {}

  emitFinished(context: MonitorContext): void {
    const { server, state, pinned } = context;
    const prefix = `${server.id}|`;
    // 取消置顶的会话移出跟踪，不再补发。
    for (const id of [...this.prevBusy]) {
      if (!pinned.has(server.id, id)) this.prevBusy.delete(id);
    }

    for (const key of pinned.list()) {
      if (!key.startsWith(prefix)) continue;
      const sessionId = key.slice(prefix.length);
      if (state.busy.has(sessionId)) {
        this.prevBusy.add(sessionId);
      } else if (this.prevBusy.delete(sessionId)) {
        context.emitEvent({
          serverId: server.id,
          sessionId,
          sessionTitle: state.titles.get(sessionId) ?? null,
          kind: "pinnedFinished",
          eventKey: `pinned-done-${sessionId}`,
          body: null,
          occurredAt: new Date(),
        });
      }
    }
  }
}

/** 置顶运行中 0 → 置顶已完成 1 → 其余 2。 */
function sessionRank(session: SessionSummary): number {
  if (!session.pinned) return 2;
  return session.done ? 1 : 0;
}

/**
 * 由协议状态汇总出给前端的会话列表与状态。
 *
 * 排序是 置顶运行中 → 置顶已完成 → 其余，同级按标题再按 id，保证顺序稳定，
 * 否则前端会因顺序抖动误判成状态变化而重绘。
 */
export function buildStatus(
  serverId: string,
  connected: boolean,
  state: ProtocolState,
  error: string | null,
  pinned: PinnedSessions,
  now: Date,
): ServerStatusView {
  const titleOf = (id: string): string => {
    const title = state.titles.get(id);
    return title && title.length > 0 ? title : "会话";
  };

  const sessions: SessionSummary[] = [...state.busy].map((id) => ({
    id,
    title: titleOf(id),
    activity: state.activityText(id),
    pinned: pinned.has(serverId, id),
    done: false,
  }));

  // 置顶但已不忙碌的会话仍以完成行留在列表里，提醒用户介入。
  const prefix = `${serverId}|`;
  for (const key of pinned.list()) {
    if (!key.startsWith(prefix)) continue;
    const sessionId = key.slice(prefix.length);
    if (state.busy.has(sessionId)) continue;
    sessions.push({
      id: sessionId,
      title: titleOf(sessionId),
      activity: "已完成，等你介入",
      pinned: true,
      done: true,
    });
  }

  sessions.sort((a, b) => {
    const rank = sessionRank(a) - sessionRank(b);
    if (rank !== 0) return rank;
    if (a.title !== b.title) return a.title < b.title ? -1 : 1;
    return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
  });

  return {
    connected,
    activeCount: state.busy.size,
    sessions,
    serverVersion: state.serverVersion,
    lastCheckedAt: now.toISOString(),
    error,
  };
}

type MonitorRunner = (context: MonitorContext) => Promise<void>;

const RUNNERS: Record<ServerConfig["backend"], MonitorRunner> = {
  kimi: runKimi,
  dsh: runDsh,
  opencode: runOpencode,
};

/** 监控任务向上层汇报的出口。 */
export interface MonitorSink {
  emitStatus: (status: ServerStatusView) => void;
  emitEvent: (event: AgentEvent) => void;
}

/**
 * 跑一个服务的监控循环：连接失败按退避重试，直到被取消。
 *
 * 协议状态由这里持有——会话列表、忙碌、活动行都从它汇总，所以只有这里
 * 才知道该组装出什么样的状态；runner 只负责报"连上了没有/报什么错"。
 */
export async function runSingleServer(
  server: ServerConfig,
  pinned: PinnedSessions,
  token: CancelToken,
  sink: MonitorSink,
): Promise<void> {
  const runner = RUNNERS[server.backend];
  const backoff = new ReconnectBackoff();
  const state = new ProtocolState();

  const report = ({ connected, error }: { connected: boolean; error: string | null }): void => {
    sink.emitStatus(buildStatus(server.id, connected, state, error, pinned, new Date()));
  };

  const context: MonitorContext = { server, state, pinned, token, emitStatus: report, emitEvent: sink.emitEvent };
  const pins = new PinTracker(pinned);

  try {
    while (!token.isCancelled) {
      try {
        await runner(context);
        backoff.reset();
      } catch (error) {
        if (token.isCancelled || error instanceof MonitorCancelled) break;
        report({
          connected: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
      if (token.isCancelled) break;
      await token.sleep(backoff.nextDelay());
    }
  } finally {
    report({ connected: false, error: null });
  }
}

interface MonitorTask {
  fingerprint: string;
  server: ServerConfig;
  token: CancelToken;
}

function fingerprint(server: ServerConfig): string {
  return JSON.stringify(server);
}

/** 管理所有服务的监控任务：配置变化时重启对应任务。 */
export class MonitorManager {
  private readonly tasks = new Map<string, MonitorTask>();

  constructor(
    private readonly pinned: PinnedSessions,
    private readonly onUpdate: (serverId: string, update: MonitorUpdate) => void,
  ) {}

  /** 对齐任务与配置集合：新增的启动、配置变了的重启、多余的停掉。 */
  syncServers(servers: ServerConfig[]): void {
    const seen = new Set<string>();
    for (const server of servers) {
      seen.add(server.id);
      const print = fingerprint(server);
      const existing = this.tasks.get(server.id);
      if (existing && existing.fingerprint === print) continue;
      this.stopOne(server.id);
      this.start(server, print);
    }
    for (const id of [...this.tasks.keys()]) {
      if (!seen.has(id)) this.stopOne(id);
    }
  }

  /** 强制重连全部（设置里的"重新连接所有服务"）。 */
  reconnectAll(): void {
    for (const [id, task] of [...this.tasks]) {
      this.stopOne(id);
      this.start(task.server, task.fingerprint);
    }
  }

  /** 全部停止：进程退出或配置整体替换时调用。 */
  shutdown(): void {
    for (const id of [...this.tasks.keys()]) this.stopOne(id);
  }

  private start(server: ServerConfig, print: string): void {
    const token = new CancelToken();
    this.tasks.set(server.id, { fingerprint: print, server, token });

    void runSingleServer(server, this.pinned, token, {
      emitStatus: (status) =>
        this.onUpdate(server.id, { kind: "status", serverId: server.id, status }),
      emitEvent: (event) => this.onUpdate(server.id, { kind: "event", event }),
    }).catch((error: unknown) => {
      // 任务本身崩了要报出来，不能让服务悄悄从列表里消失。
      this.onUpdate(server.id, {
        kind: "status",
        serverId: server.id,
        status: {
          connected: false,
          activeCount: 0,
          sessions: [],
          serverVersion: null,
          lastCheckedAt: new Date().toISOString(),
          error: `监控任务异常退出：${error instanceof Error ? error.message : String(error)}`,
        },
      });
    });
  }

  private stopOne(id: string): void {
    const task = this.tasks.get(id);
    if (!task) return;
    task.token.cancel();
    this.tasks.delete(id);
  }
}