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
    crate::oracle::zsh_path()
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

/// A loop's exit status (Src/loop.c): the condition-false exit restores the
/// last completed body's status (`lastval = oldval`, c:454-458), a `break`
/// keeps its own status (bin_break returns 0, Src/builtin.c:5863), and a
/// `continue` still reaches the post-body `oldval = lastval` (c:529-541).
/// zshrs had a `break` restore the PREVIOUS iteration's status, so
/// `while true; do …; [[ c ]] && break; done` ended 1 after any earlier
/// iteration's `[[ ]]` was false.
mod loop_exit_status {
    use super::*;

    #[test]
    fn break_ends_the_loop_with_its_own_status() {
        assert_parity(r#"n=0; while true; do (( n++ )); [[ $n -gt 2 ]] && break; done; print rc=$?"#);
        assert_parity(r#"n=0; until false; do (( n++ )); [[ $n -gt 2 ]] && break; done; print rc=$?"#);
        assert_parity(r#"n=0; while true; do (( n++ )); [[ $n -gt 2 ]] && { false; break }; done; print rc=$?"#);
        assert_parity(r#"f() { return 3 }; n=0; while true; do (( n++ )); [[ $n -gt 2 ]] && break; f; done; print rc=$?"#);
        assert_parity(r#"for i in 1 2; do false; break; done; print rc=$?; repeat 2 do false; break; done; print rc=$?"#);
        assert_parity(r#"n=0; while true; do (( n++ )); b=1; (( n > 2 )) && break $b; false; done; print rc=$?"#);
    }

    #[test]
    fn continue_feeds_the_status_the_condition_exit_restores() {
        assert_parity(r#"n=0; while (( n++ < 3 )); do [[ $n == 3 ]] && continue; false; done; print rc=$?"#);
        assert_parity(r#"n=0; while (( n++ < 3 )); do false; done; print rc=$?; while false; do :; done; print rc=$?"#);
    }
}

/// The word of `${x=word}`, `${x:=word}` and `${x::=word}` goes through
/// `multsub(&val, spbreak ? PREFORK_SINGLE : PREFORK_NOSHWORDSPLIT, NULL, …)`
/// (Src/subst.c:3256-3258), not a single-word expansion, so prefork brace-
/// expands it and joins the words: `${x:={a,b}}` assigns "a b". prefork's
/// `hasbraces` also honours BRACE_CCL (Src/glob.c:2049) for an operand.
/// docs/BUGS.md #647.
mod brace_expansion_in_operands {
    use super::*;

    #[test]
    fn assign_default_forms_brace_expand_their_word() {
        assert_parity(r#"print -r -- ${x:={a,b}}; print -r -- "[$x]""#);
        assert_parity(r#"print -r -- ${x={a,b}} ${y::=pre{a,b}post}; print -r -- "[$x] [$y]""#);
        assert_parity(r#"x=; print -r -- ${x:={1..3}}; print -r -- "[$x]""#);
        assert_parity(r#"v=w; print -r -- ${x:=$v{1,2}}; print -r -- "[$x]""#);
    }

    #[test]
    fn quoting_and_options_still_suppress_it() {
        assert_parity(r#"print -r -- "${x:={a,b}}"; print -r -- "[$x]""#);
        assert_parity(r#"print -r -- ${x:='{a,b}'} ${y:=\{a,b\}}; print -r -- "[$x] [$y]""#);
        assert_parity(r#"setopt ignorebraces; print -r -- ${x:={a,b}}; print -r -- "[$x]""#);
        assert_parity(r#"setopt shwordsplit; print -r -- ${x:={a,b}}; print -r -- "[$x]""#);
        assert_parity(r#"x=keep; print -r -- ${x:={a,b}}; print -r -- "[$x]""#);
    }

    #[test]
    fn brace_ccl_applies_to_an_operand() {
        assert_parity(r#"setopt braceccl; print -r -- ${x:-{ab}} ${y:={a-c}}; print -r -- "[$y]""#);
    }
}

/// The `~` flag (GLOB_SUBST) shtokenizes the substituted value, so prefork's
/// file pass (Src/subst.c:178-182) expands a leading `~` or `=cmd` and, in an
/// assignment (`PREFORK_ASSIGN`, Src/exec.c:2603), a `~` after `:`. No filename
/// generation follows a scalar assignment. zshrs stored `~/foo` verbatim.
/// docs/BUGS.md #644.
mod globsubst_file_expansion_on_assignment {
    use super::*;

    #[test]
    fn a_whole_word_reference_is_file_expanded() {
        assert_parity(r#"HOME=/hh; x="~/foo"; y=${~x}; z=$~x; print -r -- "[$y] [$z]""#);
        assert_parity(r#"HOME=/hh; x="a:~/b"; y=${~x}; print -r -- "[$y]""#);
        assert_parity(r#"HOME=/hh; x="~/foo"; local l=${~x}; typeset t=$~x; print -r -- "[$l] [$t]""#);
        assert_parity(r#"HOME=/hh; f() { local p=${~1}; print -r -- "[$p]" }; f '~/x'"#);
        assert_parity(r#"x='a*'; y=$~x; print -r -- "[$y]""#);
    }

    #[test]
    fn quotes_segments_and_the_off_toggle_leave_it_alone() {
        assert_parity(r#"HOME=/hh; x="~/foo"; y="${~x}"; z=foo$~x; print -r -- "[$y] [$z]""#);
        assert_parity(r#"HOME=/hh; x="~/foo"; y=${~~x}; local l=pre${~x}; print -r -- "[$y] [$l]""#);
        assert_parity(r#"HOME=/hh; x="~/foo"; setopt shfileexpansion; y=${~x}; print -r -- "[$y]""#);
    }

    /// `$~N` / `$~#`: an unbraced positional (every digit, as zsh reads `$12`)
    /// or one-character special parameter, as `${~N}` names it.
    #[test]
    fn unbraced_positional_and_special_parameters_take_the_flag() {
        assert_parity(r#"set -- x y; print -r -- $~2 ${~2} $~#"#);
        assert_parity(r#"set -- 1 2 3 4 5 6 7 8 9 10 11 12; print -r -- $~12"#);
        assert_parity(r#"HOME=/hh; set -- '~/p'; y=$~1; print -r -- "[$y]""#);
    }
}

/// zsh has no `caller`, `help`, `mapfile`, `readarray` or `compopt`: in
/// `--zsh` each misses like any unknown command (Src/exec.c:903), with the
/// script / `(eval)` / function context in the message, however the name is
/// reached, while a user function of that name still runs. docs/BUGS.md
/// #475, #504.
mod bash_only_builtins_are_absent {
    use super::*;

    #[test]
    fn literal_and_runtime_names_miss() {
        assert_parity(r#"{ caller; print $?; help; print $?; readarray; print $?; compopt; print $? } 2>&1"#);
        assert_parity(r#"{ c=caller; $c; print $?; c=help; $c; print $?; c=readarray; $c; print $? } 2>&1"#);
        assert_parity(r#"{ eval caller; f() { c=mapfile; $c; print $? }; f } 2>&1"#);
    }

    #[test]
    fn a_user_function_of_that_name_still_runs() {
        assert_parity(r#"help() { print mine }; help; c=help; $c"#);
    }
}

/// `jobs %%` / `%+` / `%` / `%-` with no such job: the dev tree reports the
/// spec, `%%: no such job` (upstream 54294, Src/jobs.c getjob), not 5.9's
/// "no current job". docs/BUGS.md #393.
mod jobs_current_previous_spec_messages {
    use super::*;

    #[test]
    fn no_current_or_previous_job_names_the_spec() {
        assert_parity(r#"jobs %% 2>&1; jobs %+ 2>&1; jobs % 2>&1; jobs %- 2>&1; print $?"#);
    }
}

/// An empty parenthesised operand is an error, not 0. `(( () ))` used to be
/// stripped to the empty expression (status 1, silent): execarith evaluates the
/// text between `((` and `))` as written (Src/exec.c:5302-5308), so the error
/// context also keeps its trailing blank. An empty subscript flag group sets no
/// flag, so `${a[()]}` is the empty numeric operand (Src/params.c:1405 getarg,
/// c:1635 mathevalarg). docs/BUGS.md #527, #1035.
mod empty_parenthesised_operands {
    use super::*;

    #[test]
    fn arithmetic_command_rejects_an_empty_group() {
        assert_parity(r#"{ (( () )); print rc=$?; (( ( ) )); print rc=$?; (( (()) )); print rc=$? } 2>&1"#);
        assert_parity(r#"{ (( 1 + () )); print rc=$?; (( ()+1 )); print rc=$?; (( (1) )); print rc=$? } 2>&1"#);
    }

    #[test]
    fn empty_subscript_flag_group_is_an_empty_index() {
        assert_parity(r#"a=(1 2 3); { print -r -- ${a[()]}; print rc=$? } 2>&1"#);
        assert_parity(r#"a=(1 2 3); print -r -- ${a[()2]} ${a[()-1]} ${a[()1+1]} ${a[()2,3]}"#);
        assert_parity(r#"{ print -r -- ${${:-a b}[()]}; print rc=$? } 2>&1"#);
    }
}

/// A `>(cmd)` child is an AUXILIARY process of the job that consumed it
/// (Src/exec.c:5052 `addproc(pid, NULL, 1, …)`), so a builtin, group,
/// subshell or function that writes to one finishes only when the child has:
/// its output lands before the next command. An external command expands its
/// redirections in the forked child, so the shell itself does not wait there.
/// docs/BUGS.md #1062.
mod process_substitution_output_is_waited_for {
    use super::*;

    #[test]
    fn a_shell_command_waits_for_its_writer_child() {
        assert_parity(r#"print x > >(cat); print rc=$?"#);
        assert_parity(r#"{ print x > >(cat) }; ( print y > >(cat) ); print rc=$?"#);
        assert_parity(r#"f() { print x > >(cat) }; f; f > >(cat); print rc=$?"#);
        assert_parity(r#"print x > >(cat) > >(cat); print rc=$?"#);
        assert_parity(r#"print x > >(sleep 0.2; cat); print after"#);
    }

    #[test]
    fn an_enclosing_redirect_keeps_the_wait_until_its_scope_ends() {
        assert_parity(r#"{ print a > /dev/null; print b } > >(cat); print end"#);
        assert_parity(r#"for i in 1 2; do print $i > >(cat); done; print end"#);
    }
}

/// The `[@]` splat of an UNSET name is an empty WORD (Src/subst.c:3603-3610
/// leaves `isarr` at 0), so unquoted it is removed by prefork
/// (c:184-187) and quoted it stays one empty word. An UNBRACED `$a[@]` only
/// subscripts with KSH_ARRAYS unset (c:2800-2802), and quoted it keeps one
/// word per element like `"${a[@]}"` (c:2916 `isarr = -1`), empty elements
/// included (c:4387).
mod unset_and_unbraced_splats {
    use super::*;

    #[test]
    fn an_unquoted_splat_of_an_unset_name_is_no_word() {
        assert_parity(r#"f(){ print $#; }; f ${nope[@]}; f ${nope[@]} "${nope[@]}"; f x${nope[@]} ${nope[@]}y"#);
        assert_parity(r#"for w in ${nope[@]}; do print it; done; a=(${nope[@]}); print ${#a}"#);
        assert_parity(r#"setopt shwordsplit; f(){ print $#; }; f ${nope[@]}"#);
    }

    #[test]
    fn unbraced_splat_under_ksh_arrays_is_literal_text() {
        assert_parity(r#"setopt ksharrays; a=(1 2); printf '<%s>' "$a[0]y" "$a[@]y" "$a[*]y" "$a[@]"; print"#);
        assert_parity(r#"setopt ksharrays nonomatch; a=(1 2); printf '<%s>' $a[@] $a[*]; print"#);
        assert_parity(r#"a=(1 2); f(){ setopt localoptions ksharrays nonomatch; printf '<%s>' $a[@]; print; }; f; printf '<%s>' $a[@]; print"#);
    }

    #[test]
    fn quoted_unbraced_splat_keeps_its_elements() {
        assert_parity(r#"a=(1 2); printf '<%s>' x"$a[@]"y x"$a[@]" "$a[@]"y "$a[@]""$a[@]" x"$a[*]"y; print"#);
        assert_parity(r#"a=(1 '' 2); printf '<%s>' x"$a[@]"y; a=('' 1 ''); printf '<%s>' x"$a[@]"y "$a[@]"y; print"#);
        assert_parity(r#"e=(); printf '<%s>' x"$e[@]"y "$e[@]"y; set -- p q; printf '<%s>' x"$argv[@]"y; print"#);
    }
}

/// An associative array's values come out in the table's visit order
/// (Src/hashtable.c scanhashtable) for every whole-array read, including the
/// `"${h[@]}"` / `${h[@]}` / `"${h[*]}"` splats, not in insertion order.
#[test]
fn assoc_value_splats_follow_hash_order() {
    assert_parity(r#"typeset -A h=(b 2 a 1 c 3 foo x bar y zz q); print -- "${h[@]}"; print -- ${h[@]}; print -- "${h[*]}"; for v in "${h[@]}"; do printf '%s ' $v; done; print"#);
}

/// Word lists past 255 words: the VM's call ops count arguments in a u8, so
/// a longer list wrapped mod 256 (`for` over 300 words ran 44 times).
mod word_lists_past_255_words {
    use super::*;

    fn words(n: usize) -> String {
        (1..=n).map(|i| format!("w{i}")).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn for_and_select_lists() {
        for n in [255, 256, 300] {
            let w = words(n);
            assert_parity(&format!("n=0; for k in {w}; do ((n++)); done; print $n $k"));
            assert_parity(&format!(r#"select x in {w}; do print -- "[$x]"; break; done <<< {n} 2>/dev/null"#));
        }
    }

    #[test]
    fn precommands_dynamic_words_and_anonymous_functions() {
        let w = words(300);
        assert_parity(&format!("builtin print {w} | wc -w; c=print; $c {w} | wc -w"));
        assert_parity(&format!("builtin print {w} 2>/dev/null | wc -w; f(){{ print $#; }}; c=f; $c {w}"));
        assert_parity(&format!("() {{ print $# $1 ${{@[-1]}}; }} {w}"));
    }

    #[test]
    fn subscripted_array_assignment() {
        let w = words(300);
        assert_parity(&format!("a=(1 2 3); a[2,2]=({w}); print ${{#a}} $a[1] $a[2] $a[-1]; a[2]+=({w}); print ${{#a}}"));
        assert_parity(&format!("set -- x y; 2=({w}); print $# $2"));
    }
}

/// `=~` hands its pattern to regcomp(3), which decides what is a valid ERE and
/// whose regerror(3) text is the diagnostic (Src/Modules/regex.c:78-81). A `)`
/// with no open group is an ordinary character.
#[test]
fn regex_compile_follows_the_c_library() {
    assert_parity(
        r#"for p in '[' 'a{2,1}' '(?i)AB' '(' '*a' 'a**' '\' '[[:nope:]]' 'a{1' 'x{99999}' '[z-a]' x; do [[ x =~ $p ]]; print rc=$?; done 2>&1 | cut -d: -f3-"#,
    );
    assert_parity(r#"for t in 'a)' 'a)b' 'x)'; do [[ $t =~ "${t%?}\)?" ]] && [[ $t =~ "$t" ]]; print "$? $MATCH"; done; [[ 'x)' =~ '(x))' ]]; print $? $MATCH $match"#);
}
