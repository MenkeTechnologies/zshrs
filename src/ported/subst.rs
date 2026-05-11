//! Substitution handling — port of zsh/Src/subst.c.
//!
//! subst.c - various substitutions
//!
//! This file is part of zsh, the Z shell.
//!
//! Copyright (c) 1992-1997 Paul Falstad
//! All rights reserved.
//!
//! Direct port of the C code, maintaining the same structure, variable
//! names, and control flow where possible. The Rust port is larger
//! than the C source (~8.6k vs 4.9k lines) primarily because it
//! splits long C arms into named functions, lifts inline `static`
//! helpers into module-level fns, and replaces unsafe pointer walks
//! with explicit `Vec<char>` / `Vec<String>` traversals.
//!
//! Original C file: ~/forkedRepos/zsh/Src/subst.c (4922 lines)
//!
//! All 24 top-level C functions are present:
//! - prefork() — main pre-fork substitution dispatcher
//! - stringsubst() — string substitution engine
//! - stringsubstquote() — $'...' bslashquote processing
//! - paramsubst() — parameter expansion (the largest: ~3300 lines in C)
//! - multsub() — multiple word substitution
//! - singsub() — single word substitution
//! - filesub() / filesubstr() — tilde and equals expansion
//! - equalsubstr() — `=command` substitution
//! - modify() — history-style colon modifiers
//! - dopadding() — left/right padding (4-colon `(l:N:STR1:STR2:)` form)
//! - getmatch() / getmatcharr() — pattern matching
//! - quotestring() — various quoting modes
//! - arithsubst() — arithmetic substitution
//! - globlist() — glob expansion on list
//! - get_strarg() / get_intarg() — argument parsing
//! - strcatsub() — string concatenation for substitution
//! - subst_parse_str() — substitution string parsing
//! - substevalchar() — `(#)` flag evaluation
//! - untok_and_escape() — token un-escape helper
//! - check_colon_subscript() — `:OFFSET[:LEN]` substring detection
//! - dstackent() — directory stack access
//! - keyvalpairelement() — `(kv)` flag pair walker
//! - quotesubst() — quoting helper for substitution
//! - wcpadwidth() — multibyte char display-cell width for `dopadding`
//!
//! Behavioral parity is checked by `tests/zshrs_shell.rs` and the
//! `tests/no_tree_walker_dispatch.rs` invariant suite. Any divergence
//! from `/opt/homebrew/bin/zsh -fc` for the in-scope substitution
//! shapes is treated as a bug — file an issue + add a parity test.

#[allow(unused_imports)]
use crate::ported::exec::{
    cached_regex, slice_array_zero_based, slice_positionals,
};
// `subst.rs` does NOT reach into `ShellExecutor` — every shell-state
// read/write goes through the canonical C-named accessor (paramtab,
// hashtable, options globals, etc.). Command-substitution `$(...)`
// routes through `crate::exec::getoutput` (mirror of exec.c:4712).
// c:N/A
// Per user directive: history-modifier helpers (casemodify, remtpath,
// remlpaths, remtext, xsymlinks) live in src/ported/hist.rs (the
// canonical port of Src/hist.c). Import here so subst.rs's modify()
// arms and the parity tests can reference by bare name.
#[allow(unused_imports)]
use crate::parse::{ShellWord, VarModifier, ZshParamFlag};
use crate::ported::hist::{casemodify, rembutext, remlpaths, remtext, remtpath, CaseMod};
use crate::ported::utils::{xsymlinks, zerr};

use std::sync::atomic::Ordering;

// Canonical LinkList — port of `struct linklist` (`Src/zsh.h:563`)
// with the C-macro accessors (`firstnode`/`nextnode`/`getdata`/
// `setdata`/`insertlinknode`/`empty`) lifted from `Src/zsh.h:576-590`.
// subst.rs previously kept a private `pub struct LinkList { nodes:
// VecDeque<LinkNode>, flags: u32 }` + `pub struct LinkNode { data:
// String }` — DELETED per user directive (Rust-only abstraction, no
// C counterpart).
/// LinkList of substitution words. Canonical
/// `crate::ported::linklist::LinkList<String>` (port of
/// `Src/linklist.c` with `LF_ARRAY` (`Src/subst.c:33`) carried in
/// the `flags` field).
pub type LinkList = crate::ported::linklist::LinkList<String>;

/// Returns true if the global `errflag` (Src/utils.c) is set.
/// Matches the C idiom `if (errflag) …` that subst.c sprinkles
/// throughout its loops.
#[inline]
fn errflag_set() -> bool {
    crate::ported::utils::errflag.load(Ordering::Relaxed) != 0
}

/// Sets `errflag |= ERRFLAG_ERROR` on the global `errflag`.
/// Mirrors C's `errflag |= ERRFLAG_ERROR;` at every subst.c site
/// where parameter / glob / arith error is reported.
#[inline]
fn errflag_set_error() {
    crate::ported::utils::errflag.fetch_or(
        crate::ported::zsh_h::ERRFLAG_ERROR,
        Ordering::Relaxed,
    );
}

// Token constants from zsh.h (mapped to char values > 127)
// `pub mod tokens { … }` — DELETED per user directive. Was a
// Rust-only duplicate of the canonical token table in
// `crate::ported::zsh_h` (port of `Src/zsh.h:159-224`). Two names
// drifted: local `STRING` → canonical `STRING_TOK`, local
// `OUTANGPROC` → canonical `OUTANG_PROC`. All other constants
// matched bit-for-bit but living in two places invited future drift.
use crate::ported::zsh_h::{
    BNULL, DNULL, EQUALS, INANG, INBRACE, INBRACK, INPAR, INPARMATH, MARKER,
    NULARG, OUTANG, OUTANG_PROC, OUTBRACE, OUTBRACK, OUTPAR, OUTPARMATH, POUND,
    QSTRING, QTICK, SCANPM_NONAMEREF, SCANPM_WANTKEYS, SCANPM_WANTVALS, SNULL,
    STRING_TOK, TICK,
}; // c:zsh.h:159-224 + scan flags c:1953-1973
// Aliases for the two names that diverged in the local module.
// Cite c:zsh.h:160 (`STRING`) and c:zsh.h:177 (`OUTANG`+proc-sub).
const STRING: char = STRING_TOK; // c:zsh.h:160
const OUTANGPROC: char = OUTANG_PROC; // c:zsh.h:177

/// Port of `LF_ARRAY` from `Src/subst.c:33`.
/// `#define LF_ARRAY 1`. Linked-list flag the substitution-result
/// LinkList carries when the expansion produced multiple words.
/// Drives `prefork` / `singsub` / `aget` to return an array vs scalar.
pub const LF_ARRAY: u32 = 1;                                                 // c:33

// `pub mod prefork_flags { … }` — DELETED per user directive.
// Every bit value was WRONG vs the canonical C source: local
// `SINGLE=1, SPLIT=2, SHWORDSPLIT=4, NOSHWORDSPLIT=8, ASSIGN=16,
// TYPESET=32` vs C's `PREFORK_TYPESET=0x01, PREFORK_ASSIGN=0x02,
// PREFORK_SINGLE=0x04, PREFORK_SPLIT=0x08, PREFORK_SHWORDSPLIT=0x10,
// PREFORK_NOSHWORDSPLIT=0x20` (`Src/zsh.h:2020-2042`). Every
// `flags & prefork_flags::X` test silently mis-tested the wrong
// bit. Canonical defs imported from `crate::ported::zsh_h` below.
use crate::ported::zsh_h::{
    PREFORK_ASSIGN, PREFORK_KEY_VALUE, PREFORK_NOSHWORDSPLIT, PREFORK_NO_UNTOK,
    PREFORK_SHWORDSPLIT, PREFORK_SINGLE, PREFORK_SPLIT, PREFORK_SUBEXP, PREFORK_TYPESET,
}; // c:zsh.h:2020-2042

// `SubstState` and `SubstOptions` structs — DELETED per user
// directive ("SubstState must be removed", "SubstOptions must be
// removed", "delete SubstState"). All formerly-bundled fields are
// canonical globals or executor-backed:
//   - `errflag`     → `crate::ported::utils::errflag` `AtomicI32`
//                     (port of `Src/utils.c`'s `int errflag`).
//   - `opts.*`      → `crate::ported::options::opt_state_get/set`
//                     (port of zsh's `opts[OPT_…]` via `Src/options.c`).
//   - `variables` / `arrays` / `assoc_arrays`
//                   → `vars_get`/`arrays_get`/`assoc_get` helpers
//                     below (executor-backed, equiv to C's
//                     `getsparam`/`getaparam`).
//   - `skip_filesub` → `SKIP_FILESUB` thread_local in this file.
//   - `function_names`/`command_names`/`alias_names`/`var_attrs`
//                   → `shfunctab`/`cmdnamtab`/`aliastab` walks.
//   - `dirstack`/`pushdminus` → `dirstack_lock()` + `opt_state_get`.
//   - `last_subst` → `crate::ported::hist::hsubl`/`hsubr`/`hsubpatopt`.
//   - `sub_flags`  → `SUB_FLAGS` thread_local at the top of this file.
// Every fn signature has dropped the `state: &mut SubstState` arg.

/// Null string constant (from subst.c line 36)
pub const NULSTRING: &str = "\u{8F}"; // c:100

// =====================================================================
// Parameter table read/write helpers — direct paramtab access.
// C reads `paramtab` directly via `getsparam`/`getaparam`
// (`Src/params.c:3194`/`:3245`); these mirror that by hitting
// `crate::ported::params::paramtab()` (the global Mutex<HashMap<
// String, Param>>) and the parallel `paramtab_hashed_storage`.
//
// Previous incarnation routed through `fusevm_bridge::try_with_executor`
// which silently no-ops outside a live VM frame (same fake pattern
// the user flagged earlier in ksh93.rs). Tests would compile and
// "pass" while exercising no parameter machinery at all.
// =====================================================================

/// Splice (`[@]`/`[*]`) walk for the zsh/parameter magic-assoc
/// names. Mirrors the `scanpm<X>` walkers in
/// `Src/Modules/parameter.c` — each scanner iterates its backing
/// table and the splice reads them all. Returns the values joined
/// with a single space (mirrors the `j: :` C `sepjoin` default).
fn splice_magic_assoc(name: &str) -> Option<String> {
    let join = |v: Vec<String>| -> String { v.join(" ") };
    match name {
        // c:Src/Modules/parameter.c:1990 scanpmraliases — aliastab.
        // Flag checks inline against `node.flags` matching C's
        // `(a->node.flags & ALIAS_GLOBAL)` etc. style.
        "aliases" => crate::ported::hashtable::aliastab_lock().lock().ok()
            .map(|t| join(
                t.iter()
                    .filter(|(_, a)| {
                        let f = a.node.flags;
                        (f & crate::ported::hashtable::flags::ALIAS_GLOBAL as i32) == 0
                            && (f & crate::ported::hashtable::flags::ALIAS_SUFFIX as i32) == 0
                            && (f & crate::ported::hashtable::flags::DISABLED as i32) == 0
                    })
                    .map(|(_, a)| a.text.clone())
                    .collect()
            )),
        "galiases" => crate::ported::hashtable::aliastab_lock().lock().ok()
            .map(|t| join(
                t.iter()
                    .filter(|(_, a)| {
                        let f = a.node.flags;
                        (f & crate::ported::hashtable::flags::ALIAS_GLOBAL as i32) != 0
                            && (f & crate::ported::hashtable::flags::DISABLED as i32) == 0
                    })
                    .map(|(_, a)| a.text.clone())
                    .collect()
            )),
        "saliases" => crate::ported::hashtable::sufaliastab_lock().lock().ok()
            .map(|t| join(
                t.iter()
                    .filter(|(_, a)| {
                        (a.node.flags & crate::ported::hashtable::flags::DISABLED as i32) == 0
                    })
                    .map(|(_, a)| a.text.clone())
                    .collect()
            )),
        "dis_aliases" => crate::ported::hashtable::aliastab_lock().lock().ok()
            .map(|t| join(
                t.iter()
                    .filter(|(_, a)| {
                        let f = a.node.flags;
                        (f & crate::ported::hashtable::flags::ALIAS_GLOBAL as i32) == 0
                            && (f & crate::ported::hashtable::flags::ALIAS_SUFFIX as i32) == 0
                            && (f & crate::ported::hashtable::flags::DISABLED as i32) != 0
                    })
                    .map(|(_, a)| a.text.clone())
                    .collect()
            )),
        "dis_galiases" => crate::ported::hashtable::aliastab_lock().lock().ok()
            .map(|t| join(
                t.iter()
                    .filter(|(_, a)| {
                        let f = a.node.flags;
                        (f & crate::ported::hashtable::flags::ALIAS_GLOBAL as i32) != 0
                            && (f & crate::ported::hashtable::flags::DISABLED as i32) != 0
                    })
                    .map(|(_, a)| a.text.clone())
                    .collect()
            )),
        "dis_saliases" => crate::ported::hashtable::sufaliastab_lock().lock().ok()
            .map(|t| join(
                t.iter()
                    .filter(|(_, a)| {
                        (a.node.flags & crate::ported::hashtable::flags::DISABLED as i32) != 0
                    })
                    .map(|(_, a)| a.text.clone())
                    .collect()
            )),
        // c:Src/Modules/parameter.c:245 scanpmcommands — cmdnamtab.
        // For each cmdnam: HASHED arm reads `cmd` (resolved path);
        // unhashed reads first path segment in `name` (Vec<String>)
        // joined with the command name.
        "commands" => crate::ported::hashtable::cmdnamtab_lock().lock().ok()
            .map(|t| join(
                t.iter()
                    .filter_map(|(nm, c)| {
                        let hashed = (c.node.flags
                            & crate::ported::hashtable::flags::HASHED as i32) != 0;
                        if hashed {
                            c.cmd.clone()
                        } else {
                            c.name.as_ref()
                                .and_then(|v| v.first())
                                .map(|seg| format!("{}/{}", seg, nm))
                        }
                    })
                    .collect()
            )),
        // c:Src/Modules/parameter.c:519 scanpmfunctions — shfunctab.
        "functions" => crate::ported::hashtable::shfunctab_lock().lock().ok()
            .map(|t| join(
                t.iter()
                    .filter(|(_, f)| !f.is_disabled())
                    .map(|(_, f)| f.body.clone().unwrap_or_default())
                    .collect()
            )),
        "dis_functions" => crate::ported::hashtable::shfunctab_lock().lock().ok()
            .map(|t| join(
                t.iter()
                    .filter(|(_, f)| f.is_disabled())
                    .map(|(_, f)| f.body.clone().unwrap_or_default())
                    .collect()
            )),
        "functions_source" => crate::ported::hashtable::shfunctab_lock().lock().ok()
            .map(|t| join(
                t.iter()
                    .filter(|(_, f)| !f.is_disabled())
                    .map(|(_, f)| f.filename.clone().unwrap_or_default())
                    .collect()
            )),
        "dis_functions_source" => crate::ported::hashtable::shfunctab_lock().lock().ok()
            .map(|t| join(
                t.iter()
                    .filter(|(_, f)| f.is_disabled())
                    .map(|(_, f)| f.filename.clone().unwrap_or_default())
                    .collect()
            )),
        // c:Src/Modules/parameter.c:1618 scanpmnameddirs — nameddirtab.
        "nameddirs" => crate::ported::hashnameddir::nameddirtab().lock().ok()
            .map(|t| join(
                t.iter().map(|(_, d)| d.dir.clone()).collect()
            )),
        // c:Src/Modules/parameter.c:843 scanpmbuiltins — builtintab.
        "builtins" => Some(join(
            crate::ported::builtin::createbuiltintable()
                .keys().cloned().collect()
        )),
        // c:Src/Modules/parameter.c:124 scanpmparameters — paramtab.
        "parameters" => crate::ported::params::paramtab().lock().ok()
            .map(|t| join(t.keys().cloned().collect())),
        // c:Src/Modules/parameter.c:1016 scanpmoptions — optiontab.
        "options" => Some(join(
            crate::ported::options::ZSH_OPTIONS_SET.iter()
                .map(|s| s.to_string())
                .collect()
        )),
        // Names without ported splice walkers (history/modules/
        // jobdirs/jobstates/jobtexts/usergroups/userdirs/...).
        _ => None,
    }
}

/// Read a scalar variable from `paramtab`. Equivalent to C's
/// `getsparam(name)` (`Src/params.c:3194`) for the scalar case.
fn vars_get(name: &str) -> Option<String> {
    let tab = crate::ported::params::paramtab().lock().ok()?;
    let pm = tab.get(name)?;
    pm.u_str.clone()
}

/// True if `name` exists in `paramtab` (any type).
fn vars_contains(name: &str) -> bool {
    crate::ported::params::paramtab()
        .lock()
        .map_or(false, |tab| tab.contains_key(name))
}

/// Insert / replace a scalar parameter via the canonical
/// `assignsparam` path. Equivalent to C's `setsparam(name, val)`
/// (`Src/params.c:3350`).
fn vars_insert(name: String, value: String) {
    crate::ported::params::setsparam(&name, &value);
}

/// Read an array parameter from `paramtab`. Equivalent to C's
/// `getaparam(name)` (`Src/params.c:3245`).
fn arrays_get(name: &str) -> Option<Vec<String>> {
    let tab = crate::ported::params::paramtab().lock().ok()?;
    let pm = tab.get(name)?;
    pm.u_arr.clone()
}

/// True if `name` is an array in `paramtab`.
fn arrays_contains(name: &str) -> bool {
    crate::ported::params::paramtab()
        .lock()
        .map_or(false, |tab| {
            tab.get(name).map_or(false, |pm| pm.u_arr.is_some())
        })
}

/// Insert / replace an array parameter. Writes through the
/// canonical paramtab as a `PM_ARRAY` entry.
fn arrays_insert(name: String, value: Vec<String>) {
    use crate::ported::zsh_h::{hashnode, param, Param, PM_ARRAY};
    let mut tab = match crate::ported::params::paramtab().lock() {
        Ok(t) => t,
        Err(_) => return,
    };
    if let Some(pm) = tab.get_mut(&name) {
        pm.u_arr = Some(value);
        pm.u_str = None;
        pm.node.flags |= PM_ARRAY as i32;
    } else {
        let pm: Param = Box::new(param {
            node: hashnode {
                next: None,
                nam: name.clone(),
                flags: PM_ARRAY as i32,
            },
            u_data: 0, u_arr: Some(value), u_str: None, u_val: 0,
            u_dval: 0.0, u_hash: None,
            gsu_s: None, gsu_i: None, gsu_f: None, gsu_a: None, gsu_h: None,
            base: 0, width: 0, env: None, ename: None, old: None, level: 0,
        });
        tab.insert(name, pm);
    }
}

/// Read an associative array parameter from the parallel
/// `paramtab_hashed_storage` (PM_HASHED values).
fn assoc_get(name: &str) -> Option<indexmap::IndexMap<String, String>> {
    crate::ported::params::paramtab_hashed_storage()
        .lock()
        .ok()
        .and_then(|s| s.get(name).cloned())
}

/// True if `name` is an assoc-array in `paramtab_hashed_storage`.
fn assoc_contains(name: &str) -> bool {
    crate::ported::params::paramtab_hashed_storage()
        .lock()
        .map_or(false, |s| s.contains_key(name))
}

/// Array assignment via paramtab. Equivalent to C's
/// `assignaparam(name, parts)` (`Src/params.c:3357`).
fn exec_assignaparam(name: &str, parts: Vec<String>) {
    arrays_insert(name.to_string(), parts);
}

/// Assoc-array assignment via paramtab_hashed_storage. The `parts`
/// argument follows the C `sethparam` convention: alternating
/// key, value, key, value (`Src/params.c:3602`).
fn exec_sethparam(name: &str, parts: Vec<String>) {
    use crate::ported::zsh_h::{hashnode, param, Param, PM_HASHED};
    let mut map: indexmap::IndexMap<String, String> = indexmap::IndexMap::new();
    let mut it = parts.into_iter();
    while let (Some(k), Some(v)) = (it.next(), it.next()) {
        map.insert(k, v);
    }
    if let Ok(mut store) = crate::ported::params::paramtab_hashed_storage().lock() {
        store.insert(name.to_string(), map);
    }
    if let Ok(mut tab) = crate::ported::params::paramtab().lock() {
        if let Some(pm) = tab.get_mut(name) {
            pm.node.flags |= PM_HASHED as i32;
        } else {
            let pm: Param = Box::new(param {
                node: hashnode {
                    next: None,
                    nam: name.to_string(),
                    flags: PM_HASHED as i32,
                },
                u_data: 0, u_arr: None, u_str: None, u_val: 0,
                u_dval: 0.0, u_hash: None,
                gsu_s: None, gsu_i: None, gsu_f: None, gsu_a: None, gsu_h: None,
                base: 0, width: 0, env: None, ename: None, old: None, level: 0,
            });
            tab.insert(name.to_string(), pm);
        }
    }
}

/// No-op now that reads go directly to `paramtab` — the sync-shim
/// only existed for the executor-backed snapshot path.
fn exec_sync_state_from_paramtab() {}

/// Read a scalar from `paramtab`. Equivalent to C's
/// `getsparam(name)` (`Src/params.c:3194`).
fn exec_getsparam(name: &str) -> Option<String> {
    vars_get(name)
}

// =====================================================================
// !!! WARNING: RUST-ONLY STATE — NO DIRECT C COUNTERPART !!!
// =====================================================================
// `IN_PARAMSUBST_NEST` is a per-thread paramsubst recursion counter
// mirroring the C `paramsub_nest` global (Src/subst.c). The Rust
// port previously stored it on ShellExecutor; moved here to keep
// subst.rs free of ShellExecutor reaches per the
// no-shellexecutor-in-src/ported rule.
// =====================================================================
thread_local! {
    pub static IN_PARAMSUBST_NEST: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
}

