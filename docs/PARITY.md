# Parity Test Failures

Snapshot of `cargo test --test '*parity*' --no-fail-fast` results.

**Last run:** 2026-06-11 (late evening — full zsh_compat sweep + every
previously-failing binary re-run individually on the macOS aarch64
dev box).

## Summary

| Metric              | Count  |
| ------------------- | ------ |
| Total tests         | 43,904 |
| Passing             | 43,892 |
| **Failing**         | **9**  |
| Ignored             | 8      |
| Pass rate           | 99.98% |
| Test binaries       | 82     |
| Binaries with fails | 4      |

Delta vs the earlier 2026-06-11 full-sweep snapshot: 28 → 9 stable
failures (19 closed), 9 → 4 binaries. Closed this pass:

- **cd_options_parity ×9** — validated PWD/OLDPWD override after env
  import per Src/init.c:1241-1257 + set_pwd_env (Src/params.c:955);
  stale inherited `$PWD` no longer survives into paramtab.
- **prompt_escapes_parity ×4** — C-exact `init_term` via
  tgetent/tgetstr over `tccapnams` (Src/init.c:766-890); `%S`/`%s`
  emit the real `so`/`se` caps for the live `$TERM` (screen/tmux →
  `\e[3m`/`\e[23m`), wired via term_reinit_from_pm (params.c:5170)
  and the promptexpand lazy load (prompt.c:189-190).
- **glob_numeric_parity ×1** — absorbed `sort -n` now strtod-style
  leading-numeric-prefix.
- **xtrace_corpus_parity ×1** — fixed by the PWD + terminfo work
  (PS4 path).
- **discovered_parity_failures ×2** — `${"abc"}` / `${(Q)"abc"}` now
  "bad substitution" per Src/subst.c:2993-3004 (raw Dnull at the
  operator position); quoted-brace-body words route to the bridge so
  paramsubst's gate actually runs.
- **zsh_compat bulk_ah_fc_row_089** — reclassified flaky (below).

Flaky (pass solo / under low load; not counted):
- `read_parity::count_chars::read_k_reads_n_chars` — read -k timing
  on non-tty stdin.
- `parity_harness::zpwr_real_world_parity` — fails under full-sweep
  load only.
- `zsh_compat::bulk_ah_fc_row_089` — `[[ -N /dev/null ]]`:
  atime/mtime race on the shared device node while the suite hammers
  /dev/null in parallel.

## Per-binary failures

### binary_parity (3)

- `zcompdump_byte_identical_roundtrip`
- `zcompdump_synthesize_format`
- `zstyle_canonical_roundtrip`

### parity_harness (1)

- `corpus_parity` — AST-sexp harness, 118/128 corpus rows passing;
  the 10 divergent rows are parser-AST shape gaps (cmdsubst word
  re-rendering, `=~` Regex vs Binary node, proc-subst `>(...)` word
  text, herestring escape re-rendering).

### zinit_p10k_parity (1)

- `megamonsters::fzf_tab_swap_around_null_delim` — zsh leaves the
  literal `$'` + `'` quote chars around the decoded NUL in
  `${arr/pat/$match[2]$'\0'$match[1]}` replacements; zshrs decodes
  the whole `$'...'` region. Needs the paramstrsub quote-retention
  quirk (Src/subst.c:4274 area).

### zsh_compat_parity_gaps (4)

- `corpus_dash_fc_bulk_b::bulk_b_zcompile_tmpfile`
- `corpus_dash_fc_bulk_h::bulk_h_zcompile_then_rm_zwc`
- `corpus_dash_fc_bulk_i::bulk_i_hash_num_widgets`
- `corpus_dash_fc_bulk_p::bulk_p_fc_zcompile_empty_file`

## Themes

- **zcompile / zcompdump binary formats.** binary_parity ×3 +
  zsh_compat ×3 — byte-identical `.zwc`/`.zcompdump` emission; the
  dominant remaining arm.
- **widget table count.** `bulk_i_hash_num_widgets` — `${#widgets}`
  386 (zsh) vs 254 (zshrs); needs more zle widget registry entries.
- **AST corpus.** `corpus_parity` — 10 parser-sexp shape rows.
- **fzf-tab NUL-delimiter swap.** paramstrsub `$'...'`
  quote-retention quirk.
