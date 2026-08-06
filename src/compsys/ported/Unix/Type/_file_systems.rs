//! Port of `_file_systems` from `Completion/Unix/Type/_file_systems`.
//!
//! Full upstream body (38 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local expl fss
//! sh: 5  case $OSTYPE in
//! sh:     aix|irix|osf|solaris|dragonfly — fixed lists
//! sh:     linux*)  fixed list + /proc/filesystems (drop "nodev")
//! sh:                          + /etc/filesystems (drop "#*", strip "*")
//! sh:     freebsd*) lsvfs[3,-1]%% *  ||  fixed list
//! sh:     darwin*) autofs + /sbin/mount_*(#qN-*:s./sbin/mount_.)
//! sh:     *)       ufs
//! sh: 40 _wanted fstypes expl 'file system type' \
//! sh:        compadd "$@" -M 'L:|no=' -a "$@" - fss
//! ```

use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::{getsparam, setaparam};

fn fixed(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// sh (darwin) — `/sbin/mount_*` executables with the `/sbin/mount_`
/// prefix stripped.
fn darwin_mount_helpers() -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/sbin") {
        for e in rd.flatten() {
            if let Some(name) = e.file_name().to_str() {
                if let Some(fs) = name.strip_prefix("mount_") {
                    if !fs.is_empty() {
                        out.push(fs.to_string());
                    }
                }
            }
        }
    }
    out.sort();
    out
}

/// `_file_systems` — complete file-system type names for the running OS.
pub fn _file_systems(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_file_systems");
    // sh:5 — dispatch on $OSTYPE (runtime, like the shell case).
    let ostype = getsparam("OSTYPE").unwrap_or_default();
    let fss: Vec<String> = if ostype.starts_with("aix") {
        fixed(&["jfs", "nfs", "cdrfs"])
    } else if ostype.starts_with("irix") {
        fixed(&[
            "efs", "proc", "fd", "nfs", "iso9660", "dos", "hfs", "cachefs", "xfs",
        ])
    } else if ostype.starts_with("osf") {
        fixed(&["advfs", "ufs", "nfs", "mfs", "cdfs"])
    } else if ostype.starts_with("solaris") {
        fixed(&["ufs", "nfs", "hsfs", "s5fs", "pcfs", "cachefs", "tmpfs"])
    } else if ostype.starts_with("dragonfly") {
        fixed(&[
            "cd9660",
            "devfs",
            "ext2fs",
            "fdesc",
            "kernfs",
            "linprocfs",
            "mfs",
            "msdos",
            "nfs",
            "ntfs",
            "null",
            "nwfs",
            "portal",
            "procfs",
            "std",
            "udf",
            "ufs",
            "umap",
            "union",
        ])
    } else if ostype.starts_with("freebsd") {
        fixed(&[
            "cd9660",
            "devfs",
            "ext2fs",
            "fdescfs",
            "kernfs",
            "linprocfs",
            "linsysfs",
            "mfs",
            "msdosfs",
            "nfs",
            "ntfs",
            "nullfs",
            "nwfs",
            "portalfs",
            "procfs",
            "smbfs",
            "std",
            "tmpfs",
            "udf",
            "ufs",
            "unionfs",
            "reiserfs",
            "xfs",
            "zfs",
        ])
    } else if ostype.starts_with("darwin") {
        let mut v = vec!["autofs".to_string()];
        v.extend(darwin_mount_helpers());
        v
    } else if ostype.starts_with("linux") {
        let mut v = fixed(&[
            "adfs", "bfs", "cramfs", "ext2", "ext3", "hfs", "hpfs", "iso9660", "minix", "ntfs",
            "qnx4", "reiserfs", "romfs", "swap", "udf", "ufs", "vxfs", "xfs", "xiafs",
        ]);
        if let Ok(pf) = std::fs::read_to_string("/proc/filesystems") {
            for line in pf.lines() {
                // ${...#nodev} — drop a leading "nodev" tab-field.
                let fs = line.trim().trim_start_matches("nodev").trim();
                if !fs.is_empty() {
                    v.push(fs.to_string());
                }
            }
        }
        if let Ok(ef) = std::fs::read_to_string("/etc/filesystems") {
            for line in ef.lines() {
                // :#\#* — drop comments; #\* — strip a leading `*`.
                if line.starts_with('#') {
                    continue;
                }
                let fs = line.strip_prefix('*').unwrap_or(line).trim();
                if !fs.is_empty() {
                    v.push(fs.to_string());
                }
            }
        }
        // typeset -aU — unique, keeping first-seen order.
        let mut seen = std::collections::HashSet::new();
        v.retain(|s| seen.insert(s.clone()));
        v
    } else {
        // sh — default for all other systems.
        fixed(&["ufs"])
    };

    // sh:40
    setaparam("fss", fss);
    let mut w: Vec<String> = vec![
        "fstypes".to_string(),
        "expl".to_string(),
        "file system type".to_string(),
        "compadd".to_string(),
    ];
    w.extend(args.iter().cloned());
    w.push("-M".to_string());
    w.push("L:|no=".to_string());
    w.push("-a".to_string());
    w.extend(args.iter().cloned());
    w.push("-".to_string());
    w.push("fss".to_string());
    _wanted(&w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(1, std::sync::atomic::Ordering::Relaxed);
        let r = _file_systems(&[]);
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(r, 1);
    }
}
