#!/usr/bin/env python3
"""Regenerate docs/compsys_parity_report.html.

Sibling of ``gen_compsys_port_report.py``. That report answers "which upstream
``Completion/`` files have a Rust port"; this one answers the question that
actually gates a switch-over: **does the completion ENGINE behave
byte-identically to zsh at the terminal**.

Input is the output of real harness runs — nothing here is hand-maintained:

* ``target/parity-matrix*/native.<combo>.log`` — one
  ``scripts/comptab_parity.py`` run per zstyle combo, one line per
  (key-sequence, case) cell.
* ``target/parity-matrix*/zsh.<combo>.log`` — the same combo through
  ``scripts/compsys_parity.py`` (``parity_matrix.py:170-171`` picks the script
  from the harness name), whose cell lines carry a case NAME and a
  ``[key+key]`` suffix instead of a sequence name.
* ``target/parity-matrix*/random-combos.log`` — random SUBSETS of the live
  fixture, each shrunk to a minimal diverging statement set when it fails.

Every verdict either harness can emit is parsed and rendered as its own
category (``VERDICTS`` below). A ``TIMEOUT`` or a ``SKIP`` is never folded into
a pass: it is not evidence in either direction, and it still counts against the
denominator, because an unmeasured cell is not a passing one.

The generator REFUSES to emit a report when it finds no run data, rather than
rendering an empty page that reads as "all green". Every number on the page is
derived from the parsed logs; the provenance block records which logs, their
mtimes, and what was NOT covered.

It also RECONCILES what it parsed against the summary line each harness printed
for itself. If the per-row counts and the harness's own totals disagree, the
report says so in a banner at the top of the page and the process exits
non-zero — a scraper that silently drops rows under-reports the sweep, which is
the failure mode this check exists to make impossible to ship quietly.

Bot/LLM/scraper-friendly output, same conventions as the port reports:

* ``<!-- BEGIN-GROUP case=... -->`` / ``<!-- END-GROUP -->`` per case.
* ``<script id="compsys-parity-report-data" type="application/json">`` embeds
  the whole dataset.
* Every cell row carries a trailing ``<!-- SYM ... -->`` comment.
"""
from __future__ import annotations

import argparse
import html
import json
import os
import re
import sys
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "docs" / "compsys_parity_report.html"
DEFAULT_GLOB = "target/parity-matrix*"

sys.path.insert(0, str(ROOT / "scripts"))
from parity_corpus import CASES, KEY_SEQUENCES  # noqa: E402

# Every verdict the two harnesses can put at the head of a cell line. Read off
# their print sites, not guessed:
#   comptab_parity.py:1882   counts = {"PASS", "FAIL", "FLAKY", "TIMEOUT", "SKIP"}
#   comptab_parity.py:2308   "# categories: PASS=%d FAIL=%d TIMEOUT=%d SKIP=%d"
#   compsys_parity.py:1010   status: str = "PASS"   # PASS | FAIL | FLAKY | SKIP
#
# The old CELL_RE listed only PASS|FAIL|FLAKY, so every TIMEOUT and SKIP row a
# sweep produced was silently dropped from this page while the harness's own
# summary line still counted them — the report's numbers stopped adding up.
VERDICTS = ("PASS", "FAIL", "FLAKY", "TIMEOUT", "SKIP")
# Categories that are failures. TIMEOUT and SKIP are deliberately NOT here and
# deliberately NOT passes: neither screen was ever compared.
FAILING = ("FAIL", "FLAKY")
# Categories where no comparison happened at all.
UNMEASURED = ("TIMEOUT", "SKIP")
_V = "|".join(VERDICTS)

# comptab_parity.py:1887-1890 —
#   "%-6s %-18s %r" (main sweep) / "%-7s %-8s %r" (--mutate), then an optional
#   "  [<fingerprint>]" for FAIL/FLAKY, then an optional "  (<detail>)".
CELL_RE = re.compile(
    r"^(" + _V + r")\s+(\S+)\s+(.*?)"
    r"(?:\s\s+\[([0-9a-f]{4,})\])?"
    r"(?:\s\s+\((.*)\))?$")
# compsys_parity.py:1608/1612/1631 —
#   "{status} {case.name:16s} {buffer!r} [{keys joined by +}]" + optional
#   "  ({detail})". Different enough that one regex for both mis-parses one of
#   them, so the harness name selects which to use.
COMPSYS_CELL_RE = re.compile(
    r"^(" + _V + r")\s+(\S+)\s+(.*?)\s\[([^\]]*)\](?:\s\s+\((.*)\))?$")
# A cell-shaped line whose leading word is NOT a verdict we know. Both harnesses
# indent every screen dump by two spaces (comptab_parity.py:957,
# compsys_parity.py:590), so a bare ALLCAPS token in column 0 is a verdict, and
# an unrecognised one means a harness grew a category this scraper would drop.
UNKNOWN_VERDICT_RE = re.compile(r"^([A-Z][A-Z0-9_]{2,})\s+\S")

HDR_RE = re.compile(r"^# (\w+)\s*: (.*)$")
# comptab_parity.py:2306 "# N passed, M failed, T cell(s)"
# comptab_parity.py:1937 "# N passed, M failed, T cell(s)"      (--mutate)
# compsys_parity.py:1649 "# N passed, M failed, F flaky, T total"
# compsys_parity.py:1262 "# N passed, M failed, F flaky, T cells run"
SUMMARY_RE = re.compile(
    r"^# (\d+) passed, (\d+) failed(?:, (\d+) flaky)?, (\d+) (?:cell|total)")
# comptab_parity.py:1939 / :2308 — the authoritative per-category breakdown.
CATEGORIES_RE = re.compile(r"^# categories: (.*)$")
CATEGORY_KV_RE = re.compile(r"([A-Z]+)=(\d+)")

RC_FAIL_RE = re.compile(r"^FAIL combo (\d+)\s+\(\s*(\d+) statements\) on (.*)$")
RC_MIN_RE = re.compile(r"^\s+minimal set: (\d+) statement\(s\) -> (.*)$")
RC_INDEP_RE = re.compile(r"^\s+config-INDEPENDENT")
RC_PASS_RE = re.compile(r"^PASS combo (\d+)\s+\(\s*(\d+) statements\)")
# comptab_parity.py:1426 — a budget-exhausted combo. Same drop as CELL_RE had:
# it was printed, counted by the harness, and invisible on this page.
RC_TIMEOUT_RE = re.compile(
    r"^TIMEOUT combo (\d+)\s+\(\s*(\d+) statements\) on (.*?)(?: — (.*))?$")
