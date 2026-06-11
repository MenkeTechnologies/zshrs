# Parity Test Failures

Snapshot of `cargo test --test '*parity*' --no-fail-fast` results.

**Last run:** 2026-06-11

## Summary

| Metric              | Count  |
| ------------------- | ------ |
| Total tests         | 43,890 |
| Passing             | 43,816 |
| **Failing**         | **74** |
| Ignored             | 8      |
| Pass rate           | 99.83% |
| Test binaries       | 82     |
| Binaries with fails | 12     |

Delta vs the 2026-06-10 snapshot: 106 → 74 failing (32 closed), 25 → 12
failing binaries. Suites that went fully green since: modules_parity,
cond_parity, setopt_pattern_parity, glob_numeric_parity,
prompt_escapes_parity, real_world_idioms_parity, zsh_idioms_parity,
printf_format_parity, quoting_parity, dollar_lt_parity, alias_parity
(test was environment-broken — /opt/X11/bin/x), umask_parity,
zsh_compat dropped 42 → 47-range into the corpus rows below.

## Per-binary failures

### assoc_array_deep_parity (2)

- `lookup::lookup_key_with_space`
- `special_keys::empty_string_key`

### binary_parity (3)

- `zcompdump_byte_identical_roundtrip`
- `zcompdump_synthesize_format`
- `zstyle_canonical_roundtrip`

### discovered_parity_failures (2)

- `parity_Q_flag_on_literal_should_error`
- `parity_flag_on_literal_L_should_error`

### fd_redirect_parity (2)

- `close_fd::close_stdout_then_print_errors`
- `swap_stdout_stderr::swap_stdout_and_stderr`

### parity_harness (1)

- `corpus_parity`

### parity_survey_fc (11)

- `functions::private_scoping`
- `glob_qualifiers::qual_at_symlinks_only`
- `glob_qualifiers::qual_dot_regular_files`
- `glob_qualifiers::qual_e_glob_excl`
- `glob_qualifiers::qual_slash_dirs_only`
- `parameter_filters::subscript_n_nth_match`
- `paren_flags::paren_A_assign_assoc`
- `paren_flags::paren_B_begin_offset`
- `paren_flags::paren_E_end_offset`
- `paren_flags::paren_N_match_length`
- `setopt_subset::setopt_ksh_arrays_zero`
- `special_params::dollar_dash_options`

### read_parity (1)

- `count_chars::read_k_reads_n_chars`

### recent_ports_parity (1)

- `params_special_vars::dollar_zero_default_argv0`

### shift_positional_parity (1)

- `custom_ifs::empty_ifs_concatenates`

### time_keyword_parity (3)

- `time_stderr_present::time_default_stderr_format_has_known_marker`
- `time_stderr_present::time_emits_some_stderr_in_zsh`
- `timefmt::timefmt_J_just_command_name`

### zinit_p10k_parity (1)

- `megamonsters::fzf_tab_swap_around_null_delim`

### zsh_compat_parity_gaps (47)

