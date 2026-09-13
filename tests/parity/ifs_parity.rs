//! IFS / word-splitting parity tests.
//!
//! NOTE: zsh by default does NOT word-split parameter expansions
//! (unlike bash). It splits only via $= flag, the (s/x/) flag, or
//! when SH_WORD_SPLIT option is set.

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

mod default_no_split {
    use super::*;

    /// zsh default: $var doesn't word-split, stays one word.
    #[test]
    fn unquoted_var_doesnt_split_by_default() {
        assert_parity(r#"X="a b c"; f() { echo $#; }; f $X"#);
    }

    /// "$var" never splits.
    #[test]
    fn quoted_var_stays_one_arg() {
        assert_parity(r#"X="a b c"; f() { echo $#; }; f "$X""#);
    }
}

mod equals_split {
    use super::*;

    /// `$=var` forces splitting.
    #[test]
    fn equals_prefix_forces_split() {
        assert_parity(r#"X="a b c"; f() { echo $#; }; f $=X"#);
    }

    /// `$=var` uses $IFS for splitting.
    #[test]
    fn equals_split_uses_ifs() {
        assert_parity(r#"X="a:b:c"; IFS=:; f() { echo $#; }; f $=X"#);
    }

    /// c:Src/subst.c:2567 + c:3913 — `=` splits inside double quotes too, so
    /// a whole-word `"${=…}"` is one word per field, not a joined scalar.
    #[test]
    fn equals_split_inside_double_quotes_keeps_fields() {
        assert_parity(r#"print -l "${=$(print two words)}""#);
        assert_parity(r#"x="two words"; print -l "${=x:-y}" "${=${x}}" "${=x/o/o}""#);
        assert_parity(r#"x="two words"; a=("${=x:-y}"); print $#a; for i in "${(j: :)=x}"; do print -r "<$i>"; done"#);
        assert_parity(r#"x="two words"; print -l "${==x:-y}" "${=x:-y}""""#);
    }
}

mod sh_word_split {
    use super::*;

    /// `setopt SH_WORD_SPLIT` makes $var split like POSIX shells.
    #[test]
    fn shwordsplit_enables_unquoted_split() {
        assert_parity(r#"setopt SH_WORD_SPLIT; X="a b c"; f() { echo $#; }; f $X"#);
    }

    /// Even with SH_WORD_SPLIT, "$var" stays one arg.
    #[test]
    fn shwordsplit_doesnt_affect_quoted_var() {
        assert_parity(r#"setopt SH_WORD_SPLIT; X="a b c"; f() { echo $#; }; f "$X""#);
    }
}

mod custom_ifs {
    use super::*;

    /// IFS=: with `$=X` splits on colon.
    #[test]
    fn ifs_colon_splits_on_colon() {
        assert_parity(r#"IFS=:; X="a:b:c"; f() { echo $#; }; f $=X"#);
    }

    /// IFS empty disables splitting entirely.
    #[test]
    fn ifs_empty_disables_split() {
        assert_parity(r#"IFS=; X="a b c"; f() { echo $#; }; f $=X"#);
    }

    /// c:Src/params.c:3913-3914 + c:4745-4750 — unsetting IFS sets `ifs = NULL`,
    /// and `inittyptab` splits on the default separators again.
    #[test]
    fn unset_ifs_restores_default_splitting() {
        assert_parity(r#"IFS=; unset IFS; s="p q"; print -rl -- ${=s}; set -- "p q"; print -rl -- $=1; setopt shwordsplit; print -rl -- $s"#);
        assert_parity(r#"IFS=:; unset IFS; s="p q"; print -rl -- ${=s}; set -- "p q"; print -rl -- $=1; setopt shwordsplit; print -rl -- $s"#);
        assert_parity(r#"IFS=:; unset IFS; read x y <<< "p q"; print -r "[$x][$y]"; IFS=,; s="p,q r"; print -rl -- ${=s}"#);
        assert_parity(r#"unset IFS; s="p:q r"; print -rl -- ${=s}; IFS=:; f() { local IFS; unset IFS; print -rl -- ${=s}; }; f; print -rl -- ${=s}"#);
    }
}

mod for_loop_iteration {
    use super::*;

    /// `for x in $var` with default IFS — zsh: one iter (no split).
    #[test]
    fn for_in_unquoted_var_no_split_zsh_default() {
        assert_parity(r#"X="a b c"; n=0; for x in $X; do n=$((n+1)); done; echo $n"#);
    }

    /// With SH_WORD_SPLIT, three iters.
    #[test]
    fn for_in_unquoted_var_splits_with_shwordsplit() {
        assert_parity(
            r#"setopt SH_WORD_SPLIT; X="a b c"; n=0; for x in $X; do n=$((n+1)); done; echo $n"#,
        );
    }

    /// for-in with $=X forces split → three iters.
    #[test]
    fn for_in_equals_forces_split() {
        assert_parity(r#"X="a b c"; n=0; for x in $=X; do n=$((n+1)); done; echo $n"#);
    }

    /// For-in with literal list — clear three iters.
    #[test]
    fn for_in_literal_list_three_iters() {
        assert_parity(r#"n=0; for x in a b c; do n=$((n+1)); done; echo $n"#);
    }

    /// A bare `$@` / `$*` word list gets the word's own c:184-187 removal
    /// once; the `nulstring` fields of a SH_WORD_SPLIT re-split survive it.
    #[test]
    fn for_in_positional_splat_keeps_nulstring_fields() {
        assert_parity(r#"setopt shwordsplit; IFS=:; set -- a::b "" :c:; for i in $@; do print "<$i>"; done; for i in $* z; do print "<$i>"; done"#);
        assert_parity(r#"set -- a "" b; for i in $@ $*; do print "<$i>"; done; setopt shwordsplit; set -- "" x ""; for i in $@; do print "<$i>"; done"#);
    }

    /// c:Src/loop.c execfor — `for i;` iterates the positional parameters
    /// verbatim, never split.
    #[test]
    fn for_without_in_iterates_positionals_verbatim() {
        assert_parity(r#"setopt shwordsplit; set -- "a b" c; for i; do print "<$i>"; done; for i do print "<$i>"; done"#);
        assert_parity(r#"setopt shwordsplit; IFS=:; set -- a::b "" :c:; for i; do print "<$i>"; done; unsetopt shwordsplit; for i; do print "<$i>"; done"#);
    }
}

mod cmdsubst_split {
    use super::*;

    /// Unquoted $(...) — zsh DOES split (per zsh docs, $(...) always splits).
    #[test]
    fn cmdsubst_splits_in_zsh() {
        assert_parity(r#"f() { echo $#; }; f $(echo a b c)"#);
    }

    /// "$( )" never splits.
    #[test]
    fn quoted_cmdsubst_no_split() {
        assert_parity(r#"f() { echo $#; }; f "$(echo a b c)""#);
    }

    /// $(...) with IFS=: and colon-separated output.
    #[test]
    fn cmdsubst_with_ifs_colon() {
        assert_parity(r#"IFS=:; f() { echo $#; }; f $(echo a:b:c)"#);
    }

    /// An IFS the command substitution's body changes does not split the
    /// parent's words afterwards.
    #[test]
    fn body_ifs_change_does_not_leak_into_parent_splitting() {
        assert_parity(r#"IFS=:; print -r $(IFS=,; :); s="p,q"; print -rl -- ${=s}; s="p:q"; print -rl -- ${=s}"#);
        assert_parity(r#"IFS=:; print -r $(IFS=,); s="p:q"; print -rl -- ${=s}"#);
    }
}

mod ifs_in_read {
    use super::*;

    /// `read` uses IFS to split input.
    #[test]
    fn read_splits_on_ifs_colon() {
        assert_parity(r#"IFS=: read X Y Z <<< 'one:two:three'; echo "[$X][$Y][$Z]""#);
    }

    /// `IFS=` read keeps whole line in first var.
    #[test]
    fn read_with_empty_ifs_no_split() {
        assert_parity(r#"IFS= read X Y <<< 'one two three'; echo "[$X][$Y]""#);
    }
}

mod ifs_multi_char {
    use super::*;

    /// Multi-char IFS — each char in IFS is a splitter.
    #[test]
    fn ifs_multi_char_each_splits() {
        assert_parity(r#"IFS=':|'; f() { echo $#; }; f $=$"$(echo 'a:b|c:d')""#);
    }
}

/// c:`Src/subst.c:3912-3939` — the join-then-split block, for the `${a[*]}` /
/// `${a[@]}` / `${=a[…]}` / `${==a[…]}` splices that the compiler serves from
/// its own fast paths rather than through `paramsubst`. docs/BUGS.md #1132.
///
/// The block is GATED on `spbreak`, and inside it the ANSWER turns on two
/// things and only two:
///
/// * `nojoin` (c:1819 / :2569) — `!(ifs && *ifs)`, so 1 for an IFS that is
///   unset OR empty. c:3030-3032 then forces `isarr = -1`, which is why
///   `[@]` and `[*]` behave IDENTICALLY here and every case below pairs them.
/// * whether IFS is UNSET or merely EMPTY — c:3919's second join arm is
///   `(!ifs && isarr < 0)`, which an empty-but-set IFS fails.
///
/// So a non-empty IFS joins on `IFS[0]` and splits; an UNSET IFS joins on
/// `sepjoin`'s default `" "` and splits; an EMPTY IFS does NEITHER and the
/// original elements survive.
///
/// Every expectation is `zsh -f`, and each `assert_parity` below was verified
/// RED against a pinned build of the pre-fix tree (132 divergent cells in a
/// 1260-cell IFS x SH_WORD_SPLIT x shape x spelling x context sweep).
mod splice_join_split_c3912 {
    use super::*;

    /// c:3916 — SH_WORD_SPLIT with a non-empty IFS joins the splice on
    /// `IFS[0]` and re-splits, so an element that CONTAINS the separator
    /// comes apart. `${a[@]}` had no SH_WORD_SPLIT arm at all.
    #[test]
    fn shwordsplit_joins_and_splits_the_splice() {
        assert_parity(
            r#"setopt shwordsplit; a=('p:q' r); IFS=:; print -rl -- ${a[@]}"#,
        );
        assert_parity(
            r#"setopt shwordsplit; a=('p:q' r); IFS=:; print -rl -- ${a[*]}"#,
        );
        assert_parity(
            r#"setopt shwordsplit; a=('p:q' r); IFS=:; f(){print -r -- $#}; f ${a[@]}"#,
        );
    }

    /// c:3919 `(!ifs && isarr < 0)` — an UNSET IFS still joins, on
    /// `sepjoin`'s default `" "` (c:Src/utils.c:3941-3945), then splits on
    /// the default IFS.
    #[test]
    fn shwordsplit_unset_ifs_joins_on_space_then_splits() {
        assert_parity(
            r#"setopt shwordsplit; a=('p q' r); unset IFS; print -rl -- ${a[@]}"#,
        );
        assert_parity(
            r#"setopt shwordsplit; a=('p q' r); unset IFS; print -rl -- ${a[*]}"#,
        );
    }

    /// c:3916/:3919 both decline for `nojoin == 1` with IFS set to the EMPTY
    /// string, so c:3931's `!isarr` keeps the split off too — the elements
    /// survive untouched. The handler used to join them on `""` and then have
    /// no separator left to split on, collapsing the array into one word.
    #[test]
    fn shwordsplit_empty_ifs_leaves_the_elements_alone() {
        assert_parity(r#"setopt shwordsplit; a=(x y); IFS=; print -rl -- ${a[*]}"#);
        assert_parity(r#"setopt shwordsplit; a=(x y); IFS=; print -rl -- ${a[@]}"#);
        assert_parity(
            r#"setopt shwordsplit; a=(x y); IFS=; f(){print -r -- $#}; f ${a[*]}"#,
        );
    }

    /// c:Src/utils.c:3732 / :3752 — `spacesplit` marks an empty field
    /// delimited by IFS-NON-whitespace with `nulstring`
    /// (c:Src/subst.c:36 `{Nularg,'\0'}`), which prefork's `uremnode` (c:186)
    /// KEEPS and `remnulargs` turns into `""`. A field left by a skipped run
    /// of IFS-WHITESPACE is a real `""` and c:186 deletes it. A naive
    /// `split().filter(non-empty)` cannot tell them apart and dropped the
    /// middle word.
    #[test]
    fn nulstring_empty_fields_survive_a_non_whitespace_ifs() {
        assert_parity(r#"setopt shwordsplit; a=(x '' y); IFS=:; print -rl -- ${a[@]}"#);
        assert_parity(r#"setopt shwordsplit; a=(x '' y); IFS=:; print -rl -- ${a[*]}"#);
        assert_parity(
            r#"setopt shwordsplit; a=(x '' y); IFS=:; f(){print -r -- $#}; f ${a[@]}"#,
        );
        // The same array under a WHITESPACE IFS keeps only two words.
        assert_parity(r#"setopt shwordsplit; a=(x '' y); IFS=' '; print -rl -- ${a[@]}"#);
        // Leading / trailing non-whitespace separators each keep their field.
        assert_parity(r#"setopt shwordsplit; a=('' x ''); IFS=:; f(){print -r -- $#}; f ${a[@]}"#);
    }

    /// c:2563 — `${==NAME[…]}` clears `spbreak`, so c:3912's gate never opens
    /// however SH_WORD_SPLIT is set and the elements survive whole.
    #[test]
    fn double_equals_suppresses_the_split_under_shwordsplit() {
        assert_parity(r#"setopt shwordsplit; a=('p:q' r); IFS=:; print -rl -- ${==a[*]}"#);
        assert_parity(r#"setopt shwordsplit; a=(x '' y); IFS=:; print -rl -- ${==a[*]}"#);
        assert_parity(
            r#"setopt shwordsplit; a=('p:q' r); IFS=:; f(){print -r -- $#}; f ${==a[*]}"#,
        );
    }

    /// c:2567 — `${=NAME[…]}` forces `spbreak = 2`, which opens the block
    /// without SH_WORD_SPLIT. It runs the SAME `nojoin` arms, so an empty IFS
    /// still leaves the elements alone and an unset one still joins on `" "`.
    #[test]
    fn equals_flag_runs_the_same_nojoin_arms() {
        assert_parity(r#"a=('p q' r); IFS=:; print -rl -- ${=a[@]}"#);
        assert_parity(r#"a=('p q' r); IFS=:; print -rl -- ${=a[*]}"#);
        assert_parity(r#"a=(x y); IFS=; print -rl -- ${=a[*]}"#);
        assert_parity(r#"a=(x y); IFS=; print -rl -- ${=a[@]}"#);
        assert_parity(r#"a=('p q' r); unset IFS; print -rl -- ${=a[@]}"#);
        assert_parity(r#"a=(x '' y); IFS=:; f(){print -r -- $#}; f ${=a[@]}"#);
    }

    /// c:4226 `if (isarr && ssub) { val = sepjoin(aval, NULL, 1); }` — a
    /// PREFORK_SINGLE context joins and STOPS, `${=…}` or not. The `[@]`
    /// spelling under `=` used to reach the assignment as an array and
    /// stringify with a hardcoded space.
    #[test]
    fn scalar_assignment_joins_the_splice_on_ifs0() {
        assert_parity(r#"a=(x y); IFS=:; v=${=a[@]}; print -r -- "[$v]""#);
        assert_parity(r#"a=('p:q' r); IFS=:; v=${=a[@]}; print -r -- "[$v]""#);
        assert_parity(r#"a=(x y); IFS=:; v=${a[@]}; print -r -- "[$v]""#);
    }

    /// c:Src/params.c:428-430 — `IPDEF9("*", &pparams)` and
    /// `IPDEF9("argv", &pparams)` are ONE parameter, and c:Src/params.c:2251
    /// `isvarat = (t[0] == '@' && !t[1])` is the only shape discriminator, so
    /// `${argv:-…}` IS `${*:-…}`. The default-family rebuild kept the `argv`
    /// spelling, whose "is it set" probe tests only `@`/`*` — the expansion
    /// looked UNSET and took the default.
    #[test]
    fn argv_resolves_to_the_positional_list_in_the_default_family() {
        assert_parity(r#"set -- x y; print -r -- ${argv:-nope}"#);
        assert_parity(r#"set -- x y; print -r -- ${argv:+yes}"#);
        assert_parity(r#"set -- x y; print -r -- ${argv:?msg}"#);
        assert_parity(r#"set -- x y; print -r -- ${argv-nope}"#);
        assert_parity(r#"set -- x y; print -r -- ${argv+yes}"#);
        // The ASSIGNING ops keep the literal name: zsh rejects `${*=…}`
        // ("not an identifier: *") but accepts `${argv=…}`.
        assert_parity(r#"set -- x y; print -r -- ${argv=dflt}"#);
        assert_parity(r#"set -- x y; print -r -- ${argv:=dflt}"#);
        assert_parity(r#"set -- x y; print -r -- ${+argv}"#);
    }

    /// c:2916-2917 sets `isarr` non-zero for ANY array-shaped read, and
    /// c:3030-3032 then makes the spellings indistinguishable inside the
    /// block, so the BARE `$NAME` / `${NAME}` read of an array takes exactly
    /// the same c:3912-3939 treatment as a `[@]`/`[*]` splice. Only the
    /// splice fast paths had it; `BUILTIN_GET_VAR`'s array arm returned the
    /// element vector whatever SH_WORD_SPLIT said.
    #[test]
    fn bare_array_read_takes_the_same_block_as_the_splice() {
        assert_parity(r#"setopt shwordsplit; a=('p:q' r); IFS=:; print -rl -- $a"#);
        assert_parity(r#"setopt shwordsplit; a=('p:q' r); IFS=:; print -rl -- ${a}"#);
        assert_parity(r#"setopt shwordsplit; a=('p q' r); unset IFS; print -rl -- ${a}"#);
        assert_parity(
            r#"setopt shwordsplit; a=('p:q' r); IFS=:; f(){print -r -- $#}; f $a"#,
        );
        // c:Src/utils.c:3732 `nulstring` — an empty field an IFS-NON-space
        // separator delimits survives c:186.
        assert_parity(r#"setopt shwordsplit; a=(x '' y); IFS=:; print -rl -- $a"#);
        // IFS set but EMPTY: neither join arm fires (c:3919 wants `!ifs`, not
        // an empty one), so c:3931's `!isarr` keeps the split off too.
        assert_parity(r#"setopt shwordsplit; a=(x y); IFS=; print -rl -- $a"#);
        // Without the option the block never opens at all.
        assert_parity(r#"a=('p:q' r); IFS=:; print -rl -- $a"#);
        assert_parity(r#"a=('p:q' r); IFS=:; print -rl -- ${a}"#);
    }

    /// The BARE-name `${=NAME}` / `${==NAME}` / `$=NAME` spelling reached the
    /// split through `BUILTIN_GET_VAR_DQ`, whose array arm joins
    /// UNCONDITIONALLY. That is c:4226's `ssub` join, not c:3916/:3919, which
    /// fire only for particular `nojoin` / IFS combinations — and c:2562's
    /// `${==…}` shuts the whole block, so its elements must survive whole.
    #[test]
    fn bare_forced_split_flag_runs_the_c3912_block() {
        // c:2562 — `${==NAME}` clears `spbreak`: no join, no split.
        assert_parity(r#"a=('p:q' r); IFS=:; print -rl -- ${==a}"#);
        assert_parity(r#"a=(x '' y); IFS=:; print -rl -- ${==a}"#);
        assert_parity(r#"setopt shwordsplit; a=(x y); IFS=:; print -rl -- ${==a}"#);
        assert_parity(r#"a=('p:q' r); IFS=:; f(){print -r -- $#}; f ${==a}"#);
        // c:2569 — `${=NAME}` recomputes `nojoin` as `!(ifs && *ifs)`, so a
        // set-but-EMPTY IFS leaves the elements alone.
        assert_parity(r#"a=(x y); IFS=; print -rl -- ${=a}"#);
        assert_parity(r#"a=(x '' y); IFS=; print -rl -- ${=a}"#);
        assert_parity(r#"a=('p:q' r); IFS=; f(){print -r -- $#}; f ${=a}"#);
        // c:3033 — the QUOTED form still takes the c:3033 join first when
        // `isarr > 0`, and splits the joined text after.
        assert_parity(r#"a=(x y); IFS=:; print -rl -- "${=a}""#);
        assert_parity(r#"a=(x y); IFS=; print -rl -- "${=a}""#);
        assert_parity(r#"a=(x y); IFS=:; print -rl -- "${==a}""#);
        // c:3899-3906 — a ONE-element array is a scalar for the block, so the
        // split runs over its text.
        assert_parity(r#"a=('p q'); IFS=; print -rl -- ${=a}"#);
        assert_parity(r#"a=(only); IFS=:; print -rl -- ${=a}"#);
        // A genuine SCALAR keeps `isarr == 0` and goes straight to c:3931.
        assert_parity(r#"v='a b'; IFS=; print -rl -- ${=v}"#);
        assert_parity(r#"v='a:b'; IFS=:; print -rl -- ${=v}"#);
        assert_parity(r#"v=''; r=(${==v}); print -r -- $#r"#);
        // The unbraced `$=@` reads the positional list through the same path.
        assert_parity(r#"set -- 'a b' 'c d'; print -rl -- $=@"#);
        assert_parity(r#"set -- 'a b' 'c d'; IFS=:; print -rl -- $=@"#);
        assert_parity(r#"set -- 'a b' 'c d'; IFS=; print -rl -- $=@"#);
        // c:3913 `force_split = !ssub && …` — a PREFORK_SINGLE context joins
        // at c:4226 and never splits.
        assert_parity(r#"a=(x y); IFS=:; v=${=a}; print -r -- "[$v]""#);
    }

    /// A RANGE subscript is array-shaped (c:2916-2917), so c:3912's block
    /// applies to `${NAME[lo,hi]}` too. `BUILTIN_ARRAY_INDEX` had no
    /// SH_WORD_SPLIT arm, so the range came back as untouched elements.
    #[test]
    fn range_subscript_takes_the_block_under_shwordsplit() {
        assert_parity(r#"setopt shwordsplit; a=('p:q' r); IFS=:; print -rl -- ${a[1,2]}"#);
        assert_parity(r#"setopt shwordsplit; a=('p q' r); unset IFS; print -rl -- ${a[1,2]}"#);
        assert_parity(
            r#"setopt shwordsplit; a=('p:q' r); IFS=:; f(){print -r -- $#}; f ${a[1,2]}"#,
        );
        // IFS set but EMPTY — elements survive.
        assert_parity(r#"setopt shwordsplit; a=(x y); IFS=; print -rl -- ${a[1,2]}"#);
        // A SINGLE index is a scalar (`isarr == 0`) and is untouched by the
        // block; so is the quoted range (c:1707 `!qt`).
        assert_parity(r#"setopt shwordsplit; a=('p:q' r); IFS=:; print -rl -- ${a[1]}"#);
        assert_parity(r#"setopt shwordsplit; a=('p:q' r); IFS=:; print -rl -- "${a[1,2]}""#);
        // Without the option the block never opens.
        assert_parity(r#"a=('p:q' r); IFS=:; print -rl -- ${a[1,2]}"#);
    }
}

/// c:Src/subst.c:4387 / :4404 / :4426 — `if (qt && !*y && isarr != 2)
/// y = dupstring(nulstring)`. A QUOTED splat's empty element is stored as
/// c:36 `nulstring`, which c:183 finds NON-empty, so c:186's `uremnode`
/// never deletes it and `remnulargs` (c:170) restores `""` afterwards.
///
/// The rule stops at c:4261 `if ((!aval[0] || !aval[1]) && !plan9)`: the
/// empty and one-element cases are claimed FIRST and have their `Dnull`
/// markers stripped (c:4272-4274), so those nodes really are empty and DO
/// get removed. Element count is therefore a run-time input to the rule.
mod quoted_splat_keeps_nulstring_c4387 {
    use super::*;

    /// The word the splat is CONCATENATED into is where it shows: with no
    /// affixes the empty slot already survived, but `PRE"${a[@]}"POST` sent
    /// the assembled word through an unconditional end-of-word empty drop.
    #[test]
    fn quoted_splat_with_affixes_keeps_its_empty_element() {
        assert_parity(r#"a=(x '' y); for w in PRE"${a[@]}"POST; do print -r -- "<$w>"; done"#);
        assert_parity(r#"a=(x '' y); for w in PRE"${a[@]}"; do print -r -- "<$w>"; done"#);
        assert_parity(r#"a=(x '' y); for w in "${a[@]}"POST; do print -r -- "<$w>"; done"#);
        assert_parity(r#"set -- x '' y; for w in PRE"$@"POST; do print -r -- "<$w>"; done"#);
        assert_parity(r#"a=(x '' y); f(){ print -r -- $# }; f PRE"${a[@]}"POST"#);
        assert_parity(r#"set -- x '' y; f(){ print -r -- $# }; f PRE"$@"POST"#);
    }

    /// c:4261's cut-off — below two elements the emit block strips the quotes
    /// and the node is a true empty, so the drop still applies.
    #[test]
    fn one_and_zero_element_splats_still_drop() {
        assert_parity(r#"a=(''); f(){ print -r -- $# }; f PRE"${a[@]}"POST"#);
        assert_parity(r#"a=(); f(){ print -r -- $# }; f PRE"${a[@]}"POST"#);
        assert_parity(r#"a=(); b=(); f(){ print -r -- $# }; f "${a[@]}""${b[@]}""#);
        assert_parity(r#"a=(); print -r -- "[${a[@]}]""#);
    }

    /// An UNQUOTED splat has `qt == 0`, so its empties are genuine empty
    /// nodes and c:186 removes them — before and after this rule.
    #[test]
    fn unquoted_splat_still_drops_its_empties() {
        assert_parity(r#"a=(x '' y); f(){ print -r -- $# }; f PRE${a[@]}POST"#);
        assert_parity(r#"a=(x '' y); f(){ print -r -- $# }; f ${a[@]}"#);
        assert_parity(r#"set -- x '' y; f(){ print -r -- $# }; f $@"#);
        assert_parity(r#"a=(y '' x); for i in $a; do print -r -- "<$i>"; done"#);
    }

    /// The bit is word-scoped: a whole-word `"${a[@]}"` emits no end-of-word
    /// drop, so a following UNQUOTED word must not inherit its retention.
    #[test]
    fn retention_does_not_leak_into_the_next_word() {
        assert_parity(r#"a=(x '' y); b=(p '' q); f(){ print -r -- $# }; f "${a[@]}" X${b[@]}Y"#);
        assert_parity(r#"a=(x '' y); b=(p '' q); f(){ print -r -- $# }; f "${a[@]}" $b"#);
        assert_parity(r#"a=(x '' y); f(){ print -r -- $# }; f "${a[@]}" "${a[@]}""#);
    }

    /// A SPLIT field's empty is `nulstring` for a different reason (c:3919
    /// `sepsplit`) and must keep surviving; `"${a[*]}"` is one joined word.
    #[test]
    fn split_fields_and_star_join_are_unaffected() {
        assert_parity(
            r#"setopt shwordsplit; IFS=:; s='a::b'; for w in P${s}S; do print -r -- "<$w>"; done"#,
        );
        assert_parity(r#"setopt shwordsplit; a=(x '' y); IFS=:; f(){ print -r -- $# }; f ${a[*]}"#);
        assert_parity(r#"a=(x '' y); f(){ print -r -- $# }; f PRE"${a[*]}"POST"#);
        assert_parity(r#"a=(x '' y); f(){ print -r -- $# }; f "${(@)a}""#);
        assert_parity(r#"a=(x y ''); f(){ print -r -- $# }; f ${a}POST"#);
    }
}

/// c:Src/subst.c:318-324 — SH_WORD_SPLIT reaches paramsubst as the
/// PREFORK_SHWORDSPLIT bit stringsubst adds at its call site, so every
/// compiled `${…}` modifier shape has to split its RESULT exactly like a
/// plain `$s`, while a scalar-assignment value (PREFORK_SINGLE, c:1761)
/// keeps its separators.
mod sh_word_split_modifier_results {
    use super::*;

    #[test]
    fn modifier_results_split_on_ifs() {
        assert_parity(r#"setopt shwordsplit; s="a o b"; print -l ${s/o/x}"#);
        assert_parity(r#"setopt shwordsplit; s="a o b"; print -l ${s:-x} ${s:+$s}"#);
        assert_parity(r#"setopt shwordsplit; s="a o b"; print -l ${s#a} ${s:1} ${(U)s}"#);
        assert_parity(r#"setopt shwordsplit; s="another poxy boring string"; print -l ${${s}/o/ }"#);
        assert_parity(r#"setopt shwordsplit; t() { print $#: "$@" }; t ${:- foo bar }"#);
        assert_parity(r#"setopt shwordsplit; IFS=:; s="a:o:b"; print -l pre${s/o/x}post"#);
        assert_parity(r#"setopt shwordsplit; a=("x y" z); print -l ${a/x/q}"#);
    }

    #[test]
    fn scalar_assignment_values_stay_whole() {
        assert_parity(
            r#"setopt shwordsplit; s="a  b"; x=${s/a/c}; local y=${s/a/c}; typeset z=${s:-q}; export w=${s#a}; print -r "[$x][$y][$z][$w]"; a=(${s/a/c}); print $#a"#,
        );
        assert_parity(r#"setopt shwordsplit; s="a o b"; print -l "${s/o/x}"; [[ ${s/o/x} == "a o b" ]] || print ok"#);
    }

    /// c:Src/utils.c:3730-3760 spacesplit + c:Src/subst.c:184-187 — the empty
    /// fields a leading/trailing IFS-whitespace run leaves take the word's
    /// affixes before prefork deletes the empty nodes; IFS-non-whitespace
    /// empties (`nulstring`) always survive.
    #[test]
    fn whitespace_edge_fields_take_the_affixes() {
        assert_parity(r#"setopt shwordsplit; s=" foo bar "; print -rl -- x${s}y x$s ${s}y $s; a=(x${s}y); print $#a"#);
        assert_parity(r#"setopt shwordsplit; s=" foo bar "; e=; print -rl -- $e$s "x"$s; for w in x$s; do print -r "[$w]"; done"#);
        assert_parity(r#"setopt shwordsplit; IFS=:; s=":a::b:"; print -rl -- x${s}y x$s $s"#);
        assert_parity(r#"setopt shwordsplit; s="   "; print -rl -- x${s}y; print -rl -- x$s | wc -l"#);
    }
}

/// c:Src/loop.c:98 `execsubst(args)` → c:Src/exec.c:2744-2746 prefork, the
/// same word expansion every other argument gets: a bare `$foo` in a `for`
/// list keeps an IFS-non-whitespace empty field (`nulstring`), honours
/// KSH_ARRAYS and GLOB_SUBST, and drops only truly empty words.
mod for_list_bare_parameter {
    use super::*;

    #[test]
    fn nulstring_field_is_an_iteration() {
        assert_parity(r#"setopt shwordsplit; IFS=:; foo=1::2; for w in $foo; do print -r "[$w]"; done"#);
        assert_parity(r#"setopt shwordsplit; foo=" a  b "; for w in $foo; do print -r "[$w]"; done"#);
        assert_parity(r#"a=(y '' x); for i in $a; do print -r "[$i]"; done; e=; for w in $e; do print -r "[$w]"; done; print done"#);
    }

    #[test]
    fn options_apply_like_any_other_word() {
        assert_parity(r#"setopt ksharrays; a=(p q); for w in $a; do print -r "[$w]"; done"#);
        assert_parity(r#"cd "$(mktemp -d)" && touch zq1 zq2 && setopt globsubst && p='zq*' && for w in $p; do print -r "[$w]"; done"#);
        assert_parity(r#"set -- "a b" c; for w in $argv; do print -r "[$w]"; done; typeset -A h; h=(k v); for w in $h; do print -r "[$w]"; done"#);
    }
}

/// c:Src/lex.c dquote_parse — every `$` inside `"…"` is the Qstring token, so
/// a substitution NESTED in a quoted `${(@)…}` still expands with `qt`
/// (c:Src/subst.c:283) and c:1707's `spbreak` stays off: SH_WORD_SPLIT does
/// not split the inner value.
mod nested_substitution_in_quoted_at_flag {
    use super::*;

    #[test]
    fn inner_value_is_not_word_split() {
        assert_parity(r#"setopt shwordsplit; s=" foo bar "; print -rl -- "${(@)${:-x${s}y}}""#);
        assert_parity(r#"setopt shwordsplit; s=" foo bar "; print -rl -- "${(@)${s}}" "${(@)${s/o/o}}" "${(@)${:-$s}}""#);
    }

    #[test]
    fn explicit_splits_still_apply() {
        assert_parity(r#"s=" foo bar "; print -rl -- "${(@)${=s}}""#);
        assert_parity(r#"a=(p 'q r'); print -rl -- "${(@)${a}}" "${(@)${(s: :)${:-a b}}}""#);
        assert_parity(r#"setopt shwordsplit; s=" foo bar "; print -rl -- ${(@)${:-x${s}y}}"#);
    }
}

/// c:Src/subst.c:4237-4245 — a split default / alternate word that begins or
/// ends with IFS whitespace splits the surrounding text off at that edge
/// (multsub's MULTSUB_WS_AT_START / _AT_END).
mod default_word_whitespace_edges {
    use super::*;

    #[test]
    fn affixes_become_their_own_words() {
        assert_parity(r#"setopt shwordsplit; t() { print -r $#: "$@" }; t x${:- foo bar }y x${:- foo bar } ${:- foo bar }y"#);
        assert_parity(r#"setopt shwordsplit; t() { print -r $#: "$@" }; t x${:- foo }y x${:- foo}y x${:-foo }y x${:-   }y"#);
        assert_parity(r#"setopt shwordsplit; t() { print -r $#: "$@" }; x=1; t x${x:+ foo bar }y x${u- foo bar }y x${:- foo "bar" }y"#);
        assert_parity(r#"setopt shwordsplit; t() { print -r $#: "$@" }; t x${:- foo bar }y\z; a=(x${:- foo bar }y); t $a"#);
    }

    #[test]
    fn unsplit_contexts_are_unchanged() {
        assert_parity(r#"setopt shwordsplit; t() { print -r $#: "$@" }; t ${:- foo bar } "x${:- foo bar }y"; v=${:- foo bar }; t "[$v]""#);
        assert_parity(r#"t() { print -r $#: "$@" }; t x${:- foo bar }y"#);
    }

    /// c:Src/subst.c:566-625 — the operand's own prefork prunes the empty
    /// node a leading non-whitespace separator leaves, before the affixes.
    #[test]
    fn non_whitespace_separator_edges() {
        assert_parity(r#"setopt shwordsplit; IFS=:; print -rl -- x${:-:a::b:}y x${:-:a::b:} ${:-:a::b:}y x${:-:a}y x${:-a:}y"#);
        assert_parity(r#"setopt shwordsplit; IFS=:; x=1; print -rl -- x${u-:a::b:}y x${x:+:a::b:}y; a=(x${:-:a::b:}y); print $#a"#);
        assert_parity(r#"IFS=:; print -rl -- x${=:-:a::b:}y; a=("" x ""); print -rl -- p${:-${a[@]}}q"#);
        assert_parity(r#"setopt shwordsplit; IFS=": "; print -rl -- x${:-:a}y x${:- :a: }y; IFS=:; print -rl -- x${u:=:a::b:}y"#);
    }
}

/// c:Src/subst.c:4366-4437 — the fields of a `${=…}` split take the word's
/// text first/last like any array; only RC_EXPAND_PARAM cross-products them.
mod equals_split_fields_splice_with_affixes {
    use super::*;

    #[test]
    fn operator_forms_splice() {
        assert_parity(r#"x="a b"; print -rl -- x${=x:-q}y x${=:-a b}y x${(j:-:)=x:-q}y"#);
        assert_parity(r#"t() { print -r $#: "$@" }; t x${=:- foo bar }y"#);
        assert_parity(r#"x="a b"; print -rl -- "x${=x:-q}y" "${=x:-q}"; a=(x${=x:-q}y); print $#a"#);
    }

    #[test]
    fn rc_expand_param_still_cross_products() {
        assert_parity(r#"setopt rcexpandparam; print -rl -- x${=:-a b}y"#);
        assert_parity(r#"s="a b"; print -rl -- x${=s}y x${=s} "split ${=s} wise""#);
    }
}

/// c:Src/utils.c:3730-3760 + c:Src/subst.c:36 / :183-186 — an empty field
/// between two IFS-non-whitespace separators is `nulstring` and survives the
/// word's empty-node removal even when the split fields take the word's text;
/// only the IFS-whitespace edges are deleted when nothing attaches to them.
mod nulstring_fields_survive_affixes {
    use super::*;

    #[test]
    fn equals_split_keeps_middle_empty_field() {
        assert_parity(r#"IFS=:; s=":a::b:"; print -rl -- x${=s}y; print -rl -- x${=s}"#);
        assert_parity(r#"IFS=:; s="a::b"; print -rl -- x${=s}y ${=s}y; a=(x${=s}y); print $#a"#);
        assert_parity(r#"IFS=": "; s=" a : :b"; print -rl -- x${=s}y"#);
    }

    /// c:Src/subst.c:3911-3934 — under SH_WORD_SPLIT a splat read's elements
    /// are joined on IFS[0] and re-split, or kept when `nojoin` leaves them.
    #[test]
    fn shwordsplit_splat_read_rejoins_its_elements() {
        assert_parity(r#"setopt shwordsplit; IFS=:; s=a::b; print -rl -- x${${=s}[@]}y ${${=s}[@]}"#);
        assert_parity(r#"setopt shwordsplit; IFS=:; a=(a "" b); print -rl -- ${${a}[@]} ${(@)a} ${${a}[*]} ${(@)${a}}; f() { print $#; }; f ${a[@]} ${(@)a}"#);
        assert_parity(r#"setopt shwordsplit; IFS=; a=(x y); print -rl -- ${${a}[@]} ${(@)a}"#);
        assert_parity(r#"setopt shwordsplit; unset IFS; a=("p q" r); print -rl -- ${${a}[@]} ${(@)a}"#);
        assert_parity(r#"setopt shwordsplit; IFS=:; s=":a::b:"; print -rl -- ${(@)${=s}} "${(@)${=s}}"; a=(a "" b); v=${(@)a}; print -r "[$v]""#);
    }

    /// The same `nulstring` field through an operator on the split value
    /// (subscript, substring, pattern removal, substitution).
    #[test]
    fn equals_split_with_operator_keeps_middle_empty_field() {
        assert_parity(r#"IFS=:; s=a::b; print -rl -- x${=s[1,4]}y x${=s[1,-1]}y x${=s:0:4}y | wc -l"#);
        assert_parity(r#"IFS=:; s=a::b; print -rl -- x${=s#q}y x${=s//q/r}y; a=(x${=s[1,4]}y); print $#a"#);
        assert_parity(r#"IFS=:; s=:a::b:; print -rl -- x${=s#q}y; IFS=": "; s=" a : :b "; print -rl -- x${=s#q}y"#);
        assert_parity(r#"setopt shwordsplit; IFS=:; s=a::b; print -rl -- x${s#q}y; unsetopt shwordsplit; print -rl -- x${${=s}[@]}y"#);
    }

    #[test]
    fn whitespace_edges_and_shwordsplit_unchanged() {
        assert_parity(r#"s=" foo bar "; print -rl -- x${=s} ${=s}y x${=s}y ${=s}"#);
        assert_parity(r#"setopt shwordsplit; IFS=:; s=":a::b:"; print -rl -- x${s}y x${s} ${s}"#);
        assert_parity(r#"a=(x '' y); print -rl -- p${a}q p${a[@]}q; set -- x '' y; print -rl -- p$@q"#);
    }
}

/// c:Src/subst.c:36 — a quoted EMPTY literal (`""`, `''`) keeps its Dnull/Snull
/// in the word, so the first node (leading literal) or the last node (trailing
/// literal) of a split or array expansion is non-empty at c:183 and survives.
mod quoted_empty_literal_anchors_edge_field {
    use super::*;

    #[test]
    fn shwordsplit_edges() {
        assert_parity(r#"setopt shwordsplit; s=" a"; print -rl -- ""$s ''$s; s="a "; print -rl -- $s"""#);
        assert_parity(r#"setopt shwordsplit; s=" a b "; t() { print -r $#: "$@" }; t ""$s"" ""$s$s"""#);
        assert_parity(r#"setopt shwordsplit; e=; print -rl -- ""$e | wc -l; s=" a"; print -rl -- x""$s"#);
    }

    #[test]
    fn equals_split_and_arrays() {
        assert_parity(r#"s=" a"; print -rl -- ""${=s} ""${=s}""; s="a "; print -rl -- ${=s}''"#);
        assert_parity(r#"a=("" x ""); print -rl -- ""$a"" | wc -l; a=(x y); print -rl -- "${a[@]}" x"${a[@]}"y"#);
        assert_parity(r#"setopt shwordsplit rcexpandparam; s=" a"; print -rl -- ""$s | wc -l"#);
    }

    /// The braced `${s}` beside a quoted literal keeps its own quoting.
    #[test]
    fn braced_expansion_beside_quoted_literal() {
        assert_parity(r#"setopt shwordsplit; s=" a"; print -rl -- """${s}" ""${s} """${s}""x"; s="a "; print -rl -- "${s}""""#);
        assert_parity(r#"s=" a"; print -rl -- """${=s}" x"${s}" "${s}"; a=(x "" y); print -rl -- ""${a}"" | wc -l"#);
    }

    /// An operator expansion beside a quoted literal keeps its own quoting.
    #[test]
    fn operator_expansion_beside_quoted_literal() {
        assert_parity(r#"setopt shwordsplit; s=" a"; print -rl -- """${s:-q}" ""${s:-q} "${s:-q}""" """${s#q}" ""${s#q}"" ${s:-q}"#);
        assert_parity(r#"setopt shwordsplit; s=" a b "; print -rl -- ""${s/a/c}; print -rl -- ""${nope:- a b }"" x"${s:-q}"y"#);
        assert_parity(r#"print -r -- ${nope:-'~'}x; a=(x "" y); print -rl -- ""${a:-q}""; u=; print -rl -- ""${u:-} | wc -l"#);
    }

    /// The positional splat beside a quoted empty literal.
    #[test]
    fn positional_splat_beside_quoted_literal() {
        assert_parity(r#"set -- "" x ""; print -rl -- ""$@"" ''$@'' ""$*"" ""${@}"" ""$argv"" | wc -l"#);
        assert_parity(r#"set -- "" x; print -rl -- ""$@; set -- x ""; print -rl -- $@""; set -- "" x ""; print -rl -- "$@" | wc -l"#);
    }
}

/// c:Src/subst.c:4366-4437 then c:183-186 — an array splice's empty edge
/// elements take the word's prefix/suffix before prefork removes empty nodes.
mod array_splice_edge_empties_take_affixes {
    use super::*;

    #[test]
    fn affixed_splice_keeps_edge_elements() {
        assert_parity(r#"a=(x y ""); print -rl -- p${a[@]}q | wc -l; a=("" x); print -rl -- p${a[@]}q | wc -l"#);
        assert_parity(r#"a=("" x ""); print -rl -- p${a[@]}q ""${a[@]} ""${a[@]}"" | wc -l"#);
        assert_parity(r#"a=(x "" y); print -rl -- p${a[@]}q ${a[@]} "${a[@]}" | wc -l"#);
    }

    /// The same order for the positional splat: `$@`, `${@}`, `$*`, `$argv`.
    #[test]
    fn affixed_positional_splat_keeps_edge_elements() {
        assert_parity(r#"set -- "" x ""; print -rl -- p$@q p${@}q p$*q p$argv q p${argv}q"#);
        assert_parity(r#"set -- "" x; print -rl -- p$@; set -- x ""; print -rl -- $@q; set -- "" ""; print -rl -- p$@q"#);
        assert_parity(r#"set -- "" x ""; a=(p$@q); print $#a; for i in p$@q; do print -r "<$i>"; done; print -rl -- $@ | wc -l"#);
        assert_parity(r#"setopt rcexpandparam; set -- "" x ""; print -rl -- p$@q; v=p$@q; print -r "[$v]""#);
    }

    /// The same join-and-resplit through an affixed `${a[@]}`.
    #[test]
    fn shwordsplit_array_splice_keeps_nulstring_fields() {
        assert_parity(r#"setopt shwordsplit; IFS=:; a=(a "" b); print -rl -- x${a[@]}y ${a[@]}; b=(x${a[@]}y); print $#b"#);
        assert_parity(r#"setopt shwordsplit; IFS=:; a=(":a" "b:"); print -rl -- p${a[@]}q; for i in p${a[@]}q; do print "<$i>"; done"#);
    }

    /// SH_WORD_SPLIT's join-and-resplit of `$@` (c:3905 then c:3919) keeps the
    /// IFS-whitespace edge fields for the affixes and the `nulstring` fields.
    #[test]
    fn shwordsplit_positional_splat_keeps_fields() {
        assert_parity(r#"setopt shwordsplit; set -- "" x ""; print -rl -- p$@q p$*q $@; set -- "a b" ""; print -rl -- p$@q"#);
        assert_parity(r#"setopt shwordsplit; IFS=:; set -- a::b "" :c:; print -rl -- $@ x$@y $argv; a=($@); print $#a; f() { print $#; }; f $@"#);
        assert_parity(r#"setopt shwordsplit; IFS=:; set -- a::b "" :c:; v=$@; print -r "[$v]"; print -rl -- $* | wc -l"#);
    }
}
