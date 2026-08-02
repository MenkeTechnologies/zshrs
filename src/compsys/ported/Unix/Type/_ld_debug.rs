//! Port of `_ld_debug` from `Completion/Unix/Type/_ld_debug`.
//!
//! Full upstream body (40 lines, abridged):
//! ```text
//! sh: 1  #compdef -value-,LD_DEBUG,-default-
//! sh: 3  local vals
//! sh: 5  vals=( 'libs[…]' 'files[…]' 'bindings[…]' 'reloc[…]' 'symbols[…]'
//! sh:      'unused[…]' 'versions[…]' 'help[…]' )
//! sh:16  case $OSTYPE in
//! sh:17    solaris*) vals+=( basic cap detail demangle init long move segments
//! sh:                       strtab tls … ) ;;
//! sh:31    linux*)   vals+=( all scopes statistics ) ;;
//! sh:37  esac
//! sh:39  _values -s , capability $vals
//! ```

use crate::compsys::ported::_values::_values;
use crate::ported::params::getsparam;

/// sh:5-15 — the OS-independent LD_DEBUG capability values.
const BASE: &[&str] = &[
    "libs[display library search paths]",
    "files[show processing of files and libraries]",
    "bindings[display symbol binding]",
    "reloc[display relocation processing]",
    "symbols[display symbol table processing]",
    "unused[show unused files]",
    "versions[show version processing]",
    "help[display help message]",
];

/// sh:18-28 — Solaris-only additions.
const SOLARIS: &[&str] = &[
    "basic[provide basic trace information/warnings]",
    "cap[display hardware/software capability processing]",
    "detail[provide more info in conjunction with other options]",
    "demangle[display C++ symbol names in their demangled form]",
    "init[display init and fini processing]",
    "long[display long object names without truncation]",
    "move[display move section processing]",
    "segments[display available output segments and address/offset processing]",
    "strtab[display information about string table compression]",
    "tls[display TLS processing info]",
];

/// sh:32-34 — Linux-only additions.
const LINUX: &[&str] = &[
    "all[combine all options]",
    "scopes[display scope information]",
    "statistics[display relocation statistics]",
];

/// `_ld_debug` — complete `LD_DEBUG` capability keywords (comma-separated).
pub fn _ld_debug(_args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_ld_debug");
    // sh:5-37 — build the value list, OSTYPE-dependent.
    let mut vals: Vec<String> = BASE.iter().map(|s| s.to_string()).collect();
    let ostype = getsparam("OSTYPE").unwrap_or_default();
    if ostype.starts_with("solaris") {
        vals.extend(SOLARIS.iter().map(|s| s.to_string()));
    } else if ostype.starts_with("linux") {
        vals.extend(LINUX.iter().map(|s| s.to_string()));
    }
    // sh:39  _values -s , capability $vals
    let mut argv = vec!["-s".to_string(), ",".to_string(), "capability".to_string()];
    argv.extend(vals);
    _values(&argv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_ld_debug(&[]), 1);
    }
}
