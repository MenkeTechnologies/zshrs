#!/bin/sh
""""exec sh -c 'for c in "$SPEC_FUZZ_PYTHON" python3 python3.14 python3.13 python3.12 /opt/homebrew/bin/python3 /usr/local/bin/python3 /usr/bin/python3; do [ -n "$c" ] && command -v "$c" >/dev/null 2>&1 && "$c" -c "import sys, pyte; pyte.Screen(4,4); sys.exit(0 if sys.implementation.name[:1] == chr(99) else 1)" >/dev/null 2>&1 && exec "$c" "$@"; done; echo compsys_spec_fuzz: found no CPython with pyte installed - pip install pyte, or set SPEC_FUZZ_PYTHON >&2; exit 2' sh "$0" "$@" #"""
__doc__ = """compsys_spec_fuzz.py — GENERATIVE, HERMETIC compsys parity fuzzer.

Every other completion-parity harness in this tree draws its cases from
something this host happens to have: `comptab_parity.py` completes real
commands against the user's ~/.zpwr dump, `compsys_parity.py` replays a
curated corpus. Both measure "does zshrs complete *the completers installed
here* the same way zsh does". Neither can reach a corner of the compsys
ENGINE that no installed completer happens to use, and neither reproduces on
a different machine.

This harness inverts that. It SYNTHESISES a random completer definition —
an `_arguments` / `_values` / `_describe` / `_alternative` spec drawn from a
seeded grammar — drops it into a throwaway `$fpath`, and drives the identical
init through a pty on real `zsh -f -i` and on `zshrs --zsh -f -i`, comparing
the rendered screen byte for byte. Nothing but the zsh installation's own
function directory is on `$fpath`; `$HOME`, `$PWD` and the whole environment
are synthesised per case. The same `--seed` regenerates the same specs
anywhere.

That aims the instrument straight at the surface where this codebase's real
completion bugs have lived:

    quote-blind splitting of `(...)` action bodies    (compsys_action_word_split_quoting)
    `{-h,--help}` shared-description rows             (compsys_parity_harness_and_rust_completers)
    `*::rest` CAA_RARGS option handling               (compsys_caa_rargs_option_completion)
    `_describe -O` adding zero matches                (same)
    `_values -s ,` continuation                       (compsys_gsu_param_global_gap)

each of which is one atom of the grammar below rather than an accident of
which package was installed.

Verdicts — none of which is ever softened to make a run look greener:

    PASS        both shells rendered the identical screen
    PASS(err)   both shells rendered the identical screen AND that screen is
                a compsys ERROR — the generated spec was malformed. Counted
                and reported separately, because a run that is 100% PASS(err)
                has tested nothing.
    FAIL        the screens differ, one shell errored and the other did not,
                a shell crashed, or a shell never reached a prompt
    SKIP        the case cannot be compared at all, with a named reason. The
                only reason that exists is `unstable-reference`: real zsh
                disagreed with ITSELF on a re-run, so there is no reference
                to compare against. It is proven by a second reference
                capture, printed in full, never assumed.

A stream-only difference (identical grid, different escape bytes) is always
reported and counted; `--strict-stream` additionally makes it fail.

Typical use:

    scripts/compsys_spec_fuzz.py --seed 1 --cases 8
    scripts/compsys_spec_fuzz.py --seed 1 --cases 200 --jobs 4 --json out.json
    scripts/compsys_spec_fuzz.py --replay target/spec-fuzz-1/case0003.min.zsh
    scripts/compsys_spec_fuzz.py --spec '1:c:((a\\:"add files" b\\:"bench"))'
"""

import argparse
import difflib
import fcntl
import json
import os
import pty
import random
import re
import select
import shlex
import shutil
import signal
import string
import struct
import subprocess
import sys
import termios
import threading
import time

try:
    import pyte
except ImportError:
    sys.exit("compsys_spec_fuzz: pyte not installed (pip install pyte)")


REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SELF = os.path.join("scripts", "compsys_spec_fuzz.py")
SENTINEL = "@SFZ@"

# Serialises pty.fork()+exec: the child is a fork of a threaded interpreter and
# must not touch a lock another thread held at fork time. Free at --jobs 1.
_FORK_LOCK = threading.Lock()


class _TolerantScreen(pyte.Screen):
    """pyte.Screen that survives a private-mode SGR (``CSI ? ... m``).

    pyte forwards ``private=True`` for any CSI containing ``?``, but
    ``select_graphic_rendition`` takes no such keyword and raises TypeError
    mid-feed, aborting a whole sweep. Swallow the flag, render normally.
    (Same fix as scripts/comptab_parity.py; the two harnesses are deliberately
    independent modules, so it is repeated rather than imported.)
    """

    def select_graphic_rendition(self, *attrs, **kwargs):
        kwargs.pop("private", None)
        return super().select_graphic_rendition(*attrs, **kwargs)


# ── keys ─────────────────────────────────────────────────────────────────────

KEYS = {
    "tab": b"\t",
    "btab": b"\x1b[Z",
    "down": b"\x1b[B",
    "up": b"\x1b[A",
    "right": b"\x1b[C",
    "left": b"\x1b[D",
    "ctrl-n": b"\x0e",
    "ctrl-p": b"\x10",
    "ctrl-d": b"\x04",
    "ctrl-c": b"\x03",
    "esc": b"\x1b",
    "space": b" ",
    "enter": b"\r",
}


class UnknownKey(Exception):
    pass


def key_bytes(name):
    """Bytes for one key name, strictly.

    A single character is literal; anything longer must be a defined name. No
    `.get(name, name.encode())` fallback — that silently turns a typo into
    self-inserted characters that look like a completion bug on BOTH shells.
    """
    if name in KEYS:
        return KEYS[name]
    if len(name) == 1:
        return name.encode()
    raise UnknownKey(name)


# ── crash / diagnostic scanning ──────────────────────────────────────────────

CRASH_MARKERS = (
    "panicked at",
    "capacity overflow",
    "run with `RUST_BACKTRACE",   # the panic FOOTER, not the bare env var name
    "Segmentation fault",
    "Abort trap",
    "fatal runtime error",
)

# Complaint-shaped lines. Matched per side, then set-differenced: a message
# BOTH shells print cancels; only a one-sided message is a divergence signal.
# The first pattern carries the calling frames (`_describe:compadd:114: ...`),
# which say WHERE a message came from.
DIAG_PATTERNS = tuple(re.compile(p, re.I) for p in (
    r"\b_[a-z_][a-z0-9_]*(?::[a-z_][a-z0-9_]*)*:\d+:.*",
    r"_arguments:.*",
    r"_values:.*",
    r"_describe:.*",
    r"_alternative:.*",
    r"command not found\b.*",
    r"no such file or directory\b.*",
    r"parse error\b.*",
    r"bad pattern\b.*",
    r"bad substitution\b.*",
    r"bad math expression\b.*",
    r"bad set of key/value pairs\b.*",
    r"not valid in this context\b.*",
    r"function definition file not found\b.*",
    r"invalid argument\b.*",
    r"bad option\b.*",
    r"unknown (?:option|module|signal)\b.*",
    r"no matches found\b.*",
    r"maximum nested function level reached\b.*",
    r"can't (?:open|find|read)\b.*",
    r"not enough arguments\b.*",
    r"too many arguments\b.*",
))

_ANSI_RE = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[@-Z\\-_]")
_SHELLPREFIX_RE = re.compile(r"^(?:zsh|zshrs)(?:\s*\(\w+\))?:\s*")

QUIET, NO_OUTPUT, CAPPED = "quiet", "no-output", "capped"


