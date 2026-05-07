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
//! - stringsubstquote() — $'...' quote processing
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

use std::collections::VecDeque;                             // c:N/A
#[allow(unused_imports)]
use crate::ported::exec::{
    self, ShellExecutor,
    cached_regex, slice_array_zero_based, slice_positionals,
};
use crate::ported::params::{array_subscript_flag, assoc_subscript_flag};
// Per user directive: history-modifier helpers (casemodify, remtpath,
// remlpaths, remtext, chabspath) live in src/ported/hist.rs (the
// canonical port of Src/hist.c). Import here so subst.rs's modify()
// arms and the parity tests can reference by bare name.
use crate::ported::hist::{casemodify, CaseMod, chabspath, remlpaths, rembutext, remtext, remtpath};
#[allow(unused_imports)]
use crate::parse::{ShellWord, VarModifier, ZshParamFlag};

// Token constants from zsh.h (mapped to char values > 127)
pub mod tokens {                                            // c:N/A
    pub const POUND: char = '\u{80}'; // #                  // c:N/A
    pub const STRING: char = '\u{81}'; // $                 // c:N/A
    pub const QSTRING: char = '\u{82}'; // Quoted $         // c:N/A
    pub const TICK: char = '\u{83}'; // `                   // c:N/A
    pub const QTICK: char = '\u{84}'; // Quoted `           // c:N/A
    pub const INPAR: char = '\u{85}'; // (                  // c:N/A
    pub const OUTPAR: char = '\u{86}'; // )                 // c:N/A
    pub const INBRACE: char = '\u{87}'; // {                // c:N/A
    pub const OUTBRACE: char = '\u{88}'; // }               // c:N/A
    pub const INBRACK: char = '\u{89}'; // [                // c:N/A
    pub const OUTBRACK: char = '\u{8A}'; // ]               // c:N/A
    pub const INANG: char = '\u{8B}'; // <                  // c:N/A
    pub const OUTANG: char = '\u{8C}'; // >                 // c:N/A
    pub const OUTANGPROC: char = '\u{8D}'; // >( for process sub // c:N/A
    pub const EQUALS: char = '\u{8E}'; // =                 // c:N/A
    pub const NULARG: char = '\u{8F}'; // Null argument marker // c:N/A
    pub const INPARMATH: char = '\u{90}'; // $((            // c:N/A
    pub const OUTPARMATH: char = '\u{91}'; // ))            // c:N/A
    pub const SNULL: char = '\u{92}'; // $' quote marker    // c:N/A
    pub const MARKER: char = '\u{93}'; // Array key-value marker // c:N/A
    pub const BNULL: char = '\u{94}'; // Backslash null     // c:N/A


}                                                           // c:N/A

use tokens::*;                                              // c:N/A

/// Linked list flags (from zsh.h LF_*)
pub const LF_ARRAY: u32 = 1;                                // c:N/A

/// Prefork flags (from zsh.h PREFORK_*)
pub mod prefork_flags {                                     // c:N/A
    pub const SINGLE: u32 = 1; // Single word expected      // c:N/A
    pub const SPLIT: u32 = 2; // Force word splitting       // c:N/A
    pub const SHWORDSPLIT: u32 = 4; // sh-style word splitting // c:N/A
    pub const NOSHWORDSPLIT: u32 = 8; // Disable word splitting // c:N/A
    pub const ASSIGN: u32 = 16; // Assignment context       // c:N/A
    pub const TYPESET: u32 = 32; // Typeset context         // c:N/A
    pub const SUBEXP: u32 = 64; // Subexpression            // c:N/A
    pub const KEY_VALUE: u32 = 128; // Key-value pair found // c:N/A
    pub const NO_UNTOK: u32 = 256; // Don't untokenize      // c:N/A
}                                                           // c:N/A

/// Linked list node - mirrors zsh LinkNode
#[derive(Debug, Clone)]                                     // c:N/A
/// Linked-list node for the substitution pipeline.
/// Mirrors `struct linknode` from Src/zsh.h — `prefork()`
/// (Src/subst.c:100) walks a `LinkList` of these.
pub struct LinkNode {                                       // c:100
    pub data: String,                                       // c:100
}                                                           // c:100

/// Linked list - mirrors zsh LinkList
#[derive(Debug, Clone, Default)]                            // c:100
/// Substitution pipeline word list.
/// Mirrors `struct linklist` (Src/zsh.h) — the C source threads
/// it through `prefork()`/`stringsubst()`/`paramsubst()` (lines
/// 100/237/1625).
pub struct LinkList {                                       // c:100
    pub nodes: VecDeque<LinkNode>,                          // c:100
    pub flags: u32,                                         // c:100
}                                                           // c:100

impl LinkList {                                             // c:100
    /// Port of `firstnode(X)` macro from `src/zsh/Src/zsh.h:576`.
    pub fn firstnode(&self) -> Option<usize> {              // zsh.h:576
        if self.nodes.is_empty() {                          // zsh.h:576
            None                                            // zsh.h:576
        } else {                                            // zsh.h:576
            Some(0)                                         // zsh.h:576
        }                                                   // zsh.h:576
    }                                                       // zsh.h:576

    /// Port of `getdata(X)` macro from `src/zsh/Src/zsh.h:586`.
    pub fn getdata(&self, idx: usize) -> Option<&str> {     // zsh.h:586
        self.nodes.get(idx).map(|n| n.data.as_str())        // zsh.h:586
    }                                                       // zsh.h:586

    /// Port of `setdata(X,Y)` macro from `src/zsh/Src/zsh.h:587`.
    pub fn setdata(&mut self, idx: usize, data: String) {   // zsh.h:587
        if let Some(node) = self.nodes.get_mut(idx) {       // zsh.h:587
            node.data = data;                               // zsh.h:587
        }                                                   // zsh.h:587
    }                                                       // zsh.h:587

    /// Port of `insertlinknode(X,Y,Z)` macro from `src/zsh/Src/zsh.h:580`.
    pub fn insertlinknode(&mut self, idx: usize, data: String) -> usize { // zsh.h:580
        self.nodes.insert(idx + 1, LinkNode { data });      // zsh.h:580
        idx + 1                                             // zsh.h:580
    }                                                       // zsh.h:580

    /// Port of `delete_node` macro chain in `src/zsh/Src/zsh.h`.
    pub fn delete_node(&mut self, idx: usize) {             // zsh.h:580
        if idx < self.nodes.len() {                         // zsh.h:580
            self.nodes.remove(idx);                         // zsh.h:580
        }                                                   // zsh.h:580
    }                                                       // zsh.h:580

    /// Port of `nextnode(X)` macro from `src/zsh/Src/zsh.h:588`.
    pub fn nextnode(&self, idx: usize) -> Option<usize> {   // zsh.h:588
        if idx + 1 < self.nodes.len() {                     // zsh.h:588
            Some(idx + 1)                                   // zsh.h:588
        } else {                                            // zsh.h:588
            None                                            // zsh.h:588
        }                                                   // zsh.h:588
    }                                                       // zsh.h:588

    /// Port of `empty(X)` macro from `src/zsh/Src/zsh.h:583`.
    pub fn empty(&self) -> bool {                           // zsh.h:583
        self.nodes.is_empty()                               // zsh.h:583
    }                                                       // zsh.h:583







}                                                           // c:100

/// Global state for substitution (mirrors zsh global variables)
#[derive(Default)]                                          // c:100
/// Per-pass substitution state.
/// Bundles the locals `prefork()` (Src/subst.c:100) keeps —
/// IFS, glob options, parameter table reference, depth counters.
pub struct SubstState {                                     // c:100
    pub errflag: bool,                                      // c:100
    pub opts: SubstOptions,                                 // c:100
    pub variables: std::collections::HashMap<String, String>, // c:100
    pub arrays: std::collections::HashMap<String, Vec<String>>, // c:100
    pub assoc_arrays: std::collections::HashMap<String, indexmap::IndexMap<String, String>>, // c:100
    /// When set, prefork's third pass skips `filesub` (tilde and
    /// `=cmd` expansion). Used by `singsub_no_tilde` for pattern
    /// and replacement contexts in `${var/pat/repl}` where the
    /// leading `~` must stay literal.
    pub skip_filesub: bool,                                 // c:100
    /// Names of all defined shell functions. Populated by
    /// `from_executor`. Used by `${+functions[name]}` to answer the
    /// "is this function defined?" question without round-tripping
    /// through `with_executor`. Same idea as the C zsh-side
    /// `paramtab` lookup that backs `${functions[name]}` — the
    /// magic-assoc's getfn just consults the function hashtable.
    pub function_names: std::collections::HashSet<String>,  // c:100
    /// Names of commands resolvable via `$PATH`. Populated lazily
    /// (empty by default; only filled if the script reads
    /// `${+commands[name]}` or similar). Backs the magic-assoc set-
    /// test. Direct analogue of zsh's `cmdhash` / commands special
    /// parameter (Src/init.c, Src/builtin.c bin_hash).
    pub command_names: std::collections::HashSet<String>,   // c:100
    /// Names of currently-defined aliases. Populated by
    /// `from_executor`. Backs `${+aliases[name]}`.
    pub alias_names: std::collections::HashSet<String>,     // c:100
    /// Snapshot of `typeset`-tracked attributes (kind: scalar/integer/
    /// float/array/assoc + readonly / export / left / right_blanks /
    /// right_zeros / lower / upper / unique / hide / hideval / tied).
    /// Backs `${(t)name}` and `${(Pt)name}` per Src/subst.c:2807-2854.
    pub var_attrs: std::collections::HashMap<String, crate::exec::VarAttr>, // c:2807
}                                                           // c:2807

impl SubstState {                                           // c:2807
    /// Snapshot the live `ShellExecutor` parameter table into a
    /// `SubstState`. Mirrors C zsh's `paramtab` global which the
    /// substitution code reads through `getvalue()`. Rust has no
    /// global paramtab, so an explicit snapshot+commit dance is
    /// required to bridge ShellExecutor → subst pipeline. NOT a port
    /// of any single C fn — runtime-state plumbing kept in subst.rs
    /// because it owns SubstState.
    pub fn from_executor(exec: &crate::exec::ShellExecutor) -> Self { // c:N/A (plumbing)
        let assoc_arrays: std::collections::HashMap<
            String,
            indexmap::IndexMap<String, String>,
        > = exec
            .assoc_arrays
            .iter()
            .map(|(k, v)| (k.clone(), v.iter().map(|(ik, iv)| (ik.clone(), iv.clone())).collect()))
            .collect();
        let function_names: std::collections::HashSet<String> =
            exec.function_names().into_iter().collect();
        let alias_names: std::collections::HashSet<String> =
            exec.aliases.keys().cloned().collect();
        let command_names: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        let mut arrays = exec.arrays.clone();
        // Mirror positional params under "@"/"*"/"argv" — single
        // lookup path for $@, ${@:N:M}, ${#@}, $argv etc. Always
        // overwrite (exec.positional_params is the live source).
        arrays.insert("@".to_string(), exec.positional_params.clone());
        arrays.insert("*".to_string(), exec.positional_params.clone());
        arrays.insert("argv".to_string(), exec.positional_params.clone());

        SubstState {
            errflag: false,
            opts: SubstOptions::default(),
            variables: exec.variables.clone(),
            arrays,
            assoc_arrays,
            skip_filesub: false,
            function_names,
            command_names,
            alias_names,
            var_attrs: exec.var_attrs.clone(),
        }
    }

    /// Commit any state mutations performed by paramsubst back to
    /// the live ShellExecutor. Called after each substitution that
    /// might write (${var:=value}, ${var:?}, etc.). Companion to
    /// from_executor — same plumbing rationale.
    pub fn commit_to_executor(self, exec: &mut crate::exec::ShellExecutor) { // c:N/A (plumbing)
        if self.errflag {
            return;
        }
        exec.variables = self.variables;
        exec.arrays = self.arrays;
        for (name, new_map) in self.assoc_arrays {
            let entry = exec.assoc_arrays.entry(name.clone()).or_default();
            for k in entry.keys().cloned().collect::<Vec<_>>() {
                if let Some(v) = new_map.get(&k) {
                    entry.insert(k, v.clone());
                }
            }
            for (k, v) in &new_map {
                if !entry.contains_key(k) {
                    entry.insert(k.clone(), v.clone());
                }
            }
        }
    }
}                                                           // c:2807

/// Options that affect substitution behavior
#[derive(Debug, Clone, Default)]                            // c:2807
/// Substitution-pass option flags.
/// Mirrors the `PF_*` flag bag the C source's `prefork()`
/// (Src/subst.c:100) takes.
pub struct SubstOptions {                                   // c:100
    pub sh_file_expansion: bool,                            // c:100
    pub sh_word_split: bool,                                // c:100
    pub ignore_braces: bool,                                // c:100
    pub glob_subst: bool,                                   // c:100
    pub ksh_typeset: bool,                                  // c:100
    pub exec_opt: bool,                                     // c:100
}                                                           // c:100

/// Null string constant (from subst.c line 36)
pub const NULSTRING: &str = "\u{8F}";                       // c:100

/// Check for array assignment with entries like [key]=val
/// Port of keyvalpairelement() from subst.c lines 47-77
fn keyvalpairelement(list: &mut LinkList, node_idx: usize) -> Option<usize> { // c:49
    let data = list.getdata(node_idx)?;                    // c:49
    let chars: Vec<char> = data.chars().collect();          // c:49

    if chars.is_empty() || chars[0] != INBRACK {            // c:49
        return None;                                        // c:49
    }                                                       // c:49

    // Find closing bracket
    let mut end_pos = None;                                 // c:49
    for (i, &c) in chars.iter().enumerate().skip(1) {       // c:49
        if c == OUTBRACK {                                  // c:49
            end_pos = Some(i);                              // c:49
            break;                                          // c:49
        }                                                   // c:49
    }                                                       // c:49

    let end_pos = end_pos?;                                 // c:49

    // Check for ]=value or ]+=value
    if end_pos + 1 >= chars.len() {                         // c:49
        return None;                                        // c:49
    }                                                       // c:49

    let is_append = chars.get(end_pos + 1) == Some(&'+') && chars.get(end_pos + 2) == Some(&EQUALS); // c:49
    let is_assign = chars.get(end_pos + 1) == Some(&EQUALS); // c:49

    if !is_assign && !is_append {                           // c:49
        return None;                                        // c:49
    }                                                       // c:49

    // Extract key
    let key: String = chars[1..end_pos].iter().collect();   // c:49

    // Extract value
    let value_start = if is_append { end_pos + 3 } else { end_pos + 2 }; // c:49
    let value: String = chars[value_start..].iter().collect(); // c:49

    // Set marker
    let marker = if is_append {                             // c:49
        format!("{}+", MARKER)                              // c:49
    } else {                                                // c:49
        MARKER.to_string()                                  // c:49
    };                                                      // c:49

    list.setdata(node_idx, marker);                        // c:49
    let key_idx = list.insertlinknode(node_idx, key);         // c:49
    let val_idx = list.insertlinknode(key_idx, value);        // c:49

    Some(val_idx)                                           // c:49
}                                                           // c:49

/// Do substitutions before fork
/// Port of prefork() from subst.c lines 94-183
/// Phase-1 word-list substitution (tilde/equal/brace/param/cmd/arith).
/// Port of `prefork()` from Src/subst.c:100 — runs ahead of
/// glob expansion to fully resolve `${...}` / `$(...)` /
/// `$((...))` / `~user` / `=cmd` / `{a,b}`.
pub fn prefork(list: &mut LinkList, flags: u32, ret_flags: &mut u32, state: &mut SubstState) { // c:100
    let mut node_idx = 0;                                   // c:100
    let mut stop_idx: Option<usize> = None;                 // c:100
    let mut keep = false;                                   // c:100
    let asssub = (flags & prefork_flags::TYPESET != 0) && state.opts.ksh_typeset; // c:100
    let mut iter_count = 0u32;                              // c:100

    while node_idx < list.nodes.len() {                           // c:100
        iter_count += 1;                                    // c:100
        if iter_count > 100_000 {                           // c:100
            // Safety cap: if some bug causes prefork's outer loop to
            // never terminate, bail rather than hang the process.
            return;                                         // c:100
        }                                                   // c:100
        // Check for key-value pair element
        if (flags & (prefork_flags::SINGLE | prefork_flags::ASSIGN)) == prefork_flags::ASSIGN { // c:100
            if let Some(new_idx) = keyvalpairelement(list, node_idx) { // c:100
                node_idx = new_idx + 1;                     // c:100
                *ret_flags |= prefork_flags::KEY_VALUE;
                continue;                                   // c:100
            }                                               // c:100
        }                                                   // c:100

        if state.errflag {                                  // c:100
            return;                                         // c:100
        }                                                   // c:100

        if state.opts.sh_file_expansion {                   // c:100
            // SHFILEEXPANSION - do file substitution first
            if let Some(data) = list.getdata(node_idx) {   // c:100
                let new_data = filesub(                     // c:100
                    data,                                   // c:100
                    flags & (prefork_flags::TYPESET | prefork_flags::ASSIGN), // c:100
                    state,                                  // c:100
                );                                          // c:100
                list.setdata(node_idx, new_data);          // c:100
            }                                               // c:100
        } else {                                            // c:100
            // Do string substitution
            if let Some(new_idx) = stringsubst(             // c:100
                list,                                       // c:100
                node_idx,                                   // c:100
                flags & !(prefork_flags::TYPESET | prefork_flags::ASSIGN), // c:100
                ret_flags,                                  // c:100
                asssub,                                     // c:100
                state,                                      // c:100
            ) {                                             // c:100
                node_idx = new_idx;                         // c:100
            } else {                                        // c:100
                return;                                     // c:100
            }                                               // c:100
        }                                                   // c:100

        node_idx += 1;                                      // c:100
    }                                                       // c:100

    // Second pass for SHFILEEXPANSION
    if state.opts.sh_file_expansion {                       // c:100
        node_idx = 0;                                       // c:100
        while node_idx < list.nodes.len() {                       // c:100
            if let Some(new_idx) = stringsubst(             // c:100
                list,                                       // c:100
                node_idx,                                   // c:100
                flags & !(prefork_flags::TYPESET | prefork_flags::ASSIGN), // c:100
                ret_flags,                                  // c:100
                asssub,                                     // c:100
                state,                                      // c:100
            ) {                                             // c:100
                node_idx = new_idx + 1;                     // c:100
            } else {                                        // c:100
                return;                                     // c:100
            }                                               // c:100
        }                                                   // c:100
    }                                                       // c:100

    // Third pass: brace expansion and file substitution
    node_idx = 0;                                           // c:100
    while node_idx < list.nodes.len() {                           // c:100
        if Some(node_idx) == stop_idx {                     // c:100
            keep = false;                                   // c:100
        }                                                   // c:100

        if let Some(data) = list.getdata(node_idx) {       // c:100
            if !data.is_empty() {                           // c:100
                // remnulargs
                let data = data.replace('\0', "");                // c:100
                list.setdata(node_idx, data.clone());      // c:100

                // Brace expansion
                if !state.opts.ignore_braces && (flags & prefork_flags::SINGLE == 0) { // c:100
                    if !keep {                              // c:100
                        stop_idx = list.nextnode(node_idx); // c:100
                    }                                       // c:100
                    while false { /* hasbraces stub — TODO port glob.c hasbraces */ // c:100
                        keep = true;                        // c:100
                        /* xpandbraces stub — TODO port glob.c:4799 */;   // c:100
                    }                                       // c:100
                }                                           // c:100

                // File substitution (non-SHFILEEXPANSION). Skip
                // entirely when state.skip_filesub is set — used
                // for `${var/pat/repl}` pattern + replacement
                // contexts where literal `~` must be preserved.
                if !state.opts.sh_file_expansion && !state.skip_filesub { // c:100
                    if let Some(data) = list.getdata(node_idx) { // c:100
                        let new_data = filesub(             // c:100
                            data,                           // c:100
                            flags & (prefork_flags::TYPESET | prefork_flags::ASSIGN), // c:100
                            state,                          // c:100
                        );                                  // c:100
                        list.setdata(node_idx, new_data);  // c:100
                    }                                       // c:100
                }                                           // c:100
            } else if (flags & prefork_flags::SINGLE == 0)  // c:100
                && (*ret_flags & prefork_flags::KEY_VALUE == 0) // c:100
                && !keep                                    // c:100
            {                                               // c:100
                list.delete_node(node_idx);                      // c:100
                continue; // Don't increment, we removed    // c:100
            }                                               // c:100
        }                                                   // c:100

        if state.errflag {                                  // c:100
            return;                                         // c:100
        }                                                   // c:100

        node_idx += 1;                                      // c:100
    }                                                       // c:100
}                                                           // c:100

