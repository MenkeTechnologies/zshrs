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
/// Port of `bin_compcall()` from Src/Zle/compctl.c:1675.
///
/// Re-invokes the completion machinery from inside a `-K` function.
/// Per c:1680, `incompfunc` must be 1 (we're inside a completion
/// function); else error. Then dispatches to makecomplistctl with
/// CFN_FIRST / CFN_DEFAULT bits cleared per `-T` / `-D` opts.
///
/// CFN_* bits (c:1672-1673):
///   CFN_FIRST   = 1  — skip cc_first
///   CFN_DEFAULT = 2  — skip cc_default
pub(crate) fn bin_compcall(name: &str, argv: &[String]) -> i32 {
    // C: c:1680-1683 — incompfunc check
    let incompfunc = *INCOMPFUNC.lock().unwrap();
    if incompfunc != 1 {
        eprintln!("{}: can only be called from completion function", name);
        return 1;
    }

    // C: c:1686-1687 — option flags. Walk argv looking for -T / -D.
    let mut flags = 0_i32;
    let mut t_set = false;
    let mut d_set = false;
    for a in argv {
        if a == "-T" { t_set = true; }
        else if a == "-D" { d_set = true; }
    }
    const CFN_FIRST: i32 = 1;
    const CFN_DEFAULT: i32 = 2;
    if !t_set { flags |= CFN_FIRST; }
    if !d_set { flags |= CFN_DEFAULT; }
    makecomplistctl(flags);
    0
}

/// Are we inside a completion function? Set by the completion-driver
/// entry/exit hooks (compctl_make / compctl_cleanup). Mirrors the C
/// `incompfunc` global from Src/Zle/zle_tricky.c.
static INCOMPFUNC: Mutex<i32> = Mutex::new(0);

/// `compctl -K`'s bound `compctlread` callback.
/// Port of `compctlread()` from Src/Zle/compctl.c:189 (~150 lines).
///
/// The function reads input for the `read` builtin invoked from
/// inside a completion function (e.g. `compctl -K myfunc` calls
/// `read -E` etc.). Replaces fallback_compctlread when the compctl
/// module is loaded. Dispatches based on -l/-n/-c flags:
///   -l    → return the current line as a scalar in `reply`
///   -ln   → return the cursor word index
///   -lc   → return the count of words on the line
///   -le/-lE — print to stdout in addition to assigning
///
/// This port stubs the ZLE-state-touching arms and keeps the
/// option-walking / error-checking faithful. The actual ZLE state
/// (zlemetacs, clwords, clwnum) lives in src/ported/zle/zle_main.rs.
pub(crate) fn compctlread(name: &str, args: &[String]) -> i32 {
    // C: c:195 — must be called from compctl-invoked function
    let incompctlfunc = *INCOMPCTLFUNC.lock().unwrap();
    if !incompctlfunc {
        eprintln!("{}: option valid only in functions called via compctl", name);
        return 1;
    }
    // Walk option flags. C uses `OPT_ISSET(ops, 'X')` — Rust scans args.
    let mut opt_l = false;
    let mut opt_n = false;
    let mut opt_c = false;
    let mut opt_e = false;
    let mut opt_e_upper = false;
    let mut reply: Option<&String> = None;
    for a in args {
        if let Some(rest) = a.strip_prefix('-') {
            for ch in rest.chars() {
                match ch {
                    'l' => opt_l = true,
                    'n' => opt_n = true,
                    'c' => opt_c = true,
                    'e' => opt_e = true,
                    'E' => opt_e_upper = true,
                    _ => {}
                }
            }
        } else {
            reply = Some(a);
        }
    }
    // C: c:202-218 — `-ln` returns cursor word index. ZLE state
    // (zlemetacs) lookup deferred — return 1+ a placeholder index
    // so the typical compctl flow at least progresses without
    // erroring. Real impl needs ZLE integration.
    if opt_l && opt_n {
        let idx = 1; // placeholder for 1+zlemetacs
        if opt_e || opt_e_upper {
            println!("{}", idx);
        }
        if !opt_e {
            if let Some(_r) = reply {
                // setsparam(reply, idx_str) — defer to ZLE wiring
            }
        }
        return 0;
    }
    if opt_l && opt_c {
        // C: c:225 — return word count. Placeholder pending ZLE.
        let cnt = 0;
        if opt_e || opt_e_upper { println!("{}", cnt); }
        return 0;
    }
    // Plain `-l` or other forms — defer until ZLE state is wired.
    let _ = reply;
    0
}

/// True iff we're inside a function called via compctl -K. Mirrors
/// the C `incompctlfunc` global from Src/Zle/zle_tricky.c — set by
/// the dispatcher around the -K function call.
static INCOMPCTLFUNC: Mutex<bool> = Mutex::new(false);

/// Hook for completion-list build start.
/// Port of `ccmakehookfn()` from Src/Zle/compctl.c:1762 (~145 lines).
///
/// Called by the completion driver via `addhookfunc("compctl_make",
/// ccmakehookfn)` (boot_). Walks `cmatcher` (global -M chain),
/// builds matcher copy, runs makecomplistglobal for each, manages
/// the per-iteration ccused/ccstack lists, accumulates results into
/// pmatches/lastmatches.
///
/// This stubs the ZLE-result-state arms (matchers/ainfo/amatches/
/// pmatches all live in zle_tricky.c) and keeps the high-level
/// per-matcher loop visible. Real impl requires the matcher port.
pub(crate) fn ccmakehookfn(_dat: ()) -> i32 {
    // C: c:1773 — queue_signals — Rust uses the runtime's signal
    // queue, no explicit queue here.

    // C: c:1779-1794 — copy global cmatcher list. Stub: skip the
    // copy since matchers aren't ported.

    // C: c:1797-1901 — for each matcher, run makecomplistglobal
    // and accumulate matches. We approximate by running the dispatch
    // once with no matcher.

    // Use the lock so static analysis doesn't flag CMATCHER as unused.
    let _guard = CMATCHER.lock();
    drop(_guard);

    // C: c:1903 — restore stdout fd
    // C: c:1905 — return 0 / dat->lst = 1 path
    0
}

/// Hook for completion-list build cleanup.
/// Port of `cccleanuphookfn()` from Src/Zle/compctl.c:1909.
///
/// Called via `addhookfunc("compctl_cleanup", cccleanuphookfn)` at
/// boot_. The C body just nulls the ccused/ccstack file-statics —
/// Rust drops them automatically when the per-call state goes out
/// of scope. Kept as a name-faithful entry for the hook table.
pub(crate) fn cccleanuphookfn(_dat: ()) -> i32 {
    // C: c:1912 — `ccused = ccstack = NULL;` — Rust equivalent is
    // a no-op since per-call state is stack-allocated.
    0
}

/// `addwhat` special-value constants — port of the negative-int
/// dispatch values documented in Src/Zle/compctl.c:1940-1951:
///   ADDWHAT_FILES_OTHER     = -1  (other file specs: ~/=...)
///   ADDWHAT_UNQUOTED        = -2  (anything unquoted)
///   ADDWHAT_EXEC_CMD        = -3  (executable command names)
///   ADDWHAT_CDABLE_PARAM    = -4  (a cdable parameter)
///   ADDWHAT_FILES           = -5  (regular files)
///   ADDWHAT_GLOB_EXPAND     = -6  (glob expansions)
///   ADDWHAT_CMD_NAME        = -7  (command names from cmdnamtab)
///   ADDWHAT_EXEC_FILE       = -8  (executable files / command paths)
///   ADDWHAT_PARAM           = -9  (parameters)
/// Positive values are CC_* flag bits (per the OR-mask path).
pub mod addwhat_kind {
    pub const FILES_OTHER: i32     = -1;  // c:1949
    pub const UNQUOTED: i32        = -2;  // c:1948
    pub const EXEC_CMD: i32        = -3;  // c:1947
    pub const CDABLE_PARAM: i32    = -4;  // c:1946
    pub const FILES: i32           = -5;  // c:1941
    pub const GLOB_EXPAND: i32     = -6;  // c:1942
    pub const CMD_NAME: i32        = -7;  // c:1945
    pub const EXEC_FILE: i32       = -8;  // c:1943
    pub const PARAM: i32           = -9;  // c:1944
}

/// File-thread `addwhat` global. Port of file-static `int addwhat;`
/// from Src/Zle/compctl.c:1749. Set by the dispatcher before each
/// addmatch / dumphashtable call to communicate the source kind.
static ADDWHAT: Mutex<i32> = Mutex::new(0);

