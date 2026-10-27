# Call-Site Coverage Audit — Why The Report Flags Under-Wired Fns

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

When the audit landed (2026-05-28) roughly 395 fns flagged; the
current count is the "Under-wired" stat card in
`docs/port_report.html`. Triage falls into three buckets:

## 1. Real fakery — wire it up.

Pattern: Rust port has the fn defined and the C call sites have
direct Rust-port counterparts that could be using the same fn.
The work is the same; only the call-site syntax differs.

"Rust before" / "Rust after" are the counts at the time of the fix
(2026-05-28). "Report 2026-10-18" is `docs/port_report.html` as
generated on 2026-10-18 (v0.13.55); regenerate the report for current
numbers.

| Fixed | C calls | Rust before | Rust after | Report 2026-10-18 | Notes |
|---|---|---|---|---|---|
| `doshfunc` | 18 | 1 (5%) | 18 (100%) | 18 (100%) | Wired the C call sites per `docs/PORT.md` audit (commit `2356e1f578`) |
| `bindkey` | 69 | 3 (4%) | 71 (103%) | 66 (96%) | Rust factored into 3 methods; restored canonical fn (commits `e850760e87` + `863b6796ba`) |
| `skipparens` | 28 | 1 (4%) | 16 (57%) | 23 (82%) | 5-commit substrate-port arc: signature fix + glob/cmphaswilds/parambeg/check_param/parse/lex/subst sites (commits `2d42e23add` → `ce17a8d7a4`) |
| `inststr` | — | 10 | 13 | 13 | `processcmd` faithful 3-call port (commit `358b03d8ca`) |
| `optlookup` | 10 | 59 (590%) | 14 (140%) | 24 (240%) | Over-wired direction — Rust used `isset(optlookup("name"))` as a string→optno bridge where C uses compile-time `OPT_NAME` integer constants. Mass-replaced 39 call sites across `builtin.rs`/`signals.rs`/`params.rs`/`options.rs`/`modules/*` with the existing `zsh_h::NAME` constants (commit `a4f9a87a5c`). |

### Over-wired direction (>100%)

When Rust calls a fn more often than C, the metric reads >100%.
Two cases:

- **Real over-wiring** (fixable): Rust ports use a runtime helper
  where C uses a compile-time idiom. `optlookup` was the
  canonical case — every `isset(optlookup("name"))` site is a
  string→optno bridge that could be `isset(NAME)` against the
  existing `zsh_h::NAME` constant. Fix by replacing the runtime
  lookups with the constants (commit `a4f9a87a5c`).
- **Generic name collision** (leave it): C `push`/`add`/`write_loop`
  share names with Rust idioms (`Vec::push`, `String::add`, etc.).
  The audit excludes `.name(` method calls but not bare-fn-call
  shapes that happen to match. These are metric noise, not
  fakery — leave them flagged.

### Macro-wrapper false positives (under-wired direction)

Some C fns have a `#define` wrapper that hides the real call.
The audit greps text and can't expand macros, so wrapper calls
count under the macro name (C=0, Rust=N), while the wrapped fn
appears under-wired.

| Macro | Expansion | Rust strategy |
|---|---|---|
| `inststr(X)` | `inststrlen((X),1,-1)` (compresult.c:39) | Rust defines `inststr(s) { inststrlen(s, true, -1) }` and calls `inststr(s)` everywhere C uses the macro. Faithful to C source. |

The audit can show the wrapped name as under-wired because it
can't see the macro expansion. (`inststrlen` itself is no longer
flagged: the 2026-10-18 report counts 41 C and 41 Rust calls.) Inlining
the macro at call sites to make the audit happy would make the
Rust port DIVERGE from C source syntax — wrong direction. Leave
flagged; document the wrapper here.

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
| `dyncat` | 103 | `format!` / `String::push_str` | Rust string concat is built into the language |
| `pattrylen`, `patadd`, `patmatch` (subset) | 25-30 | Rust port routes through `matchpat` wrapper that takes raw pattern + flags | Rust port abstracts pattern compile + try into single call; C is multi-step |
| `addbufspc`, `taddchr` | 30-36 | `Vec::push` / `Vec::reserve` | Vec grows automatically |
| `strpfx` | 49 | `str::starts_with` | Built-in |
| `nicezputs`, `applytextattributes` | 40+ | `fmt::Display` / ANSI escape composition | Rust formatting + termion-style escape composition |
| `freeeprog`, `countlinknodes`, `attachtty` | 20-25 | Various Rust idiom replacements | See per-fn doc comments |

## 3. Substrate gap — port the surrounding fn first.

Pattern: C calls the fn from a code path Rust hasn't ported yet.
Wiring it requires porting the containing function first.

**Worked example — `skipparens` 1 → 16 over 5 commits (2026-05-28):**

| Commit | Coverage | Where the work went |
|---|---|---|
| `2d42e23add` | 1 → 2 (7%) | C-signature rewrite of `skipparens` + replace 1 inline depth-walk in `glob.rs:parsecomplist` |
| `cf03face45` | 2 → 7 (25%) | Faithful re-port of `cmphaswilds` (was a stub) adds 4 internal skipparens calls |
| `0536d9ddb4` | 7 → 12 (43%) | Faithful re-port of `parambeg` (was a stub, +2) + `check_param` ternary expansion (+3) |
| `cb75576c76` | 12 → 15 (54%) | `parse.rs:par_simple_wordcode` (+2) + `lex.rs:is_valid_assignment_target` (+1) |
| `ce17a8d7a4` | 15 → 16 (57%) | `subst.rs:stringsubst` `$[...]` tokenized arm routed through canonical skipparens |

At the end of that arc the remaining 12 sites all sat in unported
deeper substrate:
`docomplete`/`get_comp_string` (zle_tricky.c, 8 sites), niche
`paramsubst`/`modify` (subst.c, 3 sites), `xpandbraces`
unbalanced-brace fallback (glob.c, 1 site — bucket 2 actually).
Later substrate ports closed most of them: the 2026-10-18 report
counts 23 of 28 C sites (82%).

**Recipe** (reusable for any substrate-gap fn):

1. Find call sites: `grep -n 'fnname(' /Users/wizard/forkedRepos/zsh/Src/...c`
2. Map to containing fn: `awk '/^[a-z_]+\(.*\)$/{fn=$0; line=NR} /fnname\(/{print line": "fn}'`
3. Read C body: `awk '/^fnname/{f=1} f && /^}/{f=0; exit} f' /Users/wizard/forkedRepos/zsh/Src/X.c`
4. Find Rust counterpart: `grep -n 'fn containing_fn' src/ported/...rs`
5. Replace inline depth-walks with `crate::ported::utils::skipparens(open, close, &mut cursor)`
6. Compute char-offset advances correctly when cursor moves (Rust slice operates in bytes, char-walks need translation)
7. Verify with `cargo test -p zshrs --lib <module>` — no new test failures

## Reading the report

`docs/port_report.html` per-symbol columns:

- **C calls** — total `name(` occurrences across `src/zsh/Src/`
  (excludes the def line itself + comment lines).
- **Rust calls** — same across `src/ported/` (the `parse/` crate was
  merged into `src/ported/` in 2026-05; `gen_port_report.py`'s
  `RS_DIRS` still lists the now-absent `parse/src`).
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
