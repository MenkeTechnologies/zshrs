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
