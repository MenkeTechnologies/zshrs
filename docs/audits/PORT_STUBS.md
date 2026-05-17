# PORT_STUBS — stubs detected in src/ported/

Generated: 2026-05-17T03:49:46.683602+00:00

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

## Summary: 5 stubs across 2 files

| File | Stubs | Worst (Rust / C lines) |
|---|---|---|
| `src/ported/utils.rs` | 4 | `quotedzputs` (11 / 166) |
| `src/ported/zle/zle_utils.rs` | 1 | `shiftchars` (14 / 64) |

## Per-file detail

### `src/ported/utils.rs` — 4 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 5028 | `quotedzputs` | 11 | 166 | 6% |
| 4695 | `sb_niceformat` | 8 | 58 | 13% |
| 4729 | `is_sb_niceformat` | 4 | 15 | 26% |
| 4324 | `zreaddir` | 8 | 29 | 27% |

### `src/ported/zle/zle_utils.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 209 | `shiftchars` | 14 | 64 | 21% |

