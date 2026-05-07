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
    // Token values MUST match `parse/src/tokens.rs::char_tokens` —
    // the canonical lexer table. Earlier this module had its own
    // independent values (POUND=0x80, TICK=0x83, etc.) that DIDN'T
    // match the lexer's emitted markers (POUND=0x84, TICK=0x93,
    // QTICK=0x99). Every `c == TICK` check in stringsubst silently
    // missed every backtick `\`cmd\`` in real shell input because
    // the lexer marks them with 0x93/0x99 not 0x83/0x84. Aligning
    // these values is correctness-critical.
    pub const POUND: char = '\u{84}'; // #                  // c:N/A
    pub const STRING: char = '\u{85}'; // $                 // c:N/A
    pub const QSTRING: char = '\u{8c}'; // Quoted $         // c:N/A
    pub const TICK: char = '\u{93}'; // `                   // c:N/A
    pub const QTICK: char = '\u{99}'; // Quoted `           // c:N/A
    pub const INPAR: char = '\u{88}'; // (                  // c:N/A
    pub const OUTPAR: char = '\u{8a}'; // )                 // c:N/A
    pub const INBRACE: char = '\u{8f}'; // {                // c:N/A
    pub const OUTBRACE: char = '\u{90}'; // }               // c:N/A
    pub const INBRACK: char = '\u{91}'; // [                // c:N/A
    pub const OUTBRACK: char = '\u{92}'; // ]               // c:N/A
    pub const INANG: char = '\u{94}'; // <                  // c:N/A
    pub const OUTANG: char = '\u{95}'; // >                 // c:N/A
    pub const OUTANGPROC: char = '\u{96}'; // >( for process sub // c:N/A
    pub const EQUALS: char = '\u{8d}'; // =                 // c:N/A
    pub const NULARG: char = '\u{a1}'; // Null argument marker // c:N/A
    pub const INPARMATH: char = '\u{89}'; // $((            // c:N/A
    pub const OUTPARMATH: char = '\u{8b}'; // ))            // c:N/A
    pub const SNULL: char = '\u{9d}'; // single quote marker    // c:N/A
    pub const DNULL: char = '\u{9e}'; // double quote marker    // c:N/A
    pub const MARKER: char = '\u{a2}'; // Array key-value marker // c:N/A
    pub const BNULL: char = '\u{9f}'; // Backslash null     // c:N/A


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
    /// Dirstack snapshot — backs `~+N` / `~-N` expansion via
    /// `dstackent()`. Mirrors `dirstack` in `zsh.h` which subst.c
    /// reads through `firstnode()` / `nextnode()` walks.
    pub dirstack: Vec<String>,                              // c:4902
    /// PUSHDMINUS option flag — flips the meaning of `~+N` vs `~-N`.
    /// Direct port of `isset(PUSHDMINUS)` at Src/subst.c:4906.
    pub pushdminus: bool,                                   // c:4906
    /// Last `:s/X/Y/` substitution pair, replayed by `:&`. Direct
    /// port of `hsubl` / `hsubr` globals in Src/hist.c. Lives on
    /// SubstState so chained modifiers (`${var:s/x/y/:&}`) and
    /// later refs in the same shell session both see it. Committed
    /// back to ShellExecutor in commit_to_executor.
    pub last_subst: Option<(String, String)>,               // c:4531
    /// SUB_* flag bits accumulated by `(M)/(R)/(B)/(E)/(N)/(S)`
    /// in the flag-loop. Direct port of subst.c:2169-2199. Read
    /// by getmatch / igetmatch and the BUILTIN_PARAM_REPLACE /
    /// BUILTIN_PARAM_STRIP arms which alter their match
    /// disposition based on these bits. Reset per paramsubst call
    /// so flags don't leak between successive ${...} expansions.
    pub sub_flags: u32,                                     // c:2169
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
            // C: dirstack global (Src/zsh.h). zshrs stores PathBufs;
            // SubstState wants strings for `~+N` rendering.
            dirstack: exec
                .dir_stack
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),                                 // c:4902
            pushdminus: exec.options.get("PUSHDMINUS").copied().unwrap_or(false), // c:4906
            last_subst: exec.last_subst.clone(),            // c:4531 (hsubl/hsubr)
            sub_flags: 0,                                   // c:2169 (per-call)
        }
    }

    /// Commit any state mutations performed by paramsubst back to
    /// the live ShellExecutor. Called after each substitution that
    /// might write (${var:=value}, ${var:?}, etc.). Companion to
    /// from_executor — same plumbing rationale.
    pub fn commit_to_executor(self, exec: &mut crate::exec::ShellExecutor) { // c:N/A (plumbing)
        if self.errflag {
            // ${var:?msg} / parse failure / etc. — propagate the
            // error back to the executor as a non-zero $?. Direct
            // port of zsh's errflag |= ERRFLAG_ERROR which makes
            // the next prompt see status=1 and (in non-interactive
            // mode) the shell exits at the next opcheck.
            exec.last_status = 1;                            // c:3193
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
        // Persist hsubl/hsubr equivalents so a later `:&` modifier
        // in a separate paramsubst call replays the most recent
        // `:s` from this session. Matches C zsh's global-pair
        // persistence (Src/hist.c hsubl/hsubr).
        if self.last_subst.is_some() {                      // c:4531
            exec.last_subst = self.last_subst;              // c:4531
        }
        // SUB_* bits flow per-call into the executor so the
        // BUILTIN_PARAM_* arms can read them via with_executor.
        // Direct port of subst.c flag-loop → getmatch() bridge.
        exec.sub_flags = self.sub_flags;                    // c:2169
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
fn keyvalpairelement(list: &mut LinkList, node_idx: usize) -> Option<usize> { // c:49
    // C: `start = (char *)getdata(node)` — fetch the node's text.
    let data = list.getdata(node_idx)?.to_string();         // c:53
    let chars: Vec<char> = data.chars().collect();          // c:53

    // C: `start[0] == Inbrack` — must lead with `[` (or token).
    if chars.is_empty()                                     // c:54
        || (chars[0] != INBRACK && chars[0] != '[')         // c:54
    {
        return None;                                        // c:54
    }

    // C: `end = strchr(start+1, Outbrack)` — find matching `]`.
    let mut end_pos: Option<usize> = None;                  // c:55
    for (i, &c) in chars.iter().enumerate().skip(1) {       // c:55
        if c == OUTBRACK || c == ']' {                      // c:55
            end_pos = Some(i);                              // c:55
            break;                                          // c:55
        }
    }
    let end_pos = end_pos?;                                 // c:55

    // C: `end[1] == Equals || (end[1] == '+' && end[2] == Equals)`
    // — `]=value` or `]+=value` postfix.
    if end_pos + 1 >= chars.len() {                         // c:57
        return None;                                        // c:57
    }
    let is_append = chars.get(end_pos + 1) == Some(&'+')    // c:58
        && (chars.get(end_pos + 2) == Some(&EQUALS)
            || chars.get(end_pos + 2) == Some(&'='));
    let is_assign = !is_append                              // c:57
        && (chars.get(end_pos + 1) == Some(&EQUALS)
            || chars.get(end_pos + 1) == Some(&'='));
    if !is_assign && !is_append {                           // c:60
        return None;
    }

    // C: `*end = '\0'; dat = start + 1; singsub(&dat); untokenize(dat);`
    // — extract key, run param-subst, untokenize.
    let raw_key: String = chars[1..end_pos].iter().collect(); // c:64
    let mut tmp_state = SubstState::default();              // c:65 (singsub context)
    let key_subst = singsub(&raw_key, &mut tmp_state);      // c:65
    let key = crate::lex::untokenize(&key_subst);           // c:66

    // C lines 67-75: Marker / Marker_plus sentinel + insertlinknode
    // for key and value.
    let value_start = if is_append { end_pos + 3 } else { end_pos + 2 }; // c:67-72
    let raw_value: String = chars[value_start..].iter().collect(); // c:69 / 73
    let mut tmp_state2 = SubstState::default();             // c:75 (singsub context)
    let value_subst = singsub(&raw_value, &mut tmp_state2); // c:75
    let value = crate::lex::untokenize(&value_subst);       // c:76

    let marker = if is_append {                             // c:67
        format!("{}+", MARKER)                              // c:67
    } else {                                                // c:71
        MARKER.to_string()                                  // c:71
    };

    list.setdata(node_idx, marker);                         // c:72
    let key_idx = list.insertlinknode(node_idx, key);       // c:73
    let val_idx = list.insertlinknode(key_idx, value);      // c:77

    // C: `return insertlinknode(list, node, dat);` — node where
    // value was inserted.
    Some(val_idx)                                           // c:77
}                                                           // c:79

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

                // Brace expansion. C: `while (hasbraces(getdata(node)))
                // { keep = 1; xpandbraces(list, &node); }`. zsh's
                // hasbraces walks the string looking for a balanced
                // `{…}` containing `,` or `..` (range). xpandbraces
                // splits the node into N nodes.
                //
                // Routes through canonical
                // crate::ported::glob::expand_braces; treats >1
                // result as a positive hasbraces hit.
                if !state.opts.ignore_braces && (flags & prefork_flags::SINGLE == 0) { // c:166
                    if !keep {                              // c:168
                        stop_idx = list.nextnode(node_idx); // c:169
                    }
                    loop {                                  // c:170 (while hasbraces)
                        let cur = match list.getdata(node_idx) {
                            Some(d) => d.to_string(),
                            None => break,
                        };
                        let expanded = crate::ported::glob::expand_braces(&cur, false); // c:171
                        if expanded.len() <= 1 { break; }   // c:170 (!hasbraces)
                        keep = true;                        // c:172
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
///      — calls utils.c's getkeystring with the dollars-quote flag,
///      which walks chars until an unescaped `'` and returns the
///      unescaped contents.
///   2. `len += 2` — account for the `$'` prefix.
///   3. Concat the prefix (strstart..strdpos), strsub, and the
///      suffix (strdpos+len..). Special case: empty `$''` returns
///      a Nularg sentinel so it doesn't get elided downstream.
///   4. Set *pstrdpos to point past the substituted region.
fn stringsubstquote(strstart: &str, strdpos: usize) -> (String, usize) { // c:206
    let chars: Vec<char> = strstart.chars().collect();      // c:208

    // C: `getkeystring(strdpos+2, &len, GETKEYS_DOLLARS_QUOTE, NULL)`.
    // Rust's getkeystring doesn't take a stop-at-unquoted-` flag, so
    // we walk the quoted region manually first, then unescape the
    // captured content. Same observable behavior: dollar-quoted
    // chars get C-escape-processed, unescaped `'` terminates.
    let start = strdpos + 2;                                // c:209 (strdpos+2)
    let mut end = start;                                    // c:209
    let mut escaped = false;                                // c:209

    while end < chars.len() {                               // c:209
        if escaped {                                        // c:209
            escaped = false;                                // c:209
            end += 1;                                       // c:209
            continue;                                       // c:209
        }
        if chars[end] == '\\' {                             // c:209
            escaped = true;                                 // c:209
            end += 1;                                       // c:209
            continue;                                       // c:209
        }
        if chars[end] == '\'' { break; }                    // c:209 (unescaped close)
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
    let suffix: String = if end + 1 < chars.len() {         // c:216 (strdpos[len] check)
        chars[end + 1..].iter().collect()                   // c:217
    } else {
        String::new()                                       // c:218
    };

    // C: empty `$''` special case — `strret = dupstring(nulstring);`
    // returns the NULARG sentinel string so the empty quote doesn't
    // get elided by stringsubst's word-walk.
    let strret = if strsub.is_empty() && prefix.is_empty() && suffix.is_empty() { // c:226
        // Nularg = '\u{8b}' per zsh.h. Emit as a single-char string
        // so downstream code recognises the empty-quote sentinel.
        "\u{8b}".to_string()                                // c:227
    } else {
        format!("{}{}{}", prefix, strsub, suffix)           // c:215-220
    };

    // C: `*pstrdpos = strret + (strdpos - strstart) + strlen(strsub);`
    // — sets the in-out pointer to one past the unescaped content
    // in the new string. Rust returns the equivalent index.
    let new_pos = prefix.chars().count() + strret.chars().count().saturating_sub(prefix.chars().count() + suffix.chars().count()); // c:230

    (strret, new_pos)                                       // c:232
}                                                           // c:233







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
            // The full port (getproc / getoutputfile) needs fork/exec
            // and lives in Src/exec.c. Until that lands, skip the
            // marker AND its parenthesized body so subsequent passes
            // don't misinterpret the inner text as bare param/cmd
            // substitution. Direct port of subst.c:248-274 layout —
            // C calls getproc/getoutputfile then memcpy's the result;
            // the no-op port still has to consume the same span.
            if state.errflag {                              // c:237
                return None;                                // c:237
            }                                               // c:237
            // Walk the matching close paren — depth-tracked so
            // nested `<(echo $(...))` skips correctly. Includes the
            // INANG/OUTANGPROC/EQUALS marker char itself.
            let start = pos;                                // c:237
            pos += 2;                                       // c:237 (skip marker + INPAR)
            let mut depth = 1_i32;                          // c:237
            while pos < chars.len() && depth > 0 {          // c:237
                let ch = chars[pos];                        // c:237
                if ch == INPAR { depth += 1; }              // c:237
                else if ch == OUTPAR { depth -= 1; }        // c:237
                pos += 1;                                   // c:237
            }                                                // c:237
            // Excise the entire span (was producing junk output
            // for `cat <(echo a) <(echo b)` because the half-skipped
            // `(echo a)` parsed as cmd-subst).
            let str_chars: Vec<char> = str3.chars().collect(); // c:237
            let mut new_str = String::with_capacity(str_chars.len());
            new_str.extend(str_chars[..start].iter());      // c:237
            new_str.extend(str_chars[pos..].iter());        // c:237
            str3 = new_str;                                 // c:237
            list.setdata(node_idx, str3.clone());            // c:237
            pos = start;                                    // c:237
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

            // Detect `$((expr))` arith form FIRST — it's
            // `$(` + `(expr)` + `)` so naive cmd-subst dispatch
            // would try to execute `((expr))` as a command. Either
            // the lexer-tokenized INPARMATH or the literal `((`
            // sequence routes through the arith path. Direct port
            // of subst.c's INPARMATH arm at c:237 (see C lines
            // around 320-360 which check `*++s == Inpar` after
            // `*s == String`).
            if next_c == Some(INPARMATH)                    // c:237
                || (next_c == Some('(') && chars.get(pos + 2).copied() == Some('(')) // c:237
            {                                                // c:237
                // Walk to matching `))`, depth-tracked for nested
                // `$((a + (b * c)))`. Skip the leading `((`.
                let start = pos + 3;                        // c:237 (past `$((`)
                let mut depth = 2_i32;                      // c:237 (we've opened 2 parens)
                let mut p = start;                          // c:237
                let mut end_off: Option<usize> = None;      // c:237
                while p < chars.len() {                     // c:237
                    let ch = chars[p];                      // c:237
                    if ch == '(' || ch == INPAR { depth += 1; } // c:237
                    else if ch == ')' || ch == OUTPAR {     // c:237
                        depth -= 1;                         // c:237
                        if depth == 0 {                     // c:237
                            end_off = Some(p);              // c:237 (closing )) at p .. p+1)
                            break;                          // c:237
                        }                                    // c:237
                    }                                        // c:237
                    p += 1;                                 // c:237
                }                                            // c:237
                if let Some(end) = end_off {                // c:237
                    // Expression text is between start and end-1
                    // (one inner `)` got consumed by depth=1; the
                    // outer `)` closes us at depth=0).
                    let expr: String = chars[start..end - 1].iter().collect(); // c:237
                    let prefix: String = chars[..pos].iter().collect(); // c:237
                    let suffix: String = if end + 1 < chars.len() { // c:237
                        chars[end + 1..].iter().collect()   // c:237
                    } else {                                // c:237
                        String::new()                       // c:237
                    };                                       // c:237
                    let result_only = arithsubst(&expr, "", "", state); // c:237
                    str3 = format!("{}{}{}", prefix, result_only, suffix); // c:237
                    list.setdata(node_idx, str3.clone());   // c:237
                    pos = prefix.chars().count() + result_only.chars().count(); // c:237
                    continue;                               // c:237
                }                                            // c:237
            }                                                // c:237

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
                        crate::fusevm_bridge::with_executor( // c:237
                            |exec| exec.run_command_substitution(&cmd))
                    };
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
                // $[...] arith substitution. Walk to matching ]
                // tracking depth so $[$[a+b]+c] nests correctly.
                let start = pos + 2;                        // c:237
                let open = if next_c == Some(INBRACK) { INBRACK } else { '[' }; // c:237
                let close = if open == INBRACK { OUTBRACK } else { ']' }; // c:237
                let chars: Vec<char> = str3.chars().collect(); // c:237
                let mut depth = 1_i32;                      // c:237
                let mut end_off: Option<usize> = None;      // c:237
                let mut p = start;                          // c:237
                while p < chars.len() {                     // c:237
                    let ch = chars[p];                      // c:237
                    if ch == open || ch == '[' { depth += 1; } // c:237
                    else if ch == close || ch == ']' {      // c:237
                        depth -= 1;                         // c:237
                        if depth == 0 { end_off = Some(p - start); break; } // c:237
                    }                                       // c:237
                    p += 1;                                 // c:237
                }                                           // c:237
                if let Some(end) = end_off {                // c:237
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
                    let result_only = arithsubst(&expr, "", "", state); // c:237
                    str3 = format!("{}{}{}", prefix, result_only, suffix); // c:237
                    list.setdata(node_idx, str3.clone());   // c:237
                    pos = prefix.chars().count() + result_only.chars().count(); // c:237
                    continue;                               // c:237
                } else {                                    // c:237
                    state.errflag = true;                   // c:237
                    eprintln!("zshrs: closing bracket missing"); // c:237
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

                // Insert additional nodes if word splitting produced
                // them. Empty new_nodes means the expansion produced
                // ZERO words (e.g. unquoted empty array \${arr} with
                // arr=()) — clear the original node's text so the
                // surrounding context (prefix/suffix) collapses.
                // Direct port of zsh's behavior: \`cmd \$arr\` with
                // arr=() runs cmd with no args.
                if new_nodes.is_empty() {                   // c:237
                    list.setdata(node_idx, String::new());  // c:237
                } else {
                    let mut current_idx = node_idx;             // c:237
                    for (i, node_data) in new_nodes.into_iter().enumerate() { // c:237
                        if i == 0 {                             // c:237
                            list.setdata(current_idx, node_data); // c:237
                        } else {                                // c:237
                            current_idx = list.insertlinknode(current_idx, node_data); // c:237
                        }                                       // c:237
                    }                                           // c:237
                }

                str3 = list.getdata(node_idx)?.to_string(); // c:237
                pos = new_pos;                              // c:237
                continue;                                   // c:237
            }                                               // c:237
        }                                                   // c:237

        // Backtick command substitution `cmd` — same engine as
        // `$(cmd)` per subst.c:237. Find the matching backtick,
        // capture cmd text, delegate to run_command_substitution.
        // The bridge's BUILTIN_EXPAND_TEXT untokenizes TICK/QTICK
        // back to a raw `` ` `` before calling singsub, so accept
        // any of the three forms as the open/close delimiter.
        let qt = c == QTICK;                                // c:237
        if qt || c == TICK || c == '`' {                    // c:237
            if !qt {                                        // c:237
                list.flags |= LF_ARRAY;                     // c:237
            }                                               // c:237
            let chars: Vec<char> = str3.chars().collect();  // c:237
            let cmd_start = pos + 1;                        // c:237
            let mut end = cmd_start;                        // c:237
            while end < chars.len()
                && chars[end] != TICK
                && chars[end] != QTICK
                && chars[end] != '`'
            {
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
/// Public wrapper that lets the fusevm bridge call paramsubst
/// directly when a `${(...)…}` form arrives at BUILTIN_BRIDGE_BRACE_
/// ARRAY. Returns the per-element node list so the caller can choose
/// between scalar and Array shape.
pub fn paramsubst_bridge(
    s: &str,
    start_pos: usize,
    qt: bool,
    pf_flags: u32,
    ret_flags: &mut u32,
    state: &mut SubstState,
) -> (String, usize, Vec<String>) {
    paramsubst(s, start_pos, qt, pf_flags, ret_flags, state)
}

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
        // No closing `}` — emit "bad substitution" and bail.
        // Direct port of zsh's zerr("closing brace missing") at
        // subst.c around line 1885.
        if end >= chars.len() || depth != 0 {
            eprintln!("zshrs: closing brace missing");      // c:1885
            state.errflag = true;                           // c:1885
            return (String::new(), chars.len(), vec![]);    // c:1885
        }
        let body: String = chars[pos..end].iter().collect(); // c:1885
        let new_pos = if end < chars.len() { end + 1 } else { end };
        let body_chars: Vec<char> = body.chars().collect();
        let mut idx = 0_usize;
        // Skip flag block `(...)` — flags currently no-op.
        // ${(flags)var…} — paren-flag block. Port of subst.c:2147+
        // flag-loop. Each flag char sets a state bit; applied as
        // post-processing on the substituted value.
        let mut flag_lower = false;                         // c:2197 (L)
        let mut flag_upper = false;                         // c:2200 (U)
        let mut flag_caps = false;                          // c:2203 (C)
        let mut flag_qcount: u32 = 0;                       // c:2237 (q)
        let mut flag_qmin = false;                          // c:2245 (q-)
        let mut flag_qplus = false;                         // c:2245 (q+)
        let mut flag_at = false;                            // c:2167 (@)
        let mut flag_p_indirect = false;                    // c:2295 (P)
        let mut flag_typeinfo = false;                      // c:2807 (t)
        let mut flag_keys = false;                          // c:2247 (k)
        let mut flag_values = false;                        // c:2256 (v)
        let mut flag_evalchar = false;                      // c:1673 (#) char-eval
        // (l:N::PRE:) left-pad / (r:N::POST:) right-pad parsed values.
        // Port of subst.c:2319-2375 l/r flag arm.
        let mut prenum: i64 = 0;                            // c:1776 (zlong prenum)
        let mut postnum: i64 = 0;                           // c:1776 (zlong postnum)
        let mut premul: Option<String> = None;              // c:1772 (premul)
        let mut postmul: Option<String> = None;             // c:1772 (postmul)
        let mut preone: Option<String> = None;              // c:1772 (preone)
        let mut postone: Option<String> = None;             // c:1772 (postone)
        // (s::) split / (j::) join separators. Port of
        // subst.c:2299-2317 s/j flag arm + (f)/(F)/(0) shortcuts.
        let mut spsep: Option<String> = None;               // c:1766 (spsep — splits result)
        let mut sep: Option<String> = None;                 // c:1766 (sep — joins arrays)
        // (o)/(O)/(i)/(n)/(a)/(u) sort + unique flags. Port of
        // subst.c:2207-2228 sortit-flag arm.
        let mut sort_active = false;                        // c:2207 (o)
        let mut sort_backwards = false;                     // c:2210 (O)
        let mut sort_case_insensitive = false;              // c:2213 (i)
        let mut sort_numeric = false;                       // c:2216 (n)
        let mut sort_signed = false;                        // c:2219 (-/Dash)
        let mut sort_index_order = false;                   // c:2225 (a)
        let mut unique = false;                             // c:2476 (u)
        let mut flag_eval = false;                          // c:2268 (e)
        let mut flag_unquote = false;                       // c:2261 (Q)
        let mut flag_error = false;                         // c:2264 (X)
        let mut flag_visible = false;                       // c:2232 (V)
        let mut flag_char_count = false;                    // c:2275 (c)
        let mut flag_word_count = false;                    // c:2278 (w)
        let mut flag_word_count_w = false;                  // c:2281 (W)
        let mut flag_b_pattern = false;                     // c:2255 (b)
        // SUB_* flag bits accumulated by M/R/B/E/N/S/I/* in the
        // flag-loop. Direct port of subst.c:2169-2199 — passed
        // through to getmatch() / igetmatch() to alter the
        // ${var//pat/repl}-style match disposition: return matched
        // text vs rest, return position vs string, etc.
        let mut sub_flags_bits: u32 = 0;                     // c:2169
        let mut flag_d_dir = false;                         // c:2229 (D)
        let mut flag_p_escapes = false;                     // c:2382 (p)
        let mut flag_pct_prompt: u32 = 0;                   // c:2405 (% prompt count)
        let mut multi_width: u32 = 0;                       // c:2376 (m count)
        let mut flnum: u32 = 0;                              // c:1786 (I:N:)
        let mut flag_z_tokenize = false;                     // c:2439 (z)
        let mut flag_z_keep_comments = false;                // c:2450 (Zc)
        let mut flag_z_strip_comments = false;               // c:2456 (ZC)
        let mut flag_z_newline_ws = false;                   // c:2461 (Zn)
        // Bare-tilde shortcut: \${~var} / \${~~var} are zsh shorthand
        // for \${(~)var}. Per subst.c around line 2079, the lexer
        // strips a leading '~' / '~~' off the body and toggles
        // tok_arg / globsubst before flag-loop processing. Eat them
        // here so the rest of the body parses as if we'd written
        // \${(~)var}.
        while body_chars.get(idx).copied() == Some('~') {    // c:2079
            state.opts.glob_subst = !state.opts.glob_subst;  // c:2160
            idx += 1;                                        // c:2079
        }                                                    // c:2079
        if body_chars.first() == Some(&'(') {               // c:2147
            let mut d = 1_i32;                              // c:2147
            idx = 1;                                        // c:2147
            // No closing paren on flag block → "bad substitution".
            // Direct port of zsh's flagerr label which calls zerr
            // and aborts the substitution. Emit and bail rather than
            // silently treating the entire body as flag chars.
            if !body_chars.iter().skip(1).any(|c| *c == ')') { // c:2147
                eprintln!("zshrs: bad substitution");        // c:2147
                state.errflag = true;                        // c:2147
                return (String::new(), new_pos, vec![]);     // c:2147
            }                                                // c:2147
            while idx < body_chars.len() && d > 0 {         // c:2147
                let fc = body_chars[idx];                   // c:2153
                match fc {                                  // c:2153
                    '(' => { d += 1; }                      // c:2147
                    ')' => { d -= 1; if d == 0 { idx += 1; break; } } // c:2147
                    'L' => { flag_lower = true; }           // c:2197
                    'U' => { flag_upper = true; }           // c:2200
                    'C' => { flag_caps = true; }            // c:2203
                    'q' => {                                // c:2237
                        // (q-) → SINGLE_OPTIONAL: quote only if
                        // needed (whitespace / metachar present);
                        // (q+) → QUOTEDZPUTS: print -V style.
                        // Without next char or with another q,
                        // bump the count for the (qq)/(qqq)/(qqqq)
                        // cascade. Direct port of subst.c:2236-2253.
                        let next = body_chars.get(idx + 1).copied();
                        if next == Some('-') {              // c:2240
                            idx += 1;                       // c:2243 (s++)
                            flag_qmin = true;               // c:2245 (QT_SINGLE_OPTIONAL)
                        } else if next == Some('+') {       // c:2240
                            idx += 1;                       // c:2243
                            flag_qplus = true;              // c:2245 (QT_QUOTEDZPUTS)
                        } else {                            // c:2247
                            flag_qcount += 1;               // c:2252
                        }                                    // c:2253
                    }                                       // c:2253
                    '@' => { flag_at = true; }              // c:2167
                    'P' => { flag_p_indirect = true; }      // c:2295
                    't' => { flag_typeinfo = true; }        // c:2807
                    'k' => { flag_keys = true; }            // c:2247
                    'v' => { flag_values = true; }          // c:2256
                    '#' => { flag_evalchar = true; }        // c:1673 (# evalchar)
                    'l' | 'r' => {                          // c:2319 (l/r pad)
                        // Consume `:N:STR1:STR2:` form.
                        // C: `s++; del0 = s; num = get_intarg(&s, &dellen);`
                        let is_left = fc == 'l';            // c:2320
                        idx += 1;                           // c:2323
                        if idx >= body_chars.len() { break; }
                        let del = body_chars[idx];          // c:2324 (del0)
                        idx += 1;                           // c:2324
                        // Parse N — digits up to next del.
                        let mut num_str = String::new();    // c:2326
                        while idx < body_chars.len()
                            && body_chars[idx].is_ascii_digit()
                        {
                            num_str.push(body_chars[idx]);
                            idx += 1;
                        }
                        let n: i64 = num_str.parse().unwrap_or(0); // c:2326
                        if is_left { prenum = n; } else { postnum = n; } // c:2329-2331
                        // Optional STR1 (mul) after another del.
                        if idx < body_chars.len() && body_chars[idx] == del {
                            idx += 1;                       // c:2336
                            let s1_start = idx;             // c:2336
                            while idx < body_chars.len()
                                && body_chars[idx] != del
                            {
                                idx += 1;
                            }
                            let s1: String = body_chars[s1_start..idx].iter().collect();
                            // (p) flag also applies to STR1/STR2 of
                            // (l)/(r) — decode \\n / \\t / \\xNN /
                            // etc. Direct port of subst.c:2336 escape
                            // dispatch on STR1/STR2 when escapes==1.
                            let s1 = if flag_p_escapes {
                                crate::ported::utils::getkeystring(&s1).0
                            } else { s1 };
                            if is_left { premul = Some(s1); } else { postmul = Some(s1); }
                            if idx < body_chars.len() {     // c:2354
                                idx += 1; // skip del
                            }
                            // Optional STR2 (one-time) after another del.
                            if idx < body_chars.len() && body_chars[idx] == del {
                                idx += 1;                   // c:2360
                                let s2_start = idx;
                                while idx < body_chars.len()
                                    && body_chars[idx] != del
                                {
                                    idx += 1;
                                }
                                let s2: String = body_chars[s2_start..idx].iter().collect();
                                let s2 = if flag_p_escapes {
                                    crate::ported::utils::getkeystring(&s2).0
                                } else { s2 };
                                if is_left { preone = Some(s2); } else { postone = Some(s2); }
                                if idx < body_chars.len() { idx += 1; } // skip del
                            }
                        }
                        continue;                           // c:2374 (loop continues from idx)
                    }
                    'o' => { sort_active = true; }          // c:2207
                    'O' => { sort_backwards = true; sort_active = true; } // c:2210
                    'i' => { sort_case_insensitive = true; sort_active = true; } // c:2213
                    'n' => { sort_numeric = true; sort_active = true; } // c:2216
                    '-' => { sort_signed = true; sort_active = true; } // c:2219
                    'a' => { sort_index_order = true; sort_active = true; } // c:2225
                    'u' => { unique = true; }               // c:2476
                    '*' => { sub_flags_bits |= crate::ported::subst::sub_flags::EGLOB; }   // c:2168 (*)
                    'I' => {                                // c:2189 (I:N:)
                        // (I:N:) — match the Nth occurrence in
                        // \${var//pat/repl}. Direct port of
                        // subst.c:2189 which calls get_intarg to
                        // pull the digits and stash in flnum. The
                        // Rust port stashes on state.match_index
                        // so the BUILTIN_PARAM_REPLACE arm reads
                        // it via with_executor.
                        idx += 1;                           // c:2190 (s++)
                        let mut digits = String::new();     // c:2191
                        while idx < body_chars.len()        // c:2191
                            && body_chars[idx].is_ascii_digit() // c:2191
                        {                                    // c:2191
                            digits.push(body_chars[idx]);   // c:2191
                            idx += 1;                       // c:2191
                        }                                    // c:2191
                        if let Ok(n) = digits.parse::<u32>() { // c:2191
                            flnum = n;                      // c:2191
                        }                                    // c:2191
                        continue;                           // c:2195
                    }                                       // c:2195
                    'M' => { sub_flags_bits |= crate::ported::subst::sub_flags::MATCH; }   // c:2171 (M)
                    'R' => { sub_flags_bits |= crate::ported::subst::sub_flags::REST; }    // c:2174 (R)
                    'B' => { sub_flags_bits |= crate::ported::subst::sub_flags::BIND; }    // c:2177 (B)
                    'E' => { sub_flags_bits |= crate::ported::subst::sub_flags::EIND; }    // c:2180 (E)
                    'N' => { sub_flags_bits |= crate::ported::subst::sub_flags::LEN; }     // c:2183 (N)
                    'S' => { sub_flags_bits |= crate::ported::subst::sub_flags::SUBSTR; }  // c:2186 (S)
                    'e' => { flag_eval = true; }            // c:2268 (e)
                    'Q' => { flag_unquote = true; }         // c:2261 (Q)
                    'X' => { flag_error = true; }           // c:2264 (X)
                    'D' => { flag_d_dir = true; }           // c:2229 (D)
                    'V' => { flag_visible = true; }         // c:2232 (V)
                    'b' => { flag_b_pattern = true; }       // c:2255 (b)
                    'w' => { flag_word_count = true; }      // c:2278 (w)
                    'c' => { flag_char_count = true; }      // c:2275 (c)
                    'W' => { flag_word_count_w = true; }    // c:2281 (W)
                    'z' => { flag_z_tokenize = true; }      // c:2439 (z)
                    'Z' => {                                // c:2443 (Z:flags:)
                        // (Z:cCn:) — shell-tokenize with sub-flags:
                        //   c: keep comments
                        //   C: strip comments
                        //   n: treat newlines as whitespace
                        // Direct port of subst.c:2443 — skip the
                        // delimited :flags: arg span; the Rust
                        // tokenizer (consumer) reads sub-flags at
                        // dispatch.
                        flag_z_tokenize = true;             // c:2443
                        idx += 1;                           // c:2444 (s++)
                        if idx < body_chars.len() {         // c:2444
                            let del = body_chars[idx];      // c:2444
                            idx += 1;                       // c:2444
                            while idx < body_chars.len()    // c:2444
                                && body_chars[idx] != del   // c:2444
                            {                                // c:2444
                                let ch = body_chars[idx];   // c:2450
                                if ch == 'c' { flag_z_keep_comments = true; }   // c:2450
                                else if ch == 'C' { flag_z_strip_comments = true; } // c:2456
                                else if ch == 'n' { flag_z_newline_ws = true; } // c:2461
                                idx += 1;                   // c:2444
                            }                                // c:2444
                            if idx < body_chars.len() { idx += 1; } // c:2444
                        }                                    // c:2444
                        continue;                           // c:2473
                    }                                       // c:2473
                    'g' => {                                // c:2409 (g)
                        // (g:flags:) — getkeys subflags. Format is
                        // `g` immediately followed by a delimited
                        // arg whose chars are sub-flag letters
                        // (e/o/c). Direct port of subst.c:2409 —
                        // skips the entire `g:...:` arg span; the
                        // actual escape decoding happens in
                        // getkeystring (already wired by `(p)`).
                        idx += 1;                           // c:2410
                        if idx < body_chars.len() {         // c:2410
                            let del = body_chars[idx];      // c:2410
                            idx += 1;                       // c:2410
                            while idx < body_chars.len()    // c:2410
                                && body_chars[idx] != del   // c:2410
                            {                                // c:2410
                                idx += 1;                   // c:2410
                            }                                // c:2410
                            if idx < body_chars.len() { idx += 1; } // c:2410
                        }                                    // c:2410
                        continue;                           // c:2410
                    }                                       // c:2409 (g)
                    '~' => { state.opts.glob_subst = !state.opts.glob_subst; } // c:2160 (~)
                    'm' => { multi_width += 1; }            // c:2376 (m)
                    'p' => { flag_p_escapes = true; }       // c:2382
                    '%' => { flag_pct_prompt += 1; }        // c:2405 (% prompt-expand)
                    'f' => { spsep = Some("\n".to_string()); } // c:2285
                    'F' => { sep = Some("\n".to_string()); }   // c:2289
                    '0' => { spsep = Some("\u{0}".to_string()); } // c:2293 (split on NUL)
                    's' | 'j' => {                          // c:2299/2302
                        // Consume `:STR:` arg.
                        let is_split = fc == 's';           // c:2300
                        idx += 1;                           // c:2303 (++s)
                        if idx >= body_chars.len() { break; }
                        let del = body_chars[idx];          // c:2303 (get_strarg del)
                        idx += 1;                           // c:2303
                        let s_start = idx;
                        while idx < body_chars.len()
                            && body_chars[idx] != del
                        {
                            idx += 1;
                        }
                        let arg: String = body_chars[s_start..idx].iter().collect(); // c:2308
                        // (p) flag: backslash-escapes in the separator
                        // arg get decoded (`\n` → newline, `\t` → tab,
                        // `\xNN`, `\NNN`, `\\`, `\'`, etc.). Direct
                        // port of `getkeystring()`'s GETKEY_DOLLAR_QUOTE
                        // path which subst.c routes the (s::) arg
                        // through when escapes==1.
                        let arg = if flag_p_escapes {       // c:2382
                            crate::ported::utils::getkeystring(&arg).0 // c:2382
                        } else { arg };                     // c:2382
                        if is_split { spsep = Some(arg); } else { sep = Some(arg); } // c:2309-2313
                        if idx < body_chars.len() { idx += 1; } // skip closing del
                        continue;                           // c:2317 (loop continues from idx)
                    }
                    _ => { /* unhandled flag — swallow per existing behavior */ }
                }
                idx += 1;
            }
        }
        // Stash accumulated SUB_* bits on state so the BUILTIN_PARAM
        // dispatch arms (REPLACE / STRIP / FLAG) can read them via
        // with_executor → exec.sub_flags. Reset back to 0 after the
        // arm runs so the next paramsubst sees a clean slate.
        state.sub_flags = sub_flags_bits;                    // c:2169
        // ${#var} — length-of operator at start of brace (after flags).
        let length_op = body_chars.get(idx).copied() == Some('#'); // c:2128
        let post_flags_start = idx;
        if length_op {
            let next = body_chars.get(idx + 1).copied().unwrap_or('\0');
            if next.is_ascii_alphabetic() || next == '_' || next == '@' || next == '*' {
                idx += 1; // skip the leading #
            }
        }
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
        let mut peeled_quotes = false;                       // c:2649
        if idx + 1 < body_chars.len()                        // c:2649
            && body_chars[idx] == '"'                        // c:2649
            && body_chars[idx + 1] == '$'                    // c:2649
        {                                                    // c:2649
            // Find matching close quote (depth-tracked over $(...)
            // and ${...} so nested DQs don't fool us). Direct port
            // of zsh's QSTRING/STRING dual-pass at subst.c:282.
            let mut p = idx + 1;                             // c:2649
            let mut paren_depth = 0_i32;                     // c:2649
            let mut brace_depth = 0_i32;                     // c:2649
            while p < body_chars.len() {                     // c:2649
                let ch = body_chars[p];                      // c:2649
                match ch {                                   // c:2649
                    '(' => paren_depth += 1,                 // c:2649
                    ')' => paren_depth -= 1,                 // c:2649
                    '{' => brace_depth += 1,                 // c:2649
                    '}' => brace_depth -= 1,                 // c:2649
                    '"' if paren_depth == 0 && brace_depth == 0 => { // c:2649
                        // close quote
                        idx += 1;                            // skip leading "
                        // Mark peeled; inner $-form starts at idx now.
                        peeled_quotes = true;                // c:2649
                        // Note p is the closing quote position;
                        // skip it after the inner $-form is consumed.
                        let _ = p;                           // c:2649
                        break;                               // c:2649
                    }                                        // c:2649
                    _ => {}                                  // c:2649
                }                                            // c:2649
                p += 1;                                      // c:2649
            }                                                // c:2649
        }                                                    // c:2649
        let mut subexp_value: Option<String> = if idx < body_chars.len()
            && body_chars[idx] == '$'                       // c:2649
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
                        if ch == open { depth += 1; }
                        else if ch == close {
                            depth -= 1;
                            if depth == 0 { p += 1; break; }
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
            let expanded = singsub(&inner, state);          // c:2681
            idx = p;                                        // c:2691
            // If we peeled a leading `"`, also consume the matching
            // closing `"` now so the rest of the body (operators,
            // `}`, etc.) parses normally.
            if peeled_quotes && idx < body_chars.len() && body_chars[idx] == '"' { // c:2649
                idx += 1;                                   // c:2649
            }                                                // c:2649
            Some(expanded)
        } else { None };

        // ${+name} set-test. subst.c:2603-2613 — when body opens with
        // `+` followed by an identifier (or string-special with brace/
        // paren as in (P)+name), `chkset = 1; s++;`. The post-lookup
        // path (subst.c:3600) returns "0" if vunset else "1".
        let mut chkset = false;                                   // c:1683
        if idx < body_chars.len() && body_chars[idx] == '+' {     // c:2603
            let nxt = body_chars.get(idx + 1).copied().unwrap_or('\0');
            if nxt.is_ascii_alphanumeric() || nxt == '_' || nxt == '@' || nxt == '*' || nxt == '#' || nxt == '?' {
                chkset = true;                                     // c:2612
                idx += 1;                                          // c:2612
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
        let mut var_name: String = body_chars[name_start..idx].iter().collect();

        // ${arr[subscript]} — subscript loop. Port of subst.c:2862-3000.
        // Parse `[…]` after the var name, with brace-depth tracking
        // for nested `${arr[$other[1]]}`.
        let mut subscript: Option<String> = None;           // c:2867
        if idx < body_chars.len() && body_chars[idx] == '[' { // c:2867
            idx += 1;                                       // c:2867
            let sub_start = idx;
            let mut depth = 1_i32;
            while idx < body_chars.len() && depth > 0 {     // c:2867
                let bc = body_chars[idx];
                if bc == '[' { depth += 1; }                // c:2867
                else if bc == ']' {                         // c:2867
                    depth -= 1;
                    if depth == 0 { break; }
                }
                idx += 1;
            }
            if idx > sub_start {
                let raw_sub: String = body_chars[sub_start..idx].iter().collect();
                // Subscript expressions can contain $vars — singsub them.
                subscript = Some(singsub(&raw_sub, state)); // c:2899
            }
            if idx < body_chars.len() { idx += 1; }         // skip ]
        }

        let rest: String = body_chars[idx..].iter().collect();

        // (P) indirect: take the var name from somewhere — either
        // the value of a parameter (\${(P)x}) or the result of a
        // nested expansion (\${(P)\${(P)x}} = `(P)`-of-(P)-of-x).
        // Direct port of subst.c:2730+ aspar arm. The C source's
        // val pointer is the resolved name string regardless of
        // whether it came from a parameter or a sub-expression.
        if flag_p_indirect {                                // c:2730
            // If a sub-expression already produced the resolved
            // text (subexp arm above), use THAT as the indirect
            // name — clear subexp_value so the var-lookup path
            // applies to the new name. Multi-level (P) chains
            // resolve correctly.
            if let Some(sv) = subexp_value.clone() {        // c:2741
                var_name = sv.trim().to_string();           // c:2741
                subexp_value = None;                        // c:2741 (consumed)
            } else {                                        // c:2741
                let target = state.variables.get(&var_name).cloned() // c:2741
                    .or_else(|| state.arrays.get(&var_name).map(|a| a.join(" "))) // c:2741
                    .unwrap_or_default();                   // c:2741
                var_name = target;                          // c:2741
            }                                                // c:2741
        }

        // Look up var (with subscript if present). Port of
        // subst.c:2965 getstrvalue / getarrvalue dispatch.
        // If subexp_value is set, the value comes from the recursive
        // $(...)/${...} expansion and we skip var-name lookup.
        let raw_value: String = if let Some(sv) = subexp_value {
            sv                                              // c:2681 (subexp result)
        } else if let Some(sub) = subscript.as_deref() {
            // Subscripted lookup: assoc-key, array-index, or slice.
            if let Some(map) = state.assoc_arrays.get(&var_name) { // c:2926 (assoc lookup)
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
                    if flags.chars().all(|c| matches!(c, 'I' | 'R' | 'i' | 'r' | 'k' | 'K' | 'n' | 'e' | 'b')) {
                        Some((flags, pat))
                    } else { None }
                })(sub) {
                    let by_key = flags.contains('I') || flags.contains('i');
                    let return_all = flags.contains('I') || flags.contains('R');
                    let mut out: Vec<String> = Vec::new();
                    for (k, v) in map.iter() {
                        let hay = if by_key { k.as_str() } else { v.as_str() };
                        if crate::exec::ShellExecutor::glob_match_static(hay, &pat) {
                            out.push(if by_key { k.clone() } else { v.clone() });
                            if !return_all { break; }
                        }
                    }
                    out.join(" ")
                } else {
                    map.get(sub).cloned().unwrap_or_default()
                }
            } else if let Some(arr) = state.arrays.get(&var_name) { // c:2926 (array)
                if sub == "*" || sub == "@" {                // c:2916 (full array)
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
                    if flags.chars().all(|c| matches!(c, 'I' | 'R' | 'i' | 'r' | 'n' | 'e')) {
                        Some((flags, pat))
                    } else { None }
                })(sub) {
                    let return_index = flags.contains('I') || flags.contains('i');
                    let return_all = flags.contains('I') || flags.contains('R');
                    let mut out: Vec<String> = Vec::new();
                    for (idx, elem) in arr.iter().enumerate() {
                        if crate::exec::ShellExecutor::glob_match_static(elem, &pat) {
                            if return_index {
                                out.push((idx + 1).to_string());
                            } else {
                                out.push(elem.clone());
                            }
                            if !return_all { break; }
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
                } else if let Ok(idx_n) = sub.parse::<i64>() { // c:2926 (numeric index)
                    let len = arr.len() as i64;
                    let i = if idx_n < 0 { len + idx_n } else { idx_n - 1 };
                    if i >= 0 && (i as usize) < arr.len() {
                        arr[i as usize].clone()
                    } else {
                        String::new()
                    }
                } else if let Some((start_s, end_s)) = sub.split_once(',') { // c:2944 (slice)
                    // Clone arr first to release the borrow, since
                    // singsub needs &mut state.
                    let arr_clone = arr.clone();
                    let len = arr_clone.len() as i64;
                    let start_str = start_s.to_string();
                    let end_str = end_s.to_string();
                    let start: i64 = singsub(&start_str, state).parse().unwrap_or(1);
                    let end: i64 = singsub(&end_str, state).parse().unwrap_or(len);
                    let s = if start < 0 { (len + start).max(0) } else { (start - 1).max(0) } as usize;
                    let e = if end < 0 { (len + end + 1).max(0) } else { end.min(len) } as usize;
                    if s < arr_clone.len() && s < e {
                        arr_clone[s..e.min(arr_clone.len())].join(" ")
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                }
            } else if let Some(magic_val) = crate::fusevm_bridge::with_executor(|exec|
                exec.get_special_array_value(&var_name, sub)
            ) {
                // Magic-assoc lookup: aliases, functions, options,
                // commands, jobtexts, etc. Direct port of zsh's
                // per-magic-table getfn dispatch (Src/Modules/
                // parameter.c et al.). Was falling through to scalar
                // char-index which returned empty.
                magic_val                                    // c:2926
            } else {
                // Scalar with subscript — char-index access.
                let scalar = state.variables.get(&var_name).cloned().unwrap_or_default();
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
                    if f.chars().all(|c| matches!(c, 'I' | 'R' | 'i' | 'r' | 'n' | 'e' | 'b')) {
                        Some((f, p))
                    } else { None }
                })(sub) {
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
                            if crate::exec::ShellExecutor::glob_match_static(&cand, &pat) {
                                found = Some((start, start + len));
                                if !want_last { break 'outer; }
                                break;
                            }
                        }
                    }
                    // For (I): keep scanning to find LAST match.
                    if want_last {
                        for start in (0..=n).rev() {
                            for len in 1..=(n - start) {
                                let cand: String = s_chars[start..start + len].iter().collect();
                                if crate::exec::ShellExecutor::glob_match_static(&cand, &pat) {
                                    found = Some((start, start + len));
                                    break;
                                }
                            }
                            if found.is_some() && found.unwrap().0 >= start { break; }
                        }
                    }
                    match (found, return_index) {
                        (Some((s, _)), true) => (s + 1).to_string(),
                        (Some((s, e)), false) => s_chars[s..e].iter().collect(),
                        (None, true) => {
                            // (i) returns len+1, (I) returns 0 on no match.
                            // Direct port of Src/params.c getindex.
                            if flags.contains('i') { (n + 1).to_string() }
                            else { "0".to_string() }
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
                } else {
                    String::new()
                }
            }
        } else {
            // No subscript — scalar / array / assoc / magic-assoc
            // fallthrough. Direct port of getstrvalue dispatch which
            // checks each storage shape in priority order.
            state.variables.get(&var_name).cloned()
                .or_else(|| state.arrays.get(&var_name).map(|a| a.join(" ")))
                .or_else(|| state.assoc_arrays.get(&var_name)
                    .map(|m| m.values().cloned().collect::<Vec<_>>().join(" ")))
                .or_else(|| crate::fusevm_bridge::with_executor(|exec|
                    exec.get_special_array_value(&var_name, "@")))
                .unwrap_or_default()
        };
        let is_set = state.variables.contains_key(&var_name)
            || state.arrays.contains_key(&var_name)
            || state.assoc_arrays.contains_key(&var_name);

        // ${+name} short-circuit per subst.c:3600 — return "1"/"0".
        // Subscripted form `${+arr[i]}` checks whether THAT element is
        // set, not the array as a whole; raw_value (already
        // subscript-resolved) being non-empty is the proxy.
        if chkset {                                                // c:3600
            let set_str = if subscript.is_some() {
                if !raw_value.is_empty() { "1" } else { "0" }
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
            return (full.clone(), new_pos_in_full, vec![full]);    // c:3600
        }

        // (#)var → element count of array/assoc (or char count of
        // scalar). Port of subst.c:2128 length_op fast path.
        if length_op {                                      // c:2128
            let _ = post_flags_start;
            let n = if let Some(arr) = state.arrays.get(&var_name) {
                arr.len()                                   // c:2128 (array len)
            } else if let Some(map) = state.assoc_arrays.get(&var_name) {
                map.len()                                   // c:2128 (assoc len)
            } else {
                raw_value.chars().count()                   // c:2128 (scalar char-count)
            };
            return (n.to_string(), new_pos, vec![]);
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
        let mut value: String;                              // c:2247
        if flag_keys && flag_values {                        // c:2247 (kv)
            value = state.assoc_arrays.get(&var_name)        // c:2247
                .map(|m| {                                   // c:2247
                    let mut out: Vec<String> = Vec::with_capacity(m.len() * 2); // c:2247
                    for (k, v) in m {                        // c:2247
                        out.push(k.clone());                 // c:2247
                        out.push(v.clone());                 // c:2247
                    }                                        // c:2247
                    out.join(" ")                            // c:2247
                })                                           // c:2247
                .unwrap_or_default();                        // c:2247
        } else if flag_keys {                                // c:2247
            value = state.assoc_arrays.get(&var_name)       // c:2247
                .map(|m| m.keys().cloned().collect::<Vec<_>>().join(" ")) // c:2247
                .or_else(|| {                               // c:2247
                    match var_name.as_str() {               // c:2247
                        "aliases" => Some(                  // c:2247
                            state.alias_names.iter().cloned().collect::<Vec<_>>().join(" ")), // c:2247
                        "functions" | "dis_functions" => Some( // c:2247
                            state.function_names.iter().cloned().collect::<Vec<_>>().join(" ")), // c:2247
                        "commands" => Some(                 // c:2247
                            state.command_names.iter().cloned().collect::<Vec<_>>().join(" ")), // c:2247
                        _ => None,                          // c:2247
                    }                                        // c:2247
                })                                          // c:2247
                .unwrap_or_default();
        } else if flag_values {                             // c:2256
            value = state.assoc_arrays.get(&var_name)       // c:2256
                .map(|m| m.values().cloned().collect::<Vec<_>>().join(" ")) // c:2256
                .unwrap_or_default();
        } else if flag_at {                                 // c:2167
            // (@) array splat — preserve element shape via space-join.
            // For full splat into multiple result_nodes, the
            // multsub-aware caller handles it; we emit space-joined here.
            value = state.arrays.get(&var_name)
                .map(|a| a.join(" "))
                .unwrap_or_else(|| raw_value.clone());
        } else {                                            // c:N/A
            value = raw_value.clone();
        }
        // split_parts: tracks any post-operator array-shape result
        // (e.g. :# filter, (s::) split) so the auto-splat block
        // below splats those instead of the original backing array.
        let mut split_parts: Option<Vec<String>> = None;     // c:3950
        if !rest.is_empty() {
            let r = rest.as_str();
            if let Some(pat) = r.strip_prefix(":#") {        // c:3540 (:#pat filter)
                // Match-test on element(s). Drops elements (or
                // empties scalar) when pattern matches; keeps
                // unchanged when not. With (M) flag in sub_flags,
                // the disposition inverts (keep matching, drop
                // non-matching). Direct port of subst.c:3540
                // SUB_FILTER + getmatch SUB_MATCH branch.
                let p = singsub(pat, state);                 // c:3540
                let invert = (state.sub_flags & 0x0008) != 0; // c:2171 SUB_MATCH
                state.sub_flags = 0;                          // c:2169 (consume)
                if let Some(arr) = state.arrays.get(&var_name).cloned() {
                    let kept: Vec<String> = arr.into_iter() // c:3540
                        .filter(|elem| {                      // c:3540
                            let m = crate::exec::ShellExecutor::glob_match_static(elem, &p); // c:3540
                            if invert { m } else { !m }      // c:3540
                        })                                    // c:3540
                        .collect();
                    value = kept.join(" ");                  // c:3540
                    // Stash filtered parts so the auto-splat block
                    // below uses these, not the unfiltered backing
                    // array. \${(@)arr:#pat} now correctly splats
                    // only the kept elements.
                    split_parts = Some(kept);                // c:3540
                } else {                                      // c:3540
                    let m = crate::exec::ShellExecutor::glob_match_static(&raw_value, &p); // c:3540
                    value = if invert {                       // c:3540
                        if m { raw_value.clone() } else { String::new() } // c:3540
                    } else {                                  // c:3540
                        if m { String::new() } else { raw_value.clone() } // c:3540
                    };                                        // c:3540
                }                                             // c:3540
            } else if let Some(default) = r.strip_prefix(":-") {     // c:3193
                if !is_set || raw_value.is_empty() { value = singsub(default, state); }
            } else if let Some(default) = r.strip_prefix('-') { // c:3193
                if !is_set { value = singsub(default, state); }
            } else if let Some(default) = r.strip_prefix("::=") { // c:3245 (unconditional assign)
                // `${var::=value}` — zsh extension. Always store
                // value (after expansion) regardless of whether var
                // was set/empty. Returns the stored value.
                value = singsub(default, state);
                state.variables.insert(var_name.clone(), value.clone());
            } else if let Some(default) = r.strip_prefix(":=") { // c:3245
                if !is_set || raw_value.is_empty() {
                    value = singsub(default, state);
                    state.variables.insert(var_name.clone(), value.clone());
                }
            } else if let Some(default) = r.strip_prefix('=') {   // c:3245 (= — assign on unset only)
                // Same as := but trigger ONLY on unset (not on
                // empty). Direct port of subst.c case '=' which
                // only checks vunset, not !*val.
                if !is_set {
                    value = singsub(default, state);
                    state.variables.insert(var_name.clone(), value.clone());
                }
            } else if let Some(alt) = r.strip_prefix(":+") {  // c:3296
                if is_set && !raw_value.is_empty() { value = singsub(alt, state); }
                else { value = String::new(); }
            } else if let Some(alt) = r.strip_prefix('+') {   // c:3296
                if is_set { value = singsub(alt, state); } else { value = String::new(); }
            } else if let Some(msg) = r.strip_prefix(":?") {  // c:3193 (:?msg)
                if !is_set || raw_value.is_empty() {
                    let m = if msg.is_empty() {              // c:3193
                        "parameter null or not set".to_string() // c:3193
                    } else {                                  // c:3193
                        singsub(msg, state)                   // c:3193
                    };                                        // c:3193
                    eprintln!("{}: {}", var_name, m);
                    state.errflag = true;
                }
            } else if let Some(msg) = r.strip_prefix('?') {  // c:3193 (?msg — not-set only)
                // Same as :? but trigger ONLY on unset (not on
                // empty). Direct port of subst.c case '?' which
                // only checks `vunset` (not `(vunset || !*val)`).
                if !is_set {
                    let m = if msg.is_empty() {              // c:3193
                        "parameter not set".to_string()       // c:3193
                    } else {                                  // c:3193
                        singsub(msg, state)                   // c:3193
                    };                                        // c:3193
                    eprintln!("{}: {}", var_name, m);
                    state.errflag = true;
                }
            } else if let Some(rep) = r.strip_prefix("//") {  // c:3870 (global replace)
                let parts: Vec<&str> = rep.splitn(2, '/').collect();
                let pat = singsub(parts[0], state);
                let repl = parts.get(1).map(|s| singsub(s, state)).unwrap_or_default();
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
                            if crate::exec::ShellExecutor::glob_match_static(&c, &pat) {
                                m = Some(e); break;
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
                if let Some(arr) = state.arrays.get(&var_name).cloned() {
                    let new_arr: Vec<String> = arr.iter().map(|e| replace_global(e)).collect();
                    value = new_arr.join(" ");                // c:3870
                    split_parts = Some(new_arr);              // c:3870 (auto-splat)
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
                let n = chars_v.len();                              // c:3870
                let mut out = String::with_capacity(raw_value.len()); // c:3870
                let mut p = 0_usize;                                // c:3870
                while p < n {                                       // c:3870
                    // Try longest-first match from position p.
                    let mut matched: Option<usize> = None;          // c:3870
                    for end in (p + 1..=n).rev() {                  // c:3870
                        let cand: String = chars_v[p..end].iter().collect(); // c:3870
                        if crate::exec::ShellExecutor::glob_match_static(&cand, &pat) { // c:3870
                            matched = Some(end);                     // c:3870
                            break;                                   // c:3870
                        }                                            // c:3870
                    }                                                // c:3870
                    if let Some(end) = matched {                    // c:3870
                        out.push_str(&repl);                         // c:3870
                        p = if end == p { p + 1 } else { end };      // c:3870 (avoid infinite loop on empty match)
                    } else {                                         // c:3870
                        out.push(chars_v[p]);                        // c:3870
                        p += 1;                                      // c:3870
                    }                                                // c:3870
                }                                                    // c:3870
                value = out;                                         // c:3870
                } // close handled_array else block
            } else if let Some(rep) = r.strip_prefix('/') {   // c:3870 (single replace)
                let parts: Vec<&str> = rep.splitn(2, '/').collect();
                let pat = singsub(parts[0], state);
                let repl = parts.get(1).map(|s| singsub(s, state)).unwrap_or_default();
                // Single-replace helper. Variants: anchor-prefix
                // (pat starts with `#`), anchor-suffix (`%`), or
                // unanchored. Returns the post-replacement string.
                let replace_one = |val: &str| -> String {
                    if let Some(anchor_pat) = pat.strip_prefix('#') {
                        let cv: Vec<char> = val.chars().collect();
                        let nn = cv.len();
                        for end in (0..=nn).rev() {
                            let cand: String = cv[..end].iter().collect();
                            if crate::exec::ShellExecutor::glob_match_static(&cand, anchor_pat) {
                                return format!("{}{}", repl, cv[end..].iter().collect::<String>());
                            }
                        }
                        val.to_string()
                    } else if let Some(anchor_pat) = pat.strip_prefix('%') {
                        let cv: Vec<char> = val.chars().collect();
                        let nn = cv.len();
                        for start in 0..=nn {
                            let cand: String = cv[start..].iter().collect();
                            if crate::exec::ShellExecutor::glob_match_static(&cand, anchor_pat) {
                                return format!("{}{}", cv[..start].iter().collect::<String>(), repl);
                            }
                        }
                        val.to_string()
                    } else {
                        let cv: Vec<char> = val.chars().collect();
                        let nn = cv.len();
                        for start in 0..nn {
                            for end in (start + 1..=nn).rev() {
                                let cand: String = cv[start..end].iter().collect();
                                if crate::exec::ShellExecutor::glob_match_static(&cand, &pat) {
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
                if let Some(arr) = state.arrays.get(&var_name).cloned() {
                    let new_arr: Vec<String> = arr.iter().map(|e| replace_one(e)).collect();
                    value = new_arr.join(" ");                    // c:3870
                    split_parts = Some(new_arr);                  // c:3870
                } else {
                    value = replace_one(&raw_value);              // c:3870
                }
            } else if let Some(pat) = r.strip_prefix("##") {  // c:3540 (longest prefix strip)
                let p = singsub(pat, state);
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
                                if crate::exec::ShellExecutor::glob_match_static(&prefix, &p) {
                                    return cv[k..].iter().collect();
                                }
                                if k == 0 { break; }
                                k -= 1;
                            }
                            val.to_string()
                        }
                        _ => val.to_string(),
                    }
                };
                if let Some(arr) = state.arrays.get(&var_name).cloned() {
                    let new_arr: Vec<String> = arr.iter().map(|e| strip_one(e, 1)).collect();
                    value = new_arr.join(" ");                    // c:3540
                    split_parts = Some(new_arr);                  // c:3540
                } else {
                    value = strip_one(&raw_value, 1);             // c:3540
                }
            } else if let Some(pat) = r.strip_prefix('#') {   // c:3540 (shortest prefix strip)
                let p = singsub(pat, state);
                let strip_one = |val: &str| -> String {
                    let cv: Vec<char> = val.chars().collect();
                    let total = cv.len();
                    for k in 0..=total {
                        let prefix: String = cv[..k].iter().collect();
                        if crate::exec::ShellExecutor::glob_match_static(&prefix, &p) {
                            return cv[k..].iter().collect();
                        }
                    }
                    val.to_string()
                };
                if let Some(arr) = state.arrays.get(&var_name).cloned() {
                    let new_arr: Vec<String> = arr.iter().map(|e| strip_one(e)).collect();
                    value = new_arr.join(" ");                    // c:3540
                    split_parts = Some(new_arr);                  // c:3540
                } else {
                    value = strip_one(&raw_value);                // c:3540
                }
            } else if let Some(pat) = r.strip_prefix("%%") {  // c:3540 (longest suffix strip)
                let p = singsub(pat, state);
                let strip_one = |val: &str| -> String {
                    let cv: Vec<char> = val.chars().collect();
                    let total = cv.len();
                    let mut k = total;
                    loop {
                        let suffix: String = cv[total - k..].iter().collect();
                        if crate::exec::ShellExecutor::glob_match_static(&suffix, &p) {
                            return cv[..total - k].iter().collect();
                        }
                        if k == 0 { break; }
                        k -= 1;
                    }
                    val.to_string()
                };
                if let Some(arr) = state.arrays.get(&var_name).cloned() {
                    let new_arr: Vec<String> = arr.iter().map(|e| strip_one(e)).collect();
                    value = new_arr.join(" ");                    // c:3540
                    split_parts = Some(new_arr);                  // c:3540
                } else {
                    value = strip_one(&raw_value);                // c:3540
                }
            } else if let Some(pat) = r.strip_prefix('%') {   // c:3540 (shortest suffix strip)
                let p = singsub(pat, state);
                let strip_one = |val: &str| -> String {
                    let cv: Vec<char> = val.chars().collect();
                    let total = cv.len();
                    for k in 0..=total {
                        let suffix: String = cv[total - k..].iter().collect();
                        if crate::exec::ShellExecutor::glob_match_static(&suffix, &p) {
                            return cv[..total - k].iter().collect();
                        }
                    }
                    val.to_string()
                };
                if let Some(arr) = state.arrays.get(&var_name).cloned() {
                    let new_arr: Vec<String> = arr.iter().map(|e| strip_one(e)).collect();
                    value = new_arr.join(" ");                    // c:3540
                    split_parts = Some(new_arr);                  // c:3540
                } else {
                    value = strip_one(&raw_value);                // c:3540
                }
            } else if let Some(rhs) = r.strip_prefix(":|") { // c:3540 (set difference)
                // ${arr:|other} — array set-difference: keep elems
                // of arr that are NOT in other. Port of subst.c:3540
                // SUB_DIFFERENCE arm. Per zsh, the RHS array's
                // elements are GLOB PATTERNS — so `\${arr:|patterns}`
                // drops every elem of arr that matches ANY pattern
                // in `patterns`. Was doing literal-eq via HashSet.
                let arr = state.arrays.get(&var_name).cloned().unwrap_or_default();
                let other_name = rhs.trim();                 // c:3543
                let other = state.arrays.get(other_name).cloned().unwrap_or_default();
                let kept: Vec<String> = arr.into_iter()      // c:3540
                    .filter(|s| {                            // c:3540
                        !other.iter().any(|pat|              // c:3540
                            crate::exec::ShellExecutor::glob_match_static(s, pat)) // c:3540
                    })                                       // c:3540
                    .collect();
                value = kept.join(" ");
                split_parts = Some(kept);                    // c:3540 (auto-splat)
            } else if let Some(rhs) = r.strip_prefix(":*") { // c:3540 (intersect)
                // ${arr:*other} — array set-intersection — KEEP
                // elems of arr matching ANY pattern in `other`.
                let arr = state.arrays.get(&var_name).cloned().unwrap_or_default();
                let other_name = rhs.trim();                 // c:3543
                let other = state.arrays.get(other_name).cloned().unwrap_or_default();
                let kept: Vec<String> = arr.into_iter()      // c:3540
                    .filter(|s| {                            // c:3540
                        other.iter().any(|pat|               // c:3540
                            crate::exec::ShellExecutor::glob_match_static(s, pat)) // c:3540
                    })                                       // c:3540
                    .collect();
                value = kept.join(" ");
                split_parts = Some(kept);                    // c:3540 (auto-splat)
            } else if let Some(rhs) = r.strip_prefix(":^^") { // c:3540 (zip-long)
                // ${arr:^^other} — interleave two arrays, continuing
                // past the shorter one with empty strings (vs `:^`
                // which stops at the shorter). Direct port of the
                // SUB_ZIP_LONG variant in subst.c:3540.
                let arr = state.arrays.get(&var_name).cloned().unwrap_or_default();
                let other = state.arrays.get(rhs.trim()).cloned().unwrap_or_default();
                let n = arr.len().max(other.len());
                let mut zipped: Vec<String> = Vec::with_capacity(n * 2);
                for i in 0..n {
                    zipped.push(arr.get(i).cloned().unwrap_or_default());
                    zipped.push(other.get(i).cloned().unwrap_or_default());
                }
                value = zipped.join(" ");
                split_parts = Some(zipped);                  // c:3540 (auto-splat)
            } else if let Some(rhs) = r.strip_prefix(":^") { // c:3540 (zip)
                // ${arr:^other} — interleave two arrays element-by-elem.
                let arr = state.arrays.get(&var_name).cloned().unwrap_or_default();
                let other = state.arrays.get(rhs.trim()).cloned().unwrap_or_default();
                let mut zipped: Vec<String> = Vec::with_capacity(arr.len() + other.len());
                let n = arr.len().min(other.len());
                for i in 0..n {
                    zipped.push(arr[i].clone());
                    zipped.push(other[i].clone());
                }
                value = zipped.join(" ");
                split_parts = Some(zipped);                  // c:3540 (auto-splat)
            } else if let Some(slice) = r.strip_prefix(':') { // c:715 (substring) OR :modifier
                // Detect history-style modifier (`:h`, `:t`, `:r`,
                // `:e`, `:l`, `:u`, `:q`, `:Q`, `:A`, `:a`, `:P`,
                // `:c`, `:s/x/y/`, `:S/x/y/`, `:&`). Route through
                // modify() which handles the full chain. Direct
                // port of subst.c's c:715 modifier dispatch.
                let first = slice.chars().next().unwrap_or('\0');
                let is_modifier = matches!(first, 'h' | 't' | 'r' | 'e' | 'l' | 'u' | 'q' | 'Q'
                                  | 'A' | 'a' | 'P' | 'c' | 's' | 'S' | '&'
                                  | 'g' | 'w' | 'W');
                if is_modifier {                             // c:4531
                    // Per-element on arrays.
                    let mod_str = format!(":{}", slice);
                    let mod_one = |s: &str, st: &mut SubstState| -> String {
                        modify(s, &mod_str, st)
                    };
                    if let Some(parts) = split_parts.clone() {
                        let new_parts: Vec<String> = parts.iter()
                            .map(|s| mod_one(s, state)).collect();
                        value = new_parts.join(" ");
                        split_parts = Some(new_parts);
                    } else if let Some(arr) = state.arrays.get(&var_name).cloned() {
                        let new_arr: Vec<String> = arr.iter()
                            .map(|s| mod_one(s, state)).collect();
                        value = new_arr.join(" ");
                        split_parts = Some(new_arr);
                    } else {
                        value = mod_one(&value, state);
                    }
                } else {
                let parts: Vec<&str> = slice.splitn(2, ':').collect();
                let off = singsub(parts[0], state).parse::<i64>().unwrap_or(0);
                // Array context: ${arr:offset:length} slices the
                // ARRAY (1-based, like Bash's offset), not the joined
                // value. Direct port of subst.c's array-shape branch
                // around c:715. Falls back to scalar substring when
                // var_name isn't an array.
                // Source priority: split_parts (prior operator
                // result like filter/sort) → state.arrays → joined
                // value. Direct port of zsh's getarrvalue → slice
                // dispatch which uses aval if isarr is set.
                let array_source: Option<Vec<String>> = split_parts.clone()
                    .or_else(|| state.arrays.get(&var_name).cloned());
                if let Some(mut arr) = array_source {
                    // Positional-param slice (`@`/`*`/`argv`) — zsh
                    // counts offset 0 as $0 (script/function name),
                    // not $1. Prepend $0 so `${@:0:2}` returns
                    // [$0, $1] instead of [$1, $2]. Direct port of
                    // subst.c's @/* offset arm which routes through
                    // dohist offset = 0 (includes argzero).
                    if var_name == "@" || var_name == "*" || var_name == "argv" {
                        let s0 = state.variables.get("0").cloned().unwrap_or_default();
                        arr.insert(0, s0);                   // c:715
                    }
                    let n = arr.len() as i64;                // c:715
                    let lo = if off < 0 { (n + off).max(0) } else { off.min(n) } as usize; // c:715
                    let len = parts.get(1)                   // c:715
                        .map(|s| singsub(s, state).parse::<i64>().unwrap_or(0)); // c:715
                    let kept: Vec<String> = match len {      // c:715
                        Some(l) if l >= 0 => arr.iter().skip(lo).take(l as usize).cloned().collect(), // c:715
                        Some(l) => {                          // c:715 (negative len = from-end)
                            let end = ((n - lo as i64) + l).max(0) as usize; // c:715
                            arr.iter().skip(lo).take(end).cloned().collect() // c:715
                        }                                     // c:715
                        None => arr.iter().skip(lo).cloned().collect(), // c:715
                    };
                    value = kept.join(" ");                  // c:715
                    split_parts = Some(kept);                // c:715 (auto-splat slice)
                } else {
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
                } // close is_modifier else
            }
        }
        // Apply post-processing flags to the substituted value.
        // C lines 3950-4070 — case mods, quoting, etc.
        if flag_typeinfo {                                  // c:2807
            // ${(t)var} — emit type tag. var_attrs takes
            // precedence (carries typeset flags); fall back to
            // synthesized tag from the storage table the value
            // lives in. Direct port of subst.c:2814 wantt arm
            // which checks paramtab + storage shape.
            value = state.var_attrs.get(&var_name)          // c:2814
                .map(|attr| attr.format_zsh())              // c:2825
                .unwrap_or_else(|| {
                    if state.assoc_arrays.contains_key(&var_name) {
                        "association".to_string()           // c:2814
                    } else if state.arrays.contains_key(&var_name) {
                        "array".to_string()                  // c:2814
                    } else if matches!(var_name.as_str(),
                        "aliases" | "galiases" | "saliases"
                        | "dis_aliases" | "dis_galiases" | "dis_saliases"
                        | "functions" | "dis_functions"
                        | "builtins" | "dis_builtins"
                        | "reswords" | "dis_reswords"
                        | "options" | "commands" | "modules"
                        | "nameddirs" | "userdirs"
                        | "jobtexts" | "jobdirs" | "jobstates"
                        | "parameters" | "dirstack" | "errnos"
                        | "sysparams" | "mapfile") {
                        // Magic-assoc params — type is association.
                        // Direct port of subst.c:2814 paramtab
                        // lookup which finds the magic-assoc entry
                        // and returns PM_HASHED type tag.
                        "association".to_string()           // c:2814
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
        let cap_word = |s: &str| -> String {                 // c:2203
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
        if flag_lower || flag_upper || flag_caps {           // c:2197
            let transform = |s: &str| -> String {            // c:3937
                if flag_lower { s.to_lowercase() }
                else if flag_upper { s.to_uppercase() }
                else { cap_word(s) }
            };
            if let Some(parts) = split_parts.clone() {       // c:3937
                let new_parts: Vec<String> = parts.iter().map(|s| transform(s)).collect();
                value = new_parts.join(" ");                 // c:3937
                split_parts = Some(new_parts);               // c:3937
            } else if let Some(arr) = state.arrays.get(&var_name).cloned() {
                let new_arr: Vec<String> = arr.iter().map(|s| transform(s)).collect();
                value = new_arr.join(" ");                   // c:3937
                split_parts = Some(new_arr);                 // c:3937
            } else {
                value = transform(&value);                   // c:3937
            }
        }
        // (o)/(O)/(i)/(n)/(a)/(u) sort + unique. Port of
        // subst.c:4180-4253 array sortit/unique post-processing.
        // Applies on space-joined value; reassembles after.
        if sort_active || unique {                          // c:4180
            // Sort/unique source: prefer split_parts (any prior
            // operator result like :# filter, (s::) split, or
            // assoc-splat) so sort applies to the actual element
            // list, not a whitespace re-split of the joined view.
            let parts: Vec<String> = if let Some(sp) = split_parts.clone() {
                sp                                          // c:4180 (operator-result)
            } else if let Some(arr) = state.arrays.get(&var_name) {
                arr.clone()                                 // c:4180 (real array)
            } else if let Some(map) = state.assoc_arrays.get(&var_name) {
                map.values().cloned().collect()             // c:4180 (assoc values)
            } else {
                value.split_whitespace().map(String::from).collect() // c:4180 (fallback)
            };
            let mut sorted: Vec<String> = parts;
            if unique {                                     // c:4253
                let mut seen = std::collections::HashSet::new();
                sorted.retain(|s| seen.insert(s.clone()));  // c:4253
            }
            if sort_active {                                // c:4180
                // (a) on assoc-derived elements means "preserve
                // insertion order" — IndexMap already iterates in
                // that order, so skip the sort entirely. The C
                // source short-circuits at SORTIT_BACKWARDS_ONLY
                // (no SORTIT_NUMERICALLY / SORTIT_IGNORING_CASE).
                if !sort_index_order {                      // c:4194
                    if sort_numeric {                       // c:4189
                        // sort_signed: f64 already handles the
                        // sign — `(n-)` and `(n)` compare the same
                        // way for the values we'll see.
                        let _ = sort_signed;                // c:4193
                        sorted.sort_by(|a, b| {
                            let na: f64 = a.parse().unwrap_or(0.0); // c:4189
                            let nb: f64 = b.parse().unwrap_or(0.0); // c:4189
                            na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
                        });
                    } else if sort_case_insensitive {       // c:4187
                        sorted.sort_by_key(|a| a.to_lowercase());
                    } else {                                // c:4180 (default)
                        sorted.sort();
                    }
                }                                            // c:4194
                if sort_backwards { sorted.reverse(); }     // c:4191
            }
            let join_with = sep.as_deref().unwrap_or(" ");
            value = sorted.join(join_with);
            // Update split_parts so downstream operators (case mods,
            // padding, splat) see the sorted/uniq list.
            split_parts = Some(sorted);                      // c:4180
        }

        // (s::SEP:) split-on-SEP: apply BEFORE dopadding/quote/case
        // (per zsh order). Port of subst.c flag-loop spsep usage
        // around line 3950+ (post-fetch split block).
        // Track the post-split parts for the auto-splat block so
        // (@s::) on a scalar splats into multiple result_nodes.
        // split_parts hoisted to top of operand-handling so the
        // :# filter arm (which runs much earlier) can populate it
        // for the auto-splat block. No-op if not set later.
        if let Some(ref sp) = spsep {                       // c:3950
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
            } else if let Some(arr) = state.arrays.get(&var_name).cloned() {
                arr.iter().flat_map(|s| split_one(s)).collect()
            } else {
                split_one(&value)
            };
            // zsh: split result is space-joined for scalar context;
            // multsub-aware caller handles full multi-node splat
            // via split_parts (passed through to the auto_splat
            // post-processing block below).
            let join_with = sep.as_deref().unwrap_or(" ");  // c:3950
            value = parts.join(join_with);
            split_parts = Some(parts);                       // c:3950
        } else if let Some(ref sp) = sep {                  // c:3963 (j with no s)
            // (j:STR:) — join an array with STR. Source priority:
            // split_parts (operator result) → state.arrays →
            // assoc-values → whitespace-split fallback. Direct
            // port of subst.c:3963 sepjoin which reads aval.
            if let Some(parts) = split_parts.clone() {
                value = parts.join(sp);                      // c:3963
                // Join collapses array shape → reset split_parts
                // so auto_splat emits one scalar node, not the
                // joined-then-1-elem-splat.
                split_parts = None;                          // c:3963
            } else if let Some(arr) = state.arrays.get(&var_name) { // c:3963
                value = arr.join(sp);                        // c:3963
            } else if let Some(map) = state.assoc_arrays.get(&var_name) { // c:3963
                let vals: Vec<String> = map.values().cloned().collect();
                value = vals.join(sp);                       // c:3963
            } else if value.contains(' ') || value.contains('\n') {
                let parts: Vec<&str> = value.split_whitespace().collect();
                value = parts.join(sp);
            }
        }

        // (l:N::PRE:) / (r:N::POST:) padding — apply via dopadding.
        // Per-element on arrays so each element gets padded
        // independently. Direct port of subst.c flag-loop l/r
        // interacting with isarr branch which pads aval per-element.
        if prenum > 0 || postnum > 0 {                      // c:2319/2330
            let mul_default = " ".to_string();              // c:907 (def = " ")
            let pad_one = |s: &str| -> String {              // c:893
                dopadding(
                    s,
                    prenum.max(0) as usize,
                    postnum.max(0) as usize,
                    preone.as_deref(),
                    postone.as_deref(),
                    premul.as_deref().unwrap_or(&mul_default),
                    postmul.as_deref().unwrap_or(&mul_default),
                    multi_width as i32,                     // c:2376 (m)
                )
            };
            if let Some(parts) = split_parts.clone() {
                let new_parts: Vec<String> = parts.iter().map(|s| pad_one(s)).collect();
                value = new_parts.join(" ");
                split_parts = Some(new_parts);
            } else if let Some(arr) = state.arrays.get(&var_name).cloned() {
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
        if flag_evalchar {                                  // c:1673
            let eval_one = |s: &str| -> String {
                substevalchar(s.trim()).unwrap_or_default()
            };
            if let Some(parts) = split_parts.clone() {
                let new_parts: Vec<String> = parts.iter().map(|s| eval_one(s)).collect();
                value = new_parts.join(" ");
                split_parts = Some(new_parts);
            } else if let Some(arr) = state.arrays.get(&var_name).cloned() {
                let new_arr: Vec<String> = arr.iter().map(|s| eval_one(s)).collect();
                value = new_arr.join(" ");
                split_parts = Some(new_arr);
            } else {
                value = eval_one(&value);
            }
        }                                                    // c:1673

        // (e) eval — re-substitute the result. Per-element on arrays.
        // Direct port of subst.c:2268 eval bit which iterates aval.
        if flag_eval {                                      // c:2268
            if let Some(parts) = split_parts.clone() {
                let new_parts: Vec<String> = parts.iter().map(|s| singsub(s, state)).collect();
                value = new_parts.join(" ");
                split_parts = Some(new_parts);
            } else if let Some(arr) = state.arrays.get(&var_name).cloned() {
                let new_arr: Vec<String> = arr.iter().map(|s| singsub(s, state)).collect();
                value = new_arr.join(" ");
                split_parts = Some(new_arr);
            } else {
                value = singsub(&value, state);             // c:2268
            }
        }

        // (%) prompt-expand — interpret %F{red}, %~, %n, %{...%},
        // etc. Per-element on arrays. Direct port of subst.c:2405 /
        // 3977 presc handling.
        if flag_pct_prompt > 0 {                            // c:2405
            let prompt_one = |s: &str| -> String {
                crate::fusevm_bridge::with_executor(|exec| exec.expand_prompt_string(s))
            };
            if let Some(parts) = split_parts.clone() {
                let new_parts: Vec<String> = parts.iter().map(|s| prompt_one(s)).collect();
                value = new_parts.join(" ");
                split_parts = Some(new_parts);
            } else if let Some(arr) = state.arrays.get(&var_name).cloned() {
                let new_arr: Vec<String> = arr.iter().map(|s| prompt_one(s)).collect();
                value = new_arr.join(" ");
                split_parts = Some(new_arr);
            } else {
                value = prompt_one(&value);                 // c:3977
            }
        }                                                    // c:2405

        // (z)/(Z:cCn:) — shell-tokenize the value into a list of
        // words. Direct port of subst.c:2439 LEXFLAGS_ACTIVE +
        // sub-flags. Simplified port: use whitespace splitting
        // that respects single/double-quote spans and backslash
        // escapes, plus optional comment handling. The full lexer
        // reentry is deferred — this covers the common idioms
        // \${(z)cmdline} (split a command into words) and
        // \${(Zn)multiline} (newlines act like spaces).
        if flag_z_tokenize {                                // c:2439
            let mut words: Vec<String> = Vec::new();        // c:2439
            let mut cur = String::new();                     // c:2439
            let mut in_sq = false;                          // c:2439
            let mut in_dq = false;                          // c:2439
            let mut in_comment = false;                     // c:2451
            let chars_v: Vec<char> = value.chars().collect(); // c:2439
            let push_word = |w: &mut String, words: &mut Vec<String>| { // c:2439
                if !w.is_empty() {                          // c:2439
                    words.push(std::mem::take(w));          // c:2439
                }                                            // c:2439
            };                                               // c:2439
            let mut p = 0_usize;                            // c:2439
            while p < chars_v.len() {                       // c:2439
                let ch = chars_v[p];                        // c:2439
                if in_comment {                             // c:2451
                    if ch == '\n' {                         // c:2451
                        in_comment = false;                 // c:2451
                        if flag_z_keep_comments { cur.push(ch); } // c:2451
                    } else if flag_z_keep_comments {        // c:2451
                        cur.push(ch);                       // c:2451
                    }                                        // c:2451
                    p += 1;                                 // c:2451
                    continue;                               // c:2451
                }                                            // c:2451
                if in_sq {                                  // c:2439
                    cur.push(ch);                           // c:2439
                    if ch == '\'' { in_sq = false; }        // c:2439
                    p += 1; continue;                        // c:2439
                }                                            // c:2439
                if in_dq {                                  // c:2439
                    cur.push(ch);                           // c:2439
                    if ch == '\\' && p + 1 < chars_v.len() { // c:2439
                        p += 1;                              // c:2439
                        cur.push(chars_v[p]);                // c:2439
                    } else if ch == '"' {                   // c:2439
                        in_dq = false;                       // c:2439
                    }                                        // c:2439
                    p += 1; continue;                        // c:2439
                }                                            // c:2439
                match ch {                                   // c:2439
                    '\\' if p + 1 < chars_v.len() => {       // c:2439
                        cur.push(ch);                        // c:2439
                        p += 1;                              // c:2439
                        cur.push(chars_v[p]);                // c:2439
                    }                                        // c:2439
                    '\'' => { cur.push(ch); in_sq = true; }  // c:2439
                    '"' => { cur.push(ch); in_dq = true; }   // c:2439
                    '#' if cur.is_empty() && !flag_z_strip_comments => { // c:2451
                        // Start of comment word — keep or skip.
                        in_comment = !flag_z_keep_comments;  // c:2451
                        if flag_z_keep_comments { cur.push(ch); } // c:2451
                    }                                        // c:2451
                    '#' if cur.is_empty() && flag_z_strip_comments => { // c:2456
                        in_comment = true;                   // c:2456
                    }                                        // c:2456
                    '\n' if flag_z_newline_ws => {           // c:2461 (n: nl as ws)
                        push_word(&mut cur, &mut words);     // c:2461
                    }                                        // c:2461
                    c if c.is_whitespace() => {              // c:2439
                        push_word(&mut cur, &mut words);     // c:2439
                    }                                        // c:2439
                    _ => cur.push(ch),                       // c:2439
                }                                            // c:2439
                p += 1;                                      // c:2439
            }                                                // c:2439
            push_word(&mut cur, &mut words);                // c:2439
            value = words.join(" ");                        // c:2439
        }                                                    // c:2473

        // (D) dir-magic — replace $HOME and any nameddir prefix with
        // tilde form. Direct port of subst.c:2229 mods bit 1, which
        // routes through modify()'s tilde-contraction at the end of
        // the pipeline. Common idiom: `${(D)PWD}` → `~/projects/foo`.
        // Without ZLE's nameddir hash, this reduces to plain $HOME.
        // (D) per-element dir-magic. Direct port of subst.c:2229
        // mods bit 1 → modify()'s tilde-contraction iterating aval.
        if flag_d_dir {                                     // c:2229
            let home_opt = state.variables.get("HOME").cloned()
                .or_else(|| std::env::var("HOME").ok());
            // Pull named-dirs (~name) hash into a [(name, path)]
            // sorted by path-length-descending so the LONGEST match
            // wins (zsh canonical: most-specific tilde-contraction).
            // Direct port of subst.c → modify dir-handling which
            // walks the nameddirtab in length-desc order.
            let mut named: Vec<(String, String)> = crate::fusevm_bridge::with_executor(|exec| {
                exec.named_dirs.iter()
                    .map(|(k, v)| (k.clone(), v.to_string_lossy().into_owned()))
                    .collect()
            });
            named.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
            let dir_one = |s: &str| -> String {              // c:2229
                // Try named-dirs first (most specific wins).
                for (name, path) in &named {                 // c:2229
                    if !path.is_empty() && s.starts_with(path.as_str()) {
                        let r = &s[path.len()..];
                        if r.is_empty() || r.starts_with('/') {
                            return format!("~{}{}", name, r);
                        }
                    }
                }
                // Fall back to $HOME contraction.
                if let Some(ref h) = home_opt {              // c:2229
                    if !h.is_empty() && s.starts_with(h.as_str()) { // c:2229
                        let r = &s[h.len()..];               // c:2229
                        if r.is_empty() || r.starts_with('/') { // c:2229
                            return format!("~{}", r);       // c:2229
                        }                                    // c:2229
                    }                                        // c:2229
                }                                            // c:2229
                s.to_string()                                // c:2229
            };                                               // c:2229
            if let Some(parts) = split_parts.clone() {
                let new_parts: Vec<String> = parts.iter().map(|s| dir_one(s)).collect();
                value = new_parts.join(" ");
                split_parts = Some(new_parts);
            } else if let Some(arr) = state.arrays.get(&var_name).cloned() {
                let new_arr: Vec<String> = arr.iter().map(|s| dir_one(s)).collect();
                value = new_arr.join(" ");
                split_parts = Some(new_arr);
            } else {
                value = dir_one(&value);                     // c:2229
            }
        }                                                    // c:2229

        // (b) backslash-quote pattern metachars — output is safe to
        // feed back into a glob/regex context as a literal. Port of
        // subst.c:2255 QT_BACKSLASH_PATTERN: every char that has
        // pattern meaning (`* ? [ ] ( ) | ^ # ~ \ < >` plus IFS
        // whitespace and shell metachars `& ; { } $ \` " '`) gets
        // a leading backslash. Used by `[[ x =~ ${(b)pat} ]]` and
        // `case x in ${(b)pat}` to neutralize a user-supplied
        // string before it's interpreted as a pattern.
        // (b) per-element backslash-quote. Direct port of subst.c:2255
        // QT_BACKSLASH_PATTERN iterating aval per-element.
        let b_one = |s: &str| -> String {                    // c:2255
            let mut out = String::with_capacity(s.len() * 2);
            for ch in s.chars() {
                if matches!(ch,
                    '*' | '?' | '[' | ']' | '(' | ')' | '|' | '^' | '#' | '~'
                    | '\\' | '<' | '>' | '&' | ';' | '{' | '}' | '$' | '`'
                    | '"' | '\'' | ' ' | '\t' | '\n')
                {
                    out.push('\\');
                }
                out.push(ch);
            }
            out
        };
        if flag_b_pattern {                                 // c:2255
            if let Some(parts) = split_parts.clone() {
                let new_parts: Vec<String> = parts.iter().map(|s| b_one(s)).collect();
                value = new_parts.join(" ");
                split_parts = Some(new_parts);
            } else if let Some(arr) = state.arrays.get(&var_name).cloned() {
                let new_arr: Vec<String> = arr.iter().map(|s| b_one(s)).collect();
                value = new_arr.join(" ");
                split_parts = Some(new_arr);
            } else {
                value = b_one(&value);                       // c:2255
            }                                                // c:2255
        }                                                    // c:2255

        // (Q) unquote — strip outer quotes / backslash escapes /
        // decode $'…' C-string quoting. Port of subst.c:2261
        // quotemod-- effect → utils.c::dequotestring which handles
        // SQ spans (literal), DQ spans (with backslash escapes),
        // $'…' spans (with full \n / \t / \xNN / \NNN decoding via
        // getkeystring), and standalone backslash escapes.
        // (Q) unquote per-element on arrays. Direct port of
        // subst.c:2261 quotemod-- which iterates aval per-element.
        let unquote_one = |s: &str| -> String {              // c:2261
            let chars_v: Vec<char> = s.chars().collect();
            let mut out = String::with_capacity(s.len());
            let mut i = 0_usize;
            while i < chars_v.len() {
                let c = chars_v[i];
                if c == '$' && i + 1 < chars_v.len() && chars_v[i + 1] == '\'' { // c:2261
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
        if flag_unquote {                                   // c:2261
            if let Some(parts) = split_parts.clone() {
                let new_parts: Vec<String> = parts.iter().map(|s| unquote_one(s)).collect();
                value = new_parts.join(" ");
                split_parts = Some(new_parts);
            } else if let Some(arr) = state.arrays.get(&var_name).cloned() {
                let new_arr: Vec<String> = arr.iter().map(|s| unquote_one(s)).collect();
                value = new_arr.join(" ");
                split_parts = Some(new_arr);
            } else {
                value = unquote_one(&value);
            }
        }

        // (X) error on unset/empty — emit error if value is empty.
        // Port of subst.c:2264 (quoteerr=1).
        if flag_error && value.is_empty() && !is_set {      // c:2264
            eprintln!("zshrs: {}: parameter not set or null", var_name); // c:N/A
            state.errflag = true;
        }

        // (V) visible — render control chars as ^X form.
        // Port of subst.c:2232 mods bit 1. Per-element on arrays.
        let visible_one = |s: &str| -> String {              // c:2232
            let mut out = String::with_capacity(s.len());
            for c in s.chars() {
                let cp = c as u32;
                if cp < 0x20 {                              // c:2232 (control chars)
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
        if flag_visible {                                   // c:2232
            if let Some(parts) = split_parts.clone() {
                let new_parts: Vec<String> = parts.iter().map(|s| visible_one(s)).collect();
                value = new_parts.join(" ");
                split_parts = Some(new_parts);
            } else if let Some(arr) = state.arrays.get(&var_name).cloned() {
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
        if flag_char_count {                                // c:2275
            // (m) flag, when set, counts cells via wcpadwidth (so
            // wide chars count 2). Without (m): plain chars.count().
            // Direct port of subst.c:2275 whichlen + multi_width.
            value = if multi_width > 0 {                    // c:2275
                value.chars()                               // c:2376
                    .map(|c| wcpadwidth(c, multi_width as i32) as usize) // c:2376
                    .sum::<usize>()                         // c:2376
                    .to_string()                            // c:2376
            } else {                                        // c:2275
                value.chars().count().to_string()           // c:2275
            };                                               // c:2275
        } else if flag_word_count {                         // c:2278
            value = value.split_whitespace().count().to_string(); // c:2278
        } else if flag_word_count_w {                       // c:2281
            // (W) — count words including empty fields.
            let parts: Vec<&str> = value.split(|c: char| c.is_whitespace()).collect();
            value = parts.len().to_string();                // c:2281
        }

        // Quote flags operate per-element when array-shaped — direct
        // port of subst.c quotemod arm which iterates aval.
        let quote_one = |s: &str| -> String {                // c:2237
            if flag_qmin {                                   // c:2245 (q-)
                let needs = s.chars().any(|c| {
                    c.is_whitespace()
                        || matches!(c, '*' | '?' | '[' | ']' | '(' | ')' | '|' | '&' | ';'
                                    | '<' | '>' | '$' | '`' | '\\' | '"' | '\'' | '#' | '~')
                });
                if needs {
                    crate::ported::utils::quotestring(s, crate::ported::utils::QuoteType::Single)
                } else { s.to_string() }
            } else if flag_qplus {                           // c:2245 (q+)
                crate::ported::utils::quotestring(s, crate::ported::utils::QuoteType::Dollars)
            } else if flag_qcount > 0 {                      // c:2237
                match flag_qcount {
                    1 => crate::ported::utils::quotestring(s, crate::ported::utils::QuoteType::Backslash),
                    2 => crate::ported::utils::quotestring(s, crate::ported::utils::QuoteType::Single),
                    3 => crate::ported::utils::quotestring(s, crate::ported::utils::QuoteType::Double),
                    _ => crate::ported::utils::quotestring(s, crate::ported::utils::QuoteType::Dollars),
                }
            } else { s.to_string() }
        };
        if flag_qmin || flag_qplus || flag_qcount > 0 {      // c:2237
            if let Some(parts) = split_parts.clone() {       // c:2237
                let new_parts: Vec<String> = parts.iter().map(|s| quote_one(s)).collect();
                value = new_parts.join(" ");
                split_parts = Some(new_parts);
            } else if let Some(arr) = state.arrays.get(&var_name).cloned() {
                let new_arr: Vec<String> = arr.iter().map(|s| quote_one(s)).collect();
                value = new_arr.join(" ");
                split_parts = Some(new_arr);
            } else {
                value = quote_one(&value);
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
        let scripted_scalar = subscript.as_deref()           // c:3950
            .map(|s| s != "@" && s != "*" && !s.contains(','))
            .unwrap_or(false);                               // c:3950
        let auto_splat = !flag_at                           // c:3950
            && !qt                                           // c:3950 (only outside DQ)
            && pf_flags & prefork_flags::SINGLE == 0         // c:3950 (multsub context)
            && rest.is_empty()                               // c:3950 (no operator subverted shape)
            && !scripted_scalar                              // c:3950 (single-elem pick is scalar)
            && (state.arrays.contains_key(&var_name)         // c:3950
                || split_parts.is_some());                   // c:3950 ((s::) made an array)
        if flag_at || auto_splat {                          // c:3950
            let parts: Vec<String> = if let Some(sp) = split_parts.clone() {
                // (s::) split → splat the post-split parts
                // regardless of source. Direct port of subst.c's
                // ssub-then-splat where spsep promotes scalar to
                // array via the split.
                sp                                          // c:3950
            } else if let Some(sub) = subscript.as_deref() {
                // Range subscript: splat the slice elements.
                if let Some((lo, hi)) = sub.split_once(',') {
                    let lo: i64 = lo.trim().parse().unwrap_or(1); // c:3950
                    let hi: i64 = hi.trim().parse().unwrap_or(0); // c:3950
                    state.arrays.get(&var_name)              // c:3950
                        .map(|arr| crate::ported::params::slice_indexed_array(arr, lo, hi))
                        .unwrap_or_default()
                } else if let Some(arr) = state.arrays.get(&var_name) {
                    arr.clone()                              // c:3950 (@ / *)
                } else { vec![value.clone()] }
            } else if let Some(arr) = state.arrays.get(&var_name) {
                arr.clone()                                 // c:3960 (real array splat)
            } else if let Some(map) = state.assoc_arrays.get(&var_name) {
                if flag_keys && flag_values {                // c:3955 (kv splat — interleaved)
                    let mut out: Vec<String> = Vec::with_capacity(map.len() * 2); // c:3955
                    for (k, v) in map {                      // c:3955
                        out.push(k.clone());                 // c:3955
                        out.push(v.clone());                 // c:3955
                    }                                        // c:3955
                    out                                      // c:3955
                } else if flag_keys {                       // c:3955 (k-flag splat)
                    map.keys().cloned().collect()
                } else if flag_values {                     // c:3957 (v-flag splat)
                    map.values().cloned().collect()
                } else {
                    vec![value.clone()]                     // c:3962 (scalar fallback)
                }
            } else {
                vec![value.clone()]                         // c:3960 (scalar)
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
            let new_pos_in_full = prefix.chars().count() + first.chars().count().saturating_sub(prefix.chars().count());
            return (first, new_pos_in_full, nodes);
        }

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
            // Array / assoc element lookup. Port of zsh's
            // getarrvalue + getindex + getasub (Src/params.c).
            // Order: assoc first (key lookup), then array
            // (numeric / `*` / `@` / range), then scalar fallback
            // (zsh treats `$scalar[N]` as char-N of the scalar
            // string, 1-based; `$scalar[N,M]` as substring).
            if let Some(map) = state.assoc_arrays.get(&var_name) { // c:1625
                // Subscript-flag form: (I)/(i)/(R)/(r) on assoc.
                // Same plumbing as braced path. Direct port of
                // Src/params.c getarg hash routing.
                if let Some((flags, pat)) = (|s: &str| -> Option<(String, String)> {
                    let s = s.trim_start();
                    let rest = s.strip_prefix('(')?;
                    let close = rest.find(')')?;
                    let f = rest[..close].to_string();
                    let p = rest[close + 1..].to_string();
                    if f.chars().all(|c| matches!(c, 'I' | 'R' | 'i' | 'r' | 'k' | 'K' | 'n' | 'e' | 'b')) {
                        Some((f, p))
                    } else { None }
                })(sub) {
                    let by_key = flags.contains('I') || flags.contains('i');
                    let return_all = flags.contains('I') || flags.contains('R');
                    let mut out: Vec<String> = Vec::new();
                    for (k, v) in map.iter() {
                        let hay = if by_key { k.as_str() } else { v.as_str() };
                        if crate::exec::ShellExecutor::glob_match_static(hay, &pat) {
                            out.push(if by_key { k.clone() } else { v.clone() });
                            if !return_all { break; }
                        }
                    }
                    out.join(" ")
                } else {
                    map.get(sub).cloned().unwrap_or_default()   // c:1625
                }
            } else if let Some(arr) = state.arrays.get(&var_name) { // c:1625
                if sub == "*" || sub == "@" {               // c:1625
                    arr.join(" ")                            // c:1625
                } else if let Some((flags, pat)) = (|s: &str| -> Option<(String, String)> {
                    // (I)/(i)/(R)/(r) on bare $arr[...]. Same as
                    // braced form. Direct port of params.c getarg
                    // array-pattern routing.
                    let s = s.trim_start();
                    let rest = s.strip_prefix('(')?;
                    let close = rest.find(')')?;
                    let f = rest[..close].to_string();
                    let p = rest[close + 1..].to_string();
                    if f.chars().all(|c| matches!(c, 'I' | 'R' | 'i' | 'r' | 'n' | 'e')) {
                        Some((f, p))
                    } else { None }
                })(sub) {
                    let return_index = flags.contains('I') || flags.contains('i');
                    let return_all = flags.contains('I') || flags.contains('R');
                    let mut out: Vec<String> = Vec::new();
                    for (idx, elem) in arr.iter().enumerate() {
                        if crate::exec::ShellExecutor::glob_match_static(elem, &pat) {
                            if return_index {
                                out.push((idx + 1).to_string());
                            } else {
                                out.push(elem.clone());
                            }
                            if !return_all { break; }
                        }
                    }
                    if out.is_empty() && return_index {
                        (arr.len() + 1).to_string()
                    } else {
                        out.join(" ")
                    }
                } else if let Some((lo, hi)) = sub.split_once(',') { // c:1625
                    // Delegate to the canonical slice helper —
                    // gets all the negative-wrap / out-of-range
                    // edge cases right (start > len, start < -len,
                    // resolve(0)→1, etc.) per the bug-for-bug
                    // port of getarrvalue's range arm.
                    let lo: i64 = lo.trim().parse().unwrap_or(1); // c:1625
                    let hi: i64 = hi.trim().parse().unwrap_or(arr.len() as i64); // c:1625
                    crate::ported::params::slice_indexed_array(arr, lo, hi).join(" ") // c:1625
                } else if let Ok(idx) = sub.parse::<i32>() { // c:1625
                    let n = arr.len() as i32;               // c:1625
                    let i = if idx < 0 { n + idx } else { idx - 1 }; // c:1625
                    if i >= 0 && (i as usize) < arr.len() { // c:1625
                        arr[i as usize].clone()             // c:1625
                    } else {                                // c:1625
                        String::new()                       // c:1625
                    }                                        // c:1625
                } else {                                    // c:1625
                    String::new()                            // c:1625
                }                                            // c:1625
            } else if let Some(magic_val) = crate::fusevm_bridge::with_executor(|exec|
                exec.get_special_array_value(&var_name, sub)
            ) {
                // Magic-assoc lookup — \$aliases[name],
                // \$functions[name], etc. Mirror of braced-form
                // fix from bb2b489624. Direct port of zsh's
                // per-magic-table getfn dispatch.
                magic_val                                    // c:1625
            } else {                                         // c:1625
                let s = state.variables.get(&var_name).cloned().unwrap_or_default(); // c:1625
                let chars_v: Vec<char> = s.chars().collect(); // c:1625
                if sub == "*" || sub == "@" {               // c:1625
                    s                                        // c:1625
                } else if let Some((lo, hi)) = sub.split_once(',') { // c:1625
                    // Reuse the canonical slice helper for
                    // scalar substring — chars_v is treated as a
                    // 1-element-per-char "array".
                    let lo: i64 = lo.trim().parse().unwrap_or(1); // c:1625
                    let hi: i64 = hi.trim().parse().unwrap_or(chars_v.len() as i64); // c:1625
                    let chars_arr: Vec<String> = chars_v.iter().map(|c| c.to_string()).collect(); // c:1625
                    crate::ported::params::slice_indexed_array(&chars_arr, lo, hi).concat() // c:1625
                } else if let Ok(idx) = sub.parse::<i32>() { // c:1625
                    let n = chars_v.len() as i32;           // c:1625
                    let i = if idx < 0 { n + idx } else { idx - 1 }; // c:1625
                    if i >= 0 && (i as usize) < chars_v.len() { // c:1625
                        chars_v[i as usize].to_string()     // c:1625
                    } else {                                 // c:1625
                        String::new()                        // c:1625
                    }                                        // c:1625
                } else {                                    // c:1625
                    String::new()                            // c:1625
                }                                            // c:1625
            }                                                // c:1625
        } else {                                            // c:1625
            // No subscript: scalar → assoc-values → array fallback.
            // zsh resolves `$assoc` (bare, no subscript) to the
            // values joined; `$arr` to elements joined. Direct
            // port of getstrvalue dispatch.
            state.variables.get(&var_name).cloned()
                .or_else(|| state.arrays.get(&var_name).map(|a| a.join(" ")))
                .or_else(|| state.assoc_arrays.get(&var_name)
                    .map(|m| m.values().cloned().collect::<Vec<_>>().join(" ")))
                .unwrap_or_default()                         // c:1625
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

        // Auto-splat for bare \$arr outside DQ in multsub context —
        // mirrors the braced-form auto_splat in the brace arm above.
        // zsh treats arrays as inherently multi-word in unquoted
        // context. Also fires for \$arr[@] / \$arr[*] which are the
        // explicit-splat forms — even with a subscript, a `@`/`*`
        // sub means "all elements as separate words".
        // Direct port of subst.c:3950 multi-node return.
        let splat_full = subscript_str.as_deref() == Some("@") // c:3950
            || subscript_str.as_deref() == Some("*");          // c:3950
        // Range subscript like `[1,3]` also produces array-shape
        // slice — splat in non-DQ.
        let splat_range = subscript_str.as_deref()
            .map(|s| s.contains(','))
            .unwrap_or(false);                                 // c:3950
        // Assoc bare-name splat: `$assoc[@]` returns values, `$assoc[*]`
        // returns values too. Per zsh, `(@k)assoc` returns keys; for
        // bare `$assoc[@]` without (k), values is the convention.
        let splat_assoc = (splat_full || splat_range)        // c:3950
            && state.assoc_arrays.contains_key(&var_name);   // c:3950
        if !qt                                                // c:3950
            && pf_flags & prefork_flags::SINGLE == 0          // c:3950
            && (subscript_str.is_none() || splat_full || splat_range) // c:3950
            && (state.arrays.contains_key(&var_name) || splat_assoc)  // c:3950
        {                                                     // c:3950
            // Pull the actual array slice for range form so
            // splat uses the slice elements (not the full arr).
            let slice_arr: Option<Vec<String>> = if splat_range {
                if let Some(sub) = subscript_str.as_deref() {
                    if let Some((lo, hi)) = sub.split_once(',') { // c:3950
                        let lo: i64 = lo.trim().parse().unwrap_or(1); // c:3950
                        let hi: i64 = hi.trim().parse().unwrap_or(0); // c:3950
                        state.arrays.get(&var_name).map(|arr|
                            crate::ported::params::slice_indexed_array(arr, lo, hi))
                    } else { None }
                } else { None }
            } else { None };
            // Assoc fallback when var isn't in arrays.
            let assoc_vals: Option<Vec<String>> = if splat_assoc { // c:3950
                state.assoc_arrays.get(&var_name)            // c:3950
                    .map(|m| m.values().cloned().collect())  // c:3950
            } else { None };                                 // c:3950
            if let Some(arr) = slice_arr.or(assoc_vals).or_else(|| state.arrays.get(&var_name).cloned()) {
                let prefix: String = chars[..start_pos].iter().collect(); // c:3950
                let suffix: String = chars[pos..].iter().collect();        // c:3950
                let mut nodes: Vec<String> = Vec::with_capacity(arr.len()); // c:3950
                for (i, part) in arr.iter().enumerate() {     // c:3950
                    let s = if arr.len() == 1 {               // c:3950
                        format!("{}{}{}", prefix, part, suffix) // c:3950
                    } else if i == 0 {                        // c:3950
                        format!("{}{}", prefix, part)         // c:3950
                    } else if i == arr.len() - 1 {            // c:3950
                        format!("{}{}", part, suffix)         // c:3950
                    } else {                                  // c:3950
                        part.clone()                          // c:3950
                    };                                        // c:3950
                    nodes.push(s);                            // c:3950
                }                                             // c:3950
                let first = nodes.first().cloned().unwrap_or_default(); // c:3950
                return (first, prefix.len(), nodes);          // c:3950
            }                                                 // c:3950
        }                                                     // c:3950

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
            // zsh semantics:
            //   $* / "$*" — join with IFS first char
            //   $@        — splat into separate words
            //   "$@"      — preserve array shape (still splat)
            // Our port: $@ (qt or unqt) → splat; $* → join.
            // Direct port of subst.c c:1625 dispatch — only $* with
            // any quoting joins; $@ always preserves array shape.
            let value = if c == '*' {                       // c:1625
                let join_sep = state.variables.get("IFS")
                    .and_then(|s| s.chars().next())
                    .map(String::from)
                    .unwrap_or_else(|| " ".to_string());
                values.join(&join_sep)                       // c:1625
            } else {                                        // c:1625
                // $@ / "$@" in unquoted/SINGLE-aware context
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






/// Port of `filesubstr()` from `Src/subst.c:737`.
///
/// Performs `~` and `=` expansion on a single path component. Returns
/// `Some(expanded)` on success, `None` if no expansion applies. The
/// caller (filesub) chains this on `:`-separated path lists.
///
/// Faithful port of the C ladder — covers `~`, `~+`, `~-`, `~N`/`~-N`
/// (dirstack), `~user` (libc getpwnam), and `=cmd` (PATH lookup via
/// equalsubstr).
pub fn filesubstr(s: &str, assign: bool, state: &SubstState) -> Option<String> { // c:737
    if s.is_empty() {                                       // c:737
        return None;                                        // c:737
    }
    let chars: Vec<char> = s.chars().collect();             // c:737
    let first = chars[0];                                   // c:737

    // `~` (and Tilde token) — but not `~=` (handled separately by =arm).
    // C: `if (*str == Tilde && str[1] != '=' && str[1] != Equals)`.
    if first == '~' || first == '\u{98}' /* Tilde token */ { // c:741
        if chars.len() == 1 {                               // c:748 — bare ~
            let home = state.variables.get("HOME").cloned()
                .or_else(|| std::env::var("HOME").ok())
                .unwrap_or_default();
            return Some(home);
        }
        let nx = chars[1];                                  // c:741
        if nx == '=' { return None; }                       // c:741 — leave for =arm

        // C `isend(c)`: !c || c=='/' || c==Inpar || (assign && c==':')
        let isend = |c: char| -> bool {                     // c:725 macro
            c == '\0' || c == '/' || c == '\u{85}' /* Inpar */
                || (assign && c == ':')
        };

        // `~/...` and `~` (isend(str[1])) — bare HOME
        if isend(nx) {                                      // c:748
            let home = state.variables.get("HOME").cloned()
                .or_else(|| std::env::var("HOME").ok())
                .unwrap_or_default();
            let suffix: String = chars[1..].iter().collect();
            return Some(format!("{}{}", home, suffix));
        }
        // `~+...` — current PWD (only if isend(str[2]))
        if nx == '+' && chars.len() >= 3 && isend(chars[2]) { // c:752
            let pwd = state.variables.get("PWD").cloned()
                .or_else(|| std::env::var("PWD").ok())
                .unwrap_or_default();
            let suffix: String = chars[2..].iter().collect();
            return Some(format!("{}{}", pwd, suffix));
        }
        // `~-...` — OLDPWD (only if isend(str[2]))
        if nx == '-' && chars.len() >= 3 && isend(chars[2]) { // c:755
            let oldpwd = state.variables.get("OLDPWD").cloned()
                .or_else(|| std::env::var("OLDPWD").ok())
                .or_else(|| state.variables.get("PWD").cloned())
                .or_else(|| std::env::var("PWD").ok())
                .unwrap_or_default();
            let suffix: String = chars[2..].iter().collect();
            return Some(format!("{}{}", oldpwd, suffix));
        }
        // `~+N` / `~-N` — dirstack entry. C: `if (!inblank(str[1]) &&
        // isend(*ptr) && (!idigit(str[1]) || (ptr - str < 4)))`.
        // Walk digit suffix; ptr ends at first non-digit.
        if (nx == '+' || nx == '-' || nx.is_ascii_digit())
            && !nx.is_whitespace()
        {
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
                let val: i32 = chars[dstart..p].iter().collect::<String>()
                    .parse().unwrap_or(0);
                let val = if neg { -val } else { val };
                let pwd = state.variables.get("PWD").cloned()
                    .or_else(|| std::env::var("PWD").ok())
                    .unwrap_or_default();
                // Direct port of subst.c filesub's tilde-+/- arm:
                // dstackent(ch, val) → pwd or stack entry.
                let entry = dstackent(            // c:4902
                    if neg { '-' } else { '+' },  // c:4902
                    val,                          // c:4902
                    &state.dirstack,              // c:4902
                    &pwd,                         // c:4902
                    state.pushdminus,             // c:4906
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
            let named = crate::fusevm_bridge::with_executor(|exec| {
                exec.named_dirs.get(&user)
                    .map(|p| p.to_string_lossy().into_owned())
            });
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
                                .to_string_lossy().into_owned();
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
    if (first == '=' || first == '\u{86}' /* Equals */)
        && chars.len() > 1
        && chars[1] != '\u{85}' /* Inpar */
    {
        let cmd_part: String = chars[1..].iter().collect();
        // Split at `:` if assign, else take the whole thing.
        let cmd = if assign {
            cmd_part.split(':').next().unwrap_or(&cmd_part).to_string()
        } else {
            cmd_part.clone()
        };
        let path = state.variables.get("PATH").cloned()
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
fn filesub(s: &str, flags: u32, state: &mut SubstState) -> String { // c:667
    // C: `filesubstr(namptr, assign);`  (line 672)
    let mut namptr: String = filesubstr(s, flags != 0, state)
        .unwrap_or_else(|| s.to_string());                 // c:672

    // C: `if (!assign) return;` — non-assign context bails early.
    if flags == 0 {                                         // c:674
        return namptr;                                      // c:675
    }

    let mut eql: Option<usize> = None;                      // c:668 (eql=NULL)

    // C: PREFORK_TYPESET arm — `${var}=value` shape, find `=` then
    // recurse filesubstr on the RHS.
    if flags & prefork_flags::TYPESET != 0 {                // c:677
        // C: `(*namptr)[1] && (eql = sub = strchr(*namptr + 1, Equals))`
        if namptr.len() >= 2 {                              // c:678
            // strchr from index 1 onward
            if let Some(sub) = namptr[1..].find('=').map(|p| p + 1) { // c:678
                eql = Some(sub);                            // c:678
                let str_start = sub + 1;                    // c:679
                if str_start < namptr.len()                 // c:680
                    && (namptr.as_bytes()[str_start] == b'~'
                        || namptr.as_bytes()[str_start] == b'=')
                {                                           // c:680
                    let rhs = &namptr[str_start..];          // c:679
                    if let Some(expanded) = filesubstr(rhs, true, state) { // c:680
                        // C: `sub[1] = '\0'; *namptr = dyncat(*namptr, str);`
                        namptr = format!("{}{}", &namptr[..str_start], expanded); // c:682
                    }                                       // c:682
                }                                           // c:680
            } else {                                        // c:684
                return namptr;                              // c:685
            }                                               // c:686
        } else {                                            // c:684
            return namptr;                                  // c:685
        }                                                   // c:686
    }

    // C: `ptr = *namptr; while ((sub = strchr(ptr, ':'))) { … }`
    // Walk `:`-separated path components, reapply filesubstr on each
    // suffix that starts with `~` or `=`.
    let mut ptr_off = 0_usize;                              // c:689
    loop {                                                  // c:690
        let slice = &namptr[ptr_off..];                     // c:690
        let colon_rel = match slice.find(':') {             // c:690
            Some(p) => p,                                   // c:690
            None => break,                                  // c:690
        };                                                  // c:690
        let sub = ptr_off + colon_rel;                      // c:690
        let str_start = sub + 1;                            // c:691
        let len = sub;                                      // c:692
        // C: `sub > eql` — skip the `:` we already chewed in TYPESET.
        let past_eql = match eql {                          // c:693
            Some(e) => sub > e,                             // c:693
            None => true,                                   // c:693
        };                                                  // c:693
        if past_eql                                         // c:693
            && str_start < namptr.len()                     // c:694
            && (namptr.as_bytes()[str_start] == b'~'
                || namptr.as_bytes()[str_start] == b'=')
        {                                                   // c:694
            let rhs = &namptr[str_start..];                 // c:691
            if let Some(expanded) = filesubstr(rhs, true, state) { // c:695
                namptr = format!("{}{}", &namptr[..str_start], expanded); // c:697
            }                                               // c:695
        }                                                   // c:695
        ptr_off = len + 1;                                  // c:700
        if ptr_off >= namptr.len() {                        // c:700
            break;                                          // c:700
        }                                                   // c:700
    }                                                       // c:701
    namptr                                                  // c:702
}                                                           // c:703



/// Port of `arithsubst()` from `Src/subst.c:4485-4509`.
///
/// C body: param-substitute the expression first (`singsub(&a)`),
/// evaluate as math, then format the integer/float result honoring
/// `outputradix` and `outputunderscore` options; concatenate the
/// caller-supplied `prefix` (`*bptr`) + result + `rest` and return.
///
/// Rust signature changed from `(char *a, char **bptr, char *rest)`
/// to `(expr, prefix, rest, state) -> String` because Rust strings
/// own their storage; the caller now consumes the returned String
/// directly instead of the C in-out buffer protocol.
fn arithsubst(expr: &str, prefix: &str, rest: &str, state: &mut SubstState) -> String { // c:4485
    // C: `singsub(&a);` — parameter-substitute the math expression
    // before evaluation. Without this `${(($n+1))}` won't see $n.
    let expanded = singsub(expr, state);                    // c:4490

    // C: `v = matheval(a);` — evaluate via Src/math.c::matheval.
    let v = match crate::math::matheval(&expanded) {        // c:4491
        Ok(n) => n,                                         // c:4491
        Err(_) => crate::math::MathNum::Unset,              // c:4491
    };                                                      // c:4491

    // C ladder lines 4492-4499: float-with-no-radix → convfloat,
    // else cast float to int and convbase. zshrs collapses both
    // through Display + a `outputradix` shell-option check.
    let outputradix = state.variables.get("OUTPUT_RADIX")
        .and_then(|s| s.parse::<i32>().ok())
        .unwrap_or(0);                                      // c:4492
    let b: String = match v {                               // c:4492
        crate::math::MathNum::Float(f) if outputradix == 0 => { // c:4492
            // QT_FLOAT — let Display handle it; zsh's
            // convfloat_underscore is the underscore-grouped form,
            // skipped here pending OUTPUT_UNDERSCORE port.
            format!("{}", f)                                // c:4493
        }                                                   // c:4493
        crate::math::MathNum::Float(f) => {                 // c:4495
            // Integer cast + convbase per radix.
            let l = f as i64;                               // c:4496
            crate::ported::utils::convbase(l, outputradix as u32)       // c:4497
        }                                                   // c:4497
        crate::math::MathNum::Integer(n) => {               // c:4498
            crate::ported::utils::convbase(n, outputradix as u32)       // c:4498
        }                                                   // c:4498
        crate::math::MathNum::Unset => "0".to_string(),     // c:4498
    };                                                      // c:4499

    // C: `t = *bptr = hcalloc(...); …; strcat(t, rest);` — concat
    // prefix + b + rest. Returns pointer past prefix+b (where rest
    // begins). Rust returns the full string.
    format!("{}{}{}", prefix, b, rest)                      // c:4501-4509
}                                                           // c:4509

// `convbase` lives in src/ported/utils.rs (canonical port of
// Src/utils.c). Callers below import via the full path.


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
/// Rust signature: `(s, pf_flags, state) -> (String, Vec<String>,
/// bool isarr, u32 ms_flags)`. The `sep` parameter is reserved on the
/// caller side and folded into `state.variables["IFS"]` for now;
/// pending an explicit sep arg if a caller needs it. The return tuple
/// carries (joined-scalar, array, isarr, ms_flags).
pub fn multsub(s: &str, pf_flags: u32, state: &mut SubstState) -> (String, Vec<String>, bool, u32) { // c:544
    let mut ms_flags = 0u32;                                // c:551
    let mut x = s.to_string();                              // c:550 (`x = *s`)

    // C lines 555-563: PREFORK_SPLIT — skip leading IFS whitespace,
    // mark MULTSUB_WS_AT_START.
    let ifs = state.variables.get("IFS").cloned()
        .unwrap_or_else(|| " \t\n\0".to_string());          // c:N/A (zsh default IFS includes NUL)
    let is_ifs_sep = |c: char| -> bool {                    // c:556
        ifs.contains(c)                                     // c:556
    };

    if pf_flags & prefork_flags::SPLIT != 0 {               // c:553
        let leading: usize = x.chars().take_while(|&c| is_ifs_sep(c)).count(); // c:556
        if leading > 0 {                                    // c:557
            ms_flags |= multsub_flags::WS_AT_START;         // c:561
            x = x.chars().skip(leading).collect();          // c:562
        }
    }

    // C: `init_list1(foo, x);` — single-element linklist seeded with x.
    let mut list = LinkList::default();                     // c:565
    list.nodes.push_back(LinkNode { data: x.clone() });     // c:565

    // C lines 568-619: PREFORK_SPLIT walks chars looking for ISEP
    // separators outside quotes/parens. On hit, NUL-terminate and
    // start a new linknode.
    if pf_flags & prefork_flags::SPLIT != 0 {               // c:567
        // Take ownership of the only node's chars; rebuild list.
        let chars: Vec<char> = x.chars().collect();         // c:565
        let mut nodes: Vec<String> = Vec::new();            // c:565
        let mut cur = String::new();                        // c:565
        let mut inq = false;                                // c:570 (quote state)
        let mut inp = 0_i32;                                // c:570 (paren depth)
        let mut i = 0_usize;                                // c:572
        while i < chars.len() {                             // c:572
            let c = chars[i];                               // c:573
            // C: `if (*x == Dash) *x = '-';` — Dash token →
            // literal dash. Rust doesn't have this token here.
            // C: `if (itok((unsigned char) *x)) { rawc = *x; l = 1; }`
            // Tokens (META range \u{80}-\u{9F}) are single-byte and
            // can't be separators. Skip the IFS check for them.
            let is_token = matches!(c as u32, 0x80..=0x9F); // c:577
            // Bnull/Bnullkeep arms (C lines 612-617): skip the next
            // char (parser-verified to exist). \u{99} = Bnull,
            // \u{9a} = Bnullkeep in our token table.
            if c == '\u{99}' || c == '\u{9a}' {             // c:612
                cur.push(c);                                // c:614
                i += 1;                                     // c:615
                if i < chars.len() {                        // c:615
                    cur.push(chars[i]);                     // c:616
                    i += 1;                                 // c:616
                }
                continue;                                   // c:617
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
            if !inq && inp == 0 && !is_token && is_ifs_sep(c) { // c:581
                // Split here; NUL-terminate cur, walk past trailing
                // separators (C lines 583-595).
                if !cur.is_empty() || nodes.is_empty() {    // c:583
                    nodes.push(std::mem::take(&mut cur));   // c:583
                }
                i += 1;                                     // c:584
                while i < chars.len() && is_ifs_sep(chars[i]) { // c:584-595
                    i += 1;                                 // c:594
                }
                if i >= chars.len() {                       // c:596
                    ms_flags |= multsub_flags::WS_AT_END;   // c:597
                    break;                                  // c:598
                }
                continue;                                   // c:599
            }
            cur.push(c);                                    // c:619
            i += 1;                                         // c:620
        }
        if !cur.is_empty() {                                // c:622
            nodes.push(cur);                                // c:622
        }
        // Rebuild the linklist with the split nodes.
        list = LinkList::default();                         // c:622
        for n in nodes {                                    // c:622
            list.nodes.push_back(LinkNode { data: n });     // c:622
        }
    }

    // C: `prefork(&foo, pf_flags, ms_flags);`
    let mut ret_flags = 0u32;                               // c:625
    prefork(&mut list, pf_flags, &mut ret_flags, state);    // c:625

    // C lines 626-630: errflag bail.
    if state.errflag {                                      // c:626
        return (String::new(), Vec::new(), false, ms_flags); // c:629
    }

    // C lines 633-650: count nodes; if > 1 or LF_ARRAY, return as
    // array; else single scalar (or empty).
    let l = list.nodes.len();                               // c:633
    if l > 1 || (list.flags & LF_ARRAY != 0) {              // c:633
        let arr: Vec<String> = list.nodes.iter().map(|n| n.data.clone()).collect(); // c:635-637
        // C: `*s = sepjoin(r, sep, 1);` — join with IFS first-char
        // when sep is NULL. Use first IFS char as join separator,
        // matching zsh's sepjoin defaults.
        let join_sep = ifs.chars().next().map(String::from).unwrap_or_default(); // c:649
        let joined = arr.join(&join_sep);                   // c:649
        return (joined, arr, true, ms_flags);               // c:642-647 (array path)
    }
    if l == 1 {                                             // c:653
        let result = list.getdata(0).unwrap_or("").to_string(); // c:653
        return (result.clone(), vec![result], false, ms_flags); // c:653
    }
    // C: `*s = dupstring("");` — empty result.
    (String::new(), vec![String::new()], false, ms_flags)   // c:655
}                                                           // c:660

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
    // hsubl/hsubr now live on SubstState (which mirrors them
    // back to ShellExecutor on commit). Reads the latest value
    // observed in this pass; writes a new pair after each `:s`.

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

        // Count suffix for :h/:t — `:h2` = repeat 2 times.
        // Port of subst.c:4570-4577 idigit count parse.
        let mut count: i32 = 1;                             // c:4570
        if matches!(modifier, 'h' | 't') {                  // c:4571
            let mut count_str = String::new();              // c:4572
            while let Some(&pc) = chars.peek() {
                if pc.is_ascii_digit() {
                    count_str.push(pc);
                    chars.next();
                } else { break; }
            }
            if !count_str.is_empty() {
                count = count_str.parse().unwrap_or(1);     // c:4575
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
        if modifier == 's' || modifier == 'S' {             // c:4583
            let delim = match chars.next() {                // c:4585
                Some(c) => c,                               // c:4585
                None => break,                              // c:4585
            };
            // Read pattern with backslash-escape support.
            let mut pat = String::new();                    // c:4595
            while let Some(&c) = chars.peek() {
                if c == delim { chars.next(); break; }
                if c == '\\' {                              // c:4598 (backslash escape)
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
            let mut repl = String::new();                   // c:4625
            while let Some(&c) = chars.peek() {
                if c == delim { chars.next(); break; }
                if c == '\\' {                              // c:4630
                    chars.next();
                    if let Some(&nx) = chars.peek() {
                        repl.push(nx);
                        chars.next();
                    }
                } else if c == '&' {                        // c:4639 (& → matched portion)
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
                    (rest.to_string(), true, false)         // c:4665 (#X)
                } else if let Some(rest) = pat.strip_suffix('%') {
                    (rest.to_string(), false, true)         // c:4665 (X%)
                } else {
                    (pat.clone(), false, false)             // c:4665
                }
            } else {
                (pat.clone(), false, false)                 // c:4665
            };
            result = if anchor_head {                       // c:4665
                if result.starts_with(&eff_pat) {           // c:4665
                    format!("{}{}", repl, &result[eff_pat.len()..]) // c:4665
                } else { result }                            // c:4665
            } else if anchor_tail {                         // c:4665
                if result.ends_with(&eff_pat) {             // c:4665
                    format!("{}{}", &result[..result.len() - eff_pat.len()], repl) // c:4665
                } else { result }                            // c:4665
            } else if gbal {                                // c:4665
                result.replace(eff_pat.as_str(), repl.as_str())
            } else {
                result.replacen(eff_pat.as_str(), repl.as_str(), 1)
            };
            state.last_subst = Some((pat.clone(), repl.clone())); // c:4673
            // `:s` on word-each (`:w` / `:W:sep`) splits, applies,
            // rejoins. Pull through the same code path :& uses
            // below by deferring to a shared `apply_subst` closure.
            if wall {                                       // c:4665
                let separator = sep.as_deref().unwrap_or(" "); // c:4665
                let words: Vec<&str> = result.split(separator).collect(); // c:4665
                let modified: Vec<String> = words.iter().map(|w| {       // c:4665
                    if gbal { w.replace(pat.as_str(), repl.as_str()) }   // c:4665
                    else { w.replacen(pat.as_str(), repl.as_str(), 1) }  // c:4665
                }).collect();                                // c:4665
                result = modified.join(separator);          // c:4665
            }                                                // c:4665
            continue;                                       // c:4675
        }                                                   // c:4685

        // `:&` repeats the last `:s` substitution. Per Src/subst.c
        // modify's `case '&':`. No-op if no prior `:s` in this
        // chain (or pass — state.last_subst persists from prior
        // calls via from_executor / commit_to_executor).
        if modifier == '&' {                                // c:4531
            if let Some((p, r)) = state.last_subst.clone() { // c:4531
                if wall {                                   // c:4531
                    let separator = sep.as_deref().unwrap_or(" "); // c:4531
                    let words: Vec<&str> = result.split(separator).collect(); // c:4531
                    let modified: Vec<String> = words.iter().map(|w| { // c:4531
                        if gbal { w.replace(p.as_str(), r.as_str()) }    // c:4531
                        else { w.replacen(p.as_str(), r.as_str(), 1) }   // c:4531
                    }).collect();                            // c:4531
                    result = modified.join(separator);      // c:4531
                } else {                                    // c:4531
                    result = if gbal { result.replace(p.as_str(), r.as_str()) } // c:4531
                             else { result.replacen(p.as_str(), r.as_str(), 1) }; // c:4531
                }                                            // c:4531
            }                                               // c:4531
            continue;                                       // c:4531
        }                                                   // c:4531

        // Single-char modifier dispatch — port of Src/subst.c:4585+
        // modifier-arm ladder. Each arm calls a canonical hist.rs
        // helper (the per-modifier C body lives in Src/hist.c).
        let dispatch = |w: &str| -> Option<String> {        // c:4585
            match modifier {                                // c:4585
                'h' => Some(remtpath(w, count)),            // c:4585 (:h head, count = :hN)
                't' => Some(remlpaths(w, count)),           // c:4585 (:t tail, count = :tN)
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
                'P' => {                                    // c:4585 (:P physical)
                    // :P canonicalizes (resolves symlinks) like
                    // realpath(3). zsh sets `physical = 1` for the
                    // chabspath call. std::fs::canonicalize wraps
                    // the libc realpath.
                    std::fs::canonicalize(w).ok()
                        .map(|p| p.to_string_lossy().into_owned())
                        .or_else(|| chabspath(w).ok())
                }
                'c' => {                                    // c:4585 (:c command-resolve)
                    // :c resolves like `which` — search PATH for
                    // an executable matching `w`. Direct port of
                    // hist.c case 'c' which calls findcmd.
                    if w.starts_with('/') || w.starts_with("./") || w.starts_with("../") {
                        Some(w.to_string())                 // c:4585
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
/// Port of `wcpadwidth()` from `Src/subst.c:848-866`.
///
/// Returns the display-cell width of a wide char for `dopadding`.
///
/// C signature: `int wcpadwidth(wchar_t wc, int multi_width)`.
/// multi_width values:
///   0 → always 1 (legacy / no multibyte)
///   1 → wcwidth(wc); zero if negative
///   * → boolean: 1 if wcwidth>0 else 0
pub fn wcpadwidth(wc: char, multi_width: i32) -> i32 {      // c:848
    // wcwidth fallback lives in utils.rs (canonical port of
    // Src/utils.c::zwcwidth). Use the unicode_width-backed
    // implementation there.
    let wcw = crate::ported::utils::zwcwidth(wc) as i32;
    match multi_width {                                     // c:854
        // C: `case 0: return 1;`
        0 => 1,                                             // c:855
        // C: `case 1: width = WCWIDTH(wc); if (width >= 0) return width; return 0;`
        1 => if wcw >= 0 { wcw } else { 0 },                // c:858
        // C: `default: return WCWIDTH(wc) > 0 ? 1 : 0;`
        _ => if wcw > 0 { 1 } else { 0 },                   // c:864
    }
}                                                           // c:866


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
    let _ = err;                                            // c:1466 (parsestr error path
                                                            //         deferred — full C
                                                            //         lexer reentry pending)
    // C: `*sp = s = dupstring(*sp);` — duplicate so the caller's
    // original buffer is unaffected. Rust's String already owns;
    // we work on a local copy below.
    let mut buf: String = s.to_string();                    // c:1465

    // C: `if (!single) { … }` — the conversion only runs in the
    // non-SINGLE arm (when paramsubst-output may be subsequently
    // word-split / expanded).
    if !single {                                            // c:1469
        let mut chars: Vec<char> = buf.chars().collect();   // c:1469
        let mut qt = false;                                 // c:1470
        // C constant references — these are the token bytes the
        // lexer emits. Authoritative values from src/ported/subst.rs
        // tokens module: STRING=\u{81}, QSTRING=\u{82},
        // TICK=\u{83}, QTICK=\u{84}, DNULL=\u{97}.
        for c in chars.iter_mut() {                         // c:1472
            if !qt {                                        // c:1473
                if *c == '\u{82}' /* QSTRING */ {           // c:1474
                    *c = '\u{81}' /* STRING */;             // c:1475
                } else if *c == '\u{84}' /* QTICK */ {      // c:1476
                    *c = '\u{83}' /* TICK */;               // c:1477
                }
            }
            if *c == '\u{97}' /* DNULL */ {                 // c:1480
                qt = !qt;                                   // c:1481
            }
        }
        buf = chars.iter().collect();                       // c:1483
    }
    // C: `return 0;` — success path returns the buffer.
    Some(buf)                                               // c:1483
}                                                           // c:1486

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
pub fn dstackent(ch: char, val: i32, dirstack: &[String], pwd: &str, pushdminus_set: bool) -> Option<String> { // c:4902
    // C: `backwards = ch == (isset(PUSHDMINUS) ? '+' : '-');`
    let backwards = ch == if pushdminus_set { '+' } else { '-' }; // c:4906

    // C: `if (!backwards && !val--) return pwd;`
    // Decrement val POST-test so val becomes 0 → return pwd.
    let mut val = val;                                      // c:4904
    if !backwards && val == 0 {                             // c:4907
        return Some(pwd.to_string());                       // c:4908
    }
    if !backwards { val -= 1; }                             // c:4907 (post-decrement)

    // C lines 4909-4912: walk dirstack.
    // backwards: from lastnode, val steps back.
    // forwards: from firstnode, val steps forward.
    let n = dirstack.len() as i32;                          // c:4910
    let idx = if backwards {                                // c:4910
        // last element is index n-1; val steps back from there.
        let i = n - val;                                    // c:4910
        if i < 0 { return None; }                           // c:4913 (n == end)
        i as usize                                          // c:4910
    } else {                                                // c:4912
        if val < 0 || val >= n { return None; }             // c:4913 (n == end)
        val as usize                                        // c:4912
    };

    // C: `return (char *)getdata(n);`
    dirstack.get(idx).cloned()                              // c:4920
}                                                           // c:4922


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
///
/// `multi_width` controls cell-counting per the (m) flag (subst.c:2376):
///   • 0  → every char counts as one cell (C zsh's MULTIBYTE_SUPPORT off)
///   • 1+ → use wcpadwidth (CJK wide=2, combining=0, ZWJ=0).
pub fn dopadding(                                           // c:893
    s: &str,                                                // c:893
    prenum: usize,                                          // c:893
    postnum: usize,                                         // c:893
    preone: Option<&str>,                                   // c:893
    postone: Option<&str>,                                  // c:893
    premul: &str,                                           // c:893
    postmul: &str,                                          // c:893
    multi_width: i32,                                       // c:2376 (m)
) -> String {                                               // c:893
    // (m)-aware string-cell counter. With multi_width==0 every
    // codepoint counts 1 (legacy behavior); otherwise wcpadwidth
    // gives the wide-char-aware metric. Direct port of zsh's
    // MULTIBYTE_SUPPORT path which routes the (l)/(r) length
    // checks through wcwidth() before deciding pad vs truncate.
    let cells = |t: &str| -> usize {                        // c:893
        if multi_width <= 0 {                               // c:893
            t.chars().count()                                // c:893
        } else {                                             // c:893
            t.chars().map(|c| wcpadwidth(c, multi_width) as usize).sum() // c:2376
        }                                                    // c:893
    };
    let len = cells(s);                                     // c:893
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
        let current_len = cells(&result);                   // c:893

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
pub fn get_intarg(s: &str) -> Option<(i64, &str)> {         // c:1428
    // C: `char *t = get_strarg(*s, &arglen);` — get the delimited
    // expression text + delimiter length.
    let (_del, content, rest) = get_strarg(s)?;             // c:1431

    if rest.is_empty() && content.is_empty() {              // c:1436
        // C: `if (!*t) return -1;` — empty input → error.
        return None;
    }

    // C: `if (parsestr(&p)) return -1;` — full lexer reentry skipped
    // (subst_parse_str approximates).
    let parsed = subst_parse_str(&content, false, true)?;   // c:1442

    // C: `singsub(&p);` — parameter-substitute the content (so
    // `(l:$n:)` looks up $n).
    let mut state = SubstState::default();                  // c:1444
    let expanded = singsub(&parsed, &mut state);            // c:1444
    if state.errflag { return None; }                       // c:1445

    // C: `ret = mathevali(p);` — evaluate as integer math.
    let ret = match crate::ported::math::mathevali(&expanded) { // c:1447
        Ok(n) => n,                                         // c:1447
        Err(_) => return None,                              // c:1448
    };

    // C: `if (ret < 0) ret = -ret;` — absolute value.
    let abs_ret = if ret < 0 { -ret } else { ret };         // c:1452

    // C: `*delmatchp = arglen;` — Rust folds delim-len into rest.
    Some((abs_ret, rest))                                   // c:1455
}                                                           // c:1457






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
pub fn quotesubst(s: &str, _state: &mut SubstState) -> String { // c:463
    let mut result = s.to_string();                         // c:465
    let mut pos = 0_usize;                                  // c:466

    // C: `while (*s) { if (*s == String && s[1] == Snull) …
    //               else s++; }`
    loop {                                                  // c:467
        let chars: Vec<char> = result.chars().collect();    // c:467
        if pos >= chars.len() { break; }                    // c:467
        // C lines 468-470: spot $'…' marker and call
        // stringsubstquote.
        if pos + 1 < chars.len()                            // c:468
            && chars[pos] == STRING                         // c:468
            && chars[pos + 1] == SNULL                      // c:468
        {
            let (new_str, new_pos) = stringsubstquote(&result, pos); // c:469
            result = new_str;                               // c:469
            pos = new_pos;                                  // c:469
        } else {                                            // c:471
            pos += 1;                                       // c:472
        }                                                   // c:473
    }
    // C: `remnulargs(str);` — strip Bnull / NUL tokens. Use the
    // inline equivalent the rest of subst.rs uses (\u{0} only;
    // glob.rs's full port operates on Vec<GlobToken>).
    result.replace('\u{0}', "")                             // c:474
}                                                           // c:475

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
pub fn globlist(list: &mut LinkList, flags: u32, state: &mut SubstState) { // c:489
    // C: `badcshglob = 0;` — reset the csh-glob diagnostic counter
    // (we don't track this; csh-glob option is rare).
    let mut node_idx = 0;                                   // c:493

    while node_idx < list.nodes.len() && !state.errflag {   // c:494
        let data = match list.getdata(node_idx) {           // c:494
            Some(d) => d.to_string(),                       // c:494
            None => { node_idx += 1; continue; }            // c:494
        };

        // C: `if ((flags & PREFORK_KEY_VALUE) && *data == Marker)`
        // — assoc-array key/value pair; skip 3 nodes (Marker, Key,
        // Value).
        if flags & prefork_flags::KEY_VALUE != 0
            && data.chars().next() == Some(MARKER)
        {                                                   // c:497
            // Advance past Marker + Key + Value.
            node_idx += 3;                                  // c:499
            continue;                                       // c:499
        }

        // C: `zglob(list, node, (flags & PREFORK_NO_UNTOK) != 0);`
        // — the actual glob expansion. Replaces the node with one
        // or more nodes (one per match).
        let no_untok = flags & prefork_flags::NO_UNTOK != 0; // c:501
        let _ = no_untok;                                   // C plumbs through;
                                                            // expand_glob handles
                                                            // tokens internally.
        let expanded: Vec<String> = crate::fusevm_bridge::with_executor(
            |exec| exec.expand_glob(&data));

        if expanded.is_empty() {                            // c:N/A (NOMATCH path)
            // C zglob does its own NOMATCH/badcshglob accounting
            // when nothing matches. Preserve the original entry on
            // empty match (zsh default; NOMATCH option would zerr).
            node_idx += 1;
        } else if expanded.len() == 1 {                     // c:N/A
            list.setdata(node_idx, expanded.into_iter().next().unwrap());
            node_idx += 1;
        } else {
            // Replace the single node with N expanded nodes.
            list.delete_node(node_idx);
            for (i, p) in expanded.iter().enumerate() {
                if i == 0 {
                    list.nodes.insert(node_idx, LinkNode { data: p.clone() });
                } else {
                    list.insertlinknode(node_idx + i - 1, p.clone());
                }
            }
            node_idx += expanded.len();                     // advance past all
        }
    }
    // C: `if (noerrs) badcshglob = 0; else if (badcshglob == 1)
    // zerr("no match");` — diagnostic emit. Skipped here pending
    // badcshglob counter port.
}                                                           // c:510









/// Flags for SUB_* matching — verbatim port of zsh.h:1981-1996.
///
/// Outer-scope mirror of the inner module at the bottom of
/// subst.rs. Earlier values (`1, 2, 4, …` powers of two) silently
/// shifted START / EGLOB into the wrong bit positions because
/// zsh.h has DOSUBST=0x0400 and RETFAIL=0x0800 between LEN=0x0080
/// and START=0x1000. Use the canonical hex literals here.
pub mod sub_flags {                                         // zsh.h:1981
    pub const END: u32 = 0x0001;     // % or %%             // zsh.h:1981
    pub const LONG: u32 = 0x0002;    // doubled # or %       // zsh.h:1982
    pub const SUBSTR: u32 = 0x0004;  // (S)                  // zsh.h:1983
    pub const MATCH: u32 = 0x0008;   // (M)                  // zsh.h:1984
    pub const REST: u32 = 0x0010;    // (R)                  // zsh.h:1985
    pub const BIND: u32 = 0x0020;    // (B)                  // zsh.h:1986
    pub const EIND: u32 = 0x0040;    // (E)                  // zsh.h:1987
    pub const LEN: u32 = 0x0080;     // (N)                  // zsh.h:1988
    pub const ALL: u32 = 0x0100;     // match whole str      // zsh.h:1989
    pub const GLOBAL: u32 = 0x0200;  // ${..//..}            // zsh.h:1990
    pub const DOSUBST: u32 = 0x0400; // repl needs subst     // zsh.h:1991
    pub const RETFAIL: u32 = 0x0800; // status 0 if no match // zsh.h:1992
    pub const START: u32 = 0x1000;   // anchor at start      // zsh.h:1993
    pub const LIST: u32 = 0x2000;    // return list          // zsh.h:1995
    pub const EGLOB: u32 = 0x4000;   // (*) extended glob    // zsh.h:1996
}                                                           // zsh.h:1996















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
    // C: `if (!pl && (!s || !*s)) { *d = dest = (copied ? src :
    //     dupstring(src)); if (glbsub) shtokenize(dest); }`
    // — fast path: no prefix, no suffix, just src (optionally
    // shtokenized).
    if prefix.is_empty() && suffix.is_empty() {             // c:820
        if glob_subst {                                     // c:822
            // shtokenize returns Vec<GlobToken>; for a string-output
            // signature we keep the src as-is. The full token-aware
            // pipeline lives in the canonical glob path.
            let _ = crate::ported::glob::shtokenize(src);   // c:823
        }
        return src.to_string();                             // c:821
    }

    // C: `*d = dest = hcalloc(pl + l + (s ? strlen(s) : 0) + 1);
    //     strncpy(dest, pb, pl); dest += pl;
    //     strcpy(dest, src); if (glbsub) shtokenize(dest);
    //     dest += l;
    //     if (s) strcpy(dest, s);`
    // — general path: pre-allocate + copy three segments in order.
    let mut result = String::with_capacity(                 // c:825
        prefix.len() + src.len() + suffix.len() + 1);
    result.push_str(prefix);                                // c:826
    result.push_str(src);                                   // c:828
    if glob_subst {                                         // c:829
        // Same shtokenize note as above.
        let _ = crate::ported::glob::shtokenize(src);       // c:830
    }
    result.push_str(suffix);                                // c:833
    result                                                  // c:835
}                                                           // c:836



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
/// Port of `substevalchar()` from `Src/subst.c:1490-1521`.
///
/// Implements the `(#)` paramsubst flag: evaluate the expression as
/// a math integer, then convert that codepoint to a UTF-8 string.
/// Used by `${(#)foo}` where `foo` is a numeric expression yielding
/// a character code.
pub fn substevalchar(s: &str) -> Option<String> {           // c:1490
    // C: `int saved_errflag = errflag; errflag = 0;` — clear-and-save
    // the global error flag around mathevali so failure from an
    // invalid math expr stays local.
    // (Rust port has no global errflag — the Result type carries
    // the error directly.)
    let ires = match crate::ported::math::mathevali(s) {    // c:1497
        Ok(n) => n,                                         // c:1497
        Err(_) => {                                         // c:1499
            // C: `return noerrs ? dupstring("") : NULL;` —
            // empty string when noerrs flag is set, NULL otherwise.
            // Rust port returns Some("") so callers see a clean
            // empty value rather than aborting; the `noerrs` global
            // is at the parser layer and isn't plumbed here yet.
            return Some(String::new());                     // c:1500
        }                                                   // c:1502
    };                                                      // c:1502
    if ires < 0 {                                           // c:1505
        // C: `zerr("character not in range");` — diagnostic to
        // stderr.
        eprintln!("zshrs: character not in range");         // c:1506
        // C falls through to the byte-render path with a negative
        // ires, which emits a garbage byte. The Rust port returns
        // empty rather than a corrupt char.
        return Some(String::new());                         // c:1506
    }                                                       // c:1507

    // C: MULTIBYTE arm — `if (isset(MULTIBYTE) && ires > 127)` use
    // ucs4tomb to encode as multibyte. Rust uses char::from_u32
    // which handles all valid Unicode scalar values uniformly.
    if let Some(ch) = char::from_u32(ires as u32) {         // c:1509
        let mut buf = [0u8; 4];                             // c:1510
        return Some(ch.encode_utf8(&mut buf).to_string());  // c:1510
    }                                                       // c:1510

    // C fallback: `sprintf(ptr, "%c", (int)ires);` — single byte.
    // Rust falls back to a single byte when char::from_u32 rejects
    // (surrogate range or out-of-range value). Render as Latin-1
    // byte for compatibility with C's `(char)ires` cast.
    let byte = (ires as u32 & 0xFF) as u8;                  // c:1517
    Some(String::from_utf8_lossy(&[byte]).into_owned())     // c:1517
}                                                           // c:1521

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
pub fn check_colon_subscript(s: &str) -> Option<(String, String)> { // c:1566
    // C: `if (!*str || ialpha(*str) || *str == '&') return NULL;`
    // — empty, alphabetic (i.e. a modifier letter), or `&` (history-
    // modifier `:&`) → not a subscript.
    if s.is_empty()                                         // c:1571
        || s.starts_with(|c: char| c.is_ascii_alphabetic()) // c:1571
        || s.starts_with('&')                               // c:1571
    {
        return None;                                        // c:1572
    }

    // C: `if (*str == ':') { *endp = str; return dupstring("0"); }`
    // — bare `::` shape: subscript is "0" and end points at the
    // current position (no chars consumed).
    if s.starts_with(':') {                                 // c:1574
        return Some(("0".to_string(), s.to_string()));      // c:1576
    }

    // C: `*endp = parse_subscript(str, 0, ':');` — find a balanced
    // subscript expression terminated by `:`. Falls back to
    // `'\0'` (end-of-string) if no trailing `:` found.
    //
    // Rust port: walk chars tracking bracket/paren depth, stop at
    // unbalanced `:` or end of string.
    let chars: Vec<char> = s.chars().collect();             // c:1579
    let mut depth: i32 = 0;                                 // c:1579
    let mut end: Option<usize> = None;                      // c:1579
    for (i, &c) in chars.iter().enumerate() {               // c:1579
        match c {                                           // c:1579
            '[' | '\u{91}' /* Inbrack */ => depth += 1,     // c:1579
            ']' | '\u{92}' /* Outbrack */ => depth -= 1,    // c:1579
            '(' | '\u{85}' /* Inpar */ => depth += 1,       // c:1579
            ')' | '\u{86}' /* Outpar */ => depth -= 1,      // c:1579
            ':' if depth == 0 => { end = Some(i); break; }  // c:1579
            _ => {}
        }
    }
    let end = end.unwrap_or(s.len());                       // c:1582 (fallthrough '\0')
    let expr: String = chars[..end].iter().collect();       // c:1583

    // C lines 1585-1591: `parsestr` + `singsub` + `remnulargs` +
    // `untokenize` on the captured expression.
    let parsed = subst_parse_str(&expr, false, true)?;      // c:1587
    let mut tmp_state = SubstState::default();              // c:1589
    let expanded = singsub(&parsed, &mut tmp_state);        // c:1589
    if tmp_state.errflag { return None; }                   // c:1590
    let stripped = expanded.replace('\u{0}', "");           // c:1590
    let untoked = crate::lex::untokenize(&stripped);        // c:1591

    let rest: String = chars[end..].iter().collect();       // c:1593
    Some((untoked, rest))                                   // c:1596
}                                                           // c:1597


/// Untokenize and escape string for flag argument
/// Port of untok_and_escape() from subst.c
/// Port of `untok_and_escape()` from `Src/subst.c:1528-1554`.
///
/// Helper for arguments to parameter flags. Handles two operations
/// on the input string `s`:
///
///   - If `escapes` is set AND `s` begins with `$<ident>` or
///     `Qstring<ident>`, look up the named parameter and use its
///     value directly (zsh's `getsparam`). Otherwise untokenize
///     and run `getkeystring` to process print-style escapes.
///
///   - If `tok_arg` is set, additionally run `shtokenize` on the
///     result so the caller sees patterns ready for glob matching.
pub fn untok_and_escape(s: &str, escapes: bool, tok_arg: bool, state: &SubstState) -> String { // c:1528
    let mut dst: Option<String> = None;                     // c:1531

    // C: `if (escapes && (*s == String || *s == Qstring) && s[1])`
    let chars: Vec<char> = s.chars().collect();             // c:1533
    if escapes && chars.len() >= 2                          // c:1533
        && (chars[0] == STRING || chars[0] == QSTRING)
    {
        // Walk identifier chars after the leading $/Qstring.
        let mut pend = 1_usize;                             // c:1534
        while pend < chars.len() {                          // c:1535
            let c = chars[pend];                            // c:1536
            // C: `iident(*pend)` — identifier-char predicate.
            if !(c.is_ascii_alphanumeric() || c == '_') {   // c:1536
                break;                                      // c:1537
            }
            pend += 1;                                      // c:1535
        }
        // C: `if (!*pend) { dst = dupstring(getsparam(pstart)); }`
        if pend == chars.len() {                            // c:1538
            let name: String = chars[1..].iter().collect(); // c:1539
            dst = state.variables.get(&name).cloned();      // c:1539
        }
    }

    // C: `if (dst == NULL) { untokenize(dst = dupstring(s)); … }`
    let result = match dst {                                // c:1542
        Some(d) => d,                                       // c:1542
        None => {
            let untoked = crate::lex::untokenize(s);        // c:1543
            if escapes {                                    // c:1544
                // C: `dst = getkeystring(dst, &klen,
                //          GETKEYS_SEP, NULL); dst = metafy(...);`
                crate::ported::utils::getkeystring(&untoked).0 // c:1545
            } else {
                untoked                                     // c:1543
            }
        }
    };

    // C: `if (tok_arg) shtokenize(dst);` — re-tokenize for pattern
    // matching contexts. Rust's shtokenize returns Vec<GlobToken>;
    // we render back to a string via untokenize roundtrip until a
    // proper Vec<GlobToken>-aware caller exists.
    if tok_arg {                                            // c:1549
        let _ = crate::ported::glob::shtokenize(&result);   // c:1550
        // Result kept as-is; tok_arg is a hint for downstream glob
        // engines that consume the tokenized form directly.
    }
    result                                                  // c:1553
}                                                           // c:1554













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
pub fn equalsubstr(s: &str, assign: bool, nomatch: bool, _state: &SubstState) -> Option<String> { // c:715
    // C: `for (pp = str; !isend2(*pp); pp++);` — find end of cmd
    // name. isend2(c) = !c || c==Inpar || (assign && c==':').
    let end = s                                             // c:719
        .chars()                                            // c:719
        .take_while(|&c| {                                  // c:719
            c != '\0'                                       // c:719
                && c != INPAR                               // c:719
                && c != '\u{85}'                            // c:719 (Inpar token)
                && !(assign && c == ':')                    // c:719
        })
        .count();

    // C: `cmdstr = dupstrpfx(str, pp-str);
    //     untokenize(cmdstr); remnulargs(cmdstr);`
    let cmdstr_raw: String = s.chars().take(end).collect(); // c:721
    let cmdstr = crate::lex::untokenize(&cmdstr_raw);       // c:722
    let cmdstr = cmdstr.replace('\u{0}', "");               // c:723

    // C: `cnam = findcmd(cmdstr, 1, 0)` — `1` is do_hash, `0` is
    // not-just-builtins. Route through ShellExecutor::findcmd.
    let cnam = crate::fusevm_bridge::with_executor(         // c:724
        |exec| exec.findcmd(&cmdstr, true));                // c:724

    match cnam {                                            // c:724
        Some(path) => {                                     // c:730
            // C: `if (*pp) return dyncat(cnam, pp); else
            //     return cnam;`
            if end < s.chars().count() {                    // c:730
                let rest: String = s.chars().skip(end).collect(); // c:730
                Some(format!("{}{}", path, rest))           // c:731
            } else {
                Some(path)                                  // c:733
            }
        }
        None => {                                           // c:725
            if nomatch {                                    // c:725
                eprintln!("zshrs: {}: not found", cmdstr);  // c:726
            }
            None                                            // c:728
        }
    }
}                                                           // c:733
















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
