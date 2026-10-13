import { describe, expect, it, vi } from "vitest";
import type { ServerStatus, ServerSummary } from "../../model";
import { BLINK, EXPR_CADENCE, EXPRESSIONS, POOLS } from "./expression-data";
import {
  BLINK_MS,
  BODY_H,
  BODY_W,
  DEPTH_CUTOFF,
  GrokFace,
  blinkScaleAt,
  breathScale,
  projectEye,
  smoothGaze,
  springStep,
  turnRadians,
} from "./face";
import {
  FACE_STATES,
  FaceState,
  baseFaceState,
  faceReactionForEvent,
  faceSignalsFromServers,
} from "./expression-state";

describe("表情数据", () => {
  it("25 个表情，每个 2 只眼 × 48 点", () => {
    expect(EXPRESSIONS).toHaveLength(25);
    for (const expression of EXPRESSIONS) {
      expect(expression).toHaveLength(2);
      for (const ring of expression) {
        expect(ring).toHaveLength(48);
        for (const [x, y] of ring) {
          expect(Number.isFinite(x) && Number.isFinite(y)).toBe(true);
        }
      }
    }
  });

  it("10 个状态都能查到表情池与节奏，池内下标都指向存在的表情", () => {
    expect(FACE_STATES).toHaveLength(10);
    for (const state of FACE_STATES) {
      expect(POOLS[state]?.length).toBeGreaterThan(0);
      expect(EXPR_CADENCE[state]).toHaveLength(2);
      for (const index of POOLS[state]) expect(EXPRESSIONS[index]).toBeDefined();
    }
  });

  it("眨眼节奏只覆盖部分状态，未覆盖的表示不自动眨眼", () => {
    expect(BLINK.idle).toEqual([6000, 14000]);
    expect(BLINK.sleeping).toBeUndefined();
  });
});

describe("弹簧积分", () => {
  it("从 0 收敛到 1", () => {
    let morph = 0;
    let velocity = 0;
    for (let i = 0; i < 120; i++) {
      ({ morph, velocity } = springStep(morph, velocity, 7, 1 / 60));
    }
    expect(morph).toBeCloseTo(1, 3);
    expect(Math.abs(velocity)).toBeLessThan(0.01);
  });

  it("不回弹过冲：峰值不超过 1.02", () => {
    let morph = 0;
    let velocity = 0;
    let peak = 0;
    for (let i = 0; i < 300; i++) {
      ({ morph, velocity } = springStep(morph, velocity, 7, 1 / 60));
      peak = Math.max(peak, morph);
    }
    expect(peak).toBeGreaterThan(0.9);
    expect(peak).toBeLessThan(1.02);
  });

  it("帧间隔取到上限 0.1s 也稳定", () => {
    let morph = 0;
    let velocity = 0;
    for (let i = 0; i < 200; i++) {
      ({ morph, velocity } = springStep(morph, velocity, 7, 0.1));
    }
    expect(Number.isFinite(morph) && Number.isFinite(velocity)).toBe(true);
    expect(morph).toBeCloseTo(1, 6);
    expect(Math.abs(velocity)).toBeLessThan(0.001);
  });

  it("偏离 1 时往回拉，速度带它继续走", () => {
    expect(springStep(0, 0, 7, 1 / 60).velocity).toBeGreaterThan(0);
    expect(springStep(2, 0, 7, 1 / 60).velocity).toBeLessThan(0);
    expect(springStep(1, 0, 7, 1 / 60).velocity).toBe(0);
  });
});

describe("眨眼", () => {
  it("进度为 0 时全开", () => {
    expect(blinkScaleAt(0)).toBe(1);
  });

  it("进行中（t 在 0 到 0.99）时低于全开", () => {
    expect(blinkScaleAt(0.99)).toBeLessThan(1);
  });

  it("中点闭到最小 0.04，只留一条缝", () => {
    expect(blinkScaleAt(0.42)).toBe(0.04);
    expect(blinkScaleAt(0.41)).toBeLessThan(0.06);
  });

  it("后半程对称睁开", () => {
    expect(blinkScaleAt(0.71)).toBeCloseTo(0.5, 6);
    expect(blinkScaleAt(0.9)).toBeGreaterThan(0.8);
    expect(blinkScaleAt(0.9)).toBeLessThan(blinkScaleAt(0.95));
  });

  it("进度 >= 1 视为已经睁开（空闲态）", () => {
    expect(blinkScaleAt(1)).toBe(1);
    expect(blinkScaleAt(2)).toBe(1);
  });
});

