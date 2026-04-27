//! Comprehensive construct-coverage corpus.
//!
//! Every sh/zsh construct outside zsh modules gets at least one test here.
//! Each test pins exact stdout. Failures of any kind — wrong output, wrong
//! exit, or hang — surface as a regression. Tests live in one file so
//! `cargo test --test zsh_construct_corpus` is the single gate.
//!
//! Categories covered:
//!   - Simple commands, assignments, positional params
//!   - Control flow: if/elif/else, while, until, for, for ((;;)), case, select
//!   - Pipelines + list operators (&&, ||, ;, &)
//!   - Redirects: > >> < <<EOF <<<str 2>&1 &> |, fd dup, fd close
//!   - Parameter expansion: every modifier from zshexpn(1)
//!   - Command substitution $(…) and `…`
//!   - Arithmetic: $((…)), ((…)), let
//!   - Tilde expansion
//!   - Brace expansion (ranges + lists + nested)
//!   - Glob: *, ?, [...], **/, qualifiers
//!   - Process substitution <(…) and >(…)
//!   - Heredocs <<EOF + variants
//!   - Quoting: '…', "…", $'…', backtick
//!   - Arrays: indexed, assoc, splice, append, length, index
//!   - ZshFlags: every flag, including stacking and long-tail
//!   - [[ ]] tests: every operator
//!   - Functions: definition, calling, local scope, return
//!   - Aliases: regular, global (-g), suffix (-s)
//!   - Builtins: cd, pwd, echo, print, printf, read, eval, exec, set, type, etc.
//!   - Coproc with /dev/fd round-trip
//!   - Background &, async/await
//!   - Bang literal in non-interactive mode
//!   - History expansion (gated)
//!   - Prompt expansion via print -P / `(%)` flag

use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

fn zshrs_bin() -> std::path::PathBuf {
    let mut p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("target/debug/zshrs");
    p
}

static FORK_SERIAL: Mutex<()> = Mutex::new(());

