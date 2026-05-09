# Port Checklist — `src/ported/` 100% C Parity

Working list for the line-by-line port pass. Each file gets a single
checkbox; tick when the file's Rust port is verified function-by-function
**AND** struct-by-struct against its C counterpart in `~/forkedRepos/zsh/Src/`.

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
has Rust-only types to delete: `modules/stat.rs`, `modules/nearcolor.rs`,
`modules/example.rs`, `modules/mapfile.rs`, `modules/hlgroup.rs`,
`modules/zprof.rs`. These get re-done from the top under the new rules.

---

## ✅ DONE — verified (zero stubs + zero Rust-only types + name-matched)

- [x] `modules/random_real.rs` ↔ `Modules/random_real.c` — 0 structs/enums (matches C); 3 fns (`random_real`, `_zclz64`, `random_64bit`) all body-ported with C-line citations.
- [x] `zle/textobjects.rs` ↔ `Zle/textobjects.c` — deleted Rust-only `TextObjectType`/`TextObjectKind`/`TextObject` enums+struct + Rust-only `Zle::select_text_object`/`select_word_object`/`select_sentence_object`/`select_paragraph_object`/`select_pair_object`/`select_quote_object` impl block. Now: 0 structs/enums, 3 free fns (`blankwordclass`, `selectword`, `selectargument`) matching C 1:1.
- [x] `modules/socket.rs` ↔ `Modules/socket.c` — deleted Rust-only `ZsocketOptions` struct + `UnixSocket` struct (incl. `UnixSocket::new`). `bin_zsocket(args, options)` collapsed to `bin_zsocket(args)` with inline `-a`/`-d`/`-l`/`-t`/`-v` flag parsing matching the C builtin spec `"ad:ltv"` (socket.c:276). Now: 0 structs/enums, 7 fns matching C 1:1.
- [x] `zle/deltochar.rs` ↔ `Zle/deltochar.c` — deleted Rust-only signature `deltochar(buffer, cursor, target, direction, inclusive) -> Option<(usize,usize)>` and replaced with C-faithful `deltochar(zle: &mut Zle) -> i32` that ports the C body line-by-line (deltochar.c:38-79) including `getfullchar` lookahead, `zmult`-driven repeat loop, and forekill/backkill dispatch. Also fixed broken stubs `forekill` and `backkill` in `zle_utils.rs` at source (rule 5) — now real ports of zle_utils.c:1064 and :1045 over `&mut Zle`. Now: 0 structs/enums in deltochar.rs, 7 fns matching C 1:1.
- [x] `loop.rs` ↔ `loop.c` — already compliant from prior dissolution pass: 0 structs/enums (deleted dead `LoopState`/`ForIterator`/`CForState`/`TryState` in earlier work), 7 tree-walker entries (`execfor`/`execselect`/`execwhile`/`execrepeat`/`execif`/`execcase`/`exectry`) match C names. Bodies are `unreachable!()` per the 96-test architectural invariant (fusevm bytecode in `compile_zsh.rs` replaces tree-walker dispatch).
- [x] `modules/newuser.rs` ↔ `Modules/newuser.c` — already compliant from prior pass: 0 structs/enums, 7 fns matching C names (setup_/features_/enables_/boot_/cleanup_/finish_/check_dotfile), all bodies cited.

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
- [ ] `modules/mapfile.rs` ↔ `Modules/mapfile.c`
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
- [ ] `modules/nearcolor.rs` ↔ `Modules/nearcolor.c`
- [ ] `zle/zle_word.rs` ↔ `Zle/zle_word.c`
- [ ] `modules/hlgroup.rs` ↔ `Modules/hlgroup.c`
- [ ] `modules/param_private.rs` ↔ `Modules/param_private.c`
- [ ] `modules/zprof.rs` ↔ `Modules/zprof.c`
- [ ] `modules/zftp.rs` ↔ `Modules/zftp.c`
- [ ] `modules/parameter.rs` ↔ `Modules/parameter.c`
- [ ] `loop.rs` ↔ `loop.c`
- [ ] `modules/example.rs` ↔ `Modules/example.c`
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
