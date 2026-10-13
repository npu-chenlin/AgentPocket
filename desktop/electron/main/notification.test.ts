import { describe, expect, it } from "vitest";

import {
  loadManualPeers,
  parseOnlinePeers,
  peersFilePath,
  probePeer,
} from "./discovery";
import { validateToml } from "./kimi-config";
import { NotificationPolicy, notificationBody, notificationTitle } from "./notification";
import type { AgentEvent, AgentEventKind } from "./protocol/state";
import { reconnectLabel, tooltipText } from "./tray";
import type { ServerStatusView } from "./monitor/manager";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

/** 与 core/src/discovery.rs 的 FIXTURE 同源，防止字段名漂移。 */
const TAILSCALE_FIXTURE = JSON.stringify({
  Self: { HostName: "self-host", TailscaleIPs: ["100.64.0.1"], Online: true },
  Peer: {
    k1: {
      HostName: "peer-a",
      TailscaleIPs: ["100.64.0.2", "fd7a:115c:a1e0::1"],
      Online: true,
    },
    k2: { HostName: "peer-b", TailscaleIPs: ["100.64.0.3"], Online: false },
    k3: { HostName: "peer-c", Online: true },
  },
});

describe("tailscale status 解析", () => {
  it("只留在线且带 IPv4 的 peer", () => {
    expect(parseOnlinePeers(TAILSCALE_FIXTURE)).toEqual([
      { name: "peer-a", ip: "100.64.0.2" },
    ]);
  });

  it("顶层 Self 不算 peer", () => {
    const names = parseOnlinePeers(TAILSCALE_FIXTURE).map((peer) => peer.name);
    expect(names).not.toContain("self-host");
  });

  it("只有 IPv6 的 peer 被排除", () => {
    const json = JSON.stringify({
      Peer: {
        k: { HostName: "v6-only", TailscaleIPs: ["fd7a:115c:a1e0::1"], Online: true },
      },
    });
    expect(parseOnlinePeers(json)).toEqual([]);
  });

  it("输出按名字排序，保证顺序稳定", () => {
    const json = JSON.stringify({
      Peer: {
        a: { HostName: "zeta", TailscaleIPs: ["100.64.0.9"], Online: true },
        b: { HostName: "alpha", TailscaleIPs: ["100.64.0.8"], Online: true },
      },
    });
    expect(parseOnlinePeers(json).map((peer) => peer.name)).toEqual(["alpha", "zeta"]);
  });

  it("非法 JSON 与缺 Peer 都当作没发现", () => {
    expect(parseOnlinePeers("not json")).toEqual([]);
    expect(parseOnlinePeers("{}")).toEqual([]);
    expect(parseOnlinePeers(JSON.stringify({ Peer: null }))).toEqual([]);
  });
});

describe("手动 peer", () => {
  it("文件缺失时返回空", () => {
    const dir = mkdtempSync(join(tmpdir(), "ap-disc-"));
    expect(loadManualPeers(dir)).toEqual([]);
  });

  it("读出登记的节点并标记为 manual", () => {
    const dir = mkdtempSync(join(tmpdir(), "ap-disc-"));
    writeFileSync(
      peersFilePath(dir),
      JSON.stringify({ peers: [{ name: "桌面机", host: "100.64.0.9" }] }),
      "utf8",
    );
    expect(loadManualPeers(dir)).toEqual([
      { name: "桌面机", host: "100.64.0.9", version: null, manual: true },
    ]);
  });

  it("语法错的 peers.json 不让整个发现失败", () => {
    const dir = mkdtempSync(join(tmpdir(), "ap-disc-"));
    writeFileSync(peersFilePath(dir), "{ 手写的坏 JSON", "utf8");
    expect(loadManualPeers(dir)).toEqual([]);
  });

  it("缺 host 的条目被跳过", () => {
    const dir = mkdtempSync(join(tmpdir(), "ap-disc-"));
    writeFileSync(peersFilePath(dir), JSON.stringify({ peers: [{ name: "没地址" }] }), "utf8");
    expect(loadManualPeers(dir)).toEqual([]);
  });
});

describe("探测不可达节点", () => {
  it("连不上返回 null 而不是抛错", async () => {
    // 127.0.0.1:1 上不会有服务在听；离线节点是常态不是错误。
    await expect(probePeer("127.0.0.1", "dead", 500)).resolves.toBeNull();
  });
});

describe("config.toml 校验", () => {
  it("合法 TOML 通过", () => {
    expect(() =>
      validateToml(
        ['model = "kimi-k2"', "", "[provider]", 'base_url = "http://x"', "enabled = true"].join(
          "\n",
        ),
      ),
    ).not.toThrow();
  });

  it("注释里的引号与括号不算未闭合", () => {
    expect(() => validateToml('a = "x" # 这里有 " 和 [ 符号\n')).not.toThrow();
  });

  it("未闭合字符串报错", () => {
    expect(() => validateToml('a = "没关\n')).toThrow(/未闭合的字符串/);
  });

  it("未闭合的 [ 报错", () => {
    expect(() => validateToml("[provider\nkey = 1\n")).toThrow(/未闭合的 \[/);
  });

  it("多余的 ] 报错", () => {
    expect(() => validateToml("a = 1 ]\n")).toThrow(/多余的 \]/);
  });

  it("顶层键重复报错", () => {
    expect(() => validateToml("a = 1\nb = 2\na = 3\n")).toThrow(/顶层的键 a 重复/);
  });

  it("同一段内的键重复报错并指出段名", () => {
    expect(() => validateToml("[x]\na = 1\na = 2\n")).toThrow(/段 \[x\] 的键 a 重复/);
  });

  it("不同段里的同名键不算重复", () => {
    expect(() => validateToml('[x]\na = 1\n\n[y]\na = 2\n')).not.toThrow();
  });

  it("非法控制字符报错", () => {
    expect(() => validateToml("a = \"x\u0001y\"\n")).toThrow(/控制字符/);
  });
});

