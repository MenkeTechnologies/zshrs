# PORT_STUBS — stubs detected in src/ported/

Generated: 2026-05-14T21:17:02.291673+00:00

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

## Summary: 172 stubs across 46 files

| File | Stubs | Worst (Rust / C lines) |
|---|---|---|
| `src/ported/jobs.rs` | 8 | `clearjobtab` (1 / 22) |
| `src/ported/modules/zutil.rs` | 8 | `zalloc_default_array` (1 / 17) |
| `src/ported/zle/zle_hist.rs` | 8 | `doisearch` (12 / 454) |
| `src/ported/zle/zle_refresh.rs` | 8 | `moveto` (2 / 44) |
| `src/ported/prompt.rs` | 7 | `prompttrunc` (1 / 183) |
| `src/ported/utils.rs` | 7 | `lchdir` (8 / 298) |
| `src/ported/zle/compctl.rs` | 7 | `makecomplistflags` (19 / 746) |
| `src/ported/zle/zle_main.rs` | 7 | `getbyte` (10 / 74) |
| `src/ported/hashtable.rs` | 6 | `hashdir` (4 / 54) |
| `src/ported/pattern.rs` | 6 | `patmatch` (4 / 558) |
| `src/ported/zle/compresult.rs` | 6 | `do_single` (4 / 180) |
| `src/ported/zle/zle_tricky.rs` | 6 | `doexpansion` (1 / 58) |
| `src/ported/builtin.rs` | 5 | `cd_new_pwd` (6 / 71) |
| `src/ported/glob.rs` | 5 | `zglob` (13 / 604) |
| `src/ported/hist.rs` | 5 | `bufferwords` (15 / 158) |
| `src/ported/init.rs` | 5 | `init_term` (8 / 86) |
| `src/ported/parse.rs` | 5 | `check_dump_file` (1 / 83) |
| `src/ported/zle/zle_utils.rs` | 5 | `stringaszleline` (14 / 102) |
| `src/ported/modules/db_gdbm.rs` | 4 | `gdbmgetfn` (7 / 28) |
| `src/ported/signals.rs` | 4 | `killrunjobs` (1 / 13) |
| `src/ported/zle/complist.rs` | 4 | `compprintfmt` (4 / 232) |
| `src/ported/zle/zle_params.rs` | 4 | `get_zle_state` (9 / 46) |
| `src/ported/zle/zle_thingy.rs` | 4 | `bin_zle_call` (7 / 95) |
| `src/ported/zle/zle_vi.rs` | 4 | `vigetkey` (5 / 32) |
| `src/ported/zle/termquery.rs` | 3 | `handle_color` (1 / 24) |
| `src/ported/zle/zle_keymap.rs` | 3 | `getrestchar_keybuf` (2 / 39) |
| `src/ported/zle/zle_move.rs` | 3 | `backwardmetafiedchar` (3 / 73) |
| `src/ported/compat.rs` | 2 | `zgettime_monotonic_if_available` (23 / 309) |
| `src/ported/input.rs` | 2 | `inputline` (5 / 88) |
| `src/ported/mem.rs` | 2 | `hrealloc` (3 / 102) |
| `src/ported/modules/curses.rs` | 2 | `zccmd_input` (33 / 162) |
| `src/ported/zle/compcore.rs` | 2 | `set_comp_sep` (9 / 319) |
| `src/ported/zle/zle_misc.rs` | 2 | `makesuffixstr` (3 / 42) |
| `src/ported/math.rs` | 1 | `setmathvar` (7 / 44) |
| `src/ported/module.rs` | 1 | `printmodulenode` (10 / 89) |
| `src/ported/modules/files.rs` | 1 | `recursivecmd` (12 / 59) |
| `src/ported/modules/mathfunc.rs` | 1 | `math_string` (12 / 66) |
| `src/ported/modules/pcre.rs` | 1 | `cond_pcre_match` (19 / 70) |
| `src/ported/modules/termcap.rs` | 1 | `scantermcap` (8 / 78) |
| `src/ported/modules/watch.rs` | 1 | `readwtab` (20 / 169) |
| `src/ported/modules/zprof.rs` | 1 | `name_for_anonymous_function` (1 / 11) |
| `src/ported/modules/zpty.rs` | 1 | `newptycmd` (18 / 118) |
| `src/ported/options.rs` | 1 | `createoptiontable` (4 / 16) |
| `src/ported/params.rs` | 1 | `assignaparam` (45 / 187) |
| `src/ported/zle/compmatch.rs` | 1 | `bld_line` (40 / 137) |
| `src/ported/zle/computil.rs` | 1 | `cfp_add_sdirs` (18 / 73) |

