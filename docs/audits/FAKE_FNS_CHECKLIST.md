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
- [ ] `zle/zle_main.rs::getbyte` — C zle_main.c:861 (76) — signal queue / timeout / EOF / device-reattach
- [ ] `zle/zle_main.rs::raw_getbyte` — C zle_main.c:506 (242) — signal masks / watch-fd / timeout recalc (~80 of 242)
- [ ] `zle/zle_main.rs::zleread` — C zle_main.c:1216 (127+) — skeleton; undo/history/hooks/prompt setup missing
- [ ] `zle/zle_utils.rs::showmsg` — C zle_utils.c:1310 (72) — multibyte width-aware message display
- [ ] `zle/zle_utils.rs::spaceinline` — C zle_utils.c:784 (54) — buffer insertion w/ region-highlight adjustment
- [ ] `zle/zle_move.rs::backwardmetafiedchar` — C zle_move.c:170 (75) — UTF-8/Meta backward scan w/ combining chars
- [ ] `zle/zle_vi.rs::vireplacechars` — C zle_vi.c:594 (62) — newline-special + combining-char + shiftchars edge cases
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

## Counts

- PORT: 52  (genuine fakes — C work faked; require faithful port)
  - done: 2 (`lchdir`, `findsep`) — remaining 50
- UNFAKE: 0  (all 4 candidates verified not-fake)
- NOT FAKE (excluded): 67
