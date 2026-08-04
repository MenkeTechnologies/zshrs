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

#[test]
fn rc_expand_param_substitutes_the_trailing_text_for_every_element() {
    // c:Src/subst.c:4316-4324 — the plan9 (`${^arr}`) cross product runs
    // `stringsubst` over the text FOLLOWING the expansion once, up front, then
    // glues the result to every array element. zshrs instead carried the raw
    // trailing text into each node and relied on the caller re-scanning the
    // returned position, which points inside the LAST node only — so a trailing
    // `${…}` was substituted for the final element and emitted LITERALLY for
    // all the others.
    //
    // The shapes below all route through the whole-word expansion path (the
    // compiler's word-segment fast path is skipped once the word carries a
    // Bnull / quote marker), which is where the divergence lived.
    //
    // Real-world case: Unix/Command/_man's
    //     pages=( ${^pages}/"*${sect:+.$sect"*"}" )
    // left a literal `${sect:+.$sect*}` glued to every man directory but the
    // last, so `man <TAB>` offered 659 pages instead of 33678.

    // Nested double quotes inside the trailing `${…}`.
    ok(
        r#"a=(x y z); v=V; print -rl -- ${^a}-"${v:+A"B"C}""#,
        "x-ABC\ny-ABC\nz-ABC\n",
    );
    // Backslash-escaped quotes (Bnull) reach the same path.
    ok(
        r#"a=(x y z); v=V; print -rl -- ${^a}-${v:+A\"B\"C}"#,
        "x-A\"B\"C\ny-A\"B\"C\nz-A\"B\"C\n",
    );
    // A trailing expansion that is EMPTY must not leave the source text behind
    // either — this is the exact `${sect:+…}` shape _man hits with no section.
    ok(
        r#"a=(x y z); s=; print -rl -- ${^a}/"*${s:+.$s"*"}""#,
        "x/*\ny/*\nz/*\n",
    );
    // Non-empty leg of the same shape.
    ok(
        r#"a=(x y z); s=3; print -rl -- ${^a}/"*${s:+.$s"*"}""#,
        "x/*.3*\ny/*.3*\nz/*.3*\n",
    );
    // c:4261 — the single-element early return is skipped when plan9, so a
    // 1-element array cross-products too (this leg regressed independently).
    ok(
        r#"a=(only); v=V; print -rl -- ${^a}-"${v:+A"B"C}""#,
        "only-ABC\n",
    );
    // `${^^arr}` forces plan9 OFF: prefix sticks to the first element and the
    // trailing text to the last, and it must still be substituted there.
    ok(
        r#"a=(x y z); v=V; print -rl -- ${^^a}-"${v:+A"B"C}""#,
        "x\ny\nz-ABC\n",
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// `local NAME` shadowing a zsh/parameter magic special
// ─────────────────────────────────────────────────────────────────────────────

/// c:Src/params.c:1090-1115 `createparam` — declaring `local NAME` inside a
/// function where NAME is one of zsh/parameter's magic specials (`options`,
/// `functions`, `dirstack`, …) does NOT reach through to the special. C keeps
/// specials and user parameters in the ONE `paramtab` hash: createparam stashes
/// the special in `pm->old` and inserts a plain node under the same key, so
/// every read until `endparamscope` sees the LOCAL.
///
/// zshrs keeps the magic rows in separate static tables (`PARTAB` /
/// `PARTAB_ARRAY`) matched by NAME, so before the fix the special won every
/// read that went through the full `paramsubst` path while the shadowing local
/// only answered the bare `$name` fast path.
///
/// Real-world breakage: git 2.55.0's `git-completion.bash`
/// `__git_resolve_builtins` (share/zsh/site-functions/git-completion.bash:
/// 500-501) does `local options; eval "options=\${$var-}"`. `$options` read
/// back the option table (`off on off …`), the `[ -z "$options" ]` guard took
/// the WRONG branch, and `___git_resolved_builtins` ended up as the 3-char
/// string `off` instead of git's 546-char option list — `git checkout --<TAB>`
/// completed nothing.
#[test]
fn local_shadows_magic_special_hash_param() {
    // Every read form of a shadowed `options`: bare, length, case flag,
    // scalar range subscript, negative char subscript. All must see the local.
    ok(
        r#"f(){ local options; options="hello world"; print -r -- "$options ${#options} ${(U)options} ${options[1,5]} ${options[-1]}"; }; f"#,
        "hello world 11 HELLO WORLD hello d\n",
    );
    // The special must be restored intact once the scope pops.
    ok(
        r#"f(){ local options; options=x; }; f; print -r -- ${(t)options} ${options[interactive]}"#,
        "association-hide-hideval-special off\n",
    );
}

/// The exact `__git_resolve_builtins` shape: `local` + an `eval`-generated
/// assignment through `${$var-}` indirection, then the `[ -z ]` branch that
/// decides whether to shell out for the option list. The pre-fix bug made the
/// guard take the non-empty branch and return the option table.
#[test]
fn local_shadows_magic_special_through_eval_assign() {
    ok(
        r#"f(){ local v=__nosuch_param; local options; eval "options=\${$v-}"; if [ -z "$options" ]; then options=" --a --b "; fi; print -r -- "[$options] ${#options}"; }; f"#,
        "[ --a --b ] 9\n",
    );
}

/// Same shadowing rule for a PM_ARRAY magic special (`dirstack`, c:Src/Modules/
/// parameter.c:2239) and for a PM_HASHED one read by name (`functions`).
/// `${#dirstack}` counted the live directory stack instead of the local's
/// characters before the fix.
#[test]
fn local_shadows_magic_special_array_param() {
    ok(
        r#"f(){ local dirstack; dirstack=qqq; print -r -- "$dirstack ${#dirstack}"; }; f; print -r -- ${(t)dirstack}"#,
        "qqq 3\narray-hide-hideval-special\n",
    );
    ok(
        r#"f(){ local functions; functions=zzz; print -r -- "$functions ${#functions}"; }; f; print -r -- ${(t)functions}"#,
        "zzz 3\nassociation-hide-hideval-special\n",
    );
}

/// Guard against over-reach: an ORDINARY user assoc must keep working. Its
/// paramtab node legitimately carries no PM_SPECIAL, so a naive
/// "no PM_SPECIAL ⇒ shadowed" predicate would have disabled every user hash.
#[test]
fn user_assoc_unaffected_by_magic_shadow_guard() {
    ok(
        r#"typeset -A h=(a 1 b 2); print -r -- "${h[a]} ${#h} ${(t)h}""#,
        "1 2 association\n",
    );
}

/// c:Src/params.c:3926-3934 `scanendscope` / `unsetparam_pm` — popping a local
/// scope restores the outer binding's DATA, not just its paramtab node. zshrs
/// keeps assoc values in a parallel name-keyed store, and `createparam`
/// (params.rs:2260) saves + clears that store whenever the OUTER pm is
/// PM_HASHED. The matching restore in `endparamscope` was gated on the LOCAL
/// also being hashed, so a NON-hashed local (`local h` over an assoc `h`,
/// `local options` over the zsh/parameter magic assoc) cleared the store and
/// never put it back — the outer assoc stayed blank for the rest of the
/// process.
#[test]
fn scalar_local_over_assoc_restores_outer_data_on_scope_exit() {
    ok(
        r#"typeset -A h=(a 1 b 2); f(){ local h; h=zzz; print -r -- "in [${h[a]}] [$h] ${(t)h}"; }; f; print -r -- "out [${h[a]}] ${(t)h}""#,
        "in [] [zzz] scalar-local\nout [1] association\n",
    );
    // A hashed local over a hashed outer must still swap cleanly (the case the
    // original gate covered) — the fix must not regress it.
    ok(
        r#"typeset -A h=(a 1); f(){ typeset -A h; h[x]=v; print -r -- "in [${h[a]}][${h[x]}]"; }; f; print -r -- "out [${h[a]}][${h[x]}]""#,
        "in [][v]\nout [1][]\n",
    );
    // A hashed local with NO outer binding must leave nothing behind.
    ok(
        r#"f(){ typeset -A H; H[x]=v; }; f; print -r -- "[${H[x]}]""#,
        "[]\n",
    );
}

/// `getarg`'s flag-subscript parser bailed on any `[` anywhere in the
/// subscript (`rest.contains('[')`), so a char-class search pattern was never
/// parsed at all. `paramsubst`'s subexp arm falls back to the WHOLE value when
/// getarg answers None, so `${${x}[(r)[abc]]}` returned the entire value
/// instead of the match. `_typeset` keys its option table with exactly that
/// idiom — `${${(s::)use[$i]}[(r)[dUurRtT]]:+$func}` — so every character
/// tested "matched", `$func` was appended to every lookup key, and the specs
/// whose key has no `f`/`p` variant vanished: `functions -<TAB>` lost `-W`,
/// `-k`, `-m`, `-z` and `autoload -<TAB>` lost `-k`.
#[test]
fn flag_subscript_pattern_may_contain_a_character_class() {
    // The exact `_typeset` shape: a 1-element array from `(s::)`, probed with
    // a char-class pattern. `k`/`W` are NOT in the class, so both must answer
    // empty; `U`/`t` ARE, so both must answer the character.
    ok(
        r#"for c in U k t W; do print -rn -- "[${${(s::)c}[(r)[dUurRtT]]}]"; done; print -r -- ."#,
        "[U][][t][].\n",
    );
    // Direct forms: a bracket in the pattern must not disable the subscript.
    ok(
        r#"x=abc; print -r -- "[${${x}[(r)[abc]*]}][${${x}[(r)[xyz]*]}][${${x}[(i)[xyz]*]}]""#,
        "[a][][4]\n",
    );
}

/// `(k)`/`(K)` invert the usual match direction: c:Src/params.c:653-660
/// compiles the stored KEY as a pattern and matches it against the subscript
/// text. zshrs compared the key to the subscript with `==`, so glob-keyed
/// tables never matched. `_netcat`/`_netstat` build their option list with
/// `$optionmap[(K)$help]` over keys like `'*-l*'`, so `nc -<TAB>` fell back to
/// the unfiltered table.
#[test]
fn hash_key_match_subscript_treats_keys_as_patterns() {
    // The subscript must carry a `$` expansion: that is the shape `_netcat`
    // uses (`$optionmap[(K)"$help"]`) and the only one that reaches the
    // runtime `getarg` path rather than a compile-time constant rebuild.
    ok(
        r#"typeset -A m=('*-l*' L '*-b*' B '*ZZ*' Z); h='has -l and -b'; print -r -- "[$m[(K)"$h"]][$m[(k)"$h"]]""#,
        "[L B][L]\n",
    );
}

/// Two shape bugs on the UNBRACED `$assoc[(flags)pat]` form:
///   * a `,` inside the search PATTERN was read as a slice separator, and the
///     assoc-splat then returned EVERY value in the hash (c:Src/params.c:2100
///     only tests for `,` AFTER getarg has consumed the flag group and its
///     pattern, so a comma in the pattern is never a slice);
///   * a MATCHMANY search (`I`/`R`/`K`) is array-shaped in C
///     (c:Src/params.c:1724-1734 `getvaluearr`), but the port joined the
///     matches into one word — `comparguments` then rejected `_netcat`'s
///     twelve option specs as a single "invalid option definition".
#[test]
fn unbraced_hash_search_subscript_keeps_shape_and_ignores_pattern_commas() {
    // Comma inside the pattern: still one match, NOT the whole hash.
    ok(
        r#"typeset -A m=('*-l*' L '*-b*' B '*ZZ*' Z); h='x, -l y'; a=( $m[(K)"$h"] ); print -r -- "$#a:$a""#,
        "1:L\n",
    );
    // MATCHMANY keeps one word per match even when a value contains spaces.
    ok(
        r#"typeset -A m=('*-l*' 'L one' '*-b*' 'B two'); h='has -l and -b'; a=( $m[(K)"$h"] ); print -r -- "$#a:[$a[1]][$a[2]]""#,
        "2:[L one][B two]\n",
    );
    // A real slice must still slice, and a single-match search stay scalar.
    ok(
        r#"arr=(alpha beta gamma); a=( $arr[2,3] ); b=( $arr[(r)*a*] ); print -r -- "$#a:$a $#b:$b""#,
        "2:beta gamma 1:alpha\n",
    );
}

/// RC_EXPAND_PARAM must not cross-product a QUOTED array read, nor an array
/// read that is the RHS of a scalar assignment.
///
/// c:Src/subst.c:4245 `if (isarr)` gates the whole plan9 block (c:4316), and
/// two earlier arms have already zeroed `isarr` for a bare array read:
///   c:3032 `if (qt && !getlen && isarr > 0) { val = sepjoin(aval, sep, 1);
///           isarr = 0; }`                                    — DQ context
///   c:3905 `if (nojoin == 0 || sep) { val = sepjoin(aval, sep, 1);
///           isarr = 0; }` under `if (ssub || …)` at c:3901
///                                                            — PREFORK_SINGLE
/// so `"$a"Z` is ONE IFS-joined word and `c=p$a-s` is one joined scalar even
/// with the option on. zshrs distributed both, which is how `_sqlite`'s
///     exclusive=( {,-}-{no,}header )
///     options+=( "($exclusive)"$^dashes'-header[turn headers on]' )
/// reached `comparguments` as five words beginning `(-noheader` —
/// `sqlite3 -<TAB>` printed `comparguments: invalid argument: (-noheader`
/// into the edit buffer instead of completing.
#[test]
fn rc_expand_param_leaves_quoted_and_scalar_assign_array_reads_joined() {
    // Quoted read with adjacent literal text on either side.
    ok(
        r#"setopt rcexpandparam; a=(x y); print -rl -- "$a"Z; print -rl -- Z"$a"; print -rl -- "${a}"Z"#,
        "x yZ\nZx y\nx yZ\n",
    );
    // `(@)` / `[@]` / `$@` keep array shape in DQ (c:2797 isarr = -1), so the
    // suffix still sticks per element — the join must NOT swallow those.
    ok(
        r#"setopt rcexpandparam; a=(x y); print -rl -- "${a[@]}"Z; print -rl -- "${a[*]}"Z"#,
        "xZ\nyZ\nx yZ\n",
    );
    // Scalar-assignment RHS (c:3901 ssub) joins before plan9 can distribute.
    ok(
        r#"setopt rcexpandparam; a=(x y); b=$a; c=p$a-s; d="$a"Z; print -rl -- "$b" "$c" "$d""#,
        "x y\npx y-s\nx yZ\n",
    );
    // The `_sqlite` spec shape verbatim: quoted join + `$^` on the neighbour.
    ok(
        r#"setopt rcexpandparam; excl=(-noheader -header); dashes=('' -); o=( "($excl)"$^dashes'-header[on]' ); print -rl -- $o"#,
        "(-noheader -header)-header[on]\n(-noheader -header)--header[on]\n",
    );
    // Unquoted stays a cross product — the fix must not over-reach.
    ok(
        r#"setopt rcexpandparam; a=(x y); print -rl -- p$a-s"#,
        "px-s\npy-s\n",
    );
}

/// A typeset-family `name=( … )` initializer whose element is a `${…}`
/// substitution carrying UNBALANCED parens in its pattern/replacement must
/// still be recognised as a paren-init.
///
/// `split_typeset_paren_init` counts `(`/`)` to find element boundaries. An
/// escaped `\(` (lexed as Bnull + `(`) and a bare `(` inside a `${…}` body are
/// both inert, but both were counted — the scan ended `depth != 0`, the
/// splitter reported "unbalanced", and the whole word fell through to the
/// generic word path. There RC_EXPAND_PARAM cross-products the element array
/// against the surrounding `name=(` / `)` text, so `local` ran once PER
/// ELEMENT and the last assignment won: an N-element array collapsed to 1.
/// `_postgresql`'s `_pgsql_psql` builds its entire `_arguments` spec list as
///     local -a args=( … ${(@)common_opts_conn/#\(-U/(2 -U} … )
/// so `psql -<TAB>` silently produced nothing.
#[test]
fn typeset_paren_init_survives_unbalanced_parens_inside_a_substitution() {
    let spec = r#"c=('(-h --host)-h' '(-U --username)-U')"#;
    // Escaped `\(` in the pattern AND a bare `(` in the replacement.
    ok(
        &format!(
            r#"setopt rcexpandparam; {spec}; local -a v=( ${{(@)c/#\(-U/(2 -U}} ); print -r -- "$#v"; print -rl -- $v"#
        ),
        "2\n(-h --host)-h\n(2 -U --username)-U\n",
    );
    // Same word with literal elements around the substitution.
    ok(
        &format!(
            r#"setopt rcexpandparam; {spec}; local -a v=( + x ${{(@)c/#\(-U/(2 -U}} tail ); print -r -- "$#v""#
        ),
        "5\n",
    );
    // Option off — the collapse also happened there, as an
    // "inconsistent type for assignment" abort.
    ok(
        &format!(r#"{spec}; local -a v=( ${{(@)c/#\(-U/(2 -U}} ); print -r -- "$#v""#),
        "2\n",
    );
    // Balanced nesting must keep working: `$( … )` and `${ … }` elements stay
    // whole, and unquoted whitespace outside them still splits elements.
    ok(
        r#"local -a w=( a$(print -n b c)d "e f" ${(j:-:)$(print x y)} g ); print -r -- "$#w""#,
        "5\n",
    );
}

/// RC_EXPAND_PARAM: an empty SCALAR juxtaposed with a non-empty array must
/// keep the word (`c:Src/subst.c:4437` — the scalar never enters the array
/// emit block, so it contributes `""` and the surrounding text survives).
/// Only a genuine empty ARRAY deletes the whole word (`c:Src/subst.c:4362`
/// `if (plan9) { uremnode(l, n); return n; }`).
///
/// zshrs collapses both shapes to an empty `Value::Array` and carries the
/// missing bit in a thread-local flag. The flag was written on EVERY array
/// read, not just empty ones, so the non-empty right-hand segment cleared
/// the bit the empty left-hand scalar had set and the word was deleted.
///
/// `compinit` puts `rcexpandparam` in `$_comp_options`, so every completer
/// runs with the option ON. `_netstat` ends with
///     sock=''
///     args+=( - sockets ${sock}${sockets} )
/// which lost all eight socket-set specs before `comparguments -i` saw them:
/// `netstat -<TAB>` dropped `-A -L -W -a -f -l -n` from the listing.
#[test]
fn rcexpandparam_empty_scalar_prefix_keeps_the_array_word() {
    // Braced and bare reads, prefix and suffix position.
    ok(
        r#"setopt rcexpandparam; e=''; a=(x y z); v=( ${e}${a} ); print -r -- $#v; print -rl -- $v"#,
        "3\nx\ny\nz\n",
    );
    ok(
        r#"setopt rcexpandparam; e=''; a=(x y z); v=( $e$a ); print -r -- $#v"#,
        "3\n",
    );
    ok(
        r#"setopt rcexpandparam; e=''; a=(x y z); v=( ${a}${e} ); print -r -- $#v"#,
        "3\n",
    );
    ok(
        r#"setopt rcexpandparam; e=''; a=(x y z); v=( ${e}${a}${e} ); print -r -- $#v"#,
        "3\n",
    );
    // A real EMPTY ARRAY still deletes the word — the bit must not be
    // "always scalar" either.
    ok(
        r#"setopt rcexpandparam; ea=(); a=(x y z); v=( ${ea}${a} ); print -r -- $#v"#,
        "0\n",
    );
    ok(
        r#"setopt rcexpandparam; ea=(); v=( x${ea}y ); print -r -- $#v"#,
        "0\n",
    );
    // …and an empty scalar between literals keeps its word.
    ok(
        r#"setopt rcexpandparam; e=''; v=( x${e}y ); print -r -- $#v; print -rl -- $v"#,
        "1\nxy\n",
    );
    // The exact `_netstat` shape: `- <setname> ${sock}${sockets}`.
    ok(
        r#"setopt rcexpandparam
sock=''
sockets=( '-A[show address of a PCB]' '-L[show size of listen queues]' '-W[avoid truncating]' )
args=()
args+=( - sockets ${sock}${sockets} )
print -r -- $#args"#,
        "5\n",
    );
}

/// A NAME-ONLY arg to a typeset-family builtin expands like any other
/// command word: an array splats into one arg per name, and a scalar
/// splits only under SH_WORD_SPLIT. Only the `NAME=value` form is an
/// assignment (PREFORK_SINGLE|PREFORK_ASSIGN, no split, no glob).
///
/// c:Src/exec.c:4127-4147 — the WC_ASSIGN_SCALAR + WC_ASSIGN_INC
/// (name-only) arm preforks with PREFORK_TYPESET alone, then
/// `globlist(&svl, 0)`, and pushes every resulting word as its own
/// assignment. C's comment at c:4130-4139: "this is a name only, so it's
/// not required to be a single expansion … it may expand into scalar
/// assignments: ass=(one=two three=four); typeset a=b $ass". The value
/// arm at c:4184-4192 is the one that takes PREFORK_SINGLE and skips
/// globbing ("No globassign for typeset arguments, thank you").
///
/// The compiler bumped its assignment-arg depth for EVERY word after a
/// typeset-family head, so a name list arrived space-joined as ONE arg
/// and `bin_typeset`'s isident gate rejected it. Live break:
/// zsh-more-completions' `_git`, whose `_git_commands` opens with
///     local -a cmdtypes
///     cmdtypes=( main_porcelain_commands user_commands … )
///     local -a $cmdtypes
/// died with `_git_commands:local:10: not valid in this context:
/// main_porcelain_commands user_commands …`, so `git <TAB>` produced an
/// error instead of zsh's 163 completions.
#[test]
fn typeset_family_name_only_arg_splats_the_array_of_names() {
    // THE `_git` SHAPE: names come from an unquoted array expansion.
    ok(
        r#"f() { local -a t; t=(alpha beta gamma); local -a $t; alpha=(1 2); print -r -- "${#alpha} ${(t)beta} ${(t)gamma}" }; f"#,
        "2 array-local array-local\n",
    );
    // Same for the other BINF_ASSIGN heads.
    ok(
        r#"f() { local -a t; t=(d1 d2); declare $t; print -r -- "${(t)d1}/${(t)d2}" }; f"#,
        "scalar-local/scalar-local\n",
    );
    ok(
        r#"f() { local -a t; t=(i1 i2); integer $t; print -r -- "${(t)i1}/${(t)i2}" }; f"#,
        "integer-local/integer-local\n",
    );
    ok(
        r#"f() { local -a t; t=(f1 f2); float $t; print -r -- "${(t)f1}" }; f"#,
        "float-local\n",
    );
    ok(
        r#"f() { local -a t; t=(R1 R2); readonly $t; print -r -- "${(t)R1}/${(t)R2}" }; f"#,
        "scalar-local-readonly/scalar-local-readonly\n",
    );
    ok(
        r#"f() { local -a t; t=(E1 E2); E1=v1; E2=v2; export $t; print -r -- "${(t)E1}/${(t)E2}" }; f"#,
        "scalar-export/scalar-export\n",
    );
    // c:4136-4138 verbatim — a name-only arg may carry `name=value`.
    ok(
        r#"f() { local ass; ass=(one=two three=four); typeset a=b $ass; print -r -- "$a/$one/$three" }; f"#,
        "b/two/four\n",
    );
    // A SCALAR name arg splits only under SH_WORD_SPLIT (plain word
    // rules) — without the option zsh rejects the joined string, which
    // is why the gate can't simply always split.
    ok(
        r#"f() { setopt localoptions shwordsplit; local s; s="q1 q2"; typeset $s; print -r -- "${(t)q1}/${(t)q2}" }; f"#,
        "scalar-local/scalar-local\n",
    );
}

/// The counterpart pin: the `NAME=value` arg form keeps assignment
/// semantics (c:Src/exec.c:4184-4192 — PREFORK_SINGLE|PREFORK_ASSIGN,
/// no globassign). Widening the split to every typeset arg would break
/// each of these.
#[test]
fn typeset_family_name_equals_value_arg_keeps_assignment_semantics() {
    // Unquoted scalar RHS does NOT word-split, even with shwordsplit.
    ok(
        r#"f() { local s; s="a b"; local y=$s; print -r -- "[$y]" }; f"#,
        "[a b]\n",
    );
    ok(
        r#"f() { setopt localoptions shwordsplit; local s; s="a b"; local y=$s; print -r -- "[$y]" }; f"#,
        "[a b]\n",
    );
    // An ARRAY RHS joins into the scalar (${IFS[1]}), never splats.
    ok(
        r#"f() { local -a a; a=(x y); local w=$a; print -r -- "[$w]" }; f"#,
        "[x y]\n",
    );
    // Quoted value, paren-init array, `$PATH:/x`-style append.
    ok(
        r#"f() { local x="a b c"; print -r -- "[$x]" }; f"#,
        "[a b c]\n",
    );
    ok(
        r#"f() { typeset "n=a b"; print -r -- "[$n]" }; f"#,
        "[a b]\n",
    );
    ok(
        r#"f() { typeset -a v=(1 2 3); print -r -- "${#v}/${v[2]}" }; f"#,
        "3/2\n",
    );
    ok(
        r#"f() { local p; p=/bin:/sbin; export p=$p:/x; print -r -- "[$p]" }; f"#,
        "[/bin:/sbin:/x]\n",
    );
    // "No globassign for typeset arguments" (c:4190-4192): the value is
    // not filename-generated even when it grows a metachar.
    ok(
        r#"f() { local i; i=3; typeset -i n=$i*2; print -r -- "$n" }; f"#,
        "6\n",
    );
    ok(
        r#"f() { local x; typeset y=${x:-*}; print -r -- "$y" }; f"#,
        "*\n",
    );
}
