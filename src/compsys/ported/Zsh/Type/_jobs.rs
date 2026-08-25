//! Port of `_jobs` from `Completion/Zsh/Type/_jobs`.
//!
//! Full upstream body (84 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 5  if [[ "$1" = -t ]]; then …
//! sh: 8  zstyle -t … prefix-hidden && pfx=''
//! sh: 9  zstyle -T … verbose       && desc=yes
//! sh:11  if [[ "$1" = -r ]]; then jids=( "${(@k)jobstates[(R)running*]}" )
//! sh:15  elif [[ "$1" = -s ]]; then jids=( "${(@k)jobstates[(R)suspended*]}" )
//! sh:18  else jids=( "${(@k)jobtexts}" )
//! sh:25  fi
//! sh:24  if zstyle -T … how-many; then how=$expls fi
//! sh:30  for job in $jids do … build display lines …
//! sh:80  if [[ -n "$desc" ]]; then
//! sh:81    _wanted -V jobs expl "$expls" compadd -d disp "$@" - "$jids[@]"
//! sh:82  else
//! sh:83    _wanted jobs expl "$expls" compadd "$@" "$pfx$jids[@]"
//! sh:84  fi
//! ```
//!
//! Reads `$jobtexts` / `$jobstates` assoc-arrays. Supports `-r`
//! (running only), `-s` (suspended only), `-t` (prefix-needed
//! guard).

use crate::compsys::ported::_wanted::_wanted;
use crate::ported::modules::zutil::testforstyle;
use crate::ported::params::{getaparam, gethkparam, gethparam, getsparam, setaparam};
use crate::ported::zle::compcore::get_compstate_str;

/// Helper: key/value pairs of an associative parameter.
///
/// `$jobtexts` and `$jobstates` are PM_HASHED magic parameters
/// (`${(t)jobtexts}` = `association-readonly-hide-hideval-special`), and
/// `getaparam` only ever returns PM_ARRAY values, so a getaparam-only read
/// came back empty and `_jobs` produced no matches even with a live job:
/// after `sleep 300 &`, `kill %<TAB>` completed to `%sleep` in zsh and to
/// nothing in zshrs. Read the hash the way `_files.rs:68-82` already does —
/// `gethkparam` for the keys, `gethparam` for the values in the same scan
/// order (c:params.c:3131 / c:3117) — and keep the flat key/value-array path
/// as the fallback for assocs staged with `setaparam`.
fn assoc_chunks(name: &str) -> Vec<(String, String)> {
    let keys = gethkparam(name).unwrap_or_default();
    if !keys.is_empty() {
        let vals = gethparam(name).unwrap_or_default();
        return keys
            .into_iter()
            .enumerate()
            .map(|(i, k)| (k, vals.get(i).cloned().unwrap_or_default()))
            .collect();
    }
    let arr = getaparam(name).unwrap_or_default();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < arr.len() {
        out.push((arr[i].clone(), arr[i + 1].clone()));
        i += 2;
    }
    out
}

