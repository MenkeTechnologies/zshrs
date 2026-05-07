#!/usr/bin/env python3
"""One-shot mover: extract every ShellExecutor::builtin_* whose C
counterpart lives in src/zsh/Src/builtin.c out of src/ported/exec.rs
and append it to src/ported/builtin.rs (single appended impl block).

Mirrors the approach proven in scripts/move_ext_builtins.py:
- Walks exec.rs tracking top-level impl blocks; methods inside
  `impl Trait for X` are SKIPPED (visibility qualifiers illegal).
- Captures preceding contiguous `///`/`//`/`#[..]` lines.
- Bumps captured signatures to `pub(crate) fn`; does NOT touch
  unrelated methods. (Visibility on cross-file callees is bumped
  on-demand after the build error pass.)
"""
from __future__ import annotations
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EXEC = ROOT / "src/ported/exec.rs"
DEST = ROOT / "src/ported/builtin.rs"

# Methods whose C handler lives in Src/builtin.c (mapped via the
# BUILTINS table; one bin_* often backs many builtin names).
TARGETS = {
    # bin_enable
    "builtin_enable", "builtin_disable",
    # bin_set
    "builtin_set",
    # bin_pwd
    "builtin_pwd", "builtin_pwd_with_args",
    # bin_dirs
    "builtin_dirs",
    # bin_cd
    "builtin_cd", "builtin_pushd", "builtin_popd",
    # bin_fc
    "builtin_fc", "builtin_history", "builtin_r",
    # bin_typeset
    "builtin_typeset", "builtin_typeset_named", "builtin_declare",
    "builtin_export", "builtin_float", "builtin_integer",
    "builtin_local", "builtin_readonly",
    # bin_functions
    "builtin_functions", "builtin_autoload",
    # bin_unset
    "builtin_unset",
    # bin_whence
    "builtin_whence", "builtin_type", "builtin_where", "builtin_which",
    # bin_hash
    "builtin_hash", "builtin_rehash",
    # bin_unhash
    "builtin_unhash", "builtin_unalias", "builtin_unfunction",
    # bin_alias
    "builtin_alias",
    # bin_print
    "builtin_print", "builtin_echo", "builtin_printf", "builtin_pushln",
    # bin_shift
    "builtin_shift",
    # bin_getopts
    "builtin_getopts",
    # bin_break
    "builtin_break", "builtin_continue", "builtin_exit", "builtin_return",
    # bin_dot
    "builtin_source", "builtin_source_named",
    # bin_emulate
    "builtin_emulate",
    # bin_eval
    "builtin_eval",
    # bin_read
    "builtin_read", "builtin_getln",
    # bin_test
    "builtin_test",
    # bin_times
    "builtin_times",
    # bin_trap
    "builtin_trap",
    # bin_ttyctl
    "builtin_ttyctl",
    # bin_let
    "builtin_let",
    # bin_umask
    "builtin_umask",
}

SIG_RE = re.compile(r"^(    )(pub(?:\(crate\))? )?(fn (builtin_[a-z0-9_]+))\b")
ATTR_OR_DOC_RE = re.compile(r"^    (?://|///|#\[)")
END_RE = re.compile(r"^    \}\s*$")
IMPL_OPEN_RE = re.compile(r"^impl(?:<[^>]*>)?\s+(.+?)\s*\{")


def find_blocks(lines):
    blocks = []
    in_trait_impl = False
    impl_depth = 0
    for i, line in enumerate(lines):
        if not line.startswith(" ") and not line.startswith("\t"):
            m = IMPL_OPEN_RE.match(line)
            if m:
                in_trait_impl = " for " in m.group(1)
                impl_depth = 1
                continue
            if line.rstrip("\n") == "}" and impl_depth > 0:
                impl_depth = 0
                in_trait_impl = False
                continue
        m = SIG_RE.match(line)
        if not m:
            continue
        name = m.group(4)
        if name not in TARGETS:
            continue
        start = i
        j = i - 1
        while j >= 0 and ATTR_OR_DOC_RE.match(lines[j]):
            start = j
            j -= 1
        end = None
        for k in range(i + 1, len(lines)):
            if END_RE.match(lines[k]):
                end = k
                break
        if end is None:
            raise RuntimeError(f"No closing brace for {name} at line {i+1}")
        blocks.append((start, end, name, in_trait_impl))
    return blocks


def main():
    src = EXEC.read_text()
    lines = src.splitlines(keepends=True)
    blocks = find_blocks(lines)
    found = {b[2] for b in blocks}
    missing = TARGETS - found
    if missing:
        print(f"WARNING: {len(missing)} target methods not found:")
        for n in sorted(missing):
            print(f"  {n}")
    print(f"found {len(blocks)} method blocks (targets={len(TARGETS)})")

    trait_blocks = [b for b in blocks if b[3]]
    if trait_blocks:
        print(f"WARNING: {len(trait_blocks)} target methods inside trait impls -- skipping:")
        for b in trait_blocks:
            print(f"  {b[2]} at line {b[0]+1}")
    blocks = [b for b in blocks if not b[3]]

    extracted_chunks = []
    for start, end, name, _ in sorted(blocks, key=lambda b: b[0]):
        chunk = "".join(lines[start:end + 1])
        new_lines = []
        for ln in chunk.splitlines(keepends=True):
            m = SIG_RE.match(ln)
            if m and (m.group(2) is None or m.group(2).strip() == ""):
                new_lines.append(f"{m.group(1)}pub(crate) {m.group(3)}{ln[m.end():]}")
            else:
                new_lines.append(ln)
        extracted_chunks.append("".join(new_lines))

    for start, end, _, _ in sorted(blocks, key=lambda b: b[0], reverse=True):
        end_strip = end + 1
        if end_strip < len(lines) and lines[end_strip].strip() == "":
            end_strip += 1
        del lines[start:end_strip]

    EXEC.write_text("".join(lines))
    print(f"removed {len(blocks)} methods from exec.rs")

    dest_text = DEST.read_text()
    marker = "// BEGIN moved-from-exec-rs\n"
    if marker in dest_text:
        raise SystemExit("ERROR: builtin.rs already contains a moved-from-exec-rs block")

    additions = []
    additions.append("\n")
    additions.append("// ===========================================================\n")
    additions.append("// Methods moved verbatim from src/ported/exec.rs because their\n")
    additions.append("// C counterpart lives in src/zsh/Src/builtin.c. Rust permits\n")
    additions.append("// multiple inherent impl blocks for the same type within a\n")
    additions.append("// crate, so call sites in exec.rs and elsewhere are unchanged.\n")
    additions.append("// ===========================================================\n")
    additions.append("\n")
    additions.append(marker)
    additions.append("impl crate::ported::exec::ShellExecutor {\n")
    for chunk in extracted_chunks:
        additions.append(chunk)
    additions.append("}\n")
    additions.append("// END moved-from-exec-rs\n")

    DEST.write_text(dest_text + "".join(additions))
    print(f"appended {len(extracted_chunks)} methods to builtin.rs")


if __name__ == "__main__":
    main()
