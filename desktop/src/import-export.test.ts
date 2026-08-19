import { describe, expect, it } from "vitest";
import { CREDENTIAL_WARNING, importIssueText, importPreviewText } from "./import-export";

describe("import/export presentation", () => {
  it("shows valid and invalid counts without imported values", () => {
    const preview = {
      importId: "opaque-id",
      validCount: 2,
      invalid: [{ index: 2, reason: "invalid host" }],
    };
    expect(importPreviewText(preview)).toBe("2 条有效，1 条无效");
    expect(importIssueText(preview)).toEqual(["第 3 条：invalid host"]);
    expect(JSON.stringify({ text: importPreviewText(preview) })).not.toContain("token");
  });

  it("uses the exact credential warning", () => {
    expect(CREDENTIAL_WARNING).toBe("文件包含服务器访问凭据，请勿公开分享");
  });
});
