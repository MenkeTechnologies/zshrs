# PORT_STUBS — stubs detected in src/ported/

Generated: 2026-05-24T20:42:32.307054+00:00

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

## Summary: 2 stubs across 2 files

| File | Stubs | Worst (Rust / C lines) |
|---|---|---|
| `src/ported/exec.rs` | 1 | `namedpipe` (24 / 1068) |
| `src/ported/prompt.rs` | 1 | `addbufspc` (1 / 15) |

## Per-file detail

### `src/ported/exec.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 3135 | `namedpipe` | 24 | 1068 | 2% |

### `src/ported/prompt.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 483 | `addbufspc` | 1 | 15 | 6% |

