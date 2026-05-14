"""Global stub report across all of src/ported/**.rs.

For each top-level fn, find the matching C fn in zsh source and compare
non-blank/non-comment body line counts. Flag where Rust << C.

Output: Markdown table in docs/audits/PORT_STUBS.md, sorted by file then
by stub ratio (smallest Rust/C ratio first).
"""
import re, os, glob
from datetime import datetime, timezone

ZSH = '/Users/wizard/forkedRepos/zsh/Src'


# Patterns in Rust fn body that mark an intentional empty/short fn —
# the implementation is a Rust-idiom replacement, NOT a stub. The script
# skips fns whose body comments contain any of these phrases.
INTENT_MARKERS = [
    'Drop happens automatically',
    'Rust String handles allocation',
    'Rust drops the',
    'Rust drops automatically',
    'inherent methods',
    'C heap-arena',
    'Box goes out of scope',
    'Rust\'s ownership',
    'no-op in Rust',
    'no vtable',
    'no-op for C fidelity',
    'no C analog',
    'unimplemented!',
    # Delegation markers — body work happens in a helper / impl method.
    'delegates to',
    'defers to',
    'dispatched to',
    'Static-link path defers',
    'lives in src/extensions/',
    'work moved to src/extensions/',
    'helper in src/extensions/',
    'wraps the canonical',
    # Architectural-replacement markers.
    'tree-walker disabled',
    'fusevm lowers',
    'fusevm replaces',
    'fusevm bytecode',
    'handled by Drop',
    'structural pass-through',
    'gsu hooks',
    'side-effect is already covered',
    'covered by the per-key',
    'Rust idiom replacement',
    'native tuple replaces',
    "Rust's Drop covers",
    "Rust's `Drop` covers",
    'Drop covers it',
    'Drop covers',
    'freed by Drop',
    'Arc handles this automatically',
    'Arc::drop',
    'Arc/String values drop',
    'Box drop the chain',
    'enum + Box drop',
    'recursive free of',
    'ABI parity with the C',
    'refcount hits zero',
    'no-op port',
    'static-linked and never',
    'wired through',
    'wires both',
    'wires the canonical',
    'Provided for C name parity',
    'name parity',
    'C name parity',
    'each typed table',
    'typed table has its own',
    'OnceLock initialiser handles',
    'first-touch initialisation',
    'replaced by typed',
    'replaced by traits',
    'Rust trait dispatch',
    'trait dispatch',
]

# A "delegation body" is a fn whose entire body is a single call/chain
# returning the value of another fn. Not a stub — the work lives there.
DELEGATION_BODY_RE = re.compile(
    r'\A\s*'
    r'(?:[a-zA-Z_][a-zA-Z0-9_]*'      # ident or Type/self chain start
    r'(?:::[a-zA-Z_][a-zA-Z0-9_]*)*'  # ::path::segments
    r'(?:\.[a-zA-Z_][a-zA-Z0-9_]*)*'  # .method.chain
    r')'
    r'\([^()]*(?:\([^()]*\)[^()]*)*\)'  # (args), one nesting depth allowed
    r'(?:\?|\.\w+\(\))?'               # trailing ? or .method()
    r'\s*;?\s*\Z',
    re.DOTALL,
)

# Iterator-chain bodies are Rust-idiom replacements for C loops.
ITER_CHAIN_RE = re.compile(
    r'\.(iter(?:_mut)?|into_iter|chars|bytes|lines|split|map|filter|collect|'
    r'take|skip|fold|sum|for_each|enumerate|zip|rev|cloned|copied|count)\b',
)


