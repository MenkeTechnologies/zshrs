# Port Checklist — `src/ported/` 100% C Parity

Working list for the line-by-line port pass. Each file gets a single
checkbox; tick when the file's Rust port is verified function-by-function
against its C counterpart in `~/forkedRepos/zsh/Src/`.

**Methodology per file:**
1. Read every `pub fn` / `fn` in the Rust file.
2. For each one, locate the matching C function in the corresponding `.c` file (every Rust fn already carries a `/// Port of <c_name>() from Src/...:NNNN` citation per PORT.md).
3. Read the C body line-by-line.
4. Compare the Rust body — port any missing logic, fix divergences, replace stubs with real bodies.
5. Add stub-detection patterns to the file as we go (no more `-> i32 { 0 }` placeholders if a real port is reachable).
6. Run `cargo build --lib` after each file.
7. Commit when the file is fully ported.

**Stub form to eradicate:** `pub fn name() -> i32 { 0 }` — the name-parity placeholder.
**Acceptance per file:** zero stubs, every C fn body translated, drift gate clean.

---

## ✅ DONE — verified zero-stub (20)

These already passed the audit. We re-verify them at the end, not now.

- [ ] `compat.rs` ↔ `compat.c` (17 fns)
- [ ] `context.rs` ↔ `context.c` (4 fns)
- [ ] `hashnameddir.rs` ↔ `hashnameddir.c` (7 fns)
- [ ] `hashtable.rs` ↔ `hashtable.c` (49 fns)
- [ ] `hist.rs` ↔ `hist.c` (76 fns)
- [ ] `input.rs` ↔ `input.c` (18 fns)
- [ ] `jobs.rs` ↔ `jobs.c` (57 fns)
- [ ] `linklist.rs` ↔ `linklist.c` (19 fns)
- [ ] `math.rs` ↔ `math.c` (20 fns)
- [ ] `modentry.rs` ↔ `modentry.c` (1 fn)
- [ ] `openssh_bsd_setres_id.rs` ↔ `openssh_bsd_setres_id.c` (2 fns)
- [ ] `options.rs` ↔ `options.c` (17 fns)
- [ ] `pattern.rs` ↔ `pattern.c` (40 fns)
- [ ] `signals.rs` ↔ `signals.c` (27 fns)
- [ ] `sort.rs` ↔ `sort.c` (3 fns)
- [ ] `string.rs` ↔ `string.c` (12 fns)
- [ ] `subst.rs` ↔ `subst.c` (24 fns)
- [ ] `text.rs` ↔ `text.c` (16 fns)
- [ ] `utils.rs` ↔ `utils.c` (171 fns)
- [ ] `zle/zle_bindings.rs` ↔ `Zle/zle_bindings.c` (0 C fns — table-only file)

## 🟢 NEAR — 1–3 stubs (6)

- [ ] `params.rs` ↔ `params.c` (1 stub / 172 fns)
- [ ] `prompt.rs` ↔ `prompt.c` (1 stub / 32 fns)
- [ ] `init.rs` ↔ `init.c` (1 stub / 23 fns)
- [ ] `zle/compctl.rs` ↔ `Zle/compctl.c` (2 stubs / 46 fns)
- [ ] `modules/curses.rs` ↔ `Modules/curses.c` (3 stubs / 44 fns)
- [ ] `mem.rs` ↔ `mem.c` (2 stubs / 23 fns)

## 🟡 PARTIAL — 10–40% stubs (6)

- [ ] `cond.rs` ↔ `cond.c` (1 stub / 10 fns)
- [ ] `builtin.rs` ↔ `builtin.c` (14 stubs / 67 fns)
- [ ] `modules/attr.rs` ↔ `Modules/attr.c` (3 stubs / 14 fns)
- [ ] `zle/compresult.rs` ↔ `Zle/compresult.c` (8 stubs / 26 fns)
- [ ] `modules/cap.rs` ↔ `Modules/cap.c` (3 stubs / 9 fns)
- [ ] `glob.rs` ↔ `glob.c` (19 stubs / 52 fns)