/// Per-completion match list. Port of file-static `LinkList` of
/// matches in zle_tricky.c. The Rust port keeps a per-call Vec so
/// addmatch can accumulate results without touching ZLE globals.
static MATCH_LIST: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Add a match to the per-call result list.
/// Port of `addmatch()` from Src/Zle/compctl.c:1925 (~150 lines).
///
/// The C body is a switch over `addwhat` (file static) that:
///   - addwhat ∈ {-1, -5, -6, -7, -8, CC_FILES} → file-match path
///     (calls comp_match with prefix/suffix, applies fignore, etc.)
///   - addwhat ∈ {CC_QUOTEFLAG, -2, -3, -4, -9} → conditional accept
///   - addwhat > 0 with CC_* bits → hash-node-flag dispatch (vars,
///     funcs, builtins, aliases, bindings filtered by per-flag bits)
///   - else → reject
/// Then comp_match builds the Cline and calls addmatch1 to push.
///
/// This port keeps the addwhat-based dispatch shape but defers the
/// comp_match / Cline / fignore / per-Param-flag arms (those need
/// the matcher + Param-table ports). For now: the function records
/// `s` into MATCH_LIST when addwhat is one of the accept values
/// — sufficient for unit tests that exercise the accept/reject
/// dispatch without driving the full ZLE pipeline.
pub(crate) fn addmatch(s: &str, _t: Option<&str>) {
    let aw = *ADDWHAT.lock().unwrap();
    // C: c:1957-1990 — file-thread accept.
    let file_thread = matches!(
        aw,
        addwhat_kind::FILES_OTHER
            | addwhat_kind::FILES
            | addwhat_kind::GLOB_EXPAND
            | addwhat_kind::CMD_NAME
            | addwhat_kind::EXEC_FILE
    ) || (aw > 0 && (aw as u64 & cc_flags::FILES) != 0);
    if file_thread {
        // C: c:1988 — for -7 (CMD_NAME), check findcmd; we accept
        // unconditionally here pending findcmd port.
        MATCH_LIST.lock().unwrap().push(s.to_string());
        return;
    }
    // C: c:1991-2014 — conditional-accept thread. We accept the
    // simple unquoted / quote-flag / exec / cdable-param / param
    // cases; per-Param flag filtering pending Param table port.
    if matches!(
        aw,
        addwhat_kind::UNQUOTED
            | addwhat_kind::EXEC_CMD
            | addwhat_kind::CDABLE_PARAM
            | addwhat_kind::PARAM
    ) {
        MATCH_LIST.lock().unwrap().push(s.to_string());
        return;
    }
    if aw > 0 {
        // CC_QUOTEFLAG / CC_BINDINGS / CC_SHFUNCS / etc. — accept;
        // per-flag filtering pending hash-node integration.
        MATCH_LIST.lock().unwrap().push(s.to_string());
    }
    // else: reject — match dropped on the floor per the C `return` path.
}

/// Build the tilde-expansion (named-directory) list.
/// Port of `maketildelist()` from Src/Zle/compctl.c:2054.
///
/// C body fills the nameddirtab hash table then scans it via
/// scanhashtable with addhnmatch as the callback. Rust port walks
/// the named-dir table from src/ported/utils.rs (or env $HOME-derived
/// usernames) — for the foundation, we iterate any registered
/// named-dir entries via the executor's nameddirtab equivalent.
pub(crate) fn maketildelist() {
    // The named-dir table lookup happens via the ShellExecutor in
    // zshrs. Direct iteration here would couple compctl to that
    // module; for the foundation we leave the iteration to the
    // dispatcher that wraps maketildelist + addhnmatch.
    // C: c:2058 `nameddirtab->filltable(nameddirtab)` — pre-populate
    // from /etc/passwd or the equivalent.
    // C: c:2060 `scanhashtable(nameddirtab, …, addhnmatch, 0)` —
    // the per-entry callback here is addhnmatch.
}

/// Hash-pattern match for `compctl -x` n[…] / N[…] conditions.
/// Port of `getcpat()` from Src/Zle/compctl.c:2068.
///
/// C signature: `int getcpat(char *str, int cpatindex, char *cpat,
/// int class)` — searches `str` for the `cpatindex`-th occurrence
/// of `cpat` (positive index = forward, negative = backward, 0 = first).
/// `class` toggles char-class mode (each cpat char tests if str's
/// char is in the class) vs literal-substring mode.
///
/// Returns the 1-based index of the match end, or -1 if not found.
pub(crate) fn getcpat(str: &str, cpatindex: i32, cpat: &str, class: i32) -> i32 {
    // C: c:2073 — empty string → -1
    if str.is_empty() {
        return -1;
    }
    // C: c:2076 — strip backslashes from cpat
    let cpat_clean: String = {
        let mut out = String::with_capacity(cpat.len());
        let mut chars = cpat.chars().peekable();
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
        out
    };
    // C: c:2078-2081 — index normalization
    let (mut idx, backward) = if cpatindex == 0 {
        (1_i32, false)
    } else if cpatindex < 0 {
        (-cpatindex, true)
    } else {
        (cpatindex, false)
    };

    let str_chars: Vec<char> = str.chars().collect();
    let cpat_chars: Vec<char> = cpat_clean.chars().collect();
    let n = str_chars.len();

    // C: c:2083-2095 — the search loop, walks forward or backward.
    let positions: Vec<usize> = if backward {
        (0..n).rev().collect()
    } else {
        (0..n).collect()
    };
    for s_start in positions {
        if class != 0 {
            // C: c:2087-2090 — class mode: if str[s_start] is in
            // the class set (any char of cpat), count it.
            let sc = str_chars[s_start];
            if cpat_chars.iter().any(|&p| p == sc) {
                idx -= 1;
                if idx == 0 {
                    return (s_start + 1) as i32;
                }
            }
        } else {
            // C: c:2090-2094 — literal substring match.
            let mut t = s_start;
            let mut p = 0;
            while t < n && p < cpat_chars.len() && str_chars[t] == cpat_chars[p] {
                t += 1;
                p += 1;
            }
            if p == cpat_chars.len() {
                idx -= 1;
                if idx == 0 {
                    return t as i32;
                }
            }
        }
    }
    -1
}

/// Dump every entry of a hash table as a match.
/// Port of `dumphashtable()` from Src/Zle/compctl.c:2105.
///
/// C body: sets `addwhat = what`, iterates every node in `ht->nodes`,
/// calls `addmatch(node->nam, (char*)node)`. Rust takes an iterable
/// of names since the hash-table abstractions differ.
pub(crate) fn dumphashtable<I: IntoIterator<Item = String>>(names: I, what: i32) {
    // C: c:2111 — set addwhat global before the iteration
    *ADDWHAT.lock().unwrap() = what;
    for nam in names {
        addmatch(&nam, None);
    }
}

/// Hash-node → match adapter for scanhashtable callbacks.
/// Port of `addhnmatch()` from Src/Zle/compctl.c:2122.
///
/// Trivial wrapper: ignores `flags` and forwards the node name to
/// addmatch with `t=NULL`. Used by maketildelist's scanhashtable
/// invocation (c:2060).
pub(crate) fn addhnmatch(name: &str, _flags: i32) {
    addmatch(name, None);
}

/// Expand a string via prefork (parameter / arith / cmd-sub /
/// tilde / brace / glob), suppressing errors.
/// Port of `getreal()` from Src/Zle/compctl.c:2131.
///
/// C body builds a one-element LinkList, sets `noerrs=1`, runs
/// `prefork(l, 0, NULL)`, then returns the first element if the
/// list is non-empty and the first elem has content; else returns
/// the original string.
///
/// Rust: routes through `singsub` since that's the equivalent
/// "expand a single word with errors swallowed". Returns owned
/// String (vs C's heap-string-pointer).
pub(crate) fn getreal(str_in: &str) -> String {
    // C: c:2135 — save noerrs
    // C: c:2138-2139 — prefork the duplicated string
    // Routes through singsub when a VM/executor is available;
    // outside the VM (unit tests, direct calls) returns the input
    // unchanged. Direct port of the C "noerrs swallow" path.
    let result = crate::fusevm_bridge::try_with_executor(|exec| {
        let mut state = crate::ported::subst::SubstState::from_executor(exec);
        crate::ported::subst::singsub(str_in, &mut state)
    });
    // C: c:2141-2143 — non-empty + first char non-empty → use it.
    match result {
        Some(s) if !s.is_empty() => s,
        _ => str_in.to_string(),
    }
}

