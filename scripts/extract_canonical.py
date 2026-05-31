#!/usr/bin/env python3
"""Build data/grammar/canonical.json from upstream zsh C + zshrs Rust.

Single source of truth for keywords / builtins / options / special vars
/ parameter flags / glob qualifiers / operators / redirections,
consumed by the per-surface generators:

* scripts/gen_grammar_lsp.py       (Rust const arrays + LSP completion words)
* scripts/gen_grammar_intellij.py  (Kotlin setOf + Color Settings + plugin.xml)
* scripts/gen_grammar_docs.py      (HTML tables + LaTeX tables)

Re-extract whenever the upstream zsh sources land new entries; never
hand-edit canonical.json directly except for the curated tables
(parameter flags, glob qualifiers, operators, redirections, special
operator categories) that have no single C table.

Run: ``python3 scripts/extract_canonical.py``
"""
from __future__ import annotations

import json
import re
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ZSRC = ROOT / "src" / "zsh" / "Src"
OUT = ROOT / "data" / "grammar" / "canonical.json"


def slurp(p: Path) -> str:
    return p.read_text(encoding="utf-8", errors="replace")


# ── Keywords (reswds[]) ─────────────────────────────────────────────
RESWD_RE = re.compile(r'\{\{NULL,\s*"([^"]+)"\s*,\s*0\}\s*,\s*([A-Z_]+)\}')

KEYWORD_CATEGORIES = {
    # control flow
    "if": "control", "then": "control", "else": "control", "elif": "control", "fi": "control",
    "for": "control", "foreach": "control", "while": "control", "until": "control",
    "do": "control", "done": "control", "case": "control", "esac": "control",
    "select": "control", "repeat": "control", "in": "control", "end": "control",
    "[[": "control", "]]": "control",
    # declaration
    "typeset": "decl", "declare": "decl", "export": "decl", "readonly": "decl",
    "integer": "decl", "float": "decl", "local": "decl", "private": "decl",
    # function
    "function": "fn",
    # IO / source
    # (none in reswds[] proper — handled below in zshrs/POSIX-extension list)
    # modifiers
    "time": "modifier", "nocorrect": "modifier", "coproc": "modifier",
    # grouping
    "{": "grouping", "}": "grouping",
    # negation
    "!": "operator",
}


def extract_keywords() -> list[dict]:
    src = slurp(ZSRC / "hashtable.c")
    # narrow to the reswds[] block
    m = re.search(r'static struct reswd reswds\[\] = \{(.*?)\n\};', src, re.S)
    body = m.group(1) if m else ""
    names = [name for name, _ in RESWD_RE.findall(body)]

    # zshrs/extension keywords missing from upstream reswds[] that the
    # IntelliJ lexer and LSP nonetheless tag as keyword tokens.
    extras = {
        "in": "control",           # parser keyword, not in reswds[]
        "]]": "control",           # cond close, lexer-recognized
        "always": "control",       # try/always blocks (Misc/try.zsh)
        "noglob": "modifier",      # precommand modifier
        "break": "loop", "continue": "loop", "return": "loop", "exit": "loop",
        "logout": "loop",
        "source": "io", ".": "io", "eval": "io", "exec": "io", "trap": "io",
        # POSIX-shell builtin-like keywords commonly highlighted
        "builtin": "modifier", "command": "modifier",
        "let": "decl", "set": "decl", "shift": "decl",
    }
    seen = set(names)
    out = []
    for name in names:
        out.append({
            "name": name,
            "category": KEYWORD_CATEGORIES.get(name, "control"),
            "origin": "reswds[]",
        })
    for name, cat in extras.items():
        if name in seen:
            continue
        out.append({
            "name": name,
            "category": cat,
            "origin": "extension",
        })
    out.sort(key=lambda d: (d["category"], d["name"]))
    return out


# ── Core builtins (Src/builtin.c) ────────────────────────────────────
BUILTIN_RE = re.compile(r'BUILTIN\("([^"]+)"')


