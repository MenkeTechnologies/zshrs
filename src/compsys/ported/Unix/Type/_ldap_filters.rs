//! Port of `_ldap_filters` from `Completion/Unix/Type/_ldap_filters`.
//!
//! LDAP search filters conforming to RFC4515. The shell builds a large
//! `zregexparse` spec array `query` and hands it to `_regex_arguments` to
//! generate the `_ldap_search_filters` function, then calls it:
//! ```text
//! sh:11  matchingrules=( … RFC4517 rule names … )
//! sh:23  classes=( … objectClass values … )
//! sh:43  compquote open close andop orop; open=${(q)open} close=${(q)close}
//! sh:46  [[ -z $compstate[quote] && -z $PREFIX ]] && pre='"('
//! sh:48  zstyle -s …operators list-separator sep || sep=--
//! sh:49  print -v disp -f "%s $sep %s" \| or \& and \! not
//! sh:53  query=( … big zregexparse spec … )
//! sh:90  _regex_arguments _ldap_search_filters "$query[@]"
//! sh:91  _ldap_search_filters
//! ```
//!
//! Every RFC4515 filter branch of the `_regex_arguments` query (sh:53-88) is
//! reproduced verbatim in `build_query` — operators (`! | &`), the per-attribute
//! value completers (homeDirectory→`_directories`, loginShell→`/etc/shells`,
//! mail→`_email_addresses`, objectClass→`classes`, uid/automountKey→`_users`,
//! cn→`_alternative`), matching-rules, comparison operators, object-value
//! message, brackets and nest tracking. The ACTION strings (`:tag:desc:cmd`,
//! `-'code'`) are eval'd at completion time by the regex engine, not here — so
//! they keep their `${(Q)open}` / `${(Q)close}` / `${pre:-…}` / `${=query[…]}`
//! text verbatim and read the parameters this port sets, exactly as upstream
//! does. Resolving them at build time is not equivalent: it turns a substituted
//! `(` into a source-literal one, and a source-literal `(` is a filename-
//! generation pattern (`bad pattern: (` on zsh and zshrs alike).
//!
//! `compquote` (sh:43) re-quotes the `( ) & |` operator variables for the
//! CURRENT quoting context (`$compstate[quote]`). This port applies the
//! unquoted-context result — backslash-quoting — plus the subsequent `${(q)…}`
//! pass (sh:44) directly. `print -v disp` (sh:49) builds the operator display
//! array directly.

