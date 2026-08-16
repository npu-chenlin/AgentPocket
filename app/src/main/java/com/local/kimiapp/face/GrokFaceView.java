package com.local.kimiapp.face;

import android.content.Context;
import android.graphics.Canvas;
import android.graphics.Color;
import android.graphics.LinearGradient;
import android.graphics.Matrix;
import android.graphics.Paint;
import android.graphics.Path;
import android.graphics.Shader;
import android.os.SystemClock;
import android.util.AttributeSet;
import android.view.Choreographer;
import android.view.View;

import java.util.Random;

/**
 * 小 Grok 头像：GrokBot 表情引擎的 Android 移植。
 *
 * 绘制：身体为蓝渐变椭圆，眼睛为 48 点路径。动画：
 * 弹簧插值切换表情（morph 0→1）、两段式眨眼（320ms）、视线偏移、
 * 球面投影旋转（透视缩放 + 深度淡出）、按状态的自动表情/眨眼节奏。
 *
 * 坐标系沿用 gist viewBox 229 空间，onDraw 时整体缩放适配 View 尺寸。
 */
public class GrokFaceView extends View implements Choreographer.FrameCallback {
    // ---- 229 坐标系常量（与 gist viewBox 一致）----
    private static final float CX = 114.2705f;    // 身体/球面中心 x
    private static final float RADIUS = 105f;     // 球面半径
    private static final float BODY_W = 228.54f;  // 身体宽度
    private static final float BODY_H = 228.54f;  // 身体高度（绘制为圆）
    private static final float GAZE_SCALE_X = 13.2f;
    private static final float GAZE_SCALE_Y = 8.4f;
    private static final float DEG_TO_RAD = (float) (Math.PI / 180.0);
    private static final float BLINK_MS = 320f;

    private final Paint bodyPaint = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Paint eyePaint = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Matrix eyeMatrix = new Matrix();
    private final Path[] eyePaths = new Path[2];
    private final Path clipPath = new Path();
    private final Random random = new Random();

    // ---- 表情弹簧插值 ----
    private float[][][] currentRings;              // [2][48][2]，可变起点
    private float[][][] targetRings;               // [2][48][2]，引用 ExpressionData
    private float morph = 1f;
    private float velocity = 0f;
    private float frequency = 7f;
    private int expressionIndex = 0;

    // ---- 视线 / 旋转 ----
    private float gazeX = 0f, gazeY = 0f;          // 当前（平滑）
    private float gazeTargetX = 0f, gazeTargetY = 0f;
    private float turn = 0f;                       // 弧度
    private long turnAnimStart = -1L;
    private long turnAnimDuration = 0L;
    private float turnAnimAmplitude = 0f;          // 度
    private float baseScale = 1f;

    // ---- 眨眼 / 自动行为 ----
    private long blinkStart = -1L;
    private long nextBlinkAt = Long.MAX_VALUE;
    private long nextExpressionAt = Long.MAX_VALUE;

    // ---- 状态 ----
    private GrokFaceState faceState = GrokFaceState.IDLE;
    private int[] pool;
    private long[] exprCadence;
    private long[] blinkCadence;

    // ---- 帧循环 ----
    private boolean frameScheduled = false;
    private long lastFrameNanos = 0L;

    public GrokFaceView(Context context) {
        this(context, null);
    }

    public GrokFaceView(Context context, AttributeSet attrs) {
        super(context, attrs);
        eyePaint.setColor(Color.WHITE);
        eyePaint.setStyle(Paint.Style.FILL);
        bodyPaint.setStyle(Paint.Style.FILL);
        for (int i = 0; i < 2; i++) eyePaths[i] = new Path();
        currentRings = copyRings(0);
        targetRings = ExpressionData.EXPRESSIONS[0];
        setFaceState(GrokFaceState.IDLE);
    }

    // ------------------------------------------------------------------
    // 公共 API
    // ------------------------------------------------------------------

    public void setFaceState(GrokFaceState state) {
        if (state == null) return;
        faceState = state;
        pool = ExpressionData.POOLS.get(state.gistState);
        exprCadence = ExpressionData.EXPR_CADENCE.get(state.gistState);
        blinkCadence = ExpressionData.BLINK.get(state.gistState);
        if (pool != null && pool.length > 0) selectExpression(pool[0]);
        long now = SystemClock.uptimeMillis();
        nextBlinkAt = blinkCadence == null ? Long.MAX_VALUE : now + randomIn(blinkCadence);
        nextExpressionAt = exprCadence == null ? Long.MAX_VALUE : now + randomIn(exprCadence);
        ensureFrame();
    }

    public GrokFaceState getFaceState() {
        return faceState;
    }

