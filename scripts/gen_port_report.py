#!/usr/bin/env python3
"""Regenerate docs/port_report.html and refresh the auto-generated slice of docs/report.html.

Walks zsh C sources + Rust port and produces a styled HTML report
with the hud-static / tutorial-app look used by docs/report.html.

The output is also designed to be bot/LLM/scraper friendly:

* A leading <!--PORT-REPORT-SCHEMA--> comment documents every column.
* A <script id="port-report-data" type="application/json"> block
  embeds the entire dataset as JSON so consumers can `grep -A1
  port-report-data` and parse without rendering HTML or running JS.
* Each per-cfile group is wrapped in
  `<!-- BEGIN-GROUP cfile=... -->` / `<!-- END-GROUP cfile=... -->`
  markers, and every symbol row carries a trailing `<!-- SYM ... -->`
  comment with all its columns as `key=value` pairs. This means
  collapsing the JS-driven UI never hides the data from a bot.
"""
from __future__ import annotations
import json
import os, re, html, sys
from datetime import date
from pathlib import Path
from collections import defaultdict

ROOT = Path(__file__).resolve().parent.parent
ZSH_SRC = ROOT / "src" / "zsh" / "Src"
# Only scan the 1:1 port tree. `src/extensions/` (features zsh lacks)
# and `src/recorder/` (recorder feature) are non-ports by design and
# do not belong in the port report. `bins/` is also excluded — it
# contains crate entry points, not C ports.
RS_DIRS = [ROOT / "src" / "ported", ROOT / "parse" / "src"]
OUT = ROOT / "docs" / "port_report.html"

C_KEYWORDS = {
    "if","for","while","switch","return","else","do","sizeof","static",
    "extern","struct","union","enum","typedef","const","volatile","inline",
    "register","auto","goto","break","continue","case","default",
}

# ── C function index ─────────────────────────────────────────────────────────
RE_C_FN = re.compile(r"^([A-Za-z_][A-Za-z0-9_]*)\s*\(")

# C source files that are NOT upstream zsh and must be excluded from
# the port report. Add stems here as they get introduced.
C_EXCLUDE_STEMS = {"zshrs_dump", "main"}

def walk_c() -> dict[str, list[tuple[str,int]]]:
    """name -> [(rel_path, line)]"""
    idx: dict[str, list[tuple[str,int]]] = defaultdict(list)
    roots = [ZSH_SRC]
    for c in sorted(ZSH_SRC.rglob("*.c")):
        if c.stem in C_EXCLUDE_STEMS:
            continue
        rel = c.relative_to(ROOT).as_posix()
        try:
            lines = c.read_text(errors="replace").splitlines()
        except Exception:
            continue
        for i, line in enumerate(lines, 1):
            if not line or line[0].isspace() or line.startswith(("/", "*", "#")):
                continue
            m = RE_C_FN.match(line)
            if not m:
                continue
            name = m.group(1)
            if name in C_KEYWORDS:
                continue
            # require '{' on this line or next non-empty within ~5 lines
            tail = " ".join(lines[i-1:i+6])
            if "{" not in tail:
                continue
            # exclude obvious type defs / casts: must look like fn def, not call.
            # Heuristic: if line ends with ';' before '{' it's a decl/proto.
            if ";" in line and "{" not in line:
                continue
            idx[name].append((rel, i))
    return idx

# ── Body-length helpers (count source lines between { and } for a fn) ────────
# Cache `(rel_path, body_idx)` per file so the same source is parsed once even
# when many fns reference it.
_body_cache_c: dict[str, dict[str, int]] = {}
_body_cache_rs: dict[str, dict[str, int]] = {}

def _skip_lex(src: str, pos: int, is_rust: bool = False) -> int:
    """Skip past one string/char literal or line/block comment starting at pos.
    Returns new pos (unchanged if pos isn't a lex boundary).

    Rust mode: distinguish char literals (`'a'`) from lifetimes
    (`'static`, `'a`). A `'` followed by ident-char + non-`'` is a
    lifetime — leave pos at the `'` and let the caller advance by
    one char so we don't scan-forward looking for a non-existent
    closing quote."""
    if pos >= len(src):
        return pos
    c = src[pos]
    if c == '/' and pos + 1 < len(src):
        if src[pos+1] == '/':
            nl = src.find('\n', pos + 2)
            return nl + 1 if nl != -1 else len(src)
        if src[pos+1] == '*':
            end = src.find('*/', pos + 2)
            return end + 2 if end != -1 else len(src)
    if c == '"':
        pos += 1
        while pos < len(src):
            if src[pos] == '\\': pos += 2; continue
            if src[pos] == '"': return pos + 1
            pos += 1
        return pos
    if c == "'":
        # Rust lifetime detection: `'<ident>` not followed by `'` is
        # a lifetime, not a char literal. Probe the next ~12 chars
        # for a closing `'`; if absent (and the first char is ident),
        # treat as lifetime — pos stays, caller advances by 1.
        if is_rust and pos + 1 < len(src):
            nxt = src[pos + 1]
            if nxt.isalpha() or nxt == '_':
                # Look ahead for closing quote within reasonable
                # char-literal length.
                j = pos + 1
                while j < len(src) and j - pos < 12:
                    if src[j] == '\\': j += 2; continue
                    if src[j] == "'":
                        # Real char literal like `'\n'` or `'a'`.
                        return j + 1
                    j += 1
                # No closing quote in range → lifetime annotation.
                # Leave pos at the `'` so the caller advances 1 char.
                return pos
        pos += 1
        while pos < len(src):
            if src[pos] == '\\': pos += 2; continue
            if src[pos] == "'": return pos + 1
            pos += 1
        return pos
    return pos

def _index_bodies(src: str, fn_re: re.Pattern, is_rust: bool) -> dict[str, int]:
    """Return name -> body line count for every top-level fn defined in src.
    Body line count = source lines between (and excluding) the matching
    `{` and `}` braces, with blank/comment-only lines stripped."""
    bodies: dict[str, int] = {}
    for m in fn_re.finditer(src):
        name = m.group(1)
        # Locate the `(` after the name.
        paren = src.find('(', m.end() - 1)
        if paren == -1:
            continue
        # Skip balanced parens for the arg list.
        pos = paren + 1
        depth = 1
        while pos < len(src) and depth > 0:
            new_pos = _skip_lex(src, pos, is_rust)
            if new_pos != pos:
                pos = new_pos
                continue
            c = src[pos]
            if c == '(': depth += 1
            elif c == ')': depth -= 1
            pos += 1
        # Skip return type / where-clause / attrs until `{` or `;`.
        # For Rust, track `[`/`]` (array types like `[u8; 4]`) and
        # `<`/`>` (generics) bracket depth — `;` inside those is type
        # syntax, not a fn-decl terminator.
        sq_depth = 0
        ang_depth = 0
        while pos < len(src):
            new_pos = _skip_lex(src, pos, is_rust)
            if new_pos != pos:
                pos = new_pos
                continue
            c = src[pos]
            if is_rust:
                if c == '[': sq_depth += 1
                elif c == ']': sq_depth = max(0, sq_depth - 1)
                elif c == '<': ang_depth += 1
                elif c == '>': ang_depth = max(0, ang_depth - 1)
            if sq_depth == 0 and ang_depth == 0:
                if c == '{' or c == ';':
                    break
            pos += 1
        if pos >= len(src) or src[pos] == ';':
            # `;`-terminated decl (extern FFI prototype, fn forward
            # declaration). Don't overwrite a previously-seen real
            # body with 0 — use setdefault so a real `{` body found
            # earlier in the file survives.
            bodies.setdefault(name, 0)
            continue
        # Walk to matching close brace.
        body_start = pos + 1
        depth = 1
        pos = body_start
        while pos < len(src) and depth > 0:
            new_pos = _skip_lex(src, pos, is_rust)
            if new_pos != pos:
                pos = new_pos
                continue
            c = src[pos]
            if c == '{': depth += 1
            elif c == '}': depth -= 1
            pos += 1
        body_lines = src[body_start:max(pos - 1, body_start)].split('\n')
        def _is_block_comment_cont(l: str) -> bool:
            """A line is a block-comment continuation only when it
            starts with `*` followed by a comment-context char
            (space, tab, newline, `/`, another `*`). Avoids
            misfiring on pointer-deref / assignment lines like
            `*p = x;` or `*(char**)y = z;`."""
            s = l.lstrip()
            if not s.startswith('*'):
                return False
            if len(s) == 1:
                return True
            return s[1] in (' ', '\t', '\n', '/', '*')
        actual = [l for l in body_lines
                  if l.strip()
                  and not l.lstrip().startswith('//')
                  and not l.lstrip().startswith('/*')
                  and not _is_block_comment_cont(l)
                  and not l.lstrip().startswith('#')]
        # Same fn name may appear multiple times (e.g. `#[cfg(unix)]` vs
        # `#[cfg(not(unix))]` Rust variants, or `#if defined(...)` C
        # variants) — keep the largest body so a 1-line platform shim
        # doesn't shadow a 100-line real impl.
        bodies[name] = max(bodies.get(name, 0), len(actual))
    return bodies

def c_body_lines(rel_path: str, name: str) -> int:
    """Return body line count for C fn `name` in file `rel_path`."""
    if rel_path not in _body_cache_c:
        full = ROOT / rel_path
        try:
            src = full.read_text(errors="replace")
        except Exception:
            _body_cache_c[rel_path] = {}
            return 0
        _body_cache_c[rel_path] = _index_bodies(src, RE_C_FN_DEF, is_rust=False)
    return _body_cache_c[rel_path].get(name, 0)

