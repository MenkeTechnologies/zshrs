# PORT_STUBS — stubs detected in src/ported/

Generated: 2026-07-03T08:10:16.908154+00:00

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

## Summary: 69 stubs across 32 files

| File | Stubs | Worst (Rust / C lines) |
|---|---|---|
| `src/ported/hashtable.rs` | 10 | `newhashtable` (1 / 19) |
| `src/ported/mem.rs` | 10 | `malloc` (1 / 165) |
| `src/ported/utils.rs` | 6 | `mb_metacharlenconv_r` (5 / 32) |
| `src/ported/module.rs` | 3 | `load_and_bind` (1 / 20) |
| `src/ported/modules/zutil.rs` | 3 | `newzstyletable` (1 / 15) |
| `src/ported/zle/compcore.rs` | 3 | `set_comp_sep` (9 / 321) |
| `src/ported/zle/zle_keymap.rs` | 3 | `newkeytab` (1 / 15) |
| `src/ported/compat.rs` | 2 | `zgettime_monotonic_if_available` (23 / 404) |
| `src/ported/parse.rs` | 2 | `par_subsh` (6 / 34) |
| `src/ported/zle/compctl.rs` | 2 | `createcompctltable` (4 / 14) |
| `src/ported/zle/complete.rs` | 2 | `set_compstate` (3 / 26) |
| `src/ported/zle/zle_params.rs` | 2 | `get_cursor` (1 / 11) |
| `src/ported/zle/zle_utils.rs` | 2 | `sizeline` (5 / 20) |
| `src/ported/builtins/rlimits.rs` | 1 | `printrlim` (1 / 13) |
| `src/ported/exec.rs` | 1 | `namedpipe` (24 / 1085) |
| `src/ported/glob.rs` | 1 | `zglob` (62 / 614) |
| `src/ported/hashnameddir.rs` | 1 | `createnameddirtable` (2 / 15) |
| `src/ported/hist.rs` | 1 | `lockhistfile` (25 / 124) |
| `src/ported/init.rs` | 1 | `init_shout` (4 / 23) |
| `src/ported/modules/db_gdbm.rs` | 1 | `unmetafy_zalloc` (3 / 11) |
| `src/ported/modules/parameter.rs` | 1 | `setfunctions` (4 / 15) |
| `src/ported/modules/pcre.rs` | 1 | `pcre_callout` (1 / 15) |
| `src/ported/modules/watch.rs` | 1 | `readwtab` (24 / 183) |
| `src/ported/modules/zpty.rs` | 1 | `ptysettyinfo` (6 / 21) |
| `src/ported/options.rs` | 1 | `createoptiontable` (4 / 16) |
| `src/ported/pattern.rs` | 1 | `charsub` (5 / 17) |
| `src/ported/signals.rs` | 1 | `wait_for_processes` (11 / 102) |
| `src/ported/zle/deltochar.rs` | 1 | `boot_` (1 / 11) |
| `src/ported/zle/termquery.rs` | 1 | `probe_terminal` (46 / 208) |
| `src/ported/zle/zle_move.rs` | 1 | `backwardmetafiedchar` (3 / 75) |
| `src/ported/zle/zle_thingy.rs` | 1 | `createthingytab` (1 / 13) |
| `src/ported/zle/zle_tricky.rs` | 1 | `pfxlen` (4 / 38) |

## Per-file detail

### `src/ported/hashtable.rs` — 10 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 86 | `newhashtable` | 1 | 19 | 5% |
| 2130 | `createreswdtable` | 1 | 16 | 6% |
| 1040 | `hashdir` | 4 | 58 | 6% |
| 1007 | `createcmdnamtable` | 1 | 14 | 7% |
| 1209 | `createshfunctable` | 1 | 13 | 7% |
| 2487 | `createhisttable` | 1 | 13 | 7% |
| 912 | `expandhashtable` | 2 | 15 | 13% |
| 923 | `resizehashtable` | 4 | 18 | 22% |
| 1321 | `freeshfuncnode` | 4 | 17 | 23% |
| 952 | `printhashtabinfo` | 6 | 22 | 27% |

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

### `src/ported/utils.rs` — 6 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 7058 | `mb_metacharlenconv_r` | 5 | 32 | 15% |
| 100 | `set_widearray` | 6 | 30 | 20% |
| 6246 | `metafy` | 11 | 54 | 20% |
| 6393 | `unmeta` | 8 | 35 | 22% |
| 3226 | `timespec_diff_us` | 5 | 18 | 27% |
| 1502 | `delprepromptfn` | 4 | 14 | 28% |

### `src/ported/module.rs` — 3 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 3045 | `load_and_bind` | 1 | 20 | 5% |
| 3394 | `module_func` | 1 | 16 | 6% |
| 3077 | `try_load_module` | 4 | 14 | 28% |

### `src/ported/modules/zutil.rs` — 3 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 775 | `newzstyletable` | 1 | 15 | 6% |
| 166 | `freestylenode` | 2 | 10 | 20% |
| 792 | `setstypat` | 15 | 58 | 25% |

### `src/ported/zle/compcore.rs` — 3 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1493 | `set_comp_sep` | 9 | 321 | 2% |
| 3342 | `freematches` | 6 | 26 | 23% |
| 559 | `callcompfunc` | 103 | 345 | 29% |

### `src/ported/zle/zle_keymap.rs` — 3 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 287 | `newkeytab` | 1 | 15 | 6% |
| 176 | `emptykeymapnamtab` | 1 | 14 | 7% |
| 134 | `createkeymapnamtab` | 1 | 13 | 7% |

### `src/ported/compat.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 69 | `zgettime_monotonic_if_available` | 23 | 404 | 5% |
| 262 | `zgetdir` | 15 | 146 | 10% |

### `src/ported/parse.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 2224 | `par_subsh` | 6 | 34 | 17% |
| 4772 | `load_dump_file` | 12 | 47 | 25% |

### `src/ported/zle/compctl.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 97 | `createcompctltable` | 4 | 14 | 28% |
| 247 | `freecompcond` | 11 | 37 | 29% |

### `src/ported/zle/complete.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1642 | `set_compstate` | 3 | 26 | 11% |
| 853 | `bin_compadd` | 38 | 233 | 16% |

### `src/ported/zle/zle_params.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 144 | `get_cursor` | 1 | 11 | 9% |
| 608 | `set_killring` | 6 | 27 | 22% |

### `src/ported/zle/zle_utils.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 47 | `sizeline` | 5 | 20 | 25% |
| 530 | `zle_free_positions` | 3 | 11 | 27% |

### `src/ported/builtins/rlimits.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 274 | `printrlim` | 1 | 13 | 7% |

### `src/ported/exec.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 3257 | `namedpipe` | 24 | 1085 | 2% |

### `src/ported/glob.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1448 | `zglob` | 62 | 614 | 10% |

### `src/ported/hashnameddir.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 35 | `createnameddirtable` | 2 | 15 | 13% |

### `src/ported/hist.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 4098 | `lockhistfile` | 25 | 124 | 20% |

### `src/ported/init.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 642 | `init_shout` | 4 | 23 | 17% |

### `src/ported/modules/db_gdbm.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1248 | `unmetafy_zalloc` | 3 | 11 | 27% |

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
| 565 | `readwtab` | 24 | 183 | 13% |

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
| 2744 | `charsub` | 5 | 17 | 29% |

### `src/ported/signals.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 322 | `wait_for_processes` | 11 | 102 | 10% |

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

### `src/ported/zle/zle_tricky.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1983 | `pfxlen` | 4 | 38 | 10% |

