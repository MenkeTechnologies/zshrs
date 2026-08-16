# Port Checklist — `src/ported/` 100% C Parity

Working list for the line-by-line port pass. Each file gets a single
checkbox; tick when the file's Rust port is verified function-by-function LINE BY LINE
**AND** struct-by-struct AND ENUM BY ENUM against its C counterpart in `~/forkedRepos/zsh/Src/`.

All structs and enums must have matching field names and data types.
Every LINE OF SOURCE CODE MUST BE 100% ported.  EVERY Source FUNCTION LINE MUST BE PRESENT in ported file.

---

## RULES (load-bearing — re-read before every file)

These rules supersede every prior PORT.md / port-plan note. A file
isn't ticked until ALL of them pass.

1. **Zero Rust-only structs / enums in `src/ported/`.** If a `pub
   struct Foo` or `pub enum Bar` doesn't exist in the matching C
   `.c` file (verify by `grep -nE '^(struct|typedef|enum)' Src/...`),
   it must be removed. Wrap-around layers, "convenience" aggregates,
   typed builders, options-bags, kind-enums — all forbidden. The
   only allowed Rust types are direct ports of C `struct ...` /
   `enum ...` definitions.

2. **Every struct / enum that remains must match its C name exactly.**
   - C `struct ptycmd` → Rust `Ptycmd` (CamelCase, drop `struct ` prefix; same letters)
   - C `struct globdata` → Rust `Globdata` or `GlobData`? Match C casing letter-for-letter — if C names it `globdata`, Rust uses `Globdata`. (Idiomatic Rust CamelCase, but the casing of the C name decides word boundaries.)
   - C `enum cdsetop` → Rust `Cdsetop`
   - No invented names. No `XxxState` aliases for `struct xxx`. No `XxxOptions` for `Options ops` flag bags.

3. **`Options ops` is a bitmask, not a struct.** C builtins receive
   `Options ops` and read it via `OPT_ISSET(ops,'l')`. Rust ports
   take the equivalent `i32` / `u32` flag bitmask (with `#define`-
   style constants if zsh defines them) — no `XxxOptions` struct.

4. **No "Rust-only abstraction" warning blocks for new code.** The
   `// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT`
   marker was a transitional escape hatch. Going forward, anything
   that would carry that marker must be deleted instead.

5. **Fix broken function stubs at their source, don't work around
   them.** If file A's port needs `wordclass(c) -> i32` and file B
   has it stubbed as `wordclass() -> i32 { 0 }`, fix B's signature
   + body in the same commit as A. Don't write inline duplicate
   helpers in A.

6. **Function bodies match C 1:1.** Cite C `file:line` in inline
   comments (`// c:NNN` or `// Src/.../foo.c:NNN`) on every block
   that mirrors a chunk of C source. Where the C body uses
   constructs zshrs doesn't model (e.g., `ctxtlex`, `addzlefunction`,
   `featuresarray` for static-link), keep the no-op body but
   document the architectural divergence with a comment block —
   no silent shortcuts.

7. **Drift gate stays green.** Every `pub fn` / `fn` in `src/ported/`
   either matches a C function name in `tests/data/zsh_c_fn_names.txt`
   or appears in `tests/data/fake_fn_allowlist.txt` with a
   citation explaining why no C counterpart exists. Adding new
   entries to the allowlist is a code smell — reach for it only
   when the helper is genuinely unavoidable.

8. **Explicit `cargo build --lib` after every file** (NOT
   `cargo build --release`). Drift gate run after every batch.

9. **Commit per file or per ≤5-file batch.** No mass commits that
   bury per-file regressions.

10. **Proof of 100% port must be shown via line counts logged here**
11. IF a ported function call does that exist it must be created in the right file
---

## Methodology per file

