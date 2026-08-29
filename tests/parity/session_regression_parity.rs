//! Regression guards for the parity gaps closed in the v0.12.38 cycle, plus
//! the shell idioms immediately around them.
//!
//! Every test here asserts CURRENT, CORRECT behaviour — these are tripwires,
//! not aspirational pins. Each one was measured against `/bin/zsh` before
//! being written, and the modules are grouped by the bug they defend so a
//! future refactor that reopens one is attributed immediately.
//!
//! Deliberately free of anything a headless Linux CI cannot do: no pty, no
//! job-control timing that depends on scheduling order, no locale-sensitive
//! collation, no network, and no reliance on files outside the test's own
//! temp dir.

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
    let o = Command::new(zsh_path()).args(["-fc", s]).output().expect("zsh");
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

/// Assert stdout AND exit status match the reference shell.
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

// ───────────────── #1088 — a leading `~` in a match pattern ─────────────────

/// c:Src/cond.c:299 `singsub`s the `==` RHS before `patcompile`, and
/// c:Src/loop.c:610 does the same for `case`, so an unquoted leading `~` in a
/// PATTERN is a home directory. powerlevel10k's `_POWERLEVEL9K_DIR_CLASSES`
/// walk is the consumer that exposed it (internal/p10k.zsh:2029).
mod tilde_in_match_patterns {
    use super::*;

    /// The p10k class walk in miniature: first match wins, so `~` must not
    /// swallow a subfolder and `~/*` must beat the catch-all.
    #[test]
    fn dir_class_walk_picks_the_first_matching_class() {
        assert_parity(
            r#"for cwd in "$HOME" "$HOME/a/b" /etc/zshrc /tmp; do
                 for a in '/etc|/etc/*' '~' '~/*' '*'; do
                   [[ $cwd == ${~a} ]] && { print -r -- "$cwd -> $a"; break }
                 done
               done"#,
        );
    }

