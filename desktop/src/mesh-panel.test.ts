import { describe, expect, it } from "vitest";
import type { MeshPeerView } from "./model";
import { renderMeshPeerList } from "./mesh-panel";

function peerFixture(overrides: Partial<MeshPeerView> = {}): MeshPeerView {
  return {
    name: "工作站",
    host: "100.64.0.7",
    version: null,
    online: true,
    manual: false,
    ...overrides,
  };
}

describe("renderMeshPeerList", () => {
  it("renders online rows with name, host, version and pull/push buttons", () => {
    const html = renderMeshPeerList({
      peers: [peerFixture({ name: "Home", host: "100.1.2.3", version: "0.36.0" })],
      loading: false,
    });
    expect(html).toContain("Home");
    expect(html).toContain("100.1.2.3");
    expect(html).toContain("v0.36.0");
    expect(html).toContain("mesh-dot--online");
    expect(html).toContain('data-mesh-action="pull"');
    expect(html).toContain('data-mesh-action="push"');
    expect(html).toContain('data-mesh-host="100.1.2.3"');
  });

  it("omits the version segment when the peer reports none", () => {
    const html = renderMeshPeerList({ peers: [peerFixture()], loading: false });
    expect(html).not.toContain("mesh-peer-version");
  });

  it("shows offline manual peers with a gray dot and offline label", () => {
    const html = renderMeshPeerList({
      peers: [peerFixture({ online: false, manual: true })],
      loading: false,
    });
    expect(html).toContain("离线");
    expect(html).not.toContain("mesh-dot--online");
  });

  it("shows a discovering placeholder while loading", () => {
    const html = renderMeshPeerList({ peers: [], loading: true });
    expect(html).toContain("发现中…");
    expect(html).not.toContain("data-mesh-action");
  });

  it("shows the empty state text when no peers are found", () => {
    const html = renderMeshPeerList({ peers: [], loading: false });
    expect(html).toContain("未发现 AgentPocket 节点");
  });

  it("escapes peer fields", () => {
    const html = renderMeshPeerList({
      peers: [peerFixture({ name: '<img src=x onerror="boom">', host: 'h"<b>' })],
      loading: false,
    });
    expect(html).not.toContain("<img");
    expect(html).toContain("&lt;img");
    expect(html).not.toContain('h"<b>');
  });

  it("never renders token-like content", () => {
    const html = renderMeshPeerList({
      peers: [peerFixture({ name: "工作站", host: "100.64.0.7", version: "0.36.0" })],
      loading: false,
    });
    expect(html).not.toContain("token");
    expect(html).not.toContain("令牌");
  });
});
