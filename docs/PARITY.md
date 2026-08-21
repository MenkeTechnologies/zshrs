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

`fuzz_discovered_parity::quote_flag_formatting::nul_quoting_format` shares a
root cause with the `quote` fuzz mode: zshrs represents zsh's token bytes
(`0x84`–`0xA1`, `Src/lex.c:38` `ztokens`) as Rust `char`s, so a genuine
codepoint in U+0084–U+00A1 is indistinguishable from a token. C avoids the
collision by Meta-escaping bytes. Verified boundary: U+009F/A0/A1 mangle,
U+00A2 and above round-trip cleanly. Fixing it needs a representation
change, not a call-site patch.

## Relationship to the other two measurements

This suite is zshrs's own hand-written parity corpus. Two other numbers
measure compatibility from different angles, and all three should be read
together (see the Compatibility measurement section in `README.md`):

- **Differential fuzz** (`bins/parity-fuzz.rs`) — 22,200 generated cases
  against real zsh, 27 divergences across 71-of-74 clean modes.
- **Upstream ztst corpus** (`tests/ztst_runner.rs`) — 70 passing, 0
  failing, 1,292 cases pinned `#[ignore]` with per-case gap reasons.

The ztst pins are the largest honest measure of remaining debt; this
suite's 5 and the fuzzer's 27 are both much smaller because each samples
a narrower slice of the language.
