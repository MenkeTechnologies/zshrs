# FAKE_FNS_CHECKLIST — stub triage for src/ported/

Derived from `docs/audits/PORT_STUBS.md` (119 ratio-flagged fns, 44 files),
then each fn read against its C counterpart in
`/Users/wizard/forkedRepos/zsh/Src/` and classified by hand.

Three dispositions:

- **PORT** — the C body does real algorithmic work that the Rust body
  fakes (heuristic / early return / faked success). Port the full C body
  faithfully (line-by-line, `// c:NNN` citations). These are the
  dangerous fakes: they inflate the "ported" count and lie about success.
- **UNFAKE** — C work is dead/out-of-scope in zshrs (dlsym/dlopen in a
  static-link build, `.zwc` wordcode emit, faked signature). Replace the
  faked-success body with honest `unimplemented!()` (keeps the name-parity
  anchor per the no-delete-shims rule) — never leave it returning a fake 0.
- **NOT FAKE** — ratio false-positive: legitimate Rust idiom (native
  alloc / HashMap / Drop / `&str` UTF-8) or an already-faithful port the
  detector mismeasured. Listed at the bottom so they are not re-triaged.

---

## PORT — real C work faked (priority order: compat floor first)

### Completion result/list engine (highest blast radius — ZLE completion)
- [ ] `zle/compresult.rs::do_single` — C compresult.c:963 (180) — replaces minfo/brace/AUTO_REMOVE_SLASH logic with a 1-line `instmatch` delegate
- [ ] `zle/compresult.rs::cline_str` — C compresult.c:165 (271) — missing `ins=1` buffer-edit mode, brace insertion, posl population
- [ ] `zle/compresult.rs::instmatch` — C compresult.c:578 (99) — ipre/pre/ppre/str insertion + brace position (brpl) tracking missing
- [ ] `zle/compresult.rs::cut_cline` — C compresult.c:46 (110) — Rust sig changed to trivial truncation; Cline-chain restructuring gone
- [ ] `zle/compresult.rs::instmatch`/`do_allmatches` — C compresult.c:895 (49) — `matches.join(sep)` vs LinkList walk + flag branching
- [ ] `zle/compresult.rs::hasbrpsfx` — C compresult.c:683 (39) — `contains('{')` heuristic vs metafy_line/instmatch/lastpre-postbr compare
- [ ] `zle/compresult.rs::valid_match` — C compresult.c:1206 (32) — starts/ends_with predicate vs minfo/amatches chain walk w/ CMF_DUMMY/MULT
- [ ] `zle/compresult.rs::build_pos_string` — C compresult.c:489 (23) — `format!("{}/{}")` vs LinkList colon-string build
- [ ] `zle/complist.rs::domenuselect` — C complist.c:2383 (940) — interactive menu loop (keys, incremental search, refresh) stubbed to var decls + guards
- [ ] `zle/complist.rs::clnicezputs` — C complist.c:715 (107) — color/width/right-align formatting reduced to demeta-only
- [ ] `zle/compmatch.rs::bld_line` — C compmatch.c:1734 (139) — ~25% (CPAT walk); brace nesting + accent-equivalence + cross-class missing

### Completion core / compctl
- [ ] `zle/compcore.rs::set_comp_sep` — C compcore.c:1459 (321) — lexer replay / token narrowing / qp-qs split / wb-we-offs update stubbed
- [ ] `zle/compcore.rs::callcompfunc` — C compcore.c:543 (345) — shfunc dispatch / arg build / lex wrap / post-return only ~50 lines of ~391
- [ ] `zle/compctl.rs::makecomplistflags` — C compctl.c:3043 (745) — ~15% of CC_* flag matrix (files/dirs/named/func/ylist/str); rest missing
- [ ] `zle/compctl.rs::makecomplistext` — C compctl.c:2647 (150) — numeric-range only; evalcompcond string/pattern path missing
- [ ] `zle/compctl.rs::makecomplistctl` — C compctl.c:2313 (74) — recursion bookkeeping + incompfunc state missing
- [ ] `zle/compctl.rs::addmatch` — C compctl.c:1926 (103) — ADDWHAT heuristic vs file-test/path-build/CC_* filtering
- [ ] `zle/complete.rs::bin_compadd` — C complete.c:611 (233) — flag parse + Cadata build + match insertion (~70 of 252 lines)
- [ ] `zle/computil.rs::cfp_add_sdirs` — C computil.c:4763 (73) — GLOBDOTS / COMPPREFIX path handling missing

