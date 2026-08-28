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
        assert_parity(
            r#"[[ a == a ]]; print r$?; [[ 1 -eq 1 ]]; print r$?; [[ foo =~ f.. ]]; print r$?"#,
        );
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

    /// `[[ -X a ]]` (unknown unary condition) → "unknown condition" + abort.
    #[test]
    fn bracket_unknown_unary_aborts() {
        assert_parity(r#"[[ -X a ]]; echo after"#);
    }

    /// `[[ a -xyz b ]]` (unknown binary condition) → "unknown condition" + abort.
    #[test]
    fn bracket_unknown_binary_aborts() {
        assert_parity(r#"[[ a -xyz b ]]; echo after"#);
    }
}

mod escaped_dollar_in_cond_pattern {
    //! An escaped `\$` in a `[[ = ]]` pattern is a LITERAL dollar. The
    //! compile-time pattern splitter treated the `$` after a
    //! Bnull/backslash escape as a substitution start: `[[ $v = \$* ]]`
    //! substituted `$*` (positionals), and f-sy-h's dollar matcher
    //! `\$[{]` compiled `$[…]` old-style MATH on the class body —
    //! "bad math expression: illegal character: 0x8f" on every
    //! keystroke, after which `-fast-highlight-string`'s while loop
    //! never advanced and the shell spun at 100% CPU (the
    //! sample-reported infinite loop).
    use super::*;

    #[test]
    fn escaped_dollar_star_matches_literal_dollar() {
        assert_parity(r#"v='$x'; [[ $v = \$* ]] && print yes || print no"#);
    }

    #[test]
    fn escaped_dollar_class_matches() {
        assert_parity(r#"v='$a'; [[ $v = \$[a-z]* ]] && print yes || print no"#);
    }

    #[test]
    fn escaped_dollar_brace_class_fsh_shape() {
        assert_parity(r#"v='${x}'; [[ $v = \$[{]* ]] && print yes || print no"#);
    }

    /// The full f-sy-h `-fast-highlight-string` dollar matcher.
    #[test]
    fn fsh_string_highlight_pattern_matches() {
        assert_parity(
            r#"setopt extendedglob
local _mybuf='say $there ok'
if [[ $_mybuf = (#b)[^\$\\]#((\$(#B)([#+^=~](#c1,2))(#c0,1)(#B)([a-zA-Z_:][a-zA-Z0-9_:]#|[0-9]##)(#b)(\[[^\]]#\])(#c0,1))|(\$[{](#B)([#+^=~](#c1,2))(#c0,1)(#b)(\([a-zA-Z0-9_:@%#]##\))(#c0,1)[a-zA-Z0-9_:#]##(\[[^\]]#\])(#c0,1)[}])|\$|[\\][\'\"\$]|[\\](*))(*) ]]; then
  print -r -- "mb1=$mbegin[1] m1=$match[1]"
else
  print no-match
fi"#,
        );
    }

    /// Active substitutions in patterns still substitute (guard against
    /// over-escaping): `$H*` with H=foo must match foo-prefixed strings.
    #[test]
    fn active_dollar_in_pattern_still_substitutes() {
        assert_parity(r#"H=foo; [[ foobar = $H* ]] && print yes || print no"#);
    }
}

mod star_literal_star_fast_path {
    //! pattryrefs takes a substring fast path for `*literal*` (and
    //! `(#i)*literal*` when pattern+subject are ASCII) — the shape
    //! history-search-multi-word scans 566k history entries with.
    //! These pin the fast path to the full matcher's semantics.
    use super::*;

    #[test]
    fn star_lit_star_basic() {
        assert_parity(
            r#"v="abc needle xyz"; [[ $v = *needle* ]] && print y || print n; [[ $v = *missing* ]] && print y || print n"#,
        );
    }

    #[test]
    fn igncase_ascii() {
        assert_parity(
            r#"setopt extendedglob; v="abc NeEdLe xyz"; [[ $v = (#i)*needle* ]] && print y || print n; [[ $v = (#i)*NEEDLE* ]] && print y || print n; [[ $v = (#i)*nope* ]] && print y || print n"#,
        );
    }

    #[test]
    fn igncase_nonascii_falls_back() {
        assert_parity(
            r#"setopt extendedglob; v="pré Über post"; [[ $v = (#i)*über* ]] && print y || print n; [[ $v = (#i)*PRÉ* ]] && print y || print n"#,
        );
    }

    #[test]
    fn edge_positions_and_empty() {
        assert_parity(
            r#"v="needle"; [[ $v = *needle* ]] && print y || print n; [[ "" = ** ]] && print y || print n; [[ needle2 = *needle* ]] && print y || print n; [[ 2needle = *needle* ]] && print y || print n"#,
        );
    }

    /// Shapes that must NOT take the fast path still match correctly.
    #[test]
    fn non_fastpath_shapes_unaffected() {
        assert_parity(
            r#"setopt extendedglob; v="a needle b"; [[ $v = *need(le|il)* ]] && print y || print n; [[ $v = *needle*b ]] && print y || print n; [[ $v = (#b)*(needle)* ]] && print m=$match[1] || print n"#,
        );
    }

    /// The hsmw scan shape end-to-end on $history.
    #[test]
    fn history_r_subscript_scan() {
        assert_parity(
            r#"setopt extendedglob
fc -p /dev/null 100 100 2>/dev/null
print -s "echo alpha one"; print -s "echo beta two"; print -s "echo gamma three"
print -r -- "${history[(R)(#i)*BETA*]}""#,
        );
    }
}

/// A backslash in a match pattern means one of two OPPOSITE things depending
/// on where it came from, and the pattern compiler has to keep them apart:
///
///   * SOURCE-level quoting — `[[ "man ls" = man\ * ]]`. The lexer quotes the
///     space; the pattern is `man ` followed by an active `*`.
///   * DATA — `p='a\ b'; [[ x == ${~p} ]]`. The backslash is an ordinary
///     character of the value and stays in the pattern as itself.
///
/// c:Src/glob.c:3633-3643 `zshtokenize` draws the line: a raw backslash is
/// rewritten to a quote marker ONLY when the next character is a glob
/// metacharacter (the `for (t = ztokens; *t; t++)` scan); before anything else
/// no arm fires and both bytes survive as literals.
///
/// zshrs collapsed both spellings into `\X` and honored the escape
/// unconditionally, so every data-provenance backslash matched one character
/// too few. zpwr's `_files` override is the real-world casualty: its dedup
/// guard `(( $tried[(I)${(q)tmp}] ))` uses a `${(q)}`-quoted needle, so any
/// value containing a space matched a pattern group it should not have and
/// that group was `continue`d past on every file completion. BUGS.md #1090.
mod backslash_provenance_in_patterns {
    use super::*;

    /// Fixture: element 1 holds a plain space, element 2 holds a REAL
    /// backslash followed by a space.
    const FIX: &str = "a=('a b'); b=('a\\ b'); p='a\\ b'\n";

    fn assert_fix(body: &str) {
        assert_parity(&format!("{FIX}{body}"));
    }

    // ── data provenance: the backslash is a character of the value ──

    #[test]
    fn subscript_I_data_backslash_does_not_match_plain_space() {
        assert_fix(r#"print -r -- "A=${a[(I)$p]}""#);
    }

    #[test]
    fn subscript_I_data_backslash_matches_literal_backslash() {
        assert_fix(r#"print -r -- "B=${b[(I)$p]}""#);
    }

    #[test]
    fn subscript_r_data_backslash_returns_the_backslash_element() {
        assert_fix(r#"print -r -- "R=${b[(r)$p]}""#);
    }

    #[test]
    fn globsubst_cond_data_backslash_does_not_match_plain_space() {
        assert_fix(r#"[[ 'a b' == ${~p} ]] && print T1match || print T1no"#);
    }

    #[test]
    fn globsubst_cond_data_backslash_matches_literal_backslash() {
        assert_fix(r#"[[ 'a\ b' == ${~p} ]] && print T2match || print T2no"#);
    }

    #[test]
    fn globsubst_case_data_backslash() {
        assert_fix(
            r#"case 'a b'  in ${~p}) print C1match;; *) print C1no;; esac
               case 'a\ b' in ${~p}) print C2match;; *) print C2no;; esac"#,
        );
    }

    /// The `${(q)}`-quoted-needle shape `_files` actually uses.
    #[test]
    fn quoted_needle_subscript_search() {
        assert_parity(
            r#"tried=('x' 'a b'); tmp='a b'
               print -r -- "hit=${tried[(I)${(q)tmp}]}""#,
        );
    }

    // ── source provenance: the backslash is shell quoting ──

    #[test]
    fn source_quoted_space_still_matches_with_active_star() {
        assert_parity(
            r#"for b in "man ls" "git log" "manatee x"; do
                 if [[ "$b" = man\ * ]]; then print "$b -> already-man"; else print "$b -> wrap"; fi
               done"#,
        );
    }

    #[test]
    fn source_quoted_space_in_case_arm() {
        assert_parity(r#"case 'man ls' in man\ *) print yes;; *) print no;; esac"#);
    }

    // ── escapes before real metacharacters are honored on BOTH paths ──

    #[test]
    fn escaped_star_is_literal_from_source_and_from_data() {
        assert_parity(
            r#"d=('a*b'); q='a\*b'
               print -r -- "src=${d[(I)a\*b]} data=${d[(I)$q]}"
               [[ 'a*b' == ${~q} ]] && print Qmatch || print Qno
               [[ 'azb' == ${~q} ]] && print Zmatch || print Zno"#,
        );
    }

    #[test]
    fn escaped_dollar_is_literal() {
        assert_parity(r#"c=('a$b'); print -r -- "d=${c[(I)a\$b]}""#);
    }

    /// A trailing lone backslash must not be swallowed.
    #[test]
    fn trailing_lone_backslash_in_pattern() {
        assert_parity(
            r#"e=('ab\' 'ab'); f='ab\'
               print -r -- "t=${e[(I)$f]}""#,
        );
    }
}

/// The other half of the BUGS.md #1090 split: a backslash that IS shell
/// quoting must keep working everywhere it already did. These are the exact
/// shapes the first (reverted) attempt at #1090 regressed — `${branch//\%/%%}`
/// stopped substituting, `${local_dir//\//--}` stopped splitting and
/// `${entry%\%*}` stopped stripping — plus the subscript case, where zsh
/// applies the DATA rule even to source text because a subscript pattern
/// reaches `patcompile` through `zshtokenize` alone (c:Src/params.c:1727).
mod backslash_source_quoting_still_works {
    use super::*;

    /// f-sy-h / p10k percent-doubling: `\%` in a `${//}` pattern is a quoted
    /// `%`, so it matches a bare `%`.
    #[test]
    fn replace_escaped_percent_doubles_it() {
        assert_parity(
            r#"for branch in 'feat%50' 'plain' '%%lead' 'a%b%c'; do
                 print -r -- "[$branch] -> [${branch//\%/%%}]"
               done"#,
        );
    }

    /// zinit path encoding: `\/` in a `${//}` pattern is a quoted separator,
    /// not the pattern/replacement delimiter.
    #[test]
    fn replace_escaped_slash_is_the_pattern_not_the_delimiter() {
        assert_parity(
            r#"for local_dir in 'a/b/c' '/lead' 'trail/' 'none'; do
                 print -r -- "[$local_dir] -> [${local_dir//\//--}]"
               done"#,
        );
    }

    /// fast-syntax-highlighting chroma split: `%` anchored strip with a
    /// source-escaped `%` in the pattern.
    #[test]
    fn suffix_strip_with_escaped_percent() {
        assert_parity(
            r#"for entry in '/main.ch%git' '/-grep.ch' 'x%y%z' ''; do
                 print -r -- "file=[${entry%\%*}] arg=[${(M)entry%\%*}]"
               done"#,
        );
    }

    /// A subscript pattern is tokenized as a VALUE even when it was typed in
    /// the source, so `\ ` there is a literal backslash + space — real zsh
    /// answers 2, the element that actually holds a backslash.
    #[test]
    fn subscript_source_escaped_space_is_data_not_a_quote() {
        assert_parity(
            r#"z=('a b' 'a\ b')
               print -r -- "space=${z[(I)a\ b]}"
               print -r -- "dollar=${z[(I)a\$b]}""#,
        );
    }
}

/// The full `zshtokenize` escape matrix for a DATA pattern (BUGS.md #1090).
/// `\X` quotes X only when X reaches the `ztokens` scan — the `switch` labels
/// at c:Src/glob.c:3596 / 3613-3615 / 3619-3631 plus the `\` case at c:3589.
/// A character that is in the `ztokens` TABLE but has no switch label (`$`,
/// `{`, `}`, backtick, `,`, `'`, `"`) keeps both bytes as literal data.
mod zshtokenize_escape_matrix_for_data_patterns {
    use super::*;

    /// Every glob metacharacter: `\X` from a value is a literal X.
    #[test]
    fn escape_before_a_metachar_quotes_it() {
        assert_parity(
            r#"setopt extendedglob
arr=( 'a*b' 'a-b' 'a!b' 'a=b' 'a~b' 'a<b' 'a[b' 'a#b' 'a^b' 'a?b' 'a(b' 'a|b' 'a\b' )
for q in 'a\*b' 'a\-b' 'a\!b' 'a\=b' 'a\~b' 'a\<b' 'a\[b' 'a\#b' 'a\^b' 'a\?b' 'a\(b' 'a\|b' 'a\\b'; do
  print -r -- "q=[$q] I=${arr[(I)$q]} r=[${arr[(r)$q]}]"
done"#,
        );
    }

    /// The `ztokens`-table members with no switch label: the backslash is
    /// ordinary data and matches itself.
    #[test]
    fn escape_before_a_non_metachar_stays_literal() {
        assert_parity(
            r#"arr=( 'a$b' 'a\$b' 'a{b' 'a\{b' 'a,b' 'a\,b' 'a`b' 'a\`b' )
for q in 'a\$b' 'a\{b' 'a\,b' 'a\`b'; do
  print -r -- "q=[$q] I=${arr[(I)$q]}"
done"#,
        );
    }
}