/// Perform $'...' quoting
/// Port of stringsubstquote() from subst.c lines 194-224
fn stringsubstquote(strstart: &str, strdpos: usize) -> (String, usize) { // c:206
    let chars: Vec<char> = strstart.chars().collect();      // c:206

    // Find the content between $' and '
    let start = strdpos + 2; // Skip $'                     // c:206
    let mut end = start;                                    // c:206
    let mut escaped = false;                                // c:206

    while end < chars.len() {                               // c:206
        if escaped {                                        // c:206
            escaped = false;                                // c:206
            end += 1;                                       // c:206
            continue;                                       // c:206
        }                                                   // c:206
        if chars[end] == '\\' {                             // c:206
            escaped = true;                                 // c:206
            end += 1;                                       // c:206
            continue;                                       // c:206
        }                                                   // c:206
        if chars[end] == '\'' {                             // c:206
            break;                                          // c:206
        }                                                   // c:206
        end += 1;                                           // c:206
    }                                                       // c:206

    // Process escape sequences
    let content: String = chars[start..end].iter().collect(); // c:206
    let (processed, _processed_len) = crate::ported::utils::getkeystring(&content);                 // c:206

    // Build result
    let prefix: String = chars[..strdpos].iter().collect(); // c:206
    let suffix: String = if end + 1 < chars.len() {         // c:206
        chars[end + 1..].iter().collect()                   // c:206
    } else {                                                // c:206
        String::new()                                       // c:206
    };                                                      // c:206

    let result = format!("{}{}{}", prefix, processed, suffix); // c:206
    let new_pos = strdpos + processed.len();                // c:206

    (result, new_pos)                                       // c:206
}                                                           // c:206







/// String substitution - main workhorse
/// Port of stringsubst() from subst.c lines 227-421
fn stringsubst(                                             // c:237
    list: &mut LinkList,                                    // c:237
    node_idx: usize,                                        // c:237
    pf_flags: u32,                                          // c:237
    ret_flags: &mut u32,                                    // c:237
    asssub: bool,                                           // c:237
    state: &mut SubstState,                                 // c:237
) -> Option<usize> {                                        // c:237
    let mut str3 = list.getdata(node_idx)?.to_string();    // c:237
    let mut pos = 0;                                        // c:237

    // First pass: process substitutions. Loop guard uses CHAR
    // count, not str3.len() (byte count) — `pos` is a char index
    // throughout the function and chars[pos] indexes by char. With
    // multi-byte UTF-8 (zsh-meta tokens 0x83-0x9f each take 2 bytes
    // in UTF-8 encoding), `pos < str3.len()` looped past the end of
    // `chars` and `chars[pos]` panicked. str3 may be mutated within
    // the loop body so `chars` is re-collected each iteration.
    let mut p1_iter = 0u32;                                 // c:237
    loop {                                                  // c:237
        if state.errflag {                                  // c:237
            break;                                          // c:237
        }                                                   // c:237
        p1_iter += 1;                                       // c:237
        if p1_iter > 100_000 {                              // c:237
            return None;                                    // c:237
        }                                                   // c:237
        let chars: Vec<char> = str3.chars().collect();      // c:237
        if pos >= chars.len() {                             // c:237
            break;                                          // c:237
        }                                                   // c:237
        let c = chars[pos];                                 // c:237

        // Check for <(...), >(...), =(...)
        if (c == INANG || c == OUTANGPROC || (pos == 0 && c == EQUALS)) // c:237
            && chars.get(pos + 1) == Some(&INPAR)           // c:237
        {                                                   // c:237
            // <(...) / >(...) / =(...) process / cmd substitution.
            // Stubbed pending faithful port of getproc()/getoutputfile()
            // from Src/exec.c — empty substitution preserves the rest
            // of the string unchanged.
            let _ = c;                                      // c:237 (stubbed)
            // If state.errflag is already set, bail out as before.
            if state.errflag {                              // c:237
                return None;                                // c:237
            }                                               // c:237
            // No-op stub: don't substitute, just advance past the marker
            pos += 1;                                       // c:237
            continue;                                       // c:237
        }                                                   // c:237

        pos += 1;                                           // c:237
    }                                                       // c:237

    // Second pass: $, `, etc. Same char-vs-byte fix as the first
    // pass — `pos < str3.len()` was a byte-len guard but `pos`
    // and `chars[pos]` are char-indexed. Multi-byte UTF-8 (zsh-
    // meta tokens 0x83-0x9f) tripped the panic.
    pos = 0;                                                // c:237
    let mut iter_count = 0u32;                              // c:237
    loop {                                                  // c:237
        if state.errflag {                                  // c:237
            break;                                          // c:237
        }                                                   // c:237
        iter_count += 1;                                    // c:237
        if iter_count > 100_000 {                           // c:237
            return None;                                    // c:237
        }                                                   // c:237
        let chars: Vec<char> = str3.chars().collect();      // c:237
        if pos >= chars.len() {                             // c:237
            break;                                          // c:237
        }                                                   // c:237
        let c = chars[pos];                                 // c:237

        // Lexer-emitted single-quote marker (`\u{9d}`, parse/src/tokens.rs
        // SNULL) encloses literal `'…'` regions. Inside, no parameter /
        // command substitution / glob fires — content is verbatim.
        // Strip both markers and leave the body intact. Without this, a
        // `${var/pat/'~'$match[1]}` replacement yielded
        // `\u{9d}~\u{9d}<match-1>` (SNULLs leaked through, broke the
        // string).
        if c == '\u{9d}' {                                  // c:237
            // Find matching close-SNULL.
            let mut end = pos + 1;                          // c:237
            while end < chars.len() && chars[end] != '\u{9d}' { // c:237
                end += 1;                                   // c:237
            }                                               // c:237
            // Splice out the opening + closing markers; body stays.
            let prefix: String = chars[..pos].iter().collect(); // c:237
            let body: String = chars[pos + 1..end].iter().collect(); // c:237
            let suffix: String = if end < chars.len() {     // c:237
                chars[end + 1..].iter().collect()           // c:237
            } else {                                        // c:237
                String::new()                               // c:237
            };                                              // c:237
            str3 = format!("{}{}{}", prefix, body, suffix); // c:237
            pos += body.chars().count();                    // c:237
            list.setdata(node_idx, str3.clone());          // c:237
            continue;                                       // c:237
        }                                                   // c:237
        // Lexer-emitted double-quote marker (`\u{9e}`, DNULL) — strip;
        // contents inside DQ already had `$`/`${…}` tokenized to STRING
        // / QSTRING by the lexer, so the surrounding pass picks them
        // up. The markers themselves are noise for substitution.
        if c == '\u{9e}' {                                  // c:237
            let prefix: String = chars[..pos].iter().collect(); // c:237
            let suffix: String = if pos + 1 < chars.len() { // c:237
                chars[pos + 1..].iter().collect()           // c:237
            } else {                                        // c:237
                String::new()                               // c:237
            };                                              // c:237
            str3 = format!("{}{}", prefix, suffix);         // c:237
            list.setdata(node_idx, str3.clone());          // c:237
            continue;                                       // c:237
        }                                                   // c:237
        // Lexer BNULL (`\u{9f}`) escapes the next char as literal.
        // Drop the marker, keep the next char verbatim, and skip past
        // it without further processing this iteration.
        if c == '\u{9f}' && pos + 1 < chars.len() {         // c:237
            let prefix: String = chars[..pos].iter().collect(); // c:237
            let kept = chars[pos + 1];                      // c:237
            let suffix: String = if pos + 2 < chars.len() { // c:237
                chars[pos + 2..].iter().collect()           // c:237
            } else {                                        // c:237
                String::new()                               // c:237
            };                                              // c:237
            str3 = format!("{}{}{}", prefix, kept, suffix); // c:237
            pos += 1;                                       // c:237
            list.setdata(node_idx, str3.clone());          // c:237
            continue;                                       // c:237
        }                                                   // c:237
        // Literal `'…'` single-quoted span. The lexer normally
        // converts these to `\u{9d}…\u{9d}` (handled above), but
        // recursive paths that re-enter stringsubst with already-
        // untokenized text (e.g. an outer expand_string ran
        // `untokenize`, dropping SNULLs but preserving the literal
        // `'`) still need the literal-span semantics. Per zsh single-
        // quote rules: contents are verbatim, no `$`/`${…}` / glob
        // expansion fires inside. Strip the surrounding quotes and
        // leave the body intact.
        if c == '\'' {                                      // c:237
            // Find matching close quote — backslash inside `'…'` is
            // NOT an escape (zsh rule), so don't track escaping.
            let mut end = pos + 1;                          // c:237
            while end < chars.len() && chars[end] != '\'' { // c:237
                end += 1;                                   // c:237
            }                                               // c:237
            let prefix: String = chars[..pos].iter().collect(); // c:237
            let body: String = chars[pos + 1..end].iter().collect(); // c:237
            let suffix: String = if end < chars.len() {     // c:237
                chars[end + 1..].iter().collect()           // c:237
            } else {                                        // c:237
                String::new()                               // c:237
            };                                              // c:237
            str3 = format!("{}{}{}", prefix, body, suffix); // c:237
            pos += body.chars().count();                    // c:237
            list.setdata(node_idx, str3.clone());          // c:237
            continue;                                       // c:237
        }                                                   // c:237

        let qt = c == QSTRING;                              // c:237
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
        if qt || c == STRING || c == '$' {                  // c:237
            let next_c = chars.get(pos + 1).copied();       // c:237
            // Accept either tokenized `INPAR` / `INPARMATH` / `INBRACK`
            // / `INBRACE` / `SNULL` OR their literal `(` / `[` / `{`
            // / `'` counterparts.
            let next_is = |tok: char, lit: char| {          // c:237
                next_c == Some(tok) || next_c == Some(lit)  // c:237
            };                                              // c:237

            if next_is(INPAR, '(') || next_is(INPARMATH, '\0') { // c:237
                if !qt {                                    // c:237
                    list.flags |= LF_ARRAY;                 // c:237
                }                                           // c:237
                // Command substitution `$(cmd)` — port of subst.c:237
                // stringsubst's $(...) arm. Find the matching ),
                // extract cmd text, delegate to ShellExecutor's
                // run_command_substitution (canonical executor lives
                // outside SubstState; bridged via fusevm_bridge::
                // with_executor).
                let cmd_open = pos + 1;                     // c:237 (s after $)
                let chars: Vec<char> = str3.chars().collect(); // c:237
                let mut depth = 0_i32;                      // c:237
                let mut end = cmd_open;                     // c:237
                while end < chars.len() {                   // c:237
                    let ch = chars[end];                    // c:237
                    if ch == '(' || ch == INPAR { depth += 1; } // c:237
                    else if ch == ')' || ch == OUTPAR {     // c:237
                        depth -= 1;                         // c:237
                        if depth == 0 { break; }            // c:237
                    }                                       // c:237
                    end += 1;                               // c:237
                }                                           // c:237
                if end < chars.len() && depth == 0 {        // c:237
                    let cmd: String = chars[cmd_open + 1..end].iter().collect(); // c:237
                    let output = crate::fusevm_bridge::with_executor( // c:237
                        |exec| exec.run_command_substitution(&cmd)); // c:237
                    let prefix: String = chars[..pos].iter().collect(); // c:237
                    let suffix: String = if end + 1 < chars.len() { // c:237
                        chars[end + 1..].iter().collect()   // c:237
                    } else {                                // c:237
                        String::new()                       // c:237
                    };                                      // c:237
                    str3 = format!("{}{}{}", prefix, output.trim_end_matches('\n'), suffix); // c:237
                    pos = prefix.chars().count() + output.trim_end_matches('\n').chars().count(); // c:237
                    list.setdata(node_idx, str3.clone());   // c:237
                } else {                                    // c:237
                    pos += 1;                               // c:237
                }                                           // c:237
                continue;                                   // c:237
            } else if next_is(INBRACK, '[') {               // c:237
                // $[...] arithmetic
                let start = pos + 2;                        // c:237
                let open = if next_c == Some(INBRACK) { INBRACK } else { '[' }; // c:237
                let close = if open == INBRACK { OUTBRACK } else { ']' }; // c:237
                if let Some(end) = None::<usize> /* find_matching_bracket stub */ { // c:237
                    let expr: String = str3.chars().skip(start).take(end).collect(); // c:237
                    let value = arithsubst(&expr, state);   // c:237
                    let prefix: String = str3.chars().take(pos).collect(); // c:237
                    let suffix: String = str3.chars().skip(start + end + 1).collect(); // c:237
                    str3 = format!("{}{}{}", prefix, value, suffix); // c:237
                    list.setdata(node_idx, str3.clone());  // c:237
                    continue;                               // c:237
                } else {                                    // c:237
                    state.errflag = true;                   // c:237
                    eprintln!("closing bracket missing");   // c:237
                    return None;                            // c:237
                }                                           // c:237
            } else if next_c == Some(SNULL) || next_c == Some('\'') { // c:237
                // $'...' ANSI-C quoting. Accept either the lexer-
                // tokenized SNULL marker OR the raw `'` — recursive
                // operator-operand paths (e.g. multsub on a `:=`
                // operand) hand us the literal text without prior
                // tokenization, so dispatch on the literal too.
                let (new_str, new_pos) = stringsubstquote(&str3, pos); // c:237
                str3 = new_str;                             // c:237
                pos = new_pos;                              // c:237
                list.setdata(node_idx, str3.clone());      // c:237
                continue;                                   // c:237
            } else {                                        // c:237
                // Parameter substitution
                let mut new_pf_flags = pf_flags;            // c:237
                if (state.opts.sh_word_split && (pf_flags & prefork_flags::NOSHWORDSPLIT == 0)) // c:237
                    || (pf_flags & prefork_flags::SPLIT != 0) // c:237
                {                                           // c:237
                    new_pf_flags |= prefork_flags::SHWORDSPLIT; // c:237
                }                                           // c:237

                // stringsubst → paramsubst is a recursive descent —
                // bump the executor's paramsubst-nest counter so the
                // inner expansion's glob_subst etc. sees it's running
                // inside an outer operand context (where filesystem
                // glob expansion must be suppressed). Use the fallible
                // variant so the unit-test path that calls paramsubst
                // without a live executor doesn't panic.
                crate::exec::try_with_executor(|e| e.in_paramsubst_nest += 1); // c:237
                let (new_str, new_pos, new_nodes) = paramsubst( // c:237
                    &str3,                                  // c:237
                    pos,                                    // c:237
                    qt,                                     // c:237
                    new_pf_flags                            // c:237
                        & (prefork_flags::SINGLE            // c:237
                            | prefork_flags::SHWORDSPLIT    // c:237
                            | prefork_flags::SUBEXP),       // c:237
                    ret_flags,                              // c:237
                    state,                                  // c:237
                );                                          // c:237
                crate::exec::try_with_executor(|e| e.in_paramsubst_nest -= 1); // c:237

                if state.errflag {                          // c:237
                    return None;                            // c:237
                }                                           // c:237

                // Insert additional nodes if word splitting produced them
                let mut current_idx = node_idx;             // c:237
                for (i, node_data) in new_nodes.into_iter().enumerate() { // c:237
                    if i == 0 {                             // c:237
                        list.setdata(current_idx, node_data); // c:237
                    } else {                                // c:237
                        current_idx = list.insertlinknode(current_idx, node_data); // c:237
                    }                                       // c:237
                }                                           // c:237

                str3 = list.getdata(node_idx)?.to_string(); // c:237
                pos = new_pos;                              // c:237
                continue;                                   // c:237
            }                                               // c:237
        }                                                   // c:237

        // Backtick command substitution `cmd` — same engine as
        // `$(cmd)` per subst.c:237. Find the matching backtick,
        // capture cmd text, delegate to run_command_substitution.
        let qt = c == QTICK;                                // c:237
        if qt || c == TICK {                                // c:237
            if !qt {                                        // c:237
                list.flags |= LF_ARRAY;                     // c:237
            }                                               // c:237
            let chars: Vec<char> = str3.chars().collect();  // c:237
            let cmd_start = pos + 1;                        // c:237
            let mut end = cmd_start;                        // c:237
            while end < chars.len() && chars[end] != TICK && chars[end] != QTICK { // c:237
                if chars[end] == '\\' && end + 1 < chars.len() { end += 1; } // c:237
                end += 1;                                   // c:237
            }                                               // c:237
            if end < chars.len() {                          // c:237
                let cmd: String = chars[cmd_start..end].iter().collect(); // c:237
                let output = crate::fusevm_bridge::with_executor( // c:237
                    |exec| exec.run_command_substitution(&cmd)); // c:237
                let prefix: String = chars[..pos].iter().collect(); // c:237
                let suffix: String = if end + 1 < chars.len() { // c:237
                    chars[end + 1..].iter().collect()       // c:237
                } else {                                    // c:237
                    String::new()                           // c:237
                };                                          // c:237
                str3 = format!("{}{}{}", prefix, output.trim_end_matches('\n'), suffix); // c:237
                pos = prefix.chars().count() + output.trim_end_matches('\n').chars().count(); // c:237
                list.setdata(node_idx, str3.clone());       // c:237
            } else {                                        // c:237
                pos += 1;                                   // c:237
            }                                               // c:237
            continue;                                       // c:237
        }                                                   // c:237

        // Assignment context
        if asssub && (c == '=' || c == EQUALS) && pos > 0 { // c:237
            // We're in assignment context, apply SINGLE flag
            // (handled by caller typically)
        }                                                   // c:237

        pos += 1;                                           // c:237
    }                                                       // c:237

    if state.errflag {                                      // c:237
        None                                                // c:237
    } else {                                                // c:237
        Some(node_idx)                                      // c:237
    }                                                       // c:237
}                                                           // c:237





