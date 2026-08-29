//! Parity for the runtime context a shell exposes about itself: the special
//! parameters, the hook functions, `cd` navigation, file tests, anonymous
//! functions, the quoting-flag family and `select`.
//!
//! Prompt themes and framework glue read this surface constantly —
//! `$funcstack` for call-site reporting, `$LINENO` for tracebacks, `chpwd` for
//! directory hooks, `${(q)}` for round-tripping user data through `eval`. It
//! is exactly the sort of thing that is never called deliberately in a test
//! and breaks silently in production.
//!
//! House rules for this file, each earned from a probe that produced a FALSE
//! divergence during the sweep behind it:
//!
//!   * **No `mktemp` path may reach stdout.** The two shells get different
//!     temp dirs, so printing one always "diverges". `cd` in and print
//!     basenames or counts.
//!   * **Never assert on `$0` or the shell's own name.** zsh reports `zsh`
//!     and zshrs reports `zshrs`; that is correct, not a gap.
//!   * **No wall-clock, scheduling order, tty, or locale-sensitive
//!     collation.** Everything here is deterministic and headless.
//!   * **Every case must print something under the reference shell.** A
//!     script that outputs nothing in both shells passes on an empty-vs-empty
//!     comparison and guards nothing.

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

fn shell_out(bin: &str, args: &[&str], script: &str) -> (String, i32) {
    let o = Command::new(bin)
        .args(args)
        .arg(script)
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("shell spawn");
    (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        o.status.code().unwrap_or(-1),
    )
}

fn assert_parity(script: &str) {
    if !zsh_available() {
        return;
    }
    let (zo, ze) = shell_out(zsh_path(), &["-fc"], script);
    let (ro, re) = shell_out(zshrs_bin().to_str().unwrap(), &["--zsh", "-f", "-c"], script);
    assert_eq!(
        zo, ro,
        "stdout divergence on script:\n{script}\n--- zsh ---\n{zo:?}\n--- zshrs ---\n{ro:?}"
    );
    assert_eq!(ze, re, "exit divergence on script:\n{script}");
}

/// Compare stderr as well — for diagnostics, the message IS the behaviour.
fn assert_stderr_parity(script: &str) {
    if !zsh_available() {
        return;
    }
    let za = Command::new(zsh_path()).args(["-fc", script]).output().expect("zsh");
    let ra = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", script])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    let z = String::from_utf8_lossy(&za.stderr).into_owned();
    let r = String::from_utf8_lossy(&ra.stderr).into_owned();
    assert!(!z.trim().is_empty(), "reference produced no diagnostic for:\n{script}");
    assert_eq!(z, r, "stderr divergence on script:\n{script}");
}

// ───────────────────────── special parameters ─────────────────────────

mod special_parameters {
    use super::*;

