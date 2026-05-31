#!/usr/bin/env python3
"""Regenerate the canonical regions of the JetBrains plugin from
data/grammar/canonical.json.

Targets:
  * editors/intellij/.../ZshrsLexer.kt — 7 setOf blocks
      control-keywords, decl-keywords, fn-keywords, loop-keywords,
      modifier-keywords, io-keywords, builtins
  * editors/intellij/.../ZshrsColorSettingsPage.kt — color-settings-demo
  * editors/intellij/src/main/resources/META-INF/plugin.xml —
      plugin-description (rewrites COUNT:* sentinel values)

Re-run after editing canonical.json. Output is verbatim Kotlin / XML;
the JetBrains plugin's Kotlin compile (`gradle compileKotlin`) should
remain green.
"""
from __future__ import annotations

import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CANON = ROOT / "data" / "grammar" / "canonical.json"
IJ = ROOT / "editors" / "intellij" / "src" / "main"
LEXER = IJ / "kotlin" / "com" / "menketechnologies" / "zshrs" / "ZshrsLexer.kt"
COLORS = IJ / "kotlin" / "com" / "menketechnologies" / "zshrs" / "ZshrsColorSettingsPage.kt"
PLUGIN = IJ / "resources" / "META-INF" / "plugin.xml"


def kt_escape(s: str) -> str:
    """Kotlin string-literal escape."""
    return s.replace("\\", "\\\\").replace("\"", "\\\"")


def render_setof(indent: int, items: list[str], line_width: int = 92) -> str:
    """Pretty-print a `setOf("a", "b", ...)` over multiple lines, wrapping
    at line_width. The opening `setOf(` and closing `)` belong to the
    caller; this returns only the items grouped into wrapped sub-lines."""
    pre = " " * indent
    lines: list[str] = []
    buf: list[str] = []
    cur_len = len(pre)
    for it in items:
        token = f"\"{kt_escape(it)}\","
        candidate_len = cur_len + (1 if buf else 0) + len(token)
        if buf and candidate_len > line_width:
            lines.append(pre + " ".join(buf))
            buf = [token]
            cur_len = len(pre) + len(token)
        else:
            buf.append(token)
            cur_len = candidate_len
    if buf:
        lines.append(pre + " ".join(buf))
    return "\n".join(lines)


def replace_kt_block(src: str, tag: str, val_name: str, items: list[str]) -> str:
    pat = re.compile(
        rf"(// BEGIN-CANONICAL: {re.escape(tag)}\n).*?(\n\s*// END-CANONICAL: {re.escape(tag)})",
        re.S,
    )
    if not pat.search(src):
        raise SystemExit(f"marker for tag '{tag}' not found in {LEXER}")

    if len(items) == 1 and len(items[0]) < 30:
        block = f'        private val {val_name} = setOf("{kt_escape(items[0])}")'
    else:
        body = render_setof(indent=12, items=items)
        block = (
            f"        private val {val_name} = setOf(\n"
            f"{body}\n"
            f"        )"
        )
    return pat.sub(lambda m: m.group(1) + block + m.group(2), src)


