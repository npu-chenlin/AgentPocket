# GrokFace 悬浮球 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 KimiWeb 的 56dp 悬浮球升级为"小 Grok 头像"：完整移植 gist "Atelier GrokBot" 的表情引擎（25 表情弹簧插值 / 两段式眨眼 / 视线偏移 / 球面投影旋转），并把应用事件（会话忙碌、任务完成/失败、等待审批/回答、服务器离线、页面加载、拖拽）映射为表情状态。

**Architecture:** 新增 `com.local.kimiapp.face` 包：`ExpressionData`（从 gist HTML 生成的静态数据）、`GrokFaceState`（应用侧状态枚举）、`GrokFaceView`（自定义 View，Choreographer 驱动动画循环）。`MainActivity` 用 GrokFaceView 替换现有眼睛，`KeepAliveService` 通过 SharedPreferences 发布 `active_count` / `last_event`，MainActivity 监听后驱动表情状态。

**Tech Stack:** Android 原生 Java（Java 8 源码兼容）、compileSdk 35 / minSdk 26、AGP 8.9.0、Gradle 8.13、JDK 17。不新增任何第三方依赖。

## Global Constraints

- **每任务提交**：用户已确认建 `grok-face-floating-ball` 分支并每任务提交（全程本地，不 push）。所有实现工作区在 `/home/user/progs/KimiCodeWebApp/.worktrees/grok-face`，任务中的 `cd` 均已指向该目录；`local.properties`、`.superpowers/`、`docs/superpowers/` 为本地/未跟踪文件，不纳入提交。
- **Java 8 源码兼容**：`app/build.gradle` 的 `compileOptions` 为 `VERSION_1_8`，代码不得使用 Java 9+ 语法（无 `var`、无 switch 表达式等）。
- **不新增第三方依赖**：只用 Android 平台 API（View / Canvas / Choreographer / SystemClock / SharedPreferences）。
- **表情数据必须与 gist 完全一致**：25 个表情、每表情 2 只眼睛、每只眼睛 48 点。
- **构建环境固定**：
  - `JAVA_HOME=/home/user/software/jdk17`
  - `GRADLE=/home/user/software/gradle-8.13/bin/gradle`
  - `sdk.dir=/home/user/software/android-sdk`（写入仓库根 `local.properties`）
- **构建产物**：`app/build/outputs/apk/debug/app-debug.apk`。
- 悬浮球尺寸保持 56dp，交互模型（点击唤醒/打开列表、拖拽吸附、3.5s 睡眠）保持不变。

---

### Task 1: 构建工具链 + 基线构建

**Files:**
- Create: `/home/user/progs/KimiCodeWebApp/local.properties`（仓库根）

**Interfaces:**
- Produces: 可用的构建环境（JDK 17 / Gradle / Android SDK），后续所有任务的构建验证都依赖它。

- [ ] **Step 1: 确认下载完成并解压**

下载任务（后台已启动）产物在 `/home/user/software/`：`jdk17.tar.gz`、`gradle-8.13-bin.zip`、`cmdtools.zip`。先确认三个文件都到位（体积约 190MB / 130MB / 130MB），再解压：

```bash
ls -la /home/user/software/
cd /home/user/software
tar xzf jdk17.tar.gz && mv jdk-17* jdk17
unzip -q gradle-8.13-bin.zip          # 若 unzip 不存在，用 python3 -m zipfile -e gradle-8.13-bin.zip .
mkdir -p android-sdk/cmdline-tools
unzip -q cmdtools.zip -d android-sdk/cmdline-tools
mv android-sdk/cmdline-tools/cmdline-tools android-sdk/cmdline-tools/latest
rm -f jdk17.tar.gz gradle-8.13-bin.zip cmdtools.zip
```

预期：`/home/user/software/jdk17/bin/java`、`/home/user/software/gradle-8.13/bin/gradle`、`/home/user/software/android-sdk/cmdline-tools/latest/bin/sdkmanager` 均存在。

- [ ] **Step 2: 验证 JDK 版本并安装 SDK 组件**

```bash
export JAVA_HOME=/home/user/software/jdk17
$JAVA_HOME/bin/java -version        # 预期: openjdk version "17.x"
export PATH=$JAVA_HOME/bin:/home/user/software/android-sdk/cmdline-tools/latest/bin:$PATH
export ANDROID_HOME=/home/user/software/android-sdk
yes | sdkmanager --licenses >/dev/null
sdkmanager "platform-tools" "platforms;android-35" "build-tools;35.0.0"
```

预期：三条命令退出码 0，`/home/user/software/android-sdk/platforms/android-35` 与 `build-tools/35.0.0` 存在。

