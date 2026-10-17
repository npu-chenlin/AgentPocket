/**
 * dsh cookie 的签发规则。
 *
 * 这是这次改动里最容易悄悄改坏的地方：签名覆盖的是 JSON.stringify 出来的那个
 * 字节串，键序一改签名就对不上，而症状只是 401——看起来像"密钥填错了"。
 * 所以这里钉住三个已知地址的 cookie 名，以及一个固定时间点的完整签名。
 */

import { createHmac } from "node:crypto";

import { describe, expect, it } from "vitest";

import { dshAuthority, dshCookie, dshCookieHeaders, dshCookieName, mintDshCookie } from "./dsh-auth";
import type { ServerConfig } from "./model";

/** dsh 实际写出的那种 32 字节 base64url 密钥。 */
const SECRET = "Vma9wVA3-rjRiCS6JJY0ynuhEC80pRyv9fJLDzLNEZI";

function server(overrides: Partial<ServerConfig> = {}): ServerConfig {
  return {
    id: "s1",
    name: "dsh",
    host: "100.115.54.102",
    port: 3080,
    token: SECRET,
    backend: "dsh",
    ...overrides,
  };
}

/** 从 `name=value` 里拆开。 */
function parts(header: string): { name: string; value: string } {
  const at = header.indexOf("=");
  return { name: header.slice(0, at), value: header.slice(at + 1) };
}

describe("dsh cookie 名", () => {
  it("与 dsh 自己的推导一致", () => {
    // dsh: dsh-auth- + base64url(sha256("host:port"))；这三个值与实测一致。
    expect(dshCookieName("100.115.54.102:3080")).toBe(
      "dsh-auth-UoviXftkqFTciBFSBAmCAxlDngpyRq75w2EgUFHInR8",
    );
    expect(dshCookieName("127.0.0.1:3080")).toBe(
      "dsh-auth-VPhEEcLKeqRDBoBalzN2Nm7CnfxKhLE00pKIDWxt1sw",
    );
    expect(dshCookieName("cvlife-system-product-name.tail4a0ecb.ts.net:3080")).toBe(
      "dsh-auth-o4_PIcdT9XDWtpi4ZI2CsszAA_n2naU3MXccfl2px4s",
    );
  });
});

describe("dsh cookie 签发", () => {
  it("值是三段式，且载荷键序与 dsh 一致", () => {
    const cookie = mintDshCookie(SECRET, "100.115.54.102:3080", 1_792_233_000_000)!;
    const { name, value } = parts(cookie.header);
    expect(name).toBe(dshCookieName("100.115.54.102:3080"));

    const segments = value.split(".");
    expect(segments).toHaveLength(3);
    expect(segments[0]).toBe("v1");

    const body = Buffer.from(segments[1].replace(/-/g, "+").replace(/_/g, "/"), "base64").toString("utf8");
    // 键序固定，签到过期时间也是 issuedAt + 30 天。
    expect(body).toBe(
      '{"version":1,"authority":"100.115.54.102:3080","issuedAt":1792233000000,"expiresAt":1794825000000}',
    );
  });

  it("签名与独立算出的 HMAC 一致", () => {
    const cookie = mintDshCookie(SECRET, "127.0.0.1:3080", 1_792_233_000_000)!;
    const { value } = parts(cookie.header);
    const [, body, signature] = value.split(".");
    const key = Buffer.from(SECRET.replace(/-/g, "+").replace(/_/g, "/"), "base64");
    const expected = createHmac("sha256", key).update(body).digest()
      .toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
    expect(signature).toBe(expected);
  });

  it("同一输入的签名可复现", () => {
    expect(mintDshCookie(SECRET, "127.0.0.1:3080", 1_792_233_000_000))
      .toEqual(mintDshCookie(SECRET, "127.0.0.1:3080", 1_792_233_000_000));
  });

  it("不同 authority 不共用 cookie", () => {
    const a = mintDshCookie(SECRET, "a.example:3080", 1)!;
    const b = mintDshCookie(SECRET, "b.example:3080", 1)!;
    expect(parts(a.header).name).not.toBe(parts(b.header).name);
  });

  it("密钥不合法时返回 null 而不是签出坏 cookie", () => {
    // 长度 %4==1 在 base64 里不可能；含非 base64url 字符也直接拒。
    expect(mintDshCookie("abcde", "h:1")).toBeNull();
    expect(mintDshCookie("not base64!", "h:1")).toBeNull();
    expect(mintDshCookie("", "h:1")).toBeNull();
    expect(mintDshCookie("  ", "h:1")).toBeNull();
  });
});

describe("dsh 服务取值", () => {
  it("authority 是 host:port", () => {
    expect(dshAuthority(server())).toBe("100.115.54.102:3080");
  });

  it("非 dsh 后端不给 cookie", () => {
    expect(dshCookie(server({ backend: "kimi" }))).toBeNull();
    expect(dshCookieHeaders(server({ backend: "opencode" }))).toEqual({});
  });

  it("密钥缺失时不给 Cookie 头，请求照常发出", () => {
    expect(dshCookieHeaders(server({ token: "" }))).toEqual({});
  });

  it("密钥可用时给出 Cookie 头", () => {
    const headers = dshCookieHeaders(server());
    expect(Object.keys(headers)).toEqual(["Cookie"]);
    expect(headers.Cookie).toMatch(/^dsh-auth-[A-Za-z0-9_-]+=v1\./);
  });
});