function event(overrides: Partial<AgentEvent> = {}): AgentEvent {
  return {
    serverId: "s1",
    sessionId: "sess-1",
    sessionTitle: "重构登录",
    kind: "completed",
    eventKey: "e1",
    body: null,
    // 必须晚于通知时间基准，否则会被"启动前的旧事件"这道闸拦掉。
    occurredAt: new Date("2026-01-01T00:30:00Z"),
    ...overrides,
  };
}

describe("通知", () => {
  const started = new Date("2026-01-01T00:00:00Z");
  const server = { name: "工作站", host: "100.64.0.2", port: 4096, backend: "kimi" as const };

  it("同一事件半小时内只提醒一次", () => {
    const policy = new NotificationPolicy();
    const at = new Date("2026-01-01T01:00:00Z");
    expect(policy.shouldSend(event(), started, at)).toBe(true);
    expect(policy.shouldSend(event(), started, at)).toBe(false);
    // 超过 TTL 后重新允许。
    expect(policy.shouldSend(event(), started, new Date(at.getTime() + 31 * 60_000))).toBe(true);
  });

  it("不同服务的同一 eventKey 不算重复", () => {
    const policy = new NotificationPolicy();
    const at = new Date("2026-01-01T01:00:00Z");
    expect(policy.shouldSend(event({ serverId: "s1" }), started, at)).toBe(true);
    expect(policy.shouldSend(event({ serverId: "s2" }), started, at)).toBe(true);
  });

  it("不同会话的同一 eventKey 不算重复", () => {
    const policy = new NotificationPolicy();
    const at = new Date("2026-01-01T01:00:00Z");
    expect(policy.shouldSend(event({ sessionId: "a" }), started, at)).toBe(true);
    expect(policy.shouldSend(event({ sessionId: "b" }), started, at)).toBe(true);
  });

  it("监控启动前发生的事件不通知", () => {
    const policy = new NotificationPolicy();
    // 事件发生在"监控启动"之前——那是服务端基线里的陈年旧账。
    const stale = event({ occurredAt: new Date("2025-12-31T23:00:00Z") });
    expect(policy.shouldSend(stale, started)).toBe(false);
  });

  it("恰好等于启动时刻的事件也不通知", () => {
    const policy = new NotificationPolicy();
    expect(policy.shouldSend(event({ occurredAt: started }), started)).toBe(false);
  });

  it("缓存容量有界", () => {
    const policy = new NotificationPolicy(60_000, 4);
    const at = new Date("2026-01-01T01:00:00Z");
    for (let i = 0; i < 50; i += 1) {
      policy.shouldSend(event({ eventKey: `e${i}` }), started, at);
    }
    // 超出容量的老键已被丢弃，所以最早的键又允许通过了。
    expect(policy.shouldSend(event({ eventKey: "e0" }), started, at)).toBe(true);
    // 而仍在容量内的键还是被去重。
    expect(policy.shouldSend(event({ eventKey: "e49" }), started, at)).toBe(false);
  });

  it("正文带服务名与摘要", () => {
    expect(notificationBody(event({ body: "已完成 3 个文件" }), server)).toBe(
      "[工作站] 已完成 3 个文件",
    );
  });

  it("没有摘要时退回会话标题", () => {
    expect(notificationBody(event(), server)).toBe("[工作站] 重构登录");
  });

  it("服务名为空时用 host:port", () => {
    expect(
      notificationBody(event({ body: null, sessionTitle: null }), {
        ...server,
        name: "  ",
      }),
    ).toBe("[100.64.0.2:4096]");
  });

  it("标题按后端与事件类型组合", () => {
    expect(notificationTitle("kimi", "completed")).toBe("Kimi Code · 任务完成");
    expect(notificationTitle("dsh", "approvalRequired")).toBe("DeepSeek Harness · 等待审批");
    expect(notificationTitle("opencode", "failed")).toBe("OpenCode · 任务失败");
    expect(notificationTitle("opencode", "questionRequired")).toBe("OpenCode · 待回答");
    expect(notificationTitle("kimi", "pinnedFinished")).toBe("Kimi Code · 等你介入");
  });
});

describe("托盘文案", () => {
  function status(connected: boolean, activeCount: number): ServerStatusView {
    return {
      connected,
      activeCount,
      sessions: [],
      serverVersion: null,
      lastCheckedAt: null,
      error: null,
    };
  }

  it("tooltip 汇总在线数与运行中任务", () => {
    expect(
      tooltipText(
        { a: status(true, 2), b: status(true, 0), c: status(false, 0) },
        3,
      ),
    ).toBe("已连接 2/3 台，运行 2 个任务");
  });

  it("没有服务时是 0/0", () => {
    expect(tooltipText({}, 0)).toBe("已连接 0/0 台，运行 0 个任务");
  });

  it("重连菜单在有任务运行时带数量", () => {
    expect(reconnectLabel(0)).toBe("重连全部");
    expect(reconnectLabel(3)).toBe("重连全部（3 个运行中）");
  });
});

// 让未使用的类型导入参与类型检查，防止 AgentEventKind 被悄悄删掉。
export type _KindCheck = AgentEventKind;