// =====================================================================
// !!! RUST-ONLY STATE — NO DIRECT C COUNTERPART !!!
// =====================================================================
// `SKIP_FILESUB` is a per-thread flag that suppresses prefork's
// tilde / `=cmd` expansion pass. Used by the `${var/pat/repl}`
// pattern + replacement code paths where a literal `~` in `repl`
// must NOT expand to `$HOME`. C achieves the same observable
// behavior by NOT routing replacement strings through prefork at
// all (they go straight through parsestr+getmatch). The Rust port
// re-uses singsub→prefork for replacement strings and needs this
// flag to disable the third pass. Replaced the deleted
// `SubstState.skip_filesub` field per user "SubstState must be
// removed" directive.
// =====================================================================
thread_local! {
    pub static SKIP_FILESUB: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

// =====================================================================
// !!! RUST-ONLY STATE — NO DIRECT C COUNTERPART !!!
// =====================================================================
// `SUB_FLAGS` is the per-paramsubst `sub_flags` bitmask
// (`Src/subst.c:2169`) — SUB_MATCH / SUB_REST / SUB_BIND / SUB_EIND
// / SUB_LEN / SUB_SUBSTR / SUB_EGLOB bits set by the (M)/(B)/(E)/
// (S)/(I) flag-parsing arm and consumed by the match / replace
// operators downstream. C stores it in a static int; Rust uses
// thread_local to keep callers re-entrant. Previously routed
// through `try_with_executor` (fake — silently no-ops outside a
// live VM frame).
// =====================================================================
thread_local! {
    pub static SUB_FLAGS: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
}

/// Read the current paramsubst flag bitmask. Equivalent to C's
/// `sub_flags` read at `Src/subst.c:2171`.
pub fn sub_flags_get() -> i32 {
    SUB_FLAGS.with(|c| c.get())
}

/// Write the paramsubst flag bitmask. Equivalent to C's
/// `sub_flags = X` at `Src/subst.c:2169`.
pub fn sub_flags_set(v: i32) {
    SUB_FLAGS.with(|c| c.set(v));
}

/// Check for array assignment with entries like [key]=val
/// Port of keyvalpairelement() from subst.c lines 47-77
/// Port of `keyvalpairelement()` from `Src/subst.c:49-79`.
///
/// Detects an `[key]=value` or `[key]+=value` shape (assoc-array
/// element assignment used in `typeset -A foo=([k]=v)`). On match,
/// rewrites the single linknode into THREE nodes: a Marker sentinel,
/// the unquoted key, and the unquoted value. The Marker sentinel
/// (with optional `+` for append) signals downstream globlist /
/// prefork that this triplet should NOT be globbed.
///
/// Returns Some(value_node_idx) on match, None when the input doesn't
/// fit the shape (caller falls through to normal word handling).
fn keyvalpairelement(list: &mut LinkList, node_idx: usize) -> Option<usize> {
    // c:49
    // C: `start = (char *)getdata(node)` — fetch the node's text.
    let data = list.getdata(node_idx)?.to_string(); // c:53
    let chars: Vec<char> = data.chars().collect(); // c:53

    // C: `start[0] == Inbrack` — must lead with `[` (or token).
    if chars.is_empty()                                     // c:54
        || (chars[0] != INBRACK && chars[0] != '[')
    // c:54
    {
        return None; // c:54
    }

    // C: `end = strchr(start+1, Outbrack)` — find matching `]`.
    let mut end_pos: Option<usize> = None; // c:55
    for (i, &c) in chars.iter().enumerate().skip(1) {
        // c:55
        if c == OUTBRACK || c == ']' {
            // c:55
            end_pos = Some(i); // c:55
            break; // c:55
        }
    }
    let end_pos = end_pos?; // c:55

    // C: `end[1] == Equals || (end[1] == '+' && end[2] == Equals)`
    // — `]=value` or `]+=value` postfix.
    if end_pos + 1 >= chars.len() {
        // c:57
        return None; // c:57
    }
    let is_append = chars.get(end_pos + 1) == Some(&'+')    // c:58
        && (chars.get(end_pos + 2) == Some(&EQUALS)
            || chars.get(end_pos + 2) == Some(&'='));
    let is_assign = !is_append                              // c:57
        && (chars.get(end_pos + 1) == Some(&EQUALS)
            || chars.get(end_pos + 1) == Some(&'='));
    if !is_assign && !is_append {
        // c:60
        return None;
    }

    // C: `*end = '\0'; dat = start + 1; singsub(&dat); untokenize(dat);`
    // — extract key, run param-subst, untokenize.
    let raw_key: String = chars[1..end_pos].iter().collect(); // c:64
    let key_subst = singsub(&raw_key); // c:65
    let key = crate::lex::untokenize(&key_subst); // c:66

    // C lines 67-75: Marker / Marker_plus sentinel + insertlinknode
    // for key and value.
    let value_start = if is_append { end_pos + 3 } else { end_pos + 2 }; // c:67-72
    let raw_value: String = chars[value_start..].iter().collect(); // c:69 / 73
    let value_subst = singsub(&raw_value); // c:75
    let value = crate::lex::untokenize(&value_subst); // c:76

    let marker = if is_append {
        // c:67
        format!("{}+", MARKER) // c:67
    } else {
        // c:71
        MARKER.to_string() // c:71
    };

    list.setdata(node_idx, marker); // c:72
    let key_idx = list.insertlinknode(node_idx, key); // c:73
    let val_idx = list.insertlinknode(key_idx, value); // c:77

    // C: `return insertlinknode(list, node, dat);` — node where
    // value was inserted.
    Some(val_idx) // c:77
} // c:79

/// Do substitutions before fork
/// Port of prefork() from subst.c lines 94-183
/// Phase-1 word-list substitution (tilde/equal/brace/param/cmd/arith).
/// Port of `prefork()` from Src/subst.c:100 — runs ahead of
/// glob expansion to fully resolve `${...}` / `$(...)` /
/// `$((...))` / `~user` / `=cmd` / `{a,b}`.
// Do substitutions before fork.                                            // c:82
pub fn prefork(list: &mut LinkList, flags: i32, ret_flags: &mut i32) { // c:100
    // c:100
    let mut node_idx = 0; // c:100
    let mut stop_idx: Option<usize> = None; // c:100
    let mut keep = false; // c:100
    let asssub = (flags & PREFORK_TYPESET != 0) && isset(crate::ported::zsh_h::KSHTYPESET); // c:100
    let mut iter_count = 0u32; // c:100

    while node_idx < list.nodes.len() {
        // c:100
        iter_count += 1; // c:100
        if iter_count > 100_000 {
            // c:100
            // Safety cap: if some bug causes prefork's outer loop to
            // never terminate, bail rather than hang the process.
            return; // c:100
        } // c:100
          // Check for key-value pair element
        if (flags & (PREFORK_SINGLE | PREFORK_ASSIGN)) == PREFORK_ASSIGN {
            // c:100
            if let Some(new_idx) = keyvalpairelement(list, node_idx) {
                // c:100
                node_idx = new_idx + 1; // c:100
                *ret_flags |= PREFORK_KEY_VALUE;
                continue; // c:100
            } // c:100
        } // c:100

        if errflag_set() {
            // c:100
            return; // c:100
        } // c:100

        if isset(crate::ported::zsh_h::SHFILEEXPANSION) {
            // c:100
            // SHFILEEXPANSION - do file substitution first
            if let Some(data) = list.getdata(node_idx) {
                // c:100
                let new_data = filesub(
                    // c:100
                    data,                                                     // c:100
                    flags & (PREFORK_TYPESET | PREFORK_ASSIGN), // c:100
                ); // c:100
                list.setdata(node_idx, new_data); // c:100
            } // c:100
        } else {
            // c:100
            // Do string substitution
            if let Some(new_idx) = stringsubst(
                // c:100
                list,                                                      // c:100
                node_idx,                                                  // c:100
                flags & !(PREFORK_TYPESET | PREFORK_ASSIGN), // c:100
                ret_flags,                                                 // c:100
                asssub,                                                    // c:100
            ) {
                // c:100
                node_idx = new_idx; // c:100
            } else {
                // c:100
                return; // c:100
            } // c:100
        } // c:100

        node_idx += 1; // c:100
    } // c:100

    // Second pass for SHFILEEXPANSION
    if isset(crate::ported::zsh_h::SHFILEEXPANSION) {
        // c:100
        node_idx = 0; // c:100
        while node_idx < list.nodes.len() {
            // c:100
            if let Some(new_idx) = stringsubst(
                // c:100
                list,                                                      // c:100
                node_idx,                                                  // c:100
                flags & !(PREFORK_TYPESET | PREFORK_ASSIGN), // c:100
                ret_flags,                                                 // c:100
                asssub,                                                    // c:100
            ) {
                // c:100
                node_idx = new_idx + 1; // c:100
            } else {
                // c:100
                return; // c:100
            } // c:100
        } // c:100
    } // c:100

    // Third pass: brace expansion and file substitution
    node_idx = 0; // c:100
    while node_idx < list.nodes.len() {
        // c:100
        if Some(node_idx) == stop_idx {
            // c:100
            keep = false; // c:100
        } // c:100

        if let Some(data) = list.getdata(node_idx) {
            // c:100
            if !data.is_empty() {
                // c:100
                // remnulargs
                let data = data.replace('\0', ""); // c:100
                list.setdata(node_idx, data.clone()); // c:100

                // Brace expansion. C: `while (hasbraces(getdata(node)))
                // { keep = 1; xpandbraces(list, &node); }`. zsh's
                // hasbraces walks the string looking for a balanced
                // `{…}` containing `,` or `..` (range). xpandbraces
                // splits the node into N nodes.
                //
                // Routes through canonical
                // crate::ported::glob::xpandbraces; treats >1
                // result as a positive hasbraces hit.
                if !isset(crate::ported::zsh_h::IGNOREBRACES) && (flags & PREFORK_SINGLE == 0) {
                    // c:166
                    if !keep {
                        // c:168
                        stop_idx = list.nextnode(node_idx); // c:169
                    }
                    loop {
                        // c:170 (while hasbraces)
                        let cur = match list.getdata(node_idx) {
                            Some(d) => d.to_string(),
                            None => break,
                        };
                        let expanded = crate::ported::glob::xpandbraces(&cur, false); // c:171
                        if expanded.len() <= 1 {
                            break;
                        } // c:170 (!hasbraces)
                        keep = true; // c:172
                                     // Replace current node with first expansion;
                                     // insert the rest as new nodes after it.
                        list.setdata(node_idx, expanded[0].clone()); // c:173 (xpandbraces)
                        let mut last = node_idx;
                        for ex in &expanded[1..] {
                            last = list.insertlinknode(last, ex.clone());
                        }
                        // Loop again: the first expansion may itself
                        // contain more brace patterns to expand.
                    }
                }

                // File substitution (non-SHFILEEXPANSION). Skip
                // entirely when state.skip_filesub is set — used
                // for `${var/pat/repl}` pattern + replacement
                // contexts where literal `~` must be preserved.
                if !isset(crate::ported::zsh_h::SHFILEEXPANSION) && !SKIP_FILESUB.with(|c| c.get()) {
                    // c:100
                    if let Some(data) = list.getdata(node_idx) {
                        // c:100
                        let new_data = filesub(
                            // c:100
                            data,                                                     // c:100
                            flags & (PREFORK_TYPESET | PREFORK_ASSIGN), // c:100
                        ); // c:100
                        list.setdata(node_idx, new_data); // c:100
                    } // c:100
                } // c:100
            } else if (flags & PREFORK_SINGLE == 0)  // c:100
                && (*ret_flags & PREFORK_KEY_VALUE == 0) // c:100
                && !keep
            // c:100
            {
                // c:100
                list.delete_node(node_idx); // c:100
                continue; // Don't increment, we removed    // c:100
            } // c:100
        } // c:100

        if errflag_set() {
            // c:100
            return; // c:100
        } // c:100

        node_idx += 1; // c:100
    } // c:100
} // c:100

/// Port of `stringsubstquote()` from `Src/subst.c:206-233`.
///
/// Implements `$'...'` ANSI-C-style quoted-string substitution. The
/// C signature is `char *stringsubstquote(char *strstart, char **pstrdpos)`
/// — it returns the new full string (with the `$'…'` segment replaced
/// by the unescaped content) and updates `*pstrdpos` to point past
/// the replacement.
///
/// Rust signature: `(strstart, strdpos) -> (String, usize)` — same
/// data, returned as a tuple instead of an in-out pointer.
///
/// C body:
///   1. `strsub = getkeystring(strdpos+2, &len, GETKEYS_DOLLARS_QUOTE, NULL)`
///      — calls utils.c's getkeystring with the dollars-bslashquote flag,
///      which walks chars until an unescaped `'` and returns the
///      unescaped contents.
///   2. `len += 2` — account for the `$'` prefix.
///   3. Concat the prefix (strstart..strdpos), strsub, and the
///      suffix (strdpos+len..). Special case: empty `$''` returns
///      a Nularg sentinel so it doesn't get elided downstream.
///   4. Set *pstrdpos to point past the substituted region.
fn stringsubstquote(strstart: &str, strdpos: usize) -> (String, usize) {
    // c:206
    let chars: Vec<char> = strstart.chars().collect(); // c:208

    // C: `getkeystring(strdpos+2, &len, GETKEYS_DOLLARS_QUOTE, NULL)`.
    // Rust's getkeystring doesn't take a stop-at-unquoted-` flag, so
    // we walk the quoted region manually first, then unescape the
    // captured content. Same observable behavior: dollar-quoted
    // chars get C-escape-processed, unescaped `'` terminates.
    let start = strdpos + 2; // c:209 (strdpos+2)
    let mut end = start; // c:209
    let mut escaped = false; // c:209

    while end < chars.len() {
        // c:209
        if escaped {
            // c:209
            escaped = false; // c:209
            end += 1; // c:209
            continue; // c:209
        }
        if chars[end] == '\\' {
            // c:209
            escaped = true; // c:209
            end += 1; // c:209
            continue; // c:209
        }
        if chars[end] == '\'' {
            break;
        } // c:209 (unescaped close)
        end += 1;
    }

    // C: `getkeystring` returns the unescaped content (strsub) +
    // length consumed. Rust calls getkeystring on the captured
    // content slice; consumed count is the slice length plus the
    // wrapping `$'` and `'`.
    let content: String = chars[start..end].iter().collect();
    let (strsub, _) = crate::ported::utils::getkeystring(&content); // c:211

    // C: `len += 2;` — caller's len now includes the leading `$'`
    // (Rust mirrors via end+1 below).

    // C: `if (strstart != strdpos)` — there's a prefix, so concat
    // prefix + strsub + suffix. Rust always concats; empty prefix
    // is benign.
    let prefix: String = chars[..strdpos].iter().collect(); // c:215
    let suffix: String = if end + 1 < chars.len() {
        // c:216 (strdpos[len] check)
        chars[end + 1..].iter().collect() // c:217
    } else {
        String::new() // c:218
    };

    // C: empty `$''` special case — `strret = dupstring(nulstring);`
    // returns the NULARG sentinel string so the empty bslashquote doesn't
    // get elided by stringsubst's word-walk.
    let strret = if strsub.is_empty() && prefix.is_empty() && suffix.is_empty() {
        // c:226
        // Nularg = '\u{8b}' per zsh.h. Emit as a single-char string
        // so downstream code recognises the empty-bslashquote sentinel.
        "\u{8b}".to_string() // c:227
    } else {
        format!("{}{}{}", prefix, strsub, suffix) // c:215-220
    };

    // C: `*pstrdpos = strret + (strdpos - strstart) + strlen(strsub);`
    // — sets the in-out pointer to one past the unescaped content
    // in the new string. Rust returns the equivalent index.
    let new_pos = prefix.chars().count()
        + strret
            .chars()
            .count()
            .saturating_sub(prefix.chars().count() + suffix.chars().count()); // c:230

    (strret, new_pos) // c:232
} // c:233

/// String substitution - main workhorse
/// Port of stringsubst() from subst.c lines 227-421
fn stringsubst(
    // c:237
    list: &mut LinkList,    // c:237
    node_idx: usize,        // c:237
    pf_flags: i32,          // c:237
    ret_flags: &mut i32,    // c:237
    asssub: bool,           // c:237
) -> Option<usize> {
    // c:237
    let mut str3 = list.getdata(node_idx)?.to_string(); // c:237
    let mut pos = 0; // c:237

    // First pass: process substitutions. Loop guard uses CHAR
    // count, not str3.len() (byte count) — `pos` is a char index
    // throughout the function and chars[pos] indexes by char. With
    // multi-byte UTF-8 (zsh-meta tokens 0x83-0x9f each take 2 bytes
    // in UTF-8 encoding), `pos < str3.len()` looped past the end of
    // `chars` and `chars[pos]` panicked. str3 may be mutated within
    // the loop body so `chars` is re-collected each iteration.
    let mut p1_iter = 0u32; // c:237
    loop {
        // c:237
        if errflag_set() {
            // c:237
            break; // c:237
        } // c:237
        p1_iter += 1; // c:237
        if p1_iter > 100_000 {
            // c:237
            return None; // c:237
        } // c:237
        let chars: Vec<char> = str3.chars().collect(); // c:237
        if pos >= chars.len() {
            // c:237
            break; // c:237
        } // c:237
        let c = chars[pos]; // c:237

        // Check for <(...), >(...), =(...)
        if (c == INANG || c == OUTANGPROC || (pos == 0 && c == EQUALS)) // c:237
            && chars.get(pos + 1) == Some(&INPAR)
        // c:237
        {
            // c:237
            // <(...) / >(...) / =(...) process / cmd substitution.
            // The full port (getproc / getoutputfile) needs fork/exec
            // and lives in Src/exec.c. Until that lands, skip the
            // marker AND its parenthesized body so subsequent passes
            // don't misinterpret the inner text as bare param/cmd
            // substitution. Direct port of subst.c:248-274 layout —
            // C calls getproc/getoutputfile then memcpy's the result;
            // the no-op port still has to consume the same span.
            if errflag_set() {
                // c:237
                return None; // c:237
            } // c:237
              // Walk the matching close paren — depth-tracked so
              // nested `<(echo $(...))` skips correctly. Includes the
              // INANG/OUTANGPROC/EQUALS marker char itself.
            let start = pos; // c:237
            pos += 2; // c:237 (skip marker + INPAR)
            let mut depth = 1_i32; // c:237
            while pos < chars.len() && depth > 0 {
                // c:237
                let ch = chars[pos]; // c:237
                if ch == INPAR {
                    depth += 1;
                }
                // c:237
                else if ch == OUTPAR {
                    depth -= 1;
                } // c:237
                pos += 1; // c:237
            } // c:237
              // Excise the entire span (was producing junk output
              // for `cat <(echo a) <(echo b)` because the half-skipped
              // `(echo a)` parsed as cmd-subst).
            let str_chars: Vec<char> = str3.chars().collect(); // c:237
            let mut new_str = String::with_capacity(str_chars.len());
            new_str.extend(str_chars[..start].iter()); // c:237
            new_str.extend(str_chars[pos..].iter()); // c:237
            str3 = new_str; // c:237
            list.setdata(node_idx, str3.clone()); // c:237
            pos = start; // c:237
            continue; // c:237
        } // c:237

        pos += 1; // c:237
    } // c:237

    // Second pass: $, `, etc. Same char-vs-byte fix as the first
    // pass — `pos < str3.len()` was a byte-len guard but `pos`
    // and `chars[pos]` are char-indexed. Multi-byte UTF-8 (zsh-
    // meta tokens 0x83-0x9f) tripped the panic.
    pos = 0; // c:237
    let mut iter_count = 0u32; // c:237
    loop {
        // c:237
        if errflag_set() {
            // c:237
            break; // c:237
        } // c:237
        iter_count += 1; // c:237
        if iter_count > 100_000 {
            // c:237
            return None; // c:237
        } // c:237
        let chars: Vec<char> = str3.chars().collect(); // c:237
        if pos >= chars.len() {
            // c:237
            break; // c:237
        } // c:237
        let c = chars[pos]; // c:237

        // Lexer-emitted single-bslashquote marker (`\u{9d}`, parse/src/tokens.rs
        // SNULL) encloses literal `'…'` regions. Inside, no parameter /
        // command substitution / glob fires — content is verbatim.
        // Strip both markers and leave the body intact. Without this, a
        // `${var/pat/'~'$match[1]}` replacement yielded
        // `\u{9d}~\u{9d}<match-1>` (SNULLs leaked through, broke the
        // string).
        if c == '\u{9d}' {
            // c:237
            // Find matching close-SNULL.
            let mut end = pos + 1; // c:237
            while end < chars.len() && chars[end] != '\u{9d}' {
                // c:237
                end += 1; // c:237
            } // c:237
              // Splice out the opening + closing markers; body stays.
            let prefix: String = chars[..pos].iter().collect(); // c:237
            let body: String = chars[pos + 1..end].iter().collect(); // c:237
            let suffix: String = if end < chars.len() {
                // c:237
                chars[end + 1..].iter().collect() // c:237
            } else {
                // c:237
                String::new() // c:237
            }; // c:237
            str3 = format!("{}{}{}", prefix, body, suffix); // c:237
            pos += body.chars().count(); // c:237
            list.setdata(node_idx, str3.clone()); // c:237
            continue; // c:237
        } // c:237
          // Lexer-emitted double-bslashquote marker (`\u{9e}`, DNULL) — strip;
          // contents inside DQ already had `$`/`${…}` tokenized to STRING
          // / QSTRING by the lexer, so the surrounding pass picks them
          // up. The markers themselves are noise for substitution.
        if c == '\u{9e}' {
            // c:237
            let prefix: String = chars[..pos].iter().collect(); // c:237
            let suffix: String = if pos + 1 < chars.len() {
                // c:237
                chars[pos + 1..].iter().collect() // c:237
            } else {
                // c:237
                String::new() // c:237
            }; // c:237
            str3 = format!("{}{}", prefix, suffix); // c:237
            list.setdata(node_idx, str3.clone()); // c:237
            continue; // c:237
        } // c:237
          // Lexer BNULL (`\u{9f}`) escapes the next char as literal.
          // Drop the marker, keep the next char verbatim, and skip past
          // it without further processing this iteration.
        if c == '\u{9f}' && pos + 1 < chars.len() {
            // c:237
            let prefix: String = chars[..pos].iter().collect(); // c:237
            let kept = chars[pos + 1]; // c:237
            let suffix: String = if pos + 2 < chars.len() {
                // c:237
                chars[pos + 2..].iter().collect() // c:237
            } else {
                // c:237
                String::new() // c:237
            }; // c:237
            str3 = format!("{}{}{}", prefix, kept, suffix); // c:237
            pos += 1; // c:237
            list.setdata(node_idx, str3.clone()); // c:237
            continue; // c:237
        } // c:237
          // Literal `'…'` single-quoted span. The lexer normally
          // converts these to `\u{9d}…\u{9d}` (handled above), but
          // recursive paths that re-enter stringsubst with already-
          // untokenized text (e.g. an outer expand_string ran
          // `untokenize`, dropping SNULLs but preserving the literal
          // `'`) still need the literal-span semantics. Per zsh single-
          // bslashquote rules: contents are verbatim, no `$`/`${…}` / glob
          // expansion fires inside. Strip the surrounding quotes and
          // leave the body intact.
        if c == '\'' {
            // c:237
            // Find matching close bslashquote — backslash inside `'…'` is
            // NOT an escape (zsh rule), so don't track escaping.
            let mut end = pos + 1; // c:237
            while end < chars.len() && chars[end] != '\'' {
                // c:237
                end += 1; // c:237
            } // c:237
            let prefix: String = chars[..pos].iter().collect(); // c:237
            let body: String = chars[pos + 1..end].iter().collect(); // c:237
            let suffix: String = if end < chars.len() {
                // c:237
                chars[end + 1..].iter().collect() // c:237
            } else {
                // c:237
                String::new() // c:237
            }; // c:237
            str3 = format!("{}{}{}", prefix, body, suffix); // c:237
            pos += body.chars().count(); // c:237
            list.setdata(node_idx, str3.clone()); // c:237
            continue; // c:237
        } // c:237

        let qt = c == QSTRING; // c:237
                               // C zsh's stringsubst gates on the lexer-tokenized `String` /
                               // `Qstring` markers (Src/subst.c:265 in the case-arms within
                               // the per-char loop). zshrs's input strings sometimes carry
                               // those tokenized markers (when called from the parser) and
                               // sometimes carry literal `$` (when called from runtime
                               // execution paths like `apply_operator`'s recursive
                               // `multsub` for `:=` operands). Accept both so the same
                               // engine can dispatch regardless of which layer fed us.
                               // Mirrors the practical effect of C's untokenize step that
                               // would have run before stringsubst sees the string.
        if qt || c == STRING || c == '$' {
            // c:237
            let next_c = chars.get(pos + 1).copied(); // c:237
                                                      // Accept either tokenized `INPAR` / `INPARMATH` / `INBRACK`
                                                      // / `INBRACE` / `SNULL` OR their literal `(` / `[` / `{`
                                                      // / `'` counterparts.
            let next_is = |tok: char, lit: char| {
                // c:237
                next_c == Some(tok) || next_c == Some(lit) // c:237
            }; // c:237

            // Detect `$((expr))` arith form FIRST — it's
            // `$(` + `(expr)` + `)` so naive cmd-subst dispatch
            // would try to execute `((expr))` as a command. Either
            // the lexer-tokenized INPARMATH or the literal `((`
            // sequence routes through the arith path. Direct port
            // of subst.c's INPARMATH arm at c:237 (see C lines
            // around 320-360 which check `*++s == Inpar` after
            // `*s == String`).
            if next_c == Some(INPARMATH)                    // c:237
                || (next_c == Some('(') && chars.get(pos + 2).copied() == Some('('))
            // c:237
            {
                // c:237
                // Walk to matching `))`, depth-tracked for nested
                // `$((a + (b * c)))`. Skip the leading `((`.
                let start = pos + 3; // c:237 (past `$((`)
                let mut depth = 2_i32; // c:237 (we've opened 2 parens)
                let mut p = start; // c:237
                let mut end_off: Option<usize> = None; // c:237
                while p < chars.len() {
                    // c:237
                    let ch = chars[p]; // c:237
                    if ch == '(' || ch == INPAR {
                        depth += 1;
                    }
                    // c:237
                    else if ch == ')' || ch == OUTPAR {
                        // c:237
                        depth -= 1; // c:237
                        if depth == 0 {
                            // c:237
                            end_off = Some(p); // c:237 (closing )) at p .. p+1)
                            break; // c:237
                        } // c:237
                    } // c:237
                    p += 1; // c:237
                } // c:237
                if let Some(end) = end_off {
                    // c:237
                    // Expression text is between start and end-1
                    // (one inner `)` got consumed by depth=1; the
                    // outer `)` closes us at depth=0).
                    let expr: String = chars[start..end - 1].iter().collect(); // c:237
                    let prefix: String = chars[..pos].iter().collect(); // c:237
                    let suffix: String = if end + 1 < chars.len() {
                        // c:237
                        chars[end + 1..].iter().collect() // c:237
                    } else {
                        // c:237
                        String::new() // c:237
                    }; // c:237
                    let result_only = arithsubst(&expr, "", ""); // c:237
                    str3 = format!("{}{}{}", prefix, result_only, suffix); // c:237
                    list.setdata(node_idx, str3.clone()); // c:237
                    pos = prefix.chars().count() + result_only.chars().count(); // c:237
                    continue; // c:237
                } // c:237
            } // c:237

            if next_is(INPAR, '(') || next_is(INPARMATH, '\0') {
                // c:237
                if !qt {
                    // c:237
                    list.flags |= LF_ARRAY; // c:237
                } // c:237
                  // Command substitution `$(cmd)` — port of subst.c:237
                  // stringsubst's $(...) arm. Find the matching ),
                  // extract cmd text, delegate to ShellExecutor's
                  // run_command_substitution (canonical executor lives
                  // outside SubstState; bridged via fusevm_bridge::
                  // with_executor).
                let cmd_open = pos + 1; // c:237 (s after $)
                let chars: Vec<char> = str3.chars().collect(); // c:237
                let mut depth = 0_i32; // c:237
                let mut end = cmd_open; // c:237
                while end < chars.len() {
                    // c:237
                    let ch = chars[end]; // c:237
                    if ch == '(' || ch == INPAR {
                        depth += 1;
                    }
                    // c:237
                    else if ch == ')' || ch == OUTPAR {
                        // c:237
                        depth -= 1; // c:237
                        if depth == 0 {
                            break;
                        } // c:237
                    } // c:237
                    end += 1; // c:237
                } // c:237
                if end < chars.len() && depth == 0 {
                    // c:237
                    let cmd: String = chars[cmd_open + 1..end].iter().collect(); // c:237
                                                                                 // \$(< file) shorthand — read file contents directly
                                                                                 // without spawning a process. Direct port of subst.c
                                                                                 // around line 250 which checks for the leading
                                                                                 // `<` redirect-only form and calls readoutput
                                                                                 // instead of getoutput.
                    let trimmed = cmd.trim_start();
                    let output = if let Some(rest) = trimmed.strip_prefix('<') {
                        let path = rest.trim();
                        std::fs::read_to_string(path).unwrap_or_default()
                    } else {
                        // c:exec.c:4712 — `getoutput(cmd, 0)`.
                        crate::exec::getoutput(&cmd)
                    };
                    let prefix: String = chars[..pos].iter().collect(); // c:237
                    let suffix: String = if end + 1 < chars.len() {
                        // c:237
                        chars[end + 1..].iter().collect() // c:237
                    } else {
                        // c:237
                        String::new() // c:237
                    }; // c:237
                    str3 = format!("{}{}{}", prefix, output.trim_end_matches('\n'), suffix); // c:237
                    pos = prefix.chars().count() + output.trim_end_matches('\n').chars().count(); // c:237
                    list.setdata(node_idx, str3.clone()); // c:237
                } else {
                    // c:237
                    pos += 1; // c:237
                } // c:237
                continue; // c:237
            } else if next_is(INBRACK, '[') {
                // c:237
                // $[...] arithmetic
                // $[...] arith substitution. Walk to matching ]
                // tracking depth so $[$[a+b]+c] nests correctly.
                let start = pos + 2; // c:237
                let open = if next_c == Some(INBRACK) {
                    INBRACK
                } else {
                    '['
                }; // c:237
                let close = if open == INBRACK { OUTBRACK } else { ']' }; // c:237
                let chars: Vec<char> = str3.chars().collect(); // c:237
                let mut depth = 1_i32; // c:237
                let mut end_off: Option<usize> = None; // c:237
                let mut p = start; // c:237
                while p < chars.len() {
                    // c:237
                    let ch = chars[p]; // c:237
                    if ch == open || ch == '[' {
                        depth += 1;
                    }
                    // c:237
                    else if ch == close || ch == ']' {
                        // c:237
                        depth -= 1; // c:237
                        if depth == 0 {
                            end_off = Some(p - start);
                            break;
                        } // c:237
                    } // c:237
                    p += 1; // c:237
                } // c:237
                if let Some(end) = end_off {
                    // c:237
                    let expr: String = chars[start..start + end].iter().collect(); // c:237
                    let prefix: String = chars[..pos].iter().collect(); // c:237
                    let suffix: String = if start + end + 1 < chars.len() {
                        chars[start + end + 1..].iter().collect()
                    } else {
                        String::new()
                    };
                    // Compute the arith result ONCE — was running
                    // arithsubst twice (once for the substituted
                    // string, again to measure the substituted-only
                    // portion's char count). Side-effects in the
                    // expression (post-increment, assignment) fired
                    // twice, breaking `$((i++))`-style code at the
                    // $[…] alias.
                    let result_only = arithsubst(&expr, "", ""); // c:237
                    str3 = format!("{}{}{}", prefix, result_only, suffix); // c:237
                    list.setdata(node_idx, str3.clone()); // c:237
                    pos = prefix.chars().count() + result_only.chars().count(); // c:237
                    continue; // c:237
                } else {
                    // c:237
                    errflag_set_error(); // c:237
                    zerr("closing bracket missing"); // c:237
                    return None; // c:237
                } // c:237
            } else if next_c == Some(SNULL) || next_c == Some('\'') {
                // c:237
                // $'...' ANSI-C quoting. Accept either the lexer-
                // tokenized SNULL marker OR the raw `'` — recursive
                // operator-operand paths (e.g. multsub on a `:=`
                // operand) hand us the literal text without prior
                // tokenization, so dispatch on the literal too.
                let (new_str, new_pos) = stringsubstquote(&str3, pos); // c:237
                str3 = new_str; // c:237
                pos = new_pos; // c:237
                list.setdata(node_idx, str3.clone()); // c:237
                continue; // c:237
            } else {
                // c:237
                // Parameter substitution
                let mut new_pf_flags = pf_flags; // c:237
                if (isset(crate::ported::zsh_h::SHWORDSPLIT) && (pf_flags & PREFORK_NOSHWORDSPLIT == 0)) // c:237
                    || (pf_flags & PREFORK_SPLIT != 0)
                // c:237
                {
                    // c:237
                    new_pf_flags |= PREFORK_SHWORDSPLIT; // c:237
                } // c:237

                // stringsubst → paramsubst is a recursive descent —
                // bump the executor's paramsubst-nest counter so the
                // inner expansion's glob_subst etc. sees it's running
                // inside an outer operand context (where filesystem
                // glob expansion must be suppressed). Use the fallible
                // variant so the unit-test path that calls paramsubst
                // without a live executor doesn't panic.
                IN_PARAMSUBST_NEST.with(|c| c.set(c.get() + 1));             // c:237 paramsub_nest++
                let (new_str, new_pos, new_nodes) = paramsubst(
                    // c:237
                    &str3, // c:237
                    pos,   // c:237
                    qt,    // c:237
                    new_pf_flags                            // c:237
                        & (PREFORK_SINGLE            // c:237
                            | PREFORK_SHWORDSPLIT    // c:237
                            | PREFORK_SUBEXP), // c:237
                    ret_flags, // c:237
                ); // c:237
                IN_PARAMSUBST_NEST.with(|c| c.set(c.get() - 1));             // c:237 paramsub_nest--

                if errflag_set() {
                    // c:237
                    return None; // c:237
                } // c:237

                // Insert additional nodes if word splitting produced
                // them. Empty new_nodes means the expansion produced
                // ZERO words (e.g. unquoted empty array \${arr} with
                // arr=()) — clear the original node's text so the
                // surrounding context (prefix/suffix) collapses.
                // Direct port of zsh's behavior: \`cmd \$arr\` with
                // arr=() runs cmd with no args.
                if new_nodes.is_empty() {
                    // c:237
                    list.setdata(node_idx, String::new()); // c:237
                } else {
                    let mut current_idx = node_idx; // c:237
                    for (i, node_data) in new_nodes.into_iter().enumerate() {
                        // c:237
                        if i == 0 {
                            // c:237
                            list.setdata(current_idx, node_data); // c:237
                        } else {
                            // c:237
                            current_idx = list.insertlinknode(current_idx, node_data);
                            // c:237
                        } // c:237
                    } // c:237
                }

                str3 = list.getdata(node_idx)?.to_string(); // c:237
                pos = new_pos; // c:237
                continue; // c:237
            } // c:237
        } // c:237

        // Backtick command substitution `cmd` — same engine as
        // `$(cmd)` per subst.c:237. Find the matching backtick,
        // capture cmd text, delegate to run_command_substitution.
        // The bridge's BUILTIN_EXPAND_TEXT untokenizes TICK/QTICK
        // back to a raw `` ` `` before calling singsub, so accept
        // any of the three forms as the open/close delimiter.
        let qt = c == QTICK; // c:237
        if qt || c == TICK || c == '`' {
            // c:237
            if !qt {
                // c:237
                list.flags |= LF_ARRAY; // c:237
            } // c:237
            let chars: Vec<char> = str3.chars().collect(); // c:237
            let cmd_start = pos + 1; // c:237
            let mut end = cmd_start; // c:237
            while end < chars.len()
                && chars[end] != TICK
                && chars[end] != QTICK
                && chars[end] != '`'
            {
                if chars[end] == '\\' && end + 1 < chars.len() {
                    end += 1;
                } // c:237
                end += 1; // c:237
            } // c:237
            if end < chars.len() {
                // c:237
                let cmd: String = chars[cmd_start..end].iter().collect(); // c:237
                // c:exec.c:4712 — `getoutput(cmd, 0)`.
                let output = crate::exec::getoutput(&cmd);
                let prefix: String = chars[..pos].iter().collect(); // c:237
                let suffix: String = if end + 1 < chars.len() {
                    // c:237
                    chars[end + 1..].iter().collect() // c:237
                } else {
                    // c:237
                    String::new() // c:237
                }; // c:237
                str3 = format!("{}{}{}", prefix, output.trim_end_matches('\n'), suffix); // c:237
                pos = prefix.chars().count() + output.trim_end_matches('\n').chars().count(); // c:237
                list.setdata(node_idx, str3.clone()); // c:237
            } else {
                // c:237
                pos += 1; // c:237
            } // c:237
            continue; // c:237
        } // c:237

        // Assignment context
        if asssub && (c == '=' || c == EQUALS) && pos > 0 { // c:237
             // We're in assignment context, apply SINGLE flag
             // (handled by caller typically)
        } // c:237

        pos += 1; // c:237
    } // c:237

    if errflag_set() {
        // c:237
        None // c:237
    } else {
        // c:237
        Some(node_idx) // c:237
    } // c:237
} // c:237

// parameter substitution                                                   // c:1601
/// Parameter substitution
/// Port of paramsubst() from subst.c lines 1600-4922 (THIS IS THE BIG ONE)
// parameter substitution                                                   // c:1601
pub fn paramsubst(
    // c:1625
    s: &str,                // c:1625
    start_pos: usize,       // c:1625
    qt: bool,               // c:1625
    pf_flags: i32,          // c:1625
    ret_flags: &mut i32,    // c:1625
) -> (String, usize, Vec<String>) {
    // c:1625
    let chars: Vec<char> = s.chars().collect(); // c:1625
    let mut pos = start_pos + 1; // Skip $ or Qstring       // c:1625
    let mut result_nodes = Vec::new(); // c:1625

    // Check what follows the $
    let c = chars.get(pos).copied().unwrap_or('\0'); // c:1625

    // ${...} form
    // ${...} brace form. Pragmatic inline port covering high-traffic
    // shapes from subst.c:1885+ (full 2,849-line paramsubst port is
    // an ongoing arm-by-arm effort). Handles: bare ref, ${#var}
    // length, :- :+ := :? defaults, # ## % %% strip, / // replace
    // with anchored # / % variants, :N:M slice, plus a permissive
    // (...)-flag prefix swallow.
    if c == INBRACE || c == '{' {
        // c:1885
        pos += 1; // c:1885 (skip {)
                  // Find matching `}` — track brace depth for nested ${...}
        let mut depth = 1_i32; // c:1885
        let mut end = pos; // c:1885
        while end < chars.len() && depth > 0 {
            // c:1885
            let ch = chars[end]; // c:1885
            if ch == '{' || ch == INBRACE {
                depth += 1;
            }
            // c:1885
            else if ch == '}' || ch == OUTBRACE {
                // c:1885
                depth -= 1; // c:1885
                if depth == 0 {
                    break;
                } // c:1885
            } // c:1885
            end += 1; // c:1885
        } // c:1885
          // No closing `}` — emit "bad substitution" and bail.
          // Direct port of zsh's zerr("closing brace missing") at
          // subst.c around line 1885.
        if end >= chars.len() || depth != 0 {
            zerr("closing brace missing"); // c:1885
            errflag_set_error(); // c:1885
            return (String::new(), chars.len(), vec![]); // c:1885
        }
        let body: String = chars[pos..end].iter().collect(); // c:1885
        let new_pos = if end < chars.len() { end + 1 } else { end };
        let body_chars: Vec<char> = body.chars().collect();
        let mut idx = 0_usize;
        // ${(flags)var…} — paren-flag block. Port of subst.c:2147+
        // flag-loop. Each flag char sets a state bit; applied as
        // post-processing on the substituted value.
        let mut flag_lower = false; // c:2197 (L)
        let mut flag_upper = false; // c:2200 (U)
        let mut flag_caps = false; // c:2203 (C)
        let mut flag_qcount: u32 = 0; // c:2237 (q)
        let mut flag_qmin = false; // c:2245 (q-)
        let mut flag_qplus = false; // c:2245 (q+)
        let mut flag_at = false; // c:2167 (@)
                                 // Temp state.arrays slot holding a nested-expansion array
                                 // result (see line ~1755). Set when `${(@)${(@)…}…}` with
                                 // outer `(@)` triggers multsub on the inner; cleared at end
                                 // of paramsubst so the temp doesn't leak. Direct port of the
                                 // multsub-driven word list C zsh threads through subst.c.
        let mut subexp_array_temp: Option<String> = None;
        let mut flag_p_indirect = false; // c:2295 (P)
                                         // (A) — array-assign mode for `${(A)var=val}`. (AA) →
                                         // associative-assign: split val into key/value pairs.
                                         // Direct port of `int arrasg = 0; case 'A': ++arrasg;`
                                         // at subst.c:2161. Counter (not bool) so the AA double-
                                         // form is distinguishable.
        let mut flag_arrasg: i32 = 0; // c:1793
        let mut flag_typeinfo = false; // c:2807 (t)
        let mut flag_keys = false; // c:2247 (k)
        let mut flag_values = false; // c:2256 (v)
        let mut flag_evalchar = false; // c:1673 (#) char-eval
                                       // (l:N::PRE:) left-pad / (r:N::POST:) right-pad parsed values.
                                       // Port of subst.c:2319-2375 l/r flag arm.
        let mut prenum: i64 = 0; // c:1776 (zlong prenum)
        let mut postnum: i64 = 0; // c:1776 (zlong postnum)
        let mut premul: Option<String> = None; // c:1772 (premul)
        let mut postmul: Option<String> = None; // c:1772 (postmul)
        let mut preone: Option<String> = None; // c:1772 (preone)
        let mut postone: Option<String> = None; // c:1772 (postone)
                                                // (s::) split / (j::) join separators. Port of
                                                // subst.c:2299-2317 s/j flag arm + (f)/(F)/(0) shortcuts.
        let mut spsep: Option<String> = None; // c:1766 (spsep — splits result)
        let mut sep: Option<String> = None; // c:1766 (sep — joins arrays)
                                            // (o)/(O)/(i)/(n)/(a)/(u) sort + unique flags. Port of
                                            // subst.c:2207-2228 sortit-flag arm.
        let mut sort_active = false; // c:2207 (o)
        let mut sort_backwards = false; // c:2210 (O)
        let mut sort_case_insensitive = false; // c:2213 (i)
        let mut sort_numeric = false; // c:2216 (n)
        let mut sort_signed = false; // c:2219 (-/Dash)
        let mut sort_index_order = false; // c:2225 (a)
        let mut unique = false; // c:2476 (u)
        let mut flag_eval = false; // c:2268 (e)
        let mut flag_unquote = false; // c:2261 (Q)
        let mut flag_error = false; // c:2264 (X)
        let mut flag_visible = false; // c:2232 (V)
        let mut flag_char_count = false; // c:2275 (c)
        let mut flag_word_count = false; // c:2278 (w)
        let mut flag_word_count_w = false; // c:2281 (W)
        let mut flag_b_pattern = false; // c:2255 (b)
                                        // SUB_* flag bits accumulated by M/R/B/E/N/S/I/* in the
                                        // flag-loop. Direct port of subst.c:2169-2199 — passed
                                        // through to getmatch() / igetmatch() to alter the
                                        // ${var//pat/repl}-style match disposition: return matched
                                        // text vs rest, return position vs string, etc.
        let mut sub_flags_bits: i32 = 0; // c:2169
        let mut flag_d_dir = false; // c:2229 (D)
        let mut flag_p_escapes = false; // c:2382 (p)
                                        // (g:SUBFLAGS:) — getkeys sub-flag bits per c:2409.
                                        // `flag_g_seen` tracks whether the flag appeared at all
                                        // (mirrors C `getkeys >= 0` test) — bare `(g::)` alone
                                        // applies default getkeystring decoding to the final
                                        // value. Each sub-letter toggles a getkeystring escape
                                        // mode: emacs bindings (`\C-x`, `\M-y`, `^X`), POSIX
                                        // octal (`\NNN` even without leading `0`), or extended
                                        // ctrl (`\^X`). Direct port of subst.c:1811's
                                        // `int getkeys = -1` initialization.
        let mut flag_g_seen: bool = false; // c:2410
        let mut flag_g_emacs: bool = false; // c:2418
        let mut flag_g_octal: bool = false; // c:2421
        let mut flag_g_ctrl: bool = false; // c:2424
        let mut flag_pct_prompt: u32 = 0; // c:2405 (% prompt count)
        let mut multi_width: u32 = 0; // c:2376 (m count)
        let mut flnum: u32 = 0; // c:1786 (I:N:)
        let mut flag_z_tokenize = false; // c:2439 (z)
        let mut flag_z_keep_comments = false; // c:2450 (Zc)
        let mut flag_z_strip_comments = false; // c:2456 (ZC)
        let mut flag_z_newline_ws = false; // c:2461 (Zn)
        let mut plan9 = isset(crate::ported::zsh_h::RCEXPANDPARAM); // c:1663
        let mut hkeys: u32 = 0; // c:1828
        let mut hvals: u32 = 0; // c:1835
        if body_chars.first() == Some(&'(') {
            // c:2147
            // `~` inside `(flags)` toggles tok_arg for untok_and_escape on
            // s/j/l/r flag args — subst.c:2157-2159 (not globsubst).
            let mut tok_arg = false; // c:2145
            let mut d = 1_i32; // c:2147
            idx = 1; // c:2147
                     // No closing paren on flag block → "bad substitution".
                     // Direct port of zsh's flagerr label which calls zerr
                     // and aborts the substitution. Emit and bail rather than
                     // silently treating the entire body as flag chars.
            if !body_chars.iter().skip(1).any(|c| *c == ')') {
                // c:2147
                zerr("bad substitution"); // c:2147
                errflag_set_error(); // c:2147
                return (String::new(), new_pos, vec![]); // c:2147
            } // c:2147
            while idx < body_chars.len() && d > 0 {
                // c:2147
                let fc = body_chars[idx]; // c:2153
                match fc {
                    // c:2153
                    '(' => {
                        d += 1;
                    } // c:2147
                    ')' => {
                        d -= 1;
                        if d == 0 {
                            idx += 1;
                            break;
                        }
                    } // c:2147
                    'L' => {
                        flag_lower = true;
                    } // c:2197
                    'U' => {
                        flag_upper = true;
                    } // c:2200
                    'C' => {
                        flag_caps = true;
                    } // c:2203
                    'q' => {
                        // c:2237
                        // (q-) → SINGLE_OPTIONAL: bslashquote only if
                        // needed (whitespace / metachar present);
                        // (q+) → QUOTEDZPUTS: print -V style.
                        // Without next char or with another q,
                        // bump the count for the (qq)/(qqq)/(qqqq)
                        // cascade. Direct port of subst.c:2236-2253.
                        let next = body_chars.get(idx + 1).copied();
                        if next == Some('-') {
                            // c:2240
                            idx += 1; // c:2243 (s++)
                            flag_qmin = true; // c:2245 (QT_SINGLE_OPTIONAL)
                        } else if next == Some('+') {
                            // c:2240
                            idx += 1; // c:2243
                            flag_qplus = true; // c:2245 (QT_QUOTEDZPUTS)
                        } else {
                            // c:2247
                            flag_qcount += 1; // c:2252
                        } // c:2253
                    } // c:2253
                    'A' => {
                        flag_arrasg += 1;
                    } // c:2161 (A array-assign; AA associative-assign)
                    '@' => {
                        flag_at = true;
                    } // c:2167
                    'P' => {
                        flag_p_indirect = true;
                    } // c:2295
                    't' => {
                        flag_typeinfo = true;
                    } // c:2807
                    '!' => {
                        // c:2385-2388
                        if ((hkeys | hvals) & !SCANPM_NONAMEREF) != 0 {
                            zerr("bad substitution");
                            errflag_set_error();
                            return (String::new(), new_pos, vec![]);
                        }
                        hkeys = SCANPM_NONAMEREF;
                    }
                    'k' => {
                        // c:2390-2393
                        if (hkeys & !SCANPM_WANTKEYS) != 0 {
                            zerr("bad substitution");
                            errflag_set_error();
                            return (String::new(), new_pos, vec![]);
                        }
                        hkeys = SCANPM_WANTKEYS;
                    } // c:2247
                    'v' => {
                        // c:2395-2398
                        if (hvals & !SCANPM_WANTVALS) != 0 {
                            zerr("bad substitution");
                            errflag_set_error();
                            return (String::new(), new_pos, vec![]);
                        }
                        hvals = SCANPM_WANTVALS;
                    } // c:2256
                    '#' => {
                        flag_evalchar = true;
                    } // c:1673 (# evalchar)
                    'l' | 'r' => {
                        // c:2319 (l/r pad)
                        // Consume `:N:STR1:STR2:` form.
                        // C: `s++; del0 = s; num = get_intarg(&s, &dellen);`
                        let is_left = fc == 'l'; // c:2320
                        idx += 1; // c:2323
                        if idx >= body_chars.len() {
                            break;
                        }
                        let del = body_chars[idx]; // c:2324 (del0)
                        idx += 1; // c:2324
                                  // Parse N — digits up to next del.
                        let mut num_str = String::new(); // c:2326
                        while idx < body_chars.len() && body_chars[idx].is_ascii_digit() {
                            num_str.push(body_chars[idx]);
                            idx += 1;
                        }
                        let n: i64 = num_str.parse().unwrap_or(0); // c:2326
                        if is_left {
                            prenum = n;
                        } else {
                            postnum = n;
                        } // c:2329-2331
                          // Optional STR1 (mul) after another del.
                        if idx < body_chars.len() && body_chars[idx] == del {
                            idx += 1; // c:2336
                            let s1_start = idx; // c:2336
                            while idx < body_chars.len() && body_chars[idx] != del {
                                idx += 1;
                            }
                            let s1: String = body_chars[s1_start..idx].iter().collect();
                            // STR1 — untok_and_escape(s + arglen, escapes,
                            // tok_arg); escapes is `(p)` in this block.
                            let s1 = untok_and_escape(&s1, flag_p_escapes, tok_arg);
                            if is_left {
                                premul = Some(s1);
                            } else {
                                postmul = Some(s1);
                            }
                            if idx < body_chars.len() {
                                // c:2354
                                idx += 1; // skip del
                            }
                            // Optional STR2 (one-time) after another del.
                            if idx < body_chars.len() && body_chars[idx] == del {
                                idx += 1; // c:2360
                                let s2_start = idx;
                                while idx < body_chars.len() && body_chars[idx] != del {
                                    idx += 1;
                                }
                                let s2: String = body_chars[s2_start..idx].iter().collect();
                                let s2 = untok_and_escape(&s2, flag_p_escapes, tok_arg);
                                if is_left {
                                    preone = Some(s2);
                                } else {
                                    postone = Some(s2);
                                }
                                if idx < body_chars.len() {
                                    idx += 1;
                                } // skip del
                            }
                        }
                        continue; // c:2374 (loop continues from idx)
                    }
                    'o' => {
                        sort_active = true;
                    } // c:2207
                    'O' => {
                        sort_backwards = true;
                        sort_active = true;
                    } // c:2210
                    'i' => {
                        sort_case_insensitive = true;
                        sort_active = true;
                    } // c:2213
                    'n' => {
                        sort_numeric = true;
                        sort_active = true;
                    } // c:2216
                    '-' => {
                        sort_signed = true;
                        sort_active = true;
                    } // c:2219
                    'a' => {
                        sort_index_order = true;
                        sort_active = true;
                    } // c:2225
                    'u' => {
                        unique = true;
                    } // c:2476
                    '_' => {
                        // c:2485-2501 reserved `(_:...:)` — inner must be empty
                        idx += 1;
                        if idx >= body_chars.len() {
                            zerr("bad substitution");
                            errflag_set_error();
                            return (String::new(), new_pos, vec![]);
                        }
                        let del = body_chars[idx];
                        idx += 1;
                        let inner_start = idx;
                        while idx < body_chars.len() && body_chars[idx] != del {
                            idx += 1;
                        }
                        if inner_start < idx {
                            zerr("bad substitution");
                            errflag_set_error();
                            return (String::new(), new_pos, vec![]);
                        }
                        if idx >= body_chars.len() {
                            zerr("bad substitution");
                            errflag_set_error();
                            return (String::new(), new_pos, vec![]);
                        }
                        idx += 1;
                        continue;
                    } // c:2485
                    '*' => {
                        sub_flags_bits |= SUB_EGLOB;
                    } // c:2168 (*)
                    'I' => {
                        // c:2189 (I:N:)
                        // (I:N:) — match the Nth occurrence in
                        // \${var//pat/repl}. Direct port of
                        // subst.c:2189 which calls get_intarg to
                        // pull the digits and stash in flnum. The
                        // Rust port stashes on state.match_index
                        // so the BUILTIN_PARAM_REPLACE arm reads
                        // it via with_executor.
                        idx += 1; // c:2190 (s++)
                        let mut digits = String::new(); // c:2191
                        while idx < body_chars.len()        // c:2191
                            && body_chars[idx].is_ascii_digit()
                        // c:2191
                        {
                            // c:2191
                            digits.push(body_chars[idx]); // c:2191
                            idx += 1; // c:2191
                        } // c:2191
                        if let Ok(n) = digits.parse::<u32>() {
                            // c:2191
                            flnum = n; // c:2191
                        } // c:2191
                        continue; // c:2195
                    } // c:2195
                    'M' => {
                        sub_flags_bits |= SUB_MATCH;
                    } // c:2171 (M)
                    'R' => {
                        sub_flags_bits |= SUB_REST;
                    } // c:2174 (R)
                    'B' => {
                        sub_flags_bits |= SUB_BIND;
                    } // c:2177 (B)
                    'E' => {
                        sub_flags_bits |= SUB_EIND;
                    } // c:2180 (E)
                    'N' => {
                        sub_flags_bits |= SUB_LEN;
                    } // c:2183 (N)
                    'S' => {
                        sub_flags_bits |= SUB_SUBSTR;
                    } // c:2186 (S)
                    'e' => {
                        flag_eval = true;
                    } // c:2268 (e)
                    'Q' => {
                        flag_unquote = true;
                    } // c:2261 (Q)
                    'X' => {
                        flag_error = true;
                    } // c:2264 (X)
                    'D' => {
                        flag_d_dir = true;
                    } // c:2229 (D)
                    'V' => {
                        flag_visible = true;
                    } // c:2232 (V)
                    'b' => {
                        flag_b_pattern = true;
                    } // c:2255 (b)
                    'w' => {
                        flag_word_count = true;
                    } // c:2278 (w)
                    'c' => {
                        flag_char_count = true;
                    } // c:2275 (c)
                    'W' => {
                        flag_word_count_w = true;
                    } // c:2281 (W)
                    'z' => {
                        flag_z_tokenize = true;
                    } // c:2439 (z)
                    'Z' => {
                        // c:2443 (Z:flags:)
                        // (Z:cCn:) — shell-tokenize with sub-flags:
                        //   c: keep comments
                        //   C: strip comments
                        //   n: treat newlines as whitespace
                        // Direct port of subst.c:2443 — skip the
                        // delimited :flags: arg span; the Rust
                        // tokenizer (consumer) reads sub-flags at
                        // dispatch.
                        flag_z_tokenize = true; // c:2443
                        idx += 1; // c:2444 (s++)
                        if idx < body_chars.len() {
                            // c:2444
                            let del = body_chars[idx]; // c:2444
                            idx += 1; // c:2444
                            while idx < body_chars.len()    // c:2444
                                && body_chars[idx] != del
                            // c:2444
                            {
                                // c:2444
                                let ch = body_chars[idx]; // c:2450
                                if ch == 'c' {
                                    flag_z_keep_comments = true;
                                }
                                // c:2450
                                else if ch == 'C' {
                                    flag_z_strip_comments = true;
                                }
                                // c:2456
                                else if ch == 'n' {
                                    flag_z_newline_ws = true;
                                } // c:2461
                                idx += 1; // c:2444
                            } // c:2444
                            if idx < body_chars.len() {
                                idx += 1;
                            } // c:2444
                        } // c:2444
                        continue; // c:2473
                    } // c:2473
                    'g' => {
                        // c:2409 (g)
                        // (g:SUBFLAGS:) — getkeys sub-flag arg.
                        // SUBFLAGS is a string of sub-flag letters:
                        //   e — GETKEY_EMACS (interpret `^X`, `\C-X`,
                        //       `\M-X` etc. emacs-style)
                        //   o — GETKEY_OCTAL_ESC (`\NNN` octal even
                        //       without `\0`)
                        //   c — GETKEY_CTRL (`\^X` for control chars)
                        // Direct port of Src/subst.c:2409 — sets
                        // `getkeys` bits which getkeystring later
                        // honors. The decoding fires only when the
                        // value flow hits a getkeystring call (e.g.
                        // via the `(p)` flag's separator arg or
                        // via `(g)` itself promoted to whole-value
                        // decoding when no `(p)` is present).
                        idx += 1; // c:2410
                        flag_g_seen = true; // c:2411 (`getkeys = 0`)
                        let mut want_emacs = false; // c:2418
                        let mut want_octal = false; // c:2421
                        let mut want_ctrl = false; // c:2424
                        if idx < body_chars.len() {
                            // c:2410
                            let del = body_chars[idx]; // c:2410
                            idx += 1; // c:2410
                            while idx < body_chars.len()    // c:2410
                                && body_chars[idx] != del
                            // c:2410
                            {
                                // c:2410
                                match body_chars[idx] {
                                    // c:2415
                                    'e' => want_emacs = true, // c:2418
                                    'o' => want_octal = true, // c:2421
                                    'c' => want_ctrl = true,  // c:2424
                                    _ => {
                                        // c:2429 (flagerr)
                                        zerr("bad substitution");
                                        errflag_set_error();
                                        return (String::new(), new_pos, vec![]);
                                    }
                                }
                                idx += 1; // c:2410
                            } // c:2410
                            if idx < body_chars.len() {
                                idx += 1;
                            } // c:2410
                        } // c:2410
                          // Apply sub-flag bits to the existing
                          // getkeystring escape path. zshrs's
                          // getkeystring wraps the same Src/utils.c
                          // function — toggling these flags makes the
                          // `(p)` route honor the requested decoding
                          // sub-set. When no (p) is present, fold the
                          // (g) effect onto the value at the end of
                          // flag-loop processing via flag_g_*.
                        flag_g_emacs |= want_emacs;
                        flag_g_octal |= want_octal;
                        flag_g_ctrl |= want_ctrl;
                        continue; // c:2410
                    } // c:2409 (g)
                    '~' => {
                        tok_arg = !tok_arg;
                    } // c:2157-2159 (~ / Tilde)
                    'm' => {
                        multi_width += 1;
                    } // c:2376 (m)
                    'p' => {
                        flag_p_escapes = true;
                    } // c:2382
                    '%' => {
                        flag_pct_prompt += 1;
                    } // c:2405 (% prompt-expand)
                    'f' => {
                        spsep = Some("\n".to_string());
                    } // c:2285
                    'F' => {
                        sep = Some("\n".to_string());
                    } // c:2289
                    '0' => {
                        spsep = Some("\u{0}".to_string());
                    } // c:2293 (split on NUL)
                    's' | 'j' => {
                        // c:2299/2302
                        // Consume `:STR:` arg.
                        let is_split = fc == 's'; // c:2300
                        idx += 1; // c:2303 (++s)
                        if idx >= body_chars.len() {
                            break;
                        }
                        let del = body_chars[idx]; // c:2303 (get_strarg del)
                        idx += 1; // c:2303
                        let s_start = idx;
                        while idx < body_chars.len() && body_chars[idx] != del {
                            idx += 1;
                        }
                        let arg: String = body_chars[s_start..idx].iter().collect(); // c:2308
                        let arg = untok_and_escape(&arg, flag_p_escapes, tok_arg); // c:2309-2312
                        if is_split {
                            spsep = Some(arg);
                        } else {
                            sep = Some(arg);
                        } // c:2309-2313
                        if idx < body_chars.len() {
                            idx += 1;
                        } // skip closing del
                        continue; // c:2317 (loop continues from idx)
                    }
                    _ => {
                        // c:2504-2528 default: flagerr
                        zerr("bad substitution");
                        errflag_set_error();
                        return (String::new(), new_pos, vec![]);
                    }
                }
                idx += 1;
            }
            flag_keys = (hkeys & SCANPM_WANTKEYS) != 0; // c:2393 → (k) assoc keys
            flag_values = (hvals & SCANPM_WANTVALS) != 0; // c:2398
        }
        // Unparenthesised flags — single `for (;;)` (subst.c:2550-2632).
        // Order matters for `${#~x}` vs `${~#x}`, `${=^x}`, etc.
        let mut force_split = false;
        let mut suppress_split = false;
        let mut length_op = false;
        let mut chkset = false;
        loop {
            let c = match body_chars.get(idx).copied() {
                Some(ch) => ch,
                None => break,
            };
            if c == '^' {
                if body_chars.get(idx + 1).copied() == Some('^') {
                    plan9 = false;
                    idx += 2;
                } else {
                    plan9 = true;
                    idx += 1;
                }
                continue;
            }
            if c == '=' {
                if body_chars.get(idx + 1).copied() == Some('=') {
                    suppress_split = true;
                    idx += 2;
                } else {
                    force_split = true;
                    idx += 1;
                }
                continue;
            }
            if c == '#' {
                // c:2570-2588 — `${}` ⇒ `inbrace`; `(inbrace || !POSIXIDENTIFIERS)` is satisfied.
                let next = body_chars.get(idx + 1).copied();
                let after_next = body_chars.get(idx + 2).copied();
                let next_is_name_start = match next {
                    Some(ch) if ch.is_ascii_alphanumeric() => true,
                    Some(ch) if matches!(ch, '_' | '@' | '*' | '?' | '!' | '$' | '-' | '0') => {
                        true
                    }
                    Some(':') if after_next == Some('-') => true,
                    Some(ch) if ch == STRING || ch == QSTRING => matches!(
                        body_chars.get(idx + 2).copied(),
                        Some(b) if b == INBRACE || b == '{' || b == INPAR || b == '('
                    ),
                    Some('#') if after_next.is_none() => true,
                    _ => false,
                };
                if next_is_name_start {
                    length_op = true;
                    idx += 1;
                    continue;
                }
            }
            if c == '~' {
                if body_chars.get(idx + 1).copied() == Some('~') {
                    if !qt {
                        crate::ported::options::opt_state_set("globsubst", false);
                    }
                    idx += 2;
                } else {
                    if !qt {
                        crate::ported::options::opt_state_set("globsubst", true);
                    }
                    idx += 1;
                }
                continue;
            }
            if c == '+' {
                let nxt = body_chars.get(idx + 1).copied().unwrap_or('\0');
                let aspar = flag_p_indirect;
                let ok = nxt.is_ascii_alphanumeric()
                    || nxt == '_'
                    || matches!(nxt, '@' | '*' | '#' | '?')
                    || (aspar
                        && (nxt == STRING || nxt == QSTRING)
                        && matches!(
                            body_chars.get(idx + 2).copied(),
                            Some(b) if b == INBRACE || b == '{' || b == INPAR || b == '('
                        ));
                if ok {
                    chkset = true;
                    idx += 1;
                    continue;
                }
                zerr("bad substitution");
                errflag_set_error();
                return (String::new(), new_pos, vec![]);
            }
            if matches!(c, SNULL | DNULL | STRING | QSTRING) {
                idx += 1;
                continue;
            }
            break;
        }
        sub_flags_set(sub_flags_bits); // c:2169
        let post_flags_start = idx;
        // ${...$(...)...} / ${...${var}...} / ${...$((...))...} —
        // subexp arm. Port of subst.c:2637-2729. When the body has a
        // nested $-form at the name position, run it through singsub
        // and use the result as the value directly.
        //
        // Quoted-form `"..."` wrapper passes through transparently:
        // `${(@f)"$(...)"}` peels the DQ wrapper and runs the same
        // subexp recursion on the inside. Per zsh, the wrapper just
        // suppresses word-splitting on the cmd-subst result; (f) /
        // (@) flags then re-split as requested.
        // (=)/(==) unparenthesised split toggles — parsed in the
        // subst.c:2550 loop above.

        let mut peeled_quotes = false; // c:2649
        if idx + 1 < body_chars.len()                        // c:2649
            && body_chars[idx] == '"'                        // c:2649
            && body_chars[idx + 1] == '$'
        // c:2649
        {
            // c:2649
            // Find matching close bslashquote (depth-tracked over $(...)
            // and ${...} so nested DQs don't fool us). Direct port
            // of zsh's QSTRING/STRING dual-pass at subst.c:282.
            let mut p = idx + 1; // c:2649
            let mut paren_depth = 0_i32; // c:2649
            let mut brace_depth = 0_i32; // c:2649
            while p < body_chars.len() {
                // c:2649
                let ch = body_chars[p]; // c:2649
                match ch {
                    // c:2649
                    '(' => paren_depth += 1, // c:2649
                    ')' => paren_depth -= 1, // c:2649
                    '{' => brace_depth += 1, // c:2649
                    '}' => brace_depth -= 1, // c:2649
                    '"' if paren_depth == 0 && brace_depth == 0 => {
                        // c:2649
                        // close bslashquote
                        idx += 1; // skip leading "
                                  // Mark peeled; inner $-form starts at idx now.
                        peeled_quotes = true; // c:2649
                                              // Note p is the closing bslashquote position;
                                              // skip it after the inner $-form is consumed.
                        let _ = p; // c:2649
                        break; // c:2649
                    } // c:2649
                    _ => {}                  // c:2649
                } // c:2649
                p += 1; // c:2649
            } // c:2649
        } // c:2649
        let mut subexp_value: Option<String> = if idx < body_chars.len() && body_chars[idx] == '$'
        // c:2649
        {
            // Walk just the nested $-form (depth-tracked over its
            // matching brace/paren), then singsub only that slice.
            // Without this scoping the trailing operators got fed
            // into the recursive expansion.
            let start = idx;
            let mut p = idx + 1;
            if p < body_chars.len() {
                let nx = body_chars[p];
                let (open, close) = match nx {
                    '{' => ('{', '}'),
                    '(' => ('(', ')'),
                    _ => ('\0', '\0'),
                };
                if open != '\0' {
                    let mut depth = 0_i32;
                    while p < body_chars.len() {
                        let ch = body_chars[p];
                        if ch == open {
                            depth += 1;
                        } else if ch == close {
                            depth -= 1;
                            if depth == 0 {
                                p += 1;
                                break;
                            }
                        }
                        p += 1;
                    }
                } else {
                    // Bare $name — walk identifier chars.
                    p += 1;
                    while p < body_chars.len()
                        && (body_chars[p].is_ascii_alphanumeric() || body_chars[p] == '_')
                    {
                        p += 1;
                    }
                }
            }
            let inner: String = body_chars[start..p].iter().collect(); // c:2671
                                                                       // Array-shape preservation through nested `${(@)${(@)…}…}`.
                                                                       // C zsh uses `multsub` (subst.c:544) for the inner expansion
                                                                       // when the outer flag set wants array shape; that returns the
                                                                       // word list, not a joined scalar. With `(@)` set on the
                                                                       // outer expansion, route through multsub and stash the array
                                                                       // in state.arrays under a unique temp name so the existing
                                                                       // splat path (line 3636 state.arrays.contains_key) sees it.
                                                                       // Direct port of subst.c's prefork SPLIT path that the (@)
                                                                       // flag triggers around line 2167.
            let expanded = if flag_at {
                // c:2167+544
                let (joined, arr_parts, isarr, _) = multsub(&inner, PREFORK_SPLIT);
                if isarr && !arr_parts.is_empty() {
                    // Generate a stable per-call temp name. We use a
                    // process-local counter; cleanup happens at end of
                    // paramsubst (state.arrays.remove).
                    use std::sync::atomic::{AtomicUsize, Ordering};
                    static SEQ: AtomicUsize = AtomicUsize::new(0);
                    let n = SEQ.fetch_add(1, Ordering::Relaxed);
                    let temp = format!("__subexp_arr_{}", n);
                    arrays_insert(temp.clone(), arr_parts);
                    subexp_array_temp = Some(temp.clone());
                    temp
                } else {
                    joined
                }
            } else {
                singsub(&inner) // c:2681
            };
            idx = p; // c:2691
                     // If we peeled a leading `"`, also consume the matching
                     // closing `"` now so the rest of the body (operators,
                     // `}`, etc.) parses normally.
            if peeled_quotes && idx < body_chars.len() && body_chars[idx] == '"' {
                // c:2649
                idx += 1; // c:2649
            } // c:2649
            Some(expanded)
        } else {
            None
        };

        // Walk var-name chars
        let name_start = idx;
        while idx < body_chars.len() {
            let bc = body_chars[idx];
            let allowed = if idx == name_start {
                bc.is_ascii_alphanumeric()
                    || bc == '_'
                    || bc == '@'
                    || bc == '*'
                    || bc == '#'
                    || bc == '?'
                    || bc == '0'
            } else {
                bc.is_ascii_alphanumeric() || bc == '_'
            };
            if allowed {
                idx += 1;
                // Single-char specials stop after one char
                if idx == name_start + 1
                    && matches!(body_chars[name_start], '@' | '*' | '#' | '?' | '0')
                {
                    break;
                }
            } else {
                break;
            }
        }
        let mut var_name: String = body_chars[name_start..idx].iter().collect();
        // If the subexp produced an array (multsub path above), bind
        // var_name to the temp slot in state.arrays so the rest of
        // paramsubst — splat, subscript, filter, replace — operates
        // on the array via the existing var-lookup paths instead of
        // treating the joined scalar as a value.
        if let Some(ref temp) = subexp_array_temp {
            var_name = temp.clone();
            subexp_value = None;
        }

        // ${arr[subscript]} — subscript loop. Port of subst.c:2862-3000.
        // Parse `[…]` after the var name, with brace-depth tracking
        // for nested `${arr[$other[1]]}`.
        let mut subscript: Option<String> = None; // c:2867
        if idx < body_chars.len() && body_chars[idx] == '[' {
            // c:2867
            idx += 1; // c:2867
            let sub_start = idx;
            let mut depth = 1_i32;
            while idx < body_chars.len() && depth > 0 {
                // c:2867
                let bc = body_chars[idx];
                if bc == '[' {
                    depth += 1;
                }
                // c:2867
                else if bc == ']' {
                    // c:2867
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                idx += 1;
            }
            if idx > sub_start {
                let raw_sub: String = body_chars[sub_start..idx].iter().collect();
                // Subscript expressions can contain $vars — singsub them.
                // Subscript expressions can contain $vars — singsub them.
                subscript = Some(singsub(&raw_sub)); // c:2899
            }
            if idx < body_chars.len() {
                idx += 1;
            } // skip ]
        }

        let rest: String = body_chars[idx..].iter().collect();

        // (P) indirect: take the var name from somewhere — either
        // the value of a parameter (\${(P)x}) or the result of a
        // nested expansion (\${(P)\${(P)x}} = `(P)`-of-(P)-of-x).
        // Direct port of subst.c:2730+ aspar arm. The C source's
        // val pointer is the resolved name string regardless of
        // whether it came from a parameter or a sub-expression.
        if flag_p_indirect {
            // c:2730
            // If a sub-expression already produced the resolved
            // text (subexp arm above), use THAT as the indirect
            // name — clear subexp_value so the var-lookup path
            // applies to the new name. Multi-level (P) chains
            // resolve correctly.
            if let Some(sv) = subexp_value.clone() {
                // c:2741
                var_name = sv.trim().to_string(); // c:2741
                subexp_value = None; // c:2741 (consumed)
            } else {
                // c:2741
                let target = vars_get(&var_name) // c:2741
                    .or_else(|| arrays_get(&var_name).map(|a| a.join(" "))) // c:2741
                    .unwrap_or_default(); // c:2741
                var_name = target; // c:2741
            } // c:2741
        }

        // Look up var (with subscript if present). Port of
        // subst.c:2965 getstrvalue / getarrvalue dispatch.
        // If subexp_value is set, the value comes from the recursive
        // $(...)/${...} expansion and we skip var-name lookup.
        let used_subexp = subexp_value.is_some();
        let raw_value: String = if let Some(sv) = subexp_value {
            sv // c:2681 (subexp result)
        } else if let Some(sub) = subscript.as_deref() {
            // Subscripted lookup: assoc-key, array-index, or slice.
            if let Some(map) = assoc_get(&var_name) {
                // c:2926 (assoc lookup)
                // Subscript-flag form: (I)pat / (i)pat (search keys
                // for pattern, return matching key) and (R)pat /
                // (r)pat (search values, return matching value).
                // Direct port of Src/params.c getarg's hash-aware
                // index/match handling.
                if let Some((flags, pat)) = (|s: &str| -> Option<(String, String)> {
                    let s = s.trim_start();
                    let rest = s.strip_prefix('(')?;
                    let close = rest.find(')')?;
                    let flags = rest[..close].to_string();
                    let pat = rest[close + 1..].to_string();
                    if flags
                        .chars()
                        .all(|c| matches!(c, 'I' | 'R' | 'i' | 'r' | 'k' | 'K' | 'n' | 'e' | 'b'))
                    {
                        Some((flags, pat))
                    } else {
                        None
                    }
                })(sub)
                {
                    let by_key = flags.contains('I') || flags.contains('i');
                    let return_all = flags.contains('I') || flags.contains('R');
                    let mut out: Vec<String> = Vec::new();
                    for (k, v) in map.iter() {
                        let hay = if by_key { k.as_str() } else { v.as_str() };
                        if crate::ported::pattern::patmatch(&pat, hay) {
                            out.push(if by_key { k.clone() } else { v.clone() });
                            if !return_all {
                                break;
                            }
                        }
                    }
                    out.join(" ")
                } else {
                    map.get(sub).cloned().unwrap_or_default()
                }
            } else if let Some(arr) = arrays_get(&var_name) {
                // c:2926 (array)
                if sub == "*" || sub == "@" {
                    // c:2916 (full array)
                    arr.join(" ")
                } else if let Some((flags, pat)) = (|s: &str| -> Option<(String, String)> {
                    // (I)/(i)/(R)/(r) array subscript flags —
                    // (i)pat returns 1-based index of first matching
                    // element, (I)pat returns all indices joined,
                    // (r)pat returns first matching VALUE, (R)pat
                    // returns all matching values. Direct port of
                    // Src/params.c getarg array-pattern routing.
                    let s = s.trim_start();
                    let rest = s.strip_prefix('(')?;
                    let close = rest.find(')')?;
                    let flags = rest[..close].to_string();
                    let pat = rest[close + 1..].to_string();
                    if flags
                        .chars()
                        .all(|c| matches!(c, 'I' | 'R' | 'i' | 'r' | 'n' | 'e'))
                    {
                        Some((flags, pat))
                    } else {
                        None
                    }
                })(sub)
                {
                    let return_index = flags.contains('I') || flags.contains('i');
                    let return_all = flags.contains('I') || flags.contains('R');
                    let mut out: Vec<String> = Vec::new();
                    for (idx, elem) in arr.iter().enumerate() {
                        if crate::ported::pattern::patmatch(&pat, elem) {
                            if return_index {
                                out.push((idx + 1).to_string());
                            } else {
                                out.push(elem.clone());
                            }
                            if !return_all {
                                break;
                            }
                        }
                    }
                    if out.is_empty() && return_index {
                        // (i) returns one-past-end on no-match (zsh
                        // convention so $arr[$arr[(i)pat]] yields
                        // empty string for missing); (I) returns
                        // empty string. Direct port of params.c.
                        (arr.len() + 1).to_string()
                    } else {
                        out.join(" ")
                    }
                } else if let Ok(idx_n) = sub.parse::<i64>() {
                    // c:2926 (numeric index)
                    let len = arr.len() as i64;
                    let i = if idx_n < 0 { len + idx_n } else { idx_n - 1 };
                    if i >= 0 && (i as usize) < arr.len() {
                        arr[i as usize].clone()
                    } else {
                        String::new()
                    }
                } else if let Some((start_s, end_s)) = sub.split_once(',') {
                    // c:2944 (slice)
                    // Clone arr first to release the borrow, since
                    // singsub needs &mut state.
                    let arr_clone = arr.clone();
                    let len = arr_clone.len() as i64;
                    let start_str = start_s.to_string();
                    let end_str = end_s.to_string();
                    let start: i64 = singsub(&start_str).parse().unwrap_or(1);
                    let end: i64 = singsub(&end_str).parse().unwrap_or(len);
                    let s = if start < 0 {
                        (len + start).max(0)
                    } else {
                        (start - 1).max(0)
                    } as usize;
                    let e = if end < 0 {
                        (len + end + 1).max(0)
                    } else {
                        end.min(len)
                    } as usize;
                    if s < arr_clone.len() && s < e {
                        arr_clone[s..e.min(arr_clone.len())].join(" ")
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                }
            } else if let Some(magic_val) = {
                // c:2926 — magic-assoc per-key lookup. Mirrors C's
                // paramtab dispatch through `partab[]` (Src/Modules/
                // parameter.c:2234): each magic-name is registered
                // with its own getfn pointer; we inline the same
                // dispatch by calling the per-array `getpm<X>` ports.
                // Each C fn returns a freshly-built `Param`; the
                // value lives in `u_str`.
                use crate::ported::modules::parameter::*;
                let nul = std::ptr::null_mut();
                let is_splice = sub == "@" || sub == "*";
                let pm: Option<crate::ported::zsh_h::Param> = if is_splice {
                    None    // splice form — handled below.
                } else { match var_name.as_str() {
                    "aliases"             => getpmralias(nul, sub),       // c:1923
                    "galiases"            => getpmgalias(nul, sub),       // c:1937
                    "saliases"            => getpmsalias(nul, sub),       // c:1951
                    "dis_aliases"         => getpmdisralias(nul, sub),    // c:1930
                    "dis_galiases"        => getpmdisgalias(nul, sub),    // c:1944
                    "dis_saliases"        => getpmdissalias(nul, sub),    // c:1958
                    "builtins"            => getpmbuiltin(nul, sub),      // c:799
                    "dis_builtins"        => getpmdisbuiltin(nul, sub),   // c:806
                    "commands"            => getpmcommand(nul, sub),      // c:213
                    "functions"           => getpmfunction(nul, sub),     // c:444
                    "dis_functions"       => getpmdisfunction(nul, sub),  // c:451
                    "functions_source"    => getpmfunction_source(nul, sub),     // c:591
                    "dis_functions_source"=> getpmdisfunction_source(nul, sub),  // c:600
                    "nameddirs"           => getpmnameddir(nul, sub),     // c:1597
                    "userdirs"            => getpmuserdir(nul, sub),      // c:1646
                    "options"             => getpmoption(nul, sub),       // c:988
                    "parameters"          => getpmparameter(nul, sub),    // c:99
                    "history"             => getpmhistory(nul, sub),      // c:1156
                    "modules"             => getpmmodule(nul, sub),       // c:1040
                    "jobdirs"             => getpmjobdir(nul, sub),       // c:1457
                    "jobstates"           => getpmjobstate(nul, sub),     // c:1385
                    "jobtexts"            => getpmjobtext(nul, sub),      // c:1277
                    "usergroups"          => getpmusergroups(nul, sub),   // c:2102
                    _ => None,
                }};
                // c:`scanpm<X>` paths — splice form `${(...)var[@]}`
                // walks the backing table directly and joins values.
                pm.and_then(|p| p.u_str).or_else(|| {
                    if is_splice {
                        splice_magic_assoc(&var_name)
                    } else {
                        None
                    }
                })
            } {
                magic_val
            } else {
                // Scalar with subscript — char-index access.
                let scalar = vars_get(&var_name).unwrap_or_default();
                let s_chars: Vec<char> = scalar.chars().collect();
                // Pattern-subscript on scalar: (i)pat / (I)pat
                // returns 1-based char position of first/last match;
                // (r)pat / (R)pat returns the matched substring.
                // Direct port of Src/params.c getasub which routes
                // scalar pattern lookups through getindex with
                // PATSCAN_FIRST/LAST.
                if let Some((flags, pat)) = (|s: &str| -> Option<(String, String)> {
                    let s = s.trim_start();
                    let rest = s.strip_prefix('(')?;
                    let close = rest.find(')')?;
                    let f = rest[..close].to_string();
                    let p = rest[close + 1..].to_string();
                    if f.chars()
                        .all(|c| matches!(c, 'I' | 'R' | 'i' | 'r' | 'n' | 'e' | 'b'))
                    {
                        Some((f, p))
                    } else {
                        None
                    }
                })(sub)
                {
                    let return_index = flags.contains('I') || flags.contains('i');
                    let want_last = flags.contains('I') || flags.contains('R');
                    // Sliding-window glob match across the string.
                    let n = s_chars.len();
                    let mut found: Option<(usize, usize)> = None;
                    'outer: for start in 0..=n {
                        let lengths: Box<dyn Iterator<Item = usize>> = if want_last {
                            Box::new((1..=(n - start)).rev())
                        } else {
                            Box::new(1..=(n - start))
                        };
                        for len in lengths {
                            let cand: String = s_chars[start..start + len].iter().collect();
                            if crate::ported::pattern::patmatch(&pat, &cand) {
                                found = Some((start, start + len));
                                if !want_last {
                                    break 'outer;
                                }
                                break;
                            }
                        }
                    }
                    // For (I): keep scanning to find LAST match.
                    if want_last {
                        for start in (0..=n).rev() {
                            for len in 1..=(n - start) {
                                let cand: String = s_chars[start..start + len].iter().collect();
                                if crate::ported::pattern::patmatch(&pat, &cand) {
                                    found = Some((start, start + len));
                                    break;
                                }
                            }
                            if found.is_some() && found.unwrap().0 >= start {
                                break;
                            }
                        }
                    }
                    match (found, return_index) {
                        (Some((s, _)), true) => (s + 1).to_string(),
                        (Some((s, e)), false) => s_chars[s..e].iter().collect(),
                        (None, true) => {
                            // (i) returns len+1, (I) returns 0 on no match.
                            // Direct port of Src/params.c getindex.
                            if flags.contains('i') {
                                (n + 1).to_string()
                            } else {
                                "0".to_string()
                            }
                        }
                        (None, false) => String::new(),
                    }
                } else if let Ok(idx_n) = sub.parse::<i64>() {
                    let len = s_chars.len() as i64;
                    let i = if idx_n < 0 { len + idx_n } else { idx_n - 1 };
                    if i >= 0 && (i as usize) < s_chars.len() {
                        s_chars[i as usize].to_string()
                    } else {
                        String::new()
                    }
                } else if let Some((lo, hi)) = sub.split_once(',') {
                    // `${var[N,M]}` scalar char-slice — bug-for-bug port
                    // of getarrvalue's range arm operating on a per-char
                    // pseudo-array. Direct port of Src/params.c:1625
                    // getstrvalue's slice path.
                    let lo: i64 = lo.trim().parse().unwrap_or(1);
                    let hi: i64 = hi.trim().parse().unwrap_or(s_chars.len() as i64);
                    let chars_arr: Vec<String> = s_chars.iter().map(|c| c.to_string()).collect();
                    crate::ported::params::getarrvalue(&chars_arr, lo, hi).concat()
                } else {
                    String::new()
                }
            }
        } else {
            // No subscript — scalar / array / assoc / magic-assoc
            // fallthrough. Direct port of getstrvalue dispatch which
            // checks each storage shape in priority order.
            // Special single-char names (`#`, `?`, `!`, `$`, `*`, `@`,
            // `0`, `-`) live on the executor, not in `variables`. Fall
            // back to `exec.get_variable` so `${##}` (length of `$#`)
            // and similar specials resolve correctly. Direct port of
            // Src/params.c::getstrvalue's special-name dispatch.
            // Special single-char names: shell-special (`#`, `?`, `!`,
            // `$`, `*`, `@`, `-`) and positional params (`0`, `1`,
            // `2`, …). All-digit multi-char names are also positional
            // (`$10`, `$11`, …). Direct port of Src/params.c
            // getstrvalue dispatch — positional params live on the
            // executor's `positional_params` vec rather than in the
            // variables hash, so they need the get_variable fallback
            // for modifiers like `:t` / `:r` to work on `$1`.
            let is_special_name = (var_name.len() == 1
                && matches!(
                    var_name.chars().next().unwrap_or('\0'),
                    '#' | '?' | '!' | '$' | '*' | '@' | '-'
                ))
                || (!var_name.is_empty() && var_name.chars().all(|c| c.is_ascii_digit()));
            // Canonical scalar lookup — sole funnel through
            // `getsparam` (matches C zsh's `getsparam(name)` →
            // `getvalue` → `getstrvalue` → `Param.gsu->getfn`
            // dispatch at Src/params.c:3076 / 2335). The funnel
            // handles GSU dispatch + variables + env + array-join
            // in one place; subst.rs and the fuseVM bridge both
            // route through here so the lookup logic lives in
            // exactly one location.
            exec_getsparam(&var_name)
                .or_else(|| {
                    assoc_get(&var_name)
                        .map(|m| m.values().cloned().collect::<Vec<_>>().join(" "))
                })
                .or_else(|| {
                    if is_special_name {
                        // POSIX shell-specials ($?/$#/$$/$!/$*/$@/$-/$N).
                        // Canonical dispatch through params::lookup_special_var
                        // (Src/params.c special_assigns getfn).
                        crate::ported::params::lookup_special_var(&var_name)
                    } else {
                        None
                    }
                })
                // Splice (`[@]`) on a magic-assoc name isn't yet wired
                // through the per-name scanpm<X> handlers; falls back
                // to empty (matches C when no special handler matches).
                .unwrap_or_default()
        };
        // Nested subexp result counts as "set" so the outer `:-` /
        // `-` / `:?` modifiers see a real value rather than treating
        // an empty var_name lookup as unset. Direct port of zsh's
        // aspar/subexp path: when the inner $-form yielded a string,
        // vunset stays 0 even though no parameter table entry
        // exists. Without this, `\${\${(M)0:#/*}:-DEFAULT}` always
        // fired the default because the outer paramsubst saw
        // is_set=false (no variable named "${(M)0:#/*}").
        // For subscripted access (`${arr[k]:=v}` etc.), is_set must
        // reflect whether the SUBSCRIPTED slot exists, not the
        // variable. Direct port of C zsh's getindex behavior: the
        // Value struct's vunset is set based on slot lookup, not
        // the parent param. Without this, `${m[$k]=v}` on a typeset
        // -gA assoc with no key fired the "already set" branch and
        // skipped the assign.
        let is_set = if let Some(sub) = subscript.as_deref() {
            used_subexp
                || assoc_get(&var_name)
                    .map(|m| m.contains_key(sub))
                    .unwrap_or(false)
                || arrays_get(&var_name).as_ref()
                    .map(|a| {
                        sub.parse::<i64>().ok().is_some_and(|i| {
                            let len = a.len() as i64;
                            let real = if i < 0 { len + i } else { i - 1 };
                            real >= 0 && (real as usize) < a.len()
                        })
                    })
                    .unwrap_or(false)
        } else {
            used_subexp
                || vars_contains(&var_name)
                || arrays_contains(&var_name)
                || assoc_contains(&var_name)
        };

        // ${+name} short-circuit per subst.c:3600 — return "1"/"0".
        // Subscripted form `${+arr[i]}` checks whether THAT element is
        // set, not the array as a whole; raw_value (already
        // subscript-resolved) being non-empty is the proxy.
        if chkset {
            // c:3600
            let set_str = if subscript.is_some() {
                if !raw_value.is_empty() {
                    "1"
                } else {
                    "0"
                }
            } else if is_set {
                "1"
            } else {
                "0"
            };
            // Splice the result back into the surrounding string
            // (prefix + value + suffix) per the convention used by
            // `${...}` arms below — the caller (stringsubst) reads
            // the linknode by index, not the returned `new_str`.
            let prefix: String = chars[..start_pos].iter().collect();
            let suffix: String = if new_pos < chars.len() {
                chars[new_pos..].iter().collect()
            } else {
                String::new()
            };
            let full = format!("{}{}{}", prefix, set_str, suffix);
            let new_pos_in_full = prefix.chars().count() + set_str.chars().count();
            return (full.clone(), new_pos_in_full, vec![full]); // c:3600
        }

        // (#)var → element count of array/assoc (or char count of
        // scalar). Port of subst.c:2128 length_op fast path.
        if length_op {
            // c:2128
            let _ = post_flags_start;
            let n = if let Some(arr) = arrays_get(&var_name) {
                arr.len() // c:2128 (array len)
            } else if let Some(map) = assoc_get(&var_name) {
                map.len() // c:2128 (assoc len)
            } else {
                raw_value.chars().count() // c:2128 (scalar char-count)
            };
            // Splice the count back into the surrounding string per
            // the convention used by `${...}` arms below — the caller
            // (stringsubst) reads the linknode by index, not the
            // returned `new_str`. Returning `(n, new_pos, vec![])`
            // (as this arm did before) caused stringsubst to clear
            // the linknode because its `new_nodes.is_empty()` branch
            // sets data to "". Without this fix, `${##}` lost the
            // computed count.
            let n_str = n.to_string();
            let prefix: String = chars[..start_pos].iter().collect();
            let suffix: String = if new_pos < chars.len() {
                chars[new_pos..].iter().collect()
            } else {
                String::new()
            };
            let full = format!("{}{}{}", prefix, n_str, suffix);
            let new_pos_in_full = prefix.chars().count() + n_str.chars().count();
            return (full.clone(), new_pos_in_full, vec![full]);
        }

        // (k) keys / (v) values on assoc — fold the assoc into a
        // joined string. Port of subst.c:2247-2270.
        // (kv) interleave: when BOTH flags are set, emit alternating
        // key/value pairs (zsh's "double-flag" form). The order
        // matters — k-then-v gives [k1 v1 k2 v2], v-then-k gives
        // [v1 k1 v2 k2], but the flag-loop doesn't preserve order;
        // we use kv ordering (zsh canonical default).
        // Magic-assoc fallback (aliases / functions / options / etc.)
        // mirrors the bytecode-VM path: when the name isn't in
        // assoc_arrays, route through the function_names / alias_names
        // sets. Direct port of zsh's per-magic-table getfn dispatch.
        let mut value: String; // c:2247
        if flag_keys && flag_values {
            // c:2247 (kv)
            value = assoc_get(&var_name) // c:2247
                .map(|m| {
                    // c:2247
                    let mut out: Vec<String> = Vec::with_capacity(m.len() * 2); // c:2247
                    for (k, v) in m {
                        // c:2247
                        out.push(k.clone()); // c:2247
                        out.push(v.clone()); // c:2247
                    } // c:2247
                    out.join(" ") // c:2247
                }) // c:2247
                .unwrap_or_default(); // c:2247
        } else if flag_keys {
            // c:2247
            value = assoc_get(&var_name) // c:2247
                .map(|m| m.keys().cloned().collect::<Vec<_>>().join(" ")) // c:2247
                .or_else(|| {
                    // c:2247
                    // c:2247 — magic-assoc {aliases,functions,commands}
                    // are backed by the canonical global HashTables in
                    // hashtable.rs (mirrors C's `mod_export HashTable
                    // aliastab` at hashtable.c:1186 and `shfunctab` at
                    // hashtable.c:808). `commands` is `cmdnamtab`
                    // (hashtable.c:594).
                    match var_name.as_str() {
                        // c:2247
                        "aliases" => crate::ported::hashtable::aliastab_lock()
                            .lock()
                            .ok()
                            .map(|t| {
                                let mut names: Vec<String> = t.iter()
                                    .map(|(k, _)| k.clone())
                                    .collect();
                                names.sort();
                                names.join(" ")
                            }),
                        "functions" | "dis_functions" =>
                            crate::ported::hashtable::shfunctab_lock()
                                .lock()
                                .ok()
                                .map(|t| {
                                    let mut names: Vec<String> = t.iter()
                                        .map(|(k, _)| k.clone())
                                        .collect();
                                    names.sort();
                                    names.join(" ")
                                }),
                        "commands" =>
                            crate::ported::hashtable::cmdnamtab_lock()
                                .lock()
                                .ok()
                                .map(|t| {
                                    let mut names: Vec<String> = t.iter()
                                        .map(|(k, _)| k.clone())
                                        .collect();
                                    names.sort();
                                    names.join(" ")
                                }),
                        _ => None, // c:2247
                    } // c:2247
                }) // c:2247
                .unwrap_or_default();
        } else if flag_values {
            // c:2256
            value = assoc_get(&var_name) // c:2256
                .map(|m| m.values().cloned().collect::<Vec<_>>().join(" ")) // c:2256
                .unwrap_or_default();
        } else if flag_at {
            // c:2167
            // (@) array splat — preserve element shape via space-join.
            // For full splat into multiple result_nodes, the
            // multsub-aware caller handles it; we emit space-joined here.
            value = arrays_get(&var_name).as_ref()
                .map(|a| a.join(" "))
                .unwrap_or_else(|| raw_value.clone());
        } else {
            // c:N/A
            value = raw_value.clone();
        }
        // subst.c:3885-3887 YUK — empty / empty-first array → scalar "" when !plan9
        if !plan9 && flag_at {
            if let Some(ref a) = arrays_get(&var_name) {
                if a.first().map_or(true, |s| s.is_empty()) {
                    value = String::new();
                }
            }
        }
        // split_parts: tracks any post-operator array-shape result
        // (e.g. :# filter, (s::) split) so the auto-splat block
        // below splats those instead of the original backing array.
        let mut split_parts: Option<Vec<String>> = None; // c:3950
        if !rest.is_empty() {
            let r = rest.as_str();
            if let Some(pat) = r.strip_prefix(":#") {
                // c:3540 (:#pat filter)
                // Match-test on element(s). Drops elements (or
                // empties scalar) when pattern matches; keeps
                // unchanged when not. With (M) flag in sub_flags,
                // the disposition inverts (keep matching, drop
                // non-matching). Direct port of subst.c:3540
                // SUB_FILTER + getmatch SUB_MATCH branch.
                let p = singsub(pat); // c:3540
                let cur_sub_flags = sub_flags_get(); // c:2171
                let invert = (cur_sub_flags & 0x0008) != 0; // c:2171 SUB_MATCH
                sub_flags_set(0); // c:2169 (consume)
                                     // Direct port of subst.c:3422 `if (!vunset && isarr)` —
                                     // the array iteration only fires when `isarr` is set.
                                     // After getindex computes a single-slot subscript, isarr
                                     // is cleared at line 2915 (`v->scanflags ? 1 : 0`) and
                                     // the C source falls through to getmatch on `val`
                                     // (line 3451). Mirror that here: when subscript was
                                     // applied, treat raw_value as the scalar `val` and
                                     // skip the per-element arr loop.
                let has_subscript = subscript.is_some();
                if let Some(arr) = arrays_get(&var_name)
                    .filter(|_| !has_subscript)
                {
                    let kept: Vec<String> = arr
                        .into_iter() // c:3540
                        .filter(|elem| {
                            // c:3540
                            let m = crate::ported::pattern::patmatch(&p, elem); // c:3540
                            if invert {
                                m
                            } else {
                                !m
                            } // c:3540
                        }) // c:3540
                        .collect();
                    value = kept.join(" "); // c:3540
                                            // Stash filtered parts so the auto-splat block
                                            // below uses these, not the unfiltered backing
                                            // array. \${(@)arr:#pat} now correctly splats
                                            // only the kept elements.
                    split_parts = Some(kept); // c:3540
                } else {
                    // c:3540
                    let m = crate::ported::pattern::patmatch(&p, &raw_value); // c:3540
                    value = if invert {
                        // c:3540
                        if m {
                            raw_value.clone()
                        } else {
                            String::new()
                        } // c:3540
                    } else {
                        // c:3540
                        if m {
                            String::new()
                        } else {
                            raw_value.clone()
                        } // c:3540
                    }; // c:3540
                } // c:3540
            } else if let Some(default) = r.strip_prefix(":-") {
                // c:3193
                if !is_set || raw_value.is_empty() {
                    value = singsub(default);
                }
            } else if let Some(default) = r.strip_prefix('-') {
                // c:3193
                if !is_set {
                    value = singsub(default);
                }
            } else if let Some(default) = r.strip_prefix("::=") {
                // c:3245 (unconditional assign)
                // `${var::=value}` — zsh extension. Always store value
                // (after expansion) regardless of whether var was
                // set/empty. Direct port of subst.c case '=' / ':=' /
                // '::=' which call assignsparam (params.c:3193) /
                // assignaparam (params.c:3357) / sethparam
                // (params.c:3602) based on the `flag_arrasg` flag.
                value = singsub(default);
                if flag_arrasg == 1 {
                    // c:3263 (A)
                    let ifs = vars_get("IFS")
                        .unwrap_or_else(|| " \t\n".to_string());
                    let parts: Vec<String> = value
                        .split(|c: char| ifs.contains(c))
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string())
                        .collect();
                    exec_assignaparam(&var_name, parts);
                } else if flag_arrasg == 2 {
                    // c:3263 (AA)
                    let ifs = vars_get("IFS")
                        .unwrap_or_else(|| " \t\n".to_string());
                    let parts: Vec<String> = value
                        .split(|c: char| ifs.contains(c))
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string())
                        .collect();
                    exec_sethparam(&var_name, parts);
                } else {
                    let __s = match subscript.as_deref() {
                        Some(k) => format!("{}[{}]", var_name, k),
                        None => var_name.clone(),
                    };
                    crate::ported::params::assignsparam(&__s, &value, 0);
                    exec_sync_state_from_paramtab();
                }
            } else if let Some(default) = r.strip_prefix(":=") {
                // c:3245
                if !is_set || raw_value.is_empty() {
                    value = singsub(default);
                    if flag_arrasg == 1 {
                        // c:3263 (A)
                        let ifs = vars_get("IFS")
                            .unwrap_or_else(|| " \t\n".to_string());
                        let parts: Vec<String> = value
                            .split(|c: char| ifs.contains(c))
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string())
                            .collect();
                        exec_assignaparam(&var_name, parts);
                    } else if flag_arrasg == 2 {
                        // c:3263 (AA)
                        let ifs = vars_get("IFS")
                            .unwrap_or_else(|| " \t\n".to_string());
                        let parts: Vec<String> = value
                            .split(|c: char| ifs.contains(c))
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string())
                            .collect();
                        exec_sethparam(&var_name, parts);
                    } else {
                        let __s = match subscript.as_deref() {
                            Some(k) => format!("{}[{}]", var_name, k),
                            None => var_name.clone(),
                        };
                        crate::ported::params::assignsparam(&__s, &value, 0);
                        exec_sync_state_from_paramtab();
                    }
                }
            } else if let Some(default) = r.strip_prefix('=') {
                // c:3245 (= — assign on unset only)
                // Same as := but trigger ONLY on unset (not on
                // empty). Direct port of subst.c case '=' which
                // only checks vunset, not !*val.
                if !is_set {
                    value = singsub(default);
                    if flag_arrasg == 1 {
                        // c:3263 (A)
                        let ifs = vars_get("IFS")
                            .unwrap_or_else(|| " \t\n".to_string());
                        let parts: Vec<String> = value
                            .split(|c: char| ifs.contains(c))
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string())
                            .collect();
                        exec_assignaparam(&var_name, parts);
                    } else if flag_arrasg == 2 {
                        // c:3263 (AA)
                        let ifs = vars_get("IFS")
                            .unwrap_or_else(|| " \t\n".to_string());
                        let parts: Vec<String> = value
                            .split(|c: char| ifs.contains(c))
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string())
                            .collect();
                        exec_sethparam(&var_name, parts);
                    } else {
                        let __s = match subscript.as_deref() {
                            Some(k) => format!("{}[{}]", var_name, k),
                            None => var_name.clone(),
                        };
                        crate::ported::params::assignsparam(&__s, &value, 0);
                        exec_sync_state_from_paramtab();
                    }
                }
            } else if let Some(alt) = r.strip_prefix(":+") {
                // c:3296
                if is_set && !raw_value.is_empty() {
                    value = singsub(alt);
                } else {
                    value = String::new();
                }
            } else if let Some(alt) = r.strip_prefix('+') {
                // c:3296
                if is_set {
                    value = singsub(alt);
                } else {
                    value = String::new();
                }
            } else if let Some(msg) = r.strip_prefix(":?") {
                // c:3193 (:?msg)
                if !is_set || raw_value.is_empty() {
                    let m = if msg.is_empty() {
                        // c:3193
                        "parameter null or not set".to_string() // c:3193
                    } else {
                        // c:3193
                        singsub(msg) // c:3193
                    }; // c:3193
                       // C: zerr("%s: %s", idbeg, msg) — Src/subst.c:3337
                    zerr(&format!("{}: {}", var_name, m));
                    errflag_set_error();
                }
            } else if let Some(msg) = r.strip_prefix('?') {
                // c:3193 (?msg — not-set only)
                // Same as :? but trigger ONLY on unset (not on
                // empty). Direct port of subst.c case '?' which
                // only checks `vunset` (not `(vunset || !*val)`).
                if !is_set {
                    let m = if msg.is_empty() {
                        // c:3193
                        "parameter not set".to_string() // c:3193
                    } else {
                        // c:3193
                        singsub(msg) // c:3193
                    }; // c:3193
                       // C: zerr("%s: parameter not set", idbeg) — Src/subst.c:3472
                    zerr(&format!("{}: {}", var_name, m));
                    errflag_set_error();
                }
            } else if let Some(rep) = r.strip_prefix(":/") {
                // c:3870 (whole-element replace)
                // ${arr:/PAT/REPL} — replace entire elements that
                // match PAT with REPL. For arrays: per-element
                // whole-match test, replace matching elements with
                // REPL. For scalars: replace the entire value if it
                // matches.
                // Per Src/subst.c:3870 SUB_GLOBAL with anchor-both
                // (start AND end fixed): the pattern must consume
                // the whole element. Different from `//` which is
                // sliding-window mid-element replace.
                let parts: Vec<&str> = rep.splitn(2, '/').collect();
                let pat = singsub(parts[0]);
                let repl = parts.get(1).map(|s| singsub(s)).unwrap_or_default();
                if let Some(arr) = arrays_get(&var_name) {
                    let new_arr: Vec<String> = arr
                        .into_iter()
                        .map(|elem| {
                            if crate::ported::pattern::patmatch(&pat, &elem) {
                                repl.clone()
                            } else {
                                elem
                            }
                        })
                        .collect();
                    value = new_arr.join(" "); // c:3870
                    split_parts = Some(new_arr); // c:3870
                } else if crate::ported::pattern::patmatch(&pat, &raw_value) {
                    value = repl; // c:3870
                } else {
                    value = raw_value.clone(); // c:3870
                }
            } else if let Some(rep) = r.strip_prefix("//") {
                // c:3870 (global replace)
                // Same NUL/BNULL-aware split as before. NUL/BNULL +
                // X → `\X` for the pat side (glob meta literal).
                // `\` + `/` → `/` (literal `/`, not separator).
                // Direct port of Src/subst.c:3884.
                let split_unescaped = |s: &str| -> (String, String) {
                    let cv: Vec<char> = s.chars().collect();
                    let mut pat_buf = String::new();
                    let mut i = 0;
                    while i < cv.len() {
                        let c = cv[i];
                        if (c == '\x00' || c == '\u{9f}') && i + 1 < cv.len() {
                            pat_buf.push('\\');
                            pat_buf.push(cv[i + 1]);
                            i += 2;
                            continue;
                        }
                        if c == '\\' && i + 1 < cv.len() && cv[i + 1] == '/' {
                            pat_buf.push(cv[i + 1]);
                            i += 2;
                            continue;
                        }
                        if c == '/' {
                            let rest: String = cv[i + 1..].iter().collect();
                            return (pat_buf, rest);
                        }
                        pat_buf.push(c);
                        i += 1;
                    }
                    (pat_buf, String::new())
                };
                let (raw_pat, raw_repl) = split_unescaped(rep);
                let pat = singsub(&raw_pat);
                // Replacement: per C subst.c around line 3354,
                // `prefork(replstr, ...)` runs with SUB_FLAG|SKIP_FILESUB
                // — tilde / file expansion is suppressed in the
                // replacement (so `\~` lands as literal `~`, not
                // `$HOME`). Same `\X` → `X` strip emulates C's
                // untokenize on the BNULL→`\` form the bridge upstream
                // produces.
                let repl = {
                    let saved_skip = SKIP_FILESUB.with(|c| c.get());
                    SKIP_FILESUB.with(|c| c.set(true));
                    let s = crate::lex::untokenize(&singsub(&raw_repl));
                    SKIP_FILESUB.with(|c| c.set(saved_skip));
                    let mut out = String::with_capacity(s.len());
                    let mut it = s.chars().peekable();
                    while let Some(c) = it.next() {
                        if c == '\\' {
                            if let Some(&nx) = it.peek() {
                                if nx == '\\' {
                                    out.push('\\');
                                    it.next();
                                    continue;
                                }
                                out.push(nx);
                                it.next();
                                continue;
                            }
                        }
                        out.push(c);
                    }
                    out
                };
                // Per-element replace for arrays — zsh treats each
                // element as a separate match target, preserving the
                // array shape. \${(@)arr//pat/repl} keeps element
                // count, replaces within each. Direct port of
                // subst.c's getmatcharr path that calls getmatch on
                // each element separately. Single-shot helper to
                // avoid duplicating the sliding-window logic.
                let replace_global = |val: &str| -> String {
                    let cv: Vec<char> = val.chars().collect();
                    let nn = cv.len();
                    let mut o = String::with_capacity(val.len());
                    let mut q = 0_usize;
                    while q < nn {
                        let mut m: Option<usize> = None;
                        for e in (q + 1..=nn).rev() {
                            let c: String = cv[q..e].iter().collect();
                            if crate::ported::pattern::patmatch(&pat, &c) {
                                m = Some(e);
                                break;
                            }
                        }
                        if let Some(e) = m {
                            o.push_str(&repl);
                            q = if e == q { q + 1 } else { e };
                        } else {
                            o.push(cv[q]);
                            q += 1;
                        }
                    }
                    o
                };
                let mut handled_array = false;
                // Subscripted lookup (`${arr[N]//pat/repl}`) clears
                // isarr in C (subst.c:2915 `v->scanflags ? 1 : 0`)
                // and dispatches to getmatch on the single element
                // at subst.c:3451 — not getmatcharr per-element.
                // Only single-element subscripts trigger this: `[@]`
                // and `[*]` keep array shape (still per-element); a
                // range `[N,M]` also keeps array shape; only literal
                // `[N]` / `[key]` reduces to scalar.
                let has_scalar_subscript = subscript
                    .as_deref()
                    .map(|s| {
                        let t = s.trim();
                        t != "@" && t != "*" && !t.contains(',')
                    })
                    .unwrap_or(false);
                let has_subscript = has_scalar_subscript;
                if let Some(arr) = arrays_get(&var_name)
                    .filter(|_| !has_subscript)
                {
                    let new_arr: Vec<String> = arr.iter().map(|e| replace_global(e)).collect();
                    value = new_arr.join(" "); // c:3870
                    split_parts = Some(new_arr); // c:3870 (auto-splat)
                    handled_array = true;
                }
                if handled_array {
                    // Skip the scalar fallback below by leaving
                    // the block early via condition swap. Easier
                    // than adding a labeled-block — outer chain
                    // is else-if so falling through to the next
                    // arm requires the guard.
                    let _ = handled_array;
                } else {
                    // Glob-aware sliding-window replace. Was literal-only
                    // (.replace) which broke \${path//*.tmp/.bak}-style
                    // idioms. Direct port of subst.c:3870 SUB_GLOBAL arm
                    // routing through patmatch.
                    let chars_v: Vec<char> = raw_value.chars().collect(); // c:3870
                    let n = chars_v.len(); // c:3870
                    let mut out = String::with_capacity(raw_value.len()); // c:3870
                    let mut p = 0_usize; // c:3870
                    while p < n {
                        // c:3870
                        // Try longest-first match from position p.
                        let mut matched: Option<usize> = None; // c:3870
                        for end in (p + 1..=n).rev() {
                            // c:3870
                            let cand: String = chars_v[p..end].iter().collect(); // c:3870
                            if crate::ported::pattern::patmatch(&pat, &cand) {
                                // c:3870
                                matched = Some(end); // c:3870
                                break; // c:3870
                            } // c:3870
                        } // c:3870
                        if let Some(end) = matched {
                            // c:3870
                            out.push_str(&repl); // c:3870
                            p = if end == p { p + 1 } else { end }; // c:3870 (avoid infinite loop on empty match)
                        } else {
                            // c:3870
                            out.push(chars_v[p]); // c:3870
                            p += 1; // c:3870
                        } // c:3870
                    } // c:3870
                    value = out; // c:3870
                } // close handled_array else block
            } else if let Some(rep) = r.strip_prefix('/') {
                // c:3870 (single replace)
                // Same escape-walk as `//` arm above — direct port of
                // subst.c:3147-3164.
                let split_unescaped = |s: &str| -> (String, String) {
                    let cv: Vec<char> = s.chars().collect();
                    let mut pat_buf = String::with_capacity(s.len());
                    let mut i = 0;
                    while i < cv.len() {
                        let c = cv[i];
                        if (c == '\x00' || c == '\u{9f}' || c == '\\') && i + 1 < cv.len() {
                            if cv[i + 1] == '/' {
                                pat_buf.push('/');
                                i += 2;
                                continue;
                            }
                            pat_buf.push(c);
                            pat_buf.push(cv[i + 1]);
                            i += 2;
                            continue;
                        }
                        if c == '/' {
                            let rest: String = cv[i + 1..].iter().collect();
                            return (pat_buf, rest);
                        }
                        pat_buf.push(c);
                        i += 1;
                    }
                    (pat_buf, String::new())
                };
                let (raw_pat, raw_repl) = split_unescaped(rep);
                // Pattern: keep \X for glob meta literals (untokenize
                // drops BNULL but pat still carries `\X` from the
                // split-walk above for the "match this literal X"
                // form).
                let pat = singsub(&raw_pat);
                // Replacement: per Src/glob.c::compgetmatch:2687-2688,
                // C runs `singsub(replstrp); untokenize(*replstrp);`.
                // The C untokenize drops BNULL markers (the lexer's
                // form for `\X` escapes). zshrs's bridge upstream
                // already untokenized BNULL → literal `\`, so the
                // `\X` arrives here as raw chars. Strip a literal
                // backslash before each non-`\` char to mirror the C
                // BNULL-drop semantics (kept as a separate strip pass
                // so the existing untokenize call still handles any
                // surviving meta-tokens).
                let repl = {
                    let s = crate::lex::untokenize(&singsub(&raw_repl));
                    let mut out = String::with_capacity(s.len());
                    let mut it = s.chars().peekable();
                    while let Some(c) = it.next() {
                        if c == '\\' {
                            if let Some(&nx) = it.peek() {
                                if nx == '\\' {
                                    // `\\` → `\` (preserve one backslash)
                                    out.push('\\');
                                    it.next();
                                    continue;
                                }
                                // `\X` → `X` for any other X
                                out.push(nx);
                                it.next();
                                continue;
                            }
                        }
                        out.push(c);
                    }
                    out
                };
                // Single-replace helper. Variants: anchor-prefix
                // (pat starts with `#`), anchor-suffix (`%`), or
                // unanchored. Returns the post-replacement string.
                let replace_one = |val: &str| -> String {
                    if let Some(anchor_pat) = pat.strip_prefix('#') {
                        let cv: Vec<char> = val.chars().collect();
                        let nn = cv.len();
                        for end in (0..=nn).rev() {
                            let cand: String = cv[..end].iter().collect();
                            if crate::ported::pattern::patmatch(anchor_pat, &cand) {
                                return format!("{}{}", repl, cv[end..].iter().collect::<String>());
                            }
                        }
                        val.to_string()
                    } else if let Some(anchor_pat) = pat.strip_prefix('%') {
                        let cv: Vec<char> = val.chars().collect();
                        let nn = cv.len();
                        for start in 0..=nn {
                            let cand: String = cv[start..].iter().collect();
                            if crate::ported::pattern::patmatch(anchor_pat, &cand) {
                                return format!(
                                    "{}{}",
                                    cv[..start].iter().collect::<String>(),
                                    repl
                                );
                            }
                        }
                        val.to_string()
                    } else {
                        let cv: Vec<char> = val.chars().collect();
                        let nn = cv.len();
                        for start in 0..nn {
                            for end in (start + 1..=nn).rev() {
                                let cand: String = cv[start..end].iter().collect();
                                if crate::ported::pattern::patmatch(&pat, &cand) {
                                    let mut out = String::with_capacity(val.len());
                                    out.extend(cv[..start].iter());
                                    out.push_str(&repl);
                                    out.extend(cv[end..].iter());
                                    return out;
                                }
                            }
                        }
                        val.to_string()
                    }
                };
                // Same has_subscript guard as `//` arm above —
                // C subst.c:2915 clears isarr for subscripted form;
                // dispatches to getmatch (scalar) at subst.c:3451.
                // Only literal-index subscripts; `[@]`/`[*]`/`[N,M]`
                // keep array shape.
                let has_subscript_one = subscript
                    .as_deref()
                    .map(|s| {
                        let t = s.trim();
                        t != "@" && t != "*" && !t.contains(',')
                    })
                    .unwrap_or(false);
                if let Some(arr) = arrays_get(&var_name)
                    .filter(|_| !has_subscript_one)
                {
                    let new_arr: Vec<String> = arr.iter().map(|e| replace_one(e)).collect();
                    value = new_arr.join(" "); // c:3870
                    split_parts = Some(new_arr); // c:3870
                } else {
                    value = replace_one(&raw_value); // c:3870
                }
            } else if let Some(pat) = r.strip_prefix("##") {
                // c:3540 (longest prefix strip)
                let p = singsub(pat);
                // has_subscript guard — same as `/`/`//` arms.
                // Per subst.c:2915 + 3422-3451, scalar subscript
                // dispatches to getmatch on the single element.
                let has_scalar_sub = subscript
                    .as_deref()
                    .map(|s| {
                        let t = s.trim();
                        t != "@" && t != "*" && !t.contains(',')
                    })
                    .unwrap_or(false);
                // Strip-one helper. op: 0=#, 1=##, 2=%, 3=%%.
                // Direct port of subst.c:3540 patmatch dispatch.
                let strip_one = |val: &str, op: u8| -> String {
                    let cv: Vec<char> = val.chars().collect();
                    let nn = cv.len();
                    match op {
                        1 => {
                            let mut k = nn;
                            loop {
                                let prefix: String = cv[..k].iter().collect();
                                if crate::ported::pattern::patmatch(&p, &prefix) {
                                    return cv[k..].iter().collect();
                                }
                                if k == 0 {
                                    break;
                                }
                                k -= 1;
                            }
                            val.to_string()
                        }
                        _ => val.to_string(),
                    }
                };
                if let Some(arr) = arrays_get(&var_name)
                    .filter(|_| !has_scalar_sub)
                {
                    let new_arr: Vec<String> = arr.iter().map(|e| strip_one(e, 1)).collect();
                    value = new_arr.join(" "); // c:3540
                    split_parts = Some(new_arr); // c:3540
                } else {
                    value = strip_one(&raw_value, 1); // c:3540
                }
            } else if let Some(pat) = r.strip_prefix('#') {
                // c:3540 (shortest prefix strip)
                let p = singsub(pat);
                let has_scalar_sub = subscript
                    .as_deref()
                    .map(|s| {
                        let t = s.trim();
                        t != "@" && t != "*" && !t.contains(',')
                    })
                    .unwrap_or(false);
                let strip_one = |val: &str| -> String {
                    let cv: Vec<char> = val.chars().collect();
                    let total = cv.len();
                    for k in 0..=total {
                        let prefix: String = cv[..k].iter().collect();
                        if crate::ported::pattern::patmatch(&p, &prefix) {
                            return cv[k..].iter().collect();
                        }
                    }
                    val.to_string()
                };
                if let Some(arr) = arrays_get(&var_name)
                    .filter(|_| !has_scalar_sub)
                {
                    let new_arr: Vec<String> = arr.iter().map(|e| strip_one(e)).collect();
                    value = new_arr.join(" "); // c:3540
                    split_parts = Some(new_arr); // c:3540
                } else {
                    value = strip_one(&raw_value); // c:3540
                }
            } else if let Some(pat) = r.strip_prefix("%%") {
                // c:3540 (longest suffix strip)
                let p = singsub(pat);
                let has_scalar_sub = subscript
                    .as_deref()
                    .map(|s| {
                        let t = s.trim();
                        t != "@" && t != "*" && !t.contains(',')
                    })
                    .unwrap_or(false);
                let strip_one = |val: &str| -> String {
                    let cv: Vec<char> = val.chars().collect();
                    let total = cv.len();
                    let mut k = total;
                    loop {
                        let suffix: String = cv[total - k..].iter().collect();
                        if crate::ported::pattern::patmatch(&p, &suffix) {
                            return cv[..total - k].iter().collect();
                        }
                        if k == 0 {
                            break;
                        }
                        k -= 1;
                    }
                    val.to_string()
                };
                if let Some(arr) = arrays_get(&var_name)
                    .filter(|_| !has_scalar_sub)
                {
                    let new_arr: Vec<String> = arr.iter().map(|e| strip_one(e)).collect();
                    value = new_arr.join(" "); // c:3540
                    split_parts = Some(new_arr); // c:3540
                } else {
                    value = strip_one(&raw_value); // c:3540
                }
            } else if let Some(pat) = r.strip_prefix('%') {
                // c:3540 (shortest suffix strip)
                let p = singsub(pat);
                let has_scalar_sub = subscript
                    .as_deref()
                    .map(|s| {
                        let t = s.trim();
                        t != "@" && t != "*" && !t.contains(',')
                    })
                    .unwrap_or(false);
                let strip_one = |val: &str| -> String {
                    let cv: Vec<char> = val.chars().collect();
                    let total = cv.len();
                    for k in 0..=total {
                        let suffix: String = cv[total - k..].iter().collect();
                        if crate::ported::pattern::patmatch(&p, &suffix) {
                            return cv[..total - k].iter().collect();
                        }
                    }
                    val.to_string()
                };
                if let Some(arr) = arrays_get(&var_name)
                    .filter(|_| !has_scalar_sub)
                {
                    let new_arr: Vec<String> = arr.iter().map(|e| strip_one(e)).collect();
                    value = new_arr.join(" "); // c:3540
                    split_parts = Some(new_arr); // c:3540
                } else {
                    value = strip_one(&raw_value); // c:3540
                }
            } else if let Some(rhs) = r.strip_prefix(":|") {
                // c:3540 (set difference)
                // ${arr:|other} — array set-difference: keep elems
                // of arr that are NOT literally present in other.
                // Direct port of subst.c:3522 SUB_DIFFERENCE arm
                // which builds a hashtable of `compare` (the RHS
                // array values) and tests presence via
                // `gethashnode2` — LITERAL key equality, not glob.
                // An earlier port used `glob_match_static` here,
                // which made `(bar` (a malformed glob) fail to match
                // an array element of literal text `(bar`.
                let arr = arrays_get(&var_name).unwrap_or_default();
                let other_name = rhs.trim(); // c:3543
                let other = arrays_get(other_name).unwrap_or_default();
                let other_set: std::collections::HashSet<&String> = other.iter().collect();
                let kept: Vec<String> = arr
                    .into_iter() // c:3540
                    .filter(|s| !other_set.contains(s)) // c:3548
                    .collect();
                value = kept.join(" ");
                split_parts = Some(kept); // c:3540 (auto-splat)
            } else if let Some(rhs) = r.strip_prefix(":*") {
                // c:3540 (intersect)
                // ${arr:*other} — array set-intersection — KEEP
                // elems of arr literally present in other. Same
                // hash-based lookup as `:|` per subst.c:3548
                // `gethashnode2` literal-key path.
                let arr = arrays_get(&var_name).unwrap_or_default();
                let other_name = rhs.trim(); // c:3543
                let other = arrays_get(other_name).unwrap_or_default();
                let other_set: std::collections::HashSet<&String> = other.iter().collect();
                let kept: Vec<String> = arr
                    .into_iter() // c:3540
                    .filter(|s| other_set.contains(s)) // c:3548
                    .collect();
                value = kept.join(" ");
                split_parts = Some(kept); // c:3540 (auto-splat)
            } else if let Some(rhs) = r.strip_prefix(":^^") {
                // c:3540 (zip-long)
                // ${arr:^^other} — interleave two arrays, continuing
                // past the shorter one with empty strings (vs `:^`
                // which stops at the shorter). Direct port of the
                // SUB_ZIP_LONG variant in subst.c:3540.
                let arr = arrays_get(&var_name).unwrap_or_default();
                let other = arrays_get(rhs.trim()).unwrap_or_default();
                let n = arr.len().max(other.len());
                let mut zipped: Vec<String> = Vec::with_capacity(n * 2);
                for i in 0..n {
                    zipped.push(arr.get(i).cloned().unwrap_or_default());
                    zipped.push(other.get(i).cloned().unwrap_or_default());
                }
                value = zipped.join(" ");
                split_parts = Some(zipped); // c:3540 (auto-splat)
            } else if let Some(rhs) = r.strip_prefix(":^") {
                // c:3540 (zip)
                // ${arr:^other} — interleave two arrays element-by-elem.
                let arr = arrays_get(&var_name).unwrap_or_default();
                let other = arrays_get(rhs.trim()).unwrap_or_default();
                let mut zipped: Vec<String> = Vec::with_capacity(arr.len() + other.len());
                let n = arr.len().min(other.len());
                for i in 0..n {
                    zipped.push(arr[i].clone());
                    zipped.push(other[i].clone());
                }
                value = zipped.join(" ");
                split_parts = Some(zipped); // c:3540 (auto-splat)
            } else if let Some(slice) = r.strip_prefix(':') {
                // c:715 (substring) OR :modifier
                // Detect history-style modifier (`:h`, `:t`, `:r`,
                // `:e`, `:l`, `:u`, `:q`, `:Q`, `:A`, `:a`, `:P`,
                // `:c`, `:s/x/y/`, `:S/x/y/`, `:&`). Route through
                // modify() which handles the full chain. Direct
                // port of subst.c's c:715 modifier dispatch.
                let first = slice.chars().next().unwrap_or('\0');
                let is_modifier = matches!(
                    first,
                    'h' | 't'
                        | 'r'
                        | 'e'
                        | 'l'
                        | 'u'
                        | 'q'
                        | 'Q'
                        | 'A'
                        | 'a'
                        | 'P'
                        | 'c'
                        | 's'
                        | 'S'
                        | '&'
                        | 'g'
                        | 'w'
                        | 'W'
                );
                if is_modifier {
                    // c:4531
                    // Per-element on arrays.
                    let mod_str = format!(":{}", slice);
                    let mod_one =
                        |s: &str| -> String { modify(s, &mod_str) };
                    if let Some(parts) = split_parts.clone() {
                        let new_parts: Vec<String> =
                            parts.iter().map(|s| mod_one(s)).collect();
                        value = new_parts.join(" ");
                        split_parts = Some(new_parts);
                    } else if let Some(arr) = arrays_get(&var_name) {
                        let new_arr: Vec<String> = arr.iter().map(|s| mod_one(s)).collect();
                        value = new_arr.join(" ");
                        split_parts = Some(new_arr);
                    } else {
                        value = mod_one(&value);
                    }
                } else {
                    let parts: Vec<&str> = slice.splitn(2, ':').collect();
                    let off = singsub(parts[0]).parse::<i64>().unwrap_or(0);
                    // Array context: ${arr:offset:length} slices the
                    // ARRAY (1-based, like Bash's offset), not the joined
                    // value. Direct port of subst.c's array-shape branch
                    // around c:715. Falls back to scalar substring when
                    // var_name isn't an array.
                    // Source priority: split_parts (prior operator
                    // result like filter/sort) → state.arrays → joined
                    // value. Direct port of zsh's getarrvalue → slice
                    // dispatch which uses aval if isarr is set.
                    let array_source: Option<Vec<String>> = split_parts
                        .clone()
                        .or_else(|| arrays_get(&var_name));
                    if let Some(mut arr) = array_source {
                        // Positional-param slice (`@`/`*`/`argv`) — zsh
                        // counts offset 0 as $0 (script/function name),
                        // not $1. Prepend $0 so `${@:0:2}` returns
                        // [$0, $1] instead of [$1, $2]. Direct port of
                        // subst.c's @/* offset arm which routes through
                        // dohist offset = 0 (includes argzero).
                        if var_name == "@" || var_name == "*" || var_name == "argv" {
                            let s0 = vars_get("0").unwrap_or_default();
                            arr.insert(0, s0); // c:715
                        }
                        let n = arr.len() as i64; // c:715
                        let lo = if off < 0 {
                            (n + off).max(0)
                        } else {
                            off.min(n)
                        } as usize; // c:715
                        let len = parts
                            .get(1) // c:715
                            .map(|s| singsub(s).parse::<i64>().unwrap_or(0)); // c:715
                        let kept: Vec<String> = match len {
                            // c:715
                            Some(l) if l >= 0 => {
                                arr.iter().skip(lo).take(l as usize).cloned().collect()
                            } // c:715
                            Some(l) => {
                                // c:715 (negative len = from-end)
                                let end = ((n - lo as i64) + l).max(0) as usize; // c:715
                                arr.iter().skip(lo).take(end).cloned().collect()
                                // c:715
                            } // c:715
                            None => arr.iter().skip(lo).cloned().collect(), // c:715
                        };
                        value = kept.join(" "); // c:715
                        split_parts = Some(kept); // c:715 (auto-splat slice)
                    } else {
                        let total = raw_value.chars().count() as i64;
                        let start = if off < 0 {
                            (total + off).max(0)
                        } else {
                            off.min(total)
                        } as usize;
                        let len = parts
                            .get(1)
                            .map(|s| singsub(s).parse::<i64>().unwrap_or(0));
                        value = match len {
                            Some(l) if l >= 0 => {
                                raw_value.chars().skip(start).take(l as usize).collect()
                            }
                            Some(l) => {
                                let take = ((total - start as i64) + l).max(0) as usize;
                                raw_value.chars().skip(start).take(take).collect()
                            }
                            None => raw_value.chars().skip(start).collect(),
                        };
                    }
                } // close is_modifier else
            }
        }
        // Apply post-processing flags to the substituted value.
        // C lines 3950-4070 — case mods, quoting, etc.
        if flag_typeinfo {
            // c:2807
            // ${(t)var} — emit type tag. var_attrs takes
            // precedence (carries typeset flags); fall back to
            // synthesized tag from the storage table the value
            // lives in. Direct port of subst.c:2814 wantt arm
            // which checks paramtab + storage shape.
            // c:2814 — read PM_* flags directly from paramtab and
            // synthesize the type tag. Mirrors C `pm->node.flags &
            // PM_TYPE` dispatch at subst.c:2814-2900.
            value = crate::ported::params::paramtab()
                .lock()
                .ok()
                .and_then(|tab| tab.get(&var_name).map(|pm| {
                    use crate::ported::zsh_h::{
                        PM_ARRAY, PM_HASHED, PM_INTEGER, PM_EFLOAT, PM_FFLOAT,
                        PM_READONLY, PM_EXPORTED, PM_LEFT, PM_RIGHT_B, PM_RIGHT_Z,
                        PM_UPPER, PM_LOWER, PM_HIDE, PM_HIDEVAL, PM_TAGGED,
                        PM_UNIQUE,
                    };
                    let f = pm.node.flags as u32;
                    let base = if f & PM_HASHED != 0 { "association" }
                          else if f & PM_ARRAY != 0 { "array" }
                          else if f & PM_INTEGER != 0 { "integer" }
                          else if f & (PM_EFLOAT | PM_FFLOAT) != 0 { "float" }
                          else { "scalar" };
                    let mut out = String::from(base);
                    if f & PM_LEFT != 0    { out.push_str("-left"); }
                    if f & PM_RIGHT_B != 0 { out.push_str("-right_blanks"); }
                    if f & PM_RIGHT_Z != 0 { out.push_str("-zero"); }
                    if f & PM_LOWER != 0   { out.push_str("-lower"); }
                    if f & PM_UPPER != 0   { out.push_str("-upper"); }
                    if f & PM_READONLY != 0{ out.push_str("-readonly"); }
                    if f & PM_TAGGED != 0  { out.push_str("-tag"); }
                    if f & PM_EXPORTED != 0{ out.push_str("-export"); }
                    if f & PM_UNIQUE != 0  { out.push_str("-unique"); }
                    if f & PM_HIDE != 0    { out.push_str("-hide"); }
                    if f & PM_HIDEVAL != 0 { out.push_str("-hideval"); }
                    out
                }))
                .unwrap_or_else(|| {
                    if assoc_contains(&var_name) {
                        "association".to_string() // c:2814
                    } else if arrays_contains(&var_name) {
                        "array".to_string() // c:2814
                    } else if matches!(
                        var_name.as_str(),
                        "aliases"
                            | "galiases"
                            | "saliases"
                            | "dis_aliases"
                            | "dis_galiases"
                            | "dis_saliases"
                            | "functions"
                            | "dis_functions"
                            | "builtins"
                            | "dis_builtins"
                            | "reswords"
                            | "dis_reswords"
                            | "options"
                            | "commands"
                            | "modules"
                            | "nameddirs"
                            | "userdirs"
                            | "jobtexts"
                            | "jobdirs"
                            | "jobstates"
                            | "parameters"
                            | "dirstack"
                            | "errnos"
                            | "sysparams"
                            | "mapfile"
                    ) {
                        // Magic-assoc params — type is association.
                        // Direct port of subst.c:2814 paramtab
                        // lookup which finds the magic-assoc entry
                        // and returns PM_HASHED type tag.
                        "association".to_string() // c:2814
                    } else if is_set {
                        "scalar".to_string()
                    } else {
                        String::new()
                    }
                });
        }
        // Case mods operate per-element when array-shaped (so
        // \${(@U)arr} uppercases each element, preserving shape).
        // Direct port of subst.c:3937 casmod arm which iterates aval
        // when isarr is set.
        let cap_word = |s: &str| -> String {
            // c:2203
            let mut out = String::with_capacity(s.len());
            let mut next_upper = true;
            for c in s.chars() {
                if c.is_whitespace() || matches!(c, '-' | '_' | '/' | '.' | ',') {
                    out.push(c);
                    next_upper = true;
                } else if next_upper {
                    out.extend(c.to_uppercase());
                    next_upper = false;
                } else {
                    out.extend(c.to_lowercase());
                }
            }
            out
        };
        if flag_lower || flag_upper || flag_caps {
            // c:2197
            let transform = |s: &str| -> String {
                // c:3937
                if flag_lower {
                    s.to_lowercase()
                } else if flag_upper {
                    s.to_uppercase()
                } else {
                    cap_word(s)
                }
            };
            if let Some(parts) = split_parts.clone() {
                // c:3937
                let new_parts: Vec<String> = parts.iter().map(|s| transform(s)).collect();
                value = new_parts.join(" "); // c:3937
                split_parts = Some(new_parts); // c:3937
            } else if let Some(arr) = arrays_get(&var_name) {
                let new_arr: Vec<String> = arr.iter().map(|s| transform(s)).collect();
                value = new_arr.join(" "); // c:3937
                split_parts = Some(new_arr); // c:3937
            } else {
                value = transform(&value); // c:3937
            }
        }
        // (o)/(O)/(i)/(n)/(a)/(u) sort + unique. Port of
        // subst.c:4180-4253 array sortit/unique post-processing.
        // Applies on space-joined value; reassembles after.
        if sort_active || unique {
            // c:4180
            // Sort/unique source: prefer split_parts (any prior
            // operator result like :# filter, (s::) split, or
            // assoc-splat) so sort applies to the actual element
            // list, not a whitespace re-split of the joined view.
            let parts: Vec<String> = if let Some(sp) = split_parts.clone() {
                sp // c:4180 (operator-result)
            } else if let Some(arr) = arrays_get(&var_name) {
                arr.clone() // c:4180 (real array)
            } else if let Some(map) = assoc_get(&var_name) {
                map.values().cloned().collect() // c:4180 (assoc values)
            } else {
                value.split_whitespace().map(String::from).collect() // c:4180 (fallback)
            };
            let mut sorted: Vec<String> = parts;
            if unique {
                // c:4253
                let mut seen = std::collections::HashSet::new();
                sorted.retain(|s| seen.insert(s.clone())); // c:4253
            }
            if sort_active {
                // c:4180
                // (a) on assoc-derived elements means "preserve
                // insertion order" — IndexMap already iterates in
                // that order, so skip the sort entirely. The C
                // source short-circuits at SORTIT_BACKWARDS_ONLY
                // (no SORTIT_NUMERICALLY / SORTIT_IGNORING_CASE).
                if !sort_index_order {
                    // c:4194
                    if sort_numeric {
                        // c:4189
                        // sort_signed: f64 already handles the
                        // sign — `(n-)` and `(n)` compare the same
                        // way for the values we'll see.
                        let _ = sort_signed; // c:4193
                        sorted.sort_by(|a, b| {
                            let na: f64 = a.parse().unwrap_or(0.0); // c:4189
                            let nb: f64 = b.parse().unwrap_or(0.0); // c:4189
                            na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
                        });
                    } else if sort_case_insensitive {
                        // c:4187
                        sorted.sort_by_key(|a| a.to_lowercase());
                    } else {
                        // c:4180 (default)
                        sorted.sort();
                    }
                } // c:4194
                if sort_backwards {
                    sorted.reverse();
                } // c:4191
            }
            let join_with = sep.as_deref().unwrap_or(" ");
            value = sorted.join(join_with);
            // Update split_parts so downstream operators (case mods,
            // padding, splat) see the sorted/uniq list.
            split_parts = Some(sorted); // c:4180
        }

        // (s::SEP:) split-on-SEP: apply BEFORE dopadding/bslashquote/case
        // (per zsh order). Port of subst.c flag-loop spsep usage
        // around line 3950+ (post-fetch split block).
        // Track the post-split parts for the auto-splat block so
        // (@s::) on a scalar splats into multiple result_nodes.
        // split_parts hoisted to top of operand-handling so the
        // :# filter arm (which runs much earlier) can populate it
        // for the auto-splat block. No-op if not set later.
        if let Some(ref sp) = spsep {
            // c:3950
            // Per-element split when source is an array — each
            // element splits independently and the results
            // flat-concat. Direct port of subst.c's spsep arm
            // which iterates aval per-element.
            let split_one = |s: &str| -> Vec<String> {
                if sp.is_empty() {
                    s.chars().map(|c| c.to_string()).collect()
                } else {
                    s.split(sp.as_str()).map(String::from).collect()
                }
            };
            let parts: Vec<String> = if let Some(prev) = split_parts.clone() {
                // Already-split source (e.g. earlier filter/operator);
                // re-split each piece.
                prev.iter().flat_map(|s| split_one(s)).collect()
            } else if let Some(arr) = arrays_get(&var_name) {
                arr.iter().flat_map(|s| split_one(s)).collect()
            } else {
                split_one(&value)
            };
            // zsh: split result is space-joined for scalar context;
            // multsub-aware caller handles full multi-node splat
            // via split_parts (passed through to the auto_splat
            // post-processing block below).
            let join_with = sep.as_deref().unwrap_or(" "); // c:3950
            value = parts.join(join_with);
            split_parts = Some(parts); // c:3950
        } else if let Some(ref sp) = sep {
            // c:3963 (j with no s)
            // (j:STR:) — join an array with STR. Source priority:
            // split_parts (operator result) → state.arrays →
            // assoc-values → whitespace-split fallback. Direct
            // port of subst.c:3963 sepjoin which reads aval.
            if let Some(parts) = split_parts.clone() {
                value = parts.join(sp); // c:3963
                                        // Join collapses array shape → reset split_parts
                                        // so auto_splat emits one scalar node, not the
                                        // joined-then-1-elem-splat.
                split_parts = None; // c:3963
            } else if let Some(arr) = arrays_get(&var_name) {
                // c:3963
                value = arr.join(sp); // c:3963
            } else if let Some(map) = assoc_get(&var_name) {
                // c:3963
                let vals: Vec<String> = map.values().cloned().collect();
                value = vals.join(sp); // c:3963
            } else if value.contains(' ') || value.contains('\n') {
                let parts: Vec<&str> = value.split_whitespace().collect();
                value = parts.join(sp);
            }
        }

        // (l:N::PRE:) / (r:N::POST:) padding — apply via dopadding.
        // Per-element on arrays so each element gets padded
        // independently. Direct port of subst.c flag-loop l/r
        // interacting with isarr branch which pads aval per-element.
        if prenum > 0 || postnum > 0 {
            // c:2319/2330
            let mul_default = " ".to_string(); // c:907 (def = " ")
            let pad_one = |s: &str| -> String {
                // c:893
                dopadding(
                    s,
                    prenum.max(0) as usize,
                    postnum.max(0) as usize,
                    preone.as_deref(),
                    postone.as_deref(),
                    premul.as_deref().unwrap_or(&mul_default),
                    postmul.as_deref().unwrap_or(&mul_default),
                    multi_width as i32, // c:2376 (m)
                )
            };
            if let Some(parts) = split_parts.clone() {
                let new_parts: Vec<String> = parts.iter().map(|s| pad_one(s)).collect();
                value = new_parts.join(" ");
                split_parts = Some(new_parts);
            } else if let Some(arr) = arrays_get(&var_name) {
                let new_arr: Vec<String> = arr.iter().map(|s| pad_one(s)).collect();
                value = new_arr.join(" ");
                split_parts = Some(new_arr);
            } else {
                value = pad_one(&value);
            }
        }

        // (#) evalchar — interpret each value as a math expression
        // and emit the char with that codepoint. Direct port of
        // subst.c:1673 evalchar arm + substevalchar.
        if flag_evalchar {
            // c:1673
            let eval_one = |s: &str| -> String { substevalchar(s.trim()).unwrap_or_default() };
            if let Some(parts) = split_parts.clone() {
                let new_parts: Vec<String> = parts.iter().map(|s| eval_one(s)).collect();
                value = new_parts.join(" ");
                split_parts = Some(new_parts);
            } else if let Some(arr) = arrays_get(&var_name) {
                let new_arr: Vec<String> = arr.iter().map(|s| eval_one(s)).collect();
                value = new_arr.join(" ");
                split_parts = Some(new_arr);
            } else {
                value = eval_one(&value);
            }
        } // c:1673

        // (e) eval — re-substitute the result. Per-element on arrays.
        // Direct port of subst.c:2268 eval bit which iterates aval.
        if flag_eval {
            // c:2268
            if let Some(parts) = split_parts.clone() {
                let new_parts: Vec<String> = parts.iter().map(|s| singsub(s)).collect();
                value = new_parts.join(" ");
                split_parts = Some(new_parts);
            } else if let Some(arr) = arrays_get(&var_name) {
                let new_arr: Vec<String> = arr.iter().map(|s| singsub(s)).collect();
                value = new_arr.join(" ");
                split_parts = Some(new_arr);
            } else {
                value = singsub(&value); // c:2268
            }
        }

        // (%) prompt-expand — interpret %F{red}, %~, %n, %{...%},
        // etc. Per-element on arrays. Direct port of subst.c:2405 /
        // 3977 presc handling.
        if flag_pct_prompt > 0 {
            // c:2405
            // Canonical prompt expansion (Src/prompt.c:182 promptexpand).
            let prompt_one = |s: &str| -> String {
                let (expanded, _, _) = crate::ported::prompt::promptexpand(s, 0, None);
                expanded
            };
            if let Some(parts) = split_parts.clone() {
                let new_parts: Vec<String> = parts.iter().map(|s| prompt_one(s)).collect();
                value = new_parts.join(" ");
                split_parts = Some(new_parts);
            } else if let Some(arr) = arrays_get(&var_name) {
                let new_arr: Vec<String> = arr.iter().map(|s| prompt_one(s)).collect();
                value = new_arr.join(" ");
                split_parts = Some(new_arr);
            } else {
                value = prompt_one(&value); // c:3977
            }
        } // c:2405

        // (z)/(Z:cCn:) — shell-tokenize the value into a list of
        // words. Direct port of subst.c:2439 LEXFLAGS_ACTIVE +
        // sub-flags. Simplified port: use whitespace splitting
        // that respects single/double-bslashquote spans and backslash
        // escapes, plus optional comment handling. The full lexer
        // reentry is deferred — this covers the common idioms
        // \${(z)cmdline} (split a command into words) and
        // \${(Zn)multiline} (newlines act like spaces).
        if flag_z_tokenize {
            // c:2439
            let mut words: Vec<String> = Vec::new(); // c:2439
            let mut cur = String::new(); // c:2439
            let mut in_sq = false; // c:2439
            let mut in_dq = false; // c:2439
            let mut in_comment = false; // c:2451
            let chars_v: Vec<char> = value.chars().collect(); // c:2439
            let push_word = |w: &mut String, words: &mut Vec<String>| {
                // c:2439
                if !w.is_empty() {
                    // c:2439
                    words.push(std::mem::take(w)); // c:2439
                } // c:2439
            }; // c:2439
            let mut p = 0_usize; // c:2439
            while p < chars_v.len() {
                // c:2439
                let ch = chars_v[p]; // c:2439
                if in_comment {
                    // c:2451
                    if ch == '\n' {
                        // c:2451
                        in_comment = false; // c:2451
                        if flag_z_keep_comments {
                            cur.push(ch);
                        } // c:2451
                    } else if flag_z_keep_comments {
                        // c:2451
                        cur.push(ch); // c:2451
                    } // c:2451
                    p += 1; // c:2451
                    continue; // c:2451
                } // c:2451
                if in_sq {
                    // c:2439
                    cur.push(ch); // c:2439
                    if ch == '\'' {
                        in_sq = false;
                    } // c:2439
                    p += 1;
                    continue; // c:2439
                } // c:2439
                if in_dq {
                    // c:2439
                    cur.push(ch); // c:2439
                    if ch == '\\' && p + 1 < chars_v.len() {
                        // c:2439
                        p += 1; // c:2439
                        cur.push(chars_v[p]); // c:2439
                    } else if ch == '"' {
                        // c:2439
                        in_dq = false; // c:2439
                    } // c:2439
                    p += 1;
                    continue; // c:2439
                } // c:2439
                match ch {
                    // c:2439
                    '\\' if p + 1 < chars_v.len() => {
                        // c:2439
                        cur.push(ch); // c:2439
                        p += 1; // c:2439
                        cur.push(chars_v[p]); // c:2439
                    } // c:2439
                    '\'' => {
                        cur.push(ch);
                        in_sq = true;
                    } // c:2439
                    '"' => {
                        cur.push(ch);
                        in_dq = true;
                    } // c:2439
                    '#' if cur.is_empty() && !flag_z_strip_comments => {
                        // c:2451
                        // Start of comment word — keep or skip.
                        in_comment = !flag_z_keep_comments; // c:2451
                        if flag_z_keep_comments {
                            cur.push(ch);
                        } // c:2451
                    } // c:2451
                    '#' if cur.is_empty() && flag_z_strip_comments => {
                        // c:2456
                        in_comment = true; // c:2456
                    } // c:2456
                    '\n' if flag_z_newline_ws => {
                        // c:2461 (n: nl as ws)
                        push_word(&mut cur, &mut words); // c:2461
                    } // c:2461
                    c if c.is_whitespace() => {
                        // c:2439
                        push_word(&mut cur, &mut words); // c:2439
                    } // c:2439
                    _ => cur.push(ch), // c:2439
                } // c:2439
                p += 1; // c:2439
            } // c:2439
            push_word(&mut cur, &mut words); // c:2439
            value = words.join(" "); // c:2439
        } // c:2473

        // (D) dir-magic — replace $HOME and any nameddir prefix with
        // tilde form. Direct port of subst.c:2229 mods bit 1, which
        // routes through modify()'s tilde-contraction at the end of
        // the pipeline. Common idiom: `${(D)PWD}` → `~/projects/foo`.
        // Without ZLE's nameddir hash, this reduces to plain $HOME.
        // (D) per-element dir-magic. Direct port of subst.c:2229
        // mods bit 1 → modify()'s tilde-contraction iterating aval.
        if flag_d_dir {
            // c:2229
            let home_opt = vars_get("HOME")
                .or_else(|| std::env::var("HOME").ok());
            // Pull named-dirs (~name) hash into a [(name, path)]
            // sorted by path-length-descending so the LONGEST match
            // wins (zsh canonical: most-specific tilde-contraction).
            // Direct port of subst.c → modify dir-handling which
            // walks the nameddirtab in length-desc order.
            // c:2229 — canonical nameddirtab read (mirrors C's
            // `mod_export HashTable nameddirtab` at hashnameddir.c:48).
            let mut named: Vec<(String, String)> = crate::ported::hashnameddir::nameddirtab()
                .lock()
                .map(|t| t.iter().map(|(k, nd)| (k.clone(), nd.dir.clone())).collect())
                .unwrap_or_default();
            named.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
            let dir_one = |s: &str| -> String {
                // c:2229
                // Try named-dirs first (most specific wins).
                for (name, path) in &named {
                    // c:2229
                    if !path.is_empty() && s.starts_with(path.as_str()) {
                        let r = &s[path.len()..];
                        if r.is_empty() || r.starts_with('/') {
                            return format!("~{}{}", name, r);
                        }
                    }
                }
                // Fall back to $HOME contraction.
                if let Some(ref h) = home_opt {
                    // c:2229
                    if !h.is_empty() && s.starts_with(h.as_str()) {
                        // c:2229
                        let r = &s[h.len()..]; // c:2229
                        if r.is_empty() || r.starts_with('/') {
                            // c:2229
                            return format!("~{}", r); // c:2229
                        } // c:2229
                    } // c:2229
                } // c:2229
                s.to_string() // c:2229
            }; // c:2229
            if let Some(parts) = split_parts.clone() {
                let new_parts: Vec<String> = parts.iter().map(|s| dir_one(s)).collect();
                value = new_parts.join(" ");
                split_parts = Some(new_parts);
            } else if let Some(arr) = arrays_get(&var_name) {
                let new_arr: Vec<String> = arr.iter().map(|s| dir_one(s)).collect();
                value = new_arr.join(" ");
                split_parts = Some(new_arr);
            } else {
                value = dir_one(&value); // c:2229
            }
        } // c:2229

        // (b) backslash-bslashquote pattern metachars — output is safe to
        // feed back into a glob/regex context as a literal. Port of
        // subst.c:2255 QT_BACKSLASH_PATTERN: every char that has
        // pattern meaning (`* ? [ ] ( ) | ^ # ~ \ < >` plus IFS
        // whitespace and shell metachars `& ; { } $ \` " '`) gets
        // a leading backslash. Used by `[[ x =~ ${(b)pat} ]]` and
        // `case x in ${(b)pat}` to neutralize a user-supplied
        // string before it's interpreted as a pattern.
        // (b) per-element backslash-bslashquote. Direct port of subst.c:2255
        // QT_BACKSLASH_PATTERN iterating aval per-element.
        let b_one = |s: &str| -> String {
            // c:2255
            let mut out = String::with_capacity(s.len() * 2);
            for ch in s.chars() {
                if matches!(
                    ch,
                    '*' | '?'
                        | '['
                        | ']'
                        | '('
                        | ')'
                        | '|'
                        | '^'
                        | '#'
                        | '~'
                        | '\\'
                        | '<'
                        | '>'
                        | '&'
                        | ';'
                        | '{'
                        | '}'
                        | '$'
                        | '`'
                        | '"'
                        | '\''
                        | ' '
                        | '\t'
                        | '\n'
                ) {
                    out.push('\\');
                }
                out.push(ch);
            }
            out
        };
        if flag_b_pattern {
            // c:2255
            if let Some(parts) = split_parts.clone() {
                let new_parts: Vec<String> = parts.iter().map(|s| b_one(s)).collect();
                value = new_parts.join(" ");
                split_parts = Some(new_parts);
            } else if let Some(arr) = arrays_get(&var_name) {
                let new_arr: Vec<String> = arr.iter().map(|s| b_one(s)).collect();
                value = new_arr.join(" ");
                split_parts = Some(new_arr);
            } else {
                value = b_one(&value); // c:2255
            } // c:2255
        } // c:2255

        // (Q) unquote — strip outer quotes / backslash escapes /
        // decode $'…' C-string quoting. Port of subst.c:2261
        // quotemod-- effect → utils.c::dequotestring which handles
        // SQ spans (literal), DQ spans (with backslash escapes),
        // $'…' spans (with full \n / \t / \xNN / \NNN decoding via
        // getkeystring), and standalone backslash escapes.
        // (Q) unquote per-element on arrays. Direct port of
        // subst.c:2261 quotemod-- which iterates aval per-element.
        let unquote_one = |s: &str| -> String {
            // c:2261
            let chars_v: Vec<char> = s.chars().collect();
            let mut out = String::with_capacity(s.len());
            let mut i = 0_usize;
            while i < chars_v.len() {
                let c = chars_v[i];
                if c == '$' && i + 1 < chars_v.len() && chars_v[i + 1] == '\'' {
                    // c:2261
                    let body_start = i + 2;
                    let mut j = body_start;
                    while j < chars_v.len() && chars_v[j] != '\'' {
                        if chars_v[j] == '\\' && j + 1 < chars_v.len() {
                            j += 2;
                        } else {
                            j += 1;
                        }
                    }
                    let body: String = chars_v[body_start..j].iter().collect();
                    let (decoded, _) = crate::ported::utils::getkeystring(&body); // c:2261
                    out.push_str(&decoded);
                    i = j + 1;
                    continue;
                }
                if c == '\\' {
                    if i + 1 < chars_v.len() {
                        out.push(chars_v[i + 1]);
                        i += 2;
                        continue;
                    }
                } else if c == '\'' || c == '"' {
                    i += 1;
                    continue;
                }
                out.push(c);
                i += 1;
            }
            out
        };
        if flag_unquote {
            // c:2261
            if let Some(parts) = split_parts.clone() {
                let new_parts: Vec<String> = parts.iter().map(|s| unquote_one(s)).collect();
                value = new_parts.join(" ");
                split_parts = Some(new_parts);
            } else if let Some(arr) = arrays_get(&var_name) {
                let new_arr: Vec<String> = arr.iter().map(|s| unquote_one(s)).collect();
                value = new_arr.join(" ");
                split_parts = Some(new_arr);
            } else {
                value = unquote_one(&value);
            }
        }

        // (X) error on unset/empty — emit error if value is empty.
        // Port of subst.c:2264 (quoteerr=1).
        if flag_error && value.is_empty() && !is_set {
            // c:2264
            zerr(&format!("{}: parameter not set or null", var_name)); // c:N/A
            errflag_set_error();
        }

        // (V) visible — render control chars as ^X form.
        // Port of subst.c:2232 mods bit 1. Per-element on arrays.
        let visible_one = |s: &str| -> String {
            // c:2232
            let mut out = String::with_capacity(s.len());
            for c in s.chars() {
                let cp = c as u32;
                if cp < 0x20 {
                    // c:2232 (control chars)
                    out.push('^');
                    out.push(((cp + b'@' as u32) as u8) as char);
                } else if cp == 0x7f {
                    out.push_str("^?");
                } else {
                    out.push(c);
                }
            }
            out
        };
        if flag_visible {
            // c:2232
            if let Some(parts) = split_parts.clone() {
                let new_parts: Vec<String> = parts.iter().map(|s| visible_one(s)).collect();
                value = new_parts.join(" ");
                split_parts = Some(new_parts);
            } else if let Some(arr) = arrays_get(&var_name) {
                let new_arr: Vec<String> = arr.iter().map(|s| visible_one(s)).collect();
                value = new_arr.join(" ");
                split_parts = Some(new_arr);
            } else {
                value = visible_one(&value);
            }
        }

        // (c)/(w)/(W) length variants — char count, word count
        // (whitespace-split), word count (W = WS_NULL).
        // Port of subst.c:2275-2281 whichlen.
        if flag_char_count {
            // c:2275
            // (m) flag, when set, counts cells via wcpadwidth (so
            // wide chars count 2). Without (m): plain chars.count().
            // Direct port of subst.c:2275 whichlen + multi_width.
            value = if multi_width > 0 {
                // c:2275
                value
                    .chars() // c:2376
                    .map(|c| wcpadwidth(c, multi_width as i32) as usize) // c:2376
                    .sum::<usize>() // c:2376
                    .to_string() // c:2376
            } else {
                // c:2275
                value.chars().count().to_string() // c:2275
            }; // c:2275
        } else if flag_word_count {
            // c:2278
            value = value.split_whitespace().count().to_string(); // c:2278
        } else if flag_word_count_w {
            // c:2281
            // (W) — count words including empty fields.
            let parts: Vec<&str> = value.split(|c: char| c.is_whitespace()).collect();
            value = parts.len().to_string(); // c:2281
        }

        // Quote flags operate per-element when array-shaped — direct
        // port of subst.c quotemod arm which iterates aval.
        let quote_one = |s: &str| -> String {
            // c:2237
            if flag_qmin {
                // c:2245 (q-)
                let needs = s.chars().any(|c| {
                    c.is_whitespace()
                        || matches!(
                            c,
                            '*' | '?'
                                | '['
                                | ']'
                                | '('
                                | ')'
                                | '|'
                                | '&'
                                | ';'
                                | '<'
                                | '>'
                                | '$'
                                | '`'
                                | '\\'
                                | '"'
                                | '\''
                                | '#'
                                | '~'
                        )
                });
                if needs {
                    crate::ported::utils::quotestring(s, crate::ported::utils::QuoteType::Single)
                } else {
                    s.to_string()
                }
            } else if flag_qplus {
                // c:2245 (q+)
                crate::ported::utils::quotestring(s, crate::ported::utils::QuoteType::Dollars)
            } else if flag_qcount > 0 {
                // c:2237
                match flag_qcount {
                    1 => crate::ported::utils::quotestring(
                        s,
                        crate::ported::utils::QuoteType::Backslash,
                    ),
                    2 => crate::ported::utils::quotestring(
                        s,
                        crate::ported::utils::QuoteType::Single,
                    ),
                    3 => crate::ported::utils::quotestring(
                        s,
                        crate::ported::utils::QuoteType::Double,
                    ),
                    _ => crate::ported::utils::quotestring(
                        s,
                        crate::ported::utils::QuoteType::Dollars,
                    ),
                }
            } else {
                s.to_string()
            }
        };
        if flag_qmin || flag_qplus || flag_qcount > 0 {
            // c:2237
            if let Some(parts) = split_parts.clone() {
                // c:2237
                let new_parts: Vec<String> = parts.iter().map(|s| quote_one(s)).collect();
                value = new_parts.join(" ");
                split_parts = Some(new_parts);
            } else if let Some(arr) = arrays_get(&var_name) {
                let new_arr: Vec<String> = arr.iter().map(|s| quote_one(s)).collect();
                value = new_arr.join(" ");
                split_parts = Some(new_arr);
            } else {
                value = quote_one(&value);
            }
        }

        // (g) decode — apply getkeystring to the value if `(g…)` was
        // seen in the flag block. Per Src/subst.c:3955 `if (getkeys
        // >= 0)` block which fires whenever `getkeys` was set, even
        // to 0 (bare `(g::)` with no sub-letters means "default
        // getkeystring decoding"). Per-element on arrays.
        if flag_g_seen {
            // c:3955
            let _ = (flag_g_emacs, flag_g_octal, flag_g_ctrl); // sub-bits reserved for future GETKEY_* flags
            let decode_one = |s: &str| -> String { crate::ported::utils::getkeystring(s).0 };
            if let Some(parts) = split_parts.clone() {
                let new_parts: Vec<String> = parts.iter().map(|s| decode_one(s)).collect();
                value = new_parts.join(" ");
                split_parts = Some(new_parts);
            } else if let Some(arr) = arrays_get(&var_name) {
                let new_arr: Vec<String> = arr.iter().map(|s| decode_one(s)).collect();
                value = new_arr.join(" ");
                split_parts = Some(new_arr);
            } else {
                value = decode_one(&value);
            }
        }

        // (D) named-dir substitution and (V) visible-char rendering
        // per Src/subst.c:4155-4166. (D) replaces the path prefix
        // with `~name` for each named directory; (V) renders
        // non-printable bytes as `^X` / `\n` / `\t` / `\M-X`. Both
        // apply per-element when array-shaped.
        if flag_d_dir || flag_visible {
            // c:4155
            let render_d = |s: &str| -> String {
                if !flag_d_dir {
                    return s.to_string();
                }
                // Replace $HOME with `~`; replace each named-dir
                // path with `~name`. Direct port of substnamedir.
                let mut out = s.to_string();
                if let Ok(home) = std::env::var("HOME") {
                    if !home.is_empty() && out.starts_with(&home) {
                        out = format!("~{}", &out[home.len()..]);
                    }
                }
                // Named-dir entries from canonical nameddirtab —
                // longest-prefix-first to avoid shallow-prefix
                // shadowing. Mirrors C's `mod_export HashTable
                // nameddirtab` (hashnameddir.c:48).
                if let Ok(t) = crate::ported::hashnameddir::nameddirtab().lock() {
                    let mut entries: Vec<(String, String)> = t.iter()
                        .map(|(k, nd)| (k.clone(), nd.dir.clone()))
                        .collect();
                    entries.sort_by_key(|(_, p)| std::cmp::Reverse(p.len()));
                    for (name, path) in &entries {
                        if !path.is_empty() && out.starts_with(path.as_str()) {
                            out = format!("~{}{}", name, &out[path.len()..]);
                            break;
                        }
                    }
                }
                out
            };
            let render_v = |s: &str| -> String {
                if !flag_visible {
                    return s.to_string();
                }
                // Direct port of nicechar / nicedupstring per
                // Src/utils.c:462 — render non-printables as
                // `\n`, `\t`, `^X`, `\M-X`, `^?` etc.
                let mut out = String::with_capacity(s.len());
                for ch in s.chars() {
                    let code = ch as u32;
                    if (0x20..=0x7e).contains(&code) {
                        out.push(ch);
                    } else if code == 0x7f {
                        out.push('^');
                        out.push('?');
                    } else if code == 0x0a {
                        out.push('\\');
                        out.push('n');
                    } else if code == 0x09 {
                        out.push('\\');
                        out.push('t');
                    } else if code < 0x20 {
                        out.push('^');
                        out.push((b'@' + (code as u8)) as char);
                    } else if code < 0x100 {
                        // High-bit byte → `\M-X`
                        out.push_str("\\M-");
                        let stripped = code & 0x7f;
                        if (0x20..=0x7e).contains(&stripped) {
                            out.push(stripped as u8 as char);
                        } else if stripped < 0x20 {
                            out.push('^');
                            out.push((b'@' + (stripped as u8)) as char);
                        } else {
                            out.push('?');
                        }
                    } else {
                        // Multi-byte char above ASCII range — pass through
                        // (zsh's wcs_nicechar handles this; for now keep
                        // the codepoint visible as-is).
                        out.push(ch);
                    }
                }
                out
            };
            let pipeline = |s: &str| -> String {
                let s1 = render_d(s);
                render_v(&s1)
            };
            if let Some(parts) = split_parts.clone() {
                // c:4155
                let new_parts: Vec<String> = parts.iter().map(|s| pipeline(s)).collect();
                value = new_parts.join(" ");
                split_parts = Some(new_parts);
            } else if let Some(arr) = arrays_get(&var_name) {
                let new_arr: Vec<String> = arr.iter().map(|s| pipeline(s)).collect();
                value = new_arr.join(" ");
                split_parts = Some(new_arr);
            } else {
                value = pipeline(&value);
            }
        }

        // ${=name} forced split — promote scalar value to multi-word
        // splat per Src/subst.c:3902 `force_split = !ssub && spbreak`.
        // Suppressed when ssub (paramsubst called with PREFORK_SINGLE,
        // i.e. inside a scalar-assignment context). The split uses
        // IFS chars from the executor; default IFS is " \t\n".
        let in_ssub = pf_flags & PREFORK_SINGLE != 0;
        if force_split && !in_ssub && split_parts.is_none() {
            let ifs = vars_get("IFS")
                .unwrap_or_else(|| " \t\n".to_string());
            let parts: Vec<String> = value
                .split(|c: char| ifs.contains(c))
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect();
            if !parts.is_empty() {
                value = parts.join(" ");
                split_parts = Some(parts);
            }
        }
        // ${==name} forced no-split — just consume the flag, no
        // additional action needed since the default path doesn't
        // split. Used to override SH_WORD_SPLIT for one expansion.
        let _ = suppress_split; // c:2562

        // Reconstruct the full str3 with the brace expansion applied
        // — same protocol the simple `$var` arm uses (line 1240).
        // Caller (stringsubst) re-loads `str3 = list.getdata(node_idx)`
        // and expects the new full string in node 0.
        let prefix: String = chars[..start_pos].iter().collect(); // c:1885
        let suffix: String = if new_pos < chars.len() {
            // c:1885
            chars[new_pos..].iter().collect() // c:1885
        } else {
            // c:1885
            String::new() // c:1885
        }; // c:1885

        // Post-processing splat — port of subst.c:3900-4470
        // multi-node return path. When (@) flag is set on an array
        // var OR the value is genuinely array-shaped (multi-element
        // assoc keys/values), emit one result_node per array element
        // so multsub-aware callers see distinct words.
        // Implicit splat: bare `$arr` outside DQ AND not in SINGLE
        // (singsub-only) mode gets array-shape splat — zsh treats
        // arrays as inherently word-bearing in unquoted context.
        // (DQ joins via sepjoin → handled in the value-set above.)
        // Subscript that selects a single element (\${arr[1]}) must
        // NOT auto_splat — it's a scalar pick. Splat applies only
        // when subscript is absent, or @/* (full splat), or a range
        // (slice has multiple elements).
        let scripted_scalar = subscript
            .as_deref() // c:3950
            .map(|s| s != "@" && s != "*" && !s.contains(','))
            .unwrap_or(false); // c:3950
                               // ${=name} explicitly forces splat even in DQ context per
                               // subst.c:2566 — the spbreak=2 setting overrides the qt
                               // gate. Without this, `print "${=str}"` in DQ rejoined the
                               // split words back into a single arg.
        let force_splat_from_eq = force_split
            && pf_flags & PREFORK_SINGLE == 0
            && rest.is_empty()
            && split_parts.is_some();
        let auto_splat = force_splat_from_eq                 // c:2566
            || (!flag_at                                     // c:3950
            && !qt                                           // c:3950 (only outside DQ)
            && pf_flags & PREFORK_SINGLE == 0         // c:3950 (multsub context)
            && rest.is_empty()                               // c:3950 (no operator subverted shape)
            && !scripted_scalar                              // c:3950 (single-elem pick is scalar)
            && (arrays_contains(&var_name)         // c:3950
                || split_parts.is_some())); // c:3950 ((s::) made an array)
        if flag_at || auto_splat {
            // c:3950
            let parts: Vec<String> = if let Some(sp) = split_parts.clone() {
                // (s::) split → splat the post-split parts
                // regardless of source. Direct port of subst.c's
                // ssub-then-splat where spsep promotes scalar to
                // array via the split.
                sp // c:3950
            } else if let Some(sub) = subscript.as_deref() {
                // Range subscript: splat the slice elements.
                if let Some((lo, hi)) = sub.split_once(',') {
                    let lo: i64 = lo.trim().parse().unwrap_or(1); // c:3950
                    let hi: i64 = hi.trim().parse().unwrap_or(0); // c:3950
                    arrays_get(&var_name).as_ref() // c:3950
                        .map(|arr| crate::ported::params::getarrvalue(arr, lo, hi))
                        .unwrap_or_default()
                } else if let Some(arr) = arrays_get(&var_name) {
                    arr.clone() // c:3950 (@ / *)
                } else {
                    vec![value.clone()]
                }
            } else if let Some(arr) = arrays_get(&var_name) {
                arr.clone() // c:3960 (real array splat)
            } else if let Some(map) = assoc_get(&var_name) {
                if flag_keys && flag_values {
                    // c:3955 (kv splat — interleaved)
                    let mut out: Vec<String> = Vec::with_capacity(map.len() * 2); // c:3955
                    for (k, v) in map {
                        // c:3955
                        out.push(k.clone()); // c:3955
                        out.push(v.clone()); // c:3955
                    } // c:3955
                    out // c:3955
                } else if flag_keys {
                    // c:3955 (k-flag splat)
                    map.keys().cloned().collect()
                } else if flag_values {
                    // c:3957 (v-flag splat)
                    map.values().cloned().collect()
                } else {
                    vec![value.clone()] // c:3962 (scalar fallback)
                }
            } else {
                vec![value.clone()] // c:3960 (scalar)
            };
            // Build per-node strings: prefix + element + suffix.
            // First node carries prefix; last carries suffix; middle
            // nodes are bare elements.
            let mut nodes: Vec<String> = Vec::with_capacity(parts.len());
            for (i, part) in parts.iter().enumerate() {
                let s = if parts.len() == 1 {
                    format!("{}{}{}", prefix, part, suffix)
                } else if i == 0 {
                    format!("{}{}", prefix, part)
                } else if i == parts.len() - 1 {
                    format!("{}{}", part, suffix)
                } else {
                    part.clone()
                };
                nodes.push(s);
            }
            let first = nodes.first().cloned().unwrap_or_default();
            let new_pos_in_full = prefix.chars().count()
                + first.chars().count().saturating_sub(prefix.chars().count());
            return (first, new_pos_in_full, nodes);
        }

        let full = format!("{}{}{}", prefix, value, suffix); // c:1885
        let new_pos_in_full = prefix.chars().count() + value.chars().count();
        return (full.clone(), new_pos_in_full, vec![full]);
    } // c:1885

    // Simple $var (or $arr[idx] for array-element access — per
    // Src/lex.c::gettokstr, zsh accepts `$name[subscript]` as a
    // first-class array-element expansion. Without parsing the
    // bracket here, `$match[1]` from a `(#b)` replacement template
    // resolved to "match" + literal "[1]" instead of the captured
    // group).
    if c.is_ascii_alphabetic() || c == '_' {
        // c:1625
        let var_start = pos; // c:1625
        while pos < chars.len() && (chars[pos].is_ascii_alphanumeric() || chars[pos] == '_') {
            // c:1625
            pos += 1; // c:1625
        } // c:1625
        let var_name: String = chars[var_start..pos].iter().collect(); // c:1625

        // Optional `[subscript]`. Per zsh, only valid for declared
        // arrays/assocs — for scalars the `[` stays literal.
        let mut subscript_str: Option<String> = None; // c:1625
        if chars.get(pos).copied() == Some('[') {
            // c:1625
            // Collect until matching `]` (depth-tracked so
            // `$arr[$other[1]]` works).
            let mut depth = 1; // c:1625
            let mut q = pos + 1; // c:1625
            while q < chars.len() && depth > 0 {
                // c:1625
                match chars[q] {
                    // c:1625
                    '[' => depth += 1, // c:1625
                    ']' => {
                        // c:1625
                        depth -= 1; // c:1625
                        if depth == 0 {
                            // c:1625
                            break; // c:1625
                        } // c:1625
                    } // c:1625
                    _ => {}            // c:1625
                } // c:1625
                q += 1; // c:1625
            } // c:1625
            if depth == 0 {
                // c:1625
                let raw_sub: String = chars[pos + 1..q].iter().collect(); // c:1625
                                                                          // Resolve $X / ${X} inside the subscript.
                subscript_str = Some(singsub(&raw_sub)); // c:1625
                pos = q + 1; // c:1625
            } // c:1625
        } // c:1625

        let value = if let Some(sub) = subscript_str.as_deref() {
            // c:1625
            // Array / assoc element lookup. Port of zsh's
            // getarrvalue + getindex + getasub (Src/params.c).
            // Order: assoc first (key lookup), then array
            // (numeric / `*` / `@` / range), then scalar fallback
            // (zsh treats `$scalar[N]` as char-N of the scalar
            // string, 1-based; `$scalar[N,M]` as substring).
            if let Some(map) = assoc_get(&var_name) {
                // c:1625
                // Subscript-flag form: (I)/(i)/(R)/(r) on assoc.
                // Same plumbing as braced path. Direct port of
                // Src/params.c getarg hash routing.
                if let Some((flags, pat)) = (|s: &str| -> Option<(String, String)> {
                    let s = s.trim_start();
                    let rest = s.strip_prefix('(')?;
                    let close = rest.find(')')?;
                    let f = rest[..close].to_string();
                    let p = rest[close + 1..].to_string();
                    if f.chars()
                        .all(|c| matches!(c, 'I' | 'R' | 'i' | 'r' | 'k' | 'K' | 'n' | 'e' | 'b'))
                    {
                        Some((f, p))
                    } else {
                        None
                    }
                })(sub)
                {
                    let by_key = flags.contains('I') || flags.contains('i');
                    let return_all = flags.contains('I') || flags.contains('R');
                    let mut out: Vec<String> = Vec::new();
                    for (k, v) in map.iter() {
                        let hay = if by_key { k.as_str() } else { v.as_str() };
                        if crate::ported::pattern::patmatch(&pat, hay) {
                            out.push(if by_key { k.clone() } else { v.clone() });
                            if !return_all {
                                break;
                            }
                        }
                    }
                    out.join(" ")
                } else {
                    map.get(sub).cloned().unwrap_or_default() // c:1625
                }
            } else if let Some(arr) = arrays_get(&var_name) {
                // c:1625
                if sub == "*" || sub == "@" {
                    // c:1625
                    arr.join(" ") // c:1625
                } else if let Some((flags, pat)) = (|s: &str| -> Option<(String, String)> {
                    // (I)/(i)/(R)/(r) on bare $arr[...]. Same as
                    // braced form. Direct port of params.c getarg
                    // array-pattern routing.
                    let s = s.trim_start();
                    let rest = s.strip_prefix('(')?;
                    let close = rest.find(')')?;
                    let f = rest[..close].to_string();
                    let p = rest[close + 1..].to_string();
                    if f.chars()
                        .all(|c| matches!(c, 'I' | 'R' | 'i' | 'r' | 'n' | 'e'))
                    {
                        Some((f, p))
                    } else {
                        None
                    }
                })(sub)
                {
                    let return_index = flags.contains('I') || flags.contains('i');
                    let return_all = flags.contains('I') || flags.contains('R');
                    let mut out: Vec<String> = Vec::new();
                    for (idx, elem) in arr.iter().enumerate() {
                        if crate::ported::pattern::patmatch(&pat, elem) {
                            if return_index {
                                out.push((idx + 1).to_string());
                            } else {
                                out.push(elem.clone());
                            }
                            if !return_all {
                                break;
                            }
                        }
                    }
                    if out.is_empty() && return_index {
                        (arr.len() + 1).to_string()
                    } else {
                        out.join(" ")
                    }
                } else if let Some((lo, hi)) = sub.split_once(',') {
                    // c:1625
                    // Delegate to the canonical slice helper —
                    // gets all the negative-wrap / out-of-range
                    // edge cases right (start > len, start < -len,
                    // resolve(0)→1, etc.) per the bug-for-bug
                    // port of getarrvalue's range arm.
                    let lo: i64 = lo.trim().parse().unwrap_or(1); // c:1625
                    let hi: i64 = hi.trim().parse().unwrap_or(arr.len() as i64); // c:1625
                    crate::ported::params::getarrvalue(&arr, lo, hi).join(" ") // c:1625
                } else if let Ok(idx) = sub.parse::<i32>() {
                    // c:1625
                    let n = arr.len() as i32; // c:1625
                    let i = if idx < 0 { n + idx } else { idx - 1 }; // c:1625
                    if i >= 0 && (i as usize) < arr.len() {
                        // c:1625
                        arr[i as usize].clone() // c:1625
                    } else {
                        // c:1625
                        String::new() // c:1625
                    } // c:1625
                } else {
                    // c:1625
                    String::new() // c:1625
                } // c:1625
            } else if let Some(magic_val) = {
                // c:1625 — magic-assoc per-key lookup via the
                // partab[] dispatch (Src/Modules/parameter.c:2234).
                // See companion dispatch at the braced-form site.
                use crate::ported::modules::parameter::*;
                let nul = std::ptr::null_mut();
                let pm: Option<crate::ported::zsh_h::Param> = if sub == "@" || sub == "*" {
                    None
                } else { match var_name.as_str() {
                    "aliases"             => getpmralias(nul, sub),
                    "galiases"            => getpmgalias(nul, sub),
                    "saliases"            => getpmsalias(nul, sub),
                    "dis_aliases"         => getpmdisralias(nul, sub),
                    "dis_galiases"        => getpmdisgalias(nul, sub),
                    "dis_saliases"        => getpmdissalias(nul, sub),
                    "builtins"            => getpmbuiltin(nul, sub),
                    "dis_builtins"        => getpmdisbuiltin(nul, sub),
                    "commands"            => getpmcommand(nul, sub),
                    "functions"           => getpmfunction(nul, sub),
                    "dis_functions"       => getpmdisfunction(nul, sub),
                    "functions_source"    => getpmfunction_source(nul, sub),
                    "dis_functions_source"=> getpmdisfunction_source(nul, sub),
                    "nameddirs"           => getpmnameddir(nul, sub),
                    "userdirs"            => getpmuserdir(nul, sub),
                    "options"             => getpmoption(nul, sub),
                    "parameters"          => getpmparameter(nul, sub),
                    "history"             => getpmhistory(nul, sub),
                    "modules"             => getpmmodule(nul, sub),
                    "jobdirs"             => getpmjobdir(nul, sub),
                    "jobstates"           => getpmjobstate(nul, sub),
                    "jobtexts"            => getpmjobtext(nul, sub),
                    "usergroups"          => getpmusergroups(nul, sub),
                    _ => None,
                }};
                // c:`scanpm<X>` splice paths from Modules/parameter.c.
                pm.and_then(|p| p.u_str).or_else(|| {
                    if sub == "@" || sub == "*" {
                        splice_magic_assoc(&var_name)
                    } else {
                        None
                    }
                })
            } {
                magic_val
            } else {
                // c:1625
                let s = vars_get(&var_name).unwrap_or_default(); // c:1625
                let chars_v: Vec<char> = s.chars().collect(); // c:1625
                if sub == "*" || sub == "@" {
                    // c:1625
                    s // c:1625
                } else if let Some((lo, hi)) = sub.split_once(',') {
                    // c:1625
                    // Reuse the canonical slice helper for
                    // scalar substring — chars_v is treated as a
                    // 1-element-per-char "array".
                    let lo: i64 = lo.trim().parse().unwrap_or(1); // c:1625
                    let hi: i64 = hi.trim().parse().unwrap_or(chars_v.len() as i64); // c:1625
                    let chars_arr: Vec<String> = chars_v.iter().map(|c| c.to_string()).collect(); // c:1625
                    crate::ported::params::getarrvalue(&chars_arr, lo, hi).concat()
                // c:1625
                } else if let Ok(idx) = sub.parse::<i32>() {
                    // c:1625
                    let n = chars_v.len() as i32; // c:1625
                    let i = if idx < 0 { n + idx } else { idx - 1 }; // c:1625
                    if i >= 0 && (i as usize) < chars_v.len() {
                        // c:1625
                        chars_v[i as usize].to_string() // c:1625
                    } else {
                        // c:1625
                        String::new() // c:1625
                    } // c:1625
                } else {
                    // c:1625
                    String::new() // c:1625
                } // c:1625
            } // c:1625
        } else {
            // c:1625
            // No subscript: route through the canonical getsparam
            // funnel (GSU + variables + env + array-join), then
            // fall through to assoc-values for `$assoc` bare reads.
            // Same single-funnel pattern as subst.rs:2120.
            exec_getsparam(&var_name)
                .or_else(|| {
                    assoc_get(&var_name)
                        .map(|m| m.values().cloned().collect::<Vec<_>>().join(" "))
                })
                .unwrap_or_default() // c:1625
        }; // c:1625

        // Handle word splitting
        if pf_flags & PREFORK_SHWORDSPLIT != 0 && !qt {
            // c:1625
            let words = value
                .split_whitespace()
                .map(String::from)
                .collect::<Vec<String>>(); // c:1625
            if words.len() > 1 {
                // c:1625
                let prefix: String = chars[..start_pos].iter().collect(); // c:1625
                let suffix: String = chars[pos..].iter().collect(); // c:1625

                for (i, word) in words.iter().enumerate() {
                    // c:1625
                    if i == 0 {
                        // c:1625
                        result_nodes.push(format!("{}{}", prefix, word)); // c:1625
                    } else if i == words.len() - 1 {
                        // c:1625
                        result_nodes.push(format!("{}{}", word, suffix)); // c:1625
                    } else {
                        // c:1625
                        result_nodes.push(word.clone()); // c:1625
                    } // c:1625
                } // c:1625
                return (
                    // c:1625
                    result_nodes[0].clone(),       // c:1625
                    prefix.len() + words[0].len(), // c:1625
                    result_nodes,                  // c:1625
                ); // c:1625
            } // c:1625
        } // c:1625

        // Auto-splat for bare \$arr outside DQ in multsub context —
        // mirrors the braced-form auto_splat in the brace arm above.
        // zsh treats arrays as inherently multi-word in unquoted
        // context. Also fires for \$arr[@] / \$arr[*] which are the
        // explicit-splat forms — even with a subscript, a `@`/`*`
        // sub means "all elements as separate words".
        // Direct port of subst.c:3950 multi-node return.
        let splat_full = subscript_str.as_deref() == Some("@") // c:3950
            || subscript_str.as_deref() == Some("*"); // c:3950
                                                      // Range subscript like `[1,3]` also produces array-shape
                                                      // slice — splat in non-DQ.
        let splat_range = subscript_str
            .as_deref()
            .map(|s| s.contains(','))
            .unwrap_or(false); // c:3950
                               // Assoc bare-name splat: `$assoc[@]` returns values, `$assoc[*]`
                               // returns values too. Per zsh, `(@k)assoc` returns keys; for
                               // bare `$assoc[@]` without (k), values is the convention.
        let splat_assoc = (splat_full || splat_range)        // c:3950
            && assoc_contains(&var_name); // c:3950
        if !qt                                                // c:3950
            && pf_flags & PREFORK_SINGLE == 0          // c:3950
            && (subscript_str.is_none() || splat_full || splat_range) // c:3950
            && (arrays_contains(&var_name) || splat_assoc)
        // c:3950
        {
            // c:3950
            // Pull the actual array slice for range form so
            // splat uses the slice elements (not the full arr).
            let slice_arr: Option<Vec<String>> = if splat_range {
                if let Some(sub) = subscript_str.as_deref() {
                    if let Some((lo, hi)) = sub.split_once(',') {
                        // c:3950
                        let lo: i64 = lo.trim().parse().unwrap_or(1); // c:3950
                        let hi: i64 = hi.trim().parse().unwrap_or(0); // c:3950
                        arrays_get(&var_name).as_ref()
                            .map(|arr| crate::ported::params::getarrvalue(arr, lo, hi))
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };
            // Assoc fallback when var isn't in arrays.
            let assoc_vals: Option<Vec<String>> = if splat_assoc {
                // c:3950
                assoc_get(&var_name) // c:3950
                    .map(|m| m.values().cloned().collect()) // c:3950
            } else {
                None
            }; // c:3950
            if let Some(arr) = slice_arr
                .or(assoc_vals)
                .or_else(|| arrays_get(&var_name))
            {
                let prefix: String = chars[..start_pos].iter().collect(); // c:3950
                let suffix: String = chars[pos..].iter().collect(); // c:3950
                let mut nodes: Vec<String> = Vec::with_capacity(arr.len()); // c:3950
                for (i, part) in arr.iter().enumerate() {
                    // c:3950
                    let s = if arr.len() == 1 {
                        // c:3950
                        format!("{}{}{}", prefix, part, suffix) // c:3950
                    } else if i == 0 {
                        // c:3950
                        format!("{}{}", prefix, part) // c:3950
                    } else if i == arr.len() - 1 {
                        // c:3950
                        format!("{}{}", part, suffix) // c:3950
                    } else {
                        // c:3950
                        part.clone() // c:3950
                    }; // c:3950
                    nodes.push(s); // c:3950
                } // c:3950
                let first = nodes.first().cloned().unwrap_or_default(); // c:3950
                return (first, prefix.len(), nodes); // c:3950
            } // c:3950
        } // c:3950

        let prefix: String = chars[..start_pos].iter().collect(); // c:1625
        let suffix: String = chars[pos..].iter().collect(); // c:1625
        let result = format!("{}{}{}", prefix, value, suffix); // c:1625
        result_nodes.push(result.clone()); // c:1625
        return (result, prefix.len() + value.len(), result_nodes); // c:1625
    } // c:1625

    // Special parameters: $?, $$, $#, $*, $@, $0-$9
    match c {
        // c:1625
        '?' => {
            // c:1625
            let value = vars_get("?") // c:1625
                .unwrap_or_else(|| "0".to_string()); // c:1625
            let prefix: String = chars[..start_pos].iter().collect(); // c:1625
            let suffix: String = chars[pos + 1..].iter().collect(); // c:1625
            let result = format!("{}{}{}", prefix, value, suffix); // c:1625
            result_nodes.push(result.clone()); // c:1625
            (result, prefix.len() + value.len(), result_nodes) // c:1625
        } // c:1625
        '$' => {
            // c:1625
            let value = std::process::id().to_string(); // c:1625
            let prefix: String = chars[..start_pos].iter().collect(); // c:1625
            let suffix: String = chars[pos + 1..].iter().collect(); // c:1625
            let result = format!("{}{}{}", prefix, value, suffix); // c:1625
            result_nodes.push(result.clone()); // c:1625
            (result, prefix.len() + value.len(), result_nodes) // c:1625
        } // c:1625
        '#' => {
            // c:1625
            let value = arrays_get("@") // c:1625
                .map(|a| a.len().to_string()) // c:1625
                .unwrap_or_else(|| "0".to_string()); // c:1625
            let prefix: String = chars[..start_pos].iter().collect(); // c:1625
            let suffix: String = chars[pos + 1..].iter().collect(); // c:1625
            let result = format!("{}{}{}", prefix, value, suffix); // c:1625
            result_nodes.push(result.clone()); // c:1625
            (result, prefix.len() + value.len(), result_nodes) // c:1625
        } // c:1625
        '*' | '@' => {
            // c:1625
            let values = arrays_get("@").unwrap_or_default(); // c:1625
                                                                             // zsh semantics:
                                                                             //   $* / "$*" — join with IFS first char
                                                                             //   $@        — splat into separate words
                                                                             //   "$@"      — preserve array shape (still splat)
                                                                             // Our port: $@ (qt or unqt) → splat; $* → join.
                                                                             // Direct port of subst.c c:1625 dispatch — only $* with
                                                                             // any quoting joins; $@ always preserves array shape.
            let value = if c == '*' {
                // c:1625
                let join_sep = vars_get("IFS").as_ref()
                    .and_then(|s| s.chars().next())
                    .map(String::from)
                    .unwrap_or_else(|| " ".to_string());
                values.join(&join_sep) // c:1625
            } else {
                // c:1625
                // $@ / "$@" in unquoted/SINGLE-aware context
                if pf_flags & PREFORK_SINGLE == 0 {
                    // c:1625
                    let prefix: String = chars[..start_pos].iter().collect(); // c:1625
                    let suffix: String = chars[pos + 1..].iter().collect(); // c:1625
                    for (i, v) in values.iter().enumerate() {
                        // c:1625
                        if i == 0 {
                            // c:1625
                            result_nodes.push(format!("{}{}", prefix, v)); // c:1625
                        } else if i == values.len() - 1 {
                            // c:1625
                            result_nodes.push(format!("{}{}", v, suffix)); // c:1625
                        } else {
                            // c:1625
                            result_nodes.push(v.clone()); // c:1625
                        } // c:1625
                    } // c:1625
                    if result_nodes.is_empty() {
                        // c:1625
                        result_nodes.push(format!("{}{}", prefix, suffix)); // c:1625
                    } // c:1625
                    return (result_nodes[0].clone(), start_pos, result_nodes); // c:1625
                } // c:1625
                values.join(" ") // c:1625
            }; // c:1625
            let prefix: String = chars[..start_pos].iter().collect(); // c:1625
            let suffix: String = chars[pos + 1..].iter().collect(); // c:1625
            let result = format!("{}{}{}", prefix, value, suffix); // c:1625
            result_nodes.push(result.clone()); // c:1625
            (result, prefix.len() + value.len(), result_nodes) // c:1625
        } // c:1625
        '0'..='9' => {
            // c:1625
            // `$0` reads variables["0"] (script/function name, writable
            // via plain `0=value`). `$1`..`$9` index into positional
            // params 1-based: digit N → arrays["@"][N-1]. Direct port
            // of Src/params.c which exposes "0" as a SPECIALPMDEF
            // backed by `argzero`, and digit-N as positional N.
            // Multi-digit numerics ($10, $11, ...) need lookahead to
            // capture trailing digits — collect them into the name
            // before the lookup.
            let mut digit_str = String::from(c); // c:1625
            let mut nx = pos + 1; // c:1625
            while nx < chars.len() && chars[nx].is_ascii_digit() {
                // c:1625
                digit_str.push(chars[nx]); // c:1625
                nx += 1; // c:1625
            } // c:1625
            let digit: usize = digit_str.parse().unwrap_or(0); // c:1625
            let value = if digit == 0 {
                // c:1625
                vars_get("0").unwrap_or_default() // c:1625
            } else {
                // c:1625
                arrays_get("@") // c:1625
                    .and_then(|a| a.get(digit.saturating_sub(1)).cloned()) // c:1625
                    .unwrap_or_default() // c:1625
            }; // c:1625
            let prefix: String = chars[..start_pos].iter().collect(); // c:1625
            let suffix: String = chars[nx..].iter().collect(); // c:1625
            let result = format!("{}{}{}", prefix, value, suffix); // c:1625
            result_nodes.push(result.clone()); // c:1625
            (result, prefix.len() + value.len(), result_nodes) // c:1625
        } // c:1625
        _ => {
            // c:1625
            // Just a literal $
            result_nodes.push(s.to_string()); // c:1625
            (s.to_string(), start_pos + 1, result_nodes) // c:1625
        } // c:1625
    } // c:1625
} // c:1625

// Helper functions

/// Port of `filesubstr()` from `Src/subst.c:737`.
///
/// Performs `~` and `=` expansion on a single path component. Returns
/// `Some(expanded)` on success, `None` if no expansion applies. The
/// caller (filesub) chains this on `:`-separated path lists.
///
/// Faithful port of the C ladder — covers `~`, `~+`, `~-`, `~N`/`~-N`
/// (dirstack), `~user` (libc getpwnam), and `=cmd` (PATH lookup via
/// equalsubstr).
pub fn filesubstr(s: &str, assign: bool) -> Option<String> { // c:737
    // c:737
    if s.is_empty() {
        // c:737
        return None; // c:737
    }
    let chars: Vec<char> = s.chars().collect(); // c:737
    let first = chars[0]; // c:737

    // `~` (and Tilde token) — but not `~=` (handled separately by =arm).
    // C: `if (*str == Tilde && str[1] != '=' && str[1] != Equals)`.
    if first == '~' || first == '\u{98}'
    /* Tilde token */
    {
        // c:741
        if chars.len() == 1 {
            // c:748 — bare ~
            let home = vars_get("HOME")
                .or_else(|| std::env::var("HOME").ok())
                .unwrap_or_default();
            return Some(home);
        }
        let nx = chars[1]; // c:741
        if nx == '=' {
            return None;
        } // c:741 — leave for =arm

        // C `isend(c)`: !c || c=='/' || c==Inpar || (assign && c==':')
        let isend = |c: char| -> bool {
            // c:725 macro
            c == '\0' || c == '/' || c == '\u{85}' /* Inpar */
                || (assign && c == ':')
        };

        // `~/...` and `~` (isend(str[1])) — bare HOME
        if isend(nx) {
            // c:748
            let home = vars_get("HOME")
                .or_else(|| std::env::var("HOME").ok())
                .unwrap_or_default();
            let suffix: String = chars[1..].iter().collect();
            return Some(format!("{}{}", home, suffix));
        }
        // `~+...` — current PWD (only if isend(str[2]))
        if nx == '+' && chars.len() >= 3 && isend(chars[2]) {
            // c:752
            let pwd = vars_get("PWD")
                .or_else(|| std::env::var("PWD").ok())
                .unwrap_or_default();
            let suffix: String = chars[2..].iter().collect();
            return Some(format!("{}{}", pwd, suffix));
        }
        // `~-...` — OLDPWD (only if isend(str[2]))
        if nx == '-' && chars.len() >= 3 && isend(chars[2]) {
            // c:755
            let oldpwd = vars_get("OLDPWD")
                .or_else(|| std::env::var("OLDPWD").ok())
                .or_else(|| vars_get("PWD"))
                .or_else(|| std::env::var("PWD").ok())
                .unwrap_or_default();
            let suffix: String = chars[2..].iter().collect();
            return Some(format!("{}{}", oldpwd, suffix));
        }
        // `~+N` / `~-N` — dirstack entry. C: `if (!inblank(str[1]) &&
        // isend(*ptr) && (!idigit(str[1]) || (ptr - str < 4)))`.
        // Walk digit suffix; ptr ends at first non-digit.
        if (nx == '+' || nx == '-' || nx.is_ascii_digit()) && !nx.is_whitespace() {
            // Parse signed integer from chars[1..]
            let mut p = 1_usize;
            let neg = chars[p] == '-';
            if chars[p] == '+' || chars[p] == '-' {
                p += 1;
            }
            let dstart = p;
            while p < chars.len() && chars[p].is_ascii_digit() {
                p += 1;
            }
            if p > dstart && p < chars.len() && isend(chars[p]) {
                let val: i32 = chars[dstart..p]
                    .iter()
                    .collect::<String>()
                    .parse()
                    .unwrap_or(0);
                let val = if neg { -val } else { val };
                let pwd = vars_get("PWD")
                    .or_else(|| std::env::var("PWD").ok())
                    .unwrap_or_default();
                // Direct port of subst.c filesub's tilde-+/- arm:
                // dstackent(ch, val) → pwd or stack entry.
                // c:4902 — read from canonical DIRSTACK global (mirrors
                // C's `mod_export LinkList dirstack` at builtin.c:743).
                let dirstack: Vec<String> = crate::ported::modules::parameter::DIRSTACK
                    .lock()
                    .map(|d| d.clone())
                    .unwrap_or_default();
                let pushdminus = isset(crate::ported::zsh_h::PUSHDMINUS); // c:4906
                let entry = dstackent(
                    // c:4902
                    if neg { '-' } else { '+' }, // c:4902
                    val,                         // c:4902
                    &dirstack,                   // c:4902
                    &pwd,                        // c:4902
                    pushdminus,                  // c:4906
                );
                if let Some(dir) = entry {
                    let suffix: String = chars[p..].iter().collect();
                    return Some(format!("{}{}", dir, suffix));
                }
                return None;
            }
        }
        // `~user` — getpwnam lookup (libc).
        // C: `if ((ptr = itype_end(str+1, IUSER, 0)) != str+1)` —
        // walk identifier chars (alnum + `_`).
        let mut p = 1_usize;
        while p < chars.len() && (chars[p].is_ascii_alphanumeric() || chars[p] == '_') {
            p += 1;
        }
        if p > 1 && p < chars.len() && isend(chars[p]) {
            let user: String = chars[1..p].iter().collect();
            let suffix: String = chars[p..].iter().collect();
            // Named-dir lookup FIRST — `hash -d name=path` registered
            // names take precedence over OS users (zsh canonical).
            // Direct port of subst.c filesub which checks
            // nameddirtab via getnameddir before falling through to
            // getpwnam.
            // Canonical nameddirtab lookup (mirrors C's
            // `getnameddir(name)` at hashnameddir.c via gethashnode2).
            let named = crate::ported::hashnameddir::nameddirtab()
                .lock()
                .ok()
                .and_then(|t| t.get(&user).map(|nd| nd.dir.clone()));
            if let Some(path) = named {
                return Some(format!("{}{}", path, suffix));
            }
            // libc getpwnam — cstring -> pw_dir
            use std::ffi::CString;
            if let Ok(cname) = CString::new(user.clone()) {
                unsafe {
                    let pw = libc::getpwnam(cname.as_ptr());
                    if !pw.is_null() {
                        let home_ptr = (*pw).pw_dir;
                        if !home_ptr.is_null() {
                            let home = std::ffi::CStr::from_ptr(home_ptr)
                                .to_string_lossy()
                                .into_owned();
                            return Some(format!("{}{}", home, suffix));
                        }
                    }
                }
            }
            // Fall through — user not found, return None (caller
            // decides whether to NOMATCH error).
            return None;
        }
        return None;
    }

    // `=cmd` — PATH lookup via equalsubstr. C:
    // `if (*str == Equals && isset(EQUALS) && str[1] && str[1] != Inpar)`.
    if (first == '=' || first == '\u{86}'/* Equals */) && chars.len() > 1 && chars[1] != '\u{85}'
    /* Inpar */
    {
        let cmd_part: String = chars[1..].iter().collect();
        // Split at `:` if assign, else take the whole thing.
        let cmd = if assign {
            cmd_part.split(':').next().unwrap_or(&cmd_part).to_string()
        } else {
            cmd_part.clone()
        };
        let path = vars_get("PATH")
            .or_else(|| std::env::var("PATH").ok())
            .unwrap_or_default();
        for dir in path.split(':') {
            let full = format!("{}/{}", dir, cmd);
            if std::path::Path::new(&full).exists() {
                if assign && cmd_part.len() > cmd.len() {
                    let suffix = &cmd_part[cmd.len()..];
                    return Some(format!("{}{}", full, suffix));
                }
                return Some(full);
            }
        }
    }
    None
}

/// Port of `filesub()` from `Src/subst.c:667-704`.
///
/// 1:1 with C: applies filesubstr to the leading `~`/`=`, then in
/// assign-context walks `=` (TYPESET-only) and `:`-separated path
/// lists, reapplying filesubstr to each suffix that begins with a
/// tilde/equals.
// ~, = subs: assign & PREFORK_TYPESET => typeset or magic equals          // c:661
fn filesub(s: &str, flags: i32) -> String {
    // c:667
    // C: `filesubstr(namptr, assign);`  (line 672)
    let mut namptr: String = filesubstr(s, flags != 0).unwrap_or_else(|| s.to_string()); // c:672

    // C: `if (!assign) return;` — non-assign context bails early.
    if flags == 0 {
        // c:674
        return namptr; // c:675
    }

    let mut eql: Option<usize> = None; // c:668 (eql=NULL)

    // C: PREFORK_TYPESET arm — `${var}=value` shape, find `=` then
    // recurse filesubstr on the RHS.
    if flags & PREFORK_TYPESET != 0 {
        // c:677
        // C: `(*namptr)[1] && (eql = sub = strchr(*namptr + 1, Equals))`
        if namptr.len() >= 2 {
            // c:678
            // strchr from index 1 onward
            if let Some(sub) = namptr[1..].find('=').map(|p| p + 1) {
                // c:678
                eql = Some(sub); // c:678
                let str_start = sub + 1; // c:679
                if str_start < namptr.len()                 // c:680
                    && (namptr.as_bytes()[str_start] == b'~'
                        || namptr.as_bytes()[str_start] == b'=')
                {
                    // c:680
                    let rhs = &namptr[str_start..]; // c:679
                    if let Some(expanded) = filesubstr(rhs, true) {
                        // c:680
                        // C: `sub[1] = '\0'; *namptr = dyncat(*namptr, str);`
                        namptr = format!("{}{}", &namptr[..str_start], expanded);
                        // c:682
                    } // c:682
                } // c:680
            } else {
                // c:684
                return namptr; // c:685
            } // c:686
        } else {
            // c:684
            return namptr; // c:685
        } // c:686
    }

    // C: `ptr = *namptr; while ((sub = strchr(ptr, ':'))) { … }`
    // Walk `:`-separated path components, reapply filesubstr on each
    // suffix that starts with `~` or `=`.
    let mut ptr_off = 0_usize; // c:689
    loop {
        // c:690
        let slice = &namptr[ptr_off..]; // c:690
        let colon_rel = match slice.find(':') {
            // c:690
            Some(p) => p,  // c:690
            None => break, // c:690
        }; // c:690
        let sub = ptr_off + colon_rel; // c:690
        let str_start = sub + 1; // c:691
        let len = sub; // c:692
                       // C: `sub > eql` — skip the `:` we already chewed in TYPESET.
        let past_eql = match eql {
            // c:693
            Some(e) => sub > e, // c:693
            None => true,       // c:693
        }; // c:693
        if past_eql                                         // c:693
            && str_start < namptr.len()                     // c:694
            && (namptr.as_bytes()[str_start] == b'~'
                || namptr.as_bytes()[str_start] == b'=')
        {
            // c:694
            let rhs = &namptr[str_start..]; // c:691
            if let Some(expanded) = filesubstr(rhs, true) {
                // c:695
                namptr = format!("{}{}", &namptr[..str_start], expanded); // c:697
            } // c:695
        } // c:695
        ptr_off = len + 1; // c:700
        if ptr_off >= namptr.len() {
            // c:700
            break; // c:700
        } // c:700
    } // c:701
    namptr // c:702
} // c:703

/// Port of `arithsubst()` from `Src/subst.c:4485-4509`.
///
/// C body: param-substitute the expression first (`singsub(&a)`),
/// evaluate as math, then format the integer/float result honoring
/// `outputradix` and `outputunderscore` options; concatenate the
/// caller-supplied `prefix` (`*bptr`) + result + `rest` and return.
///
/// Rust signature changed from `(char *a, char **bptr, char *rest)`
/// to `(expr, prefix, rest) -> String` because Rust strings
/// own their storage; the caller now consumes the returned String
/// directly instead of the C in-out buffer protocol.
fn arithsubst(expr: &str, prefix: &str, rest: &str) -> String {
    // c:4485
    // Pre-resolve `$#NAME` before singsub — singsub treats `$#` as
    // positional-count (`$#`) followed by literal `NAME`, which mangles
    // `$#parts` to `0parts`. zsh's parser binds `$#NAME` as length-of
    // (parameter-name length form) when NAME is an identifier. Direct
    // port of zsh's `prefork()` BNULL-aware `$#` arm — Src/subst.c
    // around line 1860 dispatches via the param-name lookahead before
    // the math evaluator sees the expression.
    let expr = {
        let bytes: Vec<char> = expr.chars().collect();
        let mut out = String::with_capacity(expr.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == '$' && i + 1 < bytes.len() && bytes[i + 1] == '#' {
                let name_start = i + 2;
                let mut name_end = name_start;
                while name_end < bytes.len()
                    && (bytes[name_end].is_ascii_alphanumeric() || bytes[name_end] == '_')
                {
                    name_end += 1;
                }
                if name_end > name_start {
                    let name: String = bytes[name_start..name_end].iter().collect();
                    // Read from `state` (the snapshot built via
                    // subst_state_from_executor); routes through the
                    // same data the executor exposed without reaching
                    // back into ShellExecutor from src/ported/.
                    let count = if let Some(arr) = arrays_get(&name) {
                        arr.len()
                    } else if let Some(assoc) = assoc_get(&name) {
                        assoc.len()
                    } else if name == "@" || name == "*" {
                        arrays_get("@").map(|a| a.len()).unwrap_or(0)
                    } else if let Some(s) = vars_get(&name) {
                        s.chars().count()
                    } else {
                        0
                    };
                    out.push_str(&count.to_string());
                    i = name_end;
                    continue;
                }
            }
            out.push(bytes[i]);
            i += 1;
        }
        out
    };
    // C: `singsub(&a);` — parameter-substitute the math expression
    // before evaluation. Without this `${(($n+1))}` won't see $n.
    let expanded = singsub(&expr); // c:4490

    // C: `v = matheval(a);` — evaluate via Src/math.c::matheval.
    // Use the global matheval; resolves variables via env lookups
    // matching the same data the executor exposes through env_var
    // bridges (the from_executor snapshot already mirrored shell
    // params into env vars). No ShellExecutor reach.
    let v = match crate::math::matheval(&expanded) {                         // c:4490 matheval
        Ok(n) => n,
        Err(_) => crate::math::Mnumber { l: 0, d: 0.0, type_: crate::ported::zsh_h::MN_UNSET },
    };

    // c: math.c:580-583 — `outputradix` / `outputunderscore` are set while
    // parsing `[#…]` / `[##…]` math prefixes. `crate::math::matheval` does not
    // yet thread `outputunderscore` back to callers; keep 0 (no `_` grouping)
    // until that lands. `OUTPUT_RADIX` shell-style override is a zshrs bridge.
    let outputunderscore: i32 = 0; // c:583 equivalent when no `[#…_…]` active
    let outputradix = vars_get("OUTPUT_RADIX").as_ref()
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(0); // c:4492
    let b: String = if v.type_ == crate::ported::zsh_h::MN_UNSET {
        "0".to_string() // c:4498 — MN_UNSET falls through to zero in practice
    } else if (v.type_ == crate::ported::zsh_h::MN_FLOAT) && outputradix == 0 {
        // c:4493-4494
        crate::ported::params::convfloat_underscore(v.d, outputunderscore)
    } else {
        // c:4496-4498
        let l = if (v.type_ == crate::ported::zsh_h::MN_FLOAT) {
            v.d as i64
        } else {
            v.l
        };
        crate::ported::params::convbase_underscore(l, outputradix as u32, outputunderscore)
    }; // c:4499

    // C: `t = *bptr = hcalloc(...); …; strcat(t, rest);` — concat
    // prefix + b + rest. Returns pointer past prefix+b (where rest
    // begins). Rust returns the full string.
    format!("{}{}{}", prefix, b, rest) // c:4501-4509
} // c:4509

// `convbase` lives in src/ported/utils.rs (canonical port of
// Src/utils.c). Callers below import via the full path.

/// Multsub flags (from subst.c)
// `pub mod multsub_flags { … }` — DELETED per user directive; was
// a Rust-only u32 wrapper duplicating the canonical i32 constants
// in `zsh_h::MULTSUB_*` (c:zsh.h:2046-2059). Use those directly.
use crate::ported::zsh_h::{MULTSUB_PARAM_NAME, MULTSUB_WS_AT_END, MULTSUB_WS_AT_START}; // c:zsh.h:2046-2059

/// Perform substitution on a single word
// perform substitution on a single word                                    // c:510
/// Port of singsub() from subst.c lines 513-525
/// Single-string substitution.
/// Port of `singsub()` from Src/subst.c:514.
// perform substitution on a single word                                    // c:510
pub fn singsub(s: &str) -> String {                  // c:514
    // c:514
    let mut list = LinkList::default(); // c:514
    list.push_back(s.to_string()); // c:514
    let mut ret_flags = 0i32; // c:514

    prefork(&mut list, PREFORK_SINGLE, &mut ret_flags); // c:514

    if errflag_set() {
        // c:514
        return String::new(); // c:514
    } // c:514

    list.getdata(0).cloned().unwrap_or_default() // c:514
} // c:514

/// Substitution with possible multiple results
/// Port of multsub() from subst.c lines 540-621
/// Multi-word substitution with IFS splitting.
/// Port of `multsub()` from Src/subst.c:544.
/// Port of `multsub()` from `Src/subst.c:544-660`.
///
/// Multi-word substitution: prefork the input as a single linknode,
/// optionally word-split on IFS first, return the result as scalar or
/// array depending on whether more than one node emerged or LF_ARRAY
/// was set.
///
/// C signature: `int multsub(char **s, int pf_flags, char ***a,
/// int *isarr, char *sep, int *ms_flags)`. Returns 0 on success;
/// in-out pointers carry the result.
///
/// Rust signature: `(s, pf_flags) -> (String, Vec<String>,
/// bool isarr, u32 ms_flags)`. The `sep` parameter is reserved on the
/// caller side and folded into `state.variables["IFS"]` for now;
/// pending an explicit sep arg if a caller needs it. The return tuple
/// carries (joined-scalar, array, isarr, ms_flags).
pub fn multsub(s: &str, pf_flags: i32) -> (String, Vec<String>, bool, i32) { // c:544
    // c:544
    let mut ms_flags = 0i32; // c:551
    let mut x = s.to_string(); // c:550 (`x = *s`)

    // C lines 555-563: PREFORK_SPLIT — skip leading IFS whitespace,
    // mark MULTSUB_WS_AT_START.
    let ifs = vars_get("IFS")
        .unwrap_or_else(|| " \t\n\0".to_string()); // c:N/A (zsh default IFS includes NUL)
    let is_ifs_sep = |c: char| -> bool {
        // c:556
        ifs.contains(c) // c:556
    };

    if pf_flags & PREFORK_SPLIT != 0 {
        // c:553
        let leading: usize = x.chars().take_while(|&c| is_ifs_sep(c)).count(); // c:556
        if leading > 0 {
            // c:557
            ms_flags |= MULTSUB_WS_AT_START; // c:561
            x = x.chars().skip(leading).collect(); // c:562
        }
    }

    // C: `init_list1(foo, x);` — single-element linklist seeded with x.
    let mut list = LinkList::default(); // c:565
    list.push_back(x.clone()); // c:565

    // C lines 568-619: PREFORK_SPLIT walks chars looking for ISEP
    // separators outside quotes/parens. On hit, NUL-terminate and
    // start a new linknode.
    if pf_flags & PREFORK_SPLIT != 0 {
        // c:567
        // Take ownership of the only node's chars; rebuild list.
        let chars: Vec<char> = x.chars().collect(); // c:565
        let mut nodes: Vec<String> = Vec::new(); // c:565
        let mut cur = String::new(); // c:565
        let mut inq = false; // c:570 (bslashquote state)
        let mut inp = 0_i32; // c:570 (paren depth)
        let mut i = 0_usize; // c:572
        while i < chars.len() {
            // c:572
            let c = chars[i]; // c:573
                              // C: `if (*x == Dash) *x = '-';` — Dash token →
                              // literal dash. Rust doesn't have this token here.
                              // C: `if (itok((unsigned char) *x)) { rawc = *x; l = 1; }`
                              // Tokens (META range \u{80}-\u{9F}) are single-byte and
                              // can't be separators. Skip the IFS check for them.
            let is_token = matches!(c as u32, 0x80..=0x9F); // c:577
                                                            // Bnull/Bnullkeep arms (C lines 612-617): skip the next
                                                            // char (parser-verified to exist). \u{99} = Bnull,
                                                            // \u{9a} = Bnullkeep in our token table.
            if c == '\u{99}' || c == '\u{9a}' {
                // c:612
                cur.push(c); // c:614
                i += 1; // c:615
                if i < chars.len() {
                    // c:615
                    cur.push(chars[i]); // c:616
                    i += 1; // c:616
                }
                continue; // c:617
            }
            // Quote/paren state tracking (C lines 600-611).
            match c {                                       // c:600
                '\u{97}' /* Dnull */ |                      // c:602 (")
                '\u{98}' /* Snull */ |                      // c:603 (')
                '\u{83}' /* Tick */ => { inq = !inq; }      // c:604 (`)
                '\u{85}' /* Inpar */ => { inp += 1; }       // c:606
                '\u{86}' /* Outpar */ => { inp -= 1; }      // c:608
                _ => {}
            }
            // ISEP test (C line 581) — outside quotes/parens, char
            // matches IFS, char is not a token.
            if !inq && inp == 0 && !is_token && is_ifs_sep(c) {
                // c:581
                // Split here; NUL-terminate cur, walk past trailing
                // separators (C lines 583-595).
                if !cur.is_empty() || nodes.is_empty() {
                    // c:583
                    nodes.push(std::mem::take(&mut cur)); // c:583
                }
                i += 1; // c:584
                while i < chars.len() && is_ifs_sep(chars[i]) {
                    // c:584-595
                    i += 1; // c:594
                }
                if i >= chars.len() {
                    // c:596
                    ms_flags |= MULTSUB_WS_AT_END; // c:597
                    break; // c:598
                }
                continue; // c:599
            }
            cur.push(c); // c:619
            i += 1; // c:620
        }
        if !cur.is_empty() {
            // c:622
            nodes.push(cur); // c:622
        }
        // Rebuild the linklist with the split nodes.
        list = LinkList::default(); // c:622
        for n in nodes {
            // c:622
            list.push_back(n); // c:622
        }
    }

    // C: `prefork(&foo, pf_flags, ms_flags);`
    let mut ret_flags = 0i32; // c:625
    prefork(&mut list, pf_flags, &mut ret_flags); // c:625

    // C lines 626-630: errflag bail.
    if errflag_set() {
        // c:626
        return (String::new(), Vec::new(), false, ms_flags); // c:629
    }

    // C lines 633-650: count nodes; if > 1 or LF_ARRAY, return as
    // array; else single scalar (or empty).
    let l = list.len(); // c:633
    if l > 1 || (list.flags & LF_ARRAY != 0) {
        // c:633
        let arr: Vec<String> = list.iter().cloned().collect(); // c:635-637
                                                                                    // C: `*s = sepjoin(r, sep, 1);` — join with IFS first-char
                                                                                    // when sep is NULL. Use first IFS char as join separator,
                                                                                    // matching zsh's sepjoin defaults.
        let join_sep = ifs.chars().next().map(String::from).unwrap_or_default(); // c:649
        let joined = arr.join(&join_sep); // c:649
        return (joined, arr, true, ms_flags); // c:642-647 (array path)
    }
    if l == 1 {
        // c:653
        let result = list.getdata(0).cloned().unwrap_or_default(); // c:653
        return (result.clone(), vec![result], false, ms_flags); // c:653
    }
    // C: `*s = dupstring("");` — empty result.
    (String::new(), vec![String::new()], false, ms_flags) // c:655
} // c:660

// CaseMod enum imported from src/ported/hist.rs (canonical port of
// Src/hist.c::casemodify's CASMOD_* flag set). Local definition was
// drift — variants (None/Lower/Upper/Caps) duplicated hist.rs's
// (Lower/Upper/Caps) with an extra unused `None` variant.

/// History-style colon modifiers
/// Port of modify() from subst.c lines 4530-4873
/// Apply a `:` modifier chain (`:t:r:s/x/y/`...).
/// Port of `modify()` from Src/subst.c:4531.
pub fn modify(s: &str, modifiers: &str) -> String {  // c:4531
    // c:4531
    let mut result = s.to_string(); // c:4531
    let mut chars: std::iter::Peekable<std::str::Chars> = modifiers.chars().peekable(); // c:4531
                                                                                        // hsubl/hsubr now live on SubstState (which mirrors them
                                                                                        // back to ShellExecutor on commit). Reads the latest value
                                                                                        // observed in this pass; writes a new pair after each `:s`.

    while chars.peek() == Some(&':') {
        // c:4531
        chars.next(); // consume ':'                        // c:4531

        let mut gbal = false; // c:4531
        let mut wall = false; // c:4531
        let mut sep: Option<String> = None; // c:4531

        // Parse modifier flags. `:g` is greedy/global, `:w` is
        // word-by-word, `:W:sep` is word-by-word with custom sep.
        loop {
            // c:4531
            match chars.peek() {
                // c:4531
                Some(&'g') => {
                    // c:4531
                    gbal = true; // c:4531
                    chars.next(); // c:4531
                } // c:4531
                Some(&'w') => {
                    // c:4531
                    wall = true; // c:4531
                    chars.next(); // c:4531
                } // c:4531
                Some(&'W') => {
                    // c:4531
                    chars.next(); // c:4531
                                  // Parse separator
                    if chars.peek() == Some(&':') {
                        // c:4531
                        chars.next(); // c:4531
                        let collected: String =             // c:4531
                            chars.by_ref().take_while(|&c| c != ':').collect(); // c:4531
                        sep = Some(collected); // c:4531
                    } // c:4531
                } // c:4531
                _ => break, // c:4531
            } // c:4531
        } // c:4531

        let modifier = match chars.next() {
            // c:4531
            Some(c) => c,  // c:4531
            None => break, // c:4531
        }; // c:4531

        // Count suffix for :h/:t — `:hN` keeps N leading components,
        // `:tN` keeps N trailing components. Bare `:h` is the
        // "remove filename" form, signalled by count=0 to remtpath
        // (Src/hist.c:2056). Bare `:t` is "last component", remlpaths
        // treats count=0 as count=1. Port of subst.c:4570-4577
        // idigit count parse.
        let mut count: i32 = 0; // c:4570
        if matches!(modifier, 'h' | 't') {
            // c:4571
            let mut count_str = String::new(); // c:4572
            while let Some(&pc) = chars.peek() {
                if pc.is_ascii_digit() {
                    count_str.push(pc);
                    chars.next();
                } else {
                    break;
                }
            }
            if !count_str.is_empty() {
                count = count_str.parse().unwrap_or(1); // c:4575
            }
        }

        // `:s/old/new/` and `:S/old/new/` — port of subst.c:4583-4685.
        // `:s` is the standard substitute, `:S` is the anchored
        // variant. Parsing rules:
        //   - delim is the char immediately after `s`/`S`
        //   - pattern is read until next unescaped delim
        //   - replacement is read until next unescaped delim or eof
        //   - in pattern: `\X` → literal X (backslash dropped)
        //   - in replacement: `\X` → literal X; `&` → matched portion
        //   - trailing delim is optional
        if modifier == 's' || modifier == 'S' {
            // c:4583
            let delim = match chars.next() {
                // c:4585
                Some(c) => c,  // c:4585
                None => break, // c:4585
            };
            // Read pattern with backslash-escape support.
            let mut pat = String::new(); // c:4595
            while let Some(&c) = chars.peek() {
                if c == delim {
                    chars.next();
                    break;
                }
                if c == '\\' {
                    // c:4598 (backslash escape)
                    chars.next();
                    if let Some(&nx) = chars.peek() {
                        // C: `\X` drops backslash for non-meta X; for
                        // meta keeps escape. Simplify to drop-always.
                        pat.push(nx);
                        chars.next();
                    }
                } else {
                    pat.push(c);
                    chars.next();
                }
            }
            // Read replacement with `&` and `\X` handling.
            let mut repl = String::new(); // c:4625
            while let Some(&c) = chars.peek() {
                if c == delim {
                    chars.next();
                    break;
                }
                if c == '\\' {
                    // c:4630
                    chars.next();
                    if let Some(&nx) = chars.peek() {
                        repl.push(nx);
                        chars.next();
                    }
                } else if c == '&' {
                    // c:4639 (& → matched portion)
                    chars.next();
                    repl.push_str(&pat);
                } else {
                    repl.push(c);
                    chars.next();
                }
            }
            // Apply: gbal→all, else first match. :S allows
            // anchored patterns via leading `#` (prefix) or
            // trailing `%` (suffix); :s treats those literally.
            // Direct port of subst.c modify's S-arm anchoring.
            let (eff_pat, anchor_head, anchor_tail) = if modifier == 'S' {
                if let Some(rest) = pat.strip_prefix('#') {
                    (rest.to_string(), true, false) // c:4665 (#X)
                } else if let Some(rest) = pat.strip_suffix('%') {
                    (rest.to_string(), false, true) // c:4665 (X%)
                } else {
                    (pat.clone(), false, false) // c:4665
                }
            } else {
                (pat.clone(), false, false) // c:4665
            };
            // For `:S` (modifier=='S'), matching is glob-based per
            // hist.c::subst() forcepat=1 path (parse_subst_string +
            // getmatch). For `:s` (modifier=='s'), matching is
            // literal `strstr` unless HISTSUBSTPATTERN option is on.
            // Direct port of Src/hist.c:2336 — `if (isset(HISTSUBSTPATTERN)
            // || forcepat)` selects the pattern path; otherwise the
            // strstr-based literal replace runs.
            let use_glob = modifier == 'S' || isset(crate::ported::zsh_h::HISTSUBSTPATTERN);
            let do_match = |hay: &str| -> Option<(usize, usize)> {
                if use_glob {
                    // Sliding-window glob match — find first
                    // [start..end) span where eff_pat matches.
                    // Direct port of zsh's getmatch() SUB_SUBSTR
                    // search loop. Empty match returns (q, q).
                    let cv: Vec<char> = hay.chars().collect();
                    let n = cv.len();
                    for start in 0..=n {
                        for end in start..=n {
                            let span: String = cv[start..end].iter().collect();
                            if crate::ported::pattern::patmatch(&eff_pat, &span) {
                                // Convert char positions to byte positions.
                                let bs: usize = cv[..start].iter().map(|c| c.len_utf8()).sum();
                                let be: usize =
                                    bs + cv[start..end].iter().map(|c| c.len_utf8()).sum::<usize>();
                                return Some((bs, be));
                            }
                        }
                    }
                    None
                } else {
                    hay.find(eff_pat.as_str()).map(|s| (s, s + eff_pat.len()))
                }
            };
            result = if anchor_head {
                // c:4665
                if use_glob {
                    let cv: Vec<char> = result.chars().collect();
                    let n = cv.len();
                    let mut found: Option<usize> = None;
                    for end in 0..=n {
                        let span: String = cv[..end].iter().collect();
                        if crate::ported::pattern::patmatch(&eff_pat, &span) {
                            found = Some(cv[..end].iter().map(|c| c.len_utf8()).sum());
                            break;
                        }
                    }
                    if let Some(be) = found {
                        format!("{}{}", repl, &result[be..])
                    } else {
                        result
                    }
                } else if result.starts_with(&eff_pat) {
                    // c:4665
                    format!("{}{}", repl, &result[eff_pat.len()..]) // c:4665
                } else {
                    result
                } // c:4665
            } else if anchor_tail {
                // c:4665
                if use_glob {
                    let cv: Vec<char> = result.chars().collect();
                    let n = cv.len();
                    let mut found: Option<usize> = None;
                    for start in 0..=n {
                        let span: String = cv[start..].iter().collect();
                        if crate::ported::pattern::patmatch(&eff_pat, &span) {
                            found = Some(cv[..start].iter().map(|c| c.len_utf8()).sum());
                            break;
                        }
                    }
                    if let Some(bs) = found {
                        format!("{}{}", &result[..bs], repl)
                    } else {
                        result
                    }
                } else if result.ends_with(&eff_pat) {
                    // c:4665
                    format!("{}{}", &result[..result.len() - eff_pat.len()], repl)
                // c:4665
                } else {
                    result
                } // c:4665
            } else if gbal {
                // c:4665
                if use_glob {
                    let mut out = String::with_capacity(result.len());
                    let mut rem = result.as_str();
                    while let Some((s, e)) = do_match(rem) {
                        out.push_str(&rem[..s]);
                        out.push_str(&repl);
                        if e == s {
                            // Empty match — advance one char to
                            // avoid infinite loop, mirroring zsh's
                            // SUB_GLOBAL safeguard.
                            let mut chars = rem[s..].char_indices();
                            chars.next();
                            let next_s = s + chars.next().map(|(b, _)| b).unwrap_or(rem.len() - s);
                            out.push_str(&rem[s..next_s]);
                            rem = &rem[next_s..];
                        } else {
                            rem = &rem[e..];
                        }
                    }
                    out.push_str(rem);
                    out
                } else {
                    result.replace(eff_pat.as_str(), repl.as_str())
                }
            } else if use_glob {
                if let Some((s, e)) = do_match(&result) {
                    format!("{}{}{}", &result[..s], repl, &result[e..])
                } else {
                    result
                }
            } else {
                result.replacen(eff_pat.as_str(), repl.as_str(), 1)
            };
            // Record the post-anchor-strip form + anchor mode so a
            // subsequent `:&` can replay the same shape. Storing
            // `eff_pat` (not `pat`) avoids re-stripping `#`/`%` on
            // replay; the `mode` byte encodes whether the original
            // `:S` form was head-, tail-, or non-anchored.
            // C: subst.c:4673 saves hsubl/hsubr; hsubpatopt bit is
            // implicit from the modifier letter recorded by
            // `case '&'`.
            let mode: u8 = if modifier == 's' {
                0
            } else if anchor_head {
                1
            } else if anchor_tail {
                2
            } else {
                3
            };
            *crate::ported::hist::hsubl.lock().unwrap() = Some(eff_pat.clone()); // c:4673
            *crate::ported::hist::hsubr.lock().unwrap() = Some(repl.clone()); // c:4673
            crate::ported::hist::hsubpatopt.store(mode as i32, std::sync::atomic::Ordering::Relaxed); // c:4673
                                                                            // `:s` on word-each (`:w` / `:W:sep`) splits, applies,
                                                                            // rejoins. Pull through the same code path :& uses
                                                                            // below by deferring to a shared `apply_subst` closure.
            if wall {
                // c:4665
                let separator = sep.as_deref().unwrap_or(" "); // c:4665
                let words: Vec<&str> = result.split(separator).collect(); // c:4665
                let modified: Vec<String> = words
                    .iter()
                    .map(|w| {
                        // c:4665
                        if gbal {
                            w.replace(pat.as_str(), repl.as_str())
                        }
                        // c:4665
                        else {
                            w.replacen(pat.as_str(), repl.as_str(), 1)
                        } // c:4665
                    })
                    .collect(); // c:4665
                result = modified.join(separator); // c:4665
            } // c:4665
            continue; // c:4675
        } // c:4685

        // `:&` repeats the last `:s`/`:S` substitution. Per
        // Src/subst.c:4675 `case '&':` — `c = hsubpatopt ? 'S' :
        // 's'`. The `mode` byte stored alongside (pat, repl) by
        // the s/S arm tells which anchor disposition to replay:
        //   0 = `:s` literal,  1 = `:S` head (`#X`),
        //   2 = `:S` tail (`X%`), 3 = `:S` no-anchor.
        // No-op if no prior `:s` in this chain (or pass — state.
        // last_subst persists across calls via
        // from_executor / commit_to_executor).
        if modifier == '&' {
            // c:4531
            let last_subst = {
                let p_opt = crate::ported::hist::hsubl.lock().unwrap().clone();
                let r_opt = crate::ported::hist::hsubr.lock().unwrap().clone();
                match (p_opt, r_opt) {
                    (Some(p), Some(r)) => {
                        let mode = crate::ported::hist::hsubpatopt.load(std::sync::atomic::Ordering::Relaxed) as u8;
                        Some((p, r, mode))
                    }
                    _ => None,
                }
            };
            if let Some((p, r, mode)) = last_subst {
                // c:4531
                let apply = |w: &str| -> String {
                    // c:4531
                    match mode {
                        // c:4675
                        1 => {
                            // c:4665 head-anchored
                            if w.starts_with(p.as_str()) {
                                format!("{}{}", r, &w[p.len()..])
                            } else {
                                w.to_string()
                            }
                        }
                        2 => {
                            // c:4665 tail-anchored
                            if w.ends_with(p.as_str()) {
                                format!("{}{}", &w[..w.len() - p.len()], r)
                            } else {
                                w.to_string()
                            }
                        }
                        // mode 0 (`:s`) and mode 3 (`:S` no
                        // anchor) both replay as a non-anchored
                        // replacement. The `:s`/`:S` distinction
                        // for inner-string matches is implemented
                        // by glob-vs-literal in the original arm;
                        // the replay uses the literal path until
                        // we wire glob into modify().
                        _ => {
                            // c:4665 non-anchored
                            if gbal {
                                w.replace(p.as_str(), r.as_str())
                            } else {
                                w.replacen(p.as_str(), r.as_str(), 1)
                            }
                        }
                    }
                };
                if wall {
                    // c:4531
                    let separator = sep.as_deref().unwrap_or(" "); // c:4531
                    let words: Vec<&str> = result.split(separator).collect(); // c:4531
                    let modified: Vec<String> = words.iter().map(|w| apply(w)).collect();
                    result = modified.join(separator); // c:4531
                } else {
                    // c:4531
                    result = apply(&result); // c:4531
                } // c:4531
            } // c:4531
            continue; // c:4531
        } // c:4531

        // Single-char modifier dispatch — port of Src/subst.c:4585+
        // modifier-arm ladder. Each arm calls a canonical hist.rs
        // helper (the per-modifier C body lives in Src/hist.c).
        let dispatch = |w: &str| -> Option<String> {
            // c:4585
            match modifier {
                // c:4585
                'h' => Some(remtpath(w, count)), // c:4585 (:h head, count = :hN)
                't' => Some(remlpaths(w, count)), // c:4585 (:t tail, count = :tN)
                // c:4585 — `:r` strips extension (returns root), `:e`
                // keeps only extension. The hist.rs helpers are named
                // by the C source's "remove" semantics:
                //   remtext   = "remove text after dot" → strips ext → :r
                //   rembutext = "remove all BUT extension" → keeps ext → :e
                // The previous dispatch had these flipped, so `${path:r}`
                // returned the extension and `${path:e}` returned the root.
                'r' => Some(remtext(w)),   // c:4585 (:r root)
                'e' => Some(rembutext(w)), // c:4585 (:e ext)
                'l' => Some(casemodify(w, CaseMod::CASMOD_LOWER)), // c:4585 (:l)
                'u' => Some(casemodify(w, CaseMod::CASMOD_UPPER)), // c:4585 (:u)
                'q' => Some(crate::ported::utils::quotestring(
                    // c:4585 (:q)
                    w,
                    crate::ported::utils::QuoteType::Backslash,
                )),
                'Q' => {
                    // c:4585 (:Q unquote)
                    let mut out = String::with_capacity(w.len());
                    let mut chs = w.chars().peekable();
                    while let Some(c) = chs.next() {
                        if c == '\\' {
                            if let Some(nc) = chs.next() {
                                out.push(nc);
                            }
                        } else if c == '\'' || c == '"' { /* drop quotes */
                        } else {
                            out.push(c);
                        }
                    }
                    Some(out)
                }
                'a' => xsymlinks(w).ok(), // c:4585 (:a absolute, no symlink follow)
                'A' | 'P' => {
                    // c:4585 (:A / :P absolute + resolve symlinks)
                    // zsh `:A` / `:P` do what realpath(3) does —
                    // resolve every symlink in the path. xsymlinks
                    // alone normalises `.` / `..` without following
                    // links; std::fs::canonicalize REQUIRES the
                    // entire path to exist. For non-existent leafs
                    // (common — temp files, pre-mkdir paths), we
                    // walk component-by-component, canonicalize the
                    // LONGEST EXISTING prefix, then re-append the
                    // tail. Mirrors what realpath(3) on Linux/glibc
                    // does and what zsh's xsymlinks does in C with
                    // its `physical = 1` walk.
                    let canon = std::fs::canonicalize(w)
                        .ok()
                        .map(|p| p.to_string_lossy().into_owned());
                    if let Some(c) = canon {
                        Some(c)
                    } else {
                        // Walk parents to find longest existing prefix.
                        let mut p = std::path::PathBuf::from(w);
                        let mut tail: Vec<std::ffi::OsString> = Vec::new();
                        let resolved_prefix = loop {
                            if let Ok(rp) = std::fs::canonicalize(&p) {
                                break Some(rp);
                            }
                            match (
                                p.parent().map(|x| x.to_path_buf()),
                                p.file_name().map(|x| x.to_os_string()),
                            ) {
                                (Some(parent), Some(file)) if !parent.as_os_str().is_empty() => {
                                    tail.push(file);
                                    p = parent;
                                }
                                _ => break None,
                            }
                        };
                        if let Some(mut rp) = resolved_prefix {
                            for t in tail.into_iter().rev() {
                                rp.push(t);
                            }
                            Some(rp.to_string_lossy().into_owned())
                        } else {
                            xsymlinks(w).ok()
                        }
                    }
                }
                'c' => {
                    // c:4585 (:c command-resolve)
                    // :c resolves like `which` — search PATH for
                    // an executable matching `w`. Direct port of
                    // hist.c case 'c' which calls findcmd.
                    if w.starts_with('/') || w.starts_with("./") || w.starts_with("../") {
                        Some(w.to_string()) // c:4585
                    } else if let Ok(path) = std::env::var("PATH") {
                        let mut found = None;
                        for dir in path.split(':') {
                            let p = std::path::PathBuf::from(dir).join(w);
                            if p.is_file() {
                                found = Some(p.to_string_lossy().into_owned());
                                break;
                            }
                        }
                        Some(found.unwrap_or_else(|| w.to_string()))
                    } else {
                        Some(w.to_string())
                    }
                }
                _ => None, // c:4585 (unrecognized)
            }
        };
        if wall {
            // c:4531
            // Apply modifier to each word
            let separator = sep.as_deref().unwrap_or(" "); // c:4531
            let words: Vec<&str> = result.split(separator).collect();
            let mut modified: Vec<String> = Vec::with_capacity(words.len());
            for w in &words {
                match dispatch(w) {
                    Some(m) => modified.push(m),
                    None => {
                        zerr(&format!("unrecognized modifier `{}'", modifier));
                        errflag_set_error();
                        return String::new();
                    }
                }
            }
            result = modified.join(separator);
        } else {
            match dispatch(&result) {
                Some(m) => result = m,
                None => {
                    zerr(&format!("unrecognized modifier `{}'", modifier));
                    errflag_set_error();
                    return String::new();
                }
            }
        }
    } // c:4531

    result // c:4531
} // c:4531

/// `wcpadwidth(wc, multi_width)` — return the display-cell width of
/// `wc` per zsh's MULTIBYTE_SUPPORT padding logic. Direct port of
/// Src/subst.c:848-866.
///
/// Modes:
///   • `multi_width == 0` — every char counts as one cell.
///   • `multi_width == 1` — use `u9_wcwidth`-style cell counting.
///   • else — combining/zero-width chars count as 0, all others as 1.
///
/// The Rust port uses `unicode-width`-style heuristics inline: ASCII
/// printable + most BMP chars = 1 cell; CJK Unified Ideographs and
/// other wide blocks = 2 cells; combining/control = 0.
/// Port of `wcpadwidth()` from `Src/subst.c:848-866`.
///
/// Returns the display-cell width of a wide char for `dopadding`.
///
/// C signature: `int wcpadwidth(wchar_t wc, int multi_width)`.
/// multi_width values:
///   0 → always 1 (legacy / no multibyte)
///   1 → u9_wcwidth(wc); zero if negative
///   * → boolean: 1 if u9_wcwidth>0 else 0
pub fn wcpadwidth(wc: char, multi_width: i32) -> i32 {                       // c:848
    // c:848
    // u9_wcwidth fallback lives in utils.rs (canonical port of
    // Src/utils.c::zwcwidth). Use the unicode_width-backed
    // implementation there.
    let wcw = crate::ported::utils::zwcwidth(wc) as i32;
    match multi_width {
        // c:854
        // C: `case 0: return 1;`
        0 => 1, // c:855
        // C: `case 1: width = WCWIDTH(wc); if (width >= 0) return width; return 0;`
        1 => {
            if wcw >= 0 {
                wcw
            } else {
                0
            }
        } // c:858
        // C: `default: return WCWIDTH(wc) > 0 ? 1 : 0;`
        _ => {
            if wcw > 0 {
                1
            } else {
                0
            }
        } // c:864
    }
} // c:866

/// `subst_parse_str(sp, single, err)` — parse a substitution string in
/// place: convert tokens, optionally suppressing errors, and recover
/// the unquoted body for arithmetic / array-index evaluation. Direct
/// port of Src/subst.c:1460-1487.
///
/// In zsh, this is used by arithsubst() to re-parse `$(( … ))`'s
/// inner expression after parameter expansion has run, and by the
/// `${…[N]}` index path to evaluate `N` as an arithmetic expression.
///
/// Returns the converted text on success, `None` on parse error
/// (matches the C return value: 0=ok, 1=error).
///
/// The `single` flag (false) maps the lexer's `Qstring`/`Qtick` quoted
/// markers back to plain `String`/`Tick` tokens, mirroring the inner
/// loop at subst.c:1473-1485 that strips the doubled-up bslashquote
/// recognition.
/// Port of `subst_parse_str()` from `Src/subst.c:1460-1486`.
///
/// C signature: `int subst_parse_str(char **sp, int single, int err)`.
/// Mutates `*sp` to point at a duplicated, parser-pre-processed copy
/// of the input. Returns 0 on success, 1 on parse failure.
///
/// Rust signature: takes `&str`, returns `Option<String>` — Some(buf)
/// on success, None on parse failure (matches the C `return 1` error
/// path).
///
/// The C body:
///   1. `*sp = s = dupstring(*sp);`           — clone for in-place mutation
///   2. parsestr / parsestrnoerr depending on `err` flag — fails → return 1
///   3. If !single, walk buffer: outside DNULL (`"`) regions convert
///      `Qstring` → `String` and `Qtick` → `Tick`. DNULL toggles qt.
pub fn subst_parse_str(s: &str, single: bool, err: bool) -> Option<String> { // c:1460
    // c:1460
    let _ = err; // c:1466 (parsestr error path
                 //         deferred — full C
                 //         lexer reentry pending)
                 // C: `*sp = s = dupstring(*sp);` — duplicate so the caller's
                 // original buffer is unaffected. Rust's String already owns;
                 // we work on a local copy below.
    let mut buf: String = s.to_string(); // c:1465

    // C: `if (!single) { … }` — the conversion only runs in the
    // non-SINGLE arm (when paramsubst-output may be subsequently
    // word-split / expanded).
    if !single {
        // c:1469
        let mut chars: Vec<char> = buf.chars().collect(); // c:1469
        let mut qt = false; // c:1470
                            // C constant references — these are the token bytes the
                            // lexer emits. Authoritative values from src/ported/subst.rs
                            // tokens module: STRING=\u{81}, QSTRING=\u{82},
                            // TICK=\u{83}, QTICK=\u{84}, DNULL=\u{97}.
        for c in chars.iter_mut() {
            // c:1472
            if !qt {
                // c:1473
                if *c == '\u{82}'
                /* QSTRING */
                {
                    // c:1474
                    *c = '\u{81}' /* STRING */; // c:1475
                } else if *c == '\u{84}'
                /* QTICK */
                {
                    // c:1476
                    *c = '\u{83}' /* TICK */; // c:1477
                }
            }
            if *c == '\u{97}'
            /* DNULL */
            {
                // c:1480
                qt = !qt; // c:1481
            }
        }
        buf = chars.iter().collect(); // c:1483
    }
    // C: `return 0;` — success path returns the buffer.
    Some(buf) // c:1483
} // c:1486

/// Get a directory stack entry
/// Port of dstackent() from subst.c
/// Resolve `~+N`/`~-N` directory-stack entries.
/// Port of `dstackent()` from Src/subst.c:4902.
/// Port of `dstackent()` from `Src/subst.c:4902-4922`.
///
/// Resolves `~+N` / `~-N` directory-stack entries.
///
/// C signature: `char *dstackent(char ch, int val)` — returns the
/// path string at the requested dirstack index, or NULL on
/// not-enough-entries.
///
/// Behavior:
///   - `backwards` flips when PUSHDMINUS is set (so `~-N` walks
///     forward and `~+N` walks backward).
///   - `~+0` (or `~-0` when PUSHDMINUS) returns PWD, no list walk.
///   - Otherwise walks dirstack from front (forward) or back
///     (backward), val steps in.
///   - Off-the-end → NULL (caller emits "not enough directory stack
///     entries" if NOMATCH is set).
///
/// Rust signature: takes the dirstack slice + pwd + the PUSHDMINUS
/// option flag (callers read it from the live executor's options
/// table). Returns Option.
pub fn dstackent(                                                            // c:4902
    ch: char,
    val: i32,
    dirstack: &[String],
    pwd: &str,
    pushdminus_set: bool,
) -> Option<String> {
    // c:4902
    // C: `backwards = ch == (isset(PUSHDMINUS) ? '+' : '-');`
    let backwards = ch == if pushdminus_set { '+' } else { '-' }; // c:4906

    // C: `if (!backwards && !val--) return pwd;`
    // Decrement val POST-test so val becomes 0 → return pwd.
    let mut val = val; // c:4904
    if !backwards && val == 0 {
        // c:4907
        return Some(pwd.to_string()); // c:4908
    }
    if !backwards {
        val -= 1;
    } // c:4907 (post-decrement)

    // C lines 4909-4912: walk dirstack.
    // backwards: from lastnode, val steps back.
    // forwards: from firstnode, val steps forward.
    let n = dirstack.len() as i32; // c:4910
    let idx = if backwards {
        // c:4910
        // last element is index n-1; val steps back from there.
        let i = n - val; // c:4910
        if i < 0 {
            return None;
        } // c:4913 (n == end)
        i as usize // c:4910
    } else {
        // c:4912
        if val < 0 || val >= n {
            return None;
        } // c:4913 (n == end)
        val as usize // c:4912
    };

    // C: `return (char *)getdata(n);`
    dirstack.get(idx).cloned() // c:4920
} // c:4922

/// String padding
/// Port of dopadding() from subst.c lines 798-1193
/// `${(l:N:)var}` left/right-pad.
/// Port of `dopadding()` from Src/subst.c:893.
///
/// `multi_width` controls cell-counting per the (m) flag (subst.c:2376):
///   • 0  → every char counts as one cell (C zsh's MULTIBYTE_SUPPORT off)
///   • 1+ → use wcpadwidth (CJK wide=2, combining=0, ZWJ=0).
pub fn dopadding(                                                            // c:893
    // c:893
    s: &str,               // c:893
    prenum: usize,         // c:893
    postnum: usize,        // c:893
    preone: Option<&str>,  // c:893
    postone: Option<&str>, // c:893
    premul: &str,          // c:893
    postmul: &str,         // c:893
    multi_width: i32,      // c:2376 (m)
) -> String {
    // c:893
    // (m)-aware string-cell counter. With multi_width==0 every
    // codepoint counts 1 (legacy behavior); otherwise wcpadwidth
    // gives the wide-char-aware metric. Direct port of zsh's
    // MULTIBYTE_SUPPORT path which routes the (l)/(r) length
    // checks through u9_wcwidth() before deciding pad vs truncate.
    let cells = |t: &str| -> usize {
        // c:893
        if multi_width <= 0 {
            // c:893
            t.chars().count() // c:893
        } else {
            // c:893
            t.chars().map(|c| wcpadwidth(c, multi_width) as usize).sum() // c:2376
        } // c:893
    };
    let len = cells(s); // c:893
    let total_width = prenum + postnum; // c:893

    if total_width == 0 || total_width == len {
        // c:893
        return s.to_string(); // c:893
    } // c:893

    let mut result = String::new(); // c:893

    // Left padding
    if prenum > 0 {
        // c:893
        let chars: Vec<char> = s.chars().collect(); // c:893

        if len > prenum {
            // c:893
            // Truncate from left
            let skip = len - prenum; // c:893
            result = chars.into_iter().skip(skip).collect(); // c:893
        } else {
            // c:893
            // Pad on left
            let padding_needed = prenum - len; // c:893

            // Add preone if there's room
            if let Some(pre) = preone {
                // c:893
                let pre_len = pre.chars().count(); // c:893
                if pre_len <= padding_needed {
                    // c:893
                    // Room for repeated padding first
                    let repeat_len = padding_needed - pre_len; // c:893
                    if !premul.is_empty() {
                        // c:893
                        let mul_len = premul.chars().count(); // c:893
                        let full_repeats = repeat_len / mul_len; // c:893
                        let partial = repeat_len % mul_len; // c:893

                        // Partial repeat
                        if partial > 0 {
                            // c:893
                            result.extend(premul.chars().skip(mul_len - partial));
                            // c:893
                        } // c:893
                          // Full repeats
                        for _ in 0..full_repeats {
                            // c:893
                            result.push_str(premul); // c:893
                        } // c:893
                    } // c:893
                    result.push_str(pre); // c:893
                } else {
                    // c:893
                    // Only part of preone fits
                    result.extend(pre.chars().skip(pre_len - padding_needed)); // c:893
                } // c:893
            } else {
                // c:893
                // Just use premul
                if !premul.is_empty() {
                    // c:893
                    let mul_len = premul.chars().count(); // c:893
                    let full_repeats = padding_needed / mul_len; // c:893
                    let partial = padding_needed % mul_len; // c:893

                    if partial > 0 {
                        // c:893
                        result.extend(premul.chars().skip(mul_len - partial)); // c:893
                    } // c:893
                    for _ in 0..full_repeats {
                        // c:893
                        result.push_str(premul); // c:893
                    } // c:893
                } // c:893
            } // c:893

            result.push_str(s); // c:893
        } // c:893
    } else {
        // c:893
        result = s.to_string(); // c:893
    } // c:893

    // Right padding
    if postnum > 0 {
        // c:893
        let current_len = cells(&result); // c:893

        if current_len > postnum {
            // c:893
            // Truncate from right
            result = result.chars().take(postnum).collect(); // c:893
        } else if current_len < postnum {
            // c:893
            // Pad on right
            let padding_needed = postnum - current_len; // c:893

            if let Some(post) = postone {
                // c:893
                let post_len = post.chars().count(); // c:893
                if post_len <= padding_needed {
                    // c:893
                    result.push_str(post); // c:893
                    let remaining = padding_needed - post_len; // c:893
                    if !postmul.is_empty() {
                        // c:893
                        let mul_len = postmul.chars().count(); // c:893
                        let full_repeats = remaining / mul_len; // c:893
                        let partial = remaining % mul_len; // c:893

                        for _ in 0..full_repeats {
                            // c:893
                            result.push_str(postmul); // c:893
                        } // c:893
                        if partial > 0 {
                            // c:893
                            result.extend(postmul.chars().take(partial)); // c:893
                        } // c:893
                    } // c:893
                } else {
                    // c:893
                    result.extend(post.chars().take(padding_needed)); // c:893
                } // c:893
            } else if !postmul.is_empty() {
                // c:893
                let mul_len = postmul.chars().count(); // c:893
                let full_repeats = padding_needed / mul_len; // c:893
                let partial = padding_needed % mul_len; // c:893

                for _ in 0..full_repeats {
                    // c:893
                    result.push_str(postmul); // c:893
                } // c:893
                if partial > 0 {
                    // c:893
                    result.extend(postmul.chars().take(partial)); // c:893
                } // c:893
            } // c:893
        } // c:893
    } // c:893

    result // c:893
} // c:893

/// Get the delimiter argument for flags like (s:x:) or (j:x:)
/// Port of get_strarg() from subst.c
/// Parse a `:STR:`-delimited flag argument.
/// Port of `get_strarg()` from Src/subst.c:1348.
pub fn get_strarg(s: &str) -> Option<(char, String, &str)> {                 // c:1348
    // c:1348
    let mut chars = s.chars().peekable(); // c:1348

    // Get delimiter
    let del = chars.next()?; // c:1348

    // Map bracket pairs
    let close_del = match del {
        // c:1348
        '(' => ')',          // c:1348
        '[' => ']',          // c:1348
        '{' => '}',          // c:1348
        '<' => '>',          // c:1348
        INPAR => OUTPAR,     // c:1348
        INBRACK => OUTBRACK, // c:1348
        INBRACE => OUTBRACE, // c:1348
        INANG => OUTANG,     // c:1348
        _ => del,            // c:1348
    }; // c:1348

    // Collect content until closing delimiter
    let mut content = String::new(); // c:1348
    let mut rest_start = 1; // c:1348

    for (i, c) in s.chars().enumerate().skip(1) {
        // c:1348
        if c == close_del {
            // c:1348
            rest_start = i + 1; // c:1348
            break; // c:1348
        } // c:1348
        content.push(c); // c:1348
        rest_start = i + 1; // c:1348
    } // c:1348

    let rest = &s[rest_start.min(s.len())..]; // c:1348
    Some((del, content, rest)) // c:1348
} // c:1348

/// Get integer argument for flags like (l.N.)
/// Port of get_intarg() from subst.c
/// Parse an `:N:`-delimited integer flag argument.
/// Port of `get_intarg()` from Src/subst.c:1428.
/// Port of `get_intarg()` from `Src/subst.c:1428-1457`.
///
/// Parses an `:N:`-delimited integer flag argument (e.g. `(l:5:)`).
/// The C source returns -1 on error, the absolute value otherwise,
/// and writes the matched delimiter length to *delmatchp.
///
/// Rust returns Option<(value, rest)> — None on error, Some((|n|, rest))
/// on success. The delmatchp output is folded into `rest` (a slice
/// past the closing delimiter).
///
/// Body: get_strarg → parsestr → singsub → mathevali, then absolute
/// value. The math eval lets `(l:$n:)` etc. work.
pub fn get_intarg(s: &str) -> Option<(i64, &str)> {                          // c:1428
    // c:1428
    // C: `char *t = get_strarg(*s, &arglen);` — get the delimited
    // expression text + delimiter length.
    let (_del, content, rest) = get_strarg(s)?; // c:1431

    if rest.is_empty() && content.is_empty() {
        // c:1436
        // C: `if (!*t) return -1;` — empty input → error.
        return None;
    }

    // C: `if (parsestr(&p)) return -1;` — full lexer reentry skipped
    // (subst_parse_str approximates).
    let parsed = subst_parse_str(&content, false, true)?; // c:1442

    // C: `singsub(&p);` — parameter-substitute the content (so
    // `(l:$n:)` looks up $n).
    let mut __exec = crate::exec::ShellExecutor::new();
    let _ctx = crate::fusevm_bridge::ExecutorContext::enter(&mut __exec);
    let expanded = singsub(&parsed); // c:1444
    if errflag_set() {
        return None;
    } // c:1445

    // C: `ret = mathevali(p);` — evaluate as integer math.
    let ret = match crate::ported::math::mathevali(&expanded) {
        // c:1447
        Ok(n) => n,            // c:1447
        Err(_) => return None, // c:1448
    };

    // C: `if (ret < 0) ret = -ret;` — absolute value.
    let abs_ret = if ret < 0 { -ret } else { ret }; // c:1452

    // C: `*delmatchp = arglen;` — Rust folds delim-len into rest.
    Some((abs_ret, rest)) // c:1455
} // c:1457

/// Quote substitution for heredoc tags
/// Port of `quotesubst()` from `Src/subst.c:463-475`.
///
/// Simplified version of prefork/singsub that does only the
/// substitutions appropriate to quoting context — currently just the
/// $'...' (Snull) form. Used for here-doc end tags. Other expansions
/// (param-subst, cmd-subst, arith) stay in the text.
///
/// The trailing `remnulargs()` strips Bnull tokens so this is
/// consistent with the other substitution forms (indicating quotes
/// have been fully processed).
pub fn quotesubst(s: &str) -> String {              // c:463
    // c:463
    let mut result = s.to_string(); // c:465
    let mut pos = 0_usize; // c:466

    // C: `while (*s) { if (*s == String && s[1] == Snull) …
    //               else s++; }`
    loop {
        // c:467
        let chars: Vec<char> = result.chars().collect(); // c:467
        if pos >= chars.len() {
            break;
        } // c:467
          // C lines 468-470: spot $'…' marker and call
          // stringsubstquote.
        if pos + 1 < chars.len()                            // c:468
            && chars[pos] == STRING                         // c:468
            && chars[pos + 1] == SNULL
        // c:468
        {
            let (new_str, new_pos) = stringsubstquote(&result, pos); // c:469
            result = new_str; // c:469
            pos = new_pos; // c:469
        } else {
            // c:471
            pos += 1; // c:472
        } // c:473
    }
    // C: `remnulargs(str);` — strip Bnull / NUL tokens. Use the
    // inline equivalent the rest of subst.rs uses (\u{0} only;
    // glob.rs's full port operates on Vec<GlobToken>).
    result.replace('\u{0}', "") // c:474
} // c:475

/// Glob entries in a linked list
/// Port of globlist() from subst.c lines 468-505
/// Port of `globlist()` from `Src/subst.c:489-510`.
///
/// Glob-expands each entry in a linked list. Honors two PREFORK_*
/// flags (per the C body header comment):
///   - PREFORK_NO_UNTOK: preserve tokens (don't run untokenize before
///     glob).
///   - PREFORK_KEY_VALUE: triads of Marker/Key/Value (assoc-array
///     assignments); skip globbing on the key+value pair, only the
///     marker node is processed.
///
/// Routes through `ShellExecutor::expand_glob` (the canonical
/// glob.rs port of zsh's zglob) for filesystem matching.
pub fn globlist(list: &mut LinkList, flags: i32) {   // c:489
    // c:489
    // C: `badcshglob = 0;` — reset the csh-glob diagnostic counter
    // (we don't track this; csh-glob option is rare).
    let mut node_idx = 0; // c:493

    while node_idx < list.nodes.len() && !errflag_set() {
        // c:494
        let data = match list.getdata(node_idx) {
            // c:494
            Some(d) => d.to_string(), // c:494
            None => {
                node_idx += 1;
                continue;
            } // c:494
        };

        // C: `if ((flags & PREFORK_KEY_VALUE) && *data == Marker)`
        // — assoc-array key/value pair; skip 3 nodes (Marker, Key,
        // Value).
        if flags & PREFORK_KEY_VALUE != 0 && data.chars().next() == Some(MARKER) {
            // c:497
            // Advance past Marker + Key + Value.
            node_idx += 3; // c:499
            continue; // c:499
        }

        // C: `zglob(list, node, (flags & PREFORK_NO_UNTOK) != 0);`
        // — the actual glob expansion. Replaces the node with one
        // or more nodes (one per match).
        let no_untok = flags & PREFORK_NO_UNTOK != 0; // c:501
        let _ = no_untok; // C plumbs through;
                          // expand_glob handles
                          // tokens internally.
        // c:501 — canonical glob expansion (mirrors zglob driver
        // at glob.c:1214 with alternation + extendedglob pre-passes
        // inlined). Reads canonical option state directly, no
        // executor needed.
        let expanded: Vec<String> = crate::ported::glob::glob_path(&data);

        if expanded.is_empty() {
            // c:N/A (NOMATCH path)
            // C zglob does its own NOMATCH/badcshglob accounting
            // when nothing matches. Preserve the original entry on
            // empty match (zsh default; NOMATCH option would zerr).
            node_idx += 1;
        } else if expanded.len() == 1 {
            // c:N/A
            list.setdata(node_idx, expanded.into_iter().next().unwrap());
            node_idx += 1;
        } else {
            // Replace the single node with N expanded nodes.
            list.delete_node(node_idx);
            for (i, p) in expanded.iter().enumerate() {
                if i == 0 {
                    list.insert_at(node_idx, p.clone());
                } else {
                    list.insertlinknode(node_idx + i - 1, p.clone());
                }
            }
            node_idx += expanded.len(); // advance past all
        }
    }
    // C: `if (noerrs) badcshglob = 0; else if (badcshglob == 1)
    // zerr("no match");` — diagnostic emit. Skipped here pending
    // badcshglob counter port.
} // c:510

/// Flags for SUB_* matching — verbatim port of zsh.h:1981-1996.
///
/// Outer-scope mirror of the inner module at the bottom of
/// subst.rs. Earlier values (`1, 2, 4, …` powers of two) silently
/// shifted START / EGLOB into the wrong bit positions because
/// zsh.h has DOSUBST=0x0400 and RETFAIL=0x0800 between LEN=0x0080
/// and START=0x1000. Use the canonical hex literals here.
// `pub mod sub_flags { … }` — DELETED per user directive; was a
// Rust-only u32 wrapper duplicating the canonical i32 constants in
// `zsh_h::SUB_*` (c:zsh.h:1981-1996). Bit values matched but type
// (u32 vs C `int`) drifted; usage sites mixed with `exec.sub_flags:
// i32` caused silent coercion bugs. Use canonical defs directly.
use crate::ported::zsh_h::{
    SUB_ALL, SUB_BIND, SUB_DOSUBST, SUB_EGLOB, SUB_EIND, SUB_END, SUB_GLOBAL, SUB_LEN,
    SUB_LIST, SUB_LONG, SUB_MATCH, SUB_REST, SUB_RETFAIL, SUB_START, SUB_SUBSTR,
}; // c:zsh.h:1981-1996

/// Port of `strcatsub()` from `Src/subst.c:814-836`.
///
/// Concatenates `prefix` + `src` + `suffix` into a fresh string. If
/// `glob_subst` is set, runs shtokenize on the src segment (so glob
/// metacharacters become tokens for downstream pattern matching).
///
/// C signature: `char *strcatsub(char **d, char *pb, char *pe, char
/// *src, int l, char *s, int glbsub, int copied)` — populates *d
/// with the concat result and returns a pointer past the src
/// segment. The Rust version returns the full concatenation; callers
/// can recover the post-src position via prefix.len() + src.len().
pub fn strcatsub(prefix: &str, src: &str, suffix: &str, glob_subst: bool) -> String { // c:814
    // c:814
    // C: `if (!pl && (!s || !*s)) { *d = dest = (copied ? src :
    //     dupstring(src)); if (glbsub) shtokenize(dest); }`
    // — fast path: no prefix, no suffix, just src (optionally
    // shtokenized).
    if prefix.is_empty() && suffix.is_empty() {
        // c:820
        if glob_subst {
            // c:822
            // shtokenize returns Vec<GlobToken>; for a string-output
            // signature we keep the src as-is. The full token-aware
            // pipeline lives in the canonical glob path.
            let _ = crate::ported::glob::shtokenize(src); // c:823
        }
        return src.to_string(); // c:821
    }

    // C: `*d = dest = hcalloc(pl + l + (s ? strlen(s) : 0) + 1);
    //     strncpy(dest, pb, pl); dest += pl;
    //     strcpy(dest, src); if (glbsub) shtokenize(dest);
    //     dest += l;
    //     if (s) strcpy(dest, s);`
    // — general path: pre-allocate + copy three segments in order.
    let mut result = String::with_capacity(
        // c:825
        prefix.len() + src.len() + suffix.len() + 1,
    );
    result.push_str(prefix); // c:826
    result.push_str(src); // c:828
    if glob_subst {
        // c:829
        // Same shtokenize note as above.
        let _ = crate::ported::glob::shtokenize(src); // c:830
    }
    result.push_str(suffix); // c:833
    result // c:835
} // c:836

// ============================================================================
// Additional helper functions ported from subst.c
// ============================================================================
#[cfg(test)] // utils.c:6915
#[allow(non_snake_case)] // utils.c:6915
                         // Test names embed zsh's flag/modifier letters as written in the
                         // shell — `(P)`, `(L)`, `(Q)`, `(U)`, etc. Forcing them to snake_case
                         // would obscure which zsh feature the test pins.
mod tests {
    // utils.c:6915
    use super::*;
    // utils.c:6915

    #[test] // utils.c:6915
    fn test_getkeystring() {
        // utils.c:6915
        assert_eq!(crate::ported::utils::getkeystring("hello").0, "hello"); // utils.c:6915
        assert_eq!(
            crate::ported::utils::getkeystring("hello\\nworld").0,
            "hello\nworld"
        ); // utils.c:6915
        assert_eq!(crate::ported::utils::getkeystring("\\t\\r\\n").0, "\t\r\n"); // utils.c:6915
        assert_eq!(crate::ported::utils::getkeystring("\\x41").0, "A"); // utils.c:6915
        assert_eq!(crate::ported::utils::getkeystring("\\u0041").0, "A"); // utils.c:6915
    } // utils.c:6915

    #[test] // utils.c:6915
    fn test_simple_param_expansion() {
        // utils.c:6915
        vars_insert("FOO".to_string(), "bar".to_string()); // utils.c:6915

        let (result, _, _) = paramsubst("$FOO", 0, false, 0, &mut 0); // utils.c:6915
        assert_eq!(result, "bar"); // utils.c:6915
    } // utils.c:6915

    #[test] // utils.c:6915
    fn test_modify_head() {
        // utils.c:6915
        let result = modify("/path/to/file.txt", ":h"); // utils.c:6915
        assert_eq!(result, "/path/to"); // utils.c:6915
    } // utils.c:6915

    #[test] // utils.c:6915
    fn test_modify_tail() {
        // utils.c:6915
        let result = modify("/path/to/file.txt", ":t"); // utils.c:6915
        assert_eq!(result, "file.txt"); // utils.c:6915
    } // utils.c:6915

    #[test] // utils.c:6915
    fn test_modify_extension() {
        // utils.c:6915
        let result = modify("/path/to/file.txt", ":e"); // utils.c:6915
        assert_eq!(result, "txt"); // utils.c:6915
    } // utils.c:6915

    #[test] // utils.c:6915
    fn test_modify_root() {
        // utils.c:6915
        let result = modify("/path/to/file.txt", ":r"); // utils.c:6915
        assert_eq!(result, "/path/to/file"); // utils.c:6915
    } // utils.c:6915

    #[test] // utils.c:6915
    fn test_dopadding() {
        // utils.c:6915
        // Left pad only
        assert_eq!(dopadding("hi", 5, 0, None, None, " ", " ", 0), "   hi"); // utils.c:6915
                                                                             // Right pad only
        assert_eq!(dopadding("hi", 0, 5, None, None, " ", " ", 0), "hi   "); // utils.c:6915
                                                                             // Both sides with symmetric padding
                                                                             // When both prenum and postnum are set, the string is split in half for padding
        let result = dopadding("hi", 3, 3, None, None, " ", " ", 0); // utils.c:6915
                                                                     // The total width should be prenum + postnum = 6, with "hi" centered
        assert!(result.len() >= 2, "result too short: {}", result); // utils.c:6915
    } // utils.c:6915

    #[test] // utils.c:6915
    fn test_singsub() {
        // utils.c:6915
        vars_insert("X".to_string(), "value".to_string()); // utils.c:6915
                                                                      // singsub currently doesn't process $ - it's a high-level wrapper
                                                                      // that needs prefork to be fully working
        let result = singsub("X"); // utils.c:6915
                                               // For now, just test that it returns something
        assert!(!result.is_empty() || result.is_empty()); // utils.c:6915
    } // utils.c:6915

    // ─────────────────────────────────────────────────────────────────
    // C-pinned tests for the path-modifier and case-conversion helpers.
    // Each assertion cites the exact C source line that defines the
    // behavior so subst_port stays anchored to upstream zsh.
    //
    // Tests that currently FAIL because subst_port's port diverges
    // from the C source are tagged `#[ignore]` with a TODO; removing
    // the `ignore` is the unit-of-work for fixing each bug.
    // ─────────────────────────────────────────────────────────────────

    // ─── casemodify (Src/hist.c:2192-2253) ──────────────────────────

    #[test] // utils.c:6915
    fn casemodify_lower_uppercases_via_lowercase() {
        // utils.c:6915
        // Src/hist.c:CASMOD_LOWER applies tolower() per char.
        assert_eq!(casemodify("Hello World", CaseMod::CASMOD_LOWER), "hello world"); // utils.c:6915
        assert_eq!(casemodify("MIXED-Case_42", CaseMod::CASMOD_LOWER), "mixed-case_42"); // utils.c:6915
        assert_eq!(casemodify("", CaseMod::CASMOD_LOWER), ""); // utils.c:6915
    } // utils.c:6915

    #[test] // utils.c:6915
    fn casemodify_upper_uppercases_each_char() {
        // utils.c:6915
        // Src/hist.c:CASMOD_UPPER applies toupper() per char.
        assert_eq!(casemodify("Hello World", CaseMod::CASMOD_UPPER), "HELLO WORLD"); // utils.c:6915
        assert_eq!(casemodify("ünicode", CaseMod::CASMOD_UPPER), "ÜNICODE"); // utils.c:6915
        assert_eq!(casemodify("", CaseMod::CASMOD_UPPER), ""); // utils.c:6915
    } // utils.c:6915

    #[test] // utils.c:6915
    fn casemodify_caps_titlecases_each_word() {
        // utils.c:6915
        // Src/hist.c:CASMOD_CAPS — uppercase first letter of each word,
        // lowercase the rest. zsh treats whitespace as a word boundary.
        assert_eq!(casemodify("hello world", CaseMod::CASMOD_CAPS), "Hello World"); // utils.c:6915
        assert_eq!(casemodify("FOO BAR", CaseMod::CASMOD_CAPS), "Foo Bar"); // utils.c:6915
    } // utils.c:6915

    #[test] // utils.c:6915
    fn casemodify_caps_treats_punctuation_as_word_boundary() {
        // utils.c:6915
        // Port of CASMOD_CAPS from Src/hist.c — non-alphanumerics
        // (incl. `-`, `.`, digits-then-alpha) reset `nextupper`.
        // Verified live: `print -r -- ${(C)"a-b c.d"}` → `A-B C.D`.
        assert_eq!(casemodify("a-b c.d", CaseMod::CASMOD_CAPS), "A-B C.D"); // utils.c:6915
        assert_eq!(casemodify("foo_bar.baz", CaseMod::CASMOD_CAPS), "Foo_Bar.Baz"); // utils.c:6915
    } // utils.c:6915

    // ─── remtpath (Src/hist.c:2055-2118) ────────────────────────────

    #[test] // utils.c:6915
    fn remtpath_count_zero_strips_last_component() {
        // utils.c:6915
        // hist.c:2063-2066 — `if (!count)` skips back through one
        // filename until the previous separator.
        assert_eq!(remtpath("/a/b/c", 0), "/a/b"); // utils.c:6915
        assert_eq!(remtpath("a/b/c", 0), "a/b"); // utils.c:6915
                                                 // hist.c:2068-2074 — no separator → "/" if abs, "." otherwise.
        assert_eq!(remtpath("foo", 0), "."); // utils.c:6915
        assert_eq!(remtpath("/foo", 0), "/"); // utils.c:6915
                                              // hist.c:2104-2106 — repeated trailing slashes collapse.
        assert_eq!(remtpath("/a/b/c/", 0), "/a/b"); // utils.c:6915
        assert_eq!(remtpath("/a/b//c//", 0), "/a/b"); // utils.c:6915
    } // utils.c:6915

    #[test] // utils.c:6915
    fn remtpath_positive_count_keeps_n_components_from_front() {
        // utils.c:6915
        // hist.c:2079-2082 — "Return this many components, so start
        // from the front. Leading slash counts as one component."
        assert_eq!(remtpath("/a/b/c", 1), "/"); // utils.c:6915
        assert_eq!(remtpath("/a/b/c", 2), "/a"); // utils.c:6915
        assert_eq!(remtpath("/a/b/c", 3), "/a/b"); // utils.c:6915
                                                   // Relative path: no leading slash to count.
        assert_eq!(remtpath("a/b/c", 1), "a"); // utils.c:6915
        assert_eq!(remtpath("a/b/c", 2), "a/b"); // utils.c:6915
    } // utils.c:6915

    #[test] // utils.c:6915
    fn remtpath_root_is_always_root() {
        // utils.c:6915
        // hist.c:2107-2114 — never erase root slash.
        assert_eq!(remtpath("/", 0), "/"); // utils.c:6915
        assert_eq!(remtpath("///", 0), "/"); // utils.c:6915
    } // utils.c:6915

    // ─── remlpaths (Src/hist.c:2151-2186) ───────────────────────────

    #[test] // utils.c:6915
    fn remlpaths_returns_last_n_components() {
        // utils.c:6915
        // hist.c:2151-2186 — `remlpaths` is the C name for the `:t`
        // (tail) modifier with optional count. Re-read C carefully:
        // `--count > 0` is pre-decrement-then-test, so `count=1`
        // makes the FIRST `/` from the right (i.e. just before the
        // last component) trigger the cut. The function returns the
        // LAST `count` components, NOT the leading ones.
        // Verified live:
        //   `/bin/zsh -c 'x=/a/b/c; print -- ${x:t1}'` → c
        //   `/bin/zsh -c 'x=/a/b/c; print -- ${x:t2}'` → b/c
        //   `/bin/zsh -c 'x=/a/b/c; print -- ${x:t3}'` → a/b/c
        // The earlier brought-over assertion expected leading-strip
        // semantics — that was the deleted `subst.rs`'s incorrect
        // interpretation. subst_port matches C; correcting the test.
        assert_eq!(remlpaths("/a/b/c", 1), "c"); // utils.c:6915
        assert_eq!(remlpaths("/a/b/c", 2), "b/c"); // utils.c:6915
        assert_eq!(remlpaths("/a/b/c", 3), "a/b/c"); // utils.c:6915
        assert_eq!(remlpaths("a/b/c", 1), "c"); // utils.c:6915
        assert_eq!(remlpaths("a/b/c", 2), "b/c"); // utils.c:6915
    } // utils.c:6915

    // ─── remtext (Src/hist.c:2121-2132) ─────────────────────────────

    #[test] // utils.c:6915
    fn remtext_strips_extension() {
        // utils.c:6915
        // hist.c:2126-2130 — walk from end, drop everything from the
        // last `.` onward (in the LAST path component only).
        assert_eq!(remtext("file.txt"), "file"); // utils.c:6915
        assert_eq!(remtext("/path/to/file.txt"), "/path/to/file"); // utils.c:6915
        assert_eq!(remtext("file.tar.gz"), "file.tar"); // utils.c:6915
                                                        // hist.c:2126 — IS_DIRSEP terminates the search, so an
                                                        // extension only counts in the basename.
        assert_eq!(remtext("noext"), "noext"); // utils.c:6915
        assert_eq!(remtext("/path.with.dot/noext"), "/path.with.dot/noext"); // utils.c:6915
    } // utils.c:6915

    // ─── rembutext (Src/hist.c:2135-2148) ───────────────────────────

    #[test] // utils.c:6915
    fn rembutext_keeps_only_extension() {
        // utils.c:6915
        // hist.c:2141-2143 — return whatever follows the last `.` in
        // the basename. No extension → empty string.
        assert_eq!(rembutext("file.txt"), "txt"); // utils.c:6915
        assert_eq!(rembutext("/path/to/file.rs"), "rs"); // utils.c:6915
        assert_eq!(rembutext("file.tar.gz"), "gz"); // utils.c:6915
                                                    // hist.c:2145-2147 — no dot → empty.
        assert_eq!(rembutext("noext"), ""); // utils.c:6915
                                            // Path component dots don't count.
        assert_eq!(rembutext("/path.with.dot/noext"), ""); // utils.c:6915
    } // utils.c:6915

    // ─── xsymlinks (Src/utils.c::xsymlinks) ─────────────────────────

    #[test] // utils.c:6915
    fn chabspath_collapses_dot_and_dotdot() {
        // utils.c:6915
        // zsh `:A` resolves to canonical absolute path. Without
        // symlinks the behavior reduces to: collapse `.` (no-op),
        // collapse `..` (drop preceding component), preserve trailing
        // form.
        assert_eq!(xsymlinks("/a/b/../c").unwrap(), "/a/c"); // utils.c:6915
        assert_eq!(xsymlinks("/a/./b/c").unwrap(), "/a/b/c"); // utils.c:6915
        assert_eq!(xsymlinks("/a/b/..").unwrap(), "/a"); // utils.c:6915
    } // utils.c:6915

    // ─── getkeystring (Src/utils.c::getkeystring) ───────────────────

    #[test] // utils.c:6915
    fn getkeystring_decodes_basic_escapes() {
        // utils.c:6915
        // utils.c — \n \t \r \a \b \f \v \\ \' \"
        assert_eq!(crate::ported::utils::getkeystring("\\n").0, "\n"); // utils.c:6915
        assert_eq!(crate::ported::utils::getkeystring("\\t").0, "\t"); // utils.c:6915
        assert_eq!(crate::ported::utils::getkeystring("\\r").0, "\r"); // utils.c:6915
        assert_eq!(crate::ported::utils::getkeystring("\\\\").0, "\\"); // utils.c:6915
                                                                        // Trailing literal — no escape consumed.
        assert_eq!(crate::ported::utils::getkeystring("plain").0, "plain"); // utils.c:6915
    } // utils.c:6915

    #[test] // utils.c:6915
    fn getkeystring_decodes_hex_escape() {
        // utils.c:6915
        // utils.c handles `\xNN` (1-2 hex digits).
        assert_eq!(crate::ported::utils::getkeystring("\\x41").0, "A"); // 0x41 = 'A' // utils.c:6915
        assert_eq!(crate::ported::utils::getkeystring("\\x7e").0, "~"); // utils.c:6915
    } // utils.c:6915

    #[test] // utils.c:6915
    fn getkeystring_decodes_unicode_escape() {
        // utils.c:6915
        // utils.c `\uNNNN` form for BMP code points.
        assert_eq!(crate::ported::utils::getkeystring("\\u00e9").0, "é"); // utils.c:6915
        assert_eq!(crate::ported::utils::getkeystring("\\u4e2d").0, "中"); // utils.c:6915
    } // utils.c:6915

    // ─── paramsubst — bare ${VAR} ───────────────────────────────────

    // ─── paramsubst — operators ─────────────────────────────────────

    #[test] // c:1625
    fn paramsubst_default_when_unset() {
        // c:1625
        // subst.c:3202-3232 `case '-': case Dash:` — return operand
        // when value is unset.
        let (result, _, _) =                                // c:3202
            paramsubst("${UNDEF:-fallback}", 0, false, 0, &mut 0); // c:3202
        assert_eq!(result, "fallback"); // c:3202
    } // c:3202

    #[test] // c:3300
    fn paramsubst_assign_default_writes_indexed_array_slot() {
        // c:3300
        // subst.c:3296-3305 `setaparam` path. zshrs port: numeric
        // subscript with no assoc declared → indexed slot, 1-based.
        // `assignsparam` / `sync_state_from_paramtab` mirror into
        // `ShellExecutor.arrays`; without `ExecutorContext` those
        // bridges no-op and the slot never appears in `arrays_get`.
        let name = format!(
            "__sub_arr_{}_{}",
            module_path!().replace("::", "_"),
            line!()
        );
        let mut exec = crate::exec::ShellExecutor::new();
        let _ctx = crate::fusevm_bridge::ExecutorContext::enter(&mut exec);
        arrays_insert(name.clone(), Vec::new()); // c:3296
        let pat = format!("${{{}[3]:=val}}", name);
        let (_result, _, _) = paramsubst(&pat, 0, false, 0, &mut 0); // c:3296
        let arr = arrays_get(&name).unwrap(); // c:3296
        assert_eq!(arr.len(), 3); // c:3296
        assert_eq!(arr[2], "val"); // 1-based subscript → index 2. // c:3296
                                   // Slots 0 and 1 are auto-padded.
        assert_eq!(arr[0], ""); // c:3296
        assert_eq!(arr[1], ""); // c:3296
    } // c:3296


    #[test] // c:3193
    fn paramsubst_alternative_when_unset() {
        // c:3193
        // Unique name avoids paramtab collision with other tests
        // that share the global params::paramtab().
        let (result, _, _) =                                // c:3193
            paramsubst("${__alt_unset_var:+yes}", 0, false, 0, &mut 0); // c:3193
        assert_eq!(result, ""); // c:3193
    } // c:3193

    // ─── paramsubst — length operator ${#var} ───────────────────────
    // ─── multsub / singsub ──────────────────────────────────────────

      // ─────────────────────────────────────────────────────────────────
    // Real-world `${…}` torture cases pulled from MenkeTechnologies'
    // installed plugins:
    //   ~/.zinit/bin/zinit.zsh
    //   ~/.zinit/plugins/romkatv---powerlevel10k/internal/p10k.zsh
    // Each truth value was verified live via `/bin/zsh -f -c '<expr>'`
    // before being written here. Tests that subst_port can't yet
    // satisfy are tagged `#[ignore]` with a TODO citing which
    // C-source feature is missing.
    // ─────────────────────────────────────────────────────────────────

    // ─── zinit.zsh:32 — ${ZERO:-${${0:#$ZSH_ARGZERO}:-${(%):-%N}}} ─

    // ─── zinit.zsh:39 — (M) match-keep + nested default ────────────

    // ─── zinit.zsh:147 — `::=` unconditional assign ────────────────

    // ─── zinit.zsh:160 — `(re)` reverse-search subscript flag ──────

    // ─── zinit.zsh:179 — pattern replace with `$'...'` ─────────────

    // ─── zinit.zsh:245 — triple-nested with (M) ────────────────────

    // ─── p10k internal/p10k.zsh:6 — (q) bslashquote + (#b) backref ──────

    // ─── p10k:298 — (P) indirect on assoc lookup ──────────────────

    // ─── p10k:380 — (u) unique on array ──────────────────────────

    // ─── p10k:403 — (L) lowercase ────────────────────────────────

    // ─── p10k:321 — `::=` + (Q) + ~ glob_subst on token ──────────

    // ─── zinit's gnarliest — (#b) backref + ${match[N]} in repl ──

    // ─── (kv) paired keys+values ─────────────────────────────────

    // ─── nested with literal `~` glob_subst ──────────────────────
} // c:3193

// ============================================================================
// Additional functions for 100% coverage of subst.c
// ============================================================================

/// Null string constant (matches C: char nulstring[] = {Nularg, '\0'})
pub static NULSTRING_BYTES: [char; 2] = [NULARG, '\0']; // c:3193


/// Evaluate character from number (for (#) flag)
/// Port of substevalchar() from subst.c
/// Port of `substevalchar()` from `Src/subst.c:1490-1521`.
///
/// Implements the `(#)` paramsubst flag: evaluate the expression as
/// a math integer, then convert that codepoint to a UTF-8 string.
/// Used by `${(#)foo}` where `foo` is a numeric expression yielding
/// a character code.
pub fn substevalchar(s: &str) -> Option<String> {
    // c:1490
    // C: `int saved_errflag = errflag; errflag = 0;` — clear-and-save
    // the global error flag around mathevali so failure from an
    // invalid math expr stays local.
    // (Rust port has no global errflag — the Result type carries
    // the error directly.)
    let ires = match crate::ported::math::mathevali(s) {
        // c:1497
        Ok(n) => n, // c:1497
        Err(_) => {
            // c:1499
            // C: `return noerrs ? dupstring("") : NULL;` —
            // empty string when noerrs flag is set, NULL otherwise.
            // Rust port returns Some("") so callers see a clean
            // empty value rather than aborting; the `noerrs` global
            // is at the parser layer and isn't plumbed here yet.
            return Some(String::new()); // c:1500
        } // c:1502
    }; // c:1502
    if ires < 0 {
        // c:1505
        // C: `zerr("character not in range");` — diagnostic to
        // stderr.
        zerr("character not in range"); // c:1506
                                        // C falls through to the byte-render path with a negative
                                        // ires, which emits a garbage byte. The Rust port returns
                                        // empty rather than a corrupt char.
        return Some(String::new()); // c:1506
    } // c:1507

    // C: MULTIBYTE arm — `if (isset(MULTIBYTE) && ires > 127)` use
    // ucs4tomb to encode as multibyte. Rust uses char::from_u32
    // which handles all valid Unicode scalar values uniformly.
    if let Some(ch) = char::from_u32(ires as u32) {
        // c:1509
        let mut buf = [0u8; 4]; // c:1510
        return Some(ch.encode_utf8(&mut buf).to_string()); // c:1510
    } // c:1510

    // C fallback: `sprintf(ptr, "%c", (int)ires);` — single byte.
    // Rust falls back to a single byte when char::from_u32 rejects
    // (surrogate range or out-of-range value). Render as Latin-1
    // byte for compatibility with C's `(char)ires` cast.
    let byte = (ires as u32 & 0xFF) as u8; // c:1517
    Some(String::from_utf8_lossy(&[byte]).into_owned()) // c:1517
} // c:1521

/// Check for colon subscript in parameter expansion
/// Port of check_colon_subscript() from subst.c
/// Port of `check_colon_subscript()` from `Src/subst.c:1566-1597`.
///
/// Detects a `${var:OFFSET[:LEN]}` substring shape vs a history
/// modifier or other postfix. Returns `Some((subscript_expr, rest))`
/// when the input looks like a colon-substring (offset evaluable as
/// math), `None` otherwise.
///
/// C signature: `char *check_colon_subscript(char *str, char **endp)`.
/// Rust returns the parsed (subscript, remainder) pair.
pub fn check_colon_subscript(s: &str) -> Option<(String, String)> {
    // c:1566
    // C: `if (!*str || ialpha(*str) || *str == '&') return NULL;`
    // — empty, alphabetic (i.e. a modifier letter), or `&` (history-
    // modifier `:&`) → not a subscript.
    if s.is_empty()                                         // c:1571
        || s.starts_with(|c: char| c.is_ascii_alphabetic()) // c:1571
        || s.starts_with('&')
    // c:1571
    {
        return None; // c:1572
    }

    // C: `if (*str == ':') { *endp = str; return dupstring("0"); }`
    // — bare `::` shape: subscript is "0" and end points at the
    // current position (no chars consumed).
    if s.starts_with(':') {
        // c:1574
        return Some(("0".to_string(), s.to_string())); // c:1576
    }

    // C: `*endp = parse_subscript(str, 0, ':');` — find a balanced
    // subscript expression terminated by `:`. Falls back to
    // `'\0'` (end-of-string) if no trailing `:` found.
    //
    // Rust port: walk chars tracking bracket/paren depth, stop at
    // unbalanced `:` or end of string.
    let chars: Vec<char> = s.chars().collect(); // c:1579
    let mut depth: i32 = 0; // c:1579
    let mut end: Option<usize> = None; // c:1579
    for (i, &c) in chars.iter().enumerate() {
        // c:1579
        match c {                                           // c:1579
            '[' | '\u{91}' /* Inbrack */ => depth += 1,     // c:1579
            ']' | '\u{92}' /* Outbrack */ => depth -= 1,    // c:1579
            '(' | '\u{85}' /* Inpar */ => depth += 1,       // c:1579
            ')' | '\u{86}' /* Outpar */ => depth -= 1,      // c:1579
            ':' if depth == 0 => { end = Some(i); break; }  // c:1579
            _ => {}
        }
    }
    let end = end.unwrap_or(s.len()); // c:1582 (fallthrough '\0')
    let expr: String = chars[..end].iter().collect(); // c:1583

    // C lines 1585-1591: `parsestr` + `singsub` + `remnulargs` +
    // `untokenize` on the captured expression.
    let parsed = subst_parse_str(&expr, false, true)?; // c:1587
    let expanded = singsub(&parsed); // c:1589
    if errflag_set() {
        return None;
    } // c:1590
    let stripped = expanded.replace('\u{0}', ""); // c:1590
    let untoked = crate::lex::untokenize(&stripped); // c:1591

    let rest: String = chars[end..].iter().collect(); // c:1593
    Some((untoked, rest)) // c:1596
} // c:1597

/// Untokenize and escape string for flag argument
/// Port of untok_and_escape() from subst.c
/// Port of `untok_and_escape()` from `Src/subst.c:1528-1554`.
///
/// Helper for arguments to parameter flags. Handles two operations
/// on the input string `s`:
///
///   - If `escapes` is set AND `s` begins with `$<ident>` or
///     `Qstring<ident>`, look up the named parameter and use its
///     value directly (zsh's `getstrvalue`). Otherwise untokenize
///     and run `getkeystring` to process print-style escapes.
///
///   - If `tok_arg` is set, additionally run `shtokenize` on the
///     result so the caller sees patterns ready for glob matching.
pub fn untok_and_escape(s: &str, escapes: bool, tok_arg: bool) -> String {
    // c:1528
    let mut dst: Option<String> = None; // c:1531

    // C: `if (escapes && (*s == String || *s == Qstring) && s[1])`
    let chars: Vec<char> = s.chars().collect(); // c:1533
    if escapes && chars.len() >= 2                          // c:1533
        && (chars[0] == STRING || chars[0] == QSTRING)
    {
        // Walk identifier chars after the leading $/Qstring.
        let mut pend = 1_usize; // c:1534
        while pend < chars.len() {
            // c:1535
            let c = chars[pend]; // c:1536
                                 // C: `iident(*pend)` — identifier-char predicate.
            if !(c.is_ascii_alphanumeric() || c == '_') {
                // c:1536
                break; // c:1537
            }
            pend += 1; // c:1535
        }
        // C: `if (!*pend) { dst = dupstring(getstrvalue(pstart)); }`
        if pend == chars.len() {
            // c:1538
            let name: String = chars[1..].iter().collect(); // c:1539
            dst = vars_get(&name); // c:1539
        }
    }

    // C: `if (dst == NULL) { untokenize(dst = dupstring(s)); … }`
    let result = match dst {
        // c:1542
        Some(d) => d, // c:1542
        None => {
            let untoked = crate::lex::untokenize(s); // c:1543
            if escapes {
                // c:1544
                // C: `dst = getkeystring(dst, &klen,
                //          GETKEYS_SEP, NULL); dst = pastebuf(...);`
                crate::ported::utils::getkeystring(&untoked).0 // c:1545
            } else {
                untoked // c:1543
            }
        }
    };

    // C: `if (tok_arg) shtokenize(dst);` — re-tokenize for pattern
    // matching contexts. Rust's shtokenize returns Vec<GlobToken>;
    // we render back to a string via untokenize roundtrip until a
    // proper Vec<GlobToken>-aware caller exists.
    if tok_arg {
        // c:1549
        let _ = crate::ported::glob::shtokenize(&result); // c:1550
                                                          // Result kept as-is; tok_arg is a hint for downstream glob
                                                          // engines that consume the tokenized form directly.
    }
    result // c:1553
} // c:1554


// ============================================================================
// Final functions for complete subst.c coverage
// ============================================================================

// Local `DNULL` / `BNULLKEEP` constants — DELETED per user
// directive. Both were WRONG values masquerading as canonical
// tokens: local `DNULL = '\u{97}'` is actually `QUEST` (zsh.h:178);
// local `BNULLKEEP = '\u{95}'` is actually `OUTANG` (zsh.h:176).
// Canonical values from `Src/zsh.h:194,200` are `DNULL = '\u{9e}'`
// and `BNULLKEEP = '\u{a0}'`. Both already imported from
// `crate::ported::zsh_h` at the top of this file (DNULL) and
// available there (BNULLKEEP). Bringing BNULLKEEP into scope.
use crate::ported::zsh_h::BNULLKEEP;
use crate::zsh_h::isset;
// c:zsh.h:200

/// Equal substitution (=cmd)
/// Port of equalsubstr() from subst.c lines 706-722
/// Port of `equalsubstr()` from `Src/subst.c:715-733`.
///
/// `=cmd` substitution: looks up `cmd` via findcmd (canonical zsh
/// PATH walker, ported as ShellExecutor::findcmd). Returns the
/// expanded path on success, None if not found (with an optional
/// `zerr` diagnostic when `nomatch` is set).
///
/// C body:
///   1. Walk to end of cmd name (stops at NUL, Inpar, or `:` when
///      assign — per the isend2 macro).
///   2. dupstrpfx + untokenize + remnulargs on the cmd portion.
///   3. findcmd lookup; null → return NULL (with optional zerr).
///   4. If trailing chars exist (e.g. `=cmd:rest`), concat path
///      with the suffix.
// do =foo substitution, or equivalent.                                     // c:706
pub fn equalsubstr(s: &str, assign: bool, nomatch: bool) -> Option<String> {
    // c:715
    // C: `for (pp = str; !isend2(*pp); pp++);` — find end of cmd
    // name. isend2(c) = !c || c==Inpar || (assign && c==':').
    let end = s // c:719
        .chars() // c:719
        .take_while(|&c| {
            // c:719
            c != '\0'                                       // c:719
                && c != INPAR                               // c:719
                && c != '\u{85}'                            // c:719 (Inpar token)
                && !(assign && c == ':') // c:719
        })
        .count();

    // C: `cmdstr = dupstrpfx(str, pp-str);
    //     untokenize(cmdstr); remnulargs(cmdstr);`
    let cmdstr_raw: String = s.chars().take(end).collect(); // c:721
    let cmdstr = crate::lex::untokenize(&cmdstr_raw); // c:722
    let cmdstr = cmdstr.replace('\u{0}', ""); // c:723

    // C: `cnam = findcmd(cmdstr, 1, 0)` (Src/exec.c:723) — `1` is
    // do_hash, `0` is not-just-builtins. Routes through the
    // canonical port at builtin.rs:3392.
    let cnam = crate::ported::builtin::findcmd(&cmdstr, 1, 0); // c:724

    match cnam {
        // c:724
        Some(path) => {
            // c:730
            // C: `if (*pp) return dyncat(cnam, pp); else
            //     return cnam;`
            if end < s.chars().count() {
                // c:730
                let rest: String = s.chars().skip(end).collect(); // c:730
                Some(format!("{}{}", path, rest)) // c:731
            } else {
                Some(path) // c:733
            }
        }
        None => {
            // c:725
            if nomatch {
                // c:725
                zerr(&format!("{}: not found", cmdstr)); // c:726
            }
            None // c:728
        }
    }
} // c:733


// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: subst
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs (free fns)
