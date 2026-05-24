#!/usr/bin/env python3
"""Generate Rust doc tables from upstream zsh yodl source files.

Pulls (canonical name, markdown body) pairs out of every
`item(tt(NAME)...)( ... )` block, plus the negated alias pindex /
vindex / findex lines that precede each item.

Supports four upstream files via `--source`:

  options    Doc/Zsh/options.yo    → zsh_option_docs.rs
  builtins   Doc/Zsh/builtins.yo   → zsh_builtin_docs.rs
  keywords   Doc/Zsh/grammar.yo    → zsh_keyword_docs.rs
  specials   Doc/Zsh/params.yo     → zsh_special_var_docs.rs

Run:
  ./scripts/gen_option_docs.py --source options  ~/forkedRepos/zsh/Doc/Zsh/options.yo  > src/extensions/zsh_option_docs.rs
  ./scripts/gen_option_docs.py --source builtins ~/forkedRepos/zsh/Doc/Zsh/builtins.yo > src/extensions/zsh_builtin_docs.rs
  ./scripts/gen_option_docs.py --source keywords ~/forkedRepos/zsh/Doc/Zsh/grammar.yo  > src/extensions/zsh_keyword_docs.rs
  ./scripts/gen_option_docs.py --source specials ~/forkedRepos/zsh/Doc/Zsh/params.yo   > src/extensions/zsh_special_var_docs.rs

Re-run whenever the upstream zsh source updates. The output file is
deterministic so diffs stay reviewable.
"""

import re
import sys
import datetime
from pathlib import Path

# Per-source config. Each entry tells the generator:
#   alias_macros    yodl macros that introduce alias names before
#                   an `item(...)` block.
#   name_re         regex for the canonical name accepted by item(...).
#   table_name      Rust `pub const NAME_DOCS: &[(&str, &str)]` name.
#   alias_table     Rust alias-table name (or None to skip).
#   lookup_fn       Rust lookup-fn name.
#   uppercase_alias whether the lookup helper upper-cases the key.
#   key_prefix      string prefixed to every alias and canonical key
#                   in the alias table (e.g. `$` for special vars).
SOURCES = {
    "options": {
        "alias_macros": ("pindex",),
        "name_re": r"[A-Z_][A-Z0-9_]*",
        "table_name": "OPTION_DOCS",
        "alias_table": "OPTION_ALIASES",
        "lookup_fn": "lookup_option_doc",
        "uppercase_alias": True,
        "key_prefix": "",
        "kind_label": "option",
    },
    "builtins": {
        "alias_macros": ("findex", "pindex", "cindex"),
        "name_re": r"[A-Za-z_][A-Za-z0-9_]*",
        "table_name": "BUILTIN_DOCS",
        # Emit an alias table so xitem-only entries (`xitem(tt(comptags))`
        # followed by `item(tt(comptry))`) and findex-declared synonyms
        # (`findex(comptags)` + `findex(comptry)` → same body) resolve.
        # Without aliasing 23/152 canonical builtins fall through to the
        # placeholder; with it the count drops to the truly-undocumented
        # internals (chdir/bye/declare/etc.) and they get a hand-curated
        # fallback in `lookup_doc`.
        "alias_table": "BUILTIN_ALIASES",
        "lookup_fn": "lookup_builtin_doc",
        "uppercase_alias": False,
        "key_prefix": "",
        "kind_label": "builtin",
    },
    "keywords": {
        "alias_macros": ("findex", "pindex", "cindex"),
        "name_re": r"[A-Za-z_][A-Za-z0-9_]*",
        "table_name": "KEYWORD_DOCS",
        "alias_table": None,
        "lookup_fn": "lookup_keyword_doc",
        "uppercase_alias": False,
        "key_prefix": "",
        "kind_label": "keyword",
    },
    "specials": {
        "alias_macros": ("vindex", "pindex", "cindex"),
        "name_re": r"[A-Za-z_][A-Za-z0-9_]*",
        "table_name": "SPECIAL_VAR_DOCS",
        # Lowercase `path` + uppercase `PATH` share one body; emit an
        # alias table so the LSP resolves both to the same entry. Same
        # pattern for `cdpath`/`CDPATH`, `fpath`/`FPATH`, `manpath`/
        # `MANPATH`, `prompt`/`PROMPT`, etc.
        "alias_table": "SPECIAL_VAR_ALIASES",
        "lookup_fn": "lookup_special_var_doc",
        "uppercase_alias": False,
        "key_prefix": "",
        "kind_label": "special variable",
        # Entries without a preceding `vindex(NAME)` are subscript-
        # flag items / similar yodl noise — skip them. Only real
        # parameters get a `vindex(...)` declaration.
        "require_alias": True,
    },
}


