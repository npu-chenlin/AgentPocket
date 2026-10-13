<script lang="ts">
  import { onMount } from "svelte";
  import type { FaceState } from "./expression-state";
  import { GrokFace } from "./face";

  interface Props {
    /** 别叫 state：`$state(...)` 会被当成对它的 store 订阅。 */
    faceState: FaceState;
    /** 自增令牌：变了就让球转一圈（整轮跑完时），装球时不转。 */
    spin?: number;
    /** 边长（CSS 像素）。 */
    size?: number;
  }

  let { faceState, spin = 0, size = 132 }: Props = $props();

  let canvas = $state<HTMLCanvasElement | null>(null);
  let face = $state<GrokFace | null>(null);
  let lastSpin = -1;

  // 画布像素按 devicePixelRatio 放大：桌面缩放不是 100% 时，1x 的位图会糊。
  let dpr = $state(window.devicePixelRatio || 1);
  let pixels = $derived(Math.max(1, Math.round(size * dpr)));
  onMount(() => {
    const resize = () => { dpr = window.devicePixelRatio || 1; };
    window.addEventListener("resize", resize);
    return () => window.removeEventListener("resize", resize);
  });

  onMount(() => {
    const context = canvas?.getContext("2d") ?? null;
    if (!canvas || !context) return;
    face = new GrokFace(context, pixels, pixels);
    face.start();
    return () => {
      face?.dispose();
      face = null;
    };
  });

  // 状态与尺寸统一在这里推给渲染器：构造和后续变化走同一条路径，不留两处真相。
  $effect(() => {
    if (!face) return;
    face.setFaceState(faceState);
    face.resize(pixels, pixels);
  });

  $effect(() => {
    if (!face || spin === lastSpin) return;
    lastSpin = spin;
    if (spin > 0) face.spinOnce();
  });
</script>

<canvas bind:this={canvas} width={pixels} height={pixels} style="width:{size}px;height:{size}px"
></canvas>
