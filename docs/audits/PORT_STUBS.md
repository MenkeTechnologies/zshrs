# PORT_STUBS — stubs detected in src/ported/

Generated: 2026-06-15T21:44:53.973116+00:00

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

## Summary: 116 stubs across 44 files

| File | Stubs | Worst (Rust / C lines) |
|---|---|---|
| `src/ported/hashtable.rs` | 11 | `newhashtable` (1 / 19) |
| `src/ported/mem.rs` | 10 | `malloc` (1 / 165) |
| `src/ported/utils.rs` | 9 | `mb_metacharlenconv_r` (5 / 32) |
| `src/ported/zle/compresult.rs` | 8 | `do_single` (4 / 180) |
| `src/ported/zle/compctl.rs` | 6 | `makecomplistflags` (70 / 745) |
| `src/ported/zle/zle_main.rs` | 5 | `describekeybriefly` (2 / 28) |
| `src/ported/zle/zle_refresh.rs` | 4 | `zrefresh` (87 / 658) |
| `src/ported/zle/zle_utils.rs` | 4 | `showmsg` (4 / 72) |
| `src/ported/module.rs` | 3 | `load_and_bind` (1 / 20) |
| `src/ported/modules/zutil.rs` | 3 | `newzstyletable` (1 / 15) |
| `src/ported/parse.rs` | 3 | `build_cur_dump` (5 / 80) |
| `src/ported/zle/compcore.rs` | 3 | `set_comp_sep` (9 / 321) |
| `src/ported/zle/zle_keymap.rs` | 3 | `newkeytab` (1 / 15) |
| `src/ported/zle/zle_tricky.rs` | 3 | `get_comp_string` (28 / 794) |
| `src/ported/compat.rs` | 2 | `zgettime_monotonic_if_available` (23 / 404) |
| `src/ported/exec.rs` | 2 | `namedpipe` (24 / 1085) |
| `src/ported/glob.rs` | 2 | `zglob` (60 / 614) |
| `src/ported/hist.rs` | 2 | `lockhistfile` (25 / 124) |
| `src/ported/init.rs` | 2 | `init_shout` (4 / 23) |
| `src/ported/modules/db_gdbm.rs` | 2 | `unmetafy_zalloc` (3 / 11) |
| `src/ported/prompt.rs` | 2 | `match_highlight` (10 / 75) |
| `src/ported/zle/complete.rs` | 2 | `set_compstate` (3 / 26) |
| `src/ported/zle/complist.rs` | 2 | `domenuselect` (125 / 940) |
| `src/ported/zle/zle_hist.rs` | 2 | `doisearch` (19 / 462) |
| `src/ported/zle/zle_params.rs` | 2 | `get_cursor` (1 / 11) |
| `src/ported/builtin.rs` | 1 | `cd_new_pwd` (10 / 71) |
| `src/ported/builtins/rlimits.rs` | 1 | `printrlim` (1 / 13) |
| `src/ported/hashnameddir.rs` | 1 | `createnameddirtable` (2 / 15) |
| `src/ported/modules/curses.rs` | 1 | `zccmd_input` (36 / 173) |
| `src/ported/modules/param_private.rs` | 1 | `setup_` (1 / 12) |
| `src/ported/modules/parameter.rs` | 1 | `setfunctions` (4 / 15) |
| `src/ported/modules/pcre.rs` | 1 | `pcre_callout` (1 / 15) |
| `src/ported/modules/watch.rs` | 1 | `readwtab` (24 / 183) |
| `src/ported/modules/zpty.rs` | 1 | `ptysettyinfo` (6 / 21) |
| `src/ported/options.rs` | 1 | `createoptiontable` (4 / 16) |
| `src/ported/pattern.rs` | 1 | `charsub` (5 / 17) |
| `src/ported/signals.rs` | 1 | `wait_for_processes` (11 / 102) |
| `src/ported/zle/compmatch.rs` | 1 | `bld_line` (40 / 139) |
| `src/ported/zle/computil.rs` | 1 | `cfp_add_sdirs` (18 / 73) |
| `src/ported/zle/deltochar.rs` | 1 | `boot_` (1 / 11) |
| `src/ported/zle/termquery.rs` | 1 | `probe_terminal` (46 / 208) |
| `src/ported/zle/zle_move.rs` | 1 | `backwardmetafiedchar` (3 / 75) |
| `src/ported/zle/zle_thingy.rs` | 1 | `createthingytab` (1 / 13) |
| `src/ported/zle/zle_vi.rs` | 1 | `getvirange` (4 / 83) |