# Every yodl `*index(...)` macro that may appear between an entry's
# primary-alias declarations and its `item(...)` body. All of these
# must be silently skipped without clearing `pending_aliases`.
INDEX_MACROS = ("pindex", "vindex", "findex", "cindex", "tindex")


YODL_INLINE = [
    # Order matters — the literal-paren producers (`LPAR()`, `RPAR()`,
    # `PLUS()`) run BEFORE the macro processors so a `tt(PLUS())` collapses
    # to `tt(+)` and then to `` `+` `` instead of `tt(+)` (whose inner
    # `(+)` would otherwise trip the no-nested-parens guard).
    (re.compile(r"LPAR\(\)"), "("),
    (re.compile(r"RPAR\(\)"), ")"),
    (re.compile(r"PLUS\(\)"), "+"),
    # `sitem(KEY)(VALUE)` — yodl short-item list entry. Two paren
    # groups. Rendered as a markdown list bullet so the `KEY → VALUE`
    # pairing survives. MUST appear before the unanchored `em(...)`
    # rule below or `sit` + `em(...)` substring-matches first and
    # mangles the output into `sit_KEY_(VALUE)`.
    (re.compile(r"sitem\(([^()]*)\)\(([^()]*)\)"), r"- \1 — \2"),
    (re.compile(r"startsitem\(\s*\)"), ""),
    (re.compile(r"endsitem\(\s*\)"), ""),
    # Body-internal `item(KEY)(VALUE)` pairs — these are NESTED items
    # inside a body the top-level parse_yo already extracted (e.g. the
    # bindkey body has `item(\`-e\`)(Selects keymap "emacs"…)`. Render
    # them as markdown list bullets so the option-list structure
    # survives in hover text. The `xitem(KEY)` header-only form (no
    # body paren) collapses to a bare bullet.
    (re.compile(r"\bitem\(([^()]*)\)\(([^()]*)\)"), r"\n- **\1** — \2\n"),
    (re.compile(r"\bxitem\(([^()]*)\)"), r"\n- **\1**\n"),
    (re.compile(r"\bstartitem\(\s*\)"), ""),
    (re.compile(r"\benditem\(\s*\)"), ""),
    (re.compile(r"NOTRANS\(([^()]*)\)"), r"\1"),
    # `\b` boundaries on tt/em/var/bf so they don't substring-match
    # inside identifiers like `sitem`, `startitem`, `bindkey -em`, etc.
    (re.compile(r"\btt\(([^()]*)\)"), r"`\1`"),
    (re.compile(r"\bem\(([^()]*)\)"), r"_\1_"),
    (re.compile(r"\bvar\(([^()]*)\)"), r"_\1_"),
    (re.compile(r"\bbf\(([^()]*)\)"), r"**\1**"),
    (re.compile(r"\bnoderef\(([^()]*)\)"), r"(\1)"),
    (re.compile(r"\bmanref\(([^,()]*),\s*([^()]*)\)"), r"`\1(\2)`"),
    (re.compile(r"\bfootnote\(([^()]*)\)"), r"(\1)"),
    # Section / cross references — drop the module suffix and keep the
    # human-readable section title.
    (re.compile(r"\bsectref\(([^,()]*)\)\(([^()]*)\)"), r"_\1_ (\2)"),
    (re.compile(r"\bifztexi\(([^()]*)\)"), r"\1"),
    (re.compile(r"\bifnztexi\(([^()]*)\)"), r"\1"),
    # `example(...)` and `indent(...)` wrap blocks — collapse to inline
    # so subsequent passes see plain prose. Real block-rendering would
    # require a structural parser; not worth it for hover text.
    (re.compile(r"\bexample\(([^()]*)\)"), r"\n\n    \1\n\n"),
    (re.compile(r"\bindent\(([^()]*)\)"), r"\n    \1\n"),
    # Yodl prose-quote forms. The leading-backtick + trailing-apostrophe
    # is yodl's smart-quote convention; conversion order matters because
    # `tt(X)` runs FIRST and turns into `` `X` `` — so a source `` ``tt(X)' ``
    # becomes `` ``X`' `` (double-backtick + content + backtick + apostrophe)
    # by the time we get here. Patterns run longest-first.
    #
    #   ``X`'  →  "X"     (yodl double-open quote wrapping a tt-converted span)
    #   ``X''  →  "X"     (yodl double-open / double-close)
    #   `X'    →  "X"     (yodl single quote pair)
    (re.compile(r"``([^`\n']+)`'"), r'"\1"'),
    (re.compile(r"``([^`\n']+)''"), r'"\1"'),
    (re.compile(r"`([^`\n']+)'"), r'"\1"'),
]


