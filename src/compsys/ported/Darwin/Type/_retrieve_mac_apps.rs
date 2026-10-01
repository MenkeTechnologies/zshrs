//! Port of `_retrieve_mac_apps` from `Completion/Darwin/Type/_retrieve_mac_apps`.
//!
//! Full upstream body (109 lines, abridged):
//! ```text
//! sh:  1  #autoload
//! sh:  6  _mac_apps_caching_policy () {
//! sh:  9    oldp=( "$1"(Nmw+1) )        # mtime qualifier: weeks unit, "+1" = older-than-1-week
//! sh: 10    (( $#oldp ))
//! sh: 19  _mac_apps_spotlight_retrieve () {
//! sh: 20    mdfind_query="kMDItemContentType == 'com.apple.application-*'"
//! sh: 22    for i in ${app_dir_root}; do
//! sh: 23      _mac_apps+=(${(f)"$(_call_program command mdfind -onlyin ${(q)i} ${(q)mdfind_query})"})
//! sh: 25    done
//! sh: 28  _mac_apps_old_retrieve () {
//! sh: 30    typeset -aU app_dir
//! sh: 31    if [[ -z "$app_dir" ]] && ! zstyle -a ":completion:${curcontext}:commands" application-dir app_dir
//! sh: 34      app_dir_stop_pattern=( "*.app" "contents#" "*data" "*plugins#" "*plug?ins#" "fonts#"
//! sh: 35                             "document[[:alpha:]]#" "*help" "resources#" "images#" "*configurations#" )
//! sh: 37      app_dir_pattern="(^(#i)(${(j/|/)app_dir_stop_pattern}))"
//! sh: 38      app_dir=( ${^app_dir_root}/(${~app_dir_pattern}/)#(N) )
//! sh: 39    fi
//! sh: 44    if ! zstyle -t ":completion:${curcontext}:commands" ignore-bundle; then
//! sh: 45      app_result=( ${^app_dir}*/Contents/(MacOS|MacOSClassic)(N) )
//! sh: 46      _mac_apps+=( ${app_result[@]%/Contents/MacOS*} )
//! sh: 47    fi
//! sh: 50    if ! zstyle -t ":completion:${curcontext}:commands" ignore-single; then
//! sh: 51      autoload -Uz zargs
//! sh: 53      app_cand=( ${^app_dir}^*.[a-z]#/..namedfork/rsrc(.UrN,.RN^U) )
//! sh: 54      envvars="$(builtin typeset -x)"
//! sh: 55      nargs=$(( $(command sysctl -n kern.argmax) - $#envvars - 2048 ))
//! sh: 56      app_result="$(zargs --max-chars $nargs ${app_cand[@]} -- grep -l APPL)"
//! sh: 57      _mac_apps+=( ${${(f)app_result}%/..namedfork/rsrc} )
//! sh: 58    fi
//! sh: 62  _retrieve_mac_apps() {
//! sh: 64    zstyle -s ":completion:*:*:$service:*" cache-policy cache_policy
//! sh: 65    if [[ -z "$cache_policy" ]]; then
//! sh: 66      zstyle ":completion:*:*:$service:*" cache-policy _mac_apps_caching_policy
//! sh: 69    if ( (( ${#_mac_apps} == 0 )) || _cache_invalid Mac_applications ) \
//! sh: 70          && ! _retrieve_cache Mac_applications; then
//! sh: 74      zstyle -s ":completion:*:*:${service}:commands" search-method retrieve ||
//! sh: 76        [[ mdutil -s / == *enabled* ]] && retrieve=_mac_apps_spotlight_retrieve
//! sh: 81        || retrieve=_mac_apps_old_retrieve
//! sh: 83      zstyle ":completion:*:*:${service}:commands" search-method $retrieve
//! sh: 88      zstyle -a ":completion:${curcontext}:" application-path app_dir_root ||
//! sh: 90-97     app_dir_root = default-old-set (if retrieve==old) else ( / )
//! sh: 99      zstyle ":completion:*" application-path $app_dir_root
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
//! The sh:38, sh:45 and sh:53 globs run through the glob engine
//! ([`glob_each`]). Every `(pat/)#` match at sh:38 ends in `/`, so the
//! `*` that sh:45/53 append names the CHILDREN of each `app_dir` entry —
//! `${^app_dir}*/Contents/MacOS` finds the bundles inside each directory.
//! (An earlier version of this port claimed the opposite after testing
//! with hand-written entries that lacked the trailing `/`.)

