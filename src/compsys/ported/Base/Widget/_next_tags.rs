//! Port of `_next_tags` from
//! `Completion/Base/Widget/_next_tags`.
//!
//! Full upstream body (141 lines, abridged):
//! ```text
//! sh: 1  #compdef -k list-choices \C-xn
//! sh:  5  _next_tags() {
//! sh:  7    local ins ops="$PREFIX$SUFFIX"
//! sh:  9    unfunction _all_labels _next_label
//! sh: 11    _all_labels() { … filter via $_next_tags_not … }
//! sh: 60    _next_label() { … same filter … }
//! sh: 90    ins="${compstate[old_insert]:+1}"
//! sh:100    if [[ "$ops" = "$_next_tags_pfx$_next_tags_sfx" && -n "$_next_tags_in" ]]; then
//! sh:101      _next_tags_not+=("$_next_tags_in")
//! sh:104    else
//! sh:105      _next_tags_pfx="$PREFIX"
//! sh:106      _next_tags_sfx="$SUFFIX"
//! sh:107      _next_tags_not=()
//! sh:108    fi
//! sh:111    _main_complete
//! sh:120    compstate[insert]="${ins}"
//! sh:140  }
//! ```
//!
//! Cycles through alternate tag sets on repeated `\C-xn`. Wraps
//! the dispatch in `_shadow _all_labels _next_label` so the
//! caller-side filter overrides install/restore against the real
//! `shfunctab` (the override bodies themselves stay in shell
//! until the inner-fn rewrite layer exists; the shadow scoping is
//! already correct).

use crate::compsys::ported::_shadow::{_shadow, _unshadow};
use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam};
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};

/// `_next_tags` — bind `\C-xn` to "show me the next tag set".
pub fn _next_tags() -> i32 {
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let suffix = getsparam("SUFFIX").unwrap_or_default();
    let ops = format!("{}{}", prefix, suffix);

    let prev_pfx = getsparam("_next_tags_pfx").unwrap_or_default();
    let prev_sfx = getsparam("_next_tags_sfx").unwrap_or_default();
    let combined_prev = format!("{}{}", prev_pfx, prev_sfx);
    let in_spec = getsparam("_next_tags_in").unwrap_or_default();

    // sh:100  same line as last call?
    if ops == combined_prev && !in_spec.is_empty() {
        let mut not = getaparam("_next_tags_not").unwrap_or_default();
        not.push(in_spec.clone());
        setaparam("_next_tags_not", not);
    } else {
        // sh:105
        let _ = setsparam("_next_tags_pfx", &prefix);
        let _ = setsparam("_next_tags_sfx", &suffix);
        setaparam("_next_tags_not", Vec::new());
    }

    // sh:9 — `unfunction _all_labels _next_label`. We model this
    //   as `_shadow` on both names: snapshot the prior shfunctab
    //   entries (so `_unshadow` restores them), letting any
    //   subsequent override land safely.
    let _ = _shadow(&["_all_labels".to_string(), "_next_label".to_string()]);

    // sh:111
    let ret = dispatch_function_call("_main_complete", &[]).unwrap_or(1);

    // sh:118 — `_unshadow _all_labels _next_label` (implicit via
    //   the shell `local -h` scoping). Single pop covers both.
    let _ = _unshadow();

    // sh:120
    let old_insert = get_compstate_str("old_insert").unwrap_or_default();
    let ins = if old_insert.is_empty() { "" } else { "1" };
    set_compstate_str("insert", ins);
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "");
        let _ = setsparam("SUFFIX", "");
        let _ = setsparam("_next_tags_pfx", "");
        let _ = setsparam("_next_tags_sfx", "");
        let _ = setsparam("_next_tags_in", "");
        assert_eq!(_next_tags(), 1);
    }
}
