#!/usr/bin/env python3
"""For every fn in src/ported/modules/*.rs, try to find a matching C fn in
the corresponding Src/Modules/<base>.c.

  - If found:  prepend  /// Port of `<cname>()` from `Src/Modules/<base>.c:NNNN`.
  - If NOT found: prepend
        /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
        /// of any function in `Src/Modules/<base>.c`.

Idempotent: skips fns that already carry a `Port of` doc OR a
`WARNING: THIS IS ADHOC` doc.

Matching strategy (first hit wins):
  1. Existing `/// Port of NAME` doc-comment immediately above the fn
     — trust it, look up NAME in the C index.
  2. Existing `// C: name(` tag in the doc block.
  3. Identical rust-fn-name in the C index.
  4. Fuzzy candidates: builtin_foo→bin_foo, bin_zfoo→bin_foo,
     zfoo→foo, foo_handler→foo, set_foo→bin_setattr-style.
  5. Token-overlap of the fn body (rust body vs each C fn body)
     above a min threshold.
"""
from __future__ import annotations
import os, re, sys
from pathlib import Path

ROOT  = Path(__file__).resolve().parent.parent
RS_DIR = ROOT / "src" / "ported" / "modules"
C_DIR  = ROOT / "src" / "zsh" / "Src" / "Modules"

# ── token filter ─────────────────────────────────────────────────────────────
COMMON = {
    "if","for","while","switch","return","else","do","sizeof","static",
    "extern","struct","union","enum","typedef","const","volatile","inline",
    "register","auto","goto","break","continue","case","default","NULL",
    "void","int","char","long","short","unsigned","signed","float","double",
    "size_t","ssize_t","FILE","TRUE","FALSE",
    "let","mut","fn","loop","match","in","as","ref","pub","self","Self",
    "Some","None","Ok","Err","true","false","String","str","i32","u32",
    "i64","u64","usize","isize","Vec","HashMap","Option","Result","Box",
    "to_string","unwrap","clone","push","len","is_empty","into","from",
    "args","arg","name","ret","val","s","p","i","j","k","n","m","x","y",
}
TOK = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
def toks(line: str) -> set[str]:
    return {t for t in TOK.findall(line) if t not in COMMON and len(t) > 2}

# ── C indexer ────────────────────────────────────────────────────────────────
RE_C_FN = re.compile(r"^([A-Za-z_][A-Za-z0-9_]*)\s*\(")

def index_c(c_path: Path) -> dict[str, dict]:
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
            if m and m.group(1) not in COMMON:
                blk_open = -1
                for j in range(i, min(i+6, n)):
                    if "{" in lines[j] and ";" not in lines[j].split("{",1)[0]:
                        blk_open = j; break
                if blk_open < 0:
                    i += 1; continue
                depth = 0; end = blk_open
                for j in range(blk_open, n):
                    for ch in lines[j]:
                        if ch == "{": depth += 1
                        elif ch == "}":
                            depth -= 1
                            if depth == 0:
                                end = j; break
                    if depth == 0 and j >= blk_open: break
                name = m.group(1)
                body_text = "\n".join(lines[blk_open+1:end])
                body_toks = toks(body_text)
                if name not in out:
                    out[name] = {"start": i+1, "end": end+1, "body_toks": body_toks}
                i = end + 1
                continue
        i += 1
    return out

# ── Rust fn scanner ──────────────────────────────────────────────────────────
RE_RS_FN_SIG = re.compile(r"^(\s*)(?:pub\S*\s+|async\s+|unsafe\s+|extern\s+\"[^\"]*\"\s+)*fn\s+([A-Za-z_][A-Za-z0-9_]*)\b")
RE_PORT_DOC  = re.compile(r"Port(?:s|ed|ing)?\s+of\s+`?([A-Za-z_][A-Za-z0-9_]*)`?", re.IGNORECASE)
RE_C_TAG     = re.compile(r"//\s*[Cc]:\s*([A-Za-z_][A-Za-z0-9_]*)\s*\(")
RE_WARN      = re.compile(r"WARNING:\s*THIS IS ADHOC IMPLEMENTATION", re.IGNORECASE)

def find_rs_fns(text: str):
    """Yield (sig_line, body_open_line, body_close_line, name, indent)."""
    lines = text.splitlines()
    i = 0
    while i < len(lines):
        m = RE_RS_FN_SIG.match(lines[i])
        if m:
            indent, name = m.group(1), m.group(2)
            # Skip fn DECLARATIONS (no body) — line ends with `;` and has no `{`
            # before any `;`. Probe up to 4 lines ahead for an opening brace.
            probe_text = " ".join(lines[i:i+5])
            semi = probe_text.find(";")
            brace = probe_text.find("{")
            if brace < 0 or (semi >= 0 and semi < brace):
                i += 1
                continue
            j = i; depth = 0; opened = False; sig_end = i
            while j < len(lines):
                line = lines[j]
                for ch in line:
                    if ch == "{":
                        if not opened: opened = True; sig_end = j
                        depth += 1
                    elif ch == "}":
                        depth -= 1
                        if opened and depth == 0:
                            yield (i, sig_end, j, name, indent)
                            i = j + 1
                            break
                if opened and depth == 0: break
                j += 1
            else:
                return
            continue
        i += 1

