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
- 同时监控多个 Kimi/dsh/OpenCode Agent 服务，在侧边栏显示在线状态、版本号和活跃会话数。
- 任务完成、失败或中断、等待审批、等待回答时发送系统通知；可从托盘或主窗口在应用内打开服务/会话。
- 添加、编辑、删除服务连接；导入/导出与 Android 兼容的统一服务连接配置。
- 生成二维码与 Android 做手机配对 / 服务连接同步。
- 节点面板发现 AgentPocket 节点，查看 Kimi CLI 和 Kimi Web 状态，获取/发送 Kimi 配置，升级 CLI，并按需重启 Kimi Web。

## 界面结构

窗口里没有标题栏，内容区直接从顶边开始，省下 56px 垂直空间。窗口级操作全在**原生菜单栏**：

| 菜单 | 内容 |
| --- | --- |
| 文件 | 导入服务连接…、导出服务连接…、手机配对…、设置、退出 |
| 编辑 | 撤销/重做/剪切/复制/粘贴/全选 |
| 视图 | 折叠/展开侧边栏、窗口置顶、重新加载、开发者工具、缩放、全屏 |
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

侧边栏可折叠（折叠后 52px，只留图标、任务数角标和展开按钮），折叠状态存在 localStorage——
这是纯界面偏好，不进 `config.json`，也不该同步给 daemon 和手机端。

## 内嵌 Agent 视图

点击服务或会话时，Agent 页面会嵌在主窗口的内容区里，而不是跳到系统浏览器：

- 每个服务一个 `WebContentsView`，切换时只改可见性不重建，页面状态因此得以保留。
- 前端用 CSS 决定布局，把内容区矩形上报给主进程；主进程据此调用 `WebContentsView.setBounds`。
  坐标系与 DOM 视口一致，原生只跟随，不自己算布局——所以窗口移动/缩放不需要额外同步。
- 视图只允许导航到当前服务的 origin，站外链接交给系统浏览器打开。
- 渲染进程与 Agent 视图都启用了 `contextIsolation`；Agent 视图跑在 `sandbox: true` 下且拿不到
  我们的 preload，因此远程页面无法调用 AgentPocket 的任何命令。

### 凭据处理

- **Kimi**：访问令牌通过 URL fragment 传递，由前端消费，不走网络。
- **dsh**：无令牌。
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

应用图标与托盘图标由 `resources/make-icons.py` 生成（纯几何绘制，不依赖外部素材），改动后重跑即可：

```shell
python3 resources/make-icons.py
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