def strip_yodl(text: str) -> str:
    """Convert yodl markup to markdown / plain text."""
    # Repeat until no inline macros remain (handles nested simple cases).
    for _ in range(8):
        prev = text
        for rx, repl in YODL_INLINE:
            text = rx.sub(repl, text)
        if text == prev:
            break
    # Drop residual nested `xx(yy)` we couldn't match — leave the inner text.
    # `\b` boundary so we don't substring-strip `em(` from inside `item(`,
    # `sitem(`, `xitem(`, `enditem(`, etc.
    text = re.sub(r"\b(?:tt|em|var|bf)\(([^()]*)\)", r"\1", text)
    # Yodl list markers we can't render cleanly — drop them. Match the
    # full `(start|end)(s?)itemize?_*()_*` family that survives parsing.
    text = re.sub(r"\bstartitem\(\s*\)\s*\n?", "", text)
    text = re.sub(r"\benditem\(\s*\)\s*\n?", "", text)
    text = re.sub(r"\bstartsitem\(\s*\)\s*\n?", "", text)
    text = re.sub(r"\bendsitem\(\s*\)\s*\n?", "", text)
    # Legacy single-line `startit__`/`endit__` markers (produced by older
    # rule before sitem became a first-class transform).
    text = re.sub(r"\bstart(?:s)?it_+\s*\n?", "", text)
    text = re.sub(r"\bend(?:s)?it_+\s*\n?", "", text)
    text = re.sub(r"^x?it_", "", text, flags=re.MULTILINE)
    # Stray `COMMENT(...)` blocks left over by the parse (we don't have
    # nested-paren matching for arbitrary depth — drop the prefix).
    text = re.sub(r"COMMENT\(\!MOD\![^\n]*", "", text)
    text = re.sub(r"\!MOD\!\)", "", text)
    return text


