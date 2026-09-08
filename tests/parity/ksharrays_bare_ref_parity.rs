//! `setopt KSH_ARRAYS` — a BARE array/hash reference is element 0, and every
//! operator downstream sees only that element.
//!
//! c:Src/params.c:2286-2288, in `fetchvalue`:
//!
//! ```c
//! } else if (!(scanflags & SCANPM_ASSIGNING) && v->scanflags &&
//!            itype_end(t, INAMESPC, 1) != t && isset(KSHARRAYS))
//!     v->end = 1, v->scanflags = 0;
//! ```
//!
//! Not assigning + a real identifier + KSH_ARRAYS clamps the Value to a single
//! element, so `$a` IS `$a[0]` and `${a:u}` / `${a/p/r}` / `${a:h}` all operate
//! on that one string. zshrs re-reads the parameter at ~90 operator arms
//! instead of holding one Value, and the clamp was applied at only nine of
//! them. The two symptoms that produced:
//!
//!   * element-wise operators (`:u`, `:l`, `/`, `#`, `%`) folded EVERY element;
//!   * the double-quoted modifier arm re-fetched and sepjoin'd the whole array,
//!     so `"${a:h}"` took the head of the JOINED text (`/x/one.txt /y`).
//!
//! Both are the same missing clamp seen through different consumers.
//!
//! The `[@]`/`[*]` rows are the other half of the contract: an explicit splat
//! keeps the whole array (`v->scanflags` survives because `getindex` ran), and
//! `${(A)a}` keeps it too because `(A)` sets `SCANPM_ASSIGNING`
//! (c:Src/subst.c:2767-2768). `$@` / `$*` are not identifiers, so they never
//! clamp at all.
//!
//! Every case is also asserted with KSH_ARRAYS UNSET, which is the default and
//! must not move.
//!
//! Skip pattern: tests no-op silently when `zsh` isn't on PATH.

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

/// Wrap an expansion so word BOUNDARIES are visible: a joined
/// `"/x/one.txt /y/two.txt"` and a two-word splat print differently.
fn script(setopt: &str, setup: &str, expr: &str) -> String {
    format!(
        "{setopt} ksharrays\n{setup}\nset -- {expr}\nprintf '%d' $#\nprintf ' <%s>' \"$@\"\nprint"
    )
}

fn run(bin: &str, args: &[&str], s: &str) -> (String, i32) {
    let mut cmd = Command::new(bin);
    cmd.args(args).arg(s);
    let o = cmd.env_remove("ZSHRS_CACHE").output().expect("spawn shell");
    (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        o.status.code().unwrap_or(-1),
    )
}

/// Assert byte parity for one expansion under BOTH option states.
fn both_states(setup: &str, expr: &str) {
    if !zsh_available() {
        return;
    }
    let rs = zshrs_bin();
    let rs = rs.to_str().expect("utf-8 path");
    for setopt in ["setopt", "unsetopt"] {
        let s = script(setopt, setup, expr);
        let (zo, zx) = run(zsh_path(), &["-f", "-c"], &s);
        let (ro, rx) = run(rs, &["--zsh", "-f", "-c"], &s);
        assert_eq!(
            zo, ro,
            "stdout divergence under `{setopt} ksharrays`:\n{s}\n--- zsh ---\n{zo:?}\n--- zshrs ---\n{ro:?}"
        );
        assert_eq!(zx, rx, "exit divergence under `{setopt} ksharrays`:\n{s}");
    }
}

const ARR: &str = "a=(/x/one.txt /y/two.txt)";
const HASH: &str = "typeset -A h; h=(k1 /a/v1.txt k2 /b/v2.txt)";

/// The element-wise symptom: the operator folded every element.
mod folds_every_element {
    use super::*;

    #[test]
    fn case_upper_modifier() {
        both_states(ARR, "${a:u}");
    }

    #[test]
    fn case_lower_modifier() {
        both_states(ARR, "${a:l}");
    }

    #[test]
    fn replace_first_match() {
        both_states(ARR, "${a/one/Z}");
    }

    #[test]
    fn replace_all_matches() {
        both_states(ARR, "${a//o/Z}");
    }

    #[test]
    fn strip_shortest_prefix() {
        both_states(ARR, "${a#/x/}");
    }

    #[test]
    fn strip_longest_prefix() {
        both_states(ARR, "${a##*/}");
    }

    #[test]
    fn strip_shortest_suffix() {
        both_states(ARR, "${a%.txt}");
    }

    #[test]
    fn strip_longest_suffix() {
        both_states(ARR, "${a%%.*}");
    }

    #[test]
    fn quote_modifier() {
        both_states(ARR, "${a:q}");
    }

