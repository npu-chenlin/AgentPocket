import { describe, expect, it } from "vitest";

import { isActiveStatus } from "./kimi";

/**
 * 会话列表里的 `activity.status` 是否代表"还在跑"。
 *
 * 这段逻辑以前是"排除 idle"，于是 `failed` / `aborted` 这类**已经结束**的
 * 终态也被当成忙碌。它们从没进入过运行期，因此永远收不到
 * `work_changed(busy=false)`，只能一直挂在"活跃会话"里——这就是界面上
 * 越积越多的那些钉子的来源。
 */
describe("会话列表 status 判定", () => {
  it("running 算活跃", () => {
    expect(isActiveStatus("running")).toBe(true);
  });

  it("等待审批/回答算活跃（需要用户介入）", () => {
    expect(isActiveStatus("awaiting_approval")).toBe(true);
    expect(isActiveStatus("awaiting_question")).toBe(true);
  });

  it("idle 不算活跃", () => {
    expect(isActiveStatus("idle")).toBe(false);
  });

  it("failed 与 aborted 是终态，不算活跃", () => {
    // 这两个是真实观测到的取值（`/api/v2/sessions` 的 activity.status）。
    // 详情端点对它们的权威回答是 busy=false。
    expect(isActiveStatus("failed")).toBe(false);
    expect(isActiveStatus("aborted")).toBe(false);
    expect(isActiveStatus("completed")).toBe(false);
  });

  it("未知状态一律按没在跑处理", () => {
    // 白名单的意义：服务端将来新增状态时，默认不误报成忙碌。
    for (const unknown of ["", "paused", "queued", "something_new"]) {
      expect(isActiveStatus(unknown)).toBe(false);
    }
  });
});