def parse_yo(src: str, cfg: dict):
    """Yield (canonical_name, aliases, body_markdown) tuples.

    `aliases` is the list of `pindex(...)` names that preceded the
    `item(...)(` line — these are the negated / compact forms
    (`NO_NAME`, `NAMENAME`, `NONAMENAME`) that should hover-resolve
    to the same body.

    Item lines come in several shapes:
        item(tt(NAME))(
        item(tt(NAME) (tt(-X)))(            ← single-char alias flag
        item(tt(NAME) <K> <S>)(             ← deprecation markers
    Body opens at the `)(` boundary where item-header depth returns
    to 0 and a fresh `(` opens.
    """
    alias_macros = cfg["alias_macros"]
    # Capture the first identifier inside `tt(...)`, then accept any
    # non-`)` content until the closing paren of `tt`. This is needed
    # for multi-word headers like `item(tt(zsystem flock -u) var(fd))(`
    # where the canonical name is `zsystem` but the rest of the tt body
    # describes a subcommand. Also covers `xitem(tt(print )[…])` (space
    # before close) and `item(tt(NAME) (tt(-J)))(` (option-flag alias).
    name_re = re.compile(r"\s*item\(tt\((" + cfg["name_re"] + r")[^)]*\)")
    # Some entries are declared as a stack of `xitem(tt(NAME)...)` /
    # `xitem(SPACES()...)` lines followed by a final `item(...)( body )`
    # where the `item(...)` header may or may not start with `tt(NAME)`.
    # Track the last `xitem(tt(NAME))` so we can fall back to it when
    # the `item(...)` header begins with `SPACES()` / `var(...)` etc.
    xitem_re = re.compile(r"\s*xitem\(tt\((" + cfg["name_re"] + r")[^)]*\)")
    lines = src.split("\n")
    i = 0
    n = len(lines)
    pending_aliases = []
    pending_canonical = None
    while i < n:
        l = lines[i]
        stripped = l.lstrip()
        # Carry alias-introducing macros forward into the next item.
        # The first alias macro in the config is the "primary" — its
        # names actually get emitted into the alias table.
        #
        # We have to silently skip ALL recognized yodl index macros,
        # not just the per-source alias set: every `.yo` interleaves
        # `pindex`/`vindex`/`findex`/`cindex`/`tindex` lines between an
        # entry's primary-alias declarations and its `item(...)` line.
        # E.g. options.yo lines 60-65 are `pindex(AUTO_CD)` ×4 then
        # `cindex(cd, automatic)` then `item(tt(AUTO_CD)…)`. If we let
        # that `cindex` fall through to the "not an item" branch below
        # it clears `pending_aliases` and we lose the 3 negated/compact
        # forms (`AUTOCD`, `NO_AUTO_CD`, `NOAUTOCD`) → 121/203 zsh
        # options end up with no resolvable doc body via the canonical
        # `ZSH_OPTIONS_SET` lookup, even though upstream declares them.
        matched_alias = False
        for mac in INDEX_MACROS:
            if stripped.startswith(f"{mac}("):
                if mac == alias_macros[0]:
                    m = re.match(rf"\s*{mac}\(([^)]+)\)", l)
                    if m:
                        pending_aliases.append(m.group(1).strip())
                matched_alias = True
                break
        if matched_alias:
            i += 1
            continue
        # `xitem(...)` — a header-only continuation entry. No body.
        # If it carries a `tt(NAME)` we remember it as the canonical
        # name for the eventual `item(...)` body. Either way, skip it
        # WITHOUT clearing pending state — the body item may be N
        # xitems later, and intermediate xitems with `var(...)` /
        # `SPACES(...)` headers shouldn't blow away pending_canonical.
        if stripped.startswith("xitem("):
            xm = xitem_re.match(l)
            if xm:
                pending_canonical = xm.group(1).strip()
            i += 1
            continue
        if stripped.startswith("xrefdef"):
            i += 1
            continue
        if not stripped or stripped.startswith("sect(")  \
                or stripped.startswith("subsect(")        \
                or stripped.startswith("startitem(")     \
                or stripped.startswith("enditem("):
            # Section boundaries reset pending aliases — a section
            # change without an intervening item means the pindex
            # lines didn't bind to anything.
            if stripped.startswith(("sect(", "subsect(",
                                     "startitem(", "enditem(")):
                pending_aliases = []
            i += 1
            continue
        # `item(tt(NAME)...)(` opens an entry. If the item header
        # doesn't start with `tt(NAME)` (e.g. `item(SPACES()…)(` after
        # a stack of xitem continuations) fall back to:
        #   1. The latest `xitem(tt(NAME))` we saw, then
        #   2. The first pending alias (from findex/pindex/vindex).
        m = name_re.match(l)
        is_plain_item = stripped.startswith("item(")
        if not m and not is_plain_item:
            # NOT clearing pending state here on purpose. Intervening
            # yodl macros (`redef(SPACES)(...)`, `COMMENT(...)`, plain
            # body text outside of a current item, etc.) can sit
            # between an entry's alias declarations and its `item(...)`
            # line — see mod_stat.yo:7-17 where `redef(SPACES)` between
            # the `findex(zstat)` / `findex(stat)` declarations and the
            # `item(tt(stat) ...)` block used to drop the `zstat` alias
            # entirely. Pending state is reset only at the next real
            # section boundary (`sect(`/`subsect(`/`startitem(`/`enditem(`)
            # OR when consumed by an `item(...)`.
            i += 1
            continue
        # If this source requires entries to be vindex / pindex /
        # findex-introduced (the alias macro is the disambiguator
        # against unrelated `item(tt(...))` blocks in the same file
        # — e.g. params.yo also documents subscript flags as `item`s)
        # then skip items with no pending alias declared.
        if cfg.get("require_alias") and not pending_aliases:
            i += 1
            continue
        if m:
            name = m.group(1).strip()
        elif pending_canonical:
            name = pending_canonical
        elif pending_aliases:
            name = pending_aliases[0]
        else:
            i += 1
            continue
        aliases = pending_aliases
        pending_aliases = []
        pending_canonical = None
        # Walk the item-header to find the `)(` body boundary. Skip
        # past the opening `item(` (counted at depth 1).
        item_open = l.find("item(")
        cursor = item_open + len("item(")
        depth = 1
        # Walk through the line(s) of the header until depth==0, then
        # the next `(` opens the body.
        body_start_col = None
        cur_line = i
        while True:
            line_chars = lines[cur_line]
            while cursor < len(line_chars):
                ch = line_chars[cursor]
                if ch == "(":
                    depth += 1
                elif ch == ")":
                    depth -= 1
                    if depth == 0:
                        # Next non-whitespace char should be `(`
                        # opening the body.
                        cursor += 1
                        while cursor < len(line_chars) and line_chars[cursor] in " \t":
                            cursor += 1
                        if cursor < len(line_chars) and line_chars[cursor] == "(":
                            body_start_col = cursor + 1
                            break
                        # No body opens here — entry malformed; skip.
                        break
                cursor += 1
            if body_start_col is not None or depth == 0:
                break
            # Continue onto the next line of the header.
            cur_line += 1
            cursor = 0
            if cur_line >= n:
                break
        if body_start_col is None:
            i += 1
            continue
        # Body: from (cur_line, body_start_col) to the matching `)`.
        body_lines = []
        bdepth = 1
        first = True
        bi = cur_line
        bc = body_start_col
        while bi < n:
            line_chars = lines[bi]
            seg_start = bc if first else 0
            j = seg_start
            done = False
            while j < len(line_chars):
                ch = line_chars[j]
                if ch == "(":
                    bdepth += 1
                elif ch == ")":
                    bdepth -= 1
                    if bdepth == 0:
                        # Body ends at j on this line.
                        body_lines.append(line_chars[seg_start:j])
                        done = True
                        break
                j += 1
            if not done:
                body_lines.append(line_chars[seg_start:])
            else:
                break
            first = False
            bi += 1
        body = "\n".join(body_lines).strip("\n")
        body = strip_yodl(body)
        body = "\n".join(ln.rstrip() for ln in body.split("\n"))
        body = re.sub(r"\n{3,}", "\n\n", body).strip()
        yield (name, aliases, body)
        i = bi + 1


