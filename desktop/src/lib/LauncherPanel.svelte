<script lang="ts">
  import type { Backend, ServerStatus, ServerSummary, SessionSummary } from "../model";

  interface Props {
    servers: ServerSummary[];
    statuses: Record<string, ServerStatus>;
    activeServerId: string | null;
    onOpen: (id: string, sessionId: string | null) => void;
    onOpenInNewPane: (id: string, sessionId: string | null) => void;
    onAdd: () => void;
    onEdit: (id: string) => void;
    onDelete: (id: string) => void;
    onTogglePin: (id: string, sessionId: string) => void;
    onClose: () => void;
  }

  let {
    servers,
    statuses,
    activeServerId,
    onOpen,
    onOpenInNewPane,
    onAdd,
    onEdit,
    onDelete,
    onTogglePin,
    onClose,
  }: Props = $props();

  let tab = $state<"servers" | "sessions">("servers");

  interface SessionRow extends SessionSummary {
    serverId: string;
    serverName: string;
  }

  /** 汇总所有服务的忙碌会话，置顶排前面。 */
  let sessions = $derived(
    servers
      .flatMap((server) =>
        (statuses[server.id]?.sessions ?? []).map((session) => ({
          ...session,
          serverId: server.id,
          serverName: server.name,
        })),
      )
      .sort((a, b) => rank(a) - rank(b)),
  );

  /**
   * 排序权重：置顶且运行中 → 置顶已完成（等你介入）→ 其余。
   * 已完成但被置顶的会话要留在最上面提醒用户。
   */
  function rank(row: SessionRow): number {
    if (row.pinned && !row.done) return 0;
    if (row.pinned && row.done) return 1;
    return 2;
  }

  function mark(backend: Backend): string {
    return backend === "kimi" ? "K" : backend === "opencode" ? "OC" : "🐋";
  }

  function label(backend: Backend): string {
    return backend === "kimi" ? "Kimi" : backend === "opencode" ? "OpenCode" : "dsh";
  }
</script>

<div class="launcher" role="dialog" aria-label="悬浮球面板">
  <div class="launcher__head">
    <div class="launcher__tabs" role="tablist">
      <button
        class="launcher__tab"
        class:launcher__tab--active={tab === "servers"}
        type="button"
        role="tab"
        aria-selected={tab === "servers"}
        onclick={() => (tab = "servers")}>服务连接</button
      >
      <button
        class="launcher__tab"
        class:launcher__tab--active={tab === "sessions"}
        type="button"
        role="tab"
        aria-selected={tab === "sessions"}
        onclick={() => (tab = "sessions")}
      >
        活跃会话{#if sessions.length > 0}<span class="launcher__badge">{sessions.length}</span>{/if}
      </button>
    </div>
    <button class="launcher__close" type="button" aria-label="关闭" onclick={onClose}>×</button>
  </div>

  {#if tab === "servers"}
    <div class="launcher__list">
      {#each servers as server (server.id)}
        {@const status = statuses[server.id]}
        <div class="launcher__row" class:launcher__row--active={server.id === activeServerId}>
          <button
            class="launcher__main"
            type="button"
            onclick={() => onOpen(server.id, null)}
            title="在当前窗格打开"
          >
            <span class="launcher__mark" data-backend={server.backend}>{mark(server.backend)}</span>
            <span class="launcher__text">
              <span class="launcher__name">
                {server.name}
                {#if server.id === activeServerId}<span class="launcher__current">当前</span>{/if}
              </span>
              <span class="launcher__meta">
                {label(server.backend)} · {server.host}:{server.port}
                <span
                  class="launcher__dot"
                  class:launcher__dot--on={status?.connected}
                  aria-hidden="true"
                ></span>
              </span>
            </span>
          </button>
          <div class="launcher__actions">
            <button
              class="icon-button"
              type="button"
              aria-label="在新窗格打开"
              title="在新窗格打开"
              onclick={() => onOpenInNewPane(server.id, null)}>⧉</button
            >
            <button
              class="icon-button"
              type="button"
              aria-label="编辑"
              title="编辑"
              onclick={() => onEdit(server.id)}>✎</button
            >
            <button
              class="icon-button icon-button--danger"
              type="button"
              aria-label="删除"
              title="删除"
              onclick={() => onDelete(server.id)}>🗑</button
            >
          </div>
        </div>
      {:else}
        <p class="launcher__empty">还没有服务连接。</p>
      {/each}
    </div>
    <div class="launcher__foot">
      <button class="primary-button" type="button" onclick={onAdd}>＋ 添加服务连接</button>
    </div>
  {:else}
    <div class="launcher__list">
      {#each sessions as row (row.serverId + row.id)}
        <div class="launcher__row" class:launcher__row--pinned={row.pinned}>
          <button class="launcher__main" type="button" onclick={() => onOpen(row.serverId, row.id)}>
            <span class="launcher__text">
              <span class="launcher__name">
                {row.title || row.id}
                {#if row.done}<span class="launcher__done">已完成</span>{/if}
              </span>
              <span class="launcher__meta">
                {row.serverName}{#if row.activity} · {row.activity}{/if}
              </span>
            </span>
          </button>
          <div class="launcher__actions">
            <button
              class="icon-button"
              type="button"
              aria-label={row.pinned ? "取消置顶" : "置顶"}
              title={row.pinned ? "取消置顶" : "置顶后完成时会通知"}
              onclick={() => onTogglePin(row.serverId, row.id)}>{row.pinned ? "★" : "☆"}</button
            >
          </div>
        </div>
      {:else}
        <p class="launcher__empty">当前没有正在运行的会话。</p>
      {/each}
    </div>
  {/if}
</div>