def rs_body_lines(rel_path: str, name: str) -> int:
    """Return body line count for Rust fn `name` in file `rel_path`."""
    if rel_path not in _body_cache_rs:
        full = ROOT / rel_path
        try:
            src = full.read_text(errors="replace")
        except Exception:
            _body_cache_rs[rel_path] = {}
            return 0
        _body_cache_rs[rel_path] = _index_bodies(src, RE_RS_FN_DEF, is_rust=True)
    return _body_cache_rs[rel_path].get(name, 0)

# Body-indexer needs a regex that finds fn definitions with their NAME group
# at position 1. RE_C_FN above matches the line-prefix variant; the body
# indexer wants any-position matches, so use a slightly different anchor.
RE_C_FN_DEF = re.compile(r"^([A-Za-z_][A-Za-z0-9_]*)\s*\(", re.MULTILINE)
RE_RS_FN_DEF = re.compile(
    r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?"
    r"(?:extern\s+\"[^\"]+\"\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\b",
    re.MULTILINE,
)

# ── Rust function & port-comment index ───────────────────────────────────────
RE_RS_FN = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?(?:extern\s+\"[^\"]+\"\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\b")
RE_PORT_COMMENT = re.compile(
    r"(?:Port(?:s|ed|ing)?\s+of|Mirrors?|Wrapper\s+for|Equivalent\s+(?:of|to)|Based\s+on|Implements?|Direct\s+port\s+of)"
    r"\s+`?([A-Za-z_][A-Za-z0-9_]*)`?\s*(?:\(\)|\b)",
    re.IGNORECASE,
)
RE_C_TAG = re.compile(r"//\s*[Cc]:\s*([A-Za-z_][A-Za-z0-9_]*)\s*\(")
# Any backticked C-style identifier in a doc comment, e.g. `selectinword`
# or `bin_print()`. Used to extract additional cited names from doc lines
# that read e.g. "Port of the dispatcher behind `selectinword` /
# `selectaword`" — RE_PORT_COMMENT alone would only capture "the" there.
RE_BACKTICK_IDENT = re.compile(r"`([A-Za-z_][A-Za-z0-9_]*)\s*(?:\(\))?`")
# Cited C source paths like `Src/Zle/textobjects.c:41` or
# `Src/builtin.c`. Used to credit Rust files that cite a C file as a
# whole (independent of which fn names the citation mentions).
RE_C_PATH_CITATION = re.compile(r"Src/[A-Za-z0-9_./-]+\.c(?::\d+)?")
# Verbs that mark a doc-comment line as a port citation. We only mine
# RE_BACKTICK_IDENT / RE_C_PATH_CITATION on lines that contain one of
# these — keeps body comments from polluting the index.
RE_PORT_VERB = re.compile(
    r"(?i)\b(port(?:s|ed|ing)?\b|mirror(?:s|ed|ing)?\b|wrapper\b|equivalent\b|based\s+on|implements?\b|direct\s+port\b|behind\b|chunk\s+\d|see\s+`?Src/|from\s+`?Src/)"
)

def walk_rs() -> tuple[dict[str, list[tuple[str,int]]], dict[str, set[str]], dict[str, set[str]]]:
    """
    fn_defs:       rust_name -> [(rel_path, line)]
    port_mentions: c_name    -> {rel_path, ...}  (from doc-comments / // C: tags)
    cfile_cites:   c_rel_path -> {rel_path, ...} (from "Src/foo.c" path citations)
    """
    fn_defs: dict[str, list[tuple[str,int]]] = defaultdict(list)
    port_mentions: dict[str, set[str]] = defaultdict(set)
    cfile_cites: dict[str, set[str]] = defaultdict(set)
    for d in RS_DIRS:
        for f in sorted(d.rglob("*.rs")):
            rel = f.relative_to(ROOT).as_posix()
            try:
                text = f.read_text(errors="replace")
            except Exception:
                continue
            for i, line in enumerate(text.splitlines(), 1):
                m = RE_RS_FN.match(line)
                if m:
                    fn_defs[m.group(1)].append((rel, i))
                ls = line.lstrip()
                is_doc = ls.startswith("///") or ls.startswith("//!") or ls.startswith("/*") or ls.startswith("*") or ls.startswith("//")
                m2 = RE_PORT_COMMENT.search(line)
                if m2 and is_doc:
                    port_mentions[m2.group(1)].add(rel)
                m3 = RE_C_TAG.search(line)
                if m3:
                    port_mentions[m3.group(1)].add(rel)
                # Mine extra cited names + cited C-paths from any doc
                # line that carries a port-citation verb.
                if is_doc and RE_PORT_VERB.search(line):
                    for nm in RE_BACKTICK_IDENT.findall(line):
                        port_mentions[nm].add(rel)
                # A `Src/.../foo.c[:NNN]` reference in any doc-comment
                # line is itself a port citation. Always honour it,
                # independent of port-verb presence — and pick up
                # backticked names on the same line too.
                if is_doc:
                    for path_match in RE_C_PATH_CITATION.findall(line):
                        c_rel = "src/zsh/" + path_match.split(":", 1)[0]
                        cfile_cites[c_rel].add(rel)
                        for nm in RE_BACKTICK_IDENT.findall(line):
                            port_mentions[nm].add(rel)
    return fn_defs, port_mentions, cfile_cites

# ── Expected destination map ─────────────────────────────────────────────────
def expected_for(c_path: str) -> list[str]:
    """zsh/Src/foo.c -> the Rust path. Strict 1:1, byte-for-byte stem.

    No prefix or suffix stripping of any kind — Rust file stems are
    identical to C file stems.
    """
    base = os.path.basename(c_path)
    stem = base[:-2] if base.endswith(".c") else base
    if "/Modules/" in c_path:
        return [f"src/ported/modules/{stem}.rs"]
    if "/Builtins/" in c_path:
        return [f"src/ported/builtins/{stem}.rs"]
    if "/Zle/" in c_path:
        return [f"src/ported/zle/{stem}.rs"]
    # Lexer + parser live in the main runtime crate under src/ported/.
    if stem in ("lex", "parse"):
        return [f"src/ported/{stem}.rs"]
    return [f"src/ported/{stem}.rs"]

# ── Build report ─────────────────────────────────────────────────────────────
GENERIC_NAME_THRESHOLD = 4

