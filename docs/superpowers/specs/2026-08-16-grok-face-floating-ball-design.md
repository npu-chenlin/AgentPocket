# GrokFace 悬浮球 — 设计文档

- 日期：2026-08-16
- 项目：KimiCodeWebApp（KimiWeb，非官方 Android 客户端）
- 状态：已确认

## 背景与目标

KimiWeb 主界面右上角有一个 56dp 的悬浮球（`MainActivity.installServerHandle()`），用于切换/管理服务器。当前它是蓝渐变圆形 + 两只简单白眼睛（scaleY 眨眼动画）。

用户提供了一个交互原型 gist（"Atelier GrokBot"），内含一套完整的 SVG 头像表情引擎：25 个表情（每只眼睛 48 个路径点）、弹簧物理插值、两段式眨眼、视线偏移、球面投影旋转、按状态自动切换表情池与节奏的状态机。

目标：把悬浮球升级为"小 Grok 头像"，完整移植该表情引擎的动画能力，并将应用真实事件（会话忙碌、任务完成/失败、等待审批/回答、服务器在线状态、页面加载、拖拽）映射为表情状态。

## 范围

**做：**

- 完整移植 25 个表情的路径数据（2 只眼睛 × 48 点）
- 弹簧插值表情切换、两段式眨眼、视线偏移、球面投影旋转
- 10 个应用侧状态：SLEEPING / WAKING / IDLE / WORKING / LOADING / CELEBRATE / THINKING / SAD / DROWSY / DRAGGING
- 事件接线：KeepAliveService 事件 → 悬浮球状态
- 保留现有 FloatingX 拖拽吸附、点击唤醒/打开服务器列表的交互模型

**不做（YAGNI）：**

- 不做 gist 里的实验室 UI（滑块、tab、形状目录等）
- 不做形状切换（blob/pebble 等 18 种身体形状），身体固定为现有蓝渐变圆形
- 不做"双尺寸动态缩放"（用户已确认保持 56dp）
- 不新增依赖库，不用 Lottie / WebView

## 架构

### 新增文件（`app/src/main/java/com/local/kimiapp/face/`）

1. **`ExpressionData.java`** — 静态数据常量：
   - `EXPRESSIONS`：`float[25][2][48][2]`（25 个表情 × 左右眼 × 48 点 × x/y）
   - `POOLS` / `EXPR_CADENCE` / `BLINK`：按 gist 状态名保存的表情池索引、表情切换节奏（ms 区间）、眨眼节奏（ms 区间），只需保留本设计用到的状态
   - 由 `tools/gen_expression_data.py` 从 gist HTML 中的 JS 数据生成，脚本一并提交以便重新生成

2. **`GrokFaceState.java`** — 应用侧状态枚举（10 个），每个状态映射到 gist 内部状态名（从而取得 POOLS/CADENCE/BLINK），并携带一次性事件标志（如 CELEBRATE 是短暂状态）

3. **`GrokFaceView.java`** — 自定义 View：
   - `onDraw`：绘制身体（蓝渐变椭圆 + 底部内阴影 + 顶部高光，沿用现有渐变配色）+ 两只眼睛（Path，白色填充）
   - Choreographer.FrameCallback 驱动动画循环：弹簧积分、眨眼、视线、球面旋转、自动表情切换、idle 呼吸
   - 公共 API：`setState(GrokFaceState)`、`setGaze(float dx, float dy)`、`wake()`、`sleep()`、`spinOnce()`、`setLoading(boolean)`

### 修改文件

1. **`MainActivity.java`**：
   - `installServerHandle()` 中用 `GrokFaceView` 替换原来的眼睛 LinearLayout；FloatingX 的拖拽/吸附/点击逻辑保留
   - 点击逻辑保持不变：睡眠态点击 → 唤醒；唤醒态点击 → 打开服务器列表
   - 新增 SharedPreferences 监听（复用 `showServerList()` 里已有的 health prefs 监听模式）接收 KeepAliveService 发布的运行状态，驱动 `GrokFaceView.setState()`
   - WebView 加载进度驱动 LOADING 状态
   - 拖拽时驱动 DRAGGING 状态 + 视线跟随拖拽方向

2. **`KeepAliveService.java`**：
   - 事件发布通道：向 `HEALTH_PREFS` 写入 `active_count`（忙碌会话数）和 `last_event`（`complete` / `approval` / `question` / `aborted` / `online` / `offline`，带时间戳）
   - 在已有的 WebSocket 事件处理点（`handleProtocolEvent` / `handleAgentEvent` / `setHealth` / `updateSummary`）写入上述字段，不改变现有通知逻辑

## 动画引擎规格（照搬 gist）

坐标系：gist 的 viewBox 为 229×229（身体直径 228.5），渲染时整体缩放适配 View 尺寸。

### 弹簧插值（表情 morph）

```
velocity += (-2 * freq * velocity - freq * freq * (morph - 1)) * dt
morph += velocity * dt
```

- `freq` 默认 7.0（gist 滑块范围 4–12，取默认值即可，不暴露 UI）
- `dt` 取帧间隔，上限 0.1s
- morph 非有限数时重置为 1、velocity 为 0（NaN 防护）
- 切换表情：`current = target`（把当前插值结果固定为起点）、`morph = 0`，弹簧向 1 归位；绘制时按 `morph` 在 current 与 target 之间线性混合每个点

