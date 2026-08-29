//! Parity coverage for the builtin + module surface an interactive config
//! actually leans on: traps, the `read`/`print` flag families, the loadable
//! modules, prompt escapes, multios, the directory stack, alias forms and
//! glob qualifiers.
//!
//! These are breadth tripwires. The suite already goes deep on expansion and
//! completion; this file covers the shell surface around them, where a
//! regression is cheap to introduce and expensive to notice — a broken
//! `read -A` or `print -v` does not fail loudly, it silently produces the
//! wrong shape three layers into someone's plugin.
//!
//! Two rules every case here follows, learned from probes that produced false
//! divergences:
//!
//!   * **Never let a `mktemp` path reach stdout.** The two shells get
//!     different temp dirs, so a script that prints one always "diverges".
//!     Print basenames, counts, or `cd` in first.
//!   * **No wall-clock, no scheduling order, no tty.** Everything here runs
//!     headless on Linux CI.

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

fn run(bin: &str, args: &[&str], script: &str) -> R {
    let mut c = Command::new(bin);
    c.args(args).arg(script).env_remove("ZSHRS_CACHE");
    let o = c.output().expect("shell spawn");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}

/// Compare stdout AND exit status against the reference shell.
fn assert_parity(script: &str) {
    if !zsh_available() {
        return;
    }
    let z = run(zsh_path(), &["-fc"], script);
    let r = run(zshrs_bin().to_str().unwrap(), &["--zsh", "-f", "-c"], script);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on script:\n{script}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        z.stdout, r.stdout
    );
    assert_eq!(z.exit, r.exit, "exit divergence on script:\n{script}");
}

// ─────────────────────────────── traps ───────────────────────────────

mod traps {
    use super::*;

