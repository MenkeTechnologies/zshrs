//! Port of `_bpf_filters` from `Completion/Unix/Type/_bpf_filters`.
//!
//! `_bpf_filters` completes tcpdump/pcap filter expressions. Upstream (217
//! lines) computes OS-dependent word tables (`bpf_tables`), then registers a
//! large `_regex_arguments _bpf …` completion state machine and runs it:
//! ```text
//! sh:  6  networks=( … )   subtypes=( mgt … )   flags=( len … tcp … icmp … )
//! sh: 33  case $OSTYPE in solaris*|*bsd*) fields/protos/dirs/relop … esac
//! sh: 63  compquote suf
//! sh: 70  _regex_arguments _bpf /$'[^\0]#\0'/ \( … full spec … \) \#
//! sh:217  _bpf "$@"
//! ```
//!
//! The full `_regex_arguments` spec (sh:70-216) is reproduced word-for-word in
//! `build_bpf_spec` — every alternation, guard (`-'code'`) and
//! `:tag:desc:action` from the shell, with the OS tables (`$fields`/`$protos`/
//! `$dirs`/`$relop`), the `$WORD` pattern and the `$networks` sub-spec
//! interpolated at build time, and the eval-time refs (`$suf`/`$values`/`$dir`/
//! `$wlantype`/`$packet`/`$proto`/`$repeat`/`$flags`/`$subtypes`) left verbatim
//! for the regex engine. Shell `$'…'` NUL bytes map to `\u{0}`; `\(`/`\)`/`\|`/
//! `\#` grouping tokens become the words `(`/`)`/`|`/`#`.

use crate::compsys::ported::_regex_arguments::regex_arguments_byname;
use crate::compsys::ported::_regex_arguments::{_regex_arguments, dispatch_registered};
use crate::ported::params::{getsparam, setaparam};

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
    // sh:43-44 — (free|open)bsd* pf(4) fields (REPLACES fields).
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
    t
}

