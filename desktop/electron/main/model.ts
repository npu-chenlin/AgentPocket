/**
 * 领域模型：从 Rust core/src/model.rs 迁移而来。
 *
 * 保持 camelCase 字段名不变——配置文件和 Android 端都依赖这个格式。
 */

export type Backend = "kimi" | "dsh" | "opencode";

export interface ServerConfig {
  id: string;
  name: string;
  host: string;
  port: number;
  token: string;
  backend: Backend;
}

export interface MeshPeerEntry {
  name: string;
  host: string;
}

export interface DesktopSettings {
  startHidden: boolean;
  autostart: boolean;
  notifications: boolean;
  meshPeers: MeshPeerEntry[];
}

export interface AppConfig {
  schema: number;
  activeId: string | null;
  servers: ServerConfig[];
  settings: DesktopSettings;
}

export const DEFAULT_SCHEMA = 1;

export function defaultSettings(): DesktopSettings {
  return {
    startHidden: true,
    // 托盘监控类应用的意义就是常驻：装完即后台常驻，用户可在设置里关掉。
    autostart: true,
    notifications: true,
    meshPeers: [],
  };
}

export function defaultConfig(): AppConfig {
  return {
    schema: DEFAULT_SCHEMA,
    activeId: null,
    servers: [],
    settings: defaultSettings(),
  };
}

export class ValidationError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ValidationError";
  }
}

/** host 字段只接受裸主机名/IP，不含 scheme、路径、空白或端口。 */
export function validateServer(server: ServerConfig): void {
  if (!Number.isInteger(server.port) || server.port <= 0 || server.port > 65535) {
    throw new ValidationError("port must be greater than zero");
  }
  const host = server.host ?? "";
  if (
    host.length === 0 ||
    /\s/.test(host) ||
    host.includes(":") ||
    host.includes("/")
  ) {
    throw new ValidationError("host must not contain a scheme, path, whitespace, or port");
  }
}

export function serverBaseUrl(server: ServerConfig): URL {
  validateServer(server);
  return new URL(`http://${server.host}:${server.port}`);
}

/**
 * opencode v2 的登录凭据：token 字段承载 `user:pass`（HTTP Basic）。
 *
 * opencode 的用户名固定为 `opencode`（服务端忽略 OPENCODE_SERVER_USERNAME），
 * 只写密码时补默认用户名。token 字段对 kimi 是访问令牌、对 dsh 无效。
 */
export function opencodeCredentials(
  server: ServerConfig,
): { user: string; pass: string } | null {
  if (server.backend !== "opencode") return null;
  const raw = (server.token ?? "").trim();
  if (raw.length === 0) return null;
  const idx = raw.indexOf(":");
  if (idx < 0) return { user: "opencode", pass: raw };
  return { user: raw.slice(0, idx).trim(), pass: raw.slice(idx + 1) };
}

/** opencode 请求用的 Authorization 头值（HTTP Basic）。 */
export function opencodeBasicHeader(server: ServerConfig): string | null {
  const credentials = opencodeCredentials(server);
  if (!credentials) return null;
  const raw = `${credentials.user}:${credentials.pass}`;
  return `Basic ${Buffer.from(raw, "utf8").toString("base64")}`;
}

/** 不含 token 的列表视图，防止凭据经由列表接口泄漏。 */
export interface ServerSummary {
  id: string;
  name: string;
  host: string;
  port: number;
  backend: Backend;
}

export function toSummary(server: ServerConfig): ServerSummary {
  return {
    id: server.id,
    name: server.name,
    host: server.host,
    port: server.port,
    backend: server.backend,
  };
}

/** 编辑表单专用的 DTO，含 token。仅在用户明确编辑该服务时返回。 */
export interface ServerForEdit {
  id: string;
  name: string;
  host: string;
  port: number;
  backend: Backend;
  token: string;
}

export function toForEdit(server: ServerConfig): ServerForEdit {
  return { ...server };
}