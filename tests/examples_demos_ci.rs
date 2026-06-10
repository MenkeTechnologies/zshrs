//! CI runner — every `examples/demos/*.zsh` must execute clean under
//! `zshrs --zsh`.
//!
//! Each demo is a self-contained, deterministic zsh script that
//! exercises a specific feature surface (arithmetic, arrays, assoc
//! arrays, control flow, functions, recursion, brace/parameter
//! expansion, heredocs, command/process substitution, pipes,
//! printf, traps, IFS-splitting, pattern matching, etc.). The CI
//! harness asserts:
//!
//!  1. Exit code `0` (no panic, no zshrs internal error).
//!  2. Non-empty stdout (every demo prints something verifiable).
//!  3. No `panic`/`assertion failed`/`thread '.*' panicked` markers
//!     in stderr (Rust-internal crashes leak through these tokens).
//!
//! Each demo gets its own `#[test]` so cargo-test reports per-demo
//! pass/fail and parallel runners (`cargo test --jobs N`) execute
//! independently. Tests skip silently if the `zshrs` binary isn't
//! built (matches existing `recent_ports_parity.rs` pattern so
//! local-dev flows that haven't run `cargo build` aren't penalized).
//!
//! Per-test timeout: 30s wall-clock. Real-world demos finish well
//! under 500ms each; a >30s run signals a regression worth failing
//! on.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn zshrs_bin() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    [
        manifest.join("target/debug/zshrs"),
        manifest.join("target/release/zshrs"),
    ]
    .into_iter()
    .find(|cand| cand.exists())
}

fn demos_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/demos")
}

/// Spawn `zshrs --zsh <script>` with a wall-clock cap. Returns
/// (exit-code, stdout, stderr) on completion. Panics on spawn
/// failure or timeout.
fn run_demo(bin: &Path, script: &Path) -> (i32, String, String) {
    let mut child = Command::new(bin)
        .args(["--zsh"])
        .arg(script)
        .env_remove("ZSHRS_CACHE")
        .env_remove("ZDOTDIR")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn zshrs");

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut out = Vec::new();
                let mut err = Vec::new();
                if let Some(mut so) = child.stdout.take() {
                    use std::io::Read;
                    let _ = so.read_to_end(&mut out);
                }
                if let Some(mut se) = child.stderr.take() {
                    use std::io::Read;
                    let _ = se.read_to_end(&mut err);
                }
                return (
                    status.code().unwrap_or(-1),
                    String::from_utf8_lossy(&out).into_owned(),
                    String::from_utf8_lossy(&err).into_owned(),
                );
            }
            Ok(None) => {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    panic!(
                        "demo {} exceeded 30s wall-clock — likely a regression",
                        script.display()
                    );
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("waitpid {}: {e}", script.display()),
        }
    }
}

/// Per-demo pass: exit-0, non-empty stdout, no panic-prefixed stderr.
fn assert_demo_runs_clean(script_name: &str) {
    let bin = match zshrs_bin() {
        Some(b) => b,
        None => {
            eprintln!("skip: zshrs binary not built — run `cargo build` first");
            return;
        }
    };
    let script = demos_dir().join(script_name);
    assert!(
        script.exists(),
        "demo {} missing — file rename without test update?",
        script.display()
    );

    let (code, stdout, stderr) = run_demo(&bin, &script);

    assert_eq!(
        code,
        0,
        "{} exited {} (expected 0)\nstderr:\n{}\nstdout-tail:\n{}",
        script_name,
        code,
        stderr,
        stdout.lines().rev().take(20).collect::<Vec<_>>().join("\n")
    );

    assert!(
        !stdout.trim().is_empty(),
        "{} produced empty stdout — demos must print verifiable output",
        script_name
    );

    for needle in ["panicked at", "panic:", "assertion failed"] {
        assert!(
            !stderr.contains(needle),
            "{} stderr contained Rust-panic token {:?}:\n{}",
            script_name,
            needle,
            stderr
        );
    }
}

// One #[test] per demo so cargo reports per-demo pass/fail and
// parallel test runners execute independently.
macro_rules! demo_test {
    ($name:ident, $file:expr) => {
        #[test]
        fn $name() {
            assert_demo_runs_clean($file);
        }
    };
}

