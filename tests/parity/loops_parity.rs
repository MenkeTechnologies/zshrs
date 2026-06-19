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
