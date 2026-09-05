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
//! CURRENT quoting context, and the PATTERN elements of the spec interpolate
//! the result, so the spec is NOT the same on every call. `ldapsearch <TAB>`
//! completes an unquoted word and gets `open`=`\\\(`; the tab after that
//! completes the `"(` the first one inserted, which is a word inside double
//! quotes, where `(` needs no quoting and `open` is `\(`. Baking the
//! unquoted-context text in made every pattern demand a literal backslash the
//! second word does not have, so nothing matched and the second TAB completed
//! in silence where zsh lists 69 matches. `print -v disp` (sh:49) builds the
//! operator display array directly.

use crate::compsys::ported::_regex_arguments::_regex_arguments;
use crate::compsys::ported::shared::LocalScope;
use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getsparam, setaparam, setsparam};
use crate::ported::utils::quotestring;
use crate::ported::zle::compcore::get_compstate_str;
use crate::ported::zle::computil::bin_compquote;
use crate::ported::zsh_h::{options, MAX_OPS, PM_ARRAY, QT_BACKSLASH};

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

/// sh:43 — the parameters `compquote` re-quotes, in source order.
const OPERATOR_PARAMS: [&str; 4] = ["open", "close", "andop", "orop"];

/// `compquote` takes no options here (sh:43 passes none).
fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

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

    // sh:7  `local open='(' close=')' andop='&' orop='|'`
    setsparam("open", "(");
    setsparam("close", ")");
    setsparam("andop", "&");
    setsparam("orop", "|");
    // sh:43 — `compquote open close andop orop`. The result depends on the
    // CURRENT quoting context (`comp_quote` quotes with `*compqstack`,
    // Src/Zle/computil.c:3691-3705), so it cannot be precomputed: on an
    // unquoted word compquote backslash-quotes and `open` becomes `\(`, while
    // on a word already inside double quotes — which is what the SECOND
    // `ldapsearch <TAB>` completes, since the first one inserts `"(` — `(`
    // needs no quoting and `open` stays `(`.
    let params = OPERATOR_PARAMS.map(String::from);
    let _ = bin_compquote("compquote", &params, &make_ops(), 0);
    // sh:44 — `open=${(q)open} close=${(q)close}`.
    setsparam(
        "open",
        &quotestring(&getsparam("open").unwrap_or_default(), QT_BACKSLASH),
    );
    setsparam(
        "close",
        &quotestring(&getsparam("close").unwrap_or_default(), QT_BACKSLASH),
    );
    tracing::debug!(
        target: "compsys::_ldap_filters",
        open = %getsparam("open").unwrap_or_default(),
        close = %getsparam("close").unwrap_or_default(),
        andop = %getsparam("andop").unwrap_or_default(),
        orop = %getsparam("orop").unwrap_or_default(),
        quote = %get_compstate_str("quote").unwrap_or_default(),
        prefix = %getsparam("PREFIX").unwrap_or_default(),
        "sh:43-44 bracket text for this quoting context",
    );

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
    // `${=query[nest]}` back out of it, so it is a parameter as well. The
    // PATTERN elements are double-quoted in the shell, so they interpolate
    // `$open` / `$close` / `$andop` / `${(q)orop}` at spec-BUILD time and
    // therefore carry whatever compquote just produced for this context.
    let query = build_query(
        &getsparam("open").unwrap_or_default(),
        &getsparam("close").unwrap_or_default(),
        &getsparam("andop").unwrap_or_default(),
        &quotestring(&getsparam("orop").unwrap_or_default(), QT_BACKSLASH),
    );
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

