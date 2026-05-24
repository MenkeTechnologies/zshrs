//! `[[ ... ]]` conditional-expression parity tests — pin each operator
//! against real zsh 5.9.
//!
//! File-test cases construct temp files and test against them.

use std::path::{Path, PathBuf};
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("zshrs")
}

fn zsh_path() -> &'static str {
    if Path::new("/opt/homebrew/bin/zsh").exists() {
        "/opt/homebrew/bin/zsh"
    } else if Path::new("/usr/local/bin/zsh").exists() {
        "/usr/local/bin/zsh"
    } else {
        "/bin/zsh"
    }
}

fn zsh_available() -> bool {
    Command::new(zsh_path())
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

struct ShellResult {
    stdout: String,
    stderr: String,
    exit: i32,
}

fn run_zsh(script: &str) -> ShellResult {
    let out = Command::new(zsh_path())
        .args(["-fc", script])
        .output()
        .expect("invoke zsh");
    ShellResult {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit: out.status.code().unwrap_or(-1),
    }
}

fn run_zshrs(script: &str) -> ShellResult {
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", script])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke zshrs");
    ShellResult {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit: out.status.code().unwrap_or(-1),
    }
}

fn assert_parity(script: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_zsh(script);
    let r = run_zshrs(script);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on script:\n{script}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        z.stdout, r.stdout
    );
    assert_eq!(z.exit, r.exit, "exit divergence on script:\n{script}");
}

mod string_eq_ne {
    use super::*;