# comptab_parity.py:1470-1471
RC_DIVERGED_RE = re.compile(r"^# (\d+)/(\d+) combo\(s\) diverged")
RC_BUDGET_RE = re.compile(r"^# (\d+)/(\d+) combo\(s\) ran out of measurement budget")

# Buffer -> case name, so a log line can be mapped back to the corpus entry.
BY_BUFFER = {c.buffer: c for c in CASES}


def _unrepr(s: str) -> str:
    """`repr()` of a str, back to the str. Falls back to the literal text."""
    try:
        return eval(s, {"__builtins__": {}})
    except Exception:
        return s


def _cell(status: str, seq: str, buf: str, keys: str, detail: str,
          fingerprint: str | None) -> dict:
    case = BY_BUFFER.get(buf)
    return {
        "status": status,
        "sequence": seq,
        "keys": keys,
        "buffer": buf,
        "case": case.name if case else "(ad-hoc)",
        "tags": list(case.tags) if case else [],
        "note": case.note if case else "",
        "detail": detail or "",
        "fingerprint": fingerprint or "",
    }


def parse_combo_log(path: Path, harness: str) -> dict:
    """One `comptab_parity.py` (native) or `compsys_parity.py` (zsh) run.

    Returns the cells AND what the harness said about itself, so `reconcile()`
    can check the two against each other. Nothing here filters a row out: the
    old version only kept a cell whose second field was a known key-sequence
    name, which dropped every `compsys_parity.py` row (that field is a case
    name there) on top of dropping every TIMEOUT and SKIP.
    """
    meta: dict[str, str] = {}
    cells: list[dict] = []
    summary: dict | None = None
    categories: dict[str, int] = {}
    unknown_verdicts: dict[str, int] = {}
    rx = COMPSYS_CELL_RE if harness == "zsh" else CELL_RE
    for line in path.read_text(errors="replace").splitlines():
        # CATEGORIES_RE first: `# categories: PASS=1 ...` also satisfies HDR_RE
        # (`# <word>: <rest>`), and letting the header rule win files the
        # authoritative per-category breakdown away as a meta setting, which is
        # how the cross-check silently did nothing.
        m = CATEGORIES_RE.match(line)
        if m:
            categories = {k: int(v) for k, v in CATEGORY_KV_RE.findall(m.group(1))}
            continue
        m = HDR_RE.match(line)
        if m:
            meta[m.group(1)] = m.group(2).strip()
            continue
        m = SUMMARY_RE.match(line)
        if m:
            summary = {
                "passed": int(m.group(1)),
                "failed": int(m.group(2)),
                # compsys_parity.py reports flaky OUTSIDE `failed`;
                # comptab_parity.py folds it in and prints no flaky field.
                "flaky": int(m.group(3)) if m.group(3) is not None else None,
                "cells": int(m.group(4)),
                "line": line,
            }
            continue
        m = rx.match(line)
        if m:
            if harness == "zsh":
                status, seq, buf_repr, keys, detail = m.groups()
                fp = None
                keys = keys or ""
            else:
                status, seq, buf_repr, fp, detail = m.groups()
                keys = "+".join(KEY_SEQUENCES.get(seq, []))
            cells.append(_cell(status, seq, _unrepr(buf_repr), keys, detail, fp))
            continue
        m = UNKNOWN_VERDICT_RE.match(line)
        if m and m.group(1) not in VERDICTS and m.group(1) not in ("BEGIN", "END"):
            unknown_verdicts[m.group(1)] = unknown_verdicts.get(m.group(1), 0) + 1
    return {"meta": meta, "cells": cells, "summary": summary,
            "categories": categories, "unknown_verdicts": unknown_verdicts}


def parse_random_log(path: Path) -> dict:
    """The random-subset fuzz run."""
    meta: dict[str, str] = {}
    combos: list[dict] = []
    summary: dict[str, int] = {}
    cur: dict | None = None
    for line in path.read_text(errors="replace").splitlines():
        m = HDR_RE.match(line)
        if m:
            meta[m.group(1)] = m.group(2).strip()
        m = RC_DIVERGED_RE.match(line)
        if m:
            summary["diverged"] = int(m.group(1))
            summary["combos"] = int(m.group(2))
            continue
        m = RC_BUDGET_RE.match(line)
        if m:
            summary["timeout"] = int(m.group(1))
            summary["combos"] = int(m.group(2))
            continue
        m = RC_PASS_RE.match(line)
        if m:
            combos.append({"index": int(m.group(1)), "statements": int(m.group(2)),
                           "status": "PASS", "case": "", "minimal": None,
                           "config_independent": False, "path": "", "detail": ""})
            cur = None
            continue
        # A budget-exhausted combo is neither a pass nor a divergence. It used
        # to match no regex here at all, so the page showed N-1 combos and
        # called the missing one nothing.
        m = RC_TIMEOUT_RE.match(line)
        if m:
            combos.append({"index": int(m.group(1)), "statements": int(m.group(2)),
                           "status": "TIMEOUT", "case": _unrepr(m.group(3).strip()),
                           "minimal": None, "config_independent": False,
                           "path": "", "detail": (m.group(4) or "").strip()})
            cur = None
            continue
        m = RC_FAIL_RE.match(line)
        if m:
            cur = {"index": int(m.group(1)), "statements": int(m.group(2)),
                   "status": "FAIL", "case": m.group(3).strip(), "minimal": None,
                   "config_independent": False, "path": "", "detail": ""}
            combos.append(cur)
            continue
        if cur is not None:
            if RC_INDEP_RE.match(line):
                cur["config_independent"] = True
            m = RC_MIN_RE.match(line)
            if m:
                cur["minimal"] = int(m.group(1))
                cur["path"] = m.group(2).strip()
    return {"meta": meta, "combos": combos, "summary": summary}