describe("转头", () => {
  it("t=0 不偏转", () => {
    expect(turnRadians(0, 85)).toBe(0);
  });

  it("t=0.25 摆到峰值 75% 处", () => {
    const deg = (turnRadians(0.25, 85) * 180) / Math.PI;
    expect(deg).toBeCloseTo(85 * 0.75, 6);
  });

  it("t=0.5 摆回正前方", () => {
    expect(turnRadians(0.5, 85)).toBeCloseTo(0, 12);
  });

  it("t>=1 归零（越界的 t 也夹回来）", () => {
    expect(turnRadians(1, 85)).toBeCloseTo(0, 12);
    expect(turnRadians(1.5, 85)).toBeCloseTo(0, 12);
    expect(turnRadians(-1, 85)).toBeCloseTo(0, 12);
  });

  it("角度为 0 时始终不偏转", () => {
    expect(turnRadians(0.25, 0)).toBe(0);
  });
});

describe("视线平滑与呼吸", () => {
  it("按 dt*10 的比例逼近目标", () => {
    expect(smoothGaze(0, 1, 1 / 60)).toBeCloseTo(1 / 6, 6);
  });

  it("dt 很大时一步到位、不过冲", () => {
    expect(smoothGaze(0, 1, 1)).toBe(1);
    expect(smoothGaze(0, -1, 0.5)).toBe(-1);
  });

  it("呼吸在 4 秒周期内摆动 ±1.2%", () => {
    expect(breathScale(0)).toBeCloseTo(1, 12);
    expect(breathScale(1000)).toBeCloseTo(1.012, 12);
    expect(breathScale(3000)).toBeCloseTo(0.988, 12);
    expect(breathScale(4000)).toBeCloseTo(1, 12);
  });
});

describe("球面投影", () => {
  const eyes = EXPRESSIONS[0];
  const center = (ring: number[][]): [number, number] => {
    let x = 0;
    let y = 0;
    for (const [px, py] of ring) {
      x += px;
      y += py;
    }
    return [x / 48, y / 48];
  };

  it("正前方时深度最大、不缩放", () => {
    const [centerX, centerY] = center(eyes[0]);
    const placed = projectEye({
      centerX,
      centerY,
      turn: 0,
      gazeX: 0,
      gazeY: 0,
      blink: 1,
      baseScale: 1,
    });
    expect(placed.depth).toBeGreaterThan(0.9);
    expect(placed.scaleX).toBeCloseTo(1, 6);
    expect(placed.scaleY).toBeCloseTo(1, 6);
  });

  it("转到背面时深度掉到 0 附近（调用方据此淡出）", () => {
    const [centerX, centerY] = center(eyes[1]);
    const placed = projectEye({
      centerX,
      centerY,
      turn: Math.PI / 2,
      gazeX: 0,
      gazeY: 0,
      blink: 1,
      baseScale: 1,
    });
    expect(placed.depth).toBeLessThan(DEPTH_CUTOFF);
  });

  it("视线按常量比例平移", () => {
    const [centerX, centerY] = center(eyes[0]);
    const placed = projectEye({
      centerX,
      centerY,
      turn: 0,
      gazeX: 1,
      gazeY: 1,
      blink: 1,
      baseScale: 1,
    });
    const base = projectEye({
      centerX,
      centerY,
      turn: 0,
      gazeX: 0,
      gazeY: 0,
      blink: 1,
      baseScale: 1,
    });
    expect(placed.x - base.x).toBeCloseTo(13.2, 9);
    expect(placed.y - base.y).toBeCloseTo(8.4, 9);
  });

  it("眨眼压扁纵向缩放", () => {
    const [centerX, centerY] = center(eyes[0]);
    const placed = projectEye({
      centerX,
      centerY,
      turn: 0,
      gazeX: 0,
      gazeY: 0,
      blink: 0.1,
      baseScale: 1,
    });
    expect(placed.scaleY).toBeCloseTo(0.1, 9);
  });
});

