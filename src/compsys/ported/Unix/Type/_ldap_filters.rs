//! Port of `_ldap_filters` from `Completion/Unix/Type/_ldap_filters`.
//!
//! LDAP search filters conforming to RFC4515. The shell builds a large
//! `zregexparse` spec array `query` and hands it to `_regex_arguments` to
//! generate the `_ldap_search_filters` function, then calls it:
//! ```text
//! sh: 9  matchingrules=( … RFC4517 rule names … )
//! sh:22  classes=( … objectClass values … )
//! sh:40  compquote open close andop orop; open=${(q)open} close=${(q)close}
//! sh:43  zstyle -s …operators list-separator sep || sep=--
//! sh:49  print -v disp -f "%s $sep %s" \| or \& and \! not
//! sh:47  query=( … big zregexparse spec … )
//! sh:90  _regex_arguments _ldap_search_filters "$query[@]"
//! sh:91  _ldap_search_filters
//! ```
//!
//! Every RFC4515 filter branch of the `_regex_arguments` query (sh:52-89) is
//! reproduced verbatim in `build_query` — operators (`! | &`), the per-attribute
//! value completers (homeDirectory→`_directories`, loginShell→`/etc/shells`,
//! mail→`_email_addresses`, objectClass→`classes`, uid/automountKey→`_users`,
//! cn→`_alternative`), matching-rules, comparison operators, object-value
//! message, brackets and nest tracking. The ACTION strings (`:tag:desc:cmd`,
//! `-'code'`) are eval'd at completion time by the regex engine, not here.
//!
//! `compquote` (sh:43) is the completion-system builtin that re-quotes the
//! `( ) & |` operator variables for the CURRENT quoting context
//! (`$compstate[quote]`). zshrs does not yet expose that builtin, so this port
//! applies the subsequent `${(q)…}` backslash-quoting (sh:44) directly — which
//! is exactly what `compquote` reduces to in the common unquoted context
//! (`$compstate[quote]` empty). `print -v disp` (sh:49) builds the operator
//! display array directly.

use crate::compsys::ported::_regex_arguments::_regex_arguments;
use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getsparam, setaparam};

/// sh:9-20 — RFC4517 matching-rule names (referenced by an action string).
const MATCHING_RULES: &[&str] = &[
    "bitStringMatch",
    "booleanMatch",
    "caseExactIA5Match",
    "caseExactMatch",
    "caseExactOrderingMatch",
    "caseExactSubstringsMatch",
    "caseIgnoreIA5Match",
    "caseIgnoreIA5SubstringsMatch",
    "caseIgnoreListMatch",
    "caseIgnoreListSubstringsMatch",
    "caseIgnoreMatch",
    "caseIgnoreOrderingMatch",
    "caseIgnoreSubstringsMatch",
    "directoryStringFirstComponentMatch",
    "distinguishedNameMatch",
    "generalizedTimeMatch",
    "generalizedTimeOrderingMatch",
    "integerFirstComponentMatch",
    "integerMatch",
    "integerOrderingMatch",
    "keywordMatch",
    "numericStringMatch",
    "numericStringOrderingMatch",
    "numericStringSubstringsMatch",
    "objectIdentifierFirstComponentMatch",
    "objectIdentifierMatch",
    "octetStringMatch",
    "octetStringOrderingMatch",
    "telephoneNumberMatch",
    "telephoneNumberSubstringsMatch",
    "uniqueMemberMatch",
    "wordMatch",
];

