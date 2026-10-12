/**
 * 配置读写。从 core/src/config.rs 迁移。
 *
 * 刻意不带备份回退：配置损坏就报错，让人按真实原因排查，而不是悄悄加载一份
 * 旧配置继续跑。写入走临时文件 + 原子替换，避免半截文件。
 *
 * 配置文件格式不能变——Android 端和 daemon 都依赖它。
 */

import { randomUUID } from "node:crypto";
import { constants } from "node:fs";
import { access, mkdir, readFile, rename, writeFile } from "node:fs/promises";
import { basename, dirname, join } from "node:path";

import {
  DEFAULT_SCHEMA,
  defaultSettings,
  toSummary,
  validateServer,
  type AppConfig,
  type Backend,
  type DesktopSettings,
  type ServerConfig,
  type ServerSummary,
} from "./model";

export type ImportMode = "merge" | "replace";

export interface ImportPreview {
  validServers: number;
  invalidServers: number;
  servers: ServerSummary[];
}

export interface ImportPreviewData {
  preview: ImportPreview;
  config: AppConfig;
}

export class ConfigError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ConfigError";
  }
}

interface ParsedConfig {
  config: AppConfig;
  invalidServers: number;
  serverEntries: number;
}

export class ConfigStore {
  constructor(
    private readonly appDir: string,
    private readonly configPath: string,
  ) {}

  static open(appDir: string): ConfigStore {
    return new ConfigStore(appDir, join(appDir, "config.json"));
  }

  get dir(): string {
    return this.appDir;
  }

  get path(): string {
    return this.configPath;
  }

  /**
   * 读取配置。
   *
   * 配置坏了就直接报错，不静默回退到备份——掩盖掉真实原因只会让人在错误的原因上
   * 排查。需要保留旧副本的场景由调用方显式处理。
   */
  async load(): Promise<AppConfig> {
    if (!(await exists(this.configPath))) {
      return emptyConfig();
    }
    const parsed = parseConfig(await readFile(this.configPath, "utf8"));
    return loadableConfig(parsed);
  }

  async save(config: AppConfig): Promise<void> {
    validateAppConfig(config);
    await mkdir(this.appDir, { recursive: true });
    await atomicWrite(this.configPath, JSON.stringify(config, null, 2));
  }

  /**
   * 读-改-写事务，全程串行。
   *
   * 桌面端是单进程，跨进程并发只有 daemon 可能碰到，而 daemon 不写桌面配置
   * 目录，所以进程内队列就够了。
   */
  async update(mutate: (config: AppConfig) => void | Promise<void>): Promise<AppConfig> {
    const task = this.queue.then(async () => {
      const config = await this.load();
      await mutate(config);
      await this.save(config);
      return config;
    });
    // 队列本身不该因为一次失败而中断后续操作。
    this.queue = task.then(
      () => undefined,
      () => undefined,
    );
    return task;
  }

  private queue: Promise<unknown> = Promise.resolve();

  /** 解析内存中的导入内容（粘贴 JSON），不碰文件系统。 */
  async previewImportText(content: string): Promise<ImportPreviewData> {
    return importPreviewFromParsed(parseConfig(content));
  }

  async applyImportAndSave(data: ImportPreviewData, mode: ImportMode): Promise<AppConfig> {
    if (data.config.servers.length === 0) {
      throw new ConfigError("import contains no valid servers");
    }
    return this.queue.then(async () => {
      const current = await this.load();
      const merged = mergeImport(current, data, mode);
      await this.save(merged);
      return merged;
    });
  }

  /**
   * 统一交换格式（schema + activeId + servers）。
   *
   * 刻意不含桌面本地设置——这份格式要与 Android 端和扫码同步共用。
   */
  exportText(config: AppConfig): string {
    validateSchema(config.schema);
    const exchange = {
      schema: config.schema,
      activeId: config.activeId,
      servers: config.servers,
    };
    return JSON.stringify(exchange, null, 2);
  }

  async exportTo(config: AppConfig, path: string): Promise<void> {
    await atomicWrite(path, this.exportText(config));
  }
}

function emptyConfig(): AppConfig {
  return { schema: DEFAULT_SCHEMA, activeId: null, servers: [], settings: defaultSettings() };
}

async function exists(path: string): Promise<boolean> {
  try {
    await access(path, constants.F_OK);
    return true;
  } catch {
    return false;
  }
}

/** 临时文件 + 原子替换；内容仅当前用户可读。 */
export async function atomicWrite(path: string, contents: string): Promise<void> {
  await mkdir(dirname(path), { recursive: true });
  const temp = join(
    dirname(path),
    `.${basename(path)}.tmp-${process.pid}-${randomUUID()}`,
  );
  await writeFile(temp, contents, { mode: 0o600 });
  await rename(temp, path);
}

