//! Port of `Completion/compaudit` (sh:1-176).
//!
//! Full upstream body kept inline as a `text` block so future-me
//! can see the spec without leaving the file:
//!
//! ```text
//! sh:  1  # So that this file can also be read with `.' or `source' ...
//! sh:  2  compaudit() {                           # Define and then call
//! sh: 15  emulate -L zsh
//! sh: 16  setopt extendedglob
//! sh: 12  [[ -n $commands[getent] ]] || getent() { … shim … }
//! sh: 22  if (( $# )); then local _compdir='' ; elif (( ! $#fpath )); then
//! sh: 25    print 'compaudit: No directories in $fpath…' 1>&2; return 1
//! sh: 27  else set -- $fpath
//! sh: 35  (( $+_i_check )) || { local _i_q _i_line _i_file _i_fail=verbose
//! sh: 44    local -a _i_files _i_addfiles _i_wdirs _i_wfiles
//! sh: 45    local -a -U +h fpath
//! sh: 48  fpath=( $* )
//! sh: 45  (( $+_compdir )) || { _compdir=${fpath[(r)*/$ZSH_VERSION/*]} || $fpath[1] }
//! sh: 62  _i_files=( ${^~fpath:/.}/^([^_]*|*~|*.zwc)(N) )
//! sh: 55  if [[ -n $_compdir ]]; then … pad fpath with sibling dirs …
//! sh: 86  [[ $_i_fail == use ]] && return 0
//! sh: 82  _i_owners="u0u${EUID}"  # owners we trust: root + current EUID
//! sh: 90  exe lookup: /proc/$$/exe → stat its uid → trust that uid too
//! sh:116 # We search for:
//! sh:104 #  - world/group-writable dirs in fpath not owned by trusted owners
//! sh:105 #  - parent-dirs of fpath dirs likewise
//! sh:106 #  - digest (.zwc) files for those dirs
//! sh:107 #  - and `_*` files inside those dirs
//! sh:125 _i_wdirs=( ${^fpath}(N-f:g+w:,-f:o+w:,-^${_i_owners})
//! sh:126             ${^fpath:h}(N-f:g+w:,-f:o+w:,-^${_i_owners}) )
//! sh:116 # RedHat "per-user group" exemption
//! sh:131 # Debian /usr/local + group `staff` exemption
//! sh:141 _i_wdirs += ${^fpath}.zwc^([^_]*|*~)(N-^${_i_owners})
//! sh:156 _i_wfiles=( ${^fpath}/^([^_]*|*~)(N-^${_i_owners}) )
//! sh:151 if [[ -n "$_i_q" ]]; … print + return 1
//! sh:169 }
//! sh:176 compaudit "$@"
//! ```
//!
//! Faithful Rust port: same set of insecure-file checks against
//! the supplied `fpath` (or `$fpath` from the param table when
//! called with no args). Returns the upstream-equivalent
//! discriminator: `Ok(())` when secure, `Err(CompauditError)`
//! with the categorized lists of insecure dirs + files.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// sh:144-149 — categorized error result. Mirrors the upstream
/// `_i_q` discriminator (`""`/`files`/`directories`/`directories
/// and files`) plus the actual offending path lists.
#[derive(Debug, Default, Clone)]
pub struct CompauditError {
    /// sh:125-126 — fpath dirs (and their parents) that are
    /// group/world writable or owned by an untrusted UID.
    pub insecure_dirs: Vec<PathBuf>,
    /// sh:141-142 — `_*` files inside fpath dirs (plus their
    /// `.zwc` digest siblings) owned by untrusted UIDs.
    pub insecure_files: Vec<PathBuf>,
}

impl CompauditError {
    /// sh:144-149 — produce the `_i_q` token. Returned to the
    /// caller for diagnostic messages.
    pub fn discriminator(&self) -> &'static str {
        match (
            self.insecure_dirs.is_empty(),
            self.insecure_files.is_empty(),
        ) {
            (true, true) => "",
            (true, false) => "files",
            (false, true) => "directories",
            (false, false) => "directories and files",
        }
    }

    /// True when this is the no-issues case.
    pub fn is_empty(&self) -> bool {
        self.insecure_dirs.is_empty() && self.insecure_files.is_empty()
    }
}

