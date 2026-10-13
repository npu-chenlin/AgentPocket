<script lang="ts">
  import { onMount } from "svelte";
  import type { BallSnapshot } from "../../electron/main/ball-ipc";
  import GrokFace from "./face/GrokFace.svelte";
  import type { FaceState } from "./face/expression-state";
  const SIZE = 56;
  let faceState = $state<FaceState>("idle");
  let open = $state(false);
  let spin = $state(0);
  let asleep = $state(false);
  let dragging = $state(false);
  let failure = $state("");
  let timer: ReturnType<typeof setTimeout>;
  let startX = 0;
  let startY = 0;
  let moved = false;
  let revision = -1;
  function bridge() {
    if (!window.agentpocketBall) throw new Error("悬浮球 preload 未注入");
    return window.agentpocketBall;
  }
  function run(promise: Promise<unknown>) { void promise.catch(error => { failure = String(error); }); }
  function wake() {
    asleep = false;
    clearTimeout(timer);
    timer = setTimeout(() => { if (!open && !dragging && faceState === "idle") asleep = true; }, 3500);
  }
  function apply(state: BallSnapshot) {
    if (state.revision <= revision) return;
    revision = state.revision;
    faceState = state.face;
    open = state.open;
    spin = state.spin;
    wake();
  }
  function down(event: PointerEvent) {
    if (event.button !== 0) return;
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    startX = event.screenX; startY = event.screenY;
    moved = false; dragging = true;
    run(bridge().drag("start"));
    wake();
  }
  function move(event: PointerEvent) {
    if (!dragging) return;
    if (Math.hypot(event.screenX - startX, event.screenY - startY) > 4) moved = true;
    if (moved) run(bridge().drag("move"));
  }
  function up(event: PointerEvent) {
    if (!dragging) return;
    dragging = false;
    run(bridge().drag(event.type === "pointercancel" || !moved ? "cancel" : "end"));
    if (event.type === "pointercancel") moved = true;
    (event.currentTarget as HTMLElement).releasePointerCapture(event.pointerId);
    wake();
  }
  function click() {
    if (moved) { moved = false; return; }
    run(bridge().toggle());
    wake();
  }
  onMount(() => {
    try {
      const off = bridge().onState(apply);
      run(bridge().getState().then(apply));
      wake();
      return () => { clearTimeout(timer); off(); };
    } catch (error) { failure = String(error); }
  });
</script>
<button class="ball" class:ball--asleep={asleep} class:ball--dragging={dragging}
  type="button" aria-label="打开悬浮球面板" aria-expanded={open} title={failure || "打开悬浮球面板"}
  onpointerdown={down} onpointermove={move} onpointerup={up} onpointercancel={up}
  onlostpointercapture={() => { if (dragging) { dragging = false; moved = true; run(bridge().drag("cancel")); } }} onclick={click}>
  <span class="ball__body" aria-hidden="true"></span>
  <span class="ball__shade" aria-hidden="true"></span>
  <span class="ball__gloss" aria-hidden="true"></span>
  <span class="ball__face"><GrokFace faceState={asleep ? "sleeping" : faceState} {spin} size={SIZE} /></span>
  {#if failure}<span role="alert">!</span>{/if}
</button>
