//! Port of `Src/Zle/compctl.c` — the legacy `compctl` builtin and its
//! supporting completion machinery (predates compsys).
//!
//! 4076 lines / 47 fns. This file ports the type definitions, constants,
//! and simpler free fns first; large fns (`makecomplist*`, `bin_compctl`,
//! `printcompctl`) are stubbed with C source-line citations and ported
//! incrementally.
//!
//! Citations: every fn comment references `Src/Zle/compctl.c:<line>` so
//! drift can be checked against the upstream snapshot.

#![allow(dead_code)]
#![allow(clippy::too_many_arguments)]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

// =================================================================
// Type definitions — port of Src/Zle/compctl.h:32-115
// =================================================================

/// Mask of completion targets — port of `CC_*` constants from
/// Src/Zle/compctl.h:117-149. Each bit selects one completion-source
/// kind (files, vars, jobs, etc.) the compctl spec expands.
pub mod cc_flags {
    pub const FILES: u64       = 1 << 0;   // compctl.h:118
    pub const COMMPATH: u64    = 1 << 1;   // compctl.h:119
    pub const REMOVE: u64      = 1 << 2;   // compctl.h:120
    pub const OPTIONS: u64     = 1 << 3;   // compctl.h:121
    pub const VARS: u64        = 1 << 4;   // compctl.h:122
    pub const BINDINGS: u64    = 1 << 5;   // compctl.h:123
    pub const ARRAYS: u64      = 1 << 6;   // compctl.h:124
    pub const INTVARS: u64     = 1 << 7;   // compctl.h:125
    pub const SHFUNCS: u64     = 1 << 8;   // compctl.h:126
    pub const PARAMS: u64      = 1 << 9;   // compctl.h:127
    pub const ENVVARS: u64     = 1 << 10;  // compctl.h:128
    pub const JOBS: u64        = 1 << 11;  // compctl.h:129
    pub const RUNNING: u64     = 1 << 12;  // compctl.h:130
    pub const STOPPED: u64     = 1 << 13;  // compctl.h:131
    pub const BUILTINS: u64    = 1 << 14;  // compctl.h:132
    pub const ALREG: u64       = 1 << 15;  // compctl.h:133
    pub const ALGLOB: u64      = 1 << 16;  // compctl.h:134
    pub const USERS: u64       = 1 << 17;  // compctl.h:135
    pub const DISCMDS: u64     = 1 << 18;  // compctl.h:136
    pub const EXCMDS: u64      = 1 << 19;  // compctl.h:137
    pub const SCALARS: u64     = 1 << 20;  // compctl.h:138
    pub const READONLYS: u64   = 1 << 21;  // compctl.h:139
    pub const SPECIALS: u64    = 1 << 22;  // compctl.h:140
    pub const DELETE: u64      = 1 << 23;  // compctl.h:141
    pub const NAMED: u64       = 1 << 24;  // compctl.h:142
    pub const QUOTEFLAG: u64   = 1 << 25;  // compctl.h:143
    pub const EXTCMDS: u64     = 1 << 26;  // compctl.h:144
    pub const RESWDS: u64      = 1 << 27;  // compctl.h:145
    pub const DIRS: u64        = 1 << 28;  // compctl.h:146
    pub const EXPANDEXPL: u64  = 1 << 30;  // compctl.h:148
    pub const RESERVED: u64    = 1 << 31;  // compctl.h:149
}

/// `mask2` (secondary completion-target flags) — port of the second
/// `CC_*` block in Src/Zle/compctl.h:151-158.
pub mod cc_flags2 {
    pub const NOSORT: u64   = 1 << 0;  // compctl.h:152
    pub const XORCONT: u64  = 1 << 1;  // compctl.h:153
    pub const CCCONT: u64   = 1 << 2;  // compctl.h:154
    pub const PATCONT: u64  = 1 << 3;  // compctl.h:155
    pub const DEFCONT: u64  = 1 << 4;  // compctl.h:156
    pub const UNIQCON: u64  = 1 << 5;  // compctl.h:157
    pub const UNIQALL: u64  = 1 << 6;  // compctl.h:158
}

/// `-x` condition types — port of `CCT_*` constants from
/// Src/Zle/compctl.h:76-89.
pub mod cct {
    pub const UNUSED: i32   = 0;   // compctl.h:76
    pub const POS: i32      = 1;   // compctl.h:77
    pub const CURSTR: i32   = 2;   // compctl.h:78
    pub const CURPAT: i32   = 3;   // compctl.h:79
    pub const WORDSTR: i32  = 4;   // compctl.h:80
    pub const WORDPAT: i32  = 5;   // compctl.h:81
    pub const CURSUF: i32   = 6;   // compctl.h:82
    pub const CURPRE: i32   = 7;   // compctl.h:83
    pub const CURSUB: i32   = 8;   // compctl.h:84
    pub const CURSUBC: i32  = 9;   // compctl.h:85
    pub const NUMWORDS: i32 = 10;  // compctl.h:86
    pub const RANGESTR: i32 = 11;  // compctl.h:87
    pub const RANGEPAT: i32 = 12;  // compctl.h:88
    pub const QUOTE: i32    = 13;  // compctl.h:89
}

/// Internal `cclist` flags — port of `COMP_*` from
/// Src/Zle/compctl.c:53-58.
pub mod comp_op {
    pub const LIST: u32      = 1 << 0;  // compctl.c:53 (-L)
    pub const COMMAND: u32   = 1 << 1;  // compctl.c:54 (-C)
    pub const DEFAULT: u32   = 1 << 2;  // compctl.c:55 (-D)
    pub const FIRST: u32     = 1 << 3;  // compctl.c:56 (-T)
    pub const REMOVE: u32    = 1 << 4;  // compctl.c:57
    pub const LISTMATCH: u32 = 1 << 5;  // compctl.c:58 (-L and -M)
    /// Composite — any of the special-target flags.
    pub const SPECIAL: u32 = COMMAND | DEFAULT | FIRST;  // compctl.c:60
}

/// `-x` per-condition descriptor.
/// Port of `struct compcond` from Src/Zle/compctl.h:54-74.
#[derive(Debug, Clone, Default)]
pub struct CompCond {
    /// Next condition AND'd with this one (`,`).
    pub and: Option<Box<CompCond>>,
    /// Next condition OR'd with this one (`|`).
    pub or: Option<Box<CompCond>>,
    /// Condition type — one of `cct::*`.
    pub typ: i32,
    /// Array length (unioned data uses this for bounds).
    pub n: i32,
    /// Per-type union — split into Rust enum variants below.
    pub data: CompCondData,
}

/// Per-type data for `CompCond`. Direct port of the `union { struct r,s,l }`
/// in Src/Zle/compctl.h:58-73 — the C union is dispatched by `typ`.
#[derive(Debug, Clone, Default)]
pub enum CompCondData {
    /// `CCT_POS` / `CCT_NUMWORKS` — numeric range pair.
    Range { a: Vec<i32>, b: Vec<i32> },
    /// `CCT_CURSTR`/`CCT_CURPAT`/`CCT_CURSUF`/`CCT_CURPRE`/etc. —
    /// position-indexed string list.
    Strings { p: Vec<i32>, s: Vec<String> },
    /// `CCT_RANGESTR`/`CCT_RANGEPAT` — paired string lists for range tests.
    StringRange { a: Vec<String>, b: Vec<String> },
    /// Empty (CCT_UNUSED).
    #[default]
    Empty,
}

