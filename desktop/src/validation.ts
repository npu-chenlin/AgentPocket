import type { ServerDraft } from "./model";

export type ServerField = "name" | "host" | "port" | "backend";
export type ServerValidationErrors = Partial<Record<ServerField, string>>;

export function validateServerDraft(
  draft: Pick<ServerDraft, "name" | "host" | "port" | "token" | "backend">,
): ServerValidationErrors {
  const errors: ServerValidationErrors = {};
  const name = draft.name.trim();
  const host = draft.host.trim();

  if (!name) {
    errors.name = "请输入服务器名称";
  }

  if (
    !host ||
    host !== draft.host ||
    /\s/.test(host) ||
    host.includes(":") ||
    host.includes("/") ||
    host.includes("\\")
  ) {
    errors.host = "主机地址不能包含协议、端口、路径或空格";
  }

  if (!Number.isInteger(draft.port) || draft.port < 1 || draft.port > 65_535) {
    errors.port = "端口必须是 1–65535 之间的整数";
  }

  if (draft.backend !== "kimi" && draft.backend !== "dsh") {
    errors.backend = "请选择有效的后端类型";
  }

  return errors;
}

export function hasValidationErrors(errors: ServerValidationErrors): boolean {
  return Object.keys(errors).length > 0;
}
