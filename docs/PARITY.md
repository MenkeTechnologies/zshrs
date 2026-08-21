# Parity Test Failures

Snapshot of `cargo test --test '*parity*' --no-fail-fast` results.

**Last run:** 2026-08-20 (full sweep on the macOS aarch64 dev box, with
`cargo build -p zshrs-daemon` done first — see the note on
`binary_parity` below).

## Summary

| Metric              | Count  |
| ------------------- | ------ |
| Total tests         | 46,724 |
| Passing             | 46,666 |
| **Failing**         | **24** |
| Ignored             | 34     |
| Pass rate           | 99.95% |
| Test binaries       | 2      |

Failures by binary:

| Binary | Failing | Area |
| ------ | ------- | ---- |
| `config_state_parity` | 15 | Whole-config final-state diffs (zpwr / zinit / p10k real-world loads, `typeset -p` roundtrip, hook arrays) |
| `zsh_compat_parity_gaps` | 4 | `trap -` handler listing, `-o hup`, `times` summary, `set +o` full dump |
| `fuzz_discovered_parity` | 2 | NUL quoting format; string-form `TRAPINT` replacing a function trap |
| `modules_parity` | 1 | `zmodload` nonexistent-module diagnostic prefix |
| `case_parity` | 1 | `case` branch body seeing inherited status |

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
  against real zsh, 27 divergences across 70-of-74 clean modes.
- **Upstream ztst corpus** (`tests/ztst_runner.rs`) — 70 passing, 0
  failing, 1,292 cases pinned `#[ignore]` with per-case gap reasons.

The ztst pins are the largest honest measure of remaining debt; this
suite's 24 and the fuzzer's 27 are both much smaller because each samples
a narrower slice of the language.
