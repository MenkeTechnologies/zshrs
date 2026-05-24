# PORT_STUBS — stubs detected in src/ported/

Generated: 2026-05-24T21:41:39.873750+00:00

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

## Summary: 171 stubs across 48 files

| File | Stubs | Worst (Rust / C lines) |
|---|---|---|
| `src/ported/utils.rs` | 14 | `lchdir` (8 / 327) |
| `src/ported/hashtable.rs` | 11 | `newhashtable` (1 / 19) |
| `src/ported/mem.rs` | 10 | `malloc` (1 / 165) |
| `src/ported/zle/complist.rs` | 10 | `putfilecol` (2 / 71) |
| `src/ported/zle/zle_refresh.rs` | 10 | `addmultiword` (1 / 14) |
| `src/ported/zle/compresult.rs` | 8 | `do_single` (4 / 182) |
| `src/ported/zle/zle_keymap.rs` | 8 | `getrestchar_keybuf` (2 / 39) |
| `src/ported/zle/zle_utils.rs` | 8 | `showmsg` (4 / 72) |
| `src/ported/zle/compctl.rs` | 6 | `makecomplistflags` (19 / 746) |
| `src/ported/zle/zle_main.rs` | 6 | `describekeybriefly` (2 / 28) |
| `src/ported/zle/zle_tricky.rs` | 6 | `doexpansion` (1 / 58) |
| `src/ported/module.rs` | 5 | `load_and_bind` (1 / 20) |
| `src/ported/modules/zutil.rs` | 5 | `map_opt_desc` (1 / 16) |
| `src/ported/init.rs` | 4 | `init_term` (9 / 91) |
| `src/ported/parse.rs` | 4 | `build_dump` (2 / 75) |
| `src/ported/prompt.rs` | 4 | `addbufspc` (1 / 15) |
| `src/ported/zle/zle_vi.rs` | 4 | `getvirange` (4 / 82) |
| `src/ported/glob.rs` | 3 | `get_match_ret` (5 / 71) |
| `src/ported/modules/db_gdbm.rs` | 3 | `gdbmhashsetfn` (7 / 47) |
| `src/ported/pattern.rs` | 3 | `patmatch` (4 / 575) |
| `src/ported/zle/zle_move.rs` | 3 | `backwardmetafiedchar` (3 / 75) |
| `src/ported/builtin.rs` | 2 | `bin_print` (202 / 812) |
| `src/ported/compat.rs` | 2 | `zgettime_monotonic_if_available` (23 / 404) |
| `src/ported/exec.rs` | 2 | `namedpipe` (24 / 1076) |
| `src/ported/hist.rs` | 2 | `lockhistfile` (25 / 124) |
| `src/ported/modules/curses.rs` | 2 | `zccmd_input` (36 / 179) |
| `src/ported/modules/zftp.rs` | 2 | `newsession` (1 / 18) |
| `src/ported/modules/zpty.rs` | 2 | `newptycmd` (20 / 147) |
| `src/ported/zle/compcore.rs` | 2 | `set_comp_sep` (9 / 321) |
| `src/ported/zle/zle_hist.rs` | 2 | `doisearch` (19 / 462) |
| `src/ported/builtins/rlimits.rs` | 1 | `printrlim` (1 / 13) |
| `src/ported/hashnameddir.rs` | 1 | `createnameddirtable` (2 / 15) |
| `src/ported/input.rs` | 1 | `inputline` (5 / 88) |
| `src/ported/jobs.rs` | 1 | `addfilelist` (4 / 15) |
| `src/ported/modules/hlgroup.rs` | 1 | `scangroup` (1 / 20) |
| `src/ported/modules/mathfunc.rs` | 1 | `math_string` (16 / 68) |
| `src/ported/modules/param_private.rs` | 1 | `setup_` (1 / 12) |
| `src/ported/modules/pcre.rs` | 1 | `pcre_callout` (1 / 15) |
| `src/ported/modules/watch.rs` | 1 | `readwtab` (24 / 189) |
| `src/ported/options.rs` | 1 | `createoptiontable` (4 / 16) |
| `src/ported/signals.rs` | 1 | `wait_for_processes` (11 / 102) |
| `src/ported/zle/complete.rs` | 1 | `set_compstate` (3 / 26) |
| `src/ported/zle/compmatch.rs` | 1 | `bld_line` (40 / 139) |
| `src/ported/zle/computil.rs` | 1 | `cfp_add_sdirs` (18 / 73) |
| `src/ported/zle/deltochar.rs` | 1 | `boot_` (1 / 11) |
| `src/ported/zle/termquery.rs` | 1 | `probe_terminal` (46 / 208) |
| `src/ported/zle/zle_params.rs` | 1 | `get_cursor` (1 / 11) |
| `src/ported/zle/zle_thingy.rs` | 1 | `createthingytab` (1 / 13) |