- [ ] **Step 3: 写 local.properties**

```bash
echo "sdk.dir=/home/user/software/android-sdk" > /home/user/progs/KimiCodeWebApp/.worktrees/grok-face/local.properties
```

- [ ] **Step 4: 基线构建（改动前先确认能编译）**

```bash
cd /home/user/progs/KimiCodeWebApp/.worktrees/grok-face
JAVA_HOME=/home/user/software/jdk17 /home/user/software/gradle-8.13/bin/gradle assembleDebug
```

预期：`BUILD SUCCESSFUL`（首次构建会从 google()/mavenCentral() 下载 AGP 与依赖，耗时较长）。产物 `app/build/outputs/apk/debug/app-debug.apk` 存在。若失败，先记录错误（可能是网络/代理问题），不要跳过本步骤。

---

### Task 2: 生成表情数据 ExpressionData.java

**Files:**
- Create: `tools/gen_expression_data.py`
- Create: `app/src/main/java/com/local/kimiapp/face/ExpressionData.java`（脚本生成）

**Interfaces:**
- Produces:
  - `public static final float[][][][] ExpressionData.EXPRESSIONS` — `[25][2][48][2]`（表情 × 眼睛 × 点 × x/y）
  - `public static final Map<String,int[]> ExpressionData.POOLS` — gist 状态名 → 表情索引池（仅含本设计用到的 10 个状态）
  - `public static final Map<String,long[]> ExpressionData.EXPR_CADENCE` — 状态 → 表情切换节奏 `[minMs, maxMs]`
  - `public static final Map<String,long[]> ExpressionData.BLINK` — 状态 → 眨眼节奏；无眨眼的状态（sleeping/waking）不包含该键

- [ ] **Step 1: 写生成脚本**

