# Parity Test Failures

Snapshot of `cargo test --test '*parity*' --no-fail-fast` results.

**Last run:** 2026-08-21 (full sweep on the macOS aarch64 dev box, with
`cargo build -p zshrs-daemon` done first — see the note on
`binary_parity` below).

**Read the failure count serially.** These tests spawn real `zsh` and
`zshrs` processes and compare whole-shell state, so they are load
sensitive: a parallel full-suite run has reported 22 and 33 failures where
re-running the very same tests with `--test-threads=1` passed them. The
counts below are the serial ones.

## Summary

| Metric              | Count  |
| ------------------- | ------ |
| Total tests         | 46,724 |
| Passing             | 46,685 |
| **Failing**         | **5**  |
| Ignored             | 34     |
| Pass rate           | 99.99% |
| Test binaries       | 2      |

The five remaining failures, and why each is still open:

| Test | Status |
| ---- | ------ |
| `fuzz_discovered_parity::…::nul_quoting_format` | Structural. C stores NUL METAFIED, so `$'\0'` survives a parse/eval round trip; zshrs holds a real `\u{0}` in a Rust `String`. Same root cause as the `quote` fuzz mode. |
| `zsh_compat_parity_gaps::…::zmodload_capital_R_complete` | Not a gap. zshrs deliberately repurposes `-R` WITHOUT `-A` to load native Rust plugin cdylibs (`src/ported/module.rs`); C's alias-removal meaning is preserved for `-A -R`. |
| `config_state_parity::real_all_installed_plugins_final_state` | Four openshift aliases. Under `RC_QUOTES`, zshrs reads `''` inside a single-quoted alias body as a literal quote where zsh concatenates. zsh's own side is INPUT-BUFFER dependent — inserting one no-op line into the plugin flips zsh to zshrs's reading — so which behaviour is the target needs deciding before it is pinned. |
| `…::bulk_vw_fc_row_015`, `…::times_builtin_summary` | Load artifacts: both pass under `--test-threads=1`. |

Closed since the 2026-08-20 snapshot (24 -> 5), all traced to C:

| Fix | C reference | Tests |
| --- | ----------- | ----- |
| Inherited `SIGQUIT` SIG_IGN never recorded as an ignored trap | `init.c:1444-1445` | 15 |
| Inherited `SIGHUP` SIG_IGN never cleared the `HUP` option | `init.c:1451-1452` | 2 |
| `case` reset `$?` before the branch body instead of after a no-match | `loop.c:613/672/705` | 1 |
| `zle -C` widgets listed as `-N`, dropping the completion triple | `zle_thingy.c:517-536` | plugin state |
| `zmodload` used the no-dynamic-loading message form | `module.c:1622` / BUGS.md #376 | 1 |

The first two share one root cause worth remembering: `cargo test`
spawns its shells with SIGQUIT already ignored, exactly like `nohup`, so
every `config_state_parity` failure was the single line
`[TRAPS] only in zsh  : trap -- '' QUIT`. They passed when run
individually from an interactive shell and failed as a suite.

`binary_parity`'s three daemon-RPC tests (`zcompdump_byte_identical_roundtrip`,
`zcompdump_synthesize_format`, `zstyle_canonical_roundtrip`) pass once
`cargo build -p zshrs-daemon` has run. Cargo does not hand
`CARGO_BIN_EXE_zshrs-daemon` to the root crate's integration tests, so the
fallback path expects the binary pre-built; without it they fail for a
harness reason, not a code gap. Build the daemon before reading this suite.

zshrs keeps zsh's lexer tokens (the bytes `0x84`–`0xA2`, `Src/zsh.h:159-224`)
as Private Use Area chars at U+E084–U+E0A2 (`src/token_char.rs`), so real
U+0084–U+00A2 text (NBSP, `¡`, `¢`, NEL, C1 controls) never reads as a token.
The older U+0084–U+00A1 encoding mangled those chars under `(q)`, `(V)` and
`(qqqq)`, and on the line-at-a-time stdin path.

## Relationship to the other two measurements

This suite is zshrs's own hand-written parity corpus. Two other numbers
measure compatibility from different angles, and all three should be read
together (see the Compatibility measurement section in `README.md`):

- **Differential fuzz** (`bins/parity-fuzz.rs`) — 22,200 generated cases
  against real zsh, 27 divergences across 71-of-74 clean modes.
- **zsh's own test suite** (`scripts/ztst_compsys.py --core`, oracle zsh
  5.9.2 from the `zsh-5.9.2` tag) — `main` at `431cc21de5` passes 2,068 of the 2,068
  assertions zsh passes. Detail in `docs/parity_report.html`.
- **In-tree ztst corpus** (`tests/ztst_runner.rs`, no oracle) — 2,625 of
  2,796 chunks pass at `17bf07d5ed`; see `README.md` for what the failures are.

The oracle-backed ztst figure is the largest measure of remaining debt; this
suite's 5 and the fuzzer's 27 sample narrower slices of the language.
