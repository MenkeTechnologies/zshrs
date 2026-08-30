//! Port of `_perl_modules` from `Completion/Unix/Type/_perl_modules`.
//!
//! Full upstream body (153 lines):
//! ```text
//! sh:  1  #compdef pmpath pmvers pmdesc pmload pmexp pmeth pmls pmcat pman …
//! sh: 36  _perl_modules () {
//! sh: 42    zstyle -s …: cache-policy update_policy; [[ -z ]] && zstyle … _perl_modules_caching_policy
//! sh: 48    --perl-hierarchy=… → restrict_hierarchy;  --strip-prefix → strip_perl_prefix
//! sh: 57    -tP → sufpat="(.pm|.pod)" with_pod=_with_pod
//! sh: 64    if service==(perl|perldoc) && whence ${(Q)words[1]}%doc: perl=$_; perl_modules=_<san>_modules…
//! sh: 67    elif whence perl: perl=perl; perl_modules=_perl_modules…
//! sh: 70    else perl=; perl_modules=_unknown_perl_modules…
//! sh: 75    if ( $+perl_modules==0 || _cache_invalid <key> ) && ! _retrieve_cache <key>; then
//! sh: 78      if try-to-use-pminst && $+commands[pminst]: set -A $perl_modules $(pminst)
//! sh: 82      else inc=( $(perl -e 'print "@INC"') )  (or guessed @INC + $PERL5LIB if no perl)
//! sh: 99        for libdir: new_pms=( $libdir/{[A-Za-z]*/***/,}*${~sufpat}~*blib* ); strip libdir/; :r:fs#/#::#
//! sh:116      _store_cache <key> $perl_modules
//! sh:122    restrict_hierarchy → perl_subset=( ${(PM)…} )( - prefix if strip )
//! sh:131    _wanted modules expl 'Perl module' compadd "$@" -a - $perl_modules
//! sh:134  _perl_modules_caching_policy() { >1wk old OR perllocal.pod newer → 0 (invalid) }
//! sh:153  _perl_modules "$@"
//! ```

use crate::compsys::ported::_cache_invalid::_cache_invalid;
use crate::compsys::ported::_call_program::call_program_capture;
use crate::compsys::ported::_message::_message;
use crate::compsys::ported::_retrieve_cache::_retrieve_cache;
use crate::compsys::ported::_store_cache::_store_cache;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::exec::findcmd;
use crate::ported::modules::zutil::{bin_zstyle, lookupstyle};
use crate::ported::params::{getaparam, getsparam, setaparam};
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// zsh `${(Q)s}` — strip one level of shell quoting (surrounding `'…'`/`"…"`
/// and backslash escapes).
fn unquote_q(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            }
            '\'' | '"' => {} // drop surrounding quotes
            _ => out.push(c),
        }
    }
    out
}

/// sh:66 `_${${perl//[^[:alnum:]]/_}#_}` — non-alnum → `_`, strip one leading `_`.
fn sanitize_perl(perl: &str) -> String {
    let repl: String = perl
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    repl.strip_prefix('_').unwrap_or(&repl).to_string()
}

/// sh:104-110 — recursively collect module files under `libdir`, excluding any
/// path containing `blib` (`~*blib*`), convert each to `Foo::Bar` (strip
/// `libdir/`, drop the `.pm`/`.pod` suffix, `/`→`::`). `pod` adds `.pod`.
fn scan_libdir(libdir: &str, pod: bool) -> Vec<String> {
    let mut out = Vec::new();
    let base = std::path::Path::new(libdir);
    let mut stack = vec![base.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for ent in rd.flatten() {
            let path = ent.path();
            // ~*blib* — skip any component containing "blib".
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.contains("blib"))
            {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let is_mod = name.ends_with(".pm") || (pod && name.ends_with(".pod"));
            if !is_mod {
                continue;
            }
            if let Ok(rel) = path.strip_prefix(base) {
                let rel_s = rel.to_string_lossy();
                let no_suf = rel_s
                    .strip_suffix(".pm")
                    .or_else(|| rel_s.strip_suffix(".pod"))
                    .unwrap_or(&rel_s);
                out.push(no_suf.replace('/', "::"));
            }
        }
    }
    out
}

