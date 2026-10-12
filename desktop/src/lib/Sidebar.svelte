<script lang="ts">
  import type { Backend, ServerStatus, ServerSummary } from "../model";

  interface Props {
    servers: ServerSummary[];
    statuses: Record<string, ServerStatus>;
    expanded: ReadonlySet<string>;
    selectedId: string | null;
    selectedSessionId: string | null;
    /** 折叠为图标条：省出宽度给内容区，导航仍可用。 */
    collapsed?: boolean;
    onToggleCollapsed?: () => void;
    onSelectServer: (id: string) => void;
    onSelectSession: (id: string, sessionId: string) => void;
    onToggleExpand: (id: string) => void;
    onTogglePin: (id: string, sessionId: string) => void;
    onEdit: (id: string) => void;
    onDelete: (id: string) => void;
    onAdd: () => void;
  }

  let {
    servers,
    statuses,
    expanded,
    selectedId,
    selectedSessionId,
    collapsed = false,
    onToggleCollapsed,
    onSelectServer,
    onSelectSession,
    onToggleExpand,
    onTogglePin,
    onEdit,
    onDelete,
    onAdd,
  }: Props = $props();

  function mark(backend: Backend): string {
    return backend === "kimi" ? "K" : backend === "opencode" ? "OC" : "🐋";
  }

  function label(backend: Backend): string {
    return backend === "kimi" ? "Kimi" : backend === "opencode" ? "OpenCode" : "dsh";
  }
</script>

<nav class="sidebar{collapsed ? ' sidebar--collapsed' : ''}" aria-label="服务连接">
  <div class="sidebar__head">
    {#if collapsed}
      <button
        class="icon-button sidebar__expand"
        type="button"
        aria-label="展开侧边栏"
        title="展开侧边栏"
        onclick={() => onToggleCollapsed?.()}
      >
        <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true"
          ><path
            d="M14.5 7 9.5 12l5 5"
            fill="none"
            stroke="currentColor"
            stroke-width="1.8"
            stroke-linecap="round"
            stroke-linejoin="round"
          /></svg
        >
      </button>
    {:else}
      <h2 class="sidebar__title">服务</h2>
      <div class="sidebar__head-actions">
        <button
          class="icon-button"
          type="button"
          onclick={() => onToggleCollapsed?.()}
          aria-label="折叠侧边栏"
          title="折叠侧边栏"
        >
          <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true"
            ><path
              d="M9.5 7l5 5-5 5"
              fill="none"
              stroke="currentColor"
              stroke-width="1.8"
              stroke-linecap="round"
              stroke-linejoin="round"
            /></svg
          >
        </button>
        <button class="icon-button" type="button" onclick={onAdd} aria-label="添加服务连接">＋</button>
      </div>
    {/if}
  </div>

  {#if servers.length === 0}
    <div class="sidebar__empty">
      <p>还没有服务连接</p>
      <button class="primary-button" type="button" onclick={onAdd}>添加第一个</button>
    </div>
  {:else}
    <ul class="sidebar__list">
      {#each servers as server (server.id)}
        {@const status = statuses[server.id]}
        {@const sessions = status?.sessions ?? []}
        {@const isExpanded = expanded.has(server.id)}
        {@const connected = status?.connected === true}
        <li
          class="sidebar__group"
          class:sidebar__group--active={selectedId === server.id}
        >
          <div class="sidebar__row">
            <span
              class="sidebar__mark backend-logo backend-logo--{server.backend}{connected
                ? ''
                : ' backend-logo--offline'}"
              data-backend={server.backend}
              title="{label(server.backend)} · {server.name}（{connected ? '在线' : '离线'}）"
              aria-label={label(server.backend)}
            >
              {mark(server.backend)}
              <span
                class="status-dot{connected ? ' status-dot--online' : ''}"
                title={connected ? "已连接" : "未连接"}
              ></span>
            </span>
            {#if !collapsed}
              <button
                class="sidebar__name"
                type="button"
                aria-current={selectedId === server.id ? "true" : undefined}
                onclick={() => onSelectServer(server.id)}
              >
                <span class="sidebar__label">{server.name}</span>
                <span class="sidebar__meta">
                  <span>{connected ? "在线" : "离线"}</span>
                  {#if status?.serverVersion}
                    <span class="sidebar__version" title="服务端版本">v{status.serverVersion}</span>
                  {/if}
                  {#if status?.activeCount}
                    <span class="sidebar__count">{status.activeCount} 个任务</span>
                  {/if}
                </span>
              </button>
              {#if connected && sessions.length > 0}
                <button
                  class="icon-button sidebar__toggle"
                  type="button"
                  aria-expanded={isExpanded}
                  aria-label="展开 {server.name} 的会话"
                  onclick={() => onToggleExpand(server.id)}
                >
                  <span class="status-chevron" class:status-chevron--open={isExpanded} aria-hidden="true"
                    >▾</span
                  >
                </button>
              {/if}
            {:else}
              <!-- 折叠态：整个图标可点，标题走 title 提示 -->
              <button
                class="sidebar__compact-hit"
                type="button"
                aria-current={selectedId === server.id ? "true" : undefined}
                aria-label="打开 {server.name}"
                onclick={() => onSelectServer(server.id)}
              ></button>
              {#if status?.activeCount}
                <span class="sidebar__compact-badge" aria-label="{status.activeCount} 个运行中任务">
                  {status.activeCount}
                </span>
              {/if}
            {/if}
          </div>

          {#if !collapsed && isExpanded && connected && sessions.length > 0}
            <ul class="sidebar__sessions">
              {#each sessions as session (session.id)}
                {@const active = selectedSessionId === session.id && selectedId === server.id}
                <li>
                  <div class="sidebar__session{active ? ' sidebar__session--active' : ''}">
                    <button
                      class="sidebar__session-main"
                      type="button"
                      onclick={() => onSelectSession(server.id, session.id)}
                    >
                      <span class="sidebar__session-title">{session.title}</span>
                      {#if session.activity}
                        <span class="sidebar__session-activity" class:sidebar__session-activity--done={session.done}>
                          {session.activity}
                        </span>
                      {/if}
                    </button>
                    <button
                      class="session-pin{session.pinned ? ' session-pin--active' : ''}"
                      type="button"
                      aria-pressed={session.pinned}
                      aria-label="置顶 {session.title}"
                      title={session.pinned ? "取消置顶" : "置顶，完成后会通知你"}
                      onclick={() => onTogglePin(server.id, session.id)}
                    >
                      <svg viewBox="0 0 24 24" width="12" height="12" aria-hidden="true"
                        ><path
                          d="M16 9V4h1c.55 0 1-.45 1-1s-.45-1-1-1H7c-.55 0-1 .45-1 1s.45 1 1 1h1v5c0 1.66-1.34 3-3 3v2h5.97v7l1 1 1-1v-7H19v-2c-1.66 0-3-1.34-3-3z"
                          fill="currentColor"
                        /></svg
                      >
                    </button>
                  </div>
                </li>
              {/each}
            </ul>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}

  {#if selectedId && !collapsed}
    <div class="sidebar__foot">
      <button class="text-button" type="button" onclick={() => onEdit(selectedId)}>编辑</button>
      <button class="text-button" type="button" onclick={() => onDelete(selectedId)}>删除</button>
    </div>
  {/if}
</nav>