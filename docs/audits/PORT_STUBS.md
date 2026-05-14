# PORT_STUBS — stubs detected in src/ported/

Generated: 2026-05-14T21:41:24.381590+00:00

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

## Summary: 0 stubs across 0 files

| File | Stubs | Worst (Rust / C lines) |
|---|---|---|

## Per-file detail

