# Parity Test Failures

Snapshot of `cargo test --test '*parity*' --no-fail-fast` results.

**Last run:** 2026-06-12 (full zsh_compat sweep + every
previously-failing binary re-run individually on the macOS aarch64
dev box).

## Summary

| Metric              | Count  |
| ------------------- | ------ |
| Total tests         | 43,904 |
| Passing             | 43,898 |
| **Failing**         | **3**  |
| Ignored             | 8      |
| Pass rate           | 99.99% |
| Test binaries       | 82     |
| Binaries with fails | 1      |

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
- **2026-06-12: fzf-tab NUL-delimiter swap** (zinit_p10k 192/192) —
  DQ `$'` stays literal in `/`-replacements per Src/subst.c:301
  (Snull-token-only ANSI-C trigger; dquote_parse emits Qstring + raw
  `'`, Src/lex.c:1519-1556), BUILTIN_PARAM_REPLACE honors its dq
  flag, and stringsubst advances to the LAST spliced node per
  Src/subst.c:339 so prefork never re-scans (= double-expands)
  splat-inserted values.
- **2026-06-12: zsh_compat zcompile rows ×3** — `.zwc` wordcode dump
  emission (Src/parse.c:3334-3482 port); zsh_compat now 40924/1
  (the -N flake).
- **2026-06-12: invariant gates green** — no_tree_walker_dispatch
  160/160 (dynamic-name AOP intercept gate added; two stale pins
  repaired in fcda59d530), tree_walker_absent 8/8.
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

## Themes

- **zcompdump / zstyle byte formats.** The single remaining arm:
  byte-identical `.zcompdump` synthesis/roundtrip + the zstyle
  canonical dump (`.zwc` emission itself landed 2026-06-12).