/// Per-command compctl spec.
/// Port of `struct compctl` from Src/Zle/compctl.h:93-115.
#[derive(Debug, Default)]
pub struct CompCtl {
    /// Reference count — C uses C-style refcounting; Rust uses `Arc`.
    /// Kept as a field for parity-with-C debug visibility.
    pub refc: i32,
    /// Next compctl in `-x` chain.
    pub next: Option<Arc<CompCtl>>,
    /// Mask of `cc_flags::*` — primary completion targets.
    pub mask: u64,
    /// Secondary mask (extension flags).
    pub mask2: u64,
    /// `-k VAR` — variable name to read completions from.
    pub keyvar: Option<String>,
    /// `-g GLOB` — glob pattern.
    pub glob: Option<String>,
    /// `-s STR` — expansion string.
    pub str_expansion: Option<String>,
    /// `-K FUNC` — completion function.
    pub func: Option<String>,
    /// `-X EXPL` — explanation string.
    pub explain: Option<String>,
    /// `-y LIST` — user-defined description for listing.
    pub ylist: Option<String>,
    /// `-P` — prefix.
    pub prefix: Option<String>,
    /// `-S` — suffix.
    pub suffix: Option<String>,
    /// `-l` — subcommand name.
    pub subcmd: Option<String>,
    /// `-1` — substr name.
    pub substr: Option<String>,
    /// `-w` — with directory.
    pub withd: Option<String>,
    /// `-H` — history pattern.
    pub hpat: Option<String>,
    /// `-H` — number of history events to search.
    pub hnum: i32,
    /// `-J` / `-V` — group name.
    pub gname: Option<String>,
    /// `-x` — first compctl after the condition.
    pub ext: Option<Arc<CompCtl>>,
    /// `-x` — condition for this compctl.
    pub cond: Option<CompCond>,
    /// `+` — xor'd next compctl.
    pub xor: Option<Arc<CompCtl>>,
    /// `-M` — matcher control (Cmatcher in C).
    /// Type kept as Option<String> placeholder until cmatch port lands.
    pub matcher: Option<String>,
    /// `-M` — matcher string.
    pub mstr: Option<String>,
}

/// Per-pattern compctl entry.
/// Port of `struct patcomp` from Src/Zle/compctl.h:46-50 — linked list
/// of pattern→compctl mappings the lookup walks for `compctl -p PAT`.
#[derive(Debug)]
pub struct PatComp {
    pub next: Option<Box<PatComp>>,
    pub pat: String,
    pub cc: Arc<CompCtl>,
}

/// Hash-table node for `compctltab` — port of `struct compctlp` from
/// Src/Zle/compctl.h:39-42.
#[derive(Debug)]
pub struct CompCtlp {
    pub name: String,
    pub cc: Arc<CompCtl>,
}

// =================================================================
// Globals — port of Src/Zle/compctl.c:36-66
// =================================================================

/// Global cmatcher list. Port of file-static `Cmlist cmatcher;` at
/// Src/Zle/compctl.c:36. Type kept as Option<String> placeholder.
static CMATCHER: Mutex<Option<String>> = Mutex::new(None);

/// `compctltab` hash table — name → CompCtl.
/// Port of `HashTable compctltab;` at Src/Zle/compctl.c:46.
static COMPCTL_TAB: Mutex<Option<HashMap<String, Arc<CompCtl>>>> = Mutex::new(None);

/// Pattern-compctl list. Port of `Patcomp patcomps;` at
/// Src/Zle/compctl.c:51.
static PATCOMPS: Mutex<Vec<(String, Arc<CompCtl>)>> = Mutex::new(Vec::new());

/// `cclist` — flag for listing/command/default/first completion.
/// Port of file-static `int cclist;` at Src/Zle/compctl.c:63.
static CCLIST: Mutex<u32> = Mutex::new(0);

/// `showmask` — mask determining what to print.
/// Port of file-static `unsigned long showmask;` at Src/Zle/compctl.c:66.
static SHOWMASK: Mutex<u64> = Mutex::new(0);

// =================================================================
// Free fns — start of compctl.c proper
// =================================================================

/// Initialize the `compctltab` hash table.
/// Port of `createcompctltable()` from Src/Zle/compctl.c:69. The C
/// version wires hash function pointers (hasher, addnode, getnode,
/// printnode, freenode); Rust uses a plain HashMap so the wiring
/// reduces to allocation.
pub(crate) fn createcompctltable() {
    let mut g = COMPCTL_TAB.lock().unwrap();
    *g = Some(HashMap::new());
    let mut p = PATCOMPS.lock().unwrap();
    p.clear();
}

/// Free a `compctlp` hash node.
/// Port of `freecompctlp()` from Src/Zle/compctl.c:91. Rust's Arc
/// drop handles the inner CompCtl free; this is the entry the C
/// hash table calls back when removing a node.
pub(crate) fn freecompctlp(name: &str) {
    let mut g = COMPCTL_TAB.lock().unwrap();
    if let Some(map) = g.as_mut() {
        map.remove(name);
    }
}

/// Free a `compctl` spec.
/// Port of `freecompctl()` from Src/Zle/compctl.c:102. C uses
/// reference counting + manual `zsfree` of every string member +
/// recursive free of `ext`/`xor` chains. Rust's Arc handles this
/// automatically when the last reference drops.
pub(crate) fn freecompctl(_cc: Arc<CompCtl>) {
    // Arc::drop recursively frees the spec when refcount hits zero.
    // Direct port of compctl.c:104-141 — the C ladder of `zsfree(...)`
    // calls is the equivalent of letting the Arc/String values drop.
}

/// Free a `compcond` spec.
/// Port of `freecompcond()` from Src/Zle/compctl.c:145. C walks the
/// or/and chain, freeing per-type union data. Rust's enum + Box
/// drop the chain automatically; this is the entry kept for ABI
/// parity with the C source.
pub(crate) fn freecompcond(_cc: CompCond) {
    // Drop handles the chain — direct equivalent of compctl.c:148-186.
}

/// Copy a cmatcher list. Stub.
/// Port of `cpcmlist()` from Src/Zle/compctl.c:188. The C version
/// deep-copies a Cmlist linked list. Stubbed pending Cmlist port.
pub(crate) fn cpcmlist(_l: Option<String>) -> Option<String> {
    None
}

/// Set a global matcher. Stub.
/// Port of `set_gmatcher()` from Src/Zle/compctl.c:225 — `compctl -M`
/// global matcher install. Stubbed pending matcher-string parser.
pub(crate) fn set_gmatcher(_name: &str, _argv: &[String]) -> i32 {
    0
}

/// Get a global matcher. Stub.
/// Port of `get_gmatcher()` from Src/Zle/compctl.c:281.
pub(crate) fn get_gmatcher(_name: &str, _argv: &[String]) -> i32 {
    0
}

/// Print a global matcher. Stub.
/// Port of `print_gmatcher()` from Src/Zle/compctl.c:301.
pub(crate) fn print_gmatcher(_ac: i32) {}

