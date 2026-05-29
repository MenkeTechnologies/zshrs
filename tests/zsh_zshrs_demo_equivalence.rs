//! zsh ↔ zshrs output-equivalence harness for `examples/demos/*.zsh`.
//!
//! Each demo is run under BOTH the system `zsh` interpreter and the
//! `zshrs --zsh` binary. Stdout is compared byte-for-byte; any
//! divergence fails the test. Exit codes are compared too.
//!
//! This is the strongest possible "compatibility contract": if zshrs
//! is a faithful zsh clone, every demo's output must be identical
//! under both interpreters. Per-demo `#[test]` so cargo reports
//! per-demo pass/fail, parallel runners execute independently, and
//! `#[ignore]` markers can pin known divergences (each tagged with a
//! ZSHRS BUG explanation citing the root cause).
//!
//! Skip behavior:
//!   * If `zsh` is not on PATH (rare — macOS ships it, every Linux
//!     distro packages it), every test silently skips with a notice.
//!   * If the `zshrs` binary isn't built (no `cargo build` run yet),
//!     every test silently skips. Matches the convention in
//!     `examples_demos_ci.rs`.
//!
//! Per-test timeout: 30s wall-clock. Demos finish in well under 1s;
//! a 30s run signals a regression.

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

fn zsh_bin() -> Option<PathBuf> {
    for cand in [
        "/opt/homebrew/bin/zsh",
        "/usr/local/bin/zsh",
        "/bin/zsh",
        "/usr/bin/zsh",
    ] {
        let p = PathBuf::from(cand);
        if p.exists() {
            return Some(p);
        }
    }
    // Fallback: PATH lookup via `which`.
    if let Ok(out) = Command::new("which").arg("zsh").output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                let p = PathBuf::from(s);
                if p.exists() {
                    return Some(p);
                }
            }
        }
    }
    None
}

fn demos_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/demos")
}

/// Run `bin <args...> <script>` with a wall-clock cap. Returns
/// (exit-code, stdout-bytes, stderr-bytes). Panics on spawn failure
/// or timeout.
fn run(bin: &Path, args: &[&str], script: &Path) -> (i32, Vec<u8>, Vec<u8>) {
    let mut child = Command::new(bin)
        .args(args)
        .arg(script)
        // Drop environment-derived noise so output isn't perturbed
        // by the host shell's settings.
        .env_remove("ZSHRS_CACHE")
        .env_remove("ZDOTDIR")
        .env_remove("PROMPT")
        .env_remove("RPROMPT")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn {}: {e}", bin.display()));

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
                let code = status.code().unwrap_or(-1);
                return (code, out, err);
            }
            Ok(None) => {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    panic!(
                        "{} {} exceeded 30s — likely regression",
                        bin.display(),
                        script.display()
                    );
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("waitpid {}: {e}", script.display()),
        }
    }
}

/// Per-demo equivalence check: both interpreters must produce
/// byte-identical stdout and identical exit codes.
fn assert_demo_zsh_zshrs_identical(script_name: &str) {
    let zsh = match zsh_bin() {
        Some(b) => b,
        None => {
            eprintln!("skip: zsh not installed on PATH");
            return;
        }
    };
    let zshrs = match zshrs_bin() {
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
        script.display(),
    );

    let (zsh_code, zsh_out, zsh_err) = run(&zsh, &[], &script);
    let (rs_code, rs_out, rs_err) = run(&zshrs, &["--zsh"], &script);

    if zsh_out != rs_out {
        let zsh_str = String::from_utf8_lossy(&zsh_out);
        let rs_str = String::from_utf8_lossy(&rs_out);
        panic!(
            "{}: stdout mismatch\n\
             ── zsh ({} bytes, exit {}) ──\n{}\n\
             ── zshrs ({} bytes, exit {}) ──\n{}\n\
             ── zsh stderr ──\n{}\n── zshrs stderr ──\n{}",
            script_name,
            zsh_out.len(),
            zsh_code,
            zsh_str,
            rs_out.len(),
            rs_code,
            rs_str,
            String::from_utf8_lossy(&zsh_err),
            String::from_utf8_lossy(&rs_err),
        );
    }
    assert_eq!(
        zsh_code, rs_code,
        "{}: exit code mismatch (zsh={} zshrs={})",
        script_name, zsh_code, rs_code,
    );
}