    #[test]
    fn padding_flag_left() {
        both_states(ARR, "${(l:12:)a}");
    }

    #[test]
    fn padding_flag_right() {
        both_states(ARR, "${(r:12:)a}");
    }

    #[test]
    fn visible_flag() {
        both_states(ARR, "${(V)a}");
    }

    #[test]
    fn quote_flag() {
        both_states(ARR, "${(q)a}");
    }

    #[test]
    fn default_when_set() {
        both_states(ARR, "${a:-EMPTY}");
    }

    #[test]
    fn error_operator_when_set() {
        both_states(ARR, "${a:?}");
    }

    #[test]
    fn split_flag() {
        both_states(ARR, "${(s:/:)a}");
    }

    /// c:2288's other half, `v->scanflags = 0`: with the array shape gone,
    /// c:Src/subst.c:3665's slice block never runs and `${a:OFFSET}` is a
    /// CHARACTER substring of element 0.
    #[test]
    fn substring_offset_is_a_character_offset() {
        both_states(ARR, "${a:1}");
    }

    #[test]
    fn substring_offset_and_length() {
        both_states(ARR, "${a:1:1}");
    }

    #[test]
    fn substring_offset_zero() {
        both_states(ARR, "${a:0}");
    }

    #[test]
    fn substring_offset_past_the_end() {
        both_states(ARR, "${a:100}");
    }

    #[test]
    fn substring_negative_offset() {
        both_states(ARR, "${a: -3}");
    }

    #[test]
    fn expand_flag() {
        both_states(ARR, "${(e)a}");
    }

    #[test]
    fn char_flag_on_numbers() {
        both_states("a=(65 66)", "${(#)a}");
    }

    /// `argv` IS an identifier, so an operator on it clamps like any other
    /// array name. (The BARE `$argv` / `${argv}` read never reaches paramsubst
    /// — it is answered by the BUILTIN_GET_VAR fast path — and still returns
    /// every positional; that path is not what this file pins.)
    #[test]
    fn argv_with_a_modifier() {
        both_states("set -- /p/a /q/b", "${argv:h}");
    }

    #[test]
    fn argv_with_a_strip() {
        both_states("set -- /p/a /q/b", "${argv#/p/}");
    }
}

/// The joined-string symptom: the double-quoted modifier arm re-fetched the
/// whole array and sepjoin'd it, so the modifier ran on the joined text.
mod dq_modifier_saw_the_join {
    use super::*;

    #[test]
    fn head_modifier() {
        both_states(ARR, "\"${a:h}\"");
    }

    #[test]
    fn tail_modifier() {
        both_states(ARR, "\"${a:t}\"");
    }

    #[test]
    fn root_modifier() {
        both_states(ARR, "\"${a:r}\"");
    }

    #[test]
    fn extension_modifier() {
        both_states(ARR, "\"${a:e}\"");
    }

    #[test]
    fn case_upper_modifier() {
        both_states(ARR, "\"${a:u}\"");
    }

    #[test]
    fn quote_modifier() {
        both_states(ARR, "\"${a:q}\"");
    }

    #[test]
    fn substitute_modifier() {
        both_states(ARR, "\"${a:s/one/Z/}\"");
    }

    #[test]
    fn unquoted_head_modifier() {
        both_states(ARR, "${a:h}");
    }

    #[test]
    fn unquoted_tail_modifier() {
        both_states(ARR, "${a:t}");
    }

    #[test]
    fn unquoted_root_modifier() {
        both_states(ARR, "${a:r}");
    }
}

/// The clamp must NOT fire: an explicit subscript or splat, a non-identifier
/// name, or `(A)`'s SCANPM_ASSIGNING.
mod clamp_must_not_fire {
    use super::*;

    #[test]
    fn at_splat_keeps_every_element() {
        both_states(ARR, "${a[@]}");
    }

    #[test]
    fn star_splat_keeps_every_element() {
        both_states(ARR, "${a[*]}");
    }

    #[test]
    fn quoted_at_splat_keeps_every_element() {
        both_states(ARR, "\"${a[@]}\"");
    }

    /// The compiler rebuilt `${a[@]#pat}` as `${a#pat}` for the runtime, which
    /// threw the splat away before paramsubst could record it (Bug #1054's
    /// shape, in the Strip arm).
    #[test]
    fn at_splat_with_strip() {
        both_states(ARR, "${a[@]#/x/}");
    }

    #[test]
    fn at_splat_with_longest_suffix_strip() {
        both_states(ARR, "${a[@]%.txt}");
    }

    /// Same reconstruction gap in the Replace arm.
    #[test]
    fn at_splat_with_replace() {
        both_states(ARR, "${a[@]/one/Z}");
    }

