#!/usr/bin/env python3
"""Regenerate the ZSH CORPUS section inside docs/index.html.

Scans every ``*.zsh`` under the repo (excluding ``target/`` /
``node_modules/`` / ``.git/``), groups by top-level path, emits:

* totals + per-group LOC table
* full per-file inventory table (sortable, grouped)
* top N files by LOC embedded as collapsible <details> blocks

Replaces the block between
``<!-- BEGIN ZSH_CORPUS -->`` and ``<!-- END ZSH_CORPUS -->``
markers in docs/index.html, leaving the surrounding doc untouched.

Run: ``python3 scripts/gen_zsh_corpus.py``
"""
from __future__ import annotations

import html
from datetime import date
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
INDEX = ROOT / "docs" / "index.html"
SKIP_DIR_PARTS = {"target", "node_modules", ".git"}
EMBED_TOP_N = 10
EMBED_LINE_CAP = 800  # truncate embedded sources beyond this many lines

BEGIN = "<!-- BEGIN ZSH_CORPUS -->"
END = "<!-- END ZSH_CORPUS -->"


def is_skipped(p: Path) -> bool:
    return any(part in SKIP_DIR_PARTS for part in p.parts)


def group_key(rel: Path) -> str:
    """Bucket relative path into a display group.

    tests/* keeps the second segment (tests/parity_corpus, etc.);
    src/* keeps the second segment (src/zsh); everything else uses
    the first segment.
    """
    parts = rel.parts
    if len(parts) == 1:
        return "(root)"
    if parts[0] in ("tests", "src") and len(parts) >= 3:
        return f"{parts[0]}/{parts[1]}"
    return parts[0]


def collect() -> tuple[list[tuple[Path, int]], dict[str, tuple[int, int]]]:
    """(files_with_loc_sorted_desc, group_stats)."""
    files: list[tuple[Path, int]] = []
    for p in sorted(ROOT.rglob("*.zsh")):
        rel = p.relative_to(ROOT)
        if is_skipped(rel):
            continue
        try:
            with p.open("rb") as f:
                loc = sum(1 for _ in f)
        except OSError:
            continue
        files.append((rel, loc))
    files.sort(key=lambda t: (-t[1], str(t[0])))

    groups: dict[str, list[int]] = {}
    for rel, loc in files:
        groups.setdefault(group_key(rel), []).append(loc)
    group_stats = {g: (len(v), sum(v)) for g, v in groups.items()}
    return files, group_stats


def fmt_int(n: int) -> str:
    return f"{n:,}"


def render_group_table(group_stats: dict[str, tuple[int, int]],
                       total_files: int, total_loc: int) -> str:
    rows = []
    ordered = sorted(group_stats.items(), key=lambda kv: -kv[1][1])
    for g, (cnt, loc) in ordered:
        pct = (loc / total_loc * 100.0) if total_loc else 0.0
        rows.append(
            f"  <tr><td><code>{html.escape(g)}</code></td>"
            f"<td class=\"num\">{fmt_int(cnt)}</td>"
            f"<td class=\"num\">{fmt_int(loc)}</td>"
            f"<td class=\"num\">{pct:5.1f}%</td></tr>"
        )
    rows.append(
        "  <tr class=\"zc-total\"><td><strong>TOTAL</strong></td>"
        f"<td class=\"num\"><strong>{fmt_int(total_files)}</strong></td>"
        f"<td class=\"num\"><strong>{fmt_int(total_loc)}</strong></td>"
        "<td class=\"num\"><strong>100.0%</strong></td></tr>"
    )
    body = "\n".join(rows)
    return (
        "<table class=\"arch-table zc-table\">\n"
        "  <thead><tr><th>Group</th><th>Files</th><th>LOC</th><th>Share</th></tr></thead>\n"
        f"  <tbody>\n{body}\n  </tbody>\n"
        "</table>"
    )


