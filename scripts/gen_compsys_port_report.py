#!/usr/bin/env python3
"""Regenerate docs/compsys_port_report.html.

Walks the upstream zsh ``Completion/`` shell tree and the project's
``compsys/ported/`` directory, and produces a styled HTML report
showing which upstream completion files have a Rust engine port
(``.rs``) and which have only a shell mirror copy.

The compsys port model (per zshrs2 design):

* Engine functions (``Base/{Completer,Core,Utility,Widget}``,
  ``Zsh/Context``, plus engine-only entries in ``Unix/Type``,
  ``Zsh/Type``) → ported to Rust under ``compsys/ported/<mirror>``.
* End-user shell completers (everything else — ``*/Command`` dirs,
  ``Zsh/Function``, end-user ``*/Type`` entries) → upstream shell
  files copied as-is into ``compsys/ported/<mirror>`` and dispatched
  via the ``_call_function`` bridge.

Bot/LLM/scraper-friendly output:

* ``<!-- BEGIN-GROUP dir=... -->`` / ``<!-- END-GROUP -->`` markers
  wrap each upstream directory.
* ``<script id="compsys-port-report-data" type="application/json">``
  embeds the entire dataset so consumers can ``grep -A1`` and parse
  without rendering HTML or running JS.
* Every file row carries a trailing ``<!-- SYM ... -->`` comment
  with all columns as ``key=value`` pairs.

Upstream location: defaults to ``$HOME/forkedRepos/zsh/Completion``;
override with ``--upstream`` flag or ``ZSHRS_UPSTREAM_COMPLETION``
env var.
"""
from __future__ import annotations

import argparse
import html
import json
import os
import sys
from collections import defaultdict
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PORTED = ROOT / "compsys" / "ported"
OUT = ROOT / "docs" / "compsys_port_report.html"

DEFAULT_UPSTREAM = Path(
    os.environ.get(
        "ZSHRS_UPSTREAM_COMPLETION",
        str(Path.home() / "forkedRepos" / "zsh" / "Completion"),
    )
)


def list_upstream(upstream: Path) -> dict[str, list[str]]:
    """dir-relpath -> [filename, ...] (sorted)."""
    out: dict[str, list[str]] = {}
    for sub in sorted(upstream.rglob("*")):
        if sub.is_dir():
            continue
        if sub.name.startswith("."):
            continue
        rel_dir = str(sub.parent.relative_to(upstream))
        if rel_dir == ".":
            rel_dir = "<top>"
        out.setdefault(rel_dir, []).append(sub.name)
    for k in out:
        out[k].sort()
    return out


# Rust files that are crate infrastructure (mod plumbing, shared helpers,
# top-level entries) rather than ports of an upstream shell function.
# Reported separately from drift candidates.
INFRA_RS = {
    "mod.rs",
    "shared.rs",
}

# Upstream Completion/ subdirs that hold engine code by definition. Files
# in any of these dirs are classified as engine regardless of port status.
ENGINE_DIRS = {
    "Base/Completer",
    "Base/Core",
    "Base/Utility",
    "Base/Widget",
    "Zsh/Context",
}

# Top-level engine entry scripts (siblings of Completion/<dir>/ — no
# subdir). Same classification rule as ENGINE_DIRS.
ENGINE_TOP_FILES = {
    "bashcompinit",
    "compaudit",
    "compdump",
    "compinit",
    "compinstall",
}


def is_engine(dir_: str, name: str, rs_present: bool) -> bool:
    """A file is engine if it sits in an engine dir, is a top-level engine
    script, or has been ported to Rust (engine = anything we ported)."""
    if dir_ in ENGINE_DIRS:
        return True
    if dir_ == "<top>" and name in ENGINE_TOP_FILES:
        return True
    return rs_present


