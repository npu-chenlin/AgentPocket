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
    Backend,
    ImportMode,
    ImportPreview,
    MeshPeerView,
    ServerDraft,
    ServerStatus,
    SyncOption,
  } from "../model";
  import { hasValidationErrors, validateServerDraft } from "../validation";
  import * as api from "./api";
  import { errorMessage } from "./api";
  import LauncherPanel from "./LauncherPanel.svelte";
  import AgentWorkspace from "./AgentWorkspace.svelte";
  import type { WorkspaceController } from "./workspace-controller";
  import type { WorkspaceTab } from "../../shared/workspace";
  import { modal } from "./modal";
  import SettingsDialog from "./SettingsDialog.svelte";
  import { baseFaceState, faceReactionForEvent, faceSignalsFromServers, type FaceState } from "./face/expression-state";
  type Toast = { id: number; text: string; kind: "info" | "error" | "success" };

  let view = $state<AppView | null>(null);
  let expanded = $state<Set<string>>(new Set());
  let controller: WorkspaceController | null = null;
  let tabs = $state<WorkspaceTab[]>([]);
  let activeTabId = $state<string | null>(null);
  let pinned = $state(false);
  let launcherOpen = $state(false);
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

  function openServerIn(_paneId: string | null, serverId: string, sessionId: string | null): void {
    const name = view?.servers.find(s => s.id === serverId)?.name ?? serverId;
    controller?.open(serverId, sessionId, sessionId ? `${name} · ${sessionId}` : name);
  }

  function openInNewPane(id: string, sessionId: string | null): void {
    controller?.open(id, sessionId, view?.servers.find(s => s.id === id)?.name ?? id, "right");
  }

  function statusForServer(id: string): ServerStatus | undefined {
    return view?.statuses[id];
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

  async function probeBackend(event: MouseEvent): Promise<void> {
    event.preventDefault();
    const errors = validateServerDraft(draft);
    delete errors.name;
    formErrors = { ...errors };
    formMessage = "";
    if (hasValidationErrors(errors)) return;
    const button = event.currentTarget as HTMLButtonElement | null;
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

  async function handleMeshAction(action: string, host: string, note = ""): Promise<void> {
    const name = meshPeers.find((p) => p.host === host)?.name ?? host;
    try {
      if (action === "refresh") {
        await refreshMeshPeers();
      } else if (action === "add") {
        if (!view) return;
        const address = host.trim();
        if (!address || /[\s/:]/.test(address)) throw new Error("请填写裸主机名或 IPv4 地址");
        applyView(await api.updateSettings({ ...view.settings, meshPeers: [
          ...view.settings.meshPeers.filter(peer => peer.host !== address),
          { host: address, name: note.trim() || address },
        ] }));
        await refreshMeshPeers();
      } else if (action === "pull") {
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
  // effect 体里读到的状态当成依赖，而 applyView() 会写 view，那样会自触发成
  // 无限循环（表现为白屏 + IPC 风暴）。
  onMount(() => {
    const cleanup: Array<() => void> = [];


    cleanup.push(api.onStateChanged(applyView));

    // 悬浮球在独立窗口里，它点一下要通过主进程转达"开合面板"。
    cleanup.push(api.onBallToggle(() => (launcherOpen = !launcherOpen)));

    // 原生菜单栏的动作。菜单项在主进程，这里接到渲染进程已有的函数上。
    cleanup.push(
      api.onMenuAction((action) => {
        switch (action) {
          case "import":
            pasteOpen = true;
            break;
          case "export":
            void openExport();
            break;
          case "phone-pair":
            void openSync();
            break;
          case "settings":
            openSettings();
            break;
          case "toggle-launcher":
            launcherOpen = !launcherOpen;
            break;
          case "toggle-always-on-top":
            void togglePinned();
            break;
          default:
            // 认不出的动作直接忽略：菜单与渲染进程版本不一致时不该把界面搞坏。
            break;
        }
      }),
    );
    cleanup.push(
      api.onAgentEvent((payload) => {
        if (payload && typeof payload === "object" && "ball-error" in payload) {
          toast(`悬浮球不可用，请使用工具栏或菜单打开面板：${String(payload["ball-error"])}`, "error");
        }
        // 手机端配对取走配置后主进程会广播，这里给出反馈。
        if (payload && typeof payload === "object" && "phone-config-fetched" in payload) {
          toast("手机已获取配置", "success");
          if (syncOpen) {
            syncStatus = "手机已获取配置";
            syncStatusKind = "success";
          }
        }
        // 任务完成/等待介入时让悬浮球表态。
        if (payload && typeof payload === "object" && "kind" in payload) {
          reactToFaceEvent(String((payload as { kind: unknown }).kind));
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

  let activeServerId = $derived(tabs.find(t => t.id === activeTabId)?.serverId ?? null);
  let baseFace = $derived(baseFaceState(faceSignalsFromServers({
    loading: !view, servers: view?.servers ?? [], statuses: view?.statuses ?? {},
  })));
  let eventFace = $state<FaceState | null>(null);
  let faceState = $derived(eventFace ?? baseFace);
  let spinToken = $state(0);
  let faceTimer: ReturnType<typeof setTimeout>;
  $effect(() => { void api.publishBallState({ face: faceState, open: launcherOpen, spin: spinToken }); });
  onMount(() => () => clearTimeout(faceTimer));

  function reactToFaceEvent(event: string | null | undefined): void {
    const reaction = faceReactionForEvent(event);
    if (!reaction) return;
    clearTimeout(faceTimer);
    eventFace = reaction.state;
    if (reaction.spin) spinToken += 1;
    faceTimer = setTimeout(() => { eventFace = null; }, 3000);
  }
</script>

<div class="app-shell">
  <div class="workspace" style="display:flex;flex-direction:column">
    <AgentWorkspace servers={view?.servers ?? null} overlaid={dialogOpen || launcherOpen}
      onReady={(value) => { controller = value; }}
      onChange={(next, active) => { tabs = next; activeTabId = active; }}
      onError={(message) => toast(message, "error")} onOpen={() => (launcherOpen = true)} />
  </div>

  {#if launcherOpen}
    <!--
      点空白处收起面板。面板打开时 overlay 已让主进程隐藏原生网页
      （可见 workspace-controller.setOverlay），所以整片内容区的点击
      都能落到这层 DOM 上；否则会被 WebContentsView 吃掉。
      放在面板之前，靠 DOM 顺序保证点面板本身不会命中外层。
    -->
    <div
      class="launcher-scrim"
      role="presentation"
      onclick={() => (launcherOpen = false)}
    ></div>
    <LauncherPanel
      servers={view?.servers ?? []}
      statuses={view?.statuses ?? {}}
      activeServerId={activeServerId}
      onOpen={(id, sessionId) => {
        launcherOpen = false;
        void openServerIn(null, id, sessionId);
      }}
      onOpenInNewPane={(id, sessionId) => {
        launcherOpen = false;
        openInNewPane(id, sessionId);
      }}
      onAdd={addServer}
      onEdit={editServer}
      onDelete={deleteServer}
      onTogglePin={togglePin}
      onClose={() => (launcherOpen = false)}
    />
  {/if}

  <div class="toast-root" aria-live="polite">
    {#each toasts as item (item.id)}
      <div class="toast" data-kind={item.kind}>{item.text}</div>
    {/each}
  </div>
</div>

<SettingsDialog
  open={settingsOpen}
  settings={view?.settings ?? { startHidden: false, autostart: true, notifications: true, meshPeers: [] }}
  peers={meshPeers}
  peersLoading={meshLoading}
  onClose={() => (settingsOpen = false)}
  onToggleSetting={toggleSetting}
  onReconnectAll={reconnectAll}
  onMeshAction={handleMeshAction}
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
        <small>Kimi 填访问令牌；dsh 填 <code>~/.dsh/.credentials.yaml</code> 里的 browser-session 密钥；OpenCode 填登录密码（或 用户名:密码，默认用户 opencode）。仅在编辑时读取，不会出现在服务连接列表中。</small>
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