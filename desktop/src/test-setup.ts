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
 * （比如侧边栏折叠状态）。不清理的话，前一个用例折叠了侧边栏，后一个用例
 * 一挂载就是折叠态——表现为"服务名凭空不见了"，和真实故障很难区分。
 */
beforeEach(() => {
  localStorage.clear();
});