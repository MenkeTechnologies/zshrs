#!/usr/bin/env python3
"""Normalize file-header docstrings on every Rust port under
``compsys/ported/`` so every port carries a uniform, debuggable
upstream citation.

Standard header shape:

    //! Port of `_<name>` from `Completion/<dir>/<name>`.
    //!
    //! Full upstream body (<N> lines verbatim):
    //! ```text
    //! sh:1  <line 1>
    //! sh:2  <line 2>
    //! ...
    //! ```
    //!
    //! [optional extra Rust-only notes preserved from old header]

Rules:

* Only the leading ``//!`` block at the top of the file is replaced.
* The first non-doc line (``use ...``, ``pub fn ...``, ``#[...]``,
  any non-``//!`` line) marks the end of the header — body and tests
  below are left byte-for-byte untouched.
* If the matching upstream shell file is absent, the file is skipped
  with a notice (so we don't strip the existing header in error).
* Existing extra-context notes after the shell body block are
  preserved IF the previous header contained a ``Strict Rust port``
  or ``Notes`` section — we capture the remainder of the old header
  after the shell ``text`` block ends and re-attach it.

Run with ``--dry-run`` first to see what changes; default rewrites.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PORTED = ROOT / "src" / "compsys" / "ported"


def split_header(text: str) -> tuple[list[str], str]:
    """Split a Rust file into (header_lines_with_trailing_newlines, rest)."""
    lines = text.splitlines(keepends=True)
    end = 0
    for i, line in enumerate(lines):
        stripped = line.lstrip()
        if stripped.startswith("//!"):
            end = i + 1
            continue
        if stripped == "":
            # blank line WITHIN header — keep absorbing if next non-blank
            # is still //!
            ahead = next(
                (
                    j
                    for j in range(i + 1, len(lines))
                    if lines[j].strip() != ""
                ),
                None,
            )
            if ahead is not None and lines[ahead].lstrip().startswith("//!"):
                continue
            break
        break
    return lines[:end], "".join(lines[end:])


def make_header(
    name: str, citation_path: str, shell_text: str, extra_notes: str | None
) -> str:
    """Build the standardized //! header block."""
    shell_lines = shell_text.splitlines()
    width = max(2, len(str(len(shell_lines))))
    body_lines = [
        f"//! sh:{i:>{width}}  {line}".rstrip()
        for i, line in enumerate(shell_lines, 1)
    ]

    pieces = [
        f"//! Port of `{name}` from `{citation_path}`.",
        "//!",
        f"//! Full upstream body ({len(shell_lines)} lines verbatim):",
        "//! ```text",
        *body_lines,
        "//! ```",
    ]
    if extra_notes and extra_notes.strip():
        pieces.append("//!")
        for line in extra_notes.splitlines():
            line = line.rstrip()
            if line.startswith("//!"):
                pieces.append(line)
            elif line == "":
                pieces.append("//!")
            else:
                pieces.append("//! " + line)
    pieces.append("")  # trailing blank to separate header from body
    return "\n".join(pieces) + "\n"


# Extra-notes detector: capture everything in the OLD header that
# comes AFTER the closing fence of the previous shell-body code block.
EXTRA_NOTES_RE = re.compile(
    r"//!\s*```\s*\n((?:.*\n)*?)(?=^use\s|^pub\s|^fn\s|^#\[|\Z)",
    re.MULTILINE,
)


def extract_extra_notes(old_header: str) -> str | None:
    """Pull any post-shell-body notes from the OLD header for preservation."""
    matches = list(re.finditer(r"//!\s*```\s*\n", old_header))
    if not matches:
        return None
    after = old_header[matches[-1].end() :]
    note_lines = []
    for line in after.splitlines():
        stripped = line.lstrip()
        if not stripped.startswith("//!"):
            break
        content = stripped[3:].lstrip()
        if content.startswith("Local shell reference") or content.startswith(
            "Upstream shell source"
        ) or content.startswith("Full upstream body"):
            continue
        note_lines.append(content)
    while note_lines and not note_lines[0]:
        note_lines.pop(0)
    while note_lines and not note_lines[-1]:
        note_lines.pop()
    return "\n".join(note_lines) if note_lines else None


def process_file(rs_path: Path, dry_run: bool) -> str:
    """Return one of: 'rewrote', 'unchanged', 'no_shell', 'no_header'."""
    rel = rs_path.relative_to(PORTED).as_posix()
    shell_path = rs_path.with_suffix("")
    if not shell_path.is_file():
        return "no_shell"
    rel_dir = str(rs_path.parent.relative_to(PORTED))
    name = rs_path.stem
    # Top-level files (compinit, compdump) live at the root of
    # Completion/, not under a subdir — drop the "." dir component.
    citation_path = (
        f"Completion/{name}" if rel_dir == "." else f"Completion/{rel_dir}/{name}"
    )

    text = rs_path.read_text()
    header_lines, body = split_header(text)
    if not header_lines:
        return "no_header"
    old_header = "".join(header_lines)

    shell_text = shell_path.read_text().rstrip("\n")
    extras = extract_extra_notes(old_header)
    new_header = make_header(name, citation_path, shell_text, extras)

    if new_header == old_header:
        return "unchanged"

    if dry_run:
        return "rewrote"

    rs_path.write_text(new_header + body)
    return "rewrote"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--dry-run", action="store_true",
                    help="report which files would change; do not write")
    ap.add_argument("paths", nargs="*", type=Path,
                    help="optional list of specific .rs paths to process")
    args = ap.parse_args()

    if args.paths:
        rs_files = [p.resolve() for p in args.paths]
    else:
        rs_files = sorted(PORTED.rglob("*.rs"))
        # Skip infrastructure files
        rs_files = [f for f in rs_files if f.name not in {"mod.rs", "shared.rs"}]

    counts = {"rewrote": 0, "unchanged": 0, "no_shell": 0, "no_header": 0}
    rewrote_paths: list[str] = []
    no_shell_paths: list[str] = []
    no_header_paths: list[str] = []
    for f in rs_files:
        outcome = process_file(f, args.dry_run)
        counts[outcome] += 1
        rel = f.relative_to(PORTED).as_posix()
        if outcome == "rewrote":
            rewrote_paths.append(rel)
        elif outcome == "no_shell":
            no_shell_paths.append(rel)
        elif outcome == "no_header":
            no_header_paths.append(rel)

    verb = "would rewrite" if args.dry_run else "rewrote"
    print(f"{verb}: {counts['rewrote']}  unchanged: {counts['unchanged']}  "
          f"no-shell-counterpart: {counts['no_shell']}  no-header: {counts['no_header']}")
    if no_shell_paths:
        print("\nFiles with no shell counterpart (header NOT normalized):")
        for p in no_shell_paths:
            print(f"  {p}")
    if no_header_paths:
        print("\nFiles with no //! header (skipped):")
        for p in no_header_paths:
            print(f"  {p}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
