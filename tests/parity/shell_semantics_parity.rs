//! Parity coverage for the semantics a framework sits on top of: the option
//! matrix, word splitting, scoping rules, `emulate` modes, `zstyle`
//! precedence, here-documents, pipeline status and the arithmetic surface.
//!
//! These are the rules zpwr / zinit / powerlevel10k depend on being *exactly*
//! right — not features anyone calls directly, but the substrate under every
//! line they run. A divergence here does not break one plugin, it changes the
//! meaning of everyone's code at once, which is why they are worth pinning
//! even though all of them currently pass.
//!
//! House rules for cases in this file, each learned from a probe that produced
//! a FALSE divergence:
//!
//!   * **No `mktemp` path may reach stdout** — the two shells get different
//!     temp dirs, so printing one always "diverges". `cd` in first and print
//!     basenames or counts.
//!   * **No wall-clock, scheduling order, tty or locale-sensitive collation.**
//!     Everything here is deterministic and headless.

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

fn out_of(bin: &str, args: &[&str], script: &str) -> (String, i32) {
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
    let (zo, ze) = out_of(zsh_path(), &["-fc"], script);
    let (ro, re) = out_of(zshrs_bin().to_str().unwrap(), &["--zsh", "-f", "-c"], script);
    assert_eq!(
        zo, ro,
        "stdout divergence on script:\n{script}\n--- zsh ---\n{zo:?}\n--- zshrs ---\n{ro:?}"
    );
    assert_eq!(ze, re, "exit divergence on script:\n{script}");
}

// ───────────────────── word splitting and the glob options ─────────────────────

mod splitting_and_glob_options {
    use super::*;

    /// Unquoted `$v` does NOT split in zsh unless SH_WORD_SPLIT is set — the
    /// single most load-bearing difference from every other shell.
    #[test]
    fn shwordsplit_changes_unquoted_expansion_arity() {
        assert_parity(
            r#"v="a b  c"; f(){ print -r -- "$#" }
               f $v
               setopt shwordsplit
               f $v"#,
        );
    }

    #[test]
    fn ifs_splitting_keeps_empty_fields_and_honours_newline() {
        assert_parity(
            r#"IFS=:; v="a:b::c"; a=($=v); print -r -- "n=$#a [${(j:|:)a}]"
               IFS=$'\n'; w=$'x\ny'; b=($=w); print -r -- "n=$#b""#,
        );
    }

    #[test]
    fn ksharrays_switches_to_zero_based_indexing() {
        assert_parity(r#"setopt ksharrays; a=(x y z); print -r -- "${a[0]}/${a[1]}/${#a[@]}""#);
    }

    /// The three no-match policies, each in its own subshell so the option
    /// does not leak. `cd` in first so no temp path can reach stdout.
    #[test]
    fn nullglob_cshnullglob_and_nomatch_differ_on_a_failed_glob() {
        assert_parity(
            r#"d=$(mktemp -d) || exit 1; cd $d; touch f
               ( setopt nullglob;    a=(nope*);   print -r -- "null=$#a" )
               # CSH_NULL_GLOB errors only when EVERY glob in the word list
               # fails; a literal word does not count, so both operands must be
               # globs for the surviving-match behaviour to show.
               ( setopt cshnullglob; a=(f* nope*); print -r -- "csh=$#a" )
               ( setopt nomatch;     print nope* ) 2>&1 | sed 's|.*|nomatch-errored|'
               cd /; rm -rf -- $d"#,
        );
    }
}

// ───────────────────────────── scoping rules ─────────────────────────────

mod scoping {
    use super::*;

