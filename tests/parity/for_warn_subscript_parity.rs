//! Three divergences that stopped zsh-syntax-highlighting under zshrs, each
//! measured against the reference zsh (stdout, stderr and exit status):
//!
//! * `for 1 2 3; do …; done` — a multi-name `for` with no `in` list bound the
//!   space-joined name `1 2 3` as ONE identifier (`not an identifier: 1 2 3`).
//!   The plugin unpacks `$@` three at a time this way in
//!   `_zsh_highlight_main_add_many_region_highlights`.
//! * `typeset -ga NAME` under WARN_CREATE_GLOBAL warned
//!   `array parameter NAME created globally in function f`; zsh's typeset
//!   declares through createparam / assignaparam without `ASSPM_WARN`, so it
//!   never warns. The plugin runs `typeset -ga ${cache_place}` with the
//!   option on.
//!   the numeric subscript path and failed `bad math expression`.

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

fn zsh_available() -> bool {
    Command::new(crate::oracle::zsh_path())
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// (stdout, stderr, exit)
type Out = (String, String, i32);

fn collect(o: std::process::Output) -> Out {
    (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
        o.status.code().unwrap_or(-1),
    )
}

fn run_zsh(s: &str) -> Out {
    collect(
        Command::new(crate::oracle::zsh_path())
            .args(["-f", "-c", s])
            .output()
            .expect("zsh"),
    )
}

fn run_zshrs(s: &str) -> Out {
    collect(
        Command::new(zshrs_bin())
            .args(["--zsh", "-f", "-c", s])
            .env_remove("ZSHRS_CACHE")
            .output()
            .expect("zshrs"),
    )
}

/// Assert stdout, stderr and exit status match the reference shell. stderr
/// carries the diagnostics these cases are about, so it is compared too.
fn assert_parity(script: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_zsh(script);
    let r = run_zshrs(script);
    assert_eq!(z.0, r.0, "stdout divergence on:\n{script}");
    assert_eq!(z.1, r.1, "stderr divergence on:\n{script}");
    assert_eq!(z.2, r.2, "exit divergence on:\n{script}");
}

mod multi_name_for_without_in {
    use super::*;

    /// Names are bound three at a time from `$@`; the short last group pads
    /// with empty strings (c:Src/loop.c:155-162).
    #[test]
    fn three_names_walk_the_positionals() {
        assert_parity(r#"f(){ for 1 2 3; do print -r -- "[$1][$2][$3]"; done }; f a b c d e"#);
    }

    #[test]
    fn two_names_at_top_level() {
        assert_parity(r#"set -- a b c d; for 1 2; do print -r -- "$1-$2"; done"#);
    }

    /// Ordinary named names with no `in` list take the same path.
    #[test]
    fn named_variables_without_in() {
        assert_parity(r#"f(){ local x y; for x y; do print -r -- "$x:$y"; done }; f 1 2 3 4"#);
    }

    /// The `in` form must be unchanged by the shared bind code.
    #[test]
    fn in_form_still_pairs() {
        assert_parity(r#"for a b in 1 2 3 4 5; do print -r -- "$a-$b"; done"#);
    }
}

mod typeset_global_under_warncreateglobal {
    use super::*;

    #[test]
    fn typeset_ga_by_expanded_name_is_silent() {
        assert_parity(
            r#"f(){ setopt localoptions warncreateglobal; local c=_cache_a; typeset -ga ${c}; : ${(AP)c::="p q"}; print -r -- ${(P)c} }; f"#,
        );
    }

    #[test]
    fn typeset_ga_literal_and_with_value_are_silent() {
        assert_parity(
            r#"f(){ setopt localoptions warncreateglobal; typeset -ga LIT; typeset -ga VAL=(x y); typeset -gaU UNQ; print -r -- ${#VAL} }; f"#,
        );
    }

    /// A real implicit global creation must still warn — the fix is in the
    /// declaration path, not in the option.
    #[test]
    fn plain_array_assignment_still_warns() {
        assert_parity(r#"f(){ setopt localoptions warncreateglobal; PLAIN=(1 2) }; f"#);
    }

    #[test]
    fn plain_scalar_assignment_still_warns() {
        assert_parity(r#"f(){ setopt localoptions warncreateglobal; SC=v }; f"#);
    }
}

mod typeset_assoc_declaration {
    use super::*;

    /// `typeset -gA NAME` declares through createparam: silent, and the type
    /// must come out as an association (not an array).
    #[test]
    fn declaration_is_silent_and_hashed() {
        assert_parity(
            r#"f(){ setopt localoptions warncreateglobal; typeset -gA H1; H1[x]=1; local c=H6; typeset -gA ${c}; print ${(t)H1} ${(t)H6} ${H1[x]} }; f"#,
        );
    }

    #[test]
    fn declaration_with_value_is_silent_and_hashed() {
        assert_parity(
            r#"f(){ setopt localoptions warncreateglobal; typeset -gA H2=(a 1 b 2) H4=([k]=v); print ${(t)H2} ${(kv)H2} ${(kv)H4} }; f"#,
        );
    }

    #[test]
    fn top_level_typeset_a_keeps_the_hash_type() {
        assert_parity(r#"typeset -A H=(a 1); print ${(t)H}; typeset -gA G; G=(x y); print ${(t)G} ${(kv)G}"#);
    }

    /// An implicit global hash assignment must still warn.
    #[test]
    fn plain_hash_assignment_still_warns() {
        assert_parity(r#"f(){ setopt localoptions warncreateglobal; H7=(a 1) }; f"#);
    }
}

mod scalar_subscript_flag_in_math {
    use super::*;

    #[test]
    fn index_of_equals_in_scalar() {
        assert_parity(r#"arg="cmds=("; print $(( arg[(i)=] + 1 ))"#);
    }

    #[test]
    fn last_index_flag() {
        assert_parity(r#"arg=abcabc; print $(( arg[(I)b] ))"#);
    }

    /// A miss yields length + 1, as for arrays.
    #[test]
    fn miss_is_length_plus_one() {
        assert_parity(r#"arg=abc; print $(( arg[(i)x] ))"#);
    }

    /// The exact statement from `_zsh_highlight_main_highlighter_highlight_list`.
    #[test]
    fn plugin_statement_inside_a_function() {
        assert_parity(r#"f(){ local arg="a=b"; local i=$(( arg[(i)=] + 1 )); print $i }; f"#);
    }

    /// Flag with a `b:N:` start offset still resolves.
    #[test]
    fn flag_with_start_offset() {
        assert_parity(r#"arg=abcabc; print $(( arg[(ib:3:)b] ))"#);
    }

    /// `(r)`/`(R)`/`(k)`/`(K)` yield the character at the match, which math
    /// then evaluates; a miss is empty, i.e. 0 (c:Src/params.c:1394-1431).
    #[test]
    fn reverse_subscript_flags_on_a_scalar() {
        assert_parity(
            r#"arg=12345; print $(( arg[(r)3] )) $(( arg[(R)4] )) $(( arg[(r)9] )) $(( arg[(r)3*] + 1 ))"#,
        );
    }

    #[test]
    fn reverse_flags_non_numeric_and_nth_match() {
        assert_parity(
            r#"arg=1a2a3; print $(( arg[(R)[0-9]] )) $(( arg[(r)[0-9]] )) $(( arg[(k)2] )) $(( arg[(K)1] )) $(( arg[(rn:2:)[0-9]] ))"#,
        );
        assert_parity(r#"arg=abc; print $(( arg[(r)b] )) $(( arg[(r)x] ))"#);
    }

    /// Plain numeric subscript on a scalar is unchanged.
    #[test]
    fn numeric_subscript_unchanged() {
        assert_parity(r#"arg=12345; print $(( arg[2,3] ))"#);
    }
}
