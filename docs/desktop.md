# Desktop

Desktop 是一个 Tauri 托盘应用，适合在电脑上常驻监控多个 Agent 服务，并管理服务连接和节点。

## 能力

- 托盘常驻，关闭窗口后继续监听。
- 同时监控多个 Kimi/dsh Agent 服务，显示在线状态、版本和活跃会话。
- 任务完成、失败或中断、等待审批、等待回答时发送系统通知；可从托盘或主窗口在浏览器中打开服务/会话。
- 添加、编辑、删除服务连接；导入/导出与 Android 兼容的统一服务连接配置。
- 生成二维码与 Android 做手机配对 / 服务连接同步。
- 节点面板发现 AgentPocket 节点，查看 Kimi CLI 和 Kimi Web 状态，获取/发送 Kimi 配置，升级 CLI，并按需重启 Kimi Web。

## 服务连接

添加连接时，主机只填写域名或 IP，不包含 `http://`、端口或路径。示例：

- Kimi：主机为 Tailscale IP，端口 `58627`
- dsh：主机为 Tailscale IP，端口 `3080`（或你设置的转发端口）

“同步”指手机配对和服务连接同步；节点面板中的“拉取/推送”指节点间 Kimi 配置分发，两者不要混用。

## 安装与构建

Linux 需要 Node.js 22、Rust stable，以及 Tauri 所需的 WebKitGTK、AppIndicator、OpenSSL 和构建工具。Ubuntu 22.04 可安装：

```shell
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

开发版：

```shell
cd desktop
npm ci
npm run tauri dev
```

测试和构建：

```shell
npm test
npm run tauri build
```

若只产出二进制（不打安装包），必须启用 `custom-protocol`：

```shell
npm run build && cargo build --release --features tauri/custom-protocol
```

不带该 feature 的二进制是开发模式：它不会嵌入前端资源，启动后会尝试加载 `localhost:1420`；未运行开发服务器时将出现连接失败。

产物位于 `desktop/src-tauri/target/release/bundle/`。macOS DMG 可使用 `--bundles dmg` 构建；未配置 Developer ID 时生成的是临时签名、未公证的本地构建包。Windows 与 macOS 尚未进行正式发布验证。

## 安全提示

Desktop 配置和导出文件包含服务连接凭据，请勿公开分享。节点面板使用 Tailnet 内的 AgentPocket 端点；节点端点无独立鉴权，信任边界是你的 Tailnet。
