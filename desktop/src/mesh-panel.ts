import type { MeshPeerView } from "./model";
import { escapeHtml } from "./server-list";

export interface MeshSectionState {
  peers: MeshPeerView[];
  loading: boolean;
}

/** 设置对话框 Mesh 区块的 peer 列表（纯函数，无 DOM 副作用）。 */
export function renderMeshPeerList(state: MeshSectionState): string {
  if (state.loading) {
    return `<p class="mesh-placeholder">发现中…</p>`;
  }
  if (state.peers.length === 0) {
    return `<p class="mesh-placeholder">未发现 AgentPocket 节点</p>`;
  }
  return state.peers.map(renderMeshPeerRow).join("");
}

function renderMeshPeerRow(peer: MeshPeerView): string {
  const name = escapeHtml(peer.name);
  const host = escapeHtml(peer.host);
  const version = peer.version
    ? `
        <span class="server-meta-sep" aria-hidden="true">·</span>
        <span class="mesh-peer-version">v${escapeHtml(peer.version)}</span>`
    : "";
  const offline = peer.online
    ? ""
    : `
        <span class="server-meta-sep" aria-hidden="true">·</span>
        <span class="mesh-peer-offline">离线</span>`;
  // 手动登记的 peer 才允许删除；发现得到的 tailscale 节点不显示 ✕。
  const remove = peer.manual
    ? `
        <button class="mesh-remove" type="button" data-mesh-action="remove" data-mesh-host="${host}" title="删除该 peer">×</button>`
    : "";

  return `
    <div class="mesh-peer">
      <span class="mesh-dot${peer.online ? " mesh-dot--online" : ""}" aria-hidden="true"></span>
      <span class="mesh-peer-copy">
        <strong class="mesh-peer-name">${name}</strong>
        <span class="mesh-peer-meta">
          <span class="mesh-peer-host">${host}</span>${version}${offline}
        </span>
      </span>
      <span class="mesh-peer-actions">
        <button class="text-button" type="button" data-mesh-action="pull" data-mesh-host="${host}" aria-label="从 ${name} 拉取 config.toml">拉取</button>
        <button class="text-button" type="button" data-mesh-action="push" data-mesh-host="${host}" aria-label="推送 config.toml 到 ${name}">推送</button>
      </span>${remove}
    </div>`;
}