/// Build the full `_regex_arguments _bpf …` spec (sh:70-216) as an argv, first
/// element `_bpf` (the generated matcher name).
fn build_bpf_spec(t: &BpfTables) -> Vec<String> {
    let nul = "\u{0}";
    // sh:16  local WORD=$'[^ \0]##[ \0]##'
    let word = format!("/[^ {n}]##[ {n}]##/", n = nul);
    // `[ \0]` char class (space or NUL) used throughout the `$'…'` patterns.
    let sp = format!("[ {n}]", n = nul);
    let protos = t.protos.join(" ");
    let relop = t.relop.join(" ");
    let fields = if t.fields.is_empty() {
        String::new()
    } else {
        format!(" {}", t.fields.join(" "))
    };
    let dirs = if t.dirs.is_empty() {
        String::new()
    } else {
        format!(" {}", t.dirs.join(" "))
    };

    // sh:11-25  $networks sub-spec, spliced wherever `$networks` appears.
    let networks: Vec<String> = vec![
        format!("/[^/ {n}]#/", n = nul),
        "(".into(),
        format!("/{sp}/"),
        ": _message -e networks network".into(),
        format!("/mask{sp}/"),
        ":masks:mask:(mask)".into(),
        word.clone(),
        ":netmasks:netmask:".into(),
        "|".into(),
        "///".into(),
        word.clone(),
        ": _message -e masks \"netmask length (bits)\"".into(),
        ")".into(),
    ];

    let mut s: Vec<String> = Vec::new();
    // A macro (not a closure) so `s.extend(networks)` can borrow `s` between
    // pushes — a closure capturing `&mut s` would hold the borrow open.
    macro_rules! p {
        ($w:expr) => {
            s.push(String::from($w))
        };
    }

    // ── sh:70 top ────────────────────────────────────────────────────────
    p!("_bpf");
    p!(&format!("/[^{n}]#{n}/", n = nul));
    p!("("); // group A (top-level repeatable filter term)
             // sh:71  leading not / open-paren
    p!(&format!("/(not{sp}#|!{sp}#|(\\\\|)\\({sp}#)/"));
    p!(":operators:operator:(not \\()");
    p!("#");
    p!("("); // group B (a term: expression | qualifier chain)
    p!("("); // group C (an arithmetic/offset expression, starred)
    p!("("); // group D (the expression head)
             // sh:76
    p!(&format!(
        "/(0x[0-9a-f]##|[0-9]##|${{(j.|.)${{=flags}}}}){sp}#/"
    ));
    p!("-((repeat != 2))");
    p!(":expressions:expression:compadd ${=flags[$packet]}");
    p!("|");
    p!(&format!("/[a-z]##(\\\\|)\\[[^\\]]##(\\\\|)\\]{sp}#/"));
    p!("|");
    p!("/[a-z]##(\\\\|)\\[[^:\\]]##:/");
    p!("/[]/");
    p!(":sizes:field size (bytes):compadd -S \"$suf\" 1 2 4");
    p!("|");
    p!("/tcp(\\\\|)\\[/");
    p!("-packet=tcp");
    p!("/[]/");
    p!(":offsets:header offset:compadd -S \"$suf \" tcpflags");
    p!("|");
    p!("/icmp(\\\\|)\\[/");
    p!("-packet=icmp");
    p!("/[]/");
    p!(":offsets:header offset:compadd -S \"$suf \" icmptype icmpcode");
    p!("|");
    p!("/[a-z]##(\\\\|)\\[/");
    p!("/[]/");
    p!(":offsets:offset:");
    p!(")"); // close D
    p!("("); // group E (an operator after the expression head)
    p!(&format!("/(\\\\|)([<>=!](\\\\|)[<>=]|[<>&|=+*/%^-]){sp}#/"));
    p!("-repeat=0");
    p!(&format!(
        ":operators:operator:(+ - = != < > <= >= & | {relop} and or)"
    ));
    p!("//");
    p!(": _message -e expressions expression");
    p!("|");
    p!("//");
    p!("-repeat=2");
    p!(")"); // close E
    p!(")"); // close C
    p!("#");
    p!("//");
    p!("-(( repeat == 2))");
    p!("//");
    p!("-repeat=1");
    p!("|"); // B-alt
             // sh:104  ether proto
    p!(&format!("/ether{sp}proto{sp}/"));
    p!(&word);
    p!(":protocols:protocol:(\\ip \\ip6 \\arp \\rarp \\atalk \\aarp \\dec \\net \\sca \\lat \\mopdl \\moprc \\iso \\stp \\ipx \\netbeui)");
    p!("|"); // B-alt
             // sh:107  less/greater
    p!(&format!("/(less|greater){sp}/"));
    p!(":fields:field:(less greater)");
    p!(&word);
    p!(":numbers:length (bytes):");
    p!("|"); // B-alt
             // sh:111  (F)(G): protocol qualifier group, then the value group
    p!("("); // group F
    p!(&format!(
        "/(tcp|udp|icmp|ether|ip|ip6|arp|rarp|decnet|bootp|dhcp|dhcp6|apple|pppoe|pppoed|ldap|ah|esp|slp|sctp|ospf|iso|clnp|esis|isis|atalk|aarp|iso|stp|ipx|netbeui|lat|moprc|mopdl){sp}/"
    ));
    p!(&format!(
        ":protocols:protocol qualifier:(tcp udp icmp ether tr ip ip6 arp rarp decnet {protos})"
    ));
    p!("|");
    p!(&format!("/((fddi|tr|wlan){sp}|)/"));
    p!("-(( ++proto ))");
    p!(")"); // close F
    p!("("); // group G (the qualifier/value alternatives)
             // sh:118  mpls
    p!(&format!("/mpls{sp}/"));
    p!(&format!("/((0x|)[0-9a-f]##{sp}|)/"));
    p!(": _message -e labels \"label number\"");
    p!("|");
    // sh:121  geneve
    p!(&format!("/geneve{sp}/"));
    p!(&format!("/((0x|)[0-9a-f]##{sp}|)/"));
    p!(": _message -e vnis \"vni\"");
    p!("|");
    // sh:124  pppoes
    p!(&format!("/pppoes{sp}/"));
    p!(&format!("/((0x|)[0-9a-f]##{sp}|)/"));
    p!(": _message -e session-ids \"session id\"");
    p!("|");
    // sh:127  proto
    p!(&format!("/proto{sp}/"));
    p!(":fields:field:(proto)");
    p!(&word);
    p!(":protocols:protocol:(\\icmp \\icmp6 \\igmp \\igrp \\pim \\ah \\esp \\vrrp \\udp \\tcp)");
    p!("|");
    // sh:130  broadcast/multicast
    p!(&format!("/(broad|multi)cast{sp}/"));
    p!(":fields:field:(broadcast multicast)");
    p!("|");
    // sh:132  type / subtype
    p!(&format!("/type{sp}/"));
    p!(":fields:field:(type)");
    p!(&word);
    p!("-wlantype=${match%?}");
    p!(":wlan-types:wlan type:(mgt ctl data)");
    p!("("); // subtype sub-group
    p!(&format!("/subtype{sp}/"));
    p!(":fields:field:(subtype)");
    p!(&word);
    p!(":subtypes:subtype:compadd ${=subtypes[$wlantype]:-$subtypes}");
    p!("|");
    p!(")"); // close subtype sub-group
    p!("|");
    // sh:141  protochain
    p!(&format!("/protochain{sp}/"));
    p!(":fields:field:(protochain)");
    p!(&word);
    p!(":protocols:protocol:");
    p!("|");
    // sh:145  vlan-id
    p!(&format!("/vlan-id{sp}/"));
    p!(&word);
    p!(":vlans:vlan:");
    p!("|");
    // sh:148  vlan
    p!(&format!("/vlan{sp}/"));
    p!(":fields:field:(vlan)");
    p!("("); // vlan sub-group
    p!(&word);
    p!(": _message -e vlans vlan");
    p!("|");
    p!(")"); // close vlan sub-group
    p!("|");
    p!("("); // group H (direction qualifiers)
             // sh:154
    p!(&format!("/(ra|ta|addr[1-4]|inbound|outbound){sp}/"));
    p!(&format!(
        ":directions:direction qualifier:(src dst inbound outbound ra ta addr1 addr2 addr3 addr4{dirs})"
    ));
    p!("|");
    // sh:157  src/dst
    p!(&format!("/(src|from|dst|to){sp}/"));
    p!("-values=${values:-hosts};dir=$match");
    p!("("); // src/dst sub-group
    p!(&format!("/(or|and){sp}/"));
    p!(":operators:operator:(or and)");
    p!(&format!("/(src|dst){sp}/"));
    p!(":directions:direction qualifier:compadd ${${${${dir%?}:/dst/to}:/(src|from)/dst}:/to/src}");
    p!("|");
    p!(")"); // close src/dst sub-group
    p!("|");
    p!(")"); // close H (trailing empty alt)
    p!("("); // group I (the value alternatives)
             // sh:167  host/gateway
    p!(&format!("/(host|gateway){sp}/"));
    p!(&format!(":fields:field:(host gateway{fields})"));
    p!(&word);
    p!("-values=hosts");
    p!(":hosts:host:_hosts");
    p!("|");
    // sh:171  inet host
    p!(&format!("/inet(6|){sp}/"));
    p!("("); // inet host sub-group
    p!(&format!("/host{sp}/"));
    p!(":fields:field:(host)");
    p!("|");
    p!(")"); // close inet host sub-group
    p!(&word);
    p!("-values=hosts");
    p!(":hosts:host:_hosts");
    p!("|");
    // sh:178  ethertype
    p!(&format!("/ethertype{sp}/"));
    p!(&word);
    p!(":numbers:number:");
    p!("|");
    // sh:181  ipaddr/etheraddr/atalkaddr
    p!(&format!("/(ipaddr|etheraddr|atalkaddr){sp}/"));
    p!(&word);
    p!(":addresses:address:");
    p!("|");
    // sh:184  llc
    p!(&format!("/llc{sp}/"));
    p!(&format!(
        "/((s|u|rr|rnr|rej|ui|ua|disc|sabme|test|xid|frmr){sp}|)/"
    ));
    p!(":types:type:(s u rr rnr rej ui ua disc sabme test xid frmr)");
    p!("|");
    // sh:187  ifname/on
    p!(&format!("/(ifname|on){sp}/"));
    p!(&word);
    p!(":interfaces:interface:_net_interfaces");
    p!("|");
    // sh:190  rnr/rulenum/srnr/subruleset
    p!(&format!("/(rnr|rulenum|srnr|subruleset){sp}/"));
    p!(&word);
    p!(":rules:rule number:");
    p!("|");
    // sh:193  reason
    p!(&format!("/reason{sp}/"));
    p!(&word);
    p!(":reasons:reason:(match bad-offset fragment short normalize memory)");
    p!("|");
    // sh:196  rset/ruleset
    p!(&format!("/(rset|ruleset){sp}/"));
    p!(&word);
    p!(":rule-sets:rule set:");
    p!("|");
    // sh:199  action
    p!(&format!("/action{sp}/"));
    p!(&word);
    p!(":actions:action:(pass block nat rdr binat scrub)");
    p!("|");
    // sh:202  rpc
    p!(&format!("/rpc{sp}/"));
    p!("("); // rpc sub-group
    p!(&format!("/[^, {n}]##{sp}/", n = nul));
    p!(":programs:rpc program:compadd -qS, - ${${(f)\"$(</etc/rpc)\"}%%[[:blank:]]#*}");
    p!("|");
    p!(&format!("/[^, {n}]##,[^, {n}]##,/", n = nul));
    p!(&format!("/[^, {n}]##{sp}/", n = nul));
    p!(":procedures:procedure:");
    p!("|");
    p!(&format!("/[^, {n}]##,/", n = nul));
    p!(&format!("/[^, {n}]##{sp}/", n = nul));
    p!(":versions:version:");
    p!(")"); // close rpc sub-group
    p!("|");
    // sh:212  zone
    p!(&format!("/zone{sp}/"));
    p!(&word);
    p!(":zones:zone:_zones");
    p!("|");
    // sh:215  port
    p!(&format!("/port{sp}/"));
    p!(":fields:field:(port)");
    p!(&word);
    p!("-values=ports");
    p!(":ports:port:_ports");
    p!("|");
    // sh:219  portrange
    p!(&format!("/portrange{sp}/"));
    p!("-values=portranges");
    p!(":fields:field:(portrange)");
    p!(&format!("/[^ {n}-]##-/", n = nul));
    p!(":ports:port:_ports -S-");
    p!(&word);
    p!(":ports:port:_ports");
    p!("|");
    // sh:224  net
    p!(&format!("/net{sp}/"));
    p!("-values=networks");
    p!(":fields:field:(net)");
    s.extend(networks.clone());
    p!("|");
    // sh:227  values=hosts fallback
    p!("//");
    p!("-[[ $values = hosts ]]");
    p!(&word);
    p!(":hosts:host:_hosts");
    p!("|");
    // sh:230  values=ports fallback
    p!("//");
    p!("-[[ $values = ports ]]");
    p!(&word);
    p!(":ports:port:_ports");
    p!("|");
    // sh:233  values=networks fallback
    p!("//");
    p!("-[[ $values = networks ]]");
    s.extend(networks.clone());
    p!("|");
    // sh:236  values=portranges fallback
    p!("//");
    p!("-[[ $values = portranges ]]");
    p!(&format!("/[^ {n}-]##-/", n = nul));
    p!(":ports:port:_ports -S-");
    p!(&word);
    p!(":ports:port:_ports");
    p!("|");
    // sh:240  final proto fallback
    p!("//");
    p!("-(( ++proto ))");
    p!(")"); // close I
    p!(")"); // close G
    p!(")"); // close B
             // sh:212-216 epilogue
    p!("//");
    p!("-(( proto < 2 ))");
    p!(&format!("/(and|or|&&|\\\\|\\\\||\\\\)){sp}/"));
    p!("-proto=0");
    p!(":operators:operator:compadd and or \\)");
    p!(")"); // close A
    p!("#");

    s
}

