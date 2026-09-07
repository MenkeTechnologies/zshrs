#!/usr/bin/env python3
"""gen_parity_combos.py — derive the zstyle combo fixtures from the live one.

    scripts/gen_parity_combos.py            # regenerate scripts/parity_combos/

The combos used to be hand-maintained copies of the full fixture with a few
lines deleted, so every refresh of `parity_zstyle.zsh` left them stale. They
are now DERIVED: each combo is a BASE config with one axis removed (or one
axis forced to a specific value), which is what isolates a rendering
difference to a single style.

Two shapes:

  drop-<axis>   the full config minus every statement setting that style.
                Answers "does the divergence need this style to be set?"
  force-<name>  a base config plus an override pinning one style to a value.
                Answers "does it need this style set to THIS value?" — not the
                same question, since unset and set-to-off differ for styles
                whose compsys default is on.

WHY A FORCE COMBO NAMES ITS BASE
--------------------------------
A forced statement only measures something if it WINS the zstyle lookup. It
does not always. `zstyle` keeps the patterns for one style sorted by weight
descending (c:Src/Modules/zutil.c:91-92) and `lookupstyle` returns the FIRST
one that matches the context (c:zutil.c:443-459) — so a more specific pattern
already in the base silently outranks the override, which then never takes
effect on either shell. The high bits of the weight are the number of
`:`-separated components (c:zutil.c:355-357), so component COUNT dominates
specificity before the per-component score is even consulted.

`parity_zstyle.zsh` is a real daily-driver config and contains

    zstyle ':completion:*:*:*:*:*' menu 'select=0' interactive

which is six colons against the two of `:completion:*`. Every `menu` lookup
compsys performs carries six colons — `_setup` looks the style up at
`":completion:${curcontext}:$1"` and `curcontext` is
`<function>:<completer>:<command>:<argument>` (sh:vendor/zsh/functions/_setup:67)
— so that line matches all of them and a `:completion:*` menu override on the
full base is inert. Measured, both shells agreeing:

    zsh  -f : full + `zstyle ':completion:*' menu yes select=0`
              -> `zstyle -a ':completion::complete:ls::' menu` = select=0 interactive
    zshrs   : same

Such a combo is a DECOY: it certifies a shell that is broken for the value it
claims to test. `menu yes select=0` on the full base PASSED against a zshrs
built before e43acd9414, the commit that fixed exactly that value leaving the
inserted match off the command line; on the `minimal` base the same force
FAILS there and passes on current main.

So each force combo names the base it is layered on, and `check_forces()`
refuses to emit any combo whose override cannot win a lookup at any of
`PROBE_CONTEXTS`. zsh itself arbitrates — reimplementing the weight rule and
zsh pattern matching here would be a second, unverified copy of the thing
being tested.
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import tempfile

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FIXTURE = os.path.join(REPO, "scripts", "parity_zstyle.zsh")
OUTDIR = os.path.join(REPO, "scripts", "parity_combos")

# The shell that arbitrates the shadow check. It is the REFERENCE zsh on
# purpose: the question a force combo has to answer is "does this override win
# under the semantics the harness compares against", and zshrs is the shell
# under test, not the authority on them.
ZSH = os.environ.get("PARITY_ZSH", "zsh")

# Contexts the shadow check probes. Every one has the shape a real compsys
# lookup has — `:completion:<function>:<completer>:<command>:<argument>:<tag>`,
# six colons — because that is what `_setup` and `_description` build
# (sh:vendor/zsh/functions/_setup:67). Probing `:completion:*`-shaped contexts
# instead would report a force as effective at a context compsys never looks
# anything up at, which is the decoy this check exists to stop.
PROBE_CONTEXTS = (
    ":completion::complete:ls::default",
    ":completion::complete:ls::all-files",
    ":completion::complete:ls::descriptions",
    ":completion::complete:git::heads",
    ":completion::complete:zsh:option-1-1:options",
    ":completion::approximate:ls::corrections",
    ":completion::complete:zinit:argument-rest:plugins",
)

# axis name -> style name(s) whose statements get dropped
DROP_AXES: dict[str, tuple[str, ...]] = {
    "menu": ("menu",),
    "format": ("format", "auto-description"),
    "listcolors": ("list-colors",),
    "listprompt": ("list-prompt",),
    "selectprompt": ("select-prompt",),
    "groupname": ("group-name", "group-order"),
    "verbose": ("verbose", "extra-verbose"),
    "matcherlist": ("matcher-list", "matcher"),
    "completer": ("completer",),
    "listpacked": ("list-packed", "list-rows-first", "list-grouped"),
    "acceptexact": ("accept-exact", "accept-exact-dirs"),
    "squeezeslashes": ("squeeze-slashes",),
    "specialdirs": ("special-dirs",),
    "ignored": ("ignored-patterns", "ignore-parents", "ignore-line"),
    "insertunambiguous": ("insert-unambiguous", "original", "prompt"),
    "hosts": ("hosts", "users", "users-hosts"),
    "cache": ("use-cache", "cache-path", "cache-policy"),
}

# A deliberately tiny config: the two styles that most change list shape.
MINIMAL = (
    "zstyle ':completion:*:descriptions' format '-<<%d>>-'",
    "zstyle ':completion:*' group-name ''",
)

# combo name -> (base name, zstyle lines appended AFTER that base)
#
# The base is `full` unless the live config sets the same style at a
# specificity the override cannot beat. Every `menu` override is on `minimal`
# for that reason and that reason only — see the module docstring. It is not a
# preference: `check_forces()` rejects the `full` variants outright.
FORCE_COMBOS: dict[str, tuple[str, tuple[str, ...]]] = {
    "menu-select": ("minimal", ("zstyle ':completion:*' menu select",)),
    # `interactive` puts the menuselect keymap into filter mode: printable
    # characters narrow the list instead of self-inserting. Without this combo
    # the menusel_* sequences never reach that keymap at all.
    "menu-select-interactive": (
        "minimal", ("zstyle ':completion:*' menu select interactive",)),
    "menu-select-search": (
        "minimal", ("zstyle ':completion:*' menu select search",)),
    "menu-off": ("minimal", ("zstyle ':completion:*' menu no",)),
    "menu-yes-eq-2": ("minimal", ("zstyle ':completion:*' menu yes=2",)),
    # A truth-word AND a select= count in one value. Neither half alone trips
    # the entry path `e43acd9414` fixed: `menu yes` leaves MENUSELECT unset so
    # the `menu_start` hook bails, and `menu select=0` picks
    # insert=automenu-unambiguous so `menucmp` is never set and the hook never
    # runs. Only the combination reaches `domenuselect` with a buffer to sync.
    "menu-yes-select-0": (
        "minimal", ("zstyle ':completion:*' menu yes select=0",)),
    "listpacked-on": ("full", ("zstyle ':completion:*' list-packed true",)),
    "listpacked-off": ("full", ("zstyle ':completion:*' list-packed false",)),
    "rowsfirst-on": ("full", ("zstyle ':completion:*' list-rows-first true",)),
    "groupname-off": ("full", ("zstyle ':completion:*' group-name",)),
    "verbose-off": ("full", ("zstyle ':completion:*' verbose false",)),
    "no-descriptions": (
        "full", ("zstyle ':completion:*:descriptions' format ''",)),
    "single-completer": ("full", ("zstyle ':completion:*' completer _complete",)),
    "complete-then-approximate": (
        "full", ("zstyle ':completion:*' completer _complete _approximate",)),
    "matcher-caseless": (
        "full", ("zstyle ':completion:*' matcher-list 'm:{a-zA-Z}={A-Za-z}'",)),
    "matcher-none": ("full", ("zstyle ':completion:*' matcher-list ''",)),
}

HEADER = """\
# GENERATED by scripts/gen_parity_combos.py — do not hand-edit.
# Source fixture: scripts/parity_zstyle.zsh
# {what}
"""


def statements(path: str) -> list[str]:
    return [l.rstrip("\n") for l in open(path) if l.startswith("zstyle ")]


def style_of(stmt: str) -> str | None:
    """The style NAME in a `zstyle <context> <style> [values...]` statement.

    The context is quoted (`':completion:*'`) or a bare word; the style is the
    token right after it. `-e`/`-L`-style flags are not emitted by `zstyle -L`
    for value statements, so a positional split is enough.
    """
    m = re.match(r"^zstyle\s+(?:'[^']*'|\"[^\"]*\"|\S+)\s+(\S+)", stmt)
    return m.group(1) if m else None


def zq(s: str) -> str:
    """`s` as a single-quoted zsh word."""
    return "'" + s.replace("'", "'\\''") + "'"


def probe_script(base: list[str], forced: list[str], styles: list[str]) -> str:
    """A zsh script printing `<style>\\t<context>\\t<verdict>` per probe.

    It resolves each style twice — once with the overrides ALONE, which is what
    the combo intends, and once with the base underneath them, which is what
    the file actually does — and compares the two value ARRAYS in zsh, so no
    serialisation of style values back into Python can distort the verdict.

        NOMATCH    the override's own pattern does not match this context, so
                   this context says nothing about it
        EFFECTIVE  the override wins the lookup here
        SHADOWED   a base statement outranks it here (c:zutil.c:443-459)
    """
    out = [
        "emulate -L zsh",
        "zmodload -i zsh/zutil",
        "typeset -A fh ff eh ef",
        "typeset -a v",
        "typeset k s c verdict",
        "ctxs=( %s )" % " ".join(zq(c) for c in PROBE_CONTEXTS),
        "styles=( %s )" % " ".join(zq(s) for s in styles),
        "",
        "# 1. the overrides alone — the value the combo means to pin.",
    ]
    out += forced
    out += [
        "for s in $styles; do for c in $ctxs; do",
        '  k="$s|$c"',
        "  if zstyle -a $c $s v; then",
        '    fh[$k]=1; ff[$k]="${(pj:\\0:)v}"',
        "  else",
        '    fh[$k]=0; ff[$k]=""',
        "  fi",
        "done; done",
        "",
        "zstyle -d",
        "",
        "# 2. the base underneath them — the value the file resolves to.",
    ]
    out += base + forced
    out += [
        "for s in $styles; do for c in $ctxs; do",
        '  k="$s|$c"',
        "  if zstyle -a $c $s v; then",
        '    eh[$k]=1; ef[$k]="${(pj:\\0:)v}"',
        "  else",
        '    eh[$k]=0; ef[$k]=""',
        "  fi",
        "done; done",
        "",
        "for s in $styles; do for c in $ctxs; do",
        '  k="$s|$c"',
        "  if (( ! fh[$k] )); then",
        "    verdict=NOMATCH",
        '  elif (( eh[$k] )) && [[ "$ef[$k]" == "$ff[$k]" ]]; then',
        "    verdict=EFFECTIVE",
        "  else",
        "    verdict=SHADOWED",
        "  fi",
        "  print -r -- \"$s\"$'\\t'\"$c\"$'\\t'\"$verdict\"",
        "done; done",
    ]
    return "\n".join(out) + "\n"


def verdicts(base: list[str], forced: list[str]) -> list[tuple[str, str, str]]:
    styles = sorted({s for s in (style_of(f) for f in forced) if s})
    with tempfile.NamedTemporaryFile("w", suffix=".zsh", delete=False) as f:
        f.write(probe_script(base, forced, styles))
        path = f.name
    try:
        p = subprocess.run([ZSH, "-f", path], capture_output=True, text=True)
    except FileNotFoundError:
        sys.exit(f"{ZSH} not found — the shadow check needs the reference zsh "
                 f"(set PARITY_ZSH to point at it)")
    finally:
        os.unlink(path)
    if p.returncode != 0:
        sys.exit(f"shadow-check probe failed (rc={p.returncode}):\n{p.stderr}")
    rows = []
    for line in p.stdout.splitlines():
        parts = line.split("\t")
        if len(parts) == 3:
            rows.append((parts[0], parts[1], parts[2]))
    return rows


def check_forces(bases: dict[str, list[str]]) -> list[str]:
    """Names of force combos whose override can never win a lookup.

    A combo passes when EVERY style it overrides resolves to the forced value
    at at least one probe context. One is enough: `no-descriptions` pins the
    `:completion:*:descriptions` format, which only the descriptions context
    can exercise, and demanding all of them would reject it for being
    correctly targeted.
    """
    bad = []
    for name, (base_name, forced) in FORCE_COMBOS.items():
        rows = verdicts(bases[base_name], list(forced))
        by_style: dict[str, list[tuple[str, str]]] = {}
        for style, ctx, verdict in rows:
            by_style.setdefault(style, []).append((ctx, verdict))
        for style, seen in sorted(by_style.items()):
            if any(v == "EFFECTIVE" for _, v in seen):
                continue
            bad.append(name)
            print(f"  SHADOWED force-{name}: `{style}` on base `{base_name}` "
                  f"never resolves to the forced value")
            for ctx, verdict in seen:
                print(f"      {verdict:9s} {ctx}")
    return bad


def write(outdir: str, name: str, what: str, lines: list[str]) -> None:
    path = os.path.join(outdir, f"{name}.zsh")
    with open(path, "w") as f:
        f.write(HEADER.format(what=what))
        f.write("\n".join(lines) + "\n")
        # `statements()` keeps only lines starting with "zstyle ", so the
        # fixture's own definitions never reach a combo. Without them every
        # one of these files — `full.zsh` included, which claims to be the
        # complete live config — names zpwrMonthlyCachingPolicy /
        # _megacomplete / _fasd_zsh_word_complete* in a function-valued style
        # without defining them, and compsys quietly takes a different path:
        # an unknown completer is skipped and a missing cache-policy reads as
        # "always rebuild".
        #
        # INLINED, not sourced: a combo has to be a single self-contained
        # file that can be copied or sourced from anywhere. A `source`d
        # sibling also needs a path expression, and every such expression is
        # a footgun — `${0:A:h}` silently resolves against $PWD when
        # FUNCTION_ARGZERO is unset, so the definitions would just quietly
        # not load.
        f.write("\n" + stub_body())


def stub_body() -> str:
    """Definitions from parity_zstyle_stubs.zsh, the source of truth."""
    path = os.path.join(REPO, "scripts", "parity_zstyle_stubs.zsh")
    text = open(path).read()
    return text[text.index("# --- cache-policy"):]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--fixture", default=FIXTURE)
    ap.add_argument("--outdir", default=OUTDIR)
    args = ap.parse_args()

    if not os.path.exists(args.fixture):
        sys.exit(f"fixture not found: {args.fixture}")
    os.makedirs(args.outdir, exist_ok=True)
    outdir = args.outdir

    base = statements(args.fixture)
    bases = {"full": base, "minimal": list(MINIMAL), "none": []}

    unknown = sorted({b for b, _ in FORCE_COMBOS.values()} - set(bases))
    if unknown:
        sys.exit(f"force combo names an unknown base: {', '.join(unknown)}")

    # BEFORE anything is written: a combo whose override loses the lookup is a
    # decoy that certifies the value it claims to test without ever setting it,
    # so it must not reach disk at all.
    print(f"# shadow check ({ZSH}, {len(PROBE_CONTEXTS)} contexts)")
    bad = check_forces(bases)
    if bad:
        print(f"\n{len(bad)} force combo(s) shadowed — nothing written.")
        print("Layer them on a base that does not already set the style at a "
              "higher specificity (`minimal`), or drop the style from the "
              "base with a drop- axis.")
        return 1
    print("  all force overrides win a lookup")

    written = []

    write(outdir, "full", "the complete live config, unmodified", base)
    written.append(("full", "-", len(base)))

    write(outdir, "minimal", "only the two list-shaping styles", list(MINIMAL))
    written.append(("minimal", "-", len(MINIMAL)))

    write(outdir, "none", "no styles at all — compsys defaults", [])
    written.append(("none", "-", 0))

    for axis, style_names in DROP_AXES.items():
        kept = [s for s in base if style_of(s) not in style_names]
        dropped = len(base) - len(kept)
        if dropped == 0:
            # Nothing in the live config sets it, so the combo would be a
            # duplicate of `full`. Say so rather than emitting a decoy.
            print(f"  skip drop-{axis}: live config sets none of {style_names}")
            continue
        write(outdir, f"drop-{axis}",
              f"full config minus every `{'`/`'.join(style_names)}` statement "
              f"({dropped} dropped)",
              kept)
        written.append((f"drop-{axis}", "full", len(kept)))

    for name, (base_name, extra) in FORCE_COMBOS.items():
        stmts = bases[base_name]
        write(outdir, f"force-{name}",
              f"`{base_name}` base, then `{extra[0]}`",
              stmts + ["", "# forced axis"] + list(extra))
        written.append((f"force-{name}", base_name, len(stmts) + len(extra)))

    print(f"{len(written)} combo(s) in {outdir}:")
    for name, base_name, n in written:
        print(f"  {name:28s} {base_name:8s} {n:4d} statement(s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