def main() -> int:
    print("indexing C sources...", file=sys.stderr)
    c_idx = walk_c()
    print(f"  {len(c_idx)} unique C names", file=sys.stderr)
    print("indexing Rust sources...", file=sys.stderr)
    rs_defs, port_mentions, cfile_cites = walk_rs()
    print(f"  {len(rs_defs)} unique Rust fn names, {len(port_mentions)} port-comment mentions, "
          f"{len(cfile_cites)} cited C paths", file=sys.stderr)

    # Names with 4+ unrelated rust-file definitions are treated as "generic"
    # (e.g. `free`, `init`, `cleanup_`) — only port-comment mentions count.
    generic = {name for name, locs in rs_defs.items()
               if len({Path(p).parent.as_posix() for p,_ in locs}) >= GENERIC_NAME_THRESHOLD}

    # primary C file = first c_idx[name] entry (alphabetical by rel path).
    rows: list[dict] = []
    seen_rust_only_names: set[str] = set()

    for name in sorted(c_idx.keys()):
        c_locs = sorted(c_idx[name])
        primary_c = c_locs[0][0]  # full rel path src/zsh/Src/...
        # short form for filter / display: drop "src/zsh/Src/"
        cf_short = primary_c.replace("src/zsh/Src/", "")
        expected_for_row = expected_for(primary_c)
        rust_locs: list[tuple[str,int]] = []
        if name in generic:
            # Generic name (`setup_`, `boot_`, `cleanup_` etc. — defined
            # in 40+ unrelated module files as the standard module-
            # lifecycle hooks). Restrict the Rust hit to the file that
            # actually corresponds to this C source's expected port
            # destination — otherwise the row drags 40 noisy hits.
            all_defs = rs_defs.get(name, [])
            rust_locs = [loc for loc in all_defs if loc[0] in expected_for_row]
        else:
            rust_locs = list(rs_defs.get(name, []))
        rust_files = {p for p,_ in rust_locs}
        rust_files |= port_mentions.get(name, set())
        expected = expected_for_row

        # Body line counts: pick the primary (first) C and Rust hit.
        c_body = c_body_lines(c_locs[0][0], name) if c_locs else 0
        rs_body = rs_body_lines(rust_locs[0][0], name) if rust_locs else 0
        if rust_files:
            if any(f in expected for f in rust_files):
                placement = "correct" if rust_files <= set(expected) else "split"
            else:
                placement = "misplaced" if expected else "unmapped"
            # Status reflects implementation reality, not just name match:
            #   ported  = real Rust fn whose body is at least 30% of the
            #             C body (or C has no body — name-parity port).
            #   stub    = Rust fn defined but body is empty / comment-
            #             only, OR Rust body < 30% of C body (faithful
            #             ports should not be that small). 30% is the
            #             same threshold gen_port_stubs.py flags as a
            #             stub.
            #   missing = only doc-comment mentions ("Port of foo()")
            #             with no actual Rust fn definition
            # Body-size thresholds mirror gen_port_stubs.py: only flag
            # stub when C body is non-trivial (>= 10 lines). Tiny C
            # fns (1-9 line one-liners) get the benefit of the doubt
            # since an empty Rust body for a tiny C fn is often
            # intentional (drop-cascade no-op, name-parity shim).
            STUB_RATIO_THRESHOLD = 30  # percent
            STUB_MIN_C_BODY = 10  # lines
            if not rust_locs:
                status = "missing"
            elif (c_body >= STUB_MIN_C_BODY
                  and (rs_body * 100 / c_body) < STUB_RATIO_THRESHOLD):
                status = "stub"
            else:
                status = "ported"
        else:
            placement = "—"
            status = "unported"
        rows.append({
            "status": status, "placement": placement, "cfile": cf_short,
            "name": name,
            "c_locs": c_locs, "rust_locs": rust_locs,
            "c_body": c_body, "rust_body": rs_body,
            "rust_pointer_files": sorted(port_mentions.get(name, set()) - {p for p,_ in rust_locs}),
            "expected": expected,
        })

    # Rust-only names (defined in rust but no matching C symbol).
    for name in sorted(rs_defs.keys()):
        if name in c_idx:
            continue
        if name in generic:
            continue
        rust_locs = sorted(rs_defs[name])
        rs_body = rs_body_lines(rust_locs[0][0], name) if rust_locs else 0
        rows.append({
            "status": "rust-only", "placement": "—", "cfile": "(rust-only)",
            "name": name, "c_locs": [], "rust_locs": rust_locs,
            "c_body": 0, "rust_body": rs_body,
            "rust_pointer_files": [], "expected": [],
        })

    # Stats
    total = len(rows)
    n_ported    = sum(1 for r in rows if r["status"]=="ported")
    n_stub      = sum(1 for r in rows if r["status"]=="stub")
    n_missing   = sum(1 for r in rows if r["status"]=="missing")
    n_unported  = sum(1 for r in rows if r["status"]=="unported")
    n_rustonly  = sum(1 for r in rows if r["status"]=="rust-only")
    n_correct   = sum(1 for r in rows if r["placement"]=="correct")
    n_split     = sum(1 for r in rows if r["placement"]=="split")
    n_misplaced = sum(1 for r in rows if r["placement"]=="misplaced")
    n_unmapped  = sum(1 for r in rows if r["placement"]=="unmapped")
    print(f"  rows: {total} (ported={n_ported}, stub={n_stub}, missing={n_missing}, unported={n_unported}, rust-only={n_rustonly})", file=sys.stderr)
    print(f"  placement: correct={n_correct}, split={n_split}, misplaced={n_misplaced}, unmapped={n_unmapped}", file=sys.stderr)

    cfiles = sorted({r["cfile"] for r in rows})

    # ── Per-C-file aggregation ──────────────────────────────────────────────
    # cfile (short) -> {total, ported, unported, rust_files: set, expected: list, c_full: rel_path, c_lines: int}
    by_cfile: dict[str, dict] = {}
    for r in rows:
        if r["cfile"] == "(rust-only)":
            continue
        cf = r["cfile"]
        rec = by_cfile.setdefault(cf, {
            "total": 0, "ported": 0, "unported": 0,
            "rust_files": set(),
            "expected": [],
            "c_full": "src/zsh/Src/" + cf,
        })
        rec["total"] += 1
        if r["status"] == "ported":
            rec["ported"] += 1
        else:
            rec["unported"] += 1
        for p, _ln in r["rust_locs"]:
            rec["rust_files"].add(p)
        rec["rust_files"].update(r["rust_pointer_files"])
        if not rec["expected"]:
            rec["expected"] = r["expected"]

    # Fold in file-level citations (`Src/Zle/textobjects.c:41` mentions
    # in any doc comment) so per-cfile rows reflect every Rust file that
    # ports any part of this C file, not just per-symbol matches.
    for c_rel, rust_set in cfile_cites.items():
        cf_short = c_rel.replace("src/zsh/Src/", "")
        rec = by_cfile.get(cf_short)
        if rec is not None:
            rec["rust_files"].update(rust_set)

    # also pull in C file line counts for display
    for cf, rec in by_cfile.items():
        try:
            rec["c_lines"] = sum(1 for _ in (ROOT / rec["c_full"]).open("rb"))
        except Exception:
            rec["c_lines"] = 0

    # ── HTML ────────────────────────────────────────────────────────────────
    def cell_c(locs: list[tuple[str,int]]) -> str:
        if not locs: return ""
        parts = [f'<a href="../{html.escape(p)}#L{ln}">{html.escape(p.replace("src/zsh/Src/","Src/"))}:{ln}</a>'
                 for p, ln in locs]
        return "<br>".join(parts)
    def cell_rs(locs: list[tuple[str,int]], pointers: list[str]) -> str:
        parts = [f'<a href="../{html.escape(p)}#L{ln}">{html.escape(p)}:{ln}</a>'
                 for p, ln in locs]
        for p in pointers:
            parts.append(f'<a href="../{html.escape(p)}" class="ptr">{html.escape(p)} <span class="ptag">[port-doc]</span></a>')
        return "<br>".join(parts)
    def cell_expected(exp: list[str]) -> str:
        if not exp: return ""
        return '<span class="expected">expected: ' + ", ".join(html.escape(e) for e in exp) + "</span>"

    # ── C↔Rust file map rows ───────────────────────────────────────────────
    file_map_rows = []
    for cf in sorted(by_cfile.keys()):
        rec = by_cfile[cf]
        cov_pct = (rec["ported"] / rec["total"] * 100) if rec["total"] else 0
        if cov_pct >= 95: cov_cls = "ok"
        elif cov_pct >= 50: cov_cls = "mid"
        elif cov_pct > 0: cov_cls = "low"
        else: cov_cls = "none"
        rust_actual = sorted(rec["rust_files"])
        actual_html = "<br>".join(
            f'<a href="../{html.escape(p)}">{html.escape(p)}</a>'
            + (f' <span class="missing">[file missing]</span>' if not (ROOT / p).exists() else '')
            for p in rust_actual
        ) or '<span class="expected">— no rust hits —</span>'
        expected_html = ", ".join(
            f'<code>{html.escape(e)}</code>'
            + (f' <span class="missing">[missing]</span>' if not (ROOT / e).exists() else '')
            for e in rec["expected"]
        ) or '<span class="expected">— no rule —</span>'
        file_map_rows.append(
            f'<tr data-cf="{html.escape(cf)}" class="cov-{cov_cls}">'
            f'<td><a href="../{html.escape(rec["c_full"])}"><code>{html.escape(cf)}</code></a></td>'
            f'<td class="num">{rec["c_lines"]:,}</td>'
            f'<td class="num">{rec["total"]}</td>'
            f'<td class="num ported-num">{rec["ported"]}</td>'
            f'<td class="num unported-num">{rec["unported"]}</td>'
            f'<td class="num cov-pct">{cov_pct:.0f}%</td>'
            f'<td>{expected_html}</td>'
            f'<td>{actual_html}</td>'
            f'</tr>'
        )

    cfile_options = "\n".join(f'<option>{html.escape(c)}</option>' for c in cfiles)

    # Group symbol rows by C file, with a sticky-ish header per group.
    # Sort: real C files alphabetically, then the synthetic "(rust-only)"
    # bucket last; within each group, sort by symbol name.
    def _group_key(r: dict) -> tuple:
        cf = r["cfile"]
        return (1 if cf == "(rust-only)" else 0, cf, r["name"])

    body_rows = []
    last_cf: str | None = None
    for r in sorted(rows, key=_group_key):
        if r["cfile"] != last_cf:
            # Close previous group's marker.
            if last_cf is not None:
                body_rows.append(f'<!-- END-GROUP cfile={last_cf} -->')
            cf = r["cfile"]
            # Count symbols in this group for the header.
            grp = [x for x in rows if x["cfile"] == cf]
            n = len(grp)
            n_p = sum(1 for x in grp if x["status"] == "ported")
            n_u = sum(1 for x in grp if x["status"] == "unported")
            n_r = sum(1 for x in grp if x["status"] == "rust-only")
            body_rows.append(
                f'<!-- BEGIN-GROUP cfile={cf} symbols={n} '
                f'ported={n_p} unported={n_u} rust_only={n_r} -->'
            )
            label = (
                f'<span class="grp-tog">[+]</span> '
                f'<code>{html.escape(cf)}</code> &mdash; {n} symbol{"s" if n != 1 else ""}'
                f' &middot; <span style="color:var(--green)">{n_p} ported</span>'
                + (f' &middot; <span style="color:#ff6b6b">{n_u} unported</span>' if n_u else "")
                + (f' &middot; <span style="color:#ffb800">{n_r} rust-only</span>' if n_r else "")
            )
            body_rows.append(
                f'<tr class="grp-row" data-grp="{html.escape(cf)}" onclick="tg(this)">'
                f'<td colspan="7" class="grp-cell">{label}</td>'
                f'</tr>'
            )
            last_cf = cf
        cls = f'st-{r["status"]} pl-{r["placement"]} grp-child'
        # Bot-friendly inline summary: every column as key=value, plus
        # the first C location and the first Rust hit (when present).
        c_first = (r["c_locs"][0][0] + ":" + str(r["c_locs"][0][1])) if r["c_locs"] else ""
        rust_paths = sorted({p for p, _ in r["rust_locs"]} | set(r["rust_pointer_files"]))
        sym_comment = (
            f'<!-- SYM name={r["name"]} status={r["status"]} '
            f'placement={r["placement"]} cfile={r["cfile"]} '
            f'c_loc={c_first} '
            f'rust={"|".join(rust_paths) if rust_paths else "-"} '
            f'expected={"|".join(r["expected"]) if r["expected"] else "-"} -->'
        )
        body_rows.append(sym_comment)
        body_rows.append(
            f'<tr class="{cls}" '
            f'style="display:none" '
            f'data-name="{html.escape(r["name"])}" '
            f'data-status="{r["status"]}" '
            f'data-placement="{r["placement"]}" '
            f'data-file="{html.escape(r["cfile"])}">'
            f'<td class="status">{r["status"]}</td>'
            f'<td class="placement">{r["placement"]}</td>'
            f'<td><code>{html.escape(r["cfile"])}</code></td>'
            f'<td><b>{html.escape(r["name"])}</b></td>'
            f'<td>{cell_c(r["c_locs"])}</td>'
            f'<td>{cell_rs(r["rust_locs"], r["rust_pointer_files"])}</td>'
            f'<td>{cell_expected(r["expected"])}</td>'
            f'</tr>'
        )
    if last_cf is not None:
        body_rows.append(f'<!-- END-GROUP cfile={last_cf} -->')

    # ── Per-fn line-count table (filterable + sortable) ───────────────────
    # Flat row per (C-fn, primary Rust hit) pair. Lets you grep
    # "which C fns have a big Rust body?" or "which short C fns are
    # bloated in Rust?" or "show me everything in glob.c sorted by C
    # body desc". Includes rust-only rows too (cfile = "(rust-only)").
    # Three tables: lc_rows = real C↔Rust pairs, ro_rows = rust-only fns,
    # ex_rows = exec.c (tree-walker; replaced by fusevm bytecode, NOT a
    # 1:1 port target — segregate so it doesn't drag the lc gradient).
    lc_rows: list[str] = []
    ro_rows: list[str] = []
    ex_rows: list[str] = []
    # Sort by ratio ascending so the worst porting gaps surface at the top.
    # Rust-only rows have no ratio — push them to the bottom (sort key
    # below pins them to +inf). Within the same ratio, fall back to
    # (cfile, name) for stable order.
    def _row_sort_key(r: dict) -> tuple:
        if r["cfile"] == "(rust-only)":
            return (2, 0, r["cfile"], r["name"])  # rust-only last
        cb = r.get("c_body", 0)
        rb = r.get("rust_body", 0)
        if cb > 0:
            ratio = round(rb / cb * 100)
        elif rb > 0:
            ratio = 100
        else:
            ratio = 100
        return (0, ratio, r["cfile"], r["name"])
    for r in sorted(rows, key=_row_sort_key):
        c_first = r["c_locs"][0] if r["c_locs"] else ("", 0)
        rs_first = r["rust_locs"][0] if r["rust_locs"] else ("", 0)
        c_file_short = c_first[0].replace("src/zsh/Src/", "") if c_first[0] else ""
        c_line = c_first[1]
        rs_file = rs_first[0]
        rs_line = rs_first[1]
        c_body = r.get("c_body", 0)
        r_body = r.get("rust_body", 0)
        # Ratio (Rust/C) as integer pct. Three special cases:
        #   - C=0, Rust=0:   intentionally empty on both sides (no-op
        #                    fn like `nohw`, `noop_function`). Treat as
        #                    100% match (green) — there's nothing to
        #                    port.
        #   - C=0, Rust>0:   C decl-only / macro shell with a real Rust
        #                    body. Mark as 100% (green) — Rust does
        #                    more than C surface implies.
        #   - C>0, Rust=0:   TRUE porting gap. Ratio=0% (red).
        #   - both >0:       normal Rust/C pct.
        if c_body > 0:
            ratio = round(r_body / c_body * 100)
        elif r_body > 0:
            ratio = 100  # Rust does work where C is decl-only
        else:
            ratio = 100  # both empty: nothing to port, matched
        # Row tint: red (low ratio) → yellow (~50%) → green (≥100%).
        # Hue 0=red, 60=yellow, 120=green. Cap ratio at 100 so very-large
        # Rust ports still show pure green instead of rotating past green.
        hue = max(0, min(int(ratio), 100)) * 1.2  # 0..120
        row_style = f"background:hsl({hue:.0f},45%,11%);"
        c_cell = (
            f'<a href="../{html.escape(c_first[0])}#L{c_line}">'
            f'{html.escape(c_file_short)}:{c_line}</a>'
            if c_first[0] else '<span class="expected">—</span>'
        )
        rs_cell = (
            f'<a href="../{html.escape(rs_file)}#L{rs_line}">'
            f'{html.escape(rs_file)}:{rs_line}</a>'
            if rs_file else '<span class="expected">—</span>'
        )
        # Rust-only rows go to a separate table — slimmer schema since
        # the C columns / ratio are meaningless for them.
        if r["cfile"] == "(rust-only)":
            ro_rows.append(
                f'<tr class="ro-row" '
                f'data-name="{html.escape(r["name"])}" '
                f'data-rbody="{r_body}" data-rline="{rs_line}">'
                f'<td><b>{html.escape(r["name"])}</b></td>'
                f'<td>{rs_cell}</td>'
                f'<td class="num">{r_body or ""}</td>'
                f'</tr>'
            )
            continue
        # exec.c rows go to their own table — the C tree-walker is
        # replaced by fusevm bytecode, so per-fn ratios there are
        # noise (a `walk_*` fn with 200 lines of C and no Rust hit
        # is intentional, not a stub).
        row_cls = "ex-row" if r["cfile"] == "exec.c" else "lc-row"
        target_rows = ex_rows if r["cfile"] == "exec.c" else lc_rows
        target_rows.append(
            f'<tr class="{row_cls}" '
            f'style="{row_style}" '
            f'data-name="{html.escape(r["name"])}" '
            f'data-cfile="{html.escape(r["cfile"])}" '
            f'data-status="{r["status"]}" '
            f'data-cbody="{c_body}" data-rbody="{r_body}" '
            f'data-cline="{c_line}" data-rline="{rs_line}" '
            f'data-ratio="{ratio}">'
            f'<td><b>{html.escape(r["name"])}</b></td>'
            f'<td>{c_cell}</td>'
            f'<td class="num">{c_body}</td>'
            f'<td>{rs_cell}</td>'
            f'<td class="num">{r_body}</td>'
            f'<td class="num">{ratio}%</td>'
            f'<td class="status">{r["status"]}</td>'
            f'</tr>'
        )

    # ── Bot-friendly JSON dataset + schema comment ────────────────────────
    schema_doc = """\
<!--PORT-REPORT-SCHEMA
This file is the definitive C->Rust port mapping for zshrs.

Machine-readable surfaces (use these, not the rendered HTML):

1. JSON dataset embedded in a script tag with id "port-report-data"
   and type "application/json". Schema:
     {
       "generated": ISO-8601 timestamp,
       "stats": { total, ported, unported, rust_only, correct,
                  split, misplaced, unmapped },
       "files": [                             # one entry per C file
         { "cfile":   "Builtins/rlimits.c",
           "c_full":  "src/zsh/Src/Builtins/rlimits.c",
           "c_lines": 924,
           "total":   19,
           "ported":  15,
           "unported": 4,
           "coverage_pct": 79,
           "expected_rust": ["src/ported/builtins/rlimits.rs"],
           "rust_files":   ["src/ported/builtins/rlimits.rs", ...] }
       ],
       "symbols": [                           # one entry per C function
         { "name":      "bin_limit",
           "status":    "ported"|"unported"|"rust-only",
           "placement": "correct"|"split"|"misplaced"|"unmapped"|"-",
           "cfile":     "Builtins/rlimits.c",  # "(rust-only)" when no C
           "c_locs":    [["src/zsh/Src/Builtins/rlimits.c", 519], ...],
           "rust_locs": [["src/ported/builtins/rlimits.rs", 454], ...],
           "rust_pointer_files": ["..."],   # rust files that cite via doc-comment but don't define
           "expected":  ["src/ported/builtins/rlimits.rs"] }
       ]
     }

Excluded from this report by design:
* src/extensions/ — features zsh C does NOT have
* src/recorder/  — feature-gated recorder
* src/zsh/Src/main.c, Src/Modules/zshrs_dump.c — non-port files
-->
"""
    import datetime
    dataset = {
        "generated": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "stats": {
            "total": total,
            "ported": n_ported,
            "stub": n_stub,
            "missing": n_missing,
            "unported": n_unported,
            "rust_only": n_rustonly,
            "correct": n_correct,
            "split": n_split,
            "misplaced": n_misplaced,
            "unmapped": n_unmapped,
        },
        "files": [
            {
                "cfile": cf,
                "c_full": rec["c_full"],
                "c_lines": rec["c_lines"],
                "total": rec["total"],
                "ported": rec["ported"],
                "unported": rec["unported"],
                "coverage_pct": round(
                    (rec["ported"] / rec["total"] * 100) if rec["total"] else 0, 1
                ),
                "expected_rust": list(rec["expected"]),
                "rust_files": sorted(rec["rust_files"]),
            }
            for cf, rec in sorted(by_cfile.items())
        ],
        "symbols": [
            {
                "name": r["name"],
                "status": r["status"],
                "placement": r["placement"],
                "cfile": r["cfile"],
                "c_locs": r["c_locs"],
                "rust_locs": r["rust_locs"],
                "c_body": r.get("c_body", 0),
                "rust_body": r.get("rust_body", 0),
                "rust_pointer_files": list(r["rust_pointer_files"]),
                "expected": list(r["expected"]),
            }
            for r in rows
        ],
    }
    # `</script>` inside the JSON would terminate the tag prematurely.
    json_blob = json.dumps(dataset, separators=(",", ":")).replace("</", "<\\/")
    data_script = (
        '<script id="port-report-data" type="application/json">\n'
        + json_blob
        + "\n</script>"
    )

    html_doc = f"""<!DOCTYPE html>
{schema_doc}<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="dark light">
<title>zshrs — Port Report (per-function)</title>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Orbitron:wght@400;600;700;900&family=Share+Tech+Mono&display=swap" rel="stylesheet">
<link rel="stylesheet" href="hud-static.css">
<link rel="stylesheet" href="tutorial.css">
<style>
  .tutorial-main {{ max-width: 96rem; }}
  .stat-grid {{ display:grid;grid-template-columns:repeat(auto-fill,minmax(11rem,1fr));gap:0.6rem;margin:1rem 0; }}
  .stat-card {{ border:1px solid var(--border);border-top:3px solid var(--cyan);background:var(--bg-card);padding:0.7rem 0.9rem;border-radius:2px;text-align:center; }}
  .stat-card .stat-val {{ font-family:'Orbitron',sans-serif;font-size:22px;font-weight:900;color:var(--cyan);line-height:1.1;text-shadow:0 0 14px var(--cyan-glow); }}
  .stat-card .stat-val.green   {{ color:var(--green); text-shadow:0 0 14px rgba(57,255,20,.35); }}
  .stat-card .stat-val.red     {{ color:#ff6b6b; text-shadow:0 0 14px rgba(255,107,107,.35); }}
  .stat-card .stat-val.yellow  {{ color:#ffb800; text-shadow:0 0 14px rgba(255,184,0,.35); }}
  .stat-card .stat-val.magenta {{ color:#d300c5; text-shadow:0 0 14px rgba(211,0,197,.35); }}
  .stat-card .stat-val.gray    {{ color:#8b949e; }}
  .stat-card .stat-label {{ font-family:'Orbitron',sans-serif;font-size:9px;font-weight:700;letter-spacing:1.5px;text-transform:uppercase;color:var(--text-muted);margin-top:0.4rem; }}

  .filter-bar {{ display:flex;flex-wrap:wrap;gap:0.5rem;margin:0.8rem 0;align-items:center; }}
  .filter-bar input, .filter-bar select {{
    background:var(--bg-primary);color:var(--text);
    border:1px solid var(--border);border-radius:2px;
    padding:6px 10px;font-family:'Share Tech Mono',monospace;font-size:12px;
  }}
  .filter-bar input:focus, .filter-bar select:focus {{ outline:none;border-color:var(--cyan);box-shadow:0 0 6px var(--cyan-glow); }}
  .filter-bar label {{ font-family:'Orbitron',sans-serif;font-size:10px;letter-spacing:1.2px;text-transform:uppercase;color:var(--text-muted); }}

  table.fn-table {{ width:100%;border-collapse:collapse;font-size:11.5px;margin:0.8rem 0; }}
  table.fn-table th {{ background:var(--bg-secondary);color:var(--cyan);font-family:'Orbitron',sans-serif;font-size:10px;font-weight:700;letter-spacing:1.2px;text-transform:uppercase;text-align:left;padding:7px 10px;border:1px solid var(--border);position:sticky;top:0;z-index:5; }}
  table.fn-table td {{ padding:6px 10px;border:1px solid var(--border);color:var(--text-dim);vertical-align:top; }}
  table.fn-table tr:hover td {{ background:var(--bg-hover); }}
  table.fn-table code {{ font-size:11px;color:var(--accent-light);background:var(--bg-primary);padding:1px 4px;border-radius:2px; }}
  table.fn-table a {{ color:var(--cyan);text-decoration:none; }}
  table.fn-table a:hover {{ text-decoration:underline;color:#fff; }}
  table.fn-table .ptr {{ color:#8b949e;font-style:italic; }}
  table.fn-table .ptag {{ font-size:9px;color:#d300c5;letter-spacing:0.8px; }}

  tr.grp-row td.grp-cell {{
    background:var(--bg-secondary);
    color:var(--cyan);
    font-family:'Orbitron',sans-serif;
    font-size:11px;font-weight:700;
    letter-spacing:1.2px;
    padding:8px 10px;
    border-top:2px solid var(--cyan);
    position:sticky;top:0;z-index:4;
    cursor:pointer;
    user-select:none;
  }}
  tr.grp-row.open td.grp-cell {{ background:var(--bg-hover); }}
  tr.grp-row td.grp-cell:hover {{ background:var(--bg-hover); }}
  tr.grp-row td.grp-cell .grp-tog {{
    display:inline-block;width:1.2rem;color:var(--cyan);
    font-family:'Share Tech Mono',monospace;font-weight:700;
  }}
  tr.grp-row td.grp-cell code {{
    font-family:'Share Tech Mono',monospace;
    font-size:11.5px;color:var(--accent-light);
    background:var(--bg-primary);padding:1px 6px;
  }}

  tr.st-ported    td.status {{ color:var(--green);font-weight:700; }}
  tr.st-stub      td.status {{ color:#ff8c66;font-weight:700; }}
  tr.st-missing   td.status {{ color:#d300c5;font-weight:700; }}
  tr.st-unported  td.status {{ color:#ff6b6b;font-weight:700; }}
  tr.st-rust-only td.status {{ color:#ffb800;font-weight:700; }}
  tr.pl-correct   td.placement {{ color:var(--green); }}
  tr.pl-split     td.placement {{ color:#ffb800; }}
  tr.pl-misplaced td.placement {{ color:#ff6b6b;font-weight:700; }}
  tr.pl-unmapped  td.placement {{ color:#8b949e;font-style:italic; }}

  .expected {{ color:#8b949e;font-style:italic;font-size:10.5px; }}
  .missing  {{ color:#ff6b6b;font-size:10px;letter-spacing:0.6px; }}

  table.fn-table td.num {{ text-align:right;font-family:'Share Tech Mono',monospace; }}
  table.lc-table th {{ cursor:pointer;user-select:none; }}
  table.lc-table th:hover {{ background:var(--bg-hover);color:#fff; }}
  table.lc-table th.sort-asc::after {{ content:" ▲";color:var(--cyan);font-size:9px; }}
  table.lc-table th.sort-desc::after {{ content:" ▼";color:var(--cyan);font-size:9px; }}
  /* Ratio-gradient rows: row tint is set inline via hsl(); keep the
     gradient visible on hover by overriding the generic .fn-table
     tr:hover background, and brighten the lightness instead. */
  table.lc-table tbody tr.lc-row:hover,
  table.lc-table tbody tr.ex-row:hover {{ filter:brightness(1.4); }}
  table.lc-table tbody tr.lc-row:hover td,
  table.lc-table tbody tr.ex-row:hover td {{ background:transparent; }}
  table.lc-table tbody tr.lc-row td,
  table.lc-table tbody tr.ex-row td {{ background:transparent; }}
  /* RUST-ONLY table: same sortable headers as lc-table, no gradient. */
  table.ro-table th {{ cursor:pointer;user-select:none; }}
  table.ro-table th:hover {{ background:var(--bg-hover);color:#fff; }}
  table.ro-table th.sort-asc::after {{ content:" ▲";color:var(--cyan);font-size:9px; }}
  table.ro-table th.sort-desc::after {{ content:" ▼";color:var(--cyan);font-size:9px; }}
  table.file-map td.num {{ text-align:right;font-family:'Share Tech Mono',monospace; }}
  table.file-map td.ported-num   {{ color:var(--green); }}
  table.file-map td.unported-num {{ color:#ff6b6b; }}
  table.file-map td.cov-pct      {{ font-family:'Orbitron',sans-serif;font-weight:700; }}
  table.file-map tr.cov-ok   td.cov-pct {{ color:var(--green); }}
  table.file-map tr.cov-mid  td.cov-pct {{ color:#ffb800; }}
  table.file-map tr.cov-low  td.cov-pct {{ color:#ff8c66; }}
  table.file-map tr.cov-none td.cov-pct {{ color:#ff6b6b; }}
  table.file-map tr.cov-ok   td:first-child {{ border-left:3px solid var(--green); }}
  table.file-map tr.cov-mid  td:first-child {{ border-left:3px solid #ffb800; }}
  table.file-map tr.cov-low  td:first-child {{ border-left:3px solid #ff8c66; }}
  table.file-map tr.cov-none td:first-child {{ border-left:3px solid #ff6b6b; }}
  .legend {{ font-family:'Share Tech Mono',monospace;font-size:11px;color:var(--text-dim);margin:0.6rem 0;line-height:1.7; }}
  .legend b {{ color:var(--cyan);font-family:'Orbitron',sans-serif;font-size:10px;letter-spacing:1px; }}
</style>
</head>
<body>
{data_script}
<div class="app tutorial-app">
  <div class="crt-scanline" aria-hidden="true"></div>
  <div class="crt-scanline-v" aria-hidden="true"></div>

  <header class="tutorial-header">
    <div class="tutorial-header-inner">
      <div>
        <h1 class="tutorial-brand">// ZSHRS &mdash; PER-FUNCTION PORT REPORT</h1>
        <nav class="tutorial-crumbs" aria-label="Breadcrumb">
          <span class="current">Port Report</span>
          <span class="sep">/</span>
          <a href="index.html">zshrs Docs</a>
          <span class="sep">/</span>
          <a href="report.html">Coverage Report</a>
          <span class="sep">/</span>
          <a href="https://github.com/MenkeTechnologies/zshrs" target="_blank" rel="noopener noreferrer">GitHub</a>
        </nav>
        <p style="margin:0.35rem 0 0;font-family:'Share Tech Mono',monospace;font-size:11px;color:var(--text-dim);letter-spacing:0.03em;opacity:0.8;">
          Per-symbol map of every C function in <code>src/zsh/Src/**/*.c</code> against its Rust counterpart in
          <code>src/ported/**/*.rs</code>. Detects missing ports, misplaced ports, and rust-only helpers.
          Non-port code under <code>src/extensions/</code> and <code>src/recorder/</code> is intentionally excluded.
        </p>
      </div>
    </div>
  </header>

  <main class="tutorial-main">

    <h2 class="tutorial-title"><span class="step-hash">&gt;_</span>SUMMARY</h2>
    <div class="stat-grid">
      <div class="stat-card"><div class="stat-val">{total:,}</div><div class="stat-label">Total Symbols</div></div>
      <div class="stat-card"><div class="stat-val green">{n_ported:,}</div><div class="stat-label">Ported</div></div>
      <div class="stat-card"><div class="stat-val" style="color:#ff8c66;text-shadow:0 0 14px rgba(255,140,102,.35);">{n_stub:,}</div><div class="stat-label">Stub (Rust empty)</div></div>
      <div class="stat-card"><div class="stat-val magenta">{n_missing:,}</div><div class="stat-label">Missing (cite only)</div></div>
      <div class="stat-card"><div class="stat-val red">{n_unported:,}</div><div class="stat-label">Unported (C-only)</div></div>
      <div class="stat-card"><div class="stat-val yellow">{n_rustonly:,}</div><div class="stat-label">Rust-only</div></div>
      <div class="stat-card"><div class="stat-val green">{n_correct:,}</div><div class="stat-label">Placement: Correct</div></div>
      <div class="stat-card"><div class="stat-val yellow">{n_split:,}</div><div class="stat-label">Placement: Split</div></div>
      <div class="stat-card"><div class="stat-val red">{n_misplaced:,}</div><div class="stat-label">Placement: Misplaced</div></div>
      <div class="stat-card"><div class="stat-val gray">{n_unmapped:,}</div><div class="stat-label">Placement: Unmapped</div></div>
    </div>

    <p class="legend">
      <b>STATUS</b> <span style="color:var(--green)">ported</span> = real Rust fn defined with a real body &middot;
      <span style="color:#ff8c66">stub</span> = Rust fn defined but body is empty / comment-only while C has a real body &middot;
      <span style="color:#d300c5">missing</span> = only doc-comment "Port of foo()" mention; no Rust fn defined &middot;
      <span style="color:#ff6b6b">unported</span> = C symbol with no Rust references at all &middot;
      <span style="color:#ffb800">rust-only</span> = Rust fn with no matching C symbol.<br>
      <b>PLACEMENT</b> <span style="color:var(--green)">correct</span> = lives only in expected file(s) &middot;
      <span style="color:#ffb800">split</span> = lives in expected file plus extras &middot;
      <span style="color:#ff6b6b">misplaced</span> = expected destination exists, none of the rust hits land there &middot;
      <span style="color:#8b949e">unmapped</span> = no expected destination rule for this C source.
    </p>

    <h2 class="tutorial-title"><span class="step-hash">&gt;_</span>FILE MAP &mdash; C &harr; RUST</h2>
    <p class="legend">
      Each upstream <code>zsh/Src/**/*.c</code> file paired with its expected destination
      and the set of Rust files where its symbols actually ended up. Coverage % = ported /
      total fns defined in that C file.
    </p>
    <div class="filter-bar">
      <label for="qf">filter:</label><input id="qf" placeholder="C file name…" oninput="ff()" size="22">
      <label for="cv">coverage:</label>
      <select id="cv" onchange="ff()">
        <option value="">all</option>
        <option value="ok">≥95%</option>
        <option value="mid">50&ndash;94%</option>
        <option value="low">1&ndash;49%</option>
        <option value="none">0%</option>
      </select>
    </div>
    <table class="fn-table file-map">
      <thead><tr>
        <th>C file</th><th>C lines</th><th>fns</th><th>ported</th><th>unported</th><th>coverage</th>
        <th>expected Rust destination</th><th>actual Rust files</th>
      </tr></thead>
      <tbody>
{chr(10).join(file_map_rows)}
      </tbody>
    </table>

    <h2 class="tutorial-title"><span class="step-hash">&gt;_</span>LINE COUNTS &mdash; PER FN</h2>
    <p class="legend">
      Every C function in <code>src/zsh/Src/**/*.c</code> with its primary
      Rust counterpart. Line counts are non-blank/non-comment body lines
      between the matching <code>&#123;</code>/<code>&#125;</code>.
      Click any column header to sort &middot; filter by name / file /
      status. Ratio = Rust body / C body, useful for spotting unported
      bodies (low %) or Rust-idiom replacements (low % with a marker
      comment).
    </p>
    <div class="filter-bar">
      <label for="lcq">filter:</label><input id="lcq" placeholder="fn name or C file…" oninput="lcf()" size="28">
      <label for="lcst">status:</label>
      <select id="lcst" onchange="lcf()">
        <option value="">all</option><option>ported</option><option>stub</option><option>missing</option><option>unported</option>
      </select>
      <label for="lcrat">ratio:</label>
      <select id="lcrat" onchange="lcf()">
        <option value="">all</option>
        <option value="lt10">&lt;10%</option>
        <option value="lt30">&lt;30%</option>
        <option value="ge100">≥100%</option>
        <option value="empty">porting gap (C&gt;0, Rust=0)</option>
      </select>
      <span id="lcct" class="legend" style="margin-left:auto;font-size:10px;"></span>
    </div>
    <table class="fn-table lc-table">
      <thead><tr>
        <th data-sort="name"      onclick="lcs('name')">fn name</th>
        <th data-sort="cline"     onclick="lcs('cline')">C file:line</th>
        <th data-sort="cbody"     onclick="lcs('cbody')">C lines</th>
        <th data-sort="rline"     onclick="lcs('rline')">Rust file:line</th>
        <th data-sort="rbody"     onclick="lcs('rbody')">Rust lines</th>
        <th data-sort="ratio"     class="sort-asc" onclick="lcs('ratio')">ratio</th>
        <th data-sort="status"    onclick="lcs('status')">status</th>
      </tr></thead>
      <tbody id="lc-tbody">
{chr(10).join(lc_rows)}
      </tbody>
    </table>

    <h2 class="tutorial-title"><span class="step-hash">&gt;_</span>EXEC.C (TREE-WALKER &mdash; SEGREGATED)</h2>
    <p class="legend">
      <code>exec.c</code> implements the C tree-walker interpreter
      (<code>execlist</code>/<code>execpline</code>/<code>execcmd</code>
      etc.). zshrs replaces the entire walker with <code>fusevm</code>
      bytecode compilation, so per-fn ratios here are noise — a 200-line
      <code>walk_*</code> with no Rust counterpart is intentional, not a
      stub. Listed for visibility only.
    </p>
    <div class="filter-bar">
      <label for="exq">filter:</label><input id="exq" placeholder="fn name…" oninput="exf()" size="22">
      <span id="exct" class="legend" style="margin-left:auto;font-size:10px;"></span>
    </div>
    <table class="fn-table lc-table">
      <thead><tr>
        <th data-sort="name"   onclick="exs('name')">fn name</th>
        <th data-sort="cline"  onclick="exs('cline')">C file:line</th>
        <th data-sort="cbody"  onclick="exs('cbody')">C lines</th>
        <th data-sort="rline"  onclick="exs('rline')">Rust file:line</th>
        <th data-sort="rbody"  onclick="exs('rbody')">Rust lines</th>
        <th data-sort="ratio"  onclick="exs('ratio')">ratio</th>
        <th data-sort="status" onclick="exs('status')">status</th>
      </tr></thead>
      <tbody id="ex-tbody">
{chr(10).join(ex_rows)}
      </tbody>
    </table>

    <h2 class="tutorial-title"><span class="step-hash">&gt;_</span>RUST-ONLY FNS</h2>
    <p class="legend">
      Rust functions with no matching C symbol — feature-flag helpers,
      test fns, extension code, AOP hooks. Not part of the 1:1 port,
      kept here for visibility.
    </p>
    <div class="filter-bar">
      <label for="roq">filter:</label><input id="roq" placeholder="fn name…" oninput="rof()" size="22">
      <span id="roct" class="legend" style="margin-left:auto;font-size:10px;"></span>
    </div>
    <table class="fn-table ro-table">
      <thead><tr>
        <th data-sort="name"  onclick="ros('name')">fn name</th>
        <th data-sort="rline" onclick="ros('rline')">Rust file:line</th>
        <th data-sort="rbody" onclick="ros('rbody')">Rust lines</th>
      </tr></thead>
      <tbody id="ro-tbody">
{chr(10).join(ro_rows)}
      </tbody>
    </table>

    <h2 class="tutorial-title"><span class="step-hash">&gt;_</span>SYMBOLS</h2>
    <div class="filter-bar">
      <label for="q">filter:</label><input id="q" placeholder="name…" oninput="f()" size="22">
      <label for="st">status:</label>
      <select id="st" onchange="f()">
        <option value="">all</option><option>ported</option><option>stub</option><option>missing</option><option>unported</option><option>rust-only</option>
      </select>
      <label for="pl">placement:</label>
      <select id="pl" onchange="f()">
        <option value="">all</option><option>correct</option><option>split</option><option>misplaced</option><option>unmapped</option>
      </select>
      <label for="cf">C file:</label>
      <select id="cf" onchange="f()">
        <option value="">all</option>
{cfile_options}
      </select>
    </div>

    <table class="fn-table">
      <thead><tr>
        <th>status</th><th>placement</th><th>primary C file</th><th>fn name</th>
        <th>C definition(s)</th><th>Rust counterpart(s)</th><th>expected destination</th>
      </tr></thead>
      <tbody>
{chr(10).join(body_rows)}
      </tbody>
    </table>

  </main>
</div>

<script>
function tg(gr){{
  const open = gr.classList.toggle('open');
  const tog = gr.querySelector('.grp-tog');
  if (tog) tog.textContent = open ? '[-]' : '[+]';
  const grp = gr.dataset.grp;
  document.querySelectorAll(
    'table.fn-table:not(.file-map) tbody tr.grp-child'
  ).forEach(tr => {{
    if (tr.dataset.file === grp && !tr.dataset.filteredOut) {{
      tr.style.display = open ? '' : 'none';
    }}
  }});
}}
function f(){{
  const q = document.getElementById('q').value.toLowerCase();
  const s = document.getElementById('st').value;
  const p = document.getElementById('pl').value;
  const cf = document.getElementById('cf').value;
  const filtersActive = !!(q || s || p || cf);
  // First pass: data rows.
  document.querySelectorAll('table.fn-table:not(.file-map) tbody tr.grp-child').forEach(tr => {{
    const mq = !q  || tr.dataset.name.toLowerCase().includes(q);
    const ms = !s  || tr.dataset.status === s;
    const mp = !p  || tr.dataset.placement === p;
    const mfl = !cf || tr.dataset.file === cf;
    const matches = mq && ms && mp && mfl;
    if (!matches) {{
      tr.dataset.filteredOut = '1';
      tr.style.display = 'none';
    }} else {{
      delete tr.dataset.filteredOut;
      // Force-visible when any filter is active; otherwise honour the
      // group's open/collapsed state.
      const grpRow = document.querySelector(
        'table.fn-table:not(.file-map) tbody tr.grp-row[data-grp="' +
        CSS.escape(tr.dataset.file) + '"]'
      );
      const grpOpen = grpRow && grpRow.classList.contains('open');
      tr.style.display = (filtersActive || grpOpen) ? '' : 'none';
    }}
  }});
  // Second pass: hide group headers whose data rows are all filtered.
  document.querySelectorAll('table.fn-table:not(.file-map) tbody tr.grp-row').forEach(gr => {{
    const grp = gr.dataset.grp;
    const anyMatch = Array.from(
      document.querySelectorAll('table.fn-table:not(.file-map) tbody tr.grp-child')
    ).some(tr => tr.dataset.file === grp && !tr.dataset.filteredOut);
    gr.style.display = anyMatch ? '' : 'none';
  }});
}}
function ff(){{
  const q  = document.getElementById('qf').value.toLowerCase();
  const cv = document.getElementById('cv').value;
  document.querySelectorAll('table.file-map tbody tr').forEach(tr => {{
    const mq  = !q  || tr.dataset.cf.toLowerCase().includes(q);
    const mcv = !cv || tr.classList.contains('cov-' + cv);
    tr.style.display = (mq && mcv) ? '' : 'none';
  }});
}}
// ── LINE COUNTS table: filter + sort ─────────────────────────────────────
function lcf(){{
  const q  = document.getElementById('lcq').value.toLowerCase();
  const st = document.getElementById('lcst').value;
  const rt = document.getElementById('lcrat').value;
  let shown = 0;
  document.querySelectorAll('#lc-tbody tr.lc-row').forEach(tr => {{
    const mq  = !q || tr.dataset.name.toLowerCase().includes(q)
                   || tr.dataset.cfile.toLowerCase().includes(q);
    const ms  = !st || tr.dataset.status === st;
    const ratio = parseInt(tr.dataset.ratio, 10);
    let mr = true;
    // ratio is always 0..100+ now (empty-bodied rows pin at 100).
    // "no rust" band = real porting gaps: C body > 0 + Rust body = 0.
    const cb = parseInt(tr.dataset.cbody, 10);
    const rb = parseInt(tr.dataset.rbody, 10);
    if (rt === 'lt10')   mr = ratio < 10;
    else if (rt === 'lt30')  mr = ratio < 30;
    else if (rt === 'ge100') mr = ratio >= 100;
    else if (rt === 'empty') mr = cb > 0 && rb === 0;
    const ok = mq && ms && mr;
    tr.style.display = ok ? '' : 'none';
    if (ok) shown++;
  }});
  const ct = document.getElementById('lcct');
  if (ct) ct.textContent = shown + ' / ' + document.querySelectorAll('#lc-tbody tr.lc-row').length + ' rows';
}}
// Default sort matches Python-side row order: ratio ascending.
let lcSortKey = 'ratio', lcSortDir = 1;
function lcs(key){{
  if (lcSortKey === key) lcSortDir = -lcSortDir;
  else {{ lcSortKey = key; lcSortDir = 1; }}
  const tbody = document.getElementById('lc-tbody');
  const rows = Array.from(tbody.querySelectorAll('tr.lc-row'));
  const num = ['cbody','rbody','cline','rline','ratio'].includes(key);
  rows.sort((a, b) => {{
    let va, vb;
    if (key === 'name')   {{ va = a.dataset.name;   vb = b.dataset.name; }}
    else if (key === 'status') {{ va = a.dataset.status; vb = b.dataset.status; }}
    else if (key === 'cbody')  {{ va = +a.dataset.cbody; vb = +b.dataset.cbody; }}
    else if (key === 'rbody')  {{ va = +a.dataset.rbody; vb = +b.dataset.rbody; }}
    else if (key === 'cline')  {{ va = +a.dataset.cline; vb = +b.dataset.cline; }}
    else if (key === 'rline')  {{ va = +a.dataset.rline; vb = +b.dataset.rline; }}
    else if (key === 'ratio')  {{ va = +a.dataset.ratio; vb = +b.dataset.ratio; }}
    if (num) return (va - vb) * lcSortDir;
    return va.localeCompare(vb) * lcSortDir;
  }});
  rows.forEach(r => tbody.appendChild(r));
  document.querySelectorAll('table.lc-table thead th').forEach(th => {{
    th.classList.remove('sort-asc', 'sort-desc');
    if (th.dataset.sort === key) {{
      th.classList.add(lcSortDir > 0 ? 'sort-asc' : 'sort-desc');
    }}
  }});
}}
// Initialise the row counter on load.
window.addEventListener('DOMContentLoaded', () => {{
  if (document.getElementById('lcct')) lcf();
  if (document.getElementById('roct')) rof();
  if (document.getElementById('exct')) exf();
}});
// ── EXEC.C (tree-walker, segregated) table: filter + sort ──────────────
function exf(){{
  const q = document.getElementById('exq').value.toLowerCase();
  let shown = 0;
  document.querySelectorAll('#ex-tbody tr.ex-row').forEach(tr => {{
    const ok = !q || tr.dataset.name.toLowerCase().includes(q);
    tr.style.display = ok ? '' : 'none';
    if (ok) shown++;
  }});
  const ct = document.getElementById('exct');
  if (ct) ct.textContent = shown + ' / ' + document.querySelectorAll('#ex-tbody tr.ex-row').length + ' rows';
}}
let exSortKey = null, exSortDir = 1;
function exs(key){{
  if (exSortKey === key) exSortDir = -exSortDir;
  else {{ exSortKey = key; exSortDir = 1; }}
  const tbody = document.getElementById('ex-tbody');
  const rows = Array.from(tbody.querySelectorAll('tr.ex-row'));
  const num = ['cbody','rbody','cline','rline','ratio'].includes(key);
  rows.sort((a, b) => {{
    let va, vb;
    if (key === 'name')        {{ va = a.dataset.name;     vb = b.dataset.name; }}
    else if (key === 'status') {{ va = a.dataset.status;   vb = b.dataset.status; }}
    else if (key === 'cbody')  {{ va = +a.dataset.cbody;   vb = +b.dataset.cbody; }}
    else if (key === 'rbody')  {{ va = +a.dataset.rbody;   vb = +b.dataset.rbody; }}
    else if (key === 'cline')  {{ va = +a.dataset.cline;   vb = +b.dataset.cline; }}
    else if (key === 'rline')  {{ va = +a.dataset.rline;   vb = +b.dataset.rline; }}
    else if (key === 'ratio')  {{ va = +a.dataset.ratio;   vb = +b.dataset.ratio; }}
    if (num) return (va - vb) * exSortDir;
    return va.localeCompare(vb) * exSortDir;
  }});
  rows.forEach(r => tbody.appendChild(r));
  // The exec.c table reuses .lc-table class but tags headers by data-sort;
  // pick the right one via closest tbody match.
  const tableHead = tbody.parentElement.querySelector('thead');
  if (tableHead) {{
    tableHead.querySelectorAll('th').forEach(th => {{
      th.classList.remove('sort-asc', 'sort-desc');
      if (th.dataset.sort === key) {{
        th.classList.add(exSortDir > 0 ? 'sort-asc' : 'sort-desc');
      }}
    }});
  }}
}}
// ── RUST-ONLY table: filter + sort ──────────────────────────────────────
function rof(){{
  const q = document.getElementById('roq').value.toLowerCase();
  let shown = 0;
  document.querySelectorAll('#ro-tbody tr.ro-row').forEach(tr => {{
    const ok = !q || tr.dataset.name.toLowerCase().includes(q);
    tr.style.display = ok ? '' : 'none';
    if (ok) shown++;
  }});
  const ct = document.getElementById('roct');
  if (ct) ct.textContent = shown + ' / ' + document.querySelectorAll('#ro-tbody tr.ro-row').length + ' rows';
}}
let roSortKey = null, roSortDir = 1;
function ros(key){{
  if (roSortKey === key) roSortDir = -roSortDir;
  else {{ roSortKey = key; roSortDir = 1; }}
  const tbody = document.getElementById('ro-tbody');
  const rows = Array.from(tbody.querySelectorAll('tr.ro-row'));
  const num = ['rbody', 'rline'].includes(key);
  rows.sort((a, b) => {{
    let va, vb;
    if (key === 'name')       {{ va = a.dataset.name;     vb = b.dataset.name; }}
    else if (key === 'rbody') {{ va = +a.dataset.rbody;   vb = +b.dataset.rbody; }}
    else if (key === 'rline') {{ va = +a.dataset.rline;   vb = +b.dataset.rline; }}
    if (num) return (va - vb) * roSortDir;
    return va.localeCompare(vb) * roSortDir;
  }});
  rows.forEach(r => tbody.appendChild(r));
  document.querySelectorAll('table.ro-table thead th').forEach(th => {{
    th.classList.remove('sort-asc', 'sort-desc');
    if (th.dataset.sort === key) {{
      th.classList.add(roSortDir > 0 ? 'sort-asc' : 'sort-desc');
    }}
  }});
}}
</script>
</body></html>
"""
    OUT.write_text(html_doc)
    print(f"wrote {OUT} ({len(html_doc):,} bytes)", file=sys.stderr)
    cov_path = ROOT / "docs" / "report.html"
    if cov_path.exists():
        patch_coverage_report_html(
            cov_path,
            by_cfile,
            rows,
            summary={
                "total_rows": total,
                "n_unported": n_unported,
                "n_misplaced": n_misplaced,
                "n_ported": n_ported,
            },
        )
    return 0