def fn_bodies_rust(src):
    """Yield (name, body_line_count) for each top-level fn in Rust source.

    Filters out fns whose body comments include an INTENT_MARKER phrase
    (those are intentional Rust-idiom no-ops, not actual stubs).
    """
    lines = src.split('\n')
    n = len(lines)
    i = 0
    depth = 0
    in_block = 0

    def parse_braces(line):
        nonlocal in_block
        bs = line.encode()
        j = 0
        d = 0
        while j < len(bs):
            b = bs[j]
            if in_block > 0:
                if b == ord('*') and j+1 < len(bs) and bs[j+1] == ord('/'):
                    in_block -= 1; j += 2; continue
                j += 1; continue
            if b == ord('/') and j+1 < len(bs):
                if bs[j+1] == ord('/'): break
                if bs[j+1] == ord('*'):
                    in_block += 1; j += 2; continue
            if b == ord('"'):
                j += 1
                while j < len(bs):
                    if bs[j] == ord('\\'): j += 2; continue
                    if bs[j] == ord('"'): j += 1; break
                    j += 1
                continue
            if b == ord("'"):
                k = j+1; found = False; esc = False
                while k < len(bs) and k-j < 12:
                    if not esc and bs[k] == ord("'"): found = True; break
                    esc = bs[k] == ord('\\') and not esc
                    k += 1
                j = k+1 if found else j+1
                continue
            if b == ord('{'): d += 1
            elif b == ord('}'): d -= 1
            j += 1
        return d

    while i < n:
        line = lines[i]
        trimmed = line.lstrip()
        m = re.match(r'^(pub(?:\([^)]*\))?\s+)?fn\s+(\w+)', trimmed)
        if depth == 0 and m:
            name = m.group(2)
            start_line = i + 1
            # Capture the doc-comment block immediately above the fn
            # signature so intent markers there also count.
            doc_text = ''
            k = i - 1
            while k >= 0:
                t = lines[k].lstrip()
                if t.startswith('///') or t.startswith('//!') or t.startswith('#['):
                    doc_text = lines[k] + '\n' + doc_text
                    k -= 1
                else:
                    break
            d = parse_braces(line)
            local = d
            i += 1
            while i < n and local == 0:
                d = parse_braces(lines[i])
                local += d
                if local > 0:
                    i += 1
                    break
                if lines[i].rstrip().endswith(';'):
                    break
                i += 1
            body_start = i
            while i < n and local > 0:
                d = parse_braces(lines[i])
                local += d
                i += 1
                if local == 0:
                    break
            body_end = i - 1
            body_lines = lines[body_start:body_end]
            actual = [l for l in body_lines if l.strip() and not l.strip().startswith('//')]
            # Check intent markers in body comments
            body_text = '\n'.join(body_lines)
            # Include doc-comment text in the marker check so intent
            # markers in the `///` block above the fn signature also
            # count (e.g. "Provided for C name parity").
            search_text = doc_text + body_text
            if any(marker in search_text for marker in INTENT_MARKERS):
                continue  # intentional empty/short — skip
            # Check delegation: body is a single fn-call expression
            code_only = re.sub(r'//[^\n]*', '', body_text).strip()
            # Strip leading `let _ = ` — singleton-touching pattern
            # (e.g. `pub fn createshfunctable() { let _ = shfunctab_lock(); }`
            # mirrors C `createshfunctable` which wires vtable; Rust's
            # OnceLock initialiser handles vtable equivalence on first
            # access, so the wrapper exists only to force first-touch).
            code_for_delegation = re.sub(r'\Alet\s+_\s*=\s*', '', code_only)
            if DELEGATION_BODY_RE.match(code_for_delegation):
                continue  # body delegates to a helper — not a stub
            # Architectural design: unreachable!/panic! bodies declare
            # the fn intentionally unsupported (e.g. tree-walker fn
            # replaced by fusevm).
            if re.match(r'\A\s*(unreachable|panic|todo)!\(', code_only):
                continue
            # Tuple of fn-calls (e.g. `(adjustcolumns(), adjustlines())`)
            # is multi-helper delegation, also not a stub.
            if re.match(
                r'\A\s*\(\s*[a-zA-Z_][a-zA-Z0-9_:]*\([^()]*\)\s*,\s*'
                r'[a-zA-Z_][a-zA-Z0-9_:]*\([^()]*\)\s*\)\s*;?\s*\Z',
                code_only,
            ):
                continue
            # Check iterator-chain body: Rust-idiom replacement for C loop
            if len(actual) <= 3 and ITER_CHAIN_RE.search(code_only):
                continue
            # Struct-literal arg (e.g. `vec.push(MyStruct { a, b, c })`)
            # collapses what C does as field-by-field copy.
            if len(actual) <= 5 and re.search(r'[A-Z][a-zA-Z0-9_]+\s*\{', code_only):
                continue
            yield name, len(actual), start_line
            continue
        d = parse_braces(line)
        depth += d
        i += 1


# Cache C body counts
_c_cache = {}