/// Get a compctl from arg vector — main compctl-spec parser.
/// Port of `get_compctl()` from Src/Zle/compctl.c:373 (~600 lines).
///
/// Walks `argv` letter-by-letter, applying flag bits to `cc.mask` /
/// `cc.mask2` and capturing the string args (`-K func`, `-X expl`,
/// `-P prefix`, `-S suffix`, `-g glob`, `-s str`, etc.).
///
/// Returns 0 on success, 1 on parse error. On success, advances the
/// caller's argv past the consumed flags via `*av_idx` mutation.
///
/// Currently implements the simple-flag-char arms (per-char →
/// mask bit) from compctl.c:418-508 and the simple arg-taking
/// flags. The complex arms (`-x` extended condition, `-M` matcher,
/// `-+` chains, `-t` retry spec) are left as placeholders pending
/// per-arm follow-up.
pub(crate) fn get_compctl(
    name: &str,
    av: &mut Vec<String>,
    cc: &mut CompCtl,
    first: bool,
    mut isdef: bool,
    cl: i32,
) -> i32 {
    // C: `argv = *av;` — alias the caller's array.
    let mut i: usize = 0;
    let hx = false;
    let mut cclist_local = *CCLIST.lock().unwrap();
    cc.mask2 = cc_flags2::CCCONT;                            // c:407

    // C: `compctl + foo ...` becomes default — c:392-404
    if first
        && i < av.len()
        && av[i] == "+"
        && !(i + 1 < av.len() && av[i + 1].starts_with('-') && av[i + 1].len() > 1)
    {
        i += 1;
        if i < av.len() && av[i].starts_with('-') {
            i += 1;
        }
        av.drain(0..i);
        if cl != 0 {
            return 1;
        } else {
            *CCLIST.lock().unwrap() = comp_op::REMOVE;
            return 0;
        }
    }

    // Loop through the flags. C: c:412 `for (; !ready && argv[0] && argv[0][0] == '-' && (argv[0][1] || !first); )`
    let mut ready = false;
    while !ready
        && i < av.len()
        && av[i].starts_with('-')
        && (av[i].len() > 1 || !first)
    {
        // C: bare `-` becomes `-+` to absorb the next iter — c:413-414
        if av[i].len() == 1 {
            av[i] = "-+".to_string();
        }
        // Walk chars after the `-`. C: `while (!ready && *++(*argv))`
        let arg = av[i].clone();
        let chars: Vec<char> = arg.chars().skip(1).collect();
        let mut consumed = false;
        for c in chars {
            if ready { break; }
            // Simple-flag-char dispatch — direct port of the
            // switch at c:418-508.
            match c {
                'f' => cc.mask |= cc_flags::FILES,           // c:419
                'c' => cc.mask |= cc_flags::COMMPATH,         // c:422
                'm' => cc.mask |= cc_flags::EXTCMDS,          // c:425
                'w' => cc.mask |= cc_flags::RESWDS,           // c:428
                'o' => cc.mask |= cc_flags::OPTIONS,          // c:431
                'v' => cc.mask |= cc_flags::VARS,             // c:434
                'b' => cc.mask |= cc_flags::BINDINGS,         // c:437
                'A' => cc.mask |= cc_flags::ARRAYS,           // c:440
                'I' => cc.mask |= cc_flags::INTVARS,          // c:443
                'F' => cc.mask |= cc_flags::SHFUNCS,          // c:446
                'p' => cc.mask |= cc_flags::PARAMS,           // c:449
                'E' => cc.mask |= cc_flags::ENVVARS,          // c:452
                'j' => cc.mask |= cc_flags::JOBS,             // c:455
                'r' => cc.mask |= cc_flags::RUNNING,          // c:458
                'z' => cc.mask |= cc_flags::STOPPED,          // c:461
                'B' => cc.mask |= cc_flags::BUILTINS,         // c:464
                'a' => cc.mask |= cc_flags::ALREG | cc_flags::ALGLOB, // c:467
                'R' => cc.mask |= cc_flags::ALREG,            // c:470
                'G' => cc.mask |= cc_flags::ALGLOB,           // c:473
                'u' => cc.mask |= cc_flags::USERS,            // c:476
                'd' => cc.mask |= cc_flags::DISCMDS,          // c:479
                'e' => cc.mask |= cc_flags::EXCMDS,           // c:482
                'N' => cc.mask |= cc_flags::SCALARS,          // c:485
                'O' => cc.mask |= cc_flags::READONLYS,        // c:488
                'Z' => cc.mask |= cc_flags::SPECIALS,         // c:491
                'q' => cc.mask |= cc_flags::REMOVE,           // c:494
                'U' => cc.mask |= cc_flags::DELETE,           // c:497
                'n' => cc.mask |= cc_flags::NAMED,            // c:500
                'Q' => cc.mask |= cc_flags::QUOTEFLAG,        // c:503
                '/' => cc.mask |= cc_flags::DIRS,             // c:506
                '1' => {                                       // c:722
                    cc.mask2 |= cc_flags2::UNIQALL;
                    cc.mask2 &= !cc_flags2::UNIQCON;
                }
                '2' => {                                       // c:726
                    cc.mask2 |= cc_flags2::UNIQCON;
                    cc.mask2 &= !cc_flags2::UNIQALL;
                }
                'C' => {                                       // c:777
                    if cl != 0 {
                        eprintln!("{}: illegal option -{}", name, c);
                        return 1;
                    }
                    if first && !hx {
                        cclist_local |= comp_op::COMMAND;
                    } else {
                        eprintln!("{}: misplaced command completion (-C) flag", name);
                        return 1;
                    }
                }
                'D' => {                                       // c:789
                    if cl != 0 {
                        eprintln!("{}: illegal option -{}", name, c);
                        return 1;
                    }
                    if first && !hx {
                        isdef = true;
                        cclist_local |= comp_op::DEFAULT;
                    } else {
                        eprintln!("{}: misplaced default completion (-D) flag", name);
                        return 1;
                    }
                }
                'T' => {                                       // c:802
                    if cl != 0 {
                        eprintln!("{}: illegal option -{}", name, c);
                        return 1;
                    }
                    if first && !hx {
                        cclist_local |= comp_op::FIRST;
                    } else {
                        eprintln!("{}: misplaced first completion (-T) flag", name);
                        return 1;
                    }
                }
                'L' => {                                       // c:814
                    if cl != 0 {
                        eprintln!("{}: illegal option -{}", name, c);
                        return 1;
                    }
                    if !first || hx {
                        eprintln!("{}: illegal use of -L flag", name);
                        return 1;
                    }
                    cclist_local |= comp_op::LIST;
                }
                '+' => {                                       // c:850 (xor chain marker)
                    // Marks end of this compctl spec; remainder is
                    // the next xor'd compctl. Stop the loop here;
                    // the caller iterates again for the xor chain.
                    ready = true;
                    consumed = true;
                    break;
                }
                _ => {
                    // Arg-taking flags + unknown — bail to the
                    // post-loop handler. These are c:509+ (`t` retry,
                    // `k` keyvar, `K` func, `Y`/`X` explain, `y`
                    // ylist, `P`/`S` prefix/suffix, `g` glob, `s`
                    // str, `l`/`h` subcmd/substr, `W` withd, `J`/`V`
                    // gname, `M` matcher, `H` history, `x` extended).
                    // For now, if the arg-taking char is followed by
                    // no body, consume one extra argv slot as the
                    // arg. Else ignore. Real impls land per-flag.
                    let (has_inline, inline_val) = (
                        arg.len() > 2 && arg.chars().nth(1) == Some(c),
                        if arg.len() > 2 { arg[2..].to_string() } else { String::new() },
                    );
                    let mut val: Option<String> = None;
                    if has_inline {
                        val = Some(inline_val);
                    } else if i + 1 < av.len() {
                        val = Some(av[i + 1].clone());
                        i += 1;
                    }
                    match c {
                        'k' => cc.keyvar = val,                // c:553
                        'K' => cc.func = val,                  // c:565
                        'Y' => {                                // c:577
                            cc.mask |= cc_flags::EXPANDEXPL;
                            cc.explain = val;
                        }
                        'X' => {                                // c:580
                            cc.mask &= !cc_flags::EXPANDEXPL;
                            cc.explain = val;
                        }
                        'y' => cc.ylist = val,                 // c:594
                        'P' => cc.prefix = val,                // c:606
                        'S' => cc.suffix = val,                // c:618
                        'g' => cc.glob = val,                  // c:630
                        's' => cc.str_expansion = val,         // c:642
                        'l' => cc.subcmd = val,                // c:655
                        'h' => cc.substr = val,                // c:670
                        'W' => cc.withd = val,                 // c:685
                        'J' => cc.gname = val,                 // c:697
                        'V' => {                                // c:709
                            cc.gname = val;
                            cc.mask2 |= cc_flags2::NOSORT;
                        }
                        'M' => {                                // c:730
                            // Matcher spec — full parse needs
                            // `parse_cmatcher` (Src/Zle/compmatch.c).
                            // For now, store the raw string.
                            if let Some(s) = val {
                                cc.mstr = Some(s);
                            }
                        }
                        'H' => {                                // c:757
                            // -H N PAT — number + pattern. The
                            // simple-flag walker consumed N as `val`;
                            // the next argv is PAT.
                            if let Some(s) = val {
                                cc.hnum = s.parse::<i32>().unwrap_or(0).max(0);
                            }
                            if i + 1 < av.len() {
                                cc.hpat = Some(av[i + 1].clone());
                                if cc.hpat.as_deref() == Some("*") {
                                    cc.hpat = Some(String::new());
                                }
                                i += 1;
                            }
                        }
                        't' => {                                // c:509 retry spec
                            // `-t {+|n|-|x}` controls continuation.
                            // Direct port of the switch at c:528-545.
                            if let Some(s) = val {
                                let bit = match s.as_str() {
                                    "+" => cc_flags2::XORCONT,
                                    "n" => 0,
                                    "-" => cc_flags2::PATCONT,
                                    "x" => cc_flags2::DEFCONT,
                                    _ => {
                                        eprintln!("{}: invalid retry specification character `{}`", name, s);
                                        return 1;
                                    }
                                };
                                cc.mask2 = bit;
                            }
                        }
                        _ => {
                            eprintln!("{}: unknown compctl flag `-{}`", name, c);
                            return 1;
                        }
                    }
                    consumed = true;
                    break;
                }
            }
        }
        i += 1;
        if !consumed {
            // Pure simple-flag arg — already advanced.
        }
    }

    // C: c:1582 — push the parsed cct into the caller's slot.
    av.drain(0..i);
    let _ = isdef;
    *CCLIST.lock().unwrap() = cclist_local;
    0
}

