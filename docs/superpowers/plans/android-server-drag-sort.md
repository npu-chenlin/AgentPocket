# Android server list long-press drag reorder

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development.

**Goal:** 在 `MainActivity.showServerList()` 弹出的服务器列表对话框中支持长按后拖拽排序，并把新顺序持久化到 `ServerStore`。

**Architecture:** 将现有程序化构建的 `LinearLayout` 卡片列表替换为 `RecyclerView` + `ItemTouchHelper`，长按条目即可拖动，松手后保存顺序。

## Global Constraints

- 必须保持现有视觉样式与行为：点击卡片切换服务器、编辑、删除、复制配置、刷新页面、添加服务器。
- 排序完成后立即调用 `ServerStore.save(context, servers, activeId)` 持久化。
- 只允许新增 `androidx.recyclerview` 一个依赖。
- 不得破坏 `ServerStore` 的 JSON 存储格式与 `activeId` 语义。
- 目标 `compileSdk 35`、`minSdk 26`。

## Task 1: Convert server list dialog to RecyclerView with drag reorder

**Files:**
- Modify: `app/build.gradle`
- Modify: `app/src/main/java/com/local/kimiapp/MainActivity.java`

**Interfaces:**
- Produces: `MainActivity.ServerListAdapter` 内部类（含 `ViewHolder`）。
- Produces: `ItemTouchHelper` 拖拽回调，拖动方向 `UP | DOWN`，不支持侧滑。

- [ ] **Step 1: Add RecyclerView dependency**

在 `app/build.gradle` 的 `dependencies` 块末尾追加：

```gradle
    implementation 'androidx.recyclerview:recyclerview:1.3.2'
```

- [ ] **Step 2: Replace the card loop with RecyclerView**

在 `showServerList()` 中：
1. 保留外层 `list`（`LinearLayout`）作为对话框根视图。
2. 创建一个垂直方向的 `RecyclerView`，设置 `setHasFixedSize(true)`，`LayoutManager` 为 `LinearLayoutManager(this)`。
3. 移除原来 `for (ServerStore.Server server : servers)` 构建卡片的循环代码，改为把服务器列表、当前 `activeId`、`health` SharedPreferences 传给 `ServerListAdapter`。
4. 把 `RecyclerView` 和底部操作按钮（刷新/复制/添加）一起加到 `list` 中。

- [ ] **Step 3: Implement ServerListAdapter**

内部类 `ServerListAdapter` 在 `onCreateViewHolder` 中程序化构建与现有样式一致的卡片视图：
- 左侧后端 logo（`backendIconView`），离线/未知时置灰；
- 中间垂直文本（名称 + `host:port`）；
- 若当前服务器被选中，显示 "当前" badge；
- 右侧编辑、删除图标按钮；
- 卡片背景、圆角、描边、Ripple 效果保持与原来一致；
- 卡片高度固定 `dp(72)`，底部 margin `dp(10)`。

点击事件：
- 卡片整体 → `dialog.dismiss(); switchServer(server.id);`
- 编辑按钮 → `dialog.dismiss(); showConfig(false, server, true);`
- 删除按钮 → `dialog.dismiss(); deleteServer(server);`

- [ ] **Step 4: Attach ItemTouchHelper for drag reorder**

实现 `ItemTouchHelper.SimpleCallback`：
- `getMovementFlags`：拖动 `ItemTouchHelper.UP | ItemTouchHelper.DOWN`，侧滑 `0`。
- `onMove`：交换 adapter 数据列表中的两个位置，调用 `notifyItemMoved`，然后以当前 `activeId` 调用 `ServerStore.save(context, reorderedList, activeId)`。
- `isLongPressDragEnabled` 返回 `true`（默认即可）。

- [ ] **Step 5: Build and verify**

运行：

```bash
./gradlew :app:assembleDebug
```

Expected: 编译成功，无新增弃用警告。

- [ ] **Step 6: Commit**

```bash
git add app/build.gradle app/src/main/java/com/local/kimiapp/MainActivity.java
git commit -m "feat: long-press drag to reorder server list"
```
