import "@testing-library/jest-dom/vitest";

import { cleanup } from "@testing-library/svelte";
import { beforeEach, afterEach } from "vitest";

/*
 * 每个用例后卸载组件。
 *
 * testing-library 的自动清理依赖它自己注册 afterEach，但本项目这几个用例会
 * 反复挂载 App，残留的实例会让后面的查询命中上一次渲染的内容——症状是
 * "找不到某个本该存在的文字"，排查起来很绕。显式声明更可靠。
 */
afterEach(() => {
  cleanup();
});

/*
 * 每个用例前清空 localStorage。
 *
 * jsdom 在整个文件里共用一份 localStorage，而应用会把界面偏好写进去
 * （比如分屏布局、悬浮球位置）。不清理的话，前一个用例留下的布局会让后一个
 * 用例一挂载就带着窗格——表现为"服务名凭空出现"，和真实故障很难区分。
 */
beforeEach(() => {
  localStorage.clear();
});

/*
 * jsdom 不实现 canvas，`getContext("2d")` 会打一行 "Not implemented" 警告。
 *
 * 悬浮球里的表情画布只是装饰，测试断言的是悬浮球面板与窗格接线，不需要真的画。
 * 返回 null 让组件走"拿不到 context 就跳过渲染"的分支，输出保持干净。
 */
HTMLCanvasElement.prototype.getContext = (() => null) as never;
globalThis.ResizeObserver = class {
  observe() {} unobserve() {} disconnect() {}
} as unknown as typeof ResizeObserver;
window.matchMedia = (() => ({ matches: false, addEventListener() {}, removeEventListener() {},
  addListener() {}, removeListener() {} })) as unknown as typeof window.matchMedia;
HTMLElement.prototype.scrollTo = () => {};
HTMLElement.prototype.setPointerCapture = () => {};
HTMLElement.prototype.releasePointerCapture = () => {};