## Per-file detail

### `src/ported/jobs.rs` — 8 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1055 | `clearjobtab` | 1 | 22 | 4% |
| 1117 | `spawnjob` | 4 | 24 | 16% |
| 277 | `handle_sub` | 10 | 57 | 17% |
| 895 | `freejob` | 7 | 34 | 20% |
| 402 | `update_job` | 33 | 146 | 22% |
| 810 | `addfilelist` | 4 | 15 | 26% |
| 638 | `should_report_time` | 12 | 44 | 27% |
| 545 | `printtime` | 56 | 192 | 29% |

### `src/ported/modules/zutil.rs` — 8 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1400 | `zalloc_default_array` | 1 | 17 | 5% |
| 1379 | `map_opt_desc` | 1 | 16 | 6% |
| 613 | `newzstyletable` | 1 | 15 | 6% |
| 1358 | `lookup_opt` | 1 | 13 | 7% |
| 45 | `restorematch` | 1 | 12 | 8% |
| 630 | `setstypat` | 6 | 58 | 10% |
| 113 | `freestylenode` | 2 | 10 | 20% |
| 1059 | `connectstates` | 4 | 15 | 26% |

### `src/ported/zle/zle_hist.rs` — 8 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 997 | `doisearch` | 12 | 454 | 2% |
| 1091 | `getvisrchstr` | 6 | 115 | 5% |
| 948 | `isearch_newpos` | 2 | 19 | 10% |
| 564 | `historysearchbackward` | 8 | 49 | 16% |
| 579 | `historysearchforward` | 8 | 49 | 16% |
| 664 | `insertlastword` | 18 | 97 | 18% |
| 1180 | `historybeginningsearchbackward` | 8 | 36 | 22% |
| 1195 | `historybeginningsearchforward` | 8 | 36 | 22% |

### `src/ported/zle/zle_refresh.rs` — 8 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 804 | `moveto` | 2 | 44 | 4% |
| 827 | `tc_rightcurs` | 4 | 61 | 6% |
| 503 | `freevideo` | 2 | 17 | 11% |
| 514 | `resetvideo` | 7 | 51 | 13% |
| 601 | `zrefresh` | 87 | 566 | 15% |
| 478 | `zwcputc` | 3 | 19 | 15% |
| 558 | `snextline` | 5 | 27 | 18% |
| 544 | `nextline` | 6 | 22 | 27% |

### `src/ported/prompt.rs` — 7 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 434 | `prompttrunc` | 1 | 183 | 0% |
| 1779 | `match_highlight` | 6 | 75 | 8% |
| 283 | `promptexpand` | 4 | 43 | 9% |
| 359 | `parsecolorchar` | 3 | 24 | 12% |
| 597 | `mixattrs` | 9 | 60 | 15% |
| 475 | `applytextattributes` | 8 | 52 | 15% |
| 1751 | `match_colour` | 13 | 55 | 23% |

### `src/ported/utils.rs` — 7 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 4249 | `lchdir` | 8 | 298 | 2% |
| 2123 | `ztrftime` | 28 | 192 | 14% |
| 272 | `zerrmsg` | 12 | 72 | 16% |
| 3175 | `unmeta` | 8 | 35 | 22% |
| 3056 | `metafy` | 12 | 50 | 24% |
| 975 | `checkmailpath` | 19 | 73 | 26% |
| 2840 | `itype_end` | 18 | 65 | 27% |

### `src/ported/zle/compctl.rs` — 7 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 2463 | `makecomplistflags` | 19 | 746 | 2% |
| 917 | `delpatcomp` | 2 | 13 | 15% |
| 1931 | `makecomplistctl` | 12 | 74 | 16% |
| 1581 | `addmatch` | 17 | 99 | 17% |
| 1639 | `getreal` | 2 | 11 | 18% |
| 1974 | `makecomplistext` | 36 | 150 | 24% |
| 110 | `createcompctltable` | 4 | 14 | 28% |

### `src/ported/zle/zle_main.rs` — 7 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 352 | `getbyte` | 10 | 74 | 13% |
| 68 | `zsetterm` | 10 | 70 | 14% |
| 544 | `zleread` | 20 | 125 | 16% |
| 953 | `reexpandprompt` | 5 | 26 | 19% |
| 425 | `getrestchar` | 7 | 36 | 19% |
| 1008 | `trashzle` | 5 | 22 | 22% |
| 270 | `raw_getbyte` | 46 | 195 | 23% |