## Per-file detail

### `src/ported/hashtable.rs` — 11 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 85 | `newhashtable` | 1 | 19 | 5% |
| 2131 | `createreswdtable` | 1 | 16 | 6% |
| 1017 | `hashdir` | 4 | 58 | 6% |
| 984 | `createcmdnamtable` | 1 | 14 | 7% |
| 1186 | `createshfunctable` | 1 | 13 | 7% |
| 2488 | `createhisttable` | 1 | 13 | 7% |
| 889 | `expandhashtable` | 2 | 15 | 13% |
| 900 | `resizehashtable` | 4 | 18 | 22% |
| 1298 | `freeshfuncnode` | 4 | 17 | 23% |
| 2685 | `addhistnode` | 4 | 17 | 23% |
| 929 | `printhashtabinfo` | 6 | 22 | 27% |

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

### `src/ported/utils.rs` — 9 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 6802 | `mb_metacharlenconv_r` | 5 | 32 | 15% |
| 6843 | `mb_metastrlenend` | 8 | 48 | 16% |
| 100 | `set_widearray` | 6 | 30 | 20% |
| 5990 | `metafy` | 11 | 54 | 20% |
| 6137 | `unmeta` | 8 | 35 | 22% |
| 1782 | `checkmailpath` | 19 | 75 | 25% |
| 3006 | `read_poll` | 16 | 62 | 25% |
| 3030 | `timespec_diff_us` | 5 | 18 | 27% |
| 1493 | `delprepromptfn` | 4 | 14 | 28% |

### `src/ported/zle/compresult.rs` — 8 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 349 | `do_single` | 4 | 180 | 2% |
| 250 | `hasbrpsfx` | 1 | 39 | 2% |
| 379 | `valid_match` | 1 | 32 | 3% |
| 329 | `do_allmatches` | 2 | 49 | 4% |
| 166 | `build_pos_string` | 1 | 23 | 4% |
| 71 | `cut_cline` | 5 | 81 | 6% |
| 218 | `instmatch` | 6 | 79 | 7% |
| 99 | `cline_str` | 50 | 271 | 18% |

### `src/ported/zle/compctl.rs` — 6 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 2792 | `makecomplistflags` | 70 | 745 | 9% |
| 2233 | `makecomplistctl` | 12 | 74 | 16% |
| 1845 | `addmatch` | 17 | 103 | 16% |
| 92 | `createcompctltable` | 4 | 14 | 28% |
| 2277 | `makecomplistext` | 43 | 150 | 28% |
| 242 | `freecompcond` | 11 | 37 | 29% |

### `src/ported/zle/zle_main.rs` — 5 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1125 | `describekeybriefly` | 2 | 28 | 7% |
| 406 | `getbyte` | 10 | 76 | 13% |
| 317 | `raw_getbyte` | 46 | 242 | 19% |
| 651 | `zleread` | 29 | 127 | 22% |
| 69 | `zsetterm` | 31 | 104 | 29% |

### `src/ported/zle/zle_refresh.rs` — 4 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 914 | `zrefresh` | 87 | 658 | 13% |
| 570 | `freevideo` | 4 | 19 | 21% |
| 1695 | `tc_rightcurs` | 15 | 63 | 23% |
| 1608 | `moveto` | 13 | 44 | 29% |

### `src/ported/zle/zle_utils.rs` — 4 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1255 | `showmsg` | 4 | 72 | 5% |
| 47 | `sizeline` | 5 | 20 | 25% |
| 530 | `zle_free_positions` | 3 | 11 | 27% |
| 544 | `spaceinline` | 16 | 54 | 29% |

### `src/ported/module.rs` — 3 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 3051 | `load_and_bind` | 1 | 20 | 5% |
| 3400 | `module_func` | 1 | 16 | 6% |
| 3083 | `try_load_module` | 4 | 14 | 28% |

### `src/ported/modules/zutil.rs` — 3 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 741 | `newzstyletable` | 1 | 15 | 6% |
| 758 | `setstypat` | 6 | 58 | 10% |
| 166 | `freestylenode` | 2 | 10 | 20% |

### `src/ported/parse.rs` — 3 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 4495 | `build_cur_dump` | 5 | 80 | 6% |
| 2276 | `par_subsh` | 6 | 34 | 17% |
| 4527 | `load_dump_file` | 12 | 47 | 25% |

