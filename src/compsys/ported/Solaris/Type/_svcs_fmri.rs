//! Port of `_svcs_fmri` from `Completion/Solaris/Type/_svcs_fmri`.
//!
//! Full upstream body (97 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  _svcs_fmri() {
//! sh: 4    local type="$argv[$#]"
//! sh: 5    local fmri_abbrevs m i expl
//! sh: 6    typeset -a -g _smf_fmris
//! sh: 8    local update_policy
//! sh: 9    zstyle -s ":completion:${curcontext}:" cache-policy update_policy
//! sh:10    if [[ -z "$update_policy" ]]; then
//! sh:11      zstyle ":completion:${curcontext}:" cache-policy _smf_caching_policy
//! sh:12    fi
//! sh:14    local cache_id=smf_fmri:$HOST
//! sh:17    case $type in
//! sh:18    (-i|-c)
//! sh:25      if ( [[ $#_smf_fmris -eq 0 ]] || _cache_invalid $cache_id ) \
//! sh:26        && ! _retrieve_cache $cache_id; then
//! sh:27        _smf_fmris=( ${(f)"$(svcs -a -H -o fmri)"} )
//! sh:28        _store_cache $cache_id _smf_fmris
//! sh:29      fi
//! sh:32      fmri_abbrevs=( ${(M)_smf_fmris:#((#s)|*[/:])$PREFIX*} )
//! sh:36      for ((i = 1; i <= $#fmri_abbrevs; i++ )); do
//! sh:39        fmri_abbrevs[i]=${${fmri_abbrevs[i]}/((#s)|*[\/:])(#b)($PREFIX*)/$match[1]}
//! sh:41      done
//! sh:45      if [[ $type == "-i" ]]; then
//! sh:46        local -a svcs insts nabbrevs
//! sh:47        local s
//! sh:48        svcs=( ${(u)fmri_abbrevs%:*} )
//! sh:49        for s in $svcs; do
//! sh:50          insts=( ${(@M)fmri_abbrevs:#$s:*} )
//! sh:51          if [[ $#insts -eq 1 && $insts[1] == *":default" ]]; then
//! sh:52            nabbrevs=($nabbrevs ${insts//:default})
//! sh:53          elif [[ $#insts -eq 0 ]]; then
//! sh:56            nabbrevs=($nabbrevs $s)
//! sh:57          else
//! sh:58            nabbrevs=($nabbrevs $insts)
//! sh:59          fi
//! sh:60        done
//! sh:61        fmri_abbrevs=( $nabbrevs )
//! sh:62      fi
//! sh:69      _wanted fmri expl "full or unambiguously abbreviated FMRI" \
//! sh:70        compadd $fmri_abbrevs
//! sh:71      ;;
//! sh:73    (-m)
//! sh:74      _wanted fmri expl "milestone FMRI" \
//! sh:75        compadd $(svcs -H -o fmri svc:/milestone/\*) all
//! sh:76      ;;
//! sh:78    (-r)
//! sh:80      _wanted fmri expl "restarter FMRI" \
//! sh:81        compadd master reset svc:/network/inetd:default
//! sh:82      ;;
//! sh:84    (*)
//! sh:85      _message "unknown argument to _svcs_fmri: $type"
//! sh:86      ;;
//! sh:87    esac
//! sh:88  }
//! sh:90  _smf_caching_policy() {
//! sh:93    [[ ! -f "$1" || /etc/svc/repository.db -nt "$1" ]]
//! sh:94  }
//! sh:96  _svcs_fmri "$@"
//! ```
//!
//! sh:39's substitution `${x/((#s)|*[\/:])(#b)($PREFIX*)/$match[1]}` finds
//! the *rightmost* occurrence of `$PREFIX` that sits at the start of the
//! string or immediately after a `/`/`:`, then discards everything before
//! it — zsh's greedy `*` prefers the longest consumed lead-in (i.e. the
//! last valid delimiter), which is what `find_rightmost_prefix_start`
//! below replicates. sh:32's `(M)` filter is the same existence test.

