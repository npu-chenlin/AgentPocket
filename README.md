# KimiWeb

一个面向 Kimi Code Web 的非官方轻量 Android 客户端，适合通过 Tailscale 从手机安全访问电脑上的 Kimi Code。

## 使用前提

1. 在电脑上安装并启动 Kimi Code Web。
2. 在电脑和 Android 手机上安装 Tailscale，并登录同一个 Tailnet。
3. 启动 Kimi Code Web 时允许网络访问并启用 token 鉴权。
4. 将电脑的 Tailscale IP、Kimi Code Web 端口和 token 填入 App。

例如：

```shell
kimi web --host 0.0.0.0 --port 58627
```

Kimi Code Web 通常会给出包含 token 的访问地址。可将完整地址直接粘贴进 App，例如：

```text
http://100.x.y.z:58627/#token=YOUR_TOKEN
```

其中 `100.x.y.z` 应替换为运行 Kimi Code Web 的电脑在 Tailscale 中的 IP。请勿将 Kimi Code Web 直接暴露到不受信任的公网。

## 功能

- 使用 WebView 打开自托管的 Kimi Code Web
- 一次配置 IP、端口与访问 token
- 可直接粘贴 `http://IP:PORT/#token=...` 或 Kimi 启动输出并自动识别
- 原生后台监听任务状态
- 回合完成、待回答、等待审批和失败通知
- 状态栏颜色跟随网页明暗主题

## 安装

从 [Releases](../../releases) 下载最新的 `KimiWeb-*.apk`。所有版本使用同一发布证书签名，可直接覆盖升级。

## 使用

首次启动时可以分别输入主机、端口与 token，也可以直接粘贴 Kimi Web 给出的完整地址。之后可从“Kimi 后台监听”常驻通知中的“连接设置”修改。

## 说明

本项目是非官方客户端，与 Moonshot AI/Kimi 官方无隶属关系。使用前请确保 Kimi Web 服务仅暴露在可信网络，或启用 bearer token 鉴权。

## License

[MIT](LICENSE)
