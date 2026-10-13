import { BLINK, EXPR_CADENCE, EXPRESSIONS, POOLS, type Expression } from "./expression-data";
import { FaceState } from "./expression-state";

/**
 * 小 Grok 头像：GrokBot 表情引擎的 Canvas 2D 移植（源：Android GrokFaceView.java）。
 *
 * 绘制：透明画布上每只眼睛为 48 点路径，球体由 CSS 绘制。动画：弹簧插值切换表情（morph 0→1）、
 * 两段式眨眼（320ms）、视线偏移、球面投影旋转（透视缩放 + 深度淡出）、
 * 按状态的自动表情/眨眼节奏。
 *
 * 坐标系沿用 gist viewBox 229 空间，draw 时整体缩放适配画布尺寸。
 */

// ---- 229 坐标系常量（与 gist viewBox 一致）----
export const CX = 114.2705; // 身体/球面中心 x
export const RADIUS = 105; // 球面半径
export const BODY_W = 228.54; // 身体宽度
export const BODY_H = 228.54; // 身体高度（绘制为圆）
export const GAZE_SCALE_X = 13.2;
export const GAZE_SCALE_Y = 8.4;
export const DEG_TO_RAD = Math.PI / 180;
export const BLINK_MS = 320;

/** 表情弹簧的固有频率；7 是临界阻尼，过渡不过冲。 */
export const FREQUENCY = 7;

const RING_POINTS = 48;
const EYE_COUNT = 2;
const EYE_FILL = "rgb(255, 255, 255)";


/** 转到背面（深度 <= 该值）的眼睛直接不画，避免反面轮廓露出来。 */
export const DEPTH_CUTOFF = 0.02;

export function clamp(v: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, v));
}

/**
 * 弹簧积分一步（gist 的 animate）。速度先按旧位移更新，位移再按新速度更新，
 * 顺序与 Java 一致，换成显式欧拉的另一半会改变收敛曲线。
 */
export function springStep(
  morph: number,
  velocity: number,
  frequency: number,
  dt: number,
): { morph: number; velocity: number } {
  const nextVelocity =
    velocity + (-2 * frequency * velocity - frequency * frequency * (morph - 1)) * dt;
  return { morph: morph + nextVelocity * dt, velocity: nextVelocity };
}

/** 视线平滑：每步吃掉与目标差值的 dt*10 比例，一步最多走完（dt 大时不越过目标）。 */
export function smoothGaze(current: number, target: number, dt: number): number {
  return current + (target - current) * Math.min(1, dt * 10);
}

/** 眨眼归一化进度 t → 眼睛纵向缩放：前 42% 闭上、后 58% 睁开，最小留 0.04 的缝。 */
export function blinkScaleAt(t: number): number {
  if (t >= 1) return 1;
  return Math.max(t < 0.42 ? 1 - t / 0.42 : (t - 0.42) / 0.58, 0.04);
}

/** 转头角度（弧度）：sin(2πt) 摆到最大再随 (1-t) 衰减回正，t 会被夹到 [0,1]。 */
export function turnRadians(t: number, amplitudeDeg: number): number {
  const p = clamp(t, 0, 1);
  return Math.sin(p * Math.PI * 2) * amplitudeDeg * (1 - p) * DEG_TO_RAD;
}

/** 呼吸缩放：4 秒一个周期，幅度 ±1.2%。 */
export function breathScale(nowMs: number): number {
  return 1 + 0.012 * Math.sin((2 * Math.PI * (nowMs % 4000)) / 4000);
}

/** 球面投影后的眼睛：变换用的中心与缩放，以及用于淡出的深度。 */
export interface EyePlacement {
  x: number;
  y: number;
  scaleX: number;
  scaleY: number;
  depth: number;
}

/**
 * 球面投影（gist 的 render）：眼睛中心相对球心的经度决定它转到哪一侧，
 * 深度同时决定透视缩放——转到侧面就该变小，转到背面由调用方丢弃。
 */
