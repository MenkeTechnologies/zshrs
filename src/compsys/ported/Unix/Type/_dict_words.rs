//! Port of `_dict_words` from `Completion/Unix/Type/_dict_words`.
//!
//! Full upstream body (43 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local begin end ret=1; local -a args dict dicts dictwords expl
//! sh: 5  [[ $service = dict ]] && args=( ${(kv)opt_args[(I)-([hpdauk]|--(host|port|database|noauth|user|key))]} )
//! sh:10  if   [[ -z $words[CURRENT] ]]; then _message -e dict 'dictionary word'; return 1
//! sh:12  elif [[ -z $SUFFIX ]];        then dictwords=( ${(z)${(f)"$(_call_program words dict $args -m -s prefix $PREFIX)"}} )
//! sh:14  elif [[ -z $PREFIX ]];        then dictwords=( ${(z)${(f)"$(_call_program words dict $args -m -s suffix $SUFFIX)"}} )
//! sh:16  else                               dictwords=( ${(z)${(f)"$(_call_program words dict $args -m -s regexp $PREFIX.\*$SUFFIX)"}} )
//! sh:18  fi
//! sh:20  dictwords=( ${${dictwords#\"}%\"} )      # strip surrounding quotes
//! sh:21  dicts=( ${${(M)dictwords:#*:}%:} )       # section headers (`name:`)
//! sh:23  if zstyle -t …:words separate-sections; then
//! sh:24    _tags words.$^dicts; while _tags; do for dict in $dicts; do
//! sh:26      _requested words.$dict expl "word from $dict" && { slice dictwords, compadd }
//! sh:38  else _wanted words expl word compadd -M '…' "$@" - ${dictwords:#*:}; fi
//! ```
//!
//! sh:5 approx — the `-([hpdauk]|--…)` `opt_args` slice is reproduced by
//! selecting matching keys from the flat `opt_args` assoc. sh:12-16 approx
//! — the `${(z)${(f)…}}` line-then-word split uses whitespace splitting.

use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_message::message_byname;
use crate::compsys::ported::_requested::requested_byname;
use crate::compsys::ported::_tags::tags_byname;
use crate::compsys::ported::_wanted::wanted_byname;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getsparam, setaparam};
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

/// sh:5 approx — flatten the `-h/-p/-d/-a/-u/-k` (and long-form) entries of
/// the `opt_args` assoc into `key value` pairs.
fn dict_client_args() -> Vec<String> {
    let want = |k: &str| -> bool {
        matches!(
            k,
            "-h" | "-p"
                | "-d"
                | "-a"
                | "-u"
                | "-k"
                | "--host"
                | "--port"
                | "--database"
                | "--noauth"
                | "--user"
                | "--key"
        )
    };
    let flat = getaparam("opt_args").unwrap_or_default();
    let mut out = Vec::new();
    for kv in flat.chunks(2) {
        if let Some(k) = kv.first() {
            if want(k) {
                out.push(k.clone());
                if let Some(v) = kv.get(1) {
                    if !v.is_empty() {
                        out.push(v.clone());
                    }
                }
            }
        }
    }
    out
}

