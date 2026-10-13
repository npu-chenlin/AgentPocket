<script lang="ts">
  import type { MeshPeerView } from "../model";

  interface Props {
    peers: MeshPeerView[];
    loading: boolean;
    onAction: (action: string, host: string, note?: string) => void;
  }

  let { peers, loading, onAction }: Props = $props();

  // 节点地址/备注是受控输入，由父组件持有，方便"添加"后清空。
  let host = $state("");
  let name = $state("");
</script>

<div class="mesh-section">
  <h3 class="mesh-section__title">节点管理</h3>
  <div class="mesh-peers" aria-live="polite">
    {#if loading}
      <p class="mesh-placeholder">发现中…</p>
    {:else if peers.length === 0}
      <p class="mesh-placeholder">未发现 AgentPocket 节点</p>
    {:else}
      {#each peers as peer (peer.host)}
        <div class="mesh-peer">
          <span class="mesh-dot{peer.online ? ' mesh-dot--online' : ''}" aria-hidden="true"></span>
          <span class="mesh-peer-copy">
            <strong class="mesh-peer-name">{peer.name}</strong>
            <span class="mesh-peer-meta">
              <span class="mesh-peer-host">{peer.host}</span>
              {#if peer.version}<span class="mesh-peer-version">v{peer.version}</span>{/if}
              {#if !peer.online}<span class="mesh-peer-offline">离线</span>{/if}
              {#if peer.kimiVersion}<span class="mesh-peer-version">kimi v{peer.kimiVersion}</span>{/if}
              {#if peer.webActive}<span class="mesh-peer-version">web:{peer.webPort ?? "-"}</span>{/if}
            </span>
          </span>
          <span class="mesh-peer-actions">
            <button class="text-button" type="button" onclick={() => onAction("pull", peer.host)}>获取</button>
            <button class="text-button" type="button" onclick={() => onAction("push", peer.host)}>发送</button>
            <button class="text-button" type="button" onclick={() => onAction("upgrade", peer.host)}>升级</button>
            <button class="text-button" type="button" onclick={() => onAction("restart-web", peer.host)}>重启</button>
          </span>
          {#if peer.manual}
            <button class="mesh-remove" type="button" onclick={() => onAction("remove", peer.host)} aria-label="删除 {peer.name}">×</button>
          {/if}
        </div>
      {/each}
    {/if}
  </div>
  <div class="mesh-toolbar">
    <input class="mesh-input" bind:value={host} placeholder="100.x.x.x 或 MagicDNS 名" aria-label="节点地址" />
    <input class="mesh-input" bind:value={name} placeholder="备注名（可选）" aria-label="节点备注" />
    <button class="secondary-button" type="button" onclick={() => onAction("add", host, name)}>添加</button>
    <button class="text-button" type="button" onclick={() => onAction("refresh", "")}>刷新</button>
  </div>
</div>