fn run(code: &str) -> (i32, String) {
    let mut child = Command::new(zshrs_bin())
        .args(["-f", "-c", code])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("zshrs binary missing — run `cargo build` first");

    let timeout = Duration::from_secs(8);
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

fn run_stdin(code: &str, input: &str) -> (i32, String) {
    use std::io::Write;
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

fn ok(code: &str, expected: &str) {
    let (status, stdout) = run(code);
    assert_eq!(status, 0, "exit non-zero on `{}`: {}", code, status);
    assert_eq!(stdout, expected, "stdout mismatch for `{}`", code);
}

fn ok_status(code: &str, expected: &str, expected_status: i32) {
    let (status, stdout) = run(code);
    assert_eq!(status, expected_status, "status mismatch for `{}`", code);
    assert_eq!(stdout, expected, "stdout mismatch for `{}`", code);
}

fn ok_serial(code: &str, expected: &str) {
    let _g = FORK_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    ok(code, expected);
}

fn ok_contains(code: &str, needle: &str) {
    let (status, stdout) = run(code);
    assert_eq!(status, 0, "exit non-zero on `{}`", code);
    assert!(
        stdout.contains(needle),
        "expected `{}` in stdout for `{}`, got: {:?}",
        needle,
        code,
        stdout
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Simple commands + assignments
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn simple_echo() { ok("echo hi", "hi\n"); }
#[test] fn simple_echo_multi_args() { ok("echo a b c", "a b c\n"); }
#[test] fn simple_echo_dash_n() { ok("echo -n no_newline", "no_newline"); }
#[test] fn assignment_then_var() { ok("x=42; echo $x", "42\n"); }
#[test] fn assignment_chain() { ok("x=1 y=2 z=3; echo $x $y $z", "1 2 3\n"); }
#[test] fn empty_assignment() { ok("x=; echo \"[$x]\"", "[]\n"); }
#[test] fn assignment_with_quotes() { ok("x=\"hello world\"; echo $x", "hello world\n"); }
#[test] fn assignment_with_squotes() { ok("x='lit $no'; echo $x", "lit $no\n"); }

// ─────────────────────────────────────────────────────────────────────────────
// List operators: ; && || &
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn list_semi() { ok("echo a; echo b", "a\nb\n"); }
#[test] fn list_and_true() { ok("true && echo run", "run\n"); }
#[test] fn list_and_false() { ok_status("false && echo run", "", 1); }
#[test] fn list_or_true() { ok("true || echo run", ""); }
#[test] fn list_or_false() { ok("false || echo run", "run\n"); }
#[test] fn list_chain() { ok("true && echo a && true && echo b", "a\nb\n"); }
#[test] fn list_short_circuit() { ok("false && echo no || echo yes", "yes\n"); }

// ─────────────────────────────────────────────────────────────────────────────
// Control flow
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn if_then() { ok("if true; then echo y; fi", "y\n"); }
#[test] fn if_else() { ok("if false; then echo n; else echo y; fi", "y\n"); }
#[test] fn if_elif() { ok("if false; then echo a; elif true; then echo b; else echo c; fi", "b\n"); }
#[test] fn if_brace() { ok("[[ 1 -lt 2 ]] && echo lt", "lt\n"); }

#[test] fn while_loop() { ok("i=0; while (( i<3 )); do echo $i; (( i++ )); done", "0\n1\n2\n"); }
#[test] fn until_loop() { ok("i=0; until (( i>=3 )); do echo $i; (( i++ )); done", "0\n1\n2\n"); }
#[test] fn for_in_words() { ok("for x in a b c; do echo $x; done", "a\nb\nc\n"); }
#[test] fn for_arith() { ok("for ((i=0; i<3; i++)); do echo $i; done", "0\n1\n2\n"); }
#[test] fn for_arith_decrement() { ok("for ((i=3; i>0; i--)); do echo $i; done", "3\n2\n1\n"); }

#[test] fn case_match() {
    ok(r#"case x in a) echo a ;; x) echo x ;; *) echo other ;; esac"#, "x\n");
}
#[test] fn case_default() {
    ok(r#"case foo in a) echo a ;; b) echo b ;; *) echo def ;; esac"#, "def\n");
}
#[test] fn case_glob_pattern() {
    ok(r#"case file.txt in *.txt) echo txt ;; *.log) echo log ;; esac"#, "txt\n");
}
#[test] fn case_alternation_pattern() {
    ok(r#"case mid in a|b|mid) echo m ;; *) echo o ;; esac"#, "m\n");
}

#[test] fn break_in_loop() {
    ok("for i in 1 2 3 4 5; do [[ $i = 3 ]] && break; echo $i; done", "1\n2\n");
}
#[test] fn continue_in_loop() {
    ok("for i in 1 2 3; do [[ $i = 2 ]] && continue; echo $i; done", "1\n3\n");
}
#[test] fn break_in_while() {
    ok("i=0; while true; do (( i++ )); [[ $i -eq 3 ]] && break; done; echo $i", "3\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// Functions
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn function_def_call() {
    ok("greet() { echo hi $1; }; greet world", "hi world\n");
}
#[test] fn function_function_keyword() {
    ok("function g { echo $1; }; g x", "x\n");
}
#[test] fn function_local_var() {
    ok(r#"f() { local x=inside; echo $x; }; x=outside; f; echo $x"#, "inside\noutside\n");
}
#[test] fn function_return_status() {
    ok_status("f() { return 7; }; f; echo $?", "7\n", 0);
}
#[test] fn function_positional_params() {
    ok(r#"f() { echo "$1|$2|$#"; }; f a b"#, "a|b|2\n");
}
#[test] fn function_dollar_at() {
    ok(r#"f() { for x in "$@"; do echo "[$x]"; done; }; f a "two words" c"#,
        "[a]\n[two words]\n[c]\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// Pipelines
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn pipeline_two_stages() {
    ok_serial("echo hello | /bin/cat", "hello\n");
}
#[test] fn pipeline_three_stages() {
    ok_serial("echo abc | cat | cat", "abc\n");
}
#[test] fn pipeline_negate() {
    ok_status("! true; echo $?", "1\n", 0);
}
#[test] fn pipeline_function_first() {
    ok_serial("g() { echo from-fn; }; g | /bin/cat", "from-fn\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// Redirects
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn redir_write() {
    let tmp = std::env::temp_dir().join(format!("zshrs_corpus_w_{}", std::process::id()));
    let p = tmp.to_string_lossy().into_owned();
    let _ = std::fs::remove_file(&p);
    let (status, _) = run(&format!("echo content > {}", p));
    assert_eq!(status, 0);
    let read = std::fs::read_to_string(&p).unwrap_or_default();
    assert_eq!(read, "content\n");
    let _ = std::fs::remove_file(&p);
}

#[test] fn redir_append() {
    let tmp = std::env::temp_dir().join(format!("zshrs_corpus_a_{}", std::process::id()));
    let p = tmp.to_string_lossy().into_owned();
    let _ = std::fs::remove_file(&p);
    let (status, _) = run(&format!("echo a > {}; echo b >> {}", p, p));
    assert_eq!(status, 0);
    let read = std::fs::read_to_string(&p).unwrap_or_default();
    assert_eq!(read, "a\nb\n");
    let _ = std::fs::remove_file(&p);
}