/// Parse the `-x` extended-condition compctl form.
/// Port of `get_xcompctl()` from Src/Zle/compctl.c:909 (~260 lines).
///
/// C signature: `int get_xcompctl(char *name, char ***av, Compctl cc,
/// int isdef)`. Walks the per-condition syntax `s[…][…], p[…]` …
/// and chains them as CompCond entries on `cc.ext`. Each `case`
/// letter dispatches to one CCT_* type (`s`→CURSUF, `p`→POS, etc.),
/// then the `[…]` argument syntax is parsed per-type.
///
/// Inside the `[]`, the C source uses temporary lexer-style markers
/// `\200` (CCT_END) and `\201` (CCT_AND) to mark the active `]`/`,`
/// boundaries — Rust uses Vec splits instead.
///
/// Returns 0 on success, 1 on parse error. Advances `*av` past the
/// consumed conditions.
pub(crate) fn get_xcompctl(
    name: &str,
    av: &mut Vec<String>,
    cc: &mut CompCtl,
    isdef: bool,
) -> i32 {
    let mut ready = false;
    let mut next_chain: Vec<Arc<CompCtl>> = Vec::new();

    while !ready {
        // C: c:920 — `o = m = c = (Compcond) zshcalloc(...)`
        // o tracks or-chain head, m tracks first cond (root), c tracks
        // current cond being parsed.
        let mut head: CompCond = CompCond::default();
        let mut current_or = &mut head as *mut CompCond;

        // C: c:922 — `for (t = *argv; *t;)` walk one argv slot
        if av.is_empty() {
            // C: c:1150 — missing args
            eprintln!("{}: missing command names", name);
            return 1;
        }
        let arg = av[0].clone();
        let bytes: Vec<char> = arg.chars().collect();
        let mut t = 0_usize;
        let mut current_and: Option<*mut CompCond> = None;

        while t < bytes.len() {
            // Skip leading spaces — c:923-924
            while t < bytes.len() && bytes[t] == ' ' {
                t += 1;
            }
            if t >= bytes.len() { break; }

            // C: c:926-972 — switch on condition code char
            let typ = match bytes[t] {
                'q' => cct::QUOTE,           // c:927
                's' => cct::CURSUF,          // c:930
                'S' => cct::CURPRE,          // c:933
                'p' => cct::POS,             // c:936
                'c' => cct::CURSTR,          // c:939
                'C' => cct::CURPAT,          // c:942
                'w' => cct::WORDSTR,         // c:945
                'W' => cct::WORDPAT,         // c:948
                'n' => cct::CURSUB,          // c:951
                'N' => cct::CURSUBC,         // c:954
                'm' => cct::NUMWORDS,        // c:957
                'r' => cct::RANGESTR,        // c:960
                'R' => cct::RANGEPAT,        // c:963
                _ => {
                    eprintln!("{}: unknown condition code: {}", name, bytes[t]);
                    return 1;
                }
            };

            // C: c:974 — must be followed by `[`
            if t + 1 >= bytes.len() || bytes[t + 1] != '[' {
                eprintln!("{}: expected condition after condition code: {}", name, bytes[t]);
                return 1;
            }
            t += 1;

            // C: c:985-997 — count `[…][…]` blocks (n = arity).
            // Walk balanced brackets, collecting bodies.
            let mut bodies: Vec<String> = Vec::new();
            while t < bytes.len() && bytes[t] == '[' {
                t += 1;  // skip `[`
                // skip leading spaces inside brackets — c:1028
                while t < bytes.len() && bytes[t] == ' ' { t += 1; }
                let body_start = t;
                let mut depth = 1_i32;
                while t < bytes.len() && depth > 0 {
                    if bytes[t] == '\\' && t + 1 < bytes.len() {
                        t += 2;
                        continue;
                    }
                    if bytes[t] == '[' { depth += 1; }
                    else if bytes[t] == ']' { depth -= 1; if depth == 0 { break; } }
                    t += 1;
                }
                if t >= bytes.len() {
                    eprintln!("{}: error after condition code", name);
                    return 1;
                }
                let body: String = bytes[body_start..t].iter().collect();
                bodies.push(body);
                t += 1;  // skip `]`
            }
            let n = bodies.len() as i32;

            // C: c:1009-1025 — allocate per-type data, dispatch parse.
            let data = match typ {
                t if t == cct::POS || t == cct::NUMWORDS => {
                    // c:1030-1054 — one or two ints per body.
                    let mut a: Vec<i32> = Vec::with_capacity(n as usize);
                    let mut b: Vec<i32> = Vec::with_capacity(n as usize);
                    for body in &bodies {
                        // body shape: "N" or "N,M"
                        let parts: Vec<&str> = body.splitn(2, ',').collect();
                        let av_n: i32 = parts[0].trim().parse().unwrap_or(0);
                        let bv_n: i32 = if parts.len() == 2 {
                            parts[1].trim().parse().unwrap_or(0)
                        } else {
                            av_n  // c:1042 — single arg → b copies a
                        };
                        a.push(av_n);
                        b.push(bv_n);
                    }
                    CompCondData::Range { a, b }
                }
                t if t == cct::CURSUF || t == cct::CURPRE || t == cct::QUOTE => {
                    // c:1056-1069 — single string per body.
                    let s: Vec<String> = bodies.iter().cloned().collect();
                    let p: Vec<i32> = vec![0; s.len()];
                    CompCondData::Strings { p, s }
                }
                t if t == cct::RANGESTR || t == cct::RANGEPAT => {
                    // c:1070-1099 — two strings per body, comma-separated.
                    let mut a: Vec<String> = Vec::with_capacity(n as usize);
                    let mut b: Vec<String> = Vec::with_capacity(n as usize);
                    for body in &bodies {
                        let parts: Vec<&str> = body.splitn(2, ',').collect();
                        a.push(parts[0].to_string());
                        b.push(parts.get(1).map(|s| s.to_string()).unwrap_or_default());
                    }
                    CompCondData::StringRange { a, b }
                }
                _ => {
                    // c:1100-1121 — number followed by string per body.
                    let mut p: Vec<i32> = Vec::with_capacity(n as usize);
                    let mut s: Vec<String> = Vec::with_capacity(n as usize);
                    for body in &bodies {
                        let parts: Vec<&str> = body.splitn(2, ',').collect();
                        if parts.len() != 2 {
                            eprintln!("{}: error in condition", name);
                            return 1;
                        }
                        p.push(parts[0].trim().parse().unwrap_or(0));
                        s.push(parts[1].to_string());
                    }
                    CompCondData::Strings { p, s }
                }
            };

            // Fill the current condition node.
            // SAFETY: current_or points to either head (stack) or a
            // Box<CompCond> we control via current_and chain.
            unsafe {
                let cur = match current_and {
                    Some(p) => p,
                    None => current_or,
                };
                (*cur).typ = typ;
                (*cur).n = n;
                (*cur).data = data;
            }

            // Skip trailing spaces — c:1123
            while t < bytes.len() && bytes[t] == ' ' { t += 1; }

            // C: c:1125-1134 — `,` → or-chain, else and-chain
            if t < bytes.len() && bytes[t] == ',' {
                let new_node = Box::new(CompCond::default());
                let new_ptr = Box::into_raw(new_node);
                unsafe {
                    let cur = current_and.unwrap_or(current_or);
                    (*cur).or = Some(Box::from_raw(new_ptr));
                    current_or = (*cur).or.as_mut().unwrap().as_mut() as *mut CompCond;
                }
                current_and = None;
                t += 1;
            } else if t < bytes.len() {
                let new_node = Box::new(CompCond::default());
                let new_ptr = Box::into_raw(new_node);
                unsafe {
                    let cur = current_and.unwrap_or(current_or);
                    (*cur).and = Some(Box::from_raw(new_ptr));
                    current_and = Some((*cur).and.as_mut().unwrap().as_mut() as *mut CompCond);
                }
            }
        }

        // C: c:1137-1142 — assign condition to a fresh compctl on
        // the chain, parse the flags that follow.
        let mut next_cc = CompCtl::default();
        next_cc.cond = Some(head);
        // Drop the consumed argv slot.
        av.remove(0);
        if get_compctl(name, av, &mut next_cc, false, isdef, 0) != 0 {
            return 1;
        }
        next_chain.push(Arc::new(next_cc));

        // C: c:1143-1145 — special target → finished
        let cclist = *CCLIST.lock().unwrap();
        if (av.is_empty()) && (cclist & comp_op::SPECIAL) != 0 {
            ready = true;
            continue;
        }

        // C: c:1150-1162 — look for next `-` flag block or `--` term
        if av.is_empty()
            || !av[0].starts_with('-')
            || (av[0].len() == 1 && av.len() < 2)
        {
            eprintln!("{}: missing command names", name);
            return 1;
        }
        if av[0] == "--" {
            ready = true;
        } else if av[0] == "-+" && av.len() >= 2 && av[1] == "--" {
            ready = true;
            av.remove(0);
        }
        av.remove(0);
    }

    // C: c:1167-1168 — install the chain on cc.ext.
    if let Some(first) = next_chain.into_iter().next() {
        cc.ext = Some(first);
    }
    0
}