def diagnostics(raw, pid):
    """Complaint-shaped lines in one shell's output, normalised.

    The shell-name prefix and this child's own pid are stripped so `zsh: foo`
    and `zshrs: foo` are the SAME message; only a genuinely one-sided message
    survives the later set difference.
    """
    text = _ANSI_RE.sub("", raw.decode("utf-8", "replace"))
    if pid:
        text = text.replace(str(pid), "<PID>")
    found = set()
    for line in re.split(r"[\r\n]+", text):
        line = line.strip()
        if not line or SENTINEL in line:
            continue
        line = _SHELLPREFIX_RE.sub("", line)
        for pat in DIAG_PATTERNS:
            m = pat.search(line)
            if m:
                found.add(re.sub(r"\s+", " ", m.group()).strip())
                break
    return found


# ── raw stream tokenisation (for the stream-level diff) ──────────────────────

_TOKEN_RE = re.compile(
    rb"\x1b\[[0-?]*[ -/]*[@-~]"                  # CSI
    rb"|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)"       # OSC
    rb"|\x1b[P^_X][^\x1b]*(?:\x1b\\)?"           # DCS / PM / APC / SOS
    rb"|\x1b[@-Z\\-_]"                           # two-byte ESC
    rb"|[\x00-\x08\x0b-\x1a\x1c-\x1f\x7f]"       # lone control byte
    rb"|[\t\n\r]"                                # layout controls, one each
    rb"|[^\x00-\x1f\x7f]+"                       # printable run
)

_CTRL_NAMES = {0x07: r"\a", 0x08: r"\b", 0x09: r"\t", 0x0a: r"\n",
               0x0b: r"\v", 0x0c: r"\f", 0x0d: r"\r", 0x1b: r"\e", 0x7f: r"\x7f"}


def esc_bytes(b):
    out = []
    for ch in b:
        if ch in _CTRL_NAMES:
            out.append(_CTRL_NAMES[ch])
        elif ch < 0x20:
            out.append("\\x%02x" % ch)
        else:
            out.append(chr(ch))
    return "".join(out)


def tokenize_stream(raw):
    out, i, n = [], 0, len(raw)
    while i < n:
        m = _TOKEN_RE.match(raw, i)
        if m and m.end() > i:
            out.append(esc_bytes(m.group()))
            i = m.end()
        else:
            out.append(esc_bytes(raw[i:i + 1]))
            i += 1
    return out


def stream_diff(ref, test, max_lines=30):
    a, b = tokenize_stream(ref), tokenize_stream(test)
    sm = difflib.SequenceMatcher(None, a, b, autojunk=False)
    lines = []
    for tag, i1, i2, j1, j2 in sm.get_opcodes():
        if tag == "equal":
            continue
        if len(lines) >= max_lines:
            lines.append("    ... (truncated)")
            break
        ctx = "".join(a[max(0, i1 - 2):i1])
        if ctx:
            lines.append("  after %s" % ctx[-60:])
        if tag in ("replace", "delete"):
            lines.append("    zsh   - %s" % "".join(a[i1:i2])[:180])
        if tag in ("replace", "insert"):
            lines.append("    zshrs + %s" % "".join(b[j1:j2])[:180])
    return lines


# ═════════════════════════════════════════════════════════════════════════════
# the grammar
# ═════════════════════════════════════════════════════════════════════════════
#
# Every generated construct below exists because the compsys engine has a
# distinct code path for it, and several exist because that path has already
# been wrong once in this codebase (citations in the module docstring).

# Description / value text. Deliberately hostile: each entry carries at least
# one character with meaning to the spec parser, the quoting layer, or the
# display-width math. The reason `((a\:"add files to archive"))` was worth
# generating is that the SAME string has to survive `_arguments` spec parsing,
# `compdescribe` column layout, and the shell quoting around all of it.
NASTY_WORDS = [
    "add files",
    "keep going",
    "it's here",
    'say "hi" now',
    "back\\slash",
    "100% done",
    "$HOME path",
    "`cmd` sub",
    "[bracket] in",
    "a:b colon",
    "semi;colon",
    "amp&and",
    "star*glob",
    "pipe|bar",
    "tilde~home",
    "paren(s) here",
    "brace{s} here",
    "eq=sign",
    "hash#mark",
    "at@sign",
    "plain",
    "two  spaces",
    "dash-dash --opt",
    "trailing ",
]

NON_ASCII_WORDS = [
    "h\u00e9llo w\u00f6rld",
    "caf\u00e9 na\u00efve",
    "\u65e5\u672c\u8a9e desc",
    "arrow \u2192 here",
]

# Plain identifiers, used where the spec grammar cannot carry a space.
PLAIN_WORDS = ["alpha", "beta", "gamma", "delta", "eps", "zeta", "eta", "theta",
               "one", "two", "three", "four", "five", "six"]

LONG_WORDS = ["--long-option-name", "verylongvaluename0123456789", "x"]


def zq(s):
    """zsh single-quoted form of an arbitrary string."""
    return "'" + s.replace("'", "'\\''") + "'"


def esc_bracket(s):
    """Description text going between `[` and `]` in a spec."""
    return s.replace("\\", "\\\\").replace("]", "\\]").replace("[", "\\[")


def emit_atom(a):
    """One spec atom as it appears on the generated command line.

    Almost every atom is a single quoted word. The `{-h,--help}'[desc]'`
    shared-description form is the exception: the brace list is expanded by the
    SHELL, so quoting it whole would hand `_arguments` one option literally
    named `{-h,--help}` and never exercise the shared-description path at all.
    """
    if a.startswith("{") and "}" in a:
        i = a.index("}")
        return a[:i + 1] + zq(a[i + 1:])
    return zq(a)


def esc_colon(s):
    """Text going into a `:`-delimited spec field."""
    return s.replace("\\", "\\\\").replace(":", "\\:")


class Gen:
    """Seeded generator state for ONE case.

    `rng` is seeded from (seed, index) alone, so `--seed N` plus a case index
    reproduces a generation exactly no matter how many jobs ran or in what
    order.
    """

    def __init__(self, seed, idx, non_ascii):
        self.rng = random.Random("compsys_spec_fuzz:%d:%d" % (seed, idx))
        self.non_ascii = non_ascii
        self.letters = list(string.ascii_lowercase)
        self.rng.shuffle(self.letters)
        self.li = 0
        self.wi = 0
        self.words = list(PLAIN_WORDS)
        self.rng.shuffle(self.words)

    def letter(self):
        c = self.letters[self.li % len(self.letters)]
        self.li += 1
        return c

    def word(self):
        w = self.words[self.wi % len(self.words)]
        self.wi += 1
        return w

    def desc(self):
        pool = NASTY_WORDS + (NON_ASCII_WORDS if self.non_ascii else [])
        n = self.rng.choice((1, 1, 2))
        return " ".join(self.rng.choice(pool) for _ in range(n)).strip() or "d"

    def msg(self):
        return self.rng.choice(["file", "value", "arg name", "what:ever", "n"])

    # ── actions ──────────────────────────────────────────────────────────────

    def wordlist(self):
        n = self.rng.randint(2, 4)
        return "(%s)" % " ".join(self.word() for _ in range(n))

    def described_list(self):
        """`((a\\:"desc with spaces" b\\:'other desc'))`.

        The construct that broke `_arguments`/`_alternative`/`_values` action
        splitting: the body is one shell word containing spaces and BOTH quote
        flavours, and a splitter that is blind to quoting tears it apart.
        """
        n = self.rng.randint(2, 3)
        parts = []
        for _ in range(n):
            item = self.word()
            d = self.desc()
            q = self.rng.choice(('"', "'"))
            if q == "'":
                # A single quote inside a single-quoted spec has to survive two
                # levels; keep the inner text free of `'` for the `'...'` form.
                d = d.replace("'", "")
            else:
                d = d.replace('"', "")
            parts.append("%s\\:%s%s%s" % (esc_colon(item), q, d, q))
        return "((%s))" % " ".join(parts)

    def lead(self, act):
        """Half the function actions get a LEADING SPACE.

        `_arguments` has two calling conventions for a function action and they
        are one space apart (Src/../Completion/Base/Utility/_arguments:449 vs
        :465): a leading space means "just call it", no leading space means
        "call it with the description arguments" —
        `"$action[1]" "$subopts[@]" "$expl[@]" "${(@)action[2,-1]}"`. A shell
        that drops `$expl` on the second form loses the group/description
        options for every match the action adds, and no spec that only ever
        used one convention would show it.
        """
        return (" " + act) if self.rng.random() < 0.5 else act

    def action(self, states, helpers, cmd, allow_state=True):
        choices = ["wordlist", "described", "files", "alternative",
                   "values-brace", "helper", "nothing"]
        if allow_state:
            choices += ["state", "state"]
        kind = self.rng.choice(choices)
        if kind == "wordlist":
            return self.wordlist()
        if kind == "described":
            return self.described_list()
        if kind == "files":
            return self.lead(self.rng.choice(
                ["_files", "_files -/", "_files -g '*.txt'"]))
        if kind == "alternative":
            return self.lead("_alternative %s %s" % (
                zq("%s:%s:%s" % (self.word(), esc_colon(self.desc()), self.wordlist())),
                zq("%s:%s:_files" % (self.word(), esc_colon(self.desc())))))
        if kind == "values-brace":
            return "{ _values %s %s %s }" % (
                zq(self.desc()),
                zq("%s[%s]" % (self.word(), esc_bracket(self.desc()))),
                zq("%s[%s]" % (self.word(), esc_bracket(self.desc()))))
        if kind == "helper":
            h = "_%s_helper%d" % (cmd, len(helpers))
            helpers.append(h)
            return self.lead(h)
        if kind == "state":
            s = "st%d" % len(states)
            states.append(s)
            return self.rng.choice(["->%s" % s, "->%s" % s])
        return ""