```python
#!/usr/bin/env python3
"""从 gist HTML 生成 ExpressionData.java（GrokBot 表情引擎数据）。

用法: python3 tools/gen_expression_data.py [gist_html] [输出java路径]
默认输入: /home/user/grokbot-demo/index.html
默认输出: app/src/main/java/com/local/kimiapp/face/ExpressionData.java
"""
import ast
import re
import sys
import os

USED_STATES = ["sleeping", "waking", "idle", "working", "loading",
               "celebrate", "thinking", "sad", "drowsy", "dragging"]


def extract_literal(src, name):
    """提取 `const NAME = <literal>` 并用 ast.literal_eval 解析（用于数组字面量）。"""
    marker = "const %s = " % name
    start = src.index(marker) + len(marker)
    i, depth = start, 0
    while i < len(src):
        if src[i] == '[':
            depth += 1
        elif src[i] == ']':
            depth -= 1
            if depth == 0:
                break
        i += 1
    return ast.literal_eval(src[start:i + 1])


def extract_flat_map(src, name):
    """提取 `const NAME = { key: [a,b] | null, ... }` 为 {key: (a,b) | None}。

    JS 对象字面量（裸 key + null）不是合法 Python，故用正则而非 literal_eval。
    """
    marker = "const %s = {" % name
    start = src.index(marker)
    i, depth = start, 0
    while i < len(src):
        c = src[i]
        if c == '{':
            depth += 1
        elif c == '}':
            depth -= 1
            if depth == 0:
                break
        i += 1
    body = src[start:i + 1]
    out = {}
    for key, val in re.findall(r"(\w+)\s*:\s*(\[[\d\s,]*\]|null)", body):
        if val == "null":
            out[key] = None
        else:
            nums = [int(x) for x in val.strip("[]").split(",") if x.strip()]
            out[key] = tuple(nums)
    return out


def fmt_float(v):
    s = "%.2f" % v
    if s.startswith("-0.00"):
        return "0.00f"
    return s + "f"


def java_float_array(rings):
    lines = []
    for ring in rings:
        pts = ", ".join("{%s, %s}" % (fmt_float(p[0]), fmt_float(p[1])) for p in ring)
        lines.append("                {" + pts + "}")
    return ",\n".join(lines)


def java_int_array(vals):
    return "new int[]{" + ", ".join(str(v) for v in vals) + "}"


def java_long_array(vals):
    return "new long[]{" + ", ".join(str(v) + "L" for v in vals) + "}"


def java_map_builder(name, key_type, val_expr):
    """生成 private static Map<...> name() { Map m = new HashMap<>(); m.put(...); return m; }"""
    lines = ["    private static Map<String, %s> %s() {" % (key_type, name),
             "        Map<String, %s> m = new HashMap<>();" % key_type]
    for key, val in sorted(val_expr.items()):
        lines.append('        m.put("%s", %s);' % (key, val))
    lines.append("        return m;")
    lines.append("    }")
    return "\n".join(lines)


def main():
    gist_html = sys.argv[1] if len(sys.argv) > 1 else "/home/user/grokbot-demo/index.html"
    out_path = sys.argv[2] if len(sys.argv) > 2 else \
        "app/src/main/java/com/local/kimiapp/face/ExpressionData.java"

    src = open(gist_html, encoding="utf-8").read()
    expressions = extract_literal(src, "EXPRESSIONS")
    pools = extract_flat_map(src, "POOLS")
    expr_cadence = extract_flat_map(src, "EXPR_CADENCE")
    blink = extract_flat_map(src, "BLINK")

    assert len(expressions) == 25, "预期 25 个表情，实际 %d" % len(expressions)
    for e in expressions:
        assert len(e) == 2 and all(len(r) == 48 for r in e), "表情结构异常"
    missing = [s for s in USED_STATES if s not in pools]
    assert not missing, "POOLS 缺少状态: %s" % missing

    java = []
    java.append("package com.local.kimiapp.face;")
    java.append("")
    java.append("import java.util.HashMap;")
    java.append("import java.util.Map;")
    java.append("")
    java.append("/**")
    java.append(" * GrokBot 表情引擎数据（由 tools/gen_expression_data.py 从 gist 生成，勿手改）。")
    java.append(" * EXPRESSIONS[expression][eye][point] = {x, y}，坐标系与 gist viewBox 一致。")
    java.append(" */")
    java.append("public final class ExpressionData {")
    java.append("    public static final float[][][][] EXPRESSIONS = {")
    for i, e in enumerate(expressions):
        comma = "," if i < len(expressions) - 1 else ""
        java.append("        {")
        java.append(java_float_array(e))
        java.append("        }%s" % comma)
    java.append("    };")
    java.append("")

    used_pools = {s: pools[s] for s in USED_STATES}
    used_expr = {s: expr_cadence[s] for s in USED_STATES}
    used_blink = {s: blink[s] for s in USED_STATES if blink[s] is not None}
    java.append(java_map_builder("pools", "int[]",
                                 {k: java_int_array(v) for k, v in used_pools.items()}))
    java.append("")
    java.append(java_map_builder("exprCadence", "long[]",
                                 {k: java_long_array(v) for k, v in used_expr.items()}))
    java.append("")
    java.append(java_map_builder("blink", "long[]",
                                 {k: java_long_array(v) for k, v in used_blink.items()}))
    java.append("")
    java.append("    public static final Map<String, int[]> POOLS = pools();")
    java.append("    public static final Map<String, long[]> EXPR_CADENCE = exprCadence();")
    java.append("    public static final Map<String, long[]> BLINK = blink();")
    java.append("}")
    java.append("")

    os.makedirs(os.path.dirname(out_path), exist_ok=True)
    with open(out_path, "w", encoding="utf-8") as f:
        f.write("\n".join(java))
    print("wrote %s (%d expressions)" % (out_path, len(expressions)))


if __name__ == "__main__":
    main()
```

- [ ] **Step 2: 运行脚本并核对输出**

```bash
cd /home/user/progs/KimiCodeWebApp/.worktrees/grok-face
mkdir -p tools
# 把上面 Step 1 的脚本写入 tools/gen_expression_data.py（内容见上）
python3 tools/gen_expression_data.py
head -20 app/src/main/java/com/local/kimiapp/face/ExpressionData.java
python3 - <<'EOF'
# 复核：解析生成的 EXPRESSIONS 数量与 gist 一致
import re
src = open("app/src/main/java/com/local/kimiapp/face/ExpressionData.java", encoding="utf-8").read()
marker = "EXPRESSIONS = {"
start = src.index(marker) + len(marker) - 1   # 指向外层 '{'
i, depth = start, 0
while i < len(src):
    if src[i] == '{': depth += 1
    elif src[i] == '}':
        depth -= 1
        if depth == 0: break
    i += 1
body = src[start:i+1]
nums = re.findall(r"([-0-9.]+)f", body)
assert len(nums) == 25 * 2 * 48 * 2, "float 数量异常: %d" % len(nums)
print("generated expressions: 25 rings: 2 points: 48 (floats = %d)" % len(nums))
EOF
```

预期：脚本输出 `wrote ... (25 expressions)`，复核脚本输出 `generated expressions: 25 rings: 2 points: 48`。

- [ ] **Step 3: 独立 javac 编译 ExpressionData.java 验证语法**

`ExpressionData.java` 只依赖 `java.util`，可脱离 Android SDK 用 JDK 编译：