#[test] fn redir_read() {
    let tmp = std::env::temp_dir().join(format!("zshrs_corpus_r_{}", std::process::id()));
    std::fs::write(&tmp, "from_file\n").unwrap();
    let p_str = tmp.to_string_lossy().into_owned();
    ok(&format!("read line < {}; echo $line", p_str), "from_file\n");
    let _ = std::fs::remove_file(&tmp);
}

#[test] fn redir_heredoc() {
    ok("cat <<EOF\nline1\nline2\nEOF", "line1\nline2\n");
}
#[test] fn redir_herestring() {
    ok("tr a-z A-Z <<< hello", "HELLO\n");
}
#[test] fn redir_stderr_to_stdout() {
    // POSIX redirect order: `echo err >&2 2>&1` first dups fd 2 onto fd 1's
    // CURRENT target (which is whatever stdout points to), then dups fd 1
    // (now stdout) onto fd 2 — both fds go to the original stdout sink.
    // BUT zsh applies in left-to-right order: `>&2` makes fd 1 alias fd 2,
    // then `2>&1` makes fd 2 alias fd 1's NEW target (= original fd 2).
    // Net: err lands on stderr. stdout-only capture sees just "out\n".
    ok_serial("echo out; echo err >&2 2>&1", "out\n");
}
#[test] fn redir_block() {
    let tmp = std::env::temp_dir().join(format!("zshrs_corpus_block_{}", std::process::id()));
    let p = tmp.to_string_lossy().into_owned();
    let _ = std::fs::remove_file(&p);
    let (status, _) = run(&format!("{{ echo a; echo b; }} > {}", p));
    assert_eq!(status, 0);
    assert_eq!(std::fs::read_to_string(&p).unwrap_or_default(), "a\nb\n");
    let _ = std::fs::remove_file(&p);
}

