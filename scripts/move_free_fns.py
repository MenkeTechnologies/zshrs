#!/usr/bin/env python3
"""Bulk-move free functions from src/ported/exec.rs to canonical files."""
from __future__ import annotations
import re
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EXEC = ROOT / "src/ported/exec.rs"

TARGETS = {
    # params.c
    "parse_subscript_flags":          "src/ported/params.rs",
    "array_subscript_flag":           "src/ported/params.rs",
    "assoc_subscript_flag":           "src/ported/params.rs",
    "parse_subscript_index":          "src/ported/params.rs",
    "slice_indexed_array":            "src/ported/params.rs",
    # pattern.c
    "parse_pattern_flags":            "src/ported/pattern.rs",
    "parse_pattern_flags_full":       "src/ported/pattern.rs",
    "approximate_match":              "src/ported/pattern.rs",
    "parse_numeric_range":            "src/ported/pattern.rs",
    "extendedglob_match":             "src/ported/pattern.rs",
    "ksh_extglob_body_to_regex":      "src/ported/pattern.rs",
    # subst.c
    "apply_subst_modifier":           "src/ported/subst.rs",
    "strip_match_op":                 "src/ported/subst.rs",
    "slice_scalar":                   "src/ported/subst.rs",
    # glob.c
    "expand_glob_alternation":        "src/ported/glob.rs",
    "find_top_level_tilde":           "src/ported/glob.rs",
    # math.c
    "parse_subscript_arith_compound": "src/ported/math.rs",
    "parse_subscript_arith_pre_inc":  "src/ported/math.rs",
    "parse_subscript_arith_assign":   "src/ported/math.rs",
    # text.c
    "format_function_body_zsh":       "src/ported/text.rs",
    # extension/util
    "pretty_io_err":                  "src/ported/utils.rs",
    "base64_decode":                  "src/ported/utils.rs",
    # builtin.c
    "format_alias_kv":                "src/ported/builtin.rs",
}

# Free fn at module level (indent 0).
SIG_RE = re.compile(
    r"^(pub(?:\(crate\))? )?(fn ([a-zA-Z_][a-zA-Z0-9_]*))\b"
)
ATTR_OR_DOC_RE = re.compile(r"^(?://|///|#\[)")
END_RE = re.compile(r"^\}\s*$")
ONE_LINER_RE = re.compile(r"\{\s*\}\s*$")
MARKER = "// BEGIN moved-from-exec-rs (free fns)\n"

def find_blocks(lines):
    blocks = []
    impl_depth = 0
    for i, line in enumerate(lines):
        if not line.startswith(" ") and not line.startswith("\t"):
            if re.match(r"^impl\b", line):
                impl_depth = 1
                continue
            if line.rstrip("\n") == "}" and impl_depth > 0:
                impl_depth = 0
                continue
            if impl_depth > 0:
                continue
            m = SIG_RE.match(line)
            if not m:
                continue
            name = m.group(3)
            if name not in TARGETS:
                continue
            start = i
            j = i - 1
            while j >= 0 and ATTR_OR_DOC_RE.match(lines[j]):
                start = j
                j -= 1
            if ONE_LINER_RE.search(line):
                blocks.append((start, i, name))
                continue
            end = None
            for k in range(i + 1, len(lines)):
                if END_RE.match(lines[k]):
                    end = k
                    break
            if end is None:
                raise RuntimeError(f"no closing for {name}")
            blocks.append((start, end, name))
    return blocks

def main():
    src = EXEC.read_text()
    lines = src.splitlines(keepends=True)
    blocks = find_blocks(lines)
    found = {b[2] for b in blocks}
    missing = set(TARGETS) - found
    if missing:
        print(f"WARNING: missing: {sorted(missing)}")
    print(f"found {len(blocks)} blocks")

    extracted = defaultdict(list)
    for start, end, name in sorted(blocks, key=lambda b: b[0]):
        chunk = "".join(lines[start:end + 1])
        # Bump fn to pub(crate) if no qualifier.
        new_lines = []
        for ln in chunk.splitlines(keepends=True):
            m = SIG_RE.match(ln)
            if m and (m.group(1) is None or m.group(1).strip() == ""):
                new_lines.append(f"pub(crate) {m.group(2)}{ln[m.end():]}")
            else:
                new_lines.append(ln)
        extracted[TARGETS[name]].append("".join(new_lines))

    for start, end, _ in sorted(blocks, key=lambda b: b[0], reverse=True):
        end_strip = end + 1
        if end_strip < len(lines) and lines[end_strip].strip() == "":
            end_strip += 1
        del lines[start:end_strip]
    EXEC.write_text("".join(lines))
    print(f"removed {len(blocks)} fns from exec.rs")

    for dest_rel, chunks in sorted(extracted.items()):
        dest = ROOT / dest_rel
        text = dest.read_text()
        out = ["\n", "// ===========================================================\n",
               "// Free fns moved verbatim from src/ported/exec.rs.\n",
               "// ===========================================================\n",
               MARKER]
        out.extend(chunks)
        out.append("// END moved-from-exec-rs (free fns)\n")
        dest.write_text(text + "".join(out))
        print(f"appended {len(chunks)} fns -> {dest_rel}")

if __name__ == "__main__":
    main()
