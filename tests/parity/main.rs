//! Aggregated parity-test binary — the entire parity suite as ONE target.
//!
//! Every parity `.rs` file in this directory is pulled in below as a
//! module, so the whole suite builds and runs together:
//!
//!     cargo test --test parity                 # run every parity test
//!     cargo test --test parity zpwr_corpus     # filter to one module
//!     cargo test --test parity -- --ignored    # run the documented gaps
//!
//! Cargo does NOT auto-discover files in this subdir; the `[[test]]`
//! stanza in Cargo.toml makes this file the single discovered target.
//! Add a parity file by dropping it here and adding one `mod` line below.

#![allow(non_snake_case)]
#![allow(dead_code)]
#![allow(clippy::doc_lazy_continuation)]
#![allow(clippy::needless_raw_string_hashes)]

mod parser_lock;

mod advanced_parity;
mod alias_parity;
mod always_block_parity;
mod assoc_array_deep_parity;
mod autoload_parity;
mod bare_name_subscript_default_parity;
mod bash_param_compat_parity;
mod bash_param_transform_parity;
mod bash_shopt_parity;
mod binary_parity;
mod bindkey_parity;
mod brace_parity;
mod builtin_c_parity;
mod builtin_misc_parity;
mod builtin_print_parity;
mod builtins_parity;
mod builtin_module_surface_parity;
mod case_parity;
mod cd_options_parity;
mod cmdsubst_parity;
mod command_builtin_parity;
mod command_precedence_parity;
mod cond_parity;
mod config_state_parity;
mod coproc_parity;
mod dirstack_parity;
mod discovered_gaps_2026q2_parity;
mod discovered_parity_failures;
mod dollar_lt_parity;
mod echo_builtin_parity;
mod eval_builtin_parity;
mod emulation_builtin_fmt_parity;
mod emulation_trap_function_parity;
mod exec_parity;
mod expansion_parity;
mod export_unset_parity;
mod extended_glob_parity;
mod fd_redirect_parity;
mod for_arith_store_parity;
mod funcstack_trace_parity;
mod function_parity;
mod functions_hashtable_parity;
mod fuzz_discovered_parity;
mod getopts_deep_parity;
mod glob_numeric_parity;
mod glob_parity;
mod glob_qualifiers_real_parity;
mod here_string_parity;
mod heredoc_parity;
mod if_elif_parity;
mod ifs_parity;
mod jobs_parity;
mod kill_signal_parity;
mod let_arith_command_parity;
mod lexer_parity;
mod local_tied_special_parity;
mod loops_parity;
mod magic_equal_subst_parity;
mod magic_hash_parity;
mod man_zshall_corpus_parity;
mod math_parity;
mod modules_parity;
mod noclobber_parity;
mod numeric_format_gaps_parity;
mod numeric_sort_parity;
mod omz_repo_corpus_parity;
mod omz_snippet_corpus_parity;
mod options_parity;
mod parity_harness;
mod parity_survey_fc;
mod pipeline_parity;
mod precmd_keyword_parity;
mod print_flag_interaction_parity;
mod printf_format_parity;
mod printf_percent_q_parity;
mod prompt_escapes_parity;
mod prompt_features_corpus_parity;
mod quoting_parity;
mod read_advanced_parity;
mod read_parity;
mod real_world_idioms_parity;
mod recent_ports_parity;
mod redirection_parity;
mod regex_match_parity;
mod repeat_select_parity;
mod runtime_context_parity;
mod sched_prompt_parity;
mod session_regression_parity;
mod shell_semantics_parity;
mod setopt_pattern_parity;
mod shift_positional_parity;
mod special_params_parity;
mod special_runtime_params_parity;
mod subshell_parity;
mod subshell_signal_limit_parity;
mod subst_flags_more_parity;
mod subst_split_join_parity;
mod tilde_parity;
mod time_keyword_parity;
mod trap_parity;
mod typeset_parity;
mod umask_parity;
mod unset_deep_parity;
mod whence_parity;
mod wordcode_parity;
mod xtrace_corpus_parity;
mod zdharma_corpus_parity;
mod zinit_p10k_parity;
mod zinit_plugin_corpus_parity;
mod zpwr_corpus_parity;
mod always_break_parity;
mod terminfo_parity;
mod zsh_arrays_parity;
mod zsh_compat_parity_gaps;
mod zsh_idioms_parity;
mod zsh_modules_corpus_parity;
