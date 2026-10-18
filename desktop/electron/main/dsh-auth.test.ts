/**
 * dsh 凭据：cookie 的解析、launch token 的提取与交换。
 *
 * 关键约束（旧实现最容易出错的正是这里）：dsh 的 launch token 每次启动都变，而换来的
 * cookie 由持久密钥签发、跨重启有效。所以客户端存的是 **cookie**，不是密钥，也不自己
 * 签发——这些用例把这条边界钉住。
 */

import { afterEach, describe, expect, it, vi } from "vitest";

import {
  dshCookie,
  dshCookieHeaders,
  exchangeLaunchToken,
  extractLaunchToken,
  parseDshCookie,
  resolveDshCredential,
} from "./dsh-auth";
import type { ServerConfig } from "./model";

/** dsh 实际下发的那种 cookie（名字绑定 authority，值是三段式）。 */
const COOKIE = "dsh-auth-UoviXftkqFTciBFSBAmCAxlDngpyRq75w2EgUFHInR8=v1.eyJ2ZXJzaW9uIjoxfQ.sig";

function server(overrides: Partial<ServerConfig> = {}): ServerConfig {
  return {
    id: "s1",
    name: "dsh",
    host: "100.115.54.102",
    port: 3080,
    token: COOKIE,
    backend: "dsh",
    ...overrides,
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("cookie 解析", () => {
  it("拆出名字与值", () => {
    const cookie = parseDshCookie(COOKIE)!;
    expect(cookie.name).toBe("dsh-auth-UoviXftkqFTciBFSBAmCAxlDngpyRq75w2EgUFHInR8");
    expect(cookie.value).toBe("v1.eyJ2ZXJzaW9uIjoxfQ.sig");
    expect(cookie.header).toBe(COOKIE);
  });

  it("容忍两侧空白与 Set-Cookie 的尾部属性", () => {
    expect(parseDshCookie(`  ${COOKIE}  `)!.header).toBe(COOKIE);
    expect(parseDshCookie(`${COOKIE}; Path=/; HttpOnly`)!.header).toBe(COOKIE);
  });

  it("挡住不像 cookie 的输入", () => {
    // 裸 launch token、路径、空值都不是 cookie——早于发请求就拒掉。
    expect(parseDshCookie("")).toBeNull();
    expect(parseDshCookie(null)).toBeNull();
    expect(parseDshCookie("Vma9wVA3-rjRiCS6JJY0ynuhEC80pRyv9fJLDzLNEZI")).toBeNull();
    expect(parseDshCookie("v1.abc.def")).toBeNull();
    // 名字不是 dsh 的（例如别的服务的 cookie）也不收。
    expect(parseDshCookie("session=v1.abc.def")).toBeNull();
    // 值不是三段式。
    expect(parseDshCookie("dsh-auth-x=whatever")).toBeNull();
  });
});

describe("launch token 提取", () => {
  it("从完整启动行里取", () => {
    const line = "dsh web: http://127.0.0.1:3080/?token=xG0KCw2OUHu5BHtkYTvswyRkTFyq8k1p8LgpUKUneag (LAN: http://100.1.2.3:3080/?token=xG0KCw2OUHu5BHtkYTvswyRkTFyq8k1p8LgpUKUneag)";
    expect(extractLaunchToken(line)).toBe("xG0KCw2OUHu5BHtkYTvswyRkTFyq8k1p8LgpUKUneag");
  });

  it("没有 token 时返回 null", () => {
    expect(extractLaunchToken("dsh web: http://127.0.0.1:3080/")).toBeNull();
    expect(extractLaunchToken("")).toBeNull();
    expect(extractLaunchToken(null)).toBeNull();
  });
});

describe("launch token 交换", () => {
  it("拿到 Set-Cookie 就返回 cookie", async () => {
    const fetchMock = vi.fn(async () => new Response(null, {
      status: 303,
      headers: { "set-cookie": `${COOKIE}; Max-Age=2592000; Path=/; HttpOnly; SameSite=Strict` },
    }));
    vi.stubGlobal("fetch", fetchMock);

    const cookie = await exchangeLaunchToken(server(), "launch-token-abc");
    expect(cookie!.header).toBe(COOKIE);

    // 必须带上 ?token=，且不跟随那一次 303 跳转。
    const [url, init] = fetchMock.mock.calls[0] as unknown as [URL, RequestInit];
    expect(String(url)).toContain("token=launch-token-abc");
    expect(init.redirect).toBe("manual");
  });

  it("没有 Set-Cookie / 请求失败时返回 null", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => new Response(null, { status: 303 })));
    expect(await exchangeLaunchToken(server(), "t")).toBeNull();

    vi.stubGlobal("fetch", vi.fn(async () => { throw new Error("网络不可达"); }));
    expect(await exchangeLaunchToken(server(), "t")).toBeNull();
  });
});

describe("凭据解析入口", () => {
  it("已经是 cookie 就直接用，不发请求", async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);

    const cookie = await resolveDshCredential(server(), COOKIE);
    expect(cookie!.header).toBe(COOKIE);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("粘的是启动行就去交换", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => new Response(null, {
      status: 303,
      headers: { "set-cookie": `${COOKIE}; Path=/` },
    })));

    const cookie = await resolveDshCredential(server(), "dsh web: http://h:3080/?token=abc123");
    expect(cookie!.header).toBe(COOKIE);
  });

  it("既不是 cookie 也没有 token 时返回 null", async () => {
    vi.stubGlobal("fetch", vi.fn());
    expect(await resolveDshCredential(server(), "随便什么东西")).toBeNull();
  });
});

describe("dsh 服务取值", () => {
  it("非 dsh 后端不给 cookie", () => {
    expect(dshCookie(server({ backend: "kimi" }))).toBeNull();
    expect(dshCookieHeaders(server({ backend: "opencode" }))).toEqual({});
  });

  it("凭据缺失时不给 Cookie 头，请求照常发出", () => {
    expect(dshCookieHeaders(server({ token: "" }))).toEqual({});
  });

  it("凭据可用时给出 Cookie 头", () => {
    const headers = dshCookieHeaders(server());
    expect(Object.keys(headers)).toEqual(["Cookie"]);
    expect(headers.Cookie).toBe(COOKIE);
  });
});