/// `_bpf_filters` — complete a pcap/tcpdump filter expression.
pub fn _bpf_filters(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_bpf_filters");
    let ostype = getsparam("OSTYPE").unwrap_or_default();
    let tables = bpf_tables(&ostype);

    // sh:70-216 — register the `_bpf` matcher with the full spec.
    let spec = build_bpf_spec(&tables);
    let _ = regex_arguments_byname(&spec);

    // sh:217 — `_bpf "$@"`.
    setaparam("_bpf_argv", args.to_vec());
    dispatch_registered("_bpf")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solaris_tables() {
        let t = bpf_tables("solaris2.11");
        assert!(t.fields.contains(&"ipaddr".to_string()));
        assert!(t.fields.contains(&"zone".to_string()));
        assert_eq!(t.relop, vec!["^".to_string(), "%".to_string()]);
        assert!(!t.protos.contains(&"fddi".to_string()));
    }

    #[test]
    fn openbsd_tables() {
        let t = bpf_tables("openbsd6.9");
        assert!(t.fields.contains(&"action".to_string()));
        assert!(!t.protos.contains(&"ah".to_string()));
        assert!(t.protos.contains(&"fddi".to_string()));
    }

    #[test]
    fn linux_tables_are_minimal() {
        let t = bpf_tables("linux-gnu");
        assert!(t.fields.is_empty());
        assert!(t.protos.contains(&"mpls".to_string()));
        assert!(t.protos.contains(&"ah".to_string()));
        assert_eq!(t.relop, vec![">>".to_string(), "<<".to_string()]);
    }

    #[test]
    fn spec_is_balanced_and_complete() {
        // The generated spec starts with `_bpf`, balances its `(`/`)` grouping,
        // and carries the key completion actions + OS-table interpolation.
        let t = bpf_tables("linux-gnu");
        let s = build_bpf_spec(&t);
        assert_eq!(s[0], "_bpf");
        let opens = s.iter().filter(|w| *w == "(").count();
        let closes = s.iter().filter(|w| *w == ")").count();
        assert_eq!(
            opens, closes,
            "unbalanced grouping (opens {opens} != closes {closes})"
        );
        assert!(s.iter().any(|w| w.contains(":hosts:host:_hosts")));
        assert!(s.iter().any(|w| w.contains(":ports:port:_ports")));
        assert!(s.iter().any(|w| w.contains(":actions:action:")));
        assert!(s
            .iter()
            .any(|w| w.contains(":interfaces:interface:_net_interfaces")));
        assert!(s
            .iter()
            .any(|w| w.contains("protocol qualifier:") && w.contains("mpls")));
    }

    #[test]
    fn returns_without_panic_outside_context() {
        let _g = crate::test_util::global_state_lock();
        let _ = _bpf_filters(&[]);
    }
}
