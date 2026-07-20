//! Port of `_list_files` from `Completion/Unix/Type/_list_files`.
//!
//! Helper for `_path_files` implementing the `file-list` style — the
//! `ls -l`-style long display of file matches.
//!
//! Full upstream body (69 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh:12  listfiles=(); listopts=()
//! sh:14  zstyle -a … file-list stylevals || return 1
//! sh:18  case $WIDGETSTYLE in (*complete*) what=insert;; (*) what=list;; esac
//! sh:28  for elt in $stylevals; do   # decide `ok` (use long format?)
//! sh:30    (*($what|all|true|1|yes)*=<->)  (( ${(P)#1} <= ${elt##*=} )) && ok=1
//! sh:35    ([^=]#($what|all|true|1|yes)[^=]#)  ok=1
//! sh:41  (( ok )) || return 1
//! sh:43  zmodload -F zsh/stat b:zstat … || return 1
//! sh:45  dir=${2:+$2/}; dir=${(Q)dir}
//! sh:47  for f in ${(PQ)1}; do
//! sh:48    [[ ! -e "$dir$f" ]] && listfiles+=("$dir$f") && continue
//! sh:53    zstat -s -H stat -F "%b %e %H:%M" - "$dir$f"
//! sh:55    listfiles+=("$stat[mode] nlink uid gid size mtime $f")
//! sh:60  (( ${#listfiles} )) && listopts=(-d listfiles -l -o match)
//! sh:62  return 0
//! ```
//!
//! sh:53 approx — `zstat -s` (string mode) shows uid/gid as *names*; this port
//! emits numeric uid/gid. The mode string and `%b %e %H:%M` mtime are exact.

use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getsparam, setaparam};
use chrono::{Local, TimeZone};
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;

/// sh:53 — symbolic mode string, e.g. `-rw-r--r--` / `drwxr-xr-x`.
fn mode_string(mode: u32) -> String {
    let ft = match mode & 0o170000 {
        0o040000 => 'd',
        0o120000 => 'l',
        0o060000 => 'b',
        0o020000 => 'c',
        0o010000 => 'p',
        0o140000 => 's',
        _ => '-',
    };
    let mut s = String::with_capacity(10);
    s.push(ft);
    for shift in [6, 3, 0] {
        let bits = (mode >> shift) & 0o7;
        s.push(if bits & 0o4 != 0 { 'r' } else { '-' });
        s.push(if bits & 0o2 != 0 { 'w' } else { '-' });
        s.push(if bits & 0o1 != 0 { 'x' } else { '-' });
    }
    s
}

/// `${(Q)s}` — strip one level of backslash quoting.
fn unquote(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            if let Some(n) = it.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// `_list_files` — build the `file-list` long-format display for `_path_files`.
/// `args[0]` names the array of matched files; `args[1]` (optional) is the
/// directory prefix. Populates the `listfiles` / `listopts` params.
pub fn _list_files(args: &[String]) -> i32 {
    // sh:12
    setaparam("listfiles", Vec::new());
    setaparam("listopts", Vec::new());

    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctx = format!(":completion:{}:", curcontext);
    // sh:14
    let stylevals = lookupstyle(&ctx, "file-list");
    if stylevals.is_empty() {
        return 1;
    }

    // sh:18-24
    let widgetstyle = getsparam("WIDGETSTYLE").unwrap_or_default();
    let what = if widgetstyle.contains("complete") {
        "insert"
    } else {
        "list"
    };
    let keywords = [what, "all", "true", "1", "yes"];
    let name = args.first().cloned().unwrap_or_default();
    let nmatch = getaparam(&name).map(|a| a.len()).unwrap_or(0);

    // sh:28-40 — decide whether to use the long format.
    let mut ok = false;
    for elt in &stylevals {
        if let Some(eqpos) = elt.rfind('=') {
            let (key, num) = (&elt[..eqpos], &elt[eqpos + 1..]);
            // sh:30 — keyword before `=`, numeric threshold after.
            if keywords.iter().any(|k| key.contains(k)) {
                if let Ok(threshold) = num.trim().parse::<usize>() {
                    if nmatch <= threshold {
                        ok = true;
                    }
                    break;
                }
            }
        } else if keywords.iter().any(|k| elt.contains(k)) {
            // sh:35 — keyword, no `=`: always long format.
            ok = true;
            break;
        }
    }
    // sh:41
    if !ok {
        return 1;
    }

    // sh:45  dir=${2:+$2/}; dir=${(Q)dir}
    let dir = match args.get(1) {
        Some(d) if !d.is_empty() => unquote(&format!("{}/", d)),
        _ => String::new(),
    };

    // sh:47  for f in ${(PQ)1}
    let files = getaparam(&name).unwrap_or_default();
    let mut listfiles: Vec<String> = Vec::new();
    for raw in &files {
        let f = unquote(raw);
        let full = format!("{}{}", dir, f);
        // sh:48 — non-existent match: display the bare name.
        let Ok(md) = std::fs::symlink_metadata(&full) else {
            listfiles.push(full);
            continue;
        };
        // sh:53-58 — mode nlink uid gid size mtime name.
        let mode = mode_string(md.permissions().mode());
        let mtime = Local
            .timestamp_opt(md.mtime(), 0)
            .single()
            .map(|t| t.format("%b %e %H:%M").to_string())
            .unwrap_or_default();
        listfiles.push(format!(
            "{} {:>3} {:<8} {:<8} {:>8} {} {}",
            mode,
            md.nlink(),
            md.uid(),
            md.gid(),
            md.size(),
            mtime,
            f
        ));
    }

    // sh:60 — non-empty → set the compadd display opts.
    if !listfiles.is_empty() {
        setaparam(
            "listopts",
            vec![
                "-d".to_string(),
                "listfiles".to_string(),
                "-l".to_string(),
                "-o".to_string(),
                "match".to_string(),
            ],
        );
    }
    setaparam("listfiles", listfiles);
    // sh:62
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_string_regular_and_dir() {
        assert_eq!(mode_string(0o100644), "-rw-r--r--");
        assert_eq!(mode_string(0o040755), "drwxr-xr-x");
    }

    #[test]
    fn returns_one_without_file_list_style() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::setsparam("curcontext", ":completion::::");
        // No file-list style registered → sh:14 `return 1`.
        assert_eq!(_list_files(&["nomatches".to_string()]), 1);
    }
}
