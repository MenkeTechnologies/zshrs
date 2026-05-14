# PORT_STUBS — stubs detected in src/ported/

Generated: 2026-05-14T19:20:43.003455+00:00

## Method

For each top-level `fn` in `src/ported/**.rs`, the script finds the
same-named function in the matching upstream C source
(`/Users/wizard/forkedRepos/zsh/Src/...`) and compares non-blank/
non-comment body line counts. A fn is flagged as a stub when the
Rust body is **less than 30% of the C body** AND the C body is at
least 10 lines.

Regenerate via:
```
python3 scripts/gen_port_stubs.py
```

## Summary: 293 stubs across 52 files

| File | Stubs | Worst (Rust / C lines) |
|---|---|---|
| `src/ported/utils.rs` | 27 | `lchdir` (8 / 327) |
| `src/ported/zle/complist.rs` | 17 | `domenuselect` (1 / 925) |
| `src/ported/modules/zutil.rs` | 13 | `rparseelt` (1 / 101) |
| `src/ported/hist.rs` | 12 | `hdynread` (1 / 18) |
| `src/ported/jobs.rs` | 11 | `clearjobtab` (1 / 22) |
| `src/ported/modules/parameter.rs` | 11 | `scanpmparameters` (0 / 17) |
| `src/ported/zle/zle_refresh.rs` | 11 | `moveto` (2 / 44) |
| `src/ported/zle/zle_utils.rs` | 11 | `getzlequery` (1 / 24) |
| `src/ported/zle/zle_main.rs` | 10 | `describekeybriefly` (1 / 28) |
| `src/ported/glob.rs` | 9 | `insert` (0 / 128) |
| `src/ported/parse.rs` | 9 | `check_dump_file` (1 / 90) |
| `src/ported/zle/compctl.rs` | 9 | `freecompcond` (0 / 37) |
| `src/ported/module.rs` | 8 | `load_and_bind` (1 / 20) |
| `src/ported/zle/compresult.rs` | 8 | `do_single` (4 / 182) |
| `src/ported/zle/zle_hist.rs` | 8 | `doisearch` (12 / 462) |
| `src/ported/zle/zle_keymap.rs` | 8 | `getkeymapcmd` (1 / 89) |
| `src/ported/zle/zle_tricky.rs` | 8 | `doexpansion` (1 / 58) |
| `src/ported/zle/zle_vi.rs` | 8 | `getvirange` (5 / 83) |
| `src/ported/builtin.rs` | 7 | `cd_do_chdir` (4 / 70) |
| `src/ported/hashtable.rs` | 7 | `newhashtable` (1 / 19) |
| `src/ported/pattern.rs` | 7 | `patmatch` (4 / 575) |
| `src/ported/prompt.rs` | 7 | `prompttrunc` (1 / 193) |
| `src/ported/init.rs` | 5 | `init_term` (8 / 91) |
| `src/ported/zle/zle_params.rs` | 5 | `scan_registers` (0 / 14) |
| `src/ported/modules/db_gdbm.rs` | 4 | `gdbmgetfn` (7 / 28) |
| `src/ported/signals.rs` | 4 | `killrunjobs` (1 / 13) |
| `src/ported/zle/zle_thingy.rs` | 4 | `bin_zle_call` (7 / 95) |
| `src/ported/mem.rs` | 3 | `hrealloc` (3 / 140) |
| `src/ported/modules/pcre.rs` | 3 | `zpcre_get_substrings` (1 / 103) |
| `src/ported/zle/termquery.rs` | 3 | `handle_color` (1 / 24) |
| `src/ported/zle/zle_move.rs` | 3 | `backwardmetafiedchar` (3 / 75) |
| `src/ported/compat.rs` | 2 | `zgettime_monotonic_if_available` (23 / 404) |
| `src/ported/input.rs` | 2 | `inputline` (5 / 88) |
| `src/ported/math.rs` | 2 | `matheval` (2 / 17) |
| `src/ported/modules/curses.rs` | 2 | `zccmd_input` (33 / 179) |
| `src/ported/modules/param_private.rs` | 2 | `setup_` (1 / 12) |
| `src/ported/modules/zftp.rs` | 2 | `zfmovefd` (1 / 10) |
| `src/ported/modules/zprof.rs` | 2 | `zprof_wrapper` (1 / 69) |
| `src/ported/modules/zpty.rs` | 2 | `newptycmd` (18 / 147) |
| `src/ported/options.rs` | 2 | `dosetopt` (11 / 106) |
| `src/ported/params.rs` | 2 | `setscope_base` (3 / 13) |
| `src/ported/zle/compcore.rs` | 2 | `set_comp_sep` (9 / 321) |
| `src/ported/zle/zle_misc.rs` | 2 | `makesuffixstr` (3 / 42) |
| `src/ported/builtins/rlimits.rs` | 1 | `printrlim` (1 / 13) |
| `src/ported/modules/datetime.rs` | 1 | `getcurrenttime` (3 / 11) |
| `src/ported/modules/files.rs` | 1 | `recursivecmd` (12 / 59) |
| `src/ported/modules/hlgroup.rs` | 1 | `getgroup` (1 / 22) |
| `src/ported/modules/mathfunc.rs` | 1 | `math_string` (12 / 68) |
| `src/ported/modules/termcap.rs` | 1 | `scantermcap` (8 / 85) |
| `src/ported/modules/watch.rs` | 1 | `readwtab` (20 / 183) |
| `src/ported/zle/compmatch.rs` | 1 | `bld_line` (40 / 139) |
| `src/ported/zle/computil.rs` | 1 | `cfp_add_sdirs` (18 / 73) |

