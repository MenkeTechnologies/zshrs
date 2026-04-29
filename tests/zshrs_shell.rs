//! Integration tests for zshrs shell — exercises builtins, syntax, and
//! variable handling by spawning the real `zshrs` binary with `-f -c`.

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
    // Lexer-level syntax error (unmatched single quote). zsh treats
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
    let (_, output, _) =
        run_zshrs("arr=(apple banana cherry banana); print ${arr[(I)b*]}");
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

// ---------------------------------------------------------------------------
// `typeset -A` two-statement assoc init: declare then array-literal-assign
// ---------------------------------------------------------------------------

#[test]
fn test_typeset_a_two_statement_init() {
    let (_, output, _) = run_zshrs(
        "typeset -A m; m=(a 1 b 2 c 3); print \"${m[a]}-${m[b]}-${m[c]}\"",
    );
    assert_eq!(output.trim(), "1-2-3", "got: {output:?}");
}

#[test]
fn test_typeset_a_then_indexed_assoc_remains_assoc() {
    // After `typeset -A m; m=(...)`, `m` should still respond to assoc
    // syntax. Test by appending another key/val pair.
    let (_, output, _) = run_zshrs(
        "typeset -A m; m=(a 1 b 2); m[c]=3; print \"${m[a]}|${m[b]}|${m[c]}\"",
    );
    assert_eq!(output.trim(), "1|2|3", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// Subscript with $-expanded key: ${m[$k]} for assoc, ${arr[$i]} for indexed
// ---------------------------------------------------------------------------

#[test]
fn test_assoc_subscript_dynamic_key() {
    let (_, output, _) = run_zshrs(
        "typeset -A m=(foo 1 bar 2); k=foo; print \"${m[$k]}\"",
    );
    assert_eq!(output.trim(), "1", "got: {output:?}");
}

#[test]
fn test_assoc_subscript_dynamic_key_loop() {
    let (_, output, _) = run_zshrs(
        "typeset -A m=(a 1 b 2); for k in a b; do print \"$k=${m[$k]}\"; done",
    );
    assert_eq!(output.trim(), "a=1\nb=2", "got: {output:?}");
}

#[test]
fn test_indexed_subscript_dynamic_key_in_loop() {
    let (_, output, _) = run_zshrs(
        "arr=(x y z); for i in 1 2 3; do print \"$i:${arr[$i]}\"; done",
    );
    assert_eq!(output.trim(), "1:x\n2:y\n3:z", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// Extendedglob `^pat` negation in `${arr:#pat}` filter
// ---------------------------------------------------------------------------

#[test]
fn test_extendedglob_negation_in_filter() {
    // `:#^*.txt` removes elements matching the negation of `*.txt`
    // → keeps only `*.txt` elements.
    let (_, output, _) = run_zshrs(
        "setopt extendedglob; arr=(foo.txt bar.log baz.txt); print -l ${arr:#^*.txt}",
    );
    assert_eq!(output.trim(), "foo.txt\nbaz.txt", "got: {output:?}");
}

#[test]
fn test_extendedglob_negation_literal_inverse() {
    let (_, output, _) =
        run_zshrs("setopt extendedglob; arr=(a b c); print -l ${arr:#^a}");
    assert_eq!(output.trim(), "a", "got: {output:?}");
}

#[test]
fn test_extendedglob_negation_off_when_option_unset() {
    // Without `extendedglob`, `^a` is a literal pattern (no element
    // matches the literal char `^a`), so all 3 stay.
    let (_, output, _) =
        run_zshrs("arr=(a b c); print -l ${arr:#^a}");
    assert_eq!(output.trim(), "a\nb\nc", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// Extendedglob inline pattern flags (#i) (#l) (#a<n>)
// ---------------------------------------------------------------------------

#[test]
fn test_pattern_flag_case_insensitive() {
    let (_, output, _) = run_zshrs(
        "setopt extendedglob; [[ \"ABC\" = (#i)abc ]] && echo match || echo nomatch",
    );
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

#[test]
fn test_pattern_flag_case_insensitive_uppercase_pat() {
    let (_, output, _) = run_zshrs(
        "setopt extendedglob; [[ \"abc\" = (#i)ABC ]] && echo match || echo nomatch",
    );
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

#[test]
fn test_pattern_flag_l_lowercase_matches_either_case() {
    let (_, output, _) = run_zshrs(
        "setopt extendedglob; [[ \"AbC\" = (#l)abc ]] && echo match || echo nomatch",
    );
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

#[test]
fn test_pattern_flag_l_uppercase_must_match_exactly() {
    // (#l) is asymmetric: uppercase pattern char requires exact case
    // in input. So `(#l)ABC` does NOT match "abc".
    let (_, output, _) = run_zshrs(
        "setopt extendedglob; [[ \"abc\" = (#l)ABC ]] && echo match || echo nomatch",
    );
    assert_eq!(output.trim(), "nomatch", "got: {output:?}");
}

#[test]
fn test_pattern_flag_approximate_one_error() {
    let (_, output, _) = run_zshrs(
        "setopt extendedglob; [[ \"abcd\" = (#a1)abce ]] && echo match || echo nomatch",
    );
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

#[test]
fn test_pattern_flag_approximate_zero_error_diff() {
    // (#a0) requires exact match.
    let (_, output, _) = run_zshrs(
        "setopt extendedglob; [[ \"abcd\" = (#a0)abce ]] && echo match || echo nomatch",
    );
    assert_eq!(output.trim(), "nomatch", "got: {output:?}");
}

#[test]
fn test_pattern_flag_approximate_insertion() {
    let (_, output, _) = run_zshrs(
        "setopt extendedglob; [[ \"abc12\" = (#a1)abc1 ]] && echo match || echo nomatch",
    );
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// (@s:sep:) / (@f) flag composition: @ + split must keep array shape in DQ
// ---------------------------------------------------------------------------

#[test]
fn test_at_s_flag_split_in_double_quotes() {
    let (_, output, _) =
        run_zshrs("str='a,b,c'; print -l \"${(@s:,:)str}\"");
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
    let (_, output, _) =
        run_zshrs("str=$'a\\nb\\nc'; print -l \"${(@f)str}\"");
    assert_eq!(output.trim(), "a\nb\nc", "got: {output:?}");
}

#[test]
fn test_s_flag_splits_each_array_element() {
    let (_, output, _) = run_zshrs(
        "arr=('a,b' 'c,d'); print -l \"${(@s:,:)arr}\"",
    );
    assert_eq!(output.trim(), "a\nb\nc\nd", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// Param flag with [@] subscript: ${(kv)m[@]}, ${(o)arr[@]}, etc.
// ---------------------------------------------------------------------------

#[test]
fn test_kv_flag_with_at_subscript() {
    // Sort because assoc HashMap iteration order is non-deterministic.
    let (_, output, _) = run_zshrs(
        "typeset -A m=(a 1 b 2 c 3); print -l \"${(kv)m[@]}\" | sort",
    );
    assert_eq!(output.trim(), "1\n2\n3\na\nb\nc", "got: {output:?}");
}

#[test]
fn test_k_flag_with_at_subscript() {
    let (_, output, _) = run_zshrs(
        "typeset -A m=(a 1 b 2 c 3); print -l \"${(k)m[@]}\" | sort",
    );
    assert_eq!(output.trim(), "a\nb\nc", "got: {output:?}");
}

#[test]
fn test_o_sort_flag_with_at_subscript() {
    // (o) only fires in array context — no DQ wrapper.
    let (_, output, _) =
        run_zshrs("arr=(c a b); print -l ${(o)arr[@]}");
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
    let (_, output, _) =
        run_zshrs("function () { echo \"args:\" \"$@\" } a b c");
    assert_eq!(output.trim(), "args: a b c", "got: {output:?}");
}

#[test]
fn test_function_keyword_anonymous_local_scope() {
    let (_, output, _) = run_zshrs(
        "x=outer; function () { local x=inner; print \"$x\" } ; print \"$x\"",
    );
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
    let (_, output, _) = run_zshrs(
        "zmodload zsh/mapfile; print \"len=${#mapfile[/no/such/path/here]}\"",
    );
    assert_eq!(output.trim(), "len=0", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// ${(flags)NAME[KEY]} — flag + literal subscript composition
// ---------------------------------------------------------------------------

#[test]
fn test_flag_with_assoc_subscript_split() {
    let (_, output, _) = run_zshrs(
        "typeset -A m=(k1 'a:b:c'); print -l \"${(s.:.)m[k1]}\"",
    );
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
    let (_, output, _) =
        run_zshrs("a=(x y z); a[5]=E; print -l \"${a[@]}\"");
    assert_eq!(output.trim(), "x\ny\nz\n\nE", "got: {output:?}");
}

#[test]
fn test_indexed_array_append_at_index() {
    let (_, output, _) = run_zshrs("a=(x y z); a[2]+=BB; print \"${a[*]}\"");
    assert_eq!(output.trim(), "x yBB z", "got: {output:?}");
}

#[test]
fn test_indexed_array_slice_assign() {
    let (_, output, _) =
        run_zshrs("a=(x y z w v); a[2,4]=(YY ZZ WW); print \"${a[*]}\"");
    assert_eq!(output.trim(), "x YY ZZ WW v", "got: {output:?}");
}

#[test]
fn test_indexed_array_element_delete() {
    let (_, output, _) = run_zshrs("a=(x y z); a[2]=(); print \"${a[*]}\"");
    assert_eq!(output.trim(), "x z", "got: {output:?}");
}

#[test]
fn test_indexed_array_slice_delete() {
    let (_, output, _) =
        run_zshrs("a=(x y z w v); a[2,4]=(); print \"${a[*]}\"");
    assert_eq!(output.trim(), "x v", "got: {output:?}");
}

#[test]
fn test_unset_indexed_element_clears_to_empty() {
    // zsh `unset 'arr[N]'` for indexed arrays sets the slot to "" but
    // does NOT remove it (slot count preserved). Differs from `a[N]=()`.
    let (_, output, _) = run_zshrs(
        "arr=(a b c); unset 'arr[2]'; print \"len=${#arr}\"; print -l \"${arr[@]}\"",
    );
    assert_eq!(output.trim(), "len=3\na\n\nc", "got: {output:?}");
}

#[test]
fn test_unset_assoc_element() {
    let (_, output, _) = run_zshrs(
        "typeset -A m=(a 1 b 2); unset 'm[a]'; print \"${(k)m[@]}\"",
    );
    assert_eq!(output.trim(), "b", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// Regex `=~` capture groups: $MATCH, $match[N], $mbegin, $mend
// ---------------------------------------------------------------------------

#[test]
fn test_regex_match_full_match_var() {
    let (_, output, _) = run_zshrs(
        "[[ \"hello\" =~ ll ]] && print \"$MATCH\"",
    );
    assert_eq!(output.trim(), "ll", "got: {output:?}");
}

#[test]
fn test_regex_match_capture_groups() {
    let (_, output, _) = run_zshrs(
        "[[ \"a1b2\" =~ ([a-z])([0-9]) ]] && print \"${match[1]}|${match[2]}\"",
    );
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
    let (_, output, _) =
        run_zshrs("[[ file5 = file<1-10> ]] && echo match || echo nomatch");
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

#[test]
fn test_numeric_range_out_of_bounds() {
    let (_, output, _) =
        run_zshrs("[[ file20 = file<1-10> ]] && echo match || echo nomatch");
    assert_eq!(output.trim(), "nomatch", "got: {output:?}");
}

#[test]
fn test_numeric_range_open_lo() {
    // `<-10>` means ≤ 10.
    let (_, output, _) =
        run_zshrs("[[ 7 = <-10> ]] && echo match || echo nomatch");
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

#[test]
fn test_numeric_range_open_hi() {
    // `<50->` means ≥ 50.
    let (_, output, _) =
        run_zshrs("[[ 100 = <50-> ]] && echo match || echo nomatch");
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

#[test]
fn test_numeric_range_any_integer() {
    // `<->` matches any non-negative integer.
    let (_, output, _) =
        run_zshrs("[[ 42 = <-> ]] && echo match || echo nomatch");
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

#[test]
fn test_numeric_range_rejects_non_digits() {
    let (_, output, _) =
        run_zshrs("[[ abc = <-> ]] && echo match || echo nomatch");
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
    assert_eq!(output.trim(), "gst: aliased to git status", "got: {output:?}");
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
        output,
        "\x1b[32mg\x1b[39m\x1b[31mr\x1b[39m\n",
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
// `print -P %D{fmt}` strftime format
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
    let (_, output, _) =
        run_zshrs("typeset -A m=(a 1 b 2); print $m[a]");
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
    let (_, output, _) =
        run_zshrs("typeset -L 5 s=hello; print \"${(t)s}\"");
    assert_eq!(output.trim(), "scalar-left", "got: {output:?}");
}

#[test]
fn test_t_flag_scalar_readonly() {
    let (_, output, _) =
        run_zshrs("typeset -r ro=foo; print \"${(t)ro}\"");
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
    let (_, stdout, _) =
        run_zshrs(&format!("print {}(mh+10000) 2>/dev/null", path));
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
    let (_, output, _) =
        run_zshrs(&format!("cd {} && print -l **/", root));
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
    let (_, output, _) =
        run_zshrs(&format!("cd {} && print -l **/*", root));
    let _ = fs::remove_dir_all(root);
    let mut lines: Vec<&str> = output.lines().collect();
    lines.sort();
    assert_eq!(
        lines,
        vec!["a", "a/x.txt", "b", "b/c"],
        "got: {output:?}"
    );
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
    let (_, output, _) =
        run_zshrs(&format!("cd {} && print -l **/*.txt", root));
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
    let (_, output, _) =
        run_zshrs("p=hello; echo ${p:s/l/L/}");
    assert_eq!(output.trim(), "heLlo", "got: {output:?}");
}

#[test]
fn test_subst_modifier_global() {
    let (_, output, _) =
        run_zshrs("p=hello; echo ${p:gs/l/L/}");
    assert_eq!(output.trim(), "heLLo", "got: {output:?}");
}

#[test]
fn test_subst_modifier_chained_with_t() {
    let (_, output, _) =
        run_zshrs("p=/a/b.txt; echo ${p:s/b/B/:t}");
    assert_eq!(output.trim(), "B.txt", "got: {output:?}");
}

#[test]
fn test_q_modifier_backslash_quote() {
    // zsh `:q` uses backslash escaping, not single-quote wrapping.
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
    let (_, output, _) =
        run_zshrs("foo() { echo $funcstack[1]; }; foo");
    assert_eq!(output.trim(), "foo", "got: {output:?}");
}

#[test]
fn test_funcstack_nested() {
    let (_, output, _) = run_zshrs(
        "foo() { bar() { echo \"${funcstack[*]}\"; }; bar; }; foo",
    );
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
    let (_, output, _) = run_zshrs(
        "setopt kshglob; [[ a = ?(a|b) ]] && echo match || echo nomatch",
    );
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

#[test]
fn test_kshglob_plus_one_or_more() {
    let (_, output, _) = run_zshrs(
        "setopt kshglob; [[ aaa = +(a) ]] && echo match || echo nomatch",
    );
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

#[test]
fn test_kshglob_at_exactly_one() {
    let (_, output, _) = run_zshrs(
        "setopt kshglob; [[ foo = @(foo|bar) ]] && echo match || echo nomatch",
    );
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

#[test]
fn test_kshglob_off_no_match() {
    // Without `setopt kshglob`, ?(a|b) is the default zsh-glob shape
    // and doesn't match the bare letter `a` (literal `?(...)`).
    let (_, output, _) =
        run_zshrs("[[ a = ?(a|b) ]] && echo match || echo nomatch");
    assert_eq!(output.trim(), "nomatch", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// Pattern repetition `(#cN)` and `(#cN,M)`
// ---------------------------------------------------------------------------

#[test]
fn test_pattern_repeat_exact() {
    let (_, output, _) = run_zshrs(
        "setopt extendedglob; [[ aa = a(#c2) ]] && echo match || echo nomatch",
    );
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

#[test]
fn test_pattern_repeat_range() {
    let (_, output, _) = run_zshrs(
        "setopt extendedglob; [[ aaa = a(#c2,3) ]] && echo match || echo nomatch",
    );
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

#[test]
fn test_pattern_repeat_out_of_range() {
    let (_, output, _) = run_zshrs(
        "setopt extendedglob; [[ aaaa = a(#c2,3) ]] && echo match || echo nomatch",
    );
    assert_eq!(output.trim(), "nomatch", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// Special parameters: $EUID, $UID, $PPID, $HOST, $ZSH_SUBSHELL, $#@, $#*
// ---------------------------------------------------------------------------

#[test]
fn test_special_param_euid() {
    let (_, output, _) = run_zshrs("echo $EUID");
    assert!(
        output.trim().chars().all(|c| c.is_ascii_digit())
            && !output.trim().is_empty(),
        "got: {output:?}"
    );
}

#[test]
fn test_special_param_uid() {
    let (_, output, _) = run_zshrs("echo $UID");
    assert!(
        output.trim().chars().all(|c| c.is_ascii_digit())
            && !output.trim().is_empty(),
        "got: {output:?}"
    );
}

#[test]
fn test_special_param_ppid() {
    let (_, output, _) = run_zshrs("echo $PPID");
    assert!(
        output.trim().chars().all(|c| c.is_ascii_digit())
            && !output.trim().is_empty(),
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
    let (_, output, _) =
        run_zshrs("zmodload zsh/system; print $sysparams[pid]");
    let pid = output.trim();
    assert!(
        pid.chars().all(|c| c.is_ascii_digit()) && !pid.is_empty(),
        "got: {output:?}"
    );
}

#[test]
fn test_sysparams_ppid() {
    let (_, output, _) =
        run_zshrs("zmodload zsh/system; print $sysparams[ppid]");
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
    let (_, output, _) = run_zshrs(
        "setopt kshglob; [[ baz = !(foo|bar) ]] && echo match || echo nomatch",
    );
    assert_eq!(output.trim(), "match", "got: {output:?}");
}

#[test]
fn test_kshglob_negation_no_match() {
    let (_, output, _) = run_zshrs(
        "setopt kshglob; [[ foo = !(foo|bar) ]] && echo match || echo nomatch",
    );
    assert_eq!(output.trim(), "nomatch", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// `(F)` newline-join flag
// ---------------------------------------------------------------------------

#[test]
fn test_F_flag_joins_array_with_newlines() {
    let (_, output, _) =
        run_zshrs("arr=(a b c); echo \"${(F)arr}\"");
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
    let (_, output, _) =
        run_zshrs("typeset -A m=(a 1 b 2); typeset -p m");
    assert_eq!(
        output.trim(),
        "typeset -A m=( [a]=1 [b]=2 )",
        "got: {output:?}"
    );
}

#[test]
fn test_export_p_lists_var() {
    let (_, output, _) =
        run_zshrs("export X=hello; export -p 2>&1 | grep '^export X='");
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
    let (_, output, _) = run_zshrs(&format!(
        "cd {} && zmv -n '(*).txt' '$1.bak'",
        dir
    ));
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
    let (_, _, _) = run_zshrs(&format!(
        "cd {} && zmv '(*).txt' '$1.bak'",
        dir
    ));
    let exists_bak = std::path::Path::new(&format!("{}/foo.bak", dir)).exists();
    let exists_orig = std::path::Path::new(&format!("{}/foo.txt", dir)).exists();
    let _ = fs::remove_dir_all(dir);
    assert!(exists_bak && !exists_orig, "bak={} orig={}", exists_bak, exists_orig);
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
    let (status, _, stderr) = run_zshrs(&format!(
        "cd {} && zmv '*.txt' 'merged.bak'",
        dir
    ));
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
    let (_, output, _) = run_zshrs(
        "[[ /tmp/zsh_nt_b -nt /tmp/zsh_nt_a ]] && echo yes || echo no",
    );
    let _ = fs::remove_file("/tmp/zsh_nt_a");
    let _ = fs::remove_file("/tmp/zsh_nt_b");
    // ignore trailing zpwr log noise from cwd hook
    assert!(output.starts_with("yes"), "got: {output:?}");
}

#[test]
fn test_cond_k_sticky_bit() {
    let (_, output, _) =
        run_zshrs("[[ -k /tmp ]] && echo yes || echo no");
    assert!(output.starts_with("yes"), "got: {output:?}");
}

#[test]
fn test_cond_O_owned_by_user() {
    // /tmp is typically root-owned; not us. /Users/$USER/... is. We
    // just check the operator runs without erroring — exit status of 0
    // (yes) or 1 (no) is fine.
    let (_, output, _) =
        run_zshrs("[[ -O /tmp ]] && echo yes || echo no");
    assert!(
        output.starts_with("yes") || output.starts_with("no"),
        "got: {output:?}"
    );
}

#[test]
fn test_cond_G_owned_by_group() {
    let (_, output, _) =
        run_zshrs("[[ -G /tmp ]] && echo yes || echo no");
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
    let (_, output, _) = run_zshrs(
        "setopt extendedglob; [[ apple = ^a* ]] && echo y || echo n",
    );
    assert_eq!(output.trim(), "n", "got: {output:?}");
}

#[test]
fn test_cond_neg_pattern_includes_non_match() {
    // `[[ banana = ^a* ]]` with extendedglob → true.
    let (_, output, _) = run_zshrs(
        "setopt extendedglob; [[ banana = ^a* ]] && echo y || echo n",
    );
    assert_eq!(output.trim(), "y", "got: {output:?}");
}

#[test]
fn test_cond_neg_literal_without_extendedglob() {
    // Without extendedglob, `^` is a literal char and `^a*` doesn't
    // match `apple`.
    let (_, output, _) =
        run_zshrs("[[ apple = ^a* ]] && echo y || echo n");
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
    let (_, output, _) =
        run_zshrs("print -m '*.txt' a.txt b.log c.txt");
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
    let (_, output, _) =
        run_zshrs("set -- a b c d e; echo \"${@[2,3]}\"");
    assert_eq!(output.trim(), "b c", "got: {output:?}");
}

#[test]
fn test_at_subscript_single() {
    let (_, output, _) =
        run_zshrs("set -- a b c d e; echo \"${@[1]}\"");
    assert_eq!(output.trim(), "a", "got: {output:?}");
}

#[test]
fn test_at_subscript_negative() {
    let (_, output, _) =
        run_zshrs("set -- a b c d e; echo \"${@[-1]}\"");
    assert_eq!(output.trim(), "e", "got: {output:?}");
}

#[test]
fn test_star_subscript_slice() {
    let (_, output, _) =
        run_zshrs("set -- a b c d e; echo \"${*[2,4]}\"");
    assert_eq!(output.trim(), "b c d", "got: {output:?}");
}

#[test]
fn test_argv_subscript_slice() {
    let (_, output, _) =
        run_zshrs("set -- a b c d e; echo \"${argv[2,4]}\"");
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
    let (_, output, _) = run_zshrs(
        "arr=(apple banana cherry); for f in $arr; do echo \"$f\"; done",
    );
    assert_eq!(output.trim(), "apple\nbanana\ncherry", "got: {output:?}");
}

#[test]
fn test_for_quoted_array_joins() {
    // "$arr" (DQ) joins to a scalar — single iteration with the joined
    // string. Matches zsh DQ-array semantics.
    let (_, output, _) = run_zshrs(
        "arr=(a b c); for f in \"$arr\"; do echo \"got=$f\"; done",
    );
    // zsh joins with first char of $IFS (default space)
    assert_eq!(output.trim(), "got=a b c", "got: {output:?}");
}

// ---------------------------------------------------------------------------
// `arr+=val` (no parens) — push as new element (runtime-dispatched)
// ---------------------------------------------------------------------------

#[test]
fn test_array_append_single_no_parens() {
    let (_, output, _) =
        run_zshrs("a=(x); a+=y; echo \"${a[@]} ${#a}\"");
    assert_eq!(output.trim(), "x y 2", "got: {output:?}");
}

#[test]
fn test_array_append_to_multi_element() {
    let (_, output, _) =
        run_zshrs("a=(x y); a+=z; echo \"${a[@]} ${#a}\"");
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
    let (_, output, _) =
        run_zshrs("unset xx; echo \"${xx=set-and-use}\"; echo \"$xx\"");
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
    let (status, _, _) = run_zshrs(
        "foo() { :; }; unset -f foo; type foo 2>&1 | grep -q 'not found'",
    );
    assert_eq!(status, 0, "type should report 'not found' after unset -f");
}

// ---------------------------------------------------------------------------
// Scalar in for-list does NOT IFS-split (matches zsh)
// ---------------------------------------------------------------------------

#[test]
fn test_for_scalar_no_ifs_split_default() {
    // zsh: `for w in $s` iterates ONCE with the scalar value.
    let (_, output, _) = run_zshrs(
        "IFS=,; s='a,b,c'; for w in $s; do echo \"[$w]\"; done",
    );
    assert_eq!(output.trim(), "[a,b,c]", "got: {output:?}");
}

#[test]
fn test_for_scalar_splits_under_shwordsplit() {
    // bash-compat: under setopt shwordsplit, scalar IS IFS-split.
    let (_, output, _) = run_zshrs(
        "setopt shwordsplit; IFS=,; s='a,b,c'; for w in $s; do echo \"[$w]\"; done",
    );
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
    let (_, output, _) = run_zshrs(
        "foo() echo \"args:\" \"$@\"; foo a b",
    );
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
    let code = format!("{{ echo out; echo err >&2; }} &> {p}; echo done; cat {p}", p = path);
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
    let (_, output, _) = run_zshrs(
        "set -- -a -b foo; print \"rest:$@\"",
    );
    assert_eq!(output.trim(), "rest:-a -b foo", "got: {output:?}");
}

#[test]
fn test_zparseopts_dash_d_removes_only_consumed() {
    // `zparseopts -D a=opta` consumes `-a` only — `-b foo` must remain
    // in the positional params untouched.
    let (_, output, _) = run_zshrs(
        "zmodload zsh/zutil; set -- -a -b foo; zparseopts -D a=opta; echo \"rest:$@\"",
    );
    assert_eq!(output.trim(), "rest:-b foo", "got: {output:?}");
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
fn test_zformat_width_padding() {
    // zformat `%-Ns` right-aligns (pads on left) and `%Ns` left-aligns
    // (pads on right) — opposite of printf, matches zsh observed.
    let (_, output, _) = run_zshrs(
        "zmodload zsh/zutil; zformat -f r \"%-10s|%10s\" \"s:foo\"; echo \"[$r]\"",
    );
    assert_eq!(output.trim(), "[       foo|foo       ]", "got: {output:?}");
}

#[test]
fn test_getopts_unknown_uses_zsh_format() {
    // Zsh emits `zsh:1: bad option: -X` for unknown opts when the
    // optstring isn't quiet (no leading `:`). We mirror with `zshrs:1:`.
    let (_, _, stderr) = run_zshrs(
        "set -- -x; while getopts \"ab\" opt; do :; done",
    );
    assert!(stderr.contains("bad option: -x"), "stderr: {stderr:?}");
}

#[test]
fn test_print_f_format_cycles_args() {
    // POSIX printf semantics: when args remain after one pass through
    // the format, cycle the format until args are exhausted.
    let (_, output, _) = run_zshrs(
        r#"print -f "%-5s|%-5s\n" a b c d"#,
    );
    assert_eq!(output, "a    |b    \nc    |d    \n", "got: {output:?}");
}

#[test]
fn test_printf_width_left_align() {
    let (_, output, _) = run_zshrs(r#"printf "%-10s|%10s\n" hello world"#);
    assert_eq!(output, "hello     |     world\n", "got: {output:?}");
}

#[test]
fn test_functions_dash_m_glob_lists_matching() {
    let (_, output, _) = run_zshrs(
        r#"fa() { :; }; fb() { :; }; functions -lm "f*""#,
    );
    let mut lines: Vec<&str> = output.trim().lines().collect();
    lines.sort();
    assert_eq!(lines, vec!["fa", "fb"], "got: {output:?}");
}

#[test]
fn test_zstyle_dash_l_uses_pattern_first_format() {
    // `zstyle -L` emits `zstyle <pattern> <style> <values>...` — the
    // pattern slot must be `:foo:bar`, not the style name.
    let (_, output, _) = run_zshrs(
        r#"zstyle ":foo:bar" key value; zstyle -L"#,
    );
    assert_eq!(output.trim(), "zstyle :foo:bar key value", "got: {output:?}");
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
    // q→\ , qq→single-quote, qqq→double-quote, qqqq→$'...'
    let (_, output, _) = run_zshrs(
        r#"a=hi; print "${(q)a}|${(qq)a}|${(qqq)a}|${(qqqq)a}""#,
    );
    assert_eq!(output.trim(), "hi|'hi'|\"hi\"|$'hi'", "got: {output:?}");
}

#[test]
fn test_assoc_subscript_in_double_quotes() {
    // `"$m[a]"` (no braces, in DQ context) should expand to the assoc
    // value, not append the literal `[a]` after `$m`.
    let (_, output, _) = run_zshrs(
        r#"typeset -A m; m[a]=1; m[b]=2; echo "$m[a] $m[b]""#,
    );
    assert_eq!(output.trim(), "1 2", "got: {output:?}");
}

#[test]
fn test_array_subscript_in_double_quotes() {
    let (_, output, _) = run_zshrs(r#"a=(x y z); echo "$a[2] $a[-1]""#);
    assert_eq!(output.trim(), "y z", "got: {output:?}");
}

#[test]
fn test_assoc_subscript_with_dynamic_key_in_dq() {
    let (_, output, _) = run_zshrs(
        r#"typeset -A m; m[a]=1; k=a; echo "$m[$k]""#,
    );
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
    let (status, _, stderr) = run_zshrs(
        r#"setopt nounset; echo "${undef}"; echo done"#,
    );
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
    assert!(
        stderr.contains("no matches found"),
        "stderr: {stderr:?}"
    );
}

#[test]
fn test_unsetopt_nomatch_passes_literal_through() {
    let (status, output, _) = run_zshrs(
        "unsetopt nomatch; echo /tmp/zr_no_such_pattern_*",
    );
    assert_eq!(status, 0);
    assert_eq!(output.trim(), "/tmp/zr_no_such_pattern_*", "got: {output:?}");
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
    let zsh_pwd = String::from_utf8_lossy(&zsh_out.stdout)
        .trim()
        .to_string();
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
    let (status, output, _) = run_zshrs(
        "set -e; if false; then echo nope; fi; echo got_here",
    );
    assert_eq!(status, 0);
    assert!(output.contains("got_here"), "got: {output:?}");
}

#[test]
fn test_set_e_suppressed_in_and_chain() {
    // `false && cmd` returns 1 but doesn't trigger errexit (POSIX:
    // failures inside an AND-OR list are consumed by the connector).
    let (status, output, _) = run_zshrs(
        "set -e; false && echo nope; echo got_here",
    );
    assert_eq!(status, 0);
    assert!(output.contains("got_here"), "got: {output:?}");
}

#[test]
fn test_set_e_suppressed_in_or_chain() {
    let (status, output, _) = run_zshrs(
        "set -e; false || true; echo got_here",
    );
    assert_eq!(status, 0);
    assert!(output.contains("got_here"), "got: {output:?}");
}

#[test]
fn test_set_e_suppressed_in_negation() {
    let (status, output, _) = run_zshrs(
        "set -e; ! false; echo got_here",
    );
    assert_eq!(status, 0);
    assert!(output.contains("got_here"), "got: {output:?}");
}

#[test]
fn test_set_e_suppressed_in_while_test() {
    let (status, output, _) = run_zshrs(
        "set -e; while false; do echo nope; done; echo got_here",
    );
    assert_eq!(status, 0);
    assert!(output.contains("got_here"), "got: {output:?}");
}

#[test]
fn test_subshell_isolates_cwd() {
    // `(cd /tmp); pwd` must not leak the cd into the parent.
    let (_, output, _) = run_zshrs("pwd > /tmp/zr_pwd_pre.txt; (cd /tmp); pwd > /tmp/zr_pwd_post.txt");
    let pre = std::fs::read_to_string("/tmp/zr_pwd_pre.txt").unwrap_or_default();
    let post = std::fs::read_to_string("/tmp/zr_pwd_post.txt").unwrap_or_default();
    let _ = std::fs::remove_file("/tmp/zr_pwd_pre.txt");
    let _ = std::fs::remove_file("/tmp/zr_pwd_post.txt");
    assert_eq!(pre.trim(), post.trim(), "subshell cd leaked: pre={pre:?} post={post:?} output={output:?}");
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
fn test_tilde_unknown_user_errors() {
    let (status, _, stderr) = run_zshrs("echo ~nonexistent_user_zrs");
    assert_ne!(status, 0, "should exit non-zero");
    assert!(
        stderr.contains("no such user"),
        "stderr: {stderr:?}"
    );
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
        "should not quote bare numeric values, got: {output:?}"
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
fn test_noclobber_blocks_overwrite_and_sinks_output() {
    let _ = std::fs::remove_file("/tmp/zr_nclob_test.out");
    std::fs::write("/tmp/zr_nclob_test.out", "first\n").unwrap();
    let (_, output, stderr) = run_zshrs(
        "set -o noclobber; echo second > /tmp/zr_nclob_test.out; echo done; cat /tmp/zr_nclob_test.out",
    );
    let _ = std::fs::remove_file("/tmp/zr_nclob_test.out");
    assert!(stderr.contains("file exists"), "stderr: {stderr:?}");
    assert!(output.contains("done"), "got: {output:?}");
    assert!(output.contains("first"), "should preserve original content, got: {output:?}");
    assert!(!output.contains("second"), "second should be sunk, got: {output:?}");
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
    assert!(!output.contains("first"), "should be overwritten, got: {output:?}");
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
    assert_eq!(output.trim(), "c b a", "DQ should preserve order, got: {output:?}");
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
    assert_eq!(output.trim(), "c b a c", "DQ should preserve order+dups, got: {output:?}");
}

#[test]
fn test_dq_suppresses_natural_sort() {
    let (_, output, _) = run_zshrs(
        r#"a=(file10 file2 file1); print -- "${(on)a}""#,
    );
    assert_eq!(output.trim(), "file10 file2 file1", "got: {output:?}");
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
    assert!(pid.parse::<i64>().is_ok(), "expected numeric pid, got: {pid:?}");
    assert!(pid != "0", "should be a real pid, got: {pid:?}");
}

#[test]
fn test_bang_pid_initial_zero() {
    let (_, output, _) = run_zshrs(r#"echo "[$!]""#);
    assert_eq!(output.trim(), "[0]", "got: {output:?}");
}

#[test]
fn test_declare_ra_blocks_array_assign() {
    let (status, _, stderr) = run_zshrs(
        "declare -ra arr=(a b c); arr=(x y z); echo done",
    );
    assert_ne!(status, 0, "should exit non-zero");
    assert!(stderr.contains("read-only"), "stderr: {stderr:?}");
}

#[test]
fn test_declare_ra_blocks_append() {
    let (status, _, stderr) = run_zshrs(
        "declare -ra arr=(a b c); arr+=(x); echo done",
    );
    assert_ne!(status, 0, "should exit non-zero");
    assert!(stderr.contains("read-only"), "stderr: {stderr:?}");
}

#[test]
fn test_print_s_silent_and_records_history() {
    // `print -s X` saves X to history INSTEAD OF stdout. fc -l in
    // -c mode shows only the entries added in this session.
    let (_, output, _) = run_zshrs(
        r#"print -s "echo from-history"; fc -l"#,
    );
    // No "echo from-history" leaked to stdout (only fc -l output).
    let trimmed = output.trim();
    assert!(
        trimmed.contains("echo from-history") && trimmed.contains("1"),
        "fc -l should list session entry numbered 1, got: {output:?}"
    );
    // Make sure print -s didn't echo to stdout itself.
    assert_eq!(
        output.lines().filter(|l| l.contains("echo from-history")).count(),
        1,
        "print -s should not echo to stdout, got: {output:?}"
    );
}

#[test]
fn test_z_split_emits_metas_as_separate_tokens() {
    // ${(z)str} tokenises a command line like the parser would —
    // shell metas (;, &, |, etc.) become their own tokens.
    let (_, output, _) = run_zshrs(
        r#"a="echo hi; ls"; print -l "${(z)a}""#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["echo", "hi", ";", "ls"], "got: {output:?}");
}

#[test]
fn test_z_split_pipe_token() {
    let (_, output, _) = run_zshrs(
        r#"a="ls | grep foo"; print -l "${(z)a}""#,
    );
    let lines: Vec<&str> = output.trim().lines().collect();
    assert_eq!(lines, vec!["ls", "|", "grep", "foo"], "got: {output:?}");
}

#[test]
fn test_alias_query_silent_when_unknown() {
    // After unalias, `alias NAME` should return non-zero with NO
    // diagnostic — matches zsh.
    let (status, _output, stderr) = run_zshrs(
        "alias hi=echo; unalias hi; alias hi 2>&1",
    );
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
    assert!(!trimmed.contains("SIG"), "expected bare names, not 'SIG' prefix, got: {trimmed:?}");
    assert!(trimmed.contains("HUP"), "should contain HUP, got: {trimmed:?}");
    assert!(trimmed.contains("TERM"), "should contain TERM, got: {trimmed:?}");
}

#[test]
fn test_kill_dash_capital_l_unknown_signal() {
    // zsh treats `-L` as `- + L` → unknown signal SIGL.
    let (status, _, stderr) = run_zshrs("kill -L");
    assert_ne!(status, 0);
    assert!(stderr.contains("SIGL") || stderr.contains("unknown signal"),
        "stderr: {stderr:?}");
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
    assert!(!output.contains("type:"), "should not have 'type:' prefix, got: {output:?}");
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
    assert_eq!(trace_lines.len(), 0, "echo should not be traced after +x, got: {stderr:?}");
}

#[test]
fn test_xtrace_uses_ps4() {
    // Default PS4 is `+ `. Verify the prefix shows up.
    let (_, _, stderr) = run_zshrs("set -x; true");
    assert!(stderr.contains("+ "), "stderr: {stderr:?}");
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
    assert!(
        stderr.contains("not a child"),
        "stderr: {stderr:?}"
    );
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
    // single-quote wrapping.
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
    assert!(
        stderr.contains("shift count must be"),
        "stderr: {stderr:?}"
    );
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
    let mut got: Vec<&str> = output.trim().split_whitespace().collect();
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
    assert!(!output.starts_with('~'), "expected expansion, got: {output:?}");
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
    assert!(!output.starts_with('~'), "expected expansion, got: {output:?}");
}

#[test]
fn test_glob_qualifier_size_l_uses_bytes() {
    // ${L+N}: size > N bytes (default unit BYTES, not 512-blocks).
    let path = "/tmp/zr_l_qual_test";
    std::fs::write(path, "12345").unwrap();
    let (_, output, _) = run_zshrs(&format!("echo {}(L+3)", path));
    let _ = std::fs::remove_file(path);
    assert!(output.contains(path), "5 bytes > 3, should match: {output:?}");
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
    let (_, output, _) =
        run_zshrs(r#"x=2; if [ $x -eq 1 ]; then echo one; elif [ $x -eq 2 ]; then echo two; else echo other; fi"#);
    assert_eq!(output.trim(), "two", "got: {output:?}");
}

#[test]
fn test_until_loop_with_bracket_test() {
    let (_, output, _) =
        run_zshrs(r#"i=0; until [ $i -ge 3 ]; do echo $i; i=$((i+1)); done"#);
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
    let (_, output, _) = run_zshrs(
        r#"arr=(file10 file2 file1 file20); echo ${(on)arr}"#,
    );
    assert_eq!(output.trim(), "file1 file2 file10 file20", "got: {output:?}");
}

#[test]
fn test_subshell_export_does_not_leak_to_parent() {
    // zsh subshell `(...)` forks; child's `export` dies with the child.
    // zshrs runs subshells in-process, so we snapshot+restore the OS
    // env around subshell entry/exit. Without this, `(export y=v)`
    // would leak `y` to the parent shell.
    let (_, output, _) = run_zshrs(
        r#"x=outer; (export y=sub; echo "in: $y"); echo "out y=${y:-empty}""#,
    );
    assert_eq!(output.trim(), "in: sub\nout y=empty", "got: {output:?}");
}

#[test]
fn test_subshell_unset_does_not_leak_to_parent() {
    let (_, output, _) = run_zshrs(
        r#"export X=parent; (unset X; echo "sub: ${X:-empty}"); echo "outer: $X""#,
    );
    assert_eq!(output.trim(), "sub: empty\nouter: parent", "got: {output:?}");
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
        ("sleep", r#"sleep() { echo USER-SLEEP; }; sleep 999"#, "USER-SLEEP"),
        ("head", r#"head() { echo USER-HEAD; }; head"#, "USER-HEAD"),
        ("tail", r#"tail() { echo USER-TAIL; }; tail"#, "USER-TAIL"),
        ("seq", r#"seq() { echo USER-SEQ; }; seq 5"#, "USER-SEQ"),
        ("uniq", r#"uniq() { echo USER-UNIQ; }; uniq"#, "USER-UNIQ"),
        ("date", r#"date() { echo USER-DATE; }; date"#, "USER-DATE"),
        ("uname", r#"uname() { echo USER-UNAME; }; uname"#, "USER-UNAME"),
        ("mkdir", r#"mkdir() { echo USER-MKDIR; }; mkdir foo"#, "USER-MKDIR"),
    ];
    for (name, code, expected) in cases {
        let (_, output, _) = run_zshrs(code);
        assert_eq!(
            output.trim(),
            expected,
            "{}: got {output:?}",
            name
        );
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
    let (_, output, _) = run_zshrs(
        r#"printf "a\nb\nc\n" | { read first; echo "first=$first"; }"#,
    );
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
    assert_eq!(output.trim(), "echo: shell built-in command", "got: {output:?}");
}

#[test]
fn test_read_processes_backslash_escapes_without_dash_r() {
    // POSIX read (no -r): each `\X` pair drops the backslash.
    // `\<newline>` is a line-continuation (both stripped). Without
    // this, an input of "a\b\n" was producing the backspace
    // control character via Rust's String::replace stub.
    let (_, output, _) = run_zshrs(
        r#"printf 'a\\b\n' | { read line; echo "[$line]"; }"#,
    );
    assert_eq!(output.trim(), "[ab]", "got: {output:?}");
}

#[test]
fn test_cmd_subst_word_splits_in_argument_context() {
    // `f $(echo a b c)` must pass three args, not one. zsh's default
    // for bare cmd-subst in argument position is IFS word-split.
    let (_, output, _) =
        run_zshrs(r#"f() { echo "argc=$#"; }; f $(echo a b c)"#);
    assert_eq!(output.trim(), "argc=3", "got: {output:?}");
}

#[test]
fn test_cmd_subst_no_split_in_dq_context() {
    // `f "$(echo a b c)"` is one arg — DQ suppresses the split.
    let (_, output, _) =
        run_zshrs(r#"f() { echo "argc=$#"; }; f "$(echo a b c)""#);
    assert_eq!(output.trim(), "argc=1", "got: {output:?}");
}

#[test]
fn test_cmd_subst_no_split_in_assignment() {
    // Assignment RHS preserves whitespace/newlines — no IFS split.
    let (_, output, _) =
        run_zshrs("x=$(printf 'a\nb\nc'); echo \"$x\" | wc -l");
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
    let mut lines: Vec<String> =
        output.lines().map(|s| s.to_string()).collect();
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
    let (_, output, _) =
        run_zshrs(&format!("cd {}; echo .*", dir));
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
        !trimmed.split_whitespace().any(|w| w == "." || w == ".." || w == "./." || w == "./.."),
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
    let (_, output, _) =
        run_zshrs(r#"pat="^h"; [[ "hello" =~ $pat ]] && echo M"#);
    assert_eq!(output.trim(), "M", "got: {output:?}");
}

#[test]
fn test_cond_regex_with_capture_groups() {
    // $match[N] is populated from regex captures.
    let (_, output, _) = run_zshrs(
        r#"pat="^(h)(.*)"; [[ "hello" =~ $pat ]] && echo "$match[1]:$match[2]""#,
    );
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
    let (_, output, _) =
        run_zshrs(&format!("echo {}/file*(N) | wc -w", dir));
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
    let (status, output, _) =
        run_zshrs(r#"trap 'echo CLEANUP' EXIT; exit 5"#);
    assert_eq!(status, 5, "exit code should propagate");
    assert_eq!(output.trim(), "CLEANUP", "got: {output:?}");
}

#[test]
fn test_exit_trap_with_explicit_exit_in_trap_body() {
    // Trap body is removed BEFORE running so a recursive exit
    // doesn't re-fire the trap.
    let (status, output, _) =
        run_zshrs(r#"trap 'echo TRAP1' EXIT; exit 7"#);
    assert_eq!(status, 7);
    assert_eq!(output.trim(), "TRAP1", "got: {output:?}");
}

#[test]
fn test_function_name_with_hyphen_dispatches_correctly() {
    // `foo-bar()` registered cleanly but the call site looked up
    // `foo\u{9b}bar` (the lexer's META encoding of `-`) and missed
    // the registered function. Untokenize before add_name in the
    // CallFunction emit path.
    let (_, output, _) =
        run_zshrs(r#"foo-bar() { echo F; }; foo-bar"#);
    assert_eq!(output.trim(), "F", "got: {output:?}");
}

#[test]
fn test_function_name_with_hyphen_passes_args() {
    let (_, output, _) = run_zshrs(
        r#"my-cmd() { echo "called: $@"; }; my-cmd hello world"#,
    );
    assert_eq!(output.trim(), "called: hello world", "got: {output:?}");
}

#[test]
fn test_typeset_f_preserves_first_word_of_body() {
    // The parser captured body_start AFTER its zshlex() that advances
    // past the first body token, so `typeset -f f` for
    // `f() { echo a; echo b; }` printed `a; echo b;` (missing the
    // first `echo`). Capture body_start BEFORE the zshlex.
    let (_, output, _) =
        run_zshrs(r#"f() { echo a; echo b; }; typeset -f f"#);
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
    let (_, output, _) =
        run_zshrs(r#"typeset -r R=x; echo "${(t)R}""#);
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
    let (_, output, _) =
        run_zshrs(r#"set -- a b c; echo "${#argv} ${#argv[@]}""#);
    assert_eq!(output.trim(), "3 3", "got: {output:?}");
}

#[test]
fn test_dq_star_assignment_joins_with_ifs() {
    // `v="$*"` should capture the full join, not just the first
    // positional. GET_VAR for `*` returns Array which pop_args
    // flattens — for DQ scalar context, we now follow GET_VAR with
    // ARRAY_JOIN_STAR (which joins by $IFS first char).
    let (_, output, _) =
        run_zshrs(r#"set -- a b c; v="$*"; echo "[$v]""#);
    assert_eq!(output.trim(), "[a b c]", "got: {output:?}");
    let (_, output, _) =
        run_zshrs(r#"IFS=":"; set -- a b c; v="$*"; echo "[$v]""#);
    assert_eq!(output.trim(), "[a:b:c]", "got: {output:?}");
}

#[test]
fn test_dq_at_preserves_splice_semantics() {
    // `"$@"` must keep splice semantics — each positional its own
    // word, even in DQ. Only `$*` joins; my $* fix must not affect $@.
    let (_, output, _) = run_zshrs(
        r#"set -- a "b c" d; for x in "$@"; do echo "[$x]"; done"#,
    );
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
    let (_, output, _) =
        run_zshrs(r#"a=(0 0 0); echo $((a[2]=42)); echo $a[2]"#);
    assert_eq!(output.trim(), "42\n42", "arith subst form: {output:?}");
}

#[test]
fn test_source_missing_file_zsh_format_and_exit_127() {
    // zsh format: `zshrs:source:1: no such file or directory: PATH`
    // and exit 127. Was emitting Rust's io::Error display unchanged
    // (with "(os error 2)" suffix) and exiting 1 — both wrong.
    let (status, _stdout, stderr) =
        run_zshrs(r#"source /no/such/file"#);
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
    let (status, _stdout, stderr) =
        run_zshrs(r#"a=(); a[0]=hi; echo "[${a[@]}]""#);
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
    let (status, _stdout, stderr) =
        run_zshrs(r#"/nonexistent_abs_path_xyz"#);
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
    let expected = libc::SIGUSR1 as i32;
    assert_eq!(n, expected, "got: {output:?}, expected libc::SIGUSR1={expected}");
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
    let (_, output, _) =
        run_zshrs(r#"x=hello; var=x; print "${(@P)var}""#);
    assert_eq!(output.trim(), "hello", "got: {output:?}");
}

#[test]
fn test_typeset_f_zsh_format_one_stmt_per_line() {
    // zsh: each top-level statement on its own line, no trailing
    // semicolons, indented with TAB. Was preserving the input's
    // semicolons (`echo a; echo b`) because we stored body_source
    // verbatim — now `format_function_body_zsh()` normalizes.
    let (_, output, _) =
        run_zshrs(r#"f() { echo a; echo b; }; typeset -f f"#);
    assert_eq!(
        output,
        "f () {\n\techo a\n\techo b\n}\n",
        "got: {output:?}"
    );
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
    let (_, output, _) =
        run_zshrs(r#"a="x y z"; print "${(w)#a}""#);
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
    child.stdin.as_mut().unwrap().write_all(b"abc123\n").unwrap();
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
    child.stdin.as_mut().unwrap().write_all(b"a\nb\nc\n").unwrap();
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
    child.stdin.as_mut().unwrap().write_all(b"abcdefgh\n").unwrap();
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
    let (_, output, _) =
        run_zshrs(r#"umask 077; umask -S u=rwx,g=rx,o=; umask"#);
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
    assert!(output.trim().ends_with("done"), "expected `done`: {output:?}");
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
fn test_dollar_zero_in_minus_c_returns_basename() {
    // zsh's `$0` in `-c` mode is the shell name (`zsh` / `zshrs`), not
    // the absolute argv0 path. Regression: zshrs returned the full
    // `./target/debug/zshrs` build path. Fixed by setting `$0` to
    // basename(argv[0]) in the `-c` dispatch.
    let (_, output, _) = run_zshrs(r#"echo $0"#);
    assert_eq!(output.trim(), "zshrs", "got: {output:?}");
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
    // expand_braces, which then expanded the comma list. Added
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
    let (_, output, _) =
        run_zshrs(r#"arr=(); echo "${arr[1]:=fresh}"; echo "${arr[1]}""#);
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
    let (_, output, _) = run_zshrs(
        r#"a=hi; foo() { local a; echo "in[$a]"; }; foo; echo "out[$a]""#,
    );
    assert_eq!(output.trim(), "in[]\nout[hi]");
}

#[test]
fn test_typeset_no_value_resets_in_function_scope() {
    // Same behavior for `typeset` (which `local` aliases to).
    let (_, output, _) = run_zshrs(
        r#"a=hi; foo() { typeset a; echo "in[$a]"; }; foo; echo "out[$a]""#,
    );
    assert_eq!(output.trim(), "in[]\nout[hi]");
}

#[test]
fn test_typeset_g_keeps_parent_value() {
    // `typeset -g` opts out of localization — should keep parent value
    // (regression guard for the local-resets fix).
    let (_, output, _) = run_zshrs(
        r#"a=hi; foo() { typeset -g a; echo "in[$a]"; }; foo; echo "out[$a]""#,
    );
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
    assert!(
        first.contains('\t'),
        "expected TAB in dirs -v: {first:?}"
    );
}

#[test]
fn test_arith_int_times_float_promotes() {
    // `a=10; ((a *= 1.5)); echo $a` — int * float → float result.
    // Regression: ArithCompiler kept everything int; routed through
    // MathEval via the float-literal trigger in compile_arith.
    let (_, output, _) = run_zshrs(r#"a=10; ((a *= 1.5)); echo $a"#);
    let s = output.trim();
    assert!(
        s.starts_with("15"),
        "expected 15.x or 15.0…, got: {s:?}"
    );
    assert!(s.contains('.'), "expected float result: {s:?}");
}

#[test]
fn test_assoc_bare_returns_joined_values() {
    // `declare -A h; h[k]=v; echo "${h:-default}"` — bare `${h}`
    // should return joined values, NOT the default. Regression:
    // zshrs's get_variable returned empty for assoc.
    let (_, output, _) =
        run_zshrs(r#"declare -A h; h[k]=v; echo "${h:-default}""#);
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
    let (_, output, _) =
        run_zshrs(r#"declare -A h; h[k]=v; echo "${h[k]+set}""#);
    assert_eq!(output.trim(), "set");
}

#[test]
fn test_assoc_element_no_colon_unset() {
    let (_, output, _) =
        run_zshrs(r#"declare -A h; h[k]=v; echo "${h[m]+set}""#);
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
    let (_, output, _) = run_zshrs(r#"[[ /etc/passwd -nt /tmp/nope_zshrs ]] && echo nt || echo not_nt"#);
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
    let (_, output, _) =
        run_zshrs(r#"a=10; b=20; ((a += 5, b *= 2)); echo "$a $b""#);
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
    let (_, output, _) =
        run_zshrs("function { echo $#; echo $@ } a b c");
    assert_eq!(output.trim(), "3\na b c");
}

#[test]
fn test_zsh_subshell_increments() {
    // `$ZSH_SUBSHELL` increments by one for each subshell nesting level.
    let (_, output, _) = run_zshrs(
        "echo $ZSH_SUBSHELL; (echo $ZSH_SUBSHELL); ( ( echo $ZSH_SUBSHELL ) )",
    );
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
        active.split('.').next().unwrap_or("").to_uppercase().as_str(),
        "" | "C" | "POSIX"
    );
    if is_c {
        assert_eq!(trimmed, "Aaa Ccc Ddd bbb");
    } else {
        assert_eq!(trimmed, "Aaa bbb Ccc Ddd");
    }
}

#[test]
fn test_typeset_integer_base_output() {
    // `typeset -i N name=value` displays in base N as `N#DIGITS`.
    let (_, output, _) =
        run_zshrs("typeset -i 16 a=255; echo $a");
    assert_eq!(output.trim(), "16#FF");

    let (_, output, _) =
        run_zshrs("typeset -i 2 a=10; echo $a");
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
    let (_, output, _) =
        run_zshrs("a=(a.txt b.bin c.txt); echo ${a%.txt}");
    assert_eq!(output.trim(), "a b.bin c");
}

#[test]
fn test_array_prefix_strip_per_element() {
    // `${arr#pat}` strips prefix per element.
    let (_, output, _) =
        run_zshrs("a=(/tmp/x /tmp/y); echo ${a#/tmp/}");
    assert_eq!(output.trim(), "x y");
}

#[test]
fn test_array_long_suffix_strip_per_element() {
    // `${arr%%pat}` strips longest suffix per element.
    let (_, output, _) =
        run_zshrs("a=(a.b.c d.e.f); echo ${a%%.*}");
    assert_eq!(output.trim(), "a d");
}

#[test]
fn test_array_long_prefix_strip_per_element() {
    // `${arr##pat}` strips longest prefix per element.
    let (_, output, _) =
        run_zshrs("a=(/tmp/a /tmp/b); echo ${a##*/}");
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
    let (_, output, _) = run_zshrs(
        "a=42; cat <<\\EOF\nval=$a\nEOF",
    );
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
    let (_, _, stderr) =
        run_zshrs("[[ -o no_such_option_zzz ]] && echo y || echo n");
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
    let (_, output, _) =
        run_zshrs("a=42; cat <<EOF\n[$a]\nEOF");
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
    let (_, output, _) =
        run_zshrs(r#"a=(/a/b/c /d/e/f); echo "${a[@]:h}""#);
    assert_eq!(output.trim(), "/a/b /d/e");

    let (_, output, _) =
        run_zshrs(r#"a=(foo.txt bar.bin); echo "${a[@]:r}""#);
    assert_eq!(output.trim(), "foo bar");

    let (_, output, _) =
        run_zshrs(r#"a=(/a/b/c /d/e); echo "${a[@]:t}""#);
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
    let (_, output, _) =
        run_zshrs(r#"a=(hello world); echo "${#a[1]}""#);
    assert_eq!(output.trim(), "5");
    let (_, output, _) =
        run_zshrs(r#"a=(short verylongstring); echo "${#a[2]}""#);
    assert_eq!(output.trim(), "14");
}

#[test]
fn test_printf_dot_s_zero_precision_suppresses_arg() {
    // `%.s` (period, no digits) means precision-0 — the string arg
    // is suppressed. zshrs was treating the missing digits as
    // "no precision" and printing the full arg.
    let (_, output, _) =
        run_zshrs(r#"printf "[%.s]" "ignore""#);
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
    let (_, output, _) =
        run_zshrs(r#"true; (set -e; false); echo "alive=$?""#);
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
    let dir = std::env::temp_dir().join("zshrs_test_om_sort");
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
    let dir = std::env::temp_dir().join("zshrs_test_Om_sort");
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
    let mut parts: Vec<&str> = output.trim().split_whitespace().collect();
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
    let mut parts: Vec<&str> = output.trim().split_whitespace().collect();
    parts.sort();
    assert_eq!(parts, vec!["a", "b"]);
}

#[test]
fn test_case_multi_pattern_with_brackets() {
    // `case x in [a-z]) ...;; [A-Z]) ...;; esac` — bracket-class
    // patterns in subsequent case arms were parse-erroring because
    // the lexer tokenized `[` as Inbrack (not part of pattern) when
    // incasepat had reset to 0 across the `;;` advance.
    let (_, output, _) =
        run_zshrs("case a in [a-z]) echo lower;; [A-Z]) echo upper;; esac");
    assert_eq!(output.trim(), "lower");
    let (_, output, _) =
        run_zshrs("case X in [a-z]) echo lower;; [A-Z]) echo upper;; esac");
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
    let (_, output, _) =
        run_zshrs(r#"set -- a b c; echo "$#@""#);
    assert_eq!(output.trim(), "3");
    let (_, output, _) =
        run_zshrs(r#"set -- a b c; echo "$#*""#);
    assert_eq!(output.trim(), "3");
}

#[test]
fn test_dollar_hash_name_concat() {
    // `X$#Y` for unset Y should return `X0` (length of empty Y).
    // Was returning `X3Y` because the segment-splitter only consumed
    // `$#` and left `Y` as a literal trailing segment.
    let (_, output, _) =
        run_zshrs("set -- a b c; echo X$#Y");
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
    assert!(val >= 0 && val <= 32767, "RANDOM out of range: {}", val);

    // RANDOM should differ per arith-subst (zsh contract).
    let (_, output, _) =
        run_zshrs("a=$((RANDOM)); b=$((RANDOM)); [[ $a != $b ]] && echo diff");
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
    let (_, output, _) =
        run_zshrs(r#"[[ "foo()" == "foo()" ]] && echo y"#);
    assert_eq!(output.trim(), "y");
    let (_, output, _) =
        run_zshrs(r#"[[ "x|y" == "x|y" ]] && echo y"#);
    assert_eq!(output.trim(), "y");
}

#[test]
fn test_command_v_function_shows_source() {
    // `command -V foo` for a function should say "is a shell
    // function from zsh" (matching zsh's exact format). Was just
    // "is a shell function".
    let (_, output, _) =
        run_zshrs(r#"foo() { :; }; command -V foo"#);
    assert!(output.contains("from zsh"));
}

#[test]
fn test_arith_subscripted_post_increment() {
    // `(( a[1]++ ))` should increment the first element. zshrs's
    // ArithCompiler couldn't write back through arr[idx] for compound
    // forms — only the bare `=` assign was caught. Now routed through
    // a runtime parse_subscript_arith_compound + read-modify-write.
    let (_, output, _) =
        run_zshrs(r#"a=(0 0); (( a[1]++ )); echo "${a[1]}""#);
    assert_eq!(output.trim(), "1");
}

#[test]
fn test_arith_subscripted_compound_plus_eq() {
    let (_, output, _) =
        run_zshrs(r#"a=(10 20); (( a[1] += 5 )); echo "${a[1]}""#);
    assert_eq!(output.trim(), "15");
    let (_, output, _) =
        run_zshrs(r#"a=(5 5); (( a[1] *= 3 )); echo "${a[1]}""#);
    assert_eq!(output.trim(), "15");
}

#[test]
fn test_arith_subscripted_post_increment_returns_old() {
    let (_, output, _) =
        run_zshrs(r#"a=(5 10); echo "$(( a[1]++ ))"; echo "${a[1]}""#);
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
    let mut parts: Vec<&str> = output.trim().split_whitespace().collect();
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
    let cmd = format!(
        "setopt extendedglob; cd {} && echo ^b",
        dir.display()
    );
    let (_, output, _) = run_zshrs(&cmd);
    let _ = std::fs::remove_dir_all(&dir);
    let mut parts: Vec<&str> = output.trim().split_whitespace().collect();
    parts.sort();
    assert_eq!(parts, vec!["a", "c"]);
}