## Per-file detail

### `src/ported/utils.rs` — 14 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 7564 | `lchdir` | 8 | 327 | 2% |
| 4366 | `spacesplit` | 5 | 33 | 15% |
| 6359 | `mb_metacharlenconv_r` | 5 | 32 | 15% |
| 4381 | `findsep` | 7 | 42 | 16% |
| 6399 | `mb_metastrlenend` | 8 | 48 | 16% |
| 89 | `set_widearray` | 6 | 30 | 20% |
| 5562 | `metafy` | 11 | 54 | 20% |
| 5704 | `unmeta` | 8 | 35 | 22% |
| 2936 | `read_poll` | 16 | 62 | 25% |
| 1722 | `checkmailpath` | 19 | 73 | 26% |
| 5196 | `itype_end` | 18 | 67 | 26% |
| 2960 | `timespec_diff_us` | 5 | 18 | 27% |
| 5900 | `zreaddir` | 9 | 32 | 28% |
| 1448 | `delprepromptfn` | 4 | 14 | 28% |

### `src/ported/hashtable.rs` — 11 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 73 | `newhashtable` | 1 | 19 | 5% |
| 1536 | `createreswdtable` | 1 | 16 | 6% |
| 987 | `hashdir` | 4 | 58 | 6% |
| 954 | `createcmdnamtable` | 1 | 14 | 7% |
| 1156 | `createshfunctable` | 1 | 13 | 7% |
| 1893 | `createhisttable` | 1 | 13 | 7% |
| 859 | `expandhashtable` | 2 | 15 | 13% |
| 870 | `resizehashtable` | 4 | 18 | 22% |
| 1255 | `freeshfuncnode` | 4 | 17 | 23% |
| 2090 | `addhistnode` | 4 | 17 | 23% |
| 899 | `printhashtabinfo` | 6 | 22 | 27% |

### `src/ported/mem.rs` — 10 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 285 | `malloc` | 1 | 165 | 0% |
| 228 | `zhalloc` | 1 | 81 | 1% |
| 170 | `freeheap` | 1 | 73 | 1% |
| 178 | `popheap` | 1 | 67 | 1% |
| 263 | `hrealloc` | 3 | 140 | 2% |
| 393 | `realloc` | 1 | 32 | 3% |
| 162 | `pushheap` | 1 | 23 | 4% |
| 133 | `old_heaps` | 3 | 38 | 7% |
| 304 | `zalloc` | 1 | 10 | 10% |
| 244 | `memory_validate` | 5 | 36 | 13% |

### `src/ported/zle/complist.rs` — 10 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 699 | `putfilecol` | 2 | 71 | 2% |
| 2745 | `menuselect_bindings` | 1 | 25 | 4% |
| 716 | `asklistscroll` | 2 | 40 | 5% |
| 198 | `getcolval` | 2 | 37 | 5% |
| 1679 | `singlecalc` | 1 | 15 | 6% |
| 2338 | `msearchpop` | 1 | 10 | 10% |
| 688 | `putmatchcol` | 2 | 15 | 13% |
| 2759 | `boot_` | 2 | 15 | 13% |
| 2508 | `domenuselect` | 129 | 925 | 13% |
| 653 | `clnicezputs` | 24 | 107 | 22% |

### `src/ported/zle/zle_refresh.rs` — 10 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 629 | `addmultiword` | 1 | 14 | 7% |
| 1877 | `singmoveto` | 1 | 12 | 8% |
| 526 | `freevideo` | 2 | 19 | 10% |
| 541 | `resetvideo` | 7 | 53 | 13% |
| 1437 | `tcout_via_func` | 5 | 34 | 14% |
| 652 | `zrefresh` | 93 | 602 | 15% |
| 604 | `snextline` | 5 | 27 | 18% |
| 1393 | `tc_rightcurs` | 15 | 63 | 23% |
| 586 | `nextline` | 6 | 22 | 27% |
| 1357 | `moveto` | 13 | 44 | 29% |

### `src/ported/zle/compresult.rs` — 8 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 346 | `do_single` | 4 | 182 | 2% |
| 247 | `hasbrpsfx` | 1 | 39 | 2% |
| 376 | `valid_match` | 1 | 32 | 3% |
| 326 | `do_allmatches` | 2 | 49 | 4% |
| 163 | `build_pos_string` | 1 | 23 | 4% |
| 68 | `cut_cline` | 5 | 81 | 6% |
| 215 | `instmatch` | 6 | 80 | 7% |
| 96 | `cline_str` | 50 | 271 | 18% |