# ── one generated case ───────────────────────────────────────────────────────

class Case:
    """A generated (or replayed, or injected) completer + the line to type."""

    def __init__(self, idx, seed, cmd, kind, atoms, flags, extra, buffer, keys):
        self.idx = idx
        self.seed = seed
        self.cmd = cmd
        self.kind = kind
        self.atoms = list(atoms)     # reducible: spec strings / array entries
        self.flags = list(flags)     # reducible: harness-level flag tokens
        self.extra = dict(extra)     # states, helpers, header, arrays
        self.buffer = buffer
        self.keys = list(keys)
        self.body_override = None    # set only by --replay

    @property
    def name(self):
        return "case%04d" % self.idx if self.idx >= 0 else "adhoc"

    def clone(self, atoms=None, flags=None):
        c = Case(self.idx, self.seed, self.cmd, self.kind,
                 self.atoms if atoms is None else atoms,
                 self.flags if flags is None else flags,
                 self.extra, self.buffer, self.keys)
        c.body_override = self.body_override
        return c

    # The completer file dropped into the throwaway fpath.
    def completer(self):
        if self.body_override is not None:
            return self.body_override
        out = ["#compdef %s" % self.cmd, ""]
        for h in self.extra.get("helpers", []):
            out.append("%s() { compadd -- %s }" % (h, " ".join(PLAIN_WORDS[:3])))
        if self.extra.get("helpers"):
            out.append("")
        if self.kind == "arguments":
            out += self._arguments_body()
        elif self.kind == "values":
            out += self._values_body()
        elif self.kind == "describe":
            out += self._describe_body()
        else:
            out += self._alternative_body()
        return "\n".join(out) + "\n"

    def _arguments_body(self):
        out = ["local curcontext=\"$curcontext\" state state_descr line ret=1",
               "typeset -A opt_args", ""]
        flags = " ".join(self.flags)
        if not self.atoms:
            out.append("_arguments %s && ret=0" % flags)
        else:
            out.append("_arguments %s \\" % flags)
            for i, a in enumerate(self.atoms):
                tail = " \\" if i < len(self.atoms) - 1 else " && ret=0"
                out.append("  %s%s" % (emit_atom(a), tail))
        states = self.extra.get("states", [])
        if states:
            out.append("")
            out.append("case $state in")
            for s, act in states:
                out.append("  (%s) %s && ret=0 ;;" % (s, act))
            out.append("esac")
        out.append("")
        out.append("return ret")
        return out

    def _values_body(self):
        flags = " ".join(self.flags)
        parts = [zq(self.extra.get("header", "value"))] + [zq(a) for a in self.atoms]
        return ["_values %s %s" % (flags, " ".join(parts))]

    def _describe_body(self):
        out = ["local -a _m _d"]
        out.append("_m=( %s )" % " ".join(zq(a) for a in self.atoms))
        if self.extra.get("two_array"):
            out.append("_d=( %s )" % " ".join(
                zq(a.split(":", 1)[1] if ":" in a else a) for a in self.atoms))
        flags = " ".join(self.flags)
        arrays = "_m _d" if self.extra.get("two_array") else "_m"
        out.append("_describe %s %s %s" % (flags, zq(self.extra.get("header", "hdr")), arrays))
        return out

    def _alternative_body(self):
        if not self.atoms:
            return ["_alternative"]
        out = ["_alternative \\"]
        for i, a in enumerate(self.atoms):
            out.append("  %s%s" % (zq(a), " \\" if i < len(self.atoms) - 1 else ""))
        return out