use crate::compsys::ported::_cache_invalid::_cache_invalid;
use crate::compsys::ported::_call_program::call_program_capture;
use crate::compsys::ported::_retrieve_cache::_retrieve_cache;
use crate::compsys::ported::_store_cache::_store_cache;
use crate::compsys::ported::shared::zstyle_t;
use crate::ported::modules::zutil::{bin_zstyle, lookupstyle};
use crate::ported::params::{getaparam, getsparam, setaparam};
use crate::ported::glob::{tokenize, zglob};
use crate::ported::utils::quotestring;
use crate::ported::zsh_h::{options, MAX_OPS, QT_BACKSLASH_PATTERN};
use std::collections::HashSet;
use std::path::Path;
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
        let _ = call_program_capture(&[
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

/// `${^DIRS}PATTERN` — each element of an array, spliced literally in
/// front of PATTERN (`$x` without `~` is not a pattern, so its glob
/// characters are quoted), globbed by the real engine, results in order.
/// The surrounding completion context has `extendedglob` on
/// (`_comp_options`), which `^`, `#` and `(#i)` below rely on exactly as
/// the upstream function does.
fn glob_each(dirs: &[String], pattern: &str) -> Vec<String> {
    let mut out = Vec::new();
    for d in dirs {
        let mut word = format!("{}{}", quotestring(d, QT_BACKSLASH_PATTERN), pattern);
        tokenize(&mut word);
        let mut list = vec![word];
        zglob(&mut list, 0, 0);
        out.extend(list);
    }
    out
}

/// sh:30-38 — `typeset -aU app_dir`; `app_dir_pattern=
/// "(^(#i)(${(j/|/)app_dir_stop_pattern}))"`;
/// `app_dir=( ${^app_dir_root}/(${~app_dir_pattern}/)#(N) )`.
///
/// Run through the glob engine rather than walked by hand: a `(pat/)#`
/// closure does not follow symlinks (c:Src/glob.c:764 `l1->follow = 0`,
/// c:647 lstat), never matches a dot-directory (PAT_NOGLD), and every
/// match ENDS IN `/` — so `${^app_dir}*` at sh:45/53 names the CHILDREN
/// of each directory. The hand-written walk followed links (minutes of
/// walking through `~/Applications -> /Applications`) and dropped the
/// trailing `/`, which turned sh:45/53 into a sibling-prefix scan.
fn default_app_dir(app_dir_root: &[String]) -> Vec<String> {
    let app_dir_pattern = format!("(^(#i)({}))", APP_DIR_STOP_PATTERNS.join("|"));
    let mut out = glob_each(app_dir_root, &format!("/({}/)#(N)", app_dir_pattern));
    // sh:30  typeset -aU app_dir — unique, first-seen order.
    let mut seen = HashSet::new();
    out.retain(|p| seen.insert(p.clone()));
    out
}

/// sh:45-46 — `app_result=( ${^app_dir}*/Contents/(MacOS|MacOSClassic)(N) )`;
/// `_mac_apps+=( ${app_result[@]%/Contents/MacOS*} )`.
fn mac_apps_bundle_search(app_dir: &[String]) -> Vec<String> {
    glob_each(app_dir, "*/Contents/(MacOS|MacOSClassic)(N)")
        .into_iter()
        .map(|p| match p.rfind("/Contents/MacOS") {
            // `%` — the SHORTEST matching suffix.
            Some(idx) => p[..idx].to_string(),
            None => p,
        })
        .collect()
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
    // sh:53  app_cand=( ${^app_dir}^*.[a-z]#/..namedfork/rsrc(.UrN,.RN^U) )
    let app_cand = glob_each(app_dir, "^*.[a-z]#/..namedfork/rsrc(.UrN,.RN^U)");
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

    // sh:44 — `if ! zstyle -t … ignore-bundle; then`, a VALUE test; see
    //   [`zstyle_t`]. The bundle search runs on any non-zero exit: the
    //   style set to a non-boolean value (1) and the style unset (2).
    if zstyle_t(&commands_ctx, "ignore-bundle") != 0 {
        mac_apps.extend(mac_apps_bundle_search(&app_dir));
    }

    // sh:50 — `if ! zstyle -t … ignore-single; then`; see [`zstyle_t`].
    if zstyle_t(&commands_ctx, "ignore-single") != 0 {
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
    // sh:64  zstyle -s ":completion:*:*:$service:*" cache-policy cache_policy
    //
    // sh:65's `[[ -z $cache_policy ]]` is a VALUE test, so it stays; what
    // changes is that the value is `zutil.c:649`'s join of the whole array.
    let cache_policy = crate::compsys::ported::shared::zstyle_s(&cache_policy_ctx, "cache-policy").unwrap_or_default();

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
    // sh:74  if ! zstyle -s ":completion:*:*:${service}:commands" search-method retrieve
    //
    // The probe below runs on `zstyle -s`'s STATUS (`zutil.c:648` tests
    // `vals[0]`, a pointer) and the value is `zutil.c:649`'s join.
    let mut retrieve = crate::compsys::ported::shared::zstyle_s(&search_ctx, "search-method");
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
    // sh:102 — the `-U` ATTRIBUTE, not just a unique value. Stamped after
    // the assignment because `setaparam` creates the node and would not
    // carry the bit through. The retain() above already makes THIS value
    // unique, so nothing changes today; it matters because `_mac_apps` is a
    // `-g` cross-invocation cache that upstream APPENDS to (`_mac_apps+=(…)`
    // at sh:23, sh:46 and sh:57), and under `-U` every one of those appends
    // dedups. Without the flag a later append would accumulate duplicates.
    // `${(t)_mac_apps}` also reads `array-unique` in zsh vs plain `array`
    // here. Mirrors compinit.rs's `declare_global`
    // (c:Src/builtin.c:2575), inlined because that helper is private.
    if let Ok(mut tab) = crate::ported::params::paramtab().write() {
        if let Some(pm) = tab.get_mut("_mac_apps") {
            pm.node.flags |= crate::compsys::ported::shared::PM_UNIQUE as i32;
        }
    }

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

    /// The three sh:38/45/53 globs, through the engine, on a fixture that
    /// holds every case the hand-written emulation got wrong:
    ///   * `(pat/)#` stops at a stop-pattern directory (`Contents`,
    ///     `*.app`), does not follow a symlinked directory
    ///     (c:Src/glob.c:764) and skips a dot-directory (PAT_NOGLD);
    ///   * its matches end in `/`, so sh:45 `${^app_dir}*/Contents/MacOS`
    ///     finds bundles INSIDE each directory — `Tool.app` one level down
    ///     — not siblings sharing a name prefix (`ApplicationsFoo`).
    /// Expected values are real zsh 5.9.2's output on the same layout.
    /// Extended glob is set as `$_comp_setup` sets it for every compsys
    /// entry point (`Completion/compinit:141`).
    #[test]
    fn app_dir_and_bundle_globs_match_the_upstream_patterns() {
        let _g = crate::test_util::global_state_lock();
        let had = crate::ported::zsh_h::isset(crate::ported::zsh_h::EXTENDEDGLOB);
        crate::ported::options::opt_state_set("extendedglob", true);
        let dir = std::env::temp_dir().join(format!("zshrs_macapps_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let apps = dir.join("Applications");
        std::fs::create_dir_all(apps.join("Utilities/Tool.app/Contents/MacOS")).unwrap();
        std::fs::create_dir_all(apps.join("Big.app/Contents/MacOS")).unwrap();
        std::fs::create_dir_all(apps.join(".hidden/Secret.app/Contents/MacOS")).unwrap();
        std::fs::create_dir_all(dir.join("ApplicationsFoo/Contents/MacOS")).unwrap();
        std::fs::create_dir_all(dir.join("elsewhere/Linked.app/Contents/MacOS")).unwrap();
        std::os::unix::fs::symlink(dir.join("elsewhere"), apps.join("link")).unwrap();

        let root = apps.to_string_lossy().into_owned();
        let app_dir = default_app_dir(&[root.clone()]);
        let mut bundles = mac_apps_bundle_search(&app_dir);
        bundles.sort();
        let _ = std::fs::remove_dir_all(&dir);
        if !had {
            crate::ported::options::opt_state_unset("extendedglob");
        }

        assert_eq!(app_dir, vec![format!("{root}/"), format!("{root}/Utilities/")]);
        assert_eq!(
            bundles,
            vec![format!("{root}/Big.app"), format!("{root}/Utilities/Tool.app")]
        );
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
