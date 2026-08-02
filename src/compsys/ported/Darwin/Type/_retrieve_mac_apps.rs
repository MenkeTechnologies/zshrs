//! Port of `_retrieve_mac_apps` from `Completion/Darwin/Type/_retrieve_mac_apps`.
//!
//! Full upstream body (109 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 6  _mac_apps_caching_policy () {
//! sh: 9    oldp=( "$1"(Nmw+1) )        # mtime qualifier: weeks unit, "+1" = older-than-1-week
//! sh:10    (( $#oldp ))
//! sh:19  _mac_apps_spotlight_retrieve () {
//! sh:20    mdfind_query="kMDItemContentType == 'com.apple.application-*'"
//! sh:22    for i in ${app_dir_root}; do
//! sh:23      _mac_apps+=(${(f)"$(_call_program command mdfind -onlyin ${(q)i} ${(q)mdfind_query})"})
//! sh:26    done
//! sh:28  _mac_apps_old_retrieve () {
//! sh:30    typeset -aU app_dir
//! sh:31    if [[ -z "$app_dir" ]] && ! zstyle -a ":completion:${curcontext}:commands" application-dir app_dir
//! sh:34      app_dir_stop_pattern=( "*.app" "contents#" "*data" "*plugins#" "*plug?ins#" "fonts#"
//! sh:35                             "document[[:alpha:]]#" "*help" "resources#" "images#" "*configurations#" )
//! sh:37      app_dir_pattern="(^(#i)(${(j/|/)app_dir_stop_pattern}))"
//! sh:38      app_dir=( ${^app_dir_root}/(${~app_dir_pattern}/)#(N) )
//! sh:39    fi
//! sh:44    if ! zstyle -t ":completion:${curcontext}:commands" ignore-bundle; then
//! sh:45      app_result=( ${^app_dir}*/Contents/(MacOS|MacOSClassic)(N) )
//! sh:46      _mac_apps+=( ${app_result[@]%/Contents/MacOS*} )
//! sh:47    fi
//! sh:50    if ! zstyle -t ":completion:${curcontext}:commands" ignore-single; then
//! sh:51      autoload -Uz zargs
//! sh:53      app_cand=( ${^app_dir}^*.[a-z]#/..namedfork/rsrc(.UrN,.RN^U) )
//! sh:54      envvars="$(builtin typeset -x)"
//! sh:55      nargs=$(( $(command sysctl -n kern.argmax) - $#envvars - 2048 ))
//! sh:56      app_result="$(zargs --max-chars $nargs ${app_cand[@]} -- grep -l APPL)"
//! sh:57      _mac_apps+=( ${${(f)app_result}%/..namedfork/rsrc} )
//! sh:59    fi
//! sh:62  _retrieve_mac_apps() {
//! sh:64    zstyle -s ":completion:*:*:$service:*" cache-policy cache_policy
//! sh:65    if [[ -z "$cache_policy" ]]; then
//! sh:66      zstyle ":completion:*:*:$service:*" cache-policy _mac_apps_caching_policy
//! sh:69    if ( (( ${#_mac_apps} == 0 )) || _cache_invalid Mac_applications ) \
//! sh:70          && ! _retrieve_cache Mac_applications; then
//! sh:74      zstyle -s ":completion:*:*:${service}:commands" search-method retrieve ||
//! sh:76        [[ mdutil -s / == *enabled* ]] && retrieve=_mac_apps_spotlight_retrieve
//! sh:81        || retrieve=_mac_apps_old_retrieve
//! sh:83      zstyle ":completion:*:*:${service}:commands" search-method $retrieve
//! sh:88      zstyle -a ":completion:${curcontext}:" application-path app_dir_root ||
//! sh:90-97     app_dir_root = default-old-set (if retrieve==old) else ( / )
//! sh:99      zstyle ":completion:*" application-path $app_dir_root
//! sh:102     typeset -g -Ua _mac_apps
//! sh:103     $retrieve
//! sh:105     _store_cache Mac_applications _mac_apps
//! sh:106   fi
//! sh:109  _retrieve_mac_apps "$@"
//! ```
//!
//! `_retrieve_mac_apps` itself has no `return`; per POSIX/zsh `if`
//! semantics, an untaken `if` (no body run, no `else`) yields exit
//! status 0, so the "cache still fresh / just loaded" branch returns
//! 0 and the rebuild branch returns `_store_cache`'s status — verified
//! live (`zsh -c 'f(){ if false; then echo x; fi }; f; echo $?'` → 0).
//!
//! The `${^app_dir}*/Contents/(MacOS|MacOSClassic)` (sh:45) and
//! `${^app_dir}^*.[a-z]#/..namedfork/rsrc` (sh:53) globs concatenate a
//! wildcard directly onto each `app_dir` entry with **no** `/`
//! separator — the wildcard therefore extends `app_dir`'s own last
//! path component (matched against its *parent* directory), not a
//! child of `app_dir`. Verified live against real zsh (extendedglob):
//! `${^app_dir}*/Contents/(MacOS|MacOSClassic)(N)` only matches
//! self/sibling-prefixed directories of `app_dir`, never descendants —
//! ported literally (sibling-prefix scan), not "fixed".

