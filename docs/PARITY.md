# Parity Tests

`cargo test --test '*parity*' --no-fail-fast` runs zshrs's hand-written parity
corpus: tests that spawn real `zsh` and `zshrs` processes and compare
whole-shell state. Read failures from a serial run (`--test-threads=1`):
the tests are load sensitive, and a parallel full-suite run reports failures
that pass when re-run alone.

Two pitfalls worth remembering. `cargo test`
spawns its shells with SIGQUIT already ignored, exactly like `nohup`, so
a `config_state_parity` failure was the single line
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

## Relationship to the other measurements

This suite is zshrs's own parity corpus. Three other measurements look at
compatibility from different angles; read them together (the Compatibility
measurement section of `README.md` carries the current figures):

- **Differential fuzz** (`bins/parity-fuzz.rs`) — generated cases against
  real zsh, per mode; `--matrix` runs every dialect against its reference shell.
- **zsh's own test suite** (`scripts/ztst_compsys.py --core`, oracle zsh 5.9.2
  from the `zsh-5.9.2` tag). Detail in `docs/parity_report.html`.
- **In-tree ztst corpus** (`tests/ztst_runner.rs`, no oracle).
- **Emulation matrix** (`tests/emulation_parity.rs`) — every dialect against
  its real reference shell.

The oracle-backed ztst figure is the largest measure of remaining debt; the
other suites sample narrower slices of the language.
