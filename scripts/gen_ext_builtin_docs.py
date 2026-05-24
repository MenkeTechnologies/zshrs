#!/usr/bin/env python3
"""Extract `///` doc-comments above each extension-builtin / daemon
`z*` builtin function and emit a Rust const used by the LSP hover
cascade. Run once — the output is committed at
`src/extensions/zsh_ext_builtin_docs.rs`.

Sources:
  * `daemon/builtins.rs`           — every `fn z*(args: &[String]) -> i32`
  * `src/extensions/ext_builtins.rs` — every `pub(crate) fn builtin_NAME`

For each fn we collect the leading `///` block (contiguous, immediately
preceding the fn). Doc-comment markers + the leading ` ` after `///`
are stripped; intra-paragraph leading whitespace is preserved so usage
blocks render as indented code.

Names without a doc-comment are simply omitted from the generated
table — `lookup_doc` then falls back to the hand `EXT_BUILTIN_DOCS`
one-liner in `src/extensions/lsp.rs`.

Re-run when you add / fatten a doc-comment on a builtin:

  ./scripts/gen_ext_builtin_docs.py
"""

from __future__ import annotations

import datetime
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DAEMON = ROOT / "daemon" / "builtins.rs"
EXT = ROOT / "src" / "extensions" / "ext_builtins.rs"
OUT = ROOT / "src" / "extensions" / "zsh_ext_builtin_docs.rs"


def extract_doc_blocks(path: Path, fn_re: re.Pattern[str]) -> dict[str, str]:
    """Pull `(name, doc_body)` pairs from `path` where the fn name
    matches `fn_re` (which must have one group capturing the name)."""
    text = path.read_text(encoding="utf-8").splitlines()
    out: dict[str, str] = {}
    buf: list[str] = []
    for line in text:
        m = fn_re.search(line)
        if m and buf:
            out[m.group(1)] = "\n".join(buf).strip()
            buf = []
            continue
        # `///` blocks only count when contiguous with the next fn —
        # any non-doc, non-blank line resets the buffer.
        stripped = line.lstrip()
        if stripped.startswith("///"):
            # Strip the `///` and at most one following space.
            body = stripped[3:]
            if body.startswith(" "):
                body = body[1:]
            buf.append(body)
        elif stripped == "":
            # Blank line: end of one block, but the NEXT line might be
            # another `///` block separated by a blank for paragraph
            # break. Keep buf so multi-paragraph doc-comments survive.
            if buf:
                buf.append("")
        else:
            # Non-doc, non-blank — anything else resets.
            buf = []
    return out


def rust_escape(s: str) -> str:
    return s.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n")


def main() -> None:
    daemon_docs = extract_doc_blocks(
        DAEMON,
        re.compile(r"^fn (z[A-Za-z0-9_-]+)\(args:\s*&\[String\]\)"),
    )
    ext_docs = extract_doc_blocks(
        EXT,
        re.compile(r"^\s*pub\(crate\)\s+fn\s+builtin_([A-Za-z0-9_]+)\("),
    )
    # Source-name prefix `builtin_X` maps to the builtin spelling `X`
    # (some have underscore-to-nothing transforms — e.g. `intercept_proceed`
    # keeps its `_`). Strip the prefix only; nothing else needed.
    merged: dict[str, str] = {}
    for name, body in daemon_docs.items():
        if body.strip():
            merged[name] = body
    for name, body in ext_docs.items():
        if body.strip():
            merged[name] = body

    # Sort for review-friendly diffs.
    entries = sorted(merged.items())

    lines: list[str] = []
    lines.append("// AUTO-GENERATED. DO NOT EDIT BY HAND.")
    lines.append("//")
    lines.append("// Source: `daemon/builtins.rs` + `src/extensions/ext_builtins.rs`")
    lines.append("// — extracted `///` doc-comments above each extension /")
    lines.append("// daemon `z*` builtin function.")
    lines.append(f"// Regen: ./scripts/gen_ext_builtin_docs.py")
    lines.append(f"// Generated: {datetime.date.today().isoformat()}")
    lines.append("")
    lines.append("/// Full doc-comment bodies for zshrs-only builtins. Wins over")
    lines.append("/// the hand `EXT_BUILTIN_DOCS` one-liners in `lsp.rs` when a")
    lines.append("/// name appears here — names without a `///` block in source")
    lines.append("/// fall through to the hand fallback.")
    lines.append("pub const EXT_BUILTIN_FULL_DOCS: &[(&str, &str)] = &[")
    for name, body in entries:
        lines.append(f'    ("{rust_escape(name)}", "{rust_escape(body)}"),')
    lines.append("];")
    lines.append("")
    lines.append("/// Lookup helper used by `lsp::lookup_doc`.")
    lines.append("pub fn lookup_full(name: &str) -> Option<&'static str> {")
    lines.append("    EXT_BUILTIN_FULL_DOCS")
    lines.append("        .iter()")
    lines.append("        .find(|(k, _)| *k == name)")
    lines.append("        .map(|&(_, body)| body)")
    lines.append("}")
    lines.append("")

    OUT.write_text("\n".join(lines), encoding="utf-8")
    print(f"wrote {OUT.relative_to(ROOT)} — {len(entries)} entries")


if __name__ == "__main__":
    main()
