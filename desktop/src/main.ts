import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  CREDENTIAL_WARNING,
  chooseExportPath,
  copyTextToClipboard,
  importIssueText,
  importPreviewText,
} from "./import-export";
import {
  commands,
  emptyServerDraft,
  type AppView,
  type ImportMode,
  type ImportPreview,
  type ServerDraft,
  type ServerForEdit,
  type SyncInfo,
  type SyncOption,
} from "./model";
import { renderServerList } from "./server-list";
import {
  hasValidationErrors,
  validateServerDraft,
  type ServerValidationErrors,
} from "./validation";
import "./styles.css";

const app = document.querySelector<HTMLElement>("#app");

if (!app) {
  throw new Error("missing #app root");
}

app.innerHTML = `
  <div class="app-shell">
    <header class="app-header">
      <div class="brand">
        <span class="brand-mark" aria-hidden="true">⌁</span>
        <div class="brand-copy">
          <h1>AgentPocket</h1>
          <p>编码 Agent 桌面伴侣</p>
        </div>
      </div>
      <nav class="header-actions" aria-label="全局操作">
        <button id="paste-import" class="ghost-button" type="button">导入</button>
        <button id="open-export" class="ghost-button" type="button">导出</button>
        <button id="open-sync" class="ghost-button" type="button">同步</button>
        <button id="open-settings" class="ghost-button" type="button">设置</button>
      </nav>
      <button id="pin-window" class="pin-toggle" type="button" aria-pressed="false" aria-label="窗口置顶" title="窗口置顶">
        <svg viewBox="0 0 24 24" width="15" height="15" aria-hidden="true"><path d="M16 9V4h1c.55 0 1-.45 1-1s-.45-1-1-1H7c-.55 0-1 .45-1 1s.45 1 1 1h1v5c0 1.66-1.34 3-3 3v2h5.97v7l1 1 1-1v-7H19v-2c-1.66 0-3-1.34-3-3z" fill="currentColor"/></svg>
      </button>
    </header>

    <main class="content">
      <section class="servers-section" aria-labelledby="servers-title">
        <div class="section-heading">
          <h2 id="servers-title">服务器</h2>
          <button id="add-server" class="primary-button" type="button">添加服务器</button>
        </div>
        <div id="server-list" class="server-list" aria-live="polite"></div>
      </section>
    </main>

    <div id="toast-root" class="toast-root" aria-live="polite"></div>
  </div>

  <dialog id="settings-dialog" class="modal modal--small">
    <div class="modal__heading">
      <h2>设置</h2>
      <button class="modal__close" type="button" data-close-dialog="settings-dialog" aria-label="关闭">×</button>
    </div>
    <div class="settings-body">
      <label class="setting-row">
        <span><strong>开机启动</strong><small>登录系统后自动启动托盘</small></span>
        <input type="checkbox" role="switch" data-setting="autostart" />
      </label>
      <label class="setting-row">
        <span><strong>启动时隐藏</strong><small>直接在系统托盘中运行</small></span>
        <input type="checkbox" role="switch" data-setting="startHidden" />
      </label>
      <label class="setting-row">
        <span><strong>系统通知</strong><small>仅通知完成、失败、审批和提问</small></span>
        <input type="checkbox" role="switch" data-setting="notifications" />
      </label>
    </div>
    <div class="modal__actions">
      <button id="reconnect-all" class="secondary-button" type="button">重新连接所有服务器</button>
      <span class="modal__spacer"></span>
      <button class="text-button" type="button" data-close-dialog="settings-dialog">完成</button>
    </div>
  </dialog>

  <dialog id="server-dialog" class="modal">
    <form id="server-form" method="dialog" novalidate>
      <div class="modal__heading">
        <h2 id="server-dialog-title">添加服务器</h2>
        <button class="modal__close" type="button" data-close-dialog="server-dialog" aria-label="关闭">×</button>
      </div>
      <input type="hidden" name="id" />
      <div class="form-grid">
        <label class="field field--wide">名称<input name="name" autocomplete="off" placeholder="工作站" /><small class="field-error" data-error-for="name"></small></label>
        <label class="field field--host">主机地址<input name="host" autocomplete="off" placeholder="100.64.0.2" /><small class="field-error" data-error-for="host"></small></label>
        <label class="field field--port">端口<input name="port" type="number" min="1" max="65535" inputmode="numeric" /><small class="field-error" data-error-for="port"></small></label>
        <label class="field">后端<select name="backend"><option value="dsh">dsh</option><option value="kimi">Kimi</option></select><small class="field-error" data-error-for="backend"></small></label>
        <label class="field">访问令牌<input name="token" type="password" autocomplete="new-password" placeholder="可留空" /><small>令牌仅在编辑时读取，不会出现在服务器列表中。</small></label>
      </div>
      <div id="form-error" class="inline-error" role="alert"></div>
      <div class="modal__actions">
        <button id="probe-backend" class="secondary-button" type="button">自动识别</button>
        <span class="modal__spacer"></span>
        <button class="text-button" type="button" data-close-dialog="server-dialog">取消</button>
        <button id="save-server" class="primary-button" type="submit">保存</button>
      </div>
    </form>
  </dialog>

  <dialog id="import-dialog" class="modal modal--small">
    <form id="import-form" method="dialog">
      <div class="modal__heading"><h2>确认导入</h2></div>
      <p id="import-summary" class="modal__summary"></p>
      <ul id="import-issues" class="issue-list"></ul>
      <fieldset class="mode-picker">
        <legend>导入方式</legend>
        <label><input type="radio" name="import-mode" value="merge" checked /> 合并：同 ID 覆盖，其余保留</label>
        <label><input type="radio" name="import-mode" value="replace" /> 替换：清除现有服务器</label>
      </fieldset>
      <div id="import-error" class="inline-error" role="alert"></div>
      <div class="modal__actions">
        <span class="modal__spacer"></span>
        <button class="text-button" type="button" data-close-dialog="import-dialog">取消</button>
        <button id="apply-import" class="primary-button" type="submit">导入</button>
      </div>
    </form>
  </dialog>

  <dialog id="paste-dialog" class="modal modal--small">
    <form id="paste-form" method="dialog">
      <div class="modal__heading">
        <h2>粘贴导入</h2>
        <button class="modal__close" type="button" data-close-dialog="paste-dialog" aria-label="关闭">×</button>
      </div>
      <p class="modal__summary">把复制好的 JSON 配置粘贴到下方。</p>
      <textarea id="paste-content" class="code-area" rows="10" spellcheck="false" placeholder='[{"name":"工作站","host":"100.64.0.2","port":3080,"backend":"dsh"}]'></textarea>
      <div id="paste-error" class="inline-error" role="alert"></div>
      <div class="modal__actions">
        <span class="modal__spacer"></span>
        <button class="text-button" type="button" data-close-dialog="paste-dialog">取消</button>
        <button id="preview-paste" class="primary-button" type="submit">预览导入</button>
      </div>
    </form>
  </dialog>

  <dialog id="export-dialog" class="modal">
    <div class="modal__heading">
      <h2>导出配置</h2>
      <button class="modal__close" type="button" data-close-dialog="export-dialog" aria-label="关闭">×</button>
    </div>
    <p class="credential-warning">${CREDENTIAL_WARNING}</p>
    <p class="modal__summary">直接复制下方 JSON，或另存为文件。</p>
    <textarea id="export-text" class="code-area" rows="12" readonly spellcheck="false"></textarea>
    <div class="modal__actions">
      <button id="copy-export" class="secondary-button" type="button">复制</button>
      <span class="modal__spacer"></span>
      <button class="text-button" type="button" data-close-dialog="export-dialog">取消</button>
      <button id="confirm-export" class="primary-button" type="button">选择保存位置</button>
    </div>
  </dialog>

  <dialog id="sync-dialog" class="modal modal--small">
    <div class="modal__heading">
      <h2>手机同步</h2>
      <button class="modal__close" type="button" data-close-dialog="sync-dialog" aria-label="关闭">×</button>
    </div>
    <select id="sync-address" class="sync-address" aria-label="对外地址"></select>
    <div id="sync-qr" class="sync-qr"></div>
    <p id="sync-url" class="sync-url"></p>
    <p class="modal__summary">手机需要能访问所选地址：同一 Tailnet 选 Tailscale 地址，同一 Wi-Fi 可选局域网地址。</p>
    <p id="sync-status" class="sync-status">等待手机连接…</p>
    <div class="modal__actions">
      <span class="modal__spacer"></span>
      <button class="text-button" type="button" data-close-dialog="sync-dialog">关闭</button>
    </div>
  </dialog>`;

