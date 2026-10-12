import { svelte } from "@sveltejs/vite-plugin-svelte";
import { resolve } from "node:path";
import { defineConfig, externalizeDepsPlugin } from "electron-vite";

// electron-vite 的三段构建配置：主进程、preload、渲染进程。
//
// 渲染进程沿用仓库里已有的 Svelte 代码，只替换 IPC 层；主进程和 preload
// 放在 electron/ 下，与前端源码分开。
//
// 注意：测试配置不在这里，而在 vite.config.ts。vitest 不认识 main/preload/
// renderer 这层嵌套结构，放在这里等于没放——实际生效的是 vite.config.ts。
export default defineConfig({
  main: {
    plugins: [externalizeDepsPlugin()],
    build: {
      rollupOptions: {
        input: resolve(__dirname, "electron/main/index.ts"),
      },
    },
  },
  preload: {
    plugins: [externalizeDepsPlugin()],
    build: {
      rollupOptions: {
        input: resolve(__dirname, "electron/preload/index.ts"),
      },
    },
  },
  renderer: {
    root: resolve(__dirname, "src"),
    plugins: [svelte()],
    build: {
      rollupOptions: {
        input: resolve(__dirname, "src/index.html"),
      },
    },
  },
});