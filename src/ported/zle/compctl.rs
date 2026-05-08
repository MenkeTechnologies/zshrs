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

/// Get a compctl from arg vector. Stub for the main compctl-spec parser.
/// Port of `get_compctl()` from Src/Zle/compctl.c:382 (~600 lines).
/// The C version walks `argv` letter-by-letter applying flag bits to
/// `cc->mask`/`cc->mask2` and capturing the string args. To be ported
/// incrementally — start with the simple flag arms (`f`, `c`, `b`,
/// etc.) and add the arg-taking ones (`-K`, `-x`, `-M`) as they're
/// exercised by tests.
pub(crate) fn get_compctl(
    _name: &str,
    _av: &mut Vec<String>,
    _cc: &mut CompCtl,
    _first: bool,
    _isdef: bool,
    _cl: i32,
) -> i32 {
    0
}

/// Get an extended compctl (`-x` form).
/// Port of `get_xcompctl()` from Src/Zle/compctl.c:1025.
pub(crate) fn get_xcompctl(
    _name: &str,
    _av: &mut Vec<String>,
    _cc: &mut CompCtl,
    _isdef: bool,
) -> i32 {
    0
}

/// Assign a compctl to a name.
/// Port of `cc_assign()` from Src/Zle/compctl.c:1230. The C version
/// installs a CompCtl into the hash table, freeing any prior entry
/// (or merging in `reass` mode).
pub(crate) fn cc_assign(name: &str, cct: Arc<CompCtl>, _reass: bool) {
    let mut g = COMPCTL_TAB.lock().unwrap();
    if let Some(map) = g.as_mut() {
        map.insert(name.to_string(), cct);
    }
}

/// Reassign a compctl (compose with existing).
/// Port of `cc_reassign()` from Src/Zle/compctl.c:1262 — used when the
/// `+` operator chains a new spec onto an existing one.
pub(crate) fn cc_reassign(_cc: Arc<CompCtl>) -> Arc<CompCtl> {
    Arc::new(CompCtl::default())
}

/// Pattern-name dispatch — `compctl -p PAT`.
/// Port of `compctl_name_pat()` from Src/Zle/compctl.c:1278. Walks
/// the patcomps linked list looking for an entry matching the
/// pattern.
pub(crate) fn compctl_name_pat(_p: &[String]) -> Option<Arc<CompCtl>> {
    None
}

/// Delete a pattern compctl.
/// Port of `delpatcomp()` from Src/Zle/compctl.c:1297.
pub(crate) fn delpatcomp(n: &str) {
    let mut p = PATCOMPS.lock().unwrap();
    p.retain(|(pat, _)| pat != n);
}

/// Process the parsed compctl into the table.
/// Port of `compctl_process_cc()` from Src/Zle/compctl.c:1314 — the
/// post-parse step that installs the spec via cc_assign and handles
/// the `-D`/`-T`/`-C`/`-p` special targets.
pub(crate) fn compctl_process_cc(_s: &[String], _cc: Arc<CompCtl>) -> i32 {
    0
}

/// Print a single compctl spec.
/// Port of `printcompctl()` from Src/Zle/compctl.c:1380 — emits the
/// `compctl -...` re-runnable text for `compctl -L` listing.
pub(crate) fn printcompctl(
    _name: &str,
    _cc: &CompCtl,
    _printflags: i32,
    _ispat: bool,
) {
}

/// Print a compctl hash node.
/// Port of `printcompctlp()` from Src/Zle/compctl.c:1592 — hash-table
/// callback that calls printcompctl.
pub(crate) fn printcompctlp(name: &str, cc: &CompCtl, printflags: i32) {
    printcompctl(name, cc, printflags, false);
}

/// `compctl` builtin entry point.
/// Port of `bin_compctl()` from Src/Zle/compctl.c:1605 (~140 lines).
/// Dispatches based on flags: `-L` lists, `-D` sets default,
/// `-T` sets first, `-C` sets command, `-p` adds pattern, `-r` removes,
/// otherwise installs/updates per-name.
pub(crate) fn bin_compctl(_name: &str, _argv: &[String]) -> i32 {
    0
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

    #[test]
    fn createcompctltable_initializes_table() {
        createcompctltable();
        let g = COMPCTL_TAB.lock().unwrap();
        assert!(g.is_some());
        assert_eq!(g.as_ref().unwrap().len(), 0);
    }

    #[test]
    fn cc_assign_inserts_into_table() {
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
}
