import { mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

import { ConfigStore, type ImportPreviewData } from "./config-store";
import { defaultSettings, type AppConfig } from "./model";

async function scratch(): Promise<ConfigStore> {
  const dir = await mkdtemp(join(tmpdir(), "ap-config-"));
  return ConfigStore.open(dir);
}

function config(overrides: Partial<AppConfig> = {}): AppConfig {
  return { schema: 1, activeId: null, servers: [], settings: defaultSettings(), ...overrides };
}

function serverEntry(overrides: Record<string, unknown> = {}) {
  return {
    id: "a",
    name: "A",
    host: "100.64.0.2",
    port: 3080,
    backend: "kimi",
    token: "",
    ...overrides,
  };
}

describe("ConfigStore", () => {
  it("缺少配置文件时返回默认配置", async () => {
    const store = await scratch();
    const config = await store.load();
    expect(config.servers).toEqual([]);
    expect(config.schema).toBe(1);
  });

  it("缺 schema 的旧配置按 1 处理，并补齐缺失的设置项", async () => {
    const store = await scratch();
    await writeFile(
      store.path,
      JSON.stringify({
        activeId: null,
        servers: [],
        settings: { startHidden: true, autostart: false, notifications: true },
      }),
    );
    const loaded = await store.load();
    expect(loaded.schema).toBe(1);
    expect(loaded.settings.autostart).toBe(false);
    expect(loaded.settings.meshPeers).toEqual([]);
  });

  it("非法服务条目被跳过，其余照常加载", async () => {
    const store = await scratch();
    await writeFile(
      store.path,
      JSON.stringify({
        schema: 1,
        activeId: null,
        servers: [serverEntry({ id: "good" }), serverEntry({ id: "bad", port: 0 })],
        settings: {},
      }),
    );
    const loaded = await store.load();
    expect(loaded.servers.map((s) => s.id)).toEqual(["good"]);
  });

  it("配置损坏时直接报出原因，不静默兜底", async () => {
    const store = await scratch();
    await writeFile(store.path, "{ 这不是合法 JSON");
    await expect(store.load()).rejects.toThrow(/JSON is invalid/);
  });

  it("不支持的 schema 报出实际版本", async () => {
    const store = await scratch();
    await writeFile(
      store.path,
      JSON.stringify({ schema: 99, activeId: null, servers: [], settings: {} }),
    );
    await expect(store.load()).rejects.toThrow(/schema 99/);
  });

  it("activeId 指向已删除的服务时丢弃", async () => {
    const store = await scratch();
    await writeFile(
      store.path,
      JSON.stringify({ schema: 1, activeId: "gone", servers: [serverEntry()], settings: {} }),
    );
    const loaded = await store.load();
    expect(loaded.activeId).toBeNull();
  });

  it("重复 id 直接报出是哪个 id", async () => {
    const store = await scratch();
    await writeFile(
      store.path,
      JSON.stringify({
        schema: 1,
        activeId: null,
        servers: [serverEntry({ id: "dup" }), serverEntry({ id: "dup" })],
        settings: {},
      }),
    );
    await expect(store.load()).rejects.toThrow(/duplicate server id: dup/);
  });

  it("缺少 id 的老条目补新 id 并补空 token", async () => {
    const store = await scratch();
    await writeFile(
      store.path,
      JSON.stringify({
        schema: 1,
        activeId: null,
        servers: [{ name: "A", host: "100.64.0.2", port: 3080, backend: "kimi" }],
        settings: {},
      }),
    );
    const loaded = await store.load();
    expect(loaded.servers[0].id).toMatch(/^[0-9a-f-]{36}$/);
    expect(loaded.servers[0].token).toBe("");
  });

  it("导入预览统计合法与非法条目", async () => {
    const store = await scratch();
    const data = await store.previewImportText(
      JSON.stringify({
        schema: 1,
        activeId: null,
        servers: [serverEntry({ id: "ok" }), serverEntry({ id: "bad", port: 0 })],
        settings: {},
      }),
    );
    expect(data.preview.validServers).toBe(1);
    expect(data.preview.invalidServers).toBe(1);
  });

  it("导入内容不是对象时报错", async () => {
    const store = await scratch();
    // 错误文案沿用 Rust 侧原样（那边同样只接受对象）。
    await expect(store.previewImportText("[]")).rejects.toThrow(
      /must be an object or an array/,
    );
  });

  it("merge 模式按 id 覆盖，其余保留", async () => {
    const store = await scratch();
    // 导入是针对磁盘上的最新配置做的，所以先落盘当前状态。
    const current = config({
      servers: [
        { id: "keep", name: "Keep", host: "h", port: 1, backend: "kimi", token: "" },
        { id: "over", name: "Old", host: "h", port: 2, backend: "kimi", token: "" },
      ],
    });
    await store.save(current);

    const data: ImportPreviewData = {
      preview: { validServers: 1, invalidServers: 0, servers: [] },
      config: config({
        servers: [{ id: "over", name: "New", host: "h", port: 3, backend: "dsh", token: "" }],
      }),
    };

    const merged = await store.applyImportAndSave(data, "merge");
    expect(merged.servers.map((s) => s.id)).toEqual(["keep", "over"]);
    expect(merged.servers[1].name).toBe("New");
    // 设置保持本机的，不被导入覆盖。
    expect(merged.settings).toEqual(current.settings);
  });

  it("replace 模式清空原有服务但保留设置", async () => {
    const store = await scratch();
    await store.save(
      config({
        servers: [{ id: "old", name: "Old", host: "h", port: 1, backend: "kimi", token: "" }],
        settings: { startHidden: false, autostart: false, notifications: false, meshPeers: [] },
      }),
    );
    const data: ImportPreviewData = {
      preview: { validServers: 1, invalidServers: 0, servers: [] },
      config: config({
        servers: [{ id: "new", name: "New", host: "h", port: 2, backend: "dsh", token: "" }],
      }),
    };

    const replaced = await store.applyImportAndSave(data, "replace");
    expect(replaced.servers.map((s) => s.id)).toEqual(["new"]);
    expect(replaced.settings.startHidden).toBe(false);
    // activeId 失效时回退到第一个服务。
    expect(replaced.activeId).toBe("new");
  });

  it("导出格式不含桌面本地设置", async () => {
    const store = await scratch();
    const text = store.exportText(
      config({
        activeId: "a",
        servers: [serverEntry({ token: "tok" }) as never],
      }),
    );
    const parsed = JSON.parse(text);
    expect(parsed.settings).toBeUndefined();
    expect(parsed.servers[0].token).toBe("tok");
    expect(parsed.activeId).toBe("a");
  });

  it("update 是读-改-写事务", async () => {
    const store = await scratch();
    await store.save(config());
    const updated = await store.update((draft) => {
      draft.servers.push(serverEntry() as never);
      draft.activeId = "a";
    });
    expect(updated.servers).toHaveLength(1);

    const reloaded = await store.load();
    expect(reloaded.servers).toHaveLength(1);
    expect(reloaded.activeId).toBe("a");
  });

  it("并发 update 不会互相覆盖", async () => {
    const store = await scratch();
    await store.save(config());
    await Promise.all([
      store.update((draft) => {
        draft.servers.push(serverEntry({ id: "x" }) as never);
      }),
      store.update((draft) => {
        draft.servers.push(serverEntry({ id: "y" }) as never);
      }),
    ]);
    const reloaded = await store.load();
    expect(reloaded.servers.map((s) => s.id).sort()).toEqual(["x", "y"]);
  });
});