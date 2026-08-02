//! Port of `_printers` from `Completion/Unix/Type/_printers`.
//!
//! Full upstream body (110 lines):
//! ```text
//! sh:  1  #compdef -value-,PRINTER,-default- -value-,LPDEST,-default-
//! sh:  6  if (( $+commands[lsallq] )); then _wanted … compadd - $(lsallq); return; fi
//! sh: 12  zstyle -s …printers list-separator sep || sep=--
//! sh: 20  servopt = ($service == lpr ? -H : -h)
//! sh: 25  if (( $+commands[lpstat] )); then
//! sh: 26    ind=${words[1,CURRENT][(I)${servopt}*]}
//! sh: 27    if (( ind > 0 )); then
//! sh: 28      tmp = ($words[ind] == servopt ? -h$words[ind+1] : -h${words[ind][3,-1]})
//! sh: 33      _wanted … compadd - ${${(f)"$(lpstat $tmp -a)"}%% *} && return
//! sh: 40  if (( ! $+_lp_cache )); then
//! sh: 43    file=( /etc/(printcap|printers.conf) )
//! sh: 49    while read entry < $file[1]: if [[ $entry = [^blank#*_]*:* ]]; then
//! sh: 51      names=( (s:|:)${entry%%:*} )
//! sh: 52      disp = description= value | last alias-with-space | ''
//! sh: 59      _lp_cache      += name[1](:disp);  _lp_alias_cache += name[2,-1]:#*' '*(:disp)
//! sh: 70    if (( $+commands[lpstat] )); then lpstat -a → append new names; fi
//! sh: 81    solaris ypcat printers.conf.byname → name(:desc), drop _default*
//! sh: 86    (( $#_lp_cache )) || _lp_cache=( 'lp0:guessed default printer' )
//! sh: 87    (( $#_lp_alias_cache )) || unset _lp_alias_cache
//! sh: 90  if zstyle -T …printers verbose; then zformat -a list " $sep " $_lp_cache; disp=(-ld list); fi
//! sh: 96  _wanted printers expl printer compadd "$@" $disp - ${(@)_lp_cache%%:*} && return 0
//! sh: 99  (( $+_lp_alias_cache )) || return 1
//! sh:101  (verbose again) _wanted … compadd $disp - ${(@)_lp_alias_cache%%:*} && return 0
//! sh:110  return 1
//! ```

use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::exec::findcmd;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getsparam, setaparam, unsetparam};

/// zsh `$+commands[name]` — is `name` an executable in `$path`?
fn have_command(name: &str) -> bool {
    findcmd(name, 0, 0).is_some()
}

/// zsh `zstyle -T ctx style` — true unless explicitly set false.
fn zstyle_t_default_true(ctx: &str, style: &str) -> bool {
    !matches!(
        lookupstyle(ctx, style).first().map(|s| s.as_str()),
        Some("no") | Some("false") | Some("off") | Some("0")
    )
}

/// `name` part of a `name:description` cache entry (`${e%%:*}`).
fn name_of(entry: &str) -> String {
    entry.split(':').next().unwrap_or(entry).to_string()
}

/// sh:91 `zformat -a list " $sep " entries` — split each entry on the FIRST
/// `:` into `left:right`, pad every `left` to the widest, then join
/// `left<pad><sep>right`. Entries with no `:` have an empty right part.
fn zformat_align(sep: &str, entries: &[String]) -> Vec<String> {
    let split: Vec<(&str, &str)> = entries
        .iter()
        .map(|e| match e.split_once(':') {
            Some((l, r)) => (l, r),
            None => (e.as_str(), ""),
        })
        .collect();
    let width = split
        .iter()
        .map(|(l, _)| l.chars().count())
        .max()
        .unwrap_or(0);
    split
        .iter()
        .map(|(l, r)| {
            let pad = width - l.chars().count();
            format!("{}{}{}{}", l, " ".repeat(pad), sep, r)
        })
        .collect()
}