/// Copy fields from `cct` into the spec stored at `name`.
/// Port of `cc_assign()` from Src/Zle/compctl.c:1173 (~75 lines).
///
/// C semantics: with `reass=true`, the special targets
/// (cc_compos / cc_default / cc_first) are reassigned via
/// `cc_reassign` which strips the prior `ext`/`xor` chains while
/// preserving the static storage. Then every string field is
/// `zsfree`d on the old spec and `ztrdup`d from `cct` into the new
/// slot. Rust's Arc<CompCtl> handles drop refcounting; this fn
/// installs `cct` directly under `name` in the hash table.
///
/// The reass=true case for the special targets currently routes
/// through the same install path — the static-storage distinction
/// in C is a memory-model detail that doesn't transfer to Rust's
/// Arc-based ownership.
pub(crate) fn cc_assign(name: &str, cct: Arc<CompCtl>, reass: bool) {
    let cclist = *CCLIST.lock().unwrap();
    if reass && (cclist & comp_op::LIST) == 0 {
        // C: c:1182-1188 — reject conflicting special targets
        let conflicts = cclist == (comp_op::COMMAND | comp_op::DEFAULT)
            || cclist == (comp_op::COMMAND | comp_op::FIRST)
            || cclist == (comp_op::DEFAULT | comp_op::FIRST)
            || cclist == comp_op::SPECIAL;
        if conflicts {
            eprintln!("{}: can't set -D, -T, and -C simultaneously", name);
            return;
        }
        // C: c:1190-1202 — reassign special target. The COMMAND /
        // DEFAULT / FIRST cases install under reserved names. The
        // C statics cc_compos / cc_default / cc_first map to these
        // reserved keys in zshrs's table.
        if (cclist & comp_op::COMMAND) != 0 {
            let _ = cc_reassign(cct.clone());
            let mut g = COMPCTL_TAB.lock().unwrap();
            if g.is_none() { *g = Some(HashMap::new()); }
            if let Some(map) = g.as_mut() {
                map.insert("__cc_compos".to_string(), cct);
            }
            return;
        }
        if (cclist & comp_op::DEFAULT) != 0 {
            let _ = cc_reassign(cct.clone());
            let mut g = COMPCTL_TAB.lock().unwrap();
            if g.is_none() { *g = Some(HashMap::new()); }
            if let Some(map) = g.as_mut() {
                map.insert("__cc_default".to_string(), cct);
            }
            return;
        }
        if (cclist & comp_op::FIRST) != 0 {
            let _ = cc_reassign(cct.clone());
            let mut g = COMPCTL_TAB.lock().unwrap();
            if g.is_none() { *g = Some(HashMap::new()); }
            if let Some(map) = g.as_mut() {
                map.insert("__cc_first".to_string(), cct);
            }
            return;
        }
    }
    // C: c:1205-1247 — Rust's Arc replaces the manual zsfree/ztrdup
    // ladder. The new spec is installed under `name`; the prior
    // entry (if any) drops its refcount when this insert overwrites.
    let mut g = COMPCTL_TAB.lock().unwrap();
    if g.is_none() { *g = Some(HashMap::new()); }
    if let Some(map) = g.as_mut() {
        map.insert(name.to_string(), cct);
    }
}

/// Free a special-target compctl's chain while preserving its slot.
/// Port of `cc_reassign()` from Src/Zle/compctl.c:1252.
///
/// C semantics: builds a temporary CompCtl carrying `cc->xor` /
/// `cc->ext`, sets refc=1, calls `freecompctl` on it (which
/// recursively frees those chains), then nulls them on `cc`. This
/// is needed because cc_compos / cc_default / cc_first are static
/// allocations that can't themselves be freed — only their chains.
///
/// Rust's Arc handles refcounting. Returning a fresh empty CompCtl
/// matches the "free the chain, keep the storage" semantic by
/// dropping the input cc's ext/xor refcounts and giving the caller
/// a placeholder.
pub(crate) fn cc_reassign(_cc: Arc<CompCtl>) -> Arc<CompCtl> {
    // Arc drop on the input cc handles the C `freecompctl(c2)` call —
    // when refcount hits zero, ext/xor chains drop too. Return an
    // empty placeholder for the caller to populate.
    Arc::new(CompCtl::default())
}

/// Test whether the given string is a pattern.
/// Port of `compctl_name_pat()` from Src/Zle/compctl.c:1274.
///
/// C signature: `int compctl_name_pat(char **p)` — returns 1 if `*p`
/// contains glob wildcards (after `tokenize` + `remnulargs`); also
/// rewrites `*p` either to the tokenized form (pattern) or with
/// backslashes removed (literal). Rust port: returns `(is_pattern,
/// new_text)` tuple since we can't mutate a `&str` in-place.
///
/// Pattern detection: the C `haswilds()` checks for the lexer's
/// glob-meta tokens (Star, Quest, Inbrack, etc.). Since the input
/// here is plain user-typed text, we approximate by checking for
/// the literal `*`/`?`/`[` characters.
pub(crate) fn compctl_name_pat(p: &str) -> (bool, String) {
    // C: c:1282 `if (haswilds(s))` — has glob metas
    let has_glob = p.chars().any(|c| matches!(c, '*' | '?' | '['));
    if has_glob {
        // C: c:1283 `*p = s` — keep the (tokenized) pattern as-is.
        // Rust: return the original; caller treats as pattern.
        (true, p.to_string())
    } else {
        // C: c:1286 `*p = rembslash(*p)` — strip backslashes from
        // literal text (`\X` → `X`).
        let mut out = String::with_capacity(p.len());
        let mut chars = p.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\' {
                if let Some(&nx) = chars.peek() {
                    out.push(nx);
                    chars.next();
                    continue;
                }
            }
            out.push(c);
        }
        (false, out)
    }
}

/// Delete a pattern compctl by name.
/// Port of `delpatcomp()` from Src/Zle/compctl.c:1294. Walks the
/// patcomps list, removes the entry matching `n`, frees the cc.
/// Rust's Vec::retain handles the linked-list-style removal.
pub(crate) fn delpatcomp(n: &str) {
    let mut p = PATCOMPS.lock().unwrap();
    p.retain(|(pat, _)| pat != n);
}

/// Process the parsed compctl into the table.
/// Port of `compctl_process_cc()` from Src/Zle/compctl.c:1314 —
/// installs the spec into compctltab (or patcomps for `-p PAT`),
/// or removes entries when COMP_REMOVE is set (the `-` flag).
pub(crate) fn compctl_process_cc(s: &[String], cc: Arc<CompCtl>) -> i32 {
    let cclist = *CCLIST.lock().unwrap();
    if (cclist & comp_op::REMOVE) != 0 {
        // C: c:1320-1328 — delete entries for the listed commands
        for n in s {
            // pattern shape — `compctl -p`. compctl_name_pat
            // returns true if `n` looks like a pattern; here we
            // just check both tables.
            let mut p = PATCOMPS.lock().unwrap();
            let len_before = p.len();
            p.retain(|(pat, _)| pat != n);
            let pat_removed = p.len() != len_before;
            drop(p);
            if !pat_removed {
                if let Some(map) = COMPCTL_TAB.lock().unwrap().as_mut() {
                    map.remove(n);
                }
            }
        }
    } else {
        // C: c:1330-1351 — add the parsed compctl to the table
        for n in s {
            // For now, treat all names as plain (not pattern) —
            // pattern-mode `-p` requires get_compctl to set a flag
            // we haven't ported yet.
            let mut g = COMPCTL_TAB.lock().unwrap();
            if g.is_none() {
                *g = Some(HashMap::new());
            }
            if let Some(map) = g.as_mut() {
                map.insert(n.clone(), cc.clone());
            }
        }
    }
    0
}

