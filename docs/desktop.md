# Desktop

Desktop 是一个 Electron 托盘应用，适合在电脑上常驻监控多个 Agent 服务，并管理服务连接和节点。

> **为什么是 Electron 而不是 Tauri**：桌面端最关键的能力是把 Agent 的 Web UI 嵌进主窗口并
> 精确定位。Tauri 在 Linux 上做不到——上游 tauri-apps/tauri#10420 从 2024-07-30 起至今未实现，
> `tauri-runtime-wry` 把子 webview 塞进纵向 `gtk::Box`，wry 的 `add_to_container` 走 `pack_start`
> 会丢弃 position/size，`set_bounds` 仅在 `is_in_fixed_parent` 时生效，而 tao 全仓从不创建
> `GtkFixed`。Electron 的 `WebContentsView.setBounds` 是官方支持且实测精确生效的。代价是安装包
> 体积更大。

## 能力

- 托盘常驻，关闭窗口后继续监听。**默认开启开机自启**（可在设置或托盘菜单里关掉）。
- 同时监控多个 Kimi/dsh/OpenCode Agent 服务，在悬浮球里显示在线状态、版本号和活跃会话数。
- 分屏同时看多个 Agent 页面，窗格可拖拽重排。
- 任务完成、失败或中断、等待审批、等待回答时发送系统通知；可从托盘或主窗口在应用内打开服务/会话。
- 添加、编辑、删除服务连接；导入/导出与 Android 兼容的统一服务连接配置。
- 生成二维码与 Android 做手机配对 / 服务连接同步。
- 节点面板发现 AgentPocket 节点，查看 Kimi CLI 和 Kimi Web 状态，获取/发送 Kimi 配置，升级 CLI，并按需重启 Kimi Web。

## 界面结构

窗口里没有标题栏，内容区直接从顶边开始。窗口级操作全在**原生菜单栏**：

| 菜单 | 内容 |
| --- | --- |
| 文件 | 导入服务连接…、导出服务连接…、手机配对…、设置、退出 |
| 编辑 | 撤销/重做/剪切/复制/粘贴/全选 |
| 视图 | 显示/隐藏悬浮球面板、窗口置顶、重新加载、开发者工具、缩放、全屏 |
| 窗口 | 最小化、关闭 |
| 帮助 | 项目主页 |

- **没有配置任何快捷键**。菜单 accelerator 与系统默认编辑快捷键并存时，焦点落在输入框里也会抢键。
- 菜单项本身不含业务逻辑：点击后只向渲染进程发一个动作名（`menu:action`），由那边调用已有的函数。
  这样同一件事不会在主进程和渲染进程各写一遍。
- 菜单模板放在 `electron/main/menu-template.ts`，不依赖 Electron 运行时，
  所以 `electron/main/menu.test.ts` 能直接断言菜单结构——少配一项就是功能凭空消失，
  而界面上看不出任何异样。
- 「窗口置顶」是普通动作而非勾选项：菜单在启动时构建一次，不随置顶状态重建，
  显示一个可能过期的对勾比不显示更容易误导。

## 悬浮球

界面里没有侧边栏。窗口右侧是一个常驻的**悬浮球**，它同时承担打开面板和状态指示两个角色：

- 球面绘制与表情动画是 Android 端 `GrokFaceView` 的移植，坐标系、弹簧插值、眨眼、
  球面投影与呼吸全部照搬（`src/lib/face/`）。数据由 `tools/gen_expression_data_ts.py`
  从 `ExpressionData.java` 生成，**不要手改** `expression-data.ts`。
- 表情反映整体状态：有会话在跑 → `WORKING`，全部离线 → `DROWSY`，否则 `IDLE`；
  任务完成/等待审批/中断会让它临时切到 `CELEBRATE`/`THINKING`/`SAD`，3 秒后回落。
