#!/usr/bin/env python3
"""从 ExpressionData.java 生成 desktop/src/lib/face/expression-data.ts。

数据的真正来源是 GrokBot gist 的 HTML（见 tools/gen_expression_data.py），但那份
HTML 不在本仓库里，所以这里反过来以已生成的 Java 文件为输入，导出等价的 TS 常量。

用法: python3 tools/gen_expression_data_ts.py [输入java路径] [输出ts路径]
默认输入: app/src/main/java/com/local/kimiapp/face/ExpressionData.java
默认输出: desktop/src/lib/face/expression-data.ts
"""
import ast
import os
import re
import sys

USED_STATES = ["sleeping", "waking", "idle", "working", "loading",
               "celebrate", "thinking", "sad", "drowsy", "dragging"]


def extract_block(src, marker):
    """提取 marker 起始的配对大括号段（marker 的末字符即开括号，故从它开始计数）。"""
    start = src.index(marker) + len(marker) - 1
    i, depth = start, 0
    while i < len(src):
        if src[i] == '{':
            depth += 1
        elif src[i] == '}':
            depth -= 1
            if depth == 0:
                break
        i += 1
    return src[start:i + 1]


def parse_expressions(src):
    """Java 的 float[][][][] 字面量转 Python 嵌套 list：大括号换方括号、去掉 f 后缀。"""
    literal = extract_block(src, "EXPRESSIONS = {")
    py = literal.replace("{", "[").replace("}", "]")
    return ast.literal_eval(re.sub(r"f\b", "", py))


def parse_num_map(src, method):
    """解析 `private static Map<String, T[]> M() {...}` 里的 m.put("k", new T[]{...})。

    键名是裸标识符、值是 `new int[]{}`，都不是合法 Python，故用正则而非 literal_eval。
    """
    body = extract_block(src, "%s() {" % method)
    out = {}
    for key, vals in re.findall(r'm\.put\("(\w+)", new \w+\[\]\{([^}]*)\}\)', body):
        out[key] = [int(v.strip().rstrip("L")) for v in vals.split(",") if v.strip()]
    return out


def fmt_num(v):
    s = "%.2f" % v
    return "0.00" if s.startswith("-0.00") else s


def ts_expression(expr):
    lines = ["  ["]
    for ring in expr:
        pts = ", ".join("[%s, %s]" % (fmt_num(p[0]), fmt_num(p[1])) for p in ring)
        lines.append("    [" + pts + "],")
    lines.append("  ],")
    return lines


def ts_map(name, type_expr, doc, vals):
    lines = ["/** %s */" % doc,
             "export const %s: %s = {" % (name, type_expr)]
    for key, val in sorted(vals.items()):
        lines.append("  %s: [%s]," % (key, ", ".join(str(v) for v in val)))
    lines.append("};")
    return lines


def main():
    in_path = sys.argv[1] if len(sys.argv) > 1 else \
        "app/src/main/java/com/local/kimiapp/face/ExpressionData.java"
    out_path = sys.argv[2] if len(sys.argv) > 2 else \
        "desktop/src/lib/face/expression-data.ts"

    src = open(in_path, encoding="utf-8").read()
    expressions = parse_expressions(src)
    pools = parse_num_map(src, "pools")
    expr_cadence = parse_num_map(src, "exprCadence")
    blink = parse_num_map(src, "blink")

    assert len(expressions) == 25, "预期 25 个表情，实际 %d" % len(expressions)
    for e in expressions:
        assert len(e) == 2 and all(len(r) == 48 for r in e), "表情结构异常"
    missing = [s for s in USED_STATES if s not in pools]
    assert not missing, "POOLS 缺少状态: %s" % missing
    missing = [s for s in USED_STATES if s not in expr_cadence]
    assert not missing, "EXPR_CADENCE 缺少状态: %s" % missing

    used_pools = {s: pools[s] for s in USED_STATES}
    used_expr = {s: expr_cadence[s] for s in USED_STATES}
    used_blink = {s: blink[s] for s in USED_STATES if s in blink}

    ts = []
    ts.append("// 本文件由 tools/gen_expression_data_ts.py 从")
    ts.append("// app/src/main/java/com/local/kimiapp/face/ExpressionData.java 生成，勿手改。")
    ts.append("/**")
    ts.append(" * GrokBot 表情引擎数据。EXPRESSIONS[expression][eye][point] = [x, y]，")
    ts.append(" * 坐标系与 gist viewBox 一致（229 单位空间）。")
    ts.append(" */")
    ts.append("")
    ts.append("/** 单个表情：2 只眼睛，每只 48 个轮廓点，每点为 [x, y]。 */")
    ts.append("export type Expression = number[][][];")
    ts.append("")
    ts.append("export const EXPRESSIONS: Expression[] = [")
    for e in expressions:
        ts.extend(ts_expression(e))
    ts.append("];")
    ts.append("")
    ts.extend(ts_map("POOLS", "Record<string, readonly number[]>",
                     "gist 状态名 → 候选表情下标池。", used_pools))
    ts.append("")
    ts.extend(ts_map("EXPR_CADENCE", "Record<string, readonly [number, number]>",
                     "gist 状态名 → 自动换表情的间隔范围 [min, max]（毫秒）。", used_expr))
    ts.append("")
    ts.extend(ts_map("BLINK", "Partial<Record<string, readonly [number, number]>>",
                     "gist 状态名 → 自动眨眼间隔范围 [min, max]（毫秒）；未列出的状态不自动眨眼。",
                     used_blink))
    ts.append("")

    os.makedirs(os.path.dirname(out_path), exist_ok=True)
    with open(out_path, "w", encoding="utf-8") as f:
        f.write("\n".join(ts))
    print("wrote %s (%d expressions, %d states)" % (out_path, len(expressions), len(used_pools)))


if __name__ == "__main__":
    main()