def render_file_table(files: list[tuple[Path, int]]) -> str:
    rows = []
    cur_group = None
    for rel, loc in sorted(files, key=lambda t: (group_key(t[0]), str(t[0]))):
        g = group_key(rel)
        if g != cur_group:
            rows.append(
                f"  <tr class=\"zc-group-row\"><td colspan=\"2\">"
                f"<strong>{html.escape(g)}</strong></td></tr>"
            )
            cur_group = g
        rows.append(
            f"  <tr><td><code>{html.escape(str(rel))}</code></td>"
            f"<td class=\"num\">{fmt_int(loc)}</td></tr>"
        )
    body = "\n".join(rows)
    return (
        "<details class=\"zc-details\"><summary>"
        f"Full inventory &mdash; {fmt_int(len(files))} files (click to expand)"
        "</summary>\n"
        "<table class=\"arch-table zc-table zc-file-table\">\n"
        "  <thead><tr><th>Path</th><th>LOC</th></tr></thead>\n"
        f"  <tbody>\n{body}\n  </tbody>\n"
        "</table>\n"
        "</details>"
    )


def render_top_table(files: list[tuple[Path, int]], n: int) -> str:
    rows = []
    for rel, loc in files[:n]:
        rows.append(
            f"  <tr><td><code>{html.escape(str(rel))}</code></td>"
            f"<td class=\"num\">{fmt_int(loc)}</td></tr>"
        )
    body = "\n".join(rows)
    return (
        "<table class=\"arch-table zc-table\">\n"
        "  <thead><tr><th>Path</th><th>LOC</th></tr></thead>\n"
        f"  <tbody>\n{body}\n  </tbody>\n"
        "</table>"
    )


def render_embeds(files: list[tuple[Path, int]], n: int) -> str:
    parts = []
    for rel, loc in files[:n]:
        abs_path = ROOT / rel
        try:
            text = abs_path.read_text(encoding="utf-8", errors="replace")
        except OSError as e:
            parts.append(
                f"<p class=\"zc-err\">{html.escape(str(rel))}: read error: {html.escape(str(e))}</p>"
            )
            continue
        lines = text.splitlines()
        truncated = False
        if len(lines) > EMBED_LINE_CAP:
            shown = "\n".join(lines[:EMBED_LINE_CAP])
            truncated = True
        else:
            shown = text
        escaped = html.escape(shown)
        suffix = ""
        if truncated:
            suffix = (
                f"\n<span class=\"zc-trunc\">... truncated; "
                f"{fmt_int(len(lines) - EMBED_LINE_CAP)} more lines</span>"
            )
        parts.append(
            "<details class=\"zc-details zc-embed\">"
            f"<summary><code>{html.escape(str(rel))}</code> "
            f"&mdash; {fmt_int(loc)} LOC</summary>\n"
            f"<pre class=\"zc-src\"><code>{escaped}{suffix}</code></pre>\n"
            "</details>"
        )
    return "\n".join(parts)