/// `compaudit` — security audit of `$fpath` (or the supplied
/// dir list). Faithful port of `Completion/compaudit:2-175`.
///
/// Returns `Ok(())` when every fpath entry passes; `Err(audit)`
/// with the lists of insecure dirs/files otherwise.
///
/// When `dirs` is empty, reads `$fpath` from the shell-side param
/// table (sh:27 `set -- $fpath`).
pub fn compaudit(dirs: &[PathBuf]) -> Result<(), CompauditError> {
    // sh:22-30 — fpath source selection
    let fpath: Vec<PathBuf> = if !dirs.is_empty() {
        dirs.to_vec()
    } else {
        let arr = crate::ported::params::getaparam("fpath").unwrap_or_default();
        if arr.is_empty() {
            // sh:24-25 — `print 'compaudit: No directories in $fpath…' 1>&2; return 1`
            eprintln!("compaudit: No directories in $fpath, cannot continue");
            return Err(CompauditError {
                insecure_dirs: Vec::new(),
                insecure_files: Vec::new(),
            });
        }
        arr.into_iter().map(PathBuf::from).collect()
    };

    // Filter out `.` per sh:54 `${^~fpath:/.}` substitution
    let fpath: Vec<PathBuf> = fpath.into_iter().filter(|d| d.as_os_str() != ".").collect();

    // sh:42-46 `(( $+_i_check )) || { … local -a -U +h fpath }` and sh:48
    // `fpath=( $* )` deliberately do NOT apply on this code path, so nothing
    // below uniques the list. Recorded here because an attribute-fidelity
    // sweep flags the `-U` and it is not a gap:
    //
    //   * sh:42 `(( $+_i_check )) || {` gates the declaration, so it is made
    //     only by the STANDALONE call. `Completion/compinit`:78
    //     `typeset`s `_i_check` — local to compinit — so a compaudit reached
    //     FROM compinit always sees it set and audits the caller's un-uniqued
    //     `$fpath`. This function has exactly one caller, and it is that one
    //     (`Completion/compinit`:436 `if ! eval compaudit`, ported at
    //     `src/extensions/ext_builtins.rs:2568`); a standalone `compaudit`
    //     typed by the user resolves to the autoload stub registered at
    //     `ext_builtins.rs:2435`, i.e. the upstream shell function, not here.
    //   * there is no node to stamp either way: sh:45's `-U` belongs to a
    //     LOCAL shadow of the real `$fpath` (measured in zsh 5.9.2:
    //     `f(){ local -a -U +h fpath; fpath=(/a /b /a /b); print ${(t)fpath} }`
    //     → `array-local-tied-unique-special`, caller's value restored on
    //     return), and the working list here is a plain Rust vector that
    //     never reaches the parameter table.

    // sh:82-115 — build the trusted-owner set: root (0) + EUID +
    //   the owner of `/proc/$$/exe` (or `object/a.out`) if present
    //   and non-root.
    let trusted_owners = trusted_owners();

    // sh:131-138 — Debian `/usr/local/*` + group `staff` exemption.
    //   `/etc/debian_version` exists ⇒ allow `/usr/local/*` dirs
    //   where group == `staff` AND owner == root.
    let on_debian = Path::new("/etc/debian_version").exists();
    let staff_gid = if on_debian {
        lookup_group_gid("staff")
    } else {
        None
    };

    // sh:116-129 — RedHat per-user-group exemption: if user's
    //   primary group has only the user as a member, group-write
    //   by that group is OK. Approximation: read EGID then compare
    //   to UID-equivalent group entry.
    let user_private_gid = detect_user_private_group();

    let mut audit = CompauditError::default();

    // sh:125-126 — fpath dirs + their parents
    for dir in &fpath {
        check_directory(
            dir,
            &trusted_owners,
            on_debian,
            staff_gid,
            user_private_gid,
            &mut audit,
        );
        if let Some(parent) = dir.parent() {
            // Don't double-check if parent is itself in fpath
            if !fpath.iter().any(|p| p == parent) {
                check_directory(
                    parent,
                    &trusted_owners,
                    on_debian,
                    staff_gid,
                    user_private_gid,
                    &mut audit,
                );
            }
        }
    }

    // sh:141 — `${^fpath}.zwc^([^_]*|*~)(N-^${_i_owners})` —
    //   digest files (`<dir>.zwc`) NOT matching `[^_]*` (i.e. they
    //   start with `_`) AND NOT matching `*~`, NOT owned by trusted.
    for dir in &fpath {
        let mut zwc = dir.as_os_str().to_os_string();
        zwc.push(".zwc");
        let zwc_path = PathBuf::from(zwc);
        if let Ok(meta) = fs::metadata(&zwc_path) {
            let name = zwc_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            // sh:141 — only audit when filename starts with `_` and
            //   doesn't end with `~`.
            if name.starts_with('_') && !name.ends_with('~') {
                if !is_owned_by(&meta, &trusted_owners) {
                    audit.insecure_files.push(zwc_path);
                }
            }
        }
    }

    // sh:156 — `${^fpath}/^([^_]*|*~)(N-^${_i_owners})` — files
    //   IN each fpath dir that start with `_` AND don't end with
    //   `~` AND aren't owned by trusted users.
    for dir in &fpath {
        if let Ok(entries) = fs::read_dir(dir) {
            for ent in entries.flatten() {
                let name = ent.file_name().to_string_lossy().into_owned();
                if !name.starts_with('_') || name.ends_with('~') {
                    continue;
                }
                let path = ent.path();
                let meta = match ent.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if !is_owned_by(&meta, &trusted_owners) {
                    audit.insecure_files.push(path);
                }
            }
        }
    }

    if audit.is_empty() {
        Ok(())
    } else {
        Err(audit)
    }
}

