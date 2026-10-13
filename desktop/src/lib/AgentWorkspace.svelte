<script lang="ts">
  import { onMount } from "svelte";
  import "dockview/dist/styles/dockview.css";
  import "./workspace-theme.css";
  import * as api from "./api";
  import { WorkspaceController } from "./workspace-controller";
  import type { WorkspaceTab } from "../../shared/workspace";
  let { servers, overlaid, onChange, onError, onReady, onOpen }: {
    servers: { id: string }[] | null;
    overlaid: boolean;
    onChange: (tabs: WorkspaceTab[], activeId: string | null) => void;
    onError: (message: string) => void;
    onReady: (controller: WorkspaceController) => void;
    onOpen: () => void;
  } = $props();
  let host: HTMLDivElement;
  let controller = $state<WorkspaceController | null>(null);
  let count = $state(0);
  onMount(() => {
    controller = new WorkspaceController(host, api, (tabs, active) => { count = tabs.length; onChange(tabs, active); }, onError, onOpen);
    onReady(controller);
    return () => controller?.dispose();
  });
  $effect(() => {
    if (controller && servers) { controller.restore(servers); controller.prune(servers); }
  });
  $effect(() => controller?.setOverlay("modal", overlaid));
</script>

<div class="dock-wrapper">
  <div class="dock-host" bind:this={host}></div>
  {#if !count}
    <div class="workspace__empty"><h2>{servers?.length ? "选一个服务打开" : "还没有服务连接"}</h2>
      <p>通过悬浮球面板打开服务；拖拽标签到边缘分屏，拖动分隔条调整尺寸。</p>
      <button onclick={onOpen}>打开服务连接</button>
    </div>
  {/if}
</div>

<style>
  .dock-wrapper { flex:1; min-height:0; position:relative; }
  .dock-host { width:100%; height:100%; }
  .workspace__empty { position:absolute; inset:0; z-index:2; display:flex; flex-direction:column; align-items:center; justify-content:center; pointer-events:none; }
  .workspace__empty button { pointer-events:auto; }
  :global(.dv-sash) { z-index:20; }
</style>