function requiredElement<T extends Element>(selector: string): T {
  const element = document.querySelector<T>(selector);
  if (!element) throw new Error(`missing element: ${selector}`);
  return element;
}

const serverList = requiredElement<HTMLElement>("#server-list");
const toastRoot = requiredElement<HTMLElement>("#toast-root");
const settingsDialog = requiredElement<HTMLDialogElement>("#settings-dialog");
const serverDialog = requiredElement<HTMLDialogElement>("#server-dialog");
const serverForm = requiredElement<HTMLFormElement>("#server-form");
const importDialog = requiredElement<HTMLDialogElement>("#import-dialog");
const importForm = requiredElement<HTMLFormElement>("#import-form");
const pasteDialog = requiredElement<HTMLDialogElement>("#paste-dialog");
const pasteForm = requiredElement<HTMLFormElement>("#paste-form");
const pasteContent = requiredElement<HTMLTextAreaElement>("#paste-content");
const pasteError = requiredElement<HTMLElement>("#paste-error");
const exportDialog = requiredElement<HTMLDialogElement>("#export-dialog");
const exportText = requiredElement<HTMLTextAreaElement>("#export-text");
const copyExport = requiredElement<HTMLButtonElement>("#copy-export");
const syncDialog = requiredElement<HTMLDialogElement>("#sync-dialog");
const syncAddress = requiredElement<HTMLSelectElement>("#sync-address");
const syncQr = requiredElement<HTMLElement>("#sync-qr");
const syncUrl = requiredElement<HTMLElement>("#sync-url");
const syncStatus = requiredElement<HTMLElement>("#sync-status");