/// Print a single compctl spec.
/// Port of `printcompctl()` from Src/Zle/compctl.c:1358 (~190 lines).
///
/// Emits the `compctl -FLAGS NAME` line that re-creates the spec.
/// Direct port of the C flag-letter walk (c:1362 `css = "fcqovbAIFp..."`):
/// each char in the css string corresponds to a CC_* bit; if the bit
/// is set in cc.mask, the letter prints. Same for `mss` against mask2.
///
/// Then per-string-arg flags (-K func, -X expl, etc.), -x extended
/// chain, +xor chain. Trailing arg is the command name (or pattern
/// when ispat=true).
pub(crate) fn printcompctl(
    s: &str,
    cc: &CompCtl,
    printflags: i32,
    ispat: bool,
) {
    // C: c:1362-1364 — flag-letter strings (positional → bit index)
    const CSS: &str = "fcqovbAIFpEjrzBRGudeNOZUnQmw/";
    const MSS: &str = " pcCwWsSnNmrRq";

    // C: c:1366
    let mut flags = cc.mask;
    let flags2 = cc.mask2;

    // C: c:1369-1372 — printflags adjusts cclist mode
    const PRINT_LIST: i32 = 1 << 0;
    const PRINT_TYPE: i32 = 1 << 1;
    let mut cclist = *CCLIST.lock().unwrap();
    if (printflags & PRINT_LIST) != 0 {
        cclist |= comp_op::LIST;
    } else if (printflags & PRINT_TYPE) != 0 {
        cclist &= !comp_op::LIST;
    }

    // C: c:1374 — adjust EXCMDS if DISCMDS not set
    if (flags & cc_flags::EXCMDS) != 0 && (flags & cc_flags::DISCMDS) == 0 {
        flags &= !cc_flags::EXCMDS;
    }

    // C: c:1379 — showmask filter
    let showmask = *SHOWMASK.lock().unwrap();
    if showmask != 0 && (flags & showmask) == 0 {
        return;
    }

    // C: c:1384-1385 — clear showmask for recursive calls
    let oldshowmask = showmask;
    *SHOWMASK.lock().unwrap() = 0;

    // C: c:1388-1402 — print prefix
    if (cclist & comp_op::LIST) != 0 {
        print!("compctl");
    } else if !s.is_empty() {
        print!("compctl");
    }

    // C: c:1404-1417 — walk CSS for primary mask flags
    for (i, ch) in CSS.chars().enumerate() {
        if ch == ' ' { continue; }
        if (flags & (1u64 << i)) != 0 {
            print!(" -{}", ch);
        }
    }

    // C: walk MSS for mask2 flags (NOSORT, etc.)
    let _ = MSS;  // mss is for the printable mask2 letters; pending
                  // a full per-bit mapping in zsh's source

    // C: c:1418-1430 — string-arg flags (-K func, etc.)
    if let Some(s) = &cc.keyvar    { print!(" -k '{}'", s); }
    if let Some(s) = &cc.glob      { print!(" -g '{}'", s); }
    if let Some(s) = &cc.str_expansion { print!(" -s '{}'", s); }
    if let Some(s) = &cc.func      { print!(" -K '{}'", s); }
    if let Some(s) = &cc.explain   {
        if (cc.mask & cc_flags::EXPANDEXPL) != 0 { print!(" -Y '{}'", s); }
        else { print!(" -X '{}'", s); }
    }
    if let Some(s) = &cc.ylist     { print!(" -y '{}'", s); }
    if let Some(s) = &cc.prefix    { print!(" -P '{}'", s); }
    if let Some(s) = &cc.suffix    { print!(" -S '{}'", s); }
    if let Some(s) = &cc.subcmd    { print!(" -l '{}'", s); }
    if let Some(s) = &cc.substr    { print!(" -h '{}'", s); }
    if let Some(s) = &cc.withd     { print!(" -W '{}'", s); }
    if let Some(s) = &cc.gname     {
        if (flags2 & cc_flags2::NOSORT) != 0 { print!(" -V '{}'", s); }
        else { print!(" -J '{}'", s); }
    }
    if let Some(s) = &cc.mstr      { print!(" -M '{}'", s); }
    if cc.hnum > 0 {
        if let Some(p) = &cc.hpat {
            print!(" -H {} '{}'", cc.hnum, if p.is_empty() { "*" } else { p });
        }
    }

    // C: c:1518-1523 — xor chain
    if cc.xor.is_some() {
        print!(" +");
    }

    // C: c:1524-1543 — trailing name (or pattern)
    if !s.is_empty() && (cclist & comp_op::LIST) != 0 {
        if ispat {
            print!(" -p '{}'", s);
        } else {
            print!(" '{}'", s);
        }
    } else if !s.is_empty() {
        print!(" '{}'", s);
    }
    println!();

    // C: c:1545 — restore showmask
    *SHOWMASK.lock().unwrap() = oldshowmask;
}

/// Print a compctl hash node.
/// Port of `printcompctlp()` from Src/Zle/compctl.c:1549 — hash-table
/// callback that calls printcompctl.
pub(crate) fn printcompctlp(name: &str, cc: &CompCtl, printflags: i32) {
    printcompctl(name, cc, printflags, false);
}

/// `compctl` builtin entry point.
/// Port of `bin_compctl()` from Src/Zle/compctl.c:1561 (~110 lines).
/// Direct port of the C dispatch flow:
///   1. Reset cclist + showmask
///   2. Try `get_gmatcher` — if returns non-zero, return that-1
///   3. Allocate cct, run `get_compctl`. On failure, free + return 1
///   4. Save mask in showmask (with EXCMDS/DISCMDS adjust)
///   5. If no remaining args or COMP_LIST, free cc
///   6. If no args and no special: print all (patcomps + compctltab +
///      cc_compos/cc_default/cc_first + global matchers)
///   7. If COMP_LIST: print only the named entries
///   8. Else: install via compctl_process_cc
pub(crate) fn bin_compctl(name: &str, argv: &[String]) -> i32 {
    let mut argv: Vec<String> = argv.to_vec();
    let mut ret: i32 = 0;

    // C: c:1570-1571 — clear static flags
    *CCLIST.lock().unwrap() = 0;
    *SHOWMASK.lock().unwrap() = 0;

    // C: c:1574-1596 — parse args if any
    if !argv.is_empty() {
        // C: c:1576 — try global matcher first
        let gret = get_gmatcher(name, &argv);
        if gret != 0 {
            return gret - 1;
        }

        // C: c:1581 — allocate compctl
        let mut cc = CompCtl::default();
        // C: c:1582 — parse the spec
        if get_compctl(name, &mut argv, &mut cc, true, false, 0) != 0 {
            // freecompctl(cc) is implicit on Drop
            return 1;
        }

        // C: c:1589 — remember flags for printing
        let mut showmask = cc.mask;
        if (showmask & cc_flags::EXCMDS) != 0 && (showmask & cc_flags::DISCMDS) == 0 {
            showmask &= !cc_flags::EXCMDS;
        }
        *SHOWMASK.lock().unwrap() = showmask;

        let cclist = *CCLIST.lock().unwrap();
        // C: c:1594 — if no command args or just listing, drop cc
        if argv.is_empty() || (cclist & comp_op::LIST) != 0 {
            // cc dropped at end of if-let
        } else {
            // C: c:1656-1664 — install via compctl_process_cc
            if (cclist & comp_op::SPECIAL) != 0 {
                // C: c:1657 — special targets ignore extra args
                eprintln!("{}: extraneous commands ignored", name);
            } else {
                let cc_arc = Arc::new(cc);
                ret = compctl_process_cc(&argv, cc_arc);
            }
            return ret;
        }
    }

    let cclist = *CCLIST.lock().unwrap();

    // C: c:1601 — if no commands and no special-target flag, print all
    if argv.is_empty() && (cclist & (comp_op::SPECIAL | comp_op::LISTMATCH)) == 0 {
        // Print pattern compctls
        let pats = PATCOMPS.lock().unwrap().clone();
        for (pat, cc) in &pats {
            printcompctl(pat, cc, 0, true);
        }
        // Print all hash table entries (sorted for stable output)
        if let Some(map) = COMPCTL_TAB.lock().unwrap().as_ref() {
            let mut names: Vec<&String> = map.keys().collect();
            names.sort();
            for n in names {
                if let Some(cc) = map.get(n) {
                    printcompctlp(n, cc, 0);
                }
            }
        }
        // Print special compctls (cc_compos, cc_default, cc_first
        // are handled by the `default` table — out of scope until
        // we wire up those globals).
        print_gmatcher((cclist & comp_op::LIST) as i32);
        return ret;
    }

    // C: c:1618 — if listing, print only named entries
    if (cclist & comp_op::LIST) != 0 {
        *SHOWMASK.lock().unwrap() = 0;
        for n in &argv {
            let mut found = false;
            // Try pattern compctls first
            let pats = PATCOMPS.lock().unwrap().clone();
            for (pat, cc) in &pats {
                if pat == n {
                    printcompctl(pat, cc, 0, true);
                    found = true;
                    break;
                }
            }
            if !found {
                if let Some(map) = COMPCTL_TAB.lock().unwrap().as_ref() {
                    if let Some(cc) = map.get(n) {
                        printcompctlp(n, cc, 0);
                        found = true;
                    }
                }
            }
            if !found {
                eprintln!("{}: no compctl defined for {}", name, n);
                ret = 1;
            }
        }
        if (cclist & comp_op::LISTMATCH) != 0 {
            print_gmatcher(comp_op::LIST as i32);
        }
    }

    ret
}