### ZLE refresh / input / search engines
- [ ] `zle/zle_refresh.rs::zrefresh` — C zle_refresh.c:975 (658) — screen-refresh engine, 13% present
- [ ] `zle/zle_refresh.rs::tc_rightcurs` — C zle_refresh.c:2237 (63) — termcap cursor-right movement choices
- [ ] `zle/zle_refresh.rs::moveto` — C zle_refresh.c:2163 (44) — cursor move w/ line-wrap + coord handling
- [ ] `zle/zle_hist.rs::doisearch` — C zle_hist.c:1083 (462) — incremental-search engine; 26-line stub, no UI loop
- [ ] `zle/zle_hist.rs::getvisrchstr` — C zle_hist.c:1815 (118) — minibuffer search-string read w/ keymap switch; snapshots buffer only
- [~] `zle/zle_main.rs::getbyte` — C zle_main.c:861 (76) — core done (raw_getbyte + \n/\r swap + LASTCHAR); remaining: EINTR retry loop + EIO device-reattach edge cases. **[ZLE engine Phase 1]**
- [x] `zle/zle_main.rs::raw_getbyte` — C zle_main.c:506 (242) — **PORTED [ZLE engine Phase 1]**: replaced the busy-wait sleep loop with a real `poll(2)` over SHTTY + the `zle -F` watched fds (c:532-589), dispatching watch handlers (widget → `zlecallhook`, function → `callhookfunc` with fd + err/hup/nval flags, c:715-772). Substrate (`WATCH_FDS`/`watch_fd`/`zlecallhook`/`callhookfunc`) already existed — wired it. 93 input tests green (raw_getbyte/getbyte/getfullchar/watch).
- [ ] `zle/zle_main.rs::zleread` — C zle_main.c:1216 (127+) — skeleton; undo/history/hooks/prompt setup missing
- [ ] `zle/zle_utils.rs::showmsg` — C zle_utils.c:1310 (72) — multibyte width-aware message display
- [ ] `zle/zle_utils.rs::spaceinline` — C zle_utils.c:784 (54) — buffer insertion w/ region-highlight adjustment
- [ ] `zle/zle_move.rs::backwardmetafiedchar` — C zle_move.c:170 (75) — UTF-8/Meta backward scan w/ combining chars
- [x] `zle/zle_vi.rs::vireplacechars` — C zle_vi.c:594 (62) — **PORTED** (was faking the key read with `LASTCHAR` and dropping the region/visual path; now reads via `vigetkey`, handles char/line region selection, the `<return>`→single-newline special case, and `shiftchars`/`spaceinline` width fixup; all deps verified present; 77 zle_vi tests green)
- [ ] `zle/termquery.rs::probe_terminal` — C termquery.c:200 (208) — response-matching state machine / feature extraction; reads raw bytes only
- [ ] `zle/zle_main.rs::describekeybriefly` — C zle_main.c:1892 (28) — delegates to helper + faked 0