/// `_dict_words` — complete words via a running `dict` server (`dict -m`).
pub fn _dict_words(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_dict_words");
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let wctx = format!(":completion:{}:words", curcontext);

    // sh:5
    let client_args = if getsparam("service").as_deref() == Some("dict") {
        dict_client_args()
    } else {
        Vec::new()
    };

    // sh:10 — the current word.
    let current: usize = getsparam("CURRENT")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let words = getaparam("words").unwrap_or_default();
    let cur_word = current
        .checked_sub(1)
        .and_then(|i| words.get(i))
        .cloned()
        .unwrap_or_default();
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let suffix = getsparam("SUFFIX").unwrap_or_default();

    if cur_word.is_empty() {
        // sh:10-11
        let _ = message_byname(&[
            "-e".to_string(),
            "dict".to_string(),
            "dictionary word".to_string(),
        ]);
        return 1;
    }

    // sh:12-18 — query mode depends on which side is empty.
    let (strat, pat) = if suffix.is_empty() {
        ("prefix", prefix.clone())
    } else if prefix.is_empty() {
        ("suffix", suffix.clone())
    } else {
        ("regexp", format!("{}.*{}", prefix, suffix))
    };
    let mut cp: Vec<String> = vec!["words".to_string(), "dict".to_string()];
    cp.extend(client_args);
    cp.push("-m".to_string());
    cp.push("-s".to_string());
    cp.push(strat.to_string());
    cp.push(pat);
    let _ = _call_program(&cp);
    // sh:12-16 approx — (f) lines then (z) words.
    let mut dictwords: Vec<String> = getsparam("REPLY")
        .unwrap_or_default()
        .split_whitespace()
        .map(String::from)
        .collect();

    // sh:20 — strip a surrounding pair of `"`.
    for w in dictwords.iter_mut() {
        let t = w.strip_prefix('"').unwrap_or(w);
        let t = t.strip_suffix('"').unwrap_or(t);
        *w = t.to_string();
    }
    // sh:21 — section headers are the `name:` entries.
    let dicts: Vec<String> = dictwords
        .iter()
        .filter(|w| w.ends_with(':'))
        .map(|w| w.trim_end_matches(':').to_string())
        .collect();

    // sh:23 — separate-sections style groups words under their dictionary.
    let separate = matches!(
        lookupstyle(&wctx, "separate-sections")
            .first()
            .map(|s| s.as_str()),
        Some("yes") | Some("true") | Some("on") | Some("1")
    );
    if separate {
        // sh:24  _tags words.$^dicts
        let tag_names: Vec<String> = dicts.iter().map(|d| format!("words.{}", d)).collect();
        let _ = tags_byname(&tag_names);
        let mut ret = 1;
        // sh:24  while _tags; do
        while tags_byname(&[]) == 0 {
            // sh:25  for dict in $dicts
            for dict in &dicts {
                // sh:26  _requested words.$dict expl "word from $dict"
                if requested_byname(&[
                    format!("words.{}", dict),
                    "expl".to_string(),
                    format!("word from {}", dict),
                ]) == 0
                {
                    // sh:28-30 — slice dictwords between this header and the next.
                    let header = format!("{}:", dict);
                    let begin = dictwords.iter().position(|w| w == &header).map(|i| i + 1);
                    if let Some(begin) = begin {
                        let end = dictwords[begin..]
                            .iter()
                            .position(|w| w.ends_with(':'))
                            .map(|off| begin + off)
                            .unwrap_or(dictwords.len());
                        let expl = getaparam("expl").unwrap_or_default();
                        let mut cadd: Vec<String> = expl;
                        cadd.extend(args.iter().cloned());
                        cadd.push("-M".to_string());
                        cadd.push("m:{a-zA-Z}={A-Za-z} r:|=*".to_string());
                        cadd.push("--".to_string());
                        cadd.extend(dictwords[begin..end].iter().cloned());
                        if bin_compadd("compadd", &cadd, &make_ops(), 0) == 0 {
                            ret = 0;
                        }
                    }
                }
            }
            // sh:34  (( ret )) || break
            if ret == 0 {
                break;
            }
        }
        // sh:37
        return 1;
    }

    // sh:38  _wanted words expl word compadd -M '…' "$@" - ${dictwords:#*:}
    let plain: Vec<String> = dictwords
        .into_iter()
        .filter(|w| !w.ends_with(':'))
        .collect();
    setaparam("dictwords", plain);
    let mut wanted_argv: Vec<String> = vec![
        "words".to_string(),
        "expl".to_string(),
        "word".to_string(),
        "compadd".to_string(),
        "-M".to_string(),
        "m:{a-zA-Z}={A-Za-z} r:|=*".to_string(),
    ];
    wanted_argv.extend(args.iter().cloned());
    wanted_argv.push("-".to_string());
    wanted_argv.push("-a".to_string());
    wanted_argv.push("dictwords".to_string());
    wanted_byname(&wanted_argv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_current_word_messages_and_returns_one() {
        // sh:10-11 — no word under the cursor.
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(1, std::sync::atomic::Ordering::Relaxed);
        setaparam("words", vec!["dict".to_string(), String::new()]);
        let _ = crate::ported::params::setsparam("CURRENT", "2");
        let r = _dict_words(&[]);
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(r, 1);
    }
}
