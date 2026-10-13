<script lang="ts">
  import type { DesktopSettings, SyncInfo } from "../model";
  import type { MeshPeerView } from "../model";
  import MeshPanel from "./MeshPanel.svelte";

  interface Props {
    open: boolean;
    settings: DesktopSettings;
    peers: MeshPeerView[];
    peersLoading: boolean;
    syncInfo?: SyncInfo | null;
    syncStatus?: { text: string; kind: string };
    onClose: () => void;
    onToggleSetting: (key: "startHidden" | "autostart" | "notifications") => void;
    onReconnectAll: () => void;
    onMeshAction: (action: string, host: string, note?: string) => void;
  }

  let {
    open,
    settings,
    peers,
    peersLoading,
    syncInfo,
    syncStatus,
    onClose,
    onToggleSetting,
    onReconnectAll,
    onMeshAction,
  }: Props = $props();

  let selectedAddress = $state("");
  $effect(() => {
    if (syncInfo && !selectedAddress) selectedAddress = syncInfo.selected;
  });

  import { modal } from "./modal";
</script>

<dialog use:modal={open} class="modal modal--small" aria-labelledby="settings-dialog-title" onclose={onClose} onclick={(e) => e.target === e.currentTarget && onClose()}>
  <div class="modal__heading">
    <h2 id="settings-dialog-title">设置</h2>
    <button class="modal__close" type="button" onclick={onClose} aria-label="关闭">×</button>
  </div>
  <div class="settings-body">
    <label class="setting-row">
      <span><strong>开机启动</strong><small>登录系统后自动启动托盘</small></span>
      <input type="checkbox" role="switch" checked={settings.autostart} onchange={() => onToggleSetting("autostart")} />
    </label>
    <label class="setting-row">
      <span><strong>启动时隐藏</strong><small>直接在系统托盘中运行</small></span>
      <input type="checkbox" role="switch" checked={settings.startHidden} onchange={() => onToggleSetting("startHidden")} />
    </label>
    <label class="setting-row">
      <span><strong>系统通知</strong><small>仅通知完成、失败、审批和提问</small></span>
      <input type="checkbox" role="switch" checked={settings.notifications} onchange={() => onToggleSetting("notifications")} />
    </label>
    <MeshPanel {peers} loading={peersLoading} onAction={onMeshAction} />
  </div>
  <div class="modal__actions">
    <button class="secondary-button" type="button" onclick={onReconnectAll}>重新连接所有服务</button>
    <span class="modal__spacer"></span>
    <button class="text-button" type="button" onclick={onClose}>完成</button>
  </div>
</dialog>