// (getreal port location; impl above already routes through singsub)
/// Read a directory and add files to the matches list.
/// Port of `gen_matches_files()` from Src/Zle/compctl.c:2153.
///
/// C signature: `void gen_matches_files(int dirs, int execs, int all)`.
/// Walks the directory at `prpre` (the expanded pre-cursor path
/// component), filtering each entry per:
///   dirs   → only directories
///   execs  → only executable files
///   all    → no filter (everything except `.`/`..` unless `all`)
/// Calls addmatch for each accepted entry.
///
/// Rust port reads `prpre` (PRPRE static if set; else current dir),
/// applies the same dirent-stat dispatch.
pub(crate) fn gen_matches_files(dirs: bool, execs: bool, all: bool) {
    let prpre = PRPRE.lock().unwrap().clone().unwrap_or_else(|| ".".to_string());
    let entries = match std::fs::read_dir(&prpre) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let name = match entry.file_name().into_string() {
            Ok(n) => n,
            Err(_) => continue,
        };
        // Skip `.`/`..` unless `all` is set
        if !all && (name == "." || name == "..") {
            continue;
        }
        // Hidden-file rule: leading `.` requires `all`.
        if !all && name.starts_with('.') {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if dirs && !meta.is_dir() {
            continue;
        }
        if execs {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = meta.permissions().mode();
                if mode & 0o111 == 0 || meta.is_dir() {
                    continue;
                }
            }
            #[cfg(not(unix))]
            { continue; }
        }
        addmatch(&name, None);
    }
}

/// Pre-cursor directory path (`prpre` global). Port of file-static
/// `char *prpre` at Src/Zle/compctl.c:1736 — the directory portion
/// of the path component the cursor is in, expanded for `opendir`.
/// Set by the completion driver before calling gen_matches_files.
static PRPRE: Mutex<Option<String>> = Mutex::new(None);

/// Find a node in a linked list by data-pointer equality.
/// Port of `findnode()` from Src/Zle/compctl.c:2287.
///
/// C signature: `LinkNode findnode(LinkList list, void *dat)` —
/// walks `list` looking for the node whose data pointer == `dat`.
/// Returns the matching node or NULL.
///
/// Rust generic over `T: PartialEq` — returns the index of the
/// matching element, or None.
pub(crate) fn findnode<T: PartialEq>(list: &[T], dat: &T) -> Option<usize> {
    list.iter().position(|x| x == dat)
}

/// `cdepth` recursion guard. Port of file-static `int cdepth = 0;`
/// at Src/Zle/compctl.c:2300.
static CDEPTH: Mutex<i32> = Mutex::new(0);

/// Maximum recursion depth — port of `MAX_CDEPTH 16` macro at
/// Src/Zle/compctl.c:2302. Prevents infinite recursion between
/// compctl-driven completion and the wrapper.
const MAX_CDEPTH: i32 = 16;

/// `ccont` continuation flags. Port of file-static `unsigned long
/// ccont;` at Src/Zle/compctl.c:1714. Bitmask of CC_CCCONT/etc.
/// controlling whether the dispatch loop continues to next compctl.
static CCONT: Mutex<u64> = Mutex::new(0);

/// Build the completion list — top-level dispatch.
/// Port of `makecomplistctl()` from Src/Zle/compctl.c:2305.
///
/// Entry point used by bin_compcall and the completion driver.
/// The C body:
///   1. Recursion guard (cdepth >= MAX_CDEPTH → return 0)
///   2. SWITCHHEAPS to the compheap (Rust uses the global allocator)
///   3. Save lots of state (cmdstr, clwords, instring, qipre/qisuf,
///      isuf, autoq, offs)
///   4. Set up new state from compquote / compqiprefix / compqisuffix /
///      compisuffix / compwords / compcurrent
///   5. Set incompfunc=2 (deeper-nested marker)
///   6. Call makecomplistglobal(str, !clwpos, COMP_COMPLETE, flags)
///   7. Restore state
///   8. cdepth-- and return
///
/// This Rust port keeps the recursion guard + flag dispatch + the
/// makecomplistglobal call. The compfunc state save/restore relies
/// on ZLE-tricky globals (clwords, etc.) that aren't ported here.
pub(crate) fn makecomplistctl(flags: i32) -> i32 {
    let mut cdepth = CDEPTH.lock().unwrap();
    if *cdepth == MAX_CDEPTH {                                // c:2311
        return 0;
    }
    *cdepth += 1;                                             // c:2314
    drop(cdepth);

    // C: c:2372 — bump incompfunc to 2 (recursion marker)
    let saved_incomp = *INCOMPFUNC.lock().unwrap();
    *INCOMPFUNC.lock().unwrap() = 2;

    // C: c:2373 — recurse to global dispatch
    let str_in = "";  // placeholder; real impl reads comp_str
    let ret = makecomplistglobal(str_in, false, comp_op::LIST as i32, flags);

    *INCOMPFUNC.lock().unwrap() = saved_incomp;
    *CDEPTH.lock().unwrap() -= 1;
    ret
}

/// Line-context dispatch — global completion entry.
/// Port of `makecomplistglobal()` from Src/Zle/compctl.c:2401.
///
/// Looks at `linwhat` (IN_ENV / IN_MATH / IN_COND / IN_REDIR / else)
/// and dispatches to the appropriate compctl spec:
///   IN_ENV    → cc_default (parameter values)
///   IN_MATH   → cc_dummy (params or assoc keys)
///   IN_COND   → cc_dummy with -o/-nt/-ot/-ef logic
///   IN_REDIR  → cc_default (redirections)
///   default   → makecomplistcmd (per-command lookup)
///
/// `linwhat` and friends live in zle_tricky.c. For the foundation,
/// we assume "default" (per-command lookup) which is the most
/// common path.
pub(crate) fn makecomplistglobal(os: &str, incmd: bool, _lst: i32, flags: i32) -> i32 {
    // C: c:2406 — reset ccont
    *CCONT.lock().unwrap() = cc_flags2::CCCONT;

    // C: c:2407 — clear cc_dummy.suffix
    if let Some(d) = CC_DUMMY.lock().unwrap().as_mut() {
        // Arc<CompCtl> can't mutate easily; re-assign a fresh one
        // with cleared suffix when needed. For now, a no-op.
        let _ = d;
    }

    // C: c:2409+ — linwhat dispatch. We don't have linwhat ported;
    // fall through to the default per-command path which is the
    // most common case.
    let _ = flags;
    makecomplistcmd(os, incmd, flags)
}

/// Per-command compctl lookup + dispatch.
/// Port of `makecomplistcmd()` from Src/Zle/compctl.c:2473.
///
/// Resolves the compctl for cmdstr by:
///   1. If !CFN_FIRST: run cc_first first; bail if !CC_CCCONT
///   2. Run pattern compctls (makecomplistpc); bail if !CC_CCCONT
///   3. If cmdstr starts with `=`, expand path
///   4. Lookup cmdstr in compctltab — try full name then trailing
///      pathname component (after remlpaths)
///   5. If incmd: use cc_compos
///   6. Else if no match: cc_default (unless CFN_DEFAULT)
///   7. Call makecomplistcc(cc, os, incmd)
pub(crate) fn makecomplistcmd(os: &str, incmd: bool, flags: i32) -> i32 {
    const CFN_FIRST: i32 = 1;
    const CFN_DEFAULT: i32 = 2;
    let mut ret: i32 = 0;

    // C: c:2482 — first try cc_first
    if (flags & CFN_FIRST) == 0 {
        if let Some(cc_first) = CC_FIRST.lock().unwrap().clone() {
            makecomplistcc(&cc_first, os, incmd);
            if (*CCONT.lock().unwrap() & cc_flags2::CCCONT) == 0 {
                return 0;
            }
        }
    }

    // C: c:2491 — pattern compctls
    let cmdstr = CMDSTR.lock().unwrap().clone();
    if cmdstr.is_some() {
        ret |= makecomplistpc(os, incmd);
        if (*CCONT.lock().unwrap() & cc_flags2::CCCONT) == 0 {
            return ret;
        }
    }

    // C: c:2509 — incmd path uses cc_compos
    let cc = if incmd {
        CC_COMPOS.lock().unwrap().clone()
    } else {
        // C: c:2511-2519 — lookup compctltab[cmdstr]
        let name = match &cmdstr {
            Some(s) => s.clone(),
            None => return ret,
        };
        let table = COMPCTL_TAB.lock().unwrap();
        let from_table = table.as_ref().and_then(|m| m.get(&name).cloned());
        drop(table);
        match from_table {
            Some(c) => Some(c),
            None => {
                if (flags & CFN_DEFAULT) != 0 {
                    return ret;
                }
                ret |= 1;
                CC_DEFAULT.lock().unwrap().clone()
            }
        }
    };
    if let Some(c) = cc {
        makecomplistcc(&c, os, incmd);
    }
    ret
}

/// `cmdstr` — current command word being completed.
/// Port of file-static `char *cmdstr` (zle_tricky.c). Set by the
/// completion driver before invoking makecomplistcmd.
static CMDSTR: Mutex<Option<String>> = Mutex::new(None);

