# KimiWeb

一个面向 Kimi Code Web 的轻量 Android 客户端。

## 功能

- 使用 WebView 打开自托管的 Kimi Code Web
- 一次配置 IP、端口与访问 token
- 可直接粘贴 `http://IP:PORT/#token=...` 或 Kimi 启动输出并自动识别
- 原生后台监听任务状态
- 回合完成、待回答、等待审批和失败通知
- 状态栏颜色跟随网页明暗主题

## 安装

从 [Releases](../../releases) 下载最新的 `KimiWeb-*.apk`。所有版本必须使用同一发布证书签名，才能直接覆盖升级。

## 构建

需要 JDK 17 和 Android SDK：

```shell
./gradlew assembleDebug
```

发布签名配置保存在本机 `keystore.properties`，私钥和密码不会提交到仓库。

## 使用

首次启动时可以分别输入主机、端口与 token，也可以直接粘贴 Kimi Web 给出的完整地址。之后可从“Kimi 后台监听”常驻通知中的“连接设置”修改。

## 说明

本项目是非官方客户端，与 Moonshot AI/Kimi 官方无隶属关系。使用前请确保 Kimi Web 服务仅暴露在可信网络，或启用 bearer token 鉴权。

## License

[MIT](LICENSE)
