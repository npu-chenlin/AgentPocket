import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  CREDENTIAL_WARNING,
  chooseExportPath,
  chooseImportPath,
  importIssueText,
  importPreviewText,
} from "./import-export";
import {
  commands,
  emptyServerDraft,
  type AppView,
  type ExportFormat,
  type ImportMode,
  type ImportPreview,
  type ServerDraft,
  type ServerForEdit,
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
    <header class="hero">
      <div>
        <p class="eyebrow">AGENT CONTROL CENTER</p>
        <h1>AgentPocket</h1>
        <p class="hero__copy">安静驻留，集中监听每一台 Kimi 与 dsh 服务器。</p>
      </div>
      <button id="add-server" class="primary-button" type="button">添加服务器</button>
    </header>

    <div id="app-message" class="app-message" role="status" aria-live="polite"></div>

    <main class="content-grid">
      <section class="panel servers-panel" aria-labelledby="servers-title">
        <div class="section-heading">
          <div>
            <p class="section-kicker">SERVERS</p>
            <h2 id="servers-title">服务器</h2>
          </div>
          <button id="reconnect-all" class="secondary-button" type="button">全部重连</button>
        </div>
        <div id="server-list" class="server-list" aria-live="polite"></div>
      </section>

      <aside class="side-column">
        <section class="panel settings-panel" aria-labelledby="settings-title">
          <div class="section-heading section-heading--compact">
            <div>
              <p class="section-kicker">PREFERENCES</p>
              <h2 id="settings-title">设置</h2>
            </div>
          </div>
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
        </section>

        <section class="panel transfer-panel" aria-labelledby="transfer-title">
          <div class="section-heading section-heading--compact">
            <div>
              <p class="section-kicker">PORTABILITY</p>
              <h2 id="transfer-title">导入与导出</h2>
            </div>
          </div>
          <button id="import-config" class="secondary-button secondary-button--wide" type="button">导入 JSON</button>
          <button class="secondary-button secondary-button--wide" type="button" data-export-format="full">导出完整配置</button>
          <button class="text-button transfer-link" type="button" data-export-format="android">导出 Android 兼容列表</button>
        </section>
      </aside>
    </main>
  </div>

  <dialog id="server-dialog" class="modal">
    <form id="server-form" method="dialog" novalidate>
      <div class="modal__heading">
        <div><p class="section-kicker">CONNECTION</p><h2 id="server-dialog-title">添加服务器</h2></div>
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
      <div class="modal__heading"><div><p class="section-kicker">IMPORT</p><h2>确认导入</h2></div></div>
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

  <dialog id="export-dialog" class="modal modal--small">
    <div class="modal__heading"><div><p class="section-kicker">EXPORT</p><h2>导出配置</h2></div></div>
    <p class="credential-warning">${CREDENTIAL_WARNING}</p>
    <p class="modal__summary">导出的 JSON 会保留连接服务器所需的令牌，请仅保存到可信位置。</p>
    <div class="modal__actions">
      <span class="modal__spacer"></span>
      <button class="text-button" type="button" data-close-dialog="export-dialog">取消</button>
      <button id="confirm-export" class="primary-button" type="button">选择保存位置</button>
    </div>
  </dialog>`;

function requiredElement<T extends Element>(selector: string): T {
  const element = document.querySelector<T>(selector);
  if (!element) throw new Error(`missing element: ${selector}`);
  return element;
}

const serverList = requiredElement<HTMLElement>("#server-list");
const message = requiredElement<HTMLElement>("#app-message");
const serverDialog = requiredElement<HTMLDialogElement>("#server-dialog");
const serverForm = requiredElement<HTMLFormElement>("#server-form");
const importDialog = requiredElement<HTMLDialogElement>("#import-dialog");
const importForm = requiredElement<HTMLFormElement>("#import-form");
const exportDialog = requiredElement<HTMLDialogElement>("#export-dialog");

let currentView: AppView | null = null;
let latestRevision = -1;
let pendingImport: ImportPreview | null = null;
let pendingExportFormat: ExportFormat = "full";
let unlistenState: UnlistenFn | undefined;

function setMessage(text: string, kind: "info" | "error" | "success" = "info"): void {
  message.textContent = text;
  message.dataset.kind = text ? kind : "";
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

function applyView(view: AppView): void {
  if (view.revision <= latestRevision) return;
  latestRevision = view.revision;
  currentView = view;
  serverList.innerHTML = renderServerList(view);
  for (const control of document.querySelectorAll<HTMLInputElement>("[data-setting]")) {
    const key = control.dataset.setting as keyof AppView["settings"];
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
  } else if (id && action === "activate") {
    try {
      applyView(await invoke<AppView>(commands.setActiveServer, { id }));
    } catch (error) {
      setMessage(`设置当前服务器失败：${errorMessage(error)}`, "error");
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
    const key = control.dataset.setting as keyof AppView["settings"];
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

requiredElement<HTMLButtonElement>("#import-config").addEventListener("click", async () => {
  setMessage("");
  try {
    const path = await chooseImportPath();
    if (!path) return;
    pendingImport = await invoke<ImportPreview>(commands.previewImport, { path });
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
  } catch (error) {
    setMessage(`读取导入文件失败：${errorMessage(error)}`, "error");
  }
});

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

for (const button of document.querySelectorAll<HTMLButtonElement>("[data-export-format]")) {
  button.addEventListener("click", () => {
    pendingExportFormat = button.dataset.exportFormat as ExportFormat;
    exportDialog.showModal();
  });
}

requiredElement<HTMLButtonElement>("#confirm-export").addEventListener("click", async (event) => {
  const button = event.currentTarget as HTMLButtonElement;
  button.disabled = true;
  try {
    const path = await chooseExportPath(pendingExportFormat);
    if (!path) return;
    await invoke(commands.exportConfig, { path, format: pendingExportFormat });
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

async function initialize(): Promise<void> {
  try {
    unlistenState = await listen<AppView>("app-state-changed", (event) => {
      applyView(event.payload);
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
  });
}