def list_ported() -> dict[str, dict[str, dict]]:
    """dir-relpath -> {filename -> {shell: True/False, rs: True/False, rs_lines, shell_lines}}."""
    out: dict[str, dict[str, dict]] = {}
    for sub in sorted(PORTED.rglob("*")):
        if sub.is_dir():
            continue
        if sub.name.startswith("."):
            continue
        if sub.suffix == ".rs" and sub.name in INFRA_RS:
            continue
        rel_dir = str(sub.parent.relative_to(PORTED))
        if rel_dir == ".":
            rel_dir = "<top>"
        bucket = out.setdefault(rel_dir, {})
        if sub.suffix == ".rs":
            stem = sub.stem
            entry = bucket.setdefault(stem, {"shell": False, "rs": False, "rs_lines": 0, "shell_lines": 0})
            entry["rs"] = True
            entry["rs_lines"] = sum(1 for _ in sub.read_text(errors="replace").splitlines())
        else:
            entry = bucket.setdefault(sub.name, {"shell": False, "rs": False, "rs_lines": 0, "shell_lines": 0})
            entry["shell"] = True
            entry["shell_lines"] = sum(1 for _ in sub.read_text(errors="replace").splitlines())
    return out


def build_rows(upstream_map: dict[str, list[str]], ported_map: dict[str, dict[str, dict]]):
    """Return list of (dir, filename, in_upstream, shell_present, rs_present, shell_lines, rs_lines)."""
    rows = []
    all_dirs = sorted(set(upstream_map) | set(ported_map))
    for d in all_dirs:
        names = set(upstream_map.get(d, []))
        names |= set(ported_map.get(d, {}).keys())
        for name in sorted(names):
            in_up = name in upstream_map.get(d, [])
            pe = ported_map.get(d, {}).get(name, {})
            rs_present = bool(pe.get("rs", False))
            rows.append(
                {
                    "dir": d,
                    "name": name,
                    "in_upstream": in_up,
                    "shell_present": bool(pe.get("shell", False)),
                    "rs_present": rs_present,
                    "shell_lines": pe.get("shell_lines", 0),
                    "rs_lines": pe.get("rs_lines", 0),
                    "engine": is_engine(d, name, rs_present),
                    "ported": rs_present,
                }
            )
    return rows


def summarize(rows):
    fresh = lambda: {
        "upstream": 0,
        "shell_mirrored": 0,
        "rust_ported": 0,
        "shell_only": 0,
        "rust_only_no_upstream": 0,
        "fully_covered": 0,
        "engine": 0,
        "end_user": 0,
        "engine_ported": 0,
        "engine_unported": 0,
    }
    by_dir: dict[str, dict] = defaultdict(fresh)
    totals = fresh()
    for r in rows:
        d = r["dir"]
        s = by_dir[d]
        if r["in_upstream"]:
            s["upstream"] += 1
            totals["upstream"] += 1
            if r["shell_present"]:
                s["shell_mirrored"] += 1
                totals["shell_mirrored"] += 1
            if r["rs_present"]:
                s["rust_ported"] += 1
                totals["rust_ported"] += 1
            if r["shell_present"] and not r["rs_present"]:
                s["shell_only"] += 1
                totals["shell_only"] += 1
            if r["shell_present"] and r["rs_present"]:
                s["fully_covered"] += 1
                totals["fully_covered"] += 1
            if r["engine"]:
                s["engine"] += 1
                totals["engine"] += 1
                if r["ported"]:
                    s["engine_ported"] += 1
                    totals["engine_ported"] += 1
                else:
                    s["engine_unported"] += 1
                    totals["engine_unported"] += 1
            else:
                s["end_user"] += 1
                totals["end_user"] += 1
        else:
            if r["rs_present"]:
                s["rust_only_no_upstream"] += 1
                totals["rust_only_no_upstream"] += 1
    return by_dir, totals


