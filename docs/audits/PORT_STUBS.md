# PORT_STUBS — stubs detected in src/ported/

Generated: 2026-10-18T06:48:06.654832+00:00

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

## Summary: 62 stubs across 30 files

| File | Stubs | Worst (Rust / C lines) |
|---|---|---|
| `src/ported/mem.rs` | 10 | `malloc` (1 / 165) |
| `src/ported/hashtable.rs` | 9 | `createreswdtable` (1 / 16) |
| `src/ported/utils.rs` | 7 | `unmeta` (4 / 35) |
| `src/ported/zle/zle_keymap.rs` | 3 | `newkeytab` (1 / 15) |
| `src/ported/zle/zle_utils.rs` | 3 | `handleundo` (2 / 28) |
| `src/ported/compat.rs` | 2 | `zgettime_monotonic_if_available` (23 / 404) |
| `src/ported/module.rs` | 2 | `load_and_bind` (1 / 20) |
| `src/ported/modules/zutil.rs` | 2 | `freestylenode` (2 / 10) |
| `src/ported/parse.rs` | 2 | `load_dump_file` (12 / 47) |
| `src/ported/zle/compctl.rs` | 2 | `createcompctltable` (4 / 14) |
| `src/ported/builtins/rlimits.rs` | 1 | `printrlim` (1 / 13) |
| `src/ported/exec.rs` | 1 | `namedpipe` (24 / 1085) |
| `src/ported/glob.rs` | 1 | `zglob` (72 / 616) |
| `src/ported/hashnameddir.rs` | 1 | `createnameddirtable` (2 / 15) |
| `src/ported/hist.rs` | 1 | `lockhistfile` (25 / 124) |
| `src/ported/loop.rs` | 1 | `execselect` (5 / 112) |
| `src/ported/modules/db_gdbm.rs` | 1 | `unmetafy_zalloc` (3 / 11) |
| `src/ported/modules/parameter.rs` | 1 | `setfunctions` (4 / 15) |
| `src/ported/modules/pcre.rs` | 1 | `pcre_callout` (1 / 15) |
| `src/ported/modules/watch.rs` | 1 | `readwtab` (24 / 183) |
| `src/ported/modules/zpty.rs` | 1 | `ptysettyinfo` (6 / 21) |
| `src/ported/options.rs` | 1 | `createoptiontable` (4 / 16) |
| `src/ported/signals.rs` | 1 | `wait_for_processes` (14 / 102) |
| `src/ported/zle/compcore.rs` | 1 | `freematches` (6 / 26) |
| `src/ported/zle/complete.rs` | 1 | `bin_compadd` (61 / 235) |
| `src/ported/zle/compmatch.rs` | 1 | `match_parts` (3 / 12) |
| `src/ported/zle/deltochar.rs` | 1 | `boot_` (1 / 11) |
| `src/ported/zle/zle_move.rs` | 1 | `backwardmetafiedchar` (3 / 75) |
| `src/ported/zle/zle_thingy.rs` | 1 | `createthingytab` (1 / 13) |
| `src/ported/zle/zle_tricky.rs` | 1 | `pfxlen` (4 / 38) |

## Per-file detail

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

### `src/ported/hashtable.rs` — 9 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 2925 | `createreswdtable` | 1 | 16 | 6% |
| 1692 | `hashdir` | 4 | 58 | 6% |
| 1651 | `createcmdnamtable` | 1 | 14 | 7% |
| 1903 | `createshfunctable` | 1 | 13 | 7% |
| 3379 | `createhisttable` | 1 | 13 | 7% |
| 1556 | `expandhashtable` | 2 | 15 | 13% |
| 1567 | `resizehashtable` | 4 | 18 | 22% |
| 2029 | `freeshfuncnode` | 4 | 17 | 23% |
| 1596 | `printhashtabinfo` | 6 | 22 | 27% |