let currentView: AppView | null = null;
let latestRevision = -1;
/** 展开运行中会话面板的服务器 id；重绘时保持展开状态。 */
const expandedServers = new Set<string>();
let pendingImport: ImportPreview | null = null;
let unlistenState: UnlistenFn | undefined;
let unlistenPhoneReceived: UnlistenFn | undefined;
let unlistenPhoneFetched: UnlistenFn | undefined;

/** 以浮动 Toast 提示操作结果；空文本视为清除，直接忽略。 */
function setMessage(text: string, kind: "info" | "error" | "success" = "info"): void {
  if (!text) return;
  const toast = document.createElement("div");
  toast.className = "toast";
  toast.dataset.kind = kind;
  toast.textContent = text;
  toastRoot.appendChild(toast);
  const lifetime = kind === "error" ? 5200 : 3200;
  window.setTimeout(() => {
    toast.classList.add("toast--leaving");
    toast.addEventListener("transitionend", () => toast.remove(), { once: true });
  }, lifetime);
}

function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  try {
    return JSON.stringify(error);
  } catch {
    return "发生未知错误";
  }
}

/** data-setting 开关只覆盖布尔型设置项（meshPeers 等非布尔项不渲染开关）。 */
type BooleanSettingKey = "startHidden" | "autostart" | "notifications";

function applyView(view: AppView): void {
  if (view.revision <= latestRevision) return;
  latestRevision = view.revision;
  currentView = view;
  serverList.innerHTML = renderServerList(view, expandedServers);
  for (const control of document.querySelectorAll<HTMLInputElement>("[data-setting]")) {
    const key = control.dataset.setting as BooleanSettingKey;
    control.checked = view.settings[key];
  }
}

function formControl<T extends HTMLInputElement | HTMLSelectElement>(name: string): T {
  const control = serverForm.elements.namedItem(name);
  if (!(control instanceof HTMLInputElement) && !(control instanceof HTMLSelectElement)) {
    throw new Error(`missing form control: ${name}`);
  }
  return control as T;
}

function writeServerForm(server: ServerDraft): void {
  formControl<HTMLInputElement>("id").value = server.id;
  formControl<HTMLInputElement>("name").value = server.name;
  formControl<HTMLInputElement>("host").value = server.host;
  formControl<HTMLInputElement>("port").value = String(server.port);
  formControl<HTMLSelectElement>("backend").value = server.backend;
  formControl<HTMLInputElement>("token").value = server.token;
  showFormErrors({});
}