    #[test]
    fn star_splat_with_strip() {
        both_states(ARR, "${a[*]#/x/}");
    }

    #[test]
    fn at_splat_with_modifier() {
        both_states(ARR, "${a[@]:h}");
    }

    #[test]
    fn at_splat_with_case_modifier() {
        both_states(ARR, "${a[@]:u}");
    }

    /// A subscript means the clamp never fired, so `:OFFSET` is still the
    /// ARRAY slice.
    #[test]
    fn at_splat_slice_offset() {
        both_states(ARR, "${a[@]:1}");
    }

    #[test]
    fn at_splat_slice_offset_and_length() {
        both_states(ARR, "${a[@]:0:1}");
    }

    #[test]
    fn single_slot_substring() {
        both_states(ARR, "${a[1]:0:3}");
    }

    #[test]
    fn scalar_substring() {
        both_states("s=/p/q.txt", "${s:1:2}");
    }

    #[test]
    fn argv_splat_keeps_every_positional() {
        both_states("set -- p q", "${argv[@]}");
    }

    #[test]
    fn argv_single_slot_subscript() {
        both_states("set -- p q", "${argv[1]}");
    }

    #[test]
    fn at_flag_keeps_every_element() {
        both_states(ARR, "${(@)a}");
    }

    /// `(A)` sets `arrasg`, which becomes `SCANPM_ASSIGNING` — c:2286's first
    /// conjunct, so the clamp is skipped and the whole array survives.
    #[test]
    fn array_assign_flag_keeps_every_element() {
        both_states(ARR, "${(A)a}");
    }

    #[test]
    fn single_slot_subscript() {
        both_states(ARR, "${a[1]}");
    }

    #[test]
    fn range_subscript() {
        both_states(ARR, "${a[0,1]}");
    }

    #[test]
    fn range_subscript_with_strip() {
        both_states(ARR, "${a[0,1]#/}");
    }

    /// `itype_end("@", INAMESPC, 1)` returns the name unchanged, so positionals
    /// never clamp.
    #[test]
    fn positional_splat_unquoted() {
        both_states("set -- /p/a /q/b", "${@:h}");
    }

    #[test]
    fn positional_splat_quoted() {
        both_states("set -- /p/a /q/b", "\"$@\"");
    }

    #[test]
    fn positional_star_quoted_modifier() {
        both_states("set -- /p/a /q/b", "\"${*:h}\"");
    }

    #[test]
    fn positional_splat_with_strip() {
        both_states("set -- /p/a /q/b", "${@#/p/}");
    }

    /// A scalar has no array shape to clamp.
    #[test]
    fn scalar_is_untouched_by_the_clamp() {
        both_states("s=/p/q.txt", "${s:h}");
    }

    #[test]
    fn scalar_replace_is_untouched() {
        both_states("s=/p/q.txt", "${s/q/Z}");
    }
}

/// Shape/type reads answer about the PARAMETER, not the clamped value.
mod shape_reads {
    use super::*;

    #[test]
    fn type_flag_still_says_array() {
        both_states(ARR, "${(t)a}");
    }

    #[test]
    fn bare_length_is_the_element_length() {
        both_states(ARR, "${#a}");
    }

    #[test]
    fn splat_length_is_the_element_count() {
        both_states(ARR, "${#a[@]}");
    }

    #[test]
    fn bare_reference() {
        both_states(ARR, "$a");
    }

    #[test]
    fn braced_bare_reference() {
        both_states(ARR, "${a}");
    }

    #[test]
    fn quoted_bare_reference() {
        both_states(ARR, "\"$a\"");
    }
}

/// Order/join flags run after the clamp, so they see one element.
mod flags_after_the_clamp {
    use super::*;

    #[test]
    fn sort_flag() {
        both_states(ARR, "${(o)a}");
    }

    #[test]
    fn reverse_sort_flag() {
        both_states(ARR, "${(O)a}");
    }

    #[test]
    fn unique_flag() {
        both_states(ARR, "${(u)a}");
    }

    #[test]
    fn join_flag() {
        both_states(ARR, "${(j:x:)a}");
    }

    #[test]
    fn join_flag_quoted() {
        both_states(ARR, "\"${(j:-:)a}\"");
    }

    /// The splat form of the same flag keeps the whole array — the distinction
    /// the clamp exists to preserve.
    #[test]
    fn sort_flag_on_splat() {
        both_states(ARR, "${(o)a[@]}");
    }

    #[test]
    fn join_flag_on_splat() {
        both_states(ARR, "${(j:x:)a[@]}");
    }

    #[test]
    fn upper_flag() {
        both_states(ARR, "${(U)a}");
    }

