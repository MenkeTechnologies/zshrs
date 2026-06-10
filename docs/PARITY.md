# Parity Test Failures

Snapshot of `cargo test --test '*parity*' --no-fail-fast` results.

**Last run:** 2026-06-10

## Summary

| Metric              | Count  |
| ------------------- | ------ |
| Total tests         | 43,898 |
| Passing             | 43,784 |
| **Failing**         | **106** |
| Ignored             | 8      |
| Pass rate           | 99.76% |
| Test binaries       | 82     |
| Binaries with fails | 25     |

Largest concentration: `zsh_compat_parity_gaps` carries 42 of the 106
failures (40,883 passing — almost half the entire suite by test count
is in this single binary).

## Per-binary failures

### assoc_array_deep_parity (2)

- `lookup::lookup_key_with_space`
- `special_keys::empty_string_key`

### binary_parity (3)

- `zcompdump_byte_identical_roundtrip`
- `zcompdump_synthesize_format`
- `zstyle_canonical_roundtrip`

### cond_parity (1)

- `string_pattern_match::pattern_via_tilde_expansion`

### discovered_parity_failures (2)

- `parity_Q_flag_on_literal_should_error`
- `parity_flag_on_literal_L_should_error`

### dollar_lt_parity (1)

- `special_chars::binary_content_byte_count`

### fd_redirect_parity (2)

- `close_fd::close_stdout_then_print_errors`
- `swap_stdout_stderr::swap_stdout_and_stderr`

### glob_numeric_parity (2)

- `any_positive_int::no_integer_files_nomatch_error`
- `closed_range::closed_range_with_suffix`

### modules_parity (12)

- `files_module::zf_chmod_changes_mode`
- `langinfo_module::codeset_returns_locale_encoding`
- `langinfo_module::known_items_present`
- `pcre_extra::pcre_match_captures_three_groups`
- `system_module::errnos_first_five_by_index`
- `system_module::errnos_indexed_lookup`
- `system_module::errnos_length`
- `terminfo_extra::terminfo_bold_and_sgr0`
- `terminfo_extra::terminfo_kcuf1_byte_exact`
- `terminfo_module::terminfo_kcuu1_byte_exact`
- `terminfo_module::terminfo_kf1_byte_exact`
- `watch_module::watchfmt_consistent`

### parity_harness (1)

- `corpus_parity`

### parity_survey_fc (15)

- `functions::private_scoping`
- `glob_qualifiers::qual_at_symlinks_only`
- `glob_qualifiers::qual_dot_regular_files`
- `glob_qualifiers::qual_e_glob_excl`
- `glob_qualifiers::qual_slash_dirs_only`
- `parameter_filters::subscript_n_nth_match`
- `parameter_substitutions::slash_array_elements`
- `paren_flags::paren_A_assign_assoc`
- `paren_flags::paren_B_begin_offset`
- `paren_flags::paren_E_end_offset`
- `paren_flags::paren_N_match_length`
- `paren_flags::paren_s_split_colon`
- `paren_flags::paren_s_split_dash`
- `setopt_subset::setopt_ksh_arrays_zero`
- `special_params::dollar_dash_options`

### printf_format_parity (1)

- `escape_sequences::escape_r_carriage_return_byte_count`

### prompt_escapes_parity (4)

- `ansi_attrs::S_standout_emits_sgr`
- `ansi_attrs::s_standout_off_emits_sgr`
- `ps4_audit::pct_S_standout_on`
- `ps4_audit::pct_s_standout_off`

### quoting_parity (2)

- `ansi_c_quote::ansi_quote_carriage_return`
- `ansi_c_quote::ansi_quote_octal_escape_byte_count`

### read_parity (1)

- `count_chars::read_k_reads_n_chars`

### real_world_idioms_parity (1)

- `string_parsing::split_on_colon`

### recent_ports_parity (2)

- `params_special_vars::dollar_zero_default_argv0`
- `utils_wordcount::split_on_explicit_sep`