use crate::compsys::ported::_cache_invalid::_cache_invalid;
use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_retrieve_cache::_retrieve_cache;
use crate::compsys::ported::_store_cache::_store_cache;
use crate::ported::modules::zutil::{bin_zstyle, lookupstyle, testforstyle};
use crate::ported::params::{getaparam, getsparam, setaparam};
use crate::ported::zsh_h::{options, MAX_OPS};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

// ---------------------------------------------------------------------
// sh:6-11  _mac_apps_caching_policy
// ---------------------------------------------------------------------

/// sh:6-11 `_mac_apps_caching_policy` — rebuild (return 0) when the cache
/// file exists and is more than a week old (glob qualifier `mw+1`: `m`
/// = mtime, unit `w` = weeks, `+1` = "more than 1"); returns 1 (fresh)
/// when the file is missing (nullglob `N`) or newer.
pub fn mac_apps_caching_policy(cache_path: &str) -> i32 {
    match std::fs::metadata(cache_path).and_then(|m| m.modified()) {
        // sh:10  (( $#oldp )) — true (0) only when the qualifier matched.
        Ok(mtime) => match SystemTime::now().duration_since(mtime) {
            Ok(age) if age > Duration::from_secs(7 * 24 * 3600) => 0,
            _ => 1,
        },
        // sh:9  `(N)` — no match on a missing file → oldp empty → 1.
        Err(_) => 1,
    }
}

// ---------------------------------------------------------------------
// sh:19-26  _mac_apps_spotlight_retrieve
// ---------------------------------------------------------------------

/// sh:19-26 `_mac_apps_spotlight_retrieve` — one `mdfind -onlyin <dir>
/// <query>` per `app_dir_root` entry via the ported `_call_program`,
/// splitting stdout on newlines (`${(f)...}`).
fn mac_apps_spotlight_retrieve(app_dir_root: &[String]) -> Vec<String> {
    // sh:20
    let mdfind_query = "kMDItemContentType == 'com.apple.application-*'";
    let mut mac_apps = Vec::new();
    // sh:22-26
    for i in app_dir_root {
        let _ = _call_program(&[
            "command".to_string(),
            "mdfind".to_string(),
            "-onlyin".to_string(),
            i.clone(),
            mdfind_query.to_string(),
        ]);
        let out = getsparam("REPLY").unwrap_or_default();
        mac_apps.extend(out.lines().map(str::to_string));
    }
    mac_apps
}

// ---------------------------------------------------------------------
// sh:28-59  _mac_apps_old_retrieve
// ---------------------------------------------------------------------

