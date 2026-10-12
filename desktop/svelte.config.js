import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

export default {
  preprocess: vitePreprocess(),
  compilerOptions: {
    // TypeScript 放在 <script lang="ts"> 里，交给 vitePreprocess 转译。
    runes: true,
  },
};