### setopt_pattern_parity (1)

- `glob_options::nomatch_errors_on_unmatched_glob`

### shift_positional_parity (1)

- `custom_ifs::empty_ifs_concatenates`

### time_keyword_parity (3)

- `time_stderr_present::time_default_stderr_format_has_known_marker`
- `time_stderr_present::time_emits_some_stderr_in_zsh`
- `timefmt::timefmt_J_just_command_name`

### umask_parity (2)

- `inheritance::umask_022_creates_644_file`
- `inheritance::umask_077_creates_600_file`

### zinit_p10k_parity (3)

- `megamonsters::fzf_tab_swap_around_null_delim`
- `p10k_join_split::s_flag_splits_on_separator`
- `p10k_join_split::split_then_join_round_trip`

### zsh_compat_parity_gaps (42)

- `builtins_misc::alias_illegal_equals_syntax`
- `coproc::coproc_sets_bang_to_child_pid`
- `corpus_additional_probes::builtins_keys_line_count_wc`
- `corpus_additional_probes::emulate_sh_posixargzero_option`
- `corpus_additional_probes::glob_qual_stat_prefix_s0`
- `corpus_additional_probes::print_wrapped_array_word`
- `corpus_additional_probes::typeset_plus_x_with_r`
- `corpus_behavior_expansion_b::integer_literal_with_base_hash`
- `corpus_behavior_expansion_b::typeset_float_seconds_builtin`
- `corpus_behavior_expansion_c::options_assoc_keys_sorted`
- `corpus_behavior_expansion_c::param_ok_assoc_keys_single`
- `corpus_behavior_expansion_d::modules_assoc_kv_at`
- `corpus_dash_fc_bulk_a::bulk_background_pid_wait`
- `corpus_dash_fc_bulk_b::bulk_b_terminfo_colors_bracket`
- `corpus_dash_fc_bulk_b::bulk_b_zcompile_tmpfile`
- `corpus_dash_fc_bulk_b::bulk_b_zmodload_calendar_stderr`
- `corpus_dash_fc_bulk_c::bulk_c_coproc_cat_bang`
- `corpus_dash_fc_bulk_e::bulk_e_join_newline_j_flag`
- `corpus_dash_fc_bulk_g::bulk_g_coproc_builtin_true`
- `corpus_dash_fc_bulk_g::bulk_g_terminfo_colors_key`
- `corpus_dash_fc_bulk_g::bulk_g_zmodload_zsh_db_gdbm`
- `corpus_dash_fc_bulk_gf::bulk_gf_fc_row_024`
- `corpus_dash_fc_bulk_h::bulk_h_zcompile_then_rm_zwc`
- `corpus_dash_fc_bulk_h::bulk_h_zmodload_zsh_compwid`
- `corpus_dash_fc_bulk_i::bulk_i_hash_num_widgets`
- `corpus_dash_fc_bulk_j::bulk_j_count_modules_tables`
- `corpus_dash_fc_bulk_j::bulk_j_param_match_tilde_unquoted`
- `corpus_dash_fc_bulk_j::bulk_j_print_OSTYPE_VENDOR_UID`
- `corpus_dash_fc_bulk_k::bulk_k_integer_count_typeset_plus_i`
- `corpus_dash_fc_bulk_l::bulk_l_assoc_Mk_filter_keys`
- `corpus_dash_fc_bulk_l::bulk_l_zmodload_zsh_deltochar`
- `corpus_dash_fc_bulk_o::bulk_o_fc_assoc_sorted_values_ov`
- `corpus_dash_fc_bulk_p::bulk_p_fc_zcompile_empty_file`
- `corpus_dash_fc_bulk_pq::bulk_pq_fc_row_042`
- `corpus_dash_fc_bulk_r::bulk_r_fc_background_null_wait_bang`
- `corpus_dash_fc_language_surface::ksharrays_zero_based_index`
- `corpus_dash_fc_language_surface::typeset_plus_list_all`
- `corpus_dash_fc_language_surface::typeset_plus_m_name_list`
- `expansion_eval_arithmetic::nomatch_when_nonomatch_unset`
- `expansion_eval_arithmetic::process_substitution_word`
- `parity_harness::corpus_parity` *(reported under this binary too)*
- `typeset_and_dump::export_minus_p_full_dump`

