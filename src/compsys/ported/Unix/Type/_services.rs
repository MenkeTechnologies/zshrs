//! Port of `_services` from `Completion/Unix/Type/_services`.
//!
//! Full upstream body (36 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local -a inits xinetds alls
//! sh: 4  local expl ret=1
//! sh: 6  if [[ $OSTYPE = freebsd* ]]; then
//! sh: 7    if [[ -x /usr/sbin/service ]]; then
//! sh: 8      alls=( $(service -l) ) && ret=0
//! sh:10      _wanted services expl service compadd "$@" - $alls[@] && ret=0
//! sh:11    fi
//! sh:12  elif chkconfig --list > /dev/null 2>&1; then
//! sh:13    alls=( ${(f)"$(… chkconfig --list …)"} )
//! sh:14    inits=( … xinetd-based split, init side … )
//! sh:15    xinetds=( … xinetd side … )
//! sh:17    _alternative 'init:init service:compadd -a inits' \
//! sh:19                 'xinetd:xinetd service:compadd -a xinetds' && ret=0
//! sh:20  else
//! sh:25    scriptpath=(/etc/init.d /etc/rc.d /etc/rc.d/init.d)
//! sh:24    for dir in $scriptpath; do [[ -d $dir ]] && break; done
//! sh:29    _wanted services expl service compadd "$@" - $dir/*(-*:t) && ret=0
//! sh:30  fi
//! sh:36  return ret
//! ```
//!
//! sh:14-15 the nested xinetd-based split of `chkconfig --list` output is
//! done with straight line/field ops (`// sh:14 approx`).

use crate::compsys::ported::_alternative::_alternative;
use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::{getsparam, setaparam};

/// sh:29 approx — basenames of executable entries in `dir` (`*(-*:t)`).
fn exec_basenames(dir: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for ent in rd.flatten() {
            let path = ent.path();
            // `-*` = follow symlinks + executable; `fs::metadata` follows.
            if let Ok(md) = std::fs::metadata(&path) {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if md.is_file() && md.permissions().mode() & 0o111 != 0 {
                        if let Some(n) = path.file_name().and_then(|n| n.to_str()) {
                            out.push(n.to_string());
                        }
                    }
                }
            }
        }
    }
    out.sort();
    out
}

/// `_services` — complete system service (init/rc) names.
pub fn _services(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_services");
    let ostype = getsparam("OSTYPE").unwrap_or_default();

    // sh:6-11 — FreeBSD `service -l`.
    if ostype.starts_with("freebsd") {
        if std::path::Path::new("/usr/sbin/service").exists() {
            let _ = _call_program(&[
                "services".to_string(),
                "service".to_string(),
                "-l".to_string(),
            ]);
            let alls: Vec<String> = getsparam("REPLY")
                .unwrap_or_default()
                .split_whitespace()
                .map(String::from)
                .collect();
            let mut wanted_argv: Vec<String> = vec![
                "services".to_string(),
                "expl".to_string(),
                "service".to_string(),
                "compadd".to_string(),
            ];
            wanted_argv.extend(args.iter().cloned());
            wanted_argv.push("-".to_string());
            wanted_argv.extend(alls);
            return _wanted(&wanted_argv);
        }
        return 1;
    }

    // sh:12-18 — chkconfig (SysV + xinetd) split.
    let _ = _call_program(&[
        "services".to_string(),
        "chkconfig".to_string(),
        "--list".to_string(),
    ]);
    let chk = getsparam("REPLY").unwrap_or_default();
    if !chk.trim().is_empty() {
        // sh:14 approx — everything before the "xinetd based services:" line
        //   is SysV; everything after is xinetd. First field of each row.
        let lines: Vec<&str> = chk.lines().collect();
        let split = lines
            .iter()
            .position(|l| l.trim_start().starts_with("xinetd based"));
        let first_field = |l: &str| -> Option<String> {
            let f = l.split_whitespace().next()?;
            let f = f.trim_end_matches(':');
            if f.is_empty() {
                None
            } else {
                Some(f.to_string())
            }
        };
        let (inits, xinetds): (Vec<String>, Vec<String>) = match split {
            Some(idx) => (
                lines[..idx].iter().filter_map(|l| first_field(l)).collect(),
                lines[idx + 1..]
                    .iter()
                    .filter_map(|l| first_field(l))
                    .collect(),
            ),
            None => (
                lines.iter().filter_map(|l| first_field(l)).collect(),
                Vec::new(),
            ),
        };
        setaparam("inits", inits);
        setaparam("xinetds", xinetds);
        return _alternative(&[
            "init:init service:compadd -a inits".to_string(),
            "xinetd:xinetd service:compadd -a xinetds".to_string(),
        ]);
    }

    // sh:19-31 — init-script directory scan.
    let dir = ["/etc/init.d", "/etc/rc.d", "/etc/rc.d/init.d"]
        .into_iter()
        .find(|d| std::path::Path::new(d).is_dir())
        .unwrap_or("/etc/init.d");
    let names = exec_basenames(dir);
    let mut wanted_argv: Vec<String> = vec![
        "services".to_string(),
        "expl".to_string(),
        "service".to_string(),
        "compadd".to_string(),
    ];
    wanted_argv.extend(args.iter().cloned());
    wanted_argv.push("-".to_string());
    wanted_argv.extend(names);
    _wanted(&wanted_argv)
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
        let r = _services(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 1);
    }
}