/// `compcall` builtin entry point.
/// Port of `bin_compcall()` from Src/Zle/compctl.c:1755 — re-invokes
/// the completion machinery from inside a -K function.
pub(crate) fn bin_compcall(_name: &str, _argv: &[String]) -> i32 {
    0
}

/// `compctl -K`'s bound `compctlread` callback.
/// Port of `compctlread()` from Src/Zle/compctl.c:1795 — replaces
/// the fallback_compctlread default with the real ZLE-aware
/// implementation when the compctl module loads.
pub(crate) fn compctlread(_name: &str, _args: &[String]) -> i32 {
    0
}

/// Hook for completion-list build start.
/// Port of `ccmakehookfn()` from Src/Zle/compctl.c:1958. Called by
/// the completion driver to build the candidate list.
pub(crate) fn ccmakehookfn(_dat: ()) -> i32 {
    0
}

/// Hook for completion-list build cleanup.
/// Port of `cccleanuphookfn()` from Src/Zle/compctl.c:1996.
pub(crate) fn cccleanuphookfn(_dat: ()) -> i32 {
    0
}

/// Add a match to the result list.
/// Port of `addmatch()` from Src/Zle/compctl.c:2028.
pub(crate) fn addmatch(_s: &str, _t: &str) {}

/// Build the tilde-expansion list.
/// Port of `maketildelist()` from Src/Zle/compctl.c:2086.
pub(crate) fn maketildelist() {}

/// Get a compctl character pattern.
/// Port of `getcpat()` from Src/Zle/compctl.c:2178.
pub(crate) fn getcpat(_str: &str, _cpatindex: i32, _cpat: &str, _class: i32) -> i32 {
    0
}

/// Dump a hash table for completion.
/// Port of `dumphashtable()` from Src/Zle/compctl.c:2228.
pub(crate) fn dumphashtable(_what: i32) {}

/// Hashnode → match adapter.
/// Port of `addhnmatch()` from Src/Zle/compctl.c:2245.
pub(crate) fn addhnmatch(_name: &str, _flags: i32) {}

/// Resolve a real path.
/// Port of `getreal()` from Src/Zle/compctl.c:2275.
pub(crate) fn getreal(_str: &str) -> Option<String> {
    None
}

/// Generate file-name matches.
/// Port of `gen_matches_files()` from Src/Zle/compctl.c:2350.
pub(crate) fn gen_matches_files(_dirs: bool, _execs: bool, _all: bool) {}

/// Find a node in a linked list.
/// Port of `findnode()` from Src/Zle/compctl.c:2658.
pub(crate) fn findnode<T>(_list: &[T], _dat: &T) -> Option<usize> {
    None
}

/// Build the completion list (control entry).
/// Port of `makecomplistctl()` from Src/Zle/compctl.c:2680.
pub(crate) fn makecomplistctl(_flags: i32) {}

/// Build the global completion list.
/// Port of `makecomplistglobal()` from Src/Zle/compctl.c:2715.
pub(crate) fn makecomplistglobal(_os: &str, _incmd: bool, _lst: i32, _flags: i32) {}

/// Build the per-command completion list.
/// Port of `makecomplistcmd()` from Src/Zle/compctl.c:2843.
pub(crate) fn makecomplistcmd(_os: &str, _incmd: bool, _flags: i32) {}

/// Build the per-position completion list.
/// Port of `makecomplistpc()` from Src/Zle/compctl.c:2934.
pub(crate) fn makecomplistpc(_os: &str, _incmd: bool) {}

/// Build the completion list from a compctl spec.
/// Port of `makecomplistcc()` from Src/Zle/compctl.c:2998.
pub(crate) fn makecomplistcc(_cc: &CompCtl, _s: &str, _incmd: bool) {}

/// Build the completion list from an OR'd compctl chain.
/// Port of `makecomplistor()` from Src/Zle/compctl.c:3045.
pub(crate) fn makecomplistor(_cc: &CompCtl, _s: &str, _incmd: bool, _compadd: i32, _sub: i32) {}

/// Build the completion list — top-level dispatch.
/// Port of `makecomplistlist()` from Src/Zle/compctl.c:3081.
pub(crate) fn makecomplistlist(_cc: &CompCtl, _s: &str, _incmd: bool, _compadd: i32) {}

/// Build the extended (`-x`) completion list.
/// Port of `makecomplistext()` from Src/Zle/compctl.c:3155.
pub(crate) fn makecomplistext(_occ: &CompCtl, _os: &str, _incmd: bool) {}

/// Separate a completion string into prefix/suffix/word.
/// Port of `sep_comp_string()` from Src/Zle/compctl.c:3344.
pub(crate) fn sep_comp_string(_ss: &str, _s: &str, _noffs: i32) -> i32 {
    0
}

/// Apply flag-driven generators to populate the completion list.
/// Port of `makecomplistflags()` from Src/Zle/compctl.c:3499 — the
/// largest fn in this file (~500 lines), iterates the bits in
/// `cc->mask` and dispatches per-bit to the matching generator
/// (files, vars, jobs, etc.).
pub(crate) fn makecomplistflags(_cc: &CompCtl, _s: &str, _incmd: bool, _compadd: i32) {}

// =================================================================
// Module boot/cleanup hooks — port of compctl.c:4000+
// =================================================================

/// Setup hook — port of `setup_()` from Src/Zle/compctl.c:4001.
pub(crate) fn setup_() -> i32 {
    createcompctltable();
    0
}

/// Features hook — port of `features_()` from Src/Zle/compctl.c:4014.
pub(crate) fn features_() -> Vec<String> {
    Vec::new()
}

/// Enables hook — port of `enables_()` from Src/Zle/compctl.c:4032.
pub(crate) fn enables_() -> Vec<i32> {
    Vec::new()
}

/// Boot hook — port of `boot_()` from Src/Zle/compctl.c:4045.
pub(crate) fn boot_() -> i32 {
    0
}

/// Cleanup hook — port of `cleanup_()` from Src/Zle/compctl.c:4058.
pub(crate) fn cleanup_() -> i32 {
    0
}