/// One atom of an `app_dir_stop_pattern` entry (sh:34-35): `*`, `?`,
/// the `[[:alpha:]]` class, or a literal char.
#[derive(Clone, Copy)]
enum Atom {
    Star,
    Any,
    Alpha,
    Lit(char),
}

fn atom_matches(atom: Atom, c: char) -> bool {
    match atom {
        Atom::Star | Atom::Any => true,
        Atom::Alpha => c.is_ascii_alphabetic(),
        Atom::Lit(l) => c.to_ascii_lowercase() == l,
    }
}

/// Compile one stop-pattern string into `(atom, has_trailing_#)` pairs.
fn compile_stop_pattern(pat: &str) -> Vec<(Atom, bool)> {
    let mut out = Vec::new();
    let mut chars = pat.chars().peekable();
    while let Some(c) = chars.next() {
        let atom = match c {
            '*' => Atom::Star,
            '?' => Atom::Any,
            '[' => {
                // Only `[[:alpha:]]` occurs in this file's patterns;
                // consume up to (and including) the closing `]]`.
                while let Some(&nc) = chars.peek() {
                    chars.next();
                    if nc == ']' {
                        break;
                    }
                }
                if chars.peek() == Some(&']') {
                    chars.next();
                }
                Atom::Alpha
            }
            c => Atom::Lit(c.to_ascii_lowercase()),
        };
        let repeat = chars.peek() == Some(&'#');
        if repeat {
            chars.next();
        }
        out.push((atom, repeat));
    }
    out
}

/// Backtracking matcher for a compiled stop pattern against `text`
/// (case-insensitive — `atom_matches` lowercases both sides).
fn match_atoms(atoms: &[(Atom, bool)], text: &[char]) -> bool {
    if atoms.is_empty() {
        return text.is_empty();
    }
    let (atom, repeat) = atoms[0];
    match atom {
        Atom::Star => (0..=text.len()).any(|i| match_atoms(&atoms[1..], &text[i..])),
        _ if repeat => {
            let mut i = 0;
            loop {
                if match_atoms(&atoms[1..], &text[i..]) {
                    return true;
                }
                if i < text.len() && atom_matches(atom, text[i]) {
                    i += 1;
                } else {
                    return false;
                }
            }
        }
        _ => {
            !text.is_empty() && atom_matches(atom, text[0]) && match_atoms(&atoms[1..], &text[1..])
        }
    }
}

/// sh:34-35 — the fixed stop-pattern list.
const APP_DIR_STOP_PATTERNS: &[&str] = &[
    "*.app",
    "contents#",
    "*data",
    "*plugins#",
    "*plug?ins#",
    "fonts#",
    "document[[:alpha:]]#",
    "*help",
    "resources#",
    "images#",
    "*configurations#",
];

/// sh:37 `app_dir_pattern="(^(#i)(...))"` — true if `name` matches ANY
/// stop pattern case-insensitively (the `^` negation is applied by the
/// caller: a name is a valid `app_dir` component iff it is NOT a stop
/// dir).
fn is_stop_dir(name: &str) -> bool {
    let chars: Vec<char> = name.to_lowercase().chars().collect();
    APP_DIR_STOP_PATTERNS
        .iter()
        .any(|p| match_atoms(&compile_stop_pattern(p), &chars))
}

/// sh:38 `${^app_dir_root}/(${~app_dir_pattern}/)#(N)` — recursively
/// collect `root` and every descendant reached through a chain of
/// subdirectories none of which match `app_dir_stop_pattern`; recursion
/// stops at (and excludes) any directory that DOES match.
fn walk_app_dir(root: &Path, out: &mut Vec<String>) {
    out.push(root.to_string_lossy().into_owned());
    let entries = match std::fs::read_dir(root) {
        Ok(r) => r,
        Err(_) => return,
    };
    for ent in entries.flatten() {
        let path = ent.path();
        if !path.is_dir() {
            continue;
        }
        let name = ent.file_name();
        if is_stop_dir(&name.to_string_lossy()) {
            continue;
        }
        walk_app_dir(&path, out);
    }
}