### `src/ported/hashtable.rs` — 6 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 838 | `hashdir` | 4 | 54 | 7% |
| 716 | `expandhashtable` | 2 | 15 | 13% |
| 725 | `resizehashtable` | 4 | 18 | 22% |
| 1047 | `freeshfuncnode` | 4 | 17 | 23% |
| 1570 | `addhistnode` | 4 | 17 | 23% |
| 761 | `printhashtabinfo` | 6 | 22 | 27% |

### `src/ported/pattern.rs` — 6 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1081 | `patmatch` | 4 | 558 | 0% |
| 874 | `patcompnot` | 1 | 17 | 5% |
| 1041 | `pattryrefs` | 17 | 202 | 8% |
| 1110 | `patmatchrange` | 21 | 110 | 19% |
| 340 | `patcompswitch` | 24 | 88 | 27% |
| 1137 | `patmatchindex` | 18 | 63 | 28% |

### `src/ported/zle/compresult.rs` — 6 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 316 | `do_single` | 4 | 180 | 2% |
| 297 | `do_allmatches` | 2 | 47 | 4% |
| 157 | `build_pos_string` | 1 | 18 | 5% |
| 69 | `cut_cline` | 5 | 81 | 6% |
| 207 | `instmatch` | 6 | 80 | 7% |
| 96 | `cline_str` | 45 | 268 | 16% |

### `src/ported/zle/zle_tricky.rs` — 6 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 734 | `doexpansion` | 1 | 58 | 1% |
| 688 | `get_comp_string` | 15 | 794 | 1% |
| 765 | `pfxlen` | 4 | 35 | 11% |
| 855 | `listlist` | 33 | 174 | 18% |
| 377 | `docomplete` | 40 | 210 | 19% |
| 1034 | `expandcmdpath` | 6 | 28 | 21% |

### `src/ported/builtin.rs` — 5 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1326 | `cd_new_pwd` | 6 | 71 | 8% |
| 4368 | `bin_print` | 67 | 777 | 8% |
| 5057 | `eval` | 4 | 45 | 8% |
| 5197 | `bin_read` | 111 | 533 | 20% |
| 1299 | `cd_try_chdir` | 11 | 43 | 25% |

### `src/ported/glob.rs` — 5 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 596 | `zglob` | 13 | 604 | 2% |
| 844 | `get_match_ret` | 5 | 71 | 7% |
| 293 | `scanner` | 17 | 162 | 10% |
| 1370 | `qualsheval` | 7 | 25 | 28% |
| 523 | `glob_exec_string` | 7 | 24 | 29% |

### `src/ported/hist.rs` — 5 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1441 | `bufferwords` | 15 | 158 | 9% |
| 1355 | `savehistfile` | 22 | 191 | 11% |
| 1397 | `lockhistfile` | 21 | 109 | 19% |
| 1460 | `histsplitwords` | 24 | 117 | 20% |
| 1048 | `casemodify` | 23 | 101 | 22% |

### `src/ported/init.rs` — 5 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 315 | `init_term` | 8 | 86 | 9% |
| 661 | `source` | 11 | 97 | 11% |
| 617 | `run_init_scripts` | 8 | 54 | 14% |
| 179 | `parseopts` | 30 | 142 | 21% |
| 296 | `init_shout` | 4 | 15 | 26% |

### `src/ported/parse.rs` — 5 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 3499 | `check_dump_file` | 1 | 83 | 1% |
| 3478 | `try_dump_file` | 1 | 31 | 3% |
| 3491 | `try_source_file` | 1 | 25 | 4% |
| 1639 | `par_subsh` | 6 | 34 | 17% |
| 1587 | `par_while` | 9 | 32 | 28% |

### `src/ported/zle/zle_utils.rs` — 5 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 96 | `stringaszleline` | 14 | 102 | 13% |
| 174 | `zle_restore_positions` | 7 | 51 | 13% |
| 195 | `spaceinline` | 8 | 54 | 14% |
| 249 | `cuttext` | 18 | 79 | 22% |
| 161 | `zle_save_positions` | 8 | 34 | 23% |

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
| 635 | `removetrap` | 5 | 41 | 12% |
| 285 | `wait_for_processes` | 10 | 81 | 12% |
| 462 | `killjb` | 5 | 39 | 12% |

