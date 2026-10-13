import { describe, expect, it } from "vitest";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { ConfigStore } from "./config-store";

describe("旧版配置格式兼容性（隔离样本）", () => {
  it("读取旧版字段，不触碰 HOME 或真实用户配置", async () => {
    const directory = await mkdtemp(join(tmpdir(), "agentpocket-config-test-"));
    try {
      await mkdir(directory, { recursive: true });
      await writeFile(join(directory, "config.json"), JSON.stringify({ schema: 1, activeId: "sample",
        servers: [{ id: "sample", name: "本地样本", host: "127.0.0.1", port: 3080, backend: "dsh", token: "synthetic" }] }));
      const config = await ConfigStore.open(directory).load();
      expect(config.schema).toBe(1);
      expect(config.servers[0].id).toBe("sample");
    } finally { await rm(directory, { recursive: true, force: true }); }
  });
});
