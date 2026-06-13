//! Positional params and shift parity:
//! `$#`, `$*`, `$@`, `$N`, `${N}`, `$argv`, `shift`, `set --`.

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

mod set_dash_dash {
    use super::*;

    /// `set -- a b c` sets positionals.
    #[test]
    fn set_dash_dash_assigns() {
        assert_parity(r#"set -- a b c; echo "$1/$2/$3""#);
    }

    /// `set --` clears all positionals.
    #[test]
    fn set_dash_dash_empty_clears() {
        assert_parity(r#"set -- a b c; set --; echo "[$1] count=$#""#);
    }

    /// `set --` with mixed values.
    #[test]
    fn set_dash_dash_with_spaces() {
        assert_parity(r#"set -- "a b" "c d"; echo "$1|$2""#);
    }
}

mod count {
    use super::*;

    /// `$#` returns count.
    #[test]
    fn dollar_hash_count_three() {
        assert_parity(r#"set -- a b c; echo $#"#);
    }

    #[test]
    fn dollar_hash_count_zero() {
        assert_parity(r#"echo $#"#);
    }

    #[test]
    fn dollar_hash_count_one() {
        assert_parity(r#"set -- "single arg"; echo $#"#);
    }
}

mod numeric_positionals {
    use super::*;

    #[test]
    fn unset_positional_empty() {
        assert_parity(r#"set -- a; echo "[$2]""#);
    }

    /// `${10}` braces required for 2-digit positions.
    #[test]
    fn double_digit_positional() {
        assert_parity(r#"set -- 1 2 3 4 5 6 7 8 9 ten; echo "${10}""#);
    }

    /// Without braces, `$10` = `$1` + literal `0`.
    #[test]
    fn ten_unbraced_is_one_zero() {
        assert_parity(r#"set -- a b c; echo "$10""#);
    }
}

mod star_at_difference {
    use super::*;

    /// `$*` joined by first char of IFS (space by default).
    #[test]
    fn star_joined_unquoted() {
        assert_parity(r#"set -- a b c; echo $*"#);
    }

    /// `"$*"` joined as single string.
    #[test]
    fn star_joined_quoted() {
        assert_parity(r#"set -- a b c; printf '<%s>' "$*"; echo"#);
    }

    /// `"$@"` keeps args separate.
    #[test]
    fn at_separate_quoted() {
        assert_parity(r#"set -- a b c; printf '<%s>' "$@"; echo"#);
    }

    /// `$@` unquoted same as `$*` unquoted (subject to splitting).
    #[test]
    fn at_unquoted_same_as_star() {
        assert_parity(r#"set -- a b c; printf '<%s>' $@; echo"#);
    }

    /// `$*` with empty arg.
    #[test]
    fn star_with_empty_arg() {
        assert_parity(r#"set -- a "" c; printf '<%s>' "$*"; echo"#);
    }

    /// `$@` with empty arg keeps the empty position.
    #[test]
    fn at_with_empty_arg() {
        assert_parity(r#"set -- a "" c; printf '<%s>' "$@"; echo"#);
    }
}

mod custom_ifs {
    use super::*;

    /// IFS affects `$*` join.
    #[test]
    fn custom_ifs_joins_with_first_char() {
        assert_parity(r#"set -- a b c; IFS=:; echo "$*""#);
    }

    /// Empty IFS → `$*` concatenates.
    #[test]
    fn empty_ifs_concatenates() {
        assert_parity(r#"set -- a b c; IFS=""; echo "$*""#);
    }
}

mod shift_builtin {
    use super::*;

    /// `shift` by 1.
    #[test]
    fn shift_one() {
        assert_parity(r#"set -- a b c; shift; echo "$@""#);
    }

    /// `shift 2` shifts by 2.
    #[test]
    fn shift_two() {
        assert_parity(r#"set -- a b c d e; shift 2; echo "$@""#);
    }

    /// `shift 0` no-op.
    #[test]
    fn shift_zero() {
        assert_parity(r#"set -- a b c; shift 0; echo "$@""#);
    }

    /// `shift` past end → error / nothing shifted.
    #[test]
    fn shift_past_end() {
        assert_parity(r#"set -- a b; shift 5 2>/dev/null; echo "exit=$? count=$#""#);
    }
}

mod argv_alias {
    use super::*;

    /// zsh's `$argv` is alias for positional array.
    #[test]
    fn argv_alias_array() {
        assert_parity(r#"set -- a b c; echo "${argv[1]}/${argv[2]}/${argv[3]}""#);
    }

    /// `${#argv}` matches `$#`.
    #[test]
    fn argv_count_matches_dollar_hash() {
        assert_parity(r#"set -- a b c d; echo "${#argv} $#""#);
    }

    /// `${argv[@]}` = `"$@"`.
    #[test]
    fn argv_at_matches_dollar_at() {
        assert_parity(r#"set -- a b c; printf '<%s>' "${argv[@]}"; echo"#);
    }
}

mod range_subscript {
    use super::*;

    /// `${@:1:2}` first 2.
    #[test]
    fn at_with_zsh_range_subscript() {
        // zsh syntax: ${argv[1,2]} or ${@[1,2]}.
        assert_parity(r#"set -- a b c d e; echo "${@[1,2]}""#);
    }

    /// `$@[2,4]` middle slice.
    #[test]
    fn at_with_range_2_4() {
        assert_parity(r#"set -- a b c d e; echo "${@[2,4]}""#);
    }

    /// `$@[-2,-1]` last 2.
    #[test]
    fn at_with_negative_range() {
        assert_parity(r#"set -- a b c d e; echo "${@[-2,-1]}""#);
    }
}

mod in_function {
    use super::*;

    /// Function has its own positionals.
    #[test]
    fn function_has_own_positionals() {
        assert_parity(
            r#"
f() { echo "fn: $1/$2/$# outer: $myout"; }
myout=outer
set -- a b
f x y z
echo "after: $1/$2/$#"
"#,
        );
    }

    /// $0 inside function = function name (zsh behavior).
    #[test]
    fn dollar_zero_in_function() {
        assert_parity(
            r#"
greet() { echo $0; }
greet
"#,
        );
    }
}

mod compound_use {
    use super::*;

    /// Loop over `"$@"`.
    #[test]
    fn for_loop_over_at() {
        assert_parity(
            r#"
set -- alpha beta gamma
for x in "$@"; do echo "[$x]"; done
"#,
        );
    }

    /// Iterate by index.
    #[test]
    fn for_loop_with_index() {
        assert_parity(
            r#"
set -- a b c
i=1
while (( i <= $# )); do
  echo "$i:${(P)i}"
  (( i++ ))
done
"#,
        );
    }
}