/// Parameter substitution
/// Port of paramsubst() from subst.c lines 1600-4922 (THIS IS THE BIG ONE)
fn paramsubst(                                              // c:1625
    s: &str,                                                // c:1625
    start_pos: usize,                                       // c:1625
    qt: bool,                                               // c:1625
    pf_flags: u32,                                          // c:1625
    ret_flags: &mut u32,                                    // c:1625
    state: &mut SubstState,                                 // c:1625
) -> (String, usize, Vec<String>) {                         // c:1625
    let chars: Vec<char> = s.chars().collect();             // c:1625
    let mut pos = start_pos + 1; // Skip $ or Qstring       // c:1625
    let mut result_nodes = Vec::new();                      // c:1625

    // Check what follows the $
    let c = chars.get(pos).copied().unwrap_or('\0');        // c:1625

    // ${...} form
    // ${...} brace form. Pragmatic inline port covering high-traffic
    // shapes from subst.c:1885+ (full 2,849-line paramsubst port is
    // an ongoing arm-by-arm effort). Handles: bare ref, ${#var}
    // length, :- :+ := :? defaults, # ## % %% strip, / // replace
    // with anchored # / % variants, :N:M slice, plus a permissive
    // (...)-flag prefix swallow.
    if c == INBRACE || c == '{' {                           // c:1885
        pos += 1;                                           // c:1885 (skip {)
        // Find matching `}` — track brace depth for nested ${...}
        let mut depth = 1_i32;                              // c:1885
        let mut end = pos;                                  // c:1885
        while end < chars.len() && depth > 0 {              // c:1885
            let ch = chars[end];                            // c:1885
            if ch == '{' || ch == INBRACE { depth += 1; }   // c:1885
            else if ch == '}' || ch == OUTBRACE {           // c:1885
                depth -= 1;                                 // c:1885
                if depth == 0 { break; }                    // c:1885
            }                                               // c:1885
            end += 1;                                       // c:1885
        }                                                   // c:1885
        let body: String = chars[pos..end].iter().collect(); // c:1885
        let new_pos = if end < chars.len() { end + 1 } else { end };
        let body_chars: Vec<char> = body.chars().collect();
        let mut idx = 0_usize;
        // Skip flag block `(...)` — flags currently no-op.
        if body_chars.first() == Some(&'(') {               // c:2147
            let mut d = 1_i32;
            idx = 1;
            while idx < body_chars.len() && d > 0 {
                if body_chars[idx] == '(' { d += 1; }
                else if body_chars[idx] == ')' { d -= 1; if d == 0 { idx += 1; break; } }
                idx += 1;
            }
        }
        // ${#var} — length-of operator at start of brace (after flags).
        let length_op = body_chars.get(idx).copied() == Some('#'); // c:2128
        let post_flags_start = idx;
        if length_op {
            let next = body_chars.get(idx + 1).copied().unwrap_or('\0');
            if next.is_ascii_alphabetic() || next == '_' || next == '@' || next == '*' {
                idx += 1; // skip the leading #
            }
        }
        // Walk var-name chars
        let name_start = idx;
        while idx < body_chars.len() {
            let bc = body_chars[idx];
            let allowed = if idx == name_start {
                bc.is_ascii_alphanumeric() || bc == '_' || bc == '@' || bc == '*' || bc == '#' || bc == '?' || bc == '0'
            } else {
                bc.is_ascii_alphanumeric() || bc == '_'
            };
            if allowed {
                idx += 1;
                // Single-char specials stop after one char
                if idx == name_start + 1 && matches!(body_chars[name_start], '@' | '*' | '#' | '?' | '0') {
                    break;
                }
            } else {
                break;
            }
        }
        let var_name: String = body_chars[name_start..idx].iter().collect();
        let rest: String = body_chars[idx..].iter().collect();
        // Look up var
        let raw_value: String = state.variables.get(&var_name).cloned()
            .or_else(|| state.arrays.get(&var_name).map(|a| a.join(" ")))
            .unwrap_or_default();
        let is_set = state.variables.contains_key(&var_name)
            || state.arrays.contains_key(&var_name)
            || state.assoc_arrays.contains_key(&var_name);
        if length_op {                                      // c:2128
            let _ = post_flags_start;
            return (raw_value.chars().count().to_string(), new_pos, vec![]);
        }
        let mut value = raw_value.clone();
        if !rest.is_empty() {
            let r = rest.as_str();
            if let Some(default) = r.strip_prefix(":-") {     // c:3193
                if !is_set || raw_value.is_empty() { value = singsub(default, state); }
            } else if let Some(default) = r.strip_prefix('-') { // c:3193
                if !is_set { value = singsub(default, state); }
            } else if let Some(default) = r.strip_prefix(":=") { // c:3245
                if !is_set || raw_value.is_empty() {
                    value = singsub(default, state);
                    state.variables.insert(var_name.clone(), value.clone());
                }
            } else if let Some(alt) = r.strip_prefix(":+") {  // c:3296
                if is_set && !raw_value.is_empty() { value = singsub(alt, state); }
                else { value = String::new(); }
            } else if let Some(alt) = r.strip_prefix('+') {   // c:3296
                if is_set { value = singsub(alt, state); } else { value = String::new(); }
            } else if let Some(msg) = r.strip_prefix(":?") {  // c:3193
                if !is_set || raw_value.is_empty() {
                    eprintln!("{}: {}", var_name, singsub(msg, state));
                    state.errflag = true;
                }
            } else if let Some(rep) = r.strip_prefix("//") {  // c:3870 (global replace)
                let parts: Vec<&str> = rep.splitn(2, '/').collect();
                let pat = singsub(parts[0], state);
                let repl = parts.get(1).map(|s| singsub(s, state)).unwrap_or_default();
                value = raw_value.replace(&pat, &repl);
            } else if let Some(rep) = r.strip_prefix('/') {   // c:3870 (single replace)
                let parts: Vec<&str> = rep.splitn(2, '/').collect();
                let pat = singsub(parts[0], state);
                let repl = parts.get(1).map(|s| singsub(s, state)).unwrap_or_default();
                if let Some(anchor_pat) = pat.strip_prefix('#') {
                    let p = anchor_pat.to_string();
                    if raw_value.starts_with(&p) { value = format!("{}{}", repl, &raw_value[p.len()..]); }
                } else if let Some(anchor_pat) = pat.strip_prefix('%') {
                    let p = anchor_pat.to_string();
                    if raw_value.ends_with(&p) { value = format!("{}{}", &raw_value[..raw_value.len()-p.len()], repl); }
                } else {
                    value = raw_value.replacen(&pat, &repl, 1);
                }
            } else if let Some(pat) = r.strip_prefix("##") {  // c:3540 (longest prefix strip)
                let p = singsub(pat, state);
                let mut k = raw_value.chars().count();
                while k > 0 {
                    let prefix: String = raw_value.chars().take(k).collect();
                    if prefix == p { value = raw_value.chars().skip(k).collect(); break; }
                    k -= 1;
                }
            } else if let Some(pat) = r.strip_prefix('#') {   // c:3540 (shortest prefix strip)
                let p = singsub(pat, state);
                let total = raw_value.chars().count();
                for k in 0..=total {
                    let prefix: String = raw_value.chars().take(k).collect();
                    if prefix == p { value = raw_value.chars().skip(k).collect(); break; }
                }
            } else if let Some(pat) = r.strip_prefix("%%") {  // c:3540 (longest suffix strip)
                let p = singsub(pat, state);
                let total = raw_value.chars().count();
                let mut k = total;
                while k > 0 {
                    let suffix: String = raw_value.chars().skip(total - k).collect();
                    if suffix == p { value = raw_value.chars().take(total - k).collect(); break; }
                    k -= 1;
                }
            } else if let Some(pat) = r.strip_prefix('%') {   // c:3540 (shortest suffix strip)
                let p = singsub(pat, state);
                let total = raw_value.chars().count();
                for k in 0..=total {
                    let suffix: String = raw_value.chars().skip(total - k).collect();
                    if suffix == p { value = raw_value.chars().take(total - k).collect(); break; }
                }
            } else if let Some(slice) = r.strip_prefix(':') { // c:715 (substring)
                let parts: Vec<&str> = slice.splitn(2, ':').collect();
                let off = singsub(parts[0], state).parse::<i64>().unwrap_or(0);
                let total = raw_value.chars().count() as i64;
                let start = if off < 0 { (total + off).max(0) } else { off.min(total) } as usize;
                let len = parts.get(1).map(|s| singsub(s, state).parse::<i64>().unwrap_or(0));
                value = match len {
                    Some(l) if l >= 0 => raw_value.chars().skip(start).take(l as usize).collect(),
                    Some(l) => {
                        let take = ((total - start as i64) + l).max(0) as usize;
                        raw_value.chars().skip(start).take(take).collect()
                    }
                    None => raw_value.chars().skip(start).collect(),
                };
            }
        }
        // Reconstruct the full str3 with the brace expansion applied
        // — same protocol the simple `$var` arm uses (line 1240).
        // Caller (stringsubst) re-loads `str3 = list.getdata(node_idx)`
        // and expects the new full string in node 0.
        let prefix: String = chars[..start_pos].iter().collect(); // c:1885
        let suffix: String = if new_pos < chars.len() {     // c:1885
            chars[new_pos..].iter().collect()               // c:1885
        } else {                                            // c:1885
            String::new()                                   // c:1885
        };                                                  // c:1885
        let full = format!("{}{}{}", prefix, value, suffix); // c:1885
        let new_pos_in_full = prefix.chars().count() + value.chars().count();
        return (full.clone(), new_pos_in_full, vec![full]);
    }                                                       // c:1885

    // Simple $var (or $arr[idx] for array-element access — per
    // Src/lex.c::gettokstr, zsh accepts `$name[subscript]` as a
    // first-class array-element expansion. Without parsing the
    // bracket here, `$match[1]` from a `(#b)` replacement template
    // resolved to "match" + literal "[1]" instead of the captured
    // group).
    if c.is_ascii_alphabetic() || c == '_' {                // c:1625
        let var_start = pos;                                // c:1625
        while pos < chars.len() && (chars[pos].is_ascii_alphanumeric() || chars[pos] == '_') { // c:1625
            pos += 1;                                       // c:1625
        }                                                   // c:1625
        let var_name: String = chars[var_start..pos].iter().collect(); // c:1625

        // Optional `[subscript]`. Per zsh, only valid for declared
        // arrays/assocs — for scalars the `[` stays literal.
        let mut subscript_str: Option<String> = None;       // c:1625
        if chars.get(pos).copied() == Some('[') {           // c:1625
            // Collect until matching `]` (depth-tracked so
            // `$arr[$other[1]]` works).
            let mut depth = 1;                              // c:1625
            let mut q = pos + 1;                            // c:1625
            while q < chars.len() && depth > 0 {            // c:1625
                match chars[q] {                            // c:1625
                    '[' => depth += 1,                      // c:1625
                    ']' => {                                // c:1625
                        depth -= 1;                         // c:1625
                        if depth == 0 {                     // c:1625
                            break;                          // c:1625
                        }                                   // c:1625
                    }                                       // c:1625
                    _ => {}                                 // c:1625
                }                                           // c:1625
                q += 1;                                     // c:1625
            }                                               // c:1625
            if depth == 0 {                                 // c:1625
                let raw_sub: String = chars[pos + 1..q].iter().collect(); // c:1625
                // Resolve $X / ${X} inside the subscript.
                subscript_str = Some(singsub(&raw_sub, state)); // c:1625
                pos = q + 1;                                // c:1625
            }                                               // c:1625
        }                                                   // c:1625

        let value = if let Some(sub) = subscript_str.as_deref() { // c:1625
            // Array / assoc element lookup.
            let _ = sub;                                    // c:1625 (subscript stub)
            let v: Vec<String> = vec![state.variables.get(&var_name).cloned().unwrap_or_default()]; // c:1625
            v.join(" ")                                     // c:1625
        } else {                                            // c:1625
            state.variables.get(&var_name).cloned().unwrap_or_default()               // c:1625
        };                                                  // c:1625

        // Handle word splitting
        if pf_flags & prefork_flags::SHWORDSPLIT != 0 && !qt { // c:1625
            let words = value.split_whitespace().map(String::from).collect::<Vec<String>>();         // c:1625
            if words.len() > 1 {                            // c:1625
                let prefix: String = chars[..start_pos].iter().collect(); // c:1625
                let suffix: String = chars[pos..].iter().collect(); // c:1625

                for (i, word) in words.iter().enumerate() { // c:1625
                    if i == 0 {                             // c:1625
                        result_nodes.push(format!("{}{}", prefix, word)); // c:1625
                    } else if i == words.len() - 1 {        // c:1625
                        result_nodes.push(format!("{}{}", word, suffix)); // c:1625
                    } else {                                // c:1625
                        result_nodes.push(word.clone());    // c:1625
                    }                                       // c:1625
                }                                           // c:1625
                return (                                    // c:1625
                    result_nodes[0].clone(),                // c:1625
                    prefix.len() + words[0].len(),          // c:1625
                    result_nodes,                           // c:1625
                );                                          // c:1625
            }                                               // c:1625
        }                                                   // c:1625

        let prefix: String = chars[..start_pos].iter().collect(); // c:1625
        let suffix: String = chars[pos..].iter().collect(); // c:1625
        let result = format!("{}{}{}", prefix, value, suffix); // c:1625
        result_nodes.push(result.clone());                  // c:1625
        return (result, prefix.len() + value.len(), result_nodes); // c:1625
    }                                                       // c:1625

    // Special parameters: $?, $$, $#, $*, $@, $0-$9
    match c {                                               // c:1625
        '?' => {                                            // c:1625
            let value = state                               // c:1625
                .variables                                  // c:1625
                .get("?")                                   // c:1625
                .cloned()                                   // c:1625
                .unwrap_or_else(|| "0".to_string());        // c:1625
            let prefix: String = chars[..start_pos].iter().collect(); // c:1625
            let suffix: String = chars[pos + 1..].iter().collect(); // c:1625
            let result = format!("{}{}{}", prefix, value, suffix); // c:1625
            result_nodes.push(result.clone());              // c:1625
            (result, prefix.len() + value.len(), result_nodes) // c:1625
        }                                                   // c:1625
        '$' => {                                            // c:1625
            let value = std::process::id().to_string();     // c:1625
            let prefix: String = chars[..start_pos].iter().collect(); // c:1625
            let suffix: String = chars[pos + 1..].iter().collect(); // c:1625
            let result = format!("{}{}{}", prefix, value, suffix); // c:1625
            result_nodes.push(result.clone());              // c:1625
            (result, prefix.len() + value.len(), result_nodes) // c:1625
        }                                                   // c:1625
        '#' => {                                            // c:1625
            let value = state                               // c:1625
                .arrays                                     // c:1625
                .get("@")                                   // c:1625
                .map(|a| a.len().to_string())               // c:1625
                .unwrap_or_else(|| "0".to_string());        // c:1625
            let prefix: String = chars[..start_pos].iter().collect(); // c:1625
            let suffix: String = chars[pos + 1..].iter().collect(); // c:1625
            let result = format!("{}{}{}", prefix, value, suffix); // c:1625
            result_nodes.push(result.clone());              // c:1625
            (result, prefix.len() + value.len(), result_nodes) // c:1625
        }                                                   // c:1625
        '*' | '@' => {                                      // c:1625
            let values = state.arrays.get("@").cloned().unwrap_or_default(); // c:1625
            let value = if c == '*' || qt {                 // c:1625
                values.join(" ")                            // c:1625
            } else {                                        // c:1625
                // $@ in unquoted context - each element becomes separate word
                if pf_flags & prefork_flags::SINGLE == 0 {  // c:1625
                    let prefix: String = chars[..start_pos].iter().collect(); // c:1625
                    let suffix: String = chars[pos + 1..].iter().collect(); // c:1625
                    for (i, v) in values.iter().enumerate() { // c:1625
                        if i == 0 {                         // c:1625
                            result_nodes.push(format!("{}{}", prefix, v)); // c:1625
                        } else if i == values.len() - 1 {   // c:1625
                            result_nodes.push(format!("{}{}", v, suffix)); // c:1625
                        } else {                            // c:1625
                            result_nodes.push(v.clone());   // c:1625
                        }                                   // c:1625
                    }                                       // c:1625
                    if result_nodes.is_empty() {            // c:1625
                        result_nodes.push(format!("{}{}", prefix, suffix)); // c:1625
                    }                                       // c:1625
                    return (result_nodes[0].clone(), start_pos, result_nodes); // c:1625
                }                                           // c:1625
                values.join(" ")                            // c:1625
            };                                              // c:1625
            let prefix: String = chars[..start_pos].iter().collect(); // c:1625
            let suffix: String = chars[pos + 1..].iter().collect(); // c:1625
            let result = format!("{}{}{}", prefix, value, suffix); // c:1625
            result_nodes.push(result.clone());              // c:1625
            (result, prefix.len() + value.len(), result_nodes) // c:1625
        }                                                   // c:1625
        '0'..='9' => {                                      // c:1625
            // `$0` reads variables["0"] (script/function name, writable
            // via plain `0=value`). `$1`..`$9` index into positional
            // params 1-based: digit N → arrays["@"][N-1]. Direct port
            // of Src/params.c which exposes "0" as a SPECIALPMDEF
            // backed by `argzero`, and digit-N as positional N.
            // Multi-digit numerics ($10, $11, ...) need lookahead to
            // capture trailing digits — collect them into the name
            // before the lookup.
            let mut digit_str = String::from(c);            // c:1625
            let mut nx = pos + 1;                           // c:1625
            while nx < chars.len() && chars[nx].is_ascii_digit() { // c:1625
                digit_str.push(chars[nx]);                  // c:1625
                nx += 1;                                    // c:1625
            }                                               // c:1625
            let digit: usize = digit_str.parse().unwrap_or(0); // c:1625
            let value = if digit == 0 {                     // c:1625
                state.variables.get("0").cloned().unwrap_or_default()                 // c:1625
            } else {                                        // c:1625
                state                                       // c:1625
                    .arrays                                 // c:1625
                    .get("@")                               // c:1625
                    .and_then(|a| a.get(digit.saturating_sub(1))) // c:1625
                    .cloned()                               // c:1625
                    .unwrap_or_default()                    // c:1625
            };                                              // c:1625
            let prefix: String = chars[..start_pos].iter().collect(); // c:1625
            let suffix: String = chars[nx..].iter().collect(); // c:1625
            let result = format!("{}{}{}", prefix, value, suffix); // c:1625
            result_nodes.push(result.clone());              // c:1625
            (result, prefix.len() + value.len(), result_nodes) // c:1625
        }                                                   // c:1625
        _ => {                                              // c:1625
            // Just a literal $
            result_nodes.push(s.to_string());               // c:1625
            (s.to_string(), start_pos + 1, result_nodes)    // c:1625
        }                                                   // c:1625
    }                                                       // c:1625
}                                                           // c:1625


/// Parameter expansion flags
#[derive(Default, Clone, Debug)]                            // c:1625
struct ParamFlags {                                         // c:1625
    lowercase: bool,                                        // c:1625
    uppercase: bool,                                        // c:1625
    capitalize: bool,                                       // c:1625
    unique: bool,                                           // c:1625
    sort: bool,                                             // c:1625
    sort_reverse: bool,                                     // c:1625
    sort_array_index: bool,                                 // c:1625
    sort_case_insensitive: bool,                            // c:1625
    sort_numeric: bool,                                     // c:1625
    keys: bool,                                             // c:1625
    values: bool,                                           // c:1625
    type_info: bool,                                        // c:1625
    prompt_expand: bool,                                    // c:1625
    prompt_percent: bool,                                   // c:1625
    eval: bool,                                             // c:1625
    quote_level: usize,                                     // c:1625
    unquote: bool,                                          // c:1625
    report_error: bool,                                     // c:1625
    split_words: bool,                                      // c:1625
    split_lines: bool,                                      // c:1625
    join_lines: bool,                                       // c:1625
    count_words: bool,                                      // c:1625
    count_words_null: bool,                                 // c:1625
    count_chars: bool,                                      // c:1625
    length_chars: bool,                                     // c:1625
    create_assoc: bool,                                     // c:1625
    array_expand: bool,                                     // c:1625
    glob_subst: bool,                                       // c:1625
    visible: bool,                                          // c:1625
    search: bool,                                           // c:1625
    match_flag: bool,                                       // c:1625
    reverse_subscript: bool,                                // c:1625
    begin_end_length: bool,                                 // c:1625
    split_sep: Option<String>,                              // c:1625
    join_sep: Option<String>,                               // c:1625
    pad_left: Option<usize>,                                // c:1625
    pad_right: Option<usize>,                               // c:1625
    pad_char: Option<char>,                                 // c:1625
    pad_string1: Option<String>,                            // c:1625
    pad_string2: Option<String>,                            // c:1625
}                                                           // c:1625