/// sh:86 `perl -e 'print "@INC"'` — the module search path for `$perl`.
fn perl_inc(perl: &str) -> Vec<String> {
    let _ = call_program_capture(&[
        "perl-inc".to_string(),
        perl.to_string(),
        "-e".to_string(),
        "print \"@INC\"".to_string(),
    ]);
    getsparam("REPLY")
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// sh:92-93 — guessed @INC when perl isn't on `$PATH`:
/// `/usr/lib/perl5{,/{site_perl/,}<5->.<num>}` plus `$PERL5LIB` (`:`-split).
fn guessed_inc() -> Vec<String> {
    let mut inc = Vec::new();
    let root = std::path::Path::new("/usr/lib/perl5");
    if root.is_dir() {
        inc.push(root.to_string_lossy().into_owned());
        // versioned subdirs `5.NN` and `site_perl/5.NN`.
        for sub in ["", "site_perl"] {
            let dir = if sub.is_empty() {
                root.to_path_buf()
            } else {
                root.join(sub)
            };
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for ent in rd.flatten() {
                    let n = ent.file_name();
                    let ns = n.to_string_lossy();
                    // `<5->.([0-9]##)` — 5-or-more `.` digits.
                    if ns.starts_with("5.") && ns[2..].chars().all(|c| c.is_ascii_digit()) {
                        inc.push(ent.path().to_string_lossy().into_owned());
                    }
                }
            }
        }
    }
    if let Some(p5) = getsparam("PERL5LIB") {
        inc.extend(p5.split(':').filter(|s| !s.is_empty()).map(str::to_string));
    }
    inc
}

/// sh:134-151 `_perl_modules_caching_policy` — cache is invalid (return 0) when
/// the cache file `$1` is more than a week old, or any
/// `/usr/lib/perl5/**/perllocal.pod` is newer than it. Wired into the router so
/// `_cache_invalid` can dispatch it.
pub fn _perl_modules_caching_policy(args: &[String]) -> i32 {
    use std::time::{Duration, SystemTime};
    let cachefile = match args.first() {
        Some(f) => f,
        None => return 1,
    };
    let cache_mtime = match std::fs::metadata(cachefile).and_then(|m| m.modified()) {
        Ok(t) => t,
        Err(_) => return 0, // no cache file → rebuild.
    };
    // sh:139 `"$1"(mw+1)` — older than one week → invalid.
    if let Ok(age) = SystemTime::now().duration_since(cache_mtime) {
        if age > Duration::from_secs(7 * 24 * 3600) {
            return 0;
        }
    }
    // sh:142-148 — any perllocal.pod newer than the cache → invalid.
    let mut stack = vec![std::path::PathBuf::from("/usr/lib/perl5")];
    while let Some(dir) = stack.pop() {
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for ent in rd.flatten() {
            let path = ent.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().and_then(|n| n.to_str()) == Some("perllocal.pod") {
                if let Ok(pm) = std::fs::metadata(&path).and_then(|m| m.modified()) {
                    if pm > cache_mtime {
                        return 0;
                    }
                }
            }
        }
    }
    1
}

