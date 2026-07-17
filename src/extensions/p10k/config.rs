//! p10k configuration fallback chain — port of `_p9k_param`
//! (powerlevel10k internal/p10k.zsh:474-507) plus the boolean
//! semantics of `_p9k_declare -b` (p10k.zsh:141-151).
//!
//! In zsh, p10k copies every user `POWERLEVEL9K_*` parameter into an
//! internal `_POWERLEVEL9K_*` twin at init (p10k.zsh:7589-7594):
//!
//!   for var in ${(@)${parameters[(I)POWERLEVEL9K_*]}/...}; do
//!     case $parameters[$var] in
//!       (scalar|integer|float)*) typeset -g _$var=${(P)var};;
//!       array*)                  eval 'typeset -ga '_$var'=(...)';;
//!     esac
//!   done
//!
//! and `_p9k_param` probes the `_POWERLEVEL9K_*` copies. This Rust
//! port reads the user-facing `POWERLEVEL9K_*` parameters directly
//! from paramtab — the copy layer exists in zsh only so p10k can
//! normalize/own the values; the probe ORDER is identical.
//!
//! p10k's `_p9k_cache[$key]` memoization layer (p10k.zsh:476-479) is
//! intentionally omitted: a paramtab read is a RwLock-read +
//! HashMap-lookup and needs no invalidation story when the user
//! re-sources ~/.p10k.zsh.

use crate::ported::params::{getaparam, getsparam};