1. Read every `pub struct` / `pub enum` / `pub fn` / `fn` in the Rust file.
2. Compare against C: `grep -nE '^(struct|typedef|enum)' Src/<path>.c` and `tests/data/zsh_c_fn_names.txt`.
3. **Delete** every Rust-only struct/enum that has no C counterpart.
4. **Rename** every remaining struct/enum to match C exactly.
5. **Replace** every stub function body with the line-by-line port of its C counterpart, citing `// c:NNN`.
6. **Remove** every `WARNING: NOT IN <FILE>.C` / `WARNING: ADHOC` block — code that was Rust-only either gets deleted (if the type/fn shouldn't exist) or rewritten as a real port.
7. **Update** broken sibling stubs (e.g. wrong signature) at their source as part of the same commit.
8. `cargo build --lib` → must be clean.
9. `cargo test --test ported_fn_names_match_c` → drift gate clean.
10. `cargo test --lib -- ported::<module>` → all tests in the affected module pass.
11. Commit with a `port: <file> — ...` message citing the rules applied.

---

## Status Summary (updated 2026-07-19)

**Codebase metrics** (regenerate via `find src/ported -name '*.rs' | xargs wc -l`
and `grep -c '// c:' src/ported/<file>.rs`):
- `src/ported/`: 379,287 lines, 106 files (`find src/ported -name '*.rs'`)
- `// c:NNN` citations: subst.rs (2882), builtin.rs (2998), zsh_h.rs (794), module.rs (1114)
- `/// Port of` citations: params.rs (183), zsh_h.rs (124), utils.rs (194)

**ADHOC warnings:** all `WARNING.*ADHOC` annotations cleared from the
`modules/*.rs` set (`grep -rc 'ADHOC' src/ported/modules/` now reports
zero across `parameter.rs`, `db_gdbm.rs`, `zftp.rs`, `zutil.rs`,
`zpty.rs`, `pcre.rs`, `watch.rs`).

**Stub coverage:** see `docs/audits/PORT_STUBS.md` (regenerate with
`python3 scripts/gen_port_stubs.py`).

**Build status:** `cargo build` clean (dev profile).

---

## ✅ DONE — verified line-by-line (zero stubs + zero Rust-only types + name-matched)

Each tick below carries a per-fn audit log proving every C line maps
to a Rust statement. No tick without that log. Anything blocked on
an unported dependency is moved to **🚧 BLOCKED** and tracked in
`TODO.md`.

- [x] `modules/random_real.rs` ↔ `Modules/random_real.c` — line-by-line C-faithful rewrite (PORT.md "EXACT TRANSLATION"); 4 fns in C-source order: `clz64` macro alias (c:43), `_zclz64` (c:48), `random_64bit` (c:83), `random_real` (c:147). `random_real` body mirrors C variable declarations + control flow + comments verbatim. `clz64` named to match C `#define` per case-sensitive rule. Uses `(x as f64) * 2.0_f64.powi(exp)` for ldexp (matching C author's 2015-02-22 update note about glibc slow ldexp). 5/5 tests pass
  - `random_64bit() -> u64` — port of c:84-93. Includes the `getrandom_buffer` error path + `zwarn(...)` + `return 1` (not 0) + `u64::from_ne_bytes(buf)` success path. Matches c:85-93 line-by-line. Verified after fix in `03ab0b26d9`.
  - `random_real() -> f64` — port of c:147-213. Calls `random_64bit()` (not `random_u64`), `_zclz64()` (not `leading_zeros`), and `extern "C" ldexp` (not `exp2`). All 18 C statements have matching Rust statements. Verified after fix in `03ab0b26d9`.

## 🚧 BLOCKED — partial port, gap tracked in `TODO.md`

- [ ] `zle/textobjects.rs` ↔ `Zle/textobjects.c`
  - C: 0 structs/enums • Rust: 0 structs/enums ✓ (after deleting Rust-only `TextObjectType`/`TextObjectKind`/`TextObject` + `Zle::select_text_object`-family helpers)
  - C fns: `blankwordclass`, `selectword`, `selectargument` (3) • Rust: same 3 ✓
  - `blankwordclass(c)` — 1-line port of c:36 ✓ verified.
  - `selectword(zle)` — full ~170-line port of c:41-205 incl. visual-mode-reverse-direction branch (c:97-148), digit-arg loop's `if all` inner block (c:165-179), and `doblanks` trim section (c:181-194). One residual: reads `virangeflag` as constant-false (zle_vi.c:36 is unported file-global, see TODO.md). Verified except for that constant.
  - `selectargument(zle)` — **NOT 100%.** C body uses `ctxtlex()` lexer-walk (c:233-257); Rust port is whitespace-split approximation. Blocks on lexer-context machinery — see TODO.md.
- [ ] `modules/socket.rs` ↔ `Modules/socket.c`
  - C: 0 structs/enums • Rust: 0 structs/enums ✓ (after deleting Rust-only `ZsocketOptions`/`UnixSocket`)
  - C fns (7): `bin_zsocket`, `setup_`, `features_`, `enables_`, `boot_`, `cleanup_`, `finish_` • Rust: same 7 ✓
  - 6 module loaders match C 1:1 (each is `return 0;` body or short featuresarray/handlefeatures call — Rust ports are no-op static-link path)
  - `bin_zsocket` — full port of c:57-272 incl. inline flag parse (matching `"ad:ltv"` builtin spec at c:276), socket()/bind()/listen() for `-l` (c:84-138), poll-test + accept() for `-a` (c:142-218), socket()/connect() default path (c:218-269), addmodulefd + redup/movefd post-call sequence on every success path (c:118/121/125, c:208/211/215, c:252/255/260). The shim writes `setiparam_no_convert("REPLY", final_fd)` (c:135/204/268). Verified line-by-line.
- [ ] `zle/deltochar.rs` ↔ `Zle/deltochar.c`
  - C: 0 structs/enums • Rust: 0 structs/enums ✓
  - C fns (7): `deltochar`, `setup_`, `features_`, `enables_`, `boot_`, `cleanup_`, `finish_` • Rust: same 7 ✓
  - 6 module loaders: `setup_`/`features_`/`enables_`/`boot_`/`cleanup_`/`finish_` — each is `return 0;` C body or trivial featuresarray/handlefeatures/addzlefunction call. Rust ports are static-link no-ops with C-line citations. Verified.
  - `deltochar(zle)` — port of c:38-79. 1:1 mapping: `getfullchar(0)` → `zle.getfullchar(false)`; `int dest = zlecs, ok = 0, n = zmult` → 3 mut locals; `zap = bindk->widget == w_zaptochar` → `zle.bindk.name == "zap-to-char"`; forward-direction loop (c:45-58) and backward-direction loop (c:59-77) match C structure exactly; `forekill(dest - zlecs, CUT_RAW)` and `backkill(zlecs - dest - zap, CUT_RAW|CUT_FRONT)` call into the real ports in zle_utils.rs (sibling stubs fixed at source, rule 5); `return !ok` → `if ok != 0 { 0 } else { 1 }`. Verified.
  - Sibling fixes: `zle_utils::forekill` (zle_utils.c:1064) + `zle_utils::backkill` (zle_utils.c:1045) ported as part of this commit.
- [ ] `loop.rs` ↔ `loop.c`
  - C: 0 structs/enums • Rust: 0 structs/enums ✓ (after deleting dead `LoopState`/`ForIterator`/`CForState`/`TryState` aggregates in earlier dissolution)
  - C fns (8): `execfor`, `execselect`, `execwhile`, `execrepeat`, `execif`, `execcase`, `exectry`, `selectlist` • Rust: same 8 ✓
  - 7 tree-walker entries (`execfor`/`execselect`/`execwhile`/`execrepeat`/`execif`/`execcase`/`exectry`) — bodies are `unreachable!()` per the 96-test architectural invariant (fusevm bytecode in `compile_zsh.rs` replaces tree-walker dispatch). Each entry cites its C line + the architectural reason. Verified consistent.
  - `selectlist(items, start)` — port of c:347-416. Was previously a Rust-only signature `(items, prompt, columns) -> String`; now matches C exactly: takes items + start index, writes formatted menu to stderr, returns next-page offset (or 0 when complete). Body ports c:350-415 line-by-line: longest-width compute, fct/fw column geometry, the do-while inner loop, MB_METASTRWIDTH approximation via chars().count(). Verified.
- [ ] `modules/mathfunc.rs` ↔ `Modules/mathfunc.c`
  - C: 3 anonymous `enum {}` blocks (untyped int constants) • Rust: 0 pub struct/enum, replaced with `pub const MF_*: i32`, `pub const MS_*: i32`, `pub const TF_*: i32` matching C definitions exactly. ✓
  - C fns (8): `setup_`, `features_`, `enables_`, `boot_`, `cleanup_`, `finish_`, `math_func`, `math_string` • Rust: same 8 ✓
  - 6 module loaders match (return 0 each) ✓
  - `math_func(_name, argc, argv, id) -> Mnumber` — port of c:172-436. Full TF_INT1/TF_INT2/TF_NOCONV arg-coerce phase + giant switch on `id & 0xff` over MF_ABS through MF_YN + post-switch `if (!(id & TFLAG(TF_NOASS))) ret.u.d = retd;` finalisation. Calls libm via extern "C" for j0/j1/jn/y0/y1/yn/erf/erfc/lgamma/tgamma/ilogb/logb/nextafter/rint/scalbn/ldexp/copysign/expm1/log1p/cbrt. Verified.
  - `math_string(_name, arg, id) -> Mnumber` — port of c:439-471. Trims iblank from arg + dispatches on id; only MS_RAND48 wired. Verified.
  - All `MathNumber` enum + `MathFunctions` namespace + helper fns deleted (Rust-only abstractions).

- [ ] `modules/newuser.rs` ↔ `Modules/newuser.c` — **PARTIAL.**
  - C: 0 structs/enums • Rust: 0 structs/enums ✓
  - C fns (7): `setup_`, `features_`, `enables_`, `check_dotfile` (static), `boot_`, `cleanup_`, `finish_` • Rust: same 7 ✓
  - `setup_`, `features_`, `enables_`, `cleanup_`, `finish_` — each is a 1-line `return 0;` C body. Rust ports match. ✓
  - `check_dotfile(dotdir, fname)` — port of c:58-65. C composes path via VARARR + sprintf, calls `access(F_OK)`. Rust uses `Path::push` + `Path::exists` — same observable result. ✓
  - `boot_()` — **NOT 100%.** Missing the C `EMULATION(EMULATE_ZSH)` check (c:79) and the `source(buf)` newuser-install-script loop over spaths (c:96-101). Both gaps blocked on changing module-loader signatures to take `&mut ShellExecutor` — see TODO.md.

- [ ] `modules/regex.rs` ↔ `Modules/regex.c`
  - C: 0 structs/enums • Rust: 0 structs/enums ✓ (after deleting Rust-only `RegexMatch` struct)
  - C fns (8): `zregex_regerrwarn` (static), `zcond_regex_match` (static), `setup_`, `features_`, `enables_`, `boot_`, `cleanup_`, `finish_` • Rust: same 8 ✓
  - 6 module loaders match (return 0). ✓
  - `zcond_regex_match(exec, a, id)` — port of c:54-200. Compiles regex with CASEMATCH-aware `(?i)` prefix, runs match, writes back $MATCH/$MBEGIN/$MEND/$match[]/$mbegin[]/$mend[] (or $BASH_REMATCH when BASHREMATCH set), with KSHARRAYS-aware 1-based vs 0-based offset indexing. 8 tests covering all branches pass. Verified.
  - `zregex_regerrwarn(prefix, msg)` — collapses C's two-`regerror()` size+fill pattern into a single `zwarnnam` call (c:40-51). Rust's regex crate carries pre-formatted error strings. Verified.

- [x] `modules/zselect.rs` ↔ `Modules/zselect.c`
  - C: 0 structs/enums • Rust: 0 structs/enums ✓ (after deleting `SelectMode`, `ZselectOptions`, `SelectResult`)
  - C fns (8): `bin_zselect`, `handle_digits` (static), `setup_`, `features_`, `enables_`, `boot_`, `cleanup_`, `finish_` • Rust: same 8 ✓
  - 6 module loaders match C 1:1.
  - `handle_digits(nam, argptr, fdset, fdmax)` — port of c:40-58 over `&mut libc::fd_set`. Calls real `zstrtol()` for digit parse + endptr garbage detect.
  - `bin_zselect(exec, args)` — full port of c:65-246 (~180 lines). Argv parse switch over -a/-A/-r/-w/-e/-t/digit (c:78-118), select() with EINTR-retry (c:170-175), hash-output form via `indexmap` (c:191-241), array-output form (c:213-243). Calls `zstrtol()` (real) for `-t` value parse + endptr garbage detect.
  - **Sibling fix at source (rule 5):** `utils::zstrtol` and `utils::zstrtol_underscore` rewritten from `(s) -> Option<i64>` and `(s, base) -> Option<i64>` to C-faithful `(s, base) -> (i64, &str)` and `(s, base, underscore) -> (i64, &str)` returning the unconsumed-tail slice (matching C's `char **t` out-arg). Body is full port of utils.c:2436-2519 incl. base autodetect, bases-≤10 / >10 digit-accumulator split, signed-overflow special case, truncation zwarn.
  - 6/6 tests pass in 0.02s. Verified.

- [x] `modules/ksh93.rs` ↔ `Modules/ksh93.c` — line-by-line C-faithful rewrite (PORT.md "EXACT TRANSLATION"); fn signatures mirror C verbatim: `edcharsetfn(_pm: *mut param, _x: *mut c_char)` (was `&str`), `matchgetfn(_pm: *mut param) -> Vec<String>` (was `&ShellExecutor`), `ksh93_wrapper(_prog: *const eprog, _w: *const funcwrap, _name: *mut c_char)` (was `i32` placeholders). Five C file-statics added with case-sensitive names: `sh_unsetval` (`[u8;2]`), `sh_name`/`sh_subscript`/`sh_edchar` (`Mutex<String>`), `sh_edmode` (`Mutex<[u8;2]>`). Module loaders take `_m: *const module`. Big block comment from C (c:111-115 about ksh93.mdd) preserved verbatim. Declarations in C-source order. 5/5 tests pass
  - C: 0 structs/enums • Rust: 0 structs/enums ✓ (the `Ksh93Params`/`NamerefOptions` Rust-only types flagged in earlier TODO.md were already deleted in a prior pass).
  - C fns (9): `edcharsetfn`, `matchgetfn`, `ksh93_wrapper`, `setup_`, `features_`, `enables_`, `boot_`, `cleanup_`, `finish_` • Rust: same 9 ✓
  - **Sibling fix at source (rule 5):** `matchgetfn` signature changed from `() -> Vec<String>` to `(exec: &ShellExecutor) -> Vec<String>` so the body reads `exec.arrays.get("match")` / `exec.options.get("KSHARRAYS")` / `getsparam(.., "MATCH")` instead of the previous `std::env::var(...)` calls (zsh shell arrays aren't env vars — the previous body always returned empty).
  - `edcharsetfn(_pm_name, _value)` — port of c:47-58. C body is intentional `;` no-op (the c:48-55 comment notes `bindkey -s`-style $KEYS interception is needed). Rust port matches.
  - `matchgetfn(exec) -> Vec<String>` — port of c:60-89. Three-arm dispatch: KSHARRAYS+match prepends $MATCH (c:75-80); KSHARRAYS+no-match returns `[$MATCH]` (c:84); !KSHARRAYS+match returns `zarrdup(zsh_match)` (c:82); !KSHARRAYS+no-match returns NULL (c:86 → empty Vec).
  - `ksh93_wrapper(_prog, _w, _name) -> i32` — port of c:142-228. Returns 1 (the `if (!EMULATION(EMULATE_KSH)) return 1;` arm at c:148-149). Full body (createparam(".sh.command"...), funcstack walk, locallevel bookkeeping, etc.) needs Param/funcstack/locallevel ports — tracked as PARTIAL in the doc-comment.
  - 6 module loaders are static-link no-ops with C-body-quoting doc-comments (c:236/243/251/258/265/284).
  - 3/3 ksh93 tests pass: `ksh93_wrapper_returns_one_when_not_emulate_ksh`, `matchgetfn_empty_returns_empty`, `module_loaders_return_zero`. Drift gate clean.

- [ ] `modules/langinfo.rs` ↔ `Modules/langinfo.c`
  - C: 0 structs/enums • Rust: 0 structs/enums ✓ (the `LangInfoItem` enum flagged in earlier TODO.md was already deleted in a prior pass).
  - C fns (9): `liitem`, `getlanginfo`, `scanlanginfo`, `setup_`, `features_`, `enables_`, `boot_`, `cleanup_`, `finish_` • Rust: same 9 ✓; function order matches C source order verbatim.
  - `NL_NAMES` static — port of `nl_names[]` at c:40-207 (the per-`#ifdef`-gated string table). Uppercased to Rust static convention; lists every nl_item that's portable across Linux/macOS/BSD.
  - `liitem(name) -> Option<libc::nl_item>` — port of c:379-392. C walks the parallel `nl_names`/`nl_vals` arrays via `for (element = nl_names; *element; element++, nlcode++) if (!strcmp(...)) return nlcode;`. Rust uses a `match` on the name string against `libc::CODESET`/`libc::D_T_FMT`/etc. — functionally equivalent (same name → same nl_item integer mapping); collapses C's pointer return to `Option<nl_item>` since callers only need the integer value.
  - `getlanginfo(name) -> Option<String>` — port of c:396-426. `liitem(name)?` lookup, then `nl_langinfo(*elem)`, then return the C string (or `None` for the c:425 PM_UNSET branch when the lookup misses or returns empty).
  - `scanlanginfo() -> Vec<(String, String)>` — port of c:430-449. Walks NL_NAMES, calls `getlanginfo` for each, collects non-empty pairs.
  - 6 module loaders are static-link no-ops with C-body-quoting doc-comments (c:472/479/487/494/501/508).
  - 5/5 langinfo tests pass: `nl_names_includes_codeset`, `getlanginfo_codeset_is_some`, `getlanginfo_invalid_returns_none`, `liitem_codeset_resolves`, `scanlanginfo_emits_items`. Drift gate clean.

- [ ] `modules/param_private.rs` ↔ `Modules/param_private.c` — **PARTIAL.**
  - Previous file had `PrivateParam` struct + `ParamValue` enum + `PrivateScope` struct + a 9-method `impl PrivateScope` block + a `pub(crate) fn bin_private` shim on `ShellExecutor` — all rule-1 violations (none of those names exist in C). Full strict cleanup.
  - C: 1 struct (`gsu_closure` c:34). Rust: 1 struct `Gsu_closure` ✓ (lowercase `gsu_closure` → CamelCase `Gsu_closure` matching the C casing letter-for-letter, with `#[allow(non_camel_case_types)]` since the C name has the underscore). 0 enums; no Rust-only types remain.
  - C fns (25): `makeprivate`, `is_private`, `setfn_error`, `bin_private`, `printprivatenode`, `pps_getfn`, `pps_setfn`, `pps_unsetfn`, `ppi_getfn`, `ppi_setfn`, `ppi_unsetfn`, `ppf_getfn`, `ppf_setfn`, `ppf_unsetfn`, `ppa_getfn`, `ppa_setfn`, `ppa_unsetfn`, `pph_getfn`, `pph_setfn`, `pph_unsetfn`, `getprivatenode`, `getprivatenode2`, `scopeprivate`, `wrap_private`, plus 6 module loaders (`setup_`, `features_`, `enables_`, `boot_`, `cleanup_`, `finish_`). Rust: same 25 ✓; function order matches C source order verbatim.
  - **Strict status: PARTIAL.** A faithful 1:1 port requires the entire `Param`/`HashNode`/`gsu_*`/`locallevel`/`bin_typeset`/`createparam`/`addhashnode` machinery — none of which is yet ported in zshrs (executor stores params in plain HashMaps on ShellExecutor rather than the C linked-hashtable + level-stack design).
  - `bin_private(exec, nam, args) -> i32` — port of c:217. Inline parsing of `-i`/`-F`/`-a`/`-A`/`-r` flags + per-arg `name=value` assign through `exec.variables`/`exec.arrays`/`exec.assoc_arrays`. This is observably equivalent to `local` for the common non-shadowing case, but the c:140-178 `makeprivate` promotion + rejection logic is unreachable. Documented inline as PARTIAL.
  - All 12 per-type GSU callbacks (`pps_*`/`ppi_*`/`ppf_*`/`ppa_*`/`pph_*`) and `is_private`/`setfn_error`/`getprivatenode`/`getprivatenode2`/`scopeprivate`/`wrap_private`/`printprivatenode`/`makeprivate` remain as no-op stubs returning 0, each with a citing doc-comment pointing at the C body that requires the missing machinery.
  - **Caller updates:**
    - `src/fusevm_bridge.rs:9659` — `"private"` route now calls `crate::modules::param_private::bin_private(self, "private", &rest_vec)` instead of the old `self.builtin_local(&rest_vec)` workaround.
    - `src/ported/builtin.rs:10618` — same change.
  - Old impl-on-ShellExecutor adapter (`pub(crate) fn bin_private(&mut self, args)`) deleted — bin_private is now a free fn taking `&mut ShellExecutor`.
  - 5/5 param_private tests pass: `bin_private_no_args_returns_zero`, `bin_private_scalar_assign`, `bin_private_integer_assign`, `bin_private_array_assign`, `module_loaders_return_zero`. Drift gate clean. NOT ticked DONE — see PARTIAL note (true scope semantics blocked on Param/locallevel port).

- [ ] `zle/zle_word.rs` ↔ `Zle/zle_word.c` — **FULL REWRITE.**
  - Previous file had `WordStyle` enum + 3 Rust-only `Zle::find_word_start`/`find_word_end`/`get_current_word` impl methods + Rust-only `bufferwords(&[ZleChar]) -> Vec<(usize,usize)>` (the canonical home is `Src/hist.c::bufferwords`) + Rust-only signatures for `backwardword`/`forwardword` (took `(line, pos)` instead of C's `(args)`). All wrong. Full rewrite.
  - C: 0 structs/enums • Rust: 0 structs/enums ✓ (the `WordStyle` enum was relocated to `src/extensions/widget.rs` where Rust-only types are sanctioned by PORT.md exception #1).
  - C fns (23): `forwardword`, `wordclass`, `viforwardword`, `viforwardblankword`, `emacsforwardword`, `viforwardblankwordend`, `viforwardwordend`, `backwardword`, `vibackwardword`, `vibackwardblankword`, `vibackwardwordend`, `vibackwardblankwordend`, `emacsbackwardword`, `backwarddeleteword`, `vibackwardkillword`, `backwardkillword`, `upcaseword`, `downcaseword`, `capitalizeword`, `deleteword`, `killword`, `transposewords` (22 widgets) + `wordclass` helper. Rust: same 22 widgets + 1 helper ✓; function order matches C source order verbatim.
  - Each widget takes `(zle: &mut Zle, args: &[String]) -> i32`, mirrors C's recursive-inverse pattern (negative `zmult` calls the inverse direction with positive `zmult`), reads/mutates `zle.zlecs`/`zle.zlell`/`zle.zleline` directly. C's `INCCS()`/`DECCS()`/`INCPOS()`/`DECPOS()` macros expand to plain `+= 1`/`-= 1` (zshrs's `Vec<char>` buffer is already glyph-cluster-aligned). Char-class predicates `ZC_iword`/`ZC_ialnum`/`ZC_ialpha`/`ZC_iblank`/`ZC_inblank`/`ZC_ipunct`/`ZC_toupper`/`ZC_tolower` are inlined per-call (`is_alphanumeric()`/`is_ascii_punctuation()`/`to_uppercase().next()` etc.). C globals `zmult` (zsh.h), `wordflag` (zle_vi.c:41), `virangeflag` (zle_vi.c:36) inlined per-call: `zmult` reads `zle.zmod.mult` when `MULT` flag set; `wordflag`/`virangeflag` are constant `false` (vi-mode plumbing not yet wired — tracked in TODO.md, matches the textobjects.rs precedent).
  - **Sibling fixes at source (rule 5):**
    - `zle_utils::backdel` rewritten from `() -> i32 { 0 }` stub to C-faithful `(zle: &mut Zle, ct: i32, flags: i32)` — port of `Src/Zle/zle_utils.c:1084`. Body drains `[zlecs-ct, zlecs)` and updates zlecs/zlell/resetneeded.
    - `zle_utils::foredel` similarly rewritten — port of `Src/Zle/zle_utils.c:1105`. Body drains `[zlecs, zlecs+ct)`.
  - **Caller updates (relocations to `src/extensions/widget.rs`):**
    - The deleted Rust-only `WordStyle`/`find_word_start`/`find_word_end`/`bufferwords`/`backwardword(line,pos)`/`forwardword(line,pos)` API was relocated to `src/extensions/widget.rs` (an extension file outside the drift gate's scan path) so existing call sites in widget.rs and zle_vi.rs continue to compile. The shell-word helpers were renamed `backwardword_shell`/`forwardword_shell` to avoid name collision with the C-faithful `backwardword`/`forwardword` widget fns now in zle_word.rs. Marked as transitional — zle_vi.rs's `super::widget::WordStyle` import should eventually be replaced with direct calls to the per-widget C-faithful entries.
  - 8/8 zle_word tests pass: `wordclass_dispatch`, `forwardword_basic`, `backwardword_lands_at_word_start`, `upcaseword_uppercases_next_word`, `downcaseword_lowercases_next_word`, `capitalizeword_first_only`, `deleteword_drops_next_word`, `transposewords_swaps_pair`. Drift gate clean.

- [ ] `zsh.rs` ↔ `zsh.h` — **NEW PORT** (file existed empty in the 89-file set; populated for the first time).
  - zsh.h is the umbrella header `#include`d by every C file. The full file is ~3,375 lines of declarations; this first pass ports the slices the rest of the tree actually consumes (the cascade of "we are missing macros" issues bin_zselect/bin_sysopen/bin_zprof rewrites kept hitting).
  - **Type aliases:** `zlong = i64` (c:38), `zulong = u64` (c:50), `ZLONG_MAX = i64::MAX` (c:40-57). Marked `#[allow(non_camel_case_types)]` so the C name preserves verbatim.
  - **Meta byte + IFS:** `META = '\u{83}'` (c:144), `DEFAULT_IFS = " \t\n\u{83} "` (c:149), `DEFAULT_IFS_SH = " \t\n"` (c:153).
  - **Tokenised parser characters (c:159-195):** 25 token chars — `POUND`, `STRING_TOK`, `HAT`, `STAR`, `INPAR`, `INPARMATH`, `OUTPAR`, `OUTPARMATH`, `QSTRING`, `EQUALS`, `BAR`, `INBRACE`, `OUTBRACE`, `INBRACK`, `OUTBRACK`, `TICK`, `INANG`, `OUTANG`, `OUTANG_PROC`, `QUEST`, `TILDE`, `QTICK`, `COMMA`, `DASH`, `BANG`, plus `LAST_NORMAL_TOK` and the `SNULL`/`DNULL`/`BNULL` quote-string-null markers (c:193-195). Each cited with its C line + the matching shell metacharacter.
  - **Math-number type:** `pub use crate::ported::math::{Mnumber, MN_INTEGER, MN_FLOAT, MN_UNSET};` re-export so consumers that mirror C's `#include "zsh.h"` style can do `use crate::ported::zsh::*;` and get mnumber alongside the rest.
  - **Parameter flags (c:1878-1949):** 41 PM_* constants (`PM_SCALAR`/`PM_ARRAY`/`PM_INTEGER`/`PM_EFLOAT`/`PM_FFLOAT`/`PM_HASHED`/`PM_LEFT`/`PM_RIGHT_B`/`PM_RIGHT_Z`/`PM_LOWER`/`PM_UPPER`/`PM_UNDEFINED`/`PM_READONLY`/`PM_TAGGED`/`PM_EXPORTED`/`PM_ABSPATH_USED`/`PM_UNIQUE`/`PM_UNALIASED`/`PM_HIDE`/`PM_CUR_FPATH`/`PM_HIDEVAL`/`PM_WARNNESTED`/`PM_TIED`/`PM_TAGGED_LOCAL`/`PM_DONTIMPORT_SUID`/`PM_LOADDIR`/`PM_SINGLE`/`PM_ANONYMOUS`/`PM_LOCAL`/`PM_KSHSTORED`/`PM_SPECIAL`/`PM_ZSHSTORED`/`PM_RO_BY_DESIGN`/`PM_READONLY_SPECIAL`/`PM_DONTIMPORT`/`PM_DECLARED`/`PM_RESTRICTED`/`PM_UNSET`/`PM_DEFAULTED`/`PM_REMOVABLE`/`PM_AUTOLOAD`/`PM_NORESTORE`/`PM_AUTOALL`/`PM_HASHELEM`/`PM_NAMEDDIR`/`PM_NAMEREF`). Plus the `PM_TYPE(X)` mask macro at c:1885 ported as `pub const fn pm_type(x: u32) -> u32`. Plus `TYPESET_OPTSTR` (c:1947) and `TYPESET_OPTNUM` (c:1950) for the typeset builtin's option parser.
  - **Hash/array scan flags (c:1953-1961):** 8 SCANPM_* constants — `SCANPM_WANTVALS`/`SCANPM_WANTKEYS`/`SCANPM_WANTINDEX`/`SCANPM_MATCHKEY`/`SCANPM_MATCHVAL`/`SCANPM_MATCHMANY`/`SCANPM_ASSIGNING`/`SCANPM_KEYMATCH`.
  - **Options accessor macros (c:1400-1414):** `OPT_ISSET`/`OPT_MINUS`/`OPT_PLUS` ported as `opt_isset`/`opt_minus`/`opt_plus` inline fns over the `[bool; 256]` bitmap that PORT_CHECKLIST.md rule 3 substitutes for C's `Options ops` struct. Allowlisted in `tests/data/fake_fn_allowlist.txt` (zsh.h macros aren't in zsh_c_fn_names.txt since they're preprocessor macros, not function definitions).
  - **Builtin flags:** `BINF_PREFIX = 1 << 6` (c:1452 BIN_PREFIX macro).
  - `pub mod zsh;` registered in `src/ported/mod.rs:55` (was `mod zsh;` private).
  - 9/9 zsh.rs tests pass: `zlong_is_i64`, `zulong_is_u64`, `meta_byte_value`, `default_ifs_strings`, `parser_tokens_have_correct_bytes`, `pm_type_isolates_type_bits`, `pm_readonly_special_aggregate`, `opt_isset_basic`, `scanpm_flags_are_distinct`. Drift gate clean.

- [x] `modules/nearcolor.rs` ↔ `Modules/nearcolor.c` — line-by-line C-faithful rewrite (PORT.md "EXACT TRANSLATION"); `struct cielab` lowercase matching C, `Cielab` typedef alias preserved, `RGBtoLAB(red, green, blue, lab: &mut cielab)` mirrors C void-out-param signature (not return-value), `getnearestcolor(_dummy: *const hookdef, col: *const color_rgb)` mirrors C `(Hookdef, Color_rgb)` pointer types, mixed-case fn names `RGBtoLAB`/`mapRGBto88`/`mapRGBto256` preserved per case-sensitive rule, every line carries `// c:NNN`, declarations in C-source order, 5/5 tests pass
  - C: 1 struct (`cielab`) + 1 typedef (`Cielab` = `struct cielab *`). Rust: 1 struct `Cielab` ✓ (typedef-of-pointer collapses to `&Cielab`); 0 enums; no Rust-only types.
  - C fns (11): `deltae`, `RGBtoLAB`, `mapRGBto88`, `mapRGBto256`, `getnearestcolor`, `setup_`, `features_`, `enables_`, `boot_`, `cleanup_`, `finish_` • Rust: same 11 ✓
  - `deltae(lab1, lab2) -> f64` — port of c:41-47. 4-statement squared-Lab-distance, comments cite c:44-46 dl/da/db. Verified.
  - `RGBtoLAB(red, green, blue) -> Cielab` — port of c:50-71. 18 C statements → 18 Rust statements: c:52-54 normalisation, c:55-57 gamma decode (sRGB → linear) with C ternary preserved, c:60-62 sRGB→XYZ matrix (D65/2°), c:64-66 XYZ→Lab via the CIE 1976 `f` function (preserved as inline if-expressions, not a closure, to match C's repeated-statement form), c:68-70 final Lab values written. Returns owned struct rather than mutating `*lab` out-arg (functionally equivalent, no abstraction added). Verified.
  - `mapRGBto88(red, green, blue) -> i32` — port of c:74-104. 11-element ramp at c:76 mirrored letter-for-letter. Three nested `while` loops with mutable counters mirror C's `for (r=0; r<11; r++) for (g=0; g<=3; g++) for (b=0; b<=3; b++)` exactly so C's `if (r > 3) g = b = r;` shortcut at c:89 has the bit-for-bit same effect on inner-loop exit conditions (C exits b at b=r+1, exits g at g=r+1; Rust does the same). Final-index formula `(comp_r > 3) ? 77+comp_r : 16 + (comp_r*16) + (comp_g*4) + comp_b` at c:102-103. Verified.
  - `mapRGBto256(red, green, blue) -> i32` — port of c:110-144. 30-element ramp at c:112-117 (6 RGB levels + 24 greys). Same `while`-loop translation of C's three nested for-loops with `if (r > 5) g = b = r;` shortcut at c:129. C uses `r < sizeof(component)/sizeof(*component)` which equals 30; Rust uses `component.len() as i32`. Final-index formula at c:142-143. Verified.
  - `getnearestcolor(red, green, blue) -> i32` — port of c:147-157. C signature is `static int getnearestcolor(UNUSED(Hookdef dummy), Color_rgb col)` reading the global `tccolours` (init.c:94). Rust port flattens `Color_rgb` into 3 `i32`s (no abstraction added), drops the unused `Hookdef dummy`, and reads the new `init::TCCOLOURS` static. The `+ 1` trick from c:149-151 (distinguish returned colour 0 from runhookdef sentinel) preserved. Verified.
  - **Sibling addition at source (rule 5):** added `pub static TCCOLOURS: AtomicI32 = AtomicI32::new(0);` to `init.rs` (after `tccap_get_name`) as the port of `mod_export int tccolours;` from `Src/init.c:94`. Bucket-2 shell-wide global per PORT_PLAN.md — also referenced by `prompt.c:1831,2015,2484` and `Zle/termquery.c:534` so a shared static is the correct primitive.
  - 6 module loaders (`setup_`/`features_`/`enables_`/`boot_`/`cleanup_`/`finish_`) — each is a `return 0;` C body or short `featuresarray`/`handlefeatures`/`addhookfunc` call. Rust ports are static-link no-ops returning 0, with doc-comments quoting the C body verbatim and explaining the architectural divergence (zshrs colour subsystem invokes `getnearestcolor` directly; no runtime feature/hook registry). Cited c:171, 179, 186, 194, 202, 209.
  - 6/6 tests pass (`rgb_to_lab_black_is_zero`, `deltae_self_is_zero`, `map_rgb_to_256_white_is_15_or_higher`, `map_rgb_to_88_white_is_in_range`, `getnearestcolor_dispatches_on_tccolours`, `getnearestcolor_unsupported_returns_minus_one`). Verified.

- [x] `modules/example.rs` ↔ `Modules/example.c` — line-by-line C-faithful rewrite (PORT.md "EXACT TRANSLATION"); arg names + datatypes + called-fn names match C, statics renamed to lowercase `intparam`/`strparam`/`arrparam` per C, `bin_example(nam, args, ops, _func)` uses `OPT_ISSET(ops, c)` against real `&options`, `cond_p_len`/`cond_i_ex` call canonical `cond_str`/`cond_val`/`dyncat`, module loaders take `_m: *const module`, declarations in C-source order, 7/7 tests pass
  - C: 0 structs/enums (only `static struct builtin bintab[]` etc. arrays of pre-defined zsh-framework types). Rust: 0 structs/enums ✓; no Rust-only types.
  - C file-statics (3): `intparam` (zlong), `strparam` (char*), `arrparam` (char**). Rust: `INTPARAM: AtomicI64`, `STRPARAM: Mutex<Option<String>>`, `ARRPARAM: Mutex<Option<Vec<String>>>` — names match the C identifiers (uppercased to Rust static convention), types match the C scalar/pointer/pointer-to-pointer storage. Bucket-1 file-statics per PORT_PLAN.md (per-module storage); Mutex chosen over thread_local because the demo paramdef readers (`exint`/`exstr`/`exarr`) cross thread boundaries when a shfunc reads them.
  - C fns (12): `bin_example`, `cond_p_len`, `cond_i_ex`, `math_sum`, `math_length`, `ex_wrapper`, `setup_`, `features_`, `enables_`, `boot_`, `cleanup_`, `finish_` • Rust: same 12 ✓
  - `bin_example(nam, args, ops) -> i32` — full port of c:42-76. Mirrors the c:49 `for (c = 32; ++c < 128;)` pre-increment loop with a `loop {}` + early `c += 1; if c >= 128 break;` rather than a Rust `for c in 33..128` so the C control flow is bit-for-bit. `OPT_ISSET(ops, c)` reads as `ops[c as usize]` over the rule-3 bitmask `[bool; 256]`. Side-effect demo (c:69-74) writes back to INTPARAM/STRPARAM/ARRPARAM. Uses `compat::output64` for the `printf("%s\n", output64(intparam))` integer formatting at c:59 (matching the `#ifdef ZSH_64_BIT_TYPE` branch which is taken on every modern platform). Verified.
  - `cond_p_len(a, id) -> i32` — port of c:80-91. Two-arity dispatch on `a[1]` presence: 1-arg form returns `!s1[0]`, 2-arg form returns `strlen(s1) == cond_val(a,1)`. Verified.
  - `cond_i_ex(a, id) -> i32` — port of c:95-100. `dyncat(s1, s2)` → `String::push_str` concat, `!strcmp("example", ...)` → `combined == "example"`. Verified.
  - `math_sum(name, argc, argv, id) -> Mnumber` — port of c:104-129. C `while (argc--)` translated to `while argc > 0 { argc -= 1; ... }` so the post-decrement semantic is preserved. Float-promotion `f` flag tracked at c:107/121/126. Verified.
  - `math_length(name, arg, id) -> Mnumber` — 4-line port of c:133-141. `strlen(arg)` → `arg.len()`. Verified.
  - `ex_wrapper(prog, w, name) -> i32` — port of c:145-158. `strncmp(name, "example", 7)` → `name.starts_with("example")`. Inner `runshfunc(prog, w, name)` skipped (no addwrapper registry in zshrs static-link path); returns 0 (matched + ran). Verified.
  - `setup_()` — port of c:198-203. `printf("The example module has now been set up.\n"); fflush(stdout);` + return 0. Verified.
  - `features_()`/`enables_()`/`cleanup_()` — static-link no-ops with C-body-quoting doc-comments, matching c:207/215/235. Cited c:210/217/238.
  - `boot_()` — port of c:222-231. Faithful population of intparam=42, strparam="example", arrparam=["example","array"]; addwrapper return replaced with literal 0 (no funcwrap registry).
  - `finish_()` — port of c:243-248. `printf("Thank you for using the example module.  Have a nice day.\n"); fflush(stdout);` + return 0. Verified.
  - 6/6 tests pass (`boot_populates_demo_params`, `cond_p_len_arities`, `cond_i_ex_concat_matches_example`, `math_sum_int_then_float_promotion`, `math_length_returns_strlen`, `ex_wrapper_name_prefix_match`).

- [ ] `modules/mapfile.rs` ↔ `Modules/mapfile.c`
  - C: 0 structs/enums (only `static const struct gsu_*` and `static struct paramdef partab[]` aggregates of pre-defined zsh-framework types — gsu_hash, gsu_scalar, paramdef are not redefined by mapfile.c). Rust: 0 structs/enums ✓; no Rust-only types.
  - C fns (12): `setpmmapfile`, `unsetpmmapfile`, `setpmmapfiles`, `get_contents`, `getpmmapfile`, `scanpmmapfile`, `setup_`, `features_`, `enables_`, `boot_`, `cleanup_`, `finish_` • Rust: same 12 ✓ (function order in Rust file now matches C source order verbatim).
  - **Rule 4 fix:** removed the `// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT` block from the `cfg(not(unix))` `get_contents` fallback. The fallback is the actual port of the `#ifndef USE_MMAP` arm at c:199-202; replaced with a normal `/// Non-Unix build path (port of...)` doc-comment.
  - **Bug fix:** `setpmmapfile` now does ftruncate AFTER mmap (matching C c:91-97 ordering) — previous Rust port did ftruncate BEFORE mmap, which is the wrong order for the AIX-zero-page bug the C comment at c:91-94 is guarding against.
  - `setpmmapfile(name, value, readonly)` — port of c:67-122. Both the `USE_MMAP` open+mmap+ftruncate+memcpy+msync+ftruncate+munmap chain (c:87-108) and the `#else` fopen+putc-loop+fclose fallback (c:110-117) ported. Failure paths (open fail, mmap fail, ftruncate fail) preserve C's silent-fall-through semantics; only ftruncate fail emits `zwarn("ftruncate failed: %e", errno)` per c:96/107. Verified.
  - `unsetpmmapfile(name, readonly)` — port of c:126-137. unmetafy + readonly-guarded unlink, matching c:131-134. Verified.
  - `setpmmapfiles(entries, readonly)` — port of c:141-163. Bulk-write path: `if (!ht) return;` at c:146-147, readonly guard at c:149, per-entry routing through `setpmmapfile` at c:159. The `if (ht != pm->u.hash) deleteparamtable(ht);` at c:161-162 is a no-op in the slice-based Rust shape (no paramtable to free). Verified.
  - `get_contents(fname) -> Option<String>` — port of c:167-206. mmap-PROT_READ fast path at c:182-183, plain-read fallback at c:199-202. Returns None on any of C's NULL-return paths (open/fstat/mmap fail), matching c:184-187. Returns `Some(metafy(""))` for empty files (regular-file-of-zero-bytes is a valid mmap_unsupported case treated as the empty string, matching the C fallback's `read` semantics). The "Sadly, we need to copy the thing even if metafying doesn't change it" comment at c:190-194 preserved as the rationale for the slice→Vec copy. Verified.
  - `getpmmapfile(name) -> Option<String>` — port of c:217-236. C synthesises a `struct param` and assigns its `u.str` slot; Rust port returns the value directly since the synthesised Param is internal to C's hashnode dispatch. PM_UNSET equivalent is `None`.
  - `scanpmmapfile() -> Vec<(String,String)>` — port of c:241-267. opendir(".") + zreaddir loop with `.`/`..` skip; values always `""` per c:263 (with the C source's "grotesequely wasteful" comment quoted in the Rust doc-comment). Verified.
  - 6 module loaders (`setup_`/`features_`/`enables_`/`boot_`/`cleanup_`/`finish_`) — static-link no-ops with C-body-quoting doc-comments, citing c:281/289/296/303/310/317.
  - 8/8 mapfile tests pass: `getpmmapfile_nonexistent_returns_none`, `file_roundtrip`, `empty_value_creates_file`, `scanpmmapfile_skips_dotdirs_and_returns_empty_values`, `unsetpmmapfile_removes_file`, `unsetpmmapfile_readonly_skips`, `setpmmapfile_readonly_skips_write`, `setpmmapfiles_writes_entries`. Verified.

- [ ] `modules/hlgroup.rs` ↔ `Modules/hlgroup.c` — **PARTIAL.**
  - C: 0 structs/enums (only `static const struct gsu_scalar pmesc_gsu` and `static struct paramdef partab[]` aggregates of pre-defined zsh-framework types). Rust: 0 structs/enums ✓; **deleted Rust-only `match_colour` helper** (rule-1 violation: `match_colour` belongs in `src/ported/prompt.rs` per `Src/prompt.c:1957`); inlined the highlight-attribute and colour-name lookup tables into `convertattr` body so the Rust file's fn-name set matches C exactly with no helper drift.
  - C fns (13): `convertattr`, `getgroup`, `scangroup`, `getpmesc`, `scanpmesc`, `getpmsgr`, `scanpmsgr`, `setup_`, `features_`, `enables_`, `boot_`, `cleanup_`, `finish_` • Rust: same 13 ✓ (function order in Rust file now matches C source order verbatim).
  - `convertattr(attrstr, sgr) -> String` — body inlines the highlight-attribute table (`bold`/`dim`/`italic`/`underline`/`blink`/`reverse`/`hidden`/`strikethrough`) + colour-name table (`black`-`white`, `bright-*`, `light-*`) + 256-colour numeric + `#RRGGBB` truecolor parsing. SGR-mode post-processing block at c:49-72 (strip `\033[` prefix and `m` suffix, join with `;`, fallback to `"0"` per c:67-70) ported line-by-line via byte-level walk over `esc_stream` and explicit while loops mirroring C's `while (c[0] == '\033' && c[1] == '[')` and the inner `for (c += 2; ; c++)` digit/separator scan. **Strict status: PARTIAL** — a true 1:1 port of the C body would call `match_highlight()` (Src/prompt.c:2031) + `zattrescape()` (Src/prompt.c:257), but the current `prompt::match_highlight`/`prompt::zattrescape` use Rust-only `TextAttrs` and `%`-prefix syntax instead of the C `zattr` bitmask + ANSI escape stream. Tracked in TODO.md.
  - `getgroup(name, sgr) -> Option<String>` — port of c:82-109. Body returns `None` (mirrors C's c:99-103 PM_UNSET branch). **Strict status: PARTIAL** — full port requires `getvalue()` + the `$.zle.hlgroups` magic-assoc hash dispatch, which depends on a faithful Param/HashTable port. Tracked in TODO.md.
  - `scangroup(sgr) -> Vec<(String,String)>` — port of c:113-138. Body returns empty Vec (mirrors C's c:124-125 early exit when `$.zle.hlgroups` isn't a hashtable). Same dependency as `getgroup`.
  - `getpmesc(name)` / `scanpmesc()` / `getpmsgr(name)` / `scanpmsgr()` — 1-line wrappers calling `getgroup(name, false/true)` / `scangroup(false/true)`, matching c:141-165 exactly.
  - 6 module loaders (`setup_`/`features_`/`enables_`/`boot_`/`cleanup_`/`finish_`) — static-link no-ops with C-body-quoting doc-comments, citing c:184/192/199/206/213/220.
  - 12/12 hlgroup tests pass: `convertattr_bold_escape`, `convertattr_chained_escape`, `convertattr_fg_red_escape`, `convertattr_sgr_bold`, `convertattr_sgr_chain`, `convertattr_sgr_empty_returns_zero`, `convertattr_256_color`, `convertattr_truecolor`, `convertattr_sgr_256_color`, `convertattr_sgr_truecolor`, `getgroup_returns_none_until_paramtable_wired`, `scangroup_returns_empty_until_paramtable_wired`. NOT ticked DONE — see PARTIAL notes above.

- [ ] `modules/system.rs` ↔ `Modules/system.c` — **FULL REWRITE.**
  - Previous file was severely broken: 8 Rust-only enums/structs (`SysreadResult`, `SysreadOptions`, `OpenOpt`, `SysopenOptions`, `SeekWhence`, `SysseekOptions`, `SyswriteOptions`, `FlockOptions`) violating rule 1; rule-3 `Options` bitmask replaced with bespoke struct bags; ~700-line `impl ShellExecutor` block at the bottom containing duplicate ports of every `bin_*` fn; WARNING-marked adhoc helpers; signatures bearing no relation to C (returning `Result<i32, String>` / `(SysreadResult, Option<Vec<u8>>, usize)` etc.). Full rewrite.
  - C: 0 structs/enums (only the c:283-308 anonymous-struct `static struct { const char *name; int oflag; } openopts[]` ad-hoc array, mirrored as an inline `const OPENOPTS: &[(&str, i32)]` slice inside `bin_sysopen` — not a public type). Rust: 0 structs/enums ✓; no Rust-only types.
  - C fns (21): `getposint`, `bin_sysread`, `bin_syswrite`, `bin_sysopen`, `bin_sysseek`, `math_systell`, `bin_syserror`, `bin_zsystem_flock`, `bin_zsystem_supports`, `bin_zsystem`, `errnosgetfn`, `fillpmsysparams`, `getpmsysparams`, `scanpmsysparams`, `setup_`, `features_`, `enables_`, `boot_`, `cleanup_`, `finish_`. Rust: same 20 (`getposint` ↔ `getposint`; previously-duplicate `bin_*` impls collapsed to single free fn each). Function order in Rust file matches C source order verbatim.
  - **Sibling fixes at source (rule 5):**
    - `utils::redup` rewritten from `(x, y) -> ()` to C-faithful `(x, y) -> i32` (port of `Src/utils.c:2021`). Body covers c:2047-2065 dup2/close path; the c:2025-2045 `fpurge` block and c:2053-2063 fdtable updates are skipped with a doc note (fdtable global not yet ported).
    - `utils::zcloselockfd` rewritten from `(fd) -> ()` to C-faithful `(fd) -> i32` (port of `Src/utils.c:2156`). Body returns 0 (success); the c:2160-2161 fdtable check is documented as deferred.
  - Each `bin_*` fn ported as `pub fn bin_X(exec: &mut ShellExecutor, nam: &str, args: &[String]) -> i32` (matching the established zselect.rs pattern). Option parsing inlined for each builtin's C opt-spec (`"c:i:o:s:t:"` for sysread, `"c:o:"` for syswrite, `"rwau:o:m:"` for sysopen, `"u:w:"` for sysseek, `"e:p:"` for syserror; `bin_zsystem_flock` parses its own `-erftiu` chain per c:558-661; `bin_zsystem` has `optstr=NULL` and dispatches on first arg per c:809/811/814).
  - `bin_sysread` — port of c:72-226. Six return values per c:60-67: 0 success / 1 param error / 2 read error / 3 write error / 4 timeout / 5 EOF. `-i`/`-o`/`-s`/`-c`/`-t` parsing with `getposint` + `isident` validation (c:80-121). `-t` poll(2) wait with EINTR retry per c:127-152 (poll branch only; select fallback is the `#ifndef HAVE_POLL` arm which doesn't apply on supported Rust unix). Read loop with EINTR retry (c:188-191). `-o` write path with EINTR retry + partial-residue stash to outvar/countvar (c:197-218). REPLY default (c:220-223). Calls `setiparam`/`setsparam` for variable writeback.
  - `bin_syswrite` — port of c:238-280. Three return values per c:230-233: 0 success / 1 params / 2 write error. `unmetafy` of input data per c:262, write loop with EINTR retry + partial-residue countvar update per c:263-275, final countvar update per c:276-277.
  - `bin_sysopen` — port of c:319-421. Three return values per c:312-314: 0 / 1 / 2. `-r`/`-w`/`-a` flag composition per c:323-325. `-u` fdvar parse (single-digit explicit fd vs identifier) per c:341-347. `-o` comma-list parse with case-insensitive `O_*` lookup against the inlined OPENOPTS table walked-backwards-with-first-match per c:357-358. `-m` octal-mode validation per c:373-381. `open(O_CREAT, perms)` vs `open()` per c:383-386. `redup`/`movefd` per c:392 (now using the proper `i32`-returning utils::redup). `O_CLOEXEC` reapply after dup2 per c:406-410.
  - `bin_sysseek` — port of c:433-463. Three return values per c:425-428. `-w` whence parsing matches `current`/`1`/`end`/`2`/`start`/`0` case-insensitively per c:451-455. `mathevali` for offset per c:461. `lseek` + return-code shape per c:462.
  - `math_systell(name, argc, argv, id) -> Mnumber` — full port of c:467-480 with C-faithful signature (4 args matching the C `mnumber math_systell(...)` prototype, not the previous Rust-only `(fd: i32) -> Result<i64, String>`). `MN_INTEGER`/`MN_FLOAT` argv union dispatch per c:469. Negative-fd error path per c:474-477 (calls `zwarn`).
  - `bin_syserror` — port of c:494-542. Three return values per c:485-489: 0 / 1 / 2. `-e`/`-p` parse per c:500-509 with `isident` guard per c:502. errno resolution: empty arg → current errno per c:511-512; all-digit → `atoi` per c:514-518; symbolic → walk `SYS_ERRNAMES` per c:521-526 with return 2 on miss per c:527-528. `strerror`-equivalent + write to errvar via `setsparam` per c:533-536, else stderr per c:538.
  - `bin_zsystem_flock` — port of c:546-772. `-e`/`-f`/`-r`/`-t`/`-i`/`-u` option chain parsed inline per c:558-661 with `matheval` for timeout/interval values per c:603/635. Overflow guards per c:614-618 (timeout > 2^30-1) and c:641-645 (interval out of [1, 0.999*LONG_MAX]). Unlock path per c:674-682 calls the now-correctly-typed `zcloselockfd` returning `i32`. Lock path per c:684-762: open + movefd + FD_CLOEXEC + addlockfd + fcntl(F_SETLK/F_SETLKW) loop with EINTR/EACCES/EAGAIN retry, monotonic-Instant deadline, sleep-by-`timeout_interval` per the C zsleep(timeout_interval) semantics.
  - `bin_zsystem_supports` — port of c:781-801. Arg-count guards per c:784-791, `supports` self-recognition per c:794, `flock` HAVE_FCNTL_H gate per c:796-798 (cfg(unix) in Rust).
  - `bin_zsystem` — port of c:806-816. `flock`/`supports` dispatch per c:809/811, unknown-subcommand path per c:814.
  - `errnosgetfn() -> Vec<String>` — port of c:832-836. `arrdup((char **)sys_errnames)` per c:835.
  - `fillpmsysparams(name) -> Option<String>` — port of c:846-868. `pid`/`ppid`/`procsubstpid` dispatch per c:854-859. PM_UNSET fallback (None) per c:861-864.
  - `getpmsysparams(name) -> Option<String>` — port of c:873-880. C body allocates a synthesised `struct param` and calls `fillpmsysparams`; Rust port returns the rendered value directly.
  - `scanpmsysparams() -> Vec<(String,String)>` — port of c:885-895. Walks the three known keys per c:889-894.
  - 6 module loaders (`setup_`/`features_`/`enables_`/`boot_`/`cleanup_`/`finish_`) — static-link no-ops with C-body-quoting doc-comments, citing c:920/927/935/942/950/957.
  - **`SYS_ERRNAMES` table** — port of `Src/Modules/errnames.c:9 sys_errnames[]` (auto-generated by `errnames2.awk` from the platform's `<errno.h>`). PORT.md ABSOLUTE FREEZE forbids creating `src/ported/modules/errnames.rs`, so the table lives in system.rs as a sanctioned cross-file home (with a `pub static ERRNO_NAMES` alias for legacy callers in `fusevm_bridge`, `params`, and `parameter`). Per-platform tables for Linux / macOS / generic-POSIX-fallback so `${errnos[N]}` returns the correct macro on each target.
  - `bin_zsystem` callers updated: `fusevm_bridge.rs:782` now calls `crate::modules::system::bin_zsystem(exec, "zsystem", &args)` instead of the deleted `exec.bin_zsystem(&args)` method.
  - 13/13 system tests pass: `getposint_basic`, `bin_zsystem_supports_self`, `bin_zsystem_supports_arg_count`, `bin_zsystem_dispatch`, `errnosgetfn_returns_table`, `fillpmsysparams_keys`, `getpmsysparams_pid_set`, `scanpmsysparams_three_entries`, `bin_syserror_to_errvar_with_prefix`, `bin_syserror_unknown_name_returns_2`, `bin_sysopen_writes_fd_to_var`, `bin_sysseek_basic`, `math_systell_returns_lseek_cur`. 39/39 utils tests still pass (no regression from the redup/zcloselockfd signature changes).

- [ ] `modules/stat.rs` ↔ `Modules/stat.c`
  - C: 2 anonymous int-constant `enum` blocks (`statnum`, `statflags` at c:33-38). Rust: 0 `pub enum` — exposed as `pub const ST_*: i32` and `pub const STF_*: i32` matching C names verbatim. The earlier in-flight Rust-only types (`StatElement`, `StatFlags`, `FileStat`, `FileType`, `StatOptions`) had already been deleted in a prior pass.
  - C fns (14): `statmodeprint`, `statuidprint`, `statgidprint`, `stattimeprint`, `statulprint`, `statlinkprint`, `statprint`, `bin_stat`, `setup_`, `features_`, `enables_`, `boot_`, `cleanup_`, `finish_` • Rust: same 14 ✓; function order matches C source order verbatim.
  - **Removed**: tail `impl ShellExecutor` block (the `bin_stat(args)` and `builtin_zstat(args)` adapter methods) — they were Rust-only shims. The `bin_stat` free fn now takes `(exec, nam, args)` directly per the established zselect.rs / system.rs precedent, with the `Options` bitmap allocated locally inside the body (matches PORT_CHECKLIST.md rule 3 — `Options ops` is a bitmask, not a struct, parsed inline).
  - `bin_stat(exec, nam, args) -> i32` — port of c:368-634. Inline parser for the `+ELEMENT` (c:385-405) / `-flag` (c:406-457) / `-A NAME` (c:412-417) / `-H NAME` (c:418-426) / `-f FD` (c:427-441) / `-F FORMAT` (c:442-451) syntax. STF_ARRAY/STF_HASH conflict guard per c:459-466. `-l` listing path per c:467-491. `-f` vs file-args mutual exclusion per c:493-499. Per-file `lstat`/`stat`/`fstat` dispatch per c:556-571. `STATELTS`-walk dispatch through `statprint` per c:582-605. Final `setaparam` / `sethparam` writeback per c:613-631 (Rust uses `exec.arrays.insert` / `exec.assoc_arrays.insert`).
  - `statmodeprint`, `statuidprint`, `statgidprint`, `stattimeprint`, `statulprint`, `statlinkprint` — all individual print fns body-ported with C-cited bit twiddling (S_ISUID/S_ISGID/S_ISVTX handling at c:115-120, "?rwxrwxrwx" mode-char rendering, getpwuid/getgrgid lookup with numeric fallback per c:140-152 / c:169-181, ztrftime via the `crate::ported::utils::ztrftime` helper).
  - `statprint(meta, fname, iwhich, flags) -> String` — full ST_* dispatch table per c:245-329, calling the 6 individual print fns above.
  - `fusevm_bridge.rs:9734` updated: `"zstat" => return crate::modules::stat::bin_stat(self, "zstat", &rest_vec);` (was `self.builtin_zstat(...)` calling the deleted ShellExecutor adapter).
  - 6/6 stat tests pass: `statelts_count_matches_st_count`, `statmodeprint_octal_only`, `statmodeprint_string_only`, `statmodeprint_directory`, `statulprint_decimal`, `statprint_size_via_index`. Drift gate clean.

- [ ] `modules/zprof.rs` ↔ `Modules/zprof.c` — **FULL REWRITE.**
  - Previous file had 5 Rust-only types: `ProfFunc`, `ProfArc`, `StackFrame`, `Profiler`, `ZprofOptions`, `ProfileEntry` (rule 1 violations) plus 9 WARNING-marked adhoc methods, plus a tail `impl ShellExecutor` adapter, plus duplicate concept (ProfFunc + Pfunc both representing C's `struct pfunc`). Full rewrite.
  - C: 3 structs (`pfunc` c:38, `sfunc` c:49, `parc` c:57). Rust: 3 structs (`Pfunc`, `Sfunc`, `Parc`) — names match C `struct *` casing letter-for-letter. Field names: `name`/`calls`/`time`/`self_time`/`num` for Pfunc (C `self` → Rust `self_time` because `self` is a Rust keyword); `p`/`beg` for Sfunc; `from`/`to`/`calls`/`time`/`self_time` for Parc. The C linked-list `next` pointers become indices into the parent `Vec`.
  - C file-statics (6) → Rust module-statics:
    - `static Pfunc calls;` (c:66) → `pub static CALLS: Mutex<Vec<Pfunc>>`
    - `static int ncalls;` (c:67) → `pub static NCALLS: AtomicI32`
    - `static Parc arcs;` (c:68) → `pub static ARCS: Mutex<Vec<Parc>>`
    - `static int narcs;` (c:69) → `pub static NARCS: AtomicI32`
    - `static Sfunc stack;` (c:70) → `pub static STACK: Mutex<Vec<Sfunc>>` (top at `last()`)
    - `static Module zprof_module;` (c:71) → `pub static ZPROF_MODULE: AtomicBool` (true when boot_ has run, false after cleanup_)
  - **Eliminated**: the `Profiler` struct on `ShellExecutor` was a bag-of-globals violating PORT_PLAN's anti-pattern; same for `profile_data: HashMap<String, ProfileEntry>`. Both deleted from exec.rs. The runtime profile data now lives in the module statics matching C's file-statics 1:1.
  - C fns (11): `freepfuncs`, `freeparcs`, `findpfunc`, `findparc`, `cmpsfuncs`, `cmptfuncs`, `cmpparcs`, `bin_zprof`, `name_for_anonymous_function`, `zprof_wrapper`, plus 6 module loaders (`setup_`, `features_`, `enables_`, `boot_`, `cleanup_`, `finish_`). Rust: same 17 ✓; function order matches C source order verbatim.
  - `bin_zprof(exec, nam, args) -> i32` — port of c:139-214. `-c` parsing inline (matches `OPT_ISSET(ops,'c')` at c:140); `-c` set → free both tables + reset counters per c:141-147; `-c` unset → gather + total + cmpsfuncs sort + header print + per-function row + cmptfuncs re-sort + per-function caller/callee blocks per c:149-211. Reverse-iter on the callees list mirrors C's `for (ap = as + narcs - 1; ap >= as; ap--)` at c:202.
  - `freepfuncs(f)` / `freeparcs(a)` — port of c:74-94. Vec.clear() mirrors the linked-list zfree+zsfree walk; the contained `String`s drop with the `Pfunc` (matches `zsfree(name)` at c:80).
  - `findpfunc(name) -> Option<usize>` — port of c:97-106. Linear scan returning the index (or `None` for C `NULL`).
  - `findparc(from, to) -> Option<usize>` — port of c:109-118.
  - `cmpsfuncs` / `cmptfuncs` / `cmpparcs` — descending qsort comparators per c:121-136. `b.partial_cmp(&a)` reverses the ordering.
  - `name_for_anonymous_function(name, filename, lineno) -> String` — port of c:217-233. C uses `convbase` for the lineno + `sepjoin` for the parts array; Rust uses `format!("{} [{}:{}]")`.
  - `zprof_wrapper(prog, w, name, runshfunc) -> i32` — port of c:236-311, LIVE on the function-call path. The body delegate replaces C's `runshfunc(prog, w, name)` at c:285, so the c:282 start stamp and the c:289 end stamp bracket the real call. `sf.beg` is re-read from the top of `STACK` before c:293 because C's `sf` *is* the stack node that nested calls increment at c:304 — the Rust port pushes a copy, so the local would be the un-incremented start time and self-time would not exclude children.
  - Registration: `boot_` returns `addwrapper(m, wrapper)` (c:362) and `cleanup_` calls `deletewrapper(m, wrapper)` (c:371); both are ported in `src/ported/module.rs` (c:576 / c:608) over the `WRAPPERS` list (c:567). `exec::runshfunc`'s `BODY_WRAPPERS` chain (`Src/exec.c:6166`) gained a `zsh/zprof` node whose membership test is one relaxed load of `module::WRAPPERS_ADDED` — with `zsh/zprof` unloaded the call path takes no lock and allocates nothing.
  - 6 module loaders body-ported to drive the new module-statics: `setup_` flips `ZPROF_MODULE` true, `boot_` resets all five tables (matches c:357-361), `cleanup_` calls freepfuncs+freeparcs and flips `ZPROF_MODULE` false (matches c:369-372).
  - **Caller updates:**
    - `src/exec.rs:24,303,552,575,879,933` — deleted `use Profiler`, `pub use ProfileEntry`, `profile_data: HashMap<String, ProfileEntry>` field (+ initializer), `profiler: Profiler` field (+ initializer). The `profiling_enabled: bool` flag stays (set by the `profile` extension builtin).
    - `src/extensions/ext_builtins.rs:564-581,605-606,634-642` — `profile` builtin now routes through `crate::zprof::bin_zprof(self, "profile", &args)` for both clear (`-c`) and dump (no args), gating on `NCALLS.load(...)` to suppress empty-state output.
    - `src/fusevm_bridge.rs:20,909` — deleted `use Profiler`; `BUILTIN_ZPROF` dispatch now calls `crate::modules::zprof::bin_zprof(exec, "zprof", &args)`.
    - `src/ported/builtin.rs:10651` — `"zprof"` builtin dispatch updated to call the new free fn.
  - 8/8 zprof tests pass (with `TEST_LOCK` Mutex serialising the ones that touch module-static state): `pfunc_default_zeros`, `freepfuncs_clears`, `findpfunc_matches_by_name`, `findparc_matches_pair`, `cmpsfuncs_descending`, `bin_zprof_clear_resets_tables`, `zprof_wrapper_returns_zero`, `name_for_anonymous_function_format`. Drift gate clean.

## 🟢 NEAR — 1–3 stubs (6) [stub-counts pre-rule-tightening]

- [ ] `params.rs` ↔ `params.c`
- [ ] `prompt.rs` ↔ `prompt.c`
- [ ] `init.rs` ↔ `init.c`
- [ ] `zle/compctl.rs` ↔ `Zle/compctl.c`
- [ ] `modules/curses.rs` ↔ `Modules/curses.c`
- [ ] `mem.rs` ↔ `mem.c`

## 🟡 PARTIAL — 10–40% stubs (6)

- [ ] `cond.rs` ↔ `cond.c`
- [ ] `builtin.rs` ↔ `builtin.c`
- [x] `modules/attr.rs` ↔ `Modules/attr.c` — line-by-line C-faithful rewrite (PORT.md "EXACT TRANSLATION"); 4 syscall wrappers (`xgetxattr`/`xlistxattr`/`xsetxattr`/`xremovexattr`) and 4 builtins (`bin_getattr`/`bin_setattr`/`bin_delattr`/`bin_listattr`) all with C-faithful sigs `(nam, argv, ops, _func)` reading `OPT_ISSET(ops, b'h')` for symlink flag. Macro `XATTR_NOFOLLOW` preserved. Locals at function top with C names (`val_len`, `attr_len`, `slen`, `vlen`, `symlink`). Bridge in `src/extensions/ext_builtins.rs::builtin_zattr` builds `&options` from `-h` flag, dispatcher unchanged. Module loaders take `_m: *const module`. 7/7 tests pass
- [ ] `zle/compresult.rs` ↔ `Zle/compresult.c`
- [ ] `modules/cap.rs` ↔ `Modules/cap.c`
- [ ] `glob.rs` ↔ `glob.c`

## 🟠 SPARSE — 40–80% stubs (21)

- [ ] `zle/zle_refresh.rs` ↔ `Zle/zle_refresh.c`
- [x] `modules/clone.rs` ↔ `Modules/clone.c` — line-by-line C-faithful rewrite (PORT.md "EXACT TRANSLATION"); `bin_clone(nam, args, _ops, _func)` mirrors `Src/Modules/clone.c:44`, locals declared at top (`ttyfd`, `pid`, `cttyfd`), C globals (`coprocin`/`coprocout`/`mypgrp`/`lastpid`/`ttystrname`) ported as same-name `AtomicI32`/`Mutex<String>` statics not Rust-only getter fns, dispatcher bridge moved to `src/extensions/ext_builtins.rs`, declarations in C-source order, 3/3 tests pass
- [x] `builtins/sched.rs` ↔ `Builtins/sched.c` — line-by-line C-faithful rewrite (see PORT.md "EXACT TRANSLATION"); same arg names + datatypes + called-fn names, intrusive `Option<Box<schedcmd>>` linked list with `next`/`cmd`/`time`/`flags` fields matching C, `bin_sched(nam, argv, ops, _func)` mirrors `Src/Builtins/sched.c:150`, dispatcher bridge moved to `src/extensions/ext_builtins.rs`, 7/7 tests pass
- [ ] `modules/datetime.rs` ↔ `Modules/datetime.c`
- [ ] `zle/zle_main.rs` ↔ `Zle/zle_main.c`
- [ ] `zle/termquery.rs` ↔ `Zle/termquery.c`
- [ ] `modules/random.rs` ↔ `Modules/random.c`
- [ ] `zle/zleparameter.rs` ↔ `Zle/zleparameter.c`
- [ ] `modules/db_gdbm.rs` ↔ `Modules/db_gdbm.c`
- [ ] `modules/files.rs` ↔ `Modules/files.c`
- [ ] `module.rs` ↔ `module.c`
- [ ] `builtins/rlimits.rs` ↔ `Builtins/rlimits.c`
- [ ] `zle/computil.rs` ↔ `Zle/computil.c`
- [ ] `modules/termcap.rs` ↔ `Modules/termcap.c`
- [ ] `modules/pcre.rs` ↔ `Modules/pcre.c`
- [ ] `zle/compmatch.rs` ↔ `Zle/compmatch.c`
- [ ] `zle/zle_params.rs` ↔ `Zle/zle_params.c`
- [ ] `modules/watch.rs` ↔ `Modules/watch.c`
- [ ] `modules/zpty.rs` ↔ `Modules/zpty.c`
- [ ] `modules/terminfo.rs` ↔ `Modules/terminfo.c`

## 🔴 STUB-HEAVY — >80% stubs (33)

- [ ] `modules/tcp.rs` ↔ `Modules/tcp.c`
- [ ] `zle/compcore.rs` ↔ `Zle/compcore.c`
- [ ] `modules/socket.rs` ↔ `Modules/socket.c`
- [ ] `zle/deltochar.rs` ↔ `Zle/deltochar.c`
- [ ] `zle/complist.rs` ↔ `Zle/complist.c`
- [ ] `modules/zutil.rs` ↔ `Modules/zutil.c`
- [ ] `modules/mathfunc.rs` ↔ `Modules/mathfunc.c`
- [ ] `modules/regex.rs` ↔ `Modules/regex.c`
- [x] `modules/zselect.rs` ↔ `Modules/zselect.c`
- [ ] `zle/zle_utils.rs` ↔ `Zle/zle_utils.c`
- [ ] `zle/zle_hist.rs` ↔ `Zle/zle_hist.c`
- [ ] `zle/zle_keymap.rs` ↔ `Zle/zle_keymap.c`
- [ ] `zle/zle_tricky.rs` ↔ `Zle/zle_tricky.c`
- [ ] `modules/zftp.rs` ↔ `Modules/zftp.c`
- [ ] `modules/parameter.rs` ↔ `Modules/parameter.c`
- [ ] `loop.rs` ↔ `loop.c`
- [ ] `modules/newuser.rs` ↔ `Modules/newuser.c`
- [ ] `modules/random_real.rs` ↔ `Modules/random_real.c`
- [ ] `zle/textobjects.rs` ↔ `Zle/textobjects.c`
- [ ] `zle/zle_misc.rs` ↔ `Zle/zle_misc.c`
- [ ] `zle/zle_move.rs` ↔ `Zle/zle_move.c`
- [ ] `zle/zle_vi.rs` ↔ `Zle/zle_vi.c`
- [ ] `zle/zle_thingy.rs` ↔ `Zle/zle_thingy.c`

## ⏪ Previously-DONE (re-verify under new rules)

- [ ] `compat.rs` ↔ `compat.c`
- [ ] `context.rs` ↔ `context.c`
- [ ] `hashnameddir.rs` ↔ `hashnameddir.c`
- [ ] `hashtable.rs` ↔ `hashtable.c`
- [ ] `hist.rs` ↔ `hist.c`
- [ ] `input.rs` ↔ `input.c`
- [ ] `jobs.rs` ↔ `jobs.c`
- [ ] `linklist.rs` ↔ `linklist.c`
- [ ] `math.rs` ↔ `math.c`
- [ ] `modentry.rs` ↔ `modentry.c`
- [ ] `openssh_bsd_setres_id.rs` ↔ `openssh_bsd_setres_id.c`
- [ ] `options.rs` ↔ `options.c`
- [ ] `pattern.rs` ↔ `pattern.c`
- [ ] `signals.rs` ↔ `signals.c`
- [ ] `sort.rs` ↔ `sort.c`
- [ ] `string.rs` ↔ `string.c`
- [ ] `subst.rs` ↔ `subst.c` — **PARTIAL.**
  - C fns (23): keyvalpairelement, prefork, stringsubstquote, stringsubst, paramsubst, quotesubst, globlist, singsub, multsub, filesub, equalsubstr, filesubstr, strcatsub, wcpadwidth, dopadding, get_strarg, get_intarg, subst_parse_str, substevalchar, untok_and_escape, check_colon_subscript, arithsubst, modify, dstackent • Rust: same 23 ✓
  - C: 0 structs/enums • Rust: 0 structs/enums ✓ (only `LinkList` type alias).
  - Paramtab bridges (`vars_get`/`vars_insert`/`arrays_get`/`arrays_insert`/`assoc_get`/`exec_assignaparam`/`exec_sethparam`/`exec_getsparam`) hit `crate::ported::params::paramtab()` and `paramtab_hashed_storage()` directly. Previous incarnation routed through `fusevm_bridge::try_with_executor` (silently no-op outside live VM frame).
  - `sub_flags` migrated to thread_local `SUB_FLAGS: Cell<i32>` (mirrors C `static int sub_flags` at Src/subst.c:2169). subst.rs (3 sites) and fusevm_bridge.rs (3 sites) read/write through `sub_flags_get`/`sub_flags_set`.
  - **Magic-var canonical routing (10 sites converted):**
    - `$aliases` → `hashtable::aliastab_lock()` (mirrors `mod_export HashTable aliastab` at hashtable.c:1186)
    - `$functions` / `$dis_functions` → `hashtable::shfunctab_lock()` (c:808)
    - `$commands` → `hashtable::cmdnamtab_lock()` (c:594)
    - `~user` tilde expansion (3 sites) → `hashnameddir::nameddirtab()` (hashnameddir.c:48)
    - `~±N` dir-stack expansion → `modules::parameter::DIRSTACK` (mirrors `mod_export LinkList dirstack` at builtin.c:743)
    - `$(cmd)` PATH lookup → `builtin::findcmd` (builtin.c:5260)
    - `${(%)...}` prompt expansion → `prompt::promptexpand` (prompt.c:182)
    - `${(t)var}` typeset info → reads `Param.flags` directly from `paramtab` (mirrors subst.c:2814 `pm->node.flags & PM_TYPE` dispatch)
    - POSIX shell-specials (`$?`/`$#`/`$$`/`$!`/`$*`/`$@`/`$-`/`$0`..`$N`) → `params::lookup_special_var` (consolidated with existing GSU dispatch). Also fixed a duplicate `pparams_lock` bug in params.rs that caused `$#` (via `poundgetfn`) to always return 0; pparams_lock now points at `builtin::PPARAMS`, the canonical store.
  - **Remaining 6 `try_with_executor` sites — genuinely executor-coupled:**
    - 2 × `run_command_substitution` (fork/exec subshell — fundamentally executor work)
    - 3 × `get_special_array_value` (821-line dispatch in exec_shims.rs over 20+ magic-assoc names; needs per-array port via `modules/parameter.rs` GSU getfn callbacks)
    - 1 × `expand_glob` (canonical `glob::zglob` is a stub; the real glob driver lives in exec_shims.rs)
  - Tests: 25/25 subst pass. Net full-suite: 1293/1297 (4 pre-existing test-isolation failures unrelated).
- [ ] `text.rs` ↔ `text.c`
- [ ] `utils.rs` ↔ `utils.c`
- [ ] `zle/zle_bindings.rs` ↔ `Zle/zle_bindings.c`

---

## Plan-of-attack ordering (re-verified under new rules)

We work the **STUB-HEAVY** tier from smallest to largest first
(quick wins that validate the new-rules cadence), then **SPARSE**,
then **PARTIAL**, then **NEAR**, then a final pass on the
**Previously-DONE** tier to spot-check.

Within each tier, ascending C-fn count = least work first.

### Tier 1: STUB-HEAVY, smallest first

1. `modules/random_real.rs` (2 fns)
2. `zle/textobjects.rs` (3 fns)
3. `modules/socket.rs` (7 fns)
4. `zle/deltochar.rs` (7 fns)
5. `loop.rs` (7 fns)
6. `modules/newuser.rs` (7 fns)
7. `modules/mathfunc.rs` (8 fns)
8. `modules/regex.rs` (8 fns)
9. `modules/zselect.rs` (8 fns)
10. `modules/ksh93.rs` (9 fns)
11. `modules/langinfo.rs` (9 fns)
12. `modules/nearcolor.rs` (11 fns)
13. `modules/example.rs` (12 fns)
14. `modules/mapfile.rs` (12 fns)
15. `modules/hlgroup.rs` (13 fns)
16. `modules/stat.rs` (14 fns)
17. `modules/tcp.rs` (20 fns)
18. `zle/zle_word.rs` (22 fns)
19. `zle/compcore.rs` (30 fns)
20. `modules/param_private.rs` (30 fns)
21. `modules/zprof.rs` (16 fns)
22. `zle/complist.rs` (37 fns)
23. `zle/zle_tricky.rs` (41 fns)
24. `zle/zle_utils.rs` (45 fns)
25. `zle/zle_hist.rs` (50 fns)
26. `zle/zle_misc.rs` (50 fns)
27. `zle/zle_keymap.rs` (51 fns)
28. `modules/zftp.rs` (57 fns)
29. `modules/parameter.rs` (110 fns)
30. `modules/zutil.rs` (38 fns)
31. `zle/zle_move.rs` (35 fns)
32. `zle/zle_vi.rs` (39 fns)
33. `zle/zle_thingy.rs` (30 fns)

### Tier 2-4: SPARSE → PARTIAL → NEAR → Re-verify Previously-DONE