## Per-file detail

### `src/ported/utils.rs` — 27 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 4182 | `lchdir` | 8 | 327 | 2% |
| 2037 | `spckword` | 10 | 156 | 6% |
| 3251 | `ztrsub` | 1 | 13 | 7% |
| 3348 | `mb_niceformat` | 6 | 70 | 8% |
| 2079 | `ztrftime` | 28 | 202 | 13% |
| 269 | `zerrmsg` | 12 | 81 | 14% |
| 2182 | `spacesplit` | 5 | 33 | 15% |
| 3438 | `mb_metacharlenconv_r` | 5 | 32 | 15% |
| 3538 | `sb_niceformat` | 9 | 55 | 16% |
| 1872 | `checkrmall` | 8 | 48 | 16% |
| 2193 | `findsep` | 7 | 42 | 16% |
| 3476 | `mb_metastrlenend` | 8 | 48 | 16% |
| 325 | `nicechar_sel` | 5 | 29 | 17% |
| 2000 | `getquery` | 13 | 74 | 17% |
| 68 | `set_widearray` | 6 | 30 | 20% |
| 2357 | `sepjoin` | 5 | 23 | 21% |
| 3003 | `metafy` | 12 | 54 | 22% |
| 3122 | `unmeta` | 8 | 35 | 22% |
| 512 | `slashsplit` | 4 | 17 | 23% |
| 3258 | `zreaddir` | 8 | 32 | 25% |
| 3780 | `quotedzputs` | 38 | 152 | 25% |
| 1761 | `read_poll` | 16 | 62 | 25% |
| 968 | `checkmailpath` | 19 | 73 | 26% |
| 1344 | `zclose` | 4 | 15 | 26% |
| 2787 | `itype_end` | 18 | 67 | 26% |
| 1782 | `timespec_diff_us` | 5 | 18 | 27% |
| 863 | `delprepromptfn` | 4 | 14 | 28% |