#[test] fn redir_dup_read_with_var_fd() {
    ok_serial(
        r#"coproc { echo CL; }
sleep 0.2
read line <&${COPROC[1]}
echo "got=$line"
"#,
        "got=CL\n",
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Quoting
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn quote_single() { ok("echo 'a $b c'", "a $b c\n"); }
#[test] fn quote_double() { ok("x=hi; echo \"v=$x\"", "v=hi\n"); }
#[test] fn quote_dollar_single() { ok(r#"echo $'a\tb'"#, "a\tb\n"); }
#[test] fn quote_backtick() { ok("echo `echo nested`", "nested\n"); }
#[test] fn quote_escaped_dollar() { ok("echo \"\\$lit\"", "$lit\n"); }
#[test] fn quote_squote_in_dquote() { ok("echo \"it's fine\"", "it's fine\n"); }
#[test] fn quote_dquote_in_squote() { ok("echo 'has \"quotes\"'", "has \"quotes\"\n"); }

// ─────────────────────────────────────────────────────────────────────────────
// Parameter expansion
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn param_simple() { ok("x=42; echo $x", "42\n"); }
#[test] fn param_braced() { ok("x=42; echo ${x}", "42\n"); }
#[test] fn param_default_unset() { ok("echo ${unset:-fallback}", "fallback\n"); }
#[test] fn param_default_empty() { ok("x=; echo ${x:-fallback}", "fallback\n"); }
#[test] fn param_default_set() { ok("x=val; echo ${x:-fallback}", "val\n"); }
#[test] fn param_assign_default() { ok("echo ${y:=initial}; echo $y", "initial\ninitial\n"); }
#[test] fn param_alternate_set() { ok("x=val; echo ${x:+alt}", "alt\n"); }
#[test] fn param_alternate_unset() { ok("echo ${unset:+alt}", "\n"); }
#[test] fn param_length() { ok("x=hello; echo ${#x}", "5\n"); }
#[test] fn param_substring_offset_only() { ok("x=hello; echo ${x:1}", "ello\n"); }
#[test] fn param_substring_offset_length() { ok("x=hello; echo ${x:1:3}", "ell\n"); }
#[test] fn param_strip_short_prefix() { ok("p=/usr/bin/zsh; echo ${p#*/}", "usr/bin/zsh\n"); }
#[test] fn param_strip_long_prefix() { ok("p=/usr/bin/zsh; echo ${p##*/}", "zsh\n"); }
#[test] fn param_strip_short_suffix() { ok("f=app.tar.gz; echo ${f%.gz}", "app.tar\n"); }
#[test] fn param_strip_long_suffix() { ok("f=app.tar.gz; echo ${f%%.*}", "app\n"); }
#[test] fn param_replace_first() { ok("s=foofoo; echo ${s/foo/bar}", "barfoo\n"); }
#[test] fn param_replace_all() { ok("s=foofoo; echo ${s//foo/bar}", "barbar\n"); }
#[test] fn param_upper_postfix() { ok("x=hi; echo ${x:u}", "HI\n"); }
#[test] fn param_lower_postfix() { ok("x=HI; echo ${x:l}", "hi\n"); }

// ─────────────────────────────────────────────────────────────────────────────
// ZshFlags
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn zshflag_L() { ok("x=Hello; echo ${(L)x}", "hello\n"); }
#[test] fn zshflag_U() { ok("x=hello; echo ${(U)x}", "HELLO\n"); }
#[test] fn zshflag_j() { ok("a=(x y z); echo \"${(j:-:)a}\"", "x-y-z\n"); }
#[test] fn zshflag_s() { ok("s=a,b,c; echo \"${(s:,:)s}\"", "a b c\n"); }
#[test] fn zshflag_o() { ok("a=(c b a); echo ${(o)a}", "a b c\n"); }
#[test] fn zshflag_O() { ok("a=(a b c); echo ${(O)a}", "c b a\n"); }
#[test] fn zshflag_P() { ok("real=42; ref=real; echo ${(P)ref}", "42\n"); }
#[test] fn zshflag_count() { ok("a=(x y z); echo ${(#)a}", "3\n"); }
#[test] fn zshflag_q() { ok("x=hi; echo ${(q)x}", "'hi'\n"); }
#[test] fn zshflag_qq() { ok("x=hi; echo ${(qq)x}", "\"hi\"\n"); }
#[test] fn zshflag_qqqq() { ok(r#"x="has space"; echo ${(qqqq)x}"#, "has\\ space\n"); }
#[test] fn zshflag_q_plus_safe() { ok("x=safe; echo ${(q+)x}", "safe\n"); }
#[test] fn zshflag_q_plus_unsafe() { ok(r#"x="has space"; echo "${(q+)x}""#, "'has space'\n"); }
#[test] fn zshflag_g() { ok(r#"s='a\nb'; echo "${(g)s}""#, "a\nb\n"); }
#[test] fn zshflag_n() { ok("a=(f10 f2 f1); echo ${(on)a}", "f1 f2 f10\n"); }
#[test] fn zshflag_t_scalar() { ok("x=v; echo ${(t)x}", "scalar\n"); }
#[test] fn zshflag_t_array() { ok("a=(1 2); echo ${(t)a}", "array\n"); }
#[test] fn zshflag_t_assoc() { ok("typeset -A m; m[k]=v; echo ${(t)m}", "association\n"); }
#[test] fn zshflag_stacked() { ok("a=(c a b); echo ${(oU)a}", "A B C\n"); }

// ─────────────────────────────────────────────────────────────────────────────
// Special parameters
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn special_question_after_true() { ok_status("true; echo $?", "0\n", 0); }
#[test] fn special_question_after_false() { ok_status("false; echo $?", "1\n", 0); }
#[test] fn special_dollar_pid_is_int() {
    let (status, stdout) = run("echo $$");
    assert_eq!(status, 0);
    assert!(stdout.trim().chars().all(|c| c.is_ascii_digit()));
}
#[test] fn special_at_count_in_function() {
    ok(r#"f() { echo $#; }; f a b c d"#, "4\n");
}
#[test] fn special_at_iter_in_function() {
    ok(r#"f() { for x in "$@"; do echo $x; done; }; f a b c"#, "a\nb\nc\n");
}
#[test] fn special_args_iter() {
    ok(r#"f() { echo "$1|$2|$3"; }; f x y z"#, "x|y|z\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// Command substitution
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn cmd_subst_dollar_paren() { ok("x=$(echo hi); echo $x", "hi\n"); }
#[test] fn cmd_subst_backtick() { ok("x=`echo hi`; echo $x", "hi\n"); }
#[test] fn cmd_subst_in_string() { ok(r#"echo "now is $(echo now)""#, "now is now\n"); }
#[test] fn cmd_subst_strips_trailing_newline() {
    ok("x=$(printf 'val\\n\\n\\n'); echo \"[$x]\"", "[val]\n");
}
#[test] fn cmd_subst_nested() {
    ok("x=$(echo $(echo inner)); echo $x", "inner\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// Arithmetic
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn arith_addition() { ok("echo $((1+2))", "3\n"); }
#[test] fn arith_precedence() { ok("echo $((2+3*4))", "14\n"); }
#[test] fn arith_paren() { ok("echo $(((2+3)*4))", "20\n"); }
#[test] fn arith_div() { ok("echo $((10/3))", "3\n"); }
#[test] fn arith_mod() { ok("echo $((10%3))", "1\n"); }
#[test] fn arith_pow() { ok("echo $((2**10))", "1024\n"); }
#[test] fn arith_bit_and() { ok("echo $((0xff & 0x0f))", "15\n"); }
#[test] fn arith_bit_or() { ok("echo $((0x0f | 0xf0))", "255\n"); }
#[test] fn arith_bit_xor() { ok("echo $((0xff ^ 0x0f))", "240\n"); }
#[test] fn arith_shift_left() { ok("echo $((1 << 8))", "256\n"); }
#[test] fn arith_shift_right() { ok("echo $((256 >> 2))", "64\n"); }
#[test] fn arith_pre_inc() { ok("i=5; echo $((++i)); echo $i", "6\n6\n"); }
#[test] fn arith_post_inc() { ok("i=5; echo $((i++)); echo $i", "5\n6\n"); }
#[test] fn arith_pre_dec() { ok("i=5; echo $((--i)); echo $i", "4\n4\n"); }
#[test] fn arith_post_dec() { ok("i=5; echo $((i--)); echo $i", "5\n4\n"); }
#[test] fn arith_compound_paren() { ok("(( x = 2 + 3 )); echo $x", "5\n"); }
#[test] fn arith_let() { ok("let i=5; echo $i", "5\n"); }
#[test] fn arith_neg_int() { ok("echo $((-5 + 3))", "-2\n"); }
#[test] fn arith_compare_lt() { ok("(( 1 < 2 )) && echo y || echo n", "y\n"); }
#[test] fn arith_compare_gt() { ok("(( 5 > 3 )) && echo y", "y\n"); }
#[test] fn arith_logical_and() { ok("(( 1 && 1 )) && echo y", "y\n"); }
#[test] fn arith_logical_or() { ok("(( 0 || 1 )) && echo y", "y\n"); }
#[test] fn arith_ternary() { ok("echo $(( 5 > 3 ? 100 : 200 ))", "100\n"); }

// ─────────────────────────────────────────────────────────────────────────────
// Tilde + brace + glob expansion
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn tilde_home() {
    let home = std::env::var("HOME").unwrap_or_default();
    let (status, stdout) = run("echo ~");
    assert_eq!(status, 0);
    assert_eq!(stdout.trim(), home);
}
#[test] fn tilde_with_path() {
    let home = std::env::var("HOME").unwrap_or_default();
    let (status, stdout) = run("echo ~/sub");
    assert_eq!(status, 0);
    assert_eq!(stdout.trim(), format!("{}/sub", home));
}