/// sh:22-38 — objectClass values (referenced by an action string).
const CLASSES: &[&str] = &[
    "automount",
    "automountMap",
    "cosTemplate",
    "dcObject",
    "device",
    "dnaSharedConfig",
    "domain",
    "domainRelatedObject",
    "DUAConfigProfile",
    "extensibleObject",
    "groupOfNames",
    "groupOfPrincipals",
    "ieee802device",
    "inetOrgPerson",
    "inetuser",
    "ipaassociation",
    "ipaca",
    "ipacaacl",
    "ipaCertificate",
    "ipaCertMapConfigObject",
    "ipacertprofile",
    "ipaConfigObject",
    "ipaDomainIDRange",
    "ipaDomainLevelConfig",
    "ipaGuiConfig",
    "ipahbacrule",
    "ipahbacservice",
    "ipahbacservicegroup",
    "ipahost",
    "ipahostgroup",
    "ipaIDrange",
    "ipaKeyPolicy",
    "ipakrbprincipal",
    "ipaNameResolutionData",
    "ipaNTDomainAttrs",
    "ipaNTGroupAttrs",
    "ipaNTUserAttrs",
    "ipaobject",
    "ipaPublicKeyObject",
    "ipaReplTopoManagedServer",
    "ipaservice",
    "ipaSshGroupOfPubKeys",
    "ipasshhost",
    "ipasshuser",
    "ipasudorule",
    "ipaSupportedDomainLevelConfig",
    "ipaTrustedADDomainRange",
    "ipaUserAuthTypeClass",
    "ipausergroup",
    "ipHost",
    "krbContainer",
    "krbprincipal",
    "krbprincipalaux",
    "krbrealmcontainer",
    "krbTicketPolicyAux",
    "mepManagedEntry",
    "mepOriginEntry",
    "nestedGroup",
    "nisDomainObject",
    "nisNetgroup",
    "nsContainer",
    "nsDS5Replica",
    "nshost",
    "organization",
    "organizationalPerson",
    "organizationalRole",
    "organizationalUnit",
    "person",
    "pilotObject",
    "pkiCA",
    "pkiuser",
    "posixAccount",
    "posixGroup",
    "pwdPolicy",
    "shadowAccount",
    "simpleSecurityObject",
    "top",
];

/// `_ldap_filters` — complete RFC4515 LDAP search filter expressions.
pub fn _ldap_filters(_args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_ldap_filters");
    // sh:6  [[ -prefix - ]] && return 1  — an option prefix is not a filter.
    if getsparam("PREFIX").unwrap_or_default().starts_with('-') {
        return 1;
    }

    // Referenced arrays for the action strings.
    setaparam(
        "matchingrules",
        MATCHING_RULES.iter().map(|s| s.to_string()).collect(),
    );
    setaparam("classes", CLASSES.iter().map(|s| s.to_string()).collect());

    // sh:43 — list-separator style (default `--`).
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let sep = lookup_sep(&curcontext);
    // sh:44 — the operator display array (`| -- or`, `& -- and`, `! -- not`).
    setaparam(
        "disp",
        vec![
            format!("| {} or", sep),
            format!("& {} and", sep),
            format!("! {} not", sep),
        ],
    );
    // sh:45 — end display (`) -- end`).
    setaparam("end", vec![format!(") {} end", sep)]);
    // sh:46 — excl chars used by the `compadd -F` globs.
    setaparam(
        "excl",
        vec!["!".to_string(), "\\|".to_string(), "&".to_string()],
    );

    // sh:47-113 — the zregexparse spec (see build_query). sh:40 approx —
    // `${open}`==`\(`, `${close}`==`\)`, `${(Q)open}`==`(`, `${(Q)close}`==`)`,
    // `${(q)orop}`==`\|`, `${andop}`==`&`.
    let mut argv = vec!["_ldap_search_filters".to_string()];
    argv.extend(build_query());
    // sh: 90
    let _ = _regex_arguments(&argv);
    // sh: 91
    dispatch_function_call("_ldap_search_filters", &[]).unwrap_or(1)
}

fn lookup_sep(curcontext: &str) -> String {
    let ctx = format!(":completion:{}:operators", curcontext);
    crate::ported::modules::zutil::lookupstyle(&ctx, "list-separator")
        .into_iter()
        .next()
        .unwrap_or_else(|| "--".to_string())
}

