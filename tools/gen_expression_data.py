#!/usr/bin/env python3
"""从 gist HTML 生成 ExpressionData.java（GrokBot 表情引擎数据）。

用法: python3 tools/gen_expression_data.py [gist_html] [输出java路径]
默认输入: /home/user/grokbot-demo/index.html
默认输出: app/src/main/java/com/local/kimiapp/face/ExpressionData.java
"""
import ast
import re
import sys
import os

USED_STATES = ["sleeping", "waking", "idle", "working", "loading",
               "celebrate", "thinking", "sad", "drowsy", "dragging"]


def extract_literal(src, name):
    """提取 `const NAME = <literal>` 并用 ast.literal_eval 解析（用于数组字面量）。"""
    marker = "const %s = " % name
    start = src.index(marker) + len(marker)
    i, depth = start, 0
    while i < len(src):
        if src[i] == '[':
            depth += 1
        elif src[i] == ']':
            depth -= 1
            if depth == 0:
                break
        i += 1
    return ast.literal_eval(src[start:i + 1])


def extract_flat_map(src, name):
    """提取 `const NAME = { key: [a,b] | null, ... }` 为 {key: (a,b) | None}。

    JS 对象字面量（裸 key + null）不是合法 Python，故用正则而非 literal_eval。
    """
    marker = "const %s = {" % name
    start = src.index(marker)
    i, depth = start, 0
    while i < len(src):
        c = src[i]
        if c == '{':
            depth += 1
        elif c == '}':
            depth -= 1
            if depth == 0:
                break
        i += 1
    body = src[start:i + 1]
    out = {}
    for key, val in re.findall(r"(\w+)\s*:\s*(\[[\d\s,]*\]|null)", body):
        if val == "null":
            out[key] = None
        else:
            nums = [int(x) for x in val.strip("[]").split(",") if x.strip()]
            out[key] = tuple(nums)
    return out


def fmt_float(v):
    s = "%.2f" % v
    if s.startswith("-0.00"):
        return "0.00f"
    return s + "f"


def java_float_array(rings):
    lines = []
    for ring in rings:
        pts = ", ".join("{%s, %s}" % (fmt_float(p[0]), fmt_float(p[1])) for p in ring)
        lines.append("                {" + pts + "}")
    return ",\n".join(lines)


def java_int_array(vals):
    return "new int[]{" + ", ".join(str(v) for v in vals) + "}"


def java_long_array(vals):
    return "new long[]{" + ", ".join(str(v) + "L" for v in vals) + "}"


def java_map_builder(name, key_type, val_expr):
    """生成 private static Map<...> name() { Map m = new HashMap<>(); m.put(...); return m; }"""
    lines = ["    private static Map<String, %s> %s() {" % (key_type, name),
             "        Map<String, %s> m = new HashMap<>();" % key_type]
    for key, val in sorted(val_expr.items()):
        lines.append('        m.put("%s", %s);' % (key, val))
    lines.append("        return m;")
    lines.append("    }")
    return "\n".join(lines)


def main():
    gist_html = sys.argv[1] if len(sys.argv) > 1 else "/home/user/grokbot-demo/index.html"
    out_path = sys.argv[2] if len(sys.argv) > 2 else \
        "app/src/main/java/com/local/kimiapp/face/ExpressionData.java"

    src = open(gist_html, encoding="utf-8").read()
    expressions = extract_literal(src, "EXPRESSIONS")
    pools = extract_flat_map(src, "POOLS")
    expr_cadence = extract_flat_map(src, "EXPR_CADENCE")
    blink = extract_flat_map(src, "BLINK")

    assert len(expressions) == 25, "预期 25 个表情，实际 %d" % len(expressions)
    for e in expressions:
        assert len(e) == 2 and all(len(r) == 48 for r in e), "表情结构异常"
    missing = [s for s in USED_STATES if s not in pools]
    assert not missing, "POOLS 缺少状态: %s" % missing

    java = []
    java.append("package com.local.kimiapp.face;")
    java.append("")
    java.append("import java.util.HashMap;")
    java.append("import java.util.Map;")
    java.append("")
    java.append("/**")
    java.append(" * GrokBot 表情引擎数据（由 tools/gen_expression_data.py 从 gist 生成，勿手改）。")
    java.append(" * EXPRESSIONS[expression][eye][point] = {x, y}，坐标系与 gist viewBox 一致。")
    java.append(" */")
    java.append("public final class ExpressionData {")
    java.append("    public static final float[][][][] EXPRESSIONS = {")
    for i, e in enumerate(expressions):
        comma = "," if i < len(expressions) - 1 else ""
        java.append("        {")
        java.append(java_float_array(e))
        java.append("        }%s" % comma)
    java.append("    };")
    java.append("")

    used_pools = {s: pools[s] for s in USED_STATES}
    used_expr = {s: expr_cadence[s] for s in USED_STATES}
    used_blink = {s: blink[s] for s in USED_STATES if blink[s] is not None}
    java.append(java_map_builder("pools", "int[]",
                                 {k: java_int_array(v) for k, v in used_pools.items()}))
    java.append("")
    java.append(java_map_builder("exprCadence", "long[]",
                                 {k: java_long_array(v) for k, v in used_expr.items()}))
    java.append("")
    java.append(java_map_builder("blink", "long[]",
                                 {k: java_long_array(v) for k, v in used_blink.items()}))
    java.append("")
    java.append("    public static final Map<String, int[]> POOLS = pools();")
    java.append("    public static final Map<String, long[]> EXPR_CADENCE = exprCadence();")
    java.append("    public static final Map<String, long[]> BLINK = blink();")
    java.append("}")
    java.append("")

    os.makedirs(os.path.dirname(out_path), exist_ok=True)
    with open(out_path, "w", encoding="utf-8") as f:
        f.write("\n".join(java))
    print("wrote %s (%d expressions)" % (out_path, len(expressions)))


if __name__ == "__main__":
    main()