export function projectEye(input: {
  centerX: number;
  centerY: number;
  turn: number;
  gazeX: number;
  gazeY: number;
  blink: number;
  baseScale: number;
}): EyePlacement {
  const baseLon = Math.asin(clamp((input.centerX - CX) / RADIUS, -1, 1));
  const lon = baseLon + input.turn;
  const depth = Math.cos(lon);
  const perspective =
    Math.max(depth, DEPTH_CUTOFF) / Math.max(Math.cos(baseLon), DEPTH_CUTOFF);
  return {
    x: CX + RADIUS * Math.sin(lon) + input.gazeX * GAZE_SCALE_X,
    y: input.centerY + input.gazeY * GAZE_SCALE_Y,
    scaleX: clamp(perspective * input.baseScale, 0.02, 2.4),
    scaleY: clamp(input.blink * input.baseScale, 0.02, 2.4),
    depth,
  };
}

export interface GrokFaceOptions {
  /** 时钟（毫秒），默认 performance.now()；测试可注入。 */
  now?: () => number;
  /** 随机数 [0,1)，默认 Math.random()；测试可注入以求确定性。 */
  random?: () => number;
}

/** 内部状态快照，供测试与调试读。 */
export interface FaceSnapshot {
  faceState: FaceState;
  morph: number;
  velocity: number;
  gazeX: number;
  gazeY: number;
  turn: number;
  expressionIndex: number;
  running: boolean;
}

export class GrokFace {
  private readonly ctx: CanvasRenderingContext2D;
  private width: number;
  private height: number;
  private readonly now: () => number;
  private readonly random: () => number;

  // ---- 表情弹簧插值 ----
  private currentRings: Expression; // 可变起点
  private targetRings: Expression; // 引用 expression-data
  private morph = 1;
  private velocity = 0;
  private expressionIndex = 0;

  // ---- 视线 / 旋转 ----
  private gazeX = 0;
  private gazeY = 0; // 当前（平滑）
  private gazeTargetX = 0;
  private gazeTargetY = 0;
  private turn = 0; // 弧度
  private turnAnimStart = -1;
  private turnAnimDuration = 0;
  private turnAnimAmplitude = 0; // 度
  private baseScale = 1; // gist 里可整体缩放球面，Android 固定 1

  // ---- 眨眼 / 自动行为 ----
  private blinkStart = -1;
  private nextBlinkAt = Number.MAX_SAFE_INTEGER;
  private nextExpressionAt = Number.MAX_SAFE_INTEGER;

  // ---- 状态 ----
  private faceState: FaceState = FaceState.IDLE;
  private pool: readonly number[] | undefined;
  private exprCadence: readonly [number, number] | undefined;
  private blinkCadence: readonly [number, number] | undefined;

  // ---- 帧循环 ----
  private running = false;
  private frameScheduled = false;
  private frameHandle: number | null = null;
  private delayHandle: ReturnType<typeof setTimeout> | null = null;
  private lastFrameMs = 0;

  constructor(
    ctx: CanvasRenderingContext2D,
    width: number,
    height: number,
    options: GrokFaceOptions = {},
  ) {
    this.ctx = ctx;
    this.width = width;
    this.height = height;
    this.now = options.now ?? (() => performance.now());
    this.random = options.random ?? Math.random;
    this.currentRings = copyRings(0);
    this.targetRings = EXPRESSIONS[0];
    this.setFaceState(FaceState.IDLE);
  }

  // ------------------------------------------------------------------
  // 公共 API
  // ------------------------------------------------------------------

  setFaceState(state: FaceState): void {
    this.faceState = state;
    this.pool = POOLS[state];
    this.exprCadence = EXPR_CADENCE[state];
    this.blinkCadence = BLINK[state];
    if (this.pool && this.pool.length > 0) this.selectExpression(this.pool[0]);
    const now = this.now();
    this.nextBlinkAt = this.blinkCadence
      ? now + this.randomIn(this.blinkCadence)
      : Number.MAX_SAFE_INTEGER;
    this.nextExpressionAt = this.exprCadence
      ? now + this.randomIn(this.exprCadence)
      : Number.MAX_SAFE_INTEGER;
    this.ensureFrame();
  }

  getFaceState(): FaceState {
    return this.faceState;
  }

  setGaze(dx: number, dy: number): void {
    this.gazeTargetX = clamp(dx, -1, 1);
    this.gazeTargetY = clamp(dy, -1, 1);
  }

  blinkNow(): void {
    this.blinkStart = this.now();
  }

