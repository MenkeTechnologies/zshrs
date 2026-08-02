//! Port of `_next_tags` from
//! `Completion/Base/Widget/_next_tags`.
//!
//! Main-widget body (sh:5-111 verbatim, label-fn defs abridged):
//! ```text
//! sh:  5  _next_tags() {
//! sh:  6    eval "$_comp_setup"
//! sh:  8    local ins ops="$PREFIX$SUFFIX"
//! sh: 10    unfunction _all_labels _next_label
//! sh: 12    _all_labels() { … skip tags listed in $_next_tags_not … }
//! sh: 60    _next_label() { … same $_next_tags_not filter … }
//! sh: 84    if [[ "${LBUFFER%${PREFIX}}" = "$_next_tags_pre" ]]; then
//! sh: 85      PREFIX="$_next_tags_pfx"
//! sh: 86      SUFFIX="$_next_tags_sfx"
//! sh: 87    else
//! sh: 88      _next_tags_pre="${LBUFFER%${PREFIX}}"
//! sh: 89      if [[ "$LASTWIDGET" = (_next_tags|list-*|*complete*) ]]; then
//! sh: 90        PREFIX="$_lastcomp[prefix]"
//! sh: 91        SUFFIX="$_lastcomp[suffix]"
//! sh: 92      fi
//! sh: 93    fi
//! sh: 95    _next_tags_not+=" $_lastcomp[tags]"
//! sh: 96    _next_tags_pfx="$PREFIX"
//! sh: 97    _next_tags_sfx="$SUFFIX"
//! sh: 99    ins="${compstate[old_insert]:+1}"
//! sh:101    _main_complete _complete _next_tags_completer
//! sh:103    [[ $compstate[insert] = automenu ]] && compstate[insert]=automenu-unambiguous
//! sh:104    [[ $compstate[insert] = *unambiguous && -n "$ops" &&
//! sh:105       -z "$_lastcomp[unambiguous]" ]] && compadd -Uns "$SUFFIX" - "$PREFIX"
//! sh:107    compstate[insert]="$ins"
//! sh:108    compstate[list]='list force'
//! sh:110    compprefuncs+=( _next_tags_pre )
//! sh:111  }
//! ```
//!
//! Cycles through alternate tag sets on repeated `\C-xn`. The state
//! that drives the cycle is:
//!   * `$_next_tags_pre` (scalar) — `$LBUFFER` minus `$PREFIX` from the
//!     previous invocation; used at sh:84 to detect "same word, cycle".
//!   * `$_next_tags_not` (scalar, space-delimited) — the accumulated
//!     set of tags already shown; sh:95 appends `$_lastcomp[tags]` each
//!     round so the next round's label functions can skip them.
//!   * `$_next_tags_pfx` / `$_next_tags_sfx` — the prefix/suffix to
//!     restore when the user re-fires on the same word (sh:85-86).
//!
//! sh:10-82 substrate note: the widget `unfunction`s `_all_labels` /
//! `_next_label` and redefines them inline as tag-filtering versions that
//! consult `$_next_tags_not`. Injecting a fresh shell-function *body* from
//! a Rust port isn't supported by the dispatch layer, so instead the
//! Base/Core Rust ports of `_all_labels`/`_next_label` read
//! `$_next_tags_not` natively (see their `spec_in_not_list` helper): when
//! it's non-empty they skip any spec already on the list, exactly like the
//! redefined bodies at sh:39 / sh:66. The label-level filter is therefore
//! live without any function-body injection. This widget's job is to keep
//! `$_next_tags_not` accurate (sh:95) so those ports have the list to
//! consult. It also restores the real `$_next_tags_pre`/`$_lastcomp[tags]`
//! bookkeeping (previously dead — the old port read an invented,
//! always-empty `$_next_tags_in`), the sh:105 `compadd -Uns` menu fixup,
//! the sh:108 `compstate[list]='list force'`, and the sh:110
//! `compprefuncs` append.