### `src/ported/zle/complist.rs` — 17 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 779 | `domenuselect` | 1 | 925 | 0% |
| 623 | `clprintm` | 1 | 154 | 0% |
| 658 | `complistmatches` | 1 | 103 | 0% |
| 769 | `msearch` | 1 | 60 | 1% |
| 568 | `compprintfmt` | 4 | 236 | 1% |
| 221 | `getcols` | 1 | 45 | 2% |
| 651 | `singledraw` | 1 | 44 | 2% |
| 742 | `setmstatus` | 1 | 43 | 2% |
| 516 | `putfilecol` | 2 | 71 | 2% |
| 827 | `menuselect_bindings` | 1 | 25 | 4% |
| 532 | `asklistscroll` | 2 | 40 | 5% |
| 199 | `getcolval` | 2 | 37 | 5% |
| 634 | `singlecalc` | 1 | 15 | 6% |
| 760 | `msearchpop` | 1 | 10 | 10% |
| 506 | `putmatchcol` | 2 | 15 | 13% |
| 840 | `boot_` | 2 | 15 | 13% |
| 472 | `clnicezputs` | 25 | 107 | 23% |

### `src/ported/modules/zutil.rs` — 13 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1076 | `rparseelt` | 1 | 101 | 0% |
| 1163 | `rmatch` | 1 | 90 | 1% |
| 1141 | `rparseseq` | 1 | 41 | 2% |
| 1370 | `zalloc_default_array` | 1 | 17 | 5% |
| 1349 | `map_opt_desc` | 1 | 16 | 6% |
| 613 | `newzstyletable` | 1 | 15 | 6% |
| 1152 | `rparsealt` | 1 | 13 | 7% |
| 1328 | `lookup_opt` | 1 | 13 | 7% |
| 45 | `restorematch` | 1 | 12 | 8% |
| 1085 | `rparseclo` | 1 | 10 | 10% |
| 630 | `setstypat` | 6 | 58 | 10% |
| 113 | `freestylenode` | 2 | 10 | 20% |
| 1059 | `connectstates` | 4 | 15 | 26% |

### `src/ported/hist.rs` — 12 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1194 | `hdynread` | 1 | 18 | 5% |
| 176 | `iaddtoline` | 1 | 13 | 7% |
| 1431 | `bufferwords` | 15 | 158 | 9% |
| 1345 | `savehistfile` | 22 | 199 | 11% |
| 1517 | `saveandpophiststack` | 2 | 16 | 12% |
| 1387 | `lockhistfile` | 21 | 124 | 16% |
| 1450 | `histsplitwords` | 24 | 119 | 20% |
| 1038 | `casemodify` | 23 | 103 | 22% |
| 870 | `getargspec` | 8 | 34 | 23% |
| 491 | `putoldhistentryontop` | 8 | 33 | 24% |
| 373 | `histreduceblanks` | 10 | 39 | 25% |
| 1027 | `remlpaths` | 7 | 26 | 26% |

### `src/ported/jobs.rs` — 11 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1055 | `clearjobtab` | 1 | 22 | 4% |
| 1117 | `spawnjob` | 4 | 24 | 16% |
| 277 | `handle_sub` | 10 | 57 | 17% |
| 2083 | `gettrapnode` | 6 | 30 | 20% |
| 895 | `freejob` | 7 | 34 | 20% |
| 638 | `should_report_time` | 12 | 55 | 21% |
| 402 | `update_job` | 33 | 150 | 22% |
| 545 | `printtime` | 56 | 239 | 23% |
| 1130 | `shelltime` | 13 | 51 | 25% |
| 810 | `addfilelist` | 4 | 15 | 26% |
| 1497 | `addbgstatus` | 12 | 44 | 27% |

### `src/ported/modules/parameter.rs` — 11 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 157 | `scanpmparameters` | 0 | 17 | 0% |
| 1091 | `scanpmmodules` | 0 | 47 | 0% |
| 1164 | `scanpmhistory` | 0 | 18 | 0% |
| 1231 | `scanpmjobtexts` | 0 | 21 | 0% |
| 1257 | `scanpmjobstates` | 0 | 21 | 0% |
| 1289 | `scanpmjobdirs` | 0 | 21 | 0% |
| 1608 | `setaliases` | 0 | 25 | 0% |
| 743 | `funcfiletracegetfn` | 4 | 29 | 13% |
| 562 | `scanfunctions` | 12 | 50 | 24% |
| 730 | `funcsourcetracegetfn` | 4 | 16 | 25% |
| 717 | `functracegetfn` | 4 | 15 | 26% |

