//! Loop parity tests — for/while/until/repeat with break, continue, nesting.

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

struct R {
    stdout: String,
    exit: i32,
}

fn run_zsh(s: &str) -> R {
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .output()
        .expect("zsh");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}

fn run_zshrs(s: &str) -> R {
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", s])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}

fn assert_parity(s: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_zsh(s);
    let r = run_zshrs(s);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on:\n{s}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        z.stdout, r.stdout
    );
    assert_eq!(z.exit, r.exit);
}

mod for_in {
    use super::*;

    #[test]
    fn for_in_word_list() {
        assert_parity("for x in a b c; do echo $x; done");
    }

    #[test]
    fn for_in_empty_list() {
        assert_parity("for x in; do echo $x; done; echo done");
    }

    #[test]
    fn for_in_one_word() {
        assert_parity("for x in only; do echo $x; done");
    }

    #[test]
    fn for_in_with_quoted_words() {
        assert_parity(r#"for x in "hi there" "go"; do echo $x; done"#);
    }

    #[test]
    fn for_in_brace_expansion() {
        assert_parity("for x in {1..5}; do echo $x; done");
    }

    #[test]
    fn for_in_array() {
        assert_parity("arr=(a b c); for x in $arr; do echo $x; done");
    }

    #[test]
    fn for_in_array_splat() {
        assert_parity(r#"arr=(a b c); for x in "${arr[@]}"; do echo $x; done"#);
    }

    /// `for x` (no `in`) iterates positional params.
    #[test]
    fn for_implicit_iterates_positional() {
        assert_parity("set -- alpha beta gamma; for x; do echo $x; done");
    }
}

mod c_style_for {
    use super::*;

    #[test]
    fn c_style_for_simple() {
        assert_parity("for ((i=0; i<3; i++)); do echo $i; done");
    }

    #[test]
    fn c_style_for_count_down() {
        assert_parity("for ((i=5; i>0; i--)); do echo $i; done");
    }

    #[test]
    fn c_style_for_step_two() {
        assert_parity("for ((i=0; i<10; i+=2)); do echo $i; done");
    }

    #[test]
    fn c_style_for_empty_body_loops() {
        assert_parity("for ((i=0; i<3; i++)); do :; done; echo done");
    }

    #[test]
    fn c_style_for_zero_iterations() {
        assert_parity("for ((i=10; i<5; i++)); do echo $i; done; echo done");
    }
}

mod while_loop {
    use super::*;

    #[test]
    fn while_decrement_counter() {
        assert_parity("i=3; while (( i > 0 )); do echo $i; (( i-- )); done");
    }

    #[test]
    fn while_false_zero_iters() {
        assert_parity("while false; do echo never; done; echo done");
    }

    /// `while read` pattern.
    #[test]
    fn while_read_with_heredoc() {
        assert_parity("while read line; do echo got: $line; done <<EOF\none\ntwo\nthree\nEOF");
    }
}

mod until_loop {
    use super::*;

    #[test]
    fn until_increment_counter() {
        assert_parity("i=0; until (( i >= 3 )); do echo $i; (( i++ )); done");
    }

    #[test]
    fn until_true_zero_iters() {
        assert_parity("until true; do echo never; done; echo done");
    }
}

mod repeat_loop {
    use super::*;

    #[test]
    fn repeat_three_times() {
        assert_parity("repeat 3 do echo hello; done");
    }

    #[test]
    fn repeat_zero_times() {
        assert_parity("repeat 0 do echo nope; done; echo done");
    }

    #[test]
    fn repeat_with_arithmetic_count() {
        assert_parity("repeat $((2+3)) do echo x; done");
    }
}

mod break_continue {
    use super::*;

    #[test]
    fn break_exits_loop() {
        assert_parity("for i in 1 2 3 4 5; do (( i == 3 )) && break; echo $i; done");
    }

    #[test]
    fn continue_skips_iteration() {
        assert_parity("for i in 1 2 3 4 5; do (( i == 3 )) && continue; echo $i; done");
    }

    #[test]
    fn break_two_exits_two_levels() {
        assert_parity(
            r#"
for i in 1 2 3; do
  for j in a b c; do
    [[ $j == b ]] && break 2
    echo "$i$j"
  done
done
echo done
"#,
        );
    }

    #[test]
    fn continue_two_skips_outer() {
        assert_parity(
            r#"
for i in 1 2 3; do
  for j in a b c; do
    [[ $j == b ]] && continue 2
    echo "$i$j"
  done
done
"#,
        );
    }
}

mod nested {
    use super::*;

    #[test]
    fn double_nested_for() {
        assert_parity(
            r#"
for i in 1 2; do
  for j in a b; do
    echo "$i$j"
  done
done
"#,
        );
    }

    #[test]
    fn for_inside_while() {
        assert_parity(
            r#"
n=2
while (( n > 0 )); do
  for x in a b; do echo "$n-$x"; done
  (( n-- ))
done
"#,
        );
    }

    #[test]
    fn while_inside_for() {
        assert_parity(
            r#"
for outer in 1 2; do
  i=0
  while (( i < 2 )); do
    echo "$outer-$i"
    (( i++ ))
  done
done
"#,
        );
    }
}

mod loop_exit_status {
    use super::*;

    /// Loop exit status = exit status of last command in last iteration.
    #[test]
    fn for_loop_exit_status_from_last_iter() {
        assert_parity("for i in 1 2 3; do (( i % 2 == 1 )); done; echo $?");
    }

    /// Zero iterations → exit 0.
    #[test]
    fn while_loop_no_iterations_exit_zero() {
        assert_parity("while false; do :; done; echo $?");
    }

    /// `break` in loop → exit 0.
    #[test]
    fn break_yields_zero_exit() {
        assert_parity("for i in 1 2; do break; done; echo $?");
    }
}

mod round_pins {
    use super::*;

    #[test]
    fn c_style_for_numeric() {
        assert_parity("for ((i=1;i<=3;i++)); do print -r $i; done");
    }

    #[test]
    fn until_once() {
        assert_parity("i=0; until (( i >= 1 )); do print -r $i; (( i++ )); done");
    }

    #[test]
    fn continue_skips_iteration() {
        assert_parity("for i in 1 2 3; do (( i == 2 )) && continue; print -r $i; done");
    }
}

mod fold_in_pipeline {
    use super::*;

    #[test]
    fn for_loop_piped_to_grep() {
        assert_parity("for i in alpha beta gamma; do echo $i; done | grep e");
    }

    #[test]
    fn while_loop_in_pipeline_to_sort() {
        assert_parity("for i in 3 1 4 1 5 9; do echo $i; done | sort -n | head -3");
    }
}

/// The `for` loop carries the word-list expansion status (or, for a
/// literal list, the previous command's $?) into the FIRST body
/// iteration; only an EMPTY list resets $? to 0 (c:Src/loop.c execfor).
mod for_loop_status_carry {
    use super::*;

    #[test]
    fn previous_status_into_first_iter() {
        assert_parity("(exit 2); for x in 1 2; do print $?; done; echo end=$?");
    }

    #[test]
    fn cmdsubst_status_into_first_iter() {
        assert_parity("false; for x in $(echo 1 2; (exit 3)); do print $?; done");
    }

    #[test]
    fn last_body_status_kept_with_cmdsubst_list() {
        assert_parity("false; for x in $(echo 1; false); do echo $?; (exit 4); done; echo exit=$?");
    }

    #[test]
    fn empty_body_resets_to_zero() {
        assert_parity("false; for x in $(echo 1; false); do done; echo $?");
    }

    #[test]
    fn empty_cmdsubst_list_resets_to_zero() {
        assert_parity("false; for x in $(exit 4); do print no; done; echo $?");
    }

    #[test]
    fn empty_glob_list_resets_to_zero() {
        assert_parity("false; for x in NoSuch*(N); do print no; done; echo $?");
    }

    #[test]
    fn literal_list_first_iter_carries() {
        assert_parity("(exit 7); for x in a b c; do echo $?; done");
    }

    #[test]
    fn positional_carries_status() {
        assert_parity("set -- a b; (exit 5); for x; do echo $?; (exit 6); done");
    }
}

/// `Src/loop.c:141-145` + `:199-203` (execfor), `:478-481` (execwhile),
/// `:534-537` (execrepeat) — a loop that abandons its body because `errflag`
/// is set forces the escaping status:
/// `if (errflag) { if (breaks) breaks--; lastval = 1; break; }`.
/// So a fatal error inside a loop leaves 1, while the same command outside a
/// loop leaves its own status (a bad `[[ ]]` pattern is 2).
mod errflag_abort_status {
    use super::*;

    #[test]
    fn bad_pattern_in_loop_body_exits_one() {
        assert_parity("setopt extendedglob; for i in 1 2; do [[ abc == [ ]]; done; print never");
        assert_parity("setopt extendedglob; while true; do [[ abc == [ ]]; done; print never");
        assert_parity("setopt extendedglob; until false; do [[ abc == [ ]]; done; print never");
        assert_parity("setopt extendedglob; repeat 2; do [[ abc == [ ]]; done; print never");
        assert_parity(
            "setopt extendedglob; for (( i=0; i<2; i++ )); do [[ abc == [ ]]; done; print never",
        );
    }

    /// The same fatal error OUTSIDE any loop keeps the command's own status.
    #[test]
    fn bad_pattern_outside_a_loop_keeps_its_own_status() {
        assert_parity("setopt extendedglob; [[ abc == [ ]]");
        assert_parity("setopt extendedglob; { [[ abc == [ ]] } 2>/dev/null; print rc=$?");
    }

    /// The loop still aborts on the first offending iteration, and the
    /// function it sits in propagates the forced 1.
    #[test]
    fn loop_aborts_at_the_error_and_propagates_through_a_function() {
        assert_parity(
            "setopt extendedglob; for i in 1 2; do print A; [[ abc == [ ]]; print B; done; print never",
        );
        assert_parity(
            "setopt extendedglob; f(){ for i in 1 2; do [[ abc == [ ]]; done; }; f; print rc=$?",
        );
    }

    /// Other errflag sources take the same path.
    #[test]
    fn arithmetic_and_readonly_errors_in_a_loop_exit_one() {
        assert_parity("for i in 1 2; do : $((1/0)); done; print never");
        assert_parity("while true; do : $((1/0)); done; print never");
        assert_parity("for i in 1 2; do typeset -r RO=1; RO=2; done; print never");
    }

    /// An ERREXIT abort is NOT an errflag abort: C leaves execlist through
    /// `zexit(lastval)` and never reaches the loop's `if (errflag)` arm, so
    /// the failing command's own status must survive.
    #[test]
    fn errexit_abort_keeps_the_failing_commands_status() {
        assert_parity("setopt errexit; for i in 1 2; do (exit 7); done; print never");
        assert_parity("setopt errexit; while true; do false; done; print never");
        assert_parity("setopt errexit; repeat 2; do false; done; print never");
        assert_parity("setopt errexit; f(){ for i in 1 2; do false; done; }; f; print never");
    }

    /// Ordinary loop exits are untouched.
    #[test]
    fn normal_loop_exits_are_unaffected() {
        assert_parity("for i in 1 2; do false; done; print rc=$?");
        assert_parity("for i in 1 2; do (exit 7); done; print rc=$?");
        assert_parity("f(){ for i in 1 2; do return 5; done; }; f; print rc=$?");
        assert_parity("for i in 1 2 3; do [[ $i == 2 ]] && break; done; print rc=$?");
        assert_parity("for i in 1 2; do exit 9; done; print never");
    }
}

/// c:Src/loop.c:95-100 (execfor) and c:250-255 (execselect) — `execsubst(args);
/// if (errflag) { state->pc = end; …; return 1; }`. A word list that fails to
/// expand ends the loop with status 1 before the empty-list `lastval = 0`.
/// zshrs reached that reset (the failed glob left an empty list) and exited 0.
mod word_list_expansion_error {
    use super::*;

    #[test]
    fn a_nomatch_in_the_word_list_exits_one() {
        assert_parity("for w in zshrs_nomatch_q*; do :; done; print never");
        assert_parity("print hi; for w in zshrs_nomatch_q*; do :; done");
        assert_parity("for w in a zshrs_nomatch_q*; do print $w; done");
        assert_parity("for w in zshrs_nomatch_q*; print $w");
        assert_parity("foreach w (zshrs_nomatch_q*) print $w; end");
        assert_parity("select w in zshrs_nomatch_q*; do :; done");
        assert_parity("f(){ for w in zshrs_nomatch_q*; do :; done; }; f; print never");
    }

    /// An empty list still resets the status to 0.
    #[test]
    fn an_empty_list_still_exits_zero() {
        assert_parity("false; for w in; do :; done; print rc=$?");
        assert_parity("false; for w in $(true); do :; done; print rc=$?");
        assert_parity("false; select w in; do :; done; print rc=$?");
        assert_parity("for w in a b; do print $w; done; print rc=$?");
    }
}

/// c:Src/parse.c:1175 / :1537 / :1586 (and :1182 / :1544 for the brace form)
/// — `incmdpos = 0; zshlex();` after the loop closer. Under IGNORE_BRACES a
/// bare `}` is a reserved word only in command position, so `{ for …; done }`
/// is a parse error in zsh. zshrs lexed past `done` in command position.
mod closing_brace_after_done_under_ignorebraces {
    use super::*;

    #[test]
    fn a_brace_right_after_done_is_a_parse_error() {
        assert_parity("setopt ignorebraces; eval '{ for i in 1; do print $i; done }' 2>/dev/null; print rc=$?");
        assert_parity("setopt ignorebraces; eval '{ while false; do :; done }' 2>/dev/null; print rc=$?");
        assert_parity("setopt ignorebraces; eval '{ select x in; do :; done }' 2>/dev/null; print rc=$?");
    }

    #[test]
    fn other_closers_and_the_default_options_are_unchanged() {
        assert_parity("setopt ignorebraces; eval '{ if true; then print t; fi }'; print rc=$?");
        assert_parity("setopt ignorebraces; eval '{ repeat 1 print r }'; print rc=$?");
        assert_parity("{ for i in 1; do print $i; done }; for i in 1 2; do print $i; done | cat");
        assert_parity("for i in 1; { print brace $i }; print next");
    }
}