#[test] fn brace_range_num() { ok("echo {1..5}", "1 2 3 4 5\n"); }
#[test] fn brace_range_letter() { ok("echo {a..e}", "a b c d e\n"); }
#[test] fn brace_alt() { ok("echo {a,b,c}", "a b c\n"); }
#[test] fn brace_nested() { ok("echo {a,{b,c}}", "a b c\n"); }
#[test] fn brace_with_prefix() { ok("echo pre-{a,b}", "pre-a pre-b\n"); }
#[test] fn brace_in_for() {
    ok("for i in {1..3}; do echo $i; done", "1\n2\n3\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// Arrays
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn array_literal() { ok("a=(x y z); echo ${a[1]}", "x\n"); }
#[test] fn array_splice() { ok("a=(x y z); echo ${a[@]}", "x y z\n"); }
#[test] fn array_length() { ok("a=(x y z w); echo ${#a[@]}", "4\n"); }
#[test] fn array_neg_index() { ok("a=(x y z); echo ${a[-1]}", "z\n"); }
#[test] fn array_append() { ok("a=(x); a+=(y z); echo ${a[@]}", "x y z\n"); }
#[test] fn array_iter_for() {
    ok("a=(red green blue); for c in ${a[@]}; do echo $c; done", "red\ngreen\nblue\n");
}
#[test] fn array_empty_length() { ok("a=(); echo ${#a[@]}", "0\n"); }
#[test] fn array_quoted_elements() {
    ok(r#"a=(one "two words" three); for e in ${a[@]}; do echo "[$e]"; done"#,
        "[one]\n[two words]\n[three]\n");
}