### `src/ported/zle/zle_refresh.rs` — 11 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 804 | `moveto` | 2 | 44 | 4% |
| 827 | `tc_rightcurs` | 4 | 63 | 6% |
| 582 | `addmultiword` | 1 | 14 | 7% |
| 929 | `singmoveto` | 1 | 12 | 8% |
| 503 | `freevideo` | 2 | 19 | 10% |
| 478 | `zwcputc` | 3 | 24 | 12% |
| 514 | `resetvideo` | 7 | 53 | 13% |
| 601 | `zrefresh` | 87 | 602 | 14% |
| 849 | `tcout_via_func` | 5 | 34 | 14% |
| 558 | `snextline` | 5 | 27 | 18% |
| 544 | `nextline` | 6 | 22 | 27% |

### `src/ported/zle/zle_utils.rs` — 11 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 522 | `getzlequery` | 1 | 24 | 4% |
| 607 | `showmsg` | 5 | 72 | 6% |
| 74 | `zlecharasstring` | 3 | 43 | 6% |
| 985 | `get_undo_current_change` | 1 | 11 | 9% |
| 211 | `shiftchars` | 7 | 64 | 10% |
| 96 | `stringaszleline` | 14 | 110 | 12% |
| 174 | `zle_restore_positions` | 7 | 51 | 13% |
| 195 | `spaceinline` | 8 | 54 | 14% |
| 249 | `cuttext` | 18 | 79 | 22% |
| 161 | `zle_save_positions` | 8 | 34 | 23% |
| 53 | `sizeline` | 5 | 18 | 27% |

### `src/ported/zle/zle_main.rs` — 10 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 855 | `describekeybriefly` | 1 | 28 | 3% |
| 621 | `execzlefunc` | 11 | 149 | 7% |
| 448 | `redrawhook` | 2 | 25 | 8% |
| 68 | `zsetterm` | 10 | 104 | 9% |
| 352 | `getbyte` | 10 | 76 | 13% |
| 544 | `zleread` | 20 | 127 | 15% |
| 270 | `raw_getbyte` | 46 | 242 | 19% |
| 953 | `reexpandprompt` | 5 | 26 | 19% |
| 425 | `getrestchar` | 7 | 36 | 19% |
| 1008 | `trashzle` | 5 | 22 | 22% |

### `src/ported/glob.rs` — 9 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 276 | `insert` | 0 | 128 | 0% |
| 1058 | `igetmatch` | 1 | 268 | 0% |
| 322 | `parsecomplist` | 1 | 55 | 1% |
| 596 | `zglob` | 13 | 612 | 2% |
| 844 | `get_match_ret` | 5 | 71 | 7% |
| 293 | `scanner` | 17 | 162 | 10% |
| 1370 | `qualsheval` | 7 | 25 | 28% |
| 1334 | `qualsize` | 10 | 35 | 28% |
| 523 | `glob_exec_string` | 7 | 24 | 29% |

### `src/ported/parse.rs` — 9 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 3499 | `check_dump_file` | 1 | 90 | 1% |
| 3361 | `build_dump` | 2 | 75 | 2% |
| 3478 | `try_dump_file` | 1 | 31 | 3% |
| 3491 | `try_source_file` | 1 | 25 | 4% |
| 3422 | `build_cur_dump` | 5 | 80 | 6% |
| 3571 | `dump_autoload` | 2 | 19 | 10% |
| 1639 | `par_subsh` | 6 | 34 | 17% |
| 3454 | `load_dump_file` | 12 | 47 | 25% |
| 1587 | `par_while` | 9 | 32 | 28% |

