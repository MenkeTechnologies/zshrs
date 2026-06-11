# Parity Test Failures

Snapshot of `cargo test --test '*parity*' --no-fail-fast` results.

**Last run:** 2026-06-11 (late evening — full zsh_compat sweep + every
previously-failing binary re-run individually on the macOS aarch64
dev box).

## Summary

| Metric              | Count  |
| ------------------- | ------ |
| Total tests         | 43,904 |
| Passing             | 43,894 |
| **Failing**         | **7**  |
| Ignored             | 8      |
| Pass rate           | 99.98% |
| Test binaries       | 82     |
| Binaries with fails | 3      |

Delta vs the earlier 2026-06-11 full-sweep snapshot: 28 → 9 stable
failures (19 closed), 9 → 4 binaries — then 8 after merging the
parallel branch: its zle commit (7a3c00cf53, every iwidgets.list
thingy binds a widget per Src/Zle/zle_thingy.c:1022) closed
bulk_i_hash_num_widgets. Closed this pass:

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
- **corpus_parity (10 rows → 0, now 128/128)** — (a) the parity
  decoder's strings now stay tokenized (byte→char widened) so BOTH
  harness sides canonicalize through the one ported untokenize
  (Src/exec.c:2077 + Src/lex.c:38 ztokens) in `ast_sexp::emit_str`;
  the byte-level `zwc::untokenize` it previously used had no
  Qstring/Qtick/OutangProc arms and dropped Bnull, so `"$(...)"`,
  ``"`...`"``, `>(...)`, dq-escapes and `$'...'` rows falsely
  diverged. (b) `[[ x =~ pat ]]` now builds the Regex cond node for
  the lexer's token forms (Equals `\u{8d}`/Tilde `\u{98}`) per
  par_cond_triple Src/parse.c:2685-2691 — the old check only matched
  ASCII `=~`, so every real regex cond decoded as Binary.

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

### zinit_p10k_parity (1)

- `megamonsters::fzf_tab_swap_around_null_delim` — zsh leaves the
  literal `$'` + `'` quote chars around the decoded NUL in
  `${arr/pat/$match[2]$'\0'$match[1]}` replacements; zshrs decodes
  the whole `$'...'` region. Needs the paramstrsub quote-retention
  quirk (Src/subst.c:4274 area).

### zsh_compat_parity_gaps (3)

- `corpus_dash_fc_bulk_b::bulk_b_zcompile_tmpfile`
- `corpus_dash_fc_bulk_h::bulk_h_zcompile_then_rm_zwc`
- `corpus_dash_fc_bulk_p::bulk_p_fc_zcompile_empty_file`

## Themes

- **zcompile / zcompdump binary formats.** binary_parity ×3 +
  zsh_compat ×3 — byte-identical `.zwc`/`.zcompdump` emission; the
  dominant remaining arm.
- **fzf-tab NUL-delimiter swap.** paramstrsub `$'...'`
  quote-retention quirk.
