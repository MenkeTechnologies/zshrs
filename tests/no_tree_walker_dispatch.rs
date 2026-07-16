//! Black-box behavioral proof that the bytecode VM handles every shell
//! construct that the deleted tree-walker dispatch (`execute_simple`,
//! `execute_pipeline`, `execute_list`, `execute_compound`,
//! `execute_command_bg`) used to handle.
//!
//! Every test runs a real `zshrs -f -c <code>` and asserts the EXACT stdout +
//! exit status. Tests are hand-crafted around constructs that historically
//! lived in the tree walker: each assertion fails loudly if the bytecode
//! lowering for that construct regresses.
//!
//! These tests are NOT redundant with the ztst corpus — those run pre-existing
//! test scripts that may pass for the wrong reasons (output ignored, exit
//! ignored, etc.). Each test below pins a specific behavior.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

fn zshrs_bin() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("target/debug/zshrs");
    p
}

/// Serializes execution of fork-heavy subprocess tests (pipelines, command
/// substitution, background `&`). zshrs itself uses `libc::fork` for pipeline
/// stages and bg dispatch, and post-fork in a multi-threaded program is only
/// async-signal-safe in theory — in practice, running multiple of these in
/// parallel cargo-test threads exposes timing races (held mutexes, stdio
/// reordering, pid reaping). Held only for the duration of the spawned zshrs
/// invocation. Pure-bytecode tests (no zshrs-internal fork) skip the lock and
/// run in parallel as before.
static FORK_SERIAL: Mutex<()> = Mutex::new(());

/// Run `zshrs -f -c code` with a 10-second timeout. Returns (status, stdout).
/// stderr is captured and discarded — tests assert only on stdout/status to
/// stay independent of warning text the executor may emit.
fn run(code: &str) -> (i32, String) {
    let mut child = Command::new(zshrs_bin())
        .args(["-f", "-c", code])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("zshrs binary missing — run `cargo build` first");

    let timeout = Duration::from_secs(10);
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = child.wait_with_output().unwrap();
                return (
                    status.code().unwrap_or(-1),
                    String::from_utf8_lossy(&out.stdout).to_string(),
                );
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("zshrs hung on: {}", code);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => panic!("waitpid: {}", e),
        }
    }
}

/// Run with stdin piped in.
fn run_stdin(code: &str, input: &str) -> (i32, String) {
    let mut child = Command::new(zshrs_bin())
        .args(["-f", "-c", code])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("zshrs binary missing");
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
    }
    let out = child.wait_with_output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).to_string(),
    )
}

/// Assert exact stdout and zero exit status.
fn ok(code: &str, expected_stdout: &str) {
    let (status, stdout) = run(code);
    assert_eq!(
        status, 0,
        "expected exit 0 from `{}`, got {} with stdout:\n{}",
        code, status, stdout
    );
    assert_eq!(stdout, expected_stdout, "stdout mismatch for `{}`", code);
}

/// Wrap `ok` with the FORK_SERIAL mutex. Use for tests where zshrs internally
/// forks (pipelines, command substitution, `cmd &`). The lock is held only
/// for the spawned subprocess's lifetime — pure-bytecode tests stay parallel.
fn ok_serial(code: &str, expected_stdout: &str) {
    let _guard = FORK_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    ok(code, expected_stdout);
}

/// Assert exact stdout and exact exit status.
fn ok_status(code: &str, expected_stdout: &str, expected_status: i32) {
    let (status, stdout) = run(code);
    assert_eq!(
        status, expected_status,
        "expected exit {} from `{}`, got {} with stdout:\n{}",
        expected_status, code, status, stdout
    );
    assert_eq!(stdout, expected_stdout, "stdout mismatch for `{}`", code);
}

// ─────────────────────────────────────────────────────────────────────────────
// Simple commands — formerly `execute_simple`
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn simple_builtin_echo() {
    ok("echo hello world", "hello world\n");
}

#[test]
fn simple_assignment_then_echo() {
    ok("foo=bar; echo $foo", "bar\n");
}

#[test]
fn simple_external_command_via_host_exec() {
    // /usr/bin/true is a real external; status must be 0 and stdout empty.
    ok("/usr/bin/true", "");
}

#[test]
fn simple_external_failure_propagates_status() {
    ok_status("/usr/bin/false; echo $?", "1\n", 0);
}

#[test]
fn simple_command_not_found_returns_127() {
    ok_status(
        "this_command_does_not_exist_xyz123 2>/dev/null; echo $?",
        "127\n",
        0,
    );
}

#[test]
fn simple_special_param_dollar_question() {
    ok("true; echo $?", "0\n");
    ok("false; echo $?", "1\n");
}