def collect(run_dirs: list[Path]) -> dict:
    runs: list[dict] = []
    randoms: list[dict] = []
    sources: list[dict] = []
    for d in run_dirs:
        for log in sorted(d.glob("*.log")):
            stat = log.stat()
            src = {
                "path": str(log.relative_to(ROOT)),
                "mtime": datetime.fromtimestamp(stat.st_mtime, timezone.utc)
                .isoformat(timespec="seconds"),
                "bytes": stat.st_size,
            }
            if log.name == "random-combos.log":
                r = parse_random_log(log)
                if r["combos"]:
                    r["source"] = src
                    randoms.append(r)
                    sources.append(src | {"kind": "random-combos"})
                continue
            # native.<combo>.log / zsh.<combo>.log, and the sharded form
            # native.<combo>.shard<N>.log that `parity_matrix.py --jobs` emits.
            # Shards are slices of the SAME combo run, so they must merge back
            # into one combo or the per-combo rollup double-counts it as N
            # separate configs. The harness prefix also decides which cell-line
            # shape to parse (parity_matrix.py:170-171).
            parts = log.stem.split(".", 1)
            harness = parts[0]
            parsed = parse_combo_log(log, harness)
            if not parsed["cells"] and not parsed["summary"]:
                continue
            combo = parts[1] if len(parts) > 1 else log.stem
            combo = re.sub(r"\.shard\d+$", "", combo)
            parsed["harness"] = harness
            parsed["combo"] = combo
            parsed["source"] = src
            runs.append(parsed)
            sources.append(src | {"kind": "combo", "combo": parsed["combo"]})
    # Reconciliation is per-LOG: each log carries its own summary line, and
    # merging shards first would compare one shard's totals against every
    # shard's rows.
    per_log = [{"path": r["source"]["path"], "harness": r["harness"],
                "combo": r["combo"], "cells": r["cells"],
                "summary": r["summary"], "categories": r["categories"],
                "unknown_verdicts": r["unknown_verdicts"]} for r in runs]
    merged: dict[tuple[str, str], dict] = {}
    for r in runs:
        key = (r["harness"], r["combo"])
        if key in merged:
            merged[key]["cells"].extend(r["cells"])
            merged[key]["meta"].update(r["meta"])
        else:
            merged[key] = dict(r, cells=list(r["cells"]))
    return {"runs": list(merged.values()), "randoms": randoms,
            "sources": sources, "per_log": per_log}


def reconcile(data: dict) -> list[str]:
    """Do the rows we parsed add up to what the harness said it ran?

    This exists because the scraper CAN drop rows — it did, for every TIMEOUT
    and SKIP, from the day those verdicts were introduced — and a dropped row is
    invisible: the page just shows a smaller, greener table. Comparing the rows
    against the harness's own self-reported totals turns a silent drop into a
    loud one. Every disagreement is returned as a human-readable line; the
    caller banners them and exits non-zero.
    """
    out: list[str] = []
    for lg in data.get("per_log", []):
        name = lg["path"]
        rows = lg["cells"]
        n = {v: sum(1 for c in rows if c["status"] == v) for v in VERDICTS}
        for verdict, count in sorted(lg["unknown_verdicts"].items()):
            out.append(
                f"{name}: {count} line(s) start with the unrecognised verdict "
                f"{verdict!r} — this scraper knows only {', '.join(VERDICTS)}, "
                f"so those rows are NOT on this page")
        s = lg["summary"]
        if s is None:
            out.append(f"{name}: no summary line found — cannot verify that the "
                       f"{len(rows)} parsed row(s) are all of them")
            continue
        if len(rows) != s["cells"]:
            out.append(f"{name}: parsed {len(rows)} cell row(s) but the harness "
                       f"reported {s['cells']} ({s['line'].strip()})")
        if n["PASS"] != s["passed"]:
            out.append(f"{name}: parsed {n['PASS']} PASS row(s), harness "
                       f"reported {s['passed']} passed")
        if s["flaky"] is None:
            # comptab_parity.py:2306 folds FLAKY into `failed`.
            if n["FAIL"] + n["FLAKY"] != s["failed"]:
                out.append(f"{name}: parsed {n['FAIL']} FAIL + {n['FLAKY']} FLAKY "
                           f"row(s), harness reported {s['failed']} failed")
        else:
            # compsys_parity.py:1649 reports them separately.
            if n["FAIL"] != s["failed"] or n["FLAKY"] != s["flaky"]:
                out.append(f"{name}: parsed {n['FAIL']} FAIL / {n['FLAKY']} FLAKY "
                           f"row(s), harness reported {s['failed']} failed / "
                           f"{s['flaky']} flaky")
        for k, v in sorted(lg["categories"].items()):
            if k not in VERDICTS:
                out.append(f"{name}: harness counted a category {k}={v} that this "
                           f"scraper does not know how to parse")
            elif n[k] != v:
                out.append(f"{name}: parsed {n[k]} {k} row(s), harness's own "
                           f"`# categories:` line says {k}={v}")
    for r in data.get("randoms", []):
        name = r["source"]["path"] if "source" in r else "random-combos.log"
        s = r.get("summary", {})
        combos = r["combos"]
        if s.get("combos") is not None and len(combos) != s["combos"]:
            out.append(f"{name}: parsed {len(combos)} combo row(s) but the "
                       f"harness ran {s['combos']}")
        nfail = sum(1 for c in combos if c["status"] == "FAIL")
        if s.get("diverged") is not None and nfail != s["diverged"]:
            out.append(f"{name}: parsed {nfail} diverging combo(s), harness "
                       f"reported {s['diverged']}")
        nto = sum(1 for c in combos if c["status"] == "TIMEOUT")
        if nto != s.get("timeout", 0):
            out.append(f"{name}: parsed {nto} TIMEOUT combo(s), harness reported "
                       f"{s.get('timeout', 0)}")
    return out


