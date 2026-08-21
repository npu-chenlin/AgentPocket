# Android

Android 客户端适合在手机上访问电脑中的 Agent 服务，并在任务需要关注时回到会话。

## 能力

- 内嵌 Kimi Code Web 和 DeepSeek Harness Web。
- 添加、编辑、删除和排序多个服务连接；可选择 Kimi 或 dsh 后端。
- 后台监听已保存服务连接的状态，接收完成、失败、等待回答和等待审批通知。
- 从通知或活跃会话入口返回对应会话。
- 使用屏幕侧边悬浮入口切换服务连接；悬浮界面包含状态和表情反馈。
- 通过扫码与 Desktop 互传服务连接列表。

## 添加服务连接

1. 让手机与运行 Agent 服务的电脑登录同一个 Tailnet。
2. 确认 Agent 服务监听在手机可访问的地址；dsh 需要额外的本地转发时，先完成转发。
3. 在 App 中填写名称、主机、端口、后端和可选 token。
4. 使用服务连接打开 Web；手机端沿用 Agent 服务已有的会话。

服务连接保存的是访问信息，不是节点配置。导入导出或扫码时，内容可能包含 token。

## dsh 限制

Android 当前不支持新建 dsh 工作区。请在电脑浏览器中创建工作区，然后从手机使用已有会话。dsh 模型调用仍需在 Agent 服务所在电脑配置 `DEEPSEEK_API_KEY` 或在其 Models 页面配置凭据。

## 构建

需要 Android SDK、JDK 和项目要求的 Gradle/Android Gradle Plugin 环境：

```shell
gradle assembleDebug
gradle assembleRelease
```

产物位于 `app/build/outputs/apk/`。Release 签名由根目录的 `keystore.properties`（若存在）配置；不要把 keystore 或凭据提交到仓库。