demo_test!(d01_hello, "01_hello.zsh");
demo_test!(d02_arithmetic, "02_arithmetic.zsh");
demo_test!(d03_strings, "03_strings.zsh");
demo_test!(d04_arrays, "04_arrays.zsh");
demo_test!(d05_assoc_arrays, "05_assoc_arrays.zsh");
demo_test!(d06_control_flow, "06_control_flow.zsh");
demo_test!(d07_for_loops, "07_for_loops.zsh");
demo_test!(d08_functions, "08_functions.zsh");
demo_test!(d09_recursion, "09_recursion.zsh");
demo_test!(d10_fizzbuzz, "10_fizzbuzz.zsh");
demo_test!(d11_fibonacci, "11_fibonacci.zsh");
demo_test!(d12_quicksort, "12_quicksort.zsh");
demo_test!(d13_prime_sieve, "13_prime_sieve.zsh");
demo_test!(d14_brace_expansion, "14_brace_expansion.zsh");
demo_test!(d15_parameter_expansion, "15_parameter_expansion.zsh");
demo_test!(d16_parameter_flags, "16_parameter_flags.zsh");
demo_test!(d17_heredocs, "17_heredocs.zsh");
demo_test!(d18_cmd_substitution, "18_cmd_substitution.zsh");
demo_test!(d19_pipes_and_filters, "19_pipes_and_filters.zsh");
demo_test!(d20_process_substitution, "20_process_substitution.zsh");
demo_test!(d21_printf_demo, "21_printf_demo.zsh");
demo_test!(d22_trap_exit, "22_trap_exit.zsh");
demo_test!(d23_ifs_split, "23_ifs_split.zsh");
demo_test!(d24_word_count, "24_word_count.zsh");
demo_test!(d25_reverse_string, "25_reverse_string.zsh");
demo_test!(d26_anonymous_fn, "26_anonymous_fn.zsh");
demo_test!(d27_positional_args, "27_positional_args.zsh");
demo_test!(d28_typeset, "28_typeset.zsh");
demo_test!(d29_matrix_print, "29_matrix_print.zsh");
demo_test!(d30_pattern_match, "30_pattern_match.zsh");
demo_test!(d31_stack, "31_stack.zsh");
demo_test!(d32_queue, "32_queue.zsh");
demo_test!(d33_binary_search, "33_binary_search.zsh");
demo_test!(d34_bubble_sort, "34_bubble_sort.zsh");
demo_test!(d35_insertion_sort, "35_insertion_sort.zsh");
demo_test!(d36_selection_sort, "36_selection_sort.zsh");
demo_test!(d37_counting_sort, "37_counting_sort.zsh");
demo_test!(d38_set_ops, "38_set_ops.zsh");
demo_test!(d39_matrix_multiply, "39_matrix_multiply.zsh");
demo_test!(d40_roman_numerals, "40_roman_numerals.zsh");
demo_test!(d41_base_convert, "41_base_convert.zsh");
demo_test!(d42_tower_of_hanoi, "42_tower_of_hanoi.zsh");
demo_test!(d43_collatz, "43_collatz.zsh");
demo_test!(d44_happy_numbers, "44_happy_numbers.zsh");
demo_test!(d45_armstrong, "45_armstrong.zsh");
demo_test!(d46_perfect_numbers, "46_perfect_numbers.zsh");
demo_test!(d47_rot13, "47_rot13.zsh");
demo_test!(d48_atbash, "48_atbash.zsh");
demo_test!(d49_word_reverse, "49_word_reverse.zsh");
demo_test!(d50_histogram, "50_histogram.zsh");
demo_test!(d51_csv_parse, "51_csv_parse.zsh");
demo_test!(d52_env_basics, "52_env_basics.zsh");
demo_test!(d53_file_tests, "53_file_tests.zsh");
demo_test!(d54_date_format, "54_date_format.zsh");
demo_test!(d55_read_loop, "55_read_loop.zsh");
demo_test!(d56_exit_codes, "56_exit_codes.zsh");
demo_test!(d57_atoi_itoa, "57_atoi_itoa.zsh");
demo_test!(d58_gcd_lcm, "58_gcd_lcm.zsh");
demo_test!(d59_string_reverse_ops, "59_string_reverse_ops.zsh");
demo_test!(d60_mapfile_like, "60_mapfile_like.zsh");
demo_test!(d61_zsh_modifiers, "61_zsh_modifiers.zsh");
demo_test!(d62_param_flags_match, "62_param_flags_match.zsh");
demo_test!(d63_param_flags_join_split, "63_param_flags_join_split.zsh");
demo_test!(d64_param_flags_case, "64_param_flags_case.zsh");
demo_test!(d65_param_flags_sort, "65_param_flags_sort.zsh");
demo_test!(d66_param_flags_format, "66_param_flags_format.zsh");
demo_test!(d67_glob_qualifiers, "67_glob_qualifiers.zsh");
demo_test!(d68_extended_glob, "68_extended_glob.zsh");
demo_test!(d69_assoc_advanced, "69_assoc_advanced.zsh");
demo_test!(d70_array_set_ops_zsh, "70_array_set_ops_zsh.zsh");
demo_test!(d71_array_pattern_filter, "71_array_pattern_filter.zsh");
demo_test!(d72_typeset_int_base, "72_typeset_int_base.zsh");
demo_test!(d73_print_columnar, "73_print_columnar.zsh");
demo_test!(d74_print_prompt_escapes, "74_print_prompt_escapes.zsh");
demo_test!(d75_zparseopts, "75_zparseopts.zsh");
demo_test!(d76_mathfunc, "76_mathfunc.zsh");
demo_test!(d77_datetime, "77_datetime.zsh");
demo_test!(d78_setopt_local_scope, "78_setopt_local_scope.zsh");
demo_test!(d79_eval_dynamic_dispatch, "79_eval_dynamic_dispatch.zsh");
demo_test!(d80_anon_fn_args, "80_anon_fn_args.zsh");
demo_test!(d81_compound_defaults, "81_compound_defaults.zsh");
demo_test!(d82_brace_advanced, "82_brace_advanced.zsh");
demo_test!(d83_history_modifiers, "83_history_modifiers.zsh");
demo_test!(d84_subst_split_complex, "84_subst_split_complex.zsh");
demo_test!(d85_zcalc_repl, "85_zcalc_repl.zsh");
demo_test!(d86_setopt_exhaustive, "86_setopt_exhaustive.zsh");
demo_test!(d87_read_advanced, "87_read_advanced.zsh");
demo_test!(d88_printf_format_advanced, "88_printf_format_advanced.zsh");
demo_test!(d89_regex_match, "89_regex_match.zsh");
demo_test!(d90_type_whence, "90_type_whence.zsh");
demo_test!(d91_hash_builtin, "91_hash_builtin.zsh");
demo_test!(d92_arithmetic_for, "92_arithmetic_for.zsh");
demo_test!(d93_nested_assoc, "93_nested_assoc.zsh");
demo_test!(d94_case_advanced, "94_case_advanced.zsh");
demo_test!(d95_fd_redirection, "95_fd_redirection.zsh");
demo_test!(d96_strict_mode, "96_strict_mode.zsh");
demo_test!(d97_indirection, "97_indirection.zsh");
demo_test!(d98_coreutils_builtins, "98_coreutils_builtins.zsh");
demo_test!(d99_negative_indexing, "99_negative_indexing.zsh");
demo_test!(d100_zsh_features_summary, "100_zsh_features_summary.zsh");
demo_test!(d101_subshell_grouping, "101_subshell_grouping.zsh");
demo_test!(
    d102_function_introspection,
    "102_function_introspection.zsh"
);
demo_test!(d103_exit_traps_advanced, "103_exit_traps_advanced.zsh");
demo_test!(d104_strict_arithmetic, "104_strict_arithmetic.zsh");
demo_test!(d105_dispatch_table, "105_dispatch_table.zsh");
demo_test!(d106_pipe_chains, "106_pipe_chains.zsh");
demo_test!(d107_eval_metaprogramming, "107_eval_metaprogramming.zsh");
demo_test!(d108_globsubst_globalias, "108_globsubst_globalias.zsh");
demo_test!(d109_arith_truth_tables, "109_arith_truth_tables.zsh");
demo_test!(d110_misc_advanced, "110_misc_advanced.zsh");
demo_test!(d111_let_builtin, "111_let_builtin.zsh");
demo_test!(d112_assignment_forms, "112_assignment_forms.zsh");
demo_test!(d113_tied_arrays, "113_tied_arrays.zsh");
demo_test!(d114_local_modifiers, "114_local_modifiers.zsh");
demo_test!(d115_param_strip_advanced, "115_param_strip_advanced.zsh");
demo_test!(d116_cond_numeric_ops, "116_cond_numeric_ops.zsh");
demo_test!(d117_backref_replacement, "117_backref_replacement.zsh");
demo_test!(d118_recursive_glob, "118_recursive_glob.zsh");
demo_test!(d119_background_wait, "119_background_wait.zsh");
demo_test!(d120_utf8_strings, "120_utf8_strings.zsh");
demo_test!(d121_mini_cat, "121_mini_cat.zsh");
demo_test!(d122_mini_grep, "122_mini_grep.zsh");
demo_test!(d123_mini_wc, "123_mini_wc.zsh");
demo_test!(d124_url_encode, "124_url_encode.zsh");
demo_test!(d125_json_pretty, "125_json_pretty.zsh");
demo_test!(d126_xml_escape, "126_xml_escape.zsh");
demo_test!(d127_string_trim, "127_string_trim.zsh");
demo_test!(d128_csv_writer, "128_csv_writer.zsh");
demo_test!(d129_assoc_serialize, "129_assoc_serialize.zsh");
demo_test!(d130_ini_parser, "130_ini_parser.zsh");
demo_test!(d131_emulate_modes, "131_emulate_modes.zsh");
demo_test!(d132_ksh_patterns, "132_ksh_patterns.zsh");
demo_test!(d133_zstyle_demo, "133_zstyle_demo.zsh");
demo_test!(d134_compdef_signatures, "134_compdef_signatures.zsh");
demo_test!(d135_bindkey_config, "135_bindkey_config.zsh");
demo_test!(d136_path_manipulation, "136_path_manipulation.zsh");
demo_test!(d137_named_pipes, "137_named_pipes.zsh");
demo_test!(d138_lock_files, "138_lock_files.zsh");
demo_test!(d139_env_manipulation, "139_env_manipulation.zsh");
demo_test!(d140_signal_handling, "140_signal_handling.zsh");
demo_test!(d141_color_codes, "141_color_codes.zsh");
demo_test!(d142_calc_engine, "142_calc_engine.zsh");
demo_test!(d143_todo_app, "143_todo_app.zsh");
demo_test!(d144_graph_bfs, "144_graph_bfs.zsh");
demo_test!(d145_state_machine, "145_state_machine.zsh");
demo_test!(d146_topological_sort, "146_topological_sort.zsh");
demo_test!(d147_pomodoro_timer, "147_pomodoro_timer.zsh");
demo_test!(d148_inventory_system, "148_inventory_system.zsh");
demo_test!(d149_event_log, "149_event_log.zsh");
demo_test!(d150_lru_cache, "150_lru_cache.zsh");
demo_test!(d151_priority_queue, "151_priority_queue.zsh");
demo_test!(d152_bloom_filter, "152_bloom_filter.zsh");
demo_test!(d153_trie, "153_trie.zsh");
demo_test!(d154_levenshtein, "154_levenshtein.zsh");
demo_test!(d155_text_diff, "155_text_diff.zsh");
demo_test!(d156_simple_template, "156_simple_template.zsh");
demo_test!(d157_observer_pattern, "157_observer_pattern.zsh");
demo_test!(d158_simulate_random, "158_simulate_random.zsh");
demo_test!(d159_bank_account, "159_bank_account.zsh");
demo_test!(d160_zshrs_capabilities, "160_zshrs_capabilities.zsh");
demo_test!(d161_dirstack, "161_dirstack.zsh");
demo_test!(d162_umask_ulimit, "162_umask_ulimit.zsh");
demo_test!(d163_quoting_flags, "163_quoting_flags.zsh");
demo_test!(d164_z_split_shell, "164_z_split_shell.zsh");
demo_test!(d165_print_advanced, "165_print_advanced.zsh");
demo_test!(d166_glob_flags, "166_glob_flags.zsh");
demo_test!(d167_mini_find, "167_mini_find.zsh");
demo_test!(d168_mini_make, "168_mini_make.zsh");
demo_test!(d169_markdown_to_text, "169_markdown_to_text.zsh");
demo_test!(d170_regex_tester, "170_regex_tester.zsh");
demo_test!(d171_moving_average, "171_moving_average.zsh");
demo_test!(d172_password_check, "172_password_check.zsh");
demo_test!(d173_anagram_finder, "173_anagram_finder.zsh");
demo_test!(d174_number_to_words, "174_number_to_words.zsh");
demo_test!(d175_ascii_chart, "175_ascii_chart.zsh");
demo_test!(d176_game_of_life, "176_game_of_life.zsh");
demo_test!(d177_days_between, "177_days_between.zsh");
demo_test!(d178_maze_generator, "178_maze_generator.zsh");
demo_test!(d179_lottery_sim, "179_lottery_sim.zsh");
demo_test!(d180_calendar, "180_calendar.zsh");
demo_test!(d181_fizzbuzz_variants, "181_fizzbuzz_variants.zsh");
demo_test!(d182_progress_bar, "182_progress_bar.zsh");
demo_test!(d183_word_frequency, "183_word_frequency.zsh");
demo_test!(d184_simple_cron, "184_simple_cron.zsh");
demo_test!(d185_final_recap, "185_final_recap.zsh");
demo_test!(d186_alias_forms, "186_alias_forms.zsh");
demo_test!(d187_hex_dump, "187_hex_dump.zsh");
demo_test!(d188_ip_parser, "188_ip_parser.zsh");
demo_test!(d189_http_status, "189_http_status.zsh");
demo_test!(d190_ansi_stripper, "190_ansi_stripper.zsh");
demo_test!(d191_retry_backoff, "191_retry_backoff.zsh");
demo_test!(d192_memoize, "192_memoize.zsh");
demo_test!(d193_log_rotate, "193_log_rotate.zsh");
demo_test!(d194_url_parser, "194_url_parser.zsh");
demo_test!(d195_sha_simple_hash, "195_sha_simple_hash.zsh");
demo_test!(d196_base64, "196_base64.zsh");
demo_test!(d197_csv_full_parse, "197_csv_full_parse.zsh");
demo_test!(d198_yaml_lite, "198_yaml_lite.zsh");
demo_test!(d199_color_picker, "199_color_picker.zsh");
demo_test!(d200_milestone, "200_milestone.zsh");
demo_test!(d201_unit_converter, "201_unit_converter.zsh");
demo_test!(d202_tokenizer, "202_tokenizer.zsh");
demo_test!(d203_argv_dispatch, "203_argv_dispatch.zsh");
demo_test!(d204_string_interpolation, "204_string_interpolation.zsh");
demo_test!(d205_zsh_in_scripts, "205_zsh_in_scripts.zsh");
demo_test!(d206_assoc_iteration, "206_assoc_iteration.zsh");
demo_test!(d207_lru_with_ttl, "207_lru_with_ttl.zsh");
demo_test!(d208_directory_walker, "208_directory_walker.zsh");
demo_test!(d209_command_pipeline, "209_command_pipeline.zsh");
demo_test!(d210_quine, "210_quine.zsh");
demo_test!(d211_csv_to_md, "211_csv_to_md.zsh");
demo_test!(d212_markdown_table, "212_markdown_table.zsh");
demo_test!(d213_ssh_config_parser, "213_ssh_config_parser.zsh");
demo_test!(d214_chess_board, "214_chess_board.zsh");
demo_test!(d215_tic_tac_toe, "215_tic_tac_toe.zsh");
demo_test!(d216_deck_of_cards, "216_deck_of_cards.zsh");
demo_test!(d217_guess_number, "217_guess_number.zsh");
demo_test!(d218_quiz_game, "218_quiz_game.zsh");
demo_test!(d219_madlibs, "219_madlibs.zsh");
demo_test!(d220_expense_tracker, "220_expense_tracker.zsh");
demo_test!(d221_zsh_xtrace, "221_zsh_xtrace.zsh");
demo_test!(d222_zsh_psvars, "222_zsh_psvars.zsh");
demo_test!(d223_funcstack, "223_funcstack.zsh");
demo_test!(d224_git_log_parser, "224_git_log_parser.zsh");
demo_test!(d225_nginx_log_analyze, "225_nginx_log_analyze.zsh");
demo_test!(d226_todo_categories, "226_todo_categories.zsh");
demo_test!(d227_hashtable_oa, "227_hashtable_oa.zsh");
demo_test!(d228_stack_machine, "228_stack_machine.zsh");
demo_test!(d229_search_filter, "229_search_filter.zsh");
demo_test!(d230_menu_system, "230_menu_system.zsh");
demo_test!(d231_text_adventure, "231_text_adventure.zsh");
demo_test!(d232_time_tracker, "232_time_tracker.zsh");
demo_test!(d233_word_chain, "233_word_chain.zsh");
demo_test!(d234_simple_kvs, "234_simple_kvs.zsh");
demo_test!(d235_grand_finale, "235_grand_finale.zsh");
demo_test!(d236_zsh_hooks, "236_zsh_hooks.zsh");
demo_test!(d237_zsh_autoload, "237_zsh_autoload.zsh");
demo_test!(d238_dijkstra, "238_dijkstra.zsh");
demo_test!(d239_sudoku_validate, "239_sudoku_validate.zsh");
demo_test!(d240_lights_out, "240_lights_out.zsh");
demo_test!(d241_hangman, "241_hangman.zsh");
demo_test!(d242_number_sequences, "242_number_sequences.zsh");
demo_test!(d243_sierpinski, "243_sierpinski.zsh");
demo_test!(d244_mandelbrot_ascii, "244_mandelbrot_ascii.zsh");
demo_test!(d245_toml_parser, "245_toml_parser.zsh");
demo_test!(d246_env_file_parser, "246_env_file_parser.zsh");
demo_test!(d247_shebang_detector, "247_shebang_detector.zsh");
demo_test!(d248_charset_validator, "248_charset_validator.zsh");
demo_test!(d249_whitespace_normalizer, "249_whitespace_normalizer.zsh");
demo_test!(d250_shopping_cart, "250_shopping_cart.zsh");
demo_test!(d251_vigenere_cipher, "251_vigenere_cipher.zsh");
demo_test!(d252_caesar_cipher, "252_caesar_cipher.zsh");
demo_test!(d253_word_search, "253_word_search.zsh");
demo_test!(d254_ascii_clock, "254_ascii_clock.zsh");
demo_test!(d255_ipv6_parser, "255_ipv6_parser.zsh");
demo_test!(d256_recipe_converter, "256_recipe_converter.zsh");
demo_test!(d257_memory_match, "257_memory_match.zsh");
demo_test!(d258_substitution_cipher, "258_substitution_cipher.zsh");
demo_test!(d259_boggle_solver, "259_boggle_solver.zsh");
demo_test!(d260_final_v3, "260_final_v3.zsh");
demo_test!(d261_prime_factorize, "261_prime_factorize.zsh");
demo_test!(d262_miller_rabin, "262_miller_rabin.zsh");
demo_test!(d263_extended_gcd, "263_extended_gcd.zsh");
demo_test!(d264_a_star_pathfind, "264_a_star_pathfind.zsh");
demo_test!(d265_kruskal_mst, "265_kruskal_mst.zsh");
demo_test!(d266_prim_mst, "266_prim_mst.zsh");
demo_test!(d267_floyd_warshall, "267_floyd_warshall.zsh");
demo_test!(d268_bellman_ford, "268_bellman_ford.zsh");
demo_test!(d269_n_queens, "269_n_queens.zsh");
demo_test!(d270_fifteen_puzzle, "270_fifteen_puzzle.zsh");
demo_test!(d271_hanoi_animated, "271_hanoi_animated.zsh");
demo_test!(d272_markdown_to_html, "272_markdown_to_html.zsh");
demo_test!(d273_http_parser, "273_http_parser.zsh");
demo_test!(d274_log_format_detect, "274_log_format_detect.zsh");
demo_test!(d275_csv_merge, "275_csv_merge.zsh");
demo_test!(d276_blackjack, "276_blackjack.zsh");
demo_test!(d277_dice_game, "277_dice_game.zsh");
demo_test!(d278_rps, "278_rps.zsh");
demo_test!(d279_xor_cipher, "279_xor_cipher.zsh");
demo_test!(d280_otp_pad, "280_otp_pad.zsh");
demo_test!(d281_zsh_periodic, "281_zsh_periodic.zsh");
demo_test!(d282_zsh_argv_special, "282_zsh_argv_special.zsh");
demo_test!(d283_zsh_traps_full, "283_zsh_traps_full.zsh");
demo_test!(d284_atomic_write, "284_atomic_write.zsh");
demo_test!(d285_banner_v4, "285_banner_v4.zsh");
demo_test!(d286_segment_tree, "286_segment_tree.zsh");
demo_test!(d287_fenwick_tree, "287_fenwick_tree.zsh");
demo_test!(d288_kmp_match, "288_kmp_match.zsh");
demo_test!(d289_rabin_karp, "289_rabin_karp.zsh");
demo_test!(d290_manacher, "290_manacher.zsh");
demo_test!(d291_reservoir_sample, "291_reservoir_sample.zsh");
demo_test!(d292_skiplist, "292_skiplist.zsh");
demo_test!(d293_suffix_array, "293_suffix_array.zsh");
demo_test!(d294_word_ladder, "294_word_ladder.zsh");
demo_test!(d295_soundex, "295_soundex.zsh");
demo_test!(d296_minesweeper, "296_minesweeper.zsh");
demo_test!(d297_mastermind, "297_mastermind.zsh");
demo_test!(d298_ttt_minimax, "298_ttt_minimax.zsh");
demo_test!(d299_conway_animated, "299_conway_animated.zsh");
demo_test!(d300_milestone_300, "300_milestone_300.zsh");
demo_test!(d301_url_template, "301_url_template.zsh");
demo_test!(d302_sql_mini_parser, "302_sql_mini_parser.zsh");
demo_test!(d303_zsh_print_z, "303_zsh_print_z.zsh");
demo_test!(d304_zsh_unhash_pattern, "304_zsh_unhash_pattern.zsh");
demo_test!(d305_zsh_typeset_m, "305_zsh_typeset_m.zsh");
demo_test!(d306_zsh_zle_widgets, "306_zsh_zle_widgets.zsh");
demo_test!(d307_zsh_compsys_args, "307_zsh_compsys_args.zsh");
demo_test!(d308_kruskal_algo_density, "308_kruskal_algo_density.zsh");
demo_test!(d309_state_machine_dsl, "309_state_machine_dsl.zsh");
demo_test!(d310_banner_v5, "310_banner_v5.zsh");
demo_test!(d311_bst, "311_bst.zsh");
demo_test!(d312_avl_tree, "312_avl_tree.zsh");
demo_test!(d313_bloom_filter_v2, "313_bloom_filter_v2.zsh");
demo_test!(d314_deque, "314_deque.zsh");
demo_test!(d315_ring_buffer, "315_ring_buffer.zsh");
demo_test!(d316_ipv4_subnet, "316_ipv4_subnet.zsh");
demo_test!(d317_mac_address, "317_mac_address.zsh");
demo_test!(d318_file_checksum, "318_file_checksum.zsh");
demo_test!(d319_anagram_solver, "319_anagram_solver.zsh");
demo_test!(d320_leet_speak, "320_leet_speak.zsh");
demo_test!(d321_pig_latin, "321_pig_latin.zsh");
demo_test!(d322_history_parser, "322_history_parser.zsh");
demo_test!(d323_ssh_known_hosts, "323_ssh_known_hosts.zsh");
demo_test!(d324_brace_advanced, "324_brace_advanced.zsh");
demo_test!(d325_zsh_print_more, "325_zsh_print_more.zsh");
demo_test!(d326_zsh_compinit, "326_zsh_compinit.zsh");
demo_test!(d327_zsh_extended_glob, "327_zsh_extended_glob.zsh");
demo_test!(d328_zsh_kv_assoc, "328_zsh_kv_assoc.zsh");
demo_test!(d329_max_subarray, "329_max_subarray.zsh");
demo_test!(d330_lis_lcs, "330_lis_lcs.zsh");
demo_test!(d331_knapsack, "331_knapsack.zsh");
demo_test!(d332_coin_change, "332_coin_change.zsh");
demo_test!(d333_topological_sort, "333_topological_sort.zsh");
demo_test!(d334_lru_cache, "334_lru_cache.zsh");
demo_test!(d335_banner_v6, "335_banner_v6.zsh");
demo_test!(d336_roman_numeral, "336_roman_numeral.zsh");
demo_test!(d337_trie_advanced, "337_trie_advanced.zsh");
demo_test!(d338_z_function, "338_z_function.zsh");
demo_test!(
    d339_longest_common_substring,
    "339_longest_common_substring.zsh"
);
demo_test!(d340_palindromic_subseq, "340_palindromic_subseq.zsh");
demo_test!(d341_nim_game, "341_nim_game.zsh");
demo_test!(d342_peg_solitaire, "342_peg_solitaire.zsh");
demo_test!(d343_rfc2822_date, "343_rfc2822_date.zsh");
demo_test!(d344_iso8601, "344_iso8601.zsh");
demo_test!(d345_pollard_rho, "345_pollard_rho.zsh");
demo_test!(d346_continued_fraction, "346_continued_fraction.zsh");
demo_test!(d347_transposition_cipher, "347_transposition_cipher.zsh");
demo_test!(d348_color_conversions, "348_color_conversions.zsh");
demo_test!(d349_zsh_eval_context, "349_zsh_eval_context.zsh");
demo_test!(d350_milestone_350, "350_milestone_350.zsh");
demo_test!(d351_sokoban_small, "351_sokoban_small.zsh");
demo_test!(d352_text_wrap, "352_text_wrap.zsh");
demo_test!(d353_unicode_utils, "353_unicode_utils.zsh");
demo_test!(d354_url_encode, "354_url_encode.zsh");
demo_test!(d355_calendar_print, "355_calendar_print.zsh");
demo_test!(d356_disjoint_set, "356_disjoint_set.zsh");
demo_test!(d357_priority_queue, "357_priority_queue.zsh");
demo_test!(d358_zsh_funcfile, "358_zsh_funcfile.zsh");
demo_test!(d359_zsh_param_complete, "359_zsh_param_complete.zsh");
demo_test!(d360_banner_v7, "360_banner_v7.zsh");
demo_test!(d361_json_parser_full, "361_json_parser_full.zsh");
demo_test!(d362_xml_parser_full, "362_xml_parser_full.zsh");
demo_test!(d363_arith_expr_evaluator, "363_arith_expr_evaluator.zsh");
demo_test!(d364_csv_rfc4180, "364_csv_rfc4180.zsh");
demo_test!(d365_mini_lisp, "365_mini_lisp.zsh");
demo_test!(d366_sudoku_solver_bt, "366_sudoku_solver_bt.zsh");
demo_test!(d367_banner_v8, "367_banner_v8.zsh");
demo_test!(d368_bencode_roundtrip, "368_bencode_roundtrip.zsh");
demo_test!(d369_hamming_7_4, "369_hamming_7_4.zsh");
demo_test!(d370_skyline, "370_skyline.zsh");
demo_test!(d371_brainfuck_interp, "371_brainfuck_interp.zsh");
demo_test!(d372_ackermann, "372_ackermann.zsh");
demo_test!(d373_lzw_codec, "373_lzw_codec.zsh");
demo_test!(d374_elias_gamma, "374_elias_gamma.zsh");
demo_test!(d375_banner_v9, "375_banner_v9.zsh");