### `src/ported/zle/compcore.rs` — 3 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1505 | `set_comp_sep` | 9 | 321 | 2% |
| 3355 | `freematches` | 6 | 26 | 23% |
| 587 | `callcompfunc` | 103 | 345 | 29% |

### `src/ported/zle/zle_keymap.rs` — 3 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 287 | `newkeytab` | 1 | 15 | 6% |
| 176 | `emptykeymapnamtab` | 1 | 14 | 7% |
| 134 | `createkeymapnamtab` | 1 | 13 | 7% |

### `src/ported/zle/zle_tricky.rs` — 3 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1058 | `get_comp_string` | 28 | 794 | 3% |
| 1386 | `pfxlen` | 4 | 38 | 10% |
| 1482 | `listlist` | 32 | 174 | 18% |

### `src/ported/compat.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 69 | `zgettime_monotonic_if_available` | 23 | 404 | 5% |
| 262 | `zgetdir` | 15 | 146 | 10% |

### `src/ported/exec.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 3266 | `namedpipe` | 24 | 1085 | 2% |
| 7860 | `execpline` | 68 | 237 | 28% |

### `src/ported/glob.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1249 | `zglob` | 60 | 614 | 9% |
| 381 | `scanner` | 31 | 162 | 19% |

### `src/ported/hist.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 3791 | `lockhistfile` | 25 | 124 | 20% |
| 3652 | `savehistfile` | 48 | 199 | 24% |

### `src/ported/init.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 643 | `init_shout` | 4 | 23 | 17% |
| 1511 | `source` | 22 | 97 | 22% |

### `src/ported/modules/db_gdbm.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1231 | `unmetafy_zalloc` | 3 | 11 | 27% |
| 653 | `gdbmhashsetfn` | 13 | 47 | 27% |

### `src/ported/prompt.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 3281 | `match_highlight` | 10 | 75 | 13% |
| 373 | `promptexpand` | 7 | 43 | 16% |

### `src/ported/zle/complete.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1618 | `set_compstate` | 3 | 26 | 11% |
| 853 | `bin_compadd` | 38 | 233 | 16% |

### `src/ported/zle/complist.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 2853 | `domenuselect` | 125 | 940 | 13% |
| 766 | `clnicezputs` | 24 | 107 | 22% |

### `src/ported/zle/zle_hist.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1415 | `doisearch` | 19 | 462 | 4% |
| 1534 | `getvisrchstr` | 6 | 118 | 5% |

### `src/ported/zle/zle_params.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 144 | `get_cursor` | 1 | 11 | 9% |
| 608 | `set_killring` | 6 | 27 | 22% |

### `src/ported/builtin.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 2265 | `cd_new_pwd` | 10 | 71 | 14% |

### `src/ported/builtins/rlimits.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 274 | `printrlim` | 1 | 13 | 7% |

### `src/ported/hashnameddir.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 35 | `createnameddirtable` | 2 | 15 | 13% |

### `src/ported/modules/curses.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1314 | `zccmd_input` | 36 | 173 | 20% |

### `src/ported/modules/param_private.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1130 | `setup_` | 1 | 12 | 8% |

### `src/ported/modules/parameter.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 963 | `setfunctions` | 4 | 15 | 26% |

### `src/ported/modules/pcre.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 213 | `pcre_callout` | 1 | 15 | 6% |

### `src/ported/modules/watch.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 559 | `readwtab` | 24 | 183 | 13% |

### `src/ported/modules/zpty.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 148 | `ptysettyinfo` | 6 | 21 | 28% |

### `src/ported/options.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 170 | `createoptiontable` | 4 | 16 | 25% |

### `src/ported/pattern.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 2658 | `charsub` | 5 | 17 | 29% |

### `src/ported/signals.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 322 | `wait_for_processes` | 11 | 102 | 10% |

### `src/ported/zle/compmatch.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 2304 | `bld_line` | 40 | 139 | 28% |

### `src/ported/zle/computil.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 7493 | `cfp_add_sdirs` | 18 | 73 | 24% |

### `src/ported/zle/deltochar.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 141 | `boot_` | 1 | 11 | 9% |

### `src/ported/zle/termquery.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 150 | `probe_terminal` | 46 | 208 | 22% |

### `src/ported/zle/zle_move.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 243 | `backwardmetafiedchar` | 3 | 75 | 4% |

### `src/ported/zle/zle_thingy.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 48 | `createthingytab` | 1 | 13 | 7% |

### `src/ported/zle/zle_vi.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 269 | `getvirange` | 4 | 83 | 4% |

