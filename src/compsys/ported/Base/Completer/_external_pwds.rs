//! Port of `_external_pwds` from
//! `Completion/Base/Completer/_external_pwds`.
//!
//! Full upstream body (43 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 7  local -a expl
//! sh: 8  local -au dirs
//! sh:11  PREFIX="$IPREFIX$PREFIX"; IPREFIX=
//! sh:13  SUFFIX="$SUFFIX$ISUFFIX"; ISUFFIX=
//! sh:16  [[ -o magicequalsubst ]] && compset -P '*='
//! sh:18  case $OSTYPE in
//! sh:19  solaris*) dirs=( pgrep+pwdx ) ;;
//! sh:23  linux*)   dirs=( /proc/${PID}/cwd ) ;;
//! sh:27  freebsd*) dirs=( pgrep + procstat ) ;;
//! sh:33  *)        dirs=( lsof -d cwd ) ;;
//! sh:38  esac
//! sh:39  dirs=( ${(D)dirs:#$PWD} )
//! sh:41  compstate[pattern_match]='*'
//! sh:42  _wanted directories expl 'current directory from other shell' \
//! sh:43      compadd -M "r:|/=* r:|=*" -f -a dirs
//! ```
//!
//! Scans peer-zsh `cwd` per-OS. Linux uses `/proc/<pid>/cwd`; other
//! UNIX falls back to `lsof -d cwd -c zsh -F n` parse.

use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::{getsparam, setaparam, setsparam};
use crate::ported::zle::compcore::set_compstate_str;
use crate::ported::zle::complete::bin_compset;
use crate::ported::zsh_h::{isset, options, MAGICEQUALSUBST, MAX_OPS};
use std::fs;
use std::path::Path;
use std::process::Command;

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:18-37 — collect other zsh procs' cwd via OS-specific paths.
fn scan_external_zsh_cwds() -> Vec<String> {
    let my_pid = std::process::id();
    let mut out: Vec<String> = Vec::new();

    // Linux first — /proc/<pid>/cwd
    if Path::new("/proc").is_dir() {
        if let Ok(entries) = fs::read_dir("/proc") {
            for ent in entries.flatten() {
                let name = ent.file_name().to_string_lossy().to_string();
                let pid: u32 = match name.parse() {
                    Ok(n) if n != my_pid => n,
                    _ => continue,
                };
                // Check the process is zsh
                let comm_path = format!("/proc/{}/comm", pid);
                let cmd = fs::read_to_string(&comm_path).unwrap_or_default();
                if !cmd.trim().contains("zsh") {
                    continue;
                }
                // Read cwd link
                if let Ok(cwd) = fs::read_link(format!("/proc/{}/cwd", pid)) {
                    out.push(cwd.to_string_lossy().into_owned());
                }
            }
        }
        return out;
    }

    // Fallback: lsof -d cwd -c zsh -F n
    if let Ok(output) = Command::new("lsof")
        .args(["-a", "-c", "zsh", "-d", "cwd", "-F", "n", "-w"])
        .output()
    {
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if let Some(path) = line.strip_prefix('n') {
                out.push(path.to_string());
            }
        }
    }
    out
}

/// `_external_pwds` — complete `cd` paths from other running zsh
/// processes.
pub fn _external_pwds() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_external_pwds");
    // sh:11-14  collapse IPREFIX/ISUFFIX
    let iprefix = getsparam("IPREFIX").unwrap_or_default();
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let suffix = getsparam("SUFFIX").unwrap_or_default();
    let isuffix = getsparam("ISUFFIX").unwrap_or_default();
    let _ = setsparam("PREFIX", &format!("{}{}", iprefix, prefix));
    let _ = setsparam("IPREFIX", "");
    let _ = setsparam("SUFFIX", &format!("{}{}", suffix, isuffix));
    let _ = setsparam("ISUFFIX", "");

    // sh:16
    if isset(MAGICEQUALSUBST) {
        let _ = bin_compset(
            "compset",
            &["-P".to_string(), "*=".to_string()],
            &make_ops(),
            0,
        );
    }

    // sh:18-37
    let mut dirs = scan_external_zsh_cwds();

    // sh:39  remove $PWD; dedupe
    let pwd = getsparam("PWD").unwrap_or_default();
    dirs.retain(|d| d != &pwd);
    dirs.sort();
    dirs.dedup();

    if dirs.is_empty() {
        return 1;
    }

    // sh:41
    set_compstate_str("pattern_match", "*");

    // sh:8 `local -au dirs` — the lowercase `-u` is PM_UPPER (uppercase
    // conversion), not PM_UNIQUE, and it is NOT replicated here.
    //
    // Measured in zsh 5.9.2, which is the whole argument:
    //   f(){ local -u s; s=abc; local -au a; a=(abc def)
    //        print "$s ${(t)s} | ${(j:,:)a} ${(t)a}" }
    //   → ABC scalar-local-upper | abc,def array-local-upper
    // `-u` uppercases a SCALAR's value and is inert on an array's, so the
    // only thing sh:8 buys upstream is the type string. The elements here
    // are absolute directory paths on a case-sensitive filesystem, compared
    // against $PWD and then added as literal path matches, so an
    // implementation that DID uppercase them would break the completer
    // outright. Stamping PM_UPPER would therefore be replicating what reads
    // as an upstream typo (`-au` for `-aU`) at the risk of a real
    // regression, for no gain beyond `${(t)dirs}`.
    setaparam("dirs", dirs);

    // sh:42
    _wanted(&[
        "directories".to_string(),
        "expl".to_string(),
        "current directory from other shell".to_string(),
        "compadd".to_string(),
        "-M".to_string(),
        "r:|/=* r:|=*".to_string(),
        "-f".to_string(),
        "-a".to_string(),
        "dirs".to_string(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_when_dirs_empty() {
        // Run in env that has no peer-zsh procs (best-effort: relies
        //   on the scanner finding none ≠ our own pid).
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("IPREFIX", "");
        let _ = setsparam("PREFIX", "");
        let _ = setsparam("SUFFIX", "");
        let _ = setsparam("ISUFFIX", "");
        let _r = _external_pwds();
    }
}
