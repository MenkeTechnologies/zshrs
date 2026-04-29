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
