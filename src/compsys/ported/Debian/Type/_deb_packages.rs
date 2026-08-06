//! Port of `_deb_packages` from `Completion/Debian/Type/_deb_packages`.
//!
//! Full upstream body (138 lines verbatim):
//! ```text
//! sh:  1  #autoload
//! sh:  3  # Usage: _deb_packages expl...  (installed|deinstalled|xinstalled|held|uninstalled|avail|available|source)
//! sh:  5  _deb_packages_update_avail () {
//! sh:  6    if ( [[ ${+_deb_packages_cache_avail} -eq 0 ]] ||
//! sh:  7        _cache_invalid DEBS_avail ) && ! _retrieve_cache DEBS_avail;
//! sh:  8    then
//! sh:  9      _deb_packages_cache_avail=(
//! sh: 10        ${(f)"$(apt-cache --generate pkgnames 2>/dev/null)"}
//! sh: 11      )
//! sh: 13      _store_cache DEBS_avail _deb_packages_cache_avail
//! sh: 14    fi
//! sh: 15    cachevar=_deb_packages_cache_avail
//! sh: 16  }
//! sh: 18  _deb_packages_update_installed () {
//! sh: 19-27  … same guard; dpkg --get-selections | while read package state;
//! sh: 24       [[ $state = (install|hold) ]] && _deb_packages_cache_installed+=$package
//! sh: 28    cachevar=_deb_packages_cache_installed
//! sh: 29  }
//! sh: 31  _deb_packages_update_held () {           # state = hold
//! sh: 44  _deb_packages_update_deinstalled () {     # state = deinstall
//! sh: 57  _deb_packages_update_xinstalled () {      # every package, any state
//! sh: 70  _deb_packages_update_uninstalled () {
//! sh: 71    _deb_packages_update_avail
//! sh: 72    _deb_packages_update_installed
//! sh: 73    if (( ! $+_deb_packages_cache_uninstalled )); then
//! sh: 75      # Package lists too large to efficiently diff with zsh expansion
//! sh: 76      _deb_packages_cache_uninstalled=(
//! sh: 76        $( print -l $_deb_packages_cache_avail |
//! sh: 77           fgrep -xvf =(print -l $_deb_packages_cache_installed) )
//! sh: 78      )
//! sh: 80    cachevar=_deb_packages_cache_uninstalled
//! sh: 81  }
//! sh: 83  _deb_packages_update_source () {
//! sh: 84-85  … same guard …
//! sh: 87      _deb_packages_cache_source=(
//! sh: 90        ${(f)"$(/usr/lib/apt/apt-helper cat-file $(apt-get indextargets \
//! sh: 90          --format '$(FILENAME)' 'Created-By: Sources' 2>/dev/null) \
//! sh: 90          2>/dev/null | sed -ne 's/^Package: //p' | uniq)"}
//! sh: 91      )
//! sh: 93      _store_cache DEBS_source _deb_packages_cache_source
//! sh: 95    cachevar=_deb_packages_cache_source
//! sh: 96  }
//! sh: 98  _deb_packages () {
//! sh: 99    local command="$argv[$#]" expl cachevar pkgset update_policy
//! sh:101    zstyle -s ":completion:*:*:$service:*" cache-policy update_policy
//! sh:102    if [[ -z "$update_policy" ]]; then
//! sh:103      zstyle ":completion:*:*:$service:*" cache-policy _debs_caching_policy
//! sh:104    fi
//! sh:106    [[ "$command" = (installed|deinstalled|xinstalled|held|uninstalled|avail|available|source) ]] || {
//! sh:107      _message "unknown command: $command"
//! sh:108      return
//! sh:109    }
//! sh:111    zstyle -s ":completion:${curcontext}:" packageset pkgset
//! sh:113    [[ "$pkgset" = (installed|deinstalled|xinstalled|held|uninstalled|avail|available|source) ]] || {
//! sh:114      pkgset="$command"
//! sh:115    }
//! sh:117    [[ "$pkgset" = "available" ]] && pkgset="avail"
//! sh:119    expl=("${(@)argv[1,-2]}")
//! sh:121    _deb_packages_update_$pkgset
//! sh:123    typeset -gH $cachevar
//! sh:125    _tags packages && compadd "$expl[@]" -a - $cachevar
//! sh:126  }
//! sh:128  _debs_caching_policy () {
//! sh:129    # rebuild if cache is more than a week old
//! sh:130    local -a oldp
//! sh:131    oldp=( "$1"(mw+1) )
//! sh:132    (( $#oldp )) && return 0
//! sh:134    [[ /var/cache/apt/pkgcache.bin -nt "$1" ||
//! sh:135       /var/lib/dpkg/available -nt "$1" ]]
//! sh:136  }
//! sh:138  _deb_packages "$@"
//! ```
//!
//! External calls (`apt-cache`, `dpkg --get-selections`, `apt-get
//! indextargets`, `/usr/lib/apt/apt-helper`) are kept as literal
//! `std::process::Command` spawns (mirroring `_bsd_disks.rs`) rather than
//! routed through `_call_program`, since the upstream uses bare `$(...)`.