def extract_core_builtins() -> list[dict]:
    src = slurp(ZSRC / "builtin.c")
    m = re.search(r'static struct builtin builtins\[\] =(.*?)\n\};', src, re.S)
    body = m.group(1) if m else ""
    names = sorted(set(BUILTIN_RE.findall(body)))
    return [{"name": n, "origin": "zsh/Src/builtin.c"} for n in names]


# ── Module builtins (Src/Modules/*.c) ────────────────────────────────
def extract_module_builtins() -> list[dict]:
    out = []
    mods_dir = ZSRC / "Modules"
    if not mods_dir.is_dir():
        return out
    for p in sorted(mods_dir.glob("*.c")):
        body = slurp(p)
        # only scan declared builtin tables, not BUILTIN("…") tokens
        # found in comments / examples.
        for m in re.finditer(r'static struct builtin [A-Za-z_]+\[\] =\s*\{(.*?)\n\};',
                             body, re.S):
            names = sorted(set(BUILTIN_RE.findall(m.group(1))))
            for n in names:
                out.append({"name": n, "origin": f"module:{p.stem}"})
    # dedup by name, keep first-seen module
    seen = {}
    for entry in out:
        seen.setdefault(entry["name"], entry)
    return sorted(seen.values(), key=lambda d: d["name"])


# ── zshrs extension builtins (daemon/builtins.rs + src/extensions/) ──
def extract_zshrs_ext_builtins() -> list[dict]:
    out: set[str] = set()
    for rel in [
        "daemon/builtins.rs",
        "src/extensions/ext_builtins.rs",
    ]:
        p = ROOT / rel
        if not p.is_file():
            continue
        for m in re.finditer(r'"(z[a-z][a-z_0-9]*)"\s*=>', slurp(p)):
            out.add(m.group(1))
    return [{"name": n, "origin": "zshrs-ext"} for n in sorted(out)]


# ── Options (Src/options.c::optns[]) ─────────────────────────────────
OPTN_RE = re.compile(r'\{\{NULL,\s*"([^"]+)"')


def extract_options() -> list[dict]:
    src = slurp(ZSRC / "options.c")
    m = re.search(r'static struct optname optns\[\] = \{(.*?)\n\};', src, re.S)
    body = m.group(1) if m else ""
    names = sorted(set(OPTN_RE.findall(body)))
    return [{"name": n, "origin": "options.c"} for n in names]


# ── Special vars (Src/params.c::special_params[]) ────────────────────
IPDEF_RE = re.compile(r'IPDEF[A-Z0-9]*\("([^"]+)"')


def extract_special_vars() -> list[dict]:
    src = slurp(ZSRC / "params.c")
    m = re.search(r'static initparam special_params\[\] =\s*\{(.*?)\n\};', src, re.S)
    body = m.group(1) if m else ""
    names = sorted(set(IPDEF_RE.findall(body)))
    return [{"name": n, "origin": "params.c"} for n in names]


# ── Parameter flags (curated — these live as letters inside ${(X)var}, no C table)
PARAM_FLAGS: list[tuple[str, str]] = [
    ("@", "array-context retain $@ semantics; quoting-preserving even in scalar context"),
    ("A", "create as an array"),
    ("a", "sort by array index"),
    ("c", "count words in a parameter (e.g., scalar split count)"),
    ("C", "capitalize words"),
    ("D", "treat as DIRECTORY name (apply directory substitution like ~/...)"),
    ("e", "perform parameter expansion / arithmetic / etc. on the result"),
    ("f", "split result at newlines"),
    ("F", "join array elements with newlines"),
    ("g", "process escape sequences like print does (g:o: process octals, g:c: process \\c)"),
    ("i", "case-insensitive sort"),
    ("j", "join array with separator: ${(j:sep:)arr}"),
    ("k", "for assoc arrays: keys ${(k)hash}"),
    ("K", "subscript flags: use keys"),
    ("L", "lowercase"),
    ("M", "match: use longest match (also for case-insensitivity in sort)"),
    ("n", "numeric sort"),
    ("o", "sort ascending"),
    ("O", "sort descending"),
    ("p", "interpret embedded escape sequences in j/s separator"),
    ("P", "treat value as parameter name → indirect (P)"),
    ("q", "quote the result (q-/q+/qq/qqq variants for shell-quote levels)"),
    ("Q", "remove quoting"),
    ("r", "right-justify within field width: ${(r:N::pad:)var}"),
    ("l", "left-justify within field width: ${(l:N::pad:)var}"),
    ("s", "split at separator: ${(s:sep:)var}"),
    ("S", "subscript: search subscript ranges"),
    ("t", "test parameter type"),
    ("u", "unique (dedupe array)"),
    ("U", "uppercase"),
    ("v", "for assoc arrays: values ${(v)hash}"),
    ("V", "make invisible / control chars visible"),
    ("w", "split into words"),
    ("W", "split into words (alternate)"),
    ("z", "split as the shell would (z-tokens)"),
    ("#", "expand result as arithmetic; numeric value"),
    ("%", "expand prompt percent escapes in result"),
    ("~", "treat values as patterns (e.g., for /pat/repl)"),
]


