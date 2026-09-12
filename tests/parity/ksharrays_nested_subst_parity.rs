//! `setopt KSH_ARRAYS` — the element-0 clamp applies to a PARAMETER FETCH, and
//! a nested substitution never performs one.
//!
//! The clamp is c:Src/params.c:2286-2288, inside `fetchvalue`:
//!
//! ```c
//! } else if (!(scanflags & SCANPM_ASSIGNING) && v->scanflags &&
//!            itype_end(t, INAMESPC, 1) != t && isset(KSHARRAYS))
//!     v->end = 1, v->scanflags = 0;
//! ```
//!
//! `paramsubst` calls `fetchvalue` exactly once, at c:Src/subst.c:2801, and
//! that call sits under c:Src/subst.c:2764:
//!
//! ```c
//! if (!subexp || aspar) {
//!     ...
//!     if (!rplyvar && (!(v = fetchvalue(&vbuf, (subexp ? &ov : &s), ...
//! ```
//!
//! A nested `${…}` sets `subexp = 1` at c:Src/subst.c:2650 and gets its value
//! from `multsub(&val, PREFORK_SUBEXP, &aval, &isarr, NULL, &ms_flags)` at
//! c:Src/subst.c:2683. No parameter is fetched, so the clamp cannot reach the
//! array that comes up — a split flag's result keeps every element:
//!
//! ```
//! % zsh -f -c 'setopt ksharrays; v="a b c"; print -r -- ${#${(z)v}}'
//! 3
//! ```
//!
//! zshrs materialises that inner array into a synthetic `__subexp_arr_N`
//! parameter so its existing splat/subscript/filter arms can read it by name.
//! The temp name is a plain identifier with no subscript, which is exactly the
//! clamp's predicate, so the whole nested result was truncated to one element.
//! The defect was in the CARRIER, not in any one flag, which is why `(z)`,
//! `(s: :)`, `(f)` and `(0)` lost their elements together and why `(j)`, `(o)`,
//! `(u)`, `(U)`, `:u`, `:h`, `:#` and `(@)` all read only the first one.
//!
//! `(P)` is the exception, and it is in the same C line. An inner `${(P)n}`
//! never returns a value: c:Src/subst.c:2757 sets `*ret_flags |=
//! MULTSUB_PARAM_NAME` and the outer instance splices the dereferenced NAME
//! back into the expression and turns the subexp off again
//! (c:Src/subst.c:2707-2709 `s = dyncat(val, s); subexp = 0;`). So the outer
//! really does fetch a parameter, and it really does clamp.
//!
//! Every case is asserted under BOTH option states; `unsetopt ksharrays` is
//! the default and must not move.
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

fn run(bin: &str, args: &[&str], s: &str) -> (Vec<u8>, i32) {
    let o = Command::new(bin)
        .args(args)
        .arg(s)
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("spawn shell");
    (o.stdout, o.status.code().unwrap_or(-1))
}

/// Values are multi-character on purpose: with single-char elements an element
/// COUNT and a string LENGTH are the same number, so a collapse to element 0
/// would be invisible.
const SETUP: &str = "kv='aa bb cc'; kf=$'aa\\nbb\\ncc'; ka=(aa bb cc); kp=ka";

/// Assert byte parity for one expansion under BOTH option states.
fn both_states(expr: &str) {
    if !zsh_available() {
        return;
    }
    let rs = zshrs_bin();
    let rs = rs.to_str().expect("utf-8 path");
    for setopt in ["setopt", "unsetopt"] {
        let s = format!("{setopt} ksharrays; {SETUP}\nprint -r -- {expr}");
        let (zo, zx) = run(zsh_path(), &["-f", "-c"], &s);
        let (ro, rx) = run(rs, &["--zsh", "-f", "-c"], &s);
        assert_eq!(
            zo,
            ro,
            "stdout diverges under `{setopt} ksharrays`:\n{s}\n--- zsh   --- {:?}\n--- zshrs --- {:?}",
            String::from_utf8_lossy(&zo),
            String::from_utf8_lossy(&ro)
        );
        assert_eq!(zx, rx, "exit status diverges under `{setopt} ksharrays`:\n{s}");
    }
}

/// Run one operator against every SPLIT-FLAG subject. All four produce an
/// array the same way, through `multsub`, so all four must behave alike — the
/// point of the fix is that the rule is about the carrier, not the flag.
fn every_split_flag(op_around_subject: &dyn Fn(&str) -> String) {
    for subject in ["${(z)kv}", "${(s: :)kv}", "${(f)kf}"] {
        both_states(&op_around_subject(subject));
    }
}

/// The reported shape: a split flag's element count.
mod split_flag_result_keeps_every_element {
    use super::*;

    #[test]
    fn z_flag_count() {
        both_states("${#${(z)kv}}");
    }

    #[test]
    fn s_flag_count() {
        both_states("${#${(s: :)kv}}");
    }

    #[test]
    fn f_flag_count() {
        both_states("${#${(f)kf}}");
    }

    #[test]
    fn nul_flag_count() {
        if !zsh_available() {
            return;
        }
        // `(0)` splits on NUL, so build the source with `$'…'` directly.
        let rs = zshrs_bin();
        let rs = rs.to_str().expect("utf-8 path");
        for setopt in ["setopt", "unsetopt"] {
            let s = format!(
                "{setopt} ksharrays; kn=$'aa\\x00bb\\x00cc'\nprint -r -- ${{#${{(0)kn}}}}"
            );
            let (zo, _) = run(zsh_path(), &["-f", "-c"], &s);
            let (ro, _) = run(rs, &["--zsh", "-f", "-c"], &s);
            assert_eq!(zo, ro, "`(0)` count diverges under `{setopt} ksharrays`:\n{s}");
        }
    }