def generate(seed, idx, args):
    g = Gen(seed, idx, args.non_ascii)
    cmd = "fzc%04d" % (idx if idx >= 0 else 9999)
    kinds = args.kinds
    kind = g.rng.choice(kinds)
    states = []
    helpers = []
    atoms = []
    flags = []
    extra = {}

    if kind == "arguments":
        for f in ("-s", "-S", "-C", "-w", "-W"):
            if g.rng.random() < 0.35:
                flags.append(f)
        if g.rng.random() < 0.15:
            flags.append('-A "-*"')
        n = g.rng.randint(2, 6)
        made_rest = False
        made_num = 0
        for _ in range(n):
            forms = ["flag", "flag", "optarg", "eqarg", "excl", "repeat",
                     "shared", "plusflag", "num"]
            if not made_rest:
                forms += ["rest", "rest2", "rest3"]
            form = g.rng.choice(forms)
            if form == "flag":
                atoms.append("-%s[%s]" % (g.letter(), esc_bracket(g.desc())))
            elif form == "plusflag":
                atoms.append("+%s[%s]" % (g.letter(), esc_bracket(g.desc())))
            elif form == "optarg":
                atoms.append("-%s+[%s]:%s:%s" % (
                    g.letter(), esc_bracket(g.desc()), esc_colon(g.msg()),
                    g.action(states, helpers, cmd)))
            elif form == "eqarg":
                atoms.append("--%s=-[%s]:%s:%s" % (
                    g.word(), esc_bracket(g.desc()), esc_colon(g.msg()),
                    g.action(states, helpers, cmd)))
            elif form == "excl":
                a, b = g.letter(), g.word()
                atoms.append("(-%s --%s)-%s[%s]" % (a, b, a, esc_bracket(g.desc())))
                atoms.append("(-%s --%s)--%s[%s]" % (a, b, b, esc_bracket(g.desc())))
            elif form == "repeat":
                atoms.append("*-%s[%s]" % (g.letter(), esc_bracket(g.desc())))
            elif form == "shared":
                # `{-h,--help}[desc]` — one description shared by two names.
                # compdescribe has to interleave the alias names with the
                # description into ONE row; getting that wrong is a pinned bug.
                c, w = g.letter(), g.word()
                atoms.append("{-%s,--%s}[%s]" % (c, w, esc_bracket(g.desc())))
            elif form == "num":
                made_num += 1
                atoms.append("%d:%s:%s" % (made_num, esc_colon(g.msg()),
                                           g.action(states, helpers, cmd)))
            elif form in ("rest", "rest2", "rest3"):
                made_rest = True
                colons = {"rest": ":", "rest2": "::", "rest3": ":::"}[form]
                atoms.append("*%s%s:%s" % (colons, esc_colon(g.msg()),
                                           g.action(states, helpers, cmd,
                                                    allow_state=(form == "rest"))))
        extra["states"] = [(s, g.action([], helpers, cmd, allow_state=False) or
                            "_files") for s in states]

    elif kind == "values":
        if g.rng.random() < 0.6:
            flags.append("-s " + zq(g.rng.choice([",", ":", "+"])))
        if g.rng.random() < 0.3:
            flags.append("-S " + zq(g.rng.choice(["=", ":"])))
        if g.rng.random() < 0.2:
            flags.append("-w")
        extra["header"] = g.desc()
        for _ in range(g.rng.randint(2, 5)):
            form = g.rng.choice(["plain", "plain", "witharg", "excl"])
            if form == "plain":
                atoms.append("%s[%s]" % (g.word(), esc_bracket(g.desc())))
            elif form == "witharg":
                atoms.append("%s[%s]:%s:%s" % (
                    g.word(), esc_bracket(g.desc()), esc_colon(g.msg()),
                    g.action([], helpers, cmd, allow_state=False)))
            else:
                a, b = g.word(), g.word()
                atoms.append("(%s)%s[%s]" % (a, b, esc_bracket(g.desc())))

    elif kind == "describe":
        for f in ("-o", "-O", "-V", "-1", "-2", "-J", "-x"):
            if g.rng.random() < 0.18:
                flags.append(f)
        if g.rng.random() < 0.4:
            flags.append("-t " + g.word())
        extra["header"] = g.desc()
        extra["two_array"] = g.rng.random() < 0.35
        opts = "-o" in flags or "-O" in flags
        shared = g.desc()          # deliberately reused: shared-description rows
        for i in range(g.rng.randint(2, 5)):
            name = ("-" + g.letter()) if opts else g.word()
            d = shared if g.rng.random() < 0.45 else g.desc()
            atoms.append("%s:%s" % (esc_colon(name), d))

    else:  # alternative
        for _ in range(g.rng.randint(2, 4)):
            act = g.action([], helpers, cmd, allow_state=False) or "_files"
            atoms.append("%s:%s:%s" % (g.word(), esc_colon(g.desc()), act))

    extra["helpers"] = helpers

    prefix = g.rng.choice(["", "", "-", "--", "-" + g.letters[0],
                           PLAIN_WORDS[0][0], "a", "x"])
    buf = "%s %s" % (cmd, prefix)
    return Case(idx, seed, cmd, kind, atoms, flags, extra, buf, args.keys)


# ═════════════════════════════════════════════════════════════════════════════
# hermetic environment
# ═════════════════════════════════════════════════════════════════════════════

def hermetic_fpath(zsh):
    """The fpath a completely clean `zsh -f` compiles in.

    Read from `env -i` on purpose: this session exports FPATH (the user's ~50
    plugin dirs), and `zsh -f` honours an inherited FPATH, so reading it any
    other way silently drags the whole host completion set into a harness whose
    entire point is that it does not depend on one.
    """
    out = subprocess.run([zsh, "-f", "-c", "print -rl -- $fpath"],
                         capture_output=True, text=True, timeout=20, env={}).stdout
    dirs = [d for d in out.splitlines() if d and os.path.isdir(d)]
    if not dirs:
        sys.exit("compsys_spec_fuzz: %s reports an empty fpath — cannot build a "
                 "hermetic compsys environment" % zsh)
    return dirs


# Files the `_files` actions complete against. Fixed content so a `_files`
# action is a deterministic comparison instead of whatever the cwd holds.
WORK_FILES = ["alpha.txt", "beta.txt", "gamma.log", "delta.conf"]
WORK_DIRS = ["adir", "bdir"]


def build_case_dir(root, case, fpath_dirs):
    """Materialise one case's throwaway $fpath, $HOME, cwd and init file."""
    d = os.path.join(root, case.name)
    fp = os.path.join(d, "fpath")
    work = os.path.join(d, "work")
    os.makedirs(fp, exist_ok=True)
    os.makedirs(work, exist_ok=True)
    for f in WORK_FILES:
        p = os.path.join(work, f)
        if not os.path.exists(p):
            open(p, "w").close()
    for sub in WORK_DIRS:
        os.makedirs(os.path.join(work, sub), exist_ok=True)

    with open(os.path.join(fp, "_" + case.cmd), "w") as f:
        f.write(case.completer())

    init = os.path.join(d, "init.zsh")
    with open(init, "w") as f:
        f.write(
            "# generated by %s — seed %d %s\n"
            "fpath=( %s )\n"
            "PROMPT='%s '\n"
            "RPROMPT=''\n"
            "PS2='> '\n"
            "setopt no_beep\n"
            "builtin cd %s\n"
            "autoload -Uz compinit\n"
            "compinit -u -D\n"
            "print -u2 ''\n"
            % (SELF, case.seed, case.name,
               " ".join(shlex.quote(p) for p in [fp] + fpath_dirs),
               SENTINEL, shlex.quote(work)))
    return d, init


def child_env(case_dir, non_ascii):
    """The environment BOTH shells get — built from scratch, nothing inherited.

    HOME and cwd are inside the case directory, so `~` and `_files` are
    hermetic too. PATH is the parent's only so the harness can find the shells'
    own helpers; no completion in the generated grammar shells out to a
    host command.
    """
    env = {
        "TERM": "xterm-256color",
        "LANG": "en_US.UTF-8" if non_ascii else "C",
        "LC_ALL": "en_US.UTF-8" if non_ascii else "C",
        "PATH": "/usr/bin:/bin:/usr/sbin:/sbin",
        "HOME": case_dir,
        # zshrs's autosuggest / syntax-highlight layer has no zsh counterpart;
        # its ghost text would diff on every case. Silences that LAYER only —
        # the completion engine under test is untouched.
        "ZSHRS_NATIVE_ZLE_FX": "0",
        # zshrs ships ~145 builtins zsh does not; any listing that enumerates
        # $builtins diverges by construction. Hides the extension names for the
        # comparison only; dispatch is unchanged.
        "ZSHRS_HIDE_EXT_BUILTINS": "1",
        "RUST_BACKTRACE": "1",
    }
    for k in ("ZSHRS_LOG", "RUST_LOG"):
        if k in os.environ:
            env[k] = os.environ[k]
    return env


# ═════════════════════════════════════════════════════════════════════════════
# one shell on one pty
# ═════════════════════════════════════════════════════════════════════════════

