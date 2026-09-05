//! Port of `_bpf_filters` from `Completion/Unix/Type/_bpf_filters`.
//!
//! `_bpf_filters` completes tcpdump/pcap filter expressions. Upstream (217
//! lines) computes OS-dependent word tables (`bpf_tables`), then registers a
//! large `_regex_arguments _bpf …` completion state machine and runs it:
//! ```text
//! sh:  6  networks=( … )   subtypes=( mgt … )   flags=( len … tcp … icmp … )
//! sh:  8  local suf=']'
//! sh: 33  case $OSTYPE in solaris*|*bsd*) fields/protos/dirs/relop … esac
//! sh: 58  compquote suf
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

use crate::compsys::ported::_regex_arguments::{_regex_arguments, dispatch_registered};
use crate::compsys::ported::shared::LocalScope;
use crate::ported::params::{getsparam, setaparam, sethparam, setsparam};
use crate::ported::zle::computil::bin_compquote;
use crate::ported::zsh_h::{options, MAX_OPS};

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
    // sh:10  local WORD=$'[^ \0]##[ \0]##'
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

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:8 + sh:58 — `local suf=']'` followed by `compquote suf`.
///
/// Three of the spec's action strings interpolate `$suf`
/// (`compadd -S "$suf" 1 2 4`, `compadd -S "$suf " tcpflags`, and the icmp
/// twin) and they are eval'd BY NAME while `_bpf_filters`' frame is still
/// live, so `suf` has to exist as a real parameter.
///
/// compquote quotes for the CURRENT quoting context — `comp_quote` quotes with
/// `*compqstack` (`Src/Zle/computil.c:3691-3705`) — so the bracket cannot be
/// baked into the spec. Measured in real zsh, `tcpdump tcp\[<TAB>` completes
/// an unquoted word and inserts `tcpflags\] `, while `tcpdump 'tcp[<TAB>` is
/// already inside single quotes and inserts `tcpflags] `. With `suf` never set
/// at all the port inserted a bare `tcpflags ` in both.
fn publish_suffix() {
    setsparam("suf", "]");
    let _ = bin_compquote("compquote", &["suf".to_string()], &make_ops(), 0);
}

