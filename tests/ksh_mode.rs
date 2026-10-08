//! Tests for `zshrs --ksh` drop-in mode.
//!
//! Verifies the CLI flag is parsed, the ksh option presets are
//! applied (matching `emulate ksh` from Src/options.c), and basic
//! ksh-style behaviors work (0-indexed arrays via `ksharrays`).

use std::process::Command;

fn zshrs_bin() -> String {
    env!("CARGO_BIN_EXE_zshrs").to_string()
}

/// Assert an option's state under `--ksh` the way the shell itself reports it.
///
/// These tests used to grep `setopt`'s listing. That cannot work: `emulate ksh`
/// sets KSH_OPTION_PRINT, which switches `setopt` from bare names to a
/// two-column `name<pad>on|off` table — real zsh does exactly the same, and
/// zshrs matches it byte-for-byte:
///
/// ```text
/// $ zsh   -fc 'emulate ksh; setopt' | head -1   ->  noaliases             off
/// $ zshrs --ksh -c 'setopt'         | head -1   ->  noaliases             off
/// ```
///
/// So `setopt | grep -x ksharrays` could never match, and the five "sets"
/// tests failed against correct behaviour. Worse, the two "unsets" tests
/// PASSED for the wrong reason — grep matched nothing for any option, so
/// `|| echo absent` fired regardless of the real state and they asserted
/// nothing at all.
///
/// `[[ -o NAME ]]` reads the option state itself, so it is independent of the
/// listing format and strictly stronger than the grep it replaces. Every
/// expectation below was verified against `zsh -fc 'emulate ksh; [[ -o X ]]'`.
fn ksh_opt(name: &str) -> String {
    let (out, _, _) = run_ksh(&format!("[[ -o {name} ]] && echo on || echo off"));
    out.trim().to_string()
}