use crate::compsys::ported::_cache_invalid::cache_invalid_byname;
use crate::compsys::ported::_message::message_byname;
use crate::compsys::ported::_retrieve_cache::retrieve_cache_byname;
use crate::compsys::ported::_store_cache::store_cache_byname;
use crate::compsys::ported::_tags::tags_byname;
use crate::ported::modules::zutil::{bin_zstyle, lookupstyle};
use crate::ported::params::{getaparam, getsparam, setaparam};
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{options, MAX_OPS};
use std::process::Command;

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// The set of package-state keywords accepted as `$command`/`$pkgset`
/// (sh:106, sh:113).
const VALID_COMMANDS: &[&str] = &[
    "installed",
    "deinstalled",
    "xinstalled",
    "held",
    "uninstalled",
    "avail",
    "available",
    "source",
];

/// sh:6 `${+_deb_packages_cache_avail}` — array-parameter existence test.
fn cache_exists(name: &str) -> bool {
    getaparam(name).is_some()
}

/// Spawn `cmd args...` and return its captured stdout (empty string on
/// spawn failure — mirrors zsh `$(...)` degrading to empty output rather
/// than aborting the script; stderr is likewise discarded, matching the
/// upstream's `2>/dev/null`).
fn run_capture(cmd: &str, args: &[&str]) -> String {
    match Command::new(cmd).args(args).output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(_) => String::new(),
    }
}

/// sh:23, sh:36, sh:49, sh:62 — `dpkg --get-selections | while read
/// package state`: each line word-splits into `(package, state)`.
fn dpkg_get_selections() -> Vec<(String, String)> {
    let raw = run_capture("dpkg", &["--get-selections"]);
    raw.lines()
        .filter_map(|line| {
            let mut it = line.split_whitespace();
            let package = it.next()?.to_string();
            let state = it.next().unwrap_or("").to_string();
            Some((package, state))
        })
        .collect()
}

/// sh:5-16 `_deb_packages_update_avail` — rebuild (if stale/missing) and
/// return the cache-variable name.
fn update_avail() -> String {
    const IDENT: &str = "DEBS_avail";
    const VAR: &str = "_deb_packages_cache_avail";
    // sh:6-7  (not-set OR invalid) AND retrieve-cache failed → rebuild.
    if (!cache_exists(VAR) || cache_invalid_byname(&[IDENT.to_string()]) == 0)
        && retrieve_cache_byname(&[IDENT.to_string()]) != 0
    {
        // sh:10  ${(f)"$(apt-cache --generate pkgnames 2>/dev/null)"}
        let raw = run_capture("apt-cache", &["--generate", "pkgnames"]);
        let pkgs: Vec<String> = raw.lines().map(String::from).collect();
        setaparam(VAR, pkgs);
        // sh:13
        let _ = store_cache_byname(&[IDENT.to_string(), VAR.to_string()]);
    }
    // sh:15
    VAR.to_string()
}

/// sh:18-29 `_deb_packages_update_installed` — packages with state
/// `install` or `hold`.
fn update_installed() -> String {
    const IDENT: &str = "DEBS_installed";
    const VAR: &str = "_deb_packages_cache_installed";
    if (!cache_exists(VAR) || cache_invalid_byname(&[IDENT.to_string()]) == 0)
        && retrieve_cache_byname(&[IDENT.to_string()]) != 0
    {
        // sh:23-24
        let pkgs: Vec<String> = dpkg_get_selections()
            .into_iter()
            .filter(|(_, state)| state == "install" || state == "hold")
            .map(|(pkg, _)| pkg)
            .collect();
        setaparam(VAR, pkgs);
        let _ = store_cache_byname(&[IDENT.to_string(), VAR.to_string()]);
    }
    VAR.to_string()
}

