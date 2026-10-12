<script lang="ts">
  import { onMount } from "svelte";
  import type { Backend } from "../model";
  import { setWebviewBounds } from "./api";

  interface Props {
    selectedName: string | null;
    selectedBackend: Backend | null;
    hasServers: boolean;
  }

  let { selectedName, selectedBackend, hasServers }: Props = $props();

  let stage = $state<HTMLElement | null>(null);

  /**
   * 把内容区矩形同步给主进程：Agent 视图是窗口的子视图，坐标系与 DOM 视口
   * 一致，原生只跟随，不自己算布局。
   */
  function reportBounds(): void {
    if (!stage) return;
    const rect = stage.getBoundingClientRect();
    void setWebviewBounds({
      x: rect.left,
      y: rect.top,
      width: rect.width,
      height: rect.height,
    });
  }

  onMount(() => {
    reportBounds();
    // ResizeObserver 比 window.resize 可靠：侧边栏折叠、滚动条出现等布局变化
    // 都会触发。窗口整体缩放/移动不用管——子视图会跟着父窗口自动走。
    // 老 WebKit 可能没有这个 API，缺失时退回 window.resize。
    const observer =
      typeof ResizeObserver === "undefined" ? null : new ResizeObserver(reportBounds);
    if (observer && stage) observer.observe(stage);
    window.addEventListener("resize", reportBounds);
    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", reportBounds);
    };
  });
</script>

<section class="stage" bind:this={stage} aria-label={selectedName ?? "内容区"}>
  {#if !hasServers}
    <div class="stage__empty">
      <div class="empty-state__icon" aria-hidden="true">⌁</div>
      <p>添加 Kimi / OpenCode / dsh 服务后，这里会直接显示它的 Web 界面。</p>
    </div>
  {:else if !selectedName}
    <div class="stage__empty">
      <p>从左侧选择一个服务或会话。</p>
    </div>
  {:else}
    <!-- WebView 覆盖在这一块上，这里只留底色避免切换时闪烁。 -->
    <div class="stage__backdrop" data-backend={selectedBackend} aria-hidden="true">
      <span>{selectedName}</span>
    </div>
  {/if}
</section>