    public void setGaze(float dx, float dy) {
        gazeTargetX = clamp(dx, -1f, 1f);
        gazeTargetY = clamp(dy, -1f, 1f);
    }

    public void blinkNow() {
        blinkStart = SystemClock.uptimeMillis();
    }

    public void playTurn(float amplitudeDeg, long durationMs) {
        turnAnimAmplitude = amplitudeDeg;
        turnAnimDuration = durationMs;
        turnAnimStart = SystemClock.uptimeMillis();
    }

    public void spinOnce() {
        playTurn(85f, 1200L);
    }

    public void wakeTurn() {
        playTurn(28f, 700L);
    }

    // ------------------------------------------------------------------
    // 表情插值（gist selectExpression / displayedRings / animate）
    // ------------------------------------------------------------------

    private float[][][] copyRings(int index) {
        float[][][] src = ExpressionData.EXPRESSIONS[index];
        float[][][] copy = new float[2][48][2];
        for (int e = 0; e < 2; e++)
            for (int p = 0; p < 48; p++) {
                copy[e][p][0] = src[e][p][0];
                copy[e][p][1] = src[e][p][1];
            }
        return copy;
    }

    private void selectExpression(int index) {
        if (index < 0 || index >= ExpressionData.EXPRESSIONS.length) return;
        currentRings = displayedRings();          // 固定当前帧为插值起点
        targetRings = ExpressionData.EXPRESSIONS[index];
        expressionIndex = index;
        morph = 0f;
        velocity = 0f;
    }

    private float[][][] displayedRings() {
        float[][][] out = new float[2][48][2];
        float m = clamp(morph, 0f, 1f);
        for (int e = 0; e < 2; e++)
            for (int p = 0; p < 48; p++) {
                out[e][p][0] = currentRings[e][p][0]
                        + (targetRings[e][p][0] - currentRings[e][p][0]) * m;
                out[e][p][1] = currentRings[e][p][1]
                        + (targetRings[e][p][1] - currentRings[e][p][1]) * m;
            }
        return out;
    }

    // ------------------------------------------------------------------
    // 帧循环
    // ------------------------------------------------------------------

    @Override protected void onAttachedToWindow() {
        super.onAttachedToWindow();
        ensureFrame();
    }

    @Override protected void onDetachedFromWindow() {
        super.onDetachedFromWindow();
        if (frameScheduled) {
            Choreographer.getInstance().removeFrameCallback(this);
            frameScheduled = false;
        }
    }

    private void ensureFrame() {
        if (!frameScheduled && isAttachedToWindow()) {
            frameScheduled = true;
            lastFrameNanos = System.nanoTime();
            Choreographer.getInstance().postFrameCallback(this);
        }
    }

    @Override public void doFrame(long frameTimeNanos) {
        frameScheduled = false;
        float dt = Math.min((frameTimeNanos - lastFrameNanos) / 1e9f, 0.1f);
        lastFrameNanos = frameTimeNanos;
        step(dt);
        invalidate();
        boolean settled = Math.abs(morph - 1f) < 0.03f && Math.abs(velocity) < 1f;
        if (faceState == GrokFaceState.SLEEPING && settled) {
            Choreographer.getInstance().postFrameCallbackDelayed(this, 250L);  // 睡眠降频 ~4fps
        } else {
            Choreographer.getInstance().postFrameCallback(this);
        }
        frameScheduled = true;
    }

    private void step(float dt) {
        long now = SystemClock.uptimeMillis();
        // 弹簧积分（gist animate）：velocity += (-2*freq*v - freq^2*(morph-1)) * dt
        velocity += (-2f * frequency * velocity - frequency * frequency * (morph - 1f)) * dt;
        morph += velocity * dt;
        if (!Float.isFinite(morph)) {
            morph = 1f;
            velocity = 0f;
        }
        // 视线平滑
        gazeX += (gazeTargetX - gazeX) * Math.min(1f, dt * 10f);
        gazeY += (gazeTargetY - gazeY) * Math.min(1f, dt * 10f);
        // 自动眨眼（gist scheduleBlink）
        if (blinkCadence != null && now >= nextBlinkAt) {
            blinkStart = now;
            nextBlinkAt = now + randomIn(blinkCadence);
        }
        // 自动表情（gist scheduleExpression）
        if (exprCadence != null && now >= nextExpressionAt) {
            if (pool != null && pool.length > 0) {
                int pick = pool[random.nextInt(pool.length)];
                if (pool.length > 1) {
                    while (pick == expressionIndex) pick = pool[random.nextInt(pool.length)];
                }
                selectExpression(pick);
            }
            nextExpressionAt = now + randomIn(exprCadence);
        }
        // 转头动画（gist spin-demo 公式：turn = sin(2πt) * amp * (1-t)）
        if (turnAnimAmplitude != 0f) {
            float t = Math.min((now - turnAnimStart) / (float) turnAnimDuration, 1f);
            turn = (float) (Math.sin(t * Math.PI * 2) * turnAnimAmplitude * (1f - t)) * DEG_TO_RAD;
            if (t >= 1f) {
                turnAnimAmplitude = 0f;
                turn = 0f;
            }
        }
    }