```bash
cd /home/user/progs/KimiCodeWebApp/.worktrees/grok-face
mkdir -p /tmp/facecheck
cp app/src/main/java/com/local/kimiapp/face/ExpressionData.java /tmp/facecheck/
cd /tmp/facecheck
/home/user/software/jdk17/bin/javac ExpressionData.java
```

预期：编译通过，生成 `ExpressionData.class`，无警告无错误。

- [ ] **Step 4: 提交**

```bash
cd /home/user/progs/KimiCodeWebApp/.worktrees/grok-face
git add tools/gen_expression_data.py app/src/main/java/com/local/kimiapp/face/ExpressionData.java
git commit -m "feat(face): add GrokBot expression data + generator"
```

预期：`[grok-face-floating-ball <sha>] feat(face): ...` 一行输出。

---

### Task 3: GrokFaceState + GrokFaceView

**Files:**
- Create: `app/src/main/java/com/local/kimiapp/face/GrokFaceState.java`
- Create: `app/src/main/java/com/local/kimiapp/face/GrokFaceView.java`

**Interfaces:**
- Consumes: `ExpressionData.EXPRESSIONS` / `POOLS` / `EXPR_CADENCE` / `BLINK`（Task 2）
- Produces（MainActivity 在 Task 4 使用）:
  - `enum GrokFaceState { SLEEPING, WAKING, IDLE, WORKING, LOADING, CELEBRATE, THINKING, SAD, DROWSY, DRAGGING }`，字段 `public final String gistState`
  - `GrokFaceView(Context)` — 构造
  - `void setFaceState(GrokFaceState)` — 切换状态（表情池/节奏，并选中该状态池第 1 个表情）
  - `GrokFaceState getFaceState()`
  - `void setGaze(float dx, float dy)` — 视线，参数范围 [-1,1]，内部平滑
  - `void blinkNow()` — 手动眨一次
  - `void playTurn(float amplitudeDeg, long durationMs)` — 转头动画（gist spin 公式）
  - `void spinOnce()` — 转一圈（85°，1200ms），celebration 用
  - `void wakeTurn()` — 小幅转头（28°，700ms），唤醒用

- [ ] **Step 1: 写 GrokFaceState.java**

```java
package com.local.kimiapp.face;

/** 悬浮球应用侧状态，映射到 gist 内部状态名（用于查表情池/节奏）。 */
public enum GrokFaceState {
    SLEEPING("sleeping"),
    WAKING("waking"),
    IDLE("idle"),
    WORKING("working"),
    LOADING("loading"),
    CELEBRATE("celebrate"),
    THINKING("thinking"),
    SAD("sad"),
    DROWSY("drowsy"),
    DRAGGING("dragging");

    /** gist 状态名，用于从 ExpressionData 查 POOLS / EXPR_CADENCE / BLINK。 */
    public final String gistState;

    GrokFaceState(String gistState) {
        this.gistState = gistState;
    }
}
```

- [ ] **Step 2: 写 GrokFaceView.java（核心动画引擎）**

```java
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
        canvas.clipOval(CX - BODY_W / 2f, CX - BODY_W / 2f, CX + BODY_W / 2f, CX + BODY_W / 2f);
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
```

- [ ] **Step 3: 编译验证**

```bash
cd /home/user/progs/KimiCodeWebApp/.worktrees/grok-face
JAVA_HOME=/home/user/software/jdk17 /home/user/software/gradle-8.13/bin/gradle compileDebugJavaWithJavac
```

预期：`BUILD SUCCESSFUL`。若报错，优先检查：方法签名与 Task 2 的 ExpressionData 字段名一致（`POOLS`/`EXPR_CADENCE`/`BLINK`/`EXPRESSIONS`）、`Long.MAX_VALUE` 已用 `java.lang`（无需 import）。

- [ ] **Step 4: 提交**

```bash
cd /home/user/progs/KimiCodeWebApp/.worktrees/grok-face
git add app/src/main/java/com/local/kimiapp/face/GrokFaceState.java app/src/main/java/com/local/kimiapp/face/GrokFaceView.java
git commit -m "feat(face): add GrokFaceView animation engine"
```

预期：提交成功一行输出。

---

### Task 4: MainActivity 集成悬浮球

**Files:**
- Modify: `app/src/main/java/com/local/kimiapp/MainActivity.java`

**Interfaces:**
- Consumes:
  - Task 3 的 `GrokFaceState` / `GrokFaceView`（API 见 Task 3 Interfaces）
  - KeepAliveService 发布的 prefs 契约（Task 5 实现，先按此契约写）：`HEALTH_PREFS` 中 `active_count`（int）、`last_event`（String，取值 `complete`/`approval`/`question`/`aborted`）、`last_event_ts`（long）