use crate::compsys::ported::_regex_arguments::_regex_arguments;
use crate::compsys::ported::shared::LocalScope;
use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getsparam, setaparam, setsparam};
use crate::ported::zle::compcore::get_compstate_str;
use crate::ported::zsh_h::PM_ARRAY;

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
    // sh:5-7 — `local -a expl excl optype disp end pre` / `local -i nest=0` /
    // `local open='(' close=')' andop='&' orop='|'`. These are not bookkeeping:
    // the ACTION strings are eval'd later (by `zregexparse`, while this frame is
    // still live) and read `$open`, `$close`, `$pre`, `$nest`, `$optype` and
    // `$query` BY NAME, so they have to exist as real parameters — and be gone
    // again when `_ldap_filters` returns, since `query`/`nest`/`open` are names
    // other completers use too.
    let mut _locals = LocalScope::declare(
        &[
            "expl",
            "excl",
            "optype",
            "disp",
            "end",
            "pre",
            "matchingrules",
            "classes",
            "query",
        ],
        PM_ARRAY,
    );
    _locals.also(&["nest", "open", "close", "andop", "orop", "sep"], 0);
    setsparam("nest", "0"); // sh:6

    // sh:9  [[ -prefix - ]] && return 1  — an option prefix is not a filter.
    if getsparam("PREFIX").unwrap_or_default().starts_with('-') {
        return 1;
    }

    // Referenced arrays for the action strings.
    setaparam(
        "matchingrules",
        MATCHING_RULES.iter().map(|s| s.to_string()).collect(),
    );
    setaparam("classes", CLASSES.iter().map(|s| s.to_string()).collect());

    // sh:43-44 — `compquote open close andop orop` then
    // `open=${(q)open} close=${(q)close}`. In the unquoted context compquote
    // backslash-quotes, so `open` becomes `\(` and the `${(q)…}` pass makes it
    // `\\\(`; the action strings undo one level with `${(Q)open}`. Both values
    // are set as parameters rather than baked into the action text, because
    // baking them turns a SUBSTITUTED `(` into a SOURCE-LITERAL one, and a
    // source-literal `(` in `${pre:-(}` is filename generation — `zsh -fc 'p=;
    // print -r -- ${p:-(}'` is "bad pattern: (" on zsh and zshrs alike.
    setsparam("open", r"\\\(");
    setsparam("close", r"\\\)");
    setsparam("andop", r"\&");
    setsparam("orop", r"\|");

    // sh:46 — `[[ -z $compstate[quote] && -z $PREFIX ]] && pre='"('`:
    // default to double rather than backslash quoting. This is what makes the
    // completed filter open with `"(` instead of a bare `(`.
    if get_compstate_str("quote").unwrap_or_default().is_empty()
        && getsparam("PREFIX").unwrap_or_default().is_empty()
    {
        setsparam("pre", "\"(");
    }

    // sh:48 — list-separator style (default `--`).
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let sep = lookup_sep(&curcontext);
    // sh:49 — the operator display array (`| -- or`, `& -- and`, `! -- not`).
    setaparam(
        "disp",
        vec![
            format!("| {} or", sep),
            format!("& {} and", sep),
            format!("! {} not", sep),
        ],
    );
    // sh:50 — end display (`) -- end`).
    setaparam("end", vec![format!(") {} end", sep)]);
    // sh:51 — excl chars used by the `compadd -F` globs.
    setaparam(
        "excl",
        vec!["!".to_string(), "\\|".to_string(), "&".to_string()],
    );

    // sh:53-88 — the zregexparse spec (see build_query). sh:79/83 read
    // `${=query[nest]}` back out of it, so it is a parameter as well.
    let query = build_query();
    setaparam("query", query.clone());
    let mut argv = vec!["_ldap_search_filters".to_string()];
    argv.extend(query);
    // sh:90
    let _ = _regex_arguments(&argv);
    // sh:91
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
    // NUL + whitespace class for the leading skip pattern (sh:54).
    let skip = "/*\u{0}[ \t\n]#/".to_string();
    let s = |x: &str| x.to_string();
    // The `${…}` in the ACTION elements is deliberate: zregexparse eval's each
    // action at completion time, when `$open`/`$close`/`$pre`/`$query` are the
    // live parameters set above. The PATTERN elements are built at spec-build
    // time in the shell (they are double-quoted), so they carry the resolved
    // `${open}`==`\\\(` / `${close}`==`\\\)` / `${(q)orop}`==`\\\|` /
    // `${andop}`==`\&` text instead. Both halves were read off a real zsh run:
    // `_regex_arguments` was wrapped in a completion of `ldapsearch <TAB>` and
    // its argv dumped, so every element below is what zsh actually passes.
    vec![
        s("("), skip, s(")"),
        s("("),
        s("("), s(r"/\\\(!/"), s("-optype[++nest]=1;pre=\"\""),          // sh:56
        s("|"), s(r"/\\\(\\\|/"), s("-optype[++nest]=2;pre=\"\""),       // sh:57
        s("|"), s(r"/\\\(\&/"), s("-optype[++nest]=3;pre=\"\""),         // sh:58
        s("|"), s("/[]/"),                                               // sh:59
        s(r#":operators:operator:compadd -F "( ${(q)excl[optype[nest]]} )" -d disp -P ${pre:-${(Q)open}} -S ${(Q)open} \| \& \!"#),
        s(")"),
        s("|"),
        s("("), s(r"/\\\([^\)]##/"), s(r"%\\\)%"),                       // sh:61
        s("|"), s(r"/\\\((#i)homeDirectory=/"), s("/[]/"),               // sh:62
        s(r#":directories:directory:_directories -P / -W / -r ") \t\n\-""#),
        s("|"), s(r"/\\\((#i)loginShell=/"), s("/[]/"),                  // sh:63
        s(r#":shells:shell:compadd -S ${(Q)close} ${(f)^"$(</etc/shells)"}(N)"#),
        s("|"), s(r"/\\\((#i)mail=/"), s("/[]/"),                        // sh:64
        s(":email-addresses:mail:_email_addresses -S ${(Q)close}"),
        s("|"), s(r"/\\\((#i)objectClass=/"), s("/[]/"),                 // sh:65
        s(r#":object-classes:class:compadd -S ${(Q)close} -M "m:{a-zA-Z}={A-Za-z} r:[^A-Z]||[A-Z]=* r:|=*" -a classes"#),
        s("|"), s(r"/\\\((#i)(automountKey|(member|)uid)=/"), s("/[]/"), // sh:66
        s(":users:username:_users -S ${(Q)close}"),
        s("|"), s(r"/\\\((#i)cn=/"), s("/[]/"),                          // sh:67
        s(r#":cn:cn: _alternative "users:user:_users -S ${close}" "groups:group:_groups -S ${close}" "hosts:host:_hosts -S ${close}""#),
        s("|"),
        s("/[^:=<>~]##/"), s("%[=:<>~]%"), s("-pre=\"\""),               // sh:69
        s(r#":object-types:object type:_ldap_attributes -P ${pre:-${(Q)open}}  -S = -r ":=~<> \t\n\-""#), // sh:70
        s("("),
        s("/:/"),                                                        // sh:72
        s("/[^:]##:=/"),                                                 // sh:73
        s(r#":matching-rules:matching rule:compadd -S ":=" -a matchingrules"#),
        s("|"),
        s("/([~<>]|)=/"),                                                // sh:75
        s(r#":operators:operator:compadd -S "" "<=" \>= \~="#),
        s(")"),
        s(r"/[^\\)]##/"), s(r"%\\\)%"),                                  // sh:77
        s(r#": _message -e object-values "object value (* for presence check)""#),
        s(")"),
        s(r"/\\\)/"), s("-(( nest ))"),                                  // sh:79
        s(r#":brackets:bracket:compadd ${=query[nest]:+-S ""} \)"#),
        s("("),
        s(r"/\\\)/"),                                                    // sh:83
        s(r#":operators:operator:compadd ${=query[nest-1]:+-S ""} -d end -P ${(Q)close} """#),
        s("("), s("//"), s("-(( --nest ))"), s("|"), s("//"), s("-((!nest))"), s("/[]/"), s(": compadd \"\""), s(")"), // sh:84
        s(")"), s("#"),                                                  // sh:85
        s("//"), s("-(( nest && optype[nest] > 1 ))"),                   // sh:86
        s(")"), s("#"),                                                  // sh:87
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
