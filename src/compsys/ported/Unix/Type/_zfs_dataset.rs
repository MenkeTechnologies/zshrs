//! Port of `_zfs_dataset` from `Completion/Unix/Type/_zfs_dataset`.
//!
//! Full upstream body (104 lines, abridged):
//! ```text
//! sh:  1  #autoload
//! sh: 15  zparseopts -D -E e:=expl_type_arr p=paths_allowed r1=rsrc r2=rdst t+:=type
//! sh: 17  suf=( -r '\n\t\- @\#' )
//! sh: 18-25  build $typearg from -t types (fs/vol/snap/share/bookmark) + $implementation
//! sh: 27-30  if paths_allowed && PREFIX == /* → _path_files; return
//! sh: 32-42  rename-source (-r1): restrict $typearg by the -r/-p words / $implementation
//! sh: 44-58  rename-dest (-r2): @-snapshot → _message; else parent filesystem
//! sh: 60-66  clone list / plain `zfs list -H -o name $typearg`
//! sh: 68-82  openzfs snapshot-range (%,) completion via compset + compadd
//! sh: 84-92  $expl_type from typearg; mtpt adds mountpoints
//! sh: 94-96  -e override of $expl_type
//! sh: 98  _description datasets expl "$expl_type"
//! sh: 99  _multi_parts $suf "$@" "$expl[@]" -q / datasetlist
//! ```
//!
//! `$words`/`$CURRENT`/`$implementation`/`$opt_args` are read from the
//! completion params; the `${(f)"$(zfs list …)"}:#no … available` filters
//! (sh:60-66) drop the tool's "no datasets available" sentinel lines.

use crate::compsys::ported::_description::_description;
use crate::compsys::ported::_files::_files;
use crate::compsys::ported::_message::_message;
use crate::compsys::ported::_multi_parts::_multi_parts;
use crate::ported::params::{getaparam, getsparam, setaparam};
use crate::ported::zle::complete::{bin_compadd, bin_compset};
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:15 zparseopts result for `_zfs_dataset`'s specific spec.
struct Opts {
    expl_type: Option<String>, // -e VALUE
    paths_allowed: bool,       // -p
    rsrc: bool,                // -r1
    rdst: bool,                // -r2
    type_vals: Vec<String>,    // -t VALUE (repeatable)
    rest: Vec<String>,         // remaining positional ($@)
}

/// sh:15 — `zparseopts -D -E e:=… p=… r1=… r2=… t+:=…`. `-E` extracts
/// recognized options from anywhere; the rest stay positional.
fn parse_opts(args: &[String]) -> Opts {
    let mut o = Opts {
        expl_type: None,
        paths_allowed: false,
        rsrc: false,
        rdst: false,
        type_vals: Vec::new(),
        rest: Vec::new(),
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-e" => {
                if i + 1 < args.len() {
                    o.expl_type = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "-t" => {
                if i + 1 < args.len() {
                    o.type_vals.push(args[i + 1].clone());
                    i += 2;
                } else {
                    i += 1;
                }
            }
            "-p" => {
                o.paths_allowed = true;
                i += 1;
            }
            "-r1" => {
                o.rsrc = true;
                i += 1;
            }
            "-r2" => {
                o.rdst = true;
                i += 1;
            }
            other => {
                o.rest.push(other.to_string());
                i += 1;
            }
        }
    }
    o
}

/// Run `zfs list -H -o <cols> [typeargs] 2>/dev/null`, dropping the
/// "no … available" sentinel lines (sh:60-66).
fn zfs_list(cols: &str, typeargs: &[String]) -> Vec<String> {
    let mut cmd = std::process::Command::new("zfs");
    cmd.args(["list", "-H", "-o", cols]);
    cmd.args(typeargs);
    let out = cmd.output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with("no ") && !l.contains(" available"))
            .map(String::from)
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn has(vals: &[String], needle: &str) -> bool {
    vals.iter().any(|v| v == needle)
}