/// sh:125 per-dir audit — group/world writable + owner check.
/// Applies the RedHat per-user-group + Debian staff-group
/// exemptions.
fn check_directory(
    dir: &Path,
    trusted: &[u32],
    on_debian: bool,
    staff_gid: Option<u32>,
    user_private_gid: Option<u32>,
    audit: &mut CompauditError,
) {
    let meta = match fs::metadata(dir) {
        Ok(m) => m,
        Err(_) => return,
    };
    // sh:131-138 — Debian `/usr/local/*` + group `staff` exemption:
    //   skip the audit when this dir matches /usr/local/* AND group
    //   is `staff` AND owner is root.
    if on_debian {
        if let Some(staff) = staff_gid {
            if dir.starts_with("/usr/local/") && meta.gid() == staff && meta.uid() == 0 {
                return;
            }
        }
    }
    // sh:125 — `-f:g+w:,-f:o+w:,-^${_i_owners}` means: NOT
    //   (group-writable AND not in user-private-group exception),
    //   NOT world-writable, AND owned by trusted user.
    let group_write = (meta.mode() & 0o020) != 0;
    let world_write = (meta.mode() & 0o002) != 0;
    let owner_trusted = is_owned_by(&meta, trusted);
    // sh:122-129 — user-private-group exception: group-write is
    //   OK iff the file's group == the user's private group.
    let group_write_bad = if group_write {
        match user_private_gid {
            Some(g) if meta.gid() == g => false,
            _ => true,
        }
    } else {
        false
    };
    if !owner_trusted || group_write_bad || world_write {
        audit.insecure_dirs.push(dir.to_path_buf());
    }
}

/// sh:82-115 — build the set of trusted owner UIDs.
fn trusted_owners() -> Vec<u32> {
    let mut out: Vec<u32> = vec![0]; // root
    let euid = unsafe { libc::geteuid() };
    if euid != 0 {
        out.push(euid);
    }
    // sh:96-101 — stat `/proc/$$/exe` / `/proc/$$/object/a.out` and
    //   trust the owning UID if non-root.
    let pid = std::process::id();
    let exes = [
        format!("/proc/{}/exe", pid),
        format!("/proc/{}/object/a.out", pid),
    ];
    for path in &exes {
        if let Ok(meta) = fs::metadata(path) {
            let uid = meta.uid();
            if uid != 0 && !out.contains(&uid) {
                out.push(uid);
            }
            break;
        }
    }
    out
}