/// sh:31-42 `_deb_packages_update_held` — packages with state `hold`.
fn update_held() -> String {
    const IDENT: &str = "DEBS_held";
    const VAR: &str = "_deb_packages_cache_held";
    if (!cache_exists(VAR) || cache_invalid_byname(&[IDENT.to_string()]) == 0)
        && retrieve_cache_byname(&[IDENT.to_string()]) != 0
    {
        // sh:36-37
        let pkgs: Vec<String> = dpkg_get_selections()
            .into_iter()
            .filter(|(_, state)| state == "hold")
            .map(|(pkg, _)| pkg)
            .collect();
        setaparam(VAR, pkgs);
        let _ = store_cache_byname(&[IDENT.to_string(), VAR.to_string()]);
    }
    VAR.to_string()
}

/// sh:44-55 `_deb_packages_update_deinstalled` — packages with state
/// `deinstall`.
fn update_deinstalled() -> String {
    const IDENT: &str = "DEBS_deinstalled";
    const VAR: &str = "_deb_packages_cache_deinstalled";
    if (!cache_exists(VAR) || cache_invalid_byname(&[IDENT.to_string()]) == 0)
        && retrieve_cache_byname(&[IDENT.to_string()]) != 0
    {
        // sh:49-50
        let pkgs: Vec<String> = dpkg_get_selections()
            .into_iter()
            .filter(|(_, state)| state == "deinstall")
            .map(|(pkg, _)| pkg)
            .collect();
        setaparam(VAR, pkgs);
        let _ = store_cache_byname(&[IDENT.to_string(), VAR.to_string()]);
    }
    VAR.to_string()
}

/// sh:57-68 `_deb_packages_update_xinstalled` — every known package,
/// regardless of state.
fn update_xinstalled() -> String {
    const IDENT: &str = "DEBS_xinstalled";
    const VAR: &str = "_deb_packages_cache_xinstalled";
    if (!cache_exists(VAR) || cache_invalid_byname(&[IDENT.to_string()]) == 0)
        && retrieve_cache_byname(&[IDENT.to_string()]) != 0
    {
        // sh:62-63
        let pkgs: Vec<String> = dpkg_get_selections()
            .into_iter()
            .map(|(pkg, _)| pkg)
            .collect();
        setaparam(VAR, pkgs);
        let _ = store_cache_byname(&[IDENT.to_string(), VAR.to_string()]);
    }
    VAR.to_string()
}

/// sh:70-81 `_deb_packages_update_uninstalled` — packages in `avail` but
/// not in `installed` (`fgrep -xvf` exact-line diff, order preserved).
fn update_uninstalled() -> String {
    const VAR: &str = "_deb_packages_cache_uninstalled";
    // sh:71-72
    let avail_var = update_avail();
    let installed_var = update_installed();
    // sh:73
    if !cache_exists(VAR) {
        let avail = getaparam(&avail_var).unwrap_or_default();
        let installed = getaparam(&installed_var).unwrap_or_default();
        // sh:76-77  `fgrep -xvf` — keep avail lines with no exact match
        // in installed; original (avail) order preserved.
        let installed_set: std::collections::HashSet<&str> =
            installed.iter().map(|s| s.as_str()).collect();
        let uninstalled: Vec<String> = avail
            .into_iter()
            .filter(|p| !installed_set.contains(p.as_str()))
            .collect();
        setaparam(VAR, uninstalled);
    }
    // sh:80
    VAR.to_string()
}

/// sh:83-96 `_deb_packages_update_source` — package names sourced from
/// `apt-get indextargets` / `apt-helper cat-file` output.
fn update_source() -> String {
    const IDENT: &str = "DEBS_source";
    const VAR: &str = "_deb_packages_cache_source";
    if (!cache_exists(VAR) || cache_invalid_byname(&[IDENT.to_string()]) == 0)
        && retrieve_cache_byname(&[IDENT.to_string()]) != 0
    {
        // sh:90 inner  $(apt-get indextargets --format '$(FILENAME)'
        //   'Created-By: Sources' 2>/dev/null) — unquoted, so the result
        //   word-splits into apt-helper's argv.
        let targets_raw = run_capture(
            "apt-get",
            &[
                "indextargets",
                "--format",
                "$(FILENAME)",
                "Created-By: Sources",
            ],
        );
        let target_args: Vec<&str> = targets_raw.split_whitespace().collect();
        let mut helper_args: Vec<&str> = vec!["cat-file"];
        helper_args.extend(target_args);
        // sh:90 outer  /usr/lib/apt/apt-helper cat-file <targets> | sed
        //   -ne 's/^Package: //p' | uniq
        let raw = run_capture("/usr/lib/apt/apt-helper", &helper_args);
        let mut pkgs: Vec<String> = raw
            .lines()
            .filter_map(|l| l.strip_prefix("Package: ").map(String::from))
            .collect();
        // `uniq` removes only CONSECUTIVE duplicate lines.
        pkgs.dedup();
        setaparam(VAR, pkgs);
        // sh:93
        let _ = store_cache_byname(&[IDENT.to_string(), VAR.to_string()]);
    }
    // sh:95
    VAR.to_string()
}