class Session:
    def __init__(self, argv, env, rows, cols, settle_ms, cwd):
        self.rows, self.cols = rows, cols
        self.settle = settle_ms / 1000.0
        self.screen = _TolerantScreen(cols, rows)
        self.stream = pyte.ByteStream(self.screen)
        self.raw = bytearray()
        self.mark = 0
        self.dead = False
        self.status = None
        self.events = []
        with _FORK_LOCK:
            self.pid, self.fd = pty.fork()
            if self.pid == 0:
                try:
                    os.chdir(cwd)
                    os.execvpe(argv[0], argv, env)
                except BaseException as exc:   # pragma: no cover — child only
                    try:
                        os.write(2, ("exec failed: %s\n" % exc).encode())
                    finally:
                        os._exit(127)
        try:
            fcntl.ioctl(self.fd, termios.TIOCSWINSZ,
                        struct.pack("HHHH", rows, cols, 0, 0))
        except OSError:
            self.close()
            raise

    def _read_once(self, timeout):
        r, _, _ = select.select([self.fd], [], [], timeout)
        if not r:
            return False
        try:
            data = os.read(self.fd, 65536)
        except OSError:
            self.dead = True
            return False
        if not data:
            self.dead = True
            return False
        self.raw.extend(data)
        self.stream.feed(data)
        return True

    def settle_out(self, max_wait=10.0, first_wait=6.0, phase="?"):
        """Read until the screen stops changing; record HOW the wait ended.

        QUIET is the only outcome meaning "this screen is final". CAPPED (still
        flowing at the budget) and NO_OUTPUT (nothing ever arrived) are kept so
        a verdict taken under them can be reported as worth less.
        """
        start = last = time.monotonic()
        seen = False
        first = None
        while True:
            now = time.monotonic()
            if now - start > max_wait:
                outcome = CAPPED if seen else NO_OUTPUT
                self.events.append((phase, outcome, first, now - start))
                return outcome
            got = self._read_once(0.05)
            now = time.monotonic()
            if got:
                if first is None:
                    first = now - start
                seen, last = True, now
            elif not seen:
                if now - start > first_wait:
                    self.events.append((phase, NO_OUTPUT, first, now - start))
                    return NO_OUTPUT
            elif now - last >= self.settle:
                self.events.append((phase, QUIET, first, now - start))
                return QUIET

    def wait_prompt(self, timeout=40.0):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self._read_once(0.05)
            if any(SENTINEL in line for line in self.screen.display):
                return True
            if self.dead:
                return False
        return False

    def send(self, data):
        try:
            os.write(self.fd, data)
        except OSError:
            self.dead = True

    def clear(self):
        self.send(b"\x0c")
        self.settle_out(max_wait=3.0, first_wait=2.0, phase="clear")

    def grid(self):
        rows = [r.rstrip() for r in self.screen.display]
        while rows and rows[-1] == "":
            rows.pop()
        return [self._mask(r) for r in rows]

    def attrs(self, nrows):
        buf = self.screen.buffer
        out = []
        for y in range(min(nrows, self.rows)):
            row = buf[y]
            out.append(tuple(
                (c.fg, c.bg, c.bold, c.italics, c.underscore,
                 c.strikethrough, c.reverse, c.blink)
                for c in (row[x] for x in range(self.cols))
            ))
        return out

    def cursor(self):
        return (self.screen.cursor.y, self.screen.cursor.x)

    def _mask(self, row):
        """Replace THIS child's own pid with a stable token.

        Two live processes cannot share a pid, so a screen that shows `$$`
        could never compare equal however correct zshrs is. Only this session's
        exact pid is substituted — not a general digit mask; any other number
        on the screen still has to match byte for byte.
        """
        return row.replace(str(self.pid), "<PID>") if self.pid else row

    def crashed(self):
        text = self.raw.decode("utf-8", "replace")
        return [m for m in CRASH_MARKERS if m in text]

    def close(self):
        for b in (b"\x03", b"\x04"):
            try:
                os.write(self.fd, b)
            except OSError:
                pass
        try:
            os.close(self.fd)
        except OSError:
            pass
        for sig in (signal.SIGHUP, signal.SIGTERM, signal.SIGKILL):
            try:
                pid, st = os.waitpid(self.pid, os.WNOHANG)
                if pid == self.pid:
                    self.status = st
                    return
                os.kill(self.pid, sig)
            except OSError:
                return
            for _ in range(20):
                try:
                    pid, st = os.waitpid(self.pid, os.WNOHANG)
                    if pid == self.pid:
                        self.status = st
                        return
                except OSError:
                    return
                select.select([], [], [], 0.025)


def exit_note(status):
    if status is None:
        return None
    if os.WIFSIGNALED(status):
        sig = os.WTERMSIG(status)
        if sig in (signal.SIGHUP, signal.SIGTERM, signal.SIGKILL):
            return None                      # our own teardown
        try:
            name = signal.Signals(sig).name
        except ValueError:
            name = "?"
        return "killed by signal %d (%s)" % (sig, name)
    if os.WIFEXITED(status) and os.WEXITSTATUS(status) not in (0, 1):
        return "exited %d" % os.WEXITSTATUS(status)
    return None


def _secs(v):
    return "none" if v is None else "%.2fs" % v


class Capture:
    def __init__(self, grid=None, reason=None, crash=None, attrs=None,
                 cursor=None, raw=b"", diags=None, events=(), status=None):
        self.grid = grid
        self.reason = reason
        self.crash = crash or []
        self.attrs = attrs or []
        self.cursor = cursor
        self.raw = raw
        self.diags = diags or set()
        self.events = list(events)
        self.status = status

    def warnings(self):
        out = []
        for phase, outcome, first, waited in self.events:
            if outcome == CAPPED:
                out.append("settle capped after %s — screen may be mid-render "
                           "(first byte %s, waited %.1fs)"
                           % (phase, _secs(first), waited))
            elif outcome == NO_OUTPUT and phase.startswith("key "):
                out.append("no output at all after %s (waited %.1fs)"
                           % (phase, waited))
        note = exit_note(self.status)
        if note:
            out.append("child %s" % note)
        return out


def capture(argv, env, args, init_file, cwd, buf, keys):
    sess = Session(argv, env, args.rows, args.cols, args.settle, cwd)
    result = Capture(reason="capture aborted before any screen was taken")
    try:
        sess.settle_out(max_wait=4.0, first_wait=3.0, phase="boot")
        sess.send(("source %s\n" % shlex.quote(init_file)).encode())
        if not sess.wait_prompt(timeout=args.boot_timeout):
            result = Capture(reason="never reached a prompt (compinit hang or crash)",
                             crash=sess.crashed())
        else:
            sess.clear()
            sess.mark = len(sess.raw)
            if buf:
                sess.send(buf.encode())
                sess.settle_out(max_wait=3.0, first_wait=1.0, phase="buffer")
            for k in keys:
                sess.send(key_bytes(k))
                sess.settle_out(max_wait=15.0, first_wait=10.0, phase="key %r" % k)
            sess.settle_out(max_wait=3.0, first_wait=0.6, phase="final")
            rows = sess.grid()
            result = Capture(grid=rows, attrs=sess.attrs(len(rows)),
                             cursor=sess.cursor(),
                             raw=bytes(sess.raw[sess.mark:]),
                             diags=diagnostics(sess.raw[sess.mark:], sess.pid))
            crash = sess.crashed()
            if crash:
                result.reason = "crashed: " + ", ".join(crash)
                result.crash = crash
            elif sess.dead:
                result.reason = "shell exited mid-case"
    except Exception as exc:
        result = Capture(reason="harness error during capture: %r" % (exc,))
    finally:
        sess.close()
        result.events = list(sess.events)
        result.status = sess.status
    return result


# ═════════════════════════════════════════════════════════════════════════════
# comparison
# ═════════════════════════════════════════════════════════════════════════════

def diff_grids(ref, test):
    n = max(len(ref), len(test))
    out = []
    for i in range(n):
        a = ref[i] if i < len(ref) else "<absent>"
        b = test[i] if i < len(test) else "<absent>"
        if a != b:
            out.append((i, a, b))
    return out


def first_diff_cell(a, b):
    for i, (x, y) in enumerate(zip(a, b)):
        if x != y:
            return i
    return min(len(a), len(b))


def attr_diff_rows(ref, test):
    return [i for i in range(min(len(ref), len(test))) if ref[i] != test[i]]


PASS, PASS_ERR, FAIL, SKIP = "PASS", "PASS(err)", "FAIL", "SKIP"

# Failure classes worth reducing, and worth proving the reference is stable for.
# `ref-broken` / `test-broken` are excluded: a shell that never booted or
# crashed is not a property of any one spec atom, so delta-debugging it would
# just blame whichever atom happened to survive.
SHRINKABLE = ("grid-diff", "one-sided-error", "strict")