### zsh_idioms_parity (1)

- `nested_expansions::split_via_var`

## Themes

Failure categories visible above:

- **(k) options assoc-keys ordering / sort.** `options_assoc_keys_sorted`,
  `param_ok_assoc_keys_single`, `bulk_l_assoc_Mk_filter_keys`,
  `bulk_o_fc_assoc_sorted_values_ov`. Mirrors tier-3 task items #13-#16.
- **typeset +X listing arms.** `typeset_plus_x_with_r`, `typeset_plus_list_all`,
  `typeset_plus_m_name_list`, `bulk_k_integer_count_typeset_plus_i`,
  `export_minus_p_full_dump`, `typeset_float_seconds_builtin`. Tier-1 task #5
  + tier-2 #12 territory.
- **terminfo / termcap byte-exact emit.** `terminfo_k*_byte_exact`,
  `terminfo_bold_and_sgr0`, `bulk_b_terminfo_colors_bracket`,
  `bulk_g_terminfo_colors_key`. Partially addressed by the 978376c4ea
  `bin_echotc` tputs/tgoto port; remaining gaps likely live in
  `bin_echoti` or the colors-cap special path.
- **`%S`/`%s` standout in PS1/PS4.** `ansi_attrs::*_standout_*`,
  `ps4_audit::pct_*_standout_*`. Whole arm pending.
- **glob-qualifier no-match exit & stderr.** `nomatch_*`, `qual_*`.
  Tier-1 task #4 + #7.
- **`coproc`/background-pid `$!` wiring.** `coproc_sets_bang_to_child_pid`,
  `bulk_c_coproc_cat_bang`, `bulk_g_coproc_builtin_true`,
  `bulk_background_pid_wait`, `bulk_r_fc_background_null_wait_bang`.
- **`zmodload` + module-side initialization.** `bulk_b_zmodload_calendar_stderr`,
  `bulk_g_zmodload_zsh_db_gdbm`, `bulk_h_zmodload_zsh_compwid`,
  `bulk_l_zmodload_zsh_deltochar`, `bulk_j_count_modules_tables`,
  `modules_assoc_kv_at`.
- **`zcompile` round-trip / wordcode parity.** `zcompdump_*`,
  `bulk_b_zcompile_tmpfile`, `bulk_h_zcompile_then_rm_zwc`,
  `bulk_p_fc_zcompile_empty_file`, `zstyle_canonical_roundtrip`.
- **Single-binary corpus parity rows** (`corpus_parity`, `bulk_gf_fc_row_024`,
  `bulk_ah_fc_row_089`, `bulk_pq_fc_row_042`). One-off corpus entries that
  exercise multiple subsystems each.

## Refresh procedure

To regenerate this doc after a change:

```sh
cargo test --test '*parity*' --no-fail-fast 2>&1 \
    | grep -E '^test result:' \
    | awk '{p+=$4; f+=$6; i+=$8} END {print "passed:",p,"failed:",f,"ignored:",i}'

# Per-binary failing-test names:
for t in tests/*parity*.rs; do
    name="${t#tests/}"; name="${name%.rs}"
    fails=$(cargo test --test "$name" --no-fail-fast 2>&1 \
        | grep -aE '^test .* FAILED$' \
        | sed 's/^test //;s/ \.\.\. FAILED$//')
    [[ -n "$fails" ]] && {
        printf '### %s (%d)\n\n' "$name" "$(echo "$fails" | wc -l | tr -d ' ')"
        echo "$fails" | sort | sed 's/^/- `/;s/$/`/'
        echo
    }
done
```

`cargo test --test '*parity*'` stops on first failure without
`--no-fail-fast`; always include the flag.