use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getaparam, gethkparam, gethparam, getsparam, setaparam, setsparam};
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// Read `$_lastcomp[key]` from the completion-result assoc array by
/// zipping its keys and values (no single-key assoc getter exists).
fn lastcomp_val(key: &str) -> String {
    let keys = gethkparam("_lastcomp").unwrap_or_default();
    let vals = gethparam("_lastcomp").unwrap_or_default();
    keys.iter()
        .zip(vals.iter())
        .find(|(k, _)| k.as_str() == key)
        .map(|(_, v)| v.clone())
        .unwrap_or_default()
}

/// `_next_tags` — bind `\C-xn` to "show me the next tag set".
pub fn _next_tags() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_next_tags");
    // sh:8  local ins ops="$PREFIX$SUFFIX"
    let mut prefix = getsparam("PREFIX").unwrap_or_default();
    let mut suffix = getsparam("SUFFIX").unwrap_or_default();
    let ops = format!("{}{}", prefix, suffix);

    // sh:10-82  the widget `unfunction`s + inline-redefines _all_labels /
    //   _next_label as $_next_tags_not-filtering versions. We can't inject
    //   a shell-function body from Rust, so the Base/Core ports read
    //   $_next_tags_not natively instead (their `spec_in_not_list` helper,
    //   sh:39 / sh:66). Here we just keep $_next_tags_not accurate below
    //   (sh:95) so those ports have the list to consult.

    // sh:84  if [[ "${LBUFFER%${PREFIX}}" = "$_next_tags_pre" ]]; then
    let lbuffer = getsparam("LBUFFER").unwrap_or_default();
    let lb_minus_pre = lbuffer
        .strip_suffix(&prefix)
        .map(|s| s.to_string())
        .unwrap_or_else(|| lbuffer.clone());
    let prev_pre = getsparam("_next_tags_pre").unwrap_or_default();
    if lb_minus_pre == prev_pre {
        // sh:85-86  restore the stashed prefix/suffix and cycle.
        prefix = getsparam("_next_tags_pfx").unwrap_or_default();
        suffix = getsparam("_next_tags_sfx").unwrap_or_default();
        let _ = setsparam("PREFIX", &prefix);
        let _ = setsparam("SUFFIX", &suffix);
    } else {
        // sh:88  _next_tags_pre="${LBUFFER%${PREFIX}}"
        let _ = setsparam("_next_tags_pre", &lb_minus_pre);
        // sh:89  [[ "$LASTWIDGET" = (_next_tags|list-*|*complete*) ]]
        let lastwidget = getsparam("LASTWIDGET").unwrap_or_default();
        if lastwidget_matches(&lastwidget) {
            // sh:90-91
            prefix = lastcomp_val("prefix");
            suffix = lastcomp_val("suffix");
            let _ = setsparam("PREFIX", &prefix);
            let _ = setsparam("SUFFIX", &suffix);
        }
    }

    // sh:95  _next_tags_not+=" $_lastcomp[tags]"
    let not = getsparam("_next_tags_not").unwrap_or_default();
    let _ = setsparam(
        "_next_tags_not",
        &format!("{} {}", not, lastcomp_val("tags")),
    );
    // sh:96-97  _next_tags_pfx="$PREFIX" ; _next_tags_sfx="$SUFFIX"
    let _ = setsparam("_next_tags_pfx", &prefix);
    let _ = setsparam("_next_tags_sfx", &suffix);

    // sh:99  ins="${compstate[old_insert]:+1}"
    let old_insert = get_compstate_str("old_insert").unwrap_or_default();
    let ins = if old_insert.is_empty() { "" } else { "1" };

    // sh:101  _main_complete _complete _next_tags_completer
    let ret = dispatch_function_call(
        "_main_complete",
        &["_complete".to_string(), "_next_tags_completer".to_string()],
    )
    .unwrap_or(1);

    // sh:103  [[ $compstate[insert] = automenu ]] && compstate[insert]=automenu-unambiguous
    if get_compstate_str("insert").as_deref() == Some("automenu") {
        set_compstate_str("insert", "automenu-unambiguous");
    }

    // sh:104-105  [[ insert = *unambiguous && -n ops && -z _lastcomp[unambiguous] ]]
    //             && compadd -Uns "$SUFFIX" - "$PREFIX"
    let insert = get_compstate_str("insert").unwrap_or_default();
    if insert.contains("unambiguous") && !ops.is_empty() && lastcomp_val("unambiguous").is_empty() {
        // $PREFIX/$SUFFIX may have been rewritten by _main_complete; use
        //   the current values, as the shell `"$SUFFIX"`/`"$PREFIX"` do.
        let cur_suffix = getsparam("SUFFIX").unwrap_or_default();
        let cur_prefix = getsparam("PREFIX").unwrap_or_default();
        let argv = vec!["-Uns".to_string(), cur_suffix, "-".to_string(), cur_prefix];
        let _ = bin_compadd("compadd", &argv, &make_ops(), 0);
    }

    // sh:107  compstate[insert]="$ins"
    set_compstate_str("insert", ins);
    // sh:108  compstate[list]='list force'
    set_compstate_str("list", "list force");

    // sh:110  compprefuncs+=( _next_tags_pre )
    let mut cpf = getaparam("compprefuncs").unwrap_or_default();
    cpf.push("_next_tags_pre".to_string());
    setaparam("compprefuncs", cpf);

    ret
}