## 🟠 SPARSE — 40–80% stubs (21)

- [ ] `zle/zle_refresh.rs` ↔ `Zle/zle_refresh.c` (14 stubs / 35 fns) — 40%
- [ ] `modules/clone.rs` ↔ `Modules/clone.c` (3 stubs / 7 fns) — 43%
- [ ] `modules/system.rs` ↔ `Modules/system.c` (9 stubs / 20 fns) — 45%
- [ ] `builtins/sched.rs` ↔ `Builtins/sched.c` (5 stubs / 11 fns) — 45%
- [ ] `modules/datetime.rs` ↔ `Modules/datetime.c` (6 stubs / 12 fns) — 50%
- [ ] `zle/zle_main.rs` ↔ `Zle/zle_main.c` (19 stubs / 37 fns) — 51%
- [ ] `zle/termquery.rs` ↔ `Zle/termquery.c` (14 stubs / 24 fns) — 58%
- [ ] `modules/random.rs` ↔ `Modules/random.c` (6 stubs / 10 fns) — 60%
- [ ] `zle/zleparameter.rs` ↔ `Zle/zleparameter.c` (6 stubs / 10 fns) — 60%
- [ ] `modules/db_gdbm.rs` ↔ `Modules/db_gdbm.c` (12 stubs / 19 fns) — 63%
- [ ] `modules/files.rs` ↔ `Modules/files.c` (17 stubs / 25 fns) — 68%
- [ ] `module.rs` ↔ `module.c` (60 stubs / 88 fns) — 68%
- [ ] `builtins/rlimits.rs` ↔ `Builtins/rlimits.c` (13 stubs / 19 fns) — 68%
- [ ] `zle/computil.rs` ↔ `Zle/computil.c` (47 stubs / 68 fns) — 69%
- [ ] `modules/termcap.rs` ↔ `Modules/termcap.c` (7 stubs / 10 fns) — 70%
- [ ] `modules/pcre.rs` ↔ `Modules/pcre.c` (10 stubs / 14 fns) — 71%
- [ ] `zle/compmatch.rs` ↔ `Zle/compmatch.c` (22 stubs / 30 fns) — 73%
- [ ] `zle/zle_params.rs` ↔ `Zle/zle_params.c` (47 stubs / 64 fns) — 73%
- [ ] `modules/watch.rs` ↔ `Modules/watch.c` (12 stubs / 16 fns) — 75%
- [ ] `modules/zpty.rs` ↔ `Modules/zpty.c` (15 stubs / 20 fns) — 75%
- [ ] `modules/terminfo.rs` ↔ `Modules/terminfo.c` (7 stubs / 9 fns) — 78%

## 🔴 STUB-HEAVY — >80% stubs (33)

