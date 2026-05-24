# PORT_STUBS — stubs detected in src/ported/

Generated: 2026-05-24T17:33:57.225984+00:00

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
| `src/ported/prompt.rs` | 1 | `addbufspc` (1 / 15) |
| `src/ported/zle/complist.rs` | 1 | `domenuselect` (85 / 916) |

## Per-file detail

### `src/ported/exec.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 3129 | `namedpipe` | 24 | 1068 | 2% |

### `src/ported/prompt.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 483 | `addbufspc` | 1 | 15 | 6% |

### `src/ported/zle/complist.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 2508 | `domenuselect` | 85 | 916 | 9% |