# Core `Src/*.c` files surfaced on docs/report.html (dashboard only).
CORE_COVERAGE_FILES: list[tuple[str, str, str]] = [
    ("lex.c", "ported/lex.rs", "src/ported/lex.rs"),
    ("parse.c", "ported/parse.rs", "src/ported/parse.rs"),
    ("subst.c", "src/ported/subst.rs", "src/ported/subst.rs"),
    ("math.c", "src/ported/math.rs", "src/ported/math.rs"),
    (
        "exec.c",
        'src/exec.rs <span style="opacity:.6;">(re-exported as <code>crate::ported::exec</code>)</span>',
        "src/exec.rs",
    ),
    ("params.c", "src/ported/params.rs", "src/ported/params.rs"),
    ("pattern.c", "src/ported/pattern.rs", "src/ported/pattern.rs"),
    ("glob.c", "src/ported/glob.rs", "src/ported/glob.rs"),
    ("jobs.c", "src/ported/jobs.rs", "src/ported/jobs.rs"),
    ("hist.c", "src/ported/hist.rs", "src/ported/hist.rs"),
    ("utils.c", "src/ported/utils.rs", "src/ported/utils.rs"),
    ("prompt.c", "src/ported/prompt.rs", "src/ported/prompt.rs"),
    ("init.c", "src/ported/init.rs", "src/ported/init.rs"),
    ("signals.c", "src/ported/signals.rs", "src/ported/signals.rs"),
]