# Inline supplement to hud-static.css + tutorial.css — only the table /
# tag / dir-group bits that aren't in those shared stylesheets. Keep
# in sync with docs/port_report.html (the C-codebase port report) so
# both reports look the same.
EXTRA_CSS = """
  .tutorial-main { max-width: 96rem; }
  .stat-grid { display:grid;grid-template-columns:repeat(auto-fill,minmax(11rem,1fr));gap:0.6rem;margin:1rem 0; }
  .stat-card { border:1px solid var(--border);border-top:3px solid var(--cyan);background:var(--bg-card);padding:0.7rem 0.9rem;border-radius:2px;text-align:center; }
  .stat-card .stat-val { font-family:'Orbitron',sans-serif;font-size:22px;font-weight:900;color:var(--cyan);line-height:1.1;text-shadow:0 0 14px var(--cyan-glow); }
  .stat-card .stat-val.green   { color:var(--green); text-shadow:0 0 14px rgba(57,255,20,.35); }
  .stat-card .stat-val.red     { color:#ff6b6b; text-shadow:0 0 14px rgba(255,107,107,.35); }
  .stat-card .stat-val.yellow  { color:#ffb800; text-shadow:0 0 14px rgba(255,184,0,.35); }
  .stat-card .stat-val.magenta { color:#d300c5; text-shadow:0 0 14px rgba(211,0,197,.35); }
  .stat-card .stat-val.gray    { color:#8b949e; }
  .stat-card .stat-label { font-family:'Orbitron',sans-serif;font-size:9px;font-weight:700;letter-spacing:1.5px;text-transform:uppercase;color:var(--text-muted);margin-top:0.4rem; }

  table.fn-table { width:100%;border-collapse:collapse;font-size:11.5px;margin:0.8rem 0; }
  table.fn-table th { background:var(--bg-secondary);color:var(--cyan);font-family:'Orbitron',sans-serif;font-size:10px;font-weight:700;letter-spacing:1.2px;text-transform:uppercase;text-align:left;padding:7px 10px;border:1px solid var(--border); }
  table.fn-table td { padding:6px 10px;border:1px solid var(--border);color:var(--text-dim);vertical-align:top; }
  table.fn-table tr:hover td { background:var(--bg-hover); }
  table.fn-table code { font-size:11px;color:var(--accent-light);background:var(--bg-primary);padding:1px 4px;border-radius:2px; }
  table.fn-table a { color:var(--cyan);text-decoration:none; }
  table.fn-table a:hover { text-decoration:underline;color:#fff; }
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
    font-family:'Share Tech Mono',monospace;
    font-size:11.5px;color:var(--accent-light);
    background:var(--bg-primary);padding:1px 6px;
  }
  tr.grp-row td.grp-cell .counts {
    float:right;color:var(--text-muted);font-weight:400;
    font-family:'Share Tech Mono',monospace;font-size:10.5px;
    letter-spacing:0;text-transform:none;
  }
  tr.detail-row.hidden { display:none; }

  /* role + ported tag pills (single classes; status uses C-report scheme) */
  .pill { display:inline-block;padding:1px 7px;border-radius:2px;
    font-family:'Share Tech Mono',monospace;font-size:10.5px;font-weight:700;
    letter-spacing:0.5px;text-transform:uppercase; }
  .pill.engine  { background:rgba(57,255,20,.12);color:var(--green);border:1px solid rgba(57,255,20,.3); }
  .pill.enduser { background:rgba(139,148,158,.12);color:#8b949e;border:1px solid rgba(139,148,158,.3); }
  .pill.yes     { background:rgba(57,255,20,.12);color:var(--green);border:1px solid rgba(57,255,20,.3); }
  .pill.no      { background:rgba(139,148,158,.12);color:#8b949e;border:1px solid rgba(139,148,158,.3); }

  tr.st-engine-shell td.status { color:var(--green);font-weight:700; }
  tr.st-engine       td.status { color:var(--green);font-weight:700; }
  tr.st-shell        td.status { color:#ffb800;font-weight:700; }
  tr.st-missing      td.status { color:#d300c5;font-weight:700; }
  tr.st-rust-only    td.status { color:#ff6b6b;font-weight:700; }
"""