function readServerForm(): ServerDraft {
  return {
    id: formControl<HTMLInputElement>("id").value,
    name: formControl<HTMLInputElement>("name").value.trim(),
    host: formControl<HTMLInputElement>("host").value.trim(),
    port: formControl<HTMLInputElement>("port").valueAsNumber,
    backend: formControl<HTMLSelectElement>("backend").value as ServerDraft["backend"],
    token: formControl<HTMLInputElement>("token").value,
  };
}

function showFormErrors(errors: ServerValidationErrors, general = ""): void {
  for (const field of ["name", "host", "port", "backend"] as const) {
    requiredElement<HTMLElement>(`[data-error-for="${field}"]`).textContent = errors[field] ?? "";
  }
  const formMessage = requiredElement<HTMLElement>("#form-error");
  formMessage.textContent = general;
  formMessage.dataset.kind = general.startsWith("已识别") ? "success" : general ? "error" : "";
}

function showServerDialog(server: ServerDraft, title: string): void {
  writeServerForm(server);
  requiredElement<HTMLElement>("#server-dialog-title").textContent = title;
  serverDialog.showModal();
  formControl<HTMLInputElement>("name").focus();
}

async function editServer(id: string): Promise<void> {
  setMessage("");
  try {
    const server = await invoke<ServerForEdit>(commands.getServerForEdit, { id });
    showServerDialog(server, "编辑服务器");
  } catch (error) {
    setMessage(`无法读取服务器：${errorMessage(error)}`, "error");
  }
}

async function deleteServer(id: string): Promise<void> {
  const server = currentView?.servers.find((item) => item.id === id);
  if (!server || !window.confirm(`确定删除“${server.name}”吗？`)) return;
  try {
    applyView(await invoke<AppView>(commands.deleteServer, { id }));
    setMessage(`已删除“${server.name}”`, "success");
  } catch (error) {
    setMessage(`删除失败：${errorMessage(error)}`, "error");
  }
}

serverList.addEventListener("click", async (event) => {
  const button = (event.target as Element).closest<HTMLButtonElement>("button[data-action]");
  if (!button) return;
  const action = button.dataset.action;
  const id = button.dataset.id;
  if (action === "add") {
    showServerDialog(emptyServerDraft(), "添加服务器");
  } else if (id && action === "edit") {
    await editServer(id);
  } else if (id && action === "delete") {
    await deleteServer(id);
  } else if (id && action === "toggle-sessions") {
    if (expandedServers.has(id)) {
      expandedServers.delete(id);
    } else {
      expandedServers.add(id);
    }
    if (currentView) {
      serverList.innerHTML = renderServerList(currentView, expandedServers);
    }
  } else if (id && action === "open-session") {
    try {
      await invoke(commands.openServer, { id, sessionId: button.dataset.sessionId ?? null });
    } catch (error) {
      setMessage(`打开失败：${errorMessage(error)}`, "error");
    }
  } else if (id && action === "open") {
    try {
      await invoke(commands.openServer, { id, sessionId: null });
    } catch (error) {
      setMessage(`打开失败：${errorMessage(error)}`, "error");
    }
  }
});

requiredElement<HTMLButtonElement>("#add-server").addEventListener("click", () => {
  showServerDialog(emptyServerDraft(), "添加服务器");
});

requiredElement<HTMLButtonElement>("#open-settings").addEventListener("click", () => {
  settingsDialog.showModal();
});

const pinButton = requiredElement<HTMLButtonElement>("#pin-window");
pinButton.addEventListener("click", async () => {
  const next = pinButton.getAttribute("aria-pressed") !== "true";
  try {
    await invoke(commands.setAlwaysOnTop, { pinned: next });
    pinButton.setAttribute("aria-pressed", String(next));
  } catch (error) {
    setMessage(`置顶失败：${errorMessage(error)}`, "error");
  }
});

serverForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  const draft = readServerForm();
  const errors = validateServerDraft(draft);
  showFormErrors(errors);
  if (hasValidationErrors(errors)) return;

  const saveButton = requiredElement<HTMLButtonElement>("#save-server");
  saveButton.disabled = true;
  try {
    applyView(await invoke<AppView>(commands.saveServer, { server: draft }));
    serverDialog.close();
    setMessage(`已保存“${draft.name}”`, "success");
  } catch (error) {
    showFormErrors({}, `保存失败：${errorMessage(error)}`);
  } finally {
    saveButton.disabled = false;
  }
});

