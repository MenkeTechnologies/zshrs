//! Port of `_ldap_attributes` from `Completion/Unix/Type/_ldap_attributes`.
//!
//! Full upstream body (27 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local -a expl attrs
//! sh: 9  attrs=( associatedDomain authenticationMethod … userSMIMECertificate )
//!        (note: the `;` in `cACertificate;binary` separates words — see ATTRS)
//! sh:25  _description ldap-attributes expl "ldap attribute"
//! sh:26  compadd "${@:/-X/-x}" "${expl[@]:/-X/-x}" \
//! sh:27      -M 'm:{a-zA-Z}={A-Za-z} r:[^A-Z]||[A-Z]=* r:|=*' -a attrs
//! ```
//!
//! sh:26 — `${@:/-X/-x}` / `${expl[@]:/-X/-x}` rewrite any `-X` (exclusive
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

/// sh:9-23 — combined OpenLDAP + FreeIPA attribute names.
///
/// sh:11 and sh:21 read `cACertificate;binary` / `userCertificate;binary`, but
/// the `;` is unquoted inside an array literal and zsh's lexer separates words
/// there: `zsh -f -c 'a=( x cACertificate;binary cn ); print -l $#a $a'` prints
/// `4 / x / cACertificate / binary / cn`. So upstream's array holds
/// `cACertificate`, `binary`, `userCertificate`, `binary` as four elements, the
/// duplicate `binary` collapses in compadd, and the offered set is 66 names —
/// which with the three filter operators is the 69 matches real zsh reports for
/// `ldapsearch <TAB>`. Keeping `cACertificate;binary` as ONE element instead
/// offered 67 and listed a name zsh never lists.
const ATTRS: &[&str] = &[
    "associatedDomain",
    "authenticationMethod",
    "automountInformation",
    "automountKey",
    "automountMapName",
    "bindTimeLimit",
    "cACertificate",
    "binary", // sh:11 — `;` separates words in an array literal
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
    "userCertificate",
    "binary", // sh:21 — duplicate; compadd collapses it
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
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_ldap_attributes");
    // sh:25
    let _ = _description(&[
        "ldap-attributes".to_string(),
        "expl".to_string(),
        "ldap attribute".to_string(),
    ]);
    // sh:26-27
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

    /// The `;` in upstream's array literal is a word separator, not part of a
    /// name — measured in real zsh (see the ATTRS doc comment). A `;binary`
    /// element here is a name zsh never offers, and it also costs a match:
    /// 66 unique names, not 65.
    #[test]
    fn semicolon_names_are_separate_elements() {
        assert!(!ATTRS.iter().any(|a| a.contains(';')));
        assert_eq!(ATTRS.iter().filter(|a| **a == "binary").count(), 2);
        let unique: std::collections::HashSet<&&str> = ATTRS.iter().collect();
        assert_eq!(unique.len(), 66);
    }

    #[test]
    fn x_flag_rewritten_to_lowercase() {
        assert_eq!(
            x_to_lower(&["-X".to_string(), "keep".to_string()]),
            vec!["-x".to_string(), "keep".to_string()]
        );
    }
}
