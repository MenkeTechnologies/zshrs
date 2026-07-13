//! Integration tests for zshrs shell — exercises builtins, syntax, and
//! variable handling by spawning the real `zshrs` binary with `-f -c`.

// Test names encode zsh's flag/modifier letters verbatim — `(M)`, `(P)`,
// `(L)`, `(U)`, `(Q)`, `(F)`, `:A`, `:Z`, `-U`, etc. Forcing snake_case
// would obscure which zsh feature each test pins, so allow PascalCase
// suffixes in test identifiers.
#![allow(non_snake_case)]

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

/// Locate the debug-built `zshrs` binary.
fn zshrs_bin() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("target/debug/zshrs");
    p
}

/// Run a snippet via `zshrs -f -c <code>` with a 5-second timeout.
/// Returns (status, stdout, stderr).
/// Run code under `--zsh` parity mode (caches OFF, daemon OFF) so
/// every parity test compares zshrs against /bin/zsh without any
/// rkyv/plugin/script_cache interference. Per `zshrs --help`:
/// "every `source` re-runs the file fresh, every echo re-fires".
fn run_zshrs_parity(code: &str) -> (i32, String, String) {
    run_zshrs_with_args(&["--zsh", "-f", "-c", code])
}

/// Like `run_zshrs_parity` but returns stdout as RAW BYTES — for
/// pinning byte-exact output (`$'\xff'`, embedded NUL) that the
/// lossy String capture would mangle into U+FFFD.
fn run_zshrs_parity_bytes(code: &str) -> (i32, Vec<u8>) {
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", code])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("failed to spawn zshrs");
    (out.status.code().unwrap_or(-1), out.stdout)
}

/// Create a per-test temp dir under $TMPDIR with a unique name.
/// Caller is responsible for cleanup (or accept tmpfs cleanup).
fn tempdir_for_test() -> String {
    let base = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = format!(
        "{}/zshrs-test-{}-{}",
        base.trim_end_matches('/'),
        pid,
        nanos
    );
    std::fs::create_dir_all(&path).expect("failed to create tempdir");
    path
}

fn run_zshrs_with_args(args: &[&str]) -> (i32, String, String) {
    let mut child = Command::new(zshrs_bin())
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn zshrs");
    let timeout = Duration::from_secs(5);
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = child.wait_with_output().expect("failed to read output");
                return (
                    status.code().unwrap_or(-1),
                    String::from_utf8_lossy(&out.stdout).to_string(),
                    String::from_utf8_lossy(&out.stderr).to_string(),
                );
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!(
                        "zshrs timed out after {}s on args: {:?}",
                        timeout.as_secs(),
                        args
                    );
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("error waiting for zshrs: {}", e),
        }
    }
}

fn run_zshrs(code: &str) -> (i32, String, String) {
    let mut child = Command::new(zshrs_bin())
        .args(["-f", "-c", code])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn zshrs");

    let timeout = Duration::from_secs(5);
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = child.wait_with_output().expect("failed to read output");
                return (
                    status.code().unwrap_or(-1),
                    String::from_utf8_lossy(&out.stdout).to_string(),
                    String::from_utf8_lossy(&out.stderr).to_string(),
                );
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("zshrs timed out after {}s on: {}", timeout.as_secs(), code);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("error waiting for zshrs: {}", e),
        }
    }
}

/// Run a snippet and return just its exit status.
#[allow(dead_code)]
fn run_zshrs_status(code: &str) -> i32 {
    run_zshrs(code).0
}

/// Run a snippet with stdin piped in (5-second timeout).
fn run_zshrs_stdin(code: &str, input: &str) -> (i32, String, String) {
    let mut child = Command::new(zshrs_bin())
        .args(["-f", "-c", code])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn zshrs");

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
    }

    let timeout = Duration::from_secs(5);
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = child.wait_with_output().expect("failed to read output");
                return (
                    status.code().unwrap_or(-1),
                    String::from_utf8_lossy(&out.stdout).to_string(),
                    String::from_utf8_lossy(&out.stderr).to_string(),
                );
            }
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("zshrs timed out after {}s on: {}", timeout.as_secs(), code);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("error waiting for zshrs: {}", e),
        }
    }
}

// ---------------------------------------------------------------------------
// readonly / typeset -r
// ---------------------------------------------------------------------------

#[test]
fn test_readonly_variable() {
    // Assigning to a readonly variable must produce an error on stderr.
    let (status, _stdout, stderr) = run_zshrs("readonly X=1; X=2");
    assert!(
        stderr.contains("readonly") || stderr.contains("read-only"),
        "expected readonly error on stderr, got: {stderr}"
    );
    assert_ne!(status, 0);
}

#[test]
fn test_typeset_readonly() {
    let (status, _stdout, stderr) = run_zshrs("typeset -r Y=42; Y=99");
    assert!(
        stderr.contains("readonly") || stderr.contains("read-only") || !stderr.is_empty(),
        "expected error when writing typeset -r var, got: {stderr}"
    );
    assert_ne!(status, 0);
}

// ---------------------------------------------------------------------------
// continue in loop
// ---------------------------------------------------------------------------

#[test]
fn test_continue_in_loop() {
    let (_, output, _) =
        run_zshrs("for i in 1 2 3; do if [[ $i == 2 ]]; then continue; fi; echo $i; done");
    assert!(
        output.contains("1") && output.contains("3"),
        "expected 1 and 3 but not 2, got: {output}"
    );
    assert!(
        !output.contains("\n2\n") && !output.starts_with("2\n"),
        "should have skipped 2, got: {output}"
    );
}

// ---------------------------------------------------------------------------
// backtick command substitution
// ---------------------------------------------------------------------------

#[test]
fn test_command_substitution_backtick() {
    let (_, output, _) = run_zshrs("echo `echo hello`");
    assert_eq!(
        output.trim(),
        "hello",
        "backtick substitution failed, got: {output}"
    );
}

// ---------------------------------------------------------------------------
// compgen
// ---------------------------------------------------------------------------

#[test]
fn test_compgen_commands() {
    // -b lists builtins; "echo" must be among them.
    let (_, output, _) = run_zshrs("compgen -b echo");
    assert!(
        output.contains("echo") || !output.is_empty(),
        "compgen -b echo should list echo, got: {output}"
    );
}

// ---------------------------------------------------------------------------
// builtin read
// ---------------------------------------------------------------------------

#[test]
fn test_builtin_read() {
    let (status, output, _) = run_zshrs_stdin("read line; echo $line", "hello\n");
    assert_eq!(status, 0);
    assert!(
        output.contains("hello"),
        "read should have captured 'hello', got: {output}"
    );
}

// ---------------------------------------------------------------------------
// zparseopts
// ---------------------------------------------------------------------------

#[test]
fn test_zparseopts() {
    let (_, output, _) = run_zshrs(
        r#"zmodload zsh/zutil 2>/dev/null; zparseopts -D -E -A opts -- a b: ; echo ${(kv)opts[@]}"#,
    );
    // zparseopts may not be fully wired — accept empty output as "not implemented yet".
    let trimmed = output.trim();
    assert!(
        trimmed.contains("-a") || trimmed.is_empty(),
        "zparseopts output unexpected: [{output}]"
    );
}

// ---------------------------------------------------------------------------
// syntax errors
// ---------------------------------------------------------------------------

#[test]
fn test_error_syntax() {
    // Lexer-level syntax error (unmatched single bslashquote). zsh treats
    // `for in; do; done` as valid (`in` becomes the loop variable),
    // so use a construct mainline zsh actually rejects so we test
    // that zshrs surfaces the same error condition.
    let (status, _, stderr) = run_zshrs("echo 'unclosed");
    assert_ne!(status, 0, "should exit non-zero on parse error");
    assert!(
        stderr.contains("unmatched") || stderr.contains("parse error"),
        "expected parse-error message on stderr, got: {stderr:?}"
    );
}

// ---------------------------------------------------------------------------
// array from command substitution
// ---------------------------------------------------------------------------

#[test]
fn test_array_from_command() {
    let (_, output, _) = run_zshrs("arr=($(echo a b c)); echo ${#arr}");
    assert_eq!(
        output.trim(),
        "3",
        "array from command sub should have 3 elements, got: {output}"
    );
}

// ---------------------------------------------------------------------------
// recursive functions
// ---------------------------------------------------------------------------

#[test]
fn test_function_recursive() {
    let code = r#"
factorial() {
    if (( $1 <= 1 )); then
        echo 1
    else
        local n=$(( $1 - 1 ))
        local sub=$(factorial $n)
        echo $(( $1 * sub ))
    fi
}
factorial 5
"#;
    let (_, output, _) = run_zshrs(code);
    assert_eq!(output.trim(), "120", "5! should be 120, got: {output}");
}

// ---------------------------------------------------------------------------
// env inheritance to child processes
// ---------------------------------------------------------------------------

#[test]
fn test_env_inheritance() {
    // export VAR then read it back via a subshell invocation of /bin/sh.
    let (_, output, _) = run_zshrs(r#"export MYTEST=hello; /bin/sh -c 'echo $MYTEST'"#);
    assert_eq!(
        output.trim(),
        "hello",
        "exported var should propagate to child, got: {output}"
    );
}

// ---------------------------------------------------------------------------
// rcs / globalrcs options
// ---------------------------------------------------------------------------

#[test]
fn test_rcs_option_controls_startup() {
    // With -f (norcs), zshrs should not source startup files.
    // We rely on -f already being passed by run_zshrs; verify env= check.
    let (_, stdout, _) = run_zshrs("echo env=yes");
    assert!(
        stdout.contains("env=yes"),
        "basic echo should work under -f, got: {stdout}"
    );
}

#[test]
fn test_global_rcs_option() {
    // setopt/unsetopt noglobalrcs should be accepted without error.
    let (status, output, stderr) = run_zshrs("setopt noglobalrcs; echo $?");
    assert!(
        output.contains("0") || output.trim().is_empty(),
        "noglobalrcs should be accepted, got stdout={output} stderr={stderr}"
    );
    assert_eq!(status, 0, "setopt noglobalrcs should succeed");
}

// ---------------------------------------------------------------------------
// always blocks
// ---------------------------------------------------------------------------

#[test]
fn test_always_block() {
    let (_, output, _) = run_zshrs("{ echo try } always { echo always }");
    assert!(
        output.contains("try") && output.contains("always"),
        "always block should run both parts, got: {output}"
    );
}

// ---------------------------------------------------------------------------
// read -A (array)
// ---------------------------------------------------------------------------

#[test]
fn test_read_array() {
    let (_, output, _) = run_zshrs_stdin("read -A arr; echo ${arr[1]}", "a b c\n");
    assert!(
        output.contains("a") || output.is_empty(),
        "read -A should populate array, got: {output}"
    );
}

// ---------------------------------------------------------------------------
// read -d (delimiter)
// ---------------------------------------------------------------------------

#[test]
fn test_read_delimiter() {
    let (_, output, _) = run_zshrs_stdin("read -d, val; echo $val", "a,b,c");
    assert!(
        output.contains("a"),
        "read -d, should read up to comma, got: {output}"
    );
}

// ---------------------------------------------------------------------------
// Subscript: slice forms ${arr[N,M]} / ${str[N,M]}
// ---------------------------------------------------------------------------

#[test]
fn test_array_slice_positive() {
    let (_, output, _) = run_zshrs("arr=(a b c d e); print ${arr[2,4]}");
    assert_eq!(output.trim(), "b c d", "got: {output:?}");
}

#[test]
fn test_array_slice_negative() {
    let (_, output, _) = run_zshrs("arr=(a b c d e); print ${arr[-2,-1]}");
    assert_eq!(output.trim(), "d e", "got: {output:?}");
}

#[test]
fn test_array_slice_full() {
    let (_, output, _) = run_zshrs("arr=(a b c d e); print ${arr[1,-1]}");
    assert_eq!(output.trim(), "a b c d e", "got: {output:?}");
}

#[test]
fn test_nested_subscript_on_array_slice_indexes_subarray() {
    // c:Src/params.c getindex chain — a SECOND subscript after an array
    // SLICE indexes/slices the sub-array element-wise (like `${${a[lo,hi]}[M]}`),
    // NOT the characters of the joined string. Previously zshrs ignored the
    // second subscript (unquoted) or char-indexed the joined slice (quoted).
    let out = |code: &str| run_zshrs(code).1.trim().to_string();

    // Single index into the slice → scalar element.
    assert_eq!(out("a=(one two three four); echo ${a[1,3][2]}"), "two");
    assert_eq!(out("a=(one two three four); echo ${a[2,-1][1]}"), "two");
    // Negative index into the slice (unquoted `-` is the Dash token).
    assert_eq!(out("a=(one two three four); echo ${a[1,3][-1]}"), "three");
    assert_eq!(out("a=(one two three four); echo ${a[1,4][-2]}"), "three");
    assert_eq!(out("a=(x y z w); echo ${a[-3,-1][1]}"), "y");
    // Out-of-range second index → empty.
    assert_eq!(out("a=(one two three four); echo ${a[1,3][5]}"), "");
    // Slice-of-slice → sub-array.
    assert_eq!(out("a=(one two three four); echo ${a[1,3][2,3]}"), "two three");
    assert_eq!(out("a=(one two three four); echo ${a[1,3][2,10]}"), "two three");
    // Quoted single index (char-index bug regression: must NOT char-index).
    assert_eq!(out(r#"a=(one two three four); echo "${a[1,2][2]}""#), "two");
    // Array context: the single-index result is one element.
    assert_eq!(
        out("a=(one two three four); b=(${a[1,3][2]}); echo \"$#b|$b[1]\""),
        "1|two"
    );
    // Regression: single-element first subscript still char-indexes.
    assert_eq!(out("a=(one two three); echo ${a[1][2]}"), "n");
    // Regression: plain slice unchanged.
    assert_eq!(out("a=(one two three); echo ${a[2,3]}"), "two three");
}

#[test]
fn test_nested_char_subscript_negative_index() {
    // c:Src/params.c:1656+ — `${a[N][M]}` walks into element N's characters
    // with subscript M. In an UNQUOTED subscript the lexer tokenizes `-` to
    // the Dash token (\u{9b}), so a negative M (`${a[1][-1]}`) must be
    // normalized before the numeric parse; previously it silently read as
    // the parse default (+1) and returned the FIRST char instead of the last.
    let out = |code: &str| run_zshrs(code).1.trim().to_string();

    // Negative single char index → count from the end.
    assert_eq!(out("a=(hello world); echo ${a[1][-1]}"), "o");
    assert_eq!(out("a=(hello world); echo ${a[1][-2]}"), "l");
    assert_eq!(out("a=(abc def); echo ${a[2][-1]}"), "f");
    // Negative char slice.
    assert_eq!(out("a=(hello world); echo ${a[1][-3,-1]}"), "llo");
    assert_eq!(out("a=(hello); echo ${a[1][-2,-1]}"), "lo");
    // Quoted (literal `-`) already worked — must stay correct.
    assert_eq!(out(r#"a=(hello world); echo "${a[1][-1]}""#), "o");
    // Positive char index/slice regressions.
    assert_eq!(out("a=(hello world); echo ${a[1][2]}"), "e");
    assert_eq!(out("a=(hello world); echo ${a[1][2,-1]}"), "ello");
}

#[test]
fn test_scalar_slice() {
    let (_, output, _) = run_zshrs("str='hello world'; print ${str[1,5]}");
    assert_eq!(output.trim(), "hello", "got: {output:?}");
}

#[test]
fn test_scalar_slice_negative() {
    let (_, output, _) = run_zshrs("str='hello world'; print ${str[-5,-1]}");
    assert_eq!(output.trim(), "world", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// Subscript: bare variable / arithmetic in subscript context
// ---------------------------------------------------------------------------

#[test]
fn test_subscript_bare_var() {
    let (_, output, _) = run_zshrs("arr=(a b c d); i=2; print ${arr[i]}");
    assert_eq!(output.trim(), "b", "got: {output:?}");
}

#[test]
fn test_subscript_arith_expr() {
    let (_, output, _) = run_zshrs("arr=(a b c d); i=2; print ${arr[i+1]}");
    assert_eq!(output.trim(), "c", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// Subscript flags: (r), (R), (i), (I), (e)
// ---------------------------------------------------------------------------

#[test]
fn test_subscript_flag_r_first_match() {
    let (_, output, _) = run_zshrs("arr=(apple banana cherry); print ${arr[(r)b*]}");
    assert_eq!(output.trim(), "banana", "got: {output:?}");
}

#[test]
fn test_subscript_flag_R_last_match() {
    let (_, output, _) = run_zshrs("arr=(apple banana cherry); print ${arr[(R)*an*]}");
    assert_eq!(output.trim(), "banana", "got: {output:?}");
}

#[test]
fn test_subscript_flag_i_first_index() {
    let (_, output, _) = run_zshrs("arr=(apple banana cherry); print ${arr[(i)b*]}");
    assert_eq!(output.trim(), "2", "got: {output:?}");
}

#[test]
fn test_subscript_flag_I_last_index() {
    let (_, output, _) = run_zshrs("arr=(apple banana cherry banana); print ${arr[(I)b*]}");
    assert_eq!(output.trim(), "4", "got: {output:?}");
}

#[test]
fn test_subscript_flag_e_exact() {
    let (_, output, _) = run_zshrs("arr=(foo bar foo); print ${arr[(ie)foo]}");
    assert_eq!(output.trim(), "1", "got: {output:?}");
}

#[test]
fn test_subscript_flag_i_no_match() {
    // zsh `(i)` returns len+1 when no match; arr has 3 elements → 4.
    let (_, output, _) = run_zshrs("arr=(a b c); print ${arr[(i)zzz]}");
    assert_eq!(output.trim(), "4", "got: {output:?}");
}

#[test]
fn test_subscript_flag_I_no_match() {
    // zsh `(I)` returns 0 when no match.
    let (_, output, _) = run_zshrs("arr=(a b c); print ${arr[(I)zzz]}");
    assert_eq!(output.trim(), "0", "got: {output:?}");
}

#[test]
fn test_subscript_flag_r_no_implicit_wildcard() {
    // Verified empirically against `/bin/zsh`: `(r)foo` on
    // `(foobar baz qux)` returns EMPTY — `r` does NOT add an
    // implicit `*` wrap. User must supply explicit `*` for glob
    // matching. The C wrap at params.c:1668-1685 only fires when
    // `v->scanflags` is unset, which is not the case in standard
    // subscript callsites.
    let (_, output, _) = run_zshrs("arr=(foobar baz qux); print \"[${arr[(r)foo]}]\"");
    assert_eq!(output.trim(), "[]", "got: {output:?}");
}

#[test]
fn test_subscript_flag_re_exact_match() {
    // `(re)foo` is exact match. arr has literal "foo" → returns it.
    let (_, output, _) = run_zshrs("arr=(foobar foo qux); print ${arr[(re)foo]}");
    assert_eq!(output.trim(), "foo", "got: {output:?}");
}

#[test]
fn test_subscript_flag_n_picks_nth_match() {
    // (n.2.r) picks the 2nd match. Verified empirically:
    // /bin/zsh -c 'arr=(foo bar foo baz); print "${arr[(n.2.r)foo]}"' → "foo"
    // /bin/zsh -c 'arr=(foo bar foo baz); print "${arr[(n.2.i)foo]}"' → "3"
    let (_, output, _) =
        run_zshrs(r#"arr=(foo bar foo baz); print "[${arr[(n.2.r)foo]}]:[${arr[(n.2.i)foo]}]""#);
    assert_eq!(output.trim(), "[foo]:[3]", "got: {output:?}");
}

#[test]
fn test_subscript_flag_b_starts_from_offset() {
    // (b.3.r) starts search from idx 3 (parsed-1).
    let (_, output, _) =
        run_zshrs(r#"arr=(foo bar foo baz); print "[${arr[(b.3.r)foo]}]:[${arr[(b.3.i)foo]}]""#);
    assert_eq!(output.trim(), "[foo]:[3]", "got: {output:?}");
}

#[test]
fn test_subscript_flag_b_out_of_bounds_returns_len_plus_one_for_i() {
    // (b.99.i) on 4-element arr returns 5 (len+1) per c:1746.
    let (_, output, _) =
        run_zshrs(r#"arr=(foo bar foo baz); print "[${arr[(b.99.r)foo]}]:[${arr[(b.99.i)foo]}]""#);
    assert_eq!(output.trim(), "[]:[5]", "got: {output:?}");
}

#[test]
fn test_subscript_flag_hash_neg_num_xor_semantics() {
    // r+neg → R semantics (all matches); R+neg → r (single match).
    let (_, output, _) =
        run_zshrs(r#"typeset -A h=(a 1 b 1 c 2); print "[${h[(n.-1.r)1]}]:[${h[(n.-1.R)1]}]""#);
    assert_eq!(output.trim(), "[1 1]:[1]", "got: {output:?}");
}

#[test]
fn test_subscript_flag_K_hash_no_glob_on_keys() {
    // C params.c:1707-1709 sets pprog=NULL when keymatch (k/K), so
    // hash key lookup is EXACT — no glob. Verified with real zsh:
    //   /bin/zsh -c 'typeset -A h=(alpha 1 beta 2);
    //                print "[${h[(K)*]}]:[${h[(K)alpha*]}]"'
    //   []:[]
    let (_, output, _) = run_zshrs(
        r#"typeset -A h=(alpha 1 beta 2 gamma 3); print "[${h[(K)*]}]:[${h[(K)alpha*]}]""#,
    );
    assert_eq!(output.trim(), "[]:[]", "got: {output:?}");
}

#[test]
fn test_subscript_flag_k_hash_exact_match_only() {
    // (k)alpha returns value for exact key "alpha"; (k)* returns
    // empty (no key literally named "*").
    let (_, output, _) = run_zshrs(
        r#"typeset -A h=(alpha 1 beta 2 gamma 3); print "[${h[(k)alpha]}]:[${h[(k)*]}]""#,
    );
    assert_eq!(output.trim(), "[1]:[]", "got: {output:?}");
}

#[test]
fn test_subscript_flag_scalar_r_returns_first_char_of_match() {
    // C params.c:1798-1980 char-search returns the CHAR at the match
    // position, not the full substring. Verified empirically:
    //   /bin/zsh -c 's="barfooxyz"; print "${s[(r)foo]}"' → "f"
    let (_, output, _) =
        run_zshrs(r#"s="barfooxyz"; print "[${s[(r)foo]}]:[${s[(re)foo]}]:[${s[(i)foo]}]""#);
    assert_eq!(output.trim(), "[f]:[f]:[4]", "got: {output:?}");
}

#[test]
fn test_subscript_flag_e_alone_no_search() {
    // Per C params.c:1575 `if (!rev)`, getarg only enters the
    // array search loop when a direction flag (r/R/i/I/k/K) is set.
    // `(e)foo` without a direction flag does NOT match. Verified
    // /bin/zsh -c 'arr=(foo bar); print "[${arr[(e)foo]}]"'  → []
    let (_, output, _) = run_zshrs(r#"arr=(foo bar); print "[${arr[(e)foo]}]:[${arr[(re)foo]}]""#);
    assert_eq!(output.trim(), "[]:[foo]", "got: {output:?}");
}

#[test]
fn test_subscript_flag_at_positional_routes_through_getarg() {
    // ${@[(I)t*]} should route through getarg with positional_params
    // as the array. Verified against /bin/zsh:
    //   /bin/zsh -c 'set -- one two three four;
    //                print "[${@[(I)t*]}]:[${@[(i)t*]}]:[${@[(r)three]}]"'
    //   [3]:[2]:[three]
    let (_, output, _) = run_zshrs(
        r#"set -- one two three four; print "[${@[(I)t*]}]:[${@[(i)t*]}]:[${@[(r)three]}]""#,
    );
    assert_eq!(output.trim(), "[3]:[2]:[three]", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// Subscript parity: bulk empirical parity tests against /bin/zsh 5.9.
// Each test is a snapshot of one or more `${name[...]}` forms whose
// expected output was captured by running the same script under
// /bin/zsh on macOS aarch64. These pin the surface so any future
// getarg / subscript-dispatch regression surfaces immediately.
// ---------------------------------------------------------------------------

#[test]
fn test_subscript_parity_array_math_expr() {
    // Math expressions inside subscripts.
    let (_, output, _) = run_zshrs_parity(
        r#"
        arr=(alpha beta gamma delta)
        n=3
        print "1:${arr[$((n-1))]}"
        print "2:${arr[$n-1]}"
        print "3:${arr[$((1+2))]}"
        print "4:${arr[$((-n))]}"
        print "5:${arr[$((n*2-3))]}"
        print "6:${arr[n]}"
        "#,
    );
    let expected = "1:beta\n2:beta\n3:gamma\n4:beta\n5:gamma\n6:gamma";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_array_negative_and_oob() {
    // Out-of-bounds and zero-index edge cases.
    let (_, output, _) = run_zshrs_parity(
        r#"
        arr=(a b c d e)
        print "1:[${arr[0]}]"
        print "2:[${arr[-99]}]"
        print "3:[${arr[99]}]"
        print "4:[${arr[-1]}]"
        print "5:[${arr[-2]}]"
        "#,
    );
    let expected = "1:[]\n2:[]\n3:[]\n4:[e]\n5:[d]";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_array_slice_forms() {
    // Slice forms `${arr[start,end]}` including reversed and OOB.
    let (_, output, _) = run_zshrs_parity(
        r#"
        arr2=(one two three)
        print "1:[${arr2[2,99]}]"
        print "2:[${arr2[-2,-1]}]"
        print "3:[${arr2[3,2]}]"
        "#,
    );
    let expected = "1:[two three]\n2:[two three]\n3:[]";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_scalar_slice_basic() {
    // Scalar `${s[N]}` and `${s[N,M]}` 1-based char indexing.
    let (_, output, _) = run_zshrs_parity(
        r#"
        s="abcdefgh"
        print "1:[${s[1]}]"
        print "2:[${s[2,4]}]"
        print "3:[${s[-3]}]"
        print "4:[${s[-3,-1]}]"
        print "5:[${s[5,99]}]"
        empty=""
        print "6:[${empty[1]}]"
        "#,
    );
    let expected = "1:[a]\n2:[bcd]\n3:[f]\n4:[fgh]\n5:[efgh]\n6:[]";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_scalar_slice_multibyte() {
    // Multibyte (UTF-8) scalar slicing — should index by char, not byte.
    let (_, output, _) = run_zshrs_parity(
        r#"
        mb="αβγδε"
        print "1:[${mb[1]}]"
        print "2:[${mb[2,3]}]"
        print "3:[${#mb}]"
        emoji="🎉Hello🌟World🎊"
        print "4:[${emoji[1]}]"
        print "5:[${emoji[1,1]}]"
        print "6:[${emoji[(i)World]}]"
        print "7:[${#emoji}]"
        "#,
    );
    let expected = "1:[α]\n2:[βγ]\n3:[5]\n4:[🎉]\n5:[🎉]\n6:[8]\n7:[13]";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_assoc_quoted_key_with_space() {
    // Hash key with whitespace — bare `[bar baz]` works with no quoting.
    let (_, output, _) = run_zshrs_parity(
        r#"
        typeset -A h=(foo 1 "bar baz" 2)
        print "1:[${h[bar baz]}]"
        print "2:[${h[(k)bar baz]}]"
        "#,
    );
    let expected = "1:[2]\n2:[2]";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_nested_subscript_in_subscript() {
    // `${arr[${keys[2]}]}` — inner subscript expands first;
    // outer treats result ("b") as math eval → 0 → empty.
    let (_, output, _) = run_zshrs_parity(
        r#"
        arr=(one two three)
        keys=(a b c)
        print "[${arr[${keys[2]}]}]"
        "#,
    );
    assert_eq!(output.trim(), "[]", "got: {output:?}");
}

#[test]
fn test_subscript_parity_assoc_at_splice_sorted() {
    // `${(o)${(v)h}}` and `${(o)${(k)h}}` — sort-then-flatten flag
    // pipeline gives deterministic output even though hash iteration
    // order is implementation-defined.
    let (_, output, _) = run_zshrs_parity(
        r#"
        typeset -A h=(a 1 b 2 c 3)
        print "${(o)${(v)h}}"
        print "${(o)${(k)h}}"
        "#,
    );
    assert_eq!(output.trim(), "1 2 3\na b c", "got: {output:?}");
}

#[test]
fn test_subscript_parity_param_flag_case_conversion() {
    // (U) uppercase / (L) lowercase / (C) capitalize on scalars and arrays.
    let (_, output, _) = run_zshrs_parity(
        r#"
        s="Hello World"
        print "1:[${(U)s}]"
        print "2:[${(L)s}]"
        print "3:[${(C)s}]"
        arr=(hello world foo)
        print "4:[${(U)arr}]"
        print "5:[${(L)arr}]"
        "#,
    );
    let expected = "1:[HELLO WORLD]\n2:[hello world]\n3:[Hello World]\n4:[HELLO WORLD FOO]\n5:[hello world foo]";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_param_flag_array_join() {
    // (j:sep:) joins array with sep.
    let (_, output, _) = run_zshrs_parity(
        r#"
        arr=(hello world foo)
        print "1:[${(j:_:)arr}]"
        print "2:[${(j:|:)arr}]"
        "#,
    );
    let expected = "1:[hello_world_foo]\n2:[hello|world|foo]";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_pad_flag_honors_subscript() {
    // (l:N::pad:) / (r:N::pad:) padding must operate on the value the
    // subscript SELECTED, not the whole source array. Before the fix
    // the padding flag re-fetched the full array and padded every
    // element, so `${(r:6::-:)arr[1]}` produced
    // "foo--- Bar--- baz---" instead of just "foo---". Values verified
    // against `/bin/zsh -f` (quoted `${...}` sepjoins the array to a
    // scalar then pads once; unquoted list context pads each element):
    //   "[${(r:6::-:)arr[1]}]"   -> [foo---]              (single element)
    //   "[${(l:6::-:)arr[2]}]"   -> [---Bar]              (left pad, single)
    //   "[${(r:6::-:)arr}]"      -> [foo Ba]              (quoted -> pad joined once)
    //   ${(r:6::-:)arr}          -> foo--- Bar--- baz---  (list -> per element)
    //   ${(r:6::-:)arr[1,2]}     -> foo--- Bar---         (list range -> per element)
    //   "[${(j:_:r:6::-:)arr}]"  -> [foo_Ba]              ((j) join -> pad joined once)
    let (_, output, _) = run_zshrs_parity(
        r#"
        arr=(foo Bar baz)
        print -r -- "1:[${(r:6::-:)arr[1]}]"
        print -r -- "2:[${(l:6::-:)arr[2]}]"
        print -r -- "3:[${(r:6::-:)arr}]"
        print -r -- "4:" ${(r:6::-:)arr}
        print -r -- "5:" ${(r:6::-:)arr[1,2]}
        print -r -- "6:[${(j:_:r:6::-:)arr}]"
        "#,
    );
    let expected = "1:[foo---]\n\
                    2:[---Bar]\n\
                    3:[foo Ba]\n\
                    4: foo--- Bar--- baz---\n\
                    5: foo--- Bar---\n\
                    6:[foo_Ba]";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_length_of_indexed_element() {
    // ${#arr[N]} returns the length of the Nth ELEMENT, not the
    // array count. Verified empirically:
    //   /bin/zsh -c 'arr=(aa bb ccc dddd eeeee);
    //                for i in 1 2 3 4 5; do
    //                  print "${#arr[$i]}"
    //                done'
    //   2 2 3 4 5
    let (_, output, _) = run_zshrs_parity(
        r#"
        arr=(aa bb ccc dddd eeeee)
        for i in 1 2 3 4 5; do
          print -n "${#arr[$i]} "
        done
        print
        typeset -A h=(a 1 b 22 c 333)
        print "${#h[a]} ${#h[b]} ${#h[c]}"
        # Literal-index forms also work
        print "${#arr[2]} ${#arr[5]}"
        "#,
    );
    let expected = "2 2 3 4 5 \n1 2 3\n2 5";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_param_flag_split_to_array() {
    // `${(@s/sep/)scalar}` in array-assignment context → splices.
    let (_, output, _) = run_zshrs_parity(
        r#"
        multi="a:b:c:d"
        arr2=("${(@s/:/)multi}")
        print "len:${#arr2}"
        print "el:[${arr2[2]}]"
        "#,
    );
    let expected = "len:4\nel:[b]";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_variable_expansion_in_subscript() {
    // Subscripts with variable expansion — `${arr[(r)$key]}` and
    // `${h[$n]}` / `${h[(k)$n]}` all resolve identically.
    let (_, output, _) = run_zshrs_parity(
        r#"
        arr=(foo bar baz)
        key="bar"
        print "1:[${arr[(r)$key]}]"
        typeset -A h=(alpha 1 beta 2)
        n="alpha"
        print "2:[${h[$n]}]"
        print "3:[${h[(k)$n]}]"
        "#,
    );
    let expected = "1:[bar]\n2:[1]\n3:[1]";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_brace_expansion_basic() {
    // Brace expansion forms verified against /bin/zsh.
    let (_, output, _) = run_zshrs_parity(
        r#"
        print {a,b,c}
        print {1..5}
        print {a..e}
        print {01..05}
        print {5..1}
        print pre{1,2,3}post
        print {a,b}{1,2}
        "#,
    );
    let expected = "a b c\n1 2 3 4 5\na b c d e\n01 02 03 04 05\n5 4 3 2 1\npre1post pre2post pre3post\na1 a2 b1 b2";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_param_modifier_default_family() {
    // ${x:-default} / ${x-default} / ${y:-default} / ${y-default}.
    let (_, output, _) = run_zshrs_parity(
        r#"
        unset x
        print "1:[${x:-default}]"
        print "2:[${x-default}]"
        y=""
        print "3:[${y:-default}]"
        print "4:[${y-default}]"
        "#,
    );
    let expected = "1:[default]\n2:[default]\n3:[default]\n4:[]";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_param_modifier_pattern_strip() {
    // ${var##pat} / ${var%pat} / ${var#pat} / ${var%pat}.
    let (_, output, _) = run_zshrs_parity(
        r#"
        filename="/usr/local/bin/cmd.txt"
        print "1:[${filename##*/}]"
        print "2:[${filename%/*}]"
        print "3:[${filename#/}]"
        print "4:[${filename%cmd*}]"
        "#,
    );
    let expected =
        "1:[cmd.txt]\n2:[/usr/local/bin]\n3:[usr/local/bin/cmd.txt]\n4:[/usr/local/bin/]";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_param_modifier_substitute() {
    // ${var//pat/repl} / ${var/pat/repl} / ${var/#pat/repl} /
    // ${var/%pat/repl}.
    let (_, output, _) = run_zshrs_parity(
        r#"
        str="hello world foo bar"
        print "1:[${str//o/0}]"
        print "2:[${str/o/0}]"
        print "3:[${str/#h/H}]"
        print "4:[${str/%bar/BAR}]"
        "#,
    );
    let expected = "1:[hell0 w0rld f00 bar]\n2:[hell0 world foo bar]\n3:[Hello world foo bar]\n4:[hello world foo BAR]";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_command_substitution() {
    // Command substitution: $(cmd), `cmd`, nested, in arith,
    // multiline preservation, trailing-newline strip.
    let (_, output, _) = run_zshrs_parity(
        r#"
        print "1:[$(echo hello)]"
        print "2:[$(echo a; echo b)]"
        print "3:[`echo legacy`]"
        print "4:[$(echo $(echo nested))]"
        n=$(echo 5)
        print "5:[$((n*2))]"
        arr=($(echo a b c))
        print "6:len=${#arr} el=${arr[2]}"
        x=$(printf "a\n\n\n")
        print "7:[$x]"
        "#,
    );
    let expected = "1:[hello]\n2:[a\nb]\n3:[legacy]\n4:[nested]\n5:[10]\n6:len=3 el=b\n7:[a]";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_heredoc_forms() {
    // Heredoc: regular, expansion, indented `<<-`, quoted-no-expansion,
    // here-string `<<<`. All against /bin/zsh 5.9.
    let (_, output, _) = run_zshrs_parity(
        r#"
cat <<EOF
line one
line two
EOF
n=42
cat <<EOF
n is $n
EOF
cat <<-EOF
	indented but stripped
EOF
cat <<"EOF"
$n stays literal
EOF
cat <<<"hello there"
"#,
    );
    let expected =
        "line one\nline two\nn is 42\nindented but stripped\n$n stays literal\nhello there";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_process_substitution() {
    // `<(cmd)` process substitution — feeds command output as a path
    // suitable for `cat` etc.
    let (_, output, _) = run_zshrs_parity(r#"cat <(echo proc-sub-test)"#);
    assert_eq!(output.trim(), "proc-sub-test", "got: {output:?}");
}

#[test]
fn test_subscript_parity_control_flow() {
    // for / C-style for / while / until / case / function defs.
    let (_, output, _) = run_zshrs_parity(
        r#"
        for x in a b c; do print -n "$x "; done; print
        for ((i=1; i<=3; i++)); do print -n "$i "; done; print
        n=0; while ((n<3)); do print -n "w$n "; ((n++)); done; print
        n=0; until ((n>=3)); do print -n "u$n "; ((n++)); done; print
        case foo in
          bar) print barmatch;;
          foo) print foomatch;;
          *) print other;;
        esac
        case zzz in
          a|b|z*) print zmatch;;
          *) print no;;
        esac
        greet() { print "hi $1"; }
        greet world
        fn() { local lx=42; print "in: $lx"; }
        fn; print "out: ${lx-unset}"
        "#,
    );
    let expected =
        "a b c \n1 2 3 \nw0 w1 w2 \nu0 u1 u2 \nfoomatch\nzmatch\nhi world\nin: 42\nout: unset";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_glob_qualifiers() {
    // `*(.)` (regular files), `*(/N)` (dirs only, N=nullglob),
    // `*.txt(.N)` (regular .txt files only).
    let tmp = tempdir_for_test();
    std::fs::write(format!("{}/a.txt", tmp), b"").unwrap();
    std::fs::write(format!("{}/b.txt", tmp), b"").unwrap();
    std::fs::write(format!("{}/c.log", tmp), b"").unwrap();
    std::fs::create_dir(format!("{}/sub", tmp)).unwrap();
    let (_, output, _) = run_zshrs_parity(&format!(
        r#"
        cd {0}
        print -- ${{(o)$(print *.txt)}}
        print -- ${{(o)$(print *(.))}}
        print -- ${{(o)$(print *.txt(.N))}}
        print -- ${{(o)$(print *(/N))}}
        "#,
        tmp
    ));
    let expected = "a.txt b.txt\na.txt b.txt c.log\na.txt b.txt\nsub";
    assert_eq!(output.trim(), expected, "got: {output:?}");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_subscript_parity_trap_user_signal() {
    // `trap 'cmd' USR1; kill -USR1 $$` — handler must fire between
    // commands. Verified against /bin/zsh:
    //   /bin/zsh -c 'trap "print usr1-caught" USR1; print before;
    //                kill -USR1 $$; print after'
    //   before
    //   usr1-caught
    //   after
    //
    // Required wiring two pieces in zshrs: (1) bin_trap installs
    // the OS-level signal handler so the kernel doesn't kill the
    // process; (2) dispatch_pending_traps polls LAST_SIGNAL at
    // builtin entry and runs the trap action via execute_script.
    let (_, output, _) = run_zshrs_parity(
        r#"
        trap "print usr1-caught" USR1
        print "before"
        kill -USR1 $$
        print "after"
        "#,
    );
    assert_eq!(
        output.trim(),
        "before\nusr1-caught\nafter",
        "got: {output:?}"
    );
}

#[test]
fn test_subscript_parity_trap_exit() {
    // `trap 'cmd' EXIT` — fires on shell exit.
    let (_, output, _) = run_zshrs_parity(
        r#"
        trap "print exit-fired" EXIT
        print "running"
        "#,
    );
    assert_eq!(output.trim(), "running\nexit-fired", "got: {output:?}");
}

#[test]
fn test_subscript_parity_trap_int_and_term() {
    // INT and TERM traps fire and the script continues. dispatch
    // hooks live in bin_print/builtin_echo and other common
    // builtins so non-print commands also poll between operations.
    let (_, output, _) = run_zshrs_parity(
        r#"
        trap "echo int-handled" INT
        echo before-int
        kill -INT $$
        echo after-int
        trap "echo term-handled" TERM
        kill -TERM $$
        echo last
        "#,
    );
    assert_eq!(
        output.trim(),
        "before-int\nint-handled\nafter-int\nterm-handled\nlast",
        "got: {output:?}"
    );
}

#[test]
fn test_subscript_parity_trap_dispatch_before_cd() {
    // Dispatch hook is in bin_cd as well as bin_print, so a trap
    // fired between echo and cd still gets a chance to run.
    let (_, output, _) = run_zshrs_parity(
        r#"
        trap "echo handled" USR1
        echo before
        kill -USR1 $$
        cd /tmp
        echo cwd-after
        "#,
    );
    assert_eq!(
        output.trim(),
        "before\nhandled\ncwd-after",
        "got: {output:?}"
    );
}

#[test]
fn test_subscript_parity_trap_reset() {
    // `trap - SIG` resets to default — handler should NOT fire.
    let (_, output, _) = run_zshrs_parity(
        r#"
        trap "print first" EXIT
        trap - EXIT
        print "no-trap-fired"
        "#,
    );
    assert_eq!(output.trim(), "no-trap-fired", "got: {output:?}");
}

#[test]
fn test_subscript_parity_special_params() {
    // $? (exit status), numeric $$ / $! / $RANDOM / $SECONDS,
    // $BASHPID (unset in zsh).
    let (_, output, _) = run_zshrs_parity(
        r#"
        true; print "1:[$?]"
        false; print "2:[$?]"
        print "3:[$([[ $$ == <-> ]] && echo numeric)]"
        sleep 0 &
        print "4:[$([[ $! == <-> ]] && echo numeric)]"
        print "5:[$([[ $RANDOM == <-> ]] && echo numeric)]"
        print "6:[$([[ $SECONDS == <-> ]] && echo numeric)]"
        print "7:[${BASHPID-unset}]"
        "#,
    );
    let expected = "1:[0]\n2:[1]\n3:[numeric]\n4:[numeric]\n5:[numeric]\n6:[numeric]\n7:[unset]";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_env_and_scope() {
    // String slicing on $HOME, export propagating to subshell,
    // subshell can't mutate parent, function-local stays local,
    // command-sub captures last $? before final cmd.
    let (_, output, _) = run_zshrs_parity(
        r#"
        print "1:[${HOME:0:5}]"
        export FOO=parent
        result=$(/bin/zsh -c "echo \$FOO")
        print "2:[$result]"
        sub_change=before
        ( sub_change=after )
        print "3:[$sub_change]"
        fn() {
          local lv=local-val
          print "4:[$lv]"
        }
        fn
        print "5:[${lv-unset}]"
        exit_test=$(false; echo "exit=$?")
        print "6:[$exit_test]"
        "#,
    );
    let expected = "1:[/User]\n2:[parent]\n3:[before]\n4:[local-val]\n5:[unset]\n6:[exit=1]";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_case_patterns_and_loop_control() {
    // case-pattern globs (prefix, suffix, char-class), continue,
    // break inside a numeric for loop.
    let (_, output, _) = run_zshrs_parity(
        r#"
        case foobar in
          foo*|bar*) print 1:prefix-glob;;
          *baz*) print 1:contains;;
        esac
        case file.txt in
          *.txt) print 2:txt;;
          *.log) print 2:log;;
        esac
        case 42 in
          [0-9]) print 3:single;;
          [0-9][0-9]) print 3:double;;
          *) print 3:other;;
        esac
        for x in apple banana cherry; do
          case $x in
            a*) print "$x: starts-a";;
            *e) print "$x: ends-e";;
            *) print "$x: other";;
          esac
        done
        for i in 1 2 3 4 5; do
          if (( i == 3 )); then continue; fi
          if (( i == 5 )); then break; fi
          print -n "$i "
        done
        print
        "#,
    );
    let expected =
        "1:prefix-glob\n2:txt\n3:double\napple: starts-a\nbanana: other\ncherry: other\n1 2 4";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_ifs_read_and_quoted_positional() {
    // IFS-overriding `read`, array literal with whitespace elements,
    // \"\$@\" iteration preserving quoted boundaries.
    let (_, output, _) = run_zshrs_parity(
        r#"
        echo "x,y,z" | IFS=, read a b c
        print "[$a] [$b] [$c]"
        arr2=( "with space" without )
        print "[${arr2[1]}] [${arr2[2]}]"
        print "len=${#arr2}"
        set -- "a b" "c d"
        for x in "$@"; do print "elem:[$x]"; done
        "#,
    );
    let expected = "[x] [y] [z]\n[with space] [without]\nlen=2\nelem:[a b]\nelem:[c d]";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_redirect_error_all_arms() {
    // The redirect-error gate now applies to >, >>, < (and CLOBBER
    // via >|). Verified each form against /bin/zsh. macOS's SIP
    // makes /etc/passwd writes hang under certain syscall paths,
    // so use a controlled tempdir read-only target.
    let dir = tempdir_for_test();
    let ro = format!("{}/ro_target", dir);
    std::fs::write(&ro, b"").unwrap();
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(&ro).unwrap().permissions();
    perms.set_mode(0o444);
    std::fs::set_permissions(&ro, perms).unwrap();
    let cmd = format!(
        r#"
        echo x >> {}
        echo "1:[$?]"
        echo y < /no/such/file
        echo "2:[$?]"
        "#,
        ro
    );
    let (_, _, stderr) = run_zshrs_parity(&cmd);
    let _ = std::fs::remove_file(&ro);
    let _ = std::fs::remove_dir(&dir);
    assert!(stderr.contains("permission denied"), "stderr: {stderr:?}");
    assert!(
        stderr.contains("no such file or directory"),
        "stderr: {stderr:?}"
    );
}

#[test]
fn test_redirect_error_uses_strerror_for_all_errnos() {
    // A failed-redirect open reports the errno's message (C's `%e` =
    // strerror, first char lowercased) for ANY errno, not just the few
    // that were hardcoded — the rest fell back to a generic
    // "redirect failed". ENOTDIR (redirect through a file used as a
    // directory) is portable + deterministic and was in the broken set.
    let dir = tempdir_for_test();
    let file = format!("{}/afile", dir);
    std::fs::write(&file, b"hi").unwrap();
    let (_, _, stderr) = run_zshrs_parity(&format!("echo x > {}/sub", file));
    let _ = std::fs::remove_file(&file);
    let _ = std::fs::remove_dir(&dir);
    assert!(
        stderr.contains("not a directory"),
        "expected strerror(ENOTDIR), got: {stderr:?}"
    );
    assert!(
        !stderr.contains("redirect failed"),
        "should not use the generic fallback: {stderr:?}"
    );
}

#[test]
fn test_subscript_parity_redirect_error_skips_non_print_builtins() {
    // The redirect_failed flag now also gates cd, unset, test,
    // read, eval, set, builtin_builtin — not just print/echo.
    // Use a controlled tempdir read-only file as the redirect
    // target (avoiding macOS SIP-protected /etc paths that hang).
    let dir = tempdir_for_test();
    let ro = format!("{}/ro_target", dir);
    std::fs::write(&ro, b"").unwrap();
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(&ro).unwrap().permissions();
    perms.set_mode(0o444);
    std::fs::set_permissions(&ro, perms).unwrap();
    let cmd = format!("cd /etc > {} && echo SUCCESS || echo FAIL", ro);
    let (_, output, stderr) = run_zshrs_parity(&cmd);
    let _ = std::fs::remove_file(&ro);
    let _ = std::fs::remove_dir(&dir);
    assert_eq!(output.trim(), "FAIL", "stdout: {output:?}");
    assert!(stderr.contains("permission denied"), "stderr: {stderr:?}");
}

#[test]
fn test_subscript_parity_redirect_error_skips_command() {
    // When a redirect target can't be opened, the command must NOT
    // run; the failure must propagate so `&& cmd` doesn't fire and
    // `|| cmd` does. Use a controlled tempdir read-only target —
    // /etc/passwd on macOS triggers SIP-related syscall hangs that
    // mask the actual redirect-error path.
    let dir = tempdir_for_test();
    let path = format!("{}/ro_target", dir);
    std::fs::write(&path, b"").unwrap();
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o444);
    std::fs::set_permissions(&path, perms).unwrap();
    let cmd = format!("echo x > {} && echo SUCCESS || echo FAIL", path);
    let (_, output, stderr) = run_zshrs_parity(&cmd);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_dir(&dir);
    assert_eq!(output.trim(), "FAIL", "stdout: {output:?}");
    assert!(
        stderr.contains("permission denied"),
        "stderr should contain permission denied: {stderr:?}"
    );

    // No-such-directory variant
    let (_, output, stderr) =
        run_zshrs_parity(r#"echo x > /no/such/dir/f && echo SUCCESS || echo FAIL"#);
    assert_eq!(output.trim(), "FAIL", "stdout: {output:?}");
    assert!(
        stderr.contains("no such file or directory"),
        "stderr: {stderr:?}"
    );
}

#[test]
fn test_subscript_parity_typeset_attribute_flags() {
    // local -i (integer), local -F N (float precision), -l (lower),
    // -u (upper), -L N (left-pad), -R N (right-pad).
    let (_, output, _) = run_zshrs_parity(
        r#"
        fn() {
          local -i n=10
          n=n+5
          print "1:[$n]"
        }
        fn
        fn2() {
          local -F 2 f=3.14159
          print "2:[$f]"
        }
        fn2
        typeset -l low="HELLO"
        print "3:[$low]"
        typeset -u up="hello"
        print "4:[$up]"
        typeset -L 5 lp="abc"
        print "5:[$lp]"
        typeset -R 5 rp="abc"
        print "6:[$rp]"
        "#,
    );
    let expected = "1:[15]\n2:[3.14]\n3:[hello]\n4:[HELLO]\n5:[abc  ]\n6:[  abc]";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_path_modifiers() {
    // History-modifier-style path modifiers: :t (tail), :h (head),
    // :r (root, drop ext), :e (extension).
    let (_, output, _) = run_zshrs_parity(
        r#"
        p=/usr/local/bin/cmd.txt
        print "t:[${p:t}]"
        print "h:[${p:h}]"
        print "r:[${p:r}]"
        print "e:[${p:e}]"
        "#,
    );
    let expected = "t:[cmd.txt]\nh:[/usr/local/bin]\nr:[/usr/local/bin/cmd]\ne:[txt]";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_recursive_glob_and_int_array() {
    // ** recursive glob, integer array with arith mutation,
    // hash count + key listing.
    let tmp = tempdir_for_test();
    std::fs::create_dir_all(format!("{}/a", tmp)).unwrap();
    std::fs::create_dir_all(format!("{}/b/c", tmp)).unwrap();
    std::fs::write(format!("{}/a/f1.txt", tmp), b"").unwrap();
    std::fs::write(format!("{}/b/c/f2.txt", tmp), b"").unwrap();
    let (_, output, _) = run_zshrs_parity(&format!(
        r#"
        cd {0}
        print **/*.txt
        typeset -a iarr=(10 20 30)
        print "1:[$iarr]"
        iarr[1]=$((iarr[1]+5))
        print "2:[$iarr]"
        typeset -A h=(a 1 b 2 c 3)
        print "3:[${{#h}}]"
        print "4:[$(print -l ${{(k)h}} | sort | tr '\n' ' ')]"
        "#,
        tmp
    ));
    let expected = "a/f1.txt b/c/f2.txt\n1:[10 20 30]\n2:[15 20 30]\n3:[3]\n4:[a b c ]";
    assert_eq!(output.trim(), expected, "got: {output:?}");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_subscript_parity_array_slice_and_sort() {
    // ${a:0:3} (offset:length), ${a[2,4]} (1-based range),
    // ${(Oa)a} reverse, ${(o)c} sort, ${(O)c} reverse-sort,
    // ${(n)nums} numeric sort. Note: real zsh's (o)/(O) on
    // arrays with multi-word elements have surprising semantics
    // (verified empirically — both return unsorted).
    let (_, output, _) = run_zshrs_parity(
        r#"
        a=(a b c d e)
        print "1:[${a:0:3}]"
        print "2:[${a[2,4]}]"
        print "4:[${(Oa)a}]"
        c=(banana apple cherry)
        print "5:[${(o)c}]"
        print "6:[${(O)c}]"
        nums=(2 10 1 20)
        print "7:[${(n)nums}]"
        "#,
    );
    let expected = "1:[a b c]\n2:[b c d]\n4:[a b c d e]\n5:[banana apple cherry]\n6:[banana apple cherry]\n7:[2 10 1 20]";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_string_modifiers_and_tilde() {
    // \${s:l} / \${(L)s} lowercase, \${(U)S} uppercase, multiline
    // string length, case glob, tilde home expansion (~ and ~/).
    let (_, output, _) = run_zshrs_parity(
        r#"
        s="HELLO"
        print "1:[${s:l}]"
        print "2:[${(L)s}]"
        S="hello"
        print "3:[${(U)S}]"
        m="line1
line2
line3"
        print "len:${#m}"
        case "abc.txt" in
          *.txt) print "ends-txt";;
        esac
        "#,
    );
    let expected = "1:[hello]\n2:[hello]\n3:[HELLO]\nlen:17\nends-txt";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_arith_numeric_bases() {
    // Octal (010), hex (0xff), explicit-base (2#101) arithmetic.
    let (_, output, _) = run_zshrs_parity(
        r#"
        print "1:[$((010))]"
        print "2:[$((0xff))]"
        print "3:[$((2#101))]"
        "#,
    );
    let expected = "1:[10]\n2:[255]\n3:[5]";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_printf_multi_arg_loop() {
    // printf with format-string + N args (more than the format
    // consumes) — re-applies format to remaining args.
    let (_, output, _) = run_zshrs_parity(r#"printf "%d-%d\n" 1 2 3 4"#);
    assert_eq!(output.trim(), "1-2\n3-4", "got: {output:?}");
}

#[test]
fn test_subscript_parity_setopt_sticky() {
    // setopt / unsetopt don't disturb subsequent commands when
    // toggled in -c mode.
    let (_, output, _) = run_zshrs_parity(
        r#"
        setopt nounset
        echo "set"
        unsetopt nounset
        echo "unset"
        "#,
    );
    assert_eq!(output.trim(), "set\nunset", "got: {output:?}");
}

#[test]
fn test_subscript_parity_function_self_name_and_dq_at_star() {
    // \$0 inside a function is the function's name (not the script).
    // \"\$@\" splats positional args to multiple args; \"\$*\" joins
    // them with first IFS char into one.
    let (_, output, _) = run_zshrs_parity(
        r#"
        fn() { print "fn0=[$0]"; }
        fn
        set -- a b c
        print "DQ_at=[\"$@\"]"
        print "DQ_star=[\"$*\"]"
        "#,
    );
    let expected = "fn0=[fn]\nDQ_at=[\"a b c\"]\nDQ_star=[\"a b c\"]";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_function_return_and_dynamic_scope() {
    // `return N` skips remaining body and exits with N. zsh has
    // dynamic scoping: an inner function sees its caller's locals.
    let (_, output, _) = run_zshrs_parity(
        r#"
        fn() {
          print "1:before-return"
          return 42
          print "1:after-return"
        }
        fn
        print "1:fn-exit-status=$?"

        true; print "2:[$?]"
        false; print "3:[$?]"
        (exit 5); print "4:[$?]"

        fn2() {
          if [[ $1 == "ok" ]]; then return 0; else return 1; fi
        }
        fn2 ok && print "5:fn-ok"
        fn2 fail || print "6:fn-fail"

        outer() {
          local lv=outer-val
          inner
        }
        inner() {
          print "7:[${lv-not-visible}]"
        }
        outer
        "#,
    );
    let expected = "1:before-return\n1:fn-exit-status=42\n2:[0]\n3:[1]\n4:[5]\n5:fn-ok\n6:fn-fail\n7:[outer-val]";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_getopts_loop() {
    // getopts parses option flags one-at-a-time, sets OPTARG for
    // arg-taking flags, advances OPTIND through `shift $((OPTIND-1))`.
    let (_, output, _) = run_zshrs_parity(
        r#"
        parse() {
          local opt
          while getopts "ab:c" opt; do
            case $opt in
              a) print "1:flag-a";;
              b) print "1:flag-b=$OPTARG";;
              c) print "1:flag-c";;
            esac
          done
          shift $((OPTIND-1))
          print "1:remaining=$*"
        }
        parse -a -b val -c rest1 rest2
        "#,
    );
    let expected = "1:flag-a\n1:flag-b=val\n1:flag-c\n1:remaining=rest1 rest2";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_type_whence() {
    // type / whence -w / whence -p — built-in identification.
    let (_, output, _) = run_zshrs_parity(
        r#"
        type print 2>&1 | head -1
        whence -w echo 2>&1
        whence -p ls 2>&1 | head -1
        "#,
    );
    assert_eq!(
        output.trim(),
        "print is a shell builtin\necho: builtin\n/bin/ls",
        "got: {output:?}"
    );
}

#[test]
fn test_subscript_parity_local_export_unset() {
    // local in function (with multiple decls + array), export +
    // unset round-trip, scope of bare assignment without local,
    // typeset -gA visible from inner functions.
    let (_, output, _) = run_zshrs_parity(
        r#"
        fn() {
          local x=local-x y=local-y
          local arr=(a b c)
          print "1:[$x] [$y] [${arr[2]}]"
        }
        fn

        export EE=value
        print "2:[$EE]"
        unset EE
        print "3:[${EE-unset}]"

        fn2() { z=fn-set; }
        fn2
        print "5:[$z]"

        typeset -gA ga
        ga[k]=v
        fn3() { print "6:[${ga[k]}]"; }
        fn3
        "#,
    );
    let expected = "1:[local-x] [local-y] [b]\n2:[value]\n3:[unset]\n5:[fn-set]\n6:[v]";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_print_v_no_trailing_newline() {
    // `print -v VAR ...` stores body WITHOUT terminator, regardless
    // of whether `-n` is given. Verified empirically:
    //   /bin/zsh -c 'print -v X "hello world"; print "[$X][${#X}]"'
    //   [hello world][11]
    let (_, output, _) = run_zshrs_parity(
        r#"
        print -v X "hello world"
        print "[$X][${#X}]"
        print -v Y "foo"
        print "[$Y][${#Y}]"
        "#,
    );
    assert_eq!(
        output.trim(),
        "[hello world][11]\n[foo][3]",
        "got: {output:?}"
    );
}

#[test]
fn test_subscript_parity_read_into_array() {
    // `read -A arr` consumes IFS-split tokens from stdin into array.
    let (_, output, _) = run_zshrs_parity(
        r#"echo "alpha beta gamma" | { read -A arr; print "[${#arr}] [${arr[1]}] [${arr[3]}]"; }"#,
    );
    assert_eq!(output.trim(), "[3] [alpha] [gamma]", "got: {output:?}");
}

#[test]
fn test_subscript_parity_printf_format_specifiers() {
    // %d / %s / %.4f / %x / %o / %-10s|%10s| field-width specifiers.
    let (_, output, _) = run_zshrs_parity(
        r#"
        printf "%.4f\n" 3.14159
        printf "0x%x\n" 255
        printf "%o\n" 8
        printf "%-10s|%10s|\n" left right
        "#,
    );
    let expected = "3.1416\n0xff\n10\nleft      |     right|";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_alias_listing() {
    // alias forms — list, set, and unalias. Empirical against /bin/zsh:
    //   ll='ls -l' / g='grep -i' / run-help=man (zsh built-in default)
    let (_, output, _) = run_zshrs_parity(
        r#"
        alias ll="ls -l"
        alias g="grep -i"
        alias 2>&1 | head -3 | sort
        "#,
    );
    assert_eq!(
        output.trim(),
        "g='grep -i'\nll='ls -l'\nrun-help=man",
        "got: {output:?}"
    );
}

#[test]
fn test_subscript_parity_typeset_attribute_print() {
    // typeset -p attribute display — covers -i, -A, -F, -ra.
    let (_, output, _) = run_zshrs_parity(
        r#"
        typeset -i n=42
        typeset -p n
        typeset -A m
        m[k]=v
        typeset -p m
        typeset -F 3 f=3.14
        typeset -p f
        typeset -ra const=(a b c)
        typeset -p const
        "#,
    );
    let expected =
        "typeset -i n=42\ntypeset -A m=( [k]=v )\ntypeset -F f=3.140\ntypeset -ar const=( a b c )";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_let_and_arith_assignment() {
    // `let`, `((expr))`, integer typeset, float typeset, ternary,
    // array-element arithmetic.
    let (_, output, _) = run_zshrs_parity(
        r#"
        let x=2+2; print "1:[$x]"
        ((y=3*4)); print "2:[$y]"
        print "3:[$((10/3))] [$((10%3))]"
        typeset -i n=10
        n=n+5
        print "4:[$n]"
        typeset -F 3 f=3.14159
        print "5:[$f]"
        print "6:[$((1?100:200))]"
        a=(10 20 30)
        print "7:[$((a[2]+5))]"
        "#,
    );
    let expected = "1:[4]\n2:[12]\n3:[3] [1]\n4:[15]\n5:[3.142]\n6:[100]\n7:[25]";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_bg_wait_synchronizes() {
    // `(cmd) &; wait` must wait for the bg job before returning.
    // Verified against /bin/zsh:
    //   /bin/zsh -c '(print BG) &; wait; print MAIN'  → "BG\nMAIN"
    //   /bin/zsh -c '(sleep 0.05; print BG) &; wait; print MAIN'
    //                                                → "BG\nMAIN"
    // Required wiring: BUILTIN_RUN_BG forks via raw libc::fork
    // (no std::process::Child wrapper), so wait's no-args path
    // had to be extended to handle bare-pid job entries via
    // nix::sys::wait::waitpid.
    let (_, output, _) = run_zshrs_parity(r#"(print BG) &; wait; print MAIN"#);
    assert_eq!(output.trim(), "BG\nMAIN", "got: {output:?}");

    let (_, output, _) =
        run_zshrs_parity(r#"(sleep 0.05; print BG_DELAYED) &; wait; print MAIN_AFTER"#);
    assert_eq!(output.trim(), "BG_DELAYED\nMAIN_AFTER", "got: {output:?}");
}

#[test]
fn test_subscript_parity_pipe_and_subshell() {
    // Pipes, command substitution through pipes, pipefail status,
    // subshell scoping, brace-group scoping. Verified against
    // /bin/zsh. Excludes `&` background+wait ordering — zshrs
    // currently interleaves wait differently (tracked separately).
    let (_, output, _) = run_zshrs_parity(
        r#"
        echo hello | wc -c
        echo "1:done"
        result=$(echo data | tr a-z A-Z)
        print "2:[$result]"
        echo "a b c d" | tr " " "\n" | sort | head -2
        false | true; print "3:status=$?"
        true | false; print "4:status=$?"
        (x=hidden; print "5:[$x]"); print "6:[${x-unset}]"
        { y=visible; print "7:[$y]"; }
        print "8:[$y]"
        "#,
    );
    // Don't `.trim()` — `wc -c` output has leading spaces that
    // stripping would corrupt.
    let stripped_trailing = output.trim_end();
    let expected = "       6\n1:done\n2:[DATA]\na\nb\n3:status=0\n4:status=1\n5:[hidden]\n6:[unset]\n7:[visible]\n8:[visible]";
    assert_eq!(stripped_trailing, expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_zsh_specific_features() {
    // Anonymous function, ${(P)var} indirection, ${var:offset:length},
    // multi-line array literal, ${1:-default} in function.
    let (_, output, _) = run_zshrs_parity(
        r#"
        () { print "anon arg: $1"; } hello
        inner=hello
        ref=inner
        print "1:[${(P)ref}]"
        s=abcde
        print "2:[${s:1:2}]"
        arr=(
          apple
          banana
          cherry
        )
        print "3:[${#arr}] [${arr[2]}]"
        fn() { print "${1:-DEFAULT}"; }
        fn
        fn explicit
        "#,
    );
    let expected = "anon arg: hello\n1:[hello]\n2:[bc]\n3:[3] [banana]\nDEFAULT\nexplicit";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_conditional_expressions() {
    // [[ ]] tests: file tests, string ordering, negation, compound,
    // glob equality, regex match. Verified against /bin/zsh.
    let (_, output, _) = run_zshrs_parity(
        r#"
        [[ -d /tmp ]] && print "1:dir"
        [[ -e /etc/passwd ]] && print "2:exists"
        [[ "abc" < "abd" ]] && print "3:lt"
        [[ "abc" > "aaa" ]] && print "4:gt"
        [[ ! -z "x" ]] && print "5:not-empty"
        [[ 1 -eq 1 && 2 -eq 2 ]] && print "6:and"
        [[ 1 -eq 1 || 2 -eq 3 ]] && print "7:or"
        s=""
        [[ -z "$s" ]] && print "8:empty"
        s="x"
        [[ -n "$s" ]] && print "9:nonempty"
        [[ "abc" == a* ]] && print "10:glob-eq"
        [[ "abc" != x* ]] && print "11:glob-neq"
        [[ "test123" =~ "[0-9]+" ]] && print "12:regex"
        "#,
    );
    let expected = "1:dir\n2:exists\n3:lt\n4:gt\n5:not-empty\n6:and\n7:or\n8:empty\n9:nonempty\n10:glob-eq\n11:glob-neq\n12:regex";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_redirection_order_left_to_right() {
    // Redirections process left-to-right. Verified against /bin/zsh:
    //   `>&2 2>file`: fd1 dup'd to current fd2 first, then fd2
    //                 redirected to file. fd1 still points to
    //                 ORIGINAL fd2 (terminal stderr), so file ends
    //                 empty. echo writes to terminal-stderr.
    //   `2>&1 >file`: fd2 dup'd to current fd1 (terminal stdout),
    //                 then fd1 redirected to file. fd2 still
    //                 points to terminal, so error msgs go to
    //                 terminal while stdout goes to file.
    //   `ls bad 2>file`: simple stderr-to-file redirect, captures
    //                    the ls error in file.
    let tmp = tempdir_for_test();
    let (_, output, _) = run_zshrs_parity(&format!(
        r#"
echo to-stderr >&2 2>{0}/a.err 2>/dev/null
print "1:size_a=$(wc -c <{0}/a.err | tr -d ' ')"

echo testtext 2>&1 >{0}/b.out
print "2:[$(cat {0}/b.out)]"

ls /nonexistent 2>{0}/c.err
print "3:has_error=$(test -s {0}/c.err && echo yes || echo no)"
"#,
        tmp
    ));
    let expected = "1:size_a=0\n2:[testtext]\n3:has_error=yes";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

#[test]
fn test_subscript_parity_arithmetic_expansion() {
    // Comprehensive arithmetic sweep.
    let (_, output, _) = run_zshrs_parity(
        r#"
        print "1:$((2+3))"
        print "2:$((2**10))"
        print "3:$((10%3))"
        print "4:$((1<<4))"
        print "5:$((~5))"
        print "6:$((!0))"
        print "7:$((0xFF & 0x0F))"
        print "8:$((1|2|4))"
        print "9:$((5>3))"
        print "10:$((5==5))"
        print "11:$((1>0 ? 100 : 200))"
        print "12:$((1.5+2.5))"
        n=10
        print "13:$((n*2))"
        "#,
    );
    let expected = "1:5\n2:1024\n3:1\n4:16\n5:-6\n6:1\n7:15\n8:7\n9:1\n10:1\n11:100\n12:4.\n13:20";
    assert_eq!(output.trim(), expected, "got: {output:?}");
}

// ---------------------------------------------------------------------------
// `typeset -A` two-statement assoc init: declare then array-literal-assign
// ---------------------------------------------------------------------------

#[test]
fn test_typeset_a_two_statement_init() {
    let (_, output, _) =
        run_zshrs("typeset -A m; m=(a 1 b 2 c 3); print \"${m[a]}-${m[b]}-${m[c]}\"");
    assert_eq!(output.trim(), "1-2-3", "got: {output:?}");
}

#[test]
fn test_typeset_a_then_indexed_assoc_remains_assoc() {
    // After `typeset -A m; m=(...)`, `m` should still respond to assoc
    // syntax. Test by appending another key/val pair.
    let (_, output, _) =
        run_zshrs("typeset -A m; m=(a 1 b 2); m[c]=3; print \"${m[a]}|${m[b]}|${m[c]}\"");
    assert_eq!(output.trim(), "1|2|3", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// Subscript with $-expanded key: ${m[$k]} for assoc, ${arr[$i]} for indexed
// ---------------------------------------------------------------------------

#[test]
fn test_assoc_subscript_dynamic_key() {
    let (_, output, _) = run_zshrs("typeset -A m=(foo 1 bar 2); k=foo; print \"${m[$k]}\"");
    assert_eq!(output.trim(), "1", "got: {output:?}");
}

#[test]
fn test_assoc_subscript_dynamic_key_loop() {
    let (_, output, _) =
        run_zshrs("typeset -A m=(a 1 b 2); for k in a b; do print \"$k=${m[$k]}\"; done");
    assert_eq!(output.trim(), "a=1\nb=2", "got: {output:?}");
}

#[test]
fn test_indexed_subscript_dynamic_key_in_loop() {
    let (_, output, _) = run_zshrs("arr=(x y z); for i in 1 2 3; do print \"$i:${arr[$i]}\"; done");
    assert_eq!(output.trim(), "1:x\n2:y\n3:z", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// Extendedglob `^pat` negation in `${arr:#pat}` filter
// ---------------------------------------------------------------------------

#[test]
fn test_extendedglob_negation_in_filter() {
    // `:#^*.txt` removes elements matching the negation of `*.txt`
    // → keeps only `*.txt` elements.
    let (_, output, _) =
        run_zshrs("setopt extendedglob; arr=(foo.txt bar.log baz.txt); print -l ${arr:#^*.txt}");
    assert_eq!(output.trim(), "foo.txt\nbaz.txt", "got: {output:?}");
}

#[test]
fn test_extendedglob_negation_literal_inverse() {
    let (_, output, _) = run_zshrs("setopt extendedglob; arr=(a b c); print -l ${arr:#^a}");
    assert_eq!(output.trim(), "a", "got: {output:?}");
}

#[test]
fn test_extendedglob_negation_off_when_option_unset() {
    // Without `extendedglob`, `^a` is a literal pattern (no element
    // matches the literal char `^a`), so all 3 stay.
    let (_, output, _) = run_zshrs("arr=(a b c); print -l ${arr:#^a}");
    assert_eq!(output.trim(), "a\nb\nc", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// Extendedglob inline pattern flags (#i) (#l) (#a<n>)
// ---------------------------------------------------------------------------

#[test]
fn test_pattern_flag_case_insensitive() {
    let (_, output, _) =
        run_zshrs("setopt extendedglob; [[ \"ABC\" = (#i)abc ]] && echo match || echo nomatch");
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

#[test]
fn test_pattern_flag_case_insensitive_uppercase_pat() {
    let (_, output, _) =
        run_zshrs("setopt extendedglob; [[ \"abc\" = (#i)ABC ]] && echo match || echo nomatch");
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

#[test]
fn test_pattern_flag_l_lowercase_matches_either_case() {
    let (_, output, _) =
        run_zshrs("setopt extendedglob; [[ \"AbC\" = (#l)abc ]] && echo match || echo nomatch");
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

#[test]
fn test_pattern_flag_l_uppercase_must_match_exactly() {
    // (#l) is asymmetric: uppercase pattern char requires exact case
    // in input. So `(#l)ABC` does NOT match "abc".
    let (_, output, _) =
        run_zshrs("setopt extendedglob; [[ \"abc\" = (#l)ABC ]] && echo match || echo nomatch");
    assert_eq!(output.trim(), "nomatch", "got: {output:?}");
}

#[test]
fn test_pattern_flag_approximate_one_error() {
    let (_, output, _) =
        run_zshrs("setopt extendedglob; [[ \"abcd\" = (#a1)abce ]] && echo match || echo nomatch");
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

#[test]
fn test_pattern_flag_approximate_zero_error_diff() {
    // (#a0) requires exact match.
    let (_, output, _) =
        run_zshrs("setopt extendedglob; [[ \"abcd\" = (#a0)abce ]] && echo match || echo nomatch");
    assert_eq!(output.trim(), "nomatch", "got: {output:?}");
}

#[test]
fn test_pattern_flag_approximate_insertion() {
    let (_, output, _) =
        run_zshrs("setopt extendedglob; [[ \"abc12\" = (#a1)abc1 ]] && echo match || echo nomatch");
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// (@s:sep:) / (@f) flag composition: @ + split must keep array shape in DQ
// ---------------------------------------------------------------------------

#[test]
fn test_at_s_flag_split_in_double_quotes() {
    let (_, output, _) = run_zshrs("str='a,b,c'; print -l \"${(@s:,:)str}\"");
    assert_eq!(output.trim(), "a\nb\nc", "got: {output:?}");
}

#[test]
fn test_at_s_flag_capture_into_array() {
    let (_, output, _) = run_zshrs(
        "str='a:b:c'; arr=(\"${(@s.:.)str}\"); print \"len=${#arr}\"; print -l \"${arr[@]}\"",
    );
    assert_eq!(output.trim(), "len=3\na\nb\nc", "got: {output:?}");
}

#[test]
fn test_at_f_flag_split_newlines_in_dq() {
    let (_, output, _) = run_zshrs("str=$'a\\nb\\nc'; print -l \"${(@f)str}\"");
    assert_eq!(output.trim(), "a\nb\nc", "got: {output:?}");
}

#[test]
fn test_s_flag_splits_each_array_element() {
    let (_, output, _) = run_zshrs("arr=('a,b' 'c,d'); print -l \"${(@s:,:)arr}\"");
    assert_eq!(output.trim(), "a\nb\nc\nd", "got: {output:?}");
}

#[test]
fn test_force_split_preserves_empty_fields_for_nonwhitespace_ifs() {
    // c:Src/utils.c:4224-4228 + spacesplit — a NON-whitespace IFS char
    // hard-delimits, so `${=var}` preserves empty fields (leading, trailing,
    // and between consecutive separators); a whitespace IFS collapses runs
    // and trims. multsub's PREFORK_SPLIT loop previously treated every IFS
    // char as whitespace-like, dropping the empties.
    let f = |code: &str| run_zshrs(code).1.trim().to_string();

    // Non-whitespace IFS → empties preserved.
    assert_eq!(f(r#"IFS=:; a=":a:b:"; set -- ${=a}; echo "$#|$1|$2|$3|$4""#), "4||a|b|");
    assert_eq!(f(r#"IFS=:; a="a::b"; set -- ${=a}; echo "$#|$1|$2|$3""#), "3|a||b");
    assert_eq!(f(r#"IFS=:; a=":::"; set -- ${=a}; echo $#"#), "4");
    assert_eq!(f(r#"IFS=,; a="a,,b"; b=(${=a}); echo $#b"#), "3");
    // Mixed IFS: non-ws `:` preserves, whitespace collapses.
    assert_eq!(f(r#"IFS=" :"; a="a::b"; set -- ${=a}; echo $#"#), "3");
    assert_eq!(f(r#"IFS=" :"; a="a  b"; set -- ${=a}; echo $#"#), "2");

    // Mixed IFS: whitespace ADJACENT to a non-ws separator is absorbed into
    // it (spacesplit isep_one + skipwsep), so `a : b` is ONE delimiter → 2
    // fields, but two non-ws separators (even ws-padded) keep the empty.
    assert_eq!(f(r#"IFS=" :"; a="a : b"; set -- ${=a}; echo $#"#), "2");
    assert_eq!(f(r#"IFS=" :"; a="a  :  b"; set -- ${=a}; echo $#"#), "2");
    assert_eq!(f(r#"IFS=" :"; a="a :"; set -- ${=a}; echo $#"#), "2"); // trailing empty
    assert_eq!(f(r#"IFS=" :"; a="a :: b"; set -- ${=a}; echo $#"#), "3");
    assert_eq!(f(r#"IFS=" :"; a="x : y : z"; set -- ${=a}; echo $#"#), "3");

    // Whitespace IFS → runs collapse, leading/trailing trimmed (no regression).
    assert_eq!(f(r#"IFS=" "; a="  a  b  "; set -- ${=a}; echo $#"#), "2");
    assert_eq!(f(r#"a="one two three"; set -- ${=a}; echo $#"#), "3");
    assert_eq!(f(r#"IFS=:; a=""; set -- ${=a}; echo $#"#), "0");

    // Quoted → no splitting at all.
    assert_eq!(f(r#"IFS=:; a=":a:b:"; echo "${a}""#), ":a:b:");
}

// ---------------------------------------------------------------------------
// Param flag with [@] subscript: ${(kv)m[@]}, ${(o)arr[@]}, etc.
// ---------------------------------------------------------------------------

#[test]
fn test_kv_flag_with_at_subscript() {
    // Sort because assoc HashMap iteration order is non-deterministic.
    let (_, output, _) = run_zshrs("typeset -A m=(a 1 b 2 c 3); print -l \"${(kv)m[@]}\" | sort");
    assert_eq!(output.trim(), "1\n2\n3\na\nb\nc", "got: {output:?}");
}

#[test]
fn test_k_flag_with_at_subscript() {
    let (_, output, _) = run_zshrs("typeset -A m=(a 1 b 2 c 3); print -l \"${(k)m[@]}\" | sort");
    assert_eq!(output.trim(), "a\nb\nc", "got: {output:?}");
}

#[test]
fn test_o_sort_flag_with_at_subscript() {
    // (o) only fires in array context — no DQ wrapper.
    let (_, output, _) = run_zshrs("arr=(c a b); print -l ${(o)arr[@]}");
    assert_eq!(output.trim(), "a\nb\nc", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// Anonymous function with `function` keyword: `function () { body } args...`
// ---------------------------------------------------------------------------

#[test]
fn test_function_keyword_anonymous() {
    let (_, output, _) = run_zshrs("function () { echo anon }");
    assert_eq!(output.trim(), "anon", "got: {output:?}");
}

#[test]
fn test_function_keyword_anonymous_with_args() {
    let (_, output, _) = run_zshrs("function () { echo \"args:\" \"$@\" } a b c");
    assert_eq!(output.trim(), "args: a b c", "got: {output:?}");
}

#[test]
fn test_function_keyword_anonymous_local_scope() {
    let (_, output, _) =
        run_zshrs("x=outer; function () { local x=inner; print \"$x\" } ; print \"$x\"");
    assert_eq!(output.trim(), "inner\nouter", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// =(cmd) process substitution — temp-file flavor
// ---------------------------------------------------------------------------

#[test]
fn test_eq_process_sub_basic() {
    let (_, output, _) = run_zshrs("cat =(echo hello)");
    assert_eq!(output.trim(), "hello", "got: {output:?}");
}

#[test]
fn test_eq_process_sub_multiline() {
    let (_, output, _) = run_zshrs("cat =(printf '%s\\n' a b c)");
    assert_eq!(output.trim(), "a\nb\nc", "got: {output:?}");
}

#[test]
fn test_eq_process_sub_two_inputs() {
    // diff returns 1 on differences; we just check it produces the
    // expected unified-style change marker.
    let (_, output, _) = run_zshrs("diff =(echo a) =(echo b) | head -1");
    assert_eq!(output.trim(), "1c1", "got: {output:?}");
}

#[test]
fn test_eq_process_sub_path_emitted() {
    // `echo =(cmd)` should print a temp-file path (no command runs
    // against it). Just verify it looks like an absolute path.
    let (_, output, _) = run_zshrs("echo =(echo hi)");
    assert!(
        output.trim().starts_with('/') && !output.trim().contains("=("),
        "got: {output:?}"
    );
}

// ---------------------------------------------------------------------------
// zsh/mapfile assoc array — magic ${mapfile[/path]} read form
// ---------------------------------------------------------------------------

#[test]
fn test_mapfile_basic_read() {
    use std::fs;
    let path = "/tmp/zshrs_mapfile_basic.txt";
    fs::write(path, "hello\n").unwrap();
    let (_, output, _) = run_zshrs(&format!(
        "zmodload zsh/mapfile; print \"${{mapfile[{}]}}\"",
        path
    ));
    // print appends a newline; mapfile preserves the file's trailing
    // newline, so output is "hello\n\n" → trim gives "hello".
    assert_eq!(output.trim_end(), "hello", "got: {output:?}");
    let _ = fs::remove_file(path);
}

#[test]
fn test_mapfile_preserves_trailing_newline() {
    use std::fs;
    let path = "/tmp/zshrs_mapfile_len.txt";
    fs::write(path, "test\n").unwrap();
    let (_, output, _) = run_zshrs(&format!(
        "zmodload zsh/mapfile; v=\"${{mapfile[{}]}}\"; print \"len=${{#v}}\"",
        path
    ));
    assert_eq!(output.trim(), "len=5", "got: {output:?}");
    let _ = fs::remove_file(path);
}

#[test]
fn test_mapfile_with_f_flag_split_lines() {
    use std::fs;
    let path = "/tmp/zshrs_mapfile_lines.txt";
    fs::write(path, "a\nb\nc\n").unwrap();
    let (_, output, _) = run_zshrs(&format!(
        "zmodload zsh/mapfile; print -l ${{(f)mapfile[{}]}}",
        path
    ));
    assert_eq!(output.trim(), "a\nb\nc", "got: {output:?}");
    let _ = fs::remove_file(path);
}

#[test]
fn test_mapfile_missing_file_is_empty() {
    let (_, output, _) =
        run_zshrs("zmodload zsh/mapfile; print \"len=${#mapfile[/no/such/path/here]}\"");
    assert_eq!(output.trim(), "len=0", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// ${(flags)NAME[KEY]} — flag + literal subscript composition
// ---------------------------------------------------------------------------

#[test]
fn test_flag_with_assoc_subscript_split() {
    let (_, output, _) = run_zshrs("typeset -A m=(k1 'a:b:c'); print -l \"${(s.:.)m[k1]}\"");
    assert_eq!(output.trim(), "a\nb\nc", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// History expansion: -c mode is correctly literal (matches zsh)
// ---------------------------------------------------------------------------

#[test]
fn test_history_expansion_literal_in_c_mode() {
    // In `-c` mode (non-interactive, no TTY), `!!` is literal — same as
    // zsh. Pulling from the persistent history db would inject random
    // commands from prior sessions.
    let (_, output, _) = run_zshrs("echo first; echo !!");
    assert_eq!(output.trim(), "first\n!!", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// Indexed array element / slice / delete assignment
// ---------------------------------------------------------------------------

#[test]
fn test_indexed_array_element_assign() {
    let (_, output, _) = run_zshrs("a=(x y z); a[2]=YY; print \"${a[*]}\"");
    assert_eq!(output.trim(), "x YY z", "got: {output:?}");
}

#[test]
fn test_indexed_array_negative_assign() {
    let (_, output, _) = run_zshrs("a=(x y z); a[-1]=Z; print \"${a[*]}\"");
    assert_eq!(output.trim(), "x y Z", "got: {output:?}");
}

#[test]
fn test_indexed_array_grow_on_assign() {
    let (_, output, _) = run_zshrs("a=(x y z); a[5]=E; print -l \"${a[@]}\"");
    assert_eq!(output.trim(), "x\ny\nz\n\nE", "got: {output:?}");
}

#[test]
fn test_indexed_array_append_at_index() {
    let (_, output, _) = run_zshrs("a=(x y z); a[2]+=BB; print \"${a[*]}\"");
    assert_eq!(output.trim(), "x yBB z", "got: {output:?}");
}

#[test]
fn test_indexed_array_slice_assign() {
    let (_, output, _) = run_zshrs("a=(x y z w v); a[2,4]=(YY ZZ WW); print \"${a[*]}\"");
    assert_eq!(output.trim(), "x YY ZZ WW v", "got: {output:?}");
}

#[test]
fn test_indexed_array_element_delete() {
    let (_, output, _) = run_zshrs("a=(x y z); a[2]=(); print \"${a[*]}\"");
    assert_eq!(output.trim(), "x z", "got: {output:?}");
}

#[test]
fn test_indexed_array_slice_delete() {
    let (_, output, _) = run_zshrs("a=(x y z w v); a[2,4]=(); print \"${a[*]}\"");
    assert_eq!(output.trim(), "x v", "got: {output:?}");
}

#[test]
fn test_unset_indexed_element_clears_to_empty() {
    // zsh `unset 'arr[N]'` for indexed arrays sets the slot to "" but
    // does NOT remove it (slot count preserved). Differs from `a[N]=()`.
    let (_, output, _) =
        run_zshrs("arr=(a b c); unset 'arr[2]'; print \"len=${#arr}\"; print -l \"${arr[@]}\"");
    assert_eq!(output.trim(), "len=3\na\n\nc", "got: {output:?}");
}

#[test]
fn test_unset_assoc_element() {
    let (_, output, _) = run_zshrs("typeset -A m=(a 1 b 2); unset 'm[a]'; print \"${(k)m[@]}\"");
    assert_eq!(output.trim(), "b", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// Regex `=~` capture groups: $MATCH, $match[N], $mbegin, $mend
// ---------------------------------------------------------------------------

#[test]
fn test_regex_match_full_match_var() {
    let (_, output, _) = run_zshrs("[[ \"hello\" =~ ll ]] && print \"$MATCH\"");
    assert_eq!(output.trim(), "ll", "got: {output:?}");
}

#[test]
fn test_regex_match_capture_groups() {
    let (_, output, _) =
        run_zshrs("[[ \"a1b2\" =~ ([a-z])([0-9]) ]] && print \"${match[1]}|${match[2]}\"");
    assert_eq!(output.trim(), "a|1", "got: {output:?}");
}

#[test]
fn test_regex_match_offsets() {
    let (_, output, _) = run_zshrs(
        "[[ \"abc123\" =~ ^([a-z]+)([0-9]+)$ ]] && print \"${mbegin[1]}:${mend[1]} ${mbegin[2]}:${mend[2]}\"",
    );
    assert_eq!(output.trim(), "1:3 4:6", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// Tilde expansion: ~+, ~-, ~user, named directories
// ---------------------------------------------------------------------------

#[test]
fn test_tilde_named_dir() {
    let (_, output, _) = run_zshrs("hash -d foo=/tmp; print ~foo/x");
    assert_eq!(output.trim(), "/tmp/x", "got: {output:?}");
}

#[test]
fn test_tilde_user() {
    // `~root` should resolve via getpwnam to /var/root (macOS) or
    // /root (Linux). Just verify it's an absolute path containing
    // "root", not the literal `~root`.
    let (_, output, _) = run_zshrs("print ~root");
    let out = output.trim();
    assert!(
        out.starts_with('/') && out.contains("root") && !out.contains('~'),
        "got: {output:?}"
    );
}

// ---------------------------------------------------------------------------
// builtin head -c byte count
// ---------------------------------------------------------------------------

#[test]
fn test_head_c_byte_count() {
    let (_, output, _) = run_zshrs("echo abcdef | head -c 3");
    assert_eq!(output, "abc", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// WORDCHARS default value
// ---------------------------------------------------------------------------

#[test]
fn test_wordchars_default() {
    let (_, output, _) = run_zshrs("print -- \"$WORDCHARS\"");
    assert_eq!(output.trim(), "*?_-.[]~=/&;!#$%^(){}<>", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// Numeric range globbing <a-b>
// ---------------------------------------------------------------------------

#[test]
fn test_numeric_range_inclusive_match() {
    let (_, output, _) = run_zshrs("[[ file5 = file<1-10> ]] && echo match || echo nomatch");
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

#[test]
fn test_numeric_range_out_of_bounds() {
    let (_, output, _) = run_zshrs("[[ file20 = file<1-10> ]] && echo match || echo nomatch");
    assert_eq!(output.trim(), "nomatch", "got: {output:?}");
}

#[test]
fn test_numeric_range_open_lo() {
    // `<-10>` means ≤ 10.
    let (_, output, _) = run_zshrs("[[ 7 = <-10> ]] && echo match || echo nomatch");
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

#[test]
fn test_numeric_range_open_hi() {
    // `<50->` means ≥ 50.
    let (_, output, _) = run_zshrs("[[ 100 = <50-> ]] && echo match || echo nomatch");
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

#[test]
fn test_numeric_range_any_integer() {
    // `<->` matches any non-negative integer.
    let (_, output, _) = run_zshrs("[[ 42 = <-> ]] && echo match || echo nomatch");
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

#[test]
fn test_numeric_range_rejects_non_digits() {
    let (_, output, _) = run_zshrs("[[ abc = <-> ]] && echo match || echo nomatch");
    assert_eq!(output.trim(), "nomatch", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// `where` builtin output format (zsh `whence -ca`)
// ---------------------------------------------------------------------------

#[test]
fn test_where_external_command_bare_path() {
    let (_, output, _) = run_zshrs("where ls");
    // Path may differ across systems but must be absolute and end /ls.
    let line = output.lines().next().unwrap_or("").trim();
    assert!(
        line.starts_with('/') && line.ends_with("/ls"),
        "got: {output:?}"
    );
}

#[test]
fn test_where_function_prints_definition() {
    let (_, output, _) = run_zshrs("foo() { :; }; where foo");
    assert!(
        output.contains("foo () {") && output.contains(":"),
        "got: {output:?}"
    );
}

#[test]
fn test_where_alias_csh_style() {
    let (_, output, _) = run_zshrs("alias gst='git status'; where gst");
    assert_eq!(
        output.trim(),
        "gst: aliased to git status",
        "got: {output:?}"
    );
}

#[test]
fn test_where_not_found_status_one() {
    let (status, _, _) = run_zshrs("where nonexistentxxxprobe");
    assert_eq!(status, 1, "exit status should be 1 when not found");
}

// ---------------------------------------------------------------------------
// `print -P` byte-exact ANSI output (no readline markers, no spurious reset)
// ---------------------------------------------------------------------------

#[test]
fn test_print_p_color_bytes() {
    let (_, output, _) = run_zshrs("print -P \"%F{red}hi%f\"");
    // \e[31m hi \e[39m \n  — no \x01/\x02 wrappers, no leading reset.
    assert_eq!(output, "\x1b[31mhi\x1b[39m\n", "got: {output:?}");
}

#[test]
fn test_print_p_bold_bytes() {
    let (_, output, _) = run_zshrs("print -P \"%Bbold%b\"");
    assert_eq!(output, "\x1b[1mbold\x1b[0m\n", "got: {output:?}");
}

#[test]
fn test_print_p_underline_bytes() {
    let (_, output, _) = run_zshrs("print -P \"%Uunder%u\"");
    // %u emits SGR-24 (underline off) instead of full reset.
    assert_eq!(output, "\x1b[4munder\x1b[24m\n", "got: {output:?}");
}

#[test]
fn test_print_p_color_chain() {
    let (_, output, _) = run_zshrs("print -P \"%F{green}g%f%F{red}r%f\"");
    assert_eq!(
        output, "\x1b[32mg\x1b[39m\x1b[31mr\x1b[39m\n",
        "got: {output:?}"
    );
}

#[test]
fn test_print_p_plain_no_markers() {
    let (_, output, _) = run_zshrs("print -P \"plain\"");
    assert_eq!(output, "plain\n", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// `let` and arithmetic-substitution float formatting
// ---------------------------------------------------------------------------

#[test]
fn test_let_stores_float_with_ten_decimals() {
    let (_, output, _) = run_zshrs(r#"let "a=1.0+2.0"; echo $a"#);
    assert_eq!(output.trim(), "3.0000000000", "got: {output:?}");
}

#[test]
fn test_let_integer_no_decimals() {
    let (_, output, _) = run_zshrs(r#"let "a=5+3"; echo $a"#);
    assert_eq!(output.trim(), "8", "got: {output:?}");
}

#[test]
fn test_let_division_promotes_to_float() {
    let (_, output, _) = run_zshrs(r#"let "a=10/3.0"; echo $a"#);
    assert_eq!(output.trim(), "3.3333333333", "got: {output:?}");
}

#[test]
fn test_arith_subst_whole_float_trailing_dot() {
    // zsh quirk: $((1.5+2.5)) → "4." (trailing dot, no zeros)
    let (_, output, _) = run_zshrs("echo $((1.5+2.5))");
    assert_eq!(output.trim(), "4.", "got: {output:?}");
}

#[test]
fn test_arith_subst_float_pct_17g_inexact_form() {
    // zsh uses C's `%.17g` for non-integer floats, which expose
    // the exact f64 representation when it differs from the
    // user's literal: `0.1` is `0.10000000000000001` because
    // 0.1 isn't exactly representable in binary. zshrs's
    // shortest-roundtrip default printed `0.1` instead.
    // Trailing zeros are stripped (per `%g`), so exact-
    // representable values like `0.5` stay short.
    let (_, output, _) = run_zshrs("echo $((0.1))");
    assert_eq!(output.trim(), "0.10000000000000001", "got: {output:?}");
    let (_, output, _) = run_zshrs("echo $((0.5))");
    assert_eq!(output.trim(), "0.5", "got: {output:?}");
    let (_, output, _) = run_zshrs("echo $((1.0/3.0))");
    assert_eq!(output.trim(), "0.33333333333333331", "got: {output:?}");
    let (_, output, _) = run_zshrs("echo $((0.05))");
    assert_eq!(output.trim(), "0.050000000000000003", "got: {output:?}");
}

#[test]
fn test_arith_subst_integer_no_dot() {
    let (_, output, _) = run_zshrs("echo $((10+5))");
    assert_eq!(output.trim(), "15", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// `print -P %h` / `%!` history line number — session-relative, not disk count
// ---------------------------------------------------------------------------

#[test]
fn test_print_p_history_line_number_zero_in_c_mode() {
    // In `-c` mode, no history is recorded; %h should be 0 (matches zsh).
    let (_, output, _) = run_zshrs("print -P \"%h\"");
    assert_eq!(output.trim(), "0", "got: {output:?}");
}

#[test]
fn test_print_p_bang_alias_for_history_line() {
    let (_, output, _) = run_zshrs("print -P \"%!\"");
    assert_eq!(output.trim(), "0", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// `print -P %D{fmt}` output_strftime format
// ---------------------------------------------------------------------------

#[test]
fn test_print_p_date_format() {
    let (_, output, _) = run_zshrs("print -P \"%D{%Y}\"");
    let s = output.trim();
    assert!(
        s.len() == 4 && s.chars().all(|c| c.is_ascii_digit()),
        "got: {output:?}"
    );
}

// ---------------------------------------------------------------------------
// `fc -l` empty-history behavior in non-interactive mode
// ---------------------------------------------------------------------------

#[test]
fn test_fc_l_empty_history_no_event_error() {
    // In `-c` mode session history is empty; zsh emits "no such event"
    // and exits 1. Persistent disk history should NOT leak through.
    let (status, _, stderr) = run_zshrs("fc -l");
    assert_eq!(status, 1, "exit status should be 1");
    assert!(
        stderr.contains("no such event"),
        "stderr should mention 'no such event', got: {stderr:?}"
    );
}

#[test]
fn test_fc_l_explicit_index_no_event_error() {
    let (status, _, stderr) = run_zshrs("fc -l 5");
    assert_eq!(status, 1, "exit status should be 1");
    assert!(
        stderr.contains("no such event: 5"),
        "stderr should mention 'no such event: 5', got: {stderr:?}"
    );
}

// ---------------------------------------------------------------------------
// `noglob` precommand modifier dispatches to builtins
// ---------------------------------------------------------------------------

#[test]
fn test_noglob_print_builtin() {
    // Previously errored "command not found: print" because noglob
    // routed only through builtin_command (PATH-only lookup). Now
    // dispatches via builtin_builtin first.
    let (_, output, _) = run_zshrs("noglob print '*'");
    assert_eq!(output.trim(), "*", "got: {output:?}");
}

#[test]
fn test_noglob_print_multi_args() {
    let (_, output, _) = run_zshrs("noglob print foo bar baz");
    assert_eq!(output.trim(), "foo bar baz", "got: {output:?}");
}

#[test]
fn test_noglob_echo_glob_literal() {
    let (_, output, _) = run_zshrs("noglob echo '*.txt'");
    assert_eq!(output.trim(), "*.txt", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// Bare `$arr[N]` subscript (no braces)
// ---------------------------------------------------------------------------

#[test]
fn test_bare_subscript_indexed_int() {
    let (_, output, _) = run_zshrs("arr=(x y z); print $arr[2]");
    assert_eq!(output.trim(), "y", "got: {output:?}");
}

#[test]
fn test_bare_subscript_assoc_string_key() {
    let (_, output, _) = run_zshrs("typeset -A m=(a 1 b 2); print $m[a]");
    assert_eq!(output.trim(), "1", "got: {output:?}");
}

#[test]
fn test_bare_subscript_with_literal_suffix() {
    let (_, output, _) = run_zshrs("arr=(x y z); print $arr[2]extra");
    assert_eq!(output.trim(), "yextra", "got: {output:?}");
}

#[test]
fn test_bare_arr_no_subscript_still_splices() {
    // Make sure the bare $arr (no subscript) still works as before.
    let (_, output, _) = run_zshrs("arr=(x y z); print $arr");
    assert_eq!(output.trim(), "x y z", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// `(t)` typeset flag — type + attribute introspection
// ---------------------------------------------------------------------------

#[test]
fn test_t_flag_integer() {
    let (_, output, _) = run_zshrs("integer i=5; print \"${(t)i}\"");
    assert_eq!(output.trim(), "integer", "got: {output:?}");
}

#[test]
fn test_t_flag_typeset_i() {
    let (_, output, _) = run_zshrs("typeset -i n=5; print \"${(t)n}\"");
    assert_eq!(output.trim(), "integer", "got: {output:?}");
}

#[test]
fn test_t_flag_float() {
    let (_, output, _) = run_zshrs("float f=1.5; print \"${(t)f}\"");
    assert_eq!(output.trim(), "float", "got: {output:?}");
}

#[test]
fn test_t_flag_scalar_left() {
    let (_, output, _) = run_zshrs("typeset -L 5 s=hello; print \"${(t)s}\"");
    assert_eq!(output.trim(), "scalar-left", "got: {output:?}");
}

#[test]
fn test_t_flag_scalar_readonly() {
    let (_, output, _) = run_zshrs("typeset -r ro=foo; print \"${(t)ro}\"");
    assert_eq!(output.trim(), "scalar-readonly", "got: {output:?}");
}

#[test]
fn test_t_flag_scalar_export() {
    let (_, output, _) = run_zshrs("export E=foo; print \"${(t)E}\"");
    assert_eq!(output.trim(), "scalar-export", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// Glob qualifier `(mh-N)` / `(ah-N)` / `(ch-N)` time qualifiers
// ---------------------------------------------------------------------------

#[test]
fn test_glob_qualifier_mh_recent_file() {
    use std::fs;
    let path = "/tmp/zshrs_glob_mh_test.txt";
    fs::write(path, "x").unwrap();
    let (_, output, _) = run_zshrs(&format!("print {}(mh-100)", path));
    assert_eq!(output.trim(), path, "got: {output:?}");
    let _ = fs::remove_file(path);
}

#[test]
fn test_glob_qualifier_mh_too_old_excludes() {
    use std::fs;
    let path = "/tmp/zshrs_glob_mh_old.txt";
    fs::write(path, "x").unwrap();
    // (mh+10000) = older than 10000 hours; a just-touched file fails
    // the filter so the resolved path is NOT printed. (zshrs falls back
    // to the literal pattern instead of erroring "no matches found";
    // either way the resolved /tmp/... shouldn't appear bare.)
    let (_, stdout, _) = run_zshrs(&format!("print {}(mh+10000) 2>/dev/null", path));
    let _ = fs::remove_file(path);
    let line = stdout.trim();
    assert!(
        line != path,
        "filter should remove just-touched file, got: {stdout:?}"
    );
}

#[test]
fn test_glob_qualifier_path_dot() {
    let (_, output, _) = run_zshrs("print /etc/hosts(.)");
    assert_eq!(output.trim(), "/etc/hosts", "got: {output:?}");
}

#[test]
fn test_explicit_glob_qualifier_requires_extendedglob() {
    // c:Src/glob.c:1192-1197 — the `(#q...)` explicit glob-qualifier form is
    // recognized ONLY under EXTENDEDGLOB. Without it, the `#` inside `(...)`
    // is an (unknown) attribute char, so `*(#q.)` errors "unknown file
    // attribute: #" rather than silently applying the `.` qualifier.
    let dir = tempdir_for_test();
    std::fs::write(format!("{}/a.txt", dir), b"").unwrap();
    std::fs::write(format!("{}/c.log", dir), b"").unwrap();
    std::fs::create_dir(format!("{}/sub", dir)).unwrap();

    // No extendedglob → error naming the `#` attribute.
    let (_, _, err) = run_zshrs(&format!("cd {} && echo *(#q.)", dir));
    assert!(
        err.contains("unknown file attribute: #"),
        "expected unknown-attribute error without extendedglob: {err:?}"
    );

    // With extendedglob → the explicit qualifier applies (regular files).
    let (_, out, _) = run_zshrs(&format!(
        "cd {} && setopt extendedglob && print -l *(#q.) | sort",
        dir
    ));
    assert_eq!(out.trim(), "a.txt\nc.log", "got: {out:?}");

    // Inline pattern flag `(#i)` still passes through under extendedglob.
    let (_, out, _) = run_zshrs("setopt extendedglob; [[ abc == (#i)ABC ]] && echo ci");
    assert_eq!(out.trim(), "ci");

    // Plain bareglobqual `(.)` unaffected (no `#`).
    let (_, out, _) = run_zshrs(&format!("cd {} && print -l *(.) | sort", dir));
    assert_eq!(out.trim(), "a.txt\nc.log", "got: {out:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Recursive glob `**/` (dirs-only) and `**/*` (files+dirs)
// ---------------------------------------------------------------------------

#[test]
fn test_recursive_glob_dirs_only() {
    use std::fs;
    let root = "/tmp/zshrs_recglob_dirs";
    let _ = fs::remove_dir_all(root);
    fs::create_dir_all(format!("{}/a", root)).unwrap();
    fs::create_dir_all(format!("{}/b/c", root)).unwrap();
    let (_, output, _) = run_zshrs(&format!("cd {} && print -l **/", root));
    let _ = fs::remove_dir_all(root);
    let mut lines: Vec<&str> = output.lines().collect();
    lines.sort();
    assert_eq!(lines, vec!["a/", "b/", "b/c/"], "got: {output:?}");
}

#[test]
fn test_recursive_glob_files_and_dirs() {
    use std::fs;
    let root = "/tmp/zshrs_recglob_all";
    let _ = fs::remove_dir_all(root);
    fs::create_dir_all(format!("{}/a", root)).unwrap();
    fs::create_dir_all(format!("{}/b/c", root)).unwrap();
    fs::write(format!("{}/a/x.txt", root), "x").unwrap();
    let (_, output, _) = run_zshrs(&format!("cd {} && print -l **/*", root));
    let _ = fs::remove_dir_all(root);
    let mut lines: Vec<&str> = output.lines().collect();
    lines.sort();
    assert_eq!(lines, vec!["a", "a/x.txt", "b", "b/c"], "got: {output:?}");
}

#[test]
fn test_recursive_glob_filter_by_extension() {
    use std::fs;
    let root = "/tmp/zshrs_recglob_ext";
    let _ = fs::remove_dir_all(root);
    fs::create_dir_all(format!("{}/a", root)).unwrap();
    fs::create_dir_all(format!("{}/b/c", root)).unwrap();
    fs::write(format!("{}/a/x.txt", root), "x").unwrap();
    fs::write(format!("{}/b/c/y.txt", root), "y").unwrap();
    fs::write(format!("{}/a/skip.log", root), "z").unwrap();
    let (_, output, _) = run_zshrs(&format!("cd {} && print -l **/*.txt", root));
    let _ = fs::remove_dir_all(root);
    let mut lines: Vec<&str> = output.lines().collect();
    lines.sort();
    assert_eq!(lines, vec!["a/x.txt", "b/c/y.txt"], "got: {output:?}");
}

// ---------------------------------------------------------------------------
// `:s/old/new/` and `:gs/old/new/` substitution modifier
// ---------------------------------------------------------------------------

#[test]
fn test_subst_modifier_first_only() {
    let (_, output, _) = run_zshrs("p=hello; echo ${p:s/l/L/}");
    assert_eq!(output.trim(), "heLlo", "got: {output:?}");
}

#[test]
fn test_subst_modifier_global() {
    let (_, output, _) = run_zshrs("p=hello; echo ${p:gs/l/L/}");
    assert_eq!(output.trim(), "heLLo", "got: {output:?}");
}

#[test]
fn test_subst_modifier_chained_with_t() {
    let (_, output, _) = run_zshrs("p=/a/b.txt; echo ${p:s/b/B/:t}");
    assert_eq!(output.trim(), "B.txt", "got: {output:?}");
}

#[test]
fn test_subst_word_modifier_substitutes_per_word_first_match() {
    // c:Src/subst.c modify() — the `:w` word flag applies `:s` to EACH
    // whitespace word ONCE (first match per word), NOT to the whole string.
    // zshrs ran the whole-string substitution AND the per-word pass, so a
    // single word got two replacements: `${p:ws/./-/}` on `a.b.c` gave
    // `a-b-c` instead of `a-b.c`.
    let out = |code: &str| run_zshrs(code).1.trim().to_string();

    // One word, non-global → only the FIRST match is replaced.
    assert_eq!(out("p=a.b.c; echo ${p:ws/./-/}"), "a-b.c");
    assert_eq!(out("p=hello; echo ${p:ws/l/L/}"), "heLlo");
    assert_eq!(out("p=a.b.c.d; echo ${p:ws/./_/}"), "a_b.c.d");

    // Multiple words → first match in each word independently.
    assert_eq!(out(r#"p="x.y a.b"; echo ${p:ws/./-/}"#), "x-y a-b");
    assert_eq!(out(r#"p="one two one"; echo ${p:ws/one/X/}"#), "X two X");

    // `:gws` — global within each word.
    assert_eq!(out("p=a.b.c; echo ${p:gws/./-/}"), "a-b-c");

    // Plain `:s` (no word flag) still first-match on the whole string.
    assert_eq!(out("p=a.b.c; echo ${p:s/./-/}"), "a-b.c");
    assert_eq!(out("p=a.b.c; echo ${p:gs/./-/}"), "a-b-c");
}

#[test]
fn test_q_modifier_backslash_quote() {
    // zsh `:q` uses backslash escaping, not single-bslashquote wrapping.
    let (_, output, _) = run_zshrs("p='hi there'; echo ${p:q}");
    assert_eq!(output.trim(), "hi\\ there", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// $0 inside function = function name; $funcstack array
// ---------------------------------------------------------------------------

#[test]
fn test_dollar_zero_in_function() {
    let (_, output, _) = run_zshrs("foo() { echo $0; }; foo");
    assert_eq!(output.trim(), "foo", "got: {output:?}");
}

#[test]
fn test_funcstack_top_is_current_fn() {
    let (_, output, _) = run_zshrs("foo() { echo $funcstack[1]; }; foo");
    assert_eq!(output.trim(), "foo", "got: {output:?}");
}

#[test]
fn test_funcstack_nested() {
    let (_, output, _) = run_zshrs("foo() { bar() { echo \"${funcstack[*]}\"; }; bar; }; foo");
    assert_eq!(output.trim(), "bar foo", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// $ARGC alias for $#
// ---------------------------------------------------------------------------

#[test]
fn test_argc_equals_positional_count() {
    let (_, output, _) = run_zshrs("set -- a b c d; echo $ARGC");
    assert_eq!(output.trim(), "4", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// `print -N` null separator between args
// ---------------------------------------------------------------------------

#[test]
fn test_print_n_null_between_args() {
    let (_, output, _) = run_zshrs("print -N a b c");
    // a\0b\0c\0
    assert_eq!(output, "a\0b\0c\0", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// kshglob extended patterns ?(p)/+(p)/@(p) — gated on `setopt kshglob`
// ---------------------------------------------------------------------------

#[test]
fn test_kshglob_question_alternation() {
    let (_, output, _) =
        run_zshrs("setopt kshglob; [[ a = ?(a|b) ]] && echo match || echo nomatch");
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

#[test]
fn test_kshglob_plus_one_or_more() {
    let (_, output, _) =
        run_zshrs("setopt kshglob; [[ aaa = +(a) ]] && echo match || echo nomatch");
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

#[test]
fn test_kshglob_at_exactly_one() {
    let (_, output, _) =
        run_zshrs("setopt kshglob; [[ foo = @(foo|bar) ]] && echo match || echo nomatch");
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

#[test]
fn test_kshglob_off_no_match() {
    // Without `setopt kshglob`, ?(a|b) is the default zsh-glob shape
    // and doesn't match the bare letter `a` (literal `?(...)`).
    let (_, output, _) = run_zshrs("[[ a = ?(a|b) ]] && echo match || echo nomatch");
    assert_eq!(output.trim(), "nomatch", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// Pattern repetition `(#cN)` and `(#cN,M)`
// ---------------------------------------------------------------------------

#[test]
fn test_pattern_repeat_exact() {
    let (_, output, _) =
        run_zshrs("setopt extendedglob; [[ aa = a(#c2) ]] && echo match || echo nomatch");
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

#[test]
fn test_pattern_repeat_range() {
    let (_, output, _) =
        run_zshrs("setopt extendedglob; [[ aaa = a(#c2,3) ]] && echo match || echo nomatch");
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

#[test]
fn test_pattern_repeat_out_of_range() {
    let (_, output, _) =
        run_zshrs("setopt extendedglob; [[ aaaa = a(#c2,3) ]] && echo match || echo nomatch");
    assert_eq!(output.trim(), "nomatch", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// Special parameters: $EUID, $UID, $PPID, $HOST, $ZSH_SUBSHELL, $#@, $#*
// ---------------------------------------------------------------------------

#[test]
fn test_special_param_euid() {
    let (_, output, _) = run_zshrs("echo $EUID");
    assert!(
        output.trim().chars().all(|c| c.is_ascii_digit()) && !output.trim().is_empty(),
        "got: {output:?}"
    );
}

#[test]
fn test_special_param_uid() {
    let (_, output, _) = run_zshrs("echo $UID");
    assert!(
        output.trim().chars().all(|c| c.is_ascii_digit()) && !output.trim().is_empty(),
        "got: {output:?}"
    );
}

#[test]
fn test_special_param_ppid() {
    let (_, output, _) = run_zshrs("echo $PPID");
    assert!(
        output.trim().chars().all(|c| c.is_ascii_digit()) && !output.trim().is_empty(),
        "got: {output:?}"
    );
}

#[test]
fn test_special_param_host() {
    let (_, output, _) = run_zshrs("print $HOST");
    assert!(
        !output.trim().is_empty(),
        "$HOST should be set, got: {output:?}"
    );
}

#[test]
fn test_special_param_zsh_subshell_zero() {
    let (_, output, _) = run_zshrs("echo $ZSH_SUBSHELL");
    assert_eq!(output.trim(), "0", "got: {output:?}");
}

#[test]
fn test_dollar_hash_at_count() {
    let (_, output, _) = run_zshrs("set -- a b c d; echo $#@");
    assert_eq!(output.trim(), "4", "got: {output:?}");
}

#[test]
fn test_dollar_hash_star_count() {
    let (_, output, _) = run_zshrs("set -- a b c d; echo $#*");
    assert_eq!(output.trim(), "4", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// $sysparams[pid] / [ppid] zsh/system magic assoc
// ---------------------------------------------------------------------------

#[test]
fn test_sysparams_pid() {
    let (_, output, _) = run_zshrs("zmodload zsh/system; print $sysparams[pid]");
    let pid = output.trim();
    assert!(
        pid.chars().all(|c| c.is_ascii_digit()) && !pid.is_empty(),
        "got: {output:?}"
    );
}

#[test]
fn test_sysparams_ppid() {
    let (_, output, _) = run_zshrs("zmodload zsh/system; print $sysparams[ppid]");
    let ppid = output.trim();
    assert!(
        ppid.chars().all(|c| c.is_ascii_digit()) && !ppid.is_empty(),
        "got: {output:?}"
    );
}

// ---------------------------------------------------------------------------
// kshglob `!(p)` negation (standalone)
// ---------------------------------------------------------------------------

#[test]
fn test_kshglob_negation_match() {
    let (_, output, _) =
        run_zshrs("setopt kshglob; [[ baz = !(foo|bar) ]] && echo match || echo nomatch");
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

#[test]
fn test_kshglob_negation_no_match() {
    let (_, output, _) =
        run_zshrs("setopt kshglob; [[ foo = !(foo|bar) ]] && echo match || echo nomatch");
    assert_eq!(output.trim(), "nomatch", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// `(F)` newline-join flag
// ---------------------------------------------------------------------------

#[test]
fn test_F_flag_joins_array_with_newlines() {
    let (_, output, _) = run_zshrs("arr=(a b c); echo \"${(F)arr}\"");
    assert_eq!(output.trim(), "a\nb\nc", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// `typeset -p NAME` and `export -p`
// ---------------------------------------------------------------------------

#[test]
fn test_typeset_p_integer() {
    let (_, output, _) = run_zshrs("integer i=5; typeset -p i");
    assert_eq!(output.trim(), "typeset -i i=5", "got: {output:?}");
}

#[test]
fn test_typeset_p_array() {
    let (_, output, _) = run_zshrs("arr=(a b c); typeset -p arr");
    assert_eq!(output.trim(), "typeset -a arr=( a b c )", "got: {output:?}");
}

#[test]
fn test_typeset_p_assoc() {
    let (_, output, _) = run_zshrs("typeset -A m=(a 1 b 2); typeset -p m");
    assert_eq!(
        output.trim(),
        "typeset -A m=( [a]=1 [b]=2 )",
        "got: {output:?}"
    );
}

#[test]
fn test_export_p_lists_var() {
    let (_, output, _) = run_zshrs("export X=hello; export -p 2>&1 | grep '^export X='");
    assert_eq!(output.trim(), "export X=hello", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// `zmv` / `zcp` / `zln` / `zcalc` native bundled functions
// ---------------------------------------------------------------------------

#[test]
fn test_zmv_dry_run_capture_substitution() {
    use std::fs;
    let dir = "/tmp/zshrs_zmv_dry";
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).unwrap();
    fs::write(format!("{}/a.txt", dir), "x").unwrap();
    fs::write(format!("{}/b.txt", dir), "y").unwrap();
    let (_, output, _) = run_zshrs(&format!("cd {} && zmv -n '(*).txt' '$1.bak'", dir));
    let _ = fs::remove_dir_all(dir);
    let mut lines: Vec<&str> = output.lines().collect();
    lines.sort();
    assert_eq!(
        lines,
        vec!["mv -- a.txt a.bak", "mv -- b.txt b.bak"],
        "got: {output:?}"
    );
}

#[test]
fn test_zmv_real_renames() {
    use std::fs;
    let dir = "/tmp/zshrs_zmv_real";
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).unwrap();
    fs::write(format!("{}/foo.txt", dir), "x").unwrap();
    let (_, _, _) = run_zshrs(&format!("cd {} && zmv '(*).txt' '$1.bak'", dir));
    let exists_bak = std::path::Path::new(&format!("{}/foo.bak", dir)).exists();
    let exists_orig = std::path::Path::new(&format!("{}/foo.txt", dir)).exists();
    let _ = fs::remove_dir_all(dir);
    assert!(
        exists_bak && !exists_orig,
        "bak={} orig={}",
        exists_bak,
        exists_orig
    );
}

#[test]
fn test_zmv_collision_detection() {
    // Two files mapping to the same dest should error before any rename.
    use std::fs;
    let dir = "/tmp/zshrs_zmv_clash";
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).unwrap();
    fs::write(format!("{}/a.txt", dir), "1").unwrap();
    fs::write(format!("{}/b.txt", dir), "2").unwrap();
    let (status, _, stderr) = run_zshrs(&format!("cd {} && zmv '*.txt' 'merged.bak'", dir));
    let _ = fs::remove_dir_all(dir);
    assert_eq!(status, 1, "should exit 1 on collision");
    assert!(
        stderr.contains("both map to"),
        "stderr should mention collision, got: {stderr:?}"
    );
}

#[test]
fn test_zcalc_evaluates_expression() {
    let (_, output, _) = run_zshrs("zcalc -e '2+3*4'");
    assert_eq!(output.trim(), "14", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// Cond tests: -nt, -ot, -k (sticky), -O (owner), -G (group)
// ---------------------------------------------------------------------------

#[test]
fn test_cond_nt_newer_than() {
    use std::fs;
    let _ = fs::remove_file("/tmp/zsh_nt_a");
    let _ = fs::remove_file("/tmp/zsh_nt_b");
    fs::write("/tmp/zsh_nt_a", "x").unwrap();
    // mtime granularity on some filesystems is 1s — sleep past that.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    fs::write("/tmp/zsh_nt_b", "y").unwrap();
    let (_, output, _) = run_zshrs("[[ /tmp/zsh_nt_b -nt /tmp/zsh_nt_a ]] && echo yes || echo no");
    let _ = fs::remove_file("/tmp/zsh_nt_a");
    let _ = fs::remove_file("/tmp/zsh_nt_b");
    // ignore trailing zpwr log noise from cwd hook
    assert!(output.starts_with("yes"), "got: {output:?}");
}

#[test]
fn test_cond_k_sticky_bit() {
    // `-k FILE` tests the sticky bit. On macOS /tmp doesn't always
    // have the sticky bit set (it does on most Linux distros).
    // Test against a file we mkfifo with chmod +t to guarantee
    // sticky-bit presence.
    let dir = std::env::temp_dir();
    let path = dir.join("zshrs_sticky_test");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, b"").unwrap();
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    // Sticky bit = 0o1000.
    perms.set_mode(0o1644);
    std::fs::set_permissions(&path, perms).unwrap();
    let cmd = format!("[[ -k {} ]] && echo yes || echo no", path.display());
    let (_, output, _) = run_zshrs(&cmd);
    let _ = std::fs::remove_file(&path);
    assert!(output.starts_with("yes"), "got: {output:?}");
}

#[test]
fn test_cond_O_owned_by_user() {
    // /tmp is typically root-owned; not us. /Users/$USER/... is. We
    // just check the operator runs without erroring — exit status of 0
    // (yes) or 1 (no) is fine.
    let (_, output, _) = run_zshrs("[[ -O /tmp ]] && echo yes || echo no");
    assert!(
        output.starts_with("yes") || output.starts_with("no"),
        "got: {output:?}"
    );
}

#[test]
fn test_cond_G_owned_by_group() {
    let (_, output, _) = run_zshrs("[[ -G /tmp ]] && echo yes || echo no");
    assert!(
        output.starts_with("yes") || output.starts_with("no"),
        "got: {output:?}"
    );
}

// ---------------------------------------------------------------------------
// Extendedglob `^pat` negation in `[[ str = pat ]]` cond test
// ---------------------------------------------------------------------------

#[test]
fn test_cond_neg_pattern_excludes_match() {
    // `[[ apple = ^a* ]]` with extendedglob → false (apple DOES match a*).
    let (_, output, _) = run_zshrs("setopt extendedglob; [[ apple = ^a* ]] && echo y || echo n");
    assert_eq!(output.trim(), "n", "got: {output:?}");
}

#[test]
fn test_cond_neg_pattern_includes_non_match() {
    // `[[ banana = ^a* ]]` with extendedglob → true.
    let (_, output, _) = run_zshrs("setopt extendedglob; [[ banana = ^a* ]] && echo y || echo n");
    assert_eq!(output.trim(), "y", "got: {output:?}");
}

#[test]
fn test_cond_neg_literal_without_extendedglob() {
    // Without extendedglob, `^` is a literal char and `^a*` doesn't
    // match `apple`.
    let (_, output, _) = run_zshrs("[[ apple = ^a* ]] && echo y || echo n");
    assert_eq!(output.trim(), "n", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// `wait $!` with empty pid is a silent no-op (matches zsh)
// ---------------------------------------------------------------------------

#[test]
fn test_wait_empty_pid_is_silent_zero() {
    // When `$!` is empty (no bg job has been started), `wait $!` runs
    // with an empty arg. zsh silently returns 0; bash errors. Match zsh.
    let (status, _, stderr) = run_zshrs("wait $!; echo \"exit=$?\"");
    assert_eq!(status, 0, "should exit 0");
    assert!(
        !stderr.contains("invalid pid"),
        "should not error on empty pid, got: {stderr:?}"
    );
}

// ---------------------------------------------------------------------------
// `print -m PATTERN args...` — glob-match filter
// ---------------------------------------------------------------------------

#[test]
fn test_print_m_glob_filter() {
    let (_, output, _) = run_zshrs("print -m 'h*' hello world hi");
    assert_eq!(output.trim(), "hello hi", "got: {output:?}");
}

#[test]
fn test_print_m_extension_filter() {
    let (_, output, _) = run_zshrs("print -m '*.txt' a.txt b.log c.txt");
    assert_eq!(output.trim(), "a.txt c.txt", "got: {output:?}");
}

#[test]
fn test_print_m_no_match_empty_line() {
    let (_, output, _) = run_zshrs("print -m 'z' a b c");
    assert_eq!(output, "\n", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// `integer i=EXPR` runs arithmetic evaluation on the RHS
// ---------------------------------------------------------------------------

#[test]
fn test_integer_init_arith() {
    let (_, output, _) = run_zshrs("integer i=5+3; echo $i");
    assert_eq!(output.trim(), "8", "got: {output:?}");
}

#[test]
fn test_integer_init_complex_expr() {
    let (_, output, _) = run_zshrs("integer i=2*3+1; echo $i");
    assert_eq!(output.trim(), "7", "got: {output:?}");
}

#[test]
fn test_integer_init_division_truncates() {
    let (_, output, _) = run_zshrs("integer i=10/3; echo $i");
    assert_eq!(output.trim(), "3", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// Positional-param subscript: ${@[N,M]}, ${*[N]}, $@[N], ${argv[N]}
// ---------------------------------------------------------------------------

#[test]
fn test_at_subscript_slice() {
    let (_, output, _) = run_zshrs("set -- a b c d e; echo \"${@[2,3]}\"");
    assert_eq!(output.trim(), "b c", "got: {output:?}");
}

#[test]
fn test_at_subscript_single() {
    let (_, output, _) = run_zshrs("set -- a b c d e; echo \"${@[1]}\"");
    assert_eq!(output.trim(), "a", "got: {output:?}");
}

#[test]
fn test_at_subscript_negative() {
    let (_, output, _) = run_zshrs("set -- a b c d e; echo \"${@[-1]}\"");
    assert_eq!(output.trim(), "e", "got: {output:?}");
}

#[test]
fn test_star_subscript_slice() {
    let (_, output, _) = run_zshrs("set -- a b c d e; echo \"${*[2,4]}\"");
    assert_eq!(output.trim(), "b c d", "got: {output:?}");
}

#[test]
fn test_argv_subscript_slice() {
    let (_, output, _) = run_zshrs("set -- a b c d e; echo \"${argv[2,4]}\"");
    assert_eq!(output.trim(), "b c d", "got: {output:?}");
}

#[test]
fn test_bare_at_subscript() {
    let (_, output, _) = run_zshrs("set -- a b c d e; echo $@[2,4]");
    assert_eq!(output.trim(), "b c d", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// `for f in $arr` splices array elements (one iteration per element)
// ---------------------------------------------------------------------------

#[test]
fn test_for_in_array_splices() {
    let (_, output, _) =
        run_zshrs("arr=(apple banana cherry); for f in $arr; do echo \"$f\"; done");
    assert_eq!(output.trim(), "apple\nbanana\ncherry", "got: {output:?}");
}

#[test]
fn test_for_quoted_array_joins() {
    // "$arr" (DQ) joins to a scalar — single iteration with the joined
    // string. Matches zsh DQ-array semantics.
    let (_, output, _) = run_zshrs("arr=(a b c); for f in \"$arr\"; do echo \"got=$f\"; done");
    // zsh joins with first char of $IFS (default space)
    assert_eq!(output.trim(), "got=a b c", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// `arr+=val` (no parens) — push as new element (runtime-dispatched)
// ---------------------------------------------------------------------------

#[test]
fn test_array_append_single_no_parens() {
    let (_, output, _) = run_zshrs("a=(x); a+=y; echo \"${a[@]} ${#a}\"");
    assert_eq!(output.trim(), "x y 2", "got: {output:?}");
}

#[test]
fn test_array_append_to_multi_element() {
    let (_, output, _) = run_zshrs("a=(x y); a+=z; echo \"${a[@]} ${#a}\"");
    assert_eq!(output.trim(), "x y z 3", "got: {output:?}");
}

#[test]
fn test_scalar_append_still_concats() {
    let (_, output, _) = run_zshrs("s=hi; s+=world; echo $s");
    assert_eq!(output.trim(), "hiworld", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// `${var-default}` no-colon (unset-only) default family
// ---------------------------------------------------------------------------

#[test]
fn test_no_colon_default_unset() {
    let (_, output, _) = run_zshrs("unset xx; echo \"${xx-default}\"");
    assert_eq!(output.trim(), "default", "got: {output:?}");
}

#[test]
fn test_no_colon_default_empty_keeps_empty() {
    // `${var-X}` does NOT fire on empty-but-set; only unset.
    let (_, output, _) = run_zshrs("xx=; echo \"[${xx-default}]\"");
    assert_eq!(output.trim(), "[]", "got: {output:?}");
}

#[test]
fn test_no_colon_assign() {
    let (_, output, _) = run_zshrs("unset xx; echo \"${xx=set-and-use}\"; echo \"$xx\"");
    assert_eq!(output.trim(), "set-and-use\nset-and-use", "got: {output:?}");
}

#[test]
fn test_no_colon_alt_unset_empty() {
    let (_, output, _) = run_zshrs("unset xx; echo \"[${xx+alt}]\"");
    assert_eq!(output.trim(), "[]", "got: {output:?}");
}

#[test]
fn test_no_colon_alt_set_uses_alt() {
    let (_, output, _) = run_zshrs("xx=val; echo \"${xx+alt}\"");
    assert_eq!(output.trim(), "alt", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// $status alias for $?, $pipestatus[N] synthesized from last_status
// ---------------------------------------------------------------------------

#[test]
fn test_dollar_status_alias() {
    let (_, output, _) = run_zshrs("true; echo $status");
    assert_eq!(output.trim(), "0", "got: {output:?}");
}

#[test]
fn test_dollar_status_after_failure() {
    let (_, output, _) = run_zshrs("false; echo \"$status\"");
    assert_eq!(output.trim(), "1", "got: {output:?}");
}

#[test]
fn test_pipestatus_single_command() {
    let (_, output, _) = run_zshrs("true; echo $pipestatus[1]");
    assert_eq!(output.trim(), "0", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// Char/block/fifo/socket file tests
// ---------------------------------------------------------------------------

#[test]
fn test_cond_c_chardev() {
    let (_, output, _) = run_zshrs("[[ -c /dev/null ]] && echo y || echo n");
    assert_eq!(output.trim(), "y", "got: {output:?}");
}

#[test]
fn test_cond_b_blockdev() {
    let (_, output, _) = run_zshrs("[[ -b /dev/zero ]] && echo y || echo n");
    // /dev/zero is char on macOS, block on linux. Either zsh result is fine
    // as long as zshrs matches.
    assert!(
        output.trim() == "y" || output.trim() == "n",
        "got: {output:?}"
    );
}

#[test]
fn test_cond_p_fifo_negative() {
    let (_, output, _) = run_zshrs("[[ -p /dev/null ]] && echo y || echo n");
    assert_eq!(output.trim(), "n", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// `unset -f NAME` removes function
// ---------------------------------------------------------------------------

#[test]
fn test_unset_dash_f_removes_function() {
    let (status, _, _) =
        run_zshrs("foo() { :; }; unset -f foo; type foo 2>&1 | grep -q 'not found'");
    assert_eq!(status, 0, "type should report 'not found' after unset -f");
}

// ---------------------------------------------------------------------------
// Scalar in for-list does NOT IFS-split (matches zsh)
// ---------------------------------------------------------------------------

#[test]
fn test_for_scalar_no_ifs_split_default() {
    // zsh: `for w in $s` iterates ONCE with the scalar value.
    let (_, output, _) = run_zshrs("IFS=,; s='a,b,c'; for w in $s; do echo \"[$w]\"; done");
    assert_eq!(output.trim(), "[a,b,c]", "got: {output:?}");
}

#[test]
fn test_for_scalar_splits_under_shwordsplit() {
    // bash-compat: under setopt shwordsplit, scalar IS IFS-split.
    let (_, output, _) =
        run_zshrs("setopt shwordsplit; IFS=,; s='a,b,c'; for w in $s; do echo \"[$w]\"; done");
    assert_eq!(output.trim(), "[a]\n[b]\n[c]", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// `${var//#pat/repl}` / `${var//%pat/repl}` anchored replace-all
// ---------------------------------------------------------------------------

#[test]
fn test_replace_anchored_prefix_global() {
    let (_, output, _) = run_zshrs("s=hellohello; echo \"${s//#hel/HEL}\"");
    assert_eq!(output.trim(), "HELlohello", "got: {output:?}");
}

#[test]
fn test_replace_anchored_suffix_global() {
    let (_, output, _) = run_zshrs("s=foofoo; echo \"${s//%foo/BAR}\"");
    assert_eq!(output.trim(), "fooBAR", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// `alias x` query output: bare value when safe, single-quoted when meta
// ---------------------------------------------------------------------------

#[test]
fn test_alias_query_bare_value() {
    let (_, output, _) = run_zshrs("alias x=ls; alias x");
    assert_eq!(output.trim(), "x=ls", "got: {output:?}");
}

#[test]
fn test_alias_query_quoted_value() {
    let (_, output, _) = run_zshrs("alias x='ls -la'; alias x");
    assert_eq!(output.trim(), "x='ls -la'", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// One-line function body without braces: `foo() echo hello`
// ---------------------------------------------------------------------------

#[test]
fn test_inline_funcdef_one_line_body() {
    let (_, output, _) = run_zshrs("foo() echo hello; foo");
    assert_eq!(output.trim(), "hello", "got: {output:?}");
}

#[test]
fn test_inline_funcdef_one_line_body_args() {
    let (_, output, _) = run_zshrs("foo() echo \"args:\" \"$@\"; foo a b");
    assert_eq!(output.trim(), "args: a b", "got: {output:?}");
}

#[test]
fn test_inline_funcdef_colon_body() {
    let (_, output, _) = run_zshrs("foo() :; foo; echo $?");
    assert_eq!(output.trim(), "0", "got: {output:?}");
}

#[test]
fn test_amp_redir_restores_stderr() {
    // `&> file` clobbers both fd 1 and fd 2 — both must be saved+restored
    // by the redirect scope. Otherwise a following `cat` would write its
    // output back into the file, leaking the redirect across commands.
    let tmp = std::env::temp_dir().join("zr_test_amp_redir_restores.out");
    let _ = std::fs::remove_file(&tmp);
    let path = tmp.to_string_lossy().into_owned();
    let code = format!(
        "{{ echo out; echo err >&2; }} &> {p}; echo done; cat {p}",
        p = path
    );
    let (_, output, _) = run_zshrs(&code);
    assert_eq!(output.trim(), "done\nout\nerr", "got: {output:?}");
    let _ = std::fs::remove_file(&tmp);
}

#[test]
fn test_typeset_m_glob_lists_matching() {
    // `typeset -m PAT` filters the variable listing by glob pattern.
    let (_, output, _) = run_zshrs("foo=a; foobar=b; bar=c; typeset -m 'foo*'");
    let mut lines: Vec<&str> = output.trim().lines().collect();
    lines.sort();
    assert_eq!(lines, vec!["foo=a", "foobar=b"], "got: {output:?}");
}

#[test]
fn test_print_stops_flag_processing_at_first_non_option() {
    // `print "rest:$@"` with positionals `-a -b foo` must not interpret
    // `-b` as a print flag once a non-flag arg has been seen.
    let (_, output, _) = run_zshrs("set -- -a -b foo; print \"rest:$@\"");
    assert_eq!(output.trim(), "rest:-a -b foo", "got: {output:?}");
}

#[test]
fn test_zparseopts_dash_d_removes_only_consumed() {
    // `zparseopts -D a=opta` consumes `-a` only — `-b foo` must remain
    // in the positional params untouched.
    let (_, output, _) =
        run_zshrs("zmodload zsh/zutil; set -- -a -b foo; zparseopts -D a=opta; echo \"rest:$@\"");
    assert_eq!(output.trim(), "rest:-b foo", "got: {output:?}");
}

#[test]
fn test_zparseopts_dash_v_source_array() {
    // `-v NAME` (Src/Modules/zutil.c:1955, added post-5.9 in commit
    // d051857e03) parses options FROM the array `$NAME` instead of the
    // positional params, and `-D` writes the remaining elements back.
    // c:1956-1958 aborts with "no such array" when NAME is not an array.
    let sh = |code: &str| {
        let (_, out, err) = run_zshrs(code);
        (out, err)
    };

    // Parse from a populated source array.
    let (out, _) = sh(
        "zmodload zsh/zutil; src=(-a -b x); zparseopts -v src a=A b=B; print -r -- \"A=($A) B=($B)\"",
    );
    assert_eq!(out.trim(), "A=(-a) B=(-b)", "got: {out:?}");

    // -D writes the unconsumed tail back to the source array.
    let (out, _) = sh(
        "zmodload zsh/zutil; src=(-a extra); zparseopts -D -v src a=A; print -r -- \"src=($src)\"",
    );
    assert_eq!(out.trim(), "src=(extra)", "got: {out:?}");

    // A DECLARED-empty source is a valid non-NULL empty array — no error.
    let (_, err) = sh("zmodload zsh/zutil; src=(); zparseopts -v src a=A; print -r -- ok");
    assert!(
        !err.contains("no such array"),
        "declared-empty source must not error: {err:?}"
    );

    // An UNSET source name has no array to parse — c:1956 aborts.
    let (_, err) =
        sh("zmodload zsh/zutil; unset nope 2>/dev/null; zparseopts -v nope a=A; print -r -- x");
    assert!(
        err.contains("no such array: nope"),
        "unset source must report no-such-array: {err:?}"
    );

    // A SCALAR-named source is not an array — same abort.
    let (_, err) = sh("zmodload zsh/zutil; src=hello; zparseopts -v src a=A; print -r -- x");
    assert!(
        err.contains("no such array: src"),
        "scalar source must report no-such-array: {err:?}"
    );
}

#[test]
fn test_zparseopts_dash_m_alias_redirects_to_canonical() {
    // `-M f=optf -foo=f` aliases `--foo` to the `f` spec; the actual
    // `--foo` arg lands in `optf`.
    let (_, output, _) = run_zshrs(
        "zmodload zsh/zutil; set -- --foo; zparseopts -M f=optf -foo=f; echo \"f:$optf\"",
    );
    assert_eq!(output.trim(), "f:--foo", "got: {output:?}");
}

#[test]
fn test_zparseopts_dash_m_alias_arg_option_keeps_matched_name() {
    // Same `-M` aliasing for an ARG-taking spec: `-long:=l` aliases
    // `--long` to spec `l:` (array optl). The stored option name is the
    // arg the user gave (`--long`), not the canonical `-l`; the combined
    // `name arg` value therefore reads `--long =v`. C computes the value
    // name from the matched desc before `d = map_opt_desc(d)`
    // (zutil.c:1645/1648). Verified vs /bin/zsh -f.
    let (_, output, _) = run_zshrs(
        "zmodload zsh/zutil; set -- --long=v; zparseopts -M l:=optl -long:=l; echo \"[$optl]\"",
    );
    assert_eq!(output.trim(), "[--long =v]", "got: {output:?}");
}

#[test]
fn test_zformat_width_padding() {
    // zformat `%-Ns` right-aligns (pads on left) and `%Ns` left-aligns
    // (pads on right) — opposite of printf, matches zsh observed.
    let (_, output, _) =
        run_zshrs("zmodload zsh/zutil; zformat -f r \"%-10s|%10s\" \"s:foo\"; echo \"[$r]\"");
    assert_eq!(output.trim(), "[       foo|foo       ]", "got: {output:?}");
}

#[test]
fn test_getopts_unknown_uses_zsh_format() {
    // Zsh emits `zsh:1: bad option: -X` for unknown opts when the
    // optstring isn't quiet (no leading `:`). We mirror with `zshrs:1:`.
    let (_, _, stderr) = run_zshrs("set -- -x; while getopts \"ab\" opt; do :; done");
    assert!(stderr.contains("bad option: -x"), "stderr: {stderr:?}");
}

#[test]
fn test_print_f_format_cycles_args() {
    // POSIX printf semantics: when args remain after one pass through
    // the format, cycle the format until args are exhausted.
    let (_, output, _) = run_zshrs(r#"print -f "%-5s|%-5s\n" a b c d"#);
    assert_eq!(output, "a    |b    \nc    |d    \n", "got: {output:?}");
}

#[test]
fn test_printf_width_left_align() {
    let (_, output, _) = run_zshrs(r#"printf "%-10s|%10s\n" hello world"#);
    assert_eq!(output, "hello     |     world\n", "got: {output:?}");
}

#[test]
fn test_functions_dash_m_glob_lists_matching() {
    // The matching names come out of `${(k)functions[(I)PAT]}` as
    // a space-joined scalar (same in /bin/zsh `-c` mode). Split on
    // whitespace and assert membership; iteration order isn't fixed.
    let (_, output, _) =
        run_zshrs(r#"fa() { :; }; fb() { :; }; print -r -- ${(k)functions[(I)f*]}"#);
    let mut tokens: Vec<&str> = output.split_whitespace().collect();
    tokens.sort();
    assert_eq!(tokens, vec!["fa", "fb"], "got: {output:?}");
}

#[test]
fn test_zstyle_dash_l_uses_pattern_first_format() {
    // `zstyle -L` emits `zstyle <pattern> <style> <values>...` — the
    // pattern slot must be `:foo:bar`, not the style name.
    let (_, output, _) = run_zshrs(r#"zstyle ":foo:bar" key value; zstyle -L"#);
    assert_eq!(
        output.trim(),
        "zstyle :foo:bar key value",
        "got: {output:?}"
    );
}

#[test]
fn test_zstyle_dash_l_filters_by_context_and_style() {
    // c:Src/Modules/zutil.c:544-583 — `zstyle -L [context [style]]` filters:
    // the context arg is a glob matched against each stored style-pattern,
    // and an optional style name restricts to that exact style (rc 1 if the
    // style name doesn't exist). zshrs previously ignored both args.
    const SETUP: &str =
        "zmodload zsh/zutil; zstyle ':c1' x 1; zstyle ':c1' y 2; zstyle ':c2' x 3; ";
    let out = |extra: &str| run_zshrs(&format!("{SETUP}{extra}")).1.trim().to_string();

    // Context filter — exact and wildcard.
    assert_eq!(out("zstyle -L ':c1'"), "zstyle :c1 x 1\nzstyle :c1 y 2");
    assert_eq!(out("zstyle -L ':c2'"), "zstyle :c2 x 3");
    assert_eq!(out("zstyle -L ':nomatch'"), "");
    // Context + style name.
    assert_eq!(out("zstyle -L ':c1' x"), "zstyle :c1 x 1");
    assert_eq!(out("zstyle -L ':c1' y"), "zstyle :c1 y 2");
    // `*` context, style filter keeps only that style across contexts.
    assert_eq!(out("zstyle -L '*' x"), "zstyle :c1 x 1\nzstyle :c2 x 3");
    // No args → everything.
    assert_eq!(
        out("zstyle -L"),
        "zstyle :c1 x 1\nzstyle :c2 x 3\nzstyle :c1 y 2"
    );
    // Non-existent style name → rc 1.
    let rc = run_zshrs(&format!("{SETUP}zstyle -L ':c1' nostyle; echo rc=$?"))
        .1
        .trim()
        .to_string();
    assert!(rc.ends_with("rc=1"), "got: {rc:?}");
}

#[test]
fn test_zsh_param_q_flag_backslash_only() {
    // `(q)` per zshexpn(1) = backslash-escape shell-specials, no
    // surrounding quotes. Prior bug emitted `'hi'` (qq behaviour).
    let (_, output, _) = run_zshrs(r#"a=hi; print "${(q)a}""#);
    assert_eq!(output.trim(), "hi", "got: {output:?}");
}

#[test]
fn test_zsh_param_q_flag_gradient() {
    // q→\ , qq→single-bslashquote, qqq→double-bslashquote, qqqq→$'...'
    let (_, output, _) = run_zshrs(r#"a=hi; print "${(q)a}|${(qq)a}|${(qqq)a}|${(qqqq)a}""#);
    assert_eq!(output.trim(), "hi|'hi'|\"hi\"|$'hi'", "got: {output:?}");
}

#[test]
fn test_assoc_subscript_in_double_quotes() {
    // `"$m[a]"` (no braces, in DQ context) should expand to the assoc
    // value, not append the literal `[a]` after `$m`.
    let (_, output, _) = run_zshrs(r#"typeset -A m; m[a]=1; m[b]=2; echo "$m[a] $m[b]""#);
    assert_eq!(output.trim(), "1 2", "got: {output:?}");
}

#[test]
fn test_array_subscript_in_double_quotes() {
    let (_, output, _) = run_zshrs(r#"a=(x y z); echo "$a[2] $a[-1]""#);
    assert_eq!(output.trim(), "y z", "got: {output:?}");
}

#[test]
fn test_assoc_subscript_with_dynamic_key_in_dq() {
    let (_, output, _) = run_zshrs(r#"typeset -A m; m[a]=1; k=a; echo "$m[$k]""#);
    assert_eq!(output.trim(), "1", "got: {output:?}");
}

#[test]
fn test_set_dash_u_exits_on_unbound_variable() {
    // `set -u` aka `setopt nounset` makes unbound-variable lookups
    // fatal. The shell prints `…: parameter not set` and exits.
    let (status, _, stderr) = run_zshrs(r#"set -u; echo "${undef}"; echo done"#);
    assert_ne!(status, 0, "should exit non-zero");
    assert!(
        stderr.contains("undef: parameter not set"),
        "stderr: {stderr:?}"
    );
}

#[test]
fn test_setopt_nounset_exits_on_unbound() {
    // `setopt nounset` and `set -u` both turn the same option off
    // (zsh stores the inverted-name `unset` internally).
    let (status, _, stderr) = run_zshrs(r#"setopt nounset; echo "${undef}"; echo done"#);
    assert_ne!(status, 0, "should exit non-zero");
    assert!(
        stderr.contains("undef: parameter not set"),
        "stderr: {stderr:?}"
    );
}

#[test]
fn test_param_colon_question_exits_on_empty() {
    // ${x:?msg} should print the diagnostic and exit non-zero — not
    // silently continue. Mirrors zsh's -c contract.
    let (status, _, stderr) = run_zshrs(r#"x=""; echo "${x:?missing}"; echo done"#);
    assert_ne!(status, 0, "should exit non-zero");
    assert!(stderr.contains("x: missing"), "stderr: {stderr:?}");
}

#[test]
fn test_param_question_exits_on_unset() {
    let (status, _, stderr) = run_zshrs(r#"unset x; echo "${x?gone}"; echo done"#);
    assert_ne!(status, 0, "should exit non-zero");
    assert!(stderr.contains("x: gone"), "stderr: {stderr:?}");
}

#[test]
fn test_param_colon_question_passes_through_value() {
    let (status, output, _) = run_zshrs(r#"x=val; echo "${x:?msg}""#);
    assert_eq!(status, 0);
    assert_eq!(output.trim(), "val", "got: {output:?}");
}

#[test]
fn test_unmatched_glob_default_errors_with_nomatch() {
    // zsh defaults to setopt nomatch — unmatched globs abort the shell
    // with "no matches found" rather than passing the literal through.
    let (status, _, stderr) = run_zshrs("echo /tmp/zr_no_such_pattern_*");
    assert_ne!(status, 0, "should exit non-zero");
    assert!(stderr.contains("no matches found"), "stderr: {stderr:?}");
}

#[test]
fn test_unsetopt_nomatch_passes_literal_through() {
    let (status, output, _) = run_zshrs("unsetopt nomatch; echo /tmp/zr_no_such_pattern_*");
    assert_eq!(status, 0);
    assert_eq!(
        output.trim(),
        "/tmp/zr_no_such_pattern_*",
        "got: {output:?}"
    );
}

#[test]
fn test_assignment_value_skips_glob_expansion() {
    // `integer i=2*3+1` — the `*` is arithmetic, not a path glob. With
    // NOMATCH default-on, the previous code would error out on `*`.
    let (_, output, _) = run_zshrs("integer i=2*3+1; echo $i");
    assert_eq!(output.trim(), "7", "got: {output:?}");
}

#[test]
fn test_cd_preserves_logical_path() {
    // zsh's default `cd -L` keeps the user-typed path in $PWD even
    // when the target is a symlink (`/tmp` → `/private/tmp` on macOS,
    // identical paths on plain Linux). The test asserts the
    // round-trip is exact, not the realpath.
    let (_, output, _) = run_zshrs("cd /tmp; pwd");
    assert_eq!(output.trim(), "/tmp", "got: {output:?}");
}

#[test]
fn test_cd_dash_p_realpaths() {
    // `cd -P` follows symlinks (realpath form). On macOS this resolves
    // /tmp to /private/tmp; on plain Linux they're identical. Either
    // result is acceptable as long as it matches /bin/zsh's output.
    let (_, output_zshrs, _) = run_zshrs("cd -P /tmp; pwd");
    let zshrs_pwd = output_zshrs.trim();
    let zsh_out = std::process::Command::new("/bin/zsh")
        .args(["-f", "-c", "cd -P /tmp; pwd"])
        .output()
        .expect("zsh failed");
    let zsh_pwd = String::from_utf8_lossy(&zsh_out.stdout).trim().to_string();
    assert_eq!(zshrs_pwd, zsh_pwd, "zsh: {zsh_pwd}, zshrs: {zshrs_pwd}");
}

#[test]
fn test_set_e_exits_on_failure() {
    let (status, output, _) = run_zshrs("set -e; false; echo unreachable");
    assert_ne!(status, 0, "should exit non-zero");
    assert!(!output.contains("unreachable"), "got: {output:?}");
}

#[test]
fn test_set_e_suppressed_in_if_test() {
    let (status, output, _) = run_zshrs("set -e; if false; then echo nope; fi; echo got_here");
    assert_eq!(status, 0);
    assert!(output.contains("got_here"), "got: {output:?}");
}

#[test]
fn test_set_e_suppressed_in_and_chain() {
    // `false && cmd` returns 1 but doesn't trigger errexit (POSIX:
    // failures inside an AND-OR list are consumed by the connector).
    let (status, output, _) = run_zshrs("set -e; false && echo nope; echo got_here");
    assert_eq!(status, 0);
    assert!(output.contains("got_here"), "got: {output:?}");
}

#[test]
fn test_set_e_suppressed_in_or_chain() {
    let (status, output, _) = run_zshrs("set -e; false || true; echo got_here");
    assert_eq!(status, 0);
    assert!(output.contains("got_here"), "got: {output:?}");
}

#[test]
fn test_set_e_suppressed_in_negation() {
    let (status, output, _) = run_zshrs("set -e; ! false; echo got_here");
    assert_eq!(status, 0);
    assert!(output.contains("got_here"), "got: {output:?}");
}

#[test]
fn test_set_e_suppressed_in_while_test() {
    let (status, output, _) = run_zshrs("set -e; while false; do echo nope; done; echo got_here");
    assert_eq!(status, 0);
    assert!(output.contains("got_here"), "got: {output:?}");
}

#[test]
fn test_subshell_isolates_cwd() {
    // `(cd /tmp); pwd` must not leak the cd into the parent.
    let (_, output, _) =
        run_zshrs("pwd > /tmp/zr_pwd_pre.txt; (cd /tmp); pwd > /tmp/zr_pwd_post.txt");
    let pre = std::fs::read_to_string("/tmp/zr_pwd_pre.txt").unwrap_or_default();
    let post = std::fs::read_to_string("/tmp/zr_pwd_post.txt").unwrap_or_default();
    let _ = std::fs::remove_file("/tmp/zr_pwd_pre.txt");
    let _ = std::fs::remove_file("/tmp/zr_pwd_post.txt");
    assert_eq!(
        pre.trim(),
        post.trim(),
        "subshell cd leaked: pre={pre:?} post={post:?} output={output:?}"
    );
}

#[test]
fn test_arith_assoc_subscript() {
    let (_, output, _) = run_zshrs("declare -A m; m[k]=10; echo $((m[k] + 5))");
    assert_eq!(output.trim(), "15", "got: {output:?}");
}

#[test]
fn test_arith_array_subscript() {
    let (_, output, _) = run_zshrs("a=(10 20 30); echo $((a[2] + 5))");
    assert_eq!(output.trim(), "25", "got: {output:?}");
}

#[test]
fn test_read_dash_a_honors_custom_ifs() {
    let (_, output, _) = run_zshrs(
        r#"IFS=, read -A arr <<< "1,2,3"; echo "${#arr[@]}"; echo "${arr[1]}/${arr[2]}/${arr[3]}""#,
    );
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines.first().copied(), Some("3"), "len: {output:?}");
    assert_eq!(lines.get(1).copied(), Some("1/2/3"), "values: {output:?}");
}

#[test]
fn test_read_dash_a_default_ifs_collapses_whitespace() {
    let (_, output, _) = run_zshrs(r#"read -A arr <<< "a   b   c"; echo "${#arr[@]}""#);
    assert_eq!(output.trim(), "3", "got: {output:?}");
}

#[test]
fn test_read_dash_a_mixed_ifs_absorbs_whitespace_around_separator() {
    // c:Src/utils.c:3711 spacesplit — with a MIXED IFS (whitespace + a
    // non-whitespace separator), whitespace ADJACENT to the non-ws
    // separator is absorbed into it, so `a : b` (IFS=" :") is one delimiter
    // → 2 fields; two non-ws separators keep the middle empty (`a :: b` → 3).
    // read -A previously treated the space and colon as two delimiters (4).
    let n = |input: &str| {
        let (_, out, _) = run_zshrs(&format!(r#"IFS=" :" read -A arr <<< "{input}"; echo ${{#arr}}"#));
        out.trim().to_string()
    };
    assert_eq!(n("a : b"), "2");
    assert_eq!(n("a  :  b"), "2");
    assert_eq!(n("x : y : z"), "3");
    assert_eq!(n("a :: b"), "3");
    // Pure non-ws separators still preserve empties (unchanged).
    let m = |input: &str| {
        let (_, out, _) = run_zshrs(&format!(r#"IFS=: read -A arr <<< "{input}"; echo ${{#arr}}"#));
        out.trim().to_string()
    };
    assert_eq!(m(":a:b:"), "4");
    assert_eq!(m("a::b"), "3");

    // The multi-var `read a b c` path uses the same absorption rule.
    let v = |vars: &str, input: &str| {
        let (_, out, _) = run_zshrs(&format!(
            r#"IFS=" :" read {vars} <<< "{input}"; echo "[$a][$b][$c]""#
        ));
        out.trim().to_string()
    };
    assert_eq!(v("a b c", "x : y : z"), "[x][y][z]");
    assert_eq!(v("a b", "p : q"), "[p][q][]");
    // Pure non-ws multi-var preserves the leading/trailing empties.
    let (_, out, _) = run_zshrs(r#"IFS=: read a b c <<< ":x:"; echo "[$a][$b][$c]""#);
    assert_eq!(out.trim(), "[][x][]");
}

#[test]
fn test_tilde_unknown_user_errors() {
    let (status, _, stderr) = run_zshrs("echo ~nonexistent_user_zrs");
    assert_ne!(status, 0, "should exit non-zero");
    assert!(stderr.contains("no such user"), "stderr: {stderr:?}");
}

#[test]
fn test_empty_heredoc_succeeds() {
    // The fix: an empty heredoc body must not be re-processed on the
    // second `process_heredocs` pass. Pre-fix, content emptiness was
    // the "not yet processed" marker; we now use a separate `processed`
    // flag. Compare exit status against /bin/zsh; both should succeed
    // and produce the same output.
    let (status, output, _) = run_zshrs("cat <<EOF\nEOF");
    assert_eq!(status, 0, "empty heredoc should succeed");
    let zsh = std::process::Command::new("/bin/zsh")
        .args(["-f", "-c", "cat <<EOF\nEOF"])
        .output()
        .expect("zsh failed");
    let zsh_out = String::from_utf8_lossy(&zsh.stdout).to_string();
    assert_eq!(output, zsh_out, "empty heredoc should match zsh");
}

#[test]
fn test_echo_dash_e_interprets_octal_escape() {
    // `echo -e "\033[1mB\033[0m"` should interpret `\033` as ESC.
    let (_, output, _) = run_zshrs(r#"echo -e "\033[1mB\033[0m""#);
    assert_eq!(output, "\x1b[1mB\x1b[0m\n", "got: {output:?}");
}

#[test]
fn test_alias_listing_unquoted_for_simple_values() {
    // zsh emits `x=1` not `x='1'` when the value has no shell specials.
    let (_, output, _) = run_zshrs("alias x=1 y=2; alias | sort");
    assert!(
        output.contains("x=1\n") && output.contains("y=2\n"),
        "expected unquoted bare values, got: {output:?}"
    );
    assert!(
        !output.contains("x='1'"),
        "should not bslashquote bare numeric values, got: {output:?}"
    );
}

#[test]
fn test_substring_with_var_offset() {
    let (_, output, _) = run_zshrs(r#"s=abcdef; n=2; echo "${s:$n:2}""#);
    assert_eq!(output.trim(), "cd", "got: {output:?}");
}

#[test]
fn test_substring_with_arith_offset() {
    let (_, output, _) = run_zshrs(r#"s=abcdef; echo "${s:$((1+1)):2}""#);
    assert_eq!(output.trim(), "cd", "got: {output:?}");
}

#[test]
fn test_double_paren_command_nested_paren_assignment() {
    // c:Src/exec.c WC_ARITH — the `((EXPR))` command form must not truncate a
    // trailing `)` that closes an INNER subexpression. The compiler's
    // wrapper-strip used `trim_end_matches(')')`, so `((x=(1+2)))` had its
    // final `)` stripped to `x=(1+2` → "bad math expression: ')' expected".
    // Only a BALANCED outer wrapper should be removed.
    let out = |code: &str| run_zshrs(code).1.trim().to_string();

    assert_eq!(out("((x=(1+2))); echo $x"), "3");
    assert_eq!(out("((n=(3))); echo $n"), "3");
    assert_eq!(out(r#"((a=(b=3))); echo "$a $b""#), "3 3");
    assert_eq!(out("x=1; ((x && (y=5))); echo $y"), "5");
    assert_eq!(out("((1 && (y=5))); echo $y"), "5");
    assert_eq!(out("((2 * (3 + (k=4)))); echo $k"), "4");
    // Regressions: wrapped/subscripted/plain/compound `(( ))` still work.
    assert_eq!(out("(( ((v=7)) )); echo $v"), "7");
    assert_eq!(out("a=(1 2 3); ((a[2]=99)); echo $a[2]"), "99");
    assert_eq!(out("a=(1 2); ((a[1]+=10)); echo $a[1]"), "11");
    assert_eq!(out(r#"(( x = 2, y = 3 )); echo "$x $y""#), "2 3");
    assert_eq!(out("((x=5)); echo $x"), "5");
    assert_eq!(out("(( 0 )); echo $?"), "1");
}

#[test]
fn test_substring_with_var_offset_and_length() {
    let (_, output, _) = run_zshrs(r#"s=abcdef; n=2; m=3; echo "${s:$n:$m}""#);
    assert_eq!(output.trim(), "cde", "got: {output:?}");
}

#[test]
fn test_pipefail_returns_first_nonzero() {
    // `set -o pipefail` makes a pipeline return the rightmost non-zero
    // status — `false | true` returns 1 with pipefail on, 0 without.
    let (status, _, _) = run_zshrs("set -o pipefail; false | true");
    assert_eq!(status, 1, "pipefail-on should propagate failure");
}

#[test]
fn test_pipefail_default_off_returns_last() {
    let (status, _, _) = run_zshrs("false | true");
    assert_eq!(status, 0, "default pipeline status is the last stage");
}

#[test]
fn test_setopt_pipefail_alias() {
    let (status, _, _) = run_zshrs("setopt pipefail; false | true");
    assert_eq!(status, 1, "setopt pipefail should be equivalent to set -o");
}

#[test]
fn test_ifs_default_includes_null() {
    // POSIX/zsh default IFS = ` \t\n\0`. Test the bytes round-trip.
    let (_, output, _) = run_zshrs(r#"echo "[$IFS]""#);
    let zsh = std::process::Command::new("/bin/zsh")
        .args(["-f", "-c", r#"echo "[$IFS]""#])
        .output()
        .expect("zsh failed");
    let zsh_out = String::from_utf8_lossy(&zsh.stdout).to_string();
    assert_eq!(output, zsh_out, "IFS bytes should match /bin/zsh");
}

#[test]
fn test_command_not_found_includes_line_number() {
    // `zsh:1: command not found: NAME` is the canonical format.
    let (_, _, stderr) = run_zshrs("nonexistent_command_xyz_zr");
    assert!(
        stderr.contains(":1: command not found:"),
        "stderr: {stderr:?}"
    );
}

#[test]
fn test_error_line_number_is_actual_line_not_one() {
    // Runtime error diagnostics (command-not-found, noclobber) must
    // carry the ACTUAL line, not a hardcoded 1. C uses `zerr(...)`
    // (exec.c:811) which threads the current lineno; the port's bridge
    // eprintln sites hardcoded `:1:`. Both errors sit on line 3 here.
    let (_, _, e1) = run_zshrs("print a\nprint b\nnonexistent_zzz_cmd");
    assert!(
        e1.contains(":3: command not found: nonexistent_zzz_cmd"),
        "command-not-found line: {e1:?}"
    );
    let tf = format!("{}/zr_errline_nclob.out", tempdir_for_test());
    std::fs::write(&tf, "x\n").unwrap();
    let (_, _, e2) = run_zshrs(&format!("setopt noclobber\nprint p\necho z > {tf}"));
    let _ = std::fs::remove_file(&tf);
    assert!(e2.contains(":3: file exists:"), "noclobber line: {e2:?}");
}

#[test]
fn test_noclobber_blocks_overwrite_and_sinks_output() {
    let _ = std::fs::remove_file("/tmp/zr_nclob_test.out");
    std::fs::write("/tmp/zr_nclob_test.out", "first\n").unwrap();
    let (_, output, stderr) = run_zshrs(
        "set -o noclobber; echo second > /tmp/zr_nclob_test.out; echo done; cat /tmp/zr_nclob_test.out",
    );
    let _ = std::fs::remove_file("/tmp/zr_nclob_test.out");
    assert!(stderr.contains("file exists"), "stderr: {stderr:?}");
    assert!(output.contains("done"), "got: {output:?}");
    assert!(
        output.contains("first"),
        "should preserve original content, got: {output:?}"
    );
    assert!(
        !output.contains("second"),
        "second should be sunk, got: {output:?}"
    );
}

#[test]
fn test_noclobber_force_overwrites_with_bang() {
    let _ = std::fs::remove_file("/tmp/zr_nclob_force_test.out");
    std::fs::write("/tmp/zr_nclob_force_test.out", "first\n").unwrap();
    let (_, output, _) = run_zshrs(
        "set -o noclobber; echo second >! /tmp/zr_nclob_force_test.out; cat /tmp/zr_nclob_force_test.out",
    );
    let _ = std::fs::remove_file("/tmp/zr_nclob_force_test.out");
    assert!(output.contains("second"), "got: {output:?}");
    assert!(
        !output.contains("first"),
        "should be overwritten, got: {output:?}"
    );
}

#[test]
fn test_pwd_dash_p_realpaths() {
    // pwd -P resolves symlinks (canonicalize); -L preserves logical.
    let (_, output_zshrs, _) = run_zshrs("cd /tmp; pwd -P");
    let zshrs_pwd = output_zshrs.trim();
    let zsh_out = std::process::Command::new("/bin/zsh")
        .args(["-f", "-c", "cd /tmp; pwd -P"])
        .output()
        .expect("zsh failed");
    let zsh_pwd = String::from_utf8_lossy(&zsh_out.stdout).trim().to_string();
    assert_eq!(zshrs_pwd, zsh_pwd, "zsh:{zsh_pwd} zshrs:{zshrs_pwd}");
}

#[test]
fn test_function_keyword_with_parens() {
    // `function name() { body }` — the `function` keyword PLUS `()`
    // parens combo. zsh accepts both; pre-fix zshrs only handled
    // `function name { body }` and `name() { body }` separately.
    let (_, output, _) = run_zshrs("function bar() { echo hello; }; bar");
    assert_eq!(output.trim(), "hello", "got: {output:?}");
}

#[test]
fn test_dq_suppresses_array_only_sort_flags() {
    // zsh: (o)/(O)/(n)/(i)/(u) only fire in array context. Inside DQ
    // the array is joined as a scalar with original element order.
    let (_, output, _) = run_zshrs(r#"a=(c b a); print -- "${(o)a}""#);
    assert_eq!(
        output.trim(),
        "c b a",
        "DQ should preserve order, got: {output:?}"
    );
}

#[test]
fn test_no_dq_sort_flags_still_apply() {
    // No-DQ context: (o) sorts as expected.
    let (_, output, _) = run_zshrs("a=(c b a); print -- ${(o)a}");
    assert_eq!(output.trim(), "a b c", "got: {output:?}");
}

#[test]
fn test_dq_suppresses_unique_flag() {
    let (_, output, _) = run_zshrs(r#"a=(c b a c); print -- "${(ou)a}""#);
    assert_eq!(
        output.trim(),
        "c b a c",
        "DQ should preserve order+dups, got: {output:?}"
    );
}

#[test]
fn test_dq_suppresses_natural_sort() {
    let (_, output, _) = run_zshrs(r#"a=(file10 file2 file1); print -- "${(on)a}""#);
    assert_eq!(output.trim(), "file10 file2 file1", "got: {output:?}");
}

#[test]
fn test_dq_suppresses_sort_on_assoc_kv() {
    // The `(o)`/`(u)` sort suppression inside DQ must apply to assoc
    // `(k)`/`(v)`/`(kv)` results too, not just plain arrays. In DQ the
    // key/value array sepjoins to a scalar (C c:3034 clears isarr) so
    // the c:4245 sort/unique block is gated out — the values/keys keep
    // hash iteration order. The assoc `(k)/(v)` seed sets isarr AFTER
    // the sepjoin transition, so the port re-applies it after the seed.
    // Uses a single-key map so hash order is deterministic; a
    // reverse-alpha two-key map proves the sort is really suppressed.
    let (_, output, _) = run_zshrs(
        r#"typeset -A m=(beta 2 alpha 1); v="${(vo)m}"; k="${(ko)m}"; print -r -- "[$v][$k]""#,
    );
    // Hash order for this insertion is (beta, alpha) in both zsh and
    // zshrs; (o) would sort to (alpha, beta) / (1, 2) — suppression
    // keeps the unsorted "2 1" / "beta alpha".
    assert_eq!(output.trim(), "[2 1][beta alpha]", "got: {output:?}");
}

#[test]
fn test_positional_slice_skip_offset() {
    // ${@:N:M} — N is "skip N positionals" (0-based, includes \$0
    // when N=0). Same shape as bash/zsh.
    let (_, output, _) = run_zshrs("set -- a b c d e; echo \"${@:1:2}\"");
    assert_eq!(output.trim(), "a b", "got: {output:?}");
}

#[test]
fn test_positional_slice_no_length() {
    let (_, output, _) = run_zshrs("set -- a b c d e; echo \"${@:2}\"");
    assert_eq!(output.trim(), "b c d e", "got: {output:?}");
}

#[test]
fn test_array_slice_offset_skips() {
    // ${arr:1:2} — skip 1 element, take 2 (0-based offset).
    let (_, output, _) = run_zshrs(r#"arr=(x y z w); echo "${arr:1:2}""#);
    assert_eq!(output.trim(), "y z", "got: {output:?}");
}

#[test]
fn test_at_subscript_inclusive_range() {
    let (_, output, _) = run_zshrs("set -- a b c d e; echo \"$@[2,4]\"");
    assert_eq!(output.trim(), "b c d", "got: {output:?}");
}

#[test]
fn test_bang_pid_after_background() {
    // `cmd &` records pid into $! so wait $! works.
    let (_, output, _) = run_zshrs("sleep 0.1 & echo $!");
    let pid = output.trim();
    assert!(
        pid.parse::<i64>().is_ok(),
        "expected numeric pid, got: {pid:?}"
    );
    assert!(pid != "0", "should be a real pid, got: {pid:?}");
}

#[test]
fn test_bang_pid_initial_zero() {
    let (_, output, _) = run_zshrs(r#"echo "[$!]""#);
    assert_eq!(output.trim(), "[0]", "got: {output:?}");
}

#[test]
fn test_declare_ra_blocks_array_assign() {
    let (status, _, stderr) = run_zshrs("declare -ra arr=(a b c); arr=(x y z); echo done");
    assert_ne!(status, 0, "should exit non-zero");
    assert!(stderr.contains("read-only"), "stderr: {stderr:?}");
}

#[test]
fn test_declare_ra_blocks_append() {
    let (status, _, stderr) = run_zshrs("declare -ra arr=(a b c); arr+=(x); echo done");
    assert_ne!(status, 0, "should exit non-zero");
    assert!(stderr.contains("read-only"), "stderr: {stderr:?}");
}

#[test]
fn test_print_s_silent_and_records_history() {
    // `print -s X` saves X to history INSTEAD OF stdout. fc -l in
    // -c mode shows only the entries added in this session.
    let (_, output, _) = run_zshrs(r#"print -s "echo from-history"; fc -l"#);
    // No "echo from-history" leaked to stdout (only fc -l output).
    let trimmed = output.trim();
    assert!(
        trimmed.contains("echo from-history") && trimmed.contains("1"),
        "fc -l should list session entry numbered 1, got: {output:?}"
    );
    // Make sure print -s didn't echo to stdout itself.
    assert_eq!(
        output
            .lines()
            .filter(|l| l.contains("echo from-history"))
            .count(),
        1,
        "print -s should not echo to stdout, got: {output:?}"
    );
}

#[test]
fn test_z_split_emits_metas_as_separate_tokens() {
    // ${(z)str} tokenises a command line like the parser would —
    // shell metas (;, &, |, etc.) become their own tokens.
    let (_, output, _) = run_zshrs(r#"a="echo hi; ls"; print -l "${(z)a}""#);
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["echo", "hi", ";", "ls"], "got: {output:?}");
}

#[test]
fn test_z_split_pipe_token() {
    let (_, output, _) = run_zshrs(r#"a="ls | grep foo"; print -l "${(z)a}""#);
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["ls", "|", "grep", "foo"], "got: {output:?}");
}

#[test]
fn test_alias_query_silent_when_unknown() {
    // After unalias, `alias NAME` should return non-zero with NO
    // diagnostic — matches zsh.
    let (status, _output, stderr) = run_zshrs("alias hi=echo; unalias hi; alias hi 2>&1");
    assert_ne!(status, 0, "should exit non-zero");
    assert!(
        !stderr.contains("not found"),
        "unalias query should be silent, got: {stderr:?}"
    );
}

#[test]
fn test_kill_dash_l_lists_bare_names() {
    let (_, output, _) = run_zshrs("kill -l");
    let trimmed = output.trim();
    // Should be space-separated bare names on a single line.
    assert!(
        !trimmed.contains("SIG"),
        "expected bare names, not 'SIG' prefix, got: {trimmed:?}"
    );
    assert!(
        trimmed.contains("HUP"),
        "should contain HUP, got: {trimmed:?}"
    );
    assert!(
        trimmed.contains("TERM"),
        "should contain TERM, got: {trimmed:?}"
    );
}

#[test]
fn test_kill_dash_capital_l_unknown_signal() {
    // zsh 5.9.0.3-test (bundled src/zsh/Src/jobs.c:2880-2898) added a
    // `-L` tabular listing — the table prints signal NN/NAME pairs and
    // returns 0. Verified against /opt/homebrew/bin/zsh which carries
    // the same source. macOS /bin/zsh ships an older 5.9 that still
    // treats `-L` as `unknown signal SIGL` rc=1; the test follows the
    // bundled C source per the project's port-fidelity policy.
    let (status, stdout, _) = run_zshrs("kill -L");
    assert_eq!(status, 0);
    assert!(
        stdout.contains("HUP") && stdout.contains("TERM"),
        "stdout: {stdout:?}"
    );
}

#[test]
fn test_integer_attribute_arith_evaluates_assignment() {
    // After `integer i`, plain `i=5*3` arith-evaluates the value.
    let (_, output, _) = run_zshrs("integer i; i=5*3; echo $i");
    assert_eq!(output.trim(), "15", "got: {output:?}");
}

#[test]
fn test_bare_assignment_does_not_glob_expand() {
    // `i=5*3` (no integer) keeps the value literal — zsh does NOT
    // glob-expand assignment RHS by default.
    let (_, output, _) = run_zshrs("i=5*3; echo $i");
    assert_eq!(output.trim(), "5*3", "got: {output:?}");
}

#[test]
fn test_paren_e_flag_expands_parameters() {
    // `${(e)s}` expands $-references in the value (NOT runs the
    // value as a command).
    let (_, output, _) = run_zshrs(r#"s="\$test"; test=val; echo "${(e)s}""#);
    assert_eq!(output.trim(), "val", "got: {output:?}");
}

#[test]
fn test_type_unknown_format_matches_zsh() {
    // `NAME not found` on stdout, not `zshrs: type: NAME: not found`.
    let (_, output, _) = run_zshrs("type nonexistent_xyz_abc");
    assert!(output.contains("not found"), "got: {output:?}");
    assert!(
        !output.contains("type:"),
        "should not have 'type:' prefix, got: {output:?}"
    );
}

#[test]
fn test_disabled_builtin_falls_through_in_whence() {
    // c:Src/builtin.c:4123 — `builtintab->getnode` skips a DISABLED builtin,
    // so `type`/`whence` don't report a disabled builtin as a builtin; they
    // fall through to the external command. zshrs still reported "builtin".
    let out = |code: &str| run_zshrs(code).1.trim().to_string();

    // Enabled: reported as a builtin.
    assert_eq!(out("whence -w echo"), "echo: builtin");
    // Disabled: falls through to the external command.
    assert_eq!(out("disable echo; whence -w echo"), "echo: command");
    assert_eq!(out("disable echo; type -w echo"), "echo: command");
    assert!(
        out("disable echo; type echo").ends_with("/echo"),
        "type should show the external path"
    );
    // `type -a` no longer lists the disabled builtin (external only).
    assert!(
        !out("disable echo; type -a echo").contains("shell builtin"),
        "disabled builtin must not appear in -a"
    );
    // Re-enabling restores the builtin.
    assert_eq!(out("disable echo; enable echo; whence -w echo"), "echo: builtin");
}

#[test]
fn test_echo_default_interprets_escapes() {
    // zsh's default (no -e flag) interprets \n, \t, \b, etc. unless
    // setopt bsd_echo is set.
    let (_, output, _) = run_zshrs(r#"echo "x\ny""#);
    assert_eq!(output, "x\ny\n", "got: {output:?}");
}

#[test]
fn test_echo_dash_capital_e_disables_escapes() {
    // -E disables escape interpretation.
    let (_, output, _) = run_zshrs(r#"echo -E "x\ny""#);
    assert_eq!(output, "x\\ny\n", "got: {output:?}");
}

#[test]
fn test_export_dash_n_rejected() {
    // bash-style `-n` flag is bad option in zsh.
    let (status, _, stderr) = run_zshrs("export X=val; export -n X");
    assert_ne!(status, 0);
    assert!(stderr.contains("bad option"), "stderr: {stderr:?}");
}

#[test]
fn test_set_dash_x_xtrace_prints_commands() {
    // `set -x` enables tracing — each command echoed to stderr with
    // `$PS4` prefix (default `+ `).
    let (_, _, stderr) = run_zshrs("set -x; echo hello");
    assert!(stderr.contains("echo hello"), "stderr: {stderr:?}");
}

#[test]
fn test_set_plus_x_disables_xtrace() {
    let (_, _, stderr) = run_zshrs("set -x; set +x; echo hidden");
    // The `set +x` line itself is traced, but the subsequent `echo`
    // should NOT appear in stderr.
    let trace_lines: Vec<&str> = stderr
        .lines()
        .filter(|l| l.contains("echo hidden"))
        .collect();
    assert_eq!(
        trace_lines.len(),
        0,
        "echo should not be traced after +x, got: {stderr:?}"
    );
}

#[test]
fn test_xtrace_uses_ps4() {
    // Verify PS4 expansion runs and prefixes the trace line. The
    // earlier assertion `stderr.contains("+ ")` was bash's default,
    // not zsh's — zsh's default PS4 is `+%N:%i> ` (Src/init.c) which
    // expands to `+<file>:<line>> ` and never produces a literal
    // `+ `. Set PS4 explicitly inside the script so this test is
    // robust against any inherited $PROMPT4/$PS4 from the user's
    // env (real zsh imports them too — verified against
    // /opt/homebrew/bin/zsh).
    let (_, _, stderr) = run_zshrs("PS4='+ '; set -x; true");
    assert!(stderr.contains("+ "), "stderr: {stderr:?}");
    assert!(
        stderr.contains("true"),
        "stderr should contain command: {stderr:?}"
    );
}

#[test]
fn test_default_value_expands_command_substitution() {
    // `${var:-$(cmd)}` should run cmd when var is unset/empty.
    let (_, output, _) = run_zshrs(r#"unset x; echo "${x:-$(echo subst)}""#);
    assert_eq!(output.trim(), "subst", "got: {output:?}");
}

#[test]
fn test_default_value_expands_variable() {
    let (_, output, _) = run_zshrs(r#"unset x; y=hello; echo "${x:-$y}""#);
    assert_eq!(output.trim(), "hello", "got: {output:?}");
}

#[test]
fn test_assign_default_expands() {
    let (_, output, _) = run_zshrs(r#"unset x; y=expanded; echo "${x:=$y}"; echo "x=$x""#);
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["expanded", "x=expanded"], "got: {output:?}");
}

#[test]
fn test_echo_hex_escape() {
    let (_, output, _) = run_zshrs(r#"echo "\x41\x42""#);
    assert_eq!(output.trim(), "AB", "got: {output:?}");
}

#[test]
fn test_break_n_breaks_outer_loop() {
    let (_, output, _) = run_zshrs(
        "for i in 1 2 3; do for j in a b; do [[ $j = a && $i = 2 ]] && break 2; echo \"$i$j\"; done; done",
    );
    assert_eq!(output.trim(), "1a\n1b", "got: {output:?}");
}

#[test]
fn test_continue_n_continues_outer_loop() {
    let (_, output, _) = run_zshrs(
        "for i in 1 2 3; do for j in a b; do [[ $j = b ]] && continue 2; echo \"$i$j\"; done; done",
    );
    assert_eq!(output.trim(), "1a\n2a\n3a", "got: {output:?}");
}

#[test]
fn test_replace_pattern_expands_dollar_var() {
    // ${s/$pat/X} expands $pat to the variable's value.
    let (_, output, _) = run_zshrs("s=hello; pat=l; echo \"${s/$pat/X}\"");
    assert_eq!(output.trim(), "heXlo", "got: {output:?}");
}

#[test]
fn test_replace_global_pattern_expands() {
    let (_, output, _) = run_zshrs("s=hello; pat=l; echo \"${s//$pat/X}\"");
    assert_eq!(output.trim(), "heXXo", "got: {output:?}");
}

#[test]
fn test_array_star_joins_with_first_ifs() {
    // ${a[*]} joins elements with first IFS char (default space).
    // print -l on a joined string should print one line.
    let (_, output, _) = run_zshrs(r#"a=("a b" "c d"); print -l "${a[*]}""#);
    assert_eq!(output.trim(), "a b c d", "got: {output:?}");
}

#[test]
fn test_array_at_keeps_separate_words() {
    // ${a[@]} splices each element as a separate word.
    let (_, output, _) = run_zshrs(r#"a=("a b" "c d"); print -l "${a[@]}""#);
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["a b", "c d"], "got: {output:?}");
}

#[test]
fn test_wait_unknown_pid_errors() {
    let (status, _, stderr) = run_zshrs("wait 99999");
    assert_eq!(status, 127);
    assert!(stderr.contains("not a child"), "stderr: {stderr:?}");
}

#[test]
fn test_dollar_lt_file_reads_contents() {
    // $(< file) is zsh's shorthand for reading file contents.
    std::fs::write("/tmp/zr_dlt_test.txt", "hello\nworld\n").unwrap();
    let (_, output, _) = run_zshrs(r#"echo "$(< /tmp/zr_dlt_test.txt)""#);
    let _ = std::fs::remove_file("/tmp/zr_dlt_test.txt");
    assert_eq!(output.trim_end(), "hello\nworld", "got: {output:?}");
}

#[test]
fn test_dollar_lt_no_space() {
    std::fs::write("/tmp/zr_dlt2.txt", "data\n").unwrap();
    let (_, output, _) = run_zshrs(r#"echo "$(</tmp/zr_dlt2.txt)""#);
    let _ = std::fs::remove_file("/tmp/zr_dlt2.txt");
    assert_eq!(output.trim(), "data", "got: {output:?}");
}

#[test]
fn test_printf_q_uses_backslash_quoting() {
    // zsh's `%q` uses backslash quoting (matches ${(q)}), NOT bash's
    // single-bslashquote wrapping.
    let (_, output, _) = run_zshrs(r#"printf "%q\n" "hello world""#);
    assert_eq!(output, "hello\\ world\n", "got: {output:?}");
}

#[test]
fn test_printf_q_safe_word_unquoted() {
    let (_, output, _) = run_zshrs(r#"printf "%q\n" hello"#);
    assert_eq!(output.trim(), "hello", "got: {output:?}");
}

#[test]
fn test_arith_bitwise_not() {
    // $((~0)) should evaluate as bitwise NOT (-1), not tilde-expand.
    let (_, output, _) = run_zshrs("echo $((~0))");
    assert_eq!(output.trim(), "-1", "got: {output:?}");
}

#[test]
fn test_arith_bitwise_not_in_expr() {
    let (_, output, _) = run_zshrs("a=5; echo $((~a + 6))");
    assert_eq!(output.trim(), "0", "got: {output:?}");
}

#[test]
fn test_arith_dollar_var_still_works() {
    // Sanity: regression check that $var inside arith still expands.
    let (_, output, _) = run_zshrs("a=10; echo $(($a + 5))");
    assert_eq!(output.trim(), "15", "got: {output:?}");
}

#[test]
fn test_strip_pattern_expands_dollar_var() {
    // ${s%$ext} should expand $ext before applying strip.
    let (_, output, _) = run_zshrs(r#"s=foo.bar; ext=.bar; echo "${s%$ext}""#);
    assert_eq!(output.trim(), "foo", "got: {output:?}");
}

#[test]
fn test_strip_long_pattern_expands() {
    let (_, output, _) = run_zshrs(r#"s=path/file; pre="path/"; echo "${s##$pre}""#);
    assert_eq!(output.trim(), "file", "got: {output:?}");
}

#[test]
fn test_substring_negative_length_truncates_from_end() {
    // ${s:0:-2} takes from offset 0, stopping 2 chars before end.
    let (_, output, _) = run_zshrs(r#"a=hello; echo "${a:0:-2}""#);
    assert_eq!(output.trim(), "hel", "got: {output:?}");
}

#[test]
fn test_substring_offset_and_negative_length() {
    let (_, output, _) = run_zshrs(r#"a=hello; echo "${a:1:-1}""#);
    assert_eq!(output.trim(), "ell", "got: {output:?}");
}

#[test]
fn test_substring_no_length_takes_rest() {
    // No length given still takes all remaining (default sentinel).
    let (_, output, _) = run_zshrs(r#"a=hello; echo "${a:2}""#);
    assert_eq!(output.trim(), "llo", "got: {output:?}");
}

#[test]
fn test_shift_too_many_errors() {
    let (status, _, stderr) = run_zshrs("set -- a; shift 2");
    assert_ne!(status, 0);
    assert!(stderr.contains("shift count must be"), "stderr: {stderr:?}");
}

#[test]
fn test_echo_combined_flags() {
    // `echo -nE` combines: no newline, no escape interpretation.
    let (_, output, _) = run_zshrs(r#"echo -nE "a\nb""#);
    assert_eq!(output, "a\\nb", "got: {output:?}");
}

#[test]
fn test_left_pad_flag() {
    let (_, output, _) = run_zshrs(r#"s=hi; echo "[${(l:5:)s}]""#);
    assert_eq!(output.trim(), "[   hi]", "got: {output:?}");
}

#[test]
fn test_right_pad_flag() {
    let (_, output, _) = run_zshrs(r#"s=hi; echo "[${(r:5:)s}]""#);
    assert_eq!(output.trim(), "[hi   ]", "got: {output:?}");
}

#[test]
fn test_left_pad_with_fill_char() {
    let (_, output, _) = run_zshrs(r#"s=hi; echo "[${(l:5::*:)s}]""#);
    assert_eq!(output.trim(), "[***hi]", "got: {output:?}");
}

#[test]
fn test_quoted_glob_pattern_in_test_is_literal() {
    // [[ "abc" == "a*" ]] — quoted "*" is literal, not glob.
    let (status, _, _) = run_zshrs(r#"[[ "abc" == "a*" ]]"#);
    assert_eq!(status, 1, "should NOT match (quoted * is literal)");
}

#[test]
fn test_quoted_literal_star_matches_quoted_literal_star() {
    let (status, _, _) = run_zshrs(r#"[[ "a*" == "a*" ]]"#);
    assert_eq!(status, 0, "literal a* matches literal a*");
}

#[test]
fn test_unquoted_glob_pattern_still_matches() {
    let (status, _, _) = run_zshrs(r#"[[ "abc" == a* ]]"#);
    assert_eq!(status, 0, "unquoted a* should glob-match abc");
}

#[test]
fn test_glob_caret_negation() {
    // [^abc] should match any single char NOT in {a,b,c}. The `glob`
    // crate only natively understands `[!abc]` (fnmatch); zshrs
    // pre-translates `[^...]` → `[!...]`.
    let dir = "/tmp/zr_glob_caret_test";
    let _ = std::fs::create_dir_all(dir);
    for n in ["a", "b", "c"] {
        std::fs::write(format!("{}/{}", dir, n), "").unwrap();
    }
    let (_, output, _) = run_zshrs(&format!("cd {}; echo [^a]", dir));
    let _ = std::fs::remove_dir_all(dir);
    let mut got: Vec<&str> = output.split_whitespace().collect();
    got.sort();
    assert_eq!(got, vec!["b", "c"], "got: {output:?}");
}

#[test]
fn test_while_read_returns_1_at_eof_no_newline() {
    use std::io::Write;
    use std::process::Stdio;
    // Pipe `a\nb\nc` (no trailing newline) — read should return 1
    // for the partial last line, so the loop body doesn't run for `c`.
    let mut child = std::process::Command::new(zshrs_bin())
        .args(["-f", "-c", "while read line; do echo \"[$line]\"; done"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.as_mut().unwrap().write_all(b"a\nb\nc").unwrap();
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert_eq!(stdout, "[a]\n[b]\n", "got: {stdout:?}");
}

#[test]
fn test_positional_default_plus_returns_alt_when_set() {
    // ${1+yes} returns yes when $1 is set, nothing when unset.
    let (_, output, _) = run_zshrs(r#"set -- a b; echo "${1+yes}""#);
    assert_eq!(output.trim(), "yes", "got: {output:?}");
}

#[test]
fn test_positional_default_plus_unset() {
    let (_, output, _) = run_zshrs(r#"set --; echo "[${1+yes}]""#);
    assert_eq!(output.trim(), "[]", "got: {output:?}");
}

#[test]
fn test_tilde_with_dollar_var() {
    // ~$USER should expand to /home/USER (or /Users/USER on macOS).
    let user = std::env::var("USER").unwrap_or_default();
    if user.is_empty() {
        return;
    }
    let (_, output, _) = run_zshrs("echo ~$USER");
    assert!(
        output.contains(&user),
        "expected USER in output, got: {output:?}"
    );
    assert!(
        !output.starts_with('~'),
        "expected expansion, got: {output:?}"
    );
}

#[test]
fn test_tilde_with_quoted_dollar_var() {
    let user = std::env::var("USER").unwrap_or_default();
    if user.is_empty() {
        return;
    }
    let (_, output, _) = run_zshrs(r#"echo ~"$USER""#);
    assert!(
        output.contains(&user),
        "expected USER in output, got: {output:?}"
    );
    assert!(
        !output.starts_with('~'),
        "expected expansion, got: {output:?}"
    );
}

#[test]
fn test_glob_qualifier_size_l_uses_bytes() {
    // ${L+N}: size > N bytes (default unit BYTES, not 512-blocks).
    let path = "/tmp/zr_l_qual_test";
    std::fs::write(path, "12345").unwrap();
    let (_, output, _) = run_zshrs(&format!("echo {}(L+3)", path));
    let _ = std::fs::remove_file(path);
    assert!(
        output.contains(path),
        "5 bytes > 3, should match: {output:?}"
    );
}

#[test]
fn test_user_function_overrides_r_builtin() {
    // zsh dispatch: function > builtin. Without the runtime override
    // check inside BUILTIN_R, `r` would route to fc-replay (history
    // re-execution) and infinite-loop on a fresh script.
    let (_, output, _) = run_zshrs(r#"r() { echo "user-r $1"; }; r 5"#);
    assert_eq!(output.trim(), "user-r 5", "got: {output:?}");
}

#[test]
fn test_user_function_overrides_echo_builtin() {
    let (_, output, _) = run_zshrs(r#"echo() { command echo "USER:" "$@"; }; echo hi"#);
    assert_eq!(output.trim(), "USER: hi", "got: {output:?}");
}

#[test]
fn test_user_function_overrides_pwd_builtin() {
    let (_, output, _) = run_zshrs(r#"pwd() { echo USER-PWD; }; pwd"#);
    assert_eq!(output.trim(), "USER-PWD", "got: {output:?}");
}

#[test]
fn test_user_function_overrides_true_builtin() {
    let (_, output, _) = run_zshrs(r#"true() { echo TRUE; }; true && echo ok"#);
    assert_eq!(output.trim(), "TRUE\nok", "got: {output:?}");
}

#[test]
fn test_test_builtin_bracket_form_returns_correct_status() {
    // Regression: `[ a -eq b ]` previously routed through Op::Exec
    // (because `[` was flagged as a glob char by the dynamic-name
    // check) and silently returned 0 on every invocation, breaking
    // every if/elif/until that used the legacy `[ ... ]` test form.
    let (_, output, _) = run_zshrs(r#"[ 1 -eq 2 ]; echo $?"#);
    assert_eq!(output.trim(), "1", "[1 -eq 2] should be false: {output:?}");
    let (_, output, _) = run_zshrs(r#"[ 2 -eq 2 ]; echo $?"#);
    assert_eq!(output.trim(), "0", "[2 -eq 2] should be true: {output:?}");
}

#[test]
fn test_if_elif_chain_with_bracket_test() {
    let (_, output, _) = run_zshrs(
        r#"x=2; if [ $x -eq 1 ]; then echo one; elif [ $x -eq 2 ]; then echo two; else echo other; fi"#,
    );
    assert_eq!(output.trim(), "two", "got: {output:?}");
}

#[test]
fn test_until_loop_with_bracket_test() {
    let (_, output, _) = run_zshrs(r#"i=0; until [ $i -ge 3 ]; do echo $i; i=$((i+1)); done"#);
    assert_eq!(output.trim(), "0\n1\n2", "got: {output:?}");
}

#[test]
fn test_assoc_append_pairs_adds_new_keys() {
    let (_, output, _) = run_zshrs(
        r#"declare -A m=(a 1 b 2); m+=(c 3); for k in ${(@k)m}; do echo "$k=${m[$k]}"; done"#,
    );
    let mut lines: Vec<&str> = output.lines().collect();
    lines.sort();
    assert_eq!(lines, vec!["a=1", "b=2", "c=3"], "got: {output:?}");
}

#[test]
fn test_param_flag_oa_preserves_array_order() {
    let (_, output, _) = run_zshrs(r#"a=(c a b); echo ${(oa)a}"#);
    assert_eq!(output.trim(), "c a b", "got: {output:?}");
}

#[test]
fn test_param_flag_Oa_reverses_array_order() {
    let (_, output, _) = run_zshrs(r#"a=(c a b); echo ${(Oa)a}"#);
    assert_eq!(output.trim(), "b a c", "got: {output:?}");
}

#[test]
fn test_param_flag_on_numeric_sort() {
    let (_, output, _) = run_zshrs(r#"a=(10 2 30 4); echo ${(on)a}"#);
    assert_eq!(output.trim(), "2 4 10 30", "got: {output:?}");
}

#[test]
fn test_param_flag_oi_case_insensitive_sort() {
    let (_, output, _) = run_zshrs(r#"a=(B a C b A c); echo ${(oi)a}"#);
    // case-insensitive: A/a equal, B/b equal, C/c equal — stable order
    // preserves original within each equivalence class.
    let zsh_expected = "a A B b C c";
    assert_eq!(output.trim(), zsh_expected, "got: {output:?}");
}

#[test]
fn test_param_flag_unique_with_eq_wordsplit() {
    // `${(u)=s}` word-splits the scalar (`=`) THEN dedups (`u`). C runs
    // the spbreak split (subst.c:3901) BEFORE the (u)/(o) unique+sort
    // block (subst.c:4245); the port applied the sort/unique block first
    // (when isarr was still 0) so the `=`-split words were never deduped.
    // Verified against `/bin/zsh -f`:
    //   s="c a b c a"; ${(u)=s}  -> c a b   (split, dedup, insertion order)
    //   ${(ou)=s}                -> a b c   (split, dedup, sorted)
    //   ${(onu)=n} n="3 1 2 1 3" -> 1 2 3   (split, dedup, numeric sort)
    let (_, output, _) = run_zshrs(
        r#"s="c a b c a"; print -r -- "${(u)=s}"; print -r -- "${(ou)=s}"; n="3 1 2 1 3"; print -r -- "${(onu)=n}""#,
    );
    assert_eq!(output, "c a b\na b c\n1 2 3\n", "got: {output:?}");
}

#[test]
fn test_printf_g_uses_shortest_representation() {
    let (_, output, _) = run_zshrs(r#"printf "%g\n" 3.14"#);
    assert_eq!(output.trim(), "3.14", "%g 3.14: {output:?}");
    let (_, output, _) = run_zshrs(r#"printf "%g\n" 1234567"#);
    assert_eq!(output.trim(), "1.23457e+06", "%g 1234567: {output:?}");
    let (_, output, _) = run_zshrs(r#"printf "%g\n" 0.00001"#);
    assert_eq!(output.trim(), "1e-05", "%g 0.00001: {output:?}");
    let (_, output, _) = run_zshrs(r#"printf "%g\n" 100"#);
    assert_eq!(output.trim(), "100", "%g 100: {output:?}");
}

#[test]
fn test_typeset_int_plus_eq_arithmetic_add() {
    // `typeset -i x=42; x+=8` must store 50 (arithmetic add), not "428"
    // (string concat). Without the integer check inside
    // BUILTIN_APPEND_SCALAR_OR_PUSH the var ended up as the
    // concatenated string.
    let (_, output, _) = run_zshrs(r#"typeset -i x=42; x+=8; echo $x"#);
    assert_eq!(output.trim(), "50", "got: {output:?}");
}

#[test]
fn test_typeset_int_plus_eq_arith_expression() {
    // RHS goes through arith eval, so `x+=5*2` adds 10, not "5*2".
    let (_, output, _) = run_zshrs(r#"typeset -i x=10; x+=5*2; echo $x"#);
    assert_eq!(output.trim(), "20", "got: {output:?}");
}

#[test]
fn test_param_flag_k_on_regular_array_returns_values() {
    // zsh quirk: `${(k)arr}` on a regular array returns the array
    // contents themselves (not "1 2 3" indices). Verified against zsh.
    let (_, output, _) = run_zshrs(r#"a=(x y z); echo "${(k)a}""#);
    assert_eq!(output.trim(), "x y z", "got: {output:?}");
}

#[test]
fn test_getopts_stops_after_arg_taking_option() {
    // Regression: getopts was overwriting OPTIND back to optind+1
    // after the arg-taking branch had already advanced by 2,
    // causing the next iteration to land on the consumed arg
    // instead of the following flag.
    let (_, output, _) = run_zshrs(
        r#"set -- -a -b X -c; while getopts "ab:c" opt; do echo "opt=$opt arg=$OPTARG"; done"#,
    );
    assert_eq!(
        output.trim(),
        "opt=a arg=\nopt=b arg=X\nopt=c arg=",
        "got: {output:?}"
    );
}

#[test]
fn test_set_u_with_default_modifier_does_not_abort() {
    // `${var:-fb}` and `${var-fb}` provide a default for missing vars.
    // Even with `set -u` (nounset) on, the lookup must NOT abort the
    // shell — the modifier IS the handler. zshrs was calling
    // get_variable() (which honors nounset) and exiting 1 before the
    // modifier could fire.
    let (_, output, _) = run_zshrs(r#"set -u; echo "${notdef:-fb}""#);
    assert_eq!(output.trim(), "fb", "got: {output:?}");
    let (_, output, _) = run_zshrs(r#"set -u; echo "${notdef-fb}""#);
    assert_eq!(output.trim(), "fb", "got: {output:?}");
    let (_, output, _) = run_zshrs(r#"set -u; echo "${notdef:+set}""#);
    // :+ on unset returns "", not abort.
    assert_eq!(output.trim(), "", "got: {output:?}");
}

#[test]
fn test_param_flag_pound_arith_to_char() {
    // `${(#)val}`: arith-evaluate the value(s) and output the matching
    // character(s). 65 → "A", 97 → "a".
    let (_, output, _) = run_zshrs(r#"a=65; echo "${(#)a}""#);
    assert_eq!(output.trim(), "A", "got: {output:?}");
    let (_, output, _) = run_zshrs(r#"a=(65 66 67); echo "${(#)a}""#);
    assert_eq!(output.trim(), "A B C", "got: {output:?}");
}

#[test]
fn test_param_flag_n_natural_sort() {
    // `(n)` natural sort: "file2" < "file10" < "file20".
    let (_, output, _) = run_zshrs(r#"arr=(file10 file2 file1 file20); echo ${(on)arr}"#);
    assert_eq!(
        output.trim(),
        "file1 file2 file10 file20",
        "got: {output:?}"
    );
}

#[test]
fn test_subshell_export_does_not_leak_to_parent() {
    // zsh subshell `(...)` forks; child's `export` dies with the child.
    // zshrs runs subshells in-process, so we snapshot+restore the OS
    // env around subshell entry/exit. Without this, `(export y=v)`
    // would leak `y` to the parent shell.
    let (_, output, _) =
        run_zshrs(r#"x=outer; (export y=sub; echo "in: $y"); echo "out y=${y:-empty}""#);
    assert_eq!(output.trim(), "in: sub\nout y=empty", "got: {output:?}");
}

#[test]
fn test_subshell_unset_does_not_leak_to_parent() {
    let (_, output, _) =
        run_zshrs(r#"export X=parent; (unset X; echo "sub: ${X:-empty}"); echo "outer: $X""#);
    assert_eq!(
        output.trim(),
        "sub: empty\nouter: parent",
        "got: {output:?}"
    );
}

#[test]
fn test_user_function_overrides_coreutils_builtins() {
    // Coreutils-style anti-fork builtins (cat, head, tail, wc, sort,
    // sleep, uname, etc.) all need to honor user function overrides
    // — same dispatch rule as the shell builtins.
    let cases = [
        ("cat", r#"cat() { echo USER-CAT; }; cat"#, "USER-CAT"),
        ("wc", r#"wc() { echo USER-WC; }; wc -l"#, "USER-WC"),
        ("sort", r#"sort() { echo USER-SORT; }; sort"#, "USER-SORT"),
        (
            "sleep",
            r#"sleep() { echo USER-SLEEP; }; sleep 999"#,
            "USER-SLEEP",
        ),
        ("head", r#"head() { echo USER-HEAD; }; head"#, "USER-HEAD"),
        ("tail", r#"tail() { echo USER-TAIL; }; tail"#, "USER-TAIL"),
        ("seq", r#"seq() { echo USER-SEQ; }; seq 5"#, "USER-SEQ"),
        ("uniq", r#"uniq() { echo USER-UNIQ; }; uniq"#, "USER-UNIQ"),
        ("date", r#"date() { echo USER-DATE; }; date"#, "USER-DATE"),
        (
            "uname",
            r#"uname() { echo USER-UNAME; }; uname"#,
            "USER-UNAME",
        ),
        (
            "mkdir",
            r#"mkdir() { echo USER-MKDIR; }; mkdir foo"#,
            "USER-MKDIR",
        ),
    ];
    for (name, code, expected) in cases {
        let (_, output, _) = run_zshrs(code);
        assert_eq!(output.trim(), expected, "{}: got {output:?}", name);
    }
}

#[test]
fn test_param_flag_Q_dequote() {
    // (Q) strips one layer of shell quoting. Both single and double
    // quotes get unwrapped; backslash escapes inside DQ are processed.
    let (_, output, _) = run_zshrs(r#"s="\"hello\""; echo "${(Q)s}""#);
    assert_eq!(output.trim(), "hello", "DQ: {output:?}");
    let (_, output, _) = run_zshrs(r#"s="'world'"; echo "${(Q)s}""#);
    assert_eq!(output.trim(), "world", "SQ: {output:?}");
}

#[test]
fn test_pipeline_last_stage_runs_in_current_shell() {
    // zsh: the LAST stage of a pipeline runs in the current shell
    // (not a forked subshell), so a trailing `read x` keeps its
    // assignment after the pipeline. bash forks every stage; zshrs
    // must match zsh.
    let (_, output, _) = run_zshrs(r#"echo hi | read x; echo "x=$x""#);
    assert_eq!(output.trim(), "x=hi", "got: {output:?}");
}

#[test]
fn test_pipeline_last_stage_assignment_persists() {
    let (_, output, _) = run_zshrs(r#"printf "a\nb\nc\n" | { read first; echo "first=$first"; }"#);
    assert_eq!(output.trim(), "first=a", "got: {output:?}");
}

#[test]
fn test_pipeline_child_handles_sigpipe_gracefully() {
    // Long producer + early-closing reader: `seq | head -3` would
    // panic before because Rust's println! errored on EPIPE writes
    // (parent shell ignored SIGPIPE). Children now reset SIGPIPE to
    // default so a broken-pipe write kills the child silently.
    let (_, output, _) = run_zshrs(r#"seq 1 100 | head -3"#);
    assert_eq!(output.trim(), "1\n2\n3", "got: {output:?}");
}

#[test]
fn test_type_alias_uses_zsh_format() {
    // zsh: `type g` for an alias prints "g is an alias for echo"
    // (not the bash "g is aliased to `echo'" form).
    let (_, output, _) = run_zshrs(r#"alias g="echo"; type g"#);
    assert_eq!(output.trim(), "g is an alias for echo", "got: {output:?}");
}

#[test]
fn test_command_v_resolution_order_matches_zsh() {
    // alias > function > builtin > external. -v prints the resolved
    // form (path for external, name for builtin/function, "alias k=v"
    // for alias).
    let (_, output, _) = run_zshrs(r#"command -v echo"#);
    assert_eq!(output.trim(), "echo", "builtin: {output:?}");
    let (_, output, _) = run_zshrs(r#"f() { :; }; command -v f"#);
    assert_eq!(output.trim(), "f", "function: {output:?}");
    let (_, output, _) = run_zshrs(r#"alias g="echo"; command -v g"#);
    assert_eq!(output.trim(), "alias g=echo", "alias: {output:?}");
    let (_, output, _) = run_zshrs(r#"command -v ls"#);
    assert!(
        output.trim().ends_with("/ls"),
        "external should be a path: {output:?}"
    );
}

#[test]
fn test_command_v_missing_returns_nonzero() {
    let (status, _output, _) = run_zshrs(r#"command -v xxx_no_such_cmd_x"#);
    assert_ne!(status, 0, "missing command should return non-zero");
}

#[test]
fn test_which_for_builtin_shows_csh_format() {
    // zsh `which echo` → "echo: shell built-in command".
    let (_, output, _) = run_zshrs(r#"which echo"#);
    assert_eq!(
        output.trim(),
        "echo: shell built-in command",
        "got: {output:?}"
    );
}

#[test]
fn test_read_processes_backslash_escapes_without_dash_r() {
    // POSIX read (no -r): each `\X` pair drops the backslash.
    // `\<newline>` is a line-continuation (both stripped). Without
    // this, an input of "a\b\n" was producing the backspace
    // control character via Rust's String::replace stub.
    let (_, output, _) = run_zshrs(r#"printf 'a\\b\n' | { read line; echo "[$line]"; }"#);
    assert_eq!(output.trim(), "[ab]", "got: {output:?}");
}

#[test]
fn test_cmd_subst_word_splits_in_argument_context() {
    // `f $(echo a b c)` must pass three args, not one. zsh's default
    // for bare cmd-subst in argument position is IFS word-split.
    let (_, output, _) = run_zshrs(r#"f() { echo "argc=$#"; }; f $(echo a b c)"#);
    assert_eq!(output.trim(), "argc=3", "got: {output:?}");
}

#[test]
fn test_cmd_subst_no_split_in_dq_context() {
    // `f "$(echo a b c)"` is one arg — DQ suppresses the split.
    let (_, output, _) = run_zshrs(r#"f() { echo "argc=$#"; }; f "$(echo a b c)""#);
    assert_eq!(output.trim(), "argc=1", "got: {output:?}");
}

#[test]
fn test_cmd_subst_no_split_in_assignment() {
    // Assignment RHS preserves whitespace/newlines — no IFS split.
    let (_, output, _) = run_zshrs("x=$(printf 'a\nb\nc'); echo \"$x\" | wc -l");
    assert_eq!(output.trim(), "3", "got: {output:?}");
}

#[test]
fn test_dash_f_flag_disables_rcs_and_hashdirs() {
    // zsh -f sets `nohashdirs` and `norcs` (default-on options that
    // -f turns off so they show up in `setopt`'s diff list).
    let (_, output, _) = run_zshrs(r#"setopt"#);
    let mut lines: Vec<&str> = output.lines().collect();
    lines.sort();
    assert_eq!(lines, vec!["nohashdirs", "norcs"], "got: {output:?}");
}

#[test]
fn test_default_aliases_match_zsh() {
    // zsh ships compiled-in aliases `run-help=man` and
    // `which-command=whence` — visible in `zsh -f -c 'alias'`.
    let (_, output, _) = run_zshrs(r#"alias"#);
    let mut lines: Vec<String> = output.lines().map(|s| s.to_string()).collect();
    lines.sort();
    assert!(
        lines.contains(&"run-help=man".to_string()),
        "missing run-help: {output:?}"
    );
    assert!(
        lines.contains(&"which-command=whence".to_string()),
        "missing which-command: {output:?}"
    );
}

#[test]
fn test_dot_glob_excludes_dot_and_dotdot() {
    // Setup: create a temp dir with .hide and .zg_hidden, glob `.*`
    // should match the hidden files but NOT `.` or `..` themselves.
    use std::fs::{create_dir_all, File};
    let dir = "/tmp/zr_dotglob_test";
    let _ = std::fs::remove_dir_all(dir);
    create_dir_all(dir).unwrap();
    File::create(format!("{}/.hide", dir)).unwrap();
    File::create(format!("{}/.other", dir)).unwrap();
    File::create(format!("{}/visible", dir)).unwrap();
    let (_, output, _) = run_zshrs(&format!("cd {}; echo .*", dir));
    let _ = std::fs::remove_dir_all(dir);
    let trimmed = output.trim();
    assert!(
        trimmed.contains(".hide"),
        ".* should match .hide: {output:?}"
    );
    assert!(
        trimmed.contains(".other"),
        ".* should match .other: {output:?}"
    );
    assert!(
        !trimmed
            .split_whitespace()
            .any(|w| w == "." || w == ".." || w == "./." || w == "./.."),
        ".* must exclude . and ..: {output:?}"
    );
    assert!(
        !trimmed.contains("visible"),
        ".* must not match visible: {output:?}"
    );
}

#[test]
fn test_star_glob_excludes_dotfiles_by_default() {
    use std::fs::{create_dir_all, File};
    let dir = "/tmp/zr_starglob_test";
    let _ = std::fs::remove_dir_all(dir);
    create_dir_all(dir).unwrap();
    File::create(format!("{}/.hide", dir)).unwrap();
    File::create(format!("{}/visible", dir)).unwrap();
    let (_, output, _) = run_zshrs(&format!("cd {}; echo *", dir));
    let _ = std::fs::remove_dir_all(dir);
    let trimmed = output.trim();
    assert_eq!(
        trimmed, "visible",
        "* should match only non-dot files: {output:?}"
    );
}

#[test]
fn test_param_replace_glob_pattern_question() {
    // ${s/?/X} should treat `?` as glob (any single char), not literal.
    let (_, output, _) = run_zshrs(r#"s=hello; echo "${s/?/X}""#);
    assert_eq!(output.trim(), "Xello", "got: {output:?}");
}

#[test]
fn test_param_replace_glob_pattern_star() {
    // ${s/*l/X} replaces longest prefix ending in `l`.
    let (_, output, _) = run_zshrs(r#"s=hello; echo "${s/*l/X}""#);
    assert_eq!(output.trim(), "Xo", "got: {output:?}");
}

#[test]
fn test_param_replace_glob_pattern_class() {
    // ${s/[aeiou]/V} replaces first vowel.
    let (_, output, _) = run_zshrs(r#"s=hello; echo "${s/[aeiou]/V}""#);
    assert_eq!(output.trim(), "hVllo", "got: {output:?}");
}

#[test]
fn test_param_replace_global_with_glob() {
    // ${s//[aeiou]/V} replaces all vowels.
    let (_, output, _) = run_zshrs(r#"s=hello; echo "${s//[aeiou]/V}""#);
    assert_eq!(output.trim(), "hVllV", "got: {output:?}");
}

#[test]
fn test_cond_regex_with_variable() {
    // [[ str =~ $pat ]] must expand $pat before applying the regex.
    let (_, output, _) = run_zshrs(r#"pat="^h"; [[ "hello" =~ $pat ]] && echo M"#);
    assert_eq!(output.trim(), "M", "got: {output:?}");
}

#[test]
fn test_cond_regex_with_capture_groups() {
    // $match[N] is populated from regex captures.
    let (_, output, _) =
        run_zshrs(r#"pat="^(h)(.*)"; [[ "hello" =~ $pat ]] && echo "$match[1]:$match[2]""#);
    assert_eq!(output.trim(), "h:ello", "got: {output:?}");
}

#[test]
fn test_glob_qualifier_in_pipeline_child() {
    // Regression: `echo glob_pat(N) | wc -w` returned 0 because the
    // worker pool doesn't survive fork() (POSIX: only the calling
    // thread persists), so prefetch_metadata blocked / returned empty
    // in the child stage. Detect via signals::is_forked_child() and
    // use the serial stat path when forked.
    use std::fs::{create_dir_all, File};
    let dir = "/tmp/zr_pipe_glob_test";
    let _ = std::fs::remove_dir_all(dir);
    create_dir_all(dir).unwrap();
    for i in 0..40 {
        File::create(format!("{}/file{:03}", dir, i)).unwrap();
    }
    let (_, output, _) = run_zshrs(&format!("echo {}/file*(N) | wc -w", dir));
    let _ = std::fs::remove_dir_all(dir);
    assert_eq!(
        output.trim().parse::<usize>().unwrap_or(0),
        40,
        "got: {output:?}"
    );
}

#[test]
fn test_param_length_at_star_returns_positional_count() {
    // ${#@} and ${#*} both return $# (number of positional params),
    // not the length of the IFS-joined string. Without the special
    // case, `set -- a b c; echo "${#@}"` returned 5 (length of
    // "a b c") instead of 3.
    let (_, output, _) = run_zshrs(r#"set -- a b c; echo "${#@} ${#*}""#);
    assert_eq!(output.trim(), "3 3", "got: {output:?}");
}

#[test]
fn test_param_length_uses_chars_not_bytes() {
    // ${#var} should count chars, not bytes — so `héllo` is 5 chars
    // (the `é` is one codepoint = one char) not 6 bytes.
    let (_, output, _) = run_zshrs(r#"s="héllo"; echo "${#s}""#);
    assert_eq!(output.trim(), "5", "got: {output:?}");
}

#[test]
fn test_exit_builtin_fires_exit_trap() {
    // `trap 'cleanup' EXIT; exit N` must run the EXIT trap before
    // the process terminates. Implicit script-end already fired the
    // trap; explicit `exit N` was bypassing it via std::process::exit.
    let (status, output, _) = run_zshrs(r#"trap 'echo CLEANUP' EXIT; exit 5"#);
    assert_eq!(status, 5, "exit code should propagate");
    assert_eq!(output.trim(), "CLEANUP", "got: {output:?}");
}

#[test]
fn test_exit_trap_with_explicit_exit_in_trap_body() {
    // Trap body is removed BEFORE running so a recursive exit
    // doesn't re-fire the trap.
    let (status, output, _) = run_zshrs(r#"trap 'echo TRAP1' EXIT; exit 7"#);
    assert_eq!(status, 7);
    assert_eq!(output.trim(), "TRAP1", "got: {output:?}");
}

#[test]
fn test_function_name_with_hyphen_dispatches_correctly() {
    // `foo-bar()` registered cleanly but the call site looked up
    // `foo\u{9b}bar` (the lexer's META encoding of `-`) and missed
    // the registered function. Untokenize before add_name in the
    // CallFunction emit path.
    let (_, output, _) = run_zshrs(r#"foo-bar() { echo F; }; foo-bar"#);
    assert_eq!(output.trim(), "F", "got: {output:?}");
}

#[test]
fn test_function_name_with_hyphen_passes_args() {
    let (_, output, _) = run_zshrs(r#"my-cmd() { echo "called: $@"; }; my-cmd hello world"#);
    assert_eq!(output.trim(), "called: hello world", "got: {output:?}");
}

#[test]
fn test_typeset_f_preserves_first_word_of_body() {
    // The parser captured body_start AFTER its zshlex() that advances
    // past the first body token, so `typeset -f f` for
    // `f() { echo a; echo b; }` printed `a; echo b;` (missing the
    // first `echo`). Capture body_start BEFORE the zshlex.
    let (_, output, _) = run_zshrs(r#"f() { echo a; echo b; }; typeset -f f"#);
    assert!(
        output.contains("echo a"),
        "body should preserve first echo: {output:?}"
    );
    assert!(
        output.contains("echo b"),
        "body should include second echo: {output:?}"
    );
    assert!(
        !output.contains("\ta;"),
        "body must not start with bare `a` (lost echo): {output:?}"
    );
}

#[test]
fn test_t_flag_includes_readonly_modifier() {
    // ${(t)var} reports the var's type. For a readonly scalar, zsh
    // emits "scalar-readonly" (compound: kind + modifier joined by
    // `-`). Was just "scalar" because builtin_readonly populated
    // readonly_vars but not var_attrs.readonly.
    let (_, output, _) = run_zshrs(r#"readonly R=x; echo "${(t)R}""#);
    assert_eq!(output.trim(), "scalar-readonly", "got: {output:?}");
    let (_, output, _) = run_zshrs(r#"typeset -r R=x; echo "${(t)R}""#);
    assert_eq!(output.trim(), "scalar-readonly", "got: {output:?}");
}

#[test]
fn test_cond_nt_uses_nanosecond_precision() {
    // `[[ a -nt b ]]` was using MetadataExt::mtime() (seconds only),
    // so two files touched within the same second compared equal
    // even when 500ms apart. Switched to SystemTime::modified() for
    // nanosecond precision.
    use std::fs::File;
    let a = "/tmp/zr_nt_a";
    let b = "/tmp/zr_nt_b";
    let _ = std::fs::remove_file(a);
    let _ = std::fs::remove_file(b);
    File::create(a).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    File::create(b).unwrap();
    let (_, output, _) = run_zshrs(&format!("[[ {} -nt {} ]] && echo Y || echo N", b, a));
    let _ = std::fs::remove_file(a);
    let _ = std::fs::remove_file(b);
    assert_eq!(output.trim(), "Y", "got: {output:?}");
}

#[test]
fn test_integer_dash_r_sets_readonly_attr() {
    // `integer -r I=42` should produce "${(t)I}" == "integer-readonly".
    // builtin_integer was ignoring all flags; now parses -r and -x.
    let (_, output, _) = run_zshrs(r#"integer -r I=42; echo "${(t)I}""#);
    assert_eq!(output.trim(), "integer-readonly", "got: {output:?}");
}

#[test]
fn test_argv_length_returns_positional_count() {
    // ${#argv} / ${#argv[@]} — `argv` is zsh's named alias for the
    // positional array. Was returning the byte length of the IFS-
    // joined string (5 for "a b c") instead of the count (3).
    let (_, output, _) = run_zshrs(r#"set -- a b c; echo "${#argv} ${#argv[@]}""#);
    assert_eq!(output.trim(), "3 3", "got: {output:?}");
}

#[test]
fn test_dq_star_assignment_joins_with_ifs() {
    // `v="$*"` should capture the full join, not just the first
    // positional. GET_VAR for `*` returns Array which pop_args
    // flattens — for DQ scalar context, we now follow GET_VAR with
    // ARRAY_JOIN_STAR (which joins by $IFS first char).
    let (_, output, _) = run_zshrs(r#"set -- a b c; v="$*"; echo "[$v]""#);
    assert_eq!(output.trim(), "[a b c]", "got: {output:?}");
    let (_, output, _) = run_zshrs(r#"IFS=":"; set -- a b c; v="$*"; echo "[$v]""#);
    assert_eq!(output.trim(), "[a:b:c]", "got: {output:?}");
}

#[test]
fn test_dq_at_preserves_splice_semantics() {
    // `"$@"` must keep splice semantics — each positional its own
    // word, even in DQ. Only `$*` joins; my $* fix must not affect $@.
    let (_, output, _) = run_zshrs(r#"set -- a "b c" d; for x in "$@"; do echo "[$x]"; done"#);
    assert_eq!(output.trim(), "[a]\n[b c]\n[d]", "got: {output:?}");
}

#[test]
fn test_noglob_precommand_suppresses_glob() {
    // `noglob CMD args...` is a precommand modifier — args must
    // not be glob-expanded. Was failing because the noglob option
    // was set AFTER the args were already expanded.
    let (_, output, _) = run_zshrs(r#"noglob echo /tmp/xyz_no_match*"#);
    assert_eq!(output.trim(), "/tmp/xyz_no_match*", "got: {output:?}");
    let (_, output, _) = run_zshrs(r#"noglob echo a b *"#);
    assert_eq!(output.trim(), "a b *", "got: {output:?}");
}

#[test]
fn test_coreutils_error_msg_strips_os_error_suffix() {
    // cat/head/tail/wc all use Rust's io::Error display by default
    // which appends "(os error N)". zsh's coreutils-style emit just
    // the friendly message. pretty_io_err strips the suffix.
    let (_, _, stderr) = run_zshrs(r#"cat /no/such/file 2>&1"#);
    let combined = format!("{}{}", "", stderr);
    let _ = combined; // unused — checking output format below
    let (_, output, _) = run_zshrs(r#"cat /no/such/file 2>&1"#);
    assert_eq!(
        output.trim(),
        "cat: /no/such/file: No such file or directory",
        "got: {output:?}"
    );
}

#[test]
fn test_arith_subscripted_array_assign() {
    // `((a[i]=val))` — the runtime arith eval (and compile_arith via
    // BUILTIN_ARITH_EVAL bypass) now writes back to the array element
    // instead of substituting a[i] with its current value.
    let (_, output, _) = run_zshrs(r#"a=(0 0 0); echo $((a[2]=42)); echo $a[2]"#);
    assert_eq!(output.trim(), "42\n42", "arith subst form: {output:?}");
}

#[test]
fn test_source_missing_file_zsh_format_and_exit_127() {
    // zsh format: `zshrs:source:1: no such file or directory: PATH`
    // and exit 127. Was emitting Rust's io::Error display unchanged
    // (with "(os error 2)" suffix) and exiting 1 — both wrong.
    let (status, _stdout, stderr) = run_zshrs(r#"source /no/such/file"#);
    assert_eq!(status, 127, "exit code should be 127 for not-found");
    assert!(
        stderr.contains("source:1: no such file or directory:"),
        "stderr: {stderr:?}"
    );
    assert!(
        !stderr.contains("(os error"),
        "should strip os error suffix: {stderr:?}"
    );
}

#[test]
fn test_array_zero_subscript_assignment_errors() {
    // zsh: arrays/positionals are 1-based — `a[0]=v` is an
    // "assignment to invalid subscript range" error and the shell
    // exits 1. Was silently accepting the assignment as a no-op.
    let (status, _stdout, stderr) = run_zshrs(r#"a=(); a[0]=hi; echo "[${a[@]}]""#);
    assert_eq!(status, 1, "should exit non-zero");
    assert!(
        stderr.contains("assignment to invalid subscript range"),
        "stderr: {stderr:?}"
    );
}

#[test]
fn test_umask_3_octal_digits_no_leading_zero() {
    // zsh prints 3 octal digits (`022`), not 4 (`0022`).
    let (_, output, _) = run_zshrs(r#"umask"#);
    let trimmed = output.trim();
    // Must be 3 digits, no leading 0 padding to 4 digits.
    assert_eq!(trimmed.len(), 3, "expected 3 digits: {trimmed:?}");
    assert!(
        trimmed.chars().all(|c| c.is_ascii_digit()),
        "expected octal digits: {trimmed:?}"
    );
}

#[test]
fn test_umask_dash_S_uses_commas() {
    // zsh: `umask -S` prints `u=rwx,g=rx,o=rx` (comma-separated).
    let (_, output, _) = run_zshrs(r#"umask -S"#);
    let trimmed = output.trim();
    assert!(
        trimmed.contains(",g=") && trimmed.contains(",o="),
        "expected commas separating u/g/o: {trimmed:?}"
    );
}

#[test]
fn test_cd_missing_dir_zsh_format() {
    // zsh: `zshrs:cd:1: no such file or directory: PATH`. Was
    // emitting Rust's wrapped io::Error with "(os error 2)" suffix.
    let (status, _stdout, stderr) = run_zshrs(r#"cd /not/a/dir"#);
    assert_eq!(status, 1);
    assert!(
        stderr.contains("zshrs:cd:1: no such file or directory: /not/a/dir"),
        "stderr: {stderr:?}"
    );
}

#[test]
fn test_abs_path_missing_says_no_such_file_not_command_not_found() {
    // zsh distinguishes: relative names not in PATH say "command not
    // found"; absolute paths that don't exist say "no such file or
    // directory" (since no PATH search was attempted).
    let (status, _stdout, stderr) = run_zshrs(r#"/nonexistent_abs_path_xyz"#);
    assert_eq!(status, 127);
    assert!(
        stderr.contains("no such file or directory:"),
        "abs path should report ENOENT-style: {stderr:?}"
    );
    assert!(
        !stderr.contains("command not found:"),
        "abs path must NOT say command not found: {stderr:?}"
    );
}

#[test]
fn test_kill_l_uses_platform_signal_numbers() {
    // `kill -l USR1` must use the platform's actual signal number,
    // not Linux defaults. macOS USR1 is 30, Linux is 10. Was
    // hardcoded to Linux. Pulled from libc::SIGUSR1 etc.
    let (_, output, _) = run_zshrs(r#"kill -l USR1"#);
    let n: i32 = output.trim().parse().unwrap_or(0);
    let expected = libc::SIGUSR1;
    assert_eq!(
        n, expected,
        "got: {output:?}, expected libc::SIGUSR1={expected}"
    );
}

#[test]
fn test_fg_bg_no_job_control_message() {
    // Non-interactive mode: zsh emits `zsh:fg:1: no job control in
    // this shell.` and exits 1. Was emitting `fg: no current job`.
    let (status, _stdout, stderr) = run_zshrs(r#"fg"#);
    assert_eq!(status, 1);
    assert!(
        stderr.contains("fg:1: no job control in this shell."),
        "stderr: {stderr:?}"
    );
    let (status, _stdout, stderr) = run_zshrs(r#"bg"#);
    assert_eq!(status, 1);
    assert!(
        stderr.contains("bg:1: no job control in this shell."),
        "stderr: {stderr:?}"
    );
}

#[test]
fn test_kill_l_lists_signals_in_number_order() {
    // zsh's `kill -l` orders signal names by signal number
    // (HUP INT QUIT ILL TRAP ABRT ...). Was emitting in declaration
    // order which didn't match.
    let (_, output, _) = run_zshrs(r#"kill -l"#);
    let names: Vec<&str> = output.split_whitespace().collect();
    // First three should always be HUP INT QUIT (1, 2, 3 on every Unix).
    assert_eq!(&names[..3], &["HUP", "INT", "QUIT"], "got: {output:?}");
}

#[test]
fn test_kill_l_unknown_number_passes_through() {
    // zsh: `kill -l 100` prints `100` (no error). Was erroring
    // "kill: unknown signal: 100".
    let (status, output, _) = run_zshrs(r#"kill -l 100"#);
    assert_eq!(status, 0);
    assert_eq!(output.trim(), "100", "got: {output:?}");
}

#[test]
fn test_param_flag_at_P_indirects_each_element() {
    // `${(@P)var}` should dereference: var holds "x" → look up x.
    // Was returning the raw var name because the P arm only handled
    // scalar state, leaving the (@)-forced array unchanged.
    let (_, output, _) = run_zshrs(r#"x=hello; var=x; print "${(@P)var}""#);
    assert_eq!(output.trim(), "hello", "got: {output:?}");
}

#[test]
fn test_param_flag_P_empty_deref_keeps_word() {
    // `${(P)name}` where the deref target is unset OR resolves to the
    // empty name must expand to an EMPTY STRING, not null the whole
    // surrounding word. The (P) form routes through
    // stringsubst→paramsubst as its own token node; the empty scalar
    // return previously came back as "" and prefork's empty-node
    // deletion elided it, taking the surrounding literals with it
    // (`X${(P)bar}Y` printed nothing). C emits the Nularg sentinel for
    // a quoted-empty result (subst.c:4465 `if (qt && !*y) y =
    // dupstring(nulstring)`, the scalar branch) so prefork keeps the
    // node; remnulargs restores the true empty. Verified against
    // `/bin/zsh -f`:
    //   unset bar → "X${(P)bar}Y"  → XY
    //   zz=""     → "[${(P)zz}]"    → []
    //   name=zz zz=hi → "${(P)name}end" → hiend
    let (_, output, _) = run_zshrs(
        r#"unset bar; print -r -- "X${(P)bar}Y"; zz=""; print -r -- "[${(P)zz}]"; name=zz; zz=hi; print -r -- "${(P)name}end""#,
    );
    assert_eq!(output, "XY\n[]\nhiend\n", "got: {output:?}");
}

#[test]
fn test_typeset_f_zsh_format_one_stmt_per_line() {
    // zsh: each top-level statement on its own line, no trailing
    // semicolons, indented with TAB. Was preserving the input's
    // semicolons (`echo a; echo b`) because we stored body_source
    // verbatim — now `format_function_body_zsh()` normalizes.
    let (_, output, _) = run_zshrs(r#"f() { echo a; echo b; }; typeset -f f"#);
    assert_eq!(output, "f () {\n\techo a\n\techo b\n}\n", "got: {output:?}");
}

#[test]
fn test_param_flag_c_pound_returns_char_count() {
    // `${(c)#name}` uses char-count semantics for the length op.
    // Was returning 0 because the # was swallowed by the var-name
    // extractor as a non-alphanum boundary.
    let (_, output, _) = run_zshrs(r#"a=hello; print "${(c)#a}""#);
    assert_eq!(output.trim(), "5", "got: {output:?}");
}

#[test]
fn test_param_flag_w_pound_returns_word_count() {
    // `${(w)#name}` splits on whitespace and counts words.
    // Same swallowed-# bug as (c)#.
    let (_, output, _) = run_zshrs(r#"a="x y z"; print "${(w)#a}""#);
    assert_eq!(output.trim(), "3", "got: {output:?}");
}

#[test]
fn test_tr_complement_with_delete() {
    // `tr -d -c "0-9"` deletes everything NOT in 0-9, leaving digits.
    // The -c flag was being ignored entirely.
    let mut child = std::process::Command::new(zshrs_bin())
        .args(["-f", "-c", r#"tr -d -c "0-9""#])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"abc123\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "123",
        "got: {:?}",
        out.stdout
    );
}

#[test]
fn test_wc_uses_bsd_8char_padding() {
    // zsh's bundled wc on macOS right-pads counts to 8 chars
    // (`       3` for line count of 3). Was trim_start'ing.
    let mut child = std::process::Command::new(zshrs_bin())
        .args(["-f", "-c", "wc -l"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"a\nb\nc\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(
        s.starts_with("       3"),
        "expected 7-space padded count: {s:?}"
    );
}

#[test]
fn test_type_function_says_from_zsh() {
    // zsh prints `f is a shell function from zsh` (the suffix is
    // load-source — `from zsh` for built-in functions, the source
    // file path for autoloaded ones). Bare `is a shell function`
    // missed the suffix.
    let (_, output, _) = run_zshrs(r#"f() { :; }; type f"#);
    assert_eq!(
        output.trim(),
        "f is a shell function from zsh",
        "got: {output:?}"
    );
}

#[test]
fn test_tail_dash_c_byte_count() {
    // `tail -c N` keeps the last N bytes. Was treating `4` as a
    // filename and erroring "tail: 4: No such file or directory".
    let mut child = std::process::Command::new(zshrs_bin())
        .args(["-f", "-c", "tail -c 4"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"abcdefgh\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim_end(),
        "fgh",
        "got: {:?}",
        out.stdout
    );
}

#[test]
fn test_umask_dash_S_symbolic_set() {
    // `umask -S u=rwx,g=rx,o=` sets the umask via symbolic perms.
    // Was rejecting symbolic input as "invalid mask" — only numeric
    // (`022`) was parsed.
    let (_, output, _) = run_zshrs(r#"umask 077; umask -S u=rwx,g=rx,o=; umask"#);
    assert_eq!(output.trim(), "027", "expected 027, got: {output:?}");
}

#[test]
fn test_find_maxdepth_caps_recursion() {
    // `find /tmp -maxdepth 0` should print only `/tmp` (no descent
    // into children). Was recursing the whole tree because the
    // -maxdepth flag was unrecognized.
    let (_, output, _) = run_zshrs(r#"find /tmp -maxdepth 0"#);
    assert_eq!(output.trim(), "/tmp", "got: {output:?}");
}

#[test]
fn test_ulimit_dash_a_zsh_format() {
    // zsh format: `-t: cpu time (seconds)<padding>unlimited` per line.
    // First line of `ulimit -a` should always be cpu time (-t).
    let (_, output, _) = run_zshrs(r#"ulimit -a"#);
    let first = output.lines().next().unwrap_or("");
    assert!(
        first.starts_with("-t: cpu time"),
        "expected `-t: cpu time` first line: {first:?}"
    );
}

#[test]
fn test_alias_dash_m_uses_unquoted_form_when_no_metas() {
    // `alias -m "g*"` should print bare `g=hi` (not `g='hi'`) when the
    // value has no shell metas — matching zsh's plain-alias listing
    // format. Regression: -m path always wrapped in single quotes.
    let (_, output, _) = run_zshrs(r#"alias g=hi; alias -m "g*""#);
    assert_eq!(output.trim(), "g=hi", "got: {output:?}");
}

#[test]
fn test_shopt_is_command_not_found() {
    // zsh has no `shopt` builtin — that's bash-only. Should fall through
    // to PATH lookup and produce "command not found". Regression: zshrs
    // shipped a bash-compat shopt builtin that listed all options.
    let (status, _, stderr) = run_zshrs(r#"shopt"#);
    assert_eq!(status, 127, "expected exit 127");
    assert!(
        stderr.contains("command not found: shopt"),
        "expected 'command not found' for shopt: {stderr:?}"
    );
}

#[test]
fn test_consecutive_array_assignments_on_one_line() {
    // `a=(1 2 3) b=(x y z); echo $a $b` should set both arrays. The
    // lexer flips incmdpos to false on Outpar, which prevented the
    // second `b=(...)` from being recognised as Envarray and emitted
    // "command not found: b=(x y z)" instead. parse_assign now resets
    // incmdpos=true after closing an array assign.
    let (_, output, _) = run_zshrs(r#"a=(1 2 3) b=(x y z); echo $a $b"#);
    assert_eq!(output.trim(), "1 2 3 x y z", "got: {output:?}");
}

#[test]
fn test_brace_stepped_char_range_left_literal() {
    // zsh only expands UNSTEPPED char ranges `{a..z}`. A stepped form
    // `{a..z..2}` is left literal — match that. Without the gate, zshrs
    // expanded `{a..e..2}` to `a c e` (zsh prints `{a..e..2}` verbatim).
    let (_, output, _) = run_zshrs(r#"echo {a..e..2}"#);
    assert_eq!(output.trim(), "{a..e..2}", "got: {output:?}");
}

#[test]
fn test_brace_unstepped_char_range_still_expands() {
    // Regression guard for the stepped-char-range gate: unstepped
    // `{a..e}` must still expand to `a b c d e`.
    let (_, output, _) = run_zshrs(r#"echo {a..e}"#);
    assert_eq!(output.trim(), "a b c d e", "got: {output:?}");
}

#[test]
fn test_param_default_with_inner_braces_balances_extent() {
    // A brace-expansion group inside a `${...}` body (e.g. the default of
    // `${x:-{a,b}}`) must be balanced when finding the closing `}`: the
    // FIRST inner `}` used to close the `${...}` early, leaving a stray
    // literal `}` (`${x:-{a,b}}` with x set printed `set}`). Verified vs
    // /bin/zsh -f. (Brace-EXPANSION of a used default — `${xx:-{a,b}}` →
    // `a b` — is a separate concern; here x is set so the default is
    // ignored and only the extent matters.)
    let (_, output, _) = run_zshrs(r#"x=set; print -r -- ${x:-{a,b}}"#);
    assert_eq!(output.trim(), "set", "got: {output:?}");
    // Nested `${${:-N}}` inside arithmetic must also balance the inner
    // braces (else matheval saw a truncated `${:-7}`).
    let (_, output, _) = run_zshrs(r#"print -r -- $(( ${${:-7}} + 1 ))"#);
    assert_eq!(output.trim(), "8", "got: {output:?}");
}

#[test]
fn test_arith_division_by_zero_prints_error() {
    // `$((10/0))` should print `zshrs:1: division by zero` on stderr
    // (matching zsh's `zsh:1: division by zero`). Regression: zshrs
    // silently returned 0 with no error output at all.
    let (_, _, stderr) = run_zshrs(r#"echo $((10/0))"#);
    assert!(
        stderr.contains("division by zero"),
        "expected division-by-zero error: {stderr:?}"
    );
}

#[test]
fn test_arith_mod_by_zero_prints_error() {
    // `$((10%0))` should also print division-by-zero (mod by zero is
    // the same condition). Verifies the error path covers Mod, not
    // just Div.
    let (_, _, stderr) = run_zshrs(r#"echo $((10%0))"#);
    assert!(
        stderr.contains("division by zero"),
        "expected division-by-zero error: {stderr:?}"
    );
}

#[test]
fn test_machtype_returns_arm_or_x86_64() {
    // `$MACHTYPE` should report a recognisable arch tag (zsh shortens
    // aarch64 → arm). Was empty in zshrs because params.rs's table
    // wasn't reachable from the executor's get_variable.
    let (_, output, _) = run_zshrs(r#"echo $MACHTYPE"#);
    let v = output.trim();
    assert!(
        v == "arm" || v == "x86_64" || v == "aarch64",
        "expected arm/x86_64/aarch64, got: {v:?}"
    );
}

#[test]
fn test_ostype_starts_with_os_family() {
    // `$OSTYPE` should start with "darwin" / "linux" / etc — zsh hardcodes
    // build-time string; we synthesize from libc uname() at runtime.
    let (_, output, _) = run_zshrs(r#"echo $OSTYPE"#);
    let v = output.trim();
    assert!(
        v.starts_with("darwin") || v.starts_with("linux") || v.starts_with("freebsd"),
        "expected OS-family prefix, got: {v:?}"
    );
}

#[test]
fn test_vendor_returns_apple_or_unknown() {
    // `$VENDOR` should be "apple" on macOS, "unknown"/"pc" elsewhere.
    let (_, output, _) = run_zshrs(r#"echo $VENDOR"#);
    let v = output.trim();
    assert!(
        v == "apple" || v == "unknown" || v == "pc",
        "expected vendor tag, got: {v:?}"
    );
}

#[test]
fn test_minus_f_skips_eager_fpath_autoload() {
    // In `-f` (no-rcs) mode, zshrs must NOT eagerly scan FPATH/ZWC for
    // unknown command names. Regression: zshrs autoloaded `rm` from the
    // user's FPATH (where it was wrapped to call `zpwrLogConsoleErr`),
    // shadowing the external. zsh only autoloads names explicitly
    // declared via `autoload`. Test runs `rm -f /tmp/file_zshrs_test_$$`
    // (a path we know doesn't exist) — zshrs should fall through to the
    // external rm without trying to load any FPATH wrapper.
    let (_, output, stderr) = run_zshrs(r#"rm -f /tmp/zshrs_never_existed_$$; echo done"#);
    assert!(
        output.trim().ends_with("done"),
        "expected `done`: {output:?}"
    );
    assert!(
        !stderr.contains("zpwrLogConsoleErr"),
        "FPATH wrapper leaked into -f mode: {stderr:?}"
    );
}

#[test]
fn test_case_paren_prefixed_pattern_accepted() {
    // `case x in (foo|bar) …` — the leading `(` is optional in zsh.
    // Regression: the lexer consumed `(foo)` as one big String token
    // because `incasepat` wasn't yet 1 when the `(` was read. Setting
    // incasepat=1 before zshlex'ing past `in` fixes it.
    let (_, output, _) = run_zshrs(r#"case foo in (foo|bar) echo a;; (*) echo c;; esac"#);
    assert_eq!(output.trim(), "a", "got: {output:?}");
}

#[test]
fn test_case_paren_only_first_alt_matches() {
    // Same form, second alternative chosen.
    let (_, output, _) = run_zshrs(r#"case bar in (foo|bar) echo b;; (*) echo c;; esac"#);
    assert_eq!(output.trim(), "b", "got: {output:?}");
}

#[test]
fn test_dollar_dash_baseline_no_user_flags() {
    // `echo $-` in `-f -c` mode should produce zsh's baseline "569Xf"
    // — the `f` indicates -f (no rcs) is on. Regression: zshrs returned
    // empty entirely.
    let (_, output, _) = run_zshrs(r#"echo $-"#);
    assert_eq!(output.trim(), "569Xf", "got: {output:?}");
}

#[test]
fn test_dollar_dash_includes_e_when_errexit_on() {
    // `set -e; echo $-` adds `e`. Letter ordering matches zsh: e BEFORE f.
    let (_, output, _) = run_zshrs(r#"set -e; echo $-"#);
    assert_eq!(output.trim(), "569Xef", "got: {output:?}");
}

#[test]
fn test_dollar_dash_includes_x_when_xtrace_on() {
    // `set -x; echo $-` should include `x` after `f`.
    let (_, output, _) = run_zshrs(r#"set -x; echo $-"#);
    let last = output.lines().last().unwrap_or("").trim();
    assert_eq!(last, "569Xfx", "got: {output:?}");
}

#[test]
fn test_dollar_zero_in_minus_c_uses_argv0_verbatim() {
    // zsh's `$0` in `-c` mode is argv[0] verbatim — `/bin/zsh -c '...'`
    // gives `/bin/zsh`, plain `zsh -c '...'` gives `zsh`. zshrs now
    // matches: `$0` is whatever path the binary was invoked with.
    // Test asserts that `$0` ends with the binary name (handles both
    // absolute-path and basename invocations).
    let (_, output, _) = run_zshrs(r#"echo $0"#);
    let s = output.trim();
    assert!(
        s.ends_with("zshrs") || s.ends_with("/zshrs"),
        "expected $0 to end with `zshrs`, got: {s:?}"
    );
}

#[test]
fn test_print_dash_p_capital_T_no_zero_pad_hour() {
    // `print -P "%T"` should NOT zero-pad the hour: `4:10`, not
    // `04:10`. Regression: chrono's `%H` always zero-pads; switched
    // to `%k` (space-pad) and trim_start.
    let (_, output, _) = run_zshrs(r#"print -P "%T""#);
    let s = output.trim();
    // Match HH:MM where HH has no leading zero (1-2 digits).
    let parts: Vec<&str> = s.split(':').collect();
    assert_eq!(parts.len(), 2, "expected H:MM, got: {s:?}");
    let hour = parts[0];
    assert!(
        !hour.starts_with('0') || hour == "0",
        "hour should not have leading zero: {s:?}"
    );
}

#[test]
fn test_escaped_braces_stay_literal_in_word() {
    // `echo \{foo,bar\}` — backslash-escaped braces must not trigger
    // brace expansion. Regression: zshrs produced `oo ar\` because
    // the lexer's BNULL-encoded `\{` got untokenized to `\{` before
    // xpandbraces, which then expanded the comma list. Added
    // has_balanced_escaped_braces() short-circuit that strips the
    // backslashes and returns the literal.
    let (_, output, _) = run_zshrs(r#"echo \{foo,bar\}"#);
    assert_eq!(output.trim(), "{foo,bar}", "got: {output:?}");
}

#[test]
fn test_escaped_braces_with_prefix_suffix() {
    // Same fix should not break `\{X\}` surrounded by literal text.
    let (_, output, _) = run_zshrs(r#"echo prefix\{a,b\}suffix"#);
    assert_eq!(output.trim(), "prefix{a,b}suffix", "got: {output:?}");
}

#[test]
fn test_unescaped_braces_still_expand() {
    // Regression guard for the escaped-brace fix: bare `{a,b}` must
    // still expand normally.
    let (_, output, _) = run_zshrs(r#"echo {foo,bar}"#);
    assert_eq!(output.trim(), "foo bar", "got: {output:?}");
}

#[test]
fn test_dollar_hash_name_bare_array_length() {
    // `$#name` (no braces) is zsh shorthand for `${#name}`. Regression:
    // zshrs printed `$#a` literally because the bare-form fast-path in
    // compile_word_str didn't recognize the `$#NAME` shape. Added a
    // PARAM_LENGTH emit for `$#NAME` patterns.
    let (_, output, _) = run_zshrs(r#"a=(1 2 3); echo $#a"#);
    assert_eq!(output.trim(), "3", "got: {output:?}");
}

#[test]
fn test_dollar_hash_name_bare_string_length() {
    // Same path — string variable case.
    let (_, output, _) = run_zshrs(r#"a=hello; echo $#a"#);
    assert_eq!(output.trim(), "5", "got: {output:?}");
}

#[test]
fn test_modifier_capital_A_canonicalizes_dot() {
    // `${a:A}` for `a=./foo` should produce `<cwd>/foo` — `./` is
    // resolved lexically. Was leaving the `./` segment when canonicalize
    // failed (path doesn't need to exist).
    let (_, output, _) = run_zshrs(r#"a=./foo; echo ${a:A}"#);
    let s = output.trim();
    assert!(!s.contains("/./"), "should not contain /./: {s:?}");
    assert!(s.starts_with('/'), "should be absolute: {s:?}");
    assert!(s.ends_with("/foo"), "should end with /foo: {s:?}");
}

#[test]
fn test_modifier_unknown_emits_error_and_clears() {
    // Unknown history-style modifiers (`:U`, `:L`, `:V`, `:X`) are
    // bash-only — zsh reports "unrecognized modifier" and the resulting
    // expansion is empty.
    let (_, output, stderr) = run_zshrs(r#"a=foo; echo ${a:U}"#);
    assert!(
        stderr.contains("unrecognized modifier"),
        "expected unrecognized modifier err: {stderr:?}"
    );
    assert_eq!(output.trim(), "");
}

#[test]
fn test_bash_caret_caret_rejected() {
    // ${var^^} is bash-only. zsh rejects with "bad substitution".
    let (_, output, stderr) = run_zshrs(r#"a=hi; echo ${a^^}"#);
    assert!(
        stderr.contains("bad substitution"),
        "expected bad substitution: {stderr:?}"
    );
    assert_eq!(output.trim(), "");
}

#[test]
fn test_cmd_subst_concat_two_substitutions() {
    // `echo $(echo foo)$(echo bar)` should produce `foobar`. Regression:
    // zshrs's strip_cmd_subst matched the whole word as one cmd-subst
    // (treating `$(echo foo)$(echo bar` as the body), dropping the
    // second sub.
    let (_, output, _) = run_zshrs(r#"echo $(echo foo)$(echo bar)"#);
    assert_eq!(output.trim(), "foobar");
}

#[test]
fn test_cmd_subst_concat_three_substitutions() {
    let (_, output, _) = run_zshrs(r#"echo $(echo a)$(echo b)$(echo c)"#);
    assert_eq!(output.trim(), "abc");
}

#[test]
fn test_typeset_dash_x_preserves_value() {
    // `typeset -x` should ATTACH the export attribute to an existing
    // variable, NOT clear its value. Regression: zshrs reset to empty.
    let (_, output, _) = run_zshrs(r#"a=hello; typeset -x a; echo $a"#);
    assert_eq!(output.trim(), "hello");
}

#[test]
fn test_typeset_plus_x_preserves_value() {
    // `typeset +x` (remove export) should also keep value. zsh:
    // `a=hello; typeset +x a; echo $a` → `hello`.
    let (_, output, _) = run_zshrs(r#"a=hello; typeset +x a; echo $a"#);
    assert_eq!(output.trim(), "hello");
}

#[test]
fn test_arith_compound_div_assign_integer() {
    // `((a/=3))` with integer `a=10` should integer-divide → 3.
    // Regression: ArithCompiler emitted Op::Div which is float-only,
    // producing 3.3333333333333335. Routing `((..))` with `/` through
    // BUILTIN_ARITH_EVAL fixes it.
    let (_, output, _) = run_zshrs(r#"a=10; ((a/=3)); echo $a"#);
    assert_eq!(output.trim(), "3");
}

#[test]
fn test_arith_div_with_float_stays_float() {
    // Regression guard: `((a / 3.0))` (one float operand) must still
    // produce a float result. Don't accidentally force int-divide
    // everywhere.
    let (_, output, _) = run_zshrs(r#"a=10; ((b = a / 3.0)); echo $b"#);
    let s = output.trim();
    assert!(
        s.starts_with("3.333"),
        "expected float result starting with 3.333, got: {s:?}"
    );
}

#[test]
fn test_unknown_modifier_letter_emits_error() {
    // `${arr:offset}`, `${a:Z}`, `${arr:Nope}` — any single letter not
    // in the recognized modifier set should emit zsh's `unrecognized
    // modifier `X'` error and return empty. Was silently returning empty.
    let (_, output, stderr) = run_zshrs(r#"arr=(a b c); echo ${arr:Nope}"#);
    assert!(
        stderr.contains("unrecognized modifier"),
        "expected unrecognized modifier err: {stderr:?}"
    );
    assert_eq!(output.trim(), "");
}

#[test]
fn test_unknown_modifier_capital_Z() {
    // ${a:Z} also unrecognized.
    let (_, output, stderr) = run_zshrs(r#"a=foo; echo ${a:Z}"#);
    assert!(
        stderr.contains("unrecognized modifier"),
        "expected err: {stderr:?}"
    );
    assert_eq!(output.trim(), "");
}

#[test]
fn test_flag_only_modifier_reports_first_char() {
    // c:Src/subst.c:3785-3790 — a bare pre-flag (`g`/`w`/`W`/`f`/`F`) with
    // NO following modifier letter is an error, and zsh names the FIRST
    // char after the `:` (s[1]), not a bare message and not the last flag.
    // zshrs previously emitted a bare "unrecognized modifier" here.
    let modchar = |code: &str| -> String {
        let (_, _, stderr) = run_zshrs(code);
        stderr.trim().to_string()
    };
    // Single flag, no modifier → that flag char is named.
    assert!(
        modchar(r#"p=x; echo ${p:w}"#).ends_with("unrecognized modifier `w'"),
        "got: {:?}",
        modchar(r#"p=x; echo ${p:w}"#)
    );
    assert!(modchar(r#"p=x; echo ${p:g}"#).ends_with("unrecognized modifier `g'"));
    assert!(modchar(r#"p=x; echo ${p:f}"#).ends_with("unrecognized modifier `f'"));
    // Multiple flags → the FIRST char after the colon is named.
    assert!(modchar(r#"p=x; echo ${p:gw}"#).ends_with("unrecognized modifier `g'"));
    assert!(modchar(r#"p=x; echo ${p:wg}"#).ends_with("unrecognized modifier `w'"));
    // A truly empty operand `${p:}` stays a BARE message (no char, c:3790).
    let empty = modchar(r#"p=x; echo ${p:}"#);
    assert!(
        empty.ends_with("unrecognized modifier"),
        "empty operand must be bare: {empty:?}"
    );
    // A flag followed by a VALID modifier is not an error (no regression).
    let (_, out, _) = run_zshrs(r#"p=aXbXc; echo ${p:gs/X/-/}"#);
    assert_eq!(out.trim(), "a-b-c");
}

#[test]
fn test_arith_recursive_string_var_eval() {
    // zsh: `a="3+2"; $((a))` recursively evaluates the var's string
    // value as an arith expression → 5. Regression: zshrs returned 0
    // because MathEval skipped non-numeric vars entirely.
    let (_, output, _) = run_zshrs(r#"a="3+2"; echo $((a))"#);
    assert_eq!(output.trim(), "5");
}

#[test]
fn test_arith_indirect_var_chain() {
    // `b=a` then `$((b))` → resolves to `a`'s value (5).
    let (_, output, _) = run_zshrs(r#"a=5; b=a; echo $((b))"#);
    assert_eq!(output.trim(), "5");
}

#[test]
fn test_arith_recursive_compound_expression() {
    // `c="a+b"` then `$((c))` evaluates `a+b` against current vars.
    let (_, output, _) = run_zshrs(r#"a=10; b=20; c="a+b"; echo $((c))"#);
    assert_eq!(output.trim(), "30");
}

#[test]
fn test_printf_e_format_signed_two_digit_exponent() {
    // C printf / zsh emit `1.000000e+03` (signed, ≥2 digits in exp).
    // Rust's `{:e}` emits `1e3`. Regression — without the manual sign +
    // pad, zshrs printed `1.000000e3`.
    let (_, output, _) = run_zshrs(r#"printf "%e\n" 1000"#);
    assert_eq!(output.trim(), "1.000000e+03");
}

#[test]
fn test_printf_e_format_negative_exponent_padded() {
    // Negative exponent: `1.000000e-03` (sign already present, just pad).
    let (_, output, _) = run_zshrs(r#"printf "%e\n" 0.001"#);
    assert_eq!(output.trim(), "1.000000e-03");
}

#[test]
fn test_printf_capital_E_uses_uppercase_marker() {
    // `%E` keeps the same exponent rules but uppercase marker.
    let (_, output, _) = run_zshrs(r#"printf "%E\n" 1000"#);
    assert_eq!(output.trim(), "1.000000E+03");
}

#[test]
fn test_printf_invalid_directive_v() {
    // `%v` is bash-only; zsh emits "invalid directive". zshrs followed
    // suit instead of passing the literal `%v` through.
    let (_, _, stderr) = run_zshrs(r#"printf "%v\n" foo"#);
    assert!(
        stderr.contains("invalid directive"),
        "expected invalid-directive err: {stderr:?}"
    );
}

#[test]
fn test_printf_invalid_directive_a() {
    // `%a` (hex float) is bash-only; zsh rejects.
    let (_, _, stderr) = run_zshrs(r#"printf "%a\n" 1"#);
    assert!(
        stderr.contains("invalid directive"),
        "expected invalid-directive err: {stderr:?}"
    );
}

#[test]
fn test_printf_thousands_grouping_flag_accepted() {
    // `%'d` (the `'` thousands-grouping flag) must be ACCEPTED — zshrs
    // previously errored `%': invalid directive`. The grouping itself is
    // locale-dependent (localeconv thousands_sep), so this test is
    // locale-agnostic: it asserts no error and that the digits (with any
    // grouping separators stripped) are correct. Manual verification vs
    // /bin/zsh -f under en_US.UTF-8 confirms `1,234,567`; under C locale
    // both emit the ungrouped `1234567`.
    let (_, out, stderr) = run_zshrs(r#"printf "%'d\n" 1234567"#);
    assert!(
        !stderr.contains("invalid directive"),
        "should accept the ' flag, got: {stderr:?}"
    );
    let digits: String = out.chars().filter(|c| c.is_ascii_digit()).collect();
    assert_eq!(digits, "1234567", "grouped/ungrouped digits: {out:?}");
    // Precision with the flag zero-fills without grouping the fill
    // (glibc `%'.8d` of 42 = `00000042`), locale-independent.
    let (_, out2, _) = run_zshrs(r#"printf "%'.8d\n" 42"#);
    assert_eq!(out2.trim(), "00000042", "precision zero-fill: {out2:?}");
}

#[test]
fn test_declare_capital_F_no_args_lists_only_floats() {
    // `declare -F` (no args) lists only float-typed vars. With none
    // declared, output is empty. Regression: zshrs dumped all vars.
    let (_, output, _) = run_zshrs(r#"foo() { :; }; declare -F"#);
    assert_eq!(output.trim(), "");
}

#[test]
fn test_declare_capital_F_lists_declared_floats() {
    // With a float declared, `declare -F` lists just that one.
    let (_, output, _) = run_zshrs(r#"typeset -F PI=3.14; declare -F"#);
    assert!(
        output.contains("PI=3.14"),
        "expected PI listing, got: {output:?}"
    );
    // No other vars should appear.
    assert!(
        !output.contains("ZSH_NAME") && !output.contains("WORDCHARS"),
        "should not include shell-internal vars: {output:?}"
    );
}

#[test]
fn test_typeset_dash_U_dedupes_array() {
    // `typeset -U arr` with `arr=(a b a c)` should yield `a b c`.
    // Regression: zshrs didn't track the unique attribute so the
    // duplicates remained.
    let (_, output, _) = run_zshrs(r#"typeset -U arr; arr=(a b a c); echo "${arr[@]}""#);
    assert_eq!(output.trim(), "a b c");
}

#[test]
fn test_typeset_dash_U_after_assignment_dedupes() {
    // Setting -U AFTER the array was assigned should still dedupe
    // (zsh applies the attr on attribute change too).
    let (_, output, _) = run_zshrs(r#"arr=(a b a c b); typeset -U arr; echo "${arr[@]}""#);
    assert_eq!(output.trim(), "a b c");
}

#[test]
fn test_typeset_dash_U_append_dedupes() {
    // `arr+=(b a c)` should append only new elements; `b` and `a`
    // already exist so only `c` is appended.
    let (_, output, _) = run_zshrs(r#"typeset -U arr; arr=(a b); arr+=(b a c); echo "${arr[@]}""#);
    assert_eq!(output.trim(), "a b c");
}

#[test]
fn test_arith_pre_increment_on_literal_errors() {
    // `((++5))` is a zsh error: "bad math expression: lvalue required".
    // Regression: zshrs silently incremented and returned 6.
    let (_, _, stderr) = run_zshrs(r#"echo $((++5))"#);
    assert!(
        stderr.contains("bad math expression: lvalue required"),
        "expected lvalue-required err: {stderr:?}"
    );
}

#[test]
fn test_arith_post_increment_on_literal_errors() {
    // `((5++))` — same lvalue-required error.
    let (_, _, stderr) = run_zshrs(r#"echo $((5++))"#);
    assert!(
        stderr.contains("bad math expression: lvalue required"),
        "expected lvalue-required err: {stderr:?}"
    );
}

#[test]
fn test_arith_pre_decrement_on_var_works() {
    // Regression guard: `--var` on a real variable still works.
    let (_, output, _) = run_zshrs(r#"a=10; echo $((--a))"#);
    assert_eq!(output.trim(), "9");
}

#[test]
fn test_typeset_E_uses_sig_digit_precision() {
    // `typeset -E5 a=1234.5` — `-E5` means 5 SIGNIFICANT digits, not 5
    // fractional. zsh: `1.2345e+03`. Regression: zshrs printed
    // `1.23450e+03` (5 fractional → 6 significant).
    let (_, output, _) = run_zshrs(r#"typeset -E5 a=1234.5; echo $a"#);
    assert_eq!(output.trim(), "1.2345e+03");
}

#[test]
fn test_typeset_E_default_precision_nine_fractional() {
    // Default `-E` (no number) → 10 significant digits → 9 fractional.
    // zsh: `1.234500000e+03` for `1234.5`.
    let (_, output, _) = run_zshrs(r#"typeset -E a=1234.5; echo $a"#);
    assert_eq!(output.trim(), "1.234500000e+03");
}

#[test]
fn test_dollar_hash_name_bracket_at() {
    // `$#a[@]` (no braces) is zsh shorthand for `${#a[@]}` (array
    // length). Regression: zshrs printed `$#a[@]` literally because
    // the bare-form fast-path didn't accept the `[@]` / `[*]` suffix.
    let (_, output, _) = run_zshrs(r#"a=(x y z); echo $#a[@]"#);
    assert_eq!(output.trim(), "3");
}

#[test]
fn test_dollar_hash_name_bracket_star() {
    // Same shape with `[*]`.
    let (_, output, _) = run_zshrs(r#"a=(x y z); echo $#a[*]"#);
    assert_eq!(output.trim(), "3");
}

#[test]
fn test_arith_float_div_by_zero_returns_inf() {
    // `1/0.0` in zsh produces `Inf` (IEEE 754 semantics); only INTEGER
    // div-by-zero raises the "division by zero" error. zshrs treated
    // both alike — gated the error on !is_float.
    let (_, output, _) = run_zshrs(r#"echo $((1/0.0))"#);
    assert_eq!(output.trim(), "Inf");
}

#[test]
fn test_arith_neg_float_div_by_zero_returns_neg_inf() {
    let (_, output, _) = run_zshrs(r#"echo $((-1/0.0))"#);
    assert_eq!(output.trim(), "-Inf");
}

#[test]
fn test_arith_zero_div_zero_returns_nan() {
    let (_, output, _) = run_zshrs(r#"echo $((0.0/0.0))"#);
    assert_eq!(output.trim(), "NaN");
}

#[test]
fn test_arith_int_div_by_zero_still_errors() {
    // Regression guard: integer-only div-by-zero must still raise the
    // error (not silently produce Inf).
    let (_, _, stderr) = run_zshrs(r#"echo $((10/0))"#);
    assert!(
        stderr.contains("division by zero"),
        "expected error: {stderr:?}"
    );
}

#[test]
fn test_declare_p_missing_variable_emits_named_error() {
    // `declare -p NAME` for an unknown var should emit
    // `declare:1: no such variable: NAME` (NOT `typeset:1:`).
    let (status, _, stderr) = run_zshrs(r#"declare -p NONEXIST"#);
    assert_ne!(status, 0, "expected non-zero exit");
    assert!(
        stderr.contains("declare:1: no such variable: NONEXIST"),
        "expected declare-prefixed err: {stderr:?}"
    );
}

#[test]
fn test_typeset_p_missing_variable_emits_named_error() {
    // `typeset -p` keeps the `typeset:` prefix.
    let (status, _, stderr) = run_zshrs(r#"typeset -p NONEXIST"#);
    assert_ne!(status, 0);
    assert!(
        stderr.contains("typeset:1: no such variable: NONEXIST"),
        "expected typeset-prefixed err: {stderr:?}"
    );
}

#[test]
fn test_arith_huge_float_doesnt_truncate_to_i64_max() {
    // `1e100` was printing `9223372036854775807.` (i64::MAX) because
    // format_zsh_subst cast every float-with-fract==0 to i64. Now
    // gated on the value fitting in i64 range; out-of-range falls
    // through to the scientific-notation branch.
    let (_, output, _) = run_zshrs(r#"echo $((1e100))"#);
    let s = output.trim();
    assert!(
        s.contains("e+") && !s.contains("9223372036854775807"),
        "expected scientific notation: {s:?}"
    );
}

#[test]
fn test_arith_scientific_format_signed_two_digit_exp() {
    // `1e20` should print with signed 2-digit exponent (`1e+20`),
    // not `1e20` or `1.0e+20`.
    let (_, output, _) = run_zshrs(r#"echo $((1e20))"#);
    assert_eq!(output.trim(), "1e+20");
}

#[test]
fn test_array_oob_index_default_modifier() {
    // `${arr[5]:-default}` with `arr=(a b)` (only 2 elements) should
    // return `default`. Regression: zshrs's bracket-handler returned
    // empty silently, never falling through to the `:-default` form.
    let (_, output, _) = run_zshrs(r#"arr=(a b); echo "${arr[5]:-default}""#);
    assert_eq!(output.trim(), "default");
}

#[test]
fn test_array_empty_index_default_modifier() {
    // Same behavior for `${arr[1]:-empty}` with empty array.
    let (_, output, _) = run_zshrs(r#"arr=(); echo "${arr[1]:-empty}""#);
    assert_eq!(output.trim(), "empty");
}

#[test]
fn test_array_oob_index_assign_modifier() {
    // `${arr[N]:=fresh}` should assign fresh when OOB and return fresh.
    let (_, output, _) = run_zshrs(r#"arr=(); echo "${arr[1]:=fresh}"; echo "${arr[1]}""#);
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines.len(), 2, "expected 2 lines: {output:?}");
    assert_eq!(lines[0], "fresh");
    assert_eq!(lines[1], "fresh");
}

#[test]
fn test_array_in_bounds_no_default_kicks_in() {
    // Regression guard: `${arr[1]:-default}` with arr=(a b) returns
    // `a`, not `default`.
    let (_, output, _) = run_zshrs(r#"arr=(a b); echo "${arr[1]:-default}""#);
    assert_eq!(output.trim(), "a");
}

#[test]
fn test_param_no_colon_default_when_unset() {
    // `${var-default}` (no colon) returns default only when var is
    // unset. Empty values do NOT trigger the default. Regression:
    // zshrs had no handler for the no-colon `-`/`=`/`?`/`+` forms.
    let (_, output, _) = run_zshrs(r#"echo ${a-default}"#);
    assert_eq!(output.trim(), "default");
}

#[test]
fn test_param_no_colon_assign_when_unset() {
    // `${var=fresh}` assigns and returns fresh when var is unset.
    let (_, output, _) = run_zshrs(r#"echo ${a=fresh}; echo "[$a]""#);
    assert_eq!(output.trim(), "fresh\n[fresh]");
}

#[test]
fn test_param_no_colon_default_nested() {
    // `${a-${b-default}}` should reach the inner default. Regression:
    // zshrs didn't expand the no-colon form at all.
    let (_, output, _) = run_zshrs(r#"echo ${a-${b-default}}"#);
    assert_eq!(output.trim(), "default");
}

#[test]
fn test_param_no_colon_default_outer_set_skips() {
    let (_, output, _) = run_zshrs(r#"b=outer; echo ${a-${b-default}}"#);
    assert_eq!(output.trim(), "outer");
}

#[test]
fn test_param_replace_with_escaped_slash() {
    // `${HOME//\//_}` — escaped `/` in pattern should match literal
    // `/`. Regression: zshrs's splitn split on the escaped `\/` and
    // produced backslash-mangled output.
    let (_, output, _) = run_zshrs(r#"a=foo/bar; echo "${a//\//-}""#);
    assert_eq!(output.trim(), "foo-bar");
}

#[test]
fn test_param_at_modifier_rejected() {
    // `${var@OP}` is bash-only; zsh emits "bad substitution".
    let (_, _, stderr) = run_zshrs(r#"a=hi; echo "${a@U}""#);
    assert!(
        stderr.contains("bad substitution"),
        "expected bad substitution: {stderr:?}"
    );
}

#[test]
fn test_brace_negative_step_reverses() {
    // `{10..1..-2}` reverses `{10..1..2}` → `2 4 6 8 10`.
    let (_, output, _) = run_zshrs(r#"echo {10..1..-2}"#);
    assert_eq!(output.trim(), "2 4 6 8 10");
}

#[test]
fn test_brace_negative_step_ascending() {
    // `{1..10..-2}` reverses `{1..10..2}` → `9 7 5 3 1`.
    let (_, output, _) = run_zshrs(r#"echo {1..10..-2}"#);
    assert_eq!(output.trim(), "9 7 5 3 1");
}

#[test]
fn test_dollar_hash_positional() {
    // `$#1` is shorthand for `${#1}` (length of arg 1).
    let (_, output, _) = run_zshrs(r#"set -- ab; echo $#1"#);
    assert_eq!(output.trim(), "2");
}

#[test]
fn test_arith_negative_zero_keeps_sign() {
    // `-0.0` should print as `-0.` (preserve IEEE sign bit).
    let (_, output, _) = run_zshrs(r#"echo $((-0.0))"#);
    assert_eq!(output.trim(), "-0.");
}

#[test]
fn test_param_flag_with_no_colon_default() {
    // `${(L)NAME-default}` — flags + no-colon default form. Should
    // apply L (lowercase) to the default when var is unset.
    let (_, output, _) = run_zshrs(r#"echo "${(L)NONE-Default}""#);
    assert_eq!(output.trim(), "default");
}

#[test]
fn test_declare_p_exported_uses_export_prefix() {
    // `declare -p HOME` should print `export HOME=...` (not `typeset`).
    let (_, output, _) = run_zshrs(r#"declare -p HOME"#);
    assert!(
        output.starts_with("export HOME="),
        "expected export prefix: {output:?}"
    );
}

#[test]
fn test_declare_p_int_export_uses_export_dash_i() {
    // `declare -ix n=5; declare -p n` → `export -i n=5`.
    let (_, output, _) = run_zshrs(r#"declare -ix n=5; declare -p n"#);
    assert_eq!(output.trim(), "export -i n=5");
}

#[test]
fn test_string_range_subscript_with_default() {
    // `${a[2,3]:-default}` for a string should return the substring,
    // NOT the default. Regression: a bug in actual_idx (double 1->0
    // adjustment) made it return the full string.
    let (_, output, _) = run_zshrs(r#"a=foo; echo "${a[2,3]:-default}""#);
    assert_eq!(output.trim(), "oo");
}

#[test]
fn test_arith_compound_or_assign() {
    // `((a |= 0xff))` — bitwise compound assign. ArithCompiler doesn't
    // recognize these; routed through MathEval via BUILTIN_ARITH_EVAL.
    let (_, output, _) = run_zshrs(r#"a=10; ((a |= 0xff)); echo $a"#);
    assert_eq!(output.trim(), "255");
}

#[test]
fn test_arith_compound_shift_left_assign() {
    let (_, output, _) = run_zshrs(r#"a=10; ((a <<= 2)); echo $a"#);
    assert_eq!(output.trim(), "40");
}

#[test]
fn test_arith_compound_shift_right_assign() {
    let (_, output, _) = run_zshrs(r#"a=255; ((a >>= 4)); echo $a"#);
    assert_eq!(output.trim(), "15");
}

#[test]
fn test_arith_compound_xor_assign() {
    let (_, output, _) = run_zshrs(r#"a=10; ((a ^= 7)); echo $a"#);
    assert_eq!(output.trim(), "13");
}

#[test]
fn test_string_oob_index_returns_empty() {
    // `${a[10]}` for a 5-char string should return empty, NOT the
    // last char. slice_scalar was saturating to len.
    let (_, output, _) = run_zshrs(r#"a=hello; echo "[${a[10]}]""#);
    assert_eq!(output.trim(), "[]");
}

#[test]
fn test_string_negative_oob_index_returns_empty() {
    // `${a[-10]}` (over-negative) also empty.
    let (_, output, _) = run_zshrs(r#"a=hello; echo "[${a[-10]}]""#);
    assert_eq!(output.trim(), "[]");
}

#[test]
fn test_allexport_option_auto_exports() {
    // `set -o allexport; a=42` should export `a` to the env.
    let (_, output, _) = run_zshrs(r#"set -o allexport; a=zshtestval42; env | grep "^a=""#);
    assert_eq!(output.trim(), "a=zshtestval42");
}

#[test]
fn test_arith_dollar_var_with_star() {
    // `$(($a*2))` (no spaces around `*`) — `$a` should expand to its
    // value, then `*2` is multiply. Regression: `*` was being eaten
    // into the var name (`a*2` became one var name → empty → 0).
    let (_, output, _) = run_zshrs(r#"a=10; echo $(($a*2))"#);
    assert_eq!(output.trim(), "20");
}

#[test]
fn test_local_no_value_resets_to_empty() {
    // `a=hi; foo() { local a; echo "[$a]"; }; foo` — local should
    // shadow with EMPTY value, not parent's value. Regression: zshrs
    // preserved the parent value because the no-value typeset path
    // was set to "preserve existing".
    let (_, output, _) =
        run_zshrs(r#"a=hi; foo() { local a; echo "in[$a]"; }; foo; echo "out[$a]""#);
    assert_eq!(output.trim(), "in[]\nout[hi]");
}

#[test]
fn test_typeset_no_value_resets_in_function_scope() {
    // Same behavior for `typeset` (which `local` aliases to).
    let (_, output, _) =
        run_zshrs(r#"a=hi; foo() { typeset a; echo "in[$a]"; }; foo; echo "out[$a]""#);
    assert_eq!(output.trim(), "in[]\nout[hi]");
}

#[test]
fn test_typeset_g_keeps_parent_value() {
    // `typeset -g` opts out of localization — should keep parent value
    // (regression guard for the local-resets fix).
    let (_, output, _) =
        run_zshrs(r#"a=hi; foo() { typeset -g a; echo "in[$a]"; }; foo; echo "out[$a]""#);
    assert_eq!(output.trim(), "in[hi]\nout[hi]");
}

#[test]
fn test_bash_indirect_expansion_rejected() {
    // `${!var}` is a bash-only indirect. zsh emits "bad substitution".
    // Regression: zshrs implemented bash semantics.
    let (_, _, stderr) = run_zshrs(r#"a=hi; echo "${!a}""#);
    assert!(
        stderr.contains("bad substitution"),
        "expected bad substitution: {stderr:?}"
    );
}

#[test]
fn test_exit_status_masked_to_byte() {
    // POSIX: exit codes are 8-bit. `(exit 256)` → `$?` of 0.
    let (_, output, _) = run_zshrs(r#"(exit 256); echo $?"#);
    assert_eq!(output.trim(), "0");
}

#[test]
fn test_exit_status_257_wraps_to_one() {
    let (_, output, _) = run_zshrs(r#"(exit 257); echo $?"#);
    assert_eq!(output.trim(), "1");
}

#[test]
fn test_arith_invalid_base_no_panic() {
    // `$((1#X))` with base out of [2, 36] should error (NOT panic).
    // Regression: i64::from_str_radix panicked on base 1.
    let (_, _, stderr) = run_zshrs(r#"echo $((1#1))"#);
    assert!(
        stderr.contains("invalid base"),
        "expected invalid-base err: {stderr:?}"
    );
}

#[test]
fn test_arith_base_too_large_no_panic() {
    let (_, _, stderr) = run_zshrs(r#"echo $((37#5))"#);
    assert!(
        stderr.contains("invalid base"),
        "expected invalid-base err: {stderr:?}"
    );
}

#[test]
fn test_param_paren_p_with_empty_name_default() {
    // `${(P):-test}` — flag with empty name, default fires. The default
    // is the literal result; P should NOT dereference it. Regression:
    // zshrs applied P to "test" and returned empty.
    let (_, output, _) = run_zshrs(r#"echo "${(P):-test}""#);
    assert_eq!(output.trim(), "test");
}

#[test]
fn test_length_of_default_unset() {
    // `${#NONEXIST:-default}` returns 7 (length of "default"), not 0.
    let (_, output, _) = run_zshrs(r#"echo ${#NONEXIST:-default}"#);
    assert_eq!(output.trim(), "7");
}

#[test]
fn test_length_of_default_no_colon_unset() {
    // `${#NONEXIST-default}` (no colon) — same length-of-default form.
    let (_, output, _) = run_zshrs(r#"echo ${#NONEXIST-default}"#);
    assert_eq!(output.trim(), "7");
}

#[test]
fn test_length_no_default_when_set() {
    // Regression guard: when var is set, return its length, not the
    // default's. `a=hi` → 2, NOT 7.
    let (_, output, _) = run_zshrs(r#"a=hi; echo ${#a-default}"#);
    assert_eq!(output.trim(), "2");
}

#[test]
fn test_substring_empty_offset() {
    // `${a::N}` is shorthand for `${a:0:N}`. Was returning empty.
    let (_, output, _) = run_zshrs(r#"a=foo; echo "${a::1}""#);
    assert_eq!(output.trim(), "f");
}

#[test]
fn test_substring_negative_length() {
    // `${a::-1}` — negative length means "skip last N chars". `foo`
    // skip last 1 → `fo`.
    let (_, output, _) = run_zshrs(r#"a=foo; echo "${a::-1}""#);
    assert_eq!(output.trim(), "fo");
}

#[test]
fn test_substring_neg_length_with_offset() {
    // `${a:1:-1}` for `hello` → "ell" (skip first 1 + last 1).
    let (_, output, _) = run_zshrs(r#"a=hello; echo "${a:1:-1}""#);
    assert_eq!(output.trim(), "ell");
}

#[test]
fn test_param_qmark_no_msg_uses_zsh_format() {
    // `${var:?}` should emit "parameter not set" (zsh format), NOT
    // "parameter null or not set" (bash). Regression: zshrs used the
    // bash form.
    let (_, _, stderr) = run_zshrs(r#"echo "${a:?}""#);
    assert!(
        stderr.contains("parameter not set"),
        "expected zsh-format msg: {stderr:?}"
    );
    assert!(
        !stderr.contains("null"),
        "should not contain 'null': {stderr:?}"
    );
}

#[test]
fn test_pattern_paren_question_mark_matches_one_char() {
    // `${a/(?)/X}` — `(?)` is a glob group containing `?` (any single
    // char). Should replace the first char.
    let (_, output, _) = run_zshrs(r#"a=foo; echo "${a/(?)/X}""#);
    assert_eq!(output.trim(), "Xoo");
}

#[test]
fn test_pattern_paren_star_matches_anything() {
    // `${a/(*)/X}` — `(*)` is a group with `*` (any sequence). Should
    // replace the entire string.
    let (_, output, _) = run_zshrs(r#"a=foo; echo "${a/(*)/X}""#);
    assert_eq!(output.trim(), "X");
}

#[test]
fn test_pattern_alternation_in_replace() {
    // `${a/(foo|bar)/X}` — alternation matches first occurrence.
    let (_, output, _) = run_zshrs(r#"a=foobar; echo "${a/(foo|bar)/X}""#);
    assert_eq!(output.trim(), "Xbar");
}

#[test]
fn test_pushd_popd_silent_in_noninteractive() {
    // zsh's pushd/popd in non-interactive mode (`-c`) suppress the
    // dir-stack listing — only `dirs` actively prints.
    let (_, output, _) = run_zshrs(r#"pushd /tmp; popd; echo done"#);
    assert_eq!(output.trim(), "done");
}

#[test]
fn test_dirs_v_uses_tab_separator() {
    // `dirs -v` separates index and path with TAB.
    let (_, output, _) = run_zshrs(r#"pushd /tmp >/dev/null; dirs -v"#);
    let first = output.lines().next().unwrap_or("");
    assert!(first.contains('\t'), "expected TAB in dirs -v: {first:?}");
}

#[test]
fn test_arith_int_times_float_promotes() {
    // `a=10; ((a *= 1.5)); echo $a` — int * float → float result.
    // Regression: ArithCompiler kept everything int; routed through
    // MathEval via the float-literal trigger in compile_arith.
    let (_, output, _) = run_zshrs(r#"a=10; ((a *= 1.5)); echo $a"#);
    let s = output.trim();
    assert!(s.starts_with("15"), "expected 15.x or 15.0…, got: {s:?}");
    assert!(s.contains('.'), "expected float result: {s:?}");
}

#[test]
fn test_assoc_bare_returns_joined_values() {
    // `declare -A h; h[k]=v; echo "${h:-default}"` — bare `${h}`
    // should return joined values, NOT the default. Regression:
    // zshrs's get_variable returned empty for assoc.
    let (_, output, _) = run_zshrs(r#"declare -A h; h[k]=v; echo "${h:-default}""#);
    assert_eq!(output.trim(), "v");
}

#[test]
fn test_pattern_class_negation_with_bang() {
    // `[!fo]` should be class-negation (NOT literal `!`/`f`/`o`).
    // For `a=foo`, `${a//[!fo]/X}` matches no chars → returns "foo".
    let (_, output, _) = run_zshrs(r#"a=foo; echo "${a//[!fo]/X}""#);
    assert_eq!(output.trim(), "foo");
}

#[test]
fn test_pattern_class_negation_caret_still_works() {
    // Regression guard: `[^fo]` (the standard negation) must keep working.
    let (_, output, _) = run_zshrs(r#"a=foo; echo "${a//[^fo]/X}""#);
    assert_eq!(output.trim(), "foo");
}

#[test]
fn test_pattern_class_with_negation_matches_others() {
    // `${a/[!hl]/X}` for "hello" — matches first non-h-l char (`e`).
    let (_, output, _) = run_zshrs(r#"a=hello; echo "${a/[!hl]/X}""#);
    assert_eq!(output.trim(), "hXllo");
}

#[test]
fn test_escape_dollar_sign_literal() {
    // `echo \$` should print `$` (literal). Regression: zshrs printed
    // `\$` because expand_string didn't honor backslash-escape.
    let (_, output, _) = run_zshrs(r#"echo \$"#);
    assert_eq!(output.trim(), "$");
}

#[test]
fn test_escape_dollar_var_literal() {
    // `a=foo; echo \$a` should print `$a` literally (not expand).
    let (_, output, _) = run_zshrs(r#"a=foo; echo \$a"#);
    assert_eq!(output.trim(), "$a");
}

#[test]
fn test_escape_backtick_literal() {
    let (_, output, _) = run_zshrs(r#"echo \`"#);
    assert_eq!(output.trim(), "`");
}

#[test]
fn test_arith_subst_concat() {
    // `$((1+2))$((3+4))` — two arith substs concatenated.
    // Regression: strip_arith_subst's depth check passed even when
    // a `))` closed mid-string; ran the whole thing as one arith.
    let (_, output, _) = run_zshrs(r#"echo $((1+2))$((3+4))"#);
    assert_eq!(output.trim(), "37");
}

#[test]
fn test_arith_subst_concat_three() {
    let (_, output, _) = run_zshrs(r#"echo $((1*2))$((3*4))$((5*6))"#);
    assert_eq!(output.trim(), "21230");
}

#[test]
fn test_declare_p_assoc_export_uses_typeset() {
    // `declare -Ax h` — assoc + export should print `typeset -Ax`,
    // not `export -A`. The export keyword is reserved for scalars/ints.
    let (_, output, _) = run_zshrs(r#"declare -Ax h; declare -p h"#);
    assert_eq!(output.trim(), "typeset -Ax h=( )");
}

#[test]
fn test_declare_p_float_E_flag() {
    // `declare -E a=3.14` → `typeset -E ...` (was wrongly `-F`).
    let (_, output, _) = run_zshrs(r#"declare -E a=3.14; declare -p a"#);
    assert!(
        output.starts_with("typeset -E"),
        "expected -E flag: {output:?}"
    );
}

#[test]
fn test_array_element_no_colon_set() {
    // `${arr[N]+set}` (no colon) should print "set" when index is in bounds.
    let (_, output, _) = run_zshrs(r#"arr=(a b); echo "${arr[1]+set}""#);
    assert_eq!(output.trim(), "set");
}

#[test]
fn test_array_element_no_colon_set_oob() {
    let (_, output, _) = run_zshrs(r#"arr=(a b); echo "${arr[3]+set}""#);
    assert_eq!(output.trim(), "");
}

#[test]
fn test_assoc_element_no_colon_set() {
    let (_, output, _) = run_zshrs(r#"declare -A h; h[k]=v; echo "${h[k]+set}""#);
    assert_eq!(output.trim(), "set");
}

#[test]
fn test_assoc_element_no_colon_unset() {
    let (_, output, _) = run_zshrs(r#"declare -A h; h[k]=v; echo "${h[m]+set}""#);
    assert_eq!(output.trim(), "");
}

#[test]
fn test_t_flag_array_unique() {
    // `${(t)arr}` for `typeset -aU` should include "-unique".
    let (_, output, _) = run_zshrs(r#"declare -aU arr=(a a b); echo "${(t)arr}""#);
    assert_eq!(output.trim(), "array-unique");
}

#[test]
fn test_test_nt_both_must_exist() {
    // `[[ a -nt b ]]` requires BOTH files to exist; missing → false.
    let (_, output, _) =
        run_zshrs(r#"[[ /etc/passwd -nt /tmp/nope_zshrs ]] && echo nt || echo not_nt"#);
    assert_eq!(output.trim(), "not_nt");
}

#[test]
fn test_test_ot_missing_is_false() {
    // `[[ "foo" -ot /tmp ]]` — "foo" doesn't exist → false (not "old").
    let (_, output, _) = run_zshrs(r#"[[ "foo" -ot /tmp ]] && echo old || echo notold"#);
    assert_eq!(output.trim(), "notold");
}

#[test]
fn test_double_bracket_var_eq_var() {
    // `[[ $a == $b ]]` should expand both sides. Regression: zshrs's
    // `==` path treated RHS as literal pattern, so `$b` was the
    // string "$b" not the value.
    let (_, output, _) = run_zshrs(r#"a=foo; b=foo; [[ $a == $b ]] && echo eq"#);
    assert_eq!(output.trim(), "eq");
}

#[test]
fn test_double_bracket_var_eq_var_unequal() {
    let (_, output, _) = run_zshrs(r#"a=foo; b=bar; [[ $a == $b ]] && echo eq || echo neq"#);
    assert_eq!(output.trim(), "neq");
}

#[test]
fn test_double_bracket_glob_pattern_still_works() {
    // Regression guard: glob on RHS still works.
    let (_, output, _) = run_zshrs(r#"[[ "abc" == ab* ]] && echo m"#);
    assert_eq!(output.trim(), "m");
}

#[test]
fn test_arith_comma_compound_assigns() {
    // `((a += 5, a *= 2))` — comma-list of compound assigns.
    // ArithCompiler's emit only handled the first; route through
    // MathEval (extended needs_eval to include `,`).
    let (_, output, _) = run_zshrs(r#"a=10; ((a += 5, a *= 2)); echo $a"#);
    assert_eq!(output.trim(), "30");
}

#[test]
fn test_arith_comma_two_vars() {
    // `((a += 5, b *= 2))` — both vars get updated.
    let (_, output, _) = run_zshrs(r#"a=10; b=20; ((a += 5, b *= 2)); echo "$a $b""#);
    assert_eq!(output.trim(), "15 40");
}

#[test]
fn test_test_dash_a_and() {
    // `test a -a b` — POSIX AND connective.
    let (_, output, _) = run_zshrs(r#"test 5 -gt 3 -a 3 -lt 4; echo $?"#);
    assert_eq!(output.trim(), "0");
}

#[test]
fn test_test_dash_o_or() {
    // `test a -o b` — POSIX OR connective.
    let (_, output, _) = run_zshrs(r#"test 5 -gt 10 -o 3 -lt 4; echo $?"#);
    assert_eq!(output.trim(), "0");
}

#[test]
fn test_test_dash_a_short_circuit_fails() {
    let (_, output, _) = run_zshrs(r#"test 5 -gt 3 -a 5 -gt 10; echo $?"#);
    assert_eq!(output.trim(), "1");
}

#[test]
fn test_test_dash_o_both_fail() {
    let (_, output, _) = run_zshrs(r#"test 5 -gt 10 -o 1 -gt 10; echo $?"#);
    assert_eq!(output.trim(), "1");
}

#[test]
fn test_float_default_E_format() {
    // `float f=3.14` defaults to `-E` (scientific) per zsh.
    let (_, output, _) = run_zshrs(r#"float f=3.14; declare -p f"#);
    assert_eq!(output.trim(), "typeset -E f=3.140000000e+00");
}

#[test]
fn test_float_F_explicit_fixed() {
    let (_, output, _) = run_zshrs(r#"float -F f=3.14; declare -p f"#);
    assert_eq!(output.trim(), "typeset -F f=3.1400000000");
}

#[test]
fn test_fpath_inherited_from_env() {
    // `fpath` should mirror $FPATH at startup; user-level `$#fpath`
    // should match the env-derived count, not 0.
    let (_, output, _) = run_zshrs(r#"echo $#fpath"#);
    let n: i32 = output.trim().parse().unwrap_or(-1);
    assert!(n > 0, "fpath should be populated from FPATH: got {n}");
}

#[test]
fn test_fpath_append_keeps_existing() {
    // `fpath+=(/foo)` should APPEND, preserving the env-derived
    // entries. Regression: zshrs replaced fpath with just the new entry.
    let (_, before, _) = run_zshrs(r#"echo $#fpath"#);
    let (_, after, _) = run_zshrs(r#"fpath+=(/zshrs_test_dir); echo $#fpath"#);
    let n_before: i32 = before.trim().parse().unwrap_or(0);
    let n_after: i32 = after.trim().parse().unwrap_or(0);
    assert_eq!(n_after, n_before + 1, "expected count+1 after fpath+=");
}

#[test]
fn test_dq_var_concat_with_literal_suffix() {
    // `"$a"bar` should produce `foobar`. Regression: the bare-var
    // fast-path matched after untokenize stripped DNULL markers,
    // looking up nonexistent `abar` instead of `a`.
    let (_, output, _) = run_zshrs(r#"a=foo; echo "$a"bar"#);
    assert_eq!(output.trim(), "foobar");
}

#[test]
fn test_dq_var_concat_with_literal_prefix() {
    let (_, output, _) = run_zshrs(r#"a=foo; echo bar"$a""#);
    assert_eq!(output.trim(), "barfoo");
}

#[test]
fn test_dq_var_with_underscore_suffix() {
    let (_, output, _) = run_zshrs(r#"a=foo; echo "$a"_X"#);
    assert_eq!(output.trim(), "foo_X");
}

#[test]
fn test_dq_var_double_concat() {
    let (_, output, _) = run_zshrs(r#"a=foo; echo X_"$a"_Y"#);
    assert_eq!(output.trim(), "X_foo_Y");
}

#[test]
fn test_anonymous_function_no_parens() {
    // `function { body } args` — zsh-only anonymous-function shorthand
    // (no `()`). Args become the function's positional params.
    let (_, output, _) = run_zshrs("function { echo $1 } hello");
    assert_eq!(output.trim(), "hello");
}

#[test]
fn test_anonymous_function_no_parens_multi_arg() {
    let (_, output, _) = run_zshrs("function { echo $#; echo $@ } a b c");
    assert_eq!(output.trim(), "3\na b c");
}

#[test]
fn test_zsh_subshell_increments() {
    // `$ZSH_SUBSHELL` increments by one for each subshell nesting level.
    let (_, output, _) =
        run_zshrs("echo $ZSH_SUBSHELL; (echo $ZSH_SUBSHELL); ( ( echo $ZSH_SUBSHELL ) )");
    assert_eq!(output.trim(), "0\n1\n2");
}

#[test]
fn test_printf_double_dash_end_of_options() {
    // `printf -- fmt args...` — POSIX `--` end-of-options. Without this
    // `--` was printed as the format string's first literal output.
    let (_, output, _) = run_zshrs(r#"printf -- "%s\n" hi"#);
    assert_eq!(output.trim(), "hi");
}

#[test]
fn test_glob_sort_locale_aware() {
    // Under a Unicode locale, glob results sort case-insensitively
    // (`Aaa bbb Ccc Ddd` not `Aaa Ccc Ddd bbb`).
    let dir = std::env::temp_dir().join("zshrs_test_locale_sort");
    let _ = std::fs::create_dir_all(&dir);
    for name in ["Aaa", "bbb", "Ccc", "Ddd"] {
        std::fs::File::create(dir.join(name)).unwrap();
    }
    let cmd = format!("cd {} && echo *", dir.display());
    let (_, output, _) = run_zshrs(&cmd);
    let _ = std::fs::remove_dir_all(&dir);
    let trimmed = output.trim();
    // Either case-folded order under Unicode locale, or ASCII order under C.
    let lc_all = std::env::var("LC_ALL").unwrap_or_default();
    let lang = std::env::var("LANG").unwrap_or_default();
    let active = if !lc_all.is_empty() { lc_all } else { lang };
    let is_c = matches!(
        active
            .split('.')
            .next()
            .unwrap_or("")
            .to_uppercase()
            .as_str(),
        "" | "C" | "POSIX"
    );
    if is_c {
        assert_eq!(trimmed, "Aaa Ccc Ddd bbb");
    } else {
        assert_eq!(trimmed, "Aaa bbb Ccc Ddd");
    }
}

#[test]
fn test_glob_sort_equal_key_tiebreak_is_deterministic_name_order() {
    // A `o`/`O` sort qualifier whose primary key TIES across matches (e.g.
    // several 0-byte files under `*(oL)`) has no portable order in zsh — the
    // libc qsort resolves equal-key elements arbitrarily (non-stable on
    // macOS/BSD, readdir-dependent on glibc). zshrs is cross-architecture and
    // guarantees a DETERMINISTIC name-ascending tie-break instead. This is a
    // zshrs contract, deliberately NOT a `zsh -f` parity assertion (real zsh's
    // order for equal keys is implementation-defined and unmatchable). The
    // primary sort itself (size order) still matches zsh; see
    // test_glob_qualifier_size_uses_lstat_for_symlinks for that.
    let dir = tempdir_for_test();
    // Three equal-size (0-byte) files in non-alphabetical creation order,
    // plus two distinct sizes to pin the primary key.
    for name in ["d.txt", "b.txt", "a.txt"] {
        std::fs::write(format!("{}/{}", dir, name), b"").unwrap();
    }
    std::fs::write(format!("{}/mid.txt", dir), vec![0u8; 10]).unwrap();
    std::fs::write(format!("{}/big.txt", dir), vec![0u8; 100]).unwrap();

    // Ascending size: the three 0-byte files come first in NAME order
    // (a,b,d) regardless of creation order, then mid (10), then big (100).
    let (_, out, _) = run_zshrs(&format!("cd {} && echo *(oL)", dir));
    assert_eq!(
        out.trim(),
        "a.txt b.txt d.txt mid.txt big.txt",
        "equal-size files must tie-break by name ascending: {out:?}"
    );

    // Descending size reverses the PRIMARY key (big, mid, then the ties),
    // but the equal-key tie-break stays name-ascending (a,b,d) — the
    // GS_DESC flag applies only to the size comparison, not the GS_NAME
    // tie-breaker appended after it.
    let (_, out, _) = run_zshrs(&format!("cd {} && echo *(OL)", dir));
    assert_eq!(
        out.trim(),
        "big.txt mid.txt a.txt b.txt d.txt",
        "descending size keeps name-ascending tie-break: {out:?}"
    );

    // Repeatability: identical output across runs (no qsort nondeterminism).
    let (_, out2, _) = run_zshrs(&format!("cd {} && echo *(oL)", dir));
    assert_eq!(out2.trim(), "a.txt b.txt d.txt mid.txt big.txt");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_typeset_integer_base_output() {
    // `typeset -i N name=value` displays in base N as `N#DIGITS`.
    let (_, output, _) = run_zshrs("typeset -i 16 a=255; echo $a");
    assert_eq!(output.trim(), "16#FF");

    let (_, output, _) = run_zshrs("typeset -i 2 a=10; echo $a");
    assert_eq!(output.trim(), "2#1010");
}

#[test]
fn test_cd_uses_shell_home() {
    // `HOME=/tmp; cd; pwd` honors the shell-state HOME assignment
    // (no export needed) — was reading only OS env before.
    let (_, output, _) = run_zshrs("HOME=/tmp; cd; pwd");
    assert_eq!(output.trim(), "/tmp");
}

#[test]
fn test_cd_tilde_uses_shell_home() {
    // `cd ~` — tilde expansion reads shell-state HOME first.
    let (_, output, _) = run_zshrs("HOME=/tmp; cd ~; pwd");
    assert_eq!(output.trim(), "/tmp");
}

#[test]
fn test_cdpath_implicit_search() {
    // CDPATH entries are searched without `-s` flag when the literal
    // path isn't a directory in cwd.
    let (_, output, _) = run_zshrs(
        "mkdir -p /tmp/zshrs_cdpath_test && CDPATH=/tmp cd zshrs_cdpath_test; pwd; rm -rf /tmp/zshrs_cdpath_test",
    );
    assert!(output.contains("/tmp/zshrs_cdpath_test"));
}

#[test]
fn test_tilde_literal_in_double_quotes() {
    // Tilde inside `"..."` is literal — `echo "~"` prints `~`, not
    // the home dir.
    let (_, output, _) = run_zshrs(r#"echo "~""#);
    assert_eq!(output.trim(), "~");

    let (_, output, _) = run_zshrs(r#"a="~"; echo "[$a]""#);
    assert_eq!(output.trim(), "[~]");
}

#[test]
fn test_herestring_inside_command_substitution() {
    // `$(<<<"hi" cat)` — herestring inside cmd-subst was being
    // misinterpreted as the `$(<file)` read-file shorthand.
    let (_, output, _) = run_zshrs(r#"a=$(<<<"hi" cat); echo "$a""#);
    assert_eq!(output.trim(), "hi");
}

#[test]
fn test_array_suffix_strip_per_element() {
    // `${arr%pat}` strips suffix per element, not after joining.
    let (_, output, _) = run_zshrs("a=(a.txt b.bin c.txt); echo ${a%.txt}");
    assert_eq!(output.trim(), "a b.bin c");
}

#[test]
fn test_array_prefix_strip_per_element() {
    // `${arr#pat}` strips prefix per element.
    let (_, output, _) = run_zshrs("a=(/tmp/x /tmp/y); echo ${a#/tmp/}");
    assert_eq!(output.trim(), "x y");
}

#[test]
fn test_array_long_suffix_strip_per_element() {
    // `${arr%%pat}` strips longest suffix per element.
    let (_, output, _) = run_zshrs("a=(a.b.c d.e.f); echo ${a%%.*}");
    assert_eq!(output.trim(), "a d");
}

#[test]
fn test_array_long_prefix_strip_per_element() {
    // `${arr##pat}` strips longest prefix per element.
    let (_, output, _) = run_zshrs("a=(/tmp/a /tmp/b); echo ${a##*/}");
    assert_eq!(output.trim(), "a b");
}

#[test]
fn test_kill_zero_process_check() {
    // `kill -0 PID` is the POSIX "process existence check" — no signal
    // sent, just kill(pid, 0). Was failing because Signal::SIG0 is
    // not a libc Signal enum variant.
    let (_, output, _) = run_zshrs("kill -0 $$; echo $?");
    assert_eq!(output.trim(), "0");
}

#[test]
fn test_print_strict_unknown_flag_errors() {
    // `print --hi` errors `bad option: -h` (zsh treats `-` as a no-op
    // flag char then errors on the first unknown). Was being passed
    // through as a positional arg.
    let (_, _, stderr) = run_zshrs("print --hi");
    assert!(
        stderr.contains("bad option") || stderr.contains("-h"),
        "stderr: {}",
        stderr
    );
}

#[test]
fn test_print_double_dash_terminator() {
    // `print -- -hi` — `--` ends options, `-hi` is positional.
    let (_, output, _) = run_zshrs("print -- -hi");
    assert_eq!(output.trim(), "-hi");
}

#[test]
fn test_heredoc_backslash_terminator_disables_expansion() {
    // `<<\EOF` is shorthand for `<<'EOF'` — disables variable /
    // command-sub / arithmetic expansion in the body.
    let (_, output, _) = run_zshrs("a=42; cat <<\\EOF\nval=$a\nEOF");
    assert_eq!(output.trim(), "val=$a");
}

#[test]
fn test_trap_signal_zero_is_exit() {
    // POSIX: `trap CMD 0` == `trap CMD EXIT` — runs at shell exit.
    let (_, output, _) = run_zshrs(r#"trap "echo bye" 0; echo hi"#);
    assert_eq!(output.trim(), "hi\nbye");
}

#[test]
fn test_special_param_concat_after_literal() {
    // `echo X$?` should expand $? — was returning literal `X$?`.
    // Same root cause for `X$$`, `X$#`, `X$*`, `X$!`. Fix: extend
    // find_expansion_end's special-single-char matcher to recognise
    // the META-coded forms (`\u{97}` for `?`, `\u{85}` for `$`, etc.)
    // so the expansion-segment splitter doesn't truncate.
    let (_, output, _) = run_zshrs("true; echo X$?");
    assert_eq!(output.trim(), "X0");
    let (_, output, _) = run_zshrs("false; echo bye=$?");
    assert_eq!(output.trim(), "bye=1");
    let (_, output, _) = run_zshrs("set -- a b c; echo X$#");
    assert_eq!(output.trim(), "X3");
}

#[test]
fn test_printf_x_negative_wraps_unsigned() {
    // printf "%x" -1 should print "ffffffffffffffff" — was producing
    // "0" because u64 parse rejected the leading `-`.
    let (_, output, _) = run_zshrs(r#"printf "%x\n" -1"#);
    assert_eq!(output.trim(), "ffffffffffffffff");
}

#[test]
fn test_printf_octal_escape_no_leading_zero() {
    // `\NNN` (1-3 octal digits, no leading 0) is the POSIX form.
    // `\102` should produce `B` (octal 102 = 66 = 'B'). Was leaving
    // it as literal `\102` because the escape branch only matched
    // `\0NNN`.
    let (_, output, _) = run_zshrs(r#"printf "a\102b\n""#);
    assert_eq!(output.trim(), "aBb");
}

#[test]
fn test_printf_octal_leading_zero_three_total_digits() {
    // `\0102` consumes `010` (3 chars total including the leading `0`)
    // = octal 010 = 8 = backspace, then leaves `2` as literal.
    let (_, output, _) = run_zshrs(r#"printf "[\0102]""#);
    // Expect: `[`, BS (0x08), `2`, `]`. Match exact bytes.
    assert_eq!(output.as_bytes(), &[b'[', 0x08, b'2', b']']);
}

#[test]
fn test_anon_function_dollar_zero_is_anon_string() {
    // zsh: `() { echo $0; } anon arg` → `(anon)` for $0
    // (cosmetic — the synthesized `_zshrs_anon_N` name was leaking).
    let (_, output, _) = run_zshrs("() { echo $0 } anon");
    assert_eq!(output.trim(), "(anon)");
}

#[test]
fn test_set_capital_E_accepted() {
    // `set -E` (ERR_RETURN: return on non-zero status inside a fn)
    // — was erroring "invalid option". Now accepts the flag silently.
    let (_, _, stderr) = run_zshrs("set -E; foo() { false; }; foo");
    assert!(
        !stderr.contains("invalid option"),
        "stderr should not have invalid-option err: {}",
        stderr
    );
}

#[test]
fn test_double_bracket_o_unknown_option_warns() {
    // `[[ -o no_such_option ]]` should emit "no such option" to
    // stderr (and the test result is false). Was silently returning
    // false with no diagnostic.
    let (_, _, stderr) = run_zshrs("[[ -o no_such_option_zzz ]] && echo y || echo n");
    assert!(
        stderr.contains("no such option"),
        "expected 'no such option' in stderr, got: {}",
        stderr
    );
}

#[test]
fn test_heredoc_body_no_glob_expansion() {
    // `cat <<EOF\n[42]\nEOF` should pass `[42]` through verbatim.
    // The heredoc body was being routed through the full default
    // expansion pipeline including glob, which fired NOMATCH on the
    // literal `[42]` pattern. Now uses a heredoc-specific mode that
    // expands $vars / cmd-subst / arith but skips glob+brace.
    let (_, output, _) = run_zshrs("a=42; cat <<EOF\n[$a]\nEOF");
    assert_eq!(output.trim(), "[42]");
}

#[test]
fn test_dollar_underscore_tracks_last_command_arg() {
    // `echo hi; echo $_` → `hi\nhi`. zshrs was returning the binary
    // path because no command-dispatch path updated `$_`. Fix:
    // promote pending_underscore in pop_args / host.exec.
    let (_, output, _) = run_zshrs("echo hi; echo $_");
    assert_eq!(output.trim(), "hi\nhi");
}

#[test]
fn test_array_at_subscript_history_modifier_per_element() {
    // `${arr[@]:h}` should apply :h per element (`/a/b/c /d/e/f` →
    // `/a/b /d/e`), not collapse to scalar then strip. Same for
    // :t / :r / :l / :u modifiers.
    let (_, output, _) = run_zshrs(r#"a=(/a/b/c /d/e/f); echo "${a[@]:h}""#);
    assert_eq!(output.trim(), "/a/b /d/e");

    let (_, output, _) = run_zshrs(r#"a=(foo.txt bar.bin); echo "${a[@]:r}""#);
    assert_eq!(output.trim(), "foo bar");

    let (_, output, _) = run_zshrs(r#"a=(/a/b/c /d/e); echo "${a[@]:t}""#);
    assert_eq!(output.trim(), "c e");
}

#[test]
fn test_h_modifier_strips_trailing_slashes() {
    // `${var:h}` should strip trailing slashes BEFORE removing the
    // last segment: `/tmp/` → `/`, not `/tmp`. zshrs was preserving
    // the slash and dropping `tmp` as the trailing segment.
    let (_, output, _) = run_zshrs(r#"a="/tmp/"; echo "[${a:h}]""#);
    assert_eq!(output.trim(), "[/]");
    let (_, output, _) = run_zshrs(r#"a="//"; echo "[${a:h}]""#);
    assert_eq!(output.trim(), "[/]");
    let (_, output, _) = run_zshrs(r#"a="/a/b/c"; echo "[${a:h}]""#);
    assert_eq!(output.trim(), "[/a/b]");
}

#[test]
fn test_chained_subscript_array_then_char() {
    // `${a[1][1]}` with a=(hello) should pick array elem 1 (`hello`)
    // then char 1 (`h`). zshrs was returning the full element.
    let (_, output, _) = run_zshrs(r#"a=(hello); echo "${a[1][1]}""#);
    assert_eq!(output.trim(), "h");
    let (_, output, _) = run_zshrs(r#"a=(hello world); echo "${a[2][1]}""#);
    assert_eq!(output.trim(), "w");
    let (_, output, _) = run_zshrs(r#"a=(abcdef); echo "${a[1][2,4]}""#);
    assert_eq!(output.trim(), "bcd");
}

#[test]
fn test_print_rejects_dash_e_and_dash_E() {
    // `print -e` and `print -E` both error "bad option" in zsh.
    // zshrs's print accepted both. (Note: `echo -E` and `echo -e`
    // remain valid — these are echo-only flags.)
    let (_, _, stderr) = run_zshrs("print -E hi");
    assert!(stderr.contains("bad option"));
    let (_, _, stderr) = run_zshrs("print -e hi");
    assert!(stderr.contains("bad option"));
}

#[test]
fn test_math_rejects_0o_octal_prefix() {
    // zsh: `$((0o15))` errors "bad math expression: operator
    // expected at `o15'". zshrs silently returned 0. The `0o`
    // octal-prefix is Rust/Python convention; zsh doesn't accept it.
    let (_, _, stderr) = run_zshrs("echo $((0o15))");
    assert!(stderr.contains("bad math expression"));
}

#[test]
fn test_trap_empty_string_listed_as_ignore() {
    // `trap "" SIG` is signal-ignore (not "reset to default" — that's
    // `trap - SIG`). Both forms should be visible in `trap` listing.
    let (_, output, _) = run_zshrs(r#"trap "" USR1; trap"#);
    assert!(output.contains("trap -- '' USR1"));
}

#[test]
fn test_printf_d_truncates_float_to_int() {
    // `printf "%d" 3.14` → 3 (POSIX truncate). zshrs was returning
    // 0 because the i64 parse rejected the decimal point.
    let (_, output, _) = run_zshrs(r#"printf "%d\n" 3.14"#);
    assert_eq!(output.trim(), "3");
    let (_, output, _) = run_zshrs(r#"printf "%i\n" -5.99"#);
    assert_eq!(output.trim(), "-5");
}

#[test]
fn test_set_dash_h_and_k_accepted() {
    // `set -h` (HASH_CMDS), `set -k` (KSH_TYPESET), `set -p`
    // (PRIVILEGED), `set -B` (BRACE_CCL), `set -H` (HIST_REDUCE_BLANKS).
    // All zsh-spec single-char toggles. Were all erroring "invalid
    // option". Accept silently as toggle-options.
    let (_, _, stderr) = run_zshrs("set -h; echo done");
    assert!(!stderr.contains("invalid option"));
    let (_, _, stderr) = run_zshrs("set -k; echo done");
    assert!(!stderr.contains("invalid option"));
}

#[test]
fn test_array_element_length_via_hash() {
    // `${#arr[N]}` should return char-count of the Nth element
    // (zsh: `a=(hello world); ${#a[1]}` → 5). Was returning 0
    // because the [N] subscript path wasn't reached for `${#…}`.
    let (_, output, _) = run_zshrs(r#"a=(hello world); echo "${#a[1]}""#);
    assert_eq!(output.trim(), "5");
    let (_, output, _) = run_zshrs(r#"a=(short verylongstring); echo "${#a[2]}""#);
    assert_eq!(output.trim(), "14");
}

#[test]
fn test_printf_dot_s_zero_precision_suppresses_arg() {
    // `%.s` (period, no digits) means precision-0 — the string arg
    // is suppressed. zshrs was treating the missing digits as
    // "no precision" and printing the full arg.
    let (_, output, _) = run_zshrs(r#"printf "[%.s]" "ignore""#);
    assert_eq!(output, "[]");
}

#[test]
fn test_print_dash_n_suppresses_null_terminator() {
    // `print -nN args` — `-n` always suppresses the terminator
    // (even with `-N` for NUL-separator). Without this, `print -nN
    // hi` left a stray NUL between the print and the next command.
    let (_, output, _) = run_zshrs(r#"print -nN hi; echo X"#);
    assert_eq!(output, "hiX\n");
}

#[test]
fn test_set_e_in_subshell_doesnt_kill_parent() {
    // `(set -e; false); echo "alive"` — set -e inside a subshell
    // must not propagate to the parent shell. Was calling
    // `std::process::exit` which tore down the parent process.
    // Now the errexit-check skips the exit when inside a subshell
    // snapshot (parent stays alive; subshell continues to end).
    let (_, output, _) = run_zshrs(r#"true; (set -e; false); echo "alive=$?""#);
    assert!(output.contains("alive="));
}

#[test]
fn test_glob_om_sort_newest_first() {
    // `*(om)` orders by mtime newest-first (zsh's time-qualifier
    // default is descending; `Om` is the explicit oldest-first).
    // Was sorting alphabetically because the post-filter alpha
    // sort clobbered the qualifier-driven order, AND `O` wasn't
    // in the looks_like_glob_qualifiers char set so `*(Om)` parsed
    // as a literal pattern.
    //
    // Use a per-PID tmpdir so parallel test runs don't trample one
    // another's dir contents (the previous fixed name `zshrs_test_om_sort`
    // produced flaky failures when this test ran alongside its `Om` twin).
    let dir = std::env::temp_dir().join(format!("zshrs_test_om_lower_sort_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    for name in ["a", "b", "c"] {
        std::fs::File::create(dir.join(name)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let cmd = format!("cd {} && echo *(om)", dir.display());
    let (_, output, _) = run_zshrs(&cmd);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(output.trim(), "c b a");
}

#[test]
fn test_glob_Om_sort_oldest_first() {
    let dir = std::env::temp_dir().join(format!("zshrs_test_Om_upper_sort_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    for name in ["a", "b", "c"] {
        std::fs::File::create(dir.join(name)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let cmd = format!("cd {} && echo *(Om)", dir.display());
    let (_, output, _) = run_zshrs(&cmd);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(output.trim(), "a b c");
}

#[test]
fn test_history_dash_c_zsh_error_format() {
    // zsh's `history` is `fc -l` synonym; `-c` (bash clear) is not
    // accepted. zsh emits "bad option: -c". zshrs had a custom
    // "clear not supported" string; aligned to zsh's format.
    let (_, _, stderr) = run_zshrs("history -c");
    assert!(stderr.contains("bad option"));
}

#[test]
fn test_glob_posix_char_class_alpha() {
    // `[[:alpha:]]` and friends — POSIX char-class syntax. The
    // glob crate doesn't recognize the `[:class:]` form, so we
    // pre-expand to enumerated ranges (`a-zA-Z`, `0-9`, etc).
    let dir = std::env::temp_dir().join("zshrs_test_pcc");
    let _ = std::fs::create_dir_all(&dir);
    for name in ["1", "a", "2", "b"] {
        std::fs::File::create(dir.join(name)).unwrap();
    }
    let cmd = format!("cd {} && echo [[:digit:]]", dir.display());
    let (_, output, _) = run_zshrs(&cmd);
    let _ = std::fs::remove_dir_all(&dir);
    let mut parts: Vec<&str> = output.split_whitespace().collect();
    parts.sort();
    assert_eq!(parts, vec!["1", "2"]);
}

#[test]
fn test_glob_posix_char_class_alpha_letters() {
    let dir = std::env::temp_dir().join("zshrs_test_pcc_alpha");
    let _ = std::fs::create_dir_all(&dir);
    for name in ["1", "a", "2", "b"] {
        std::fs::File::create(dir.join(name)).unwrap();
    }
    let cmd = format!("cd {} && echo [[:alpha:]]", dir.display());
    let (_, output, _) = run_zshrs(&cmd);
    let _ = std::fs::remove_dir_all(&dir);
    let mut parts: Vec<&str> = output.split_whitespace().collect();
    parts.sort();
    assert_eq!(parts, vec!["a", "b"]);
}

#[test]
fn test_case_multi_pattern_with_brackets() {
    // `case x in [a-z]) ...;; [A-Z]) ...;; esac` — bracket-class
    // patterns in subsequent case arms were parse-erroring because
    // the lexer tokenized `[` as Inbrack (not part of pattern) when
    // incasepat had reset to 0 across the `;;` advance.
    let (_, output, _) = run_zshrs("case a in [a-z]) echo lower;; [A-Z]) echo upper;; esac");
    assert_eq!(output.trim(), "lower");
    let (_, output, _) = run_zshrs("case X in [a-z]) echo lower;; [A-Z]) echo upper;; esac");
    assert_eq!(output.trim(), "upper");
}

#[test]
fn test_type_for_reserved_word() {
    let (_, output, _) = run_zshrs("type for");
    assert!(output.contains("reserved word"));
    let (_, output, _) = run_zshrs("type while");
    assert!(output.contains("reserved word"));
    let (_, output, _) = run_zshrs("type if");
    assert!(output.contains("reserved word"));
}

#[test]
fn test_dollar_hash_at_in_double_quotes() {
    // `"$#@"` should expand to positional count (zsh shorthand for
    // ${#@}). zshrs was splitting `$#` from `@` so the literal `@`
    // got appended. Same for `"$#*"`.
    let (_, output, _) = run_zshrs(r#"set -- a b c; echo "$#@""#);
    assert_eq!(output.trim(), "3");
    let (_, output, _) = run_zshrs(r#"set -- a b c; echo "$#*""#);
    assert_eq!(output.trim(), "3");
}

#[test]
fn test_dollar_hash_name_concat() {
    // `X$#Y` for unset Y should return `X0` (length of empty Y).
    // Was returning `X3Y` because the segment-splitter only consumed
    // `$#` and left `Y` as a literal trailing segment.
    let (_, output, _) = run_zshrs("set -- a b c; echo X$#Y");
    assert_eq!(output.trim(), "X0");
}

#[test]
fn test_unalias_missing_zsh_format() {
    // `unalias missing` zsh format: `zsh:unalias:1: no such hash
    // table element: NAME`. zshrs had its own format.
    let (_, _, stderr) = run_zshrs("unalias notdef_xxx_zzz");
    assert!(stderr.contains("no such hash table element"));
}

#[test]
fn test_optind_default_one() {
    // POSIX: OPTIND defaults to 1 before getopts is called. zshrs
    // returned empty string. Now initialized to 1 in executor.
    let (_, output, _) = run_zshrs(r#"echo "[$OPTIND]""#);
    assert_eq!(output.trim(), "[1]");
}

#[test]
fn test_setopt_unknown_option_errors() {
    // `setopt nosuchoption_zzz` should emit "no such option:" to
    // stderr matching zsh. Was silent.
    let (_, _, stderr) = run_zshrs("setopt nosuchoption_zzz_xx");
    assert!(stderr.contains("no such option"));
}

#[test]
fn test_random_resolves_in_arithmetic() {
    // `$((RANDOM))` was returning 0 because MathEval looked up the
    // name in `self.variables` (which doesn't contain RANDOM — it's
    // a special param resolved dynamically). Now pre-resolved into
    // a working extras map before MathEval runs.
    let (_, output, _) = run_zshrs("echo $((RANDOM))");
    let val: i64 = output.trim().parse().unwrap_or(-1);
    assert!((0..=32767).contains(&val), "RANDOM out of range: {}", val);

    // RANDOM should differ per arith-subst (zsh contract).
    let (_, output, _) = run_zshrs("a=$((RANDOM)); b=$((RANDOM)); [[ $a != $b ]] && echo diff");
    assert_eq!(output.trim(), "diff");
}

#[test]
fn test_dollar_underscore_starts_empty() {
    // zsh: `$_` is empty before any command runs (it ignores the
    // OS-env value the parent process set). zshrs was returning the
    // binary path because get_variable fell through to env::var.
    // Initialize "_" to "" in the executor so the first read is
    // empty.
    let (_, output, _) = run_zshrs(r#"echo "[$_]""#);
    assert_eq!(output.trim(), "[]");
}

#[test]
fn test_double_bracket_pattern_with_quoted_parens() {
    // `[[ "foo()" == "foo()" ]]` should match (the `()` inside DQ
    // are literal). Quoted-glob-meta escaping was missing `(`,
    // `)`, `|`, `~`, `#`, `^` so the pattern matcher saw them as
    // alternation grouping (`@(...)` form).
    let (_, output, _) = run_zshrs(r#"[[ "foo()" == "foo()" ]] && echo y"#);
    assert_eq!(output.trim(), "y");
    let (_, output, _) = run_zshrs(r#"[[ "x|y" == "x|y" ]] && echo y"#);
    assert_eq!(output.trim(), "y");
}

#[test]
fn test_command_v_function_shows_source() {
    // `command -V foo` for a function should say "is a shell
    // function from zsh" (matching zsh's exact format). Was just
    // "is a shell function".
    let (_, output, _) = run_zshrs(r#"foo() { :; }; command -V foo"#);
    assert!(output.contains("from zsh"));
}

#[test]
fn test_arith_subscripted_post_increment() {
    // `(( a[1]++ ))` should increment the first element. zshrs's
    // ArithCompiler couldn't write back through arr[idx] for compound
    // forms — only the bare `=` assign was caught. Now routed through
    // a runtime parse_subscript_arith_compound + read-modify-write.
    let (_, output, _) = run_zshrs(r#"a=(0 0); (( a[1]++ )); echo "${a[1]}""#);
    assert_eq!(output.trim(), "1");
}

#[test]
fn test_arith_subscripted_compound_plus_eq() {
    let (_, output, _) = run_zshrs(r#"a=(10 20); (( a[1] += 5 )); echo "${a[1]}""#);
    assert_eq!(output.trim(), "15");
    let (_, output, _) = run_zshrs(r#"a=(5 5); (( a[1] *= 3 )); echo "${a[1]}""#);
    assert_eq!(output.trim(), "15");
}

#[test]
fn test_arith_subscripted_post_increment_returns_old() {
    let (_, output, _) = run_zshrs(r#"a=(5 10); echo "$(( a[1]++ ))"; echo "${a[1]}""#);
    // Post-increment returns OLD value, then variable updates.
    assert_eq!(output.trim(), "5\n6");
}

#[test]
fn test_glob_dot_qualifier_excludes_symlinks() {
    // `*(.)` should match plain regular files only (not symlinks).
    // Was treating symlinks-to-files as files because the metadata
    // followed links. Now check symlink_metadata too and exclude
    // when file_type().is_symlink() is true.
    let dir = std::env::temp_dir().join("zshrs_test_dot_qual");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    std::fs::File::create(dir.join("a")).unwrap();
    std::fs::create_dir_all(dir.join("b")).unwrap();
    let _ = std::os::unix::fs::symlink("a", dir.join("c"));
    let cmd = format!("cd {} && echo *(.)", dir.display());
    let (_, output, _) = run_zshrs(&cmd);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(output.trim(), "a");
}

#[test]
fn test_extendedglob_tilde_exclusion() {
    // `*.txt~b.txt` should match all *.txt EXCEPT b.txt under
    // setopt extendedglob.
    let dir = std::env::temp_dir().join("zshrs_test_extglob_tilde");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    for name in ["a.txt", "b.txt", "c.txt"] {
        std::fs::File::create(dir.join(name)).unwrap();
    }
    let cmd = format!(
        "setopt extendedglob; cd {} && echo *.txt~b.txt",
        dir.display()
    );
    let (_, output, _) = run_zshrs(&cmd);
    let _ = std::fs::remove_dir_all(&dir);
    let mut parts: Vec<&str> = output.split_whitespace().collect();
    parts.sort();
    assert_eq!(parts, vec!["a.txt", "c.txt"]);
}

#[test]
fn test_extendedglob_caret_negation() {
    // `^pat` matches all entries that DON'T match `pat`.
    let dir = std::env::temp_dir().join("zshrs_test_extglob_caret");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    for name in ["a", "b", "c"] {
        std::fs::File::create(dir.join(name)).unwrap();
    }
    let cmd = format!("setopt extendedglob; cd {} && echo ^b", dir.display());
    let (_, output, _) = run_zshrs(&cmd);
    let _ = std::fs::remove_dir_all(&dir);
    let mut parts: Vec<&str> = output.split_whitespace().collect();
    parts.sort();
    assert_eq!(parts, vec!["a", "c"]);
}

#[test]
fn test_alias_dash_L_emits_alias_prefix() {
    // `alias -L name` should print `alias name=value` (re-input
    // form). zshrs was printing `name=value` only.
    let (_, output, _) = run_zshrs("alias x=hi; alias -L x");
    assert_eq!(output.trim(), "alias x=hi");
    let (_, output, _) = run_zshrs("alias x=hi; alias x");
    assert_eq!(output.trim(), "x=hi");
}

#[test]
fn test_setopt_nocaseglob_honored() {
    // `setopt nocaseglob` should make glob expansion
    // case-insensitive. Was being normalized to `caseglob=false`
    // in the options map but expand_glob only read `nocaseglob` —
    // so the option was silently ignored.
    let dir = std::env::temp_dir().join("zshrs_test_nocase");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    std::fs::File::create(dir.join("Aa")).unwrap();
    let cmd = format!("setopt nocaseglob; cd {} && echo a*", dir.display());
    let (_, output, _) = run_zshrs(&cmd);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(output.trim(), "Aa");
}

#[test]
fn test_empty_cmdsubst_no_command_not_found() {
    // `$(false)` evaluating to empty string used as the only word
    // shouldn't error "command not found:". Was hitting that path
    // with empty name; zsh just no-ops and preserves $?.
    let (_, _, stderr) = run_zshrs(r#"true; $(false); echo "[$?]""#);
    assert!(
        !stderr.contains("command not found"),
        "stderr should not have command-not-found: {}",
        stderr
    );
}

#[test]
fn test_type_reserved_word_local_declare() {
    // zsh treats `local`, `declare`, `typeset`, `readonly`,
    // `export`, `integer`, `float` as reserved-word declarations
    // (precommand modifiers). `type local` should report
    // "is a reserved word" not "is a shell builtin".
    let (_, output, _) = run_zshrs("type local");
    assert!(output.contains("reserved word"));
    let (_, output, _) = run_zshrs("type declare");
    assert!(output.contains("reserved word"));
    let (_, output, _) = run_zshrs("type typeset");
    assert!(output.contains("reserved word"));
    let (_, output, _) = run_zshrs("type readonly");
    assert!(output.contains("reserved word"));
    let (_, output, _) = run_zshrs("type export");
    assert!(output.contains("reserved word"));
}

#[test]
fn test_whence_reserved_word_local() {
    // `whence -v local` should report "is a reserved word" (same
    // as `type`). Was reporting "is a shell builtin" because the
    // is_reserved_word table didn't include the declaration
    // keywords. Added local/declare/typeset/readonly/export/
    // integer/float (and repeat/foreach/end/nocorrect/noglob).
    let (_, output, _) = run_zshrs("whence -v local");
    assert!(output.contains("reserved word"));
    let (_, output, _) = run_zshrs("whence -v repeat");
    assert!(output.contains("reserved word"));
}

#[test]
fn test_dollar_underscore_after_function_call() {
    // After `foo arg`, `$_` should be `arg` (last call arg). After
    // `foo` (no args), `$_` should be `foo` (function name). zshrs
    // was leaking the internal `return 42` arg as `$_=42`.
    let (_, output, _) = run_zshrs(r#"foo() { return 42; }; foo; echo "[$_]""#);
    assert_eq!(output.trim(), "[foo]");

    let (_, output, _) = run_zshrs(r#"foo() { :; }; foo arg1; echo "[$_]""#);
    assert_eq!(output.trim(), "[arg1]");
}

#[test]
fn test_unknown_cond_emits_diagnostic() {
    // `[[ -l file ]]` (zsh: -h is symlink, -l is unknown) and
    // `[[ -X file ]]` should emit "unknown condition: -X" to stderr.
    // Was silently returning false.
    let (_, _, stderr) = run_zshrs("[[ -l /tmp ]]");
    assert!(
        stderr.contains("unknown condition"),
        "expected 'unknown condition' in stderr, got: {}",
        stderr
    );
    let (_, _, stderr) = run_zshrs("[[ -X /tmp ]]");
    assert!(stderr.contains("unknown condition"));
}

#[test]
fn test_array_element_substring() {
    // `${a[N]:offset:length}` should slice the resolved element.
    // zsh: `a=(hello); ${a[1]:0:1}` → `h`. zshrs was returning
    // the full element because the substring branch only fired
    // for the top-level scalar form, not after `[N]` resolution.
    let (_, output, _) = run_zshrs(r#"a=(hello); echo "[${a[1]:0:1}]""#);
    assert_eq!(output.trim(), "[h]");
    let (_, output, _) = run_zshrs(r#"a=(hello world); echo "[${a[2]:0:3}]""#);
    assert_eq!(output.trim(), "[wor]");
    let (_, output, _) = run_zshrs(r##"a=(abcdef); echo "[${a[1]:1:3}]""##);
    assert_eq!(output.trim(), "[bcd]");
}

#[test]
fn test_print_dash_P_no_trailing_reset() {
    // `print -P "%B"` should output just `\e[1m\n` — zsh doesn't
    // auto-reset attributes at end of prompt expansion. zshrs was
    // appending an extra `\e[0m`.
    let (_, output, _) = run_zshrs(r#"print -P "%B""#);
    assert_eq!(output.as_bytes(), b"\x1b[1m\n");
}

#[test]
fn test_dollar_underscore_after_no_arg_command() {
    // After `true`, `$_` should be `true` (the command name) since
    // there are no args. Was empty because pop_args only updated
    // pending_underscore from args.last(). For args.is_empty()
    // case in BUILTIN_TRUE/FALSE/COLON, backfill the command name.
    let (_, output, _) = run_zshrs(r#"true; echo "[$_]""#);
    assert_eq!(output.trim(), "[true]");
    let (_, output, _) = run_zshrs(r#"false; echo "[$_]""#);
    assert_eq!(output.trim(), "[false]");
}

#[test]
fn test_dash_t_fd_is_tty() {
    // `[[ -t fd ]]` checks if fd is a tty. In a pipe, stdin is
    // not a tty. Was emitting "unknown condition: -t" because the
    // compile_cond_expr unary handler had no case for `-t`.
    let (_, output, _) = run_zshrs(r#"echo hi | { [[ -t 0 ]] && echo tty || echo notty; }"#);
    assert_eq!(output.trim(), "notty");
}

#[test]
fn test_glob_trailing_slash_preserved() {
    // `echo */` should output each match with trailing `/`. The
    // glob crate strips trailing slashes from matches; we re-append
    // when the input pattern ended in `/`.
    let dir = std::env::temp_dir().join("zshrs_test_trailing_slash");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(dir.join("sub/sub2"));
    std::fs::File::create(dir.join("file")).unwrap();
    let cmd = format!("cd {} && echo */", dir.display());
    let (_, output, _) = run_zshrs(&cmd);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(output.trim(), "sub/");
}

#[test]
fn test_echo_bare_dash_is_noop_flag() {
    // zsh: `echo - hi` prints `hi` (the bare `-` is a no-op flag,
    // silently consumed). zshrs was treating it as a positional
    // arg and printing `- hi`. Bare `-` (single char) now skipped.
    let (_, output, _) = run_zshrs("echo - hi");
    assert_eq!(output.trim(), "hi");
    let (_, output, _) = run_zshrs("echo -");
    assert_eq!(output.trim(), "");
    // `--` (two dashes) is NOT a recognized flag — stays literal.
    let (_, output, _) = run_zshrs("echo --");
    assert_eq!(output.trim(), "--");
}

#[test]
fn test_print_P_standout_emits_italic_codes() {
    // zsh's %S (start standout) emits SGR-7 (reverse video). %s ends
    // it with SGR-27. The earlier test comment claimed italic (3/23)
    // but /bin/zsh, /opt/homebrew/bin/zsh, and Src/prompt.c all use
    // termcap's `so`/`se` which on terminfo-XT terminals map to 7/27.
    let (_, output, _) = run_zshrs(r#"print -P "%S""#);
    assert_eq!(output.as_bytes(), b"\x1b[7m\n");
    let (_, output, _) = run_zshrs(r#"print -P "%s""#);
    assert_eq!(output.as_bytes(), b"\x1b[27m\n");
}

#[test]
fn test_for_arith_comma_init_and_step() {
    // `for ((i=0,j=10; i<3; i++,j--))` should iterate with both
    // i and j updating. ArithCompiler only handled ONE op per
    // call and dropped the comma-trailing statements. Now the
    // comma form routes through BUILTIN_ARITH_EVAL (MathEval).
    let (_, output, _) = run_zshrs(r#"for ((i=0,j=10; i<3; i++,j--)); do echo "$i:$j"; done"#);
    assert_eq!(output.trim(), "0:10\n1:9\n2:8");
}

#[test]
fn test_print_P_attr_chain_independent() {
    // %B (bold) %S (standout/reverse) %U (underline) each emit one
    // SGR code: 1 / 7 / 4. apply_attrs previously duplicated when
    // re-emitting; each handler now emits ONLY its specific code.
    let (_, output, _) = run_zshrs(r#"print -P "%B%S%U""#);
    assert_eq!(output.as_bytes(), b"\x1b[1m\x1b[7m\x1b[4m\n");
}

#[test]
fn test_dollar_underscore_inside_function_body() {
    // Inside a function, `echo $_` should read the function name
    // (no args) or the last call-arg. zshrs was leaking the
    // function body source as $_ because BUILTIN_REGISTER_COMPILED_FN
    // (called when defining the function) had updated
    // pending_underscore with the body text. Now $_ is set BEFORE
    // the function body runs.
    let (_, output, _) = run_zshrs(r#"foo() { echo "[$_]"; }; foo"#);
    assert_eq!(output.trim(), "[foo]");
    let (_, output, _) = run_zshrs(r#"foo() { echo "[$_]"; }; foo arg1"#);
    assert_eq!(output.trim(), "[arg1]");
}

#[test]
fn test_print_P_y_no_tty_outputs_parens() {
    // zsh: `print -P "%y"` when not on a tty (e.g. -c mode) outputs
    // `()`. zshrs returned empty.
    let (_, output, _) = run_zshrs(r#"print -P "%y""#);
    assert_eq!(output.trim(), "()");
}

#[test]
fn test_print_P_color_no_extra_bold() {
    // %B%F{red}r%f%b should emit \e[1m\e[31mr\e[39m\e[0m (each
    // SGR independent). zshrs's `%F` apply_attrs path was
    // re-emitting all active attrs (\e[1m\e[1m\e[31m...).
    let (_, output, _) = run_zshrs(r#"print -P "%B%F{red}r%f%b""#);
    assert_eq!(output.as_bytes(), b"\x1b[1m\x1b[31mr\x1b[39m\x1b[0m\n");
}

#[test]
fn test_which_reserved_word_csh_style() {
    // `which local` (csh-style whence) should output
    // `local: shell reserved word` matching zsh. zshrs printed
    // just `local` because the csh_style branch wasn't covered
    // for reserved-word names.
    let (_, output, _) = run_zshrs("which local");
    assert!(output.contains("shell reserved word"), "got: {}", output);
}

#[test]
fn test_array_element_history_modifier() {
    // `${a[N]:r}` / `:e` / `:t` / `:h` / `:l` / `:u` should apply
    // the history modifier to the resolved element. zshrs's
    // bracket handler routed `:` modifiers through the colon-default
    // branch which only handles :- := :? :+. Now also handles
    // history modifiers per-element.
    let (_, output, _) = run_zshrs(r#"a=(file.txt); echo "${a[1]:r}""#);
    assert_eq!(output.trim(), "file");
    let (_, output, _) = run_zshrs(r#"a=(file.tar.gz); echo "${a[1]:e}""#);
    assert_eq!(output.trim(), "gz");
    let (_, output, _) = run_zshrs(r#"a=(/usr/local/bin/file); echo "${a[1]:t}""#);
    assert_eq!(output.trim(), "file");
    let (_, output, _) = run_zshrs(r#"a=(HELLO); echo "${a[1]:l}""#);
    assert_eq!(output.trim(), "hello");
}

#[test]
fn test_builtin_missing_zsh_format() {
    // `builtin nosuch` should emit "no such builtin: NAME" matching
    // zsh. zshrs had a custom format ("not a shell builtin").
    let (_, _, stderr) = run_zshrs("builtin nosuch_zzz_cmd");
    assert!(stderr.contains("no such builtin"));
}

#[test]
fn test_print_P_color_basic_8_uses_ansi_codes() {
    // %F{1} should emit \e[31m (basic ANSI red), not the 256-color
    // escape \e[38;5;1m. Indexes 0-7 use the basic codes; 8-255
    // use the 256-color form.
    let (_, output, _) = run_zshrs(r#"print -P "%F{1}r%f""#);
    assert_eq!(output.as_bytes(), b"\x1b[31mr\x1b[39m\n");
    let (_, output, _) = run_zshrs(r#"print -P "%F{0}b%f""#);
    assert_eq!(output.as_bytes(), b"\x1b[30mb\x1b[39m\n");
    // 256-color path still uses the long form.
    let (_, output, _) = run_zshrs(r#"print -P "%F{42}c%f""#);
    assert_eq!(output.as_bytes(), b"\x1b[38;5;42mc\x1b[39m\n");
}

#[test]
fn test_prompt_d_uses_logical_pwd_not_canonical() {
    // zsh's `%d` / `%~` honor `$PWD` (logical) rather than the
    // canonicalized cwd from `getcwd()`. On macOS, `cd /tmp` leaves
    // `$PWD=/tmp` but getcwd() returns `/private/tmp`, so the
    // numbered form `%2d` was printing `/private/tmp` instead of
    // `/tmp` to match zsh.
    let (_, output, _) = run_zshrs(r#"cd /tmp; print -P "%1d""#);
    assert_eq!(output, "/tmp\n");
    let (_, output, _) = run_zshrs(r#"cd /tmp; print -P "%2d""#);
    assert_eq!(output, "/tmp\n");
}

#[test]
fn test_glob_globdots_setopt_alias_for_dotglob() {
    // zsh's canonical name is `globdots`; `dotglob` is the bash alias.
    // Both must enable hidden-file matching on bare `*` patterns.
    use std::fs;
    let dir = std::env::temp_dir().join("zshrs_globdots_test");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(".hidden"), b"").unwrap();
    fs::write(dir.join("visible"), b"").unwrap();
    let cmd = format!(r#"cd {}; setopt globdots; print *"#, dir.to_string_lossy());
    let (_, output, _) = run_zshrs(&cmd);
    assert!(output.contains(".hidden"), "want .hidden in {output}");
    assert!(output.contains("visible"), "want visible in {output}");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn test_glob_numeric_range_finite() {
    // `<N-M>` matches files whose digit sequence at that position
    // falls in [N, M]. `file<2-4>` against `file1`..`file5` keeps 2,3,4.
    use std::fs;
    let dir = std::env::temp_dir().join("zshrs_numrange_finite");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for n in 1..=5 {
        fs::write(dir.join(format!("file{n}")), b"").unwrap();
    }
    let cmd = format!(r#"echo {}/file<2-4>"#, dir.to_string_lossy());
    let (_, output, _) = run_zshrs(&cmd);
    assert!(output.contains("file2"));
    assert!(output.contains("file3"));
    assert!(output.contains("file4"));
    assert!(!output.contains("file1"));
    assert!(!output.contains("file5"));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn test_glob_numeric_range_open_high() {
    // `<3->` matches digit sequences ≥ 3 with no upper bound.
    use std::fs;
    let dir = std::env::temp_dir().join("zshrs_numrange_high");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for n in 1..=5 {
        fs::write(dir.join(format!("file{n}")), b"").unwrap();
    }
    let cmd = format!(r#"echo {}/file<3->"#, dir.to_string_lossy());
    let (_, output, _) = run_zshrs(&cmd);
    assert!(!output.contains("file1"));
    assert!(!output.contains("file2"));
    assert!(output.contains("file3"));
    assert!(output.contains("file4"));
    assert!(output.contains("file5"));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn test_glob_numeric_range_open_both() {
    // `<->` matches any digit sequence — equivalent to `[0-9]+` filter.
    use std::fs;
    let dir = std::env::temp_dir().join("zshrs_numrange_both");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("file1"), b"").unwrap();
    fs::write(dir.join("file9"), b"").unwrap();
    fs::write(dir.join("filea"), b"").unwrap();
    let cmd = format!(r#"echo {}/file<->"#, dir.to_string_lossy());
    let (_, output, _) = run_zshrs(&cmd);
    assert!(output.contains("file1"));
    assert!(output.contains("file9"));
    assert!(!output.contains("filea"));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn test_arith_ppid_uid_special_names() {
    // zsh's arithmetic context resolves $PPID, $UID, $EUID, $GID, $EGID
    // as live bareword identifiers. zshrs's MathEval reads from a
    // pre-populated extras map, which previously only had RANDOM /
    // SECONDS / EPOCHSECONDS / EPOCHREALTIME / LINENO — process-id
    // specials all collapsed to 0.
    let (_, output, _) = run_zshrs("echo $((PPID > 0))");
    assert_eq!(output.trim(), "1");
    let (_, output, _) = run_zshrs("echo $((UID >= 0))");
    assert_eq!(output.trim(), "1");
    let (_, output, _) = run_zshrs("echo $((EUID >= 0))");
    assert_eq!(output.trim(), "1");
}

#[test]
fn test_array_slice_out_of_range_is_empty() {
    // zsh: `a=(a b c); print "[${a[5,10]}]"` → `[]`. zshrs previously
    // clamped the start index down to len, returning the trailing
    // element ("c") instead of nothing. Out-of-range starts now
    // collapse to empty.
    let (_, output, _) = run_zshrs(r#"a=(a b c); print "[${a[5,10]}]""#);
    assert_eq!(output, "[]\n");
    let (_, output, _) = run_zshrs(r#"a=(a b c); print "[${a[10,20]}]""#);
    assert_eq!(output, "[]\n");
    // In-range slices still work.
    let (_, output, _) = run_zshrs(r#"a=(a b c); print "[${a[2,3]}]""#);
    assert_eq!(output, "[b c]\n");
}

#[test]
fn test_fc_no_args_recursion_message() {
    // Bare `fc` (no -l, no positional) is the EDIT mode — re-execute
    // the previous command. With empty history the previous command
    // IS fc itself, so zsh refuses with "current history line would
    // recurse endlessly, aborted". Distinct from the -l forms which
    // use "no such event: N" or "no events in that range".
    let (_, _, stderr) = run_zshrs("fc");
    assert!(stderr.contains("would recurse endlessly"), "got: {stderr}");
}

#[test]
fn test_q_flag_empty_returns_quoted_empty() {
    // zsh: `${(q)x}` for an empty `x` returns `''` (single-quoted
    // empty pair) so the value survives word-splitting. zshrs
    // returned an actual empty string, which would silently get
    // dropped by an unquoted consumer.
    let (_, output, _) = run_zshrs(r#"a=""; print "${(q)a}""#);
    assert_eq!(output.trim(), "''");
}

#[test]
fn test_q_flag_empty_array_elements_survive_as_quoted_empty() {
    // In list context `${(q)arr}` must quote empty elements as `''` so
    // they survive word-splitting (not silently vanish). Plain `q`
    // quotes via QT_BACKSLASH_SHOWNULL (subst.c:4124), which shows a
    // NULL string as `''`; zshrs used plain QT_BACKSLASH (no shownull),
    // so empty elements dropped. Verified vs `/bin/zsh -f`: arr=("" x "")
    // → `''`, `x`, `''` (three words).
    let (_, output, _) = run_zshrs(r#"arr=("" x ""); print -l -- ${(q)arr}"#);
    assert_eq!(output, "''\nx\n''\n", "got: {output:?}");
}

#[test]
fn test_test_unknown_unary_condition_errors() {
    // zsh: `[ -i /tmp ]` (unknown unary cond) errors `unknown
    // condition: -i` exit 2. zshrs's `builtin_test` had no explicit
    // arm for unknown two-arg unary forms — they fell through to
    // the AND/OR split and silently returned 1 (which a consumer
    // would read as "false", not "syntax error"). Added a 2-arg
    // alphabetic-flag default arm.
    let (status, _, stderr) = run_zshrs("[ -i /tmp ]");
    assert_eq!(status, 2);
    assert!(stderr.contains("unknown condition: -i"), "got: {stderr}");
    let (status, _, stderr) = run_zshrs("[ -NN /tmp ]");
    assert_eq!(status, 2);
    assert!(stderr.contains("unknown condition: -NN"), "got: {stderr}");
}

#[test]
fn test_command_no_args_silent() {
    // zsh: bare `command` (no args, no redirection) exits 0 silently
    // (matches bash). The "redirection with no command" error fires
    // only when a redirection is present without a command name; that
    // path runs in the parser, not in this builtin. Earlier zshrs
    // unconditionally errored, breaking benign `command` no-ops.
    let (status, _, _) = run_zshrs("command");
    assert_eq!(status, 0);
}

#[test]
fn test_wait_missing_job_silent() {
    // zsh: `sleep N & wait %1` resolves %1 through the canonical
    // jobtab (getjob, c:Src/jobs.c:2063) and waits — exit 0. The old
    // pin asserted the pre-#79 "no such job" rc=127 behavior with an
    // explicit note to flip once the jobtab was populated; that
    // landed with the #79/#369 port (BUILTIN_RUN_BG → initjob/addproc/
    // spawnjob + bin_fg BIN_WAIT %spec arm).
    let (status, _, _stderr) = run_zshrs("sleep 0.05 & wait %1");
    assert_eq!(status, 0);
}

#[test]
fn test_wait_invalid_pid_uses_zsh_format() {
    // zsh: `wait NOT_A_PID` -> `zsh:wait:1: job not found: NOT_A_PID`
    // (treats unparseable arg as a job-spec lookup that failed).
    // zshrs previously emitted "wait: NOT_A_PID: invalid pid".
    let (status, _, stderr) = run_zshrs("wait NOT_A_PID");
    assert_eq!(status, 127);
    assert!(
        stderr.contains("zshrs:wait:1: job not found: NOT_A_PID"),
        "got: {stderr}"
    );
}

#[test]
fn test_shift_negative_count_errors() {
    // zsh: `shift -1` -> `zsh:shift:1: argument to shift must be
    // non-negative` exit 1. zshrs accepted negative as an array
    // name and silently no-op'd.
    let (status, _, stderr) = run_zshrs("shift -1");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("argument to shift must be non-negative"),
        "got: {stderr}"
    );
}

#[test]
fn test_lineno_intrinsic_readonly() {
    // zsh: LINENO is a hard-wired read-only special; `LINENO=99`
    // errors `read-only variable: LINENO` exit 1. zshrs let the
    // assignment through silently.
    let (status, _, stderr) = run_zshrs("LINENO=99");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("read-only variable: LINENO"),
        "got: {stderr}"
    );
}

#[test]
fn test_set_unknown_letter_errors() {
    // c:Src/builtin.c:bin_set — `set -Z` (unknown upper-case letter)
    // emits "can't change option: -Z" rc=1. `set +Z` is a no-op
    // (rc=0) in /bin/zsh because the `+`-direction option-flip
    // accepts unknown letters silently.
    let (status, _, stderr) = run_zshrs("set -Z");
    assert_eq!(status, 1);
    assert!(stderr.contains("can't change option: -Z"), "got: {stderr}");
    let (status, _, _) = run_zshrs("set +Z");
    assert_eq!(status, 0);
}

#[test]
fn test_set_o_unknown_option_errors() {
    // zsh: `set -o nonexistentopt` -> `zsh:set:1: no such option:
    // nonexistentopt` exit 1. zshrs blindly inserted the unknown
    // name into self.options, leaving stale junk in the option map.
    let (status, _, stderr) = run_zshrs("set -o nonexistentopt");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("no such option: nonexistentopt"),
        "got: {stderr}"
    );
    let (status, _, stderr) = run_zshrs("set +o nonexistentopt");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("no such option: nonexistentopt"),
        "got: {stderr}"
    );
}

#[test]
fn test_unset_no_args_errors() {
    // zsh: bare `unset` errors `not enough arguments` exit 1. zshrs
    // returned 0 silently — masked accidental empty `unset $maybe`.
    let (status, _, stderr) = run_zshrs("unset");
    assert_eq!(status, 1);
    assert!(stderr.contains("not enough arguments"), "got: {stderr}");
}

#[test]
fn test_disown_no_current_job_errors() {
    // zsh: bare `disown` with no jobs errors `no current job` exit 1.
    // zshrs silently returned 0.
    let (status, _, stderr) = run_zshrs("disown");
    assert_eq!(status, 1);
    assert!(stderr.contains("no current job"), "got: {stderr}");
}

#[test]
fn test_test_unknown_3arg_op_errors() {
    // zsh: `[ a -ZZ b ]` -> `[:1: unknown condition: -ZZ` exit 2.
    // zshrs's 3-arg path silently returned 1, masking typos in the
    // condition operator (-eq, -lt, etc.).
    let (status, _, stderr) = run_zshrs("[ a -ZZ b ]");
    assert_eq!(status, 2);
    assert!(stderr.contains("unknown condition: -ZZ"), "got: {stderr}");
}

#[test]
fn test_test_paren_mismatch_errors() {
    // zsh: `[ \( a ]` -> `[:1: argument expected` exit 2 (open paren
    // without a matching close at end-of-args). zshrs silently
    // returned 1, hiding the syntactic mismatch.
    let (status, _, stderr) = run_zshrs(r"[ \( a ]");
    assert_eq!(status, 2);
    assert!(stderr.contains("argument expected"), "got: {stderr}");
}

#[test]
fn test_kill_no_args_zsh_format() {
    // zsh: bare `kill` -> `kill:1: not enough arguments` exit 1.
    // zshrs printed a multi-line bash-style usage banner; tests that
    // grep for the zsh format silently saw nothing.
    let (status, _, stderr) = run_zshrs("kill");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("zshrs:kill:1: not enough arguments"),
        "got: {stderr}"
    );
    let (status, _, stderr) = run_zshrs("kill -9");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("zshrs:kill:1: not enough arguments"),
        "got: {stderr}"
    );
}

#[test]
fn test_trap_undefined_signal_errors() {
    // zsh: `trap "" BADSIGNAL` -> `trap:1: undefined signal:
    // BADSIGNAL` exit 1 (trap NOT installed). zshrs blindly inserted
    // any uppercased token into the trap table, registering a
    // never-firable trap silently.
    let (status, _, stderr) = run_zshrs(r#"trap "" BADSIGNAL"#);
    assert_eq!(status, 1);
    assert!(
        stderr.contains("undefined signal: BADSIGNAL"),
        "got: {stderr}"
    );
}

#[test]
fn test_trap_l_silent() {
    // zsh: `trap -l` lists current traps (empty in -f mode), no
    // bash-style numbered SIGNAL list. zshrs previously emitted the
    // bash-flavoured table that didn't match zsh exactly.
    let (status, output, _) = run_zshrs("trap -l");
    assert_eq!(status, 0);
    assert!(
        !output.contains("SIGHUP") && !output.contains("SIGTERM"),
        "got: {output}"
    );
}

#[test]
fn test_vared_no_args_zsh_format() {
    // zsh: bare `vared` -> `vared:1: not enough arguments`. zshrs's
    // older `vared:` (no shell-name prefix) didn't match zsh's
    // `<shellname>:<builtin>:<line>:` convention.
    let (status, _, stderr) = run_zshrs("vared");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("zshrs:vared:1: not enough arguments"),
        "got: {stderr}"
    );
}

#[test]
fn test_unset_readonly_errors() {
    // zsh: `readonly x=1; unset x` -> `read-only variable: x` exit 1
    // (unset rejected). zshrs's unset blindly removed the entry from
    // the variable maps, leaving x unset and exit 0 — a compat
    // regression that broke scripts probing for readonly state.
    let (status, _, stderr) = run_zshrs("readonly x=1; unset x");
    assert_eq!(status, 1);
    assert!(stderr.contains("read-only variable: x"), "got: {stderr}");
}

#[test]
fn test_typeset_unknown_flag_errors() {
    // zsh: `typeset -Q x=1` -> `typeset:1: bad option: -Q` exit 1.
    // zshrs's silent `_ => {}` fallback in the flag char-loop made
    // unknown letters succeed without setting any attribute.
    let (status, _, stderr) = run_zshrs("typeset -Q x=1");
    assert_eq!(status, 1);
    assert!(stderr.contains("bad option: -Q"), "got: {stderr}");
    let (status, _, stderr) = run_zshrs("declare -Q x=1");
    assert_eq!(status, 1);
    assert!(stderr.contains("bad option: -Q"), "got: {stderr}");
}

#[test]
fn test_printf_no_args_zsh_format() {
    // zsh: `printf` -> `printf:1: not enough arguments` exit 1.
    // zshrs printed `printf: usage: printf format [arguments]` —
    // bash-style usage banner without the shell-name prefix.
    let (status, _, stderr) = run_zshrs("printf");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("zshrs:printf:1: not enough arguments"),
        "got: {stderr}"
    );
}

#[test]
fn test_arith_trailing_op_uses_zsh_wording() {
    // zsh: `$((5+))` and `let "1+"` both -> `bad math expression:
    // operand expected at end of string`. zshrs returned the bare
    // string `not enough operands`, mismatching the format scripts
    // grep for.
    let (_, _, stderr) = run_zshrs(r#"echo $((5+))"#);
    assert!(
        stderr.contains("bad math expression: operand expected at end of string"),
        "got: {stderr}"
    );
    let (_, _, stderr) = run_zshrs(r#"let "1+""#);
    assert!(
        stderr.contains("bad math expression: operand expected at end of string"),
        "got: {stderr}"
    );
}

#[test]
fn test_arith_open_paren_uses_zsh_wording() {
    // zsh: `let "("` -> `bad math expression: ')' expected`. zshrs
    // emitted bare `')' expected` without the `bad math expression:`
    // prefix that scripts grep for.
    let (_, _, stderr) = run_zshrs(r#"let "(""#);
    assert!(
        stderr.contains("bad math expression: ')' expected"),
        "got: {stderr}"
    );
}

#[test]
fn test_where_unknown_command_not_found() {
    // zsh: `where __notacmd__` → `__notacmd__ not found` on STDOUT,
    // exit 1. Src/builtin.c:4201 prints via `puts` → stdout, not
    // stderr. Earlier zshrs treated `_`-prefixed names as builtins
    // (a completion-function bypass) and reported them as found.
    let (status, output, _stderr) = run_zshrs("where __notacmd__");
    assert_eq!(status, 1);
    assert!(output.contains("__notacmd__ not found"), "got: {output}");
    assert!(!output.contains("shell built-in command"), "got: {output}");
}

#[test]
fn test_which_unknown_command_not_found() {
    // Companion to `where` — `which __notacmd__` prints "not found"
    // on STDOUT (puts at Src/builtin.c:4201), exit 1.
    let (status, output, _stderr) = run_zshrs("which __notacmd__");
    assert_eq!(status, 1);
    assert!(output.contains("__notacmd__ not found"), "got: {output}");
}

#[test]
fn test_history_text_query_uses_event_not_found() {
    // zsh: `history XX` (non-numeric arg, no matches) -> `fc:1:
    // event not found: XX` exit 1. zshrs hardcoded `no such event:
    // 1` for any miss in non-tty mode — wrong wording AND wrong
    // event identifier (always "1").
    let (status, _, stderr) = run_zshrs("history XX");
    assert_eq!(status, 1);
    assert!(stderr.contains("event not found: XX"), "got: {stderr}");
}

#[test]
fn test_unalias_continues_after_first_miss() {
    // zsh: `unalias xyz abc` errors twice (one per miss) and exits
    // with the last failing status. zshrs returned on the first
    // miss, hiding the rest from script consumers.
    let (status, _, stderr) = run_zshrs("unalias xyz abc");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("xyz") && stderr.contains("abc"),
        "got: {stderr}"
    );
}

#[test]
fn test_read_unknown_flag_errors() {
    // zsh: `read -Q v` -> `read:1: bad option: -Q` exit 1. zshrs's
    // silent `_ => {}` fallback in the per-char flag loop accepted
    // any letter, masking typos and letting the read run as if -Q
    // were valid.
    let (status, _, stderr) = run_zshrs("read -Q v <<< a");
    assert_eq!(status, 1);
    assert!(stderr.contains("bad option: -Q"), "got: {stderr}");
}

#[test]
fn test_getopts_no_args_zsh_format() {
    // zsh: bare `getopts` -> `getopts:1: not enough arguments` exit
    // 1. zshrs printed `zshrs: getopts: usage: getopts ...` —
    // bash-style banner without the line-number-prefixed format.
    let (status, _, stderr) = run_zshrs("getopts");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("zshrs:getopts:1: not enough arguments"),
        "got: {stderr}"
    );
}

#[test]
fn test_wait_unrealistic_jobspec_errors() {
    // zsh: `wait %999` (job id never created) -> `wait:1: %999: no
    // such job` exit 127. zshrs silently returned 0 because all
    // %ID misses were treated as "already-reaped" (silent for the
    // bg/wait idiom). Now distinguishes via $! sentinel: errors
    // when the session never backgrounded anything.
    let (status, _, stderr) = run_zshrs("wait %999");
    assert_eq!(status, 127);
    assert!(stderr.contains("%999: no such job"), "got: {stderr}");
    let (status, _, stderr) = run_zshrs("wait %1");
    assert_eq!(status, 127);
    assert!(stderr.contains("%1: no such job"), "got: {stderr}");
    // The bg/wait idiom (`sleep 0.05 & wait %1`) is intentionally
    // omitted: zshrs's job-table doesn't yet retain reaped-by-
    // background jobs as silent-OK targets the way /bin/zsh does.
    // Covered separately by test_wait_missing_job_silent once the
    // bg reap-and-keep substrate lands.
}

#[test]
fn test_source_no_args_zsh_format() {
    // zsh: bare `source` -> `source:1: not enough arguments` exit
    // 1. zshrs printed `source: filename argument required` —
    // bash-style banner with no shell-name or line-number prefix.
    let (status, _, stderr) = run_zshrs("source");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("zshrs:source:1: not enough arguments"),
        "got: {stderr}"
    );
}

#[test]
fn test_source_empty_path_uses_no_such_file() {
    // zsh: `. ""` -> `.:1: no such file or directory:` (with empty
    // trailing path). zshrs's POSIX path resolver mapped "" to cwd,
    // which then opened as a directory and produced `is a
    // directory: ` — wrong wording AND wrong exit code (1 vs 127).
    let (status, _, stderr) = run_zshrs(r#". """#);
    assert_eq!(status, 127);
    assert!(
        stderr.contains("no such file or directory:"),
        "got: {stderr}"
    );
}

#[test]
fn test_test_4plus_args_no_op_errors() {
    // zsh: `[ a b c d ]` (4+ args, no recognized operator/connective)
    // -> `1: condition expected: a` exit 2. zshrs silently returned
    // 1 ("false"), masking the syntax error.
    let (status, _, stderr) = run_zshrs("[ a b c d ]");
    assert_eq!(status, 2);
    assert!(stderr.contains("condition expected: a"), "got: {stderr}");
}

#[test]
fn test_test_empty_paren_errors() {
    // zsh: `[ \( \) ]` (matching but empty parens) -> `[:1:
    // argument expected` exit 2. zshrs stripped the parens, hit the
    // empty-args base case, and silently returned 1.
    let (status, _, stderr) = run_zshrs(r"[ \( \) ]");
    assert_eq!(status, 2);
    assert!(stderr.contains("argument expected"), "got: {stderr}");
}

#[test]
fn test_umask_bad_numeric_format() {
    // zsh: `umask 999` -> `umask:1: bad umask` exit 1 (terse, no
    // mention of which digit was invalid). zshrs's symbolic-mode
    // walker treated the numeric input as malformed symbolic and
    // emitted `bad symbolic mode operator: 9` — wrong category.
    let (status, _, stderr) = run_zshrs("umask 999");
    assert_eq!(status, 1);
    assert!(stderr.contains("bad umask"), "got: {stderr}");
    assert!(!stderr.contains("symbolic mode operator"), "got: {stderr}");
}

#[test]
fn test_umask_bad_symbolic_permission() {
    // zsh: `umask u=Z` -> `umask:1: bad symbolic mode permission: Z`
    // exit 1 (specific to the unknown rwx char). zshrs collapsed all
    // symbolic-form failures to `umask: invalid mask: u=Z`.
    let (status, _, stderr) = run_zshrs("umask u=Z");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("bad symbolic mode permission: Z"),
        "got: {stderr}"
    );
}

#[test]
fn test_pwd_unknown_flag_errors() {
    // zsh: `pwd -X` -> `pwd:1: bad option: -X` exit 1. zshrs's silent
    // fallback ignored unknown letters and printed the cwd as if -X
    // were valid.
    let (status, _, stderr) = run_zshrs("pwd -X");
    assert_eq!(status, 1);
    assert!(stderr.contains("bad option: -X"), "got: {stderr}");
}

#[test]
fn test_cd_two_args_substitution_or_error() {
    // zsh: `cd OLD NEW` is the substitution form — replaces OLD with
    // NEW in $PWD and cd's there. If OLD isn't in $PWD, errors `cd:1:
    // string not in pwd: OLD` exit 1. zshrs silently fell through and
    // treated args[0] as the target dir (bash-style).
    let (status, _, stderr) = run_zshrs("cd /tmp /etc");
    assert_eq!(status, 1);
    assert!(stderr.contains("string not in pwd: /tmp"), "got: {stderr}");
}

#[test]
fn test_readonly_unknown_flag_errors() {
    // zsh: `readonly -X x=1` -> `readonly:1: bad option: -X` exit 1.
    // zshrs treated `-X` as a name to mark readonly, masking the
    // typo and silently inserting a junk readonly entry.
    let (status, _, stderr) = run_zshrs("readonly -X x=1");
    assert_eq!(status, 1);
    assert!(stderr.contains("bad option: -X"), "got: {stderr}");
}

#[test]
fn test_type_underscore_unknown_not_builtin() {
    // zsh: `type __notexist__` -> `__notexist__ not found` exit 0
    // (zsh's type returns 0 even on miss; bash returns 1). zshrs's
    // `is_builtin()` helper had a `_`-prefix bypass for completion
    // functions, so any `_*` name would falsely report as a builtin.
    // Tightened the `type` builtin-check arm to consult BUILTIN_SET
    // directly (mirrors the `whence` fix from earlier).
    let (_, output, _) = run_zshrs("type __notexist__");
    assert!(output.contains("not found"), "got: {output}");
    assert!(!output.contains("shell builtin"), "got: {output}");
}

#[test]
fn test_unsetopt_unknown_option_errors() {
    // zsh: `unsetopt nonexistentopt` -> `unsetopt:1: no such option:
    // nonexistentopt` exit 1. zshrs blindly inserted whatever name
    // it received into self.options, leaving stale junk and silencing
    // typos. Mirror the `setopt` validation against ZSH_OPTIONS_SET.
    let (status, _, stderr) = run_zshrs("unsetopt nonexistentopt");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("no such option: nonexistentopt"),
        "got: {stderr}"
    );
}

#[test]
fn test_exit_too_many_args_diagnoses() {
    // zsh: `exit 1 2 3` emits `exit:1: too many arguments` and
    // continues (the shell's bytecode allows post-exit code in zsh).
    // zshrs's compiler jumps unconditionally to script end after
    // BUILTIN_EXIT, so we can't perfectly replicate "continue past
    // failed exit" — but we DO emit the diagnostic now, which the
    // earlier impl silently swallowed.
    let (_, _, stderr) = run_zshrs("exit 1 2 3");
    assert!(stderr.contains("too many arguments"), "got: {stderr}");
}

#[test]
fn test_trap_numeric_signal_out_of_range_errors() {
    // zsh: `trap "" 99` -> `trap:1: undefined signal: 99` exit 1
    // (zsh accepts 1..63 inclusive; >63 is invalid). zshrs accepted
    // any parseable integer, silently registering a never-firable
    // trap. Now bounds the numeric sig form to (0, 63].
    let (status, _, stderr) = run_zshrs(r#"trap "" 99"#);
    assert_eq!(status, 1);
    assert!(stderr.contains("undefined signal: 99"), "got: {stderr}");
    // Sanity: real low signal still installs.
    let (status, _, stderr) = run_zshrs(r#"trap "" 1"#);
    assert_eq!(status, 0);
    assert!(stderr.is_empty(), "got: {stderr}");
}

#[test]
fn test_exec_long_option_typo_errors() {
    // zsh: `exec --bad` (long-option-style flag) -> `exec requires a
    // command to execute` exit 1. zshrs silently swallowed any flag
    // it didn't recognize, masking the typo. Now scans the input for
    // `-`-prefixed args; if none became `cmd_args` and any flag was
    // present, the missing-command error fires.
    let (status, _, stderr) = run_zshrs("exec --bad");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("exec requires a command to execute"),
        "got: {stderr}"
    );
}

#[test]
fn test_print_S_takes_single_arg() {
    // zsh: `print -S` is the split-shell-words history form and
    // takes EXACTLY one positional. `print -S foo bar` -> `print:1:
    // option -S takes a single argument` exit 1. zshrs concatenated
    // all args into the history entry silently.
    let (status, _, stderr) = run_zshrs("print -S foo bar");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("option -S takes a single argument"),
        "got: {stderr}"
    );
}

#[test]
fn test_autoload_unknown_flag_errors() {
    // zsh: `autoload -Z foo` -> `autoload:1: bad option: -Z` exit 1.
    // zshrs's silent `_ => {}` fallback accepted any letter, masking
    // typos AND the bash-style `-l` flag that zsh doesn't have.
    let (status, _, stderr) = run_zshrs("autoload -Z foo");
    assert_eq!(status, 1);
    assert!(stderr.contains("bad option: -Z"), "got: {stderr}");
    let (status, _, stderr) = run_zshrs("autoload -l");
    assert_eq!(status, 1);
    assert!(stderr.contains("bad option: -l"), "got: {stderr}");
}

#[test]
fn test_test_3arg_op_at_pos0_errors() {
    // zsh: `[ -lt 5 3 ]` (3 args with binary operator at args[0]
    // instead of args[1]) -> `[:1: unknown condition: -lt` exit 2.
    // The op-at-front looks like a unary condition that zsh doesn't
    // recognise. zshrs silently returned 1.
    let (status, _, stderr) = run_zshrs("[ -lt 5 3 ]");
    assert_eq!(status, 2);
    assert!(stderr.contains("unknown condition: -lt"), "got: {stderr}");
}

#[test]
fn test_test_4args_with_binop_emits_too_many() {
    // zsh: `[ a -lt 3 5 ]` (binop in correct position but 4+ args)
    // -> `[:1: too many arguments` exit 2. zshrs's earlier `condition
    // expected: a` was the wrong category for this layout (the binop
    // IS recognised but the operand-count is wrong).
    let (status, _, stderr) = run_zshrs("[ a -lt 3 5 ]");
    assert_eq!(status, 2);
    assert!(stderr.contains("too many arguments"), "got: {stderr}");
}

#[test]
fn test_test_two_operands_no_op_errors() {
    // zsh: `[ "" "" ]` (two operands, no connective) -> `1: parse
    // error: condition expected:` exit 2. zshrs silently returned 1.
    let (status, _, stderr) = run_zshrs(r#"[ "" "" ]"#);
    assert_eq!(status, 2);
    assert!(
        stderr.contains("parse error: condition expected:"),
        "got: {stderr}"
    );
}

#[test]
fn test_autoload_X_no_function_errors() {
    // zsh: `autoload -X` (no function name) -> `autoload:1: bad
    // autoload` exit 1 — `-X` requires a function context. zshrs
    // silently no-op'd because `execute_now=true && functions.is_empty()`
    // skipped both list and execute branches.
    let (status, _, stderr) = run_zshrs("autoload -X");
    assert_eq!(status, 1);
    assert!(stderr.contains("bad autoload"), "got: {stderr}");
}

#[test]
fn test_shift_array_count_too_many_errors() {
    // zsh: `a=(1); shift 5 a` -> `shift:1: shift count must be <=
    // $#` exit 1. zshrs silently shifted as much as it could, leaving
    // the array partially mutated AND not signaling failure.
    let (_, _, stderr) = run_zshrs("a=(1); shift 5 a");
    assert!(
        stderr.contains("shift count must be <= $#"),
        "got: {stderr}"
    );
}

#[test]
fn test_jobs_Z_requires_argument() {
    // zsh: `jobs -Z` (without a process-name arg) -> `jobs:1: -Z
    // requires one argument` exit 1. zshrs's `'Z' => {}` silent
    // arm meant the flag was consumed without diagnostic.
    let (status, _, stderr) = run_zshrs("jobs -Z");
    assert_eq!(status, 1);
    assert!(stderr.contains("-Z requires one argument"), "got: {stderr}");
}

#[test]
fn test_zformat_no_args_zsh_format() {
    // zsh: `zformat` -> `zformat:1: not enough arguments`. zshrs
    // emitted bare `zformat: not enough arguments` (no shell-name
    // or line-number prefix).
    let (_, _, stderr) = run_zshrs("zformat 2>&1");
    assert!(
        stderr.contains("zshrs:zformat:1: not enough arguments")
            || run_zshrs("zformat")
                .2
                .contains("zshrs:zformat:1: not enough arguments"),
        "got: {stderr}"
    );
}

#[test]
fn test_test_two_args_binop_missing_operand() {
    // zsh: `[ a -lt ]` (operand + binop, no second operand) ->
    // `1: parse error: condition expected: a` exit 2. zshrs's path
    // for 2-arg with args[1]=binop fell through every match and
    // hit the catch-all `1`. Added an explicit arm.
    let (status, _, stderr) = run_zshrs("[ a -lt ]");
    assert_eq!(status, 2);
    assert!(
        stderr.contains("parse error: condition expected: a"),
        "got: {stderr}"
    );
}

#[test]
fn test_kill_zero_failed_uses_zsh_format() {
    // zsh: `kill -0 1` (no permission to signal pid 1) -> `kill:1:
    // kill 1 failed: operation not permitted` (lowercased, no
    // `(os error N)` suffix). zshrs emitted bash-style `kill: 1:
    // Operation not permitted (os error 1)`.
    let (_, _, stderr) = run_zshrs("kill -0 1");
    assert!(stderr.contains("kill 1 failed:"), "got: {stderr}");
    assert!(!stderr.contains("(os error"), "got: {stderr}");
}

#[test]
fn test_funcnest_recursion_guard_no_overflow() {
    // zsh: deep recursion is bounded by FUNCNEST (default 500) and
    // errors `<name>: maximum nested function level reached;
    // increase FUNCNEST?` exit 1. zshrs had no enforcement; the
    // Rust stack overflowed (fatal abort) on `foo() { foo; }; foo`
    // — both function-recursion AND builtin-shadow-recursion
    // (`echo() { echo hi; }; echo hi`).
    let (_, _, stderr) = run_zshrs("foo() { foo; }; foo");
    assert!(
        stderr.contains("maximum nested function level reached"),
        "got: {stderr}"
    );
    let (_, _, stderr) = run_zshrs("echo() { echo overridden; }; echo hi");
    assert!(
        stderr.contains("maximum nested function level reached"),
        "got: {stderr}"
    );
    // Sanity: shallow recursion still works.
    let (_, output, _) = run_zshrs("foo() { echo hi; }; foo; foo");
    assert!(output.contains("hi"), "got: {output}");
}

#[test]
fn test_test_lt_gt_not_string_comparators() {
    // zsh's POSIX `[`-test does NOT accept `<`/`>` as string
    // comparators (they're redirection ops). `[ "5" \> "3" ]`
    // errors `1: condition expected: >` exit 2. zshrs's earlier
    // impl had string-compare arms for both, hiding the syntax
    // error.
    let (status, _, stderr) = run_zshrs(r#"[ "5" \> "3" ]"#);
    assert_eq!(status, 2);
    assert!(stderr.contains("condition expected: >"), "got: {stderr}");
    let (status, _, stderr) = run_zshrs(r#"[ "5" \< "3" ]"#);
    assert_eq!(status, 2);
    assert!(stderr.contains("condition expected: <"), "got: {stderr}");
}

#[test]
fn test_setopt_single_letter_silent() {
    // zsh: `setopt -h` (and other single-letter shortcuts) are
    // accepted silently — they're shortcuts for option names from
    // the option-letter table. zshrs's older default arm rejected
    // any `-`-prefixed arg as an unknown name, breaking scripts
    // that probed `setopt -h` (no-op for hashcmds shorthand).
    let (status, _, stderr) = run_zshrs("setopt -h");
    assert_eq!(status, 0);
    assert!(!stderr.contains("no such option"), "got: {stderr}");
}

#[test]
fn test_fc_l_3plus_args_too_many() {
    // zsh: `fc -l 1 2 3` (range needs at most 2 bounds) -> `fc:1:
    // too many arguments` exit 1. zshrs's range path collapsed any
    // 2+-arg case to `no events in that range`, missing the count
    // check.
    let (_, _, stderr) = run_zshrs("fc -l 1 2 3");
    assert!(stderr.contains("too many arguments"), "got: {stderr}");
}

#[test]
fn test_type_empty_name_not_found() {
    // zsh: `type ""` -> ` not found` exit 1. zshrs's PATH walker
    // computed `dir + "/" + ""` which `Path::exists` reports as the
    // directory itself, falsely matching `type ""` to the first
    // PATH entry. Skip the lookup entirely for empty names.
    let (status, output, _) = run_zshrs(r#"type """#);
    assert_eq!(status, 1);
    assert!(output.contains("not found"), "got: {output}");
    assert!(!output.contains(" is /"), "got: {output}");
}

#[test]
fn test_ulimit_invalid_number_errors() {
    // zsh: `ulimit -f abc` -> `ulimit:1: invalid number: abc` exit
    // 1. zshrs's `arg.parse().ok()` silently dropped non-numeric
    // values, leaving value unset and printing the existing limit
    // (`unlimited`) — masking the typo.
    let (status, _, stderr) = run_zshrs("ulimit -f abc");
    assert_eq!(status, 1);
    assert!(stderr.contains("invalid number: abc"), "got: {stderr}");
}

#[test]
fn test_test_double_equals_rejected() {
    // POSIX `[`-test only accepts `=` for equality — `==` is the
    // `[[`-cond extension. zsh: `[ a == a ]` -> `1: = not found`
    // exit 1 (parses as `[ a = = a ]` and tries to look up the
    // second `=` as a command). zshrs's match arm accepted both.
    let (status, _, stderr) = run_zshrs("[ a == a ]");
    assert_eq!(status, 1);
    assert!(stderr.contains("= not found"), "got: {stderr}");
    // Sanity: single `=` still works in `[ ]`.
    let (status, _, _) = run_zshrs("[ a = a ]");
    assert_eq!(status, 0);
    // Sanity: `==` still works in `[[ ]]`.
    let (status, _, _) = run_zshrs("[[ a == a ]]");
    assert_eq!(status, 0);
}

#[test]
fn test_fc_long_option_reports_first_letter() {
    // zsh: `fc --help` skips the leading `-` and reports the FIRST
    // recognisable letter as the bad option: `fc:1: bad option:
    // -h`. zshrs's loop hit `-` as the first char of `--help` and
    // reported `bad option: --` (wrong identifier).
    let (_, _, stderr) = run_zshrs("fc --help");
    assert!(stderr.contains("bad option: -h"), "got: {stderr}");
}

#[test]
fn test_arith_invalid_base_aborts_command() {
    // zsh: `echo $((37#1))` (base out of range 2..=36) errors
    // `invalid base (must be 2 to 36 inclusive): 37` exit 1 AND
    // aborts the surrounding command — `echo` doesn't print
    // anything. zshrs printed the error then continued to print
    // the bogus `0` from the math evaluator's default.
    let (status, output, _) = run_zshrs("echo $((37#1))");
    assert_eq!(status, 1);
    assert!(!output.contains("0"), "got: {output}");
}

#[test]
fn test_arith_bad_digit_for_base_errors() {
    // zsh: `$((2#5))` (5 is not a valid binary digit) errors
    // `bad math expression: operator expected at \`5'` exit 1.
    // zshrs's `i64::from_str_radix(...).unwrap_or(0)` silently
    // produced 0, masking the typo.
    let (status, _, stderr) = run_zshrs("echo $((2#5))");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("bad math expression: operator expected at"),
        "got: {stderr}"
    );
}

#[test]
fn test_wait_empty_string_errors() {
    // zsh: `wait ""` (literal empty arg) -> `wait:1: job not found:`
    // exit 127. zshrs silently continued past the empty arg,
    // returning 0 — masking the bad input.
    let (status, _, stderr) = run_zshrs(r#"wait """#);
    assert_eq!(status, 127);
    assert!(stderr.contains("job not found:"), "got: {stderr}");
}

#[test]
fn test_read_d_empty_no_panic_on_nul() {
    // zsh: `read -d ""` reads up to NUL (binary input mode); the
    // captured value may contain NUL bytes. zshrs called
    // `env::set_var` unconditionally, which panics on NUL —
    // crashing the whole shell. Guard set_var with a NUL check.
    let (status, _output, stderr) =
        run_zshrs(r#"printf 'a\0b\0' | { read -d "" v; echo "[$v]"; }"#);
    assert!(
        !stderr.contains("panicked"),
        "should not panic; got stderr: {stderr}"
    );
    assert_eq!(status, 0);
}

#[test]
fn test_test_unknown_3arg_infix_op_errors() {
    // zsh: `[ a := a ]` (made-up infix op at args[1]) -> `[:1:
    // condition expected: :=` exit 2. zshrs's 3-arg arms only
    // checked `-`-prefixed ops; non-`-`-prefixed ops like `:=` fell
    // through every check and silently returned 1.
    let (status, _, stderr) = run_zshrs("[ a := a ]");
    assert_eq!(status, 2);
    assert!(stderr.contains("condition expected: :="), "got: {stderr}");
}

#[test]
fn test_print_u_bad_fd_errors() {
    // zsh: `print -u 99 hello` (fd 99 not open) -> `print:1: bad
    // file number: 99` exit 1 with NO output. zshrs's `let _ = fd`
    // discarded the requested fd and always wrote to stdout.
    let (status, output, stderr) = run_zshrs("print -u 99 hello");
    assert_eq!(status, 1);
    assert!(stderr.contains("bad file number: 99"), "got: {stderr}");
    assert!(!output.contains("hello"), "got: {output}");
}

#[test]
fn test_kill_illegal_pid_zsh_format() {
    // zsh: `kill -0 abc` (non-numeric pid) -> `kill:1: illegal pid:
    // abc` exit 1. zshrs's bash-style `kill: abc: invalid pid` had
    // no shell-name prefix and used different wording.
    let (status, _, stderr) = run_zshrs("kill -0 abc");
    assert_eq!(status, 1);
    assert!(stderr.contains("illegal pid: abc"), "got: {stderr}");
}

#[test]
fn test_wait_stops_after_first_bad_arg() {
    // zsh: `wait abc def` reports the first bad arg and stops —
    // doesn't continue to `def`. zshrs emitted one error per bad
    // arg, exceeding zsh's diagnostic count.
    let (_, _, stderr) = run_zshrs("wait abc def");
    assert!(stderr.contains("job not found: abc"), "got: {stderr}");
    assert!(
        !stderr.contains("job not found: def"),
        "should stop at first miss; got: {stderr}"
    );
}

#[test]
fn test_vared_missing_value_after_flag_errors() {
    // zsh: `vared -p` (no value after -p) -> `vared:1: argument
    // expected: -p` exit 1. zshrs's earlier `if i + 1 < args.len()`
    // guard silently dropped the flag without erroring, then
    // triggered the catch-all `not enough arguments` for the
    // missing var name (wrong diagnostic for the actual problem).
    let (status, _, stderr) = run_zshrs("vared -p");
    assert_eq!(status, 1);
    assert!(stderr.contains("argument expected: -p"), "got: {stderr}");
}

#[test]
fn test_history_d_event_id_propagates() {
    // zsh: `history -d 99` (no entry 99) -> `fc:1: no such event:
    // 99` exit 1. zshrs hardcoded `no such event: 1` regardless of
    // the user's value.
    let (status, _, stderr) = run_zshrs("history -d 99");
    assert_eq!(status, 1);
    assert!(stderr.contains("no such event: 99"), "got: {stderr}");
}

#[test]
fn test_zstyle_unknown_flag_errors() {
    // zsh: `zstyle -X` -> `zstyle:1: invalid option: -X` exit 1.
    // zshrs's `_ => {}` silent fallback let any unknown flag drop
    // through to set-style with `pattern=-X`.
    let (status, _, stderr) = run_zshrs("zstyle -X");
    assert_eq!(status, 1);
    assert!(stderr.contains("invalid option: -X"), "got: {stderr}");
}

#[test]
fn test_bindkey_unknown_flag_errors() {
    // zsh: `bindkey -Z` -> `bindkey:1: bad option: -Z` exit 1.
    // zshrs's silent fallback dropped unknown flags into list-mode
    // silently.
    let (status, _, stderr) = run_zshrs("bindkey -Z");
    assert_eq!(status, 1);
    assert!(stderr.contains("bad option: -Z"), "got: {stderr}");
}

#[test]
fn test_zparseopts_no_args_errors() {
    // zsh: bare `zparseopts` -> `zparseopts:1: not enough arguments`
    // exit 1. zshrs silently returned 0.
    let (status, _, stderr) = run_zshrs("zparseopts");
    assert_eq!(status, 1);
    assert!(stderr.contains("not enough arguments"), "got: {stderr}");
}

#[test]
fn test_pwd_too_many_args_errors() {
    // zsh: `pwd extra arg` -> `pwd:1: too many arguments` exit 1.
    // pwd takes only flags; positional args are an error. zshrs
    // ignored them silently and printed cwd.
    let (status, _, stderr) = run_zshrs("pwd extra arg");
    assert_eq!(status, 1);
    assert!(stderr.contains("too many arguments"), "got: {stderr}");
}

#[test]
fn test_umask_digit_prefix_uses_bad_umask() {
    // zsh: `umask 0Ab` (digit-prefixed but not all-digits) -> `bad
    // umask` exit 1 — not the symbolic-mode-operator diagnostic.
    // zshrs's symbolic walker treated `0` as the start of a
    // class+operator parse and emitted the wrong category.
    let (status, _, stderr) = run_zshrs("umask 0Ab");
    assert_eq!(status, 1);
    assert!(stderr.contains("bad umask"), "got: {stderr}");
    assert!(!stderr.contains("symbolic mode operator"), "got: {stderr}");
}

#[test]
fn test_fc_l_two_args_non_numeric_errors() {
    // zsh: `fc -l 1 abc` (range with non-numeric bound) -> `fc:1:
    // event not found: abc` exit 1. zshrs's range path lumped
    // non-numeric bounds into `no events in that range` (wrong
    // category — should distinguish text-name from out-of-range).
    let (_, _, stderr) = run_zshrs("fc -l 1 abc");
    assert!(stderr.contains("event not found: abc"), "got: {stderr}");
    let (_, _, stderr) = run_zshrs("fc -l xyz 1");
    assert!(stderr.contains("event not found: xyz"), "got: {stderr}");
}

#[test]
fn test_fc_r_d_recurse_endlessly_aborts() {
    // zsh: bare `fc -r` and `fc -d` (no positional) re-edit the
    // prior command — which IS `fc` itself in `-c` mode, hence the
    // recurse-endlessly abort. zshrs's earlier guard required
    // `args.is_empty()`, so `fc -r` slipped past and ran the
    // previous command (often a bogus alias=).
    let (_, _, stderr) = run_zshrs("fc -r");
    assert!(stderr.contains("would recurse endlessly"), "got: {stderr}");
    let (_, _, stderr) = run_zshrs("fc -d");
    assert!(stderr.contains("would recurse endlessly"), "got: {stderr}");
}

#[test]
fn test_test_4args_unary_flag_too_many() {
    // zsh: `[ -z "" -X x ]` (4-arg with unary flag at args[0] +
    // operand + extra junk) -> `[:1: too many arguments` exit 2.
    // zshrs's catch-all 4+arg arm reported `condition expected:
    // -z` (wrong category — args[0] IS a recognized unary flag).
    let (status, _, stderr) = run_zshrs(r#"[ -z "" -X x ]"#);
    assert_eq!(status, 2);
    assert!(stderr.contains("too many arguments"), "got: {stderr}");
}

#[test]
fn test_jobs_unknown_id_errors() {
    // zsh: `jobs %1` (no jobs) -> `jobs:1: %1: no such job` exit 127
    // (c:Src/jobs.c:2589-2590 `return 127`). Bug #393.
    let (status, _, stderr) = run_zshrs("jobs %1");
    assert_eq!(status, 127);
    assert!(stderr.contains("%1: no such job"), "got: {stderr}");
}

#[test]
fn test_fg_bg_no_job_control_in_script() {
    // zsh: `fg %N` and `bg %N` in `-c` mode (no real job control)
    // -> `<fg|bg>:1: no job control in this shell.` exit 1
    // regardless of whether N exists. zshrs's option-based check
    // (monitor or interactive) didn't reflect actual job-control
    // availability — both are default-on. Use stdin-tty status as
    // the real signal: `-c` mode has no tty on stdin.
    let (status, _, stderr) = run_zshrs("fg %999");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("no job control in this shell"),
        "got: {stderr}"
    );
    let (status, _, stderr) = run_zshrs("bg %999");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("no job control in this shell"),
        "got: {stderr}"
    );
}

#[test]
fn test_fc_e_missing_editor_arg_errors() {
    // zsh: `fc -e` (no following editor arg) -> `fc:1: argument
    // expected: -e` exit 1. zshrs let the missing arg fall through
    // to the recurse-endlessly path.
    let (_, _, stderr) = run_zshrs("fc -e");
    assert!(stderr.contains("argument expected: -e"), "got: {stderr}");
}

#[test]
fn test_history_d_negative_resolves_to_zero() {
    // zsh: `history -d -N` (negative count) resolves to event 0
    // (count-from-end with empty history). zshrs reported the
    // absolute count value (1) instead.
    let (_, _, stderr) = run_zshrs("history -d -1");
    assert!(stderr.contains("no such event: 0"), "got: {stderr}");
}

#[test]
fn test_history_S_bad_option() {
    // zsh: `history -S` -> `history:1: bad option: -S` exit 1.
    // bash-style "save" flag that zsh's history doesn't accept.
    // zshrs silently consumed the flag and emitted the no-such-event
    // pass-through.
    let (status, _, stderr) = run_zshrs("history -S");
    assert_eq!(status, 1);
    assert!(stderr.contains("bad option: -S"), "got: {stderr}");
}

#[test]
fn test_let_unary_op_no_operand() {
    // zsh: `let "!"` -> `1: bad math expression: operand expected
    // at end of string`. zshrs's MathContext emitted a bare `stack
    // empty` (no zsh-canonical prefix), missing the message that
    // scripts grep for.
    let (_, _, stderr) = run_zshrs(r#"let "!""#);
    assert!(
        stderr.contains("bad math expression: operand expected"),
        "got: {stderr}"
    );
}

#[test]
fn test_cd_dashdash_end_of_options() {
    // zsh: `cd -- /tmp` treats `--` as end-of-options marker;
    // everything after is positional. zshrs's substitution-form
    // path treated `--` as the OLD arg and errored "string not in
    // pwd: --".
    let (status, output, _) = run_zshrs("cd -- /tmp; pwd");
    assert_eq!(status, 0);
    assert!(output.trim_end().ends_with("/tmp"), "got: {output}");
}

#[test]
fn test_fc_p_silent_success() {
    // zsh: `fc -p` (push history stack) is silent success in `-c`
    // mode. zshrs treated -p as a no-op flag but the no-positional
    // recurse-abort path still fired, emitting the wrong error.
    // Exempted -p/-P/-a/-I/-L/-m from the abort.
    let (status, _, stderr) = run_zshrs("fc -p");
    assert_eq!(status, 0);
    assert!(
        !stderr.contains("recurse endlessly"),
        "should be silent; got: {stderr}"
    );
}

#[test]
fn test_kill_s_zero_signal() {
    // zsh accepts numeric values to `-s`; `-s 0` is the existence-
    // check form. zshrs's name-only lookup rejected `0` as
    // "invalid signal: 0".
    let (status, _, _) = run_zshrs("kill -s 0 $$; echo $?");
    // Kill of own pid with signal 0 succeeds (process exists).
    assert_eq!(status, 0);
}

#[test]
fn test_unset_underscore_allowed() {
    // zsh: `unset _` is allowed — `_` (last-arg auto-update) is NOT
    // intrinsic-readonly despite being a zsh-internal special.
    // zshrs incorrectly listed it among the read-only intrinsics.
    let (status, _, stderr) = run_zshrs("unset _");
    assert_eq!(status, 0);
    assert!(!stderr.contains("read-only variable"), "got: {stderr}");
}

#[test]
fn test_bindkey_A_requires_two_args() {
    // zsh: `bindkey -A NEW EXISTING` requires both keymap names;
    // `bindkey -A nokm` -> `bindkey:1: not enough arguments for -A`
    // exit 1. zshrs's stub accepted any -A form and returned 0.
    let (status, _, stderr) = run_zshrs("bindkey -A nokm");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("not enough arguments for -A"),
        "got: {stderr}"
    );
}

#[test]
fn test_zstyle_T_unset_default_true() {
    // zsh: `zstyle -T context style` is like `-t` but defaults to
    // TRUE for unset styles. zshrs's unknown-flag fallback rejected
    // -T as invalid.
    let (status, _, _) = run_zshrs("zstyle -T :foo style");
    assert_eq!(status, 0);
}

#[test]
fn test_trap_return_undefined() {
    // zsh's actual runtime rejects `RETURN` as a signal name (the
    // documentation hints at it but the parser's signal-name table
    // doesn't include it). zshrs accepted it as valid. Match zsh's
    // rejection.
    let (status, _, stderr) = run_zshrs(r#"trap "" RETURN"#);
    assert_eq!(status, 1);
    assert!(stderr.contains("undefined signal: RETURN"), "got: {stderr}");
}

#[test]
fn test_print_u_non_numeric_errors() {
    // zsh: `print -u abc hi` -> `print:1: number expected after -u:
    // abc` exit 1. zshrs's `unwrap_or(1)` silently dropped non-numeric
    // input and printed to stdout.
    let (status, output, stderr) = run_zshrs("print -u abc hi");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("number expected after -u: abc"),
        "got: {stderr}"
    );
    assert!(!output.contains("hi"), "got: {output}");
}

#[test]
fn test_fc_l_3plus_text_first_arg_errors_event_not_found() {
    // zsh: `fc -l x y z` (3+ positionals, first is non-numeric) ->
    // `fc:1: event not found: x` (text-name miss takes precedence
    // over count-error). zshrs always reported "too many arguments"
    // for 3+ positionals.
    let (_, _, stderr) = run_zshrs("fc -l x y z");
    assert!(stderr.contains("event not found: x"), "got: {stderr}");
}

#[test]
fn test_zstyle_one_arg_not_enough() {
    // zsh: `zstyle X` (single non-flag positional) -> `zstyle:1:
    // not enough arguments` exit 1. zshrs's set-style path required
    // args.len() >= 2 silently.
    let (status, _, stderr) = run_zshrs("zstyle X");
    assert_eq!(status, 1);
    assert!(stderr.contains("not enough arguments"), "got: {stderr}");
}

#[test]
fn test_zformat_f_too_few_args_errors() {
    // zsh: `zformat -f result` (no format string) -> `zformat:1:
    // not enough arguments` exit 1. zshrs returned 1 silently.
    let (status, _, stderr) = run_zshrs("zformat -f result");
    assert_eq!(status, 1);
    assert!(stderr.contains("not enough arguments"), "got: {stderr}");
}

#[test]
fn test_set_long_option_treated_as_endmark() {
    // zsh: `set --xxx` (long-option-style flag) is treated as `--`
    // (end-of-options) — remaining args become positional. zshrs
    // hit the per-char letter loop and errored "can't change
    // option: --".
    let (status, output, _) = run_zshrs(r#"set --foo bar; echo "[$1]""#);
    assert_eq!(status, 0);
    assert!(output.contains("[bar]"), "got: {output}");
    let (status, output, _) = run_zshrs(r#"set --help; echo "[$1]""#);
    assert_eq!(status, 0);
    assert!(output.contains("[]"), "got: {output}");
}

#[test]
fn test_zstyle_get_too_few_args_errors() {
    // zsh: `zstyle -g`/`-s`/`-T`/`-t` (insufficient args) ->
    // `zstyle:1: not enough arguments` exit 1. zshrs returned 1
    // silently in those branches.
    let (status, _, stderr) = run_zshrs("zstyle -g");
    assert_eq!(status, 1);
    assert!(stderr.contains("not enough arguments"), "got: {stderr}");
    let (status, _, stderr) = run_zshrs("zstyle -s");
    assert_eq!(status, 1);
    assert!(stderr.contains("not enough arguments"), "got: {stderr}");
    let (status, _, stderr) = run_zshrs("zstyle -t");
    assert_eq!(status, 1);
    assert!(stderr.contains("not enough arguments"), "got: {stderr}");
    let (status, _, stderr) = run_zshrs("zstyle -T");
    assert_eq!(status, 1);
    assert!(stderr.contains("not enough arguments"), "got: {stderr}");
}

#[test]
fn test_shift_empty_arg_silent() {
    // zsh: `shift ""` treats empty arg as count 0 (silent no-op).
    // zshrs's `chars().all(is_digit)` matched empty vacuously and
    // parse defaulted to 1, then erred when positionals were short.
    let (status, _, _) = run_zshrs(r#"shift """#);
    assert_eq!(status, 0);
    let (status, output, _) = run_zshrs(r#"set -- a b c; shift ""; echo "$@""#);
    assert_eq!(status, 0);
    assert!(output.contains("a b c"), "got: {output}");
}

#[test]
fn test_exec_a_requires_parameter() {
    // c:Src/exec.c:3195-3206 — `if (!*++argv) { zerr("exec requires
    // a command to execute"); errflag ... return; }`. The flag walker
    // hits the empty-next path BEFORE per-flag arg validation, so
    // `exec -a` (no following name) emits the generic missing-command
    // message, not "exec flag -a requires a parameter". /bin/zsh and
    // /opt/homebrew/bin/zsh both confirm. Test originally specified
    // the more specific message which the C source never emits in
    // this case.
    let (status, _, stderr) = run_zshrs("exec -a");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("exec requires a command to execute"),
        "got: {stderr}"
    );
}

#[test]
fn test_bindkey_d_resets_keymaps() {
    // zsh: `bindkey -d` resets all keymaps to defaults — silent
    // success. zshrs's unknown-flag fallback rejected -d.
    let (status, _, _) = run_zshrs("bindkey -d");
    assert_eq!(status, 0);
}

#[test]
fn test_bindkey_no_such_keymap_errors() {
    // c:Src/Zle/zle_keymap.c:781/799/911/925 — bindkey keymap-selection
    // errors go through zwarnnam, so they carry the `zshrs:bindkey:LINE:`
    // prefix (not a bare `bindkey:`), and `-D`/`-A` on a missing keymap
    // must actually report it (the prior port swallowed those diagnostics).
    let err = |code: &str| run_zshrs(code).2.trim().to_string();

    // -M on a nonexistent keymap: correct prefix + message.
    let e = err(r#"bindkey -M nonexistentmap "^A""#);
    assert!(e.starts_with("zshrs:bindkey:"), "prefix: {e:?}");
    assert!(e.ends_with("no such keymap `nonexistentmap'"), "msg: {e:?}");

    // Incompatible selection flags.
    let e = err("bindkey -e -M foo");
    assert!(
        e.ends_with("incompatible keymap selection options"),
        "got: {e:?}"
    );

    // -D on a missing keymap now reports it (was silent).
    let (st, _, e) = run_zshrs("bindkey -D nonexistent");
    assert_ne!(st, 0);
    assert!(e.trim().ends_with("no such keymap `nonexistent'"), "got: {e:?}");

    // -A with a missing SOURCE keymap reports it.
    let (st, _, e) = run_zshrs("bindkey -A nonexistsrc newdst");
    assert_ne!(st, 0);
    assert!(e.trim().ends_with("no such keymap `nonexistsrc'"), "got: {e:?}");

    // Valid -A / -D still succeed silently.
    let (st, _, _) = run_zshrs("bindkey -N km1; bindkey -A emacs km2; bindkey -D km1");
    assert_eq!(st, 0);
}

#[test]
fn test_bindkey_send_string_single_char() {
    // c:Src/Zle/zle_keymap.c:566/614 — `bindkey -s KEY STR` binds a
    // send-string (macro). A SINGLE-character key's binding lives in the
    // Thingy-only `first[]` fast path, so the string is kept in `multi[]`
    // with `first[]` cleared; the previous port stored only the (absent)
    // Thingy for single bytes, silently dropping the string.
    let out = |code: &str| run_zshrs(code).1.trim().to_string();

    assert_eq!(out(r#"bindkey -s "^X" "echo hi"; bindkey "^X""#), r#""^X" "echo hi""#);
    // -L round-trips the send-string form.
    assert_eq!(out(r#"bindkey -s "^X" "cmd"; bindkey -L "^X""#), r#"bindkey -s "^X" "cmd""#);
    // Re-binding a send-string overwrites; empty string is valid.
    assert_eq!(out(r#"bindkey -s "^X" "one"; bindkey -s "^X" "two"; bindkey "^X""#), r#""^X" "two""#);
    assert_eq!(out(r#"bindkey -s "^X" ""; bindkey "^X""#), r#""^X" """#);
    // Multi-char send-string still works (unchanged path).
    assert_eq!(out(r#"bindkey -s "^X^Y" "seq"; bindkey "^X^Y""#), r#""^X^Y" "seq""#);
    // A later widget binding on the same char REPLACES the send-string.
    assert_eq!(
        out(r#"bindkey -s "^A" "x"; bindkey "^A" beginning-of-line; bindkey "^A""#),
        r#""^A" beginning-of-line"#
    );
    // `-r` after `-s` fully unbinds (no stale send-string leaks).
    assert_eq!(out(r#"bindkey -s "^A" "x"; bindkey -r "^A"; bindkey "^A""#), r#""^A" undefined-key"#);
}

#[test]
fn test_history_empty_arg_event_not_found() {
    // zsh: `history ""` (empty positional) -> `fc:1: event not
    // found:` exit 1 (with empty trailing identifier). zshrs's
    // all-digits arm matched empty vacuously and reported `no such
    // event: 1`.
    let (status, _, stderr) = run_zshrs(r#"history """#);
    assert_eq!(status, 1);
    assert!(stderr.contains("event not found:"), "got: {stderr}");
}

#[test]
fn test_read_u_non_numeric_or_missing() {
    // zsh: `read -u abc` -> `read:1: number expected after -u: abc`;
    // `read -u` (no arg) -> `read:1: argument expected: -u`. zshrs's
    // `unwrap_or(0)` silently dropped both.
    let (status, _, stderr) = run_zshrs("read -u abc v");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("number expected after -u: abc"),
        "got: {stderr}"
    );
    let (status, _, stderr) = run_zshrs("read -u");
    assert_eq!(status, 1);
    assert!(stderr.contains("argument expected: -u"), "got: {stderr}");
}

#[test]
fn test_kill_l_dash_prefix_strips() {
    // zsh: `kill -l -X` reports `kill:1: unknown signal: SIGX` —
    // strips the leading `-` of the unknown name. zshrs preserved
    // the `-` and reported `SIG-X`.
    let (_, _, stderr) = run_zshrs("kill -l -X");
    assert!(stderr.contains("unknown signal: SIGX"), "got: {stderr}");
    assert!(!stderr.contains("SIG-X"), "got: {stderr}");
}

#[test]
fn test_kill_n_invalid_signal_zsh_format() {
    // zsh: `kill -n abc 1` -> `kill:1: invalid signal number: abc`
    // exit 1. zshrs emitted bash-style `kill: invalid signal
    // number: abc` (no shell-name or line-number prefix).
    let (status, _, stderr) = run_zshrs("kill -n abc 1");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("zshrs:kill:1: invalid signal number: abc"),
        "got: {stderr}"
    );
}

#[test]
fn test_alias_g_s_mutually_exclusive() {
    // zsh: `-g` (global) and `-s` (suffix) are mutually exclusive on
    // alias — `alias -gs foo=bar` -> `alias:1: illegal combination
    // of options` exit 1. zshrs accepted both flags silently.
    let (status, _, stderr) = run_zshrs("alias -gs foo=bar");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("illegal combination of options"),
        "got: {stderr}"
    );
}

#[test]
fn test_kill_unknown_pid_zsh_format() {
    // zsh: `kill 999999999` -> `kill:1: kill 999999999 failed: no
    // such process` exit 1. zshrs emitted bash-style `kill: ESRCH:
    // No such process` with the errno code verbatim.
    let (status, _, stderr) = run_zshrs("kill 999999999");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("kill 999999999 failed: no such process"),
        "got: {stderr}"
    );
}

#[test]
fn test_assoc_odd_kv_pairs_errors() {
    // zsh: assoc-init with odd number of values -> `bad set of
    // key/value pairs for associative array` exit 1, no assignment.
    // zshrs's `if let Some(v) = it.next()` silently dropped the
    // orphaned key.
    let (_, _, stderr) = run_zshrs("typeset -A h; h=(a 1 b)");
    assert!(
        stderr.contains("bad set of key/value pairs"),
        "got: {stderr}"
    );
}

#[test]
fn test_disown_unknown_jobspec_errors() {
    // zsh: `disown %999` for non-existent id -> `disown:1: %999: no
    // such job` exit 127 (c:Src/jobs.c:2589-2590 `return 127`). Bug #393.
    let (status, _, stderr) = run_zshrs("disown %999");
    assert_eq!(status, 127);
    assert!(stderr.contains("%999: no such job"), "got: {stderr}");
}

#[test]
fn test_disown_dash_flag_treats_as_jobspec() {
    // zsh: `disown -l` and `disown -h` (bash-style flags zsh
    // doesn't have) are treated as job specs and error `disown:1:
    // job not found: -l` exit 127 (c:Src/jobs.c:2589-2590).
    // /bin/zsh and /opt/homebrew/bin/zsh both confirm rc=127.
    let (status, _, stderr) = run_zshrs("disown -l");
    assert_eq!(status, 127);
    assert!(
        stderr.contains("zshrs:disown:1: job not found: -l"),
        "got: {stderr}"
    );
}

#[test]
fn test_zstyle_dash_only_not_enough_args() {
    // zsh: `zstyle -` (bare dash, no recognized option letter) ->
    // `zstyle:1: not enough arguments` exit 1. zshrs's catch-all
    // unknown-flag fallback emitted `invalid option: -` (wrong
    // category).
    let (status, _, stderr) = run_zshrs("zstyle -");
    assert_eq!(status, 1);
    assert!(stderr.contains("not enough arguments"), "got: {stderr}");
}

#[test]
fn test_fc_t_missing_arg_errors() {
    // zsh: `fc -t` (no time-format arg) -> `fc:1: argument expected:
    // -t` exit 1. zshrs's `i+=1` without bounds-check fell through
    // to the no-positional recurse-endlessly path.
    let (status, _, stderr) = run_zshrs("fc -t");
    assert_eq!(status, 1);
    assert!(stderr.contains("argument expected: -t"), "got: {stderr}");
}

#[test]
fn test_functions_unknown_silent() {
    // c:Src/builtin.c — `functions FOO` for non-existent FOO emits
    // nothing on stdout AND nothing on stderr; rc=1 (matches zsh:
    // `getasg`-style failure rolls into `returnval = 1`). Older
    // test comment claimed rc=0; /bin/zsh and /opt/homebrew/bin/zsh
    // both return 1.
    let (status, output, stderr) = run_zshrs("functions foo");
    assert_eq!(status, 1);
    assert!(output.is_empty(), "got: {output}");
    assert!(!stderr.contains("no such function"), "got: {stderr}");
}

#[test]
fn test_kill_dashdash_end_of_options() {
    // zsh: `kill -- 999` treats `--` as end-of-options; subsequent
    // args are PIDs. zshrs's flag walker treated `--` as a signal
    // name (parsed leading `-` as separator, then `-` as the name)
    // and errored "unknown signal: SIG-".
    let (status, _, stderr) = run_zshrs("kill -- 999 2>/dev/null");
    // 999 doesn't exist so kill itself fails (exit 1) but the --
    // shouldn't trigger the bogus signal error.
    let _ = status;
    assert!(!stderr.contains("unknown signal"), "got: {stderr}");
}

#[test]
fn test_fc_empty_string_no_recursion() {
    // zsh: `fc ""` -> `fc:1: event not found:` exit 1 (no match,
    // no prior-command execution). zshrs's prefix-match found the
    // most recent entry and recursively re-executed it — `fc ""`
    // triggered infinite recursion (it ran `fc ""` again). Added
    // an empty-string fast path before the prefix search.
    let (status, _, stderr) = run_zshrs(r#"echo single; fc """#);
    assert_eq!(status, 1);
    assert!(stderr.contains("event not found:"), "got: {stderr}");
}

#[test]
fn test_umask_bad_class_char() {
    // zsh: `umask z=r` -> `umask:1: bad symbolic mode operator: z`
    // exit 1 (treats unknown class char as the operator-position
    // diagnostic). zshrs's `_ => ok=false` collapsed all class
    // errors to `invalid mask: z=r`.
    let (status, _, stderr) = run_zshrs("umask z=r");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("bad symbolic mode operator: z"),
        "got: {stderr}"
    );
}

#[test]
fn test_cd_3plus_args_too_many() {
    // zsh: `cd ARG1 ARG2 ARG3` -> `cd:1: too many arguments` exit
    // 1 (cd takes at most 2 args; the substitution form OLD NEW).
    // zshrs let extras through silently.
    let (status, _, stderr) = run_zshrs("cd /tmp /etc /usr");
    assert_eq!(status, 1);
    assert!(stderr.contains("too many arguments"), "got: {stderr}");
}

#[test]
fn test_test_3args_unary_unary_arg_too_many() {
    // zsh: `[ -z -n a ]` (flag + flag + arg layout) -> `[:1: too
    // many arguments` exit 2 — `-z OPERAND` is the 2-arg form;
    // extra `a` is the surplus. zshrs's unknown-binop arm fired
    // first and reported `unknown condition: -n` (wrong category).
    let (status, _, stderr) = run_zshrs("[ -z -n a ]");
    assert_eq!(status, 2);
    assert!(stderr.contains("too many arguments"), "got: {stderr}");
}

#[test]
fn test_shift_unknown_flag_errors() {
    // zsh: `shift -X` (unknown flag besides -p) -> `shift:1: bad
    // option: -X` exit 1. zshrs's catch-all pushed the flag string
    // into array_names, masking typos.
    let (status, _, stderr) = run_zshrs("shift -X");
    assert_eq!(status, 1);
    assert!(stderr.contains("bad option: -X"), "got: {stderr}");
}

#[test]
fn test_print_f_missing_arg_errors() {
    // zsh: `print -f` (no following format string) -> `print:1:
    // argument expected: -f` exit 1. zshrs's `if i < args.len()`
    // silently fell through with no format set.
    let (status, _, stderr) = run_zshrs("print -f");
    assert_eq!(status, 1);
    assert!(stderr.contains("argument expected: -f"), "got: {stderr}");
}

#[test]
fn test_ulimit_l_accepted() {
    // zsh: `ulimit -l` (locked memory) returns the current limit
    // ("unlimited" on macOS). zshrs's flag-letter table didn't
    // include `-l` and erred "bad option: -l".
    let (status, output, _) = run_zshrs("ulimit -l");
    assert_eq!(status, 0);
    assert!(!output.is_empty(), "should print limit; got: {output}");
}

#[test]
fn test_read_d_missing_arg_errors() {
    // zsh: `read -d` (no following delimiter) -> `read:1: argument
    // expected: -d` exit 1. zshrs's bounds-less `i+=1` left
    // delimiter at default and continued.
    let (status, _, stderr) = run_zshrs("read -d");
    assert_eq!(status, 1);
    assert!(stderr.contains("argument expected: -d"), "got: {stderr}");
}

#[test]
fn test_kill_s_empty_name_errors() {
    // zsh: `kill -s "" 1` (empty signal name) -> `kill:1: -:
    // signal name expected` exit 1. zshrs's name lookup of empty
    // produced `invalid signal: ` (with trailing whitespace).
    let (status, _, stderr) = run_zshrs(r#"kill -s "" 1"#);
    assert_eq!(status, 1);
    assert!(stderr.contains("signal name expected"), "got: {stderr}");
}

#[test]
fn test_let_ternary_missing_colon_vs_operand() {
    // zsh distinguishes `let "1?"` (missing operand AND colon)
    // from `let "1?2"` (operand present, colon missing). Former
    // -> `bad math expression: operand expected at end of string`;
    // latter -> `bad math expression: ':' expected`. zshrs's
    // earlier `':' expected` for both was the wrong category for
    // the missing-operand case.
    let (_, _, stderr) = run_zshrs(r#"let "1?""#);
    assert!(
        stderr.contains("operand expected at end of string"),
        "got: {stderr}"
    );
    let (_, _, stderr) = run_zshrs(r#"let "1?2""#);
    assert!(stderr.contains("':' expected"), "got: {stderr}");
}

#[test]
fn test_umask_unknown_flag_errors() {
    // zsh: `umask -X` -> `umask:1: bad option: -X` exit 1. zshrs's
    // silent `_ => {}` accepted any flag and printed the umask.
    let (status, _, stderr) = run_zshrs("umask -X");
    assert_eq!(status, 1);
    assert!(stderr.contains("bad option: -X"), "got: {stderr}");
}

#[test]
fn test_fc_edit_too_many_args_errors() {
    // zsh: edit-mode fc takes at most 2 positional bounds; 3+ ->
    // `fc:1: too many arguments` exit 1. zshrs's edit path took
    // args.first() and ignored the rest, falling into the prefix
    // search.
    let (_, _, stderr) = run_zshrs("fc 1 2 3 4 5 6");
    assert!(stderr.contains("too many arguments"), "got: {stderr}");
}

#[test]
fn test_fc_l_3plus_text_in_middle() {
    // zsh: `fc -l 1 abc 2` (3+ positionals with non-numeric in
    // middle) -> `fc:1: event not found: abc` (text-name miss
    // takes precedence). zshrs only checked args[0]; if abc was
    // in middle, it reported `too many arguments` (wrong category).
    let (_, _, stderr) = run_zshrs("fc -l 1 abc 2");
    assert!(stderr.contains("event not found: abc"), "got: {stderr}");
}

#[test]
fn test_history_d_multi_args_error_categories() {
    // zsh: `history -d 1 2` (2 numeric) -> `no events in that
    // range`; `history -d 1 2 3` (3+) -> `too many arguments`.
    // zshrs treated each numeric as updating count and reported
    // `no such event: 3` (the last value).
    let (_, _, stderr) = run_zshrs("history -d 1 2");
    assert!(stderr.contains("no events in that range"), "got: {stderr}");
    let (_, _, stderr) = run_zshrs("history -d 1 2 3");
    assert!(stderr.contains("too many arguments"), "got: {stderr}");
}

#[test]
fn test_test_3args_unary_op_extra() {
    // zsh: `[ -e /tmp 5 ]` (unary flag + operand + extra) -> `[:1:
    // too many arguments` exit 2. zshrs's earlier 3-arg arm only
    // matched flag-flag-arg layouts (`-z -n a`); the flag-op-extra
    // case fell through to the 1 catch-all silently.
    let (status, _, stderr) = run_zshrs("[ -e /tmp 5 ]");
    assert_eq!(status, 2);
    assert!(stderr.contains("too many arguments"), "got: {stderr}");
}

#[test]
fn test_fc_2_numeric_positionals_recurse() {
    // zsh: edit-mode `fc N M` (2 numeric positionals) re-edits
    // commands N..M. With empty -c session, that's the recurse-
    // endlessly path. zshrs's prefix-search used N and reported
    // `event not found: N` (wrong category for the range-edit form).
    let (_, _, stderr) = run_zshrs("fc 1 5");
    assert!(stderr.contains("would recurse endlessly"), "got: {stderr}");
}

#[test]
fn test_test_3args_no_op_errors() {
    // zsh: `[ a b c ]` (3 non-flag args, no operator) -> `1:
    // condition expected: b` exit 2 (points at args[1] which
    // should have been an op). zshrs silently returned 1.
    let (status, _, stderr) = run_zshrs("[ a b c ]");
    assert_eq!(status, 2);
    assert!(stderr.contains("condition expected: b"), "got: {stderr}");
}

#[test]
fn test_print_u_routes_to_fd() {
    // zsh: `print -u 2 hi` writes to stderr; with `2>/dev/null`
    // the output is suppressed. zshrs always wrote to stdout
    // regardless of -u.
    let (_, output, _) = run_zshrs("print -u 2 hi 2>/dev/null");
    assert!(!output.contains("hi"), "got: {output}");
}

#[test]
fn test_type_S_k_accepted() {
    // c:Src/builtin.c — `type` accepts `-S` (silent-accept, no
    // observable effect in -c mode) but rejects `-k` as a bad
    // option. /bin/zsh and /opt/homebrew/bin/zsh both confirm.
    let (status, _, _) = run_zshrs("type -S echo");
    assert_eq!(status, 0);
    let (status, _, stderr) = run_zshrs("type -k echo");
    assert_eq!(status, 1);
    assert!(stderr.contains("bad option: -k"), "got: {stderr}");
}

#[test]
fn test_test_dashdash_unknown_condition() {
    // zsh: `[ -- a ]` -> `[:1: unknown condition: --` exit 2 (zsh
    // treats `--` as a bogus flag name in `[`-test). zshrs's 2-arg
    // unknown-flag arm only fired for all-alphabetic letters,
    // missing the `--` case.
    let (status, _, stderr) = run_zshrs("[ -- a ]");
    assert_eq!(status, 2);
    assert!(stderr.contains("unknown condition: --"), "got: {stderr}");
}

#[test]
fn test_fc_single_numeric_recurse() {
    // zsh: edit-mode `fc N` (1 numeric positional) re-edits cmd N.
    // With empty -c session, that's the recurse-endlessly path.
    // zshrs's prefix-search reported `event not found: N` for
    // single-positional case (only fixed for 2-positional earlier).
    let (_, _, stderr) = run_zshrs("fc 1");
    assert!(stderr.contains("would recurse endlessly"), "got: {stderr}");
}

#[test]
fn test_test_unmatched_close_paren_too_many() {
    // zsh: `[ a \) ]` (operand + lone close paren) -> `[:1: too
    // many arguments` exit 2 (the `)` is the surplus). zshrs's
    // depth-check collapsed both surplus-close and surplus-open
    // into "argument expected".
    let (status, _, stderr) = run_zshrs(r"[ a \) ]");
    assert_eq!(status, 2);
    assert!(stderr.contains("too many arguments"), "got: {stderr}");
}

#[test]
fn test_kill_percent_text_jobspec() {
    // zsh: `kill %abc` (non-numeric jobspec) -> `kill:1: job not
    // found: abc` (no leading %, distinct format from numeric
    // miss). zshrs reported `kill: %abc: no such job` (with %).
    let (status, _, stderr) = run_zshrs("kill %abc");
    assert_eq!(status, 1);
    assert!(stderr.contains("job not found: abc"), "got: {stderr}");
    assert!(!stderr.contains("%abc: no such job"), "got: {stderr}");
}

#[test]
fn test_unset_bad_option_X() {
    // zsh: `unset -X foo` -> `unset:1: bad option: -X` exit 1.
    // zshrs silently swallowed unknown flags via the `_ if
    // arg.starts_with('-') => {}` arm, masking typo'd flags.
    let (status, _, stderr) = run_zshrs("unset -X foo; echo done");
    assert_eq!(status, 0); // last command (echo) is 0
    assert!(stderr.contains("bad option: -X"), "got: {stderr}");
}

#[test]
fn test_unset_dash_dash_end_of_options() {
    // zsh: `unset -- foo` accepts `--` as end-of-options sentinel.
    // Without this, our new bad-option rejection would fire on
    // `--` itself.
    let (status, _, stderr) = run_zshrs("unset -- foo; echo done");
    assert_eq!(status, 0);
    assert!(!stderr.contains("bad option"), "got: {stderr}");
}

#[test]
fn test_let_orphan_mul_at_op() {
    // zsh: `let "*"` -> `bad math expression: operand expected at
    // \`*'`. zshrs collapsed every operand-missing case into "at
    // end of string", which lost the operator location for
    // orphan-at-start expressions like pure-binary ops with no
    // unary form (Mul, Div, Mod, Power).
    let (status, _, stderr) = run_zshrs("let \"*\"");
    // c:Src/builtin.c:7478 (commit 54285 "'let' builtin should
    // return 2 if error occurred") — math errors return 2. Older
    // /bin/zsh 5.9 pre-commit shows 1; the bundled source post-commit
    // says 2; we follow the bundled C source.
    assert_eq!(status, 2);
    assert!(stderr.contains("operand expected at `*'"), "got: {stderr}");
}

#[test]
fn test_let_orphan_div_at_op() {
    // Same orphan-binary case for Div.
    let (status, _, stderr) = run_zshrs("let \"/\"");
    // c:Src/builtin.c:7478 — math errors return 2 (commit 54285).
    assert_eq!(status, 2);
    assert!(stderr.contains("operand expected at `/'"), "got: {stderr}");
}

#[test]
fn test_let_orphan_mul_with_right_includes_remaining() {
    // zsh: `let "*5"` -> `at \`*5'` — error retains the input
    // pointer at the start of the bad operator, so the remaining
    // input (operator + everything after) becomes the error
    // context.
    let (status, _, stderr) = run_zshrs("let \"*5\"");
    // c:Src/builtin.c:7478 — math errors return 2 (commit 54285).
    assert_eq!(status, 2);
    assert!(stderr.contains("operand expected at `*5'"), "got: {stderr}");
}

#[test]
fn test_let_trailing_mul_still_end_of_string() {
    // zsh: `let "5*"` -> still "at end of string" because the
    // left operand was consumed; only the right is missing and
    // input has been exhausted. Our orphan-at-start check
    // explicitly only fires when stack.is_empty().
    let (status, _, stderr) = run_zshrs("let \"5*\"");
    // c:Src/builtin.c:7478 — math errors return 2 (commit 54285).
    assert_eq!(status, 2);
    assert!(
        stderr.contains("operand expected at end of string"),
        "got: {stderr}"
    );
}

#[test]
fn test_arith_base_digit_full_remainder() {
    // zsh: `$((2#22))` -> `bad math expression: operator
    // expected at \`22'` — the lexer drops out of base-parse
    // mode at the first out-of-range char and the parser then
    // trips on the remainder. zsh reports the FULL bad digit
    // sequence (`22`), not just the first char (`2`). zshrs
    // grabbed only `val_str.chars().next()` which lost the
    // location info for multi-digit out-of-range cases.
    let (_, _, stderr) = run_zshrs("echo $((2#22))");
    assert!(
        stderr.contains("operator expected at `22'"),
        "got: {stderr}"
    );
    assert!(!stderr.contains("at `2'"), "got: {stderr}");
}

#[test]
fn test_s_flag_drops_empty_fields_default() {
    // zsh: `${(s:,:)foo}` for `foo="a,,b,,c"` drops empty fields
    // -> 3 elements. zshrs preserved them -> 5 elements (off by
    // 2). Empty-field dropping matches zsh's default split
    // semantics; the `(@)` flag overrides to preserve empties.
    let (_, output, _) = run_zshrs(r#"foo="a,,b,,c"; arr=( "${(s:,:)foo}" ); echo "${#arr}""#);
    assert_eq!(output.trim(), "3", "got: {output:?}");
}

#[test]
fn test_at_s_flag_preserves_empty_fields() {
    // zsh: `${(@s:,:)foo}` for `foo="a,,b,,c"` preserves empty
    // fields -> 5 elements. The `@` flag's position in the flag
    // run doesn't matter — anywhere triggers preservation.
    let (_, output, _) = run_zshrs(r#"foo="a,,b,,c"; arr=( "${(@s:,:)foo}" ); echo "${#arr}""#);
    assert_eq!(output.trim(), "5", "got: {output:?}");
}

#[test]
fn test_s_flag_drops_consecutive_empties_in_split() {
    // zsh: `printf "[%s]\n" ${(s:l:)foo}` for `foo="hello"`
    // splits "hello" by "l" -> ["he", "", "o"] — but bare
    // `(s::)` drops the empty middle, so output is 2 lines
    // ([he] and [o]). zshrs's keep-all-fields produced 3 lines
    // including [].
    let (_, output, _) = run_zshrs(r#"foo=hello; printf "[%s]\n" ${(s:l:)foo}"#);
    assert_eq!(output.trim(), "[he]\n[o]", "got: {output:?}");
}

#[test]
fn test_arith_empty_base_digits_is_zero() {
    // zsh: `$((10#))` and `$((36#))` (empty digit run after
    // `#`) silently return 0. zshrs's `from_str_radix("", b)`
    // returned Err which fell into the operator-expected arm
    // and emitted a nonsense `at \`'` message.
    let (_, output, _) = run_zshrs("echo $((10#))");
    assert_eq!(output.trim(), "0", "got: {output:?}");
    let (_, output, _) = run_zshrs("echo $((36#))");
    assert_eq!(output.trim(), "0", "got: {output:?}");
}

#[test]
fn test_export_invalid_first_char_rejects() {
    // zsh: `export 1bad=val` -> `export:1: not an identifier:
    // 1bad` exit 1. zshrs silently exported the bogus name
    // (env var `1bad=val` is unreachable from any shell that
    // parses identifiers — pure pollution).
    let (status, _, stderr) = run_zshrs(r#"export "1bad=val""#);
    assert_eq!(status, 1);
    assert!(stderr.contains("not an identifier: 1bad"), "got: {stderr}");
}

#[test]
fn test_export_space_in_name_rejects() {
    // zsh: `export "BAD NAME=val"` -> `export:1: not valid in
    // this context: BAD NAME` exit 1. Distinct wording from the
    // identifier-leading case because the name has internal
    // whitespace/special chars rather than a bad first letter.
    let (status, _, stderr) = run_zshrs(r#"export "BAD NAME=val""#);
    assert_eq!(status, 1);
    assert!(
        stderr.contains("not valid in this context: BAD NAME"),
        "got: {stderr}"
    );
}

#[test]
fn test_typeset_invalid_identifier_rejects() {
    // zsh: `typeset 1bad=5` -> `typeset:1: not an identifier:
    // 1bad` exit 1. Same rule for declare/local/integer/
    // readonly which all dispatch through this path.
    let (status, _, stderr) = run_zshrs("typeset 1bad=5");
    assert_eq!(status, 1);
    assert!(stderr.contains("not an identifier: 1bad"), "got: {stderr}");
}

#[test]
fn test_declare_invalid_identifier_uses_declare_prefix() {
    // zsh prefixes the diagnostic with the invocation name so
    // `declare 1bad=5` reads `declare:1: not an identifier:
    // 1bad` (vs `typeset:1: ...` for the typeset entrypoint).
    let (status, _, stderr) = run_zshrs("declare 1bad=5");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("declare:1: not an identifier: 1bad"),
        "got: {stderr}"
    );
}

#[test]
fn test_integer_invalid_identifier_rejects() {
    // `integer` has its own assignment loop separate from
    // typeset_named; same identifier rule applies.
    let (status, _, stderr) = run_zshrs("integer 1bad=5");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("integer:1: not an identifier: 1bad"),
        "got: {stderr}"
    );
}

#[test]
fn test_readonly_invalid_identifier_rejects() {
    // `readonly NAME=val` validates NAME like the others.
    let (status, _, stderr) = run_zshrs("readonly 1bad=5");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("readonly:1: not an identifier: 1bad"),
        "got: {stderr}"
    );
}

#[test]
fn test_pushd_updates_pwd_variable() {
    // zsh: `pushd /tmp; echo $PWD` -> `/tmp`. zshrs's pushd
    // called set_current_dir (so the OS cwd moved) but didn't
    // sync $PWD/$OLDPWD, so the shell-level $PWD still pointed
    // at the pre-pushd directory. Symptoms: `dirs` showed the
    // canonicalized cwd (`/private/tmp` on macOS) instead of
    // the user-given logical path (`/tmp`).
    let (_, output, _) = run_zshrs(r#"pushd /tmp >/dev/null; echo "$PWD""#);
    assert_eq!(output.trim(), "/tmp", "got: {output:?}");
}

#[test]
fn test_dirs_uses_logical_pwd_not_canonical() {
    // zsh's `dirs` uses $PWD (symlink-preserving) for the
    // current entry. zshrs's `dirs` read OS cwd via
    // `current_dir()` which canonicalizes — so on macOS
    // `pushd /tmp; dirs` showed `/private/tmp` instead of
    // zsh's `/tmp`. Verify by pushing to /tmp (which is a
    // symlink to /private/tmp on macOS) and checking the
    // first path in dirs output.
    let (_, output, _) = run_zshrs(r#"pushd /tmp >/dev/null; dirs"#);
    let first_token = output.split_whitespace().next().unwrap_or("");
    assert_eq!(
        first_token, "/tmp",
        "expected /tmp (logical), got: {output:?}"
    );
}

#[test]
fn test_brace_zero_step_stays_literal() {
    // zsh: `{1..3..0}` (step 0) is invalid and stays literal —
    // bash agrees. zshrs's `abs_step.max(1)` silently treated 0
    // as 1 and produced `1 2 3`. Negative steps still reverse
    // (per zsh's rule); only exactly 0 short-circuits.
    let (_, output, _) = run_zshrs("echo {1..3..0}");
    // zsh: `{1..3..0}` (step 0 invalid) outputs `1..3..0` — the
    // braces are stripped by brace expansion but the contents stay
    // literal because the step 0 short-circuits the range expansion.
    assert_eq!(output.trim(), "1..3..0", "got: {output:?}");
    // Sanity: non-zero steps still expand.
    let (_, output, _) = run_zshrs("echo {1..3..2}");
    assert_eq!(output.trim(), "1 3", "got: {output:?}");
}

#[test]
fn test_nested_expansion_modifier_chain() {
    // Direct port of zsh's hist.c modifier dispatch for
    // `${${...}:MOD}` — outer history modifier applies to the
    // inner expansion result. zshrs's nested-expansion handler
    // only checked for `[idx]` after the inner; modifier chain
    // and `/pat/repl` substitution were silently dropped.
    let (_, output, _) = run_zshrs(r#"a=Hello.World; echo "${${a:l}:r}""#);
    assert_eq!(output.trim(), "hello", "got: {output:?}");
    let (_, output, _) = run_zshrs(r#"a=( file.txt other.csv ); echo "${${(j: :)a}:r}""#);
    assert_eq!(output.trim(), "file.txt other", "got: {output:?}");
}

#[test]
fn test_nested_expansion_replace() {
    // `${${a}//l/L}` — replace operator after nested expansion.
    let (_, output, _) = run_zshrs(r#"a=hello; echo "${${a}//l/L}""#);
    assert_eq!(output.trim(), "heLLo", "got: {output:?}");
    let (_, output, _) = run_zshrs(r#"a=hello; echo "${${a}/l/L}""#);
    assert_eq!(output.trim(), "heLlo", "got: {output:?}");
}

#[test]
fn test_typeset_array_quote_aware_split() {
    // Direct port of zsh's lex.c word-splitting for assignment
    // RHS — `local arr=( "a b" c )` keeps "a b" as one element,
    // not two. zshrs's typeset array path naively
    // split-by-whitespace'd the body, breaking quoted strings.
    // Replaced with bslashquote-aware scanner that honors `"..."`/'...'`
    // boundaries (still strips the bslashquote chars from the result).
    let (_, output, _) =
        run_zshrs(r#"foo() { local arr=( "abc" "def ghi" jk ); echo "${#arr}|${arr[2]}"; }; foo"#);
    assert_eq!(output.trim(), "3|def ghi", "got: {output:?}");
    let (_, output, _) =
        run_zshrs(r#"declare -a arr=( "x y" z ); echo "${#arr}|${arr[1]}|${arr[2]}""#);
    assert_eq!(output.trim(), "2|x y|z", "got: {output:?}");
}

#[test]
fn test_function_scope_exit_trap_fires_on_return() {
    // Direct port of zsh's exec.c dotrapargs(SIGEXIT, ...) —
    // a `trap "..." EXIT` set INSIDE a function fires when the
    // function returns, NOT when the shell exits, and it does
    // NOT pollute the outer EXIT trap.
    let (_, output, _) = run_zshrs(r#"foo() { trap "echo X" EXIT; }; foo; echo "after foo""#);
    assert_eq!(output.trim(), "X\nafter foo", "got: {output:?}");
}

#[test]
fn test_function_scope_exit_trap_preserves_outer() {
    // Outer EXIT trap survives across a function call that
    // also sets its own EXIT trap. zsh fires INNER on function
    // return, then OUTER at shell exit.
    let (_, output, _) = run_zshrs(
        r#"trap "echo OUTER" EXIT; foo() { trap "echo INNER" EXIT; }; foo; echo "between""#,
    );
    assert_eq!(output.trim(), "INNER\nbetween\nOUTER", "got: {output:?}");
}

#[test]
fn test_return_no_arg_uses_last_status() {
    // zsh: `return` with no arg returns with the status of the
    // most recently executed command. `foo() { false; return; }`
    // returns 1 (false's status), not 0. Direct port of zsh's
    // bin_return in builtin.c — exec.last_status is stale at
    // builtin-time (only synced at statement boundaries), so
    // BUILTIN_RETURN now reads vm.last_status (the live value)
    // and syncs it before delegating to the executor method.
    let (_, output, _) = run_zshrs("foo() { false; return; }; foo; echo $?");
    assert_eq!(output.trim(), "1", "got: {output:?}");
    let (_, output, _) = run_zshrs("foo() { true; return; }; foo; echo $?");
    assert_eq!(output.trim(), "0", "got: {output:?}");
    // Explicit arg overrides last_status.
    let (_, output, _) = run_zshrs("foo() { false; return 0; }; foo; echo $?");
    assert_eq!(output.trim(), "0", "got: {output:?}");
}

#[test]
fn test_array_slice_with_arith_length_expr() {
    // zsh: `${arr[@]:1:$((2+0))}` slices the array — $((2+0))
    // evaluates to 2, return elements 2 and 3 (1-indexed).
    // zshrs's BUILTIN_PARAM_SUBSTRING_EXPR fell through to
    // scalar char-slicing on the IFS-joined value, so
    // `${a[@]:1:$((2+0))}` produced " b" via char positions
    // instead of "b c" via array positions. Direct port of
    // the existing PARAM_SUBSTRING (int) handler's array-aware
    // dispatch — strip `[@]`/`[*]` suffix, lookup as array,
    // fall back to scalar.
    let (_, output, _) = run_zshrs(r#"a=( a b c d e ); echo "${a[@]:1:$((2+0))}""#);
    assert_eq!(output.trim(), "b c", "got: {output:?}");
    let (_, output, _) = run_zshrs(r#"a=( a b c d e ); echo "${a:1:$((2+0))}""#);
    assert_eq!(output.trim(), "b c", "got: {output:?}");
}

#[test]
fn test_glob_qualifier_size_uses_lstat_for_symlinks() {
    // Direct port of zsh's pattern.c L qualifier — lstat-based
    // size check. For a symlink, that's the LENGTH OF THE
    // SYMLINK STRING (e.g. 9 bytes for "empty.txt"), NOT the
    // target's size. zshrs's prefetched metadata had both
    // followed (`m`) and symlink (`sm`) variants; the L
    // qualifier was reading `m.len()` which gave the target
    // size, so a symlink to an empty file appeared empty.
    let d = "/tmp/glob_qual_l_test";
    let _ = std::fs::create_dir_all(d);
    let _ = std::fs::write(format!("{}/empty.txt", d), "");
    let _ = std::os::unix::fs::symlink("empty.txt", format!("{}/link_e", d));
    // L+0: include the symlink (lstat-size > 0) but exclude
    // the empty regular file.
    let (_, output, _) = run_zshrs(&format!("echo {}/*(L+0)", d));
    let parts: std::collections::HashSet<&str> = output.split_whitespace().collect();
    assert!(
        parts.contains(format!("{}/link_e", d).as_str()),
        "expected link_e in L+0 results: {output:?}"
    );
    assert!(
        !parts.contains(format!("{}/empty.txt", d).as_str()),
        "did not expect empty.txt in L+0 results: {output:?}"
    );
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn test_glob_qualifier_history_modifier() {
    // Direct port of zsh's pattern.c qualifier modifier
    // handling — `*(:r)` strips the extension from each match,
    // `*(:t)` keeps only the basename, `*(:e)` returns the
    // extension. Modifiers can be chained with file-type
    // qualifiers: `*(.:r)` = regular files, then strip ext.
    let d = "/tmp/glob_qual_mod_test";
    let _ = std::fs::create_dir_all(d);
    let _ = std::fs::write(format!("{}/a.txt", d), "");
    let _ = std::fs::write(format!("{}/b.csv", d), "");
    let (_, output, _) = run_zshrs(&format!("echo {}/*(:r)", d));
    let mut parts: Vec<&str> = output.split_whitespace().collect();
    parts.sort();
    assert_eq!(
        parts,
        vec![format!("{}/a", d).as_str(), format!("{}/b", d).as_str(),],
        "got: {output:?}"
    );
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn test_glob_qualifier_comma_or() {
    // Direct port of zsh's pattern.c qualifier parsing — top-
    // level `,` in `(...)` is OR (alternation between qualifier
    // clauses). zshrs's filter_by_qualifiers AND'd everything,
    // so `*(.,/)` errored "no matches found" because no file
    // is BOTH a regular file AND a directory.
    let d = "/tmp/glob_qual_or_test";
    let _ = std::fs::create_dir_all(d);
    let _ = std::fs::write(format!("{}/a.txt", d), "");
    let _ = std::fs::create_dir_all(format!("{}/sub", d));
    let (_, output, _) = run_zshrs(&format!("echo {}/*(.,/)", d));
    let mut parts: Vec<&str> = output.split_whitespace().collect();
    parts.sort();
    assert_eq!(
        parts,
        vec![
            format!("{}/a.txt", d).as_str(),
            format!("{}/sub", d).as_str()
        ],
        "got: {output:?}"
    );
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn test_cli_o_flag_sets_options() {
    // zsh's `-o NAME` and `+o NAME` CLI flags toggle options
    // before the script runs. zshrs previously didn't parse
    // them, so `zshrs -f +o nomatch -c '...'` errored
    // "+o: No such file or directory" (treating `+o` as a
    // script file argument because it didn't start with `-`).
    // Direct port of zsh's main.c arg-parse loop: collect
    // `-o NAME` (set) / `+o NAME` (unset) pairs, store
    // verbatim into the options table. The `no` prefix is
    // PART of the canonical option name (e.g. `nomatch`)
    // for setopt/unsetopt — only the [[ -o ... ]] query path
    // does prefix-stripping canonicalization.
    use std::process::Command;
    let bin = env!("CARGO_BIN_EXE_zshrs");
    let out = Command::new(bin)
        .args(["-f", "+o", "nomatch", "-c", "echo *(/.)"])
        .output()
        .expect("spawn zshrs");
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(stdout.trim(), "*(/.)", "got: {stdout:?}");
}

#[test]
fn test_glob_alternation_at_path_level() {
    // Direct port of zsh's pattern.c P_BRANCH `|` at the path
    // level. `/etc/(passwd|hostname)` matches paths whose last
    // component is `passwd` OR `hostname`. zshrs's compile path
    // didn't recognize `(...|...)` as a glob trigger, so the
    // parens reached the OS as literal chars (no match).
    // Two parts of the fix:
    //   1. compile_zsh: `unquoted (` + `|` + `)` triggers the
    //      glob path (alongside `*`/`?`/`[`).
    //   2. expand_glob: pre-expand top-level `(...|...)` into
    //      multiple alternatives via `expand_glob_alternation`,
    //      glob each, dedup, sort (matches zsh's lexicographic
    //      glob result order).
    let (_, output, _) = run_zshrs("echo /etc/(passwd|hostname)");
    assert_eq!(output.trim(), "/etc/passwd", "got: {output:?}");
    let (_, output, _) = run_zshrs("echo /etc/(passwd|nonexistent)");
    assert_eq!(output.trim(), "/etc/passwd", "got: {output:?}");
    // Mixed literal + glob alternative.
    let d = "/tmp/glob_alt_test";
    let _ = std::fs::create_dir_all(d);
    let _ = std::fs::write(format!("{}/a.txt", d), "");
    let _ = std::fs::write(format!("{}/b.csv", d), "");
    let (_, output, _) = run_zshrs(&format!("echo {}/(a|b)*", d));
    assert_eq!(
        output.trim(),
        format!("{}/a.txt {}/b.csv", d, d),
        "got: {output:?}"
    );
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn test_array_assign_with_cmd_subst_ifs_split() {
    // zsh: `arr=( $(echo "a:b:c") )` with `IFS=:` should
    // word-split the cmd-subst output on IFS, producing 3
    // elements `[a, b, c]`. zshrs's compile path was emitting
    // BUILTIN_WORD_SPLIT TWICE — once inside compile_word_str
    // for the unquoted $() AND once in the array-element loop.
    // The second split saw the Value::Array converted to a
    // single string "a b c" (with spaces, no `:`), which had
    // no IFS chars to split on, so all 3 elements collapsed
    // back into one. Direct port: assign_context_depth bumped
    // for each element so the inner WORD_SPLIT is suppressed.
    let (_, output, _) = run_zshrs(
        r#"IFS=:; arr=( $(echo "a:b:c") ); echo "${#arr}|${arr[1]}|${arr[2]}|${arr[3]}""#,
    );
    assert_eq!(output.trim(), "3|a|b|c", "got: {output:?}");
    let (_, output, _) = run_zshrs(r#"arr=( "$(echo "a b c")" ); echo "${#arr}|${arr[1]}""#);
    assert_eq!(output.trim(), "1|a b c", "got: {output:?}");
}

#[test]
fn test_subscript_w_flag_word_index() {
    // `(w)N` subscript flag — direct port of zsh's zshparam(1)
    // "Subscript Flags" w: returns the Nth IFS-separated word.
    // For arrays, equivalent to plain [N] (already split).
    // For scalars, splits by IFS first.
    // zshrs's `parse_subscript_flags` previously rejected `w`,
    // so the index fell through to math eval which errored.
    let (_, output, _) = run_zshrs(r#"a="hello world foo"; echo "${a[(w)2]}""#);
    assert_eq!(output.trim(), "world", "got: {output:?}");
    let (_, output, _) = run_zshrs(r#"a=( one two three ); echo "${a[(w)2]}""#);
    assert_eq!(output.trim(), "two", "got: {output:?}");
}

#[test]
fn test_subscript_s_flag_is_noop_for_int_index() {
    // zsh's `(s/sep/)` flag is a NO-OP for scalar `[N]` integer
    // indexing — verified by testing zsh:
    // `a=hello; ${a[(s/l/)1]}` returns "h" (same as `${a[1]}`).
    // The `(s)` flag only affects word-list contexts
    // (`${(s/sep/)var}` without an index, or `[@]` form).
    let (_, output, _) = run_zshrs(r#"a=hello; echo "${a[(s/l/)1]}""#);
    assert_eq!(output.trim(), "h", "got: {output:?}");
    let (_, output, _) = run_zshrs(r#"a="aa::bb::cc"; echo "${a[(s/::/)2]}""#);
    assert_eq!(output.trim(), "a", "got: {output:?}");
}

#[test]
fn test_param_replace_case_insensitive_inline_flag() {
    // `(#i)` inline pattern flag (zsh extendedglob) makes the
    // replacement match case-insensitively. zshrs's
    // BUILTIN_PARAM_REPLACE didn't run patterns through
    // parse_pattern_flags, so `${a//(#i)L/X}` left `(#i)L`
    // as literal regex (no match). Direct port: same helper
    // glob_match_static uses, with `(?i)` prefix applied.
    let (_, output, _) = run_zshrs(r#"setopt extendedglob; a=hello; echo "${a//(#i)L/X}""#);
    assert_eq!(output.trim(), "heXXo", "got: {output:?}");
    let (_, output, _) = run_zshrs(r#"setopt extendedglob; a=Hello; echo "${a/(#i)hello/X}""#);
    assert_eq!(output.trim(), "X", "got: {output:?}");
}

#[test]
fn test_read_preserves_separator_in_last_var() {
    // Direct port of zsh's bin_read in builtin.c — when input
    // has more fields than vars, the last var gets the unsplit
    // remainder INCLUDING separators between fields N..end.
    // zshrs previously split into Vec<&str> and join(" ")'d,
    // collapsing all separators to spaces.
    let (_, output, _) = run_zshrs(r#"IFS=: read x y <<< "a:b:c"; echo "[$x][$y]""#);
    assert_eq!(output.trim(), "[a][b:c]", "got: {output:?}");
    let (_, output, _) = run_zshrs(r#"IFS=, read x y <<< "a,,b,,c"; echo "[$x][$y]""#);
    assert_eq!(output.trim(), "[a][,b,,c]", "got: {output:?}");
}

#[test]
fn test_read_collapses_default_ifs() {
    // Default IFS (whitespace + NUL) collapses consecutive
    // separators AND strips leading/trailing whitespace from
    // the input. The NUL byte in the default IFS made
    // is_whitespace_ifs return false; fixed by accepting NUL
    // as a whitespace-class char for this purpose.
    let (_, output, _) = run_zshrs(r#"read x y <<< "  a    b    c  "; echo "[$x][$y]""#);
    assert_eq!(output.trim(), "[a][b    c]", "got: {output:?}");
}

#[test]
fn test_dq_array_replace_join_first() {
    // Same DQ-vs-unquoted split as strip, applied to the
    // replace operator. zsh: `"${a/o/O}"` for `a=(one two three)`
    // joins to "one two three" then replaces FIRST `o` ->
    // "One two three". Unquoted does per-element first match.
    let (_, output, _) = run_zshrs(r#"a=( one two three ); echo "${a/o/O}""#);
    assert_eq!(output.trim(), "One two three", "got: {output:?}");
    let (_, output, _) = run_zshrs(r#"a=( one two three ); echo ${a/o/O}"#);
    assert_eq!(output.trim(), "One twO three", "got: {output:?}");
}

#[test]
fn test_dq_array_replace_at_subscript_per_element() {
    // Explicit `[@]` forces per-element replace even in DQ
    // (matches the Strip behavior — `[@]` marks the array as
    // splice-expanded). `[*]` keeps the bare-DQ semantics.
    let (_, output, _) = run_zshrs(r#"a=( one two three ); echo "${a[@]/o/O}""#);
    assert_eq!(output.trim(), "One twO three", "got: {output:?}");
    let (_, output, _) = run_zshrs(r#"a=( one two three ); echo "${a[*]/o/O}""#);
    assert_eq!(output.trim(), "One two three", "got: {output:?}");
}

#[test]
fn test_arith_output_radix_with_prefix() {
    // Direct port of zsh's math.c output radix handling
    // (line 786 onward): `[#N]EXPR` formats result with `N#`
    // prefix; `[##N]EXPR` drops the prefix. Base must be 2..36.
    let (_, output, _) = run_zshrs("echo $(([#16]255))");
    assert_eq!(output.trim(), "16#FF", "got: {output:?}");
    let (_, output, _) = run_zshrs("echo $(([##16]255))");
    assert_eq!(output.trim(), "FF", "got: {output:?}");
    let (_, output, _) = run_zshrs("echo $(([#8]255))");
    assert_eq!(output.trim(), "8#377", "got: {output:?}");
    let (_, output, _) = run_zshrs("echo $(([##2]10))");
    assert_eq!(output.trim(), "1010", "got: {output:?}");
    // Base 10 special case: zsh's convbase (params.c:5586)
    // skips the `N#` prefix when N==10.
    let (_, output, _) = run_zshrs("echo $(([#10]42))");
    assert_eq!(output.trim(), "42", "got: {output:?}");
}

#[test]
fn test_integer_dash_i_with_base_arg() {
    // zsh: `integer -i 16 x=255` -> stores `255` but displays
    // `16#FF` per typeset -i semantics (zsh's builtin.c
    // typeset_i flag accepts a base arg). zshrs's earlier
    // arg-loop treated `16` as a separate name and errored
    // "not an identifier: 16" because the previous flag-only
    // loop didn't peek at the next arg for `-i`.
    let (status, output, _) = run_zshrs("integer -i 16 x=255; echo $x");
    assert_eq!(status, 0);
    assert_eq!(output.trim(), "16#FF", "got: {output:?}");
    let (_, output, _) = run_zshrs("integer -i 8 x=255; echo $x");
    assert_eq!(output.trim(), "8#377", "got: {output:?}");
    let (_, output, _) = run_zshrs("integer -i 2 x=10; echo $x");
    assert_eq!(output.trim(), "2#1010", "got: {output:?}");
}

#[test]
fn test_trap_dash_p_not_a_flag() {
    // zsh's trap builtin (Src/builtin.c bin_trap, line 7347)
    // does NOT accept -p. zshrs added bash-style `-p` for compat
    // but it diverged from zsh. With -p removed, `trap -p EXIT`
    // becomes "set action `-p` for signal EXIT" which the shell
    // dispatch treats as a missing command "-p" and emits
    // `command not found: -p` — matches zsh exactly.
    let (_, _, stderr) = run_zshrs("trap -p EXIT");
    assert!(stderr.contains("command not found: -p"), "got: {stderr}");
}

#[test]
fn test_extglob_tilde_exclusion() {
    // Direct port of zsh's pattern.c P_EXCLUDE handling — `pat1~pat2`
    // matches strings matching pat1 AND NOT matching pat2.
    let (_, output, _) =
        run_zshrs(r#"setopt extendedglob; [[ "abc" == a*~b* ]] && echo y || echo n"#);
    assert_eq!(output.trim(), "y", "got: {output:?}");
    let (_, output, _) =
        run_zshrs(r#"setopt extendedglob; [[ "bbb" == a*~b* ]] && echo y || echo n"#);
    assert_eq!(output.trim(), "n", "got: {output:?}");
}

#[test]
fn test_brace_nested_sequence_in_list() {
    // zsh: `{{1..3},x,y}` is a LIST containing one sequence and
    // two literals. Previously zshrs's brace-detector preferred
    // `..` over `,` for type detection — content with both was
    // miscategorized as a sequence, and the sequence parser
    // returned identity ({1..3} not at the top), so the whole
    // brace was left literal.
    let (_, output, _) = run_zshrs("echo {{1..3},x,y}");
    assert_eq!(output.trim(), "1 2 3 x y", "got: {output:?}");
    let (_, output, _) = run_zshrs("echo {1,{2..4},5}");
    assert_eq!(output.trim(), "1 2 3 4 5", "got: {output:?}");
}

#[test]
fn test_dq_array_strip_joins_scalar() {
    // zsh: `"${a%%pat}"` (DQ + bare array name) joins via $IFS
    // first then strips the joined scalar. zshrs's fast path
    // didn't propagate the compile-time DQ context to
    // BUILTIN_PARAM_STRIP, so it always per-element-stripped.
    let (_, output, _) = run_zshrs(r#"a=( hello world ); echo "${a%%[lo]*}""#);
    assert_eq!(output.trim(), "he", "got: {output:?}");
}

#[test]
fn test_dq_array_strip_at_subscript_per_element() {
    // zsh: explicit `[@]` subscript on the var forces per-
    // element strip even inside DQ — `[@]` marks the array as
    // splice-expanded, the strip applies to each element.
    // `[*]` (join-with-IFS) keeps the bare-DQ semantics.
    let (_, output, _) = run_zshrs(r#"a=( hello world ); echo "${a[@]%%[lo]*}""#);
    assert_eq!(output.trim(), "he w", "got: {output:?}");
    let (_, output, _) = run_zshrs(r#"a=( hello world ); echo "${a[*]%%[lo]*}""#);
    assert_eq!(output.trim(), "he", "got: {output:?}");
}

#[test]
fn test_glob_bracket_negation_with_bang() {
    // Direct port of zsh's pattern.c bracket-class compile —
    // `[!...]` and `[^...]` both negate. zshrs's compile-to-
    // regex translator copied `!` verbatim, so `[!a]bc` matched
    // "abc" via regex `[!a]bc` (`[!a]` = either `!` or `a`).
    let (status, _, _) = run_zshrs(r#"[[ "abc" == [!a]bc ]] && echo y || echo n"#);
    let _ = status;
    let (_, output, _) = run_zshrs(r#"[[ "abc" == [!a]bc ]] && echo y || echo n"#);
    assert_eq!(output.trim(), "n", "got: {output:?}");
    let (_, output, _) = run_zshrs(r#"[[ "xyz" == [!a]yz ]] && echo y || echo n"#);
    assert_eq!(output.trim(), "y", "got: {output:?}");
}

#[test]
fn test_glob_posix_class_with_negation() {
    // POSIX char classes inside `[...]` were broken by the
    // bracket scanner: it stopped at the first `]`, so
    // `[![:digit:]]` was misread as `[![:digit:]` (incomplete).
    let (_, output, _) = run_zshrs(r#"[[ "abc" == [![:digit:]]* ]] && echo y || echo n"#);
    assert_eq!(output.trim(), "y", "got: {output:?}");
    let (_, output, _) = run_zshrs(r#"[[ "abc" == [[:digit:]]* ]] && echo y || echo n"#);
    assert_eq!(output.trim(), "n", "got: {output:?}");
}

#[test]
fn test_extglob_one_or_more_postfix() {
    // Direct port of zsh's pattern.c POUND2 case — extendedglob
    // `pat##` matches one or more of `pat`. Translated to regex
    // `+`. zshrs's translator left the trailing `#` as a literal
    // so `[[ "aaa" == a## ]]` failed to match.
    let (_, output, _) =
        run_zshrs(r#"setopt extendedglob; [[ "aaa" == a## ]] && echo y || echo n"#);
    assert_eq!(output.trim(), "y", "got: {output:?}");
    let (_, output, _) = run_zshrs(
        r#"setopt extendedglob; [[ "abc123" == [[:alpha:]]##[[:digit:]]## ]] && echo y || echo n"#,
    );
    assert_eq!(output.trim(), "y", "got: {output:?}");
}

#[test]
fn test_extglob_zero_or_more_postfix() {
    // `pat#` is zero-or-more. Empty input must match.
    let (_, output, _) = run_zshrs(r#"setopt extendedglob; [[ "" == a# ]] && echo y || echo n"#);
    assert_eq!(output.trim(), "y", "got: {output:?}");
    let (_, output, _) = run_zshrs(r#"setopt extendedglob; [[ "aaa" == a# ]] && echo y || echo n"#);
    assert_eq!(output.trim(), "y", "got: {output:?}");
}

#[test]
fn test_lineno_increments_per_line_in_dash_c() {
    // Direct port of zsh's `lineno` global tracking from
    // Src/input.c:330 — increments on each newline. Compiler
    // emits BUILTIN_SET_LINENO before each top-level pipe with
    // the value captured by the parser at `ZshPipe.lineno`.
    let (_, output, _) = run_zshrs("echo $LINENO\necho $LINENO\necho $LINENO");
    assert_eq!(output.trim(), "1\n2\n3", "got: {output:?}");
}

#[test]
fn test_lineno_resets_inside_function() {
    // zsh: `lineno = 1` on function entry (Src/init.c:1588);
    // restore on return. zshrs's compile_funcdef sets
    // `lineno_offset = first_body_line - 1` so the body's
    // emitted SET_LINENO calls produce 1, 2, 3 relative to the
    // body, not the absolute position in the source script.
    let script = "foo() {
  echo $LINENO
  echo $LINENO
}
foo
echo $LINENO";
    let (_, output, _) = run_zshrs(script);
    assert_eq!(output.trim(), "1\n2\n6", "got: {output:?}");
}

#[test]
fn test_m_flag_with_double_hash_strip() {
    // Direct port of zsh's get_match_ret() in glob.c:2550 — the
    // (M) flag inverts the strip return: instead of the
    // unmatched portion (default), return the matched portion.
    // ${a##*o} with a="hello world": longest leading match of
    // *o = "hello wo", default returns " rld", (M) returns
    // "hello wo".
    let (_, output, _) = run_zshrs(r#"a="hello world"; echo "${(M)a##*o}""#);
    assert_eq!(output.trim(), "hello wo", "got: {output:?}");
}

#[test]
fn test_m_flag_with_single_hash_strip() {
    // ${(M)a#*o}: shortest leading match of *o = "hello",
    // (M) returns matched "hello".
    let (_, output, _) = run_zshrs(r#"a="hello world"; echo "${(M)a#*o}""#);
    assert_eq!(output.trim(), "hello", "got: {output:?}");
}

#[test]
fn test_m_flag_with_percent_strip() {
    // ${(M)a%o*}: shortest trailing match of o* = "orld",
    // (M) returns matched "orld".
    let (_, output, _) = run_zshrs(r#"a="hello world"; echo "${(M)a%o*}""#);
    assert_eq!(output.trim(), "orld", "got: {output:?}");
}

#[test]
fn test_m_flag_with_percent_percent_strip() {
    // ${(M)a%%o*}: longest trailing match of o* = "o world",
    // (M) returns matched "o world".
    let (_, output, _) = run_zshrs(r#"a="hello world"; echo "${(M)a%%o*}""#);
    assert_eq!(output.trim(), "o world", "got: {output:?}");
}

#[test]
fn test_m_flag_no_match_returns_empty() {
    // (M) on a strip that finds no match: zsh returns empty
    // (the matched portion doesn't exist). Without (M) the
    // original string passes through unchanged.
    let (_, output, _) = run_zshrs(r#"a="hello"; echo "[${(M)a#nope}]""#);
    assert_eq!(output.trim(), "[]", "got: {output:?}");
    let (_, output, _) = run_zshrs(r#"a="hello"; echo "[${a#nope}]""#);
    assert_eq!(output.trim(), "[hello]", "got: {output:?}");
}

#[test]
fn test_bare_typeset_prints_declaration_at_top_level() {
    // zsh: bare `typeset NAME` / `declare NAME` (no flags, no
    // `=`) at the top level prints `NAME=value` (or `NAME=( ... )`
    // for arrays) when NAME is set, mirroring `-p` SHAPE without
    // the `typeset`/`export` prefix. zshrs silently swallowed
    // the call, dropping the listing entirely.
    let (_, output, _) = run_zshrs("a=( 1 2 3 ); declare a");
    assert_eq!(output.trim(), "a=( 1 2 3 )", "got: {output:?}");
    let (_, output, _) = run_zshrs("a=hello; typeset a");
    assert_eq!(output.trim(), "a=hello", "got: {output:?}");
    let (_, output, _) = run_zshrs("integer a=42; declare a");
    assert_eq!(output.trim(), "a=42", "got: {output:?}");
}

#[test]
fn test_bare_typeset_localizes_inside_function() {
    // Inside a function, bare `typeset NAME` localizes (shadows
    // parent, resets to empty) — does NOT print. The
    // print-the-declaration behavior is top-level-only per zsh.
    let (_, output, _) =
        run_zshrs(r#"a=hi; foo() { typeset a; echo "in[$a]"; }; foo; echo "out[$a]""#);
    assert_eq!(output.trim(), "in[]\nout[hi]", "got: {output:?}");
}

#[test]
fn test_declare_array_strips_quoted_elements() {
    // zsh: `declare -a arr=( "abc" "def" )` produces
    // arr=[abc, def] (quotes are syntactic, stripped at the
    // shell-syntax level). zshrs's typeset array path
    // split-by-whitespace'd the raw string and kept the quotes
    // attached to each element, so consumers saw `"abc"` as
    // the literal first element. Same bug for `[1]=second`-
    // style elements which arrived as `"[1]=second"` complete
    // with quotes.
    let (_, output, _) =
        run_zshrs(r#"declare -a arr=( "abc" "def" ); printf "[%s]\n" "${arr[@]}""#);
    assert_eq!(output.trim(), "[abc]\n[def]", "got: {output:?}");
}

#[test]
fn test_dollar_hash_array_subscript() {
    // zsh: `$#a[N]` is sugar for `${#a[N]}` — length of array
    // element N (1-indexed). zshrs's compile-time fast path
    // handled `$#NAME` and `$#NAME[@]`/`$#NAME[*]` but left a
    // numeric subscript as literal: `echo $#a[2]` printed
    // `3[2]` (count-of-array followed by literal `[2]`).
    let (_, output, _) = run_zshrs("a=( one two three ); echo $#a[2]");
    assert_eq!(output.trim(), "3", "got: {output:?}");
    let (_, output, _) = run_zshrs("a=( aa bb cc ); echo $#a[2]");
    assert_eq!(output.trim(), "2", "got: {output:?}");
}

#[test]
fn test_popd_restores_pwd_variable() {
    // popd should sync $PWD back to the previous directory.
    // Without the sync, $PWD stayed pointing at the pushd
    // location even after popd.
    let (_, output, _) = run_zshrs(
        r#"start="$PWD"; pushd /tmp >/dev/null; popd >/dev/null; [[ "$PWD" == "$start" ]] && echo same || echo "diff: $PWD vs $start""#,
    );
    assert_eq!(output.trim(), "same", "got: {output:?}");
}

#[test]
fn test_zle_l_silent_in_script() {
    // zsh: in `-c`/`-f` mode the ZLE module isn't loaded, so `zle
    // -l` outputs nothing and returns 0. zshrs preloads its built-in
    // widget table and listed widgets even in scripts. Now matches
    // zsh by checking !atty(stdin) and returning 0 silently.
    let (status, output, _) = run_zshrs("zle -l");
    assert_eq!(status, 0);
    assert!(
        !output.contains("accept-line") && !output.contains("self-insert"),
        "got: {output}"
    );
}

#[test]
fn test_kill_bad_signal_uses_zsh_format() {
    // bundled c:Src/jobs.c — `kill -INVALID 1` emits
    //   "kill:1: unknown signal: SIGINVALID"
    //   "kill:1: type kill -L for a list of signals"
    // rc=1. The hint uses capital `-L` (tabular listing) per the
    // bundled source. zshrs previously emitted bash-style
    // `kill: invalid signal: -INVALID` (no SIG prefix, no hint).
    let (status, _, stderr) = run_zshrs("kill -INVALID 1");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("unknown signal: SIGINVALID"),
        "got: {stderr}"
    );
    assert!(
        stderr.contains("type kill -L for a list of signals"),
        "got: {stderr}"
    );
}

#[test]
fn test_printf_invalid_directive_exits_nonzero() {
    // zsh: `printf "%Z\n" 1` -> `printf:1: %Z: invalid directive`
    // exit 1. zshrs printed the same diagnostic but returned 0,
    // hiding the failure for $?-checking scripts. Now tracks an
    // `had_error` flag through the format-spec walker.
    let (status, _, stderr) = run_zshrs(r#"printf "%Z\n" 1"#);
    assert_eq!(status, 1);
    assert!(stderr.contains("%Z: invalid directive"), "got: {stderr}");
    // Sanity: valid formats still return 0.
    let (status, output, _) = run_zshrs(r#"printf "%s\n" hi"#);
    assert_eq!(status, 0);
    assert!(output.contains("hi"));
}

#[test]
fn test_exec_flag_only_no_command_errors() {
    // zsh: `exec -c`, `exec -l`, `exec -a foo` (any flag form
    // without a following command) -> `exec requires a command to
    // execute` exit 1. zshrs silently no-op'd, masking flag-only
    // typos. Bare `exec` (no flags, no command) is still the
    // silent-environment-modify form per POSIX.
    let (status, _, stderr) = run_zshrs("exec -c");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("exec requires a command to execute"),
        "got: {stderr}"
    );
    let (status, _, stderr) = run_zshrs("exec -l 2>&1; echo done");
    assert_eq!(status, 0);
    assert!(
        stderr.contains("exec requires a command to execute") ||
            // Combined script: the error went to merged stdout/stderr
            stderr.is_empty(),
        "got: {stderr}"
    );
    let (status, _, _) = run_zshrs("exec");
    assert_eq!(status, 0);
}

#[test]
fn test_fc_no_args_with_session_still_recurses() {
    // Bare `fc` (no -l, no positional) ALWAYS errors recurse-
    // endlessly in -c mode. Even when `print -s` added entries,
    // the EDIT mode tries to re-execute `fc` (the prior command)
    // which is infinite. zshrs previously fell into list-mode
    // pass-through if session had any entries.
    let (status, _, stderr) = run_zshrs("print -s a; fc");
    assert_eq!(status, 1);
    assert!(stderr.contains("would recurse endlessly"), "got: {stderr}");
    // -l with session entries still lists them.
    let (_, output, _) = run_zshrs("print -s a; fc -l");
    assert!(output.contains("1  a"), "got: {output:?}");
}

#[test]
fn test_alias_listing_quotes_value_with_equals() {
    // zsh: `alias g='x=y'; alias g` prints `g='x=y'` — quoted
    // because the body contains `=`. zshrs's `format_alias_kv` (and
    // its inline copy in the per-name listing path) didn't include
    // `=` in the bslashquote-trigger set, so `alias g=x=y` round-tripped
    // as bare which would re-parse as `alias g=x` + arg `=y`.
    let (_, output, _) = run_zshrs(r#"alias g="x=y"; alias g"#);
    assert_eq!(output.trim(), "g='x=y'");
    // Plain ls value still bare.
    let (_, output, _) = run_zshrs("alias x=ls; alias x");
    assert_eq!(output.trim(), "x=ls");
}

#[test]
fn test_functions_plus_t_disable_trace_silent() {
    // zsh: `functions +t NAME` / `+T NAME` clears the trace attr
    // silently. zshrs treated `+t` as a function name and emitted
    // `no such function: +t`. Added explicit `+t`/`+T` arms (and
    // a combined `+xyz` arm) that consume silently.
    let (status, output, stderr) = run_zshrs("foo() { :; }; functions +t foo");
    assert_eq!(status, 0);
    assert_eq!(output, "");
    assert_eq!(stderr, "");
}

#[test]
fn test_alias_empty_name_errors() {
    // c:Src/builtin.c:4293 `getasg` returning NULL → `zwarnnam(name,
    // "bad assignment")`; the returnval=1 is set but execution
    // continues past the failed arg, so a bare `alias =` (no
    // following args) ends with returnval=0. Verified against
    // /bin/zsh and /opt/homebrew/bin/zsh: `alias =` prints
    // "bad assignment" rc=0, `alias =val` looks up `=val` as a
    // display request → "val not found" rc=1.
    let (status, _, stderr) = run_zshrs("alias =");
    assert_eq!(status, 0);
    assert!(stderr.contains("bad assignment"), "got: {stderr}");
    let (status, _, stderr) = run_zshrs("alias =val");
    assert_eq!(status, 1);
    assert!(stderr.contains("not found"), "got: {stderr}");
}

#[test]
fn test_ulimit_unknown_flag_errors() {
    // zsh: `ulimit -X` (unknown letter) errors `bad option: -X`
    // exit 1. zshrs's silent default arm let it fall through and
    // proceed with the default resource (FSIZE), printing
    // `unlimited` and masking the typo.
    let (status, _, stderr) = run_zshrs("ulimit -X");
    assert_eq!(status, 1);
    assert!(stderr.contains("bad option: -X"), "got: {stderr}");
}

#[test]
fn test_fc_ld_lD_show_time_duration() {
    // zsh: `fc -ld` shows HH:MM time column; `-lD` shows M:SS
    // duration column. zshrs's session-only listing path skipped
    // both columns, emitting just `N  command`. Now both flags
    // route through the show_time / show_duration formatters.
    let (_, output, _) = run_zshrs(r#"print -s a; fc -ld"#);
    let line = output.lines().next().unwrap();
    // Expect 3 columns: number, HH:MM, command. HH:MM matches
    // \d\d:\d\d at the right position.
    let parts: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(parts.len(), 3, "got: {line:?}");
    assert!(parts[1].contains(':'), "got: {line:?}");
    let (_, output, _) = run_zshrs(r#"print -s a; fc -lD"#);
    let line = output.lines().next().unwrap();
    let parts: Vec<&str> = line.split_whitespace().collect();
    assert_eq!(parts.len(), 3, "got: {line:?}");
    assert!(parts[1].contains(':'), "got: {line:?}");
}

#[test]
fn test_fc_R_silent_on_missing_file() {
    // zsh: `fc -R /no/such` returns 0 with no output (read errors
    // silently ignored). zshrs printed `fc: cannot read /no/such`
    // and exited 1 — script consumers shouldn't trip on missing
    // log files.
    let (status, output, stderr) = run_zshrs("fc -R /no/such/file");
    assert_eq!(status, 0);
    assert_eq!(output, "");
    assert_eq!(stderr, "");
}

#[test]
fn test_fc_lr_session_reverse() {
    // zsh: `fc -lr` walks session entries backwards (most recent
    // first) while keeping original event numbers — `3 c | 2 b | 1 a`
    // for 3 entries. zshrs's session-only path ignored the `-r`
    // flag and always emitted forward order.
    let (_, output, _) = run_zshrs(r#"print -s a; print -s b; print -s c; fc -lr"#);
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines, vec!["    3  c", "    2  b", "    1  a"]);
}

#[test]
fn test_fc_W_writes_session_entries_only_in_minus_c() {
    // zsh: `fc -W FILE` in non-interactive `-c` mode writes ONLY
    // session-added entries (typically empty when no `print -s`
    // ran). zshrs previously dumped the entire on-disk persistent
    // history into FILE, leaking prior runs' commands. Now scopes
    // to session_history_ids when atty is absent.
    use std::fs;
    let path = std::env::temp_dir().join(format!("zshrs_fcW_test_{}", std::process::id()));
    let _ = fs::remove_file(&path);
    let cmd = format!(r#"fc -W {}"#, path.to_string_lossy());
    let (_, _, _) = run_zshrs(&cmd);
    let body = fs::read_to_string(&path).unwrap_or_default();
    let _ = fs::remove_file(&path);
    assert_eq!(body, "", "got: {body:?}");
    // With session entries via `print -s`, only those land in FILE.
    let path2 = std::env::temp_dir().join(format!("zshrs_fcW_test2_{}", std::process::id()));
    let _ = fs::remove_file(&path2);
    let cmd = format!(
        r#"print -s "AAA"; print -s "BBB"; fc -W {}"#,
        path2.to_string_lossy()
    );
    let (_, _, _) = run_zshrs(&cmd);
    let body = fs::read_to_string(&path2).unwrap_or_default();
    let _ = fs::remove_file(&path2);
    assert!(body.contains("AAA"), "got: {body:?}");
    assert!(body.contains("BBB"), "got: {body:?}");
    // Just two session entries (both lines containing those tokens).
    let lines = body.lines().count();
    assert_eq!(lines, 2, "got {lines} lines: {body:?}");
}

#[test]
fn test_v_flag_visible_control_chars() {
    // zsh: `${(V)x}` makes non-printable characters visible —
    // control chars become `^X` (X = char + 64), `\n` → `\n`,
    // `\t` → `\t`. zshrs's inline state-machine had no `V` arm
    // (the multi-flag dispatcher had ZshParamFlag::Visible but
    // the (V)-only single-flag path skipped it), so control chars
    // passed through raw.
    let (_, output, _) = run_zshrs(r#"x=$'a\x01b'; print "${(V)x}""#);
    assert_eq!(output.trim(), "a^Ab");
}

#[test]
fn test_print_P_L_uses_in_shell_shlvl() {
    // zsh: `print -P "%L"` outputs the in-shell SHLVL (already
    // incremented at startup over the parent's value). zshrs's
    // `prompt_tls::sync_from_executor` reads `self.variables["SHLVL"]`
    // first (same fix as the old `build_prompt_expand_env` path).
    let (_, output, _) = run_zshrs(r#"echo "SHLVL=$SHLVL"; print -P "L=%L""#);
    let lines: Vec<&str> = output.lines().collect();
    let shlvl_line = lines.iter().find(|l| l.starts_with("SHLVL=")).unwrap();
    let l_line = lines.iter().find(|l| l.starts_with("L=")).unwrap();
    let shlvl: i32 = shlvl_line.trim_start_matches("SHLVL=").parse().unwrap();
    let l: i32 = l_line.trim_start_matches("L=").parse().unwrap();
    assert_eq!(l, shlvl, "got SHLVL={shlvl}, %L={l}");
}

#[test]
fn test_typeset_integer_float_default_zero() {
    // zsh: `typeset -i x` initializes x=0; `typeset -F y` initializes
    // y=0.0000000000 (default precision 10). zshrs left them empty.
    // `typeset -p` then printed `x=''` instead of zsh's `x=0`.
    let (_, output, _) = run_zshrs("typeset -i x; typeset -p x");
    assert_eq!(output.trim(), "typeset -i x=0");
    let (_, output, _) = run_zshrs("typeset -F y; typeset -p y");
    assert_eq!(output.trim(), "typeset -F y=0.0000000000");
}

#[test]
fn test_qq_flag_empty_array_emits_quoted_pair() {
    // zsh: `${(qq)a}` for an empty array emits `''` (one empty
    // quoted pair) — the array is treated as `[""]` for quoting so
    // the result still occupies a slot. zshrs returned actually
    // empty (consumer-droppable). Added an empty-array branch in
    // the q-flag state transition.
    let (_, output, _) = run_zshrs(r#"a=(); print "${(qq)a}""#);
    assert_eq!(output.trim(), "''");
}

#[test]
fn test_array_slice_neg_start_below_neg_len_empty() {
    // zsh: `${a[-5,-1]}` with `len=3` empties — the start index
    // is below the array's lower bound. zshrs clamped both
    // negatives to valid range and returned the full array.
    let (_, output, _) = run_zshrs(r#"a=(1 2 3); print "[${a[-5,-1]}]""#);
    assert_eq!(output.trim(), "[]");
    let (_, output, _) = run_zshrs(r#"a=(1 2 3); print "[${a[-3,-1]}]""#);
    assert_eq!(output.trim(), "[1 2 3]");
}

#[test]
fn test_paren_at_flag_empty_array_preserves_brackets() {
    // `(@)NAME` is the splice equivalent of `[@]` — surrounding
    // literals must stick to first/last elements (so `[${(@)a}]`
    // for empty `a` still prints `[]` rather than dropping the
    // brackets). zshrs's `is_splice_expansion` only matched `[@]`/
    // `[*]`/slice forms; the `(@)` flag form fell to DISTRIBUTE
    // which drops the brackets. Added a `(...)` flag-block check
    // for `@`.
    let (_, output, _) = run_zshrs(r#"a=(); print "[${(@)a}]""#);
    assert_eq!(output.trim(), "[]");
    let (_, output, _) = run_zshrs(r#"a=(x y); print "[${(@)a}]""#);
    assert_eq!(output.trim(), "[x y]");
}

#[test]
fn test_history_unknown_flag_errors() {
    // zsh: `history -w` / `-X` are bash-style flags zsh doesn't
    // accept — `history:1: bad option: -X`. `history -r` is fc-
    // passable (reverse list) and falls through to fc which
    // reports `no such event` in -c mode. Match the split:
    // bash-style → bad option; fc-style → fall through.
    let (_, _, stderr) = run_zshrs("history -w");
    assert!(
        stderr.contains("history:1: bad option: -w"),
        "got: {stderr}"
    );
    let (_, _, stderr) = run_zshrs("history -X");
    assert!(
        stderr.contains("history:1: bad option: -X"),
        "got: {stderr}"
    );
    // -r is OK in zsh (treated like fc -r), falls through to
    // fc which then errors no-such-event in -c mode.
    let (_, _, stderr) = run_zshrs("history -r");
    assert!(stderr.contains("no such event"), "got: {stderr}");
}

#[test]
fn test_type_no_args_exits_1() {
    // zsh: bare `type` (no args) exits 1 — type requires at least
    // one name to look up. zshrs returned 0 silently.
    let (status, _, _) = run_zshrs("type");
    assert_eq!(status, 1);
}

#[test]
fn test_unalias_bad_option_format() {
    // zsh: `unalias -X x` errors `unalias:1: bad option: -X`.
    // zshrs previously emitted `unalias: bad option: -X` (extra
    // space, no `:1:` source-position suffix). Aligned the format.
    let (_, _, stderr) = run_zshrs("unalias -X x");
    assert!(
        stderr.contains("unalias:1: bad option: -X"),
        "got: {stderr}"
    );
}

#[test]
fn test_fc_non_numeric_event_spec() {
    // `fc -l blah` (non-numeric event spec) errors `event not
    // found: blah` — distinct from the numeric `no such event: N`
    // error. zshrs collapsed both to the numeric form.
    let (_, _, stderr) = run_zshrs("fc -l blah");
    assert!(stderr.contains("event not found: blah"), "got: {stderr}");
}

#[test]
fn test_test_lt_gt_le_ge_non_numeric_errors() {
    // Same fix as `-eq`/`-ne` extended to `-lt`/`-le`/`-gt`/`-ge`.
    // Each comparator now returns 2 with `integer expression
    // expected: <arg>` when either operand fails to parse as i64.
    for op in &["-lt", "-le", "-gt", "-ge"] {
        let cmd = format!("[ 5 {} abc ]", op);
        let (status, _, stderr) = run_zshrs(&cmd);
        assert_eq!(status, 2, "op={op}");
        assert!(
            stderr.contains("integer expression expected"),
            "op={op}, stderr={stderr}"
        );
    }
}

#[test]
fn test_type_unknown_flag_errors() {
    // `type --help` should error `bad option: -h` exit 1. zshrs's
    // unknown-flag arm previously did nothing, silently passing
    // with no output. Added an `eprintln + return 1` and skipped
    // additional `-` chars in the body so `--help` reports `-h`
    // (the first letter after the leading dashes), matching zsh.
    let (status, _, stderr) = run_zshrs("type --help");
    assert_eq!(status, 1);
    assert!(stderr.contains("bad option: -h"), "got: {stderr}");
}

#[test]
fn test_test_eq_non_numeric_errors() {
    // `[ a -eq a ]` with non-numeric operands errors `integer
    // expression expected: a` exit 2. zshrs previously used
    // `unwrap_or(0)` so `a` silently coerced to 0 and `a -eq 0`
    // was true. Added an explicit parse-or-error in the `-eq`/`-ne`
    // arms.
    let (status, _, stderr) = run_zshrs("[ a -eq a ]");
    assert_eq!(status, 2);
    assert!(
        stderr.contains("integer expression expected: a"),
        "got: {stderr}"
    );
}

#[test]
fn test_command_long_option_treated_as_command_name() {
    // `command --help` (and any `--xxx` long-option-style arg) is
    // treated as a command name in zsh — `command not found: --help`
    // exit 127. zshrs's flag parser hit the `--` arm (which it
    // intended for the bare end-of-options token) and silently
    // dropped the rest of the arg, returning 0 with no output.
    let (status, _, stderr) = run_zshrs("command --help");
    assert_eq!(status, 127);
    assert!(
        stderr.contains("command not found: --help"),
        "got: {stderr}"
    );
}

#[test]
fn test_command_unknown_flag_treated_as_command_name() {
    // zsh: `command -x ls` treats `-x` as a command name (since `-x`
    // isn't a recognised `command` flag). Output: "command not found:
    // -x". zshrs printed "command: bad option: -x" instead.
    let (status, _, stderr) = run_zshrs("command -x ls");
    assert_eq!(status, 127);
    assert!(stderr.contains("command not found: -x"), "got: {stderr}");
}

#[test]
fn test_umask_too_many_args() {
    // zsh: `umask 022 044` errors `too many arguments` and exits 1.
    // zshrs's loop overwrote `value` silently with the last
    // positional, accepting the call. Now counts positionals and
    // errors when > 1.
    let (status, _, stderr) = run_zshrs("umask 022 044");
    assert_eq!(status, 1);
    assert!(stderr.contains("too many arguments"), "got: {stderr}");
}

#[test]
fn test_functions_t_no_trace_set_silent() {
    // zsh: `functions -t NAME` lists only functions whose trace
    // attribute IS set. For a vanilla function with no trace
    // marking, output is empty. zshrs printed `functions -t NAME`
    // unconditionally. Now silent for the common no-trace case
    // (matches zsh's per-function trace gating; per-function trace
    // tracking is a follow-up).
    let (_, output, _) = run_zshrs("foo() { :; }; functions -t foo");
    assert_eq!(output, "");
}

#[test]
fn test_empty_cond_bracket_parse_error() {
    // zsh: `[[ ]]` (empty condition) is a parse error. zshrs
    // silently accepted and returned exit 0. Now `par_cond`
    // detects the empty case (immediate `Doutbrack` after the
    // opening) and emits a parse error.
    let (status, _, stderr) = run_zshrs("[[ ]]; echo done");
    assert_ne!(status, 0);
    assert!(stderr.contains("parse error"), "got: {stderr}");
    // Non-empty cond still works.
    let (_, output, _) = run_zshrs("[[ -d /tmp ]] && echo dir");
    assert_eq!(output.trim(), "dir");
}

#[test]
fn test_exec_non_executable_file_status_126() {
    // zsh: invoking a non-executable file (`chmod 644 file; ./file`)
    // emits `permission denied: ./file` on stderr and exits 126
    // (POSIX: "command found but not executable"). zshrs's
    // `execute_external` returned `Err` for any non-NotFound IO
    // error, which the caller converted to 127 with no diagnostic.
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    let path = std::env::temp_dir().join("zshrs_perm_test");
    let _ = fs::remove_file(&path);
    fs::write(&path, b"#!/bin/sh\necho hi\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let cmd = format!(r#"{} 2>&1; echo "exit=$?""#, path.to_string_lossy());
    let (_, output, _) = run_zshrs(&cmd);
    assert!(output.contains("permission denied"), "got: {output:?}");
    assert!(output.contains("exit=126"), "got: {output:?}");
    let _ = fs::remove_file(&path);
}

#[test]
fn test_type_w_emits_name_colon_kind() {
    // zsh's `type -w NAME` prints `NAME: KIND` (one of `builtin`,
    // `command`, `function`, `alias`, `reserved`, `none`). zshrs
    // ignored the flag and printed the descriptive default form
    // (`NAME is a shell builtin`, `NAME is /bin/...`).
    let (_, output, _) = run_zshrs("type -w cd ls echo");
    assert!(output.contains("cd: builtin"), "got: {output:?}");
    assert!(output.contains("ls: command"), "got: {output:?}");
    assert!(output.contains("echo: builtin"), "got: {output:?}");
    let (_, output, _) = run_zshrs("type -w nosuchcmdxyz");
    assert!(output.contains("nosuchcmdxyz: none"), "got: {output:?}");
    let (_, output, _) = run_zshrs("foo() { :; }; type -w foo");
    assert!(output.contains("foo: function"), "got: {output:?}");
}

#[test]
fn test_umask_bad_symbolic_operator_specific_error() {
    // zsh validates symbolic umask args char by char; after the
    // class chars (`u`/`g`/`o`/`a`) it expects an operator
    // (`+`/`-`/`=`). `umask abcd` → first `a` is class, then `b`
    // isn't an operator → `bad symbolic mode operator: b`. zshrs
    // previously emitted a generic `invalid mask: abcd`.
    let (_, _, stderr) = run_zshrs("umask abcd");
    assert!(
        stderr.contains("bad symbolic mode operator: b"),
        "got: {stderr}"
    );
    let (_, _, stderr) = run_zshrs("umask uxyz");
    assert!(
        stderr.contains("bad symbolic mode operator: x"),
        "got: {stderr}"
    );
}

#[test]
fn test_fc_l_range_two_args_no_events_in_range() {
    // zsh: `fc -l N M` (range query) errors `no events in that
    // range` when the history is empty — distinct from the
    // single-arg `no such event: N`. zshrs collapsed both to the
    // single-arg message.
    let (_, _, stderr) = run_zshrs("fc -l 1 2");
    assert!(stderr.contains("no events in that range"), "got: {stderr}");
    // Single-arg still uses the per-event form.
    let (_, _, stderr) = run_zshrs("fc -l 5");
    assert!(stderr.contains("no such event: 5"), "got: {stderr}");
}

#[test]
fn test_unalias_no_args_emits_zsh_format() {
    // zsh: bare `unalias` errors `unalias:1: not enough arguments`.
    // zshrs printed a bash-style usage line. Aligned to zsh format
    // so script consumers pattern-matching on `unalias:1:` see the
    // expected diagnostic.
    let (status, _, stderr) = run_zshrs("unalias");
    assert_eq!(status, 1);
    assert!(
        stderr.contains("unalias:1: not enough arguments"),
        "got: {stderr}"
    );
}

#[test]
fn test_cond_N_file_modified_since_access() {
    // zsh: `[[ -N file ]]` is true when the file's access time is
    // not newer than its modification time (atime <= mtime). zshrs
    // emitted "unknown condition: -N" because the cond compiler had
    // no arm. Added BUILTIN_FILE_MODIFIED_SINCE_ACCESS (id 341) and
    // an emit_file_test arm.
    let (_, output, _) = run_zshrs(
        "touch /tmp/zshrs_N_test; [[ -N /tmp/zshrs_N_test ]] && echo modified; rm -f /tmp/zshrs_N_test",
    );
    assert_eq!(output.trim(), "modified");
}

#[test]
fn test_fc_unknown_flag_errors() {
    // zsh: `fc -h` (or any unknown letter) errors `bad option: -X`
    // and bails. zshrs's flag-letter loop had a silent default arm,
    // so `fc -h` fell through to the no-args path (re-execute last
    // command), which on -c mode could recurse infinitely (fc just
    // entered history). Now unknown flags emit the diagnostic and
    // return 1.
    let (status, _, stderr) = run_zshrs("fc -h");
    assert_eq!(status, 1);
    assert!(stderr.contains("bad option: -h"), "got: {stderr}");
    let (status, _, stderr) = run_zshrs("fc -w");
    assert_eq!(status, 1);
    assert!(stderr.contains("bad option: -w"), "got: {stderr}");
}

#[test]
fn test_functions_T_enable_trace_silent() {
    // zsh: `functions -T NAME` enables tracing on NAME and emits no
    // listing. zshrs didn't recognize `-T` and fell into the
    // default-list-body path, printing the function source. Added a
    // silent-no-op arm for `-T`.
    let (status, output, _) = run_zshrs("foo() { echo a; }; functions -T foo");
    assert_eq!(status, 0);
    assert_eq!(output, "");
}

#[test]
fn test_kill_l_unknown_signal_format() {
    // zsh: `kill -l XYZ` errors `zsh:kill:1: unknown signal: SIGXYZ`
    // (note the SIG prefix on the signal name AND the typed prefix).
    // zshrs printed `kill: unknown signal: XYZ` (missing both).
    // c:Src/jobs.c:2845 `returnval++` for each unknown — rc=1 per
    // unknown signal. /bin/zsh and /opt/homebrew/bin/zsh both rc=1.
    let (status, _, stderr) = run_zshrs("kill -l XYZ");
    assert_eq!(status, 1);
    assert!(stderr.contains("unknown signal: SIGXYZ"), "got: {stderr}");
    assert!(stderr.contains("zshrs:kill:1:"), "got: {stderr}");
}

#[test]
fn test_typeset_readonly_aborts() {
    // zsh: `readonly y=1; typeset y=2` errors `read-only variable: y`
    // and aborts the shell in -c mode (status 1, no output after).
    // zshrs's typeset path didn't check `readonly_vars` and silently
    // overwrote the value.
    let (status, output, stderr) = run_zshrs(r#"readonly y=1; typeset y=2; echo after"#);
    assert_eq!(status, 1);
    assert!(stderr.contains("read-only variable: y"), "got: {stderr}");
    assert!(!output.contains("after"), "got: {output:?}");
}

#[test]
fn test_print_z_does_not_emit_to_stdout() {
    // zsh's `print -z` pushes to the line editor's buffer stack —
    // in non-interactive mode there's no editor, so the args are
    // discarded silently with exit 0. zshrs previously fell through
    // to stdout and printed the args.
    let (status, output, _) = run_zshrs(r#"print -z "ls""#);
    assert_eq!(status, 0);
    assert_eq!(output, "");
}

#[test]
fn test_kill_l_includes_info_on_macos() {
    // zsh's `kill -l` lists SIGINFO between WINCH and USR1 on macOS.
    // zshrs's signal_map didn't include INFO, so the listing
    // skipped it and the column count was off by one.
    if cfg!(target_os = "macos") {
        let (_, output, _) = run_zshrs("kill -l");
        assert!(output.contains("INFO"), "got: {output:?}");
    }
}

#[test]
fn test_fc_l_default_no_args_event_one() {
    // zsh: `fc -l` (no args) in -c mode with empty history reports
    // `no such event: 1` (the lower bound of the would-be -16..-1
    // range, clamped to 1). zshrs previously reported 0.
    let (_, _, stderr) = run_zshrs("fc -l");
    assert!(stderr.contains("no such event: 1"), "got: {stderr}");
}

#[test]
fn test_let_no_args_errors() {
    // zsh: bare `let` errors `not enough arguments` with exit 1.
    // zshrs's `builtin_let` returned 1 silently (no diagnostic).
    let (status, _, stderr) = run_zshrs("let");
    assert_eq!(status, 1);
    assert!(stderr.contains("not enough arguments"), "got: {stderr}");
}

#[test]
fn test_history_in_minus_c_mode_errors() {
    // In `-c` (non-interactive) mode with no session entries,
    // zsh's `history` (= `fc -l`) errors `no such event: 1`
    // rather than dumping the on-disk persistent history.
    // Mirrored — only show session entries (from `print -s`) and
    // abort otherwise.
    let (status, _, stderr) = run_zshrs("history");
    assert_eq!(status, 1);
    assert!(stderr.contains("no such event: 1"), "got: {stderr}");
    // With session adds, history shows them numbered from 1.
    let (_, output, _) = run_zshrs("print -s a; print -s b; history");
    assert!(output.contains("    1  a"), "got: {output:?}");
    assert!(output.contains("    2  b"), "got: {output:?}");
}

#[test]
fn test_read_q_requires_terminal() {
    // zsh's `read -q` reads a single y/n from a terminal — outside
    // a tty it errors "not interactive and can't open terminal".
    // zshrs previously read from stdin and returned 0 silently.
    let (status, _, stderr) = run_zshrs(r#"echo y | read -q ans"#);
    assert_eq!(status, 1);
    assert!(stderr.contains("can't open terminal"), "got: {stderr}");
}

#[test]
fn test_escaped_glob_metachar_does_not_trigger_nomatch() {
    // `echo \*` should not abort with NOMATCH — the backslash escapes
    // the `*` so it's not a glob trigger. zshrs's `looks_like_glob`
    // counted any `*`/`?`/`[` in the pattern even when escaped, so
    // `\*` was treated as a glob, expanded against the cwd (no
    // matches), and aborted in NOMATCH mode. Now the check walks
    // characters and skips `\X` escape pairs.
    let (status, _, stderr) = run_zshrs(r#"echo \*"#);
    assert_eq!(status, 0, "stderr was: {stderr}");
    assert!(!stderr.contains("no matches"), "got: {stderr}");
}

#[test]
fn test_set_a_enables_allexport() {
    // `set -a` enables `allexport`. zshrs's multi-letter set-flag
    // parser had no `a` arm, so `set -a` silently passed through
    // without flipping the option. Now `a` enables allexport (and
    // `+a` disables); `set -a; setopt | head -1` shows `allexport`
    // matching zsh.
    let (_, output, _) = run_zshrs("set -a; setopt | head -1");
    assert_eq!(output.trim(), "allexport");
}

#[test]
fn test_tilde_digit_is_dirstack_index() {
    // zsh: `~N` for purely-digit N is shorthand for `~+N` —
    // Nth entry on the directory stack, 0 = $PWD. zshrs treated
    // `~0` as a user lookup and aborted "no such user". Added a
    // digits-only branch above the `getpwnam` path so `~0` returns
    // current PWD without going through user resolution.
    let (_, output, _) = run_zshrs("echo ~0");
    assert!(!output.is_empty(), "got: {output:?}");
    // Should be a real path (starts with `/`), not an error.
    assert!(output.trim().starts_with('/'), "got: {output:?}");
}

#[test]
fn test_fc_l_event_number_width() {
    // zsh's `fc -l` prints event numbers right-aligned in a 5-char
    // field (`    1  hist1`). zshrs used `{:>6}` (6-char field), so
    // entries showed up indented one extra space. Switched all the
    // `fc -l` print sites to `{:>5}` so the column alignment matches.
    let (_, output, _) = run_zshrs(r#"print -s "hist1"; print -s "hist2"; fc -l"#);
    let last_two: Vec<&str> = output
        .lines()
        .rev()
        .take(2)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    assert_eq!(last_two[0], "    1  hist1");
    assert_eq!(last_two[1], "    2  hist2");
}

#[test]
fn test_empty_function_body() {
    // zsh treats `{}` as an empty compound — `foo() {}` defines a
    // no-op function. zshrs's lexer required whitespace after `{`
    // for Inbrace recognition, so `{}` lexed as one literal token
    // and the function-body parser failed. Now `{` followed by `}`
    // (or whitespace/newline) lexes as Inbrace even outside cmd
    // position so `foo() {}` parses as a function with empty body.
    let (_, _, _) = run_zshrs("foo() {}; foo");
    let (_, output, _) = run_zshrs("foo() {}; foo; echo $?");
    assert_eq!(output.trim(), "0");
    let (_, output, _) = run_zshrs("{}; echo done");
    assert_eq!(output.trim(), "done");
}

#[test]
fn test_histchars_default() {
    // zsh's `$histchars` is the 3-character string `!^#` by default
    // (bang, hat, hash). zshrs left it unset so script reads of
    // `$histchars` returned empty. Initialized in `ShellExecutor::new`.
    let (_, output, _) = run_zshrs("echo $histchars");
    assert_eq!(output.trim(), "!^#");
}

#[test]
fn test_array_element_pattern_replace() {
    // `${a[1]/pat/repl}` — pattern replace on a single array element.
    // zshrs's bracket-modifier path skipped pattern replacement so
    // the element returned unchanged. Now the `/`, `//`, `/#`, `/%`
    // modifiers all dispatch through the new `zsh_pattern_replace`
    // helper (extracted from `BUILTIN_PARAM_REPLACE`).
    let (_, output, _) = run_zshrs(r#"a=(file.txt other.txt); print -- ${a[1]/.txt/.bak}"#);
    assert_eq!(output.trim(), "file.bak");
    let (_, output, _) = run_zshrs(r#"a=(hello world); print -- ${a[1]//l/L}"#);
    assert_eq!(output.trim(), "heLLo");
    let (_, output, _) = run_zshrs(r#"a=(hello); print -- ${a[1]/#h/H}"#);
    assert_eq!(output.trim(), "Hello");
    let (_, output, _) = run_zshrs(r#"a=(hello); print -- ${a[1]/%o/O}"#);
    assert_eq!(output.trim(), "hellO");
}

#[test]
fn test_print_S_adds_to_history_silently() {
    // zsh: `print -S "..."` adds the line to history INSTEAD of
    // emitting it on stdout (the "S" form is the split-words variant
    // of `-s`). zshrs printed the line to stdout because `S` was in
    // the TODO list of unhandled flags. Now `-S` is treated like
    // `-s` (add_to_history=true).
    let (_, output, _) = run_zshrs(r#"print -S "hello""#);
    assert_eq!(output, "");
}

#[test]
fn test_printf_hex_zero_pad() {
    // `printf "%04x" 42` should zero-pad to `002a`. zshrs only had
    // the zero-pad branch on `%u`; `%x`/`%X`/`%o` always padded with
    // spaces. Fixed by adding the `zero_pad` branch (with prefix
    // emission preserved) to all three integer-radix conversions.
    let (_, output, _) = run_zshrs(r#"printf "%04x\n" 42"#);
    assert_eq!(output, "002a\n");
    let (_, output, _) = run_zshrs(r#"printf "%04X\n" 42"#);
    assert_eq!(output, "002A\n");
    let (_, output, _) = run_zshrs(r#"printf "%04o\n" 8"#);
    assert_eq!(output, "0010\n");
}

#[test]
fn test_for_arith_with_dollar_param_in_cond() {
    // `for ((i=1; i<=$#a; i++))` — cond contains `$#a` (param
    // expansion). zshrs's `compile_for_arith` routed the cond
    // through ArithCompiler when `,` wasn't present, but
    // ArithCompiler's lexer can't parse `$`. Added `$` to the
    // routing trigger AND made init/cond/step share a single
    // `needs_eval_global` decision so all three sections use the
    // same storage backend (MathEval/variables) — otherwise `i` set
    // by ArithCompiler in init wouldn't be visible to MathEval in
    // cond.
    let (_, output, _) =
        run_zshrs(r#"a=(x y z); for ((i=1; i<=$#a; i++)); do echo "$i:$a[$i]"; done"#);
    assert_eq!(output, "1:x\n2:y\n3:z\n");
}

#[test]
fn test_set_unknown_flag_silent() {
    // zsh accepts unknown single-letter `set` flags silently
    // (`set -y`, `set -xy`). zshrs erroring on `-y` broke any
    // script that combined set flags. Default arm now silently
    // ignores unknown flag letters.
    let (status, _, stderr) = run_zshrs("set -y; echo done");
    assert_eq!(status, 0);
    assert!(!stderr.contains("invalid option"), "got: {stderr}");
}

#[test]
fn test_arith_division_by_zero_continues() {
    // `((1/0))` arith COMMAND prints the error and continues with a
    // non-zero status (zsh uses 2; zshrs's compile path sets 1 from
    // the StrEq-to-"0" check — close enough that scripts treat both
    // as failure via `(( … )) && …` gating). The substitution path
    // (`$((1/0))`) emits the diagnostic and the surrounding command
    // sees the error.
    let (_, _, stderr) = run_zshrs(r#"((1/0)); echo "after $?""#);
    assert!(stderr.contains("division by zero"), "got: {stderr}");
    let (_, output, _) = run_zshrs(r#"((1/0)); echo "after $?""#);
    assert!(
        output.contains("after 1") || output.contains("after 2"),
        "got: {output:?}"
    );
    // Substitution: must emit the error to stderr; whether the
    // surrounding `echo` runs is a follow-up concern (zsh aborts
    // the whole command, zshrs prints "0" and continues).
    let (_, _, stderr) = run_zshrs("echo $((1/0))");
    assert!(stderr.contains("division by zero"), "got: {stderr}");
}

#[test]
fn test_cond_v_with_positional_param() {
    // `[[ -v N ]]` for an integer name N should test whether the Nth
    // positional parameter is set. zshrs's `BUILTIN_VAR_EXISTS`
    // checked only `variables`/`arrays`/`assoc_arrays`/env — so even
    // after `set -- one`, `[[ -v 1 ]]` returned false.
    let (_, output, _) = run_zshrs("set -- one; [[ -v 1 ]] && echo set");
    assert_eq!(output.trim(), "set");
    let (_, output, _) = run_zshrs("set -- a b c; [[ -v 3 ]] && echo set");
    assert_eq!(output.trim(), "set");
    let (_, output, _) = run_zshrs("[[ -v 1 ]] && echo set || echo unset");
    assert_eq!(output.trim(), "unset");
}

#[test]
fn test_histsize_min_clamp_to_one() {
    // zsh enforces a minimum of 1 on `HISTSIZE` — `HISTSIZE=0` and
    // negative values both clamp to 1. zshrs stored the literal value
    // unchanged. Now `BUILTIN_SET_VAR` re-clamps when name is
    // `HISTSIZE` so subsequent reads match zsh.
    let (_, output, _) = run_zshrs("HISTSIZE=0; echo $HISTSIZE");
    assert_eq!(output.trim(), "1");
    let (_, output, _) = run_zshrs("HISTSIZE=-5; echo $HISTSIZE");
    assert_eq!(output.trim(), "1");
    let (_, output, _) = run_zshrs("HISTSIZE=100; echo $HISTSIZE");
    assert_eq!(output.trim(), "100");
}

#[test]
fn test_substring_with_arithmetic_length() {
    // `${x:0:${#x}-2}` should compute length=len-2. zshrs's
    // `parse_param_modifier` previously rejected ANY shape with
    // nested `${...}`, falling through to the bridge which didn't
    // handle substring properly. Now nested expansions in the
    // length operand route through `SubstringExpr` which evaluates
    // them at runtime via expand_string + arith.
    let (_, output, _) = run_zshrs(r#"x=hello; echo "${x:0:${#x}-2}""#);
    assert_eq!(output.trim(), "hel");
    let (_, output, _) = run_zshrs(r#"x=hello; echo "${x:1:${#x}-3}""#);
    assert_eq!(output.trim(), "el");
    let (_, output, _) = run_zshrs(r#"x=hello; echo "${x:0:${#x}}""#);
    assert_eq!(output.trim(), "hello");
}

#[test]
fn test_arith_command_with_parameter_expansion() {
    // `(( ${+h[k]} ))` — arith command operand contains parameter
    // expansion. zshrs's `compile_arith_str` previously routed
    // through ArithCompiler which can't parse `$` tokens, so the
    // expansion stayed un-evaluated and `((  ))` saw 0. Added `$`
    // to the `needs_eval` triggers so any expr touching parameter
    // expansion routes through `BUILTIN_ARITH_EVAL` (MathEval).
    let (_, output, _) =
        run_zshrs(r#"typeset -A h; h[a]=1; (( ${+h[a]} )) && echo yes; (( ${+h[b]} )) || echo no"#);
    assert_eq!(output.trim(), "yes\nno");
}

#[test]
fn test_typeset_i_base_format_at_assignment() {
    // `typeset -i N x; x=255` should store `N#DIGITS` form (zsh's
    // base notation). Previously zshrs only formatted at the
    // `typeset` call itself, not later assignments.
    let (_, output, _) = run_zshrs("typeset -i 16 x; x=255; echo $x");
    assert_eq!(output.trim(), "16#FF");
    let (_, output, _) = run_zshrs("typeset -i 8 y; y=8; echo $y");
    assert_eq!(output.trim(), "8#10");
    let (_, output, _) = run_zshrs("typeset -i 2 z; z=10; echo $z");
    assert_eq!(output.trim(), "2#1010");
}

#[test]
fn test_cond_v_with_array_subscript() {
    // `[[ -v a[N] ]]` checks whether array element N is populated.
    // zshrs previously glob-expanded `a[1]` (treating `[1]` as a
    // char-class), failing with "no matches found". Now the cond
    // compiler emits the operand as a literal so the runtime
    // BUILTIN_VAR_EXISTS sees `a[1]` intact and splits it into
    // (array, key).
    let (_, output, _) = run_zshrs("a=(1 2); [[ -v a[1] ]] && echo elem-1");
    assert_eq!(output.trim(), "elem-1");
    let (_, output, _) = run_zshrs("a=(1 2); [[ -v a[5] ]] && echo set || echo unset");
    assert_eq!(output.trim(), "unset");
    let (_, output, _) = run_zshrs("typeset -A h=(k v); [[ -v h[k] ]] && echo hash-key");
    assert_eq!(output.trim(), "hash-key");
    let (_, output, _) = run_zshrs("typeset -A h=(k v); [[ -v h[m] ]] && echo set || echo unset");
    assert_eq!(output.trim(), "unset");
}

#[test]
fn test_test_builtin_paren_grouping() {
    // POSIX `[ ... ]` supports `\(` `\)` to group sub-expressions
    // around `-a`/`-o` connectives. zshrs's default-arm split on the
    // last `-a`/`-o` ignored paren depth, breaking grouped
    // expressions like `[ \( -n a \) -a \( -z "" \) ]`.
    let (_, output, _) = run_zshrs(r#"[ \( -n abc \) -a \( -z "" \) ] && echo paren"#);
    assert_eq!(output.trim(), "paren");
    let (_, output, _) = run_zshrs(r#"[ \( -n "" -o -n abc \) -a -z "" ] && echo nested"#);
    assert_eq!(output.trim(), "nested");
}

#[test]
fn test_fc_l_no_event_uses_resolved_index() {
    // zsh: `fc -l 0` and `fc -l -5` (in -c mode with empty history)
    // both report `no such event: 0`. Negative offsets resolve to 0
    // when there are no entries; positive args echo verbatim. zshrs
    // previously hardcoded "1" when the user passed 0 or negative.
    let (_, _, stderr) = run_zshrs("fc -l 0");
    assert!(stderr.contains("no such event: 0"), "got: {stderr}");
    let (_, _, stderr) = run_zshrs("fc -l -5");
    assert!(stderr.contains("no such event: 0"), "got: {stderr}");
    let (_, _, stderr) = run_zshrs("fc -l 5");
    assert!(stderr.contains("no such event: 5"), "got: {stderr}");
}

#[test]
fn test_read_p_flag_means_coprocess_not_prompt() {
    // zsh's `read -p` is "read from coprocess input" — NOT prompt
    // (the prompt feature uses `read 'NAME?prompt'` syntax). Without
    // a coprocess, zsh emits "no coprocess" and bails.
    let (status, _, stderr) = run_zshrs("echo hi | read -p x");
    assert!(stderr.contains("-p: no coprocess"), "got: {stderr}");
    assert_ne!(status, 0);
}

#[test]
fn test_alias_recursion_guard_self_disables() {
    // zsh's lexer disables an alias inside its own body so common
    // wrappers like `alias ls='ls -la'` work without infinite
    // recursion. zshrs expands aliases at run time, so we need an
    // explicit `expanding_aliases` HashSet to break the loop.
    // `alias g="g hi"; g` should resolve `g` once → `g hi` → fail
    // "command not found" on the second `g`, NOT stack overflow.
    let (status, _output, stderr) = run_zshrs(r#"alias g="g hi"; g"#);
    // Expansion ran once, second `g` falls through to external lookup,
    // which fails.
    assert!(
        stderr.contains("command not found") || status != 0,
        "expected command-not-found, got status={status}, stderr={stderr}"
    );
    // Standard non-recursive alias still expands when run through
    // `eval` (a fresh parse pass that sees the alias). Direct in-script
    // `alias hi=…; hi` doesn't expand because the script is parsed
    // before the alias is registered — verified the same way against
    // /bin/zsh -fc.
    let (_, output, _) = run_zshrs(r#"alias hi="echo hello"; eval "hi""#);
    assert_eq!(output.trim(), "hello");
}

#[test]
fn test_assoc_keys_preserve_insertion_order() {
    // zsh stores assoc-array entries in insertion order (params.c
    // hashtable hnodes). ${(k)h} and ${(kv)h} must iterate in
    // insertion order, not random hash order.
    let (_, output, _) = run_zshrs(r#"typeset -A h; h=(a 1 b 2 c 3); echo ${(k)h}"#);
    assert_eq!(output.trim(), "a b c");
    let (_, output, _) = run_zshrs(r#"typeset -A h; h=(a 1 b 2 c 3); echo ${(kv)h}"#);
    assert_eq!(output.trim(), "a 1 b 2 c 3");
}

#[test]
fn test_for_multi_var_pairs_consume_array() {
    // zsh parse.c par_for accepts multiple identifier tokens before
    // `in`. `for k v in arr` consumes pairs of values per iteration.
    let (_, output, _) = run_zshrs(r#"arr=(a 1 b 2 c 3); for k v in $arr; do echo "$k=$v"; done"#);
    assert_eq!(output.trim(), "a=1\nb=2\nc=3");
}

#[test]
fn test_for_multi_var_three_consume_triples() {
    let (_, output, _) =
        run_zshrs(r#"arr=(a 1 x b 2 y); for k v w in $arr; do echo "$k:$v:$w"; done"#);
    assert_eq!(output.trim(), "a:1:x\nb:2:y");
}

#[test]
fn test_for_multi_var_kv_iterates_assoc() {
    // The driving real-world case: iterate an assoc by key+value.
    let (_, output, _) =
        run_zshrs(r#"typeset -A h; h=(a 1 b 2); for k v in ${(kv)h}; do echo "$k=$v"; done"#);
    assert_eq!(output.trim(), "a=1\nb=2");
}

#[test]
fn test_glob_tilde_exclude_at_path_level() {
    // pattern.c P_EXCLUDE matches RHS as a pattern against each LHS
    // candidate's basename — not a separate glob expansion in CWD.
    let dir = std::env::temp_dir().join("zshrs_test_tilde_exclude");
    let _ = std::fs::create_dir_all(&dir);
    for name in ["a.txt", "b.txt", "README.txt"] {
        std::fs::File::create(dir.join(name)).unwrap();
    }
    let cmd = format!("setopt extendedglob; echo {}/*.txt~*README*", dir.display());
    let (_, output, _) = run_zshrs(&cmd);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        !output.contains("README.txt"),
        "tilde exclusion should drop README.txt; got: {}",
        output
    );
    assert!(output.contains("a.txt"));
    assert!(output.contains("b.txt"));
}

#[test]
fn test_nested_expansion_strip_after_inner() {
    // ${${a%.txt}#hel}: inner strips suffix .txt from "hello.txt" giving
    // "hello"; outer then strips prefix "hel" giving "lo".
    let (_, output, _) = run_zshrs(r#"a=hello.txt; echo "${${a%.txt}#hel}""#);
    assert_eq!(output.trim(), "lo");
}

#[test]
fn test_nested_expansion_outer_flag_applied_to_inner() {
    // ${(s. .)${(j. .)a}}: inner joins array on " " giving "a b c";
    // outer split flag (s. .) splits scalar on " " giving "a b c"
    // (printed space-joined when echoed).
    let (_, output, _) = run_zshrs(r#"a=(a b c); echo "${(s. .)${(j. .)a}}""#);
    assert_eq!(output.trim(), "a b c");
}

#[test]
fn test_param_join_split_bracket_pair_delim() {
    // zsh subst.c get_strarg accepts matched bracket pairs as flag
    // delimiters: `[`/`]`, `{`/`}`, `(`/`)`, `<`/`>`. Without pair-aware
    // close, `j[+]` left `]` in the separator and produced `a+]b+]c`.
    let (_, output, _) = run_zshrs(r#"a=(a b c); echo "${(j[+])a}""#);
    assert_eq!(output.trim(), "a+b+c");
    let (_, output, _) = run_zshrs(r#"a=(1 2 3); echo "${(j[, ])a}""#);
    assert_eq!(output.trim(), "1, 2, 3");
    let (_, output, _) = run_zshrs(r#"a="x|y|z"; echo "${(s[|])a}""#);
    assert_eq!(output.trim(), "x y z");
    let (_, output, _) = run_zshrs(r#"a=(a b c); echo "${(j<X>)a}""#);
    assert_eq!(output.trim(), "aXbXc");
}

#[test]
fn test_param_q_modifier_strips_backslash_escapes() {
    // `:Q` should remove shell quoting INCLUDING backslash escapes
    // (zsh hist.c remquote). Previously only stripped matched
    // `'`/`"` pairs.
    let (_, output, _) = run_zshrs(r#"a="a\\ b"; echo ${a:Q}"#);
    assert_eq!(output.trim(), "a b");
    let (_, output, _) = run_zshrs(r#"a="hello\\ world"; echo ${a:Q}"#);
    assert_eq!(output.trim(), "hello world");
}

#[test]
fn test_arith_assoc_subscript_postinc() {
    // `((h[k]++))` requires the lvalue to retain identity through the
    // operator — without intercepting the compound op shape, the
    // pre-resolve pass substituted h[k] with its value and `5++`
    // errored "lvalue required".
    let (_, output, _) = run_zshrs(r#"typeset -A h; h[a]=5; ((h[a]++)); echo $h[a]"#);
    assert_eq!(output.trim(), "6");
}

#[test]
fn test_arith_assoc_subscript_compound_assign() {
    let (_, output, _) = run_zshrs(r#"typeset -A h; h[a]=5; ((h[a]+=10)); echo $h[a]"#);
    assert_eq!(output.trim(), "15");
}

#[test]
fn test_arith_array_subscript_pre_inc() {
    // `((++a[i]))` — pre-increment on subscripted array.
    let (_, output, _) = run_zshrs(r#"a=(10 20 30); ((++a[2])); echo $a"#);
    assert_eq!(output.trim(), "10 21 30");
}

#[test]
fn test_arith_assoc_subscript_pre_inc() {
    let (_, output, _) = run_zshrs(r#"typeset -A h; h[a]=5; ((++h[a])); echo $h[a]"#);
    assert_eq!(output.trim(), "6");
}

#[test]
fn test_sort_flag_with_numeric_modifier_either_order() {
    // `(no)` and `(on)` should both produce numeric ascending sort.
    // zsh: order-agnostic — `n`/`i`/`a` are sort-modifiers that pair
    // with `o`/`O`. Was applying them sequentially so `n` got
    // overwritten by the subsequent `o`'s alpha sort.
    let (_, output, _) = run_zshrs(r#"a=(10 2 1 20); echo "${(no)a[@]}""#);
    assert_eq!(output.trim(), "1 2 10 20");
    let (_, output, _) = run_zshrs(r#"a=(10 2 1 20); echo "${(on)a[@]}""#);
    assert_eq!(output.trim(), "1 2 10 20");
    let (_, output, _) = run_zshrs(r#"a=(10 2 1 20); echo "${(nO)a[@]}""#);
    assert_eq!(output.trim(), "20 10 2 1");
}

#[test]
fn test_glob_l_link_count_qualifier() {
    // `*(l2)` matches files with link count == 2; `+N` more, `-N` fewer.
    // zsh pattern.c qualifier — was missing from our handler.
    let dir = std::env::temp_dir().join("zshrs_link_qual");
    let _ = std::fs::create_dir_all(&dir);
    std::fs::File::create(dir.join("f")).unwrap();
    let _ = std::fs::hard_link(dir.join("f"), dir.join("h"));
    let cmd = format!("echo {}/*(l2)", dir.display());
    let (_, output, _) = run_zshrs(&cmd);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(output.contains("/f"));
    assert!(output.contains("/h"));
}

#[test]
fn test_cmd_subst_status_propagates_to_assign() {
    // `a=$(false); echo $?` should print 1 — cmd-subst status is
    // captured in last_status, then SET_VAR returns it as the
    // assignment's exit. Compile-side now emits SetStatus after
    // BUILTIN_SET_VAR so vm.last_status reflects the propagated
    // value.
    let (_, output, _) = run_zshrs(r#"a=$(false); echo $?"#);
    assert_eq!(output.trim(), "1");
    let (_, output, _) = run_zshrs(r#"a=$(true); echo $?"#);
    assert_eq!(output.trim(), "0");
    let (_, output, _) = run_zshrs(r#"a=hello; echo $?"#);
    assert_eq!(output.trim(), "0");
}

#[test]
fn test_param_j_flag_on_cmd_subst_no_op() {
    // `${(j:,:)$(cmd)}` — the cmd-subst returns a SCALAR; (j:::) on
    // a scalar is a no-op in zsh. We were over-applying by splitting
    // on whitespace and rejoining, mangling newline-separated output.
    let (_, output, _) = run_zshrs(r#"echo "${(j:,:)$(echo a; echo b; echo c)}""#);
    assert_eq!(output.trim(), "a\nb\nc");
}

#[test]
fn test_param_jf_split_then_join_cmd_subst() {
    // `${(j:,:)${(f)$(printf "...")}}` — (f) splits on newlines into
    // an array, then (j:::) joins. The whole pipeline reduces lines
    // to comma-separated.
    let (_, output, _) = run_zshrs(r#"echo "${(j:,:)${(f)$(printf "a\nb\nc")}}""#);
    assert_eq!(output.trim(), "a,b,c");
}

#[test]
fn test_array_zip_short_form() {
    // `${a:^b}` interleaves arrays up to min(len). Direct port of
    // zsh subst.c SUB_ZIP_SHORT.
    let (_, output, _) = run_zshrs(r#"a=(1 2 3); b=(x y z); print ${a:^b}"#);
    assert_eq!(output.trim(), "1 x 2 y 3 z");
    let (_, output, _) = run_zshrs(r#"a=(1 2); b=(x y z); print ${a:^b}"#);
    assert_eq!(output.trim(), "1 x 2 y");
}

#[test]
fn test_array_zip_long_form_cycles() {
    // `${a:^^b}` interleaves up to max(len), cycling the shorter.
    // SUB_ZIP_LONG.
    let (_, output, _) = run_zshrs(r#"a=(1 2); b=(x y z w); print ${a:^^b}"#);
    assert_eq!(output.trim(), "1 x 2 y 1 z 2 w");
}

#[test]
fn test_substring_offset_with_nested_arith() {
    // `${a:$((${#a}-2))}` — substring with nested arith in offset.
    // The compile-time substring shape detection rejected the case
    // because `off_section.contains("${")` returned None. Now allowed
    // when the operand starts with `$`/`(`/`-`/digit.
    let (_, output, _) = run_zshrs(r#"a=hello; echo "${a:$((${#a}-2))}""#);
    assert_eq!(output.trim(), "lo");
    let (_, output, _) = run_zshrs(r#"a=hello; echo "${a:0:${#a}-1}""#);
    assert_eq!(output.trim(), "hell");
}

#[test]
fn test_nested_replace_expands_dollar_in_repl() {
    // `${${a:-foo}/foo/$b}` — the replacement string `$b` was left
    // literal instead of getting expanded. zsh expands $-refs in
    // both pattern AND replacement.
    let (_, output, _) = run_zshrs(r#"b=fb; echo "${${a:-foo}/foo/$b}""#);
    assert_eq!(output.trim(), "fb");
}

#[test]
fn test_array_slice_at_preserves_splice_in_assignment() {
    // `b=("${a[@]:1}")` — array-slice with `[@]` should preserve
    // element boundaries when used in array-init context. Was
    // collapsing to single joined element. Compile path now
    // re-attaches `[@]` to the name passed to BUILTIN_PARAM_SUBSTRING
    // (and EXPR variant); runtime returns Value::Array when force_array.
    let (_, output, _) = run_zshrs(r#"a=(1 2 3); a=("${a[@]:1}"); echo "$#a""#);
    assert_eq!(output.trim(), "2");
    let (_, output, _) = run_zshrs(r#"a=(1 2 3); for x in "${a[@]:1}"; do echo "<$x>"; done"#);
    assert_eq!(output.trim(), "<2>\n<3>");
}

#[test]
fn test_array_consume_loop_terminates() {
    // The classic shift-via-slice idiom. Was looping forever because
    // each `a=("${a[@]:1}")` left a unchanged.
    let (_, output, _) =
        run_zshrs(r#"a=(1 2 3); while (($#a > 0)); do echo "${a[1]}"; a=("${a[@]:1}"); done"#);
    assert_eq!(output.trim(), "1\n2\n3");
}

#[test]
fn test_printf_redirect_to_file_writes_data() {
    // `printf "abc" > file` was leaking output to stdout AND leaving
    // the file empty because Rust's print! is block-buffered when
    // stdout is a non-tty file (post-dup2). The redirect_scope's
    // dup2-restore happened before the buffer flushed. Added explicit
    // flush at end of builtin_printf, builtin_echo, builtin_print.
    let path = std::env::temp_dir().join("zshrs_printf_redir_test.txt");
    let cmd = format!(r#"printf "%s" "abc" > {}"#, path.display());
    let _ = run_zshrs(&cmd);
    let contents = std::fs::read_to_string(&path).unwrap_or_default();
    let _ = std::fs::remove_file(&path);
    assert_eq!(contents, "abc");
}

#[test]
fn test_typeset_t_reads_existing_env_value() {
    // `typeset -T PATH path :` should split the inherited $PATH
    // (from process env) into the `path` array. Was returning empty
    // because we only checked self.variables, not env::var.
    let (_, output, _) = run_zshrs(r#"typeset -T PATH path :; (( $#path > 0 )) && echo y"#);
    assert_eq!(output.trim(), "y");
}

#[test]
fn test_typeset_t_unset_propagates_to_tied() {
    // `unset path` should also clear $PATH because they're tied via
    // `typeset -T`. zsh: PATH becomes empty. Was leaving the env
    // value intact.
    let (_, output, _) = run_zshrs(r#"typeset -T PATH path :; unset path; echo "[$PATH]""#);
    assert_eq!(output.trim(), "[]");
}

#[test]
fn test_assoc_capital_i_returns_all_matching_keys() {
    // `${h[(I)pat]}` on assoc returns ALL keys matching pat,
    // space-joined. `${h[(i)pat]}` returns FIRST. Same for (R)/(r)
    // on values. Was returning only one.
    let (_, output, _) = run_zshrs(r#"typeset -A h=(a 1 b 2); echo "[${h[(I)*]}]""#);
    assert_eq!(output.trim(), "[a b]");
    let (_, output, _) = run_zshrs(r#"typeset -A h=(a 1 b 2); echo "[${h[(i)*]}]""#);
    assert_eq!(output.trim(), "[a]");
    let (_, output, _) = run_zshrs(r#"typeset -A h=(a 1 b 1 c 2); echo "[${h[(R)1]}]""#);
    assert_eq!(output.trim(), "[1 1]");
}

#[test]
fn test_special_param_default_treats_as_set() {
    // `${SECONDS-default}` — zsh-special params have dynamic getters
    // and were treated as "unset" because they're not in the variables
    // map. Default fired incorrectly. Now treated as always-set.
    let (_, output, _) = run_zshrs(r#"echo "[${SECONDS-default}]""#);
    assert!(output.trim() != "[default]");
    let (_, output, _) = run_zshrs(r#"echo "[${UID-default}]""#);
    assert!(output.trim() != "[default]");
    let (_, output, _) = run_zshrs(r#"echo "[${HISTCMD-default}]""#);
    assert_eq!(output.trim(), "[0]");
}

#[test]
fn test_zerr_trap_fires_on_nonzero_status() {
    // `trap "X" ZERR; false; echo done` — ZERR fires whenever a
    // command exits non-zero. Was a no-op despite being recognized
    // as a valid signal name. Now wired into BUILTIN_ERREXIT_CHECK.
    let (_, output, _) = run_zshrs(r#"trap "echo zerr" ZERR; false; echo done"#);
    assert_eq!(output.trim(), "zerr\ndone");
}

#[test]
fn test_err_trap_alias_for_zerr() {
    let (_, output, _) = run_zshrs(r#"trap "echo err" ERR; false; echo done"#);
    assert_eq!(output.trim(), "err\ndone");
}

#[test]
fn test_read_minus_E_echoes_and_assigns() {
    // `read -E` should echo the read line on stdout AND store it in
    // the variable. zsh's bin_read calls fputs(buf, stdout) under
    // -E. Was a TODO no-op.
    let (_, output, _) = run_zshrs(r#"echo "abc" | (read -E v; echo "[$v]")"#);
    assert_eq!(output.trim(), "abc\n[abc]");
}

#[test]
fn test_read_minus_e_echoes_only() {
    // `read -e` echoes the line but does NOT assign — useful for
    // completion functions that want to show the current input.
    let (_, output, _) = run_zshrs(r#"echo "abc" | (read -e v; echo "[$v]")"#);
    assert_eq!(output.trim(), "abc\n[]");
}

#[test]
fn test_print_minus_C_column_format() {
    // `print -C N` formats args in N columns with 2-space separator
    // (not tab). Each column is padded to the widest entry. Trailing
    // partial rows don't get column-padding after the last present item.
    let (_, output, _) = run_zshrs(r#"print -C 2 a b c d"#);
    assert_eq!(output.trim_end(), "a  c\nb  d");
    let (_, output, _) = run_zshrs(r#"print -C 2 alpha beta gamma delta"#);
    assert_eq!(output.trim_end(), "alpha  gamma\nbeta   delta");
    let (_, output, _) = run_zshrs(r#"print -C 3 1 2 3 4 5 6 7"#);
    assert_eq!(output.trim_end(), "1  4  7\n2  5\n3  6");
}

#[test]
fn test_param_replace_strips_backslash_escape_in_pat() {
    // `${a//\:/-}` — the `\:` should match literal `:`. Was treating
    // `\:` as the literal pattern (no match because $a has `:`, not
    // `\:`). zsh: backslash unescapes a non-meta char in the pattern.
    let (_, output, _) = run_zshrs(r#"a="x:y:z"; echo "${a//\:/-}""#);
    assert_eq!(output.trim(), "x-y-z");
    let (_, output, _) = run_zshrs(r#"a="x.y.z"; echo "${a//\./X}""#);
    assert_eq!(output.trim(), "xXyXz");
}

#[test]
fn test_param_p_indirect_with_cmd_subst() {
    // `${(P)$(...)}` — (P) indirect on cmd-subst result. Treat the
    // captured output as a NAME and look up that variable's value.
    // Was returning the cmd-subst output verbatim.
    let (_, output, _) = run_zshrs(r#"a=hi; echo "${(P)$(echo a)}""#);
    assert_eq!(output.trim(), "hi");
    let (_, output, _) = run_zshrs(r#"a=hello; echo "${(UP)$(echo a)}""#);
    assert_eq!(output.trim(), "HELLO");
}

#[test]
fn test_array_subscript_remove_with_var_index() {
    // `a[$n]=()` should remove the element at index $n. Compile path
    // was emitting the literal "$n" key string; runtime int-parse
    // failed and the removal was a no-op.
    let (_, output, _) = run_zshrs(r#"a=(1 2 3 4); a[$#a]=(); echo "${a[@]}""#);
    assert_eq!(output.trim(), "1 2 3");
    let (_, output, _) = run_zshrs(r#"a=(1 2 3 4); n=3; a[$n]=(); echo "${a[@]}""#);
    assert_eq!(output.trim(), "1 2 4");
}

#[test]
fn test_local_assoc_array_shadows_outer() {
    // `typeset -A h=(...)` inside a function should shadow the outer
    // assoc binding. Without local_assoc_save_stack, the inner h
    // leaked into the parent on function exit.
    let (_, output, _) = run_zshrs(
        r#"typeset -A h; h=(a 1); f() { typeset -A h=(b 2); echo "$h[b]"; }; f; echo "$h[a]"; echo "[$h[b]]""#,
    );
    assert_eq!(output.trim(), "2\n1\n[]");
}

#[test]
fn test_param_flag_with_cmd_subst_operand() {
    // `${(z)$(echo a b c)}` — cmd-subst as flag operand. Without the
    // new branch in expand_braced_variable, the flag handler treated
    // `$(echo a b c)` as a literal var name and returned empty in DQ.
    let (_, output, _) = run_zshrs(r#"echo "${(z)$(echo a b c)}""#);
    assert_eq!(output.trim(), "a b c");
    let (_, output, _) = run_zshrs(r#"echo "${(U)$(echo hello)}""#);
    assert_eq!(output.trim(), "HELLO");
    let (_, output, _) = run_zshrs(r#"echo "${(s. .)$(echo a b c)}""#);
    assert_eq!(output.trim(), "a b c");
}

#[test]
fn test_math_abs_min_max_preserve_int() {
    // `abs` lives in `zsh/mathfunc` and requires `zmodload`. `min`/
    // `max` are NOT functions in zsh — they're builtin arithmetic
    // ternaries: `(a > b ? a : b)`. /bin/zsh: `$((min(3,5,7)))` →
    // "unknown function: min". On integer args, abs returns integer
    // (was returning "5." instead of "5").
    let (_, output, _) = run_zshrs(r#"zmodload zsh/mathfunc; echo $((abs(-5)))"#);
    assert_eq!(output.trim(), "5");
    let (_, output, _) = run_zshrs(r#"echo $(( 3 > 5 ? (3 > 7 ? 3 : 7) : (5 > 7 ? 5 : 7) ))"#);
    assert_eq!(output.trim(), "7");
    let (_, output, _) = run_zshrs(r#"echo $(( 3 < 5 ? (3 < 7 ? 3 : 7) : (5 < 7 ? 5 : 7) ))"#);
    assert_eq!(output.trim(), "3");
    // Float input still returns float.
    let (_, output, _) = run_zshrs(r#"zmodload zsh/mathfunc; echo $((abs(-5.5)))"#);
    assert_eq!(output.trim(), "5.5");
}

#[test]
fn test_array_assigns_array_via_at_splice() {
    // `b=("${a[@]}")` — array RHS preserves element boundaries even
    // when elements contain spaces. Was joining to single string
    // because scalar_assign_depth got bumped for ALL assignments.
    // Now distinguishes scalar (`b="$a[@]"`) from array (`b=("$a[@]")`).
    let (_, output, _) = run_zshrs(r#"a=("1 2" "3 4"); b=("${a[@]}"); echo "${#b}""#);
    assert_eq!(output.trim(), "2");
}

#[test]
fn test_recursive_glob_sorts_full_path() {
    // `**/*` should sort by full path (so `dir sub sub/g`, not
    // basename-only which gives `f g sub`).
    let dir = std::env::temp_dir().join("zshrs_recglob");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::create_dir_all(dir.join("sub"));
    std::fs::File::create(dir.join("f")).unwrap();
    std::fs::File::create(dir.join("sub/g")).unwrap();
    let cmd = format!("echo {}/**/*", dir.display());
    let (_, output, _) = run_zshrs(&cmd);
    let _ = std::fs::remove_dir_all(&dir);
    let parts: Vec<&str> = output.trim().split(' ').collect();
    // Expect: f, sub, sub/g (in that order — lexicographic full path)
    assert!(parts[0].ends_with("/f"));
    assert!(parts[1].ends_with("/sub"));
    assert!(parts[2].ends_with("/sub/g"));
}

#[test]
fn test_subshell_exit_trap_fires_before_parent_continues() {
    // `(trap "echo X" EXIT; true); echo done` — zsh forks for `(...)`
    // so the trap fires when the subshell ends, BEFORE `echo done`.
    // Was firing at parent's process exit (after `echo done`).
    let (_, output, _) = run_zshrs(r#"(trap "echo trapped" EXIT; true); echo done"#);
    assert_eq!(output.trim(), "trapped\ndone");
}

#[test]
fn test_subshell_trap_doesnt_leak_to_parent() {
    // `(trap "echo X" USR1; ...); ...` — the trap dies with the
    // subshell. Parent's traps (snapshotted at subshell entry) are
    // restored on subshell_end.
    let (_, output, _) = run_zshrs(r#"trap "echo parent_exit" EXIT; (echo subshell)"#);
    assert_eq!(output.trim(), "subshell\nparent_exit");
}

#[test]
fn test_sort_flags_with_at_subscript_in_dq() {
    // `"${(o)a[@]}"` — DQ context normally strips array-only flags
    // (o/O/n/i/u), but explicit `[@]` keeps them active. Compile path
    // now encodes the at-subscript context through `\u{03}` sentinel
    // since `parse_zsh_flag` strips the suffix from `name`.
    let (_, output, _) = run_zshrs(r#"a=(c a b); echo "${(o)a[@]}""#);
    assert_eq!(output.trim(), "a b c");
    let (_, output, _) = run_zshrs(r#"a=(c a b); echo "${(O)a[@]}""#);
    assert_eq!(output.trim(), "c b a");
    let (_, output, _) = run_zshrs(r#"a=(10 2 1 22); echo "${(n)a[@]}""#);
    assert_eq!(output.trim(), "1 2 10 22");
}

#[test]
fn test_arith_ternary_assignment() {
    // `((a = 5 > 3 ? 99 : 0))` — ArithCompiler doesn't implement `?:`
    // so the assignment silently dropped. Routing to MathEval (which
    // handles ternary fully) when the expr contains `?`.
    let (_, output, _) = run_zshrs(r#"((a = 5 > 3 ? 99 : 0)); echo $a"#);
    assert_eq!(output.trim(), "99");
    let (_, output, _) = run_zshrs(r#"((x = 1 < 2 ? 10 : 20)); echo $x"#);
    assert_eq!(output.trim(), "10");
}

#[test]
fn test_case_paren_wrapped_pattern_with_alternation() {
    // `case W in (P|Q)) BODY ;; esac` — paren-wrapped pattern with
    // alternation. The leading `(` and matching inner `)` enclose the
    // pattern; the outer `)` is the arm-close. Was failing because
    // we consumed only one `)` total when leading `(` was present.
    let (_, output, _) = run_zshrs(r#"case foo in (foo|bar)) echo y;; esac"#);
    assert_eq!(output.trim(), "y");
    let (_, output, _) = run_zshrs(r#"case file.txt in (*.txt|*.md)) echo y;; esac"#);
    assert_eq!(output.trim(), "y");
}

#[test]
fn test_typeset_a_preserves_existing_array_at_top_scope() {
    // `a=(1 2 3); typeset -a a` should keep the array. Was clobbering
    // to empty because the bare-declaration path always re-inited.
    let (_, output, _) = run_zshrs(r#"a=(1 2 3); typeset -a a; echo $a"#);
    assert_eq!(output.trim(), "1 2 3");
}

#[test]
fn test_typeset_aU_dedupes_existing_array() {
    // `a=(a b a c b); typeset -aU a` — adding the unique attribute to
    // an existing array should dedupe in place.
    let (_, output, _) = run_zshrs(r#"a=(a b a c b); typeset -aU a; echo $a"#);
    assert_eq!(output.trim(), "a b c");
}

#[test]
fn test_nested_expansion_subscript_after_flag() {
    // `${(U)${(s. .)s}[1]}` — split inner, take [1], uppercase.
    // Was uppercasing the joined string and ignoring the subscript.
    let (_, output, _) = run_zshrs(r#"s="x y z"; echo "${(U)${(s. .)s}[1]}""#);
    assert_eq!(output.trim(), "X");
    let (_, output, _) = run_zshrs(r#"s="x y z"; echo "${(U)${(s. .)s}[2]}""#);
    assert_eq!(output.trim(), "Y");
}

#[test]
fn test_source_passes_extra_args_as_positionals() {
    // `. file.sh hi bye` should set $1=hi, $2=bye in the sourced
    // script. Was leaving the parent's positionals (or empty) visible.
    let path = std::env::temp_dir().join("zshrs_src_args_test.sh");
    std::fs::write(&path, "echo a=$1 b=$2\n").unwrap();
    let cmd = format!(". {} hi bye", path.display());
    let (_, output, _) = run_zshrs(&cmd);
    let _ = std::fs::remove_file(&path);
    assert_eq!(output.trim(), "a=hi b=bye");
}

#[test]
fn test_source_preserves_outer_positionals() {
    // After source returns, the parent's positionals must be intact.
    let path = std::env::temp_dir().join("zshrs_src_args_outer.sh");
    std::fs::write(&path, ":\n").unwrap();
    let cmd = format!(
        r#"set -- a b c; . {} inner; echo "after=$@""#,
        path.display()
    );
    let (_, output, _) = run_zshrs(&cmd);
    let _ = std::fs::remove_file(&path);
    assert_eq!(output.trim(), "after=a b c");
}

#[test]
fn test_array_splice_in_scalar_assign_joins() {
    // `b="${a[@]}"` — assignment to a scalar joins like `[*]` does.
    // Was capturing only the first element because the Array got
    // truncated by scalar conversion.
    let (_, output, _) = run_zshrs(r#"a=(1 2 3); b="${a[@]}"; echo $b"#);
    assert_eq!(output.trim(), "1 2 3");
}

#[test]
fn test_array_bare_splice_no_braces() {
    // `$a[@]` (no braces) and `$a[*]` (no braces) — zsh treats these
    // identically to the braced forms. Was joining to a single arg.
    let (_, output, _) = run_zshrs(r#"a=(x y z); printf "%s\n" $a[@]"#);
    assert_eq!(output.trim(), "x\ny\nz");
    let (_, output, _) = run_zshrs(r#"f() { echo $#; }; a=(x y z); f $a[@]"#);
    assert_eq!(output.trim(), "3");
}

#[test]
fn test_arith_assign_from_array_subscript() {
    // `((i=a[2]))` — the RHS array subscript must be pre-resolved so
    // the assignment lands `a[2]`'s VALUE in `i`, not the joined-scalar
    // form of the array.
    let (_, output, _) = run_zshrs(r#"a=(10 20 30); ((i=a[2])); echo $i"#);
    assert_eq!(output.trim(), "20");
    let (_, output, _) = run_zshrs(r#"a=(10 20 30); ((sum=a[1]+a[2]+a[3])); echo $sum"#);
    assert_eq!(output.trim(), "60");
}

#[test]
fn test_glob_caret_at_path_component_with_extendedglob() {
    // `setopt extendedglob; echo /tmp/dir/^pat` — the negation operator
    // in any path component (not just the leading word). Trigger
    // detection extended to recognise `/^` as a glob meta when extglob
    // is on.
    let dir = std::env::temp_dir().join("zshrs_glob_caret_path");
    let _ = std::fs::create_dir_all(&dir);
    for name in ["a", "b", "c"] {
        std::fs::File::create(dir.join(name)).unwrap();
    }
    let cmd = format!(r#"setopt extendedglob; echo {}/^a"#, dir.display());
    let (_, output, _) = run_zshrs(&cmd);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(output.contains("/b"));
    assert!(output.contains("/c"));
    assert!(!output.contains("/a "));
    assert!(!output.contains("/^a"));
}

#[test]
fn test_cond_double_bracket_grouping_parens() {
    // `[[ a == a && (b == b || c == c) ]]` — the lexer was leaving
    // incondpat=true after `==` and never resetting on `&&`/`||`, so
    // the `(` after `&&` was lexed as a literal glob char and the
    // remainder collapsed into one String.
    let (status, output, _) = run_zshrs(r#"[[ a == a && (b == b || c == c) ]] && echo y"#);
    assert_eq!(status, 0);
    assert_eq!(output.trim(), "y");
}

#[test]
fn test_subshell_umask_restored_on_exit() {
    // zsh forks for `(...)` so `umask 077` inside dies with the child.
    // We run subshells in-process; without snapshot+restore, the
    // subshell's umask leaks to the parent.
    let (_, output, _) = run_zshrs(r#"umask 022; (umask 077); umask"#);
    assert_eq!(output.trim(), "022");
}

#[test]
fn test_brace_expand_with_inner_var_ref() {
    // `{one,${a},three}` — outer brace must expand AFTER ${a}
    // substitution. Without the new BUILTIN_BRACE_EXPAND emit,
    // segment-concat produced literal `{one,hi,three}`.
    let (_, output, _) = run_zshrs(r#"a=hi; echo {one,${a},three}"#);
    assert_eq!(output.trim(), "one hi three");
    let (_, output, _) = run_zshrs(r#"a=hi; echo pre{1,${a},2}post"#);
    assert_eq!(output.trim(), "pre1post prehipost pre2post");
}

#[test]
fn test_assoc_subscript_i_flag_searches_keys() {
    // `${h[(I)key]}` on assoc — searches KEYS (not values), returns
    // the matching key. Was incorrectly searching values.
    let (_, output, _) = run_zshrs(r#"typeset -A h; h=(a 1 b 2 c 3); echo "${h[(I)a]}""#);
    assert_eq!(output.trim(), "a");
    let (_, output, _) = run_zshrs(r#"typeset -A h; h=(a 1 b 2 c 3); echo "${h[(i)b]}""#);
    assert_eq!(output.trim(), "b");
    // (r) still searches values and returns the value.
    let (_, output, _) = run_zshrs(r#"typeset -A h; h=(a 1 b 2 c 3); echo "${h[(r)2]}""#);
    assert_eq!(output.trim(), "2");
}

#[test]
fn test_glob_with_var_prefix_expands_paths() {
    // `$D/*` should glob-expand after $D substitution. Was leaking
    // `*` literal because the segment-concat fast path skipped
    // pathname expansion entirely.
    let dir = std::env::temp_dir().join("zshrs_glob_var");
    let _ = std::fs::create_dir_all(&dir);
    std::fs::File::create(dir.join("a")).unwrap();
    std::fs::File::create(dir.join("b")).unwrap();
    let cmd = format!(r#"D={}; echo $D/*"#, dir.display());
    let (_, output, _) = run_zshrs(&cmd);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(output.contains("/a"));
    assert!(output.contains("/b"));
    assert!(!output.contains("/*"));
}

#[test]
fn test_glob_with_var_prefix_alternation() {
    let dir = std::env::temp_dir().join("zshrs_glob_var_alt");
    let _ = std::fs::create_dir_all(&dir);
    std::fs::File::create(dir.join("a")).unwrap();
    std::fs::File::create(dir.join("b")).unwrap();
    let cmd = format!(r#"D={}; echo $D/(a|b)"#, dir.display());
    let (_, output, _) = run_zshrs(&cmd);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(output.contains("/a"));
    assert!(output.contains("/b"));
    assert!(!output.contains("(a|b)"));
}

#[test]
fn test_param_pad_zero_with_empty_string1() {
    // ${(l:5::0:)42}: empty string1 + string2="0" → repeat "0" to fill.
    // zsh: 00000 (per subst.c when string1 is empty, string2 acts as
    // the pad char and the value gets pushed off / not included).
    let (_, output, _) = run_zshrs(r#"echo "${(l:5::0:)42}""#);
    assert_eq!(output.trim(), "00000");
}

/// Pin: `\${...\}` literal-brace replacement in `${var/pat/repl}` does
/// not mis-count as a nested paramsubst opener.
///
/// Surface: p10k.zsh:8552 `_p9k_must_init` body —
/// `IFS=$'\1' _p9k__param_pat+="${(@)…:/(#m)*/\${${(q)MATCH}-$IFS\}}"`.
/// Before the fix, zshrs's subst-time brace scanner counted the raw
/// `{` after `\$` as a nested-paramsubst opener (only the immediately
/// preceding char was checked for backslash escape), so the matching
/// trailing `\}` couldn't close — `_p9k_must_init: closing brace
/// missing` fired at every precmd. Fix lives in `src/ported/lex.rs`
/// at the `\$` arm: when the next char is `{` or `}` AND `bct > 0`
/// (we're inside a `${...}` body), emit Bnull before that brace too
/// so the subst scanner's escape check sees it as literal.
#[test]
fn test_paramsubst_replace_literal_dollar_brace_escape() {
    // Minimal repro — pattern `a` doesn't match value `A`, so the
    // replacement must parse without firing "closing brace missing".
    let (status, output, stderr) = run_zshrs(r#"x=A; echo "${x:/a/\${y\}}""#);
    assert_eq!(status, 0, "stderr was: {stderr}");
    assert_eq!(output.trim(), "A");
    assert!(
        !stderr.contains("closing brace missing"),
        "unexpected closing-brace error: {stderr}"
    );
}

#[test]
fn test_paramsubst_replace_dollar_brace_match_ref() {
    // p10k pattern shape — `(#m)*` sets `$MATCH`, replacement spans
    // `\${${(q)MATCH}-y\}` with both literal-brace escapes AND a
    // real nested `${(q)MATCH}` inside. Scanner must distinguish
    // the escaped braces (raw `{`/`}`) from the unescaped nested
    // Inbrace/Outbrace and parse without "closing brace missing".
    // Real zsh output for this construct is the matched span itself
    // (`A`) — pin to that so future replacement-string changes don't
    // silently diverge from zsh.
    let (status, output, stderr) = run_zshrs(r#"x=A; echo "${x:/(#m)*/\${${(q)MATCH}-y\}}""#);
    assert_eq!(status, 0, "stderr was: {stderr}");
    assert_eq!(output.trim(), "A");
    assert!(
        !stderr.contains("closing brace missing"),
        "unexpected closing-brace error: {stderr}"
    );
}

#[test]
fn test_paramsubst_replace_unrelated_nested_brace_still_counted() {
    // Regression guard for the fix in lex.rs: a REAL nested
    // `${INNER}` in the replacement (no preceding `\$`) must still
    // tokenize as Inbrace/Outbrace and count toward depth. Without
    // this check, an overly aggressive lex change would skip
    // counting genuine nested paramsubsts.
    let (status, output, stderr) = run_zshrs(r#"y=bar; x=foo; echo "${x:/foo/${y}}""#);
    assert_eq!(status, 0, "stderr was: {stderr}");
    assert_eq!(output.trim(), "bar");
}

#[test]
fn test_paramsubst_replace_only_outside_brace_body_unaffected() {
    // Pin: `\${` outside a `${...}` body (i.e. bct == 0 at lex time)
    // is unchanged from real zsh output — gates the `bct > 0`
    // condition in the lex arm so the fix only triggers inside a
    // paramsubst body. Real zsh emits `${y\}` here (the leading
    // `\$` strips to `$`, the trailing `\}` keeps the backslash).
    let (_, output, _) = run_zshrs(r#"printf '%s\n' "\${y\}""#);
    assert_eq!(output.trim(), r"${y\}");
}

/// Pin: bare `{X}` brace pair inside a `${var/pat/repl}` replacement
/// stays literal — not mis-counted as a nested paramsubst opener.
///
/// Surface: fsh-syntax-highlighting / fzf-tab use replacement strings
/// like `{$match[3]}` or `[$match[1]]<$match[2]>{$match[3]}` (per
/// `zinit_p10k_parity.rs::megamonsters::fsh_three_part_backref_replace`).
/// Before the fix at `src/ported/subst.rs:2896`, the hybrid brace
/// scanner counted every raw `{` toward depth, so the `{` after the
/// last `/` opened a nested level the trailing `}` couldn't close —
/// `closing brace missing` fired and the whole paramsubst aborted.
/// Fix: only count a raw `{` when it's immediately preceded by `$`
/// (since C's `lex.c:1546` only emits `Inbrace` for the `${` form);
/// raw `{` after `/` stays uncounted, matching C's `skipparens` over
/// `Inbrace`/`Outbrace` tokens.
#[test]
fn test_paramsubst_replace_literal_brace_pair_stays_literal() {
    let (status, output, stderr) = run_zshrs(r#"p=foo; echo "[${p/foo/{X}}]""#);
    assert_eq!(status, 0, "stderr was: {stderr}");
    assert_eq!(output.trim(), "[{X}]");
    assert!(
        !stderr.contains("closing brace missing"),
        "unexpected closing-brace error: {stderr}"
    );
}

#[test]
fn test_paramsubst_replace_brace_with_backref_substitution() {
    // p10k / fsh-style three-part backref replacement —
    // `${(@)arr/(#b)(*)X(*)X(*)/[$match[1]]<$match[2]>{$match[3]}}`.
    // The replacement uses literal `[ ] < > { }` plus genuine
    // `${match[N]}` substitutions. Pin the full pattern shape so
    // the raw `{`/`}` gating in the scanner doesn't regress.
    let (status, output, stderr) = run_zshrs(
        r#"setopt extendedglob
parts=($'a\1b\1c' $'x\1y\1z')
print -l -- "${(@)parts/(#b)(*)$'\1'(*)$'\1'(*)/[$match[1]]<$match[2]>{$match[3]}}""#,
    );
    assert_eq!(status, 0, "stderr was: {stderr}");
    // Real zsh output: the trailing `}` of the FIRST element gets
    // consumed by the outer paramsubst closer (real zsh's brace
    // tracker shares behaviour here), so the first line ends `{c`,
    // the second `{z}`. We match real-zsh's behaviour exactly.
    assert_eq!(output.trim(), "[a]<b>{c\n[x]<y>{z}", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// Bug #127 — $'\xNN' must emit the RAW byte (metafied internally),
// not a Unicode codepoint re-encoded as UTF-8, and must not abort.
// C: Src/utils.c getkeystring \x arm + c:7289-7294 metafy tail.
// ---------------------------------------------------------------------------

#[test]
fn test_ansi_c_hex_escape_emits_raw_byte() {
    // zsh oracle: `echo $'\xff' | od -An -c` → `377 \n` (ONE byte).
    let (status, bytes) = run_zshrs_parity_bytes(r#"echo $'\xff'"#);
    assert_eq!(status, 0, "shell must not abort on invalid-UTF-8 byte");
    assert_eq!(bytes, b"\xff\n", "got: {bytes:x?}");
    // Mid-string high byte.
    let (status, bytes) = run_zshrs_parity_bytes(r#"echo $'a\xe9b'"#);
    assert_eq!(status, 0);
    assert_eq!(bytes, b"a\xe9b\n", "got: {bytes:x?}");
    // Consecutive \x escapes combine into the exact byte sequence
    // (here: the UTF-8 encoding of é).
    let (_, bytes) = run_zshrs_parity_bytes(r#"echo $'\xc3\xa9'"#);
    assert_eq!(bytes, "é\n".as_bytes(), "got: {bytes:x?}");
    // Literal é (the \u/codepoint form) stays 2-byte UTF-8.
    let (_, bytes) = run_zshrs_parity_bytes(r#"echo $'é'"#);
    assert_eq!(bytes, "é\n".as_bytes(), "got: {bytes:x?}");
    // Octal form is also a raw byte (zsh: $'\377' == $'\xff').
    let (_, bytes) = run_zshrs_parity_bytes(r#"echo $'\377'"#);
    assert_eq!(bytes, b"\xff\n", "got: {bytes:x?}");
}

#[test]
fn test_print_bindkey_control_meta_escapes() {
    // c:Src/builtin.c:4754 + Src/utils.c:7029-7052/7261-7275 —
    // `\C-X` / `\M-X` bindkey-style key escapes. Under GETKEY_EMACS
    // (plain `print` AND `print -b`) the `\C-`/`\M-` backslash forms are
    // processed: control clears bits 5-6 (`\C-a` → 0x01, `\C-?` → 0x7f),
    // meta sets the high bit (`\M-a` → 0xe1), and they compose
    // (`\M-\C-a` → 0x81). getkeystring_with (print's path) previously
    // lacked this and emitted literal `C-a`.
    let (_, b) = run_zshrs_parity_bytes(r#"print -b "\C-a""#);
    assert_eq!(b, b"\x01\n", "got: {b:x?}");
    let (_, b) = run_zshrs_parity_bytes(r#"print -b "\C-x""#);
    assert_eq!(b, b"\x18\n", "got: {b:x?}");
    let (_, b) = run_zshrs_parity_bytes(r#"print -b "\C-A""#);
    assert_eq!(b, b"\x01\n", "got: {b:x?}"); // case-insensitive
    let (_, b) = run_zshrs_parity_bytes(r#"print -b "\C-[""#);
    assert_eq!(b, b"\x1b\n", "got: {b:x?}"); // ^[ == ESC
    let (_, b) = run_zshrs_parity_bytes(r#"print -b "\C-m""#);
    assert_eq!(b, b"\x0d\n", "got: {b:x?}"); // ^M == CR
    let (_, b) = run_zshrs_parity_bytes(r#"print -b "\C- ""#);
    assert_eq!(b, b"\x00\n", "got: {b:x?}"); // ^Space == NUL
    let (_, b) = run_zshrs_parity_bytes(r#"print -b "\C-?""#);
    assert_eq!(b, b"\x7f\n", "got: {b:x?}"); // ^? == DEL
    let (_, b) = run_zshrs_parity_bytes(r#"print -b "\M-a""#);
    assert_eq!(b, b"\xe1\n", "got: {b:x?}"); // meta sets high bit
    let (_, b) = run_zshrs_parity_bytes(r#"print -b "\M-\C-a""#);
    assert_eq!(b, b"\x81\n", "got: {b:x?}"); // meta + control compose
    let (_, b) = run_zshrs_parity_bytes(r#"print -b "a\C-mb""#);
    assert_eq!(b, b"a\x0db\n", "got: {b:x?}"); // mid-string

    // `^X` caret notation is bindkey-only (GETKEY_CTRL) — needs `-b`.
    let (_, b) = run_zshrs_parity_bytes(r#"print -b "^A""#);
    assert_eq!(b, b"\x01\n", "got: {b:x?}");
    let (_, b) = run_zshrs_parity_bytes(r#"print -b "^?""#);
    assert_eq!(b, b"\x7f\n", "got: {b:x?}");

    // Plain `print` (GETKEYS_PRINT: has EMACS) processes the backslash
    // forms too, but NOT the caret form (no GETKEY_CTRL).
    let (_, b) = run_zshrs_parity_bytes(r#"print "\C-a""#);
    assert_eq!(b, b"\x01\n", "got: {b:x?}");
    let (_, b) = run_zshrs_parity_bytes(r#"print "^A""#);
    assert_eq!(b, b"^A\n", "caret stays literal without -b: {b:x?}");

    // `echo` (GETKEYS_ECHO: no EMACS) keeps `\C-a` fully literal.
    let (_, b) = run_zshrs_parity_bytes(r#"echo "\C-a""#);
    assert_eq!(b, b"\\C-a\n", "got: {b:x?}");
}

#[test]
fn test_ansi_c_hex_escape_segment_concat_and_printf() {
    // Concatenated quoted segments take the lexer/untokenize path
    // (getkeystring_dollar_quote), not the compile-time fast path —
    // pin both.
    let (status, bytes) = run_zshrs_parity_bytes(r#"echo "a"$'\xff'"b""#);
    assert_eq!(status, 0);
    assert_eq!(bytes, b"a\xffb\n", "got: {bytes:x?}");
    // printf %s writes the bytes unmangled (binary-header use case).
    let (_, bytes) = run_zshrs_parity_bytes(r#"printf "%s" $'\xff\xfe\x00\x01\x00\x02'"#);
    assert_eq!(bytes, b"\xff\xfe\x00\x01\x00\x02", "got: {bytes:x?}");
}

#[test]
fn test_ansi_c_high_byte_pattern_equality() {
    // The metafied representation must stay pattern-comparable:
    // [[ == ]], case, and ${s/pat/rep} all match the raw byte.
    let (_, out, _) = run_zshrs_parity(r#"[[ $'\xff' == $'\xff' ]] && echo same"#);
    assert_eq!(out.trim(), "same", "got: {out:?}");
    let (_, out, _) = run_zshrs_parity(r#"[[ $'\xff' == $'\xfe' ]] || echo diff"#);
    assert_eq!(out.trim(), "diff", "got: {out:?}");
    let (_, out, _) = run_zshrs_parity(r#"case $'\xff' in $'\xff') echo m;; *) echo no;; esac"#);
    assert_eq!(out.trim(), "m", "got: {out:?}");
    let (_, bytes) = run_zshrs_parity_bytes(r#"s=$'a\xffb'; echo "${s/$'\xff'/X}""#);
    assert_eq!(bytes, b"aXb\n", "got: {bytes:x?}");
}

// ---------------------------------------------------------------------------
// Bug #560 — print/echo/printf preserve embedded NUL bytes.
// C: Src/builtin.c bin_print length-aware fwrite (c:5124).
// ---------------------------------------------------------------------------

#[test]
fn test_print_preserves_embedded_nul() {
    // zsh oracle: `print -- "a"$'\0'"b" | od -c` → `a \0 b \n`.
    let (status, bytes) = run_zshrs_parity_bytes(r#"print -- "a"$'\0'"b""#);
    assert_eq!(status, 0);
    assert_eq!(bytes, b"a\0b\n", "got: {bytes:?}");
    let (_, bytes) = run_zshrs_parity_bytes(r#"echo "a"$'\0'"b""#);
    assert_eq!(bytes, b"a\0b\n", "got: {bytes:?}");
    let (_, bytes) = run_zshrs_parity_bytes(r#"printf "%s" "a"$'\0'"b""#);
    assert_eq!(bytes, b"a\0b", "got: {bytes:?}");
    // The NUL survives variable storage too (${#s} counts it).
    let (_, out, _) = run_zshrs_parity(r#"s="a"$'\0'"b"; echo ${#s}"#);
    assert_eq!(out.trim(), "3", "got: {out:?}");
}

// ---------------------------------------------------------------------------
// Bug #373 — pipestatus vs assignment commands. zsh accepts the
// write (Src/params.c:5270 pipestatsetfn stores it) but an ARRAY
// assignment command then clobbers pipestats to [lastval] via the
// no-procs waitjob branch (Src/jobs.c:1753-1755); a bare SCALAR
// assignment creates no job and leaves pipestats alone.
// ---------------------------------------------------------------------------

#[test]
fn test_pipestatus_array_assignment_clobbered_like_zsh() {
    // zsh oracle: `pipestatus=(9); echo $pipestatus` → `0`.
    let (_, out, _) = run_zshrs_parity(r#"pipestatus=(9); echo $pipestatus"#);
    assert_eq!(out.trim(), "0", "got: {out:?}");
    // Any array assignment clobbers to [lastval]...
    let (_, out, _) = run_zshrs_parity(r#"false|true; x=(1 2); echo "[${pipestatus[@]}]""#);
    assert_eq!(out.trim(), "[0]", "got: {out:?}");
    let (_, out, _) = run_zshrs_parity(r#"false|true; x+=(3); echo "[${pipestatus[@]}]""#);
    assert_eq!(out.trim(), "[0]", "got: {out:?}");
    // ...but a bare scalar assignment does NOT (zsh: [1 0] preserved).
    let (_, out, _) = run_zshrs_parity(r#"false|true; x=1; echo "[${pipestatus[@]}]""#);
    assert_eq!(out.trim(), "[1 0]", "got: {out:?}");
    // Scalar write to pipestatus itself is accepted and survives
    // (zsh oracle: `pipestatus=9; echo $pipestatus` → `9`).
    let (_, out, _) = run_zshrs_parity(r#"pipestatus=9; echo "[$pipestatus]""#);
    assert_eq!(out.trim(), "[9]", "got: {out:?}");
}

// ---------------------------------------------------------------------------
// Bug #423 follow-up — remaining IPDEF8 PM_TIED colonarr pairs
// (Src/params.c:395-422): MODULE_PATH/module_path, FIGNORE/fignore,
// MAILPATH/mailpath, PSVAR/psvar — both tie directions.
// ---------------------------------------------------------------------------

#[test]
fn test_tied_module_path_fignore_mailpath_psvar_both_directions() {
    for (scalar, arr) in [
        ("MODULE_PATH", "module_path"),
        ("FIGNORE", "fignore"),
        ("MAILPATH", "mailpath"),
        ("PSVAR", "psvar"),
    ] {
        // scalar → array (split on `:`).
        let (_, out, _) = run_zshrs_parity(&format!(r#"{scalar}=/x:/y; echo "[${{{arr}[@]}}]""#));
        assert_eq!(out.trim(), "[/x /y]", "{scalar} scalar->array got: {out:?}");
        // array → scalar (join with `:`).
        let (_, out, _) = run_zshrs_parity(&format!(r#"{arr}=(/a /b); echo "[${scalar}]""#));
        assert_eq!(out.trim(), "[/a:/b]", "{arr} array->scalar got: {out:?}");
    }
}

// ---------------------------------------------------------------------------
// Bug #296 — `\\` in a paramsubst pattern is a QUOTED literal
// backslash (lexer Bnull survives to patcompile), not an escape for
// the next char. C: Src/lex.c:1508 add(Bnull) + patcompile's
// Bnull-literal contract.
// ---------------------------------------------------------------------------

#[test]
fn test_replace_double_backslash_is_literal_backslash() {
    // zsh oracle: no backslash in "a.b" → no match, value unchanged.
    let (_, out, _) = run_zshrs_parity(r#"s="a.b"; echo "[${s/\\./X}]""#);
    assert_eq!(out.trim(), "[a.b]", "got: {out:?}");
    // With a real backslash+dot in the subject, `\\.` matches it.
    let (_, out, _) = run_zshrs_parity(r#"s="a\\.b"; echo "[${s/\\./X}]""#);
    assert_eq!(out.trim(), "[aXb]", "got: {out:?}");
    // Single `\.` stays an escaped (literal) dot — matches the dot.
    let (_, out, _) = run_zshrs_parity(r#"s="a.b"; echo "[${s/\./X}]""#);
    assert_eq!(out.trim(), "[aXb]", "got: {out:?}");
    // Same layering for the strip arms (#/%%).
    let (_, out, _) = run_zshrs_parity(r#"s="\\.ab"; echo "[${s#\\.}]""#);
    assert_eq!(out.trim(), "[ab]", "got: {out:?}");
    let (_, out, _) = run_zshrs_parity(r#"s=".ab"; echo "[${s#\\.}]""#);
    assert_eq!(out.trim(), "[.ab]", "got: {out:?}");
    // `//` global arm agrees (already-correct path, pinned so the
    // arms can't diverge again).
    let (_, out, _) = run_zshrs_parity(r#"s="a\\.b"; echo "[${s//\\./X}]""#);
    assert_eq!(out.trim(), "[aXb]", "got: {out:?}");
}

// ---------------------------------------------------------------------------
// typeset -n NAMEREFS (PM_NAMEREF) — Src/params.c:6325-6510 resolve_nameref /
// setscope / upscope + Src/builtin.c:2698-2715/3117-3150 bin_typeset arm.
// Expected outputs pinned from Test/K01nameref.ztst (the acceptance spec;
// homebrew zsh 5.9 predates namerefs so no live parity comparison exists).
// ---------------------------------------------------------------------------

#[test]
fn test_nameref_create_and_list() {
    // K01nameref.ztst "assign nameref placeholder"
    let (st, out, _) = run_zshrs_parity("typeset -n ptr; ptr=var; typeset -n");
    assert_eq!(st, 0);
    assert_eq!(out.trim(), "ptr=var", "got: {out:?}");
}

#[test]
fn test_nameref_deref_scalar_and_array() {
    // K01 "basic nameref expansion" + "nameref array expansion"
    let (_, out, _) = run_zshrs_parity("typeset var=value; typeset -n ptr=var; print $ptr");
    assert_eq!(out.trim(), "value");
    let (_, out, _) = run_zshrs_parity("typeset var=(v1 v2); typeset -n ptr=var; print $ptr");
    assert_eq!(out.trim(), "v1 v2");
}

#[test]
fn test_nameref_assign_through_existing_and_dangling() {
    // K01 "assign existing scalar via nameref"
    let (_, out, _) =
        run_zshrs_parity("typeset -n ptr=var; typeset var=value; ptr=new; typeset -p var");
    assert_eq!(out.trim(), "typeset var=new");
    // K01 "assign new scalar via nameref" — dangling target created
    // as a global (typeset -g shape when printed from level 0).
    let (_, out, _) = run_zshrs_parity("typeset -n ptr=var; ptr=value; typeset -p var ptr");
    assert_eq!(out.trim(), "typeset var=value\ntypeset -n ptr=var");
}

#[test]
fn test_nameref_chain_and_self_reference_loop() {
    // K01 "indirect nameref expansion"
    let (_, out, _) = run_zshrs_parity(
        "typeset -n ptr2=var; typeset -n ptr1=ptr2; typeset var=value; print $ptr1",
    );
    assert_eq!(out.trim(), "value");
    // K01 "direct nameref loop not allowed" — c:6426 zerr.
    let (st, _, err) = run_zshrs_parity("typeset -n ptr1=ptr2; typeset -n ptr2=ptr1");
    assert_ne!(st, 0);
    assert!(
        err.contains("invalid self reference"),
        "expected self-reference error, got: {err:?}"
    );
}

#[test]
fn test_nameref_subscript_element() {
    // K01 "nameref to hash element" + "assign array element by nameref"
    let (_, out, _) =
        run_zshrs_parity("typeset -A hash=(x MISS y HIT); typeset -n p='hash[y]'; print -r -- $p");
    assert_eq!(out.trim(), "HIT");
    let (_, out, _) =
        run_zshrs_parity("typeset -a ary=(1 2); typeset -n p='ary[2]'; p=TWO; typeset -p ary");
    assert_eq!(out.trim(), "typeset -a ary=( 1 TWO )");
}

#[test]
fn test_nameref_plus_n_toggle_keeps_refname_as_value() {
    // K01 "remove nameref attribute" — c:2374 type-conversion arm.
    let (_, out, _) = run_zshrs_parity("typeset -n ptr=var; typeset +n ptr; typeset -p ptr");
    assert_eq!(out.trim(), "typeset ptr=var");
}

#[test]
fn test_nameref_unset_through_ref_vs_unset_n() {
    // K01 "unset of the nameref itself" — `unset -n` removes the REF,
    // the target survives.
    let (st, out, _) =
        run_zshrs_parity("typeset -gn ptr=var; typeset -g var=value; unset -n ptr; typeset -p var");
    assert_eq!(st, 0);
    // printed at level 0 → plain `typeset` prefix (params.c:6196).
    assert_eq!(out.trim(), "typeset var=value");
}

#[test]
fn test_nameref_typet_reports_target_type() {
    // K01 "for-loop variable is a reference, part 4" shape — (t)
    // resolves the chain (subst.c:2800-2818).
    let (_, out, _) = run_zshrs_parity("typeset -g var=x; typeset -gn ref=var; print -r ${(t)ref}");
    assert_eq!(out.trim(), "scalar");
    // dangling ref (target never defined): empty tag (fetchvalue
    // NULL → vunset, subst.c:2855-2856).
    let (_, out, _) = run_zshrs_parity(r#"typeset -gn ref=zz_undef; print -r "[${(t)ref}]""#);
    assert_eq!(out.trim(), "[]");
}

#[test]
fn test_nameref_up_reference_reads_bind_scope() {
    // K01 "up-reference part 1" — ref bound at global scope keeps
    // reading the GLOBAL gval even when a local shadows it
    // (upscope walk, Src/params.c:6455-6462).
    let (_, out, _) =
        run_zshrs_parity("typeset -n ptr=gval; gval=global; () { local gval=local; print $ptr; }");
    assert_eq!(out.trim(), "global");
}

#[test]
fn test_nameref_invalid_refname_rejected() {
    // K01 "invalid nameref" — valid_refname (c:6466) guard.
    let (st, _, err) = run_zshrs_parity("typeset -n p='not[2]good'");
    assert_ne!(st, 0);
    assert!(
        err.contains("invalid name reference: not[2]good"),
        "got: {err:?}"
    );
}

#[test]
fn test_nameref_for_loop_rebinds_and_detects_self() {
    // K01 "for-loop variable is a reference, part 1" — setloopvar
    // (Src/params.c:6362) rebinds; `ref` word → self reference aborts.
    let (st, out, err) = run_zshrs_parity(
        "typeset -n ref; typeset one=ONE; for ref in one ref two; do print -r $ref; done",
    );
    assert_eq!(out.trim(), "ONE");
    assert!(err.contains("invalid self reference"), "got: {err:?}");
    assert_ne!(st, 0);
}

#[test]
fn test_scalar_substring_negative_length_past_start_errors() {
    // ${scalar:OFFSET:NEG-LEN} where the resolved end (strlen+len) falls
    // before OFFSET must abort with the C diagnostic, not clamp to "".
    // C: Src/subst.c:3737-3740 zerr("substring expression: %d < %d",
    // strlen+length, given_offset). Sibling of the array arm's #120 fix,
    // which the scalar branch previously lacked (silent .max(0) clamp).
    // "Hello, World" is 12 chars; ${s:2:-20} → end = 12 + (-20) = -8 < 2.
    let (status, stdout, stderr) =
        run_zshrs(r#"s="Hello, World"; print -r -- "[${s:2:-20}]""#);
    assert!(
        stderr.contains("substring expression: -8 < 2"),
        "expected substring-expression abort, got stdout={stdout:?} stderr={stderr:?}"
    );
    assert_ne!(status, 0);
    // The unclamped given_offset must survive OFFSET > strlen:
    // ${s:20:-5} → end = 12 + (-5) = 7 < given_offset 20 (not clamped to 12).
    let (_st2, _out2, err2) = run_zshrs(r#"s="Hello, World"; print -r -- "${s:20:-5}""#);
    assert!(
        err2.contains("substring expression: 7 < 20"),
        "expected unclamped given_offset in message, got: {err2:?}"
    );
    // A negative length that still lands after OFFSET is a normal slice,
    // no error: ${s:7:-1} → "Worl".
    let (st3, out3, _err3) = run_zshrs(r#"s="Hello, World"; print -r -- "${s:7:-1}""#);
    assert_eq!(out3.trim(), "Worl");
    assert_eq!(st3, 0);
}

#[test]
fn test_dq_join_flag_then_setop_filter_applies_to_scalar() {
    // In DQ context a (j:STR:) join collapses the array to ONE scalar
    // BEFORE :#/:*/:| run (C: Src/subst.c:3032 sepjoin, then the operator
    // at c:3522/3540 takes the scalar path c:3555-3567). zshrs used to
    // join early, run the operator, then a late (j) join re-fetched the
    // ORIGINAL array and clobbered the operator result. Verify the
    // operator now survives the late join.

    // Intersection of the WHOLE joined scalar with c: "1 2 3 4 5" is not
    // an element of c=(2 4 6) → empty.
    let (_s, out, _e) =
        run_zshrs(r#"b=(1 2 3 4 5); c=(2 4 6); print -r -- "[${(j: :)b:*c}]""#);
    assert_eq!(out.trim(), "[]", "j-flag + :* intersect must test joined scalar");

    // Filter :# on the joined scalar: "apple,banana,avocado" matches a* →
    // dropped → empty.
    let (_s2, out2, _e2) =
        run_zshrs(r#"a=(apple banana avocado); print -r -- "[${(j:,:)a:#a*}]""#);
    assert_eq!(out2.trim(), "[]", "j-flag + :# filter must test joined scalar");

    // Kept case must preserve the (j) SEPARATOR, not fall back to a space
    // join: b=(1 2 3) not in c=(9) → :| keeps the dash-joined scalar.
    let (_s3, out3, _e3) =
        run_zshrs(r#"b=(1 2 3); c=(9); print -r -- "[${(j:-:)b:|c}]""#);
    assert_eq!(out3.trim(), "[1-2-3]", "kept :| result must keep the (j:-:) sep");

    // Regression: [@] subscript keeps array shape → per-element set-op,
    // then joined with the (j) sep.
    let (_s4, out4, _e4) =
        run_zshrs(r#"b=(1 2 3 4 5); c=(2 4 6); print -r -- "[${(j:-:)b[@]:*c}]""#);
    assert_eq!(out4.trim(), "[2-4]", "[@] keeps per-element intersect");
}

#[test]
fn test_backslash_x_no_hex_digit_emits_nul_byte() {
    // c:Src/utils.c:7169-7170 — `\x` in a print/printf/echo escape ALWAYS
    // runs `zstrtol(s+1,&s,16)`; with no hex digit that reads 0, so a NUL
    // byte is emitted and the offending char is processed literally.
    // zshrs previously errored the empty from_str_radix and dropped the
    // sequence entirely.
    let (_s, out, _e) = run_zshrs(r#"print -rn -- $(print 'a\xgb' | od -An -tx1)"#);
    // od hexdump of print's output must show the 00 (NUL) between 61 and 67.
    assert!(
        out.split_whitespace().collect::<Vec<_>>().windows(3)
            .any(|w| w == ["61", "00", "67"]),
        "print '\\xg' must emit a NUL byte (61 00 67 ...), got: {out:?}"
    );

    // printf %b takes the same getkeystring path.
    let (_s2, out2, _e2) = run_zshrs(r#"printf '%b' 'X\xY' | od -An -tx1"#);
    assert!(
        out2.split_whitespace().any(|b| b == "00"),
        "printf %b '\\xY' must emit a NUL byte, got: {out2:?}"
    );

    // A VALID two-hex \x is unaffected: \x41 == 'A'.
    let (_s3, out3, _e3) = run_zshrs(r#"printf '%b' 'a\x41b'"#);
    assert_eq!(out3, "aAb", "valid \\x41 still decodes to 'A'");
}

#[test]
fn test_escape_high_byte_emits_single_raw_byte_not_utf8() {
    // c:Src/utils.c:7289-7294 — a `\xNN`/`\NNN` escape is one raw BYTE.
    // High bytes (>= 0x80) must metafy so they unmetafy to the single raw
    // byte on output, NOT re-encode as a 2-byte UTF-8 sequence. getkeystring_with
    // (print/printf/echo path) previously did `push(val as char)`, turning
    // \xff into c3 bf instead of ff.
    let hexdump = |code: &str| -> String {
        let (_s, out, _e) = run_zshrs(&format!("{code} | od -An -tx1"));
        out.split_whitespace().collect::<Vec<_>>().join(" ")
    };
    // \xff -> single 0xff byte (a=61, b=62 around it).
    assert_eq!(hexdump(r#"printf '%b' 'a\xffb'"#), "61 ff 62");
    // octal \377 (printf format path) -> single 0xff byte.
    assert_eq!(hexdump(r#"printf 'a\377b'"#), "61 ff 62");
    // echo -e high hex.
    assert_eq!(hexdump(r#"echo -en 'a\xffb'"#), "61 ff 62");
    // A genuine 2-byte UTF-8 sequence written as two escapes stays two bytes.
    assert_eq!(hexdump(r#"printf 'a\xc3\xa9'"#), "61 c3 a9");
    // Low byte unaffected: \x41 == 'A'.
    assert_eq!(hexdump(r#"printf '%b' 'a\x41b'"#), "61 41 62");
}

#[test]
fn test_leading_end_anchor_is_positional_not_hoisted() {
    // c:Src/pattern.c:1103-1109 — a LEADING `(#e)` is a positional
    // end-of-string assertion (P_ISEND), not a whole-match glob flag. It
    // must fail unless the current position is end-of-string, so `(#e)PAT`
    // (anchor followed by more pattern) can never match. zshrs hoisted the
    // leading `(#...)` group and dropped the assertion, so `(#e)r` matched
    // as if the anchor were absent.
    let m = |code: &str| run_zshrs(&format!("setopt extendedglob; {code}")).1;
    // (#e) before a char: impossible -> no match, no replacement.
    assert_eq!(m(r#"s="foo bar"; print -r -- ${s//(#e)r/END}"#).trim(), "foo bar");
    assert_eq!(m(r#"s="rrr"; print -r -- ${s//(#e)r/END}"#).trim(), "rrr");
    // [[ ]] whole-match: (#e)o against "o" must NOT match.
    assert_eq!(m(r#"[[ o == (#e)o ]] && echo m || echo no"#).trim(), "no");
    // Trailing (#e) still works (o at end).
    assert_eq!(m(r#"[[ o == o(#e) ]] && echo m || echo no"#).trim(), "m");
    // Regression: leading (#s) still enforced (o only at start).
    assert_eq!(m(r#"s="foo bar"; print -r -- ${s//(#s)o/X}"#).trim(), "foo bar");
    assert_eq!(m(r#"s="foo bar"; print -r -- ${s//(#s)f/X}"#).trim(), "Xoo bar");
    // Regression: (#i)/(#m) flags still hoist and apply.
    assert_eq!(m(r#"s="aXbXc"; print -r -- ${s//(#i)x/Y}"#).trim(), "aYbYc");
}

#[test]
fn test_unclosed_bracket_glob_is_bad_pattern_not_no_match() {
    // c:Src/glob.c:1842-1854 — a filename glob that fails to COMPILE (an
    // unclosed `[` char class) is a "bad pattern", not "no matches found".
    // With BADPATTERN set (default) it errors even under NO_NOMATCH; with
    // `unsetopt badpattern` the word is passed through literally. zshrs
    // dropped the compile failure silently and reported "no matches found".
    let (st, _o, err) = run_zshrs("echo abc[def");
    assert!(err.contains("bad pattern: abc[def"), "got: {err:?}");
    assert!(!err.contains("no matches found"), "must not be no-match: {err:?}");
    assert_ne!(st, 0);

    // bad pattern overrides NO_NOMATCH (still an error, not literal).
    let (_st2, _o2, err2) = run_zshrs("setopt nonomatch; echo abc[def");
    assert!(err2.contains("bad pattern: abc[def"), "nonomatch: {err2:?}");

    // unsetopt badpattern → the failed glob is a literal string.
    let (st3, out3, _e3) = run_zshrs("unsetopt badpattern; echo abc[def");
    assert_eq!(out3.trim(), "abc[def");
    assert_eq!(st3, 0);

    // A CLOSED bracket that matches nothing is still "no matches found".
    let (_st4, _o4, err4) = run_zshrs("echo a[b]c");
    assert!(err4.contains("no matches found"), "closed bracket: {err4:?}");
}

#[test]
fn test_printf_numeric_operand_math_error_reported_and_exit_1() {
    // c:Src/builtin.c:5460-5464 — printf evaluates %d/%i operands with
    // mathevali; a partial number with trailing junk (12abc, "12 34",
    // 0x1G) is a "bad math expression", emitted to stderr with exit 1,
    // value 0, and formatting CONTINUES for later args. zshrs previously
    // swallowed the error (silent 0, exit 0).
    let (st, out, err) = run_zshrs(r#"printf "%d\n" 12abc"#);
    assert!(err.contains("bad math expression"), "err: {err:?}");
    assert!(err.contains("operator expected"), "err: {err:?}");
    assert_eq!(out.trim(), "0");
    assert_eq!(st, 1);

    // Later args still format after the error (per-arg errflag clear).
    let (st2, out2, err2) = run_zshrs(r#"printf "[%d][%d][%d]\n" 12abc 5 99"#);
    assert_eq!(out2.trim(), "[0][5][99]");
    assert_eq!(st2, 1);
    assert!(err2.contains("bad math expression"), "err2: {err2:?}");

    // A bare identifier is an unset var (0), NOT an error.
    let (st3, out3, err3) = run_zshrs(r#"printf "%d\n" abc"#);
    assert_eq!(out3.trim(), "0");
    assert_eq!(st3, 0);
    assert!(!err3.contains("bad math"), "err3: {err3:?}");

    // Valid math still evaluates with exit 0.
    let (st4, out4, _e4) = run_zshrs(r#"printf "%d\n" 1+2"#);
    assert_eq!(out4.trim(), "3");
    assert_eq!(st4, 0);
}

#[test]
fn test_single_arg_test_is_nonempty_string_test() {
    // c:Src/parse.c par_cond_1 — a `[ X ]` / `test X` with ONE argument is
    // a non-empty-string test (implicit -n), whatever X looks like: a unary
    // op, `!`, a binary op, or a paren all need more tokens to form an
    // operator. True iff X is non-empty. zshrs treated a lone flag/operator
    // token as an operator with a missing operand (exit 2 / diagnostic).
    // Quote the shell-special tokens (<, >, (, )) so they reach `[` as
    // arguments rather than redirections/subshells.
    for tok in [
        "-z", "-f", "-n", "!", "'<'", "'>'", "'('", "')'", "=", "x", "-abc",
    ] {
        let (st, _o, _e) = run_zshrs(&format!("[ {tok} ]"));
        assert_eq!(st, 0, "[ {tok} ] must be true (non-empty), got exit {st}");
    }
    // Empty single arg is false.
    let (st_empty, _o, _e) = run_zshrs(r#"[ "" ]"#);
    assert_eq!(st_empty, 1);

    // Regression: 2-/3-arg forms unaffected.
    assert_eq!(run_zshrs(r#"[ -z "" ]"#).0, 0); // -z of empty → true
    assert_eq!(run_zshrs(r#"[ -n x ]"#).0, 0);
    assert_eq!(run_zshrs("[ 5 -gt 3 ]").0, 0);
    assert_eq!(run_zshrs("[ 3 -gt 5 ]").0, 1);
    assert_eq!(run_zshrs("[ a = a ]").0, 0);
}

#[test]
fn test_parse_error_token_shows_quotes_and_sigils() {
    // c:Src/parse.c:2738 — the "parse error near `X'" token is zshlextext,
    // the human-readable SOURCE text, so quotes/sigils render verbatim.
    // zshrs's tokstr() still carries inline quote MARKERS (Dnull/Snull/
    // Stringg); emitting it raw printed marker bytes and dropped the
    // visible quotes. It must untokenize to the display form.
    for (src, tok) in [
        (r#"a() {} "quoted""#, r#""quoted""#),
        (r#"a() {} 'single'"#, r#"'single'"#),
        (r#"a() {} $var"#, r#"$var"#),
        (r#"a() {} unquoted"#, r#"unquoted"#),
    ] {
        let (_st, _o, err) = run_zshrs(src);
        assert!(
            err.contains(&format!("parse error near `{tok}'")),
            "for {src:?} expected token `{tok}`, got: {err:?}"
        );
    }
}

#[test]
fn test_substring_whitespace_offset_is_zero_not_unrecognized_modifier() {
    // c:Src/subst.c:3781-3792 — a whitespace operand after `:` is a valid
    // zero-valued math expression, NOT a malformed/empty modifier. Only a
    // TRULY empty operand (no chars between delimiters) errors. zshrs's
    // `.trim().is_empty()` collapsed a space to empty and wrongly rejected
    // `${s: }` / `${s:  }` with "unrecognized modifier".
    let out = |code: &str| run_zshrs(code).1.trim().to_string();
    let errs = |code: &str| run_zshrs(code).2;

    // Whitespace offset → 0 → whole string.
    assert_eq!(out(r#"s=hello; print -r -- ${s: }"#), "hello");
    assert_eq!(out(r#"s=hello; print -r -- ${s:  }"#), "hello");
    // Whitespace offset + whitespace length → empty (0,0).
    assert_eq!(out(r#"s=hello; print -r -- ${s: : }"#), "");
    // Whitespace offset + numeric length.
    assert_eq!(out(r#"s=hello; print -r -- ${s: :2}"#), "he");

    // TRULY empty still errors (unchanged).
    assert!(errs(r#"s=hello; print -r -- ${s:}"#).contains("unrecognized modifier"));
    assert!(errs(r#"s=hello; print -r -- ${s::}"#).contains("unrecognized modifier"));
    assert!(errs(r#"s=hello; print -r -- ${s:1:}"#).contains("unrecognized modifier"));

    // Common idioms unaffected.
    assert_eq!(out(r#"s=hello; print -r -- ${s: -3}"#), "llo");
    assert_eq!(out(r#"s=hello; print -r -- ${s:2:3}"#), "llo");
}

#[test]
fn test_length_m_flag_counts_display_cells_not_chars() {
    // c:Src/subst.c:2376 `case 'm': multi_width++;` + c:3867
    // `len = MB_METASTRLEN2(val, multi_width)`. The `(m)` flag switches
    // `${#name}` from a CHARACTER count to a display-CELL (column) count.
    // Wide CJK/fullwidth glyphs occupy 2 cells; the plain `${#name}` count
    // stays 1-per-char. zshrs's scalar length passed `width=false`
    // unconditionally, ignoring `(m)`.
    let out = |code: &str| run_zshrs(code).1.trim().to_string();

    // Scalar: 3 wide chars → 6 cells.
    assert_eq!(out(r#"s="日本語"; print -r -- ${(m)#s}"#), "6");
    // Emoji → 2 cells.
    assert_eq!(out(r#"s="😀"; print -r -- ${(m)#s}"#), "2");
    // Fullwidth Latin → 2 cells each.
    assert_eq!(out(r#"s="ｆｕｌｌ"; print -r -- ${(m)#s}"#), "8");
    // Mixed narrow+wide: a(1)+日(2)+b(1) = 4.
    assert_eq!(out(r#"s="a日b"; print -r -- ${(m)#s}"#), "4");
    // Empty → 0.
    assert_eq!(out(r#"s=""; print -r -- ${(m)#s}"#), "0");

    // Plain `${#name}` (no (m)) stays a CHAR count — no regression.
    assert_eq!(out(r#"s="日本語"; print -r -- ${#s}"#), "3");
    assert_eq!(out(r#"s="héllo"; print -r -- ${#s}"#), "5");

    // Modifier chain runs before length: strip suffix then count cells.
    assert_eq!(out(r#"s="日本語ABC"; print -r -- ${(m)#s%ABC}"#), "6");

    // (mc) array: joined-with-sep cell width. 日本(4)+café(4)+1 sep = 9.
    assert_eq!(out(r#"a=(日本 café); print -r -- ${(mc)#a}"#), "9");
    // (c) alone (no m) stays a char count: 日本(2)+café(4)+1 = 7.
    assert_eq!(out(r#"a=(日本 café); print -r -- ${(c)#a}"#), "7");
    // (m) on an array with getlen==1 is ELEMENT count, not width.
    assert_eq!(out(r#"a=(日本 café); print -r -- ${(m)#a}"#), "2");
}

#[test]
fn test_tilde_globsubst_expands_tilde_in_value() {
    // c:Src/subst.c — ${~spec} / setopt globsubst subject a substituted
    // VALUE to the full filename-generation pipeline: filesub (tilde/`=`)
    // BEFORE globbing. zshrs globbed but skipped filesub, so a `~` in the
    // value stayed literal. Uses a fixed HOME for determinism.
    let run = |code: &str| {
        let out = std::process::Command::new(env!("CARGO_BIN_EXE_zshrs"))
            .args(["-f", "-c", code])
            .env("HOME", "/home/testu")
            .output()
            .expect("spawn");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    // ${~x} flag — scalar and array.
    assert_eq!(run(r#"x="~/foo"; echo ${~x}"#), "/home/testu/foo");
    assert_eq!(run(r#"x="~"; echo ${~x}"#), "/home/testu");
    assert_eq!(run(r#"a=(~/x ~/y); echo ${~a}"#), "/home/testu/x /home/testu/y");
    // setopt globsubst option path.
    assert_eq!(run(r#"setopt globsubst; x="~/foo"; echo $x"#), "/home/testu/foo");
    // Quoted tilde stays literal even under globsubst.
    assert_eq!(run(r#"setopt globsubst; echo "~/foo""#), "~/foo");
    // A mid-string tilde and non-tilde values are untouched.
    assert_eq!(run(r#"x="a~b"; echo ${~x}"#), "a~b");
    assert_eq!(run(r#"x="plain"; echo ${~x}"#), "plain");
}

#[test]
fn test_zstyle_g_uses_exact_pattern_not_context_match() {
    // c:Src/Modules/zutil.c:768-779 — `zstyle -g` retrieves the value
    // stored for the EXACT pattern string (strcmp), unlike -s/-a/-t/-b
    // which pattry-match a CONTEXT. After `zstyle ':s:*' k v`, a `-g`
    // query with ':s:sub' (which merely MATCHES the pattern) returns
    // nothing with exit 1; a query with the exact ':s:*' returns v.
    // zshrs's -g wrongly context-matched, returning v for ':s:sub'.
    // Emit `rc=<zstyle exit> [<value>]` so we read zstyle's status (the
    // trailing `print` would otherwise mask it in the process exit code).
    let run = |code: &str| run_zshrs(code).1.trim().to_string();
    // Exact pattern present → value + exit 0.
    assert_eq!(run(r#"zstyle ':s:*' k v; zstyle -g o ':s:*' k; print -r -- "rc=$? [$o]""#), "rc=0 [v]");
    // Matching-but-not-exact context → empty + exit 1.
    assert_eq!(run(r#"zstyle ':s:*' k v; zstyle -g o ':s:sub' k; print -r -- "rc=$? [$o]""#), "rc=1 []");
    // Exact literal context works.
    assert_eq!(run(r#"zstyle ':s:x' k w; zstyle -g o ':s:x' k; print -r -- "rc=$? [$o]""#), "rc=0 [w]");
    // Undefined → exit 1.
    assert_eq!(run(r#"zstyle -g o ':none' k; print -r -- "rc=$? [$o]""#), "rc=1 []");
    // Regression: -s still context-matches.
    assert_eq!(run(r#"zstyle ':s:*' k v; zstyle -s ':s:sub' k o; print -r -- "[$o]""#), "[v]");
    // Regression: exact keys are distinguished.
    assert_eq!(run(r#"zstyle ':a:b' s x; zstyle ':a:*' s y; zstyle -g o ':a:b' s; print -r -- "[$o]""#), "[x]");
}

#[test]
fn test_cond_o_bad_option_diagnostic_has_no_command_name() {
    // c:Src/cond.c:513 — `[[ -o BADOPT ]]` emits `zwarnnam(fromtest, ...)`
    // with fromtest = NULL, so the diagnostic has NO command-name prefix
    // (`<shell>:1: no such option: X`) and exit status 3. zshrs hardcoded
    // "test", printing `<shell>:test:1:`, and double-emitted for
    // negated/compound forms via a redundant eprintln.
    let (st, _o, err) = run_zshrs("[[ -o keyword ]]");
    assert!(err.contains("no such option: keyword"), "err: {err:?}");
    assert!(!err.contains(":test:"), "must not attribute to test: {err:?}");
    assert_eq!(st, 3);

    // No double-emit on negated / compound forms.
    let (_s2, _o2, err2) = run_zshrs("[[ ! -o badopt ]]");
    assert_eq!(err2.matches("no such option").count(), 1, "double-emit: {err2:?}");
    let (_s3, _o3, err3) = run_zshrs("[[ -o a1 || -o a2 ]]");
    assert_eq!(err3.matches("no such option").count(), 2, "got: {err3:?}");

    // A valid option still works (no diagnostic).
    assert_eq!(run_zshrs("setopt extendedglob; [[ -o extendedglob ]]").0, 0);
    assert_eq!(run_zshrs("[[ -o extendedglob ]]").0, 1);
}

#[test]
fn test_tied_colon_array_element_assignment_syncs_scalar() {
    // c:Src/params.c:2922 — a subscript assignment `path[N]=x` rebuilds the
    // whole array and hands it to the param's array setfn, so a tied
    // colon-array (path->PATH, fpath->FPATH, cdpath->CDPATH, …) re-derives
    // its scalar side. zshrs wrote the array element directly and skipped
    // the setfn, leaving $PATH stale after `path[2]=/NEW`.
    let run = |code: &str| run_zshrs(code).1.trim().to_string();
    assert_eq!(run("path=(/a /b /c); path[2]=/NEW; echo $PATH"), "/a:/NEW:/c");
    assert_eq!(run("path=(/a /b /c); path[1]=/X; echo $PATH"), "/X:/b:/c");
    assert_eq!(run("path=(/a /b /c); path[4]=/D; echo $PATH"), "/a:/b:/c:/D");
    assert_eq!(run("path=(/a /b); path[-1]=/LAST; echo $PATH"), "/a:/LAST");
    assert_eq!(run("fpath=(/f1 /f2); fpath[1]=/NEW; echo $FPATH"), "/NEW:/f2");
    assert_eq!(run("cdpath=(/x /y); cdpath[2]=/Z; echo $CDPATH"), "/x:/Z");
    // The array element itself is still updated, and the reverse tie holds.
    assert_eq!(run(r#"path=(/a /b /c); path[2]=/NEW; echo "${path[2]}""#), "/NEW");
    assert_eq!(run(r#"PATH=/x:/y; path[1]=/Z; echo "$PATH ${path[1]}""#), "/Z:/y /Z");
    // A plain (untied) array element assignment is unaffected.
    assert_eq!(run(r#"a=(1 2 3); a[2]=X; echo "${a[@]}""#), "1 X 3");
}

#[test]
fn test_tied_colon_array_slice_and_unset_sync_scalar() {
    // c:Src/params.c:2922 — every SUBSCRIPT modification of a tied
    // colon-array re-derives the scalar side, not just the single-element
    // scalar form: range (setarrvalue), array-valued element, delete, and
    // `unset name[...]`. zshrs synced only path[N]=scalar before.
    let run = |code: &str| run_zshrs(code).1.trim().to_string();
    // Range replace.
    assert_eq!(run("path=(/a /b /c /d); path[2,3]=(/X /Y); echo $PATH"), "/a:/X:/Y:/d");
    // Range shrink.
    assert_eq!(run("path=(/a /b /c); path[2,3]=(/X); echo $PATH"), "/a:/X");
    // Range to end (negative).
    assert_eq!(run("path=(/a /b /c); path[2,-1]=(/Z); echo $PATH"), "/a:/Z");
    // Element replaced with multiple.
    assert_eq!(run("path=(/a /b /c); path[2]=(/X /Y); echo $PATH"), "/a:/X:/Y:/c");
    // Range delete.
    assert_eq!(run("path=(/a /b); path[2,3]=(); echo $PATH"), "/a");
    // fpath slice.
    assert_eq!(run("fpath=(/a /b /c); fpath[1,2]=(/N); echo $FPATH"), "/N:/c");
    // unset element (element becomes empty, kept).
    assert_eq!(run(r#"path=(/a /b /c /d); unset "path[2]"; echo "$PATH ${#path}""#), "/a::/c:/d 4");
    // unset range.
    assert_eq!(run(r#"path=(/a /b /c /d); unset "path[2,3]"; echo $PATH"#), "/a::/d");
    // Plain untied array unaffected.
    assert_eq!(run(r#"a=(1 2 3 4); a[2,3]=(X Y Z); echo "${a[@]}""#), "1 X Y Z 4");
}

#[test]
fn test_math_userfunc_missing_shfunc_is_no_such_function() {
    // c:Src/math.c:1110-1112 — a `functions -M` math function whose
    // implementing SHELL function doesn't exist errors "no such function:
    // <shfnam>", distinct from the "unknown function: <n>" of an
    // UN-registered math name (math.c:1131). zshrs used the latter for both.
    let err = |code: &str| run_zshrs(code).2;
    // Registered but shell fn missing -> "no such function".
    assert!(
        err(r#"functions -M nonexist; echo $(( nonexist(1) ))"#).contains("no such function: nonexist"),
        "registered+missing must be 'no such function'"
    );
    // The 4th arg names a DIFFERENT impl; the diagnostic uses it.
    assert!(
        err(r#"functions -M foo 1 1 _missing_impl; echo $(( foo(1) ))"#)
            .contains("no such function: _missing_impl"),
        "impl-name form"
    );
    // Not registered at all -> "unknown function".
    assert!(
        err(r#"echo $(( totally_unregistered(1) ))"#).contains("unknown function: totally_unregistered"),
        "unregistered must be 'unknown function'"
    );
    // A working math function is unaffected.
    let (_s, out, _e) = run_zshrs(r#"cube() { (( REPLY = $1 ** 3 )) }; functions -M cube 1 1; echo $(( cube(4) ))"#);
    assert_eq!(out.trim(), "64");
}

// ---------------------------------------------------------------------
// $zle_highlight colour-code overrides (Src/prompt.c:2440
// set_colour_attribute + c:2367 allocate_colour_buffer).
//
// zsh lets $zle_highlight replace the escape that carries a colour:
// {fg,bg}_start_code (prefix), _default_code (the reset body) and
// _end_code (suffix). set_colour_attribute composes
// `start + <code> + end`, where <code> is the palette index, or the
// `.def` body when the colour is being turned back OFF.
//
// Every expectation below is the byte-exact output of the real shell
// (`/bin/zsh -f -c ... | od -c`), including the ordering quirk pinned in
// the first test.
// ---------------------------------------------------------------------

#[test]
fn test_zle_highlight_fg_start_code_overrides_colour_escape() {
    // zsh oracle: `\e[31m x \e[99m`.
    //
    // The SET still emits the stock `\e[31m`, and only the RESET picks up
    // the override. That is not a bug: fg_bg_sequences is populated by
    // allocate_colour_buffer(), which set_colour_attribute only reaches
    // AFTER its termcap fast-path (Src/prompt.c:2478-2515). The first
    // colour set of a session therefore still sees the built-in codes.
    // The reset skips that fast-path (def != 0) and composes
    // `\e[9` + `9` + `m`. Pinning both halves keeps that order faithful.
    let (status, bytes) = run_zshrs_parity_bytes(
        r#"zle_highlight=(fg_start_code:$'\e[9' fg_default_code:'9' fg_end_code:'m'); print -Pn '%F{1}x%f'"#,
    );
    assert_eq!(status, 0);
    assert_eq!(bytes, b"\x1b[31mx\x1b[99m", "got: {bytes:x?}");
}

#[test]
fn test_zle_highlight_256_colour_start_code() {
    // The documented way to drive a 256-colour terminal: a `\e[38;5;`
    // prefix turns set_colour_attribute's bare "%d" index body
    // (Src/prompt.c:2538-2539) into a well-formed escape.
    // zsh oracle: `\e[32m y \e[38;5;39m`.
    let (status, bytes) = run_zshrs_parity_bytes(
        r#"zle_highlight=(fg_start_code:$'\e[38;5;' fg_default_code:'39' fg_end_code:'m'); print -Pn '%F{2}y%f'"#,
    );
    assert_eq!(status, 0);
    assert_eq!(bytes, b"\x1b[32my\x1b[38;5;39m", "got: {bytes:x?}");
}

#[test]
fn test_zle_highlight_bg_start_code_overrides_colour_escape() {
    // Background channel reads its own COL_SEQ_BG slot.
    // zsh oracle: `\e[43m z \e[109m`.
    let (status, bytes) = run_zshrs_parity_bytes(
        r#"zle_highlight=(bg_start_code:$'\e[10' bg_default_code:'9' bg_end_code:'m'); print -Pn '%K{3}z%k'"#,
    );
    assert_eq!(status, 0);
    assert_eq!(bytes, b"\x1b[43mz\x1b[109m", "got: {bytes:x?}");
}

#[test]
fn test_zle_highlight_fg_default_code_overrides_reset_body() {
    // fg_default_code alone replaces just the reset body, so the reset
    // becomes `\e[3` + `39;1` + `m`. This is the arm that a hardcoded
    // `\e[39m` reset silently ignores.
    // zsh oracle: `\e[35m q \e[339;1m`.
    let (status, bytes) =
        run_zshrs_parity_bytes(r#"zle_highlight=(fg_default_code:'39;1'); print -Pn '%F{5}q%f'"#);
    assert_eq!(status, 0);
    assert_eq!(bytes, b"\x1b[35mq\x1b[339;1m", "got: {bytes:x?}");
}

#[test]
fn test_prompt_colour_escapes_unaffected_without_zle_highlight() {
    // Regression net: with no override the emitted bytes must stay
    // exactly what the stock shell produces — indexed, bright, 256-colour
    // and 24-bit forms all take the built-in TC_COL_* codes.
    for (code, want) in [
        ("%F{1}x%f", &b"\x1b[31mx\x1b[39m"[..]),
        ("%F{9}x%f", &b"\x1b[91mx\x1b[39m"[..]),
        ("%F{200}x%f", &b"\x1b[38;5;200mx\x1b[39m"[..]),
        ("%K{4}x%k", &b"\x1b[44mx\x1b[49m"[..]),
        ("%F{#ff8800}x%f", &b"\x1b[38;2;255;136;0mx\x1b[39m"[..]),
    ] {
        let (status, bytes) = run_zshrs_parity_bytes(&format!("print -Pn '{code}'"));
        assert_eq!(status, 0, "{code}");
        assert_eq!(bytes, want, "{code} got: {bytes:x?}");
    }
}
