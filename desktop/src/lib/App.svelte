<script lang="ts">
  import { onMount } from "svelte";
  import {
    CREDENTIAL_WARNING,
    chooseExportPath,
    copyTextToClipboard,
    importIssueText,
    importPreviewText,
  } from "../import-export";
  import { emptyServerDraft } from "../model";
  import type {
    AppView,
    ImportMode,
    ImportPreview,
    MeshPeerView,
    ServerDraft,
    SyncOption,
  } from "../model";
  import { hasValidationErrors, validateServerDraft } from "../validation";
  import * as api from "./api";
  import { errorMessage } from "./api";
  import MeshPanel from "./MeshPanel.svelte";
  import { modal } from "./modal";
  import SettingsDialog from "./SettingsDialog.svelte";
  import Sidebar from "./Sidebar.svelte";
  import Stage from "./Stage.svelte";

  type Toast = { id: number; text: string; kind: "info" | "error" | "success" };

  let view = $state<AppView | null>(null);
  let expanded = $state<Set<string>>(new Set());
  let selectedId = $state<string | null>(null);
  let selectedSessionId = $state<string | null>(null);
  let pinned = $state(false);
  let toasts = $state<Toast[]>([]);
  let toastSeq = 0;

  let settingsOpen = $state(false);
  let serverOpen = $state(false);
  let serverTitle = $state("添加服务连接");
  let draft = $state<ServerDraft>(emptyServerDraft());
  let formErrors = $state<Record<string, string>>({});
  let formMessage = $state("");
  let formMessageKind = $state<"success" | "error" | "">("");

  let importOpen = $state(false);
  let importMode = $state<ImportMode>("merge");
  let importError = $state("");
  let pendingImport = $state<ImportPreview | null>(null);

  let pasteOpen = $state(false);
  let pasteContent = $state("");
  let pasteError = $state("");

  let exportOpen = $state(false);
  let exportText = $state("");

  let syncOpen = $state(false);
  let syncOptions = $state<SyncOption[]>([]);
  let syncSelected = $state("");
  let syncStatus = $state("等待手机连接…");
  let syncStatusKind = $state<"success" | "error" | "">("");

  let meshPeers = $state<MeshPeerView[]>([]);
  let meshLoading = $state(false);

  let selectedServer = $derived(view?.servers.find((s) => s.id === selectedId) ?? null);

  function toast(text: string, kind: Toast["kind"] = "info"): void {
    if (!text) return;
    const id = ++toastSeq;
    toasts = [...toasts, { id, text, kind }];
    setTimeout(() => {
      toasts = toasts.filter((t) => t.id !== id);
    }, kind === "error" ? 5200 : 3200);
  }

  function applyView(next: AppView): void {
    view = next;
  }

  // WebView 是原生控件，层级在 DOM 之上；弹窗打开时必须收起，否则会被盖住。
  $effect(() => {
    const overlaid =
      settingsOpen || serverOpen || importOpen || pasteOpen || exportOpen || syncOpen;
    void api.setWebviewOverlaid(overlaid);
  });

  async function selectServer(id: string): Promise<void> {
    selectedId = id;
    selectedSessionId = null;
    try {
      await api.openServer(id, null);
    } catch (error) {
      toast(`打开失败：${errorMessage(error)}`, "error");
    }
  }

  async function selectSession(id: string, sessionId: string): Promise<void> {
    selectedId = id;
    selectedSessionId = sessionId;
    try {
      await api.openServer(id, sessionId);
    } catch (error) {
      toast(`打开失败：${errorMessage(error)}`, "error");
    }
  }

  function toggleExpand(id: string): void {
    const next = new Set(expanded);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    expanded = next;
  }

  async function togglePin(id: string, sessionId: string): Promise<void> {
    try {
      const result = await api.toggleSessionPin(id, sessionId);
      applyView(result.view);
      toast(result.pinned ? "已置顶，完成后会通知你" : "已取消置顶", "success");
    } catch (error) {
      toast(`置顶失败：${errorMessage(error)}`, "error");
    }
  }

  async function editServer(id: string): Promise<void> {
    try {
      draft = await api.getServerForEdit(id);
      serverTitle = "编辑服务连接";
      formErrors = {};
      formMessage = "";
      serverOpen = true;
    } catch (error) {
      toast(`无法读取服务连接：${errorMessage(error)}`, "error");
    }
  }

  async function deleteServer(id: string): Promise<void> {
    const server = view?.servers.find((s) => s.id === id);
    if (!server || !window.confirm(`确定删除“${server.name}”吗？`)) return;
    try {
      applyView(await api.deleteServer(id));
      if (selectedId === id) {
        selectedId = null;
        selectedSessionId = null;
      }
      toast(`已删除“${server.name}”`, "success");
    } catch (error) {
      toast(`删除失败：${errorMessage(error)}`, "error");
    }
  }

  function addServer(): void {
    draft = emptyServerDraft();
    serverTitle = "添加服务连接";
    formErrors = {};
    formMessage = "";
    serverOpen = true;
  }

  async function saveServer(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    const errors = validateServerDraft(draft);
    formErrors = { ...errors };
    formMessage = "";
    if (hasValidationErrors(errors)) return;
    try {
      applyView(await api.saveServer(draft));
      serverOpen = false;
      toast(`已保存“${draft.name}”`, "success");
    } catch (error) {
      formMessage = `保存失败：${errorMessage(error)}`;
      formMessageKind = "error";
    }
  }

  async function probeBackend(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    const errors = validateServerDraft(draft);
    delete errors.name;
    formErrors = { ...errors };
    formMessage = "";
    if (hasValidationErrors(errors)) return;
    const button = event.submitter as HTMLButtonElement | null;
    if (button) button.disabled = true;
    try {
      const backend = await api.probeBackend(draft);
      draft = { ...draft, backend };
      formMessage = `已识别为 ${backend === "kimi" ? "Kimi" : backend === "opencode" ? "OpenCode" : "dsh"}`;
      formMessageKind = "success";
    } catch (error) {
      formMessage = `识别失败，已保留当前选择：${errorMessage(error)}`;
      formMessageKind = "error";
    } finally {
      if (button) button.disabled = false;
    }
  }

  async function refreshMeshPeers(): Promise<void> {
    meshLoading = true;
    try {
      meshPeers = await api.discoverMeshPeers();
    } catch (error) {
      meshPeers = [];
      toast(`发现节点失败：${errorMessage(error)}`, "error");
    } finally {
      meshLoading = false;
    }
  }

  async function handleMeshAction(action: string, host: string): Promise<void> {
    const name = meshPeers.find((p) => p.host === host)?.name ?? host;
    try {
      if (action === "pull") {
        const result = await api.meshPull(host);
        const backup = result.backedUp ? "，旧文件已备份为 config.toml.bak" : "";
        toast(`已从 ${name} 获取 config.toml（${result.bytes} 字节）${backup}`, "success");
      } else if (action === "push") {
        const result = await api.meshPush(host);
        toast(`已将 config.toml（${result.bytes} 字节）发送到 ${name}`, "success");
      } else if (action === "upgrade") {
        toast(`正在升级 ${name} 的 Kimi Code CLI（可能需要数分钟）…`);
        toast(`${name}：${await api.meshKimiUpgrade(host)}`, "success");
        await refreshMeshPeers();
      } else if (action === "restart-web") {
        if (!window.confirm(`确认重启 ${name} 的 kimi web 服务？`)) return;
        try {
          toast(await api.meshKimiWebRestart(host, false), "success");
        } catch (firstError) {
          const reason = errorMessage(firstError);
          if (reason.includes("会话运行中") && window.confirm(`${reason}\n仍要强制重启吗？`)) {
            toast(await api.meshKimiWebRestart(host, true), "success");
          } else {
            toast(`重启已取消：${reason}`);
            return;
          }
        }
        await refreshMeshPeers();
      } else if (action === "remove") {
        if (!view) return;
        applyView(
          await api.updateSettings({
            ...view.settings,
            meshPeers: view.settings.meshPeers.filter((peer) => peer.host !== host),
          }),
        );
        toast("已删除节点", "success");
        await refreshMeshPeers();
      }
    } catch (error) {
      const prefix =
        action === "pull"
          ? "获取配置失败"
          : action === "push"
            ? "发送配置失败"
            : action === "upgrade"
              ? "升级失败"
              : "重启失败";
      toast(`${prefix}：${errorMessage(error)}`, "error");
    }
  }

  async function previewPaste(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    const content = pasteContent.trim();
    if (!content) {
      pasteError = "请先粘贴配置 JSON";
      return;
    }
    try {
      pendingImport = await api.previewImportText(content);
      pasteOpen = false;
      importError = "";
      importOpen = true;
    } catch (error) {
      pasteError = `无法解析导入内容：${errorMessage(error)}`;
    }
  }

  async function applyImport(event: SubmitEvent): Promise<void> {
    event.preventDefault();
    if (!pendingImport) return;
    try {
      applyView(await api.applyImport(pendingImport.importId, importMode));
      importOpen = false;
      pendingImport = null;
      toast("配置导入完成", "success");
    } catch (error) {
      importError = `导入失败：${errorMessage(error)}`;
    }
  }

  async function openExport(): Promise<void> {
    try {
      exportText = await api.exportConfigText();
      exportOpen = true;
    } catch (error) {
      toast(`导出失败：${errorMessage(error)}`, "error");
    }
  }

  async function copyExport(): Promise<void> {
    try {
      await copyTextToClipboard(exportText);
      toast("已复制到剪贴板", "success");
    } catch (error) {
      toast(`复制失败：${errorMessage(error)}`, "error");
    }
  }

  async function saveExport(): Promise<void> {
    try {
      const path = await chooseExportPath();
      if (!path) return;
      await api.exportConfig(path);
      exportOpen = false;
      toast("配置已导出", "success");
    } catch (error) {
      toast(`导出失败：${errorMessage(error)}`, "error");
    }
  }

  async function openSync(): Promise<void> {
    syncOptions = [];
    syncSelected = "";
    syncStatus = "等待手机连接…";
    syncStatusKind = "";
    syncOpen = true;
    try {
      const info = await api.startSyncServer();
      syncOptions = info.options;
      syncSelected = info.selected;
    } catch (error) {
      syncStatus = `启动手机配对服务失败：${errorMessage(error)}`;
      syncStatusKind = "error";
    }
  }

  async function closeSync(): Promise<void> {
    syncOpen = false;
    syncOptions = [];
    syncSelected = "";
    await api.stopSyncServer().catch(() => {
      // 服务可能已因超时自停，忽略。
    });
  }

  function openSettings(): void {
    settingsOpen = true;
    void refreshMeshPeers();
  }

  async function toggleSetting(key: "startHidden" | "autostart" | "notifications"): Promise<void> {
    if (!view) return;
    const previous = view.settings[key];
    const next = { ...view.settings, [key]: !previous };
    try {
      applyView(await api.updateSettings(next));
      toast("设置已更新", "success");
    } catch (error) {
      toast(`设置更新失败：${errorMessage(error)}`, "error");
    }
  }

  async function reconnectAll(): Promise<void> {
    try {
      await api.reconnectAll();
      toast("已请求所有服务立即重连", "success");
    } catch (error) {
      toast(`重连失败：${errorMessage(error)}`, "error");
    }
  }

  async function togglePinned(): Promise<void> {
    const next = !pinned;
    try {
      await api.setAlwaysOnTop(next);
      pinned = next;
    } catch (error) {
      toast(`置顶失败：${errorMessage(error)}`, "error");
    }
  }

  // 订阅后端事件只做一次。这里必须用 onMount 而不是 $effect：$effect 会把
  // 订阅后端事件只做一次。这里必须用 onMount 而不是 $effect：$effect 会把
  // effect 体里读到的状态当成依赖，而 applyView() 会写 view，那样会自触发成
  // 无限循环（表现为白屏 + IPC 风暴）。
  onMount(() => {
    const cleanup: Array<() => void> = [];
    cleanup.push(api.onStateChanged(applyView));
    cleanup.push(
      api.onAgentEvent((payload) => {
        // 手机端配对取走配置后主进程会广播，这里给出反馈。
        if (payload && typeof payload === "object" && "phone-config-fetched" in payload) {
          toast("手机已获取配置", "success");
          if (syncOpen) {
            syncStatus = "手机已获取配置";
            syncStatusKind = "success";
          }
        }
      }),
    );
    void api
      .getAppView()
      .then(applyView)
      .catch((error: unknown) => {
        toast(`无法加载应用状态：${errorMessage(error)}`, "error");
      });
    return () => {
      cleanup.forEach((off) => off());
    };
  });

  let dialogOpen = $derived(
    settingsOpen || serverOpen || importOpen || pasteOpen || exportOpen || syncOpen,
  );
