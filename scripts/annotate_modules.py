#!/usr/bin/env python3
"""Annotate every executable statement in src/ported/modules/*.rs with a
`// c:NNNN` reference back to Src/Modules/<base>.c, plus a
`/// Port of NAME() from Src/Modules/<base>.c:NNNN` doc-comment on
each ported fn signature.

Heuristic: per rust fn, locate the matching C fn by name. For each
non-empty / non-pure-comment line inside the rust body, pick the
C body line with the highest token-overlap as the c:NNNN target.
Fall back to the C fn's start line.

Idempotent: skips files / lines that already carry `// c:` tags.
"""
from __future__ import annotations
import re, sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
RS_DIR = ROOT / "src" / "ported" / "modules"
C_DIR = ROOT / "src" / "zsh" / "Src" / "Modules"

C_KW = {
    "if","for","while","switch","return","else","do","sizeof","static",
    "extern","struct","union","enum","typedef","const","volatile","inline",
    "register","auto","goto","break","continue","case","default","NULL",
    "void","int","char","long","short","unsigned","signed","float","double",
    "size_t","ssize_t","FILE","TRUE","FALSE",
}
RS_KW = {
    "if","else","let","mut","fn","return","for","while","loop","match",
    "in","as","ref","pub","self","Self","Some","None","Ok","Err","true",
    "false","String","str","i32","u32","i64","usize","Vec","HashMap",
    "to_string","unwrap","clone",
}
COMMON = C_KW | RS_KW

TOK = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
def toks(line: str) -> set[str]:
    return {t for t in TOK.findall(line) if t not in COMMON and len(t) > 2}

# ── C indexer ────────────────────────────────────────────────────────────────
RE_C_FN = re.compile(r"^([A-Za-z_][A-Za-z0-9_]*)\s*\(")

def index_c(c_path: Path) -> dict[str, dict]:
    """name -> {start, end, body_lines: [(lineno, text)]}"""
    out: dict[str, dict] = {}
    if not c_path.exists():
        return out
    lines = c_path.read_text(errors="replace").splitlines()
    n = len(lines)
    i = 0
    while i < n:
        line = lines[i]
        if line and not line[0].isspace() and not line.startswith(("/", "*", "#")):
            m = RE_C_FN.match(line)
            if m and m.group(1) not in C_KW:
                # find opening brace within next 6 lines
                blk_open = -1
                for j in range(i, min(i+6, n)):
                    if "{" in lines[j] and ";" not in lines[j].split("{",1)[0]:
                        blk_open = j; break
                if blk_open < 0:
                    i += 1; continue
                # walk braces to find end
                depth = 0
                end = blk_open
                for j in range(blk_open, n):
                    for ch in lines[j]:
                        if ch == "{": depth += 1
                        elif ch == "}":
                            depth -= 1
                            if depth == 0:
                                end = j; break
                    if depth == 0 and j >= blk_open: break
                name = m.group(1)
                body = [(k+1, lines[k]) for k in range(blk_open+1, end)]
                if name not in out:
                    out[name] = {"start": i+1, "end": end+1, "body": body}
                i = end + 1
                continue
        i += 1
    return out

# ── Rust annotator ───────────────────────────────────────────────────────────
RE_RS_FN_SIG = re.compile(r"^(\s*)(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?(?:extern\s+\"[^\"]+\"\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\b")
RE_PORT_DOC = re.compile(r"Port(?:s|ed|ing)?\s+of\s+`?([A-Za-z_][A-Za-z0-9_]*)`?", re.IGNORECASE)

def candidate_c_names(rs_name: str) -> list[str]:
    """Generate C-name candidates for a rust fn name."""
    out = [rs_name]
    # builtin_foo  -> bin_foo, foo
    if rs_name.startswith("builtin_"):
        rest = rs_name[len("builtin_"):]
        out += [f"bin_{rest}", rest]
    # bin_zfoo -> bin_foo (strip leading 'z' on the builtin payload)
    m = re.match(r"^(bin|builtin)_z([a-z][a-zA-Z0-9_]*)$", rs_name)
    if m:
        out.append(f"bin_{m.group(2)}")
    # zfoo_handler -> foo_handler
    if rs_name.startswith("z") and len(rs_name) > 1 and rs_name[1].islower():
        out.append(rs_name[1:])
    return out

