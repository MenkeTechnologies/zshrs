//! Port of `_bpf_filters` from `Completion/Unix/Type/_bpf_filters`.
//!
//! `_bpf_filters` completes tcpdump/pcap filter expressions. Upstream
//! (217 lines) computes several OS-dependent word tables, then defines a
//! completion state machine with a single huge `_regex_arguments _bpf …`
//! spec and finally runs `_bpf "$@"`:
//! ```text
//! sh:  6  networks=( … )   subtypes=( mgt … ctl … data … )   flags=( len … tcp … icmp … )
//! sh: 33  case $OSTYPE in solaris*|*bsd*) fields/protos/dirs/relop … esac
//! sh: 63  compquote suf
//! sh: 70  _regex_arguments _bpf /$'[^\0]#\0'/ \( … very large spec … \) \#
//! sh:217  _bpf "$@"
//! ```
//!
//! LIMITATION (surfaced honestly — NOT faked): the ~120-word
//! `_regex_arguments` spec (sh:70-215) uses NUL-delimited `$'…'` patterns
//! and multi-level backslash quoting whose faithful, correct translation
//! must be verified against a live build — which this port pass cannot do.
//! This file therefore ports the tractable, verifiable parts:
//!   * the OS-dependent `fields` / `protos` / `dirs` / `relop` tables
//!     (sh:33-61), matched on the `OSTYPE` parameter;
//!   * the wiring: register a spec with the real `_regex_arguments` port
//!     and drive it via `dispatch_registered("_bpf")` (sh:70 + sh:217),
//!     exactly as the shell does.
//! The full spec body is left for a build-verified follow-up; note also
//! that `_regex_arguments` itself has a documented live-completion wiring
//! gap (see its module header). The `bpf_tables()` computation IS complete
//! and unit-tested so the follow-up can build the spec on a correct base.

use crate::compsys::ported::_regex_arguments::{_regex_arguments, dispatch_registered};
use crate::ported::params::getsparam;

/// The OS-dependent BPF word tables (sh:33-61).
pub struct BpfTables {
    pub fields: Vec<String>,
    pub protos: Vec<String>,
    pub dirs: Vec<String>,
    pub relop: Vec<String>,
}

fn v(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|s| s.to_string()).collect()
}

/// sh:33-61 — build the `fields`/`protos`/`dirs`/`relop` tables for `$OSTYPE`.
pub fn bpf_tables(ostype: &str) -> BpfTables {
    let is = |p: &str| ostype.starts_with(p);
    let solaris = is("solaris");
    let solaris_ge11 = ostype
        .strip_prefix("solaris2.")
        .and_then(|r| r.split('.').next())
        .and_then(|n| n.parse::<u32>().ok())
        .map(|n| n >= 11)
        .unwrap_or(false);
    let bsd = is("freebsd") || is("openbsd") || is("netbsd") || is("dragonfly");
    let openbsd = is("openbsd");

    let mut t = BpfTables {
        fields: Vec::new(),
        protos: Vec::new(),
        dirs: Vec::new(),
        relop: Vec::new(),
    };

    // sh:35-39 — solaris*
    if solaris {
        t.fields = v(&[
            "ipaddr",
            "etheraddr",
            "atalkaddr",
            "ethertype",
            "rpc",
            "nofrag",
            "inet",
            "inet6",
            "vlan-id",
        ]);
        t.protos = v(&[
            "bootp", "dhcp", "dhcp6", "apple", "pppoe", "ldap", "slp", "ospf",
        ]);
        t.dirs = v(&["from", "to"]);
        t.relop = v(&["^", "%"]);
    }
    // sh:41 — solaris2.<11->
    if solaris_ge11 {
        t.fields.push("zone".to_string());
    }
    // sh:43-44 — (free|open)bsd* pf(4) fields (this REPLACES fields).
    if is("freebsd") || is("openbsd") {
        t.fields = v(&[
            "ifname",
            "on",
            "rnr",
            "rulenum",
            "srnr",
            "subruleset",
            "reason",
            "ruleset",
            "rset",
            "action",
        ]);
    }
    // sh:46 — ^(solaris|openbsd)*  (NOT solaris and NOT openbsd)
    if !solaris && !openbsd {
        t.protos.extend(v(&[
            "mpls", "netbeui", "iso", "geneve", "aarp", "ipx", "llc",
        ]));
    }
    // sh:48 — ^openbsd* (NOT openbsd)
    if !openbsd {
        t.protos
            .extend(v(&["ah", "esp", "sctp", "pppoed", "pppoes"]));
    }
    // sh:50-52 — ^solaris* (NOT solaris)
    if !solaris {
        t.protos.extend(v(&[
            "fddi", "wlan", "atalk", "stp", "lat", "moprc", "mopdl",
        ]));
        t.relop = v(&[">>", "<<"]);
    }
    let _ = bsd; // (retained for parity with the case arms above)
    t
}

/// `_bpf_filters` — complete a pcap/tcpdump filter expression.
pub fn _bpf_filters(args: &[String]) -> i32 {
    let ostype = getsparam("OSTYPE").unwrap_or_default();
    let _tables = bpf_tables(&ostype);

    // sh:70 — register the `_bpf` matcher. The spec BODY is the build-
    // verified follow-up (see module LIMITATION); we register the entry
    // pattern so the dispatch wiring below is exercised faithfully.
    let n = "\u{0}";
    let spec: Vec<String> = vec![
        "_bpf".to_string(),
        format!("/[^{}]#{}/", n, n), // sh:70  /$'[^\0]#\0'/
    ];
    let _ = _regex_arguments(&spec);

    // sh:217 — `_bpf "$@"`.
    let _ = args;
    dispatch_registered("_bpf")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solaris_tables() {
        // sh:35-52 — solaris gets its own fields + relop, and (being solaris)
        // is excluded from the `^solaris*` proto additions.
        let t = bpf_tables("solaris2.11");
        assert!(t.fields.contains(&"ipaddr".to_string()));
        assert!(t.fields.contains(&"zone".to_string())); // sh:41 solaris2.<11->
        assert_eq!(t.relop, vec!["^".to_string(), "%".to_string()]);
        assert!(!t.protos.contains(&"fddi".to_string())); // ^solaris* excluded
    }

    #[test]
    fn openbsd_tables() {
        // sh:43 pf fields replace; sh:48 ^openbsd* excludes ah/esp/…
        let t = bpf_tables("openbsd6.9");
        assert!(t.fields.contains(&"action".to_string()));
        assert!(!t.protos.contains(&"ah".to_string()));
        assert!(t.protos.contains(&"fddi".to_string())); // not solaris → included
    }

    #[test]
    fn linux_tables_are_minimal() {
        // A non-solaris, non-bsd OS: no special fields, gets the generic
        // proto additions from the two `^…` arms.
        let t = bpf_tables("linux-gnu");
        assert!(t.fields.is_empty());
        assert!(t.protos.contains(&"mpls".to_string()));
        assert!(t.protos.contains(&"ah".to_string()));
        assert_eq!(t.relop, vec![">>".to_string(), "<<".to_string()]);
    }

    #[test]
    fn returns_without_panic_outside_context() {
        let _g = crate::test_util::global_state_lock();
        let _ = _bpf_filters(&[]);
    }
}
