//! Pin `zsh -fc` against `zshrs --zsh -f -c`: default shell state,
//! CLI-linked flags (`$-`), and small language surfaces that must stay
//! aligned in parity mode.
//!
//! zshrs uses `-f` on our side of the harness so we match zsh's `-fc`
//! (no RC files) and avoid user `.zshenv` stdout leaking into captured
//! output — the same observable surface zsh's parity tests target.

use std::path::PathBuf;
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

/// Match [`run_zsh`]: `-f` suppresses RC noise; `--zsh` enables parity mode.
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

fn run_zshrs_compat(script: &str) -> ShellResult {
    let out = Command::new(zshrs_bin())
        .args(["--zsh-compat", "-f", "-c", script])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke zshrs --zsh-compat");
    ShellResult {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit: out.status.code().unwrap_or(-1),
    }
}

fn assert_parity(script: &str) {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let z = run_zsh(script);
    let r = run_zshrs(script);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on script:\n{}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        script, z.stdout, r.stdout
    );
    assert_eq!(
        z.stderr, r.stderr,
        "stderr divergence on script:\n{}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        script, z.stderr, r.stderr
    );
    assert_eq!(
        z.exit, r.exit,
        "exit divergence on script:\n{}\n--- zsh ---\n{}\n--- zshrs ---\n{}",
        script, z.exit, r.exit
    );
}

fn assert_parity_with_trailing_args(script: &str, trailing: &[&str]) {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let z = {
        let mut c = Command::new(zsh_path());
        c.arg("-fc").arg(script);
        for a in trailing {
            c.arg(a);
        }
        let out = c.output().expect("invoke zsh");
        ShellResult {
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            exit: out.status.code().unwrap_or(-1),
        }
    };
    let r = {
        let mut c = Command::new(zshrs_bin());
        c.args(["--zsh", "-f", "-c", script])
            .env_remove("ZSHRS_CACHE");
        for a in trailing {
            c.arg(a);
        }
        let out = c.output().expect("invoke zshrs");
        ShellResult {
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            exit: out.status.code().unwrap_or(-1),
        }
    };
    assert_eq!(
        z.stdout, r.stdout,
        "script={} trailing={trailing:?}",
        script
    );
    assert_eq!(z.stderr, r.stderr);
    assert_eq!(z.exit, r.exit);
}

mod shell_flags_dollar_minus {
    use super::*;

    #[test]
    fn default_plus_f_matches_zsh_fc() {
        assert_parity("echo $-");
    }

    #[test]
    fn after_set_errtrace_e() {
        assert_parity("set -e; echo $-");
    }

    #[test]
    fn after_set_xtrace_x_stderr_and_stdout() {
        // xtrace: PS4 + command line on stderr, `echo` result on stdout.
        assert_parity("set -x; echo $-");
    }
}

mod identity_and_defaults {
    use super::*;

    #[test]
    fn zsh_name_is_zsh() {
        assert_parity("echo $ZSH_NAME");
    }

    #[test]
    fn shlvl_integer_echo() {
        assert_parity("echo $SHLVL");
    }

    #[test]
    fn wordchars_default() {
        assert_parity("echo $WORDCHARS");
    }

    #[test]
    fn histchars_default() {
        assert_parity("echo $histchars");
    }

    #[test]
    fn optind_before_getopts() {
        assert_parity("echo $OPTIND");
    }
}

mod posix_c_positionals {
    use super::*;

