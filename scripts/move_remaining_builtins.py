#!/usr/bin/env python3
"""Bulk-move the 50 remaining ShellExecutor::builtin_* methods (full
bodies, no canonical port elsewhere) out of src/ported/exec.rs into
the file that mirrors their C source location.

Mirrors the approach proven in scripts/move_builtins_to_builtin_rs.py:
- Walks exec.rs tracking top-level impl blocks; methods inside
  `impl Trait for X` are SKIPPED (visibility qualifiers illegal).
- Captures preceding contiguous `///`/`//`/`#[..]` lines.
- Bumps captured signatures to `pub(crate) fn`.
- Appends each per-target group inside a single
  `impl crate::ported::exec::ShellExecutor { ... }` block at the
  bottom of the destination file, guarded by a unique marker.
"""
from __future__ import annotations
import re
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EXEC = ROOT / "src/ported/exec.rs"

# name -> destination file (relative to repo root)
TARGETS: dict[str, str] = {
    # Src/jobs.c -> src/ported/jobs.rs
    "builtin_bg":      "src/ported/jobs.rs",
    "builtin_fg":      "src/ported/jobs.rs",
    "builtin_jobs":    "src/ported/jobs.rs",
    "builtin_wait":    "src/ported/jobs.rs",
    "builtin_disown":  "src/ported/jobs.rs",
    "builtin_kill":    "src/ported/jobs.rs",
    "builtin_suspend": "src/ported/jobs.rs",

    # Src/builtin.c (dispatcher / parse-time builtins) -> src/ported/builtin.rs
    "builtin_builtin":  "src/ported/builtin.rs",
    "builtin_command":  "src/ported/builtin.rs",
    "builtin_exec":     "src/ported/builtin.rs",
    "builtin_noglob":   "src/ported/builtin.rs",
    "builtin_zcompile": "src/ported/builtin.rs",

    # Src/options.c -> src/ported/options.rs
    "builtin_setopt":   "src/ported/options.rs",
    "builtin_unsetopt": "src/ported/options.rs",

    # Src/module.c -> src/ported/module.rs
    "builtin_zmodload": "src/ported/module.rs",

    # Src/Modules/files.c -> src/ported/modules/files.rs
    "builtin_chmod": "src/ported/modules/files.rs",
    "builtin_chown": "src/ported/modules/files.rs",
    "builtin_chgrp": "src/ported/modules/files.rs",
    "builtin_ln":    "src/ported/modules/files.rs",
    "builtin_mv":    "src/ported/modules/files.rs",
    "builtin_mkdir": "src/ported/modules/files.rs",
    "builtin_rm":    "src/ported/modules/files.rs",
    "builtin_rmdir": "src/ported/modules/files.rs",
    "builtin_sync":  "src/ported/modules/files.rs",

    # Src/Modules/system.c -> src/ported/modules/system.rs
    "builtin_syserror":          "src/ported/modules/system.rs",
    "builtin_sysopen":           "src/ported/modules/system.rs",
    "builtin_sysread":           "src/ported/modules/system.rs",
    "builtin_sysseek":           "src/ported/modules/system.rs",
    "builtin_syswrite":          "src/ported/modules/system.rs",
    "builtin_zsystem":           "src/ported/modules/system.rs",
    "builtin_zsystem_flock":     "src/ported/modules/system.rs",
    "builtin_zsystem_supports":  "src/ported/modules/system.rs",

    # Src/Modules/zutil.c -> src/ported/modules/zutil.rs
    "builtin_zformat":     "src/ported/modules/zutil.rs",
    "builtin_zparseopts":  "src/ported/modules/zutil.rs",
    "builtin_zregexparse": "src/ported/modules/zutil.rs",
    "builtin_zstyle":      "src/ported/modules/zutil.rs",

    # Src/Modules/db_gdbm.c -> src/ported/modules/db_gdbm.rs
    "builtin_zgdbmpath": "src/ported/modules/db_gdbm.rs",
    "builtin_ztie":      "src/ported/modules/db_gdbm.rs",
    "builtin_zuntie":    "src/ported/modules/db_gdbm.rs",

    # Src/Modules/stat.c -> src/ported/modules/stat.rs
    "builtin_zstat": "src/ported/modules/stat.rs",

    # Src/Modules/terminfo.c -> src/ported/modules/terminfo.rs
    "builtin_echoti": "src/ported/modules/terminfo.rs",

    # Src/Zle/zle_main.c (interactive) -> src/ported/zle/zle_main.rs
    "builtin_bindkey": "src/ported/zle/zle_main.rs",
    "builtin_vared":   "src/ported/zle/zle_main.rs",
    "builtin_zle":     "src/ported/zle/zle_main.rs",

    # Src/Zle/computil.c -> src/ported/zle/computil.rs
    "builtin_compquote":  "src/ported/zle/computil.rs",
    "builtin_comptags":   "src/ported/zle/computil.rs",
    "builtin_comptry":    "src/ported/zle/computil.rs",
    "builtin_compvalues": "src/ported/zle/computil.rs",

    # Src/Zle/complete.c -> src/ported/zle/compcore.rs (closest match)
    "builtin_compadd": "src/ported/zle/compcore.rs",
    "builtin_compset": "src/ported/zle/compcore.rs",
}

