# Call-Site Coverage Audit — Why The Report Flags 393 Under-Wired Fns

`docs/port_report.html` now compares C call-site count vs Rust
call-site count for every ported fn (`gen_port_report.py` added
`c_calls` / `rust_calls` / `call_pct` columns per-symbol). When
Rust calls a C-canonical fn at <30% of the C sites, the row turns
red and the row counts toward the "Under-wired" stat card.

The metric exists because of `doshfunc`: C had 18 external call
sites, Rust had 1 (all other dispatch went through a single
`dispatch_function_call` indirection that wrapped doshfunc once).
That was real fakery — the canonical scope-management entry was
defined in Rust but bypassed everywhere except one call site.

After the audit landed, ~395 fns flagged. Triage falls into three
buckets:

## 1. Real fakery — wire it up.

Pattern: Rust port has the fn defined and the C call sites have
direct Rust-port counterparts that could be using the same fn.
The work is the same; only the call-site syntax differs.

| Fixed | C calls | Rust before | Rust after | Notes |
|---|---|---|---|---|
| `doshfunc` | 18 | 1 (5%) | 18 (100%) | Wired all 22 sites per `docs/PORT.md` audit (commits `b83cb32`/`a77af29`) |
| `bindkey` | 69 | 3 (4%) | 71 (103%) | Rust factored into 3 methods; restored canonical fn (commit `591929e`) |
| `skipparens` | 28 | 1 (4%) | 2 (7%) | Wired one glob.rs inlined paren-walk; remaining sites are in unported substrate |

## 2. Architectural divergence — leave it.

Pattern: Rust replaces the C pattern with a different idiom that
does the same work but doesn't go through the named fn. Wiring
these would be metric-gaming, not real porting work.

| C fn | C calls | Rust replacement | Why |
|---|---|---|---|
| `ztrdup` | 579 | `.clone()` / `String::from` | Rust owns memory; no manual dup |
| `zsfree`, `zalloc`, `zhalloc`, `zshcalloc`, `hcalloc`, `zfree`, `freearray` | 100-280 | `Drop` / `Vec` / `String` | Rust ownership eliminates explicit lifecycle calls |
| `newlinklist`, `addlinknode`, `firstnode`, `nextnode`, etc. | 50-100 | `Vec<T>` + iter methods | LinkList replaced by Vec |
| `refthingy` / `unrefthingy` | 94 | `Arc<Thingy>::clone` / `Drop` | Refcounting via Arc |
| `metafy` | 101 | mixed (28 in modules using foreign byte buffers; rest are no-ops because Rust strings are UTF-8) | Storage divergence — Rust doesn't need meta-encoding for internal strings |
| `tokenize` | 54 | `patcompile` does internal tokenization | Rust pattern compile internalizes the step |
| `arrlen` | 96 | `.len()` on `&[T]` | One-line wrapper; replacing every `.len()` with `arrlen(.)` is pure metric-gaming |
| `dupstring`, `dupstrpfx` | 466 | `.to_string()` / `.into()` | Same memory-mgmt story |
| `strcmp`, `strncmp`, `strlen`, `memcpy`, `memset` | many | `==` / `.len()` / `.copy_from_slice` | C stdlib equivalents replaced by Rust idioms |
| `scanhashtable`, `getnode`, `addnode`, `freenode`, `removenode`, `newhashtable` | 50-200 | `HashMap` / `BTreeMap` iter + ops | Hashtable plumbing replaced by std collections |

## 3. Substrate gap — port the surrounding fn first.

Pattern: C calls the fn from a code path Rust hasn't ported yet.
Wiring it requires porting the containing function first.
Example: `skipparens` has 26 remaining C call sites in
`zle_tricky.c`/`compcore.c` paths that the Rust port hasn't
reached yet (lex/parse substrate).

## Reading the report

`docs/port_report.html` per-symbol columns:

- **C calls** — total `name(` occurrences across `src/zsh/Src/`
  (excludes the def line itself + comment lines).
- **Rust calls** — same across `src/ported/` + `parse/src/`.
- **call %** — Rust / C. <30% red, 30-79% yellow, ≥80% green,
  gray for `unported` rows where Rust=0 is the expected "not done
  yet" state, not the fakery signal.

The "Under-wired (call <30%)" stat card on the hero stats is the
top-level health metric. **It will never reach 0** because of the
arch-divergence bucket above — set a realistic floor based on the
arch-divergence count and treat any rise from there as a real
regression.

## How to fix a flagged fn

1. **Classify** — bucket 1 (real fakery), 2 (arch divergence), or
   3 (substrate gap). Most fns will land in bucket 2.
2. If bucket 1: write a single per-fn fix commit following the
   `doshfunc`/`bindkey` pattern. Rewrite the Rust port to take
   the C signature exactly; route every C-equivalent Rust call
   site through the canonical fn.
3. If bucket 2: add a comment line near the def explaining the
   architectural replacement (so future audits don't re-flag it).
   Optionally: extend `CALL_COUNT_SKIP_NAMES` in
   `scripts/gen_port_report.py` to suppress the row.
4. If bucket 3: don't touch the call sites. Port the containing
   fn first; the call coverage will fix itself as substrate lands.