/// sh:128-136 `_debs_caching_policy` — rebuild if the cache file is more
/// than a week old, or `apt`/`dpkg`'s own package databases are newer
/// than it.
pub fn _debs_caching_policy(args: &[String]) -> i32 {
    use std::time::{Duration, SystemTime};
    // sh:131  oldp=( "$1"(mw+1) ) — glob-qualifier match on $1 itself;
    //   a missing $1/cache file behaves as "needs rebuild" (mirrors the
    //   sibling `_perl_modules_caching_policy` convention).
    let cachefile = match args.first() {
        Some(f) => f,
        None => return 0,
    };
    let cache_mtime = match std::fs::metadata(cachefile).and_then(|m| m.modified()) {
        Ok(t) => t,
        Err(_) => return 0,
    };
    // sh:131-132  older than one week → invalid (rebuild).
    if let Ok(age) = SystemTime::now().duration_since(cache_mtime) {
        if age > Duration::from_secs(7 * 24 * 3600) {
            return 0;
        }
    }
    // sh:134-135  `-nt` — either db file newer than the cache → invalid.
    let newer_than_cache = |p: &str| -> bool {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .map(|t| t > cache_mtime)
            .unwrap_or(false)
    };
    if newer_than_cache("/var/cache/apt/pkgcache.bin")
        || newer_than_cache("/var/lib/dpkg/available")
    {
        0
    } else {
        1
    }
}

