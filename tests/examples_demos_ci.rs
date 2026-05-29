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
