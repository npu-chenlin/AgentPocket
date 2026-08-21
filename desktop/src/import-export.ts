import { save } from "@tauri-apps/plugin-dialog";
import type { ImportPreview } from "./model";

export const CREDENTIAL_WARNING = "文件包含 Agent 服务访问凭据，请勿公开分享";

export function importPreviewText(preview: ImportPreview): string {
  const invalidCount = preview.invalid.length;
  return `${preview.validCount} 条有效，${invalidCount} 条无效`;
}

export function importIssueText(preview: ImportPreview): string[] {
  return preview.invalid.map((issue) => `第 ${issue.index + 1} 条：${issue.reason}`);
}

export async function chooseExportPath(): Promise<string | null> {
  return save({
    title: "导出配置",
    defaultPath: "agentpocket-config.json",
    filters: [{ name: "JSON 配置", extensions: ["json"] }],
  });
}

/** 复制文本到系统剪贴板；优先 Clipboard API，回退到 execCommand。 */
export async function copyTextToClipboard(text: string): Promise<void> {
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(text);
    return;
  }
  const area = document.createElement("textarea");
  area.value = text;
  area.style.position = "fixed";
  area.style.opacity = "0";
  document.body.appendChild(area);
  area.select();
  const ok = document.execCommand("copy");
  area.remove();
  if (!ok) throw new Error("复制失败");
}