function validateSchema(schema: number): void {
  if (schema !== 1) {
    throw new ConfigError(`unsupported configuration schema ${schema}; expected schema 1`);
  }
}

function parseConfig(content: string): ParsedConfig {
  let value: unknown;
  try {
    value = JSON.parse(content);
  } catch (error) {
    throw new ConfigError(`configuration JSON is invalid: ${(error as Error).message}`);
  }
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new ConfigError("configuration must be an object or an array");
  }

  const file = value as {
    schema?: number;
    activeId?: string | null;
    servers?: unknown[];
    settings?: Partial<DesktopSettings>;
  };
  const schema = file.schema ?? DEFAULT_SCHEMA;
  validateSchema(schema);

  const entries = Array.isArray(file.servers) ? file.servers : [];
  const { servers, invalidServers } = parseServers(entries);
  validateServers(servers);

  // activeId 指向不存在的服务时丢弃，避免旧配置直接打不开。
  const activeId =
    typeof file.activeId === "string" && servers.some((server) => server.id === file.activeId)
      ? file.activeId
      : null;

  return {
    config: {
      schema,
      activeId,
      servers,
      settings: { ...defaultSettings(), ...(file.settings ?? {}) },
    },
    invalidServers,
    serverEntries: entries.length,
  };
}

function parseServers(values: unknown[]): { servers: ServerConfig[]; invalidServers: number } {
  const servers: ServerConfig[] = [];
  for (const raw of values) {
    const entry = raw as Partial<ServerConfig>;
    if (typeof entry?.name !== "string") continue;
    if (typeof entry.host !== "string") continue;
    if (typeof entry.backend !== "string") continue;
    const candidate: ServerConfig = {
      // 缺 id 的老条目补一个新 id，和 Rust 侧一样保证导入后可编辑。
      id: typeof entry.id === "string" && entry.id.length > 0 ? entry.id : randomUUID(),
      name: entry.name,
      host: entry.host,
      port: Number(entry.port ?? 0),
      token: typeof entry.token === "string" ? entry.token : "",
      backend: entry.backend as Backend,
    };
    if (Number.isInteger(candidate.port) && candidate.port > 0) {
      try {
        validateServer(candidate);
        servers.push(candidate);
      } catch {
        // 单条非法不阻断整体加载，计入 invalidServers。
      }
    }
  }
  return { servers, invalidServers: values.length - servers.length };
}

function validateServers(servers: ServerConfig[]): void {
  const ids = new Set<string>();
  for (const server of servers) {
    try {
      validateServer(server);
    } catch {
      throw new ConfigError("server configuration is invalid");
    }
    if (ids.has(server.id)) {
      throw new ConfigError(`duplicate server id: ${server.id}`);
    }
    ids.add(server.id);
  }
}

function validateAppConfig(config: AppConfig): void {
  validateSchema(config.schema);
  validateServers(config.servers);
  if (config.activeId && !config.servers.some((server) => server.id === config.activeId)) {
    throw new ConfigError(`active server does not exist: ${config.activeId}`);
  }
}

function loadableConfig(parsed: ParsedConfig): AppConfig {
  // 文件里有服务条目但一条都不可用时视作损坏：多半是被手工改坏了。
  if (parsed.serverEntries > 0 && parsed.config.servers.length === 0) {
    throw new ConfigError("import contains no valid servers");
  }
  return parsed.config;
}

function importPreviewFromParsed(parsed: ParsedConfig): ImportPreviewData {
  const servers = parsed.config.servers.map(toSummary);
  return {
    preview: {
      validServers: parsed.config.servers.length,
      invalidServers: parsed.invalidServers,
      servers,
    },
    config: parsed.config,
  };
}

function mergeImport(
  current: AppConfig,
  data: ImportPreviewData,
  mode: ImportMode,
): AppConfig {
  if (mode === "replace") {
    const replacement: AppConfig = { ...data.config, settings: current.settings };
    if (
      !replacement.activeId ||
      !replacement.servers.some((server) => server.id === replacement.activeId)
    ) {
      replacement.activeId = replacement.servers[0]?.id ?? null;
    }
    return replacement;
  }

  const merged: AppConfig = { ...current, servers: [...current.servers] };
  for (const imported of data.config.servers) {
    const index = merged.servers.findIndex((server) => server.id === imported.id);
    if (index >= 0) merged.servers[index] = imported;
    else merged.servers.push(imported);
  }
  return merged;
}