# Parity Test Failures

Snapshot of `cargo test --test '*parity*' --no-fail-fast` results.

**Last run:** 2026-06-11 (evening pass — re-ran every binary that had
failures in the morning snapshot, plus the suites touched by the
day's fixes; binaries untouched and green in the morning run are
assumed unchanged).

## Summary

| Metric              | Count  |
| ------------------- | ------ |
| Total tests         | 43,890 |
| Passing             | 43,869 |
| **Failing**         | **13** |
| Ignored             | 8      |
| Pass rate           | 99.97% |
| Test binaries       | 82     |
| Binaries with fails | 6      |

Delta vs the 2026-06-11 morning snapshot: 74 → 13 failing (61 closed),
12 → 6 failing binaries. Suites that went fully green since:
parity_survey_fc (assoc odd-count errflag abort, `(e:CODE:)` qualifier
REPLY/`./`-prefix + single-quote body protection, `touch -t` in the
absorbed builtin, `sort` strcoll collation, `private` scoping via
makeprivate scan + getprivatenode lookup walk + wrap_private fn-call
hook), assoc_array_deep_parity, recent_ports_parity,
fd_redirect_parity, time_keyword_parity, shift_positional_parity,
quoting_parity + printf_format_parity (absorbed `wc` byte/line counts
now POSIX — raw-byte streaming, no phantom trailing newline), and
zsh_compat_parity_gaps dropped 47 → 5.

`read_parity::count_chars::read_k_reads_n_chars` is FLAKY (read -k
timing on non-tty stdin) — failed once, passed on rerun; not counted.

## Per-binary failures

### binary_parity (3)

- `zcompdump_byte_identical_roundtrip`
- `zcompdump_synthesize_format`
- `zstyle_canonical_roundtrip`

### discovered_parity_failures (2)

- `parity_Q_flag_on_literal_should_error`
- `parity_flag_on_literal_L_should_error`

### parity_harness (1)

- `corpus_parity`

### zinit_p10k_parity (1)

- `megamonsters::fzf_tab_swap_around_null_delim`

### zsh_compat_parity_gaps (5)

- `corpus_dash_fc_bulk_ah::bulk_ah_fc_row_089`
- `corpus_dash_fc_bulk_b::bulk_b_zcompile_tmpfile`
- `corpus_dash_fc_bulk_h::bulk_h_zcompile_then_rm_zwc`
- `corpus_dash_fc_bulk_i::bulk_i_hash_num_widgets`
- `corpus_dash_fc_bulk_p::bulk_p_fc_zcompile_empty_file`

### read_parity (0–1, flaky)

- `count_chars::read_k_reads_n_chars` — non-deterministic on non-tty
  stdin; see note above.

## Themes

- **zcompile / zcompdump binary formats.** `zcompdump_*`,
  `zstyle_canonical_roundtrip`, `bulk_b_zcompile_tmpfile`,
  `bulk_h_zcompile_then_rm_zwc`, `bulk_p_fc_zcompile_empty_file` —
  the dominant remaining arm: byte-identical `.zwc`/`.zcompdump`
  emission.
- **`(Q)`/`(L)` flag-on-literal error paths.** discovered_parity ×2.
- **widget table count.** `bulk_i_hash_num_widgets` — `$widgets`
  population vs zsh's zle module.
- **corpus harness.** `corpus_parity` — parse error near `)` at corpus
  row :43306; one corpus row, not a suite-wide arm.
- **fzf-tab NUL-delimiter swap.** single megamonster pending.
