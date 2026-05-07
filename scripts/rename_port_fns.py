#!/usr/bin/env python3
"""Apply rename pairs from /tmp/renames.tsv to the codebase.

Each line of renames.tsv is `<rust_path>\t<line>\t<old_name>\t<new_name>`.
For each row this script:
  1. Skips if old_name == new_name (no work).
  2. Verifies old_name appears as a fn definition in <rust_path>.
  3. Walks all *.rs under src/ and tests/, replacing whole-word
     occurrences of old_name with new_name.
  4. Optionally guards against collisions: refuses to rename if
     new_name already appears as a fn def somewhere else (would
     create a duplicate symbol).

Run with --dry-run to preview, or --apply to actually edit.

Usage:
    python3 scripts/rename_port_fns.py [--dry-run|--apply] [--limit N] \\
        [--filter SUBSTR] [--exclude SUBSTR]
"""
import argparse
import os
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RENAMES_TSV = Path("/tmp/renames.tsv")


def load_renames():
    pairs = []
    with open(RENAMES_TSV) as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            parts = line.split("\t")
            if len(parts) != 4:
                continue
            rust_path, lineno, old, new = parts
            if old == new:
                continue
            pairs.append((rust_path, int(lineno), old, new))
    return pairs


def find_call_sites(name):
    """Files where `name` appears as a whole word."""
    try:
        out = subprocess.check_output(
            ["grep", "-rl", f"\\b{name}\\b", str(ROOT / "src"), str(ROOT / "tests")],
            text=True,
        )
    except subprocess.CalledProcessError:
        return []
    return [Path(p) for p in out.strip().splitlines() if p.endswith(".rs")]


def whole_word_replace(text, old, new):
    """Replace whole-word occurrences of old with new — but NOT when
    the identifier is qualified by an external path like `libc::X`,
    `ffi::X`, `c::X`, or `zsh_sys::X`. Those refer to FFI bindings
    whose name follows the foreign C convention (e.g. `libc::getxattr`
    for the actual libc syscall). Renaming those would break the
    extern binding lookup.
    """
    # Negative lookbehind for `<ident>::` qualifiers. Rust paths are
    # `::`-separated; if the previous chars are `::` then this is a
    # qualified call, leave it alone.
    pattern = re.compile(rf"(?<!::)\b{re.escape(old)}\b")
    return pattern.subn(new, text)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--apply", action="store_true")
    ap.add_argument("--limit", type=int, default=0,
                    help="Process at most N renames")
    ap.add_argument("--filter", default="",
                    help="Only process renames whose rust_path contains this substring")
    ap.add_argument("--exclude", default="",
                    help="Skip renames whose rust_path contains this substring")
    args = ap.parse_args()

    if not (args.dry_run or args.apply):
        ap.error("specify --dry-run or --apply")

    pairs = load_renames()
    if args.filter:
        pairs = [p for p in pairs if args.filter in p[0]]
    if args.exclude:
        pairs = [p for p in pairs if args.exclude not in p[0]]
    if args.limit:
        pairs = pairs[:args.limit]

    print(f"Processing {len(pairs)} renames")

    total_files_changed = set()
    total_replacements = 0
    skipped = []

    for rust_path, lineno, old, new in pairs:
        # Sanity: does the file have a fn def with the old name?
        target = ROOT / rust_path
        if not target.exists():
            skipped.append((rust_path, old, new, "file missing"))
            continue
        src = target.read_text()
        # Look for `fn <old>(` somewhere in the file.
        if not re.search(rf"\bfn\s+{re.escape(old)}\b", src):
            skipped.append((rust_path, old, new, "no fn def for old name"))
            continue

        # Collision check: does the new name already exist as a fn?
        # Allow rename if it's the same file (already has both, e.g.
        # cfg-gated dup) and we want to unify.
        sites = find_call_sites(new)
        new_already_def = any(
            re.search(rf"\bfn\s+{re.escape(new)}\b", p.read_text())
            for p in sites
        )
        if new_already_def:
            # Find which file(s).
            def_files = []
            for p in sites:
                try:
                    if re.search(rf"\bfn\s+{re.escape(new)}\b", p.read_text()):
                        def_files.append(str(p.relative_to(ROOT)))
                except Exception:
                    pass
            skipped.append((rust_path, old, new,
                            f"collision: new name already defined in {def_files}"))
            continue

        # Find all files with old-name occurrences.
        callers = find_call_sites(old)
        # Add the target file in case grep missed it.
        if target not in callers:
            callers.append(target)

        per_pair_replacements = 0
        per_pair_files = []
        for c in callers:
            text = c.read_text()
            new_text, n = whole_word_replace(text, old, new)
            if n > 0:
                per_pair_replacements += n
                per_pair_files.append((c, n, new_text))

        if per_pair_replacements == 0:
            skipped.append((rust_path, old, new, "no replacements"))
            continue

        if args.apply:
            for c, _n, new_text in per_pair_files:
                c.write_text(new_text)

        total_replacements += per_pair_replacements
        for c, _, _ in per_pair_files:
            total_files_changed.add(c)
        prefix = "[dry] " if args.dry_run else "[apply] "
        print(f"{prefix}{rust_path}: {old} -> {new} "
              f"({per_pair_replacements} replacement(s) across "
              f"{len(per_pair_files)} file(s))")

    print()
    print(f"Total: {total_replacements} replacements in "
          f"{len(total_files_changed)} files")
    if skipped:
        print(f"Skipped {len(skipped)}:")
        for rust_path, old, new, reason in skipped:
            print(f"  {rust_path}: {old} -> {new} ({reason})")


if __name__ == "__main__":
    main()
