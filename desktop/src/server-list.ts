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

function backendMark(backend: Backend, connected: boolean): string {
  const label = backend === "kimi" ? "Kimi" : "dsh";
  const mark = backend === "kimi" ? "K" : "🐋";
  return `
    <span class="server-badge">
      <span class="backend-logo backend-logo--${backend}${connected ? "" : " backend-logo--offline"}" data-backend="${backend}" aria-label="${label}">${mark}</span>
      <span class="status-dot${connected ? " status-dot--online" : ""}" aria-hidden="true"></span>
    </span>`;
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

/** 运行中会话下拉面板：仅在线且存在忙碌会话时渲染。 */
function renderSessionsPanel(
  server: ServerSummary,
  status: ServerStatus | undefined,
  expanded: boolean,
): string {
  const sessions = status?.sessions ?? [];
  if (status?.connected !== true || sessions.length === 0) return "";
  const id = escapeHtml(server.id);
  const rows = expanded
    ? `<ul class="session-list">${sessions
        .map(
          (session) => `
        <li>
          <button class="session-row" type="button" data-action="open-session"
            data-id="${id}" data-session-id="${escapeHtml(session.id)}"
            aria-label="打开会话 ${escapeHtml(session.title)}">
            <span class="session-spinner" aria-hidden="true"></span>
            <span class="session-title">${escapeHtml(session.title)}</span>
          </button>
        </li>`,
        )
        .join("")}</ul>`
    : "";
  return `
    <div class="server-sessions">
      <button class="sessions-toggle" type="button" data-action="toggle-sessions" data-id="${id}"
        aria-expanded="${expanded}">
        ${sessions.length} 个运行中会话 ${expanded ? "▴" : "▾"}
      </button>
      ${rows}
    </div>`;
}

function renderServerCard(
  server: ServerSummary,
  status: ServerStatus | undefined,
  expanded: boolean,
): string {
  const connected = status?.connected === true;
  const id = escapeHtml(server.id);
  const name = escapeHtml(server.name);
  const address = escapeHtml(`${server.host}:${server.port}`);

  return `
    <article class="server-card${connected ? "" : " server-card--offline"}" data-server-id="${id}">
      <button class="server-open" type="button" data-action="open" data-id="${id}" aria-label="在浏览器中打开 ${name}">
        ${backendMark(server.backend, connected)}
        <span class="server-copy">
          <span class="server-title-row">
            <strong>${name}</strong>
          </span>
          <span class="server-meta">
            <span class="server-address">${address}</span>
            <span class="server-meta-sep" aria-hidden="true">·</span>
            <span class="server-status${connected ? " server-status--online" : ""}">${statusLabel(status)}</span>
          </span>
        </span>
      </button>
      ${renderSessionsPanel(server, status, expanded)}
      <div class="server-actions" aria-label="${name} 操作">
        <button class="icon-button" type="button" data-action="edit" data-id="${id}" aria-label="编辑 ${name}">编辑</button>
        <button class="icon-button icon-button--danger" type="button" data-action="delete" data-id="${id}" aria-label="删除 ${name}">删除</button>
      </div>
      <span class="server-open-hint" aria-hidden="true">↗</span>
    </article>`;
}

export function renderServerList(view: AppView, expanded: ReadonlySet<string> = new Set()): string {
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
    .map((server) => renderServerCard(server, view.statuses[server.id], expanded.has(server.id)))
    .join("");
}