### `src/ported/zle/compctl.rs` — 9 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 147 | `freecompcond` | 0 | 37 | 0% |
| 2463 | `makecomplistflags` | 19 | 746 | 2% |
| 917 | `delpatcomp` | 2 | 13 | 15% |
| 1931 | `makecomplistctl` | 12 | 74 | 16% |
| 1581 | `addmatch` | 17 | 99 | 17% |
| 1639 | `getreal` | 2 | 11 | 18% |
| 1974 | `makecomplistext` | 36 | 150 | 24% |
| 110 | `createcompctltable` | 4 | 14 | 28% |
| 1266 | `ccmakehookfn` | 32 | 112 | 28% |

### `src/ported/module.rs` — 8 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 815 | `load_and_bind` | 1 | 20 | 5% |
| 1079 | `module_func` | 1 | 16 | 6% |
| 283 | `checkaddparam` | 1 | 15 | 6% |
| 854 | `try_load_module` | 1 | 14 | 7% |
| 36 | `printmodulenode` | 10 | 89 | 11% |
| 2022 | `bin_zmodload_features` | 26 | 217 | 11% |
| 1942 | `unload_named_module` | 5 | 36 | 13% |
| 2129 | `autofeatures` | 45 | 160 | 28% |

### `src/ported/zle/compresult.rs` — 8 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 316 | `do_single` | 4 | 182 | 2% |
| 227 | `hasbrpsfx` | 1 | 39 | 2% |
| 336 | `valid_match` | 1 | 32 | 3% |
| 297 | `do_allmatches` | 2 | 49 | 4% |
| 157 | `build_pos_string` | 1 | 23 | 4% |
| 69 | `cut_cline` | 5 | 81 | 6% |
| 207 | `instmatch` | 6 | 80 | 7% |
| 96 | `cline_str` | 45 | 271 | 16% |

### `src/ported/zle/zle_hist.rs` — 8 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 997 | `doisearch` | 12 | 462 | 2% |
| 1091 | `getvisrchstr` | 6 | 118 | 5% |
| 948 | `isearch_newpos` | 2 | 19 | 10% |
| 564 | `historysearchbackward` | 8 | 49 | 16% |
| 579 | `historysearchforward` | 8 | 49 | 16% |
| 664 | `insertlastword` | 18 | 97 | 18% |
| 1180 | `historybeginningsearchbackward` | 8 | 36 | 22% |
| 1195 | `historybeginningsearchforward` | 8 | 36 | 22% |

### `src/ported/zle/zle_keymap.rs` — 8 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1247 | `getkeymapcmd` | 1 | 89 | 1% |
| 1309 | `getkeycmd` | 1 | 29 | 3% |
| 1233 | `getrestchar_keybuf` | 2 | 39 | 5% |
| 137 | `emptykeymapnamtab` | 1 | 14 | 7% |
| 1205 | `default_bindings` | 17 | 121 | 14% |
| 1185 | `add_cursor_key` | 3 | 18 | 16% |
| 994 | `bin_bindkey_meta` | 4 | 19 | 21% |
| 885 | `bin_bindkey_lsmaps` | 4 | 16 | 25% |

### `src/ported/zle/zle_tricky.rs` — 8 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 734 | `doexpansion` | 1 | 58 | 1% |
| 688 | `get_comp_string` | 15 | 794 | 1% |
| 1057 | `endoflist` | 1 | 12 | 8% |
| 765 | `pfxlen` | 4 | 38 | 10% |
| 855 | `listlist` | 33 | 174 | 18% |
| 377 | `docomplete` | 40 | 210 | 19% |
| 1034 | `expandcmdpath` | 6 | 28 | 21% |
| 952 | `magicspace` | 7 | 25 | 28% |

### `src/ported/zle/zle_vi.rs` — 8 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 129 | `getvirange` | 5 | 83 | 6% |
| 548 | `virepeatchange` | 1 | 16 | 6% |
| 785 | `viquotedinsert` | 2 | 20 | 10% |
| 78 | `startvichange` | 3 | 22 | 13% |
| 103 | `vigetkey` | 5 | 34 | 14% |
| 147 | `dovilinerange` | 4 | 23 | 17% |
| 409 | `vireplacechars` | 14 | 62 | 22% |
| 231 | `videletechar` | 5 | 19 | 26% |

