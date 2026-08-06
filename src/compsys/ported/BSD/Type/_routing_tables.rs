//! Port of `_routing_tables` from `Completion/BSD/Type/_routing_tables`.
//!
//! Full upstream body (7 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local expl
//! sh: 5  _description routing-tables expl 'routing table'
//! sh: 6  compadd "$@" "$expl[@]" -  ${(s: :)${${(M)${(f)"$(_call_program routing-tables netstat -R)"}:#  Routing tables#: *}#*: }}
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

/// sh:6 — `${${(M)${(f)"..."}:#  Routing tables#: *}#*: }`.
///
/// Match the whole line against `  Routing table` + zero-or-more `s`
/// (the `s#` extended-glob atom) + `: ` + anything, then strip the
/// shortest `*: ` prefix (i.e. everything through the first `": "`).
/// Returns the remainder (the space-separated table names) on match.
fn strip_routing_tables_header(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("  Routing table")?;
    let rest = rest.trim_start_matches('s');
    rest.strip_prefix(": ")
}

/// `_routing_tables` — offer the list of routing table names reported
/// by `netstat -R` (BSD).
pub fn _routing_tables(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_routing_tables");
    // sh:5  _description routing-tables expl 'routing table'
    let _ = _description(&[
        "routing-tables".to_string(),
        "expl".to_string(),
        "routing table".to_string(),
    ]);

    // sh:6  $(_call_program routing-tables netstat -R)
    let _ = _call_program(&["routing-tables".to_string(), "netstat -R".to_string()]);
    let out = getsparam("REPLY").unwrap_or_default();

    // sh:6  ${(f)"$(...)"} split into lines, ${(M)...:#  Routing tables#: *}
    //       keep only the header line, ${...#*: } strip the header prefix,
    //       ${(s: :)...} split the remainder on spaces.
    let mut tables: Vec<String> = Vec::new();
    for line in out.lines() {
        if let Some(rest) = strip_routing_tables_header(line) {
            tables.extend(rest.split(' ').filter(|s| !s.is_empty()).map(String::from));
        }
    }

    // sh:6  compadd "$@" "$expl[@]" -  ${...}
    let mut cadd: Vec<String> = args.to_vec();
    cadd.extend(getaparam("expl").unwrap_or_default());
    cadd.push("-".to_string());
    cadd.extend(tables);
    bin_compadd("compadd", &cadd, &make_ops(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_header_matches_singular_and_plural() {
        assert_eq!(
            strip_routing_tables_header("  Routing tables: route ipx ns"),
            Some("route ipx ns")
        );
        assert_eq!(
            strip_routing_tables_header("  Routing table: route"),
            Some("route")
        );
    }

    #[test]
    fn strip_header_rejects_non_header_lines() {
        assert_eq!(strip_routing_tables_header("Internet:"), None);
        assert_eq!(strip_routing_tables_header(""), None);
    }

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(_routing_tables(&[]), 1);
    }
}
