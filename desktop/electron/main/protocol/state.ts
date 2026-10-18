/**
 *
 * 只关注主 agent：子代理的事件一律忽略，避免它们触发通知、污染忙碌状态。
 */

export type AgentEventKind =
  | "completed"
  | "failed"
  | "approvalRequired"
  | "questionRequired"
  | "pinnedFinished";

export interface AgentEvent {
  serverId: string;
  sessionId: string | null;
  sessionTitle: string | null;
  kind: AgentEventKind;
  eventKey: string;
  body: string | null;
  occurredAt: Date;
}

/** 单个会话的实时活动，来自相位与工具事件。 */
export interface SessionActivity {
  /** 展示文本，如 "Bash · git push" 或 "思考中"。 */
  display: string | null;
  /** 当前相位指向的工具调用。 */
  currentTool: { id: string; name: string } | null;
  /** 工具命令首行预览，按 toolCallId 索引。 */
  toolCommands: Map<string, string>;
  /** 待审批 / 待回答。 */
  pending: string | null;
}

export class ProtocolState {
  readonly titles = new Map<string, string>();
  /** 有效忙碌：服务器报忙碌且（主回合活跃 或 仍有后台任务）。 */
  readonly busy = new Set<string>();
  /** 服务器侧原始忙碌。 */
  readonly rawBusy = new Set<string>();
  /** 主回合已结束的会话；不在集合中默认视为活跃。 */
  readonly mainTurnInactive = new Set<string>();
  readonly bgRunning = new Map<string, number>();
  baselineComplete = false;
  serverVersion: string | null = null;
  readonly activities = new Map<string, SessionActivity>();
  /** 已纳入订阅的会话 id。 */
  readonly subscribed = new Set<string>();
  /**
   * dsh：`origin === "subagent"` 的会话。
   *
   * 子代理的回合事件不通知用户——它们由父会话的结果体现，逐个报会刷屏。
   * 判断放在这里而不是解析层，是因为"要不要订阅"是监控循环的决策。
   */
  readonly subagentSessions = new Set<string>();

  activityFor(sessionId: string): SessionActivity {
    let activity = this.activities.get(sessionId);
    if (!activity) {
      activity = {
        display: null,
        currentTool: null,
        toolCommands: new Map(),
        pending: null,
      };
      this.activities.set(sessionId, activity);
    }
    return activity;
  }

  /** 活动行文本：待审批/待回答 > 等后台 > 相位文本。 */
  activityText(sessionId: string): string | null {
    const activity = this.activities.get(sessionId);
    if (activity?.pending === "approval") return "等待审批";
    if (activity?.pending === "question") return "等待回答";

    if (this.mainTurnInactive.has(sessionId)) {
      const running = this.bgRunning.get(sessionId) ?? 0;
      return running > 0
        ? `主 agent 已完成 · 等 ${running} 个后台任务`
        : "主 agent 已完成";
    }
    return activity?.display ?? null;
  }

  /** 有效忙碌 = 服务器报 busy 且（主回合活跃 或 仍有后台任务）。 */
  applyEffectiveBusy(sessionId: string): void {
    const raw = this.rawBusy.has(sessionId);
    const mainActive = !this.mainTurnInactive.has(sessionId);
    const running = this.bgRunning.get(sessionId) ?? 0;
    if (raw && (mainActive || running > 0)) {
      this.busy.add(sessionId);
    } else {
      this.busy.delete(sessionId);
      this.activities.delete(sessionId);
    }
  }
}