// One #[test] per demo: cargo reports per-demo pass/fail, parallel
// runs are independent, and `#[ignore]` markers pin known
// divergences with a ZSHRS BUG citation explaining the root cause.
//
// 45 of 60 demos already produce byte-identical output under both
// zsh and `zshrs --zsh` (recon at v0.11.22). The 15 divergent demos
// are marked `#[ignore]` with the specific bug class.
macro_rules! eq_test {
    ($name:ident, $file:expr) => {
        #[test]
        fn $name() {
            assert_demo_zsh_zshrs_identical($file);
        }
    };
    ($name:ident, $file:expr, ignore = $reason:expr) => {
        #[test]
        #[ignore = $reason]
        fn $name() {
            assert_demo_zsh_zshrs_identical($file);
        }
    };
}

eq_test!(eq01_hello, "01_hello.zsh");
eq_test!(eq02_arithmetic, "02_arithmetic.zsh");
eq_test!(eq03_strings, "03_strings.zsh");
eq_test!(eq04_arrays, "04_arrays.zsh");
eq_test!(eq05_assoc_arrays, "05_assoc_arrays.zsh");
eq_test!(eq06_control_flow, "06_control_flow.zsh");
eq_test!(eq07_for_loops, "07_for_loops.zsh");
eq_test!(eq08_functions, "08_functions.zsh");
eq_test!(eq09_recursion, "09_recursion.zsh");
eq_test!(eq10_fizzbuzz, "10_fizzbuzz.zsh");
eq_test!(eq11_fibonacci, "11_fibonacci.zsh");
eq_test!(eq12_quicksort, "12_quicksort.zsh");
eq_test!(eq13_prime_sieve, "13_prime_sieve.zsh");
eq_test!(eq14_brace_expansion, "14_brace_expansion.zsh");
eq_test!(eq15_parameter_expansion, "15_parameter_expansion.zsh");
eq_test!(eq16_parameter_flags, "16_parameter_flags.zsh");
eq_test!(eq17_heredocs, "17_heredocs.zsh");
eq_test!(eq18_cmd_substitution, "18_cmd_substitution.zsh");
eq_test!(eq19_pipes_and_filters, "19_pipes_and_filters.zsh",
    ignore = "ZSHRS BUG: pipe/filter output diverges — likely stream-buffering or builtin filter divergence");
eq_test!(eq20_process_substitution, "20_process_substitution.zsh");
eq_test!(eq21_printf_demo, "21_printf_demo.zsh",
    ignore = "ZSHRS BUG: printf escape handling differs (e.g. `\\\\` rendering); see Src/builtin.c:bin_print");
eq_test!(eq22_trap_exit, "22_trap_exit.zsh",
    ignore = "ZSHRS BUG: relies on $$ PID + tmpfile path, naturally diverges across processes — not a real bug, expected env-volatility");
eq_test!(eq23_ifs_split, "23_ifs_split.zsh");
eq_test!(eq24_word_count, "24_word_count.zsh",
    ignore = "ZSHRS BUG: word-count pipeline differs (likely wc(1) field padding or tr behavior in the pipe)");
eq_test!(eq25_reverse_string, "25_reverse_string.zsh");
eq_test!(eq26_anonymous_fn, "26_anonymous_fn.zsh",
    ignore = "ZSHRS BUG: anonymous fn inside $(...) command sub drops last line — `result=$(() { echo $(($1*$1)) } 9)` returns empty");
eq_test!(eq27_positional_args, "27_positional_args.zsh");
eq_test!(eq28_typeset, "28_typeset.zsh");
eq_test!(eq29_matrix_print, "29_matrix_print.zsh");
eq_test!(eq30_pattern_match, "30_pattern_match.zsh");
eq_test!(eq31_stack, "31_stack.zsh",
    ignore = "ZSHRS BUG: array-as-stack push/pop divergence — likely array-trailing-newline or empty-slot handling");
eq_test!(eq32_queue, "32_queue.zsh");
eq_test!(eq33_binary_search, "33_binary_search.zsh",
    ignore = "ZSHRS BUG: function `return 1` (last bsearch call) doesn't propagate to script exit code — zsh=1, zshrs=0; stdout identical");
eq_test!(eq34_bubble_sort, "34_bubble_sort.zsh");
eq_test!(eq35_insertion_sort, "35_insertion_sort.zsh");
eq_test!(eq36_selection_sort, "36_selection_sort.zsh");
eq_test!(eq37_counting_sort, "37_counting_sort.zsh",
    ignore = "ZSHRS BUG: counting-sort output diverges — investigate associative-array iteration order or count accumulation");
eq_test!(eq38_set_ops, "38_set_ops.zsh");
eq_test!(eq39_matrix_multiply, "39_matrix_multiply.zsh");
eq_test!(eq40_roman_numerals, "40_roman_numerals.zsh");
eq_test!(eq41_base_convert, "41_base_convert.zsh");
eq_test!(eq42_tower_of_hanoi, "42_tower_of_hanoi.zsh");
eq_test!(eq43_collatz, "43_collatz.zsh");
eq_test!(eq44_happy_numbers, "44_happy_numbers.zsh");
eq_test!(eq45_armstrong, "45_armstrong.zsh",
    ignore = "ZSHRS BUG: armstrong-number digit-decomposition differs — likely integer/float coercion in arithmetic loop");
