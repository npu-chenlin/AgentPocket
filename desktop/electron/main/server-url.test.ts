import { describe, expect, it } from "vitest";
import {
  opencodeCredentials,
  opencodeBasicHeader,
  validateServer,
  type ServerConfig,
} from "./model";
import { buildServerUrl, opencodeServerKey } from "./server-url";

// 下面的用例逐条对应 core/src/{model,server_url}.rs 的 Rust 测试，
// 两边行为必须一致；改动任何一处都要同步。

function server(token: string, backend: ServerConfig["backend"]): ServerConfig {
  return { id: "s1", name: "Work", host: "100.64.0.2", port: 3080, token, backend };
}

describe("buildServerUrl", () => {
  it("kimi 根地址带尾斜杠", () => {
    expect(buildServerUrl(server("", "kimi")).toString()).toBe("http://100.64.0.2:3080/");
  });

  it("kimi 会话地址带 session 路径", () => {
    expect(buildServerUrl(server("", "kimi"), "sess-123").toString()).toBe(
      "http://100.64.0.2:3080/sessions/sess-123",
    );
  });

  it("kimi 会话 id 为空时打开根路径", () => {
    expect(buildServerUrl(server("", "kimi"), "").toString()).toBe("http://100.64.0.2:3080/");
  });

  it("dsh 始终用根地址", () => {
    expect(buildServerUrl(server("secret", "dsh"), "sess-123").toString()).toBe(
      "http://100.64.0.2:3080/",
    );
    expect(buildServerUrl(server("secret", "dsh")).toString()).toBe("http://100.64.0.2:3080/");
  });

  it("kimi 地址用片段携带 token", () => {
    expect(buildServerUrl(server("super-secret", "kimi"), "sess").toString()).toBe(
      "http://100.64.0.2:3080/sessions/sess#token=super-secret",
    );
    // 特殊字符按百分号编码处理，与 Android 的 Uri.encode 行为一致。
    expect(buildServerUrl(server("a b+c/d", "kimi")).toString()).toBe(
      "http://100.64.0.2:3080/#token=a%20b%2Bc%2Fd",
    );
  });

  it("dsh 地址永不携带 token", () => {
    expect(buildServerUrl(server("secret", "dsh")).toString()).toBe("http://100.64.0.2:3080/");
  });

  it("opencode 会话地址编码服务器 origin", () => {
    expect(buildServerUrl(server("opencode:secret", "opencode"), "ses_abc").toString()).toBe(
      "http://100.64.0.2:3080/server/aHR0cDovLzEwMC42NC4wLjI6MzA4MA/session/ses_abc",
    );
  });

  it("opencode 路由忽略凭据部分", () => {
    const srv = { ...server("bob:hunter2", "opencode"), host: "127.0.0.1", port: 4096 };
    expect(buildServerUrl(srv, "ses_1").toString()).toBe(
      "http://127.0.0.1:4096/server/aHR0cDovLzEyNy4wLjAuMTo0MDk2/session/ses_1",
    );
  });

  it("server key 与前端编码一致", () => {
    expect(opencodeServerKey(new URL("http://127.0.0.1:4096/"))).toBe(
      "aHR0cDovLzEyNy4wLjAuMTo0MDk2",
    );
  });
});

describe("opencode 凭据", () => {
  it("只写密码时补默认用户名", () => {
    const srv = { ...server("", "opencode"), token: "s3cret" };
    expect(opencodeCredentials(srv)).toEqual({ user: "opencode", pass: "s3cret" });
    expect(opencodeBasicHeader(srv)).toBe("Basic b3BlbmNvZGU6czNjcmV0");
  });

  it("显式 user:pass 原样解析", () => {
    const srv = { ...server("", "opencode"), token: "bob:hunter2" };
    expect(opencodeCredentials(srv)).toEqual({ user: "bob", pass: "hunter2" });
    expect(opencodeBasicHeader(srv)).toBe("Basic Ym9iOmh1bnRlcjI=");
  });

  it("密码里的冒号保留", () => {
    const srv = { ...server("", "opencode"), token: "opencode:a:b:c" };
    expect(opencodeCredentials(srv)).toEqual({ user: "opencode", pass: "a:b:c" });
  });

  it("其他后端或空凭据返回 null", () => {
    // kimi 的 token 绝不能被当成 opencode 凭据。
    const kimi = { ...server("tok", "kimi"), port: 58627 };
    expect(opencodeCredentials(kimi)).toBeNull();
    expect(opencodeBasicHeader(kimi)).toBeNull();
    const blank = { ...server("", "opencode"), token: "  " };
    expect(opencodeBasicHeader(blank)).toBeNull();
  });
});

describe("validateServer", () => {
  it("拒绝非法 host 与端口", () => {
    expect(() => validateServer(server("", "dsh"))).not.toThrow();
    expect(() =>
      validateServer({ ...server("", "dsh"), host: "http://host" }),
    ).toThrow();
    expect(() => validateServer({ ...server("", "dsh"), host: "host:3080" })).toThrow();
    expect(() => validateServer({ ...server("", "dsh"), port: 0 })).toThrow();
  });
});