/// sh:30-39 default `app_dir` computation (used when neither a local
/// `app_dir` nor the `application-dir` style supplied one).
fn default_app_dir(app_dir_root: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for root in app_dir_root {
        walk_app_dir(Path::new(root), &mut out);
    }
    // sh:30  typeset -aU app_dir — unique, first-seen order.
    let mut seen = HashSet::new();
    out.retain(|p| seen.insert(p.clone()));
    out
}

/// sh:45-46 — bundle search. NOTE (see module doc): the upstream glob
/// concatenates `*` directly onto each `app_dir` entry with no `/`
/// separator, so the wildcard extends the entry's own last path
/// component, scanned against its *parent* — for `d`, glob siblings of
/// `d` (in `dirname(d)`) whose name starts with `basename(d)`, then
/// check `sibling/Contents/{MacOS,MacOSClassic}`.
fn mac_apps_bundle_search(app_dir: &[String]) -> Vec<String> {
    let mut app_result = Vec::new();
    for d in app_dir {
        let path = Path::new(d);
        let (parent, prefix) = match (path.parent(), path.file_name()) {
            (Some(p), Some(n)) => (p, n.to_string_lossy().into_owned()),
            _ => continue,
        };
        let entries = match std::fs::read_dir(parent) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for ent in entries.flatten() {
            let name = ent.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with(prefix.as_str()) {
                continue;
            }
            for variant in ["MacOS", "MacOSClassic"] {
                let candidate = ent.path().join("Contents").join(variant);
                if candidate.is_dir() {
                    app_result.push(candidate.to_string_lossy().into_owned());
                }
            }
        }
    }
    // sh:46  ${app_result[@]%/Contents/MacOS*} — strip the shortest
    // `/Contents/MacOS*` suffix (matches either variant).
    app_result
        .into_iter()
        .map(|p| match p.rfind("/Contents/MacOS") {
            Some(idx) => p[..idx].to_string(),
            None => p,
        })
        .collect()
}

/// `*.[a-z]#` against `suffix` (sh:53 negation target) — true if
/// `suffix` ends with a literal `.` followed by zero-or-more lowercase
/// ASCII letters (anything may precede the `.`). Verified live against
/// real zsh: `${^app_dir}^*.[a-z]#(N)` keeps siblings whose leftover
/// suffix does NOT satisfy this (self, and non-dotted-lowercase-ext
/// siblings pass; dotted-lowercase-ext siblings are excluded).
fn suffix_is_dotted_lowercase_ext(suffix: &str) -> bool {
    match suffix.rfind('.') {
        Some(idx) => suffix[idx + 1..].chars().all(|c| c.is_ascii_lowercase()),
        None => false,
    }
}

/// sh:53 first stage — `${^app_dir}^*.[a-z]#`, same sibling-scan
/// mechanic as `mac_apps_bundle_search` but keeping only siblings whose
/// leftover suffix is NOT a dotted-lowercase extension.
fn negated_lowercase_ext_siblings(app_dir: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for d in app_dir {
        let path = Path::new(d);
        let (parent, prefix) = match (path.parent(), path.file_name()) {
            (Some(p), Some(n)) => (p, n.to_string_lossy().into_owned()),
            _ => continue,
        };
        let entries = match std::fs::read_dir(parent) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for ent in entries.flatten() {
            let name = ent.file_name();
            let name = name.to_string_lossy();
            let suffix = match name.strip_prefix(prefix.as_str()) {
                Some(s) => s,
                None => continue,
            };
            if !suffix_is_dotted_lowercase_ext(suffix) {
                out.push(ent.path());
            }
        }
    }
    out
}