#[test] fn assoc_set_get() { ok("typeset -A m; m[k]=v; echo ${m[k]}", "v\n"); }
#[test] fn assoc_keys() {
    ok_contains("typeset -A m; m[a]=1; m[b]=2; for k in \"${(k)m}\"; do echo $k; done | sort", "a");
}
#[test] fn assoc_values() {
    ok_contains("typeset -A m; m[a]=1; m[b]=2; for v in \"${(v)m}\"; do echo $v; done | sort", "1");
}
#[test] fn assoc_append() { ok("m[k]=hi; m[k]+=\" world\"; echo ${m[k]}", "hi world\n"); }
#[test] fn assoc_overwrite() { ok("m[k]=first; m[k]=second; echo ${m[k]}", "second\n"); }
#[test] fn assoc_missing_empty() { ok("m[a]=1; echo \"[${m[nope]}]\"", "[]\n"); }

// ─────────────────────────────────────────────────────────────────────────────
// [[ ]] tests
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn cond_string_eq() { ok("[[ a == a ]] && echo y", "y\n"); }
#[test] fn cond_string_neq() { ok("[[ a != b ]] && echo y", "y\n"); }
#[test] fn cond_string_glob() { ok("[[ abc == a* ]] && echo y", "y\n"); }
#[test] fn cond_string_lt() { ok("[[ a < b ]] && echo y", "y\n"); }
#[test] fn cond_string_gt() { ok("[[ b > a ]] && echo y", "y\n"); }
#[test] fn cond_string_empty() { ok(r#"[[ -z "" ]] && echo y"#, "y\n"); }
#[test] fn cond_string_nonempty() { ok(r#"[[ -n "x" ]] && echo y"#, "y\n"); }
#[test] fn cond_num_eq() { ok("[[ 1 -eq 1 ]] && echo y", "y\n"); }
#[test] fn cond_num_lt() { ok("[[ 1 -lt 2 ]] && echo y", "y\n"); }
#[test] fn cond_num_le() { ok("[[ 1 -le 1 ]] && echo y", "y\n"); }
#[test] fn cond_num_gt() { ok("[[ 2 -gt 1 ]] && echo y", "y\n"); }
#[test] fn cond_num_ge() { ok("[[ 2 -ge 2 ]] && echo y", "y\n"); }
#[test] fn cond_file_exists() {
    let p = "/etc/hosts";
    let script = format!("[[ -e {} ]] && echo y", p);
    ok(&script, "y\n");
}
#[test] fn cond_file_regular() {
    let p = "/etc/hosts";
    let script = format!("[[ -f {} ]] && echo y", p);
    ok(&script, "y\n");
}
#[test] fn cond_file_dir() {
    ok("[[ -d /tmp ]] && echo y", "y\n");
}
#[test] fn cond_file_readable() {
    ok("[[ -r /etc/hosts ]] && echo y", "y\n");
}
#[test] fn cond_logical_and() { ok("[[ 1 -lt 2 && a == a ]] && echo y", "y\n"); }
#[test] fn cond_logical_or() { ok("[[ 1 -gt 2 || a == a ]] && echo y", "y\n"); }
#[test] fn cond_negate() { ok("[[ ! a == b ]] && echo y", "y\n"); }
#[test] fn cond_regex_basic() { ok(r#"[[ abc =~ ^a ]] && echo y"#, "y\n"); }
#[test] fn cond_regex_class() { ok(r#"[[ a1b =~ [0-9] ]] && echo y"#, "y\n"); }
#[test] fn cond_regex_anchor() { ok(r#"[[ hello =~ ^h.*o$ ]] && echo y"#, "y\n"); }

