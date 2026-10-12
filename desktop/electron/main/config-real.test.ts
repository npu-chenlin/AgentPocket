import { describe, expect, it } from "vitest";

import { ConfigStore } from "./config-store";

/**
 * 确认 TS 版 ConfigStore 能读出 Tauri 版写下的真实配置。
 *
 * 这是迁移期的一次性核对：读不出来就说明两版格式对不上，属于真问题，
 * 不该用 try/catch 跳过去。
 */
const REAL_CONFIG_DIR = `${process.env.HOME}/.local/share/com.local.agentpocket.desktop`;

describe("真实配置兼容性", () => {
  it("能读出 Tauri 版写下的配置", async () => {
    const store = ConfigStore.open(REAL_CONFIG_DIR);
    const config = await store.load();

    console.log("schema:", config.schema, "activeId:", config.activeId);
    console.log(
      "服务:",
      config.servers.map((s) => `${s.name}/${s.backend}/${s.host}:${s.port}`),
    );
    expect(config.schema).toBe(1);
    expect(Array.isArray(config.servers)).toBe(true);
  });
});