//! Port of `_terminals` from `Completion/Unix/Type/_terminals`.
//!
//! Full upstream body (37 lines verbatim):
//! ```text
//! sh: 1  #compdef infocmp -value-,TERM,-default-
//! sh: 3  local entry
//! sh: 4  local -aU desc
//! sh: 5  local -a terms names
//! sh: 7  desc=(
//! sh: 8    $TERMINFO ~/.terminfo $TERMINFO_DIRS /usr/{,share/}{,lib/}terminfo
//! sh: 9    /{etc,lib}/terminfo
//! sh:10  )
//! sh:11  desc=( $desc(N:P) ) # may have symlinks to the same path
//! sh:12  terms=( $desc/*/^*+?*(N:t) ) # entries named with a + are common includes
//! sh:14  if [[ $OSTYPE = (freebsd|dragonfly)* ]]; then
//! sh:15    while read entry; do
//! sh:16      [[ "$entry" != [^[:blank:]\#\*_]*:* ]] && continue
//! sh:18      names=( ${${(s:|:)entry%%:*}##[[:blank:]]#} )
//! sh:19      if [[ $#names -gt 1 && $names[-1] = *\ * ]]; then
//! sh:20        terms+=( ${^names[1,-2]:#*[ +]?*}:${names[-1]} )
//! sh:21      else
//! sh:22        terms+=( ${names:#*\ *} )
//! sh:23      fi
//! sh:24    done < /etc/termcap
//! sh:26  elif [[ $OSTYPE = netbsd* ]]; then
//! sh:27    grep $'^[^#\t]*,$' /usr/share/misc/terminfo | while read entry; do
//! sh:28      names=( ${(s:|:)entry%,} )
//! sh:29      if [[ $#names -gt 1 && $names[-1] = *\ * ]]; then
//! sh:30        terms+=( ${^names[1,-2]:#*[ +]?*}:${names[-1]} )
//! sh:31      else
//! sh:32        terms+=( ${names:#*\ *} )
//! sh:33      fi
//! sh:34    done
//! sh:35  fi
//! sh:37  _describe -t terminals 'terminal name' terms "$@"
//! ```
//!
//! sh:8 brace expansion is precomputed into the concrete terminfo
//! directory list.
//!
//! sh:11 and sh:12 are evaluated under the `$_comp_setup` option set
//! (`Completion/compinit:139-171` — `rcexpandparam`, `nullglob`,
//! `extendedglob`, `bareglobqual`), so the trailing glob qualifiers
//! distribute over EVERY element of `$desc`, not just the last one:
//!
//!   * sh:11 `$desc(N:P)` — each element is globbed with `N` (drop the
//!     word when it matches nothing, i.e. when the directory does not
//!     exist) and rewritten by the `:P` modifier (realpath: resolve
//!     every symlink component). The result lands back in an `-aU`
//!     array, so two entries that are symlinks to one directory
//!     collapse — the point of the sh:11 comment.
//!   * sh:12 `$desc/*/^*+?*(N:t)` — two levels down from each
//!     directory, EXCLUDING (`^`, extended-glob negation) every entry
//!     whose name matches `*+?*`, i.e. every name with a `+` that is
//!     not its last character. Those are terminfo "common include"
//!     stanzas (`xterm+sl-twm`, `hp+pfk-cr`, …), not terminal names.
//!     `:t` keeps the basename.
//!
//! The previous port of this file transcribed a much older upstream
//! (`_wanted terminals expl 'terminal name' compadd "$@" - $desc/*/*(N:t)`).
//! Losing sh:12's `^*+?*` negation admitted the 165 include stanzas on
//! this host; the 6 of them that also contain a `-` survived the
//! `r:|?=**` matcher and made the group one match taller than zsh's,
//! which shifted every column of the `infocmp -<TAB>` listing.

use crate::compsys::ported::_describe::_describe;
use crate::compsys::ported::shared::{declare_locals, PM_ARRAY, PM_UNIQUE};
use crate::ported::glob::{tokenize, zglob};
use crate::ported::params::{getsparam, setaparam};

/// sh:12 — glob one `$desc` element two levels deep with the `^*+?*`
/// exclusion and the `(N:t)` qualifier list, i.e. the upstream pattern
/// verbatim: `N` drops the word when nothing matched, `:t` reduces each
/// match to its basename. The trailing filter is belt-and-braces for a
/// caller that reaches here with `NULL_GLOB`/`BARE_GLOB_QUAL` off, when
/// an unmatched pattern would otherwise survive as a literal.
fn glob_basenames(pat: &str) -> Vec<String> {
    let mut list = {
        let mut s = pat.to_string();
        tokenize(&mut s);
        vec![s]
    };
    zglob(&mut list, 0, 0);
    list.into_iter()
        .filter(|e| {
            !e.as_bytes()
                .iter()
                .any(|&b| matches!(b, b'*' | b'?' | b'[' | b']' | b'^'))
        })
        .collect()
}