def _line_count(path: Path) -> int:
    try:
        return sum(1 for _ in path.open("rb"))
    except Exception:
        return 0


def patch_coverage_report_html(
    report_path: Path,
    by_cfile: dict,
    rows: list[dict],
    summary: dict[str, int] | None = None,
) -> None:
    """Rewrite the auto-generated slice of docs/report.html (core file table).

    Keeps styling/marketing prose outside the PORT_REPORT markers untouched,
    but replaces hard-coded per-file stats with numbers derived from the same
    C/Rust index as port_report.html (regex C fn defs + Rust fn defs + port
    doc mining — heuristic, not proof of behavioral parity).
    """
    body_parts: list[str] = []
    sum_c = sum_r = sum_total = sum_sn = sum_rn = sum_ported = 0
    for cf, rust_disp, rust_rel in CORE_COVERAGE_FILES:
        rec = by_cfile.get(cf, {})
        c_lines = int(rec.get("c_lines") or 0)
        rust_lines = _line_count(ROOT / rust_rel)
        file_rows = [r for r in rows if r["cfile"] == cf]
        total = len(file_rows)
        same_name = sum(1 for r in file_rows if r.get("rust_locs"))
        ported = sum(1 for r in file_rows if r["status"] == "ported")
        renamed = max(0, total - same_name)
        ratio = (100.0 * rust_lines / c_lines) if c_lines else 0.0
        cov_pct = (100.0 * ported / total) if total else 0.0
        sum_c += c_lines
        sum_r += rust_lines
        sum_total += total
        sum_sn += same_name
        sum_rn += renamed
        sum_ported += ported
        if cov_pct >= 95:
            bar_cls = "green"
        elif cov_pct >= 50:
            bar_cls = "yellow"
        elif cov_pct > 0:
            bar_cls = "magenta"
        else:
            bar_cls = "magenta"
        st = "&#x2705;" if ported == total and total else "&#x26A0;&#xFE0F;"
        body_parts.append(
            '        <tr><td>'
            f'{html.escape(cf)}</td><td>{rust_disp}</td>'
            f'<td class="num">{c_lines:,}</td>'
            f'<td class="num">{rust_lines:,}</td>'
            f'<td class="num">{ratio:.1f}%</td>'
            f'<td class="num">{total}</td>'
            f'<td class="num">{same_name}</td>'
            f'<td class="num">{renamed}</td>'
            f'<td><div class="bar-wrap"><div class="bar-fill {bar_cls}" style="width:{min(100.0, cov_pct):.1f}%"></div>'
            f'<span class="bar-pct">{cov_pct:.1f}%</span></div></td>'
            f'<td class="status">{st}</td></tr>'
        )
    total_ratio = (100.0 * sum_r / sum_c) if sum_c else 0.0
    total_cov = (100.0 * sum_ported / sum_total) if sum_total else 0.0
    tfoot = (
        '<tfoot><tr class="total-row">'
        '<td colspan="2" style="font-family:\'Orbitron\',sans-serif;font-size:10px;letter-spacing:1px;">'
        "TOTAL (core)</td>"
        f'<td class="num">{sum_c:,}</td>'
        f'<td class="num">{sum_r:,}</td>'
        f'<td class="num">{total_ratio:.1f}%</td>'
        f'<td class="num">{sum_total}</td>'
        f'<td class="num">{sum_sn}</td>'
        f'<td class="num">{sum_rn}</td>'
        f'<td><div class="bar-wrap"><div class="bar-fill cyan" style="width:{min(100.0, total_cov):.1f}%"></div>'
        f'<span class="bar-pct">{total_cov:.1f}%</span></div></td>'
        '<td class="status" style="font-size:18px;">&#x2139;&#xFE0F;</td>'
        "</tr></tfoot>"
    )
    block = (
        "<!-- PORT_REPORT:BEGIN:CORETABLE -->\n        <tbody>\n"
        + "\n".join(body_parts)
        + "\n        </tbody>\n        "
        + tfoot
        + "\n        <!-- PORT_REPORT:END:CORETABLE -->"
    )
    text = report_path.read_text(encoding="utf-8", errors="replace")
    begin = "<!-- PORT_REPORT:BEGIN:CORETABLE -->"
    end = "<!-- PORT_REPORT:END:CORETABLE -->"
    if begin not in text or end not in text:
        print(f"warning: {report_path} missing PORT_REPORT markers; skipping dashboard patch", file=sys.stderr)
        return
    pre, rest = text.split(begin, 1)
    _, post = rest.split(end, 1)
    text = pre + block + post
    if summary:
        desc = (
            "zshrs port/coverage dashboard. "
            f"Indexer: {summary['total_rows']:,} unique C symbols; "
            f"{summary['n_unported']:,} C-only, {summary['n_misplaced']:,} misplaced, "
            f"{summary['n_ported']:,} with a Rust counterpart. "
            f"{sum_total} symbols in the 14-file core slice (table below). "
            "Regenerate: python3 scripts/gen_port_report.py. "
            f"Updated {date.today().isoformat()}."
        )
        text, n_meta = re.subn(
            r'(<meta\s+name="description"\s+content=")[^"]*("\s*>)',
            lambda m: m.group(1) + html.escape(desc) + m.group(2),
            text,
            count=1,
        )
        if n_meta != 1:
            print(f"warning: description meta replace matched {n_meta} times", file=sys.stderr)
        text, n_card = re.subn(
            r'(<div class="stat-card"><div class="stat-val yellow">)[^<]*(</div>\s*<div class="stat-label">C-only symbols \(index\)</div></div>)',
            lambda m: m.group(1) + f"{summary['n_unported']:,}" + m.group(2),
            text,
            count=1,
        )
        if n_card != 1:
            print(f"warning: C-only stat-card replace matched {n_card} times", file=sys.stderr)
    report_path.write_text(text, encoding="utf-8")
    print(f"patched {report_path} core table ({sum_total} C symbols indexed in core files)", file=sys.stderr)

if __name__ == "__main__":
    raise SystemExit(main())