/// Pattern compctl iteration — `compctl -p PAT cmd`.
/// Port of `makecomplistpc()` from Src/Zle/compctl.c:2529.
///
/// Walks the patcomps list, compiles each pattern, tries it against
/// cmdstr (and optionally against the resolved command path). On
/// match, runs makecomplistcc for that pattern's spec.
pub(crate) fn makecomplistpc(os: &str, incmd: bool) -> i32 {
    let mut ret: i32 = 0;
    let cmdstr = CMDSTR.lock().unwrap().clone();
    let cmd = match cmdstr {
        Some(s) => s,
        None => return 0,
    };

    let pats = PATCOMPS.lock().unwrap().clone();
    for (pat, cc) in &pats {
        // C: c:2542 — compile pattern, try match against cmdstr
        if crate::exec::ShellExecutor::glob_match_static(&cmd, pat) {
            makecomplistcc(cc, os, incmd);
            ret |= 2;
            if (*CCONT.lock().unwrap() & cc_flags2::CCCONT) == 0 {
                return ret;
            }
        }
    }
    ret
}

/// Per-compctl entry — track usage + dispatch the OR chain.
/// Port of `makecomplistcc()` from Src/Zle/compctl.c:2557.
///
/// Bumps refc on cc, adds it to ccused list, resets ccont, calls
/// makecomplistor. The ccused list lets later cleanup free all
/// compctls used during a single completion.
pub(crate) fn makecomplistcc(cc: &Arc<CompCtl>, s: &str, incmd: bool) {
    // C: c:2560 — refc++ (Arc handles this)
    let _ = cc.clone();

    // C: c:2562 — initialize ccused list
    let mut ccused = CCUSED.lock().unwrap();
    ccused.push(cc.clone());
    drop(ccused);

    // C: c:2565 — reset ccont
    *CCONT.lock().unwrap() = 0;

    // C: c:2567 — dispatch OR chain
    makecomplistor(cc, s, incmd, 0, 0);
}

/// `ccused` — per-completion list of compctls used. Port of
/// file-static `LinkList ccused` at Src/Zle/compctl.c:1702.
static CCUSED: Mutex<Vec<Arc<CompCtl>>> = Mutex::new(Vec::new());

/// Walk the [x]or chain of compctls.
/// Port of `makecomplistor()` from Src/Zle/compctl.c:2573.
///
/// C body:
///   - Loop over xors (cc->xor chain)
///   - For each, call makecomplistlist
///   - Track newly-added matches (mn diff)
///   - Stop based on ccont bits (CC_PATCONT, CC_DEFCONT, CC_XORCONT)
pub(crate) fn makecomplistor(cc: &Arc<CompCtl>, s: &str, incmd: bool, compadd: i32, sub: i32) {
    let mut current = cc.clone();
    loop {
        makecomplistlist(&current, s, incmd, compadd);
        // Walk to next xor
        match &current.xor {
            Some(next) => current = next.clone(),
            None => break,
        }
        let _ = sub;
    }
}

/// Top-level per-compctl dispatch.
/// Port of `makecomplistlist()` from Src/Zle/compctl.c:3081.
///
/// Routes to either makecomplistext (for -x extended conditions)
/// or makecomplistflags (for the regular flag-mask compctl).
pub(crate) fn makecomplistlist(cc: &Arc<CompCtl>, s: &str, incmd: bool, compadd: i32) {
    if cc.ext.is_some() {
        // C: c:3155 — extended -x conditions
        makecomplistext(cc, s, incmd);
    } else {
        // C: c:3499 — regular flag-driven completion
        makecomplistflags(cc, s, incmd, compadd);
    }
}

/// Extended (`-x`) completion list builder.
/// Port of `makecomplistext()` from Src/Zle/compctl.c:3155.
///
/// Walks cc.ext chain (the per-condition compctls), evaluates each
/// condition against the current line state, and dispatches to
/// makecomplistflags for the first matching condition's spec.
pub(crate) fn makecomplistext(occ: &Arc<CompCtl>, os: &str, incmd: bool) {
    // Walk the ext chain — each entry has a Compcond + a CompCtl.
    let mut current = occ.ext.clone();
    while let Some(cc) = current {
        // Evaluate the condition (port of c:2658 condition-eval loop).
        // For now, accept all conditions and run flags.
        if let Some(_cond) = &cc.cond {
            // TODO: full condition eval per cct::* dispatch.
            // For the foundation, treat conditions as always-true.
        }
        makecomplistflags(&cc, os, incmd, 0);
        current = cc.next.clone();
    }
}

// =================================================================
// zle_tricky.c state required by sep_comp_string and the
// completion-driver hooks. Ports of the file-statics in
// Src/Zle/zle_tricky.c that compctl reads/writes during the
// completion flow. Each is a `Mutex<...>` singleton matching the
// C global's name + type (translated to Rust idioms).
// =================================================================

/// `we` / `wb` — word end / begin positions (1-based byte offsets
/// into zlemetaline). Port of `int wb, we;` at Src/Zle/zle_tricky.c.
static WE: Mutex<i32> = Mutex::new(0);
static WB: Mutex<i32> = Mutex::new(0);

/// `zlemetacs` — cursor position (byte offset). Port of `int zlemetacs;`.
static ZLEMETACS: Mutex<i32> = Mutex::new(0);

/// `zlemetall` — line length in bytes. Port of `int zlemetall;`.
static ZLEMETALL: Mutex<i32> = Mutex::new(0);

/// `zlemetaline` — the actual line buffer. Port of `char *zlemetaline;`.
static ZLEMETALINE: Mutex<String> = Mutex::new(String::new());

/// `noerrs` / `noaliases` — lexer error/alias-suppression flags.
static NOERRS: Mutex<i32> = Mutex::new(0);
static NOALIASES: Mutex<i32> = Mutex::new(0);

/// `instring` — quoting context (QT_NONE/SINGLE/DOUBLE/DOLLARS/BACKSLASH/
/// BACKTICK). Port of `int instring;`. Mirrors zsh.h QT_* enum.
pub mod qt {
    pub const NONE: i32      = 0;  // unquoted
    pub const SINGLE: i32    = 1;  // '...'
    pub const DOUBLE: i32    = 2;  // "..."
    pub const DOLLARS: i32   = 3;  // $'...'
    pub const BACKSLASH: i32 = 4;  // \X escape
    pub const BACKTICK: i32  = 5;  // `...`
}
static INSTRING: Mutex<i32> = Mutex::new(qt::NONE);

/// `inbackt` — inside backtick command-substitution. Port of `int inbackt;`.
static INBACKT: Mutex<i32> = Mutex::new(0);

/// `autoq` — auto-quote chars to insert with completed match. Port of
/// `char *autoq;`.
static AUTOQ: Mutex<String> = Mutex::new(String::new());

/// `compqstack` — current quoting-context stack. Port of `char *compqstack;`.
static COMPQSTACK: Mutex<String> = Mutex::new(String::new());

/// `qipre` / `qisuf` — quoted ignored prefix/suffix from the
/// completion driver. Port of `char *qipre, *qisuf;`.
static QIPRE: Mutex<String> = Mutex::new(String::new());
static QISUF: Mutex<String> = Mutex::new(String::new());

/// `compqiprefix` / `compqisuffix` / `compisuffix` — completion-context
/// state from the user's compfunc. Port of those file-statics.
static COMPQIPREFIX: Mutex<String> = Mutex::new(String::new());
static COMPQISUFFIX: Mutex<String> = Mutex::new(String::new());
static COMPISUFFIX: Mutex<String> = Mutex::new(String::new());

/// `compwords` — current word array from the completion driver.
static COMPWORDS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static COMPCURRENT: Mutex<i32> = Mutex::new(0);

/// `clwords` / `clwsize` / `clwnum` / `clwpos` — current line word
/// array + sizes used by the completion code.
static CLWORDS: Mutex<Vec<String>> = Mutex::new(Vec::new());
static CLWSIZE: Mutex<i32> = Mutex::new(0);
static CLWNUM: Mutex<i32> = Mutex::new(0);
static CLWPOS: Mutex<i32> = Mutex::new(0);

/// `offs` — completion offset into the current word.
static OFFS: Mutex<i32> = Mutex::new(0);

/// `addedx` — non-zero while the dummy `x` cursor marker is in
/// the line being lexed.
static ADDEDX: Mutex<i32> = Mutex::new(0);

/// `lexflags` — lexer mode flags (LEXFLAGS_ZLE etc.). Port of
/// `int lexflags;` from Src/lex.c.
static LEXFLAGS: Mutex<i32> = Mutex::new(0);

/// LEXFLAGS_ZLE — the bit set during ZLE-driven completion lex.
/// Port of `LEXFLAGS_ZLE` from Src/zsh.h.
const LEXFLAGS_ZLE: i32 = 1 << 0;

/// `brange` / `erange` — `-l` word-range begin/end.
static BRANGE: Mutex<i32> = Mutex::new(0);
static ERANGE: Mutex<i32> = Mutex::new(0);

/// `linwhat` — line-context kind (IN_ENV/IN_MATH/IN_COND/IN_REDIR/0).
/// Port of `int linwhat;` from zle_tricky.c.
pub mod linwhat_kind {
    pub const NONE: i32      = 0;
    pub const IN_ENV: i32    = 1;
    pub const IN_MATH: i32   = 2;
    pub const IN_COND: i32   = 3;
    pub const IN_REDIR: i32  = 4;
}
static LINWHAT: Mutex<i32> = Mutex::new(0);

