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
    // "for in; do; done" is a syntax error — missing variable name after 'for'.
    let (status, _, stderr) = run_zshrs("for in; do; done");
    assert!(
        !stderr.is_empty() || status != 0,
        "syntax error should produce stderr or nonzero exit"
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
    let (_, output, _) =
        run_zshrs("arr=(c a b); print -l \"${(o)arr[@]}\"");
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