JS = """
// Collapse / expand each dir group when its header row is clicked.
document.querySelectorAll('tr.grp-row').forEach(g => {
  const dir = g.dataset.dir;
  const tog = g.querySelector('.grp-tog');
  const rows = document.querySelectorAll(`tr.detail-row[data-dir="${CSS.escape(dir)}"]`);
  g.addEventListener('click', () => {
    const open = g.classList.toggle('open');
    if (tog) tog.textContent = open ? '[-]' : '[+]';
    rows.forEach(r => r.classList.toggle('hidden', !open));
  });
});
// All groups start collapsed except the one in the URL hash.
const hashDir = location.hash.startsWith('#dir-') ? location.hash.slice(5) : null;
document.querySelectorAll('tr.grp-row').forEach(g => {
  if (g.dataset.dir.replaceAll('/', '_') === hashDir) {
    g.click();
    g.scrollIntoView({ block: 'start' });
  }
});
"""


def role_pill(row: dict) -> str:
    if row["engine"]:
        return '<span class="pill engine">engine</span>'
    return '<span class="pill enduser">end-user</span>'


def ported_pill(row: dict) -> str:
    if row["ported"]:
        return '<span class="pill yes">yes</span>'
    return '<span class="pill no">no</span>'


def status_label(row: dict) -> tuple[str, str]:
    """Return (row-class, visible-text)."""
    if not row["in_upstream"]:
        return ("st-rust-only", "rust-only")
    if row["rs_present"] and row["shell_present"]:
        return ("st-engine-shell", "engine + shell")
    if row["rs_present"]:
        return ("st-engine", "engine")
    if row["shell_present"]:
        return ("st-shell", "shell only")
    return ("st-missing", "missing")