/// `linredir` — non-zero when completing inside a redirection.
static LINREDIR: Mutex<i32> = Mutex::new(0);

/// `insubscr` — non-zero inside an array subscript context.
static INSUBSCR: Mutex<i32> = Mutex::new(0);

/// Inull-token chars from Src/zsh.h. These are the byte values
/// the lexer uses to mark suppressed quoted-region boundaries
/// (Snull = single-quote, Dnull = double-quote, Bnull = backslash,
/// String/Qstring = `$`/`'$'` markers).
pub const SNULL: char  = '\u{9d}';  // Single-quote null
pub const DNULL: char  = '\u{9e}';  // Double-quote null
pub const BNULL: char  = '\u{9f}';  // Backslash null
pub const STRING_TOK: char  = '\u{85}';  // META-$
pub const QSTRING_TOK: char = '\u{84}';  // QSTRING (for $'...')

/// Test whether `c` is one of the inull token chars.
/// Port of `inull()` macro from Src/zsh.h — `c >= Pound && c <= LAST_NORMAL_TOK`
/// per the zsh range. We use the explicit set since the byte values
/// are stable across the codebase.
fn inull(c: char) -> bool {
    matches!(c, SNULL | DNULL | BNULL | STRING_TOK | QSTRING_TOK)
}

/// Separate the cursor word into prefix/word/suffix components.
/// Port of `sep_comp_string()` from Src/Zle/compctl.c:2806 (~225 lines).
///
/// C signature: `int sep_comp_string(char *ss, char *s, int noffs)`.
///
/// The function constructs a synthetic line of the form `ss + " " +
/// s[..noffs] + 'x' + s[noffs..]` and runs the lexer over it to
/// recover word boundaries with the cursor (the inserted 'x') in
/// view. Then adjusts wb/we/zlemetacs to reflect positions inside
/// the lexed word, accounting for inull markers. Pushes results
/// into clwords + cmdstr + qipre/qisuf and dispatches to
/// makecomplistcmd.
///
/// Faithful port:
///   - constructs the temp buffer per c:2827-2832
///   - applies rembslash if QT_BACKSLASH stack head (c:2833)
///   - state save/restore for instring/inbackt/noaliases/autoq (c:2810-2813)
///   - state save/restore for clwords/cmdstr/qipre/qisuf (c:2980-3023)
///   - inull/Bnull adjustment loop (c:2931-2952)
///   - nested makecomplistcmd dispatch (c:3006)
///
/// The actual `ctxtlex()` driver is replaced by Rust's `ZshLexer`
/// from parse/src/lex.rs — for this port we approximate by
/// splitting the temp string on whitespace + tracking the cursor
/// word. Full lexer-token reconstruction (LEXERR/STRING/ENDINPUT
/// handling for unbalanced quotes per c:2842-2855) is the
/// remaining gap; the foundation here handles plain-token cases
/// which cover the most common compctl flows.
pub(crate) fn sep_comp_string(ss: &str, s: &str, noffs: i32) -> i32 {
    // C: c:2810-2813 — save state to restore on exit
    let owe = *WE.lock().unwrap();
    let owb = *WB.lock().unwrap();
    let ocs = *ZLEMETACS.lock().unwrap();
    let oll = *ZLEMETALL.lock().unwrap();
    let ois = *INSTRING.lock().unwrap();
    let oib = *INBACKT.lock().unwrap();
    let ona = *NOALIASES.lock().unwrap();
    let ne = *NOERRS.lock().unwrap();
    let ol = ZLEMETALINE.lock().unwrap().clone();
    let oaq = AUTOQ.lock().unwrap().clone();

    let sl = ss.len() as i32;
    let mut got = false;
    let mut i = 0_i32;
    let mut cur: i32 = -1;
    let mut swb = 0_i32;
    let mut swe = 0_i32;
    let mut soffs = 0_i32;
    let mut ns: String = String::new();
    let mut foo: Vec<String> = Vec::new();

    // C: c:2823-2832 — build the temp buffer with cursor `x` marker.
    // tmp = ss + " " + s[..noffs] + 'x' + s[noffs..]
    *ADDEDX.lock().unwrap() = 1;
    *NOERRS.lock().unwrap() = 1;
    *LEXFLAGS.lock().unwrap() = LEXFLAGS_ZLE;
    let mut tmp = String::with_capacity(ss.len() + 3 + s.len());
    tmp.push_str(ss);
    tmp.push(' ');
    let s_chars: Vec<char> = s.chars().collect();
    let noffs_u = (noffs as usize).min(s_chars.len());
    let s_pre: String = s_chars[..noffs_u].iter().collect();
    let s_post: String = s_chars[noffs_u..].iter().collect();
    tmp.push_str(&s_pre);
    let scs_initial = sl + 1 + noffs;
    *ZLEMETACS.lock().unwrap() = scs_initial;
    let mut scs = scs_initial;
    tmp.push('x');
    tmp.push_str(&s_post);
    let tl = tmp.len() as i32;

    // C: c:2833 — apply rembslash if QT_BACKSLASH stack head
    let qstack_head = COMPQSTACK.lock().unwrap().chars().next().unwrap_or(qt::NONE as u8 as char);
    let remq = qstack_head as i32 == qt::BACKSLASH;
    if remq {
        // rembslash — strip backslashes
        let mut stripped = String::with_capacity(tmp.len());
        let mut chars = tmp.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\' {
                if let Some(&_nx) = chars.peek() {
                    // Skip backslash, keep next char
                    continue;
                }
            }
            stripped.push(c);
        }
        tmp = stripped;
    }

    // C: c:2835-2839 — push input, set zlemetaline
    *ZLEMETALINE.lock().unwrap() = tmp.clone();
    *ZLEMETALL.lock().unwrap() = tl - 1;
    *NOALIASES.lock().unwrap() = 1;

    // C: c:2840-2873 — lex loop. We approximate ctxtlex() with a
    // whitespace-tokenize + cursor-word detection. Real lexer
    // integration requires the parse crate's ZshLexer wired with
    // ZLE input-stack semantics.
    {
        let chars: Vec<char> = tmp.chars().collect();
        let mut t_start = 0_usize;
        let mut idx = 0_usize;
        let mut word_idx = 0_i32;
        while idx <= chars.len() {
            let at_end = idx == chars.len();
            let is_sep = !at_end && chars[idx] == ' ';
            if at_end || is_sep {
                if idx > t_start {
                    let token: String = chars[t_start..idx].iter().collect();
                    let abs_start = t_start as i32;
                    let abs_end = idx as i32;
                    foo.push(token.clone());
                    // C: c:2862-2871 — first time scs falls inside
                    // a token, that's the cursor word.
                    if !got && scs >= abs_start && scs <= abs_end {
                        got = true;
                        cur = word_idx;
                        swb = abs_start;
                        swe = abs_end;
                        soffs = scs - swb;
                        // C: chuck(p + soffs) — remove the dummy 'x'
                        let mut t = token.clone();
                        if (soffs as usize) < t.len() {
                            t.remove(soffs as usize);
                        }
                        ns = t;
                    }
                    word_idx += 1;
                }
                t_start = idx + 1;
            }
            if at_end { break; }
            idx += 1;
        }
        i = word_idx;
    }

    *NOALIASES.lock().unwrap() = ona;
    *NOERRS.lock().unwrap() = ne;
    *WB.lock().unwrap() = owb;
    *WE.lock().unwrap() = owe;
    *ZLEMETACS.lock().unwrap() = ocs;
    *ZLEMETALINE.lock().unwrap() = ol;
    *ZLEMETALL.lock().unwrap() = oll;

    // C: c:2885 — bail if no cursor word found
    if cur < 0 || i < 1 {
        return 1;
    }

    // C: c:2887-2896 — check_param dispatch (params + Snull/Dnull
    // marker conversion). Skipped pending check_param port.

    // C: c:2898-2929 — quote-prefix detection. Examine ns[0] for
    // SNULL/DNULL/STRING_TOK/QSTRING_TOK and adjust instring + autoq.
    let ts = ns.clone();
    let _ = ts.clone();
    let first_char = ns.chars().next();
    let is_quoted_open = matches!(
        first_char,
        Some(SNULL) | Some(DNULL)
    ) || (matches!(first_char, Some(STRING_TOK) | Some(QSTRING_TOK))
        && ns.chars().nth(1) == Some(SNULL));

    if is_quoted_open {
        let new_instring = match first_char {
            Some(SNULL) => qt::SINGLE,
            Some(DNULL) => qt::DOUBLE,
            _ => qt::DOLLARS,
        };
        *INSTRING.lock().unwrap() = new_instring;
        *INBACKT.lock().unwrap() = 0;
        swb += 1;
        // C: c:2921 — if the closing quote-marker matches at end, swe--
        if let (Some(first), Some(last)) = (ns.chars().next(), ns.chars().last()) {
            if first == last && ns.len() >= 2 {
                swe -= 1;
            }
        }
        // C: c:2925 — autoq from compqstack[1] and multiquote
        let qstack = COMPQSTACK.lock().unwrap().clone();
        if qstack.len() >= 2 {
            *AUTOQ.lock().unwrap() = String::new();
        } else {
            *AUTOQ.lock().unwrap() = ts.clone();
        }
    } else {
        *INSTRING.lock().unwrap() = qt::NONE;
        *AUTOQ.lock().unwrap() = String::new();
    }

    // C: c:2931-2952 — inull walk: drop inull markers from ns,
    // adjusting scs/soffs/swb as we go.
    let mut ns_chars: Vec<char> = ns.chars().collect();
    let mut p_idx = 0_usize;
    let mut walk_i = swb;
    while p_idx < ns_chars.len() {
        let c = ns_chars[p_idx];
        if inull(c) {
            if walk_i < scs {
                soffs -= 1;
                if remq && c == BNULL && p_idx + 1 < ns_chars.len() {
                    swb -= 2;
                }
            }
            let next = ns_chars.get(p_idx + 1).copied();
            if next.is_some() || c != BNULL {
                if c == BNULL {
                    if scs == walk_i + 1 {
                        scs += 1;
                        soffs += 1;
                    }
                } else if scs > walk_i {
                    scs -= 1;
                    walk_i -= 1;  // C: `scs > i--`
                }
            } else if scs == swe {
                scs -= 1;
            }
            ns_chars.remove(p_idx);
            // Don't advance p_idx — re-check the new char at p_idx
            // (matches C's `chuck(p--); p++;` next-iter increment).
            walk_i -= 1;
        } else {
            p_idx += 1;
            walk_i += 1;
        }
    }
    ns = ns_chars.iter().collect();

    // C: c:2961-2974 — build qp/qs from ss + qipre/qisuf
    let qipre_val = QIPRE.lock().unwrap().clone();
    let qisuf_val = QISUF.lock().unwrap().clone();
    let qp = format!("{}{}", qipre_val, &s[..((swb - sl - 1).max(0) as usize).min(s.len())]);
    if swe < swb {
        swe = swb;
    }
    swe -= sl + 1;
    let s_len = s.len() as i32;
    if swe > s_len {
        swe = s_len;
        if (ns.len() as i32) > swe - swb + 1 {
            ns.truncate((swe - swb + 1) as usize);
        }
    }
    let qs_start = (swe.max(0) as usize).min(s.len());
    let qs = format!("{}{}", &s[qs_start..], qisuf_val);
    let s_chars_len = ns.len() as i32;
    if soffs > s_chars_len {
        soffs = s_chars_len;
    }

    // C: c:2980-3023 — state save/restore + nested makecomplistcmd
    let ow = CLWORDS.lock().unwrap().clone();
    let os = CMDSTR.lock().unwrap().clone();
    let oqp = QIPRE.lock().unwrap().clone();
    let oqs = QISUF.lock().unwrap().clone();
    let oqst = COMPQSTACK.lock().unwrap().clone();
    let olws = *CLWSIZE.lock().unwrap();
    let olwn = *CLWNUM.lock().unwrap();
    let olwp = *CLWPOS.lock().unwrap();
    let obr = *BRANGE.lock().unwrap();
    let oer = *ERANGE.lock().unwrap();
    let oof = *OFFS.lock().unwrap();
    let occ = *CCONT.lock().unwrap();

    // C: c:2986-2989 — push current quote char onto compqstack
    let new_quote_char = if *INSTRING.lock().unwrap() != qt::NONE {
        char::from_u32(*INSTRING.lock().unwrap() as u32).unwrap_or('\\')
    } else {
        char::from_u32(qt::BACKSLASH as u32).unwrap_or('\\')
    };
    let mut new_compqstack = String::new();
    new_compqstack.push(new_quote_char);
    new_compqstack.push_str(&oqst);
    *COMPQSTACK.lock().unwrap() = new_compqstack;

    // C: c:2991-2997 — install foo into clwords
    *CLWSIZE.lock().unwrap() = foo.len() as i32;
    *CLWNUM.lock().unwrap() = foo.len() as i32;
    *CLWORDS.lock().unwrap() = foo.clone();
    *CLWPOS.lock().unwrap() = cur;
    *CMDSTR.lock().unwrap() = foo.first().cloned();
    *BRANGE.lock().unwrap() = 0;
    *ERANGE.lock().unwrap() = (foo.len() as i32) - 1;
    *QIPRE.lock().unwrap() = qp;
    *QISUF.lock().unwrap() = qs;
    *OFFS.lock().unwrap() = soffs;
    *CCONT.lock().unwrap() = cc_flags2::CCCONT;

    // C: c:3006 — nested dispatch
    const CFN_FIRST: i32 = 1;
    let _ = makecomplistcmd(&ns, cur == 0, CFN_FIRST);

    *CCONT.lock().unwrap() = occ;
    *OFFS.lock().unwrap() = oof;
    *CMDSTR.lock().unwrap() = os;
    *CLWORDS.lock().unwrap() = ow;
    *CLWSIZE.lock().unwrap() = olws;
    *CLWNUM.lock().unwrap() = olwn;
    *CLWPOS.lock().unwrap() = olwp;
    *BRANGE.lock().unwrap() = obr;
    *ERANGE.lock().unwrap() = oer;
    *QIPRE.lock().unwrap() = oqp;
    *QISUF.lock().unwrap() = oqs;
    *COMPQSTACK.lock().unwrap() = oqst;

    *AUTOQ.lock().unwrap() = oaq;
    *INSTRING.lock().unwrap() = ois;
    *INBACKT.lock().unwrap() = oib;

    0
}

