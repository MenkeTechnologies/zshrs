# PORT_STUBS — stubs detected in src/ported/

Generated: 2026-05-14T21:33:46.579479+00:00

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

## Summary: 63 stubs across 36 files

| File | Stubs | Worst (Rust / C lines) |
|---|---|---|
| `src/ported/modules/db_gdbm.rs` | 4 | `gdbmgetfn` (7 / 28) |
| `src/ported/zle/complist.rs` | 4 | `compprintfmt` (4 / 232) |
| `src/ported/zle/zle_refresh.rs` | 4 | `moveto` (2 / 44) |
| `src/ported/zle/compctl.rs` | 3 | `makecomplistflags` (19 / 746) |
| `src/ported/zle/termquery.rs` | 3 | `handle_color` (1 / 24) |
| `src/ported/zle/zle_hist.rs` | 3 | `doisearch` (12 / 454) |
| `src/ported/zle/zle_keymap.rs` | 3 | `getrestchar_keybuf` (2 / 39) |
| `src/ported/zle/zle_move.rs` | 3 | `backwardmetafiedchar` (3 / 73) |
| `src/ported/builtin.rs` | 2 | `bin_print` (67 / 777) |
| `src/ported/compat.rs` | 2 | `zgettime_monotonic_if_available` (23 / 309) |
| `src/ported/input.rs` | 2 | `inputline` (5 / 88) |
| `src/ported/modules/curses.rs` | 2 | `zccmd_input` (33 / 162) |
| `src/ported/zle/compcore.rs` | 2 | `set_comp_sep` (9 / 319) |
| `src/ported/zle/zle_misc.rs` | 2 | `makesuffixstr` (3 / 42) |
| `src/ported/zle/zle_tricky.rs` | 2 | `pfxlen` (4 / 35) |
| `src/ported/zle/zle_vi.rs` | 2 | `dovilinerange` (4 / 23) |
| `src/ported/glob.rs` | 1 | `get_match_ret` (5 / 71) |
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
| `src/ported/parse.rs` | 1 | `check_dump_file` (1 / 83) |
| `src/ported/pattern.rs` | 1 | `patmatchrange` (21 / 110) |
| `src/ported/prompt.rs` | 1 | `parsecolorchar` (3 / 24) |
| `src/ported/utils.rs` | 1 | `lchdir` (8 / 298) |
| `src/ported/zle/compmatch.rs` | 1 | `bld_line` (40 / 137) |
| `src/ported/zle/compresult.rs` | 1 | `cline_str` (45 / 268) |
| `src/ported/zle/computil.rs` | 1 | `cfp_add_sdirs` (18 / 73) |
| `src/ported/zle/zle_main.rs` | 1 | `getrestchar` (7 / 36) |

## Per-file detail

### `src/ported/modules/db_gdbm.rs` — 4 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 248 | `gdbmgetfn` | 7 | 28 | 25% |
| 267 | `gdbmsetfn` | 7 | 28 | 25% |
| 477 | `gdbmuntie` | 3 | 12 | 25% |
| 927 | `unmetafy_zalloc` | 3 | 11 | 27% |

### `src/ported/zle/complist.rs` — 4 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 568 | `compprintfmt` | 4 | 232 | 1% |
| 532 | `asklistscroll` | 2 | 40 | 5% |
| 199 | `getcolval` | 2 | 37 | 5% |
| 472 | `clnicezputs` | 25 | 104 | 24% |

### `src/ported/zle/zle_refresh.rs` — 4 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 816 | `moveto` | 2 | 44 | 4% |
| 839 | `tc_rightcurs` | 4 | 61 | 6% |
| 613 | `zrefresh` | 87 | 566 | 15% |
| 478 | `zwcputc` | 3 | 19 | 15% |

### `src/ported/zle/compctl.rs` — 3 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 2463 | `makecomplistflags` | 19 | 746 | 2% |
| 1581 | `addmatch` | 17 | 99 | 17% |
| 1974 | `makecomplistext` | 36 | 150 | 24% |

### `src/ported/zle/termquery.rs` — 3 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 235 | `handle_color` | 1 | 24 | 4% |
| 249 | `query_terminal` | 7 | 42 | 16% |
| 167 | `probe_terminal` | 46 | 205 | 22% |

### `src/ported/zle/zle_hist.rs` — 3 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1008 | `doisearch` | 12 | 454 | 2% |
| 1102 | `getvisrchstr` | 6 | 115 | 5% |
| 959 | `isearch_newpos` | 2 | 19 | 10% |

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

### `src/ported/builtin.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 4368 | `bin_print` | 67 | 777 | 8% |
| 5197 | `bin_read` | 111 | 533 | 20% |

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

### `src/ported/zle/zle_tricky.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 765 | `pfxlen` | 4 | 35 | 11% |
| 855 | `listlist` | 33 | 174 | 18% |

### `src/ported/zle/zle_vi.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 147 | `dovilinerange` | 4 | 23 | 17% |
| 231 | `videletechar` | 5 | 19 | 26% |

### `src/ported/glob.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 844 | `get_match_ret` | 5 | 71 | 7% |

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

### `src/ported/parse.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 3499 | `check_dump_file` | 1 | 83 | 1% |

### `src/ported/pattern.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1110 | `patmatchrange` | 21 | 110 | 19% |

### `src/ported/prompt.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 359 | `parsecolorchar` | 3 | 24 | 12% |

### `src/ported/utils.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 4270 | `lchdir` | 8 | 298 | 2% |

### `src/ported/zle/compmatch.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 2032 | `bld_line` | 40 | 137 | 29% |

### `src/ported/zle/compresult.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 96 | `cline_str` | 45 | 268 | 16% |

### `src/ported/zle/computil.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 6654 | `cfp_add_sdirs` | 18 | 73 | 24% |

### `src/ported/zle/zle_main.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 428 | `getrestchar` | 7 | 36 | 19% |