  playTurn(amplitudeDeg: number, durationMs: number): void {
    this.turnAnimAmplitude = amplitudeDeg;
    this.turnAnimDuration = durationMs;
    this.turnAnimStart = this.now();
  }

  spinOnce(): void {
    this.playTurn(85, 1200);
  }

  wakeTurn(): void {
    this.playTurn(28, 700);
  }

  resize(width: number, height: number): void {
    this.width = width;
    this.height = height;
  }

  start(): void {
    if (this.running) return;
    this.running = true;
    this.ensureFrame();
  }

  stop(): void {
    this.running = false;
    this.cancelFrame();
  }

  /** 只停帧循环：对象不持有需要显式释放的资源，剩下的交给 GC。 */
  dispose(): void {
    this.stop();
  }

  isRunning(): boolean {
    return this.running;
  }

  snapshot(): FaceSnapshot {
    return {
      faceState: this.faceState,
      morph: this.morph,
      velocity: this.velocity,
      gazeX: this.gazeX,
      gazeY: this.gazeY,
      turn: this.turn,
      expressionIndex: this.expressionIndex,
      running: this.running,
    };
  }

  /** 推进一帧动画：dt 为秒，now 为毫秒（对应 Java 的 frameTimeNanos / uptimeMillis）。 */
  step(dt: number, now: number = this.now()): void {
    const spring = springStep(this.morph, this.velocity, FREQUENCY, dt);
    this.morph = spring.morph;
    this.velocity = spring.velocity;
    if (!Number.isFinite(this.morph)) {
      this.morph = 1;
      this.velocity = 0;
    }

    this.gazeX = smoothGaze(this.gazeX, this.gazeTargetX, dt);
    this.gazeY = smoothGaze(this.gazeY, this.gazeTargetY, dt);

    if (this.blinkCadence && now >= this.nextBlinkAt) {
      this.blinkStart = now;
      this.nextBlinkAt = now + this.randomIn(this.blinkCadence);
    }

    if (this.exprCadence && now >= this.nextExpressionAt) {
      if (this.pool && this.pool.length > 0) {
        const pool = this.pool;
        let pick = pool[Math.floor(this.random() * pool.length)];
        // 摇到当前表情等于没换，重摇（与 Java 相同；池内不重复，必然摇得出去）
        while (pool.length > 1 && pick === this.expressionIndex) {
          pick = pool[Math.floor(this.random() * pool.length)];
        }
        this.selectExpression(pick);
      }
      this.nextExpressionAt = now + this.randomIn(this.exprCadence);
    }

    if (this.turnAnimAmplitude !== 0) {
      const t = Math.min((now - this.turnAnimStart) / this.turnAnimDuration, 1);
      this.turn = turnRadians(t, this.turnAnimAmplitude);
      if (t >= 1) {
        this.turnAnimAmplitude = 0;
        this.turn = 0;
      }
    }
  }

  // ------------------------------------------------------------------
  // 帧循环
  // ------------------------------------------------------------------

  private ensureFrame(): void {
    if (!this.running || this.frameScheduled) return;
    this.lastFrameMs = this.now();
    this.scheduleFrame(0);
  }

  private scheduleFrame(delayMs: number): void {
    this.frameScheduled = true;
    if (delayMs > 0) {
      this.delayHandle = setTimeout(() => {
        this.delayHandle = null;
        if (this.running) this.frameHandle = requestAnimationFrame(this.frame);
      }, delayMs);
    } else {
      this.frameHandle = requestAnimationFrame(this.frame);
    }
  }

  private cancelFrame(): void {
    if (this.frameHandle !== null) {
      cancelAnimationFrame(this.frameHandle);
      this.frameHandle = null;
    }
    if (this.delayHandle !== null) {
      clearTimeout(this.delayHandle);
      this.delayHandle = null;
    }
    this.frameScheduled = false;
  }

  private readonly frame = (nowMs: number): void => {
    this.frameHandle = null;
    this.frameScheduled = false;
    const dt = Math.min((nowMs - this.lastFrameMs) / 1000, 0.1);
    this.lastFrameMs = nowMs;
    this.step(dt, this.now());
    this.draw();
    const settled = Math.abs(this.morph - 1) < 0.03 && Math.abs(this.velocity) < 1;
    // 睡着又动完了就降频到 ~4fps：全屏重绘没必要每帧跑，省电
    this.scheduleFrame(this.faceState === FaceState.SLEEPING && settled ? 250 : 0);
  };

