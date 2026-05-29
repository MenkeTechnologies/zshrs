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
    for cand in [
        manifest.join("target/debug/zshrs"),
        manifest.join("target/release/zshrs"),
    ] {
        if cand.exists() {
            return Some(cand);
        }
    }
    None
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
demo_test!(d102_function_introspection, "102_function_introspection.zsh");
demo_test!(d103_exit_traps_advanced, "103_exit_traps_advanced.zsh");
demo_test!(d104_strict_arithmetic, "104_strict_arithmetic.zsh");
demo_test!(d105_dispatch_table, "105_dispatch_table.zsh");
demo_test!(d106_pipe_chains, "106_pipe_chains.zsh");
demo_test!(d107_eval_metaprogramming, "107_eval_metaprogramming.zsh");
demo_test!(d108_globsubst_globalias, "108_globsubst_globalias.zsh");
demo_test!(d109_arith_truth_tables, "109_arith_truth_tables.zsh");
demo_test!(d110_misc_advanced, "110_misc_advanced.zsh");

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
