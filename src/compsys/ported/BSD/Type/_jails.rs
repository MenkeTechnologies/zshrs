//! Port of `_jails` from `Completion/BSD/Type/_jails`.
//!
//! Head comment (sh:3-7) — options:
//!   -0        include jid 0 as a match for the host system
//!   -o param  jail parameter to complete instead of jid -
//!                e.g. name, path, ip4.addr, host.hostname
//!
//! Full upstream body (34 lines, abridged — the head is a usage comment):
//! ```text
//! sh: 1  #autoload
//! sh: 9  local addhost host param desc=1
//! sh:10  local -a jails args expl
//! sh:11  zparseopts -D -K -E 0=addhost o:=param
//! sh:12  param=${param[2]:-name}
//! sh:14  jails=( ${${(f)"$(_call_program jails jls $param name)"}/ /:} )
//! sh:15  case $param in
//! sh:16    jid) host=0 ;;
//! sh:17    name)
//! sh:18      host=0
//! sh:19      desc=0
//! sh:20    ;;
//! sh:21    path)
//! sh:22      host=/
//! sh:23      args=( -M 'r:|/=* r:|=*' )
//! sh:24    ;;
//! sh:25    ip4.addr) args=( -M 'r:|.=* r:|=*' ) ;;
//! sh:26  esac
//! sh:27  [[ -n $addhost && -n $host ]] && jails+=( "$host:$HOST" )
//! sh:29  if (( desc )); then
//! sh:30    _describe -t jails jail jails "$@" "$args[@]"
//! sh:31  else
//! sh:32    _wanted jails expl jail compadd "$@" "$args[@]" - ${jails%:*}
//! sh:33  fi
//! ```

use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_describe::_describe;
use crate::compsys::ported::_wanted::wanted_byname;
use crate::ported::params::{getsparam, setaparam, unsetparam};

/// sh:11 — bridge for `zparseopts -D -K -E 0=addhost o:=param`. `-E`
/// means the whole argv is scanned (not just a leading option run);
/// `-D` removes matched flags/values, leaving everything else as the
/// passthrough `rest` (the `"$@"` later handed to `_describe`/`_wanted`).
fn zparse_0_o(args: &[String]) -> (bool, Option<String>, Vec<String>) {
    let mut addhost = false;
    let mut param = None;
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-0" => {
                addhost = true; // sh:11 0=addhost
                i += 1;
            }
            "-o" => {
                // sh:11 o:=param — value-taking.
                if i + 1 < args.len() {
                    param = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    i += 1;
                }
            }
            _ => {
                rest.push(args[i].clone());
                i += 1;
            }
        }
    }
    (addhost, param, rest)
}

/// sh:14 — `${line/ /:}`: replace only the *first* space in a line
/// with a colon (zsh single-slash substitution replaces one match).
fn first_space_to_colon(line: &str) -> String {
    match line.find(' ') {
        Some(pos) => {
            let mut s = String::with_capacity(line.len());
            s.push_str(&line[..pos]);
            s.push(':');
            s.push_str(&line[pos + 1..]);
            s
        }
        None => line.to_string(),
    }
}

/// sh:32 — `${jails%:*}`: strip the shortest suffix starting at the
/// *last* colon (i.e. drop the trailing `:name` field).
fn strip_after_last_colon(s: &str) -> String {
    match s.rfind(':') {
        Some(pos) => s[..pos].to_string(),
        None => s.to_string(),
    }
}