- Produces: 悬浮球 UI 集成 + 事件监听

- [ ] **Step 1: 加 import 与字段**

在 `MainActivity.java` 顶部 import 区加：

```java
import com.local.kimiapp.face.GrokFaceState;
import com.local.kimiapp.face.GrokFaceView;
```

在字段区（`private WebView webView;` 附近）加：

```java
private GrokFaceView faceView;
private GrokFaceState lastFaceState;
private final android.os.Handler mainHandler = new android.os.Handler();
private final Runnable faceFallback = this::applyBaseFaceState;
```

`faceFallback` 引用了第 4 步的 `applyBaseFaceState()`，需与本任务一起完成（Java 字段初始化顺序：方法引用在运行时才解析，编译期只要求方法存在）。

- [ ] **Step 2: 在 onCreate 注册事件监听**

在 `onCreate` 中 `buildUi();` 之后加一行：

```java
registerFaceEventListeners();
```

- [ ] **Step 3: 改写 installServerHandle()（用 GrokFaceView 替换眼睛）**

将 `installServerHandle(FrameLayout root)` 整体替换为以下版本（保留 FloatingX 拖拽/吸附、点击唤醒/打开列表、3.5s 睡眠、半藏边缘等行为；删除原来的 eyes/eyeViews/blink[0]）：

```java
private void installServerHandle(FrameLayout root) {
    FrameLayout handle = new FrameLayout(this);
    handle.setLayoutParams(new FrameLayout.LayoutParams(dp(56), dp(56)));
    handle.setContentDescription("切换服务器");
    handle.setElevation(dp(8));
    styleServerHandle(handle);
    faceView = new GrokFaceView(this);
    handle.addView(faceView, new FrameLayout.LayoutParams(-1, -1));
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
        handle.addOnLayoutChangeListener((view, left, top, right, bottom,
                oldLeft, oldTop, oldRight, oldBottom) -> view.setSystemGestureExclusionRects(
                Collections.singletonList(new Rect(0, 0, view.getWidth(), view.getHeight()))));
    }

    final IFxScopeControl[] control = new IFxScopeControl[1];
    final boolean[] awake = new boolean[]{false};
    final boolean[] dragged = new boolean[]{false};
    final float[] touchStart = new float[2];
    final long[] touchStartedAt = new long[1];
    final int touchSlop = ViewConfiguration.get(this).getScaledTouchSlop();
    final Runnable[] sleep = new Runnable[1];
    final Runnable[] handleClick = new Runnable[1];
    // 睡眠态：略缩小、降透明度，半藏于屏幕边缘
    handle.setAlpha(0.72f);
    handle.setScaleX(0.94f);
    handle.setScaleY(0.94f);

    sleep[0] = () -> {
        if (control[0] == null || !awake[0]) return;
        awake[0] = false;
        applyFaceState(GrokFaceState.SLEEPING);
        handle.animate().cancel();
        handle.animate().alpha(0.72f).scaleX(0.94f).scaleY(0.94f).setDuration(180).start();
        boolean left = control[0].getX() + dp(28) < root.getWidth() / 2f;
        control[0].move(left ? -dp(28) : root.getWidth() - dp(28), control[0].getY(), false);
    };

    handleClick[0] = () -> {
        handle.removeCallbacks(sleep[0]);
        if (!awake[0]) {
            awake[0] = true;
            faceView.wakeTurn();
            applyFaceState(GrokFaceState.WAKING);
            handle.animate().cancel();
            handle.animate().alpha(1f).scaleX(1f).scaleY(1f).setDuration(240)
                    .setInterpolator(new OvershootInterpolator(1.6f)).start();
            boolean left = control[0].getX() + dp(28) < root.getWidth() / 2f;
            control[0].move(left ? dp(8) : root.getWidth() - dp(64), control[0].getY(), false);
            mainHandler.removeCallbacks(faceFallback);
            mainHandler.postDelayed(faceFallback, 900L);   // WAKING 短暂展示后回落基础状态
            handle.postDelayed(sleep[0], 3500);
        } else {
            showServerList();
            handle.postDelayed(sleep[0], 3500);
        }
    };

    FxScopeHelper helper = FxScopeHelper.builder()
            .setLayoutView(handle)
            .setManagerParams(new FrameLayout.LayoutParams(dp(56), dp(56)))
            .setGravity(FxGravity.RIGHT_OR_CENTER)
            .setEnableEdgeAdsorption(true)
            .setEdgeAdsorbDirection(FxAdsorbDirection.LEFT_OR_RIGHT)
            .setEnableScrollOutsideScreen(true)
            .setHalfHidePercent(0.5f)
            .setEnableAnimation(true)
            .setTouchListener(new IFxTouchListener() {
                @Override public void onDown() { handle.removeCallbacks(sleep[0]); }
                @Override public void onDragIng(MotionEvent event, float x, float y) { }
                @Override public boolean onTouch(MotionEvent event, IFxInternalHelper helper) {
                    if (event.getActionMasked() == MotionEvent.ACTION_DOWN) {
                        touchStart[0] = event.getRawX();
                        touchStart[1] = event.getRawY();
                        touchStartedAt[0] = System.currentTimeMillis();
                        dragged[0] = false;
                    } else if (event.getActionMasked() == MotionEvent.ACTION_MOVE) {
                        boolean wasDragged = dragged[0];
                        dragged[0] = Math.hypot(event.getRawX() - touchStart[0],
                                event.getRawY() - touchStart[1]) >= touchSlop;
                        if (dragged[0]) {
                            if (!wasDragged) {
                                applyFaceState(GrokFaceState.DRAGGING);
                                faceView.setGaze(0f, 0f);
                            }
                            faceView.setGaze(clampUnit((event.getRawX() - touchStart[0]) / dp(70)),
                                    clampUnit((event.getRawY() - touchStart[1]) / dp(70)));
                        }
                    } else if (event.getActionMasked() == MotionEvent.ACTION_UP) {
                        long duration = System.currentTimeMillis() - touchStartedAt[0];
                        if (!dragged[0] && duration <= 600) handleClick[0].run();
                        else if (dragged[0]) {
                            awake[0] = false;
                            faceView.setGaze(0f, 0f);
                            applyFaceState(GrokFaceState.SLEEPING);
                            handle.animate().cancel();
                            handle.animate().alpha(0.72f).scaleX(0.94f).scaleY(0.94f)
                                    .setDuration(180).start();
                        }
                    }
                    return false;
                }
                @Override public boolean onInterceptTouchEvent(MotionEvent event, IFxInternalHelper helper) { return false; }
                @Override public void onUp() {
                    if (!dragged[0] && awake[0]) handle.postDelayed(sleep[0], 3500);
                }
            }).build();
    control[0] = helper.toControl(root);
    control[0].show();
}

private float clampUnit(float v) {
    return Math.max(-1f, Math.min(1f, v));
}
```