/// sh:18/sh:28 + sh:19-23/sh:29-33 — turn one termcap/terminfo header
/// line's alias list into the `terms` entries it contributes.
///
/// `names` is the `|`-separated alias list; when it has more than one
/// element and the last one contains a space it is the human-readable
/// description, so every remaining alias is emitted as
/// `alias:description` (sh:20 / sh:30) with the aliases that match
/// `*[ +]?*` dropped. Otherwise the aliases without a space are
/// emitted bare (sh:22 / sh:32).
fn termcap_names_to_terms(names: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let last = match names.last() {
        Some(l) => l,
        None => return out,
    };
    if names.len() > 1 && last.contains(' ') {
        // sh:20 / sh:30 — `${^names[1,-2]:#*[ +]?*}:${names[-1]}`
        for n in &names[..names.len() - 1] {
            let b = n.as_bytes();
            // `*[ +]?*`: a space or `+` with at least one character
            // after it.
            let excluded = b
                .iter()
                .position(|&c| c == b' ' || c == b'+')
                .is_some_and(|i| i + 1 < b.len());
            if !excluded {
                out.push(format!("{}:{}", n, last));
            }
        }
    } else {
        // sh:22 / sh:32 — `${names:#*\ *}`
        out.extend(names.iter().filter(|n| !n.contains(' ')).cloned());
    }
    out
}