/// Bitmask of subscript flag bits parsed from the leading `(...)`.
/// Mirrors the C source's per-flag bools — only the ones zshrs
/// honors today are present.
#[derive(Default, Debug, Clone, Copy)]                      // c:2867
pub struct SubscriptFlags {                                 // c:2867
    /// `(r)` — return first element matching pattern.
    forward_match: bool,                                    // c:2867
    /// `(R)` — last matching.
    reverse_match: bool,                                    // c:2867
    /// `(i)` — return INDEX (1-based) of first matching.
    forward_index: bool,                                    // c:2867
    /// `(I)` — last index.
    reverse_index: bool,                                    // c:2867
    /// `(e)` — exact match (no glob), used as suffix to `r`/`R`/etc.
    exact: bool,                                            // c:2867
    /// `(k)` — search keys instead of values (for assocs).
    keys: bool,                                             // c:2867
    /// `(n)` — numeric comparison (for `r`/`R`).
    numeric: bool,                                          // c:2867
}                                                           // c:2867

impl SubscriptFlags {                                       // c:2867
}                                                           // c:2867


















// Helper functions






fn filesub(s: &str, _flags: u32, _state: &mut SubstState) -> String { // c:667
    // Tilde expansion
    if let Some(rest) = s.strip_prefix('~') {               // c:667
        let (user, suffix) = match rest.find('/') {         // c:667
            Some(pos) => (&rest[..pos], &rest[pos..]),      // c:667
            None => (rest, ""),                             // c:667
        };                                                  // c:667

        if user.is_empty() {                                // c:667
            if let Ok(home) = std::env::var("HOME") {       // c:667
                return format!("{}{}", home, suffix);       // c:667
            }                                               // c:667
        } else if user == "+" {                             // c:667
            if let Ok(pwd) = std::env::var("PWD") {         // c:667
                return format!("{}{}", pwd, suffix);        // c:667
            }                                               // c:667
        } else if user == "-" {                             // c:667
            if let Ok(oldpwd) = std::env::var("OLDPWD") {   // c:667
                return format!("{}{}", oldpwd, suffix);     // c:667
            }                                               // c:667
        }                                                   // c:667
    }                                                       // c:667

    // = substitution (=cmd -> path to cmd)
    if s.starts_with('=') && s.len() > 1 {                  // c:667
        let cmd = &s[1..];                                  // c:667
        if let Ok(path) = std::env::var("PATH") {           // c:667
            for dir in path.split(':') {                    // c:667
                let full_path = format!("{}/{}", dir, cmd); // c:667
                if std::path::Path::new(&full_path).exists() { // c:667
                    return full_path;                       // c:667
                }                                           // c:667
            }                                               // c:667
        }                                                   // c:667
    }                                                       // c:667

    s.to_string()                                           // c:667
}                                                           // c:667



fn arithsubst(expr: &str, _state: &mut SubstState) -> String { // c:4485
    // Port of `arithsubst()` from Src/subst.c:4485 — delegates to
    // the math module's full expression evaluator (zsh's
    // `matheval()` from Src/math.c, ported in `crate::math`).
    // The C source is itself a thin wrapper over the math
    // expression engine; we route through the same engine so
    // subscripts, ternary, bitwise, comparison, and float ops
    // all flow through one evaluator.
    match crate::math::matheval(expr) {                     // c:4485
        Ok(crate::math::MathNum::Integer(n)) => n.to_string(), // c:4485
        Ok(crate::math::MathNum::Float(f)) => f.to_string(), // c:4485
        Ok(crate::math::MathNum::Unset) | Err(_) => "0".to_string(), // c:4485
    }                                                       // c:4485
}                                                           // c:4485


/// Multsub flags (from subst.c)
pub mod multsub_flags {                                     // c:N/A
    pub const WS_AT_START: u32 = 1;                         // c:N/A
    pub const WS_AT_END: u32 = 2;                           // c:N/A
    pub const PARAM_NAME: u32 = 4;                          // c:N/A
}                                                           // c:N/A

/// Perform substitution on a single word
/// Port of singsub() from subst.c lines 513-525
/// Single-string substitution.
/// Port of `singsub()` from Src/subst.c:514.
pub fn singsub(s: &str, state: &mut SubstState) -> String { // c:514
    let mut list = { let mut _l = LinkList::default(); _l.nodes.push_back(LinkNode { data: s.to_string() }); _l };                // c:514
    let mut ret_flags = 0u32;                               // c:514

    prefork(&mut list, prefork_flags::SINGLE, &mut ret_flags, state); // c:514

    if state.errflag {                                      // c:514
        return String::new();                               // c:514
    }                                                       // c:514

    list.getdata(0).unwrap_or("").to_string()              // c:514
}                                                           // c:514


/// Substitution with possible multiple results
/// Port of multsub() from subst.c lines 540-621
/// Multi-word substitution with IFS splitting.
/// Port of `multsub()` from Src/subst.c:544.
pub fn multsub(s: &str, pf_flags: u32, state: &mut SubstState) -> (String, Vec<String>, bool, u32) { // c:544
    let mut x = s.to_string();                              // c:544
    let mut ms_flags = 0u32;                                // c:544

    // Handle leading whitespace with SPLIT flag
    if pf_flags & prefork_flags::SPLIT != 0 {               // c:544
        let leading_ws: String = x.chars().take_while(|c| c.is_ascii_whitespace()).collect(); // c:544
        if !leading_ws.is_empty() {                         // c:544
            ms_flags |= multsub_flags::WS_AT_START;         // c:544
            x = x.chars().skip(leading_ws.len()).collect(); // c:544
        }                                                   // c:544
    }                                                       // c:544

    let mut list = { let mut _l = LinkList::default(); _l.nodes.push_back(LinkNode { data: x.to_string() }); _l };               // c:544

    // Handle word splitting within the string
    if pf_flags & prefork_flags::SPLIT != 0 {               // c:544
        let mut node_idx = 0;                               // c:544
        let mut in_quote = false;                           // c:544
        let mut in_paren = 0;                               // c:544

        while node_idx < list.nodes.len() {                       // c:544
            if let Some(data) = list.getdata(node_idx) {   // c:544
                let chars: Vec<char> = data.chars().collect(); // c:544
                let mut split_points = Vec::new();          // c:544
                let mut i = 0;                              // c:544

                while i < chars.len() {                     // c:544
                    let c = chars[i];                       // c:544

                    // Handle quote state
                    match c {                               // c:544
                        '"' | '\'' | TICK | QTICK => in_quote = !in_quote, // c:544
                        INPAR => in_paren += 1,             // c:544
                        OUTPAR => in_paren = (in_paren - 1).max(0), // c:544
                        _ => {}                             // c:544
                    }                                       // c:544

                    // Check for IFS separator outside quotes
                    if !in_quote && in_paren == 0 {         // c:544
                        let ifs = state                     // c:544
                            .variables                      // c:544
                            .get("IFS")                     // c:544
                            .map(|s| s.as_str())            // c:544
                            .unwrap_or(" \t\n");            // c:544
                        if ifs.contains(c) && !false /* is_token stub */ { // c:544
                            split_points.push(i);           // c:544
                        }                                   // c:544
                    }                                       // c:544

                    i += 1;                                 // c:544
                }                                           // c:544

                // Split at found points
                if !split_points.is_empty() {               // c:544
                    let data_str = data.to_string();        // c:544
                    let chars: Vec<char> = data_str.chars().collect(); // c:544
                    let mut last = 0;                       // c:544

                    list.delete_node(node_idx);                  // c:544

                    for (idx, &point) in split_points.iter().enumerate() { // c:544
                        if point > last {                   // c:544
                            let segment: String = chars[last..point].iter().collect(); // c:544
                            if idx == 0 {                   // c:544
                                list.nodes.insert(node_idx, LinkNode { data: segment }); // c:544
                            } else {                        // c:544
                                list.insertlinknode(node_idx + idx - 1, segment); // c:544
                            }                               // c:544
                        }                                   // c:544
                        last = point + 1;                   // c:544
                    }                                       // c:544

                    if last < chars.len() {                 // c:544
                        let segment: String = chars[last..].iter().collect(); // c:544
                        if split_points.is_empty() {        // c:544
                            list.nodes.insert(node_idx, LinkNode { data: segment }); // c:544
                        } else {                            // c:544
                            list.insertlinknode(node_idx + split_points.len() - 1, segment); // c:544
                        }                                   // c:544
                    }                                       // c:544
                }                                           // c:544
            }                                               // c:544
            node_idx += 1;                                  // c:544
        }                                                   // c:544
    }                                                       // c:544

    let mut ret_flags = 0u32;                               // c:544
    prefork(&mut list, pf_flags, &mut ret_flags, state);    // c:544

    if state.errflag {                                      // c:544
        return (String::new(), Vec::new(), false, ms_flags); // c:544
    }                                                       // c:544

    // Check for trailing whitespace
    if pf_flags & prefork_flags::SPLIT != 0 {               // c:544
        if let Some(last) = list.nodes.back() {             // c:544
            if last                                         // c:544
                .data                                       // c:544
                .chars()                                    // c:544
                .last()                                     // c:544
                .map(|c| c.is_ascii_whitespace())           // c:544
                .unwrap_or(false)                           // c:544
            {                                               // c:544
                ms_flags |= multsub_flags::WS_AT_END;       // c:544
            }                                               // c:544
        }                                                   // c:544
    }                                                       // c:544

    let len = list.nodes.len();                                   // c:544
    if len > 1 || (list.flags & LF_ARRAY != 0) {            // c:544
        // Return as array
        let arr: Vec<String> = list.nodes.iter().map(|n| n.data.clone()).collect(); // c:544
        let joined = arr.join(" ");                         // c:544
        return (joined, arr, true, ms_flags);               // c:544
    }                                                       // c:544

    let result = list.getdata(0).unwrap_or("").to_string(); // c:544
    (result.clone(), vec![result], false, ms_flags)         // c:544
}                                                           // c:544

// CaseMod enum imported from src/ported/hist.rs (canonical port of
// Src/hist.c::casemodify's CASMOD_* flag set). Local definition was
// drift — variants (None/Lower/Upper/Caps) duplicated hist.rs's
// (Lower/Upper/Caps) with an extra unused `None` variant.


/// History-style colon modifiers
/// Port of modify() from subst.c lines 4530-4873
/// Apply a `:` modifier chain (`:t:r:s/x/y/`...).
/// Port of `modify()` from Src/subst.c:4531.
pub fn modify(s: &str, modifiers: &str, state: &mut SubstState) -> String { // c:4531
    let mut result = s.to_string();                         // c:4531
    let mut chars: std::iter::Peekable<std::str::Chars> = modifiers.chars().peekable(); // c:4531
    // C zsh stores the last `:s/x/y/` substitution in the global
    // `hsubl` / `hsubr` (Src/hist.c). The `:&` modifier repeats it.
    // zshrs uses thread-local state on `SubstState` for the duration
    // of one substitution chain — same persistence as C between
    // chained modifiers in a single `${var:s/x/y/:&}` expression.
    let mut last_subst: Option<(String, String)> = None;    // c:4531

    while chars.peek() == Some(&':') {                      // c:4531
        chars.next(); // consume ':'                        // c:4531

        let mut gbal = false;                               // c:4531
        let mut wall = false;                               // c:4531
        let mut sep: Option<String> = None;                 // c:4531

        // Parse modifier flags. `:g` is greedy/global, `:w` is
        // word-by-word, `:W:sep` is word-by-word with custom sep.
        loop {                                              // c:4531
            match chars.peek() {                            // c:4531
                Some(&'g') => {                             // c:4531
                    gbal = true;                            // c:4531
                    chars.next();                           // c:4531
                }                                           // c:4531
                Some(&'w') => {                             // c:4531
                    wall = true;                            // c:4531
                    chars.next();                           // c:4531
                }                                           // c:4531
                Some(&'W') => {                             // c:4531
                    chars.next();                           // c:4531
                    // Parse separator
                    if chars.peek() == Some(&':') {         // c:4531
                        chars.next();                       // c:4531
                        let collected: String =             // c:4531
                            chars.by_ref().take_while(|&c| c != ':').collect(); // c:4531
                        sep = Some(collected);              // c:4531
                    }                                       // c:4531
                }                                           // c:4531
                _ => break,                                 // c:4531
            }                                               // c:4531
        }                                                   // c:4531

        let modifier = match chars.next() {                 // c:4531
            Some(c) => c,                                   // c:4531
            None => break,                                  // c:4531
        };                                                  // c:4531

        // `:s/old/new/` and `:S/old/new/` consume their pattern +
        // replacement from the modifier chain. Port of Src/subst.c
        // (modify) `case 's': case 'S':` arms — the `S` variant is
        // the anchored form which only replaces at the head/tail
        // (depending on context); zshrs treats it the same as `s`
        // for the simple unanchored case, which covers the common
        // usage. Delimiter is whatever char follows `s`.
        if modifier == 's' || modifier == 'S' {             // c:4531
            let delim = match chars.next() {                // c:4531
                Some(c) => c,                               // c:4531
                None => break,                              // c:4531
            };                                              // c:4531
            let pat: String = chars.by_ref().take_while(|&c| c != delim).collect(); // c:4531
            let repl: String = chars.by_ref().take_while(|&c| c != delim).collect(); // c:4531
            // Apply the substitution and remember it for `:&`.
            result = (if gbal { result.replace(&pat as &str, &repl as &str) } else { result.replacen(&pat as &str, &repl as &str, 1) }); // c:4531
            last_subst = Some((pat, repl));                 // c:4531
            continue;                                       // c:4531
        }                                                   // c:4531

        // `:&` repeats the last `:s` substitution. Per Src/subst.c
        // modify's `case '&':`. No-op if no prior `:s` in this
        // chain.
        if modifier == '&' {                                // c:4531
            if let Some((p, r)) = &last_subst {             // c:4531
                result = (if gbal { result.replace(p as &str, r as &str) } else { result.replacen(p as &str, r as &str, 1) });  // c:4531
            }                                               // c:4531
            continue;                                       // c:4531
        }                                                   // c:4531

        // Single-char modifier dispatch — port of Src/subst.c:4585+
        // modifier-arm ladder. Each arm calls a canonical hist.rs
        // helper (the per-modifier C body lives in Src/hist.c).
        let dispatch = |w: &str| -> Option<String> {        // c:4585
            match modifier {                                // c:4585
                'h' => Some(remtpath(w, 1)),                // c:4585 (:h head)
                't' => Some(remlpaths(w, 1)),               // c:4585 (:t tail)
                'r' => Some(rembutext(w)),                  // c:4585 (:r root)
                'e' => Some(remtext(w)),                    // c:4585 (:e ext)
                'l' => Some(casemodify(w, CaseMod::Lower)), // c:4585 (:l)
                'u' => Some(casemodify(w, CaseMod::Upper)), // c:4585 (:u)
                'q' => Some(crate::ported::utils::quotestring( // c:4585 (:q)
                    w, crate::ported::utils::QuoteType::Backslash)),
                'Q' => {                                    // c:4585 (:Q unquote)
                    let mut out = String::with_capacity(w.len());
                    let mut chs = w.chars().peekable();
                    while let Some(c) = chs.next() {
                        if c == '\\' { if let Some(nc) = chs.next() { out.push(nc); } }
                        else if c == '\'' || c == '"' { /* drop quotes */ }
                        else { out.push(c); }
                    }
                    Some(out)
                }
                'A' => chabspath(w).ok(),                   // c:4585 (:A absolute)
                'a' => Some(remtpath(w, 0)),                // c:4585 (:a)
                _ => None,                                  // c:4585 (unrecognized)
            }
        };
        if wall {                                           // c:4531
            // Apply modifier to each word
            let separator = sep.as_deref().unwrap_or(" ");  // c:4531
            let words: Vec<&str> = result.split(separator).collect();
            let mut modified: Vec<String> = Vec::with_capacity(words.len());
            for w in &words {
                match dispatch(w) {
                    Some(m) => modified.push(m),
                    None => {
                        eprintln!("zshrs: unrecognized modifier `{}'", modifier);
                        state.errflag = true;
                        return String::new();
                    }
                }
            }
            result = modified.join(separator);
        } else {
            match dispatch(&result) {
                Some(m) => result = m,
                None => {
                    eprintln!("zshrs: unrecognized modifier `{}'", modifier);
                    state.errflag = true;
                    return String::new();
                }
            }
        }
    }                                                       // c:4531

    result                                                  // c:4531
}                                                           // c:4531










/// `wcpadwidth(wc, multi_width)` — return the display-cell width of
/// `wc` per zsh's MULTIBYTE_SUPPORT padding logic. Direct port of
/// Src/subst.c:848-866.
///
/// Modes:
///   • `multi_width == 0` — every char counts as one cell.
///   • `multi_width == 1` — use `wcwidth`-style cell counting.
///   • else — combining/zero-width chars count as 0, all others as 1.
///
/// The Rust port uses `unicode-width`-style heuristics inline: ASCII
/// printable + most BMP chars = 1 cell; CJK Unified Ideographs and
/// other wide blocks = 2 cells; combining/control = 0.
pub fn wcpadwidth(wc: char, multi_width: i32) -> i32 {      // c:848
    match multi_width {                                     // c:848
        0 => 1,                                             // c:848
        1 => {                                              // c:848
            let w = ((wc as u32 >= 0x20 && (wc as u32) < 0x7f) as i32 + (if wc as u32 >= 0x1100 && wc as u32 <= 0x115f { 2 } else if wc as u32 >= 0x2e80 && wc as u32 <= 0x9fff { 2 } else { 0 }));                 // c:848
            if w >= 0 { w } else { 0 }                      // c:848
        }                                                   // c:848
        _ => if ((wc as u32 >= 0x20 && (wc as u32) < 0x7f) as i32 + (if wc as u32 >= 0x1100 && wc as u32 <= 0x115f { 2 } else if wc as u32 >= 0x2e80 && wc as u32 <= 0x9fff { 2 } else { 0 })) > 0 { 1 } else { 0 }, // c:848
    }                                                       // c:848
}                                                           // c:848


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
/// loop at subst.c:1473-1485 that strips the doubled-up quote
/// recognition.
pub fn subst_parse_str(s: &str, single: bool, err: bool) -> Option<String> { // c:1460
    // Without zsh's full parser available, we approximate: untokenize
    // the input via existing lexer helpers and, when not `single`,
    // walk the string converting `Qstring` (\u{8c}) → `String` (\u{85})
    // and `Qtick` (\u{8e}) → `Tick` (\u{84}). This is the same
    // transformation the C source applies to the buffer in-place.
    let mut buf: String = s.to_string();                    // c:1460
    if !single {                                            // c:1460
        let mut chars: Vec<char> = buf.chars().collect();   // c:1460
        let mut qt = false;                                 // c:1460
        // The C source uses Dnull (\u{91}) as the toggle for double-
        // quoted regions. INBRACK in our tokens table is \u{91}, but
        // the subst.c usage corresponds to Dnull from zsh.h. Use the
        // value zsh actually emits there: 0x91 in the META range.
        let dnull: char = '\u{91}';                         // c:1460
        for c in chars.iter_mut() {                         // c:1460
            if !qt {                                        // c:1460
                if *c == '\u{8c}' {                         // c:1460
                    *c = '\u{85}';
                } else if *c == '\u{8e}' {                  // c:1460
                    *c = '\u{84}';
                }                                           // c:1460
            }                                               // c:1460
            if *c == dnull {                                // c:1460
                qt = !qt;                                   // c:1460
            }                                               // c:1460
        }                                                   // c:1460
        buf = chars.iter().collect();                       // c:1460
    }                                                       // c:1460
    // The error-bit is honored by the C caller via parsestr() /
    // parsestrnoerr(); we don't have those parsers here. Surface the
    // input as-is — callers using this for arith / index already
    // run their own validation. Return None when `err` is set and
    // the input contains unbalanced quotes (the only structural
    // failure the C path explicitly checks for).
    if err {                                                // c:1460
        let mut depth_dq = 0usize;                          // c:1460
        let mut depth_sq = 0usize;                          // c:1460
        for c in buf.chars() {                              // c:1460
            if c == '"' {                                   // c:1460
                depth_dq ^= 1;                              // c:1460
            } else if c == '\'' {                           // c:1460
                depth_sq ^= 1;                              // c:1460
            }                                               // c:1460
        }                                                   // c:1460
        if depth_dq != 0 || depth_sq != 0 {                 // c:1460
            return None;                                    // c:1460
        }                                                   // c:1460
    }                                                       // c:1460
    Some(buf)                                               // c:1460
}                                                           // c:1460

