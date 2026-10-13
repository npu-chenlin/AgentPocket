import { defineConfig } from "@playwright/test";
export default defineConfig({ testDir: "./e2e", workers: 1, timeout: 90_000,
  expect: { timeout: 10_000 }, reporter: "list", use: { trace: "retain-on-failure" } });