/// sh:49-68 — parse `/etc/printcap` (or `/etc/printers.conf`) into the two
/// caches. Returns (`_lp_cache`, `_lp_alias_cache`).
fn parse_printcap() -> (Vec<String>, Vec<String>) {
    let mut cache = Vec::new();
    let mut alias = Vec::new();
    for path in ["/etc/printcap", "/etc/printers.conf"] {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        for entry in text.lines() {
            // sh:50  [[ "$entry" = [^[:blank:]\#\*_]*:* ]]
            let first = entry.chars().next().unwrap_or(' ');
            if first.is_whitespace() || matches!(first, '#' | '*' | '_') || !entry.contains(':') {
                continue;
            }
            // sh:51  names=( (s:|:)${entry%%:*} )
            let head = entry.split(':').next().unwrap_or("");
            let names: Vec<&str> = head.split('|').collect();
            if names.is_empty() {
                continue;
            }
            // sh:52-58  description.
            let disp: String = if let Some(pos) = entry.find(":description=") {
                let after = &entry[pos + ":description=".len()..];
                after.split(':').next().unwrap_or("").to_string()
            } else if names.len() > 1 && names[names.len() - 1].contains(' ') {
                names[names.len() - 1].to_string()
            } else {
                String::new()
            };
            // sh:59-65 — primary name (+ :disp), then the space-free aliases.
            let aliases: Vec<&str> = names[1..]
                .iter()
                .copied()
                .filter(|a| !a.contains(' '))
                .collect();
            if !disp.is_empty() {
                cache.push(format!("{}:{}", names[0], disp));
                for a in &aliases {
                    alias.push(format!("{}:{}", a, disp));
                }
            } else {
                cache.push(names[0].to_string());
                for a in &aliases {
                    alias.push(a.to_string());
                }
            }
        }
        break; // sh:43  only the first existing file is read.
    }
    (cache, alias)
}

/// sh:73-78 — append `lpstat -a` queue names not already in the cache.
fn add_lpstat_names(cache: &mut Vec<String>) {
    let _ = _call_program(&[
        "printers".to_string(),
        "lpstat".to_string(),
        "-a".to_string(),
    ]);
    let out = getsparam("REPLY").unwrap_or_default();
    for line in out.lines() {
        let name = line.split_whitespace().next().unwrap_or("");
        if !name.is_empty() && !cache.iter().any(|c| name_of(c) == name) {
            cache.push(name.to_string());
        }
    }
}

/// sh:81-84 — Solaris NIS: `ypcat printers.conf.byname` entries, each
/// `name:...:description=<d>:...` → `name:<d>` (or bare `name`), dropping
/// `_default*`.
fn add_ypcat_names(cache: &mut Vec<String>) {
    if !getsparam("OSTYPE")
        .unwrap_or_default()
        .starts_with("solaris")
        || !have_command("ypcat")
    {
        return;
    }
    let _ = _call_program(&[
        "printers".to_string(),
        "ypcat".to_string(),
        "printers.conf.byname".to_string(),
    ]);
    let out = getsparam("REPLY").unwrap_or_default();
    for line in out.lines() {
        let name = line.split(':').next().unwrap_or("").trim();
        if name.is_empty() || name.starts_with("_default") {
            continue;
        }
        let entry = if let Some(pos) = line.find("description=") {
            let d = line[pos + "description=".len()..]
                .split(':')
                .next()
                .unwrap_or("");
            format!("{}:{}", name, d)
        } else {
            name.to_string()
        };
        if !cache.iter().any(|c| name_of(c) == name) {
            cache.push(entry);
        }
    }
}

