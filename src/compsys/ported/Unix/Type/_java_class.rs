//! Port of `_java_class` from `Completion/Unix/Type/_java_class`.
//!
//! Full upstream body (25 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 6  local classpath i expl; local -a c; local method type
//! sh:10  zparseopts -D -E -a classpath t:=type m:=method cp: classpath:
//! sh:12  classpath="${${classpath[2]:-${CLASSPATH:-.}}//\\:/:}"
//! sh:15  for i in "${(s.:.)classpath}"; do
//! sh:16    [[ -z $i ]] && i=.
//! sh:17    if [[ -f $i ]] && [[ "$i" == *.(jar|zip|war|ear) ]]; then
//! sh:17      c+=( ${${${(M)$(_call_program jar_classes jar -tf $i)##*.class}%%.class}:gs#/#.#} )
//! sh:19    elif [[ -d $i ]]; then
//! sh:20      c+=( $i/**/*.class(.:r:s/.class//:s#$i/##:gs#/#.#) )
//! sh:24  _wanted classes expl 'java class' compadd "$@" -M 'r:|.=* r:|=*' -a - c
//! ```
//!
//! sh:17 — jars via `_call_program jar -tf` (REPLY), keeping `*.class`
//! entries in package form. sh:20 — directories through the glob engine
//! with the upstream pattern and qualifier ([`glob_classes`]).

use crate::compsys::ported::_call_program::call_program_capture;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::glob::{tokenize, zglob};
use crate::ported::params::{getsparam, setaparam};
use crate::ported::utils::quotestring;
use crate::ported::zsh_h::QT_BACKSLASH_PATTERN;
use std::path::Path;

/// sh:9 — pull `-t`/`-m`/`-cp`/`--classpath` (each takes a value) out of the
/// argv, returning (classpath-value, remaining-args). `-D` removes the parsed
/// options from the positional list handed to compadd.
fn parse_opts(args: &[String]) -> (Option<String>, Vec<String>) {
    let mut cp: Option<String> = None;
    let mut rest: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        let takes_val = matches!(
            a.as_str(),
            "-t" | "-m" | "-cp" | "-classpath" | "--classpath"
        );
        if takes_val {
            let val = args.get(i + 1).cloned().unwrap_or_default();
            if matches!(a.as_str(), "-cp" | "-classpath" | "--classpath") {
                cp = Some(val);
            }
            i += 2;
        } else {
            rest.push(a.clone());
            i += 1;
        }
    }
    (cp, rest)
}

/// sh:20 — `c+=( $i/**/*.class(.:r:s/.class//:s#$i/##:gs#/#.#) )`, run by
/// the glob engine. `**/` neither follows a symlinked directory
/// (c:Src/glob.c:724 `follow = (instr[2] == Star)`, false for two stars)
/// nor enters a dot-directory (PAT_NOGLD), and the qualifier's modifiers
/// produce the dotted names directly. A hand-written walk that followed
/// links and descended `.git`/`target` took 8.5s in this repository where
/// zsh takes 1s.
///
/// `$i` is spliced in as a parameter value: quoted in the path (not a
/// pattern), literal inside `:s#…##` (`:s` matches a plain string).
fn glob_classes(i: &str) -> Vec<String> {
    let mut word = format!(
        "{}/**/*.class(.:r:s/.class//:s#{}/##:gs#/#.#)",
        quotestring(i, QT_BACKSLASH_PATTERN),
        i
    );
    tokenize(&mut word);
    let mut list = vec![word];
    zglob(&mut list, 0, 0);
    list
}