- [ ] **Step 4: 加状态解析与事件监听方法**

在 `MainActivity` 类内（`installServerHandle` 之后）加：

```java
private void registerFaceEventListeners() {
    SharedPreferences health = getSharedPreferences(KeepAliveService.HEALTH_PREFS, MODE_PRIVATE);
    health.registerOnSharedPreferenceChangeListener(facePrefsListener);
    applyBaseFaceState();
}

private final SharedPreferences.OnSharedPreferenceChangeListener facePrefsListener =
        (prefs, key) -> {
            if (key == null || faceView == null) return;
            if (key.equals("active_count")) {
                applyBaseFaceState();
            } else if (key.equals("last_event")) {
                long ts = prefs.getLong("last_event_ts", 0L);
                if (System.currentTimeMillis() - ts > 8000L) return;   // 过期事件忽略
                String ev = prefs.getString("last_event", "");
                GrokFaceState s = null;
                if ("complete".equals(ev)) {
                    s = GrokFaceState.CELEBRATE;
                    faceView.spinOnce();
                } else if ("approval".equals(ev) || "question".equals(ev)) {
                    s = GrokFaceState.THINKING;
                } else if ("aborted".equals(ev)) {
                    s = GrokFaceState.SAD;
                }
                if (s != null) {
                    applyFaceState(s);
                    mainHandler.removeCallbacks(faceFallback);
                    mainHandler.postDelayed(faceFallback, 3000L);      // 3s 后回落
                }
            }
        };

private void applyFaceState(GrokFaceState state) {
    if (faceView == null || state == lastFaceState) return;
    lastFaceState = state;
    faceView.setFaceState(state);
}

private void applyBaseFaceState() {
    if (faceView == null) return;
    GrokFaceState s;
    if (webView != null && progress != null && progress.getVisibility() == View.VISIBLE) {
        s = GrokFaceState.LOADING;
    } else {
        SharedPreferences health = getSharedPreferences(KeepAliveService.HEALTH_PREFS, MODE_PRIVATE);
        int active = health.getInt("active_count", 0);
        if (active > 0) {
            s = GrokFaceState.WORKING;
        } else {
            boolean known = false;
            boolean online = false;
            for (ServerStore.Server server : ServerStore.load(this)) {
                if (!health.contains("checked_" + server.id)) continue;
                known = true;
                if (health.getBoolean("online_" + server.id, false)) online = true;
            }
            s = known && !online ? GrokFaceState.DROWSY : GrokFaceState.IDLE;
        }
    }
    applyFaceState(s);
}
```