/// Finish hook — port of `finish_()` from Src/Zle/compctl.c:4072.
pub(crate) fn finish_() -> i32 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialize tests that touch the singleton state — `cargo test`
    /// runs tests in parallel and the static `COMPCTL_TAB` / `CCLIST`
    /// would interleave. The parking_lot variant would deadlock-free
    /// across panics; std::sync::Mutex is fine since each test runs
    /// quickly and panics propagate.
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn createcompctltable_initializes_table() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createcompctltable();
        let g = COMPCTL_TAB.lock().unwrap();
        assert!(g.is_some());
        assert_eq!(g.as_ref().unwrap().len(), 0);
    }

    #[test]
    fn cc_assign_inserts_into_table() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createcompctltable();
        let cc = Arc::new(CompCtl {
            mask: cc_flags::FILES,
            ..Default::default()
        });
        cc_assign("ls", cc, false);
        let g = COMPCTL_TAB.lock().unwrap();
        assert!(g.as_ref().unwrap().contains_key("ls"));
    }

    #[test]
    fn freecompctlp_removes_entry() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createcompctltable();
        cc_assign("rm", Arc::new(CompCtl::default()), false);
        freecompctlp("rm");
        let g = COMPCTL_TAB.lock().unwrap();
        assert!(!g.as_ref().unwrap().contains_key("rm"));
    }

    #[test]
    fn cc_flags_bit_layout_matches_c_compctlh() {
        // Spot-check that the bit values match the C constants.
        assert_eq!(cc_flags::FILES, 1);
        assert_eq!(cc_flags::COMMPATH, 2);
        assert_eq!(cc_flags::OPTIONS, 8);
        assert_eq!(cc_flags::JOBS, 1 << 11);
    }

    #[test]
    fn cct_constants_match_c_compctlh() {
        assert_eq!(cct::POS, 1);
        assert_eq!(cct::CURPAT, 3);
        assert_eq!(cct::QUOTE, 13);
    }

    #[test]
    fn comp_op_special_combines_command_default_first() {
        assert_eq!(
            comp_op::SPECIAL,
            comp_op::COMMAND | comp_op::DEFAULT | comp_op::FIRST
        );
    }

    #[test]
    fn cc_flags2_constants_match_c_compctlh() {
        assert_eq!(cc_flags2::NOSORT, 1);
        assert_eq!(cc_flags2::CCCONT, 4);
        assert_eq!(cc_flags2::UNIQALL, 1 << 6);
    }

    #[test]
    fn get_compctl_simple_flag_chars_set_mask() {
        // `compctl -fcv ls` — files + commpath + vars
        let mut argv = vec!["-fcv".to_string(), "ls".to_string()];
        let mut cc = CompCtl::default();
        let r = get_compctl("compctl", &mut argv, &mut cc, true, false, 0);
        assert_eq!(r, 0);
        assert_ne!(cc.mask & cc_flags::FILES, 0);
        assert_ne!(cc.mask & cc_flags::COMMPATH, 0);
        assert_ne!(cc.mask & cc_flags::VARS, 0);
        // `ls` should remain in argv
        assert_eq!(argv, vec!["ls".to_string()]);
    }

    #[test]
    fn get_compctl_combined_a_sets_alreg_and_alglob() {
        let mut argv = vec!["-a".to_string(), "ls".to_string()];
        let mut cc = CompCtl::default();
        get_compctl("compctl", &mut argv, &mut cc, true, false, 0);
        assert_ne!(cc.mask & cc_flags::ALREG, 0);
        assert_ne!(cc.mask & cc_flags::ALGLOB, 0);
    }

    #[test]
    fn get_compctl_arg_taking_K_captures_function_name() {
        let mut argv = vec!["-K".to_string(), "_my_completer".to_string(), "myfunc".to_string()];
        let mut cc = CompCtl::default();
        get_compctl("compctl", &mut argv, &mut cc, true, false, 0);
        assert_eq!(cc.func.as_deref(), Some("_my_completer"));
        assert_eq!(argv, vec!["myfunc".to_string()]);
    }

    #[test]
    fn get_compctl_inline_arg_K_captures_function_name() {
        // `-K_my_func`  → the K flag char with inline arg
        let mut argv = vec!["-K_my_func".to_string(), "myfunc".to_string()];
        let mut cc = CompCtl::default();
        get_compctl("compctl", &mut argv, &mut cc, true, false, 0);
        assert_eq!(cc.func.as_deref(), Some("_my_func"));
    }

    #[test]
    fn get_compctl_P_S_capture_prefix_suffix() {
        let mut argv = vec![
            "-P".to_string(), "before-".to_string(),
            "-S".to_string(), "-after".to_string(),
            "cmd".to_string()
        ];
        let mut cc = CompCtl::default();
        get_compctl("compctl", &mut argv, &mut cc, true, false, 0);
        assert_eq!(cc.prefix.as_deref(), Some("before-"));
        assert_eq!(cc.suffix.as_deref(), Some("-after"));
    }

    #[test]
    fn get_compctl_1_2_set_uniq_flags() {
        let mut argv = vec!["-1".to_string(), "ls".to_string()];
        let mut cc = CompCtl::default();
        get_compctl("compctl", &mut argv, &mut cc, true, false, 0);
        assert_ne!(cc.mask2 & cc_flags2::UNIQALL, 0);
        assert_eq!(cc.mask2 & cc_flags2::UNIQCON, 0);
    }

    #[test]
    fn get_compctl_V_implies_NOSORT() {
        let mut argv = vec!["-V".to_string(), "mygroup".to_string(), "cmd".to_string()];
        let mut cc = CompCtl::default();
        get_compctl("compctl", &mut argv, &mut cc, true, false, 0);
        assert_eq!(cc.gname.as_deref(), Some("mygroup"));
        assert_ne!(cc.mask2 & cc_flags2::NOSORT, 0);
    }

    #[test]
    fn bin_compctl_install_then_lookup_via_table() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createcompctltable();
        let r = bin_compctl("compctl", &["-f".to_string(), "mycmd".to_string()]);
        assert_eq!(r, 0);
        let g = COMPCTL_TAB.lock().unwrap();
        assert!(g.as_ref().unwrap().contains_key("mycmd"));
        let cc = g.as_ref().unwrap().get("mycmd").unwrap();
        assert_ne!(cc.mask & cc_flags::FILES, 0);
    }

    #[test]
    fn compctl_name_pat_detects_glob_wildcards() {
        // Glob-meta chars present → pattern.
        let (is_pat, _) = compctl_name_pat("ls*");
        assert!(is_pat);
        let (is_pat, _) = compctl_name_pat("foo?bar");
        assert!(is_pat);
        let (is_pat, _) = compctl_name_pat("[abc]");
        assert!(is_pat);
    }

    #[test]
    fn compctl_name_pat_strips_backslashes_from_literal() {
        let (is_pat, out) = compctl_name_pat("\\$home");
        assert!(!is_pat);
        // Backslash dropped, `$` kept.
        assert_eq!(out, "$home");
    }

    #[test]
    fn delpatcomp_removes_matching_pattern() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut p = PATCOMPS.lock().unwrap();
        p.push(("foo*".to_string(), Arc::new(CompCtl::default())));
        p.push(("bar*".to_string(), Arc::new(CompCtl::default())));
        drop(p);
        delpatcomp("foo*");
        let p = PATCOMPS.lock().unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].0, "bar*");
    }

    #[test]
    fn cc_assign_with_reass_command_target_uses_special_key() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createcompctltable();
        *CCLIST.lock().unwrap() = comp_op::COMMAND;
        cc_assign("compctl", Arc::new(CompCtl {
            mask: cc_flags::FILES,
            ..Default::default()
        }), true);
        let g = COMPCTL_TAB.lock().unwrap();
        assert!(g.as_ref().unwrap().contains_key("__cc_compos"));
        // Reset for other tests.
        drop(g);
        *CCLIST.lock().unwrap() = 0;
    }

    #[test]
    fn cc_assign_with_reass_default_target_uses_special_key() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createcompctltable();
        *CCLIST.lock().unwrap() = comp_op::DEFAULT;
        cc_assign("compctl", Arc::new(CompCtl::default()), true);
        let g = COMPCTL_TAB.lock().unwrap();
        assert!(g.as_ref().unwrap().contains_key("__cc_default"));
        drop(g);
        *CCLIST.lock().unwrap() = 0;
    }

    #[test]
    fn cc_assign_rejects_conflicting_special_targets() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createcompctltable();
        *CCLIST.lock().unwrap() = comp_op::COMMAND | comp_op::DEFAULT;
        cc_assign("compctl", Arc::new(CompCtl::default()), true);
        let g = COMPCTL_TAB.lock().unwrap();
        // Should have been rejected — neither key installed.
        assert!(!g.as_ref().unwrap().contains_key("__cc_compos"));
        assert!(!g.as_ref().unwrap().contains_key("__cc_default"));
        drop(g);
        *CCLIST.lock().unwrap() = 0;
    }

    #[test]
    fn compctl_process_cc_remove_deletes_named_entries() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        createcompctltable();
        cc_assign("foo", Arc::new(CompCtl::default()), false);
        cc_assign("bar", Arc::new(CompCtl::default()), false);
        *CCLIST.lock().unwrap() = comp_op::REMOVE;
        compctl_process_cc(&["foo".to_string()], Arc::new(CompCtl::default()));
        let g = COMPCTL_TAB.lock().unwrap();
        let map = g.as_ref().unwrap();
        assert!(!map.contains_key("foo"));
        assert!(map.contains_key("bar"));
        // Reset cclist for other tests.
        *CCLIST.lock().unwrap() = 0;
    }
}