def update_lexer(data: dict) -> None:
    # Map canonical → existing Kotlin sets. The Kotlin lexer has its own
    # taxonomy that differs slightly from canonical:
    #   * CONTROL_KEYWORDS in canonical = category "control" + "grouping"
    #     + the operator-like "!".
    #   * MODIFIER_KEYWORDS overlaps with canonical "modifier" and a
    #     bunch of conventional builtins (alias, setopt, autoload,
    #     bindkey, compdef, …) the lexer colors as keyword instead.
    # The simplest sustainable rule: pull from canonical "keywords" only
    # for the strict subset categories below, and keep MODIFIER_KEYWORDS
    # / BUILTINS as the canonical union of modifier-flavor builtins +
    # core/module/ext builtins.
    canon_by_cat: dict[str, list[str]] = {}
    for e in data["keywords"]:
        canon_by_cat.setdefault(e["category"], []).append(e["name"])

    control = sorted(set(canon_by_cat.get("control", [])
                         + canon_by_cat.get("grouping", [])
                         + canon_by_cat.get("operator", [])))
    decl = sorted(set(canon_by_cat.get("decl", [])))
    fn = sorted(set(canon_by_cat.get("fn", [])))
    loop = sorted(set(canon_by_cat.get("loop", [])))
    # Modifier set = canonical "modifier" + "io" categories +
    # canonical builtin names that are conventionally tagged as
    # "modifier-flavor" in shells: alias/unalias/setopt/unsetopt/
    # zstyle/zmodload/zle/autoload/bindkey/compdef/compinit/zcompile/
    # zparseopts/zformat/zmv/zftp/zcalc/ztcp/zsystem.
    modifier_names = set(canon_by_cat.get("modifier", []))
    modifier_names.update({
        "alias", "unalias", "setopt", "unsetopt",
        "zstyle", "zmodload", "zle",
        "autoload", "bindkey", "compdef", "compinit", "compinstall",
        "compaudit", "zcompile", "zparseopts", "zformat",
        "zmv", "zcp", "zln", "zftp", "zcalc", "ztcp", "zsystem",
        "zsh-newuser-install",
        "fpath", "manpath", "path", "cdpath", "fignore",
    })
    modifier = sorted(modifier_names)

    io_names = set(canon_by_cat.get("io", []))
    io_names.update({
        "echo", "print", "printf", "read", "readarray", "mapfile",
        "true", "false", ":",
    })
    io = sorted(io_names)

    # Builtin set = every canonical builtin (core + module + zshrs ext)
    # MINUS anything we already tagged as keyword/io/modifier above.
    excluded = set(control) | set(decl) | set(fn) | set(loop) \
        | set(modifier) | set(io)
    builtin_pool = set()
    for k in ("core_builtins", "module_builtins", "zshrs_ext_builtins"):
        for e in data[k]:
            builtin_pool.add(e["name"])
    builtins = sorted(builtin_pool - excluded)

    src = LEXER.read_text(encoding="utf-8")
    src = replace_kt_block(src, "control-keywords", "CONTROL_KEYWORDS", control)
    src = replace_kt_block(src, "decl-keywords", "DECL_KEYWORDS", decl)
    src = replace_kt_block(src, "fn-keywords", "FN_KEYWORDS", fn)
    src = replace_kt_block(src, "loop-keywords", "LOOP_KEYWORDS", loop)
    src = replace_kt_block(src, "modifier-keywords", "MODIFIER_KEYWORDS", modifier)
    src = replace_kt_block(src, "io-keywords", "IO_KEYWORDS", io)
    src = replace_kt_block(src, "builtins", "BUILTINS", builtins)
    LEXER.write_text(src, encoding="utf-8")
    print(f"updated {LEXER}: "
          f"{len(control)} ctrl, {len(decl)} decl, {len(fn)} fn, "
          f"{len(loop)} loop, {len(modifier)} mod, {len(io)} io, "
          f"{len(builtins)} builtin")


# ── Color settings demo ─────────────────────────────────────────────
def build_demo(data: dict) -> str:
    """Construct a Kotlin raw-string-friendly demo that shows every
    color category. We use `${"$"}` interpolation for the `$` sigil since
    the file uses Kotlin triple-quoted strings."""
    # Read existing demo back if present; otherwise build one.
    # We embed a comprehensive demo regardless — every grammar category
    # has at least one occurrence.
    DOL = '${"$"}'
    # Triple-quoted Kotlin string: NEWLINES are literal; we just write
    # the script. Each line is 4 indents in.
    lines = [
        "            #!/usr/bin/env zshrs",
        "            ## demo.zsh — every token category for color tweaking.",
        "            ## Doc comments (##) get their own color slot, distinct",
        "            ## from regular # remarks below. Sourced from",
        "            ## data/grammar/canonical.json — regenerate the demo",
        "            ## via scripts/gen_grammar_intellij.py.",
        "            # Regular code comment — uses the plain Line-comment slot.",
        "",
        "            # ── options + module loading ──",
        "            setopt EXTENDED_GLOB NULL_GLOB PIPE_FAIL NO_CLOBBER PROMPT_SUBST",
        "            unsetopt BG_NICE",
        "            zmodload zsh/datetime zsh/parameter zsh/zle",
        "            autoload -Uz compinit && compinit -d ~/.cache/zshrs/zcompdump",
        "",
        "            # ── decl keywords + special vars ──",
        "            typeset -gA Z_PLUGIN_CACHE",
        "            local -i count=0",
        f"            local TIMER={DOL}EPOCHREALTIME",
        f"            integer pid={DOL}{DOL} ppid={DOL}PPID",
        f"            readonly RPROMPT_BACKUP={DOL}RPROMPT",
        "",
        "            ## Greet the user — attached as `greet`'s function doc.",
        "            function greet() {",
        f"                local name=\"{DOL}{{1:-world}}\"",
        f"                print -r -- \"hello, {DOL}name (PID={DOL}{DOL})\"",
        "                return 0",
        "            }",
        "",
        "            # ── for / glob qualifiers / param expansion ──",
        "            for f in ~/.zsh/*.zsh(N.r); do",
        f"                source \"{DOL}f\"",
        f"                (( count++ ))",
        f"                printf '%s\\n' \"{DOL}{{f:t:r}}\"  # :t = tail, :r = root",
        "            done",
        "",
        "            # ── conditionals + arithmetic ──",
        f"            if [[ -n \"{DOL}HOME\" && -d \"{DOL}HOME/bin\" ]]; then",
        f"                path=(\"{DOL}HOME/bin\" {DOL}path)",
        f"            elif (( count == 0 || {DOL}#argv == 0 )); then",
        "                echo \"no plugins\" >&2",
        "            fi",
        "",
        "            # ── heredoc + pipe + redirect + background ──",
        "            cat <<EOF | grep -E 'foo|bar' &>/tmp/out.log &",
        "            multi-line",
        "            heredoc body",
        "            EOF",
        "",
        "            # ── case with all 3 branch terminators ──",
        f"            case {DOL}1 in",
        f"                start|run)  greet \"{DOL}@\" ;;",
        f"                stop)       kill %1 ;;&  # ;;& fall-through-and-test",
        f"                status)     jobs -l ;|   # ;| fall-through unconditional",
        f"                *)          print \"usage: {DOL}0 {{start|stop}}\" ;;",
        "            esac",
        "",
        "            # ── parameter flags + special syntaxes ──",
        f"            print -- {DOL}{{(L)PATH}}            # lowercase",
        f"            print -- {DOL}{{(j:,:)path}}         # join array with ','",
        f"            print -- {DOL}{{(s:/:)PWD}}          # split on '/'",
        f"            print -- {DOL}{{(P)var}}             # indirect ref",
        f"            print -- {DOL}{{var//pat/repl}}      # replace all",
        f"            print -- {DOL}{{var:#pat}}           # remove matching",
        "",
        "            # ── strings: ANSI-C, single, double, backtick, locale ──",
        "            local ansi=$'tab\\there\\n'",
        f"            local literal='no {DOL}expand here'",
        f"            local interp=\"expand {DOL}var here\"",
        "            local cmdsub=`uname -m`",
        f"            local trans={DOL}\"locale-translated\"",
        "",
        "            # ── arithmetic command + (( )) + regex + procsub ──",
        "            (( a = 1 + 2 * 3, b = a ** 2 ))",
        f"            [[ \"{DOL}str\" =~ ^[A-Z]+{DOL} ]] && print match",
        "            diff <(sort a.txt) <(sort b.txt) >(tee out.log)",
        "",
        "            # ── try/always ──",
        "            {",
        "                might_fail",
        "            } always {",
        "                cleanup_temp_files",
        "            }",
        "",
        "            # ── history expansion + word modifiers ──",
        "            !!:gs/foo/bar/  # global substitute on previous cmd",
        "            !$              # last word of previous cmd",
        "",
        "            # ── zshrs-exclusive: parallel + AOP ──",
        "            parallel for-each url in \"https://example.com/\"; do",
        f"                curl -s \"{DOL}url\"",
        "            done",
        "            intercept before 'rm' record-trash",
        "",
    ]
    return "\n".join(lines)