- [ ] **Step 5: WebView 加载进度驱动 LOADING**

在 `buildUi()` 的 `onProgressChanged` 中，把 `progress.setVisibility(...)` 那行替换为：

```java
@Override public void onProgressChanged(WebView view, int value) {
    progress.setProgress(value);
    boolean loading = value < 100;
    progress.setVisibility(loading ? View.VISIBLE : View.GONE);
    if (loading) applyFaceState(GrokFaceState.LOADING);
    else applyBaseFaceState();
}
```

- [ ] **Step 6: 编译验证**

```bash
cd /home/user/progs/KimiCodeWebApp/.worktrees/grok-face
JAVA_HOME=/home/user/software/jdk17 /home/user/software/gradle-8.13/bin/gradle compileDebugJavaWithJavac
```

预期：`BUILD SUCCESSFUL`。常见错误点：`faceFallback` 字段初始化顺序（引用方法须已定义）、import 缺失、`progress`/`webView` 字段在 `applyBaseFaceState` 中的可空访问（buildUi 先于 registerFaceEventListeners 调用，且 applyBaseFaceState 内已判空）。

- [ ] **Step 7: 提交**

```bash
cd /home/user/progs/KimiCodeWebApp/.worktrees/grok-face
git add app/src/main/java/com/local/kimiapp/MainActivity.java
git commit -m "feat(face): integrate GrokFaceView into floating ball"
```

预期：提交成功一行输出。

---

### Task 5: KeepAliveService 事件发布

**Files:**
- Modify: `app/src/main/java/com/local/kimiapp/KeepAliveService.java`

**Interfaces:**
- Consumes: 无（独立改动）
- Produces: `HEALTH_PREFS` 中新键——`active_count`（int，忙碌会话总数，`updateSummary()` 写入）、`last_event`（String，`complete`/`approval`/`question`/`aborted`）、`last_event_ts`（long，毫秒时间戳）。Task 4 已按此契约消费。

- [ ] **Step 1: updateSummary() 写入 active_count**

在 `updateSummary()` 方法内 `notify(...)` 之后加：

```java
getSharedPreferences(HEALTH_PREFS, MODE_PRIVATE).edit()
        .putInt("active_count", active).apply();
```

- [ ] **Step 2: 加 publishEvent() 辅助方法**

在 `updateSummary()` 方法下方加：

```java
private void publishEvent(String event) {
    getSharedPreferences(HEALTH_PREFS, MODE_PRIVATE).edit()
            .putString("last_event", event)
            .putLong("last_event_ts", System.currentTimeMillis())
            .apply();
}
```

- [ ] **Step 3: handleProtocolEvent status_changed 里发布事件**

在 `handleProtocolEvent` 的 `case "event.session.status_changed"` 分支内，四个 `maybeNotify` 分支各加一行 `publishEvent(...)`。注意这些调用放在 `if (MainActivity.isVisible) return;` **之前**（悬浮球只在应用可见时存在，事件须始终发布）：

```java
if ("running".equals(previous) && "idle".equals(status)) {
    publishEvent("complete");
    maybeNotify(sessionId, eventKey(msg, "status-complete"),
            "Kimi Code · 回合完成", getTitle(sessionId));
} else if ("awaiting_approval".equals(status)) {
    publishEvent("approval");
    maybeNotify(sessionId, eventKey(msg, "status-approval"),
            "Kimi Code · 等待审批", getTitle(sessionId));
} else if ("awaiting_question".equals(status)) {
    publishEvent("question");
    maybeNotify(sessionId, eventKey(msg, "status-question"),
            "Kimi Code · 待回答", getTitle(sessionId));
} else if ("aborted".equals(status)) {
    publishEvent("aborted");
    maybeNotify(sessionId, eventKey(msg, "status-aborted"),
            "Kimi Code · 回合失败", getTitle(sessionId));
}
```

- [ ] **Step 4: work_changed 的 pending 分支发布事件**

在 `case "event.session.work_changed"` 的 `if ("approval".equals(pending)) ... else if ("question".equals(pending))` 两个分支中，各加一行 `publishEvent("approval")` / `publishEvent("question")`（同样放在 isVisible 判断之前）：

