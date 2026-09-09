//! Port of `_dns_types` from `Completion/Unix/Type/_dns_types`.
//!
//! Full upstream body (8 lines verbatim):
//! ```text
//! sh:1  #autoload
//! sh:2
//! sh:3  local expl
//! sh:4  _description dns-types expl 'DNS type'
//! sh:5  compadd "$@" "$expl[@]" -M 'm:{a-z}={A-Z}' \
//! sh:6      ANY A AAAA AFSDB APL AXFR CAA CDNSKEY CDS CERT CNAME DHCID DLV DNAME \
//! sh:7      DNSKEY DS HIP HINFO IPSECKEY IXFR KEY KX LOC MX NAPTR NS NSEC NSEC3 \
//! sh:8      NSEC3PARAM OPT PTR RRSIG RP SIG SOA SPF SRV SSHFP TA TKEY TLSA TSIG TXT
//! ```

use crate::compsys::ported::_description::_description;
use crate::ported::params::getaparam;
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

/// sh:6-8 — the fixed list of DNS resource-record type names.
const DNS_TYPES: &[&str] = &[
    "ANY",
    "A",
    "AAAA",
    "AFSDB",
    "APL",
    "AXFR",
    "CAA",
    "CDNSKEY",
    "CDS",
    "CERT",
    "CNAME",
    "DHCID",
    "DLV",
    "DNAME",
    "DNSKEY",
    "DS",
    "HIP",
    "HINFO",
    "IPSECKEY",
    "IXFR",
    "KEY",
    "KX",
    "LOC",
    "MX",
    "NAPTR",
    "NS",
    "NSEC",
    "NSEC3",
    "NSEC3PARAM",
    "OPT",
    "PTR",
    "RRSIG",
    "RP",
    "SIG",
    "SOA",
    "SPF",
    "SRV",
    "SSHFP",
    "TA",
    "TKEY",
    "TLSA",
    "TSIG",
    "TXT",
];

/// `_dns_types` — complete DNS resource-record type names.
pub fn _dns_types(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_dns_types");
    // sh:3 — `local expl`.
    //
    // This port does not assign `expl` itself; it hands the NAME to
    // `_wanted`/`_description`, and `_description` fills it through
    // `setaparam` — `createparam(name, PM_SCALAR)` with no PM_LOCAL
    // (shared.rs:16-30). The array was therefore born at level 0 and
    // `endparamscope` had nothing to unwind, so one TAB left `expl`
    // in the user's shell. Measured through a pty, `${(t)expl}` and
    // the value read back at the next prompt after a single TAB on a
    // `_dns_types` wrapper:
    //
    //   zsh  : [][]
    //   zshrs: [array][-J|-default-]
    //
    // kind 0, as sh:3 spells a bare `local`.
    crate::compsys::ported::shared::declare_locals(&["expl"], 0);
    // sh:4
    let _ = _description(&[
        "dns-types".to_string(),
        "expl".to_string(),
        "DNS type".to_string(),
    ]);
    // sh:5-8 — compadd "$@" "$expl[@]" -M 'm:{a-z}={A-Z}' <types>
    let expl = getaparam("expl").unwrap_or_default();
    let mut cadd: Vec<String> = args.to_vec();
    cadd.extend(expl);
    cadd.push("-M".to_string());
    cadd.push("m:{a-z}={A-Z}".to_string());
    cadd.extend(DNS_TYPES.iter().map(|s| s.to_string()));
    bin_compadd("compadd", &cadd, &make_ops(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(0, Ordering::Relaxed);
        // Outside a completion function, bin_compadd refuses (returns 1).
        assert_eq!(_dns_types(&[]), 1);
    }
}