### `src/ported/builtin.rs` — 7 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1258 | `cd_do_chdir` | 4 | 70 | 5% |
| 4368 | `bin_print` | 67 | 812 | 8% |
| 1326 | `cd_new_pwd` | 6 | 71 | 8% |
| 5057 | `eval` | 4 | 45 | 8% |
| 4908 | `zexit` | 6 | 53 | 11% |
| 5197 | `bin_read` | 111 | 583 | 19% |
| 1299 | `cd_try_chdir` | 11 | 46 | 23% |

### `src/ported/hashtable.rs` — 7 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 62 | `newhashtable` | 1 | 19 | 5% |
| 838 | `hashdir` | 4 | 58 | 6% |
| 716 | `expandhashtable` | 2 | 15 | 13% |
| 725 | `resizehashtable` | 4 | 18 | 22% |
| 1047 | `freeshfuncnode` | 4 | 17 | 23% |
| 1570 | `addhistnode` | 4 | 17 | 23% |
| 761 | `printhashtabinfo` | 6 | 22 | 27% |

### `src/ported/pattern.rs` — 7 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1081 | `patmatch` | 4 | 575 | 0% |
| 874 | `patcompnot` | 1 | 17 | 5% |
| 1041 | `pattryrefs` | 17 | 202 | 8% |
| 1181 | `savepatterndisables` | 1 | 10 | 10% |
| 1110 | `patmatchrange` | 21 | 113 | 18% |
| 340 | `patcompswitch` | 24 | 88 | 27% |
| 1137 | `patmatchindex` | 18 | 63 | 28% |

### `src/ported/prompt.rs` — 7 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 434 | `prompttrunc` | 1 | 193 | 0% |
| 1779 | `match_highlight` | 6 | 75 | 8% |
| 283 | `promptexpand` | 4 | 43 | 9% |
| 359 | `parsecolorchar` | 3 | 24 | 12% |
| 597 | `mixattrs` | 9 | 60 | 15% |
| 475 | `applytextattributes` | 8 | 52 | 15% |
| 1751 | `match_colour` | 13 | 55 | 23% |

### `src/ported/init.rs` — 5 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 315 | `init_term` | 8 | 91 | 8% |
| 661 | `source` | 11 | 97 | 11% |
| 617 | `run_init_scripts` | 8 | 62 | 12% |
| 296 | `init_shout` | 4 | 23 | 17% |
| 179 | `parseopts` | 30 | 144 | 20% |

### `src/ported/zle/zle_params.rs` — 5 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 601 | `scan_registers` | 0 | 14 | 0% |
| 773 | `get_zle_state` | 9 | 46 | 19% |
| 536 | `set_killring` | 6 | 27 | 22% |
| 206 | `set_rbuffer` | 4 | 14 | 28% |
| 188 | `set_lbuffer` | 5 | 17 | 29% |

### `src/ported/modules/db_gdbm.rs` — 4 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 248 | `gdbmgetfn` | 7 | 28 | 25% |
| 267 | `gdbmsetfn` | 7 | 28 | 25% |
| 477 | `gdbmuntie` | 3 | 12 | 25% |
| 927 | `unmetafy_zalloc` | 3 | 11 | 27% |

### `src/ported/signals.rs` — 4 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 452 | `killrunjobs` | 1 | 13 | 7% |
| 285 | `wait_for_processes` | 10 | 102 | 9% |
| 635 | `removetrap` | 5 | 45 | 11% |
| 462 | `killjb` | 5 | 39 | 12% |

### `src/ported/zle/zle_thingy.rs` — 4 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1062 | `bin_zle_call` | 7 | 95 | 7% |
| 697 | `bin_zle_refresh` | 6 | 31 | 19% |
| 1125 | `bin_zle_fd` | 21 | 81 | 25% |
| 1024 | `bin_zle_flags` | 11 | 39 | 28% |