  // ------------------------------------------------------------------
  // 表情插值（gist selectExpression / displayedRings）
  // ------------------------------------------------------------------

  private selectExpression(index: number): void {
    if (index < 0 || index >= EXPRESSIONS.length) return;
    this.currentRings = this.displayedRings(); // 固定当前帧为插值起点
    this.targetRings = EXPRESSIONS[index];
    this.expressionIndex = index;
    this.morph = 0;
    this.velocity = 0;
  }

  private displayedRings(): Expression {
    const m = clamp(this.morph, 0, 1);
    const out: Expression = [];
    for (let e = 0; e < EYE_COUNT; e++) {
      const from = this.currentRings[e];
      const to = this.targetRings[e];
      const ring: number[][] = [];
      for (let p = 0; p < RING_POINTS; p++) {
        ring.push([
          from[p][0] + (to[p][0] - from[p][0]) * m,
          from[p][1] + (to[p][1] - from[p][1]) * m,
        ]);
      }
      out.push(ring);
    }
    return out;
  }

  // ------------------------------------------------------------------
  // 绘制
  // ------------------------------------------------------------------

  draw(): void {
    const ctx = this.ctx;
    const vw = this.width;
    const vh = this.height;
    if (vw <= 0 || vh <= 0) return;
    ctx.clearRect(0, 0, vw, vh);
    const s = Math.min(vw / BODY_W, vh / BODY_H);
    ctx.save();
    ctx.translate((vw - BODY_W * s) / 2, (vh - BODY_H * s) / 2);
    ctx.scale(s, s);

    // 眼睛裁剪在身体圆内，避免转头时越界
    ctx.save();
    ctx.beginPath();
    ctx.ellipse(CX, CX, BODY_W / 2, BODY_H / 2, 0, 0, Math.PI * 2);
    ctx.clip();
    const rings = this.displayedRings();
    const blink = this.blinkScale();
    ctx.fillStyle = EYE_FILL;
    for (let eye = 0; eye < EYE_COUNT; eye++) {
      const ring = rings[eye];
      let centerX = 0;
      let centerY = 0;
      for (let p = 0; p < RING_POINTS; p++) {
        centerX += ring[p][0];
        centerY += ring[p][1];
      }
      centerX /= RING_POINTS;
      centerY /= RING_POINTS;

      const placed = projectEye({
        centerX,
        centerY,
        turn: this.turn,
        gazeX: this.gazeX,
        gazeY: this.gazeY,
        blink,
        baseScale: this.baseScale,
      });
      if (placed.depth <= DEPTH_CUTOFF) continue;

      // Canvas 2D 在 moveTo/lineTo 时就把当前矩阵烘进路径了（Java 的 Path 是绘制时
      // 才套 Matrix），所以必须先把变换设好，再建路径——顺序反了就画在原地了。
      ctx.save();
      ctx.translate(placed.x, placed.y);
      ctx.scale(placed.scaleX, placed.scaleY);
      ctx.translate(-centerX, -centerY);
      ctx.beginPath();
      ctx.moveTo(ring[0][0], ring[0][1]);
      for (let p = 1; p < RING_POINTS; p++) ctx.lineTo(ring[p][0], ring[p][1]);
      ctx.closePath();
      ctx.fill();
      ctx.restore();
    }
    ctx.restore();
    ctx.restore();
  }

  private blinkScale(): number {
    if (this.blinkStart < 0) return 1;
    const t = (this.now() - this.blinkStart) / BLINK_MS;
    if (t >= 1) {
      this.blinkStart = -1;
      return 1;
    }
    return blinkScaleAt(t);
  }

  private randomIn(range: readonly [number, number]): number {
    return range[0] + Math.floor(this.random() * (range[1] - range[0]));
  }
}

function copyRings(index: number): Expression {
  const src = EXPRESSIONS[index];
  const copy: Expression = [];
  for (let e = 0; e < EYE_COUNT; e++) {
    const ring: number[][] = [];
    for (let p = 0; p < RING_POINTS; p++) ring.push([src[e][p][0], src[e][p][1]]);
    copy.push(ring);
  }
  return copy;
}