/// `_zfs_dataset` — complete ZFS dataset names (filesystems, volumes,
/// snapshots, …).
pub fn _zfs_dataset(args: &[String]) -> i32 {
    let opts = parse_opts(args);
    let implementation = getsparam("implementation").unwrap_or_default();
    let prefix = getsparam("PREFIX").unwrap_or_default();

    // sh:17
    let mut suf: Vec<String> = vec!["-r".to_string(), "\n\t\\- @\\#".to_string()];

    // sh:18-25 — assemble $typearg from the requested -t kinds.
    let mut kinds: Vec<String> = Vec::new();
    if has(&opts.type_vals, "fs") {
        kinds.push("filesystem".to_string());
    }
    if has(&opts.type_vals, "vol") {
        kinds.push("volume".to_string());
    }
    if has(&opts.type_vals, "snap") || prefix.contains('@') {
        kinds.push("snapshot".to_string());
    }
    if has(&opts.type_vals, "share") && implementation == "solaris" {
        kinds.push("share".to_string());
    }
    if has(&opts.type_vals, "bookmark") && implementation == "openzfs" {
        kinds.push("bookmark".to_string());
    }
    let mut typearg: Vec<String> = if !kinds.is_empty() {
        vec!["-t".to_string(), kinds.join(",")]
    } else if !opts.type_vals.is_empty() && opts.paths_allowed {
        // sh:23-24 — zfs list with the raw -t arg (paths_allowed ⇒ zfs list).
        vec!["-t".to_string(), opts.type_vals.join(",")]
    } else {
        Vec::new()
    };

    // sh:27-30 — with -p and an absolute PREFIX, complete filesystem paths.
    if opts.paths_allowed && prefix.starts_with('/') {
        return _files(&[]);
    }

    let words = getaparam("words").unwrap_or_default();
    let current: usize = getsparam("CURRENT")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    // $words[CURRENT-1] (1-based CURRENT).
    let prev_word = if current >= 2 {
        words.get(current - 2).cloned().unwrap_or_default()
    } else {
        String::new()
    };

    // sh:32-42 — rename source.
    if opts.rsrc {
        typearg = if has(&words, "-r") {
            vec!["-t".to_string(), "snapshot".to_string()]
        } else if has(&words, "-p") {
            vec!["-t".to_string(), "filesystem,volume".to_string()]
        } else if implementation == "openzfs" {
            vec!["-t".to_string(), "filesystem,snapshot,volume".to_string()]
        } else {
            vec![
                "-t".to_string(),
                "filesystem,share,snapshot,volume".to_string(),
            ]
        };
    }

    // sh:44-58 — rename destination.
    let mut expl_type_override = opts.expl_type.clone();
    if opts.rdst {
        if prev_word.contains('@') {
            // sh:48
            return _message(&[
                "-e".to_string(),
                "snapshot name (beginning with \"@\")".to_string(),
            ]);
        } else {
            // sh:54-56
            let parent = prev_word.split('/').next().unwrap_or("").to_string();
            typearg = vec![
                "-t".to_string(),
                "filesystem".to_string(),
                "-r".to_string(),
                parent,
            ];
            expl_type_override = Some("parent dataset".to_string());
        }
    }

    // sh:60-66 — build the dataset list.
    let mut datasetlist: Vec<String> = if has(&opts.type_vals, "clone") {
        // sh:61 — filesystems that have an origin (clones).
        let out = std::process::Command::new("zfs")
            .args(["list", "-H", "-o", "name,origin", "-t", "filesystem"])
            .output();
        match out {
            Ok(o) => String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|l| {
                    let mut it = l.split('\t');
                    let name = it.next()?;
                    let origin = it.next().unwrap_or("-");
                    if origin != "-" {
                        Some(name.to_string())
                    } else {
                        None
                    }
                })
                .collect(),
            Err(_) => Vec::new(),
        }
    } else {
        // sh:64-65 — range mode adds a creation sort + different suffix.
        if has(&opts.type_vals, "range") && implementation == "openzfs" {
            typearg.push("-s".to_string());
            typearg.push("creation".to_string());
            suf = vec!["-S%".to_string(), "-r".to_string(), "\n\t\\- ,".to_string()];
        }
        zfs_list("name", &typearg)
    };

    // sh:68-82 — openzfs snapshot-range (%,) completion.
    if has(&opts.type_vals, "range")
        && (prefix.contains('%') || prefix.contains(','))
        && implementation == "openzfs"
    {
        let base = prefix.split('@').next().unwrap_or("").to_string();
        if datasetlist
            .iter()
            .any(|d| d.starts_with(&format!("{}@", base)))
        {
            // Keep only this dataset's snapshots, stripped to the snapshot name.
            datasetlist = datasetlist
                .iter()
                .filter(|d| d.starts_with(&format!("{}@", base)))
                .map(|d| d.splitn(2, '@').nth(1).unwrap_or("").to_string())
                .collect();
            let _ = bin_compset(
                "compset",
                &["-P".to_string(), "*[@,]".to_string()],
                &make_ops(),
                0,
            );
            let mut expl_pfx = String::new();
            if bin_compset(
                "compset",
                &["-P".to_string(), "*%".to_string()],
                &make_ops(),
                0,
            ) == 0
            {
                suf = vec!["-qS,".to_string()];
                expl_pfx = "end ".to_string();
            }
            let _ = _description(&[
                "snapshots".to_string(),
                "expl".to_string(),
                format!("{}snapshot", expl_pfx),
            ]);
            setaparam("datasetlist", datasetlist);
            let expl = getaparam("expl").unwrap_or_default();
            let mut cadd = suf.clone();
            cadd.extend(expl);
            cadd.push("-a".to_string());
            cadd.push("datasetlist".to_string());
            return bin_compadd("compadd", &cadd, &make_ops(), 0);
        } else {
            return _message(&[
                "-e".to_string(),
                "snapshots".to_string(),
                "snapshot".to_string(),
            ]);
        }
    }

    // sh:84-92 — description type; mtpt adds mountpoints.
    let mut expl_type = if typearg.len() >= 2 {
        typearg[1].replace(',', "/")
    } else {
        String::new()
    };
    if has(&opts.type_vals, "mtpt") {
        let mlist = zfs_list("mountpoint", &typearg);
        datasetlist.extend(mlist);
        expl_type = format!("{}/mountpoint", expl_type);
    }
    // sh:94-96 — explicit -e override.
    if let Some(e) = expl_type_override {
        expl_type = e;
    }

    // sh:98
    let _ = _description(&["datasets".to_string(), "expl".to_string(), expl_type]);
    // sh:99  _multi_parts $suf "$@" "$expl[@]" -q / datasetlist
    setaparam("datasetlist", datasetlist);
    let expl = getaparam("expl").unwrap_or_default();
    let mut mp: Vec<String> = suf;
    mp.extend(opts.rest);
    mp.extend(expl);
    mp.push("-q".to_string());
    mp.push("/".to_string());
    mp.push("datasetlist".to_string());
    _multi_parts(&mp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_opts_extracts_flags_and_types() {
        let o = parse_opts(&[
            "-p".to_string(),
            "-t".to_string(),
            "fs".to_string(),
            "-t".to_string(),
            "snap".to_string(),
            "positional".to_string(),
        ]);
        assert!(o.paths_allowed);
        assert_eq!(o.type_vals, vec!["fs".to_string(), "snap".to_string()]);
        assert_eq!(o.rest, vec!["positional".to_string()]);
    }

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::setsparam("PREFIX", "");
        let _ = crate::ported::params::setsparam("implementation", "");
        assert_eq!(_zfs_dataset(&[]), 1);
    }
}