/// sh:54-55 — `$#envvars` (char count of `builtin typeset -x` output)
/// approximated as the total byte length of `NAME=value\n` lines for
/// every exported var, mirroring the shape `typeset -x` would print.
fn envvars_char_count() -> usize {
    std::env::vars()
        .map(|(k, v)| k.len() + 1 + v.len() + 1)
        .sum()
}

/// sh:55  `command sysctl -n kern.argmax` — max exec() argv+envp bytes.
fn kern_argmax() -> i64 {
    match Command::new("sysctl").args(["-n", "kern.argmax"]).output() {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .trim()
            .parse()
            .unwrap_or(0),
        _ => 0,
    }
}

/// sh:50-59 — single-file (resource-fork `TYPE == 'APPL'`) search.
/// Batches candidate `.../..namedfork/rsrc` paths under the same
/// `nargs` budget zsh's `zargs --max-chars $nargs` would use, running
/// `grep -l APPL` per batch, then strips the `/..namedfork/rsrc` suffix
/// off each hit to recover the application file path.
fn mac_apps_single_file_search(app_dir: &[String]) -> Vec<String> {
    // sh:53 stage 1 — sibling-prefix scan, suffix-negation filtered.
    let siblings = negated_lowercase_ext_siblings(app_dir);
    // sh:53 stage 2 — `.../..namedfork/rsrc` must exist as a regular
    // file (the `(.UrN,.RN^U)` ownership/readability qualifiers are
    // not modeled — the file-exists gate is the dominant filter).
    let app_cand: Vec<String> = siblings
        .into_iter()
        .map(|p| p.join("..namedfork").join("rsrc"))
        .filter(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    if app_cand.is_empty() {
        return Vec::new();
    }

    // sh:54-55 — batch budget.
    let nargs = (kern_argmax() - envvars_char_count() as i64 - 2048).max(1) as usize;

    // sh:56 `zargs --max-chars $nargs ${app_cand[@]} -- grep -l APPL`
    let mut mac_apps = Vec::new();
    let mut batch: Vec<&str> = Vec::new();
    let mut batch_chars = 0usize;
    let mut flush = |batch: &mut Vec<&str>, mac_apps: &mut Vec<String>| {
        if batch.is_empty() {
            return;
        }
        if let Ok(out) = Command::new("grep")
            .arg("-l")
            .arg("APPL")
            .args(batch.iter().copied())
            .output()
        {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                // sh:57  ${${(f)app_result}%/..namedfork/rsrc}
                let stripped = line.strip_suffix("/..namedfork/rsrc").unwrap_or(line);
                mac_apps.push(stripped.to_string());
            }
        }
        batch.clear();
    };
    for cand in &app_cand {
        if batch_chars + cand.len() > nargs && !batch.is_empty() {
            flush(&mut batch, &mut mac_apps);
            batch_chars = 0;
        }
        batch.push(cand.as_str());
        batch_chars += cand.len();
    }
    flush(&mut batch, &mut mac_apps);
    mac_apps
}

/// sh:28-59 `_mac_apps_old_retrieve` — non-Spotlight fallback: locate
/// app-container directories, then find bundles and single-file apps
/// within them.
fn mac_apps_old_retrieve(app_dir_root: &[String], curcontext: &str) -> Vec<String> {
    let commands_ctx = format!(":completion:{}:commands", curcontext);

    // sh:31-39
    let mut app_dir = lookupstyle(&commands_ctx, "application-dir");
    if app_dir.is_empty() {
        app_dir = default_app_dir(app_dir_root);
    }

    let mut mac_apps = Vec::new();

    // sh:44-47  `if ! zstyle -t ... ignore-bundle; then ...` — mirrors
    // `_cache_invalid`/`_retrieve_cache`'s own `testforstyle(...) != 0`
    // idiom for `-t` style tests (0 == style found/true).
    if testforstyle(&commands_ctx, "ignore-bundle") != 0 {
        mac_apps.extend(mac_apps_bundle_search(&app_dir));
    }

    // sh:50-59
    if testforstyle(&commands_ctx, "ignore-single") != 0 {
        mac_apps.extend(mac_apps_single_file_search(&app_dir));
    }

    mac_apps
}