    #[test]
    fn explicit_zero_and_rest() {
        assert_parity_with_trailing_args(r#"echo "$0-$1-$2-$3""#, &["nom", "aa", "bb", "cc"]);
    }

    #[test]
    fn star_joins_ifs_first_field() {
        assert_parity(r#"set -- p q; printf '[%s]\n' "$*""#);
    }

    #[test]
    fn at_preserves_separate_words() {
        assert_parity(r#"set -- p q; printf '[%s]\n' "$@""#);
    }

    #[test]
    fn argv_array_matches_positionals() {
        // `$argv` as a scalar joins with spaces; `"${argv[@]}"` pins the
        // array enumeration path (matches `print -l` on `$*`).
        assert_parity(
            r#"set -- x y; print -l -- "${argv[@]}"
echo "len=${#argv}""#,
        );
    }
}

mod conditionals_and_arith_cond {
    use super::*;

    #[test]
    fn double_bracket_pattern_glob() {
        assert_parity(r#"[[ xyz == *z ]] && echo glob"#);
    }

    #[test]
    fn double_bracket_string_equality() {
        assert_parity(r#"[[ a = a ]] && echo eq"#);
    }

    #[test]
    fn double_bracket_n_empty() {
        assert_parity(r#"[[ -n $PWD ]] && echo haspwd"#);
    }

    #[test]
    fn arith_cond_numeric() {
        assert_parity(r#"(( 7 > 3 )) && echo t"#);
    }
}

mod small_language_surface {
    use super::*;

    #[test]
    fn brace_expansion_commas() {
        assert_parity(r#"echo {a,b,c}"#);
    }

    #[test]
    fn arithmetic_power() {
        assert_parity(r#"echo $(( 2 ** 3 ))"#);
    }

    #[test]
    fn array_one_indexed_first_el() {
        assert_parity(r#"a=(u v w); echo $a[1]"#);
    }

    #[test]
    fn print_r_no_brand_mangling() {
        assert_parity(r#"print -r -- '-n--'"#);
    }
}

mod zsh_compat_cli_alias {
    use super::*;

    /// `--zsh-compat` must run the same parity surface as `--zsh`.
    #[test]
    fn matches_zsh_flag_for_script() {
        if !zsh_available() {
            return;
        }
        let script = r#"print -r -- "${${(q-)IFS}:0:3}""#;
        let z = run_zsh(script);
        let a = run_zshrs(script);
        let b = run_zshrs_compat(script);
        assert_eq!(z.stdout, a.stdout);
        assert_eq!(a.stdout, b.stdout);
        assert_eq!(z.exit, a.exit);
        assert_eq!(a.exit, b.exit);
    }
}

// ════════════════════════════════════════════════════════════════════
// Regression pins for the 2026-08-30 parity fixes. Each script below
// was verified by hand against `zsh -fc` before being pinned here; the
// absolute assertions keep the pin meaningful on a CI box with no zsh
// installed (where `assert_parity` skips).
// ════════════════════════════════════════════════════════════════════

/// A subscripted assignment must report the exit status of a command
/// substitution on its RHS, exactly like a scalar assignment does.
///
/// Fix: src/extensions/compile_zsh.rs — `compile_assign`'s
/// BUILTIN_SET_SUBSCRIPT_RANGE and BUILTIN_SET_ASSOC arms returned
/// without recording `last_assign_had_cmd_subst`, so
/// BUILTIN_ASSIGN_ONLY_STATUS took its `else { 0 }` branch.
/// c:Src/exec.c:3396 `lastval = cmdoutval`.
#[test]
fn assign_subscript_reports_cmdsubst_exit_status() {
    let script = r#"typeset -gA h; typeset -ga a
h[k]="$(false)"; print "assoc_false=$?"
h[k]="$(true)";  print "assoc_true=$?"
a[2]="$(false)"; print "array_false=$?"
s="$(false)";    print "scalar_false=$?""#;
    let r = run_zshrs(script);
    assert_eq!(
        r.stdout, "assoc_false=1\nassoc_true=0\narray_false=1\nscalar_false=1\n",
        "subscripted assignment must propagate cmdoutval like a scalar one"
    );
    assert_parity(script);
}

/// The shape that made this user-visible: VCS_INFO_detect_git uses the
/// assignment AS the condition of an `&&` chain. Outside a repository
/// `rev-parse` fails, so zsh takes the false branch and never runs the
/// git backend. Reporting success left `gitdir` empty and
/// VCS_INFO_git_getbranch read `$gitdir/HEAD` as `/HEAD`.
#[test]
fn assign_subscript_as_condition_fails_when_cmdsubst_fails() {
    let script = r#"typeset -gA vcs_comm
if vcs_comm[gitdir]="$(false)"; then
  print "SUCCESS gitdir=[${vcs_comm[gitdir]}]"
else
  print "FAIL gitdir=[${vcs_comm[gitdir]}]"
fi"#;
    let r = run_zshrs(script);
    assert_eq!(
        r.stdout, "FAIL gitdir=[]\n",
        "a failed $() in a subscripted assignment must fail the && chain"
    );
    assert_parity(script);
}

/// `${~...}` inside a `:=` / `::=` word must not leave GLOB_SUBST on
/// for the enclosing expansion. C keeps `globsubst` paramsubst-LOCAL
/// (c:Src/subst.c:1669) and clears it after the word unless the OUTER
/// spec forced it (c:3231-3232 `if (globsubst != 2) globsubst = 0;`).
///
/// Fix: src/ported/subst.rs — the assign arms now snapshot/restore
/// GLOB_SUBST + TILDE_GLOBSUBST_CARRIER around the word expansion.
/// Leaking it filename-globbed the result, so an unterminated bracket
/// became a fatal `bad pattern`. fast-syntax-highlighting runs
/// `: ${expanded_path::=${~_mybuf}}` on every keystroke.
#[test]
fn tilde_glob_inside_assign_word_does_not_leak() {
    for pat in ["[[", "foo[", "[a", "a(b", "*.txt", "["] {
        for op in ["::=", ":="] {
            let script = format!(
                "b={}\n: ${{e{}${{~b}}}}\nprint -r -- \"[$e]\"",
                shell_quote(pat),
                op
            );
            let r = run_zshrs(&script);
            assert_eq!(
                r.stdout,
                format!("[{}]\n", pat),
                "`{}` with b={:?} must assign the literal, not glob it",
                op,
                pat
            );
            assert!(
                !r.stderr.contains("bad pattern"),
                "no `bad pattern` for b={:?} op={} (stderr {:?})",
                pat,
                op,
                r.stderr
            );
            assert_parity(&script);
        }
    }
}

/// The other half of c:3231-3232: a `~` on the OUTER spec is "forced"
/// (globsubst == 2) and MUST still glob. Guards against fixing the
/// leak by disabling the flag outright.
#[test]
fn tilde_glob_on_outer_assign_spec_still_globs() {
    let script = r#"d=$(mktemp -d) || exit 1
cd "$d" || exit 1
: > f1.txt; : > f2.txt
b='*.txt'
print -r -- ${~e::=$b}
cd /; command rm -rf "$d""#;
    let r = run_zshrs(script);
    assert_eq!(
        r.stdout, "f1.txt f2.txt\n",
        "an outer `~` on the assign spec stays forced and still globs"
    );
    assert_parity(script);
}

/// `(V)` (render non-printing chars visible, c:Src/subst.c:2232) must
/// apply to a SUBSCRIPTED element, not just to a whole array or scalar.
///
/// Fix: src/extensions/compile_zsh.rs folded `(V)` into the same
/// "redundant flag" predicate as `(v)` and compiled `${(V)a[1]}` to a
/// bare BUILTIN_ARRAY_INDEX, dropping the flag; and src/ported/subst.rs
/// gates both `(V)` arms on `subscript.is_none()` so the arm that
/// re-fetches the array by name cannot discard the element selection.
#[test]
fn v_flag_applies_to_subscripted_element() {
    let script = r#"a=($'x\ny' $'p\tq')
typeset -A h; h[k]=$'x\ny'
s=$'x\ny'
print -r -- "elem1=${(V)a[1]}"
print -r -- "elem2=${(V)a[2]}"
print -r -- "neg=${(V)a[-1]}"
print -r -- "range=${(V)a[1,2]}"
print -r -- "assoc=${(V)h[k]}"
print -r -- "whole=${(V)a}"
print -r -- "scalar=${(V)s}""#;
    let r = run_zshrs(script);
    assert_eq!(
        r.stdout,
        "elem1=x\\ny\nelem2=p\\tq\nneg=p\\tq\nrange=x\\ny p\\tq\nassoc=x\\ny\nwhole=x\\ny p\\tq\nscalar=x\\ny\n",
        "(V) must escape non-printing chars through a subscript too"
    );
    assert_parity(script);
}

/// The folds that must KEEP working: `(v)` really is redundant with a
/// simple subscript (it asks for an assoc element's value), and `(k)`
/// yields the key. Pins that the `(V)` fix did not disable them.
#[test]
fn lowercase_v_and_k_subscript_folds_still_apply() {
    let script = r#"typeset -A h; h[k]=$'x\ny'; h[j]=plain
print -r -- "v=${(v)h[k]}"
print -r -- "k=${(k)h[j]}""#;
    let r = run_zshrs(script);
    assert_eq!(
        r.stdout, "v=x\ny\nk=j\n",
        "(v) stays a value fetch (raw newline) and (k) stays a key fetch"
    );
    assert_parity(script);
}

/// execcmd_exec's c:3315-3318 sweep isolates the head word, globs it,
/// then re-merges ahead of the tail. The merge moved from a per-element
/// `Vec::insert` (O(K*n) memmove) to a single `splice(0..0, ..)`; this
/// pins the ordering contract that made the swap safe — the globbed
/// head lands first, in match order, and the tail args keep their order.
///
/// Fix: src/ported/exec.rs.
#[test]
fn globbed_command_word_keeps_head_then_tail_order() {
    let script = r#"d=$(mktemp -d) || exit 1
cd "$d" || exit 1
cat > runme <<'SH'
#!/bin/sh
for a in "$@"; do printf '%s|' "$a"; done; echo
SH
chmod 755 runme
./run*e alpha beta gamma
cd /; command rm -rf "$d""#;
    let r = run_zshrs(script);
    assert_eq!(
        r.stdout, "alpha|beta|gamma|\n",
        "head glob resolves to the command and tail args keep order"
    );
    assert_parity(script);
}

/// Single-quote a string for embedding in a shell script.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}