class Verdict:
    def __init__(self, case, status, detail, ref, test, diffs, category=""):
        self.case = case
        self.status = status
        self.detail = detail
        self.ref = ref
        self.test = test
        self.diffs = diffs or []
        self.category = category      # coarse failure class, used by the shrinker
        self.stream_only = False
        self.attr_rows = []
        self.cursor_differs = False
        self.ref_only_diags = set()
        self.test_only_diags = set()
        self.duration = 0.0
        self.fixture = None
        if ref and test and ref.grid is not None and test.grid is not None:
            text_diff = {i for i, _a, _b in self.diffs}
            self.attr_rows = [r for r in attr_diff_rows(ref.attrs, test.attrs)
                              if r not in text_diff]
            self.cursor_differs = ref.cursor != test.cursor
            self.ref_only_diags = ref.diags - test.diags
            self.test_only_diags = test.diags - ref.diags
            self.stream_only = (not self.diffs) and ref.raw != test.raw


def render(rows):
    return "\n".join("  %2d| %s" % (i, r) for i, r in enumerate(rows)) or "  <empty>"


def judge(args, case, ref, test):
    """Returns (status, detail, diffs, category).

    Ordering matters. A shell that never booted or crashed is a FAIL naming
    that, never a quiet skip. `PASS(err)` is reserved for the case where the
    two shells rendered the IDENTICAL screen and that screen contains a compsys
    diagnostic — the generated spec was malformed and both shells agreed about
    it, which is a real (if weak) parity observation and is counted apart from
    a clean pass so a run cannot look green on nothing but garbage specs.
    """
    if ref.reason:
        return FAIL, "reference zsh: %s" % ref.reason, None, "ref-broken"
    if test.reason:
        return FAIL, "zshrs: %s" % test.reason, None, "test-broken"
    d = diff_grids(ref.grid, test.grid)
    if d:
        cat = "grid-diff"
        if bool(ref.diags) != bool(test.diags):
            cat = "one-sided-error"
        return FAIL, "%d row(s) differ" % len(d), d, cat
    extra = []
    if args.compare_attrs:
        rows = attr_diff_rows(ref.attrs, test.attrs)
        if rows:
            extra.append("%d row(s) differ in SGR attributes only" % len(rows))
    if args.strict_cursor and ref.cursor != test.cursor:
        extra.append("cursor %s vs %s" % (ref.cursor, test.cursor))
    if args.strict_stream and ref.raw != test.raw:
        extra.append("raw escape streams differ")
    if extra:
        return FAIL, "; ".join(extra), [], "strict"
    if ref.diags:
        return PASS_ERR, "both shells report: %s" % "; ".join(sorted(ref.diags))[:160], [], ""
    return PASS, "", [], ""


def run_case(args, root, fpath_dirs, case):
    """Drive one case on both shells and judge it."""
    t0 = time.monotonic()
    cdir, init = build_case_dir(root, case, fpath_dirs)
    env = child_env(cdir, args.non_ascii)
    work = os.path.join(cdir, "work")
    ref = capture([args.zsh, "-f", "-i"], env, args, init, work, case.buffer, case.keys)
    test = capture(args.test_argv, env, args, init, work, case.buffer, case.keys)
    status, detail, diffs, cat = judge(args, case, ref, test)

    v = Verdict(case, status, detail, ref, test, diffs, cat)

    # A failure is only meaningful if the REFERENCE is stable. Prove it rather
    # than assume it: re-run zsh against itself. Disagreeing with itself is the
    # one and only condition under which a case is skipped, and the second
    # reference capture is printed so the claim is checkable.
    if status == FAIL and args.self_check and cat in SHRINKABLE:
        ref2 = capture([args.zsh, "-f", "-i"], env, args, init, work,
                       case.buffer, case.keys)
        if ref2.reason or (ref.grid != ref2.grid):
            v.status = SKIP
            v.category = "unstable-reference"
            v.detail = ("real zsh disagreed with itself on a re-run "
                        "(%s) — nothing to compare against"
                        % (ref2.reason or "%d row(s) differ between the two zsh runs"
                           % len(diff_grids(ref.grid, ref2.grid))))
            v.ref2 = ref2
    v.duration = time.monotonic() - t0
    return v


# ═════════════════════════════════════════════════════════════════════════════
# shrinking
# ═════════════════════════════════════════════════════════════════════════════

