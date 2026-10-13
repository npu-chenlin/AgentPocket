import type { ServerStatus, ServerSummary } from "../../model";

/**
 * 表情状态，对应 Android 的 GrokFaceState：枚举值本身就是 gist 内部状态名，
 * 同时也是 expression-data 里 POOLS / EXPR_CADENCE / BLINK 三张表的键。
 */
export const FaceState = {
  SLEEPING: "sleeping",
  WAKING: "waking",
  IDLE: "idle",
  WORKING: "working",
  LOADING: "loading",
  CELEBRATE: "celebrate",
  THINKING: "thinking",
  SAD: "sad",
  DROWSY: "drowsy",
  DRAGGING: "dragging",
} as const;

export type FaceState = (typeof FaceState)[keyof typeof FaceState];

export const FACE_STATES: readonly FaceState[] = Object.values(FaceState);

/** 基础表情的输入信号（Android MainActivity.applyBaseFaceState 的三条判据）。 */
export interface FaceSignals {
  /** 页面正在加载（对应进度条可见）。 */
  loading: boolean;
  /** 所有服务上正在运行的会话总数。 */
  activeCount: number;
  /** 已探测过的服务全部离线；一台都没探测过时为 false（还不算"离线"）。 */
  allOffline: boolean;
}

/**
 * 无事件时的底色表情。优先级：加载 > 忙 > 全离线 > 空闲。
 * 对应 Android MainActivity.applyBaseFaceState。
 */
export function baseFaceState({ loading, activeCount, allOffline }: FaceSignals): FaceState {
  if (loading) return FaceState.LOADING;
  if (activeCount > 0) return FaceState.WORKING;
  if (allOffline) return FaceState.DROWSY;
  return FaceState.IDLE;
}

/** agent 事件触发的表情：state 是要切到的表情，spin 表示额外转一圈。 */
export interface FaceReaction {
  state: FaceState;
  /** 整轮跑完值得庆祝，对应 Android 的 faceView.spinOnce()。 */
  spin: boolean;
}

/**
 * 从服务状态汇总出基础表情信号。
 *
 * 只统计有 status 的服务：没有 status 的就是还没探测过，不能算"离线"，
 * 否则刚启动、一台都还没连上时会误判成 DROWSY。
 */
export function faceSignalsFromServers(input: {
  loading: boolean;
  servers: ServerSummary[];
  statuses: Record<string, ServerStatus>;
}): FaceSignals {
  let activeCount = 0;
  let checked = 0;
  let online = 0;
  for (const server of input.servers) {
    const status = input.statuses[server.id];
    if (!status) continue;
    checked += 1;
    activeCount += status.activeCount;
    if (status.connected) online += 1;
  }
  return { loading: input.loading, activeCount, allOffline: checked > 0 && online === 0 };
}

/**
 * agent 事件 → 表情。对应 Android MainActivity 的 last_event 监听。
 * 返回 null 表示这个事件不影响表情；调用方负责在 3 秒后回落到 baseFaceState。
 */
export function faceReactionForEvent(event: string | null | undefined): FaceReaction | null {
  switch (event) {
    case "complete":
      return { state: FaceState.CELEBRATE, spin: true };
    case "approval":
    case "question":
      return { state: FaceState.THINKING, spin: false };
    case "aborted":
      return { state: FaceState.SAD, spin: false };
    default:
      return null;
  }
}