# ── Glob qualifiers (curated — single-letter modifiers inside *(X)) ──
GLOB_QUALIFIERS: list[tuple[str, str]] = [
    ("/", "directories only"),
    (".", "regular files only"),
    ("@", "symbolic links only"),
    ("=", "sockets only"),
    ("p", "named pipes (FIFOs) only"),
    ("*", "executable plain files only"),
    ("%", "device special files only"),
    ("%b", "block special files"),
    ("%c", "character special files"),
    ("r", "owner-readable"),
    ("w", "owner-writable"),
    ("x", "owner-executable"),
    ("A", "group-readable"),
    ("I", "group-writable"),
    ("E", "group-executable"),
    ("R", "world-readable"),
    ("W", "world-writable"),
    ("X", "world-executable"),
    ("s", "setuid (S_ISUID)"),
    ("S", "setgid (S_ISGID)"),
    ("t", "sticky bit"),
    ("d N", "device number N"),
    ("l[+-=]N", "exactly / less-than / greater-than N hard links"),
    ("U", "owned by EUID"),
    ("G", "owned by EGID"),
    ("u N", "owned by uid N"),
    ("g N", "group gid N"),
    ("f spec", "permission mask: f:o+w: e.g."),
    ("L [+-=] N", "size: blocks / k / m / p suffix (Lk / Lm / Lp)"),
    ("a [Mwhms] [+-=] N", "access time"),
    ("m [Mwhms] [+-=] N", "modify time"),
    ("c [Mwhms] [+-=] N", "ctime"),
    ("o [name|size|links|mtime|atime|ctime]", "sort ascending"),
    ("O [name|size|links|mtime|atime|ctime]", "sort descending"),
    ("[N,M]", "select range of matches"),
    ("e:str:", "external test: each match passed to expression"),
    ("+func", "external test: each match passed to function"),
    ("N", "nullglob: silently drop unmatched pattern"),
    ("D", "include dotfiles"),
    ("Y N", "limit to N results"),
    ("M", "include directory names as if trailing slash"),
    (":mod", "apply history modifier (e.g., :t :h :r :e)"),
    ("^", "negate the qualifier list"),
    (",", "OR-combine qualifier groups"),
]