describe("状态映射", () => {
  it("加载中盖过一切", () => {
    expect(baseFaceState({ loading: true, activeCount: 3, allOffline: true })).toBe(
      FaceState.LOADING,
    );
  });

  it("有会话在跑就是 WORKING", () => {
    expect(baseFaceState({ loading: false, activeCount: 1, allOffline: false })).toBe(
      FaceState.WORKING,
    );
    expect(baseFaceState({ loading: false, activeCount: 1, allOffline: true })).toBe(
      FaceState.WORKING,
    );
  });

  it("没任务但全离线就是 DROWSY", () => {
    expect(baseFaceState({ loading: false, activeCount: 0, allOffline: true })).toBe(
      FaceState.DROWSY,
    );
  });

  it("其余是 IDLE", () => {
    expect(baseFaceState({ loading: false, activeCount: 0, allOffline: false })).toBe(
      FaceState.IDLE,
    );
  });

  it("agent 事件映射到表情", () => {
    expect(faceReactionForEvent("complete")).toEqual({ state: FaceState.CELEBRATE, spin: true });
    expect(faceReactionForEvent("approval")).toEqual({ state: FaceState.THINKING, spin: false });
    expect(faceReactionForEvent("question")).toEqual({ state: FaceState.THINKING, spin: false });
    expect(faceReactionForEvent("aborted")).toEqual({ state: FaceState.SAD, spin: false });
    expect(faceReactionForEvent("started")).toBeNull();
    expect(faceReactionForEvent("")).toBeNull();
    expect(faceReactionForEvent(null)).toBeNull();
    expect(faceReactionForEvent(undefined)).toBeNull();
  });

  it("服务状态汇总：忙碌数累加、探测过才谈得上离线", () => {
    const servers: ServerSummary[] = [
      { id: "s1", name: "工作站", host: "100.64.0.2", port: 4096, backend: "opencode" },
      { id: "s2", name: "宿舍", host: "100.64.0.3", port: 4096, backend: "kimi" },
    ];
    const status = (overrides: Partial<ServerStatus>): ServerStatus => ({
      connected: true,
      activeCount: 0,
      sessions: [],
      serverVersion: null,
      lastCheckedAt: null,
      error: null,
      ...overrides,
    });

    expect(
      faceSignalsFromServers({
        loading: false,
        servers,
        statuses: { s1: status({ activeCount: 2 }), s2: status({ activeCount: 1 }) },
      }),
    ).toEqual({ loading: false, activeCount: 3, allOffline: false });

    expect(
      faceSignalsFromServers({
        loading: false,
        servers,
        statuses: { s1: status({ connected: false }), s2: status({ connected: false }) },
      }),
    ).toEqual({ loading: false, activeCount: 0, allOffline: true });

    // 一台都没探测过：不算离线，否则刚启动就会打盹
    expect(
      faceSignalsFromServers({ loading: false, servers, statuses: {} }),
    ).toEqual({ loading: false, activeCount: 0, allOffline: false });

    // 有 status 的服务里只要有一台在线就不算全离线
    expect(
      faceSignalsFromServers({
        loading: false,
        servers: [servers[0]],
        statuses: { s1: status({ connected: true }) },
      }).allOffline,
    ).toBe(false);
  });
});

type StubCtx = ReturnType<typeof stubCtx>;

function stubCtx() {
  const gradient = { addColorStop: vi.fn() };
  return {
    clearRect: vi.fn(),
    save: vi.fn(),
    restore: vi.fn(),
    translate: vi.fn(),
    scale: vi.fn(),
    beginPath: vi.fn(),
    moveTo: vi.fn(),
    lineTo: vi.fn(),
    closePath: vi.fn(),
    ellipse: vi.fn(),
    clip: vi.fn(),
    fill: vi.fn(),
    createLinearGradient: vi.fn(() => gradient),
  };
}

function makeFace(ctx: StubCtx, overrides: { now?: () => number } = {}): GrokFace {
  return new GrokFace(ctx as unknown as CanvasRenderingContext2D, 264, 264, {
    now: overrides.now ?? (() => 0),
    random: () => 0.5,
  });
}

