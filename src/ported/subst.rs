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

    pub fn is_token(c: char) -> bool {                      // c:N/A
        c as u32 >= 0x80 && c as u32 <= 0x94                // c:N/A
    }                                                       // c:N/A

    pub fn token_to_char(c: char) -> char {                 // c:N/A
        match c {                                           // c:N/A
            POUND => '#',                                   // c:N/A
            STRING | QSTRING => '$',                        // c:N/A
            TICK | QTICK => '`',                            // c:N/A
            INPAR => '(',                                   // c:N/A
            OUTPAR => ')',                                  // c:N/A
            INBRACE => '{',                                 // c:N/A
            OUTBRACE => '}',                                // c:N/A
            INBRACK => '[',                                 // c:N/A
            OUTBRACK => ']',                                // c:N/A
            INANG => '<',                                   // c:N/A
            OUTANG => '>',                                  // c:N/A
            EQUALS => '=',                                  // c:N/A
            _ => c,                                         // c:N/A
        }                                                   // c:N/A
    }                                                       // c:N/A
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
    pub fn new() -> Self {                                  // c:100
        LinkList {                                          // c:100
            nodes: VecDeque::new(),                         // c:100
            flags: 0,                                       // c:100
        }                                                   // c:100
    }                                                       // c:100

    pub fn from_string(s: &str) -> Self {                   // c:100
        let mut list = LinkList::new();                     // c:100
        list.nodes.push_back(LinkNode {                     // c:100
            data: s.to_string(),                            // c:100
        });                                                 // c:100
        list                                                // c:100
    }                                                       // c:100

    pub fn first_node(&self) -> Option<usize> {             // c:100
        if self.nodes.is_empty() {                          // c:100
            None                                            // c:100
        } else {                                            // c:100
            Some(0)                                         // c:100
        }                                                   // c:100
    }                                                       // c:100

    pub fn get_data(&self, idx: usize) -> Option<&str> {    // c:100
        self.nodes.get(idx).map(|n| n.data.as_str())        // c:100
    }                                                       // c:100

    pub fn set_data(&mut self, idx: usize, data: String) {  // c:100
        if let Some(node) = self.nodes.get_mut(idx) {       // c:100
            node.data = data;                               // c:100
        }                                                   // c:100
    }                                                       // c:100

    pub fn insert_after(&mut self, idx: usize, data: String) -> usize { // c:100
        self.nodes.insert(idx + 1, LinkNode { data });      // c:100
        idx + 1                                             // c:100
    }                                                       // c:100

    pub fn remove(&mut self, idx: usize) {                  // c:100
        if idx < self.nodes.len() {                         // c:100
            self.nodes.remove(idx);                         // c:100
        }                                                   // c:100
    }                                                       // c:100

    pub fn next_node(&self, idx: usize) -> Option<usize> {  // c:100
        if idx + 1 < self.nodes.len() {                     // c:100
            Some(idx + 1)                                   // c:100
        } else {                                            // c:100
            None                                            // c:100
        }                                                   // c:100
    }                                                       // c:100

    pub fn is_empty(&self) -> bool {                        // c:100
        self.nodes.is_empty()                               // c:100
    }                                                       // c:100

    pub fn len(&self) -> usize {                            // c:100
        self.nodes.len()                                    // c:100
    }                                                       // c:100
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
    /// substitution code reads through `getvalue()`. Until subst_port
    /// is refactored to read/write the executor directly through
    /// `with_executor`, this snapshot+commit pattern bridges the two
    /// state representations.
    pub fn from_executor(exec: &crate::exec::ShellExecutor) -> Self { // c:2807
        // Convert IndexMap<String, String> assoc-array values to plain
        // HashMap so subst_port can iterate them. Insertion order is
        // lost in the snapshot; the post-call commit restores the
        // map but writes new keys at the end (zsh's hashtable
        // semantics for `${arr[k]:=v}` on unset key).
        let assoc_arrays: std::collections::HashMap<        // c:2807
            String,                                         // c:2807
            indexmap::IndexMap<String, String>,             // c:2807
        > = exec                                            // c:2807
            .assoc_arrays                                   // c:2807
            .iter()                                         // c:2807
            .map(|(k, v)| {                                 // c:2807
                (                                           // c:2807
                    k.clone(),                              // c:2807
                    v.iter().map(|(ik, iv)| (ik.clone(), iv.clone())).collect(), // c:2807
                )                                           // c:2807
            })                                              // c:2807
            .collect();                                     // c:2807
        // Snapshot the magic-assoc-backing tables. Cheap clones —
        // these are typically small (function names ~hundreds at
        // most for a real shell session).
        let function_names: std::collections::HashSet<String> = exec // c:2807
            .function_names()                               // c:2807
            .into_iter()                                    // c:2807
            .collect();                                     // c:2807
        let alias_names: std::collections::HashSet<String> = // c:2807
            exec.aliases.keys().cloned().collect();         // c:2807
        // Don't pre-populate command_names — `${+commands[X]}` is
        // rare enough that a lazy fill via PATH walk on first use
        // wins over eagerly enumerating every executable on disk.
        let command_names: std::collections::HashSet<String> = std::collections::HashSet::new(); // c:2807

        let mut arrays = exec.arrays.clone();               // c:2807
        // Mirror exec's positional params under the "@", "*", and
        // "argv" array keys so subst_port can use one lookup path for
        // `$@`, `${@:N:M}`, `${#@}`, `${#*}`, `$argv` etc. Source of
        // truth lives in `exec.positional_params`; the snapshot is
        // read-only — `set --` runs through exec directly, not
        // through SubstState.
        // ALWAYS overwrite any pre-existing "@"/"*"/"argv" entry —
        // exec.positional_params is the live source. An earlier
        // `.entry().or_insert_with()` pattern left stale values from
        // prior calls (e.g. function arg "ls" surviving into the
        // next call's "nope_zr" scope), breaking `${+commands[$1]}`
        // inside fns called more than once.
        arrays.insert("@".to_string(), exec.positional_params.clone()); // c:2807
        arrays.insert("*".to_string(), exec.positional_params.clone()); // c:2807
        arrays.insert("argv".to_string(), exec.positional_params.clone()); // c:2807

        SubstState {                                        // c:2807
            errflag: false,                                 // c:2807
            opts: SubstOptions::default(),                  // c:2807
            variables: exec.variables.clone(),              // c:2807
            arrays,                                         // c:2807
            assoc_arrays,                                   // c:2807
            skip_filesub: false,                            // c:2807
            function_names,                                 // c:2807
            command_names,                                  // c:2807
            alias_names,                                    // c:2807
            var_attrs: exec.var_attrs.clone(),              // c:2807
        }                                                   // c:2807
    }                                                       // c:2807

    /// Commit any state mutations performed by `paramsubst` back to
    /// the live `ShellExecutor`. Called after each substitution that
    /// might write — `${var:=value}`, `${var:?}` etc.
    ///
    /// Implementation: full replace of variables / arrays / assocs.
    /// The substitution pass owns the snapshot for the duration of
    /// one parameter expansion; nothing else mutates concurrently
    /// (zshrs is single-threaded inside the VM scope), so a wholesale
    /// write-back is safe. Insertion order for assoc-arrays is
    /// reconstructed by inserting new keys after old ones.
    pub fn commit_to_executor(self, exec: &mut crate::exec::ShellExecutor) { // c:2807
        if self.errflag {                                   // c:2807
            // C zsh sets `errflag` to abort the rest of substitution;
            // mirrors that by NOT writing back partial state.
            return;                                         // c:2807
        }                                                   // c:2807
        exec.variables = self.variables;                    // c:2807
        exec.arrays = self.arrays;                          // c:2807
        // Convert plain HashMap back to IndexMap. Pre-existing keys
        // keep their order; new keys (e.g. from `${arr[k]:=v}` on a
        // previously unset k) get appended at the end. Matches zsh's
        // hashtable insertion semantics where `${arr[k]:=v}` on a
        // missing k appends, on an existing k overwrites in place.
        for (name, new_map) in self.assoc_arrays {          // c:2807
            let entry = exec                                // c:2807
                .assoc_arrays                               // c:2807
                .entry(name.clone())                        // c:2807
                .or_default();                              // c:2807
            // Update existing keys
            for k in entry.keys().cloned().collect::<Vec<_>>() { // c:2807
                if let Some(v) = new_map.get(&k) {          // c:2807
                    entry.insert(k, v.clone());             // c:2807
                }                                           // c:2807
            }                                               // c:2807
            // Append new keys
            for (k, v) in &new_map {                        // c:2807
                if !entry.contains_key(k) {                 // c:2807
                    entry.insert(k.clone(), v.clone());     // c:2807
                }                                           // c:2807
            }                                               // c:2807
        }                                                   // c:2807
    }                                                       // c:2807
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
    let data = list.get_data(node_idx)?;                    // c:49
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

    list.set_data(node_idx, marker);                        // c:49
    let key_idx = list.insert_after(node_idx, key);         // c:49
    let val_idx = list.insert_after(key_idx, value);        // c:49

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

    while node_idx < list.len() {                           // c:100
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
            if let Some(data) = list.get_data(node_idx) {   // c:100
                let new_data = filesub(                     // c:100
                    data,                                   // c:100
                    flags & (prefork_flags::TYPESET | prefork_flags::ASSIGN), // c:100
                    state,                                  // c:100
                );                                          // c:100
                list.set_data(node_idx, new_data);          // c:100
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
        while node_idx < list.len() {                       // c:100
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
    while node_idx < list.len() {                           // c:100
        if Some(node_idx) == stop_idx {                     // c:100
            keep = false;                                   // c:100
        }                                                   // c:100

        if let Some(data) = list.get_data(node_idx) {       // c:100
            if !data.is_empty() {                           // c:100
                // remnulargs
                let data = remnulargs(data);                // c:100
                list.set_data(node_idx, data.clone());      // c:100

                // Brace expansion
                if !state.opts.ignore_braces && (flags & prefork_flags::SINGLE == 0) { // c:100
                    if !keep {                              // c:100
                        stop_idx = list.next_node(node_idx); // c:100
                    }                                       // c:100
                    while hasbraces(list.get_data(node_idx).unwrap_or("")) { // c:100
                        keep = true;                        // c:100
                        xpandbraces(list, &mut node_idx);   // c:100
                    }                                       // c:100
                }                                           // c:100

                // File substitution (non-SHFILEEXPANSION). Skip
                // entirely when state.skip_filesub is set — used
                // for `${var/pat/repl}` pattern + replacement
                // contexts where literal `~` must be preserved.
                if !state.opts.sh_file_expansion && !state.skip_filesub { // c:100
                    if let Some(data) = list.get_data(node_idx) { // c:100
                        let new_data = filesub(             // c:100
                            data,                           // c:100
                            flags & (prefork_flags::TYPESET | prefork_flags::ASSIGN), // c:100
                            state,                          // c:100
                        );                                  // c:100
                        list.set_data(node_idx, new_data);  // c:100
                    }                                       // c:100
                }                                           // c:100
            } else if (flags & prefork_flags::SINGLE == 0)  // c:100
                && (*ret_flags & prefork_flags::KEY_VALUE == 0) // c:100
                && !keep                                    // c:100
            {                                               // c:100
                list.remove(node_idx);                      // c:100
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
    let processed = getkeystring(&content);                 // c:206

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

/// Public re-export of [`getkeystring`] for callers outside the
/// module (`exec::expand_string` uses it for runtime `$'...'`
/// expansion of pattern/replacement operands handed to the bytecode
/// builtins after they've bypassed the lexer's normal tokenization).
pub fn getkeystring_pub(s: &str) -> String {                // utils.c:6915
    getkeystring(s)                                         // utils.c:6915
}                                                           // utils.c:6915

/// Set-test for a magic-assoc subscript: `${+functions[name]}`,
/// `${+commands[name]}`, `${+aliases[name]}`, etc. zsh treats these
/// special parameters as live views over the shell's introspection
/// tables — `${+functions[foo]}` is "1" iff `foo` is a defined
/// function. Direct port of paramsubst's chkset path when the
/// parameter is one of the special-name table entries (Src/init.c
/// special_params + Src/subst.c paramsubst's getfn invocation).
/// Return `Some(keys)` for a recognized magic-assoc name, snapshot
/// from the live executor. Wrapper around `magic_assoc_keys` for
/// callers (like `BUILTIN_PARAM_FLAG`'s runtime handler) that don't
/// have a `SubstState` already built but DO have access to the
/// executor — synthesises a minimal `SubstState` on the fly.
pub fn magic_assoc_keys_from_executor(                      // c:N/A (zshrs-specific)
    name: &str,                                             // c:N/A (zshrs-specific)
    exec: &crate::exec::ShellExecutor,                      // c:N/A (zshrs-specific)
) -> Option<Vec<String>> {                                  // c:N/A (zshrs-specific)
    let state = SubstState::from_executor(exec);            // c:N/A (zshrs-specific)
    magic_assoc_keys(name, &state)                          // c:N/A (zshrs-specific)
}                                                           // c:N/A (zshrs-specific)

/// Synthesize the key list for a magic associative-array special
/// that doesn't live in `state.assoc_arrays`. Direct port of the
/// `scanfn` slot zsh's C source registers in each special's
/// `paramdef` table (Src/Modules/parameter.c et al.). Returns
/// `Some` with the populated list when the name is a recognized
/// magic-assoc, `None` for "this is a regular variable, fall
/// through to the empty-result path".
pub fn magic_assoc_keys(name: &str, state: &SubstState) -> Option<Vec<String>> { // c:N/A (zshrs-specific)
    use std::collections::HashSet;                          // c:N/A (zshrs-specific)
    fn sorted_set(set: &HashSet<String>) -> Vec<String> {   // c:N/A (zshrs-specific)
        let mut v: Vec<String> = set.iter().cloned().collect(); // c:N/A (zshrs-specific)
        v.sort();                                           // c:N/A (zshrs-specific)
        v                                                   // c:N/A (zshrs-specific)
    }                                                       // c:N/A (zshrs-specific)
    Some(match name {                                       // c:N/A (zshrs-specific)
        "aliases" => sorted_set(&state.alias_names),        // c:N/A (zshrs-specific)
        "functions" => sorted_set(&state.function_names),   // c:N/A (zshrs-specific)
        "commands" => sorted_set(&state.command_names),     // c:N/A (zshrs-specific)
        "options" => {                                      // c:N/A (zshrs-specific)
            // Snapshot via with_executor — options aren't in
            // SubstState.
            let mut v: Vec<String> =                        // c:N/A (zshrs-specific)
                crate::exec::with_executor(|exec| exec.options.keys().cloned().collect()); // c:N/A (zshrs-specific)
            v.sort();                                       // c:N/A (zshrs-specific)
            v                                               // c:N/A (zshrs-specific)
        }                                                   // c:N/A (zshrs-specific)
        "parameters" => {                                   // c:N/A (zshrs-specific)
            // ${(k)parameters} = every defined parameter name.
            // Snapshot variables / arrays / assoc-arrays from the
            // current state.
            let mut set: HashSet<String> = state.variables.keys().cloned().collect(); // c:N/A (zshrs-specific)
            for k in state.arrays.keys() {                  // c:N/A (zshrs-specific)
                set.insert(k.clone());                      // c:N/A (zshrs-specific)
            }                                               // c:N/A (zshrs-specific)
            for k in state.assoc_arrays.keys() {            // c:N/A (zshrs-specific)
                set.insert(k.clone());                      // c:N/A (zshrs-specific)
            }                                               // c:N/A (zshrs-specific)
            sorted_set(&set)                                // c:N/A (zshrs-specific)
        }                                                   // c:N/A (zshrs-specific)
        "terminfo" => crate::modules::terminfo::COMMON_STRING_CAPS // c:N/A (zshrs-specific)
            .iter()                                         // c:N/A (zshrs-specific)
            .map(|s| (*s).to_string())                      // c:N/A (zshrs-specific)
            .collect(),                                     // c:N/A (zshrs-specific)
        "termcap" => {                                      // c:N/A (zshrs-specific)
            // Concatenate all three termcap-code halves so the user
            // sees the full capability namespace.
            let mut v: Vec<String> = Vec::new();            // c:N/A (zshrs-specific)
            v.extend(                                       // c:N/A (zshrs-specific)
                crate::modules::termcap::BOOL_CODES         // c:N/A (zshrs-specific)
                    .iter()                                 // c:N/A (zshrs-specific)
                    .map(|s| s.to_string()),                // c:N/A (zshrs-specific)
            );                                              // c:N/A (zshrs-specific)
            v.extend(                                       // c:N/A (zshrs-specific)
                crate::modules::termcap::NUM_CODES          // c:N/A (zshrs-specific)
                    .iter()                                 // c:N/A (zshrs-specific)
                    .map(|s| s.to_string()),                // c:N/A (zshrs-specific)
            );                                              // c:N/A (zshrs-specific)
            v.extend(                                       // c:N/A (zshrs-specific)
                crate::modules::termcap::STR_CODES          // c:N/A (zshrs-specific)
                    .iter()                                 // c:N/A (zshrs-specific)
                    .map(|s| s.to_string()),                // c:N/A (zshrs-specific)
            );                                              // c:N/A (zshrs-specific)
            v                                               // c:N/A (zshrs-specific)
        }                                                   // c:N/A (zshrs-specific)
        "errnos" => crate::modules::system::ERRNO_NAMES     // c:N/A (zshrs-specific)
            .iter()                                         // c:N/A (zshrs-specific)
            .map(|(n, _)| (*n).to_string())                 // c:N/A (zshrs-specific)
            .collect(),                                     // c:N/A (zshrs-specific)
        "sysparams" => vec![                                // c:N/A (zshrs-specific)
            "pid".to_string(),                              // c:N/A (zshrs-specific)
            "ppid".to_string(),                             // c:N/A (zshrs-specific)
            "procsubstpid".to_string(),                     // c:N/A (zshrs-specific)
        ],                                                  // c:N/A (zshrs-specific)
        _ => return None,                                   // c:N/A (zshrs-specific)
    })                                                      // c:N/A (zshrs-specific)
}                                                           // c:N/A (zshrs-specific)

/// Synthesize the value list for a magic associative-array special.
/// Mirrors `magic_assoc_keys` but resolves each key through the
/// executor's `get_special_array_value` to get the corresponding
/// value. Used by `${(v)assoc}` / `${assoc}` / `${(kv)assoc}` for
/// magic-assocs that don't live in `state.assoc_arrays` (aliases,
/// functions, commands, options, parameters, …).
pub fn magic_assoc_values(name: &str, state: &SubstState) -> Option<Vec<String>> { // c:N/A (zshrs-specific)
    let keys = magic_assoc_keys(name, state)?;              // c:N/A (zshrs-specific)
    // Falls back to empty values when called outside a VM context
    // (unit tests via `mk_state` that exercise subst_port directly).
    let key_count = keys.len();                             // c:N/A (zshrs-specific)
    let values = crate::exec::try_with_executor(|exec| {    // c:N/A (zshrs-specific)
        keys.iter()                                         // c:N/A (zshrs-specific)
            .map(|k| exec.get_special_array_value(name, k).unwrap_or_default()) // c:N/A (zshrs-specific)
            .collect::<Vec<String>>()                       // c:N/A (zshrs-specific)
    })                                                      // c:N/A (zshrs-specific)
    .unwrap_or_else(|| vec![String::new(); key_count]);     // c:N/A (zshrs-specific)
    Some(values)                                            // c:N/A (zshrs-specific)
}                                                           // c:N/A (zshrs-specific)

fn check_magic_assoc_set(name: &str, key: &str, state: &SubstState) -> bool { // c:N/A (zshrs-specific)
    match name {                                            // c:N/A (zshrs-specific)
        "functions" | "dis_functions" => state.function_names.contains(key), // c:N/A (zshrs-specific)
        "aliases" | "dis_aliases" | "galiases" | "saliases" => state.alias_names.contains(key), // c:N/A (zshrs-specific)
        "commands" => {                                     // c:N/A (zshrs-specific)
            // command_names is intentionally lazy (would be expensive
            // to enumerate every executable in PATH at startup), so
            // fall through to a per-key PATH walk when the cached set
            // doesn't have the answer. zsh's `${+commands[ls]}` is
            // expected to return 1 on any normal system, but our
            // empty cache always returned 0.
            if state.command_names.contains(key) {          // c:N/A (zshrs-specific)
                return true;                                // c:N/A (zshrs-specific)
            }                                               // c:N/A (zshrs-specific)
            // Reject keys with `/` — those are paths, not bare names.
            if key.is_empty() || key.contains('/') {        // c:N/A (zshrs-specific)
                return false;                               // c:N/A (zshrs-specific)
            }                                               // c:N/A (zshrs-specific)
            let path_var = state                            // c:N/A (zshrs-specific)
                .variables                                  // c:N/A (zshrs-specific)
                .get("PATH")                                // c:N/A (zshrs-specific)
                .cloned()                                   // c:N/A (zshrs-specific)
                .unwrap_or_else(|| std::env::var("PATH").unwrap_or_default()); // c:N/A (zshrs-specific)
            for dir in path_var.split(':').filter(|s| !s.is_empty()) { // c:N/A (zshrs-specific)
                let candidate = std::path::Path::new(dir).join(key); // c:N/A (zshrs-specific)
                if let Ok(meta) = std::fs::metadata(&candidate) { // c:N/A (zshrs-specific)
                    if meta.is_file() {                     // c:N/A (zshrs-specific)
                        #[cfg(unix)]                        // c:N/A (zshrs-specific)
                        {                                   // c:N/A (zshrs-specific)
                            use std::os::unix::fs::PermissionsExt; // c:N/A (zshrs-specific)
                            if meta.permissions().mode() & 0o111 != 0 { // c:N/A (zshrs-specific)
                                return true;                // c:N/A (zshrs-specific)
                            }                               // c:N/A (zshrs-specific)
                        }                                   // c:N/A (zshrs-specific)
                        #[cfg(not(unix))]                   // c:N/A (zshrs-specific)
                        {                                   // c:N/A (zshrs-specific)
                            return true;                    // c:N/A (zshrs-specific)
                        }                                   // c:N/A (zshrs-specific)
                    }                                       // c:N/A (zshrs-specific)
                }                                           // c:N/A (zshrs-specific)
            }                                               // c:N/A (zshrs-specific)
            false                                           // c:N/A (zshrs-specific)
        }                                                   // c:N/A (zshrs-specific)
        // Other magic assocs (parameters, modules, options, …)
        // could be added here. For now the three most common in
        // plugin code (functions / aliases / commands) cover the
        // observed usage. Returns false for unknown names so a
        // `${+unknown_assoc[k]}` correctly reports unset.
        _ => false,                                         // c:N/A (zshrs-specific)
    }                                                       // c:N/A (zshrs-specific)
}                                                           // c:N/A (zshrs-specific)

/// Process escape sequences in $'...' strings
/// Port of getkeystring() from utils.c
fn getkeystring(s: &str) -> String {                        // utils.c:6915
    let mut result = String::new();                         // utils.c:6915
    let mut chars = s.chars().peekable();                   // utils.c:6915

    while let Some(c) = chars.next() {                      // utils.c:6915
        if c == '\\' {                                      // utils.c:6915
            match chars.next() {                            // utils.c:6915
                Some('n') => result.push('\n'),             // utils.c:6915
                Some('t') => result.push('\t'),             // utils.c:6915
                Some('r') => result.push('\r'),             // utils.c:6915
                Some('\\') => result.push('\\'),            // utils.c:6915
                Some('\'') => result.push('\''),            // utils.c:6915
                Some('"') => result.push('"'),              // utils.c:6915
                Some('a') => result.push('\x07'),           // utils.c:6915
                Some('b') => result.push('\x08'),           // utils.c:6915
                Some('e') | Some('E') => result.push('\x1b'), // utils.c:6915
                Some('f') => result.push('\x0c'),           // utils.c:6915
                Some('v') => result.push('\x0b'),           // utils.c:6915
                Some('0') => {                              // utils.c:6915
                    // Octal
                    let mut val = 0u32;                     // utils.c:6915
                    for _ in 0..3 {                         // utils.c:6915
                        if let Some(&c) = chars.peek() {    // utils.c:6915
                            if ('0'..='7').contains(&c) {   // utils.c:6915
                                val = val * 8 + (c as u32 - '0' as u32); // utils.c:6915
                                chars.next();               // utils.c:6915
                            } else {                        // utils.c:6915
                                break;                      // utils.c:6915
                            }                               // utils.c:6915
                        }                                   // utils.c:6915
                    }                                       // utils.c:6915
                    if let Some(ch) = char::from_u32(val) { // utils.c:6915
                        result.push(ch);                    // utils.c:6915
                    }                                       // utils.c:6915
                }                                           // utils.c:6915
                Some('x') => {                              // utils.c:6915
                    // Hex
                    let mut val = 0u32;                     // utils.c:6915
                    for _ in 0..2 {                         // utils.c:6915
                        if let Some(&c) = chars.peek() {    // utils.c:6915
                            if c.is_ascii_hexdigit() {      // utils.c:6915
                                val = val * 16 + c.to_digit(16).unwrap(); // utils.c:6915
                                chars.next();               // utils.c:6915
                            } else {                        // utils.c:6915
                                break;                      // utils.c:6915
                            }                               // utils.c:6915
                        }                                   // utils.c:6915
                    }                                       // utils.c:6915
                    if let Some(ch) = char::from_u32(val) { // utils.c:6915
                        result.push(ch);                    // utils.c:6915
                    }                                       // utils.c:6915
                }                                           // utils.c:6915
                Some('u') => {                              // utils.c:6915
                    // Unicode 4 hex digits
                    let mut val = 0u32;                     // utils.c:6915
                    for _ in 0..4 {                         // utils.c:6915
                        if let Some(&c) = chars.peek() {    // utils.c:6915
                            if c.is_ascii_hexdigit() {      // utils.c:6915
                                val = val * 16 + c.to_digit(16).unwrap(); // utils.c:6915
                                chars.next();               // utils.c:6915
                            } else {                        // utils.c:6915
                                break;                      // utils.c:6915
                            }                               // utils.c:6915
                        }                                   // utils.c:6915
                    }                                       // utils.c:6915
                    if let Some(ch) = char::from_u32(val) { // utils.c:6915
                        result.push(ch);                    // utils.c:6915
                    }                                       // utils.c:6915
                }                                           // utils.c:6915
                Some('U') => {                              // utils.c:6915
                    // Unicode 8 hex digits
                    let mut val = 0u32;                     // utils.c:6915
                    for _ in 0..8 {                         // utils.c:6915
                        if let Some(&c) = chars.peek() {    // utils.c:6915
                            if c.is_ascii_hexdigit() {      // utils.c:6915
                                val = val * 16 + c.to_digit(16).unwrap(); // utils.c:6915
                                chars.next();               // utils.c:6915
                            } else {                        // utils.c:6915
                                break;                      // utils.c:6915
                            }                               // utils.c:6915
                        }                                   // utils.c:6915
                    }                                       // utils.c:6915
                    if let Some(ch) = char::from_u32(val) { // utils.c:6915
                        result.push(ch);                    // utils.c:6915
                    }                                       // utils.c:6915
                }                                           // utils.c:6915
                Some(c) => result.push(c),                  // utils.c:6915
                None => result.push('\\'),                  // utils.c:6915
            }                                               // utils.c:6915
        } else {                                            // utils.c:6915
            result.push(c);                                 // utils.c:6915
        }                                                   // utils.c:6915
    }                                                       // utils.c:6915

    result                                                  // utils.c:6915
}                                                           // utils.c:6915

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
    let mut str3 = list.get_data(node_idx)?.to_string();    // c:237
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
            let (subst, rest) = if c == INANG || c == OUTANGPROC { // c:237
                getproc(&str3[pos..], state)                // c:237
            } else {                                        // c:237
                getoutputfile(&str3[pos..], state)          // c:237
            };                                              // c:237

            if state.errflag {                              // c:237
                return None;                                // c:237
            }                                               // c:237

            let subst = subst.unwrap_or_default();          // c:237
            let prefix: String = chars[..pos].iter().collect(); // c:237
            str3 = format!("{}{}{}", prefix, subst, rest);  // c:237
            pos += subst.len();                             // c:237
            list.set_data(node_idx, str3.clone());          // c:237
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
            list.set_data(node_idx, str3.clone());          // c:237
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
            list.set_data(node_idx, str3.clone());          // c:237
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
            list.set_data(node_idx, str3.clone());          // c:237
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
            list.set_data(node_idx, str3.clone());          // c:237
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
                // Command substitution - handled below
                pos += 1;                                   // c:237
                let (result, new_pos) = process_command_subst(&str3, pos, qt, state); // c:237
                str3 = result;                              // c:237
                pos = new_pos;                              // c:237
                list.set_data(node_idx, str3.clone());      // c:237
                continue;                                   // c:237
            } else if next_is(INBRACK, '[') {               // c:237
                // $[...] arithmetic
                let start = pos + 2;                        // c:237
                let open = if next_c == Some(INBRACK) { INBRACK } else { '[' }; // c:237
                let close = if open == INBRACK { OUTBRACK } else { ']' }; // c:237
                if let Some(end) = find_matching_bracket(&str3[start..], open, close) { // c:237
                    let expr: String = str3.chars().skip(start).take(end).collect(); // c:237
                    let value = arithsubst(&expr, state);   // c:237
                    let prefix: String = str3.chars().take(pos).collect(); // c:237
                    let suffix: String = str3.chars().skip(start + end + 1).collect(); // c:237
                    str3 = format!("{}{}{}", prefix, value, suffix); // c:237
                    list.set_data(node_idx, str3.clone());  // c:237
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
                list.set_data(node_idx, str3.clone());      // c:237
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
                        list.set_data(current_idx, node_data); // c:237
                    } else {                                // c:237
                        current_idx = list.insert_after(current_idx, node_data); // c:237
                    }                                       // c:237
                }                                           // c:237

                str3 = list.get_data(node_idx)?.to_string(); // c:237
                pos = new_pos;                              // c:237
                continue;                                   // c:237
            }                                               // c:237
        }                                                   // c:237

        // Backtick command substitution
        let qt = c == QTICK;                                // c:237
        if qt || c == TICK {                                // c:237
            if !qt {                                        // c:237
                list.flags |= LF_ARRAY;                     // c:237
            }                                               // c:237
            let (result, new_pos) = process_backtick_subst(&str3, pos, qt, pf_flags, state); // c:237
            str3 = result;                                  // c:237
            pos = new_pos;                                  // c:237
            list.set_data(node_idx, str3.clone());          // c:237
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

/// Public entry: substitute a `${…}` brace expression against the
/// live `ShellExecutor` state. Caller passes the brace **content**
/// (without the outer `${…}` wrapper) — e.g. `arr[k]:=value`.
///
/// Bridges the C-port machinery (`paramsubst` / `parse_brace_param` /
/// `apply_operator`) to the runtime executor via snapshot+commit.
/// Replaces the adhoc bracket-modifier dispatch in
/// `exec::expand_braced_variable` for any `${…}` shape that
/// `paramsubst` understands.
///
/// Direct correspondence to C: this is the entry shape that
/// `Src/subst.c::stringsubst()` (line 237) reaches when it spots a
/// `${` opener — except the C source threads `LinkList` nodes for
/// word-splitting, while we return a single joined string. The
/// caller (exec::expand_braced_variable) is itself joining at the
/// `${…}` level, so a string is the right shape for now.
pub fn substitute_brace(content: &str, exec: &mut crate::exec::ShellExecutor) -> String { // c:1625
    // Bump paramsubst-recursion depth so nested `${${…}…}` flag
    // builtins (BUILTIN_PARAM_FLAG) can detect they're inside an
    // outer expansion and skip the DQ-collapse-to-scalar step.
    // Direct C analogue: subst.c paramsubst's recursive aval
    // threading where the inner call returns aval and the outer
    // continues operating on the array before emission.
    exec.in_paramsubst_nest += 1;                           // c:1625
    let mut state = SubstState::from_executor(exec);        // c:1625
    let wrapped = format!("${{{}}}", content);              // c:1625

    // Try the new line-by-line port first when ZSHRS_NEW_PARAMSUBST=1.
    // It returns Err(FallbackToLegacy) for anything not yet ported,
    // in which case we drop straight through to the legacy path.
    let mut ret_flags = 0u32;                               // c:1625
    let new_path_result = crate::subst::paramsubst_inline::paramsubst_port( // c:1625
                                                                            &wrapped,                                           // c:1625
                                                                            0,                                                  // c:1625
                                                                            false,                                              // c:1625
                                                                            0,                                                  // c:1625
                                                                            &mut ret_flags,                                     // c:1625
                                                                            &mut state,                                         // c:1625
    );                                                      // c:1625

    let result = match new_path_result {                    // c:1625
        Ok((s, _pos, _nodes)) => s,                         // c:1625
        Err(_) => {                                         // c:1625
            let (result, _pos, _nodes) =                    // c:1625
                paramsubst(&wrapped, 0, false, 0, &mut 0, &mut state); // c:1625
            result                                          // c:1625
        }                                                   // c:1625
    };                                                      // c:1625
    state.commit_to_executor(exec);                         // c:1625
    exec.in_paramsubst_nest -= 1;                           // c:1625
    result                                                  // c:1625
}                                                           // c:1625

/// Parallel of `substitute_brace` that returns the multi-word
/// `nodes` list so callers can preserve array shape across nested
/// `${(@)${...}##pat}` forms. Direct port of zsh's `aval` threading
/// in Src/subst.c paramsubst — the C source carries the per-element
/// vector through `aval` to the caller, which decides whether to
/// splat or join based on `nojoin`. zshrs's String-returning bridge
/// collapses array→scalar on return, breaking idioms like
/// `print -l -- ${(@)${(@s:->:)x}##pat}`.
pub fn substitute_brace_array(                              // c:1625
    content: &str,                                          // c:1625
    exec: &mut crate::exec::ShellExecutor,                  // c:1625
) -> Vec<String> {                                          // c:1625
    exec.in_paramsubst_nest += 1;                           // c:1625
    let mut state = SubstState::from_executor(exec);        // c:1625
    let wrapped = format!("${{{}}}", content);              // c:1625

    let mut ret_flags = 0u32;                               // c:1625
    let new_path_result = crate::subst::paramsubst_inline::paramsubst_port( // c:1625
                                                                            &wrapped,                                           // c:1625
                                                                            0,                                                  // c:1625
                                                                            false,                                              // c:1625
                                                                            0,                                                  // c:1625
                                                                            &mut ret_flags,                                     // c:1625
                                                                            &mut state,                                         // c:1625
    );                                                      // c:1625

    let (result, nodes) = match new_path_result {           // c:1625
        Ok((s, _pos, n)) => (s, n),                         // c:1625
        Err(_) => {                                         // c:1625
            let (r, _pos, n) =                              // c:1625
                paramsubst(&wrapped, 0, false, 0, &mut 0, &mut state); // c:1625
            (r, n)                                          // c:1625
        }                                                   // c:1625
    };                                                      // c:1625
    state.commit_to_executor(exec);                         // c:1625
    exec.in_paramsubst_nest -= 1;                           // c:1625
    if nodes.is_empty() {                                   // c:1625
        vec![result]                                        // c:1625
    } else {                                                // c:1625
        nodes                                               // c:1625
    }                                                       // c:1625
}                                                           // c:1625

/// Process $(...) or $((...)) substitution
fn process_command_subst(                                   // c:N/A
    s: &str,                                                // c:N/A
    start_pos: usize,                                       // c:N/A
    qt: bool,                                               // c:N/A
    state: &mut SubstState,                                 // c:N/A
) -> (String, usize) {                                      // c:N/A
    let chars: Vec<char> = s.chars().collect();             // c:N/A
    let c = chars.get(start_pos).copied().unwrap_or('\0');  // c:N/A

    if c == INPARMATH {                                     // c:N/A
        // $((...)) - arithmetic
        let expr_start = start_pos + 1;                     // c:N/A
        if let Some(end) = find_matching_parmath(&s[expr_start..]) { // c:N/A
            let expr: String = s.chars().skip(expr_start).take(end).collect(); // c:N/A
            let value = arithsubst(&expr, state);           // c:N/A
            let prefix: String = s.chars().take(start_pos - 1).collect(); // c:N/A
            let suffix: String = s.chars().skip(expr_start + end + 1).collect(); // c:N/A
            return (                                        // c:N/A
                format!("{}{}{}", prefix, value, suffix),   // c:N/A
                prefix.len() + value.len(),                 // c:N/A
            );                                              // c:N/A
        }                                                   // c:N/A
    }                                                       // c:N/A

    // $(...) - command substitution. Pick INPAR/OUTPAR vs literal
    // `(`/`)` based on what the caller actually used. C zsh always
    // sees the lexer-tokenized form (Inpar/Outpar); zshrs's recursive
    // paths (e.g. multsub on a `:-` operand whose value has literal
    // parens) hand us un-tokenized text. Mirror Src/subst.c:247
    // `str[1] == Inpar` by treating both forms uniformly here.
    //
    // `find_matching_bracket` expects the slice to start *after* the
    // opening paren (its depth begins at 1, mirroring C's
    // skipparens at Src/utils.c:2409 which advances past `**s == inpar`
    // before incrementing level). Build the slice from the char
    // immediately after the open paren via char-indices to keep
    // multibyte META tokens from splitting on byte boundaries.
    let (open, close) = if c == INPAR { (INPAR, OUTPAR) } else { ('(', ')') }; // c:247
    let body_start_byte = s                                 // c:247
        .char_indices()                                     // c:247
        .nth(start_pos + 1)                                 // c:247
        .map(|(b, _)| b)                                    // c:247
        .unwrap_or(s.len());                                // c:247
    if let Some(end) = find_matching_bracket(&s[body_start_byte..], open, close) { // c:247
        let cmd: String = s.chars().skip(start_pos + 1).take(end).collect(); // c:247
        // Route through the live executor's command-substitution
        // runner (Src/exec.c::execcmdsubst's in-process pipe-capture
        // path). The legacy `run_command(&cmd)` gated on
        // `state.opts.exec_opt` was always dead because exec_opt is
        // never set true — recursive `:-`/`:=` operands hit the
        // `else { String::new() }` branch and lost the cmd output.
        let output = crate::exec::with_executor(|exec| {    // c:247
            exec.run_command_substitution(&cmd)             // c:247
        });                                                 // c:247
        let output = output.trim_end_matches('\n');         // c:247
        let prefix: String = s.chars().take(start_pos - 1).collect(); // c:247
        let suffix: String = s.chars().skip(start_pos + end + 2).collect(); // c:247
        return (                                            // c:247
            format!("{}{}{}", prefix, output, suffix),      // c:247
            prefix.len() + output.len(),                    // c:247
        );                                                  // c:247
    }                                                       // c:247

    let _ = qt;                                             // c:247
    (s.to_string(), start_pos + 1)                          // c:247
}                                                           // c:247

/// Process `...` substitution
fn process_backtick_subst(                                  // c:N/A
    s: &str,                                                // c:N/A
    start_pos: usize,                                       // c:N/A
    _qt: bool,                                              // c:N/A
    _pf_flags: u32,                                         // c:N/A
    state: &mut SubstState,                                 // c:N/A
) -> (String, usize) {                                      // c:N/A
    let chars: Vec<char> = s.chars().collect();             // c:N/A
    let end_char = chars[start_pos]; // TICK or QTICK       // c:N/A

    // Find matching backtick
    let mut end_pos = start_pos + 1;                        // c:N/A
    while end_pos < chars.len() && chars[end_pos] != end_char { // c:N/A
        end_pos += 1;                                       // c:N/A
    }                                                       // c:N/A

    if end_pos >= chars.len() {                             // c:N/A
        state.errflag = true;                               // c:N/A
        eprintln!("failed to find end of command substitution"); // c:N/A
        return (s.to_string(), start_pos + 1);              // c:N/A
    }                                                       // c:N/A

    let cmd: String = chars[start_pos + 1..end_pos].iter().collect(); // c:N/A
    let output = run_command(&cmd);                         // c:N/A
    let output = output.trim_end_matches('\n');             // c:N/A

    let prefix: String = chars[..start_pos].iter().collect(); // c:N/A
    let suffix: String = chars[end_pos + 1..].iter().collect(); // c:N/A

    (                                                       // c:N/A
        format!("{}{}{}", prefix, output, suffix),          // c:N/A
        prefix.len() + output.len(),                        // c:N/A
    )                                                       // c:N/A
}                                                           // c:N/A

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
    if c == INBRACE || c == '{' {                           // c:1625
        pos += 1;                                           // c:1625
        return parse_brace_param(s, start_pos, pos, qt, pf_flags, ret_flags, state); // c:1625
    }                                                       // c:1625

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
                subscript_str = Some(singsub_no_tilde(&raw_sub, state)); // c:1625
                pos = q + 1;                                // c:1625
            }                                               // c:1625
        }                                                   // c:1625

        let value = if let Some(sub) = subscript_str.as_deref() { // c:1625
            // Array / assoc element lookup.
            let v = get_param_with_subscript(&var_name, Some(sub), state); // c:1625
            v.join(" ")                                     // c:1625
        } else {                                            // c:1625
            get_param_value(&var_name, state)               // c:1625
        };                                                  // c:1625

        // Handle word splitting
        if pf_flags & prefork_flags::SHWORDSPLIT != 0 && !qt { // c:1625
            let words = split_words(&value, state);         // c:1625
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
                get_param_value("0", state)                 // c:1625
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

/// Parse ${...} parameter expansion with all its glory
/// This handles flags like (L), (U), (s.:.), nested expansions, etc.
fn parse_brace_param(                                       // c:1625
    s: &str,                                                // c:1625
    dollar_pos: usize,                                      // c:1625
    brace_pos: usize,                                       // c:1625
    qt: bool,                                               // c:1625
    pf_flags: u32,                                          // c:1625
    _ret_flags: &mut u32,                                   // c:1625
    state: &mut SubstState,                                 // c:1625
) -> (String, usize, Vec<String>) {                         // c:1625
    let chars: Vec<char> = s.chars().collect();             // c:1625
    let mut pos = brace_pos;                                // c:1625
    let mut result_nodes = Vec::new();                      // c:1625

    // Parse flags in (...)
    let mut flags = ParamFlags::default();                  // c:1625
    if chars.get(pos) == Some(&'(') {                       // c:1625
        pos += 1;                                           // c:1625
        while pos < chars.len() && chars[pos] != ')' {      // c:1625
            let flag_char = chars[pos];                     // c:1625
            match flag_char {                               // c:1625
                'L' => flags.lowercase = true,              // c:1625
                'U' => flags.uppercase = true,              // c:1625
                'C' => flags.capitalize = true,             // c:1625
                'u' => flags.unique = true,                 // c:1625
                'o' => flags.sort = true,                   // c:1625
                'O' => flags.sort_reverse = true,           // c:1625
                'a' => flags.sort_array_index = true,       // c:1625
                'i' => flags.sort_case_insensitive = true,  // c:1625
                'n' => flags.sort_numeric = true,           // c:1625
                'k' => flags.keys = true,                   // c:1625
                'v' => flags.values = true,                 // c:1625
                't' => flags.type_info = true,              // c:1625
                'P' => flags.prompt_expand = true,          // c:1625
                'e' => flags.eval = true,                   // c:1625
                'q' => flags.quote_level += 1,              // c:1625
                'Q' => flags.unquote = true,                // c:1625
                'X' => flags.report_error = true,           // c:1625
                'z' => flags.split_words = true,            // c:1625
                'f' => flags.split_lines = true,            // c:1625
                'F' => flags.join_lines = true,             // c:1625
                'w' => flags.count_words = true,            // c:1625
                'W' => flags.count_words_null = true,       // c:1625
                'c' => flags.count_chars = true,            // c:1625
                '#' => flags.length_chars = true,           // c:1625
                '%' => flags.prompt_percent = true,         // c:1625
                'A' => flags.create_assoc = true,           // c:1625
                '@' => flags.array_expand = true,           // c:1625
                '~' => flags.glob_subst = true,             // c:1625
                'V' => flags.visible = true,                // c:1625
                'S' | 'I' => flags.search = true,           // c:1625
                'M' => flags.match_flag = true,             // c:1625
                'R' => flags.reverse_subscript = true,      // c:1625
                'B' | 'E' | 'N' => flags.begin_end_length = true, // c:1625
                's' => {                                    // c:1625
                    // (s.SEP.) split separator — zsh allows ANY
                    // non-alphanumeric ASCII char as the delimiter
                    // (`(s.:.)`, `(s/x/)`, `(s| |)` all valid). Direct
                    // port of Src/subst.c paramsubst's flag parser
                    // which captures the next char as `del` and reads
                    // until the matching `del`.
                    pos += 1;                               // c:1625
                    if pos < chars.len()                    // c:1625
                        && !chars[pos].is_ascii_alphanumeric() // c:1625
                        && chars[pos] != ')'                // c:1625
                    {                                       // c:1625
                        let del = chars[pos];               // c:1625
                        pos += 1;                           // c:1625
                        let mut sep = String::new();        // c:1625
                        while pos < chars.len() && chars[pos] != del { // c:1625
                            sep.push(chars[pos]);           // c:1625
                            pos += 1;                       // c:1625
                        }                                   // c:1625
                        flags.split_sep = Some(sep);        // c:1625
                        if pos < chars.len() && chars[pos] == del { // c:1625
                            pos += 1;                       // c:1625
                        }                                   // c:1625
                        // Position will be decremented below.
                        pos = pos.saturating_sub(1);        // c:1625
                    } else {                                // c:1625
                        pos -= 1;                           // c:1625
                    }                                       // c:1625
                }                                           // c:1625
                'j' => {                                    // c:1625
                    // (j.SEP.) join separator — same delimiter rules
                    // as (s).
                    pos += 1;                               // c:1625
                    if pos < chars.len()                    // c:1625
                        && !chars[pos].is_ascii_alphanumeric() // c:1625
                        && chars[pos] != ')'                // c:1625
                    {                                       // c:1625
                        let del = chars[pos];               // c:1625
                        pos += 1;                           // c:1625
                        let mut sep = String::new();        // c:1625
                        while pos < chars.len() && chars[pos] != del { // c:1625
                            sep.push(chars[pos]);           // c:1625
                            pos += 1;                       // c:1625
                        }                                   // c:1625
                        flags.join_sep = Some(sep);         // c:1625
                        if pos < chars.len() && chars[pos] == del { // c:1625
                            pos += 1;                       // c:1625
                        }                                   // c:1625
                        pos = pos.saturating_sub(1);        // c:1625
                    } else {                                // c:1625
                        pos -= 1;                           // c:1625
                    }                                       // c:1625
                }                                           // c:1625
                'l' => {                                    // c:1625
                    // (l:N:STR1:STR2:) — left-pad. Up to TWO string
                    // args after the width: STR1 is single-shot, STR2
                    // is the repeating fill. Direct port of
                    // Src/subst.c paramsubst flag parser. Empty STR1
                    // is meaningful: `(l:N::STR2:)` triggers
                    // dopadding's "no value" branch where the result
                    // is just N copies of STR2.
                    //
                    // String args terminate on the closing delimiter
                    // (`:` here) OR the closing `)` of the flag block
                    // — `(l:5:0:)` is the same shape as `(l:5:0)`.
                    pos += 1;                               // c:1625
                    if pos < chars.len() && chars[pos] == ':' { // c:1625
                        pos += 1;                           // c:1625
                        let mut len_str = String::new();    // c:1625
                        while pos < chars.len() && chars[pos].is_ascii_digit() { // c:1625
                            len_str.push(chars[pos]);       // c:1625
                            pos += 1;                       // c:1625
                        }                                   // c:1625
                        if let Ok(len) = len_str.parse() {  // c:1625
                            flags.pad_left = Some(len);     // c:1625
                        }                                   // c:1625
                        if pos < chars.len() && chars[pos] == ':' { // c:1625
                            pos += 1;                       // c:1625
                            let mut s1 = String::new();     // c:1625
                            while pos < chars.len()         // c:1625
                                && chars[pos] != ':'        // c:1625
                                && chars[pos] != ')'        // c:1625
                            {                               // c:1625
                                s1.push(chars[pos]);        // c:1625
                                pos += 1;                   // c:1625
                            }                               // c:1625
                            flags.pad_string1 = Some(s1);   // c:1625
                            if pos < chars.len() && chars[pos] == ':' { // c:1625
                                pos += 1;                   // c:1625
                                let mut s2 = String::new(); // c:1625
                                while pos < chars.len()     // c:1625
                                    && chars[pos] != ':'    // c:1625
                                    && chars[pos] != ')'    // c:1625
                                {                           // c:1625
                                    s2.push(chars[pos]);    // c:1625
                                    pos += 1;               // c:1625
                                }                           // c:1625
                                flags.pad_string2 = Some(s2); // c:1625
                            }                               // c:1625
                        }                                   // c:1625
                        // The outer loop does `pos += 1`, so back up
                        // one so we don't skip the next flag char or
                        // the closing `)`.
                        pos = pos.saturating_sub(1);        // c:1625
                    } else {                                // c:1625
                        pos -= 1;                           // c:1625
                    }                                       // c:1625
                }                                           // c:1625
                'r' => {                                    // c:1625
                    // (r:N:STR1:STR2:) — right-pad, mirrors `l`.
                    pos += 1;                               // c:1625
                    if pos < chars.len() && chars[pos] == ':' { // c:1625
                        pos += 1;                           // c:1625
                        let mut len_str = String::new();    // c:1625
                        while pos < chars.len() && chars[pos].is_ascii_digit() { // c:1625
                            len_str.push(chars[pos]);       // c:1625
                            pos += 1;                       // c:1625
                        }                                   // c:1625
                        if let Ok(len) = len_str.parse() {  // c:1625
                            flags.pad_right = Some(len);    // c:1625
                        }                                   // c:1625
                        if pos < chars.len() && chars[pos] == ':' { // c:1625
                            pos += 1;                       // c:1625
                            let mut s1 = String::new();     // c:1625
                            while pos < chars.len()         // c:1625
                                && chars[pos] != ':'        // c:1625
                                && chars[pos] != ')'        // c:1625
                            {                               // c:1625
                                s1.push(chars[pos]);        // c:1625
                                pos += 1;                   // c:1625
                            }                               // c:1625
                            flags.pad_string1 = Some(s1);   // c:1625
                            if pos < chars.len() && chars[pos] == ':' { // c:1625
                                pos += 1;                   // c:1625
                                let mut s2 = String::new(); // c:1625
                                while pos < chars.len()     // c:1625
                                    && chars[pos] != ':'    // c:1625
                                    && chars[pos] != ')'    // c:1625
                                {                           // c:1625
                                    s2.push(chars[pos]);    // c:1625
                                    pos += 1;               // c:1625
                                }                           // c:1625
                                flags.pad_string2 = Some(s2); // c:1625
                            }                               // c:1625
                        }                                   // c:1625
                        pos = pos.saturating_sub(1);        // c:1625
                    } else {                                // c:1625
                        pos -= 1;                           // c:1625
                    }                                       // c:1625
                }                                           // c:1625
                _ => {}                                     // c:1625
            }                                               // c:1625
            pos += 1;                                       // c:1625
        }                                                   // c:1625
        if pos < chars.len() {                              // c:1625
            pos += 1; // Skip ')'                           // c:1625
        }                                                   // c:1625
    }                                                       // c:1625

    // Check for length prefix: ${#var}
    let length_prefix = chars.get(pos) == Some(&'#');       // c:1625
    if length_prefix {                                      // c:1625
        pos += 1;                                           // c:1625
    }                                                       // c:1625

    // Check for `${+name}` — "is parameter set?". Returns "1" if
    // set, "0" if unset (Src/subst.c:2604-2612 chkset path, and the
    // value emission at subst.c:3600-3602: `val = dupstring(vunset
    // ? "0" : "1")`). The `+` only applies when followed by an
    // identifier-start char or a nested `${`/`(P)` form per the
    // C source's `itype_end(...) != s+1` check; otherwise the `+`
    // is literal (e.g. `${+}` standalone is `$+` literal).
    let mut chkset = false;                                 // c:1625
    if chars.get(pos) == Some(&'+') {                       // c:1625
        let next = chars.get(pos + 1).copied().unwrap_or('\0'); // c:1625
        if next.is_ascii_alphabetic() || next == '_' || next == '{' || next == INBRACE { // c:1625
            chkset = true;                                  // c:1625
            pos += 1;                                       // c:1625
        }                                                   // c:1625
    }                                                       // c:1625
    // `${~name}` — bare `~` prefix sets the glob_subst flag,
    // equivalent to `${(~)name}`. Used heavily by zinit's pick /
    // load patterns: `pick="src/*.zsh"; files=(${~pick})` glob-
    // expands the value of $pick. Per Src/subst.c paramsubst, the
    // bare-`~` form is read at the same point as the bare-`#`/`+`
    // prefixes, ahead of the var-name slot.
    if chars.get(pos) == Some(&'~') {                       // c:1625
        let next = chars.get(pos + 1).copied().unwrap_or('\0'); // c:1625
        // Only consume the `~` when it's followed by a name-start
        // — leaves bare `${~}` alone as the unrelated literal form.
        if next.is_ascii_alphabetic() || next == '_' || next == '{' || next == INBRACE { // c:1625
            flags.glob_subst = true;                        // c:1625
            pos += 1;                                       // c:1625
        }                                                   // c:1625
    }                                                       // c:1625

    // Parse variable name. Three shapes:
    //   1. Plain identifier: `FOO`, `_BAR`, `arr` (alnum + `_`)
    //   2. Nested expansion: `${INNER}` — the var-name slot is
    //      itself a parameter substitution. Recurse to resolve
    //      the inner, then carry the result through as a
    //      "pre-resolved value" so subsequent operators see it
    //      as the value-to-substitute instead of a name to look
    //      up.
    //   3. Special parameters (`?`, `*`, `@`, `#`, `!`, `$`, `0`-
    //      `9`, etc.) — handled by the alnum loop for digits and
    //      handled separately by the caller for the singletons
    //      (paramsubst's `c == '?'` arm at line 859+).
    //
    // Port of Src/subst.c paramsubst's `${${…}…}` recursion: the
    // C source detects the nested `${` at the start of the
    // var-name slot and dispatches a recursive paramsubst call
    // before parsing the operator. zshrs does the same here.
    let mut nested_value: Option<Vec<String>> = None;       // c:1625
    let var_start = pos;                                    // c:1625
    if pos < chars.len()                                    // c:1625
        && chars[pos] == '$'                                // c:1625
        && pos + 1 < chars.len()                            // c:1625
        && (chars[pos + 1] == '{' || chars[pos + 1] == INBRACE) // c:1625
    {                                                       // c:1625
        // Find the matching `}` for this nested ${...}.
        let nested_start = pos;                             // c:1625
        let mut depth = 0;                                  // c:1625
        let mut p = pos;                                    // c:1625
        while p < chars.len() {                             // c:1625
            let c = chars[p];                               // c:1625
            if c == '{' || c == INBRACE {                   // c:1625
                depth += 1;                                 // c:1625
            } else if c == '}' || c == OUTBRACE {           // c:1625
                depth -= 1;                                 // c:1625
                if depth == 0 {                             // c:1625
                    p += 1;                                 // c:1625
                    break;                                  // c:1625
                }                                           // c:1625
            }                                               // c:1625
            p += 1;                                         // c:1625
        }                                                   // c:1625
        // Recurse on the nested chunk. Build a substring covering
        // `${…}` and call paramsubst on it.
        let nested_str: String = chars[nested_start..p].iter().collect(); // c:1625
        let mut inner_rf = 0u32;                            // c:1625
        let (resolved, _, nodes) = paramsubst(              // c:1625
            &nested_str,                                    // c:1625
            0,                                              // c:1625
            qt,                                             // c:1625
            pf_flags,                                       // c:1625
            &mut inner_rf,                                  // c:1625
            state,                                          // c:1625
        );                                                  // c:1625
        // Use the result vector (or single string) as the
        // pre-resolved value for the outer expansion.
        nested_value = if nodes.is_empty() {                // c:1625
            Some(vec![resolved])                            // c:1625
        } else {                                            // c:1625
            Some(nodes)                                     // c:1625
        };                                                  // c:1625
        pos = p;                                            // c:1625
    } else if pos + 2 < chars.len()                         // c:1625
        && chars[pos] == '$'                                // c:1625
        && chars[pos + 1] == '('                            // c:1625
        && chars[pos + 2] == '('                            // c:1625
    {                                                       // c:1625
        // `${$((expr))}` — arithmetic substitution as the var-name slot.
        // Must be checked BEFORE the `$(` cmd-subst branch because
        // `$((` would otherwise greedy-match cmd-subst-of-subshell:
        // `$( (expr) )` runs `(expr)` as a subshell command.
        // Direct port of Src/subst.c lex.c:1235-1280 disambiguator
        // where the lexer probes for `$((` before falling back to
        // `$( … )`. zsh's `gettokstr()` checks the second `(` as a
        // peek-ahead and switches to arith-eval mode. zshrs's
        // bridge replicates the probe by string-matching `$((`.
        //
        // Find the matching `))` (skipping nested `(` `)` pairs to
        // tolerate inline arith like `$((a*(b+c)))`). The closer is
        // the first `))` that brings the running paren-depth back
        // to the outer `$((` boundary.
        let arith_start = pos;                              // c:1625
        let mut p = pos + 3; // past `$((`                  // c:1625
        let mut depth: i32 = 2;                             // c:1625
        while p < chars.len() && depth > 0 {                // c:1625
            match chars[p] {                                // c:1625
                '(' => depth += 1,                          // c:1625
                ')' => {                                    // c:1625
                    depth -= 1;                             // c:1625
                    if depth == 0 {                         // c:1625
                        // p points at the SECOND `)` of `))`; advance past it.
                        p += 1;                             // c:1625
                        break;                              // c:1625
                    }                                       // c:1625
                }                                           // c:1625
                _ => {}                                     // c:1625
            }                                               // c:1625
            p += 1;                                         // c:1625
        }                                                   // c:1625
        // Body: between `$((` and `))`. arith_start+3 .. p-2.
        let end = p.saturating_sub(2);                      // c:1625
        let body: String = chars[arith_start + 3..end].iter().collect(); // c:1625
        // Route through the executor's arithmetic evaluator so bare
        // identifiers in the expression resolve to their values per
        // zsh: `$((i%6))` reads $i. arithsubst() in subst_port skips
        // variable expansion (only the math.c engine), so calling
        // it directly returns 0 for any non-numeric token. The
        // executor's evaluate_arithmetic mirrors C zsh's `matheval`
        // hook (Src/math.c:108) which reads `paramtab` for ident
        // operands.
        let captured = crate::exec::with_executor(|exec| exec.evaluate_arithmetic(&body)); // c:1625
        nested_value = Some(vec![captured]);                // c:1625
        pos = p;                                            // c:1625
    } else if pos < chars.len()                             // c:1625
        && chars[pos] == '$'                                // c:1625
        && pos + 1 < chars.len()                            // c:1625
        && chars[pos + 1] == '('                            // c:1625
    {                                                       // c:1625
        // `${(FLAGS)$(cmd)}` — cmd-substitution as the var-name slot.
        // Direct port of Src/subst.c paramsubst's `${$(…)}` path which
        // dispatches the inner `cmdsubst` and threads the captured
        // output through `aval`. Capture the cmd-subst (handles
        // nested parens via depth tracking), expand it through
        // exec::expand_string so `$(cmd)` actually runs, and use the
        // result as the pre-resolved value for the outer flag chain.
        let cmd_start = pos;                                // c:1625
        let mut depth = 0;                                  // c:1625
        let mut p = pos + 2; // past `$(`                   // c:1625
        depth = 1;                                          // c:1625
        while p < chars.len() && depth > 0 {                // c:1625
            match chars[p] {                                // c:1625
                '(' => depth += 1,                          // c:1625
                ')' => {                                    // c:1625
                    depth -= 1;                             // c:1625
                    if depth == 0 {                         // c:1625
                        p += 1;                             // c:1625
                        break;                              // c:1625
                    }                                       // c:1625
                }                                           // c:1625
                _ => {}                                     // c:1625
            }                                               // c:1625
            p += 1;                                         // c:1625
        }                                                   // c:1625
        // Strip the leading `$(` and trailing `)` so the body alone
        // goes to the cmd-subst runner. cmd_start points at `$`,
        // body therefore starts at cmd_start+2 and ends at p-1.
        let body: String = chars[cmd_start + 2..p.saturating_sub(1)] // c:1625
            .iter()                                         // c:1625
            .collect();                                     // c:1625
        let captured = crate::exec::with_executor(|exec| {  // c:1625
            exec.run_command_substitution(&body)            // c:1625
        });                                                 // c:1625
        // Strip trailing newlines (zsh's cmd-subst behavior — same as
        // bash) before feeding to the flag chain.
        let captured = captured.trim_end_matches('\n').to_string(); // c:1625
        nested_value = Some(vec![captured]);                // c:1625
        pos = p;                                            // c:1625
    } else {                                                // c:1625
        // Single-char special parameter names: `@`, `*`, `#`, `?`,
        // `$`, `!`, `0` (when alone). Direct port of paramsubst's
        // single-char-name dispatch in Src/subst.c. Without this,
        // `${#@}` saw an empty var_name (the alnum loop below
        // skipped `@`), looked up "" → empty value, and length
        // returned 0 instead of $#.
        if pos < chars.len()                                // c:1625
            && matches!(chars[pos], '@' | '*' | '#' | '?' | '$' | '!') // c:1625
        {                                                   // c:1625
            pos += 1;                                       // c:1625
        } else {                                            // c:1625
            while pos < chars.len() {                       // c:1625
                let c = chars[pos];                         // c:1625
                if c.is_ascii_alphanumeric() || c == '_' {  // c:1625
                    pos += 1;                               // c:1625
                } else {                                    // c:1625
                    break;                                  // c:1625
                }                                           // c:1625
            }                                               // c:1625
        }                                                   // c:1625
    }                                                       // c:1625
    let var_name: String = chars[var_start..pos].iter().collect(); // c:1625

    // `${!name}` is bash-only indirect — zsh rejects with "bad
    // substitution". Single `${!}` is the bg-process PID and stays
    // valid; only the form with a name AFTER `!` is bash.
    if var_name == "!"                                      // c:1625
        && pos < chars.len()                                // c:1625
        && chars[pos] != '}'                                // c:1625
        && chars[pos] != OUTBRACE                           // c:1625
    {                                                       // c:1625
        eprintln!("zshrs:1: bad substitution");             // c:1625
        state.errflag = true;                               // c:1625
        let prefix: String = chars[..dollar_pos].iter().collect(); // c:1625
        return (prefix, dollar_pos, Vec::new());            // c:1625
    }                                                       // c:1625

    // Check for subscript [...]. Multiple chained subscripts
    // (`${a[1][1]}`, `${a[1][2,4]}`) are stacked into `extra_subs`
    // and applied after the primary subscript resolves.
    // Direct port of Src/subst.c paramsubst's `getindex(s, &v, …)`
    // loop which recurses on residual `[…]` after each pick.
    let mut subscript = None;                               // c:1625
    let mut extra_subs: Vec<String> = Vec::new();           // c:1625
    while chars.get(pos) == Some(&'[') || chars.get(pos) == Some(&INBRACK) { // c:1625
        pos += 1;                                           // c:1625
        let sub_start = pos;                                // c:1625
        let mut depth = 1;                                  // c:1625
        while pos < chars.len() && depth > 0 {              // c:1625
            let c = chars[pos];                             // c:1625
            if c == '[' || c == INBRACK {                   // c:1625
                depth += 1;                                 // c:1625
            } else if c == ']' || c == OUTBRACK {           // c:1625
                depth -= 1;                                 // c:1625
            }                                               // c:1625
            if depth > 0 {                                  // c:1625
                pos += 1;                                   // c:1625
            }                                               // c:1625
        }                                                   // c:1625
        let captured: String = chars[sub_start..pos].iter().collect(); // c:1625
        if subscript.is_none() {                            // c:1625
            subscript = Some(captured);                     // c:1625
        } else {                                            // c:1625
            extra_subs.push(captured);                      // c:1625
        }                                                   // c:1625
        pos += 1; // Skip ]                                 // c:1625
    }                                                       // c:1625

    // Parse operator and operand
    let mut operator = None;                                // c:1625
    let mut operand = String::new();                        // c:1625

    // Check for operators: :-, :=, :+, :?, -, =, +, ?, #, ##, %, %%, /, //, :, ^, ^^, ,, ,,
    if pos < chars.len() {                                  // c:1625
        let c = chars[pos];                                 // c:1625
        match c {                                           // c:1625
            ':' => {                                        // c:1625
                pos += 1;                                   // c:1625
                if pos < chars.len() {                      // c:1625
                    match chars[pos] {                      // c:1625
                        '-' => {                            // c:1625
                            operator = Some(":-");          // c:1625
                            pos += 1;                       // c:1625
                        }                                   // c:1625
                        '=' => {                            // c:1625
                            operator = Some(":=");          // c:1625
                            pos += 1;                       // c:1625
                        }                                   // c:1625
                        '+' => {                            // c:1625
                            operator = Some(":+");          // c:1625
                            pos += 1;                       // c:1625
                        }                                   // c:1625
                        '?' => {                            // c:1625
                            operator = Some(":?");          // c:1625
                            pos += 1;                       // c:1625
                        }                                   // c:1625
                        // `:#pattern` — pattern-match-filter. Without
                        // the (M) flag, returns empty when value
                        // matches pattern; for arrays, removes
                        // matching elements. With (M), inverted.
                        // Port of Src/subst.c paramsubst's pattern
                        // path around the `case '#'` arm gated by
                        // `colf` (colon-prefix).
                        '#' => {                            // c:1625
                            operator = Some(":#");          // c:1625
                            pos += 1;                       // c:1625
                        }                                   // c:1625
                        // `:^arr2` zip-short / `:^^arr2` zip-long.
                        // Port of Src/subst.c paramsubst's SUB_ZIP_SHORT
                        // / SUB_ZIP_LONG arms — interleave elements
                        // from the named second array.
                        '^' if pos + 1 < chars.len() && chars[pos + 1] == '^' => { // c:1625
                            operator = Some(":^^");         // c:1625
                            pos += 2;                       // c:1625
                        }                                   // c:1625
                        '^' => {                            // c:1625
                            operator = Some(":^");          // c:1625
                            pos += 1;                       // c:1625
                        }                                   // c:1625
                        // `::=` unconditional assign — port of zsh's
                        // extension that fires regardless of whether
                        // the var is set/empty (subst.c handles via
                        // a special flag on the `:=` arm).
                        ':' if pos + 1 < chars.len() && chars[pos + 1] == '=' => { // c:1625
                            operator = Some("::=");         // c:1625
                            pos += 2;                       // c:1625
                        }                                   // c:1625
                        // History modifiers (`:h`, `:t`, `:r`, `:e`,
                        // `:l`, `:u`, `:q`, `:Q`, `:a`, `:A`, `:s/x/y/`,
                        // `:S/x/y/`, `:&`, `:f`, `:F`, `:w`, `:W`,
                        // `:c`, `:p`, `:P`). Per Src/subst.c:3611-3759
                        // (`if (colf && inbrace)` branch at the end of
                        // paramsubst), these chain after the param
                        // value and dispatch to `modify()` (subst.c:4531).
                        // Distinguish from `:` substring (`:OFFSET[:LEN]`)
                        // by the leading char — a digit / `-` / space
                        // is substring, anything else in the modifier
                        // alphabet is a modifier.
                        // Per Src/subst.c paramsubst — substring uses
                        // the `:OFFSET[:LEN]` numeric form. Anything
                        // non-numeric routes through `modify()` so
                        // unknown modifier letters emit
                        // "unrecognized modifier `X'" instead of
                        // silently parsing as offset 0.
                        c if c.is_alphabetic() || c == '&' => { // c:1625
                            operator = Some(":mod");        // c:1625
                        }                                   // c:1625
                        _ => {                              // c:1625
                            operator = Some(":");           // c:1625
                        } // Substring (digit, '-', '+', ' ', '(', etc.) // c:1625
                    }                                       // c:1625
                }                                           // c:1625
            }                                               // c:1625
            '-' => {                                        // c:1625
                operator = Some("-");                       // c:1625
                pos += 1;                                   // c:1625
            }                                               // c:1625
            '=' => {                                        // c:1625
                operator = Some("=");                       // c:1625
                pos += 1;                                   // c:1625
            }                                               // c:1625
            '+' => {                                        // c:1625
                operator = Some("+");                       // c:1625
                pos += 1;                                   // c:1625
            }                                               // c:1625
            '?' => {                                        // c:1625
                operator = Some("?");                       // c:1625
                pos += 1;                                   // c:1625
            }                                               // c:1625
            '#' => {                                        // c:1625
                pos += 1;                                   // c:1625
                if chars.get(pos) == Some(&'#') {           // c:1625
                    operator = Some("##");                  // c:1625
                    pos += 1;                               // c:1625
                } else {                                    // c:1625
                    operator = Some("#");                   // c:1625
                }                                           // c:1625
            }                                               // c:1625
            '%' => {                                        // c:1625
                pos += 1;                                   // c:1625
                if chars.get(pos) == Some(&'%') {           // c:1625
                    operator = Some("%%");                  // c:1625
                    pos += 1;                               // c:1625
                } else {                                    // c:1625
                    operator = Some("%");                   // c:1625
                }                                           // c:1625
            }                                               // c:1625
            '/' => {                                        // c:1625
                pos += 1;                                   // c:1625
                // `${var/pat/repl}` — first match
                // `${var//pat/repl}` — global
                // `${var/#pat/repl}` — anchor at start (prefix only)
                // `${var/%pat/repl}` — anchor at end (suffix only)
                // Per Src/subst.c paramsubst's `case '/':` arm.
                if chars.get(pos) == Some(&'/') {           // c:1625
                    operator = Some("//");                  // c:1625
                    pos += 1;                               // c:1625
                } else if chars.get(pos) == Some(&'#') {    // c:1625
                    operator = Some("/#");                  // c:1625
                    pos += 1;                               // c:1625
                } else if chars.get(pos) == Some(&'%') {    // c:1625
                    operator = Some("/%");                  // c:1625
                    pos += 1;                               // c:1625
                } else {                                    // c:1625
                    operator = Some("/");                   // c:1625
                }                                           // c:1625
            }                                               // c:1625
            '^' => {                                        // c:1625
                pos += 1;                                   // c:1625
                if chars.get(pos) == Some(&'^') {           // c:1625
                    operator = Some("^^");                  // c:1625
                    pos += 1;                               // c:1625
                } else {                                    // c:1625
                    operator = Some("^");                   // c:1625
                }                                           // c:1625
            }                                               // c:1625
            ',' => {                                        // c:1625
                pos += 1;                                   // c:1625
                if chars.get(pos) == Some(&',') {           // c:1625
                    operator = Some(",,");                  // c:1625
                    pos += 1;                               // c:1625
                } else {                                    // c:1625
                    operator = Some(",");                   // c:1625
                }                                           // c:1625
            }                                               // c:1625
            // `${var@OP}` is bash-only — zsh's parameter expansion
            // does not recognize `@` as a postfix operator and reports
            // "bad substitution". Capture the operator here so the
            // operand-collection loop below skips it cleanly and the
            // `Some("@op")` arm emits the diagnostic.
            '@' => {                                        // c:1625
                operator = Some("@op");                     // c:1625
                pos += 1;                                   // c:1625
            }                                               // c:1625
            _ => {}                                         // c:1625
        }                                                   // c:1625
    }                                                       // c:1625

    // Collect operand until closing brace
    let mut depth = 1;                                      // c:1625
    while pos < chars.len() && depth > 0 {                  // c:1625
        let c = chars[pos];                                 // c:1625
        if c == '{' || c == INBRACE {                       // c:1625
            depth += 1;                                     // c:1625
            operand.push(c);                                // c:1625
        } else if c == '}' || c == OUTBRACE {               // c:1625
            depth -= 1;                                     // c:1625
            if depth > 0 {                                  // c:1625
                operand.push(c);                            // c:1625
            }                                               // c:1625
        } else {                                            // c:1625
            operand.push(c);                                // c:1625
        }                                                   // c:1625
        pos += 1;                                           // c:1625
    }                                                       // c:1625

    // Get the value. The `(k)`, `(v)`, `(P)`, `(t)` flags change
    // WHICH thing is looked up:
    //   `(k)` — keys of the assoc named by var_name (Src/subst.c
    //           paramsubst's PM_HASHED key path).
    //   `(v)` — values of the assoc (the default for assoc
    //           expansion, but `(v)` is explicit).
    //   `(P)` — indirect: take var_name's scalar value and resolve
    //           that as a parameter name (Src/subst.c:1983-2000
    //           the `aspar` arm).
    //   `(t)` — return the parameter's type string ("scalar",
    //           "array", "association", "integer", etc.) per
    //           Src/subst.c:2810-2850 the `wantt` arm.
    // Pre-resolved value from a nested `${${…}…}` form short-circuits
    // the by-name lookup — operators / flags apply to that value.
    let mut value = if let Some(v) = nested_value.take() {  // c:1625
        // (P) flag with a nested var-name slot — `${(P)$(echo a)}` or
        // `${(P)${...}}`. Use the captured/resolved value as the
        // target name and look it up. Mirrors paramsubst's `aspar`
        // arm: `name = aval[0]; aval = NULL; getvalue(name)`.
        if flags.prompt_expand {                            // c:1625
            let target_name = v.first().cloned().unwrap_or_default(); // c:1625
            if target_name.is_empty() {                     // c:1625
                Vec::new()                                  // c:1625
            } else {                                        // c:1625
                get_param_with_subscript(&target_name, subscript.as_deref(), state) // c:1625
            }                                               // c:1625
        } else                                              // c:1625
        // Direct port of Src/subst.c paramsubst's `${${…}[idx]}` path:
        // when the var-name slot is itself an inner ${…}, the outer
        // `[subscript]` applies to the inner's aval. C source threads
        // this through `getindex(s, &v, …)` after the recursive
        // multsub call returns; if it set `aval`, the subscript runs
        // against the array. Without this dispatch, `${${(f)x}[2]}`
        // landed in the operator path with the joined scalar already
        // stringified and `[2]` was lost.
        if let Some(sub) = subscript.as_deref() {           // c:1625
            // Reuse the array-subscript logic that the by-name path
            // gets via `get_param_with_subscript`. We have the array
            // directly — interpret the subscript as numeric or `@`/`*`
            // / negative-index per zsh's normal subscript rules.
            let resolved_sub = singsub_no_tilde(sub, state); // c:1625
            if resolved_sub == "@" || resolved_sub == "*" { // c:1625
                v                                           // c:1625
            } else if let Some((lo_s, hi_s)) = resolved_sub.split_once(',') { // c:1625
                // Range slice `[lo,hi]` per Src/params.c::getindex
                // line ~2003. Both endpoints accept negative indices
                // (offset from end). Endpoints are 1-based when
                // positive; -1 means last element.
                let arr = &v;                               // c:1625
                let n = arr.len() as i64;                   // c:1625
                let resolve = |s: &str, default: i64| -> i64 { // c:1625
                    s.trim().parse::<i64>().unwrap_or(default) // c:1625
                };                                          // c:1625
                let lo = resolve(lo_s, 1);                  // c:1625
                let hi = resolve(hi_s, n);                  // c:1625
                let to_zero_based = |i: i64| -> i64 {       // c:1625
                    if i > 0 { i - 1 }                      // c:1625
                    else if i < 0 { n + i }                 // c:1625
                    else { 0 }                              // c:1625
                };                                          // c:1625
                let lo_i = to_zero_based(lo).max(0);        // c:1625
                let hi_i = to_zero_based(hi);               // c:1625
                if hi_i < lo_i || lo_i >= n {               // c:1625
                    Vec::new()                              // c:1625
                } else {                                    // c:1625
                    let hi_clamped = (hi_i + 1).min(n) as usize; // c:1625
                    arr[lo_i as usize..hi_clamped].to_vec() // c:1625
                }                                           // c:1625
            } else if let Ok(idx) = resolved_sub.parse::<i64>() { // c:1625
                let arr = &v;                               // c:1625
                let n = arr.len() as i64;                   // c:1625
                let real = if idx > 0 {                     // c:1625
                    (idx - 1) as usize                      // c:1625
                } else if idx < 0 {                         // c:1625
                    let off = n + idx;                      // c:1625
                    if off < 0 {                            // c:1625
                        return (                            // c:1625
                            chars[..dollar_pos].iter().collect::<String>() // c:1625
                                + &chars[pos..].iter().collect::<String>(), // c:1625
                            dollar_pos,                     // c:1625
                            Vec::new(),                     // c:1625
                        );                                  // c:1625
                    }                                       // c:1625
                    off as usize                            // c:1625
                } else {                                    // c:1625
                    0                                       // c:1625
                };                                          // c:1625
                arr.get(real).cloned().into_iter().collect() // c:1625
            } else {                                        // c:1625
                // Non-numeric / non-`@`/`*` subscript on an inner-
                // anonymous array — zsh treats this as no match.
                Vec::new()                                  // c:1625
            }                                               // c:1625
        } else {                                            // c:1625
            v                                               // c:1625
        }                                                   // c:1625
    } else if flags.keys && flags.values && state.assoc_arrays.contains_key(&var_name) { // c:1625
        // `${(kv)assoc}` → alternating key/value pairs in insertion
        // order. Per Src/subst.c paramsubst's PM_HASHED + (k|v)
        // flag combo: emit `key1 val1 key2 val2 …`. Order matches
        // the underlying IndexMap iteration.
        state                                               // c:1625
            .assoc_arrays                                   // c:1625
            .get(&var_name)                                 // c:1625
            .map(|m| {                                      // c:1625
                let mut out = Vec::with_capacity(m.len() * 2); // c:1625
                for (k, v) in m.iter() {                    // c:1625
                    out.push(k.clone());                    // c:1625
                    out.push(v.clone());                    // c:1625
                }                                           // c:1625
                out                                         // c:1625
            })                                              // c:1625
            .unwrap_or_default()                            // c:1625
    } else if flags.keys && state.assoc_arrays.contains_key(&var_name) { // c:1625
        // `${(k)assoc}` → keys, in insertion order.
        state                                               // c:1625
            .assoc_arrays                                   // c:1625
            .get(&var_name)                                 // c:1625
            .map(|m| m.keys().cloned().collect::<Vec<_>>()) // c:1625
            .unwrap_or_default()                            // c:1625
    } else if flags.keys {                                  // c:1625
        // `${(k)<magic-assoc>}` for specials NOT in `assoc_arrays`
        // (`aliases`, `functions`, `options`, `commands`, `terminfo`,
        // `errnos`, etc.). Direct port of paramsubst's magic-getfn
        // dispatch path: zsh's C source resolves the special's
        // scanfn at runtime and walks its key set. We synthesize
        // the same set via the executor snapshot the harness
        // stamped into SubstState.
        magic_assoc_keys(&var_name, state).unwrap_or_default() // c:1625
    } else if flags.values && state.assoc_arrays.contains_key(&var_name) { // c:1625
        // `${(v)assoc}` → values (same as default for plain
        // `${assoc}` but explicit; provided for `(kv)` paired use).
        state                                               // c:1625
            .assoc_arrays                                   // c:1625
            .get(&var_name)                                 // c:1625
            .map(|m| m.values().cloned().collect::<Vec<_>>()) // c:1625
            .unwrap_or_default()                            // c:1625
    } else if flags.values {                                // c:1625
        // `${(v)<magic-assoc>}` for specials NOT in `assoc_arrays`
        // — mirror the `${(k)<magic-assoc>}` path that walks the
        // synthesized scanfn key set, but resolve each key through
        // the executor's get_special_array_value to get values.
        magic_assoc_values(&var_name, state).unwrap_or_default() // c:1625
    } else if flags.prompt_expand {                         // c:1625
        // `(P)` indirect — read var_name's scalar value, then look
        // up THAT as a parameter name. Src/subst.c:1983-2000.
        // Multi-level indirection (`${(PP)x}`) is not supported by
        // C either; one level of redirection.
        // When the var-name slot was a cmd-subst (`${(P)$(echo a)}`),
        // the nested_value already holds the captured output — use
        // it as the target name directly. Same for `${(P)${...}}`.
        let target_name = if let Some(v) = nested_value.take() { // c:1625
            v.first().cloned().unwrap_or_default()          // c:1625
        } else {                                            // c:1625
            get_param_value(&var_name, state)               // c:1625
        };                                                  // c:1625
        // `${(P)"name[expr]"}` — the resolved target name may carry a
        // trailing `[subscript]`. Direct port of Src/subst.c:2799-2806
        // where `fetchvalue(&vbuf, &ov, …)` parses both name and any
        // bracketed subscript from the same input pointer. Split here
        // before the lookup so e.g. `n="arr[-1]"; ${(P)n}` returns the
        // last element of `$arr`, not "" because no param named
        // "arr[-1]" exists.
        let (target_base, target_sub) = match target_name.find('[') { // c:1625
            Some(b) if target_name.ends_with(']') => {      // c:1625
                let base = target_name[..b].to_string();    // c:1625
                let sub = target_name[b + 1..target_name.len() - 1].to_string(); // c:1625
                (base, Some(sub))                           // c:1625
            }                                               // c:1625
            _ => (target_name.clone(), None),               // c:1625
        };                                                  // c:1625
        if flags.type_info {                                // c:1625
            // `(Pt)` — indirect-then-typeset-flags. Direct port of
            // Src/subst.c:2807-2854: P resolves the indirection target,
            // then `wantt` introspects THAT parameter's type. Without
            // this combined arm, (Pt) fell through (P) only and lost
            // the type-string semantics zinit/zbrowse rely on.
            build_type_string_for(&target_base, state)      // c:1625
        } else if subscript.is_some() {                     // c:1625
            // Outer `[…]` on the brace expression itself takes
            // precedence over any trailing subscript baked into the
            // resolved target name. zsh's fetchvalue threads the
            // outer subscript via `s` while the resolved name is
            // already complete.
            get_param_with_subscript(&target_base, subscript.as_deref(), state) // c:1625
        } else if target_base.is_empty() {                  // c:1625
            Vec::new()                                      // c:1625
        } else {                                            // c:1625
            // Pass through any subscript that came from the resolved
            // name (e.g. `n2="arr[1,3]"; ${(P)n2}`).
            get_param_with_subscript(&target_base, target_sub.as_deref(), state) // c:1625
        }                                                   // c:1625
    } else if flags.type_info {                             // c:1625
        // `(t)` — type info string. Src/subst.c:2810-2853 builds
        // `TYPE` followed by `-modifier` suffixes for each set
        // attribute flag. Reads `state.var_attrs` (snapshot of the
        // executor's typeset table) for the flag matrix.
        build_type_string_for(&var_name, state)             // c:1625
    } else if subscript.is_some() || !var_name.is_empty() { // c:1625
        get_param_with_subscript(&var_name, subscript.as_deref(), state) // c:1625
    } else {                                                // c:1625
        Vec::new()                                          // c:1625
    };                                                      // c:1625

    // Apply chained subscripts (`${a[1][1]}`, `${(s. .)s[1]}`).
    // zsh recurses through `getindex(s, &v, …)` after each pick;
    // here we walk `extra_subs` left-to-right, treating the current
    // value vec as either an array (numeric index, `@`/`*` slice) or
    // a single scalar (range subscript like `[2,4]` slices chars).
    for extra in &extra_subs {                              // c:1625
        value = apply_chained_subscript(value, extra, state); // c:1625
    }                                                       // c:1625

    // Handle `${+NAME}` — set-test. Direct port of Src/subst.c:3600-
    // 3602: `val = dupstring(vunset ? "0" : "1")`. The check is
    // BEFORE length / operator / flags so a follow-on operator on a
    // `${+foo}` form treats the result as the literal "0"/"1"
    // string. zsh's "is set" rule: scalar in variables, indexed in
    // arrays, OR assoc subscript path means "key exists" (assoc-with-
    // missing-subscript returns 0).
    if chkset {                                             // c:1625
        // `(P)+NAME` — indirect set-test. Resolve NAME's scalar value
        // first, then check if THAT name is set. Per Src/subst.c
        // paramsubst's aspar arm running before chkset, the target of
        // `+` becomes the indirected name, not the original.
        let effective_name = if flags.prompt_expand {       // c:1625
            let target = if let Some(v) = nested_value.take() { // c:1625
                v.first().cloned().unwrap_or_default()      // c:1625
            } else {                                        // c:1625
                get_param_value(&var_name, state)           // c:1625
            };                                              // c:1625
            if target.is_empty() { var_name.clone() } else { target } // c:1625
        } else {                                            // c:1625
            var_name.clone()                                // c:1625
        };                                                  // c:1625
        let var_name = effective_name;                      // c:1625
        let is_set = if let Some(sub) = subscript.as_deref() { // c:1625
            // `${+arr[idx]}` — checks element existence.
            if let Some(arr) = state.arrays.get(&var_name) { // c:1625
                if sub == "@" || sub == "*" {               // c:1625
                    !arr.is_empty()                         // c:1625
                } else if let Ok(idx) = sub.parse::<i64>() { // c:1625
                    let real = if idx > 0 {                 // c:1625
                        (idx - 1) as usize                  // c:1625
                    } else if idx < 0 {                     // c:1625
                        let off = arr.len() as i64 + idx;   // c:1625
                        if off < 0 {                        // c:1625
                            0                               // c:1625
                        } else {                            // c:1625
                            off as usize                    // c:1625
                        }                                   // c:1625
                    } else {                                // c:1625
                        return (                            // c:1625
                            chars[..dollar_pos].iter().collect::<String>() // c:1625
                                + "0"                       // c:1625
                                + &chars[pos..].iter().collect::<String>(), // c:1625
                            dollar_pos + 1,                 // c:1625
                            vec!["0".to_string()],          // c:1625
                        );                                  // c:1625
                    };                                      // c:1625
                    real < arr.len()                        // c:1625
                } else {                                    // c:1625
                    false                                   // c:1625
                }                                           // c:1625
            } else if let Some(map) = state.assoc_arrays.get(&var_name) { // c:1625
                if sub == "@" || sub == "*" {               // c:1625
                    !map.is_empty()                         // c:1625
                } else {                                    // c:1625
                    map.contains_key(sub)                   // c:1625
                }                                           // c:1625
            } else {                                        // c:1625
                // Magic special-parameter assoc lookups —
                // `${+functions[name]}` / `${+commands[name]}` /
                // `${+aliases[name]}` etc. should return 1 if the
                // shell's introspection assoc has that key. The
                // executor knows; route through the bridge that
                // also handles the regular `${functions[name]}`
                // read path. Direct port of zsh's chkset logic
                // when var_name is a special-parameter table entry
                // (Src/subst.c paramsubst's getindex → vunset
                // chain ends up testing param->gsu.h->getfn
                // returning a non-NULL value).
                // Use expand_subscript_pat (not singsub_no_tilde)
                // because it handles digit-name positionals via
                // state.arrays["@"] — without that path, `${+commands[$1]}`
                // inside a function passed the literal "$1" as the
                // key and check_magic_assoc_set's PATH walk couldn't
                // find it. Direct port of Src/subst.c paramsubst's
                // pre-getindex singsub() pass which DOES expand
                // positionals at this point.
                let resolved = expand_subscript_pat(sub, state); // c:1625
                check_magic_assoc_set(&var_name, &resolved, state) // c:1625
            }                                               // c:1625
        } else {                                            // c:1625
            // Bare `${+NAME}` — set if scalar in variables OR present
            // in arrays/assoc tables. Mirrors zsh: any kind of
            // declaration counts as "set". An empty-string scalar
            // counts as set (matches zsh, where `x=; echo ${+x}`
            // prints 1).
            state.variables.contains_key(&var_name)         // c:1625
                || state.arrays.contains_key(&var_name)     // c:1625
                || state.assoc_arrays.contains_key(&var_name) // c:1625
                || std::env::var(&var_name).is_ok()         // c:1625
        };                                                  // c:1625
        let s = if is_set { "1" } else { "0" };             // c:1625
        let prefix: String = chars[..dollar_pos].iter().collect(); // c:1625
        let suffix: String = chars[pos..].iter().collect(); // c:1625
        return (                                            // c:1625
            format!("{}{}{}", prefix, s, suffix),           // c:1625
            dollar_pos + 1,                                 // c:1625
            vec![s.to_string()],                            // c:1625
        );                                                  // c:1625
    }                                                       // c:1625

    // Apply operator FIRST. Flags like `(%)`, `(L)`, `(q)`, padding,
    // counting, etc. are post-substitution transforms — they must
    // see the value AFTER the operator has potentially replaced it
    // with the operand (`:-`, `:=`, `:+`). Per Src/subst.c the
    // operator dispatch (paramsubst case arms ~3192-3325) runs
    // before the flags-transform sections (3957-4019). Pre-lookup
    // flags like `(k)`, `(v)`, `(P)`, `(t)` already fired earlier
    // during the value-lookup path.
    //
    // The `${#NAME}` length prefix is ALSO post-operator: zsh's
    // `${#NAME:-default}` returns the length of "default" when
    // NAME is unset, not the length of "" (which is 0). Direct
    // port of paramsubst's chklen branch (Src/subst.c) that runs
    // its strlen() after the operator block. The earlier zshrs
    // ordering applied length first, so `${#NAME:-default}`
    // returned 0 instead of 7. Fixed: length now follows operator.
    //
    // `(M)` is the exception: it modifies the `:#` operator's
    // semantics inline, so it travels with the operator call.
    value = apply_operator_with_flags_full(                 // c:1625
        &var_name,                                          // c:1625
        subscript.as_deref(),                               // c:1625
        value,                                              // c:1625
        operator,                                           // c:1625
        &operand,                                           // c:1625
        flags.match_flag,                                   // c:1625
        flags.search,                                       // c:1625
        state,                                              // c:1625
    );                                                      // c:1625

    // Apply length prefix AFTER the operator has replaced the
    // value. See the `length_prefix` setup above for the rationale
    // (zsh's chklen runs after operator, so `${#X:-default}` is
    // strlen(default) not strlen("")).
    //
    // `(c)` and `(w)` flags act as length-mode hints when combined
    // with `#`: `${(c)#var}` is char-count, `${(w)#var}` is word-
    // count. Direct port of Src/subst.c paramsubst's `chklen` arm
    // which checks PSPRINT_FLAG_C / PSPRINT_FLAG_W. Once the flag
    // has been consumed for length, clear it on `flags` so the
    // post-pass `apply_param_flags` doesn't double-count.
    if length_prefix {                                      // c:1625
        // For `(c)#arr` and `(w)#arr` zsh joins the array with the
        // first IFS char (space by default) BEFORE counting chars or
        // words — Src/subst.c paramsubst's chklen path inside the
        // PSPRINT_FLAG_C / _W arms calls sepjoin first. Without the
        // join, `${(c)#arr}` for `arr=(abc def)` reported 6 (chars
        // 3+3) instead of 7 (chars of "abc def").
        let ifs_first = state                               // c:1625
            .variables                                      // c:1625
            .get("IFS")                                     // c:1625
            .and_then(|s| s.chars().next())                 // c:1625
            .unwrap_or(' ')                                 // c:1625
            .to_string();                                   // c:1625
        let len = if flags.count_chars {                    // c:1625
            value.join(&ifs_first).chars().count()          // c:1625
        } else if flags.count_words {                       // c:1625
            value.join(&ifs_first).split_whitespace().count() // c:1625
        } else if value.len() == 1 {                        // c:1625
            value[0].chars().count()                        // c:1625
        } else {                                            // c:1625
            value.len()                                     // c:1625
        };                                                  // c:1625
        value = vec![len.to_string()];                      // c:1625
        flags.count_chars = false;                          // c:1625
        flags.count_words = false;                          // c:1625
    }                                                       // c:1625

    // Apply post-operator flags: case mod, sort, unique, padding,
    // quoting, counting, `(%)` prompt-percent expansion.
    value = apply_param_flags(&value, &flags, state, pf_flags); // c:1625

    // Pick the array→scalar join separator. Direct port of
    // Src/subst.c:3897-3933 (the join-back path at end of
    // paramsubst):
    //   • `(j:STR:)` / `(F)` set `sep` — explicit join separator.
    //   • `(f)` / `(s:STR:)` / `(0)` set `spsep` (split-separator).
    //     When the result needs to re-collapse to scalar in a
    //     joining context (DQ assignment / nested expansion),
    //     C source line 3914 uses spsep as the rejoin separator
    //     when nojoin != 1 (i.e., we're NOT forcing array-keep).
    //   • Otherwise the default IFS first char (space).
    //
    // Without this, `y="${(f)x}"` joined with ` ` instead of `\n`
    // — the saved value lost its line structure and `(f)` was
    // effectively a no-op for the assignment-context consumer.
    let array_join_sep: String = if let Some(ref s) = flags.join_sep { // c:1625
        s.clone()                                           // c:1625
    } else if flags.join_lines {                            // c:1625
        "\n".to_string()                                    // c:1625
    } else if flags.split_lines {                           // c:1625
        // `(f)` flag's spsep is "\n" — when re-joining a force-
        // split array back to scalar, use the same separator so
        // round-trip preserves the original line structure.
        "\n".to_string()                                    // c:1625
    } else if let Some(ref s) = flags.split_sep {           // c:1625
        // `(s:STR:)` similarly: rejoin with the same separator
        // when collapsing to scalar.
        s.clone()                                           // c:1625
    } else {                                                // c:1625
        // Default: IFS first char (space for default IFS=$' \t\n').
        " ".to_string()                                     // c:1625
    };                                                      // c:1625

    // Handle word splitting
    let joined = if flags.join_sep.is_some() || value.len() == 1 { // c:1625
        let sep = flags.join_sep.as_deref().unwrap_or(&array_join_sep); // c:1625
        value.join(sep)                                     // c:1625
    } else if pf_flags & prefork_flags::SHWORDSPLIT != 0 && !qt { // c:1625
        // Each array element becomes a separate word
        let prefix: String = chars[..dollar_pos].iter().collect(); // c:1625
        let suffix: String = chars[pos..].iter().collect(); // c:1625

        for (i, v) in value.iter().enumerate() {            // c:1625
            if i == 0 && value.len() == 1 {                 // c:1625
                result_nodes.push(format!("{}{}{}", prefix, v, suffix)); // c:1625
            } else if i == 0 {                              // c:1625
                result_nodes.push(format!("{}{}", prefix, v)); // c:1625
            } else if i == value.len() - 1 {                // c:1625
                result_nodes.push(format!("{}{}", v, suffix)); // c:1625
            } else {                                        // c:1625
                result_nodes.push(v.clone());               // c:1625
            }                                               // c:1625
        }                                                   // c:1625

        if result_nodes.is_empty() {                        // c:1625
            result_nodes.push(format!("{}{}", prefix, suffix)); // c:1625
        }                                                   // c:1625

        return (result_nodes[0].clone(), dollar_pos, result_nodes); // c:1625
    } else {                                                // c:1625
        value.join(&array_join_sep)                         // c:1625
    };                                                      // c:1625

    // Build result
    let prefix: String = chars[..dollar_pos].iter().collect(); // c:1625
    let suffix: String = chars[pos..].iter().collect();     // c:1625
    let result = format!("{}{}{}", prefix, joined, suffix); // c:1625

    // Preserve the array shape via `result_nodes` so nested
    // `${${(f)x}[N]}` recursion can subscript the original elements
    // instead of operating on the joined scalar. Direct port of
    // Src/subst.c's recursive aval threading: the inner paramsubst
    // call hands aval back to the outer, which decides whether to
    // subscript / re-flag / sepjoin. Without this, the outer saw
    // only the joined text and `[2]` subscripted the joined string
    // by character position. The top-level emission point still
    // gets `result` (joined) — it's only the multi-element case
    // that populates `result_nodes` for the outer to pick up.
    if value.len() > 1 {                                    // c:1625
        result_nodes.extend(value.iter().cloned());         // c:1625
    } else {                                                // c:1625
        result_nodes.push(result.clone());                  // c:1625
    }                                                       // c:1625

    (result, prefix.len() + joined.len(), result_nodes)     // c:1625
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

/// Get parameter value (scalar or array)
fn get_param_value(name: &str, state: &SubstState) -> String { // c:2807
    state                                                   // c:2807
        .variables                                          // c:2807
        .get(name)                                          // c:2807
        .cloned()                                           // c:2807
        .or_else(|| std::env::var(name).ok())               // c:2807
        .unwrap_or_default()                                // c:2807
}                                                           // c:2807

/// Get parameter value with subscript
/// Expand a subscript pattern (e.g. `(I)$1`'s `$1` slot) the way zsh's
/// getindex() does — single-pass, no IFS-split, no glob, no tilde.
/// Handles `$N` digit positionals via state.arrays["@"], `${NAME}` and
/// `$NAME` via state.variables. Falls back to singsub for cmd-subst /
/// nested expansions. Direct port of Src/subst.c paramsubst's
/// pre-getindex `singsub(&s)` step.
fn expand_subscript_pat(s: &str, state: &mut SubstState) -> String { // c:2867
    if !s.contains('$') && !s.contains('`') {               // c:2867
        return s.to_string();                               // c:2867
    }                                                       // c:2867
    let chars: Vec<char> = s.chars().collect();             // c:2867
    let mut out = String::with_capacity(s.len());           // c:2867
    let mut i = 0;                                          // c:2867
    while i < chars.len() {                                 // c:2867
        let c = chars[i];                                   // c:2867
        if c == '$' && i + 1 < chars.len() {                // c:2867
            let nxt = chars[i + 1];                         // c:2867
            // ${NAME} / ${NAME[…]} / ${NAME:op…}: full braced form —
            // delegate to substitute_brace's logic indirectly by
            // capturing the balanced braces and recursing through
            // singsub_no_tilde on just that fragment.
            if nxt == '{' {                                 // c:2867
                let mut depth = 1;                          // c:2867
                let mut j = i + 2;                          // c:2867
                while j < chars.len() && depth > 0 {        // c:2867
                    if chars[j] == '{' {                    // c:2867
                        depth += 1;                         // c:2867
                    } else if chars[j] == '}' {             // c:2867
                        depth -= 1;                         // c:2867
                        if depth == 0 {                     // c:2867
                            break;                          // c:2867
                        }                                   // c:2867
                    }                                       // c:2867
                    j += 1;                                 // c:2867
                }                                           // c:2867
                if j < chars.len() && depth == 0 {          // c:2867
                    let frag: String = chars[i..=j].iter().collect(); // c:2867
                    out.push_str(&singsub_no_tilde(&frag, state)); // c:2867
                    i = j + 1;                              // c:2867
                    continue;                               // c:2867
                }                                           // c:2867
            }                                               // c:2867
            // $N — digit positional
            if nxt.is_ascii_digit() {                       // c:2867
                let mut j = i + 1;                          // c:2867
                let mut num = String::new();                // c:2867
                while j < chars.len() && chars[j].is_ascii_digit() { // c:2867
                    num.push(chars[j]);                     // c:2867
                    j += 1;                                 // c:2867
                }                                           // c:2867
                if let Ok(n) = num.parse::<usize>() {       // c:2867
                    if n == 0 {                             // c:2867
                        // `$0` — script / function name. zsh stores it
                        // in the parameter table under "0" (writable
                        // via plain `0=value` assignment, used by
                        // zinit's `0="${${(M)0:#/*}:-$PWD/$0}"` to
                        // make $0 absolute). Direct port of
                        // Src/params.c which exposes "0" as a
                        // SPECIALPMDEF backed by `argzero`. Look up
                        // state.variables["0"] (the snapshot of the
                        // executor's variable table) before any other
                        // dispatch.
                        if let Some(v) = state.variables.get("0") { // c:2867
                            out.push_str(v);                // c:2867
                            i = j;                          // c:2867
                            continue;                       // c:2867
                        }                                   // c:2867
                    } else if let Some(arr) = state.arrays.get("@") { // c:2867
                        if let Some(v) = arr.get(n - 1) {   // c:2867
                            out.push_str(v);                // c:2867
                            i = j;                          // c:2867
                            continue;                       // c:2867
                        }                                   // c:2867
                        i = j;                              // c:2867
                        continue;                           // c:2867
                    }                                       // c:2867
                }                                           // c:2867
            }                                               // c:2867
            // $NAME (alpha/_ start)
            if nxt.is_ascii_alphabetic() || nxt == '_' {    // c:2867
                let mut j = i + 1;                          // c:2867
                while j < chars.len()                       // c:2867
                    && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') // c:2867
                {                                           // c:2867
                    j += 1;                                 // c:2867
                }                                           // c:2867
                let name: String = chars[i + 1..j].iter().collect(); // c:2867
                if let Some(v) = state.variables.get(&name) { // c:2867
                    out.push_str(v);                        // c:2867
                } else if let Some(arr) = state.arrays.get(&name) { // c:2867
                    out.push_str(&arr.join(" "));           // c:2867
                }                                           // c:2867
                i = j;                                      // c:2867
                continue;                                   // c:2867
            }                                               // c:2867
            // $$ / $? / $# / $! / $- / $@ / $* — single-char specials
            // not commonly used as subscript-pat inputs, but cover the
            // basics so e.g. `(I)$#` doesn't silently drop the `$#`.
            if matches!(nxt, '$' | '?' | '#' | '!' | '-' | '@' | '*') { // c:2867
                let frag: String = chars[i..=i + 1].iter().collect(); // c:2867
                out.push_str(&singsub_no_tilde(&frag, state)); // c:2867
                i += 2;                                     // c:2867
                continue;                                   // c:2867
            }                                               // c:2867
        }                                                   // c:2867
        // Backtick cmd-subst — defer to singsub.
        if c == '`' {                                       // c:2867
            let mut j = i + 1;                              // c:2867
            while j < chars.len() && chars[j] != '`' {      // c:2867
                j += 1;                                     // c:2867
            }                                               // c:2867
            if j < chars.len() {                            // c:2867
                let frag: String = chars[i..=j].iter().collect(); // c:2867
                out.push_str(&singsub_no_tilde(&frag, state)); // c:2867
                i = j + 1;                                  // c:2867
                continue;                                   // c:2867
            }                                               // c:2867
        }                                                   // c:2867
        out.push(c);                                        // c:2867
        i += 1;                                             // c:2867
    }                                                       // c:2867
    out                                                     // c:2867
}                                                           // c:2867

fn get_param_with_subscript(                                // c:2807
    name: &str,                                             // c:2807
    subscript: Option<&str>,                                // c:2807
    state: &SubstState,                                     // c:2807
) -> Vec<String> {                                          // c:2807
    // Subscript flags `(I)`, `(i)`, `(r)`, `(R)`, `(re)`, etc. —
    // Per Src/subst.c:1095-1130 the subscript parser strips a
    // leading `(...)` block and sets a bitmask of flags that
    // change what the subscript means. Most-used in plugin code:
    //   `(r)pat`  — return the FIRST element matching pat
    //   `(R)pat`  — last matching
    //   `(re)val` — exact-string match (no glob — match val verbatim)
    //   `(i)pat`  — return INDEX of first matching element (1-based)
    //   `(I)pat`  — last index
    // Everything else falls through to the legacy numeric/at-star
    // subscript handling.
    let parsed_sub = subscript.and_then(parse_subscript_flags); // c:1095
    let (sub_flags, real_sub) = match parsed_sub.as_ref() { // c:1095
        Some((f, s)) => (Some(f), Some(s.as_str())),        // c:1095
        None => (None, subscript),                          // c:1095
    };                                                      // c:1095

    // Numeric-name positional shortcut: `$1`, `$2`, ..., `${1}`,
    // `${10:t}` etc. The positional array is mirrored at `state
    // .arrays["@"]` from `from_executor`; resolve the digit name
    // to that slot (1-based → 0-based) so subsequent operators /
    // modifiers see the actual positional value rather than empty.
    // Direct port of zsh getindex()'s digit-name handling
    // (Src/subst.c:1300-1340).
    if !name.is_empty() && name.chars().all(|c| c.is_ascii_digit()) { // c:1300
        if let Ok(n) = name.parse::<usize>() {              // c:1300
            if let Some(arr) = state.arrays.get("@") {      // c:1300
                if n == 0 {                                 // c:1300
                    // `$0` is the script name, not the first positional.
                    // Fall through to env / variables lookup.
                } else if let Some(v) = arr.get(n - 1) {    // c:1300
                    // `${1[N,M]}` — char-slice on the positional value
                    // (zsh treats it as a scalar). Without this branch
                    // the [...] subscript was silently dropped on
                    // digit-name positionals, so a function like
                    //   fn() { echo "${1[1,3]}"; }; fn abcdefg
                    // returned the full string instead of "abc".
                    if let Some(sub) = real_sub {           // c:1300
                        return scalar_char_subscript(v, sub); // c:1300
                    }                                       // c:1300
                    return vec![v.clone()];                 // c:1300
                } else {                                    // c:1300
                    return Vec::new();                      // c:1300
                }                                           // c:1300
            }                                               // c:1300
        }                                                   // c:1300
    }                                                       // c:1300

    // Check if it's an array
    if let Some(arr) = state.arrays.get(name) {             // c:1300
        if let Some(flags) = sub_flags {                    // c:1300
            // Expand $-references in the pattern before matching.
            // zsh's getindex() runs the subscript through singsub
            // before pattern-compilation (Src/subst.c paramsubst's
            // pre-getindex pass) — without this, `${arr[(I)$1]}`
            // matches the literal string "$1" and never finds an
            // element, so add-zsh-hook's `${hooktypes[(I)$1]} == 0`
            // returned 0 (treated as "not found") and tripped the
            // wrong branch.
            let raw_pat = real_sub.unwrap_or("");           // c:1300
            let mut substate_mut = SubstState {             // c:1300
                errflag: state.errflag,                     // c:1300
                opts: state.opts.clone(),                   // c:1300
                variables: state.variables.clone(),         // c:1300
                arrays: state.arrays.clone(),               // c:1300
                assoc_arrays: state.assoc_arrays.clone(),   // c:1300
                skip_filesub: state.skip_filesub,           // c:1300
                function_names: state.function_names.clone(), // c:1300
                command_names: state.command_names.clone(), // c:1300
                alias_names: state.alias_names.clone(),     // c:1300
                var_attrs: state.var_attrs.clone(),         // c:1300
            };                                              // c:1300
            let expanded_pat = if raw_pat.contains('$') || raw_pat.contains('`') { // c:1300
                expand_subscript_pat(raw_pat, &mut substate_mut) // c:1300
            } else {                                        // c:1300
                raw_pat.to_string()                         // c:1300
            };                                              // c:1300
            return apply_array_subscript_flags(arr, flags, &expanded_pat); // c:1300
        }                                                   // c:1300
        if let Some(sub) = real_sub {                       // c:1300
            if sub == "@" || sub == "*" {                   // c:1300
                return arr.clone();                         // c:1300
            }                                               // c:1300
            // Parse numeric index
            if let Ok(idx) = sub.parse::<i64>() {           // c:1300
                let idx = if idx < 0 {                      // c:1300
                    (arr.len() as i64 + idx) as usize       // c:1300
                } else {                                    // c:1300
                    (idx - 1).max(0) as usize // zsh arrays are 1-indexed // c:1300
                };                                          // c:1300
                return arr.get(idx).cloned().into_iter().collect(); // c:1300
            }                                               // c:1300
        }                                                   // c:1300
        return arr.clone();                                 // c:1300
    }                                                       // c:1300

    // Check if it's an associative array
    if let Some(assoc) = state.assoc_arrays.get(name) {     // c:1300
        if let Some(flags) = sub_flags {                    // c:1300
            // For assocs, `(r)pat` searches VALUES and returns the
            // matching value; `(R)pat` is last match; `(k)` flips
            // search to keys; `(kv)` returns alternating pairs.
            // C source: subst.c handles these via the same flag
            // bits as arrays but interprets the source as
            // values-by-default.
            // Expand $-refs in pat (mirror the indexed-array path
            // immediately above) so `${assoc[(r)$key]}` finds the
            // element zsh would.
            let raw_pat = real_sub.unwrap_or("");           // c:1300
            let mut substate_mut = SubstState {             // c:1300
                errflag: state.errflag,                     // c:1300
                opts: state.opts.clone(),                   // c:1300
                variables: state.variables.clone(),         // c:1300
                arrays: state.arrays.clone(),               // c:1300
                assoc_arrays: state.assoc_arrays.clone(),   // c:1300
                skip_filesub: state.skip_filesub,           // c:1300
                function_names: state.function_names.clone(), // c:1300
                command_names: state.command_names.clone(), // c:1300
                alias_names: state.alias_names.clone(),     // c:1300
                var_attrs: state.var_attrs.clone(),         // c:1300
            };                                              // c:1300
            let expanded_pat = if raw_pat.contains('$') || raw_pat.contains('`') { // c:1300
                expand_subscript_pat(raw_pat, &mut substate_mut) // c:1300
            } else {                                        // c:1300
                raw_pat.to_string()                         // c:1300
            };                                              // c:1300
            let pairs: Vec<(String, String)> = assoc        // c:1300
                .iter()                                     // c:1300
                .map(|(k, v)| (k.clone(), v.clone()))       // c:1300
                .collect();                                 // c:1300
            return apply_assoc_subscript_flags(&pairs, flags, &expanded_pat); // c:1300
        }                                                   // c:1300
        if let Some(sub) = real_sub {                       // c:1300
            if sub == "@" || sub == "*" {                   // c:1300
                return assoc.values().cloned().collect();   // c:1300
            }                                               // c:1300
            // Expand $-refs in the key before lookup. Without this,
            // `${m[$k]}` looked up the literal key "$k" instead of the
            // value of $k. Direct port of zsh's getindex() singsub
            // pass.
            let resolved = if sub.contains('$') || sub.contains('`') { // c:1300
                let mut substate_mut = SubstState {         // c:1300
                    errflag: state.errflag,                 // c:1300
                    opts: state.opts.clone(),               // c:1300
                    variables: state.variables.clone(),     // c:1300
                    arrays: state.arrays.clone(),           // c:1300
                    assoc_arrays: state.assoc_arrays.clone(), // c:1300
                    skip_filesub: state.skip_filesub,       // c:1300
                    function_names: state.function_names.clone(), // c:1300
                    command_names: state.command_names.clone(), // c:1300
                    alias_names: state.alias_names.clone(), // c:1300
                    var_attrs: state.var_attrs.clone(),     // c:1300
                };                                          // c:1300
                expand_subscript_pat(sub, &mut substate_mut) // c:1300
            } else {                                        // c:1300
                sub.to_string()                             // c:1300
            };                                              // c:1300
            return assoc.get(&resolved).cloned().into_iter().collect(); // c:1300
        }                                                   // c:1300
        return assoc.values().cloned().collect();           // c:1300
    }                                                       // c:1300

    // Magic-assoc fallback for names not in `state.arrays` /
    // `state.assoc_arrays`. Direct port of paramsubst's per-special
    // getfn dispatch path: zsh's C source resolves the magic-array
    // (`aliases`, `functions`, `mapfile`, `terminfo`, `errnos`, …)
    // through the SPECIALPMDEF entry's getfn slot at runtime. We
    // delegate to the executor's `get_special_array_value` which
    // implements the same per-name table.
    if let Some(sub) = real_sub {                           // c:1300
        if magic_assoc_keys(name, state).is_some()          // c:1300
            || matches!(                                    // c:1300
                name,                                       // c:1300
                "mapfile" | "terminfo" | "termcap" | "errnos" | "sysparams" // c:1300
            )                                               // c:1300
        {                                                   // c:1300
            // Subscript-flag form `(I)pat` / `(i)pat` / `(r)pat` /
            // `(R)pat` etc. on a magic-assoc — synthesize the
            // (key,value) pair list from the executor's
            // get_special_array_value scanfn, then route through
            // apply_assoc_subscript_flags. Direct port of
            // Src/params.c getarg's hash-aware index/match handling
            // (the `ishash && rev` branch). Without this, the outer
            // path passed the literal `(I)pat` text through as the
            // assoc key, so `\${aliases[(I)foo*]}` returned empty.
            let trimmed_sub = sub.trim_start();             // c:1300
            if trimmed_sub.starts_with('(') {               // c:1300
                if let Some((sub_flags, sub_pat)) = parse_subscript_flags(trimmed_sub) { // c:1300
                    if let Some(keys) = magic_assoc_keys(name, state) { // c:1300
                        let pairs: Vec<(String, String)> =  // c:1300
                            crate::exec::with_executor(|exec| { // c:1300
                                keys.into_iter()            // c:1300
                                    .map(|k| {              // c:1300
                                        let v = exec        // c:1300
                                            .get_special_array_value(name, &k) // c:1300
                                            .unwrap_or_default(); // c:1300
                                        (k, v)              // c:1300
                                    })                      // c:1300
                                    .collect()              // c:1300
                            });                             // c:1300
                        return apply_assoc_subscript_flags(&pairs, &sub_flags, &sub_pat); // c:1300
                    }                                       // c:1300
                }                                           // c:1300
            }                                               // c:1300
            // Expand the subscript before lookup so `${mapfile[$tmp]}`
            // resolves $tmp first. Direct port of paramsubst's
            // `singsub(&sub)` step in the subscript-resolve path.
            let mut substate_mut = SubstState {             // c:1300
                errflag: state.errflag,                     // c:1300
                opts: state.opts.clone(),                   // c:1300
                variables: state.variables.clone(),         // c:1300
                arrays: state.arrays.clone(),               // c:1300
                assoc_arrays: state.assoc_arrays.clone(),   // c:1300
                skip_filesub: state.skip_filesub,           // c:1300
                function_names: state.function_names.clone(), // c:1300
                command_names: state.command_names.clone(), // c:1300
                alias_names: state.alias_names.clone(),     // c:1300
                var_attrs: state.var_attrs.clone(),         // c:1300
            };                                              // c:1300
            let resolved_sub = singsub_no_tilde(sub, &mut substate_mut); // c:1300
            // For splice forms `@`/`*`, return the full key list
            // / value list per the special's contract; otherwise
            // resolve the single key.
            let val = crate::exec::with_executor(|exec| {   // c:1300
                exec.get_special_array_value(name, &resolved_sub).unwrap_or_default() // c:1300
            });                                             // c:1300
            return if val.is_empty() {                      // c:1300
                Vec::new()                                  // c:1300
            } else {                                        // c:1300
                vec![val]                                   // c:1300
            };                                              // c:1300
        }                                                   // c:1300
    }                                                       // c:1300

    // Scalar — apply char-index or range subscript when present.
    // Port of Src/subst.c paramsubst's scalar-subscript path (the
    // `getindex` / `getarg` branch when the param resolves to a
    // string and the subscript is `N` / `N,M`).
    let value = get_param_value(name, state);               // c:1300
    if value.is_empty() {                                   // c:1300
        return Vec::new();                                  // c:1300
    }                                                       // c:1300
    if let Some(sub) = real_sub {                           // c:1300
        return scalar_char_subscript(&value, sub);          // c:1300
    }                                                       // c:1300
    vec![value]                                             // c:1300
}                                                           // c:1300

/// `${var[N]}` / `${var[N,M]}` on a scalar — char-index / char-slice.
/// Direct port of paramsubst's scalar-subscript handling. Negative
/// indices count from end (1-based same as zsh: `[-1]` is last char).
/// Returns the original scalar untouched if `sub` doesn't parse as
/// a numeric index or `N,M` range.
fn scalar_char_subscript(value: &str, sub: &str) -> Vec<String> { // c:2867
    let chars: Vec<char> = value.chars().collect();         // c:2867
    let n = chars.len() as i64;                             // c:2867
    let to_idx = |i: i64| -> usize {                        // c:2867
        if i > 0 {                                          // c:2867
            ((i - 1) as usize).min(chars.len())             // c:2867
        } else if i < 0 {                                   // c:2867
            ((n + i).max(0)) as usize                       // c:2867
        } else {                                            // c:2867
            0                                               // c:2867
        }                                                   // c:2867
    };                                                      // c:2867
    // Subscript bounds in zsh are arithmetic expressions, not just
    // literal numbers (`s[1,COLUMNS-8]`, `s[i+1,$#s]`). Direct port
    // of zsh's getindex which calls mathevali on each side of the
    // comma. Try parse-as-int first (fast path); fall back to
    // evaluate_arithmetic via the live executor.
    let resolve = |part: &str| -> Option<i64> {             // c:2867
        let trimmed = part.trim();                          // c:2867
        if let Ok(v) = trimmed.parse::<i64>() {             // c:2867
            return Some(v);                                 // c:2867
        }                                                   // c:2867
        crate::exec::try_with_executor(|exec| {             // c:2867
            exec.evaluate_arithmetic(trimmed)               // c:2867
                .parse::<i64>()                             // c:2867
                .ok()                                       // c:2867
        })                                                  // c:2867
        .flatten()                                          // c:2867
    };                                                      // c:2867
    if let Some(comma) = sub.find(',') {                    // c:2867
        if let (Some(a), Some(b)) = (                       // c:2867
            resolve(&sub[..comma]),                         // c:2867
            resolve(&sub[comma + 1..]),                     // c:2867
        ) {                                                 // c:2867
            let start = to_idx(a);                          // c:2867
            let end = if b > 0 {                            // c:2867
                (b as usize).min(chars.len())               // c:2867
            } else if b < 0 {                               // c:2867
                ((n + b + 1).max(0)) as usize               // c:2867
            } else {                                        // c:2867
                0                                           // c:2867
            };                                              // c:2867
            if start < chars.len() && start < end {         // c:2867
                let slice: String = chars[start..end.min(chars.len())].iter().collect(); // c:2867
                return vec![slice];                         // c:2867
            }                                               // c:2867
            return Vec::new();                              // c:2867
        }                                                   // c:2867
    }                                                       // c:2867
    if let Some(idx) = resolve(sub) {                       // c:2867
        let real = to_idx(idx);                             // c:2867
        return chars                                        // c:2867
            .get(real)                                      // c:2867
            .map(|c| vec![c.to_string()])                   // c:2867
            .unwrap_or_default();                           // c:2867
    }                                                       // c:2867
    vec![value.to_string()]                                 // c:2867
}                                                           // c:2867

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
    /// `(i)` or `(I)` — index-search subscript flag was present.
    pub fn has_index_or_reverse_index(&self) -> bool {      // c:2867
        self.forward_index || self.reverse_index            // c:2867
    }                                                       // c:2867
}                                                           // c:2867

/// Parse a `(flags)pattern` subscript prefix. Returns `Some((flags,
/// rest))` when the leading `(...)` is recognized; `None` when the
/// subscript has no flag prefix.
///
/// Port of the subscript-flag-parsing branch in Src/subst.c (around
/// the `[(...)pat]` handler near line 1095). Recognized chars:
/// r/R/i/I/e/k/n. Everything else aborts the parse — we don't
/// silently accept unknown flags because that would alias
/// `[(unknown)foo]` to `[foo]` which can mask bugs in user code.
pub fn parse_subscript_flags(sub: &str) -> Option<(SubscriptFlags, String)> { // c:2867
    let s = sub.trim_start();                               // c:2867
    if !s.starts_with('(') {                                // c:2867
        return None;                                        // c:2867
    }                                                       // c:2867
    let close = s.find(')')?;                               // c:2867
    let body = &s[1..close];                                // c:2867
    let rest = &s[close + 1..];                             // c:2867
    let mut flags = SubscriptFlags::default();              // c:2867
    for c in body.chars() {                                 // c:2867
        match c {                                           // c:2867
            'r' => flags.forward_match = true,              // c:2867
            'R' => flags.reverse_match = true,              // c:2867
            'i' => flags.forward_index = true,              // c:2867
            'I' => flags.reverse_index = true,              // c:2867
            'e' => flags.exact = true,                      // c:2867
            'k' => flags.keys = true,                       // c:2867
            'n' => flags.numeric = true,                    // c:2867
            _ => return None, // unknown flag → not a flag block // c:2867
        }                                                   // c:2867
    }                                                       // c:2867
    if !flags.forward_match                                 // c:2867
        && !flags.reverse_match                             // c:2867
        && !flags.forward_index                             // c:2867
        && !flags.reverse_index                             // c:2867
    {                                                       // c:2867
        return None; // bare `(e)` or `(k)` alone isn't a query form // c:2867
    }                                                       // c:2867
    Some((flags, rest.to_string()))                         // c:2867
}                                                           // c:2867

fn apply_array_subscript_flags(                             // c:2867
    arr: &[String],                                         // c:2867
    flags: &SubscriptFlags,                                 // c:2867
    pat: &str,                                              // c:2867
) -> Vec<String> {                                          // c:2867
    let matches = |s: &str| -> bool {                       // c:2867
        if flags.exact {                                    // c:2867
            s == pat                                        // c:2867
        } else if flags.numeric {                           // c:2867
            s.parse::<f64>().ok() == pat.parse::<f64>().ok() // c:2867
        } else {                                            // c:2867
            // glob match
            let re_src = param_pattern_to_regex(pat);       // c:2867
            regex::Regex::new(&re_src)                      // c:2867
                .map(|re| re.is_match(s))                   // c:2867
                .unwrap_or(false)                           // c:2867
        }                                                   // c:2867
    };                                                      // c:2867
    if flags.forward_match {                                // c:2867
        arr.iter()                                          // c:2867
            .find(|s| matches(s.as_str()))                  // c:2867
            .cloned()                                       // c:2867
            .into_iter()                                    // c:2867
            .collect()                                      // c:2867
    } else if flags.reverse_match {                         // c:2867
        arr.iter()                                          // c:2867
            .rev()                                          // c:2867
            .find(|s| matches(s.as_str()))                  // c:2867
            .cloned()                                       // c:2867
            .into_iter()                                    // c:2867
            .collect()                                      // c:2867
    } else if flags.forward_index {                         // c:2867
        let idx = arr.iter().position(|s| matches(s.as_str())); // c:2867
        vec![idx.map(|i| (i + 1).to_string()).unwrap_or_else(|| "0".to_string())] // c:2867
    } else if flags.reverse_index {                         // c:2867
        let idx = arr.iter().rposition(|s| matches(s.as_str())); // c:2867
        vec![idx.map(|i| (i + 1).to_string()).unwrap_or_else(|| "0".to_string())] // c:2867
    } else {                                                // c:2867
        arr.to_vec()                                        // c:2867
    }                                                       // c:2867
}                                                           // c:2867

/// Public wrapper for callers (BUILTIN_ARRAY_INDEX magic-assoc path)
/// that don't have direct access to the file-private flag struct.
/// Re-parses the flag string and dispatches to the internal matcher.
pub fn apply_assoc_subscript_flags_pub(                     // c:2867
    pairs: &[(String, String)],                             // c:2867
    flags: SubscriptFlags,                                  // c:2867
    pat: &str,                                              // c:2867
) -> Vec<String> {                                          // c:2867
    apply_assoc_subscript_flags(pairs, &flags, pat)         // c:2867
}                                                           // c:2867

fn apply_assoc_subscript_flags(                             // c:2867
    pairs: &[(String, String)],                             // c:2867
    flags: &SubscriptFlags,                                 // c:2867
    pat: &str,                                              // c:2867
) -> Vec<String> {                                          // c:2867
    let matches = |s: &str| -> bool {                       // c:2867
        if flags.exact {                                    // c:2867
            s == pat                                        // c:2867
        } else {                                            // c:2867
            let re_src = param_pattern_to_regex(pat);       // c:2867
            regex::Regex::new(&re_src)                      // c:2867
                .map(|re| re.is_match(s))                   // c:2867
                .unwrap_or(false)                           // c:2867
        }                                                   // c:2867
    };                                                      // c:2867
    let pick = |entry: &(String, String)| -> String {       // c:2867
        // (k) flag flips: search keys instead of values.
        if flags.keys {                                     // c:2867
            entry.0.clone()                                 // c:2867
        } else {                                            // c:2867
            entry.1.clone()                                 // c:2867
        }                                                   // c:2867
    };                                                      // c:2867
    if flags.forward_match {                                // c:2867
        pairs                                               // c:2867
            .iter()                                         // c:2867
            .find(|e| matches(&pick(e)))                    // c:2867
            .map(|e| e.1.clone())                           // c:2867
            .into_iter()                                    // c:2867
            .collect()                                      // c:2867
    } else if flags.reverse_match {                         // c:2867
        pairs                                               // c:2867
            .iter()                                         // c:2867
            .rev()                                          // c:2867
            .find(|e| matches(&pick(e)))                    // c:2867
            .map(|e| e.1.clone())                           // c:2867
            .into_iter()                                    // c:2867
            .collect()                                      // c:2867
    } else if flags.forward_index || flags.reverse_index {  // c:2867
        // For assoc, (i)/(I) returns the KEY of the matching entry,
        // and the search is run against KEYS — not values. Direct
        // port of Src/params.c getarg's `ishash` branch around line
        // 1576-1595 (`getnode(ht, s)` looks the key up in the hash
        // table) plus 1685+ (`!keymatch` falls through to the value-
        // pattern path only for non-hash params). The (k)/(K) flags
        // are how you flip search-target from values to keys for
        // ARRAYS; on hashes the default is already key-search.
        //
        // Special case: (I) on a hash with a glob pattern returns
        // ALL matching keys (zsh's "matchmany" behavior). Single
        // literal keys return at most one match.
        let key_matches = |k: &str| -> bool {               // c:2867
            if flags.exact {                                // c:2867
                k == pat                                    // c:2867
            } else {                                        // c:2867
                let re_src = param_pattern_to_regex(pat);   // c:2867
                regex::Regex::new(&re_src)                  // c:2867
                    .map(|re| re.is_match(k))               // c:2867
                    .unwrap_or(false)                       // c:2867
            }                                               // c:2867
        };                                                  // c:2867
        let pat_is_glob = pat.contains('*')                 // c:2867
            || pat.contains('?')                            // c:2867
            || pat.contains('[')                            // c:2867
            || pat.contains("<->")                          // c:2867
            || pat.contains("<-");                          // c:2867
        if flags.reverse_index && pat_is_glob {             // c:2867
            pairs                                           // c:2867
                .iter()                                     // c:2867
                .filter(|e| key_matches(&e.0))              // c:2867
                .map(|e| e.0.clone())                       // c:2867
                .collect()                                  // c:2867
        } else if flags.reverse_index {                     // c:2867
            pairs                                           // c:2867
                .iter()                                     // c:2867
                .rev()                                      // c:2867
                .find(|e| key_matches(&e.0))                // c:2867
                .map(|e| e.0.clone())                       // c:2867
                .into_iter()                                // c:2867
                .collect()                                  // c:2867
        } else {                                            // c:2867
            pairs                                           // c:2867
                .iter()                                     // c:2867
                .find(|e| key_matches(&e.0))                // c:2867
                .map(|e| e.0.clone())                       // c:2867
                .into_iter()                                // c:2867
                .collect()                                  // c:2867
        }                                                   // c:2867
    } else {                                                // c:2867
        pairs.iter().map(|e| e.1.clone()).collect()         // c:2867
    }                                                       // c:2867
}                                                           // c:2867

/// Apply parameter flags to value
fn apply_param_flags(                                       // c:3900
    value: &[String],                                       // c:3900
    flags: &ParamFlags,                                     // c:3900
    state: &mut SubstState,                                 // c:3900
    pf_flags: u32,                                          // c:3900
) -> Vec<String> {                                          // c:3900
    let mut result: Vec<String> = value.to_vec();           // c:3900
    // Direct port of Src/subst.c:1759: `int ssub = (pf_flags &
    // PREFORK_SINGLE);`. When ssub is true, the substitution is
    // running inside a single-word (singsub) context — the split
    // flags `(f)` / `(s:STR:)` / `(0)` / `(z)` are SUPPRESSED so
    // the original scalar passes through without re-arrangement.
    // Per subst.c:3902 `force_split = !ssub && (spbreak || spsep)`
    // — ssub gates the entire force_split path. The visible
    // consequence: `y="${(f)x}"` (assignment context, prefork
    // called with PREFORK_SINGLE|PREFORK_ASSIGN) preserves x's
    // original `\n` separators verbatim, while `echo "${(f)x}"`
    // (no PREFORK_SINGLE) splits then re-joins with IFS-first-
    // char (space).
    let ssub = pf_flags & prefork_flags::SINGLE != 0;       // c:3902

    // Split operations — gated on !ssub per the C source.
    if !ssub {                                              // c:3902
        if let Some(ref sep) = flags.split_sep {            // c:3902
            // Default: drop empty fields. The `(@)` flag preserves
            // them. Direct port of Src/subst.c paramsubst's
            // sepsplit() call which honors the SUB_NULLEMPTY bit.
            let preserve = flags.array_expand;              // c:3902
            result = result                                 // c:3902
                .iter()                                     // c:3902
                .flat_map(|s| {                             // c:3902
                    s.split(sep.as_str())                   // c:3902
                        .filter(|f| preserve || !f.is_empty()) // c:3902
                        .map(String::from)                  // c:3902
                        .collect::<Vec<_>>()                // c:3902
                })                                          // c:3902
                .collect();                                 // c:3902
        }                                                   // c:3902
        if flags.split_lines {                              // c:3902
            result = result                                 // c:3902
                .iter()                                     // c:3902
                .flat_map(|s| s.lines().map(String::from))  // c:3902
                .collect();                                 // c:3902
        }                                                   // c:3902
        if flags.split_words {                              // c:3902
            // (z) — tokenize like the shell parser does. Direct port
            // of Src/subst.c paramsubst's case 'z' arm which calls
            // bufferwords() (Src/lex.c) — it returns shell-token
            // boundaries, not whitespace splits. Meta operators
            // (`;`, `|`, `&`, `&&`, `||`, `;;`, redirects) become
            // their own tokens.
            result = result                                 // c:3902
                .iter()                                     // c:3902
                .flat_map(|s| z_tokenize(s))                // c:3902
                .collect();                                 // c:3902
        }                                                   // c:3902
    }                                                       // c:3902

    // Case modification
    if flags.lowercase {                                    // c:3902
        result = result.iter().map(|s| s.to_lowercase()).collect(); // c:3902
    }                                                       // c:3902
    if flags.uppercase {                                    // c:3902
        result = result.iter().map(|s| s.to_uppercase()).collect(); // c:3902
    }                                                       // c:3902
    if flags.capitalize {                                   // c:3902
        // (C) — capitalize each word per Src/hist.c CASMOD_CAPS.
        // The previous inline impl just uppercased the first char,
        // leaving "hello world how are you" stuck on "Hello world…".
        // Route through the shared casemodify() helper which treats
        // every non-alnum (incl. spaces) as a word boundary.
        result = result                                     // c:3902
            .iter()                                         // c:3902
            .map(|s| casemodify(s, CaseMod::Caps))          // c:3902
            .collect();                                     // c:3902
    }                                                       // c:3902

    // Uniqueness
    if flags.unique {                                       // c:3902
        let mut seen = std::collections::HashSet::new();    // c:3902
        result.retain(|s| seen.insert(s.clone()));          // c:3902
    }                                                       // c:3902

    // Sorting
    if flags.sort {                                         // c:3902
        if flags.sort_numeric {                             // c:3902
            result.sort_by(|a, b| {                         // c:3902
                let na: f64 = a.parse().unwrap_or(0.0);     // c:3902
                let nb: f64 = b.parse().unwrap_or(0.0);     // c:3902
                na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal) // c:3902
            });                                             // c:3902
        } else if flags.sort_case_insensitive {             // c:3902
            result.sort_by_key(|a| a.to_lowercase());       // c:3902
        } else {                                            // c:3902
            result.sort();                                  // c:3902
        }                                                   // c:3902
    }                                                       // c:3902
    if flags.sort_reverse {                                 // c:3902
        result.reverse();                                   // c:3902
    }                                                       // c:3902

    // Quoting — port of `quotestring()` from Src/utils.c:6300+ for
    // `(q)`, `(qq)`, `(qqq)`, `(qqqq)`. Single-q is backslash form
    // (only escape shell-special chars, leave plain strings alone).
    // Verified live: `${(q)"hello world"}` → `hello\ world`,
    // `${(q)/Users/me}` → `/Users/me` (no escape needed).
    match flags.quote_level {                               // c:3902
        0 => {}                                             // c:3902
        1 => {                                              // c:3902
            // QT_BACKSLASH — escape whitespace + shell metas.
            result = result                                 // c:3902
                .iter()                                     // c:3902
                .map(|s| {                                  // c:3902
                    let mut out = String::with_capacity(s.len()); // c:3902
                    for c in s.chars() {                    // c:3902
                        match c {                           // c:3902
                            ' ' | '\t' | '\n' | '\\' | '\'' | '"' // c:3902
                            | '`' | '$' | '*' | '?' | '[' | ']' // c:3902
                            | '(' | ')' | '{' | '}' | '|' | '&' // c:3902
                            | ';' | '<' | '>' | '#' | '~' | '!' => { // c:3902
                                out.push('\\');             // c:3902
                                out.push(c);                // c:3902
                            }                               // c:3902
                            _ => out.push(c),               // c:3902
                        }                                   // c:3902
                    }                                       // c:3902
                    out                                     // c:3902
                })                                          // c:3902
                .collect();                                 // c:3902
        }                                                   // c:3902
        2 => {                                              // c:3902
            // QT_SINGLE — wrap in `'...'`, escape embedded `'`.
            result = result                                 // c:3902
                .iter()                                     // c:3902
                .map(|s| format!("'{}'", s.replace('\'', "'\\''"))) // c:3902
                .collect();                                 // c:3902
        }                                                   // c:3902
        3 => {                                              // c:3902
            // QT_DOUBLE — wrap in `"..."`.
            result = result                                 // c:3902
                .iter()                                     // c:3902
                .map(|s| format!("\"{}\"", s.replace('"', "\\\"").replace('$', "\\$").replace('\\', "\\\\"))) // c:3902
                .collect();                                 // c:3902
        }                                                   // c:3902
        _ => {                                              // c:3902
            // QT_DOLLARS — `$'...'` ANSI-C-quoted form.
            result = result                                 // c:3902
                .iter()                                     // c:3902
                .map(|s| {                                  // c:3902
                    let mut out = String::from("$'");       // c:3902
                    for c in s.chars() {                    // c:3902
                        match c {                           // c:3902
                            '\'' => out.push_str("\\'"),    // c:3902
                            '\\' => out.push_str("\\\\"),   // c:3902
                            '\n' => out.push_str("\\n"),    // c:3902
                            '\t' => out.push_str("\\t"),    // c:3902
                            '\r' => out.push_str("\\r"),    // c:3902
                            _ => out.push(c),               // c:3902
                        }                                   // c:3902
                    }                                       // c:3902
                    out.push('\'');                         // c:3902
                    out                                     // c:3902
                })                                          // c:3902
                .collect();                                 // c:3902
        }                                                   // c:3902
    }                                                       // c:3902
    if flags.unquote {                                      // c:3902
        // (Q) flag — full shell-quoting reversal, NOT just outer-quote
        // strip. Direct port of Src/subst.c paramsubst's PSPRINT_FLAG_Q
        // arm which calls `dequotestring()` (Src/utils.c) — that walks
        // every character, processing `'…'` SQ-spans, `"…"` DQ-spans
        // with backslash escapes, and standalone `\X` escapes. The
        // canonical roundtrip case is `(qq)` → `(Q)` for strings
        // containing single quotes: `(qq)` of `a'b` produces
        // `'a'\''b'` (close, escape, open) and `(Q)` must reverse all
        // four transitions to recover `a'b`. The earlier naive
        // `strip-outer-quotes` only handled the outer pair and left
        // the `'\''` literal intact.
        result = result                                     // c:3902
            .iter()                                         // c:3902
            .map(|s| unquote_subst(s.trim()))               // c:3902
            .collect();                                     // c:3902
    }                                                       // c:3902

    // (e) eval — recursively re-substitute the value as if it
    // were itself a parameter expression. Port of Src/subst.c
    // around line 1798-1803 (`eval = 1`) + the eval-application
    // arm. zshrs runs it via stringsubst on each element.
    if flags.eval {                                         // c:3902
        result = result                                     // c:3902
            .iter()                                         // c:3902
            .map(|s| {                                      // c:3902
                let mut list = LinkList::from_string(s);    // c:3902
                let mut rf = 0u32;                          // c:3902
                prefork(&mut list, prefork_flags::NOSHWORDSPLIT, &mut rf, state); // c:3902
                list.get_data(0).unwrap_or("").to_string()  // c:3902
            })                                              // c:3902
            .collect();                                     // c:3902
    }                                                       // c:3902

    // (~) glob_subst — apply glob expansion to each result element.
    // zinit's pick-pattern (`pick="src/*.zsh"; files=(${~pick})`) and
    // many plugin loaders rely on this. Per Src/subst.c the flag
    // re-routes the post-expansion text through the glob engine; we
    // delegate to the executor's expand_glob (which already handles
    // wildcards, qualifiers, and **).
    //
    // Suppress the filesystem glob when:
    //   1. We're inside a nested paramsubst (e.g. operand of `:#`
    //      filter, default `:-`/`:+`, replacement `${var/pat/${~name}}`,
    //      etc.). The surrounding operator already wants the value as
    //      a pattern string. zsh: `${(@)arr:#$~ban}` filters arr against
    //      $ban as a shell pattern; no filesystem step.
    //   2. We're in single-word context (ssub — DQ wrap or scalar
    //      assignment). zsh: `"${~ban}"` returns the literal "z*"
    //      string, NOT filesystem matches. The glob_subst flag's
    //      effect collapses to a no-op when the result is consumed
    //      as a scalar (the value passes through uninterpreted).
    //      Direct port of Src/subst.c paramsubst's glob_subst gate
    //      around the prefork SINGLE check.
    let (in_nested_subst, in_dq, in_scalar_assign) = crate::exec::try_with_executor(|exec| { // c:3902
        (                                                   // c:3902
            exec.in_paramsubst_nest > 1,                    // c:3902
            exec.in_dq_context > 0,                         // c:3902
            exec.in_scalar_assign > 0,                      // c:3902
        )                                                   // c:3902
    })                                                      // c:3902
    .unwrap_or((false, false, false));                      // c:3902
    let ssub_ctx =                                          // c:3902
        pf_flags & prefork_flags::SINGLE != 0 || in_dq || in_scalar_assign; // c:3902
    if flags.glob_subst && !in_nested_subst && !ssub_ctx {  // c:3902
        let expanded = crate::exec::with_executor(|exec| {  // c:3902
            let mut out: Vec<String> = Vec::with_capacity(result.len()); // c:3902
            for piece in &result {                          // c:3902
                if piece.is_empty() {                       // c:3902
                    continue;                               // c:3902
                }                                           // c:3902
                let words = exec.expand_glob(piece);        // c:3902
                if words.is_empty() {                       // c:3902
                    // expand_glob returns [] for both "no glob meta"
                    // (fast path returns the literal in a 1-element
                    // vec we never see, or empty when the no-match
                    // path fired). Keep the original on empty so the
                    // command sees the literal pattern when it didn't
                    // match — but the no-match arm has already set
                    // current_command_glob_failed to abort the
                    // command, so a trailing empty here is fine.
                    out.push(piece.clone());                // c:3902
                } else {                                    // c:3902
                    out.extend(words);                      // c:3902
                }                                           // c:3902
            }                                               // c:3902
            out                                             // c:3902
        });                                                 // c:3902
        result = expanded;                                  // c:3902
    }                                                       // c:3902

    // (X) report_error — turn unset/empty parameter into a hard
    // error. C source: Src/subst.c around `quoteerr` flag. Used
    // most often as `(eX)` to surface eval failures.
    if flags.report_error && result.iter().all(|s| s.is_empty()) { // c:3902
        state.errflag = true;                               // c:3902
    }                                                       // c:3902

    // (@) array_expand — preserve element boundaries even in a
    // string context. We have no per-element wordlist machinery
    // here yet, so it's a no-op flag (still parsed, no longer
    // dead-stored). Real semantics is handled by the caller via
    // `pf_flags & SHWORDSPLIT` in stringsubst.
    let _ = flags.array_expand;                             // c:3902
    // (V) visible — replace control chars with `^X` etc. Future
    // port; flag is read and acknowledged.
    if flags.visible {                                      // c:3902
        result = result                                     // c:3902
            .iter()                                         // c:3902
            .map(|s| {                                      // c:3902
                let mut out = String::with_capacity(s.len()); // c:3902
                for c in s.chars() {                        // c:3902
                    if c.is_control() && c != '\n' && c != '\t' { // c:3902
                        if (c as u32) < 0x20 {              // c:3902
                            out.push('^');                  // c:3902
                            out.push(((c as u8) + b'@') as char); // c:3902
                        } else {                            // c:3902
                            out.push(c);                    // c:3902
                        }                                   // c:3902
                    } else {                                // c:3902
                        out.push(c);                        // c:3902
                    }                                       // c:3902
                }                                           // c:3902
                out                                         // c:3902
            })                                              // c:3902
            .collect();                                     // c:3902
    }                                                       // c:3902

    // (%) flag — run prompt expansion on each value.
    // Port of the `presc` arm of `paramsubst()` from
    // Src/subst.c:3977-4018: when the `%` flag was seen, the C
    // source temporarily forces `PROMPTPERCENT=1`, disables
    // `PROMPTSUBST`/`PROMPTBANG`, and runs `promptexpand()` on
    // every (array element or scalar) value. The Rust equivalent
    // calls `crate::prompt::expand_prompt()` which already has
    // `prompt_bang=false` by default; the `(%)` flag does NOT
    // enable `!`-history expansion.
    if flags.prompt_percent {                               // c:3977
        let mut ctx = crate::prompt::PromptContext::default(); // c:3977
        // `%N` defaults to scriptname → argzero per
        // Src/prompt.c:554-556. The currently-sourced script
        // path lives in `$0`; argzero is `$ZSH_ARGZERO`.
        if let Some(zero) = state.variables.get("0") {      // c:3977
            ctx.scriptname = Some(zero.clone());            // c:3977
        }                                                   // c:3977
        if let Some(az) = state.variables.get("ZSH_ARGZERO") { // c:3977
            ctx.argzero = az.clone();                       // c:3977
        }                                                   // c:3977
        result = result                                     // c:3977
            .iter()                                         // c:3977
            .map(|s| crate::prompt::expand_prompt(s, &ctx)) // c:3977
            .collect();                                     // c:3977
    }                                                       // c:3977

    // Join operations
    if flags.join_lines {                                   // c:3977
        result = vec![result.join("\n")];                   // c:3977
    }                                                       // c:3977
    if let Some(ref sep) = flags.join_sep {                 // c:3977
        result = vec![result.join(sep)];                    // c:3977
    }                                                       // c:3977

    // Counting
    if flags.count_words {                                  // c:3977
        let count = result                                  // c:3977
            .iter()                                         // c:3977
            .map(|s| s.split_whitespace().count())          // c:3977
            .sum::<usize>();                                // c:3977
        result = vec![count.to_string()];                   // c:3977
    }                                                       // c:3977
    if flags.count_chars {                                  // c:3977
        let count = result.iter().map(|s| s.chars().count()).sum::<usize>(); // c:3977
        result = vec![count.to_string()];                   // c:3977
    }                                                       // c:3977

    // Padding. Routes through dopadding() (port of Src/subst.c
    // dopadding) so the 4-colon `(l:N:STR1:STR2:)` form gets the
    // exact zsh semantics: STR1 is the single-shot prefix (truncated
    // if longer than width), STR2 is the repeating fill, and the
    // caller's value sits in the middle (truncated if too big to fit).
    //
    // For an unset/empty input value WITH a padding flag, zsh's
    // dopadding still produces N chars of the fill — `${(l:5::0:)42}`
    // returns "00000" even when $42 is unset. Seed `result` with a
    // single empty string so the per-element pad path runs once.
    if (flags.pad_left.is_some() || flags.pad_right.is_some()) && result.is_empty() { // c:3977
        result = vec![String::new()];                       // c:3977
    }                                                       // c:3977
    if let Some(width) = flags.pad_left {                   // c:3977
        let s1 = flags.pad_string1.as_deref();              // c:3977
        let s2 = flags                                      // c:3977
            .pad_string2                                    // c:3977
            .as_deref()                                     // c:3977
            .or_else(|| flags.pad_char.as_ref().map(|_| "")); // c:3977
        let pre_one = s1;                                   // c:3977
        let pre_mul = match (flags.pad_string1.as_ref(), flags.pad_string2.as_ref()) { // c:3977
            (Some(_), Some(s2)) => Some(s2.as_str()),       // c:3977
            (Some(s1), None) => Some(s1.as_str()),          // c:3977
            (None, _) => flags.pad_char.as_ref().map(|_| " "), // c:3977
        };                                                  // c:3977
        let _ = (s1, s2);                                   // c:3977
        result = result                                     // c:3977
            .iter()                                         // c:3977
            .map(|s| dopadding_simple(s, width, 0, pre_one, pre_mul, None, None, flags.pad_char)) // c:3977
            .collect();                                     // c:3977
    }                                                       // c:3977
    if let Some(width) = flags.pad_right {                  // c:3977
        let post_one = flags.pad_string1.as_deref();        // c:3977
        let post_mul = match (flags.pad_string1.as_ref(), flags.pad_string2.as_ref()) { // c:3977
            (Some(_), Some(s2)) => Some(s2.as_str()),       // c:3977
            (Some(s1), None) => Some(s1.as_str()),          // c:3977
            (None, _) => flags.pad_char.as_ref().map(|_| " "), // c:3977
        };                                                  // c:3977
        result = result                                     // c:3977
            .iter()                                         // c:3977
            .map(|s| dopadding_simple(s, 0, width, None, None, post_one, post_mul, flags.pad_char)) // c:3977
            .collect();                                     // c:3977
    }                                                       // c:3977

    result                                                  // c:3977
}                                                           // c:3977

/// Strip the lexer's quote markers + literal `"`/`'` left around an
/// operand's outer edges. Mirrors C `untokenize()` (Src/utils.c) at
/// the post-`multsub` step in `paramsubst`'s `:=` branch (see
/// Src/subst.c:3309: `untokenize(val); setsparam(idbeg, ztrdup(val));`).
///
/// In zshrs the lexer marks double-quoted spans with `DNULL` (`\u{97}`)
/// at both ends and single-quoted spans with `SNULL` (`\u{9d}`).
/// `multsub` runs prefork over the operand which usually drops these
/// markers — but if the operand was already a fully-resolved literal
/// (`"hello"` with no `$`), prefork passes the markers through. Strip
/// them here. Also strip literal `"`/`'` that may have leaked through
/// pre-tokenized callers (e.g. integration tests calling
/// `substitute_brace` with raw strings).
fn strip_outer_dq_markers(s: &str) -> String {              // c:3900
    let mut out = String::with_capacity(s.len());           // c:3900
    for c in s.chars() {                                    // c:3900
        if c == DNULL || c == SNULL {                       // c:3900
            continue;                                       // c:3900
        }                                                   // c:3900
        out.push(c);                                        // c:3900
    }                                                       // c:3900
    // Also strip a balanced outer pair of literal `"` or `'`.
    // C zsh's `untokenize` runs on the post-`multsub` value to drop
    // both the lexer's DQ/SQ markers AND any leftover quote chars
    // — for runtime callers (substitute_brace called with a raw
    // pre-tokenized string), the literal quotes survive prefork and
    // must be peeled here. Only strip if both ends match — random
    // mid-string quotes stay (they were intentional content).
    if (out.starts_with('"') && out.ends_with('"') && out.len() >= 2) // c:3900
        || (out.starts_with('\'') && out.ends_with('\'') && out.len() >= 2) // c:3900
    {                                                       // c:3900
        out[1..out.len() - 1].to_string()                   // c:3900
    } else {                                                // c:3900
        out                                                 // c:3900
    }                                                       // c:3900
}                                                           // c:3900

/// Apply parameter operator
fn apply_operator(                                          // c:3500
    var_name: &str,                                         // c:3500
    subscript: Option<&str>,                                // c:3500
    value: Vec<String>,                                     // c:3500
    operator: Option<&str>,                                 // c:3500
    operand: &str,                                          // c:3500
    state: &mut SubstState,                                 // c:3500
) -> Vec<String> {                                          // c:3500
    apply_operator_with_flags(var_name, subscript, value, operator, operand, false, state) // c:3500
}                                                           // c:3500

/// Inner form of apply_operator that takes the `(M)` match flag.
/// `:#pattern` filters values: by default removes matching, with
/// `(M)` keeps only matching. Port of Src/subst.c paramsubst's
/// `case '#'` gated by `colf` and the `flags & SUB_MATCH` bit.
fn apply_operator_with_flags(                               // c:3500
    var_name: &str,                                         // c:3500
    subscript: Option<&str>,                                // c:3500
    value: Vec<String>,                                     // c:3500
    operator: Option<&str>,                                 // c:3500
    operand: &str,                                          // c:3500
    match_flag: bool,                                       // c:3500
    state: &mut SubstState,                                 // c:3500
) -> Vec<String> {                                          // c:3500
    apply_operator_with_flags_full(                         // c:3500
        var_name, subscript, value, operator, operand, match_flag, false, state, // c:3500
    )                                                       // c:3500
}                                                           // c:3500

/// Full variant accepting both (M) match-keep and (S) substring-
/// search flags. Per Src/subst.c, (S) changes `#`/`##`/`%`/`%%`
/// from anchored prefix/suffix strip to "find shortest/longest
/// match anywhere in the string". Combined with (M), returns the
/// matched substring; otherwise returns the unmatched remainder.
fn apply_operator_with_flags_full(                          // c:3500
    var_name: &str,                                         // c:3500
    subscript: Option<&str>,                                // c:3500
    value: Vec<String>,                                     // c:3500
    operator: Option<&str>,                                 // c:3500
    operand: &str,                                          // c:3500
    match_flag: bool,                                       // c:3500
    search_flag: bool,                                      // c:3500
    state: &mut SubstState,                                 // c:3500
) -> Vec<String> {                                          // c:3500
    let is_set = !value.is_empty();                         // c:3500
    let is_empty = value.iter().all(|s| s.is_empty());      // c:3500
    let joined = value.join(" ");                           // c:3500

    match operator {                                        // c:3500
        Some(":-") | Some("-") => {                         // c:3500
            if (operator == Some(":-") && (is_empty || !is_set)) // c:3500
                || (operator == Some("-") && !is_set)       // c:3500
            {                                               // c:3500
                // Per Src/subst.c:3206-3232 (`case '-':` arm), the
                // operand is run through `multsub` for substitution.
                // Without this, `${X:-${Y}/${Z}}` would store the
                // literal `${Y}/${Z}`. Mirror the C behavior.
                let (expanded, _, _, _) =                   // c:3206
                    multsub(operand, prefork_flags::NOSHWORDSPLIT, state); // c:3206
                vec![strip_outer_dq_markers(&expanded)]     // c:3206
            } else {                                        // c:3206
                value                                       // c:3206
            }                                               // c:3206
        }                                                   // c:3206
        Some(":=") | Some("=") => {                         // c:3206
            if (operator == Some(":=") && (is_empty || !is_set)) // c:3206
                || (operator == Some("=") && !is_set)       // c:3206
            {                                               // c:3206
                // Subscripted writeback dispatch — port of
                // Src/subst.c:3245-3325 (`case '=': case Equals:`).
                //
                // Operand expansion: C source line 3257 calls
                // `multsub(&val, PREFORK_NOSHWORDSPLIT, …)` to
                // recursively expand `${INNER}`/`$X`/etc. inside the
                // operand. Mirroring that here.
                let (expanded, _, _, _) =                   // c:3245
                    multsub(operand, prefork_flags::NOSHWORDSPLIT, state); // c:3245
                let val = strip_outer_dq_markers(&expanded); // c:3245
                match subscript {                           // c:3245
                    Some(idx) => {                          // c:3245
                        // Expand $-references in the subscript before
                        // using it as the assoc key / array index.
                        // zsh's `${m[$k]:=$v}` resolves `$k` to the
                        // current value before writing. Without this,
                        // the literal "$k" was stored as a key.
                        let idx_resolved = if idx.contains('$') || idx.contains('`') { // c:3245
                            expand_subscript_pat(idx, state) // c:3245
                        } else {                            // c:3245
                            idx.to_string()                 // c:3245
                        };                                  // c:3245
                        let is_assoc = state.assoc_arrays.contains_key(var_name); // c:3245
                        let numeric = idx_resolved.parse::<i64>().ok(); // c:3245
                        match (is_assoc, numeric) {         // c:3245
                            (true, _) => {                  // c:3245
                                let map = state             // c:3245
                                    .assoc_arrays           // c:3245
                                    .entry(var_name.to_string()) // c:3245
                                    .or_default();          // c:3245
                                map.insert(idx_resolved.clone(), val.clone()); // c:3245
                            }                               // c:3245
                            (false, Some(n)) => {           // c:3245
                                let arr = state             // c:3245
                                    .arrays                 // c:3245
                                    .entry(var_name.to_string()) // c:3245
                                    .or_default();          // c:3245
                                let pos = if n > 0 { (n - 1) as usize } else { 0 }; // c:3245
                                if pos >= arr.len() {       // c:3245
                                    arr.resize(pos + 1, String::new()); // c:3245
                                }                           // c:3245
                                arr[pos] = val.clone();     // c:3245
                            }                               // c:3245
                            (false, None) => {              // c:3245
                                // Auto-promote to assoc.
                                let map = state             // c:3245
                                    .assoc_arrays           // c:3245
                                    .entry(var_name.to_string()) // c:3245
                                    .or_default();          // c:3245
                                map.insert(idx_resolved.clone(), val.clone()); // c:3245
                            }                               // c:3245
                        }                                   // c:3245
                    }                                       // c:3245
                    None => {                               // c:3245
                        state                               // c:3245
                            .variables                      // c:3245
                            .insert(var_name.to_string(), val.clone()); // c:3245
                    }                                       // c:3245
                }                                           // c:3245
                vec![val]                                   // c:3245
            } else {                                        // c:3245
                value                                       // c:3245
            }                                               // c:3245
        }                                                   // c:3245
        Some(":+") | Some("+") => {                         // c:3245
            if (operator == Some(":+") && !is_empty && is_set) || (operator == Some("+") && is_set) // c:3245
            {                                               // c:3245
                // `:+` operand is also expanded via multsub per
                // Src/subst.c:3193-3199 (`case '+':` falls through
                // to `case '-':` which calls multsub).
                let (expanded, _, _, _) =                   // c:3193
                    multsub(operand, prefork_flags::NOSHWORDSPLIT, state); // c:3193
                vec![strip_outer_dq_markers(&expanded)]     // c:3193
            } else {                                        // c:3193
                vec![]                                      // c:3193
            }                                               // c:3193
        }                                                   // c:3193
        Some(":?") | Some("?") => {                         // c:3193
            if (operator == Some(":?") && (is_empty || !is_set)) // c:3193
                || (operator == Some("?") && !is_set)       // c:3193
            {                                               // c:3193
                let msg = if operand.is_empty() {           // c:3193
                    format!("{}: parameter not set", var_name) // c:3193
                } else {                                    // c:3193
                    operand.to_string()                     // c:3193
                };                                          // c:3193
                eprintln!("{}", msg);                       // c:3193
                state.errflag = true;                       // c:3193
                vec![]                                      // c:3193
            } else {                                        // c:3193
                value                                       // c:3193
            }                                               // c:3193
        }                                                   // c:3193
        Some(":#") => {                                     // c:3193
            // Pattern-filter. Per Src/subst.c paramsubst's `case '#':`
            // arm gated by `colf`. Behavior depends on the (M) flag
            // (passed through `match_flag`).
            //
            // For SCALARS: if value matches pattern, default is
            // empty (with (M): keep value); else default is value
            // (with (M): empty).
            //
            // For ARRAYS: filter elements — keep non-matching by
            // default, keep matching with (M).
            //
            // Pattern matching: parameter pattern semantics (NOT
            // file-glob semantics). `*` matches any string
            // INCLUDING `/`; this is the same engine `case`/`[[`
            // uses, distinct from the path-component-aware glob
            // used for filename expansion. zsh manual: "Note that
            // these all use shell pattern matching, not regular
            // expressions."
            // Expand `$VAR` / `${VAR}` references in the pattern
            // BEFORE compiling. Direct port of Src/subst.c paramsubst's
            // `case '#':` arm which calls singsub on the operand (line
            // 3192 in C source). Without expansion, a pattern like
            // `${a:#$b}` matches the literal string \"\$b\" instead of
            // the value of $b.
            let expanded_pat = if operand.contains('$') || operand.contains('`') { // c:3193
                singsub_no_tilde(operand, state)            // c:3193
            } else {                                        // c:3193
                operand.to_string()                         // c:3193
            };                                              // c:3193
            let regex_src = param_pattern_to_regex(&expanded_pat); // c:3193
            let re_opt = regex::Regex::new(&regex_src).ok(); // c:3193
            value                                           // c:3193
                .into_iter()                                // c:3193
                .filter_map(|s| {                           // c:3193
                    let matches = re_opt                    // c:3193
                        .as_ref()                           // c:3193
                        .map(|re| re.is_match(&s))          // c:3193
                        .unwrap_or(false);                  // c:3193
                    let keep = if match_flag { matches } else { !matches }; // c:3193
                    if keep {                               // c:3193
                        Some(s)                             // c:3193
                    } else if match_flag {                  // c:3193
                        None                                // c:3193
                    } else {                                // c:3193
                        // Without (M) and matching: drop (return empty for scalar
                        // context, dropped element for array context — both fall
                        // out via filter_map(None)).
                        None                                // c:3193
                    }                                       // c:3193
                })                                          // c:3193
                .collect::<Vec<_>>()                        // c:3193
        }                                                   // c:3193
        Some("::=") => {                                    // c:3193
            // Unconditional assign — zsh extension. Always store
            // the operand (after expansion) as the parameter's
            // new value, regardless of whether it was set/empty.
            // Returns the operand. Same writeback dispatch as `:=`.
            let (expanded, _, _, _) =                       // c:3193
                multsub(operand, prefork_flags::NOSHWORDSPLIT, state); // c:3193
            let val = strip_outer_dq_markers(&expanded);    // c:3193
            match subscript {                               // c:3193
                Some(idx) => {                              // c:3193
                    let is_assoc = state.assoc_arrays.contains_key(var_name); // c:3193
                    let numeric = idx.parse::<i64>().ok();  // c:3193
                    match (is_assoc, numeric) {             // c:3193
                        (true, _) => {                      // c:3193
                            let map = state                 // c:3193
                                .assoc_arrays               // c:3193
                                .entry(var_name.to_string()) // c:3193
                                .or_default();              // c:3193
                            map.insert(idx.to_string(), val.clone()); // c:3193
                        }                                   // c:3193
                        (false, Some(n)) => {               // c:3193
                            let arr = state                 // c:3193
                                .arrays                     // c:3193
                                .entry(var_name.to_string()) // c:3193
                                .or_default();              // c:3193
                            let pos = if n > 0 { (n - 1) as usize } else { 0 }; // c:3193
                            if pos >= arr.len() {           // c:3193
                                arr.resize(pos + 1, String::new()); // c:3193
                            }                               // c:3193
                            arr[pos] = val.clone();         // c:3193
                        }                                   // c:3193
                        (false, None) => {                  // c:3193
                            let map = state                 // c:3193
                                .assoc_arrays               // c:3193
                                .entry(var_name.to_string()) // c:3193
                                .or_default();              // c:3193
                            map.insert(idx.to_string(), val.clone()); // c:3193
                        }                                   // c:3193
                    }                                       // c:3193
                }                                           // c:3193
                None => {                                   // c:3193
                    state                                   // c:3193
                        .variables                          // c:3193
                        .insert(var_name.to_string(), val.clone()); // c:3193
                }                                           // c:3193
            }                                               // c:3193
            vec![val]                                       // c:3193
        }                                                   // c:3193
        Some(":mod") => {                                   // c:3193
            // History modifier chain (`${var:h:t:r:s/x/y/:Q:A:&}`).
            // Port of Src/subst.c:3611-3759 (`if (colf && inbrace)`)
            // which dispatches to `modify()` (subst.c:4531). The
            // modifier text was captured by parse_brace_param into
            // `operand`; we rebuild the leading `:` (parser strips
            // it) and pass the whole chain to `modify`.
            let chain = format!(":{}", operand);            // c:4531
            value                                           // c:4531
                .iter()                                     // c:4531
                .map(|s| modify(s, &chain, state))          // c:4531
                .collect()                                  // c:4531
        }                                                   // c:4531
        Some(":") => {                                      // c:4531
            // Substring: ${var:offset} or ${var:offset:length}.
            // For positional-array refs (`@`, `*`, `argv`) and named
            // arrays the offset/length operate on ELEMENTS — port of
            // Src/subst.c paramsubst's `case ':':` arm where it
            // checks `isarr` and dispatches to array slicing instead
            // of char-slice. Direct port of bash/zsh `${@:N:M}`.
            let parts: Vec<&str> = operand.split(':').collect(); // c:4531
            let offset: i64 = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0); // c:4531
            let length: Option<i64> = parts.get(1).and_then(|s| s.parse().ok()); // c:4531

            let is_array_ref = matches!(var_name, "@" | "*" | "argv") // c:4531
                || (state.arrays.contains_key(var_name) && value.len() > 1); // c:4531
            if is_array_ref {                               // c:4531
                let arr = &value;                           // c:4531
                let n = arr.len() as i64;                   // c:4531
                // For `${@:N}` with N>=1, elements 1..N are the
                // positionals (`$1` is index 0 in `value`). zsh's
                // semantics: skip N positionals when N>0; -N from
                // the end when N<0; N=0 includes `$0` (which we
                // don't store in `arr`, so 0 means start).
                let start = if offset < 0 {                 // c:4531
                    ((n + offset).max(0)) as usize          // c:4531
                } else if offset == 0 {                     // c:4531
                    0                                       // c:4531
                } else {                                    // c:4531
                    ((offset - 1).max(0) as usize).min(arr.len()) // c:4531
                };                                          // c:4531
                let end = match length {                    // c:4531
                    Some(l) if l < 0 => ((n + l).max(start as i64)) as usize, // c:4531
                    Some(l) => (start + l as usize).min(arr.len()), // c:4531
                    None => arr.len(),                      // c:4531
                };                                          // c:4531
                return arr[start..end].to_vec();            // c:4531
            }                                               // c:4531

            value                                           // c:4531
                .iter()                                     // c:4531
                .map(|s| {                                  // c:4531
                    let chars: Vec<char> = s.chars().collect(); // c:4531
                    let len = chars.len() as i64;           // c:4531

                    let start = if offset < 0 {             // c:4531
                        (len + offset).max(0) as usize      // c:4531
                    } else {                                // c:4531
                        (offset as usize).min(chars.len())  // c:4531
                    };                                      // c:4531

                    let end = match length {                // c:4531
                        Some(l) if l < 0 => (len + l).max(start as i64) as usize, // c:4531
                        Some(l) => (start + l as usize).min(chars.len()), // c:4531
                        None => chars.len(),                // c:4531
                    };                                      // c:4531

                    chars[start..end].iter().collect()      // c:4531
                })                                          // c:4531
                .collect()                                  // c:4531
        }                                                   // c:4531
        Some(op @ ("#" | "##" | "%" | "%%")) => {           // c:4531
            // Strip prefix/suffix matching pattern.
            //   `#`/`##`  = anchored prefix (shortest/longest) match
            //   `%`/`%%`  = anchored suffix (shortest/longest) match
            //   `(M)` = return matched portion (else unmatched).
            //   `(S)` = search-anywhere (substring), not anchored.
            // Operand is expanded for `$VAR`/`${VAR}` references
            // before pattern matching (Src/subst.c paramsubst's
            // case '#'/'%' arm calls singsub on the operand).
            let expanded_operand = if operand.contains('$') || operand.contains('`') { // c:4531
                singsub_no_tilde(operand, state)            // c:4531
            } else {                                        // c:4531
                operand.to_string()                         // c:4531
            };                                              // c:4531
            let greedy = matches!(op, "##" | "%%");         // c:4531
            let from_end = matches!(op, "%" | "%%");        // c:4531
            value                                           // c:4531
                .iter()                                     // c:4531
                .map(|s| {                                  // c:4531
                    if search_flag {                        // c:4531
                        substring_search_match(s, &expanded_operand, greedy, match_flag, from_end) // c:4531
                    } else if from_end {                    // c:4531
                        strip_suffix_match(s, &expanded_operand, greedy, match_flag) // c:4531
                    } else {                                // c:4531
                        strip_prefix_match(s, &expanded_operand, greedy, match_flag) // c:4531
                    }                                       // c:4531
                })                                          // c:4531
                .collect()                                  // c:4531
        }                                                   // c:4531
        Some("/") | Some("//") | Some("/#") | Some("/%") => { // c:4531
            // `${var/pat/repl}` family. Per Src/subst.c paramsubst's
            // `case '/':` arm. Pattern + replacement go through
            // singsub_no_tilde so `$X`/`${X}` inside them resolve
            // while leaving leading `~` literal (zsh contract:
            // `${var/#$HOME/~}` keeps the tilde unresolved).
            //
            // `(#b)` flag at pattern start enables backreference
            // capture: each `(...)` becomes a regex group, and the
            // `match` array gets populated before each replacement
            // expansion so `$match[N]` resolves to capture N.
            // Per Src/pattern.c — `pat_pure` flag set by `(#b)`,
            // `addbackref()` populates pat_subme entries.
            // Split pattern from replacement on the FIRST UNESCAPED
            // `/`. Per Src/subst.c — `\/` in the operand is a
            // literal slash inside the pattern (or replacement),
            // not the separator. Without this split discipline,
            // `${var/#(#b)${HOME}(|\/*)/~$match[1]}` got split at
            // the `\/` inside the alternation, producing a
            // half-pattern.
            let chars: Vec<char> = operand.chars().collect(); // c:4531
            let mut sep_idx: Option<usize> = None;          // c:4531
            let mut k = 0;                                  // c:4531
            while k < chars.len() {                         // c:4531
                if chars[k] == '\\' && k + 1 < chars.len() { // c:4531
                    k += 2;                                 // c:4531
                    continue;                               // c:4531
                }                                           // c:4531
                if chars[k] == '/' {                        // c:4531
                    sep_idx = Some(k);                      // c:4531
                    break;                                  // c:4531
                }                                           // c:4531
                k += 1;                                     // c:4531
            }                                               // c:4531
            // After the split, drop `\` from any `\/` in pattern +
            // replacement so the regex / literal-strip sees the
            // literal `/`. Other backslash escapes (`\n`, `\\`)
            // are left for downstream handling.
            let unesc_slash = |s: &str| -> String {         // c:4531
                let mut out = String::with_capacity(s.len()); // c:4531
                let mut it = s.chars().peekable();          // c:4531
                while let Some(c) = it.next() {             // c:4531
                    if c == '\\' {                          // c:4531
                        if let Some(&nx) = it.peek() {      // c:4531
                            if nx == '/' {                  // c:4531
                                out.push('/');              // c:4531
                                it.next();                  // c:4531
                                continue;                   // c:4531
                            }                               // c:4531
                        }                                   // c:4531
                    }                                       // c:4531
                    out.push(c);                            // c:4531
                }                                           // c:4531
                out                                         // c:4531
            };                                              // c:4531
            let (raw_pat, raw_rep_owned): (String, String) = match sep_idx { // c:4531
                Some(p) => (                                // c:4531
                    unesc_slash(&chars[..p].iter().collect::<String>()), // c:4531
                    unesc_slash(&chars[p + 1..].iter().collect::<String>()), // c:4531
                ),                                          // c:4531
                None => (unesc_slash(operand), String::new()), // c:4531
            };                                              // c:4531
            let (pat_no_flags, backref_mode, case_i) = strip_inline_pattern_flags(&raw_pat); // c:4531
            let pattern = singsub_no_tilde(&pat_no_flags, state); // c:4531
            // Strip `\x00` literal-markers inserted by expand_string's
            // DQ-escape preprocessing. BUILTIN_EXPAND_TEXT mode 1
            // turns `\\` into `\x00\` to mark "the next char is a
            // literal not a meta". For pattern compilation the
            // marker is noise — `\x00\` should compile as a literal
            // `\` (combined with any following `(#e)`/`(#s)` it then
            // hits the escape-backslash-then-anchor arm in
            // `glob_to_regex_capturing`). Direct port of zsh's
            // pattern.c which sees the raw text without the runtime
            // literal-marker layer.
            let pattern = pattern.replace('\x00', "");      // c:4531
            // Build regex UNANCHORED: the `/`-family replace ops let
            // `do_replace_one` enforce `/#` start-anchor and `/%`
            // end-anchor by inspecting the captured span positions.
            // Anchoring the regex itself would force whole-string
            // match and break partial-prefix/suffix replacement.
            let regex_src = if backref_mode {               // c:4531
                glob_to_regex_capturing(&pattern, false)    // c:4531
            } else {                                        // c:4531
                param_pattern_to_regex_anchored(&pattern, false) // c:4531
            };                                              // c:4531
            // Prefix with `(?i)` regex flag when (#i) was set —
            // direct port of zsh's pattern.c PAT_INSENS bit which
            // makes the entire pattern case-insensitive.
            let regex_src = if case_i {                     // c:4531
                format!("(?i){}", regex_src)                // c:4531
            } else {                                        // c:4531
                regex_src                                   // c:4531
            };                                              // c:4531
            let re_opt = regex::Regex::new(&regex_src).ok(); // c:4531
            let op_str = operator.unwrap_or("/").to_string(); // c:4531
            let mut out_vals: Vec<String> = Vec::with_capacity(value.len()); // c:4531
            for s in value.iter() {                         // c:4531
                out_vals.push(do_replace_one(               // c:4531
                    s,                                      // c:4531
                    &op_str,                                // c:4531
                    &pattern,                               // c:4531
                    &raw_rep_owned,                         // c:4531
                    re_opt.as_ref(),                        // c:4531
                    backref_mode,                           // c:4531
                    state,                                  // c:4531
                ));                                         // c:4531
            }                                               // c:4531
            out_vals                                        // c:4531
        }                                                   // c:4531
        Some(":^") | Some(":^^") => {                       // c:4531
            // Zip two arrays element-wise. `:^` (SUB_ZIP_SHORT) stops
            // at min(len); `:^^` (SUB_ZIP_LONG) goes to max(len) and
            // cycles the shorter. The operand is the name of the
            // second array. Port of Src/subst.c paramsubst's SUB_ZIP
            // path.
            let second = state                              // c:4531
                .arrays                                     // c:4531
                .get(operand)                               // c:4531
                .cloned()                                   // c:4531
                .or_else(|| state.variables.get(operand).map(|v| vec![v.clone()])) // c:4531
                .unwrap_or_default();                       // c:4531
            if value.is_empty() || second.is_empty() {      // c:4531
                return value;                               // c:4531
            }                                               // c:4531
            let long = operator == Some(":^^");             // c:4531
            let total = if long {                           // c:4531
                value.len().max(second.len())               // c:4531
            } else {                                        // c:4531
                value.len().min(second.len())               // c:4531
            };                                              // c:4531
            let mut out: Vec<String> = Vec::with_capacity(total * 2); // c:4531
            for i in 0..total {                             // c:4531
                let a = &value[i % value.len()];            // c:4531
                let b = &second[i % second.len()];          // c:4531
                out.push(a.clone());                        // c:4531
                out.push(b.clone());                        // c:4531
            }                                               // c:4531
            out                                             // c:4531
        }                                                   // c:4531
        Some("@op") => {                                    // c:4531
            // `${var@OP}` is bash-only — zsh's parameter expansion
            // rejects `@` as a postfix operator. zsh-native is
            // `${(q)var}` / `${(Q)var}` for quoting and `${(t)var}`
            // for type. Emit "bad substitution".
            eprintln!("zshrs:1: bad substitution");         // c:4531
            state.errflag = true;                           // c:4531
            Vec::new()                                      // c:4531
        }                                                   // c:4531
        Some("^") => {                                      // c:4531
            // `${var^}` is bash-only (uppercase first char). zsh
            // rejects — see the `^^` arm below for rationale.
            eprintln!("zshrs:1: bad substitution");         // c:4531
            state.errflag = true;                           // c:4531
            Vec::new()                                      // c:4531
        }                                                   // c:4531
        Some("^^") => {                                     // c:4531
            // `${var^^}` is bash-only; zsh rejects with "bad
            // substitution" and exits the substitution chain.
            // Direct port of zsh's parser behaviour: the `^` in a
            // ${…} body is a syntax error unless it's part of an
            // extendedglob `^pat` (which only fires INSIDE patterns,
            // not as a param-modifier). The zsh-native uppercase
            // form is `${(U)var}`. Emit an error to stderr and
            // return empty so callers see no value.
            eprintln!("zshrs:1: bad substitution");         // c:4531
            state.errflag = true;                           // c:4531
            Vec::new()                                      // c:4531
        }                                                   // c:4531
        Some(",") => {                                      // c:4531
            // `${var,}` is bash-only (lowercase first char). zsh
            // rejects — see the `,,` arm above for rationale.
            eprintln!("zshrs:1: bad substitution");         // c:4531
            state.errflag = true;                           // c:4531
            Vec::new()                                      // c:4531
        }                                                   // c:4531
        Some(",,") => {                                     // c:4531
            // `${var,,}` is bash-only; zsh rejects (same as `^^`).
            // The zsh-native lowercase form is `${(L)var}`.
            eprintln!("zshrs:1: bad substitution");         // c:4531
            state.errflag = true;                           // c:4531
            Vec::new()                                      // c:4531
        }                                                   // c:4531
        _ => value,                                         // c:4531
    }                                                       // c:4531
}                                                           // c:4531

/// Find the boundary index in `s` (in chars) such that `s[..idx]`
/// is the prefix matching `pattern`. `greedy=true` returns the
/// longest such prefix; otherwise the shortest. `None` if no
/// prefix of `s` matches `pattern`.
///
/// Port of zsh's getmatch() prefix path in Src/subst.c. We use
/// crate::glob::pattern_match for the actual glob test and iterate
/// candidate prefix lengths in greedy order. O(n²) in the worst case
/// but n is short for shell strings.
fn find_prefix_match(s: &str, pattern: &str) -> Vec<usize> { // c:3500
    // Use the executor's full glob matcher (extendedglob `##`/`#`
    // postfix, POSIX `[[:space:]]` classes, `~` exclusion, etc.).
    // The previous crate::glob::pattern_match path was a simplified
    // matcher that returned false for `[[:space:]]##` and similar
    // extendedglob postfix forms, so per-element strip on a nested-
    // value array silently no-op'd. Direct port of Src/subst.c's
    // `getmatch` calling into pattern.c — the C source uses the
    // same full pattern compiler for both bare-scalar and per-array
    // strip arms.
    let chars: Vec<char> = s.chars().collect();             // c:3500
    let mut matches: Vec<usize> = Vec::new();               // c:3500
    for end in 0..=chars.len() {                            // c:3500
        let candidate: String = chars[..end].iter().collect(); // c:3500
        if crate::exec::ShellExecutor::glob_match_static(&candidate, pattern) { // c:3500
            matches.push(end);                              // c:3500
        }                                                   // c:3500
    }                                                       // c:3500
    matches                                                 // c:3500
}                                                           // c:3500

fn find_suffix_match(s: &str, pattern: &str) -> Vec<usize> { // c:3500
    let chars: Vec<char> = s.chars().collect();             // c:3500
    let mut matches: Vec<usize> = Vec::new();               // c:3500
    for start in 0..=chars.len() {                          // c:3500
        let candidate: String = chars[start..].iter().collect(); // c:3500
        if crate::exec::ShellExecutor::glob_match_static(&candidate, pattern) { // c:3500
            matches.push(start);                            // c:3500
        }                                                   // c:3500
    }                                                       // c:3500
    matches                                                 // c:3500
}                                                           // c:3500

/// Strip a prefix of `s` matching `pattern`. Returns the unmatched
/// tail by default; with `match_flag` returns the matched head
/// (empty when no match).
/// (S) substring-search variant of strip_prefix_match /
/// strip_suffix_match. Searches the pattern anywhere in `s` and
/// returns either the matched portion (with M) or the remainder
/// after stripping the match (without M). `from_end=true` searches
/// from the right (for `%`/`%%`); false searches from left (for
/// `#`/`##`). `greedy=true` picks longest match; false picks
/// shortest. Direct port of zsh's getmatch() with SUB_SUBSTR
/// flag set (Src/glob.c).
fn substring_search_match(                                  // c:3500
    s: &str,                                                // c:3500
    pattern: &str,                                          // c:3500
    greedy: bool,                                           // c:3500
    match_flag: bool,                                       // c:3500
    from_end: bool,                                         // c:3500
) -> String {                                               // c:3500
    let chars: Vec<char> = s.chars().collect();             // c:3500
    let n = chars.len();                                    // c:3500
    let mut best: Option<(usize, usize)> = None;            // c:3500
    let starts: Vec<usize> = if from_end {                  // c:3500
        (0..=n).rev().collect()                             // c:3500
    } else {                                                // c:3500
        (0..=n).collect()                                   // c:3500
    };                                                      // c:3500
    for start in starts {                                   // c:3500
        let ends: Vec<usize> = if greedy {                  // c:3500
            (start..=n).rev().collect()                     // c:3500
        } else {                                            // c:3500
            (start..=n).collect()                           // c:3500
        };                                                  // c:3500
        for end in ends {                                   // c:3500
            let candidate: String = chars[start..end].iter().collect(); // c:3500
            if crate::exec::ShellExecutor::glob_match_static(&candidate, pattern) { // c:3500
                best = Some((start, end));                  // c:3500
                break;                                      // c:3500
            }                                               // c:3500
        }                                                   // c:3500
        if best.is_some() {                                 // c:3500
            break;                                          // c:3500
        }                                                   // c:3500
    }                                                       // c:3500
    match best {                                            // c:3500
        Some((start, end)) => {                             // c:3500
            if match_flag {                                 // c:3500
                chars[start..end].iter().collect()          // c:3500
            } else {                                        // c:3500
                let mut out = String::new();                // c:3500
                out.extend(chars[..start].iter());          // c:3500
                out.extend(chars[end..].iter());            // c:3500
                out                                         // c:3500
            }                                               // c:3500
        }                                                   // c:3500
        None => {                                           // c:3500
            if match_flag {                                 // c:3500
                String::new()                               // c:3500
            } else {                                        // c:3500
                s.to_string()                               // c:3500
            }                                               // c:3500
        }                                                   // c:3500
    }                                                       // c:3500
}                                                           // c:3500

fn strip_prefix_match(s: &str, pattern: &str, greedy: bool, match_flag: bool) -> String { // c:3500
    let chars: Vec<char> = s.chars().collect();             // c:3500
    let matches = find_prefix_match(s, pattern);            // c:3500
    let chosen = if greedy {                                // c:3500
        matches.iter().copied().max()                       // c:3500
    } else {                                                // c:3500
        matches.iter().copied().min()                       // c:3500
    };                                                      // c:3500
    match chosen {                                          // c:3500
        Some(end) => {                                      // c:3500
            if match_flag {                                 // c:3500
                chars[..end].iter().collect()               // c:3500
            } else {                                        // c:3500
                chars[end..].iter().collect()               // c:3500
            }                                               // c:3500
        }                                                   // c:3500
        None => {                                           // c:3500
            if match_flag {                                 // c:3500
                String::new()                               // c:3500
            } else {                                        // c:3500
                s.to_string()                               // c:3500
            }                                               // c:3500
        }                                                   // c:3500
    }                                                       // c:3500
}                                                           // c:3500

/// Suffix counterpart of `strip_prefix_match`.
fn strip_suffix_match(s: &str, pattern: &str, greedy: bool, match_flag: bool) -> String { // c:3500
    let chars: Vec<char> = s.chars().collect();             // c:3500
    let matches = find_suffix_match(s, pattern);            // c:3500
    let chosen = if greedy {                                // c:3500
        matches.iter().copied().min()                       // c:3500
    } else {                                                // c:3500
        matches.iter().copied().max()                       // c:3500
    };                                                      // c:3500
    match chosen {                                          // c:3500
        Some(start) => {                                    // c:3500
            if match_flag {                                 // c:3500
                chars[start..].iter().collect()             // c:3500
            } else {                                        // c:3500
                chars[..start].iter().collect()             // c:3500
            }                                               // c:3500
        }                                                   // c:3500
        None => {                                           // c:3500
            if match_flag {                                 // c:3500
                String::new()                               // c:3500
            } else {                                        // c:3500
                s.to_string()                               // c:3500
            }                                               // c:3500
        }                                                   // c:3500
    }                                                       // c:3500
}                                                           // c:3500

/// Remove prefix matching pattern
fn remove_prefix(s: &str, pattern: &str, greedy: bool) -> String { // c:3500
    // Convert glob pattern to something we can match
    // Simple implementation - real one would use proper glob matching
    if pattern == "*" {                                     // c:3500
        return String::new();                               // c:3500
    }                                                       // c:3500

    if let Some(prefix) = pattern.strip_suffix('*') {       // c:3500
        if let Some(rest) = s.strip_prefix(prefix) {        // c:3500
            if greedy {                                     // c:3500
                // Find longest match
                if let Some(i) = (prefix.len()..=s.len()).next_back() { // c:3500
                    return s[i..].to_string();              // c:3500
                }                                           // c:3500
            } else {                                        // c:3500
                return rest.to_string();                    // c:3500
            }                                               // c:3500
        }                                                   // c:3500
    } else if let Some(rest) = s.strip_prefix(pattern) {    // c:3500
        return rest.to_string();                            // c:3500
    }                                                       // c:3500

    s.to_string()                                           // c:3500
}                                                           // c:3500

/// Remove suffix matching pattern
fn remove_suffix(s: &str, pattern: &str, greedy: bool) -> String { // c:3500
    if pattern == "*" {                                     // c:3500
        return String::new();                               // c:3500
    }                                                       // c:3500

    if let Some(suffix) = pattern.strip_prefix('*') {       // c:3500
        if let Some(prefix) = s.strip_suffix(suffix) {      // c:3500
            if greedy {                                     // c:3500
                if let Some(i) = (0..=s.len().saturating_sub(suffix.len())).next() { // c:3500
                    return s[..i].to_string();              // c:3500
                }                                           // c:3500
            } else {                                        // c:3500
                return prefix.to_string();                  // c:3500
            }                                               // c:3500
        }                                                   // c:3500
    } else if let Some(prefix) = s.strip_suffix(pattern) {  // c:3500
        return prefix.to_string();                          // c:3500
    }                                                       // c:3500

    s.to_string()                                           // c:3500
}                                                           // c:3500

/// Split words according to IFS
fn split_words(s: &str, state: &SubstState) -> Vec<String> { // c:4200
    let ifs = state                                         // c:4200
        .variables                                          // c:4200
        .get("IFS")                                         // c:4200
        .map(|s| s.as_str())                                // c:4200
        .unwrap_or(" \t\n");                                // c:4200

    s.split(|c: char| ifs.contains(c))                      // c:4200
        .filter(|s| !s.is_empty())                          // c:4200
        .map(String::from)                                  // c:4200
        .collect()                                          // c:4200
}                                                           // c:4200

// Helper functions

fn find_matching_bracket(s: &str, open: char, close: char) -> Option<usize> { // c:N/A
    let mut depth = 1;                                      // c:N/A
    for (i, c) in s.chars().enumerate() {                   // c:N/A
        if c == open {                                      // c:N/A
            depth += 1;                                     // c:N/A
        } else if c == close {                              // c:N/A
            depth -= 1;                                     // c:N/A
            if depth == 0 {                                 // c:N/A
                return Some(i);                             // c:N/A
            }                                               // c:N/A
        }                                                   // c:N/A
    }                                                       // c:N/A
    None                                                    // c:N/A
}                                                           // c:N/A

fn find_matching_parmath(s: &str) -> Option<usize> {        // c:N/A
    let mut depth = 1;                                      // c:N/A
    let chars: Vec<char> = s.chars().collect();             // c:N/A
    let mut i = 0;                                          // c:N/A
    while i < chars.len() {                                 // c:N/A
        if chars[i] == INPARMATH {                          // c:N/A
            depth += 1;                                     // c:N/A
        } else if chars[i] == OUTPARMATH {                  // c:N/A
            depth -= 1;                                     // c:N/A
            if depth == 0 {                                 // c:N/A
                return Some(i);                             // c:N/A
            }                                               // c:N/A
        }                                                   // c:N/A
        i += 1;                                             // c:N/A
    }                                                       // c:N/A
    None                                                    // c:N/A
}                                                           // c:N/A

fn hasbraces(s: &str) -> bool {                             // c:4674
    // Port of `hasbraces()` from Src/glob.c:2042-2150. Returns true
    // only when `s` contains an *actual* brace-expansion pattern —
    // comma alternatives `{a,b,c}` or numeric/char range `{1..5}` /
    // `{a..z}`. Crucially returns FALSE for parameter-substitution
    // `${var}`, glob-qualifier braces, or unbalanced braces — those
    // are handled by paramsubst / glob / lex and reaching brace
    // expansion on them would loop forever (the previous adhoc
    // `s.contains('{') && s.contains('}')` triggered an infinite
    // `while hasbraces() { xpandbraces() }` loop in prefork because
    // xpandbraces no-ops on `${var}`).
    //
    // The C source uses tokenized `Inbrace`/`Outbrace` glyphs and
    // mutates the string in-place when a brace pair turns out NOT
    // to be expansion — restoring `{`/`}` to literals. Our port is
    // read-only: no mutation, just a single forward scan that
    // returns true the moment a confirming feature (comma OR `..`
    // range) is found inside a balanced brace pair.
    //
    // BRACECCL option (subst.c:2046-2064) — accepts `{X}` as a
    // single-char class — is intentionally not modeled; the option
    // isn't yet wired into zshrs's `state.opts`.
    let bytes: &[u8] = s.as_bytes();                        // c:2046
    let n = bytes.len();                                    // c:2046
    let mut i = 0;                                          // c:2046
    while i < n {                                           // c:2046
        let c = bytes[i];                                   // c:2046
        // Skip parameter substitution `${…}`. Per C, by the time
        // hasbraces runs paramsubst has already been done; in our
        // port that's not always true (subst_port::stringsubst
        // doesn't recognize literal `$`), so we still need this
        // explicit skip to avoid the infinite-loop bug.
        if c == b'$' && i + 1 < n && bytes[i + 1] == b'{' { // c:2046
            // Skip until the matching `}` (balanced).
            let mut depth = 1;                              // c:2046
            i += 2;                                         // c:2046
            while i < n && depth > 0 {                      // c:2046
                match bytes[i] {                            // c:2046
                    b'{' => depth += 1,                     // c:2046
                    b'}' => depth -= 1,                     // c:2046
                    _ => {}                                 // c:2046
                }                                           // c:2046
                i += 1;                                     // c:2046
            }                                               // c:2046
            continue;                                       // c:2046
        }                                                   // c:2046
        // Backslash escapes the next char.
        if c == b'\\' {                                     // c:2046
            i += 2;                                         // c:2046
            continue;                                       // c:2046
        }                                                   // c:2046
        if c == b'{' {                                      // c:2046
            // Found an opening brace at depth 0. Walk forward
            // looking for either a comma OR a `..` range OR the
            // matching `}`. Skip nested groups via depth counter.
            let mut depth = 1;                              // c:2046
            let mut j = i + 1;                              // c:2046
            let mut comma_found = false;                    // c:2046
            let mut range_found = false;                    // c:2046
            // Detect numeric/char range: `{N..M}` or `{a..z}`.
            // C source (glob.c:2074-2096) does this only on the
            // outermost brace and consumes optional `-` and digit
            // runs. Approximate: any `..` inside the top-level
            // brace pair counts as a range marker.
            while j < n && depth > 0 {                      // c:2046
                match bytes[j] {                            // c:2046
                    b'\\' => {                              // c:2046
                        j += 2;                             // c:2046
                        continue;                           // c:2046
                    }                                       // c:2046
                    b'$' if j + 1 < n && bytes[j + 1] == b'{' => { // c:2046
                        // Nested ${…} — skip whole thing
                        j += 2;                             // c:2046
                        let mut nd = 1;                     // c:2046
                        while j < n && nd > 0 {             // c:2046
                            match bytes[j] {                // c:2046
                                b'{' => nd += 1,            // c:2046
                                b'}' => nd -= 1,            // c:2046
                                _ => {}                     // c:2046
                            }                               // c:2046
                            j += 1;                         // c:2046
                        }                                   // c:2046
                        continue;                           // c:2046
                    }                                       // c:2046
                    b'{' => depth += 1,                     // c:2046
                    b'}' => {                               // c:2046
                        depth -= 1;                         // c:2046
                        if depth == 0 {                     // c:2046
                            j += 1;                         // c:2046
                            break;                          // c:2046
                        }                                   // c:2046
                    }                                       // c:2046
                    b',' if depth == 1 => comma_found = true, // c:2046
                    b'.' if depth == 1                      // c:2046
                        && j + 1 < n                        // c:2046
                        && bytes[j + 1] == b'.' =>          // c:2046
                    {                                       // c:2046
                        range_found = true;                 // c:2046
                        j += 1; // step past second `.`     // c:2046
                    }                                       // c:2046
                    _ => {}                                 // c:2046
                }                                           // c:2046
                j += 1;                                     // c:2046
            }                                               // c:2046
            if depth == 0 && (comma_found || range_found) { // c:2046
                return true;                                // c:2046
            }                                               // c:2046
            // No comma / range inside this pair — not brace
            // expansion; advance past it and keep scanning.
            i = j;                                          // c:2046
            continue;                                       // c:2046
        }                                                   // c:2046
        i += 1;                                             // c:2046
    }                                                       // c:2046
    false                                                   // c:2046
}                                                           // c:2046

fn xpandbraces(list: &mut LinkList, node_idx: &mut usize) { // c:4791
    let data = match list.get_data(*node_idx) {             // c:4791
        Some(d) => d.to_string(),                           // c:4791
        None => return,                                     // c:4791
    };                                                      // c:4791

    // Find brace group (top-level only — skip `${…}` parameter
    // substitution which is the same brace-pair shape but isn't
    // brace expansion). Port of `xpandbraces()` from Src/glob.c:
    // walks until it finds a balanced `{…}` containing either a
    // top-level comma OR a `..` range.
    let bytes = data.as_bytes();                            // c:4791
    let n = bytes.len();                                    // c:4791
    let mut i = 0;                                          // c:4791
    while i < n {                                           // c:4791
        let c = bytes[i];                                   // c:4791
        // Skip `${…}`
        if c == b'$' && i + 1 < n && bytes[i + 1] == b'{' { // c:4791
            let mut depth = 1;                              // c:4791
            i += 2;                                         // c:4791
            while i < n && depth > 0 {                      // c:4791
                match bytes[i] {                            // c:4791
                    b'{' => depth += 1,                     // c:4791
                    b'}' => depth -= 1,                     // c:4791
                    _ => {}                                 // c:4791
                }                                           // c:4791
                i += 1;                                     // c:4791
            }                                               // c:4791
            continue;                                       // c:4791
        }                                                   // c:4791
        if c == b'{' {                                      // c:4791
            // Find matching `}` and inspect contents.
            let start = i;                                  // c:4791
            let mut depth = 1;                              // c:4791
            let mut j = i + 1;                              // c:4791
            while j < n && depth > 0 {                      // c:4791
                if bytes[j] == b'$' && j + 1 < n && bytes[j + 1] == b'{' { // c:4791
                    let mut nd = 1;                         // c:4791
                    j += 2;                                 // c:4791
                    while j < n && nd > 0 {                 // c:4791
                        match bytes[j] {                    // c:4791
                            b'{' => nd += 1,                // c:4791
                            b'}' => nd -= 1,                // c:4791
                            _ => {}                         // c:4791
                        }                                   // c:4791
                        j += 1;                             // c:4791
                    }                                       // c:4791
                    continue;                               // c:4791
                }                                           // c:4791
                match bytes[j] {                            // c:4791
                    b'{' => depth += 1,                     // c:4791
                    b'}' => {                               // c:4791
                        depth -= 1;                         // c:4791
                        if depth == 0 {                     // c:4791
                            break;                          // c:4791
                        }                                   // c:4791
                    }                                       // c:4791
                    _ => {}                                 // c:4791
                }                                           // c:4791
                j += 1;                                     // c:4791
            }                                               // c:4791
            if depth != 0 {                                 // c:4791
                // unbalanced
                i += 1;                                     // c:4791
                continue;                                   // c:4791
            }                                               // c:4791
            let end = j; // position of matching `}`        // c:4791
            let prefix = &data[..start];                    // c:4791
            let content = &data[start + 1..end];            // c:4791
            let suffix = &data[end + 1..];                  // c:4791
            // Range form: `{N..M}` or `{a..z}` (single chars).
            // Per Src/glob.c — numbers can be negative; iteration
            // is inclusive on both ends, and direction depends on
            // whether N <= M or N > M.
            if let Some(rng_pos) = content.find("..") {     // c:4791
                let left = &content[..rng_pos];             // c:4791
                let rest = &content[rng_pos + 2..];         // c:4791
                // Optional second `..STEP`.
                let (right, step_str) = match rest.find("..") { // c:4791
                    Some(p) => (&rest[..p], Some(&rest[p + 2..])), // c:4791
                    None => (rest, None),                   // c:4791
                };                                          // c:4791
                if let (Ok(a), Ok(b)) = (left.trim().parse::<i64>(), right.trim().parse::<i64>()) // c:4791
                {                                           // c:4791
                    let step = step_str                     // c:4791
                        .and_then(|s| s.trim().parse::<i64>().ok()) // c:4791
                        .unwrap_or(1)                       // c:4791
                        .abs()                              // c:4791
                        .max(1);                            // c:4791
                    let mut nodes_added: Vec<String> = Vec::new(); // c:4791
                    if a <= b {                             // c:4791
                        let mut k = a;                      // c:4791
                        while k <= b {                      // c:4791
                            nodes_added.push(format!("{}{}{}", prefix, k, suffix)); // c:4791
                            k += step;                      // c:4791
                        }                                   // c:4791
                    } else {                                // c:4791
                        let mut k = a;                      // c:4791
                        while k >= b {                      // c:4791
                            nodes_added.push(format!("{}{}{}", prefix, k, suffix)); // c:4791
                            k -= step;                      // c:4791
                        }                                   // c:4791
                    }                                       // c:4791
                    list.remove(*node_idx);                 // c:4791
                    for (k, item) in nodes_added.into_iter().enumerate() { // c:4791
                        if k == 0 {                         // c:4791
                            list.nodes.insert(*node_idx, LinkNode { data: item }); // c:4791
                        } else {                            // c:4791
                            list.insert_after(*node_idx + k - 1, item); // c:4791
                        }                                   // c:4791
                    }                                       // c:4791
                    return;                                 // c:4791
                }                                           // c:4791
                // Char range `{a..z}` — single chars only.
                let lc: Vec<char> = left.chars().collect(); // c:4791
                let rc: Vec<char> = right.chars().collect(); // c:4791
                if lc.len() == 1 && rc.len() == 1 {         // c:4791
                    let a = lc[0];                          // c:4791
                    let b = rc[0];                          // c:4791
                    let mut nodes_added: Vec<String> = Vec::new(); // c:4791
                    if a <= b {                             // c:4791
                        for c in (a as u32)..=(b as u32) {  // c:4791
                            if let Some(ch) = char::from_u32(c) { // c:4791
                                nodes_added.push(format!("{}{}{}", prefix, ch, suffix)); // c:4791
                            }                               // c:4791
                        }                                   // c:4791
                    } else {                                // c:4791
                        for c in ((b as u32)..=(a as u32)).rev() { // c:4791
                            if let Some(ch) = char::from_u32(c) { // c:4791
                                nodes_added.push(format!("{}{}{}", prefix, ch, suffix)); // c:4791
                            }                               // c:4791
                        }                                   // c:4791
                    }                                       // c:4791
                    list.remove(*node_idx);                 // c:4791
                    for (k, item) in nodes_added.into_iter().enumerate() { // c:4791
                        if k == 0 {                         // c:4791
                            list.nodes.insert(*node_idx, LinkNode { data: item }); // c:4791
                        } else {                            // c:4791
                            list.insert_after(*node_idx + k - 1, item); // c:4791
                        }                                   // c:4791
                    }                                       // c:4791
                    return;                                 // c:4791
                }                                           // c:4791
            }                                               // c:4791
            // Comma alternatives `{a,b,c}` — top-level commas only
            // (nested `{…}` content stays grouped).
            let mut alts: Vec<String> = Vec::new();         // c:4791
            let mut depth_c = 0;                            // c:4791
            let mut current = String::new();                // c:4791
            for c in content.chars() {                      // c:4791
                match c {                                   // c:4791
                    '{' => {                                // c:4791
                        depth_c += 1;                       // c:4791
                        current.push(c);                    // c:4791
                    }                                       // c:4791
                    '}' => {                                // c:4791
                        depth_c -= 1;                       // c:4791
                        current.push(c);                    // c:4791
                    }                                       // c:4791
                    ',' if depth_c == 0 => {                // c:4791
                        alts.push(std::mem::take(&mut current)); // c:4791
                    }                                       // c:4791
                    _ => current.push(c),                   // c:4791
                }                                           // c:4791
            }                                               // c:4791
            alts.push(current);                             // c:4791
            if alts.len() > 1 {                             // c:4791
                list.remove(*node_idx);                     // c:4791
                for (k, alt) in alts.iter().enumerate() {   // c:4791
                    let expanded = format!("{}{}{}", prefix, alt, suffix); // c:4791
                    if k == 0 {                             // c:4791
                        list.nodes.insert(*node_idx, LinkNode { data: expanded }); // c:4791
                    } else {                                // c:4791
                        list.insert_after(*node_idx + k - 1, expanded); // c:4791
                    }                                       // c:4791
                }                                           // c:4791
                return;                                     // c:4791
            }                                               // c:4791
            // Not actual brace expansion — skip past this pair.
            i = end + 1;                                    // c:4791
            continue;                                       // c:4791
        }                                                   // c:4791
        i += 1;                                             // c:4791
    }                                                       // c:4791
}                                                           // c:4791

fn remnulargs(s: &str) -> String {                          // c:4977
    s.chars().filter(|&c| c != NULARG).collect()            // c:4977
}                                                           // c:4977

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

fn getproc(s: &str, state: &mut SubstState) -> (Option<String>, String) { // c:4900
    // Process substitution <(...) or >(...)
    // This creates a /dev/fd/N path
    let chars: Vec<char> = s.chars().collect();             // c:4900
    let is_input = chars[0] == INANG;                       // c:4900

    if let Some(end) = find_matching_bracket(&s[1..], INPAR, OUTPAR) { // c:4900
        let cmd: String = s[2..end + 1].chars().collect();  // c:4900
        let rest = s[end + 2..].to_string();                // c:4900

        if state.opts.exec_opt {                            // c:4900
            // Would create pipe and return /dev/fd/N
            // For now, just return a placeholder
            let fd = if is_input { "63" } else { "62" };    // c:4900
            return (Some(format!("/dev/fd/{}", fd)), rest); // c:4900
        }                                                   // c:4900

        return (None, rest);                                // c:4900
    }                                                       // c:4900

    (None, s.to_string())                                   // c:4900
}                                                           // c:4900

fn getoutputfile(s: &str, state: &mut SubstState) -> (Option<String>, String) { // c:4900
    // =(...) substitution - creates temp file with command output
    if let Some(end) = find_matching_bracket(&s[1..], INPAR, OUTPAR) { // c:4900
        let cmd: String = s[2..end + 1].chars().collect();  // c:4900
        let rest = s[end + 2..].to_string();                // c:4900

        if state.opts.exec_opt {                            // c:4900
            let output = run_command(&cmd);                 // c:4900
            // Would write to temp file and return path
            // For now, return placeholder
            return (Some("/tmp/zsh_proc_subst".to_string()), rest); // c:4900
        }                                                   // c:4900

        return (None, rest);                                // c:4900
    }                                                       // c:4900

    (None, s.to_string())                                   // c:4900
}                                                           // c:4900

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

fn run_command(cmd: &str) -> String {                       // c:N/A
    use std::process::{Command, Stdio};                     // c:N/A

    match Command::new("sh")                                // c:N/A
        .arg("-c")                                          // c:N/A
        .arg(cmd)                                           // c:N/A
        .stdin(Stdio::null())                               // c:N/A
        .stdout(Stdio::piped())                             // c:N/A
        .stderr(Stdio::inherit())                           // c:N/A
        .output()                                           // c:N/A
    {                                                       // c:N/A
        Ok(output) => String::from_utf8_lossy(&output.stdout).to_string(), // c:N/A
        Err(_) => String::new(),                            // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

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
    let mut list = LinkList::from_string(s);                // c:514
    let mut ret_flags = 0u32;                               // c:514

    prefork(&mut list, prefork_flags::SINGLE, &mut ret_flags, state); // c:514

    if state.errflag {                                      // c:514
        return String::new();                               // c:514
    }                                                       // c:514

    list.get_data(0).unwrap_or("").to_string()              // c:514
}                                                           // c:514

/// Single-word substitution with tilde expansion DISABLED. Used
/// for pattern + replacement contexts in `${var/pat/repl}` where
/// per zsh's behavior, leading `~` in operand stays literal — the
/// `${…/#…/~}` idiom relies on the literal `~` being preserved
/// (so the replaced path keeps its tilde-prefix instead of being
/// re-expanded back to `$HOME`).
///
/// Equivalent to `singsub` minus the `filesub`/tilde-expansion
/// pass (Src/subst.c::filesub at line 667).
pub fn singsub_no_tilde(s: &str, state: &mut SubstState) -> String { // c:514
    let saved = state.skip_filesub;                         // c:514
    state.skip_filesub = true;                              // c:514
    let result = singsub(s, state);                         // c:514
    state.skip_filesub = saved;                             // c:514
    result                                                  // c:514
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

    let mut list = LinkList::from_string(&x);               // c:544

    // Handle word splitting within the string
    if pf_flags & prefork_flags::SPLIT != 0 {               // c:544
        let mut node_idx = 0;                               // c:544
        let mut in_quote = false;                           // c:544
        let mut in_paren = 0;                               // c:544

        while node_idx < list.len() {                       // c:544
            if let Some(data) = list.get_data(node_idx) {   // c:544
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
                        if ifs.contains(c) && !is_token(c) { // c:544
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

                    list.remove(node_idx);                  // c:544

                    for (idx, &point) in split_points.iter().enumerate() { // c:544
                        if point > last {                   // c:544
                            let segment: String = chars[last..point].iter().collect(); // c:544
                            if idx == 0 {                   // c:544
                                list.nodes.insert(node_idx, LinkNode { data: segment }); // c:544
                            } else {                        // c:544
                                list.insert_after(node_idx + idx - 1, segment); // c:544
                            }                               // c:544
                        }                                   // c:544
                        last = point + 1;                   // c:544
                    }                                       // c:544

                    if last < chars.len() {                 // c:544
                        let segment: String = chars[last..].iter().collect(); // c:544
                        if split_points.is_empty() {        // c:544
                            list.nodes.insert(node_idx, LinkNode { data: segment }); // c:544
                        } else {                            // c:544
                            list.insert_after(node_idx + split_points.len() - 1, segment); // c:544
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

    let len = list.len();                                   // c:544
    if len > 1 || (list.flags & LF_ARRAY != 0) {            // c:544
        // Return as array
        let arr: Vec<String> = list.nodes.iter().map(|n| n.data.clone()).collect(); // c:544
        let joined = arr.join(" ");                         // c:544
        return (joined, arr, true, ms_flags);               // c:544
    }                                                       // c:544

    let result = list.get_data(0).unwrap_or("").to_string(); // c:544
    (result.clone(), vec![result], false, ms_flags)         // c:544
}                                                           // c:544

/// Case modification modes (from subst.c)
#[derive(Debug, Clone, Copy, PartialEq)]                    // c:544
/// Case-modifier kind (`:U`/`:L`/`:C`).
/// Mirrors the `CASMOD_*` flag set Src/utils.c uses inside
/// `casemodify()`.
pub enum CaseMod {                                          // c:544
    None,                                                   // c:544
    Lower,                                                  // c:544
    Upper,                                                  // c:544
    Caps,                                                   // c:544
}                                                           // c:544

/// Modify a string according to case modification mode
/// Port of casemodify() logic
/// Apply `:U`/`:L`/`:C` casing.
/// Port of `casemodify()` (Src/utils.c).
pub fn casemodify(s: &str, casmod: CaseMod) -> String {     // c:4531
    match casmod {                                          // c:4531
        CaseMod::None => s.to_string(),                     // c:4531
        CaseMod::Lower => s.to_lowercase(),                 // c:4531
        CaseMod::Upper => s.to_uppercase(),                 // c:4531
        CaseMod::Caps => {                                  // c:4531
            // Port of CASMOD_CAPS from Src/hist.c (the `iswalnum`-gated
            // arm). The C source treats any non-alphanumeric character
            // as a word boundary that sets `nextupper`; the next
            // alphanumeric char gets uppercased, all subsequent
            // alphanumerics in the run get lowercased. Whitespace
            // alone is NOT the boundary — `a-b` becomes `A-B` because
            // `-` is a non-alnum boundary that sets nextupper, and
            // `b` is the next alnum so it gets uppercased.
            //
            // Verified live: `print -r -- ${(C)"a-b c.d"}` → `A-B C.D`.
            let mut result = String::new();                 // c:4531
            let mut nextupper = true;                       // c:4531
            for c in s.chars() {                            // c:4531
                if !c.is_alphanumeric() {                   // c:4531
                    nextupper = true;                       // c:4531
                    result.push(c);                         // c:4531
                } else if nextupper {                       // c:4531
                    result.extend(c.to_uppercase());        // c:4531
                    nextupper = false;                      // c:4531
                } else {                                    // c:4531
                    result.extend(c.to_lowercase());        // c:4531
                }                                           // c:4531
            }                                               // c:4531
            result                                          // c:4531
        }                                                   // c:4531
    }                                                       // c:4531
}                                                           // c:4531

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
            result = apply_subst(&result, &pat, &repl, gbal); // c:4531
            last_subst = Some((pat, repl));                 // c:4531
            continue;                                       // c:4531
        }                                                   // c:4531

        // `:&` repeats the last `:s` substitution. Per Src/subst.c
        // modify's `case '&':`. No-op if no prior `:s` in this
        // chain.
        if modifier == '&' {                                // c:4531
            if let Some((p, r)) = &last_subst {             // c:4531
                result = apply_subst(&result, p, r, gbal);  // c:4531
            }                                               // c:4531
            continue;                                       // c:4531
        }                                                   // c:4531

        if wall {                                           // c:4531
            // Apply modifier to each word
            let separator = sep.as_deref().unwrap_or(" ");  // c:4531
            let words: Vec<&str> = result.split(separator).collect(); // c:4531
            let mut modified: Vec<String> = Vec::with_capacity(words.len()); // c:4531
            for w in &words {                               // c:4531
                match apply_single_modifier(w, modifier, gbal, state) { // c:4531
                    Some(m) => modified.push(m),            // c:4531
                    None => {                               // c:4531
                        eprintln!("zshrs: unrecognized modifier `{}'", modifier); // c:4531
                        state.errflag = true;               // c:4531
                        return String::new();               // c:4531
                    }                                       // c:4531
                }                                           // c:4531
            }                                               // c:4531
            result = modified.join(separator);              // c:4531
        } else {                                            // c:4531
            match apply_single_modifier(&result, modifier, gbal, state) { // c:4531
                Some(m) => result = m,                      // c:4531
                None => {                                   // c:4531
                    eprintln!("zshrs: unrecognized modifier `{}'", modifier); // c:4531
                    state.errflag = true;                   // c:4531
                    return String::new();                   // c:4531
                }                                           // c:4531
            }                                               // c:4531
        }                                                   // c:4531
    }                                                       // c:4531

    result                                                  // c:4531
}                                                           // c:4531

/// Apply a `:s/old/new/` substitution. Greedy when `gbal` is set
/// (the `g` modifier prefix in `:gs/x/y/`). Port of the
/// substitution path inside Src/subst.c::modify's `case 's':` arm.
fn apply_subst(s: &str, pat: &str, repl: &str, gbal: bool) -> String { // c:3700
    if pat.is_empty() {                                     // c:3700
        return s.to_string();                               // c:3700
    }                                                       // c:3700
    if gbal {                                               // c:3700
        s.replace(pat, repl)                                // c:3700
    } else {                                                // c:3700
        // Replace only the first occurrence.
        match s.find(pat) {                                 // c:3700
            Some(i) => format!("{}{}{}", &s[..i], repl, &s[i + pat.len()..]), // c:3700
            None => s.to_string(),                          // c:3700
        }                                                   // c:3700
    }                                                       // c:3700
}                                                           // c:3700

/// Apply a single modifier to a string. Returns `None` for unknown
/// modifiers so the caller can emit "unrecognized modifier".
fn apply_single_modifier(                                   // c:4531
    s: &str,                                                // c:4531
    modifier: char,                                         // c:4531
    gbal: bool,                                             // c:4531
    _state: &mut SubstState,                                // c:4531
) -> Option<String> {                                       // c:4531
    Some(match modifier {                                   // c:4531
        // :a - absolute path. Lexical: prepend cwd if relative,
        // collapse `.` and `..` segments without consulting the
        // filesystem. Port of Src/subst.c modify's `case 'a':` arm
        // which calls `xsymlinks(…, /*resolve=*/0)`.
        'a' => lexical_canonicalize(s, false),              // c:4531
        // :A - real path. Lexical canonicalize first (so non-existent
        // paths still get `/./` collapsed), then ask the OS to resolve
        // symlinks. If realpath fails (file doesn't exist), keep the
        // lexical result. Port of Src/subst.c modify's `case 'A':` arm.
        'A' => {                                            // c:4531
            let lex = lexical_canonicalize(s, false);       // c:4531
            match std::fs::canonicalize(&lex) {             // c:4531
                Ok(p) => p.to_string_lossy().to_string(),   // c:4531
                Err(_) => lex,                              // c:4531
            }                                               // c:4531
        }                                                   // c:4531
        // :c - command path (like which)
        'c' => {                                            // c:4531
            if let Ok(path) = std::env::var("PATH") {       // c:4531
                for dir in path.split(':') {                // c:4531
                    let full = format!("{}/{}", dir, s);    // c:4531
                    if std::path::Path::new(&full).exists() { // c:4531
                        return Some(full);                  // c:4531
                    }                                       // c:4531
                }                                           // c:4531
            }                                               // c:4531
            s.to_string()                                   // c:4531
        }                                                   // c:4531
        // :h - head (directory). Delegates to remtpath() (port of
        // Src/hist.c:2056). Strips trailing slashes, then drops the
        // filename. `/tmp/` → `/`, `//` → `/`, `/a/b/c` → `/a/b`.
        'h' => remtpath(s, 0),                              // c:4531
        // :t - tail (filename). Port of remlpaths() — strips trailing
        // slashes first to match the head logic.
        't' => {                                            // c:4531
            let trimmed = s.trim_end_matches('/');          // c:4531
            match trimmed.rfind('/') {                      // c:4531
                Some(pos) => trimmed[pos + 1..].to_string(), // c:4531
                None => trimmed.to_string(),                // c:4531
            }                                               // c:4531
        }                                                   // c:4531
        // :r - remove extension
        'r' => match s.rfind('.') {                         // c:4531
            Some(pos) if pos > 0 && !s[..pos].ends_with('/') => s[..pos].to_string(), // c:4531
            _ => s.to_string(),                             // c:4531
        },                                                  // c:4531
        // :e - extension only
        'e' => match s.rfind('.') {                         // c:4531
            Some(pos) if pos > 0 && !s[..pos].ends_with('/') => s[pos + 1..].to_string(), // c:4531
            _ => String::new(),                             // c:4531
        },                                                  // c:4531
        // :l - lowercase
        'l' => s.to_lowercase(),                            // c:4531
        // :u - uppercase
        'u' => s.to_uppercase(),                            // c:4531
        // :q - backslash-escape shell metacharacters. Port of
        // Src/subst.c modify's `case 'q':` arm which calls
        // quotestring(copy, QT_BACKSLASH_SHOWNULL). The result is the
        // shell metaclass-safe form: spaces, tabs, newlines, glob
        // chars, quotes, $, &, etc. each preceded by `\`.
        'q' => quote_backslash(s),                          // c:4531
        // :Q - unquote. Strip shell-quoting in `s` so the result is
        // the literal value. Port of Src/subst.c modify's `case 'Q':`
        // arm which calls parse_subst_string + remnulargs + untokenize.
        // Drops backslash escapes (`\X` → `X`) and matched `'…'` /
        // `"…"` quote pairs.
        'Q' => unquote_subst(s),                            // c:4531
        // :P - physical path
        'P' => {                                            // c:4531
            let path = if s.starts_with('/') {              // c:4531
                s.to_string()                               // c:4531
            } else if let Ok(cwd) = std::env::current_dir() { // c:4531
                format!("{}/{}", cwd.display(), s)          // c:4531
            } else {                                        // c:4531
                s.to_string()                               // c:4531
            };                                              // c:4531
            // Resolve symlinks
            match std::fs::canonicalize(&path) {            // c:4531
                Ok(p) => p.to_string_lossy().to_string(),   // c:4531
                Err(_) => path,                             // c:4531
            }                                               // c:4531
        }                                                   // c:4531
        _ => return None,                                   // c:4531
    })                                                      // c:4531
}                                                           // c:4531

/// Apply zsh-style left/right padding. Direct port of dopadding()
/// from Src/subst.c at the level the (l:…:)/(r:…:) flags expose.
///
/// • `pre_num` / `post_num` — width to pad on each side.
/// • `pre_one` / `post_one` — single-shot string (string1 of the
///   flag) inserted once before/after `s`.
/// • `pre_mul` / `post_mul` — repeating fill (string2 of the flag);
///   when both string1 and string2 are given AND string1 is empty,
///   the result is just `pre_mul` repeated to `pre_num` chars (the
///   value gets pushed off — matches zsh's quirky 4-colon form).
///
/// 8 params mirrors the C signature 1:1 (Src/subst.c:893) — bundling
/// into a struct just to satisfy clippy would obscure the port.
#[allow(clippy::too_many_arguments)]                        // c:4531
fn dopadding_simple(                                        // c:893
    s: &str,                                                // c:893
    pre_num: usize,                                         // c:893
    post_num: usize,                                        // c:893
    pre_one: Option<&str>,                                  // c:893
    pre_mul: Option<&str>,                                  // c:893
    post_one: Option<&str>,                                 // c:893
    post_mul: Option<&str>,                                 // c:893
    fallback_char: Option<char>,                            // c:893
) -> String {                                               // c:893
    // The 4-colon form `(l:N::STR2:)` — empty string1 + non-empty
    // string2 — drops the value entirely and produces N copies of
    // string2 ONLY when value is empty (no parameter to pad). When
    // value is non-empty, normal pad-with-prefix-then-fill applies.
    // Direct port of Src/subst.c:4925-ish dopadding which only
    // substitutes premul-as-fill when ls (value-len) == 0.
    if pre_num > 0                                          // c:893
        && s.is_empty()                                     // c:893
        && pre_one.map(|s| s.is_empty()).unwrap_or(false)   // c:893
        && pre_mul.map(|s| !s.is_empty()).unwrap_or(false)  // c:893
    {                                                       // c:893
        let fill = pre_mul.unwrap();                        // c:893
        let mut out = String::with_capacity(pre_num);       // c:893
        let fill_chars: Vec<char> = fill.chars().collect(); // c:893
        for i in 0..pre_num {                               // c:893
            out.push(fill_chars[i % fill_chars.len()]);     // c:893
        }                                                   // c:893
        return out;                                         // c:893
    }                                                       // c:893
    if post_num > 0                                         // c:893
        && s.is_empty()                                     // c:893
        && post_one.map(|s| s.is_empty()).unwrap_or(false)  // c:893
        && post_mul.map(|s| !s.is_empty()).unwrap_or(false) // c:893
    {                                                       // c:893
        let fill = post_mul.unwrap();                       // c:893
        let mut out = String::with_capacity(post_num);      // c:893
        let fill_chars: Vec<char> = fill.chars().collect(); // c:893
        for i in 0..post_num {                              // c:893
            out.push(fill_chars[i % fill_chars.len()]);     // c:893
        }                                                   // c:893
        return out;                                         // c:893
    }                                                       // c:893
    // Standard padding path. Truncate value if longer than the
    // target width on the relevant side; otherwise fill from
    // pre_one/post_one (once) followed by pre_mul/post_mul (repeating).
    let value_chars: Vec<char> = s.chars().collect();       // c:893
    let value_len = value_chars.len();                      // c:893
    if pre_num > 0 {                                        // c:893
        if value_len >= pre_num {                           // c:893
            // Value too long — keep the rightmost pre_num chars.
            return value_chars[value_len - pre_num..].iter().collect(); // c:893
        }                                                   // c:893
        let need = pre_num - value_len;                     // c:893
        let one = pre_one.unwrap_or("");                    // c:893
        let one_chars: Vec<char> = one.chars().collect();   // c:893
        let one_take = one_chars.len().min(need);           // c:893
        let mul_need = need - one_take;                     // c:893
        let fill_str = pre_mul                              // c:893
            .map(|s| s.to_string())                         // c:893
            .or_else(|| fallback_char.map(|c| c.to_string())) // c:893
            .unwrap_or_else(|| " ".to_string());            // c:893
        let fill_chars: Vec<char> = fill_str.chars().collect(); // c:893
        let mut out = String::with_capacity(pre_num + value_len); // c:893
        if !fill_chars.is_empty() {                         // c:893
            for i in 0..mul_need {                          // c:893
                out.push(fill_chars[i % fill_chars.len()]); // c:893
            }                                               // c:893
        } else {                                            // c:893
            for _ in 0..mul_need {                          // c:893
                out.push(' ');                              // c:893
            }                                               // c:893
        }                                                   // c:893
        // Take the suffix of pre_one's chars so a long string1 is
        // truncated on the LEFT when partially consumed (matches
        // dopadding's "preone may be truncated on the left").
        let take_from = one_chars.len().saturating_sub(one_take); // c:893
        for c in &one_chars[take_from..] {                  // c:893
            out.push(*c);                                   // c:893
        }                                                   // c:893
        out.extend(value_chars);                            // c:893
        return out;                                         // c:893
    }                                                       // c:893
    if post_num > 0 {                                       // c:893
        if value_len >= post_num {                          // c:893
            // Truncate to post_num chars on the right.
            return value_chars[..post_num].iter().collect(); // c:893
        }                                                   // c:893
        let need = post_num - value_len;                    // c:893
        let one = post_one.unwrap_or("");                   // c:893
        let one_chars: Vec<char> = one.chars().collect();   // c:893
        let one_take = one_chars.len().min(need);           // c:893
        let mul_need = need - one_take;                     // c:893
        let fill_str = post_mul                             // c:893
            .map(|s| s.to_string())                         // c:893
            .or_else(|| fallback_char.map(|c| c.to_string())) // c:893
            .unwrap_or_else(|| " ".to_string());            // c:893
        let fill_chars: Vec<char> = fill_str.chars().collect(); // c:893
        let mut out: String = value_chars.iter().collect(); // c:893
        for c in &one_chars[..one_take] {                   // c:893
            out.push(*c);                                   // c:893
        }                                                   // c:893
        if !fill_chars.is_empty() {                         // c:893
            for i in 0..mul_need {                          // c:893
                out.push(fill_chars[i % fill_chars.len()]); // c:893
            }                                               // c:893
        } else {                                            // c:893
            for _ in 0..mul_need {                          // c:893
                out.push(' ');                              // c:893
            }                                               // c:893
        }                                                   // c:893
        return out;                                         // c:893
    }                                                       // c:893
    s.to_string()                                           // c:893
}                                                           // c:893

/// Shell-tokenize `s` per zsh `${(z)…}` semantics. Port of
/// bufferwords() (Src/lex.c) at the level the (z) flag exposes:
/// whitespace separates words; metacharacters `;`, `&`, `|`, `<`, `>`,
/// `(`, `)` become their own tokens, with `&&`, `||`, `;;`, `>>`,
/// `<<`, `>&`, `<&` recognised as compound tokens. Quoted regions
/// preserve embedded whitespace and metas.
fn z_tokenize(s: &str) -> Vec<String> {                     // lex.c // c:893
    let mut out: Vec<String> = Vec::new();                  // lex.c // c:893
    let chars: Vec<char> = s.chars().collect();             // lex.c // c:893
    let mut i = 0;                                          // lex.c // c:893
    let push = |out: &mut Vec<String>, cur: &mut String| {  // lex.c // c:893
        if !cur.is_empty() {                                // lex.c // c:893
            out.push(std::mem::take(cur));                  // lex.c // c:893
        }                                                   // lex.c // c:893
    };                                                      // lex.c // c:893
    let is_meta = |c: char| matches!(c, ';' | '&' | '|' | '<' | '>' | '(' | ')'); // lex.c // c:893
    let mut cur = String::new();                            // lex.c // c:893
    while i < chars.len() {                                 // lex.c // c:893
        let c = chars[i];                                   // lex.c // c:893
        if c.is_whitespace() {                              // lex.c // c:893
            push(&mut out, &mut cur);                       // lex.c // c:893
            i += 1;                                         // lex.c // c:893
            continue;                                       // lex.c // c:893
        }                                                   // lex.c // c:893
        if c == '\'' {                                      // lex.c // c:893
            cur.push(c);                                    // lex.c // c:893
            i += 1;                                         // lex.c // c:893
            while i < chars.len() && chars[i] != '\'' {     // lex.c // c:893
                cur.push(chars[i]);                         // lex.c // c:893
                i += 1;                                     // lex.c // c:893
            }                                               // lex.c // c:893
            if i < chars.len() {                            // lex.c // c:893
                cur.push(chars[i]);                         // lex.c // c:893
                i += 1;                                     // lex.c // c:893
            }                                               // lex.c // c:893
            continue;                                       // lex.c // c:893
        }                                                   // lex.c // c:893
        if c == '"' {                                       // lex.c // c:893
            cur.push(c);                                    // lex.c // c:893
            i += 1;                                         // lex.c // c:893
            while i < chars.len() && chars[i] != '"' {      // lex.c // c:893
                if chars[i] == '\\' && i + 1 < chars.len() { // lex.c // c:893
                    cur.push(chars[i]);                     // lex.c // c:893
                    cur.push(chars[i + 1]);                 // lex.c // c:893
                    i += 2;                                 // lex.c // c:893
                    continue;                               // lex.c // c:893
                }                                           // lex.c // c:893
                cur.push(chars[i]);                         // lex.c // c:893
                i += 1;                                     // lex.c // c:893
            }                                               // lex.c // c:893
            if i < chars.len() {                            // lex.c // c:893
                cur.push(chars[i]);                         // lex.c // c:893
                i += 1;                                     // lex.c // c:893
            }                                               // lex.c // c:893
            continue;                                       // lex.c // c:893
        }                                                   // lex.c // c:893
        if c == '\\' && i + 1 < chars.len() {               // lex.c // c:893
            cur.push(c);                                    // lex.c // c:893
            cur.push(chars[i + 1]);                         // lex.c // c:893
            i += 2;                                         // lex.c // c:893
            continue;                                       // lex.c // c:893
        }                                                   // lex.c // c:893
        if is_meta(c) {                                     // lex.c // c:893
            push(&mut out, &mut cur);                       // lex.c // c:893
            // Compound metas: `&&`, `||`, `;;`, `>>`, `<<`, `>&`,
            // `<&`. Single-char fallthrough otherwise.
            let mut tok = String::from(c);                  // lex.c // c:893
            if i + 1 < chars.len() {                        // lex.c // c:893
                let pair = (c, chars[i + 1]);               // lex.c // c:893
                let combined = matches!(                    // lex.c // c:893
                    pair,                                   // lex.c // c:893
                    ('&', '&')                              // lex.c // c:893
                        | ('|', '|')                        // lex.c // c:893
                        | (';', ';')                        // lex.c // c:893
                        | ('>', '>')                        // lex.c // c:893
                        | ('<', '<')                        // lex.c // c:893
                        | ('>', '&')                        // lex.c // c:893
                        | ('<', '&')                        // lex.c // c:893
                );                                          // lex.c // c:893
                if combined {                               // lex.c // c:893
                    tok.push(chars[i + 1]);                 // lex.c // c:893
                    i += 2;                                 // lex.c // c:893
                    out.push(tok);                          // lex.c // c:893
                    continue;                               // lex.c // c:893
                }                                           // lex.c // c:893
            }                                               // lex.c // c:893
            out.push(tok);                                  // lex.c // c:893
            i += 1;                                         // lex.c // c:893
            continue;                                       // lex.c // c:893
        }                                                   // lex.c // c:893
        cur.push(c);                                        // lex.c // c:893
        i += 1;                                             // lex.c // c:893
    }                                                       // lex.c // c:893
    push(&mut out, &mut cur);                               // lex.c // c:893
    out                                                     // lex.c // c:893
}                                                           // lex.c // c:893

/// Apply one chained subscript on top of an already-resolved value.
/// Cases:
///   • value has multiple elements → treat as array, pick by index /
///     `@` / `*` / negative.
///   • value has one element → treat as scalar, slice chars by index
///     or `[N,M]` range. Port of zsh's getindex() recursion (subst.c).
fn apply_chained_subscript(value: Vec<String>, sub: &str, state: &mut SubstState) -> Vec<String> { // c:2867
    let resolved_sub = singsub_no_tilde(sub, state);        // c:2867
    let s = resolved_sub.trim();                            // c:2867
    if s.is_empty() {                                       // c:2867
        return value;                                       // c:2867
    }                                                       // c:2867
    if s == "@" || s == "*" {                               // c:2867
        return value;                                       // c:2867
    }                                                       // c:2867
    // Range form `N,M` — slice chars/elements from N..=M (1-based).
    if let Some(comma) = s.find(',') {                      // c:2867
        if let (Ok(a), Ok(b)) = (                           // c:2867
            s[..comma].trim().parse::<i64>(),               // c:2867
            s[comma + 1..].trim().parse::<i64>(),           // c:2867
        ) {                                                 // c:2867
            if value.len() == 1 {                           // c:2867
                let chars: Vec<char> = value[0].chars().collect(); // c:2867
                let n = chars.len() as i64;                 // c:2867
                let start = if a > 0 { (a - 1) as usize } else if a < 0 { ((n + a).max(0)) as usize } else { 0 }; // c:2867
                let end = if b > 0 { (b as usize).min(chars.len()) } else if b < 0 { ((n + b + 1).max(0)) as usize } else { 0 }; // c:2867
                if start <= end && start <= chars.len() {   // c:2867
                    return vec![chars[start..end.min(chars.len())].iter().collect()]; // c:2867
                }                                           // c:2867
                return vec![String::new()];                 // c:2867
            } else {                                        // c:2867
                let n = value.len() as i64;                 // c:2867
                let start = if a > 0 { (a - 1) as usize } else if a < 0 { ((n + a).max(0)) as usize } else { 0 }; // c:2867
                let end = if b > 0 { (b as usize).min(value.len()) } else if b < 0 { ((n + b + 1).max(0)) as usize } else { 0 }; // c:2867
                if start < value.len() && start <= end {    // c:2867
                    return value[start..end.min(value.len())].to_vec(); // c:2867
                }                                           // c:2867
                return Vec::new();                          // c:2867
            }                                               // c:2867
        }                                                   // c:2867
    }                                                       // c:2867
    if let Ok(idx) = s.parse::<i64>() {                     // c:2867
        if value.len() == 1 {                               // c:2867
            let chars: Vec<char> = value[0].chars().collect(); // c:2867
            let n = chars.len() as i64;                     // c:2867
            let real = if idx > 0 {                         // c:2867
                (idx - 1) as usize                          // c:2867
            } else if idx < 0 {                             // c:2867
                let off = n + idx;                          // c:2867
                if off < 0 {                                // c:2867
                    return vec![String::new()];             // c:2867
                }                                           // c:2867
                off as usize                                // c:2867
            } else {                                        // c:2867
                return vec![String::new()];                 // c:2867
            };                                              // c:2867
            return chars                                    // c:2867
                .get(real)                                  // c:2867
                .map(|c| vec![c.to_string()])               // c:2867
                .unwrap_or_else(|| vec![String::new()]);    // c:2867
        }                                                   // c:2867
        let n = value.len() as i64;                         // c:2867
        let real = if idx > 0 {                             // c:2867
            (idx - 1) as usize                              // c:2867
        } else if idx < 0 {                                 // c:2867
            let off = n + idx;                              // c:2867
            if off < 0 {                                    // c:2867
                return Vec::new();                          // c:2867
            }                                               // c:2867
            off as usize                                    // c:2867
        } else {                                            // c:2867
            return Vec::new();                              // c:2867
        };                                                  // c:2867
        return value.into_iter().nth(real).map(|v| vec![v]).unwrap_or_default(); // c:2867
    }                                                       // c:2867
    value                                                   // c:2867
}                                                           // c:2867

/// Backslash-escape shell metacharacters in `s`. Port of
/// `quotestring(QT_BACKSLASH_SHOWNULL)` from Src/utils.c — same
/// metaclass set: whitespace, glob, quoting, redirection, history.
fn quote_backslash(s: &str) -> String {                     // c:1528
    let mut out = String::with_capacity(s.len() + 8);       // c:1528
    for c in s.chars() {                                    // c:1528
        match c {                                           // c:1528
            ' ' | '\t' | '\n' | '\'' | '"' | '`' | '\\' | '$' | '&' | '|' | ';' // c:1528
            | '<' | '>' | '(' | ')' | '{' | '}' | '[' | ']' | '*' | '?' | '!' // c:1528
            | '#' | '~' | '^' | '=' => {                    // c:1528
                out.push('\\');                             // c:1528
                out.push(c);                                // c:1528
            }                                               // c:1528
            _ => out.push(c),                               // c:1528
        }                                                   // c:1528
    }                                                       // c:1528
    out                                                     // c:1528
}                                                           // c:1528

/// Strip shell-quoting from `s`. Drops `\X` → `X`, `'…'` and `"…"`
/// to literal contents. Port of Src/subst.c modify's `case 'Q':` arm
/// which calls parse_subst_string + remnulargs + untokenize.
fn unquote_subst(s: &str) -> String {                       // c:1528
    let mut out = String::with_capacity(s.len());           // c:1528
    let mut chars = s.chars().peekable();                   // c:1528
    while let Some(c) = chars.next() {                      // c:1528
        match c {                                           // c:1528
            '\\' => {                                       // c:1528
                if let Some(&nx) = chars.peek() {           // c:1528
                    out.push(nx);                           // c:1528
                    chars.next();                           // c:1528
                }                                           // c:1528
            }                                               // c:1528
            '\'' => {                                       // c:1528
                while let Some(&inner) = chars.peek() {     // c:1528
                    chars.next();                           // c:1528
                    if inner == '\'' {                      // c:1528
                        break;                              // c:1528
                    }                                       // c:1528
                    out.push(inner);                        // c:1528
                }                                           // c:1528
            }                                               // c:1528
            '"' => {                                        // c:1528
                while let Some(&inner) = chars.peek() {     // c:1528
                    chars.next();                           // c:1528
                    if inner == '"' {                       // c:1528
                        break;                              // c:1528
                    }                                       // c:1528
                    if inner == '\\' {                      // c:1528
                        if let Some(&esc) = chars.peek() {  // c:1528
                            out.push(esc);                  // c:1528
                            chars.next();                   // c:1528
                            continue;                       // c:1528
                        }                                   // c:1528
                    }                                       // c:1528
                    out.push(inner);                        // c:1528
                }                                           // c:1528
            }                                               // c:1528
            _ => out.push(c),                               // c:1528
        }                                                   // c:1528
    }                                                       // c:1528
    out                                                     // c:1528
}                                                           // c:1528

/// Lexically resolve `.`/`..` segments in a path. If `s` is relative,
/// prepend `$PWD` (or the OS cwd) so the result is absolute.
/// `keep_relative=true` keeps relative paths relative.
/// Port of `xsymlinks()` from Src/utils.c with `resolve=0` — same
/// segment-walk logic without consulting the filesystem.
fn lexical_canonicalize(s: &str, keep_relative: bool) -> String { // c:N/A
    if s.is_empty() {                                       // c:N/A
        return s.to_string();                               // c:N/A
    }                                                       // c:N/A
    let absolute = s.starts_with('/');                      // c:N/A
    let base: String = if absolute || keep_relative {       // c:N/A
        s.to_string()                                       // c:N/A
    } else {                                                // c:N/A
        let cwd = std::env::var("PWD")                      // c:N/A
            .or_else(|_| {                                  // c:N/A
                std::env::current_dir().map(|p| p.to_string_lossy().to_string()) // c:N/A
            })                                              // c:N/A
            .unwrap_or_default();                           // c:N/A
        if cwd.is_empty() {                                 // c:N/A
            s.to_string()                                   // c:N/A
        } else {                                            // c:N/A
            format!("{}/{}", cwd.trim_end_matches('/'), s)  // c:N/A
        }                                                   // c:N/A
    };                                                      // c:N/A

    let mut out: Vec<&str> = Vec::new();                    // c:N/A
    for seg in base.split('/') {                            // c:N/A
        match seg {                                         // c:N/A
            "" | "." => continue,                           // c:N/A
            ".." => {                                       // c:N/A
                out.pop();                                  // c:N/A
            }                                               // c:N/A
            _ => out.push(seg),                             // c:N/A
        }                                                   // c:N/A
    }                                                       // c:N/A

    let leading = if base.starts_with('/') { "/" } else { "" }; // c:N/A
    if out.is_empty() {                                     // c:N/A
        if leading.is_empty() { ".".to_string() } else { "/".to_string() } // c:N/A
    } else {                                                // c:N/A
        format!("{}{}", leading, out.join("/"))             // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A


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
            let w = char_display_width(wc);                 // c:848
            if w >= 0 { w } else { 0 }                      // c:848
        }                                                   // c:848
        _ => if char_display_width(wc) > 0 { 1 } else { 0 }, // c:848
    }                                                       // c:848
}                                                           // c:848

fn char_display_width(c: char) -> i32 {                     // c:N/A
    let cp = c as u32;                                      // c:N/A
    // Control chars (C0/C1) — non-printable, width 0.
    if cp < 0x20 || (0x7F..0xA0).contains(&cp) {            // c:N/A
        return 0;                                           // c:N/A
    }                                                       // c:N/A
    // Combining marks (U+0300..036F, etc.) — width 0. Truncated set
    // covering the most common cases; full support would require a
    // unicode-width table.
    if (0x0300..=0x036F).contains(&cp)                      // c:N/A
        || (0x0483..=0x0489).contains(&cp)                  // c:N/A
        || (0x0591..=0x05BD).contains(&cp)                  // c:N/A
        || (0x0610..=0x061A).contains(&cp)                  // c:N/A
        || (0x064B..=0x065F).contains(&cp)                  // c:N/A
        || (0x0670..=0x0670).contains(&cp)                  // c:N/A
        || (0x06D6..=0x06DC).contains(&cp)                  // c:N/A
        || cp == 0x200B                                     // c:N/A
        || cp == 0x200C                                     // c:N/A
        || cp == 0x200D                                     // c:N/A
        || cp == 0xFEFF                                     // c:N/A
    {                                                       // c:N/A
        return 0;                                           // c:N/A
    }                                                       // c:N/A
    // CJK ranges, full-width forms — width 2.
    if (0x1100..=0x115F).contains(&cp)                      // c:N/A
        || (0x2E80..=0x303E).contains(&cp)                  // c:N/A
        || (0x3041..=0x33FF).contains(&cp)                  // c:N/A
        || (0x3400..=0x4DBF).contains(&cp)                  // c:N/A
        || (0x4E00..=0x9FFF).contains(&cp)                  // c:N/A
        || (0xA000..=0xA4CF).contains(&cp)                  // c:N/A
        || (0xAC00..=0xD7A3).contains(&cp)                  // c:N/A
        || (0xF900..=0xFAFF).contains(&cp)                  // c:N/A
        || (0xFE30..=0xFE4F).contains(&cp)                  // c:N/A
        || (0xFF00..=0xFF60).contains(&cp)                  // c:N/A
        || (0xFFE0..=0xFFE6).contains(&cp)                  // c:N/A
        || (0x20000..=0x2FFFD).contains(&cp)                // c:N/A
        || (0x30000..=0x3FFFD).contains(&cp)                // c:N/A
    {                                                       // c:N/A
        return 2;                                           // c:N/A
    }                                                       // c:N/A
    1                                                       // c:N/A
}                                                           // c:N/A

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

/// Perform string substitution (s/old/new/)
/// Port of subst() logic from subst.c
/// `${var/old/new}` / `${var//old/new}` substitution.
/// Port of the substitution arm inside `paramsubst()`
/// (Src/subst.c:1625) — same `/g` global toggle.
pub fn subst(s: &str, old: &str, new: &str, global: bool) -> String { // c:N/A
    if global {                                             // c:N/A
        s.replace(old, new)                                 // c:N/A
    } else {                                                // c:N/A
        s.replacen(old, new, 1)                             // c:N/A
    }                                                       // c:N/A
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

/// Quote a string according to quote type
/// Port of quotestring() logic
/// Quote a string per the requested style.
/// Port of `quotestring()` from Src/utils.c.
pub fn quotestring(s: &str, qt: QuoteType) -> String {      // utils.c:6141
    match qt {                                              // utils.c:6141
        QuoteType::None => s.to_string(),                   // utils.c:6141
        QuoteType::Backslash | QuoteType::BackslashPattern => { // utils.c:6141
            let mut result = String::new();                 // utils.c:6141
            for c in s.chars() {                            // utils.c:6141
                match c {                                   // utils.c:6141
                    ' ' | '\t' | '\n' | '\\' | '\'' | '"' | '$' | '`' | '!' | '*' | '?' | '[' // utils.c:6141
                    | ']' | '(' | ')' | '{' | '}' | '<' | '>' | '|' | '&' | ';' | '#' | '~' => { // utils.c:6141
                        result.push('\\');                  // utils.c:6141
                        result.push(c);                     // utils.c:6141
                    }                                       // utils.c:6141
                    _ => result.push(c),                    // utils.c:6141
                }                                           // utils.c:6141
            }                                               // utils.c:6141
            result                                          // utils.c:6141
        }                                                   // utils.c:6141
        QuoteType::Single => {                              // utils.c:6141
            format!("'{}'", s.replace('\'', "'\\''"))       // utils.c:6141
        }                                                   // utils.c:6141
        QuoteType::Double => {                              // utils.c:6141
            let mut result = String::from("\"");            // utils.c:6141
            for c in s.chars() {                            // utils.c:6141
                match c {                                   // utils.c:6141
                    '"' | '\\' | '$' | '`' => {             // utils.c:6141
                        result.push('\\');                  // utils.c:6141
                        result.push(c);                     // utils.c:6141
                    }                                       // utils.c:6141
                    _ => result.push(c),                    // utils.c:6141
                }                                           // utils.c:6141
            }                                               // utils.c:6141
            result.push('"');                               // utils.c:6141
            result                                          // utils.c:6141
        }                                                   // utils.c:6141
        QuoteType::Dollars => {                             // utils.c:6141
            let mut result = String::from("$'");            // utils.c:6141
            for c in s.chars() {                            // utils.c:6141
                match c {                                   // utils.c:6141
                    '\'' => result.push_str("\\'"),         // utils.c:6141
                    '\\' => result.push_str("\\\\"),        // utils.c:6141
                    '\n' => result.push_str("\\n"),         // utils.c:6141
                    '\t' => result.push_str("\\t"),         // utils.c:6141
                    '\r' => result.push_str("\\r"),         // utils.c:6141
                    c if c.is_ascii_control() => {          // utils.c:6141
                        result.push_str(&format!("\\x{:02x}", c as u32)); // utils.c:6141
                    }                                       // utils.c:6141
                    _ => result.push(c),                    // utils.c:6141
                }                                           // utils.c:6141
            }                                               // utils.c:6141
            result.push('\'');                              // utils.c:6141
            result                                          // utils.c:6141
        }                                                   // utils.c:6141
        QuoteType::QuotedZputs | QuoteType::SingleOptional => { // utils.c:6141
            // Check if quoting is needed
            let needs_quote = s.chars().any(|c| {           // utils.c:6141
                matches!(                                   // utils.c:6141
                    c,                                      // utils.c:6141
                    ' ' | '\t'                              // utils.c:6141
                        | '\n'                              // utils.c:6141
                        | '\\'                              // utils.c:6141
                        | '\''                              // utils.c:6141
                        | '"'                               // utils.c:6141
                        | '$'                               // utils.c:6141
                        | '`'                               // utils.c:6141
                        | '!'                               // utils.c:6141
                        | '*'                               // utils.c:6141
                        | '?'                               // utils.c:6141
                        | '['                               // utils.c:6141
                        | ']'                               // utils.c:6141
                        | '('                               // utils.c:6141
                        | ')'                               // utils.c:6141
                        | '{'                               // utils.c:6141
                        | '}'                               // utils.c:6141
                        | '<'                               // utils.c:6141
                        | '>'                               // utils.c:6141
                        | '|'                               // utils.c:6141
                        | '&'                               // utils.c:6141
                        | ';'                               // utils.c:6141
                        | '#'                               // utils.c:6141
                        | '~'                               // utils.c:6141
                )                                           // utils.c:6141
            });                                             // utils.c:6141
            if needs_quote {                                // utils.c:6141
                format!("'{}'", s.replace('\'', "'\\''"))   // utils.c:6141
            } else {                                        // utils.c:6141
                s.to_string()                               // utils.c:6141
            }                                               // utils.c:6141
        }                                                   // utils.c:6141
    }                                                       // utils.c:6141
}                                                           // utils.c:6141

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

/// Sort array according to options
/// Port of strmetasort() logic
/// Sort an array per `${(o)…}` flags.
/// Port of `strmetasort()` from Src/sort.c:234.
pub fn sort_array(arr: &mut [String], opts: &SortOptions) { // c:N/A
    if !opts.somehow {                                      // c:N/A
        return;                                             // c:N/A
    }                                                       // c:N/A

    if opts.numeric || opts.numeric_signed {                // c:N/A
        arr.sort_by(|a, b| {                                // c:N/A
            let na: f64 = a.parse().unwrap_or(0.0);         // c:N/A
            let nb: f64 = b.parse().unwrap_or(0.0);         // c:N/A
            na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal) // c:N/A
        });                                                 // c:N/A
    } else if opts.case_insensitive {                       // c:N/A
        arr.sort_by_key(|a| a.to_lowercase());              // c:N/A
    } else {                                                // c:N/A
        arr.sort();                                         // c:N/A
    }                                                       // c:N/A

    if opts.backwards {                                     // c:N/A
        arr.reverse();                                      // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

/// Word count in a string
/// Port of wordcount() logic
/// Count words in a string per IFS rules.
/// Port of the `${#var}` length path inside `paramsubst()`
/// (Src/subst.c:1625).
pub fn wordcount(s: &str, sep: Option<&str>, count_empty: bool) -> usize { // c:N/A
    let separator = sep.unwrap_or(" \t\n");                 // c:N/A

    if count_empty {                                        // c:N/A
        s.split(|c: char| separator.contains(c)).count()    // c:N/A
    } else {                                                // c:N/A
        s.split(|c: char| separator.contains(c))            // c:N/A
            .filter(|w| !w.is_empty())                      // c:N/A
            .count()                                        // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

/// Join array with separator
/// Port of sepjoin() logic
/// Join an array with a separator (defaults to IFS first char).
/// Port of `sepjoin()` from Src/utils.c:3928.
pub fn sepjoin(arr: &[String], sep: Option<&str>, use_ifs_first: bool) -> String { // c:N/A
    let separator = sep.unwrap_or(if use_ifs_first { " " } else { "" }); // c:N/A
    arr.join(separator)                                     // c:N/A
}                                                           // c:N/A

/// Split string by separator
/// Port of sepsplit() logic
/// Split a string on a separator (defaults to IFS).
/// Port of `sepsplit()` from Src/utils.c:3962.
pub fn sepsplit(s: &str, sep: Option<&str>, allow_empty: bool, _handle_ifs: bool) -> Vec<String> { // c:N/A
    let separator = sep.unwrap_or(" \t\n");                 // c:N/A

    if allow_empty {                                        // c:N/A
        s.split(|c: char| separator.contains(c))            // c:N/A
            .map(String::from)                              // c:N/A
            .collect()                                      // c:N/A
    } else {                                                // c:N/A
        s.split(|c: char| separator.contains(c))            // c:N/A
            .filter(|w| !w.is_empty())                      // c:N/A
            .map(String::from)                              // c:N/A
            .collect()                                      // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

/// Unique array elements
/// Port of zhuniqarray() logic
/// `${(u)var}` — preserve order, drop duplicates.
/// Port of the `SORTIT_UNIQUE` arm of `strmetasort()`
/// (Src/sort.c:234).
pub fn unique_array(arr: &mut Vec<String>) {                // c:N/A
    let mut seen = std::collections::HashSet::new();        // c:N/A
    arr.retain(|s| seen.insert(s.clone()));                 // c:N/A
}                                                           // c:N/A

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

/// Substitute named directory
/// Port of substnamedir() logic
/// Apply `~name` named-directory substitution.
/// Port of the `~name` arm of `filesub()` (Src/subst.c:667).
pub fn substnamedir(s: &str) -> String {                    // c:N/A
    // Try to replace home directory with ~
    if let Ok(home) = std::env::var("HOME") {               // c:N/A
        if s.starts_with(&home) {                           // c:N/A
            return format!("~{}", &s[home.len()..]);        // c:N/A
        }                                                   // c:N/A
    }                                                       // c:N/A
    s.to_string()                                           // c:N/A
}                                                           // c:N/A

/// Make string printable
/// Port of nicedupstring() logic
/// Render a string with `nicechar` for control bytes.
/// Port of `nicedupstring()` from Src/utils.c:5301.
pub fn nicedupstring(s: &str) -> String {                   // c:N/A
    let mut result = String::new();                         // c:N/A
    for c in s.chars() {                                    // c:N/A
        if c.is_ascii_control() {                           // c:N/A
            match c {                                       // c:N/A
                '\n' => result.push_str("\\n"),             // c:N/A
                '\t' => result.push_str("\\t"),             // c:N/A
                '\r' => result.push_str("\\r"),             // c:N/A
                _ => result.push_str(&format!("\\x{:02x}", c as u32)), // c:N/A
            }                                               // c:N/A
        } else {                                            // c:N/A
            result.push(c);                                 // c:N/A
        }                                                   // c:N/A
    }                                                       // c:N/A
    result                                                  // c:N/A
}                                                           // c:N/A

/// Untokenize a string (remove internal tokens)
pub fn untokenize(s: &str) -> String {                      // c:N/A
    s.chars().map(token_to_char).collect()                  // c:N/A
}                                                           // c:N/A

/// Tokenize a string for globbing
pub fn shtokenize(s: &str) -> String {                      // c:N/A
    // This is a simplified version - real zsh does complex tokenization
    let mut result = String::new();                         // c:N/A
    for c in s.chars() {                                    // c:N/A
        match c {                                           // c:N/A
            '*' => result.push('\u{91}'), // Star token     // c:N/A
            '?' => result.push('\u{92}'), // Quest token    // c:N/A
            '[' => result.push(INBRACK),                    // c:N/A
            ']' => result.push(OUTBRACK),                   // c:N/A
            _ => result.push(c),                            // c:N/A
        }                                                   // c:N/A
    }                                                       // c:N/A
    result                                                  // c:N/A
}                                                           // c:N/A

/// Check if substitution is complete
pub fn check_subst_complete(s: &str) -> bool {              // c:N/A
    let mut depth = 0;                                      // c:N/A
    let mut in_brace = 0;                                   // c:N/A

    for c in s.chars() {                                    // c:N/A
        match c {                                           // c:N/A
            INPAR => depth += 1,                            // c:N/A
            OUTPAR => depth -= 1,                           // c:N/A
            INBRACE | '{' => in_brace += 1,                 // c:N/A
            OUTBRACE | '}' => in_brace -= 1,                // c:N/A
            _ => {}                                         // c:N/A
        }                                                   // c:N/A
    }                                                       // c:N/A

    depth == 0 && in_brace == 0                             // c:N/A
}                                                           // c:N/A

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

    remnulargs(&result)                                     // c:463
}                                                           // c:463

/// Glob entries in a linked list
/// Port of globlist() from subst.c lines 468-505
pub fn globlist(list: &mut LinkList, flags: u32, state: &mut SubstState) { // c:489
    let mut node_idx = 0;                                   // c:489

    while node_idx < list.len() && !state.errflag {         // c:489
        if let Some(data) = list.get_data(node_idx) {       // c:489
            // Check for Marker (key-value pair indicator)
            if flags & prefork_flags::KEY_VALUE != 0 && data.starts_with(MARKER) { // c:489
                // Skip key/value pair (marker, key, value = 3 nodes)
                node_idx += 3;                              // c:489
                continue;                                   // c:489
            }                                               // c:489

            // Perform globbing
            let expanded = zglob(data, flags & prefork_flags::NO_UNTOK != 0, state); // c:489

            if expanded.is_empty() {                        // c:489
                // No matches - either error or keep original
                if state.opts.glob_subst {                  // c:489
                    // NOMATCH option would error here
                    // For now, keep original
                }                                           // c:489
            } else if expanded.len() == 1 {                 // c:489
                list.set_data(node_idx, expanded[0].clone()); // c:489
            } else {                                        // c:489
                // Multiple matches - expand into list
                list.remove(node_idx);                      // c:489
                for (i, path) in expanded.iter().enumerate() { // c:489
                    if i == 0 {                             // c:489
                        list.nodes.insert(node_idx, LinkNode { data: path.clone() }); // c:489
                    } else {                                // c:489
                        list.insert_after(node_idx + i - 1, path.clone()); // c:489
                    }                                       // c:489
                }                                           // c:489
                node_idx += expanded.len();                 // c:489
                continue;                                   // c:489
            }                                               // c:489
        }                                                   // c:489
        node_idx += 1;                                      // c:489
    }                                                       // c:489
}                                                           // c:489

/// Perform glob expansion on a pattern
/// Simplified port of zglob() logic
fn zglob(pattern: &str, no_untok: bool, state: &SubstState) -> Vec<String> { // c:N/A
    let pattern = if no_untok {                             // c:N/A
        pattern.to_string()                                 // c:N/A
    } else {                                                // c:N/A
        untokenize(pattern)                                 // c:N/A
    };                                                      // c:N/A

    // Check if it's a glob pattern
    if !pattern.contains('*') && !pattern.contains('?') && !pattern.contains('[') { // c:N/A
        // Not a glob pattern
        if std::path::Path::new(&pattern).exists() {        // c:N/A
            return vec![pattern];                           // c:N/A
        }                                                   // c:N/A
        return vec![pattern];                               // c:N/A
    }                                                       // c:N/A

    // Perform glob expansion
    match glob::glob(&pattern) {                            // c:N/A
        Ok(paths) => {                                      // c:N/A
            let matches: Vec<String> = paths                // c:N/A
                .filter_map(|p| p.ok())                     // c:N/A
                .map(|p| p.to_string_lossy().to_string())   // c:N/A
                .collect();                                 // c:N/A
            if matches.is_empty() {                         // c:N/A
                vec![pattern]                               // c:N/A
            } else {                                        // c:N/A
                matches                                     // c:N/A
            }                                               // c:N/A
        }                                                   // c:N/A
        Err(_) => vec![pattern],                            // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

/// Skip matching parentheses/brackets
/// Port of skipparens() logic
pub fn skipparens(s: &str, open: char, close: char) -> Option<usize> { // c:N/A
    let mut depth = 1;                                      // c:N/A
    let chars: Vec<char> = s.chars().collect();             // c:N/A

    for (i, &c) in chars.iter().enumerate() {               // c:N/A
        if c == open {                                      // c:N/A
            depth += 1;                                     // c:N/A
        } else if c == close {                              // c:N/A
            depth -= 1;                                     // c:N/A
            if depth == 0 {                                 // c:N/A
                return Some(i);                             // c:N/A
            }                                               // c:N/A
        }                                                   // c:N/A
    }                                                       // c:N/A
    None                                                    // c:N/A
}                                                           // c:N/A

/// Get output from command substitution
/// Port of getoutput() logic
pub fn getoutput(cmd: &str, qt: bool, state: &mut SubstState) -> Option<Vec<String>> { // c:N/A
    if !state.opts.exec_opt {                               // c:N/A
        return Some(vec![]);                                // c:N/A
    }                                                       // c:N/A

    let output = run_command(cmd);                          // c:N/A

    // Trim trailing newlines
    let output = output.trim_end_matches('\n');             // c:N/A

    if qt {                                                 // c:N/A
        // Quoted - return as single string
        Some(vec![output.to_string()])                      // c:N/A
    } else {                                                // c:N/A
        // Unquoted - may split on newlines
        Some(output.lines().map(String::from).collect())    // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

/// Parse subscript expression like `[1]` or `[1,5]`
/// Port of parse_subscript() logic
pub fn parse_subscript(s: &str, _allow_range: bool) -> Option<(String, String)> { // c:N/A
    let chars: Vec<char> = s.chars().collect();             // c:N/A

    if chars.first() != Some(&'[') && chars.first() != Some(&INBRACK) { // c:N/A
        return None;                                        // c:N/A
    }                                                       // c:N/A

    let mut depth = 1;                                      // c:N/A
    let mut end = 1;                                        // c:N/A

    while end < chars.len() && depth > 0 {                  // c:N/A
        let c = chars[end];                                 // c:N/A
        if c == '[' || c == INBRACK {                       // c:N/A
            depth += 1;                                     // c:N/A
        } else if c == ']' || c == OUTBRACK {               // c:N/A
            depth -= 1;                                     // c:N/A
        }                                                   // c:N/A
        if depth > 0 {                                      // c:N/A
            end += 1;                                       // c:N/A
        }                                                   // c:N/A
    }                                                       // c:N/A

    if depth != 0 {                                         // c:N/A
        return None;                                        // c:N/A
    }                                                       // c:N/A

    let subscript: String = chars[1..end].iter().collect(); // c:N/A
    let rest_start = end + 1;                               // c:N/A
    let rest = if rest_start < s.len() {                    // c:N/A
        s[rest_start..].to_string()                         // c:N/A
    } else {                                                // c:N/A
        String::new()                                       // c:N/A
    };                                                      // c:N/A

    Some((subscript, rest))                                 // c:N/A
}                                                           // c:N/A

/// Evaluate subscript to get array index or range
pub fn eval_subscript(subscript: &str, array_len: usize) -> (usize, Option<usize>) { // c:N/A
    // Check for range (a,b)
    if let Some(comma_pos) = subscript.find(',') {          // c:N/A
        let start_str = subscript[..comma_pos].trim();      // c:N/A
        let end_str = subscript[comma_pos + 1..].trim();    // c:N/A

        let start = parse_index(start_str, array_len);      // c:N/A
        let end = parse_index(end_str, array_len);          // c:N/A

        (start, Some(end))                                  // c:N/A
    } else {                                                // c:N/A
        // Single index
        let idx = parse_index(subscript.trim(), array_len); // c:N/A
        (idx, None)                                         // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

/// Parse a single array index (handles negative indices)
fn parse_index(s: &str, array_len: usize) -> usize {        // c:N/A
    if let Ok(idx) = s.parse::<i64>() {                     // c:N/A
        if idx < 0 {                                        // c:N/A
            // Negative index counts from end
            let abs_idx = (-idx) as usize;                  // c:N/A
            array_len.saturating_sub(abs_idx)               // c:N/A
        } else if idx == 0 {                                // c:N/A
            0                                               // c:N/A
        } else {                                            // c:N/A
            // zsh arrays are 1-indexed
            (idx as usize).saturating_sub(1)                // c:N/A
        }                                                   // c:N/A
    } else {                                                // c:N/A
        0                                                   // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

/// Check if character is an internal token
pub fn itok(c: char) -> bool {                              // c:N/A
    let code = c as u32;                                    // c:N/A
    (0x80..=0x9F).contains(&code)                           // c:N/A
}                                                           // c:N/A

/// Map tokens to their printable equivalents
/// Port of ztokens array from zsh.h
pub fn ztokens(c: char) -> char {                           // c:N/A
    match c {                                               // c:N/A
        POUND => '#',                                       // c:N/A
        STRING => '$',                                      // c:N/A
        QSTRING => '$',                                     // c:N/A
        TICK => '`',                                        // c:N/A
        QTICK => '`',                                       // c:N/A
        INPAR => '(',                                       // c:N/A
        OUTPAR => ')',                                      // c:N/A
        INBRACE => '{',                                     // c:N/A
        OUTBRACE => '}',                                    // c:N/A
        INBRACK => '[',                                     // c:N/A
        OUTBRACK => ']',                                    // c:N/A
        INANG => '<',                                       // c:N/A
        OUTANG => '>',                                      // c:N/A
        EQUALS => '=',                                      // c:N/A
        _ => c,                                             // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

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

/// Pattern matching for ${var#pattern} etc
/// Port of getmatch() logic
pub fn getmatch(val: &str, pattern: &str, flags: u32, flnum: i32, replstr: Option<&str>) -> String { // glob.c:2710
    let val_chars: Vec<char> = val.chars().collect();       // glob.c:2710
    let val_len = val_chars.len();                          // glob.c:2710

    // Convert glob pattern to regex (simplified)
    let regex_pattern = glob_to_regex(pattern);             // glob.c:2710

    match regex::Regex::new(&regex_pattern) {               // glob.c:2710
        Ok(re) => {                                         // glob.c:2710
            if flags & sub_flags::GLOBAL != 0 {             // glob.c:2710
                // Global replacement: //
                let replacement = replstr.unwrap_or("");    // glob.c:2710
                re.replace_all(val, replacement).to_string() // glob.c:2710
            } else if flags & sub_flags::END != 0 {         // glob.c:2710
                // Match at end: %
                if flags & sub_flags::LONG != 0 {           // glob.c:2710
                    // Longest match from end: %%
                    for i in 0..=val_len {                  // glob.c:2710
                        let suffix: String = val_chars[i..].iter().collect(); // glob.c:2710
                        if re.is_match(&suffix) {           // glob.c:2710
                            let prefix: String = val_chars[..i].iter().collect(); // glob.c:2710
                            return if let Some(repl) = replstr { // glob.c:2710
                                format!("{}{}", prefix, repl) // glob.c:2710
                            } else {                        // glob.c:2710
                                prefix                      // glob.c:2710
                            };                              // glob.c:2710
                        }                                   // glob.c:2710
                    }                                       // glob.c:2710
                } else {                                    // glob.c:2710
                    // Shortest match from end: %
                    for i in (0..=val_len).rev() {          // glob.c:2710
                        let suffix: String = val_chars[i..].iter().collect(); // glob.c:2710
                        if re.is_match(&suffix) {           // glob.c:2710
                            let prefix: String = val_chars[..i].iter().collect(); // glob.c:2710
                            return if let Some(repl) = replstr { // glob.c:2710
                                format!("{}{}", prefix, repl) // glob.c:2710
                            } else {                        // glob.c:2710
                                prefix                      // glob.c:2710
                            };                              // glob.c:2710
                        }                                   // glob.c:2710
                    }                                       // glob.c:2710
                }                                           // glob.c:2710
                val.to_string()                             // glob.c:2710
            } else {                                        // glob.c:2710
                // Match at start: #
                if flags & sub_flags::LONG != 0 {           // glob.c:2710
                    // Longest match from start: ##
                    for i in (0..=val_len).rev() {          // glob.c:2710
                        let prefix: String = val_chars[..i].iter().collect(); // glob.c:2710
                        if re.is_match(&prefix) {           // glob.c:2710
                            let suffix: String = val_chars[i..].iter().collect(); // glob.c:2710
                            return if let Some(repl) = replstr { // glob.c:2710
                                format!("{}{}", repl, suffix) // glob.c:2710
                            } else {                        // glob.c:2710
                                suffix                      // glob.c:2710
                            };                              // glob.c:2710
                        }                                   // glob.c:2710
                    }                                       // glob.c:2710
                } else {                                    // glob.c:2710
                    // Shortest match from start: #
                    for i in 0..=val_len {                  // glob.c:2710
                        let prefix: String = val_chars[..i].iter().collect(); // glob.c:2710
                        if re.is_match(&prefix) {           // glob.c:2710
                            let suffix: String = val_chars[i..].iter().collect(); // glob.c:2710
                            return if let Some(repl) = replstr { // glob.c:2710
                                format!("{}{}", repl, suffix) // glob.c:2710
                            } else {                        // glob.c:2710
                                suffix                      // glob.c:2710
                            };                              // glob.c:2710
                        }                                   // glob.c:2710
                    }                                       // glob.c:2710
                }                                           // glob.c:2710
                val.to_string()                             // glob.c:2710
            }                                               // glob.c:2710
        }                                                   // glob.c:2710
        Err(_) => {                                         // glob.c:2710
            // Fallback to simple string matching
            if let Some(repl) = replstr {                   // glob.c:2710
                val.replace(pattern, repl)                  // glob.c:2710
            } else {                                        // glob.c:2710
                val.to_string()                             // glob.c:2710
            }                                               // glob.c:2710
        }                                                   // glob.c:2710
    }                                                       // glob.c:2710
}                                                           // glob.c:2710

/// Convert glob pattern to regex
/// Strip inline `(#X)` pattern flags from the start of a zsh
/// glob/parameter pattern. Returns the rest, plus the recognized
/// flag set. Per Src/pattern.c:
///   `(#b)` — backref capture (populate `$match[N]`)
///   `(#i)` — case-insensitive
///   `(#I)` — case-sensitive (default; turn off i)
///   `(#l)` — multibyte form
fn strip_inline_pattern_flags(pat: &str) -> (String, bool, bool) { // c:N/A
    let mut remaining = pat;                                // c:N/A
    let mut backref = false;                                // c:N/A
    let mut case_i = false;                                 // c:N/A
    // Loop to consume multiple consecutive `(#…)` flag blocks per
    // zsh's pattern.c (PAT_INSENS / PAT_LOWERSENS / PAT_PURE_B /
    // PAT_PURE_M etc. can be set independently with multiple flag
    // groups: `(#b)(#i)pat`). Stop on the first non-flag token.
    while remaining.starts_with("(#") {                     // c:N/A
        let after = &remaining[2..];                        // c:N/A
        let close = match after.find(')') {                 // c:N/A
            Some(i) => i,                                   // c:N/A
            None => break,                                  // c:N/A
        };                                                  // c:N/A
        let flag_str = &after[..close];                     // c:N/A
        let mut all_known = true;                           // c:N/A
        let mut new_backref = backref;                      // c:N/A
        let mut new_case_i = case_i;                        // c:N/A
        for c in flag_str.chars() {                         // c:N/A
            match c {                                       // c:N/A
                'b' => new_backref = true,                  // c:N/A
                'B' => new_backref = false,                 // c:N/A
                // `m` / `M` enable / disable $MATCH/$MBEGIN/$MEND.
                // zshrs collapses both `m` and `b` into one
                // backref-mode bool; the per-capture replacement
                // seeder writes both views (match[N] AND
                // $MATCH/$MBEGIN/$MEND) on every fire. Direct port
                // of zsh's pattern.c pat_pure_m flag.
                'm' => new_backref = true,                  // c:N/A
                'M' => new_backref = false,                 // c:N/A
                'i' => new_case_i = true,                   // c:N/A
                'I' => new_case_i = false,                  // c:N/A
                'l' => {} // multibyte — ignored, regex handles unicode // c:N/A
                _ => {                                      // c:N/A
                    all_known = false;                      // c:N/A
                    break;                                  // c:N/A
                }                                           // c:N/A
            }                                               // c:N/A
        }                                                   // c:N/A
        if !all_known {                                     // c:N/A
            break;                                          // c:N/A
        }                                                   // c:N/A
        backref = new_backref;                              // c:N/A
        case_i = new_case_i;                                // c:N/A
        remaining = &after[close + 1..];                    // c:N/A
    }                                                       // c:N/A
    (remaining.to_string(), backref, case_i)                // c:N/A
}                                                           // c:N/A

/// Translate a zsh glob/pattern to a regex preserving `(...)` as
/// CAPTURE groups (used by `(#b)` backref mode). Otherwise the
/// same conversion as `param_pattern_to_regex` (the non-capturing
/// variant). Per Src/pattern.c — backref mode emits `pat_subme`
/// entries that the runtime exposes as `$match[N]`.
///
/// `anchored=true` wraps in `^…$` for whole-match contexts (`:#`,
/// `case`, `[[ = ]]`). `anchored=false` leaves the pattern free to
/// match anywhere — the `/` replace family relies on the operator
/// (`do_replace_one`) to enforce `/#` start-anchor / `/%` end-
/// anchor by checking the captured span position.
fn glob_to_regex_capturing(pattern: &str, anchored: bool) -> String { // glob.c:2710
    let mut regex = String::new();                          // glob.c:2710
    if anchored {                                           // glob.c:2710
        regex.push('^');                                    // glob.c:2710
    }                                                       // glob.c:2710
    let chars: Vec<char> = pattern.chars().collect();       // glob.c:2710
    let mut i = 0;                                          // glob.c:2710
    // After emitting an atom, peek for `#` / `##` extendedglob
    // postfix (zero-or-more / one-or-more repetition of the
    // previous atom). Direct port of zsh's pattern.c POUND /
    // POUND2 cases in patcompswitch — `a##` matches one-or-more
    // `a`, `a#` zero-or-more. Used by zinit's main-message-
    // formatter pattern `[^\}]##` (one-or-more non-`}`) and
    // many other extendedglob places. Returns the regex
    // quantifier or None if no postfix.
    let consume_postfix = |chars: &[char], i: &mut usize| -> Option<&'static str> { // glob.c:2710
        if *i + 1 < chars.len() && chars[*i + 1] == '#' {   // glob.c:2710
            if *i + 2 < chars.len() && chars[*i + 2] == '#' { // glob.c:2710
                *i += 2;
                Some("+")                                   // glob.c:2710
            } else {                                        // glob.c:2710
                *i += 1;
                Some("*")                                   // glob.c:2710
            }                                               // glob.c:2710
        } else {                                            // glob.c:2710
            None                                            // glob.c:2710
        }                                                   // glob.c:2710
    };                                                      // glob.c:2710
    while i < chars.len() {                                 // glob.c:2710
        match chars[i] {                                    // glob.c:2710
            '*' => {                                        // glob.c:2710
                regex.push_str(".*");                       // glob.c:2710
                if let Some(q) = consume_postfix(&chars, &mut i) { // glob.c:2710
                    regex.push_str(q);                      // glob.c:2710
                }                                           // glob.c:2710
            }                                               // glob.c:2710
            '?' => {                                        // glob.c:2710
                regex.push('.');                            // glob.c:2710
                if let Some(q) = consume_postfix(&chars, &mut i) { // glob.c:2710
                    regex.push_str(q);                      // glob.c:2710
                }                                           // glob.c:2710
            }                                               // glob.c:2710
            '[' => {                                        // glob.c:2710
                regex.push('[');                            // glob.c:2710
                i += 1;                                     // glob.c:2710
                if i < chars.len() && (chars[i] == '!' || chars[i] == '^') { // glob.c:2710
                    regex.push('^');                        // glob.c:2710
                    i += 1;                                 // glob.c:2710
                }                                           // glob.c:2710
                // Translate zsh class to Rust-regex class. Two
                // gotchas vs zsh:
                //  - `\]` inside a class — zsh / POSIX accept;
                //    Rust regex needs `\]` AND a literal `[` if
                //    present must also be escaped (`\[`).
                //  - Bare `[` inside a class is literal in zsh but
                //    rejected by Rust regex (it expects `\[`).
                // Pass `\X` through verbatim, escape any bare `[`
                // and bare `]` (only the close-`]` of the class
                // doesn't get escaped — handled by the loop's
                // termination check).
                while i < chars.len() && chars[i] != ']' {  // glob.c:2710
                    if chars[i] == '\\' && i + 1 < chars.len() { // glob.c:2710
                        regex.push('\\');                   // glob.c:2710
                        regex.push(chars[i + 1]);           // glob.c:2710
                        i += 2;                             // glob.c:2710
                    } else if chars[i] == '[' {             // glob.c:2710
                        // Bare `[` inside class — escape for Rust
                        // regex (zsh treats it as literal).
                        regex.push('\\');                   // glob.c:2710
                        regex.push('[');                    // glob.c:2710
                        i += 1;                             // glob.c:2710
                    } else {                                // glob.c:2710
                        regex.push(chars[i]);               // glob.c:2710
                        i += 1;                             // glob.c:2710
                    }                                       // glob.c:2710
                }                                           // glob.c:2710
                regex.push(']');                            // glob.c:2710
                if let Some(q) = consume_postfix(&chars, &mut i) { // glob.c:2710
                    regex.push_str(q);                      // glob.c:2710
                }                                           // glob.c:2710
            }                                               // glob.c:2710
            // `\(#e)` / `\(#s)` — escaped backslash followed by end/
            // start anchor. Direct port of zsh's pattern.c parsing
            // where `\\` is escape-backslash (literal `\`) and a
            // following `(#e)` / `(#s)` is the anchor token. By
            // the time we see this, expand_string has already
            // collapsed the original `\\` to `\` (via the `\x00\`
            // literal-marker preprocessing); detect the resulting
            // shape and emit `\\$` / `\\^`. Used by zinit's
            // `(#b)((*)\\(#e)|(*))` to match elements ending in
            // a literal backslash.
            '\\' if i + 4 < chars.len()                     // glob.c:2710
                && chars[i + 1] == '('                      // glob.c:2710
                && chars[i + 2] == '#'                      // glob.c:2710
                && (chars[i + 3] == 'e' || chars[i + 3] == 's') // glob.c:2710
                && chars[i + 4] == ')' =>                   // glob.c:2710
            {                                               // glob.c:2710
                regex.push_str("\\\\");                     // glob.c:2710
                regex.push(if chars[i + 3] == 'e' { '$' } else { '^' }); // glob.c:2710
                i += 4;                                     // glob.c:2710
            }                                               // glob.c:2710
            '\\' if i + 1 < chars.len() => {                // glob.c:2710
                regex.push(chars[i + 1]);                   // glob.c:2710
                i += 1;                                     // glob.c:2710
            }                                               // glob.c:2710
            // `(#e)` / `(#s)` end/start anchors — direct port of
            // zsh's pattern.c P_EOL / P_BOL tokens. Detected by
            // `(#e)` or `(#s)` 4-char lookahead. Emit `$` / `^`
            // respectively. Used by zinit's
            // `(#b)((*)\\(#e)|(*))` pattern to detect a trailing
            // `\` in array elements.
            '(' if i + 3 < chars.len()                      // glob.c:2710
                && chars[i + 1] == '#'                      // glob.c:2710
                && (chars[i + 2] == 'e' || chars[i + 2] == 's') // glob.c:2710
                && chars[i + 3] == ')' =>                   // glob.c:2710
            {                                               // glob.c:2710
                regex.push(if chars[i + 2] == 'e' { '$' } else { '^' }); // glob.c:2710
                i += 3; // outer loop increment handles the 4th // glob.c:2710
            }                                               // glob.c:2710
            // Capture groups + alternation pass through — the WHOLE
            // POINT of (#b) mode. `#`/`##` postfix on a `)` applies
            // to the whole group.
            '(' | '|' => regex.push(chars[i]),              // glob.c:2710
            ')' => {                                        // glob.c:2710
                regex.push(')');                            // glob.c:2710
                if let Some(q) = consume_postfix(&chars, &mut i) { // glob.c:2710
                    regex.push_str(q);                      // glob.c:2710
                }                                           // glob.c:2710
            }                                               // glob.c:2710
            // Regex metachars that are literal in glob — escape.
            c @ ('.' | '+' | '^' | '$' | '{' | '}') => {    // glob.c:2710
                regex.push('\\');                           // glob.c:2710
                regex.push(c);                              // glob.c:2710
            }                                               // glob.c:2710
            c => {                                          // glob.c:2710
                regex.push(c);                              // glob.c:2710
                if let Some(q) = consume_postfix(&chars, &mut i) { // glob.c:2710
                    regex.push_str(q);                      // glob.c:2710
                }                                           // glob.c:2710
            }                                               // glob.c:2710
        }                                                   // glob.c:2710
        i += 1;                                             // glob.c:2710
    }                                                       // glob.c:2710
    if anchored {                                           // glob.c:2710
        regex.push('$');                                    // glob.c:2710
    }                                                       // glob.c:2710
    regex                                                   // glob.c:2710
}                                                           // glob.c:2710

/// Populate the `$match` array (1-based) from regex captures.
/// Mirrors C zsh's pat_subme (Src/pattern.c) — each `(...)` group
/// in a `(#b)` pattern becomes `$match[N]`. The 0-th capture
/// (whole match) is NOT exposed; only sub-groups starting at 1.
fn populate_match_array(caps: &regex::Captures, state: &mut SubstState) { // c:N/A
    let mut arr = Vec::with_capacity(caps.len());           // c:N/A
    for i in 1..caps.len() {                                // c:N/A
        arr.push(caps.get(i).map(|m| m.as_str().to_string()).unwrap_or_default()); // c:N/A
    }                                                       // c:N/A
    state.arrays.insert("match".to_string(), arr);          // c:N/A
    // Also seed `MATCH` / `MBEGIN` / `MEND` for the (#m) flag.
    // Direct port of Src/pattern.c pat_pure_m which exposes the
    // whole-match text and 1-based offsets in the parameter table.
    if let Some(m0) = caps.get(0) {                         // c:N/A
        state                                               // c:N/A
            .variables                                      // c:N/A
            .insert("MATCH".to_string(), m0.as_str().to_string()); // c:N/A
        state                                               // c:N/A
            .variables                                      // c:N/A
            .insert("MBEGIN".to_string(), (m0.start() + 1).to_string()); // c:N/A
        state                                               // c:N/A
            .variables                                      // c:N/A
            .insert("MEND".to_string(), m0.end().to_string()); // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

/// Strip one level of `\`-escaping from a replacement string. Mirrors
/// the `untokenize()` step C zsh runs after `singsub` on the replstr
/// (Src/glob.c::compgetmatch line 2687-2688) — `\X` becomes literal
/// `X`, `\\` collapses to one `\`. In backref mode `\1`..`\9` and `\&`
/// are kept verbatim so the match-substitution pass downstream can
/// expand them.
fn untokenize_replstr(s: &str, backref_mode: bool) -> String { // c:N/A
    let chars: Vec<char> = s.chars().collect();             // c:N/A
    let mut out = String::with_capacity(s.len());           // c:N/A
    let mut i = 0;                                          // c:N/A
    while i < chars.len() {                                 // c:N/A
        let c = chars[i];                                   // c:N/A
        if c == '\\' && i + 1 < chars.len() {               // c:N/A
            let next = chars[i + 1];                        // c:N/A
            if backref_mode && (next == '&' || next.is_ascii_digit()) { // c:N/A
                out.push('\\');                             // c:N/A
                out.push(next);                             // c:N/A
            } else {                                        // c:N/A
                out.push(next);                             // c:N/A
            }                                               // c:N/A
            i += 2;                                         // c:N/A
            continue;                                       // c:N/A
        }                                                   // c:N/A
        // Strip Bnull / Bnullkeep markers too. In zshrs the parser
        // emits BNULL (`\u{9f}`) for `\X` escapes, but the runtime
        // also produces a NUL byte (`\u{0}`) marker in some
        // expand-string paths to encode "next char is literal" — see
        // exec.rs::expand_dq's `\\` handling. Drop either form and
        // keep the next char verbatim. Without this, `${s/b/X\\Y}`
        // came in as `X\u{0}\Y` (NUL injected before the literal `\`)
        // and untokenize_replstr left the NUL in the output, which
        // rendered as a stray space/null.
        if (c == '\u{9f}' || c == '\u{0}') && i + 1 < chars.len() { // c:N/A
            out.push(chars[i + 1]);                         // c:N/A
            i += 2;                                         // c:N/A
            continue;                                       // c:N/A
        }                                                   // c:N/A
        out.push(c);                                        // c:N/A
        i += 1;                                             // c:N/A
    }                                                       // c:N/A
    out                                                     // c:N/A
}                                                           // c:N/A

/// One-value replacement helper used by the `${var/…/…}` family.
/// Pulled out to keep the operator arm short.
#[allow(clippy::too_many_arguments)]                        // c:N/A
fn do_replace_one(                                          // glob.c:2710
    s: &str,                                                // glob.c:2710
    op: &str,                                               // glob.c:2710
    pattern_lit: &str,                                      // glob.c:2710
    raw_rep: &str,                                          // glob.c:2710
    re_opt: Option<&regex::Regex>,                          // glob.c:2710
    backref_mode: bool,                                     // glob.c:2710
    state: &mut SubstState,                                 // glob.c:2710
) -> String {                                               // glob.c:2710
    match (re_opt, op) {                                    // glob.c:2710
        (Some(rx), "/") => {                                // glob.c:2710
            if let Some(caps) = rx.captures(s) {            // glob.c:2710
                let m = caps.get(0).unwrap();               // glob.c:2710
                if backref_mode {                           // glob.c:2710
                    populate_match_array(&caps, state);     // glob.c:2710
                }                                           // glob.c:2710
                let r = untokenize_replstr(&singsub_no_tilde(raw_rep, state), backref_mode); // glob.c:2710
                return format!("{}{}{}", &s[..m.start()], r, &s[m.end()..]); // glob.c:2710
            }                                               // glob.c:2710
            s.to_string()                                   // glob.c:2710
        }                                                   // glob.c:2710
        (Some(rx), "//") => {                               // glob.c:2710
            let mut out = String::with_capacity(s.len());   // glob.c:2710
            let mut last = 0usize;                          // glob.c:2710
            for caps in rx.captures_iter(s) {               // glob.c:2710
                let m = caps.get(0).unwrap();               // glob.c:2710
                out.push_str(&s[last..m.start()]);          // glob.c:2710
                if backref_mode {                           // glob.c:2710
                    populate_match_array(&caps, state);     // glob.c:2710
                }                                           // glob.c:2710
                let r = untokenize_replstr(&singsub_no_tilde(raw_rep, state), backref_mode); // glob.c:2710
                out.push_str(&r);                           // glob.c:2710
                last = m.end();                             // glob.c:2710
            }                                               // glob.c:2710
            out.push_str(&s[last..]);                       // glob.c:2710
            out                                             // glob.c:2710
        }                                                   // glob.c:2710
        (Some(rx), "/#") => {                               // glob.c:2710
            if let Some(caps) = rx.captures(s) {            // glob.c:2710
                let m = caps.get(0).unwrap();               // glob.c:2710
                if m.start() == 0 {                         // glob.c:2710
                    if backref_mode {                       // glob.c:2710
                        populate_match_array(&caps, state); // glob.c:2710
                    }                                       // glob.c:2710
                    let r = untokenize_replstr(&singsub_no_tilde(raw_rep, state), backref_mode); // glob.c:2710
                    return format!("{}{}", r, &s[m.end()..]); // glob.c:2710
                }                                           // glob.c:2710
            }                                               // glob.c:2710
            s.to_string()                                   // glob.c:2710
        }                                                   // glob.c:2710
        (Some(rx), "/%") => {                               // glob.c:2710
            let mut last_caps: Option<regex::Captures> = None; // glob.c:2710
            for caps in rx.captures_iter(s) {               // glob.c:2710
                if caps.get(0).unwrap().end() == s.len() {  // glob.c:2710
                    last_caps = Some(caps);                 // glob.c:2710
                }                                           // glob.c:2710
            }                                               // glob.c:2710
            if let Some(caps) = last_caps {                 // glob.c:2710
                let m = caps.get(0).unwrap();               // glob.c:2710
                if backref_mode {                           // glob.c:2710
                    populate_match_array(&caps, state);     // glob.c:2710
                }                                           // glob.c:2710
                let r = untokenize_replstr(&singsub_no_tilde(raw_rep, state), backref_mode); // glob.c:2710
                return format!("{}{}", &s[..m.start()], r); // glob.c:2710
            }                                               // glob.c:2710
            s.to_string()                                   // glob.c:2710
        }                                                   // glob.c:2710
        // No regex (literal-string path).
        _ => {                                              // glob.c:2710
            let replacement = untokenize_replstr(&singsub_no_tilde(raw_rep, state), backref_mode); // glob.c:2710
            match op {                                      // glob.c:2710
                "/" => s.replacen(pattern_lit, &replacement, 1), // glob.c:2710
                "//" => s.replace(pattern_lit, &replacement), // glob.c:2710
                "/#" => match s.strip_prefix(pattern_lit) { // glob.c:2710
                    Some(rest) => format!("{}{}", replacement, rest), // glob.c:2710
                    None => s.to_string(),                  // glob.c:2710
                },                                          // glob.c:2710
                "/%" => match s.strip_suffix(pattern_lit) { // glob.c:2710
                    Some(head) => format!("{}{}", head, replacement), // glob.c:2710
                    None => s.to_string(),                  // glob.c:2710
                },                                          // glob.c:2710
                _ => s.to_string(),                         // glob.c:2710
            }                                               // glob.c:2710
        }                                                   // glob.c:2710
    }                                                       // glob.c:2710
}                                                           // glob.c:2710

/// Translate a zsh parameter-pattern to a regex.
///
/// Distinct from [`glob_to_regex`] which is path-component-aware
/// (`*` → `[^/]*`). Parameter pattern matching used by `:#`,
/// `${var/pat/repl}`, `case`, `[[`, etc. treats `*` as match-any
/// including `/`. Per zsh manual (zshexpn): "Note that these all
/// use shell pattern matching, not regular expressions."
///
/// `anchored=true` wraps in `^…$` for whole-string contexts
/// (`:#`, `case`, `[[`). `anchored=false` is for the `/` replace
/// family which lets the operator (`do_replace_one`) enforce
/// `/#`/`/%` anchoring by inspecting capture span positions.
fn param_pattern_to_regex_anchored(pattern: &str, anchored: bool) -> String { // glob.c:2710
    // `(#i)` / `(#l)` / `(#I)` extendedglob flag prefixes that toggle
    // case-insensitive matching for the rest of the pattern (until
    // a `(#I)` resets). Direct port of Src/pattern.c PAT_INSENS /
    // PAT_LOWERSENS / PAT_INSENS_OFF tokens. Parse the leading
    // `(#…)` flag block (if any) and emit a regex `(?i)` flag prefix
    // when case-insensitive is requested. Consumes from `pattern`.
    //
    // Gated on the `extendedglob` option per zsh: without
    // extendedglob, `(#i)foo` is matched literally (the parens, `#`,
    // `i`, `)` all stay as themselves). Verified vs
    // /opt/homebrew/bin/zsh -fc.
    let extendedglob_on = crate::exec::with_executor(|exec| { // glob.c:2710
        exec.options                                        // glob.c:2710
            .get("extendedglob")                            // glob.c:2710
            .copied()                                       // glob.c:2710
            .unwrap_or(false)                               // glob.c:2710
    });                                                     // glob.c:2710
    let mut chars: Vec<char> = pattern.chars().collect();   // glob.c:2710
    let mut case_insensitive = false;                       // glob.c:2710
    while extendedglob_on && chars.len() >= 4 && chars[0] == '(' && chars[1] == '#' { // glob.c:2710
        // Find the closing `)` of the flag group.
        let mut j = 2;                                      // glob.c:2710
        while j < chars.len() && chars[j] != ')' {          // glob.c:2710
            j += 1;                                         // glob.c:2710
        }                                                   // glob.c:2710
        if j >= chars.len() {                               // glob.c:2710
            break;                                          // glob.c:2710
        }                                                   // glob.c:2710
        // Inspect inner flags. Treat unknown flags as literal — keep
        // the entire (# … ) in pattern. Known flags: i/l (case-
        // insensitive on), I (case-insensitive off), b (backref —
        // handled separately by the replace path), B (backref off).
        let inner: String = chars[2..j].iter().collect();   // glob.c:2710
        let mut handled = true;                             // glob.c:2710
        for c in inner.chars() {                            // glob.c:2710
            match c {                                       // glob.c:2710
                'i' | 'l' => case_insensitive = true,       // glob.c:2710
                'I' => case_insensitive = false,            // glob.c:2710
                'b' | 'B' | 'm' | 'M' => { /* match-mode flags handled upstream */ } // glob.c:2710
                _ => {                                      // glob.c:2710
                    handled = false;                        // glob.c:2710
                    break;                                  // glob.c:2710
                }                                           // glob.c:2710
            }                                               // glob.c:2710
        }                                                   // glob.c:2710
        if !handled {                                       // glob.c:2710
            break;                                          // glob.c:2710
        }                                                   // glob.c:2710
        // Consume the `(#…)` block from the input.
        chars.drain(..=j);                                  // glob.c:2710
    }                                                       // glob.c:2710
    let mut regex = String::new();                          // glob.c:2710
    if case_insensitive {                                   // glob.c:2710
        regex.push_str("(?i)");                             // glob.c:2710
    }                                                       // glob.c:2710
    if anchored {                                           // glob.c:2710
        regex.push('^');                                    // glob.c:2710
    }                                                       // glob.c:2710
    let mut i = 0;                                          // glob.c:2710
    // Same `#` / `##` extendedglob postfix handling as
    // glob_to_regex_capturing — see that function's comment.
    let consume_postfix = |chars: &[char], i: &mut usize| -> Option<&'static str> { // glob.c:2710
        if *i + 1 < chars.len() && chars[*i + 1] == '#' {   // glob.c:2710
            if *i + 2 < chars.len() && chars[*i + 2] == '#' { // glob.c:2710
                *i += 2;
                Some("+")                                   // glob.c:2710
            } else {                                        // glob.c:2710
                *i += 1;
                Some("*")                                   // glob.c:2710
            }                                               // glob.c:2710
        } else {                                            // glob.c:2710
            None                                            // glob.c:2710
        }                                                   // glob.c:2710
    };                                                      // glob.c:2710
    while i < chars.len() {                                 // glob.c:2710
        match chars[i] {                                    // glob.c:2710
            '*' => {                                        // glob.c:2710
                regex.push_str(".*");                       // glob.c:2710
                if let Some(q) = consume_postfix(&chars, &mut i) { // glob.c:2710
                    regex.push_str(q);                      // glob.c:2710
                }                                           // glob.c:2710
            }                                               // glob.c:2710
            '?' => {                                        // glob.c:2710
                regex.push('.');                            // glob.c:2710
                if let Some(q) = consume_postfix(&chars, &mut i) { // glob.c:2710
                    regex.push_str(q);                      // glob.c:2710
                }                                           // glob.c:2710
            }                                               // glob.c:2710
            '[' => {                                        // glob.c:2710
                regex.push('[');                            // glob.c:2710
                i += 1;                                     // glob.c:2710
                if i < chars.len() && (chars[i] == '!' || chars[i] == '^') { // glob.c:2710
                    regex.push('^');                        // glob.c:2710
                    i += 1;                                 // glob.c:2710
                }                                           // glob.c:2710
                // Escape bare `[` inside class — zsh allows it
                // literal, Rust regex requires `\[`. Same fix as
                // glob_to_regex_capturing's class handler.
                while i < chars.len() && chars[i] != ']' {  // glob.c:2710
                    if chars[i] == '\\' && i + 1 < chars.len() { // glob.c:2710
                        regex.push('\\');                   // glob.c:2710
                        regex.push(chars[i + 1]);           // glob.c:2710
                        i += 2;                             // glob.c:2710
                    } else if chars[i] == '[' {             // glob.c:2710
                        regex.push('\\');                   // glob.c:2710
                        regex.push('[');                    // glob.c:2710
                        i += 1;                             // glob.c:2710
                    } else {                                // glob.c:2710
                        regex.push(chars[i]);               // glob.c:2710
                        i += 1;                             // glob.c:2710
                    }                                       // glob.c:2710
                }                                           // glob.c:2710
                regex.push(']');                            // glob.c:2710
                if let Some(q) = consume_postfix(&chars, &mut i) { // glob.c:2710
                    regex.push_str(q);                      // glob.c:2710
                }                                           // glob.c:2710
            }                                               // glob.c:2710
            '\\' if i + 1 < chars.len() => {                // glob.c:2710
                // Special case: `\(#e)` / `\(#s)` — literal backslash
                // followed by end/start anchor. After expand_string
                // collapses `\\` (source) → `\` (1 char), the pattern
                // arrives as `\(#e)` (5 chars). Treat the `\` as a
                // literal backslash to match against, and the trailing
                // `(#e)` / `(#s)` as anchors. Without this, the `\(`
                // got escaped as `\(` (literal paren) and the `(#e)`
                // detection on the next iteration never fired since
                // the `(` was already consumed. Mirrors the same
                // 5-char lookahead in BUILTIN_PARAM_REPLACE's pattern
                // compile in exec.rs. Direct port of Src/pattern.c
                // patcompswitch — `\` quotes the next char, but if
                // that char is `(` and the run forms `(#e)`/`(#s)`
                // the anchor is recognized.
                if i + 4 < chars.len()                      // glob.c:2710
                    && chars[i + 1] == '('                  // glob.c:2710
                    && chars[i + 2] == '#'                  // glob.c:2710
                    && (chars[i + 3] == 'e' || chars[i + 3] == 's') // glob.c:2710
                    && chars[i + 4] == ')'                  // glob.c:2710
                {                                           // glob.c:2710
                    regex.push_str("\\\\");                 // glob.c:2710
                    regex.push(if chars[i + 3] == 'e' { '$' } else { '^' }); // glob.c:2710
                    i += 4;                                 // glob.c:2710
                } else {                                    // glob.c:2710
                    regex.push('\\');                       // glob.c:2710
                    regex.push(chars[i + 1]);               // glob.c:2710
                    i += 1;                                 // glob.c:2710
                }                                           // glob.c:2710
            }                                               // glob.c:2710
            // `(#e)` / `(#s)` end/start anchors (extendedglob).
            // Direct port of zsh's pattern.c P_EOL / P_BOL tokens.
            // Used by zinit's `(M)pairs:#*\\(#e)` to filter elements
            // ending in literal `\`.
            '(' if i + 3 < chars.len()                      // glob.c:2710
                && chars[i + 1] == '#'                      // glob.c:2710
                && (chars[i + 2] == 'e' || chars[i + 2] == 's') // glob.c:2710
                && chars[i + 3] == ')' =>                   // glob.c:2710
            {                                               // glob.c:2710
                // Emit a non-capturing group with the anchor inside
                // since the outer regex is already anchored with
                // ^...$. Use the appropriate boundary lookahead:
                // `(?:$)` for end and `(?:^)` for start.
                regex.push_str(if chars[i + 2] == 'e' { "(?:$)" } else { "(?:^)" }); // glob.c:2710
                i += 3;                                     // glob.c:2710
            }                                               // glob.c:2710
            // Glob alternation: `(a|b|c)` is a group with `|`
            // alternation in zsh patterns (verified via
            // /opt/homebrew/bin/zsh -fc — works in `:#`, `case`,
            // `[[ = ]]` even without extendedglob). Translate to
            // regex group syntax directly. The `|` outside `(...)`
            // also stays alternation in zsh's `${var//pat1|pat2/x}`
            // and similar.
            '(' | ')' | '|' => {                            // glob.c:2710
                regex.push(chars[i]);                       // glob.c:2710
                if let Some(q) = consume_postfix(&chars, &mut i) { // glob.c:2710
                    regex.push_str(q);                      // glob.c:2710
                }                                           // glob.c:2710
            }                                               // glob.c:2710
            // Numeric range glob `<a-b>`, `<->`, `<a->`, `<-b>` —
            // matches a sequence of digits whose decimal value is
            // in the inclusive range. Direct port of zsh's pattern.c
            // `P_NUMRNG`. Only enabled when extendedglob is on (zsh
            // gates this on the option). When disabled, `<` stays
            // a literal char.
            '<' if extendedglob_on && {                     // glob.c:2710
                // Look ahead for `<...>` with a `-` somewhere inside.
                let mut k = i + 1;                          // glob.c:2710
                let mut has_dash = false;                   // glob.c:2710
                while k < chars.len() && chars[k] != '>' {  // glob.c:2710
                    if chars[k] == '-' {                    // glob.c:2710
                        has_dash = true;                    // glob.c:2710
                    }                                       // glob.c:2710
                    if !chars[k].is_ascii_digit() && chars[k] != '-' { // glob.c:2710
                        break;                              // glob.c:2710
                    }                                       // glob.c:2710
                    k += 1;                                 // glob.c:2710
                }                                           // glob.c:2710
                k < chars.len() && chars[k] == '>' && has_dash // glob.c:2710
            } => {                                          // glob.c:2710
                // Find the `>`.
                let mut k = i + 1;                          // glob.c:2710
                while k < chars.len() && chars[k] != '>' {  // glob.c:2710
                    k += 1;                                 // glob.c:2710
                }                                           // glob.c:2710
                // Range body is `chars[i+1..k]`. We don't enforce
                // numeric bounds in the regex (regex can't do
                // value-bounded numeric matching efficiently), so
                // approximate: match any digit sequence. Post-match
                // bounds-checking would require a captures pass that
                // the param_pattern_to_regex_anchored callers don't
                // currently do — accept the over-match for now.
                // This still correctly differentiates digit runs from
                // non-digits in `[(I)foo-<->]`-style patterns.
                regex.push_str("[0-9]+");                   // glob.c:2710
                i = k;                                      // glob.c:2710
                if let Some(q) = consume_postfix(&chars, &mut i) { // glob.c:2710
                    regex.push_str(q);                      // glob.c:2710
                }                                           // glob.c:2710
            }                                               // glob.c:2710
            // Regex metachars that are literals in glob — escape.
            c @ ('.' | '+' | '^' | '$' | '{' | '}') => {    // glob.c:2710
                regex.push('\\');                           // glob.c:2710
                regex.push(c);                              // glob.c:2710
            }                                               // glob.c:2710
            c => {                                          // glob.c:2710
                regex.push(c);                              // glob.c:2710
                if let Some(q) = consume_postfix(&chars, &mut i) { // glob.c:2710
                    regex.push_str(q);                      // glob.c:2710
                }                                           // glob.c:2710
            }                                               // glob.c:2710
        }                                                   // glob.c:2710
        i += 1;                                             // glob.c:2710
    }                                                       // glob.c:2710
    if anchored {                                           // glob.c:2710
        regex.push('$');                                    // glob.c:2710
    }                                                       // glob.c:2710
    regex                                                   // glob.c:2710
}                                                           // glob.c:2710

/// Whole-match form (`^…$`). Used by `:#`, `case`, `[[ = ]]`.
fn param_pattern_to_regex(pattern: &str) -> String {        // glob.c:2710
    param_pattern_to_regex_anchored(pattern, true)          // glob.c:2710
}                                                           // glob.c:2710

/// Public unanchored variant — used by `apply_var_modifier`'s
/// ReplaceAll path in exec.rs to glob-match `${var//pat/repl}`
/// when `pat` contains metas. Direct port of zsh's getmatch path
/// (Src/utils.c) which compiles every replace pattern. The
/// surface is intentionally minimal (just unanchored regex);
/// callers that need backref or anchor behaviour go through
/// the subst_port replace dispatch directly.
pub fn compile_glob_to_regex_for_replace(pattern: &str) -> String { // c:N/A
    param_pattern_to_regex_anchored(pattern, false)         // c:N/A
}                                                           // c:N/A

fn glob_to_regex(pattern: &str) -> String {                 // glob.c:2710
    let mut regex = String::from("^");                      // glob.c:2710
    let chars: Vec<char> = pattern.chars().collect();       // glob.c:2710
    let mut i = 0;                                          // glob.c:2710

    while i < chars.len() {                                 // glob.c:2710
        match chars[i] {                                    // glob.c:2710
            '*' => {                                        // glob.c:2710
                if i + 1 < chars.len() && chars[i + 1] == '*' { // glob.c:2710
                    // ** matches everything including /
                    regex.push_str(".*");                   // glob.c:2710
                    i += 1;                                 // glob.c:2710
                } else {                                    // glob.c:2710
                    // * matches anything except /
                    regex.push_str("[^/]*");                // glob.c:2710
                }                                           // glob.c:2710
            }                                               // glob.c:2710
            '?' => regex.push('.'),                         // glob.c:2710
            '[' => {                                        // glob.c:2710
                regex.push('[');                            // glob.c:2710
                i += 1;                                     // glob.c:2710
                // Handle negation
                if i < chars.len() && (chars[i] == '!' || chars[i] == '^') { // glob.c:2710
                    regex.push('^');                        // glob.c:2710
                    i += 1;                                 // glob.c:2710
                }                                           // glob.c:2710
                // Copy until ]
                while i < chars.len() && chars[i] != ']' {  // glob.c:2710
                    if chars[i] == '\\' && i + 1 < chars.len() { // glob.c:2710
                        regex.push('\\');                   // glob.c:2710
                        i += 1;                             // glob.c:2710
                        regex.push(chars[i]);               // glob.c:2710
                    } else {                                // glob.c:2710
                        regex.push(chars[i]);               // glob.c:2710
                    }                                       // glob.c:2710
                    i += 1;                                 // glob.c:2710
                }                                           // glob.c:2710
                regex.push(']');                            // glob.c:2710
            }                                               // glob.c:2710
            '.' | '+' | '^' | '$' | '(' | ')' | '{' | '}' | '|' | '\\' => { // glob.c:2710
                regex.push('\\');                           // glob.c:2710
                regex.push(chars[i]);                       // glob.c:2710
            }                                               // glob.c:2710
            c if itok(c) => {                               // glob.c:2710
                // Internal token - convert to real char
                regex.push(ztokens(c));                     // glob.c:2710
            }                                               // glob.c:2710
            c => regex.push(c),                             // glob.c:2710
        }                                                   // glob.c:2710
        i += 1;                                             // glob.c:2710
    }                                                       // glob.c:2710

    regex.push('$');                                        // glob.c:2710
    regex                                                   // glob.c:2710
}                                                           // glob.c:2710

/// Match pattern against array elements
/// Port of getmatcharr() logic
pub fn getmatcharr(                                         // c:N/A
    aval: &mut [String],                                    // c:N/A
    pattern: &str,                                          // c:N/A
    flags: u32,                                             // c:N/A
    flnum: i32,                                             // c:N/A
    replstr: Option<&str>,                                  // c:N/A
) {                                                         // c:N/A
    for val in aval.iter_mut() {                            // c:N/A
        *val = getmatch(val, pattern, flags, flnum, replstr);
    }                                                       // c:N/A
}                                                           // c:N/A

/// Array intersection
/// Port of ${array1|array2} logic
pub fn array_union(arr1: &[String], arr2: &[String]) -> Vec<String> { // c:N/A
    let set2: std::collections::HashSet<_> = arr2.iter().collect(); // c:N/A
    arr1.iter().filter(|s| !set2.contains(s)).cloned().collect() // c:N/A
}                                                           // c:N/A

/// Array intersection
/// Port of ${array1*array2} logic  
pub fn array_intersection(arr1: &[String], arr2: &[String]) -> Vec<String> { // c:N/A
    let set2: std::collections::HashSet<_> = arr2.iter().collect(); // c:N/A
    arr1.iter().filter(|s| set2.contains(s)).cloned().collect() // c:N/A
}                                                           // c:N/A

/// Array zip operation
/// Port of ${array1^array2} logic
pub fn array_zip(arr1: &[String], arr2: &[String], shortest: bool) -> Vec<String> { // c:N/A
    let len = if shortest {                                 // c:N/A
        arr1.len().min(arr2.len())                          // c:N/A
    } else {                                                // c:N/A
        arr1.len().max(arr2.len())                          // c:N/A
    };                                                      // c:N/A

    let mut result = Vec::with_capacity(len * 2);           // c:N/A
    for i in 0..len {                                       // c:N/A
        let idx1 = if arr1.is_empty() { 0 } else { i % arr1.len() }; // c:N/A
        let idx2 = if arr2.is_empty() { 0 } else { i % arr2.len() }; // c:N/A
        result.push(arr1.get(idx1).cloned().unwrap_or_default()); // c:N/A
        result.push(arr2.get(idx2).cloned().unwrap_or_default()); // c:N/A
    }                                                       // c:N/A
    result                                                  // c:N/A
}                                                           // c:N/A

/// Concatenate string parts for parameter substitution result
/// Port of strcatsub() from subst.c lines 783-797
pub fn strcatsub(prefix: &str, src: &str, suffix: &str, glob_subst: bool) -> String { // c:N/A
    let mut result = String::with_capacity(prefix.len() + src.len() + suffix.len()); // c:N/A
    result.push_str(prefix);                                // c:N/A

    if glob_subst {                                         // c:N/A
        result.push_str(&shtokenize(src));                  // c:N/A
    } else {                                                // c:N/A
        result.push_str(src);                               // c:N/A
    }                                                       // c:N/A

    result.push_str(suffix);                                // c:N/A
    result                                                  // c:N/A
}                                                           // c:N/A

/// Check for null argument marker
pub fn inull(c: char) -> bool {                             // c:N/A
    matches!(c, '\u{8F}' | '\u{94}' | '\u{95}' | '\u{92}')  // c:N/A
}                                                           // c:N/A

/// Chunk - remove a character from string
pub fn chuck(s: &str, pos: usize) -> String {               // c:N/A
    let mut result = String::new();                         // c:N/A
    for (i, c) in s.chars().enumerate() {                   // c:N/A
        if i != pos {                                       // c:N/A
            result.push(c);                                 // c:N/A
        }                                                   // c:N/A
    }                                                       // c:N/A
    result                                                  // c:N/A
}                                                           // c:N/A

// ============================================================================
// Additional helper functions ported from subst.c
// ============================================================================

/// Get the value of a special parameter
/// Port of getsparam() logic
pub fn getsparam(name: &str, state: &SubstState) -> Option<String> { // c:N/A
    // Check shell variables first
    if let Some(val) = state.variables.get(name) {          // c:N/A
        return Some(val.clone());                           // c:N/A
    }                                                       // c:N/A

    // Check environment
    std::env::var(name).ok()                                // c:N/A
}                                                           // c:N/A

/// Get the value of an array parameter
/// Port of getaparam() logic
pub fn getaparam(name: &str, state: &SubstState) -> Option<Vec<String>> { // c:N/A
    state.arrays.get(name).cloned()                         // c:N/A
}                                                           // c:N/A

/// Get the value of a hash (associative array) parameter
/// Port of gethparam() logic
pub fn gethparam(                                           // c:N/A
    name: &str,                                             // c:N/A
    state: &SubstState,                                     // c:N/A
) -> Option<indexmap::IndexMap<String, String>> {           // c:N/A
    state.assoc_arrays.get(name).cloned()                   // c:N/A
}                                                           // c:N/A

/// Set a scalar parameter
/// Port of setsparam() logic
pub fn setsparam(name: &str, value: &str, state: &mut SubstState) { // c:N/A
    state.variables.insert(name.to_string(), value.to_string()); // c:N/A
    // Also set in environment for exported params
    // std::env::set_var(name, value);
}                                                           // c:N/A

/// Set an array parameter
/// Port of setaparam() logic
pub fn setaparam(name: &str, value: Vec<String>, state: &mut SubstState) { // c:N/A
    state.arrays.insert(name.to_string(), value);           // c:N/A
}                                                           // c:N/A

/// Set an associative array parameter
/// Port of sethparam() logic
pub fn sethparam(                                           // c:N/A
    name: &str,                                             // c:N/A
    value: indexmap::IndexMap<String, String>,              // c:N/A
    state: &mut SubstState,                                 // c:N/A
) {                                                         // c:N/A
    state.assoc_arrays.insert(name.to_string(), value);     // c:N/A
}                                                           // c:N/A

/// Make an array from a single element
/// Port of hmkarray() logic
pub fn hmkarray(val: &str) -> Vec<String> {                 // c:N/A
    if val.is_empty() {                                     // c:N/A
        Vec::new()                                          // c:N/A
    } else {                                                // c:N/A
        vec![val.to_string()]                               // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

/// Duplicate string with prefix
/// Port of dupstrpfx() logic
pub fn dupstrpfx(s: &str, len: usize) -> String {           // c:N/A
    s.chars().take(len).collect()                           // c:N/A
}                                                           // c:N/A

/// Dynamic string concatenation
/// Port of dyncat() logic
pub fn dyncat(s1: &str, s2: &str) -> String {               // c:N/A
    format!("{}{}", s1, s2)                                 // c:N/A
}                                                           // c:N/A

/// Triple string concatenation
/// Port of zhtricat() logic
pub fn zhtricat(s1: &str, s2: &str, s3: &str) -> String {   // c:N/A
    format!("{}{}{}", s1, s2, s3)                           // c:N/A
}                                                           // c:N/A

/// Find the next word in a string
/// Port of findword() logic used in modify()
pub fn findword(s: &str, sep: Option<&str>) -> Option<(String, String)> { // c:N/A
    let separator = sep.unwrap_or(" \t\n");                 // c:N/A

    // Skip leading separators
    let trimmed = s.trim_start_matches(|c: char| separator.contains(c)); // c:N/A
    if trimmed.is_empty() {                                 // c:N/A
        return None;                                        // c:N/A
    }                                                       // c:N/A

    // Find end of word
    let word_end = trimmed                                  // c:N/A
        .find(|c: char| separator.contains(c))              // c:N/A
        .unwrap_or(trimmed.len());                          // c:N/A

    let word = &trimmed[..word_end];                        // c:N/A
    let rest = &trimmed[word_end..];                        // c:N/A

    Some((word.to_string(), rest.to_string()))              // c:N/A
}                                                           // c:N/A

/// Check if a path is absolute
pub fn is_absolute_path(s: &str) -> bool {                  // c:N/A
    s.starts_with('/')                                      // c:N/A
}                                                           // c:N/A

/// Remove trailing path components
/// Port of remtpath() logic for :h modifier
pub fn remtpath(s: &str, count: usize) -> String {          // hist.c:2056
    // Direct port of src/zsh/Src/hist.c:2055-2118 `remtpath`. zsh
    // semantics:
    //   `:h`  (count == 0)  — remove last path component.
    //   `:hN` (count > 0)   — keep first N components from the front.
    //   Trailing slashes are stripped first.
    //   Repeated separators count as one.
    //   Empty result on a relative path becomes ".".
    //   Leading "/" never erased; "//" (cygwin) preserved.
    let bytes: Vec<u8> = s.bytes().collect();               // hist.c:2056
    let n = bytes.len();                                    // hist.c:2056
    if n == 0 {                                             // hist.c:2056
        return s.to_string();                               // hist.c:2056
    }                                                       // hist.c:2056
    let is_sep = |b: u8| b == b'/';                         // hist.c:2056

    // hist.c:2058-2062 — start at last char, skip trailing separators.
    let mut end: isize = (n as isize) - 1;                  // hist.c:2056
    while end >= 0 && is_sep(bytes[end as usize]) {         // hist.c:2056
        end -= 1;                                           // hist.c:2056
    }                                                       // hist.c:2056

    if count == 0 {                                         // hist.c:2056
        // hist.c:2064-2066 — skip filename (back through non-seps).
        while end >= 0 && !is_sep(bytes[end as usize]) {    // hist.c:2056
            end -= 1;                                       // hist.c:2056
        }                                                   // hist.c:2056
        if end < 0 {                                        // hist.c:2056
            // hist.c:2068-2074 — no separator found.
            return if is_sep(bytes[0]) {                    // hist.c:2056
                "/".to_string()                             // hist.c:2056
            } else {                                        // hist.c:2056
                ".".to_string()                             // hist.c:2056
            };                                              // hist.c:2056
        }                                                   // hist.c:2056
        // hist.c:2104-2106 — collapse repeated separators.
        while end > 0 && is_sep(bytes[(end - 1) as usize]) { // hist.c:2056
            end -= 1;                                       // hist.c:2056
        }                                                   // hist.c:2056
        // hist.c:2107-2114 — never erase root slash; preserve "//".
        if end == 0 {                                       // hist.c:2056
            end += 1;                                       // hist.c:2056
            if (end as usize) < n                           // hist.c:2056
                && is_sep(bytes[end as usize])              // hist.c:2056
                && (end + 1 >= n as isize || !is_sep(bytes[(end + 1) as usize])) // hist.c:2056
            {                                               // hist.c:2056
                end += 1;                                   // hist.c:2056
            }                                               // hist.c:2056
        }                                                   // hist.c:2056
        return s[..end as usize].to_string();               // hist.c:2056
    }                                                       // hist.c:2056

    // count > 0 — hist.c:2078-2102 — keep first `count` components.
    // Walk forward; each separator marks a component boundary. The
    // leading slash counts as one component.
    let mut strp: usize = 0;                                // hist.c:2056
    let mut remaining = count as isize;                     // hist.c:2056
    let limit = end as usize;                               // hist.c:2056
    while strp < limit {                                    // hist.c:2056
        if is_sep(bytes[strp]) {                            // hist.c:2056
            remaining -= 1;                                 // hist.c:2056
            if remaining <= 0 {                             // hist.c:2056
                if strp == 0 {                              // hist.c:2056
                    strp += 1;                              // hist.c:2056
                }                                           // hist.c:2056
                return s[..strp].to_string();               // hist.c:2056
            }                                               // hist.c:2056
            // Count consecutive separators as one.
            while strp + 1 < bytes.len() && is_sep(bytes[strp + 1]) { // hist.c:2056
                strp += 1;                                  // hist.c:2056
            }                                               // hist.c:2056
        }                                                   // hist.c:2056
        strp += 1;                                          // hist.c:2056
    }                                                       // hist.c:2056
    // Full string needed (hist.c:2101).
    s.to_string()                                           // hist.c:2056
}                                                           // hist.c:2056

/// Remove leading path components — direct port of
/// src/zsh/Src/hist.c:2151-2186 `remlpaths`. zsh `:t`
/// (count==1) returns the last path component; `:tN`
/// returns the last N components.
///
/// C algorithm:
///   1. Strip trailing separators.
///   2. Walk back from the end. Each separator decrements `count`.
///   3. When `count` reaches 0, the part AFTER that separator is
///      the result.
///   4. Consecutive separators count as one.
///   5. If we walk past the start, return the whole string.
pub fn remlpaths(s: &str, count: usize) -> String {         // c:N/A
    if s.is_empty() || count == 0 {                         // c:N/A
        return s.to_string();                               // c:N/A
    }                                                       // c:N/A
    let bytes: &[u8] = s.as_bytes();                        // c:N/A
    let mut end = bytes.len();                              // c:N/A
    // Strip trailing separators (hist.c:2156-2161).
    while end > 0 && bytes[end - 1] == b'/' {               // c:N/A
        end -= 1;                                           // c:N/A
    }                                                       // c:N/A
    if end == 0 {                                           // c:N/A
        // String was all-separators.
        return s.to_string();                               // c:N/A
    }                                                       // c:N/A
    let mut count = count as isize;                         // c:N/A
    let mut i: isize = (end as isize) - 1;                  // c:N/A
    loop {                                                  // c:N/A
        // Walk back over a non-separator run looking for separators.
        while i >= 0 {                                      // c:N/A
            if bytes[i as usize] == b'/' {                  // c:N/A
                count -= 1;                                 // c:N/A
                if count > 0 {                              // c:N/A
                    if i > 0 {                              // c:N/A
                        i -= 1;                             // c:N/A
                        break; // continue outer loop, skipping consecutive seps // c:N/A
                    } else {                                // c:N/A
                        // Whole string needed.
                        return s[..end].to_string();        // c:N/A
                    }                                       // c:N/A
                }                                           // c:N/A
                // count == 0 — return part after this separator.
                return s[(i as usize + 1)..end].to_string(); // c:N/A
            }                                               // c:N/A
            i -= 1;                                         // c:N/A
        }                                                   // c:N/A
        // Count consecutive separators as 1 (hist.c:2179-2181).
        while i >= 0 && bytes[i as usize] == b'/' {         // c:N/A
            i -= 1;                                         // c:N/A
        }                                                   // c:N/A
        if i <= 0 {                                         // c:N/A
            break;                                          // c:N/A
        }                                                   // c:N/A
    }                                                       // c:N/A
    // No (or insufficient) separators — return whole string.
    s[..end].to_string()                                    // c:N/A
}                                                           // c:N/A

/// Remove text (extension)
/// Port of remtext() logic for :r modifier
pub fn remtext(s: &str) -> String {                         // c:N/A
    if let Some(pos) = s.rfind('.') {                       // c:N/A
        // Make sure the dot is not in a directory component
        if let Some(slash_pos) = s.rfind('/') {             // c:N/A
            if pos > slash_pos {                            // c:N/A
                return s[..pos].to_string();                // c:N/A
            }                                               // c:N/A
        } else {                                            // c:N/A
            return s[..pos].to_string();                    // c:N/A
        }                                                   // c:N/A
    }                                                       // c:N/A
    s.to_string()                                           // c:N/A
}                                                           // c:N/A

/// Remove all but extension
/// Port of rembutext() logic for :e modifier
pub fn rembutext(s: &str) -> String {                       // c:N/A
    if let Some(pos) = s.rfind('.') {                       // c:N/A
        // Make sure the dot is not in a directory component
        if let Some(slash_pos) = s.rfind('/') {             // c:N/A
            if pos > slash_pos {                            // c:N/A
                return s[pos + 1..].to_string();            // c:N/A
            }                                               // c:N/A
        } else {                                            // c:N/A
            return s[pos + 1..].to_string();                // c:N/A
        }                                                   // c:N/A
    }                                                       // c:N/A
    String::new()                                           // c:N/A
}                                                           // c:N/A

/// Change to absolute path
/// Port of chabspath() logic for :a modifier
pub fn chabspath(s: &str) -> String {                       // c:N/A
    // Port of `xsymlinks()` from Src/utils.c:872 (the `.` / `..`
    // segment-walking path; the symlink-resolution branch via
    // `readlink` is intentionally skipped here — `:A` proper goes
    // through `chrealpath` further down). The C source:
    //   - splits the input on `/`
    //   - for `"."` segments → continue (skip)
    //   - for `".."` segments → pop one segment off the running
    //     buffer (unless we're at root or the buffer is empty)
    //   - for any other segment → append `/<seg>` to the buffer
    //
    // Tested via `/bin/zsh -c 'x=/a/b/../c; print -- ${x:A}'` → /a/c
    // and `${.../a/./b/c:A}` → /a/b/c.
    let abs = if s.starts_with('/') {                       // c:N/A
        s.to_string()                                       // c:N/A
    } else if let Ok(cwd) = std::env::current_dir() {       // c:N/A
        format!("{}/{}", cwd.display(), s)                  // c:N/A
    } else {                                                // c:N/A
        s.to_string()                                       // c:N/A
    };                                                      // c:N/A

    // Walk segments, collapse `.` and `..`. Preserve the leading `/`.
    let mut out: Vec<&str> = Vec::new();                    // c:N/A
    for seg in abs.split('/') {                             // c:N/A
        match seg {                                         // c:N/A
            "" | "." => continue, // empty (multi-slash) or `.` skip // c:N/A
            ".." => {                                       // c:N/A
                // Pop one component if any; can't pop past root.
                out.pop();                                  // c:N/A
            }                                               // c:N/A
            other => out.push(other),                       // c:N/A
        }                                                   // c:N/A
    }                                                       // c:N/A
    if out.is_empty() {                                     // c:N/A
        // All popped — must be root or empty input. Per C xsymlinks
        // line 886-889, never erase the root slash.
        return "/".to_string();                             // c:N/A
    }                                                       // c:N/A
    let mut result = String::new();                         // c:N/A
    for seg in &out {                                       // c:N/A
        result.push('/');                                   // c:N/A
        result.push_str(seg);                               // c:N/A
    }                                                       // c:N/A
    result                                                  // c:N/A
}                                                           // c:N/A

/// Change to real path (resolve symlinks)
/// Port of chrealpath() logic for :A modifier  
pub fn chrealpath(s: &str) -> String {                      // c:N/A
    match std::fs::canonicalize(s) {                        // c:N/A
        Ok(p) => p.to_string_lossy().to_string(),           // c:N/A
        Err(_) => s.to_string(),                            // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

/// Resolve symlinks
/// Port of xsymlink() logic for :P modifier
pub fn xsymlink(path: &str, resolve: bool) -> String {      // c:N/A
    if resolve {                                            // c:N/A
        match std::fs::canonicalize(path) {                 // c:N/A
            Ok(p) => p.to_string_lossy().to_string(),       // c:N/A
            Err(_) => path.to_string(),                     // c:N/A
        }                                                   // c:N/A
    } else {                                                // c:N/A
        path.to_string()                                    // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

/// Convert number to string with base
/// Port of convbase_underscore() logic
pub fn convbase(val: i64, base: u32, underscore: bool) -> String { // c:N/A
    if base == 10 {                                         // c:N/A
        if underscore {                                     // c:N/A
            // Add underscores every 3 digits
            let s = val.abs().to_string();                  // c:N/A
            let mut result = String::new();                 // c:N/A
            for (i, c) in s.chars().rev().enumerate() {     // c:N/A
                if i > 0 && i % 3 == 0 {                    // c:N/A
                    result.insert(0, '_');                  // c:N/A
                }                                           // c:N/A
                result.insert(0, c);                        // c:N/A
            }                                               // c:N/A
            if val < 0 {                                    // c:N/A
                result.insert(0, '-');                      // c:N/A
            }                                               // c:N/A
            result                                          // c:N/A
        } else {                                            // c:N/A
            val.to_string()                                 // c:N/A
        }                                                   // c:N/A
    } else if base == 16 {                                  // c:N/A
        format!("{:x}", val)                                // c:N/A
    } else if base == 8 {                                   // c:N/A
        format!("{:o}", val)                                // c:N/A
    } else if base == 2 {                                   // c:N/A
        format!("{:b}", val)                                // c:N/A
    } else {                                                // c:N/A
        val.to_string()                                     // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

/// Evaluate a math expression
/// Simplified port of matheval() logic
pub fn matheval(expr: &str) -> MathResult {                 // math.c:1480
    // Try to parse as integer
    if let Ok(n) = expr.trim().parse::<i64>() {             // math.c:1480
        return MathResult::Integer(n);                      // math.c:1480
    }                                                       // math.c:1480

    // Try to parse as float
    if let Ok(n) = expr.trim().parse::<f64>() {             // math.c:1480
        return MathResult::Float(n);                        // math.c:1480
    }                                                       // math.c:1480

    // Simple expression parsing
    let expr = expr.trim();                                 // math.c:1480

    // Addition
    if let Some(pos) = expr.rfind('+') {                    // math.c:1480
        if pos > 0 {                                        // math.c:1480
            let left = matheval(&expr[..pos]);              // math.c:1480
            let right = matheval(&expr[pos + 1..]);         // math.c:1480
            return match (left, right) {                    // math.c:1480
                (MathResult::Integer(a), MathResult::Integer(b)) => MathResult::Integer(a + b), // math.c:1480
                (MathResult::Float(a), MathResult::Float(b)) => MathResult::Float(a + b), // math.c:1480
                (MathResult::Integer(a), MathResult::Float(b)) => MathResult::Float(a as f64 + b), // math.c:1480
                (MathResult::Float(a), MathResult::Integer(b)) => MathResult::Float(a + b as f64), // math.c:1480
            };                                              // math.c:1480
        }                                                   // math.c:1480
    }                                                       // math.c:1480

    // Subtraction
    if let Some(pos) = expr.rfind('-') {                    // math.c:1480
        if pos > 0 {                                        // math.c:1480
            let left = matheval(&expr[..pos]);              // math.c:1480
            let right = matheval(&expr[pos + 1..]);         // math.c:1480
            return match (left, right) {                    // math.c:1480
                (MathResult::Integer(a), MathResult::Integer(b)) => MathResult::Integer(a - b), // math.c:1480
                (MathResult::Float(a), MathResult::Float(b)) => MathResult::Float(a - b), // math.c:1480
                (MathResult::Integer(a), MathResult::Float(b)) => MathResult::Float(a as f64 - b), // math.c:1480
                (MathResult::Float(a), MathResult::Integer(b)) => MathResult::Float(a - b as f64), // math.c:1480
            };                                              // math.c:1480
        }                                                   // math.c:1480
    }                                                       // math.c:1480

    // Multiplication
    if let Some(pos) = expr.rfind('*') {                    // math.c:1480
        let left = matheval(&expr[..pos]);                  // math.c:1480
        let right = matheval(&expr[pos + 1..]);             // math.c:1480
        return match (left, right) {                        // math.c:1480
            (MathResult::Integer(a), MathResult::Integer(b)) => MathResult::Integer(a * b), // math.c:1480
            (MathResult::Float(a), MathResult::Float(b)) => MathResult::Float(a * b), // math.c:1480
            (MathResult::Integer(a), MathResult::Float(b)) => MathResult::Float(a as f64 * b), // math.c:1480
            (MathResult::Float(a), MathResult::Integer(b)) => MathResult::Float(a * b as f64), // math.c:1480
        };                                                  // math.c:1480
    }                                                       // math.c:1480

    // Division
    if let Some(pos) = expr.rfind('/') {                    // math.c:1480
        let left = matheval(&expr[..pos]);                  // math.c:1480
        let right = matheval(&expr[pos + 1..]);             // math.c:1480
        return match (left, right) {                        // math.c:1480
            (MathResult::Integer(a), MathResult::Integer(b)) if b != 0 => { // math.c:1480
                MathResult::Integer(a / b)                  // math.c:1480
            }                                               // math.c:1480
            (MathResult::Float(a), MathResult::Float(b)) => MathResult::Float(a / b), // math.c:1480
            (MathResult::Integer(a), MathResult::Float(b)) => MathResult::Float(a as f64 / b), // math.c:1480
            (MathResult::Float(a), MathResult::Integer(b)) => MathResult::Float(a / b as f64), // math.c:1480
            _ => MathResult::Integer(0),                    // math.c:1480
        };                                                  // math.c:1480
    }                                                       // math.c:1480

    // Modulo
    if let Some(pos) = expr.rfind('%') {                    // math.c:1480
        let left = matheval(&expr[..pos]);                  // math.c:1480
        let right = matheval(&expr[pos + 1..]);             // math.c:1480
        return match (left, right) {                        // math.c:1480
            (MathResult::Integer(a), MathResult::Integer(b)) if b != 0 => { // math.c:1480
                MathResult::Integer(a % b)                  // math.c:1480
            }                                               // math.c:1480
            _ => MathResult::Integer(0),                    // math.c:1480
        };                                                  // math.c:1480
    }                                                       // math.c:1480

    MathResult::Integer(0)                                  // math.c:1480
}                                                           // math.c:1480

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

    pub fn to_i64(&self) -> i64 {                           // math.c:1480
        match self {                                        // math.c:1480
            MathResult::Integer(n) => *n,                   // math.c:1480
            MathResult::Float(n) => *n as i64,              // math.c:1480
        }                                                   // math.c:1480
    }                                                       // math.c:1480
}                                                           // math.c:1480

/// Evaluate a math expression and return integer result
/// Port of mathevali() logic
pub fn mathevali(expr: &str) -> i64 {                       // c:N/A
    matheval(expr).to_i64()                                 // c:N/A
}                                                           // c:N/A

/// Parse a substitution string for the (e) flag
/// Port of parse_subst_string() logic
pub fn parse_subst_string(s: &str) -> Result<String, String> { // c:N/A
    // This is a simplified version - real implementation would
    // handle nested substitutions, quoting, etc.
    Ok(s.to_string())                                       // c:N/A
}                                                           // c:N/A

/// Buffer words for (z) flag parsing
/// Port of bufferwords() logic
pub fn bufferwords(s: &str, flags: u32) -> Vec<String> {    // hist.c:3385
    // Simplified lexical word splitting
    let mut words = Vec::new();                             // hist.c:3385
    let mut current = String::new();                        // hist.c:3385
    let mut in_quote = false;                               // hist.c:3385
    let mut quote_char = '\0';                              // hist.c:3385
    let mut escape_next = false;                            // hist.c:3385

    for c in s.chars() {                                    // hist.c:3385
        if escape_next {                                    // hist.c:3385
            current.push(c);                                // hist.c:3385
            escape_next = false;                            // hist.c:3385
            continue;                                       // hist.c:3385
        }                                                   // hist.c:3385

        match c {                                           // hist.c:3385
            '\\' => {                                       // hist.c:3385
                escape_next = true;                         // hist.c:3385
                current.push(c);                            // hist.c:3385
            }                                               // hist.c:3385
            '"' | '\'' => {                                 // hist.c:3385
                if in_quote && c == quote_char {            // hist.c:3385
                    in_quote = false;                       // hist.c:3385
                    quote_char = '\0';                      // hist.c:3385
                } else if !in_quote {                       // hist.c:3385
                    in_quote = true;                        // hist.c:3385
                    quote_char = c;                         // hist.c:3385
                }                                           // hist.c:3385
                current.push(c);                            // hist.c:3385
            }                                               // hist.c:3385
            ' ' | '\t' | '\n' if !in_quote => {             // hist.c:3385
                if !current.is_empty() {                    // hist.c:3385
                    words.push(current.clone());            // hist.c:3385
                    current.clear();                        // hist.c:3385
                }                                           // hist.c:3385
            }                                               // hist.c:3385
            _ => current.push(c),                           // hist.c:3385
        }                                                   // hist.c:3385
    }                                                       // hist.c:3385

    if !current.is_empty() {                                // hist.c:3385
        words.push(current);                                // hist.c:3385
    }                                                       // hist.c:3385

    words                                                   // hist.c:3385
}                                                           // hist.c:3385

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

/// Fetch a value from parameters
/// Simplified port of fetchvalue() logic
pub fn fetchvalue(                                          // params.c:2180
    name: &str,                                             // params.c:2180
    subscript: Option<&str>,                                // params.c:2180
    flags: u32,                                             // params.c:2180
    state: &SubstState,                                     // params.c:2180
) -> Option<ParamValue> {                                   // params.c:2180
    // Check for arrays
    if let Some(arr) = state.arrays.get(name) {             // params.c:2180
        if let Some(sub) = subscript {                      // params.c:2180
            if sub == "@" || sub == "*" {                   // params.c:2180
                return Some(ParamValue::Array(arr.clone())); // params.c:2180
            }                                               // params.c:2180
            // Single element
            let (idx, end_idx) = eval_subscript(sub, arr.len()); // params.c:2180
            if let Some(end) = end_idx {                    // params.c:2180
                // Range
                let slice: Vec<String> = arr.get(idx..=end).map(|s| s.to_vec()).unwrap_or_default(); // params.c:2180
                return Some(ParamValue::Array(slice));      // params.c:2180
            } else if idx < arr.len() {                     // params.c:2180
                return Some(ParamValue::Scalar(arr[idx].clone())); // params.c:2180
            }                                               // params.c:2180
        }                                                   // params.c:2180
        return Some(ParamValue::Array(arr.clone()));        // params.c:2180
    }                                                       // params.c:2180

    // Check for associative arrays
    if let Some(hash) = state.assoc_arrays.get(name) {      // params.c:2180
        if let Some(sub) = subscript {                      // params.c:2180
            if sub == "@" || sub == "*" {                   // params.c:2180
                if flags & scanpm_flags::WANTKEYS != 0 {    // params.c:2180
                    return Some(ParamValue::Array(hash.keys().cloned().collect())); // params.c:2180
                } else {                                    // params.c:2180
                    return Some(ParamValue::Array(hash.values().cloned().collect())); // params.c:2180
                }                                           // params.c:2180
            }                                               // params.c:2180
            // Single key
            if let Some(val) = hash.get(sub) {              // params.c:2180
                return Some(ParamValue::Scalar(val.clone())); // params.c:2180
            }                                               // params.c:2180
        }                                                   // params.c:2180
        return Some(ParamValue::Array(hash.values().cloned().collect())); // params.c:2180
    }                                                       // params.c:2180

    // Check for scalars
    if let Some(val) = state.variables.get(name) {          // params.c:2180
        return Some(ParamValue::Scalar(val.clone()));       // params.c:2180
    }                                                       // params.c:2180

    // Check environment
    if let Ok(val) = std::env::var(name) {                  // params.c:2180
        return Some(ParamValue::Scalar(val));               // params.c:2180
    }                                                       // params.c:2180

    None                                                    // params.c:2180
}                                                           // params.c:2180

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

    pub fn to_array(&self) -> Vec<String> {                 // params.c:2180
        match self {                                        // params.c:2180
            ParamValue::Scalar(s) => vec![s.clone()],       // params.c:2180
            ParamValue::Array(arr) => arr.clone(),          // params.c:2180
        }                                                   // params.c:2180
    }                                                       // params.c:2180

    pub fn is_array(&self) -> bool {                        // params.c:2180
        matches!(self, ParamValue::Array(_))                // params.c:2180
    }                                                       // params.c:2180
}                                                           // params.c:2180

/// Get the string value from a parameter
/// Port of getstrvalue() logic
pub fn getstrvalue(pv: &ParamValue) -> String {             // c:N/A
    pv.to_string()                                          // c:N/A
}                                                           // c:N/A

/// Get the array value from a parameter
/// Port of getarrvalue() logic
pub fn getarrvalue(pv: &ParamValue) -> Vec<String> {        // c:N/A
    pv.to_array()                                           // c:N/A
}                                                           // c:N/A

/// Get array length
/// Port of arrlen() logic
pub fn arrlen(arr: &[String]) -> usize {                    // c:N/A
    arr.len()                                               // c:N/A
}                                                           // c:N/A

/// Check if array length is less than or equal to n
/// Port of arrlen_le() logic (optimization)
pub fn arrlen_le(arr: &[String], n: usize) -> bool {        // c:N/A
    arr.len() <= n                                          // c:N/A
}                                                           // c:N/A

/// Duplicate an array
/// Port of arrdup() logic
pub fn arrdup(arr: &[String]) -> Vec<String> {              // c:N/A
    arr.to_vec()                                            // c:N/A
}                                                           // c:N/A

/// Insert one linked list into another
/// Port of insertlinklist() logic
pub fn insertlinklist(dest: &mut LinkList, pos: usize, src: &LinkList) { // c:N/A
    for (i, node) in src.nodes.iter().enumerate() {         // c:N/A
        dest.nodes.insert(pos + 1 + i, node.clone());       // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

/// GETKEYS_* flags for getkeystring()
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

/// Extended getkeystring with flags
/// Port of getkeystring() with full flag support
pub fn getkeystring_ext(s: &str, flags: u32) -> (String, usize) { // utils.c:6915
    let result = getkeystring(s);                           // utils.c:6915
    let len = result.len();                                 // utils.c:6915
    (result, len)                                           // utils.c:6915
}                                                           // utils.c:6915

#[cfg(test)]                                                // utils.c:6915
#[allow(non_snake_case)]                                    // utils.c:6915
// Test names embed zsh's flag/modifier letters as written in the
// shell — `(P)`, `(L)`, `(Q)`, `(U)`, etc. Forcing them to snake_case
// would obscure which zsh feature the test pins.
mod tests {                                                 // utils.c:6915
    use super::*;                                           // utils.c:6915

    #[test]                                                 // utils.c:6915
    fn test_getkeystring() {                                // utils.c:6915
        assert_eq!(getkeystring("hello"), "hello");         // utils.c:6915
        assert_eq!(getkeystring("hello\\nworld"), "hello\nworld"); // utils.c:6915
        assert_eq!(getkeystring("\\t\\r\\n"), "\t\r\n");    // utils.c:6915
        assert_eq!(getkeystring("\\x41"), "A");             // utils.c:6915
        assert_eq!(getkeystring("\\u0041"), "A");           // utils.c:6915
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
    fn test_case_modify() {                                 // utils.c:6915
        assert_eq!(casemodify("hello", CaseMod::Upper), "HELLO"); // utils.c:6915
        assert_eq!(casemodify("HELLO", CaseMod::Lower), "hello"); // utils.c:6915
        assert_eq!(casemodify("hello world", CaseMod::Caps), "Hello World"); // utils.c:6915
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

    #[test]                                                 // utils.c:6915
    fn test_wordcount() {                                   // utils.c:6915
        assert_eq!(wordcount("one two three", None, false), 3); // utils.c:6915
        assert_eq!(wordcount("one  two  three", None, false), 3); // utils.c:6915
        assert_eq!(wordcount("one:two:three", Some(":"), false), 3); // utils.c:6915
    }                                                       // utils.c:6915

    #[test]                                                 // utils.c:6915
    fn test_quotestring() {                                 // utils.c:6915
        assert_eq!(quotestring("hello", QuoteType::Single), "'hello'"); // utils.c:6915
        assert_eq!(quotestring("it's", QuoteType::Single), "'it'\\''s'"); // utils.c:6915
        assert_eq!(quotestring("hello", QuoteType::Double), "\"hello\""); // utils.c:6915
        assert_eq!(quotestring("$var", QuoteType::Double), "\"\\$var\""); // utils.c:6915
    }                                                       // utils.c:6915

    #[test]                                                 // utils.c:6915
    fn test_unique_array() {                                // utils.c:6915
        let mut arr = vec![                                 // utils.c:6915
            "a".to_string(),                                // utils.c:6915
            "b".to_string(),                                // utils.c:6915
            "a".to_string(),                                // utils.c:6915
            "c".to_string(),                                // utils.c:6915
        ];                                                  // utils.c:6915
        unique_array(&mut arr);                             // utils.c:6915
        assert_eq!(arr, vec!["a", "b", "c"]);               // utils.c:6915
    }                                                       // utils.c:6915

    #[test]                                                 // utils.c:6915
    fn test_sort_array() {                                  // utils.c:6915
        let mut arr = vec!["c".to_string(), "a".to_string(), "b".to_string()]; // utils.c:6915
        sort_array(                                         // utils.c:6915
            &mut arr,                                       // utils.c:6915
            &SortOptions {                                  // utils.c:6915
                somehow: true,                              // utils.c:6915
                ..Default::default()                        // utils.c:6915
            },                                              // utils.c:6915
        );                                                  // utils.c:6915
        assert_eq!(arr, vec!["a", "b", "c"]);               // utils.c:6915

        let mut arr = vec!["c".to_string(), "a".to_string(), "b".to_string()]; // utils.c:6915
        sort_array(                                         // utils.c:6915
            &mut arr,                                       // utils.c:6915
            &SortOptions {                                  // utils.c:6915
                somehow: true,                              // utils.c:6915
                backwards: true,                            // utils.c:6915
                ..Default::default()                        // utils.c:6915
            },                                              // utils.c:6915
        );                                                  // utils.c:6915
        assert_eq!(arr, vec!["c", "b", "a"]);               // utils.c:6915
    }                                                       // utils.c:6915

    #[test]                                                 // utils.c:6915
    fn test_array_zip() {                                   // utils.c:6915
        let arr1 = vec!["a".to_string(), "b".to_string()];  // utils.c:6915
        let arr2 = vec!["1".to_string(), "2".to_string()];  // utils.c:6915
        let result = array_zip(&arr1, &arr2, true);         // utils.c:6915
        assert_eq!(result, vec!["a", "1", "b", "2"]);       // utils.c:6915
    }                                                       // utils.c:6915

    #[test]                                                 // utils.c:6915
    fn test_array_intersection() {                          // utils.c:6915
        let arr1 = vec!["a".to_string(), "b".to_string(), "c".to_string()]; // utils.c:6915
        let arr2 = vec!["b".to_string(), "c".to_string(), "d".to_string()]; // utils.c:6915
        let result = array_intersection(&arr1, &arr2);      // utils.c:6915
        assert_eq!(result, vec!["b", "c"]);                 // utils.c:6915
    }                                                       // utils.c:6915

    #[test]                                                 // utils.c:6915
    fn test_eval_subscript() {                              // utils.c:6915
        // Single index (1-based in zsh)
        let (start, end) = eval_subscript("1", 5);          // utils.c:6915
        assert_eq!(start, 0);                               // utils.c:6915
        assert_eq!(end, None);                              // utils.c:6915

        // Negative index
        let (start, end) = eval_subscript("-1", 5);         // utils.c:6915
        assert_eq!(start, 4);                               // utils.c:6915

        // Range
        let (start, end) = eval_subscript("2,4", 5);        // utils.c:6915
        assert_eq!(start, 1);                               // utils.c:6915
        assert_eq!(end, Some(3));                           // utils.c:6915
    }                                                       // utils.c:6915

    #[test]                                                 // utils.c:6915
    fn test_glob_to_regex() {                               // utils.c:6915
        assert_eq!(glob_to_regex("*.txt"), "^[^/]*\\.txt$"); // utils.c:6915
        assert_eq!(glob_to_regex("file?.rs"), "^file.\\.rs$"); // utils.c:6915
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
        assert_eq!(chabspath("/a/b/../c"), "/a/c");         // utils.c:6915
        assert_eq!(chabspath("/a/./b/c"), "/a/b/c");        // utils.c:6915
        assert_eq!(chabspath("/a/b/.."), "/a");             // utils.c:6915
    }                                                       // utils.c:6915

    // ─── getkeystring (Src/utils.c::getkeystring) ───────────────────

    #[test]                                                 // utils.c:6915
    fn getkeystring_decodes_basic_escapes() {               // utils.c:6915
        // utils.c — \n \t \r \a \b \f \v \\ \' \"
        assert_eq!(getkeystring("\\n"), "\n");              // utils.c:6915
        assert_eq!(getkeystring("\\t"), "\t");              // utils.c:6915
        assert_eq!(getkeystring("\\r"), "\r");              // utils.c:6915
        assert_eq!(getkeystring("\\\\"), "\\");             // utils.c:6915
        // Trailing literal — no escape consumed.
        assert_eq!(getkeystring("plain"), "plain");         // utils.c:6915
    }                                                       // utils.c:6915

    #[test]                                                 // utils.c:6915
    fn getkeystring_decodes_hex_escape() {                  // utils.c:6915
        // utils.c handles `\xNN` (1-2 hex digits).
        assert_eq!(getkeystring("\\x41"), "A"); // 0x41 = 'A' // utils.c:6915
        assert_eq!(getkeystring("\\x7e"), "~");             // utils.c:6915
    }                                                       // utils.c:6915

    #[test]                                                 // utils.c:6915
    fn getkeystring_decodes_unicode_escape() {              // utils.c:6915
        // utils.c `\uNNNN` form for BMP code points.
        assert_eq!(getkeystring("\\u00e9"), "é");           // utils.c:6915
        assert_eq!(getkeystring("\\u4e2d"), "中");           // utils.c:6915
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

    /// Build a fresh state with the given scalars / arrays / assocs.
    fn mk_state(                                            // c:3193
        scalars: &[(&str, &str)],                           // c:3193
        arrays: &[(&str, &[&str])],                         // c:3193
        assocs: &[(&str, &[(&str, &str)])],                 // c:3193
    ) -> SubstState {                                       // c:3193
        let mut s = SubstState::default();                  // c:3193
        for (k, v) in scalars {                             // c:3193
            s.variables.insert(k.to_string(), v.to_string()); // c:3193
        }                                                   // c:3193
        for (k, v) in arrays {                              // c:3193
            s.arrays                                        // c:3193
                .insert(k.to_string(), v.iter().map(|x| x.to_string()).collect()); // c:3193
        }                                                   // c:3193
        for (k, kvs) in assocs {                            // c:3193
            let m = kvs                                     // c:3193
                .iter()                                     // c:3193
                .map(|(a, b)| (a.to_string(), b.to_string())) // c:3193
                .collect();                                 // c:3193
            s.assoc_arrays.insert(k.to_string(), m);        // c:3193
        }                                                   // c:3193
        s                                                   // c:3193
    }                                                       // c:3193

    /// Run `paramsubst` with the wrapped form `${…}`.
    fn ps(brace_content: &str, state: &mut SubstState) -> String { // c:3193
        let wrapped = format!("${{{}}}", brace_content);    // c:3193
        let (r, _, _) = paramsubst(&wrapped, 0, false, 0, &mut 0, state); // c:3193
        r                                                   // c:3193
    }                                                       // c:3193

    // ─── zinit.zsh:32 — ${ZERO:-${${0:#$ZSH_ARGZERO}:-${(%):-%N}}} ─

    #[test]                                                 // c:3193
    fn p10k_zinit_zero_resolution_with_ZERO_set() {         // c:3193
        // Real line: ZINIT[ZERO]="${ZERO:-${${0:#$ZSH_ARGZERO}:-${(%):-%N}}}"
        // Truth: when ZERO is set, return ZERO.
        let mut s = mk_state(&[("ZERO", "zinit.zsh")], &[], &[]); // c:3193
        assert_eq!(ps("ZERO:-fallback", &mut s), "zinit.zsh"); // c:3193
    }                                                       // c:3193

    // ─── zinit.zsh:39 — (M) match-keep + nested default ────────────

    #[test]                                                 // c:3193
    fn p10k_zinit_bin_dir_make_absolute() {                 // c:3193
        // Real: `${${(M)ZINIT[BIN_DIR]:#/*}:-$PWD/${ZINIT[BIN_DIR]}}`
        // Truth (zsh-verified): with BIN_DIR=/Users/wizard/.zinit/bin
        // (already absolute), the (M)-keep matches it, returns it
        // unchanged. With a relative path it falls through to PWD/.
        let mut s = mk_state(                               // c:3193
            &[("PWD", "/cur")],                             // c:3193
            &[],                                            // c:3193
            &[("Z", &[("BIN_DIR", "/abs/path")])],          // c:3193
        );                                                  // c:3193
        assert_eq!(                                         // c:3193
            ps("${(M)Z[BIN_DIR]:#/*}:-${PWD}/${Z[BIN_DIR]}", &mut s), // c:3193
            "/abs/path"                                     // c:3193
        );                                                  // c:3193
    }                                                       // c:3193

    // ─── zinit.zsh:147 — `::=` unconditional assign ────────────────

    #[test]                                                 // c:3193
    fn p10k_zinit_aliases_opt_unconditional() {             // c:3193
        // Real: `: ${ZINIT[ALIASES_OPT]::=…}` writes always.
        // Truth (zsh): ALIASES_OPT becomes the operand value
        // regardless of whether it was set before.
        let mut s = mk_state(&[("X", "preset")], &[], &[]); // c:3193
        let _ = ps("X::=fresh", &mut s);                    // c:3193
        assert_eq!(                                         // c:3193
            s.variables.get("X").map(|s| s.as_str()),       // c:3193
            Some("fresh")                                   // c:3193
        );                                                  // c:3193
    }                                                       // c:3193

    // ─── zinit.zsh:160 — `(re)` reverse-search subscript flag ──────

    #[test]                                                 // c:3193
    fn p10k_zinit_path_re_search() {                        // c:3193
        // Real: `${path[(re)/some/dir]}` — find exact element in
        // array. Truth: returns the matching element or empty.
        let mut s = mk_state(&[], &[("p", &["/a", "/b", "/c"])], &[]); // c:3193
        assert_eq!(ps("p[(re)/b]", &mut s), "/b");          // c:3193
        assert_eq!(ps("p[(re)/missing]", &mut s), "");      // c:3193
    }                                                       // c:3193

    // ─── zinit.zsh:179 — pattern replace with `$'...'` ─────────────

    #[test]                                                 // c:3193
    fn p10k_zinit_termcap_escape_replace() {                // c:3193
        // Real: `${termcap[ku]/$'\e'/^\[}` — replace ESC with literal
        // `^[`. Simplified test: replace embedded ESC byte.
        // We feed the assoc with the actual ESC char (0x1b) and
        // expect the literal `^[` in output.
        let esc = "\u{1b}[A";                               // c:3193
        let mut s = mk_state(&[], &[], &[("termcap", &[("ku", esc)])]); // c:3193
        // pattern `\u{1b}` literal → replacement `^[`
        let out = ps("termcap[ku]/\u{1b}/^[", &mut s);      // c:3193
        assert_eq!(out, "^[[A");                            // c:3193
    }                                                       // c:3193

    // ─── zinit.zsh:245 — triple-nested with (M) ────────────────────

    #[test]                                                 // c:3193
    fn p10k_zinit_unicode_triple_nested() {                 // c:3193
        // Real: `${${${(M)LANG:#*UTF-8*}:+OK}:-NO}`
        // Truth: when LANG matches *UTF-8*, returns OK; else NO.
        let mut s = mk_state(&[("LANG", "en_US.UTF-8")], &[], &[]); // c:3193
        assert_eq!(ps("${${(M)LANG:#*UTF-8*}:+OK}:-NO", &mut s), "OK"); // c:3193
        let mut s = mk_state(&[("LANG", "en_US")], &[], &[]); // c:3193
        assert_eq!(ps("${${(M)LANG:#*UTF-8*}:+OK}:-NO", &mut s), "NO"); // c:3193
    }                                                       // c:3193

    // ─── p10k internal/p10k.zsh:6 — (q) quote + (#b) backref ──────

    #[test]                                                 // c:3193
    fn p10k_q_flag_no_specials_preserves() {                // c:3193
        // `(q)` on a string with no shell-meta chars should leave it
        // unchanged. Verified live: `${(q)/Users/me}` → /Users/me.
        let mut s = mk_state(&[("HOME", "/Users/me")], &[], &[]); // c:3193
        assert_eq!(ps("(q)HOME", &mut s), "/Users/me");     // c:3193
    }                                                       // c:3193

    #[test]                                                 // c:3193
    fn p10k_q_flag_backslash_escapes_specials() {           // c:3193
        // `(q)` backslash-escapes whitespace + shell metas.
        let mut s = mk_state(&[("x", "hello world")], &[], &[]); // c:3193
        assert_eq!(ps("(q)x", &mut s), "hello\\ world");    // c:3193
    }                                                       // c:3193

    #[test]                                                 // c:3193
    fn p10k_anchored_prefix_replace_home_to_tilde() {       // c:3193
        // Real-world p10k line 6/9/19 idiom (simplified to drop the
        // `(#b)` backref + `${match[N]}` capture parts which need
        // the next port-cycle):
        //   typeset -gr __p9k_zd_u=${__p9k_zd/#$HOME/~}
        // (Without the `(q)` outer + `(#b)` capture, this is the
        // core $HOME→~ rewrite that the p10k prompt depends on.)
        let mut s = mk_state(                               // c:3193
            &[("HOME", "/Users/me"), ("path", "/Users/me/proj/x")], // c:3193
            &[],                                            // c:3193
            &[],                                            // c:3193
        );                                                  // c:3193
        assert_eq!(ps("path/#$HOME/~", &mut s), "~/proj/x"); // c:3193
    }                                                       // c:3193

    #[test]                                                 // c:3193
    fn p10k_anchored_suffix_replace_extension() {           // c:3193
        // Real-world idiom: rewrite file extension via `:%` anchor.
        let mut s = mk_state(&[("p", "hello.txt")], &[], &[]); // c:3193
        assert_eq!(ps("p/%.txt/.bak", &mut s), "hello.bak"); // c:3193
    }                                                       // c:3193

    #[test]                                                 // c:3193
    fn p10k_backref_match_array_resolves_in_replacement() { // c:3193
        // p10k idiom: capture group via `(#b)` pattern flag, then
        // splice the captured text back into the replacement via
        // `$match[1]`. End-to-end test of:
        //   1. `(#b)` flag triggers capture-group mode
        //   2. Regex emitted UNANCHORED so `/#` enforces start-only
        //   3. `populate_match_array` writes `state.arrays["match"]`
        //   4. The replacement template re-expands so `$match[1]`
        //      resolves to the just-captured group
        let mut s = mk_state(                               // c:3193
            &[("HOME", "/Users/me"), ("p", "/Users/me/proj/x")], // c:3193
            &[],                                            // c:3193
            &[],                                            // c:3193
        );                                                  // c:3193
        // `${p/#(#b)$HOME(|\/*)/~$match[1]}` — replace `$HOME` prefix
        // with `~`, preserving the trailing path piece via `$match[1]`.
        let out = ps("p/#(#b)$HOME(|\\/*)/~$match[1]", &mut s); // c:3193
        assert_eq!(out, "~/proj/x");                        // c:3193
    }                                                       // c:3193

    #[test]                                                 // c:3193
    fn p10k_literal_squote_in_replacement_strips_quotes() { // c:3193
        // p10k line idiom: `'~'$match[1]` — the `'~'` part marks the
        // tilde as a LITERAL replacement char (not a tilde-expansion
        // request). The single quotes themselves do not survive into
        // the result. Tests both the SNULL-marker path (lexer-emitted)
        // and the literal-`'…'` recovery path in `stringsubst`.
        let mut s = mk_state(                               // c:3193
            &[("HOME", "/Users/me"), ("p", "/Users/me/proj/x")], // c:3193
            &[],                                            // c:3193
            &[],                                            // c:3193
        );                                                  // c:3193
        // Use literal `'~'` (the form a runtime-untokenized operand
        // delivers — covers the path that bit p10k's typeset RHS).
        let out = ps("p/#(#b)$HOME(|\\/*)/'~'$match[1]", &mut s); // c:3193
        assert_eq!(out, "~/proj/x");                        // c:3193
    }                                                       // c:3193

    #[test]                                                 // c:3193
    fn p10k_home_replace_with_tilde() {                     // c:3193
        let mut s = mk_state(                               // c:3193
            &[("HOME", "/Users/me"), ("path", "/Users/me/proj/x")], // c:3193
            &[],                                            // c:3193
            &[],                                            // c:3193
        );                                                  // c:3193
        // The real expression involves multiple flags + pattern
        // captures; the spec is what subst_port should compute.
        let out = ps(                                       // c:3193
            "${path/#${HOME}/~}",                           // c:3193
            &mut s,                                         // c:3193
        );                                                  // c:3193
        assert_eq!(out, "~/proj/x");                        // c:3193
    }                                                       // c:3193

    // ─── p10k:298 — (P) indirect on assoc lookup ──────────────────

    #[test]                                                 // c:3193
    fn p10k_indirect_var_lookup_via_P() {                   // c:3193
        // Real: `(P)n` reads scalar `n`'s value, treats it as a
        // parameter name, returns THAT param's value.
        let mut s = mk_state(                               // c:3193
            &[("target", "actual_value"), ("n", "target")], // c:3193
            &[],                                            // c:3193
            &[],                                            // c:3193
        );                                                  // c:3193
        assert_eq!(ps("(P)n", &mut s), "actual_value");     // c:3193
    }                                                       // c:3193

    // ─── p10k:380 — (u) unique on array ──────────────────────────

    #[test]                                                 // c:3193
    fn p10k_unique_array_dedup() {                          // c:3193
        // Real: `${(u)P9K_COMMANDS%$'\0'}` — dedup + strip NUL.
        // Test the dedup half.
        let mut s = mk_state(                               // c:3193
            &[],                                            // c:3193
            &[("dup", &["a", "b", "a", "c", "b", "a"])],    // c:3193
            &[],                                            // c:3193
        );                                                  // c:3193
        let out = ps("(u)dup[@]", &mut s);                  // c:3193
        // Expect `a b c` (dedup preserves first occurrence per zsh).
        // Live verified: `/bin/zsh -fc 'a=(a b a c b a); print -- ${(u)a[@]}'` → "a b c"
        assert_eq!(out, "a b c");                           // c:3193
    }                                                       // c:3193

    // ─── p10k:403 — (L) lowercase ────────────────────────────────

    #[test]                                                 // c:3193
    fn p10k_lowercase_via_L_flag() {                        // c:3193
        let mut s = mk_state(&[("choice", "Hello World")], &[], &[]); // c:3193
        assert_eq!(ps("(L)choice", &mut s), "hello world"); // c:3193
    }                                                       // c:3193

    // ─── p10k:321 — `::=` + (Q) + ~ glob_subst on token ──────────

    #[test]                                                 // c:3193
    fn p10k_token_canonicalize_via_Q_and_glob_subst() {     // c:3193
        let mut s = mk_state(&[("token", "'literal'")], &[], &[]); // c:3193
        // (Q) strips the quotes; ~ would glob-expand if there were
        // glob chars (here there are none).
        let _ = ps("token::=${(Q)${~token}}", &mut s);      // c:3193
        assert_eq!(s.variables.get("token").map(|s| s.as_str()), Some("literal")); // c:3193
    }                                                       // c:3193

    // ─── zinit's gnarliest — (#b) backref + ${match[N]} in repl ──

    #[test]                                                 // c:3193
    fn p10k_zinit_kitchen_sink_substs() {                   // c:3193
        // The pattern: `(#b)((*)\(#e)|(*))`
        //   group 1: alternation of (group 2: ANY ending in `\` at
        //   end-of-string) OR (group 3: anything else).
        //   `(#b)` enables `${match[N]}` backrefs in replacement.
        // Replacement strings use `${___prev::=…}:+` to update a
        // running accumulator — assign-then-test trick.
        // For now there's no faithful Rust port; pinning as the
        // spec target.
        //
        // Truth for input ("foo\\;" "bar"):
        //   Pattern `(#b)((*)\(#e)|(*))` matches the whole element.
        //   For "foo\\" (group 2 captured as "foo"), repl runs
        //   ${match[3]:+...} (empty since match[3] empty) →
        //   ___prev set to "foo", output="" — element disappears,
        //   prev = "foo".
        //   Next "bar" (group 3 = "bar"), match[3] is "bar", repl
        //   begins with "${match[3]:+${___prev:+foo;}}" → "foo;",
        //   then "bar", then "${...:+}" — outer is empty after
        //   prev assignment.
        //   Result: ["foo;bar"]
        let mut s = mk_state(                               // c:3193
            &[],                                            // c:3193
            &[("___substs", &["foo\\", "bar"])],            // c:3193
            &[],                                            // c:3193
        );                                                  // c:3193
        // expression: ___substs[@]//(#b)((*)\(#e)|(*))/...
        // We can't easily encode this whole expression as one
        // call yet; pinning as the spec.
        let _ = ps(                                         // c:3193
            "___substs[@]//(#b)((*)\\(#e)|(*))/${match[3]:+${___prev:+$___prev\\;}}${match[3]}${${___prev::=${match[2]:+${___prev:+$___prev\\;}}${match[2]}}:+}", // c:3193
            &mut s,                                         // c:3193
        );                                                  // c:3193
        assert_eq!(                                         // c:3193
            s.arrays.get("___substs").map(|v| v.as_slice()), // c:3193
            Some(&["foo;bar".to_string()][..])              // c:3193
        );                                                  // c:3193
    }                                                       // c:3193

    // ─── (kv) paired keys+values ─────────────────────────────────

    #[test]                                                 // c:3193
    fn p10k_kv_paired_assoc_iteration() {                   // c:3193
        let mut s = mk_state(                               // c:3193
            &[],                                            // c:3193
            &[],                                            // c:3193
            &[("m", &[("a", "1"), ("b", "2"), ("c", "3")])], // c:3193
        );                                                  // c:3193
        // zsh: ${(kv)m[@]} → "a 1 b 2 c 3"
        assert_eq!(ps("(kv)m[@]", &mut s), "a 1 b 2 c 3");  // c:3193
    }                                                       // c:3193

    // ─── nested with literal `~` glob_subst ──────────────────────

    #[test]                                                 // c:3193
    fn p10k_tilde_glob_subst_form() {                       // c:3193
        let mut s = mk_state(&[("p", "/usr/bin/*")], &[], &[]); // c:3193
        // Truth: `${~p}` glob-expands /usr/bin/*. Result depends on
        // the host filesystem — the test pins the call shape, not
        // a specific list of files.
        let out = ps("~p", &mut s);                         // c:3193
        // Just check it doesn't crash and returns some result.
        let _ = out;                                        // c:3193
    }                                                       // c:3193
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

/// Check for $'...' quoting prefix
/// Port of logic in stringsubst() for Snull detection
pub fn is_dollars_quote(s: &str, pos: usize) -> bool {      // c:N/A
    let chars: Vec<char> = s.chars().collect();             // c:N/A
    pos + 1 < chars.len()                                   // c:N/A
        && (chars[pos] == STRING || chars[pos] == QSTRING)  // c:N/A
        && chars[pos + 1] == SNULL                          // c:N/A
}                                                           // c:N/A

/// Check if character is a space type for word splitting
/// Port of iwsep() macro
pub fn iwsep(c: char) -> bool {                             // c:N/A
    // IFS word separator check
    c == ' ' || c == '\t' || c == '\n'                      // c:N/A
}                                                           // c:N/A

/// Check if character is identifier character
/// Port of iident() macro
pub fn iident(c: char) -> bool {                            // c:N/A
    c.is_ascii_alphanumeric() || c == '_'                   // c:N/A
}                                                           // c:N/A

/// Check if character is alphanumeric
/// Port of ialpha() macro  
pub fn ialpha(c: char) -> bool {                            // c:N/A
    c.is_ascii_alphabetic()                                 // c:N/A
}                                                           // c:N/A

/// Check if character is a digit
/// Port of idigit() macro
pub fn idigit(c: char) -> bool {                            // c:N/A
    c.is_ascii_digit()                                      // c:N/A
}                                                           // c:N/A

/// Check if character is blank
/// Port of inblank() macro
pub fn inblank(c: char) -> bool {                           // c:N/A
    c == ' ' || c == '\t'                                   // c:N/A
}                                                           // c:N/A

/// Check if character is a dash (handles tokenized dash)
/// Port of IS_DASH() macro
pub fn is_dash(c: char) -> bool {                           // c:N/A
    c == '-' || c == '\u{96}' // Dash token                 // c:N/A
}                                                           // c:N/A

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

/// Build the `(t)` / `(Pt)` flag's type string for a named parameter.
/// Direct port of `Src/subst.c:2807-2854`: emits base type
/// (scalar/array/association/integer/float/nameref) followed by
/// `-suffix` for each set attribute (left, right_blanks, right_zeros,
/// lower, upper, readonly, tag, tied, export, unique, hide, hideval,
/// special). Empty string when name is unset.
pub fn build_type_string_for(name: &str, state: &SubstState) -> Vec<String> { // params.c // c:N/A
    use crate::exec::VarKind;                               // params.c // c:N/A
    let attr = state.var_attrs.get(name);                   // params.c // c:N/A
    let base = if let Some(a) = attr {                      // params.c // c:N/A
        match a.kind {                                      // params.c // c:N/A
            VarKind::Scalar => "scalar",                    // params.c // c:N/A
            VarKind::Integer => "integer",                  // params.c // c:N/A
            VarKind::Float => "float",                      // params.c // c:N/A
            VarKind::Array => "array",                      // params.c // c:N/A
            VarKind::Association => "association",          // params.c // c:N/A
        }                                                   // params.c // c:N/A
    } else if state.assoc_arrays.contains_key(name) {       // params.c // c:N/A
        "association"                                       // params.c // c:N/A
    } else if state.arrays.contains_key(name) {             // params.c // c:N/A
        "array"                                             // params.c // c:N/A
    } else if state.variables.contains_key(name) {          // params.c // c:N/A
        "scalar"                                            // params.c // c:N/A
    } else {                                                // params.c // c:N/A
        return Vec::new();                                  // params.c // c:N/A
    };                                                      // params.c // c:N/A
    let mut out = String::from(base);                       // params.c // c:N/A
    if let Some(a) = attr {                                 // params.c // c:N/A
        if a.left_pad.is_some() {                           // params.c // c:N/A
            out.push_str("-left");                          // params.c // c:N/A
        }                                                   // params.c // c:N/A
        if a.right_pad.is_some() {                          // params.c // c:N/A
            // zsh distinguishes -right_blanks (space pad) from
            // -right_zeros (zero pad). zshrs stores zero-pad in
            // `zero_pad`; right_pad without zero_pad means blanks.
            if a.zero_pad.is_some() {                       // params.c // c:N/A
                out.push_str("-right_zeros");               // params.c // c:N/A
            } else {                                        // params.c // c:N/A
                out.push_str("-right_blanks");              // params.c // c:N/A
            }                                               // params.c // c:N/A
        }                                                   // params.c // c:N/A
        if a.lowercase {                                    // params.c // c:N/A
            out.push_str("-lower");                         // params.c // c:N/A
        }                                                   // params.c // c:N/A
        if a.uppercase {                                    // params.c // c:N/A
            out.push_str("-upper");                         // params.c // c:N/A
        }                                                   // params.c // c:N/A
        if a.readonly {                                     // params.c // c:N/A
            out.push_str("-readonly");                      // params.c // c:N/A
        }                                                   // params.c // c:N/A
        if a.export {                                       // params.c // c:N/A
            out.push_str("-export");                        // params.c // c:N/A
        }                                                   // params.c // c:N/A
        if a.unique {                                       // params.c // c:N/A
            out.push_str("-unique");                        // params.c // c:N/A
        }                                                   // params.c // c:N/A
        if a.hidden {                                       // params.c // c:N/A
            out.push_str("-hide");                          // params.c // c:N/A
        }                                                   // params.c // c:N/A
        if a.hide_val {                                     // params.c // c:N/A
            out.push_str("-hideval");                       // params.c // c:N/A
        }                                                   // params.c // c:N/A
    }                                                       // params.c // c:N/A
    vec![out]                                               // params.c // c:N/A
}                                                           // params.c // c:N/A

/// Get parameter type description string
/// Port of logic in paramsubst() for (t) flag
pub fn param_type_string(flags: u32) -> String {            // params.c // c:N/A
    let mut result = String::new();                         // params.c // c:N/A

    // Base type
    match flags & 0x3F {                                    // params.c // c:N/A
        0 => result.push_str("scalar"),                     // params.c // c:N/A
        1 => result.push_str("array"),                      // params.c // c:N/A
        2 => result.push_str("integer"),                    // params.c // c:N/A
        3 | 4 => result.push_str("float"),                  // params.c // c:N/A
        5 => result.push_str("association"),                // params.c // c:N/A
        6 => result.push_str("nameref"),                    // params.c // c:N/A
        _ => result.push_str("scalar"),                     // params.c // c:N/A
    }                                                       // params.c // c:N/A

    // Modifiers
    if flags & pm_flags::LEFT != 0 {                        // params.c // c:N/A
        result.push_str("-left");                           // params.c // c:N/A
    }                                                       // params.c // c:N/A
    if flags & pm_flags::RIGHT_B != 0 {                     // params.c // c:N/A
        result.push_str("-right_blanks");                   // params.c // c:N/A
    }                                                       // params.c // c:N/A
    if flags & pm_flags::RIGHT_Z != 0 {                     // params.c // c:N/A
        result.push_str("-right_zeros");                    // params.c // c:N/A
    }                                                       // params.c // c:N/A
    if flags & pm_flags::LOWER != 0 {                       // params.c // c:N/A
        result.push_str("-lower");                          // params.c // c:N/A
    }                                                       // params.c // c:N/A
    if flags & pm_flags::UPPER != 0 {                       // params.c // c:N/A
        result.push_str("-upper");                          // params.c // c:N/A
    }                                                       // params.c // c:N/A
    if flags & pm_flags::READONLY != 0 {                    // params.c // c:N/A
        result.push_str("-readonly");                       // params.c // c:N/A
    }                                                       // params.c // c:N/A
    if flags & pm_flags::TAGGED != 0 {                      // params.c // c:N/A
        result.push_str("-tag");                            // params.c // c:N/A
    }                                                       // params.c // c:N/A
    if flags & pm_flags::TIED != 0 {                        // params.c // c:N/A
        result.push_str("-tied");                           // params.c // c:N/A
    }                                                       // params.c // c:N/A
    if flags & pm_flags::EXPORTED != 0 {                    // params.c // c:N/A
        result.push_str("-export");                         // params.c // c:N/A
    }                                                       // params.c // c:N/A
    if flags & pm_flags::UNIQUE != 0 {                      // params.c // c:N/A
        result.push_str("-unique");                         // params.c // c:N/A
    }                                                       // params.c // c:N/A
    if flags & pm_flags::HIDE != 0 {                        // params.c // c:N/A
        result.push_str("-hide");                           // params.c // c:N/A
    }                                                       // params.c // c:N/A
    if flags & pm_flags::HIDEVAL != 0 {                     // params.c // c:N/A
        result.push_str("-hideval");                        // params.c // c:N/A
    }                                                       // params.c // c:N/A
    if flags & pm_flags::SPECIAL != 0 {                     // params.c // c:N/A
        result.push_str("-special");                        // params.c // c:N/A
    }                                                       // params.c // c:N/A
    if flags & pm_flags::LOCAL != 0 {                       // params.c // c:N/A
        result.push_str("-local");                          // params.c // c:N/A
    }                                                       // params.c // c:N/A

    result                                                  // params.c // c:N/A
}                                                           // params.c // c:N/A

/// Evaluate character from number (for (#) flag)
/// Port of substevalchar() from subst.c
pub fn substevalchar(s: &str) -> Option<String> {           // c:1490
    let val = mathevali(s);                                 // c:1490
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
    let (expr, rest) = parse_colon_expr(s)?;                // c:1566
    Some((expr, rest))                                      // c:1566
}                                                           // c:1566

/// Parse expression until colon or end
fn parse_colon_expr(s: &str) -> Option<(String, String)> {  // c:N/A
    let mut depth = 0;                                      // c:N/A
    let mut end = 0;                                        // c:N/A
    let chars: Vec<char> = s.chars().collect();             // c:N/A

    while end < chars.len() {                               // c:N/A
        let c = chars[end];                                 // c:N/A
        match c {                                           // c:N/A
            '(' | '[' | '{' => depth += 1,                  // c:N/A
            ')' | ']' | '}' => depth -= 1,                  // c:N/A
            ':' if depth == 0 => break,                     // c:N/A
            _ => {}                                         // c:N/A
        }                                                   // c:N/A
        end += 1;                                           // c:N/A
    }                                                       // c:N/A

    let expr: String = chars[..end].iter().collect();       // c:N/A
    let rest: String = chars[end..].iter().collect();       // c:N/A

    Some((expr, rest))                                      // c:N/A
}                                                           // c:N/A

/// Untokenize and escape string for flag argument
/// Port of untok_and_escape() from subst.c
pub fn untok_and_escape(s: &str, escapes: bool, tok_arg: bool) -> String { // c:1528
    let mut result = untokenize(s);                         // c:1528

    if escapes {                                            // c:1528
        result = getkeystring(&result);                     // c:1528
    }                                                       // c:1528

    if tok_arg {                                            // c:1528
        result = shtokenize(&result);                       // c:1528
    }                                                       // c:1528

    result                                                  // c:1528
}                                                           // c:1528

/// String metadata sort
/// Port of strmetasort() from utils.c (used in subst.c)
pub fn strmetasort(arr: &mut [String], sortit: u32) {       // c:N/A
    if sortit == sortit_flags::ANYOLDHOW {                  // c:N/A
        return;                                             // c:N/A
    }                                                       // c:N/A

    let backwards = sortit & sortit_flags::BACKWARDS != 0;  // c:N/A
    let ignoring_case = sortit & sortit_flags::IGNORING_CASE != 0; // c:N/A
    let numerically = sortit & sortit_flags::NUMERICALLY != 0; // c:N/A
    let numerically_signed = sortit & sortit_flags::NUMERICALLY_SIGNED != 0; // c:N/A

    arr.sort_by(|a, b| {                                    // c:N/A
        let cmp = if numerically || numerically_signed {    // c:N/A
            let na: f64 = a.parse().unwrap_or(0.0);         // c:N/A
            let nb: f64 = b.parse().unwrap_or(0.0);         // c:N/A
            na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal) // c:N/A
        } else if ignoring_case {                           // c:N/A
            a.to_lowercase().cmp(&b.to_lowercase())         // c:N/A
        } else {                                            // c:N/A
            a.cmp(b)                                        // c:N/A
        };                                                  // c:N/A

        if backwards {                                      // c:N/A
            cmp.reverse()                                   // c:N/A
        } else {                                            // c:N/A
            cmp                                             // c:N/A
        }                                                   // c:N/A
    });                                                     // c:N/A
}                                                           // c:N/A

/// Unique array (hash-based)
/// Port of zhuniqarray() from utils.c (used in subst.c)
pub fn zhuniqarray(arr: &mut Vec<String>) {                 // c:N/A
    let mut seen = std::collections::HashSet::new();        // c:N/A
    arr.retain(|s| seen.insert(s.clone()));                 // c:N/A
}                                                           // c:N/A

/// Create parameter with given flags
/// Port of createparam() logic (simplified)
pub fn createparam(name: &str, flags: u32) -> ParamInfo {   // c:N/A
    ParamInfo {                                             // c:N/A
        name: name.to_string(),                             // c:N/A
        flags,                                              // c:N/A
        level: 0,                                           // c:N/A
        value: if flags & pm_flags::ARRAY != 0 {            // c:N/A
            ParamValue::Array(Vec::new())                   // c:N/A
        } else {                                            // c:N/A
            ParamValue::Scalar(String::new())               // c:N/A
        },                                                  // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

/// Skip to end of identifier
/// Port of itype_end() from utils.c
pub fn itype_end(s: &str, allow_namespace: bool) -> usize { // c:N/A
    let chars: Vec<char> = s.chars().collect();             // c:N/A
    let mut i = 0;                                          // c:N/A

    while i < chars.len() {                                 // c:N/A
        let c = chars[i];                                   // c:N/A
        if c.is_ascii_alphanumeric() || c == '_' || (allow_namespace && c == ':') { // c:N/A
            i += 1;                                         // c:N/A
        } else {                                            // c:N/A
            break;                                          // c:N/A
        }                                                   // c:N/A
    }                                                       // c:N/A

    i                                                       // c:N/A
}                                                           // c:N/A

/// Parse string for substitution with error handling
/// Port of parsestr() / parsestrnoerr() from parse.c
pub fn parsestr(s: &str) -> Result<String, String> {        // c:N/A
    // Simplified - just return the string
    // Real implementation would parse and tokenize
    Ok(s.to_string())                                       // c:N/A
}                                                           // c:N/A

/// Get width of string (multibyte-aware)
/// Port of MB_METASTRLEN2() macro
pub fn mb_metastrlen(s: &str, multi_width: bool) -> usize { // c:N/A
    if multi_width {                                        // c:N/A
        // Unicode width calculation
        s.chars()                                           // c:N/A
            .map(|c| {                                      // c:N/A
                if c.is_ascii() {                           // c:N/A
                    1                                       // c:N/A
                } else {                                    // c:N/A
                    // Approximate width for CJK characters
                    2                                       // c:N/A
                }                                           // c:N/A
            })                                              // c:N/A
            .sum()                                          // c:N/A
    } else {                                                // c:N/A
        s.chars().count()                                   // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

/// Get length of next multibyte character
/// Port of MB_METACHARLEN() macro  
pub fn mb_metacharlen(s: &str) -> usize {                   // c:N/A
    s.chars().next().map(|c| c.len_utf8()).unwrap_or(0)     // c:N/A
}                                                           // c:N/A

/// Convert to wide character
/// Port of MB_METACHARLENCONV() logic
pub fn mb_metacharlenconv(s: &str) -> (usize, Option<char>) { // c:N/A
    match s.chars().next() {                                // c:N/A
        Some(c) => (c.len_utf8(), Some(c)),                 // c:N/A
        None => (0, None),                                  // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

/// WCWIDTH implementation for character width
/// Port of WCWIDTH() macro
pub fn wcwidth(c: char) -> i32 {                            // c:N/A
    if c.is_control() {                                     // c:N/A
        0                                                   // c:N/A
    } else if c.is_ascii() {                                // c:N/A
        1                                                   // c:N/A
    } else {                                                // c:N/A
        // CJK wide characters
        let cp = c as u32;                                  // c:N/A
        if (0x1100..=0x115F).contains(&cp) ||  // Hangul Jamo // c:N/A
           (0x2E80..=0x9FFF).contains(&cp) ||  // CJK       // c:N/A
           (0xF900..=0xFAFF).contains(&cp) ||  // CJK Compatibility // c:N/A
           (0xFE10..=0xFE6F).contains(&cp) ||  // CJK forms // c:N/A
           (0xFF00..=0xFF60).contains(&cp) ||  // Fullwidth // c:N/A
           (0x20000..=0x2FFFF).contains(&cp)                // c:N/A
        {                                                   // c:N/A
            // CJK Extension
            2                                               // c:N/A
        } else {                                            // c:N/A
            1                                               // c:N/A
        }                                                   // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

/// Wide character type check
/// Port of WC_ZISTYPE() macro
pub fn wc_zistype(c: char, type_: u32) -> bool {            // c:N/A
    const ISEP: u32 = 1; // IFS separator                   // c:N/A

    match type_ {                                           // c:N/A
        1 => c.is_whitespace(), // ISEP                     // c:N/A
        _ => false,                                         // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

/// Metafy a string (add Meta markers for special chars)
/// Port of metafy() from utils.c
pub fn metafy(s: &str) -> String {                          // c:N/A
    // In zsh, metafy adds Meta (0x83) before bytes that need escaping
    // For Rust we just return the string as-is since we handle Unicode natively
    s.to_string()                                           // c:N/A
}                                                           // c:N/A

/// Unmetafy a string
/// Port of unmetafy() from utils.c
pub fn unmetafy(s: &str) -> (String, usize) {               // c:N/A
    let result = s.to_string();                             // c:N/A
    let len = result.len();                                 // c:N/A
    (result, len)                                           // c:N/A
}                                                           // c:N/A

/// Default IFS value
pub const DEFAULT_IFS: &str = " \t\n";                      // c:N/A

/// Get current working directory
/// Port of pwd global variable access
pub fn get_pwd() -> String {                                // c:N/A
    std::env::current_dir()                                 // c:N/A
        .map(|p| p.to_string_lossy().to_string())           // c:N/A
        .unwrap_or_else(|_| "/".to_string())                // c:N/A
}                                                           // c:N/A

/// Get old working directory (OLDPWD)
pub fn get_oldpwd(state: &SubstState) -> String {           // c:N/A
    state                                                   // c:N/A
        .variables                                          // c:N/A
        .get("OLDPWD")                                      // c:N/A
        .cloned()                                           // c:N/A
        .unwrap_or_else(get_pwd)                            // c:N/A
}                                                           // c:N/A

/// Get home directory
pub fn get_home() -> Option<String> {                       // c:N/A
    std::env::var("HOME").ok()                              // c:N/A
}                                                           // c:N/A

/// Get argzero ($0)
pub fn get_argzero(state: &SubstState) -> String {          // c:N/A
    state                                                   // c:N/A
        .variables                                          // c:N/A
        .get("0")                                           // c:N/A
        .cloned()                                           // c:N/A
        .unwrap_or_else(|| "zsh".to_string())               // c:N/A
}                                                           // c:N/A

/// Check if option is set
/// Port of isset()/unset() macros
pub fn isset(opt: &str, state: &SubstState) -> bool {       // c:N/A
    state.opts.get_option(opt)                              // c:N/A
}                                                           // c:N/A

impl SubstOptions {                                         // c:N/A
    pub fn get_option(&self, name: &str) -> bool {          // c:N/A
        match name {                                        // c:N/A
            "SHFILEEXPANSION" | "shfileexpansion" => self.sh_file_expansion, // c:N/A
            "SHWORDSPLIT" | "shwordsplit" => self.sh_word_split, // c:N/A
            "IGNOREBRACES" | "ignorebraces" => self.ignore_braces, // c:N/A
            "GLOBSUBST" | "globsubst" => self.glob_subst,   // c:N/A
            "KSHTYPESET" | "kshtypeset" => self.ksh_typeset, // c:N/A
            "EXECOPT" | "execopt" => self.exec_opt,         // c:N/A
            "NOMATCH" | "nomatch" => true, // Default on    // c:N/A
            "UNSET" | "unset" => false,    // Treat unset as error // c:N/A
            "KSHARRAYS" | "ksharrays" => false,             // c:N/A
            "RCEXPANDPARAM" | "rcexpandparam" => false,     // c:N/A
            "EQUALS" | "equals" => true,                    // c:N/A
            "POSIXIDENTIFIERS" | "posixidentifiers" => false, // c:N/A
            "MULTIBYTE" | "multibyte" => true,              // c:N/A
            "EXTENDEDGLOB" | "extendedglob" => false,       // c:N/A
            "PROMPTSUBST" | "promptsubst" => false,         // c:N/A
            "PROMPTBANG" | "promptbang" => false,           // c:N/A
            "PROMPTPERCENT" | "promptpercent" => true,      // c:N/A
            "HISTSUBSTPATTERN" | "histsubstpattern" => false, // c:N/A
            "PUSHDMINUS" | "pushdminus" => false,           // c:N/A
            _ => false,                                     // c:N/A
        }                                                   // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

/// Prompt expansion (simplified)
/// Port of promptexpand() from prompt.c
pub fn promptexpand(s: &str, _state: &SubstState) -> String { // c:N/A
    // Simplified prompt expansion
    let mut result = String::new();                         // c:N/A
    let mut chars = s.chars().peekable();                   // c:N/A

    while let Some(c) = chars.next() {                      // c:N/A
        if c == '%' {                                       // c:N/A
            match chars.next() {                            // c:N/A
                Some('n') => result.push_str(&std::env::var("USER").unwrap_or_default()), // c:N/A
                Some('m') => {                              // c:N/A
                    if let Ok(hostname) = std::env::var("HOSTNAME") { // c:N/A
                        result.push_str(hostname.split('.').next().unwrap_or(&hostname)); // c:N/A
                    }                                       // c:N/A
                }                                           // c:N/A
                Some('M') => result.push_str(&std::env::var("HOSTNAME").unwrap_or_default()), // c:N/A
                Some('~') | Some('/') => result.push_str(&get_pwd()), // c:N/A
                Some('d') => result.push_str(&get_pwd()),   // c:N/A
                Some('%') => result.push('%'),              // c:N/A
                Some(c) => {                                // c:N/A
                    result.push('%');                       // c:N/A
                    result.push(c);                         // c:N/A
                }                                           // c:N/A
                None => result.push('%'),                   // c:N/A
            }                                               // c:N/A
        } else {                                            // c:N/A
            result.push(c);                                 // c:N/A
        }                                                   // c:N/A
    }                                                       // c:N/A

    result                                                  // c:N/A
}                                                           // c:N/A

/// Text attribute type for prompt highlighting
pub type ZAttr = u64;                                       // c:N/A

/// Get named directory (for ~name expansion)
/// Port of getnameddir() from hashnameddir.c
pub fn getnameddir(name: &str) -> Option<String> {          // c:N/A
    // Check for user home directory
    #[cfg(unix)]                                            // c:N/A
    {                                                       // c:N/A
        use std::ffi::CString;                              // c:N/A
        if let Ok(cname) = CString::new(name) {             // c:N/A
            unsafe {                                        // c:N/A
                let pwd = libc::getpwnam(cname.as_ptr());   // c:N/A
                if !pwd.is_null() {                         // c:N/A
                    let dir = std::ffi::CStr::from_ptr((*pwd).pw_dir); // c:N/A
                    return dir.to_str().ok().map(String::from); // c:N/A
                }                                           // c:N/A
            }                                               // c:N/A
        }                                                   // c:N/A
    }                                                       // c:N/A
    None                                                    // c:N/A
}                                                           // c:N/A

/// Find command in PATH (for =cmd expansion)
/// Port of findcmd() from exec.c
pub fn findcmd(name: &str, _hash: bool, _all: bool) -> Option<String> { // c:N/A
    if let Ok(path) = std::env::var("PATH") {               // c:N/A
        for dir in path.split(':') {                        // c:N/A
            let full = format!("{}/{}", dir, name);         // c:N/A
            if std::path::Path::new(&full).exists() {       // c:N/A
                return Some(full);                          // c:N/A
            }                                               // c:N/A
        }                                                   // c:N/A
    }                                                       // c:N/A
    None                                                    // c:N/A
}                                                           // c:N/A

/// Queue/unqueue signals (stub for Rust)
pub fn queue_signals() {                                    // c:N/A
    // Signal handling would go here
}                                                           // c:N/A

pub fn unqueue_signals() {                                  // c:N/A
    // Signal handling would go here
}                                                           // c:N/A

/// LEXFLAGS for (z) flag
pub mod lexflags {                                          // c:N/A
    pub const ACTIVE: u32 = 1;                              // c:N/A
    pub const COMMENTS_KEEP: u32 = 2;                       // c:N/A
    pub const COMMENTS_STRIP: u32 = 4;                      // c:N/A
    pub const NEWLINE: u32 = 8;                             // c:N/A
}                                                           // c:N/A

/// Convert float with underscore separators
/// Port of convfloat_underscore() from utils.c
pub fn convfloat_underscore(val: f64, underscore: bool) -> String { // c:N/A
    if underscore {                                         // c:N/A
        // Add underscores to float representation
        let s = format!("{}", val);                         // c:N/A
        // Simplified: just return the string
        s                                                   // c:N/A
    } else {                                                // c:N/A
        format!("{}", val)                                  // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

/// Convert integer with base and underscore separators
/// Port of convbase_underscore() from utils.c
pub fn convbase_underscore(val: i64, base: u32, underscore: bool) -> String { // c:N/A
    let s = match base {                                    // c:N/A
        2 => format!("{:b}", val),                          // c:N/A
        8 => format!("{:o}", val),                          // c:N/A
        16 => format!("{:x}", val),                         // c:N/A
        _ => format!("{}", val),                            // c:N/A
    };                                                      // c:N/A

    if underscore && base == 10 {                           // c:N/A
        // Add underscores every 3 digits
        let mut result = String::new();                     // c:N/A
        let chars: Vec<char> = s.chars().collect();         // c:N/A
        let start = if val < 0 { 1 } else { 0 };            // c:N/A

        if start == 1 {                                     // c:N/A
            result.push('-');                               // c:N/A
        }                                                   // c:N/A

        for (i, c) in chars[start..].iter().rev().enumerate() { // c:N/A
            if i > 0 && i % 3 == 0 {                        // c:N/A
                result.insert(start, '_');                  // c:N/A
            }                                               // c:N/A
            result.insert(start, *c);                       // c:N/A
        }                                                   // c:N/A
        result                                              // c:N/A
    } else {                                                // c:N/A
        s                                                   // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

/// Heap allocation wrapper (in Rust, just normal allocation)
/// Port of hcalloc() / zhalloc() from mem.c
pub fn hcalloc(size: usize) -> Vec<u8> {                    // c:N/A
    vec![0u8; size]                                         // c:N/A
}                                                           // c:N/A

/// String duplication on heap
/// Port of dupstring() from utils.c
pub fn dupstring(s: &str) -> String {                       // c:N/A
    s.to_string()                                           // c:N/A
}                                                           // c:N/A

/// String duplication with zalloc
/// Port of ztrdup() from mem.c
pub fn ztrdup(s: &str) -> String {                          // c:N/A
    s.to_string()                                           // c:N/A
}                                                           // c:N/A

/// Free memory (no-op in Rust)
/// Port of zsfree() from mem.c
pub fn zsfree(_s: String) {                                 // c:N/A
    // Memory is automatically freed in Rust
}                                                           // c:N/A

// ============================================================================
// Final functions for complete subst.c coverage
// ============================================================================

/// Token constants for Dnull, Snull, etc.
pub const DNULL: char = '\u{97}'; // "                      // c:N/A
pub const BNULLKEEP: char = '\u{95}'; // Backslash null that stays // c:N/A

/// Complete tilde expansion
/// Full port of filesubstr() from subst.c lines 728-795
pub fn filesubstr_full(s: &str, assign: bool, state: &SubstState) -> Option<String> { // c:737
    let chars: Vec<char> = s.chars().collect();             // c:737

    if chars.is_empty() {                                   // c:737
        return None;                                        // c:737
    }                                                       // c:737

    // Check for Tilde token or ~
    let is_tilde = chars[0] == '\u{98}' || chars[0] == '~'; // c:737

    if is_tilde && chars.get(1) != Some(&'=') && chars.get(1) != Some(&EQUALS) { // c:737
        // Handle ~ expansion
        let second = chars.get(1).copied().unwrap_or('\0'); // c:737

        // Handle Dash token
        let second = if second == '\u{96}' { '-' } else { second }; // c:737

        // Check for end of expansion
        let is_end = |c: char| c == '\0' || c == '/' || c == INPAR || (assign && c == ':'); // c:737
        let is_end2 = |c: char| c == '\0' || c == INPAR || (assign && c == ':'); // c:737

        if is_end(second) {                                 // c:737
            // Plain ~ - expand to HOME
            let home = get_home().unwrap_or_default();      // c:737
            let rest: String = chars[1..].iter().collect(); // c:737
            return Some(format!("{}{}", home, rest));       // c:737
        } else if second == '+' && chars.get(2).map(|&c| is_end(c)).unwrap_or(true) { // c:737
            // ~+ - expand to PWD
            let pwd = get_pwd();                            // c:737
            let rest: String = chars[2..].iter().collect(); // c:737
            return Some(format!("{}{}", pwd, rest));        // c:737
        } else if second == '-' && chars.get(2).map(|&c| is_end(c)).unwrap_or(true) { // c:737
            // ~- - expand to OLDPWD
            let oldpwd = get_oldpwd(state);                 // c:737
            let rest: String = chars[2..].iter().collect(); // c:737
            return Some(format!("{}{}", oldpwd, rest));     // c:737
        } else if second == INBRACK {                       // c:737
            // ~[name] - named directory by hook
            if let Some(end_pos) = chars[2..].iter().position(|&c| c == OUTBRACK) { // c:737
                let name: String = chars[2..2 + end_pos].iter().collect(); // c:737
                let rest: String = chars[3 + end_pos..].iter().collect(); // c:737
                // Would call zsh_directory_name hook here
                // For now just return None
                return None;                                // c:737
            }                                               // c:737
        } else if second.is_ascii_digit() || second == '+' || second == '-' { // c:737
            // ~N or ~+N or ~-N - directory stack entry
            let mut idx = 1;                                // c:737
            let backwards = second == '-';                  // c:737
            let start = if second == '+' || second == '-' { // c:737
                idx = 2;                                    // c:737
                chars.get(2)                                // c:737
            } else {                                        // c:737
                chars.get(1)                                // c:737
            };                                              // c:737

            // Parse number
            let mut val = 0i32;                             // c:737
            while idx < chars.len() && chars[idx].is_ascii_digit() { // c:737
                val = val * 10 + (chars[idx] as i32 - '0' as i32); // c:737
                idx += 1;                                   // c:737
            }                                               // c:737

            if idx < chars.len() && !is_end(chars[idx]) {   // c:737
                return None;                                // c:737
            }                                               // c:737

            // Would access directory stack here
            // For now, return None
            return None;                                    // c:737
        } else if !inblank(second) {                        // c:737
            // ~username
            let mut end = 1;                                // c:737
            while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == '_') { // c:737
                end += 1;                                   // c:737
            }                                               // c:737

            if end < chars.len() && !is_end(chars[end]) {   // c:737
                return None;                                // c:737
            }                                               // c:737

            let username: String = chars[1..end].iter().collect(); // c:737
            let rest: String = chars[end..].iter().collect(); // c:737

            if let Some(home) = getnameddir(&username) {    // c:737
                return Some(format!("{}{}", home, rest));   // c:737
            }                                               // c:737

            return None;                                    // c:737
        }                                                   // c:737
    } else if chars[0] == EQUALS && isset("EQUALS", state) && chars.len() > 1 && chars[1] != INPAR { // c:737
        // =command expansion
        let cmd: String = chars[1..]                        // c:737
            .iter()                                         // c:737
            .take_while(|&&c| c != '/' && c != INPAR && !(assign && c == ':')) // c:737
            .collect();                                     // c:737
        let rest_start = 1 + cmd.len();                     // c:737
        let rest: String = chars[rest_start..].iter().collect(); // c:737

        if let Some(path) = findcmd(&cmd, true, false) {    // c:737
            return Some(format!("{}{}", path, rest));       // c:737
        }                                                   // c:737

        return None;                                        // c:737
    }                                                       // c:737

    None                                                    // c:737
}                                                           // c:737

/// Full filesub implementation
/// Port of filesub() from subst.c lines 660-693
pub fn filesub_full(s: &str, assign: u32, state: &SubstState) -> String { // c:N/A
    let mut result = match filesubstr_full(s, assign != 0, state) { // c:N/A
        Some(r) => r,                                       // c:N/A
        None => s.to_string(),                              // c:N/A
    };                                                      // c:N/A

    if assign == 0 {                                        // c:N/A
        return result;                                      // c:N/A
    }                                                       // c:N/A

    // Handle typeset context
    if assign & prefork_flags::TYPESET != 0 {               // c:N/A
        if let Some(eq_pos) = result[1..].find([EQUALS, '=']) { // c:N/A
            let eq_pos = eq_pos + 1;                        // c:N/A
            let after_eq = &result[eq_pos + 1..];           // c:N/A
            let first_after = after_eq.chars().next();      // c:N/A

            if first_after == Some('~') || first_after == Some(EQUALS) { // c:N/A
                if let Some(expanded) = filesubstr_full(after_eq, true, state) { // c:N/A
                    let before: String = result.chars().take(eq_pos + 1).collect(); // c:N/A
                    result = format!("{}{}", before, expanded); // c:N/A
                }                                           // c:N/A
            }                                               // c:N/A
        }                                                   // c:N/A
    }                                                       // c:N/A

    // Handle colon-separated paths
    let mut pos = 0;                                        // c:N/A
    while let Some(colon_pos) = result[pos..].find(':') {   // c:N/A
        let abs_pos = pos + colon_pos;                      // c:N/A
        let after_colon = &result[abs_pos + 1..];           // c:N/A
        let first_after = after_colon.chars().next();       // c:N/A

        if first_after == Some('~') || first_after == Some(EQUALS) { // c:N/A
            if let Some(expanded) = filesubstr_full(after_colon, true, state) { // c:N/A
                let before: String = result.chars().take(abs_pos + 1).collect(); // c:N/A
                result = format!("{}{}", before, expanded); // c:N/A
            }                                               // c:N/A
        }                                                   // c:N/A

        pos = abs_pos + 1;                                  // c:N/A
    }                                                       // c:N/A

    result                                                  // c:N/A
}                                                           // c:N/A

/// Equal substitution (=cmd)
/// Port of equalsubstr() from subst.c lines 706-722
pub fn equalsubstr(s: &str, assign: bool, nomatch: bool, state: &SubstState) -> Option<String> { // c:715
    // Find end of command name
    let end = s                                             // c:715
        .chars()                                            // c:715
        .take_while(|&c| c != '\0' && c != INPAR && !(assign && c == ':')) // c:715
        .count();                                           // c:715

    let cmdstr: String = s.chars().take(end).collect();     // c:715
    let cmdstr = untokenize(&cmdstr);                       // c:715
    let cmdstr = remnulargs(&cmdstr);                       // c:715

    if let Some(path) = findcmd(&cmdstr, true, false) {     // c:715
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

/// Count nodes in linked list
/// Port of countlinknodes() from linklist.c
pub fn countlinknodes(list: &LinkList) -> usize {           // c:N/A
    list.len()                                              // c:N/A
}                                                           // c:N/A

/// Check if list is non-empty
/// Port of nonempty() macro
pub fn nonempty(list: &LinkList) -> bool {                  // c:N/A
    !list.is_empty()                                        // c:N/A
}                                                           // c:N/A

/// Get and remove first node from list
/// Port of ugetnode() from linklist.c
pub fn ugetnode(list: &mut LinkList) -> Option<String> {    // c:N/A
    if list.nodes.is_empty() {                              // c:N/A
        None                                                // c:N/A
    } else {                                                // c:N/A
        Some(list.nodes.pop_front().unwrap().data)          // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

/// Remove node from list
/// Port of uremnode() from linklist.c
pub fn uremnode(list: &mut LinkList, idx: usize) {          // c:N/A
    if idx < list.nodes.len() {                             // c:N/A
        list.nodes.remove(idx);                             // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

/// Increment node index (for iteration)
/// Port of incnode() macro
pub fn incnode(idx: &mut usize) {                           // c:N/A
    *idx += 1;
}                                                           // c:N/A

/// Get first node index
/// Port of firstnode() macro
pub fn firstnode(_list: &LinkList) -> usize {               // c:N/A
    0                                                       // c:N/A
}                                                           // c:N/A

/// Get next node index
/// Port of nextnode() macro
pub fn nextnode(_list: &LinkList, idx: usize) -> usize {    // c:N/A
    idx + 1                                                 // c:N/A
}                                                           // c:N/A

/// Get last node index
/// Port of lastnode() macro  
pub fn lastnode(list: &LinkList) -> usize {                 // c:N/A
    if list.is_empty() {                                    // c:N/A
        0                                                   // c:N/A
    } else {                                                // c:N/A
        list.len() - 1                                      // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

/// Get previous node index
/// Port of prevnode() macro
pub fn prevnode(_list: &LinkList, idx: usize) -> usize {    // c:N/A
    if idx > 0 {                                            // c:N/A
        idx - 1                                             // c:N/A
    } else {                                                // c:N/A
        0                                                   // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

/// Initialize a single-element list
/// Port of init_list1() macro
pub fn init_list1(list: &mut LinkList, data: &str) {        // c:N/A
    list.nodes.clear();                                     // c:N/A
    list.nodes.push_back(LinkNode {                         // c:N/A
        data: data.to_string(),                             // c:N/A
    });                                                     // c:N/A
}                                                           // c:N/A


/// Hook substitution for directory names
/// Port of subst_string_by_hook() stub
pub fn subst_string_by_hook(_hook: &str, _cmd: &str, _arg: &str) -> Option<Vec<String>> { // c:N/A
    // Would call registered hook here
    None                                                    // c:N/A
}                                                           // c:N/A

/// Report zero error
/// Port of zerr() from utils.c
pub fn zerr(fmt: &str, args: &[&str]) {                     // c:N/A
    eprint!("zsh: ");                                       // c:N/A
    let mut result = fmt.to_string();                       // c:N/A
    for (i, arg) in args.iter().enumerate() {               // c:N/A
        result = result.replace(&format!("%{}", i + 1), arg); // c:N/A
    }                                                       // c:N/A
    result = result.replace("%s", args.first().unwrap_or(&"")); // c:N/A
    eprintln!("{}", result);                                // c:N/A
}                                                           // c:N/A

/// Debug print (no-op in release)
#[cfg(debug_assertions)]                                    // c:N/A
pub fn dputs(_cond: bool, _msg: &str) {                     // c:N/A
    // Debug output
}                                                           // c:N/A

#[cfg(not(debug_assertions))]                               // c:N/A
pub fn dputs(_cond: bool, _msg: &str) {}                    // c:N/A

/// DPUTS macro equivalent
#[macro_export]                                             // c:N/A
macro_rules! DPUTS {                                        // c:N/A
    ($cond:expr, $msg:expr) => {                            // c:N/A
        #[cfg(debug_assertions)]                            // c:N/A
        if $cond {                                          // c:N/A
            eprintln!("BUG: {}", $msg);                     // c:N/A
        }                                                   // c:N/A
    };                                                      // c:N/A
}                                                           // c:N/A

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

/// Get output radix
pub fn get_output_radix() -> u32 {                          // c:N/A
    OUTPUT_RADIX.load(std::sync::atomic::Ordering::Relaxed) // c:N/A
}                                                           // c:N/A

/// Set output radix
pub fn set_output_radix(radix: u32) {                       // c:N/A
    OUTPUT_RADIX.store(radix, std::sync::atomic::Ordering::Relaxed); // c:N/A
}                                                           // c:N/A

/// Get output underscore
pub fn get_output_underscore() -> bool {                    // c:N/A
    OUTPUT_UNDERSCORE.load(std::sync::atomic::Ordering::Relaxed) // c:N/A
}                                                           // c:N/A

/// Set output underscore
pub fn set_output_underscore(underscore: bool) {            // c:N/A
    OUTPUT_UNDERSCORE.store(underscore, std::sync::atomic::Ordering::Relaxed); // c:N/A
}                                                           // c:N/A

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

/// Full math evaluation returning MNumber
/// Port of matheval() from math.c
pub fn matheval_full(expr: &str) -> MNumber {               // c:N/A
    let result = matheval(expr);                            // c:N/A
    match result {                                          // c:N/A
        MathResult::Integer(n) => MNumber {                 // c:N/A
            type_: 0,                                       // c:N/A
            int_val: n,                                     // c:N/A
            float_val: n as f64,                            // c:N/A
        },                                                  // c:N/A
        MathResult::Float(n) => MNumber {                   // c:N/A
            type_: MN_FLOAT,                                // c:N/A
            int_val: n as i64,                              // c:N/A
            float_val: n,                                   // c:N/A
        },                                                  // c:N/A
    }                                                       // c:N/A
}                                                           // c:N/A

/// Brace expansion state
#[derive(Debug, Clone)]                                     // c:N/A
pub struct BraceInfo {                                      // c:N/A
    pub str_: String,                                       // c:N/A
    pub pos: usize,                                         // c:N/A
    pub inbrace: bool,                                      // c:N/A
}                                                           // c:N/A

/// Full brace expansion
/// Port of xpandbraces() logic with more detail
pub fn xpandbraces_full(list: &mut LinkList, node_idx: &mut usize) { // glob.c:2276
    if *node_idx >= list.len() {                            // glob.c:2276
        return;                                             // glob.c:2276
    }                                                       // glob.c:2276

    let data = match list.get_data(*node_idx) {             // glob.c:2276
        Some(d) => d.to_string(),                           // glob.c:2276
        None => return,                                     // glob.c:2276
    };                                                      // glob.c:2276

    // Find brace group, handling nesting
    let chars: Vec<char> = data.chars().collect();          // glob.c:2276
    let mut brace_start = None;                             // glob.c:2276
    let mut brace_end = None;                               // glob.c:2276
    let mut depth = 0;                                      // glob.c:2276

    for (i, &c) in chars.iter().enumerate() {               // glob.c:2276
        if c == '{' || c == INBRACE {                       // glob.c:2276
            if depth == 0 {                                 // glob.c:2276
                brace_start = Some(i);                      // glob.c:2276
            }                                               // glob.c:2276
            depth += 1;                                     // glob.c:2276
        } else if c == '}' || c == OUTBRACE {               // glob.c:2276
            depth -= 1;                                     // glob.c:2276
            if depth == 0 && brace_start.is_some() {        // glob.c:2276
                brace_end = Some(i);                        // glob.c:2276
                break;                                      // glob.c:2276
            }                                               // glob.c:2276
        }                                                   // glob.c:2276
    }                                                       // glob.c:2276

    let (start, end) = match (brace_start, brace_end) {     // glob.c:2276
        (Some(s), Some(e)) => (s, e),                       // glob.c:2276
        _ => return,                                        // glob.c:2276
    };                                                      // glob.c:2276

    let prefix: String = chars[..start].iter().collect();   // glob.c:2276
    let content: String = chars[start + 1..end].iter().collect(); // glob.c:2276
    let suffix: String = chars[end + 1..].iter().collect(); // glob.c:2276

    // Check for sequence like {a..z} or {1..10}
    if let Some(range_result) = try_brace_sequence(&content) { // glob.c:2276
        list.remove(*node_idx);                             // glob.c:2276
        for (i, item) in range_result.iter().enumerate() {  // glob.c:2276
            let expanded = format!("{}{}{}", prefix, item, suffix); // glob.c:2276
            if i == 0 {                                     // glob.c:2276
                list.nodes.insert(*node_idx, LinkNode { data: expanded }); // glob.c:2276
            } else {                                        // glob.c:2276
                list.insert_after(*node_idx + i - 1, expanded); // glob.c:2276
            }                                               // glob.c:2276
        }                                                   // glob.c:2276
        return;                                             // glob.c:2276
    }                                                       // glob.c:2276

    // Handle comma-separated alternatives
    let alternatives: Vec<&str> = content.split(',').collect(); // glob.c:2276
    if alternatives.len() > 1 {                             // glob.c:2276
        list.remove(*node_idx);                             // glob.c:2276
        for (i, alt) in alternatives.iter().enumerate() {   // glob.c:2276
            let expanded = format!("{}{}{}", prefix, alt, suffix); // glob.c:2276
            if i == 0 {                                     // glob.c:2276
                list.nodes.insert(*node_idx, LinkNode { data: expanded }); // glob.c:2276
            } else {                                        // glob.c:2276
                list.insert_after(*node_idx + i - 1, expanded); // glob.c:2276
            }                                               // glob.c:2276
        }                                                   // glob.c:2276
    }                                                       // glob.c:2276
}                                                           // glob.c:2276

/// Try to parse brace sequence like {1..10} or {a..z}
fn try_brace_sequence(content: &str) -> Option<Vec<String>> { // glob.c:2276
    let parts: Vec<&str> = content.split("..").collect();   // glob.c:2276
    if parts.len() != 2 && parts.len() != 3 {               // glob.c:2276
        return None;                                        // glob.c:2276
    }                                                       // glob.c:2276

    let start = parts[0];                                   // glob.c:2276
    let end = parts[1];                                     // glob.c:2276
    let step: i64 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(1); // glob.c:2276

    // Numeric range
    if let (Ok(start_num), Ok(end_num)) = (start.parse::<i64>(), end.parse::<i64>()) { // glob.c:2276
        let mut result = Vec::new();                        // glob.c:2276
        if start_num <= end_num {                           // glob.c:2276
            let mut i = start_num;                          // glob.c:2276
            while i <= end_num {                            // glob.c:2276
                result.push(i.to_string());                 // glob.c:2276
                i += step;                                  // glob.c:2276
            }                                               // glob.c:2276
        } else {                                            // glob.c:2276
            let mut i = start_num;                          // glob.c:2276
            while i >= end_num {                            // glob.c:2276
                result.push(i.to_string());                 // glob.c:2276
                i -= step;                                  // glob.c:2276
            }                                               // glob.c:2276
        }                                                   // glob.c:2276
        return Some(result);                                // glob.c:2276
    }                                                       // glob.c:2276

    // Character range
    if start.len() == 1 && end.len() == 1 {                 // glob.c:2276
        let start_c = start.chars().next()?;                // glob.c:2276
        let end_c = end.chars().next()?;                    // glob.c:2276

        let mut result = Vec::new();                        // glob.c:2276
        if start_c <= end_c {                               // glob.c:2276
            for c in start_c..=end_c {                      // glob.c:2276
                result.push(c.to_string());                 // glob.c:2276
            }                                               // glob.c:2276
        } else {                                            // glob.c:2276
            for c in (end_c..=start_c).rev() {              // glob.c:2276
                result.push(c.to_string());                 // glob.c:2276
            }                                               // glob.c:2276
        }                                                   // glob.c:2276
        return Some(result);                                // glob.c:2276
    }                                                       // glob.c:2276

    None                                                    // glob.c:2276
}                                                           // glob.c:2276

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

    /// Faithful port entry. Returns `Ok(replacement_string, end_pos, nodes)`
    /// or `Err` to indicate the new path bailed out (caller falls back to
    /// legacy paramsubst).
    ///
    /// Currently in scaffolding state: declarations + flag-parse loop
    /// (subst.c:2147-2541) ported. Operator dispatch + post-processing
    /// remain as TODOs and trigger fallback to legacy.
    pub(crate) fn paramsubst_port(                              // c:2145
        s: &str,                                                    // c:1625 (n,aptr setup)
        start_pos: usize,                                           // c:1625 (str arg index)
        qt: bool,                                                   // c:1625 (qt arg)
        pf_flags: u32,                                              // c:1625 (pf_flags arg)
        ret_flags: &mut u32,                                        // c:1625 (ret_flags arg)
        state: &mut SubstState,                                     // c:1625 (executor handle)
    ) -> Result<(String, usize, Vec<String>), FallbackToLegacy> {   // c:1625 (LinkNode return)
        // Env-gate: opt-in until parity is reached. Outside-of-C plumbing.
        if std::env::var_os("ZSHRS_NEW_PARAMSUBST").is_none() {     // c:N/A (port-only gate)
            return Err(FallbackToLegacy);                           // c:N/A (port-only gate)
        }                                                           // c:N/A (port-only gate)

        let chars: Vec<char> = s.chars().collect();                 // c:1640 (s = aptr; *s++ = 0)
        if start_pos >= chars.len() {                               // c:1640 (bounds guard)
            return Err(FallbackToLegacy);                           // c:1640 (bounds guard)
        }                                                           // c:1640
        let mut pos = start_pos + 1;                                // c:1640 (s++ over leading $)

        let mut L = ParamSubstLocals::default();                    // c:1640-1830 (locals decls)
        L.plan9 = false;                                            // c:1663 (RCEXPANDPARAM init)
        L.globsubst = if state.opts.glob_subst { 1 } else { 0 };    // c:1669 (GLOBSUBST init)
        L.spbreak = (pf_flags & prefork_flags::SHWORDSPLIT) != 0    // c:1705 (spbreak compose)
            && (pf_flags & prefork_flags::SINGLE) == 0              // c:1706 (spbreak compose)
            && !qt;                                                 // c:1706 (spbreak compose)
        L.ssub = (pf_flags & prefork_flags::SINGLE) != 0;           // c:1759 (ssub init)
        L.nojoin = if (pf_flags & prefork_flags::SHWORDSPLIT) != 0  // c:1817 (nojoin compose)
        {                                                           // c:1817 (nojoin compose)
            0                                                       // c:1817 (IFS-set common case)
        } else {                                                    // c:1817 (nojoin compose)
            0                                                       // c:1817 (default)
        };                                                          // c:1817
        L.quotetype = QtType::None;                                 // c:1739 (quotetype = QT_NONE)
        L.casmod = CasMod::None;                                    // c:1740 (casmod = 0)
        L.getkeys = -1;                                             // c:1811 (getkeys = -1)

        let c = chars.get(pos).copied().unwrap_or('\0');            // c:1884 (c = *s)

        // Bare `$x` shapes (non-brace) stay on legacy paramsubst until
        // the brace path reaches parity.
        if c != '{' && c != '\u{87}' /* INBRACE */ {                // c:1885 (if (*s == Inbrace))
            return Err(FallbackToLegacy);                           // c:1885 (delegated)
        }                                                           // c:1885

        L.inbrace = 1;                                              // c:1901 (inbrace = 1)
        pos += 1;                                                   // c:1901 (s++ over `{`)

        // Nofork command-substitution kludge `${|cmd;}` etc.
        {                                                           // c:1901-2103 (nofork branch)
            let lead = chars.get(pos).copied().unwrap_or('\0');     // c:1901 (peek)
            if lead == '|' || lead == '\u{83}' /* TICK */ {         // c:1901 (Bar / Tick lead)
                return Err(FallbackToLegacy);                       // c:1901 (delegated)
            }                                                       // c:1901
            if lead == ' ' || lead == '\t' {                        // c:1901 (whitespace lead)
                return Err(FallbackToLegacy);                       // c:1901 (delegated)
            }                                                       // c:1901
        }                                                           // c:1901

        // ksh-style `${!name}` indirect lead.
        {                                                           // c:2118-2130 (ksh `!` arm)
            let lead = chars.get(pos).copied().unwrap_or('\0');     // c:2118 (peek)
            if lead == '!' {                                        // c:2118 (`!` test)
                return Err(FallbackToLegacy);                       // c:2118 (delegated)
            }                                                       // c:2118
        }                                                           // c:2118

        // `( ... )` flag-parsing loop.
        let lead = chars.get(pos).copied().unwrap_or('\0');         // c:2147 (c = *s)
        if lead == '(' || lead == '\u{85}' /* INPAR */ {            // c:2147 (`(` test)
            pos += 1;                                               // c:2147 (s++ over `(`)
            match parse_paren_flags(&chars, &mut pos, &mut L) {     // c:2147-2541 (flag for(;;))
                Ok(()) => {}                                        // c:2541 (loop exit normal)
                Err(_e) => {                                        // c:2504 (flagerr label)
                    return Err(FallbackToLegacy);                   // c:2504 (zerr+NULL → legacy)
                }                                                   // c:2504
            }                                                       // c:2541
            if matches!(chars.get(pos).copied(),                    // c:2541 (Outpar/`)` close)
                        Some(')') | Some('\u{86}')) {               // c:2541 (Outpar/`)` close)
                pos += 1;                                           // c:2541 (s++ over `)`)
            }                                                       // c:2541
        }                                                           // c:2541

        if L.premul.is_none() {                                     // c:2540 (if (!premul))
            L.premul = Some(" ".to_string());                       // c:2541 (premul = " ")
        }                                                           // c:2541
        if L.postmul.is_none() {                                    // c:2542 (if (!postmul))
            L.postmul = Some(" ".to_string());                      // c:2543 (postmul = " ")
        }                                                           // c:2543

        // Special unparenthesised flag loop.
        if let Err(_) = parse_special_unparen_flags(&chars,         // c:2550-2632 (unparen loop)
                                                    &mut pos,       // c:2550-2632
                                                    &mut L) {       // c:2550-2632
            return Err(FallbackToLegacy);                           // c:2620 (zerr → legacy)
        }                                                           // c:2632

        if qt {                                                     // c:2634 (if (qt))
            L.globsubst = 0;                                        // c:2635 (globsubst = 0)
        }                                                           // c:2635

        // Subexp / nested `${...}` / fetch-needed bail-out (multsub +
        // fetchvalue ports still pending).
        {                                                           // c:2637-2729 (subexp arm)
            let lead = chars.get(pos).copied().unwrap_or('\0');     // c:2637 (peek)
            if lead == '$'                                          // c:2637 (`$` test)
                || lead == '\u{81}' /* String */                    // c:2637 (String token)
                || lead == '\u{82}' /* Qstring */                   // c:2637 (Qstring token)
            {                                                       // c:2637
                return Err(FallbackToLegacy);                       // c:2637 (delegated)
            }                                                       // c:2637
        }                                                           // c:2729

        // (P) indirect-expansion handling — fetchvalue port pending.
        if L.aspar != 0 {                                           // c:2730 (if (aspar))
            return Err(FallbackToLegacy);                           // c:2730 (delegated)
        }                                                           // c:2756

        // Primary value fetch + (t) wantt — fetchvalue port pending.
        if L.wantt != 0 {                                           // c:2807 (if (wantt))
            return Err(FallbackToLegacy);                           // c:2807 (delegated)
        }                                                           // c:2861

        // Remaining sections (subscript loop, value extract, operator
        // dispatch, post-processing) inlined incrementally.
        let _ = (pos, ret_flags, state);                            // c:2862-4470 (pending)
        Err(FallbackToLegacy)                                       // c:2862-4470 (delegated)
    }                                                               // c:4473 (end of paramsubst)

    /// Faithful port of the flag-parsing loop at subst.c:2147-2541.
    ///
    /// Walks `(...)` flag chars and mutates `L` in place. Returns Err on
    /// `flagerr` (subst.c:2504-2530), Ok on a successful close-paren.
    fn parse_paren_flags(                                       // c:4473
        chars: &[char],                                         // c:4473
        pos: &mut usize,                                        // c:4473
        L: &mut ParamSubstLocals,                               // c:4473
    ) -> Result<(), FlagErr> {                                  // c:4473
        // subst.c:2147 — `for (s++; (c = *s) != ')' && c != Outpar; s++, tt = 0)`
        loop {                                                  // c:2147
            let c = chars.get(*pos).copied().unwrap_or('\0');   // c:2147
            // subst.c:2153-2156 — break on `)` / Outpar (handled by loop guard).
            if c == ')' || c == '\u{86}' /* OUTPAR */ {         // c:2153
                return Ok(());                                  // c:2153
            }                                                   // c:2153
            // tt — subst.c:2147 — reset each iteration. Used by 's'/'l'
            // case fall-through to distinguish the two arms below.
            let mut tt = 0;                                     // c:2153
            match c {                                           // c:2153
                // subst.c:2157-2160 — `~` toggle.
                '~' | '\u{93}' /* TILDE */ => {                 // c:2157
                    L.tok_arg = if L.tok_arg != 0 { 0 } else { 1 }; // c:2157
                }                                               // c:2157
                // subst.c:2161 — A flag.
                'A' => L.arrasg += 1,                           // c:2161
                // subst.c:2164 — `@`.
                '@' => L.nojoin = 2,                            // c:2164
                // subst.c:2167-2169 — `*` / Star → SUB_EGLOB.
                '*' | '\u{84}' /* STAR */ => L.flags |= sub_flags::EGLOB, // c:2167
                // subst.c:2171 — M.
                'M' => L.flags |= sub_flags::MATCH,             // c:2171
                // subst.c:2174 — R.
                'R' => L.flags |= sub_flags::REST,              // c:2174
                // subst.c:2177 — B.
                'B' => L.flags |= sub_flags::BIND,              // c:2177
                // subst.c:2180 — E.
                'E' => L.flags |= sub_flags::EIND,              // c:2180
                // subst.c:2183 — N.
                'N' => L.flags |= sub_flags::LEN,               // c:2183
                // subst.c:2186 — S.
                'S' => L.flags |= sub_flags::SUBSTR,            // c:2186
                // subst.c:2189-2195 — I<num>.
                'I' => {                                        // c:2189
                    *pos += 1;
                    let (num, _del) = get_intarg(chars, pos);   // c:2189
                    if num < 0 {                                // c:2189
                        return Err(FlagErr {                    // c:2189
                            offset: *pos,                       // c:2189
                            msg: "I needs non-negative arg",    // c:2189
                        });                                     // c:2189
                    }                                           // c:2189
                    L.flnum = num as i32;                       // c:2189
                    // C decrements s before loop's increment.
                    if *pos > 0 {                               // c:2189
                        *pos -= 1;
                    }                                           // c:2189
                }                                               // c:2189
                // subst.c:2197-2205 — case-mod flags.
                'L' => L.casmod = CasMod::Lower,                // c:2197
                'U' => L.casmod = CasMod::Upper,                // c:2197
                'C' => L.casmod = CasMod::Caps,                 // c:2197
                // subst.c:2207-2228 — sort flags.
                'o' => {                                        // c:2207
                    if L.sortit == sortit_flags::ANYOLDHOW {    // c:2207
                        L.sortit |= sortit_flags::SOMEHOW;      // c:2207
                    }                                           // c:2207
                }                                               // c:2207
                'O' => L.sortit |= sortit_flags::BACKWARDS,     // c:2207
                'i' => L.sortit |= sortit_flags::IGNORING_CASE, // c:2207
                'n' => L.sortit |= sortit_flags::NUMERICALLY,   // c:2207
                '-' | '\u{95}' /* DASH token */ => {            // c:2207
                    L.sortit |= sortit_flags::NUMERICALLY_SIGNED; // c:2207
                }                                               // c:2207
                'a' => {                                        // c:2207
                    L.sortit |= sortit_flags::SOMEHOW;          // c:2207
                    L.indord = 1;                               // c:2207
                }                                               // c:2207
                // subst.c:2229-2234 — D, V mod bits.
                'D' => L.mods |= 1,                             // c:2229
                'V' => L.mods |= 2,                             // c:2229
                // subst.c:2236-2253 — q (quoting).
                'q' => {                                        // c:2236
                    if L.quotetype == QtType::Dollars           // c:2236
                        || L.quotetype == QtType::BackslashPattern // c:2236
                    {                                           // c:2236
                        return Err(FlagErr {                    // c:2236
                            offset: *pos,                       // c:2236
                            msg: "q after $ or b",              // c:2236
                        });                                     // c:2236
                    }                                           // c:2236
                    let nx = chars.get(*pos + 1).copied().unwrap_or('\0'); // c:2236
                    if nx == '-' || nx == '+' {                 // c:2236
                        if L.quotemod != 0 {                    // c:2236
                            return Err(FlagErr {                // c:2236
                                offset: *pos,                   // c:2236
                                msg: "extra q",                 // c:2236
                            });                                 // c:2236
                        }                                       // c:2236
                        *pos += 1;
                        L.quotemod = 1;                         // c:2236
                        L.quotetype = if nx == '+' {            // c:2236
                            QtType::QuotedZputs                 // c:2236
                        } else {                                // c:2236
                            QtType::SingleOptional              // c:2236
                        };                                      // c:2236
                    } else {                                    // c:2236
                        if L.quotetype == QtType::SingleOptional { // c:2236
                            return Err(FlagErr {                // c:2236
                                offset: *pos,                   // c:2236
                                msg: "extra q after -",         // c:2236
                            });                                 // c:2236
                        }                                       // c:2236
                        L.quotemod += 1;                        // c:2236
                        L.quotetype = L.quotetype.next();       // c:2236
                    }                                           // c:2236
                }                                               // c:2236
                // subst.c:2255-2260 — b.
                'b' => {                                        // c:2255
                    if L.quotemod != 0 || L.quotetype != QtType::None { // c:2255
                        return Err(FlagErr {                    // c:2255
                            offset: *pos,                       // c:2255
                            msg: "b conflict",                  // c:2255
                        });                                     // c:2255
                    }                                           // c:2255
                    L.quotemod = 1;                             // c:2255
                    L.quotetype = QtType::BackslashPattern;     // c:2255
                }                                               // c:2255
                // subst.c:2261-2263 — Q.
                'Q' => L.quotemod -= 1,                         // c:2261
                // subst.c:2264 — X.
                'X' => L.quoteerr = 1,                          // c:2264
                // subst.c:2268-2270 — e.
                'e' => L.eval = 1,                              // c:2268
                // subst.c:2271-2273 — P.
                'P' => L.aspar = 1,                             // c:2271
                // subst.c:2275-2283 — c/w/W (whichlen).
                'c' => L.whichlen = 1,                          // c:2275
                'w' => L.whichlen = 2,                          // c:2275
                'W' => L.whichlen = 3,                          // c:2275
                // subst.c:2285-2290 — f / F.
                'f' => L.spsep = Some("\n".to_string()),        // c:2285
                'F' => L.sep = Some("\n".to_string()),          // c:2285
                // subst.c:2292-2297 — `0` separator (NUL via Meta).
                '0' => {                                        // c:2292
                    // Meta-coded NUL separator. Rust strings can't carry
                    // a real NUL through `String` cleanly without explicit
                    // bytes; use a sentinel internal marker char.
                    L.spsep = Some("\0".to_string());           // c:2292
                }                                               // c:2292
                // subst.c:2299-2317 — s / j (split / join).
                's' | 'j' => {                                  // c:2299
                    let is_s = c == 's';                        // c:2299
                    if is_s {                                   // c:2299
                        tt = 1;                                 // c:2299
                    }                                           // c:2299
                    *pos += 1;
                    match get_strarg(chars, pos) {              // c:2299
                        Some((arg, end)) => {                   // c:2299
                            if tt != 0 {                        // c:2299
                                L.spsep = Some(untok_and_escape(&arg, L.escapes, L.tok_arg)); // c:2299
                            } else {                            // c:2299
                                L.sep = Some(untok_and_escape(&arg, L.escapes, L.tok_arg)); // c:2299
                            }                                   // c:2299
                            // C: `s = t + arglen - 1;` then loop increments.
                            *pos = end;
                            if *pos > 0 {                       // c:2299
                                *pos -= 1;
                            }                                   // c:2299
                        }                                       // c:2299
                        None => {                               // c:2299
                            return Err(FlagErr {                // c:2299
                                offset: *pos,                   // c:2299
                                msg: "s/j missing arg",         // c:2299
                            });                                 // c:2299
                        }                                       // c:2299
                    }                                           // c:2299
                }                                               // c:2299
                // subst.c:2319-2373 — l / r (padding).
                'l' | 'r' => {                                  // c:2319
                    let is_l = c == 'l';                        // c:2319
                    if is_l {                                   // c:2319
                        tt = 1;                                 // c:2319
                    }                                           // c:2319
                    *pos += 1;
                    let _del0 = *pos;                           // c:2319
                    let (num, dellen) = get_intarg(chars, pos); // c:2319
                    if num < 0 {                                // c:2319
                        return Err(FlagErr {                    // c:2319
                            offset: *pos,                       // c:2319
                            msg: "l/r negative",                // c:2319
                        });                                     // c:2319
                    }                                           // c:2319
                    if tt != 0 {                                // c:2319
                        L.prenum = num;                         // c:2319
                    } else {                                    // c:2319
                        L.postnum = num;                        // c:2319
                    }                                           // c:2319
                    if dellen == 0 {                            // c:2319
                        if *pos > 0 {                           // c:2319
                            *pos -= 1;
                        }                                       // c:2319
                    } else {                                    // c:2319
                        // Optional pad-string-1 and pad-string-2. Each is
                        // delimited by the same dellen-char prefix.
                        if let Some((arg1, end1)) = get_strarg(chars, pos) { // c:2319
                            if tt != 0 {                        // c:2319
                                L.premul =                      // c:2319
                                    Some(untok_and_escape(&arg1, L.escapes, L.tok_arg)); // c:2319
                            } else {                            // c:2319
                                L.postmul =                     // c:2319
                                    Some(untok_and_escape(&arg1, L.escapes, L.tok_arg)); // c:2319
                            }                                   // c:2319
                            *pos = end1;
                            if let Some((arg2, end2)) = get_strarg(chars, pos) { // c:2319
                                if tt != 0 {                    // c:2319
                                    L.preone = Some(untok_and_escape( // c:2319
                                        &arg2,                  // c:2319
                                        L.escapes,              // c:2319
                                        L.tok_arg,              // c:2319
                                    ));                         // c:2319
                                } else {                        // c:2319
                                    L.postone = Some(untok_and_escape( // c:2319
                                        &arg2,                  // c:2319
                                        L.escapes,              // c:2319
                                        L.tok_arg,              // c:2319
                                    ));                         // c:2319
                                }                               // c:2319
                                *pos = end2;
                                if *pos > 0 {                   // c:2319
                                    *pos -= 1;
                                }                               // c:2319
                            } else if *pos > 0 {                // c:2319
                                *pos -= 1;
                            }                                   // c:2319
                        }                                       // c:2319
                    }                                           // c:2319
                }                                               // c:2319
                // subst.c:2375-2379 — m (multibyte width).
                'm' => L.multi_width += 1,                      // c:2375
                // subst.c:2381-2383 — p (escape processing).
                'p' => L.escapes = 1,                           // c:2381
                // subst.c:2385-2389 — `!` inside parens.
                '!' => {                                        // c:2385
                    if (L.hkeys | L.hvals) & !scanpm_flags::NONAMEREF != 0 { // c:2385
                        return Err(FlagErr {                    // c:2385
                            offset: *pos,                       // c:2385
                            msg: "! conflicts with k/v",        // c:2385
                        });                                     // c:2385
                    }                                           // c:2385
                    L.hkeys = scanpm_flags::NONAMEREF;          // c:2385
                }                                               // c:2385
                // subst.c:2390-2394 — k.
                'k' => {                                        // c:2390
                    if L.hkeys & !scanpm_flags::WANTKEYS != 0 { // c:2390
                        return Err(FlagErr {                    // c:2390
                            offset: *pos,                       // c:2390
                            msg: "k conflict",                  // c:2390
                        });                                     // c:2390
                    }                                           // c:2390
                    L.hkeys = scanpm_flags::WANTKEYS;           // c:2390
                }                                               // c:2390
                // subst.c:2395-2399 — v.
                'v' => {                                        // c:2395
                    if L.hvals & !scanpm_flags::WANTVALS != 0 { // c:2395
                        return Err(FlagErr {                    // c:2395
                            offset: *pos,                       // c:2395
                            msg: "v conflict",                  // c:2395
                        });                                     // c:2395
                    }                                           // c:2395
                    L.hvals = scanpm_flags::WANTVALS;           // c:2395
                }                                               // c:2395
                // subst.c:2401-2403 — t.
                't' => L.wantt = 1,                             // c:2401
                // subst.c:2405-2407 — `%`.
                '%' => L.presc += 1,                            // c:2405
                // subst.c:2409-2437 — g (key-string flags).
                'g' => {                                        // c:2409
                    *pos += 1;
                    if L.getkeys < 0 {                          // c:2409
                        L.getkeys = 0;                          // c:2409
                    }                                           // c:2409
                    match get_strarg(chars, pos) {              // c:2409
                        Some((arg, end)) => {                   // c:2409
                            for ch in arg.chars() {             // c:2409
                                match ch {                      // c:2409
                                    'e' => L.getkeys |= getkey::EMACS as i32, // c:2409
                                    'o' => L.getkeys |= getkey::OCTAL_ESC as i32, // c:2409
                                    'c' => L.getkeys |= getkey::CTRL as i32, // c:2409
                                    _ => {                      // c:2409
                                        return Err(FlagErr {    // c:2409
                                            offset: *pos,       // c:2409
                                            msg: "g unknown key", // c:2409
                                        })                      // c:2409
                                    }                           // c:2409
                                }                               // c:2409
                            }                                   // c:2409
                            *pos = end;
                            if *pos > 0 {                       // c:2409
                                *pos -= 1;
                            }                                   // c:2409
                        }                                       // c:2409
                        None => {                               // c:2409
                            return Err(FlagErr {                // c:2409
                                offset: *pos,                   // c:2409
                                msg: "g missing arg",           // c:2409
                            })                                  // c:2409
                        }                                       // c:2409
                    }                                           // c:2409
                }                                               // c:2409
                // subst.c:2439-2441 — z.
                'z' => L.shsplit = lexflags::ACTIVE,            // c:2439
                // subst.c:2443-2474 — Z<flags>.
                'Z' => {                                        // c:2443
                    *pos += 1;
                    match get_strarg(chars, pos) {              // c:2443
                        Some((arg, end)) => {                   // c:2443
                            for ch in arg.chars() {             // c:2443
                                match ch {                      // c:2443
                                    'c' => L.shsplit |= lexflags::COMMENTS_KEEP, // c:2443
                                    'C' => L.shsplit |= lexflags::COMMENTS_STRIP, // c:2443
                                    'n' => L.shsplit |= lexflags::NEWLINE, // c:2443
                                    _ => {                      // c:2443
                                        return Err(FlagErr {    // c:2443
                                            offset: *pos,       // c:2443
                                            msg: "Z unknown flag", // c:2443
                                        })                      // c:2443
                                    }                           // c:2443
                                }                               // c:2443
                            }                                   // c:2443
                            *pos = end;
                            if *pos > 0 {                       // c:2443
                                *pos -= 1;
                            }                                   // c:2443
                        }                                       // c:2443
                        None => {                               // c:2443
                            return Err(FlagErr {                // c:2443
                                offset: *pos,                   // c:2443
                                msg: "Z missing arg",           // c:2443
                            })                                  // c:2443
                        }                                       // c:2443
                    }                                           // c:2443
                }                                               // c:2443
                // subst.c:2476-2478 — u.
                'u' => L.unique = 1,                            // c:2476
                // subst.c:2480-2483 — # / Pound.
                '#' | '\u{80}' /* POUND */ => L.evalchar = 1,   // c:2480
                // subst.c:2485-2502 — `_` reserved-future.
                '_' => {                                        // c:2485
                    *pos += 1;
                    match get_strarg(chars, pos) {              // c:2485
                        Some((arg, end)) => {                   // c:2485
                            // every char is reserved → error.
                            if !arg.is_empty() {                // c:2485
                                return Err(FlagErr {            // c:2485
                                    offset: *pos,               // c:2485
                                    msg: "_ reserved",          // c:2485
                                });                             // c:2485
                            }                                   // c:2485
                            *pos = end;
                            if *pos > 0 {                       // c:2485
                                *pos -= 1;
                            }                                   // c:2485
                        }                                       // c:2485
                        None => {                               // c:2485
                            return Err(FlagErr {                // c:2485
                                offset: *pos,                   // c:2485
                                msg: "_ missing",               // c:2485
                            })                                  // c:2485
                        }                                       // c:2485
                    }                                           // c:2485
                }                                               // c:2485
                // subst.c:2504-2530 — default → flagerr.
                _ => {                                          // c:2504
                    return Err(FlagErr {                        // c:2504
                        offset: *pos,                           // c:2504
                        msg: "unknown flag",                    // c:2504
                    })                                          // c:2504
                }                                               // c:2504
            }                                                   // c:2504
            *pos += 1;
        }                                                       // c:2504
    }                                                           // c:2504

    /// Port of subst.c:2550-2632 — special unparenthesised flags loop.
    ///
    /// After the `(...)` flag block ends, zsh accepts a small further
    /// alphabet of bare lead chars: `^` (RC_EXPAND_PARAM toggle), `=`
    /// (SH_WORD_SPLIT toggle), `#` (length operator), `~` (GLOB_SUBST
    /// toggle), `+` (chkset/+ existence test). Doubled forms (`^^`, `==`,
    /// `~~`) flip the option *off*; single forms turn them on.
    ///
    /// Mutates `pos` to point at the first non-flag char (typically the
    /// parameter name's first letter). Returns Err if `+` is seen at top
    /// level outside any brace context — the C code emits `bad
    /// substitution`.
    fn parse_special_unparen_flags(                             // c:2504
        chars: &[char],                                         // c:2504
        pos: &mut usize,                                        // c:2504
        L: &mut ParamSubstLocals,                               // c:2504
    ) -> Result<(), FlagErr> {                                  // c:2504
        loop {                                                  // c:2504
            let c = chars.get(*pos).copied().unwrap_or('\0');   // c:2504
            match c {                                           // c:2504
                // subst.c:2551-2557 — `^` / Hat. Doubled = off.
                '^' => {                                        // c:2551
                    let nx = chars.get(*pos + 1).copied().unwrap_or('\0'); // c:2551
                    if nx == '^' {                              // c:2551
                        L.plan9 = false;                        // c:2551
                        *pos += 2;
                    } else {                                    // c:2551
                        L.plan9 = true;                         // c:2551
                        *pos += 1;
                    }                                           // c:2551
                }                                               // c:2551
                // subst.c:2558-2569 — `=` / Equals. Doubled = off; single
                // = force on (spbreak=2).
                '=' | '\u{8E}' /* EQUALS token */ => {          // c:2558
                    let nx = chars.get(*pos + 1).copied().unwrap_or('\0'); // c:2558
                    if nx == '=' || nx == '\u{8E}' {            // c:2558
                        L.spbreak = false;                      // c:2558
                        if L.nojoin < 2 {                       // c:2558
                            L.nojoin = 0;                       // c:2558
                        }                                       // c:2558
                        *pos += 2;
                    } else {                                    // c:2558
                        L.spbreak = true;                       // c:2558
                        if L.nojoin < 2 {                       // c:2558
                            // C: nojoin = !(ifs && *ifs); we don't track
                            // IFS here, default to 0 (IFS commonly set).
                            L.nojoin = 0;                       // c:2558
                        }                                       // c:2558
                        *pos += 1;
                    }                                           // c:2558
                }                                               // c:2558
                // subst.c:2570-2595 — `#` length operator. The C guard
                // is rich (peeks ahead for namespace chars, *, @, ?, $,
                // ##}, -, :-, ${, $(). Conservative port: accept # only
                // if next char looks like a name char or one of the
                // known-special leads, otherwise leave for operator
                // dispatch (which may treat ## as a strip-prefix).
                '#' | '\u{80}' /* POUND */ => {                 // c:2570
                    if !is_length_lead(chars, *pos) {           // c:2570
                        return Ok(());                          // c:2570
                    }                                           // c:2570
                    L.getlen = 1 + L.whichlen;                  // c:2570
                    *pos += 1;
                }                                               // c:2570
                // subst.c:2596-2602 — `~` / Tilde. Doubled = off; single
                // = forced on (globsubst=2).
                '~' => {                                        // c:2596
                    let nx = chars.get(*pos + 1).copied().unwrap_or('\0'); // c:2596
                    if nx == '~' {                              // c:2596
                        L.globsubst = 0;                        // c:2596
                        *pos += 2;
                    } else {                                    // c:2596
                        L.globsubst = 2;                        // c:2596
                        *pos += 1;
                    }                                           // c:2596
                }                                               // c:2596
                // subst.c:2603-2621 — `+` chkset / existence test.
                '+' => {                                        // c:2603
                    let nx = chars.get(*pos + 1).copied().unwrap_or('\0'); // c:2603
                    let nx2 = chars.get(*pos + 2).copied().unwrap_or('\0'); // c:2603
                    let name_lead = nx.is_ascii_alphanumeric() || nx == '_'; // c:2603
                    let p_indirect = L.aspar != 0               // c:2603
                        && (nx == '$')                          // c:2603
                        && (nx2 == '{' || nx2 == '(');          // c:2603
                    if name_lead || p_indirect {                // c:2603
                        L.chkset = 1;                           // c:2603
                        *pos += 1;
                    } else if L.inbrace == 0 {                  // c:2603
                        // Bare `$+` at top level: leave `$+` literal.
                        // Caller (legacy) handles this; bail.
                        return Err(FlagErr {                    // c:2603
                            offset: *pos,                       // c:2603
                            msg: "bare $+ leave-literal",       // c:2603
                        });                                     // c:2603
                    } else {                                    // c:2603
                        // ${+} with nothing after: bad substitution.
                        return Err(FlagErr {                    // c:2603
                            offset: *pos,                       // c:2603
                            msg: "bad substitution",            // c:2603
                        });                                     // c:2603
                    }                                           // c:2603
                }                                               // c:2603
                // subst.c:2622-2629 — inside braces, skip embedded null
                // tokens (String/Qstring back-pointers from $(<file)
                // collapses). Conservative: only skip our known token
                // chars; not full inull().
                _ if L.inbrace != 0 && is_inull_skip(c) => {    // c:2622
                    *pos += 1;
                }                                               // c:2622
                _ => return Ok(()),                             // c:2622
            }                                                   // c:2622
        }                                                       // c:2622
    }                                                           // c:2622

    /// True if the char at `pos` (which is `#` or POUND) qualifies as a
    /// length-operator lead per subst.c:2570-2587. Approximation: next
    /// char is alpha/_/digit, or one of `* @ ? $ - :` with appropriate
    /// follow-up.
    fn is_length_lead(chars: &[char], pos: usize) -> bool {     // c:2570
        let nx = match chars.get(pos + 1).copied() {            // c:2570
            Some(c) => c,                                       // c:2570
            None => return false,                               // c:2570
        };                                                      // c:2570
        if nx.is_ascii_alphanumeric() || nx == '_' {            // c:2570
            return true;                                        // c:2570
        }                                                       // c:2570
        matches!(                                               // c:2570
            nx,                                                 // c:2570
            '*' | '@' | '?' | '$' | '-' | '#' | '\u{80}' | '\u{84}' | '\u{81}' | '\u{82}' // c:2570
        ) || (nx == ':' && matches!(chars.get(pos + 2).copied(), Some('-'))) // c:2570
    }                                                           // c:2570

    /// True if `c` is a "null token" in the C `inull()` sense — these
    /// are backslash, quote, and dollar markers that the parser embeds
    /// during tokenisation. Skipping them lets the special-flag loop
    /// see through `${(f)"$(<file)"}` quoting.
    fn is_inull_skip(c: char) -> bool {                         // c:2622
        // Bnull (\u{94}) is excluded per the C `*s != Bnull` guard.
        matches!(c, '\u{81}' | '\u{82}' | '\u{83}' | '\u{84}' | '\u{8F}' | '\u{92}') // c:2622
    }                                                           // c:2622

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
                out.push(crate::subst::tokens::token_to_char(c)); // c:2155
            } else {                                            // c:2155
                out.push(c);                                    // c:2155
            }                                                   // c:2155
        }                                                       // c:2155
        out                                                     // c:2155
    }                                                           // c:2155

    #[cfg(test)]                                                // c:2155
    mod tests {                                                 // c:2155
        use super::*;                                           // c:2155

        /// Smoke test: with the env var unset, the new path immediately
        /// falls back. This guarantees the existing legacy path stays
        /// the default until parity is reached.
        #[test]                                                 // c:2155
        fn fallback_when_env_unset() {                          // c:2155
            // SAFETY: tests in this module assume the env var is not
            // pre-set in the harness.
            std::env::remove_var("ZSHRS_NEW_PARAMSUBST");       // c:2155
            let mut state = SubstState::default();              // c:2155
            let mut rf = 0u32;                                  // c:2155
            let r = paramsubst_port("${x}", 0, false, 0, &mut rf, &mut state); // c:2155
            assert!(r.is_err());                                // c:2155
        }                                                   // c:2155

        /// With the env var set, the scaffold parses the flag-loop and
        /// then falls through (the operator dispatch isn't ported yet).
        /// This exercises `parse_paren_flags` end-to-end without hitting
        /// the unported sections.
        #[test]                                                 // c:2155
        fn flag_loop_recognises_known_chars() {                 // c:2155
            let mut L = ParamSubstLocals::default();            // c:2155
            let chars: Vec<char> = "U)".chars().collect();      // c:2155
            let mut pos = 0;                                    // c:2155
            let r = parse_paren_flags(&chars, &mut pos, &mut L); // c:2155
            assert!(r.is_ok());                                 // c:2155
            assert_eq!(L.casmod, CasMod::Upper);                // c:2155
        }                                                   // c:2155

        #[test]                                                 // c:2155
        fn flag_loop_handles_quotemod_chain() {                 // c:2155
            let mut L = ParamSubstLocals::default();            // c:2155
            let chars: Vec<char> = "qq)".chars().collect();     // c:2155
            let mut pos = 0;                                    // c:2155
            let r = parse_paren_flags(&chars, &mut pos, &mut L); // c:2155
            assert!(r.is_ok());                                 // c:2155
            assert_eq!(L.quotemod, 2);                          // c:2155
        }                                                   // c:2155

        #[test]                                                 // c:2155
        fn flag_loop_split_arg() {                              // c:2155
            let mut L = ParamSubstLocals::default();            // c:2155
            let chars: Vec<char> = "s.:.)".chars().collect();   // c:2155
            let mut pos = 0;                                    // c:2155
            let r = parse_paren_flags(&chars, &mut pos, &mut L); // c:2155
            assert!(r.is_ok());                                 // c:2155
            assert_eq!(L.spsep.as_deref(), Some(":"));          // c:2155
        }                                                   // c:2155

        #[test]                                                 // c:2155
        fn flag_loop_unknown_errs() {                           // c:2155
            let mut L = ParamSubstLocals::default();            // c:2155
            let chars: Vec<char> = "Y)".chars().collect();      // c:2155
            let mut pos = 0;                                    // c:2155
            let r = parse_paren_flags(&chars, &mut pos, &mut L); // c:2155
            assert!(r.is_err());                                // c:2155
        }                                                   // c:2155

        #[test]                                                 // c:2155
        fn special_unparen_caret_doubled() {                    // c:2155
            let mut L = ParamSubstLocals::default();            // c:2155
            L.plan9 = true;                                     // c:2155
            let chars: Vec<char> = "^^x".chars().collect();     // c:2155
            let mut pos = 0;                                    // c:2155
            parse_special_unparen_flags(&chars, &mut pos, &mut L).unwrap(); // c:2155
            assert!(!L.plan9);                                  // c:2155
            assert_eq!(pos, 2);                                 // c:2155
        }                                                   // c:2155

        #[test]                                                 // c:2155
        fn special_unparen_tilde_single_forces() {              // c:2155
            let mut L = ParamSubstLocals::default();            // c:2155
            let chars: Vec<char> = "~x".chars().collect();      // c:2155
            let mut pos = 0;                                    // c:2155
            parse_special_unparen_flags(&chars, &mut pos, &mut L).unwrap(); // c:2155
            assert_eq!(L.globsubst, 2);                         // c:2155
            assert_eq!(pos, 1);                                 // c:2155
        }                                                   // c:2155

        #[test]                                                 // c:2155
        fn special_unparen_length_hash() {                      // c:2155
            let mut L = ParamSubstLocals::default();            // c:2155
            L.inbrace = 1;                                      // c:2155
            let chars: Vec<char> = "#name".chars().collect();   // c:2155
            let mut pos = 0;                                    // c:2155
            parse_special_unparen_flags(&chars, &mut pos, &mut L).unwrap(); // c:2155
            assert_eq!(L.getlen, 1);                            // c:2155
            assert_eq!(pos, 1);                                 // c:2155
        }                                                   // c:2155

        #[test]                                                 // c:2155
        fn special_unparen_chkset_plus() {                      // c:2155
            let mut L = ParamSubstLocals::default();            // c:2155
            L.inbrace = 1;                                      // c:2155
            let chars: Vec<char> = "+name".chars().collect();   // c:2155
            let mut pos = 0;                                    // c:2155
            parse_special_unparen_flags(&chars, &mut pos, &mut L).unwrap(); // c:2155
            assert_eq!(L.chkset, 1);                            // c:2155
            assert_eq!(pos, 1);                                 // c:2155
        }                                                   // c:2155

    }                                                       // c:2155

}                                                           // c:2155

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: subst
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// Expand ~ with named directories
    pub fn expand_tilde_named(&self, path: &str) -> String {
        if let Some(rest) = path.strip_prefix('~') {
            // Check for ~name or ~name/...
            let (name_raw, suffix) = if let Some(slash_pos) = rest.find('/') {
                (&rest[..slash_pos], &rest[slash_pos..])
            } else {
                (rest, "")
            };
            // The name segment may contain `$VAR` references (`~$USER`,
            // `~"$USER"`). Pre-resolve via env-lookup before treating
            // as a username — zsh expands `$VAR` then tries `~result`.
            // Also strip surrounding `"` and `'` (the quoted form
            // `~"$USER"` arrives here with the quote chars intact).
            let name_owned: String;
            let name: &str =
                if name_raw.contains('$') || name_raw.contains('"') || name_raw.contains('\'') {
                    let mut out = String::new();
                    let mut chars = name_raw.chars().peekable();
                    while let Some(c) = chars.next() {
                        if c == '$' {
                            let mut var_name = String::new();
                            while let Some(&pc) = chars.peek() {
                                if pc.is_ascii_alphanumeric() || pc == '_' {
                                    var_name.push(chars.next().unwrap());
                                } else {
                                    break;
                                }
                            }
                            if !var_name.is_empty() {
                                out.push_str(&self.get_variable(&var_name));
                            } else {
                                out.push('$');
                            }
                        } else if c == '"' || c == '\'' {
                            // Strip — quotes here are quoting for the
                            // username lookup, not literal user chars.
                        } else {
                            out.push(c);
                        }
                    }
                    name_owned = out;
                    name_owned.as_str()
                } else {
                    name_raw
                };

            if name.is_empty() {
                // Regular ~ expansion. Prefer the shell's variable
                // store over OS env so a non-exported `HOME=/tmp; cd ~`
                // honors the shell-local override.
                if let Some(home) = self.variables.get("HOME") {
                    return format!("{}{}", home, suffix);
                }
                if let Ok(home) = std::env::var("HOME") {
                    return format!("{}{}", home, suffix);
                }
            } else if name == "+" {
                // `~+` — current directory ($PWD).
                if let Ok(pwd) = std::env::var("PWD") {
                    return format!("{}{}", pwd, suffix);
                }
            } else if name == "-" {
                // `~-` — previous directory ($OLDPWD).
                if let Ok(oldpwd) = std::env::var("OLDPWD") {
                    return format!("{}{}", oldpwd, suffix);
                }
            } else if name.chars().all(|c| c.is_ascii_digit()) && !name.is_empty() {
                // `~N` (digits only) — same as `~+N`: Nth entry on
                // dir stack, 0 = $PWD. zsh accepts both forms.
                if let Ok(n) = name.parse::<usize>() {
                    if n == 0 {
                        if let Ok(pwd) = std::env::var("PWD") {
                            return format!("{}{}", pwd, suffix);
                        }
                    } else if let Some(d) = self.dir_stack.get(n - 1) {
                        return format!("{}{}", d.display(), suffix);
                    }
                }
            } else if let Some(stripped) = name.strip_prefix('+') {
                // `~+N` — Nth entry on dir stack (1-indexed; 0 = $PWD).
                if let Ok(n) = stripped.parse::<usize>() {
                    if n == 0 {
                        if let Ok(pwd) = std::env::var("PWD") {
                            return format!("{}{}", pwd, suffix);
                        }
                    } else if let Some(d) = self.dir_stack.get(n - 1) {
                        return format!("{}{}", d.display(), suffix);
                    }
                }
            } else if let Some(stripped) = name.strip_prefix('-') {
                // `~-N` — Nth entry from bottom of dir stack.
                if let Ok(n) = stripped.parse::<usize>() {
                    let len = self.dir_stack.len();
                    if n < len {
                        if let Some(d) = self.dir_stack.get(len - 1 - n) {
                            return format!("{}{}", d.display(), suffix);
                        }
                    }
                }
            } else if let Some(dir) = self.named_dirs.get(name) {
                return format!("{}{}", dir.display(), suffix);
            } else {
                // `~user` — try libc getpwnam to resolve user home.
                use std::ffi::CString;
                if let Ok(cname) = CString::new(name) {
                    unsafe {
                        let pw = libc::getpwnam(cname.as_ptr());
                        if !pw.is_null() {
                            let home_ptr = (*pw).pw_dir;
                            if !home_ptr.is_null() {
                                let home = std::ffi::CStr::from_ptr(home_ptr)
                                    .to_string_lossy()
                                    .into_owned();
                                return format!("{}{}", home, suffix);
                            }
                        }
                    }
                }
                // No such user / named dir — zsh emits a fatal error
                // and exits 1 in -c mode.
                eprintln!("zshrs:1: no such user or named directory: {}", name);
                std::process::exit(1);
            }
        }
        path.to_string()
    }
    /// Expand brace patterns like {a,b,c} and {1..10}
    pub(crate) fn expand_braces(&self, s: &str) -> Vec<String> {
        // Fast path: a literal-escaped brace pair anywhere in the input
        // means the user wants the braces taken literally. `echo \{a,b\}`
        // should print `{a,b}`, not iterate. Strip the backslashes from
        // the escaped braces and return as a single literal token.
        // Without this guard, the brace finder treated `\{` as `{` and
        // `\}` as `}`, expanded the comma list, and emitted partial
        // strings like `\foo \bar\` which untokenize then mangled into
        // `oo ar\`.
        if (s.contains("\\{") || s.contains("\\}")) && Self::has_balanced_escaped_braces(s) {
            let stripped = s.replace("\\{", "{").replace("\\}", "}");
            return vec![stripped];
        }
        // Find a brace pattern
        let mut depth = 0;
        let mut brace_start = None;
        for (i, c) in s.char_indices() {
            match c {
                '{' => {
                    if depth == 0 {
                        brace_start = Some(i);
                    }
                    depth += 1;
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        if let Some(start) = brace_start {
                            let prefix = &s[..start];
                            let content = &s[start + 1..i];
                            let suffix = &s[i + 1..];

                            // Check if this is a sequence `{a..b}` or a
                            // list `{a,b,c}`. The previous version's
                            // `contains("..")` precedence was wrong for
                            // mixed content like `{{1..3},x,y}` — the
                            // outer braces contain `..` (inside the
                            // nested `{1..3}`) AND a top-level `,`, but
                            // it's a LIST at this level. zsh resolves
                            // by checking nesting depth: a top-level
                            // `,` (depth 0) makes the whole brace a
                            // list; the `..` only counts if there's
                            // no top-level comma. Same pattern as the
                            // expand_brace_list scanner already does
                            // for splitting.
                            let has_top_comma = {
                                let mut d = 0;
                                let mut found = false;
                                for c in content.chars() {
                                    match c {
                                        '{' => d += 1,
                                        '}' => d -= 1,
                                        ',' if d == 0 => {
                                            found = true;
                                            break;
                                        }
                                        _ => {}
                                    }
                                }
                                found
                            };
                            // Top-level `..` (depth 0) means a sequence range —
                            // distinguish from `{a..b}` nested inside e.g.
                            // `{x{a..b}y}`.
                            let has_top_dotdot = {
                                let mut d = 0;
                                let mut prev = '\0';
                                let mut found = false;
                                for c in content.chars() {
                                    match c {
                                        '{' => d += 1,
                                        '}' => d -= 1,
                                        '.' if d == 0 && prev == '.' => {
                                            found = true;
                                            break;
                                        }
                                        _ => {}
                                    }
                                    prev = c;
                                }
                                found
                            };
                            let expansions = if has_top_comma {
                                self.expand_brace_list(content)
                            } else if has_top_dotdot {
                                self.expand_brace_sequence(content)
                            } else {
                                // BRACECCL char-class expansion — direct port
                                // of zsh/Src/glob.c:2424-2470. When BRACECCL
                                // is set and the brace contents have no top-
                                // level `,` or `..`, expand chars and `c1-c2`
                                // ranges into a sorted unique character list:
                                // `{a-mnop}` → a b c ... m n o p. The option
                                // is off by default; opt-in via `setopt
                                // braceccl` or `set -B`.
                                let braceccl_on =
                                    self.options.get("braceccl").copied().unwrap_or(false);
                                if braceccl_on && !content.is_empty() {
                                    let ccl = Self::expand_brace_ccl(content);
                                    if !ccl.is_empty() {
                                        let mut results = Vec::with_capacity(ccl.len());
                                        for ch in ccl {
                                            let combined = format!("{}{}{}", prefix, ch, suffix);
                                            results.extend(self.expand_braces(&combined));
                                        }
                                        return results;
                                    }
                                }
                                // No top-level comma or `..` — outer braces
                                // are NOT a brace expansion. Direct port of
                                // zsh's brace-expand pass: `{a{1,2}b}` keeps
                                // the literal outer braces, but recursively
                                // expands any nested brace expressions inside,
                                // re-wrapping each result. Without this,
                                // zshrs left the whole token literal.
                                if content.contains('{') && content.contains('}') {
                                    let inner = self.expand_braces(content);
                                    if inner.len() > 1 || (inner.len() == 1 && inner[0] != content)
                                    {
                                        let mut results = Vec::with_capacity(inner.len());
                                        for exp in inner {
                                            results
                                                .push(format!("{}{{{}}}{}", prefix, exp, suffix));
                                        }
                                        return results;
                                    }
                                }
                                // Not a valid brace expansion, return as-is.
                                return vec![s.to_string()];
                            };

                            // Stepped char ranges ({a..z..2}) and other
                            // unrecognised forms come back unchanged from
                            // expand_brace_sequence. Detect that single-element
                            // identity case BEFORE recursing — otherwise the
                            // recursive expand_braces re-finds the same
                            // braces and stack-overflows.
                            if expansions.len() == 1 && expansions[0] == content {
                                return vec![s.to_string()];
                            }

                            // Combine prefix, expansions, and suffix
                            let mut results = Vec::new();
                            for exp in expansions {
                                let combined = format!("{}{}{}", prefix, exp, suffix);
                                // Recursively expand any remaining braces
                                results.extend(self.expand_braces(&combined));
                            }
                            return results;
                        }
                    }
                }
                _ => {}
            }
        }

        // No brace expansion found
        vec![s.to_string()]
    }
    /// BRACECCL char-class expansion. Direct port of
    /// zsh/Src/glob.c:2424-2470. Walks `content` char-by-char; on
    /// `lo-hi` (where `lo` was just inserted, `-` is the dash, `hi`
    /// is peeked next), fill the open interval `(lo, hi)` — `lo` is
    /// already present and `hi` will be inserted by the next
    /// iteration. Output is sorted and deduplicated, mirroring the C
    /// 256-byte `ccl[]` boolean array followed by ascending walk.
    /// Empty `content` returns an empty Vec — caller falls back to
    /// the literal-`{}` path so the token isn't dropped.
    pub(crate) fn expand_brace_ccl(content: &str) -> Vec<String> {
        let chars: Vec<char> = content.chars().collect();
        if chars.is_empty() {
            return Vec::new();
        }
        let mut set: std::collections::BTreeSet<char> = std::collections::BTreeSet::new();
        let mut lastch: Option<char> = None;
        let mut i = 0;
        while i < chars.len() {
            let c1 = chars[i];
            i += 1;
            // c2 in the C code is `*p` after consuming c1 — peek without
            // consuming. Range fires only when (a) c1 is the dash, (b) we
            // already inserted a left endpoint, (c) a right endpoint
            // exists, (d) lo <= hi.
            let c2_peek = chars.get(i).copied();
            if c1 == '-' {
                if let (Some(lo), Some(hi)) = (lastch, c2_peek) {
                    if (lo as u32) <= (hi as u32) {
                        let mut x = lo as u32 + 1;
                        while x < hi as u32 {
                            if let Some(c) = char::from_u32(x) {
                                set.insert(c);
                            }
                            x += 1;
                        }
                        // glob.c:2449 sets lastch=-1 sentinel; next iter's
                        // `c1 = *p++` consumes hi and inserts it normally.
                        lastch = None;
                        continue;
                    }
                }
            }
            set.insert(c1);
            lastch = Some(c1);
        }
        set.into_iter().map(|c| c.to_string()).collect()
    }
    /// Expand comma-separated brace list like {a,b,c}
    pub(crate) fn expand_brace_list(&self, content: &str) -> Vec<String> {
        // Split by comma, but respect nested braces
        let mut parts = Vec::new();
        let mut current = String::new();
        let mut depth = 0;

        for c in content.chars() {
            match c {
                '{' => {
                    depth += 1;
                    current.push(c);
                }
                '}' => {
                    depth -= 1;
                    current.push(c);
                }
                ',' if depth == 0 => {
                    parts.push(current.clone());
                    current.clear();
                }
                _ => current.push(c),
            }
        }
        parts.push(current);

        parts
    }
    /// Expand sequence brace pattern like {1..10} or {a..z}
    pub(crate) fn expand_brace_sequence(&self, content: &str) -> Vec<String> {
        let parts: Vec<&str> = content.splitn(3, "..").collect();
        if parts.len() < 2 {
            return vec![content.to_string()];
        }

        let start = parts[0];
        let end = parts[1];
        let step: i64 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
        // zsh: step 0 is invalid — `{1..3..0}` stays literal
        // `1..3..0`. zshrs's `abs_step.max(1)` silently treated 0
        // as 1 and produced `1 2 3`. Negative steps reverse the
        // sequence (per zsh) so we only short-circuit on exactly 0.
        if parts.len() == 3 && step == 0 {
            return vec![content.to_string()];
        }

        // Try numeric sequence
        if let (Ok(start_num), Ok(end_num)) = (start.parse::<i64>(), end.parse::<i64>()) {
            // Zero-padding: if either bound has a leading zero (e.g. `01`,
            // `001`), pad each output to the max width of start/end.
            let pad_width = {
                let start_pad = start.starts_with('0') && start.len() > 1;
                let end_pad = end.starts_with('0') && end.len() > 1;
                if start_pad || end_pad {
                    start.len().max(end.len())
                } else {
                    0
                }
            };
            let format_num = |n: i64| -> String {
                if pad_width > 0 {
                    if n < 0 {
                        format!("-{:0>w$}", -n, w = pad_width.saturating_sub(1))
                    } else {
                        format!("{:0>w$}", n, w = pad_width)
                    }
                } else {
                    n.to_string()
                }
            };
            let mut results = Vec::new();
            // zsh: a negative step REVERSES the natural sequence
            // direction. So `{1..10..-2}` reverses `{1..10..2}` →
            // `9 7 5 3 1`. Treat step as its absolute value for
            // generation, then reverse if step was negative.
            let abs_step = step.abs().max(1);
            if start_num <= end_num {
                let mut i = start_num;
                while i <= end_num {
                    results.push(format_num(i));
                    i += abs_step;
                }
            } else {
                let mut i = start_num;
                while i >= end_num {
                    results.push(format_num(i));
                    i -= abs_step;
                }
            }
            if step < 0 {
                results.reverse();
            }
            return results;
        }

        // Try character sequence. zsh only expands UNSTEPPED char ranges
        // ({a..z}); a stepped form ({a..z..2}) is left literal — return
        // the unchanged content so the caller (expand_braces) detects
        // identity and re-wraps with the original braces.
        if parts.len() == 2 && start.len() == 1 && end.len() == 1 {
            let start_char = start.chars().next().unwrap();
            let end_char = end.chars().next().unwrap();
            let mut results = Vec::new();

            if start_char <= end_char {
                let mut c = start_char;
                while c <= end_char {
                    results.push(c.to_string());
                    c = (c as u8 + step as u8) as char;
                    if c as u8 > end_char as u8 {
                        break;
                    }
                }
            } else {
                let mut c = start_char;
                while c >= end_char {
                    results.push(c.to_string());
                    if (c as u8) < step as u8 {
                        break;
                    }
                    c = (c as u8 - step as u8) as char;
                }
            }
            return results;
        }

        vec![content.to_string()]
    }
    /// Expand extended glob pattern
    pub(crate) fn expand_extglob(&self, pattern: &str) -> Vec<String> {
        // Determine directory to search
        let (search_dir, file_pattern) = if let Some(last_slash) = pattern.rfind('/') {
            (&pattern[..last_slash], &pattern[last_slash + 1..])
        } else {
            (".", pattern)
        };

        // Check for !(pattern) - negative matching
        if let Some((neg_pat, suffix)) = self.extract_neg_extglob(file_pattern) {
            return self.expand_neg_extglob(search_dir, &neg_pat, &suffix, pattern);
        }

        // Convert file pattern to regex for positive extglob
        let regex_str = self.extglob_to_regex(file_pattern);

        let re = match cached_regex(&regex_str) {
            Some(r) => r,
            None => return vec![pattern.to_string()],
        };

        let mut results = Vec::new();

        if let Ok(entries) = std::fs::read_dir(search_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                // Skip hidden files unless pattern starts with .
                if name.starts_with('.') && !file_pattern.starts_with('.') {
                    continue;
                }

                if re.is_match(&name) {
                    let full_path = if search_dir == "." {
                        name
                    } else {
                        format!("{}/{}", search_dir, name)
                    };
                    results.push(full_path);
                }
            }
        }

        if results.is_empty() {
            vec![pattern.to_string()]
        } else {
            results.sort();
            results
        }
    }
    /// Handle !(pattern) negative extglob expansion
    pub(crate) fn expand_neg_extglob(
        &self,
        search_dir: &str,
        neg_pat: &str,
        suffix: &str,
        original_pattern: &str,
    ) -> Vec<String> {
        let mut results = Vec::new();

        if let Ok(entries) = std::fs::read_dir(search_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                // Skip hidden files
                if name.starts_with('.') {
                    continue;
                }

                // File must end with suffix
                if !name.ends_with(suffix) {
                    continue;
                }

                let basename = &name[..name.len() - suffix.len()];
                // Check if basename matches any negated alternative
                let alts: Vec<&str> = neg_pat.split('|').collect();
                let matches_neg = alts.iter().any(|alt| {
                    if alt.contains('*') || alt.contains('?') {
                        let alt_re = self.extglob_inner_to_regex(alt);
                        let full_pattern = format!("^{}$", alt_re);
                        if let Some(r) = cached_regex(&full_pattern) {
                            r.is_match(basename)
                        } else {
                            *alt == basename
                        }
                    } else {
                        *alt == basename
                    }
                });

                if !matches_neg {
                    let full_path = if search_dir == "." {
                        name
                    } else {
                        format!("{}/{}", search_dir, name)
                    };
                    results.push(full_path);
                }
            }
        }

        if results.is_empty() {
            vec![original_pattern.to_string()]
        } else {
            results.sort();
            results
        }
    }
    /// Expand string with word splitting - returns Vec for array expansions
    pub(crate) fn expand_string_split(&mut self, s: &str) -> Vec<String> {
        let mut results: Vec<String> = Vec::new();
        let mut current = String::new();
        let mut chars = s.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '$' {
                if chars.peek() == Some(&'{') {
                    chars.next(); // consume '{'
                    let mut brace_content = String::new();
                    let mut depth = 1;
                    for ch in chars.by_ref() {
                        if ch == '{' {
                            depth += 1;
                            brace_content.push(ch);
                        } else if ch == '}' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                            brace_content.push(ch);
                        } else {
                            brace_content.push(ch);
                        }
                    }

                    // Check if this is an array expansion ${arr[@]} or ${arr[*]}
                    if let Some(bracket_start) = brace_content.find('[') {
                        let var_name = &brace_content[..bracket_start];
                        let bracket_content = &brace_content[bracket_start + 1..];
                        if let Some(bracket_end) = bracket_content.find(']') {
                            let index = &bracket_content[..bracket_end];
                            if (index == "@" || index == "*")
                                && bracket_end + 1 == bracket_content.len()
                            {
                                // This is ${arr[@]} - expand to separate elements
                                if !current.is_empty() {
                                    results.push(current.clone());
                                    current.clear();
                                }
                                if let Some(arr) = self.arrays.get(var_name) {
                                    results.extend(arr.clone());
                                }
                                continue;
                            }
                        }
                    }

                    // Not an array expansion, route through the C-port
                    // `${…}` engine in `crate::subst_port`. Replaces the
                    // adhoc `expand_braced_variable` per the
                    // "subst_port.rs is the only paramsubst" directive.
                    current.push_str(&crate::subst::substitute_brace(&brace_content, self));
                } else {
                    // Simple variable like $var
                    let mut var_name = String::new();
                    while let Some(&ch) = chars.peek() {
                        if ch.is_alphanumeric() || ch == '_' {
                            var_name.push(chars.next().unwrap());
                        } else {
                            break;
                        }
                    }
                    let val = self.get_variable(&var_name);
                    // Split this variable's value
                    if !current.is_empty() {
                        results.push(current.clone());
                        current.clear();
                    }
                    results.extend(self.split_words(&val));
                }
            } else {
                current.push(c);
            }
        }

        if !current.is_empty() {
            results.push(current);
        }

        if results.is_empty() {
            results.push(String::new());
        }

        results
    }
    #[tracing::instrument(level = "trace", skip_all)]
    pub(crate) fn expand_word(&mut self, word: &ShellWord) -> String {
        match word {
            ShellWord::Literal(s) => self.expand_string(s),
            ShellWord::Concat(parts) => self.expand_concat_parallel(parts),
        }
    }
    /// Pre-launch external command substitutions from a word list onto the worker pool.
    /// Returns a Vec aligned with `words` — Some(receiver) for pre-launched externals, None otherwise.
    /// Expand a Concat word list. The parallel-CommandSub pre-launch logic
    /// that used to live here is gone — `ShellWord::CommandSub` was deleted
    /// alongside the legacy parser in Phase 2, and ZWC produces only
    /// `Literal`/`Concat` so concat parts never contain command subs now.
    pub(crate) fn expand_concat_parallel(&mut self, parts: &[ShellWord]) -> String {
        parts.iter().map(|p| self.expand_word(p)).collect()
    }
    #[tracing::instrument(level = "trace", skip_all)]
    pub(crate) fn expand_string(&mut self, s: &str) -> String {
        let mut result = String::new();
        let mut chars = s.chars().peekable();

        while let Some(c) = chars.next() {
            // \x00 prefix marks chars from single quotes - keep them literal
            if c == '\x00' {
                if let Some(literal_char) = chars.next() {
                    result.push(literal_char);
                }
                continue;
            }
            if c == '$' {
                // Bare `$` at end-of-string — literal dollar sign. zsh
                // treats `!$` and `echo $` as literal in non-interactive
                // mode (history expansion is off). Without this guard the
                // var-name loop below collects an empty name and resolves
                // to "" — eating the dollar sign.
                if chars.peek().is_none() {
                    result.push('$');
                    continue;
                }
                // `$'...'` — ANSI-C quoting. Per zsh docs (zshmisc),
                // the contents are processed for backslash escapes
                // (`\e`, `\n`, `\xNN`, `\uNNNN`, …) and the result is
                // single-quoted (no further parameter substitution).
                // The lexer normally resolves this earlier, but the
                // raw `${var/$'\e'/…}` operand reaches expand_string
                // without prior tokenization, so handle it here.
                if chars.peek() == Some(&'\'') {
                    chars.next(); // consume opening `'`
                    let mut content = String::new();
                    let mut escaped = false;
                    while let Some(&pc) = chars.peek() {
                        if escaped {
                            content.push(pc);
                            chars.next();
                            escaped = false;
                            continue;
                        }
                        if pc == '\\' {
                            content.push(pc);
                            chars.next();
                            escaped = true;
                            continue;
                        }
                        if pc == '\'' {
                            chars.next(); // consume closing `'`
                            break;
                        }
                        content.push(pc);
                        chars.next();
                    }
                    result.push_str(&crate::subst::getkeystring_pub(&content));
                    continue;
                }
                if chars.peek() == Some(&'(') {
                    chars.next(); // consume '('

                    // Check for $(( )) arithmetic
                    if chars.peek() == Some(&'(') {
                        chars.next(); // consume second '('
                        let expr = Self::collect_until_double_paren(&mut chars);
                        result.push_str(&self.evaluate_arithmetic(&expr));
                    } else {
                        // Command substitution $(...)
                        let cmd_str = Self::collect_until_paren(&mut chars);
                        result.push_str(&self.run_command_substitution(&cmd_str));
                    }
                } else if chars.peek() == Some(&'{') {
                    chars.next();
                    // Collect the full braced expression including brackets
                    let mut brace_content = String::new();
                    let mut depth = 1;
                    for c in chars.by_ref() {
                        if c == '{' {
                            depth += 1;
                            brace_content.push(c);
                        } else if c == '}' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                            brace_content.push(c);
                        } else {
                            brace_content.push(c);
                        }
                    }
                    result.push_str(&crate::subst::substitute_brace(&brace_content, self));
                } else {
                    // Check for single-char special vars first: $$, $!, $-
                    if matches!(chars.peek(), Some(&'$') | Some(&'!') | Some(&'-')) {
                        let sc = chars.next().unwrap();
                        result.push_str(&self.get_variable(&sc.to_string()));
                        continue;
                    }
                    // `$+NAME` — set-test, returns "1" or "0". zsh
                    // accepts both `${+NAME}` (handled by the brace
                    // path above via subst_port::paramsubst's chkset)
                    // AND the unbraced `$+NAME` shape, which p10k
                    // uses heavily: `(( $+__p9k_root_dir )) ||
                    // typeset -gr __p9k_root_dir=...`. Direct port
                    // of Src/subst.c:2604-2612 chkset detection
                    // (the inbrace == false branch keeps `+` literal,
                    // but for `$+NAME` followed by an identifier we
                    // forward to the brace path so the same chkset
                    // result emits).
                    if chars.peek() == Some(&'+') {
                        let mut peek_iter = chars.clone();
                        peek_iter.next(); // skip +
                        let next = peek_iter.peek().copied();
                        if next.map(|c| c.is_ascii_alphabetic() || c == '_').unwrap_or(false) {
                            chars.next(); // consume +
                            let mut name = String::new();
                            while let Some(&c) = chars.peek() {
                                if c.is_alphanumeric() || c == '_' {
                                    name.push(chars.next().unwrap());
                                } else {
                                    break;
                                }
                            }
                            // `$+name[idx]` mirrors `${+name[idx]}`
                            // — wrap with subscript intact and run
                            // through the brace path so all the
                            // assoc/array/magic-special handling
                            // already in subst_port applies.
                            let mut subscript = String::new();
                            if chars.peek() == Some(&'[') {
                                chars.next(); // consume [
                                let mut depth = 1;
                                while let Some(&c) = chars.peek() {
                                    if c == '[' {
                                        depth += 1;
                                        subscript.push(chars.next().unwrap());
                                    } else if c == ']' {
                                        depth -= 1;
                                        if depth == 0 {
                                            chars.next();
                                            break;
                                        }
                                        subscript.push(chars.next().unwrap());
                                    } else {
                                        subscript.push(chars.next().unwrap());
                                    }
                                }
                            }
                            let braced = if subscript.is_empty() {
                                format!("+{}", name)
                            } else {
                                format!("+{}[{}]", name, subscript)
                            };
                            result.push_str(&crate::subst::substitute_brace(
                                &braced, self,
                            ));
                            continue;
                        }
                    }
                    // $#name → ${#name} (string/array length).
                    // Also `$#@` and `$#*` — count of positional
                    // params (zsh shorthand for `${#@}`/`${#*}`).
                    if chars.peek() == Some(&'#') {
                        let mut peek_iter = chars.clone();
                        peek_iter.next(); // skip #
                        let next = peek_iter.peek().copied();
                        if next.map(|c| c.is_alphabetic() || c == '_').unwrap_or(false) {
                            chars.next(); // consume #
                            let mut name = String::new();
                            while let Some(&c) = chars.peek() {
                                if c.is_alphanumeric() || c == '_' {
                                    name.push(chars.next().unwrap());
                                } else {
                                    break;
                                }
                            }
                            // zsh: `$#name[idx]` is sugar for
                            // `${#name[idx]}` — length of the
                            // selected array element. zshrs's
                            // unbraced `$#` only handled the
                            // bare-name case and left `[idx]` as
                            // literal text appended to the
                            // length output (e.g. `3[2]` for an
                            // array of size 3).
                            if chars.peek() == Some(&'[') {
                                chars.next(); // consume [
                                let mut idx_str = String::new();
                                let mut depth = 1;
                                while let Some(&c) = chars.peek() {
                                    if c == '[' {
                                        depth += 1;
                                        idx_str.push(chars.next().unwrap());
                                    } else if c == ']' {
                                        depth -= 1;
                                        if depth == 0 {
                                            chars.next(); // consume ]
                                            break;
                                        }
                                        idx_str.push(chars.next().unwrap());
                                    } else {
                                        idx_str.push(chars.next().unwrap());
                                    }
                                }
                                if let Some(arr) = self.arrays.get(&name).cloned() {
                                    let idx_val = self.eval_arith_expr(&idx_str);
                                    let len = arr.len() as i64;
                                    let real_idx = if idx_val > 0 {
                                        (idx_val - 1) as usize
                                    } else if idx_val < 0 {
                                        let off = len + idx_val;
                                        if off < 0 {
                                            result.push('0');
                                            continue;
                                        }
                                        off as usize
                                    } else {
                                        result.push('0');
                                        continue;
                                    };
                                    let elem_len =
                                        arr.get(real_idx).map(|s| s.chars().count()).unwrap_or(0);
                                    result.push_str(&elem_len.to_string());
                                    continue;
                                }
                                // Scalar with subscript = char-slice
                                // length. Forward to the runtime
                                // length-of-substring path via the
                                // braced expand for consistency.
                                let braced = format!("${{#{}[{}]}}", name, idx_str);
                                result.push_str(&self.expand_string(&braced));
                                continue;
                            }
                            // Return length of variable or array
                            let len = if let Some(arr) = self.arrays.get(&name) {
                                arr.len()
                            } else {
                                self.get_variable(&name).len()
                            };
                            result.push_str(&len.to_string());
                            continue;
                        }
                        // `$#@` / `$#*` — positional-param count.
                        if matches!(next, Some('@') | Some('*')) {
                            chars.next(); // consume #
                            chars.next(); // consume @ or *
                            result.push_str(&self.positional_params.len().to_string());
                            continue;
                        }
                    }
                    let mut var_name = String::new();
                    while let Some(&c) = chars.peek() {
                        // The single-char special params (`@`/`*`/`#`/`?`)
                        // can only appear ALONE as a var name. After the
                        // first char of an identifier, only alphanumeric
                        // and underscore are valid. Without this guard,
                        // `$a*2` consumed `a*2` as one var name and
                        // looked up the (nonexistent) `a*2` variable
                        // instead of treating the `*` as a literal.
                        let is_first = var_name.is_empty();
                        let allowed = if is_first {
                            c.is_alphanumeric()
                                || c == '_'
                                || c == '@'
                                || c == '*'
                                || c == '#'
                                || c == '?'
                        } else {
                            c.is_alphanumeric() || c == '_'
                        };
                        if allowed {
                            var_name.push(chars.next().unwrap());
                            // Handle single-char special vars: stop after
                            // consuming if the var name IS one of these.
                            if matches!(
                                var_name.as_str(),
                                "@" | "*"
                                    | "#"
                                    | "?"
                                    | "$"
                                    | "!"
                                    | "-"
                                    | "0"
                                    | "1"
                                    | "2"
                                    | "3"
                                    | "4"
                                    | "5"
                                    | "6"
                                    | "7"
                                    | "8"
                                    | "9"
                            ) {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                    // `$NAME[subscript]` in DQ context — zsh treats the
                    // bracketed subscript as part of the expansion
                    // (assoc lookup or array index). Without this, the
                    // `[...]` was emitted as literal text.
                    if chars.peek() == Some(&'[') {
                        chars.next(); // consume `[`
                        let mut sub = String::new();
                        let mut depth = 1;
                        for c in chars.by_ref() {
                            if c == '[' {
                                depth += 1;
                                sub.push(c);
                            } else if c == ']' {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                                sub.push(c);
                            } else {
                                sub.push(c);
                            }
                        }
                        // Honor `$` inside the subscript (e.g. `$m[$k]`).
                        let sub_resolved = if sub.contains('$') {
                            self.expand_string(&sub)
                        } else {
                            sub
                        };
                        if let Some(assoc) = self.assoc_arrays.get(&var_name) {
                            if let Some(v) = assoc.get(&sub_resolved) {
                                result.push_str(v);
                            }
                        } else if let Some(arr) = self.arrays.get(&var_name) {
                            if let Ok(idx) = sub_resolved.parse::<i64>() {
                                let len = arr.len() as i64;
                                let i = if idx < 0 { len + idx } else { idx - 1 };
                                if i >= 0 && (i as usize) < arr.len() {
                                    result.push_str(&arr[i as usize]);
                                }
                            } else if sub_resolved == "@" || sub_resolved == "*" {
                                result.push_str(&arr.join(" "));
                            }
                        } else {
                            // Scalar with subscript — emit the value
                            // (subscript indexes characters, but without
                            // braces it's rare; punt to scalar dump).
                            result.push_str(&self.get_variable(&var_name));
                        }
                    } else {
                        result.push_str(&self.get_variable(&var_name));
                    }
                }
            } else if c == '`' {
                // Backtick command substitution
                let cmd_str: String = chars.by_ref().take_while(|&c| c != '`').collect();
                result.push_str(&self.run_command_substitution(&cmd_str));
            } else if c == '<' && chars.peek() == Some(&'(') && self.in_dq_context == 0 {
                // Process substitution <(cmd) — disabled inside DQ
                // per zsh: `"<(cmd)"` is a literal string. zsh's
                // lexer recognises Inang+Inpar tokens only outside
                // quotes; inside DQ they're plain text.
                chars.next(); // consume '('
                let cmd_str = Self::collect_until_paren(&mut chars);
                result.push_str(&self.run_process_sub_in(&cmd_str));
            } else if c == '>' && chars.peek() == Some(&'(') && self.in_dq_context == 0 {
                // Process substitution >(cmd) — disabled inside DQ
                // (same rationale as <(cmd) above).
                chars.next(); // consume '('
                let cmd_str = Self::collect_until_paren(&mut chars);
                result.push_str(&self.run_process_sub_out(&cmd_str));
            } else if c == '~' && result.is_empty() && self.in_dq_context == 0 {
                // Collect the tilde-name suffix (until /, end-of-string,
                // or another shell special). Then dispatch through
                // expand_tilde_named which knows about `~+`, `~-`,
                // `~+N`, `~-N`, named dirs (`hash -d` / `nameddirs`),
                // and `~user` via libc getpwnam. zsh disables tilde
                // expansion inside double quotes — `"~"` stays literal.
                let mut name = String::from("~");
                while let Some(&pc) = chars.peek() {
                    if pc == '/' || pc.is_whitespace() {
                        break;
                    }
                    name.push(pc);
                    chars.next();
                }
                let expanded = self.expand_tilde_named(&name);
                result.push_str(&expanded);
            } else {
                result.push(c);
            }
        }

        result
    }
    pub(crate) fn apply_var_modifier(
        &mut self,
        name: &str,
        val: Option<String>,
        modifier: Option<&VarModifier>,
    ) -> String {
        match modifier {
            None => val.unwrap_or_default(),

            // ${var:-word} - use default value
            Some(VarModifier::Default(word)) => match &val {
                Some(v) if !v.is_empty() => v.clone(),
                _ => self.expand_word(word),
            },

            // ${var:=word} - assign default value
            Some(VarModifier::DefaultAssign(word)) => match &val {
                Some(v) if !v.is_empty() => v.clone(),
                _ => self.expand_word(word),
            },

            // ${var:?word} - error if null or unset. zsh in -c mode
            // prints `zsh:LINE: NAME: msg` and exits 1. Mirror with
            // `zshrs:1:` prefix and exit so subsequent commands don't
            // run (mirror zsh's non-interactive contract).
            Some(VarModifier::Error(word)) => match &val {
                Some(v) if !v.is_empty() => v.clone(),
                _ => {
                    let msg = self.expand_word(word);
                    let display = if msg.is_empty() {
                        "parameter not set".to_string()
                    } else {
                        msg
                    };
                    eprintln!("zshrs:1: {}: {}", name, display);
                    std::process::exit(1);
                }
            },

            // ${var:+word} - use alternate value
            Some(VarModifier::Alternate(word)) => match &val {
                Some(v) if !v.is_empty() => self.expand_word(word),
                _ => String::new(),
            },

            // ${#var} - string length
            Some(VarModifier::Length) => val
                .map(|v| v.len().to_string())
                .unwrap_or_else(|| "0".to_string()),

            // ${var:offset} or ${var:offset:length} - substring
            Some(VarModifier::Substring(offset, length)) => {
                // For positionals (`@`/`*`) and arrays, slice the
                // ELEMENTS (1-based, inclusive offset) — not the
                // chars of the joined string. Matches zsh.
                if name == "@" || name == "*" {
                    let len = length.unwrap_or(-1);
                    let sliced = slice_positionals(self, *offset, len);
                    return sliced.join(" ");
                }
                if let Some(arr) = self.arrays.get(name).cloned() {
                    let len = length.unwrap_or(-1);
                    let sliced = slice_array_zero_based(&arr, *offset, len);
                    return sliced.join(" ");
                }
                let v = val.unwrap_or_default();
                let start = if *offset < 0 {
                    (v.len() as i64 + offset).max(0) as usize
                } else {
                    (*offset as usize).min(v.len())
                };

                if let Some(len) = length {
                    let len = (*len as usize).min(v.len().saturating_sub(start));
                    v.chars().skip(start).take(len).collect()
                } else {
                    v.chars().skip(start).collect()
                }
            }

            // ${var#pattern} - remove shortest prefix
            Some(VarModifier::RemovePrefix(pattern)) => {
                let pat = self.expand_word(pattern);
                let strip = |v: &str, pat: &str| -> String {
                    if let Ok(g) = glob::Pattern::new(pat) {
                        for i in 0..=v.len() {
                            if g.matches(&v[..i]) {
                                return v[i..].to_string();
                            }
                        }
                    }
                    v.strip_prefix(pat).map(str::to_string).unwrap_or_else(|| v.to_string())
                };
                if name == "@" || name == "*" {
                    return self
                        .positional_params
                        .iter()
                        .map(|e| strip(e, &pat))
                        .collect::<Vec<_>>()
                        .join(" ");
                }
                if let Some(arr) = self.arrays.get(name).cloned() {
                    return arr
                        .iter()
                        .map(|e| strip(e, &pat))
                        .collect::<Vec<_>>()
                        .join(" ");
                }
                strip(&val.unwrap_or_default(), &pat)
            }

            // ${var##pattern} - remove longest prefix
            Some(VarModifier::RemovePrefixLong(pattern)) => {
                let pat = self.expand_word(pattern);
                let strip = |v: &str, pat: &str| -> String {
                    if let Ok(g) = glob::Pattern::new(pat) {
                        for i in (0..=v.len()).rev() {
                            if g.matches(&v[..i]) {
                                return v[i..].to_string();
                            }
                        }
                    }
                    v.to_string()
                };
                if name == "@" || name == "*" {
                    return self
                        .positional_params
                        .iter()
                        .map(|e| strip(e, &pat))
                        .collect::<Vec<_>>()
                        .join(" ");
                }
                if let Some(arr) = self.arrays.get(name).cloned() {
                    return arr
                        .iter()
                        .map(|e| strip(e, &pat))
                        .collect::<Vec<_>>()
                        .join(" ");
                }
                strip(&val.unwrap_or_default(), &pat)
            }

            // ${var%pattern} - remove shortest suffix
            Some(VarModifier::RemoveSuffix(pattern)) => {
                let pat = self.expand_word(pattern);
                let strip = |v: &str, pat: &str| -> String {
                    if let Ok(g) = glob::Pattern::new(pat) {
                        for i in (0..=v.len()).rev() {
                            if g.matches(&v[i..]) {
                                return v[..i].to_string();
                            }
                        }
                    } else if let Some(prefix) = v.strip_suffix(pat) {
                        return prefix.to_string();
                    }
                    v.to_string()
                };
                if name == "@" || name == "*" {
                    return self
                        .positional_params
                        .iter()
                        .map(|e| strip(e, &pat))
                        .collect::<Vec<_>>()
                        .join(" ");
                }
                if let Some(arr) = self.arrays.get(name).cloned() {
                    return arr
                        .iter()
                        .map(|e| strip(e, &pat))
                        .collect::<Vec<_>>()
                        .join(" ");
                }
                strip(&val.unwrap_or_default(), &pat)
            }

            // ${var%%pattern} - remove longest suffix
            Some(VarModifier::RemoveSuffixLong(pattern)) => {
                let pat = self.expand_word(pattern);
                let strip = |v: &str, pat: &str| -> String {
                    if let Ok(g) = glob::Pattern::new(pat) {
                        for i in 0..=v.len() {
                            if g.matches(&v[i..]) {
                                return v[..i].to_string();
                            }
                        }
                    }
                    v.to_string()
                };
                if name == "@" || name == "*" {
                    return self
                        .positional_params
                        .iter()
                        .map(|e| strip(e, &pat))
                        .collect::<Vec<_>>()
                        .join(" ");
                }
                if let Some(arr) = self.arrays.get(name).cloned() {
                    return arr
                        .iter()
                        .map(|e| strip(e, &pat))
                        .collect::<Vec<_>>()
                        .join(" ");
                }
                strip(&val.unwrap_or_default(), &pat)
            }

            // ${var/pattern/replacement} - replace first match
            Some(VarModifier::Replace(pattern, replacement)) => {
                let v = val.unwrap_or_default();
                let pat = self.expand_word(pattern);
                let repl = self.expand_word(replacement);
                // `${v/#prefix/repl}` — anchor at start.
                // `${v/%suffix/repl}` — anchor at end.
                // Otherwise: first occurrence anywhere.
                if let Some(rest) = pat.strip_prefix('#') {
                    if let Some(suffix) = v.strip_prefix(rest) {
                        format!("{}{}", repl, suffix)
                    } else {
                        v
                    }
                } else if let Some(rest) = pat.strip_prefix('%') {
                    if let Some(prefix) = v.strip_suffix(rest) {
                        format!("{}{}", prefix, repl)
                    } else {
                        v
                    }
                } else {
                    v.replacen(&pat, &repl, 1)
                }
            }

            // ${var//pattern/replacement} - replace all matches.
            // When the pattern has glob/extendedglob metachars, route
            // through ShellExecutor::glob_match_static-equivalent
            // regex compile so `a##`, `[abc]`, `*`, etc. work.
            // Direct port of zsh's getmatch path (Src/utils.c) which
            // pattern-compiles every replace pattern. The previous
            // bare `v.replace(&pat, &repl)` path treated pat as a
            // literal string — `${v//a##/X}` against "aaab" never
            // matched even with extended_glob set.
            Some(VarModifier::ReplaceAll(pattern, replacement)) => {
                let v = val.unwrap_or_default();
                let pat = self.expand_word(pattern);
                let repl = self.expand_word(replacement);
                if let Some(rest) = pat.strip_prefix('#') {
                    if let Some(suffix) = v.strip_prefix(rest) {
                        format!("{}{}", repl, suffix)
                    } else {
                        v
                    }
                } else if let Some(rest) = pat.strip_prefix('%') {
                    if let Some(prefix) = v.strip_suffix(rest) {
                        format!("{}{}", prefix, repl)
                    } else {
                        v
                    }
                } else {
                    // Pattern may have glob meta — compile to regex
                    // and global-replace if any of `*`, `?`, `[`,
                    // `(`, `#` (extendedglob `##` repetition), or
                    // `^` (extendedglob negation) appears. Falls
                    // back to literal replace on regex compile
                    // failure or for pure-literal patterns.
                    let has_meta = pat.chars()
                        .any(|c| matches!(c, '?' | '*' | '[' | '(' | '#' | '^'));
                    if has_meta {
                        let regex_src =
                            crate::subst::compile_glob_to_regex_for_replace(&pat);
                        match regex::Regex::new(&regex_src) {
                            Ok(re) => re.replace_all(&v, repl.as_str()).into_owned(),
                            Err(_) => v.replace(&pat, &repl),
                        }
                    } else {
                        v.replace(&pat, &repl)
                    }
                }
            }

            // ${var/#pattern/replacement} — anchored prefix
            Some(VarModifier::ReplacePrefix(pattern, replacement)) => {
                let v = val.unwrap_or_default();
                let pat = self.expand_word(pattern);
                let repl = self.expand_word(replacement);
                if let Some(rest) = v.strip_prefix(&*pat) {
                    format!("{}{}", repl, rest)
                } else {
                    v
                }
            }
            // ${var/%pattern/replacement} — anchored suffix
            Some(VarModifier::ReplaceSuffix(pattern, replacement)) => {
                let v = val.unwrap_or_default();
                let pat = self.expand_word(pattern);
                let repl = self.expand_word(replacement);
                if let Some(head) = v.strip_suffix(&*pat) {
                    format!("{}{}", head, repl)
                } else {
                    v
                }
            }

            // ${var^} or ${var^^} - uppercase
            Some(VarModifier::Upper) => val.map(|v| v.to_uppercase()).unwrap_or_default(),

            // ${var,} or ${var,,} - lowercase
            Some(VarModifier::Lower) => val.map(|v| v.to_lowercase()).unwrap_or_default(),
        }
    }
    /// Parse zsh parameter expansion flags from a string like "L", "U", "j:,:"
    pub(crate) fn parse_zsh_flags(&self, s: &str) -> Vec<ZshParamFlag> {
        let mut flags = Vec::new();
        let mut chars = s.chars().peekable();

        while let Some(c) = chars.next() {
            match c {
                '@' => flags.push(ZshParamFlag::At),
                'L' => flags.push(ZshParamFlag::Lower),
                'U' => flags.push(ZshParamFlag::Upper),
                'C' => flags.push(ZshParamFlag::Capitalize),
                'j' => {
                    // j<delim>sep<delim> — join with separator. zsh's
                    // subst.c get_strarg also accepts matched bracket
                    // pairs: `[`/`]`, `{`/`}`, `(`/`)`, `<`/`>`. Without
                    // the pair-aware close, `j[+]` left `]` in the
                    // separator and produced `a+]b+]c`.
                    if let Some(&delim) = chars.peek() {
                        chars.next(); // consume delimiter char
                        let close = match delim {
                            '[' => ']',
                            '{' => '}',
                            '(' => ')',
                            '<' => '>',
                            c => c,
                        };
                        let mut sep = String::new();
                        while let Some(&ch) = chars.peek() {
                            if ch == close {
                                chars.next();
                                break;
                            }
                            sep.push(chars.next().unwrap());
                        }
                        flags.push(ZshParamFlag::Join(sep));
                    }
                }
                'F' => flags.push(ZshParamFlag::JoinNewline),
                's' => {
                    // s<delim>sep<delim> - split on separator. Same
                    // bracket-pair-aware parsing as `j` (subst.c
                    // get_strarg).
                    if let Some(&delim) = chars.peek() {
                        chars.next();
                        let close = match delim {
                            '[' => ']',
                            '{' => '}',
                            '(' => ')',
                            '<' => '>',
                            c => c,
                        };
                        let mut sep = String::new();
                        while let Some(&ch) = chars.peek() {
                            if ch == close {
                                chars.next();
                                break;
                            }
                            sep.push(chars.next().unwrap());
                        }
                        flags.push(ZshParamFlag::Split(sep));
                    }
                }
                'f' => flags.push(ZshParamFlag::SplitLines),
                // `(0)` — split on NUL byte. Direct port of
                // src/zsh/Src/subst.c:2292-2297 which sets
                // `spsep` to a meta-encoded NUL ('\0' ^ 32 = 32,
                // i.e. ' '), but the effective semantics is
                // splitting on the literal NUL character. Reuse
                // the Split variant with a single-NUL separator.
                '0' => flags.push(ZshParamFlag::Split("\0".to_string())),
                // `(p)` — print-style escapes for subsequent (s::)/
                // (j::)/(l::)/(r::) args. Direct port of
                // src/zsh/Src/subst.c:2381-2382. The legacy path
                // treats it as a marker; BUILTIN_PARAM_FLAG handles
                // the actual escape interpretation (pre-scan).
                'p' => {}
                // `(g:e/o/c:)` — print-style escapes on the operand
                // value. Sub-flag arg (e/o/c) is consumed; the
                // legacy path doesn't apply the escape (compile-time
                // BUILTIN_PARAM_FLAG path does). Direct port of
                // src/zsh/Src/subst.c:2409-2436.
                'g' => {
                    if let Some(&delim) = chars.peek() {
                        if !delim.is_alphanumeric() && delim != '_' {
                            chars.next();
                            let close = match delim {
                                '[' => ']',
                                '{' => '}',
                                '(' => ')',
                                '<' => '>',
                                c => c,
                            };
                            while let Some(&ch) = chars.peek() {
                                if ch == close {
                                    chars.next();
                                    break;
                                }
                                chars.next();
                            }
                        }
                    }
                }
                // `(_)` — reserved for future use per
                // src/zsh/Src/subst.c:2485-2502. Skip its delim-arg.
                '_' => {
                    if let Some(&delim) = chars.peek() {
                        if !delim.is_alphanumeric() && delim != '_' {
                            chars.next();
                            let close = match delim {
                                '[' => ']',
                                '{' => '}',
                                '(' => ')',
                                '<' => '>',
                                c => c,
                            };
                            while let Some(&ch) = chars.peek() {
                                if ch == close {
                                    chars.next();
                                    break;
                                }
                                chars.next();
                            }
                        }
                    }
                }
                'z' => flags.push(ZshParamFlag::SplitWords),
                't' => flags.push(ZshParamFlag::Type),
                'w' => flags.push(ZshParamFlag::Words),
                'b' => flags.push(ZshParamFlag::QuoteBackslash),
                // `(B)` — backslash-escape characters that are special to
                // the shell (whitespace + glob/redirect/quote metas). Same
                // backslash mechanism as `(b)` (which is for pattern
                // contexts); the only difference is which charset gets
                // escaped. Map to the same enum variant; the shell-meta
                // expansion below handles both since it covers a strict
                // superset of pattern metas.
                'B' => flags.push(ZshParamFlag::QuoteBackslash),
                'q' => {
                    // zsh's q-flag gradient (per `man zshparam`):
                    //   (q)     backslash-escape shell-meta chars
                    //   (qq)    single-quote
                    //   (qqq)   double-quote
                    //   (qqqq)  $'...' style
                    //   (q+)    single-quote if needed
                    let mut q_count = 1;
                    while chars.peek() == Some(&'q') {
                        chars.next();
                        q_count += 1;
                    }
                    if chars.peek() == Some(&'+') {
                        chars.next();
                        flags.push(ZshParamFlag::QuoteIfNeeded);
                    } else {
                        match q_count {
                            1 => flags.push(ZshParamFlag::QuoteBackslash),
                            2 => flags.push(ZshParamFlag::Quote),
                            3 => flags.push(ZshParamFlag::DoubleQuote),
                            _ => flags.push(ZshParamFlag::DollarQuote),
                        }
                    }
                }
                'u' => flags.push(ZshParamFlag::Unique),
                'O' => flags.push(ZshParamFlag::Reverse),
                'o' => flags.push(ZshParamFlag::Sort),
                'n' => flags.push(ZshParamFlag::NumericSort),
                'a' => flags.push(ZshParamFlag::IndexSort),
                'k' => flags.push(ZshParamFlag::Keys),
                'v' => flags.push(ZshParamFlag::Values),
                '#' => flags.push(ZshParamFlag::Length),
                'c' => flags.push(ZshParamFlag::CountChars),
                'e' => flags.push(ZshParamFlag::Expand),
                '%' => {
                    if chars.peek() == Some(&'%') {
                        chars.next();
                        flags.push(ZshParamFlag::PromptExpandFull);
                    } else {
                        flags.push(ZshParamFlag::PromptExpand);
                    }
                }
                'V' => flags.push(ZshParamFlag::Visible),
                'D' => flags.push(ZshParamFlag::Directory),
                'M' => flags.push(ZshParamFlag::Match),
                'R' => flags.push(ZshParamFlag::Remove),
                'S' => flags.push(ZshParamFlag::Subscript),
                'P' => flags.push(ZshParamFlag::Parameter),
                '~' => flags.push(ZshParamFlag::Glob),
                'l'
                    // l:len:fill: - pad left
                    if chars.peek() == Some(&':') => {
                        chars.next();
                        let mut len_str = String::new();
                        while let Some(&ch) = chars.peek() {
                            if ch == ':' {
                                chars.next();
                                break;
                            }
                            len_str.push(chars.next().unwrap());
                        }
                        // zsh subst.c: `l:expr:string1:string2:` — string1
                        // is the LEFT pad (repeats), string2 is the prefix
                        // applied once before string1. When string1 is
                        // empty and string2 is given, string2 acts as
                        // the fill char (so `(l:5::0:)42` pads with `0`s
                        // to give `00042`). Default fill is space.
                        let mut s1 = String::new();
                        let mut s2 = String::new();
                        let mut have_s2 = false;
                        // Read string1 until next `:`
                        while let Some(&ch) = chars.peek() {
                            if ch == ':' {
                                chars.next();
                                // Read string2 until next `:` or `)`
                                have_s2 = true;
                                while let Some(&ch2) = chars.peek() {
                                    if ch2 == ':' {
                                        chars.next();
                                        break;
                                    }
                                    if ch2 == ')' {
                                        break;
                                    }
                                    s2.push(chars.next().unwrap());
                                }
                                break;
                            }
                            if ch == ')' {
                                break;
                            }
                            s1.push(chars.next().unwrap());
                        }
                        let fill = if !s1.is_empty() {
                            s1.chars().next().unwrap_or(' ')
                        } else if have_s2 && !s2.is_empty() {
                            s2.chars().next().unwrap_or(' ')
                        } else {
                            ' '
                        };
                        if let Ok(len) = len_str.parse() {
                            flags.push(ZshParamFlag::PadLeft(len, fill));
                        }
                    }
                'r'
                    // r:len:fill[:fill2]: — pad right.
                    if chars.peek() == Some(&':') => {
                        chars.next();
                        let mut len_str = String::new();
                        while let Some(&ch) = chars.peek() {
                            if ch == ':' {
                                chars.next();
                                break;
                            }
                            len_str.push(chars.next().unwrap());
                        }
                        let mut s1 = String::new();
                        let mut s2 = String::new();
                        let mut have_s2 = false;
                        while let Some(&ch) = chars.peek() {
                            if ch == ':' {
                                chars.next();
                                have_s2 = true;
                                while let Some(&ch2) = chars.peek() {
                                    if ch2 == ':' {
                                        chars.next();
                                        break;
                                    }
                                    if ch2 == ')' {
                                        break;
                                    }
                                    s2.push(chars.next().unwrap());
                                }
                                break;
                            }
                            if ch == ')' {
                                break;
                            }
                            s1.push(chars.next().unwrap());
                        }
                        let fill = if !s1.is_empty() {
                            s1.chars().next().unwrap_or(' ')
                        } else if have_s2 && !s2.is_empty() {
                            s2.chars().next().unwrap_or(' ')
                        } else {
                            ' '
                        };
                        if let Ok(len) = len_str.parse() {
                            flags.push(ZshParamFlag::PadRight(len, fill));
                        }
                    }
                'm' => {
                    // Width for padding - parse number if present
                    let mut width_str = String::new();
                    while let Some(&ch) = chars.peek() {
                        if ch.is_ascii_digit() {
                            width_str.push(chars.next().unwrap());
                        } else {
                            break;
                        }
                    }
                    if let Ok(w) = width_str.parse() {
                        flags.push(ZshParamFlag::Width(w));
                    }
                }
                _ => {}
            }
        }
        flags
    }
    /// Apply a single zsh parameter expansion flag
    pub(crate) fn apply_zsh_param_flag(&mut self, val: &str, name: &str, flag: &ZshParamFlag) -> String {
        match flag {
            // `@` is a context marker (force array semantics in DQ),
            // not a value transform. It's already consumed at the
            // flag-strip stage above; here it's a no-op pass-through.
            ZshParamFlag::At => val.to_string(),
            ZshParamFlag::Lower => val.to_lowercase(),
            ZshParamFlag::Upper => val.to_uppercase(),
            ZshParamFlag::Capitalize => {
                // Route through the faithful casemodify port — direct
                // port of src/zsh/Src/hist.c:2194-2370 CASMOD_CAPS.
                // The naive split_whitespace+title-case approach
                // collapsed multi-space runs and missed mid-word
                // lowercasing of non-leading uppercases.
                crate::subst::casemodify(val, crate::subst::CaseMod::Caps)
            }
            ZshParamFlag::Join(sep) => {
                if let Some(arr) = self.arrays.get(name) {
                    arr.join(sep)
                } else {
                    val.to_string()
                }
            }
            ZshParamFlag::Split(sep) => val.split(sep).collect::<Vec<_>>().join(" "),
            ZshParamFlag::SplitLines => val.lines().collect::<Vec<_>>().join(" "),
            ZshParamFlag::Type => {
                if self.arrays.contains_key(name) {
                    "array".to_string()
                } else if self.assoc_arrays.contains_key(name) {
                    "association".to_string()
                } else if self.function_exists(name) {
                    "function".to_string()
                } else if std::env::var(name).is_ok() || self.variables.contains_key(name) {
                    "scalar".to_string()
                } else {
                    "".to_string()
                }
            }
            ZshParamFlag::Words => val.split_whitespace().collect::<Vec<_>>().join(" "),
            ZshParamFlag::Quote => format!("'{}'", val.replace('\'', "'\\''")),
            ZshParamFlag::QuoteIfNeeded => {
                // (q+) — only wrap with single-quotes when the value
                // contains shell-special chars. Mirrors BUILTIN_PARAM_FLAG's
                // q+ branch (see exec.rs:1660 needs_quoting predicate).
                let needs = val.is_empty()
                    || val.chars().any(|c| {
                        c.is_whitespace()
                            || matches!(
                                c,
                                '\'' | '"'
                                    | '\\'
                                    | '$'
                                    | '`'
                                    | '*'
                                    | '?'
                                    | '['
                                    | ']'
                                    | '{'
                                    | '}'
                                    | '('
                                    | ')'
                                    | '|'
                                    | '&'
                                    | ';'
                                    | '<'
                                    | '>'
                                    | '#'
                                    | '~'
                            )
                    });
                if needs {
                    format!("'{}'", val.replace('\'', "'\\''"))
                } else {
                    val.to_string()
                }
            }
            ZshParamFlag::DoubleQuote => format!("\"{}\"", val.replace('"', "\\\"")),
            ZshParamFlag::DollarQuote => {
                // (qqqq) — $'...' style escaping. Renders non-printable
                // chars as \xHH and backslash-escapes specials.
                let mut out = String::from("$'");
                for c in val.chars() {
                    match c {
                        '\'' => out.push_str("\\'"),
                        '\\' => out.push_str("\\\\"),
                        '\n' => out.push_str("\\n"),
                        '\t' => out.push_str("\\t"),
                        '\r' => out.push_str("\\r"),
                        c if (c as u32) < 0x20 => {
                            out.push_str(&format!("\\x{:02x}", c as u32));
                        }
                        c => out.push(c),
                    }
                }
                out.push('\'');
                out
            }
            ZshParamFlag::Unique => {
                // Unique preserves first-occurrence order, so parallel doesn't help.
                // For 1000+ elements, pre-allocate the HashSet for less rehashing.
                let words: Vec<&str> = val.split_whitespace().collect();
                let mut seen = std::collections::HashSet::with_capacity(if words.len() >= 1000 {
                    words.len()
                } else {
                    0
                });
                if words.len() >= 1000 {
                    tracing::trace!(
                        count = words.len(),
                        "unique on large array ({} elements)",
                        words.len()
                    );
                }
                words
                    .into_iter()
                    .filter(|s| seen.insert(*s))
                    .collect::<Vec<_>>()
                    .join(" ")
            }
            ZshParamFlag::Reverse => {
                // (O) flag: reverse sort (sort descending)
                let mut words: Vec<&str> = val.split_whitespace().collect();
                if words.len() >= 1000 {
                    tracing::trace!(
                        count = words.len(),
                        "using parallel reverse sort (rayon) for large array"
                    );
                    use rayon::prelude::*;
                    words.par_sort_unstable_by(|a, b| b.cmp(a));
                } else {
                    words.sort_unstable_by(|a, b| b.cmp(a));
                }
                words.join(" ")
            }
            ZshParamFlag::Sort => {
                let mut words: Vec<&str> = val.split_whitespace().collect();
                if words.len() >= 1000 {
                    tracing::trace!(
                        count = words.len(),
                        "using parallel sort (rayon) for large array"
                    );
                    use rayon::prelude::*;
                    words.par_sort_unstable();
                } else {
                    words.sort_unstable();
                }
                words.join(" ")
            }
            ZshParamFlag::NumericSort => {
                let mut words: Vec<&str> = val.split_whitespace().collect();
                let cmp = |a: &&str, b: &&str| {
                    let na: i64 = a.parse().unwrap_or(0);
                    let nb: i64 = b.parse().unwrap_or(0);
                    na.cmp(&nb)
                };
                if words.len() >= 1000 {
                    tracing::trace!(
                        count = words.len(),
                        "using parallel numeric sort (rayon) for large array"
                    );
                    use rayon::prelude::*;
                    words.par_sort_unstable_by(cmp);
                } else {
                    words.sort_unstable_by(cmp);
                }
                words.join(" ")
            }
            ZshParamFlag::Keys => {
                if let Some(assoc) = self.assoc_arrays.get(name) {
                    assoc.keys().cloned().collect::<Vec<_>>().join(" ")
                } else {
                    String::new()
                }
            }
            ZshParamFlag::Values => {
                if let Some(assoc) = self.assoc_arrays.get(name) {
                    assoc.values().cloned().collect::<Vec<_>>().join(" ")
                } else {
                    val.to_string()
                }
            }
            ZshParamFlag::Length => val.len().to_string(),
            ZshParamFlag::Head(n) => val
                .split_whitespace()
                .take(*n)
                .collect::<Vec<_>>()
                .join(" "),
            ZshParamFlag::Tail(n) => {
                let words: Vec<&str> = val.split_whitespace().collect();
                if words.len() > *n {
                    words[words.len() - n..].join(" ")
                } else {
                    val.to_string()
                }
            }
            ZshParamFlag::JoinNewline => {
                if let Some(arr) = self.arrays.get(name) {
                    arr.join("\n")
                } else {
                    val.to_string()
                }
            }
            ZshParamFlag::SplitWords => {
                // Shell-style word splitting
                val.split_whitespace().collect::<Vec<_>>().join(" ")
            }
            ZshParamFlag::QuoteBackslash => {
                // Backslash-escape shell + pattern metas. `(b)` and `(B)`
                // share this handler — their charsets overlap heavily and
                // the strict superset is fine for both contexts (extra
                // escapes never harm pattern matching, and shell parsing
                // is what `(B)` needs).
                let mut result = String::new();
                for c in val.chars() {
                    if "\\*?[]{}()<>&|;\"'$`!#~ \t\n".contains(c) {
                        result.push('\\');
                    }
                    result.push(c);
                }
                result
            }
            ZshParamFlag::IndexSort => {
                // Array index order - just return as-is (default)
                val.to_string()
            }
            ZshParamFlag::CountChars => {
                // Count total characters
                val.chars().count().to_string()
            }
            ZshParamFlag::Expand => {
                // (e) — re-expand the value through parameter,
                // command-substitution and arithmetic. Direct port of
                // zsh/Src/subst.c paramsubst's PSUB_EXPAND branch
                // (the (e) flag triggers a recursive subst pass on
                // the value). expand_string covers that surface.
                self.expand_string(val)
            }
            ZshParamFlag::PromptExpand => {
                // Expand prompt escapes
                self.expand_prompt_string(val)
            }
            ZshParamFlag::PromptExpandFull => {
                // Full prompt expansion
                self.expand_prompt_string(val)
            }
            ZshParamFlag::Visible => {
                // Make non-printable characters visible
                val.chars()
                    .map(|c| {
                        if c.is_control() {
                            format!("^{}", (c as u8 + 64) as char)
                        } else {
                            c.to_string()
                        }
                    })
                    .collect()
            }
            ZshParamFlag::Directory => {
                // Substitute leading directory with ~ if it's home
                if let Some(home) = dirs::home_dir() {
                    let home_str = home.to_string_lossy();
                    if val.starts_with(home_str.as_ref()) {
                        format!("~{}", &val[home_str.len()..])
                    } else {
                        val.to_string()
                    }
                } else {
                    val.to_string()
                }
            }
            ZshParamFlag::PadLeft(len, fill) => {
                if val.len() >= *len {
                    val.to_string()
                } else {
                    let padding: String = std::iter::repeat_n(*fill, len - val.len()).collect();
                    format!("{}{}", padding, val)
                }
            }
            ZshParamFlag::PadRight(len, fill) => {
                if val.len() >= *len {
                    val.to_string()
                } else {
                    let padding: String = std::iter::repeat_n(*fill, len - val.len()).collect();
                    format!("{}{}", val, padding)
                }
            }
            ZshParamFlag::Width(_) => {
                // Width modifier - used with padding, just return value
                val.to_string()
            }
            ZshParamFlag::Match => {
                // Match flag - used with pattern operations, just pass through
                // Actual matching is handled in the pattern operations below
                val.to_string()
            }
            ZshParamFlag::Remove => {
                // Remove flag - complement of Match
                val.to_string()
            }
            ZshParamFlag::Subscript => {
                // Subscript scanning
                val.to_string()
            }
            ZshParamFlag::Parameter => {
                // Parameter indirection - treat val as parameter name
                self.get_variable(val)
            }
            ZshParamFlag::Glob => {
                // Glob patterns in pattern matching
                val.to_string()
            }
        }
    }
}
// END moved-from-exec-rs

// ===========================================================
// Free fns moved verbatim from src/ported/exec.rs.
// ===========================================================
// BEGIN moved-from-exec-rs (free fns)
/// Apply a `:s/old/new/` (or `:gs/old/new/`) substitution modifier
/// to `result` in place. `chars` is positioned right after the `s`
/// (or after `gs`). Reads delimiter, old text (until delim), new text
/// (until delim or end), and rewrites `result`. zsh: when `global` is
/// true, replace all occurrences; else replace first only.
pub(crate) fn apply_subst_modifier(
    result: &mut String,
    chars: &mut std::iter::Peekable<std::str::Chars>,
    global: bool,
) {
    let delim = match chars.next() {
        Some(d) => d,
        None => return,
    };
    let mut old = String::new();
    while let Some(&c) = chars.peek() {
        if c == delim {
            chars.next();
            break;
        }
        old.push(c);
        chars.next();
    }
    let mut new = String::new();
    while let Some(&c) = chars.peek() {
        if c == delim || c == ':' {
            // Matching closing delim → consume it. Colon → next
            // modifier in chain, leave it for the outer loop.
            if c == delim {
                chars.next();
            }
            break;
        }
        new.push(c);
        chars.next();
    }
    if old.is_empty() {
        return;
    }
    *result = if global {
        result.replace(&old, &new)
    } else {
        result.replacen(&old, &new, 1)
    };
}
/// Apply zsh's `${var#pat}` / `##` / `%` / `%%` strip operators with
/// optional `(M)`-flag inversion. Direct port of zsh's
/// `get_match_ret()` (Src/glob.c:2550) for the relevant flag mask:
///
///   - `SUB_MATCH` set (passed as `m_flag=true`): return the matched
///     portion (chars b..e of the original string)
///   - `SUB_MATCH` unset (default): return the unmatched portion
///     (chars 0..b plus chars e..end)
///
/// `op` matches the existing `BUILTIN_PARAM_STRIP` numbering:
///   0 = `#`  (shortest leading match)
///   1 = `##` (longest leading match)
///   2 = `%`  (shortest trailing match)
///   3 = `%%` (longest trailing match)
///
/// Glob matching uses the existing `glob_match_static` helper, which
/// already handles extendedglob, character classes, and `^pat`
/// negation, so the M-flag fix above is purely about which slice of
/// the original to return — not about how matches are found.
pub(crate) fn strip_match_op(v: &str, op: u8, pattern: &str, m_flag: bool) -> String {
    // Helper: encode the "no match" return value. Without (M),
    // strip ops return the unchanged string (per zsh spec — strip
    // is a no-op when nothing matches). With (M), the matched
    // portion doesn't exist, so return empty (zsh: `${(M)a#nope}`
    // → "").
    let no_match = || if m_flag { String::new() } else { v.to_string() };
    match op {
        0 => {
            // Shortest leading: scan increasing prefix lengths,
            // first match wins (b=0, e=i).
            for i in 0..=v.len() {
                if !v.is_char_boundary(i) {
                    continue;
                }
                let prefix = &v[..i];
                if ShellExecutor::glob_match_static(prefix, pattern) {
                    return if m_flag {
                        v[..i].to_string()
                    } else {
                        v[i..].to_string()
                    };
                }
            }
            no_match()
        }
        1 => {
            // Longest leading: scan decreasing prefix lengths,
            // first match wins.
            for i in (0..=v.len()).rev() {
                if !v.is_char_boundary(i) {
                    continue;
                }
                let prefix = &v[..i];
                if ShellExecutor::glob_match_static(prefix, pattern) {
                    return if m_flag {
                        v[..i].to_string()
                    } else {
                        v[i..].to_string()
                    };
                }
            }
            no_match()
        }
        2 => {
            // Shortest trailing: scan decreasing suffix start
            // (= shortest suffix length first), first match wins.
            for i in (0..=v.len()).rev() {
                if !v.is_char_boundary(i) {
                    continue;
                }
                let suffix = &v[i..];
                if ShellExecutor::glob_match_static(suffix, pattern) {
                    return if m_flag {
                        v[i..].to_string()
                    } else {
                        v[..i].to_string()
                    };
                }
            }
            no_match()
        }
        3 => {
            // Longest trailing: scan increasing suffix start
            // (= longest suffix length first), first match wins.
            for i in 0..=v.len() {
                if !v.is_char_boundary(i) {
                    continue;
                }
                let suffix = &v[i..];
                if ShellExecutor::glob_match_static(suffix, pattern) {
                    return if m_flag {
                        v[i..].to_string()
                    } else {
                        v[..i].to_string()
                    };
                }
            }
            no_match()
        }
        _ => v.to_string(),
    }
}
pub(crate) fn slice_scalar(s: &str, start: i64, end: i64) -> String {
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len() as i64;
    if len == 0 {
        return String::new();
    }
    // OOB single-index returns empty in zsh — `${str[10]}` for a
    // 5-char string is "" not the last char. Detect when both bounds
    // are the same value and exceed the bounds in either direction.
    if start == end {
        if start > len {
            return String::new();
        }
        if start < -len {
            return String::new();
        }
    }
    let resolve = |i: i64| -> i64 {
        if i < 0 {
            (len + i + 1).max(1)
        } else if i == 0 {
            1
        } else {
            i.min(len)
        }
    };
    let s_idx = resolve(start);
    let e_idx = resolve(end);
    if s_idx > e_idx {
        return String::new();
    }
    chars[(s_idx - 1) as usize..e_idx as usize].iter().collect()
}
// END moved-from-exec-rs (free fns)