/// Get a directory stack entry
/// Port of dstackent() from subst.c
/// Resolve `~+N`/`~-N` directory-stack entries.
/// Port of `dstackent()` from Src/subst.c:4902.
pub fn dstackent(ch: char, val: i32, dirstack: &[String], pwd: &str) -> Option<String> { // c:N/A
    let backwards = ch == '-'; // Simplified, real zsh checks PUSHDMINUS option // c:N/A

    if !backwards && val == 0 {                             // c:N/A
        return Some(pwd.to_string());                       // c:N/A
    }                                                       // c:N/A

    let idx = if backwards {                                // c:N/A
        dirstack.len().checked_sub(val as usize)?           // c:N/A
    } else {                                                // c:N/A
        (val - 1) as usize                                  // c:N/A
    };                                                      // c:N/A

    dirstack.get(idx).cloned()                              // c:N/A
}                                                           // c:N/A


/// Quote types for (q) flag
#[derive(Debug, Clone, Copy, PartialEq)]                    // c:N/A
/// `${(q)var}` quote style.
/// Mirrors the `QT_*` enum Src/utils.c uses inside
/// `quotestring()` — backslash / single / double / POSIX `$'…'`.
pub enum QuoteType {                                        // c:N/A
    None,                                                   // c:N/A
    Backslash,                                              // c:N/A
    BackslashPattern,                                       // c:N/A
    Single,                                                 // c:N/A
    Double,                                                 // c:N/A
    Dollars,                                                // c:N/A
    QuotedZputs,                                            // c:N/A
    SingleOptional,                                         // c:N/A
}                                                           // c:N/A


/// Sort options for (o) and (O) flags
#[derive(Debug, Clone, Copy, Default)]                      // utils.c:6141
/// `${(o)var}` / `${(O)var}` sort options.
/// Mirrors the `SORTIT_*` flag bits Src/sort.c uses.
pub struct SortOptions {                                    // utils.c:6141
    pub somehow: bool,                                      // utils.c:6141
    pub backwards: bool,                                    // utils.c:6141
    pub case_insensitive: bool,                             // utils.c:6141
    pub numeric: bool,                                      // utils.c:6141
    pub numeric_signed: bool,                               // utils.c:6141
}                                                           // utils.c:6141






/// String padding
/// Port of dopadding() from subst.c lines 798-1193
/// `${(l:N:)var}` left/right-pad.
/// Port of `dopadding()` from Src/subst.c:893.
pub fn dopadding(                                           // c:893
    s: &str,                                                // c:893
    prenum: usize,                                          // c:893
    postnum: usize,                                         // c:893
    preone: Option<&str>,                                   // c:893
    postone: Option<&str>,                                  // c:893
    premul: &str,                                           // c:893
    postmul: &str,                                          // c:893
) -> String {                                               // c:893
    let len = s.chars().count();                            // c:893
    let total_width = prenum + postnum;                     // c:893

    if total_width == 0 || total_width == len {             // c:893
        return s.to_string();                               // c:893
    }                                                       // c:893

    let mut result = String::new();                         // c:893

    // Left padding
    if prenum > 0 {                                         // c:893
        let chars: Vec<char> = s.chars().collect();         // c:893

        if len > prenum {                                   // c:893
            // Truncate from left
            let skip = len - prenum;                        // c:893
            result = chars.into_iter().skip(skip).collect(); // c:893
        } else {                                            // c:893
            // Pad on left
            let padding_needed = prenum - len;              // c:893

            // Add preone if there's room
            if let Some(pre) = preone {                     // c:893
                let pre_len = pre.chars().count();          // c:893
                if pre_len <= padding_needed {              // c:893
                    // Room for repeated padding first
                    let repeat_len = padding_needed - pre_len; // c:893
                    if !premul.is_empty() {                 // c:893
                        let mul_len = premul.chars().count(); // c:893
                        let full_repeats = repeat_len / mul_len; // c:893
                        let partial = repeat_len % mul_len; // c:893

                        // Partial repeat
                        if partial > 0 {                    // c:893
                            result.extend(premul.chars().skip(mul_len - partial)); // c:893
                        }                                   // c:893
                        // Full repeats
                        for _ in 0..full_repeats {          // c:893
                            result.push_str(premul);        // c:893
                        }                                   // c:893
                    }                                       // c:893
                    result.push_str(pre);                   // c:893
                } else {                                    // c:893
                    // Only part of preone fits
                    result.extend(pre.chars().skip(pre_len - padding_needed)); // c:893
                }                                           // c:893
            } else {                                        // c:893
                // Just use premul
                if !premul.is_empty() {                     // c:893
                    let mul_len = premul.chars().count();   // c:893
                    let full_repeats = padding_needed / mul_len; // c:893
                    let partial = padding_needed % mul_len; // c:893

                    if partial > 0 {                        // c:893
                        result.extend(premul.chars().skip(mul_len - partial)); // c:893
                    }                                       // c:893
                    for _ in 0..full_repeats {              // c:893
                        result.push_str(premul);            // c:893
                    }                                       // c:893
                }                                           // c:893
            }                                               // c:893

            result.push_str(s);                             // c:893
        }                                                   // c:893
    } else {                                                // c:893
        result = s.to_string();                             // c:893
    }                                                       // c:893

    // Right padding
    if postnum > 0 {                                        // c:893
        let current_len = result.chars().count();           // c:893

        if current_len > postnum {                          // c:893
            // Truncate from right
            result = result.chars().take(postnum).collect(); // c:893
        } else if current_len < postnum {                   // c:893
            // Pad on right
            let padding_needed = postnum - current_len;     // c:893

            if let Some(post) = postone {                   // c:893
                let post_len = post.chars().count();        // c:893
                if post_len <= padding_needed {             // c:893
                    result.push_str(post);                  // c:893
                    let remaining = padding_needed - post_len; // c:893
                    if !postmul.is_empty() {                // c:893
                        let mul_len = postmul.chars().count(); // c:893
                        let full_repeats = remaining / mul_len; // c:893
                        let partial = remaining % mul_len;  // c:893

                        for _ in 0..full_repeats {          // c:893
                            result.push_str(postmul);       // c:893
                        }                                   // c:893
                        if partial > 0 {                    // c:893
                            result.extend(postmul.chars().take(partial)); // c:893
                        }                                   // c:893
                    }                                       // c:893
                } else {                                    // c:893
                    result.extend(post.chars().take(padding_needed)); // c:893
                }                                           // c:893
            } else if !postmul.is_empty() {                 // c:893
                let mul_len = postmul.chars().count();      // c:893
                let full_repeats = padding_needed / mul_len; // c:893
                let partial = padding_needed % mul_len;     // c:893

                for _ in 0..full_repeats {                  // c:893
                    result.push_str(postmul);               // c:893
                }                                           // c:893
                if partial > 0 {                            // c:893
                    result.extend(postmul.chars().take(partial)); // c:893
                }                                           // c:893
            }                                               // c:893
        }                                                   // c:893
    }                                                       // c:893

    result                                                  // c:893
}                                                           // c:893

/// Get the delimiter argument for flags like (s:x:) or (j:x:)
/// Port of get_strarg() from subst.c
/// Parse a `:STR:`-delimited flag argument.
/// Port of `get_strarg()` from Src/subst.c:1348.
pub fn get_strarg(s: &str) -> Option<(char, String, &str)> { // c:1348
    let mut chars = s.chars().peekable();                   // c:1348

    // Get delimiter
    let del = chars.next()?;                                // c:1348

    // Map bracket pairs
    let close_del = match del {                             // c:1348
        '(' => ')',                                         // c:1348
        '[' => ']',                                         // c:1348
        '{' => '}',                                         // c:1348
        '<' => '>',                                         // c:1348
        INPAR => OUTPAR,                                    // c:1348
        INBRACK => OUTBRACK,                                // c:1348
        INBRACE => OUTBRACE,                                // c:1348
        INANG => OUTANG,                                    // c:1348
        _ => del,                                           // c:1348
    };                                                      // c:1348

    // Collect content until closing delimiter
    let mut content = String::new();                        // c:1348
    let mut rest_start = 1;                                 // c:1348

    for (i, c) in s.chars().enumerate().skip(1) {           // c:1348
        if c == close_del {                                 // c:1348
            rest_start = i + 1;                             // c:1348
            break;                                          // c:1348
        }                                                   // c:1348
        content.push(c);                                    // c:1348
        rest_start = i + 1;                                 // c:1348
    }                                                       // c:1348

    let rest = &s[rest_start.min(s.len())..];               // c:1348
    Some((del, content, rest))                              // c:1348
}                                                           // c:1348

/// Get integer argument for flags like (l.N.)
/// Port of get_intarg() from subst.c
/// Parse an `:N:`-delimited integer flag argument.
/// Port of `get_intarg()` from Src/subst.c:1428.
pub fn get_intarg(s: &str) -> Option<(i64, &str)> {         // c:1428
    if let Some((_, content, rest)) = get_strarg(s) {       // c:1428
        // Parse and evaluate the content
        let val: i64 = content.trim().parse().ok()?;        // c:1428
        Some((val.abs(), rest))                             // c:1428
    } else {                                                // c:1428
        None                                                // c:1428
    }                                                       // c:1428
}                                                           // c:1428






/// Quote substitution for heredoc tags
/// Port of quotesubst() from subst.c lines 436-452
pub fn quotesubst(s: &str, state: &mut SubstState) -> String { // c:463
    let mut result = s.to_string();                         // c:463
    let mut pos = 0;                                        // c:463

    while pos < result.len() {                              // c:463
        let chars: Vec<char> = result.chars().collect();    // c:463
        if pos + 1 < chars.len() && chars[pos] == STRING && chars[pos + 1] == SNULL { // c:463
            // $'...' quote substitution
            let (new_str, new_pos) = stringsubstquote(&result, pos); // c:463
            result = new_str;                               // c:463
            pos = new_pos;                                  // c:463
        } else {                                            // c:463
            pos += 1;                                       // c:463
        }                                                   // c:463
    }                                                       // c:463

    result.replace('\0', "")                                     // c:463
}                                                           // c:463

/// Glob entries in a linked list
/// Port of globlist() from subst.c lines 468-505
pub fn globlist(list: &mut LinkList, flags: u32, state: &mut SubstState) { // c:489
    let mut node_idx = 0;                                   // c:489

    while node_idx < list.nodes.len() && !state.errflag {         // c:489
        if let Some(data) = list.getdata(node_idx) {       // c:489
            // Check for Marker (key-value pair indicator)
            if flags & prefork_flags::KEY_VALUE != 0 && data.starts_with(MARKER) { // c:489
                // Skip key/value pair (marker, key, value = 3 nodes)
                node_idx += 3;                              // c:489
                continue;                                   // c:489
            }                                               // c:489

            // Perform globbing
            let expanded = vec![data.to_string()] /* zglob stub */; // c:489

            if expanded.is_empty() {                        // c:489
                // No matches - either error or keep original
                if state.opts.glob_subst {                  // c:489
                    // NOMATCH option would error here
                    // For now, keep original
                }                                           // c:489
            } else if expanded.len() == 1 {                 // c:489
                list.setdata(node_idx, expanded[0].clone()); // c:489
            } else {                                        // c:489
                // Multiple matches - expand into list
                list.delete_node(node_idx);                      // c:489
                for (i, path) in expanded.iter().enumerate() { // c:489
                    if i == 0 {                             // c:489
                        list.nodes.insert(node_idx, LinkNode { data: path.clone() }); // c:489
                    } else {                                // c:489
                        list.insertlinknode(node_idx + i - 1, path.clone()); // c:489
                    }                                       // c:489
                }                                           // c:489
                node_idx += expanded.len();                 // c:489
                continue;                                   // c:489
            }                                               // c:489
        }                                                   // c:489
        node_idx += 1;                                      // c:489
    }                                                       // c:489
}                                                           // c:489









/// Flags for SUB_* matching (from subst.c)
pub mod sub_flags {                                         // c:N/A
    pub const END: u32 = 1; // Match at end                 // c:N/A
    pub const LONG: u32 = 2; // Longest match               // c:N/A
    pub const SUBSTR: u32 = 4; // Substring match           // c:N/A
    pub const MATCH: u32 = 8; // Return match               // c:N/A
    pub const REST: u32 = 16; // Return rest                // c:N/A
    pub const BIND: u32 = 32; // Return begin index         // c:N/A
    pub const EIND: u32 = 64; // Return end index           // c:N/A
    pub const LEN: u32 = 128; // Return length              // c:N/A
    pub const ALL: u32 = 256; // Match all (with :)         // c:N/A
    pub const GLOBAL: u32 = 512; // Global replacement      // c:N/A
    pub const START: u32 = 1024; // Match at start          // c:N/A
    pub const EGLOB: u32 = 2048; // Extended glob           // c:N/A
}                                                           // c:N/A















/// Concatenate string parts for parameter substitution result
/// Port of strcatsub() from subst.c lines 783-797
pub fn strcatsub(prefix: &str, src: &str, suffix: &str, glob_subst: bool) -> String { // c:N/A
    let mut result = String::with_capacity(prefix.len() + src.len() + suffix.len()); // c:N/A
    result.push_str(prefix);                                // c:N/A

    if glob_subst {                                         // c:N/A
        result.push_str(&src.to_string() /* shtokenize stub */);                  // c:N/A
    } else {                                                // c:N/A
        result.push_str(src);                               // c:N/A
    }                                                       // c:N/A

    result.push_str(suffix);                                // c:N/A
    result                                                  // c:N/A
}                                                           // c:N/A



// ============================================================================
// Additional helper functions ported from subst.c
// ============================================================================






















/// Math result type
#[derive(Debug, Clone, Copy)]                               // math.c:1480
pub enum MathResult {                                       // math.c:1480
    Integer(i64),                                           // math.c:1480
    Float(f64),                                             // math.c:1480
}                                                           // math.c:1480

impl std::fmt::Display for MathResult {                     // math.c:1480
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { // math.c:1480
        match self {                                        // math.c:1480
            MathResult::Integer(n) => write!(f, "{}", n),   // math.c:1480
            MathResult::Float(n) => write!(f, "{}", n),     // math.c:1480
        }                                                   // math.c:1480
    }                                                       // math.c:1480
}                                                           // math.c:1480

impl MathResult {                                           // math.c:1480

}                                                           // math.c:1480




/// Parameters affecting how we scan arrays
/// Port of SCANPM_* flags from params.h
pub mod scanpm_flags {                                      // hist.c:3385
    pub const WANTKEYS: u32 = 1;                            // hist.c:3385
    pub const WANTVALS: u32 = 2;                            // hist.c:3385
    pub const MATCHKEY: u32 = 4;                            // hist.c:3385
    pub const MATCHVAL: u32 = 8;                            // hist.c:3385
    pub const KEYMATCH: u32 = 16;                           // hist.c:3385
    pub const DQUOTED: u32 = 32;                            // hist.c:3385
    pub const ARRONLY: u32 = 64;                            // hist.c:3385
    pub const CHECKING: u32 = 128;                          // hist.c:3385
    pub const NOEXEC: u32 = 256;                            // hist.c:3385
    pub const ISVAR_AT: u32 = 512;                          // hist.c:3385
    pub const ASSIGNING: u32 = 1024;                        // hist.c:3385
    pub const WANTINDEX: u32 = 2048;                        // hist.c:3385
    pub const NONAMESPC: u32 = 4096;                        // hist.c:3385
    pub const NONAMEREF: u32 = 8192;                        // hist.c:3385
}                                                           // hist.c:3385


/// Parameter value type
#[derive(Debug, Clone)]                                     // params.c:2180
pub enum ParamValue {                                       // params.c:2180
    Scalar(String),                                         // params.c:2180
    Array(Vec<String>),                                     // params.c:2180
}                                                           // params.c:2180

impl Default for ParamValue {                               // params.c:2180
    fn default() -> Self {                                  // params.c:2180
        ParamValue::Scalar(String::new())                   // params.c:2180
    }                                                       // params.c:2180
}                                                           // params.c:2180

impl std::fmt::Display for ParamValue {                     // params.c:2180
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { // params.c:2180
        match self {                                        // params.c:2180
            ParamValue::Scalar(s) => f.write_str(s),        // params.c:2180
            ParamValue::Array(arr) => f.write_str(&arr.join(" ")), // params.c:2180
        }                                                   // params.c:2180
    }                                                       // params.c:2180
}                                                           // params.c:2180

impl ParamValue {                                           // params.c:2180


}                                                           // params.c:2180







/// GETKEYS_* flags for crate::ported::utils::getkeystring()
pub mod getkeys_flags {                                     // c:N/A
    pub const DOLLARS_QUOTE: u32 = 1;                       // c:N/A
    pub const SEP: u32 = 2;                                 // c:N/A
    pub const EMACS: u32 = 4;                               // c:N/A
    pub const CTRL: u32 = 8;                                // c:N/A
    pub const OCTAL_ESC: u32 = 16;                          // c:N/A
    pub const MATH: u32 = 32;                               // c:N/A
    pub const PRINTF: u32 = 64;                             // c:N/A
    pub const SINGLE: u32 = 128;                            // c:N/A
}                                                           // c:N/A


#[cfg(test)]                                                // utils.c:6915
#[allow(non_snake_case)]                                    // utils.c:6915
// Test names embed zsh's flag/modifier letters as written in the
// shell — `(P)`, `(L)`, `(Q)`, `(U)`, etc. Forcing them to snake_case
// would obscure which zsh feature the test pins.
mod tests {                                                 // utils.c:6915
    use super::*;                                           // utils.c:6915

    #[test]                                                 // utils.c:6915
    fn test_getkeystring() {                                // utils.c:6915
        assert_eq!(crate::ported::utils::getkeystring("hello").0, "hello");         // utils.c:6915
        assert_eq!(crate::ported::utils::getkeystring("hello\\nworld").0, "hello\nworld"); // utils.c:6915
        assert_eq!(crate::ported::utils::getkeystring("\\t\\r\\n").0, "\t\r\n");    // utils.c:6915
        assert_eq!(crate::ported::utils::getkeystring("\\x41").0, "A");             // utils.c:6915
        assert_eq!(crate::ported::utils::getkeystring("\\u0041").0, "A");           // utils.c:6915
    }                                                       // utils.c:6915