/// `_perl_modules` — complete installed Perl module names (`Foo::Bar`).
pub fn _perl_modules(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_perl_modules");
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let base_ctx = format!(":completion:{}:", curcontext);

    // sh:42-46 — register the default cache-policy if the user set none.
    if lookupstyle(&base_ctx, "cache-policy").is_empty() {
        let _ = bin_zstyle(
            "zstyle",
            &[
                base_ctx.clone(),
                "cache-policy".to_string(),
                "_perl_modules_caching_policy".to_string(),
            ],
            &make_ops(),
            0,
        );
    }

    // sh:48-61 — extract local flags, leaving the rest for compadd.
    let mut restrict = String::new();
    let mut strip_prefix = false;
    let mut pod = false;
    let mut rest: Vec<String> = Vec::new();
    for a in args {
        if let Some(h) = a.strip_prefix("--perl-hierarchy=") {
            restrict = format!("{}::", h.trim_end_matches("::"));
        } else if a == "--strip-prefix" {
            strip_prefix = true;
        } else if a == "-tP" {
            pod = true;
        } else {
            rest.push(a.clone());
        }
    }
    let with_pod = if pod { "_with_pod" } else { "" };

    // sh:63-73 — pick the perl binary + its per-binary cache variable name.
    let service = getsparam("service").unwrap_or_default();
    let words = getaparam("words").unwrap_or_default();
    let (perl, perl_modules_var): (String, String) = {
        let perldoc_cand = if service == "perl" || service == "perldoc" {
            let w1 = unquote_q(&words.first().cloned().unwrap_or_default());
            let cand = w1.strip_suffix("doc").unwrap_or(&w1).to_string();
            if !cand.is_empty() && findcmd(&cand, 0, 0).is_some() {
                Some(cand)
            } else {
                None
            }
        } else {
            None
        };
        if let Some(cand) = perldoc_cand {
            let var = format!("_{}_modules{}", sanitize_perl(&cand), with_pod);
            (cand, var)
        } else if findcmd("perl", 0, 0).is_some() {
            ("perl".to_string(), format!("_perl_modules{}", with_pod))
        } else {
            (String::new(), format!("_unknown_perl_modules{}", with_pod))
        }
    };
    // sh:75 `${perl_modules#_}` — cache key (name minus leading `_`).
    let key = perl_modules_var
        .strip_prefix('_')
        .unwrap_or(&perl_modules_var)
        .to_string();

    // sh:75-117 — (param unset OR cache invalid) AND retrieve failed → rebuild.
    let need_build = (getaparam(&perl_modules_var).is_none()
        || _cache_invalid(std::slice::from_ref(&key)) == 0)
        && _retrieve_cache(std::slice::from_ref(&key)) != 0;
    if need_build {
        let try_pminst = matches!(
            lookupstyle(
                &format!(":completion:{}:modules", curcontext),
                "try-to-use-pminst"
            )
            .first()
            .map(|s| s.as_str()),
            Some("yes") | Some("true") | Some("on") | Some("1")
        );
        let mods: Vec<String> = if try_pminst && findcmd("pminst", 0, 0).is_some() {
            // sh:81  set -A $perl_modules $(pminst)
            let _ = call_program_capture(&["modules".to_string(), "pminst".to_string()]);
            getsparam("REPLY")
                .unwrap_or_default()
                .split_whitespace()
                .map(str::to_string)
                .collect()
        } else {
            // sh:82-113  @INC scan (or guessed @INC when perl is absent).
            let inc = if !perl.is_empty() {
                perl_inc(&perl)
            } else {
                let _ = _message(&["didn't find perl on $PATH; guessing @INC ...".to_string()]);
                guessed_inc()
            };
            let mut all: Vec<String> = Vec::new();
            for libdir in inc {
                if libdir == "." || libdir.is_empty() {
                    continue; // sh:101  ignore cwd.
                }
                all.extend(scan_libdir(&libdir, pod));
            }
            all
        };
        // sh:96  typeset -agU — global, unique.
        let mut uniq = mods;
        uniq.sort();
        uniq.dedup();
        setaparam(&perl_modules_var, uniq);
        // sh:116  _store_cache <key> $perl_modules (the param NAME).
        let _ = _store_cache(&[key.clone(), perl_modules_var.clone()]);
    }

    let mut modules = getaparam(&perl_modules_var).unwrap_or_default();

    // sh:122-128 — restrict to a hierarchy, optionally stripping the prefix.
    if !restrict.is_empty() {
        modules.retain(|m| m.starts_with(&restrict));
        if strip_prefix {
            modules = modules
                .iter()
                .map(|m| m.strip_prefix(&restrict).unwrap_or(m).to_string())
                .collect();
        }
    }

    // sh:131  _wanted modules expl 'Perl module' compadd "$@" -a - $perl_modules
    setaparam("_pm_subset", modules);
    let mut w = vec![
        "modules".to_string(),
        "expl".to_string(),
        "Perl module".to_string(),
        "compadd".to_string(),
    ];
    w.extend(rest);
    w.push("-a".to_string());
    w.push("-".to_string());
    w.push("_pm_subset".to_string());
    _wanted(&w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_perl_replaces_nonalnum_and_strips_leading() {
        // sh:66
        assert_eq!(sanitize_perl("/usr/bin/perl"), "usr_bin_perl");
        assert_eq!(sanitize_perl("perl"), "perl");
        assert_eq!(sanitize_perl("perl5.36"), "perl5_36");
    }

    #[test]
    fn unquote_q_strips_quotes_and_escapes() {
        assert_eq!(unquote_q("'foo'"), "foo");
        assert_eq!(unquote_q("a\\ b"), "a b");
        assert_eq!(unquote_q("plain"), "plain");
    }

    #[test]
    fn caching_policy_rebuilds_when_no_cache_file() {
        // sh:134 — missing cache file → invalid (rebuild).
        assert_eq!(
            _perl_modules_caching_policy(&["/nonexistent/zshrs/cache/file".to_string()]),
            0
        );
    }

    #[test]
    fn returns_one_with_cached_empty_modules() {
        let _g = crate::test_util::global_state_lock();
        setaparam("_perl_modules", Vec::new());
        assert_eq!(_perl_modules(&[]), 1);
    }

    #[test]
    fn hierarchy_flags_are_not_forwarded_to_compadd() {
        let _g = crate::test_util::global_state_lock();
        setaparam("_perl_modules", vec!["Foo::Bar".to_string()]);
        assert_eq!(
            _perl_modules(&[
                "--perl-hierarchy=Foo::".to_string(),
                "--strip-prefix".to_string()
            ]),
            1
        );
    }
}