    #[test]
    fn lower_flag() {
        both_states(ARR, "${(L)a}");
    }

    #[test]
    fn filter_removes_matching() {
        both_states(ARR, "${a:#*two*}");
    }

    #[test]
    fn filter_keeps_matching() {
        both_states(ARR, "${(M)a:#*two*}");
    }
}

/// Associations clamp to their first VALUE, and an explicit subscript exempts
/// them exactly as it does an array.
mod associations {
    use super::*;

    #[test]
    fn bare_hash_is_the_first_value() {
        both_states(HASH, "${h}");
    }

    #[test]
    fn hash_strip_prefix() {
        both_states(HASH, "${h#/a/}");
    }

    #[test]
    fn hash_replace() {
        both_states(HASH, "${h/v/Z}");
    }

    #[test]
    fn hash_join_flag() {
        both_states(HASH, "${(j:-:)h}");
    }

    #[test]
    fn hash_bare_length() {
        both_states(HASH, "${#h}");
    }

    #[test]
    fn hash_key_subscript() {
        both_states(HASH, "${h[k1]}");
    }

    #[test]
    fn hash_splat_keeps_every_value() {
        both_states(HASH, "${h[@]}");
    }

    #[test]
    fn hash_type_flag() {
        both_states(HASH, "${(t)h}");
    }

    #[test]
    fn hash_default_when_set() {
        both_states(HASH, "${h:-D}");
    }
}

/// Set operators fetch their RHS name through a SEPARATE `fetchvalue`, whose
/// subscript state is its own.
mod set_operators {
    use super::*;

    #[test]
    fn zip_two_arrays() {
        both_states("a=(1 2); b=(x y)", "${a:^b}");
    }

    #[test]
    fn zip_cycling() {
        both_states("a=(1 2); b=(x y)", "${a:^^b}");
    }

    #[test]
    fn difference() {
        both_states("a=(p q); b=(p)", "${a:|b}");
    }

    #[test]
    fn intersection() {
        both_states("a=(p q); b=(p)", "${a:*b}");
    }

    #[test]
    fn zip_with_splat_lhs() {
        both_states("a=(1 2); b=(x y)", "${a[@]:^b}");
    }
}

/// The colon-modifier chain on an UNBRACED reference is not applied at all
/// under KSH_ARRAYS — it stays literal text.
///
/// c:Src/subst.c:3770-3776, the whole history-style modifier block:
///
/// ```c
/// if (colf) {
///     s--;
///     if (unset(KSHARRAYS) || inbrace) {
///         if (!isarr) modify(&val, &s, inbrace);
///         else { … per-element modify … }
/// ```
///
/// `inbrace` is 0 for `$var:h`, so with KSH_ARRAYS set the guard is false and
/// neither `modify` leg runs; `s` is left pointing at the `:` and c:3805's
/// `if (!inbrace) fstr = s;` hands `:h` back as ordinary text. `${var:h}` is
/// braced, so `inbrace` carries it through in either option state — that pair
/// is the whole contract, and both directions are asserted here so a fix to
/// one cannot be bought by breaking the other.
mod unbraced_modifier_is_literal_under_ksharrays {
    use super::*;

    const S: &str = "s=/x/y/z.txt";

    #[test]
    fn head_on_a_scalar() {
        both_states(S, "$s:h");
    }

    #[test]
    fn tail_on_a_scalar() {
        both_states(S, "$s:t");
    }

    #[test]
    fn root_on_a_scalar() {
        both_states(S, "$s:r");
    }

    /// A chain: KSH_ARRAYS must leave the whole `:h:t` run as text, not just
    /// the first link.
    #[test]
    fn chained_modifiers_on_a_scalar() {
        both_states(S, "$s:h:t");
    }

    /// The `:s/…/…/` form takes an argument, so a half-applied guard would
    /// show up as a mangled tail rather than a clean literal.
    #[test]
    fn substitution_modifier_on_a_scalar() {
        both_states("s=abc", "$s:s/b/Z/");
    }

    /// The braced control: `inbrace` is 1, so the modifier applies in BOTH
    /// option states.
    #[test]
    fn braced_head_still_applies() {
        both_states(S, "${s:h}");
    }

    /// Unbraced on an array — with KSH_ARRAYS the bare reference is already
    /// clamped to element 0 and the modifier is still skipped on top of that.
    #[test]
    fn head_on_a_bare_array() {
        both_states(ARR, "$a:h");
    }

    /// Positional parameters take the same unbraced path (`$1:h`).
    #[test]
    fn head_on_a_positional() {
        both_states("set -- /p/a.txt /q/b.txt", "$1:h");
    }
}
