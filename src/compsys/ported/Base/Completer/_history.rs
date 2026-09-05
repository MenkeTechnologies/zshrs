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
use crate::ported::modules::zutil::{lookupstyle, testforstyle};
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam};
use crate::ported::zle::compcore::get_compstate_str;

/// `_history` — complete from `$historywords` array.
pub fn _history() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_history");
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

    // sh:21
    let remove_all_dups = testforstyle(&ctx, "remove-all-dups") == 0;
    let opt_prefix = if remove_all_dups { "-" } else { "-1" };
    // sh:27
    let sort_on = testforstyle(&ctx, "sort") == 0;
    let opt = if sort_on {
        format!("{}J", opt_prefix)
    } else {
        format!("{}V", opt_prefix)
    };

    // sh:33-40  range style
    let range_val = lookupstyle(&ctx, "range")
        .first()
        .cloned()
        .unwrap_or_default();
    let (mut max, slice): (usize, usize) = if !range_val.is_empty() {
        let (m_str, s_str) = if range_val.contains(':') {
            let mut parts = range_val.splitn(2, ':');
            let m = parts.next().unwrap_or("0").to_string();
            let s = parts.next().unwrap_or("0").to_string();
            (m, s)
        } else {
            (range_val.clone(), range_val)
        };
        let m = m_str.parse::<usize>().unwrap_or(hmax);
        let s = s_str.parse::<usize>().unwrap_or(m);
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