    #[test]
    fn tilde_pattern_from_a_variable_and_from_source_agree() {
        assert_parity(
            r#"c=$HOME/x; p='~/*'
               [[ $c == ${~p} ]] && print var-yes || print var-no
               [[ $c == ~/*   ]] && print src-yes || print src-no"#,
        );
    }

    /// Regression guard: a QUOTED leading tilde stays literal.
    #[test]
    fn quoted_tilde_is_not_expanded() {
        assert_parity(r#"[[ '~/a' == '~/'* ]] && print lit || print no"#);
    }
}

// ────────── #1090 — backslash provenance: source quote vs pattern DATA ──────────

/// c:Src/glob.c:3633-3643 `zshtokenize` folds `\X` into a quote marker ONLY
/// when X is a glob metacharacter; before anything else BOTH bytes survive as
/// literals. zpwr's `_files` override is the consumer — its dedup guard
/// `(( $tried[(I)${(q)tmp}] ))` uses a `${(q)}`-quoted needle.
mod backslash_provenance {
    use super::*;

    const FIX: &str = "a=('a b'); b=('a\\ b'); p='a\\ b'\n";

    fn with_fix(body: &str) -> String {
        format!("{FIX}{body}")
    }

    /// A backslash that is DATA must not match a plain space, and must match a
    /// real backslash — both directions, or the bug is only half-caught.
    #[test]
    fn data_backslash_matches_only_the_literal_backslash() {
        assert_parity(&with_fix(
            r#"print -r -- "I=${a[(I)$p]}/${b[(I)$p]} R=${b[(r)$p]}""#,
        ));
    }

    /// The `${(q)}`-quoted-needle shape `_files` actually uses.
    #[test]
    fn quoted_needle_does_not_match_the_unquoted_value() {
        assert_parity(
            r#"tried=('x' 'a b'); tmp='a b'
               print -r -- "hit=${tried[(I)${(q)tmp}]}""#,
        );
    }

    /// The globsubst leg: `${~p}` in a cond RHS and in a `case` arm.
    #[test]
    fn globsubst_pattern_keeps_data_backslash_literal() {
        assert_parity(&with_fix(
            r#"[[ 'a b'  == ${~p} ]] && print t1-match || print t1-no
               [[ 'a\ b' == ${~p} ]] && print t2-match || print t2-no
               case 'a b'  in ${~p}) print c1-match;; *) print c1-no;; esac
               case 'a\ b' in ${~p}) print c2-match;; *) print c2-no;; esac"#,
        ));
    }

    /// Source-level quoting must keep working — this is the half that an
    /// over-eager fix breaks (it regressed 8 real-world corpus tests once).
    #[test]
    fn source_quoted_space_still_matches_with_an_active_star() {
        assert_parity(
            r#"for b in "man ls" "git log" "manatee x"; do
                 [[ "$b" = man\ * ]] && print "$b -> man" || print "$b -> wrap"
               done
               case 'man ls' in man\ *) print case-yes;; *) print case-no;; esac"#,
        );
    }

    /// The substitution operators are the other source-provenance consumer.
    #[test]
    fn substitution_patterns_keep_source_escapes() {
        assert_parity(
            r#"branch="feat%50";      print -r -- "${branch//\%/%%}"
               d="path/to=val";       print -r -- "${d//\//--}"
               e='/main.ch%git';      print -r -- "[${e%\%*}]""#,
        );
    }

    /// Escapes before REAL metacharacters are honoured on both paths.
    #[test]
    fn escaped_metacharacters_are_literal_from_both_provenances() {
        assert_parity(
            r#"d=('a*b' 'azb'); q='a\*b'
               print -r -- "src=${d[(I)a\*b]} data=${d[(I)$q]}"
               c=('a$b'); print -r -- "dollar=${c[(I)a\$b]}"
               e=('ab\' 'ab'); f='ab\'; print -r -- "trail=${e[(I)$f]}""#,
        );
    }
}

// ───────── #1091 — `_arguments` action lists are an array assignment ─────────

/// c:Completion/Base/Utility/_arguments:425 — `eval ws\=\( "${action[3,-3]}" \)`.
/// The body is an array-assignment word list, so it gets the full shell
/// tokenizer. Splitting on whitespace turned one described value into four
/// matches and took `7z <TAB>` from 0.10s to over 25s.
mod arguments_action_tokenization {
    use super::*;

    #[test]
    fn escaped_colon_and_quoted_description_stay_one_word() {
        assert_parity(
            r#"for body in 'a\:"add files to archive" b\:"benchmark"' \
                           'alpha "beta gamma" delta\ eps' \
                           "x:'one two' y:three" \
                           'p q r'; do
                 eval "ws=( $body )"
                 print -r -- "n=${#ws} :: ${(j:|:)${(qq)ws[@]}}"
               done"#,
        );
    }
}

// ───────── #1089 — `return` out of a redirected compound command ─────────

/// c:Src/exec.c:4364 runs `fixfds(save)` on EVERY exit path, `return`'s
/// included. gitstatus tripped on this: its daemon sources `gitstatus/install`,
/// whose `_gitstatus_install_main` returns out of a redirected `while` loop.
mod redirect_restored_on_return {
    use super::*;

    /// Every compound shape that carries a redirection, in one script, with a
    /// second file proving fd 0 came back rather than merely closing.
    #[test]
    fn fd0_is_restored_after_return_from_each_compound_shape() {
        assert_parity(
            r#"d=$(mktemp -d) || exit 1
               printf 'IN1\nIN2\n' > $d/inner
               printf 'OUTER\n'    > $d/outer
               f1() { local l; while IFS= read -r l; do return 0; done < $d/inner }
               f2() { local l; { read -r l; return 0 } < $d/inner }
               f3() { local l; for l in a b; do return 0; done < $d/inner }
               f4() { if true; then return 0; fi < $d/inner }
               for f in f1 f2 f3 f4; do
                 { $f; print -n "$f="; cat } < $d/outer
               done
               rm -rf -- $d"#,
        );
    }

    /// Regression guard on the other side: a bare `exec` redirect is
    /// deliberately NOT restored (c:Src/exec.c:3978-3986, nullexec==1).
    #[test]
    fn bare_exec_redirect_survives_the_function() {
        assert_parity(
            r#"d=$(mktemp -d) || exit 1
               printf 'IN\n' > $d/inner; printf 'OUT\n' > $d/outer
               f() { exec < $d/inner }
               { f; print -n 'fd0='; cat } < $d/outer
               rm -rf -- $d"#,
        );
    }
}

// ─────────────────────── EXIT-trap scope interactions ───────────────────────

/// `endtrapscope` (c:Src/signals.c:885-903) pulls the SIGEXIT trap aside at
/// function return and runs it last, branching on `ZSIG_FUNC`: a STRING trap
/// (`trap "…" EXIT`) takes the `else` arm and runs `siglists[SIGEXIT]`, leaving
/// a global `TRAPEXIT` FUNCTION untouched until the shell itself exits.
mod exit_trap_scope {
    use super::*;

    #[test]
    fn global_trapexit_function_runs_once_at_shell_exit() {
        assert_parity(r#"TRAPEXIT(){ print T }; f(){ print body }; f; print after"#);
    }

    #[test]
    fn function_local_string_trap_runs_at_function_return() {
        assert_parity(r#"f(){ trap "print inner" EXIT; print body }; f; print after"#);
    }

    #[test]
    fn string_global_with_string_local_nests_correctly() {
        assert_parity(
            r#"trap "print G" EXIT; f(){ trap "print inner" EXIT; print body }; f; print after"#,
        );
    }

    #[test]
    fn function_global_with_function_local_nests_correctly() {
        assert_parity(
            r#"TRAPEXIT(){ print T }; f(){ TRAPEXIT(){ print inner }; print body }; f; print after"#,
        );
    }

    #[test]
    fn string_global_with_function_local_nests_correctly() {
        assert_parity(
            r#"trap "print G" EXIT; f(){ TRAPEXIT(){ print inner }; print body }; f; print after"#,
        );
    }

    /// The one combination that diverges: a global TRAPEXIT **function** plus a
    /// function-local **string** `trap … EXIT`. zshrs runs the global TRAPEXIT
    /// an extra time at function return —
    ///     zsh   : body / inner / after / T
    ///     zshrs : body / inner / T / after / T
    /// The other three global/local × func/string combinations all agree, so
    /// the fault is specific to the `ZSIG_FUNC` bit surviving the install of a
    /// string trap: `endtrapscope` (c:894) then takes the
    /// `removehashnode(shfunctab, "TRAPEXIT")` arm instead of the
    /// `siglists[SIGEXIT]` one. `settrap` itself looks faithful
    /// (`*slot = ZSIG_TRAPPED` then `|= flags`, c:725/738), so the bit is
    /// surviving somewhere between the install and the scope pop.
    #[test]
    #[ignore = "open gap: a global TRAPEXIT *function* plus a function-local *string* \
`trap … EXIT` runs the global one an extra time at function return. Isolated to that single \
combination; the other three func/string pairings agree. See the module doc for the \
c:Src/signals.c:894 ZSIG_FUNC branch."]
    fn function_global_with_string_local_does_not_double_fire() {
        assert_parity(
            r#"TRAPEXIT(){ print T }; f(){ trap "print inner" EXIT; print body }; f; print after"#,
        );
    }
}
