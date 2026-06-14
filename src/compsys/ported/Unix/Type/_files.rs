//! Port of `_files` from `Completion/Unix/Type/_files`.
//!
//! Full upstream body (153 lines, abridged):
//! ```text
//! sh: 1  #compdef -redirect-,-default-,-default-
//! sh: 7  _have_glob_qual $PREFIX → glob-qualifier dispatch
//! sh:27  zparseopts -a opts  '/=tmp' 'f=tmp' 'g+:-=tmp' q n 1 2 \
//! sh:28      P: S: r: R: W: x+: X+: M+: F: J+: V+: o+:
//! sh:30  type="${(@j::M)${(@)tmp#-}#?}"  # joined flag-name string
//! sh:31  if (( $tmp[(I)-g*] )); then glob=… elif [[ $type = */* ]] glob=*(#q-/)
//! sh:46  pats = file-patterns style OR list-dirs-first OR default
//! sh:71  for def in $pats; loop tag → _tags → _next_label → _path_files -g pat
//! sh:152 done; return 1
//! ```
//!
//! Common case: dispatch `_path_files` per pattern; skip glob-
//! qualifier dispatch (defer to `_path_files` itself) and the
//! recursive-files / list-dirs-first style fine-tuning.

use crate::compsys::ported::_next_label::_next_label;
use crate::compsys::ported::_tags::_tags;
use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getsparam};

/// `_files` — file/directory completion entry point. Handles
/// `-/` (dirs only), `-f` (files only), `-g <pat>` (glob filter).
pub fn _files(args: &[String]) -> i32 {
    let mut opts: Vec<String> = Vec::new();
    let mut want_dirs_only = false;
    let mut want_files_only = false;
    let mut glob: Option<String> = None;
    let mut idx = 0usize;

    // sh:27-28  inline flag-parse (the spec is too dense for naive
    //   bin_zparseopts — peek manually).
    while idx < args.len() {
        let a = &args[idx];
        match a.as_str() {
            "-/" => {
                want_dirs_only = true;
                opts.push(a.clone());
                idx += 1;
            }
            "-f" => {
                want_files_only = true;
                opts.push(a.clone());
                idx += 1;
            }
            "-g" if idx + 1 < args.len() => {
                glob = Some(args[idx + 1].clone());
                idx += 2;
            }
            s if s.starts_with("-g") => {
                glob = Some(s[2..].to_string());
                idx += 1;
            }
            _ => break,
        }
    }
    // Pass-through extras after parsed flags
    opts.extend(args[idx..].iter().cloned());

    let _ = want_files_only;

    // sh:31  default glob for dirs-only mode
    if glob.is_none() && want_dirs_only {
        glob = Some("*(#q-/)".to_string());
    }

    // sh:46-67  pats = file-patterns style OR default
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let file_patterns = lookupstyle(&format!(":completion:{}:", curcontext), "file-patterns");
    let pats: Vec<String> = if !file_patterns.is_empty() {
        file_patterns
    } else {
        let g = glob.clone().unwrap_or_else(|| "*".to_string());
        vec![
            format!("{}:globbed-files *(-/):directories", g),
            "*:all-files".to_string(),
        ]
    };

    let mut ret: i32 = 1;
    let mut tried: Vec<String> = Vec::new();

    // sh:71  for def in $pats
    for def in &pats {
        let parts: Vec<&str> = def.split(' ').filter(|s| !s.is_empty()).collect();
        for sdef in &parts {
            // sh:81-92 — pat:tag[:descr] split
            let s = sdef.replace("\\:", "\x01"); // protect escaped colons
            let mut sp = s.splitn(3, ':');
            let pat = sp.next().unwrap_or("").replace('\x01', ":");
            let tag = sp.next().unwrap_or("").replace('\x01', ":");
            let descr = sp.next().unwrap_or("file").replace('\x01', ":");
            if pat.is_empty() {
                continue;
            }
            if tried.contains(&pat) {
                continue;
            }
            tried.push(pat.clone());

            // sh:104
            let _ = _tags(&[tag.clone()]);
            loop {
                if _tags(&[]) != 0 {
                    break;
                }
                let nl = vec![tag.clone(), "expl".to_string(), descr.clone()];
                if _next_label(&nl) != 0 {
                    break;
                }
                // sh:117  _path_files -g pat "$expl[@]"
                let mut pf_argv: Vec<String> = opts.clone();
                pf_argv.push("-g".to_string());
                pf_argv.push(pat.clone());
                let expl = getaparam("expl").unwrap_or_default();
                pf_argv.extend(expl);
                if dispatch_function_call("_path_files", &pf_argv).unwrap_or(1) == 0 {
                    ret = 0;
                }
            }
            // sh:151  `*` short-circuit
            if pat == "*" {
                return ret;
            }
        }
        if ret == 0 {
            return 0;
        }
    }
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_files(&[]), 1);
    }

    #[test]
    fn accepts_dirs_only_flag() {
        let _g = crate::test_util::global_state_lock();
        let _r = _files(&["-/".to_string()]);
    }
}
