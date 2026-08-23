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
  const kimi = peer.kimiVersion
    ? `
        <span class="server-meta-sep" aria-hidden="true">·</span>
        <span class="mesh-peer-version">kimi v${escapeHtml(peer.kimiVersion)}</span>`
    : "";
  const web = peer.webActive
    ? `
        <span class="server-meta-sep" aria-hidden="true">·</span>
        <span class="mesh-peer-version">web:${peer.webPort ?? "-"}</span>`
    : "";
  // 手动登记的节点才允许删除；Tailscale 自动发现的节点不显示 ✕。
  const remove = peer.manual
    ? `
        <button class="mesh-remove" type="button" data-mesh-action="remove" data-mesh-host="${host}" title="删除该节点">×</button>`
    : "";

  return `
    <div class="mesh-peer">
      <span class="mesh-dot${peer.online ? " mesh-dot--online" : ""}" aria-hidden="true"></span>
      <span class="mesh-peer-copy">
        <strong class="mesh-peer-name">${name}</strong>
        <span class="mesh-peer-meta">
          <span class="mesh-peer-host">${host}</span>${version}${offline}${kimi}${web}
        </span>
      </span>
      <span class="mesh-peer-actions">
        <button class="text-button" type="button" data-mesh-action="pull" data-mesh-host="${host}" aria-label="从 ${name} 获取 Kimi 配置">获取</button>
        <button class="text-button" type="button" data-mesh-action="push" data-mesh-host="${host}" aria-label="发送 Kimi 配置到 ${name}">发送</button>
        <button class="text-button" type="button" data-mesh-action="upgrade" data-mesh-host="${host}" aria-label="升级 ${name} 的 Kimi Code CLI">升级</button>
        <button class="text-button" type="button" data-mesh-action="restart-web" data-mesh-host="${host}" aria-label="重启 ${name} 的 Kimi Web 服务">重启</button>
      </span>${remove}
    </div>`;
}
