//! Port of `_history` from `Completion/Base/Completer/_history`.
//!
//! Full upstream body (65 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh:19  local opt expl max slice hmax=$#historywords beg=2
//! sh:21  if zstyle -t … remove-all-dups; then opt=-  else opt=-1  fi
//! sh:27  if zstyle -t … sort; then opt+=J else opt+=V fi
//! sh:33  if zstyle -s … range max; then …
//! sh:46  PREFIX="$IPREFIX$PREFIX"
//! sh:48  SUFFIX="$SUFFIX$ISUFFIX"
//! sh:55  while [[ $compstate[nmatches] -eq 0 && beg -lt max ]]; do
//! sh:56    if [[ -n $compstate[quote] ]]
//! sh:57    then hslice=( ${(Q)historywords[beg,beg+slice]} )
//! sh:58    else hslice=( ${historywords[beg,beg+slice]} )
//! sh:59    fi
//! sh:60    _wanted "$opt" history-words expl 'history word' \
//! sh:61        compadd -Q -a hslice
//! sh:62    (( beg+=slice ))
//! sh:63  done
//! sh:65  (( $compstate[nmatches] ))
//! ```

use crate::compsys::ported::_wanted::_wanted;
use crate::compsys::ported::shared::zstyle_t;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam};
use crate::ported::zle::compcore::get_compstate_str;

/// `_history` — complete from `$historywords` array.
pub fn _history() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_history");
    // sh:19 — `local opt expl max slice hmax=$#historywords beg=2`.
    //
    // This port does not assign `expl` itself; it hands the NAME to
    // `_wanted`/`_description`, and `_description` fills it through
    // `setaparam` — `createparam(name, PM_SCALAR)` with no PM_LOCAL
    // (shared.rs:16-30). The array was therefore born at level 0 and
    // `endparamscope` had nothing to unwind, so one TAB left `expl`
    // in the user's shell. Measured through a pty, `${(t)expl}` and
    // the value read back at the next prompt after a single TAB on a
    // `_history` wrapper:
    //
    //   zsh  : [][]
    //   zshrs: [array][-J|-default-]
    //
    // kind 0, as sh:19 spells a bare `local`.
    crate::compsys::ported::shared::declare_locals(&["expl"], 0);
    // sh:54 — `local -a hslice`.
    //
    // `hslice` is the window of `$historywords` this loop hands to
    // `_wanted` by name (sh:60-63). The port writes it as a shell
    // parameter, so without the declaration a completion that fell through
    // to `_history` left the slice standing in the user's shell:
    //
    //   zsh  : hslice=[][0]        zshrs: hslice=[array][3]
    //
    // `historywords`, also written here, is deliberately NOT declared: it
    // is the zsh/parameter special the completion widget publishes, not a
    // scratch name of this function.
    crate::compsys::ported::shared::declare_locals(
        &["hslice"],
        crate::compsys::ported::shared::PM_ARRAY,
    );
    let historywords = getaparam("historywords").unwrap_or_default();
    let hmax = historywords.len();
    let mut beg = 2usize;

    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctx = format!(":completion:{}:", curcontext);

    // sh:21 — `zstyle -t`, a VALUE test; see [`zstyle_t`].
    let remove_all_dups = zstyle_t(&ctx, "remove-all-dups") == 0;
    let opt_prefix = if remove_all_dups { "-" } else { "-1" };
    // sh:27 — `zstyle -t`, a VALUE test; see [`zstyle_t`].
    let sort_on = zstyle_t(&ctx, "sort") == 0;
    let opt = if sort_on {
        format!("{}J", opt_prefix)
    } else {
        format!("{}V", opt_prefix)
    };

    // sh:33-44  if zstyle -s ":completion:${curcontext}:" range max; then … else
    //              max=$hmax; slice=$max; fi
    //
    // The branch is `zstyle -s`'s STATUS (`zutil.c:648` tests `vals[0]`, a
    // pointer), not whether the value is non-empty. `range ''` is SET: sh:34's
    // `*:*` test fails, `slice=$max=""`, sh:40's `max -gt hmax` evaluates the
    // empty string as arithmetic 0 — so the search window is ZERO and the
    // completer offers nothing. Reading it as "unset" took the sh:42-43 arm
    // instead and searched the whole history. `zutil.c:649` also joins the
    // whole value array before the `max:slice` split.
    let (mut max, slice): (usize, usize) = if let Some(range_val) =
        crate::compsys::ported::shared::zstyle_s(&ctx, "range")
    {
        let (m_str, s_str) = if range_val.contains(':') {
            let mut parts = range_val.splitn(2, ':');
            let m = parts.next().unwrap_or("0").to_string();
            let s = parts.next().unwrap_or("0").to_string();
            (m, s)
        } else {
            (range_val.clone(), range_val)
        };
        // sh:40 `[[ max -gt hmax ]]` and sh:50's `beg < max` are ARITHMETIC
        // contexts. zsh evaluates a non-numeric word there as a parameter
        // name, and an empty or unset one is 0 — so an unparseable `range`
        // gives a zero-width window, not the whole history.
        let m = m_str.parse::<usize>().unwrap_or(0);
        let s = s_str.parse::<usize>().unwrap_or(0);
        (m.min(hmax), s)
    } else {
        (hmax, hmax)
    };
    if max > hmax {
        max = hmax;
    }

    // sh:42-46  flatten quoted/unquoted PREFIX/IPREFIX semantics
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let iprefix = getsparam("IPREFIX").unwrap_or_default();
    let _ = setsparam("PREFIX", &format!("{}{}", iprefix, prefix));
    let _ = setsparam("IPREFIX", "");
    let suffix = getsparam("SUFFIX").unwrap_or_default();
    let isuffix = getsparam("ISUFFIX").unwrap_or_default();
    let _ = setsparam("SUFFIX", &format!("{}{}", suffix, isuffix));
    let _ = setsparam("ISUFFIX", "");

    // sh:50-59  walk slices until nmatches > 0 or beg >= max.
    while get_compstate_str("nmatches").and_then(|s| s.parse::<i64>().ok()) == Some(0) && beg < max
    {
        let end = (beg + slice).min(hmax);
        let hslice: Vec<String> = if beg <= end && beg >= 1 && end <= historywords.len() {
            historywords[beg - 1..end].to_vec()
        } else {
            Vec::new()
        };
        setaparam("hslice", hslice);
        let _ = _wanted(&[
            opt.clone(),
            "history-words".to_string(),
            "expl".to_string(),
            "history word".to_string(),
            "compadd".to_string(),
            "-Q".to_string(),
            "-a".to_string(),
            "hslice".to_string(),
        ]);
        beg += slice;
        if slice == 0 {
            break;
        }
    }

    // sh:65
    let nm: i64 = get_compstate_str("nmatches")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if nm == 0 {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_historywords_returns_one() {
        let _g = crate::test_util::global_state_lock();
        // `compstate[nmatches]` is a LIVE GSU integer (complete.c:1411)
        // backed by the `nmatches` counter — writing it through
        // `set_compstate_str` is a no-op, the reader always consults the
        // counter. Zero the counter itself so a non-zero count left by
        // an earlier completion test doesn't turn "no matches" into 0.
        crate::ported::zle::compcore::nmatches.store(0, std::sync::atomic::Ordering::Relaxed);
        setaparam("historywords", Vec::new());
        assert_eq!(_history(), 1);
    }
}