/// `_jails` — complete FreeBSD jail identifiers via `jls`.
pub fn _jails(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_jails");
    // sh:11
    let (addhost, param_opt, rest) = zparse_0_o(args);
    // sh:12 — ${param[2]:-name}: default when unset OR empty.
    let param = param_opt
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "name".to_string());

    // sh:14
    let _ = _call_program(&[
        "jails".to_string(),
        "jls".to_string(),
        param.clone(),
        "name".to_string(),
    ]);
    let mut jails: Vec<String> = getsparam("REPLY")
        .unwrap_or_default()
        .lines()
        .map(first_space_to_colon)
        .collect();

    // sh:15-26
    let mut host: Option<String> = None;
    let mut desc = true;
    let mut extra_args: Vec<String> = Vec::new();
    match param.as_str() {
        "jid" => host = Some("0".to_string()), // sh:16
        "name" => {
            host = Some("0".to_string()); // sh:18
            desc = false; // sh:19
        }
        "path" => {
            host = Some("/".to_string()); // sh:22
            extra_args = vec!["-M".to_string(), "r:|/=* r:|=*".to_string()]; // sh:23
        }
        "ip4.addr" => {
            extra_args = vec!["-M".to_string(), "r:|.=* r:|=*".to_string()]; // sh:25
        }
        _ => {}
    }

    // sh:27
    if addhost {
        if let Some(h) = &host {
            let hostname = getsparam("HOST").unwrap_or_default();
            jails.push(format!("{}:{}", h, hostname));
        }
    }

    // sh:29-33
    if desc {
        // sh:30  _describe -t jails jail jails "$@" "$args[@]"
        setaparam("jails", jails);
        let mut a: Vec<String> = vec![
            "-t".to_string(),
            "jails".to_string(),
            "jail".to_string(),
            "jails".to_string(),
        ];
        a.extend(rest);
        a.extend(extra_args);
        // sh:67 is a bare command word — reach it by name so `$fpath`/shfunc
        // arbitration runs and `_describe`'s locals land in its own scope.
        let r = crate::compsys::ported::shared::call_compfn("_describe", &a, || _describe(&a));
        unsetparam("jails");
        r
    } else {
        // sh:32  _wanted jails expl jail compadd "$@" "$args[@]" - ${jails%:*}
        let stripped: Vec<String> = jails.iter().map(|s| strip_after_last_colon(s)).collect();
        let mut a: Vec<String> = vec![
            "jails".to_string(),
            "expl".to_string(),
            "jail".to_string(),
            "compadd".to_string(),
        ];
        a.extend(rest);
        a.extend(extra_args);
        a.push("-".to_string());
        a.extend(stripped);
        wanted_byname(&a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zparse_pulls_0_and_o_leaving_rest() {
        let (addhost, param, rest) = zparse_0_o(&[
            "-0".to_string(),
            "-J".to_string(),
            "grp".to_string(),
            "-o".to_string(),
            "path".to_string(),
        ]);
        assert!(addhost);
        assert_eq!(param.as_deref(), Some("path"));
        assert_eq!(rest, vec!["-J".to_string(), "grp".to_string()]);
    }

    #[test]
    fn zparse_defaults_when_absent() {
        let (addhost, param, rest) = zparse_0_o(&["foo".to_string()]);
        assert!(!addhost);
        assert_eq!(param, None);
        assert_eq!(rest, vec!["foo".to_string()]);
    }

    #[test]
    fn first_space_to_colon_replaces_only_first() {
        assert_eq!(first_space_to_colon("3 myjail extra"), "3:myjail extra");
        assert_eq!(first_space_to_colon("noSpaceHere"), "noSpaceHere");
    }

    #[test]
    fn strip_after_last_colon_drops_trailing_field() {
        assert_eq!(strip_after_last_colon("0:myhost.example"), "0");
        assert_eq!(strip_after_last_colon("myjail:myjail"), "myjail");
        assert_eq!(strip_after_last_colon("nocolon"), "nocolon");
    }

    #[test]
    fn returns_one_without_registered_tags() {
        // sh:19/32 — default param ("name") ⇒ desc=0 ⇒ the `_wanted`
        // branch, which returns 1 when no completion tagset is
        // registered (mirrors `_hosts.rs`'s analogous test).
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(1, std::sync::atomic::Ordering::Relaxed);
        let r = _jails(&[]);
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(r, 1);
    }
}
