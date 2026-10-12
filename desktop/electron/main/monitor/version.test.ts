/**
 * 版本采集。
 *
 * 两个后端的版本端点与字段名都不一样。字段名写错时**不会报错**，只会静默变成
 * null，看起来像"服务端没提供版本信息"——Kimi 那条就是这么坏掉的。所以这两条
 * 解析函数各自导出并直接测真代码。
 */

import { describe, expect, it } from "vitest";

import { parseServerVersion as parseKimiVersion } from "./kimi";
import { parseServerVersion as parseOpencodeVersion } from "./opencode";

/** 逐字取自 `curl /api/v1/meta` 的真实响应。 */
const KIMI_META = JSON.stringify({
  code: 0,
  msg: "success",
  data: {
    server_version: "2.1.1",
    capabilities: { websocket: true, file_upload: true, fs_query: true, mcp: true },
    server_id: "01M4HT94ARRM34KZFPKKCNFVAX",
    dangerous_bypass_auth: true,
  },
});

/** 逐字取自 `curl /api/info` 的真实响应。 */
const OPENCODE_INFO = JSON.stringify({
  version: "2.0.26",
  pid: 1171623,
  urls: ["http://127.0.0.1:4096", "http://100.115.54.102:4096"],
  paths: { tmp: "/tmp/opencode" },
  capabilities: { persistentPty: true },
});

describe("Kimi 版本", () => {
  it("从真实响应里取出 2.1.1", () => {
    expect(parseKimiVersion(KIMI_META)).toBe("2.1.1");
  });

  it("字段是 server_version，写成 version 会得到 null（且不报错）", () => {
    // 这条锁住那个静默 bug：曾经读的是 data.version，于是永远是 null。
    const wrongField = JSON.stringify({ data: { version: "2.1.1" } });
    expect(parseKimiVersion(wrongField)).toBeNull();
  });

  it("缺字段时返回 null 而不是抛错", () => {
    expect(parseKimiVersion(JSON.stringify({ data: {} }))).toBeNull();
    expect(parseKimiVersion("{}")).toBeNull();
  });

  it("非法 JSON 抛错，让调用方能分辨是没有版本还是响应坏了", () => {
    expect(() => parseKimiVersion("<html>502</html>")).toThrow();
  });
});

describe("opencode 版本", () => {
  it("从真实响应里取出 2.0.26", () => {
    expect(parseOpencodeVersion(OPENCODE_INFO)).toBe("2.0.26");
  });

  it("字段是顶层 version，写成 data.version 会得到 null", () => {
    expect(parseOpencodeVersion(JSON.stringify({ data: { version: "2.0.26" } }))).toBeNull();
  });

  it("缺字段时返回 null 而不是抛错", () => {
    expect(parseOpencodeVersion(JSON.stringify({ pid: 1 }))).toBeNull();
  });

  it("非法 JSON 抛错", () => {
    expect(() => parseOpencodeVersion("not json")).toThrow();
  });
});