def render(rows, by_dir, totals, upstream_path: Path) -> str:
    pieces = []
    pieces.append(f"""<!DOCTYPE html>
<!-- COMPSYS-PORT-REPORT-SCHEMA
columns per upstream file:
  dir                upstream Completion/ relative directory (e.g. "Base/Utility")
  name               filename (e.g. "_arguments")
  in_upstream        true iff name exists at upstream Completion/<dir>/<name>
  shell_present      true iff compsys/ported/<dir>/<name> exists (shell mirror)
  rs_present         true iff compsys/ported/<dir>/<name>.rs exists (Rust port)
  shell_lines        line count of shell mirror file (0 if absent)
  rs_lines           line count of Rust port file (0 if absent)
  engine             true iff this is engine code (dir in Base/Completer,
                     Base/Core, Base/Utility, Base/Widget, Zsh/Context;
                     top-level engine script; or has a .rs Rust port)
  ported             true iff this file has a Rust port (== rs_present)
status legend:
  engine + shell     Rust port + upstream shell mirror both present
  engine             Rust port present, shell absent
  shell only         shell mirror present, no Rust port
  missing            upstream file present but not mirrored or ported
  rust-only          Rust file with no matching upstream entry (potential drift)
-->
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="dark light">
<title>zshrs &mdash; compsys port report</title>
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
      <h1 class="tutorial-brand">// ZSHRS &mdash; COMPSYS PORT REPORT</h1>
      <nav class="tutorial-crumbs" aria-label="Breadcrumb">
        <span class="current">Compsys Port Report</span>
        <span class="sep">/</span>
        <a href="index.html">zshrs Docs</a>
        <span class="sep">/</span>
        <a href="port_report.html">C Port Report</a>
        <span class="sep">/</span>
        <a href="https://github.com/MenkeTechnologies/zshrs" target="_blank" rel="noopener noreferrer">GitHub</a>
      </nav>
      <p style="margin:0.35rem 0 0;font-family:'Share Tech Mono',monospace;font-size:11px;color:var(--text-dim);letter-spacing:0.03em;opacity:0.8;">
        Per-file map of every upstream <code>Completion/</code> shell function against its
        Rust port in <code>compsys/ported/</code>. Engine functions
        (<code>Base/{{Completer,Core,Utility,Widget}}</code>, <code>Zsh/Context</code>,
        engine-only entries in <code>Unix/Type</code>, <code>Zsh/Type</code>,
        <code>{{Unix,Zsh}}/Command</code>, plus top-level engine scripts)
        are ported to Rust. End-user shell completers are copied as-is and
        dispatched via the <code>_call_function</code> bridge.
        Generated {date.today().isoformat()} &mdash; upstream
        <code>{html.escape(str(upstream_path))}</code>.
      </p>
    </div>
  </div>
</header>
<main class="tutorial-main">
<h2 class="tutorial-title"><span class="step-hash">&gt;_</span>SUMMARY</h2>
<div class="stat-grid">
  <div class="stat-card"><div class="stat-val">{totals['upstream']:,}</div><div class="stat-label">Upstream Files</div></div>
  <div class="stat-card"><div class="stat-val green">{totals['engine']:,}</div><div class="stat-label">Engine Files</div></div>
  <div class="stat-card"><div class="stat-val gray">{totals['end_user']:,}</div><div class="stat-label">End-User Files</div></div>
  <div class="stat-card"><div class="stat-val green">{totals['engine_ported']:,}</div><div class="stat-label">Engine Ported (Rust)</div></div>
  <div class="stat-card"><div class="stat-val yellow">{totals['engine_unported']:,}</div><div class="stat-label">Engine Not Yet Ported</div></div>
  <div class="stat-card"><div class="stat-val">{totals['shell_mirrored']:,}</div><div class="stat-label">Shell Mirrored</div></div>
  <div class="stat-card"><div class="stat-val green">{totals['fully_covered']:,}</div><div class="stat-label">Engine + Shell Both</div></div>
  <div class="stat-card"><div class="stat-val">{totals['shell_only']:,}</div><div class="stat-label">Shell Only</div></div>
  <div class="stat-card"><div class="stat-val {'green' if totals['rust_only_no_upstream'] == 0 else 'red'}">{totals['rust_only_no_upstream']:,}</div><div class="stat-label">Rust-Only (Drift)</div></div>
</div>

<h2 class="tutorial-title"><span class="step-hash">&gt;_</span>PER-DIRECTORY COVERAGE</h2>
<table class="fn-table">
<thead><tr>
  <th>dir</th><th class="num">upstream</th><th class="num">engine</th>
  <th class="num">end-user</th><th class="num">engine ported</th>
  <th class="num">shell mirrored</th><th class="num">rust-only</th>
</tr></thead>
<tbody>""")
    for d in sorted(by_dir):
        s = by_dir[d]
        anchor = f"dir-{d.replace('/', '_')}"
        pieces.append(
            f"<tr><td><a href=\"#{html.escape(anchor)}\"><code>{html.escape(d)}</code></a></td>"
            f"<td class=\"num\">{s['upstream']}</td>"
            f"<td class=\"num\">{s['engine']}</td>"
            f"<td class=\"num\">{s['end_user']}</td>"
            f"<td class=\"num\">{s['engine_ported']}</td>"
            f"<td class=\"num\">{s['shell_mirrored']}</td>"
            f"<td class=\"num\">{s['rust_only_no_upstream']}</td></tr>"
        )
    pieces.append("</tbody></table>")

    pieces.append('<h2 class="tutorial-title"><span class="step-hash">&gt;_</span>PER-FILE DETAIL</h2>')
    pieces.append('<p style="font-family:\'Share Tech Mono\',monospace;font-size:11px;color:var(--text-muted);">'
                  'Click a dir group to expand the per-file rows. Groups link from the summary table above.'
                  '</p>')

    grouped: dict[str, list[dict]] = defaultdict(list)
    for r in rows:
        grouped[r["dir"]].append(r)

    pieces.append('<table class="fn-table"><thead><tr>'
                  '<th>file</th><th>role</th><th>ported</th>'
                  '<th class="status">status</th>'
                  '<th class="num">shell L</th><th class="num">rust L</th>'
                  '</tr></thead><tbody>')

    for d in sorted(grouped):
        s = by_dir[d]
        anchor = f"dir-{d.replace('/', '_')}"
        pieces.append(f"<!-- BEGIN-GROUP dir={d} -->")
        pieces.append(
            f'<tr class="grp-row" id="{html.escape(anchor)}" data-dir="{html.escape(d)}">'
            f'<td class="grp-cell" colspan="6">'
            f'<span class="grp-tog">[+]</span> <code>{html.escape(d)}</code>'
            f'<span class="counts">{s["upstream"]} upstream &middot; '
            f'{s["engine"]} engine &middot; {s["end_user"]} end-user &middot; '
            f'{s["engine_ported"]} ported</span></td></tr>'
        )
        for r in grouped[d]:
            scls, slabel = status_label(r)
            cmt = (
                f"<!-- SYM dir={r['dir']} name={r['name']} "
                f"in_upstream={str(r['in_upstream']).lower()} "
                f"shell_present={str(r['shell_present']).lower()} "
                f"rs_present={str(r['rs_present']).lower()} "
                f"engine={str(r['engine']).lower()} "
                f"ported={str(r['ported']).lower()} "
                f"shell_lines={r['shell_lines']} rs_lines={r['rs_lines']} -->"
            )
            pieces.append(
                f'<tr class="detail-row {scls} hidden" data-dir="{html.escape(d)}">'
                f'<td><code>{html.escape(r["name"])}</code></td>'
                f'<td>{role_pill(r)}</td>'
                f'<td>{ported_pill(r)}</td>'
                f'<td class="status">{slabel}</td>'
                f'<td class="num">{r["shell_lines"] or ""}</td>'
                f'<td class="num">{r["rs_lines"] or ""}</td></tr>{cmt}'
            )
        pieces.append(f"<!-- END-GROUP dir={d} -->")
    pieces.append("</tbody></table>")

    payload = {"totals": totals, "by_dir": by_dir, "rows": rows, "upstream": str(upstream_path)}
    pieces.append(
        '<script id="compsys-port-report-data" type="application/json">'
        + html.escape(json.dumps(payload, indent=2, sort_keys=True))
        + "</script>"
    )
    pieces.append(f"<script>{JS}</script>")
    pieces.append("</main></body></html>")
    return "\n".join(pieces)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--upstream",
        type=Path,
        default=DEFAULT_UPSTREAM,
        help=f"upstream zsh Completion dir (default: {DEFAULT_UPSTREAM})",
    )
    ap.add_argument(
        "--out",
        type=Path,
        default=OUT,
        help=f"output HTML path (default: {OUT.relative_to(ROOT)})",
    )
    args = ap.parse_args()

    if not args.upstream.is_dir():
        print(f"upstream not found: {args.upstream}", file=sys.stderr)
        print(
            "set ZSHRS_UPSTREAM_COMPLETION or pass --upstream",
            file=sys.stderr,
        )
        return 1
    if not PORTED.is_dir():
        print(f"ported dir not found: {PORTED}", file=sys.stderr)
        return 1

    upstream_map = list_upstream(args.upstream)
    ported_map = list_ported()
    rows = build_rows(upstream_map, ported_map)
    by_dir, totals = summarize(rows)
    html_doc = render(rows, by_dir, totals, args.upstream)

    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(html_doc)
    print(
        f"wrote {args.out.relative_to(ROOT)}  ·  "
        f"upstream={totals['upstream']}  shell={totals['shell_mirrored']}  "
        f"rust={totals['rust_ported']}  both={totals['fully_covered']}  "
        f"rust-only={totals['rust_only_no_upstream']}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
