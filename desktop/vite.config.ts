import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";

export default defineConfig(({ mode }) => ({
  clearScreen: false,
  plugins: [svelte()],
  server: {
    port: 1420,
    strictPort: true,
  },
  test: {
    include: ["src/**/*.test.ts", "electron/**/*.test.ts"],
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test-setup.ts"],
  },
  // 组件测试必须用浏览器版 Svelte：默认解析条件会命中 index-server.js，
  // 那里 mount() 不可用（报 lifecycle_function_unavailable）。
  // vitest 的 mode 是 "test"，build 是 "production"，互不影响。
  resolve: mode === "test" ? { conditions: ["browser"] } : {},
}));