/// The flag-driven completion-list builder — workhorse fn.
/// Port of `makecomplistflags()` from Src/Zle/compctl.c:3499 (~500 lines).
///
/// Walks the bits of cc.mask and cc.mask2, dispatching per CC_* bit
/// to the matching generator:
///   CC_FILES     → gen_matches_files (regular files)
///   CC_DIRS      → gen_matches_files(dirs=true)
///   CC_COMMPATH  → command-path completion
///   CC_OPTIONS   → option completion
///   CC_VARS      → dumphashtable(paramtab, CC_VARS)
///   CC_BINDINGS  → bindings (zle widgets)
///   CC_ARRAYS    → param table filtered to PM_ARRAY
///   CC_INTVARS   → param table filtered to PM_INTEGER
///   CC_SHFUNCS   → shfunctab
///   CC_PARAMS    → paramtab non-exported
///   CC_ENVVARS   → paramtab PM_EXPORTED
///   CC_JOBS / CC_RUNNING / CC_STOPPED → job table filters
///   CC_BUILTINS  → builtintab
///   CC_USERS     → /etc/passwd users (or named-dir filltable)
///   CC_DISCMDS / CC_EXCMDS → cmdnamtab filtered by DISABLED bit
///   CC_RESWDS    → reserved-word table
///   CC_NAMED     → named-directory table
///   CC_DIRS      → directory matches
///   ... and more
///
/// Plus arg-taking flags:
///   cc.glob   → globlist expansion
///   cc.str_expansion → string-arg expansion via singsub
///   cc.func   → call user function (compctl -K)
///   cc.keyvar → read array variable for matches
///   cc.hpat   → history-pattern matches
///
/// This stub records the dispatch entry so call sites can wire to
/// it; per-bit generators land per-bit in follow-ups.
pub(crate) fn makecomplistflags(cc: &Arc<CompCtl>, s: &str, _incmd: bool, _compadd: i32) {
    let _ = (cc, s);
    // Set ccont per cc.mask2 — c:3499 loop init reads CC_CCCONT
    // from mask2 to determine dispatch continuation.
    *CCONT.lock().unwrap() = cc.mask2;

    // CC_FILES — c:3650+ in real impl
    if (cc.mask & cc_flags::FILES) != 0 {
        *ADDWHAT.lock().unwrap() = addwhat_kind::FILES;
        gen_matches_files(false, false, false);
    }
    // CC_DIRS — c:3680
    if (cc.mask & cc_flags::DIRS) != 0 {
        *ADDWHAT.lock().unwrap() = addwhat_kind::FILES;
        gen_matches_files(true, false, false);
    }
    // CC_NAMED — c:3742
    if (cc.mask & cc_flags::NAMED) != 0 {
        *ADDWHAT.lock().unwrap() = addwhat_kind::FILES_OTHER;
        maketildelist();
    }
    // Per-CC_* arms beyond these (CC_VARS, CC_SHFUNCS, etc.) need
    // hashtable iteration ports — TODO when those ports land.

    // cc.func (compctl -K) — call user function for matches.
    // Skipped pending function-dispatch wiring.

    // cc.glob — globlist expansion. Skipped pending glob-port use.

    // cc.str_expansion (-s) — call singsub on the string.
    if let Some(s) = &cc.str_expansion {
        let expanded = getreal(s);
        // Push as a single match with addwhat=GLOB_EXPAND
        *ADDWHAT.lock().unwrap() = addwhat_kind::GLOB_EXPAND;
        addmatch(&expanded, None);
    }
}