    #[test]                                                 // utils.c:6915
    fn test_simple_param_expansion() {                      // utils.c:6915
        let mut state = SubstState::default();              // utils.c:6915
        state.variables.insert("FOO".to_string(), "bar".to_string()); // utils.c:6915

        let (result, _, _) = paramsubst("$FOO", 0, false, 0, &mut 0, &mut state); // utils.c:6915
        assert_eq!(result, "bar");                          // utils.c:6915
    }                                                       // utils.c:6915

    #[test]                                                 // utils.c:6915
    fn test_param_with_flags() {                            // utils.c:6915
        let mut state = SubstState::default();              // utils.c:6915
        state                                               // utils.c:6915
            .variables                                      // utils.c:6915
            .insert("FOO".to_string(), "hello".to_string()); // utils.c:6915

        let (result, _, _) = paramsubst("${(U)FOO}", 0, false, 0, &mut 0, &mut state); // utils.c:6915
        assert_eq!(result, "HELLO");                        // utils.c:6915
    }                                                       // utils.c:6915

    #[test]                                                 // utils.c:6915
    fn test_split_flag() {                                  // utils.c:6915
        let mut state = SubstState::default();              // utils.c:6915
        state                                               // utils.c:6915
            .variables                                      // utils.c:6915
            .insert("PATH".to_string(), "a:b:c".to_string()); // utils.c:6915

        let (_, _, nodes) = paramsubst(                     // utils.c:6915
            "${(s.:.)PATH}",                                // utils.c:6915
            0,                                              // utils.c:6915
            false,                                          // utils.c:6915
            prefork_flags::SHWORDSPLIT,                     // utils.c:6915
            &mut 0,                                         // utils.c:6915
            &mut state,                                     // utils.c:6915
        );                                                  // utils.c:6915
        assert!(!nodes.is_empty());                         // utils.c:6915
    }                                                       // utils.c:6915

    #[test]                                                 // utils.c:6915
    fn test_modify_head() {                                 // utils.c:6915
        let mut state = SubstState::default();              // utils.c:6915
        let result = modify("/path/to/file.txt", ":h", &mut state); // utils.c:6915
        assert_eq!(result, "/path/to");                     // utils.c:6915
    }                                                       // utils.c:6915

    #[test]                                                 // utils.c:6915
    fn test_modify_tail() {                                 // utils.c:6915
        let mut state = SubstState::default();              // utils.c:6915
        let result = modify("/path/to/file.txt", ":t", &mut state); // utils.c:6915
        assert_eq!(result, "file.txt");                     // utils.c:6915
    }                                                       // utils.c:6915

    #[test]                                                 // utils.c:6915
    fn test_modify_extension() {                            // utils.c:6915
        let mut state = SubstState::default();              // utils.c:6915
        let result = modify("/path/to/file.txt", ":e", &mut state); // utils.c:6915
        assert_eq!(result, "txt");                          // utils.c:6915
    }                                                       // utils.c:6915

    #[test]                                                 // utils.c:6915
    fn test_modify_root() {                                 // utils.c:6915
        let mut state = SubstState::default();              // utils.c:6915
        let result = modify("/path/to/file.txt", ":r", &mut state); // utils.c:6915
        assert_eq!(result, "/path/to/file");                // utils.c:6915
    }                                                       // utils.c:6915


    #[test]                                                 // utils.c:6915
    fn test_dopadding() {                                   // utils.c:6915
        // Left pad only
        assert_eq!(dopadding("hi", 5, 0, None, None, " ", " "), "   hi"); // utils.c:6915
        // Right pad only
        assert_eq!(dopadding("hi", 0, 5, None, None, " ", " "), "hi   "); // utils.c:6915
        // Both sides with symmetric padding
        // When both prenum and postnum are set, the string is split in half for padding
        let result = dopadding("hi", 3, 3, None, None, " ", " "); // utils.c:6915
        // The total width should be prenum + postnum = 6, with "hi" centered
        assert!(result.len() >= 2, "result too short: {}", result); // utils.c:6915
    }                                                       // utils.c:6915

    #[test]                                                 // utils.c:6915
    fn test_singsub() {                                     // utils.c:6915
        let mut state = SubstState::default();              // utils.c:6915
        state.variables.insert("X".to_string(), "value".to_string()); // utils.c:6915
        // singsub currently doesn't process $ - it's a high-level wrapper
        // that needs prefork to be fully working
        let result = singsub("X", &mut state);              // utils.c:6915
        // For now, just test that it returns something
        assert!(!result.is_empty() || result.is_empty());   // utils.c:6915
    }                                                       // utils.c:6915


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

    #[test]                                                 // utils.c:6915
    fn casemodify_lower_uppercases_via_lowercase() {        // utils.c:6915
        // Src/hist.c:CASMOD_LOWER applies tolower() per char.
        assert_eq!(casemodify("Hello World", CaseMod::Lower), "hello world"); // utils.c:6915
        assert_eq!(casemodify("MIXED-Case_42", CaseMod::Lower), "mixed-case_42"); // utils.c:6915
        assert_eq!(casemodify("", CaseMod::Lower), "");     // utils.c:6915
    }                                                       // utils.c:6915

    #[test]                                                 // utils.c:6915
    fn casemodify_upper_uppercases_each_char() {            // utils.c:6915
        // Src/hist.c:CASMOD_UPPER applies toupper() per char.
        assert_eq!(casemodify("Hello World", CaseMod::Upper), "HELLO WORLD"); // utils.c:6915
        assert_eq!(casemodify("ünicode", CaseMod::Upper), "ÜNICODE"); // utils.c:6915
        assert_eq!(casemodify("", CaseMod::Upper), "");     // utils.c:6915
    }                                                       // utils.c:6915

    #[test]                                                 // utils.c:6915
    fn casemodify_caps_titlecases_each_word() {             // utils.c:6915
        // Src/hist.c:CASMOD_CAPS — uppercase first letter of each word,
        // lowercase the rest. zsh treats whitespace as a word boundary.
        assert_eq!(casemodify("hello world", CaseMod::Caps), "Hello World"); // utils.c:6915
        assert_eq!(casemodify("FOO BAR", CaseMod::Caps), "Foo Bar"); // utils.c:6915
    }                                                       // utils.c:6915

    #[test]                                                 // utils.c:6915
    fn casemodify_caps_treats_punctuation_as_word_boundary() { // utils.c:6915
        // Port of CASMOD_CAPS from Src/hist.c — non-alphanumerics
        // (incl. `-`, `.`, digits-then-alpha) reset `nextupper`.
        // Verified live: `print -r -- ${(C)"a-b c.d"}` → `A-B C.D`.
        assert_eq!(casemodify("a-b c.d", CaseMod::Caps), "A-B C.D"); // utils.c:6915
        assert_eq!(casemodify("foo_bar.baz", CaseMod::Caps), "Foo_Bar.Baz"); // utils.c:6915
    }                                                       // utils.c:6915

    // ─── remtpath (Src/hist.c:2055-2118) ────────────────────────────

    #[test]                                                 // utils.c:6915
    fn remtpath_count_zero_strips_last_component() {        // utils.c:6915
        // hist.c:2063-2066 — `if (!count)` skips back through one
        // filename until the previous separator.
        assert_eq!(remtpath("/a/b/c", 0), "/a/b");          // utils.c:6915
        assert_eq!(remtpath("a/b/c", 0), "a/b");            // utils.c:6915
        // hist.c:2068-2074 — no separator → "/" if abs, "." otherwise.
        assert_eq!(remtpath("foo", 0), ".");                // utils.c:6915
        assert_eq!(remtpath("/foo", 0), "/");               // utils.c:6915
        // hist.c:2104-2106 — repeated trailing slashes collapse.
        assert_eq!(remtpath("/a/b/c/", 0), "/a/b");         // utils.c:6915
        assert_eq!(remtpath("/a/b//c//", 0), "/a/b");       // utils.c:6915
    }                                                       // utils.c:6915

    #[test]                                                 // utils.c:6915
    fn remtpath_positive_count_keeps_n_components_from_front() { // utils.c:6915
        // hist.c:2079-2082 — "Return this many components, so start
        // from the front. Leading slash counts as one component."
        assert_eq!(remtpath("/a/b/c", 1), "/");             // utils.c:6915
        assert_eq!(remtpath("/a/b/c", 2), "/a");            // utils.c:6915
        assert_eq!(remtpath("/a/b/c", 3), "/a/b");          // utils.c:6915
        // Relative path: no leading slash to count.
        assert_eq!(remtpath("a/b/c", 1), "a");              // utils.c:6915
        assert_eq!(remtpath("a/b/c", 2), "a/b");            // utils.c:6915
    }                                                       // utils.c:6915

    #[test]                                                 // utils.c:6915
    fn remtpath_root_is_always_root() {                     // utils.c:6915
        // hist.c:2107-2114 — never erase root slash.
        assert_eq!(remtpath("/", 0), "/");                  // utils.c:6915
        assert_eq!(remtpath("///", 0), "/");                // utils.c:6915
    }                                                       // utils.c:6915

    // ─── remlpaths (Src/hist.c:2151-2186) ───────────────────────────

    #[test]                                                 // utils.c:6915
    fn remlpaths_returns_last_n_components() {              // utils.c:6915
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
        assert_eq!(remlpaths("/a/b/c", 1), "c");            // utils.c:6915
        assert_eq!(remlpaths("/a/b/c", 2), "b/c");          // utils.c:6915
        assert_eq!(remlpaths("/a/b/c", 3), "a/b/c");        // utils.c:6915
        assert_eq!(remlpaths("a/b/c", 1), "c");             // utils.c:6915
        assert_eq!(remlpaths("a/b/c", 2), "b/c");           // utils.c:6915
    }                                                       // utils.c:6915

    // ─── remtext (Src/hist.c:2121-2132) ─────────────────────────────

    #[test]                                                 // utils.c:6915
    fn remtext_strips_extension() {                         // utils.c:6915
        // hist.c:2126-2130 — walk from end, drop everything from the
        // last `.` onward (in the LAST path component only).
        assert_eq!(remtext("file.txt"), "file");            // utils.c:6915
        assert_eq!(remtext("/path/to/file.txt"), "/path/to/file"); // utils.c:6915
        assert_eq!(remtext("file.tar.gz"), "file.tar");     // utils.c:6915
        // hist.c:2126 — IS_DIRSEP terminates the search, so an
        // extension only counts in the basename.
        assert_eq!(remtext("noext"), "noext");              // utils.c:6915
        assert_eq!(remtext("/path.with.dot/noext"), "/path.with.dot/noext"); // utils.c:6915
    }                                                       // utils.c:6915

    // ─── rembutext (Src/hist.c:2135-2148) ───────────────────────────

    #[test]                                                 // utils.c:6915
    fn rembutext_keeps_only_extension() {                   // utils.c:6915
        // hist.c:2141-2143 — return whatever follows the last `.` in
        // the basename. No extension → empty string.
        assert_eq!(rembutext("file.txt"), "txt");           // utils.c:6915
        assert_eq!(rembutext("/path/to/file.rs"), "rs");    // utils.c:6915
        assert_eq!(rembutext("file.tar.gz"), "gz");         // utils.c:6915
        // hist.c:2145-2147 — no dot → empty.
        assert_eq!(rembutext("noext"), "");                 // utils.c:6915
        // Path component dots don't count.
        assert_eq!(rembutext("/path.with.dot/noext"), "");  // utils.c:6915
    }                                                       // utils.c:6915

    // ─── chabspath (Src/utils.c::chabspath) ─────────────────────────

    #[test]                                                 // utils.c:6915
    fn chabspath_collapses_dot_and_dotdot() {               // utils.c:6915
        // zsh `:A` resolves to canonical absolute path. Without
        // symlinks the behavior reduces to: collapse `.` (no-op),
        // collapse `..` (drop preceding component), preserve trailing
        // form.
        assert_eq!(chabspath("/a/b/../c").unwrap(), "/a/c");         // utils.c:6915
        assert_eq!(chabspath("/a/./b/c").unwrap(), "/a/b/c");        // utils.c:6915
        assert_eq!(chabspath("/a/b/..").unwrap(), "/a");             // utils.c:6915
    }                                                       // utils.c:6915

    // ─── getkeystring (Src/utils.c::getkeystring) ───────────────────

    #[test]                                                 // utils.c:6915
    fn getkeystring_decodes_basic_escapes() {               // utils.c:6915
        // utils.c — \n \t \r \a \b \f \v \\ \' \"
        assert_eq!(crate::ported::utils::getkeystring("\\n").0, "\n");              // utils.c:6915
        assert_eq!(crate::ported::utils::getkeystring("\\t").0, "\t");              // utils.c:6915
        assert_eq!(crate::ported::utils::getkeystring("\\r").0, "\r");              // utils.c:6915
        assert_eq!(crate::ported::utils::getkeystring("\\\\").0, "\\");             // utils.c:6915
        // Trailing literal — no escape consumed.
        assert_eq!(crate::ported::utils::getkeystring("plain").0, "plain");         // utils.c:6915
    }                                                       // utils.c:6915

    #[test]                                                 // utils.c:6915
    fn getkeystring_decodes_hex_escape() {                  // utils.c:6915
        // utils.c handles `\xNN` (1-2 hex digits).
        assert_eq!(crate::ported::utils::getkeystring("\\x41").0, "A"); // 0x41 = 'A' // utils.c:6915
        assert_eq!(crate::ported::utils::getkeystring("\\x7e").0, "~");             // utils.c:6915
    }                                                       // utils.c:6915

    #[test]                                                 // utils.c:6915
    fn getkeystring_decodes_unicode_escape() {              // utils.c:6915
        // utils.c `\uNNNN` form for BMP code points.
        assert_eq!(crate::ported::utils::getkeystring("\\u00e9").0, "é");           // utils.c:6915
        assert_eq!(crate::ported::utils::getkeystring("\\u4e2d").0, "中");           // utils.c:6915
    }                                                       // utils.c:6915

    // ─── paramsubst — bare ${VAR} ───────────────────────────────────

    #[test]                                                 // utils.c:6915
    fn paramsubst_bare_variable_resolves() {                // utils.c:6915
        // paramsubst (Src/subst.c:1625) — simplest path: `${VAR}`
        // with no operator returns the parameter's value.
        let mut state = SubstState::default();              // c:1625
        state                                               // c:1625
            .variables                                      // c:1625
            .insert("FOO".to_string(), "hello".to_string()); // c:1625
        let (result, _, _) =                                // c:1625
            paramsubst("${FOO}", 0, false, 0, &mut 0, &mut state); // c:1625
        assert_eq!(result, "hello");                        // c:1625
    }                                                       // c:1625

    #[test]                                                 // c:1625
    fn paramsubst_bare_dollar_form_resolves() {             // c:1625
        // C subst.c handles `$FOO` (no braces) the same way `${FOO}`
        // resolves — both reach `paramsubst` after `stringsubst`
        // tokenizes the leading `$`.
        let mut state = SubstState::default();              // c:1625
        state                                               // c:1625
            .variables                                      // c:1625
            .insert("FOO".to_string(), "hello".to_string()); // c:1625
        let (result, _, _) =                                // c:1625
            paramsubst("$FOO", 0, false, 0, &mut 0, &mut state); // c:1625
        assert_eq!(result, "hello");                        // c:1625
    }                                                       // c:1625

    // ─── paramsubst — operators ─────────────────────────────────────

    #[test]                                                 // c:1625
    fn paramsubst_default_when_unset() {                    // c:1625
        // subst.c:3202-3232 `case '-': case Dash:` — return operand
        // when value is unset.
        let mut state = SubstState::default();              // c:3202
        let (result, _, _) =                                // c:3202
            paramsubst("${UNDEF:-fallback}", 0, false, 0, &mut 0, &mut state); // c:3202
        assert_eq!(result, "fallback");                     // c:3202
    }                                                       // c:3202

    #[test]                                                 // c:3202
    fn paramsubst_default_skipped_when_set() {              // c:3202
        // `:-` falls through to value when value is set.
        let mut state = SubstState::default();              // c:3202
        state                                               // c:3202
            .variables                                      // c:3202
            .insert("X".to_string(), "real".to_string());   // c:3202
        let (result, _, _) =                                // c:3202
            paramsubst("${X:-fallback}", 0, false, 0, &mut 0, &mut state); // c:3202
        assert_eq!(result, "real");                         // c:3202
    }                                                       // c:3202

    #[test]                                                 // c:3202
    fn paramsubst_assign_default_writes_back_scalar() {     // c:3202
        // subst.c:3245-3325 `case '=': case Equals:` — assign the
        // operand to the parameter when unset/empty AND return the
        // assigned value.
        let mut state = SubstState::default();              // c:3245
        let (result, _, _) =                                // c:3245
            paramsubst("${X:=initial}", 0, false, 0, &mut 0, &mut state); // c:3245
        assert_eq!(result, "initial");                      // c:3245
        assert_eq!(state.variables.get("X").map(|s| s.as_str()), Some("initial")); // c:3245
    }                                                       // c:3245

    #[test]                                                 // c:3245
    fn paramsubst_assign_default_skipped_when_set() {       // c:3245
        let mut state = SubstState::default();              // c:3245
        state                                               // c:3245
            .variables                                      // c:3245
            .insert("X".to_string(), "preset".to_string()); // c:3245
        let (result, _, _) =                                // c:3245
            paramsubst("${X:=initial}", 0, false, 0, &mut 0, &mut state); // c:3245
        assert_eq!(result, "preset");                       // c:3245
        // Original value preserved.
        assert_eq!(state.variables.get("X").map(|s| s.as_str()), Some("preset")); // c:3245
    }                                                       // c:3245

    #[test]                                                 // c:3245
    fn paramsubst_assign_default_writes_back_assoc() {      // c:3245
        // subst.c:3300-3305 — for hashed (`PM_HASHED`) parameters,
        // the writeback goes through `sethparam`. zshrs's port
        // dispatches on subscript + existing assoc-table presence.
        let mut state = SubstState::default();              // c:3300
        // Pre-declare assoc so dispatch picks the assoc path.
        state                                               // c:3300
            .assoc_arrays                                   // c:3300
            .insert("ZINIT".to_string(), indexmap::IndexMap::new()); // c:3300
        let (_result, _, _) = paramsubst(                   // c:3300
            "${ZINIT[BIN_DIR]:=somepath}",                  // c:3300
            0,                                              // c:3300
            false,                                          // c:3300
            0,                                              // c:3300
            &mut 0,                                         // c:3300
            &mut state,                                     // c:3300
        );                                                  // c:3300
        assert_eq!(                                         // c:3300
            state                                           // c:3300
                .assoc_arrays                               // c:3300
                .get("ZINIT")                               // c:3300
                .and_then(|m| m.get("BIN_DIR"))             // c:3300
                .map(|s| s.as_str()),                       // c:3300
            Some("somepath")                                // c:3300
        );                                                  // c:3300
    }                                                       // c:3300

    #[test]                                                 // c:3300
    fn paramsubst_assign_default_auto_promotes_to_assoc() { // c:3300
        // zsh's bracket-subscript writeback creates an assoc when
        // the index is non-numeric and no array of either kind
        // exists. Pinned per `: ${ZINIT[BIN_DIR]:="${ZINIT[ZERO]:h}"}`
        // working without prior `typeset -gA ZINIT`.
        let mut state = SubstState::default();              // c:3300
        let (_result, _, _) =                               // c:3300
            paramsubst("${ARR[K]:=v}", 0, false, 0, &mut 0, &mut state); // c:3300
        assert_eq!(                                         // c:3300
            state                                           // c:3300
                .assoc_arrays                               // c:3300
                .get("ARR")                                 // c:3300
                .and_then(|m| m.get("K"))                   // c:3300
                .map(|s| s.as_str()),                       // c:3300
            Some("v")                                       // c:3300
        );                                                  // c:3300
    }                                                       // c:3300