    /// A function-scoped trap fires on the signal and is popped on return.
    #[test]
    fn function_scoped_signal_trap_fires_and_pops() {
        assert_parity(r#"f(){ trap "print T-$1" USR1; kill -USR1 $$; print after }; f one; print done"#);
    }

    /// `trap - SIG` inside a function resets only for that scope.
    #[test]
    fn trap_dash_resets_within_the_function_scope() {
        assert_parity(
            r#"trap "print INT" INT
               f(){ trap - INT; print body }
               f; print done"#,
        );
    }

    /// TRAPZERR runs on a non-zero status.
    #[test]
    fn trapzerr_runs_on_failure() {
        assert_parity(r#"TRAPZERR(){ print zerr }; false; print after"#);
    }

    /// ERR_RETURN unwinds the function without running the rest of it.
    #[test]
    fn errreturn_aborts_the_function_body() {
        // Scope the option to the function and let the caller survive: with a
        // bare `setopt errreturn` at top level the whole -c script aborts and
        // prints NOTHING, so the case passed on a matching exit code while
        // asserting no behaviour at all. Both shapes below print, and together
        // they pin the two halves — the body stops at the failure, and the
        // status reaches the caller.
        assert_parity(
            r#"f(){ setopt localoptions errreturn; print a; false; print unreached }
               f; print -r -- "rc=$? tail"
               setopt errreturn
               g(){ print c; false; print unreached2 }
               g || print -r -- "caught=$?""#,
        );
    }
}

// ──────────────────────────── read flag family ────────────────────────────

mod read_flags {
    use super::*;

    #[test]
    fn read_k_takes_a_single_character() {
        assert_parity(r#"read -k1 -u0 c <<< "xy"; print -r -- "[$c]""#);
    }

    #[test]
    fn read_d_uses_a_custom_delimiter() {
        assert_parity(r#"read -d : a <<< "ab:cd"; print -r -- "[$a]""#);
    }

    #[test]
    fn read_A_fills_an_array() {
        assert_parity(r#"read -A arr <<< "a b c"; print -r -- "${#arr}/${arr[2]}/${arr[-1]}""#);
    }

    /// Each `read` in a `{ }` on the right of a pipe consumes one line, and
    /// the assignments survive to the end of the group.
    #[test]
    fn successive_reads_in_a_piped_group_consume_successive_lines() {
        assert_parity(r#"print -l a b c | { read x; read y; print -r -- "$x$y" }"#);
    }
}

// ─────────────────────────── print flag family ───────────────────────────

mod print_flags {
    use super::*;

    #[test]
    fn print_v_assigns_instead_of_writing() {
        assert_parity(r#"print -v var hi; print -r -- "[$var]""#);
    }

    #[test]
    fn print_D_contracts_a_home_relative_path() {
        assert_parity(r#"print -D /tmp; print -D "$HOME/x""#);
    }

    #[test]
    fn print_n_suppresses_the_newline() {
        assert_parity(r#"print -n a; print -n b; print; print -r -- end"#);
    }

    /// `-z` pushes onto the editor buffer stack and `-s` onto history; neither
    /// writes to stdout, which is the property worth pinning.
    #[test]
    fn print_z_and_s_write_nothing_to_stdout() {
        assert_parity(r#"print -z buffered; print -s hist_entry; print -r -- only"#);
    }
}

// ──────────────────────────── loadable modules ────────────────────────────

mod modules {
    use super::*;

    #[test]
    fn datetime_strftime_formats_an_epoch() {
        assert_parity(r#"zmodload zsh/datetime; strftime -s s "%Y-%m-%d" 0; print -r -- "$s""#);
    }

    /// `zstat -H` populates an assoc keyed by field name. Compare the KEY set,
    /// not the values, which differ per file system.
    #[test]
    fn stat_H_populates_the_named_fields() {
        assert_parity(
            r#"zmodload zsh/stat
               zstat -H h /etc/hosts 2>/dev/null || { print nostat; return }
               print -r -- "${(j:,:)${(ko)h}}""#,
        );
    }

    #[test]
    fn regex_match_sets_MATCH() {
        assert_parity(
            r#"zmodload zsh/regex 2>/dev/null
               [[ abc =~ "^a.c$" ]] && print -r -- "m:$MATCH" || print n"#,
        );
    }

    #[test]
    fn system_sysopen_and_sysread_round_trip() {
        assert_parity(
            r#"zmodload zsh/system
               sysopen -r -u 3 /etc/hosts || { print noopen; return }
               sysread -i 3 -s 4 buf
               print -r -- "n=${#buf}"
               exec 3<&-"#,
        );
    }
}

// ─────────────────────────── prompt expansion ───────────────────────────

mod prompt_expansion {
    use super::*;

    /// The escapes a themed prompt leans on, rendered through `print -P`.
    #[test]
    fn colour_bold_and_ternary_escapes_render_identically() {
        assert_parity(r#"print -rP "%F{red}x%f|%B%bb|%(?.ok.bad)""#);
    }

    /// `%D{…}` is strftime; pin the SHAPE rather than the clock.
    #[test]
    fn strftime_escape_produces_a_four_digit_year() {
        assert_parity(r#"print -rP "%D{%Y}" | grep -qE '^[0-9]{4}$' && print ok || print bad"#);
    }

    /// Named directories drive `%~` and the `(D)` flag alike.
    #[test]
    fn named_directory_contracts_in_both_directions() {
        assert_parity(r#"hash -d nd=/tmp; print -r -- ~nd; print -r -- "${(D)$(print -r -- /tmp/x)}""#);
    }
}

// ───────────────────────────── redirection ─────────────────────────────

mod multios {
    use super::*;

    /// One `print` to two targets writes both — cd in first so no temp path
    /// reaches stdout.
    #[test]
    fn one_write_reaches_every_target() {
        assert_parity(
            r#"d=$(mktemp -d) || exit 1; cd $d
               print hi > a > b
               print -r -- "$(cat a)$(cat b)"
               cd /; rm -rf -- $d"#,
        );
    }

    /// Two input redirections concatenate.
    #[test]
    fn two_input_redirections_concatenate() {
        assert_parity(
            r#"d=$(mktemp -d) || exit 1; cd $d
               print -l x y > f
               cat < f < f | wc -l | tr -d ' '
               cd /; rm -rf -- $d"#,
        );
    }
}

// ───────────────────────── globbing + qualifiers ─────────────────────────

mod globbing {
    use super::*;

    #[test]
    fn recursive_glob_finds_nested_plain_files() {
        assert_parity(
            r#"d=$(mktemp -d) || exit 1; cd $d
               mkdir -p a/b; touch a/f1 a/b/f2
               print -rl -- **/*(.:t) | sort | tr '\n' ' '; print
               cd /; rm -rf -- $d"#,
        );
    }

    #[test]
    fn plain_file_qualifier_excludes_directories() {
        assert_parity(
            r#"d=$(mktemp -d) || exit 1; cd $d
               touch x1 x2; mkdir x3
               print -r -- "n=$(print -l x*(.) | wc -l | tr -d ' ')"
               cd /; rm -rf -- $d"#,
        );
    }

    #[test]
    fn globdots_includes_leading_dot_entries() {
        assert_parity(
            r#"setopt globdots
               d=$(mktemp -d) || exit 1; cd $d
               touch .h v
               print -rl -- *(:t) | sort | tr '\n' ' '; print
               cd /; rm -rf -- $d"#,
        );
    }
}

// ──────────────────────── names, aliases, functions ────────────────────────

mod names_and_aliases {
    use super::*;

    #[test]
    fn whence_command_and_builtin_agree_on_a_builtin() {
        assert_parity(r#"whence -w print; command -v print; builtin print bi"#);
    }

    #[test]
    fn global_alias_expands_mid_command() {
        assert_parity(r#"alias -g GG='| wc -l'; print -l a b GG | tr -d ' '"#);
    }

    #[test]
    fn functions_c_copies_a_function_body() {
        assert_parity(r#"f(){ print body }; functions -c f g; g; unfunction g; print -r -- "left=${+functions[g]}""#);
    }

    #[test]
    fn brace_expansion_forms_expand_in_order() {
        assert_parity(r#"a=({1..3}x); print -r -- "$a"; b=({a,b}{1,2}); print -r -- "$b""#);
    }

    /// The dirstack grows with AUTO_PUSHD; compare the COUNT, not the paths.
    #[test]
    fn autopushd_grows_the_dirstack() {
        assert_parity(
            r#"setopt autopushd
               d=$(mktemp -d) || exit 1; cd $d
               mkdir -p a b; cd a; cd ../b
               print -r -- "depth=${#dirstack}"
               cd /; rm -rf -- $d"#,
        );
    }
}