# ── Operators / redirections (curated; the comprehensive lexer view) ─
OPERATORS: list[dict] = [
    {"sym": "|",   "kind": "pipeline",     "doc": "Pipeline. stdout of LHS → stdin of RHS."},
    {"sym": "|&",  "kind": "pipeline",     "doc": "Pipeline with stderr merged (= |2>&1)."},
    {"sym": "&&",  "kind": "list",         "doc": "Logical AND: run RHS only if LHS exit==0."},
    {"sym": "||",  "kind": "list",         "doc": "Logical OR: run RHS only if LHS exit!=0."},
    {"sym": ";",   "kind": "list",         "doc": "Sequence: run RHS after LHS regardless of status."},
    {"sym": "&",   "kind": "list",         "doc": "Background: run LHS async; sets $!."},
    {"sym": ";;",  "kind": "case",         "doc": "End case arm."},
    {"sym": ";;&", "kind": "case",         "doc": "Fall through and test next case pattern."},
    {"sym": ";|",  "kind": "case",         "doc": "Fall through without test."},
    {"sym": "!",   "kind": "neg",          "doc": "Negate exit status (reserved word)."},
    {"sym": ">",   "kind": "redirect",     "doc": "Stdout redirect (overwrite)."},
    {"sym": ">>",  "kind": "redirect",     "doc": "Stdout append."},
    {"sym": "<",   "kind": "redirect",     "doc": "Stdin redirect."},
    {"sym": "<<",  "kind": "redirect",     "doc": "Heredoc; body terminated by marker."},
    {"sym": "<<-", "kind": "redirect",     "doc": "Heredoc, strip leading tabs from body."},
    {"sym": "<<<", "kind": "redirect",     "doc": "Here-string; literal text as stdin."},
    {"sym": ">|",  "kind": "redirect",     "doc": "Stdout force-overwrite (bypass NO_CLOBBER)."},
    {"sym": ">!",  "kind": "redirect",     "doc": "Same as >|; force-overwrite."},
    {"sym": "&>",  "kind": "redirect",     "doc": "Redirect both stdout and stderr (bash-compat)."},
    {"sym": "&>>", "kind": "redirect",     "doc": "Append both stdout and stderr."},
    {"sym": "2>&1","kind": "redirect",     "doc": "Duplicate fd2 to fd1 (stderr → stdout)."},
    {"sym": ">&-", "kind": "redirect",     "doc": "Close fd."},
    {"sym": "<>",  "kind": "redirect",     "doc": "Open for read+write."},
    {"sym": "<(",  "kind": "procsub",      "doc": "Process substitution: <(cmd) is a path readable from cmd's stdout."},
    {"sym": ">(",  "kind": "procsub",      "doc": "Process substitution: >(cmd) is a path writable into cmd's stdin."},
    {"sym": "=(",  "kind": "procsub",      "doc": "Zsh-only =(cmd): tempfile capture."},
    {"sym": "$(",  "kind": "subst",        "doc": "Command substitution: $(cmd) captures cmd's stdout."},
    {"sym": "${",  "kind": "subst",        "doc": "Parameter expansion: ${var}."},
    {"sym": "$((", "kind": "subst",        "doc": "Arithmetic expansion: $((expr))."},
    {"sym": "((",  "kind": "arith",        "doc": "Arithmetic command. ((expr)) exits 0 iff expr != 0."},
    {"sym": "))",  "kind": "arith",        "doc": "Close arithmetic command."},
    {"sym": "[[",  "kind": "cond",         "doc": "Open conditional command: [[ expr ]]."},
    {"sym": "]]",  "kind": "cond",         "doc": "Close conditional command."},
    {"sym": "=",   "kind": "assign",       "doc": "Assignment. Also: equality in [[ ]]."},
    {"sym": "+=",  "kind": "assign",       "doc": "Append assignment (scalar concat / array push)."},
    {"sym": "-=",  "kind": "assign",       "doc": "Numeric subtract-assign (in (( ))). "},
    {"sym": ":=",  "kind": "assign",       "doc": "${var:=default}: assign default if unset/empty."},
    {"sym": "?=",  "kind": "assign",       "doc": "(arith) ternary."},
    {"sym": "==",  "kind": "compare",      "doc": "Equality in (( )) / [[ ]]."},
    {"sym": "!=",  "kind": "compare",      "doc": "Inequality."},
    {"sym": "=~",  "kind": "compare",      "doc": "Regex match in [[ ]] (POSIX ERE / PCRE depending on opts)."},
    {"sym": "*",   "kind": "glob",         "doc": "Glob: match any sequence (including empty)."},
    {"sym": "**",  "kind": "glob",         "doc": "Recursive glob (matches dir/subdir/.../ levels)."},
    {"sym": "?",   "kind": "glob",         "doc": "Glob: match one character."},
    {"sym": "~",   "kind": "tilde",        "doc": "Tilde expansion: ~ → $HOME, ~user, ~+ / ~- / ~N for dirstack."},
    {"sym": "{a,b,c}", "kind": "brace",    "doc": "Brace expansion: comma-separated list."},
    {"sym": "{1..10}", "kind": "brace",    "doc": "Brace expansion: numeric range."},
    {"sym": "{a..z}",  "kind": "brace",    "doc": "Brace expansion: character range."},
    {"sym": "${~var}", "kind": "expansion","doc": "Treat result of var as pattern."},
    {"sym": "${^var}", "kind": "expansion","doc": "Array element rcexpansion."},
    {"sym": "${=var}", "kind": "expansion","doc": "Word-split on IFS."},
    {"sym": "$'…'", "kind": "string",      "doc": "ANSI-C quoted string: \\n \\t \\xNN etc."},
    {"sym": "$\"…\"", "kind": "string",    "doc": "Locale-translated string."},
    {"sym": "`…`", "kind": "subst",        "doc": "Backtick command substitution (legacy form of $())."},
    {"sym": "@{}", "kind": "extension",    "doc": "Zshrs @-prefix: dispatch to stryke embedded scripting."},
]