def candidate_c_names(rs_name: str) -> list[str]:
    out = [rs_name]
    if rs_name.startswith("builtin_"):
        rest = rs_name[len("builtin_"):]
        out += [f"bin_{rest}", rest]
    m = re.match(r"^(bin|builtin)_z([a-z][a-zA-Z0-9_]*)$", rs_name)
    if m:
        out.append(f"bin_{m.group(2)}")
    if rs_name.startswith("z") and len(rs_name) > 1 and rs_name[1].islower():
        out.append(rs_name[1:])
    if rs_name.startswith("get_"):
        out.append(f"get{rs_name[4:]}")
    if rs_name.startswith("set_"):
        out.append(f"set{rs_name[4:]}")
    if rs_name.startswith("do_"):
        out.append(rs_name[3:])
    # de-dup, preserve order
    seen, result = set(), []
    for c in out:
        if c not in seen:
            seen.add(c); result.append(c)
    return result

def lookup_existing_doc(lines: list[str], sig_line: int) -> tuple[str | None, bool]:
    """Walk upward from sig_line over /// /* #[ … and return (port-name, has_warn)."""
    name, has_warn = None, False
    k = sig_line - 1
    while k >= 0:
        ls = lines[k].lstrip()
        if not (ls.startswith("///") or ls.startswith("//!") or ls.startswith("//")
                or ls.startswith("#[") or ls.startswith("/*") or ls.startswith("*")
                or ls.startswith("*/")):
            break
        if RE_WARN.search(lines[k]):
            has_warn = True
        m = RE_PORT_DOC.search(lines[k])
        if m and not name:
            name = m.group(1)
        m2 = RE_C_TAG.search(lines[k])
        if m2 and not name:
            name = m2.group(1)
        k -= 1
    return name, has_warn

def best_body_match(rs_body_toks: set[str], c_idx: dict[str, dict]) -> tuple[str | None, int]:
    if not rs_body_toks:
        return None, 0
    best, score = None, 0
    for name, info in c_idx.items():
        ct = info["body_toks"]
        if not ct: continue
        sc = len(rs_body_toks & ct)
        if sc > score:
            score, best = sc, name
    return best, score

# ── Main ─────────────────────────────────────────────────────────────────────
def annotate(rs_path: Path, c_path: Path, c_idx: dict[str, dict]) -> dict:
    text = rs_path.read_text()
    lines = text.splitlines()
    c_basename = c_path.relative_to(ROOT).as_posix().replace("src/zsh/Src/", "Src/")

    matched = adhoc = skipped = 0
    insertions: list[tuple[int, list[str]]] = []  # (insert-before-line, lines)

    for sig_line, body_open, body_close, rs_name, indent in find_rs_fns(text):
        existing_name, has_warn = lookup_existing_doc(lines, sig_line)
        if has_warn or existing_name:
            skipped += 1
            continue

        chosen: tuple[str, int] | None = None
        # 1) Direct + fuzzy name candidates
        for cand in candidate_c_names(rs_name):
            if cand in c_idx:
                chosen = (cand, c_idx[cand]["start"])
                break
        # 2) Body-overlap fallback (only when C file is small enough to be cheap)
        if chosen is None and len(c_idx) <= 80:
            rs_body = "\n".join(lines[body_open+1:body_close])
            rs_body_toks = toks(rs_body)
            best, score = best_body_match(rs_body_toks, c_idx)
            if best and score >= 4:
                chosen = (best, c_idx[best]["start"])

        if chosen:
            cname, lineno = chosen
            doc = [
                f"{indent}/// Port of `{cname}()` from `{c_basename}:{lineno}`.",
            ]
            matched += 1
        else:
            doc = [
                f"{indent}/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT",
                f"{indent}/// of any function in `{c_basename}`.",
            ]
            adhoc += 1

        # Insert doc above any existing #[attrs] / doc-comment block.
        ins = sig_line
        while ins > 0:
            ls = lines[ins-1].lstrip()
            if ls.startswith("///") or ls.startswith("//!") or ls.startswith("#["):
                ins -= 1
            else:
                break
        insertions.append((ins, doc))

    # Apply insertions bottom-up so indices stay valid.
    for ins, doc in sorted(insertions, key=lambda x: -x[0]):
        for j, d in enumerate(doc):
            lines.insert(ins + j, d)

    rs_path.write_text("\n".join(lines) + ("\n" if text.endswith("\n") else ""))
    return {"matched": matched, "adhoc": adhoc, "skipped": skipped}

def main() -> int:
    only = set(sys.argv[1:])
    files = sorted(p for p in RS_DIR.glob("*.rs") if p.name != "mod.rs")
    if only:
        files = [p for p in files if p.stem in only]
    totals = {"matched":0,"adhoc":0,"skipped":0}
    for rs in files:
        c = C_DIR / f"{rs.stem}.c"
        c_idx = index_c(c)
        r = annotate(rs, c, c_idx)
        for k,v in r.items(): totals[k] += v
        print(f"{rs.name:<24}  matched:{r['matched']:4}  adhoc:{r['adhoc']:4}  "
              f"already-doc:{r['skipped']:4}  c-fns:{len(c_idx):4}", file=sys.stderr)
    print(f"\nTOTAL  matched:{totals['matched']}  adhoc:{totals['adhoc']}  "
          f"already-doc:{totals['skipped']}", file=sys.stderr)
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
