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
        c.args(["--zsh", "-f", "-c", script]).env_remove("ZSHRS_CACHE");
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
    assert_eq!(z.stdout, r.stdout, "script={} trailing={trailing:?}", script);
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
        assert_parity_with_trailing_args(
            r#"echo "$0-$1-$2-$3""#,
            &["nom", "aa", "bb", "cc"],
        );
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