/// `_java_class` — complete fully-qualified Java class names from a classpath.
pub fn _java_class(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_java_class");
    // sh:6 `local classpath i expl` and sh:7 `local -a c` — two
    // declaration lines, so two calls with the kinds the shell spells.
    // `c` is assigned with `setaparam` at rs:116 and `expl` is filled by
    // `_description` through the name handed to `_wanted` at rs:119; both
    // were born at level 0 (shared.rs:16-30). `classpath`, `i`, `method`
    // and `type` stay Rust-side. Measured on `lkjavacls <TAB>`: zsh
    // leaves both unset, zshrs left both populated — and `c` is a
    // one-letter name, so leaking it clobbers any caller loop variable
    // of that spelling.
    crate::compsys::ported::shared::declare_locals(&["expl"], 0);
    crate::compsys::ported::shared::declare_locals(
        &["c"],
        crate::compsys::ported::shared::PM_ARRAY,
    );
    // sh:9-11 — classpath = -cp value, else $CLASSPATH, else `.`; `\:` → `:`.
    let (cpval, rest) = parse_opts(args);
    let classpath = cpval
        .filter(|s| !s.is_empty())
        .or_else(|| getsparam("CLASSPATH").filter(|s| !s.is_empty()))
        .unwrap_or_else(|| ".".to_string())
        .replace("\\:", ":");

    // sh:14-21
    let mut c: Vec<String> = Vec::new();
    for raw in classpath.split(':') {
        let i = if raw.is_empty() { "." } else { raw };
        let p = Path::new(i);
        let is_archive = matches!(
            Path::new(i).extension().and_then(|e| e.to_str()),
            Some("jar") | Some("zip") | Some("war") | Some("ear")
        );
        if p.is_file() && is_archive {
            // sh:17 — jar -tf, keep `*.class`, strip `.class`, `/` → `.`.
            let _ = call_program_capture(&[
                "jar_classes".to_string(),
                "jar".to_string(),
                "-tf".to_string(),
                i.to_string(),
            ]);
            let reply = getsparam("REPLY").unwrap_or_default();
            for line in reply.split_whitespace() {
                if let Some(stem) = line.strip_suffix(".class") {
                    c.push(stem.replace('/', "."));
                }
            }
        } else if p.is_dir() {
            // sh:20
            c.extend(glob_classes(i));
        }
    }

    // sh:24 — _wanted classes expl 'java class' compadd "$@" -M … -a - c
    setaparam("c", c);
    let mut w = vec![
        "classes".to_string(),
        "expl".to_string(),
        "java class".to_string(),
        "compadd".to_string(),
    ];
    w.extend(rest);
    w.push("-M".to_string());
    w.push("r:|.=* r:|=*".to_string());
    w.push("-a".to_string());
    w.push("-".to_string());
    w.push("c".to_string());
    _wanted(&w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_java_class(&[]), 1);
    }

    /// sh:20 on a layout with a symlinked package directory and a
    /// dot-directory. Real zsh 5.9.2 gives `com.ex.A com.ex.sub.B Top`
    /// for both `i=.` (from inside) and `i=<absolute dir>`: `**/` does
    /// not follow `lnk -> com` and skips `.hid`.
    #[test]
    fn class_glob_skips_symlinked_and_dot_directories() {
        let _g = crate::test_util::global_state_lock();
        let dir = std::env::temp_dir().join(format!("zshrs_javacls_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("com/ex/sub")).unwrap();
        std::fs::create_dir_all(dir.join(".hid")).unwrap();
        for f in ["com/ex/A.class", "com/ex/sub/B.class", "Top.class", ".hid/H.class", "com/x.txt"] {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        std::os::unix::fs::symlink(dir.join("com"), dir.join("lnk")).unwrap();
        let mut got = glob_classes(&dir.to_string_lossy());
        got.sort();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(got, vec!["Top", "com.ex.A", "com.ex.sub.B"]);
    }

    #[test]
    fn parse_opts_extracts_classpath_and_keeps_rest() {
        let (cp, rest) = parse_opts(&[
            "-cp".to_string(),
            "/a:/b".to_string(),
            "-J".to_string(),
            "grp".to_string(),
        ]);
        assert_eq!(cp, Some("/a:/b".to_string()));
        assert_eq!(rest, vec!["-J".to_string(), "grp".to_string()]);
    }
}