    /// `$funcstack` is how a traceback or a logging wrapper names its caller.
    #[test]
    fn funcstack_names_the_current_and_calling_function() {
        assert_parity(r#"f(){ print -r -- "${funcstack[1]}/${funcstack[2]}/${#funcstack}" }; g(){ f }; g"#);
    }

    #[test]
    fn lineno_advances_at_top_level_and_resets_inside_a_function() {
        assert_parity("print -r -- \"$LINENO\"\nprint -r -- \"$LINENO\"\nf(){ print -r -- \"$LINENO\" }\nf\n");
    }

    /// `$_` holds the last argument of the previous command.
    #[test]
    fn underscore_holds_the_previous_commands_last_argument() {
        assert_parity(r#"print alpha >/dev/null; print -r -- "[$_]"; print beta gamma >/dev/null; print -r -- "[$_]""#);
    }

    #[test]
    fn positional_count_and_join_inside_a_function() {
        assert_parity(r#"f(){ print -r -- "${#@}/$*/${(j:-:)@}" }; f a b c"#);
    }
}

// ───────────────────────────── hook functions ─────────────────────────────

mod hooks {
    use super::*;

    /// `precmd` belongs to the interactive prompt loop — a non-interactive
    /// `-c` run must never fire it. Frameworks rely on that to stay cheap in
    /// scripts.
    #[test]
    fn precmd_does_not_fire_in_a_non_interactive_shell() {
        assert_parity(r#"precmd(){ print PRECMD-RAN }; print -r -- body"#);
    }

    /// `chpwd` fires on a directory change, in both shells, without leaking
    /// the temp path into the comparison.
    #[test]
    fn chpwd_fires_on_directory_change() {
        assert_parity(
            // Install the hook only AFTER landing in the temp dir, and move
            // solely between directories with names we chose — any `cd` that
            // lands on the mktemp dir itself prints its random basename and
            // the two shells "diverge" on nothing.
            r#"d=$(mktemp -d) || exit 1; cd $d; mkdir -p a/b
               chpwd(){ print -r -- "chpwd:${PWD:t}" }
               cd a; cd b; cd ..
               unfunction chpwd
               cd /; rm -rf -- $d"#,
        );
    }

    #[test]
    fn command_not_found_handler_intercepts_and_sets_status() {
        assert_parity(
            r#"command_not_found_handler(){ print -r -- "cnf:$1"; return 42 }
               nosuchcommand_xyz123; print -r -- "rc=$?""#,
        );
    }
}

// ──────────────────────────── directory navigation ────────────────────────────

mod navigation {
    use super::*;

    /// `cd -` returns to the previous directory. Compare basenames only.
    #[test]
    fn cd_dash_returns_to_the_previous_directory() {
        assert_parity(
            r#"d=$(mktemp -d) || exit 1; cd $d; mkdir -p a
               cd a; print -r -- "1:${PWD:t}"
               cd -  >/dev/null; print -r -- "2:${PWD:t}" | sed 's|tmp\..*|TMPDIR|'
               cd /; rm -rf -- $d"#,
        );
    }

    /// CDPATH lets a bare name resolve against a search path.
    #[test]
    fn cdpath_resolves_a_bare_directory_name() {
        assert_parity(
            r#"d=$(mktemp -d) || exit 1; mkdir -p $d/target
               CDPATH=$d; cd target >/dev/null 2>&1 && print -r -- "at:${PWD:t}"
               cd /; rm -rf -- $d"#,
        );
    }

    /// AUTO_CD is deliberately NOT tested here: it does not fire under
    /// `zsh -fc` at all (both shells report `command not found`), so a test
    /// built on it asserts nothing but the temp directory's random basename.
    /// The dirstack is the navigation behaviour that is actually observable
    /// non-interactively, so pin that instead.
    #[test]
    fn pushd_and_popd_move_through_the_dirstack() {
        assert_parity(
            r#"d=$(mktemp -d) || exit 1; cd $d; mkdir -p a b
               pushd a >/dev/null; print -r -- "1:${PWD:t} depth=${#dirstack}"
               pushd ../b >/dev/null; print -r -- "2:${PWD:t} depth=${#dirstack}"
               popd >/dev/null; print -r -- "3:${PWD:t}"
               cd /; rm -rf -- $d"#,
        );
    }
}

// ────────────────────────────── file tests ──────────────────────────────

mod file_tests {
    use super::*;

    #[test]
    fn type_and_size_operators_classify_correctly() {
        assert_parity(
            r#"d=$(mktemp -d) || exit 1; cd $d
               print -n x > full; : > empty; mkdir dir; ln -s full link
               [[ -f full  ]] && print f-file
               [[ -d dir   ]] && print d-dir
               [[ -h link  ]] && print h-symlink
               [[ -s full  ]] && print s-nonempty
               [[ -s empty ]] || print s-empty-false
               [[ -e nope  ]] || print e-missing-false
               cd /; rm -rf -- $d"#,
        );
    }

    /// `-nt` / `-ot` compare mtimes; `touch` with explicit stamps keeps this
    /// deterministic instead of leaning on a sleep.
    #[test]
    fn newer_than_and_older_than_compare_mtimes() {
        assert_parity(
            r#"d=$(mktemp -d) || exit 1; cd $d
               touch -t 202001010000 old; touch -t 202501010000 new
               [[ new -nt old ]] && print nt
               [[ old -ot new ]] && print ot
               [[ old -nt new ]] || print nt-false
               cd /; rm -rf -- $d"#,
        );
    }
}

// ─────────────────────── anonymous functions + quoting ───────────────────────

mod anonymous_functions {
    use super::*;

    #[test]
    fn anonymous_function_receives_arguments_and_scopes_locals() {
        assert_parity(
            r#"() { print -r -- "anon:$1" } arg1
               () { local x=inner; print -r -- "local:$x" }
               print -r -- "outer:${x:-unset}""#,
        );
    }
}

mod quoting_flags {
    use super::*;

    /// `${(q)}` is how frameworks round-trip arbitrary user data through
    /// `eval`; the escalating forms must stay distinct.
    #[test]
    fn q_family_escapes_at_escalating_strengths() {
        assert_parity(r#"s="a b'c\"d"; print -r -- "${(q)s}|${(qq)s}|${(qqq)s}""#);
    }

    #[test]
    fn q_then_Q_round_trips_a_value_with_a_space() {
        assert_parity(r#"a=(x "y z"); print -r -- "${(q)a[2]}|${(Q)${(q)a[2]}}""#);
    }
}

// ───────────────────────── select and history options ─────────────────────────

mod control_and_history {
    use super::*;

    /// `select` prints its menu to stderr and reads from stdin; with stdin at
    /// EOF the loop exits without running the body.
    #[test]
    fn select_with_closed_stdin_exits_without_running_the_body() {
        assert_parity(r#"select x in a b; do print -r -- "body:$x"; break; done < /dev/null; print -r -- "after:$?""#);
    }

    #[test]
    fn hist_ignore_dups_collapses_an_immediate_repeat() {
        assert_parity(
            r#"setopt histignoredups
               print -s one; print -s one; print -s two
               print -r -- "n=$(fc -l -3 2>/dev/null | wc -l | tr -d ' ')""#,
        );
    }

    #[test]
    fn printf_escape_and_time_conversions() {
        assert_parity(r#"printf "%b\n" "a\tb"; printf "%(%Y)T\n" -1 | grep -qE '^[0-9]{4}$' && print T-ok"#);
    }
}

// ──────────────────────── cond parse-error diagnostics ────────────────────────

/// `[[ word word ]]` with no operator is a parse error, and the message names
/// the offending word. c:Src/cond.c reports the RAW parse-tree word, so the
/// source spelling survives — `$t` stays `$t`.
mod cond_parse_errors {
    use super::*;

    /// A literal word renders identically in both shells.
    #[test]
    fn literal_word_is_reported_verbatim() {
        assert_stderr_parity(r#"[[ lit f ]]"#);
    }

    /// Any word carrying a substitution diverges: zsh prints the SOURCE text,
    /// zshrs prints it stripped to the bare parameter name.
    ///
    ///     [[ $t f ]]      zsh `$t`     zshrs `t`
    ///     [[ ${t} f ]]    zsh `${t}`   zshrs `t`
    ///     [[ "$t" f ]]    zsh `"$t"`   zshrs `t`
    ///     [[ $arr f ]]    zsh `$arr`   zshrs `arr`
    ///
    /// The literal case above agrees, so the message plumbing is right and
    /// only the word's provenance is lost — the diagnostic is being built from
    /// the expanded/untokenized word instead of the raw one. See BUGS.md #1121.
    #[test]
    #[ignore = "open gap: the cond parse-error diagnostic prints the substituted word rather \
than its source text — `[[ $t f ]]` reports `t` where zsh reports `$t`. Literal words agree, so \
only the raw-word provenance is missing. BUGS.md #1121."]
    fn substituted_word_is_reported_with_its_source_spelling() {
        assert_stderr_parity(r#"t=x; [[ $t f ]]"#);
    }
}