def rust_escape(s: str) -> str:
    """Escape a string for inclusion as a Rust string literal body."""
    return s.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n")


def emit_rust(entries, cfg, source_path):
    table = cfg["table_name"]
    alias_table = cfg["alias_table"]
    lookup_fn = cfg["lookup_fn"]
    label = cfg["kind_label"]
    out = []
    out.append("// AUTO-GENERATED. DO NOT EDIT BY HAND.")
    out.append("//")
    out.append(f"// Source: zsh upstream `{source_path}`.")
    out.append(f"// Regen:  ./scripts/gen_option_docs.py --source {cfg.get('_source_key', '?')} "
               f"{source_path} > <this file>")
    out.append(f"// Generated: {datetime.date.today().isoformat()}")
    out.append("")
    out.append(f"/// Canonical {label} name → markdown documentation body.")
    out.append(f"/// Pulled verbatim from every `item(tt(NAME))(...)` block in")
    out.append(f"/// the upstream yodl source. Use [`{lookup_fn}`] to resolve")
    out.append("/// case-insensitive lookups and any negated/compact aliases.")
    out.append(f"pub const {table}: &[(&str, &str)] = &[")
    for name, _aliases, body in entries:
        out.append(f'    ("{rust_escape(name)}", "{rust_escape(body)}"),')
    out.append("];")
    out.append("")
    if alias_table:
        out.append("/// Alias → canonical mapping. One entry per surface name")
        out.append("/// that should resolve to the same documentation body.")
        out.append(f"pub const {alias_table}: &[(&str, &str)] = &[")
        # Global dedup — first mapping wins. This protects against a
        # later entry's vindex list silently overriding an alias the
        # earlier entry already claimed (e.g. `PROMPT` listed under
        # both the PROMPT-family stack and a related variable's
        # follow-up block).
        global_seen: dict[str, str] = {}
        for name, aliases, _body in entries:
            for a in [name] + aliases:
                if a in global_seen:
                    continue
                global_seen[a] = name
                out.append(f'    ("{rust_escape(a)}", "{rust_escape(name)}"),')
        out.append("];")
        out.append("")
        out.append(f"/// Resolve a {label} name (case-insensitive, alias-aware)")
        out.append("/// to its canonical name + markdown body.")
        out.append(f"pub fn {lookup_fn}(name: &str) -> Option<(&'static str, &'static str)> {{")
        if cfg["uppercase_alias"]:
            out.append("    let key = name.to_ascii_uppercase();")
        else:
            out.append("    let key = name.to_string();")
        out.append(f"    let canon = {alias_table}")
        out.append("        .iter()")
        out.append("        .find(|(a, _)| *a == key.as_str())")
        out.append("        .map(|(_, c)| *c)?;")
        out.append(f"    {table}.iter().find(|(n, _)| *n == canon).map(|&(n, d)| (n, d))")
        out.append("}")
    else:
        out.append(f"/// Resolve a {label} name (exact match) to its")
        out.append("/// canonical name + markdown body.")
        out.append(f"pub fn {lookup_fn}(name: &str) -> Option<(&'static str, &'static str)> {{")
        out.append(f"    {table}.iter().find(|(n, _)| *n == name).map(|&(n, d)| (n, d))")
        out.append("}")
    return "\n".join(out) + "\n"


def main():
    args = sys.argv[1:]
    source_key = "options"
    if args and args[0] == "--source":
        source_key = args[1]
        args = args[2:]
    if len(args) < 1:
        sys.exit(f"usage: {sys.argv[0]} [--source KEY] <path-to-yo> [<more-yo>…]")
    if source_key not in SOURCES:
        sys.exit(f"unknown source {source_key!r}; one of: {', '.join(SOURCES)}")
    cfg = dict(SOURCES[source_key])
    cfg["_source_key"] = source_key
    # Each input file is parsed independently — they're separate
    # documents, so per-file pending state must reset at the
    # boundary or the last `pindex(...)` of file A could bind to the
    # first `item(...)` of file B.
    entries = []
    for p in args:
        src = Path(p).read_text(encoding="utf-8")
        entries.extend(parse_yo(src, cfg))
    if not entries:
        sys.exit(f"no entries parsed; verify the input is a yo file for source {source_key}")
    src_path = args[0] if len(args) == 1 else f"{args[0]} (+{len(args)-1} more)"
    sys.stdout.write(emit_rust(entries, cfg, src_path))


if __name__ == "__main__":
    main()