def find_rs_fns(text: str) -> list[tuple[int, int, str, str]]:
    """Return list of (sig_line, body_open_line, body_close_line, name, indent).
    Brace-balanced fn body."""
    lines = text.splitlines()
    out: list[tuple] = []
    i = 0
    while i < len(lines):
        m = RE_RS_FN_SIG.match(lines[i])
        if m:
            indent, name = m.group(1), m.group(2)
            # find first { (could be on this line or following)
            j = i
            depth = 0
            opened = False
            sig_end = i
            while j < len(lines):
                line = lines[j]
                for k, ch in enumerate(line):
                    if ch == "{":
                        if not opened: opened = True; sig_end = j
                        depth += 1
                    elif ch == "}":
                        depth -= 1
                        if opened and depth == 0:
                            out.append((i, sig_end, j, name, indent))
                            break
                if opened and depth == 0:
                    i = j + 1
                    break
                j += 1
            else:
                break
            continue
        i += 1
    return out

PAD_COL = 80  # column at which `// c:NNNN` starts

def add_tag(line: str, lineno: int) -> str:
    """Append `// c:NNNN` to line if it doesn't already have a c: tag,
    isn't blank, and isn't a pure comment line."""
    stripped = line.strip()
    if not stripped: return line
    if stripped.startswith(("//", "/*", "*", "*/")): return line
    if "// c:" in line or "//c:" in line: return line
    # avoid doc-comment lines
    if stripped.startswith(("///", "//!")): return line
    # pad with spaces up to PAD_COL
    base = line.rstrip()
    if len(base) < PAD_COL:
        base = base + " " * (PAD_COL - len(base))
    else:
        base = base + "  "
    return f"{base}// c:{lineno}"

def best_match_lineno(rs_line: str, body: list[tuple[int,str]], default: int) -> int:
    rs_t = toks(rs_line)
    if not rs_t:
        return default
    best, best_score = default, 0
    for ln, text in body:
        ct = toks(text)
        if not ct: continue
        score = len(rs_t & ct)
        if score > best_score:
            best_score, best = score, ln
    return best

def annotate_file(rs_path: Path, c_path: Path) -> tuple[int, int, int]:
    """Returns (fn_count_annotated, line_tags_added, fns_skipped_no_match)."""
    text = rs_path.read_text()
    c_idx = index_c(c_path)
    fns = find_rs_fns(text)
    if not fns:
        return (0, 0, 0)
    lines = text.splitlines()

    fn_count = 0
    line_tags = 0
    fn_skip = 0

    # Process in reverse so insertions don't shift earlier indices.
    for sig_line, body_open, body_close, name, indent in reversed(fns):
        c_fn = c_idx.get(name)
        if not c_fn:
            fn_skip += 1
            continue
        c_start = c_fn["start"]
        c_body = c_fn["body"]

        # Tag every line strictly inside body (between body_open+1 and body_close-1).
        for k in range(body_open + 1, body_close):
            lineno = best_match_lineno(lines[k], c_body, c_start)
            new = add_tag(lines[k], lineno)
            if new != lines[k]:
                lines[k] = new
                line_tags += 1

        # Insert/ensure doc-comment header above sig line.
        # Look upward past existing /// doc lines and #[attrs] to find proper insert point.
        ins = sig_line
        while ins > 0:
            prev = lines[ins - 1].lstrip()
            if prev.startswith("///") or prev.startswith("//!") or prev.startswith("#["):
                ins -= 1
                continue
            break
        # Check if any of the existing doc lines already mention "Port of <name>"
        already = False
        for k in range(ins, sig_line):
            if f"Port of {name}" in lines[k] or f"Port of `{name}`" in lines[k]:
                already = True; break
            if f"// c:{c_start}" in lines[k] or f"c:{c_start}" in lines[k]:
                already = True; break
        if not already:
            cfile_rel = c_path.relative_to(ROOT).as_posix().replace("src/zsh/Src/", "Src/")
            doc = f"{indent}/// Port of `{name}()` from `{cfile_rel}:{c_start}`."
            lines.insert(ins, doc)
            # All later indices shift +1 — but we're iterating in reverse,
            # so earlier (higher-up) fns haven't been touched yet. The
            # only items already processed are below this fn; their
            # absolute line numbers are no longer relevant, so we just
            # adjust the running cursor for our own bookkeeping if any.
        fn_count += 1

    rs_path.write_text("\n".join(lines) + ("\n" if text.endswith("\n") else ""))
    return (fn_count, line_tags, fn_skip)

def main() -> int:
    only = set(sys.argv[1:])
    files = sorted(p for p in RS_DIR.glob("*.rs") if p.name != "mod.rs")
    if only:
        files = [p for p in files if p.stem in only]
    total_fns = total_tags = total_skip = 0
    for rs in files:
        c = C_DIR / f"{rs.stem}.c"
        a, b, s = annotate_file(rs, c)
        total_fns += a; total_tags += b; total_skip += s
        print(f"{rs.name:<24}  fns:{a:4}  tags:{b:6}  unmatched:{s:4}", file=sys.stderr)
    print(f"\nTOTAL  fns:{total_fns}  tags:{total_tags}  unmatched:{total_skip}", file=sys.stderr)
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
