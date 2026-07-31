#!/usr/bin/env python3
"""Regenerate docs/compsys_parity_report.html.

Sibling of ``gen_compsys_port_report.py``. That report answers "which upstream
``Completion/`` files have a Rust port"; this one answers the question that
actually gates a switch-over: **does the completion ENGINE behave
byte-identically to zsh at the terminal**.

Input is the output of real harness runs — nothing here is hand-maintained:

* ``target/parity-matrix*/native.<combo>.log`` — one
  ``scripts/comptab_parity.py`` run per zstyle combo, one line per
  (key-sequence, case) cell: ``PASS``/``FAIL``/``FLAKY``.
* ``target/parity-matrix*/random-combos.log`` — random SUBSETS of the live
  fixture, each shrunk to a minimal diverging statement set when it fails.

The generator REFUSES to emit a report when it finds no run data, rather than
rendering an empty page that reads as "all green". Every number on the page is
derived from the parsed logs; the provenance block records which logs, their
mtimes, and what was NOT covered.

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

CELL_RE = re.compile(r"^(PASS|FAIL|FLAKY)\s+(\S+)\s+(.*?)(?:\s\s+\((.*)\))?$")
HDR_RE = re.compile(r"^# (\w+)\s*: (.*)$")
SUMMARY_RE = re.compile(r"^# (\d+) passed, (\d+) failed, (\d+) cell")
RC_FAIL_RE = re.compile(r"^FAIL combo (\d+)\s+\(\s*(\d+) statements\) on (.*)$")
RC_MIN_RE = re.compile(r"^\s+minimal set: (\d+) statement\(s\) -> (.*)$")
RC_INDEP_RE = re.compile(r"^\s+config-INDEPENDENT")
RC_PASS_RE = re.compile(r"^PASS combo (\d+)\s+\(\s*(\d+) statements\)")

# Buffer -> case name, so a log line can be mapped back to the corpus entry.
BY_BUFFER = {c.buffer: c for c in CASES}


def parse_combo_log(path: Path) -> dict:
    """One `comptab_parity.py --sequences ...` run."""
    meta: dict[str, str] = {}
    cells: list[dict] = []
    for line in path.read_text(errors="replace").splitlines():
        m = HDR_RE.match(line)
        if m:
            meta[m.group(1)] = m.group(2).strip()
            continue
        m = CELL_RE.match(line)
        if m and m.group(2) in KEY_SEQUENCES or (m and m.group(2) == "adhoc"):
            status, seq, buf_repr, detail = m.groups()
            try:
                buf = eval(buf_repr, {"__builtins__": {}})  # repr() of a str
            except Exception:
                buf = buf_repr
            case = BY_BUFFER.get(buf)
            cells.append({
                "status": status,
                "sequence": seq,
                "keys": "+".join(KEY_SEQUENCES.get(seq, [])),
                "buffer": buf,
                "case": case.name if case else "(ad-hoc)",
                "tags": list(case.tags) if case else [],
                "note": case.note if case else "",
                "detail": detail or "",
            })
    return {"meta": meta, "cells": cells}


def parse_random_log(path: Path) -> dict:
    """The random-subset fuzz run."""
    meta: dict[str, str] = {}
    combos: list[dict] = []
    cur: dict | None = None
    for line in path.read_text(errors="replace").splitlines():
        m = HDR_RE.match(line)
        if m:
            meta[m.group(1)] = m.group(2).strip()
        m = RC_PASS_RE.match(line)
        if m:
            combos.append({"index": int(m.group(1)), "statements": int(m.group(2)),
                           "status": "PASS", "case": "", "minimal": None,
                           "config_independent": False, "path": ""})
            cur = None
            continue
        m = RC_FAIL_RE.match(line)
        if m:
            cur = {"index": int(m.group(1)), "statements": int(m.group(2)),
                   "status": "FAIL", "case": m.group(3).strip(), "minimal": None,
                   "config_independent": False, "path": ""}
            combos.append(cur)
            continue
        if cur is not None:
            if RC_INDEP_RE.match(line):
                cur["config_independent"] = True
            m = RC_MIN_RE.match(line)
            if m:
                cur["minimal"] = int(m.group(1))
                cur["path"] = m.group(2).strip()
    return {"meta": meta, "combos": combos}


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
            parsed = parse_combo_log(log)
            if not parsed["cells"]:
                continue
            # native.<combo>.log / zsh.<combo>.log, and the sharded form
            # native.<combo>.shard<N>.log that `parity_matrix.py --jobs` emits.
            # Shards are slices of the SAME combo run, so they must merge back
            # into one combo or the per-combo rollup double-counts it as N
            # separate configs.
            parts = log.stem.split(".", 1)
            parsed["harness"] = parts[0]
            combo = parts[1] if len(parts) > 1 else log.stem
            combo = re.sub(r"\.shard\d+$", "", combo)
            parsed["combo"] = combo
            parsed["source"] = src
            runs.append(parsed)
            sources.append(src | {"kind": "combo", "combo": parsed["combo"]})
    merged: dict[tuple[str, str], dict] = {}
    for r in runs:
        key = (r["harness"], r["combo"])
        if key in merged:
            merged[key]["cells"].extend(r["cells"])
            merged[key]["meta"].update(r["meta"])
        else:
            merged[key] = r
    return {"runs": list(merged.values()), "randoms": randoms, "sources": sources}


def totals_of(data: dict) -> dict:
    cells = [c for r in data["runs"] for c in r["cells"]]
    t = {
        "cells": len(cells),
        "pass": sum(1 for c in cells if c["status"] == "PASS"),
        "fail": sum(1 for c in cells if c["status"] == "FAIL"),
        "flaky": sum(1 for c in cells if c["status"] == "FLAKY"),
        "combos": len({r["combo"] for r in data["runs"]}),
        "cases": len({c["case"] for c in cells}),
        "sequences": len({c["sequence"] for c in cells}),
        "corpus_cases": len(CASES),
        "corpus_sequences": len(KEY_SEQUENCES),
    }
    t["pct"] = (100.0 * t["pass"] / t["cells"]) if t["cells"] else 0.0
    rc = [c for r in data["randoms"] for c in r["combos"]]
    t["random_total"] = len(rc)
    t["random_pass"] = sum(1 for c in rc if c["status"] == "PASS")
    t["random_fail"] = sum(1 for c in rc if c["status"] == "FAIL")
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
  .pill.tag { background:rgba(139,148,158,.12);color:#8b949e;border:1px solid rgba(139,148,158,.3); }
  .pill.indep { background:rgba(211,0,197,.12);color:#d300c5;border:1px solid rgba(211,0,197,.3); }

  tr.st-PASS  td.status { color:var(--green);font-weight:700; }
  tr.st-FAIL  td.status { color:#ff6b6b;font-weight:700; }
  tr.st-FLAKY td.status { color:#ffb800;font-weight:700; }
  tr.st-NOTRUN td { color:#8b949e;opacity:0.75; }
  .pill.notrun { background:rgba(139,148,158,.12);color:#8b949e;border:1px solid rgba(139,148,158,.3); }

  .note { border:1px solid var(--border);border-left:3px solid #ffb800;
    background:var(--bg-card);padding:0.6rem 0.9rem;margin:0.9rem 0;
    font-family:'Share Tech Mono',monospace;font-size:11.5px;color:var(--text-dim); }
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


def render(data: dict, t: dict) -> str:
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
        combo_rows.append({
            "harness": r["harness"], "combo": r["combo"],
            "pass": sum(1 for c in cs if c["status"] == "PASS"),
            "fail": sum(1 for c in cs if c["status"] == "FAIL"),
            "flaky": sum(1 for c in cs if c["status"] == "FLAKY"),
            "cells": len(cs),
        })

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
  status     PASS  = grids byte-identical
             FAIL  = grids differ, reproduced on the confirm run
             FLAKY = differed once, passed on re-run -> still counted a failure
  detail     harness explanation (row count that differs, crash, boot failure)
random-combo rows are SUBSETS of the live fixture, drawn by (seed, index):
  statements how many zstyle statements the subset kept
  minimal    after delta-debugging, how many still reproduce the divergence
  config_independent  true = still diverges with ZERO styles set, so the
                      zstyle combination is not the cause
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
{gap_html}
<div class="stat-grid">
  <div class="stat-card"><div class="stat-val">{t['cells']:,}</div><div class="stat-label">Cells Run</div></div>
  <div class="stat-card"><div class="stat-val green">{t['pass']:,}</div><div class="stat-label">Byte-Identical</div></div>
  <div class="stat-card"><div class="stat-val red">{t['fail']:,}</div><div class="stat-label">Diverged</div></div>
  <div class="stat-card"><div class="stat-val yellow">{t['flaky']:,}</div><div class="stat-label">Flaky</div></div>
  <div class="stat-card"><div class="stat-val{' green' if t['pct'] == 100 else ''}">{t['pct']:.1f}%</div><div class="stat-label">Pass Rate{partial_note}</div></div>
  <div class="stat-card"><div class="stat-val">{t['combos']:,}</div><div class="stat-label">zstyle Combos</div></div>
  <div class="stat-card"><div class="stat-val">{t['cases']}/{t['corpus_cases']}</div><div class="stat-label">Cases Covered</div></div>
  <div class="stat-card"><div class="stat-val">{t['sequences']}/{t['corpus_sequences']}</div><div class="stat-label">Sequences Covered</div></div>
  <div class="stat-card"><div class="stat-val">{t['random_total']:,}</div><div class="stat-label">Random Subsets</div></div>
  <div class="stat-card"><div class="stat-val{' green' if t['random_fail'] == 0 else ' red'}">{t['random_fail']:,}</div><div class="stat-label">Subsets Diverged</div></div>
  <div class="stat-card"><div class="stat-val magenta">{t['random_indep']:,}</div><div class="stat-label">Config-Independent</div></div>
</div>
""")


    # ── per-combo rollup ────────────────────────────────────────────────
    p.append('<h2 class="tutorial-title"><span class="step-hash">&gt;_</span>BY ZSTYLE COMBO</h2>')
    p.append('<table class="fn-table"><thead><tr>'
             '<th>Harness</th><th>Combo</th><th>Cells</th><th>Pass</th>'
             '<th>Fail</th><th>Flaky</th><th>Pass rate</th></tr></thead><tbody>')
    for r in combo_rows:
        pct = 100.0 * r["pass"] / r["cells"] if r["cells"] else 0.0
        p.append(
            f'<tr><td><code>{html.escape(r["harness"])}</code></td>'
            f'<td><code>{html.escape(r["combo"])}</code></td>'
            f'<td class="num">{r["cells"]}</td>'
            f'<td class="num">{r["pass"]}</td>'
            f'<td class="num">{r["fail"]}</td>'
            f'<td class="num">{r["flaky"]}</td>'
            f'<td class="num">{pct:.1f}%</td></tr>'
            f'<!-- SYM harness={r["harness"]} combo={r["combo"]} cells={r["cells"]} '
            f'pass={r["pass"]} fail={r["fail"]} flaky={r["flaky"]} -->'
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
             '<th>Fail</th><th>Flaky</th><th>Status</th></tr></thead><tbody>')
    per_seq = defaultdict(list)
    for cs in by_case.values():
        for c in cs:
            per_seq[c["sequence"]].append(c)
    for name, keys in KEY_SEQUENCES.items():
        cs = per_seq.get(name, [])
        if not cs:
            p.append(
                f'<tr class="st-NOTRUN"><td><code>{html.escape(name)}</code></td>'
                f'<td><code>{html.escape("+".join(keys))}</code></td>'
                f'<td class="num">0</td><td class="num">-</td>'
                f'<td class="num">-</td><td class="num">-</td>'
                f'<td class="status"><span class="pill notrun">not run</span></td></tr>'
                f'<!-- SYM sequence={name} status=NOT-RUN cells=0 -->'
            )
            continue
        np_ = sum(1 for c in cs if c["status"] == "PASS")
        nf = sum(1 for c in cs if c["status"] == "FAIL")
        nx = sum(1 for c in cs if c["status"] == "FLAKY")
        st = "PASS" if nf == 0 and nx == 0 else "FAIL"
        p.append(
            f'<tr class="st-{st}"><td><code>{html.escape(name)}</code></td>'
            f'<td><code>{html.escape("+".join(keys))}</code></td>'
            f'<td class="num">{len(cs)}</td><td class="num">{np_}</td>'
            f'<td class="num">{nf}</td><td class="num">{nx}</td>'
            f'<td class="status"><span class="pill {st.lower()}">{st}</span></td></tr>'
            f'<!-- SYM sequence={name} status={st} cells={len(cs)} '
            f'pass={np_} fail={nf} flaky={nx} -->'
        )
    p.append('</tbody></table>')

    # ── per-case detail ─────────────────────────────────────────────────
    p.append('<h2 class="tutorial-title"><span class="step-hash">&gt;_</span>BY CASE</h2>')
    p.append('<table class="fn-table"><thead><tr>'
             '<th>Sequence</th><th>Keys</th><th>Combo</th><th>Status</th>'
             '<th>Detail</th></tr></thead><tbody>')
    for case in sorted(by_case):
        cs = by_case[case]
        nfail = sum(1 for c in cs if c["status"] != "PASS")
        buf = cs[0]["buffer"]
        tags = "".join(f'<span class="pill tag">{html.escape(x)}</span> '
                       for x in cs[0]["tags"])
        p.append(f'<!-- BEGIN-GROUP case={case} -->')
        p.append(
            f'<tr class="grp-row" data-grp="{html.escape(case)}" '
            f'data-fail="{1 if nfail else 0}">'
            f'<td class="grp-cell" colspan="5"><span class="grp-tog">[+]</span>'
            f'{html.escape(case)} &nbsp;<code>{html.escape(buf) or "&lt;empty&gt;"}</code> {tags}'
            f'<span class="counts">{len(cs) - nfail}/{len(cs)} identical</span>'
            f'</td></tr>'
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
                    f'<td><code>{html.escape(c["case"])}</code></td>'
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
                                     "sources": data["sources"]}, indent=1)))
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
    args.out.write_text(render(data, t))
    print(f"{args.out.relative_to(ROOT)}: {t['cells']} cell(s), "
          f"{t['pass']} pass / {t['fail']} fail / {t['flaky']} flaky "
          f"({t['pct']:.1f}%), {t['combos']} combo(s), "
          f"{t['random_total']} random subset(s)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