### `src/ported/zle/zle_keymap.rs` — 8 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1524 | `getrestchar_keybuf` | 2 | 39 | 5% |
| 264 | `newkeytab` | 1 | 15 | 6% |
| 153 | `emptykeymapnamtab` | 1 | 14 | 7% |
| 111 | `createkeymapnamtab` | 1 | 13 | 7% |
| 567 | `scankeys` | 1 | 13 | 7% |
| 1467 | `add_cursor_key` | 3 | 18 | 16% |
| 1488 | `default_bindings` | 24 | 121 | 19% |
| 1225 | `bin_bindkey_meta` | 4 | 19 | 21% |

### `src/ported/zle/zle_utils.rs` — 8 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 841 | `showmsg` | 4 | 72 | 5% |
| 73 | `zlecharasstring` | 3 | 43 | 6% |
| 95 | `stringaszleline` | 14 | 109 | 12% |
| 332 | `cuttext` | 17 | 79 | 21% |
| 165 | `zle_save_positions` | 8 | 34 | 23% |
| 181 | `zle_restore_positions` | 12 | 51 | 23% |
| 210 | `spaceinline` | 14 | 54 | 25% |
| 47 | `sizeline` | 5 | 18 | 27% |

### `src/ported/zle/compctl.rs` — 6 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 2793 | `makecomplistflags` | 19 | 746 | 2% |
| 2236 | `makecomplistctl` | 12 | 74 | 16% |
| 1846 | `addmatch` | 17 | 99 | 17% |
| 92 | `createcompctltable` | 4 | 14 | 28% |
| 2280 | `makecomplistext` | 43 | 150 | 28% |
| 242 | `freecompcond` | 11 | 37 | 29% |

### `src/ported/zle/zle_main.rs` — 6 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1088 | `describekeybriefly` | 2 | 28 | 7% |
| 414 | `getbyte` | 11 | 76 | 14% |
| 778 | `execzlefunc` | 27 | 149 | 18% |
| 1327 | `trashzle` | 4 | 22 | 18% |
| 685 | `zleread` | 26 | 127 | 20% |
| 321 | `raw_getbyte` | 50 | 242 | 20% |

### `src/ported/zle/zle_tricky.rs` — 6 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 807 | `doexpansion` | 1 | 58 | 1% |
| 699 | `get_comp_string` | 21 | 794 | 2% |
| 835 | `pfxlen` | 4 | 38 | 10% |
| 382 | `docomplete` | 32 | 210 | 15% |
| 931 | `listlist` | 32 | 174 | 18% |
| 195 | `expandorcomplete` | 4 | 16 | 25% |

### `src/ported/module.rs` — 5 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1717 | `load_and_bind` | 1 | 20 | 5% |
| 2008 | `module_func` | 1 | 16 | 6% |
| 42 | `printmodulenode` | 10 | 89 | 11% |
| 3123 | `bin_zmodload_features` | 30 | 217 | 13% |
| 3028 | `unload_named_module` | 5 | 36 | 13% |

### `src/ported/modules/zutil.rs` — 5 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 2159 | `map_opt_desc` | 1 | 16 | 6% |
| 677 | `newzstyletable` | 1 | 15 | 6% |
| 2136 | `lookup_opt` | 1 | 13 | 7% |
| 694 | `setstypat` | 6 | 58 | 10% |
| 148 | `freestylenode` | 2 | 10 | 20% |

### `src/ported/init.rs` — 4 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 473 | `init_term` | 9 | 91 | 9% |
| 444 | `init_shout` | 4 | 23 | 17% |
| 1003 | `source` | 18 | 97 | 18% |
| 284 | `parseopts` | 36 | 144 | 25% |

### `src/ported/parse.rs` — 4 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 3521 | `build_dump` | 2 | 75 | 2% |
| 3582 | `build_cur_dump` | 5 | 80 | 6% |
| 1714 | `par_subsh` | 6 | 34 | 17% |
| 3614 | `load_dump_file` | 12 | 47 | 25% |

### `src/ported/prompt.rs` — 4 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 483 | `addbufspc` | 1 | 15 | 6% |
| 345 | `promptexpand` | 4 | 43 | 9% |
| 438 | `parsecolorchar` | 3 | 24 | 12% |
| 2511 | `match_highlight` | 10 | 75 | 13% |