### Core shell
- [x] `utils.rs::lchdir` — C utils.c:7400 (161) — symlink-attack detect (lstat pre/post), dev/inode checks, dirfd fallback, hard/soft modes — **PORTED** (was 9-line `set_current_dir` fake; now full per-component lstat→chdir→re-lstat dev/ino integrity descent + restoredir + HOME/`/` fallback; signature restored to `(path, d, hard) -> i32`; 3 callers in builtin.rs/glob.rs updated, cd_get_dest return-value bug fixed to return `buf` per c:1181)
- [ ] `utils.rs::checkmailpath` — C utils.c:1621 (89) — dir recursion / S_ISDIR / parsestr+singsub param subst missing
- [x] `utils.rs::findsep` — C utils.c:3784 (52) — **PORTED** (was `str::find` on `is_ascii_whitespace`, dropped `quote` + multibyte + ISEP semantics; now faithful `(s: &mut String, pos, sep, quote) -> i32`: ISEP/`$IFS` split, in-place backslash stripping for `\<sep>` and `\\`→`\`, empty-sep single-char advance, multi-byte literal sep; 5 unit tests added; no callers — kept as faithful name anchor since `splitstring`/`findword`/`wordcount` inline the logic)
- [ ] `glob.rs::zglob` — C glob.c:1213 (614) — glob-qualifier parse (c:1240-2012) deferred; ~35-line `glob_path` delegate
- [ ] `glob.rs::scanner` — C glob.c:500 (162) — chdir+opendir tree walk simplified to `fs::read_dir`; lchdir diagnostics gone
- [ ] `hist.rs::savehistfile` — C hist.c:2922 (199) — write modes / ownership / atomic rename / HISTORY_IGNORE / backslash escaping
- [ ] `hist.rs::lockhistfile` — C hist.c:3182 (124) — symlink/link/open retry-loop locking simplified to fs2 try_lock
- [ ] `init.rs::source` — C init.c:1584 (97) — state save/restore present; parse-exec + .zwc load deferred to executor
- [ ] `signals.rs::wait_for_processes` — C signals.c:249 (102) — per-PID job-table update / cmdoutpid / execstack walk deferred
- [ ] `builtin.rs::cd_new_pwd` — C builtin.c:1188 (71) — dirstack ops (rolllist/remnode) / symlink resolve / dup detect deferred to caller
- [ ] `prompt.rs::promptexpand` — C prompt.c:182 (43) — init_term / PROMPTSUBST / putpromptchar loop / Inpar-Outpar strip in helper
- [ ] `prompt.rs::match_highlight` — C prompt.c:2031 (75) — hl=/fg=/bg=/layer=/opacity= attribute loop parser missing
- [ ] `compat.rs::zgetdir` — C compat.c:355 (146) — getcwd + walk-based fallback (stat/opendir) reduced to current_dir
- [ ] `exec.rs::execpline` — C exec.c:1724 (237) — pipe/fork/job-table primitives deferred (structural stub)
- [ ] `parse.rs::par_subsh` — C parse.c:1619 (34) — structure parsed; wordcode emit deferred
- [ ] `parse.rs::load_dump_file` — C parse.c:3675 (47) — mmap + page-align lost; plain read_to_end
- [ ] `hashtable.rs::addhistnode` — C hashtable.c:1427 (17) — HIST_MAKEUNIQUE/FOREIGN dup-flag state machine reduced to plain insert

### Modules
- [ ] `modules/curses.rs::zccmd_input` — C Modules/curses.c:1082 (173) — wgetch loop / EINTR / mouse decode stubbed
- [ ] `modules/zutil.rs::setstypat` — C Modules/zutil.c:295 (58) — weight-scoring present; parse_string eval-body parse (c:304-318) skipped
- [ ] `modules/db_gdbm.rs::gdbmhashsetfn` — C Modules/db_gdbm.c:464 (47) — gdbm_reorganize (c:489) skipped, else faithful
- [ ] `modules/db_gdbm.rs::unmetafy_zalloc` — C Modules/db_gdbm.c:760 (11) — omits zalloc buffer + memcpy; returns heap String

---

## UNFAKE — (empty after verification)

The 4 candidates the audit first flagged here were each read and found
**not** to fake success — nothing to delete:

- `module.rs::load_and_bind` (L3051) / `module.rs::module_func` (L3400) —
  return NULL on the **dlopen/dlsym path**, correct in a static-link build;
  real work routes through `setup_module → m->u.linked->setup`. Name anchors.
- `parse.rs::build_cur_dump` (L4495) — already honest: emits
  `zwarnnam(... "wordcode dump-current emit not yet ported")` + `return 1`.
- `utils.rs::read_poll` (L3005) — does real `libc::poll`; signature narrowed
  (drops C's char-read peek) but it's an honest `read -t` poll wrapper.

---

## NOT FAKE — ratio false-positives (excluded; do not re-triage)

Native-alloc / arena (no-GC idiom): `mem.rs` zhalloc, freeheap, popheap,
hrealloc, pushheap, old_heaps, zalloc, memory_validate, malloc, realloc
(last two: thin libc shims, zero Rust callers — keep as name anchors).

HashMap/OnceLock table creators + Drop teardown: `hashtable.rs`
newhashtable, createreswdtable, createcmdnamtable, createshfunctable,
createhisttable, expandhashtable, resizehashtable, freeshfuncnode,
printhashtabinfo; `hashnameddir.rs` createnameddirtable; `options.rs`
createoptiontable; `compctl.rs` createcompctltable, freecompcond;
`compcore.rs` freematches; `complete.rs` set_compstate; `zle_keymap.rs`
newkeytab, emptykeymapnamtab, createkeymapnamtab; `zle_thingy.rs`
createthingytab; `zle_params.rs` get_cursor, set_killring; `zle_utils.rs`
sizeline, zle_free_positions; `zle_refresh.rs` freevideo; `modules/zutil.rs`
newzstyletable, freestylenode.

`&str`-UTF-8 / unicode_width idiom: `utils.rs` metafy, unmeta,
mb_metacharlenconv_r, mb_metastrlenend, set_widearray, delprepromptfn,
timespec_diff_us; `pattern.rs` charsub.

Module-lifecycle name anchors (documented KEEP): `module.rs`
try_load_module; `modules/param_private.rs` setup_; `modules/pcre.rs`
pcre_callout; `deltochar.rs` boot_; `modules/zpty.rs` ptysettyinfo.

Daily-driver subset / documented divergence: `zle_main.rs` zsetterm;
`zle_vi.rs` getvirange; `init.rs` init_shout; `builtins/rlimits.rs` printrlim.

Already-faithful ports the detector mismeasured (C span overcounted):
`exec.rs` namedpipe (C exec.c:5057, real body ~18 lines);
`compat.rs` zgettime_monotonic_if_available (C compat.c:133, ~33 lines);
`modules/parameter.rs` setfunctions (complete);
`modules/watch.rs` readwtab (complete);
`hashtable.rs` hashdir (real opendir/readdir work lives in
`cmdnam_table::hash_dir`, the L1017 fn is pure delegation).

---

## Substrate triage of the remaining PORT items (for "port all 50")

Parallel read-only triage of every remaining PORT item against existing
Rust substrate. "Port all 50" is gated on substrate — most fakes exist
*because* their engine isn't built. Faking them anyway = the forbidden
structural-shell capital offense. Buckets:

**READY** — verified one-by-one. The triage's "READY" bucket proved
largely illusory: on close inspection most have hidden substrate gaps
(agent labels were wrong 4/4 on the delete-candidates and on
`addhistnode`/`spaceinline`/`bld_line`). Confirmed status:
- [x] `zle_vi.rs::vireplacechars` — **done** (vigetkey/region path)
- [x] `modules/db_gdbm.rs::gdbmhashsetfn` — **done** (closed the
  `gdbm_reorganize` gap c:501; binding existed, added a `reorganize()`
  wrapper + `if ht.is_empty()` call; 64 gdbm tests green)
- [x] `zle_utils.rs::spaceinline` — **substrate built + ported**. Added
  the `flags: i32` field to `RegionHighlight` (the `P`-prefix
  `ZRH_PREDISPLAY` flag was being parsed at zle_refresh.rs:3070 then
  discarded via `let _ = flags` — now stored). `predisplaylen` derived
  from `get_predisplay().chars().count()`. Ported the missing non-meta
  region-highlight adjustment (c:830-844) + `viinsbegin` reset
  (c:827-828): user regions past the cursor shift by `ct` on insert.
  New test pins the shift; spaceinline (4) + zle_utils (79) green. (The
  completion-only meta branch — `start_meta`/`end_meta` with
  `zlemetaline` — stays deferred; it's active only during completion.)
- [BLOCKED on engine caller] `compmatch.rs::bld_line` — completion
  cross-class equivalence. **Corrected**: NOT a substrate gap.
  `pattern_match_equivalence` is COMPLETE (tracks `lmtp`, resolves
  PP_LOWER/PP_UPPER crossings at compmatch.rs:1897 — the old "lmtp gap"
  comment was stale, now fixed). The real blocker is the engine caller:
  the live caller at compmatch.rs:2438 passes an empty `mword`
  ("CPAT_CHAR-only path"), so the EQUIV branch never fires. Faithful
  bld_line (C's two-pass genpatarr, c:1772-1875) + a non-empty mword
  require porting the completion-matcher chain first.
- [BLOCKED] `hashtable.rs::addhistnode` — `ring_get` returns a clone; no
  `ring_get_mut`, so the C `he->node.flags |= HIST_DUP` ring mutation
  can't be expressed without a new mutable ring accessor
- [x] `modules/zutil.rs::setstypat` — **done** (the `-e` eval path faked
  the program with an empty `eprog::default()`; now `parse_string` →
  `dupeprog` stores the real Eprog in `stypat.eval`; `bin_zstyle` routed
  through `setstypat` to match C and avoid a re-lock; new test pins
  `len > 0`. Residual ratio flag is a detector artifact — C inlines the
  weight scoring in `setstypat`; the Rust factored it into `set`.)
- [ ] `compcore.rs::callcompfunc`, `complete.rs::bin_compadd` (VERIFY — may
  already be adequate)
- [BLOCKED] `modules/curses.rs::zccmd_input` — non-mouse path already
  faithful; the `KEY_MOUSE` event-decode branch (c:1162-1216) needs xterm
  SGR-mouse sequence parsing in `read_key_sequence`. libncurses is NOT
  linked (curses.rs:61), so ncurses `getmouse`/`MEVENT` are unavailable —
  the decode must be reimplemented in the input layer. BLOCKED.
- likely NOT-FAKE (idiom): `modules/db_gdbm.rs::unmetafy_zalloc` — the C
  `zalloc`+`memcpy`+`zsfree` exact-size-copy dance is exactly what an owned
  Rust `String` provides; current port returns `(String, len)` — faithful
  in idiom, pending an `unmeta` NUL-safety check

**BLOCKED on engine substrate** (port requires building the engine first
— NOT faithfully portable now):
- ZLE refresh/input engine (`nbuf`/`obuf`/`zputc`/`vcs`/`vln`/`watch_fds`/
  `kungetbuf`/`zlecore`): `zrefresh`, `tc_rightcurs`, `moveto`, `getbyte`,
  `raw_getbyte`, `zleread`, `doisearch`, `getvisrchstr`, `describekeybriefly`,
  `showmsg`, `backwardmetafiedchar`, `termquery::probe_terminal`
- Completion `Cline`/`minfo`/menu-state graph (`mselect`/`mlbeg`/`mcol`/
  `lastprebr`/`lastpostbr`/`zlemetaline`): `do_single`, `cline_str`,
  `instmatch`, `cut_cline`, `do_allmatches`, `hasbrpsfx`, `valid_match`,
  `build_pos_string`, `domenuselect`, `clnicezputs`, `set_comp_sep`
- compctl `CC_*` hashtable walkers: `makecomplistflags`, `makecomplistext`,
  `makecomplistctl`, `addmatch`, `cfp_add_sdirs`
- hist ring mutable accessor (no `ring_get_mut`; `ring_get` returns a clone):
  `addhistnode`
- job-table model (`findproc`/`update_job`/`jobtab`/`exstack`):
  `wait_for_processes` (unless already adequate — see VERIFY)
- linklist + param-subst + `shout`: `checkmailpath`
- HISTORY_IGNORE/atomic-rename/lock-retry: `savehistfile`, `lockhistfile`

**VERIFY — drained.** All items audited and reclassified or partially
ported.

- [VERIFIED NOT-FAKE] `glob.rs::zglob` — audited: 143 lines (the detector
  mismeasured at 60). Full glob entry logic — np guard, GLOBOPT/haswilds/
  EXECOPT short-circuit (c:1230-1233), `enter_glob_scope` (save_globstate),
  `uremnode`, `globdata_glob` (scanner walk + qualifier parse), badcshglob
  accounting (c:1872), NULLGLOB/CSHNULLGLOB/NOMATCH (c:1843-1887), ordinary-
  string fallback (c:1882-1887), `insert_glob_match` splice (c:1995-2007).
  The first triage agent's "missing qualifier parsing (c:1240-2012)" was
  STALE — the qualifier parse landed (per the `glob_qual_arena_port`
  memory: struct-qual arena + QualArena + globdata.quals), confirmed by
  **47 `qualifier` + 5 `glob_qual` + 99 `glob_` tests passing**. Not a
  fake; ratio flag is a detector artifact. Reclassified out of PORT.

- [VERIFIED NOT-FAKE — re-architected] `exec.rs::execpline` — the
  "pipe/fork primitives deferred (structural stub)" triage label was
  WRONG. `execpline` (7860) is the real WC_PIPE dispatch loop: it handles
  the WC_SUBLIST_NOT/Z_TIMED early return (c:1677-1680) and dispatches
  every pipe stage by WC_* tag via the full `execfuncs[]` table
  (WC_SIMPLE→execsimple, WC_SUBSH→execcursh, WC_FOR→execfor, … c:5499).
  C's monolithic execpline inlines fork/pipe; the Rust splits the
  multi-stage fork/pipe isolation into `execpline2` (exec.rs:7716, **131
  lines**) + the fusevm `OpPipeCreate`/`OpFork` bytecode ops (no-fork
  architecture). Not a structural fake; the ratio flag (68/237) is a
  detector artifact (monolithic C fn vs split Rust). Reclassified out of
  the genuine-fake list.

- [VERIFIED NOT-FAKE — re-architected] `glob.rs::scanner` — C's
  monolithic 162-line `scanner` (Complist linked-list + Patprog +
  opendir/readdir) is re-architected in Rust as a component-based engine:
  a faithful `complist` port keeps the `closure` field (c:255, set to
  1/2 for `#`/`##` at glob.rs:590), and `scanner` (381) is a dispatcher
  over a `PatternComponent` enum (Pattern / Recursive / `.`,`..` nav)
  delegating to `scan_pattern`/`scan_recursive`, using `fs::read_dir`
  instead of opendir and `lchdir` for the long-path descent. Major
  features confirmed present (patterns, recursion/closure, dotdot
  navigation) and the engine passes **99 `glob_` tests**. Not a
  structural fake; the ratio flag is a detector artifact (monolithic C
  fn vs split Rust engine). Caveat: verified feature-presence + test
  pass, not an exhaustive per-qualifier line audit. Reclassified out of
  the genuine-fake list.

- [partial] `init.rs::source` — audited + highest-value gap closed. Was
  missing the `FS_SOURCE` funcstack push (c:1610-1618) — sourced files
  didn't appear in `$funcstack`/`$functrace`/`$funcfiletrace`. **Ported**
  faithfully (push after `sourcelevel++`, pop at c:1664, mirroring the
  FS_FUNC push convention in exec.rs::doshfunc); new balance test +
  source_/funcstack regression green. The core (find file, .zwc via
  try_source_file, execute via fusevm pipeline, scriptname save/restore,
  sourcelevel) was already faithful. STILL MISSING (smaller gaps, tracked):
  `lineno` reset-to-1/restore (c:1593,1615 → `$LINENO` reflects outer
  context inside sourced files), `trap_state = TRAP_STATE_INACTIVE`
  (c:1633), the `SHINSTDIN` dosetopt toggle (c:1595), and SOURCETRACE
  output (c:1597-1600).

- [AUDITED — duplicate, test-only] `prompt.rs::match_highlight` — there
  are **two** Rust `match_highlight`s. The flagged one (prompt.rs:3281)
  calls `parsehighlight` (bold/underline/standout/none/named-fg/named-bg)
  and is used **only by its own tests**. The production callers
  (zle_refresh.rs:388/3084, hlgroup.rs:60) use a separate, fuller
  `match_highlight` at **zle_refresh.rs:2599** that also handles numeric
  colors (`bg=42`) and negation (`nobold`) — see its tests 3536-3558.
  So prompt.rs:3281 is a simplified test-only duplicate, not the live
  parser and not a structural fake. ACTION ITEM (flagged, not done in a
  loop tick): consolidate the two `match_highlight` impls per the
  no-duplicate-implementations rule — needs the boss's sign-off since it
  touches the zle_refresh/hlgroup call sites. Neither impl yet covers
  C's `layer=`/`opacity=` clauses (tracked). Reclassified out of PORT
  (the flagged fn isn't the live highlight parser).

- [VERIFIED NOT-FAKE] `prompt.rs::promptexpand` — audited against C: the
  heavy lifting is faithful. `promptexpand` delegates to `expand_prompt`,
  which handles PROMPTSUBST (`parsestr`+`singsub`, c:192-212) and calls
  `putpromptchar` (c:1305) — a **1375-line** port covering the full
  `%`-escape set (`%c %~ %C %n %M %m %S %s %B %b %U %u %D %T %j %g %l %L
  %v %V %i %I %h %E %G %_ %w %y` …) with ~45 unit tests. The wrapper does
  the `ns==0` Inpar/Outpar/Nularg strip faithfully. Two **honestly
  documented** minor approximations remain: the `marker` arg is ignored
  (ZLE prompt-start positioning) and rs/Rs offsets are approximated via
  source-string `find` rather than expanded-buffer offsets (lossy when
  expansion changes length) — real limitations, disclosed in-code, not
  faked. The ratio flag measures the thin wrapper, not the 1375-line
  `putpromptchar` where the work lives. **Reclassified out of PORT**
  (two minor approximations tracked as known limitations, not fakes).

- [x] `builtin.rs::cd_new_pwd` — **audited + one real gap closed**. The
  decoupling is faithful: dirstack rotation → `bin_cd`/`bin_pushd_popd`
  (1718/1755/1808), PWD/OLDPWD shift → `bin_cd` (authoritative, logical
  path), chpwd hook → fusevm_bridge.rs:891 `callhookfunc("chpwd")` →
  utils.rs:1532 (shfunc + array), stat-validation subsumed by `lchdir`'s
  dev/ino integrity check, printing in `cd_new_pwd` itself. The one
  genuine gap: the `DIRSTACKSIZE` cap (c:1264-1271) was missing (and the
  doc comment falsely claimed it) — now ported faithfully. cd/pushd/popd
  regression green. (No dedicated unit test: `DIRSTACKSIZE` doesn't
  round-trip via `setiparam`/`getiparam` in isolated test context —
  `assignnparam` needs shell state; verified by translation + regression.)

- [VERIFIED NOT-FAKE] `parse.rs::par_subsh` — audited against C: zshrs's
  parser is **AST-based** (`par_subsh` returns `ZshCommand::Subsh`, fed
  to the fusevm compiler), not zsh's wordcode-emit (`ecadd`/`ecbuf` are a
  separate `.zwc`-compat track). C's single par_subsh emits wordcode for
  `(...)`, `{...}`, and the optional `always` block; zshrs splits these
  across AST handlers — `par_subsh` (2276) parses `(...)` (called live at
  1274), and the `{...} always {...}` construct is handled at parse.rs:7025
  (`cmdpush(CS_ALWAYS)`, matching C parse.c:1637). Output consumed by the
  compiler/heredoc/matchers (5136/10508). Faithful to the AST architecture;
  residual ratio flag is a detector artifact. **Reclassified out of PORT.**

- [AUDITED — vestigial] `parse.rs::load_dump_file` — **callerless**.
  zshrs replaced C's mmap-and-register design (parse.c:3675-3725: mmap
  the `.zwc`, push a `FuncDump` onto the global `dumps` list) with an
  owned-`Vec<u32>` model: `funcdump.map` is a `Vec<u32>`, dumps are
  loaded via `load_dump_header`, and `check_dump_file` walks `DUMPS` by
  dev/ino (parse.rs:4733-4742) — all faithful. The standalone
  `load_dump_file` just `read_to_end`s bytes and registers nothing;
  nothing calls it. Not a structural-shell fake (real read work), not
  faithful to C (no mmap/register). A faithful mmap-port is low-value
  (no caller to verify the page-align/offset arithmetic against) and
  the `map`/`addr` aliasing doesn't map cleanly to owned `Vec`s. Kept
  as a vestigial name-anchor (no-delete-shims rule). Residual ratio
  flag is a detector artifact.