/// `_bpf_filters` — complete a pcap/tcpdump filter expression.
pub fn _bpf_filters(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_bpf_filters");

    // sh:8 — `local suf=']'`, gone again when `_bpf_filters` returns.
    let mut _locals = LocalScope::declare(&["suf"], 0);

    let ostype = getsparam("OSTYPE").unwrap_or_default();
    let tables = bpf_tables(&ostype);

    // sh:22-31 — `subtypes` and `flags`, the two associative arrays declared
    // at sh:6 `local -A subtypes flags`.
    //
    // The spec below references them BY NAME and they are eval'd while this
    // frame is live: `${(j.|.)${=flags}}` (rs), `${=flags[$packet]}` and
    // `${=subtypes[$wlantype]:-$subtypes}`. The port emitted all three
    // verbatim but never DEFINED either parameter, so the subscript resolved
    // against an unset name and reached `mathevalarg`
    // (`src/ported/math.rs`, c:Src/math.c:1525-1533) with an empty string:
    //
    //     tcpdump <TAB>   zsh: the filter keywords
    //                     was: `bad math expression: empty string`
    //
    // Both shells produce that identical error given an unset `flags`, and
    // neither does with `local -A flags` — so this is a port omission, not a
    // core divergence in the arithmetic evaluator.
    sethparam(
        "subtypes",
        vec![
            "mgt".into(),
            "assoc-req assoc-resp reassoc-req reassoc-resp probe-req probe-resp              beacon atim disassoc auth deauth"
                .into(),
            "ctl".into(),
            "ps-poll rts cts ack cf-end cf-end-ack".into(),
            "data".into(),
            "data data-cf-ack data-cf-poll data-cf-ack-poll null cf-ack cf-poll              cf-ack-poll qos-data qos-data-cf-ack qos-data-cf-poll              qos-data-cf-ack-poll qos qos-cf-poll and qos-cf-ack-poll"
                .into(),
        ],
    );
    sethparam(
        "flags",
        vec![
            "len".into(),
            "len".into(),
            "tcp".into(),
            "tcp-fin tcp-syn tcp-rst tcp-push tcp-ack tcp-urg".into(),
            "icmp".into(),
            "icmp-echoreply icmp-unreach icmp-sourcequench icmp-redirect              icmp-echo icmp-routeradvert icmp-routersolicit icmp-timxceed              icmp-paramprob icmp-tstamp icmp-tstampreply icmp-ireq              icmp-ireqreply icmp-maskreq icmp-maskreply"
                .into(),
        ],
    );

    // sh:8 + sh:58 — see `publish_suffix`.
    publish_suffix();
    tracing::debug!(
        target: "compsys::_bpf_filters",
        suf = %getsparam("suf").unwrap_or_default(),
        quote = %crate::ported::zle::compcore::get_compstate_str("quote")
            .unwrap_or_default(),
        "sh:58 bracket suffix for this quoting context",
    );

    // sh:70-216 — register the `_bpf` matcher with the full spec. The spec's
    // `$suf` references are left verbatim: the regex engine expands them at
    // action time, so they pick up whatever compquote just produced.
    let spec = build_bpf_spec(&tables);
    let _ = _regex_arguments(&spec);

    // sh:217 — `_bpf "$@"`.
    setaparam("_bpf_argv", args.to_vec());
    let ret = dispatch_registered("_bpf");
    drop(_locals);
    ret
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

    /// sh:8 + sh:58 — the bracket the three `compadd -S "$suf"` actions read
    /// has to be PUBLISHED and then quoted for the live context.
    ///
    /// Oracle, measured under a pty against real zsh with the stock
    /// `Completion/` tree on `$fpath`:
    ///
    /// ```text
    ///   tcpdump tcp\[<TAB>    ->  tcpdump tcp\[tcpflags\]       (suf = \])
    ///   tcpdump 'tcp[<TAB>    ->  tcpdump 'tcp[tcpflags]        (suf = ])
    /// ```
    ///
    /// The port used to reference sh:58 in a header comment and never call it,
    /// and never set `suf` either, so both contexts inserted `tcpflags `.
    #[test]
    fn publish_suffix_quotes_the_bracket_for_the_live_context() {
        use crate::ported::params::{getsparam, unsetparam};
        use crate::ported::zle::complete::{COMPQSTACK, INCOMPFUNC};
        use crate::ported::zsh_h::QT_BACKSLASH;
        use std::sync::atomic::Ordering;

        let _g = crate::test_util::global_state_lock();
        let saved = INCOMPFUNC.load(Ordering::Relaxed);
        INCOMPFUNC.store(1, Ordering::Relaxed);

        // Unquoted word: `Src/Zle/complete.c` pushes QT_BACKSLASH for it, and
        // `]` comes back backslash-quoted.
        if let Ok(mut q) = COMPQSTACK
            .get_or_init(|| std::sync::Mutex::new(String::new()))
            .lock()
        {
            *q = (QT_BACKSLASH as u8 as char).to_string();
        }
        publish_suffix();
        let unquoted = getsparam("suf");

        // Inside quotes there is nothing on the stack to quote FOR, and
        // `bin_compquote` short-circuits at c:3691 leaving `]` alone.
        if let Ok(mut q) = COMPQSTACK.get().unwrap().lock() {
            q.clear();
        }
        publish_suffix();
        let quoted = getsparam("suf");

        INCOMPFUNC.store(saved, Ordering::Relaxed);
        unsetparam("suf");

        assert_eq!(unquoted.as_deref(), Some("\\]"), "unquoted word");
        assert_eq!(quoted.as_deref(), Some("]"), "already inside quotes");
    }
}