// =================================================================
// Module boot/cleanup hooks — port of compctl.c:4000+
// =================================================================

/// Storage for the special compctl targets — `cc_compos` (command
/// completion), `cc_default` (default completion), `cc_first`
/// (first completion). Port of the file-static C declarations at
/// Src/Zle/compctl.c:41 — `struct compctl cc_compos, cc_default,
/// cc_first, cc_dummy;`. setup_ initializes the masks; tests +
/// real-completion paths read them.
pub(crate) static CC_COMPOS: Mutex<Option<Arc<CompCtl>>> = Mutex::new(None);
pub(crate) static CC_DEFAULT: Mutex<Option<Arc<CompCtl>>> = Mutex::new(None);
pub(crate) static CC_FIRST: Mutex<Option<Arc<CompCtl>>> = Mutex::new(None);
pub(crate) static CC_DUMMY: Mutex<Option<Arc<CompCtl>>> = Mutex::new(None);

/// Last-used compctl tracking list. Port of `LinkList lastccused`
/// at Src/Zle/compctl.c:1702. setup_ initializes to empty; finish_
/// frees its contents.
static LASTCCUSED: Mutex<Vec<Arc<CompCtl>>> = Mutex::new(Vec::new());

/// Pointer to compctlread (vs fallback_compctlread). Port of the
/// `CompctlReadFn compctlreadptr` indirect dispatch at
/// Src/Modules/zle/compctl.c:4016. setup_ installs this; finish_
/// restores the fallback.
static COMPCTLREAD_INSTALLED: Mutex<bool> = Mutex::new(false);

/// Setup hook — port of `setup_()` from Src/Zle/compctl.c:4013.
///
/// Wires `compctlreadptr` to compctlread, creates the compctltab,
/// initializes the special targets:
///   cc_compos.mask  = CC_COMMPATH
///   cc_default.refc = 10000  (sentinel "never free")
///   cc_default.mask = CC_FILES
///   cc_first.refc   = 10000
///   cc_first.mask2  = CC_CCCONT
/// Clears lastccused.
pub(crate) fn setup_() -> i32 {
    *COMPCTLREAD_INSTALLED.lock().unwrap() = true;
    createcompctltable();
    *CC_COMPOS.lock().unwrap() = Some(Arc::new(CompCtl {
        mask: cc_flags::COMMPATH,                            // c:4018
        ..Default::default()
    }));
    *CC_DEFAULT.lock().unwrap() = Some(Arc::new(CompCtl {
        refc: 10000,                                          // c:4020
        mask: cc_flags::FILES,                                // c:4021
        ..Default::default()
    }));
    *CC_FIRST.lock().unwrap() = Some(Arc::new(CompCtl {
        refc: 10000,                                          // c:4023
        mask2: cc_flags2::CCCONT,                             // c:4025
        ..Default::default()
    }));
    *LASTCCUSED.lock().unwrap() = Vec::new();                 // c:4027
    0
}

/// Features hook — port of `features_()` from Src/Zle/compctl.c:4033.
///
/// Returns the list of feature strings the module exposes. zsh C
/// uses `featuresarray(m, &module_features)` which reads
/// `module_features.bn_size` (line 4005 — 2 builtins: compctl,
/// compcall). Rust returns the explicit list.
pub(crate) fn features_() -> Vec<String> {
    vec!["b:compctl".to_string(), "b:compcall".to_string()]
}

/// Enables hook — port of `enables_()` from Src/Zle/compctl.c:4041.
///
/// C delegates to `handlefeatures(m, &module_features, enables)`
/// which writes the per-feature enable bits to `*enables`. Rust
/// returns a per-feature bool vector — entries currently default
/// to enabled (1). Wiring to the module-load runtime is a separate
/// concern.
pub(crate) fn enables_() -> Vec<i32> {
    vec![1, 1]
}

/// Boot hook — port of `boot_()` from Src/Zle/compctl.c:4048.
///
/// Registers the two completion-driver hooks via
/// `addhookfunc("compctl_make", ccmakehookfn)` and
/// `addhookfunc("compctl_cleanup", cccleanuphookfn)`. Rust hooks
/// dispatch via the same names; the actual hook registry is in
/// src/ported/module.rs.
pub(crate) fn boot_() -> i32 {
    // C: c:4051-4052 — addhookfunc calls. zshrs's hook registry
    // would be wired via crate::ported::module — for the C-source
    // faithful port we keep the names + intent visible here.
    0
}

/// Cleanup hook — port of `cleanup_()` from Src/Zle/compctl.c:4057.
///
/// Reverses boot_: removes the two hooks, then disables features
/// via `setfeatureenables(m, &module_features, NULL)`.
pub(crate) fn cleanup_() -> i32 {
    // C: c:4060-4062 — deletehookfunc + setfeatureenables.
    0
}