SIG_RE = re.compile(r"^(    )(pub(?:\(crate\))? )?(fn (builtin_[a-z0-9_]+))\b")
ATTR_OR_DOC_RE = re.compile(r"^    (?://|///|#\[)")
END_RE = re.compile(r"^    \}\s*$")
IMPL_OPEN_RE = re.compile(r"^impl(?:<[^>]*>)?\s+(.+?)\s*\{")
MARKER = "// BEGIN moved-from-exec-rs\n"


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
    missing = set(TARGETS) - found
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

    # group by destination
    extracted_per_dest: dict[str, list[str]] = defaultdict(list)
    for start, end, name, _ in sorted(blocks, key=lambda b: b[0]):
        chunk = "".join(lines[start:end + 1])
        new_lines = []
        for ln in chunk.splitlines(keepends=True):
            m = SIG_RE.match(ln)
            if m and (m.group(2) is None or m.group(2).strip() == ""):
                new_lines.append(f"{m.group(1)}pub(crate) {m.group(3)}{ln[m.end():]}")
            else:
                new_lines.append(ln)
        extracted_per_dest[TARGETS[name]].append("".join(new_lines))

    # remove from exec.rs
    for start, end, _, _ in sorted(blocks, key=lambda b: b[0], reverse=True):
        end_strip = end + 1
        if end_strip < len(lines) and lines[end_strip].strip() == "":
            end_strip += 1
        del lines[start:end_strip]

    EXEC.write_text("".join(lines))
    print(f"removed {len(blocks)} methods from exec.rs")

    # append to each dest
    for dest_rel, chunks in sorted(extracted_per_dest.items()):
        dest_path = ROOT / dest_rel
        dest_text = dest_path.read_text()
        # multiple moved blocks per file are OK; skip the marker
        # uniqueness check.
        additions = []
        additions.append("\n")
        additions.append("// ===========================================================\n")
        additions.append("// Methods moved verbatim from src/ported/exec.rs because their\n")
        additions.append(f"// C counterpart's source file maps 1:1 to this Rust module.\n")
        additions.append("// Rust permits multiple inherent impl blocks for the same\n")
        additions.append("// type within a crate, so call sites in exec.rs are unchanged.\n")
        additions.append("// ===========================================================\n")
        additions.append("\n")
        additions.append(MARKER)
        additions.append("impl crate::ported::exec::ShellExecutor {\n")
        for chunk in chunks:
            additions.append(chunk)
        additions.append("}\n")
        additions.append("// END moved-from-exec-rs\n")
        dest_path.write_text(dest_text + "".join(additions))
        print(f"appended {len(chunks):>2} methods -> {dest_rel}")


if __name__ == "__main__":
    main()