use crate::compsys::ported::_cache_invalid::_cache_invalid;
use crate::compsys::ported::_message::_message;
use crate::compsys::ported::_retrieve_cache::_retrieve_cache;
use crate::compsys::ported::_store_cache::_store_cache;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::modules::zutil::{bin_zstyle, lookupstyle};
use crate::ported::params::{getaparam, getsparam, setaparam};
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

/// Spawn `cmd args...` and return its captured stdout (empty string on
/// spawn failure — zsh `$(...)` command substitution likewise degrades
/// to empty output rather than aborting the script).
fn run_capture(cmd: &str, args: &[&str]) -> String {
    match Command::new(cmd).args(args).output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(_) => String::new(),
    }
}

/// sh:32/39 — the rightmost byte offset in `s` at which `prefix` occurs
/// while sitting at the very start of the string, or immediately after a
/// `/`/`:` delimiter. `None` if no such occurrence exists.
fn find_rightmost_prefix_start(s: &str, prefix: &str) -> Option<usize> {
    let mut best: Option<usize> = None;
    // sh:32 `(#s)` alternative — start-of-string always a candidate.
    if s.starts_with(prefix) {
        best = Some(0);
    }
    // sh:32 `*[/:]` alternative — any occurrence right after a delimiter.
    for (i, c) in s.char_indices() {
        if c == '/' || c == ':' {
            let after = i + c.len_utf8();
            if s[after..].starts_with(prefix) && best.is_none_or(|b| after > b) {
                best = Some(after);
            }
        }
    }
    best
}

/// sh:32 — `(M)` keep-matching filter test.
fn matches_prefix_pattern(s: &str, prefix: &str) -> bool {
    find_rightmost_prefix_start(s, prefix).is_some()
}

/// sh:39 — strip everything before the rightmost valid `$PREFIX`
/// occurrence, keeping `$PREFIX` and the remainder of the string.
fn strip_before_prefix(s: &str, prefix: &str) -> String {
    match find_rightmost_prefix_start(s, prefix) {
        Some(pos) => s[pos..].to_string(),
        None => s.to_string(),
    }
}

/// sh:48 — `${fmri%:*}`: shortest `:*` suffix removed from the end, i.e.
/// truncate at the *last* `:` in the string (unchanged if none).
fn strip_last_colon_suffix(s: &str) -> String {
    match s.rfind(':') {
        Some(i) => s[..i].to_string(),
        None => s.to_string(),
    }
}

/// sh:52 — `${insts//:default}`: remove every literal `:default`
/// occurrence from the string.
fn remove_colon_default(s: &str) -> String {
    s.replace(":default", "")
}

/// sh:90-93 `_smf_caching_policy` — cache is invalid (return 0) when the
/// cache file `$1` doesn't exist, or `/etc/svc/repository.db` is newer
/// than it. Registered (sh:11) as the default `cache-policy` style for
/// `_svcs_fmri`'s `smf_fmri:$HOST` cache, dispatched through
/// `_cache_invalid`.
pub fn _smf_caching_policy(args: &[String]) -> i32 {
    let cachefile = match args.first() {
        Some(f) => f,
        None => return 0,
    };
    // sh:93 `! -f "$1"` — missing cache file → invalid.
    let cache_mtime = match std::fs::metadata(cachefile).and_then(|m| m.modified()) {
        Ok(t) => t,
        Err(_) => return 0,
    };
    // sh:93 `/etc/svc/repository.db -nt "$1"` — repository newer → invalid.
    match std::fs::metadata("/etc/svc/repository.db").and_then(|m| m.modified()) {
        Ok(db_mtime) if db_mtime > cache_mtime => 0,
        _ => 1,
    }
}

