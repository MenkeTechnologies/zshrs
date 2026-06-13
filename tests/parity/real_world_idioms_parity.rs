//! Real-world zsh idioms — replications of patterns from
//! oh-my-zsh, zinit, prezto, p10k, zsh-syntax-highlighting and
//! similar daily-driver frameworks. Each test is a standalone
//! mini-script that exercises a multi-feature interaction.

#![allow(non_snake_case)]

use std::path::PathBuf;
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("target").join("debug").join("zshrs")
}
fn zsh_path() -> &'static str {
    use std::path::Path;
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
    #[allow(dead_code)]
    stderr: String,
    exit: i32,
}
fn run_zsh(s: &str) -> ShellResult {
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .output()
        .expect("zsh");
    ShellResult {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
fn run_zshrs(s: &str) -> ShellResult {
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-c", s])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    ShellResult {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
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
        "stdout divergence on:\n{}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        s, z.stdout, r.stdout
    );
    assert_eq!(z.exit, r.exit, "exit divergence on:\n{}", s);
}

// ───────────────────────── zinit-style version checks ────────────────

mod version_checks {
    use super::*;

    /// zinit's autodetect of zsh version + path.
    #[test]
    fn zsh_path_test() {
        assert_parity(r#"[[ -x ${ZSH_NAME:-zsh} ]] || true; echo "name=$ZSH_NAME""#);
    }

    /// `${ZSH_VERSION:0:3}` major.minor extraction.
    #[test]
    fn zsh_version_substr() {
        assert_parity(r#"v="${ZSH_VERSION:0:3}"; [[ -n "$v" ]] && echo ok"#);
    }
}

// ───────────────────────── zinit init sketch ────────────────

mod zinit_init {
    use super::*;

    /// Typical zinit-style assoc init.
    #[test]
    fn zinit_top_level_assoc() {
        assert_parity(
            r#"typeset -gAH ZINIT
ZINIT[BIN_DIR]=/tmp/zinit
ZINIT[HOME_DIR]=/tmp/zinit-home
ZINIT[PLUGINS_DIR]=/tmp/zinit/plugins
print -- "${ZINIT[BIN_DIR]}"
print -- "${ZINIT[PLUGINS_DIR]}""#,
        );
    }

    /// Zinit-style "is plugin loaded" check via assoc lookup.
    #[test]
    fn assoc_existence_check() {
        assert_parity(
            r#"typeset -gA ZINIT_REGISTERED_PLUGINS
ZINIT_REGISTERED_PLUGINS[user/plugin]=1
[[ -n "${ZINIT_REGISTERED_PLUGINS[user/plugin]}" ]] && echo loaded
[[ -n "${ZINIT_REGISTERED_PLUGINS[other/plugin]}" ]] || echo missing"#,
        );
    }

    /// Loop over an assoc's keys.
    #[test]
    fn assoc_iterate_keys() {
        assert_parity(
            r#"typeset -A m=(z 26 a 1 m 13)
for k in ${(k)m}; do
    print -- "$k"
done | sort"#,
        );
    }
}

// ───────────────────────── p10k segment building ────────────────

mod p10k_segments {
    use super::*;

    /// Building a prompt segment with tests + concatenation.
    #[test]
    fn build_segment_via_concat() {
        assert_parity(
            r#"local segment=""
local sep="|"
local i
for i in user host pwd; do
    [[ -n "$segment" ]] && segment+="$sep"
    segment+="[$i]"
done
print -- "$segment""#,
        );
    }

    /// Conditional segment rendering.
    #[test]
    fn conditional_segment() {
        assert_parity(
            r#"local user_color="$(printf '\e[34m')" reset="$(printf '\e[0m')"
print -- "${user_color}USER${reset}" | wc -c | tr -d ' '"#,
        );
    }
}

// ───────────────────────── bash compat / autoload ────────────────

mod compat {
    use super::*;

    /// `autoload -Uz` no-op when the function is already defined.
    #[test]
    fn autoload_uz_existing() {
        assert_parity(
            r#"foo() { echo bar; }
autoload -Uz foo 2>/dev/null
foo"#,
        );
    }

    /// `${var/PAT/REPL}` global with literal regex.
    #[test]
    fn replace_with_chars() {
        assert_parity(r#"x=hello.world.txt; print -- "${x//./_}""#);
    }

    /// `${var:gs/from/to/}` (global :s/) on positional.
    #[test]
    fn pos_gs_modifier() {
        assert_parity(r#"set -- "abc abc abc"; print -- "${1:gs/abc/X/}""#);
    }
}

// ───────────────────────── PATH manipulation ────────────────

mod path_ops {
    use super::*;

    /// Add to $path with uniqueness.
    #[test]
    fn typeset_aU_path_dedup() {
        assert_parity(
            r#"typeset -aU path
path=(/a /b /a /c /b)
print -l -- "${path[@]}""#,
        );
    }

    /// Tied scalar/array PATH semantics.
    #[test]
    fn typeset_T_path_array_tied() {
        // `typeset -T PATH path :` ties them. zsh syncs both directions.
        // Just verify the join produces the same result as $PATH.
        assert_parity(
            r#"typeset -T MYPATH mypath_arr :
MYPATH=/a:/b:/c
print -- "${mypath_arr[@]}""#,
        );
    }
}

// ───────────────────────── fpath / autoload ────────────────

mod fpath {
    use super::*;

    /// $fpath array exists and can be appended.
    #[test]
    fn fpath_extend() {
        assert_parity(
            r#"original_count="${#fpath[@]}"
fpath+=(/tmp/zfunc)
new_count="${#fpath[@]}"
echo "$((new_count - original_count))""#,
        );
    }

    /// `autoload` declares functions for lazy load.
    #[test]
    fn autoload_smoke() {
        assert_parity(
            r#"autoload -Uz nonexistent_fn_zxqv 2>/dev/null
echo done"#,
        );
    }
}

// ───────────────────────── conditionals + arith combos ────────────

mod logic_combos {
    use super::*;

    /// `&&` and `||` short-circuit.
    #[test]
    fn and_or_short_circuit() {
        assert_parity(
            r#"true && echo yes && echo also
false && echo nope || echo not"#,
        );
    }

    /// `[[ ]]` with `&&` outside braces.
    #[test]
    fn double_bracket_chain() {
        assert_parity(
            r#"x=10
[[ $x -gt 5 ]] && [[ $x -lt 20 ]] && echo "in range""#,
        );
    }

    /// Conditional with grouped tests.
    #[test]
    fn cond_grouped() {
        assert_parity(
            r#"x=hello
[[ -n "$x" && ( "$x" == "hello" || "$x" == "world" ) ]] && echo match"#,
        );
    }

    /// Arithmetic in conditional.
    #[test]
    fn arith_in_test() {
        assert_parity(r#"x=5; [[ $((x*2)) -eq 10 ]] && echo yes"#);
    }
}

// ───────────────────────── common patterns ────────────────

mod common_patterns {
    use super::*;

    /// Parameter expansion with default + colon.
    #[test]
    fn default_colon_dash() {
        assert_parity(
            r#"unset x; echo "${x:-default}"
x=""; echo "${x:-default}"
x="set"; echo "${x:-default}""#,
        );
    }

    /// Parameter expansion with default + no colon.
    #[test]
    fn default_dash() {
        assert_parity(
            r#"unset x; echo "${x-default}"
x=""; echo "${x-default}"
x="set"; echo "${x-default}""#,
        );
    }

    /// `${var:?msg}` error if unset/empty.
    #[test]
    fn error_if_unset() {
        let z = run_zsh(r#"x=hello; echo "${x:?should not error}""#);
        let r = run_zshrs(r#"x=hello; echo "${x:?should not error}""#);
        assert_eq!(z.stdout, r.stdout);
    }

    /// `${var:+alt}` only-if-set.
    #[test]
    fn only_if_set() {
        assert_parity(
            r#"unset x; echo "[${x:+set}]"
x=hello; echo "[${x:+set}]""#,
        );
    }

    /// `${var:=default}` assign-if-unset.
    #[test]
    fn assign_if_unset() {
        assert_parity(r#"unset x; echo "${x:=default}"; echo "$x""#);
    }
}

// ───────────────────────── error handling ────────────────

mod err_handling {
    use super::*;

    /// `set -e` / errexit at top level — both shells continue after
    /// `false` when errexit is OFF. The subshell-propagation form is
    /// a known divergence (zsh's errexit through a `(...)` subshell
    /// has multiple corner cases).
    #[test]
    fn set_e_off_continues() {
        assert_parity(r#"set +e; false; echo "after""#);
    }

    /// Top-level `set -e` aborts on naked `false`.
    #[test]
    fn set_e_aborts_naked() {
        let z = run_zsh("set -e; false; echo after");
        let r = run_zshrs("set -e; false; echo after");
        // Neither should print "after"; both exit non-zero.
        assert!(!z.stdout.contains("after"));
        assert!(!r.stdout.contains("after"));
    }

    /// `$?` after pipeline.
    #[test]
    fn dollar_question_after_pipe() {
        assert_parity(r#"true | false; echo $?"#);
    }

    /// `pipestatus` array.
    #[test]
    fn pipestatus_after_pipe() {
        assert_parity(r#"true | false | true; print -l -- "${pipestatus[@]}""#);
    }

    /// `&&` chain status.
    #[test]
    fn and_chain_status() {
        assert_parity(
            r#"true && true && true; echo $?
true && false && true; echo $?"#,
        );
    }
}

// ───────────────────────── string parsing ────────────────

mod string_parsing {
    use super::*;

    /// CSV-like split.
    #[test]
    fn split_on_colon() {
        assert_parity(r#"x="alpha:beta:gamma"; print -l -- "${(s.:.)x}""#);
    }

    /// Split on whitespace via word-split.
    #[test]
    fn split_on_whitespace() {
        assert_parity(r#"x="a b  c   d"; print -l -- ${=x}"#);
    }

    /// `(@)` keep-array even in DQ.
    #[test]
    fn at_flag_array_keep() {
        assert_parity(
            r#"a=("first item" "second item")
print -l -- "${(@)a}""#,
        );
    }
}

// ───────────────────────── numeric conversion ────────────────

mod numeric {
    use super::*;

    /// Hex parsing in arith.
    #[test]
    fn arith_hex_input() {
        assert_parity(r#"x=0xff; echo $((x))"#);
    }

    /// Negative numbers.
    #[test]
    fn arith_negative_unary() {
        assert_parity(r#"x=10; echo $((-x))"#);
    }

    /// printf %d.
    #[test]
    fn printf_d_format() {
        assert_parity(r#"printf '%d\n' 42"#);
    }

    /// printf %x hex.
    #[test]
    fn printf_x_hex() {
        assert_parity(r#"printf '%x\n' 255"#);
    }

    /// printf %.2f float precision.
    #[test]
    fn printf_float_precision() {
        assert_parity(r#"printf '%.2f\n' 3.14159"#);
    }

    /// Integer overflow handling — i64.
    #[test]
    fn arith_large_number() {
        assert_parity(r#"echo $(( 9999999 * 9999999 ))"#);
    }
}

// ───────────────────────── command substitution shapes ────────────

mod cmd_subst_shapes {
    use super::*;

    /// `$(cmd1 | cmd2)` chained.
    #[test]
    fn cmd_subst_pipeline() {
        assert_parity(r#"x=$(echo hello | tr a-z A-Z); print -- "$x""#);
    }

    /// `$(cmd; cmd)` sequence.
    #[test]
    fn cmd_subst_sequence() {
        assert_parity(r#"x=$(echo a; echo b); print -- "[$x]""#);
    }

    /// Nested cmd-subst.
    #[test]
    fn cmd_subst_nested() {
        assert_parity(r#"echo $(echo $(echo "deep"))"#);
    }

    /// Backtick form.
    #[test]
    fn backtick_subst() {
        assert_parity(r#"x=`echo hello`; print -- "$x""#);
    }

    /// Cmd-subst as conditional argument.
    #[test]
    fn cmd_subst_in_condition() {
        assert_parity(r#"if [[ -n "$(echo hello)" ]]; then echo y; fi"#);
    }
}