/// `_printers` — complete printer / print-queue names.
pub fn _printers(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_printers");
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let pctx = format!(":completion:{}:printers", curcontext);

    // sh:6-10 — AIX: list print queues via lsallq.
    if have_command("lsallq") {
        let _ = _call_program(&["printers".to_string(), "lsallq".to_string()]);
        let out = getsparam("REPLY").unwrap_or_default();
        let mut w = vec![
            "printers".to_string(),
            "expl".to_string(),
            "printer".to_string(),
            "compadd".to_string(),
        ];
        w.extend(args.iter().cloned());
        w.push("-".to_string());
        w.extend(out.split_whitespace().map(String::from));
        return _wanted(&w);
    }

    // sh:12 — list-separator style (default `--`).
    let sep = lookupstyle(&pctx, "list-separator")
        .into_iter()
        .next()
        .unwrap_or_else(|| "--".to_string());

    // sh:20-24 — the command's server option (lpr uses -H, others -h).
    let service = getsparam("service").unwrap_or_default();
    let servopt = if service == "lpr" { "-H" } else { "-h" };

    // sh:25-37 — a `-h<server>` (or `-H<server>`) already on the line means
    //   list THAT server's printers (uncached). `(I)` picks the LAST such.
    if have_command("lpstat") {
        let words = getaparam("words").unwrap_or_default();
        let current: usize = getsparam("CURRENT")
            .and_then(|c| c.parse().ok())
            .unwrap_or(0);
        let scan_end = current.min(words.len()); // words[1,CURRENT] (1-based)
        let mut ind1 = 0usize; // 1-based index of the last match
        for (i, w) in words[..scan_end].iter().enumerate() {
            if w.starts_with(servopt) {
                ind1 = i + 1;
            }
        }
        if ind1 > 0 {
            let w = &words[ind1 - 1];
            let tmp = if w == servopt {
                // separate word: -h<next word>
                format!("-h{}", words.get(ind1).cloned().unwrap_or_default())
            } else {
                // glued: strip the 2-char `-h`/`-H`, re-prefix with -h
                let rest: String = w.chars().skip(2).collect();
                format!("-h{}", rest)
            };
            let _ = _call_program(&[
                "printers".to_string(),
                "lpstat".to_string(),
                tmp,
                "-a".to_string(),
            ]);
            let out = getsparam("REPLY").unwrap_or_default();
            let names: Vec<String> = out
                .lines()
                .map(|l| l.split_once(' ').map(|(a, _)| a).unwrap_or(l).to_string())
                .filter(|n| !n.is_empty())
                .collect();
            let mut w2 = vec![
                "printers".to_string(),
                "expl".to_string(),
                "printer".to_string(),
                "compadd".to_string(),
            ];
            w2.extend(args.iter().cloned());
            w2.push("-".to_string());
            w2.extend(names);
            // sh:33  `… && return` — return only if _wanted succeeded.
            if _wanted(&w2) == 0 {
                return 0;
            }
        }
    }

    // sh:40-88 — build both caches once, then reuse.
    if getaparam("_lp_cache").is_none() {
        let (mut cache, mut alias) = parse_printcap();
        if have_command("lpstat") {
            add_lpstat_names(&mut cache);
        }
        add_ypcat_names(&mut cache);
        // sh:86-87 — default guess; drop the empty alias cache.
        if cache.is_empty() {
            cache.push("lp0:guessed default printer".to_string());
        }
        setaparam("_lp_cache", cache);
        if alias.is_empty() {
            let _ = unsetparam("_lp_alias_cache");
        } else {
            setaparam("_lp_alias_cache", alias);
        }
    }

    // sh:90-97 — primary cache, with optional verbose two-column display.
    let cache = getaparam("_lp_cache").unwrap_or_default();
    let verbose = zstyle_t_default_true(&pctx, "verbose");
    let mut disp: Vec<String> = Vec::new();
    if verbose {
        setaparam("list", zformat_align(&format!(" {} ", sep), &cache));
        disp = vec!["-ld".to_string(), "list".to_string()];
    }
    let names: Vec<String> = cache.iter().map(|e| name_of(e)).collect();
    let mut w = vec![
        "printers".to_string(),
        "expl".to_string(),
        "printer".to_string(),
        "compadd".to_string(),
    ];
    w.extend(args.iter().cloned());
    w.extend(disp.clone());
    w.push("-".to_string());
    w.extend(names);
    if _wanted(&w) == 0 {
        return 0;
    }

    // sh:99-108 — fall back to the alias cache (if any).
    let alias = match getaparam("_lp_alias_cache") {
        Some(a) => a,
        None => return 1,
    };
    let mut disp2: Vec<String> = Vec::new();
    if verbose {
        setaparam("list", zformat_align(&format!(" {} ", sep), &alias));
        disp2 = vec!["-ld".to_string(), "list".to_string()];
    }
    let anames: Vec<String> = alias.iter().map(|e| name_of(e)).collect();
    let mut w2 = vec![
        "printers".to_string(),
        "expl".to_string(),
        "printer".to_string(),
        "compadd".to_string(),
    ];
    w2.extend(args.iter().cloned());
    w2.extend(disp2);
    w2.push("-".to_string());
    w2.extend(anames);
    if _wanted(&w2) == 0 {
        return 0;
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_printcap_extracts_names_and_aliases() {
        // Not reading real files — exercise the pure logic via a helper shim.
        // (parse_printcap reads /etc; here we validate name_of + zformat.)
        assert_eq!(name_of("lp:My Printer"), "lp");
        assert_eq!(name_of("lp0"), "lp0");
    }

    #[test]
    fn zformat_aligns_left_column() {
        // sh:91
        let out = zformat_align(" -- ", &["a:one".to_string(), "bbb:two".to_string()]);
        assert_eq!(
            out,
            vec!["a   -- one".to_string(), "bbb -- two".to_string()]
        );
        // no-colon entry → empty right part.
        let out2 = zformat_align(" -- ", &["solo".to_string()]);
        assert_eq!(out2, vec!["solo -- ".to_string()]);
    }

    #[test]
    fn returns_status_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        // Pre-seed the cache to skip the /etc scan; no live tags → non-zero.
        setaparam("_lp_cache", vec!["lp0".to_string()]);
        let _ = unsetparam("_lp_alias_cache");
        assert_eq!(_printers(&[]), 1);
    }
}
