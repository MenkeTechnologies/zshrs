//! Port of `_routing_domains` from `Completion/BSD/Type/_routing_domains`.
//!
//! Full upstream body (6 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local expl
//! sh: 5  _description routing-domains expl 'routing domain'
//! sh: 6  compadd "$@" "$expl[@]" -  ${${(M)${(f)"$(_call_program routing-domains netstat -R)"}:#Rdomain *}#Rdomain }
//! ```

use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_description::_description;
use crate::ported::params::{getaparam, getsparam};
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:6 — `${${(M)${(f)"$out"}:#Rdomain *}#Rdomain }`: split `$out` into
/// lines, keep only lines matching the glob `Rdomain *`, then strip the
/// literal `Rdomain ` prefix from each survivor.
fn rdomain_lines(out: &str) -> Vec<String> {
    out.lines()
        .filter_map(|l| l.strip_prefix("Rdomain ").map(str::to_string))
        .collect()
}

/// `_routing_domains` — complete routing domain numbers from
/// `netstat -R` output (BSD `Rdomain` column).
pub fn _routing_domains(args: &[String]) -> i32 {
    // sh:5  _description routing-domains expl 'routing domain'
    let _ = _description(&[
        "routing-domains".to_string(),
        "expl".to_string(),
        "routing domain".to_string(),
    ]);

    // sh:6  $(_call_program routing-domains netstat -R)
    let _ = _call_program(&[
        "routing-domains".to_string(),
        "netstat -R".to_string(),
    ]);
    let out = getsparam("REPLY").unwrap_or_default();
    let domains = rdomain_lines(&out);

    // sh:6  compadd "$@" "$expl[@]" -  ${...}
    let mut cadd: Vec<String> = args.to_vec();
    cadd.extend(getaparam("expl").unwrap_or_default());
    cadd.push("-".to_string());
    cadd.extend(domains);
    bin_compadd("compadd", &cadd, &make_ops(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rdomain_lines_keeps_only_prefixed_and_strips_prefix() {
        let out = "Rdomain 0\nsomething else\nRdomain 12\nRdomain12 not-a-match\n";
        assert_eq!(
            rdomain_lines(out),
            vec!["0".to_string(), "12".to_string()]
        );
    }

    #[test]
    fn rdomain_lines_empty_when_no_matches() {
        assert!(rdomain_lines("Routing tables\nDestination Gateway\n").is_empty());
    }

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(_routing_domains(&[]), 1);
    }
}
