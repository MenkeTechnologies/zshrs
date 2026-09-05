//! Port of `_dict_words` from `Completion/Unix/Type/_dict_words`.
//!
//! Full upstream body (43 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local begin end ret=1; local -a args dict dicts dictwords expl
//! sh: 7  [[ $service = dict ]] && args=( ${(kv)opt_args[(I)-([hpdauk]|--(host|port|database|noauth|user|key))]} )
//! sh:10  if   [[ -z $words[CURRENT] ]]; then _message -e dict 'dictionary word'; return 1
//! sh:13  elif [[ -z $SUFFIX ]];        then dictwords=( ${(z)${(f)"$(_call_program words dict $args -m -s prefix $PREFIX)"}} )
//! sh:15  elif [[ -z $PREFIX ]];        then dictwords=( ${(z)${(f)"$(_call_program words dict $args -m -s suffix $SUFFIX)"}} )
//! sh:17  else                               dictwords=( ${(z)${(f)"$(_call_program words dict $args -m -s regexp $PREFIX.\*$SUFFIX)"}} )
//! sh:19  fi
//! sh:21  dictwords=( ${${dictwords#\"}%\"} )      # strip surrounding quotes
//! sh:22  dicts=( ${${(M)dictwords:#*:}%:} )       # section headers (`name:`)
//! sh:24  if zstyle -t …:words separate-sections; then
//! sh:25    _tags words.$^dicts; while _tags; do for dict in $dicts; do
//! sh:28      _requested words.$dict expl "word from $dict" && { slice dictwords, compadd }
//! sh:40  else _wanted words expl word compadd -M '…' "$@" - ${dictwords:#*:}; fi
//! ```
//!
//! sh:7 approx — the `-([hpdauk]|--…)` `opt_args` slice is reproduced by
//! selecting matching keys from the flat `opt_args` assoc. sh:14-18 approx
//! — the `${(z)${(f)…}}` line-then-word split uses whitespace splitting.

use crate::compsys::ported::_call_program::call_program_capture;
use crate::compsys::ported::_message::_message;
use crate::compsys::ported::_requested::_requested;
use crate::compsys::ported::_tags::_tags;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getsparam};
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

