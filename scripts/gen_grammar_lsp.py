#!/usr/bin/env python3
"""Regenerate the canonical const arrays in src/extensions/lsp.rs and
the LSP completion-words file from data/grammar/canonical.json.

Target blocks (marked in `lsp.rs`):
    // BEGIN-CANONICAL: keywords ... // END-CANONICAL: keywords
    // BEGIN-CANONICAL: builtins ... // END-CANONICAL: builtins
    // BEGIN-CANONICAL: options  ... // END-CANONICAL: options

Target file `completions/lsp_completion_words.txt`:
    overwritten in full — one word per line, sorted, with all
    keywords + builtins (core/module/zshrs ext) + options +
    special vars (sigil-prefixed) + parameter flags +
    glob qualifiers + operators.

Re-run after editing canonical.json. `cargo build` should remain
green; this is a verbatim replace within marker boundaries.
"""
from __future__ import annotations

import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CANON = ROOT / "data" / "grammar" / "canonical.json"
LSP_RS = ROOT / "src" / "extensions" / "lsp.rs"
WORDS = ROOT / "completions" / "lsp_completion_words.txt"


def rust_escape(s: str) -> str:
    """Escape a Rust string literal."""
    return s.replace("\\", "\\\\").replace("\"", "\\\"")


def render_str_array(name: str, items: list[str]) -> str:
    lines = [f"const {name}: &[&str] = &["]
    for it in items:
        lines.append(f"    \"{rust_escape(it)}\",")
    lines.append("];")
    return "\n".join(lines)


def replace_block(src: str, tag: str, body: str) -> str:
    pat = re.compile(
        rf"(// BEGIN-CANONICAL: {re.escape(tag)}\n).*?(\n// END-CANONICAL: {re.escape(tag)})",
        re.S,
    )
    if not pat.search(src):
        raise SystemExit(f"marker for tag '{tag}' not found in {LSP_RS}")
    return pat.sub(lambda m: m.group(1) + body + m.group(2), src)


def main() -> None:
    data = json.loads(CANON.read_text(encoding="utf-8"))

    keywords = [e["name"] for e in data["keywords"]]
    # Builtin set used by LSP completion = POSIX/zsh core + module +
    # zshrs extension. De-dup, sort.
    builtins = sorted({
        *(e["name"] for e in data["core_builtins"]),
        *(e["name"] for e in data["module_builtins"]),
        *(e["name"] for e in data["zshrs_ext_builtins"]),
    })
    options = sorted({e["name"].upper() for e in data["options"]})

    src = LSP_RS.read_text(encoding="utf-8")
    src = replace_block(src, "keywords", render_str_array("KEYWORDS", keywords))
    src = replace_block(src, "builtins", render_str_array("BUILTINS", builtins))
    src = replace_block(src, "options", render_str_array("OPTIONS", options))
    LSP_RS.write_text(src, encoding="utf-8")
    print(f"updated {LSP_RS}: "
          f"{len(keywords)} keywords, "
          f"{len(builtins)} builtins, "
          f"{len(options)} options")

    # completion_words.txt — union of every named token form
    words: set[str] = set()
    words.update(keywords)
    words.update(builtins)
    words.update(options)
    # special vars: prefix with $ (sigiled), except symbolic ones
    for e in data["special_vars"]:
        n = e["name"]
        if n.isidentifier() or n.isupper() or n.islower():
            words.add(f"${n}")
        else:
            words.add(f"${n}")
    # parameter flags inside ${(X)var}
    for e in data["param_flags"]:
        words.add(f"${{({e['flag']})var}}")
    # glob qualifiers inside *(X)
    for e in data["glob_qualifiers"]:
        sym = e["sym"].split()[0]
        words.add(f"*({sym})")
    # operators (raw symbol form)
    for e in data["operators"]:
        words.add(e["sym"])
    # history expansions / modifiers
    for e in data["history_expansions"]:
        words.add(e["sym"])
    for e in data["modifiers"]:
        words.add(e["sym"])

    sorted_words = sorted(words, key=lambda s: (s.lstrip("$").lower(), s))
    WORDS.write_text("\n".join(sorted_words) + "\n", encoding="utf-8")
    print(f"updated {WORDS}: {len(sorted_words)} entries")


if __name__ == "__main__":
    main()