### `src/ported/zle/zle_vi.rs` — 4 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 227 | `getvirange` | 4 | 82 | 4% |
| 745 | `virepeatchange` | 1 | 16 | 6% |
| 580 | `vireplacechars` | 16 | 62 | 25% |
| 68 | `startvichange` | 6 | 22 | 27% |

### `src/ported/glob.rs` — 3 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1655 | `get_match_ret` | 5 | 71 | 7% |
| 1296 | `zglob` | 44 | 612 | 7% |
| 579 | `scanner` | 17 | 162 | 10% |

### `src/ported/modules/db_gdbm.rs` — 3 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 490 | `gdbmhashsetfn` | 7 | 47 | 14% |
| 531 | `gdbmuntie` | 3 | 12 | 25% |
| 986 | `unmetafy_zalloc` | 3 | 11 | 27% |

### `src/ported/pattern.rs` — 3 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1965 | `patmatch` | 4 | 575 | 0% |
| 1888 | `pattryrefs` | 41 | 202 | 20% |
| 2002 | `patmatchrange` | 26 | 113 | 23% |

### `src/ported/zle/zle_move.rs` — 3 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 134 | `backwardmetafiedchar` | 3 | 75 | 4% |
| 36 | `alignmultiwordleft` | 1 | 16 | 6% |
| 47 | `alignmultiwordright` | 1 | 13 | 7% |

### `src/ported/builtin.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 6364 | `bin_print` | 202 | 812 | 24% |
| 7882 | `bin_read` | 168 | 583 | 28% |

### `src/ported/compat.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 69 | `zgettime_monotonic_if_available` | 23 | 404 | 5% |
| 225 | `zgetdir` | 15 | 146 | 10% |

### `src/ported/exec.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 3135 | `namedpipe` | 24 | 1076 | 2% |
| 7266 | `execpline` | 64 | 237 | 27% |

### `src/ported/hist.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 3705 | `lockhistfile` | 25 | 124 | 20% |
| 3618 | `savehistfile` | 43 | 199 | 21% |

### `src/ported/modules/curses.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1065 | `zccmd_input` | 36 | 179 | 20% |
| 636 | `zccmd_delwin` | 12 | 44 | 27% |

### `src/ported/modules/zftp.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 3033 | `newsession` | 1 | 18 | 5% |
| 300 | `zfmovefd` | 1 | 10 | 10% |

### `src/ported/modules/zpty.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 240 | `newptycmd` | 20 | 147 | 13% |
| 131 | `ptysettyinfo` | 6 | 21 | 28% |

### `src/ported/zle/compcore.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1457 | `set_comp_sep` | 9 | 321 | 2% |
| 607 | `callcompfunc` | 63 | 345 | 18% |

### `src/ported/zle/zle_hist.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1452 | `doisearch` | 19 | 462 | 4% |
| 1563 | `getvisrchstr` | 6 | 118 | 5% |

### `src/ported/builtins/rlimits.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 269 | `printrlim` | 1 | 13 | 7% |

### `src/ported/hashnameddir.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 35 | `createnameddirtable` | 2 | 15 | 13% |

### `src/ported/input.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 320 | `inputline` | 5 | 88 | 5% |

### `src/ported/jobs.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1296 | `addfilelist` | 4 | 15 | 26% |

### `src/ported/modules/hlgroup.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 285 | `scangroup` | 1 | 20 | 5% |

### `src/ported/modules/mathfunc.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 58 | `math_string` | 16 | 68 | 23% |

### `src/ported/modules/param_private.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1011 | `setup_` | 1 | 12 | 8% |

### `src/ported/modules/pcre.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 167 | `pcre_callout` | 1 | 15 | 6% |

### `src/ported/modules/watch.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 452 | `readwtab` | 24 | 189 | 12% |

### `src/ported/options.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 171 | `createoptiontable` | 4 | 16 | 25% |

### `src/ported/signals.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 326 | `wait_for_processes` | 11 | 102 | 10% |

### `src/ported/zle/complete.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1535 | `set_compstate` | 3 | 26 | 11% |

### `src/ported/zle/compmatch.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 2335 | `bld_line` | 40 | 139 | 28% |

### `src/ported/zle/computil.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 7507 | `cfp_add_sdirs` | 18 | 73 | 24% |

### `src/ported/zle/deltochar.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 174 | `boot_` | 1 | 11 | 9% |

### `src/ported/zle/termquery.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 151 | `probe_terminal` | 46 | 208 | 22% |

### `src/ported/zle/zle_params.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 154 | `get_cursor` | 1 | 11 | 9% |

### `src/ported/zle/zle_thingy.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 49 | `createthingytab` | 1 | 13 | 7% |