- `builtins_misc::alias_illegal_equals_syntax`
- `coproc::coproc_sets_bang_to_child_pid`
- `corpus_additional_probes::builtins_keys_line_count_wc`
- `corpus_additional_probes::emulate_sh_posixargzero_option`
- `corpus_additional_probes::glob_qual_stat_prefix_s0`
- `corpus_additional_probes::print_wrapped_array_word`
- `corpus_additional_probes::read_q_noninteractive_herestring`
- `corpus_additional_probes::read_t0_k1_herestring`
- `corpus_additional_probes::typeset_plus_x_with_r`
- `corpus_behavior_expansion_b::integer_literal_with_base_hash`
- `corpus_behavior_expansion_b::typeset_float_seconds_builtin`
- `corpus_behavior_expansion_c::options_assoc_keys_sorted`
- `corpus_behavior_expansion_c::param_ok_assoc_keys_single`
- `corpus_behavior_expansion_d::modules_assoc_kv_at`
- `corpus_dash_fc_bulk_a::bulk_background_pid_wait`
- `corpus_dash_fc_bulk_aal::bulk_aal_fc_row_031`
- `corpus_dash_fc_bulk_abp::bulk_abp_fc_row_044`
- `corpus_dash_fc_bulk_agk::bulk_agk_fc_row_013`
- `corpus_dash_fc_bulk_ah::bulk_ah_fc_row_089`
- `corpus_dash_fc_bulk_b::bulk_b_zcompile_tmpfile`
- `corpus_dash_fc_bulk_b::bulk_b_zmodload_calendar_stderr`
- `corpus_dash_fc_bulk_c::bulk_c_coproc_cat_bang`
- `corpus_dash_fc_bulk_e::bulk_e_join_newline_j_flag`
- `corpus_dash_fc_bulk_g::bulk_g_coproc_builtin_true`
- `corpus_dash_fc_bulk_g::bulk_g_zmodload_zsh_db_gdbm`
- `corpus_dash_fc_bulk_h::bulk_h_zcompile_then_rm_zwc`
- `corpus_dash_fc_bulk_h::bulk_h_zmodload_zsh_compwid`
- `corpus_dash_fc_bulk_i::bulk_i_hash_num_widgets`
- `corpus_dash_fc_bulk_j::bulk_j_count_modules_tables`
- `corpus_dash_fc_bulk_j::bulk_j_param_match_tilde_unquoted`
- `corpus_dash_fc_bulk_j::bulk_j_print_OSTYPE_VENDOR_UID`
- `corpus_dash_fc_bulk_k::bulk_k_integer_count_typeset_plus_i`
- `corpus_dash_fc_bulk_l::bulk_l_assoc_Mk_filter_keys`
- `corpus_dash_fc_bulk_l::bulk_l_read_timeout_zero_herestring`
- `corpus_dash_fc_bulk_l::bulk_l_zmodload_zsh_deltochar`
- `corpus_dash_fc_bulk_o::bulk_o_fc_assoc_sorted_values_ov`
- `corpus_dash_fc_bulk_p::bulk_p_fc_zcompile_empty_file`
- `corpus_dash_fc_bulk_r::bulk_r_fc_background_null_wait_bang`
- `corpus_dash_fc_language_surface::ksharrays_zero_based_index`
- `corpus_dash_fc_language_surface::typeset_plus_list_all`
- `corpus_dash_fc_language_surface::typeset_plus_m_name_list`
- `expansion_eval_arithmetic::nomatch_when_nonomatch_unset`
- `expansion_eval_arithmetic::process_substitution_word`
- `io_and_read::read_k_one_byte_non_tty`
- `typeset_and_dump::export_minus_p_full_dump`

## Themes

- **typeset/export listing arms.** `typeset_plus_*`, `export_minus_p_full_dump`,
  `typeset_float_seconds_builtin`, `bulk_k_integer_count_typeset_plus_i`,
  `typeset_plus_x_with_r`.
- **options/assoc key ordering.** `options_assoc_keys_sorted`,
  `param_ok_assoc_keys_single`, `bulk_l_assoc_Mk_filter_keys`,
  `bulk_o_fc_assoc_sorted_values_ov`.
- **coproc / background `$!` wiring.** `coproc_sets_bang_to_child_pid`,
  `bulk_c_coproc_cat_bang`, `bulk_g_coproc_builtin_true`,
  `bulk_background_pid_wait`, `bulk_r_fc_background_null_wait_bang`.
- **read -k/-q/-t on non-tty stdin.** `read_k_reads_n_chars`,
  `read_k_one_byte_non_tty`, `read_q_noninteractive_herestring`,
  `read_t0_k1_herestring`, `bulk_l_read_timeout_zero_herestring`.
- **glob qualifiers.** `qual_*` (parity_survey_fc),
  `glob_qual_stat_prefix_s0`.
- **zcompile / zcompdump binary formats.** `zcompdump_*`,
  `zstyle_canonical_roundtrip`, `bulk_b_zcompile_tmpfile`,
  `bulk_h_zcompile_then_rm_zwc`, `bulk_p_fc_zcompile_empty_file`.
- **zmodload of unported modules.** calendar, zsh/db/gdbm,
  zsh/compwid, zsh/deltochar stderr/exit divergences.
- **time keyword stderr.** whole arm pending.
- **paren flags (A/B/E/N).** match-offset reporting flags pending.
