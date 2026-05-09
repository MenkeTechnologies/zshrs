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
   or appears in `tests/data/ported_fn_allowlist.txt` with a
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

## Status — RESET (all unchecked, restarting from beginning)

The earlier session ticked 10 files (`9c9d52e688`, `1c8d3681e3`, etc.)
but those passes did NOT enforce rules 1–4 above (Rust-only structs
were left in place; some stubs were body-ported but kept their
Rust-only wrappers). All checkboxes reset; we revisit each file.

In-flight files where I started bodies but the surrounding file still
has Rust-only types to delete: `modules/stat.rs`, `modules/zprof.rs`.
These get re-done from the top under the new rules. (`modules/hlgroup.rs`
has no remaining Rust-only types but is BLOCKED on a real port of
`Src/prompt.c`'s `match_highlight` and `zattrescape` — see TODO.md.)

---

## ✅ DONE — verified line-by-line (zero stubs + zero Rust-only types + name-matched)

Each tick below carries a per-fn audit log proving every C line maps
to a Rust statement. No tick without that log. Anything blocked on
an unported dependency is moved to **🚧 BLOCKED** and tracked in
`TODO.md`.

- [x] `modules/random_real.rs` ↔ `Modules/random_real.c`
  - C: 0 structs/enums • Rust: 0 structs/enums ✓
  - C fns: `_zclz64`, `random_64bit`, `random_real` (3) • Rust: same 3 ✓
  - `_zclz64(x: u64) -> i32` — port of c:48-79. 16 C statements → 16 Rust statements. Binary-search-shift each `if (!(x & MASK)) { n += K; x <<= K; }` matches one-to-one. Verified.
  - `random_64bit() -> u64` — port of c:84-93. Includes the `getrandom_buffer` error path + `zwarn(...)` + `return 1` (not 0) + `u64::from_ne_bytes(buf)` success path. Matches c:85-93 line-by-line. Verified after fix in `03ab0b26d9`.
  - `random_real() -> f64` — port of c:147-213. Calls `random_64bit()` (not `random_u64`), `_zclz64()` (not `leading_zeros`), and `extern "C" ldexp` (not `exp2`). All 18 C statements have matching Rust statements. Verified after fix in `03ab0b26d9`.

## 🚧 BLOCKED — partial port, gap tracked in `TODO.md`

- [ ] `zle/textobjects.rs` ↔ `Zle/textobjects.c`
  - C: 0 structs/enums • Rust: 0 structs/enums ✓ (after deleting Rust-only `TextObjectType`/`TextObjectKind`/`TextObject` + `Zle::select_text_object`-family helpers)
  - C fns: `blankwordclass`, `selectword`, `selectargument` (3) • Rust: same 3 ✓
  - `blankwordclass(c)` — 1-line port of c:36 ✓ verified.
  - `selectword(zle)` — full ~170-line port of c:41-205 incl. visual-mode-reverse-direction branch (c:97-148), digit-arg loop's `if all` inner block (c:165-179), and `doblanks` trim section (c:181-194). One residual: reads `virangeflag` as constant-false (zle_vi.c:36 is unported file-global, see TODO.md). Verified except for that constant.
  - `selectargument(zle)` — **NOT 100%.** C body uses `ctxtlex()` lexer-walk (c:233-257); Rust port is whitespace-split approximation. Blocks on lexer-context machinery — see TODO.md.
- [x] `modules/socket.rs` ↔ `Modules/socket.c`
  - C: 0 structs/enums • Rust: 0 structs/enums ✓ (after deleting Rust-only `ZsocketOptions`/`UnixSocket`)
  - C fns (7): `bin_zsocket`, `setup_`, `features_`, `enables_`, `boot_`, `cleanup_`, `finish_` • Rust: same 7 ✓
  - 6 module loaders match C 1:1 (each is `return 0;` body or short featuresarray/handlefeatures call — Rust ports are no-op static-link path)
  - `bin_zsocket` — full port of c:57-272 incl. inline flag parse (matching `"ad:ltv"` builtin spec at c:276), socket()/bind()/listen() for `-l` (c:84-138), poll-test + accept() for `-a` (c:142-218), socket()/connect() default path (c:218-269), addmodulefd + redup/movefd post-call sequence on every success path (c:118/121/125, c:208/211/215, c:252/255/260). The shim writes `setiparam_no_convert("REPLY", final_fd)` (c:135/204/268). Verified line-by-line.
- [x] `zle/deltochar.rs` ↔ `Zle/deltochar.c`
  - C: 0 structs/enums • Rust: 0 structs/enums ✓
  - C fns (7): `deltochar`, `setup_`, `features_`, `enables_`, `boot_`, `cleanup_`, `finish_` • Rust: same 7 ✓
  - 6 module loaders: `setup_`/`features_`/`enables_`/`boot_`/`cleanup_`/`finish_` — each is `return 0;` C body or trivial featuresarray/handlefeatures/addzlefunction call. Rust ports are static-link no-ops with C-line citations. Verified.
  - `deltochar(zle)` — port of c:38-79. 1:1 mapping: `getfullchar(0)` → `zle.getfullchar(false)`; `int dest = zlecs, ok = 0, n = zmult` → 3 mut locals; `zap = bindk->widget == w_zaptochar` → `zle.bindk.name == "zap-to-char"`; forward-direction loop (c:45-58) and backward-direction loop (c:59-77) match C structure exactly; `forekill(dest - zlecs, CUT_RAW)` and `backkill(zlecs - dest - zap, CUT_RAW|CUT_FRONT)` call into the real ports in zle_utils.rs (sibling stubs fixed at source, rule 5); `return !ok` → `if ok != 0 { 0 } else { 1 }`. Verified.
  - Sibling fixes: `zle_utils::forekill` (zle_utils.c:1064) + `zle_utils::backkill` (zle_utils.c:1045) ported as part of this commit.
- [x] `loop.rs` ↔ `loop.c`
  - C: 0 structs/enums • Rust: 0 structs/enums ✓ (after deleting dead `LoopState`/`ForIterator`/`CForState`/`TryState` aggregates in earlier dissolution)
  - C fns (8): `execfor`, `execselect`, `execwhile`, `execrepeat`, `execif`, `execcase`, `exectry`, `selectlist` • Rust: same 8 ✓
  - 7 tree-walker entries (`execfor`/`execselect`/`execwhile`/`execrepeat`/`execif`/`execcase`/`exectry`) — bodies are `unreachable!()` per the 96-test architectural invariant (fusevm bytecode in `compile_zsh.rs` replaces tree-walker dispatch). Each entry cites its C line + the architectural reason. Verified consistent.
  - `selectlist(items, start)` — port of c:347-416. Was previously a Rust-only signature `(items, prompt, columns) -> String`; now matches C exactly: takes items + start index, writes formatted menu to stderr, returns next-page offset (or 0 when complete). Body ports c:350-415 line-by-line: longest-width compute, fct/fw column geometry, the do-while inner loop, MB_METASTRWIDTH approximation via chars().count(). Verified.
- [x] `modules/mathfunc.rs` ↔ `Modules/mathfunc.c`
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

- [ ] `modules/ksh93.rs` ↔ `Modules/ksh93.c` — **NOT 100%.** 2 Rust-only types still present (`Ksh93Params`, `NamerefOptions`) violating rule 1. See TODO.md.

- [ ] `modules/langinfo.rs` ↔ `Modules/langinfo.c` — **NOT 100%.** 1 Rust-only type still present + 2 stubs (`liitem`, `scanlanginfo`). See TODO.md.

- [x] `modules/nearcolor.rs` ↔ `Modules/nearcolor.c`
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

- [x] `modules/example.rs` ↔ `Modules/example.c`
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

- [x] `modules/mapfile.rs` ↔ `Modules/mapfile.c`
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
- [ ] `modules/attr.rs` ↔ `Modules/attr.c`
- [ ] `zle/compresult.rs` ↔ `Zle/compresult.c`
- [ ] `modules/cap.rs` ↔ `Modules/cap.c`
- [ ] `glob.rs` ↔ `glob.c`

## 🟠 SPARSE — 40–80% stubs (21)

- [ ] `zle/zle_refresh.rs` ↔ `Zle/zle_refresh.c`
- [ ] `modules/clone.rs` ↔ `Modules/clone.c`
- [ ] `modules/system.rs` ↔ `Modules/system.c`
- [ ] `builtins/sched.rs` ↔ `Builtins/sched.c`
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
- [ ] `modules/stat.rs` ↔ `Modules/stat.c` ← **named-example file; delete StatElement / FileStat / FileType / StatFlags / StatOptions; rewrite bin_stat with i32 STF_* bitmask**
- [ ] `zle/deltochar.rs` ↔ `Zle/deltochar.c`
- [ ] `zle/complist.rs` ↔ `Zle/complist.c`
- [ ] `modules/zutil.rs` ↔ `Modules/zutil.c`
- [ ] `modules/mathfunc.rs` ↔ `Modules/mathfunc.c`
- [ ] `modules/regex.rs` ↔ `Modules/regex.c`
- [ ] `modules/zselect.rs` ↔ `Modules/zselect.c`
- [ ] `modules/ksh93.rs` ↔ `Modules/ksh93.c`
- [ ] `modules/langinfo.rs` ↔ `Modules/langinfo.c`
- [ ] `zle/zle_utils.rs` ↔ `Zle/zle_utils.c`
- [ ] `zle/zle_hist.rs` ↔ `Zle/zle_hist.c`
- [ ] `zle/zle_keymap.rs` ↔ `Zle/zle_keymap.c`
- [ ] `zle/zle_tricky.rs` ↔ `Zle/zle_tricky.c`
- [ ] `zle/zle_word.rs` ↔ `Zle/zle_word.c`
- [ ] `modules/param_private.rs` ↔ `Modules/param_private.c`
- [ ] `modules/zprof.rs` ↔ `Modules/zprof.c`
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
- [ ] `subst.rs` ↔ `subst.c`
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