def totals_of(data: dict) -> dict:
    cells = [c for r in data["runs"] for c in r["cells"]]
    t = {
        "cells": len(cells),
        "combos": len({r["combo"] for r in data["runs"]}),
        "cases": len({c["case"] for c in cells}),
        "sequences": len({c["sequence"] for c in cells}),
        "corpus_cases": len(CASES),
        "corpus_sequences": len(KEY_SEQUENCES),
    }
    for v in VERDICTS:
        t[v.lower()] = sum(1 for c in cells if c["status"] == v)
    t["unmeasured"] = sum(t[v.lower()] for v in UNMEASURED)
    # Denominator is EVERY row, TIMEOUT and SKIP included. A cell whose screens
    # were never both final is not a cell that passed, and putting it outside
    # the denominator would make an unfinished sweep read greener than a
    # finished one.
    t["pct"] = (100.0 * t["pass"] / t["cells"]) if t["cells"] else 0.0
    # The same rate over only the cells that actually produced a comparison, so
    # the two numbers together say how much of the sweep is evidence at all.
    t["measured"] = t["cells"] - t["unmeasured"]
    t["pct_measured"] = (100.0 * t["pass"] / t["measured"]) if t["measured"] else 0.0
    rc = [c for r in data["randoms"] for c in r["combos"]]
    t["random_total"] = len(rc)
    t["random_pass"] = sum(1 for c in rc if c["status"] == "PASS")
    t["random_fail"] = sum(1 for c in rc if c["status"] == "FAIL")
    t["random_timeout"] = sum(1 for c in rc if c["status"] == "TIMEOUT")
    t["random_indep"] = sum(1 for c in rc if c.get("config_independent"))
    return t


EXTRA_CSS = """
  .tutorial-main { max-width: 96rem; }
  .stat-grid { display:grid;grid-template-columns:repeat(auto-fill,minmax(11rem,1fr));gap:0.6rem;margin:1rem 0; }
  .stat-card { border:1px solid var(--border);border-top:3px solid var(--cyan);background:var(--bg-card);padding:0.7rem 0.9rem;border-radius:2px;text-align:center; }
  .stat-card .stat-val { font-family:'Orbitron',sans-serif;font-size:22px;font-weight:900;color:var(--cyan);line-height:1.1;text-shadow:0 0 14px var(--cyan-glow); }
  .stat-card .stat-val.green   { color:var(--green); text-shadow:0 0 14px rgba(57,255,20,.35); }
  .stat-card .stat-val.red     { color:#ff6b6b; text-shadow:0 0 14px rgba(255,107,107,.35); }
  .stat-card .stat-val.yellow  { color:#ffb800; text-shadow:0 0 14px rgba(255,184,0,.35); }
  .stat-card .stat-val.gray    { color:#8b949e; }
  .stat-card .stat-label { font-family:'Orbitron',sans-serif;font-size:9px;font-weight:700;letter-spacing:1.5px;text-transform:uppercase;color:var(--text-muted);margin-top:0.4rem; }

  table.fn-table { width:100%;border-collapse:collapse;font-size:11.5px;margin:0.8rem 0; }
  table.fn-table th { background:var(--bg-secondary);color:var(--cyan);font-family:'Orbitron',sans-serif;font-size:10px;font-weight:700;letter-spacing:1.2px;text-transform:uppercase;text-align:left;padding:7px 10px;border:1px solid var(--border); }
  table.fn-table td { padding:6px 10px;border:1px solid var(--border);color:var(--text-dim);vertical-align:top; }
  table.fn-table tr:hover td { background:var(--bg-hover); }
  table.fn-table code { font-size:11px;color:var(--accent-light);background:var(--bg-primary);padding:1px 4px;border-radius:2px; }
  table.fn-table td.num { text-align:right;font-family:'Share Tech Mono',monospace; }

  tr.grp-row td.grp-cell {
    background:var(--bg-secondary);color:var(--cyan);
    font-family:'Orbitron',sans-serif;font-size:11px;font-weight:700;
    letter-spacing:1.2px;padding:8px 10px;border-top:2px solid var(--cyan);
    cursor:pointer;user-select:none;
  }
  tr.grp-row.open td.grp-cell { background:var(--bg-hover); }
  tr.grp-row td.grp-cell:hover { background:var(--bg-hover); }
  tr.grp-row td.grp-cell .grp-tog {
    display:inline-block;width:1.2rem;color:var(--cyan);
    font-family:'Share Tech Mono',monospace;font-weight:700;
  }
  tr.grp-row td.grp-cell code {
    font-family:'Share Tech Mono',monospace;font-size:11.5px;
    color:var(--accent-light);background:var(--bg-primary);padding:1px 6px;
  }
  tr.grp-row td.grp-cell .counts {
    float:right;color:var(--text-muted);font-weight:400;
    font-family:'Share Tech Mono',monospace;font-size:10.5px;
    letter-spacing:0;text-transform:none;
  }
  tr.detail-row.hidden { display:none; }

  .pill { display:inline-block;padding:1px 7px;border-radius:2px;
    font-family:'Share Tech Mono',monospace;font-size:10.5px;font-weight:700;
    letter-spacing:0.5px;text-transform:uppercase; }
  .pill.pass { background:rgba(57,255,20,.12);color:var(--green);border:1px solid rgba(57,255,20,.3); }
  .pill.fail { background:rgba(255,107,107,.12);color:#ff6b6b;border:1px solid rgba(255,107,107,.3); }
  .pill.flaky { background:rgba(255,184,0,.12);color:#ffb800;border:1px solid rgba(255,184,0,.3); }
  .pill.timeout { background:rgba(255,138,0,.12);color:#ff8a00;border:1px solid rgba(255,138,0,.35); }
  .pill.skip { background:rgba(139,148,158,.12);color:#8b949e;border:1px solid rgba(139,148,158,.3); }
  .pill.inconclusive { background:rgba(255,138,0,.12);color:#ff8a00;border:1px solid rgba(255,138,0,.35); }
  .pill.tag { background:rgba(139,148,158,.12);color:#8b949e;border:1px solid rgba(139,148,158,.3); }
  .pill.indep { background:rgba(211,0,197,.12);color:#d300c5;border:1px solid rgba(211,0,197,.3); }

  tr.st-PASS  td.status { color:var(--green);font-weight:700; }
  tr.st-FAIL  td.status { color:#ff6b6b;font-weight:700; }
  tr.st-FLAKY td.status { color:#ffb800;font-weight:700; }
  tr.st-TIMEOUT td.status { color:#ff8a00;font-weight:700; }
  tr.st-SKIP  td.status { color:#8b949e;font-weight:700; }
  tr.st-INCONCLUSIVE td.status { color:#ff8a00;font-weight:700; }
  tr.st-NOTRUN td { color:#8b949e;opacity:0.75; }
  .pill.notrun { background:rgba(139,148,158,.12);color:#8b949e;border:1px solid rgba(139,148,158,.3); }

  .note { border:1px solid var(--border);border-left:3px solid #ffb800;
    background:var(--bg-card);padding:0.6rem 0.9rem;margin:0.9rem 0;
    font-family:'Share Tech Mono',monospace;font-size:11.5px;color:var(--text-dim); }
  .alarm { border:2px solid #ff6b6b;border-left:8px solid #ff6b6b;
    background:rgba(255,107,107,.08);padding:0.8rem 1rem;margin:1rem 0;
    font-family:'Share Tech Mono',monospace;font-size:12px;color:#ff9b9b; }
  .alarm h3 { font-family:'Orbitron',sans-serif;font-size:13px;letter-spacing:1.4px;
    color:#ff6b6b;margin:0 0 0.5rem; }
  .alarm ul { margin:0.3rem 0 0.3rem 1.1rem;padding:0; }
  .alarm li { margin:0.2rem 0; }
"""