    #[test]
    fn explicit_at_subscript_count() {
        both_states("${#${(z)kv}[@]}");
    }

    #[test]
    fn at_flag_count() {
        both_states("${#${(@s: :)kv}}");
    }
}

/// Every operator that open-coded its own copy of the clamp predicate. Each
/// one read only element 0 of the nested result before the fix.
mod operators_see_the_whole_nested_result {
    use super::*;

    #[test]
    fn join_flag() {
        every_split_flag(&|s| format!("${{(j:-:){s}}}"));
    }

    #[test]
    fn sort_flag() {
        every_split_flag(&|s| format!("${{(o){s}}}"));
    }

    #[test]
    fn reverse_sort_flag() {
        every_split_flag(&|s| format!("${{(O){s}}}"));
    }

    #[test]
    fn unique_flag() {
        every_split_flag(&|s| format!("${{(u){s}}}"));
    }

    #[test]
    fn numeric_sort_flag() {
        every_split_flag(&|s| format!("${{(n){s}}}"));
    }

    #[test]
    fn upper_flag() {
        every_split_flag(&|s| format!("${{(U){s}}}"));
    }

    #[test]
    fn lower_flag() {
        every_split_flag(&|s| format!("${{(L){s}}}"));
    }

    #[test]
    fn upper_modifier() {
        every_split_flag(&|s| format!("${{{s}:u}}"));
    }

    #[test]
    fn lower_modifier() {
        every_split_flag(&|s| format!("${{{s}:l}}"));
    }

    #[test]
    fn head_modifier() {
        every_split_flag(&|s| format!("${{{s}:h}}"));
    }

    #[test]
    fn filter_out_matching() {
        every_split_flag(&|s| format!("${{{s}:#bb}}"));
    }

    #[test]
    fn filter_keep_matching() {
        every_split_flag(&|s| format!("${{(M){s}:#bb}}"));
    }

    #[test]
    fn at_splat() {
        every_split_flag(&|s| format!("${{(@){s}}}"));
    }

    #[test]
    fn double_quoted_splat() {
        every_split_flag(&|s| format!("\"${{{s}}}\""));
    }

    #[test]
    fn join_with_multichar_separator() {
        every_split_flag(&|s| format!("${{(j/u/@){s}}}"));
    }
}

/// Indexing was already right and must stay right: KSH_ARRAYS subscripts are
/// 0-based (c:Src/params.c:1616-1619 `if (isset(KSHARRAYS) && r >= 0) r++;`),
/// so `[0]` is the first element under `setopt` and the empty slot under
/// `unsetopt`.
mod subscripting_unchanged {
    use super::*;

    #[test]
    fn index_zero() {
        both_states("${${(z)kv}[0]}");
    }

    #[test]
    fn index_one() {
        both_states("${${(z)kv}[1]}");
    }

    #[test]
    fn index_two() {
        both_states("${${(z)kv}[2]}");
    }

    #[test]
    fn unquoted_splat() {
        both_states("${(z)kv}");
    }
}

/// A BARE array reference still clamps — the fix must not disable the rule it
/// was scoped around. These are the rows from the original `ksharrays` gap.
mod bare_reference_still_clamps {
    use super::*;

    #[test]
    fn bare_splat() {
        both_states("${ka}");
    }

    #[test]
    fn bare_count() {
        both_states("${#ka}");
    }

    #[test]
    fn bare_upper_modifier() {
        both_states("${ka:u}");
    }

    #[test]
    fn bare_upper_flag() {
        both_states("${(U)ka}");
    }

    #[test]
    fn bare_join_flag() {
        both_states("${(j/u/@)ka}");
    }

    #[test]
    fn bare_sort_flag() {
        both_states("${(o)ka}");
    }

    #[test]
    fn bare_filter() {
        both_states("${ka:#bb}");
    }

    #[test]
    fn bare_head_modifier() {
        both_states("${ka:h}");
    }

    #[test]
    fn explicit_at_splat_keeps_the_array() {
        both_states("${ka[@]}");
    }

    #[test]
    fn explicit_at_splat_count() {
        both_states("${#ka[@]}");
    }
}

/// `(P)` splices a NAME, so the outer expansion fetches a real parameter and
/// the clamp applies (c:Src/subst.c:2707-2709, 2757).
mod indirect_reference_still_clamps {
    use super::*;

    #[test]
    fn p_flag_splat() {
        both_states("${(P)kp}");
    }

    #[test]
    fn p_flag_join() {
        both_states("${(j:-:)${(P)kp}}");
    }

    #[test]
    fn p_flag_sort() {
        both_states("${(o)${(P)kp}}");
    }

    #[test]
    fn p_flag_upper_modifier() {
        both_states("${${(P)kp}:u}");
    }

    #[test]
    fn p_flag_filter() {
        both_states("${${(P)kp}:#bb}");
    }

    #[test]
    fn p_flag_replace() {
        both_states("${${(P)kp}/aa/ZZ}");
    }

    #[test]
    fn p_flag_strip_prefix() {
        both_states("${${(P)kp}#aa}");
    }

    #[test]
    fn p_flag_default() {
        both_states("${${(P)kp}:-X}");
    }
}