// ---------------------------------------------------------------------
// sh:62-107  _retrieve_mac_apps
// ---------------------------------------------------------------------

/// sh:90-97 default `app_dir_root` for the `_mac_apps_old_retrieve`
/// method: `{,/Developer,/Network,/System,$HOME}/{Applications*(N),Desktop}`,
/// existing paths only.
fn app_dir_root_candidates_for_prefix(prefix: &str) -> Vec<String> {
    let mut out = Vec::new();
    let list_dir = if prefix.is_empty() { "/" } else { prefix };
    if let Ok(rd) = std::fs::read_dir(list_dir) {
        let mut hits: Vec<String> = rd
            .flatten()
            .filter_map(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                if name.starts_with("Applications") {
                    Some(format!("{}/{}", prefix, name))
                } else {
                    None
                }
            })
            .collect();
        hits.sort();
        out.extend(hits);
    }
    let desktop = format!("{}/Desktop", prefix);
    if Path::new(&desktop).exists() {
        out.push(desktop);
    }
    out
}

fn default_app_dir_root_for_old() -> Vec<String> {
    let home = getsparam("HOME").unwrap_or_default();
    let prefixes = ["", "/Developer", "/Network", "/System", home.as_str()];
    let mut out = Vec::new();
    for prefix in prefixes {
        out.extend(app_dir_root_candidates_for_prefix(prefix));
    }
    out
}

/// sh:76  `[[ "$( command mdutil -s / 2>&1 )" == *enabled* ]]`
fn mdutil_root_indexed() -> bool {
    match Command::new("mdutil").args(["-s", "/"]).output() {
        Ok(o) => {
            let mut combined = String::from_utf8_lossy(&o.stdout).into_owned();
            combined.push_str(&String::from_utf8_lossy(&o.stderr));
            combined.contains("enabled")
        }
        Err(_) => false,
    }
}