    // ------------------------------------------------------------------
    // 绘制
    // ------------------------------------------------------------------

    @Override protected void onDraw(Canvas canvas) {
        super.onDraw(canvas);
        float vw = getWidth();
        float vh = getHeight();
        if (vw <= 0 || vh <= 0) return;
        if (bodyPaint.getShader() == null) {
            bodyPaint.setShader(new LinearGradient(0f, 0f, vw, vh,
                    new int[]{Color.rgb(96, 188, 255), Color.rgb(34, 130, 244), Color.rgb(12, 56, 160)},
                    null, Shader.TileMode.CLAMP));
        }
        float s = Math.min(vw / BODY_W, vh / BODY_H);
        canvas.save();
        canvas.translate((vw - BODY_W * s) / 2f, (vh - BODY_H * s) / 2f);
        canvas.scale(s, s);
        // 身体 + 呼吸（4s 周期微幅缩放）
        float breath = 1f + 0.012f * (float) Math.sin(
                2 * Math.PI * (SystemClock.uptimeMillis() % 4000L) / 4000.0);
        canvas.save();
        canvas.translate(CX, CX);
        canvas.scale(breath, breath);
        canvas.translate(-CX, -CX);
        canvas.drawOval(CX - BODY_W / 2f, CX - BODY_W / 2f, CX + BODY_W / 2f, CX + BODY_W / 2f, bodyPaint);
        canvas.restore();
        // 眼睛（裁剪在身体圆内，避免转头时越界）
        canvas.save();
        clipPath.reset();
        clipPath.addOval(CX - BODY_W / 2f, CX - BODY_W / 2f, CX + BODY_W / 2f, CX + BODY_W / 2f, Path.Direction.CW);
        canvas.clipPath(clipPath);
        float[][][] rings = displayedRings();
        float blink = blinkScale();
        for (int eye = 0; eye < 2; eye++) {
            float[][] ring = rings[eye];
            float cx = 0f, cy = 0f;
            for (int p = 0; p < 48; p++) {
                cx += ring[p][0];
                cy += ring[p][1];
            }
            cx /= 48f;
            cy /= 48f;
            // 球面投影（gist render）
            float offset = cx - CX;
            float baseLon = (float) Math.asin(clamp(offset / RADIUS, -1f, 1f));
            float lon = baseLon + turn;
            float depth = (float) Math.cos(lon);
            float perspective = Math.max(depth, 0.02f) / Math.max((float) Math.cos(baseLon), 0.02f);
            float x = CX + RADIUS * (float) Math.sin(lon) + gazeX * GAZE_SCALE_X;
            float y = cy + gazeY * GAZE_SCALE_Y;
            float sx = clamp(perspective * baseScale, 0.02f, 2.4f);
            float sy = clamp(blink * baseScale, 0.02f, 2.4f);
            if (depth <= 0.02f) continue;         // 转到背面淡出
            Path path = eyePaths[eye];
            path.reset();
            path.moveTo(ring[0][0], ring[0][1]);
            for (int p = 1; p < 48; p++) path.lineTo(ring[p][0], ring[p][1]);
            path.close();
            // transform = translate(x,y) scale(sx,sy) translate(-cx,-cy)
            eyeMatrix.reset();
            eyeMatrix.setTranslate(x, y);
            eyeMatrix.preScale(sx, sy);
            eyeMatrix.preTranslate(-cx, -cy);
            canvas.save();
            canvas.concat(eyeMatrix);
            canvas.drawPath(path, eyePaint);
            canvas.restore();
        }
        canvas.restore();
        canvas.restore();
    }

    private float blinkScale() {
        if (blinkStart < 0) return 1f;
        float t = (SystemClock.uptimeMillis() - blinkStart) / BLINK_MS;
        if (t >= 1f) {
            blinkStart = -1L;
            return 1f;
        }
        return Math.max(t < 0.42f ? 1f - t / 0.42f : (t - 0.42f) / 0.58f, 0.04f);
    }

    private long randomIn(long[] range) {
        if (range == null || range.length < 2) return 3000L;
        return range[0] + (long) (random.nextDouble() * (range[1] - range[0]));
    }

    private static float clamp(float v, float min, float max) {
        return Math.max(min, Math.min(max, v));
    }
}
