import type { AppView, Backend, ServerStatus, ServerSummary } from "./model";

export function escapeHtml(value: string): string {
  return value.replace(/[&<>'"]/g, (character) => {
    const entities: Record<string, string> = {
      "&": "&amp;",
      "<": "&lt;",
      ">": "&gt;",
      "'": "&#39;",
      '"': "&quot;",
    };
    return entities[character];
  });
}

function backendLogo(backend: Backend, connected: boolean): string {
  const label = backend === "kimi" ? "Kimi" : "dsh";
  const mark = backend === "kimi" ? "K" : "🐋";
  return `<span class="backend-logo backend-logo--${backend}${connected ? "" : " backend-logo--offline"}" data-backend="${backend}" aria-label="${label}">${mark}</span>`;
}

function statusLabel(status: ServerStatus | undefined): string {
  if (!status?.connected) {
    return status?.error ? "离线 · 连接异常" : "离线";
  }
  if (status.activeCount > 0) {
    return `在线 · ${status.activeCount} 个任务运行中`;
  }
  return "在线";
}

function renderServerCard(
  server: ServerSummary,
  status: ServerStatus | undefined,
  active: boolean,
): string {
  const connected = status?.connected === true;
  const id = escapeHtml(server.id);
  const name = escapeHtml(server.name);
  const address = escapeHtml(`${server.host}:${server.port}`);

  return `
    <article class="server-card${connected ? "" : " server-card--offline"}" data-server-id="${id}">
      <button class="server-open" type="button" data-action="open" data-id="${id}" aria-label="在浏览器中打开 ${name}">
        ${backendLogo(server.backend, connected)}
        <span class="server-copy">
          <span class="server-title-row">
            <strong>${name}</strong>
            ${active ? '<span class="badge badge--active">当前</span>' : ""}
          </span>
          <span class="server-address">${address}</span>
          <span class="server-status${connected ? " server-status--online" : ""}">${statusLabel(status)}</span>
        </span>
      </button>
      <div class="server-actions" aria-label="${name} 操作">
        ${active ? "" : `<button class="text-button" type="button" data-action="activate" data-id="${id}">设为当前</button>`}
        <button class="icon-button" type="button" data-action="edit" data-id="${id}" aria-label="编辑 ${name}">编辑</button>
        <button class="icon-button icon-button--danger" type="button" data-action="delete" data-id="${id}" aria-label="删除 ${name}">删除</button>
      </div>
    </article>`;
}

export function renderServerList(view: AppView): string {
  if (view.servers.length === 0) {
    return `
      <div class="empty-state">
        <div class="empty-state__icon" aria-hidden="true">⌁</div>
        <h2>还没有服务器</h2>
        <p>添加 Kimi 或 dsh 服务器后，AgentPocket 会在后台持续监听。</p>
        <button class="primary-button" type="button" data-action="add">添加第一台服务器</button>
      </div>`;
  }

  return view.servers
    .map((server) =>
      renderServerCard(server, view.statuses[server.id], view.activeId === server.id),
    )
    .join("");
}