    /// zsh is dynamically scoped: a nested function sees the caller's locals.
    #[test]
    fn locals_are_visible_to_nested_functions() {
        assert_parity(r#"f(){ local -a a=(1 2); g(){ print -r -- "${#a}" }; g }; f"#);
    }

    #[test]
    fn typeset_g_escapes_the_function_scope_but_local_does_not() {
        assert_parity(r#"f(){ typeset -g G=set; local L=loc }; f; print -r -- "G=$G L=${L:-unset}""#);
    }

    /// LOCAL_OPTIONS restores the option state on return.
    #[test]
    fn localoptions_restores_on_return() {
        assert_parity(
            r#"f(){ setopt localoptions nullglob; print -r -- "in=$options[nullglob]" }
               f; print -r -- "out=$options[nullglob]""#,
        );
    }

    #[test]
    fn subshell_isolates_assignments_but_a_brace_group_does_not() {
        assert_parity(
            r#"x=1; ( x=2; print -r -- "sub=$x" ); print -r -- "after-sub=$x"
               y=1; { y=2 }; print -r -- "after-brace=$y""#,
        );
    }

    #[test]
    fn zsh_subshell_counts_nesting_depth() {
        assert_parity(
            r#"print -r -- "$ZSH_SUBSHELL"; ( print -r -- "$ZSH_SUBSHELL"; ( print -r -- "$ZSH_SUBSHELL" ) )"#,
        );
    }
}

// ──────────────────────── indirection and parameter forms ────────────────────────

mod indirection {
    use super::*;

    #[test]
    fn P_flag_dereferences_a_name_held_in_a_variable() {
        assert_parity(r#"name=target; target=val; print -r -- "${(P)name}""#);
    }

    #[test]
    fn P_flag_reaches_an_associative_arrays_keys() {
        assert_parity(r#"typeset -A h=(a 1); name=h; print -r -- "${(Pk)name}""#);
    }

    #[test]
    fn default_assign_and_error_forms_behave_distinctly() {
        assert_parity(
            r#"print -r -- "${undef1-def}"
               print -r -- "${undef2:=asg}/$undef2"
               ( print -r -- "${undef3:?boom}" ) 2>/dev/null; print -r -- "rc=$?""#,
        );
    }

    /// `${(t)…}` and `${(Pt)…}` report the declared type, which every
    /// introspecting plugin branches on.
    #[test]
    fn type_flag_reports_declared_attributes() {
        assert_parity(
            // Single-letter names collide with shell specials here — `typeset
            // -F F` plus `${(Pt)F}` makes zsh die with `bad math expression`,
            // which BOTH shells did identically, so the case passed while
            // asserting nothing. Distinct lowercase names give real output.
            r#"typeset -a ta; typeset -A th; typeset -i ti; typeset -F tf
               for v in ta th ti tf; do print -r -- "$v=${(Pt)v}"; done"#,
        );
    }
}

// ────────────────────────────── zstyle lookup ──────────────────────────────

mod zstyle_lookup {
    use super::*;

    /// The most specific matching context wins — the rule every completion
    /// config in existence depends on.
    #[test]
    fn most_specific_context_pattern_wins() {
        assert_parity(
            r#"zmodload zsh/zutil
               zstyle ":a:b:c" opt v1
               zstyle ":a:*"   opt v2
               zstyle -s ":a:b:c" opt o; print -r -- "$o""#,
        );
    }

    #[test]
    fn boolean_lookup_reports_hit_and_miss_via_status() {
        assert_parity(
            r#"zmodload zsh/zutil; zstyle ":x:*" t yes
               zstyle -t ":x:y" t; print -r -- "hit=$?"
               zstyle -t ":z:y" t; print -r -- "miss=$?""#,
        );
    }

    #[test]
    fn array_lookup_fills_the_named_array() {
        assert_parity(
            r#"zmodload zsh/zutil; zstyle ":p:*" arr a b c
               zstyle -a ":p:q" arr A; print -r -- "${#A}/${A[2]}""#,
        );
    }
}

// ───────────────────────────── here-documents ─────────────────────────────

mod heredocs {
    use super::*;

    /// A quoted delimiter suppresses expansion; an unquoted one does not.
    #[test]
    fn quoted_delimiter_suppresses_expansion() {
        assert_parity(
            "x=V\ncat <<EOF\nval=$x\nEOF\ncat <<\"EOF\"\nval=$x\nEOF\n",
        );
    }

    /// `<<-` strips leading TABS only (the body below is tab-indented).
    #[test]
    fn dash_form_strips_leading_tabs() {
        assert_parity("cat <<-EOF\n\tindented\n\tEOF\n");
    }

    #[test]
    fn file_read_substitution_and_backticks_agree() {
        assert_parity(
            r#"d=$(mktemp -d) || exit 1; cd $d
               print -n body > f
               print -r -- "[$(<f)]"
               print -r -- "[`print -n bt`]"
               print -r -- "[$(print -n "$(print -n nested)")]"
               cd /; rm -rf -- $d"#,
        );
    }
}

// ─────────────────────────── pipelines and status ───────────────────────────

mod pipeline_status {
    use super::*;

    /// `$?` is the LAST stage; `$pipestatus` is every stage.
    #[test]
    fn pipestatus_records_every_stage_in_both_orders() {
        assert_parity(
            r#"false | true; print -r -- "$?/${(j:,:)pipestatus}"
               true | false; print -r -- "$?/${(j:,:)pipestatus}""#,
        );
    }

    #[test]
    fn stderr_pipe_form_merges_both_streams() {
        assert_parity(r#"f(){ print out; print err >&2 }; f |& sort"#);
    }
}

// ───────────────────────────── arithmetic surface ─────────────────────────────

mod arithmetic {
    use super::*;

    #[test]
    fn ternary_comma_and_inline_assignment_evaluate_left_to_right() {
        assert_parity(r#"print -r -- $(( 1 ? 2 : 3 )) $(( (1,2,3) )) $(( x = 5, x * 2 ))"#);
    }

    #[test]
    fn float_and_integer_division_differ() {
        // `int()` lives in zsh/mathfunc; without the module the whole
        // expression errors and the script prints NOTHING, which made this
        // case pass while asserting nothing. The integer-vs-float contrast is
        // the actual subject, so test that directly.
        assert_parity(r#"f=1.5; print -r -- $(( f * 2 )) $(( 7 / 2 )) $(( 7 / 2.0 )) $(( 1 / 3.0 ))"#);
    }

    #[test]
    fn literal_bases_are_recognised() {
        assert_parity(r#"print -r -- $(( 0x1f )) $(( 8#17 )) $(( 2#1010 ))"#);
    }

    #[test]
    fn modulo_keeps_the_sign_of_the_dividend() {
        assert_parity(r#"print -r -- $(( 7 % -3 )) $(( -7 % 3 )) $(( 2 ** 10 ))"#);
    }
}