/// Finish hook — port of `finish_()` from Src/Zle/compctl.c:4066.
///
/// Tears down the compctltab hash table, frees lastccused, restores
/// `compctlreadptr` to the fallback. Rust drops the table on Mutex
/// reset; lastccused frees via Vec::clear; compctlreadptr is the
/// COMPCTLREAD_INSTALLED bool.
pub(crate) fn finish_() -> i32 {
    *COMPCTL_TAB.lock().unwrap() = None;                      // c:4069 deletehashtable
    LASTCCUSED.lock().unwrap().clear();                       // c:4071-4072 freelinklist
    *COMPCTLREAD_INSTALLED.lock().unwrap() = false;           // c:4074
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
    fn setup_initializes_special_targets_and_table() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        setup_();
        // cc_compos has CC_COMMPATH set
        let cc_compos = CC_COMPOS.lock().unwrap().clone();
        assert!(cc_compos.is_some());
        assert_eq!(cc_compos.unwrap().mask, cc_flags::COMMPATH);
        // cc_default has CC_FILES + refc=10000 sentinel
        let cc_default = CC_DEFAULT.lock().unwrap().clone();
        assert!(cc_default.is_some());
        let cc_default = cc_default.unwrap();
        assert_eq!(cc_default.mask, cc_flags::FILES);
        assert_eq!(cc_default.refc, 10000);
        // cc_first has CC_CCCONT in mask2
        let cc_first = CC_FIRST.lock().unwrap().clone();
        assert!(cc_first.is_some());
        assert_eq!(cc_first.unwrap().mask2, cc_flags2::CCCONT);
        // table exists
        assert!(COMPCTL_TAB.lock().unwrap().is_some());
        // compctlread installed
        assert!(*COMPCTLREAD_INSTALLED.lock().unwrap());
    }

    #[test]
    fn finish_tears_down_state() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        setup_();
        finish_();
        // Table cleared
        assert!(COMPCTL_TAB.lock().unwrap().is_none());
        // compctlread restored
        assert!(!*COMPCTLREAD_INSTALLED.lock().unwrap());
        // lastccused cleared
        assert_eq!(LASTCCUSED.lock().unwrap().len(), 0);
    }

    #[test]
    fn features_returns_two_builtins() {
        let f = features_();
        assert_eq!(f, vec!["b:compctl".to_string(), "b:compcall".to_string()]);
    }

    #[test]
    fn enables_returns_two_enabled_bits() {
        let e = enables_();
        assert_eq!(e, vec![1, 1]);
    }

    #[test]
    fn bin_compcall_outside_compfunc_errors() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        *INCOMPFUNC.lock().unwrap() = 0;
        let r = bin_compcall("compcall", &[]);
        assert_eq!(r, 1);
    }

    #[test]
    fn bin_compcall_inside_compfunc_succeeds() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        *INCOMPFUNC.lock().unwrap() = 1;
        let r = bin_compcall("compcall", &["-T".to_string()]);
        assert_eq!(r, 0);
        // Reset
        *INCOMPFUNC.lock().unwrap() = 0;
    }

    #[test]
    fn compctlread_outside_compctl_func_errors() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        *INCOMPCTLFUNC.lock().unwrap() = false;
        let r = compctlread("compctlread", &[]);
        assert_eq!(r, 1);
    }

    #[test]
    fn cccleanuphookfn_returns_zero() {
        // Trivial — no state to verify, just that it doesn't panic.
        assert_eq!(cccleanuphookfn(()), 0);
    }

    #[test]
    fn addwhat_kind_constants_match_c_compctl() {
        assert_eq!(addwhat_kind::FILES_OTHER, -1);
        assert_eq!(addwhat_kind::UNQUOTED, -2);
        assert_eq!(addwhat_kind::FILES, -5);
        assert_eq!(addwhat_kind::PARAM, -9);
    }

    #[test]
    fn addmatch_accepts_files_kind() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        MATCH_LIST.lock().unwrap().clear();
        *ADDWHAT.lock().unwrap() = addwhat_kind::FILES;
        addmatch("foo.txt", None);
        addmatch("bar.txt", None);
        let m = MATCH_LIST.lock().unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m[0], "foo.txt");
    }

    #[test]
    fn addmatch_accepts_param_kind() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        MATCH_LIST.lock().unwrap().clear();
        *ADDWHAT.lock().unwrap() = addwhat_kind::PARAM;
        addmatch("HOME", None);
        let m = MATCH_LIST.lock().unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(m[0], "HOME");
    }

    #[test]
    fn addmatch_accepts_cc_files_positive_mask() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        MATCH_LIST.lock().unwrap().clear();
        *ADDWHAT.lock().unwrap() = cc_flags::FILES as i32;
        addmatch("foo", None);
        let m = MATCH_LIST.lock().unwrap();
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn getcpat_finds_first_substring() {
        // Search "abcabc" for "bc" first occurrence → position 3
        // (1-based, points past the matched substring).
        let r = getcpat("abcabc", 1, "bc", 0);
        assert_eq!(r, 3);
    }

    #[test]
    fn getcpat_finds_second_substring() {
        // Search "abcabc" for the 2nd "bc" → position 6.
        let r = getcpat("abcabc", 2, "bc", 0);
        assert_eq!(r, 6);
    }

    #[test]
    fn getcpat_negative_index_searches_backward() {
        // Backward search "abcabc" for last "bc" → position 5.
        let r = getcpat("abcabc", -1, "bc", 0);
        assert!(r >= 0, "should find match (got {})", r);
    }

    #[test]
    fn getcpat_class_mode_matches_any_char_in_set() {
        // Search "abcdef" for any of {b, d, f} — class mode.
        // First match at index 1 (b).
        let r = getcpat("abcdef", 1, "bdf", 1);
        assert_eq!(r, 2);  // 1-based position of 'b'
    }

    #[test]
    fn getcpat_not_found_returns_negative_one() {
        let r = getcpat("hello", 1, "xyz", 0);
        assert_eq!(r, -1);
    }

    #[test]
    fn getcpat_strips_backslashes_in_pattern() {
        // `\$` in pattern should be treated as literal `$`.
        let r = getcpat("foo$bar", 1, "\\$", 0);
        assert_eq!(r, 4);  // 1-based pos right after the `$`
    }

    #[test]
    fn dumphashtable_calls_addmatch_per_entry() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        MATCH_LIST.lock().unwrap().clear();
        let entries = vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()];
        dumphashtable(entries, addwhat_kind::FILES);
        let m = MATCH_LIST.lock().unwrap();
        assert_eq!(m.len(), 3);
    }

    #[test]
    fn addhnmatch_forwards_to_addmatch() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        MATCH_LIST.lock().unwrap().clear();
        *ADDWHAT.lock().unwrap() = addwhat_kind::FILES;
        addhnmatch("xyz", 0);
        let m = MATCH_LIST.lock().unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(m[0], "xyz");
    }

    #[test]
    fn makecomplistctl_recursion_guard() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Force depth to MAX
        *CDEPTH.lock().unwrap() = MAX_CDEPTH;
        let r = makecomplistctl(0);
        assert_eq!(r, 0);
        // Reset for other tests.
        *CDEPTH.lock().unwrap() = 0;
    }

    #[test]
    fn makecomplistflags_cc_files_invokes_gen_matches() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        MATCH_LIST.lock().unwrap().clear();
        // Set prpre to a known dir we can read.
        *PRPRE.lock().unwrap() = Some(".".to_string());
        let cc = Arc::new(CompCtl {
            mask: cc_flags::FILES,
            ..Default::default()
        });
        makecomplistflags(&cc, "", false, 0);
        // Should have at least picked up Cargo.toml or similar from pwd.
        let m = MATCH_LIST.lock().unwrap();
        assert!(!m.is_empty(), "expected file matches in pwd");
    }

    #[test]
    fn makecomplistflags_cc_str_expansion_emits_one_match() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        MATCH_LIST.lock().unwrap().clear();
        let cc = Arc::new(CompCtl {
            str_expansion: Some("hardcoded".to_string()),
            ..Default::default()
        });
        makecomplistflags(&cc, "", false, 0);
        let m = MATCH_LIST.lock().unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(m[0], "hardcoded");
    }

    #[test]
    fn makecomplistor_walks_xor_chain() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        MATCH_LIST.lock().unwrap().clear();
        // Build cc1 with str "first", xor → cc2 with str "second"
        let cc2 = Arc::new(CompCtl {
            str_expansion: Some("second".to_string()),
            ..Default::default()
        });
        let cc1 = Arc::new(CompCtl {
            str_expansion: Some("first".to_string()),
            xor: Some(cc2),
            ..Default::default()
        });
        makecomplistor(&cc1, "", false, 0, 0);
        let m = MATCH_LIST.lock().unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(m[0], "first");
        assert_eq!(m[1], "second");
    }

    #[test]
    fn makecomplistcc_pushes_to_ccused() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        CCUSED.lock().unwrap().clear();
        let cc = Arc::new(CompCtl::default());
        makecomplistcc(&cc, "", false);
        let used = CCUSED.lock().unwrap();
        assert_eq!(used.len(), 1);
    }

    #[test]
    fn makecomplistpc_iterates_patcomps() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Verify makecomplistpc returns 0 when cmdstr is unset
        // (its early-bail path) — full pattern-match test requires
        // VM context for glob_match_static.
        *CMDSTR.lock().unwrap() = None;
        let r = makecomplistpc("", false);
        assert_eq!(r, 0);
    }

    #[test]
    fn findnode_returns_index_of_match() {
        let list = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(findnode(&list, &"b".to_string()), Some(1));
        assert_eq!(findnode(&list, &"z".to_string()), None);
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

    #[test]
    fn sep_comp_string_returns_zero_or_one() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // C compctl.c:2806-3030 contract — sep_comp_string only returns
        // 0 (success / dispatched) or 1 (bail, no cursor word).
        let r = sep_comp_string("", "", 0);
        assert!(r == 0 || r == 1, "expected 0 or 1, got {}", r);
    }

    #[test]
    fn sep_comp_string_round_trips_zle_state() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Pre-set zle_tricky.c globals; sep_comp_string must restore them
        // on exit (C compctl.c:2810-2813 save / 2941-2950 restore).
        *WE.lock().unwrap() = 42;
        *WB.lock().unwrap() = 7;
        *ZLEMETACS.lock().unwrap() = 11;
        *ZLEMETALL.lock().unwrap() = 99;
        *INSTRING.lock().unwrap() = qt::DOUBLE;
        *INBACKT.lock().unwrap() = 1;
        *NOALIASES.lock().unwrap() = 1;
        *NOERRS.lock().unwrap() = 0;
        *ZLEMETALINE.lock().unwrap() = "hello".to_string();
        *AUTOQ.lock().unwrap() = "Q".to_string();

        let _ = sep_comp_string("", "x", 0);

        assert_eq!(*WE.lock().unwrap(), 42);
        assert_eq!(*WB.lock().unwrap(), 7);
        assert_eq!(*ZLEMETACS.lock().unwrap(), 11);
        assert_eq!(*ZLEMETALL.lock().unwrap(), 99);
        assert_eq!(*INSTRING.lock().unwrap(), qt::DOUBLE);
        assert_eq!(*INBACKT.lock().unwrap(), 1);
        assert_eq!(*NOALIASES.lock().unwrap(), 1);
        assert_eq!(*NOERRS.lock().unwrap(), 0);
        assert_eq!(*ZLEMETALINE.lock().unwrap(), "hello");
        assert_eq!(*AUTOQ.lock().unwrap(), "Q");
    }

    #[test]
    fn inull_recognises_marker_chars() {
        // C compctl.c:2917 — INULL macro recognises SNULL/DNULL/BNULL
        // plus String/Qstring tokens for inull-walk.
        assert!(inull(SNULL));
        assert!(inull(DNULL));
        assert!(inull(BNULL));
        assert!(inull(STRING_TOK));
        assert!(inull(QSTRING_TOK));
        assert!(!inull('a'));
        assert!(!inull(' '));
    }

    #[test]
    fn qt_constants_match_c_compctl() {
        // C compctl.c:2902-2922 — instring values for sep_comp_string
        // quote-prefix detection.
        assert_eq!(qt::NONE, 0);
        assert_eq!(qt::SINGLE, 1);
        assert_eq!(qt::DOUBLE, 2);
        assert_eq!(qt::DOLLARS, 3);
        assert_eq!(qt::BACKSLASH, 4);
        assert_eq!(qt::BACKTICK, 5);
    }
}
