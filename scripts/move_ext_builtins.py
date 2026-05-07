#!/usr/bin/env python3
"""One-shot mover: extract extension-only builtin_* methods from
src/ported/exec.rs into src/extensions/ext_builtins.rs.

Extension-only = no corresponding `bin_<name>` C handler and no
matching BUILTIN("<name>", ...) row in src/zsh/Src/.
"""
from __future__ import annotations
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
EXEC = ROOT / "src/ported/exec.rs"
DEST = ROOT / "src/extensions/ext_builtins.rs"

# 86 extension-only method names (verified against src/zsh/Src/).
TARGETS = {
    "builtin_add_zsh_hook", "builtin_arch", "builtin_async", "builtin_await",
    "builtin_barrier", "builtin_base64", "builtin_basename", "builtin_caller",
    "builtin_cat", "builtin_cdreplay", "builtin_cksum", "builtin_comm",
    "builtin_compdef", "builtin_compgen", "builtin_compinit", "builtin_complete",
    "builtin_compopt", "builtin_coproc", "builtin_cp", "builtin_cut",
    "builtin_date", "builtin_dbview", "builtin_dircolors", "builtin_dirname",
    "builtin_doctor", "builtin_env", "builtin_expand", "builtin_expr",
    "builtin_factor", "builtin_find", "builtin_fold", "builtin_groups",
    "builtin_head", "builtin_help", "builtin_hostname", "builtin_id",
    "builtin_intercept", "builtin_intercept_proceed", "builtin_link",
    "builtin_logname", "builtin_mkfifo", "builtin_mktemp", "builtin_nice",
    "builtin_nl", "builtin_nocorrect", "builtin_nproc", "builtin_paste",
    "builtin_peach", "builtin_pgrep", "builtin_pmap", "builtin_printenv",
    "builtin_profile", "builtin_prompt", "builtin_promptinit", "builtin_readarray",
    "builtin_realpath", "builtin_rev", "builtin_seq", "builtin_sha256sum",
    "builtin_shopt", "builtin_shuf", "builtin_sleep", "builtin_sort", "builtin_sum",
    "builtin_tac", "builtin_tail", "builtin_tee", "builtin_touch", "builtin_tput",
    "builtin_tr", "builtin_tsort", "builtin_tty", "builtin_uname",
    "builtin_unexpand", "builtin_uniq", "builtin_unlink", "builtin_users",
    "builtin_wc", "builtin_whoami", "builtin_yes", "builtin_zattr",
    "builtin_zbuild", "builtin_zcalc", "builtin_zfiles", "builtin_zmv",
    "builtin_zsleep",
}

SIG_RE = re.compile(r"^(    )(pub(?:\(crate\))? )?(fn (builtin_[a-z0-9_]+))\b")
ATTR_OR_DOC_RE = re.compile(r"^    (?://|///|#\[)")
END_RE = re.compile(r"^    \}\s*$")
IMPL_OPEN_RE = re.compile(r"^impl(?:<[^>]*>)?\s+(.+?)\s*\{")


def find_blocks(lines):
    """Return list of (start_idx, end_idx_inclusive, name, in_trait_impl)."""
    blocks = []
    in_trait_impl = False
    impl_depth = 0
    for i, line in enumerate(lines):
        stripped = line.rstrip("\n")
        # Track top-level impl open/close (impl line at column 0)
        if not line.startswith(" ") and not line.startswith("\t"):
            m = IMPL_OPEN_RE.match(line)
            if m:
                in_trait_impl = " for " in m.group(1)
                impl_depth = 1
                continue
            if stripped == "}" and impl_depth > 0:
                impl_depth = 0
                in_trait_impl = False
                continue
        m = SIG_RE.match(line)
        if not m:
            continue
        name = m.group(4)
        if name not in TARGETS:
            continue
        # Walk backwards to capture doc / attr lines (and one optional blank)
        start = i
        j = i - 1
        while j >= 0 and ATTR_OR_DOC_RE.match(lines[j]):
            start = j
            j -= 1
        # Find matching closing brace
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
    found_names = {b[2] for b in blocks}
    missing = TARGETS - found_names
    if missing:
        print(f"WARNING: {len(missing)} target methods not found:")
        for n in sorted(missing):
            print(f"  {n}")
    print(f"found {len(blocks)} method blocks (targets={len(TARGETS)})")

    trait_blocks = [b for b in blocks if b[3]]
    if trait_blocks:
        print(f"WARNING: {len(trait_blocks)} target methods live inside trait impls — skipping:")
        for b in trait_blocks:
            print(f"  {b[2]} at line {b[0]+1}")
    blocks = [b for b in blocks if not b[3]]

    # Build extracted body and remove from exec.rs in reverse order
    extracted_chunks = []
    for start, end, name, _ in sorted(blocks, key=lambda b: b[0]):
        chunk = "".join(lines[start:end + 1])
        # Bump signature to `pub(crate) fn` if not already public
        new_chunk_lines = []
        for ln in chunk.splitlines(keepends=True):
            m = SIG_RE.match(ln)
            if m and (m.group(2) is None or m.group(2).strip() == ""):
                new_chunk_lines.append(f"{m.group(1)}pub(crate) {m.group(3)}{ln[m.end():]}")
            else:
                new_chunk_lines.append(ln)
        extracted_chunks.append("".join(new_chunk_lines))

    # Remove in reverse to keep indices valid
    for start, end, _, _ in sorted(blocks, key=lambda b: b[0], reverse=True):
        # also strip a single trailing blank line if present
        end_strip = end + 1
        if end_strip < len(lines) and lines[end_strip].strip() == "":
            end_strip += 1
        del lines[start:end_strip]

    EXEC.write_text("".join(lines))
    print(f"removed {len(blocks)} methods from exec.rs")

    # Build destination file
    header = '''//! Extension-only shell builtins (no zsh C counterpart).
//!
//! Every method here implements a builtin that does NOT exist in
//! `src/zsh/Src/` — these are zshrs-specific additions: coreutils
//! drop-ins, bash-only builtins (caller, shopt, readarray), zshrs
//! features (async, await, barrier, doctor, profile, intercept),
//! contrib autoloads exposed as builtins (compdef, compinit, zmv,
//! zcalc, peach), etc.
//!
//! These methods previously lived on `ShellExecutor` in
//! `src/ported/exec.rs`. They were bulk-moved here so that
//! `src/ported/` only contains C-port code, satisfying the
//! `port_purity` discipline described in `docs/PORT.md`.

#![allow(unused_imports)]

use crate::ported::exec::ShellExecutor;

impl ShellExecutor {
'''
    footer = "}\n"
    body = "\n".join(c.rstrip("\n") + "\n" for c in extracted_chunks)
    DEST.write_text(header + "\n" + body + footer)
    print(f"wrote {DEST.relative_to(ROOT)} with {len(extracted_chunks)} methods")


if __name__ == "__main__":
    main()