def ddmin(units, still_bad, max_probes):
    """Classic delta-debugging minimisation over a list of units.

    `still_bad(subset)` must return True when the subset still reproduces. The
    probe budget is a hard cap: each probe is two full pty sessions.
    """
    probes = [0]

    def test(sub):
        if probes[0] >= max_probes:
            raise StopIteration
        probes[0] += 1
        return still_bad(sub)

    cur = list(units)
    n = 2
    try:
        while len(cur) >= 2:
            chunk = max(1, len(cur) // n)
            parts = [cur[i:i + chunk] for i in range(0, len(cur), chunk)]
            reduced = False
            for i, p in enumerate(parts):
                comp = [u for j, q in enumerate(parts) if j != i for u in q]
                if comp and test(comp):
                    cur, n, reduced = comp, max(n - 1, 2), True
                    break
            if not reduced:
                if n >= len(cur):
                    break
                n = min(len(cur), n * 2)
    except StopIteration:
        pass
    return cur, probes[0]


def shrink_case(args, root, fpath_dirs, case, category):
    """Reduce a diverging case to a minimal set of spec atoms and flags.

    Units are the individual `_arguments` specs / `_values` values / `_describe`
    entries plus each harness-level flag, so the reproducer that comes out is a
    sentence about ONE construct rather than a wall of six.

    The reduction must keep diverging for the SAME coarse reason; a subset that
    merely becomes malformed and errors on one side is not accepted as a
    reduction of a rendering divergence.
    """
    units = ([("atom", i) for i in range(len(case.atoms))] +
             [("flag", i) for i in range(len(case.flags))])

    def build(sub):
        keep_a = sorted(i for k, i in sub if k == "atom")
        keep_f = sorted(i for k, i in sub if k == "flag")
        c = case.clone([case.atoms[i] for i in keep_a],
                       [case.flags[i] for i in keep_f])
        c.idx = case.idx
        return c

    def still_bad(sub):
        c = build(sub)
        # Shrink probes run in their own directory so the original case's
        # materialised fixture is never overwritten mid-reduction.
        v = run_case(args, os.path.join(root, "shrink"), fpath_dirs, c)
        return v.status == FAIL and v.category == category

    minimal, probes = ddmin(units, still_bad, args.shrink_probes)
    return build(minimal), probes


# ═════════════════════════════════════════════════════════════════════════════
# fixtures (write / replay)
# ═════════════════════════════════════════════════════════════════════════════

HEREDOC = "SPEC_FUZZ_COMPLETER"


def write_fixture(path, args, case, note):
    """A runnable .zsh reproducer.

    Running it materialises the same throwaway fpath and drops into an
    interactive shell with the completer loaded; `--replay`ing it re-runs the
    exact comparison the harness ran. The header carries the replay command so
    the file is self-describing when it turns up in `target/` weeks later.
    """
    os.makedirs(os.path.dirname(path), exist_ok=True)
    rel = os.path.relpath(path, REPO)
    body = case.completer()
    with open(path, "w") as f:
        f.write(
            "#!/usr/bin/env zsh\n"
            "# compsys_spec_fuzz minimal reproducer — %s\n"
            "# %s\n"
            "#\n"
            "# replay (authoritative — re-runs the exact parity comparison):\n"
            "#     %s --replay %s\n"
            "# manual (drops you in a shell with this completer loaded):\n"
            "#     zsh %s     # then type:  %s   and press: %s\n"
            "#\n"
            "# @seed %d\n"
            "# @case %d\n"
            "# @cmd %s\n"
            "# @kind %s\n"
            "# @buffer %s\n"
            "# @keys %s\n"
            "# @shell %s\n"
            % (case.name, note, SELF, rel, rel, case.buffer, ",".join(case.keys),
               case.seed, case.idx, case.cmd, case.kind, case.buffer,
               ",".join(case.keys), " ".join(args.test_argv)))
        f.write("\nemulate -L zsh\ntypeset _d=${TMPDIR:-/tmp}/spec-fuzz-repro.$$\n"
                "mkdir -p $_d/fpath\n")
        f.write("cat >$_d/fpath/_%s <<'%s'\n%s%s\n" % (case.cmd, HEREDOC, body, HEREDOC))
        f.write(
            "cat >$_d/.zshrc <<RC\n"
            "fpath=( \\$_d/fpath %s )\n"
            "PROMPT='%s '\n"
            "autoload -Uz compinit\n"
            "compinit -u -D\n"
            "RC\n"
            % (" ".join(shlex.quote(p) for p in args.fpath_dirs), SENTINEL))
        f.write("print -r -- \"# fpath dir: $_d/fpath   buffer: %s\"\n"
                % case.buffer.replace('"', '\\"'))
        f.write("ZDOTDIR=$_d exec ${SPEC_FUZZ_SHELL:-zsh} -i\n")
    os.chmod(path, 0o755)
    return path


def read_fixture(path):
    meta = {}
    lines = open(path).read().splitlines()
    for ln in lines:
        m = re.match(r"^#\s*@(\w+)\s+(.*)$", ln)
        if m:
            meta.setdefault(m.group(1), m.group(2))
    try:
        start = lines.index("cat >$_d/fpath/_%s <<'%s'" % (meta["cmd"], HEREDOC)) + 1
    except (KeyError, ValueError):
        sys.exit("compsys_spec_fuzz: %s is not a fixture written by this harness "
                 "(no @cmd header or no completer heredoc)" % path)
    end = lines.index(HEREDOC, start)
    body = "\n".join(lines[start:end]) + "\n"
    case = Case(int(meta.get("case", -1)), int(meta.get("seed", 0)), meta["cmd"],
                meta.get("kind", "replay"), [], [], {},
                meta["buffer"], meta["keys"].split(","))
    case.body_override = body
    return case


# ═════════════════════════════════════════════════════════════════════════════
# reporting
# ═════════════════════════════════════════════════════════════════════════════

def print_case_spec(case):
    print("  cmd      : %s   kind=%s" % (case.cmd, case.kind))
    print("  buffer   : %r   keys=%s" % (case.buffer, ",".join(case.keys)))
    for ln in case.completer().rstrip("\n").split("\n"):
        print("  | %s" % ln)


def print_failure(v, args):
    c = v.case
    print()
    print("=" * 78)
    print("%s %s — %s" % (v.status, c.name, v.detail))
    print("=" * 78)
    print_case_spec(c)
    ref, test = v.ref, v.test
    for label, cap in (("zsh", ref), ("zshrs", test)):
        for w in cap.warnings():
            print("  ! %-5s %s" % (label, w))
    if ref.grid is not None and test.grid is not None:
        if v.diffs:
            i, a, b = v.diffs[0]
            col = first_diff_cell(a, b)
            print("  first differing cell: row %d col %d" % (i, col))
            print("    zsh   | %s" % a)
            print("    zshrs | %s" % b)
            print("          | %s^" % (" " * col))
        print("  --- zsh grid ---")
        print(render(ref.grid))
        print("  --- zshrs grid ---")
        print(render(test.grid))
        if v.diffs and len(v.diffs) > 1:
            print("  --- differing rows (%d) ---" % len(v.diffs))
            for i, a, b in v.diffs:
                print("   %2d zsh   | %s" % (i, a))
                print("   %2d zshrs | %s" % (i, b))
        if v.cursor_differs:
            print("  cursor: zsh %s   zshrs %s" % (ref.cursor, test.cursor))
        if v.attr_rows:
            print("  rows differing in SGR attributes only: %s" % v.attr_rows)
    if v.ref_only_diags:
        print("  only zsh reported : %s" % "; ".join(sorted(v.ref_only_diags)))
    if v.test_only_diags:
        print("  only zshrs report : %s" % "; ".join(sorted(v.test_only_diags)))
    if getattr(v, "ref2", None) is not None and v.status == SKIP:
        print("  --- second zsh grid (self-check) ---")
        print(render(v.ref2.grid) if v.ref2.grid is not None else "  <no grid>")
    if ref.raw and test.raw and ref.raw != test.raw:
        sd = stream_diff(ref.raw, test.raw, args.raw_diff_lines)
        if sd:
            print("  --- raw escape-stream diff ---")
            for ln in sd:
                print(ln)
    if v.fixture:
        print("  fixture  : %s" % os.path.relpath(v.fixture, REPO))
        print("  replay   : %s --replay %s"
              % (SELF, os.path.relpath(v.fixture, REPO)))
    sys.stdout.flush()


def to_json(v):
    def side(c):
        return {
            "grid": c.grid,
            "reason": c.reason,
            "crash": c.crash,
            "cursor": list(c.cursor) if c.cursor else None,
            "diags": sorted(c.diags),
            "warnings": c.warnings(),
        }
    return {
        "case": v.case.name,
        "seed": v.case.seed,
        "index": v.case.idx,
        "kind": v.case.kind,
        "cmd": v.case.cmd,
        "buffer": v.case.buffer,
        "keys": v.case.keys,
        "completer": v.case.completer(),
        "status": v.status,
        "category": v.category,
        "detail": v.detail,
        "stream_only": v.stream_only,
        "fixture": v.fixture,
        "rows_differing": len(v.diffs),
        "ref": side(v.ref) if v.ref else None,
        "test": side(v.test) if v.test else None,
        "timing": {"seconds": round(v.duration, 2)},
    }


# ═════════════════════════════════════════════════════════════════════════════

def main():
    ap = argparse.ArgumentParser(
        description="generative, hermetic compsys completion-parity fuzzer")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--cases", type=int, default=8)
    ap.add_argument("--start", type=int, default=0,
                    help="first case index (with --seed, fixes the generation)")
    ap.add_argument("--zshrs", default=os.path.join(REPO, "target", "debug", "zshrs"))
    ap.add_argument("--zsh", default="zsh")
    ap.add_argument("--mode", choices=("zsh", "native"), default="zsh",
                    help="zsh: run the binary as `zshrs --zsh` (default; the "
                         "emulation path these specs target). native: run it "
                         "as-is.")
    ap.add_argument("--rows", type=int, default=24)
    ap.add_argument("--cols", type=int, default=80)
    ap.add_argument("--keys", default="tab",
                    help="comma-separated keys sent after the buffer")
    ap.add_argument("--kind", default="all",
                    help="comma-separated: arguments,values,describe,alternative")
    ap.add_argument("--jobs", type=int, default=1)
    ap.add_argument("--json", default=None)
    ap.add_argument("--verbose", action="store_true",
                    help="print every generated completer, not just failures")
    ap.add_argument("--replay", default=None, help="re-run a saved fixture")
    ap.add_argument("--spec", action="append", default=None,
                    help="inject a literal _arguments spec atom (repeatable) "
                         "instead of generating; runs as one ad-hoc case")
    ap.add_argument("--spec-buffer", default=None,
                    help="buffer to type for --spec (default '<cmd> ')")
    ap.add_argument("--non-ascii", action="store_true",
                    help="widen the description alphabet to non-ASCII (both "
                         "shells then run under en_US.UTF-8)")
    ap.add_argument("--settle", type=int, default=400,
                    help="ms of quiet before a screen is considered final")
    ap.add_argument("--boot-timeout", type=float, default=60.0)
    ap.add_argument("--shrink", dest="shrink", action="store_true", default=True)
    ap.add_argument("--no-shrink", dest="shrink", action="store_false")
    ap.add_argument("--shrink-probes", type=int, default=40)
    ap.add_argument("--self-check", dest="self_check", action="store_true", default=True,
                    help="on a failure, re-run real zsh against itself; a "
                         "reference that disagrees with itself makes the case "
                         "SKIP(unstable-reference) instead of a false FAIL")
    ap.add_argument("--no-self-check", dest="self_check", action="store_false")
    ap.add_argument("--strict-stream", action="store_true",
                    help="fail a case whose grids match but whose raw escape "
                         "streams differ (always reported either way)")
    ap.add_argument("--strict-cursor", action="store_true")
    ap.add_argument("--compare-attrs", action="store_true",
                    help="fail on SGR-attribute-only differences too")
    ap.add_argument("--raw-diff-lines", type=int, default=30)
    args = ap.parse_args()

    # Resolve both shells to absolute paths BEFORE anything execs them: the
    # child environment is built from scratch with a fixed PATH, and this host
    # has a second, older zsh at /bin/zsh that a bare name would silently
    # select on one side of the comparison.
    zsh_abs = shutil.which(args.zsh)
    if not zsh_abs:
        sys.exit("compsys_spec_fuzz: %s not found on PATH" % args.zsh)
    args.zsh = os.path.abspath(zsh_abs)
    args.zshrs = os.path.abspath(args.zshrs)
    if not os.path.exists(args.zshrs):
        sys.exit("compsys_spec_fuzz: %s not found (build it, or pass --zshrs)"
                 % args.zshrs)
    args.test_argv = ([args.zshrs, "-f", "-i"] if args.mode == "native"
                      else [args.zshrs, "--zsh", "-f", "-i"])
    args.keys = [k for k in args.keys.split(",") if k]
    for k in args.keys:
        key_bytes(k)                     # fail loudly on a typo, not silently
    all_kinds = ["arguments", "values", "describe", "alternative"]
    args.kinds = all_kinds if args.kind == "all" else [
        k.strip() for k in args.kind.split(",") if k.strip()]
    for k in args.kinds:
        if k not in all_kinds:
            sys.exit("compsys_spec_fuzz: unknown --kind %r (have %s)"
                     % (k, ",".join(all_kinds)))

    args.fpath_dirs = hermetic_fpath(args.zsh)

    # ── case list ────────────────────────────────────────────────────────────
    if args.replay:
        cases = [read_fixture(args.replay)]
        args.seed = cases[0].seed
    elif args.spec:
        c = Case(-1, args.seed, "fzc9999", "arguments", args.spec, ["-s"],
                 {"helpers": [], "states": []},
                 args.spec_buffer or "fzc9999 ", args.keys)
        cases = [c]
    else:
        cases = [generate(args.seed, args.start + i, args)
                 for i in range(args.cases)]

    root = os.path.join(REPO, "target", "spec-fuzz-%d" % args.seed)
    os.makedirs(root, exist_ok=True)

    print("# compsys_spec_fuzz — generative hermetic completion parity")
    print("# zsh    : %s" % args.zsh)
    print("# zshrs  : %s" % " ".join(args.test_argv))
    print("# fpath  : %s" % " ".join(args.fpath_dirs))
    print("# seed   : %d   cases %d..%d   keys=%s   %dx%d   jobs=%d"
          % (args.seed, args.start, args.start + len(cases) - 1,
             ",".join(args.keys), args.rows, args.cols, args.jobs))
    print("# kinds  : %s%s" % (",".join(args.kinds),
                               "   non-ascii" if args.non_ascii else ""))
    print("# outdir : %s" % os.path.relpath(root, REPO))
    print()
    sys.stdout.flush()

    results = []
    lock = threading.Lock()

    def work(case):
        v = run_case(args, root, args.fpath_dirs, case)
        if args.replay:
            v.fixture = args.replay
        elif v.status == FAIL and args.shrink and v.category in SHRINKABLE:
            try:
                minimal, probes = shrink_case(args, root, args.fpath_dirs,
                                              case, v.category)
                v.shrunk = (len(minimal.atoms), len(minimal.flags), probes)
                v.fixture = write_fixture(
                    os.path.join(root, "%s.min.zsh" % case.name), args, minimal,
                    "%s: %s (shrunk to %d spec(s), %d flag(s) in %d probes)"
                    % (v.category, v.detail, len(minimal.atoms),
                       len(minimal.flags), probes))
            except Exception as exc:
                print("  ! shrink failed for %s: %r" % (case.name, exc))
        elif v.status == FAIL and not args.replay:
            v.fixture = write_fixture(
                os.path.join(root, "%s.min.zsh" % case.name), args, case,
                "%s: %s (not shrunk)" % (v.category, v.detail))
        with lock:
            results.append(v)
            mark = {PASS: "PASS", PASS_ERR: "PASS(err)", FAIL: "FAIL", SKIP: "SKIP"}[v.status]
            extra = ""
            if v.stream_only:
                extra = "  [raw escape streams differ]"
            if v.status == FAIL and getattr(v, "shrunk", None):
                extra = "  [shrunk to %d spec(s)/%d flag(s) in %d probes]" % v.shrunk
            print("%-9s %-9s %-11s %-28s %s%s"
                  % (mark, v.case.name, v.case.kind, repr(v.case.buffer),
                     v.detail[:60], extra))
            sys.stdout.flush()
            if args.verbose and v.status in (PASS, PASS_ERR):
                print_case_spec(v.case)
            if v.status in (FAIL, SKIP):
                print_failure(v, args)
        return v

    if args.jobs > 1:
        from concurrent.futures import ThreadPoolExecutor
        with ThreadPoolExecutor(max_workers=args.jobs) as ex:
            list(ex.map(work, cases))
    else:
        for c in cases:
            work(c)

    results.sort(key=lambda v: v.case.idx)

    npass = sum(1 for v in results if v.status == PASS)
    nerr = sum(1 for v in results if v.status == PASS_ERR)
    nfail = sum(1 for v in results if v.status == FAIL)
    nskip = sum(1 for v in results if v.status == SKIP)
    nstream = sum(1 for v in results if v.stream_only)

    print()
    print("=" * 78)
    print("# %d case(s): %d PASS, %d PASS(err), %d FAIL, %d SKIP"
          % (len(results), npass, nerr, nfail, nskip))
    if nerr:
        print("#   PASS(err) = the generated spec was malformed and BOTH shells")
        print("#   printed the identical compsys error. Counted apart from a")
        print("#   clean pass: a run that is all PASS(err) tested nothing.")
    if nstream:
        print("#   %d case(s) matched on screen but differ in the RAW escape "
              "stream" % nstream)
        for v in results:
            if v.stream_only:
                print("#     %s (%s)" % (v.case.name, v.case.kind))
        if not args.strict_stream:
            print("#   (reported only; --strict-stream makes these fail)")
    if nskip:
        print("#   skipped, by reason:")
        reasons = {}
        for v in results:
            if v.status == SKIP:
                reasons.setdefault(v.category, []).append(v.case.name)
        for r, names in sorted(reasons.items()):
            print("#     %-22s %d  (%s)" % (r, len(names), " ".join(names)))
    if nfail:
        print("#   failures, by class:")
        classes = {}
        for v in results:
            if v.status == FAIL:
                classes.setdefault(v.category, []).append(v.case.name)
        for cname, names in sorted(classes.items()):
            print("#     %-22s %d  (%s)" % (cname, len(names), " ".join(names)))
        print("#   fixtures under %s" % os.path.relpath(root, REPO))

    if args.json:
        with open(args.json, "w") as f:
            json.dump({
                "seed": args.seed,
                "cases": len(results),
                "pass": npass, "pass_err": nerr, "fail": nfail, "skip": nskip,
                "stream_only": nstream,
                "zsh": args.zsh,
                "zshrs": args.test_argv,
                "results": [to_json(v) for v in results],
            }, f, indent=1)
        print("# json: %s" % args.json)

    bad = nfail + (nstream if args.strict_stream else 0)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
