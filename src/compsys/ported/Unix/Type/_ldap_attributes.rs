//! Port of `_ldap_attributes` from `Completion/Unix/Type/_ldap_attributes`.
//!
//! Full upstream body (27 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local -a expl attrs
//! sh: 9  attrs=( associatedDomain authenticationMethod … userSMIMECertificate )
//! sh:26  _description ldap-attributes expl "ldap attribute"
//! sh:27  compadd "${@:/-X/-x}" "${expl[@]:/-X/-x}" \
//! sh:28      -M 'm:{a-zA-Z}={A-Za-z} r:[^A-Z]||[A-Z]=* r:|=*' -a attrs
//! ```
//!
//! sh:27 — `${@:/-X/-x}` / `${expl[@]:/-X/-x}` rewrite any `-X` (exclusive
//! description) to `-x` (informational message): a custom attribute is always
//! allowed, so the offered list is advisory, not exhaustive.

use crate::compsys::ported::_description::_description;
use crate::ported::params::{getaparam, setaparam};
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

/// sh:9-25 — combined OpenLDAP + FreeIPA attribute names.
const ATTRS: &[&str] = &[
    "associatedDomain",
    "authenticationMethod",
    "automountInformation",
    "automountKey",
    "automountMapName",
    "bindTimeLimit",
    "cACertificate;binary",
    "cn",
    "dc",
    "defaultSearchBase",
    "defaultServerList",
    "description",
    "displayName",
    "dn",
    "followReferrals",
    "gecos",
    "gidNumber",
    "givenName",
    "homeDirectory",
    "info",
    "initials",
    "ipaCertIssuerSerial",
    "ipaCertSubject",
    "ipaConfigString",
    "ipaKeyExtUsage",
    "ipaKeyTrust",
    "ipaNTSecurityIdentifier",
    "ipaPublicKey",
    "ipaUniqueID",
    "ipHostNumber",
    "loginShell",
    "mail",
    "member",
    "memberUid",
    "mepManagedBy",
    "nisDomain",
    "nisNetgroupTriple",
    "o",
    "objectClass",
    "objectClassMap",
    "ou",
    "pwdAllowUserChange",
    "pwdAttribute",
    "pwdCheckQuality",
    "pwdExpireWarning",
    "pwdFailureCountInterval",
    "pwdGraceAuthNLimit",
    "pwdInHistory",
    "pwdLockout",
    "pwdLockoutDuration",
    "pwdMaxAge",
    "pwdMaxFailure",
    "pwdMinAge",
    "pwdMinLength",
    "pwdMustChange",
    "pwdSafeModify",
    "searchTimeLimit",
    "serviceSearchDescriptor",
    "sn",
    "telephoneNumber",
    "uid",
    "uidNumber",
    "userCertificate;binary",
    "userPKCS12",
    "userSMIMECertificate",
];

/// sh:27 — `${...:/-X/-x}`: rewrite each `-X` element to `-x`.
fn x_to_lower(v: &[String]) -> Vec<String> {
    v.iter()
        .map(|e| {
            if e == "-X" {
                "-x".to_string()
            } else {
                e.clone()
            }
        })
        .collect()
}

/// `_ldap_attributes` — complete LDAP attribute names (non-exhaustive).
pub fn _ldap_attributes(args: &[String]) -> i32 {
    // sh:26
    let _ = _description(&[
        "ldap-attributes".to_string(),
        "expl".to_string(),
        "ldap attribute".to_string(),
    ]);
    // sh:27-28
    setaparam("attrs", ATTRS.iter().map(|s| s.to_string()).collect());
    let expl = getaparam("expl").unwrap_or_default();
    let mut cadd = x_to_lower(args);
    cadd.extend(x_to_lower(&expl));
    cadd.push("-M".to_string());
    cadd.push("m:{a-zA-Z}={A-Za-z} r:[^A-Z]||[A-Z]=* r:|=*".to_string());
    cadd.push("-a".to_string());
    cadd.push("attrs".to_string());
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
        assert_eq!(_ldap_attributes(&[]), 1);
    }

    #[test]
    fn x_flag_rewritten_to_lowercase() {
        assert_eq!(
            x_to_lower(&["-X".to_string(), "keep".to_string()]),
            vec!["-x".to_string(), "keep".to_string()]
        );
    }
}