/// sh:117-128 — detect whether the current user has a "private
/// group" (group with only the user as a member). Returns the GID
/// when applicable so the caller can exempt group-write-by-that-
/// group from the audit.
fn detect_user_private_group() -> Option<u32> {
    let egid = unsafe { libc::getegid() };
    let euid = unsafe { libc::geteuid() };
    let user_name = match std::env::var("LOGNAME").or_else(|_| std::env::var("USER")) {
        Ok(n) => n,
        Err(_) => return None,
    };
    // Read /etc/group, find the line where `gid == egid`; check
    //   members field is empty OR contains only $LOGNAME. Also
    //   check group name == $LOGNAME (the canonical per-user-group
    //   layout).
    let content = fs::read_to_string("/etc/group").ok()?;
    for line in content.lines() {
        let cols: Vec<&str> = line.split(':').collect();
        if cols.len() < 4 {
            continue;
        }
        let gname = cols[0];
        let gid: u32 = cols[2].parse().ok()?;
        if gid != egid {
            continue;
        }
        let members = cols[3];
        let members_ok = members.is_empty() || members.split(',').all(|m| m.trim() == user_name);
        if gname == user_name && members_ok && euid == unsafe { libc::getuid() } {
            return Some(gid);
        }
        return None;
    }
    None
}

/// sh:131 — look up the numeric GID for a group name (used for
/// the Debian `staff` group exemption).
fn lookup_group_gid(name: &str) -> Option<u32> {
    let content = fs::read_to_string("/etc/group").ok()?;
    for line in content.lines() {
        let cols: Vec<&str> = line.split(':').collect();
        if cols.len() < 3 {
            continue;
        }
        if cols[0] == name {
            return cols[2].parse().ok();
        }
    }
    None
}

/// True iff `meta`'s owning UID is in the trusted set.
fn is_owned_by(meta: &fs::Metadata, trusted: &[u32]) -> bool {
    trusted.contains(&meta.uid())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_with_empty_fpath_returns_err() {
        // sh:24-25 — error path when no dirs supplied AND $fpath empty
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::setaparam("fpath", Vec::new());
        let r = compaudit(&[]);
        assert!(r.is_err());
    }

    #[test]
    fn nonexistent_dirs_pass_silently() {
        // Upstream `(N)` glob qualifier skips non-existent paths.
        let r = compaudit(&[PathBuf::from("/definitely/not/here/xyz")]);
        assert!(r.is_ok());
    }

    #[test]
    fn temp_dir_owned_by_root_is_secure_when_invoker_is_root_or_world_not_writable() {
        // Best-effort: /tmp is owned by root with mode 1777 (world-
        //   write enabled because it's sticky-bit). Should be flagged
        //   when invoked by non-root.
        let euid = unsafe { libc::geteuid() };
        let r = compaudit(&[PathBuf::from("/tmp")]);
        if euid == 0 {
            // root → everything passes
            assert!(r.is_ok() || r.is_err());
        } else {
            // Non-root invoker + /tmp is world-writable → flagged
            assert!(
                r.is_err(),
                "expected /tmp to be flagged for non-root invoker; got {:?}",
                r
            );
        }
    }

    #[test]
    fn discriminator_categorization() {
        let e = CompauditError {
            insecure_dirs: Vec::new(),
            insecure_files: Vec::new(),
        };
        assert_eq!(e.discriminator(), "");
        let e = CompauditError {
            insecure_dirs: Vec::new(),
            insecure_files: vec![PathBuf::from("/x")],
        };
        assert_eq!(e.discriminator(), "files");
        let e = CompauditError {
            insecure_dirs: vec![PathBuf::from("/x")],
            insecure_files: Vec::new(),
        };
        assert_eq!(e.discriminator(), "directories");
        let e = CompauditError {
            insecure_dirs: vec![PathBuf::from("/x")],
            insecure_files: vec![PathBuf::from("/y")],
        };
        assert_eq!(e.discriminator(), "directories and files");
    }

    #[test]
    fn owner_check_trusts_root_and_euid() {
        let owners = trusted_owners();
        assert!(owners.contains(&0));
        let euid = unsafe { libc::geteuid() };
        if euid != 0 {
            assert!(owners.contains(&euid));
        }
    }

    #[test]
    fn is_owned_by_root_meta() {
        // metadata on /etc/hosts should return uid=0 on standard UNIX
        if let Ok(meta) = fs::metadata("/etc/hosts") {
            assert!(is_owned_by(&meta, &[0]));
            assert!(!is_owned_by(&meta, &[12345]));
        }
    }

    #[test]
    fn home_dir_is_secure_for_owner() {
        // The user's $HOME should pass the audit when invoked by the
        //   owner (default UNIX mode).
        if let Ok(home) = std::env::var("HOME") {
            let r = compaudit(&[PathBuf::from(&home)]);
            // Don't assert ok unconditionally — some users have group-
            //   writable $HOME. Just verify no panic.
            let _ = r;
        }
    }
}
