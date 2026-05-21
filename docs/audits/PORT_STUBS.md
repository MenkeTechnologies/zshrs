# PORT_STUBS — stubs detected in src/ported/

Generated: 2026-05-21T21:32:53.568932+00:00

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

## Summary: 3 stubs across 3 files

| File | Stubs | Worst (Rust / C lines) |
|---|---|---|
| `src/ported/exec.rs` | 1 | `namedpipe` (24 / 1068) |
| `src/ported/utils.rs` | 1 | `zreaddir` (8 / 29) |
| `src/ported/zle/complist.rs` | 1 | `domenuselect` (85 / 916) |

## Per-file detail

### `src/ported/exec.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 2954 | `namedpipe` | 24 | 1068 | 2% |

### `src/ported/utils.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 5615 | `zreaddir` | 8 | 29 | 27% |

### `src/ported/zle/complist.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 2505 | `domenuselect` | 85 | 916 | 9% |