describe("GrokFace 渲染", () => {
  it("默认状态是 IDLE，构造后不自动跑帧循环", () => {
    const face = makeFace(stubCtx());
    expect(face.getFaceState()).toBe(FaceState.IDLE);
    expect(face.isRunning()).toBe(false);
  });

  it("draw 清空透明画布，只画两只眼睛（每只 48 点）", () => {
    const ctx = stubCtx();
    const face = makeFace(ctx);
    face.draw();
    expect(ctx.clearRect).toHaveBeenCalledWith(0, 0, 264, 264);
    expect(ctx.createLinearGradient).not.toHaveBeenCalled();
    expect(ctx.ellipse).toHaveBeenCalledTimes(1);
    expect(ctx.fill).toHaveBeenCalledTimes(2);
    expect(ctx.moveTo).toHaveBeenCalledTimes(2);
    expect(ctx.lineTo).toHaveBeenCalledTimes(2 * 47);
    expect(ctx.save).toHaveBeenCalledTimes(ctx.restore.mock.calls.length);
  });

  it("尺寸为 0 时不画", () => {
    const ctx = stubCtx();
    const face = new GrokFace(ctx as unknown as CanvasRenderingContext2D, 0, 0);
    face.draw();
    expect(ctx.fill).not.toHaveBeenCalled();
  });

  it("step 收敛到目标表情并平滑视线", () => {
    const face = makeFace(stubCtx());
    face.setFaceState(FaceState.WORKING);
    face.setGaze(1, -1);
    for (let i = 0; i < 120; i++) face.step(1 / 60, i * (1000 / 60));

    const snap = face.snapshot();
    expect(snap.expressionIndex).toBe(POOLS.working[0]);
    expect(snap.morph).toBeCloseTo(1, 3);
    expect(snap.gazeX).toBeCloseTo(1, 6);
    expect(snap.gazeY).toBeCloseTo(-1, 6);
  });

  it("视线目标被夹在 [-1, 1]", () => {
    const face = makeFace(stubCtx());
    face.setGaze(5, -5);
    face.step(1, 0);
    expect(face.snapshot().gazeX).toBe(1);
    expect(face.snapshot().gazeY).toBe(-1);
  });

  it("playTurn 在时长内摆出去、结束后归零", () => {
    let now = 1000;
    const face = makeFace(stubCtx(), { now: () => now });
    face.playTurn(85, 1200);

    // t = 0.25：sin(π/2) * 85° * 0.75
    now = 1000 + 300;
    face.step(1 / 60, now);
    expect((face.snapshot().turn * 180) / Math.PI).toBeCloseTo(85 * 0.75, 3);

    now = 1000 + 1200;
    face.step(1 / 60, now);
    expect(face.snapshot().turn).toBe(0);
  });

  it("spinOnce 用 85°/1200ms，wakeTurn 用 28°/700ms", () => {
    let now = 0;
    const face = makeFace(stubCtx(), { now: () => now });
    face.spinOnce();
    now = 300;
    face.step(1 / 60, now);
    expect((face.snapshot().turn * 180) / Math.PI).toBeCloseTo(85 * 0.75, 3);

    face.wakeTurn();
    now = 300 + 175;
    face.step(1 / 60, now);
    expect((face.snapshot().turn * 180) / Math.PI).toBeCloseTo(28 * 0.75, 3);
  });

  it("blinkNow 把眼睛纵向压扁，320ms 后恢复", () => {
    let now = 0;
    const ctx = stubCtx();
    const face = makeFace(ctx, { now: () => now });
    // draw 里最后两次 ctx.scale 就是两只眼睛的 (scaleX, scaleY)
    const eyeScaleY = (): number[] => ctx.scale.mock.calls.slice(-2).map((call) => call[1]);

    face.blinkNow();
    face.draw();
    expect(eyeScaleY()).toEqual([1, 1]);

    now = BLINK_MS * 0.42; // 闭眼中点
    ctx.scale.mockClear();
    face.draw();
    expect(eyeScaleY()).toEqual([0.04, 0.04]);

    now = BLINK_MS + 1; // 眨眼结束
    ctx.scale.mockClear();
    face.draw();
    expect(eyeScaleY()).toEqual([1, 1]);
  });

  it("自动表情节奏到点会换表情", () => {
    let now = 0;
    const face = makeFace(stubCtx(), { now: () => now });
    face.setFaceState(FaceState.CELEBRATE);
    expect(face.snapshot().expressionIndex).toBe(POOLS.celebrate[0]);

    now = 100_000; // 远超 celebrate 的 1400~2600ms 间隔
    face.step(1 / 60, now);
    // random() = 0.5 → 池内 index 1（≠ 当前下标，换得出新表情）
    expect(face.snapshot().expressionIndex).toBe(POOLS.celebrate[1]);
  });

  it("resize 清空新尺寸且不绘制第二层球体", () => {
    const ctx = stubCtx();
    const face = makeFace(ctx);
    face.draw();
    face.resize(512, 512);
    face.draw();
    expect(ctx.clearRect).toHaveBeenLastCalledWith(0, 0, 512, 512);
    expect(ctx.createLinearGradient).not.toHaveBeenCalled();
  });
});

describe("GrokFace 帧循环", () => {
  it("start 跑帧、stop 停下，可重复 start", () => {
    vi.useFakeTimers({ toFake: ["requestAnimationFrame", "cancelAnimationFrame", "setTimeout", "clearTimeout"] });
    try {
      const ctx = stubCtx();
      const face = makeFace(ctx);
      expect(face.isRunning()).toBe(false);

      face.start();
      expect(face.isRunning()).toBe(true);
      vi.advanceTimersByTime(100);
      expect(ctx.fill.mock.calls.length).toBeGreaterThan(0);

      face.stop();
      expect(face.isRunning()).toBe(false);
      const drawn = ctx.fill.mock.calls.length;
      vi.advanceTimersByTime(500);
      expect(ctx.fill.mock.calls.length).toBe(drawn);

      face.start();
      vi.advanceTimersByTime(100);
      expect(ctx.fill.mock.calls.length).toBeGreaterThan(drawn);

      face.dispose();
      expect(face.isRunning()).toBe(false);
    } finally {
      vi.useRealTimers();
    }
  });
});
