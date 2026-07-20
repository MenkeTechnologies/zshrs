//! Port of `_perl_modules` from `Completion/Unix/Type/_perl_modules`.
//!
//! Full upstream body (153 lines, abridged):
//! ```text
//! sh:  1  #compdef pmpath pmvers pmdesc pmload pmexp pmeth pmls pmcat …
//! sh: 42  _perl_modules () {
//! sh: 51    if [[ -n $argv[(r)--perl-hierarchy=*] ]]; then restrict_hierarchy=… fi
//! sh: 56    if [[ -n $argv[(r)--strip-prefix] ]]; then strip_perl_prefix=1 … fi
//! sh: 60    if [[ -n $argv[(r)-tP] ]]; then sufpat="(.pm|.pod)" … fi
//! sh: 78    if ( $+perl_modules==0 || _cache_invalid ) && ! _retrieve_cache; then
//! sh: 84      if try-to-use-pminst && $+commands[pminst]; then set -A … $(pminst)
//! sh: 87      else inc=( $(perl -e 'print "@INC"') )
//! sh: 99        for libdir in $inc; do new_pms=( $libdir/…*${~sufpat} ) … done
//! sh:113      _store_cache ${perl_modules#_} $perl_modules
//! sh:118    if [[ -n $restrict_hierarchy ]]; then perl_subset=( ${(PM)…} ) … fi
//! sh:123    _wanted modules expl 'Perl module' compadd "$@" -a - $perl_modules
//! sh:124  }
//! ```
//!
//! Approximations (`// sh:N approx`): the `pminst` / `perldoc` variants
//! (sh:64-76, 84-85), the persistent `_store_cache`/`_retrieve_cache`
//! (sh:78-113) — replaced by the in-process `_perl_modules` param cache,
//! matching the shell's `$+perl_modules` short-circuit — and the
//! `~*blib*` glob exclusion refinement are simplified. The @INC scan and
//! path→`Foo::Bar` conversion are ported faithfully.

use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::{getaparam, getsparam, setaparam};

/// sh:99-108 — recursively collect module files under `libdir`, convert
/// each to `Foo::Bar` nomenclature (strip `libdir/`, drop the suffix,
/// `/`→`::`). `pod` adds `.pod` files to the `.pm` set.
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
            let is_dir = path.is_dir();
            if is_dir {
                // ~*blib* — skip build directories.
                if path.file_name().and_then(|n| n.to_str()) == Some("blib") {
                    continue;
                }
                stack.push(path);
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let stem = if let Some(s) = name.strip_suffix(".pm") {
                Some(s)
            } else if pod {
                name.strip_suffix(".pod")
            } else {
                None
            };
            if stem.is_none() {
                continue;
            }
            // Relative path minus suffix, `/`→`::`.
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

/// sh:87  `perl -e 'print "@INC"'` — the module search path.
fn perl_inc() -> Vec<String> {
    let _ = _call_program(&[
        "perl-inc".to_string(),
        "perl".to_string(),
        "-e".to_string(),
        "print \"@INC\"".to_string(),
    ]);
    getsparam("REPLY")
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_string)
        .collect()
}

/// `_perl_modules` — complete installed Perl module names (`Foo::Bar`).
pub fn _perl_modules(args: &[String]) -> i32 {
    // sh:51-62 — extract the local flags, leaving the rest for compadd.
    let mut restrict = String::new();
    let mut strip_prefix = false;
    let mut pod = false;
    let mut rest: Vec<String> = Vec::new();
    for a in args {
        if let Some(h) = a.strip_prefix("--perl-hierarchy=") {
            // sh:52-53  `%::` then `::` re-appended.
            restrict = format!("{}::", h.trim_end_matches("::"));
        } else if a == "--strip-prefix" {
            strip_prefix = true;
        } else if a == "-tP" {
            pod = true;
        } else {
            rest.push(a.clone());
        }
    }

    // sh:78 — reuse the in-process cache (`$+perl_modules`) or rebuild.
    let cache_name = if pod {
        "_perl_modules_with_pod"
    } else {
        "_perl_modules"
    };
    if getaparam(cache_name).is_none() {
        let mut mods: Vec<String> = Vec::new();
        for libdir in perl_inc() {
            // sh:96  Ignore cwd.
            if libdir == "." || libdir.is_empty() {
                continue;
            }
            mods.extend(scan_libdir(&libdir, pod));
        }
        mods.sort();
        mods.dedup();
        setaparam(cache_name, mods);
    }
    let mut modules = getaparam(cache_name).unwrap_or_default();

    // sh:118-122 — restrict to a hierarchy, optionally stripping it.
    if !restrict.is_empty() {
        modules.retain(|m| m.starts_with(&restrict));
        if strip_prefix {
            modules = modules
                .iter()
                .map(|m| m.strip_prefix(&restrict).unwrap_or(m).to_string())
                .collect();
        }
    }

    // sh:123  _wanted modules expl 'Perl module' compadd "$@" -a - $perl_modules
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
    fn returns_one_with_cached_empty_modules() {
        let _g = crate::test_util::global_state_lock();
        setaparam("_perl_modules", Vec::new());
        assert_eq!(_perl_modules(&[]), 1);
    }

    #[test]
    fn hierarchy_flags_are_not_forwarded_to_compadd() {
        let _g = crate::test_util::global_state_lock();
        setaparam("_perl_modules", vec!["Foo::Bar".to_string()]);
        // --perl-hierarchy / --strip-prefix are consumed locally.
        assert_eq!(
            _perl_modules(&[
                "--perl-hierarchy=Foo::".to_string(),
                "--strip-prefix".to_string()
            ]),
            1
        );
    }
}