def render_section(files: list[tuple[Path, int]],
                   group_stats: dict[str, tuple[int, int]]) -> str:
    total_files = sum(c for c, _ in group_stats.values())
    total_loc = sum(l for _, l in group_stats.values())
    today = date.today().isoformat()

    parts: list[str] = []
    parts.append(BEGIN)
    parts.append("<!-- generated by scripts/gen_zsh_corpus.py -->")
    parts.append("<style>")
    parts.append(".zc-table .num { text-align: right; font-family: 'Share Tech Mono', monospace; }")
    parts.append(".zc-total td { background: var(--bg-secondary); border-top: 2px solid var(--cyan); }")
    parts.append(".zc-group-row td { background: var(--bg-secondary); color: var(--cyan); font-family: 'Orbitron', sans-serif; font-size: 10px; letter-spacing: 1.5px; text-transform: uppercase; padding: 6px 10px; }")
    parts.append(".zc-file-table { font-size: 11px; }")
    parts.append(".zc-file-table tbody tr td { padding: 3px 10px; }")
    parts.append(".zc-details { margin: 0.6rem 0; }")
    parts.append(".zc-details > summary { cursor: pointer; padding: 0.4rem 0.7rem; background: var(--bg-secondary); border: 1px solid var(--border); border-left: 2px solid var(--cyan); font-family: 'Share Tech Mono', monospace; font-size: 12px; color: var(--text); }")
    parts.append(".zc-details > summary:hover { background: var(--bg-hover); }")
    parts.append(".zc-details[open] > summary { border-bottom-color: var(--cyan); }")
    parts.append(".zc-embed pre.zc-src { max-height: 32rem; overflow: auto; margin: 0; padding: 0.7rem 1rem; border: 1px solid var(--border); border-top: none; background: var(--bg); font-family: 'Share Tech Mono', ui-monospace, monospace; font-size: 11px; color: var(--text); line-height: 1.45; white-space: pre; }")
    parts.append(".zc-trunc { display: block; margin-top: 0.5rem; color: var(--text-muted); font-style: italic; }")
    parts.append(".zc-stats { display: grid; grid-template-columns: repeat(auto-fill, minmax(10rem, 1fr)); gap: 1rem; margin: 1rem 0; text-align: center; }")
    parts.append(".zc-stats .stat-val { font-family: 'Orbitron', sans-serif; font-size: 26px; font-weight: 900; color: var(--cyan); }")
    parts.append(".zc-stats .stat-label { font-size: 10px; color: var(--text-muted); letter-spacing: 1.5px; text-transform: uppercase; margin-top: 0.3rem; }")
    parts.append("</style>")

    largest_path, largest_loc = files[0]
    parts.append("<div class=\"zc-stats\">")
    parts.append(f"  <div><div class=\"stat-val\">{fmt_int(total_files)}</div><div class=\"stat-label\">.zsh files</div></div>")
    parts.append(f"  <div><div class=\"stat-val\">{fmt_int(total_loc)}</div><div class=\"stat-label\">total LOC</div></div>")
    parts.append(f"  <div><div class=\"stat-val\">{fmt_int(len(group_stats))}</div><div class=\"stat-label\">corpus groups</div></div>")
    parts.append(f"  <div><div class=\"stat-val\">{fmt_int(largest_loc)}</div><div class=\"stat-label\">largest file (LOC)</div></div>")
    parts.append("</div>")

    parts.append(f"<p class=\"docs-build-line\">Generated {today} by <code>scripts/gen_zsh_corpus.py</code>. "
                 "All counts derived live from <code>find . -name '*.zsh'</code>; never hand-edit numbers in this section.</p>")

    parts.append("<h3>Per-group totals</h3>")
    parts.append(render_group_table(group_stats, total_files, total_loc))

    parts.append(f"<h3>Top {EMBED_TOP_N} by LOC</h3>")
    parts.append(render_top_table(files, EMBED_TOP_N))

    parts.append("<h3>Full inventory</h3>")
    parts.append(render_file_table(files))

    parts.append(f"<h3>Embedded example scripts (top {EMBED_TOP_N} by LOC)</h3>")
    parts.append(f"<p style=\"margin:0.4rem 0 0.7rem;color:var(--text-dim);font-size:12px;\">"
                 "Full source of the largest corpus scripts. Each is collapsed by default; "
                 f"sources beyond {EMBED_LINE_CAP:,} lines are truncated with a count of trailing lines.</p>")
    parts.append(render_embeds(files, EMBED_TOP_N))

    parts.append(END)
    return "\n".join(parts)


def patch_index(section: str) -> None:
    src = INDEX.read_text(encoding="utf-8")
    if BEGIN not in src or END not in src:
        raise SystemExit(
            f"markers not found in {INDEX}; insert\n  {BEGIN}\n  {END}\n"
            "into the file at the desired location first."
        )
    pre, _, rest = src.partition(BEGIN)
    _, _, post = rest.partition(END)
    new = f"{pre}{section}{post}"
    if new != src:
        INDEX.write_text(new, encoding="utf-8")
        print(f"updated {INDEX}")
    else:
        print(f"{INDEX} unchanged")


def main() -> None:
    files, group_stats = collect()
    section = render_section(files, group_stats)
    patch_index(section)


if __name__ == "__main__":
    main()