### `src/ported/zle/complist.rs` — 4 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 568 | `compprintfmt` | 4 | 232 | 1% |
| 532 | `asklistscroll` | 2 | 40 | 5% |
| 199 | `getcolval` | 2 | 37 | 5% |
| 472 | `clnicezputs` | 25 | 104 | 24% |

### `src/ported/zle/zle_params.rs` — 4 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 779 | `get_zle_state` | 9 | 46 | 19% |
| 536 | `set_killring` | 6 | 27 | 22% |
| 206 | `set_rbuffer` | 4 | 14 | 28% |
| 188 | `set_lbuffer` | 5 | 17 | 29% |

### `src/ported/zle/zle_thingy.rs` — 4 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1062 | `bin_zle_call` | 7 | 95 | 7% |
| 697 | `bin_zle_refresh` | 6 | 31 | 19% |
| 1125 | `bin_zle_fd` | 21 | 81 | 25% |
| 1024 | `bin_zle_flags` | 11 | 39 | 28% |

### `src/ported/zle/zle_vi.rs` — 4 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 103 | `vigetkey` | 5 | 32 | 15% |
| 147 | `dovilinerange` | 4 | 23 | 17% |
| 409 | `vireplacechars` | 14 | 62 | 22% |
| 231 | `videletechar` | 5 | 19 | 26% |

### `src/ported/zle/termquery.rs` — 3 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 235 | `handle_color` | 1 | 24 | 4% |
| 249 | `query_terminal` | 7 | 42 | 16% |
| 167 | `probe_terminal` | 46 | 205 | 22% |

### `src/ported/zle/zle_keymap.rs` — 3 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1233 | `getrestchar_keybuf` | 2 | 39 | 5% |
| 1205 | `default_bindings` | 17 | 121 | 14% |
| 885 | `bin_bindkey_lsmaps` | 4 | 16 | 25% |

### `src/ported/zle/zle_move.rs` — 3 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 143 | `backwardmetafiedchar` | 3 | 73 | 4% |
| 213 | `beginningoflinehist` | 7 | 32 | 21% |
| 228 | `endoflinehist` | 7 | 30 | 23% |

### `src/ported/compat.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 69 | `zgettime_monotonic_if_available` | 23 | 309 | 7% |
| 217 | `zgetdir` | 15 | 120 | 12% |

### `src/ported/input.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 293 | `inputline` | 5 | 88 | 5% |
| 420 | `inpoptop` | 8 | 28 | 28% |

### `src/ported/mem.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 229 | `hrealloc` | 3 | 102 | 2% |
| 311 | `realloc` | 2 | 32 | 6% |

### `src/ported/modules/curses.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1028 | `zccmd_input` | 33 | 162 | 20% |
| 611 | `zccmd_delwin` | 12 | 44 | 27% |

### `src/ported/zle/compcore.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1240 | `set_comp_sep` | 9 | 319 | 2% |
| 514 | `callcompfunc` | 46 | 330 | 13% |

### `src/ported/zle/zle_misc.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1519 | `makesuffixstr` | 3 | 42 | 7% |
| 1477 | `addsuffix` | 3 | 12 | 25% |

### `src/ported/math.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1339 | `setmathvar` | 7 | 44 | 15% |

### `src/ported/module.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 36 | `printmodulenode` | 10 | 89 | 11% |

### `src/ported/modules/files.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 23 | `recursivecmd` | 12 | 59 | 20% |

### `src/ported/modules/mathfunc.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 55 | `math_string` | 12 | 66 | 18% |

### `src/ported/modules/pcre.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 324 | `cond_pcre_match` | 19 | 70 | 27% |

### `src/ported/modules/termcap.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 212 | `scantermcap` | 8 | 78 | 10% |

### `src/ported/modules/watch.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 411 | `readwtab` | 20 | 169 | 11% |

### `src/ported/modules/zprof.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 314 | `name_for_anonymous_function` | 1 | 11 | 9% |

### `src/ported/modules/zpty.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 236 | `newptycmd` | 18 | 118 | 15% |

### `src/ported/options.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 155 | `createoptiontable` | 4 | 16 | 25% |

### `src/ported/params.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 4147 | `assignaparam` | 45 | 187 | 24% |

### `src/ported/zle/compmatch.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 2032 | `bld_line` | 40 | 137 | 29% |

### `src/ported/zle/computil.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 6654 | `cfp_add_sdirs` | 18 | 73 | 24% |

