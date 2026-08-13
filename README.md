# KimiWeb

一个非官方 Android 客户端，通过 Tailscale 在手机上使用电脑中的 [Kimi Code](https://github.com/MoonshotAI/kimi-code) Web，并在后台接收任务状态通知。

<p align="center">
  <img src="docs/demo.jpg" alt="KimiWeb 使用效果" width="52%">
  <img src="docs/notify-preview.jpg" alt="KimiWeb 后台任务通知" width="26%">
</p>

<p align="center">
  <img src="docs/model-selection.jpg" alt="Kimi Code 模型选择" width="52%">
  <img src="docs/multi-server.jpg" alt="KimiWeb 多服务器选择" width="26%">
</p>

## 使用前提

1. 在电脑和 Android 手机上安装 [Tailscale](https://tailscale.com/download)，并登录同一个 Tailnet。
2. 在电脑上安装 Kimi Code：

   macOS / Linux：

   ```shell
   curl -fsSL https://code.kimi.com/kimi-code/install.sh | bash
   ```

   Windows PowerShell：

   ```powershell
   irm https://code.kimi.com/kimi-code/install.ps1 | iex
   ```

3. 启动 Kimi Code Web，并允许网络访问。
4. 在 KimiWeb 中添加服务器并连接。

例如：

```shell
kimi web --dangerous-bypass-auth --host 0.0.0.0 --port 58627
```

此参数会关闭访问认证，请仅在可信的 Tailscale 网络中使用。随后将电脑的 Tailscale IP 和端口添加到 KimiWeb。

## 功能

- 在 Android 上使用 Kimi Code Web
- 管理多个服务器并显示在线状态
- 后台监听任务状态，接收完成、待回答、审批和失败通知

## 使用

首次启动时添加服务器。之后可通过屏幕侧边的悬浮入口切换或管理服务器；后台会同时监听所有已添加的服务器。

## 说明

本项目是非官方客户端，与 Moonshot AI/Kimi 官方无隶属关系。请仅在可信网络中使用。

## License

[MIT](LICENSE)
