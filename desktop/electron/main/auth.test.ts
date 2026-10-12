/**
 * 凭据注入的行为约束。
 *
 * 这里测的是"往 webRequest 里塞什么"，不碰 Electron 本身：injectableHeader
 * 把 origin 匹配、已有 Authorization 保护这几条规则拆成了纯函数，方便断言。
 */

import { describe, expect, it } from "vitest";

import { shouldInject } from "./auth";
import { opencodeBasicHeader, type ServerConfig } from "./model";

function opencodeServer(token: string): ServerConfig {
  return {
    id: "s1",
    name: "工作站",
    host: "100.115.54.102",
    port: 4096,
    backend: "opencode",
    token,
  };
}

describe("OpenCode 凭据注入", () => {
  it("算出 HTTP Basic 头，用户名缺省为 opencode", () => {
    expect(opencodeBasicHeader(opencodeServer("s3cret"))).toBe(
      `Basic ${Buffer.from("opencode:s3cret", "utf8").toString("base64")}`,
    );
  });

  it("token 写成 用户名:密码 时按原样使用", () => {
    const header = opencodeBasicHeader(opencodeServer("alice:pw"));
    expect(header).toBe(`Basic ${Buffer.from("alice:pw", "utf8").toString("base64")}`);
  });

  it("token 为空时不注入", () => {
    expect(opencodeBasicHeader(opencodeServer(""))).toBeNull();
  });

  it("同源请求注入凭据", () => {
    expect(
      shouldInject(
        "http://100.115.54.102:4096/api/session",
        "http://100.115.54.102:4096",
        "Basic xyz",
        undefined,
      ),
    ).toBe("Basic xyz");
  });

  it("已带 Authorization 时不覆盖", () => {
    // 用户改了密码，服务端认可新密码时不能被本地旧值钉住。
    expect(
      shouldInject(
        "http://100.115.54.102:4096/api/session",
        "http://100.115.54.102:4096",
        "Basic 旧的",
        "Basic 服务端给的",
      ),
    ).toBeUndefined();
  });

  it("非同源请求不注入", () => {
    expect(
      shouldInject(
        "http://evil.example.com/collect",
        "http://100.115.54.102:4096",
        "Basic xyz",
        undefined,
      ),
    ).toBeUndefined();
  });

  it("前缀相同但端口不同的源不注入", () => {
    // 防"同 IP 不同端口"这类串台。
    expect(
      shouldInject(
        "http://100.115.54.102:9999/api/session",
        "http://100.115.54.102:4096",
        "Basic xyz",
        undefined,
      ),
    ).toBeUndefined();
  });
});