### 眨眼（320ms 两段式）

```
t = (now - blinkStart) / 320
scaleY = t < 0.42 ? 1 - t/0.42 : (t - 0.42) / 0.58   // 夹取 [0.04, 1]
```

- 眨眼节奏由当前状态的 `BLINK` 区间随机取（sleeping 不眨眼）
- 手动眨眼入口 `blinkNow()`

### 视线偏移

- gazeX ∈ [-1, 1] → 像素偏移 ×13.2；gazeY ×8.4（229 坐标系，随 View 缩放）

### 球面投影旋转

每只眼睛独立计算：

```
c = centroid(ring)
baseLongitude = asin(clamp((c.x - 114.2705) / 105, -1, 1))
longitude = baseLongitude + turn
depth = cos(longitude)
perspective = max(depth, 0.02) / max(cos(baseLongitude), 0.02)
x = 114.2705 + 105 * sin(longitude) + gazeX
y = c.y + gazeY
sx = clamp(perspective * baseScale, 0.02, 2.4)
sy = clamp(blinkScale * baseScale, 0.02, 2.4)
depth <= 0.02 时眼睛淡出
```

绘制变换：`translate(x, y) scale(sx, sy) translate(-c.x, -c.y)`，再叠加到眼睛路径上。

### 自动表情

- 按状态 `EXPR_CADENCE` 区间随机定时，从 `POOLS` 中随机挑一个与当前不同的表情
- sleeping 用 13 号闭眼表情，waking 用 13 → 0
- idle 在表情 0/8 间缓慢切换（gist 节奏 9–16s），加轻微呼吸缩放

## 状态机与事件映射

| 应用状态 | 触发条件 | gist 内部状态 | 说明 |
|---|---|---|---|
| SLEEPING | 无操作 3.5s | sleeping | 闭眼、半透明、缩小、半藏边缘 |
| WAKING | 点击唤醒 | waking | 小幅转头 + 睁眼 → IDLE |
| IDLE | 无忙碌会话、非加载 | idle | 缓慢表情 + 呼吸 |
| WORKING | `active_count > 0` | working | 认真脸，节奏较快 |
| LOADING | WebView 加载中 | loading | |
| CELEBRATE | `last_event = complete` | celebrate | 开心 + 转一圈，几秒后回落 |
| THINKING | `last_event = approval/question` | thinking | 思考脸 |
| SAD | `last_event = aborted` | sad | 难过几秒后回落 |
| DROWSY | 所有服务器离线 | drowsy | 困倦脸 |
| DRAGGING | 手指拖动 | dragging | 视线跟随拖动方向 |

优先级：DRAGGING > 一次性事件（CELEBRATE/THINKING/SAD）> LOADING > WORKING > DROWSY > IDLE > SLEEPING。

一次性事件状态（CELEBRATE/THINKING/SAD）在短暂展示（约 3s，可被打断）后回落至按 `active_count` 与服务器健康度计算的基础状态：有忙碌会话 → WORKING，否则 → IDLE 或 DROWSY。

## 交互模型（保留现有行为）

- 睡眠态：alpha 0.72、scale 0.94、半藏屏幕边缘（FloatingX `move()`）
- 点击：未唤醒 → 唤醒（alpha 1.0、scale 1.0、OvershootInterpolator 1.6、从边缘滑出）→ 3.5s 后回睡眠；已唤醒 → 打开服务器列表
- 拖拽：超 touchSlop 判定为拖动，拖动中进入 DRAGGING，松手回原状态，不触发点击
- 唤醒期间眼睛可见 + 随机眨眼；睡眠期间闭眼 + 慢呼吸

## 数据生成

`tools/gen_expression_data.py`：

- 输入：gist 原始 HTML（`grokbot-demo/index.html`，位于本机 /home/user/grokbot-demo/，脚本支持传入路径）
- 输出：`ExpressionData.java`（`float[][][][]` 常量 + 状态映射常量）
- 生成后人工核对表达式数量（应为 25）与 gist 数据一致

## 性能与健壮性

- Choreographer 循环只在 View 附加时运行（`onDetachedFromWindow` 停止，防泄漏）
- 画布极小（56dp），连续 60fps 开销可接受；SLEEPING 状态降频渲染（约 4fps）省电
- 弹簧 NaN 防护（见动画规格）
- 表达式数据为编译期常量，无运行时解析开销

## 验证

1. `./gradlew assembleDebug` 编译通过（环境需 Android SDK；如环境无 SDK 则如实报告并给出本机构建指引）
2. 真机安装目测：
   - 点击唤醒/睡眠动画、眨眼、表情切换
   - 让服务器有任务运行时看 WORKING/CELEBRATE 表情
   - 断网看 DROWSY
3. 数据一致性：`ExpressionData` 中的表达式数量、POOLS/CADENCE 与 gist 数据逐项核对

## 不做的事（再次强调）

- 不暴露任何 UI 控件（无 gist 的实验室界面）
- 不做形状切换
- 不加新依赖
