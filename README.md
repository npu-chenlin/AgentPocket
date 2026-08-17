# AgentPocket

一个非官方 Android 客户端，通过 Tailscale 在手机上使用电脑中的编码 Agent Web 服务（Kimi Code、DeepSeek Harness 等），并在后台接收任务状态通知。

<p align="center">
  <img src="docs/demo.jpg" alt="AgentPocket 使用效果" width="52%">
  <img src="docs/notify-preview.jpg" alt="AgentPocket 后台任务通知" width="26%">
</p>

<p align="center">
  <img src="docs/model-selection.jpg" alt="模型选择" width="52%">
  <img src="docs/multi-server.jpg" alt="AgentPocket 多服务器选择" width="26%">
</p>

## 使用前提

1. 在电脑和 Android 手机上安装 [Tailscale](https://tailscale.com/download)，并登录同一个 Tailnet。
2. 在电脑上安装任一种编码 Agent 的 Web 服务：

   **Kimi Code**（macOS / Linux）：

   ```shell
   curl -fsSL https://code.kimi.com/kimi-code/install.sh | bash
   ```

   Windows PowerShell：

   ```powershell
   irm https://code.kimi.com/kimi-code/install.ps1 | iex
   ```

   **DeepSeek Harness**（需要 Node.js 18+）：

   ```shell
   npx -y @deepseek-ai/dsh web --trusted-host <Tailscale IP>
   ```

   国内网络可换镜像源：

   ```shell
   npx -y --registry=https://registry.npmmirror.com @deepseek-ai/dsh web --trusted-host <Tailscale IP>
   ```

3. 启动 Web 服务并允许网络访问。

   Kimi Code：

   ```shell
   kimi web --dangerous-bypass-auth --host 0.0.0.0 --port 58627
   ```

   此参数会关闭访问认证，请仅在可信的 Tailscale 网络中使用。

   DeepSeek Harness（dsh）**官方禁止绑定 `0.0.0.0`**（安全原因），只监听 `127.0.0.1`。
   手机访问需要在电脑上用一个本地反向代理，把请求从 Tailscale IP 转发到 `127.0.0.1:3080`（socat 即可，一行命令）：

   ```shell
   socat TCP-LISTEN:3080,bind=<Tailscale IP>,reuseaddr,fork TCP:127.0.0.1:3080
   ```

   `--trusted-host` 用于放行手机经反代访问时的 Host 头（`<Tailscale IP>:3080`），与上面的 socat 必须配套使用。

   dsh 模型调用需要配置 API Key，二选一：

   - 启动前 `export DEEPSEEK_API_KEY=...`
   - 在电脑浏览器打开 `http://127.0.0.1:3080`，在 Models 页面写入凭据

4. 在 AgentPocket 中添加服务器并连接。

## 功能

- 在 Android 上使用 Kimi Code Web 与 DeepSeek Harness Web
- 管理多个服务器并显示在线状态，各服务器可混用不同后端（logo 颜色标识在线/离线）
- 添加服务器时自动识别后端类型（粘贴启动信息即可，也可手动指定）
- 后台监听任务状态，接收完成、待回答、审批和失败通知

## 使用

首次启动时添加服务器。可以直接粘贴电脑上 Web 服务的启动输出，app 会自动识别地址、端口与后端类型：

- Kimi：粘贴 `kimi web --dangerous-bypass-auth ...` 启动命令或完整 URL
- dsh：在 app 中填写 `http://<Tailscale IP>:3080`（经 socat 反代后的地址），类型会自动探测为 DeepSeek Harness

之后可通过屏幕侧边的悬浮入口切换或管理服务器；后台会同时监听所有已添加的服务器。

### DeepSeek Harness 注意事项

- dsh 无 token 鉴权，靠 Host 头信任围栏：默认只信任本机回环地址；手机经 socat 反代访问时 Host 头是 `<Tailscale IP>:3080`，必须用 `--trusted-host <Tailscale IP>` 显式放行，两者配套使用。
- dsh 官方禁止 `--host 0.0.0.0`（防止把远程代码执行暴露到网络），**不要尝试**在启动命令里加这个参数。
- `settings.*`、`credentials.*` 等特权接口仅允许本机回环地址调用，手机端无法在网页中修改模型配置（请在电脑 localhost 上操作）。
- 手机端**不支持新建工作区**：创建新工作区需调用 `host.pickDirectory`（打开电脑原生目录选择器），属特权接口，手机端调用会返回 403。请在电脑浏览器中创建，手机端可正常使用已有会话与工作区。

## 说明

本项目是非官方客户端，与 Moonshot AI/Kimi、DeepSeek 官方无隶属关系。请仅在可信网络中使用。

## License

[MIT](LICENSE)