/// Split the tail of a `prompt_*` style name exactly the way the
/// zsh pattern does.
///
/// p10k:481 — `[[ ${1//-/_} == (#b)prompt_([a-z0-9_]#)(*) ]]`
///
/// `[a-z0-9_]#` is a GREEDY run of lowercase/digit/underscore, so for
/// `prompt_vcs_CLEAN` the split is `("vcs_", "CLEAN")` — the segment
/// half keeps its trailing underscore and the state half is whatever
/// remains (uppercase state names stop the class). For `prompt_dir`
/// (no state) it is `("dir", "")`.
fn split_lower_prefix(rest: &str) -> (&str, &str) {
    let end = rest
        .find(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'))
        .unwrap_or(rest.len());
    (&rest[..end], &rest[end..])
}

/// Build the probe chain of user-parameter names for
/// `p9k_param(segment, state, param)`, mirroring _p9k_param's
/// three indirections in order.
///
/// p10k:482 — `var=_POWERLEVEL9K_${${(U)match[1]}//İ/I}$match[2]_$2`
/// p10k:486 — `var=_POWERLEVEL9K_${${(U)match[1]%_}//İ/I}_$2`
/// p10k:490 — `var=_POWERLEVEL9K_$2`
///
/// The `//İ/I` in the zsh source undoes Turkish-locale dotted-İ
/// uppercasing of `i`; `to_ascii_uppercase()` sidesteps that class
/// of bug entirely (segment names are `[a-z0-9_]` by the split).
fn probe_names(segment: &str, state: Option<&str>, param: &str) -> [String; 3] {
    // p10k:481 — `${1//-/_}`: dashes in the style name become
    // underscores before matching. C callers pass `$0` — the segment
    // FUNCTION name `prompt_<segment>` (prompt_char's is
    // prompt_prompt_char). zshrs callers pass the BARE segment name,
    // prefixed unconditionally here: the old "already prefixed?"
    // heuristic mis-split the segment literally named "prompt_char"
    // as segment "char", so its scoped POWERLEVEL9K_PROMPT_CHAR_*
    // params were never probed (the user config's scoped-empty
    // LEFT_PROMPT_LAST_SEGMENT_END_SYMBOL lost to the bare global —
    // a stray end-symbol glyph painted after the prompt char).
    let seg = segment.replace('-', "_");
    let mut name = format!("prompt_{seg}");
    if let Some(st) = state {
        name.push('_');
        name.push_str(st);
    }
    let rest = &name["prompt_".len()..];
    let (m1, m2) = split_lower_prefix(rest);

    // p10k:482 — most specific: SEGMENT + STATE + PARAM
    // (match[1] uppercased keeps its trailing underscore, match[2]
    // is appended verbatim, then `_$2`).
    let p1 = format!("POWERLEVEL9K_{}{}_{}", m1.to_ascii_uppercase(), m2, param);
    // p10k:486 — `${match[1]%_}` strips ONE trailing underscore:
    // SEGMENT + PARAM without the state. When no state was present
    // this reproduces probe 1 (zsh probes the same var twice too).
    let m1_trim = m1.strip_suffix('_').unwrap_or(m1);
    let p2 = format!("POWERLEVEL9K_{}_{}", m1_trim.to_ascii_uppercase(), param);
    // p10k:490 — bare global: POWERLEVEL9K_<PARAM>.
    let p3 = format!("POWERLEVEL9K_{param}");
    [p1, p2, p3]
}

/// Walk the probe chain; `Some(value)` from the first name that is a
/// set parameter, `None` when the whole chain misses.
///
/// p10k:483/487/491 — `(( $+parameters[$var] ))` existence test, then
/// p10k:484/488/492 — `_p9k__ret=${(P)var}` indirect read. zshrs
/// `getsparam` mirrors `${(P)var}` in scalar context: set-but-empty
/// yields `Some("")`, arrays come back space-joined.
fn p9k_param_opt(segment: &str, state: Option<&str>, param: &str) -> Option<String> {
    for name in probe_names(segment, state, param) {
        if let Some(v) = getsparam(&name) {
            return Some(v);
        }
    }
    None
}

/// Port of `_p9k_param $1 $2 $3` (p10k.zsh:475-507): probe
/// `POWERLEVEL9K_<SEG>_<STATE>_<PARAM>`, then
/// `POWERLEVEL9K_<SEG>_<PARAM>`, then `POWERLEVEL9K_<PARAM>`,
/// else the caller-supplied default (p10k:494 — `_p9k__ret=$3`).
pub fn p9k_param(segment: &str, state: Option<&str>, param: &str, default: &str) -> String {
    p9k_param_opt(segment, state, param).unwrap_or_else(|| default.to_string())
}

/// Boolean read over the same probe chain.
///
/// p10k:147 — `_p9k_declare -b`: `[[ ${(P)2} == true ]] && typeset -gi
/// _$2=1 || typeset -gi _$2=0` — ONLY the exact string `true` is
/// truthy; `false`, `1`, `yes`, anything else is false. Unset falls
/// back to the declared default (p10k:149 — `typeset -gi _$2=$3`).
pub fn p9k_param_bool(segment: &str, state: Option<&str>, param: &str, default: bool) -> bool {
    match p9k_param_opt(segment, state, param) {
        Some(v) => v == "true",
        None => default,
    }
}

/// Array read over the same probe chain.
///
/// Mirrors how p10k reads array-typed config: the copy loop keeps
/// arrays as arrays (p10k:7592 — `array*) eval 'typeset -ga ...'`),
/// and consumers splat them with `("${(@P)var}")` — a scalar var under
/// that expansion becomes a one-element array. Whole-chain miss is an
/// empty vec (segments treat missing list params as empty).
pub fn p9k_param_arr(segment: &str, state: Option<&str>, param: &str) -> Vec<String> {
    for name in probe_names(segment, state, param) {
        if let Some(v) = getaparam(&name) {
            return v;
        }
        if let Some(s) = getsparam(&name) {
            // `("${(@P)var}")` on a set scalar → one-element array.
            return vec![s];
        }
    }
    Vec::new()
}

/// Read the plain global `POWERLEVEL9K_<name>` (no segment/state
/// chain) with a default — the non-`prompt_*` arm of `_p9k_param`
/// (p10k:499-505: probe `_POWERLEVEL9K_$2`, else `$3`).
pub fn p9k_global(name: &str, default: &str) -> String {
    getsparam(&format!("POWERLEVEL9K_{name}")).unwrap_or_else(|| default.to_string())
}

/// Array form of [`p9k_global`] — e.g.
/// `POWERLEVEL9K_LEFT_PROMPT_ELEMENTS`. A set scalar becomes a
/// one-element array (`("${(@P)var}")` semantics); unset is empty.
pub fn p9k_global_arr(name: &str) -> Vec<String> {
    let full = format!("POWERLEVEL9K_{name}");
    if let Some(v) = getaparam(&full) {
        return v;
    }
    if let Some(s) = getsparam(&full) {
        return vec![s];
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::options::{opt_state_get, opt_state_set};
    use crate::ported::params::{setaparam, setsparam, unsetparam};

    /// Serialize against other paramtab-touching tests and enable
    /// EXECOPT so `setsparam`/`setaparam` don't no-op (same pattern
    /// as src/ported/params.rs test module).
    fn with_exec<F: FnOnce()>(body: F) {
        let _g = crate::test_util::global_state_lock();
        let saved = opt_state_get("exec").unwrap_or(false);
        opt_state_set("exec", true);
        body();
        opt_state_set("exec", saved);
    }

    /// p10k:482 — SEGMENT+STATE+PARAM wins over SEGMENT+PARAM and
    /// bare PARAM when all three are set.
    /// Anchor: `_p9k_param prompt_vcs_CLEAN TESTFG1 def` with
    /// _POWERLEVEL9K_VCS_CLEAN_TESTFG1=76 set → 76.
    #[test]
    fn state_specific_param_wins() {
        with_exec(|| {
            setsparam("POWERLEVEL9K_VCS_CLEAN_TESTFG1", "76");
            setsparam("POWERLEVEL9K_VCS_TESTFG1", "39");
            setsparam("POWERLEVEL9K_TESTFG1", "244");
            assert_eq!(p9k_param("vcs", Some("CLEAN"), "TESTFG1", "def"), "76");
            unsetparam("POWERLEVEL9K_VCS_CLEAN_TESTFG1");
            unsetparam("POWERLEVEL9K_VCS_TESTFG1");
            unsetparam("POWERLEVEL9K_TESTFG1");
        });
    }

    /// p10k:486 — with the state-specific var unset, the
    /// SEGMENT+PARAM var (state stripped, `${match[1]%_}`) is used.
    #[test]
    fn segment_param_fallback_when_state_var_unset() {
        with_exec(|| {
            unsetparam("POWERLEVEL9K_VCS_CLEAN_TESTFG2");
            setsparam("POWERLEVEL9K_VCS_TESTFG2", "39");
            setsparam("POWERLEVEL9K_TESTFG2", "244");
            assert_eq!(p9k_param("vcs", Some("CLEAN"), "TESTFG2", "def"), "39");
            unsetparam("POWERLEVEL9K_VCS_TESTFG2");
            unsetparam("POWERLEVEL9K_TESTFG2");
        });
    }

    /// p10k:490 — bare POWERLEVEL9K_<PARAM> global is the third probe.
    #[test]
    fn global_param_fallback_when_segment_vars_unset() {
        with_exec(|| {
            unsetparam("POWERLEVEL9K_VCS_CLEAN_TESTFG3");
            unsetparam("POWERLEVEL9K_VCS_TESTFG3");
            setsparam("POWERLEVEL9K_TESTFG3", "244");
            assert_eq!(p9k_param("vcs", Some("CLEAN"), "TESTFG3", "def"), "244");
            unsetparam("POWERLEVEL9K_TESTFG3");
        });
    }

    /// p10k:494 — whole chain missing → caller default. Also covers
    /// the stateless (state=None) form where probes 1 and 2 coincide.
    #[test]
    fn default_when_nothing_set() {
        with_exec(|| {
            unsetparam("POWERLEVEL9K_DIR_TESTFG4");
            unsetparam("POWERLEVEL9K_TESTFG4");
            assert_eq!(p9k_param("dir", None, "TESTFG4", "blue"), "blue");
        });
    }

    /// Set-but-empty is a HIT, not a miss — `(( $+parameters[$var] ))`
    /// (p10k:483) is an existence test, so an empty value stops the
    /// chain and the default is NOT used.
    #[test]
    fn empty_value_is_a_hit_not_a_miss() {
        with_exec(|| {
            setsparam("POWERLEVEL9K_DIR_TESTFG5", "");
            setsparam("POWERLEVEL9K_TESTFG5", "244");
            assert_eq!(p9k_param("dir", None, "TESTFG5", "def"), "");
            unsetparam("POWERLEVEL9K_DIR_TESTFG5");
            unsetparam("POWERLEVEL9K_TESTFG5");
        });
    }

    /// p10k:481 — `${1//-/_}`: dashes in segment names map to
    /// underscores before the uppercase probe is built.
    #[test]
    fn dashed_segment_name_maps_to_underscores() {
        with_exec(|| {
            setsparam("POWERLEVEL9K_FOO_BAR_TESTFG6", "196");
            assert_eq!(p9k_param("foo-bar", None, "TESTFG6", "def"), "196");
            unsetparam("POWERLEVEL9K_FOO_BAR_TESTFG6");
        });
    }

    /// Callers may pass the full `prompt_dir` style name; it must not
    /// be double-prefixed.
    #[test]
    fn full_prompt_prefixed_segment_accepted() {
        // The segment arg is BARE by contract (probe_names prepends
        // `prompt_` unconditionally). The old "accept prompt_dir too"
        // heuristic mis-split the segment literally named prompt_char
        // as segment "char"; this pins the collision case.
        with_exec(|| {
            setsparam("POWERLEVEL9K_PROMPT_CHAR_TESTFG7", "31");
            assert_eq!(p9k_param("prompt_char", None, "TESTFG7", "def"), "31");
            unsetparam("POWERLEVEL9K_PROMPT_CHAR_TESTFG7");
        });
    }

    /// p10k:147 — `[[ ${(P)2} == true ]]`: only the literal string
    /// "true" is truthy; unset uses the default.
    #[test]
    fn bool_true_string_only() {
        with_exec(|| {
            setsparam("POWERLEVEL9K_DIR_TESTBOOL1", "true");
            assert!(p9k_param_bool("dir", None, "TESTBOOL1", false));
            setsparam("POWERLEVEL9K_DIR_TESTBOOL1", "false");
            assert!(!p9k_param_bool("dir", None, "TESTBOOL1", true));
            setsparam("POWERLEVEL9K_DIR_TESTBOOL1", "1");
            assert!(!p9k_param_bool("dir", None, "TESTBOOL1", true));
            unsetparam("POWERLEVEL9K_DIR_TESTBOOL1");
            assert!(p9k_param_bool("dir", None, "TESTBOOL1", true));
            assert!(!p9k_param_bool("dir", None, "TESTBOOL1", false));
        });
    }

    /// Array probe follows the same chain; scalar hit wraps to a
    /// one-element vec (`("${(@P)var}")` on a scalar); miss is empty.
    #[test]
    fn param_arr_reads_arrays_and_wraps_scalars() {
        with_exec(|| {
            setaparam(
                "POWERLEVEL9K_DIR_TESTARR1",
                vec!["a".into(), "b c".into()],
            );
            assert_eq!(
                p9k_param_arr("dir", None, "TESTARR1"),
                vec!["a".to_string(), "b c".to_string()]
            );
            unsetparam("POWERLEVEL9K_DIR_TESTARR1");

            setsparam("POWERLEVEL9K_DIR_TESTARR2", "solo");
            assert_eq!(
                p9k_param_arr("dir", None, "TESTARR2"),
                vec!["solo".to_string()]
            );
            unsetparam("POWERLEVEL9K_DIR_TESTARR2");

            unsetparam("POWERLEVEL9K_DIR_TESTARR3");
            unsetparam("POWERLEVEL9K_TESTARR3");
            assert!(p9k_param_arr("dir", None, "TESTARR3").is_empty());
        });
    }

    /// State-specific ARRAY beats segment-level array — probe order
    /// applies to arrays identically (p10k:482 before p10k:486).
    #[test]
    fn param_arr_probe_order() {
        with_exec(|| {
            setaparam("POWERLEVEL9K_VCS_CLEAN_TESTARR4", vec!["x".into()]);
            setaparam("POWERLEVEL9K_VCS_TESTARR4", vec!["y".into()]);
            assert_eq!(
                p9k_param_arr("vcs", Some("CLEAN"), "TESTARR4"),
                vec!["x".to_string()]
            );
            unsetparam("POWERLEVEL9K_VCS_CLEAN_TESTARR4");
            unsetparam("POWERLEVEL9K_VCS_TESTARR4");
        });
    }

    /// p10k:499-505 — plain global read with default.
    #[test]
    fn global_scalar_and_default() {
        with_exec(|| {
            setsparam("POWERLEVEL9K_TESTGLOB1", "nerdfont-complete");
            assert_eq!(p9k_global("TESTGLOB1", "ascii"), "nerdfont-complete");
            unsetparam("POWERLEVEL9K_TESTGLOB1");
            assert_eq!(p9k_global("TESTGLOB1", "ascii"), "ascii");
        });
    }

    /// Global array read: array as-is, scalar wrapped, unset empty.
    #[test]
    fn global_arr_forms() {
        with_exec(|| {
            setaparam(
                "POWERLEVEL9K_TESTGLOB2",
                vec!["dir".into(), "vcs".into()],
            );
            assert_eq!(
                p9k_global_arr("TESTGLOB2"),
                vec!["dir".to_string(), "vcs".to_string()]
            );
            unsetparam("POWERLEVEL9K_TESTGLOB2");

            setsparam("POWERLEVEL9K_TESTGLOB3", "status");
            assert_eq!(p9k_global_arr("TESTGLOB3"), vec!["status".to_string()]);
            unsetparam("POWERLEVEL9K_TESTGLOB3");

            unsetparam("POWERLEVEL9K_TESTGLOB4");
            assert!(p9k_global_arr("TESTGLOB4").is_empty());
        });
    }

    /// The greedy `[a-z0-9_]#` split (p10k:481): `prompt_vcs_CLEAN`
    /// splits as ("vcs_", "CLEAN"), `prompt_dir` as ("dir", "").
    #[test]
    fn split_matches_zsh_pattern() {
        assert_eq!(split_lower_prefix("vcs_CLEAN"), ("vcs_", "CLEAN"));
        assert_eq!(split_lower_prefix("dir"), ("dir", ""));
        assert_eq!(split_lower_prefix("dir_writable"), ("dir_writable", ""));
        assert_eq!(
            split_lower_prefix("dir_NOT_WRITABLE"),
            ("dir_", "NOT_WRITABLE")
        );
    }

    /// Probe-name construction for the stateful and stateless forms.
    #[test]
    fn probe_names_exact_order() {
        assert_eq!(
            probe_names("vcs", Some("CLEAN"), "FOREGROUND"),
            [
                "POWERLEVEL9K_VCS_CLEAN_FOREGROUND".to_string(),
                "POWERLEVEL9K_VCS_FOREGROUND".to_string(),
                "POWERLEVEL9K_FOREGROUND".to_string(),
            ]
        );
        // Stateless: probes 1 and 2 coincide, exactly as zsh probes
        // the same var twice.
        assert_eq!(
            probe_names("dir", None, "BACKGROUND"),
            [
                "POWERLEVEL9K_DIR_BACKGROUND".to_string(),
                "POWERLEVEL9K_DIR_BACKGROUND".to_string(),
                "POWERLEVEL9K_BACKGROUND".to_string(),
            ]
        );
    }
}
