//! bash-style `declare -i` / `-r` / `-a` / `-A` listings — a Rust-only extension with NO zsh C
//! counterpart.
//!
//! zsh's name-less `typeset -i` walks its whole parameter table, which in zshrs is full of zsh
//! specials (`!`, `#`, `HISTSIZE`, `fpath`, …). bash lists only the variables a script could see: its
//! own specials plus whatever the script or the environment defined. This module prints that list for
//! the flag sets whose bash output is fully determined by the specials table below.
//!
//! Selection rules, measured against bash 5.3 (`declare -i`, `-r`, `-ir`, `-a`, `-A`, `-ar`, `-ia`):
//! `-a` / `-A` restrict the TYPE (intersection); `-i` / `-r` select ATTRIBUTES and combine as a union;
//! the result is sorted by name.

use crate::ported::params::paramtab;
use crate::ported::zsh_h::{param, PM_ARRAY, PM_HASHED, PM_INTEGER, PM_READONLY, PM_SPECIAL};
use std::collections::HashSet;
use std::sync::Mutex;

/// Parameter names that existed before any user code ran.
static BASELINE: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Record the parameter table as the startup baseline. Called once, after the drop-in's
/// parameter deltas have been applied.
pub fn snapshot_baseline() {
    if let Ok(tab) = paramtab().read() {
        let names: HashSet<String> = tab.keys().cloned().collect();
        if let Ok(mut b) = BASELINE.lock() {
            *b = Some(names);
        }
    }
}

/// One of bash's own special variables.
struct Special {
    name: &'static str,
    /// `'-'` scalar, `'a'` indexed array, `'A'` associative array.
    kind: char,
    integer: bool,
    readonly: bool,
}

const SPECIALS: &[Special] = &[
    Special { name: "BASHOPTS", kind: '-', integer: false, readonly: true },
    Special { name: "BASHPID", kind: '-', integer: true, readonly: false },
    Special { name: "BASH_ALIASES", kind: 'A', integer: false, readonly: false },
    Special { name: "BASH_ARGC", kind: 'a', integer: false, readonly: false },
    Special { name: "BASH_ARGV", kind: 'a', integer: false, readonly: false },
    Special { name: "BASH_CMDS", kind: 'A', integer: false, readonly: false },
    Special { name: "BASH_LINENO", kind: 'a', integer: false, readonly: false },
    Special { name: "BASH_SOURCE", kind: 'a', integer: false, readonly: false },
    Special { name: "BASH_VERSINFO", kind: 'a', integer: false, readonly: true },
    Special { name: "DIRSTACK", kind: 'a', integer: false, readonly: false },
    Special { name: "EUID", kind: '-', integer: true, readonly: true },
    Special { name: "FUNCNAME", kind: 'a', integer: false, readonly: false },
    Special { name: "GROUPS", kind: 'a', integer: false, readonly: false },
    Special { name: "HISTCMD", kind: '-', integer: true, readonly: false },
    Special { name: "OPTIND", kind: '-', integer: true, readonly: false },
    Special { name: "PPID", kind: '-', integer: true, readonly: true },
    Special { name: "RANDOM", kind: '-', integer: true, readonly: false },
    Special { name: "SHELLOPTS", kind: '-', integer: false, readonly: true },
    Special { name: "SRANDOM", kind: '-', integer: true, readonly: false },
    Special { name: "UID", kind: '-', integer: true, readonly: true },
];

/// The value text bash prints after `=`, or `None` when it prints the name alone (`declare -i RANDOM`).
fn special_value(name: &str) -> Option<String> {
    let quoted = |s: String| Some(format!("\"{s}\""));
    match name {
        "BASHOPTS" => quoted(crate::dash_mode::bash_shoptsopts()),
        "SHELLOPTS" => quoted(crate::dash_mode::bash_shellopts()),
        "EUID" => quoted(unsafe { libc::geteuid() }.to_string()),
        "UID" => quoted(unsafe { libc::getuid() }.to_string()),
        "PPID" => quoted(unsafe { libc::getppid() }.to_string()),
        "OPTIND" => quoted(crate::ported::params::getiparam("OPTIND").to_string()),
        "BASH_VERSINFO" => Some(format!(
            "({})",
            crate::dash_mode::bash_versinfo()
                .iter()
                .enumerate()
                .map(|(i, v)| format!("[{i}]=\"{v}\""))
                .collect::<Vec<_>>()
                .join(" ")
        )),
        "BASH_ALIASES" | "BASH_ARGC" | "BASH_ARGV" | "BASH_CMDS" | "BASH_LINENO" | "BASH_SOURCE"
        | "DIRSTACK" | "GROUPS" => Some("()".to_string()),
        _ => None,
    }
}