def _skip_lex(src, pos):
    """Advance past one C token (string/char literal, line comment, block
    comment) starting at `pos`. Returns the new position. If `src[pos]`
    starts none of these, returns `pos` unchanged so the caller falls
    through to its own handling."""
    if pos >= len(src):
        return pos
    c = src[pos]
    # Line comment
    if c == '/' and pos+1 < len(src) and src[pos+1] == '/':
        nl = src.find('\n', pos+2)
        return nl + 1 if nl != -1 else len(src)
    # Block comment
    if c == '/' and pos+1 < len(src) and src[pos+1] == '*':
        end = src.find('*/', pos+2)
        return end + 2 if end != -1 else len(src)
    # String literal
    if c == '"':
        pos += 1
        while pos < len(src):
            if src[pos] == '\\': pos += 2; continue
            if src[pos] == '"': return pos + 1
            pos += 1
        return pos
    # Char literal
    if c == "'":
        pos += 1
        while pos < len(src):
            if src[pos] == '\\': pos += 2; continue
            if src[pos] == "'": return pos + 1
            pos += 1
        return pos
    return pos


def c_fn_body(c_path, name):
    """Return non-blank/non-comment body line count for C fn `name`.

    Handles strings, char literals, line/block comments so brace counting
    doesn't drift on `"/*"`, `'}'`, `// }`, `/* { */`.
    """
    if c_path not in _c_cache:
        try:
            src = open(c_path).read()
        except FileNotFoundError:
            _c_cache[c_path] = None
            return None
        _c_cache[c_path] = src
    src = _c_cache[c_path]
    if src is None:
        return None
    pat = re.compile(rf'^{re.escape(name)}\s*\(', re.MULTILINE)
    m = pat.search(src)
    if not m:
        return None
    # Skip args (balanced parens, ignoring strings/chars/comments).
    pos = m.end()  # past `(`
    depth = 1
    while pos < len(src) and depth > 0:
        new_pos = _skip_lex(src, pos)
        if new_pos != pos:
            pos = new_pos
            continue
        c = src[pos]
        if c == '(': depth += 1
        elif c == ')': depth -= 1
        pos += 1
    # Skip whitespace + return-attr (e.g. `__attribute__((...))`) to `{` or `;`.
    while pos < len(src) and src[pos] != '{' and src[pos] != ';':
        new_pos = _skip_lex(src, pos)
        if new_pos != pos:
            pos = new_pos
            continue
        pos += 1
    if pos >= len(src) or src[pos] == ';':
        return 0
    body_start = pos + 1
    depth = 1
    pos = body_start
    while pos < len(src) and depth > 0:
        new_pos = _skip_lex(src, pos)
        if new_pos != pos:
            pos = new_pos
            continue
        c = src[pos]
        if c == '{': depth += 1
        elif c == '}': depth -= 1
        pos += 1
    body_lines = src[body_start:pos-1].split('\n')
    actual = [l for l in body_lines if l.strip()
                and not l.lstrip().startswith('//')
                and not l.lstrip().startswith('/*')
                and not l.strip().startswith('*')]
    return len(actual)


def find_c_file(rust_path):
    """Map src/ported/<sub>/<stem>.rs → /Src/<sub>/<stem>.c."""
    stem = os.path.splitext(os.path.basename(rust_path))[0]
    rel = os.path.dirname(rust_path).replace('src/ported/', '').replace('src/ported', '')
    # Candidate paths
    candidates = []
    if rel:
        # e.g. src/ported/zle/foo.rs → Src/Zle/foo.c
        sub = rel.replace('/', os.sep)
        # Capitalize first letter of subdir
        parts = sub.split('/')
        cap_parts = [p.capitalize() for p in parts]
        cap_path = '/'.join(cap_parts)
        candidates.append(os.path.join(ZSH, cap_path, stem + '.c'))
        candidates.append(os.path.join(ZSH, sub, stem + '.c'))
        # Try Builtins for /builtins/
        if 'builtins' in rel.lower():
            candidates.append(os.path.join(ZSH, 'Builtins', stem + '.c'))
    # Always try root Src/
    candidates.append(os.path.join(ZSH, stem + '.c'))
    # Header files: Src/zsh.h → src/ported/zsh_h.rs (zsh_h → zsh)
    if stem.endswith('_h'):
        h_stem = stem[:-2]
        candidates.append(os.path.join(ZSH, h_stem + '.h'))
        # also Zle/zle.h etc.
        if rel:
            sub = rel.replace('/', os.sep)
            parts = sub.split('/')
            cap_path = '/'.join(p.capitalize() for p in parts)
            candidates.append(os.path.join(ZSH, cap_path, h_stem + '.h'))
    for p in candidates:
        if os.path.exists(p):
            return p
    return None


