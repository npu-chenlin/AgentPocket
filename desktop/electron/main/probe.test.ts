/**
 * dsh 探测的判定规则。
 *
 * 这里测的是"什么响应算 dsh"，不碰网络：把 manifest 判别拆成纯函数，
 * 因为最容易改坏的就是它——判错了会把这个服务当成别的后端，界面上看不出来。
 */

import { describe, expect, it } from "vitest";

import { isDshManifest, isDshUnauthenticated } from "./probe";

describe("dsh manifest 指纹", () => {
  it("认 dsh 自己的清单", () => {
    expect(isDshManifest(200, JSON.stringify({ name: "DeepSeek Harness", short_name: "DSH" }))).toBe(true);
    // 只要有 name 就够（short_name 是可选字段）。
    expect(isDshManifest(200, JSON.stringify({ name: "DeepSeek Harness" }))).toBe(true);
    expect(isDshManifest(200, JSON.stringify({ short_name: "DSH" }))).toBe(true);
  });

  it("404 / HTML / 别的 JSON 都不算", () => {
    // kimi 上这个路径是 404。
    expect(isDshManifest(404, "Not found")).toBe(false);
    // opencode 上返回一段 HTML。
    expect(isDshManifest(200, "<!doctype html><html>…")).toBe(false);
    // 别的应用的 manifest 不能误判。
    expect(isDshManifest(200, JSON.stringify({ name: "Other App" }))).toBe(false);
    expect(isDshManifest(200, "{}")).toBe(false);
  });
});

describe("dsh 未认证挑战", () => {
  it("401 + dsh 那句话才算", () => {
    expect(
      isDshUnauthenticated(401, "dsh web authentication required; reopen the URL printed by dsh web."),
    ).toBe(true);
  });

  it("别的 401 不算", () => {
    expect(isDshUnauthenticated(401, "unauthorized")).toBe(false);
    expect(isDshUnauthenticated(401, "Not found")).toBe(false);
    // 状态码不对也不算。
    expect(
      isDshUnauthenticated(200, "dsh web authentication required; reopen the URL printed by dsh web."),
    ).toBe(false);
  });
});