### `src/ported/utils.rs` — 7 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 7535 | `unmeta` | 4 | 35 | 11% |
| 8456 | `mb_metacharlenconv_r` | 5 | 32 | 15% |
| 113 | `set_widearray` | 6 | 30 | 20% |
| 7370 | `metafy` | 11 | 54 | 20% |
| 3974 | `timespec_diff_us` | 5 | 18 | 27% |
| 1726 | `delprepromptfn` | 4 | 14 | 28% |
| 7910 | `niceztrlen` | 4 | 14 | 28% |

### `src/ported/zle/zle_keymap.rs` — 3 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 285 | `newkeytab` | 1 | 15 | 6% |
| 180 | `emptykeymapnamtab` | 1 | 14 | 7% |
| 134 | `createkeymapnamtab` | 1 | 13 | 7% |

### `src/ported/zle/zle_utils.rs` — 3 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1717 | `handleundo` | 2 | 28 | 7% |
| 47 | `sizeline` | 5 | 20 | 25% |
| 639 | `zle_free_positions` | 3 | 11 | 27% |

### `src/ported/compat.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 71 | `zgettime_monotonic_if_available` | 23 | 404 | 5% |
| 265 | `zgetdir` | 15 | 146 | 10% |

### `src/ported/module.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 3746 | `load_and_bind` | 1 | 20 | 5% |
| 4118 | `module_func` | 1 | 16 | 6% |

### `src/ported/modules/zutil.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 173 | `freestylenode` | 2 | 10 | 20% |
| 859 | `setstypat` | 15 | 58 | 25% |

### `src/ported/parse.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 5533 | `load_dump_file` | 12 | 47 | 25% |
| 2631 | `par_subsh` | 9 | 34 | 26% |

### `src/ported/zle/compctl.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 98 | `createcompctltable` | 4 | 14 | 28% |
| 238 | `freecompcond` | 11 | 37 | 29% |

### `src/ported/builtins/rlimits.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 439 | `printrlim` | 1 | 13 | 7% |

### `src/ported/exec.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 3985 | `namedpipe` | 24 | 1085 | 2% |

### `src/ported/glob.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1671 | `zglob` | 72 | 616 | 11% |

### `src/ported/hashnameddir.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 35 | `createnameddirtable` | 2 | 15 | 13% |

### `src/ported/hist.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 5116 | `lockhistfile` | 25 | 124 | 20% |

### `src/ported/loop.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 484 | `execselect` | 5 | 112 | 4% |

### `src/ported/modules/db_gdbm.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1299 | `unmetafy_zalloc` | 3 | 11 | 27% |

### `src/ported/modules/parameter.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1018 | `setfunctions` | 4 | 15 | 26% |

### `src/ported/modules/pcre.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 227 | `pcre_callout` | 1 | 15 | 6% |

### `src/ported/modules/watch.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 570 | `readwtab` | 24 | 183 | 13% |

### `src/ported/modules/zpty.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 147 | `ptysettyinfo` | 6 | 21 | 28% |

### `src/ported/options.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 178 | `createoptiontable` | 4 | 16 | 25% |

### `src/ported/signals.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 328 | `wait_for_processes` | 14 | 102 | 13% |

### `src/ported/zle/compcore.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 6641 | `freematches` | 6 | 26 | 23% |

### `src/ported/zle/complete.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 978 | `bin_compadd` | 61 | 235 | 25% |

### `src/ported/zle/compmatch.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 2209 | `match_parts` | 3 | 12 | 25% |

### `src/ported/zle/deltochar.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 141 | `boot_` | 1 | 11 | 9% |

### `src/ported/zle/zle_move.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 240 | `backwardmetafiedchar` | 3 | 75 | 4% |

### `src/ported/zle/zle_thingy.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 49 | `createthingytab` | 1 | 13 | 7% |

### `src/ported/zle/zle_tricky.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 4242 | `pfxlen` | 4 | 38 | 10% |