/// `_jobs` — complete job-id specs from `$jobtexts`/`$jobstates`.
/// `-r` running only; `-s` suspended only; `-t` enables
/// prefix-needed check (returns 1 if not starting with `%`).
pub fn _jobs(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_jobs");
    // sh:3 `local expl disp jobs job jids pfx='%' desc how expls sep`. `disp`
    // is the only one of those the port hands to a builtin BY NAME (compadd
    // -ld), so it is the only one that has to exist in the paramtab — and
    // without the declaration `setaparam` would create it at level 0, where it
    // outlives the completion and `_parameters` then offers `disp` as a match
    // zsh never lists.
    // sh:3 `local expl disp jobs job jids pfx='%' desc how expls sep` —
    // `expl` is published to `_description`/`_wanted` like `disp`, so it
    // needs the same binding or it leaks into the global param table.
    let _locals = crate::compsys::ported::shared::LocalScope::declare(
        &["disp", "expl"],
        crate::ported::zsh_h::PM_ARRAY,
    );
    let mut argv = args.to_vec();
    let curcontext = getsparam("curcontext").unwrap_or_default();

    // sh:5-7  -t prefix-needed guard
    if argv.first().map(|s| s == "-t").unwrap_or(false) {
        // sh:6 is `zstyle -T`, not `-t`: the style defaults to TRUE when it
        // is not set at all. The port tested it with `-t` semantics, so with
        // no `prefix-needed` style in scope — the default — the guard never
        // fired and `_jobs` ran its `compadd` anyway. zsh returns 1 here, so
        // for `- <TAB>` (PREFIX not `%`, matches already added) zshrs issued
        // one compadd call zsh never issues. Same `-T` idiom the `verbose`
        // read below and _expand_alias.rs:113 already use.
        let jobs_ctx = format!(":completion:{}:jobs", curcontext);
        let prefix_needed = testforstyle(&jobs_ctx, "prefix-needed") == 0
            || crate::ported::modules::zutil::lookupstyle(&jobs_ctx, "prefix-needed").is_empty();
        let prefix = getsparam("PREFIX").unwrap_or_default();
        let nm: i64 = get_compstate_str("nmatches")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if prefix_needed && !prefix.starts_with('%') && nm != 0 {
            return 1;
        }
        argv.remove(0);
    }

    // sh:8-9  styles
    let mut pfx = String::from("%");
    if testforstyle(&format!(":completion:{}:jobs", curcontext), "prefix-hidden") == 0 {
        pfx.clear();
    }
    let verbose = testforstyle(&format!(":completion:{}:jobs", curcontext), "verbose") == 0
        || crate::ported::modules::zutil::lookupstyle(
            &format!(":completion:{}:jobs", curcontext),
            "verbose",
        )
        .is_empty();

    // sh:11-21  filter
    let jobstates = assoc_chunks("jobstates");
    let jobtexts = assoc_chunks("jobtexts");
    let (jids, expls): (Vec<String>, String) = match argv.first().map(|s| s.as_str()) {
        Some("-r") => {
            argv.remove(0);
            (
                jobstates
                    .iter()
                    .filter(|(_, v)| v.starts_with("running"))
                    .map(|(k, _)| k.clone())
                    .collect(),
                "running job".to_string(),
            )
        }
        Some("-s") => {
            argv.remove(0);
            (
                jobstates
                    .iter()
                    .filter(|(_, v)| v.starts_with("suspended"))
                    .map(|(k, _)| k.clone())
                    .collect(),
                "suspended job".to_string(),
            )
        }
        _ => (
            jobtexts.iter().map(|(k, _)| k.clone()).collect(),
            "job".to_string(),
        ),
    };

    // NO early return on an empty `jids` here: the shell source has none. It
    // falls through to `_wanted jobs expl "$expls" compadd …` at sh:80-84
    // unconditionally, and `_wanted` registers the description even when the
    // match list that follows is empty — which is how the tag reaches the
    // "No matches for `external command', …, `job', `parameter', …" line that
    // compresult prints when nothing matched. Bailing early skipped the
    // registration, so zshrs's enumeration was missing `job` on every
    // zero-match completion (`qzxfoo <TAB>`, and the parity case
    // `--only cmd_partial --sequences menusel_type_nomatch`). With no jobs the
    // expansion `"%$^jobs[@]"` contributes zero words, so `compadd` still adds
    // nothing and the return value is unchanged.

    let text_of = |job: &str| -> String {
        jobtexts
            .iter()
            .find(|(k, _)| k == job)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };

    // sh:27-34 — the verbose display column: `${pfx}${(r:2:: :)job} $sep
    // ${(r:COLUMNS-8:: :)jobtexts[$job]}`.
    let mut disp: Vec<String> = Vec::new();
    if verbose {
        let sep = {
            let s = crate::ported::modules::zutil::lookupstyle(
                &format!(":completion:{}:jobs", curcontext),
                "list-separator",
            );
            if s.is_empty() {
                "--".to_string() // sh:29 `|| sep=--`
            } else {
                s.join(" ")
            }
        };
        let cols = crate::ported::params::getiparam("COLUMNS").max(9) as usize;
        for job in &jids {
            disp.push(format!(
                "{}{:<2} {} {:<width$}",
                pfx,
                job,
                sep,
                text_of(job),
                width = cols - 8
            ));
        }
    }

    // sh:36 — `zstyle -s ":completion:${curcontext}:jobs" numbers how`.
    let how = crate::ported::modules::zutil::lookupstyle(
        &format!(":completion:{}:jobs", curcontext),
        "numbers",
    )
    .join(" ");

    // sh:38-77 — what actually gets added: the job NUMBERS when the `numbers`
    // style says so, otherwise the shortest unambiguous PREFIX of each job's
    // command text. The port used to add the numbers unconditionally (and, in
    // the verbose branch, without the `%`), so with one running `sleep 300`,
    // `kill %<TAB>` completed to `%sleep` in zsh and to nothing in zshrs — the
    // bare `1` did not even match the typed `%`.
    let mut jobs: Vec<String>;
    if matches!(how.as_str(), "yes" | "true" | "on" | "1") {
        jobs = jids.clone(); // sh:39
    } else {
        // sh:41-71 — grow each string one word at a time while two or more job
        // texts still match it, tracking the worst word count in `max`.
        let texts: Vec<String> = jobtexts.iter().map(|(_, v)| v.clone()).collect();
        let mut max = 0usize; // sh:41 `max=0`
        jobs = Vec::new(); // sh:46
        for i in &jids {
            let mut text = text_of(i);
            let mut s = text.split(' ').next().unwrap_or("").to_string(); // sh:49
            text = match text.split_once(' ') {
                Some((_, rest)) => rest.to_string(), // sh:51
                None => String::new(),               // sh:53
            };
            // sh:55 `tmp=( "${(@M)texts:#${str}*}" )` — an unquoted pattern, so
            // glob characters in a job's text are live, exactly as in `_dispatch`.
            let matching = |s: &str| -> usize {
                let pat = format!("{}*", s);
                texts
                    .iter()
                    .filter(|t| {
                        match crate::ported::pattern::patcompile(
                            &{
                                let mut tok = pat.clone();
                                crate::ported::glob::tokenize(&mut tok);
                                tok
                            },
                            0,
                            None,
                        ) {
                            Some(prog) => crate::ported::pattern::pattry(&prog, t),
                            None => t.starts_with(s),
                        }
                    })
                    .count()
            };
            let mut tmp = matching(&s);
            let mut num = 1usize; // sh:56
            while !text.is_empty() && tmp >= 2 {
                // sh:57
                s = format!("{} {}", s, text.split(' ').next().unwrap_or("")); // sh:58
                text = match text.split_once(' ') {
                    Some((_, rest)) => rest.to_string(), // sh:60
                    None => String::new(),               // sh:62
                };
                tmp = matching(&s); // sh:64
                num += 1; // sh:65
            }
            if num > max {
                max = num; // sh:68
            }
            jobs.push(s); // sh:70
        }
        // sh:73-77 — too many words to be useful: fall back to the numbers.
        let how_num = if !how.is_empty() && how.chars().all(|c| c.is_ascii_digit()) {
            how.parse::<usize>().ok()
        } else {
            None
        };
        match how_num {
            Some(n) if max > n => jobs = jids.clone(), // sh:74
            _ => {
                if pfx.is_empty() && verbose {
                    // sh:76 `disp=( "${(@)disp#%}" )`
                    disp = disp
                        .iter()
                        .map(|d| d.strip_prefix('%').unwrap_or(d).to_string())
                        .collect();
                }
            }
        }
    }

    // sh:80-84 — the `%` on the added matches is literal in BOTH branches;
    // `pfx` only ever affects the display column above.
    if verbose {
        setaparam("disp", disp);
        // sh:81 `_wanted jobs expl "$expls" compadd "$@" -ld disp - "%$^jobs[@]"`
        let mut w_args: Vec<String> = vec![
            "jobs".to_string(),
            "expl".to_string(),
            expls,
            "compadd".to_string(),
        ];
        w_args.extend(argv);
        w_args.push("-ld".to_string());
        w_args.push("disp".to_string());
        w_args.push("-".to_string());
        for j in &jobs {
            w_args.push(format!("%{}", j));
        }
        _wanted(&w_args)
    } else {
        // sh:83 `_wanted jobs expl "$expls" compadd "$@" - "%$^jobs[@]"`
        let mut w_args: Vec<String> = vec![
            "jobs".to_string(),
            "expl".to_string(),
            expls,
            "compadd".to_string(),
        ];
        w_args.extend(argv);
        w_args.push("-".to_string());
        for j in &jobs {
            w_args.push(format!("%{}", j));
        }
        _wanted(&w_args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn empty_jobs_returns_one() {
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        setaparam("jobtexts", Vec::new());
        setaparam("jobstates", Vec::new());
        assert_eq!(_jobs(&[]), 1);
        INCOMPFUNC.store(0, Ordering::Relaxed);
    }
}