/// Print the listing for `types` (a subset of `aA`) and `attrs` (a subset of `ir`).
pub fn list(types: &str, attrs: &str) {
    let want_i = attrs.contains('i');
    let want_r = attrs.contains('r');
    let want_a = types.contains('a');
    let want_aa = types.contains('A');
    let type_ok = |kind: char| {
        (!want_a && !want_aa) || (want_a && kind == 'a') || (want_aa && kind == 'A')
    };
    let attr_ok = |integer: bool, readonly: bool| {
        (!want_i && !want_r) || (want_i && integer) || (want_r && readonly)
    };

    let baseline = BASELINE.lock().ok().and_then(|b| b.clone()).unwrap_or_default();
    // (name, Some(special) | None for a user parameter)
    let mut rows: Vec<(String, Option<&Special>)> = SPECIALS
        .iter()
        .filter(|s| type_ok(s.kind) && attr_ok(s.integer, s.readonly))
        .map(|s| (s.name.to_string(), Some(s)))
        .collect();
    // PIPESTATUS exists once a command has run.
    let pipestatus = crate::dash_mode::bash_special_array("PIPESTATUS").unwrap_or_default();
    let show_pipestatus = !pipestatus.is_empty() && type_ok('a') && attr_ok(false, false);
    let mut users: Vec<param> = Vec::new();
    if let Ok(tab) = paramtab().read() {
        for pm in tab.values() {
            let f = pm.node.flags as u32;
            let name = &pm.node.nam;
            if baseline.contains(name)
                || SPECIALS.iter().any(|s| s.name == name)
                || name == "pipestatus"
                || (f & PM_SPECIAL) != 0
            {
                continue;
            }
            let kind = if f & PM_HASHED != 0 {
                'A'
            } else if f & PM_ARRAY != 0 {
                'a'
            } else {
                '-'
            };
            if type_ok(kind) && attr_ok(f & PM_INTEGER != 0, f & PM_READONLY != 0) {
                users.push((**pm).clone());
            }
        }
    }
    for u in &users {
        rows.push((u.node.nam.clone(), None));
    }
    let mut rows = rows;
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    let mut pipestatus_line = show_pipestatus.then(|| {
        let body: Vec<String> = pipestatus.iter().enumerate().map(|(i, v)| format!("[{i}]=\"{v}\"")).collect();
        format!("declare -a PIPESTATUS=({})", body.join(" "))
    });
    for (name, special) in rows {
        if name.as_str() > "PIPESTATUS" {
            if let Some(l) = pipestatus_line.take() {
                println!("{l}");
            }
        }
        match special {
            Some(s) => {
                let mut flags = String::new();
                if s.kind != '-' {
                    flags.push(s.kind);
                }
                if s.integer {
                    flags.push('i');
                }
                if s.readonly {
                    flags.push('r');
                }
                let flags = if flags.is_empty() { "--".to_string() } else { format!("-{flags}") };
                match special_value(s.name) {
                    Some(v) => println!("declare {flags} {name}={v}"),
                    None => println!("declare {flags} {name}"),
                }
            }
            None => {
                if let Some(mut pm) = users.iter().find(|u| u.node.nam == name).cloned() {
                    crate::ported::params::printparamnode(&mut pm, crate::ported::zsh_h::PRINT_TYPESET);
                }
            }
        }
    }
    if let Some(l) = pipestatus_line.take() {
        println!("{l}");
    }
}
