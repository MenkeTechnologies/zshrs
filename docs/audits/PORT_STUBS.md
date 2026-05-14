# PORT_STUBS — stubs detected in src/ported/

Generated: 2026-05-14T23:09:16.535003+00:00

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

## Summary: 7 stubs across 6 files

| File | Stubs | Worst (Rust / C lines) |
|---|---|---|
| `src/ported/modules/parameter.rs` | 2 | `funcsourcetracegetfn` (4 / 15) |
| `src/ported/builtin.rs` | 1 | `getasg` (8 / 27) |
| `src/ported/pattern.rs` | 1 | `restorepatterndisables` (1 / 10) |
| `src/ported/prompt.rs` | 1 | `tsetcap` (9 / 31) |
| `src/ported/zle/compctl.rs` | 1 | `ccmakehookfn` (32 / 107) |
| `src/ported/zle/computil.rs` | 1 | `cd_arrcat` (3 / 13) |

## Per-file detail

### `src/ported/modules/parameter.rs` — 2 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 736 | `funcsourcetracegetfn` | 4 | 15 | 26% |
| 723 | `functracegetfn` | 4 | 14 | 28% |

### `src/ported/builtin.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1971 | `getasg` | 8 | 27 | 29% |

### `src/ported/pattern.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1199 | `restorepatterndisables` | 1 | 10 | 10% |

### `src/ported/prompt.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 405 | `tsetcap` | 9 | 31 | 29% |

### `src/ported/zle/compctl.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 1266 | `ccmakehookfn` | 32 | 107 | 29% |

### `src/ported/zle/computil.rs` — 1 stubs

| Rust line | fn | rust body | C body | ratio |
|---|---|---|---|---|
| 846 | `cd_arrcat` | 3 | 13 | 23% |