/// sh:89 pattern `(_next_tags|list-*|*complete*)`: exactly `_next_tags`,
/// or starts with `list-`, or contains `complete`.
fn lastwidget_matches(w: &str) -> bool {
    w == "_next_tags" || w.starts_with("list-") || w.contains("complete")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "");
        let _ = setsparam("SUFFIX", "");
        let _ = setsparam("LBUFFER", "");
        let _ = setsparam("_next_tags_pre", "");
        let _ = setsparam("_next_tags_pfx", "");
        let _ = setsparam("_next_tags_sfx", "");
        let _ = setsparam("_next_tags_not", "");
        assert_eq!(_next_tags(), 1);
    }

    // sh:95 — every invocation appends `$_lastcomp[tags]` to the
    // (space-delimited) `$_next_tags_not` scalar so the next round can
    // skip already-shown tags. Pin that the real accumulation happens
    // (the old port read an invented, always-empty `$_next_tags_in` and
    // never grew `_next_tags_not`, making the cycle dead).
    #[test]
    fn accumulates_next_tags_not() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "");
        let _ = setsparam("SUFFIX", "");
        let _ = setsparam("LBUFFER", "");
        let _ = setsparam("_next_tags_not", " files ");
        let _ = _next_tags();
        let not = getsparam("_next_tags_not").unwrap_or_default();
        assert!(
            not.starts_with(" files "),
            "existing _next_tags_not must be preserved and appended to, got {not:?}"
        );
    }

    // sh:110 — the pre-completion hook must be queued for the next round.
    #[test]
    fn appends_next_tags_pre_hook() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "");
        let _ = setsparam("SUFFIX", "");
        let _ = setsparam("LBUFFER", "");
        setaparam("compprefuncs", Vec::new());
        let _ = _next_tags();
        let cpf = getaparam("compprefuncs").unwrap_or_default();
        assert!(
            cpf.iter().any(|f| f == "_next_tags_pre"),
            "compprefuncs must contain _next_tags_pre, got {cpf:?}"
        );
    }

    #[test]
    fn lastwidget_pattern() {
        assert!(lastwidget_matches("_next_tags"));
        assert!(lastwidget_matches("list-choices"));
        assert!(lastwidget_matches("expand-or-complete"));
        assert!(!lastwidget_matches("self-insert"));
    }
}