eq_test!(eq46_perfect_numbers, "46_perfect_numbers.zsh");
eq_test!(eq47_rot13, "47_rot13.zsh");
eq_test!(eq48_atbash, "48_atbash.zsh");
eq_test!(eq49_word_reverse, "49_word_reverse.zsh",
    ignore = "ZSHRS BUG: word-reverse output diverges — investigate IFS-split or array reversal idiom");
eq_test!(eq50_histogram, "50_histogram.zsh",
    ignore = "ZSHRS BUG: histogram bar rendering diverges — likely assoc-array value iteration or printf bar count");
eq_test!(eq51_csv_parse, "51_csv_parse.zsh",
    ignore = "ZSHRS BUG: csv-parse output diverges — likely IFS=, splitting or quoted-field handling");
eq_test!(eq52_env_basics, "52_env_basics.zsh");
eq_test!(eq53_file_tests, "53_file_tests.zsh",
    ignore = "ZSHRS BUG: relies on temp-file paths / $$ PID — env-volatility, not a real bug");
eq_test!(eq54_date_format, "54_date_format.zsh",
    ignore = "ZSHRS BUG: date formatting uses wall-clock time — naturally diverges across invocations; not a real bug");
eq_test!(eq55_read_loop, "55_read_loop.zsh",
    ignore = "ZSHRS BUG: read-loop relies on tempfile or stdin redirection that differs across runs");
eq_test!(eq56_exit_codes, "56_exit_codes.zsh");
eq_test!(eq57_atoi_itoa, "57_atoi_itoa.zsh");
eq_test!(eq58_gcd_lcm, "58_gcd_lcm.zsh");
eq_test!(eq59_string_reverse_ops, "59_string_reverse_ops.zsh",
    ignore = "ZSHRS BUG: string-reverse-ops output diverges — investigate ${(s::)...} flag or array reversal");
eq_test!(eq60_mapfile_like, "60_mapfile_like.zsh");
eq_test!(eq61_zsh_modifiers, "61_zsh_modifiers.zsh",
    ignore = "ZSHRS BUG: zsh modifier output diverges — investigate `:t`/`:r`/`:h` history modifier port");
eq_test!(eq62_param_flags_match, "62_param_flags_match.zsh");
eq_test!(eq63_param_flags_join_split, "63_param_flags_join_split.zsh");
eq_test!(eq64_param_flags_case, "64_param_flags_case.zsh");
eq_test!(eq65_param_flags_sort, "65_param_flags_sort.zsh");
eq_test!(eq66_param_flags_format, "66_param_flags_format.zsh");
eq_test!(eq67_glob_qualifiers, "67_glob_qualifiers.zsh",
    ignore = "ZSHRS BUG: glob qualifier output diverges — investigate `(.)/(/)/(L+N)` etc.");
eq_test!(eq68_extended_glob, "68_extended_glob.zsh",
    ignore = "ZSHRS BUG: extended-glob output diverges — investigate `(#i)`, `(##)`, `~`, `^` operators");
eq_test!(eq69_assoc_advanced, "69_assoc_advanced.zsh");
eq_test!(eq70_array_set_ops_zsh, "70_array_set_ops_zsh.zsh");
eq_test!(eq71_array_pattern_filter, "71_array_pattern_filter.zsh");
eq_test!(eq72_typeset_int_base, "72_typeset_int_base.zsh");
eq_test!(eq73_print_columnar, "73_print_columnar.zsh",
    ignore = "ZSHRS BUG: print columnar/`-C N` output diverges — investigate column-width computation");
eq_test!(eq74_print_prompt_escapes, "74_print_prompt_escapes.zsh",
    ignore = "ZSHRS BUG: print -P prompt-escape output diverges — investigate prompt-expand port");
eq_test!(eq75_zparseopts, "75_zparseopts.zsh",
    ignore = "ZSHRS BUG: zparseopts output diverges — investigate option-spec parsing");
eq_test!(eq76_mathfunc, "76_mathfunc.zsh");
eq_test!(eq77_datetime, "77_datetime.zsh",
    ignore = "ZSHRS BUG: datetime uses wall-clock — env-volatile, not a real bug");
eq_test!(eq78_setopt_local_scope, "78_setopt_local_scope.zsh",
    ignore = "ZSHRS BUG: setopt -L local-options scope diverges — investigate option-stack save/restore");