    #[test]
    fn eq_equal_strings_true() {
        assert_parity(r#"[[ "foo" == "foo" ]]; echo $?"#);
    }

    #[test]
    fn eq_different_strings_false() {
        assert_parity(r#"[[ "foo" == "bar" ]]; echo $?"#);
    }

    #[test]
    fn ne_different_strings_true() {
        assert_parity(r#"[[ "foo" != "bar" ]]; echo $?"#);
    }

    #[test]
    fn ne_equal_strings_false() {
        assert_parity(r#"[[ "foo" != "foo" ]]; echo $?"#);
    }

    #[test]
    fn empty_strings_eq_true() {
        assert_parity(r#"[[ "" == "" ]]; echo $?"#);
    }

    #[test]
    fn case_sensitive_compare() {
        assert_parity(r#"[[ "ABC" == "abc" ]]; echo $?"#);
    }
}

mod string_lex_order {
    use super::*;

    #[test]
    fn lt_lex_order_true() {
        assert_parity(r#"[[ "apple" < "banana" ]]; echo $?"#);
    }

    #[test]
    fn lt_equal_false() {
        assert_parity(r#"[[ "apple" < "apple" ]]; echo $?"#);
    }

    #[test]
    fn gt_lex_order_true() {
        assert_parity(r#"[[ "banana" > "apple" ]]; echo $?"#);
    }

    #[test]
    fn gt_equal_false() {
        assert_parity(r#"[[ "apple" > "apple" ]]; echo $?"#);
    }
}

mod string_pattern_match {
    use super::*;

    /// `==` against an UNQUOTED pattern enables glob matching.
    #[test]
    fn pattern_star_matches_anything() {
        assert_parity(r#"[[ "hello" == * ]]; echo $?"#);
    }

    #[test]
    fn pattern_prefix_match() {
        assert_parity(r#"[[ "hello world" == hello* ]]; echo $?"#);
    }

    #[test]
    fn pattern_question_matches_one_char() {
        assert_parity(r#"[[ "abc" == a?c ]]; echo $?"#);
    }

    #[test]
    fn pattern_bracket_class() {
        assert_parity(r#"[[ "file7" == file[0-9] ]]; echo $?"#);
    }

    #[test]
    fn pattern_no_match_fails() {
        assert_parity(r#"[[ "hello" == foo* ]]; echo $?"#);
    }

    #[test]
    fn pattern_via_tilde_expansion() {
        assert_parity(r#"P="foo*"; [[ "foobar" == ${~P} ]]; echo $?"#);
    }
}

mod string_emptiness {
    use super::*;

    #[test]
    fn z_flag_empty_true() {
        assert_parity(r#"[[ -z "" ]]; echo $?"#);
    }

    #[test]
    fn z_flag_nonempty_false() {
        assert_parity(r#"[[ -z "x" ]]; echo $?"#);
    }

    #[test]
    fn n_flag_nonempty_true() {
        assert_parity(r#"[[ -n "x" ]]; echo $?"#);
    }

    #[test]
    fn n_flag_empty_false() {
        assert_parity(r#"[[ -n "" ]]; echo $?"#);
    }
}

mod arith_compare {
    use super::*;

    #[test]
    fn lt_numeric() {
        assert_parity(r#"[[ 5 -lt 10 ]]; echo $?"#);
    }

    #[test]
    fn le_numeric_equal() {
        assert_parity(r#"[[ 5 -le 5 ]]; echo $?"#);
    }

    #[test]
    fn gt_numeric() {
        assert_parity(r#"[[ 10 -gt 5 ]]; echo $?"#);
    }

    #[test]
    fn ge_numeric() {
        assert_parity(r#"[[ 5 -ge 5 ]]; echo $?"#);
    }

    #[test]
    fn eq_numeric_string_compared_as_int() {
        assert_parity(r#"[[ "10" -eq "010" ]]; echo $?"#);
    }

    #[test]
    fn ne_numeric() {
        assert_parity(r#"[[ 5 -ne 6 ]]; echo $?"#);
    }
}

mod logical_combos {
    use super::*;

    #[test]
    fn and_both_true() {
        assert_parity(r#"[[ 1 -eq 1 && 2 -eq 2 ]]; echo $?"#);
    }

    #[test]
    fn and_one_false() {
        assert_parity(r#"[[ 1 -eq 1 && 2 -eq 3 ]]; echo $?"#);
    }

    #[test]
    fn or_one_true() {
        assert_parity(r#"[[ 1 -eq 2 || 3 -eq 3 ]]; echo $?"#);
    }

    #[test]
    fn or_both_false() {
        assert_parity(r#"[[ 1 -eq 2 || 3 -eq 4 ]]; echo $?"#);
    }

    #[test]
    fn not_true_becomes_false() {
        assert_parity(r#"[[ ! 1 -eq 1 ]]; echo $?"#);
    }

    #[test]
    fn not_false_becomes_true() {
        assert_parity(r#"[[ ! 1 -eq 2 ]]; echo $?"#);
    }

    #[test]
    fn nested_grouping() {
        assert_parity(r#"[[ ( 1 -eq 1 || 2 -eq 3 ) && 4 -eq 4 ]]; echo $?"#);
    }
}

mod file_tests {
    use super::*;

    /// `-e /` (root dir exists).
    #[test]
    fn e_flag_root_exists() {
        assert_parity(r#"[[ -e / ]]; echo $?"#);
    }

    /// `-d /tmp` (directory).
    #[test]
    fn d_flag_tmp_is_directory() {
        assert_parity(r#"[[ -d /tmp ]]; echo $?"#);
    }

    /// `-f /etc/hosts` (regular file — exists on every Unix).
    #[test]
    fn f_flag_hosts_is_regular_file() {
        assert_parity(r#"[[ -f /etc/hosts ]]; echo $?"#);
    }

    /// `-r /etc/hosts` (readable).
    #[test]
    fn r_flag_hosts_is_readable() {
        assert_parity(r#"[[ -r /etc/hosts ]]; echo $?"#);
    }

    /// `-e /nonexistent_xyz_path` → false.
    #[test]
    fn e_flag_missing_is_false() {
        assert_parity(r#"[[ -e /nonexistent_xyz_path_zzz_42 ]]; echo $?"#);
    }

    /// `-d /etc/hosts` (file, not dir) → false.
    #[test]
    fn d_flag_on_regular_file_is_false() {
        assert_parity(r#"[[ -d /etc/hosts ]]; echo $?"#);
    }

    /// `-f /tmp` (dir, not file) → false.
    #[test]
    fn f_flag_on_directory_is_false() {
        assert_parity(r#"[[ -f /tmp ]]; echo $?"#);
    }

    /// `-s /etc/hosts` (non-empty file).
    #[test]
    fn s_flag_hosts_is_non_empty() {
        assert_parity(r#"[[ -s /etc/hosts ]]; echo $?"#);
    }
}

mod regex_match {
    use super::*;

    /// `=~` regex match operator (BSD ERE syntax in zsh by default).
    #[test]
    fn regex_match_simple() {
        assert_parity(r#"[[ "hello123" =~ "[0-9]+" ]]; echo $?"#);
    }

    #[test]
    fn regex_no_match() {
        assert_parity(r#"[[ "hello" =~ "[0-9]+" ]]; echo $?"#);
    }

    #[test]
    fn regex_anchored_start() {
        assert_parity(r#"[[ "abc123" =~ "^abc" ]]; echo $?"#);
    }

    #[test]
    fn regex_anchored_end() {
        assert_parity(r#"[[ "abc123" =~ "123$" ]]; echo $?"#);
    }
}

mod var_existence {
    use super::*;

    #[test]
    fn z_unset_var_is_empty_true() {
        assert_parity(r#"unset X; [[ -z "$X" ]]; echo $?"#);
    }

    #[test]
    fn n_set_var_is_nonempty_true() {
        assert_parity(r#"X=value; [[ -n "$X" ]]; echo $?"#);
    }

    #[test]
    fn z_set_to_empty_string_true() {
        assert_parity(r#"X=; [[ -z "$X" ]]; echo $?"#);
    }
}

mod options {
    use super::*;

    /// `-o optname` — check whether shell option is set.
    #[test]
    fn dash_o_unset_option_false() {
        assert_parity(r#"unsetopt extendedglob; [[ -o extendedglob ]]; echo $?"#);
    }

    #[test]
    fn dash_o_set_option_true() {
        assert_parity(r#"setopt extendedglob; [[ -o extendedglob ]]; echo $?"#);
    }

    /// `-o` with `no_*` prefix → inverted check.
    #[test]
    fn dash_o_no_prefix_inverts() {
        assert_parity(r#"setopt extendedglob; [[ -o no_extendedglob ]]; echo $?"#);
    }
}