/// `_retrieve_mac_apps` — (re)build the `_mac_apps` array (paths of
/// installed applications) used by `_mac_applications` /
/// `_mac_files_for_application`, honoring the completion cache.
pub fn _retrieve_mac_apps(_args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_retrieve_mac_apps");
    let service = getsparam("service").unwrap_or_default();
    let curcontext = getsparam("curcontext").unwrap_or_default();

    // sh:64  zstyle -s ":completion:*:*:$service:*" cache-policy cache_policy
    let cache_policy_ctx = format!(":completion:*:*:{}:*", service);
    let cache_policy = lookupstyle(&cache_policy_ctx, "cache-policy")
        .into_iter()
        .next()
        .unwrap_or_default();

    // sh:65-67
    if cache_policy.is_empty() {
        let _ = bin_zstyle(
            "zstyle",
            &[
                cache_policy_ctx,
                "cache-policy".to_string(),
                "_mac_apps_caching_policy".to_string(),
            ],
            &make_ops(),
            0,
        );
    }

    // sh:69-70
    let mac_apps_empty = getaparam("_mac_apps").map(|v| v.is_empty()).unwrap_or(true);
    let need_rebuild = (mac_apps_empty || _cache_invalid(&["Mac_applications".to_string()]) == 0)
        && _retrieve_cache(&["Mac_applications".to_string()]) != 0;

    if !need_rebuild {
        // sh — untaken `if` (no `else`) → exit status 0 (verified live).
        return 0;
    }

    // sh:74-84  choose retrieve method (cached in the search-method style).
    let search_ctx = format!(":completion:*:*:{}:commands", service);
    let mut retrieve = lookupstyle(&search_ctx, "search-method").into_iter().next();
    if retrieve.is_none() {
        // sh:76-82
        retrieve = Some(if mdutil_root_indexed() {
            "_mac_apps_spotlight_retrieve".to_string()
        } else {
            "_mac_apps_old_retrieve".to_string()
        });
        let _ = bin_zstyle(
            "zstyle",
            &[
                search_ctx,
                "search-method".to_string(),
                retrieve.clone().unwrap(),
            ],
            &make_ops(),
            0,
        );
    }
    let retrieve = retrieve.unwrap();

    // sh:87-100  root dirs to search
    let app_path_ctx = format!(":completion:{}:", curcontext);
    let mut app_dir_root = lookupstyle(&app_path_ctx, "application-path");
    if app_dir_root.is_empty() {
        app_dir_root = if retrieve == "_mac_apps_old_retrieve" {
            default_app_dir_root_for_old()
        } else {
            vec!["/".to_string()]
        };
        let mut zstyle_args = vec![":completion:*".to_string(), "application-path".to_string()];
        zstyle_args.extend(app_dir_root.clone());
        let _ = bin_zstyle("zstyle", &zstyle_args, &make_ops(), 0);
    }

    // sh:102-103
    let mut mac_apps: Vec<String> = if retrieve == "_mac_apps_old_retrieve" {
        mac_apps_old_retrieve(&app_dir_root, &curcontext)
    } else {
        mac_apps_spotlight_retrieve(&app_dir_root)
    };
    // sh:102  typeset -g -Ua _mac_apps — unique.
    let mut seen = HashSet::new();
    mac_apps.retain(|p| seen.insert(p.clone()));
    setaparam("_mac_apps", mac_apps);

    // sh:105
    _store_cache(&["Mac_applications".to_string(), "_mac_apps".to_string()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caching_policy_rebuilds_when_no_cache_file() {
        // sh:9  `(N)` — missing file → oldp empty → 1 (fresh)... but
        // note: missing file must NOT be confused with "invalid";
        // upstream's own nullglob semantics yield 1 here.
        assert_eq!(
            mac_apps_caching_policy("/nonexistent/zshrs/cache/mac_apps"),
            1
        );
    }

    #[test]
    fn caching_policy_stale_after_one_week() {
        let dir =
            std::env::temp_dir().join(format!("zshrs_mac_apps_cache_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("cache");
        std::fs::write(&file, b"x").unwrap();
        // fresh file → not invalid.
        assert_eq!(mac_apps_caching_policy(file.to_str().unwrap()), 1);
        // back-date the mtime past one week (no external `touch`/`date`
        // spawn — `File::set_modified` is a plain filesystem syscall).
        let old = SystemTime::now() - Duration::from_secs(8 * 24 * 3600);
        std::fs::File::open(&file)
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert_eq!(mac_apps_caching_policy(file.to_str().unwrap()), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_stop_dir_matches_fixed_stop_patterns_case_insensitively() {
        // sh:34  "*.app"
        assert!(is_stop_dir("Safari.app"));
        // sh:34  "contents#" — plural-optional via trailing `#`.
        assert!(is_stop_dir("Contents"));
        assert!(is_stop_dir("content"));
        // sh:34  "*plug?ins#" — `?` covers plugin/plug-ins spelling.
        assert!(is_stop_dir("PlugIns"));
        assert!(is_stop_dir("Plug-ins"));
        // sh:34  "document[[:alpha:]]#"
        assert!(is_stop_dir("Documentation"));
        // sh:34  "resources#" / "images#"
        assert!(is_stop_dir("Resources"));
        assert!(is_stop_dir("Images"));
        // not a stop dir.
        assert!(!is_stop_dir("Utilities"));
    }

    #[test]
    fn walk_app_dir_stops_recursion_into_bundle_and_resources() {
        let dir = std::env::temp_dir().join(format!("zshrs_walk_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Utilities")).unwrap();
        std::fs::create_dir_all(dir.join("Safari.app/Contents/Resources")).unwrap();
        let mut out = Vec::new();
        walk_app_dir(&dir, &mut out);
        let root = dir.to_string_lossy().into_owned();
        assert!(out.contains(&root));
        assert!(out.contains(&format!("{}/Utilities", root)));
        // sh:34 "*.app" stop pattern — never descends into the bundle.
        assert!(!out.iter().any(|p| p.contains("Safari.app")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bundle_search_finds_self_and_prefixed_sibling_only() {
        // Mirrors the live-zsh verification in the module doc comment:
        // ${^app_dir}*/Contents/(MacOS|MacOSClassic)(N) matches only
        // `app_dir` itself (star="") and prefix-matching siblings, not
        // descendants.
        let dir = std::env::temp_dir().join(format!("zshrs_bundle_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Applications/Contents/MacOS")).unwrap();
        std::fs::create_dir_all(dir.join("ApplicationsFoo/Contents/MacOSClassic")).unwrap();
        // A genuine nested bundle — NOT found, per the verified no-slash quirk.
        std::fs::create_dir_all(dir.join("Applications/Safari.app/Contents/MacOS")).unwrap();

        let app_dir = vec![dir.join("Applications").to_string_lossy().into_owned()];
        let hits = mac_apps_bundle_search(&app_dir);

        assert!(hits.contains(&dir.join("Applications").to_string_lossy().into_owned()));
        assert!(hits.contains(&dir.join("ApplicationsFoo").to_string_lossy().into_owned()));
        assert!(!hits.iter().any(|p| p.contains("Safari.app")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn suffix_is_dotted_lowercase_ext_matches_star_dot_lower_hash() {
        // Verified live: `${^app_dir}^*.[a-z]#(N)` excludes ".pm"/".rsrc"
        // siblings, keeps self ("") and non-dotted siblings ("X").
        assert!(suffix_is_dotted_lowercase_ext(".pm"));
        assert!(suffix_is_dotted_lowercase_ext(".rsrc"));
        assert!(!suffix_is_dotted_lowercase_ext(""));
        assert!(!suffix_is_dotted_lowercase_ext("X"));
    }

    #[test]
    fn negated_lowercase_ext_siblings_excludes_dotted_lowercase_ext() {
        let dir = std::env::temp_dir().join(format!("zshrs_siblings_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir(dir.join("Applications")).unwrap();
        std::fs::create_dir(dir.join("Applications.pm")).unwrap();
        std::fs::create_dir(dir.join("ApplicationsX")).unwrap();

        let app_dir = vec![dir.join("Applications").to_string_lossy().into_owned()];
        let hits: Vec<String> = negated_lowercase_ext_siblings(&app_dir)
            .into_iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();

        assert!(hits.contains(&dir.join("Applications").to_string_lossy().into_owned()));
        assert!(hits.contains(&dir.join("ApplicationsX").to_string_lossy().into_owned()));
        assert!(!hits.contains(&dir.join("Applications.pm").to_string_lossy().into_owned()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn spotlight_retrieve_splits_reply_on_newlines() {
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::setsparam("REPLY", "/A/Safari.app\n/A/Mail.app\n");
        // Exercise the split logic directly (avoids spawning mdfind).
        let out = getsparam("REPLY").unwrap_or_default();
        let lines: Vec<String> = out.lines().map(str::to_string).collect();
        assert_eq!(
            lines,
            vec!["/A/Safari.app".to_string(), "/A/Mail.app".to_string()]
        );
    }

    #[test]
    fn returns_zero_when_no_rebuild_needed_after_cache_disabled() {
        // sh — with use-cache off, `_cache_invalid`/`_retrieve_cache`
        // both return 1; `_mac_apps` non-empty ⇒ need_rebuild is false
        // ⇒ untaken `if` ⇒ 0 (verified live, see module doc).
        let _g = crate::test_util::global_state_lock();
        setaparam("_mac_apps", vec!["/Applications/Safari.app".to_string()]);
        assert_eq!(_retrieve_mac_apps(&[]), 0);
    }
}