### `src/ported/mem.rs` — 3 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 229 | `hrealloc` | 3 | 140 | 2% |
| 311 | `realloc` | 2 | 32 | 6% |
| 128 | `old_heaps` | 3 | 38 | 7% |

### `src/ported/modules/pcre.rs` — 3 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 180 | `zpcre_get_substrings` | 1 | 103 | 0% |
| 143 | `pcre_callout` | 1 | 15 | 6% |
| 324 | `cond_pcre_match` | 19 | 70 | 27% |

### `src/ported/zle/termquery.rs` — 3 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 235 | `handle_color` | 1 | 24 | 4% |
| 249 | `query_terminal` | 7 | 42 | 16% |
| 167 | `probe_terminal` | 46 | 208 | 22% |

### `src/ported/zle/zle_move.rs` — 3 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 143 | `backwardmetafiedchar` | 3 | 75 | 4% |
| 213 | `beginningoflinehist` | 7 | 32 | 21% |
| 228 | `endoflinehist` | 7 | 30 | 23% |

### `src/ported/compat.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 69 | `zgettime_monotonic_if_available` | 23 | 404 | 5% |
| 217 | `zgetdir` | 15 | 146 | 10% |

### `src/ported/input.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 293 | `inputline` | 5 | 88 | 5% |
| 420 | `inpoptop` | 8 | 28 | 28% |

### `src/ported/math.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1857 | `matheval` | 2 | 17 | 11% |
| 1339 | `setmathvar` | 7 | 44 | 15% |

### `src/ported/modules/curses.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1028 | `zccmd_input` | 33 | 179 | 18% |
| 611 | `zccmd_delwin` | 12 | 44 | 27% |

### `src/ported/modules/param_private.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 795 | `setup_` | 1 | 12 | 8% |
| 181 | `is_private` | 7 | 27 | 25% |

### `src/ported/modules/zftp.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 266 | `zfmovefd` | 1 | 10 | 10% |
| 2138 | `zfgetcwd` | 8 | 29 | 27% |

### `src/ported/modules/zprof.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 346 | `zprof_wrapper` | 1 | 69 | 1% |
| 314 | `name_for_anonymous_function` | 1 | 11 | 9% |

### `src/ported/modules/zpty.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 236 | `newptycmd` | 18 | 147 | 12% |
| 126 | `ptysettyinfo` | 4 | 21 | 19% |

### `src/ported/options.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 573 | `dosetopt` | 11 | 106 | 10% |
| 155 | `createoptiontable` | 4 | 16 | 25% |

### `src/ported/params.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 6769 | `setscope_base` | 3 | 13 | 23% |
| 4147 | `assignaparam` | 45 | 187 | 24% |

### `src/ported/zle/compcore.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1240 | `set_comp_sep` | 9 | 321 | 2% |
| 514 | `callcompfunc` | 46 | 345 | 13% |

### `src/ported/zle/zle_misc.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1519 | `makesuffixstr` | 3 | 42 | 7% |
| 1477 | `addsuffix` | 3 | 12 | 25% |

### `src/ported/builtins/rlimits.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 269 | `printrlim` | 1 | 13 | 7% |

### `src/ported/modules/datetime.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 251 | `getcurrenttime` | 3 | 11 | 27% |

### `src/ported/modules/files.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 23 | `recursivecmd` | 12 | 59 | 20% |

### `src/ported/modules/hlgroup.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 207 | `getgroup` | 1 | 22 | 4% |

### `src/ported/modules/mathfunc.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 55 | `math_string` | 12 | 68 | 17% |

### `src/ported/modules/termcap.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 212 | `scantermcap` | 8 | 85 | 9% |

### `src/ported/modules/watch.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 411 | `readwtab` | 20 | 183 | 10% |

### `src/ported/zle/compmatch.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 2032 | `bld_line` | 40 | 139 | 28% |

### `src/ported/zle/computil.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 6654 | `cfp_add_sdirs` | 18 | 73 | 24% |