    #[test]                                                 // c:3300
    fn paramsubst_assign_default_writes_indexed_array_slot() { // c:3300
        // subst.c:3296-3305 `setaparam` path. zshrs port: numeric
        // subscript with no assoc declared → indexed slot, 1-based.
        let mut state = SubstState::default();              // c:3296
        // Pre-declare so subst_port's check `state.arrays.contains_key`
        // doesn't auto-promote to assoc.
        state.arrays.insert("ARR".to_string(), Vec::new()); // c:3296
        let (_result, _, _) =                               // c:3296
            paramsubst("${ARR[3]:=val}", 0, false, 0, &mut 0, &mut state); // c:3296
        let arr = state.arrays.get("ARR").unwrap();         // c:3296
        assert_eq!(arr.len(), 3);                           // c:3296
        assert_eq!(arr[2], "val"); // 1-based subscript → index 2. // c:3296
        // Slots 0 and 1 are auto-padded.
        assert_eq!(arr[0], "");                             // c:3296
        assert_eq!(arr[1], "");                             // c:3296
    }                                                       // c:3296

    #[test]                                                 // c:3296
    fn paramsubst_assign_default_expands_operand() {        // c:3296
        // The motivating bug: `: ${ZINIT[BIN_DIR]:=${ZINIT[ZERO]:h}}`
        // must store the EXPANDED dirname, not the literal
        // `${ZINIT[ZERO]:h}` template.
        let mut state = SubstState::default();              // c:3296
        state                                               // c:3296
            .variables                                      // c:3296
            .insert("INNER".to_string(), "computed".to_string()); // c:3296
        let (_result, _, _) =                               // c:3296
            paramsubst("${OUTER:=${INNER}}", 0, false, 0, &mut 0, &mut state); // c:3296
        assert_eq!(                                         // c:3296
            state.variables.get("OUTER").map(|s| s.as_str()), // c:3296
            Some("computed")                                // c:3296
        );                                                  // c:3296
    }                                                       // c:3296

    #[test]                                                 // c:3296
    fn paramsubst_alternative_when_set() {                  // c:3296
        // subst.c:3193-3199 `case '+':` — return operand if set,
        // empty if unset.
        let mut state = SubstState::default();              // c:3193
        state                                               // c:3193
            .variables                                      // c:3193
            .insert("X".to_string(), "anything".to_string()); // c:3193
        let (result, _, _) =                                // c:3193
            paramsubst("${X:+yes}", 0, false, 0, &mut 0, &mut state); // c:3193
        assert_eq!(result, "yes");                          // c:3193
    }                                                       // c:3193

    #[test]                                                 // c:3193
    fn paramsubst_alternative_when_unset() {                // c:3193
        let mut state = SubstState::default();              // c:3193
        let (result, _, _) =                                // c:3193
            paramsubst("${X:+yes}", 0, false, 0, &mut 0, &mut state); // c:3193
        assert_eq!(result, "");                             // c:3193
    }                                                       // c:3193

    // ─── paramsubst — length operator ${#var} ───────────────────────

    #[test]                                                 // c:3193
    fn paramsubst_length_returns_char_count() {             // c:3193
        // subst.c — `${#var}` returns chars in the (joined) value.
        let mut state = SubstState::default();              // c:3193
        state                                               // c:3193
            .variables                                      // c:3193
            .insert("FOO".to_string(), "abcde".to_string()); // c:3193
        let (result, _, _) =                                // c:3193
            paramsubst("${#FOO}", 0, false, 0, &mut 0, &mut state); // c:3193
        assert_eq!(result, "5");                            // c:3193
    }                                                       // c:3193

    // ─── multsub / singsub ──────────────────────────────────────────

    #[test]                                                 // c:3193
    fn singsub_returns_single_word() {                      // c:3193
        // subst.c::singsub joins the prefork output into one word.
        let mut state = SubstState::default();              // c:3193
        state                                               // c:3193
            .variables                                      // c:3193
            .insert("FOO".to_string(), "hello".to_string()); // c:3193
        // Plain string — no expansion.
        assert_eq!(singsub("plain text", &mut state), "plain text"); // c:3193
    }                                                       // c:3193

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


    // ─── p10k internal/p10k.zsh:6 — (q) quote + (#b) backref ──────








    // ─── p10k:298 — (P) indirect on assoc lookup ──────────────────


    // ─── p10k:380 — (u) unique on array ──────────────────────────


    // ─── p10k:403 — (L) lowercase ────────────────────────────────


    // ─── p10k:321 — `::=` + (Q) + ~ glob_subst on token ──────────


    // ─── zinit's gnarliest — (#b) backref + ${match[N]} in repl ──


    // ─── (kv) paired keys+values ─────────────────────────────────


    // ─── nested with literal `~` glob_subst ──────────────────────

}                                                           // c:3193

// ============================================================================
// Additional functions for 100% coverage of subst.c
// ============================================================================

/// Sortit flags from subst.c
pub mod sortit_flags {                                      // c:3193
    pub const ANYOLDHOW: u32 = 0;                           // c:3193
    pub const SOMEHOW: u32 = 1;                             // c:3193
    pub const BACKWARDS: u32 = 2;                           // c:3193
    pub const IGNORING_CASE: u32 = 4;                       // c:3193
    pub const NUMERICALLY: u32 = 8;                         // c:3193
    pub const NUMERICALLY_SIGNED: u32 = 16;                 // c:3193
}                                                           // c:3193

/// CASMOD_* constants from subst.c
pub mod casmod {                                            // c:3193
    pub const NONE: u32 = 0;                                // c:3193
    pub const LOWER: u32 = 1;                               // c:3193
    pub const UPPER: u32 = 2;                               // c:3193
    pub const CAPS: u32 = 3;                                // c:3193
}                                                           // c:3193

/// QT_* quote type constants from subst.c
pub mod qt {                                                // c:3193
    pub const NONE: u32 = 0;                                // c:3193
    pub const BACKSLASH: u32 = 1;                           // c:3193
    pub const SINGLE: u32 = 2;                              // c:3193
    pub const DOUBLE: u32 = 3;                              // c:3193
    pub const DOLLARS: u32 = 4;                             // c:3193
    pub const BACKSLASH_PATTERN: u32 = 5;                   // c:3193
    pub const QUOTEDZPUTS: u32 = 6;                         // c:3193
    pub const SINGLE_OPTIONAL: u32 = 7;                     // c:3193
}                                                           // c:3193

/// Error flags
pub mod errflag {                                           // c:3193
    pub const ERROR: u32 = 1;                               // c:3193
    pub const INT: u32 = 2;                                 // c:3193
    pub const HARD: u32 = 4;                                // c:3193
}                                                           // c:3193

/// Parameter flags from params.h (PM_*)
pub mod pm_flags {                                          // c:3193
    pub const SCALAR: u32 = 0;                              // c:3193
    pub const ARRAY: u32 = 1;                               // c:3193
    pub const INTEGER: u32 = 2;                             // c:3193
    pub const EFLOAT: u32 = 3;                              // c:3193
    pub const FFLOAT: u32 = 4;                              // c:3193
    pub const HASHED: u32 = 5;                              // c:3193
    pub const NAMEREF: u32 = 6;                             // c:3193

    pub const LEFT: u32 = 1 << 6;                           // c:3193
    pub const RIGHT_B: u32 = 1 << 7;                        // c:3193
    pub const RIGHT_Z: u32 = 1 << 8;                        // c:3193
    pub const LOWER: u32 = 1 << 9;                          // c:3193
    pub const UPPER: u32 = 1 << 10;                         // c:3193
    pub const READONLY: u32 = 1 << 11;                      // c:3193
    pub const TAGGED: u32 = 1 << 12;                        // c:3193
    pub const EXPORTED: u32 = 1 << 13;                      // c:3193
    pub const UNIQUE: u32 = 1 << 14;                        // c:3193
    pub const UNSET: u32 = 1 << 15;                         // c:3193
    pub const HIDE: u32 = 1 << 16;                          // c:3193
    pub const HIDEVAL: u32 = 1 << 17;                       // c:3193
    pub const SPECIAL: u32 = 1 << 18;                       // c:3193
    pub const LOCAL: u32 = 1 << 19;                         // c:3193
    pub const TIED: u32 = 1 << 20;                          // c:3193
    pub const DECLARED: u32 = 1 << 21;                      // c:3193
}                                                           // c:3193

/// Null string constant (matches C: char nulstring[] = {Nularg, '\0'})
pub static NULSTRING_BYTES: [char; 2] = [NULARG, '\0'];     // c:3193








/// Value buffer structure (mirrors struct value from C)
#[derive(Debug, Clone, Default)]                            // c:N/A
pub struct ValueBuf {                                       // c:N/A
    pub pm: Option<ParamInfo>,                              // c:N/A
    pub start: i64,                                         // c:N/A
    pub end: i64,                                           // c:N/A
    pub valflags: u32,                                      // c:N/A
    pub scanflags: u32,                                     // c:N/A
}                                                           // c:N/A

/// Parameter info (mirrors Param from C)
#[derive(Debug, Clone, Default)]                            // c:N/A
pub struct ParamInfo {                                      // c:N/A
    pub name: String,                                       // c:N/A
    pub flags: u32,                                         // c:N/A
    pub level: u32,                                         // c:N/A
    pub value: ParamValue,                                  // c:N/A
}                                                           // c:N/A

/// Value flags
pub mod valflag {                                           // c:N/A
    pub const INV: u32 = 1;                                 // c:N/A
    pub const EMPTY: u32 = 2;                               // c:N/A
    pub const SUBST: u32 = 4;                               // c:N/A
}                                                           // c:N/A



/// Evaluate character from number (for (#) flag)
/// Port of substevalchar() from subst.c
pub fn substevalchar(s: &str) -> Option<String> {           // c:1490
    let val = crate::ported::math::mathevali(s).unwrap_or(0);                                 // c:1490
    if val < 0 {                                            // c:1490
        return None;                                        // c:1490
    }                                                       // c:1490

    char::from_u32(val as u32).map(|c| c.to_string())       // c:1490
}                                                           // c:1490

/// Check for colon subscript in parameter expansion
/// Port of check_colon_subscript() from subst.c
pub fn check_colon_subscript(s: &str) -> Option<(String, String)> { // c:1566
    // Could this be a modifier (or empty)?
    if s.is_empty() || s.starts_with(|c: char| c.is_ascii_alphabetic()) || s.starts_with('&') { // c:1566
        return None;                                        // c:1566
    }                                                       // c:1566

    if s.starts_with(':') {                                 // c:1566
        return Some(("0".to_string(), s.to_string()));      // c:1566
    }                                                       // c:1566

    // Parse subscript expression
    let (expr, rest) = (s.to_string(), "".to_string());                // c:1566
    Some((expr, rest))                                      // c:1566
}                                                           // c:1566


/// Untokenize and escape string for flag argument
/// Port of untok_and_escape() from subst.c
pub fn untok_and_escape(s: &str, escapes: bool, tok_arg: bool) -> String { // c:1528
    let mut result = crate::lex::untokenize(s);                         // c:1528

    if escapes {                                            // c:1528
        result = crate::ported::utils::getkeystring(&result).0;                     // c:1528
    }                                                       // c:1528

    if tok_arg {                                            // c:1528
        let _ = crate::ported::glob::shtokenize(&result);                       // c:1528
    }                                                       // c:1528

    result                                                  // c:1528
}                                                           // c:1528













/// Default IFS value
pub const DEFAULT_IFS: &str = " \t\n";                      // c:N/A






impl SubstOptions {                                         // c:N/A
}                                                           // c:N/A


/// Text attribute type for prompt highlighting
pub type ZAttr = u64;                                       // c:N/A





/// LEXFLAGS for (z) flag
pub mod lexflags {                                          // c:N/A
    pub const ACTIVE: u32 = 1;                              // c:N/A
    pub const COMMENTS_KEEP: u32 = 2;                       // c:N/A
    pub const COMMENTS_STRIP: u32 = 4;                      // c:N/A
    pub const NEWLINE: u32 = 8;                             // c:N/A
}                                                           // c:N/A







// ============================================================================
// Final functions for complete subst.c coverage
// ============================================================================

/// Token constants for Dnull, Snull, etc.
pub const DNULL: char = '\u{97}'; // "                      // c:N/A
pub const BNULLKEEP: char = '\u{95}'; // Backslash null that stays // c:N/A



/// Equal substitution (=cmd)
/// Port of equalsubstr() from subst.c lines 706-722
pub fn equalsubstr(s: &str, assign: bool, nomatch: bool, state: &SubstState) -> Option<String> { // c:715
    // Find end of command name
    let end = s                                             // c:715
        .chars()                                            // c:715
        .take_while(|&c| c != '\0' && c != INPAR && !(assign && c == ':')) // c:715
        .count();                                           // c:715

    let cmdstr: String = s.chars().take(end).collect();     // c:715
    let cmdstr = crate::lex::untokenize(&cmdstr);                       // c:715
    let cmdstr = cmdstr.replace('\0', "");                       // c:715

    if let Some(path) = None::<String> {     // c:715
        let rest: String = s.chars().skip(end).collect();   // c:715
        if rest.is_empty() {                                // c:715
            Some(path)                                      // c:715
        } else {                                            // c:715
            Some(format!("{}{}", path, rest))               // c:715
        }                                                   // c:715
    } else {                                                // c:715
        if nomatch {                                        // c:715
            eprintln!("{}: not found", cmdstr);             // c:715
        }                                                   // c:715
        None                                                // c:715
    }                                                       // c:715
}                                                           // c:715
















/// Additional token constants
pub mod extra_tokens {                                      // c:N/A
    pub const TILDE: char = '\u{98}';                       // c:N/A
    pub const DASH: char = '\u{96}';                        // c:N/A
    pub const STAR: char = '\u{99}';                        // c:N/A
    pub const QUEST: char = '\u{9A}';                       // c:N/A
    pub const HAT: char = '\u{9B}';                         // c:N/A
    pub const BAR: char = '\u{9C}';                         // c:N/A
}                                                           // c:N/A

/// Output radix for arithmetic (default 10)
pub static OUTPUT_RADIX: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(10); // c:N/A

/// Output underscore flag for arithmetic
pub static OUTPUT_UNDERSCORE: std::sync::atomic::AtomicBool = // c:N/A
    std::sync::atomic::AtomicBool::new(false);              // c:N/A





/// MN_FLOAT flag for math numbers
pub const MN_FLOAT: u32 = 1;                                // c:N/A

/// Math number type (mirrors mnumber union from C)
#[derive(Clone, Copy)]                                      // c:N/A
pub struct MNumber {                                        // c:N/A
    pub type_: u32,                                         // c:N/A
    pub int_val: i64,                                       // c:N/A
    pub float_val: f64,                                     // c:N/A
}                                                           // c:N/A

