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
        // 两个 preload：主窗口的，和悬浮球窗口的（它只暴露三个接口）。
        input: {
          index: resolve(__dirname, "electron/preload/index.ts"),
          ball: resolve(__dirname, "electron/preload/ball.ts"),
        },
      },
    },
  },
  renderer: {
    // root 指向 src/ 后，插件会在 src/ 下找 svelte.config.js 而找不到，
    // 于是 runes、preprocess 这些设置全走默认值。显式指回仓库根目录的那份。
    root: resolve(__dirname, "src"),
    plugins: [svelte({ configFile: resolve(__dirname, "svelte.config.js") })],
    build: {
      rollupOptions: {
        // 两个页面：主窗口和悬浮球窗口。
        input: {
          index: resolve(__dirname, "src/index.html"),
          ball: resolve(__dirname, "src/ball.html"),
        },
      },
    },
  },
});