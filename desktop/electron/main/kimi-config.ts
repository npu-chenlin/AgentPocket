/**
 * ~/.kimi-code/config.toml 的读写。
 * 从 core/src/kimi_config.rs 迁移。
 *
 * 写之前必须校验 TOML 语法：config.toml 是 Kimi 的命脉，写坏了整个 Kimi 起不来，
 * 而错误只在 Kimi 下次启动时才暴露，那时已经很难联想到是这里写的。
 */

import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  renameSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { randomUUID } from "node:crypto";
import { dirname, join } from "node:path";
import { homedir } from "node:os";

/** <home>/.kimi-code/config.toml。 */
export function configPath(home: string = homedir()): string {
  return join(home, ".kimi-code", "config.toml");
}

/**
 * 校验 TOML 语法。
 *
 * 不引 TOML 解析库（避免为一个校验引入依赖），而是抓最常见的几类结构错误：
 * 未闭合的引号/方括号、未转义的裸控制字符、重复的顶层键。
 */
export function validateToml(text: string): void {
  const lines = text.split(/\r?\n/);

  // 引号与括号必须闭合——这是手写配置最常见的错。
  let inBasicString = false;
  let inLiteralString = false;
  let bracketDepth = 0;
  /**
   * 当前所处的段名，null 表示根表。
   *
   * 不能用 bracketDepth 判断"是不是顶层"：`[provider]` 的两个括号在同一行
   * 闭合，写完这行 depth 已经回到 0，那样段内的键会被当成顶层键，
   * 于是 `[x]\na=1\n[y]\na=2` 会被误报成"a 重复"。
   */
  let currentTable: string | null = null;
  /** 段名 → 该段已出现的键，用于段内重复检测。 */
  const seenByTable = new Map<string, Set<string>>();
  seenByTable.set("", new Set());

  const CONTROL_CHARS = new RegExp("[\\u0000-\\u0008\\u000B\\u000C\\u000E-\\u001F]");

  for (const [index, rawLine] of lines.entries()) {
    const lineNumber = index + 1;

    // TOML 不允许裸控制字符（制表符与换行除外），手写配置很容易踩到。
    if (CONTROL_CHARS.test(rawLine)) {
      throw new Error(`config.toml 第 ${lineNumber} 行含非法控制字符`);
    }

    for (let i = 0; i < rawLine.length; i += 1) {
      const ch = rawLine[i];
      if (inBasicString) {
        if (ch === "\\") i += 1;
        else if (ch === '"') inBasicString = false;
        continue;
      }
      if (inLiteralString) {
        if (ch === "'") inLiteralString = false;
        continue;
      }
      if (ch === '"') inBasicString = true;
      else if (ch === "'") inLiteralString = true;
      else if (ch === "#") break; // 行内注释到此为止
      else if (ch === "[") bracketDepth += 1;
      else if (ch === "]") bracketDepth -= 1;
      if (bracketDepth < 0) {
        throw new Error(`config.toml 第 ${lineNumber} 行有多余的 ]`);
      }
    }

    // 段头：切换当前段，并给该段建一张键表。
    const header = rawLine.match(/^\s*\[\s*([^\]]+?)\s*\]\s*(?:#.*)?$/);
    if (header) {
      currentTable = header[1];
      if (!seenByTable.has(currentTable)) seenByTable.set(currentTable, new Set());
      continue;
    }

    // 重复键会让解析器报错，但这里给的是更清楚的提示，并指出是哪个段。
    const keyMatch = rawLine.match(/^\s*([A-Za-z0-9_.-]+)\s*=(?!=)/);
    if (keyMatch) {
      const keys = seenByTable.get(currentTable ?? "") ?? new Set<string>();
      const key = keyMatch[1];
      if (keys.has(key)) {
        const where = currentTable ? `段 [${currentTable}] 的` : "顶层的";
        throw new Error(`config.toml 第 ${lineNumber} 行：${where}键 ${key} 重复`);
      }
      keys.add(key);
      seenByTable.set(currentTable ?? "", keys);
    }
  }

  if (inBasicString || inLiteralString) {
    throw new Error("config.toml 有未闭合的字符串");
  }
  if (bracketDepth !== 0) {
    throw new Error("config.toml 有未闭合的 [");
  }
}

export interface WriteResult {
  backedUp: boolean;
}

/** 校验后原子写入；已存在则先备份成 config.toml.bak。 */
export function write(content: string, home: string = homedir()): WriteResult {
  validateToml(content);
  const path = configPath(home);
  mkdirSync(dirname(path), { recursive: true });

  const backedUp = existsSync(path);
  if (backedUp) copyFileSync(path, `${path}.bak`);

  // 临时文件放同一目录，写完再原子替换，避免崩溃留下半个文件。
  const temp = join(dirname(path), `.config.toml.tmp-${process.pid}-${randomUUID()}`);
  try {
    writeFileSync(temp, content, "utf8");
    renameSync(temp, path);
  } catch (error) {
    // 失败留下的临时文件要清掉，否则目录里会堆垃圾。
    try {
      if (existsSync(temp)) unlinkSync(temp);
    } catch {
      /* 清理失败不掩盖原始错误 */
    }
    throw error;
  }
  return { backedUp };
}

export function read(home: string = homedir()): string {
  const path = configPath(home);
  if (!existsSync(path)) {
    throw new Error(`本地没有 ${path}`);
  }
  return readFileSync(path, "utf8");
}