/// Coverage pin — the directory listing must match the test list
/// 1:1. If a new demo is added without registering it here, this
/// test fails so the CI surface doesn't silently miss demos.
#[test]
fn every_demo_in_dir_has_a_test() {
    let registered: Vec<&str> = vec![
        "01_hello.zsh",
        "02_arithmetic.zsh",
        "03_strings.zsh",
        "04_arrays.zsh",
        "05_assoc_arrays.zsh",
        "06_control_flow.zsh",
        "07_for_loops.zsh",
        "08_functions.zsh",
        "09_recursion.zsh",
        "10_fizzbuzz.zsh",
        "11_fibonacci.zsh",
        "12_quicksort.zsh",
        "13_prime_sieve.zsh",
        "14_brace_expansion.zsh",
        "15_parameter_expansion.zsh",
        "16_parameter_flags.zsh",
        "17_heredocs.zsh",
        "18_cmd_substitution.zsh",
        "19_pipes_and_filters.zsh",
        "20_process_substitution.zsh",
        "21_printf_demo.zsh",
        "22_trap_exit.zsh",
        "23_ifs_split.zsh",
        "24_word_count.zsh",
        "25_reverse_string.zsh",
        "26_anonymous_fn.zsh",
        "27_positional_args.zsh",
        "28_typeset.zsh",
        "29_matrix_print.zsh",
        "30_pattern_match.zsh",
        "31_stack.zsh",
        "32_queue.zsh",
        "33_binary_search.zsh",
        "34_bubble_sort.zsh",
        "35_insertion_sort.zsh",
        "36_selection_sort.zsh",
        "37_counting_sort.zsh",
        "38_set_ops.zsh",
        "39_matrix_multiply.zsh",
        "40_roman_numerals.zsh",
        "41_base_convert.zsh",
        "42_tower_of_hanoi.zsh",
        "43_collatz.zsh",
        "44_happy_numbers.zsh",
        "45_armstrong.zsh",
        "46_perfect_numbers.zsh",
        "47_rot13.zsh",
        "48_atbash.zsh",
        "49_word_reverse.zsh",
        "50_histogram.zsh",
        "51_csv_parse.zsh",
        "52_env_basics.zsh",
        "53_file_tests.zsh",
        "54_date_format.zsh",
        "55_read_loop.zsh",
        "56_exit_codes.zsh",
        "57_atoi_itoa.zsh",
        "58_gcd_lcm.zsh",
        "59_string_reverse_ops.zsh",
        "60_mapfile_like.zsh",
        "61_zsh_modifiers.zsh",
        "62_param_flags_match.zsh",
        "63_param_flags_join_split.zsh",
        "64_param_flags_case.zsh",
        "65_param_flags_sort.zsh",
        "66_param_flags_format.zsh",
        "67_glob_qualifiers.zsh",
        "68_extended_glob.zsh",
        "69_assoc_advanced.zsh",
        "70_array_set_ops_zsh.zsh",
        "71_array_pattern_filter.zsh",
        "72_typeset_int_base.zsh",
        "73_print_columnar.zsh",
        "74_print_prompt_escapes.zsh",
        "75_zparseopts.zsh",
        "76_mathfunc.zsh",
        "77_datetime.zsh",
        "78_setopt_local_scope.zsh",
        "79_eval_dynamic_dispatch.zsh",
        "80_anon_fn_args.zsh",
        "81_compound_defaults.zsh",
        "82_brace_advanced.zsh",
        "83_history_modifiers.zsh",
        "84_subst_split_complex.zsh",
        "85_zcalc_repl.zsh",
        "86_setopt_exhaustive.zsh",
        "87_read_advanced.zsh",
        "88_printf_format_advanced.zsh",
        "89_regex_match.zsh",
        "90_type_whence.zsh",
        "91_hash_builtin.zsh",
        "92_arithmetic_for.zsh",
        "93_nested_assoc.zsh",
        "94_case_advanced.zsh",
        "95_fd_redirection.zsh",
        "96_strict_mode.zsh",
        "97_indirection.zsh",
        "98_coreutils_builtins.zsh",
        "99_negative_indexing.zsh",
        "100_zsh_features_summary.zsh",
        "101_subshell_grouping.zsh",
        "102_function_introspection.zsh",
        "103_exit_traps_advanced.zsh",
        "104_strict_arithmetic.zsh",
        "105_dispatch_table.zsh",
        "106_pipe_chains.zsh",
        "107_eval_metaprogramming.zsh",
        "108_globsubst_globalias.zsh",
        "109_arith_truth_tables.zsh",
        "110_misc_advanced.zsh",
        "111_let_builtin.zsh",
        "112_assignment_forms.zsh",
        "113_tied_arrays.zsh",
        "114_local_modifiers.zsh",
        "115_param_strip_advanced.zsh",
        "116_cond_numeric_ops.zsh",
        "117_backref_replacement.zsh",
        "118_recursive_glob.zsh",
        "119_background_wait.zsh",
        "120_utf8_strings.zsh",
        "121_mini_cat.zsh",
        "122_mini_grep.zsh",
        "123_mini_wc.zsh",
        "124_url_encode.zsh",
        "125_json_pretty.zsh",
        "126_xml_escape.zsh",
        "127_string_trim.zsh",
        "128_csv_writer.zsh",
        "129_assoc_serialize.zsh",
        "130_ini_parser.zsh",
        "131_emulate_modes.zsh",
        "132_ksh_patterns.zsh",
        "133_zstyle_demo.zsh",
        "134_compdef_signatures.zsh",
        "135_bindkey_config.zsh",
        "136_path_manipulation.zsh",
        "137_named_pipes.zsh",
        "138_lock_files.zsh",
        "139_env_manipulation.zsh",
        "140_signal_handling.zsh",
        "141_color_codes.zsh",
        "142_calc_engine.zsh",
        "143_todo_app.zsh",
        "144_graph_bfs.zsh",
        "145_state_machine.zsh",
        "146_topological_sort.zsh",
        "147_pomodoro_timer.zsh",
        "148_inventory_system.zsh",
        "149_event_log.zsh",
        "150_lru_cache.zsh",
        "151_priority_queue.zsh",
        "152_bloom_filter.zsh",
        "153_trie.zsh",
        "154_levenshtein.zsh",
        "155_text_diff.zsh",
        "156_simple_template.zsh",
        "157_observer_pattern.zsh",
        "158_simulate_random.zsh",
        "159_bank_account.zsh",
        "160_zshrs_capabilities.zsh",
        "161_dirstack.zsh",
        "162_umask_ulimit.zsh",
        "163_quoting_flags.zsh",
        "164_z_split_shell.zsh",
        "165_print_advanced.zsh",
        "166_glob_flags.zsh",
        "167_mini_find.zsh",
        "168_mini_make.zsh",
        "169_markdown_to_text.zsh",
        "170_regex_tester.zsh",
        "171_moving_average.zsh",
        "172_password_check.zsh",
        "173_anagram_finder.zsh",
        "174_number_to_words.zsh",
        "175_ascii_chart.zsh",
        "176_game_of_life.zsh",
        "177_days_between.zsh",
        "178_maze_generator.zsh",
        "179_lottery_sim.zsh",
        "180_calendar.zsh",
        "181_fizzbuzz_variants.zsh",
        "182_progress_bar.zsh",
        "183_word_frequency.zsh",
        "184_simple_cron.zsh",
        "185_final_recap.zsh",
        "186_alias_forms.zsh",
        "187_hex_dump.zsh",
        "188_ip_parser.zsh",
        "189_http_status.zsh",
        "190_ansi_stripper.zsh",
        "191_retry_backoff.zsh",
        "192_memoize.zsh",
        "193_log_rotate.zsh",
        "194_url_parser.zsh",
        "195_sha_simple_hash.zsh",
        "196_base64.zsh",
        "197_csv_full_parse.zsh",
        "198_yaml_lite.zsh",
        "199_color_picker.zsh",
        "200_milestone.zsh",
        "201_unit_converter.zsh",
        "202_tokenizer.zsh",
        "203_argv_dispatch.zsh",
        "204_string_interpolation.zsh",
        "205_zsh_in_scripts.zsh",
        "206_assoc_iteration.zsh",
        "207_lru_with_ttl.zsh",
        "208_directory_walker.zsh",
        "209_command_pipeline.zsh",
        "210_quine.zsh",
        "211_csv_to_md.zsh",
        "212_markdown_table.zsh",
        "213_ssh_config_parser.zsh",
        "214_chess_board.zsh",
        "215_tic_tac_toe.zsh",
        "216_deck_of_cards.zsh",
        "217_guess_number.zsh",
        "218_quiz_game.zsh",
        "219_madlibs.zsh",
        "220_expense_tracker.zsh",
        "221_zsh_xtrace.zsh",
        "222_zsh_psvars.zsh",
        "223_funcstack.zsh",
        "224_git_log_parser.zsh",
        "225_nginx_log_analyze.zsh",
        "226_todo_categories.zsh",
        "227_hashtable_oa.zsh",
        "228_stack_machine.zsh",
        "229_search_filter.zsh",
        "230_menu_system.zsh",
        "231_text_adventure.zsh",
        "232_time_tracker.zsh",
        "233_word_chain.zsh",
        "234_simple_kvs.zsh",
        "235_grand_finale.zsh",
        "236_zsh_hooks.zsh",
        "237_zsh_autoload.zsh",
        "238_dijkstra.zsh",
        "239_sudoku_validate.zsh",
        "240_lights_out.zsh",
        "241_hangman.zsh",
        "242_number_sequences.zsh",
        "243_sierpinski.zsh",
        "244_mandelbrot_ascii.zsh",
        "245_toml_parser.zsh",
        "246_env_file_parser.zsh",
        "247_shebang_detector.zsh",
        "248_charset_validator.zsh",
        "249_whitespace_normalizer.zsh",
        "250_shopping_cart.zsh",
        "251_vigenere_cipher.zsh",
        "252_caesar_cipher.zsh",
        "253_word_search.zsh",
        "254_ascii_clock.zsh",
        "255_ipv6_parser.zsh",
        "256_recipe_converter.zsh",
        "257_memory_match.zsh",
        "258_substitution_cipher.zsh",
        "259_boggle_solver.zsh",
        "260_final_v3.zsh",
        "261_prime_factorize.zsh",
        "262_miller_rabin.zsh",
        "263_extended_gcd.zsh",
        "264_a_star_pathfind.zsh",
        "265_kruskal_mst.zsh",
        "266_prim_mst.zsh",
        "267_floyd_warshall.zsh",
        "268_bellman_ford.zsh",
        "269_n_queens.zsh",
        "270_fifteen_puzzle.zsh",
        "271_hanoi_animated.zsh",
        "272_markdown_to_html.zsh",
        "273_http_parser.zsh",
        "274_log_format_detect.zsh",
        "275_csv_merge.zsh",
        "276_blackjack.zsh",
        "277_dice_game.zsh",
        "278_rps.zsh",
        "279_xor_cipher.zsh",
        "280_otp_pad.zsh",
        "281_zsh_periodic.zsh",
        "282_zsh_argv_special.zsh",
        "283_zsh_traps_full.zsh",
        "284_atomic_write.zsh",
        "285_banner_v4.zsh",
        "286_segment_tree.zsh",
        "287_fenwick_tree.zsh",
        "288_kmp_match.zsh",
        "289_rabin_karp.zsh",
        "290_manacher.zsh",
        "291_reservoir_sample.zsh",
        "292_skiplist.zsh",
        "293_suffix_array.zsh",
        "294_word_ladder.zsh",
        "295_soundex.zsh",
        "296_minesweeper.zsh",
        "297_mastermind.zsh",
        "298_ttt_minimax.zsh",
        "299_conway_animated.zsh",
        "300_milestone_300.zsh",
        "301_url_template.zsh",
        "302_sql_mini_parser.zsh",
        "303_zsh_print_z.zsh",
        "304_zsh_unhash_pattern.zsh",
        "305_zsh_typeset_m.zsh",
        "306_zsh_zle_widgets.zsh",
        "307_zsh_compsys_args.zsh",
        "308_kruskal_algo_density.zsh",
        "309_state_machine_dsl.zsh",
        "310_banner_v5.zsh",
        "311_bst.zsh",
        "312_avl_tree.zsh",
        "313_bloom_filter_v2.zsh",
        "314_deque.zsh",
        "315_ring_buffer.zsh",
        "316_ipv4_subnet.zsh",
        "317_mac_address.zsh",
        "318_file_checksum.zsh",
        "319_anagram_solver.zsh",
        "320_leet_speak.zsh",
        "321_pig_latin.zsh",
        "322_history_parser.zsh",
        "323_ssh_known_hosts.zsh",
        "324_brace_advanced.zsh",
        "325_zsh_print_more.zsh",
        "326_zsh_compinit.zsh",
        "327_zsh_extended_glob.zsh",
        "328_zsh_kv_assoc.zsh",
        "329_max_subarray.zsh",
        "330_lis_lcs.zsh",
        "331_knapsack.zsh",
        "332_coin_change.zsh",
        "333_topological_sort.zsh",
        "334_lru_cache.zsh",
        "335_banner_v6.zsh",
        "336_roman_numeral.zsh",
        "337_trie_advanced.zsh",
        "338_z_function.zsh",
        "339_longest_common_substring.zsh",
        "340_palindromic_subseq.zsh",
        "341_nim_game.zsh",
        "342_peg_solitaire.zsh",
        "343_rfc2822_date.zsh",
        "344_iso8601.zsh",
        "345_pollard_rho.zsh",
        "346_continued_fraction.zsh",
        "347_transposition_cipher.zsh",
        "348_color_conversions.zsh",
        "349_zsh_eval_context.zsh",
        "350_milestone_350.zsh",
        "351_sokoban_small.zsh",
        "352_text_wrap.zsh",
        "353_unicode_utils.zsh",
        "354_url_encode.zsh",
        "355_calendar_print.zsh",
        "356_disjoint_set.zsh",
        "357_priority_queue.zsh",
        "358_zsh_funcfile.zsh",
        "359_zsh_param_complete.zsh",
        "360_banner_v7.zsh",
        "361_json_parser_full.zsh",
        "362_xml_parser_full.zsh",
        "363_arith_expr_evaluator.zsh",
        "364_csv_rfc4180.zsh",
        "365_mini_lisp.zsh",
        "366_sudoku_solver_bt.zsh",
        "367_banner_v8.zsh",
        "368_bencode_roundtrip.zsh",
        "369_hamming_7_4.zsh",
        "370_skyline.zsh",
        "371_brainfuck_interp.zsh",
        "372_ackermann.zsh",
        "373_lzw_codec.zsh",
        "374_elias_gamma.zsh",
        "375_banner_v9.zsh",
    ];
    let dir = demos_dir();
    let mut on_disk: Vec<String> = match std::fs::read_dir(&dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("zsh"))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(_) => return, // dir missing in unusual checkouts; skip
    };
    on_disk.sort();
    let mut want: Vec<String> = registered.iter().map(|s| s.to_string()).collect();
    want.sort();
    assert_eq!(
        on_disk, want,
        "examples/demos/ contents ≠ registered tests — add/remove demo_test! entries to match",
    );
}