fn run_ksh(script: &str) -> (String, String, i32) {
    let out = Command::new(zshrs_bin())
        .args(["--ksh", "-c", script])
        .output()
        .expect("zshrs --ksh failed to spawn");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn ksh_mode_sets_ksharrays() {
    // ksharrays makes arrays 0-indexed (vs zsh's default 1).
    assert_eq!(ksh_opt("ksharrays"), "on", "--ksh must set ksharrays");
}

#[test]
fn ksh_mode_sets_kshglob() {
    assert_eq!(ksh_opt("kshglob"), "on", "--ksh must set kshglob");
}

#[test]
fn ksh_mode_sets_posixbuiltins() {
    assert_eq!(ksh_opt("posixbuiltins"), "on", "--ksh must set posixbuiltins");
}

#[test]
fn ksh_mode_sets_shwordsplit() {
    assert_eq!(ksh_opt("shwordsplit"), "on", "--ksh must set shwordsplit");
}

#[test]
fn ksh_mode_unsets_nomatch() {
    // Per emulate_mode_options("ksh"): nomatch is in the unset list.
    assert_eq!(ksh_opt("nomatch"), "off", "--ksh must unset nomatch");
}

#[test]
fn ksh_mode_unsets_multios() {
    assert_eq!(ksh_opt("multios"), "off", "--ksh must unset multios");
}

#[test]
fn ksh_mode_zero_indexed_arrays() {
    // Now a REAL behavioural check, not just the option bit. Under
    // KSH_ARRAYS `${a[0]}` is the first element, where zsh's default is
    // 1-based. Verified against `zsh -fc 'emulate ksh; ...'`:
    //   ${a[0]} -> x    ${a[1]} -> y    ${#a[@]} -> 3
    assert_eq!(ksh_opt("ksharrays"), "on");
    let (out, _, _) = run_ksh(r#"a=(x y z); print -r -- "${a[0]}|${a[1]}|${#a[@]}""#);
    assert_eq!(
        out.trim(),
        "x|y|3",
        "ksharrays must make subscript 0 the first element"
    );
}

#[test]
fn ksh_mode_help_lists_flag() {
    let out = Command::new(zshrs_bin())
        .arg("--help")
        .output()
        .expect("zshrs --help failed");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("--ksh"),
        "--help output missing --ksh flag:\n{}",
        stdout
    );
}

/// stdout bytes of `zshrs <flag> -f -c <script>`.
fn run_flag(flag: &str, script: &str) -> (Vec<u8>, i32) {
    let out = Command::new(zshrs_bin())
        .args([flag, "-f", "-c", script])
        .output()
        .expect("zshrs failed to spawn");
    (out.stdout, out.status.code().unwrap_or(-1))
}

/// ksh93's `print` / `echo -e` / `printf %b` decode a narrower escape set than
/// zsh: `\E` (not `\e`), `\0NNN` (not bare `\NNN`), `\uHHHH` (not `\x` / `\U`);
/// `\c` ends the output; `print -e` is an accepted no-op. Values measured on
/// ksh93u+m (`/opt/homebrew/bin/ksh`).
#[test]
fn ksh93_print_escape_set_is_narrower_than_zsh() {
    let cases: &[(&str, &[u8])] = &[
        (r"print -e 'a\tb'", b"a\tb\n"),
        (r"print '\0101|\x41|\101|\e|\U00000041|\q'", b"A|\\x41|\\101|\\e|\\U00000041|\\q\n"),
        (r"print '\E[0m|\u0041'", b"\x1b[0m|A\n"),
        (r"print '\00101'", b"\x081\n"),
        (r"print -r '\0101'", b"\\0101\n"),
        (r"print 'a\cb'; print x", b"ax\n"),
        (r"echo -e '\0101|\E|\x41|\e'", b"A|\x1b|\\x41|\\e\n"),
        (r"printf '%b\n' '\0101|\x41|\E|\e'", b"A|\\x41|\x1b|\\e\n"),
    ];
    for (script, want) in cases {
        let (out, _) = run_flag("--ksh", script);
        assert_eq!(out, *want, "{script}");
    }
}

/// A blank inside the subscript of a command-position `name[…]=value` word
/// belongs to the subscript in ksh93 (and bash); zsh splits the word there.
#[test]
fn ksh93_assignment_subscript_may_contain_blanks() {
    let (out, _) = run_flag("--ksh", r#"typeset -A h; h[a b]=1; print "${h[a b]}"; print ${!h[@]}"#);
    assert_eq!(out, b"1\na b\n");
    let (out, _) = run_flag("--ksh", "a[ 2 ]=x; print ${!a[@]}");
    assert_eq!(out, b"2\n");
    // Arguments are not assignments: the blank still splits `echo a[ b`.
    let (out, _) = run_flag("--ksh", "echo ok a[ b]");
    assert_eq!(out, b"ok a[ b]\n");
}

/// ksh93 scopes an EXIT trap to a `function name { … }` body; a POSIX
/// `name() { … }` function's EXIT trap belongs to the shell and fires at exit.
#[test]
fn ksh93_function_keyword_scopes_exit_trap() {
    let (out, _) = run_flag("--ksh", "function f { trap 'print bye' EXIT; print in; }; f; print after");
    assert_eq!(out, b"in\nbye\nafter\n");
    let (out, _) = run_flag("--ksh", "f() { trap 'print bye' EXIT; print in; }; f; print after");
    assert_eq!(out, b"in\nafter\nbye\n");
}

/// mksh's `trap` takes no options, and a usage error in a special builtin
/// aborts the non-interactive shell (status 1, nothing after it runs).
#[test]
fn mksh_trap_usage_error_aborts_the_script() {
    for script in ["trap -x; print after", "trap -l; print after", "trap -p; print after", "trap -ZZ INT; print after"] {
        for flag in ["--mksh", "--pdksh"] {
            let (out, code) = run_flag(flag, script);
            assert_eq!(out, b"", "{flag} {script}");
            assert_eq!(code, 1, "{flag} {script}");
        }
    }
    // `--` and a bad signal name are not option errors.
    let (out, code) = run_flag("--mksh", "trap -- 'print a' INT; trap 'print b' NOSUCH; print after");
    assert_eq!(out, b"after\n");
    assert_eq!(code, 0);
}