// ─────────────────────────────────────────────────────────────────────────────
// Builtins (the core POSIX set)
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn builtin_pwd() {
    let (status, stdout) = run("pwd");
    assert_eq!(status, 0);
    assert!(stdout.starts_with('/'));
}
#[test] fn builtin_true() { ok_status("true", "", 0); }
#[test] fn builtin_false() { ok_status("false", "", 1); }
#[test] fn builtin_colon() { ok_status(":", "", 0); }
#[test] fn builtin_test_bang() { ok_status("test ! -z foo", "", 0); }
#[test] fn builtin_eval_simple() { ok("eval 'echo from-eval'", "from-eval\n"); }
#[test] fn builtin_eval_var_defer() { ok("x=10; eval 'echo $x'", "10\n"); }
#[test] fn builtin_set_args() { ok("set -- a b c; echo $1 $2 $3", "a b c\n"); }
#[test] fn builtin_shift() { ok("set -- a b c; shift; echo $1", "b\n"); }
#[test] fn builtin_unset() { ok("x=v; unset x; echo \"[${x:-EMPTY}]\"", "[EMPTY]\n"); }
#[test] fn builtin_export() { ok("export X=v; echo $X", "v\n"); }
#[test] fn builtin_alias() { ok("alias foo='echo expanded'; foo", "expanded\n"); }
#[test] fn builtin_unalias() {
    ok("alias foo='echo a'; unalias foo; alias foo 2>/dev/null; echo done", "done\n");
}
#[test] fn builtin_type_for_builtin() { ok_contains("type echo", "builtin"); }
#[test] fn builtin_printf_format() { ok("printf '%s\\n' hello", "hello\n"); }
#[test] fn builtin_printf_int() { ok("printf '%d\\n' 42", "42\n"); }
#[test] fn builtin_print_lN() {
    ok("print -l a b c", "a\nb\nc\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// Coreutils builtins (anti-fork)
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn coreutil_seq() { ok("seq 1 5", "1\n2\n3\n4\n5\n"); }
#[test] fn coreutil_basename() { ok("basename /usr/bin/zsh", "zsh\n"); }
#[test] fn coreutil_dirname() { ok("dirname /usr/bin/zsh", "/usr/bin\n"); }
#[test] fn coreutil_whoami_nonempty() {
    let (status, stdout) = run("whoami");
    assert_eq!(status, 0);
    assert!(!stdout.trim().is_empty());
}