- 拖动可换位置，松手吸附到最近的左右边缘；位置与分屏布局都存 localStorage——
  纯界面偏好，不进 `config.json`，也不该同步给 daemon 和手机端。
- 3.5 秒无操作进入睡眠态（缩小、变淡）；点一下唤醒，再点开合悬浮球面板。

悬浮球面板有两个标签页：

- **服务连接**：点一行在活动窗格打开；⧉ 在新窗格打开；可编辑、删除、添加。
- **活跃会话**：汇总所有服务的忙碌会话，排序为「置顶运行中 → 置顶已完成 → 其余」。
  已完成但仍置顶的会话留在最上面，提醒你介入。点一行打开该会话。

## 分屏工作区

内容区可以并排多个 Agent 页面：

- 布局是一棵 N 叉树（`src/lib/workspace.ts`），渲染交给 [PaneForge](https://paneforge.com)：
  同方向的多次分裂会并成兄弟节点，所以「三个并列」就是一个三子组，不会套出多余的层级。
- 拖动窗格到另一个窗格的四条边即可重排；同方向会被目标所在的组直接吸收。
- 关闭窗格时兄弟自动顶替，组里只剩一个子节点就塌缩——不会留下半个空框。
- 服务被删除后，指向它的窗格会一起消失。
- 比例由 PaneForge 自己维护，结构由 AgentPocket 维护（`agentpocket.workspace`）。

## 内嵌 Agent 视图

打开服务或会话时，Agent 页面会嵌在对应的窗格里，而不是跳到系统浏览器：

- **每个服务一个 `WebContentsView`**，矩形与可见性按服务独立，分屏时多个视图同时可见。
  同一服务开在多个窗格里会**共用同一个视图**（显示同一页面）——否则同一页面要加载多份，
  内存翻倍且状态互相不一致。
- 前端用 CSS 决定布局，把各窗格的矩形按服务 id 汇总后上报给主进程；主进程据此调用
  `WebContentsView.setBounds`，并只对发生变化的视图下发。坐标系与 DOM 视口一致，
  原生只跟随，不自己算布局——所以窗口移动/缩放不需要额外同步。
- 视图只允许导航到当前服务的 origin，站外链接交给系统浏览器打开。
- 渲染进程与 Agent 视图都启用了 `contextIsolation`；Agent 视图跑在 `sandbox: true` 下且拿不到
  我们的 preload，因此远程页面无法调用 AgentPocket 的任何命令。
- 每个服务用独立的持久化 session（`persist:server-<id>`），cookie 与凭据互不串台；
  代价是 HTTP 缓存也各自一份。修改服务凭据会重建该视图，避免继续用旧 cookie。


### 凭据处理

- **Kimi**：访问令牌通过 URL fragment 传递，由前端消费，不走网络。
- **dsh**：把 `dsh web` 打印的启动行粘进 token 栏即可。客户端用其中的 launch token 向
  dsh 换取浏览器会话 cookie（`GET /?token=…` → `Set-Cookie`），保存的是这个 cookie。
  cookie 由 dsh 的持久密钥签发、30 天有效且跨 dsh 重启有效；过期或换了访问地址后，
  重新粘一次启动行。客户端不读取也不复刻 `~/.dsh/.credentials.yaml` 里的签名密钥。
- **OpenCode v2**：页面与 API 都要求 HTTP Basic。Electron 在 `session.webRequest.onBeforeSendHeaders`
  里为该服务的 origin 注入 `Authorization` 头。

  这里**没有**用 `app.on('login')`：那只在主框架导航收到 401 挑战时触发，而 OpenCode 的页面本身是
  静态的（能正常加载），真正 401 的是它随后发起的 `/api/*` 请求——XHR/fetch 的 401 不会走到
  login 事件。只注册 login 的症状恰好是「页面打开了但一直卡在 Connect to a server」。

  旧版 Tauri 为此写了一个 522 行的本机回环 HTTP 代理，Electron 下不再需要。

- 每个服务用独立的 session（partition 为 `persist:server-<id>`）：cookie 不会在服务之间串台，
  登录状态也能跨重启保留。注入前会检查请求是否已带 `Authorization`，避免用户改密码后被本地旧值钉住。

## 服务连接

添加连接时，主机只填写域名或 IP，不包含 `http://`、端口或路径。示例：

- Kimi：主机为 Tailscale IP，端口 `58627`
- dsh：主机为 Tailscale IP，端口 `3080`（或你设置的转发端口）
- OpenCode：主机为 Tailscale IP，端口 `4096`

“同步”指手机配对和服务连接同步；节点面板中的“拉取/推送”指节点间 Kimi 配置分发，两者不要混用。

## 安装与构建

Linux 需要 Node.js 22。Electron 自带运行时，除此之外只需构建工具：

```shell
sudo apt install build-essential
```

开发版：

```shell
cd desktop
npm ci
npm run dev
```

测试和构建：

```shell
npm test        # vitest
npm run build   # tsc --noEmit && electron-vite build
npm run dist    # electron-vite build && electron-builder
```

产物位于 `desktop/release/`（Linux 为 AppImage 与 `deb`，macOS 为 `dmg`，Windows 为 `nsis`）。
macOS 未配置 Developer ID 时生成的是临时签名、未公证的本地构建包。Windows 与 macOS 尚未进行正式发布验证。

`deb` 会额外做两件事：把 `dist/agentpocket-x86_64-linux-musl` 打进包内
（`/opt/AgentPocket/resources/agentpocket`），并由 `build/after-install.sh` 安装到 `/usr/bin/agentpocket`；
同时用 `Conflicts`/`Replaces` 顶掉 Tauri 时代的旧包名 `agent-pocket`。因此打 `deb` 前需要先跑
`scripts/build-daemon.sh` 生成那个二进制。该脚本整体替换了 electron-builder 的内置模板，
改动时记得同步上游模板（`app-builder-lib/templates/linux/after-install.tpl`）。

打包时需要下载**两类**东西，各自的镜像变量不同，漏掉后一类会卡在从 GitHub 直连
拉 120 MB 的 electron 运行时（表现为长时间无进展）：

| 下载内容 | 环境变量 |
| --- | --- |
| electron 运行时本体 | `ELECTRON_MIRROR` |
| electron-builder 的工具（fpm、appimage、7zip） | `ELECTRON_BUILDER_BINARIES_MIRROR` |

```shell
ELECTRON_MIRROR=https://registry.npmmirror.com/-/binary/electron/ \
ELECTRON_BUILDER_BINARIES_MIRROR=https://registry.npmmirror.com/-/binary/electron-builder-binaries \
npm run dist
```

配置目录默认沿用旧版路径 `~/.local/share/com.local.agentpocket.desktop`，这样从 Tauri 迁移后
已有的服务连接可直接使用；也可用 `AGENTPOCKET_CONFIG_DIR` 覆盖。

## 已知限制

- **重连后活动行会清空**。忙碌状态能用权威端点重建（opencode 的
  `/api/session/active`），但“正在跑什么工具”没有对应端点，只能靠事件实时累积。
  Kimi 与 opencode 都有这个特性。
- **dsh 不显示版本号**。版本采集只覆盖 Kimi（`/api/v1/meta` 的 `data.server_version`）
  与 opencode（`/api/info` 的 `version`）。dsh 没有对应的版本端点。取不到的服务
  不留空位，所以界面上表现为没有版本号而不是显示空值。
- **opencode 不处理审批通知**。该服务的权限是全开的，`Permission.Effect` 的 `ask`
  分支不会触发，因此不会有「等待审批」通知。提问（`form.*`）是支持的。

## 安全提示

Desktop 配置和导出文件包含服务连接凭据，请勿公开分享。节点面板使用 Tailnet 内的 AgentPocket 端点；节点端点无独立鉴权，信任边界是你的 Tailnet。