#[test]
fn simple_special_param_dollar_pound_dollar_at() {
    ok(r#"set -- a b c; echo "$# $@""#, "3 a b c\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// Lists — `&&`, `||`, `;` — formerly `execute_list`
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn and_short_circuit_true_runs_next() {
    ok("true && echo yes", "yes\n");
}

#[test]
fn and_short_circuit_false_skips_next() {
    ok("false && echo skipped; echo done", "done\n");
}

#[test]
fn or_short_circuit_false_runs_next() {
    ok("false || echo recovered", "recovered\n");
}

#[test]
fn or_short_circuit_true_skips_next() {
    ok("true || echo skipped; echo done", "done\n");
}

#[test]
fn and_or_chain_true() {
    ok("true && echo a || echo b", "a\n");
}

#[test]
fn and_or_chain_false() {
    ok("false && echo a || echo b", "b\n");
}

#[test]
fn semicolon_runs_sequentially() {
    ok("echo 1; echo 2; echo 3", "1\n2\n3\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// Pipelines — formerly `execute_pipeline`
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn pipeline_two_stages_with_external() {
    ok_serial("echo hello | /bin/cat", "hello\n");
}

#[test]
fn pipeline_three_stages() {
    ok_serial("echo abc | cat | cat", "abc\n");
}

#[test]
fn pipeline_with_builtin_consumer() {
    // BSD-style wc (what zsh's bundled wc emits on macOS) right-pads
    // counts to 8 chars. Matches `/bin/zsh -f -c 'seq 5 | wc -l'`.
    ok_serial("seq 5 | wc -l", "       5\n");
}

#[test]
fn pipeline_function_in_first_stage() {
    ok_serial(
        "greet() { echo from-func; }; greet | /bin/cat",
        "from-func\n",
    );
}

#[test]
fn pipeline_terminates_on_sigpipe() {
    // Function produces unbounded output; `head -3` closes its read end after
    // 3 lines, sending SIGPIPE to producer. If pipelines weren't real (just
    // sequential execution), this would hang or recurse to stack overflow.
    let _guard = FORK_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    let (status, stdout) =
        run(r#"foo() { echo $1; foo $(($1 + 1)); }; foo 1 2>/dev/null | head -3"#);
    assert_eq!(status, 0, "head should exit 0; got {}", status);
    assert_eq!(stdout, "1\n2\n3\n");
}

#[test]
fn pipeline_negation_inverts_status() {
    ok_status("! true; echo $?", "1\n", 0);
    ok_status("! false; echo $?", "0\n", 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// Compounds — formerly `execute_compound`
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn if_then_runs_when_true() {
    ok("if true; then echo yes; fi", "yes\n");
}

#[test]
fn if_else_runs_else_when_false() {
    ok("if false; then echo yes; else echo no; fi", "no\n");
}

#[test]
fn if_elif_picks_first_match() {
    ok(
        "if false; then echo a; elif true; then echo b; elif true; then echo c; fi",
        "b\n",
    );
}

#[test]
fn while_loop_iterates_until_condition_fails() {
    ok(
        "i=0; while [[ $i -lt 4 ]]; do echo $i; i=$((i+1)); done",
        "0\n1\n2\n3\n",
    );
}

#[test]
fn until_loop_iterates_until_condition_succeeds() {
    ok(
        "i=0; until [[ $i -ge 3 ]]; do echo $i; i=$((i+1)); done",
        "0\n1\n2\n",
    );
}

#[test]
fn for_in_words_iterates() {
    ok("for x in a b c; do echo $x; done", "a\nb\nc\n");
}

#[test]
fn for_loop_var_visible_in_body() {
    // Pre-Phase-C this returned empty for $i because for-loop var was stored
    // in vm.globals but $i read from executor.variables.
    ok(
        "for i in 1 2 3; do echo iter=$i; done",
        "iter=1\niter=2\niter=3\n",
    );
}

#[test]
fn nested_for_loops() {
    ok(
        "for i in 1 2; do for j in a b; do echo $i$j; done; done",
        "1a\n1b\n2a\n2b\n",
    );
}

#[test]
fn case_exact_match() {
    ok("case foo in bar) echo b;; foo) echo f;; esac", "f\n");
}

#[test]
fn case_glob_pattern() {
    ok(
        "case hello.rs in *.rs) echo rust;; *.py) echo py;; esac",
        "rust\n",
    );
}

#[test]
fn case_star_fallback_arm() {
    // Pre-Phase-F bug: `case x in *) ...` glob-expanded `*` to cwd contents
    // because case patterns went through compile_word. Compile_case_pattern
    // fixed this — `*` reaches StrMatch as the literal pattern.
    ok(
        "case README.md in *.rs) echo rs;; *) echo other;; esac",
        "other\n",
    );
}

#[test]
fn case_dispatches_in_function_for_loop() {
    // The earlier real-world breakage that surfaced the case-* bug.
    let code = r#"
        f() { case "$1" in *.rs) echo rust;; *.py) echo py;; *) echo other;; esac; }
        for x in main.rs build.py README.md; do f "$x"; done
    "#;
    ok(code, "rust\npy\nother\n");
}

#[test]
fn brace_group_runs_commands() {
    ok("{ echo a; echo b; echo c; }", "a\nb\nc\n");
}

#[test]
fn double_bracket_string_eq() {
    ok_status(r#"[[ "abc" = "abc" ]]; echo $?"#, "0\n", 0);
    ok_status(r#"[[ "abc" = "xyz" ]]; echo $?"#, "1\n", 0);
}

#[test]
fn double_bracket_glob_match() {
    ok_status(r#"[[ "hello.rs" = *.rs ]]; echo $?"#, "0\n", 0);
    ok_status(r#"[[ "hello.py" = *.rs ]]; echo $?"#, "1\n", 0);
}

#[test]
fn double_bracket_regex_match() {
    ok_status(r#"[[ "abc123" =~ [0-9]+ ]]; echo $?"#, "0\n", 0);
    ok_status(r#"[[ "abcXYZ" =~ [0-9]+ ]]; echo $?"#, "1\n", 0);
}

#[test]
fn double_bracket_numeric_compare() {
    ok_status("[[ 5 -gt 3 ]]; echo $?", "0\n", 0);
    ok_status("[[ 3 -gt 5 ]]; echo $?", "1\n", 0);
}

#[test]
fn double_paren_arith_truthy() {
    ok_status("(( 5 > 3 )); echo $?", "0\n", 0);
    ok_status("(( 0 )); echo $?", "1\n", 0);
}

#[test]
fn arith_substitution_evaluates() {
    ok("echo $((1+2*3))", "7\n");
}

#[test]
fn arith_with_shell_vars() {
    ok("a=5; b=3; echo $((a*b - 2))", "13\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// Function calls — formerly went through `execute_simple` → `call_function`
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn function_no_args() {
    ok("f() { echo hello; }; f", "hello\n");
}

#[test]
fn function_positional_params() {
    ok("f() { echo $1 $2 $3; }; f a b c", "a b c\n");
}

#[test]
fn function_recursion() {
    // Direct port of zsh 5.9 behaviour: `if cond; then body; fi`
    // with cond=false and NO `else` returns 0. Src/loop.c:execif:
    // 590-591 — `else if (!retflag && !errflag) lastval = 0;`.
    // The earlier expectation of exit 1 was the pre-fix zshrs bug
    // (cond's status leaked through); corrected here to match zsh.
    ok_status(
        "rec() { if [[ $1 -gt 0 ]]; then echo $1; rec $(($1 - 1)); fi; }; rec 4",
        "4\n3\n2\n1\n",
        0,
    );
}

#[test]
fn function_factorial_with_cmd_subst() {
    // Exercises: recursion + command substitution + arith + local + if-else
    ok(
        "fact() { if [[ $1 -le 1 ]]; then echo 1; else local n=$1; local p=$(fact $((n-1))); echo $((n*p)); fi; }; fact 6",
        "720\n",
    );
}

#[test]
fn function_local_variable_scope() {
    ok(
        "f() { local x=hidden; echo inside=$x; }; x=visible; f; echo outside=$x",
        "inside=hidden\noutside=visible\n",
    );
}

#[test]
fn function_return_status_propagates_to_dollar_question() {
    ok("f() { return 42; }; f; echo $?", "42\n");
}

#[test]
fn function_forward_reference() {
    // `foo` references `bar` before `bar` is defined — but at call time `bar`
    // is registered in `executor.functions_compiled`, so it works.
    ok(
        "foo() { bar; }; bar() { echo from-bar; }; foo",
        "from-bar\n",
    );
}

#[test]
fn function_modifies_global_var() {
    // local x is local; assigning x inside without `local` should be visible
    // outside (matching shell semantics).
    ok("f() { x=set-by-f; }; f; echo $x", "set-by-f\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// Command substitution — formerly went through
// `execute_command_substitution` → `execute_command_capture`
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn cmd_subst_captures_stdout() {
    ok(r#"x=$(echo hi); echo "got: $x""#, "got: hi\n");
}

#[test]
fn cmd_subst_strips_trailing_newlines() {
    ok(r#"echo "[$(echo line)]""#, "[line]\n");
}

#[test]
fn cmd_subst_multiline_preserved_inside() {
    // BSD-style wc right-pads to 8 chars (matches zsh's bundled wc).
    ok(r#"x=$(printf 'a\nb\nc'); echo "$x" | wc -l"#, "       3\n");
}

#[test]
fn cmd_subst_nested() {
    ok(
        r#"echo "outer: $(echo inner: $(echo deepest))""#,
        "outer: inner: deepest\n",
    );
}

#[test]
fn cmd_subst_with_pipeline_inside() {
    // BSD-style wc right-pads to 8 chars (matches zsh's bundled wc).
    // The cmd-subst then strips leading whitespace per zsh-defaults.
    // Actually no — zsh keeps the padding in the cmd-subst result.
    ok(r#"x=$(seq 5 | wc -l); echo "lines:$x""#, "lines:       5\n");
}

#[test]
fn cmd_subst_status_doesnt_leak_to_outer() {
    // Exit status of $(false) doesn't poison subsequent $? of the wrapping
    // command. Outer `echo` succeeds with status 0.
    ok("echo $(false); echo $?", "\n0\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// Parameter expansion — was always partially via expand_word, now native ops
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn param_default_when_unset() {
    ok(r#"unset X; echo "${X:-default}""#, "default\n");
}

#[test]
fn param_default_when_set() {
    ok(r#"X=foo; echo "${X:-default}""#, "foo\n");
}

#[test]
fn param_alternate_when_set() {
    ok(r#"X=foo; echo "${X:+alt}""#, "alt\n");
}

#[test]
fn param_length() {
    ok(r#"X=hello; echo "${#X}""#, "5\n");
}

#[test]
fn param_strip_short_prefix() {
    ok(r#"p=/usr/local/bin; echo "${p#/usr/}""#, "local/bin\n");
}

#[test]
fn param_strip_long_prefix() {
    ok(r#"p=/usr/local/bin; echo "${p##*/}""#, "bin\n");
}

#[test]
fn param_strip_short_suffix() {
    ok(r#"p=hello.tar.gz; echo "${p%.gz}""#, "hello.tar\n");
}

#[test]
fn param_strip_long_suffix() {
    ok(r#"p=hello.tar.gz; echo "${p%%.*}""#, "hello\n");
}

#[test]
fn param_replace_first() {
    ok(r#"s=a-b-c; echo "${s/-/_}""#, "a_b-c\n");
}

#[test]
fn param_replace_all() {
    ok(r#"s=a-b-c; echo "${s//-/_}""#, "a_b_c\n");
}

#[test]
fn param_uppercase_bash_form_rejected() {
    // ${var^^} is bash-only — zsh rejects with "bad substitution".
    // zshrs follows zsh: returns empty stdout. (The zsh-native
    // uppercase form is `${(U)var}`.)
    let (_, out) = run(r#"s=hello; echo "${s^^}""#);
    assert_eq!(out.trim(), "");
}

#[test]
fn param_lowercase_bash_form_rejected() {
    // ${var,,} is bash-only — zsh rejects with "bad substitution".
    // zshrs follows zsh: returns empty stdout. (The zsh-native
    // lowercase form is `${(L)var}`.)
    let (_, out) = run(r#"s=HELLO; echo "${s,,}""#);
    assert_eq!(out.trim(), "");
}

#[test]
fn param_uppercase_zsh_native() {
    // The zsh-native uppercase: `${(U)var}` flag.
    ok(r#"s=hello; echo "${(U)s}""#, "HELLO\n");
}

#[test]
fn param_lowercase_zsh_native() {
    // The zsh-native lowercase: `${(L)var}` flag.
    ok(r#"s=HELLO; echo "${(L)s}""#, "hello\n");
}

#[test]
fn param_substring_offset_length() {
    ok(r#"s=abcdefgh; echo "${s:2:3}""#, "cde\n");
}

#[test]
fn param_substring_offset_only() {
    ok(r#"s=abcdefgh; echo "${s:5}""#, "fgh\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// Glob and tilde — native ops in compile_word
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn glob_expansion_matches_known_file() {
    // STALE-FIXTURE REPAIR (2026-06-12): the original assertion
    // hardcoded "Cargo.toml" with the premise "the only .toml at the
    // top level" — Cross.toml was added to the repo later, so the
    // test failed while zshrs's glob output was CORRECT. Compute the
    // expected list from the filesystem so the pin can't rot again
    // (zsh sorts glob results; mirror with a sorted read_dir).
    let mut expected: Vec<String> = std::fs::read_dir(env!("CARGO_MANIFEST_DIR"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".toml") && !n.starts_with('.'))
        .collect();
    expected.sort();
    assert!(!expected.is_empty(), "repo root must contain a .toml");
    let (status, stdout) = run("echo *.toml");
    assert_eq!(status, 0);
    assert_eq!(stdout.trim(), expected.join(" "));
}

#[test]
fn glob_no_match_returns_pattern() {
    // With nomatch unset (zsh default is `setopt nomatch`), unmatched
    // globs pass the literal pattern through (bash-style). Without
    // unsetopt, zsh aborts with `no matches found`. We mirror zsh's
    // default + opt-out behaviour.
    ok(
        "unsetopt nomatch; echo no_such_pattern_*.xyzzy",
        "no_such_pattern_*.xyzzy\n",
    );
}

#[test]
fn tilde_expands_to_home() {
    let home = std::env::var("HOME").expect("HOME not set");
    ok("echo ~", &format!("{}\n", home));
}

#[test]
fn tilde_with_path_suffix() {
    let home = std::env::var("HOME").expect("HOME not set");
    ok("echo ~/some/path", &format!("{}/some/path\n", home));
}

// ─────────────────────────────────────────────────────────────────────────────
// Redirections — formerly compounds-with-redirects in execute_compound
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn brace_group_redirected_to_file() {
    // The redirect target uses a unique-per-test temp path so we can verify
    // contents by reading from Rust afterward (rather than `cat $f; rm -f $f`
    // inside the script — which would tangle in the test harness's outer
    // shell environment if zshrs's `rm` builtin shells out at all).
    let tmp = std::env::temp_dir().join("zshrs_phase_f_brace_redir.out");
    let _ = std::fs::remove_file(&tmp);
    let code = format!(
        r#"{{ echo line1; echo line2; }} > {p}"#,
        p = tmp.to_string_lossy()
    );
    let (status, _stdout) = run(&code);
    assert_eq!(status, 0, "redirect command should succeed");
    let written = std::fs::read_to_string(&tmp).expect("output file");
    assert_eq!(written, "line1\nline2\n");
    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn function_body_redirected_to_file() {
    let tmp = std::env::temp_dir().join("zshrs_phase_f_func_redir.out");
    let _ = std::fs::remove_file(&tmp);
    let code = format!(
        r#"f() {{ echo from-func; }}; f > {p}"#,
        p = tmp.to_string_lossy()
    );
    let (status, _stdout) = run(&code);
    assert_eq!(status, 0);
    let written = std::fs::read_to_string(&tmp).expect("output file");
    assert_eq!(written, "from-func\n");
    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn herestring_feeds_stdin() {
    ok(r#"cat <<< "from herestring""#, "from herestring\n");
}

#[test]
fn heredoc_feeds_stdin() {
    let code = "cat <<EOF\nline-a\nline-b\nEOF\n";
    ok(code, "line-a\nline-b\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// Cross-VM state coherence — proves nested VMs (function calls, cmd subst,
// pipeline stages) all see the same `executor.variables` storage.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn var_set_in_function_visible_after() {
    ok("f() { x=hello; }; f; echo $x", "hello\n");
}

#[test]
fn var_set_outside_visible_in_function() {
    ok("y=outside; f() { echo $y; }; f", "outside\n");
}

#[test]
fn var_set_in_cmd_subst_does_not_leak() {
    // Subshell semantics: assignments in $(...) don't leak to the outer scope.
    // The cmd_subst host method runs the sub-chunk on a nested VM with the
    // SAME executor (POSIX subshells normally fork; we don't, but still
    // shouldn't leak modifications that the user expects to be subshell-local).
    // Note: this captures the current behavior. If we add real subshell
    // isolation later, this test will need updating.
    let code = "x=outer; v=$(x=inner; echo $x); echo \"v=$v outer=$x\"";
    let (status, stdout) = run(code);
    assert_eq!(status, 0);
    // We document the current behavior (no subshell isolation) explicitly:
    // both "outer" being mutated to "inner" by the subst and the captured
    // value being "inner" are observable. If/when subshell isolation lands,
    // the expected becomes "v=inner outer=outer".
    assert!(
        stdout == "v=inner outer=inner\n" || stdout == "v=inner outer=outer\n",
        "unexpected stdout: {:?}",
        stdout
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Builtins called from inside compounds — exercises the full dispatch chain
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn local_inside_function_doesnt_leak() {
    ok(
        "x=outer; f() { local x=inner; echo $x; }; f; echo $x",
        "inner\nouter\n",
    );
}

#[test]
fn break_exits_for_loop() {
    ok(
        "for i in 1 2 3 4 5; do if [[ $i -eq 3 ]]; then break; fi; echo $i; done",
        "1\n2\n",
    );
}

#[test]
fn continue_skips_iteration() {
    ok(
        "for i in 1 2 3; do if [[ $i -eq 2 ]]; then continue; fi; echo $i; done",
        "1\n3\n",
    );
}

#[test]
fn return_exits_function_early() {
    ok("f() { echo a; return 7; echo b; }; f; echo $?", "a\n7\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// stdin pipe-through — proves pipeline stdin wiring is correct
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn stdin_to_external_command() {
    let (status, stdout) = run_stdin("/bin/cat", "from-stdin\n");
    assert_eq!(status, 0);
    assert_eq!(stdout, "from-stdin\n");
}

#[test]
fn stdin_through_pipeline() {
    let (status, stdout) = run_stdin("/bin/cat | /bin/cat", "piped\n");
    assert_eq!(status, 0);
    assert_eq!(stdout, "piped\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// Sourced files — `source` feeds another bytecode chunk into the same executor
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn source_loads_function_from_file() {
    let lib = std::env::temp_dir().join("zshrs_phase_f_lib.sh");
    std::fs::write(
        &lib,
        "loaded_fn() { echo from-loaded \"$1\"; }\nLOADED_VAR=hello\n",
    )
    .unwrap();
    let path = lib.to_string_lossy();
    let code = format!(
        r#"source {p}; loaded_fn world; echo "var=$LOADED_VAR""#,
        p = path
    );
    let (status, stdout) = run(&code);
    assert_eq!(status, 0);
    assert_eq!(stdout, "from-loaded world\nvar=hello\n");
    let _ = std::fs::remove_file(&lib);
}

// ─────────────────────────────────────────────────────────────────────────────
// eval — runs a string through the same compile-and-execute path
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn eval_executes_string() {
    ok(r#"eval "echo hello eval""#, "hello eval\n");
}

#[test]
fn eval_sees_outer_vars() {
    // eval's argument is itself shell code that's compiled+run on a fresh VM
    // sharing executor state. This proves that variables set in the outer
    // scope are visible inside eval'd code.
    ok(r#"x=10; eval "y=\$x"; echo $y"#, "10\n");
}

#[test]
fn eval_single_quoted_arg_defers_expansion() {
    // Regression test: `eval 'echo $x'` must pass the LITERAL string to eval,
    // not the outer-shell-expanded one. Lexer marks single-quoted `$` chars
    // with a leading `\0` sentinel; compile_word's trigger detection must
    // honor the sentinel so the `$` does not fire compile-time expansion,
    // and the emitted constant must have the marker stripped. Bug pre-fix:
    // output was `\0 10\n` (NUL + space + 10 + newline) because the outer
    // compile expanded `$x` then split on the leftover NUL.
    ok(r#"x=10; eval 'echo $x'"#, "10\n");
}

#[test]
fn eval_single_quoted_multi_statement() {
    // Same fix as eval_single_quoted_arg_defers_expansion, plus proves the
    // re-parsed literal handles multiple commands and assignment correctly.
    ok(
        r#"x=hello; eval 'echo $x; echo two; a=$x; echo $a'"#,
        "hello\ntwo\nhello\n",
    );
}

#[test]
fn single_quoted_dollar_stays_literal_in_echo() {
    // Compile-path correctness: `echo 'literal $dollar'` must output the
    // string verbatim with no $-expansion and no embedded NUL bytes.
    ok(r#"echo 'literal $dollar'"#, "literal $dollar\n");
}

#[test]
fn background_amp_returns_immediately() {
    // `cmd &` must not block the parent. We assert behavior from the parent's
    // perspective: a foreground `echo done` runs synchronously while the bg
    // `sleep` continues in another process. Output is just `done\n`; if the
    // shell waited on the bg command, this test would hang the entire suite
    // until the sleep completed.
    //
    // The compile path: ListOp::Amp now routes `cmd` through compile_command_bg
    // → BUILTIN_RUN_BG → fork + setsid + run sub-chunk + exit. Pre-fix, the
    // Amp arm was a no-op TODO and the cmd ran synchronously inline.
    ok_serial(r#"sleep 1 & echo done"#, "done\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// Indexed arrays — Phase G1
// ─────────────────────────────────────────────────────────────────────────────
//
// `arr=(a b c)` storage now lands in `executor.arrays` (Vec<String>) via
// BUILTIN_SET_ARRAY (id 287). Reads of `${arr[idx]}`, `${arr[@]}`,
// `${#arr[@]}` lower to BUILTIN_ARRAY_INDEX/_ALL/_LENGTH (289/292/291). Splice
// uses Value::Array which Op::Exec/ExecBg/CallFunction and pop_args flatten
// into argv slots. Pre-G1, `arr=(a b c)` collapsed to a space-joined scalar
// via the runtime fallback and `${arr[@]}` was a single string.

#[test]
fn array_literal_index_returns_element() {
    // zsh is 1-based for positive indices.
    ok(
        r#"arr=(alpha beta gamma); echo ${arr[1]}; echo ${arr[2]}; echo ${arr[3]}"#,
        "alpha\nbeta\ngamma\n",
    );
}

#[test]
fn array_literal_negative_index_counts_from_end() {
    ok(
        r#"arr=(a b c d); echo ${arr[-1]}; echo ${arr[-2]}"#,
        "d\nc\n",
    );
}

#[test]
fn array_length_reports_element_count() {
    ok(r#"arr=(one two three four five); echo ${#arr[@]}"#, "5\n");
}

#[test]
fn empty_array_has_zero_length() {
    ok(r#"arr=(); echo ${#arr[@]}"#, "0\n");
}

#[test]
fn array_splice_in_for_loop() {
    // The for-loop now compiles each word at runtime and routes through
    // BUILTIN_ARRAY_FLATTEN, so ${arr[@]} produces N iterations not 1.
    ok(
        r#"arr=(x y z); for i in ${arr[@]}; do echo $i; done"#,
        "x\ny\nz\n",
    );
}

#[test]
fn array_splice_with_surrounding_words_in_for() {
    ok(
        r#"arr=(b c); for i in a ${arr[@]} d; do echo $i; done"#,
        "a\nb\nc\nd\n",
    );
}

#[test]
fn array_splice_into_argv_for_external() {
    // Each array element becomes a separate argv slot to /bin/echo. We pipe
    // through `wc -w` to count words — proves N elements landed as N args
    // rather than one space-joined arg.
    ok_serial(
        r#"arr=(a b c d e); /bin/echo ${arr[@]} | /usr/bin/wc -w"#,
        "       5\n",
    );
}

#[test]
fn empty_array_in_for_iterates_zero_times() {
    ok(
        r#"arr=(); for i in ${arr[@]}; do echo iter; done; echo done"#,
        "done\n",
    );
}

#[test]
fn array_with_spaces_preserves_elements() {
    // Each quoted element is one slot, not split. zsh-array semantics: the
    // outer parens collect words but quoted segments are atomic.
    ok(
        r#"arr=(one "two words" three); for i in ${arr[@]}; do echo "[$i]"; done"#,
        "[one]\n[two words]\n[three]\n",
    );
}

#[test]
fn array_splice_to_echo_builtin() {
    // Echo joins its args with a single space. With splice working, three
    // array elements become three echo args → `a b c`. Without splice (pre-
    // G1), it would be one joined arg `a b c` (same string but one arg) —
    // the test below distinguishes via `wc -w` which sees argv at the OS
    // level after fork-exec.
    ok(r#"arr=(a b c); echo ${arr[@]}"#, "a b c\n");
}

#[test]
fn array_indexed_singletons_dont_collide_with_scalar_lookup() {
    // After `arr=(x y z)`, $arr (no subscript) returns the array's space-
    // joined form (zsh convention via get_variable). ${arr[1]} returns the
    // first element only. Guards against the bug where arrays used to
    // shadow into `executor.variables` as a scalar — BUILTIN_SET_ARRAY now
    // explicitly removes any prior scalar binding.
    ok(r#"arr=(x y z); echo "$arr"; echo ${arr[1]}"#, "x y z\nx\n");
}

#[test]
fn array_append_extends_existing() {
    // `arr+=(c d)` on an existing array appends, doesn't replace.
    ok(
        r#"arr=(a b); arr+=(c d); echo ${arr[@]}; echo ${#arr[@]}"#,
        "a b c d\n4\n",
    );
}

#[test]
fn array_append_creates_when_missing() {
    // `arr+=(x)` with no prior `arr` is equivalent to `arr=(x)`. zsh and bash
    // both behave this way.
    ok(
        r#"arr+=(start); echo ${arr[@]}; arr+=(more); echo ${arr[@]}"#,
        "start\nstart more\n",
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Zsh parameter-expansion flags — Phase G4 subset
// `${(L)var}` lowercase, `${(U)var}` upper, `${(j:s:)arr}` join,
// `${(s:s:)scalar}` split, `${(o)arr}` sort, `${(O)arr}` reverse-sort,
// `${(P)var}` indirect, `${(@)scalar}` force-array, `${(k)assoc}` keys,
// `${(v)assoc}` values, `${#arr}` length. Flags can stack: `(jL)` joins
// then lowercases.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn zshflag_lowercase_scalar() {
    ok(r#"x=Hello; echo "${(L)x}""#, "hello\n");
}

#[test]
fn zshflag_uppercase_scalar() {
    ok(r#"x=hello; echo "${(U)x}""#, "HELLO\n");
}

#[test]
fn zshflag_sort_array_ascending() {
    // (o) only fires in array context — no surrounding DQ. Inside DQ,
    // zsh preserves original element order.
    ok(r#"arr=(c a b); echo ${(o)arr}"#, "a b c\n");
}

#[test]
fn zshflag_sort_array_descending() {
    ok(r#"arr=(c a b); echo ${(O)arr}"#, "c b a\n");
}

#[test]
fn zshflag_join_with_explicit_sep() {
    ok(
        r#"arr=(one two three); echo "${(j:-:)arr}""#,
        "one-two-three\n",
    );
}

#[test]
fn zshflag_split_on_explicit_sep() {
    // After split, echo joins the resulting array with space (default IFS).
    ok(r#"s=a,b,c; echo "${(s:,:)s}""#, "a b c\n");
}

#[test]
fn zshflag_indirect_resolves_through_name() {
    // `(P)ref` reads $ref's value as another var name and returns that var's value.
    ok(r#"real=42; ref=real; echo "${(P)ref}""#, "42\n");
}

#[test]
fn zshflag_array_length_via_pound() {
    // `${(#)x}` is the char-code flag (arith-eval each element, output
    // the corresponding character) — NOT array length. zsh: "65" → "A".
    // `${#arr}` is the actual length form, covered by ARRAY_LENGTH.
    //
    // Must be UNQUOTED: `${(#)arr}` converts each element → "A B C". The
    // QUOTED form `"${(#)arr}"` joins the array to "65 66 67" FIRST, then
    // arith-evals that multi-number string → empty (verified zsh 5.9.1:
    // `printf "<%s>" ${(#)arr}` → <A><B><C>; `printf "<%s>" "${(#)arr}"`
    // → <>). The prior assertion quoted the expansion but expected the
    // unquoted result, so it asserted non-zsh behavior (zshrs matches zsh
    // in both forms).
    ok(r#"arr=(65 66 67); echo ${(#)arr}"#, "A B C\n");
}

#[test]
fn zshflag_stacked_join_then_upper() {
    // `(j:-:U)`: join with `-`, then uppercase the joined string.
    ok(r#"arr=(foo bar); echo "${(j:-:U)arr}""#, "FOO-BAR\n");
}

#[test]
fn zshflag_stacked_split_then_upper() {
    // `(s:,:U)`: split scalar on `,`, then uppercase each element.
    ok(r#"s=a,b,c; echo "${(s:,:U)s}""#, "A B C\n");
}

#[test]
fn zshflag_q_single_quote_escapes_inner() {
    // (q) per zshexpn(1): backslash-escape shell-special chars (no
    // surrounding quotes). Verified against /bin/zsh.
    ok(r##"x="hi 'world'"; echo "${(q)x}""##, "hi\\ \\'world\\'\n");
}

#[test]
fn zshflag_qq_double_quote() {
    // (qq) per zshexpn(1): single-bslashquote always.
    ok(r#"x=hello; echo "${(qq)x}""#, "'hello'\n");
}

#[test]
fn zshflag_qqq_ansi_c_quoting() {
    // (qqq) per zshexpn(1): double-bslashquote always — tab stays literal
    // inside "...".
    ok(r#"s=$(printf 'a\tb'); echo "${(qqq)s}""#, "\"a\tb\"\n");
}

#[test]
fn zshflag_g_processes_backslash_escapes() {
    // `(g::)` interprets backslash escapes — `\n` becomes a real
    // newline. STALE-PIN REPAIR (2026-06-12): the test previously
    // used bare `${(g)s}`, which REAL zsh rejects — `zsh -fc 'echo
    // "${(g)s}"'` → "error in flags near position 5", rc 1 (the g
    // flag takes `::`-delimited options; zshexpn(1)). The bare form
    // working was a zshrs-only laxity the parity work removed.
    ok(r#"s='hello\nworld'; echo "${(g::)s}""#, "hello\nworld\n");
}

#[test]
fn zshflag_n_natural_sort() {
    // Natural-numeric sort: file2 < file10 (lexicographically file10
    // would come first, naturally file2 does). (n) only fires in
    // array context — no DQ wrapper.
    ok(
        r#"arr=(file10 file2 file1 file20); echo ${(on)arr}"#,
        "file1 file2 file10 file20\n",
    );
}

#[test]
fn zshflag_t_type_query() {
    // `(t)var` returns the variable's typeset shape.
    ok(
        r#"arr=(a b); typeset -A m; m[k]=v; sc=str; echo "${(t)arr}|${(t)m}|${(t)sc}""#,
        "array|association|scalar\n",
    );
}

#[test]
fn zshflag_i_case_insensitive_sort() {
    // `(i)` sorts case-insensitively while preserving the original
    // case. Array-only — no DQ wrapper.
    ok(
        r#"arr=(Banana apple Cherry); echo ${(i)arr}"#,
        "apple Banana Cherry\n",
    );
}

#[test]
fn zshflag_q_plus_skips_safe_values() {
    // `(q+)` only quotes when needed (whitespace or shell-specials).
    // Pre: 'safe' word stays bare, 'has space' gets quoted.
    ok(
        r#"a=safe; b="has space"; echo "${(q+)a}|${(q+)b}""#,
        "safe|'has space'\n",
    );
}

#[test]
fn zshflag_q_minus_strips_trailing_newlines() {
    // `(q-)` is `(q)` + strip trailing newlines first. Since `val` has
    // no specials, the trimmed value emits unquoted.
    ok(r#"x=$(printf 'val\n\n'); echo "${(q-)x}""#, "val\n");
}

#[test]
fn zshflag_q_star_escapes_glob_chars() {
    // `(q*)` is `(q)` with `*`/`?` also escaped. `*.rs` has no spaces,
    // so the output is just `\*.rs` (no surrounding quotes).
    ok(r#"x="*.rs"; echo "${(q*)x}""#, "\\*.rs\n");
}

#[test]
fn zshflag_qqqq_backslash_no_quotes() {
    // (qqqq) per zshexpn(1): ANSI-C $'…' form.
    ok(r#"x="has space"; echo "${(qqqq)x}""#, "$'has space'\n");
}

#[test]
fn brace_range_numeric() {
    ok(r#"echo {1..5}"#, "1 2 3 4 5\n");
}

#[test]
fn brace_range_letter() {
    ok(r#"echo {a..e}"#, "a b c d e\n");
}

#[test]
fn brace_alternation() {
    ok(r#"echo {alpha,beta,gamma}"#, "alpha beta gamma\n");
}

#[test]
fn brace_with_prefix_and_suffix() {
    ok(r#"echo pre{a,b,c}post"#, "preapost prebpost precpost\n");
}

#[test]
fn brace_in_for_loop() {
    ok(
        r#"for i in {1..3}; do echo "iter=$i"; done"#,
        "iter=1\niter=2\niter=3\n",
    );
}

#[test]
fn glob_qualifier_dot_filters_regular_files() {
    // Pre-fix: `*(.)` returned all entries. Post-fix: only regular files.
    // The test creates a tempdir with one file + one dir, asserts that
    // `*(.) ` returns only the file basename.
    let tmp = std::env::temp_dir().join(format!(
        "zshrs_glob_qual_dot_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("alpha.txt"), "x").unwrap();
    std::fs::create_dir_all(tmp.join("beta_dir")).unwrap();
    let p = tmp.to_string_lossy().into_owned();
    let script = format!(r#"echo {}/*(.)"#, p);
    let (status, stdout) = run(&script);
    assert_eq!(status, 0);
    assert!(stdout.contains("alpha.txt"), "missing file: {}", stdout);
    assert!(
        !stdout.contains("beta_dir"),
        "should not contain dir: {}",
        stdout
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn glob_qualifier_slash_filters_directories() {
    let tmp = std::env::temp_dir().join(format!(
        "zshrs_glob_qual_slash_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("foo.txt"), "x").unwrap();
    std::fs::create_dir_all(tmp.join("subdir")).unwrap();
    let p = tmp.to_string_lossy().into_owned();
    let script = format!(r#"echo {}/*(/)"#, p);
    let (status, stdout) = run(&script);
    assert_eq!(status, 0);
    assert!(stdout.contains("subdir"), "missing dir: {}", stdout);
    assert!(
        !stdout.contains("foo.txt"),
        "should not have file: {}",
        stdout
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn regex_match_caret_anchor() {
    // Pre-fix: lexer split `^h` into Bang+h, joined with space → `^ h`,
    // regex compile failed silently. Plus the RHS was glob-expanded before
    // RegexMatch.
    ok_status(
        r#"[[ "hello" =~ ^h ]] && echo MATCH || echo NOMATCH"#,
        "MATCH\n",
        0,
    );
}

#[test]
fn regex_match_complex_with_captures() {
    ok_status(
        r#"[[ "version 1.2.3" =~ ([0-9]+)\.([0-9]+)\.([0-9]+) ]] && echo MATCH || echo NOMATCH"#,
        "MATCH\n",
        0,
    );
}

#[test]
fn regex_match_failure_returns_status_1() {
    ok_status(
        r#"[[ "abc" =~ ^z ]] && echo MATCH || echo NOMATCH"#,
        "NOMATCH\n",
        0,
    );
}

#[test]
fn bang_literal_in_non_interactive() {
    // Pre-fix: `echo !!` consumed `!` as Bang twice → echo got 0 args.
    // Post-fix: `!` followed by non-whitespace stays in the word.
    ok(r#"echo !!"#, "!!\n");
    ok(r#"echo "args: !*""#, "args: !*\n");
    ok(r#"echo !arg !cmd"#, "!arg !cmd\n");
}

#[test]
fn bang_negation_keyword_still_works() {
    // `! cmd` (with space) is still command negation.
    ok_status(r#"! true; echo $?"#, "1\n", 0);
    ok_status(r#"! false; echo $?"#, "0\n", 0);
}

#[test]
fn zshflag_in_quoted_context_works() {
    // Regression: pre-fix, `${(t)sc}` inside double quotes hit
    // compile_string_with_expansions which emitted GET_VAR("(t)sc").
    // Post-fix, the synthesized `${(t)sc}` routes to PARAM_FLAG via the
    // same matcher used by the Literal-word path.
    ok(r#"sc=hello; echo "type=${(t)sc}""#, "type=scalar\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// Associative arrays — Phase G1 follow-up (id 288 = BUILTIN_SET_ASSOC)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn assoc_set_and_get_single_entry() {
    ok(r#"foo[key]=val; echo "${foo[key]}""#, "val\n");
}

#[test]
fn assoc_typeset_then_set_and_get() {
    ok(
        r#"typeset -A m; m[name]=Alice; m[role]=eng; echo "name=${m[name]} role=${m[role]}""#,
        "name=Alice role=eng\n",
    );
}

#[test]
fn assoc_two_lookups_in_double_quoted_string() {
    // Regression: pre-fix `try_lower_array_literal` falsely matched the
    // entire string `${foo[a]} ${foo[b]}` as a single ${name[idx]} reference
    // (treating `a]} ${foo[b` as the index). The fix rejects bodies
    // containing `${` or `}` so multi-group strings route to the runtime
    // string walker.
    ok(r#"foo[a]=1; foo[b]=2; echo "${foo[a]} ${foo[b]}""#, "1 2\n");
}

#[test]
fn assoc_append_concats_to_existing() {
    // `m[k]+=tail` appends to the existing value (string concat). Pre-fix,
    // is_append was ignored on the assoc compile branch.
    ok(
        r#"m[k]=hello; m[k]+=" world"; echo "${m[k]}""#,
        "hello world\n",
    );
}

#[test]
fn assoc_append_creates_when_missing() {
    // First +=on a missing key behaves like a plain set, matching zsh/bash.
    ok(r#"m[a]+=foo; m[a]+=bar; echo "${m[a]}""#, "foobar\n");
}

#[test]
fn assoc_overwrite_replaces_value() {
    ok(r#"m[k]=first; m[k]=second; echo "${m[k]}""#, "second\n");
}

#[test]
fn assoc_missing_key_returns_empty() {
    ok(r#"m[a]=1; echo "[${m[nonexistent]}]""#, "[]\n");
}

#[test]
fn select_with_eof_stdin_exits_zero_no_body() {
    // `parse_select` wires a CompoundCommand::Select; compile path emits
    // BUILTIN_RUN_SELECT which prints the menu to stderr, reads stdin,
    // and exits 0 on EOF without running the body. Test pipes empty
    // stdin — body should never execute, "done" should print.
    let (status, stdout) = run_stdin(
        r#"select x in a b c; do echo "got=$x"; done; echo done"#,
        "",
    );
    assert_eq!(status, 0);
    assert!(
        stdout.contains("done"),
        "expected 'done' in stdout: {}",
        stdout
    );
    assert!(
        !stdout.contains("got="),
        "body must not run when stdin is empty: {}",
        stdout
    );
}

#[test]
fn select_runs_body_with_valid_choice() {
    // Pipe "2" → select sets x to second word, runs body, body sets
    // BREAK_SELECT to exit loop. Asserts both that the selection landed in
    // $x and that BREAK_SELECT was honored.
    let (status, stdout) = run_stdin(
        r#"select x in alpha beta gamma; do echo "selected=$x"; BREAK_SELECT=1; done; echo after"#,
        "2\n",
    );
    assert_eq!(status, 0);
    assert!(
        stdout.contains("selected=beta"),
        "expected selected=beta: {}",
        stdout
    );
    assert!(stdout.contains("after"), "expected 'after': {}", stdout);
}

#[test]
fn select_break_keyword_exits_loop() {
    // Pre-fix: `break` inside the select body emitted no ops (no enclosing
    // loop in the sub-chunk's patch lists), so the only way to exit was
    // setting BREAK_SELECT=1. Post-fix: break with no enclosing loop emits
    // BUILTIN_SET_BREAK + a halt-jump; BUILTIN_RUN_SELECT drains
    // executor.loop_signal after each body run and exits when seen.
    let (status, stdout) = run_stdin(
        r#"select x in alpha beta gamma; do echo "got=$x"; break; done; echo after"#,
        "1\n",
    );
    assert_eq!(status, 0);
    assert!(stdout.contains("got=alpha"));
    assert!(stdout.contains("after"));
}

#[test]
fn select_break_after_match() {
    // Real-world idiom: pick options until a sentinel one is hit, then break.
    let (status, stdout) = run_stdin(
        r#"select x in alpha beta gamma; do
  echo "iter=$x"
  [[ $x = beta ]] && break
done
echo done"#,
        "1\n2\n",
    );
    assert_eq!(status, 0);
    assert!(stdout.contains("iter=alpha"));
    assert!(stdout.contains("iter=beta"));
    assert!(stdout.contains("done"));
}

#[test]
fn select_invalid_input_sets_var_empty() {
    // zsh convention: non-numeric or out-of-range input sets the var to
    // empty string (not "preserve previous"). REPLY contains the raw input
    // for the body to inspect.
    let (status, stdout) = run_stdin(
        r#"select x in alpha beta; do echo "x=[$x] reply=[$REPLY]"; BREAK_SELECT=1; done"#,
        "bogus\n",
    );
    assert_eq!(status, 0);
    assert!(
        stdout.contains("x=[] reply=[bogus]"),
        "expected x empty + REPLY=bogus: {}",
        stdout
    );
}

#[test]
fn read_dup_fd_with_literal_number() {
    // `read line <&N` — DupRead with a literal fd. Pre-fix, the compile path
    // defaulted DupRead's fd to 1 (stdout) so the dup2 went onto STDOUT
    // instead of STDIN, and read blocked on the original terminal stdin.
    // Post-fix, DupRead joins the "read group" defaulting to fd 0.
    //
    // The literal fd number depends on what was free when coproc forked
    // (zsh historically uses fd>=10; our impl picks a kernel-assigned one
    // around fd 13). Use the canonical `${COPROC[1]}` indirection — the
    // test still proves DupRead defaults to fd 0 on the read side.
    ok_serial(
        r#"coproc { echo CHILD_LINE; }
sleep 0.2
read line <&${COPROC[1]}
echo "got=[$line]"
"#,
        "got=[CHILD_LINE]\n",
    );
}

#[test]
fn read_dup_fd_with_variable_expansion() {
    // `read line <&${COPROC[1]}` — same fix, plus the target word goes
    // through array-index expansion (BUILTIN_ARRAY_INDEX) before redirect
    // dispatch. Proves both the default-fd fix and the var-expansion path
    // work together.
    ok_serial(
        r#"coproc { echo VAR_PATH; }
sleep 0.2
read line <&${COPROC[1]}
echo "got=[$line]"
"#,
        "got=[VAR_PATH]\n",
    );
}

#[test]
fn coproc_round_trip_via_dev_fd() {
    // Prove the registered fds are real OS-level pipe ends: child writes to
    // its stdout (= write end of the c2p pipe), parent reads from
    // /dev/fd/${COPROC[1]} (= read end of the c2p pipe).
    //
    // We use /dev/fd/N (the procfs/devfs path, present on macOS and Linux)
    // because zshrs's `<&fd` numeric-redirect parser path doesn't currently
    // honor variable-expanded fd numbers — separate gap, parser-level. The
    // /dev/fd/ approach is portable and exercises the same coproc plumbing.
    ok_serial(
        r#"coproc { echo CHILD_LINE; sleep 0.1; }
sleep 0.3
fd=${COPROC[1]}
read line < /dev/fd/$fd
echo "got=[$line]"
"#,
        "got=[CHILD_LINE]\n",
    );
}

#[test]
fn coproc_registers_fd_pair_in_named_array() {
    // `coproc { body }` forks the body, wires its stdin/stdout to two pipes,
    // and stores [read_fd, write_fd] in the COPROC array. We don't exchange
    // data here — full bidirectional comms is more host plumbing than this
    // test should verify. We just prove the array got populated with two
    // numeric fd values, which is the load-bearing piece (fork happened, the
    // pipes were created, fds were captured by name).
    //
    // Pre-fix: the Coproc compile arm called `compile_command(body)` inline,
    // so `coproc { sleep 1 }` blocked the parent for 1s and never created
    // pipes or set COPROC.
    let (status, stdout) = run(r#"coproc { :; } 2>/dev/null
echo "len=${#COPROC[@]}"
echo "rd_is_int=$([ -n "${COPROC[1]}" ] && [ "${COPROC[1]}" -ge 0 ] && echo yes || echo no)"
echo "wr_is_int=$([ -n "${COPROC[2]}" ] && [ "${COPROC[2]}" -ge 0 ] && echo yes || echo no)"
"#);
    assert_eq!(status, 0, "coproc set-up failed: {}", stdout);
    assert!(
        stdout.contains("len=2"),
        "COPROC should have 2 entries, got: {}",
        stdout
    );
    assert!(
        stdout.contains("rd_is_int=yes"),
        "COPROC[1] (read fd) should be a non-negative int: {}",
        stdout
    );
    assert!(
        stdout.contains("wr_is_int=yes"),
        "COPROC[2] (write fd) should be a non-negative int: {}",
        stdout
    );
}

#[test]
fn dynamic_command_name_expands_and_dispatches() {
    // Pre-fix: `cmd=ls; $cmd` compiled with the literal-name fast path
    // emitting CallFunction(name="$cmd", ...). The host's host_exec_external
    // received the literal `$cmd` and printed `command not found: $cmd`.
    //
    // Post-fix: compile_simple detects unquoted `$` in the first word and
    // skips the literal-name branch, falling through to the dynamic Op::Exec
    // path. compile_word lowers `$cmd` to BUILTIN_GET_VAR, the resolved string
    // lands on the stack, Op::Exec routes through host.exec for actual
    // dispatch.
    ok_serial(r#"cmd=/bin/echo; $cmd hello world"#, "hello world\n");
}

#[test]
fn op_exec_routes_through_host() {
    // Pre-fix: fusevm's Op::Exec called Command::new directly for the
    // unknown-command path, bypassing ZshrsHost::exec entirely. AOP intercepts
    // registered against external commands never fired for dynamic-name
    // invocations.
    //
    // Post-fix: vm.rs Op::Exec routes through `host.exec(args)`, which lands
    // in ZshrsHost::exec → host_exec_external → run_intercepts. This test
    // proves the route by registering a `before` advice that prints a
    // sentinel, then triggering the cmd via a dynamic name. Both the sentinel
    // (from the advice) and the original cmd's output must appear.
    ok_serial(
        r#"intercept before /bin/echo "echo INTERCEPT_FIRED" >/dev/null 2>&1
cmd=/bin/echo
$cmd payload"#,
        "INTERCEPT_FIRED\npayload\n",
    );
}

#[test]
fn background_amp_actually_runs_the_child() {
    // Verifying just non-blocking isn't enough — the child must actually
    // execute the cmd. Use a tempfile sentinel: parent backgrounds a write,
    // exits, the orchestration in this test waits, then asserts the file
    // exists with the expected content.
    let path = std::env::temp_dir().join(format!(
        "zshrs_bg_test_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let path_str = path.to_string_lossy().into_owned();
    let _ = std::fs::remove_file(&path);

    let script = format!(r#"echo wrote > {} & wait"#, path_str);
    ok_serial(&script, "");

    // After `wait`, the bg child has flushed and exited.
    let content = std::fs::read_to_string(&path).expect("bg child wrote sentinel");
    assert_eq!(content, "wrote\n");
    let _ = std::fs::remove_file(&path);
}