def update_color_settings(data: dict) -> None:
    src = COLORS.read_text(encoding="utf-8")
    pat = re.compile(
        r"(// BEGIN-CANONICAL: color-settings-demo\n.*?private val DEMO = \"\"\"\n)"
        r"(.*?)"
        r"(\n        \"\"\"\.trimIndent\(\)\n\s*// END-CANONICAL: color-settings-demo)",
        re.S,
    )
    if not pat.search(src):
        raise SystemExit("color-settings-demo markers not found")
    body = build_demo(data)
    new_src = pat.sub(lambda m: m.group(1) + body + m.group(3), src)
    COLORS.write_text(new_src, encoding="utf-8")
    print(f"updated {COLORS}")


# ── plugin.xml COUNT sentinels ──────────────────────────────────────
def update_plugin_xml(data: dict) -> None:
    src = PLUGIN.read_text(encoding="utf-8")
    counts = {
        "keywords": len(data["keywords"]),
        "builtins": (
            len(data["core_builtins"])
            + len(data["module_builtins"])
            + len(data["zshrs_ext_builtins"])
        ),
        "options": len(data["options"]),
        "special_vars": len(data["special_vars"]),
        "param_flags": len(data["param_flags"]),
        "glob_qualifiers": len(data["glob_qualifiers"]),
        "operators": len(data["operators"]),
        "history_expansions": len(data["history_expansions"]),
        "modifiers": len(data["modifiers"]),
    }
    for key, n in counts.items():
        # Each sentinel line looks like:
        #   <li><!-- COUNT:keywords --><b>0</b> ...
        # Replace the <b>N</b> token but preserve everything else.
        pat = re.compile(
            rf"(<!-- COUNT:{re.escape(key)} -->\s*<b>)\d+(</b>)"
        )
        m = pat.search(src)
        if not m:
            raise SystemExit(f"COUNT:{key} sentinel not found in {PLUGIN}")
        src = pat.sub(lambda m, n=n: f"{m.group(1)}{n}{m.group(2)}", src)
    PLUGIN.write_text(src, encoding="utf-8")
    print(f"updated {PLUGIN}: {counts}")


def main() -> None:
    data = json.loads(CANON.read_text(encoding="utf-8"))
    update_lexer(data)
    update_color_settings(data)
    update_plugin_xml(data)


if __name__ == "__main__":
    main()