/// `_svcs_fmri` — complete Solaris SMF FMRIs (`svcs`/`svcadm`/`svccfg`
/// style: `-i` unique instance FMRIs, `-c` full FMRIs, `-m` milestone
/// FMRIs, `-r` restarter FMRIs).
pub fn _svcs_fmri(args: &[String]) -> i32 {
    // sh:4  local type="$argv[$#]"
    let type_ = args.last().cloned().unwrap_or_default();

    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctx = format!(":completion:{}:", curcontext);

    // sh:9-12 — register the default cache-policy if the user set none.
    if lookupstyle(&ctx, "cache-policy").is_empty() {
        let _ = bin_zstyle(
            "zstyle",
            &[
                ctx.clone(),
                "cache-policy".to_string(),
                "_smf_caching_policy".to_string(),
            ],
            &make_ops(),
            0,
        );
    }

    // sh:14
    let host = getsparam("HOST").unwrap_or_default();
    let cache_id = format!("smf_fmri:{}", host);

    match type_.as_str() {
        // sh:18-71
        "-i" | "-c" => {
            // sh:25-29 — rebuild the global FMRI cache when empty/stale
            //   and no on-disk cache could be retrieved.
            let cur_fmris = getaparam("_smf_fmris").unwrap_or_default();
            let need_build = (cur_fmris.is_empty()
                || _cache_invalid(std::slice::from_ref(&cache_id)) == 0)
                && _retrieve_cache(std::slice::from_ref(&cache_id)) != 0;
            if need_build {
                // sh:27  ${(f)"$(svcs -a -H -o fmri)"} — newline split.
                let raw = run_capture("svcs", &["-a", "-H", "-o", "fmri"]);
                let fmris: Vec<String> = raw.lines().map(str::to_string).collect();
                setaparam("_smf_fmris", fmris);
                // sh:28  _store_cache $cache_id _smf_fmris
                let _ = _store_cache(&[cache_id.clone(), "_smf_fmris".to_string()]);
            }
            let smf_fmris = getaparam("_smf_fmris").unwrap_or_default();

            // sh:32  fmri_abbrevs=( ${(M)_smf_fmris:#((#s)|*[/:])$PREFIX*} )
            let prefix = getsparam("PREFIX").unwrap_or_default();
            let mut fmri_abbrevs: Vec<String> = smf_fmris
                .iter()
                .filter(|f| matches_prefix_pattern(f, &prefix))
                .cloned()
                .collect();

            // sh:36-41 — strip everything before the abbreviation point.
            for f in fmri_abbrevs.iter_mut() {
                *f = strip_before_prefix(f, &prefix);
            }

            // sh:45-62 — for `-i`, collapse the sole ":default" instance
            //   of a service down to the bare service name.
            if type_ == "-i" {
                // sh:48  svcs=( ${(u)fmri_abbrevs%:*} )
                let mut svcs: Vec<String> = Vec::new();
                for a in &fmri_abbrevs {
                    let s = strip_last_colon_suffix(a);
                    if !svcs.contains(&s) {
                        svcs.push(s);
                    }
                }
                let mut nabbrevs: Vec<String> = Vec::new();
                for s in &svcs {
                    // sh:50  ${(@M)fmri_abbrevs:#$s:*}
                    let needle = format!("{}:", s);
                    let insts: Vec<String> = fmri_abbrevs
                        .iter()
                        .filter(|a| a.starts_with(&needle))
                        .cloned()
                        .collect();
                    if insts.len() == 1 && insts[0].ends_with(":default") {
                        // sh:51-52
                        nabbrevs.push(remove_colon_default(&insts[0]));
                    } else if insts.is_empty() {
                        // sh:53-56 — completing the instance name itself.
                        nabbrevs.push(s.clone());
                    } else {
                        // sh:57-58
                        nabbrevs.extend(insts);
                    }
                }
                // sh:61
                fmri_abbrevs = nabbrevs;
            }

            // sh:69-70
            let mut wanted_argv: Vec<String> = vec![
                "fmri".to_string(),
                "expl".to_string(),
                "full or unambiguously abbreviated FMRI".to_string(),
                "compadd".to_string(),
            ];
            wanted_argv.extend(fmri_abbrevs);
            _wanted(&wanted_argv)
        }

        // sh:73-76
        "-m" => {
            let raw = run_capture("svcs", &["-H", "-o", "fmri", "svc:/milestone/*"]);
            let mut wanted_argv: Vec<String> = vec![
                "fmri".to_string(),
                "expl".to_string(),
                "milestone FMRI".to_string(),
                "compadd".to_string(),
            ];
            wanted_argv.extend(raw.split_whitespace().map(str::to_string));
            wanted_argv.push("all".to_string());
            _wanted(&wanted_argv)
        }

        // sh:78-82
        "-r" => _wanted(&[
            "fmri".to_string(),
            "expl".to_string(),
            "restarter FMRI".to_string(),
            "compadd".to_string(),
            "master".to_string(),
            "reset".to_string(),
            "svc:/network/inetd:default".to_string(),
        ]),

        // sh:84-86
        _ => _message(&[format!("unknown argument to _svcs_fmri: {}", type_)]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_rightmost_prefix_start_of_string() {
        // sh:32 `(#s)` branch — prefix at position 0.
        assert_eq!(find_rightmost_prefix_start("ssh:default", "ssh"), Some(0));
    }

    #[test]
    fn find_rightmost_prefix_prefers_rightmost_delimiter() {
        // sh:39 greedy `*[\/:]` — picks the last `/`/`:` before an
        //   occurrence of PREFIX, not the first.
        assert_eq!(
            find_rightmost_prefix_start("svc:/network/ssh:default", "ssh"),
            Some("svc:/network/".len())
        );
    }

    #[test]
    fn find_rightmost_prefix_none_when_absent() {
        assert_eq!(find_rightmost_prefix_start("svc:/network/ssh", "xyz"), None);
    }

    #[test]
    fn strip_before_prefix_keeps_from_delimiter() {
        assert_eq!(
            strip_before_prefix("svc:/network/ssh:default", "ssh"),
            "ssh:default"
        );
    }

    #[test]
    fn strip_before_prefix_unchanged_when_no_match() {
        assert_eq!(
            strip_before_prefix("svc:/network/ssh", "xyz"),
            "svc:/network/ssh"
        );
    }

    #[test]
    fn strip_last_colon_suffix_truncates_at_last_colon() {
        // sh:48 `${fmri%:*}`
        assert_eq!(strip_last_colon_suffix("ssh:default"), "ssh");
        assert_eq!(
            strip_last_colon_suffix("svc:/network/ssh:default"),
            "svc:/network/ssh"
        );
        assert_eq!(strip_last_colon_suffix("no-colon"), "no-colon");
    }

    #[test]
    fn remove_colon_default_strips_all_occurrences() {
        // sh:52 `${insts//:default}`
        assert_eq!(remove_colon_default("ssh:default"), "ssh");
        assert_eq!(remove_colon_default("ssh"), "ssh");
    }

    #[test]
    fn smf_caching_policy_invalid_when_cache_file_missing() {
        // sh:93 `! -f "$1"` → invalid (0).
        assert_eq!(
            _smf_caching_policy(&["/nonexistent/zshrs/cache/file".to_string()]),
            0
        );
    }

    #[test]
    fn smf_caching_policy_no_args_returns_invalid() {
        assert_eq!(_smf_caching_policy(&[]), 0);
    }

    #[test]
    fn unknown_argument_dispatches_message_branch() {
        // sh:84-86 — the `(*)` arm. Without a registered `messages` tag,
        //   `_message` returns 1 (see `_message.rs`'s
        //   `default_mode_requires_messages_tag` test), so `_svcs_fmri`
        //   forwards that same return code for a bogus final argument.
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_svcs_fmri(&["-Q".to_string()]), 1);
    }

    #[test]
    fn empty_args_falls_through_to_unknown_branch() {
        // `$argv[$#]` on an empty argv is the empty string, which
        //   doesn't match any of `-i`/`-c`/`-m`/`-r` → `(*)` arm.
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_svcs_fmri(&[]), 1);
    }

    #[test]
    fn restarter_branch_returns_one_without_completion_context() {
        // sh:78-82 — `_wanted` short-circuits to 1 without a live
        //   completion context (no comptags state pre-loaded), and no
        //   external command is ever spawned for `-r`.
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_svcs_fmri(&["-r".to_string()]), 1);
    }
}