```java
if (!MainActivity.isVisible && isFreshEvent(msg)) {
    if (!pending.equals(previousPending)) {
        if ("approval".equals(pending)) {
            publishEvent("approval");
            maybeNotify(sessionId, eventKey(msg, "approval"),
                    "Kimi Code · 等待审批", getTitle(sessionId));
        } else if ("question".equals(pending)) {
            publishEvent("question");
            maybeNotify(sessionId, eventKey(msg, "question"),
                    "Kimi Code · 待回答", getTitle(sessionId));
        }
    }
}
```

- [ ] **Step 5: handleAgentEvent 发布完成/失败事件**

- `prompt.completed` 分支：`notifyTurnFinished(...)` 之前加 `publishEvent("complete");`（放在 `!MainActivity.isVisible` 判断之前）
- `prompt.aborted` 分支：`maybeNotify(...)` 之前加 `publishEvent("aborted");`

```java
} else if ("prompt.completed".equals(type)) {
    publishEvent("complete");
    if (!MainActivity.isVisible && isFreshEvent(msg)) {
        String promptId = payload.optString("promptId", eventKey(msg, "prompt-complete"));
        notifyTurnFinished(sessionId, payload.optString("reason", "completed"),
                "prompt-complete:" + promptId);
    }
} else if ("prompt.aborted".equals(type)) {
    publishEvent("aborted");
    if (!MainActivity.isVisible && isFreshEvent(msg)) {
        String promptId = payload.optString("promptId", eventKey(msg, "prompt-aborted"));
        maybeNotify(sessionId, "prompt-aborted:" + promptId,
                "Kimi Code · 回合失败", getTitle(sessionId));
    }
}
```

- [ ] **Step 6: 编译验证**

```bash
cd /home/user/progs/KimiCodeWebApp/.worktrees/grok-face
JAVA_HOME=/home/user/software/jdk17 /home/user/software/gradle-8.13/bin/gradle compileDebugJavaWithJavac
```

预期：`BUILD SUCCESSFUL`。

- [ ] **Step 7: 提交**

```bash
cd /home/user/progs/KimiCodeWebApp/.worktrees/grok-face
git add app/src/main/java/com/local/kimiapp/KeepAliveService.java
git commit -m "feat(face): publish task events for face states"
```

预期：提交成功一行输出。

---

### Task 6: 构建 APK 与装机验证

**Files:** 无新增/修改。

- [ ] **Step 1: 完整构建**

```bash
cd /home/user/progs/KimiCodeWebApp/.worktrees/grok-face
JAVA_HOME=/home/user/software/jdk17 /home/user/software/gradle-8.13/bin/gradle assembleDebug
```

预期：`BUILD SUCCESSFUL`，产物 `/home/user/progs/KimiCodeWebApp/app/build/outputs/apk/debug/app-debug.apk`。

- [ ] **Step 2: 检查是否有已连接设备**

```bash
adb devices
```

- 有设备：`adb install -r app/build/outputs/apk/debug/app-debug.apk`，请用户目测验证：
  - 点击悬浮球：唤醒转头动画 → 眨眼 → 3.5s 后睡眠（闭眼呼吸 + 半藏边缘）
  - 服务器有任务运行：WORKING 认真脸；回合完成：CELEBRATE 开心 + 转一圈
  - 等待审批/回答：THINKING 思考脸；失败：SAD 难过脸
  - 所有服务器离线：DROWSY 困倦脸
  - 拖拽悬浮球：DRAGGING 表情，视线跟随拖动方向
- 无设备：把 APK 路径交给用户，由用户自行安装（`adb install -r` 或拷贝到手机）。

- [ ] **Step 3: 数据一致性终检**

确认 `ExpressionData.java` 生成于本次 gist 数据（可对照 Task 2 Step 2 的复核输出），`POOLS`/`EXPR_CADENCE`/`BLINK` 包含 10 个状态，无多余状态、无缺失。

---

## Self-Review 记录

- **Spec 覆盖**：25 表情完整移植（Task 2）、弹簧/眨眼/视线/球面旋转（Task 3）、10 状态与事件映射（Task 3/4/5）、56dp 交互不变（Task 4）、工具链与构建验证（Task 1/6）——设计文档各节均有对应任务。
- **占位符检查**：无 TBD/TODO；所有代码步骤含完整代码或精确 diff。
- **类型一致性**：`GrokFaceState` 枚举值、`ExpressionData` 字段名（`EXPRESSIONS`/`POOLS`/`EXPR_CADENCE`/`BLINK`）、`GrokFaceView` 方法签名在 Task 3/4 间一致；prefs 契约（`active_count`/`last_event`/`last_event_ts`）在 Task 4/5 间一致。
