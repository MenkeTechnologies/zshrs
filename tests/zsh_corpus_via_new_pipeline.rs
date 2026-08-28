//! Run the full construct corpus through the lex+parse +
//! ZshCompiler pipeline. Same scripts as `zsh_construct_corpus.rs`.
//! Failures here are the punch list for `compile_zsh.rs`.

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
        .expect("zshrs binary missing");
    let timeout = Duration::from_secs(5);
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(s)) => {
                let out = child.wait_with_output().unwrap();
                return (
                    s.code().unwrap_or(-1),
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

fn ok(code: &str, expected: &str) {
    let (status, out) = run(code);
    assert_eq!(status, 0, "exit non-zero on `{}`: status={}", code, status);
    assert_eq!(out, expected, "mismatch for `{}`", code);
}

fn ok_status(code: &str, expected: &str, expected_status: i32) {
    let (status, out) = run(code);
    assert_eq!(status, expected_status, "status for `{}`", code);
    assert_eq!(out, expected, "stdout for `{}`", code);
}

fn ok_serial(code: &str, expected: &str) {
    let _g = FORK_SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    ok(code, expected);
}

// Simple commands
#[test]
fn np_echo() {
    ok("echo hi", "hi\n");
}
#[test]
fn np_echo_multi() {
    ok("echo a b c", "a b c\n");
}
#[test]
fn np_assign_then() {
    ok("x=42; echo $x", "42\n");
}
#[test]
fn np_assign_chain() {
    ok("x=1 y=2 z=3; echo $x $y $z", "1 2 3\n");
}
#[test]
fn np_squoted() {
    ok("echo 'a $b c'", "a $b c\n");
}
#[test]
fn np_dquoted() {
    ok("x=hi; echo \"v=$x\"", "v=hi\n");
}

// Lists
#[test]
fn np_semi() {
    ok("echo a; echo b", "a\nb\n");
}
#[test]
fn np_and() {
    ok("true && echo y", "y\n");
}
#[test]
fn np_or() {
    ok("false || echo y", "y\n");
}
#[test]
fn np_chain() {
    ok("false && echo no || echo yes", "yes\n");
}

// Control flow
#[test]
fn np_if() {
    ok("if true; then echo y; fi", "y\n");
}
#[test]
fn np_if_else() {
    ok("if false; then echo a; else echo b; fi", "b\n");
}
#[test]
fn np_while() {
    ok(
        "i=0; while (( i<3 )); do echo $i; (( i++ )); done",
        "0\n1\n2\n",
    );
}
#[test]
fn np_for() {
    ok("for i in 1 2 3; do echo $i; done", "1\n2\n3\n");
}
#[test]
fn np_for_arith() {
    ok("for ((i=0; i<3; i++)); do echo $i; done", "0\n1\n2\n");
}
#[test]
fn np_case() {
    ok(
        "case x in a) echo a ;; x) echo x ;; *) echo o ;; esac",
        "x\n",
    );
}
#[test]
fn np_break() {
    ok(
        "for i in 1 2 3 4 5; do [[ $i = 3 ]] && break; echo $i; done",
        "1\n2\n",
    );
}
#[test]
fn np_continue() {
    ok(
        "for i in 1 2 3; do [[ $i = 2 ]] && continue; echo $i; done",
        "1\n3\n",
    );
}

// Functions (currently TODO)
// #[test] fn np_function() { ok("greet() { echo hi $1; }; greet world", "hi world\n"); }

// Cond
#[test]
fn np_cond_eq() {
    ok("[[ a == a ]] && echo y", "y\n");
}
#[test]
fn np_cond_neq() {
    ok("[[ a != b ]] && echo y", "y\n");
}
#[test]
fn np_cond_num() {
    ok("[[ 1 -lt 2 ]] && echo y", "y\n");
}
#[test]
fn np_cond_file() {
    ok("[[ -d /tmp ]] && echo y", "y\n");
}
#[test]
fn np_cond_negate() {
    ok("[[ ! a == b ]] && echo y", "y\n");
}
#[test]
fn np_regex() {
    ok(r#"[[ abc =~ ^a ]] && echo y"#, "y\n");
}

// Arithmetic
#[test]
fn np_arith_add() {
    ok("echo $((1+2))", "3\n");
}
#[test]
fn np_arith_assign() {
    ok("(( x = 5 )); echo $x", "5\n");
}
#[test]
fn np_arith_inc() {
    ok("i=5; (( i++ )); echo $i", "6\n");
}

// Pipelines
#[test]
fn np_pipe() {
    ok_serial("echo hello | /bin/cat", "hello\n");
}

// Subshells
#[test]
fn np_subshell_isolates() {
    ok("x=outer; (x=inner; echo $x); echo $x", "inner\nouter\n");
}

// Builtins
#[test]
fn np_true_false() {
    ok_status("true", "", 0);
    ok_status("false", "", 1);
}
#[test]
fn np_eval() {
    ok("eval 'echo from-eval'", "from-eval\n");
}
#[test]
fn np_eval_var_defer() {
    ok("x=10; eval 'echo $x'", "10\n");
}

// Quoting
#[test]
fn np_dollar_single() {
    let (status, out) = run(r#"echo $'a\tb'"#);
    assert_eq!(status, 0);
    assert_eq!(out, "a\tb\n");
}

// Redirects on simple commands
#[test]
fn np_redir_write_file() {
    let p = std::env::temp_dir().join(format!("zshrs_np_redir_w_{}", std::process::id()));
    let p_str = p.to_string_lossy().into_owned();
    let _ = std::fs::remove_file(&p);
    let (status, _) = run(&format!("echo content > {}", p_str));
    assert_eq!(status, 0);
    let read = std::fs::read_to_string(&p).unwrap_or_default();
    assert_eq!(read, "content\n");
    let _ = std::fs::remove_file(&p);
}

#[test]
fn np_redir_append() {
    let p = std::env::temp_dir().join(format!("zshrs_np_redir_a_{}", std::process::id()));
    let p_str = p.to_string_lossy().into_owned();
    let _ = std::fs::remove_file(&p);
    let (status, _) = run(&format!("echo a > {}; echo b >> {}", p_str, p_str));
    assert_eq!(status, 0);
    let read = std::fs::read_to_string(&p).unwrap_or_default();
    assert_eq!(read, "a\nb\n");
    let _ = std::fs::remove_file(&p);
}

#[test]
fn np_redir_read_from_file() {
    let p = std::env::temp_dir().join(format!("zshrs_np_redir_r_{}", std::process::id()));
    std::fs::write(&p, "from-file\n").unwrap();
    let p_str = p.to_string_lossy().into_owned();
    ok(
        &format!("read line < {}; echo \"got=$line\"", p_str),
        "got=from-file\n",
    );
    let _ = std::fs::remove_file(&p);
}

#[test]
fn np_heredoc() {
    ok("cat <<EOF\nline1\nline2\nEOF", "line1\nline2\n");
}

#[test]
fn np_herestring() {
    ok("tr a-z A-Z <<< hello", "HELLO\n");
}

// Functions
#[test]
fn np_function_def_call() {
    ok("greet() { echo hi $1; }; greet world", "hi world\n");
}

#[test]
fn np_function_function_keyword() {
    ok("function f { echo $1; }; f x", "x\n");
}

#[test]
fn np_function_dollar_at() {
    ok(
        r#"f() { for x in "$@"; do echo "[$x]"; done; }; f a "two w" c"#,
        "[a]\n[two w]\n[c]\n",
    );
}

#[test]
fn np_function_local_var() {
    ok(
        r#"f() { local x=inside; echo $x; }; x=outside; f; echo $x"#,
        "inside\noutside\n",
    );
}

#[test]
fn np_function_return_status() {
    ok_status("f() { return 7; }; f; echo $?", "7\n", 0);
}

// Arrays
#[test]
fn np_array_literal() {
    ok("arr=(a b c); echo ${arr[2]}", "b\n");
}
#[test]
fn np_array_all() {
    ok("arr=(a b c); echo ${arr[@]}", "a b c\n");
}
#[test]
fn np_array_length() {
    ok("arr=(a b c); echo ${#arr}", "3\n");
}
#[test]
fn np_array_append() {
    ok("arr=(a b); arr+=(c d); echo ${arr[3]}", "c\n");
}

// Parameter expansion
#[test]
fn np_param_default() {
    ok("echo ${unset:-fallback}", "fallback\n");
}
#[test]
fn np_param_default_set() {
    ok("v=have; echo ${v:-x}", "have\n");
}
#[test]
fn np_param_strip_prefix() {
    ok("p=a/b/c; echo ${p#*/}", "b/c\n");
}
#[test]
fn np_param_strip_suffix() {
    ok("p=a/b/c; echo ${p%/*}", "a/b\n");
}
#[test]
fn np_param_length() {
    ok("v=hello; echo ${#v}", "5\n");
}

// Brace expansion
#[test]
fn np_brace_alt() {
    ok("echo {a,b,c}", "a b c\n");
}
#[test]
fn np_brace_range() {
    ok("echo {1..3}", "1 2 3\n");
}

// Command substitution
#[test]
fn np_dollar_paren() {
    ok("x=$(echo nested); echo $x", "nested\n");
}
#[test]
fn np_backtick_sub() {
    ok("x=`echo nested`; echo $x", "nested\n");
}

// Arithmetic
#[test]
fn np_arith_paren() {
    ok("echo $((2*3+4))", "10\n");
}
#[test]
fn np_arith_compare() {
    ok("(( 5 > 3 )) && echo y", "y\n");
}
#[test]
fn np_arith_ternary() {
    ok("echo $((5>3?1:0))", "1\n");
}

// Herestring with var
#[test]
fn np_herestring_var() {
    ok(r#"x=hi; tr a-z A-Z <<< "$x""#, "HI\n");
}

// Globbing
#[test]
fn np_glob_star() {
    let dir = std::env::temp_dir().join(format!("zshrs_np_glob_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join("a.txt"), "x").unwrap();
    std::fs::write(dir.join("b.txt"), "x").unwrap();
    let out = run(&format!("cd {} && echo *.txt", dir.display()));
    assert_eq!(out.0, 0);
    assert_eq!(out.1, "a.txt b.txt\n");
    let _ = std::fs::remove_dir_all(&dir);
}

// Exit / status
#[test]
fn np_exit_code() {
    ok_status("(exit 5)", "", 5);
}
#[test]
fn np_dollar_question() {
    ok("false; echo $?", "1\n");
}

// Negation
#[test]
fn np_bang_negate() {
    ok("! true; echo $?", "1\n");
}
#[test]
fn np_bang_double() {
    ok("! false; echo $?", "0\n");
}

// Until loop
#[test]
fn np_until() {
    ok(
        "i=0; until (( i>=3 )); do echo $i; (( i++ )); done",
        "0\n1\n2\n",
    );
}

// Group / subshell mix
#[test]
fn np_brace_group() {
    ok("{ echo a; echo b; }", "a\nb\n");
}

// Var unset/empty
#[test]
fn np_unset_default() {
    ok("echo ${nope:-x}", "x\n");
}
#[test]
fn np_empty_string_split() {
    ok("v=''; echo \"[$v]\"", "[]\n");
}

// Pipelines + substitutions
#[test]
fn np_pipe_with_subst() {
    ok_serial("echo hello | tr a-z A-Z", "HELLO\n");
}
#[test]
fn np_pipe_three_stages() {
    ok_serial("echo c b a | tr ' ' '\\n' | sort", "a\nb\nc\n");
}
#[test]
fn np_cmdsub_in_arg() {
    ok("echo got=$(echo foo)", "got=foo\n");
}
#[test]
fn np_cmdsub_nested() {
    ok("echo $(echo $(echo deep))", "deep\n");
}

// Param expansion variants
#[test]
fn np_param_uppercase() {
    ok("v=hi; echo ${v:u}", "HI\n");
}
#[test]
fn np_param_lowercase() {
    ok("v=HI; echo ${v:l}", "hi\n");
}
#[test]
fn np_param_replace() {
    ok(r#"v=hello; echo ${v/l/L}"#, "heLlo\n");
}
#[test]
fn np_param_replace_all() {
    ok(r#"v=hello; echo ${v//l/L}"#, "heLLo\n");
}

// Nested control flow
#[test]
fn np_nested_if_for() {
    ok(
        "for i in 1 2 3; do if (( i == 2 )); then echo two; else echo $i; fi; done",
        "1\ntwo\n3\n",
    );
}
#[test]
fn np_case_in_for() {
    ok(
        "for x in a b c; do case $x in a) echo first;; *) echo $x;; esac; done",
        "first\nb\nc\n",
    );
}

// Function recursion + locals
#[test]
fn np_function_recursion() {
    ok(
        "f() { local n=$1; if (( n <= 1 )); then echo $n; else echo $n; f $((n-1)); fi; }; f 3",
        "3\n2\n1\n",
    );
}

// Conditional chains
#[test]
fn np_chain_negate() {
    ok("! false && echo y", "y\n");
}
#[test]
fn np_chain_long() {
    ok("true && true && true && echo done", "done\n");
}

// Test/condition forms
#[test]
fn np_cond_string_empty() {
    ok("[[ -z \"\" ]] && echo y", "y\n");
}
#[test]
fn np_cond_string_nonempty() {
    ok("[[ -n abc ]] && echo y", "y\n");
}
#[test]
fn np_cond_and() {
    ok("[[ 1 -lt 2 && 3 -gt 2 ]] && echo y", "y\n");
}
#[test]
fn np_cond_or() {
    ok("[[ 1 -gt 2 || 3 -gt 2 ]] && echo y", "y\n");
}

// Read builtin + redirect
#[test]
fn np_read_from_pipe() {
    ok_serial("echo word | { read line; echo got=$line; }", "got=word\n");
}

// Variable scoping
#[test]
fn np_export_var() {
    ok("export V=ex; echo $V", "ex\n");
}
#[test]
fn np_typeset_int() {
    ok("typeset -i n=10; echo $n", "10\n");
}

// Stderr / fd redirects
#[test]
fn np_stderr_to_stdout() {
    ok("{ echo to_err >&2; } 2>&1", "to_err\n");
}
#[test]
fn np_redir_to_devnull() {
    ok("echo gone >/dev/null; echo seen", "seen\n");
}

// Multi-assignment + quoting
#[test]
fn np_multi_assign() {
    ok("a=1 b=2; echo $a $b", "1 2\n");
}
#[test]
fn np_quoted_var_in_assign() {
    ok(r#"a="hi mom"; echo "$a""#, "hi mom\n");
}

// Numeric range loops
#[test]
fn np_for_range() {
    ok("for i in {1..3}; do echo $i; done", "1\n2\n3\n");
}

// Pipeline with builtin
#[test]
fn np_pipe_to_wc() {
    ok_serial("printf 'a\\nb\\nc\\n' | wc -l | tr -d ' '", "3\n");
}

// String + arithmetic mix
#[test]
fn np_str_plus_arith() {
    ok("n=$((2+3)); echo n=$n", "n=5\n");
}

// Nested function call
#[test]
fn np_function_calls_function() {
    ok(
        "g() { echo from-g; }; f() { g; echo from-f; }; f",
        "from-g\nfrom-f\n",
    );
}

// Iterate over command substitution result
#[test]
fn np_for_over_cmdsub() {
    ok(r#"for w in $(echo a b c); do echo $w; done"#, "a\nb\nc\n");
}

// Conditional with quoted expansion
#[test]
fn np_cond_var_expand() {
    ok(r#"v=hi; [[ "$v" == hi ]] && echo y"#, "y\n");
}

// Aliases
#[test]
fn np_alias_simple() {
    // NOT `alias g=...; g` — an alias does not apply to a use in the SAME
    // parse unit, because `checkalias` fires from the lexer
    // (c:Src/lex.c:1909) and the whole `-c` argument is lexed before any of
    // it runs. Real zsh agrees, so the old expectation was asserting
    // behaviour zsh does not have:
    //   $ zsh   -fc "alias g='echo greeted'; g"  -> zsh:1: command not found: g  (127)
    //   $ zshrs --zsh -f -c "<same>"             -> identical
    // A newline instead of `;` changes nothing (both say line 2, 127).
    // `eval` re-lexes at RUNTIME, so it is the shape that actually
    // exercises alias expansion; verified `greeted` from both shells.
    ok("alias g='echo greeted'; eval g", "greeted\n");
}

// Compound assignment in for
#[test]
fn np_for_with_assign() {
    ok(
        "tot=0; for i in 1 2 3; do (( tot+=i )); done; echo $tot",
        "6\n",
    );
}

// Native fast-path coverage for bare $NAME forms
#[test]
fn np_var_basic() {
    ok("x=hello; echo $x", "hello\n");
}
#[test]
fn np_var_pos() {
    ok("set -- a b c; echo $1 $2 $3", "a b c\n");
}
#[test]
fn np_var_count() {
    ok("set -- a b c; echo $#", "3\n");
}
#[test]
fn np_var_pid() {
    ok_status("echo $$ >/dev/null", "", 0);
}
#[test]
fn np_var_dash_status() {
    ok("false; echo $?", "1\n");
}
#[test]
fn np_var_underscore() {
    // `$_` is "last argument of previous command" or empty
    ok_status("echo $_ >/dev/null; true", "", 0);
}

// Multi-word statement w/ var refs
#[test]
fn np_two_vars() {
    ok("a=1; b=2; echo $a $b", "1 2\n");
}
#[test]
fn np_var_in_assign() {
    ok("a=hi; b=$a; echo $b", "hi\n");
}

// Conditional with bare var
#[test]
fn np_cond_bare_var() {
    ok("x=set; [[ -n $x ]] && echo y", "y\n");
}

// Concatenation
#[test]
fn np_concat_var_lit() {
    ok("v=hi; echo ${v}_world", "hi_world\n");
}
#[test]
fn np_concat_lit_var() {
    ok("v=hi; echo prefix_$v", "prefix_hi\n");
}
#[test]
fn np_concat_var_var() {
    ok("a=hi; b=mom; echo $a-$b", "hi-mom\n");
}

// Escape sequences in echo
#[test]
fn np_echo_escape_n() {
    // zsh's echo defaults to BSD semantics: backslash escapes are
    // interpreted EVEN inside single quotes (the option BSD_ECHO is on
    // by default). So `echo 'a\nb'` emits `a<NL>b<NL>`, NOT a literal
    // backslash. Verified: `zsh -c "echo 'a\nb'" | xxd` → `61 0a 62 0a`.
    ok("echo 'a\\nb'", "a\nb\n");
}

// printf
#[test]
fn np_printf_basic() {
    ok("printf '%s\\n' hello", "hello\n");
}
#[test]
fn np_printf_int() {
    ok("printf 'n=%d\\n' 42", "n=42\n");
}

// Test command (POSIX [)
#[test]
fn np_test_str_eq() {
    ok("[ a = a ] && echo y", "y\n");
}
#[test]
fn np_test_int_lt() {
    ok("[ 1 -lt 2 ] && echo y", "y\n");
}

// $((...)) inside string
#[test]
fn np_arith_in_string() {
    ok(r#"echo "n=$((1+2))""#, "n=3\n");
}

// Empty subshell
#[test]
fn np_empty_subshell() {
    ok_status("()", "", 0);
}

// Brace group preserving status
#[test]
fn np_brace_keeps_status() {
    ok_status("{ false; }; echo $?", "1\n", 0);
}

// Nested cmdsub
#[test]
fn np_cmdsub_chained() {
    ok(r#"x=$(echo $(echo nested)); echo $x"#, "nested\n");
}

// Tilde expansion (limited)
#[test]
fn np_tilde_home() {
    let (status, out) = run("echo ~");
    assert_eq!(status, 0);
    assert!(!out.trim().is_empty(), "tilde should expand to non-empty");
}
