#!/usr/bin/env python3
"""Verify every `sh:NN` annotation in the compsys ports against upstream zsh.

The `// sh:NN` / `//! sh:NN` comments in `src/compsys/ported/**` cite a line of
the upstream `Completion/**` shell function the port was translated from. They
used to be documentation; since `shared::set_sh_lineno` publishes them into
user-visible diagnostics (`_describe:compdescribe:129: no parsed state`), a
stale one is a WRONG line number in an error message, so they need a checker.

Two annotation shapes exist and are checked differently:

  transcript   `//! sh:129  while compdescribe -g csl2 _args _tmpm _tmpd; do`
               The comment carries the upstream line verbatim, so the check is
               a literal comparison against upstream line 129. When it fails,
               the exact text is searched for elsewhere in the upstream file
               and the true line number is reported.

  reference    `// sh:236` inside prose, or a `sh:70-79` range.
               No verbatim text to compare. Any code it quotes is checked
               against the cited region; otherwise it is reported unverifiable.

Statuses:
  ok / ok-span / ok-fuzzy / ok-ref  the cited line corroborates the annotation
  drift                             the annotation's verbatim text lives at a
                                    DIFFERENT line, which is reported
  suspect                           a weaker signal points elsewhere; a human
                                    must read the upstream line before acting
  unverified                        nothing checkable in the annotation
  cross-file                        cites a line of a different completion fn
  out-of-range                      cited line is past the end of the file

`--fix` rewrites ONLY annotations whose true line is proven by verbatim text
match, and prose references sitting exactly on such a proven line. Anything
that cannot be pinned that way is left alone: a number is never invented to
make an annotation look right, because these numbers now reach users.
Version-skewed functions (the shipped 5.9.2 copy differs from master) are
never rewritten, since which tree the port targeted is a judgement call.

Usage:
    python3 scripts/check_sh_lineno.py                  # summary table
    python3 scripts/check_sh_lineno.py -v               # every drifted line
    python3 scripts/check_sh_lineno.py --file _describe # one port
    python3 scripts/check_sh_lineno.py --json           # machine readable
    python3 scripts/check_sh_lineno.py --fix            # apply proven fixes
    python3 scripts/check_sh_lineno.py --realign        # re-pad sh: columns
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
PORTED = REPO / "src" / "compsys" / "ported"

# The spec repo (zsh master) and the shipped 5.9.2 tree. A file that differs
# between the two is version-skewed and its annotations cannot be judged
# without deciding which tree the port targeted, so it is reported separately.
SPEC_ROOT = Path(
    os.environ.get("ZSH_SRC", "/Users/wizard/forkedRepos/zsh")
) / "Completion"
SHIPPED_ROOT = Path(
    os.environ.get(
        "ZSH_SHIPPED",
        "/opt/homebrew/Cellar/zsh/5.9.2/share/zsh/functions",
    )
)

# `//! sh:129  text`, `// sh: 21  text`, `/// sh:118-121`
TRANSCRIPT_RE = re.compile(
    r"^\s*(?://!|///|//)\s*sh:\s*(\d+)(?:\s*-\s*(\d+))?(?:[ \t](.*))?$"
)
# any `sh:NN` mention, used for the reference/prose class and for counting
ANY_RE = re.compile(r"sh:\s*(\d+)(?:\s*-\s*(\d+))?")

WS = re.compile(r"\s+")
TOKEN = re.compile(r"[A-Za-z_][A-Za-z0-9_]+")
# trailing editorial the transcripts sometimes append: `… (flag parse)`
TRAILING_NOTE = re.compile(r"\s*(?:…|\.\.\.)?\s*\((?:[^()]*)\)\s*$")


def norm(s: str) -> str:
    return WS.sub(" ", s.strip())


def tokens(s: str) -> set[str]:
    return set(TOKEN.findall(s))


def jaccard(a: set[str], b: set[str]) -> float:
    if not a or not b:
        return 0.0
    return len(a & b) / len(a | b)


@dataclass
class Ann:
    lineno: int  # line in the .rs file
    sh: int  # cited upstream line
    sh_end: int | None  # range end, if any
    text: str | None  # verbatim upstream text, if this is a transcript
    raw: str
    kind: str = "reference"  # reference | transcript
    status: str = "unchecked"  # ok | drift | suspect | unverified | out-of-range
    truth: int | None = None  # verified correct upstream line, when known
    note: str = ""
    ambiguous: bool = False  # several sh:NN share this comment line
    other_file: str | None = None  # annotation cites a DIFFERENT upstream file
    own: str = ""  # stem of the ported file, to spot cross-file citations
    # ordinals of this annotation's numbers among ALL sh: numbers on the line,
    # so a rewrite can target `sh:21,24`'s second number without ambiguity
    slots: tuple[int, ...] = ()
    nums: tuple[int, ...] = ()


@dataclass
class FileReport:
    rs: Path
    upstream: Path | None
    skew: bool = False
    anns: list[Ann] = field(default_factory=list)

    @property
    def drifted(self) -> list[Ann]:
        return [a for a in self.anns if a.status == "drift"]

    @property
    def transcripts(self) -> list[Ann]:
        return [a for a in self.anns if a.kind == "transcript"]


def find_upstream(rs: Path) -> tuple[Path | None, bool]:
    """Map a ported .rs onto its upstream shell function.

    Returns (spec_path, skewed) where skewed means the shipped 5.9.2 copy
    differs from the master copy, so line numbers may legitimately disagree.
    """
    rel = rs.relative_to(PORTED).with_suffix("")
    cand = SPEC_ROOT / rel
    if not cand.is_file():
        # fall back to a basename search: a few ports live in a different
        # subdirectory than upstream does
        hits = [p for p in SPEC_ROOT.rglob(rel.name) if p.is_file()]
        if len(hits) != 1:
            return None, False
        cand = hits[0]

    shipped = list(SHIPPED_ROOT.glob(cand.name))
    skew = False
    if shipped:
        skew = shipped[0].read_bytes() != cand.read_bytes()
    return cand, skew


# `(Completion/Unix/Command/_yp sh:94)` — the annotation cites another file,
# so checking it against THIS port's upstream would be a false positive.
OTHER_FN_RE = re.compile(r"\b(_[a-z][a-z0-9_-]{2,})\b")
OTHER_FILE_RE = re.compile(r"(Completion/[\w./-]+|\b_[A-Za-z][\w-]*)\s+sh:\s*\d")


SH_GROUP_RE = re.compile(r"sh:\s*\d+(?:\s*[-,]\s*\d+)*")
NUM_RE = re.compile(r"\d+")


def sh_groups(raw: str) -> list[tuple[re.Match[str], list[int], list[int]]]:
    """Every `sh:` group on a line with its numbers and their line-wide slots.

    `sh:21,24` and `sh:13-20` are single groups of two numbers; a line may hold
    several groups. Slots are assigned across the whole line so a rewrite can
    address exactly one number.
    """
    out = []
    slot = 0
    for m in SH_GROUP_RE.finditer(raw):
        nums = [int(x) for x in NUM_RE.findall(m.group(0))]
        slots = list(range(slot, slot + len(nums)))
        slot += len(nums)
        out.append((m, nums, slots))
    return out


def parse_file(rs: Path) -> list[Ann]:
    out: list[Ann] = []
    own = rs.stem
    for i, raw in enumerate(rs.read_text(errors="replace").splitlines(), 1):
        if "sh:" not in raw:
            continue
        groups = sh_groups(raw)
        if not groups:
            continue
        multi = len(groups) > 1 or len(groups[0][1]) > 1
        om = OTHER_FILE_RE.search(raw)
        other = None
        if om and Path(om.group(1)).name != own:
            other = om.group(1)
        m = TRANSCRIPT_RE.match(raw)
        for gm, nums, slots in groups:
            # a transcript line is `//! sh:NN  <verbatim upstream text>`:
            # the FIRST group on a line that starts with the annotation, and a
            # single number (a range carries no verbatim text)
            body = (m.group(3) or "") if m is not None else ""
            # An em dash, or a leading dash/paren, marks editorial prose
            # (`sh:36 test — a blank preceded by a non-backslash`), never a
            # verbatim upstream line.
            prose = "—" in body or body.strip().startswith(("—", "--", "-", "("))
            if m is not None and len(nums) == 1 and gm is groups[0][0] and not prose:
                text = body
                kind = "transcript"
            elif m is not None and gm is groups[0][0]:
                text = body
                kind = "reference"
            else:
                text = None
                kind = "reference"
            out.append(
                Ann(
                    lineno=i,
                    sh=nums[0],
                    sh_end=nums[-1] if len(nums) > 1 else None,
                    text=text,
                    raw=raw,
                    kind=kind,
                    ambiguous=multi,
                    other_file=other,
                    slots=tuple(slots),
                    nums=tuple(nums),
                    own=own,
                )
            )
    return out


def resolve_other(name: str) -> list[str] | None:
    """Load the upstream file an annotation explicitly names, if it exists."""
    p = Path(name)
    cand = SPEC_ROOT.parent / name if name.startswith("Completion/") else None
    if cand is None or not cand.is_file():
        hits = [q for q in SPEC_ROOT.rglob(p.name) if q.is_file()]
        if len(hits) != 1:
            return None
        cand = hits[0]
    return cand.read_text(errors="replace").splitlines()


ELLIPSIS = re.compile(r"\s*(?:…|\.\.\.)\s*")
# code spans a prose reference can be checked against: `backticked` or 'quoted'
CODE_SPAN = re.compile(r"`([^`]{3,})`|\"([^\"]{3,})\"|'([^']{3,})'")


def gap_match(want: str, hay: str) -> bool:
    """`a … b` matches a line containing a then b in order."""
    parts = [p for p in ELLIPSIS.split(want) if p]
    if len(parts) < 2:
        return False
    pos = 0
    for p in parts:
        idx = hay.find(p, pos)
        if idx < 0:
            return False
        pos = idx + len(p)
    return True


def span_text(up: list[str], start: int, count: int) -> str:
    """Upstream lines start..start+count-1 joined, with `\\` continuations
    folded, so an annotation that paraphrases a wrapped statement matches."""
    seg = [norm(l).rstrip("\\").strip() for l in up[start - 1 : start - 1 + count]]
    return norm(" ".join(s for s in seg if s))


def check(anns: list[Ann], up_own: list[str]) -> None:
    for a in anns:
        up = up_own
        if a.other_file:
            alt = resolve_other(a.other_file)
            if alt is None:
                a.status = "unverified"
                a.note = f"cites {a.other_file}, not found upstream"
                continue
            up = alt
        check_one(a, up)


def check_one(a: Ann, up: list[str]) -> None:
    n = len(up)
    index: dict[str, list[int]] = {}
    for i, line in enumerate(up, 1):
        index.setdefault(norm(line), []).append(i)

    if a.sh < 1 or a.sh > n or (a.sh_end is not None and a.sh_end > n):
        # Prose often cites a line of a DIFFERENT completion function
        # ("the sh:67 `-N` next-set path" in _x_display means _tags:67).
        # Those read as out-of-range here but are not this port's problem.
        others = {t for t in OTHER_FN_RE.findall(a.raw)} - {a.own}
        a.status = "cross-file" if others else "out-of-range"
        a.note = (
            f"cites {sorted(others)[:3]}, not this file (upstream has {n} lines)"
            if others
            else f"upstream has {n} lines"
        )
        return

    want = norm(a.text or "")
    got = norm(up[a.sh - 1])

    # ---- prose / range references: no verbatim text to diff -------------
    if a.kind != "transcript" or want.startswith(("—", "--", "-", "(")):
        if a.ambiguous:
            a.status = "unverified"
            a.note = "several sh:NN on one line, cannot bind text to one"
            return
        spans = [
            g
            for m in CODE_SPAN.finditer(a.raw)
            for g in m.groups()
            if g and not g.startswith("sh:")
        ]
        lo, hi = a.sh, (a.sh_end or a.sh)
        region = norm(" ".join(up[lo - 1 : hi]))
        if spans and any(norm(s) in region for s in spans):
            a.status = "ok-ref"
            a.truth = a.sh
            a.note = "code span present in cited region"
        elif spans:
            # is the span somewhere else in the file?
            where = [
                i
                for i, l in enumerate(up, 1)
                if any(norm(s) in norm(l) for s in spans)
            ]
            if len(where) == 1:
                a.status = "suspect"
                a.truth = where[0]
                a.note = f"cited span found at {where[0]} ({where[0] - a.sh:+d})"
            else:
                a.status = "unverified"
                a.note = "prose reference, span not in cited region"
        else:
            a.status = "unverified"
            a.note = "prose reference, nothing quotable to check"
        return

    # ---- verbatim transcripts -------------------------------------------
    if want == got:
        a.status = "ok"
        a.truth = a.sh
        return

    # a transcript of a blank upstream line carries no text
    if not want:
        a.status = "ok" if not got else "unverified"
        a.note = "" if not got else "blank annotation over non-blank line"
        return

    # exact text found elsewhere -> mechanically verifiable drift
    hits = index.get(want)
    if hits:
        best = min(hits, key=lambda h: (abs(h - a.sh), h))
        a.status = "drift"
        a.truth = best
        a.note = f"delta {best - a.sh:+d}" + (
            f" ({len(hits)} candidates)" if len(hits) > 1 else ""
        )
        return

    # paraphrase of a statement that wraps over several upstream lines
    for k in (2, 3, 4):
        if a.sh + k - 1 <= n and (
            want == span_text(up, a.sh, k) or gap_match(want, span_text(up, a.sh, k))
        ):
            a.status = "ok-span"
            a.truth = a.sh
            a.note = f"paraphrase of lines {a.sh}-{a.sh + k - 1}"
            break
    if a.status != "unchecked":
        return

    if gap_match(want, got):
        a.status = "ok-fuzzy"
        a.truth = a.sh
        a.note = "elided transcript, fragments match in order"
        return

    # transcripts sometimes truncate with `…` or append `(a note)`
    stripped = TRAILING_NOTE.sub("", want).rstrip("… .")
    if stripped and got.startswith(stripped):
        a.status = "ok-fuzzy"
        a.truth = a.sh
        a.note = "truncated transcript, prefix matches"
        return
    if stripped and len(stripped) >= 8:
        pref = [i for i, l in enumerate(up, 1) if norm(l).startswith(stripped)]
        if len(pref) == 1:
            a.status = "drift"
            a.truth = pref[0]
            a.note = f"delta {pref[0] - a.sh:+d} (prefix match)"
            return

    sim = jaccard(tokens(want), tokens(got))
    if sim >= 0.6:
        a.status = "ok-fuzzy"
        a.truth = a.sh
        a.note = f"paraphrase, token overlap {sim:.2f}"
        return

    # Best token match elsewhere. NEVER reported as `drift`: a token score
    # is not proof, and a guessed number is worse than a missing one. It is
    # `suspect` so a human reads the upstream line before changing anything.
    scored = [(jaccard(tokens(want), tokens(l)), i) for i, l in enumerate(up, 1)]
    scored.sort(key=lambda t: (-t[0], abs(t[1] - a.sh)))
    top_sim, top_line = scored[0]
    if top_sim >= 0.7 and top_line != a.sh:
        a.status = "suspect"
        a.truth = top_line
        a.note = f"best token match {top_line} ({top_line - a.sh:+d}, {top_sim:.2f})"
    else:
        a.status = "unverified"
        a.note = f"no upstream line matches (best {top_sim:.2f} @ {top_line})"


def plan_fix(r: FileReport, up: list[str]) -> dict[int, tuple[int, int]]:
    """Re-derive the correct line for every drifted transcript in `r`.

    Only ever returns a number that is PROVEN: the annotation's verbatim text
    equals upstream line N after whitespace normalisation. Where several
    upstream lines carry the same text (`fi`, `done`, `esac`), the block is
    aligned as a sequence — the assignment must stay strictly increasing, which
    is what pins a duplicate to one line. Anything that cannot be pinned that
    way is left alone, never guessed.

    Returns {rs_line: (old_sh, new_sh)}.
    """
    by_text: dict[str, list[int]] = {}
    for i, line in enumerate(up, 1):
        by_text.setdefault(norm(line), []).append(i)

    # contiguous runs of transcript comment lines
    ts = [a for a in r.anns if a.kind == "transcript" and a.text]
    blocks: list[list[Ann]] = []
    for a in ts:
        if blocks and a.lineno == blocks[-1][-1].lineno + 1:
            blocks[-1].append(a)
        else:
            blocks.append([a])

    out: dict[int, tuple[int, int]] = {}
    for blk in blocks:
        if any(blk[i].sh > blk[i + 1].sh for i in range(len(blk) - 1)):
            continue  # non-monotonic transcript: sequence alignment is invalid
        items = [(a, by_text.get(norm(a.text or ""), [])) for a in blk]
        items = [(a, c) for a, c in items if c]
        if not items:
            continue
        # DP: strictly increasing choice minimising total displacement
        INF = float("inf")
        prev: list[tuple[float, int, int]] = []  # (cost, chosen, backptr)
        table: list[list[tuple[float, int, int]]] = []
        for k, (a, cands) in enumerate(items):
            row = []
            for ci, c in enumerate(cands):
                base = abs(c - a.sh)
                if k == 0:
                    row.append((base, c, -1))
                    continue
                best = (INF, c, -1)
                for pi, (pc, pchosen, _) in enumerate(prev):
                    if pchosen < c and pc + base < best[0]:
                        best = (pc + base, c, pi)
                row.append(best)
            table.append(row)
            prev = row
        if not prev or min(p[0] for p in prev) == INF:
            continue
        # walk back
        pi = min(range(len(prev)), key=lambda i: prev[i][0])
        chosen: list[int] = []
        for k in range(len(items) - 1, -1, -1):
            cost, c, back = table[k][pi]
            chosen.append(c)
            pi = back
        chosen.reverse()
        for (a, _), c in zip(items, chosen):
            if c != a.sh:
                out[a.lineno] = (a.sh, c)
    return out


MAX_ANCHOR_GAP = 4


def plan_refs(
    r: FileReport, up: list[str], moved: dict[int, tuple[int, int]], interpolate: bool = False
) -> list[tuple[int, int, int, int]]:
    """Carry the transcript correction over to the prose/range references.

    A file whose header transcript is renumbered but whose `// sh:NN —` prose
    is not is INTERNALLY INCONSISTENT, which is worse than uniformly stale, so
    the references have to move with it. They carry no verbatim text, so each
    one is only moved when it is bracketed: the nearest proven anchor below and
    the nearest proven anchor above (within MAX_ANCHOR_GAP upstream lines) must
    agree on the same delta, which pins every line between them. When the
    reference quotes code, that quote must additionally appear at the new line.
    Anything not pinned this way is left untouched and reported.
    """
    # anchor map old_sh -> delta, from both corrected and already-correct
    # transcript entries (a verified `ok` entry is an anchor with delta 0)
    anchors: dict[int, int] = {}
    for a in r.anns:
        if a.kind == "transcript" and a.status == "ok":
            anchors[a.sh] = 0
    for _rs_line, (old, new) in moved.items():
        anchors[old] = new - old
    if not anchors:
        return []
    keys = sorted(anchors)

    def bracket(line: int) -> int | None:
        # An exact anchor is the strongest case: the reference cites the very
        # statement whose text was verified verbatim, so its delta is proven.
        if line in anchors:
            return anchors[line]
        if not interpolate:
            return None
        below = [k for k in keys if k <= line]
        above = [k for k in keys if k >= line]
        if not below or not above:
            return None
        lo, hi = below[-1], above[0]
        if line - lo > MAX_ANCHOR_GAP or hi - line > MAX_ANCHOR_GAP:
            return None
        if anchors[lo] != anchors[hi]:
            return None
        return anchors[lo]

    out: list[tuple[int, int, int, int]] = []
    for a in r.anns:
        if a.lineno in moved or a.other_file:
            continue
        if a.kind == "transcript" and a.status == "ok":
            continue
        d = bracket(a.sh)
        if d is None or d == 0:
            continue
        if a.sh_end is not None and bracket(a.sh_end) != d:
            continue
        spans = [
            g
            for m in CODE_SPAN.finditer(a.raw)
            for g in m.groups()
            if g and not g.startswith("sh:")
        ]
        if spans:
            lo, hi = a.sh + d, (a.sh_end or a.sh) + d
            region = norm(" ".join(up[lo - 1 : hi]))
            if not any(norm(s) in region for s in spans):
                continue  # quote does not corroborate the shift: leave it
        if any(bracket(x) != d for x in a.nums):
            continue
        for slot, x in zip(a.slots, a.nums):
            out.append((a.lineno, slot, x, x + d))
    return out


def apply_fix(rs: Path, edits: list[tuple[int, int, int, int]]) -> int:
    """Rewrite the slot-th `sh:` number on a line, keeping the column width so
    a transcript block stays aligned. Edits are (rs_line, slot, old, new)."""
    lines = rs.read_text().splitlines(keepends=True)
    per_line: dict[int, dict[int, tuple[int, int]]] = {}
    for rs_line, slot, old, new in edits:
        per_line.setdefault(rs_line, {})[slot] = (old, new)

    n = 0
    for rs_line, slots in per_line.items():
        src = lines[rs_line - 1]
        spans: list[tuple[int, int, int]] = []  # (start, end, slot)
        for gm, nums, gslots in sh_groups(src):
            for nm, sl in zip(NUM_RE.finditer(gm.group(0)), gslots):
                spans.append((gm.start() + nm.start(), gm.start() + nm.end(), sl))
        changed = False
        for start, end, sl in reversed(spans):
            if sl not in slots:
                continue
            old, new = slots[sl]
            if src[start:end] != str(old):
                continue  # file moved under us: refuse rather than corrupt
            rep = str(new).rjust(end - start) if len(str(new)) <= end - start else str(new)
            src = src[:start] + rep + src[end:]
            changed = True
            n += 1
        if changed:
            lines[rs_line - 1] = src
    if n:
        rs.write_text("".join(lines))
    return n


ALIGN_RE = re.compile(r"^(\s*(?://!|///|//)\s*sh:)( *)(\d+)")


def realign(rs: Path) -> int:
    """Whitespace-only: pad `sh:` so a transcript block's numbers line up.

    Renumbering `sh:9` to `sh:10` widens the field by a digit and breaks the
    column the surrounding block is aligned on. Only the run of spaces between
    `sh:` and the number changes; no number is touched.
    """
    lines = rs.read_text().splitlines(keepends=True)
    blocks: list[list[int]] = []
    for i, src in enumerate(lines):
        if ALIGN_RE.match(src):
            if blocks and blocks[-1][-1] == i - 1:
                blocks[-1].append(i)
            else:
                blocks.append([i])
    n = 0
    for blk in blocks:
        if len(blk) < 2:
            continue
        width = max(len(ALIGN_RE.match(lines[i]).group(3)) for i in blk)
        for i in blk:
            m = ALIGN_RE.match(lines[i])
            assert m is not None
            want = " " * (width - len(m.group(3)))
            if m.group(2) != want:
                lines[i] = m.group(1) + want + m.group(3) + lines[i][m.end(3) :]
                n += 1
    if n:
        rs.write_text("".join(lines))
    return n


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--realign",
        action="store_true",
        help="whitespace-only: re-pad `sh:` so each transcript block's numbers "
        "share a column (run after --fix widens a number)",
    )
    ap.add_argument(
        "--interpolate",
        action="store_true",
        help="also move a prose reference whose cited line is BETWEEN two "
        "proven anchors that agree on the delta (weaker than an exact anchor)",
    )
    ap.add_argument(
        "--fix",
        action="store_true",
        help="rewrite drifted transcript annotations whose true line is PROVEN "
        "by verbatim text match; skips version-skewed files and anything "
        "that cannot be pinned",
    )
    ap.add_argument("--file", help="only ports whose path contains this substring")
    ap.add_argument("-v", "--verbose", action="store_true", help="list every finding")
    ap.add_argument("--json", action="store_true")
    ap.add_argument(
        "--status",
        default="drift",
        help="comma list of statuses to list with -v (drift,unverified,out-of-range,ok-fuzzy,ok)",
    )
    args = ap.parse_args()
    wanted = set(args.status.split(","))

    reports: list[FileReport] = []
    no_upstream: list[Path] = []
    for rs in sorted(PORTED.rglob("*.rs")):
        anns = parse_file(rs)
        if not anns:
            continue
        if args.file and args.file not in str(rs):
            continue
        up_path, skew = find_upstream(rs)
        if up_path is None:
            no_upstream.append(rs)
            continue
        # A version-skewed function must be judged against the tree the port
        # was actually translated from, else every annotation reads as drift.
        # Score both and keep the better fit rather than assume master.
        candidates = [up_path]
        if skew:
            candidates += list(SHIPPED_ROOT.glob(up_path.name))
        best = None
        for cand in candidates:
            trial = parse_file(rs)
            check(trial, cand.read_text(errors="replace").splitlines())
            score = sum(1 for a in trial if a.status.startswith("ok"))
            if best is None or score > best[0]:
                best = (score, cand, trial)
        assert best is not None
        reports.append(
            FileReport(rs=rs, upstream=best[1], skew=skew, anns=best[2])
        )

    if args.realign:
        tot_n = 0
        for r in reports:
            k = realign(r.rs)
            if k:
                tot_n += k
                print(f"{r.rs.relative_to(REPO)}: {k} line(s) re-padded")
        print(f"\n{tot_n} lines re-padded")
        return 0

    if args.fix:
        changed = 0
        touched = 0
        for r in reports:
            if r.skew:
                continue  # cannot tell which tree the port targeted
            up = r.upstream.read_text(errors="replace").splitlines()
            plan = plan_fix(r, up)
            refs = plan_refs(r, up, plan, args.interpolate)
            slot0 = {a.lineno: a.slots[0] for a in r.anns if a.slots}
            edits = [(ln, slot0[ln], o, nw) for ln, (o, nw) in plan.items()] + refs
            if not edits:
                continue
            n = apply_fix(r.rs, edits)
            if n:
                touched += 1
                changed += n
                print(f"{r.rs.relative_to(REPO)}: {n} annotation(s) corrected")
        print(f"\n{changed} annotations rewritten across {touched} files")
        return 0

    if args.json:
        print(
            json.dumps(
                [
                    {
                        "rs": str(r.rs.relative_to(REPO)),
                        "upstream": str(r.upstream),
                        "skew": r.skew,
                        "anns": [
                            {
                                "rs_line": a.lineno,
                                "sh": a.sh,
                                "kind": a.kind,
                                "status": a.status,
                                "truth": a.truth,
                                "note": a.note,
                                "text": a.text,
                            }
                            for a in r.anns
                        ],
                    }
                    for r in reports
                ],
                indent=1,
            )
        )
        return 0

    STATUSES = (
        "ok",
        "ok-span",
        "ok-fuzzy",
        "ok-ref",
        "drift",
        "suspect",
        "unverified",
        "cross-file",
        "out-of-range",
    )
    tot = {k: 0 for k in STATUSES}
    for r in reports:
        for a in r.anns:
            tot[a.status] = tot.get(a.status, 0) + 1

    worst = sorted(reports, key=lambda r: -len(r.drifted))
    print(f"ported files with sh:NN annotations : {len(reports)}")
    print(f"files with no upstream counterpart  : {len(no_upstream)}")
    print(f"annotations total                   : {sum(tot.values())}")
    for k in STATUSES:
        print(f"  {k:<14} {tot[k]}")
    skewed = [r for r in reports if r.skew]
    print(f"version-skewed upstream files       : {len(skewed)}")
    print()
    print("worst offenders (drifted transcript annotations):")
    for r in worst[:25]:
        if not r.drifted:
            break
        deltas = sorted({(a.truth or 0) - a.sh for a in r.drifted})
        print(
            f"  {len(r.drifted):>4}/{len(r.transcripts):<4} "
            f"{r.rs.relative_to(REPO)}  deltas={deltas[:6]}"
            + ("  [SKEW]" if r.skew else "")
        )

    if args.verbose:
        print()
        for r in reports:
            rows = [a for a in r.anns if a.status in wanted]
            if not rows:
                continue
            print(f"== {r.rs.relative_to(REPO)}  ({r.upstream})")
            for a in rows:
                print(
                    f"   rs:{a.lineno:<5} sh:{a.sh:<5} -> "
                    f"{a.truth if a.truth is not None else '?':<5} "
                    f"{a.status:<12} {a.note}"
                )
                if a.text:
                    print(f"        ann: {a.text.strip()}")
    if no_upstream:
        print()
        print("no upstream counterpart (not checked):")
        for p in no_upstream:
            print(f"  {p.relative_to(REPO)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
