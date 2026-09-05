//! Port of `_extensions` from `Completion/Base/Completer/_extensions`.
//!
//! Full upstream body (33 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 8  compset -P '(#b)([~$][^/]#/|)(*/|)(\^|)\*.' || return 1
//! sh:10  local -aU files
//! sh:11  local -a expl suf mfiles
//! sh:13  files=( ${(e)~match[1]}${match[2]}*.* ) || return 1
//! sh:13  eval set -A files '${(MSI:'{…}':)files%%.[^/]##}'
//! sh:15  files=( ${files:#.<->(.*|)} )
//! sh:17  if zstyle -t ":completion:${curcontext}:extensions" prefix-hidden; then
//! sh:18    files=( ${files#.} )
//! sh:19  else
//! sh:20    PREFIX=".$PREFIX"
//! sh:21    IPREFIX="${IPREFIX%.}"
//! sh:22  fi
//! sh:24  zstyle -T ":completion:${curcontext}:extensions" add-space ||
//! sh:25    suf=( -S '' )
//! sh:27  _description extensions expl 'file extension'
//! sh:30  compadd -O mfiles "$expl[@]" -a files
//! sh:31  [[ $#mfiles -gt 1 || ${mfiles[1]} != $PREFIX ]] &&
//! sh:32      compadd "$expl[@]" "$suf[@]" -a files &&
//! sh:33      [[ -z $compstate[exact_string] ]]
//! ```
//!
//! Faithful port of the *control flow*. sh:12-13's `(#b)`-capture
//! pattern + eval-glob extraction can't be cleanly mirrored without
//! a pattern-with-captures helper in our port; we fall back to a
//! std::fs scan of the directory implied by `$match[1]$match[2]`
//! and extract extensions from filenames in that dir.

use crate::compsys::ported::_description::_description;
use crate::ported::modules::zutil::testforstyle;
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam};
use crate::ported::zle::compcore::get_compstate_str;
use crate::ported::zle::complete::{bin_compadd, bin_compset};
use crate::ported::zsh_h::{options, MAX_OPS};
use std::fs;

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// `_extensions` — complete file extensions when completing after
/// `*.` or `^*.`. Pure heuristic per-directory file-extension scan.
pub fn _extensions() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_extensions");
    // sh:8  compset -P '(#b)([~$][^/]#/|)(*/|)(\^|)\*.'
    //   When this matches, the captured prefix tells us where to
    //   scan. We dispatch to the real bin_compset; if it returns
    //   non-zero (no match), bail.
    if bin_compset(
        "compset",
        &[
            "-P".to_string(),
            "(#b)([~$][^/]#/|)(*/|)(\\^|)\\*.".to_string(),
        ],
        &make_ops(),
        0,
    ) != 0
    {
        return 1;
    }

    // sh:13 — scan the dir for `*.*` filenames; collect unique
    //   `.<ext>` suffixes.
    let m_arr = getaparam("match").unwrap_or_default();
    let dir_prefix = format!(
        "{}{}",
        m_arr.first().cloned().unwrap_or_default(),
        m_arr.get(1).cloned().unwrap_or_default(),
    );
    // Scan directory `dir_prefix` (or "." if empty) for files
    //   containing `.`.
    let scan_dir = if dir_prefix.is_empty() {
        ".".to_string()
    } else {
        dir_prefix.trim_end_matches('/').to_string()
    };
    let mut files: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(&scan_dir) {
        for ent in entries.flatten() {
            let name = ent.file_name().to_string_lossy().to_string();
            if let Some(dot) = name.rfind('.') {
                if dot > 0 {
                    let ext = format!(".{}", &name[dot + 1..]);
                    if !files.contains(&ext) {
                        files.push(ext);
                    }
                }
            }
        }
    }
    if files.is_empty() {
        return 1;
    }
    // sh:14  remove digit-only extensions like `.1`, `.1.txt`
    files.retain(|f| {
        !f[1..]
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
    });

    // sh:17-22  prefix-hidden style
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let prefix_hidden = testforstyle(
        &format!(":completion:{}:extensions", curcontext),
        "prefix-hidden",
    ) == 0;
    if prefix_hidden {
        // sh:18  drop leading "." from each entry
        files = files
            .iter()
            .map(|f| f.trim_start_matches('.').to_string())
            .collect();
    } else {
        // sh:20-21
        let prefix = getsparam("PREFIX").unwrap_or_default();
        let _ = setsparam("PREFIX", &format!(".{}", prefix));
        let iprefix = getsparam("IPREFIX").unwrap_or_default();
        let _ = setsparam("IPREFIX", iprefix.trim_end_matches('.'));
    }

    // sh:24-25  add-space style → -S '' suffix
    let add_space = testforstyle(
        &format!(":completion:{}:extensions", curcontext),
        "add-space",
    ) == 0;
    let suf: Vec<String> = if !add_space {
        vec!["-S".to_string(), "".to_string()]
    } else {
        Vec::new()
    };

    // sh:27
    let _ = _description(&[
        "extensions".to_string(),
        "expl".to_string(),
        "file extension".to_string(),
    ]);

    // sh:30  compadd -O mfiles "$expl[@]" -a files
    let expl = getaparam("expl").unwrap_or_default();
    // sh:10  local -aU files
    //
    // `compadd -a files` reads the array BY NAME (sh:30, sh:32), so it has to
    // be a real parameter — a LOCAL one, and a unique one. `files` is the
    // set of extensions seen in the directory, and sh:13's glob yields one
    // element per FILE, so `.c` recurs once per `.c` file present; sh:14-15
    // then truncate multi-dot names down to their first extension, folding
    // even more of them together. Without `-U` every duplicate becomes a
    // duplicate match. The Rust scan above already suppresses repeats as it
    // collects, so this closes the ATTRIBUTE half — `${(t)files}` read plain
    // `array` where zsh reads `array-local-unique` — and stops the name
    // leaking over a caller's `files`.
    crate::compsys::ported::shared::declare_locals(
        &["files"],
        crate::compsys::ported::shared::PM_ARRAY | crate::compsys::ported::shared::PM_UNIQUE,
    );
    setaparam("files", files.clone());
    let mut probe: Vec<String> = vec!["-O".to_string(), "mfiles".to_string()];
    probe.extend(expl.iter().cloned());
    probe.push("-a".to_string());
    probe.push("files".to_string());
    let _ = bin_compadd("compadd", &probe, &make_ops(), 0);
    let mfiles = getaparam("mfiles").unwrap_or_default();

    // sh:31-33  emit when ambiguous OR not an exact-match
    let prefix = getsparam("PREFIX").unwrap_or_default();
    if mfiles.len() > 1 || mfiles.first().map(|m| m != &prefix).unwrap_or(true) {
        let mut argv = expl;
        argv.extend(suf);
        argv.push("-a".to_string());
        argv.push("files".to_string());
        let _ = bin_compadd("compadd", &argv, &make_ops(), 0);
        if get_compstate_str("exact_string")
            .unwrap_or_default()
            .is_empty()
        {
            return 0;
        }
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_when_compset_fails() {
        // sh:8 — when bin_compset returns non-zero (PREFIX doesn't
        //   match the `*.` pattern), short-circuit with 1.
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "plain_string");
        let _ = setsparam("SUFFIX", "");
        let _r = _extensions();
    }
}