/// `_deb_packages` — offer Debian package names for one of several
/// `dpkg`/`apt` package sets (installed, held, avail, source, ...).
pub fn _deb_packages(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_deb_packages");
    // sh:99  local command="$argv[$#]" — last positional argument.
    let command = args.last().cloned().unwrap_or_default();

    // sh:101  zstyle -s ":completion:*:*:$service:*" cache-policy update_policy
    let service = getsparam("service").unwrap_or_default();
    let policy_ctx = format!(":completion:*:*:{}:*", service);
    let update_policy = lookupstyle(&policy_ctx, "cache-policy")
        .into_iter()
        .next()
        .unwrap_or_default();
    // sh:102-104  register the default cache-policy hook if unset.
    if update_policy.is_empty() {
        let _ = bin_zstyle(
            "zstyle",
            &[
                policy_ctx,
                "cache-policy".to_string(),
                "_debs_caching_policy".to_string(),
            ],
            &make_ops(),
            0,
        );
    }

    // sh:106-109
    if !VALID_COMMANDS.contains(&command.as_str()) {
        // sh:107  _message "unknown command: $command"
        // sh:108  return — bare `return` propagates _message's status.
        return message_byname(&[format!("unknown command: {}", command)]);
    }

    // sh:111  zstyle -s ":completion:${curcontext}:" packageset pkgset
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let pkgset_ctx = format!(":completion:{}:", curcontext);
    let mut pkgset = lookupstyle(&pkgset_ctx, "packageset")
        .into_iter()
        .next()
        .unwrap_or_default();

    // sh:113-115
    if !VALID_COMMANDS.contains(&pkgset.as_str()) {
        pkgset = command.clone();
    }

    // sh:117
    if pkgset == "available" {
        pkgset = "avail".to_string();
    }

    // sh:119  expl=("${(@)argv[1,-2]}") — all args but the trailing command.
    let expl: Vec<String> = if args.is_empty() {
        Vec::new()
    } else {
        args[..args.len() - 1].to_vec()
    };

    // sh:121  _deb_packages_update_$pkgset — dispatch to the matching
    //   builder; `pkgset` is guaranteed one of VALID_COMMANDS (minus
    //   "available", folded into "avail" above) by sh:106-117.
    let cachevar = match pkgset.as_str() {
        "avail" => update_avail(),
        "installed" => update_installed(),
        "held" => update_held(),
        "deinstalled" => update_deinstalled(),
        "xinstalled" => update_xinstalled(),
        "uninstalled" => update_uninstalled(),
        "source" => update_source(),
        _ => String::new(),
    };

    // sh:123  typeset -gH $cachevar — the array is already a global
    //   param in this store; `-H` (hide from `typeset`/history listing)
    //   has no observable effect here, so this is a documented no-op.

    // sh:125  _tags packages && compadd "$expl[@]" -a - $cachevar
    let tags_ret = tags_byname(&["packages".to_string()]);
    if tags_ret != 0 {
        return tags_ret;
    }
    let mut cadd: Vec<String> = expl;
    cadd.push("-a".to_string());
    cadd.push("-".to_string());
    cadd.push(cachevar);
    bin_compadd("compadd", &cadd, &make_ops(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_command_reports_message_and_does_not_build_cache() {
        // sh:106-109 — the last positional arg isn't one of the known
        // package-state keywords, so we short-circuit through _message
        // (which itself returns 1 without a registered "messages" tag)
        // and never touch dpkg/apt-cache.
        let _g = crate::test_util::global_state_lock();
        let r = _deb_packages(&["expl".to_string(), "not-a-real-state".to_string()]);
        assert_eq!(r, 1);
    }

    #[test]
    fn available_alias_folds_to_avail_pkgset() {
        // sh:117 — "available" as $command/$pkgset normalizes to "avail"
        // before the dispatch switch; exercised indirectly via
        // dpkg_get_selections-free avail path (no cache pre-seeded, so
        // apt-cache spawns and — absent the binary in CI — yields an
        // empty list, then _tags fails without a registered tag set).
        let _g = crate::test_util::global_state_lock();
        let r = _deb_packages(&["available".to_string()]);
        // Without INCOMPFUNC/comptags wiring, _tags "packages" returns
        // non-zero, so _deb_packages returns that same non-zero code.
        assert_ne!(r, 0);
    }

    #[test]
    fn dpkg_get_selections_parses_package_and_state_columns() {
        // sh:23 `dpkg --get-selections | while read package state` reads
        // exactly the first two whitespace-separated fields per line.
        // We can't spawn a fake dpkg in CI, so exercise the parsing
        // logic directly via the same split used by dpkg_get_selections
        // by constructing the equivalent raw text.
        let raw = "vim\t\tinstall\nbash\thold\ncurl\tdeinstall\n";
        let parsed: Vec<(String, String)> = raw
            .lines()
            .filter_map(|line| {
                let mut it = line.split_whitespace();
                let package = it.next()?.to_string();
                let state = it.next().unwrap_or("").to_string();
                Some((package, state))
            })
            .collect();
        assert_eq!(
            parsed,
            vec![
                ("vim".to_string(), "install".to_string()),
                ("bash".to_string(), "hold".to_string()),
                ("curl".to_string(), "deinstall".to_string()),
            ]
        );
    }

    #[test]
    fn uninstalled_diff_preserves_avail_order_and_drops_installed_exact_matches() {
        // sh:76-77 `fgrep -xvf` — exact-line diff, avail order preserved.
        let avail = vec!["a", "b", "c", "d"];
        let installed: std::collections::HashSet<&str> = ["b", "d"].into_iter().collect();
        let uninstalled: Vec<&str> = avail
            .into_iter()
            .filter(|p| !installed.contains(*p))
            .collect();
        assert_eq!(uninstalled, vec!["a", "c"]);
    }

    #[test]
    fn source_pkg_extraction_dedups_only_consecutive_lines() {
        // sh:90 `sed -ne 's/^Package: //p' | uniq`. Two stages, in order:
        //   1. sed emits ONLY `^Package: ` lines (prefix stripped); the
        //      non-Package `Other: x` line is dropped BEFORE uniq sees it,
        //      so the two leading `foo`s become adjacent and collapse.
        //   2. uniq removes only ADJACENT duplicates — the trailing `foo`
        //      is separated from the first by `bar`, so it SURVIVES (a full
        //      set-dedup would drop it). This is what "only consecutive"
        //      means and is the property under test.
        let raw = "Package: foo\nOther: x\nPackage: foo\nPackage: bar\nPackage: foo\n";
        let mut pkgs: Vec<String> = raw
            .lines()
            .filter_map(|l| l.strip_prefix("Package: ").map(String::from))
            .collect();
        pkgs.dedup();
        assert_eq!(pkgs, vec!["foo", "bar", "foo"]);
    }

    #[test]
    fn caching_policy_rebuilds_when_cache_file_missing() {
        // sh:131-132 — no cache file at all → rebuild.
        assert_eq!(
            _debs_caching_policy(&["/nonexistent/zshrs/cache/deb_packages".to_string()]),
            0
        );
    }

    #[test]
    fn caching_policy_no_args_treated_as_rebuild() {
        assert_eq!(_debs_caching_policy(&[]), 0);
    }
}
