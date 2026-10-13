import { app } from "electron";
import { isAbsolute, join } from "node:path";

const directory = process.argv.find(arg => arg.startsWith("--agentpocket-isolated="))?.split("=").slice(1).join("=");
export const isolatedMode = Boolean(directory);
if (directory) {
  if (!isAbsolute(directory)) throw new Error("隔离目录必须是绝对路径");
  app.setPath("userData", join(directory, "userData"));
  app.setPath("sessionData", join(directory, "sessionData"));
}
export const isolatedConfigDir = directory ? join(directory, "config") : null;
