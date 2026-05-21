# PORT_STUBS — stubs detected in src/ported/

Generated: 2026-05-21T15:43:01.984799+00:00

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

## Summary: 7 stubs across 4 files

| File | Stubs | Worst (Rust / C lines) |
|---|---|---|
| `src/ported/utils.rs` | 3 | `sb_niceformat` (15 / 58) |
| `src/ported/exec.rs` | 2 | `namedpipe` (24 / 1068) |
| `src/ported/zle/complist.rs` | 1 | `domenuselect` (85 / 916) |
| `src/ported/zle/zle_utils.rs` | 1 | `get_undo_current_change` (3 / 11) |

## Per-file detail

### `src/ported/utils.rs` — 3 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 5804 | `sb_niceformat` | 15 | 58 | 25% |
| 5846 | `is_sb_niceformat` | 4 | 15 | 26% |
| 5411 | `zreaddir` | 8 | 29 | 27% |

### `src/ported/exec.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 2860 | `namedpipe` | 24 | 1068 | 2% |
| 410 | `getoutput` | 11 | 67 | 16% |

### `src/ported/zle/complist.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 2505 | `domenuselect` | 85 | 916 | 9% |

### `src/ported/zle/zle_utils.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1214 | `get_undo_current_change` | 3 | 11 | 27% |

