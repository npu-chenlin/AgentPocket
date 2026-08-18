import { open, save } from "@tauri-apps/plugin-dialog";
import type { ExportFormat, ImportPreview } from "./model";

export const CREDENTIAL_WARNING = "文件包含服务器访问凭据，请勿公开分享";

export function importPreviewText(preview: ImportPreview): string {
  const source = preview.sourceKind === "android" ? "Android 服务器列表" : "完整桌面配置";
  const invalidCount = preview.invalid.length;
  return `${source}：${preview.validCount} 条有效，${invalidCount} 条无效`;
}

export function importIssueText(preview: ImportPreview): string[] {
  return preview.invalid.map((issue) => `第 ${issue.index + 1} 条：${issue.reason}`);
}

export async function chooseImportPath(): Promise<string | null> {
  const selected = await open({
    title: "导入服务器配置",
    multiple: false,
    directory: false,
    filters: [{ name: "JSON 配置", extensions: ["json"] }],
  });
  return typeof selected === "string" ? selected : null;
}

export async function chooseExportPath(format: ExportFormat): Promise<string | null> {
  return save({
    title: format === "android" ? "导出 Android 兼容配置" : "导出完整配置",
    defaultPath:
      format === "android" ? "agentpocket-servers.json" : "agentpocket-config.json",
    filters: [{ name: "JSON 配置", extensions: ["json"] }],
  });
}