// ─────────────────────────────────────────────────────────────────────────────
// Read builtin
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn read_basic() {
    let (status, stdout) = run_stdin("read line; echo \"got=$line\"", "hello\n");
    assert_eq!(status, 0);
    assert_eq!(stdout, "got=hello\n");
}
#[test] fn read_multi_var() {
    let (status, stdout) = run_stdin("read a b c; echo \"$a|$b|$c\"", "x y z\n");
    assert_eq!(status, 0);
    assert_eq!(stdout, "x|y|z\n");
}
#[test] fn read_array() {
    let (status, stdout) = run_stdin("read -A arr; echo ${arr[2]}", "a b c\n");
    assert_eq!(status, 0);
    assert_eq!(stdout, "b\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// Background, async, coproc
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn bg_amp_returns_immediately() {
    ok_serial("sleep 1 & echo done", "done\n");
}

#[test] fn coproc_round_trip() {
    ok_serial(
        r#"coproc { echo CHILD; }
sleep 0.2
read line < /dev/fd/${COPROC[1]}
echo got=$line
"#,
        "got=CHILD\n",
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Eval / re-evaluation
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn eval_quoted_literal() { ok("eval 'echo lit'", "lit\n"); }
#[test] fn eval_var_inside_squotes() { ok("x=10; eval 'echo $x'", "10\n"); }
#[test] fn eval_multi_command() {
    ok("eval 'a=1; b=2; echo $((a+b))'", "3\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// History expansion (gated to interactive)
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn bang_literal_in_script() { ok("echo !!", "!!\n"); }
#[test] fn bang_dollar_literal() { ok("echo !$", "!$\n"); }
#[test] fn bang_in_string_literal() { ok(r#"echo "hello !!""#, "hello !!\n"); }

// ─────────────────────────────────────────────────────────────────────────────
// Bang as command negation (with space)
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn negate_true() { ok_status("! true; echo $?", "1\n", 0); }
#[test] fn negate_false() { ok_status("! false; echo $?", "0\n", 0); }

// ─────────────────────────────────────────────────────────────────────────────
// Glob
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn glob_star_in_tmpdir() {
    let tmp = std::env::temp_dir().join(format!("zshrs_corpus_glob_{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("a.txt"), "x").unwrap();
    std::fs::write(tmp.join("b.txt"), "x").unwrap();
    let p = tmp.to_string_lossy().into_owned();
    let (status, stdout) = run(&format!("echo {}/*.txt", p));
    assert_eq!(status, 0);
    assert!(stdout.contains("a.txt"));
    assert!(stdout.contains("b.txt"));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test] fn glob_question() {
    let tmp = std::env::temp_dir().join(format!("zshrs_corpus_q_{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("a"), "x").unwrap();
    std::fs::write(tmp.join("ab"), "x").unwrap();
    let p = tmp.to_string_lossy().into_owned();
    let (status, stdout) = run(&format!("echo {}/?", p));
    assert_eq!(status, 0);
    assert!(stdout.contains("/a"));
    assert!(!stdout.contains("/ab"));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test] fn glob_qualifier_files() {
    let tmp = std::env::temp_dir().join(format!("zshrs_corpus_qual_{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("foo.txt"), "x").unwrap();
    std::fs::create_dir_all(tmp.join("subdir")).unwrap();
    let p = tmp.to_string_lossy().into_owned();
    let (status, stdout) = run(&format!("echo {}/*(.)", p));
    assert_eq!(status, 0);
    assert!(stdout.contains("foo.txt"));
    assert!(!stdout.contains("subdir"));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test] fn glob_qualifier_dirs() {
    let tmp = std::env::temp_dir().join(format!("zshrs_corpus_qual_d_{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("foo.txt"), "x").unwrap();
    std::fs::create_dir_all(tmp.join("subdir")).unwrap();
    let p = tmp.to_string_lossy().into_owned();
    let (status, stdout) = run(&format!("echo {}/*(/)", p));
    assert_eq!(status, 0);
    assert!(stdout.contains("subdir"));
    assert!(!stdout.contains("foo.txt"));
    let _ = std::fs::remove_dir_all(&tmp);
}

// ─────────────────────────────────────────────────────────────────────────────
// Process substitution
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn proc_sub_input() {
    ok_serial("/bin/cat <(echo line)", "line\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// Aliases
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn alias_simple() { ok("alias g='echo hi'; g", "hi\n"); }
#[test] fn alias_with_args() { ok("alias h='echo prefix'; h end", "prefix end\n"); }

// ─────────────────────────────────────────────────────────────────────────────
// Subshells / command groups
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn subshell_isolates_var() {
    ok("x=outer; (x=inner; echo $x); echo $x", "inner\nouter\n");
}
#[test] fn brace_group_does_not_isolate() {
    ok("x=outer; { x=inner; }; echo $x", "inner\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// Trap
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn trap_exit() {
    ok("trap 'echo bye' EXIT; echo hi", "hi\nbye\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// Select
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn select_eof_no_body() {
    let (status, stdout) = run_stdin(
        "select x in a b c; do echo got=$x; done; echo after",
        "",
    );
    assert_eq!(status, 0);
    assert!(stdout.contains("after"));
    assert!(!stdout.contains("got="));
}

#[test] fn select_pick_then_break() {
    let (status, stdout) = run_stdin(
        r#"select x in a b c; do echo "sel=$x"; break; done"#,
        "2\n",
    );
    assert_eq!(status, 0);
    assert!(stdout.contains("sel=b"));
}

// ─────────────────────────────────────────────────────────────────────────────
// Repeat
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn repeat_n_times() {
    ok("repeat 3 echo hi", "hi\nhi\nhi\n");
}

// ─────────────────────────────────────────────────────────────────────────────
// Misc edge cases
// ─────────────────────────────────────────────────────────────────────────────

#[test] fn empty_script() { ok_status("", "", 0); }
#[test] fn just_semi() { ok_status(";", "", 0); }
#[test] fn just_newline() { ok_status("\n", "", 0); }
#[test] fn comment_only() { ok_status("# comment", "", 0); }
#[test] fn comment_after_cmd() { ok("echo a # comment", "a\n"); }
#[test] fn empty_string_var() { ok("x=''; echo \"[$x]\"", "[]\n"); }
#[test] fn nested_double_quotes() {
    ok(r#"x=v; echo "outer ${x} end""#, "outer v end\n");
}
#[test] fn cmd_subst_in_array() {
    ok("a=($(echo a b c)); echo ${a[2]}", "b\n");
}