requiredElement<HTMLButtonElement>("#probe-backend").addEventListener("click", async (event) => {
  const draft = readServerForm();
  const errors = validateServerDraft(draft);
  delete errors.name;
  showFormErrors(errors);
  if (hasValidationErrors(errors)) return;

  const button = event.currentTarget as HTMLButtonElement;
  button.disabled = true;
  button.textContent = "识别中…";
  try {
    const backend = await invoke<ServerDraft["backend"]>(commands.probeBackend, { server: draft });
    formControl<HTMLSelectElement>("backend").value = backend;
    showFormErrors({}, `已识别为 ${backend === "kimi" ? "Kimi" : "dsh"}`);
  } catch (error) {
    showFormErrors({}, `识别失败，已保留当前选择：${errorMessage(error)}`);
  } finally {
    button.disabled = false;
    button.textContent = "自动识别";
  }
});

for (const control of document.querySelectorAll<HTMLInputElement>("[data-setting]")) {
  control.addEventListener("change", async () => {
    if (!currentView) return;
    const key = control.dataset.setting as BooleanSettingKey;
    const previous = currentView.settings[key];
    const settings = { ...currentView.settings, [key]: control.checked };
    control.disabled = true;
    try {
      applyView(await invoke<AppView>(commands.updateSettings, { settings }));
      setMessage("设置已更新", "success");
    } catch (error) {
      control.checked = previous;
      setMessage(`设置更新失败：${errorMessage(error)}`, "error");
    } finally {
      control.disabled = false;
    }
  });
}

requiredElement<HTMLButtonElement>("#reconnect-all").addEventListener("click", async (event) => {
  const button = event.currentTarget as HTMLButtonElement;
  button.disabled = true;
  try {
    await invoke(commands.reconnectAll);
    setMessage("已请求所有服务器立即重连", "success");
  } catch (error) {
    setMessage(`重连失败：${errorMessage(error)}`, "error");
  } finally {
    button.disabled = false;
  }
});

requiredElement<HTMLButtonElement>("#paste-import").addEventListener("click", () => {
  pasteContent.value = "";
  pasteError.textContent = "";
  pasteDialog.showModal();
  pasteContent.focus();
});

pasteForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  const content = pasteContent.value.trim();
  if (!content) {
    pasteError.textContent = "请先粘贴配置 JSON";
    return;
  }
  const button = requiredElement<HTMLButtonElement>("#preview-paste");
  button.disabled = true;
  try {
    pendingImport = await invoke<ImportPreview>(commands.previewImportText, { content });
    openImportConfirmation();
    pasteDialog.close();
  } catch (error) {
    pasteError.textContent = `无法解析导入内容：${errorMessage(error)}`;
  } finally {
    button.disabled = false;
  }
});

function openImportConfirmation(): void {
  if (!pendingImport) return;
  requiredElement<HTMLElement>("#import-summary").textContent = importPreviewText(pendingImport);
  const issues = requiredElement<HTMLUListElement>("#import-issues");
  issues.replaceChildren(
    ...importIssueText(pendingImport).map((text) => {
      const item = document.createElement("li");
      item.textContent = text;
      return item;
    }),
  );
  requiredElement<HTMLButtonElement>("#apply-import").disabled = pendingImport.validCount === 0;
  requiredElement<HTMLElement>("#import-error").textContent = "";
  importDialog.showModal();
}

importForm.addEventListener("submit", async (event) => {
  event.preventDefault();
  if (!pendingImport) return;
  const selected = importForm.querySelector<HTMLInputElement>('input[name="import-mode"]:checked');
  const mode = (selected?.value ?? "merge") as ImportMode;
  const button = requiredElement<HTMLButtonElement>("#apply-import");
  button.disabled = true;
  try {
    applyView(
      await invoke<AppView>(commands.applyImport, {
        importId: pendingImport.importId,
        mode,
      }),
    );
    importDialog.close();
    pendingImport = null;
    setMessage("配置导入完成", "success");
  } catch (error) {
    requiredElement<HTMLElement>("#import-error").textContent = `导入失败：${errorMessage(error)}`;
  } finally {
    button.disabled = false;
  }
});

