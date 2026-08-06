//! Port of `_java_class` from `Completion/Unix/Type/_java_class`.
//!
//! Full upstream body (25 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 6  local classpath i expl; local -a c; local method type
//! sh: 9  zparseopts -D -E -a classpath t:=type m:=method cp: classpath:
//! sh:11  classpath="${${classpath[2]:-${CLASSPATH:-.}}//\\:/:}"
//! sh:14  for i in "${(s.:.)classpath}"; do
//! sh:15    [[ -z $i ]] && i=.
//! sh:16    if [[ -f $i ]] && [[ "$i" == *.(jar|zip|war|ear) ]]; then
//! sh:17      c+=( ${${${(M)$(_call_program jar_classes jar -tf $i)##*.class}%%.class}:gs#/#.#} )
//! sh:18    elif [[ -d $i ]]; then
//! sh:19      c+=( $i/**/*.class(.:r:s/.class//:s#$i/##:gs#/#.#) )
//! sh:23  _wanted classes expl 'java class' compadd "$@" -M 'r:|.=* r:|=*' -a - c
//! ```
//!
//! sh:17/19 approx — class enumeration is done in Rust: jars via
//! `_call_program jar -tf` (REPLY), directories via a recursive `*.class`
//! walk; both strip the `.class` suffix and map `/` → `.` to package form.

use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_wanted::wanted_byname;
use crate::ported::params::{getsparam, setaparam};
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

/// sh:19 — recursively collect `*.class` files under `dir`, returning the
/// package-dotted class names (`$dir/` stripped, `.class` stripped, `/`→`.`).
fn walk_classes(dir: &Path, base: &str, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for ent in rd.flatten() {
        let path = ent.path();
        if path.is_dir() {
            walk_classes(&path, base, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("class") {
            if let Some(full) = path.to_str() {
                // strip leading `$base/`, drop `.class`, `/` → `.`
                let rel = full.strip_prefix(&format!("{}/", base)).unwrap_or(full);
                let stem = rel.strip_suffix(".class").unwrap_or(rel);
                out.push(stem.replace('/', "."));
            }
        }
    }
}

/// `_java_class` — complete fully-qualified Java class names from a classpath.
pub fn _java_class(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_java_class");
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
            let _ = _call_program(&[
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
            // sh:19
            walk_classes(p, i, &mut c);
        }
    }

    // sh:23 — _wanted classes expl 'java class' compadd "$@" -M … -a - c
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
    wanted_byname(&w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_java_class(&[]), 1);
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