/// sh:7 approx — flatten the `-h/-p/-d/-a/-u/-k` (and long-form) entries of
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

    // sh:7
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
        let _ = _message(&[
            "-e".to_string(),
            "dict".to_string(),
            "dictionary word".to_string(),
        ]);
        return 1;
    }

    // sh:13-18 — query mode depends on which side is empty.
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
    let _ = call_program_capture(&cp);
    // sh:14-18 approx — (f) lines then (z) words.
    let mut dictwords: Vec<String> = getsparam("REPLY")
        .unwrap_or_default()
        .split_whitespace()
        .map(String::from)
        .collect();

    // sh:21 — strip a surrounding pair of `"`.
    for w in dictwords.iter_mut() {
        let t = w.strip_prefix('"').unwrap_or(w);
        let t = t.strip_suffix('"').unwrap_or(t);
        *w = t.to_string();
    }
    // sh:22 — section headers are the `name:` entries.
    let dicts: Vec<String> = dictwords
        .iter()
        .filter(|w| w.ends_with(':'))
        .map(|w| w.trim_end_matches(':').to_string())
        .collect();

    // sh:24 — separate-sections style groups words under their dictionary.
    let separate = matches!(
        lookupstyle(&wctx, "separate-sections")
            .first()
            .map(|s| s.as_str()),
        Some("yes") | Some("true") | Some("on") | Some("1")
    );
    if separate {
        // sh:25  _tags words.$^dicts
        let tag_names: Vec<String> = dicts.iter().map(|d| format!("words.{}", d)).collect();
        let _ = _tags(&tag_names);
        let mut ret = 1;
        // sh:26  while _tags; do
        while _tags(&[]) == 0 {
            // sh:27  for dict in $dicts
            for dict in &dicts {
                // sh:28  _requested words.$dict expl "word from $dict"
                if _requested(&[
                    format!("words.{}", dict),
                    "expl".to_string(),
                    format!("word from {}", dict),
                ]) == 0
                {
                    // sh:29-31 — slice dictwords between this header and the next.
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
            // sh:36  (( ret )) || break
            if ret == 0 {
                break;
            }
        }
        // sh:39
        return 1;
    }

    // sh:41  _wanted words expl word compadd -M '…' "$@" - ${dictwords:#*:}
    //
    //   The words are SPLICED after the bare `-`, exactly as upstream writes
    //   them. `-` is compadd's end-of-options marker (`Src/Zle/complete.c`
    //   c:635-637, `if (!(*argv)[1]) { argv++; break; }`), so everything
    //   after it is a literal candidate. The port used to pass
    //   `- -a dictwords`, which therefore offered the two strings `-a` and
    //   `dictwords` as matches instead of the words — and offered them even
    //   when the `dict` query returned nothing at all. The sibling arm at
    //   sh:32 gets the order right (`-a -` then the array name), which is
    //   what `-a` requires: the flag must precede the terminator.
    //   `_all_labels`:13 (`__tmp=${argv[(ib:4:)-]}`) splices `$expl[@]`
    //   in BEFORE the `-`, so the description options are still options.
    let plain: Vec<String> = dictwords
        .into_iter()
        .filter(|w| !w.ends_with(':'))
        .collect();
    _wanted(&wanted_argv(args, &plain))
}

/// sh:41-42 — the `_wanted words expl word compadd …` argv.
///
/// Split out so the placement of the bare `-` can be pinned by a test: the
/// candidates follow it and nothing else may.
fn wanted_argv(args: &[String], plain: &[String]) -> Vec<String> {
    let mut argv: Vec<String> = vec![
        "words".to_string(),
        "expl".to_string(),
        "word".to_string(),
        "compadd".to_string(),
        "-M".to_string(),
        "m:{a-zA-Z}={A-Za-z} r:|=*".to_string(),
    ];
    argv.extend(args.iter().cloned());
    argv.push("-".to_string());
    argv.extend(plain.iter().cloned());
    argv
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_current_word_messages_and_returns_one() {
        // sh:10-11 — no word under the cursor.
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(1, std::sync::atomic::Ordering::Relaxed);
        crate::ported::params::setaparam("words", vec!["dict".to_string(), String::new()]);
        let _ = crate::ported::params::setsparam("CURRENT", "2");
        let r = _dict_words(&[]);
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(r, 1);
    }

    /// sh:41 — everything after compadd's bare `-` is a literal candidate
    /// (`Src/Zle/complete.c` c:635-637 consumes the `-` and leaves the option
    /// loop), so the words have to be spliced there and nothing else may.
    ///
    /// The regression this pins: the port used to emit `- -a dictwords`,
    /// which made compadd offer the two strings `-a` and `dictwords`.
    /// Measured under a pty against `zsh -f -i`, `dict` absent from the host
    /// so the query returns nothing:
    ///
    /// ```text
    ///   <cmd> d<TAB>   zsh -> d            (no matches)
    ///                  was -> dictwords
    /// ```
    #[test]
    fn candidates_follow_the_bare_dash_and_no_options_do() {
        let argv = wanted_argv(&[], &["alpha".to_string(), "beta".to_string()]);
        let dash = argv
            .iter()
            .position(|w| w == "-")
            .expect("sh:41 writes a bare `-` before the candidates");
        assert_eq!(
            &argv[dash + 1..],
            &["alpha".to_string(), "beta".to_string()]
        );
        assert!(
            !argv[dash + 1..].iter().any(|w| w.starts_with('-')),
            "an option after `-` becomes a MATCH: {:?}",
            &argv[dash + 1..]
        );
        // sh:41's fixed head, in order: the _wanted tag/name/descr, then the
        // action word, then the matcher spec.
        assert_eq!(&argv[0..4], &["words", "expl", "word", "compadd"]);
        assert_eq!(argv[4], "-M");
    }

    /// sh:41 — an empty word list must offer NOTHING. With `- -a dictwords`
    /// the port added two matches even when `dict` produced no output at all,
    /// which is how the defect showed up on a host with no `dict` installed.
    #[test]
    fn no_words_means_nothing_after_the_dash() {
        let argv = wanted_argv(&[], &[]);
        assert_eq!(argv.last().map(String::as_str), Some("-"));
    }

    /// sh:41 — the caller's `"$@"` sits between the matcher and the `-`, so
    /// its options stay options.
    #[test]
    fn caller_args_precede_the_dash() {
        let argv = wanted_argv(&["-S".to_string(), String::new()], &["w".to_string()]);
        let dash = argv.iter().position(|w| w == "-").unwrap();
        assert_eq!(argv[dash - 2], "-S");
        assert_eq!(argv[dash + 1], "w");
    }
}
