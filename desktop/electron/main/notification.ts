/**
 * 系统通知。从 desktop/src-tauri/src/notification.rs 迁移。
 *
 * 通知最容易出的问题是"同一个事件弹很多次"——重连、状态抖动、服务端重复推送
 * 都会让同一事件多次到达。所以这里有一层按事件的去重策略。
 */

import { Notification } from "electron";

import type { Backend } from "./model";
import type { AgentEvent, AgentEventKind } from "./protocol/state";

/** 去重窗口：半小时内同一事件只提醒一次。 */
const DEFAULT_TTL_MS = 30 * 60 * 1000;
/** 缓存上限，防止长时间运行后无限增长。 */
const DEFAULT_CAPACITY = 256;

/** 只有这几类值得打断用户，其余只进列表。 */
export function isNotifiableKind(kind: AgentEventKind): boolean {
  return (
    kind === "completed" ||
    kind === "failed" ||
    kind === "approvalRequired" ||
    kind === "questionRequired" ||
    kind === "pinnedFinished"
  );
}

/** 去重键要带上 session，否则不同会话撞到同一 eventKey 会被误吞。 */
export function dedupKey(event: AgentEvent): string {
  return `${event.serverId}:${event.sessionId ?? ""}:${event.eventKey}`;
}

export interface ServerLike {
  name: string;
  host: string;
  port: number;
  backend: Backend;
}

/** 正文：服务名 + 事件自带的摘要，没有摘要就退回会话标题。 */
export function notificationBody(event: AgentEvent, server: ServerLike): string {
  const body = event.body ?? event.sessionTitle ?? "";
  const trimmed = server.name.trim();
  const serverName = trimmed.length === 0 ? `${server.host}:${server.port}` : trimmed;
  return body.length === 0 ? `[${serverName}]` : `[${serverName}] ${body}`;
}

/** 标题按后端命名，一眼看出是哪个 Agent 叫的。 */
export function notificationTitle(backend: Backend, kind: AgentEventKind): string {
  const prefix =
    backend === "kimi" ? "Kimi Code" : backend === "dsh" ? "DeepSeek Harness" : "OpenCode";
  const action =
    kind === "completed"
      ? "任务完成"
      : kind === "failed"
        ? "任务失败"
        : kind === "approvalRequired"
          ? "等待审批"
          : kind === "questionRequired"
            ? "待回答"
            : "等你介入";
  return `${prefix} · ${action}`;
}

/**
 * 通知去重策略。
 *
 * 三道闸：种类过滤 → 丢弃监控启动前的旧事件 → 时间窗内去重。
 * 第二道不能省：启动瞬间会拿服务端基线，那批事件是"已经发生过的"，
 * 拿它们通知用户等于报了一堆陈年旧账。
 */
export class NotificationPolicy {
  private readonly seen = new Map<string, number>();

  constructor(
    private readonly ttlMs: number = DEFAULT_TTL_MS,
    private readonly capacity: number = DEFAULT_CAPACITY,
  ) {}

  shouldSend(
    event: AgentEvent,
    monitorStartedAt: Date,
    now: Date = new Date(),
  ): boolean {
    if (!isNotifiableKind(event.kind)) return false;
    if (event.occurredAt.getTime() <= monitorStartedAt.getTime()) return false;

    const key = dedupKey(event);
    const lastSeen = this.seen.get(key);
    if (lastSeen !== undefined && now.getTime() - lastSeen < this.ttlMs) return false;

    this.evict(now);
    this.seen.set(key, now.getTime());
    while (this.seen.size > this.capacity) {
      const oldest = this.seen.keys().next();
      if (oldest.done) break;
      this.seen.delete(oldest.value);
    }
    return true;
  }

  /** 先清过期项，让真正的重复不会一直占着位置。 */
  private evict(now: Date): void {
    for (const [key, at] of this.seen) {
      if (now.getTime() - at >= this.ttlMs) this.seen.delete(key);
    }
  }
}

export class Notifier {
  private readonly policy = new NotificationPolicy();

  constructor(private readonly monitorStartedAt: Date = new Date()) {}

  /** 返回是否真的弹了通知，便于调用方在测试里断言。 */
  notify(event: AgentEvent, server: ServerLike, enabled: boolean): boolean {
    if (!enabled) return false;
    if (!this.policy.shouldSend(event, this.monitorStartedAt)) return false;
    if (!Notification.isSupported()) {
      throw new Error("当前系统不支持原生通知");
    }
    new Notification({
      title: notificationTitle(server.backend, event.kind),
      body: notificationBody(event, server),
    }).show();
    return true;
  }
}