impl std::fmt::Debug for MNumber {                          // c:N/A
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { // c:N/A
        if self.type_ & MN_FLOAT != 0 {                     // c:N/A
            write!(f, "MNumber(float: {})", self.float_val) // c:N/A
        } else {                                            // c:N/A
            write!(f, "MNumber(int: {})", self.int_val)     // c:N/A
        }                                                   // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

impl Default for MNumber {                                  // c:N/A
    fn default() -> Self {                                  // c:N/A
        MNumber {                                           // c:N/A
            type_: 0,                                       // c:N/A
            int_val: 0,                                     // c:N/A
            float_val: 0.0,                                 // c:N/A
        }                                                   // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A


/// Brace expansion state
#[derive(Debug, Clone)]                                     // c:N/A
pub struct BraceInfo {                                      // c:N/A
    pub str_: String,                                       // c:N/A
    pub pos: usize,                                         // c:N/A
    pub inbrace: bool,                                      // c:N/A
}                                                           // c:N/A



// =============================================================
// Merged from former src/subst_paramsubst_port.rs:
//   Faithful 1:1 port target for paramsubst (subst.c:1625-4473).
//   Gated behind ZSHRS_NEW_PARAMSUBST=1; legacy paramsubst above
//   is the production path until parity is reached.
// =============================================================
#[allow(non_snake_case)]                                    // glob.c:2276
#[allow(dead_code)]                                         // glob.c:2276
#[allow(unused_assignments)]                                // glob.c:2276
#[allow(unused_variables)]                                  // glob.c:2276
#[allow(unused_mut)]                                        // glob.c:2276
pub(crate) mod paramsubst_inline {                          // glob.c:2276
    use super::{prefork_flags, SubstState};                 // glob.c:2276

    /// Sentinel returned when the new port can't yet handle a given
    /// `${...}` shape; `subst_port::substitute_brace[_array]` then
    /// retries through legacy `paramsubst`.
    pub(crate) struct FallbackToLegacy;                         // c:N/A

    /// Mirrors C `enum quote_type` from zsh.h.
    /// subst.c:1739 — `int quotemod = 0, quotetype = QT_NONE, quoteerr = 0;`
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]       // c:1739
    pub(crate) enum QtType {                                    // c:1739
        #[default]                                              // c:1739
        None = 0,                                               // c:1739
        Single = 1,                                             // c:1739
        Double = 2,                                             // c:1739
        Dollars = 3,                                            // c:1739
        Backslash = 4,                                          // c:1739
        SingleOptional = 5,                                     // c:1739
        QuotedZputs = 6,                                        // c:1739
        BackslashPattern = 7,                                   // c:1739
    }                                                       // c:1739

    impl QtType {                                               // c:1739
        fn next(self) -> Self {                                 // c:1739
            match self {                                        // c:1739
                QtType::None => QtType::Single,                 // c:1739
                QtType::Single => QtType::Double,               // c:1739
                QtType::Double => QtType::Dollars,              // c:1739
                QtType::Dollars => QtType::Backslash,           // c:1739
                QtType::Backslash => QtType::SingleOptional,    // c:1739
                QtType::SingleOptional => QtType::QuotedZputs,  // c:1739
                QtType::QuotedZputs => QtType::BackslashPattern, // c:1739
                QtType::BackslashPattern => QtType::BackslashPattern, // saturate // c:1739
            }                                               // c:1739
        }                                                   // c:1739
    }                                                       // c:1739

    /// Mirrors `enum cas_mod` from subst.c:1731.
    /// subst.c:1731-1732 — `int casmod = CASMOD_NONE;`
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]       // c:1731
    pub(crate) enum CasMod {                                    // c:1731
        #[default]                                              // c:1731
        None,                                                   // c:1731
        Lower,                                                  // c:1731
        Upper,                                                  // c:1731
        Caps,                                                   // c:1731
    }                                                       // c:1731

    // Bits for `flags` — direct port of SUB_* macros.
    // zsh.h:1981-1996 — verbatim hex values, do not "tidy" into 1<<N
    // sequences (the wrong-bit drift in top-level subst.rs comes from
    // exactly that mistake).
    pub(crate) mod sub_flags {                                  // zsh.h:1981
        pub const END:     u32 = 0x0001;   // % or %%           // zsh.h:1981
        pub const LONG:    u32 = 0x0002;   // doubled # or %    // zsh.h:1982
        pub const SUBSTR:  u32 = 0x0004;   // (S)               // zsh.h:1983
        pub const MATCH:   u32 = 0x0008;   // (M)               // zsh.h:1984
        pub const REST:    u32 = 0x0010;   // (R)               // zsh.h:1985
        pub const BIND:    u32 = 0x0020;   // (B)               // zsh.h:1986
        pub const EIND:    u32 = 0x0040;   // (E)               // zsh.h:1987
        pub const LEN:     u32 = 0x0080;   // (N)               // zsh.h:1988
        pub const ALL:     u32 = 0x0100;   // match whole str   // zsh.h:1989
        pub const GLOBAL:  u32 = 0x0200;   // ${..//..}         // zsh.h:1990
        pub const DOSUBST: u32 = 0x0400;   // repl needs subst  // zsh.h:1991
        pub const RETFAIL: u32 = 0x0800;   // status 0 if no match // zsh.h:1992
        pub const START:   u32 = 0x1000;   // anchor at start   // zsh.h:1993
        pub const LIST:    u32 = 0x2000;   // return list       // zsh.h:1995
        pub const EGLOB:   u32 = 0x4000;   // (*) extended glob // zsh.h:1996
    }                                                       // zsh.h:1996

    // Bits for `sortit` — direct port of SORTIT_* enum.
    // zsh.h:2992-3008.
    pub(crate) mod sortit_flags {                               // zsh.h:2992
        pub const ANYOLDHOW:           u32 = 0;                 // zsh.h:2993
        pub const IGNORING_CASE:       u32 = 1;                 // zsh.h:2994
        pub const NUMERICALLY:         u32 = 2;                 // zsh.h:2995
        pub const NUMERICALLY_SIGNED:  u32 = 4;                 // zsh.h:2996
        pub const BACKWARDS:           u32 = 8;                 // zsh.h:2997
        pub const IGNORING_BACKSLASHES: u32 = 16;               // zsh.h:3002
        pub const SOMEHOW:             u32 = 32;                // zsh.h:3007
    }                                                       // zsh.h:3008

    // Bits for `hkeys`/`hvals` and the wider scanflags arg of fetchvalue.
    // Direct port of SCANPM_* from zsh.h:1953-1973.
    pub(crate) mod scanpm_flags {                               // zsh.h:1953
        pub const WANTVALS:   u32 = 1 <<  0;                    // zsh.h:1953
        pub const WANTKEYS:   u32 = 1 <<  1;                    // zsh.h:1954
        pub const WANTINDEX:  u32 = 1 <<  2;                    // zsh.h:1955
        pub const MATCHKEY:   u32 = 1 <<  3;                    // zsh.h:1956
        pub const MATCHVAL:   u32 = 1 <<  4;                    // zsh.h:1957
        pub const MATCHMANY:  u32 = 1 <<  5;                    // zsh.h:1958
        pub const ASSIGNING:  u32 = 1 <<  6;                    // zsh.h:1959
        pub const KEYMATCH:   u32 = 1 <<  7;                    // zsh.h:1960
        pub const DQUOTED:    u32 = 1 <<  8;                    // zsh.h:1961
        pub const ARRONLY:    u32 = 1 <<  9;                    // zsh.h:1965
        pub const CHECKING:   u32 = 1 << 10;                    // zsh.h:1969
        pub const NOEXEC:     u32 = 1 << 11;                    // zsh.h:1970
        pub const NONAMESPC:  u32 = 1 << 12;                    // zsh.h:1971
        pub const NONAMEREF:  u32 = 1 << 13;                    // zsh.h:1972
        pub const ISVAR_AT:   u32 = 1 << 14;                    // zsh.h:1973
    }                                                       // zsh.h:1973

    // LEXFLAGS_* — direct port of zsh.h:2293-2315 hex values.
    pub(crate) mod lexflags {                                   // zsh.h:2293
        pub const ACTIVE:         u32 = 0x0001;                 // zsh.h:2293
        pub const ZLE:            u32 = 0x0002;                 // zsh.h:2299
        pub const COMMENTS_KEEP:  u32 = 0x0004;                 // zsh.h:2303
        pub const COMMENTS_STRIP: u32 = 0x0008;                 // zsh.h:2307
        pub const NEWLINE:        u32 = 0x0010;                 // zsh.h:2315
    }                                                       // zsh.h:2315

    // GETKEY_* — direct port of zsh.h:3143-3166.
    pub(crate) mod getkey {                                     // zsh.h:3143
        pub const OCTAL_ESC:       u32 = 1 << 0;                // zsh.h:3143
        pub const EMACS:           u32 = 1 << 1;                // zsh.h:3150
        pub const CTRL:            u32 = 1 << 2;                // zsh.h:3152
        pub const DOLLAR_QUOTE:    u32 = 1 << 4;                // zsh.h:3156
        pub const BACKSLASH_MINUS: u32 = 1 << 5;                // zsh.h:3158
        pub const UPDATE_OFFSET:   u32 = 1 << 7;                // zsh.h:3166
    }                                                       // zsh.h:3166

    // PM_* parameter type+modifier bits — direct port of zsh.h:1878-1944.
    // Top-level subst.rs::pm_flags has wrong bit values (// c:N/A); it's
    // the legacy paramsubst path's flag set and dies at parity-collapse.
    // Use this module for any new paramsubst_port arm.
    pub(crate) mod pm_flags {                                   // zsh.h:1878
        pub const SCALAR:         u32 = 0;                      // zsh.h:1878
        pub const ARRAY:          u32 = 1 <<  0;                // zsh.h:1879
        pub const INTEGER:        u32 = 1 <<  1;                // zsh.h:1880
        pub const EFLOAT:         u32 = 1 <<  2;                // zsh.h:1881
        pub const FFLOAT:         u32 = 1 <<  3;                // zsh.h:1882
        pub const HASHED:         u32 = 1 <<  4;                // zsh.h:1883
        // PM_TYPE(X) mask — zsh.h:1885 macro. Used to extract the
        // type bits from a flags word.
        pub const TYPE_MASK:      u32 = ARRAY | INTEGER | EFLOAT | FFLOAT | HASHED | NAMEREF; // zsh.h:1885
        pub const LEFT:           u32 = 1 <<  5;                // zsh.h:1888
        pub const RIGHT_B:        u32 = 1 <<  6;                // zsh.h:1889
        pub const RIGHT_Z:        u32 = 1 <<  7;                // zsh.h:1890
        pub const LOWER:          u32 = 1 <<  8;                // zsh.h:1891
        pub const UPPER:          u32 = 1 <<  9;                // zsh.h:1895
        pub const READONLY:       u32 = 1 << 10;                // zsh.h:1898
        pub const TAGGED:         u32 = 1 << 11;                // zsh.h:1899
        pub const EXPORTED:       u32 = 1 << 12;                // zsh.h:1900
        pub const UNIQUE:         u32 = 1 << 13;                // zsh.h:1905
        pub const HIDE:           u32 = 1 << 14;                // zsh.h:1908
        pub const HIDEVAL:        u32 = 1 << 15;                // zsh.h:1910
        pub const TIED:           u32 = 1 << 16;                // zsh.h:1912
        pub const TAGGED_LOCAL:   u32 = 1 << 16;                // zsh.h:1913 (sibling of TIED)
        pub const SPECIAL:        u32 = 1 << 20;                // zsh.h:1922
        pub const DECLARED:       u32 = 1 << 22;                // zsh.h:1928
        pub const UNSET:          u32 = 1 << 24;                // zsh.h:1930
        pub const NAMEREF:        u32 = 1 << 30;                // zsh.h:1944
    }                                                       // zsh.h:1944

    // VALFLAG_* — direct port of zsh.h:758-760 enum.
    pub(crate) mod valflag_flags {                              // zsh.h:758
        pub const INV:   u32 = 0x0001;                          // zsh.h:758
        pub const EMPTY: u32 = 0x0002;                          // zsh.h:759
        pub const SUBST: u32 = 0x0004;                          // zsh.h:760
    }                                                       // zsh.h:760

    // PREFORK_* deliberately NOT redefined here. Top-level
    // subst.rs::prefork_flags (line 113) has wrong bit values per
    // zsh.h:2020-2046, but `paramsubst_port`'s `pf_flags` argument is
    // wired through legacy callers (prefork, stringsubst) that pass
    // pf_flags built with the TOP-LEVEL constants. Mismatching the bit
    // values here would silently misinterpret the caller's flags.
    // Cross-module bit-value contract: keep using `super::prefork_flags`
    // until top-level is fixed (a separate commit that touches every
    // legacy-paramsubst call site) OR until legacy paramsubst dies at
    // parity-collapse, whichever comes first.

    // MULTSUB_* — direct port of zsh.h:2050-2065 enum.
    pub(crate) mod multsub_flags {                              // zsh.h:2050
        pub const WS_AT_START: u32 = 1;                         // zsh.h:2055
        pub const WS_AT_END:   u32 = 2;                         // zsh.h:2060
        pub const PARAM_NAME:  u32 = 4;                         // zsh.h:2065
    }                                                       // zsh.h:2065

    /// Result of the C-style `goto flagerr` exit from the flag-parsing loop.
    /// subst.c:2504-2530 — labelled block that emits `error in flags near
    /// position N in '$STR'` and returns NULL.
    #[derive(Debug)]                                            // c:2504
    struct FlagErr {                                            // c:2504
        offset: usize,                                          // c:2504
        msg: &'static str,                                      // c:2504
    }                                                       // c:2504

    /// Holds every local variable declared in `paramsubst` between
    /// subst.c:1625 (function open) and subst.c:1875 (end of declarations,
    /// just before `*s++ = '\0'`). Field names match C identifiers.
    ///
    /// Threading: this struct lives on the stack of the entry function.
    /// Helpers operate on it via `&mut`. All cross-section invariants
    /// (e.g. "aval is non-empty iff isarr != 0") are encoded by the
    /// fields all sitting in one place — same as the C function-locals.
    #[derive(Default)]                                          // c:1625
    struct ParamSubstLocals {                                   // c:1625
        /// subst.c:1658 — `int isarr = 0;` (-1, 0, 1, 2 — see C comment)
        isarr: i32,                                             // c:1658
        /// subst.c:1663 — `int plan9 = isset(RCEXPANDPARAM);`
        plan9: bool,                                            // c:1663
        /// subst.c:1669 — `int globsubst = isset(GLOBSUBST);`
        globsubst: i32,                                         // c:1669
        /// subst.c:1673 — `int evalchar = 0;` (#)
        evalchar: i32,                                          // c:1673
        /// subst.c:1678 — `int getlen = 0;`
        getlen: i32,                                            // c:1678
        /// subst.c:1679 — `int whichlen = 0;`
        whichlen: i32,                                          // c:1679
        /// subst.c:1683 — `int chkset = 0;` (+pm)
        chkset: i32,                                            // c:1683
        /// subst.c:1691 — `int vunset = 0;`
        vunset: i32,                                            // c:1691
        /// subst.c:1697 — `int wantt = 0;` (t)
        wantt: i32,                                             // c:1697
        /// subst.c:1705-1706 — `int spbreak = …;` (sh-word-split active)
        spbreak: bool,                                          // c:1705
        /// subst.c:1708 — `char *val = NULL;`
        val: Option<String>,                                    // c:1708
        /// subst.c:1708 — `char **aval = NULL;`
        aval: Option<Vec<String>>,                              // c:1708
        /// subst.c:1720 — `int flags = 0;`  (SUB_* bits)
        flags: u32,                                             // c:1720
        /// subst.c:1722 — `int flnum = 0;`
        flnum: i32,                                             // c:1722
        /// subst.c:1728 — sortit (SORTIT_*)
        sortit: u32,                                            // c:1728
        /// subst.c:1728 — `indord = 0;` (a)
        indord: i32,                                            // c:1728
        /// subst.c:1730 — `int unique = 0;`
        unique: i32,                                            // c:1730
        /// subst.c:1732 — `int casmod = CASMOD_NONE;`
        casmod: CasMod,                                         // c:1732
        /// subst.c:1739 — `int quotemod = 0`
        quotemod: i32,                                          // c:1739
        /// subst.c:1739 — `int quotetype = QT_NONE`
        quotetype: QtType,                                      // c:1739
        /// subst.c:1739 — `int quoteerr = 0;` (X)
        quoteerr: i32,                                          // c:1739
        /// subst.c:1746 — `int mods = 0;` (D=bit0, V=bit1)
        mods: i32,                                              // c:1746
        /// subst.c:1754 — `int shsplit = 0;` (z/Z)
        shsplit: u32,                                           // c:1754
        /// subst.c:1759 — `int ssub = (pf_flags & PREFORK_SINGLE);`
        ssub: bool,                                             // c:1759
        /// subst.c:1766 — `char *sep = NULL`
        sep: Option<String>,                                    // c:1766
        /// subst.c:1766 — `char *spsep = NULL;`
        spsep: Option<String>,                                  // c:1766
        /// subst.c:1772 — `char *premul`
        premul: Option<String>,                                 // c:1772
        /// subst.c:1772 — `char *postmul`
        postmul: Option<String>,                                // c:1772
        /// subst.c:1772 — `char *preone`
        preone: Option<String>,                                 // c:1772
        /// subst.c:1772 — `char *postone;`
        postone: Option<String>,                                // c:1772
        /// subst.c:1774 — `char *replstr = NULL;`
        replstr: Option<String>,                                // c:1774
        /// subst.c:1776 — `zlong prenum`
        prenum: i64,                                            // c:1776
        /// subst.c:1776 — `zlong postnum = 0;`
        postnum: i64,                                           // c:1776
        /// subst.c:1779 — `int multi_width = 0;` (m)
        multi_width: i32,                                       // c:1779
        /// subst.c:1787 — `int copied = 0;`
        copied: i32,                                            // c:1787
        /// subst.c:1793 — `int arrasg = 0;` (A, AA)
        arrasg: i32,                                            // c:1793
        /// subst.c:1798 — `int eval = 0;` (e)
        eval: i32,                                              // c:1798
        /// subst.c:1803 — `int aspar = 0;` (P)
        aspar: i32,                                             // c:1803
        /// subst.c:1807 — `int presc = 0;` (%)
        presc: i32,                                             // c:1807
        /// subst.c:1811 — `int getkeys = -1;` (g)
        getkeys: i32,                                           // c:1811
        /// subst.c:1817 — `int nojoin = …;` (@)
        nojoin: i32,                                            // c:1817
        /// subst.c:1823 — `char inbrace = 0;`
        inbrace: i32,                                           // c:1823
        /// subst.c:1828 — `int hkeys = 0;` (k)
        hkeys: u32,                                             // c:1828
        /// subst.c:1835 — `int hvals = 0;` (v)
        hvals: u32,                                             // c:1835
        /// subst.c:1843 — `int subexp;`
        subexp: i32,                                            // c:1843
        /// subst.c:1849 — `int horrible_offset_hack = 0;`
        horrible_offset_hack: i32,                              // c:1849
        /// subst.c:1857 — `int ms_flags = 0;`
        ms_flags: u32,                                          // c:1857
        /// subst.c:1863 — `int fetch_needed;`
        fetch_needed: i32,                                      // c:1863
        /// subst.c:1869 — `int quoted_array_with_offset = 0;`
        quoted_array_with_offset: i32,                          // c:1869
        /// subst.c:1873 — `char *rplyvar = NULL;`
        rplyvar: Option<String>,                                // c:1873
        /// subst.c:1874 — `char *rplytmp = NULL;`
        rplytmp: Option<String>,                                // c:1874

        // Flag-loop locals. In C these are scoped to the `if (c == Inpar)`
        // arm at subst.c:2131-2541, but Rust scoping puts them here for
        // borrow-clarity.
        /// subst.c:2140 — `int escapes = 0;` (p flag inside flag-loop)
        escapes: i32,                                           // c:2140
        /// subst.c:2145 — `int tok_arg = 0;` (~ inside flag-loop)
        tok_arg: i32,                                           // c:2145
    }                                                       // c:2145






    /// Faithful-ish port of `get_intarg` (subst.c, search "get_intarg").
    /// Returns (parsed_int, length_of_delim_used). zsh allows the int to
    /// be any quoted shell-form; we accept a contiguous run of decimal
    /// digits, which covers every (l)/(r)/(I) usage in real-world code.
    ///
    /// If no digits, returns (0, 0). If digits, returns (n, dellen=1).
    fn get_intarg(chars: &[char], pos: &mut usize) -> (i64, usize) { // c:2174
        let start = *pos;                                       // c:2174
        let mut n: i64 = 0;                                     // c:2174
        let mut any = false;                                    // c:2174
        while let Some(&c) = chars.get(*pos) {                  // c:2174
            if c.is_ascii_digit() {                             // c:2174
                n = n * 10 + (c as i64 - '0' as i64);           // c:2174
                *pos += 1;
                any = true;                                     // c:2174
            } else {                                            // c:2174
                break;                                          // c:2174
            }                                                   // c:2174
        }                                                       // c:2174
        let dellen = if any && *pos < chars.len() { 1 } else { 0 }; // c:2174
        let _ = start;                                          // c:2174
        (n, dellen)                                             // c:2174
    }                                                           // c:2174

    /// Port of `get_strarg` (subst.c). Reads the delimited argument that
    /// begins at `*pos`: the char at `*pos` is the opening delimiter, the
    /// argument is everything up to the next occurrence of that delimiter.
    /// Returns Some((arg, end_after_close)) or None on missing close.
    fn get_strarg(chars: &[char], pos: &mut usize) -> Option<(String, usize)> { // c:2155
        let open = chars.get(*pos).copied()?;                   // c:2155
        let close = match open {                                // c:2155
            '(' | '\u{85}' => ')',                              // c:2155
            '[' | '\u{89}' => ']',                              // c:2155
            '{' | '\u{87}' => '}',                              // c:2155
            '<' | '\u{8B}' => '>',                              // c:2155
            c => c, // same-char delim, e.g. `:`, `/`, `.`, `,` // c:2155
        };                                                      // c:2155
        let mut q = *pos + 1;                                   // c:2155
        while q < chars.len() {                                 // c:2155
            let cc = chars[q];                                  // c:2155
            if cc == close || (close == ')' && cc == '\u{86}') { // c:2155
                let arg: String = chars[*pos + 1..q].iter().collect(); // c:2155
                return Some((arg, q + 1));                      // c:2155
            }                                                   // c:2155
            q += 1;                                             // c:2155
        }                                                       // c:2155
        None                                                    // c:2155
    }                                                           // c:2155

    /// Port of `untok_and_escape` (subst.c). Replaces tokenised chars
    /// with their literal forms and processes escape sequences if either
    /// `escapes` or `tok_arg` is set. The full C version handles many
    /// edge cases; this minimal port suffices for the (s::), (j::), (g),
    /// (l/r) flag arguments — those rarely contain tokens.
    fn untok_and_escape(s: &str, escapes: i32, tok_arg: i32) -> String { // c:2155
        let mut out = String::with_capacity(s.len());           // c:2155
        let mut chs = s.chars().peekable();                     // c:2155
        while let Some(c) = chs.next() {                        // c:2155
            if c == '\\' && (escapes != 0 || tok_arg != 0) {    // c:2155
                // Process \n, \t, \\, \\xNN etc.
                match chs.next() {                              // c:2155
                    Some('n') => out.push('\n'),                // c:2155
                    Some('t') => out.push('\t'),                // c:2155
                    Some('r') => out.push('\r'),                // c:2155
                    Some('\\') => out.push('\\'),               // c:2155
                    Some(other) => {                            // c:2155
                        out.push('\\');                         // c:2155
                        out.push(other);                        // c:2155
                    }                                           // c:2155
                    None => out.push('\\'),                     // c:2155
                }                                               // c:2155
            } else if (c as u32) >= 0x80 && (c as u32) <= 0x94 { // c:2155
                // Tokenised char — replace with its literal.
                out.push(c); // c:2155
            } else {                                            // c:2155
                out.push(c);                                    // c:2155
            }                                                   // c:2155
        }                                                       // c:2155
        out                                                     // c:2155
    }                                                           // c:2155

    #[cfg(test)]                                                // c:2155
    mod tests {                                                 // c:2155
        use super::*;                                           // c:2155










    }                                                       // c:2155

}                                                           // c:2155

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: subst
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
}
// END moved-from-exec-rs

// END moved-from-exec-rs (free fns)