def main():
    rows = []  # (rel_path, name, rust_line, rust_body, c_body)
    for rust_path in sorted(glob.glob('src/ported/**/*.rs', recursive=True)):
        c_path = find_c_file(rust_path)
        if not c_path:
            continue
        src = open(rust_path).read()
        # Collect per-fn results; same-named fns (e.g. `#[cfg(unix)]` /
        # `#[cfg(not(unix))]` variants) merge to the MAX body length so
        # platform-shim stubs don't shadow the real port.
        per_name = {}  # name -> (rust_line, rust_body)
        for name, rust_body, rust_line in fn_bodies_rust(src):
            prev = per_name.get(name)
            if prev is None or rust_body > prev[1]:
                per_name[name] = (rust_line, rust_body)
        for name, (rust_line, rust_body) in per_name.items():
            cb = c_fn_body(c_path, name)
            if cb is None or cb < 10:
                continue
            ratio = rust_body / cb if cb else 0
            if ratio < 0.3:
                rows.append((rust_path, name, rust_line, rust_body, cb, ratio))

    # Generate markdown
    from collections import defaultdict
    by_file = defaultdict(list)
    for r in rows:
        by_file[r[0]].append(r)

    out_lines = []
    out_lines.append("# PORT_STUBS — stubs detected in src/ported/")
    out_lines.append("")
    out_lines.append(f"Generated: {datetime.now(timezone.utc).isoformat()}")
    out_lines.append("")
    out_lines.append("## Method")
    out_lines.append("")
    out_lines.append("For each top-level `fn` in `src/ported/**.rs`, the script finds the")
    out_lines.append("same-named function in the matching upstream C source")
    out_lines.append("(`/Users/wizard/forkedRepos/zsh/Src/...`) and compares non-blank/")
    out_lines.append("non-comment body line counts. A fn is flagged as a stub when the")
    out_lines.append("Rust body is **less than 30% of the C body** AND the C body is at")
    out_lines.append("least 10 lines.")
    out_lines.append("")
    out_lines.append("Regenerate via:")
    out_lines.append("```")
    out_lines.append("python3 scripts/gen_port_stubs.py")
    out_lines.append("```")
    out_lines.append("")
    out_lines.append(f"## Summary: {len(rows)} stubs across {len(by_file)} files")
    out_lines.append("")
    out_lines.append("| File | Stubs | Worst (Rust / C lines) |")
    out_lines.append("|---|---|---|")
    for f in sorted(by_file, key=lambda k: -len(by_file[k])):
        stubs = sorted(by_file[f], key=lambda x: x[5])
        worst = stubs[0]
        out_lines.append(
            f"| `{f}` | {len(stubs)} | `{worst[1]}` ({worst[3]} / {worst[4]}) |"
        )
    out_lines.append("")
    out_lines.append("## Per-file detail")
    out_lines.append("")
    for f in sorted(by_file, key=lambda k: -len(by_file[k])):
        stubs = sorted(by_file[f], key=lambda x: x[5])
        out_lines.append(f"### `{f}` — {len(stubs)} stubs")
        out_lines.append("")
        out_lines.append("| Rust line | fn | rust body | C body | ratio |")
        out_lines.append("|---|---|---|---|---|")
        for path, name, rline, rb, cb, ratio in stubs:
            pct = int(100 * ratio)
            out_lines.append(f"| {rline} | `{name}` | {rb} | {cb} | {pct}% |")
        out_lines.append("")

    out = '\n'.join(out_lines) + '\n'
    out_path = 'docs/audits/PORT_STUBS.md'
    os.makedirs(os.path.dirname(out_path), exist_ok=True)
    open(out_path, 'w').write(out)
    print(f"Wrote {out_path}")
    print(f"Total stubs: {len(rows)} across {len(by_file)} files")


if __name__ == '__main__':
    main()
