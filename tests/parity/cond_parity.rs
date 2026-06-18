//! `[[ ... ]]` conditional-expression parity tests — pin each operator
//! against real zsh 5.9.
//!
//! File-test cases construct temp files and test against them.
#![allow(non_snake_case)]

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

#[allow(dead_code)]
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

    /// BUGS.md #558 — POSIX-ERE bracket semantics: inside `[...]` a
    /// backslash is an ORDINARY class member (regcomp(3); zsh/regex
    /// hands the pattern verbatim to regcomp, Src/Modules/regex.c:78).
    /// `[a-z\n]` is the set {a..z, '\', 'n'} — NOT "a-z plus newline" —
    /// so the match stops at the line break.
    #[test]
    fn regex_bracket_backslash_n_stops_at_newline() {
        assert_parity(r#"a=$'a\nb'; [[ "$a" =~ "[a-z\n]+" ]] && print -r "[${MATCH}]""#);
    }

    /// BUGS.md #558 — the bracket members are literal '\' and 'n':
    /// "n" matches, a real newline does not, a literal backslash does.
    #[test]
    fn regex_bracket_backslash_is_literal_member() {
        assert_parity(r#"[[ "n" =~ "[\n]" ]] && echo yes || echo no"#);
        assert_parity(r#"nl=$'\n'; [[ $nl =~ "[\n]" ]] && echo yes || echo no"#);
        assert_parity(r#"a=$'a\\b'; [[ $a =~ "[a-z\n]+" ]] && print -r "[$MATCH]""#);
    }

    /// BUGS.md #558 sweep — POSIX named classes pass through, leading
    /// `]` is literal, `&` is an ordinary member (the Rust regex
    /// crate's `&&` set-intersection must not fire), negated class
    /// with backslash-n member, trailing `-` literal in a range class.
    #[test]
    fn regex_bracket_posix_class_and_metachars() {
        assert_parity(r#"[[ "abc" =~ "[[:alpha:]]+" ]] && print -r "[$MATCH]""#);
        assert_parity(r#"[[ "]" =~ "[]]" ]] && echo yes || echo no"#);
        assert_parity(r#"[[ "a&b" =~ "[a&b]+" ]] && print -r "[$MATCH]""#);
        assert_parity(r#"a=$'a\nb'; [[ $a =~ "[^\n]+" ]] && print -r "[$MATCH]""#);
        assert_parity(r#"[[ "x-z" =~ "[a-z-]+" ]] && print -r "[$MATCH]""#);
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

mod glob_numeric {
    use super::*;

    /// `<->` matches any non-negative integer (zsh glob in `[[ = ]]`).
    #[test]
    fn numeric_glob_angle_digits() {
        assert_parity(r#"[[ 42 = <-> ]]; echo $?"#);
    }

    #[test]
    fn numeric_glob_rejects_alpha() {
        assert_parity(r#"[[ abc = <-> ]]; echo $?"#);
    }
}

mod glob_anchors {
    use super::*;

    #[test]
    fn hash_hash_prefix_anchor() {
        assert_parity(r#"[[ host = ##host ]]; echo $?"#);
    }

    #[test]
    fn hash_hash_suffix_anchor() {
        assert_parity(r#"[[ host = host## ]]; echo $?"#);
    }

    #[test]
    fn extendedglob_repeat_hash() {
        assert_parity(r#"setopt extendedglob; [[ abc = [a-z]## ]]; echo $?"#);
    }

    #[test]
    fn extendedglob_case_insensitive_hash_i() {
        assert_parity(r#"setopt extendedglob; [[ abc = (#i)ABC ]]; echo $?"#);
    }
}

mod var_set_test {
    use super::*;

    #[test]
    fn dash_v_set_true() {
        assert_parity(r#"x=1; [[ -v x ]]; echo $?"#);
    }

    #[test]
    fn dash_v_unset_false() {
        assert_parity(r#"unset y; [[ -v y ]]; echo $?"#);
    }
}

mod file_tests_extra {
    use super::*;

    #[test]
    fn h_flag_symlink() {
        assert_parity(r#"[[ -h /dev/stdin ]]; echo $?"#);
    }

    #[test]
    fn p_flag_fifo() {
        assert_parity(r#"[[ -p /dev/fd/0 ]]; echo $?"#);
    }

    #[test]
    fn O_flag_owned_by_euid() {
        assert_parity(r#"[[ -O /etc/hosts ]]; echo $?"#);
    }

    #[test]
    fn G_flag_group_owned() {
        assert_parity(r#"[[ -G / ]]; echo $?"#);
    }

    #[test]
    fn ef_same_string_var() {
        assert_parity(r#"[[ v1 -ef v1 ]]; echo $?"#);
    }
}

mod extendedglob_anchors {
    use super::*;

    #[test]
    fn hash_m_sets_match_var() {
        assert_parity(r#"setopt extendedglob; [[ abc = (#m)[a-z]## ]]; print -r "$MATCH""#);
    }

    #[test]
    fn hash_b_capture_group() {
        assert_parity(r#"setopt extendedglob; [[ foo = (#b)oo ]]; echo $?"#);
    }

    #[test]
    fn hash_s_start_anchor() {
        assert_parity(r#"setopt extendedglob; [[ foo = (#s)fo ]]; echo $?"#);
    }

    #[test]
    fn hash_e_end_anchor() {
        assert_parity(r#"setopt extendedglob; [[ foo = fo(#e) ]]; echo $?"#);
    }
}

mod file_compare {
    use super::*;

    #[test]
    fn ef_same_path() {
        assert_parity(r#"[[ /etc/hosts -ef /etc/hosts ]]; echo $?"#);
    }

    #[test]
    fn nt_newer_than() {
        assert_parity(r#"[[ /etc/hosts -nt /tmp ]]; echo $?"#);
    }

    #[test]
    fn ot_older_than() {
        assert_parity(r#"[[ /tmp -ot /etc/hosts ]]; echo $?"#);
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

mod round_pins {
    use super::*;

    #[test]
    fn ef_missing_paths() {
        assert_parity(r#"[[ a -ef b ]] || echo 1; print -r $?"#);
    }

    #[test]
    fn nt_hosts_passwd() {
        assert_parity(r#"[[ /etc/hosts -nt /etc/passwd ]]; print -r $?"#);
    }

    #[test]
    fn symlink_tests() {
        assert_parity(r#"[[ -h /tmp ]] || print -r 0"#);
    }

    #[test]
    fn sticky_bit_tmp() {
        assert_parity(r#"[[ -k /tmp ]]; print -r $?"#);
    }
}

/// Bug #628 — single-char special parameters (`$$`, `$?`, `$#`, `$-`,
/// `$*`) on the pattern side of `[[ == ]]` / `case`. The compile-time
/// pattern splitter (`split_pattern_for_glob_subst`) consumed only
/// `[A-Za-z0-9_]` after `$`, so a special leaked into the literal
/// segment and the RHS compiled to the raw 2-char pattern (e.g. `$$`)
/// instead of the substituted value. C: singsub → paramsubst handles
/// these specials (Src/subst.c:2024+); Src/cond.c:303-310 singsubs the
/// raw RHS before patcompile.
mod special_param_pattern_rhs {
    use super::*;

    #[test]
    fn dollar_dollar_eq_dollar_dollar() {
        assert_parity(r#"[[ $$ == $$ ]]; echo $?"#);
    }

    #[test]
    fn dollar_dollar_single_eq() {
        assert_parity(r#"[[ $$ = $$ ]]; echo $?"#);
    }

    #[test]
    fn var_lhs_dollar_dollar_rhs() {
        assert_parity(r#"x=$$; [[ $x == $$ ]]; echo $?"#);
    }

    #[test]
    fn dq_lhs_dollar_dollar_rhs() {
        assert_parity(r#"[[ "$$" == $$ ]]; echo $?"#);
    }

    #[test]
    fn literal_dollars_do_not_match_pid() {
        assert_parity(r#"[[ "\$\$" == $$ ]]; echo $?"#);
    }

    #[test]
    fn nonmatching_value_stays_false() {
        assert_parity(r#"[[ 5 == $$ ]]; echo $?"#);
    }

    #[test]
    fn status_special_rhs() {
        assert_parity(r#"[[ $? == $? ]]; echo $?"#);
    }

    #[test]
    fn argc_special_rhs() {
        assert_parity(r#"[[ $# == $# ]]; echo $?"#);
    }

    #[test]
    fn opts_special_rhs() {
        assert_parity(r#"[[ $- == $- ]]; echo $?"#);
    }

    #[test]
    fn splat_special_rhs() {
        assert_parity(r#"set -- a b; [[ "$*" == $* ]]; echo $?"#);
    }

    #[test]
    fn case_pattern_dollar_dollar() {
        assert_parity(r#"case $$ in $$) echo M;; *) echo N;; esac"#);
    }

    #[test]
    fn case_pattern_argc_special() {
        assert_parity(r#"case $# in $#) echo M;; *) echo N;; esac"#);
    }
}

/// `-N file` — true iff modified since last read (atime <= mtime). zsh compares
/// at full nanosecond precision; zshrs once compared seconds only, so files
/// sharing a second but differing in nsec (e.g. /dev/null) diverged.
mod file_modified_since_read {
    use super::*;

    /// A freshly-written file has atime <= mtime → `-N` true.
    #[test]
    fn n_freshly_written_file() {
        assert_parity(
            r#"f=$(mktemp); print x >| $f; [[ -N $f ]]; print -r "nzf=$?"; command rm -f $f"#,
        );
    }

    /// Nonexistent file → `-N` false.
    #[test]
    fn n_nonexistent_file() {
        assert_parity(r#"[[ -N /nonexistent_zzz_qqq ]]; print -r "nzf=$?""#);
    }
}

/// `[[ ... ]]` syntax errors must abort the `-c` list (exit nonzero, no
/// trailing command runs), not silently evaluate false. par_cond_primary once
/// built `Binary(s1, op, s2)` for any middle word, so `[[ a b c ]]` ran on.
mod syntax_errors {
    use super::*;

    /// Unrecognized binary operator → "condition expected: b", abort.
    #[test]
    fn unknown_binary_operator_aborts() {
        assert_parity(r#"[[ a b c ]]; echo after"#);
    }

    /// Valid string/numeric/regex operators must still parse and run.
    #[test]
    fn valid_operators_still_work() {
        assert_parity(r#"[[ a == a ]]; print r$?; [[ 1 -eq 1 ]]; print r$?; [[ foo =~ f.. ]]; print r$?"#);
    }

    /// `test`/`[` with an unrecognized `-X` operator → "unknown condition"
    /// (cond.c:150-188), return 2. (test/`[` is non-fatal so the list
    /// continues; only `[[ ]]` aborts.)
    #[test]
    fn test_unknown_dash_condition() {
        assert_parity(r#"test a -xyz b; print -r "r=$?""#);
    }

    #[test]
    fn bracket_unknown_dash_condition() {
        assert_parity(r#"[ a -pcre-match b ]; print -r "r=$?""#);
    }
}

/// A `[[ ]]` command's status participates in errexit like any command:
/// `set -e; [[ 0 = 1 ]]` exits on the false status. `if`/`&&`/`||`/`while`
/// contexts are exempt. compile_cond once never ran the errexit check.
mod errexit {
    use super::*;

    /// `set -e` + a false `[[ ]]` aborts the list.
    #[test]
    fn set_e_false_cond_aborts() {
        assert_parity(r#"set -e; [[ 0 = 1 ]]; echo after"#);
    }

    /// True cond does not abort.
    #[test]
    fn set_e_true_cond_continues() {
        assert_parity(r#"set -e; [[ 1 = 1 ]]; echo after"#);
    }

    /// `&&` context is exempt from errexit.
    #[test]
    fn set_e_false_cond_in_and_exempt() {
        assert_parity(r#"set -e; [[ 0 = 1 ]] && echo x; echo after"#);
    }

    /// `if` context is exempt.
    #[test]
    fn set_e_false_cond_in_if_exempt() {
        assert_parity(r#"set -e; if [[ 0 = 1 ]]; then echo t; fi; echo after"#);
    }

    /// Without `set -e`, a false cond does NOT abort.
    #[test]
    fn no_set_e_false_cond_continues() {
        assert_parity(r#"[[ 0 = 1 ]]; echo after"#);
    }
}