- [ ] `modules/tcp.rs` ↔ `Modules/tcp.c` (16 stubs / 20 fns) — 80%
- [ ] `zle/compcore.rs` ↔ `Zle/compcore.c` (24 stubs / 30 fns) — 80%
- [ ] `modules/mapfile.rs` ↔ `Modules/mapfile.c` (10 stubs / 12 fns) — 83%
- [ ] `modules/socket.rs` ↔ `Modules/socket.c` (6 stubs / 7 fns) — 86%
- [ ] `modules/stat.rs` ↔ `Modules/stat.c` (12 stubs / 14 fns) — 86%
- [ ] `zle/deltochar.rs` ↔ `Zle/deltochar.c` (6 stubs / 7 fns) — 86%
- [ ] `zle/complist.rs` ↔ `Zle/complist.c` (32 stubs / 37 fns) — 86%
- [ ] `modules/zutil.rs` ↔ `Modules/zutil.c` (33 stubs / 38 fns) — 87%
- [ ] `modules/mathfunc.rs` ↔ `Modules/mathfunc.c` (7 stubs / 8 fns) — 88%
- [ ] `modules/regex.rs` ↔ `Modules/regex.c` (7 stubs / 8 fns) — 88%
- [ ] `modules/zselect.rs` ↔ `Modules/zselect.c` (7 stubs / 8 fns) — 88%
- [ ] `modules/ksh93.rs` ↔ `Modules/ksh93.c` (8 stubs / 9 fns) — 89%
- [ ] `modules/langinfo.rs` ↔ `Modules/langinfo.c` (8 stubs / 9 fns) — 89%
- [ ] `zle/zle_utils.rs` ↔ `Zle/zle_utils.c` (40 stubs / 45 fns) — 89%
- [ ] `zle/zle_hist.rs` ↔ `Zle/zle_hist.c` (45 stubs / 50 fns) — 90%
- [ ] `zle/zle_keymap.rs` ↔ `Zle/zle_keymap.c` (46 stubs / 51 fns) — 90%
- [ ] `zle/zle_tricky.rs` ↔ `Zle/zle_tricky.c` (37 stubs / 41 fns) — 90%
- [ ] `modules/nearcolor.rs` ↔ `Modules/nearcolor.c` (10 stubs / 11 fns) — 91%
- [ ] `zle/zle_word.rs` ↔ `Zle/zle_word.c` (20 stubs / 22 fns) — 91%
- [ ] `modules/hlgroup.rs` ↔ `Modules/hlgroup.c` (12 stubs / 13 fns) — 92%
- [ ] `modules/param_private.rs` ↔ `Modules/param_private.c` (28 stubs / 30 fns) — 93%
- [ ] `modules/zprof.rs` ↔ `Modules/zprof.c` (15 stubs / 16 fns) — 94%
- [ ] `modules/zftp.rs` ↔ `Modules/zftp.c` (55 stubs / 57 fns) — 96%
- [ ] `modules/parameter.rs` ↔ `Modules/parameter.c` (109 stubs / 110 fns) — 99%
- [ ] `loop.rs` ↔ `loop.c` (7 stubs / 7 fns) — 100%
- [ ] `modules/example.rs` ↔ `Modules/example.c` (12 stubs / 12 fns) — 100%
- [ ] `modules/newuser.rs` ↔ `Modules/newuser.c` (7 stubs / 7 fns) — 100%
- [ ] `modules/random_real.rs` ↔ `Modules/random_real.c` (2 stubs / 2 fns) — 100%
- [ ] `zle/textobjects.rs` ↔ `Zle/textobjects.c` (3 stubs / 3 fns) — 100%
- [ ] `zle/zle_misc.rs` ↔ `Zle/zle_misc.c` (50 stubs / 50 fns) — 100%
- [ ] `zle/zle_move.rs` ↔ `Zle/zle_move.c` (35 stubs / 35 fns) — 100%
- [ ] `zle/zle_vi.rs` ↔ `Zle/zle_vi.c` (39 stubs / 39 fns) — 100%
- [ ] `zle/zle_thingy.rs` ↔ `Zle/zle_thingy.c` (32 stubs / 30 fns) — 100%

---

## Plan-of-attack ordering

We work the **STUB-HEAVY** tier from smallest to largest first (quick wins
to validate the porting cadence), then **SPARSE**, then **PARTIAL**, then
**NEAR**, then a final pass on the **DONE** tier to spot-check.

Within each tier, ascending C-fn count = least work first. Big files
(`modules/parameter.rs`, `modules/zftp.rs`, `module.rs`) get tackled
last in their tier.

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

### Tier 2: SPARSE
Same ordering principle (small-to-large by C-fn count).

### Tier 3: PARTIAL → NEAR → DONE re-verify

---

## Per-file workflow template

When we start a file, we add a sub-checklist of every stub to replace:

```
- [ ] modules/example.rs ↔ Modules/example.c
  - [ ] `boot_(...)` — port body from example.c:NNN
  - [ ] `cleanup_(...)` — port body from example.c:NNN
  - [ ] ...
```

After every stub is filled, build clean, drift gate clean, commit.