- [VERIFIED NOT-FAKE] `compat.rs::zgetdir` — audited against C: the Rust
  faithfully implements the live getcwd branch (compat.c:360-377 —
  `current_dir()` → return cwd, set `d->dirname` at c:374). The 146-line
  C count is dominated by the opendir/readdir walk fallback (c:380-510),
  which is dead code when `USE_GETCWD` is defined — and config_h.rs:1078
  sets `USE_GETCWD = 1` on all our targets (macOS aarch64, Linux
  x86_64/aarch64). Same platform-dead-branch situation as `lchdir`'s
  no-fchdir arm. The residual ratio flag is a detector artifact.
  **Reclassified out of PORT.**

## Counts

- PORT: 52  (genuine fakes — C work faked; require faithful port)
  - done: 7 full + 1 partial (`lchdir`, `findsep`, `vireplacechars`,
    `gdbmhashsetfn`, `setstypat`, `cd_new_pwd`, `spaceinline`; partial:
    `source` — FS_SOURCE funcstack push closed) — remaining 44
  - substrate built: `RegionHighlight.flags` (unblocked `spaceinline`
    AND completed `shiftchars`'s predisplay `sub=predisplaylen` path
    c:890-903, which was hardcoded sub=0 waiting for the flag bit; new
    predisplay test proves a ZRH_PREDISPLAY region adjusts differently
    from a plain one).
  - Confirmed dead-ends (callerless/vestigial — not worth standalone
    ports): hist ring mutator (`addhistnode` — dedup handled inline in
    hend() c:1602); `backwardmetafiedchar` (substrate IS_COMBINING/
    IS_BASECHAR/alignmultiwordleft already exists, but the no-arg Rust fn
    is test-only — C's `(start,endptr,retchr)` form is used by blocked
    engines doisearch/complete/zle_misc).
  - reclassified NOT-FAKE on audit: 4 (`zgetdir` — live getcwd branch is
    faithful, walk fallback platform-dead `USE_GETCWD=1`; `par_subsh` —
    AST parser architecture, `{...}`/always split to parse.rs:7025;
    `promptexpand` — core in putpromptchar 1375 lines + 45 tests, two
    honest minor approximations; `scanner` — re-architected glob engine,
    closure/recursion/patterns present, 99 glob tests; `execpline` —
    real WC_PIPE dispatch loop, fork/pipe split into execpline2 + fusevm;
    `zglob` — 143-line full glob entry, qualifier parse landed, 47+99 tests)
  - **VERIFY bucket fully drained.** Every remaining genuine fake is in
    the engine-blocked cluster (ZLE refresh/input, completion Cline/minfo
    graph, hist ring mutator, job table) — faithful porting requires
    building the engine first; the 1-min loop cannot. Recommend stopping
    the loop here and greenlighting a focused engine build for further
    porting throughput.
  - audited vestigial/duplicate (not the live path): 2
    (`load_dump_file` — callerless, zshrs uses owned-Vec dumps via
    load_dump_header; `prompt.rs::match_highlight` — test-only duplicate,
    live parser is zle_refresh.rs::match_highlight)
  - ACTION ITEM flagged: consolidate the two `match_highlight` impls
    (no-duplicate-implementations rule; needs boss sign-off)
  - confirmed BLOCKED: `zccmd_input` (mouse decode / no ncurses)
  - NOTE: READY bucket exhausted. Remaining items are completion/ZLE-
    engine-blocked or VERIFY-bucket audits. Continued *porting* throughput
    now requires building an engine, not 1-min cherry-picking.
  - newly-confirmed BLOCKED on close inspection: `spaceinline`, `bld_line`,
    `addhistnode` (the "READY" triage bucket was over-optimistic — most
    left items are substrate-blocked; building the engines is the real
    prerequisite, not a porting sprint)
- UNFAKE: 0  (all 4 candidates verified not-fake)
- NOT FAKE (excluded): 67
