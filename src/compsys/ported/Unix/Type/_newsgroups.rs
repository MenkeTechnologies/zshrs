//! Port of `_newsgroups` from `Completion/Unix/Type/_newsgroups`.
//!
//! Full upstream body (9 lines verbatim):
//! ```text
//! sh:1  #autoload
//! sh:3  local expl
//! sh:5  : ${(A)_cache_newsgroups:=${${(f)"$(fgrep -vh \! ~/.newsrc*)"}%:*}}
//! sh:7  (( ${(w)#_cache_newsgroups} )) && _wanted newsgroups expl 'newsgroup' \
//! sh:8      _multi_parts "$@" -i . _cache_newsgroups
//! ```
//!
//! sh:5 reads every `~/.newsrc*`, drops lines containing `!`
//! (unsubscribed), and keeps the group name (text before the first
//! `:`).

use crate::compsys::ported::_wanted::wanted_byname;
use crate::ported::params::{getaparam, setaparam};

/// sh:5 — build `_cache_newsgroups` from the `~/.newsrc*` files.
fn parse_newsrc() -> Vec<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    if home.is_empty() {
        return Vec::new();
    }
    let mut groups = Vec::new();
    // ~/.newsrc* — glob the newsrc files.
    if let Ok(rd) = std::fs::read_dir(&home) {
        let mut files: Vec<std::path::PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with(".newsrc"))
                    .unwrap_or(false)
            })
            .collect();
        files.sort();
        for f in files {
            let text = std::fs::read_to_string(&f).unwrap_or_default();
            for line in text.lines() {
                // fgrep -vh \! — skip unsubscribed groups.
                if line.contains('!') {
                    continue;
                }
                // %:* — the group name before the first `:`.
                let name = line.split(':').next().unwrap_or("");
                if !name.is_empty() {
                    groups.push(name.to_string());
                }
            }
        }
    }
    groups
}

/// `_newsgroups` — complete newsgroup names from `~/.newsrc*`.
pub fn _newsgroups(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_newsgroups");
    // sh:5-6 — populate/reuse the `_cache_newsgroups` param.
    if getaparam("_cache_newsgroups").is_none() {
        setaparam("_cache_newsgroups", parse_newsrc());
    }
    let cache = getaparam("_cache_newsgroups").unwrap_or_default();
    // sh:7  (( ${(w)#_cache_newsgroups} )) — nonzero word count guard.
    if cache.is_empty() {
        return 1;
    }
    // sh:7-8  _wanted newsgroups expl 'newsgroup' _multi_parts "$@" -i . _cache_newsgroups
    let mut w = vec![
        "newsgroups".to_string(),
        "expl".to_string(),
        "newsgroup".to_string(),
        "_multi_parts".to_string(),
    ];
    w.extend(args.iter().cloned());
    w.push("-i".to_string());
    w.push(".".to_string());
    w.push("_cache_newsgroups".to_string());
    // `_multi_parts` action is dispatched through _wanted's action-runner.
    wanted_byname(&w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_with_empty_cache() {
        let _g = crate::test_util::global_state_lock();
        setaparam("_cache_newsgroups", Vec::new());
        assert_eq!(_newsgroups(&[]), 1);
    }
}