requiredElement<HTMLButtonElement>("#open-export").addEventListener("click", async (event) => {
  const button = event.currentTarget as HTMLButtonElement;
  exportText.value = "";
  button.disabled = true;
  try {
    exportText.value = await invoke<string>(commands.exportConfigText);
    exportDialog.showModal();
  } catch (error) {
    setMessage(`导出失败：${errorMessage(error)}`, "error");
  } finally {
    button.disabled = false;
  }
});

copyExport.addEventListener("click", async () => {
  if (!exportText.value) return;
  const original = copyExport.textContent;
  try {
    await copyTextToClipboard(exportText.value);
    copyExport.textContent = "已复制";
    window.setTimeout(() => {
      copyExport.textContent = original;
    }, 1500);
  } catch (error) {
    setMessage(`复制失败：${errorMessage(error)}`, "error");
  }
});

requiredElement<HTMLButtonElement>("#confirm-export").addEventListener("click", async (event) => {
  const button = event.currentTarget as HTMLButtonElement;
  button.disabled = true;
  try {
    const path = await chooseExportPath();
    if (!path) return;
    await invoke(commands.exportConfig, { path });
    exportDialog.close();
    setMessage("配置已导出", "success");
  } catch (error) {
    setMessage(`导出失败：${errorMessage(error)}`, "error");
  } finally {
    button.disabled = false;
  }
});

for (const button of document.querySelectorAll<HTMLButtonElement>("[data-close-dialog]")) {
  button.addEventListener("click", () => {
    requiredElement<HTMLDialogElement>(`#${button.dataset.closeDialog}`).close();
  });
}

let syncOptions: SyncOption[] = [];

function applySyncOption(address: string): void {
  const option = syncOptions.find((item) => item.address === address);
  if (!option) return;
  syncQr.innerHTML = option.qrSvg;
  syncUrl.textContent = option.url;
}

requiredElement<HTMLButtonElement>("#open-sync").addEventListener("click", async () => {
  syncOptions = [];
  syncAddress.replaceChildren();
  syncQr.replaceChildren();
  syncUrl.textContent = "";
  syncStatus.textContent = "等待手机连接…";
  syncStatus.dataset.kind = "";
  syncDialog.showModal();
  try {
    const info = await invoke<SyncInfo>(commands.startSyncServer);
    syncOptions = info.options;
    syncAddress.replaceChildren(
      ...info.options.map((option) => {
        const item = document.createElement("option");
        item.value = option.address;
        item.textContent = `${option.label}:${option.address}`;
        return item;
      }),
    );
    syncAddress.value = info.selected;
    applySyncOption(info.selected);
  } catch (error) {
    syncStatus.textContent = `启动同步服务失败：${errorMessage(error)}`;
    syncStatus.dataset.kind = "error";
  }
});

syncAddress.addEventListener("change", () => {
  applySyncOption(syncAddress.value);
});

syncDialog.addEventListener("close", () => {
  syncOptions = [];
  syncAddress.replaceChildren();
  syncQr.replaceChildren();
  syncUrl.textContent = "";
  void invoke(commands.stopSyncServer).catch(() => {
    // 服务器可能已因超时自停，忽略错误。
  });
});

async function initialize(): Promise<void> {
  try {
    unlistenState = await listen<AppView>("app-state-changed", (event) => {
      applyView(event.payload);
    });
    unlistenPhoneReceived = await listen<ImportPreview>("phone-config-received", (event) => {
      pendingImport = event.payload;
      openImportConfirmation();
    });
    unlistenPhoneFetched = await listen("phone-config-fetched", () => {
      setMessage("手机已获取配置", "success");
      if (syncDialog.open) {
        syncStatus.textContent = "手机已获取配置";
        syncStatus.dataset.kind = "success";
      }
    });
    applyView(await invoke<AppView>(commands.getAppView));
  } catch (error) {
    setMessage(`无法加载应用状态：${errorMessage(error)}`, "error");
  }
}

void initialize();

if (import.meta.hot) {
  import.meta.hot.dispose(() => {
    unlistenState?.();
    unlistenPhoneReceived?.();
    unlistenPhoneFetched?.();
  });
}