JS = """
document.querySelectorAll('tr.grp-row').forEach(g => {
  const key = g.dataset.grp;
  const tog = g.querySelector('.grp-tog');
  const rows = document.querySelectorAll(`tr.detail-row[data-grp="${CSS.escape(key)}"]`);
  g.addEventListener('click', () => {
    const open = g.classList.toggle('open');
    if (tog) tog.textContent = open ? '[-]' : '[+]';
    rows.forEach(r => r.classList.toggle('hidden', !open));
  });
});
// Groups with a failure start expanded — the point of the page is the gaps.
document.querySelectorAll('tr.grp-row[data-fail="1"]').forEach(g => g.click());
"""


def render(data: dict, t: dict, problems: list[str]) -> str:
    # `compsys_port_report.html` is generated on demand and is NOT one of
    # the force-added docs/*.html, so linking it unconditionally would ship
    # a 404 on Pages. Only crumb it when it is actually there.
    port_crumb = ('<a href="compsys_port_report.html">Compsys Port Report</a>\n'
                  '        <span class="sep">/</span>\n        '
                  if (ROOT / 'docs' / 'compsys_port_report.html').exists() else '')
    by_case: dict[str, list[dict]] = defaultdict(list)
    for r in data["runs"]:
        for c in r["cells"]:
            by_case[c["case"]].append(c | {"combo": r["combo"], "harness": r["harness"]})

    combo_rows = []
    for r in sorted(data["runs"], key=lambda r: (r["harness"], r["combo"])):
        cs = r["cells"]
        row = {"harness": r["harness"], "combo": r["combo"], "cells": len(cs)}
        for v in VERDICTS:
            row[v.lower()] = sum(1 for c in cs if c["status"] == v)
        combo_rows.append(row)

    seqs_run = sorted({c["sequence"] for cs in by_case.values() for c in cs})
    seqs_not_run = [s for s in KEY_SEQUENCES if s not in seqs_run]
    cases_run = set(by_case)
    cases_not_run = [c.name for c in CASES if c.name not in cases_run]

    # Anything not run has to be stated BEFORE the numbers, not after them:
    # a 37.9% pass rate over a battery that skipped every arrow direction but
    # `down` is not a measurement of arrow-key parity, and a reader must not
    # have to scroll past the stat cards to learn that.
    gap_html = ""
    if seqs_not_run or cases_not_run:
        gap_html = (
            '<div class="note"><strong>Partial coverage.</strong> This run exercised '
            f'{len(seqs_run)} of {len(KEY_SEQUENCES)} key sequences and '
            f'{len(cases_run)} of {len(CASES)} cases. Every number below covers only '
            'what was run — an unexercised sequence is not a passing one.'
        )
        if seqs_not_run:
            gap_html += ('<br>Sequences NOT run: <code>'
                         + html.escape(", ".join(seqs_not_run)) + '</code>')
        if cases_not_run:
            gap_html += ('<br>Cases NOT run: <code>'
                         + html.escape(", ".join(cases_not_run)) + '</code>')
        gap_html += '</div>'
    partial_note = " (partial)" if gap_html else ""

    # The reconciliation banner goes ABOVE everything, including the coverage
    # note: if the rows on this page do not add up to what the harness said it
    # ran, no number below is trustworthy and the reader has to know that
    # before reading any of them.
    alarm_html = ""
    if problems:
        alarm_html = (
            '<div class="alarm"><h3>&#9888; COUNTS DO NOT RECONCILE &mdash; '
            'THIS REPORT IS INCOMPLETE</h3>'
            '<p style="margin:0 0 0.4rem;">The rows parsed out of the harness logs '
            'do not match the totals those harnesses printed for themselves. Rows '
            'are being dropped or double-counted by this generator, so every '
            'number below understates or misstates the sweep. Fix the parser in '
            '<code>scripts/gen_compsys_parity_report.py</code>; do not read the '
            'table until it reconciles.</p><ul>'
            + "".join(f'<li>{html.escape(x)}</li>' for x in problems)
            + '</ul></div>'
            '<!-- RECONCILE status=FAIL problems=%d -->' % len(problems))
    else:
        alarm_html = ('<!-- RECONCILE status=OK problems=0 '
                      'checked=%d log(s) -->' % len(data.get("per_log", [])))

    p = []
    p.append(f"""<!DOCTYPE html>
<!-- COMPSYS-PARITY-REPORT-SCHEMA
Every row is one CELL: a single (zstyle-combo, key-sequence, command-line) run
through a real pty against both shells, screens diffed cell-for-cell.
  harness    native = `zshrs -f -i`; zsh = `zshrs --zsh -f -i` (emulation path)
  combo      zstyle fixture the run sourced (scripts/parity_combos/<combo>.zsh)
  case       corpus case name (scripts/parity_corpus.py CASES)
  buffer     the command line typed before the keys
  sequence   named key sequence (scripts/parity_corpus.py KEY_SEQUENCES)
  keys       the keys that sequence sends, in order
  status     PASS    = grids byte-identical
             FAIL    = grids differ, reproduced on the confirm run
             FLAKY   = differed once, passed on re-run -> still a failure
             TIMEOUT = a side ran out of MEASUREMENT budget, so its screen was
                       never final. Not a divergence and NOT a pass: the cell
                       produced no evidence either way. Re-run at --jobs 1.
             SKIP    = the case never ran here (its command is not installed).
                       Also not a pass.
  detail     harness explanation (row count that differs, crash, boot failure)
TIMEOUT and SKIP stay in the `Cells Run` denominator. `Pass Rate` is
pass/all-rows; `Measured` is the subset that actually produced a comparison.
random-combo rows are SUBSETS of the live fixture, drawn by (seed, index):
  statements how many zstyle statements the subset kept
  minimal    after delta-debugging, how many still reproduce the divergence
  config_independent  true = still diverges with ZERO styles set, so the
                      zstyle combination is not the cause
An HTML comment `<!-- RECONCILE status=OK|FAIL ... -->` records whether the
rows on this page add up to the totals the harnesses printed for themselves.
-->
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="dark light">
<title>zshrs &mdash; compsys parity report</title>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Orbitron:wght@400;600;700;900&family=Share+Tech+Mono&display=swap" rel="stylesheet">
<link rel="stylesheet" href="hud-static.css">
<link rel="stylesheet" href="tutorial.css">
<style>{EXTRA_CSS}</style>
</head>
<body>
<header class="tutorial-header">
  <div class="tutorial-header-inner">
    <div>
      <h1 class="tutorial-brand">// ZSHRS &mdash; COMPSYS PARITY REPORT</h1>
      <nav class="tutorial-crumbs" aria-label="Breadcrumb">
        <span class="current">Compsys Parity Report</span>
        <span class="sep">/</span>
        <a href="index.html">zshrs Docs</a>
        <span class="sep">/</span>
        {port_crumb}<a href="port_report.html">C Port Report</a>
        <span class="sep">/</span>
        <a href="https://github.com/MenkeTechnologies/zshrs" target="_blank" rel="noopener noreferrer">GitHub</a>
      </nav>
      <p style="margin:0.35rem 0 0;font-family:'Share Tech Mono',monospace;font-size:11px;color:var(--text-dim);letter-spacing:0.03em;opacity:0.8;">
        Runtime completion parity: every cell drives <code>zsh -f -i</code> and
        <code>zshrs</code> through a real pseudo-terminal with the SAME zstyle
        config and compinit dump, replays a key sequence, and diffs the rendered
        screens cell-for-cell. A cell passes only when the two grids are
        byte-identical. Generated {datetime.now(timezone.utc).isoformat(timespec='seconds')}
        from {len(data['sources'])} harness log(s).
      </p>
    </div>
  </div>
</header>
<main class="tutorial-main">
<h2 class="tutorial-title"><span class="step-hash">&gt;_</span>SUMMARY</h2>
{alarm_html}
{gap_html}
<div class="stat-grid">
  <div class="stat-card"><div class="stat-val">{t['cells']:,}</div><div class="stat-label">Cells Run</div></div>
  <div class="stat-card"><div class="stat-val green">{t['pass']:,}</div><div class="stat-label">Byte-Identical</div></div>
  <div class="stat-card"><div class="stat-val red">{t['fail']:,}</div><div class="stat-label">Diverged</div></div>
  <div class="stat-card"><div class="stat-val yellow">{t['flaky']:,}</div><div class="stat-label">Flaky</div></div>
  <div class="stat-card"><div class="stat-val{' yellow' if t['timeout'] else ' gray'}">{t['timeout']:,}</div><div class="stat-label">Timeout (no verdict)</div></div>
  <div class="stat-card"><div class="stat-val gray">{t['skip']:,}</div><div class="stat-label">Skipped (not run)</div></div>
  <div class="stat-card"><div class="stat-val{' green' if t['pct'] == 100 else ''}">{t['pct']:.1f}%</div><div class="stat-label">Pass Rate{partial_note}</div></div>
  <div class="stat-card"><div class="stat-val">{t['measured']:,}</div><div class="stat-label">Cells Actually Compared</div></div>
  <div class="stat-card"><div class="stat-val">{t['combos']:,}</div><div class="stat-label">zstyle Combos</div></div>
  <div class="stat-card"><div class="stat-val">{t['cases']}/{t['corpus_cases']}</div><div class="stat-label">Cases Covered</div></div>
  <div class="stat-card"><div class="stat-val">{t['sequences']}/{t['corpus_sequences']}</div><div class="stat-label">Sequences Covered</div></div>
  <div class="stat-card"><div class="stat-val">{t['random_total']:,}</div><div class="stat-label">Random Subsets</div></div>
  <div class="stat-card"><div class="stat-val{' green' if t['random_fail'] == 0 else ' red'}">{t['random_fail']:,}</div><div class="stat-label">Subsets Diverged</div></div>
  <div class="stat-card"><div class="stat-val{' yellow' if t['random_timeout'] else ' gray'}">{t['random_timeout']:,}</div><div class="stat-label">Subsets Timed Out</div></div>
  <div class="stat-card"><div class="stat-val magenta">{t['random_indep']:,}</div><div class="stat-label">Config-Independent</div></div>
</div>
<!-- SYM totals cells={t['cells']} pass={t['pass']} fail={t['fail']} flaky={t['flaky']} -->
<!-- SYM totals timeout={t['timeout']} skip={t['skip']} measured={t['measured']}
     pct_all={t['pct']:.2f} pct_measured={t['pct_measured']:.2f} -->
""")


    # ── per-combo rollup ────────────────────────────────────────────────
    p.append('<h2 class="tutorial-title"><span class="step-hash">&gt;_</span>BY ZSTYLE COMBO</h2>')
    p.append('<table class="fn-table"><thead><tr>'
             '<th>Harness</th><th>Combo</th><th>Cells</th><th>Pass</th>'
             '<th>Fail</th><th>Flaky</th><th>Timeout</th><th>Skip</th>'
             '<th>Pass rate</th></tr></thead><tbody>')
    for r in combo_rows:
        pct = 100.0 * r["pass"] / r["cells"] if r["cells"] else 0.0
        p.append(
            f'<tr><td><code>{html.escape(r["harness"])}</code></td>'
            f'<td><code>{html.escape(r["combo"])}</code></td>'
            f'<td class="num">{r["cells"]}</td>'
            f'<td class="num">{r["pass"]}</td>'
            f'<td class="num">{r["fail"]}</td>'
            f'<td class="num">{r["flaky"]}</td>'
            f'<td class="num">{r["timeout"]}</td>'
            f'<td class="num">{r["skip"]}</td>'
            f'<td class="num">{pct:.1f}%</td></tr>'
            f'<!-- SYM harness={r["harness"]} combo={r["combo"]} cells={r["cells"]} '
            f'pass={r["pass"]} fail={r["fail"]} flaky={r["flaky"]} '
            f'timeout={r["timeout"]} skip={r["skip"]} -->'
        )
    p.append('</tbody></table>')

    # ── per-sequence rollup ─────────────────────────────────────────────
    #
    # EVERY sequence in the corpus gets a row, including the ones this run did
    # not touch. Omitting them made an unexercised keystroke class look like an
    # absent problem: the `tab_up` list-clear divergence was simply not on the
    # page, because only `tab_down` had been run.
    p.append('<h2 class="tutorial-title"><span class="step-hash">&gt;_</span>BY KEY SEQUENCE</h2>')
    p.append('<table class="fn-table"><thead><tr>'
             '<th>Sequence</th><th>Keys</th><th>Cells</th><th>Pass</th>'
             '<th>Fail</th><th>Flaky</th><th>Timeout</th><th>Skip</th>'
             '<th>Status</th></tr></thead><tbody>')
    per_seq = defaultdict(list)
    for cs in by_case.values():
        for c in cs:
            per_seq[c["sequence"]].append(c)
    # Sequences the corpus names, then any the logs carried that it does not
    # (a `compsys_parity.py` log puts a CASE name in this column, and an
    # ad-hoc/mutation run puts its own label there) — listing only the known
    # names is another way to drop rows off the page.
    seq_names = list(KEY_SEQUENCES) + sorted(set(per_seq) - set(KEY_SEQUENCES))
    for name in seq_names:
        keys = KEY_SEQUENCES.get(name, [])
        cs = per_seq.get(name, [])
        if not cs:
            p.append(
                f'<tr class="st-NOTRUN"><td><code>{html.escape(name)}</code></td>'
                f'<td><code>{html.escape("+".join(keys))}</code></td>'
                f'<td class="num">0</td><td class="num">-</td>'
                f'<td class="num">-</td><td class="num">-</td>'
                f'<td class="num">-</td><td class="num">-</td>'
                f'<td class="status"><span class="pill notrun">not run</span></td></tr>'
                f'<!-- SYM sequence={name} status=NOT-RUN cells=0 -->'
            )
            continue
        n = {v: sum(1 for c in cs if c["status"] == v) for v in VERDICTS}
        keys_shown = "+".join(keys) or (cs[0]["keys"] if cs[0]["keys"] else "-")
        # A sequence every one of whose cells timed out or was skipped has NOT
        # passed — it was never measured. Saying PASS there would turn budget
        # exhaustion into evidence of parity, which is the exact inversion this
        # whole category exists to prevent.
        if n["FAIL"] or n["FLAKY"]:
            st = "FAIL"
        elif n["PASS"] == 0:
            st = "INCONCLUSIVE"
        elif n["TIMEOUT"] or n["SKIP"]:
            st = "INCONCLUSIVE"
        else:
            st = "PASS"
        p.append(
            f'<tr class="st-{st}"><td><code>{html.escape(name)}</code></td>'
            f'<td><code>{html.escape(keys_shown)}</code></td>'
            f'<td class="num">{len(cs)}</td><td class="num">{n["PASS"]}</td>'
            f'<td class="num">{n["FAIL"]}</td><td class="num">{n["FLAKY"]}</td>'
            f'<td class="num">{n["TIMEOUT"]}</td><td class="num">{n["SKIP"]}</td>'
            f'<td class="status"><span class="pill {st.lower()}">{st}</span></td></tr>'
            f'<!-- SYM sequence={name} status={st} cells={len(cs)} '
            f'pass={n["PASS"]} fail={n["FAIL"]} flaky={n["FLAKY"]} '
            f'timeout={n["TIMEOUT"]} skip={n["SKIP"]} -->'
        )
    p.append('</tbody></table>')

    # ── per-case detail ─────────────────────────────────────────────────
    p.append('<h2 class="tutorial-title"><span class="step-hash">&gt;_</span>BY CASE</h2>')
    p.append('<table class="fn-table"><thead><tr>'
             '<th>Sequence</th><th>Keys</th><th>Combo</th><th>Status</th>'
             '<th>Detail</th></tr></thead><tbody>')
    for case in sorted(by_case):
        cs = by_case[case]
        nfail = sum(1 for c in cs if c["status"] in FAILING)
        nun = sum(1 for c in cs if c["status"] in UNMEASURED)
        npass = sum(1 for c in cs if c["status"] == "PASS")
        buf = cs[0]["buffer"]
        tags = "".join(f'<span class="pill tag">{html.escape(x)}</span> '
                       for x in cs[0]["tags"])
        p.append(f'<!-- BEGIN-GROUP case={case} -->')
        p.append(
            f'<tr class="grp-row" data-grp="{html.escape(case)}" '
            f'data-fail="{1 if (nfail or nun) else 0}">'
            f'<td class="grp-cell" colspan="5"><span class="grp-tog">[+]</span>'
            f'{html.escape(case)} &nbsp;<code>{html.escape(buf) or "&lt;empty&gt;"}</code> {tags}'
            f'<span class="counts">{npass}/{len(cs)} identical'
            + (f', {nun} never measured' if nun else '')
            + '</span></td></tr>'
            f'<!-- SYM case={case} cells={len(cs)} pass={npass} '
            f'failing={nfail} unmeasured={nun} -->'
        )
        for c in sorted(cs, key=lambda c: (c["combo"], c["sequence"])):
            pill = c["status"].lower()
            p.append(
                f'<tr class="detail-row hidden st-{c["status"]}" '
                f'data-grp="{html.escape(case)}">'
                f'<td><code>{html.escape(c["sequence"])}</code></td>'
                f'<td><code>{html.escape(c["keys"])}</code></td>'
                f'<td><code>{html.escape(c["combo"])}</code></td>'
                f'<td class="status"><span class="pill {pill}">{c["status"]}</span></td>'
                f'<td>{html.escape(c["detail"])}</td></tr>'
                f'<!-- SYM case={case} sequence={c["sequence"]} combo={c["combo"]} '
                f'status={c["status"]} keys={c["keys"]} -->'
            )
        p.append('<!-- END-GROUP -->')
    p.append('</tbody></table>')

    # ── random subsets ──────────────────────────────────────────────────
    if data["randoms"]:
        p.append('<h2 class="tutorial-title"><span class="step-hash">&gt;_</span>RANDOM ZSTYLE SUBSETS</h2>')
        p.append('<p style="font-family:\'Share Tech Mono\',monospace;font-size:11.5px;color:var(--text-dim);">'
                 'Every <code>zstyle</code> statement is independent, so any subset of the live '
                 'config is itself a valid config. These are seeded random subsets — the bar is '
                 'that <em>any</em> of them renders byte-identically, not just the curated axes '
                 'above. A diverging subset is delta-debugged down to the minimal set of '
                 'statements that still reproduces it.</p>')
        p.append('<table class="fn-table"><thead><tr>'
                 '<th>Combo</th><th>Statements</th><th>Status</th><th>Case</th>'
                 '<th>Minimal</th><th>Fixture</th></tr></thead><tbody>')
        for r in data["randoms"]:
            for c in r["combos"]:
                pill = c["status"].lower()
                indep = ('<span class="pill indep">config-independent</span>'
                         if c.get("config_independent") else "")
                minimal = "" if c["minimal"] is None else str(c["minimal"])
                p.append(
                    f'<tr class="st-{c["status"]}">'
                    f'<td class="num">{c["index"]}</td>'
                    f'<td class="num">{c["statements"]}</td>'
                    f'<td class="status"><span class="pill {pill}">{c["status"]}</span> {indep}</td>'
                    f'<td><code>{html.escape(c["case"])}</code>'
                    + (f' <span class="pill timeout">{html.escape(c["detail"])}</span>'
                       if c.get("detail") else '')
                    + f'</td>'
                    f'<td class="num">{minimal}</td>'
                    f'<td><code>{html.escape(os.path.basename(c["path"]))}</code></td></tr>'
                    f'<!-- SYM random_combo={c["index"]} statements={c["statements"]} '
                    f'status={c["status"]} minimal={minimal} '
                    f'config_independent={str(bool(c.get("config_independent"))).lower()} -->'
                )
        p.append('</tbody></table>')

    # ── provenance ──────────────────────────────────────────────────────
    p.append('<h2 class="tutorial-title"><span class="step-hash">&gt;_</span>PROVENANCE</h2>')
    p.append('<table class="fn-table"><thead><tr>'
             '<th>Log</th><th>Kind</th><th>Modified (UTC)</th><th>Bytes</th>'
             '</tr></thead><tbody>')
    for s in data["sources"]:
        p.append(f'<tr><td><code>{html.escape(s["path"])}</code></td>'
                 f'<td>{html.escape(s.get("kind", ""))}</td>'
                 f'<td>{html.escape(s["mtime"])}</td>'
                 f'<td class="num">{s["bytes"]:,}</td></tr>')
    p.append('</tbody></table>')

    meta_any = data["runs"][0]["meta"] if data["runs"] else {}
    if meta_any:
        p.append('<table class="fn-table"><thead><tr><th>Setting</th><th>Value</th>'
                 '</tr></thead><tbody>')
        for k in ("mode", "dump", "zstyle", "geom"):
            if k in meta_any:
                p.append(f'<tr><td><code>{k}</code></td>'
                         f'<td><code>{html.escape(meta_any[k])}</code></td></tr>')
        p.append('</tbody></table>')

    p.append('<script id="compsys-parity-report-data" type="application/json">')
    p.append(html.escape(json.dumps({"totals": t, "runs": data["runs"],
                                     "randoms": data["randoms"],
                                     "sources": data["sources"],
                                     "reconciliation": {
                                         "ok": not problems,
                                         "logs_checked": len(data.get("per_log", [])),
                                         "problems": problems,
                                     }}, indent=1)))
    p.append('</script>')
    p.append(f'</main>\n<script>{JS}</script>\n</body>\n</html>\n')
    return "\n".join(p)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--runs", default=DEFAULT_GLOB,
                    help=f"glob of harness output dirs (default: {DEFAULT_GLOB})")
    ap.add_argument("--out", type=Path, default=OUT)
    args = ap.parse_args()

    run_dirs = [p for p in sorted(ROOT.glob(args.runs)) if p.is_dir()]
    if not run_dirs:
        sys.exit(f"gen_compsys_parity_report: no run dirs match {args.runs!r} — "
                 "run scripts/parity_matrix.py first")

    data = collect(run_dirs)
    if not data["runs"] and not data["randoms"]:
        sys.exit("gen_compsys_parity_report: run dirs contain no parseable harness "
                 "output. Refusing to emit an empty report that would read as "
                 "'all green'.")

    t = totals_of(data)
    problems = reconcile(data)
    args.out.write_text(render(data, t, problems))
    print(f"{args.out.relative_to(ROOT)}: {t['cells']} cell(s), "
          f"{t['pass']} pass / {t['fail']} fail / {t['flaky']} flaky / "
          f"{t['timeout']} timeout / {t['skip']} skip "
          f"({t['pct']:.1f}% of all rows, {t['pct_measured']:.1f}% of the "
          f"{t['measured']} actually compared), {t['combos']} combo(s), "
          f"{t['random_total']} random subset(s)")
    if problems:
        # Loud on stderr AND non-zero, on top of the banner in the page: a
        # report whose rows do not add up to the harness's own totals is a
        # report that is dropping evidence, and it must not pass silently
        # through a docs pipeline.
        print("gen_compsys_parity_report: COUNTS DO NOT RECONCILE — this report "
              "is dropping or double-counting rows:", file=sys.stderr)
        for x in problems:
            print("  ! " + x, file=sys.stderr)
        print("  the page was still written, with a banner, so the discrepancy "
              "is visible rather than silent.", file=sys.stderr)
        return 2
    print(f"  reconciled: {len(data.get('per_log', []))} combo log(s) + "
          f"{len(data['randoms'])} random-combo log(s), rows match every "
          f"harness summary line")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