</script>

<div class="app-shell">
  <header class="app-header">
    <div class="brand">
      <!-- 和应用图标/托盘图标同一个"口袋"标记，纯路径绘制，不依赖字体里的符号 -->
      <span class="brand-mark" aria-hidden="true">
        <svg viewBox="0 0 24 24" width="17" height="17"
          ><path
            d="M7 8.5h10a1.5 1.5 0 0 1 1.5 1.5v6a3 3 0 0 1-3 3h-7a3 3 0 0 1-3-3v-6A1.5 1.5 0 0 1 7 8.5Z"
            fill="none"
            stroke="currentColor"
            stroke-width="1.7"
          /><path
            d="M5.8 10.2h12.4"
            fill="none"
            stroke="currentColor"
            stroke-width="1.7"
            stroke-linecap="round"
          /></svg
        >
      </span>
      <div class="brand-copy">
        <h1>AgentPocket</h1>
        <p>编码 Agent 桌面伴侣</p>
      </div>
    </div>
    <nav class="header-actions" aria-label="全局操作">
      <button class="ghost-button" type="button" onclick={() => (pasteOpen = true)}>导入</button>
      <button class="ghost-button" type="button" onclick={openExport}>导出</button>
      <button class="ghost-button" type="button" onclick={openSync}>手机配对</button>
      <button class="ghost-button" type="button" onclick={openSettings}>设置</button>
    </nav>
    <button
      class="pin-toggle"
      class:pin-toggle--on={pinned}
      type="button"
      aria-pressed={pinned}
      aria-label="窗口置顶"
      title="窗口置顶"
      onclick={togglePinned}
    >
      <svg viewBox="0 0 24 24" width="15" height="15" aria-hidden="true"
        ><path
          d="M16 9V4h1c.55 0 1-.45 1-1s-.45-1-1-1H7c-.55 0-1 .45-1 1s.45 1 1 1h1v5c0 1.66-1.34 3-3 3v2h5.97v7l1 1 1-1v-7H19v-2c-1.66 0-3-1.34-3-3z"
          fill="currentColor"
        /></svg
      >
    </button>
  </header>

  <div class="workspace">
    {#if view}
      <Sidebar
        servers={view.servers}
        statuses={view.statuses}
        {expanded}
        {selectedId}
        {selectedSessionId}
        onSelectServer={selectServer}
        onSelectSession={selectSession}
        onToggleExpand={toggleExpand}
        onTogglePin={togglePin}
        onEdit={editServer}
        onDelete={deleteServer}
        onAdd={addServer}
      />
    {/if}
    <Stage
      selectedName={selectedServer?.name ?? null}
      selectedBackend={selectedServer?.backend ?? null}
      hasServers={(view?.servers.length ?? 0) > 0}
    />
  </div>

  <div class="toast-root" aria-live="polite">
    {#each toasts as item (item.id)}
      <div class="toast" data-kind={item.kind}>{item.text}</div>
    {/each}
  </div>
</div>

<SettingsDialog
  open={settingsOpen}
  settings={view?.settings ?? { startHidden: true, autostart: true, notifications: true, meshPeers: [] }}
  peers={meshPeers}
  peersLoading={meshLoading}
  onClose={() => (settingsOpen = false)}
  onToggleSetting={toggleSetting}
  {reconnectAll}
  onMeshAction={handleMeshAction}
  {refreshMeshPeers}
/>

<dialog
  class="modal"
  use:modal={serverOpen}
  aria-labelledby="server-dialog-title"
  onclose={() => (serverOpen = false)}
  onclick={(event) => {
    if (event.target === event.currentTarget) serverOpen = false;
  }}
>
  <form onsubmit={saveServer} novalidate>
    <div class="modal__heading">
      <h2 id="server-dialog-title">{serverTitle}</h2>
      <button class="modal__close" type="button" onclick={() => (serverOpen = false)} aria-label="关闭">×</button>
    </div>
    <div class="form-grid">
      <label class="field field--wide">名称
        <input name="name" autocomplete="off" placeholder="工作站" bind:value={draft.name} />
        {#if formErrors.name}<small class="field-error">{formErrors.name}</small>{/if}
      </label>
      <label class="field field--host">主机 IP / 域名
        <input name="host" autocomplete="off" placeholder="100.64.0.2" bind:value={draft.host} />
        {#if formErrors.host}<small class="field-error">{formErrors.host}</small>{/if}
      </label>
      <label class="field field--port">端口
        <input name="port" type="number" min="1" max="65535" inputmode="numeric" bind:value={draft.port} />
        {#if formErrors.port}<small class="field-error">{formErrors.port}</small>{/if}
      </label>
      <label class="field">Agent 类型
        <select name="backend" bind:value={draft.backend}>
          <option value="dsh">dsh</option>
          <option value="kimi">Kimi</option>
          <option value="opencode">OpenCode</option>
        </select>
        {#if formErrors.backend}<small class="field-error">{formErrors.backend}</small>{/if}
      </label>
      <label class="field">访问令牌
        <input name="token" type="password" autocomplete="new-password" placeholder="可留空" bind:value={draft.token} />
        <small>Kimi 填访问令牌；OpenCode 填登录密码（或 用户名:密码，默认用户 opencode）。仅在编辑时读取，不会出现在服务连接列表中。</small>
      </label>
    </div>
    {#if formMessage}
      <div class="inline-error" data-kind={formMessageKind} role="alert">{formMessage}</div>
    {/if}
    <div class="modal__actions">
      <button class="secondary-button" type="button" formnovalidate onclick={probeBackend}>自动识别</button>
      <span class="modal__spacer"></span>
      <button class="text-button" type="button" onclick={() => (serverOpen = false)}>取消</button>
      <button class="primary-button" type="submit">保存</button>
    </div>
  </form>
</dialog>

<dialog
  class="modal modal--small"
  use:modal={importOpen}
  aria-labelledby="import-dialog-title"
  onclose={() => (importOpen = false)}
  onclick={(event) => {
    if (event.target === event.currentTarget) importOpen = false;
  }}
>
  {#if pendingImport}
    <form onsubmit={applyImport}>
      <div class="modal__heading"><h2 id="import-dialog-title">确认导入</h2></div>
      <p class="modal__summary">{importPreviewText(pendingImport)}</p>
      {#if importIssueText(pendingImport).length > 0}
        <ul class="issue-list">
          {#each importIssueText(pendingImport) as issue (issue)}
            <li>{issue}</li>
          {/each}
        </ul>
      {/if}
      <fieldset class="mode-picker">
        <legend>导入方式</legend>
        <label><input type="radio" bind:group={importMode} value="merge" /> 合并：同 ID 覆盖，其余保留</label>
        <label><input type="radio" bind:group={importMode} value="replace" /> 替换：清除现有服务连接</label>
      </fieldset>
      {#if importError}<div class="inline-error" role="alert">{importError}</div>{/if}
      <div class="modal__actions">
        <span class="modal__spacer"></span>
        <button class="text-button" type="button" onclick={() => (importOpen = false)}>取消</button>
        <button class="primary-button" type="submit" disabled={pendingImport.validCount === 0}>导入</button>
      </div>
    </form>
  {/if}
</dialog>

<dialog
  class="modal modal--small"
  use:modal={pasteOpen}
  aria-labelledby="paste-dialog-title"
  onclose={() => (pasteOpen = false)}
  onclick={(event) => {
    if (event.target === event.currentTarget) pasteOpen = false;
  }}
>
  <form onsubmit={previewPaste}>
    <div class="modal__heading">
      <h2 id="paste-dialog-title">粘贴导入</h2>
      <button class="modal__close" type="button" onclick={() => (pasteOpen = false)} aria-label="关闭">×</button>
    </div>
    <p class="modal__summary">把复制好的 JSON 配置粘贴到下方。</p>
    <textarea
      class="code-area"
      rows="10"
      spellcheck="false"
      placeholder={'[{"name":"工作站","host":"100.64.0.2","port":3080,"backend":"dsh"}]'}
      aria-label="粘贴的配置 JSON"
      bind:value={pasteContent}
    ></textarea>
    {#if pasteError}<div class="inline-error" role="alert">{pasteError}</div>{/if}
    <div class="modal__actions">
      <span class="modal__spacer"></span>
      <button class="text-button" type="button" onclick={() => (pasteOpen = false)}>取消</button>
      <button class="primary-button" type="submit">预览导入</button>
    </div>
  </form>
</dialog>

<dialog
  class="modal"
  use:modal={exportOpen}
  aria-labelledby="export-dialog-title"
  onclose={() => (exportOpen = false)}
  onclick={(event) => {
    if (event.target === event.currentTarget) exportOpen = false;
  }}
>
  <div class="modal__heading">
    <h2 id="export-dialog-title">导出配置</h2>
    <button class="modal__close" type="button" onclick={() => (exportOpen = false)} aria-label="关闭">×</button>
  </div>
  <p class="credential-warning">{CREDENTIAL_WARNING}</p>
  <p class="modal__summary">直接复制下方 JSON，或另存为文件。</p>
  <textarea class="code-area" rows="12" readonly spellcheck="false" aria-label="导出的配置 JSON" value={exportText}></textarea>
  <div class="modal__actions">
    <button class="secondary-button" type="button" onclick={copyExport}>复制</button>
    <span class="modal__spacer"></span>
    <button class="text-button" type="button" onclick={() => (exportOpen = false)}>取消</button>
    <button class="primary-button" type="button" onclick={saveExport}>选择保存位置</button>
  </div>
</dialog>

<dialog
  class="modal modal--small"
  use:modal={syncOpen}
  aria-labelledby="sync-dialog-title"
  onclose={closeSync}
  onclick={(event) => {
    if (event.target === event.currentTarget) void closeSync();
  }}
>
  <div class="modal__heading">
    <h2 id="sync-dialog-title">手机配对</h2>
    <button class="modal__close" type="button" onclick={closeSync} aria-label="关闭">×</button>
  </div>
  {#if syncOptions.length > 0}
    <select class="sync-address" aria-label="对外地址" bind:value={syncSelected}>
      {#each syncOptions as option (option.address)}
        <option value={option.address}>{option.label}:{option.address}</option>
      {/each}
    </select>
    {@const active = syncOptions.find((option) => option.address === syncSelected)}
    {#if active}
      <div class="sync-qr">{@html active.qrSvg}</div>
      <p class="sync-url">{active.url}</p>
    {/if}
  {/if}
  <p class="modal__summary">手机需要能访问所选地址：同一 Tailnet 选 Tailscale 地址，同一 Wi-Fi 可选局域网地址。</p>
  <p class="sync-status" data-kind={syncStatusKind}>{syncStatus}</p>
  <div class="modal__actions">
    <span class="modal__spacer"></span>
    <button class="text-button" type="button" onclick={closeSync}>关闭</button>
  </div>
</dialog>

<span class="sr-only" data-dialog-open={dialogOpen} aria-hidden="true"></span>