/// sh:47-113 — the `query` array, transcribed. Group/alternation/repeat
/// markers are literal `(` `)` `|` `#`; pattern elements bake in the resolved
/// bracket constants; action elements are kept verbatim for runtime eval.
fn build_query() -> Vec<String> {
    // NUL + whitespace class for the leading skip pattern (sh:49).
    let skip = "/*\u{0}[ \t\n]#/".to_string();
    let s = |x: &str| x.to_string();
    vec![
        s("("), skip, s(")"),
        s("("),
        s("("), s("/\\(!/"), s("-optype[++nest]=1;pre=\"\""),
        s("|"), s("/\\(\\|/"), s("-optype[++nest]=2;pre=\"\""),
        s("|"), s("/\\(&/"), s("-optype[++nest]=3;pre=\"\""),
        s("|"), s("/[]/"),
        s(":operators:operator:compadd -F \"( ${(q)excl[optype[nest]]} )\" -d disp -P ${pre:-(} -S ( \\| \\& \\!"),
        s(")"),
        s("|"),
        s("("), s("/\\([^\\)]##/"), s("%\\)%"),
        s("|"), s("/\\((#i)homeDirectory=/"), s("/[]/"),
        s(":directories:directory:_directories -P / -W / -r \") \\t\\n\\-\""),
        s("|"), s("/\\((#i)loginShell=/"), s("/[]/"),
        s(":shells:shell:compadd -S ) ${(f)^\"$(</etc/shells)\"}(N)"),
        s("|"), s("/\\((#i)mail=/"), s("/[]/"),
        s(":email-addresses:mail:_email_addresses -S )"),
        s("|"), s("/\\((#i)objectClass=/"), s("/[]/"),
        s(":object-classes:class:compadd -S ) -M \"m:{a-zA-Z}={A-Za-z} r:[^A-Z]||[A-Z]=* r:|=*\" -a classes"),
        s("|"), s("/\\((#i)(automountKey|(member|)uid)=/"), s("/[]/"),
        s(":users:username:_users -S )"),
        s("|"), s("/\\((#i)cn=/"), s("/[]/"),
        s(":cn:cn: _alternative \"users:user:_users -S )\" \"groups:group:_groups -S )\" \"hosts:host:_hosts -S )\""),
        s("|"),
        s("/[^:=<>~]##/"), s("%[=:<>~]%"), s("-pre=\"\""),
        s(":object-types:object type:_ldap_attributes -P ${pre:-(}  -S = -r \":=~<> \\t\\n\\-\""),
        s("("),
        s("/:/"),
        s("/[^:]##:=/"), s(":matching-rules:matching rule:compadd -S \":=\" -a matchingrules"),
        s("|"),
        s("/([~<>]|)=/"), s(":operators:operator:compadd -S \"\" \"<=\" \\>= \\~="),
        s(")"),
        s("/[^\\\\)]##/"), s("%\\)%"), s(": _message -e object-values \"object value (* for presence check)\""),
        s(")"),
        s("/\\)/"), s("-(( nest ))"), s(":brackets:bracket:compadd ${=query[nest]:+-S \"\"} \\)"),
        s("("),
        s("/\\)/"), s(":operators:operator:compadd ${=query[nest-1]:+-S \"\"} -d end -P ) \"\""),
        s("("), s("//"), s("-(( --nest ))"), s("|"), s("//"), s("-((!nest))"), s("/[]/"), s(": compadd \"\""), s(")"),
        s(")"), s("#"),
        s("//"), s("-(( nest && optype[nest] > 1 ))"),
        s(")"), s("#"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_prefix_returns_one() {
        // sh:6 — a `-` prefix is not a filter.
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::setsparam("PREFIX", "-x");
        assert_eq!(_ldap_filters(&[]), 1);
    }

    #[test]
    fn query_is_balanced() {
        // The zregexparse spec must have balanced group markers.
        let q = build_query();
        let opens = q.iter().filter(|e| *e == "(").count();
        let closes = q.iter().filter(|e| *e == ")").count();
        assert_eq!(opens, closes, "query groups must balance");
    }
}