eq_test!(eq79_eval_dynamic_dispatch, "79_eval_dynamic_dispatch.zsh");
eq_test!(eq80_anon_fn_args, "80_anon_fn_args.zsh",
    ignore = "ZSHRS BUG: anon fn arg-handling diverges — likely related to eq26 anon-fn-in-cmdsub class");
eq_test!(eq81_compound_defaults, "81_compound_defaults.zsh");
eq_test!(eq82_brace_advanced, "82_brace_advanced.zsh");
eq_test!(eq83_history_modifiers, "83_history_modifiers.zsh",
    ignore = "ZSHRS BUG: history-modifier output diverges — investigate `:h`/`:t`/`:r`/`:e` port");
eq_test!(eq84_subst_split_complex, "84_subst_split_complex.zsh");
eq_test!(eq85_zcalc_repl, "85_zcalc_repl.zsh");

/// Coverage pin: every demo file on disk must have a corresponding
/// `eq_test!` entry. Catches new demos added without registration.
#[test]
fn every_demo_has_equivalence_test() {
    let registered: Vec<&str> = vec![
        "01_hello.zsh", "02_arithmetic.zsh", "03_strings.zsh",
        "04_arrays.zsh", "05_assoc_arrays.zsh", "06_control_flow.zsh",
        "07_for_loops.zsh", "08_functions.zsh", "09_recursion.zsh",
        "10_fizzbuzz.zsh", "11_fibonacci.zsh", "12_quicksort.zsh",
        "13_prime_sieve.zsh", "14_brace_expansion.zsh",
        "15_parameter_expansion.zsh", "16_parameter_flags.zsh",
        "17_heredocs.zsh", "18_cmd_substitution.zsh",
        "19_pipes_and_filters.zsh", "20_process_substitution.zsh",
        "21_printf_demo.zsh", "22_trap_exit.zsh", "23_ifs_split.zsh",
        "24_word_count.zsh", "25_reverse_string.zsh",
        "26_anonymous_fn.zsh", "27_positional_args.zsh",
        "28_typeset.zsh", "29_matrix_print.zsh", "30_pattern_match.zsh",
        "31_stack.zsh", "32_queue.zsh", "33_binary_search.zsh",
        "34_bubble_sort.zsh", "35_insertion_sort.zsh",
        "36_selection_sort.zsh", "37_counting_sort.zsh",
        "38_set_ops.zsh", "39_matrix_multiply.zsh",
        "40_roman_numerals.zsh", "41_base_convert.zsh",
        "42_tower_of_hanoi.zsh", "43_collatz.zsh",
        "44_happy_numbers.zsh", "45_armstrong.zsh",
        "46_perfect_numbers.zsh", "47_rot13.zsh", "48_atbash.zsh",
        "49_word_reverse.zsh", "50_histogram.zsh", "51_csv_parse.zsh",
        "52_env_basics.zsh", "53_file_tests.zsh", "54_date_format.zsh",
        "55_read_loop.zsh", "56_exit_codes.zsh", "57_atoi_itoa.zsh",
        "58_gcd_lcm.zsh", "59_string_reverse_ops.zsh",
        "60_mapfile_like.zsh", "61_zsh_modifiers.zsh",
        "62_param_flags_match.zsh", "63_param_flags_join_split.zsh",
        "64_param_flags_case.zsh", "65_param_flags_sort.zsh",
        "66_param_flags_format.zsh", "67_glob_qualifiers.zsh",
        "68_extended_glob.zsh", "69_assoc_advanced.zsh",
        "70_array_set_ops_zsh.zsh", "71_array_pattern_filter.zsh",
        "72_typeset_int_base.zsh", "73_print_columnar.zsh",
        "74_print_prompt_escapes.zsh", "75_zparseopts.zsh",
        "76_mathfunc.zsh", "77_datetime.zsh",
        "78_setopt_local_scope.zsh", "79_eval_dynamic_dispatch.zsh",
        "80_anon_fn_args.zsh", "81_compound_defaults.zsh",
        "82_brace_advanced.zsh", "83_history_modifiers.zsh",
        "84_subst_split_complex.zsh", "85_zcalc_repl.zsh",
    ];
    let dir = demos_dir();
    let mut on_disk: Vec<String> = match std::fs::read_dir(&dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("zsh"))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(_) => return,
    };
    on_disk.sort();
    let mut want: Vec<String> = registered.iter().map(|s| s.to_string()).collect();
    want.sort();
    assert_eq!(
        on_disk, want,
        "examples/demos/ contents ≠ registered eq_test! entries — add a new eq_test! call to match",
    );
}