/// `_terminals` — complete terminal names from the terminfo database.
pub fn _terminals(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_terminals");
    // sh:3-5 — `local entry`, `local -aU desc`, `local -a terms names`.
    declare_locals(&["entry"], 0);
    declare_locals(&["desc"], PM_ARRAY | PM_UNIQUE);
    declare_locals(&["terms", "names"], PM_ARRAY);

    // sh:7-10 — desc=( $TERMINFO ~/.terminfo $TERMINFO_DIRS
    //   /usr/{,share/}{,lib/}terminfo /{etc,lib}/terminfo )
    let home = getsparam("HOME").unwrap_or_default();
    let mut desc: Vec<String> = Vec::new();
    if let Some(ti) = getsparam("TERMINFO").filter(|s| !s.is_empty()) {
        desc.push(ti);
    }
    desc.push(format!("{}/.terminfo", home));
    if let Some(tid) = getsparam("TERMINFO_DIRS").filter(|s| !s.is_empty()) {
        // `$TERMINFO_DIRS` is colon-separated in the environment.
        desc.extend(tid.split(':').filter(|s| !s.is_empty()).map(String::from));
    }
    // /usr/{,share/}{,lib/}terminfo — the four brace-cross members, in
    // brace-expansion order.
    for mid in ["", "lib/", "share/", "share/lib/"] {
        desc.push(format!("/usr/{}terminfo", mid));
    }
    // /{etc,lib}/terminfo
    desc.push("/etc/terminfo".to_string());
    desc.push("/lib/terminfo".to_string());

    // sh:11 — desc=( $desc(N:P) ): `N` drops the elements that name no
    // existing directory, `:P` resolves symlinks, and the `-aU` array
    // (sh:4) then collapses the duplicates that resolution exposes.
    let mut resolved: Vec<String> = Vec::new();
    for d in &desc {
        let p = match std::fs::canonicalize(d) {
            Ok(p) => p.to_string_lossy().into_owned(),
            Err(_) => continue, // (N) — no match, word removed
        };
        if !resolved.contains(&p) {
            resolved.push(p);
        }
    }
    desc = resolved;
    setaparam("desc", desc.clone());

    // sh:12 — terms=( $desc/*/^*+?*(N:t) )
    let mut terms: Vec<String> = Vec::new();
    for d in &desc {
        terms.extend(glob_basenames(&format!("{}/*/^*+?*(N:t)", d)));
    }

    // sh:14-35 — the BSD termcap/terminfo text databases, which list
    // aliases the directory tree does not.
    let ostype = getsparam("OSTYPE").unwrap_or_default();
    if ostype.starts_with("freebsd") || ostype.starts_with("dragonfly") {
        // sh:15-24 — `while read entry; do … done < /etc/termcap`
        if let Ok(text) = std::fs::read_to_string("/etc/termcap") {
            for entry in text.lines() {
                // sh:16 — `[[ "$entry" != [^[:blank:]\#\*_]*:* ]] && continue`
                let first = match entry.chars().next() {
                    Some(c) => c,
                    None => continue,
                };
                if first == ' ' || first == '\t' || first == '#' || first == '*' || first == '_' {
                    continue;
                }
                if !entry.contains(':') {
                    continue;
                }
                // sh:18 — `names=( ${${(s:|:)entry%%:*}##[[:blank:]]#} )`
                let names: Vec<String> = entry
                    .split(':')
                    .next()
                    .unwrap_or("")
                    .split('|')
                    .map(|n| n.trim_start_matches([' ', '\t']).to_string())
                    .collect();
                terms.extend(termcap_names_to_terms(&names));
            }
        }
    } else if ostype.starts_with("netbsd") {
        // sh:27-34 — `grep $'^[^#\t]*,$' /usr/share/misc/terminfo |
        //   while read entry; do … done`
        if let Ok(text) = std::fs::read_to_string("/usr/share/misc/terminfo") {
            for entry in text.lines() {
                // grep `^[^#\t]*,$` — no `#`/TAB before a trailing comma.
                if !entry.ends_with(',') {
                    continue;
                }
                if entry[..entry.len() - 1].contains(['#', '\t']) {
                    continue;
                }
                // sh:28 — `names=( ${(s:|:)entry%,} )`
                let names: Vec<String> = entry[..entry.len() - 1]
                    .split('|')
                    .map(str::to_string)
                    .collect();
                terms.extend(termcap_names_to_terms(&names));
            }
        }
    }

    // sh:37 — _describe -t terminals 'terminal name' terms "$@"
    setaparam("terms", terms);
    let mut describe_argv: Vec<String> = vec![
        "-t".to_string(),
        "terminals".to_string(),
        "terminal name".to_string(),
        "terms".to_string(),
    ];
    describe_argv.extend(args.iter().cloned());
    _describe(&describe_argv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = _terminals(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 1);
    }

    /// sh:12 — `^*+?*` excludes the terminfo "common include" stanzas:
    /// a `+` anywhere but the final character disqualifies the name.
    ///
    /// The negation only exists under `EXTENDED_GLOB`, which every
    /// compsys entry point turns on via `$_comp_setup`
    /// (`Completion/compinit:141`); the test sets it explicitly since a
    /// bare unit-test process starts from the default option set.
    #[test]
    fn plus_include_stanzas_are_excluded() {
        let _g = crate::test_util::global_state_lock();
        let had = crate::ported::zsh_h::isset(crate::ported::zsh_h::EXTENDEDGLOB);
        crate::ported::options::opt_state_set("extendedglob", true);
        let dir = std::env::temp_dir().join(format!("zshrs_terminals_{}", std::process::id()));
        let sub = dir.join("78");
        let _ = std::fs::create_dir_all(&sub);
        for name in ["xterm", "xterm+sl-twm", "hp+pfk-cr", "vt100+", "qvt119+-w"] {
            let _ = std::fs::write(sub.join(name), b"");
        }
        let mut got = glob_basenames(&format!("{}/*/^*+?*(N:t)", dir.display()));
        got.sort();
        let _ = std::fs::remove_dir_all(&dir);
        if !had {
            crate::ported::options::opt_state_unset("extendedglob");
        }
        // `vt100+` keeps its trailing `+` (nothing follows it), the two
        // `+X` includes are dropped.
        assert_eq!(got, vec!["vt100+".to_string(), "xterm".to_string()]);
    }

    /// sh:19-23 — the alias list of a termcap header line, with and
    /// without a trailing human-readable description.
    #[test]
    fn termcap_alias_lists_split_into_terms() {
        // Description present (last field has a space): every other
        // alias is emitted as `alias:description`, and aliases holding
        // a `+`/space before their last character are dropped.
        let names: Vec<String> = ["vt100", "vt100+", "vt1 00", "vt100-am", "DEC VT100"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            termcap_names_to_terms(&names),
            vec![
                "vt100:DEC VT100".to_string(),
                "vt100+:DEC VT100".to_string(),
                "vt100-am:DEC VT100".to_string(),
            ]
        );
        // No description (the last alias holds no space): bare aliases,
        // minus the ones that do hold a space.
        let names: Vec<String> = ["vt100", "vt 100", "vt100-am"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            termcap_names_to_terms(&names),
            vec!["vt100".to_string(), "vt100-am".to_string()]
        );
    }
}
