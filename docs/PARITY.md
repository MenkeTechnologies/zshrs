# Parity Test Failures

Snapshot of `cargo test --test '*parity*' --no-fail-fast` results.

**Last run:** 2026-06-11 (evening, full sweep on the macOS aarch64 dev
box — all 82 binaries, no extrapolation).

## Summary

| Metric              | Count  |
| ------------------- | ------ |
| Total tests         | 43,904 |
| Passing             | 43,868 |
| **Failing**         | **28** |
| Ignored             | 8      |
| Pass rate           | 99.94% |
| Test binaries       | 82     |
| Binaries with fails | 9      |

Delta vs the 2026-06-11 morning snapshot: the 74-failure list dropped
to 13 (61 closed by the day's fixes), but the full sweep surfaces 15
more failures in four binaries the morning snapshot marked green
(cd_options, prompt_escapes standout arm, glob_numeric closed-range
suffix, xtrace corpus). Those 15 fail identically at pre-work commit
1694e3da96 — they predate today's changes and are environment-specific
to this machine vs wherever the morning snapshot ran. Closed today:
parity_survey_fc ×11 (assoc odd-count errflag abort, `(e:CODE:)`
qualifier REPLY/`./`-prefix + single-quote body protection, `touch -t`
in the absorbed builtin, `sort` strcoll collation, `private` scoping
via makeprivate scan + getprivatenode lookup walk + wrap_private
fn-call hook), assoc_array_deep ×2, recent_ports ×1, fd_redirect ×2,
time_keyword ×3, shift_positional ×1, quoting + printf (absorbed `wc`
byte/line counts now POSIX), zsh_compat 47 → 5.

Flaky (not counted as stable failures):
- `read_parity::count_chars::read_k_reads_n_chars` — read -k timing on
  non-tty stdin; alternates pass/fail.
- `parity_harness::zpwr_real_world_parity` — fails under full-sweep
  load, passes isolated.

## Per-binary failures

### binary_parity (3)

- `zcompdump_byte_identical_roundtrip`
- `zcompdump_synthesize_format`
- `zstyle_canonical_roundtrip`

### cd_options_parity (9) — predates 2026-06-11 work; env-specific

- `auto_cd::auto_cd_bare_dirname`
- `basic_cd::cd_dash_goes_to_oldpwd`
- `basic_cd::cd_to_subdir_then_pwd`
- `cd_L_P_flags::cd_dash_L_keeps_logical_path`
- `cd_L_P_flags::cd_dash_P_resolves_physical`
- `cd_two_args::cd_two_arg_substitutes`
- `chase_links::chase_links_resolves_symlink`
- `oldpwd::oldpwd_set_after_cd`
- `oldpwd::pwd_var_updates_on_cd`

### discovered_parity_failures (2)

- `parity_Q_flag_on_literal_should_error`
- `parity_flag_on_literal_L_should_error`

### glob_numeric_parity (1) — predates 2026-06-11 work

- `closed_range::closed_range_with_suffix`

### parity_harness (1 + 1 flaky)

- `corpus_parity` — parse error near `)` at corpus row :43306
- (`zpwr_real_world_parity` — sweep-load flake, passes isolated)

### prompt_escapes_parity (4) — predates 2026-06-11 work; env-specific

- `ansi_attrs::S_standout_emits_sgr`
- `ansi_attrs::s_standout_off_emits_sgr`
- `ps4_audit::pct_S_standout_on`
- `ps4_audit::pct_s_standout_off`

### xtrace_corpus_parity (1) — predates 2026-06-11 work

- `corpus_xtrace_parity`

### zinit_p10k_parity (1)

- `megamonsters::fzf_tab_swap_around_null_delim`

### zsh_compat_parity_gaps (5)

- `corpus_dash_fc_bulk_ah::bulk_ah_fc_row_089`
- `corpus_dash_fc_bulk_b::bulk_b_zcompile_tmpfile`
- `corpus_dash_fc_bulk_h::bulk_h_zcompile_then_rm_zwc`
- `corpus_dash_fc_bulk_i::bulk_i_hash_num_widgets`
- `corpus_dash_fc_bulk_p::bulk_p_fc_zcompile_empty_file`

## Themes

- **cd / PWD / OLDPWD arm.** cd_options_parity ×9 — largest remaining
  block; logical-vs-physical path handling and PWD/OLDPWD updates on
  this machine (macOS `/tmp` → `/private/tmp` symlink likely
  implicated).
- **zcompile / zcompdump binary formats.** `zcompdump_*`,
  `zstyle_canonical_roundtrip`, `bulk_b_zcompile_tmpfile`,
  `bulk_h_zcompile_then_rm_zwc`, `bulk_p_fc_zcompile_empty_file` —
  byte-identical `.zwc`/`.zcompdump` emission.
- **prompt standout SGR.** `%S`/`%s` escape emission ×4.
- **`(Q)`/`(L)` flag-on-literal error paths.** discovered_parity ×2.
- **widget table count.** `bulk_i_hash_num_widgets` — `$widgets`
  population vs zsh's zle module.
- **corpus harnesses.** `corpus_parity` (one row, parse error near
  `)`), `corpus_xtrace_parity`.
- **fzf-tab NUL-delimiter swap.** single megamonster pending.