/// sh:53-88 — the `query` array, transcribed. Group/alternation/repeat
/// markers are literal `(` `)` `|` `#`; action elements are kept verbatim for
/// runtime eval; PATTERN elements are double-quoted in the shell and so
/// interpolate the post-`compquote` bracket text HERE, at spec-build time.
///
/// `open`/`close` are `${(q)}`-quoted (sh:44), `andop` is used raw (sh:58) and
/// `orop` is `${(q)}`-quoted at the point of use (sh:57) — hence `qorop`.
/// Measured against real zsh (both values are what a dump of the argv zsh
/// passes to `_regex_arguments` contains):
///
/// | context                  | open     | close    | andop | qorop    |
/// |--------------------------|----------|----------|-------|----------|
/// | unquoted (`ldapsearch `) | `\\\(`   | `\\\)`   | `\&`  | `\\\|`   |
/// | inside `"` (`… "(`)      | `\(`     | `\)`     | `&`   | `\|`     |
fn build_query(open: &str, close: &str, andop: &str, qorop: &str) -> Vec<String> {
    // NUL + whitespace class for the leading skip pattern (sh:54).
    let skip = "/*\u{0}[ \t\n]#/".to_string();
    let s = |x: &str| x.to_string();
    // The `${…}` in the ACTION elements is deliberate: zregexparse eval's each
    // action at completion time, when `$open`/`$close`/`$pre`/`$query` are the
    // live parameters set above.
    vec![
        s("("), skip, s(")"),
        s("("),
        s("("), format!("/{open}!/"), s("-optype[++nest]=1;pre=\"\""),   // sh:56
        s("|"), format!("/{open}{qorop}/"), s("-optype[++nest]=2;pre=\"\""), // sh:57
        s("|"), format!("/{open}{andop}/"), s("-optype[++nest]=3;pre=\"\""), // sh:58
        s("|"), s("/[]/"),                                               // sh:59
        s(r#":operators:operator:compadd -F "( ${(q)excl[optype[nest]]} )" -d disp -P ${pre:-${(Q)open}} -S ${(Q)open} \| \& \!"#),
        s(")"),
        s("|"),
        s("("), format!(r"/{open}[^\)]##/"), format!("%{close}%"),       // sh:61
        s("|"), format!("/{open}(#i)homeDirectory=/"), s("/[]/"),        // sh:62
        s(r#":directories:directory:_directories -P / -W / -r ") \t\n\-""#),
        s("|"), format!("/{open}(#i)loginShell=/"), s("/[]/"),           // sh:63
        s(r#":shells:shell:compadd -S ${(Q)close} ${(f)^"$(</etc/shells)"}(N)"#),
        s("|"), format!("/{open}(#i)mail=/"), s("/[]/"),                 // sh:64
        s(":email-addresses:mail:_email_addresses -S ${(Q)close}"),
        s("|"), format!("/{open}(#i)objectClass=/"), s("/[]/"),          // sh:65
        s(r#":object-classes:class:compadd -S ${(Q)close} -M "m:{a-zA-Z}={A-Za-z} r:[^A-Z]||[A-Z]=* r:|=*" -a classes"#),
        s("|"), format!("/{open}(#i)(automountKey|(member|)uid)=/"), s("/[]/"), // sh:66
        s(":users:username:_users -S ${(Q)close}"),
        s("|"), format!("/{open}(#i)cn=/"), s("/[]/"),                   // sh:67
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
        s(r"/[^\\)]##/"), format!("%{close}%"),                          // sh:77
        s(r#": _message -e object-values "object value (* for presence check)""#),
        s(")"),
        format!("/{close}/"), s("-(( nest ))"),                          // sh:79
        s(r#":brackets:bracket:compadd ${=query[nest]:+-S ""} \)"#),
        s("("),
        format!("/{close}/"),                                            // sh:83
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
        let q = build_query(r"\\\(", r"\\\)", r"\&", r"\\\|");
        let opens = q.iter().filter(|e| *e == "(").count();
        let closes = q.iter().filter(|e| *e == ")").count();
        assert_eq!(opens, closes, "query groups must balance");
    }

    /// The whole point of parameterising the spec: inside double quotes
    /// `compquote` adds no backslash, so every bracket pattern loses one
    /// escape level. Measured in real zsh on the second `ldapsearch <TAB>`
    /// (`open`=`\(`, `close`=`\)`, `andop`=`&`, `${(q)orop}`=`\|`); with the
    /// unquoted-context text baked in, these patterns demand a literal
    /// backslash that the word `"(` does not contain and nothing matches.
    #[test]
    fn quoted_context_spec_drops_an_escape_level() {
        let unq = build_query(r"\\\(", r"\\\)", r"\&", r"\\\|");
        let dq = build_query(r"\(", r"\)", "&", r"\|");
        assert!(unq.contains(&r"/\\\(!/".to_string())); // sh:56
        assert!(dq.contains(&r"/\(!/".to_string()));
        assert!(unq.contains(&r"/\\\(\&/".to_string())); // sh:58
        assert!(dq.contains(&r"/\(&/".to_string()));
        assert!(unq.contains(&r"/\\\(\\\|/".to_string())); // sh:57
        assert!(dq.contains(&r"/\(\|/".to_string()));
        assert!(unq.contains(&r"/\\\)/".to_string())); // sh:79
        assert!(dq.contains(&r"/\)/".to_string()));
        assert_eq!(unq.len(), dq.len());
    }
}