# ── Special operator categories: history expansion, modifiers, etc. ──
HISTORY_EXPANSIONS = [
    ("!!",  "previous command"),
    ("!N",  "command N in history"),
    ("!-N", "command N back"),
    ("!?str", "most recent containing str"),
    ("!str", "most recent starting with str"),
    ("!$",  "last word of previous command"),
    ("!^",  "first arg of previous command"),
    ("!*",  "all args of previous command"),
    ("!:N", "Nth word of previous command"),
    ("^old^new", "quick substitute in previous command"),
]

MODIFIERS = [
    (":h", "head: dirname of path"),
    (":t", "tail: basename of path"),
    (":r", "root: strip extension"),
    (":e", "extension: keep only extension"),
    (":l", "lowercase"),
    (":u", "uppercase"),
    (":q", "quote for shell re-input"),
    (":Q", "remove quoting"),
    (":s/old/new/", "substitute first match"),
    (":gs/old/new/", "global substitute"),
    (":a", "absolutize (resolve relative path)"),
    (":A", "absolutize and resolve symlinks"),
    (":P", "physical resolved path"),
    (":x", "split into words on whitespace"),
    (":w", "select words"),
    (":F", "follow symlinks (in conjunction with above)"),
]


def main() -> None:
    data = {
        "_meta": {
            "generated": date.today().isoformat(),
            "generator": "scripts/extract_canonical.py",
            "sources": {
                "keywords": "src/zsh/Src/hashtable.c::reswds[]",
                "core_builtins": "src/zsh/Src/builtin.c::builtins[]",
                "module_builtins": "src/zsh/Src/Modules/*.c",
                "zshrs_ext_builtins": "daemon/builtins.rs + src/extensions/ext_builtins.rs",
                "options": "src/zsh/Src/options.c::optns[]",
                "special_vars": "src/zsh/Src/params.c::special_params[]",
                "param_flags": "curated (no single C table)",
                "glob_qualifiers": "curated (no single C table)",
                "operators": "curated (lexer-scattered)",
                "history_expansions": "curated",
                "modifiers": "curated",
            },
        },
        "keywords": extract_keywords(),
        "core_builtins": extract_core_builtins(),
        "module_builtins": extract_module_builtins(),
        "zshrs_ext_builtins": extract_zshrs_ext_builtins(),
        "options": extract_options(),
        "special_vars": extract_special_vars(),
        "param_flags": [{"flag": f, "doc": d} for f, d in PARAM_FLAGS],
        "glob_qualifiers": [{"sym": s, "doc": d} for s, d in GLOB_QUALIFIERS],
        "operators": OPERATORS,
        "history_expansions": [{"sym": s, "doc": d} for s, d in HISTORY_EXPANSIONS],
        "modifiers": [{"sym": s, "doc": d} for s, d in MODIFIERS],
    }
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(data, indent=2, ensure_ascii=False) + "\n",
                   encoding="utf-8")
    counts = {k: len(v) for k, v in data.items() if isinstance(v, list)}
    print(f"wrote {OUT} ({sum(counts.values())} entries)")
    for k, v in sorted(counts.items()):
        print(f"  {k:24s} {v:4d}")


if __name__ == "__main__":
    main()
