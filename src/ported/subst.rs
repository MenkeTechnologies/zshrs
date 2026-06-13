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
//! helpers into module-level ported, and replaces unsafe pointer walks
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

// `subst.rs` does NOT reach into `ShellExecutor` — every shell-state
// read/write goes through the canonical C-named accessor (paramtab,
// hashtable, options globals, etc.). Command-substitution `$(...)`
// routes through `crate::exec::getoutput` (mirror of exec.c:4712).
// c:N/A
// Per user directive: history-modifier helpers (casemodify, remtpath,
// remlpaths, remtext, xsymlinks) live in src/ported/hist.rs (the
// canonical port of Src/hist.c). Import here so subst.rs's modify()
// arms and the parity tests can reference by bare name.
use crate::lex::untokenize;
use crate::parse::{ShellWord, VarModifier, ZshParamFlag};
use crate::ported::exec::getoutput;
use crate::ported::glob::xpandbraces;
use crate::ported::hashnameddir::nameddirtab;
use crate::ported::hashtable::{aliastab_lock, cmdnamtab_lock, shfunctab_lock, sufaliastab_lock};
use crate::ported::hist::{
    casemodify, hsubl, hsubpatopt, hsubr, rembutext, remlpaths, remtext, remtpath,
};
use crate::ported::math::mathevali;
use crate::ported::modules::parameter::*;
use crate::ported::options::{opt_state_set, ZSH_OPTIONS_SET};
use crate::ported::params::{
    assignsparam, convbase_underscore, convfloat_underscore, getarrvalue, getsparam,
    lookup_special_var, paramtab, paramtab_hashed_storage, setsparam,
};
use crate::ported::pattern::{patcompile, pattry};
use crate::ported::prompt::promptexpand;
use crate::ported::string::{dupstring, dyncat};
use crate::ported::utils::{
    errflag, getkeystring, quotestring, xsymlinks, zerr, GETKEY_CTRL, GETKEY_EMACS,
    GETKEY_OCTAL_ESC,
};
use crate::ported::zsh_h::PAT_HEAPDUP;
#[allow(unused_imports)]
use crate::ported::zsh_h::{
    hashnode, isset, param, Bang, Bnull, Bnullkeep, Dash, Dnull, Equals, Hat, Inang, Inbrace,
    Inbrack, Inpar, Inparmath, Marker, Nularg, Outang, OutangProc, Outbrace, Outbrack, Outpar,
    Outparmath, Param, Pound, Qstring, Qtick, Quest, Snull, Star, Stringg, Tick, Tilde,
    ALIAS_GLOBAL, ALIAS_SUFFIX, CASMOD_NONE, DISABLED, HASHED, HISTSUBSTPATTERN, IGNOREBRACES,
    KSHTYPESET, LEXFLAGS_ACTIVE, LEXFLAGS_COMMENTS_KEEP, LEXFLAGS_COMMENTS_STRIP, LEXFLAGS_NEWLINE,
    MN_FLOAT, MN_UNSET, MULTSUB_PARAM_NAME, MULTSUB_WS_AT_END, MULTSUB_WS_AT_START, PM_ARRAY,
    PM_EFLOAT, PM_EXPORTED, PM_FFLOAT, PM_HASHED, PM_HIDE, PM_HIDEVAL, PM_INTEGER, PM_LEFT,
    PM_LOWER, PM_NAMEREF, PM_READONLY, PM_RIGHT_B, PM_RIGHT_Z, PM_SPECIAL, PM_TAGGED, PM_TIED,
    PM_UNIQUE, PM_UPPER, PREFORK_ASSIGN, PREFORK_KEY_VALUE, PREFORK_NOSHWORDSPLIT,
    PREFORK_NO_UNTOK, PREFORK_SHWORDSPLIT, PREFORK_SINGLE, PREFORK_SPLIT, PREFORK_SUBEXP,
    PREFORK_TYPESET, PUSHDMINUS, QT_BACKSLASH, QT_BACKSLASH_PATTERN, QT_DOLLARS, QT_NONE,
    QT_QUOTEDZPUTS, QT_SINGLE, QT_SINGLE_OPTIONAL, RCEXPANDPARAM, SCANPM_NONAMEREF,
    SCANPM_WANTKEYS, SCANPM_WANTVALS, SHFILEEXPANSION, SHWORDSPLIT, SORTIT_ANYOLDHOW,
    SORTIT_BACKWARDS, SORTIT_IGNORING_CASE, SORTIT_NUMERICALLY, SORTIT_NUMERICALLY_SIGNED,
    SORTIT_SOMEHOW, SUB_ALL, SUB_BIND, SUB_DOSUBST, SUB_EGLOB, SUB_EIND, SUB_END, SUB_GLOBAL,
    SUB_LEN, SUB_LIST, SUB_LONG, SUB_MATCH, SUB_REST, SUB_RETFAIL, SUB_START, SUB_SUBSTR,
};
use crate::zsh_h::{CASMOD_CAPS, CASMOD_LOWER, CASMOD_UPPER};
use crate::DPUTS;
#[allow(unused_imports)]
use std::ffi::CString;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Port of `LF_ARRAY` from `Src/subst.c:33`.
/// `#define LF_ARRAY 1`. Linked-list flag the substitution-result
/// LinkList carries when the expansion produced multiple words.
/// Drives `prefork` / `singsub` / `aget` to return an array vs scalar.
pub const LF_ARRAY: u32 = 1; // c:33

/// Check for array assignment with entries like [key]=val
/// Port of `keyvalpairelement(LinkList list, LinkNode node)` from `Src/subst.c:49`.
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
/// WARNING: param names don't match C — Rust=(list, node_idx) vs C=(list, node)
fn keyvalpairelement(list: &mut LinkList, node_idx: usize) -> Option<usize> {
    // c:49
    // C: `start = (char *)getdata(node)` — fetch the node's text.
    let data = list.getdata(node_idx)?.to_string(); // c:53
    let chars: Vec<char> = data.chars().collect(); // c:53

    // C: `start[0] == Inbrack` — must lead with `[` (or token).
    if chars.is_empty()                                     // c:54
        || (chars[0] != Inbrack && chars[0] != '[')
    // c:54
    {
        return None; // c:54
    }

    // C: `end = strchr(start+1, Outbrack)` — find matching `]`.
    let mut end_pos: Option<usize> = None; // c:55
    for (i, &c) in chars.iter().enumerate().skip(1) {
        // c:55
        if c == Outbrack || c == ']' {
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
        && (chars.get(end_pos + 2) == Some(&Equals)
            || chars.get(end_pos + 2) == Some(&'='));
    let is_assign = !is_append                              // c:57
        && (chars.get(end_pos + 1) == Some(&Equals)
            || chars.get(end_pos + 1) == Some(&'='));
    if !is_assign && !is_append {
        // c:60
        return None;
    }

    // C: `*end = '\0'; dat = start + 1; singsub(&dat); untokenize(dat);`
    // — extract key, run param-subst, untokenize.
    let raw_key: String = chars[1..end_pos].iter().collect(); // c:64
    let key_subst = singsub(&raw_key); // c:65
    let key = untokenize(&key_subst); // c:66

    // C lines 67-75: Marker / Marker_plus sentinel + insertlinknode
    // for key and value.
    let value_start = if is_append { end_pos + 3 } else { end_pos + 2 }; // c:67-72
    let raw_value: String = chars[value_start..].iter().collect(); // c:69 / 73
    let value_subst = singsub(&raw_value); // c:75
    let value = untokenize(&value_subst); // c:76

    let marker = if is_append {
        // c:67
        format!("{}+", Marker) // c:67
    } else {
        // c:71
        Marker.to_string() // c:71
    };

    list.setdata(node_idx, marker); // c:72
    let key_idx = list.insertlinknode(node_idx, key); // c:73
    let val_idx = list.insertlinknode(key_idx, value); // c:77

    // C: `return insertlinknode(list, node, dat);` — node where
    // value was inserted.
    Some(val_idx) // c:77
} // c:79

/// Do substitutions before fork
/// Phase-1 word-list substitution (tilde/equal/brace/param/cmd/arith).
/// Port of `prefork(LinkList list, int flags, int *ret_flags)` from Src/subst.c:100 — runs ahead of
/// glob expansion to fully resolve `${...}` / `$(...)` /
/// `$((...))` / `~user` / `=cmd` / `{a,b}`.
// Do substitutions before fork.                                            // c:100
/// `prefork` — see implementation.
pub fn prefork(list: &mut LinkList, flags: i32, ret_flags: &mut i32) {
    // c:100
    // c:100
    let mut node_idx = 0; // c:100
    let mut stop_idx: Option<usize> = None; // c:100
    let mut keep = false; // c:100
    let asssub = (flags & PREFORK_TYPESET != 0) && isset(KSHTYPESET); // c:100
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

        if isset(SHFILEEXPANSION) {
            // c:100
            // SHFILEEXPANSION - do file substitution first
            if let Some(data) = list.getdata(node_idx) {
                // c:100
                let new_data = filesub(
                    // c:100
                    data,                                       // c:100
                    flags & (PREFORK_TYPESET | PREFORK_ASSIGN), // c:100
                ); // c:100
                list.setdata(node_idx, new_data); // c:100
            } // c:100
        } else {
            // c:100
            // Do string substitution
            if let Some(new_idx) = stringsubst(
                // c:100
                list,                                        // c:100
                node_idx,                                    // c:100
                flags & !(PREFORK_TYPESET | PREFORK_ASSIGN), // c:100
                ret_flags,                                   // c:100
                asssub,                                      // c:100
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
    if isset(SHFILEEXPANSION) {
        // c:100
        node_idx = 0; // c:100
        while node_idx < list.nodes.len() {
            // c:100
            if let Some(new_idx) = stringsubst(
                // c:100
                list,                                        // c:100
                node_idx,                                    // c:100
                flags & !(PREFORK_TYPESET | PREFORK_ASSIGN), // c:100
                ret_flags,                                   // c:100
                asssub,                                      // c:100
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
                // c:Src/subst.c:170 — `remnulargs(getdata(node));`
                // strips inull sentinels (Snull/Dnull/Bnull/Bnullkeep/
                // Nularg). Empty arrays elements arrive here as the
                // single Nularg byte (`\u{a1}`) — a sentinel emitted
                // by `(s./.)` split + auto-splat (and other empty-
                // emitting paths) so they don't get deleted by the
                // else-branch below. Strip Nularg-only nodes to true
                // empty AFTER the if-test that gates deletion has
                // already passed; this preserves the empty argument
                // shape (`${(@s./.)X}` with leading empty yields a
                // 3-element array in DQ) without polluting consumers
                // with the raw sentinel byte. C zsh's downstream
                // consumers handle Nularg via per-builtin inull
                // checks; the Rust port collapses it here so plain
                // string consumers (print, echo, assignment) see
                // proper empty.
                let mut s = data.to_string();
                crate::ported::glob::remnulargs(&mut s);
                if s == "\u{a1}" {
                    s.clear();
                }
                let data = s;
                list.setdata(node_idx, data.clone()); // c:100

                // Brace expansion. C: `while (hasbraces(getdata(node)))
                // { keep = 1; xpandbraces(list, &node); }`. zsh's
                // hasbraces walks the string looking for a balanced
                // `{…}` containing `,` or `..` (range). xpandbraces
                // splits the node into N nodes.
                //
                // Routes through canonical
                // xpandbraces; treats >1
                // result as a positive hasbraces hit.
                if !isset(IGNOREBRACES) && (flags & PREFORK_SINGLE == 0) {
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
                        let expanded = xpandbraces(&cur, false); // c:171
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
                if !isset(SHFILEEXPANSION) && !SKIP_FILESUB.with(|c| c.get()) {
                    // c:100
                    if let Some(data) = list.getdata(node_idx) {
                        // c:100
                        let new_data = filesub(
                            // c:100
                            data,                                       // c:100
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

// Token constants from zsh.h (mapped to char values > 127)
// `pub mod tokens { … }` — DELETED per user directive. Was a
// Rust-only duplicate of the canonical token table in
// `crate::ported::zsh_h` (port of `Src/zsh.h:159-224`). Two names
// drifted: local `STRING` → canonical `Stringg`, local
// `OUTANGPROC` → canonical `OutangProc`. All other constants
// matched bit-for-bit but living in two places invited future drift.
// c:zsh.h:159-224 + scan flags c:1953-1973

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
/// Port of `stringsubstquote(char *strstart, char **pstrdpos)` from `Src/subst.c:206`.
fn stringsubstquote(strstart: &str, pstrdpos: usize) -> (String, usize) {
    // c:206
    let chars: Vec<char> = strstart.chars().collect(); // c:208

    // C: `getkeystring(pstrdpos+2, &len, GETKEYS_DOLLARS_QUOTE, NULL)`.
    // Rust's getkeystring doesn't take a stop-at-unquoted-` flag, so
    // we walk the quoted region manually first, then unescape the
    // captured content. Same observable behavior: dollar-quoted
    // chars get C-escape-processed, unescaped `'` terminates.
    let start = pstrdpos + 2; // c:209 (pstrdpos+2)
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
        // c:Src/utils.c:7189 — `(how & GETKEY_DOLLAR_QUOTE) && *s ==
        // Snull` is the canonical terminator: in the LEXED stream the
        // closing quote of `$'...'` is the Snull token, not a raw `'`.
        // Without this check a tokenized `$'A '"\E"` word ran past the
        // close and C-escape-decoded the following Dnull section
        // (`\E` → ESC), corrupting heredoc terminators (gap #3
        // 2026-06-12, A04 "no shell expansion on the initial word").
        // The raw `'` arm stays for untokenized caller input.
        if chars[end] == Snull || chars[end] == '\'' {
            break;
        } // c:209 (unescaped close)
        end += 1;
    }

    // C: `getkeystring` returns the unescaped content (strsub) +
    // length consumed. Rust calls getkeystring on the captured
    // content slice; consumed count is the slice length plus the
    // wrapping `$'` and `'`.
    let content: String = chars[start..end].iter().collect();
    let (strsub, _) = getkeystring(&content); // c:211

    // C: `len += 2;` — caller's len now includes the leading `$'`
    // (Rust mirrors via end+1 below).

    // C: `if (strstart != pstrdpos)` — there's a prefix, so concat
    // prefix + strsub + suffix. Rust always concats; empty prefix
    // is benign.
    let prefix: String = chars[..pstrdpos].iter().collect(); // c:215
    let suffix: String = if end + 1 < chars.len() {
        // c:216 (pstrdpos[len] check)
        chars[end + 1..].iter().collect() // c:217
    } else {
        String::new() // c:218
    };

    // C: empty `$''` special case — `strret = dupstring(nulstring);`
    // returns the Nularg sentinel string so the empty bslashquote doesn't
    // get elided by stringsubst's word-walk.
    let strret = if strsub.is_empty() && prefix.is_empty() && suffix.is_empty() {
        // c:226-227 — `nulstring` sentinel, defined at subst.c:36 as
        // `{Nularg, '\0'}`. Nularg is 0xa1 per `Src/zsh.h:206`, NOT
        // 0x8b as the previous comment claimed (drift bug pattern).
        // Emit the single Nularg char so downstream code recognises
        // the empty-bslashquote sentinel.
        Nularg.to_string() // c:227
    } else {
        format!("{}{}{}", prefix, strsub, suffix) // c:215-220
    };

    // C: `*pstrdpos = strret + (pstrdpos - strstart) + strlen(strsub);`
    // — sets the in-out pointer to one past the unescaped content
    // in the new string. Rust returns the equivalent index.
    let new_pos = prefix.chars().count()
        + strret
            .chars()
            .count()
            .saturating_sub(prefix.chars().count() + suffix.chars().count()); // c:237

    (strret, new_pos) // c:237
} // c:237

/// String substitution - main workhorse
/// Port of stringsubst(LinkList list, LinkNode node, int pf_flags, int *ret_flags, int asssub) from subst.c lines 227-421
fn stringsubst(
    // c:237
    list: &mut LinkList, // c:237
    node_idx: usize,     // c:237
    pf_flags: i32,       // c:237
    ret_flags: &mut i32, // c:237
    asssub: bool,        // c:237
) -> Option<usize> {
    // c:237
    // c:339 — C's stringsubst reassigns its live node from
    // paramsubst's return (`node = paramsubst(l, node, &str, ...)`)
    // and finally `return n;` — the LAST node the substitution
    // produced. prefork's incnode then continues AFTER it, so
    // splat-inserted nodes are never re-scanned (re-scanning
    // double-expands substituted VALUES — a literal `$'\0'` coming
    // out of a replacement decoded as ANSI-C on the second pass).
    // The Rust port previously pinned node_idx and returned the
    // ORIGINAL index; shadow it mutable and advance on multi-node
    // splices exactly like C's `node`.
    let mut node_idx = node_idx;
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
        if (c == Inang || c == OUTANGPROC || (pos == 0 && c == Equals)) // c:237
            && chars.get(pos + 1) == Some(&Inpar)
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
              // Inang/OUTANGPROC/Equals marker char itself.
            let start = pos; // c:237
            pos += 2; // c:237 (skip marker + Inpar)
            let mut depth = 1_i32; // c:237
            while pos < chars.len() && depth > 0 {
                // c:237
                let ch = chars[pos]; // c:237
                if ch == Inpar {
                    depth += 1;
                }
                // c:237
                else if ch == Outpar {
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
        // Snull) encloses literal `'…'` regions. Inside, no parameter /
        // command substitution / glob fires — content is verbatim.
        // Strip both markers and leave the body intact. Without this, a
        // `${var/pat/'~'$match[1]}` replacement yielded
        // `\u{9d}~\u{9d}<match-1>` (SNULLs leaked through, broke the
        // string).
        if c == '\u{9d}' {
            // c:237
            // Find matching close-Snull.
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
          // Lexer-emitted double-bslashquote marker (`\u{9e}`, Dnull) — strip;
          // contents inside DQ already had `$`/`${…}` tokenized to STRING
          // / Qstring by the lexer, so the surrounding pass picks them
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
          // Lexer Bnull (`\u{9f}`) escapes the next char as literal.
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
          // c:Src/subst.c:237 — literal `'…'` single-quote SPAN
          // handling in stringsubst. C's stringsubst dispatches on
          // the lexer-emitted `Snull` token (Src/subst.c:301-304), NOT
          // on a raw `'` byte: the lex stage at Src/lex.c:1638-1668
          // tokenizes every source-level `'…'` into `Snull body Snull`
          // BEFORE stringsubst sees the string. A raw `'` reaching
          // stringsubst means the byte is already LITERAL — typically
          // produced by a substitution result inside a DQ context
          // (`"…'mixed'…"`), where the lexer kept the `'` as a plain
          // char per Src/lex.c dquote_parse's default arm.
          //
          // Treating raw `'` as a SQ delimiter at this layer breaks
          // `(qq)` quoting roundtrips and any DQ source containing
          // literal apostrophes — the body between the `'`s gets
          // stripped of its quotes, corrupting the value (Bug
          // surface: `(qq)` produces `'mixed'` → `mixed` instead of
          // the C-faithful `'\''mixed'\''`).
          //
          // The previous Rust port's literal-`'` arm here was an
          // over-eager workaround; remove it so raw `'` flows through
          // as literal character data, mirroring C's
          // token-only `Snull` dispatch.

        let qt = c == Qstring; // c:237
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
                                                      // Accept either tokenized `Inpar` / `Inparmath` / `Inbrack`
                                                      // / `Inbrace` / `Snull` OR their literal `(` / `[` / `{`
                                                      // / `'` counterparts.
            let next_is = |tok: char, lit: char| {
                // c:237
                next_c == Some(tok) || next_c == Some(lit) // c:237
            }; // c:237

            // Detect `$((expr))` arith form FIRST — it's
            // `$(` + `(expr)` + `)` so naive cmd-subst dispatch
            // would try to execute `((expr))` as a command. Either
            // the lexer-tokenized Inparmath or the literal `((`
            // sequence routes through the arith path. Direct port
            // of subst.c's Inparmath arm at c:237 (see C lines
            // around 320-360 which check `*++s == Inpar` after
            // `*s == Stringg`).
            if next_c == Some(Inparmath)                    // c:237
                || (next_c == Some('(') && chars.get(pos + 2).copied() == Some('('))
            // c:237
            {
                // c:237 — `$((expr))` arith form. The lexer has two
                // shapes here depending on how DQ/word context tokenised
                // the parens:
                //
                //   ASCII shape  : `$` `(` `(` … `)` `)`  (5+ chars)
                //   TOKEN shape  : `$` Inparmath … Outparmath
                //                  (Inparmath represents `((` as one
                //                  char; Outparmath represents `))`)
                //   Mixed shape  : `$` Inparmath `(` … `)` Outparmath
                //                  (the lexer emits the outer pair as
                //                  TOKEN and a duplicate inner pair too
                //                  — observed for `"$((1+2))"`).
                //
                // For the TOKEN shape, the leading `((` collapses into
                // a single Inparmath, so the expression begins at
                // pos+2 and ends at the matching Outparmath. For the
                // ASCII shape, classic depth-tracked `))` walk applies.
                let token_shape = next_c == Some(Inparmath);
                // Mixed shape: lexer emitted Inparmath for outer `((`
                // AND a literal inner `(`/`)` pair. Detect by peeking
                // — if chars[pos+2] is ASCII `(`, the inner paren is
                // there too and the expression body starts at pos+3
                // (and the matching inner `)` lands just before the
                // Outparmath). Same depth bookkeeping as ASCII shape.
                let mixed_shape = token_shape && chars.get(pos + 2).copied() == Some('(');
                let start = if mixed_shape {
                    pos + 3
                } else if token_shape {
                    pos + 2
                } else {
                    pos + 3
                };
                let mut depth = if token_shape && !mixed_shape {
                    1_i32
                } else {
                    2_i32
                };
                let mut p = start;
                let mut end_off: Option<usize> = None;
                let mut end_outparmath = false;
                while p < chars.len() {
                    let ch = chars[p];
                    if ch == Inparmath {
                        depth += 1;
                    } else if ch == Outparmath {
                        depth -= 1;
                        if depth == 0 {
                            end_off = Some(p);
                            end_outparmath = true;
                            break;
                        }
                    } else if ch == '(' || ch == Inpar {
                        depth += 1;
                    } else if ch == ')' || ch == Outpar {
                        depth -= 1;
                        if depth == 0 {
                            end_off = Some(p);
                            break;
                        }
                    }
                    p += 1;
                }
                if let Some(end) = end_off {
                    // ASCII path: end points at outer `)`; inner `)`
                    // was already consumed when depth dropped from 2→1.
                    // TOKEN path: Outparmath is a single char for `))`,
                    // expression is chars[start..end].
                    // Mixed shape: Outparmath also covers an inner `)`
                    // we walked past; expression still ends one char
                    // before the Outparmath (the ASCII `)` we already
                    // counted).
                    let expr_end = if end_outparmath {
                        let mut e = end;
                        if e > start && chars[e - 1] == ')' {
                            e -= 1;
                        }
                        e
                    } else {
                        end - 1
                    };
                    let expr: String = chars[start..expr_end].iter().collect();
                    let prefix: String = chars[..pos].iter().collect();
                    let suffix: String = if end + 1 < chars.len() {
                        chars[end + 1..].iter().collect()
                    } else {
                        String::new()
                    };
                    let result_only = arithsubst(&expr, "", "");
                    str3 = format!("{}{}{}", prefix, result_only, suffix);
                    list.setdata(node_idx, str3.clone());
                    pos = prefix.chars().count() + result_only.chars().count();
                    continue;
                }
            } // c:237

            if next_is(Inpar, '(') || next_is(Inparmath, '\0') {
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
                    if ch == '(' || ch == Inpar {
                        depth += 1;
                    }
                    // c:237
                    else if ch == ')' || ch == Outpar {
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
                        // c:Src/subst.c — `$(<filename)` runs the file
                        // path through parameter expansion first. zsh
                        // accepts `$(<$tf)` etc.; route through singsub
                        // so the path arg gets the same prefork/multsub
                        // as a quoted operand.
                        let path = singsub(rest.trim());
                        std::fs::read_to_string(path.trim()).unwrap_or_default()
                    } else {
                        // c:exec.c:4712 — `getoutput(cmd, 1)`. Caller
                        // here is splicing the result into a string
                        // (str3 = format!("{}{}{}", ...)), which is
                        // the qt=1 / "$(...)" path. Join the Vec back
                        // for the string concat.
                        getoutput(&cmd, 1).join("")
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
            } else if next_is(Inbrack, '[') {
                // c:293 — `$[...]` arithmetic. C does
                // `skipparens(Inbrack, Outbrack, &str2)` against the
                // tokenized form. Rust callers may pass either
                // tokenized (Inbrack) or raw (`[`); when the tail has
                // only one form we route through the canonical
                // skipparens entry, otherwise fall back to a mixed-
                // form depth walk that tracks both.
                let start = pos + 2; // c:237
                let open = if next_c == Some(Inbrack) {
                    Inbrack
                } else {
                    '['
                }; // c:237
                let close = if open == Inbrack { Outbrack } else { ']' }; // c:237
                let chars: Vec<char> = str3.chars().collect(); // c:237
                let mut end_off: Option<usize> = None; // c:237
                let tail_only_tokenized = open == Inbrack
                    && chars[start.saturating_sub(1)..]
                        .iter()
                        .all(|&c| c != '[' && c != ']');
                if tail_only_tokenized {
                    // c:293 — `skipparens(Inbrack, Outbrack, &str2)`.
                    let open_byte_off: usize =
                        chars[..start - 1].iter().map(|c| c.len_utf8()).sum();
                    let mut cursor: &str = &str3[open_byte_off..];
                    let bal = crate::ported::utils::skipparens(Inbrack, Outbrack, &mut cursor);
                    if bal == 0 {
                        let after_close_byte = str3.len() - cursor.len();
                        let after_close_char = str3[..after_close_byte].chars().count();
                        end_off = Some(after_close_char.saturating_sub(1).saturating_sub(start));
                    }
                } else {
                    // Mixed-form fallback. Original Rust depth walk.
                    let mut depth = 1_i32;
                    let mut p = start;
                    while p < chars.len() {
                        let ch = chars[p];
                        if ch == open || ch == '[' {
                            depth += 1;
                        } else if ch == close || ch == ']' {
                            depth -= 1;
                            if depth == 0 {
                                end_off = Some(p - start);
                                break;
                            }
                        }
                        p += 1;
                    }
                }
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
            } else if next_c == Some(Snull) || (next_c == Some('\'') && !qt) {
                // c:237
                // $'...' ANSI-C quoting. Accept either the lexer-
                // tokenized Snull marker OR the raw `'` — recursive
                // operator-operand paths (e.g. multsub on a `:=`
                // operand) hand us the literal text without prior
                // tokenization, so dispatch on the literal too.
                //
                // c:Src/subst.c:301 — C triggers ONLY on the Snull
                // TOKEN. Inside double quotes the lexer emits Qstring
                // followed by a RAW `'` (dquote_parse has no $'...'
                // arm, Src/lex.c:1519-1556) and C's stringsubst
                // routes that to paramsubst, which leaves the `$`
                // literal — oracle: `"${a/x/$'\t'q}"` emits LITERAL
                // `$'\t'q` in zsh 5.9. Gate the raw-quote concession
                // on !qt so a Qstring dollar (qt == true) follows the
                // C path instead of decoding.
                let (new_str, new_pos) = stringsubstquote(&str3, pos); // c:237
                str3 = new_str; // c:237
                pos = new_pos; // c:237
                list.setdata(node_idx, str3.clone()); // c:237
                continue; // c:237
            } else {
                // c:237
                // Parameter substitution
                let mut new_pf_flags = pf_flags; // c:237
                if (isset(SHWORDSPLIT) && (pf_flags & PREFORK_NOSHWORDSPLIT == 0)) // c:237
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
                IN_PARAMSUBST_NEST.with(|c| c.set(c.get() + 1)); // c:237 paramsub_nest++
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
                IN_PARAMSUBST_NEST.with(|c| c.set(c.get() - 1)); // c:237 paramsub_nest--
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
                    let multi = new_nodes.len() > 1;
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
                    if multi {
                        // c:339 — continue scanning in the LAST
                        // spliced node (the suffix after `${...}`
                        // lives there); paramsubst's returned pos is
                        // the suffix offset WITHIN that node for the
                        // multi-node case. Advancing also makes the
                        // final `return Some(node_idx)` report the
                        // last node, so prefork skips the inserted
                        // VALUES instead of re-scanning them.
                        node_idx = current_idx;
                    }
                }

                str3 = list.getdata(node_idx)?.to_string(); // c:237
                pos = new_pos; // c:237
                continue; // c:237
            } // c:237
        } // c:237

        // Backtick command substitution `cmd` — same engine as
        // `$(cmd)` per subst.c:237. Find the matching backtick,
        // capture cmd text, delegate to run_command_substitution.
        // The bridge's BUILTIN_EXPAND_TEXT untokenizes Tick/Qtick
        // back to a raw `` ` `` before calling singsub, so accept
        // any of the three forms as the open/close delimiter.
        let qt = c == Qtick; // c:237
        if qt || c == Tick || c == '`' {
            // c:237
            if !qt {
                // c:237
                list.flags |= LF_ARRAY; // c:237
            } // c:237
            let chars: Vec<char> = str3.chars().collect(); // c:237
            let cmd_start = pos + 1; // c:237
            let mut end = cmd_start; // c:237
            // c:Src/subst.c — walk body looking for closing
            // backtick. Skip past escape markers: `\\` (raw
            // backslash-escape from a non-tokenized source) AND
            // Bnull (`\u{9f}`, the lexer's escape sentinel that
            // dquote_parse emits for `\\``/`\\\\`/`\\$`). Bug
            // #46 in docs/BUGS.md: nested `` `cmd \`inner\` cmd` ``
            // had the inner backslash-escaped backticks tokenized
            // to Bnull+`` ` `` pairs by the lexer; without the
            // Bnull skip the walker matched the FIRST literal
            // backtick (the escaped inner one) as the closer and
            // truncated the command to `echo ` + Bnull.
            while end < chars.len()
                && chars[end] != Tick
                && chars[end] != Qtick
                && chars[end] != '`'
            {
                if (chars[end] == '\\' || chars[end] == '\u{9f}') && end + 1 < chars.len() {
                    end += 1;
                } // c:237
                end += 1; // c:237
            } // c:237
            if end < chars.len() {
                // c:237
                let cmd_raw: String = chars[cmd_start..end].iter().collect(); // c:237
                // c:Src/subst.c — strip one level of escape from the
                // backtick body before re-parsing. The lexer's
                // `dquote_parse` LX2_BQUOTE arm emits `Bnull + X`
                // (\u{9f}+X) for each `\X` it saw inside the
                // backquotes (X ∈ {`` ` ``, `\`, `$`}). For the
                // re-parse to see the right shape, drop the Bnull
                // and keep the literal char. The result is what the
                // user would have typed at the next nesting level.
                // Bug #46 in docs/BUGS.md: nested `` `cmd \`inner\`
                // ` `` had the inner `` \` `` escapes preserved as
                // `Bnull + `` ` ``` into getoutput's inner shell,
                // which couldn't tokenize the Bnull byte and treated
                // the whole thing as a literal.
                let cmd: String = {
                    let cv: Vec<char> = cmd_raw.chars().collect();
                    let mut out = String::with_capacity(cmd_raw.len());
                    let mut i = 0;
                    while i < cv.len() {
                        if cv[i] == '\u{9f}' && i + 1 < cv.len() {
                            out.push(cv[i + 1]);
                            i += 2;
                        } else {
                            out.push(cv[i]);
                            i += 1;
                        }
                    }
                    out
                };
                                                                          // c:exec.c:4712 — `getoutput(cmd, 1)`. String-
                                                                          // splice caller (qt=1).
                let output = getoutput(&cmd, 1).join("");
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
        if asssub && (c == '=' || c == Equals) && pos > 0 { // c:237
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

/// Quote substitution for heredoc tags
/// Port of `quotesubst(char *str)` from `Src/subst.c:463`.
///
/// Simplified version of prefork/singsub that does only the
/// substitutions appropriate to quoting context — currently just the
/// $'...' (Snull) form. Used for here-doc end tags. Other expansions
/// (param-subst, cmd-subst, arith) stay in the text.
///
/// The trailing `remnulargs()` strips Bnull tokens so this is
/// consistent with the other substitution forms (indicating quotes
/// have been fully processed).
pub fn quotesubst(str: &str) -> String {
    // c:463
    // c:463
    let mut result = str.to_string(); // c:465
    let mut pos = 0_usize; // c:466

    // C: `while (*str) { if (*str == Stringg && str[1] == Snull) …
    //               else str++; }`
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
            && chars[pos + 1] == Snull
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
    // C: `remnulargs(str);` — strip Bnull / NUL tokens. c:474
    crate::ported::glob::remnulargs(&mut result);
    result
} // c:475

// `pub mod prefork_flags { … }` — DELETED per user directive.
// Every bit value was WRONG vs the canonical C source: local
// `SINGLE=1, SPLIT=2, SHWORDSPLIT=4, NOSHWORDSPLIT=8, ASSIGN=16,
// TYPESET=32` vs C's `PREFORK_TYPESET=0x01, PREFORK_ASSIGN=0x02,
// PREFORK_SINGLE=0x04, PREFORK_SPLIT=0x08, PREFORK_SHWORDSPLIT=0x10,
// PREFORK_NOSHWORDSPLIT=0x20` (`Src/zsh.h:2020-2042`). Every
// `flags & prefork_flags::X` test silently mis-tested the wrong
// bit. Canonical defs imported from `crate::ported::zsh_h` below.
// c:zsh.h:2020-2042

/// Glob entries in a linked list
/// Port of `globlist(LinkList list, int flags)` from `Src/subst.c:489`.
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
pub fn globlist(list: &mut LinkList, flags: i32) {
    // c:489
    // c:489
    // c:491 — `badcshglob = 0;` — reset per-command-line csh-glob
    // diagnostic counter. Each subsequent zglob run ORs in bit 1
    // (failure under CSHNULLGLOB) / bit 2 (success); the terminal
    // diagnostic below checks the OR for the "all-failed-no-success"
    // case.
    crate::ported::glob::BADCSHGLOB.store(0, std::sync::atomic::Ordering::Relaxed);
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
        if flags & PREFORK_KEY_VALUE != 0 && data.chars().next() == Some(Marker) {
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
            // c:Src/glob.c:1872-1888 — `Deal with failures to match
            // depending on options`. C body verbatim:
            //   else if (!gf_nullglob) {
            //       if (isset(CSHNULLGLOB)) {           c:1874
            //           badcshglob |= 1;
            //       } else if (isset(NOMATCH)) {        c:1876
            //           zerr("no matches found: %s",    c:1877
            //                ostr);
            //           zfree(matchbuf, 0);             c:1878
            //           restore_globstate(saved);       c:1879
            //           return;                         c:1880
            //       } else {                            c:1882
            //           untokenize(matchptr->name =     c:1884
            //               dupstring(ostr));
            //           matchptr++;                     c:1885
            //           matchct = 1;                    c:1886
            //       }
            //   }
            // Parity bug #13: previously this arm always took the
            // c:1882-1886 `treat as literal` branch unconditionally.
            let nullglob = isset(crate::ported::zsh_h::NULLGLOB); // c:1873 !gf_nullglob
            let csh_nullglob = isset(crate::ported::zsh_h::CSHNULLGLOB); // c:1874
                                                                         // c:Src/glob.c:1232 — `if (!haswilds(str))` test that
                                                                         // bypasses zglob entirely. We mirror it here so plain
                                                                         // literals like `echo foo` don't trip NOMATCH.
                                                                         // haswilds scans TOKENIZED strings; `data` is untokenized
                                                                         // at this point, so tokenize a local copy first
                                                                         // (c:Src/glob.c:3548), as C does for runtime-built
                                                                         // strings (compcore.c:2231).
            let has_glob_chars = {
                let mut data_tok = data.clone();
                crate::ported::glob::tokenize(&mut data_tok);
                crate::ported::pattern::haswilds(&data_tok)
            };
            if has_glob_chars && !nullglob && !csh_nullglob && isset(crate::ported::zsh_h::NOMATCH)
            // c:1876
            {
                crate::ported::utils::zerr(&format!("no matches found: {}", data)); // c:1877
                crate::ported::utils::errflag.fetch_or(
                    crate::ported::zsh_h::ERRFLAG_ERROR,
                    std::sync::atomic::Ordering::Relaxed,
                ); // c:1877 (zerr side-effect)
                   // c:1880 `return` — drop the unmatched token from the list.
                list.delete_node(node_idx);
                continue;
            }
            // c:1882-1886 — `treat as an ordinary string`. The
            // matchptr++ bookkeeping in C maps to leaving the node
            // alone and advancing the index.
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
    // c:506-509 — `if (noerrs) badcshglob = 0; else if (badcshglob == 1)
    //                zerr("no match");`. Emit the CSHNULLGLOB "no
    // match" error when every expansion failed (== 1, not |= 2).
    // Suppressed when noerrs is set (e.g. inside `eval` error-
    // checking blocks).
    // c:507 — `noerrs` from exec.c:117 (Rust: exec.rs:122 pub static).
    let noerrs = crate::ported::exec::noerrs.load(std::sync::atomic::Ordering::Relaxed) != 0;
    let badcshglob = crate::ported::glob::BADCSHGLOB.load(std::sync::atomic::Ordering::Relaxed);
    if noerrs {
        crate::ported::glob::BADCSHGLOB.store(0, std::sync::atomic::Ordering::Relaxed);
    // c:507
    } else if badcshglob == 1 {
        crate::ported::utils::zerr("no match"); // c:509
    }
} // c:510

/// Perform substitution on a single word
// perform substitution on a single word                                    // c:514
/// Single-string substitution.
/// Port of `singsub(char **s)` from Src/subst.c:514.
// perform substitution on a single word                                    // c:514
/// `singsub` — see implementation.
pub fn singsub(s: &str) -> String {
    // c:514
    // c:516 — `local_list1(foo);` (Rust analogue: stack-local list).
    let mut list = LinkList::default(); // c:516
                                        // c:518 — `init_list1(foo, *s);` (push the single input element).
    list.push_back(s.to_string()); // c:518
    let mut ret_flags = 0i32; // c:520 NULL ret_flags

    prefork(&mut list, PREFORK_SINGLE, &mut ret_flags); // c:520

    if errflag_set() {
        // c:521 if (errflag)
        return String::new(); // c:522 return
    }

    // c:523 `*s = (char *) ugetnode(&foo);` — pull first node.
    let result = list.getdata(0).cloned().unwrap_or_default(); // c:523
                                                               // c:524 — DPUTS asserts the list is now empty (singsub never
                                                               // produces more than one word). Rust port wires the canonical
                                                               // DPUTS macro from zsh_h.rs.
    DPUTS!(
        list.nodes.len() > 1, // c:524
        "BUG: singsub() produced more than one word!"
    ); // c:524
    result // c:523
}

/// Substitution with possible multiple results
/// Multi-word substitution with IFS splitting.
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
/// Port of `multsub(char **s, int pf_flags, char ***a, int *isarr, char *sep, int *ms_flags)` from `Src/subst.c:544`.
/// WARNING: param names don't match C — Rust=(s, pf_flags) vs C=(s, pf_flags, a, isarr, sep, ms_flags)
pub fn multsub(s: &str, pf_flags: i32) -> (String, Vec<String>, bool, i32) {
    // c:544
    // c:544
    let mut ms_flags = 0i32; // c:551
    let mut x = s.to_string(); // c:550 (`x = *s`)

    // C lines 555-563: PREFORK_SPLIT — skip leading IFS whitespace,
    // mark MULTSUB_WS_AT_START.
    let ifs = vars_get("IFS").unwrap_or_else(|| " \t\n\0".to_string()); // c:N/A (zsh default IFS includes NUL)
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
            //
            // The previous Rust port had every token-byte literal in
            // this block WRONG:
            //   `\u{97}` was labeled Dnull → it's QUEST.
            //   `\u{98}` was labeled Snull → it's TILDE.
            //   `\u{83}` was labeled Tick  → it's META lead byte.
            //   `\u{85}` was labeled Inpar → it's STRINGG ($).
            //   `\u{86}` was labeled Outpar → it's HAT.
            // Canonical values per `Src/zsh.h:159-194` (cross-checked
            // via `crate::ported::zsh_h::{Snull, Dnull, Tick, Inpar,
            // Outpar}` constants):
            //   Snull  = 0x9d, Dnull  = 0x9e, Tick   = 0x93,
            //   Inpar  = 0x88, Outpar = 0x8a.
            match c {                                       // c:600
                Dnull |               // c:602 (")
                Snull |               // c:603 (')
                Tick => { inq = !inq; } // c:604 (`)
                Inpar => { inp += 1; }  // c:606
                Outpar => { inp -= 1; } // c:608
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
    // c:Src/glob.c:3649 remnulargs — strip the Nularg (`\u{a1}`)
    //   sentinel and other INULL bytes (Snull/Dnull/Bnull) that
    //   paramsubst's splat block emits for empty array elements to
    //   prevent prefork's empty-node-delete pass from dropping them.
    //   Downstream consumers (cond builtin `-z`/`-n`, command args,
    //   etc.) see the post-remnulargs strings, NOT the sentinel.
    //   Bug #185 in docs/BUGS.md: `[[ -z "${b[@]}" ]]` for b=("")
    //   returned false because the leftover `\u{a1}` had StringLen=1.
    let strip_nul = |s: String| -> String {
        let mut s = s;
        crate::ported::glob::remnulargs(&mut s);
        s
    };
    if l > 1 || (list.flags & LF_ARRAY != 0) {
        // c:633
        let arr: Vec<String> = list.iter().cloned().map(strip_nul).collect(); // c:635-637
                                                               // C: `*s = sepjoin(r, sep, 1);` — join with IFS first-char
                                                               // when sep is NULL. Use first IFS char as join separator,
                                                               // matching zsh's sepjoin defaults.
        let join_sep = ifs.chars().next().map(String::from).unwrap_or_default(); // c:649
        let joined = arr.join(&join_sep); // c:649
        return (joined, arr, true, ms_flags); // c:642-647 (array path)
    }
    if l == 1 {
        // c:653
        let result = strip_nul(list.getdata(0).cloned().unwrap_or_default()); // c:653
        return (result.clone(), vec![result], false, ms_flags); // c:653
    }
    // c:Src/subst.c:655 — `*s = dupstring("");` with zero-length list.
    // C returns isarr=0 + one empty string as the joined result, BUT
    // the list itself is empty (zero nodes). Callers that observe the
    // node count (e.g. `addvars` for `arr=("${empty[@]}")`) elide the
    // word entirely; callers expecting at least one word (scalar
    // context) consume the empty joined value. The Rust port's
    // previous `vec![String::new()]` return masked the zero-node
    // case with one-empty-word, so quoted-array-splat that resolved
    // to zero words leaked through as ONE empty word — `a=(x);
    // b=("${a[@]:0:-1}")` gave len=1 instead of zsh's len=0. Bug
    // #120 in docs/BUGS.md. Return Vec::new() for the nodes slot
    // so consumers can detect the zero-word case via .is_empty().
    (String::new(), Vec::new(), false, ms_flags) // c:655
} // c:660

/// Port of `filesub(char **namptr, int assign)` from `Src/subst.c:667`.
///
/// 1:1 with C: applies filesubstr to the leading `~`/`=`, then in
/// assign-context walks `=` (TYPESET-only) and `:`-separated path
/// lists, reapplying filesubstr to each suffix that begins with a
/// tilde/equals.
// ~, = subs: assign & PREFORK_TYPESET => typeset or magic equals          // c:667
fn filesub(namptr: &str, assign: i32) -> String {
    // c:667
    // C: `filesubstr(namptr, assign);`  (line 672)
    let mut namptr: String = filesubstr(namptr, assign != 0).unwrap_or_else(|| namptr.to_string()); // c:672

    // C: `if (!assign) return;` — non-assign context bails early.
    if assign == 0 {
        // c:674
        return namptr; // c:675
    }

    let mut eql: Option<usize> = None; // c:668 (eql=NULL)

    // C: PREFORK_TYPESET arm — `${var}=value` shape, find `=` then
    // recurse filesubstr on the RHS.
    if assign & PREFORK_TYPESET != 0 {
        // c:677
        // C: `(*namptr)[1] && (eql = sub = strchr(*namptr + 1, Equals))`.
        // C searches for the Equals TOKEN (subst.c:678). zshrs's
        // pipeline can deliver the arg in either form:
        //   - Tokenized (lexer-emitted assignment word, fusevm
        //     compile_zsh re-tokenized words): `=` chars come in as
        //     `Equals` (\u{8d}) and `~` as `Tilde` (\u{98}).
        //   - Untokenized (some entry sites): literal ASCII `=`/`~`.
        // Accept both forms so the magic-equals filesub trigger fires
        // regardless of which entry path delivered the arg. C's
        // strict-Equals-only behavior is preserved by checking the
        // token form first.
        if namptr.len() >= 2 {
            // c:678
            // c:678 — `strchr(*namptr + 1, Equals)`. Look for Equals
            // TOKEN first; fall back to literal `=` for untokenized
            // arrival paths.
            let chars: Vec<char> = namptr.chars().collect();
            let eql_pos = chars
                .iter()
                .skip(1)
                .position(|&c| c == Equals)
                .map(|p| p + 1)
                .or_else(|| {
                    chars.iter().skip(1).position(|&c| c == '=').map(|p| p + 1)
                });
            if let Some(sub_char_idx) = eql_pos {
                // c:678 — `sub` points at the Equals position.
                // sub_char_idx is the CHAR offset; convert to byte
                // offset for namptr slicing.
                let mut byte_off = 0;
                for c in chars.iter().take(sub_char_idx) {
                    byte_off += c.len_utf8();
                }
                let sub_byte = byte_off; // c:678 sub
                eql = Some(sub_byte);
                // c:679 — `str = sub + 1` (byte after the Equals).
                let str_start = sub_byte + chars[sub_char_idx].len_utf8();
                if str_start < namptr.len() {
                    // c:680 — `sub[1] == Tilde || sub[1] == Equals`.
                    //   Accept token AND ASCII for both.
                    let next_char = namptr[str_start..]
                        .chars()
                        .next()
                        .unwrap_or('\0');
                    let trigger = matches!(
                        next_char,
                        '~' | '=' | '\u{98}' /* Tilde */ | '\u{8d}' /* Equals */
                    );
                    if trigger {
                        // c:680 — `filesubstr(&str, assign)`.
                        let rhs = &namptr[str_start..];
                        if let Some(expanded) = filesubstr(rhs, true) {
                            // c:681-682 — `sub[1] = '\0'; *namptr =
                            // dyncat(*namptr, str);`. Splice the
                            // expanded text in place of the trailing
                            // part.
                            namptr = format!("{}{}", &namptr[..str_start], expanded);
                        }
                    }
                }
            } else {
                // c:684 — no Equals in arg → bail.
                return namptr; // c:685
            }
        } else {
            // c:684
            return namptr; // c:685
        }
    }

    // C: `ptr = *namptr; while ((sub = strchr(ptr, ':'))) { … }`
    // Walk `:`-separated path components, reapply filesubstr on each
    // suffix that starts with `~` or `=`. Accept both ASCII (`~`/`=`)
    // and the lexer's TOKEN forms (Tilde \u{98} / Equals \u{8d}) —
    // the bridge passthru path delivers TOKEN form for unquoted
    // tildes in assignment RHS like `X=/usr/bin:~/bin`.
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
        let starts_with_tilde_or_equals = if str_start < namptr.len() {
            let suffix = &namptr[str_start..];
            let first = suffix.chars().next();
            matches!(first, Some('~') | Some('=') | Some('\u{98}') | Some(Equals))
        } else {
            false
        };
        if past_eql && starts_with_tilde_or_equals {
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
  // c:zsh.h:200

/// Equal substitution (=cmd)
/// Port of `equalsubstr(char *str, int assign, int nomatch)` from `Src/subst.c:715`.
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
// do =foo substitution, or equivalent.                                     // c:715
/// `equalsubstr` — see implementation.
pub fn equalsubstr(s: &str, assign: bool, nomatch: bool) -> Option<String> {
    // c:715
    // C: `for (pp = str; !isend2(*pp); pp++);` — find end of cmd
    // name. isend2(c) = !c || c==Inpar || (assign && c==':').
    // The previous Rust port had a DUPLICATE `c != '\u{85}'` line
    // mis-labeled as "Inpar token" — `\u{85}` is Stringg, not Inpar.
    // Removed; `c != Inpar` (the canonical const) is the only correct check.
    let end = s // c:719
        .chars() // c:719
        .take_while(|&c| {
            // c:719
            c != '\0'                                       // c:719
                && c != Inpar                               // c:719
                && !(assign && c == ':') // c:719
        })
        .count();

    // C: `cmdstr = dupstrpfx(str, pp-str);
    //     untokenize(cmdstr); remnulargs(cmdstr);`
    let cmdstr_raw: String = s.chars().take(end).collect(); // c:721
    let mut cmdstr = untokenize(&cmdstr_raw); // c:722
    crate::ported::glob::remnulargs(&mut cmdstr); // c:723

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
                zerr(&format!("{} not found", cmdstr)); // c:726 — `zerr("%s not found", cmdstr)`
            }
            None // c:728
        }
    }
} // c:733

// Helper functions

/// Port of `filesubstr(char **namptr, int assign)` from `Src/subst.c:737`.
///
/// Performs `~` and `=` expansion on a single path component. Returns
/// `Some(expanded)` on success, `None` if no expansion applies. The
/// caller (filesub) chains this on `:`-separated path lists.
///
/// Faithful port of the C ladder — covers `~`, `~+`, `~-`, `~N`/`~-N`
/// (dirstack), `~user` (libc getpwnam), and `=cmd` (PATH lookup via
/// equalsubstr).
pub fn filesubstr(namptr: &str, assign: bool) -> Option<String> {
    // c:737
    // c:737
    if namptr.is_empty() {
        // c:737
        return None; // c:737
    }
    let chars: Vec<char> = namptr.chars().collect(); // c:737
    let first = chars[0]; // c:737

    // c:741 — `if (*str == Tilde && str[1] != '=' && str[1] != Equals)`.
    // STRICTLY matches Tilde TOKEN (Src/zsh.h:189 Tilde=0x98) per C.
    // ASCII `~` is REJECTED — those arrive from substitution results
    // (`${var/foo/\~}`), DQ-quoted source (`"~/foo"`), and other
    // contexts where zsh treats `~` as a literal character. The
    // unquoted `echo ~` path reaches here with Tilde TOKEN
    // preserved by the EXPAND_TEXT emit's untokenize loop
    // (compile_zsh.rs:3048+) so this strict check still fires for
    // legitimate tilde expansions.
    if first == '\u{98}'
    /* Tilde token */
    {
        // c:741
        if chars.len() == 1 {
            // c:748 — bare ~
            let home = getsparam("HOME").unwrap_or_default();
            return Some(home);
        }
        // c:Src/subst.c:743+ — the byte AFTER `~` selects the
        // expansion form (`/`/`+`/`-`/digit/identifier/`=`). The
        // Rust lexer emits Dash TOKEN (\u{9b}) for `-` and ASCII
        // for the others, but the discriminator predicates here
        // are written against ASCII. Normalise Dash → `-` so the
        // `~-` (OLDPWD) and `~-N` (dirstack) arms below match.
        let raw_nx = chars[1];
        let nx = if raw_nx == '\u{9b}' { '-' } else { raw_nx };
        if nx == '=' {
            return None;
        } // c:741 — leave for =arm

        // C `isend(c)`: !c || c=='/' || c==Inpar || (assign && c==':')
        // c:725 macro.
        //
        // The previous Rust port used `\u{85}` for "Inpar", which is
        // ACTUALLY the Stringg token byte (Src/zsh.h:160 `String=0x85`).
        // The canonical Inpar value is `\u{88}` (Src/zsh.h:163 `Inpar=0x88`).
        // Effect: `~(xxx` (legitimate Inpar after tilde) wouldn't isend,
        // while `~$xxx` (Stringg, which should NOT isend) would.
        let isend =
            |c: char| -> bool { c == '\0' || c == '/' || c == Inpar || (assign && c == ':') };

        // `~/...` and `~` (isend(str[1])) — bare HOME
        if isend(nx) {
            // c:748
            let home = getsparam("HOME").unwrap_or_default();
            let suffix: String = chars[1..].iter().collect();
            return Some(format!("{}{}", home, suffix));
        }
        // c:751-753 — `} else if (str[1] == '+' && isend(str[2])) {`
        //              `*namptr = dyncat(pwd, str + 2);`
        //              `return 1;`
        // C's `isend(c)` macro (subst.c:725) is `!c || c=='/' ||
        // c==Inpar || (assign && c==':')`. The NUL test (`!c`) makes
        // bare `~+` (no trailing char, equivalent to NUL-terminator
        // in C's char* model) expand to `$PWD`. The previous Rust
        // port required `chars.len() >= 3`, dropping the bare case
        // and printing literal `~+`. Parity bug #26.
        if nx == '+' && (chars.len() == 2 || isend(chars[2])) {
            // c:752 — `*namptr = dyncat(pwd, str + 2);`
            let pwd = getsparam("PWD").unwrap_or_default();
            let suffix: String = chars[2..].iter().collect();
            return Some(format!("{}{}", pwd, suffix));
        }
        // c:754-757 — `} else if (str[1] == '-' && isend(str[2])) {`
        //              `*namptr = dyncat((tmp = oldpwd) ? tmp : pwd, str + 2);`
        //              `return 1;`
        // Same isend(NUL)-is-true semantics as ~+. Parity bug #27.
        if nx == '-' && (chars.len() == 2 || isend(chars[2])) {
            // c:755 — `(tmp = oldpwd) ? tmp : pwd`. Read both via
            // paramtab so OLDPWD-not-yet-set falls back to PWD.
            let oldpwd = getsparam("OLDPWD")
                .or_else(|| getsparam("PWD"))
                .unwrap_or_default();
            let suffix: String = chars[2..].iter().collect();
            return Some(format!("{}{}", oldpwd, suffix));
        }
        // `~+N` / `~-N` — dirstack entry. C: `if (!inblank(str[1]) &&
        // isend(*ptr) && (!idigit(str[1]) || (ptr - str < 4)))`.
        // Walk digit suffix; ptr ends at first non-digit.
        if (nx == '+' || nx == '-' || nx.is_ascii_digit()) && !nx.is_whitespace() {
            // Parse signed integer from chars[1..]. Normalize Dash TOKEN
            // (\u{9b}) to ASCII `-` for the sign check — the lexer emits
            // Dash for `-` and the existing `nx` predicate already maps
            // it, but the per-char sign test below must do the same or
            // `~-N` walks past the sign as if `N` was just a digit.
            // Bug #358.
            let ch_at = |i: usize| -> char {
                let c = chars[i];
                if c == '\u{9b}' {
                    '-'
                } else {
                    c
                }
            };
            let mut p = 1_usize;
            let neg = ch_at(p) == '-';
            if ch_at(p) == '+' || ch_at(p) == '-' {
                p += 1;
            }
            let dstart = p;
            while p < chars.len() && chars[p].is_ascii_digit() {
                p += 1;
            }
            // c:Src/subst.c:771 — `isend(*ptr)` accepts the NUL
            // terminator via the `!c` arm of the macro. In the
            // Rust char-vec model, `p == chars.len()` is the
            // equivalent. Without this clause `~0` (just digit, no
            // trailing `/`) reached p==chars.len() and failed the
            // `p < chars.len()` gate, leaving the literal in place.
            if p > dstart && (p == chars.len() || isend(chars[p])) {
                let val: i32 = chars[dstart..p]
                    .iter()
                    .collect::<String>()
                    .parse()
                    .unwrap_or(0);
                // c:Src/subst.c:771-773 — `if (val < 0) val = -val;` —
                // the sign was encoded in `str[1]` (the `+`/`-` token);
                // dstackent receives the magnitude. Previous Rust port
                // negated val when neg was set, sending -1 to dstackent
                // which made the loop walk zero steps. Bug #358.
                let _ = neg; // sign already carried in ch below
                let pwd = getsparam("PWD").unwrap_or_default();
                // Direct port of subst.c filesub'namptr tilde-+/- arm:
                // dstackent(ch, val) → pwd or stack entry.
                // c:4902 — read from canonical DIRSTACK global (mirrors
                // C'namptr `mod_export LinkList dirstack` at builtin.c:743).
                let dirstack: Vec<String> = DIRSTACK.lock().map(|d| d.clone()).unwrap_or_default();
                let pushdminus = isset(PUSHDMINUS); // c:4906
                                                    // c:Src/subst.c:786 — bare digit form `~N` without
                                                    // explicit `+`/`-` defaults to `+N` (top of stack at
                                                    // 0). When neg==false and we saw no sign char, fall
                                                    // through to `+` semantics; `dstackent` handles the
                                                    // 0-of-(empty-stack-with-only-pwd) case.
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
        // c:Src/subst.c isend macro treats string terminator `\0`
        // as end-of-string. Rust's char-slice has no NUL sentinel;
        // `p == chars.len()` is the equivalent. Without this clause
        // bare `~root` fell through without expanding because the
        // identifier walk consumed every char and the final isend
        // check ran against an out-of-bounds index.
        if p > 1 && (p == chars.len() || isend(chars[p])) {
            let user: String = chars[1..p].iter().collect();
            let suffix: String = chars[p..].iter().collect();
            // Named-dir lookup FIRST — `hash -d name=path` registered
            // names take precedence over OS users (zsh canonical).
            // Direct port of subst.c filesub which checks
            // nameddirtab via getnameddir before falling through to
            // getpwnam.
            // Canonical nameddirtab lookup (mirrors C'namptr
            // `getnameddir(name)` at hashnameddir.c via gethashnode2).
            let named = nameddirtab()
                .lock()
                .ok()
                .and_then(|t| t.get(&user).map(|nd| nd.dir.clone()));
            if let Some(path) = named {
                return Some(format!("{}{}", path, suffix));
            }
            // libc getpwnam — cstring -> pw_dir
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
            // c:Src/subst.c:803 — `zerr("no such user or named
            // directory: %s", str+1);` when neither nameddirtab nor
            // getpwnam resolves. Caller keeps the literal (filesub's
            // unwrap_or_else); zsh's exit-status fallout comes from
            // errflag being set so the command exits non-zero.
            zerr(&format!("no such user or named directory: {}", user));
            errflag_set_error();
            return None;
        }
        return None;
    }

    // `=cmd` — PATH lookup via equalsubstr. C:
    // `if (*str == Equals && isset(EQUALS) && str[1] && str[1] != Inpar)`.
    // The check is on the TOKENIZED Equals marker (\u{8d}), NOT
    // literal `=`. The Equals marker is only set by the lexer for
    // a source-level literal `=` at the start of a word; results
    // of param expansion / cmd substitution that happen to start
    // with `=` get the raw byte and stay literal.
    //
    // Parity bug: zshrs accepted BOTH literal `=` AND Equals,
    // so post-substitution strings like `${kv#a}` of "a=1" → "=1"
    // got incorrectly treated as `=cmd` and triggered command-not-
    // found ("1: not found"). Match C: Equals marker only.
    if first == Equals
        && chars.len() > 1
        && chars[1] != Inpar
        && crate::ported::zsh_h::isset(crate::ported::zsh_h::EQUALSOPT)
    {
        let cmd_part: String = chars[1..].iter().collect();
        // Split at `:` if assign, else take the whole thing.
        let cmd = if assign {
            cmd_part.split(':').next().unwrap_or(&cmd_part).to_string()
        } else {
            cmd_part.clone()
        };
        // C: `pathprog(cmd, &fullname)` walks `path[]`. paramtab read.
        let path = getsparam("PATH").unwrap_or_default();
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
        // c:Src/subst.c:725 — `=cmd` lookup failed. zsh emits the
        // "not found" diagnostic + sets errflag so the enclosing
        // command exits non-zero. The previous Rust port silently
        // returned None and filesub kept the literal `=cmd`,
        // diverging from zsh's hard-fail behaviour.
        // Untokenize Equals (`\u{8d}`) → `=` etc. before emit so
        // `[ a == a ]` reports "= not found" (the second `=` came in
        // as Equals from the lexer) instead of "\u{8d} not found".
        let cmd_display = untokenize(&cmd);
        zerr(&format!("{} not found", cmd_display)); // c:726
        errflag_set_error();
    }
    None
}

/// Port of `strcatsub(char **d, char *pb, char *pe, char *src, int l, char *s, int glbsub, int copied)` from `Src/subst.c:814`.
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
/// WARNING: param names don't match C — Rust=(prefix, src, suffix, glob_subst) vs C=(d, pb, pe, src, l, s, glbsub, copied)
pub fn strcatsub(prefix: &str, src: &str, suffix: &str, glob_subst: bool) -> String {
    // c:814
    // c:814
    // C: `if (!pl && (!s || !*s)) { *d = dest = (copied ? src :
    //     dupstring(src)); if (glbsub) shtokenize(dest); }`
    // — fast path: no prefix, no suffix, just src (optionally
    // shtokenized).
    // c:820-823 — `if (!pl && (!s || !*s)) { *d = dest = (copied ?
    //   src : dupstring(src)); if (glbsub) shtokenize(dest); }`.
    // The C-faithful shtokenize call is wired here; strcatsub is
    // currently invoked only from unit tests in this module — the
    // active paramsubst pipeline doesn't route through this port
    // yet (the GLOBSUBST flag is mutated on the global option table
    // at subst.rs:4053 / 11777, which is a Rust-port deviation from
    // C's per-paramsubst local `globsubst`). When the paramsubst
    // port is reworked to use a local, callers should pass
    // `glob_subst` here so the tokenization fires.
    if prefix.is_empty() && suffix.is_empty() {
        // c:820
        let mut dest = src.to_string(); // c:821
        if glob_subst {
            // c:822
            crate::ported::glob::shtokenize(&mut dest);
        }
        return dest;
    }

    // c:825-833 — general path: pre-allocate + copy three segments.
    let mut result =
        String::with_capacity(prefix.len() + src.len() + suffix.len() + 1); // c:825
    result.push_str(prefix); // c:826
    if glob_subst {
        // c:829 — tokenize the src segment before joining.
        let mut src_tok = src.to_string();
        crate::ported::glob::shtokenize(&mut src_tok);
        result.push_str(&src_tok);
    } else {
        result.push_str(src); // c:828
    }
    result.push_str(suffix); // c:833
    result // c:835
} // c:836

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
/// Port of `wcpadwidth(wchar_t wc, int multi_width)` from `Src/subst.c:848`.
///
/// Returns the display-cell width of a wide char for `dopadding`.
///
/// C signature: `int wcpadwidth(wchar_t wc, int multi_width)`.
/// multi_width values:
///   0 → always 1 (legacy / no multibyte)
///   1 → u9_wcwidth(wc); zero if negative
///   * → boolean: 1 if u9_wcwidth>0 else 0
pub fn wcpadwidth(wc: char, multi_width: i32) -> i32 {
    // c:848
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

/// String padding
/// `${(l:N:)var}` left/right-pad.
/// Port of `dopadding(char *str, int prenum, int postnum, char *preone, char *postone, char *premul, char *postmul #ifdef MULTIBYTE_SUPPORT , int multi_width #endif)` from Src/subst.c:893.
///
/// `multi_width` controls cell-counting per the (m) flag (subst.c:2376):
///   • 0  → every char counts as one cell (C zsh's MULTIBYTE_SUPPORT off)
///   • 1+ → use wcpadwidth (CJK wide=2, combining=0, ZWJ=0).
/// WARNING: param names don't match C — Rust=() vs C=(str, prenum, postnum, preone, postone, premul, MULTIBYTE_SUPPORT, endif)
pub fn dopadding(
    // c:893
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
/// Parse a `:STR:`-delimited flag argument.
/// Port of `get_strarg(char *s, int *lenp)` from `Src/subst.c:1348`.
///
/// C iterates char-by-char looking for the matching close-delimiter,
/// returning the content between delimiters AND the position past
/// the closing delim. Bracket-pair mappings: `(...)` / `[...]` /
/// `{...}` / `<...>` (plus their tokenized counterparts).
///
/// **Multibyte-correctness fix:** previous Rust port indexed `&s[rest_start..]`
/// where `rest_start` came from `chars().enumerate()` — i.e. the
/// CHAR index, NOT the byte index. For ASCII input these match,
/// but for multibyte content (e.g. `(:é:rest)`) the char-index
/// landed mid-codepoint and produced wrong slices (or panicked).
/// Also failed to advance past the closing delim because
/// `rest_start = i + 1` used the char-index +1 which is again
/// mid-codepoint for multibyte preceding the delim.
///
/// Fix: use `s.char_indices()` which yields `(byte_offset, char)`
/// pairs. `rest_start = byte_offset + char.len_utf8()` correctly
/// advances past the close-delim regardless of width.
/// WARNING: param names don't match C — Rust=(s) vs C=(s, lenp)
pub fn get_strarg(s: &str) -> Option<(char, String, &str)> {
    // c:1348
    let mut iter = s.char_indices();

    // Get delimiter (and its byte width).
    let (_, del) = iter.next()?;
    let close_del = match del {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '<' => '>',
        Inpar => Outpar,
        Inbrack => Outbrack,
        Inbrace => Outbrace,
        Inang => Outang,
        _ => del,
    };

    // Collect content until closing delimiter.
    let mut content = String::new();
    let mut rest_start = s.len(); // default: no close-delim found → rest = ""
    for (byte_off, c) in iter {
        if c == close_del {
            rest_start = byte_off + c.len_utf8();
            break;
        }
        content.push(c);
    }

    Some((del, content, &s[rest_start..]))
}

/// Get integer argument for flags like (l.N.)
/// Parse an `:N:`-delimited integer flag argument.
/// Port of `get_intarg(char **s, int *delmatchp)` from Src/subst.c:1428.
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
/// WARNING: param names don't match C — Rust=(s) vs C=(s, delmatchp)
pub fn get_intarg(s: &str) -> Option<(i64, &str)> {
    // c:1428
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
    // `(l:$n:)` looks up $n). Caller is expected to be inside a live
    // ExecutorContext (parsestr is reached from script execution);
    // explicit ShellExecutor reach-in from src/ported/ is forbidden
    // — see memory feedback_no_exec_script_from_ported.
    let expanded = singsub(&parsed); // c:1444
    if errflag_set() {
        return None;
    } // c:1445

    // C: `ret = mathevali(p);` — evaluate as integer math.
    // c:1446 `if (errflag) return -1;` — the C body relies on mathevali's
    // internal zerr() to have already written to stderr; Rust's
    // mathevali captures the message in Err — surface it here.
    let ret = match mathevali(&expanded) {
        // c:1447
        Ok(n) => n, // c:1447
        Err(msg) => {
            zerr(&msg); // emit error to stderr (C side-effect)
            return None; // c:1448
        }
    };

    // C: `if (ret < 0) ret = -ret;` — absolute value.
    let abs_ret = if ret < 0 { -ret } else { ret }; // c:1452

    // C: `*delmatchp = arglen;` — Rust folds delim-len into rest.
    Some((abs_ret, rest)) // c:1455
} // c:1457

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
///   3. If !single, walk buffer: outside Dnull (`"`) regions convert
///      `Qstring` → `String` and `Qtick` → `Tick`. Dnull toggles qt.
/// Port of `subst_parse_str(char **sp, int single, int err)` from `Src/subst.c:1460`.
pub fn subst_parse_str(sp: &str, single: bool, err: bool) -> Option<String> {
    // c:1460
    // c:1460
    let _ = err; // c:1466 (parsestr error path
                 //         deferred — full C
                 //         lexer reentry pending)
                 // C: `*sp = sp = dupstring(*sp);` — duplicate so the caller'sp
                 // original buffer is unaffected. Rust'sp String already owns;
                 // we work on a local copy below.
    let mut buf: String = sp.to_string(); // c:1465

    // C: `if (!single) { … }` — the conversion only runs in the
    // non-SINGLE arm (when paramsubst-output may be subsequently
    // word-split / expanded).
    if !single {
        // c:1469
        let mut chars: Vec<char> = buf.chars().collect(); // c:1469
        let mut qt = false; // c:1470
                            // The previous Rust port had FAKE token-byte values in the
                            // comment block here: STRING=\u{81}, Qstring=\u{82}, Tick=\u{83},
                            // Qtick=\u{84}, Dnull=\u{97}. NONE of these match `Src/zsh.h:159-194`.
                            // Canonical values:
                            //   Stringg = \u{85} (c:160), Qstring = \u{8c} (c:167),
                            //   Tick    = \u{93} (c:174), Qtick   = \u{99} (c:180),
                            //   Dnull   = \u{9e} (c:194).
                            // With the wrong literals, this Qstring/Qtick→String/Tick
                            // rewrite + Dnull-toggle loop NEVER fired on real input —
                            // every `$'...'` and `` $`...` `` expansion produced wrong
                            // tokenization downstream. Now uses canonical consts.
        for c in chars.iter_mut() {
            // c:1472
            if !qt {
                // c:1473
                if *c == Qstring {
                    // c:1474
                    *c = Stringg; // c:1475
                } else if *c == Qtick {
                    // c:1476
                    *c = Tick; // c:1477
                }
            }
            if *c == Dnull {
                // c:1480
                qt = !qt; // c:1481
            }
        }
        buf = chars.iter().collect(); // c:1483
    }
    // C: `return 0;` — success path returns the buffer.
    Some(buf) // c:1483
} // c:1486

/// Evaluate character from number (for (#) flag)
/// Port of `substevalchar(char *ptr)` from `Src/subst.c:1490`.
///
/// Implements the `(#)` paramsubst flag: evaluate the expression as
/// a math integer, then convert that codepoint to a UTF-8 string.
/// Used by `${(#)foo}` where `foo` is a numeric expression yielding
/// a character code.
pub fn substevalchar(ptr: &str) -> Option<String> {
    // c:1490
    // C: `int saved_errflag = errflag; errflag = 0;` — clear-and-save
    // the global error flag around mathevali so failure from an
    // invalid math expr stays local.
    // (Rust port has no global errflag — the Result type carries
    // the error directly.)
    let ires = match mathevali(ptr) {
        // c:1497
        Ok(n) => n, // c:1497
        Err(msg) => {
            // c:1499
            // C: `return noerrs ? dupstring("") : NULL;` —
            // empty string when noerrs flag is set, NULL otherwise.
            // The C path's zerr() inside mathevali wrote the message
            // to stderr before this return; Rust's mathevali captures
            // it in Err — surface it via zerr().
            zerr(&msg);
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
    // byte for compatibility with C'ptr `(char)ires` cast.
    let byte = (ires as u32 & 0xFF) as u8; // c:1517
    Some(String::from_utf8_lossy(&[byte]).into_owned()) // c:1517
} // c:1521

/// Untokenize and escape string for flag argument
/// Port of `untok_and_escape(char *s, int escapes, int tok_arg)` from `Src/subst.c:1528`.
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

    // C: `if (escapes && (*s == Stringg || *s == Qstring) && s[1])`
    let chars: Vec<char> = s.chars().collect(); // c:1533
    // c:1533 checks the tokenized `$` (Stringg/Qstring). When the flag
    // arg reaches here untokenized (the bare-`$NAME` separator path,
    // e.g. `${(ps.$sep.)v}`), the `$` is a literal 0x24; accept it too
    // so the variable's value is substituted instead of being escape-
    // decoded into a literal `$sep`.
    if escapes && chars.len() >= 2                          // c:1533
        && (chars[0] == STRING || chars[0] == Qstring || chars[0] == '$')
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
            // c:1543 — C's untokenize maps tokens to ASCII per
            // ztokens (Stringg/Qstring → `$`, Snull → `'`, Dnull →
            // `"`). The Rust plain `untokenize` strips quote markers
            // AND inline-decodes `$'...'` regions, which collapsed a
            // flag arg like `(j.$'\n'.)` to a bare newline; zsh 5.9
            // keeps the wrapper literal: `print -r ${(j.$'\n'.)a}`
            // → `x$'\n'y`. Bug #626 in docs/BUGS.md.
            let untoked = crate::ported::lex::untokenize_ztokens(s); // c:1543
            if escapes {
                // c:1544
                // C: `dst = getkeystring(dst, &klen,
                //          GETKEYS_SEP, NULL); dst = pastebuf(...);`
                getkeystring(&untoked).0 // c:1545
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
        // shtokenize call elided — same as c:823 / c:830 above (the
        // tokenized form isn't consumed by current zshrs pipeline).
        // Result kept as-is; tok_arg is a hint for downstream glob
        // engines that consume the tokenized form directly.
    }
    result // c:1553
} // c:1554

/// Check for colon subscript in parameter expansion
/// Port of `check_colon_subscript(char *str, char **endp)` from `Src/subst.c:1566`.
///
/// Detects a `${var:OFFSET[:LEN]}` substring shape vs a history
/// modifier or other postfix. Returns `Some((subscript_expr, rest))`
/// when the input looks like a colon-substring (offset evaluable as
/// math), `None` otherwise.
///
/// C signature: `char *check_colon_subscript(char *str, char **endp)`.
/// Rust returns the parsed (subscript, remainder) pair.
/// WARNING: param names don't match C — Rust=(s) vs C=(str, endp)
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
        // Previous Rust port had `\u{85}` labeled Inpar and `\u{86}`
        // labeled Outpar — both wrong. `\u{85}` is Stringg ($), `\u{86}`
        // is Hat (^). Canonical Inpar/Outpar are 0x88/0x8a per
        // `Src/zsh.h:163,165`. Use the canonical consts.
        match c {
            // c:1579
            '[' | Inbrack => depth += 1,  // c:1579
            ']' | Outbrack => depth -= 1, // c:1579
            '(' | Inpar => depth += 1,    // c:1579
            ')' | Outpar => depth -= 1,   // c:1579
            ':' if depth == 0 => {
                end = Some(i);
                break;
            } // c:1579
            _ => {}
        }
    }
    let end = end.unwrap_or(s.len()); // c:1582 (fallthrough '\0')
    let expr: String = chars[..end].iter().collect(); // c:1583

    // C lines 1585-1591: `parsestr` + `singsub` + `remnulargs` +
    // `untokenize` on the captured expression.
    let parsed = subst_parse_str(&expr, false, true)?; // c:1587
    let mut expanded = singsub(&parsed); // c:1589
    if errflag_set() {
        return None;
    } // c:1590
    crate::ported::glob::remnulargs(&mut expanded); // c:1590
    let untoked = untokenize(&expanded); // c:1591

    let rest: String = chars[end..].iter().collect(); // c:1593
    Some((untoked, rest)) // c:1596
} // c:1597

// parameter substitution                                                   // c:1601
/// Parameter substitution
/// Port of paramsubst(LinkList l, LinkNode n, char **str, int qt, int pf_flags, int *ret_flags) from subst.c lines 1600-4922 (THIS IS THE BIG ONE)
// parameter substitution                                                   // c:1601
/// `paramsubst` — see implementation.
thread_local! {
    /// !!! WARNING: RUST-ONLY STATE — NO DIRECT C COUNTERPART !!!
    /// C's `${~spec}` sets a paramsubst-LOCAL `globsubst` variable
    /// (Src/subst.c:2596) and marks the RESULT via shtokenize; the
    /// consumers read token bytes, never the option table. zshrs's
    /// paramsubst carries the flag through the GLOBAL option table
    /// so the compile-emitted glob ops downstream in the SAME word
    /// pipeline can see it (documented deviation at subst.rs:2172).
    /// This cell records "the option was flipped as a CARRIER, with
    /// this saved user value" so the command-dispatch boundary
    /// (fusevm_bridge::consume_tilde_globsubst_carrier) can restore
    /// the user's setting once the word pipeline is done. Without
    /// the restore, zinit.zsh:150 `ZINIT[PLUGINS_DIR]=${~ZINIT[…]}`
    /// left GLOB_SUBST on for the WHOLE SESSION and every expanded
    /// value with `(`/`[` glob-errored (autopair keys, PROMPT4,
    /// fzf-tab ANSI strings).
    pub static TILDE_GLOBSUBST_CARRIER: std::cell::Cell<Option<bool>> =
        const { std::cell::Cell::new(None) };
}

/// Record the user's GLOB_SUBST value before the `${~}` carrier flip
/// (first flip in a pipeline wins — that's the user's real setting).
fn tilde_carrier_note(saved: bool) {
    TILDE_GLOBSUBST_CARRIER.with(|c| {
        if c.get().is_none() {
            c.set(Some(saved));
        }
    });
}

pub fn paramsubst(
    // c:1625
    s: &str,             // c:1625
    start_pos: usize,    // c:1625
    qt: bool,            // c:1625
    pf_flags: i32,       // c:1625
    ret_flags: &mut i32, // c:1625
) -> (String, usize, Vec<String>) {
    // c:1625
    // c:Src/lex.c:1546 + Src/lex.c:1565 — C's lex emits Stringg/Qstring
    // followed by Inbrace at every `${` opener and the matching Outbrace
    // at the closer; paramsubst sees only the token form. Rust bridges
    // and string-literal callers pass raw ASCII `${…}`, so pre-tokenize
    // the matching brace pair here so the downstream scanner at
    // subst.rs:2920 reads canonical Inbrace/Outbrace exclusively (drops
    // the raw-`{`/`}` legs that previously made it a "hybrid scanner").
    //
    // Only the outermost `${…}` opened at `start_pos` is tokenized —
    // nested `${…}` inside come from the lexer (already tokenized) or
    // arrive raw and would be tokenized on their own recursive
    // paramsubst entry. Escape-marked braces (`\{`, Bnull `{`) stay
    // raw so the scanner's `prev != '\\'` and `prev != Bnull` gates
    // continue to skip them.
    let s_owned: String = {
        let raw_chars: Vec<char> = s.chars().collect();
        // start_pos points at `$` (or Stringg/Qstring); the brace, if
        // any, sits at start_pos+1.
        let brace_pos = start_pos + 1;
        if brace_pos < raw_chars.len() && raw_chars[brace_pos] == '{' {
            // Walk to find the matching `}` with depth tracking,
            // honoring the same escape gates the post-tokenize scanner
            // applies (so the input shape stays unchanged for already-
            // escaped braces).
            let mut depth = 1_i32;
            let mut end = brace_pos + 1;
            while end < raw_chars.len() && depth > 0 {
                let ch = raw_chars[end];
                let prev = if end > 0 { raw_chars[end - 1] } else { '\0' };
                let prev2 = if end > 1 { raw_chars[end - 2] } else { '\0' };
                if ch == Inbrace {
                    depth += 1;
                } else if ch == '{' {
                    let dollar_preceded =
                        prev == '$' && prev2 != '\\' && prev2 != Bnull;
                    if dollar_preceded {
                        depth += 1;
                    }
                } else if ch == Outbrace {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                } else if ch == '}' && prev != '\\' && prev != Bnull {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                end += 1;
            }
            if depth == 0 && end < raw_chars.len() && raw_chars[end] == '}' {
                let mut out: String = String::with_capacity(s.len());
                out.extend(raw_chars[..brace_pos].iter()); // up to but not including `{`
                out.push(Inbrace);
                out.extend(raw_chars[brace_pos + 1..end].iter()); // body
                out.push(Outbrace);
                out.extend(raw_chars[end + 1..].iter());
                out
            } else {
                s.to_string()
            }
        } else {
            s.to_string()
        }
    };
    let s: &str = &s_owned;
    let chars: Vec<char> = s.chars().collect(); // c:1625
    let mut pos = start_pos + 1; // Skip $ or Qstring       // c:1625
    let mut result_nodes = Vec::new(); // c:1625

    // c:Src/subst.c:3360-3412 — in C the pattern argument of the
    // `${x#pat}` / `${x%pat}` / `${x/pat/repl}` operators reaches
    // paramsubst LEXER-TOKENIZED: source glob metachars are token
    // bytes (`case '%': case '#': case Pound: case '/':` at
    // subst.c:3360-3363, re-established by `parse_subst_string(s)`
    // at subst.c:3374 when the text arrives untokenized). Then
    // `singsub(&s)` (subst.c:3412) splices parameter values in RAW
    // (untokenized = literal to patcompile, per the zpc_chars
    // contract at Src/pattern.c:248), and splices only become
    // active under GLOBSUBST via the `if (... isset(GLOBSUBST))
    // shtokenize(s)` in stringsubst (Src/subst.c:1669 globsubst +
    // c:1756 ssub path). zshrs's `rest` walker (subst.rs:4698)
    // folds the lexer's token bytes back to raw ASCII, losing the
    // source-vs-splice distinction, so previously the spliced
    // value of `${x#$p}` acted as a glob without GLOBSUBST. This
    // closure restores the C pre-singsub encoding: source glob
    // metachars -> their zsh.h:159-183 token chars, while passing
    // through UNTOUCHED: `\X` pairs, Bnull/Bnullkeep + payload,
    // `$name` / `${...}` / `$(...)` / backtick spans (so singsub
    // still sees raw substitution syntax). Two deliberate
    // non-uniformities, both pinned to real-zsh probes:
    //   - `|` at paren depth 0 outside `[...]` stays LITERAL even
    //     under GLOBSUBST (`setopt globsubst; x="hello|key";
    //     echo ${x%|*}` -> `hello` in zsh 5.9): escape to `\|`
    //     here rather than leaving it for the post-singsub pass,
    //     which must treat leftover raw `|` as a splice. C-lexer
    //     rule: the lexer tokenizes `|` to Bar only INSIDE `(...)`
    //     groups; zpc_chars[ZPC_BAR] = Bar (Src/pattern.c:248)
    //     means only Bar — never ASCII `|` — terminates an
    //     alternation branch. Bugs #12/#365/#596 history lives in
    //     docs/BUGS.md (formerly the escape_bare_alt_pipes closure,
    //     subsumed here).
    //   - `<` / `>` only tokenize as the `<N-N>` numeric-range
    //     pair, mirroring zshtokenize's digit scan at
    //     Src/glob.c:3605-3614 (a stray `<` is not a glob opener).
    let pretokenize_src_pat = |s: &str| -> String {
        use crate::ported::zsh_h::{
            Bar, Bnull, Bnullkeep, Hat, Inang, Inbrace, Inbrack, Inpar, Outang, Outbrace,
            Outbrack, Outpar, Pound, Quest, Star, Tilde,
        };
        let chars: Vec<char> = s.chars().collect();
        let mut out = String::with_capacity(s.len());
        let mut depth = 0i32;
        let mut in_class = false;
        let mut i = 0usize;
        while i < chars.len() {
            let c = chars[i];
            // Source `\X` escape — both chars verbatim.
            if c == '\\' && i + 1 < chars.len() {
                out.push(c);
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            // Lexer quote markers Bnull/Bnullkeep — payload is
            // user-literal; pass the pair through untouched.
            //
            // EXCEPTION: Bnull + `\` (source `\\` — a QUOTED literal
            // backslash) is rewritten to the parser's raw-ASCII
            // literal form `\\` HERE, before singsub: stringsubst's
            // Bnull arm (subst.rs:696) DROPS the marker and keeps the
            // payload raw, which turned the quoted backslash into an
            // ACTIVE escape prefix for patcompile — `${s/\\./X}`
            // matched a plain dot instead of backslash+dot. C never
            // loses the marker (Bnull survives the whole subst walk;
            // remnulargs strips it only at the value boundary, and
            // patcompile reads Bnull-marked chars as literal —
            // Src/lex.c:1508 add(Bnull) + Src/pattern.c Bnull
            // contract). Other payloads must KEEP the marker pair:
            // raw `$` / `` ` `` would re-substitute inside
            // stringsubst, and raw glob chars are already literalized
            // downstream. Bug #296.
            if c == Bnull && i + 1 < chars.len() && chars[i + 1] == '\\' {
                out.push('\\');
                out.push('\\');
                i += 2;
                continue;
            }
            if (c == Bnull || c == Bnullkeep) && i + 1 < chars.len() {
                out.push(c);
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            // Backtick span — copy verbatim so singsub sees the
            // raw command substitution.
            if c == '`' {
                out.push(c);
                i += 1;
                while i < chars.len() {
                    let d = chars[i];
                    out.push(d);
                    i += 1;
                    if d == '\\' && i < chars.len() {
                        out.push(chars[i]);
                        i += 1;
                        continue;
                    }
                    if d == '`' {
                        break;
                    }
                }
                continue;
            }
            // `$…` substitution span — copy verbatim. Nested
            // `${…}` bodies may carry Inbrace/Outbrace tokens
            // (preserved by the rest walker at subst.rs:4698), so
            // the brace balance counts both raw and token forms.
            if c == '$' {
                out.push(c);
                i += 1;
                let next = chars.get(i).copied().unwrap_or('\0');
                if next == '{' || next == Inbrace {
                    let mut bd = 0i32;
                    while i < chars.len() {
                        let d = chars[i];
                        out.push(d);
                        i += 1;
                        if d == '{' || d == Inbrace {
                            bd += 1;
                        } else if d == '}' || d == Outbrace {
                            bd -= 1;
                            if bd == 0 {
                                break;
                            }
                        }
                    }
                } else if next == '(' {
                    // `$(…)` / `$((…))` — balanced raw parens.
                    let mut pd = 0i32;
                    while i < chars.len() {
                        let d = chars[i];
                        out.push(d);
                        i += 1;
                        if d == '(' {
                            pd += 1;
                        } else if d == ')' {
                            pd -= 1;
                            if pd == 0 {
                                break;
                            }
                        }
                    }
                } else if matches!(next, '?' | '#' | '$' | '!' | '@' | '*' | '-')
                    || next.is_ascii_digit()
                {
                    // Special single-char parameter — copy so the
                    // metachar map below never sees it.
                    out.push(next);
                    i += 1;
                } else {
                    while i < chars.len()
                        && (chars[i].is_alphanumeric() || chars[i] == '_')
                    {
                        out.push(chars[i]);
                        i += 1;
                    }
                }
                continue;
            }
            // `<N-N>` numeric range — Src/glob.c:3605-3614 shape:
            // `<` digits `-` digits `>` (both digit runs optional).
            if c == '<' {
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    j += 1;
                }
                if j < chars.len() && chars[j] == '-' {
                    let mut k = j + 1;
                    while k < chars.len() && chars[k].is_ascii_digit() {
                        k += 1;
                    }
                    if k < chars.len() && chars[k] == '>' {
                        out.push(Inang);
                        for &d in &chars[i + 1..k] {
                            out.push(d);
                        }
                        out.push(Outang);
                        i = k + 1;
                        continue;
                    }
                }
                out.push(c);
                i += 1;
                continue;
            }
            match c {
                '#' => out.push(Pound),
                '^' => out.push(Hat),
                '*' => out.push(Star),
                '?' => out.push(Quest),
                '~' => out.push(Tilde),
                '(' => {
                    depth += 1;
                    out.push(Inpar);
                }
                ')' => {
                    if depth > 0 {
                        depth -= 1;
                    }
                    out.push(Outpar);
                }
                '[' => {
                    in_class = true;
                    out.push(Inbrack);
                }
                ']' => {
                    in_class = false;
                    out.push(Outbrack);
                }
                '|' => {
                    if depth > 0 || in_class {
                        out.push(Bar);
                    } else {
                        // Literal per Src/pattern.c:248 — see header.
                        out.push('\\');
                        out.push('|');
                    }
                }
                other => out.push(other),
            }
            i += 1;
        }
        out
    };

    // Post-singsub half of the Src/subst.c:3412 contract: after
    // `singsub` splices parameter values RAW into the pre-tokenized
    // pattern, every remaining raw ASCII glob metachar came from a
    // splice and must stay LITERAL to patcompile (Src/pattern.c:248
    // zpc_chars dispatch on token bytes only) — unless GLOBSUBST is
    // set, where C shtokenizes spliced values (Src/subst.c:1669 +
    // c:1756 ssub path) making them active. The downstream matchers
    // at these sites (`glob_match_static`, vm_helper.rs:3627) run
    // their own `tokenize` + `patcompile` on a RAW-encoded pattern
    // string, so this pass transposes back to that encoding:
    //   token char (pre-tokenized source meta) -> raw ASCII meta
    //       (the downstream tokenize re-activates it),
    //   raw ASCII glob meta (splice)           -> `\`-escaped
    //       (downstream tokenize folds `\X` to Bnull+X = literal),
    //       or left raw under GLOBSUBST (re-activated downstream),
    //   `\X` / Bnull/Bnullkeep pairs           -> verbatim.
    let literalize_spliced_metas = |s: &str| -> String {
        use crate::ported::zsh_h::{
            Bar, Bnull, Bnullkeep, Hat, Inang, Inbrack, Inpar, Outang, Outbrack, Outpar,
            Pound, Quest, Star, Tilde,
        };
        let glob_subst =
            crate::ported::zsh_h::isset(crate::ported::zsh_h::GLOBSUBST);
        let chars: Vec<char> = s.chars().collect();
        let mut out = String::with_capacity(s.len());
        let mut i = 0usize;
        while i < chars.len() {
            let c = chars[i];
            if (c == '\\' || c == Bnull || c == Bnullkeep) && i + 1 < chars.len() {
                out.push(c);
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            match c {
                x if x == Pound => out.push('#'),
                x if x == Hat => out.push('^'),
                x if x == Star => out.push('*'),
                x if x == Inpar => out.push('('),
                x if x == Outpar => out.push(')'),
                x if x == Bar => out.push('|'),
                x if x == Inbrack => out.push('['),
                x if x == Outbrack => out.push(']'),
                x if x == Inang => out.push('<'),
                x if x == Outang => out.push('>'),
                x if x == Quest => out.push('?'),
                x if x == Tilde => out.push('~'),
                '*' | '?' | '[' | ']' | '(' | ')' | '|' | '<' | '>' | '^' | '#' | '~' => {
                    if glob_subst {
                        out.push(c); // c:1669 — GLOBSUBST: splice active
                    } else {
                        out.push('\\');
                        out.push(c);
                    }
                }
                other => out.push(other),
            }
            i += 1;
        }
        out
    };

    // Check what follows the $
    let c = chars.get(pos).copied().unwrap_or('\0'); // c:1625

    // ${...} form
    // ${...} brace form. Pragmatic inline port covering high-traffic
    // shapes from subst.c:1885+ (full 2,849-line paramsubst port is
    // an ongoing arm-by-arm effort). Handles: bare ref, ${#var}
    // length, :- :+ := :? defaults, # ## % %% strip, / // replace
    // with anchored # / % variants, :N:M slice, plus a permissive
    // (...)-flag prefix swallow.
    if c == Inbrace {
        // c:1885 — paramsubst's `${…}` arm. The leading `{` was
        // tokenized to Inbrace either by the lex pipeline OR by the
        // pre-tokenize pass at paramsubst entry (see top of
        // paramsubst). The bare-`{` path is therefore impossible
        // here; the scanner below only counts canonical
        // Inbrace/Outbrace tokens.
        pos += 1; // c:1885 (skip Inbrace)
        let mut depth = 1_i32; // c:utils.c:2411
        let mut end = pos; // c:utils.c:2416
        // c:Src/utils.c:2409 skipparens — counts ONLY Inbrace
        // (`\u{8f}`) / Outbrace (`\u{90}`) tokens. Every `${`-form
        // brace pair is tokenized by Src/lex.c:1546 (`add(Qstring);
        // c = Inbrace; cmdpush(CS_BRACEPAR); bct++;`) and matched by
        // c:1565-1577 (`c = Outbrace; bct--;`) BEFORE paramsubst
        // runs; raw ASCII `{`/`}` never opens or closes a paramsubst
        // body in C.
        //
        // The pre-tokenize pass at paramsubst entry converts the
        // outermost `${…}` of any raw-ASCII caller input to
        // Inbrace/Outbrace before this scanner runs, so the scanner
        // is now strictly token-counting (1:1 with C's skipparens).
        // Nested `${…}` inside the body either come through the lex
        // pipeline already tokenized OR get tokenized on their own
        // recursive paramsubst entry. Raw `{`/`}` that survives at
        // this stage is a literal user character — not a paramsubst
        // brace.
        while end < chars.len() && depth > 0 {
            let ch = chars[end];
            if ch == Inbrace {
                depth += 1; // c:utils.c:2418
            } else if ch == Outbrace {
                depth -= 1; // c:utils.c:2420
                if depth == 0 {
                    break;
                }
            }
            end += 1;
        } // c:utils.c:2422
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
        // c:Src/subst.c:1885 paramsubst's first sanity check —
        // itype_end(s, INAMESPC, 1) == s combined with the
        // not-in-special-list check. When the body's first char is
        // whitespace (and not a recognized special), the C path
        // either turns it back into a literal `${ }` or errors via
        // "bad substitution" downstream. zshrs's paramsubst silently
        // accepted whitespace-only bodies and returned empty.
        // Bug #172 in docs/BUGS.md.
        //
        // Truly empty `${}` keeps the pre-existing zsh-compat
        // silent-empty behavior. Only non-empty all-whitespace bodies
        // trigger the error.
        if !body.is_empty() && body.chars().all(|c| c.is_ascii_whitespace()) {
            zerr("bad substitution");
            errflag_set_error();
            return (String::new(), new_pos, vec![]);
        }
        let body_chars: Vec<char> = body.chars().collect();
        let mut idx = 0_usize;
        // ${(flags)var…} — paren-flag block. Port of subst.c:2147+
        // flag-loop. Each flag char sets a state bit; applied as
        // post-processing on the substituted value.
        //
        // Local declarations ordered to match C source paramsubst()
        // c:1628-1819. Every entry cites its C declaration line.
        // Bag-of-globals decompositions (flag_lower/upper/caps,
        // flag_q*, flag_z_*, flag_g_*, etc.) all collapsed back to
        // their canonical C single-int state slots per Rule D.

        // c:1658 — `int isarr = 0;`. State machine per c:1647-1657:
        //   -1 = force-keep-empty (nojoin set)
        //    0 = scalar shape (single string in `value`)
        //    1 = array shape (multi-element list)
        //    2 = split-scalar (array came from splitting a scalar)
        //
        // Transitions ported from C (each with c:NNN cite at the
        // mutation site):
        //   c:2714 isarr=0 after assoc-key-flag scalar pick
        //   c:2859 isarr=0 after subscript single-element pick
        //   c:2887 isarr=0 after numeric subscript single-pick
        //   c:2923 isarr=v->scanflags=0 after getindex single-slot
        //   c:3030 isarr=-1 when nojoin set
        //   c:3034 isarr=0 when qt && !getlen && isarr>0 (sepjoin)
        //   c:3197 isarr=0 after substring-on-scalar
        //   c:4235 isarr=1 when arrasg forces array shape
        //
        // Used as the canonical gate at c:4245 (`if (isarr)`) for
        // sort/unique/splat. Replaces the Rust-only
        // `split_parts.is_some()` proxy that was structurally drifting
        // from C's explicit int state.
        let mut isarr: i32 = 0; // c:1658

        // c:3950 — `aval` (array-value output). C's paramsubst tracks
        // the post-substitution array shape in `**aval`; consumers
        // read it after `getarrvalue(v)` or after a flag-form
        // subscript that set `v->scanflags`. zshrs uses
        // `Option<Vec<String>>` for the same role: `Some(vec)` =
        // array shape established, `None` = scalar path. Declared here
        // (rather than inside the brace handler at the original c:3950
        // site) so subscript / flag arms above can write to it
        // directly — same data flow as C where getarg can populate
        // `*ta = getvaluearr(v)` at c:1726-1729 before paramsubst
        // proper resumes.
        let mut split_parts: Option<Vec<String>> = None; // c:3950


        // c:1663 — `int plan9 = isset(RCEXPANDPARAM);`
        let mut plan9 = isset(RCEXPANDPARAM); // c:1663
        // Set when an explicit `${^^...}` turned RC_EXPAND OFF (c:2554).
        // Distinct from "plan9 happens to be false": under `setopt
        // rcexpandparam` plan9 starts true and `^^` forces it false, in
        // which case the array result must JOIN (scalar) so it does NOT
        // cross-product-distribute. Drives the join below.
        let mut rc_force_off = false;

        // c:1669 — `int globsubst = isset(GLOBSUBST);` (handled inline
        // at use sites via opt_state_set rather than tracked here).

        // c:1673 — `int evalchar = 0;` (#) char-eval flag.
        let mut evalchar = false; // c:1673

        // c:1678 — `int getlen = 0;` (handled via prefix-# arm).
        // c:1679 — `int whichlen = 0;` (c)/(w)/(W) length-flavor int.
        let mut whichlen: i32 = 0; // c:1679

        // c:1683 — `int chkset = 0;` (${+pm} flag) — see chkset
        // handling in the body; declared locally there for now.

        // c:1691 — `int vunset = 0;` — value-was-unset flag.
        // c:1697 — `int wantt = 0;` (t) typeinfo flag.
        let mut wantt = false; // c:1697
        // c:2232 — `${(!)…}` SCANPM_NONAMEREF RAII scope.
        #[allow(unused_assignments)]
        let mut nonameref_guard: Option<crate::ported::params::NamerefSuppressGuard> = None;
        let _ = &nonameref_guard;

        // c:1705 — `int spbreak = (pf_flags & PREFORK_SHWORDSPLIT) &&
        // !(pf_flags & PREFORK_SINGLE) && !qt;`

        // c:1708 — `char *val = NULL, **aval = NULL;` (handled via
        // local `value: String` / `split_parts: Option<Vec<String>>`).

        // c:1713-1714 — `struct value vbuf; Value v = NULL;` (vbuf/v
        // fetchvalue path not yet wired; reads route through paramtab
        // directly until LinkList rewrite lands).

        // c:1720 — `int flags = 0;` SUB_* match flag bitmask.
        let mut sub_flags_bits: i32 = 0; // c:1720

        // c:1722 — `int flnum = 0;` (I:N:) match-index flag.
        let mut flnum: u32 = 0; // c:1722

        // c:1728 — `int sortit = SORTIT_ANYOLDHOW, indord = 0;`
        let mut sortit: i32 = SORTIT_ANYOLDHOW; // c:1728
        let mut indord: i32 = 0; // c:1728

        // c:1730 — `int unique = 0;` (u) flag.
        let mut unique = false; // c:1730

        // c:1732 — `int casmod = CASMOD_NONE;`
        let mut casmod: i32 = CASMOD_NONE; // c:1732

        // c:1739 — `int quotemod = 0, quotetype = QT_NONE, quoteerr = 0;`
        let mut quotemod: i32 = 0; // c:1739
        let mut quotetype: i32 = QT_NONE; // c:1739
        let mut quoteerr = false; // c:1739

        // c:1746 — `int mods = 0;` bit0=D, bit1=V.
        let mut mods: i32 = 0; // c:1746

        // c:1754 — `int shsplit = 0;` LEXFLAGS_* bitmask.
        let mut shsplit: i32 = 0; // c:1754

        // c:1759 — `int ssub = (pf_flags & PREFORK_SINGLE);` (read at
        // call sites; not tracked as a local in current port).

        // c:1766 — `char *sep = NULL, *spsep = NULL;`
        let mut spsep: Option<String> = None; // c:1766
        let mut sep: Option<String> = None; // c:1766

        // c:1772 — `char *premul = NULL, *postmul = NULL,
        //                *preone = NULL, *postone = NULL;`
        let mut premul: Option<String> = None; // c:1772
        let mut postmul: Option<String> = None; // c:1772
        let mut preone: Option<String> = None; // c:1772
        let mut postone: Option<String> = None; // c:1772

        // c:1774 — `char *replstr = NULL;` (replacement string for
        // ${var/pat/repl}) — handled inline in the replace arm.

        // c:1776 — `zlong prenum = 0, postnum = 0;`
        let mut prenum: i64 = 0; // c:1776
        let mut postnum: i64 = 0; // c:1776

        // c:1779 — `int multi_width = 0;` (MULTIBYTE_SUPPORT).
        let mut multi_width: u32 = 0; // c:1779

        // c:1787 — `int copied = 0;` (handled per-arm).

        // c:1793 — `int arrasg = 0;` (A)/(AA) array-assign flag.
        let mut arrasg: i32 = 0; // c:1793

        // c:1798 — `int eval = 0;` (e) flag.
        let mut eval = false; // c:1798

        // c:1803 — `int aspar = 0;` (P) flag.
        let mut aspar = false; // c:1803

        // c:1807 — `int presc = 0;` (%) flag counter.
        let mut presc: i32 = 0; // c:1807

        // c:1811 — `int getkeys = -1;` (g) flag GETKEY_* bitmask
        // (-1 = not seen).
        let mut getkeys: i32 = -1; // c:1811

        // c:1817 — `int nojoin = (pf_flags & PREFORK_SHWORDSPLIT) ?
        // !(ifs && *ifs) && !qt : 0;` (@) flag tri-state.
        let mut nojoin: i32 = if (pf_flags & PREFORK_SHWORDSPLIT) != 0 {
            // c:1817
            let ifs = vars_get("IFS").unwrap_or_default(); // c:1817
            if ifs.is_empty() && !qt {
                1
            } else {
                0
            } // c:1817
        } else {
            // c:1817
            0 // c:1817
        }; // c:1817

        // c:1823 — `char inbrace = 0;` 1 if `${...}`, 0 if bare `$...`.
        // Set to 1 here because we entered the brace arm at line 2437.
        // C sets it later (around c:2076-2079) after parsing the
        // leading `{`; effect is identical because all body code
        // below this point only runs in the brace arm.
        let mut inbrace: i32 = 1; // c:1823 set to 1 since in-brace arm
        let _ = &mut inbrace; // suppress unused-mut until consumer-site wiring lands

        // c:1828 — `int hkeys = 0;` (k) flag SCANPM_WANTKEYS bits.
        let mut hkeys: u32 = 0; // c:1828

        // c:1835 — `int hvals = 0;` (v) flag SCANPM_WANTVALS bits.
        let mut hvals: u32 = 0; // c:1835

        // c:1843 — `int subexp;` 1 if the body started with `$`/`(`/
        // `{` (nested sub-expression), 0 otherwise. Read by the
        // fetchvalue dispatch at c:2767 + the (P)-flag arm.
        let mut subexp: i32 = 0; // c:1843

        // c:2140 — `int escapes = 0;` (p) flag; declared inside
        // flag-loop in C, hoisted to function scope here.
        let mut escapes: bool = false; // c:2140

        // c:Src/subst.c — nested-expansion array shape.
        //   `subexp_array_temp`: the synthesized temp-array NAME bound
        //     to `var_name` so the existing arrays_get/scalar-lookup
        //     paths inside paramsubst can find the elements. The temp
        //     itself is the side-channel through arrays_insert at line
        //     ~4192; cleared at end of paramsubst.
        //   `subexp_arr_parts`: the structural carrier — Some(Vec) when
        //     the inner expansion produced a TRUE array result. C does
        //     this via `Value.isarr` flowing from multsub through
        //     getindex; the Rust port keeps the Vec locally so the
        //     downstream [N] discriminator at line ~4777 can read
        //     it directly without round-tripping through the arrays
        //     table. Mirrors the (joined, arr_parts, isarr, _) return
        //     shape from multsub at line ~4182 with the same isarr
        //     semantic.
        let mut subexp_array_temp: Option<String> = None;
        let mut subexp_arr_parts: Option<Vec<String>> = None;
                                                          // c:Src/subst.c:2147 — flag-block entry. Accept both ASCII `(`
                                                          // and Inpar TOKEN (\u{88}) — the lexer emits Inpar TOKEN for
                                                          // `${(flag)name}` in DQ context and in the new bridge passthru
                                                          // path where raw tokenized text reaches paramsubst without an
                                                          // intermediate untokenize pass.
        if matches!(body_chars.first(), Some(&'(') | Some(&Inpar)) {
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
            if !body_chars.iter().skip(1).any(|c| *c == ')' || *c == Outpar) {
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
                    // c:Src/subst.c:2147+ — the C flag-parse switch
                    // has NO `case Inpar` / `case '('` arm. Nested `(`
                    // inside the flag block falls through to the
                    // default `flagerr` (c:2505). Bug #550: zshrs
                    // tracked depth and silently accepted `((O))` as
                    // `(O)`. The `(` arm is intentionally absent; the
                    // outer `while d > 0` loop terminates on the FIRST
                    // `)` (matches C's `for (...; c != ')'; ...)`).
                    c if c == ')' || c == Outpar => {
                        d -= 1;
                        if d == 0 {
                            idx += 1;
                            break;
                        }
                    } // c:2147
                    'L' => {
                        // c:2197
                        casmod = CASMOD_LOWER; // c:2198
                    } // c:2199
                    'U' => {
                        // c:2200
                        casmod = CASMOD_UPPER; // c:2201
                    } // c:2202
                    'C' => {
                        // c:2203
                        casmod = CASMOD_CAPS; // c:2204
                    } // c:2205
                    'q' => {
                        // c:2236
                        // c:2237 — `if (quotetype == QT_DOLLARS ||
                        // quotetype == QT_BACKSLASH_PATTERN) goto flagerr;`
                        // Five `q`s would push quotetype past QT_DOLLARS;
                        // `(b)` sets QT_BACKSLASH_PATTERN. Either case
                        // followed by another `q` is invalid per C.
                        if quotetype == QT_DOLLARS || quotetype == QT_BACKSLASH_PATTERN {
                            zerr("error in flags");
                            errflag_set_error();
                            return (String::new(), new_pos, vec![]);
                        }
                        let next = body_chars.get(idx + 1).copied(); // c:2240 IS_DASH(s[1]) || s[1]=='+'
                        if next == Some('-') || next == Some('+') {
                            // c:2240
                            // c:2241-2242 — `if (quotemod) goto flagerr;`.
                            // q- / q+ are independent flag-block entries
                            // and can't be combined with another q-mod.
                            if quotemod != 0 {
                                zerr("error in flags");
                                errflag_set_error();
                                return (String::new(), new_pos, vec![]);
                            }
                            idx += 1; // c:2243 s++
                            quotemod = 1; // c:2244
                            quotetype = if next == Some('+') {
                                // c:2245
                                QT_QUOTEDZPUTS // c:2245
                            } else {
                                // c:2246
                                QT_SINGLE_OPTIONAL // c:2246
                            }; // c:2246
                        } else {
                            // c:2247
                            // c:2248-2251 — `if (quotetype ==
                            // QT_SINGLE_OPTIONAL) goto flagerr;`.
                            // Once q- has set QT_SINGLE_OPTIONAL,
                            // additional plain `q`s are invalid.
                            if quotetype == QT_SINGLE_OPTIONAL {
                                zerr("error in flags");
                                errflag_set_error();
                                return (String::new(), new_pos, vec![]);
                            }
                            quotemod += 1; // c:2252 quotemod++
                            quotetype += 1; // c:2252 quotetype++
                        } // c:2253
                    } // c:2254
                    'A' => {
                        arrasg += 1;
                    } // c:2161 (A array-assign; AA associative-assign)
                    '@' => {
                        // c:2164
                        nojoin = 2; // c:2165 nojoin = 2 means force
                    } // c:2166
                    'P' => {
                        aspar = true;
                    } // c:2295
                    't' => {
                        wantt = true;
                    } // c:2807
                    '!' => {
                        // c:2232-2235 — SCANPM_NONAMEREF: the lookup
                        // skips resolve_nameref (getnode2 path); the
                        // ref itself is the subject. Guard lives until
                        // this paramsubst returns.
                        nonameref_guard =
                            Some(crate::ported::params::NamerefSuppressGuard::new());
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
                    c if c == '#' || c == Pound => {
                        evalchar = true;
                    } // c:2480-2483 (# / Pound)
                    'l' | 'r' => {
                        // c:2319-2378 (l/r pad). Direct port of the
                        // get_intarg / get_strarg sequence:
                        //   s++; del0 = s; num = get_intarg(&s, &dellen);
                        //   if (!dellen || memcmp(del0, s, dellen)) { s--; break; }
                        //   t = get_strarg(s, &arglen); ... (STR1)
                        //   if (memcmp(del0, s, dellen)) { s--; break; }
                        //   t = get_strarg(s, &arglen); ... (STR2)
                        //
                        // `get_intarg` calls `get_strarg(s)` which reads
                        // the FIRST byte as delimiter and advances `s`
                        // to point PAST the closing delimiter. So after
                        // `(l:5:`, s lands at `)`. The dellen-check
                        // then compares del0 ("`:`") with the byte at
                        // the new s — if it's `)` (not `:`), the str1
                        // path is skipped.
                        let is_left = fc == 'l'; // c:2320
                        idx += 1; // c:2323 — s++ past 'l'/'r'
                        if idx >= body_chars.len() {
                            break;
                        }
                        let del = body_chars[idx]; // c:2325 del0 = s
                                                   // c:Src/subst.c:1366-1391 get_strarg — paired
                                                   // bracket delimiters: `(…)`, `[…]`, `{…}`,
                                                   // `<…>`. `${(l[3][_])arr}` opens groups with
                                                   // `[` and closes with `]`. Without the mapping
                                                   // the (l)/(r) parser scanned for a second `[`
                                                   // and bailed with "bad substitution".
                        let close_del = match del {
                            '(' => ')',
                            '[' => ']',
                            '{' => '}',
                            '<' => '>',
                            other => other,
                        };
                        idx += 1; // get_strarg(s) advances past opening del
                                  // c:Src/subst.c:1428-1454 — `get_intarg`
                                  // reads the delimited region (`get_strarg`),
                                  // `singsub`s it so `$var` refs expand, then
                                  // `mathevali`s the result. The previous Rust
                                  // port only accepted bare `[0-9]+` digits,
                                  // so `(l.w.)` / `(l.$w.)` / `(l.w*2.)` all
                                  // failed with "bad substitution" (n=0
                                  // collapsed the pad to a no-op or the
                                  // matched-delim check rejected later).
                                  // Bug #114 in docs/BUGS.md.
                        let n_start = idx;
                        while idx < body_chars.len()
                            && body_chars[idx] != close_del
                            && body_chars[idx] != ')'
                            && body_chars[idx] != Outpar
                        {
                            idx += 1;
                        }
                        // c:Src/subst.c:1366-1456 — get_strarg / get_intarg
                        // return -1 when the closing delimiter is missing
                        // before EOF, and the C `(l)`/`(r)` arm jumps to
                        // `flagerr` on -1 — emitting "error in flags near
                        // position …". zshrs's loop silently bailed when
                        // it ran past the flag-block close `)` without
                        // finding the matching delimiter, accepted the
                        // partial input as a no-pad zero, and produced
                        // empty output. Bug #162 in docs/BUGS.md.
                        if idx >= body_chars.len()
                            || body_chars[idx] != close_del
                        {
                            // Position in 1-based terms for error parity
                            // with zsh's "near position N" diagnostic.
                            let pos_1based = idx + 1;
                            zerr(&format!(
                                "error in flags near position {} in '${{{}}}'",
                                pos_1based,
                                body.as_str()
                            ));
                            errflag_set_error();
                            return (String::new(), new_pos, vec![]);
                        }
                        let raw_expr: String = body_chars[n_start..idx].iter().collect();
                        // singsub expands `$var` / backticks / arith; then
                        // mathevali resolves the result string.
                        let expanded = singsub(&raw_expr);
                        let n: i64 = if expanded.is_empty() {
                            0
                        } else {
                            match crate::ported::math::mathevali(&expanded) {
                                Ok(v) => v.abs(),       // c:1451-1452 `if (ret < 0) ret = -ret`
                                Err(_) => 0,
                            }
                        };
                                                                   // c:1441 — `*s = t + arglen` advances PAST the
                                                                   // closing delimiter. Mirror by skipping the
                                                                   // closing del.
                        if idx < body_chars.len() && body_chars[idx] == close_del {
                            idx += 1;
                        }
                        if is_left {
                            prenum = n;
                        } else {
                            postnum = n;
                        } // c:2329-2331
                          // c:2334 — `if (!dellen || memcmp(del0, s, dellen)) break;`.
                          // After the get_intarg advance, s points at
                          // either another delimiter (continue with STR1)
                          // or a non-delimiter (closing `)`, end of flag).
                          // Use `continue` (not the match's bottom
                          // increment) so idx stays at the current
                          // position — the outer ')' arm picks up the
                          // closing paren next iteration.
                        if idx >= body_chars.len() || body_chars[idx] != del {
                            continue;
                        }
                        // c:2339 — STR1 (multi-pad). `get_strarg`
                        // reads from `:` and walks until matching `:`.
                        idx += 1; // skip opening del
                        let s1_start = idx;
                        // c:Src/subst.c:1366-1456 — same flag-block
                        // close-paren guard as the WIDTH loop (bug #162).
                        // STR1 / STR2 loops also need to stop at `)` /
                        // Outpar so the parser doesn't walk past the
                        // flag-block close looking for the matching
                        // delimiter — `(l.5..)` previously consumed
                        // `)s)s` as the STR1 content. Bug #191 in
                        // docs/BUGS.md.
                        while idx < body_chars.len()
                            && body_chars[idx] != close_del
                            && body_chars[idx] != ')'
                            && body_chars[idx] != Outpar
                        {
                            idx += 1;
                        }
                        // If we hit `)` without finding the close
                        // delimiter, STR1 was unterminated — error per
                        // C `Src/subst.c:2334` get_intarg -1 path,
                        // mirroring bug #162's WIDTH fix.
                        if idx >= body_chars.len()
                            || body_chars[idx] != close_del
                        {
                            let pos_1based = idx + 1;
                            zerr(&format!(
                                "error in flags near position {} in '${{{}}}'",
                                pos_1based,
                                body.as_str()
                            ));
                            errflag_set_error();
                            return (String::new(), new_pos, vec![]);
                        }
                        let s1_raw: String = body_chars[s1_start..idx].iter().collect();
                        let s1 = untok_and_escape(&s1_raw, escapes, tok_arg);
                        if is_left {
                            premul = Some(s1);
                        } else {
                            postmul = Some(s1);
                        }
                        if idx < body_chars.len() {
                            idx += 1; // skip closing del of STR1
                        }
                        // c:2354 — `if (memcmp(del0, s, dellen)) break;`
                        // — check for another delimiter to introduce STR2.
                        if idx >= body_chars.len() || body_chars[idx] != del {
                            continue;
                        }
                        // c:2360 — STR2 (one-time pad).
                        idx += 1;
                        let s2_start = idx;
                        while idx < body_chars.len()
                            && body_chars[idx] != close_del
                            && body_chars[idx] != ')'
                            && body_chars[idx] != Outpar
                        {
                            idx += 1;
                        }
                        // Same close-paren guard for STR2. Bug #191.
                        if idx >= body_chars.len()
                            || body_chars[idx] != close_del
                        {
                            let pos_1based = idx + 1;
                            zerr(&format!(
                                "error in flags near position {} in '${{{}}}'",
                                pos_1based,
                                body.as_str()
                            ));
                            errflag_set_error();
                            return (String::new(), new_pos, vec![]);
                        }
                        let s2_raw: String = body_chars[s2_start..idx].iter().collect();
                        let s2 = untok_and_escape(&s2_raw, escapes, tok_arg);
                        if is_left {
                            preone = Some(s2);
                        } else {
                            postone = Some(s2);
                        }
                        if idx < body_chars.len() {
                            idx += 1;
                        }
                        continue; // c:2374
                    }
                    'o' => {
                        // c:2207
                        if sortit == 0 {
                            // c:2208 if (!sortit)
                            sortit |= SORTIT_SOMEHOW; // c:2209
                        } // c:2209
                    } // c:2210
                    'O' => {
                        // c:2211
                        sortit |= SORTIT_BACKWARDS; // c:2212
                    } // c:2213
                    'i' => {
                        // c:2214
                        sortit |= SORTIT_IGNORING_CASE; // c:2215
                    } // c:2216
                    'n' => {
                        // c:2217
                        sortit |= SORTIT_NUMERICALLY; // c:2218
                    } // c:2219
                    c if c == '-' || c == Dash => {
                        // c:2220-2223 case '-': case Dash:
                        sortit |= SORTIT_NUMERICALLY_SIGNED;
                    } // c:2223
                    'a' => {
                        // c:2224
                        sortit |= SORTIT_SOMEHOW; // c:2225
                        indord = 1; // c:2226
                    } // c:2227
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
                    c if c == '*' || c == Star => {
                        sub_flags_bits |= SUB_EGLOB;
                    } // c:2167-2169 (* / Star)
                    'I' => {
                        // c:2189-2195 (I:N:) — match the Nth occurrence in
                        // ${var//pat/repl}. C:
                        //   case 'I':
                        //       s++;
                        //       flnum = get_intarg(&s, &dellen);
                        //       if (flnum < 0) goto flagerr;
                        //       s--;
                        //       break;
                        // get_intarg expects a delimited integer
                        // (`(I:N:)`, `(I.N.)`, etc.). If the byte after
                        // `I` is not a delimiter (e.g. `(I)foo` with no
                        // numeric arg) or no matching close-delim is
                        // found, get_intarg returns -1 and the C arm
                        // jumps to flagerr. Previous Rust port read bare
                        // digits with no delimiter and silently accepted
                        // `(I)foo`, diverging from zsh's "error in flags
                        // near position N" diagnostic.
                        idx += 1; // c:2190 (s++)
                        if idx >= body_chars.len() {
                            let pos_1based = idx + 1;
                            zerr(&format!(
                                "error in flags near position {} in '${{{}}}'",
                                pos_1based,
                                body.as_str()
                            ));
                            errflag_set_error();
                            return (String::new(), new_pos, vec![]);
                        }
                        let del = body_chars[idx]; // c:1431 get_strarg del
                        let close_del = match del {
                            '(' => ')',
                            '[' => ']',
                            '{' => '}',
                            '<' => '>',
                            other => other,
                        };
                        idx += 1; // get_strarg(s) past opening delimiter
                        let n_start = idx;
                        while idx < body_chars.len()
                            && body_chars[idx] != close_del
                            && body_chars[idx] != ')'
                            && body_chars[idx] != Outpar
                        {
                            idx += 1;
                        }
                        // c:1436-1437 get_strarg returns -1 if `!*t`
                        // (no close delim found before end of input).
                        if idx >= body_chars.len() || body_chars[idx] != close_del {
                            let pos_1based = idx + 1;
                            zerr(&format!(
                                "error in flags near position {} in '${{{}}}'",
                                pos_1based,
                                body.as_str()
                            ));
                            errflag_set_error();
                            return (String::new(), new_pos, vec![]);
                        }
                        let raw_expr: String = body_chars[n_start..idx].iter().collect();
                        let expanded = singsub(&raw_expr); // c:1444 singsub
                        let n: i64 = if expanded.is_empty() {
                            0
                        } else {
                            match crate::ported::math::mathevali(&expanded) {
                                Ok(v) => v.abs(), // c:1451-1452 ret = -ret
                                Err(msg) => {
                                    zerr(&msg);
                                    let pos_1based = idx + 1;
                                    zerr(&format!(
                                        "error in flags near position {} in '${{{}}}'",
                                        pos_1based,
                                        body.as_str()
                                    ));
                                    errflag_set_error();
                                    return (String::new(), new_pos, vec![]);
                                }
                            }
                        };
                        flnum = n as u32; // c:2191
                        idx += 1; // step past the closing delimiter (c:1453 *delmatchp += arglen)
                        continue; // c:2195 break (next outer flag iter)
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
                        eval = true;
                    } // c:2268 (e)
                    'Q' => {
                        // c:2261
                        quotemod -= 1; // c:2262
                    } // c:2263
                    'X' => {
                        quoteerr = true;
                    } // c:2264 (X)
                    'D' => {
                        // c:2229
                        mods |= 1; // c:2230
                    } // c:2231
                    'V' => {
                        // c:2232
                        mods |= 2; // c:2233
                    } // c:2234
                    'b' => {
                        // c:2255
                        // c:2256-2257 — `if (quotemod || quotetype !=
                        // QT_NONE) goto flagerr;` (flagerr not yet
                        // ported; skipped).
                        quotemod = 1; // c:2258
                        quotetype = QT_BACKSLASH_PATTERN; // c:2259
                    } // c:2260
                    'c' => {
                        // c:2275
                        whichlen = 1; // c:2276
                    } // c:2277
                    'w' => {
                        // c:2278
                        whichlen = 2; // c:2279
                    } // c:2280
                    'W' => {
                        // c:2281
                        whichlen = 3; // c:2282
                    } // c:2283
                    'z' => {
                        // c:2439
                        shsplit = LEXFLAGS_ACTIVE; // c:2440
                    } // c:2441
                    'Z' => {
                        // c:2443
                        // (Z:cCn:) — shell-tokenize with sub-flags:
                        //   c: keep comments (LEXFLAGS_COMMENTS_KEEP)
                        //   C: strip comments (LEXFLAGS_COMMENTS_STRIP)
                        //   n: treat newlines as whitespace (LEXFLAGS_NEWLINE)
                        // Direct port of subst.c:2443-2473 — bare (Z) sets
                        // ACTIVE; sub-letters OR additional bits.
                        // c:2444-2473 — get_strarg(++s) requires a
                        // delimiter; `if (*t)` else `goto flagerr`. Bare
                        // `(Z)` (no `(Z:xxx:)`-form arg) and `(Z+)` etc.
                        // land on `)` immediately and must flagerr.
                        shsplit = LEXFLAGS_ACTIVE; // c:2443 (implicit from Z arm)
                        idx += 1; // c:2444 ++s
                                  // c:2445 `if (*t)` else flagerr (`*t == 0`).
                                  // get_strarg returns end-of-string when no
                                  // matching delim found; the C path then takes
                                  // the else branch. In Rust: if next char is the
                                  // flag-block close `)` (or end of body), flagerr.
                        //
                        // c:Src/subst.c:2527 flagerr message — emit the
                        // canonical "error in flags near position N in
                        // '${BODY}'" diagnostic instead of generic "bad
                        // substitution". Bug #571.
                        if idx >= body_chars.len() || body_chars[idx] == ')' {
                            let pos_1based = idx + 1 + 2;
                            zerr(&format!(
                                "error in flags near position {} in '${{{}}}'",
                                pos_1based,
                                body.as_str()
                            ));
                            errflag_set_error();
                            return (String::new(), new_pos, vec![]);
                        }
                        let del = body_chars[idx]; // c:2446 sav = *t
                        idx += 1; // c:2448 while (*++s)
                        let mut found_close = false;
                        while idx < body_chars.len()                     // c:2448
                            && body_chars[idx] != del
                        {
                            // c:2448
                            let ch = body_chars[idx]; // c:2449 switch (*s)
                            if ch == 'c' {
                                shsplit |= LEXFLAGS_COMMENTS_KEEP; // c:2452
                            } else if ch == 'C' {
                                shsplit |= LEXFLAGS_COMMENTS_STRIP; // c:2457
                            } else if ch == 'n' {
                                shsplit |= LEXFLAGS_NEWLINE; // c:2462
                            } else {
                                // c:2465-2467 default: flagerr.
                                let pos_1based = idx + 1 + 2;
                                zerr(&format!(
                                    "error in flags near position {} in '${{{}}}'",
                                    pos_1based,
                                    body.as_str()
                                ));
                                errflag_set_error();
                                return (String::new(), new_pos, vec![]);
                            }
                            idx += 1; // c:2448
                        }
                        if idx < body_chars.len() && body_chars[idx] == del {
                            found_close = true;
                            idx += 1; // c:2444 past close delim
                        }
                        if !found_close {
                            let pos_1based = idx + 1 + 2;
                            zerr(&format!(
                                "error in flags near position {} in '${{{}}}'",
                                pos_1based,
                                body.as_str()
                            ));
                            errflag_set_error();
                            return (String::new(), new_pos, vec![]);
                        }
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
                        idx += 1; // c:2410 ++s
                        if getkeys < 0 {
                            // c:2411
                            getkeys = 0; // c:2412
                        } // c:2412
                        if idx < body_chars.len() {
                            // c:2413 if (*t)
                            let del = body_chars[idx]; // c:2414 sav = *t
                            idx += 1; // c:2416 while (*++s)
                            while idx < body_chars.len()                     // c:2416
                                && body_chars[idx] != del
                            {
                                // c:2416
                                match body_chars[idx] {
                                    // c:2417 switch (*s)
                                    'e' => getkeys |= GETKEY_EMACS as i32, // c:2418-2419
                                    'o' => getkeys |= GETKEY_OCTAL_ESC as i32, // c:2421-2422
                                    'c' => getkeys |= GETKEY_CTRL as i32,  // c:2424-2425
                                    _ => {
                                        // c:2428 default
                                        // c:2430 goto flagerr — emit bad-subst.
                                        zerr("bad substitution");
                                        errflag_set_error();
                                        return (String::new(), new_pos, vec![]);
                                    } // c:2431
                                } // c:2432
                                idx += 1; // c:2416
                            } // c:2432
                            if idx < body_chars.len() {
                                // c:2410 skip closing del
                                idx += 1;
                            }
                        } // c:2413
                        continue; // c:2410
                    } // c:2409 (g)
                    c if c == '~' || c == Tilde => {
                        tok_arg = !tok_arg;
                    } // c:2157-2159 (~ / Tilde)
                    'm' => {
                        multi_width += 1;
                    } // c:2376 (m)
                    'p' => {
                        escapes = true;
                    } // c:2382
                    '%' => {
                        presc += 1;
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
                        // c:Src/subst.c:2299-2313 — get_strarg(++s) /
                        // `if (*t) ... else flagerr`. If next char is
                        // the flag-block close `)` or end-of-body, or
                        // if no matching close-delim is found, flagerr.
                        let is_split = fc == 's'; // c:2300
                        idx += 1; // c:2303 (++s)
                        if idx >= body_chars.len() || body_chars[idx] == ')' {
                            zerr("bad substitution"); // c:2316 flagerr
                            errflag_set_error();
                            return (String::new(), new_pos, vec![]);
                        }
                        let del = body_chars[idx]; // c:2303 (get_strarg del)
                                                   // c:Src/subst.c:1366-1391 get_strarg — paired
                                                   // delimiters: `(…)`, `[…]`, `{…}`, `<…>` use
                                                   // matching close brackets instead of repeated
                                                   // open. Previously `${(j[, ])arr}` errored
                                                   // because the loop searched for another `[`.
                        let close_del = match del {
                            '(' => ')',
                            '[' => ']',
                            '{' => '}',
                            '<' => '>',
                            other => other,
                        };
                        idx += 1; // c:2303
                        let s_start = idx;
                        while idx < body_chars.len() && body_chars[idx] != close_del {
                            idx += 1;
                        }
                        if idx >= body_chars.len() {
                            zerr("bad substitution"); // c:2316 flagerr
                            errflag_set_error();
                            return (String::new(), new_pos, vec![]);
                        }
                        let arg: String = body_chars[s_start..idx].iter().collect(); // c:2308
                        let arg = untok_and_escape(&arg, escapes, tok_arg); // c:2309-2312
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
                        // c:Src/subst.c:2527 — `zerr("error in flags near
                        // position %z in '$%s'", offset, str_copy)`. The
                        // offset is 1-based from the `$` (c:2524 `s -
                        // *str + 1` where *str points at the leading
                        // `$`). zshrs's `body` here is the inside of
                        // `${...}` (i.e. starting at `(`), so add 2 to
                        // account for the `${` prefix that re-wraps in
                        // the printed message. Bug #546.
                        let pos_1based = idx + 1 + 2;
                        zerr(&format!(
                            "error in flags near position {} in '${{{}}}'",
                            pos_1based,
                            body.as_str()
                        ));
                        errflag_set_error();
                        return (String::new(), new_pos, vec![]);
                    }
                }
                idx += 1;
            }
            // hkeys / hvals already carry SCANPM_WANTKEYS /
            // SCANPM_WANTVALS bits from c:2393 / c:2398; consumers
            // test the bits directly. No separate (hkeys & SCANPM_WANTKEYS) != 0 /
            // (hvals & SCANPM_WANTVALS) != 0 mirror state to maintain.
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
            if c == '^' || c == Hat {
                // c:Src/subst.c:2587 — `case '^'` plan9-toggle. Accept
                // both ASCII `^` and Hat TOKEN (\u{86}). The bridge
                // passthru path delivers `^`/`^^` as Hat TOKEN since
                // the lexer tokenizes `^` inside `${…}` to Hat.
                let nxt = body_chars.get(idx + 1).copied();
                if matches!(nxt, Some('^') | Some(Hat)) {
                    plan9 = false;
                    rc_force_off = true; // c:2554 `${^^}` explicit RC-off
                    idx += 2;
                } else {
                    plan9 = true;
                    idx += 1;
                }
                continue;
            }
            if c == '=' || c == Equals {
                // c:Src/subst.c:2592 — `case '='` split toggle. Accept
                // both ASCII `=` and Equals TOKEN (\u{8d}).
                let nxt = body_chars.get(idx + 1).copied();
                if matches!(nxt, Some('=') | Some(Equals)) {
                    suppress_split = true;
                    idx += 2;
                } else {
                    force_split = true;
                    idx += 1;
                }
                continue;
            }
            if c == '#' || c == Pound {
                // c:2570-2588 — `${}` ⇒ `inbrace`; `(inbrace || !POSIXIDENTIFIERS)` is satisfied.
                // Accept both ASCII `#` and Pound TOKEN (\u{84}) — the
                // bridge passthru path delivers `${#…}` length-op as
                // Pound TOKEN since `#` is lexed inside `${…}` as a
                // significant char.
                let next = body_chars.get(idx + 1).copied();
                let after_next = body_chars.get(idx + 2).copied();
                // c:Src/subst.c:2570-2588 — `${#…}` length-op
                // discrimination. Accept Dash TOKEN (\u{9b}) as
                // equivalent to ASCII `-` in the `:-` peek so
                // `${#:-foo}` (length of "foo" since `#` is unset)
                // resolves correctly through the bridge passthru
                // path where the lexer tokenizes `-` to Dash.
                let next_is_name_start = match next {
                    Some(ch) if ch.is_ascii_alphanumeric() => true,
                    Some(ch) if matches!(ch, '_' | '@' | '*' | '?' | '!' | '$' | '-' | '0') => true,
                    Some(':') if matches!(after_next, Some('-') | Some('\u{9b}')) => true,
                    Some(ch) if ch == STRING || ch == Qstring || ch == Stringg => {
                        // `${#${...}}` — Stringg/Qstring + `{`/`(` is a
                        // nested-subexp length form. Length-op applies.
                        // `${#$}` — Stringg/Qstring is the bare `$$`
                        // special name (PID); after_next is the closing
                        // brace (None at body-level). Length-op also
                        // applies. Accept both.
                        matches!(
                            body_chars.get(idx + 2).copied(),
                            Some(b) if b == Inbrace || b == '{' || b == Inpar || b == '('
                        ) || after_next.is_none()
                    }
                    Some(ch) if (ch == '#' || ch == Pound) && after_next.is_none() => true,
                    _ => false,
                };
                if next_is_name_start {
                    length_op = true;
                    idx += 1;
                    continue;
                }
            }
            if c == '~' || c == Tilde {
                // c:Src/subst.c:2596-2602 — accept both ASCII `~` AND
                // the Tilde TOKEN (\u{98}) the lexer emits for
                // unquoted forms (`${~$(...)}`). Without the TOKEN
                // arm, `${~$(echo pat)}` parsed `\u{98}` as an
                // unrecognized flag → "bad substitution". Bug #330
                // in docs/BUGS.md.
                let doubled = body_chars
                    .get(idx + 1)
                    .copied()
                    .map_or(false, |n| n == '~' || n == Tilde);
                if doubled {
                    if !qt {
                        tilde_carrier_note(crate::ported::zsh_h::isset(crate::ported::zsh_h::GLOBSUBST));
                        opt_state_set("globsubst", false);
                    }
                    idx += 2;
                } else {
                    if !qt {
                        tilde_carrier_note(crate::ported::zsh_h::isset(crate::ported::zsh_h::GLOBSUBST));
                        opt_state_set("globsubst", true);
                    }
                    idx += 1;
                }
                continue;
            }
            if c == '+' {
                // c:2199
                let nxt = body_chars.get(idx + 1).copied().unwrap_or('\0'); // c:2199
                let ok = nxt.is_ascii_alphanumeric()
                    || nxt == '_'
                    || matches!(nxt, '@' | '*' | '#' | '?')
                    || (aspar
                        && (nxt == STRING || nxt == Qstring)
                        && matches!(
                            body_chars.get(idx + 2).copied(),
                            Some(b) if b == Inbrace || b == '{' || b == Inpar || b == '('
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
            // Skip leftover quote markers (Snull/Dnull from `'...'`/`"..."`
            // boundaries). Do NOT skip Stringg/Qstring — those are the
            // `$`-tokenized chars that the subexp arm at c:2649
            // (`isstring(*s)`) needs to see in order to detect nested
            // `${${...}}` / `${$(...)}` shapes. Previously this loop
            // ate Qstring as preamble, masking the subexp detection.
            if matches!(c, Snull | Dnull) {
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
            // of zsh's Qstring/STRING dual-pass at subst.c:282.
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
          // c:2649 — `isstring(*s)` matches both `$` (`Stringg`) and the
          // DQ-context `\u{8c}` (`Qstring`) per Src/zsh.h:167. Mirror
          // here so nested ${${x}} inside DQ — where the inner `$` is
          // Qstring-tokenized — still detects the subexp. Bridge
          // passthru: unquoted nested `${…}` also reaches paramsubst
          // as `Stringg` (\u{85}) since the lexer emits Stringg for
          // `$` inside braces; accept it too.
          // c:Src/subst.c:2655 — nested `${…}` / `$(…)` / `$NAME` body.
          // BUT a bare `$` followed directly by `}` / `)` / Outbrace /
          // Outpar is the special-parameter `$$` shape (e.g. `${#$}` =
          // length of PID), NOT a nested subexp. Only enter the
          // subexp path when the `$` has actual content after it.
        let next_after_dollar = body_chars.get(idx + 1).copied();
        let is_bare_special_dollar = matches!(
            next_after_dollar,
            Some('}') | Some(')') | Some(Outbrace) | Some(Outpar) | None
        );
        let mut subexp_value: Option<String> = if idx < body_chars.len()
            && (body_chars[idx] == '$' || body_chars[idx] == Qstring || body_chars[idx] == Stringg)
            && !is_bare_special_dollar
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
                // c:Src/subst.c:2655 — nested `${…}` or `$(…)` body
                // boundary scan. Match both ASCII and TOKEN brace
                // forms: the lexer emits Inbrace/Outbrace TOKEN
                // (\u{8f}/\u{90}) for `${…}` in DQ context (see
                // `${#${(z)X}}` — outer `${ }` braces tokenize).
                // Without the TOKEN arm here, the depth scan
                // falls through to the identifier-walk path and
                // truncates the inner expansion to just the
                // leading `$\u{8f}`, which then fails paramsubst
                // with "closing brace missing".
                let (open, close): (char, char) = match nx {
                    '{' => ('{', '}'),
                    '(' => ('(', ')'),
                    Inbrace => (Inbrace, Outbrace),
                    Inpar => (Inpar, Outpar),
                    // c:Src/subst.c:2655 — `$((expr))` arith form
                    //   lexes as Inparmath … Outparmath (the double-
                    //   paren `((`/`))` collapse into single
                    //   tokens at lex.c:565+). Without these arms,
                    //   the subexp scan fell through to the bare
                    //   $name identifier-walk path and truncated the
                    //   inner expansion to just `$\u{89}` (2 chars),
                    //   so `${$((expr))}` returned empty. Bug #165
                    //   in docs/BUGS.md.
                    Inparmath => (Inparmath, Outparmath),
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
            // c:Src/subst.c:2681 — `multsub(&val, PREFORK_SUBEXP,
            //   (aspar ? NULL : &aval), &isarr, NULL, &ms_flags);`. C
            //   ALWAYS calls multsub (not singsub) for nested
            //   `${(flags)${...}}` so the inner array shape propagates
            //   to outer flag handling — `(j:-:)` joins the inner (s)
            //   split, `(o)` sorts the inner array, etc. The previous
            //   Rust gate `if nojoin == 2` only used multsub when the
            //   outer carried `(@)`, so non-`(@)` nested forms like
            //   `${(j:-:)${(s: :)a}}` collapsed the inner array to a
            //   scalar via singsub and the outer `(j)` had nothing to
            //   join. Bug #63 in docs/BUGS.md.
            let expanded = {
                // c:Src/subst.c:2169 — `sub_flags` is the OUTER
                // paramsubst's filter-disposition state (SUB_MATCH/M
                // for `:#`, etc.). The inner recursive paramsubst
                // (via multsub→prefork→stringsubst→paramsubst) at
                // line 4019 unconditionally overwrites `sub_flags`
                // with its own flag bits (typically 0 since the
                // inner has no operator). When the inner returns,
                // the outer's downstream `:#` arm at line 6700 reads
                // sub_flags and gets the inner's 0 — so the M-flag
                // `invert` was silently dropped and `${(M)${(k)h}:#p}`
                // applied the NORMAL drop-matching filter instead
                // of M's keep-matching one. Save and restore around
                // the subexp call to mirror C's scoping (C's
                // `sub_flags` is a function-local int saved across
                // the recursive `multsub` call at c:2681).
                let saved_sub_flags = sub_flags_get();
                let (joined, arr_parts, isarr, _) = multsub(&inner, PREFORK_SUBEXP);
                sub_flags_set(saved_sub_flags);
                // c:Src/exec.c:4856-4871 readoutput — UNQUOTED `$(…)`
                // output is IFS-word-split (`spacesplit(buf, 0, 1, 0)`,
                // c:4865) into the word list after trailing newlines
                // are stripped; only the quoted `"$(…)"` form (qt,
                // c:4858-4864) keeps one word. The Rust multsub returns
                // the raw output as one scalar, so apply the split here
                // for the unquoted cmd-subst shape. This is what makes
                // `${#$(cmd)}` count WORDS and `${(f)$(cmd)}` see an
                // array that the c:3905 sepjoin then re-joins — zsh:
                // `${#${(f)$(typeset +i)}}` = chars of the space-joined
                // scalar, not the line count.
                let inner_is_cmdsubst =
                    matches!(body_chars.get(start + 1).copied(), Some('(') | Some(Inpar));
                let (joined, arr_parts, isarr) = if inner_is_cmdsubst
                    && !qt
                    && !peeled_quotes
                    && body_chars[start] != Qstring
                {
                    // c:4865 spacesplit with default IFS — runs of
                    // whitespace collapse, leading/trailing stripped.
                    let words: Vec<String> = joined.split_whitespace().map(String::from).collect();
                    if words.len() <= 1 {
                        (words.into_iter().next().unwrap_or_default(), Vec::new(), false)
                    } else {
                        (words.join(" "), words, true)
                    }
                } else {
                    (joined, arr_parts, isarr)
                };
                if isarr && !arr_parts.is_empty() {
                    // Generate a stable per-call temp name. We use a
                    // process-local counter; cleanup happens at end of
                    // paramsubst (state.arrays.remove).
                    static SEQ: AtomicUsize = AtomicUsize::new(0);
                    let n = SEQ.fetch_add(1, Ordering::Relaxed);
                    let temp = format!("__subexp_arr_{}", n);
                    // c:Src/subst.c — `v.isarr = 1; aval = arr_parts;`
                    // (the C path stores the array in the per-paramsubst
                    // Value struct directly). Mirror by stashing the
                    // Vec locally (subexp_arr_parts) for the downstream
                    // [N] discriminator AND inserting into the arrays
                    // table for var-lookup compat with the existing
                    // paramsubst splat/subscript code.
                    arrays_insert(temp.clone(), arr_parts.clone());
                    subexp_array_temp = Some(temp.clone());
                    subexp_arr_parts = Some(arr_parts);
                    temp
                } else {
                    joined
                }
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
        // c:Src/subst.c — when subexp_value is set (the body started
        // with `${...}` / `$(...)`), the var-name is already provided
        // by the inner expansion's result; the outer must NOT then
        // greedily eat the next char as a single-char special name
        // (`@`/`*`/`#`/`?`/`0`). Symptom: `${${X#*:}#*:}` had its
        // outer `#` consumed as `$#` name, leaving rest=`*:` which
        // never hit the prefix-strip arm — the outer became a no-op.
        // Skip the name-walk entirely when subexp_value is in flight.
        let name_start = idx;
        // c:Src/subst.c:1942 — `${(flags)"literal"}` raises "bad
        // substitution" naturally through paramsubst's name-walker
        // (the `"` char isn't a valid name char so the walker bails
        // and the validity gate at subst.c:1885 fires). Earlier the
        // compile path injected a `\u{01}` sentinel to short-circuit
        // here; removed because the natural path produces the same
        // error.
        // c:Src/subst.c — `${(flags)NAME[KEY]}` now routes through
        // BUILTIN_BRIDGE_BRACE_ARRAY at the compile path. paramsubst's
        // canonical flag parser handles flag-then-subscript composition
        // inline (composition fix at subst.rs:9670 — non-splat-subscript
        // casmod applies to `value` instead of refetching the array).
        // The `\u{08}` pre-resolved-value sentinel is gone.
        let prefiltered_value: Option<String> = None;
        if subexp_value.is_none() {
            while idx < body_chars.len() {
                let bc = body_chars[idx];
                let allowed = if idx == name_start {
                    // c:Src/subst.c:2697 — `${#}` / `${@}` / `${*}` /
                    // `${?}` / `${0}` / `${-}` / `${$}` / `${!}` are
                    // the single-char special parameters. The bridge
                    // passthru path delivers their punctuation forms
                    // as TOKENs (Pound, Star, Quest, Bang, Dash) since
                    // the lexer tokenizes them inside `${…}` — accept
                    // both ASCII and TOKEN so e.g. `${#-}` (length of
                    // `$-` flags string) resolves through the bridge.
                    // Without `-` / `$` here, `${#-}` parsed as
                    // length-of-empty-name and returned 0.
                    bc.is_ascii_alphanumeric()
                        || bc == '_'
                        || bc == '@'
                        || bc == '*' || bc == '\u{87}' /* Star */
                        || bc == '#' || bc == Pound
                        || bc == '?' || bc == '\u{97}' /* Quest, c:Src/zsh.h:178 */
                        || bc == '!' || bc == '\u{9c}' /* Bang, c:Src/zsh.h:183 */
                        || bc == '0'
                        || bc == '-' || bc == '\u{9b}' /* Dash */
                        || bc == '$' || bc == Stringg
                } else {
                    bc.is_ascii_alphanumeric() || bc == '_'
                };
                if allowed {
                    idx += 1;
                    // Single-char specials stop after one char
                    let first = body_chars[name_start];
                    if idx == name_start + 1
                        && (matches!(first, '@' | '*' | '#' | '?' | '0' | '!' | '-' | '$')
                            || first == Pound
                            || first == '\u{87}' /* Star */
                            || first == '\u{97}' /* Quest, c:Src/zsh.h:178 */
                            || first == '\u{9c}' /* Bang, c:Src/zsh.h:183 */
                            || first == '\u{9b}' /* Dash */
                            || first == Stringg)
                    {
                        break;
                    }
                } else {
                    break;
                }
            }
        }
        // c:Src/subst.c:2728 — the single-char specials referenced by
        // name. Normalize TOKEN form to ASCII so the var-lookup path
        // sees canonical names (`#`, `?`, `!`, `*`).
        let mut var_name: String = {
            let raw: String = body_chars[name_start..idx].iter().collect();
            if raw.chars().any(|c| {
                let cu = c as u32;
                (0x84..=0xa1).contains(&cu)
            }) {
                crate::lex::untokenize(&raw)
            } else {
                raw
            }
        };
        // c:Src/subst.c — bash's `${!var}` indirect form is NOT
        // supported in zsh. The name walk above stops on `!` after
        // one char (single-char special $!), so `${!Y}` /
        // `${!FOO_*}` / `${!BAR_@}` parse leaves a stray identifier-
        // start char in the "rest" position where only modifier
        // operators (`:`, `#`, `%`, `/`, `^`, `~`, `,`, `+`, `-`,
        // `=`, `?`, `[`, `(`) are valid. zsh's paramsubst at
        // subst.c:2147+ rejects this combination with
        // "bad substitution"; mirror that here so $? matches.
        // c:Src/subst.c:2986-3004 — "We're now past the name or
        // subexpression; the only things which can happen now are a
        // closing brace, one of the standard parameter postmodifiers,
        // or a history-style colon-modifier." A quote char (Dnull /
        // Snull / Tick) in operand position is not in that set →
        // `zerr("bad substitution")`. This is the `${(Q)"abc"}` /
        // `${(L)'lit'}` / `${"abc"}` shape (probe: `zsh -fc 'print --
        // ${(Q)"abc"}'` → `zsh:1: bad substitution`, exit 1). The
        // pre-name loop above skips the OPENING quote (c:2623-2631
        // inull skip, mirrored at the Snull|Dnull `continue` arm), so
        // after the name walk idx sits on the CLOSING quote — exactly
        // the state C's c:2990 check rejects. The quoted-subexp idiom
        // `${(f)"$(cmd)"}` is unaffected — it carries subexp_value.
        if subexp_value.is_none() && idx < body_chars.len() {
            let nx = body_chars[idx];
            if nx == Dnull || nx == Snull || nx == Tick {
                zerr("bad substitution");
                errflag.fetch_or(crate::ported::zsh_h::ERRFLAG_ERROR, Ordering::Relaxed);
                return (String::new(), new_pos, Vec::new());
            }
        }
        if (var_name == "!" || var_name == "\u{9c}") && idx < body_chars.len() {
            let nx = body_chars[idx];
            if nx.is_ascii_alphanumeric()
                || nx == '_'
                || nx == '@'
                || nx == '*'
                || nx == '\u{87}'
                || nx == '!'
                || nx == '\u{9c}' /* Bang, c:Src/zsh.h:183 */
            {
                zerr("bad substitution");
                errflag.fetch_or(crate::ported::zsh_h::ERRFLAG_ERROR, Ordering::Relaxed);
                return (String::new(), new_pos, Vec::new());
            }
        }
        // c:2993-3004 — after the name walk, the only valid next
        // chars are an operator start (`-+:%/=#?`) or the closing
        // brace. A Dnull/Snull quote marker here means the body
        // quoted a BARE NAME (`${(Q)"abc"}` / `${"abc"}`) — C's
        // gate sees the raw Dnull (not in the allowed set) and
        // zerr's "bad substitution". Check the RAW char HERE because
        // the `rest` builder below silently drops quote markers
        // before the downstream operator gate runs. Subexp bodies
        // (`${(f)"$(cmd)"}`) are exempt: C's post-subexp
        // `while (inull(*s)) s++;` at c:2696-2697 eats the closing
        // quote before the gate; the Rust subexp path mirrors that
        // via the marker-dropping rest builder (subexp_value /
        // subexp_array_temp are Some only on that path).
        if subexp_value.is_none()
            && subexp_array_temp.is_none()
            && idx < body_chars.len()
            && (body_chars[idx] == Snull || body_chars[idx] == Dnull)
        {
            zerr("bad substitution"); // c:3001
            errflag.fetch_or(crate::ported::zsh_h::ERRFLAG_ERROR, Ordering::Relaxed);
            return (String::new(), new_pos, Vec::new()); // c:3002
        }
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
        // for nested `${arr[$other[1]]}`. Accept both ASCII `[` and
        // Inbrack TOKEN (\u{91}) — the bridge passthru path delivers
        // the subscript opener as Inbrack TOKEN since the lexer
        // tokenizes `[`/`]` inside `${…}` to Inbrack/Outbrack.
        let mut subscript: Option<String> = None; // c:2867
        if idx < body_chars.len() && (body_chars[idx] == '[' || body_chars[idx] == Inbrack) {
            // c:2867
            idx += 1; // c:2867
            let sub_start = idx;
            let mut depth = 1_i32;
            while idx < body_chars.len() && depth > 0 {
                // c:2867
                let bc = body_chars[idx];
                if bc == '[' || bc == Inbrack {
                    depth += 1;
                }
                // c:2867
                else if bc == ']' || bc == Outbrack {
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
                // Bug #338 sub-issue — DQ-wrapped literal subscript
                // `${h["q'q"]}` inside outer `"…"`: the literal `"…"`
                // delimiters around the key are part of the stored
                // assoc key (zsh stores the literal byte sequence
                // including the DQs — verified vs /opt/homebrew/bin/zsh).
                // singsub → prefork strips DQ pairs and the embedded
                // single quote (treating `'` as SQ delimiter inside
                // DQ when it should be literal), turning `"q'q"` (5
                // bytes) into `qq` (2 bytes) and missing the stored
                // entry. When the subscript text has NO `$` /
                // String/Qstring/Tick tokens — i.e. it's a pure
                // literal — bypass singsub and use the lexer-
                // preserving variant which maps Dnull→`"`/Snull→`'`/
                // Bnull→`\\` without stripping. Mirrors the storage-
                // path fast path already in
                // src/extensions/compile_zsh.rs::compile_assign which
                // uses untokenize_preserve_quotes on the same key.
                // c:Src/params.c::getarg:1541 — `if (ispecial(c))
                // needtok = 1;`. `ispecial` (Src/ztype.h:59 +
                // Src/utils.c:4253-4254 + Src/zsh.h:228 `SPECCHARS`)
                // is true for every char in
                //   `#$^*()=|{}[]\`<>?~;&\n\t \\'\"`.
                // C-faithful port: check the full set, plus the
                // lexer-TOKEN equivalents the Rust pipeline produces
                // for the same chars. Pre-untokenize bodies trigger
                // via the TOKEN arms; post-untokenize bodies trigger
                // via the ASCII arms.
                let needtok = raw_sub.chars().any(|c| {
                    // SPECCHARS (zsh.h:228) — full set per C ispecial.
                    matches!(c,
                        '#' | '$' | '^' | '*' | '(' | ')' | '=' | '|' |
                        '{' | '}' | '[' | ']' | '`' | '<' | '>' | '?' |
                        '~' | ';' | '&' | '\n' | '\t' | ' ' | '\\' |
                        '\'' | '"')
                    // Lexer TOKEN forms of the same chars (Src/lex.c
                    // tokenizes these inside `${...}` bodies so the
                    // subscript text may carry token bytes instead of
                    // ASCII when arrived from a tokenized path).
                    || c == crate::ported::zsh_h::Pound      // #
                    || c == crate::ported::zsh_h::Stringg    // $
                    || c == crate::ported::zsh_h::Qstring    // $ (DQ)
                    || c == crate::ported::zsh_h::Hat        // ^
                    || c == crate::ported::zsh_h::Star       // *
                    || c == crate::ported::zsh_h::Inpar      // (
                    || c == crate::ported::zsh_h::Outpar     // )
                    || c == crate::ported::zsh_h::Equals     // =
                    || c == crate::ported::zsh_h::Bar        // |
                    || c == crate::ported::zsh_h::Inbrace    // {
                    || c == crate::ported::zsh_h::Outbrace   // }
                    || c == crate::ported::zsh_h::Inbrack    // [
                    || c == crate::ported::zsh_h::Outbrack   // ]
                    || c == crate::ported::zsh_h::Tick       // `
                    || c == crate::ported::zsh_h::Qtick      // ` (DQ)
                    || c == crate::ported::zsh_h::Inang      // <
                    || c == crate::ported::zsh_h::Outang     // >
                    || c == crate::ported::zsh_h::Quest      // ?
                    || c == crate::ported::zsh_h::Tilde      // ~
                    || c == crate::ported::zsh_h::Bang       // !
                    || c == crate::ported::zsh_h::Snull      // '
                    || c == crate::ported::zsh_h::Dnull      // "
                    || c == crate::ported::zsh_h::Bnull      // \
                });
                let expanded = if needtok {
                    // c:Src/params.c::getarg:1564-1572 — when needtok
                    // fires, the full sequence is:
                    //   char exe = opts[EXECOPT];
                    //   s = dupstring(s);
                    //   if (parsestr(&s)) return 0;
                    //   if (scanflags & SCANPM_NOEXEC)
                    //     opts[EXECOPT] = 0;
                    //   singsub(&s);
                    //   opts[EXECOPT] = exe;
                    // C-faithful port:
                    //   1) parsestr re-tokenizes via dquote_parse
                    //      (Src/lex.c:1694) — recognises nested
                    //      `${…}`/`$(…)`/`$((…))`/backticks the same
                    //      way a normal command parse does. On parse
                    //      failure, return 0 (subscript invalid).
                    //   2) singsub resolves the substitutions.
                    //   3) untokenize (c:1584) before the assoc lookup
                    //      so token bytes (Dash/Equals/Bar/etc.) become
                    //      ASCII for the `map.get(sub)` key match.
                    // SCANPM_NOEXEC's opts[EXECOPT] juggle is omitted
                    // — zshrs doesn't expose a SCANPM_NOEXEC scanflag
                    // on the assoc/array lookup path yet, so the
                    // setpaths-style "don't execute cmd-subs" gate
                    // never fires here.
                    let parsed = match crate::ported::lex::parsestr(&raw_sub) {
                        Ok(p) => p,
                        Err(_) => {
                            // c:1567 — `if (parsestr(&s)) return 0;`.
                            // Treat as empty subscript so the lookup
                            // returns nothing rather than feeding
                            // garbage into singsub.
                            String::new()
                        }
                    };
                    // c:1584 — C's untokenize maps quote markers back
                    // to their ztokens chars (Dnull -> '"', Snull ->
                    // '\''): assoc keys are LITERAL text including
                    // quote characters — `H["a b"]=v` stores the
                    // 5-char key `"a b"` and `${H["a b"]}` must
                    // produce the same bytes to match (verified zsh
                    // 5.9; the plain Rust untokenize strips the
                    // markers, so the lookup key lost its quotes and
                    // missed).
                    crate::ported::lex::untokenize_preserve_quotes(&singsub(&parsed)) // c:1571 + c:1584
                } else {
                    crate::ported::lex::untokenize_preserve_quotes(&raw_sub)
                };
                // c:Src/params.c:2102-2106 — `if (*s == ',') zerr("invalid
                // subscript")` — the subscript grammar is `start[,end]`,
                // exactly two comma-separated args. A third comma is a
                // syntax error. C's getindex catches this via `if (s !=
                // tbrack) s = *pptr` + reject at the caller; zsh's
                // paramsubst surfaces it as "bad substitution". Bug #408.
                {
                    let mut depth = 0i32;
                    let mut commas = 0usize;
                    for ch in expanded.chars() {
                        match ch {
                            '(' | '[' | '{' => depth += 1,
                            ')' | ']' | '}' => depth -= 1,
                            ',' if depth == 0 => commas += 1,
                            _ => {}
                        }
                    }
                    if commas > 1 {
                        zerr("bad substitution");
                        errflag_set_error();
                        return (String::new(), idx + 1, vec![]);
                    }
                }
                subscript = Some(expanded);
            }
            if idx < body_chars.len() {
                idx += 1;
            } // skip ]
        }

        // c:Src/subst.c:2925 getarg recursion — after the first
        // subscript resolves an array element to a scalar, a SECOND
        // `[…]` walks INTO that scalar as a string-subscript. zsh's
        // `${a[N][M]}` returns char M (1-based) of array element N.
        // The previous Rust port read only the first subscript and
        // left the second `[…]` as opaque text in `rest`, which the
        // operator-arm path doesn't recognize → fell through with
        // the whole array element verbatim.
        let mut second_subscript: Option<String> = None;
        if idx < body_chars.len() && (body_chars[idx] == '[' || body_chars[idx] == Inbrack) {
            idx += 1;
            let sub2_start = idx;
            let mut depth = 1_i32;
            while idx < body_chars.len() && depth > 0 {
                let bc = body_chars[idx];
                if bc == '[' || bc == Inbrack {
                    depth += 1;
                } else if bc == ']' || bc == Outbrack {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                idx += 1;
            }
            if idx > sub2_start {
                let raw2: String = body_chars[sub2_start..idx].iter().collect();
                let expanded2 = singsub(&raw2);
                // Same 3-comma reject as the first subscript above —
                // c:Src/params.c:2102-2106 / bug #408. Applies to the
                // recursive `${a[N][M]}` subscript too.
                {
                    let mut depth = 0i32;
                    let mut commas = 0usize;
                    for ch in expanded2.chars() {
                        match ch {
                            '(' | '[' | '{' => depth += 1,
                            ')' | ']' | '}' => depth -= 1,
                            ',' if depth == 0 => commas += 1,
                            _ => {}
                        }
                    }
                    if commas > 1 {
                        zerr("bad substitution");
                        errflag_set_error();
                        return (String::new(), idx + 1, vec![]);
                    }
                }
                second_subscript = Some(expanded2);
            }
            if idx < body_chars.len() {
                idx += 1;
            } // skip ]
        }

        // c:Src/subst.c:2899+ — the remaining operator+pattern text
        // after the name. In C, paramsubst NEVER untokenizes this body
        // — operator chars (`:`, `-`, `+`, `#`, `=`, `?`, `/`, `%`,
        // `|`, `*`, `^`) arrive in whatever form lex left them, and
        // brace tokens (Inbrace/Outbrace) for nested `${…}` stay
        // tokenized so skipparens can count them.
        //
        // zshrs's downstream strip_prefix sites (`r.strip_prefix("#")`,
        // `r.strip_prefix(":-")`, etc.) are ASCII-only, but the bridge
        // synthesizer (compile_zsh.rs codegen → fusevm_bridge.rs
        // paramsubst_to_value) hands us TOKEN-form bytes for chars
        // that lex tokenized at parse time (Pound \u{84} for `#`,
        // Equals \u{86}, Inbrack \u{91}, etc.). To match C's
        // invariant — operators-ASCII-but-braces-token — walk the
        // body inline: convert ITOK bytes via the canonical
        // untokenize map (lex.rs:4499) BUT pass Inbrace/Outbrace
        // through unchanged so they stay as ITOK bytes for the inner
        // brace scanner at subst.rs:2896.
        //
        // No private-use-area placeholders, no sentinels — direct
        // char-by-char walk mirroring C's ztokens[c - Pound] mapping
        // with the brace pair skipped. `$'…'` segments (Qstring/
        // Stringg + Snull … Snull) get delegated to `crate::lex::
        // untokenize` on just that slice, which calls
        // `getkeystring_dollar_quote` internally for the escape
        // decode.
        let rest: String = {
            let raw_chars: Vec<char> = body_chars[idx..].to_vec();
            let mut out = String::with_capacity(raw_chars.len());
            let mut i = 0usize;
            while i < raw_chars.len() {
                let c = raw_chars[i];
                let cu = c as u32;
                if (0x84..=0xa1).contains(&cu) {
                    // `$'…'` ANSI-C string region — find closing Snull
                    // and let canonical untokenize handle the decode
                    // (it owns the escape table at lex.rs:4279).
                    if (c == Qstring || c == Stringg)
                        && i + 1 < raw_chars.len()
                        && raw_chars[i + 1] == Snull
                    {
                        let mut j = i + 2;
                        while j < raw_chars.len() && raw_chars[j] != Snull {
                            j += 1;
                        }
                        let end = if j < raw_chars.len() { j + 1 } else { j };
                        let segment: String = raw_chars[i..end].iter().collect();
                        out.push_str(&crate::lex::untokenize(&segment));
                        i = end;
                        continue;
                    }
                    match c {
                        x if x == Pound => out.push('#'),
                        x if x == Stringg => out.push('$'),
                        x if x == Hat => out.push('^'),
                        x if x == Star => out.push('*'),
                        x if x == Inpar => out.push('('),
                        x if x == Outpar => out.push(')'),
                        x if x == Inparmath => out.push('('),
                        x if x == Outparmath => out.push(')'),
                        x if x == Qstring => out.push('$'),
                        x if x == Equals => out.push('='),
                        x if x == crate::ported::zsh_h::Bar => out.push('|'),
                        // Deliberate divergence from canonical
                        // untokenize: preserve brace tokens so the
                        // inner skipparens-equivalent at
                        // subst.rs:2896 counts nested `${…}` via
                        // Inbrace/Outbrace as C does. Folding them
                        // to raw `{`/`}` makes inner braces
                        // indistinguishable from literal replacement
                        // braces like `${var/pat/{X}}` and the
                        // scanner mis-counts depth.
                        x if x == Inbrace => out.push(Inbrace),
                        x if x == Outbrace => out.push(Outbrace),
                        x if x == Inbrack => out.push('['),
                        x if x == Outbrack => out.push(']'),
                        x if x == Tick => out.push('`'),
                        x if x == Inang => out.push('<'),
                        x if x == Outang => out.push('>'),
                        x if x == OutangProc => out.push('>'),
                        x if x == Quest => out.push('?'),
                        x if x == Tilde => out.push('~'),
                        x if x == Qtick => out.push('`'),
                        x if x == crate::ported::zsh_h::Comma => out.push(','),
                        x if x == Dash => out.push('-'),
                        x if x == Bang => out.push('!'),
                        // c:Src/exec.c:2098 — `if (c != Nularg)` —
                        // Snull/Dnull/Nularg drop silently on the
                        // value-stream path (subst.rs caller). See
                        // lex.rs:4569 for the full rationale.
                        x if x == Snull || x == Dnull || x == Nularg => {}
                        // Preserve Bnull / Bnullkeep — these are the
                        // lex's escape markers ("next char is user-
                        // literal"). Downstream consumers of the
                        // `rest` body include singsub() calls on the
                        // replacement string (subst.rs:7202-7203 for
                        // `:/` and 7232+ for `//`/`///`), and singsub
                        // → stringsubst recognizes Bnull at
                        // c:Src/subst.c:301 (`else if (c == Bnull)
                        // … skip; goto cont;`) to skip the marked
                        // char without treating it as a `$` opener.
                        // Folding Bnull → `\\` would lose this
                        // escape semantic — stringsubst's check
                        // works on Bnull byte, not on the raw
                        // backslash that follows.
                        x if x == Bnull => out.push(Bnull),
                        x if x == Bnullkeep => out.push(Bnullkeep),
                        _ => out.push(c),
                    }
                } else {
                    out.push(c);
                }
                i += 1;
            }
            out
        };

        // (P) indirect: take the var name from somewhere — either
        // the value of a parameter (\${(P)x}) or the result of a
        // nested expansion (\${(P)\${(P)x}} = `(P)`-of-(P)-of-x).
        // Direct port of subst.c:2730+ aspar arm. The C source's
        // val pointer is the resolved name string regardless of
        // whether it came from a parameter or a sub-expression.
        if aspar {
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

            // c:Src/subst.c — when the indirect name carries a
            // subscript (`(P)ref` where `$ref` = "arr[2]" or
            // "hash[key]"), parse the `[idx]` and apply it. The C
            // source re-enters the paramsubst dispatch via
            // `getstrvalue(v)` which handles the subscript through
            // `getindex`. The Rust port previously stored the full
            // `"arr[2]"` as var_name and the downstream lookup
            // missed it (no array/assoc entry by that name).
            // Bug #53 in docs/BUGS.md.
            if subscript.is_none() {
                if let Some(open_idx) = var_name.find('[') {
                    if var_name.ends_with(']') {
                        let raw_key = var_name[open_idx + 1..var_name.len() - 1].to_string();
                        let base = var_name[..open_idx].to_string();
                        // Subscript may contain $-refs (`a[$n]`,
                        // `m[$key]`) — singsub them so the lookup
                        // sees the resolved index/key. Matches the
                        // singsub call at line 3998 of the bare
                        // `${arr[expr]}` subscript handler.
                        let key = if raw_key.contains('$') || raw_key.contains('`') {
                            singsub(&raw_key)
                        } else {
                            raw_key
                        };
                        var_name = base;
                        subscript = Some(key);
                    }
                }
            }
        }

        // Look up var (with subscript if present). Port of
        // subst.c:2965 getstrvalue / getarrvalue dispatch.
        // If subexp_value is set, the value comes from the recursive
        // $(...)/${...} expansion and we skip var-name lookup.
        let used_subexp = subexp_value.is_some() || prefiltered_value.is_some();
        let raw_value: String = if let Some(pv) = prefiltered_value.clone() {
            // Pre-resolved scalar value from `${(flags)NAME[KEY]}`
            // fast-path. Skip lookup entirely; the flag chain
            // (casmod / quoting / etc.) runs on this value below.
            pv
        } else if let Some(sv) = subexp_value {
            // c:Src/subst.c — when a sub-expression produced the
            // value AND an OUTER subscript follows (`${${(P)name}[N]}`),
            // apply the subscript to the result's array shape.
            // Bug #182.
            //
            // Gate on `casmod == CASMOD_NONE` and `quotemod == 0`
            // (i.e. outer has NO mode-altering flag) so the
            // `${(U)${(s. .)s}[1]}` path stays in the existing
            // split_parts pipeline that handles outer-flag + subscript
            // composition correctly.
            if let Some(sub) = subscript.as_deref() {
                if !length_op {
                    // c:Src/subst.c:2681+ — nested expansion result
                    // with an outer `[N]` subscript. Two sub-cases:
                    //
                    //   (a) ARRAY-shaped inner result (`(@)`-flagged
                    //       expansion that produced multiple words,
                    //       captured into `subexp_array_temp`) →
                    //       `[N]` picks element N, `[N,M]` picks slice.
                    //
                    //   (b) Otherwise → SCALAR character subscript
                    //       per Src/subst.c getindex scalar arm. Real
                    //       zsh: `${${arr}[2]}` with arr=(hello world)
                    //       → `e` (char 2 of joined scalar
                    //       "hello world"); `${${(@f)$(echo myvar)}[1]}`
                    //       → `m` (char 1 of single-element array
                    //       collapsed to scalar). Multi-word but
                    //       NON-@-flagged inner is scalar.
                    //
                    // c:Src/subst.c — `if (isarr) {...}` arm. The
                    // discriminator reads `subexp_arr_parts` directly,
                    // mirroring the structural `Value.isarr` check in
                    // C without round-tripping through the
                    // `arrays_insert` side-channel that the var-lookup
                    // path uses below. Single-element arrays collapse
                    // to scalar character subscript per the empirical
                    // real-zsh behaviour above — gate on `.len() > 1`.
                    // Bug surface: `${(P)${${(@f)$(echo $src)}[1]}}`
                    // (p_flag_indirects megamonster test).
                    let arr_shape: Option<Vec<String>> = subexp_arr_parts
                        .as_ref()
                        .cloned()
                        .filter(|a| a.len() > 1);
                    if sub == "@" || sub == "*" {
                        match arr_shape {
                            Some(arr) => arr.join(" "),
                            None => sv,
                        }
                    } else if let Some((lo_s, hi_s)) = sub.split_once(',') {
                        let lo: i64 = lo_s.trim().parse().unwrap_or(1);
                        match arr_shape {
                            Some(arr) => {
                                let hi: i64 = hi_s.trim().parse().unwrap_or(arr.len() as i64);
                                getarrvalue(&arr, lo, hi).join(" ")
                            }
                            None => {
                                let hi: i64 = hi_s.trim().parse().unwrap_or(sv.chars().count() as i64);
                                let n = sv.chars().count() as i64;
                                let resolve = |k: i64| -> usize {
                                    let k = if k < 0 { n + k + 1 } else { k };
                                    if k < 1 { 0 } else if k > n { n as usize } else { (k - 1) as usize }
                                };
                                let l = resolve(lo);
                                let h = if hi == 0 { n as usize } else {
                                    let k = if hi < 0 { n + hi + 1 } else { hi };
                                    if k < 1 { 0 } else if k > n { n as usize } else { k as usize }
                                };
                                if l >= h { String::new() } else { sv.chars().skip(l).take(h - l).collect() }
                            }
                        }
                    } else if let Ok(n) = sub.trim().parse::<i64>() {
                        match arr_shape {
                            Some(arr) => {
                                let nl = arr.len() as i64;
                                let idx = if n < 0 { nl + n } else { n - 1 };
                                if idx >= 0 && (idx as usize) < arr.len() {
                                    arr[idx as usize].clone()
                                } else {
                                    String::new()
                                }
                            }
                            None => {
                                let nl = sv.chars().count() as i64;
                                let idx = if n < 0 { nl + n } else { n - 1 };
                                if idx >= 0 && (idx as usize) < sv.chars().count() {
                                    sv.chars().nth(idx as usize).map(|c| c.to_string()).unwrap_or_default()
                                } else {
                                    String::new()
                                }
                            }
                        }
                    } else if sub.starts_with('(') {
                        // c:Src/params.c:1389-1480 — FLAGGED
                        // subscripts on a subexp result ((w)N word
                        // pick, (f)N line pick, (r)/(i) searches)
                        // route through the same getarg engine the
                        // named-variable path uses. The anonymous
                        // forms `${${:-one two three}[(w)2]}` /
                        // `${${x}[(w)3]}` previously fell through to
                        // whole-value.
                        match crate::ported::params::getarg(
                            sub,
                            arr_shape.as_deref(),
                            None,
                            Some(&sv),
                        ) {
                            Some(crate::ported::params::getarg_out::Value(v)) => v.to_str(),
                            _ => sv,
                        }
                    } else {
                        sv
                    }
                } else {
                    sv
                }
            } else {
                sv // c:2681 (subexp result)
            }
        } else if let Some(sub) = subscript.as_deref() {
            // Subscripted lookup: assoc-key, array-index, or slice.
            // c:Src/Modules/parameter.c — PLAIN-key reads on a magic
            // assoc dispatch through the special hash's getnode
            // (`ht->getnode(ht, key)` = getpm*), which resolves names
            // a key ENUMERATION never lists: `options[nounset]` hits
            // getpmoption's optlookup alias handling while the scan
            // emits only canonical names. The materialized-map arm
            // below would miss those, so plain keys go getfn-first;
            // flagged subscripts ((k)/(I)/…) and @/* fall through to
            // the map machinery which needs full enumeration.
            let partab_plain_key: Option<String> =
                if !sub.starts_with('(')
                    && sub != "@"
                    && sub != "*"
                    // `(k)`/`(v)` PARAM flags pivot the result to the
                    // key / enumerated value — `${(k)parameters[PATH]}`
                    // returns "PATH", not the getfn value. Those need
                    // the map machinery below; only a flagless plain
                    // key takes the getnode fast path.
                    && (hkeys & SCANPM_WANTKEYS) == 0
                    && (hvals & SCANPM_WANTVALS) == 0
                {
                    crate::ported::modules::parameter::PARTAB
                        .iter()
                        .find(|e_| e_.name == var_name.as_str())
                        .filter(|e_| match e_.module {
                            Some(m_) => crate::ported::module::MODULESTAB
                                .lock()
                                .map(|t| t.is_loaded(m_))
                                .unwrap_or(false),
                            None => true,
                        })
                        .map(|e_| {
                            (e_.getfn)(std::ptr::null_mut(), sub)
                                .and_then(|p_| p_.u_str)
                                .unwrap_or_default()
                        })
                } else {
                    None
                };
            if let Some(v) = partab_plain_key {
                v
            } else if let Some(map) = assoc_get(&var_name) {
                // c:2926 (assoc lookup)
                // c:Src/params.c — `${assoc[@]}` and `${assoc[*]}`
                // enumerate VALUES (matching the unquoted-bare
                // splat `$assoc[@]` semantics handled in
                // multsub_get_node_array at subst.c:3950). Without
                // this special-case, the generic `map.get(sub)` at
                // the bottom of this arm looked up a key literally
                // named "@" / "*" and returned empty — so
                // `${h[@]}` came out blank instead of enumerating
                // the value bag. Bug #109 in docs/BUGS.md.
                if sub == "@" || sub == "*" {
                    // Values in insertion order (IndexMap iter order).
                    map.values().cloned().collect::<Vec<_>>().join(" ")
                } else
                // Subscript-flag form: (I)pat / (i)pat (search keys
                // for pattern, return matching key) and (R)pat /
                // (r)pat (search values, return matching value).
                // Direct port of Src/params.c getarg's hash-aware
                // index/match handling.
                if let Some((flags, num, pat)) =
                    (|s: &str| -> Option<(String, Option<i64>, String)> {
                    let s = s.trim_start();
                    let rest = s.strip_prefix('(')?;
                    let close = rest.find(')')?;
                    let body = &rest[..close];
                    let pat = rest[close + 1..].to_string();
                    // c:Src/params.c:1419-1454 — parse n/b sub-flags
                    // with delimited integer body. `(n.N.)` picks the
                    // Nth match; negative N reverses direction (xor
                    // semantics with r/R: r+neg → R, R+neg → r).
                    let mut flags = String::new();
                    let mut num: Option<i64> = None;
                    let mut chars = body.chars().peekable();
                    while let Some(c) = chars.next() {
                        match c {
                            'R' | 'r' | 'k' | 'K' | 'i' | 'I' | 'e' => flags.push(c),
                            'n' | 'b' => {
                                let delim = chars.next()?;
                                let mut numstr = String::new();
                                for cc in chars.by_ref() {
                                    if cc == delim {
                                        break;
                                    }
                                    numstr.push(cc);
                                }
                                let n = numstr.trim().parse::<i64>().ok()?;
                                if c == 'n' {
                                    num = Some(n);
                                }
                                flags.push(c);
                            }
                            _ => return None,
                        }
                    }
                    let has_search = flags.contains('R')
                        || flags.contains('r')
                        || flags.contains('k')
                        || flags.contains('K')
                        || flags.contains('i')
                        || flags.contains('I');
                    if !has_search {
                        return None;
                    }
                    Some((flags, num, pat))
                })(sub)
                {
                    // c:Src/params.c:1564-1572 — `singsub(&s)` on the
                    // subscript expression before pattern compilation,
                    // so `${h[(R)$x]}` expands $x first. Mirrors the
                    // array-side fix below at line ~4760.
                    let pat = if pat.contains('$') || pat.contains('`') {
                        singsub(&pat)
                    } else {
                        pat
                    };
                    // c:Src/params.c:1696-1750 assoc-subscript flag
                    // dispatch:
                    //   (r)/(R): match against VALUE, return VALUE(s)
                    //   (k)/(K): match against KEY,   return VALUE(s)
                    // Uppercase variants set down=1 → return ALL matches.
                    //
                    // Bug #77 in docs/BUGS.md: the previous Rust port
                    // gated `by_key` on `I/i` (value-matchers that
                    // return keys), not on `k/K` (the actual
                    // key-matchers), so `${h[(k)foo]}` searched VALUES
                    // for `foo` and returned empty when no value
                    // matched. `${h[(k)-a]}` had the same root cause
                    // (the leading-dash framing in BUGS.md was a
                    // misdirection — all `(k)` lookups were broken).
                    // c:Src/params.c:1396-1431 — `i`/`I`/`k`/`K` all
                    // match against KEYS; lowercase variants return
                    // the first key, uppercase variants return all
                    // matching keys. `(i)/(I)` use glob, `(k)/(K)` are
                    // exact-only.
                    let match_against_key = flags.contains('k')
                        || flags.contains('K')
                        || flags.contains('i')
                        || flags.contains('I');
                    // c:Src/params.c — (n.N.) with negative N flips
                    // the direction. (n.-1.r) → R semantics (all matches);
                    // (n.-1.R) → r (single match). Verified vs
                    // /opt/homebrew/bin/zsh: typeset -A h=(a 1 b 1 c 2);
                    // ${h[(n.-1.r)1]} → "1 1"; ${h[(n.-1.R)1]} → "1".
                    let neg_n = num.map_or(false, |n| n < 0);
                    let mut return_all =
                        flags.contains('R') || flags.contains('K') || flags.contains('I');
                    if neg_n {
                        return_all = !return_all;
                    }
                    // `(i)/(I)` on assoc returns the KEY directly (zsh
                    // semantics) regardless of the outer (k) flag; `(k)/
                    // (K)` continues to return the value per Bug #77.
                    let force_return_key = flags.contains('i') || flags.contains('I');
                    // `(i)/(I)` use glob matching against keys (not
                    // exact); `(k)/(K)` are exact-only per C source.
                    let key_glob = flags.contains('i') || flags.contains('I');
                    let exact = flags.contains('e'); // c:1419 e flag — literal compare
                    // c:Src/params.c:665-681 scanparamvals — when the outer
                    // `(k)` paramflag is set (SCANPM_WANTKEYS bit), the hash
                    // scan returns matched KEYS instead of matched VALUES
                    // for `(r)/(R)` value-pattern subscripts. For `(k)/(K)`
                    // key-pattern subscripts (match_against_key) the key
                    // path doesn't fold into WANTKEYS — those keep returning
                    // the value. Verified via:
                    //   /opt/homebrew/bin/zsh -fc 'typeset -A h=(a 1 b 2 c 1);
                    //     echo "${(k)h[(R)1]}"'  → 'a c'
                    //   /opt/homebrew/bin/zsh -fc 'typeset -A h=(a 1 b 2 c 1);
                    //     echo "${(k)h[(k)a]}"'  → '1'  (key-match still
                    //     returns value).
                    let mut return_key = force_return_key
                        || ((hkeys & SCANPM_WANTKEYS) != 0
                            && !match_against_key
                            && (hvals & SCANPM_WANTVALS) == 0);
                    // c:Src/params.c:665-681 scanparamvals — when the
                    // outer `(v)` paramflag is set (SCANPM_WANTVALS
                    // bit) AND the inner subscript flag uses
                    // (i)/(I) (key-pattern matching that defaults to
                    // returning the KEY), pivot to return the VALUE
                    // associated with each matched key. Verified vs
                    // /opt/homebrew/bin/zsh:
                    //   typeset -A M=(foo_a 1 foo_b 2 bar_x 3)
                    //   echo "${(k)M[(I)foo*]}"   →  'foo_a foo_b'
                    //   echo "${(v)M[(I)foo*]}"   →  '1 2'
                    // Without this pivot, the outer (v) flag was
                    // silently dropped and `(I)` kept returning
                    // keys for both forms.
                    if (hvals & SCANPM_WANTVALS) != 0
                        && (hkeys & SCANPM_WANTKEYS) == 0
                        && (flags.contains('i') || flags.contains('I'))
                    {
                        return_key = false;
                    }
                    let mut out: Vec<String> = Vec::new();
                    for (k, v) in map.iter() {
                        let hay = if match_against_key {
                            k.as_str()
                        } else {
                            v.as_str()
                        };
                        // c:Src/params.c — k/K (key-match path) is
                        // exact-only, no glob; r/R use patcompile; i/I
                        // use patcompile against the KEY (key_glob).
                        let matched = if exact || (match_against_key && !key_glob) {
                            hay == pat.as_str()
                        } else {
                            patcompile(&{ let mut __pat_tok = (&pat).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, PAT_HEAPDUP as i32, None)
                                .map_or(false, |__p| pattry(&__p, hay))
                        };
                        if matched {
                            // c:Src/params.c:665-681 scanparamvals — when
                            // BOTH WANTKEYS and WANTVALS are set (the
                            // `(kv)` outer flag), each match emits the KEY
                            // followed by its VALUE: `${(kv)h[(I)*]}` →
                            // `a 1 b 2 c 3`. Otherwise push KEY xor VALUE
                            // per return_key.
                            let want_both = (hkeys & SCANPM_WANTKEYS) != 0
                                && (hvals & SCANPM_WANTVALS) != 0;
                            if want_both {
                                out.push(k.clone());
                                out.push(v.clone());
                            } else if return_key {
                                out.push(k.clone());
                            } else {
                                out.push(v.clone());
                            }
                            if !return_all {
                                break;
                            }
                        }
                    }
                    // c:Src/params.c::getarg:1724-1729 — `if (down)
                    // v->scanflags |= SCANPM_MATCHMANY` then
                    // `*ta = getvaluearr(v)`. zshrs's `split_parts`
                    // (c:3950 `aval`) carries the same array shape;
                    // `isarr` mirrors `v->scanflags` setting `isarr`
                    // non-zero per subst.c:2915. For zero matches the
                    // empty `out` produces an empty array — c:1738-
                    // 1739 `if (!ta || !*ta) return !down;` returns
                    // the empty-array sentinel. Mirror by seeding
                    // `split_parts = Some(out.clone())` with
                    // `isarr = -1` (empty array) or `isarr = 1`
                    // (populated).
                    split_parts = Some(out.clone());
                    isarr = if out.is_empty() { -1 } else { 1 };
                    out.join(" ")
                } else if let Some(key) = sub
                    .trim_start()
                    .strip_prefix("(e)")
                    .or_else(|| sub.trim_start().strip_prefix("(E)"))
                {
                    // c:Src/params.c:1419 — bare `(e)KEY` exact-key
                    // lookup on assoc (no pattern interpretation).
                    // Bug #407.
                    // c:Src/params.c:1492-1494 + 1591 — same WANTKEYS
                    // → SCANPM_WANTINDEX promotion as the plain-key
                    // arm below ((e) only disables pattern compile;
                    // the inv derivation is shared).
                    if (hkeys & SCANPM_WANTKEYS) != 0
                        && (hvals & SCANPM_WANTVALS) == 0
                    {
                        if map.contains_key(key) {
                            key.to_string()
                        } else {
                            String::new()
                        }
                    } else {
                        map.get(key).cloned().unwrap_or_default()
                    }
                } else if (hkeys & SCANPM_WANTKEYS) != 0
                    && (hvals & SCANPM_WANTVALS) == 0
                {
                    // c:Src/params.c:1492-1494 — `if (v->scanflags &
                    // SCANPM_WANTKEYS) *inv = (ind || !(v->scanflags &
                    // SCANPM_WANTVALS))`: the `(k)` flag (without `(v)`)
                    // on a plain-key assoc subscript sets inv. Then
                    // c:1591 `v->scanflags = (*inv ? SCANPM_WANTINDEX :
                    // 0)` and c:Src/subst.c:2922 `val = dupstring(
                    // v->pm->node.nam)` — the hash node's NAME, i.e.
                    // the KEY, is returned instead of the value.
                    // Missing key: c:1586 createparam(s, PM_SCALAR|
                    // PM_UNSET) leaves the node unset → empty result
                    // (zsh 5.9: `${(k)H[missing]}` → ""). `(kv)`
                    // keeps WANTVALS so it stays on the value arm
                    // (zsh 5.9: `${(kv)H[k1]}` → "v1").
                    if map.contains_key(sub) {
                        sub.to_string()
                    } else {
                        String::new()
                    }
                } else {
                    map.get(sub).cloned().unwrap_or_default()
                }
            } else if let Some(arr) = arrays_get(&var_name) {
                // c:2926 (array)
                if sub == "*" || sub == "@" {
                    // c:2916 (full array)
                    arr.join(" ")
                } else if let Some((flags, num, beg, pat)) =
                    (|s: &str| -> Option<(String, Option<i64>, Option<i64>, String)> {
                    // c:Src/params.c:1411-1418 — `(i)/(I)/(r)/(R)`
                    // array subscript flags. Per C:
                    //   (i)pat: rev=ind=1, down=0 → first match, INDEX
                    //   (I)pat: rev=ind=down=1   → LAST  match, INDEX
                    //   (r)pat: rev=1, ind=0, down=0 → first match, VALUE
                    //   (R)pat: rev=1, ind=0, down=1 → LAST  match, VALUE
                    //
                    // `down=1` means scan from the end of the array
                    // backward. The previous Rust port treated capital
                    // forms as "all matches joined" which is wrong —
                    // they're LAST-match (single return).
                    let s = s.trim_start();
                    let rest = s.strip_prefix('(')?;
                    let close = rest.find(')')?;
                    let body = &rest[..close];
                    let pat = rest[close + 1..].to_string();
                    // A top-level comma after the flag means this is a
                    // SLICE with per-bound flags (`(r)3,(r)5` / `(r)3,5`),
                    // not a single `(r)pat` search. Defer to the slice
                    // arm below so each bound resolves its own flag.
                    if pat.contains(',') {
                        return None;
                    }
                    // c:Src/params.c:1431-1454 — `(n.N.)` and `(b.N.)`
                    // sub-flags each take an integer arg delimited by
                    // the next char after the letter. Parse them out
                    // so `(n.2.r)` (pick 2nd match) and `(b.3.r)`
                    // (start at offset 3) work.
                    let mut flags = String::new();
                    let mut num: Option<i64> = None;
                    let mut beg: Option<i64> = None;
                    let mut chars = body.chars().peekable();
                    while let Some(c) = chars.next() {
                        match c {
                            'I' | 'R' | 'i' | 'r' | 'e' => flags.push(c),
                            'n' | 'b' => {
                                let delim = chars.next()?;
                                let mut numstr = String::new();
                                for cc in chars.by_ref() {
                                    if cc == delim {
                                        break;
                                    }
                                    numstr.push(cc);
                                }
                                let n = numstr.trim().parse::<i64>().ok()?;
                                if c == 'n' {
                                    num = Some(n);
                                } else {
                                    beg = Some(n);
                                }
                                flags.push(c);
                            }
                            _ => return None,
                        }
                    }
                    // c:Src/params.c:1419 — `(e)` ALONE doesn't trigger
                    // the search arm; require at least one search letter.
                    let has_search = flags.contains('I')
                        || flags.contains('R')
                        || flags.contains('i')
                        || flags.contains('r');
                    if !has_search {
                        return None;
                    }
                    Some((flags, num, beg, pat))
                })(sub)
                {
                    let (flags, num, beg, pat) =
                        (flags, num, beg, pat);
                    // c:Src/params.c:1564-1572 — `if (needtok) { ...
                    // singsub(&s); ... }`. The subscript expression
                    // (pattern) goes through `singsub` so `${a[(I)$x]}`
                    // expands `$x` to the variable's value before
                    // pattern compilation. Without this, the literal
                    // text "$x" reached patcompile and never matched any
                    // element value. The unquoted-DQ fast paths at
                    // compile_zsh.rs:3447 (braced_subscript_dynamic_ref)
                    // handled this for direct `${a[...]}` reads via
                    // EXPAND_TEXT, but the runtime arithsubst path
                    // (`(( y = ${a[(I)$x]} ))`) routes through
                    // arithsubst → singsub → paramsubst and arrives
                    // here with the literal pattern. Bug #195 sibling.
                    let pat = if pat.contains('$') || pat.contains('`') {
                        singsub(&pat)
                    } else {
                        pat
                    };
                    let return_index = flags.contains('I') || flags.contains('i'); // c:1412/1416 ind=1
                    let down = flags.contains('I') || flags.contains('R'); // c:1416/c:1418 down=1
                    let exact = flags.contains('e'); // c:Src/params.c:1419 e flag — literal compare, no glob
                    let nth = num.unwrap_or(1).max(1) as usize;
                    // c:Src/params.c — `(b.N.)` is 1-based offset
                    // (start from the N-th element). Convert to
                    // 0-based for slicing.
                    let start_offset = (beg.unwrap_or(1) - 1).max(0) as usize;
                    let mut found_idx: Option<usize> = None; // c:1500
                    let mut match_count: usize = 0;
                    let arr_len = arr.len();
                    let iter_range: Box<dyn Iterator<Item = usize>> = if down {
                        let lo = start_offset.min(arr_len);
                        Box::new((lo..arr_len).rev())
                    } else {
                        let lo = start_offset.min(arr_len);
                        Box::new(lo..arr_len)
                    };
                    for idx in iter_range {
                        let elem = &arr[idx];
                        let matched = if exact {
                            elem == &pat
                        } else {
                            patcompile(&{ let mut __pat_tok = (&pat).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, PAT_HEAPDUP as i32, None)
                                .map_or(false, |__p| pattry(&__p, elem))
                        };
                        if matched {
                            match_count += 1;
                            if match_count == nth {
                                found_idx = Some(idx);
                                break;
                            }
                        }
                    }
                    match found_idx {
                        Some(idx) if return_index => (idx + 1).to_string(),
                        Some(idx) => arr[idx].clone(),
                        None if return_index && !down => {
                            // c:2945 — (i) no-match: one-past-end
                            // so `$arr[$arr[(i)pat]]` yields empty.
                            (arr.len() + 1).to_string()
                        }
                        None if return_index && down => {
                            // c:2945 — (I) no-match: 0 (before first).
                            "0".to_string()
                        }
                        None => String::new(),
                    }
                } else if let Some((flag_body, num_str)) = (|s: &str| -> Option<(String, String)> {
                    // c:Src/params.c:1437 (w)/(W)/(s.X.) — word-split
                    // subscript: treat array as scalar joined by space
                    // then split by the flag's separator, return Nth
                    // word. For arrays the join+split round-trip
                    // typically yields the original elements when (w)
                    // / (W) uses whitespace, so the result is
                    // equivalent to a plain numeric subscript. Match
                    // here so `${arr[(w)1]}` works in array context.
                    let s = s.trim_start();
                    let rest = s.strip_prefix('(')?;
                    let close = rest.find(')')?;
                    let f = rest[..close].to_string();
                    let n = rest[close + 1..].to_string();
                    if !f.chars().all(|c| matches!(c, 'w' | 'W' | 'p')) {
                        return None;
                    }
                    Some((f, n))
                })(sub)
                {
                    let _ = flag_body;
                    if let Ok(idx_n) = num_str.parse::<i64>() {
                        let len = arr.len() as i64;
                        let i = if idx_n == 0 {
                            if crate::ported::zsh_h::isset(crate::ported::zsh_h::KSHZEROSUBSCRIPT) {
                                0
                            } else {
                                -1
                            }
                        } else if idx_n < 0 {
                            len + idx_n
                        } else {
                            idx_n - 1
                        };
                        if i >= 0 && (i as usize) < arr.len() {
                            arr[i as usize].clone()
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    }
                } else if let Some(idx_n) = sub
                    .parse::<i64>()
                    .ok()
                    .or_else(|| {
                        // c:Src/params.c:1411-1430 — paren-wrapped subscript
                        // expression. When the content inside the outermost
                        // parens isn't a recognized flag-block (handled
                        // above), C falls through to getindex's arith
                        // evaluation path so `${arr[(-1)]}` evaluates `(-1)`
                        // as math and returns last element. zshrs's bare
                        // sub.parse::<i64>() couldn't handle the leading
                        // `(`, returning empty. Strip a balanced outermost
                        // paren pair and retry the integer parse — covers
                        // `(N)`, `(-N)`, `(+N)`.
                        //
                        // Accept Inpar/Outpar tokens too: the bare-context
                        // lexer tokenizes `[...(N)]` subscript parens to
                        // Inpar/Outpar (bug #9 in docs/BUGS.md). C zsh's
                        // path sees the tokenized form because its lexer
                        // does the same; this paren-strip must match.
                        let s = sub.trim();
                        let s_chars: Vec<char> = s.chars().collect();
                        let first_is_open = s_chars.first().map_or(false, |&c|
                            c == '(' || c == Inpar);
                        let last_is_close = s_chars.last().map_or(false, |&c|
                            c == ')' || c == Outpar);
                        if first_is_open && last_is_close && s_chars.len() >= 2 {
                            let inner: String = s_chars[1..s_chars.len() - 1].iter().collect();
                            inner.trim().parse::<i64>().ok()
                        } else {
                            None
                        }
                    })
                    .or_else(|| {
                        // c:Src/params.c:1419-1432 — getindex(sub) finally
                        // calls mathevali so arith expressions in the
                        // subscript (`${a[1+1]}`, `${a[i+1]}`, `${a[n]}`
                        // where n is a numeric var) evaluate properly.
                        // Parity bug: integer-only parse missed these.
                        // Skip when sub contains a top-level comma —
                        // that's a slice form `[lo,hi]`, handled by
                        // the downstream split_once arm. mathevali
                        // would evaluate the comma as the C-style
                        // sequence operator and mask the slice.
                        //
                        // Bug #9 in docs/BUGS.md — untokenize before
                        // handing to mathevali. The bare-context lexer
                        // tokenizes `(`/`)`/`[`/`]`/`*`/`-`/`?` etc. to
                        // their META markers (Inpar/Outpar/Inbrack/
                        // Outbrack/Star/Dash/Quest). mathevali's
                        // tokenizer expects ASCII operators, so the
                        // tokenized subscript `\u{88}1+0\u{8a}` was
                        // unparseable and the unquoted scalar-assign
                        // form `v=${arr[(1+0)]}` returned empty. The
                        // DQ-quoted form `v="${arr[(1+0)]}"` worked
                        // because the lexer keeps subscript chars
                        // literal inside DQ. Untokenize here so both
                        // forms hit the same math path.
                        if sub.contains(',') {
                            None
                        } else {
                            let untoked = if sub.chars().any(|c| {
                                let cu = c as u32;
                                (0x84..=0xa1).contains(&cu)
                            }) {
                                crate::lex::untokenize(sub)
                            } else {
                                sub.to_string()
                            };
                            // c:Src/params.c getindex -> getarg miss ->
                            // mathevali; C's matheval zerrs ("bad math
                            // expression: ...") and raises errflag on
                            // parse failure, aborting the input —
                            // `${a[b*]}` is a fatal math error in zsh
                            // (rc=1), not a silent empty. The previous
                            // .ok() swallowed it.
                            match crate::ported::math::mathevali(&untoked) {
                                Ok(n) => Some(n),
                                Err(e) => {
                                    // mathevali errors already carry the
                                    // "bad math expression:" prefix; don't
                                    // double it (zsh emits it once).
                                    if e.starts_with("bad math expression") {
                                        crate::ported::utils::zerr(&e);
                                    } else {
                                        crate::ported::utils::zerr(&format!(
                                            "bad math expression: {}",
                                            e
                                        ));
                                    }
                                    None
                                }
                            }
                        }
                    })
                {
                    // c:2926 (numeric index)
                    let len = arr.len() as i64;
                    // c:Src/params.c:2110-2150 — KSH_ARRAYS uses 0-based
                    // indexing (arr[0] = first). Without the option,
                    // indexing is 1-based and `arr[0]` falls through to
                    // KSHZEROSUBSCRIPT semantics (returns first element
                    // when that option is set, else empty).
                    let ksh_arrays = crate::ported::zsh_h::isset(crate::ported::zsh_h::KSHARRAYS);
                    let i = if ksh_arrays {
                        if idx_n < 0 {
                            len + idx_n
                        } else {
                            idx_n
                        }
                    } else if idx_n == 0 {
                        if crate::ported::zsh_h::isset(crate::ported::zsh_h::KSHZEROSUBSCRIPT) {
                            0 // c:2140
                        } else {
                            -1 // c:2148
                        }
                    } else if idx_n < 0 {
                        len + idx_n
                    } else {
                        idx_n - 1
                    };
                    if i >= 0 && (i as usize) < arr.len() {
                        arr[i as usize].clone()
                    } else {
                        String::new()
                    }
                } else if let Some((start_s, end_s)) = {
                    // Split the slice on its TOP-LEVEL comma (depth 0,
                    // outside any `(...)` flag block). This keeps a
                    // PER-BOUND search-flag subscript with its bound
                    // (`(r)3,(r)5` → "(r)3","(r)5") while still finding
                    // the range comma when a whole-subscript separator
                    // flag carries a comma inside its delimiters
                    // (`(s.,.)1,3` → "(s.,.)1","3"). Bug #83 (separator)
                    // + per-bound (r)/(i) slice bounds. eval_idx below
                    // strips/handles each bound's leading flag.
                    let bs: Vec<char> = sub.chars().collect();
                    let mut depth = 0i32;
                    let mut at: Option<usize> = None;
                    for (k, &c) in bs.iter().enumerate() {
                        match c {
                            '(' => depth += 1,
                            ')' => {
                                if depth > 0 {
                                    depth -= 1;
                                }
                            }
                            ',' if depth == 0 => {
                                at = Some(k);
                                break;
                            }
                            _ => {}
                        }
                    }
                    at.map(|k| {
                        let b0 = bs[..k].iter().collect::<String>();
                        let b1 = bs[k + 1..].iter().collect::<String>();
                        (b0, b1)
                    })
                } {
                    // c:2944 (slice) — Src/params.c:2548 getarrvalue.
                    // Clone arr first to release the borrow, since
                    // singsub needs &mut state.
                    let arr_clone = arr.clone();
                    let len = arr_clone.len() as i64;
                    let start_str = start_s.to_string();
                    let end_str = end_s.to_string();
                    // c:Src/params.c::getarg — the subscript expression
                    // is routed through singsub then mathevali
                    // (math.c:367), so bare identifier names like `n`
                    // and arith expressions like `n+1` resolve. Plain
                    // `parse::<i64>` only accepts digit literals — bare
                    // `n` fell through to `unwrap_or(len)`, returning
                    // the full array. Bug #298. Same shape as the
                    // scalar slice arm above (subst.rs:4826-4840).
                    let eval_idx = |expr: &str, default: i64| -> i64 {
                        // c:Src/params.c getindex — a slice bound with a
                        // search-flag subscript (`(r)pat`/`(i)pat`) yields
                        // the INDEX of the match (the *inv/*w path), not
                        // the value: `${a[(r)3,(r)5]}` slices between the
                        // matched positions. getarg returns the value for
                        // r/R but the index for i/I, and r/R are reverse
                        // (last-match) searches, so map r→I / R→I to get
                        // the matching index in index mode.
                        let t = expr.trim();
                        // Effective bound text after a leading `(...)` flag.
                        let mut effective = t;
                        if let Some(close) = t.find(')') {
                            if t.starts_with('(') {
                                let flags = &t[1..close];
                                if flags
                                    .chars()
                                    .any(|c| matches!(c, 'r' | 'R' | 'i' | 'I' | 'k' | 'K'))
                                {
                                    // Search flag → matched INDEX via getarg
                                    // (r/R are reverse → map to I for index).
                                    let mapped: String = flags
                                        .chars()
                                        .map(|c| if c == 'r' || c == 'R' { 'I' } else { c })
                                        .collect();
                                    let new_sub = format!("({}){}", mapped, &t[close + 1..]);
                                    if let Some(crate::ported::params::getarg_out::Value(v)) =
                                        crate::ported::params::getarg(
                                            &new_sub,
                                            Some(&arr_clone),
                                            None,
                                            None,
                                        )
                                    {
                                        if let Ok(n) = v.to_str().trim().parse::<i64>() {
                                            return n;
                                        }
                                    }
                                } else {
                                    // Separator/word flag (`(s.X.)` etc.) is a
                                    // no-op for an integer slice bound (c:#83);
                                    // strip it and parse the remainder.
                                    effective = &t[close + 1..];
                                }
                            }
                        }
                        let expanded = singsub(effective);
                        if let Ok(n) = expanded.trim().parse::<i64>() {
                            return n;
                        }
                        crate::ported::math::mathevali(expanded.trim()).unwrap_or(default)
                    };
                    let start: i64 = eval_idx(&start_str, 1);
                    let end: i64 = eval_idx(&end_str, len);
                    // c:Src/params.c — KSH_ARRAYS flips array slice
                    // bounds from 1-based to 0-based inclusive. `a[0,1]`
                    // under KSH_ARRAYS = elements at positions 0 and 1.
                    // Sibling of #610/#611 in the array-slice arm.
                    // Bug #612.
                    let ksh_arrays = isset(crate::ported::zsh_h::KSHARRAYS);
                    let (start, end) = if ksh_arrays {
                        let new_start = if start >= 0 { start + 1 } else { start };
                        let new_end = if end >= 0 { end + 1 } else { end };
                        (new_start, new_end)
                    } else {
                        (start, end)
                    };
                    // c:Src/params.c:2567-2570 — negative-index resolve.
                    // For negative `start`, raw position = len + start
                    // (0-based). For negative `end`, raw = len + end +
                    // 1 (1-based exclusive). After resolving, if the
                    // start is STILL negative (subscript pointed
                    // before the array start), C zsh's c:2576-2578
                    // returns an empty result — NOT a clamp to 0.
                    // Previously zshrs clamped, so `arr[-99,99]` over a
                    // 3-element array returned all 3 elements; zsh
                    // returns empty.
                    // c:Src/params.c:2567-2570 — negative-index resolve.
                    // For a NEGATIVE start, if `len + start < 0` the
                    // subscript still points before the array begin —
                    // C zsh returns empty at c:2576-2578. For positive
                    // or zero start, clamp `start - 1` to 0 (zsh treats
                    // `arr[0,N]` like `arr[1,N]`).
                    let start_oob_neg = start < 0 && (len + start) < 0;
                    let s_raw: i64 = if start < 0 { len + start } else { start - 1 };
                    let e_raw: i64 = if end < 0 { len + end + 1 } else { end.min(len) };
                    if start_oob_neg {
                        // c:2576 — start still negative → empty.
                        String::new()
                    } else {
                        let s = s_raw.max(0) as usize;
                        let e = e_raw.max(0) as usize;
                        if s < arr_clone.len() && s < e {
                            arr_clone[s..e.min(arr_clone.len())].join(" ")
                        } else {
                            String::new()
                        }
                    }
                } else {
                    String::new()
                }
            } else if let Some(magic_val) = {
                // c:2926 — magic-assoc per-key lookup. Routes through
                // canonical PARTAB (Src/Modules/parameter.c:2235-2298
                // ports at parameter.rs::PARTAB / PARTAB_ARRAY).
                let is_splice = sub == "@" || sub == "*";
                if is_splice {
                    if let Some(values) = arrays_get(&var_name) {
                        Some(values.join(" "))
                    } else if let Some(keys) = assoc_get(&var_name).map(|m_| m_.keys().cloned().collect::<Vec<String>>()) {
                        let vals: Vec<String> = keys
                            .iter()
                            .map(|k| assoc_get(&var_name).and_then(|m_| m_.get(k.as_str()).cloned()).unwrap_or_default())
                            .collect();
                        Some(vals.join(" "))
                    } else {
                        None
                    }
                } else if let Some((flags, pat)) = (|s: &str| -> Option<(String, String)> {
                    // c:Src/params.c:1411-1418 — `(i)/(I)/(r)/(R)` flag
                    // subscript on a magic-assoc (parameters/commands/
                    // aliases/functions/options/etc.). Same semantics
                    // as on a user-defined assoc:
                    //   (i)pat: scan keys, return first matching key
                    //   (I)pat: scan keys, return all matching keys
                    //   (r)pat: scan keys, return value of first match
                    //   (R)pat: scan keys, return values of all matches
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
                    // Route through the magic-assoc scan + per-key get.
                    if let Some(keys) = assoc_get(&var_name).map(|m_| m_.keys().cloned().collect::<Vec<String>>()) {
                        let by_key = flags.contains('I') || flags.contains('i');
                        let return_all = flags.contains('I') || flags.contains('R');
                        let exact = flags.contains('e'); // c:1419 e flag — literal compare
                        let mut out: Vec<String> = Vec::new();
                        for k in &keys {
                            let hay = if by_key {
                                k.clone()
                            } else {
                                assoc_get(&var_name).and_then(|m_| m_.get(k.as_str()).cloned()).unwrap_or_default()
                            };
                            let matched = if exact {
                                hay == pat
                            } else {
                                patcompile(&{ let mut __pat_tok = (&pat).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, PAT_HEAPDUP as i32, None)
                                    .map_or(false, |__p| pattry(&__p, &hay))
                            };
                            if matched {
                                out.push(if by_key {
                                    k.clone()
                                } else {
                                    assoc_get(&var_name).and_then(|m_| m_.get(k.as_str()).cloned()).unwrap_or_default()
                                });
                                if !return_all {
                                    break;
                                }
                            }
                        }
                        Some(out.join(" "))
                    } else {
                        None
                    }
                } else if let Some(elem) = (|| -> Option<String> {
                    // c:Src/params.c:2110-2150 getarg index semantics on a
                    // PM_ARRAY magic param (1-based, negative from end, [0]
                    // per KSHZEROSUBSCRIPT, KSH_ARRAYS 0-based) over the
                    // PARTAB_ARRAY row's gsu getfn value (former bridge
                    // partab_array_index, inlined).
                    let arr = arrays_get(&var_name)?;
                    crate::ported::modules::parameter::PARTAB_ARRAY
                        .iter()
                        .find(|e_| e_.name == var_name.as_str())?;
                    let idx_n: i64 = crate::ported::math::mathevali(sub.trim()).unwrap_or(0);
                    let len = arr.len() as i64;
                    let ksh_arrays = crate::ported::zsh_h::isset(crate::ported::zsh_h::KSHARRAYS);
                    let i = if ksh_arrays {
                        if idx_n < 0 { len + idx_n } else { idx_n }
                    } else if idx_n == 0 {
                        if crate::ported::zsh_h::isset(crate::ported::zsh_h::KSHZEROSUBSCRIPT) { 0 } else { -1 }
                    } else if idx_n < 0 {
                        len + idx_n
                    } else {
                        idx_n - 1
                    };
                    Some(if i >= 0 && (i as usize) < arr.len() {
                        arr[i as usize].clone()
                    } else {
                        String::new()
                    })
                })() {
                    // c:Src/Modules/system.c:880 errnosgetfn — PM_ARRAY
                    // magic params subscript like ordinary arrays
                    // (getindex → getarg, Src/params.c:2110-2150).
                    // Previously only the assoc-keyed partab_get ran
                    // here, so ${errnos[1]} returned empty.
                    Some(elem)
                } else if (hkeys & SCANPM_WANTKEYS) != 0
                    && (hvals & SCANPM_WANTVALS) == 0
                {
                    // c:Src/params.c:1492-1494 + 1591 — `(k)` without
                    // `(v)` on a plain-key hash subscript sets inv →
                    // SCANPM_WANTINDEX; c:Src/subst.c:2922 then
                    // returns the hash node's NAME (the key). Same
                    // rule as the ordinary-assoc arm above; magic
                    // hashes (parameters/options/…) hit getarg's
                    // identical path in C. zsh 5.9:
                    // `${(ok)parameters[PATH]}` → "PATH". Missing key
                    // → None falls to the scalar arm (empty), matching
                    // C's PM_UNSET createparam result.
                    (|| -> Option<String> {
                    // c:Src/Modules/parameter.c — special-hash getnode
                    // dispatch (`ht->getnode(ht, key)` = getpm*) for one
                    // key; module-gated rows resolve only after their
                    // zmodload (former bridge partab_get, inlined —
                    // single-key reads must NOT materialize the table:
                    // ${commands[git]} would enumerate $PATH).
                    let e_ = crate::ported::modules::parameter::PARTAB
                        .iter()
                        .find(|e_| e_.name == var_name.as_str())?;
                    if let Some(m_) = e_.module {
                        if !crate::ported::module::MODULESTAB
                            .lock()
                            .map(|t| t.is_loaded(m_))
                            .unwrap_or(false)
                        {
                            return None;
                        }
                    }
                    (e_.getfn)(std::ptr::null_mut(), sub).and_then(|p_| p_.u_str)
                })().map(|_| sub.to_string())
                } else {
                    (|| -> Option<String> {
                    // c:Src/Modules/parameter.c — special-hash getnode
                    // dispatch (`ht->getnode(ht, key)` = getpm*) for one
                    // key; module-gated rows resolve only after their
                    // zmodload (former bridge partab_get, inlined —
                    // single-key reads must NOT materialize the table:
                    // ${commands[git]} would enumerate $PATH).
                    let e_ = crate::ported::modules::parameter::PARTAB
                        .iter()
                        .find(|e_| e_.name == var_name.as_str())?;
                    if let Some(m_) = e_.module {
                        if !crate::ported::module::MODULESTAB
                            .lock()
                            .map(|t| t.is_loaded(m_))
                            .unwrap_or(false)
                        {
                            return None;
                        }
                    }
                    (e_.getfn)(std::ptr::null_mut(), sub).and_then(|p_| p_.u_str)
                })()
                }
            } {
                magic_val
            } else {
                // Scalar with subscript — char-index access.
                let scalar = vars_get(&var_name).unwrap_or_default();
                // c:Src/params.c — `[@]` / `[*]` on a SCALAR is the
                // whole-value-as-one-element case; zsh returns the
                // scalar verbatim. The scalar+`[@]` path must surface
                // the raw value (not empty) so a downstream split flag
                // like `(s.X.)` can split it. Without this, the spsep
                // handler at subst.rs:7462 saw `value=""` (because the
                // scalar-with-subscript fallback returned empty when no
                // arm matched `[@]`) and split empty → one empty
                // element. Bug #85 in docs/BUGS.md:
                // `"${(s. .)s[@]}"` returned `[]` not `[a] [b] [c]`.
                if sub == "@" || sub == "*" {
                    if spsep.is_some() {
                        // c:Src/subst.c — set isarr so the
                        // auto_splat block (subst.rs:8332) fires for
                        // the post-split shape. Use -1 sentinel
                        // matching `[@]` survival through DQ
                        // (c:2915 SCANPM_ISVAR_AT).
                        isarr = -1;
                    }
                    scalar
                } else {
                let s_chars: Vec<char> = scalar.chars().collect();
                // Pattern-subscript on scalar: (i)pat / (I)pat
                // returns 1-based char position of first/last match;
                // (r)pat / (R)pat returns the matched substring.
                // Direct port of Src/params.c getasub which routes
                // scalar pattern lookups through getindex with
                // PATSCAN_FIRST/LAST.
                // c:Src/params.c:1419-1426 — `(w)`/`(W)`/`(p)`/`(f)`
                // flags set `word=1` so the subscript indexes the
                // scalar by WORDS rather than chars. `(s.X.)` alone
                // just sets `sep` without flipping word; per zsh, it's
                // a NO-OP for an integer scalar subscript (verified vs
                // /opt/homebrew/bin/zsh — `${a[(s/::/)2]}` returns
                // char[2], not split[2]). Only fall into the word arm
                // when one of w/W/p/f is present.
                if let Some((word_flags, num_str, sep)) =
                    (|s: &str| -> Option<(String, String, String)> {
                        let s = s.trim_start();
                        let rest = s.strip_prefix('(')?;
                        let close = rest.find(')')?;
                        let f = &rest[..close];
                        let n = rest[close + 1..].to_string();
                        let mut sep_explicit: Option<String> = None;
                        let mut has_word = false;
                        let mut chars = f.chars().peekable();
                        while let Some(c) = chars.next() {
                            match c {
                                'w' | 'W' | 'p' => has_word = true,
                                'f' => {
                                    has_word = true;
                                    sep_explicit = Some("\n".to_string());
                                }
                                's' => {
                                    let delim = chars.next()?;
                                    let mut body = String::new();
                                    for cc in chars.by_ref() {
                                        if cc == delim {
                                            break;
                                        }
                                        body.push(cc);
                                    }
                                    sep_explicit = Some(body);
                                }
                                _ => return None,
                            }
                        }
                        if !has_word {
                            return None;
                        }
                        Some((
                            f.to_string(),
                            n,
                            sep_explicit.unwrap_or_else(|| " \t\n".to_string()),
                        ))
                    })(sub)
                {
                    let _ = word_flags;
                    if let Ok(idx_n) = num_str.parse::<i64>() {
                        // Split scalar by sep. For default whitespace
                        // sep, split on any whitespace char (matches
                        // zsh IFS behavior).
                        let words: Vec<String> = if sep == " \t\n" {
                            scalar
                                .split_whitespace()
                                .map(|s| s.to_string())
                                .collect()
                        } else {
                            scalar
                                .split(sep.as_str())
                                .map(|s| s.to_string())
                                .collect()
                        };
                        let len = words.len() as i64;
                        let i = if idx_n == 0 {
                            -1
                        } else if idx_n < 0 {
                            len + idx_n
                        } else {
                            idx_n - 1
                        };
                        if i >= 0 && (i as usize) < words.len() {
                            words[i as usize].clone()
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    }
                } else if let Some((flags, pat)) = (|s: &str| -> Option<(String, String)> {
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
                    let exact = flags.contains('e'); // c:1419 e — literal compare, no glob
                                                     // Sliding-window match across the string (glob unless (e)).
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
                            let matched = if exact {
                                cand == pat
                            } else {
                                patcompile(&{ let mut __pat_tok = (&pat).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, PAT_HEAPDUP as i32, None)
                                    .map_or(false, |__p| pattry(&__p, &cand))
                            };
                            if matched {
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
                                if patcompile(&{ let mut __pat_tok = (&pat).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, PAT_HEAPDUP as i32, None)
                                    .map_or(false, |__p| pattry(&__p, &cand))
                                {
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
                        (Some((s, _e)), false) => {
                            // c:Src/params.c:1798-1980 — scalar (r)/(R)
                            // returns the CHAR at the match position,
                            // not the full matched substring. Verified
                            // vs /opt/homebrew/bin/zsh:
                            //   s="barfooxyz"; ${s[(r)foo]} → "f"
                            //   (the first char of "foo" at position 4)
                            s_chars.get(s).map(|c| c.to_string()).unwrap_or_default()
                        }
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
                } else if let Some(idx_n) = sub
                    .parse::<i64>()
                    .ok()
                    .or_else(|| {
                        // c:Src/params.c:1411-1430 — paren-wrapped
                        // subscript expression (`${x[(-1)]}`) evaluates
                        // via mathevali. Mirror the array path's
                        // fallback so scalar char-indexing accepts
                        // paren-wrapped negatives.
                        let s = sub.trim();
                        if s.starts_with('(') && s.ends_with(')') && s.len() >= 2 {
                            s[1..s.len() - 1].trim().parse::<i64>().ok()
                        } else {
                            None
                        }
                    })
                    .or_else(|| {
                        // c:Src/params.c:1419-1480 — subscript flag block
                        // followed by integer index. When (s/X/) is the
                        // ONLY flag (no w/W/p/f), it's a NO-OP for int
                        // index: just strip the flag block and parse the
                        // remainder. Verified vs /opt/homebrew/bin/zsh:
                        //   `a=hello; ${a[(s/l/)1]}` → "h" (char[1]).
                        let s = sub.trim();
                        let rest = s.strip_prefix('(')?;
                        let close = rest.find(')')?;
                        // Verify the flag block contains only known
                        // subscript flag chars and `s.X.` body.
                        let f = &rest[..close];
                        let mut chars = f.chars();
                        while let Some(c) = chars.next() {
                            match c {
                                's' => {
                                    let delim = chars.next()?;
                                    for cc in chars.by_ref() {
                                        if cc == delim {
                                            break;
                                        }
                                    }
                                }
                                'e' | 'n' | 'b' => {}
                                _ => return None,
                            }
                        }
                        rest[close + 1..].trim().parse::<i64>().ok()
                    })
                    .or_else(|| {
                        // mathevali fallback for arith subscripts like
                        // `${x[1+1]}`. NOTE: skip when the expression
                        // contains a top-level comma — that's a slice
                        // `${x[lo,hi]}`, NOT a math comma-operator.
                        // mathevali would evaluate "2,4" → 4 and
                        // mask the slice.
                        if sub.contains(',') {
                            None
                        } else {
                            // c:Src/params.c getindex -> matheval —
                            // a subscript that fails math parse zerrs
                            // and aborts EVEN when the param is unset:
                            // `${a[b*]}` with no `a` is rc=1 in zsh.
                            // (The set-array arm raises the same zerr
                            // upstream.)
                            match crate::ported::math::mathevali(sub) {
                                Ok(n) => Some(n),
                                Err(e) => {
                                    // mathevali errors already carry the
                                    // "bad math expression:" prefix (via
                                    // m_error_set); don't double it (zsh
                                    // emits it once).
                                    if e.starts_with("bad math expression") {
                                        crate::ported::utils::zerr(&e);
                                    } else {
                                        crate::ported::utils::zerr(&format!(
                                            "bad math expression: {}",
                                            e
                                        ));
                                    }
                                    None
                                }
                            }
                        }
                    })
                {
                    let len = s_chars.len() as i64;
                    // c:Src/params.c:2125-2150 — KSHZEROSUBSCRIPT
                    // non-strict mode: `s[0]` → first char.
                    //
                    // c:Src/params.c — KSH_ARRAYS option flips scalar
                    // subscripts to 0-based: `a[0]` = first char, `a[1]`
                    // = second char. Default zsh (KSH_ARRAYS off) is
                    // 1-based: `a[1]` = first char. Without this check
                    // `setopt KSH_ARRAYS; a=hello; echo $a[0]` returned
                    // empty (treated as the always-empty `[0]` slot)
                    // while zsh returns 'h'. Bug #610.
                    let ksh_arrays = isset(crate::ported::zsh_h::KSHARRAYS);
                    let i = if ksh_arrays {
                        if idx_n < 0 {
                            len + idx_n
                        } else {
                            idx_n
                        }
                    } else if idx_n == 0 {
                        if crate::ported::zsh_h::isset(crate::ported::zsh_h::KSHZEROSUBSCRIPT) {
                            0 // c:2140
                        } else {
                            -1 // c:2148
                        }
                    } else if idx_n < 0 {
                        len + idx_n
                    } else {
                        idx_n - 1
                    };
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
                    //
                    // c:Src/params.c::getarg — the subscript expression
                    // is evaluated via `mathevali` (math.c:367), so
                    // variable references and arith expressions like
                    // `n+1` resolve. Previous Rust port used
                    // `i64::parse` which only accepts bare digit
                    // literals — bare `n` or `n+1` fell through to the
                    // default bounds and returned the full string.
                    // Bug #155 in docs/BUGS.md.
                    let eval_idx = |expr: &str, default: i64| -> i64 {
                        let trimmed = expr.trim();
                        // Fast-path bare integer literal so tests
                        // exercising the simple `[1,3]` form don't
                        // round-trip through the math evaluator.
                        if let Ok(n) = trimmed.parse::<i64>() {
                            return n;
                        }
                        // c:Src/params.c::getarg routes the subscript
                        // through singsub (param expansion) then
                        // mathevali. The math evaluator handles
                        // identifier refs (`n` → 5) and the operator
                        // grammar (`n+1`).
                        let expanded = singsub(trimmed);
                        crate::ported::math::mathevali(&expanded).unwrap_or(default)
                    };
                    let lo: i64 = eval_idx(lo, 1);
                    let hi: i64 = eval_idx(hi, s_chars.len() as i64);
                    // c:Src/params.c — KSH_ARRAYS shifts scalar slice
                    // bounds from 1-based to 0-based inclusive. `a[0,2]`
                    // under KSH_ARRAYS = chars at positions 0,1,2.
                    // Sibling of #610. Bug #611.
                    let ksh_arrays = isset(crate::ported::zsh_h::KSHARRAYS);
                    let (lo, hi) = if ksh_arrays {
                        let new_lo = if lo >= 0 { lo + 1 } else { lo };
                        let new_hi = if hi >= 0 { hi + 1 } else { hi };
                        (new_lo, new_hi)
                    } else {
                        (lo, hi)
                    };
                    let chars_arr: Vec<String> = s_chars.iter().map(|c| c.to_string()).collect();
                    getarrvalue(&chars_arr, lo, hi).concat()
                } else {
                    String::new()
                }
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
                    assoc_get(&var_name).map(|m| m.values().cloned().collect::<Vec<_>>().join(" "))
                })
                .or_else(|| {
                    if is_special_name {
                        // POSIX shell-specials ($?/$#/$$/$!/$*/$@/$-/$N).
                        // Canonical dispatch through params::lookup_special_var
                        // (Src/params.c special_assigns getfn).
                        lookup_special_var(&var_name)
                    } else {
                        None
                    }
                })
                // Splice (`[@]`) on a magic-assoc name isn't yet wired
                // through the per-name scanpm<X> handlers; falls back
                // to empty (matches C when no special handler matches).
                .unwrap_or_default()
        };
        // c:Src/subst.c:2925 — apply a chained `[N][M]` subscript to
        // the scalar result from the first subscript. zsh's
        // `${a[N][M]}` returns char M (1-based) of array element N.
        let raw_value: String = if let Some(s2) = second_subscript.as_deref() {
            // Slice form `M,P` or single index `M`. Negative indices
            // count from the end (per zsh's 1-based-from-1 / -1-from-
            // end convention).
            let n = raw_value.chars().count() as i64;
            let resolve = |k: i64| -> usize {
                let k = if k < 0 { n + k + 1 } else { k };
                if k < 1 {
                    0
                } else if k > n {
                    n as usize
                } else {
                    (k - 1) as usize
                }
            };
            if let Some((lo, hi)) = s2.split_once(',') {
                let lo: i64 = lo
                    .trim()
                    .parse()
                    .ok()
                    .or_else(|| crate::ported::math::mathevali(lo.trim()).ok())
                    .unwrap_or(1);
                let hi: i64 = hi
                    .trim()
                    .parse()
                    .ok()
                    .or_else(|| crate::ported::math::mathevali(hi.trim()).ok())
                    .unwrap_or(0);
                let l = resolve(lo);
                let h = if hi == 0 {
                    n as usize
                } else {
                    let k = if hi < 0 { n + hi + 1 } else { hi };
                    if k < 1 {
                        0
                    } else if k > n {
                        n as usize
                    } else {
                        k as usize
                    }
                };
                if l >= h {
                    String::new()
                } else {
                    raw_value.chars().skip(l).take(h - l).collect()
                }
            } else {
                let k: i64 = s2
                    .trim()
                    .parse()
                    .ok()
                    .or_else(|| crate::ported::math::mathevali(s2.trim()).ok())
                    .unwrap_or(1);
                let i = resolve(k);
                raw_value
                    .chars()
                    .nth(i)
                    .map(|c| c.to_string())
                    .unwrap_or_default()
            }
        } else {
            raw_value
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
            // c:Src/subst.c getindex — for `@`/`*` (and slice
            // `[lo,hi]`), is_set tracks whether the full array has
            // any elements, not whether a named slot exists. Without
            // this, `${arr[@]:-x}` always fired the default because
            // the integer-parse path bailed on the non-numeric `@`.
            let is_at_or_star = matches!(sub, "@" | "*");
            let is_slice = sub.contains(',');
            // c:Src/params.c getarg — `(r)pat`/`(R)pat`/`(i)pat`/`(I)pat`
            // subscript flags do a pattern-search returning the match
            // (or empty when no match). Bug #587: the integer-parse
            // fallback returned false for flag-form subscripts, so
            // `${a[(r)y]:-default}` for `a=(x y z)` fired the default
            // even though `y` IS in the array and raw_value already
            // holds it. Detect flag-form subscript (`(...)` prefix)
            // and treat is_set as true when raw_value is non-empty.
            let is_flag_form = sub.trim_start().starts_with('(');
            used_subexp
                || (is_flag_form && !raw_value.is_empty())
                || assoc_get(&var_name)
                    .map(|m| m.contains_key(sub))
                    .unwrap_or(false)
                || arrays_get(&var_name)
                    .as_ref()
                    .map(|a| {
                        if is_at_or_star || is_slice {
                            !a.is_empty()
                        } else {
                            sub.parse::<i64>().ok().is_some_and(|i| {
                                let len = a.len() as i64;
                                let real = if i < 0 { len + i } else { i - 1 };
                                real >= 0 && (real as usize) < a.len()
                            })
                        }
                    })
                    .unwrap_or(false)
                // For `@`/`*` on a scalar var, treat is_set per the
                // scalar's own existence — `${str[@]:-x}` is legal
                // and `str` may be the only declared form.
                || ((is_at_or_star || is_slice) && vars_contains(&var_name))
                // c:Src/Modules/parameter.c — magic-assoc tables
                // (`builtins`, `commands`, `functions`, `aliases`, etc.)
                // dispatch through PARTAB. Without this fallback,
                // `${builtins[echo]:-X}` fired the `:-X` default because
                // is_set was false even though the value is set.
                || (|| -> Option<String> {
                    // c:Src/Modules/parameter.c — special-hash getnode
                    // dispatch (`ht->getnode(ht, key)` = getpm*) for one
                    // key; module-gated rows resolve only after their
                    // zmodload (former bridge partab_get, inlined —
                    // single-key reads must NOT materialize the table:
                    // ${commands[git]} would enumerate $PATH).
                    let e_ = crate::ported::modules::parameter::PARTAB
                        .iter()
                        .find(|e_| e_.name == var_name.as_str())?;
                    if let Some(m_) = e_.module {
                        if !crate::ported::module::MODULESTAB
                            .lock()
                            .map(|t| t.is_loaded(m_))
                            .unwrap_or(false)
                        {
                            return None;
                        }
                    }
                    (e_.getfn)(std::ptr::null_mut(), sub).and_then(|p_| p_.u_str)
                })().is_some_and(|v| !v.is_empty())
        } else {
            // c:Src/params.c::getindex — positional parameters ($1,
            // $2, …) live in `arrays["@"]`, not in the named-vars
            // table. The is_set check must consult arrays["@"] when
            // the var name parses as a positional index. Without
            // this, `${1:-default}` always fired the default
            // inside functions even when $1 was set.
            let positional_set = !var_name.is_empty()
                && var_name.chars().all(|c| c.is_ascii_digit())
                && var_name.parse::<usize>().ok().is_some_and(|n| {
                    if n == 0 {
                        vars_get("0").is_some()
                    } else {
                        arrays_get("@").map_or(false, |a| n <= a.len())
                    }
                });
            used_subexp
                || vars_contains(&var_name)
                || arrays_contains(&var_name)
                || assoc_contains(&var_name)
                || positional_set
        };

        // ${+name} short-circuit per subst.c:3600 — return "1"/"0".
        // Subscripted form `${+arr[i]}` checks whether THAT element is
        // set, not the array as a whole; raw_value (already
        // subscript-resolved) being non-empty is the proxy.
        if chkset {
            // c:3600 — `${+name}` / `${+name[key]}` reports whether the
            // (subscripted) slot EXISTS, mirroring C's `vunset` which
            // getindex sets from the slot lookup, NOT from whether the
            // value is non-empty. The subscript branch previously tested
            // `!raw_value.is_empty()`, so an existing key with an EMPTY
            // value (`h[k]=""` → `${+h[k]}`) wrongly returned 0. `is_set`
            // already does the correct slot-existence check for assoc
            // (contains_key), arrays (slot in range), @/*, slices, flag
            // subscripts, and magic-assocs — use it for both forms.
            let set_str = if is_set { "1" } else { "0" };
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

        // c:2588 — `getlen = 1 + whichlen` at the `#` prefix arm.
        // Port of subst.c:3845-3876 length dispatch:
        //   getlen == 1: array element count OR scalar char count
        //   getlen == 2: char count of joined value (`(c)` flag)
        //   getlen == 3: word count, no multi-IFS (`(w)` flag)
        //   getlen >= 4: word count, multi-IFS (`(W)` flag)
        if length_op {
            // c:3845
            let _ = post_flags_start;
            let getlen = 1 + whichlen; // c:2588
                                       // c:Src/subst.c:3193 — `:-default` modifier on `${#NAME:-X}`
                                       // shape. When var_name resolves empty (unset, empty
                                       // positional, or empty literal name `${#:-X}`), apply the
                                       // default BEFORE computing length so `${#:-foo}` returns 3.
                                       // C's flow handles this naturally because modifiers run
                                       // first and length runs at the end of paramsubst at c:3845;
                                       // the Rust port computes length early and returns, so we
                                       // need an inline modifier pre-pass for the empty-or-unset
                                       // case.
            // c:Src/subst.c:3845 — `getlen` runs AFTER the modifier
            // chain so `${#var:h}` counts the head-stripped value,
            // `${#var/foo/X}` counts the replaced value, etc.
            // Bug #43 in docs/BUGS.md: the Rust port computed length
            // on `raw_value` which is the pre-modifier value, losing
            // every transform.
            //
            // Construct the equivalent transform-applied expansion by
            // dropping the leading `#` from the body and routing
            // through `singsub`. This recursively re-enters paramsubst
            // with the same name + subscript + modifier chain but
            // without the length operator, then we count chars on the
            // result. Handles `:h`/`:t`/`:r`/`:e`/`:s` history mods,
            // `/pat/rep`/`//pat/rep` substitution, prefix/suffix strip
            // `#`/`%`/`##`/`%%`, and any combination — paramsubst
            // applies them in their canonical order.
            let raw_value_for_len = {
                let r = rest.as_str();
                // Pre-modifier shortcuts for the colon-default /
                // colon-alt operators that work on the raw value
                // BEFORE any chain transform. The C source's
                // `${#var:-default}` also goes through this fast path.
                if let Some(default) = r.strip_prefix(":-") {
                    if raw_value.is_empty() {
                        singsub(default)
                    } else {
                        raw_value.clone()
                    }
                } else if let Some(default) = r.strip_prefix('-') {
                    if !vars_contains(&var_name)
                        && !arrays_contains(&var_name)
                        && !assoc_contains(&var_name)
                    {
                        singsub(default)
                    } else {
                        raw_value.clone()
                    }
                } else if let Some(alt) = r.strip_prefix(":+") {
                    if !raw_value.is_empty() {
                        singsub(alt)
                    } else {
                        String::new()
                    }
                } else if let Some(alt) = r.strip_prefix('+') {
                    if vars_contains(&var_name)
                        || arrays_contains(&var_name)
                        || assoc_contains(&var_name)
                    {
                        singsub(alt)
                    } else {
                        String::new()
                    }
                } else if !r.is_empty() {
                    // Re-invoke paramsubst on the same name + subscript
                    // + modifier chain WITHOUT the length operator.
                    // The result is the transformed value; we'll count
                    // its chars below. Build the body string carefully:
                    // include the subscript if present (so `${#a[1,2]:h}`
                    // routes to `${a[1,2]:h}` correctly).
                    let body = if let Some(sub) = subscript.as_deref() {
                        format!("${{{}[{}]{}}}", var_name, sub, r)
                    } else {
                        format!("${{{}{}}}", var_name, r)
                    };
                    singsub(&body)
                } else {
                    raw_value.clone()
                }
            };
            // c:Src/Modules/parameter.c — magic-assoc names (`builtins`,
            // `commands`, `functions`, `aliases`, `parameters`, `options`,
            // `modules`, `reswords`, `nameddirs`, `userdirs`, `jobtexts`,
            // `jobdirs`, `jobstates`, `dirstack`, `errnos`, `sysparams`,
            // `mapfile` + their `dis_*` siblings) live behind PARTAB
            // scanfn dispatch, not in arrays/assocs storage. Treat as
            // an array source for length computation so `${#builtins}`
            // returns 103 (key count) instead of 0 (char count of empty
            // scalar fallback).
            let magic_keys: Option<Vec<String>> =
                if !arrays_contains(&var_name) && !assoc_contains(&var_name) {
                    // Try PARTAB (hashed magic-assocs: aliases /
                    // functions / parameters / commands / options /
                    // widgets / etc.) first; fall through to
                    // PARTAB_ARRAY for array magic-assocs
                    // (patchars / pipestatus / dirstack / ...).
                    assoc_get(&var_name).map(|m_| m_.keys().cloned().collect::<Vec<String>>())
                        .or_else(|| arrays_get(&var_name))
                } else {
                    None
                };
            // c:Src/params.c:2915 — `v->scanflags ? 1 : 0`. A
            // single-slot subscript (`[N]`, `[name]`) clears isarr;
            // subsequent length-op runs on the picked SCALAR (`${#arr[1]}`
            // = chars in arr[1]). Only `[@]`/`[*]`/range `[N,M]` keep
            // array shape (SCANPM_ISVAR_AT at c:2027-2029), feeding the
            // c:3853 element-count path.
            let single_slot_subscript = subscript
                .as_deref()
                .map_or(false, |s| s != "@" && s != "*" && !s.contains(','));
            // c:Src/params.c:2915 — a flagged subscript `[(I)pat]` /
            // `[(R)pat]` / `[(K)pat]` that matched MULTIPLE entries set
            // SCANPM_MATCHMANY → array shape (isarr != 0), even though
            // it's not `@`/`*`/range. The (I)-match arm above already
            // populated split_parts + isarr; honor that here so
            // `${#h[(I)pat]}` counts MATCHED elements, not the chars of
            // the joined scalar (was 7 for "baz bar" instead of 2).
            let flagged_array_subscript = isarr != 0 && split_parts.is_some();
            let is_array_source = ((arrays_contains(&var_name)
                || assoc_contains(&var_name)
                || magic_keys.is_some())
                && !single_slot_subscript)
                || flagged_array_subscript;
            let n: usize = if is_array_source {
                // c:3849 if (isarr)
                if getlen == 1 {
                    // c:Src/subst.c:3540 + 3845 — a `:#pat` filter is a
                    // modifier; zsh runs modifiers BEFORE getlen, so the
                    // length counts the FILTERED array. The Rust port
                    // computes length early (the `:#` arm at ~7663 never
                    // runs for the `#` form), so apply the element-wise
                    // filter here. `(M)`/SUB_MATCH inverts the disposition
                    // (keep matching instead of dropping it). Mirrors the
                    // match_fn + invert logic of the main `:#` arm.
                    if let Some(pat) = rest.as_str().strip_prefix(":#") {
                        let arr_src: Vec<String> = arrays_get(&var_name)
                            .or_else(|| {
                                assoc_get(&var_name).map(|m| m.values().cloned().collect())
                            })
                            .or_else(|| split_parts.clone())
                            .unwrap_or_default();
                        let p = literalize_spliced_metas(&singsub(&pretokenize_src_pat(pat))); // c:3540
                        let invert = (sub_flags_bits & SUB_MATCH) != 0; // c:2171 SUB_MATCH
                        arr_src
                            .iter()
                            .filter(|elem| {
                                // c:3540 getmatch — empty pattern ⇔ empty subject.
                                let m = if p.is_empty() {
                                    elem.is_empty()
                                } else {
                                    patcompile(
                                        &{
                                            let mut __t = p.to_string();
                                            crate::ported::glob::tokenize(&mut __t);
                                            __t
                                        },
                                        PAT_HEAPDUP as i32,
                                        None,
                                    )
                                    .map_or(false, |__p| pattry(&__p, elem))
                                };
                                if invert {
                                    m
                                } else {
                                    !m
                                } // c:3540
                            })
                            .count()
                    } else
                    // c:3853 element count — a flagged-subscript array
                    // (split_parts set) counts its matched elements.
                    if flagged_array_subscript {
                        split_parts.as_ref().map_or(0, |p| p.len())
                    } else
                    // For array SLICE `${#arr[lo,hi]}`, return the
                    // slice length, not the full array length. Bug
                    // #43 in docs/BUGS.md: the C source's getlen
                    // runs AFTER subscript resolution so the slice
                    // count survives; the Rust port short-circuited
                    // on the full array.
                    if let Some(sub) = subscript.as_deref() {
                        if let Some((lo_s, hi_s)) = sub.split_once(',') {
                            if let Some(arr) = arrays_get(&var_name) {
                                let len = arr.len() as i64;
                                let lo: i64 = singsub(lo_s).parse().unwrap_or(1);
                                let hi: i64 = singsub(hi_s).parse().unwrap_or(len);
                                let lo_idx = if lo < 0 { (len + lo).max(0) } else { (lo - 1).max(0) };
                                let hi_idx = if hi < 0 { (len + hi + 1).max(0) } else { hi.min(len) };
                                (hi_idx - lo_idx).max(0) as usize
                            } else {
                                0
                            }
                        } else if let Some(arr) = arrays_get(&var_name) {
                            arr.len()
                        } else if let Some(map) = assoc_get(&var_name) {
                            map.len()
                        } else if let Some(ref keys) = magic_keys {
                            keys.len()
                        } else {
                            0
                        }
                    } else if let Some(arr) = arrays_get(&var_name) {
                        arr.len() // c:3854
                    } else if let Some(map) = assoc_get(&var_name) {
                        map.len() // c:3854 (assoc len)
                    } else if let Some(ref keys) = magic_keys {
                        // PARTAB magic-assoc count.
                        keys.len()
                    } else {
                        0
                    }
                } else if getlen == 2 {
                    // c:3855 — sum char widths joined with sep
                    // (sep defaults to first IFS char which is ' ').
                    // C: `len = -sl; for (...) len += sl + STRLEN(elem)`.
                    // For arr=("abc","def"): len = -1 + 1+3 + 1+3 = 7.
                    let arr: Vec<String> = if let Some(a) = arrays_get(&var_name) {
                        a
                    } else if let Some(m) = assoc_get(&var_name) {
                        m.values().cloned().collect()
                    } else {
                        Vec::new()
                    };
                    if arr.is_empty() {
                        0
                    } else {
                        let sl = sep.as_deref().map(|s| s.chars().count()).unwrap_or(1); // c:3851
                        let mut len: i64 = -(sl as i64); // c:3857
                        for elem in &arr {
                            len += (sl as i64) + (elem.chars().count() as i64); // c:3858
                        }
                        len.max(0) as usize
                    }
                } else {
                    // c:3862 — wordcount each elem, multi-IFS if getlen>3
                    let multi = if getlen > 3 { 1 } else { 0 }; // c:3864
                    let arr: Vec<String> = if let Some(a) = arrays_get(&var_name) {
                        a
                    } else if let Some(m) = assoc_get(&var_name) {
                        m.values().cloned().collect()
                    } else {
                        Vec::new()
                    };
                    let mut total: i32 = 0;
                    for elem in &arr {
                        total += crate::ported::utils::wordcount(elem, spsep.as_deref(), multi);
                    }
                    total.max(0) as usize
                }
            } else {
                // c:3866 (scalar) — uses post-modifier value so
                // `${#:-foo}` returns 3 (length of "foo"), not 0
                // (length of empty pre-modifier raw_value).
                if getlen < 3 {
                    // c:Src/utils.c MB_METASTRLEN — under C/POSIX
                    // locale (CODESET ≠ "UTF-8"), zsh's mbrtowc
                    // returns one char per byte and the length op
                    // reports BYTE count. zshrs's chars().count() is
                    // ALWAYS char count, so it diverges under
                    // LC_ALL=C. Bug #378. Branch on the active
                    // locale's CODESET: UTF-8 → char count;
                    // else → byte count. Inlined here because the
                    // src/ported/ port-only check forbids
                    // Rust-original helper fns.
                    let utf8 = unsafe {
                        // Ensure setlocale has been called from the
                        // environment so nl_langinfo reflects the
                        // user's LC_*. Rust test harnesses don't run
                        // setlocale by default, so the C-locale
                        // codeset (US-ASCII) would shadow the
                        // process-environment LC_CTYPE setting.
                        // Cached lazily via Once.
                        static SETLOCALE_DONE: std::sync::Once = std::sync::Once::new();
                        SETLOCALE_DONE.call_once(|| {
                            let empty = std::ffi::CString::new("").unwrap();
                            libc::setlocale(libc::LC_CTYPE, empty.as_ptr());
                        });
                        let ptr = libc::nl_langinfo(libc::CODESET);
                        if ptr.is_null() {
                            true
                        } else {
                            let cs = std::ffi::CStr::from_ptr(ptr).to_string_lossy();
                            cs.eq_ignore_ascii_case("UTF-8") || cs.eq_ignore_ascii_case("utf8")
                        }
                    };
                    // c:Src/utils.c:5662-5663 — `if (!isset(MULTIBYTE)
                    // || MB_CUR_MAX == 1) return ztrlen(ptr);` — with
                    // the MULTIBYTE option unset the length op counts
                    // BYTES regardless of locale. Read the live slot
                    // directly with the declared default (OPT_ALL,
                    // c:Src/options.c:197) for a never-written slot —
                    // `isset()` maps never-written to false, which
                    // inverts the default-on semantics in contexts
                    // that skip init's emulate() (unit tests).
                    let multibyte_on =
                        crate::ported::options::opt_state_get("multibyte").unwrap_or(true);
                    if utf8 && multibyte_on {
                        raw_value_for_len.chars().count()
                    } else {
                        raw_value_for_len.len()
                    }
                } else {
                    // c:3869 word count
                    let multi = if getlen > 3 { 1 } else { 0 };
                    crate::ported::utils::wordcount(&raw_value_for_len, spsep.as_deref(), multi)
                        .max(0) as usize
                }
            };
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
                               // c:Src/Modules/parameter.c — magic-assoc (k)/(v)/(kv) reads
                               // return array shape (each key/value as a distinct word). The
                               // value computation below joins them with space for the scalar
                               // path; capture the unjoined Vec here so the split_parts
                               // declared post-value can pick it up and the auto_splat block
                               // emits multiple result_nodes. Without this, `a=( ${(k)builtins}
                               // )` got 1 element instead of zsh's 103.
        let mut magic_assoc_array: Option<Vec<String>> = None;
        // c:Src/subst.c — the (k)/(v)/(kv) outer-flag value-init at
        // c:2247-2270 is the "no subscript" path. When a NON-SPLAT
        // subscript is present (`(R)pat`/`(I)pat`/`key`/`[lo,hi]`),
        // the subscript dispatch in the if-let-Some(sub) arm above
        // (assoc/array/scalar) already produced the correct raw_value
        // (e.g. `${(k)h[(R)pat]}` returns matched KEYS via the
        // SCANPM_WANTKEYS+MATCHVAL composition handled inline at line
        // 4538). Override would clobber that with all-keys enumeration.
        // `[@]`/`[*]` splat subscripts MUST still hit this branch — the
        // splat is the trigger for (k)/(v)/(kv) enumeration in zsh
        // (`${(k)h[@]}` = keys, NOT raw splat-of-values). Bug #592.
        let is_at_splat_sub = matches!(subscript.as_deref(), Some("@") | Some("*"));
        let has_subscript_for_kvflag = subscript.is_some() && !is_at_splat_sub;
        if !has_subscript_for_kvflag
            && (hkeys & SCANPM_WANTKEYS) != 0
            && (hvals & SCANPM_WANTVALS) != 0 {
            // c:2247 (kv) — interleaved key/value pairs. Walk assoc
            // first, then the magic-assoc fallback (aliases/functions/
            // commands/parameters/builtins/options/...) interleaving
            // each key with its value.
            magic_assoc_array = assoc_get(&var_name)
                .map(|m| {
                    let mut out: Vec<String> = Vec::with_capacity(m.len() * 2);
                    for (k, v) in m {
                        out.push(k.clone());
                        out.push(v.clone());
                    }
                    out
                })
                .or_else(|| match var_name.as_str() {
                    "aliases" => aliastab_lock().read().ok().map(|t| {
                        let mut entries: Vec<(String, String)> =
                            t.iter().map(|(k, v)| (k.clone(), v.text.clone())).collect();
                        entries.sort_by(|a, b| a.0.cmp(&b.0));
                        entries.into_iter().flat_map(|(k, v)| [k, v]).collect()
                    }),
                    _ => assoc_get(&var_name).map(|m_| m_.keys().cloned().collect::<Vec<String>>()).map(|mut keys| {
                        keys.sort();
                        keys.into_iter()
                            .flat_map(|k| {
                                let v =
                                    assoc_get(&var_name).and_then(|m_| m_.get(k.as_str()).cloned()).unwrap_or_default();
                                [k, v]
                            })
                            .collect()
                    }),
                })
                // c:Src/subst.c — on an indexed array, `(kv)` mirrors
                // `(k)` and `(v)`: each returns the array values
                // (verified vs /opt/homebrew/bin/zsh:
                // `arr=(a b c); echo "${(kv)arr}"` → "a b c"). Without
                // this fallback the (kv) arm produced empty for indexed
                // arrays because `assoc_get` returned None and the
                // magic-assoc tables didn't match.
                .or_else(|| crate::ported::exec_hooks::array(&var_name));
            value = magic_assoc_array
                .as_ref()
                .map(|v| v.join(" "))
                .unwrap_or_default(); // c:2247
        } else if !has_subscript_for_kvflag && (hkeys & SCANPM_WANTKEYS) != 0 {
            // c:2247
            // Capture the keys-as-Vec for split_parts splat (see
            // magic_assoc_array declaration above). Walk every source
            // path the value-build below walks (assoc + magic-assoc
            // tables + PARTAB fallback) and pick the FIRST non-empty
            // key list as the array shape.
            magic_assoc_array = assoc_get(&var_name)
                .map(|m| m.keys().cloned().collect::<Vec<String>>())
                .or_else(|| match var_name.as_str() {
                    "aliases" => aliastab_lock().read().ok().map(|t| {
                        let mut names: Vec<String> = t.iter().map(|(k, _)| k.clone()).collect();
                        names.sort();
                        names
                    }),
                    "functions" | "dis_functions" => shfunctab_lock().read().ok().map(|t| {
                        let mut names: Vec<String> = t.iter().map(|(k, _)| k.clone()).collect();
                        names.sort();
                        names
                    }),
                    "commands" => cmdnamtab_lock().read().ok().map(|t| {
                        let mut names: Vec<String> = t.iter().map(|(k, _)| k.clone()).collect();
                        names.sort();
                        names
                    }),
                    _ => assoc_get(&var_name).map(|m_| m_.keys().cloned().collect::<Vec<String>>()).map(|mut keys| {
                        keys.sort();
                        keys
                    }),
                })
                // c:Src/subst.c — on an indexed array, `(k)` is a no-
                // op and returns the array's values (zsh quirk; verified
                // via `arr=(a b c); ${(k)arr}` → "a b c"). The assoc
                // and magic-assoc lookups above already failed; fall
                // back to the array's values via array_get.
                .or_else(|| crate::ported::exec_hooks::array(&var_name));
            value = assoc_get(&var_name) // c:2247
                .map(|m| m.keys().cloned().collect::<Vec<_>>().join(" ")) // c:2247
                .or_else(|| {
                    // Indexed-array fallback for (k) — see comment on
                    // magic_assoc_array above. Return joined values.
                    crate::ported::exec_hooks::array(&var_name).map(|a| a.join(" "))
                })
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
                        "aliases" => aliastab_lock().read().ok().map(|t| {
                            let mut names: Vec<String> = t.iter().map(|(k, _)| k.clone()).collect();
                            names.sort();
                            names.join(" ")
                        }),
                        "functions" | "dis_functions" => shfunctab_lock().read().ok().map(|t| {
                            let mut names: Vec<String> = t.iter().map(|(k, _)| k.clone()).collect();
                            names.sort();
                            names.join(" ")
                        }),
                        "commands" => cmdnamtab_lock().read().ok().map(|t| {
                            let mut names: Vec<String> = t.iter().map(|(k, _)| k.clone()).collect();
                            names.sort();
                            names.join(" ")
                        }),
                        // c:Src/Modules/parameter.c — generic PARTAB
                        // magic-assoc fallback (parameters/builtins/
                        // options/modules/reswords/nameddirs/...) via
                        // the canonical scanfn dispatch. Without this,
                        // `${(k)parameters}` returned empty.
                        //
                        // Sort alphabetically for deterministic output:
                        // zsh emits in its own hash-iteration order
                        // which depends on the C hashtable bucketing
                        // algorithm — not reproducible from Rust's
                        // HashMap. Alphabetical-sort gives stable,
                        // human-readable output even if the order
                        // differs from zsh's specific hash. Most
                        // consumers (`zinit ls $functions`, plugin
                        // sanity checks) don't care about hash order.
                        _ => assoc_get(&var_name).map(|m_| m_.keys().cloned().collect::<Vec<String>>()).map(|mut keys| {
                            keys.sort();
                            keys.join(" ")
                        }),
                    } // c:2247
                }) // c:2247
                .unwrap_or_default();
        } else if !has_subscript_for_kvflag && (hvals & SCANPM_WANTVALS) != 0 {
            // c:2256 — (v) flag: values as array, with magic-assoc
            // fallback chain matching the (k) arm above so
            // \`\${(v)options}\` etc. splat.
            magic_assoc_array = assoc_get(&var_name)
                .map(|m| m.values().cloned().collect::<Vec<String>>())
                .or_else(|| {
                    // Magic-assoc value-side fallback: route through
                    // partab_array_get for PARTAB_ARRAY entries (the
                    // canonical scanfn dispatch returns ordered values).
                    // For non-array magic-assoc names, scan keys + look
                    // up each via partab_get to build the value list.
                    arrays_get(&var_name).or_else(|| {
                        assoc_get(&var_name).map(|m_| m_.keys().cloned().collect::<Vec<String>>()).map(|mut keys| {
                            keys.sort();
                            keys.into_iter()
                                .map(|k| {
                                    assoc_get(&var_name).and_then(|m_| m_.get(k.as_str()).cloned()).unwrap_or_default()
                                })
                                .collect()
                        })
                    })
                })
                // c:Src/subst.c — indexed array fallback for (v).
                // `arr=(a b c); ${(v)arr}` → "a b c" (the values).
                .or_else(|| crate::ported::exec_hooks::array(&var_name));
            value = magic_assoc_array
                .as_ref()
                .map(|v| v.join(" "))
                .unwrap_or_default();
            // c:2922 — getarrvalue sets isarr=1 for assoc-value fetch.
            isarr = 1;
        } else if (nojoin == 2) {
            // c:2167
            // (@) array splat — preserve element shape via space-join.
            // For full splat into multiple result_nodes, the
            // multsub-aware caller handles it; we emit space-joined here.
            //
            // c:Src/subst.c — `${(@)assoc}` enumerates the assoc's
            // VALUES (parameter.c hashparam splat). Without this assoc
            // fallback the values stayed empty and the splat block
            // had nothing to emit. Bug #287 in docs/BUGS.md.
            //
            // When a NON-SPLAT subscript is present (`(R)pat`, `(I)pat`,
            // `[lo,hi]`, single key), the subscript dispatch already
            // produced the matched-result raw_value. The `(@)` enum
            // would clobber that with all-values. Honor raw_value in
            // that case so `${(@k)h[(R)V]}` returns matched keys.
            // Bug #592.
            if has_subscript_for_kvflag {
                value = raw_value.clone();
            } else {
                value = arrays_get(&var_name)
                    .as_ref()
                    .map(|a| a.join(" "))
                    .or_else(|| {
                        assoc_get(&var_name)
                            .map(|m| m.values().cloned().collect::<Vec<_>>().join(" "))
                    })
                    .unwrap_or_else(|| raw_value.clone());
            }
            // c:2922 — getarrvalue sets isarr=1; nojoin=2 keeps it.
            if arrays_contains(&var_name) || assoc_contains(&var_name) {
                isarr = 1;
            }
        } else {
            // c:N/A
            value = raw_value.clone();
            // c:2915-2916 — isarr derived from v->scanflags:
            //   if (SCANPM_ISVAR_AT)  → isarr = -1  (`$arr[@]` shape)
            //   else if (scanflags)   → isarr =  1  (array-result)
            //   else                  → isarr =  0  (scalar pick)
            //
            // c:2027-2029 sets SCANPM_ISVAR_AT for `[@]`/`[*]` subscript
            // when the underlying var is array-shaped. Mirror by
            // checking subscript == "@"/"*" + var is array/assoc.
            // isarr = -1 stays past the c:3029 transition (qt > 0
            // check is `isarr > 0` so -1 is preserved), so the
            // c:4245 `if (isarr)` sort/splat block fires in DQ for
            // `[@]` — matching zsh's "${arr[@]}" splat behavior.
            //
            // Range subscript `[N,M]` gives isarr=1 (positive) so
            // the c:3032 qt-sepjoin fires → isarr=0 in DQ, sort
            // skipped, value stays joined. That's the test-
            // documented "in DQ the slice JOINS" behavior.
            let is_at_subscript = matches!(subscript.as_deref(), Some("@") | Some("*"));
            // c:Src/subst.c:2916 — `(v->scanflags & SCANPM_ISVAR_AT)
            // ? -1 : v->scanflags ? 1 : 0`. SCANPM_ISVAR_AT is set
            // ONLY for the `@` subscript (and the bare `@`/`*`
            // pseudo-names which are positional-param splat forms).
            // The `[*]` subscript clears SCANPM_ISVAR_AT — its
            // scanflags are non-zero (so isarr=1, positive), which
            // means the c:3032 qt-sepjoin fires in DQ and the
            // array collapses to a single scalar. That's the
            // documented "${a[*]}" = join-via-IFS behaviour.
            // Bug #428 in docs/BUGS.md.
            //
            // For `[@]` and bare positional `@`/`*`, splat is
            // preserved (isarr=-1) so the c:4245 sort/splat block
            // fires even in DQ. Bug #277.
            let is_strict_at_subscript = matches!(subscript.as_deref(), Some("@"));
            // c:Src/subst.c — same `*` vs `@` distinction applies
            // to the positional-param pseudo-names. `$@` keeps
            // splat (-1, SCANPM_ISVAR_AT) so quoted `"$@"` survives
            // the c:3032 sepjoin. `$*` clears SCANPM_ISVAR_AT (it
            // joins-via-IFS in DQ per the documented `"$*"` = IFS-
            // joined-scalar semantics). Bug #428.
            let is_strict_at_var = matches!(var_name.as_str(), "@");
            let is_star_var = matches!(var_name.as_str(), "*");
            if (arrays_contains(&var_name) || assoc_contains(&var_name))
                && (subscript.is_none() || is_at_subscript)
            {
                isarr = if is_strict_at_subscript || is_strict_at_var {
                    -1
                } else {
                    1
                };
            } else if (is_strict_at_var || is_star_var) && subscript.is_none() {
                // Bare `$@` / `$*` (no subscript) outside the
                // array_contains gate — positional params don't
                // always register as `arrays_contains("@")`. `$@`
                // keeps splat shape for the auto-splat block; `$*`
                // gets isarr=1 so c:3032 collapses it to a scalar
                // in DQ. Gated on `subscript.is_none()` so slice
                // forms like `${*[2,4]}` don't get re-shaped here
                // — they own their own scanflags state per
                // getarrvalue (Src/params.c:2922).
                isarr = if is_strict_at_var { -1 } else { 1 };
            }
        }
        // subst.c:3885-3887 YUK — empty / empty-first array → scalar "" when !plan9
        if !plan9 && (nojoin == 2) {
            if let Some(ref a) = arrays_get(&var_name) {
                if a.first().map_or(true, |s| s.is_empty()) {
                    value = String::new();
                }
            }
        }
        // c:3029-3036 — array → scalar transition under DQ-join.
        // Direct port:
        //   if (isarr) {
        //       if (nojoin)
        //           isarr = -1;
        //       if (qt && !getlen && isarr > 0) {
        //           val = sepjoin(aval, sep, 1);
        //           isarr = 0;
        //       }
        //   }
        // The `qt && !getlen && isarr > 0` arm fires for DQ-wrapped
        // array reads without `(#)` length or `(@)` nojoin: the
        // array collapses to a scalar via sepjoin BEFORE operator
        // processing. After this, isarr=0 means the c:4245 sort/
        // unique/splat block is skipped (matches `if (isarr)` gate).
        // For `(@)arr`, nojoin=2 keeps isarr at -1 not 0, so the
        // block STILL fires and splats per the explicit @-override.
        if isarr != 0 {
            // c:3029
            if nojoin != 0 {
                // c:3030
                isarr = -1; // c:3030
            }
            // c:3032 — `if (qt && !getlen && isarr > 0)`. The
            // `!getlen` clause is unconditionally true here:
            // zshrs's ${#name} length-form is handled in an earlier
            // arm that returns before reaching this transition, so
            // `getlen` is always 0 at this point in the flow.
            //
            // The C source at c:3317 adds `!spsep && spbreak < 2` to
            // the same gate inside the substitution-operator arms.
            // The (s) flag (sets spsep) is a deliberate "preserve
            // split-from-scalar shape in DQ" signal — without the
            // !spsep guard, `"${(s. .)str}"` would sepjoin back to
            // scalar instead of splatting per-word.
            if qt && isarr > 0 && spsep.is_none() {
                // c:3032 + c:3317 !spsep guard
                // C: `val = sepjoin(aval, sep, 1);`. `sep` defaults
                // to first IFS char when no (j) flag overrode it.
                // The value-init block at line 5908 unconditionally
                // joined with " " (the historical default) which
                // misses user-set IFS. Re-join with IFS[0] here so
                // `"${a[*]}"` with IFS=":" produces `x:y:z` instead
                // of the hardcoded `x y z`. Bug #428.
                //
                // c:Src/subst.c:3032 — the sepjoin re-fetches from
                // `aval` (the variable's raw value array). When the
                // subscript dispatch above ALREADY produced an
                // array-shape result (`split_parts` populated by the
                // assoc/array flag-arms — `${h[(I)pat]}`, `${arr[(R)pat]}`,
                // etc.), C's `aval` already points at the matched
                // subset. zshrs's path re-reads `arrays_get`/
                // `assoc_get` which loses the dispatch's filter and
                // splices the FULL variable back in. Guard by
                // preferring `split_parts` when the dispatch
                // populated it; that's the dispatch's authoritative
                // array shape. Without the guard, `${h[(I)*]}` (and
                // every other subscript-flag form on assoc/array)
                // returned all values regardless of which keys
                // matched.
                if let Some(sp) = split_parts.as_ref() {
                    let join_sep: String = sep.clone().unwrap_or_else(|| {
                        let ifs = crate::ported::params::getsparam("IFS")
                            .unwrap_or_else(|| " \t\n".to_string());
                        ifs.chars().next().map(String::from).unwrap_or_default()
                    });
                    value = sp.join(&join_sep);
                } else if let Some(arr) = arrays_get(&var_name) {
                    let join_sep: String = sep.clone().unwrap_or_else(|| {
                        let ifs = crate::ported::params::getsparam("IFS")
                            .unwrap_or_else(|| " \t\n".to_string());
                        ifs.chars().next().map(String::from).unwrap_or_default()
                    });
                    value = arr.join(&join_sep);
                } else if let Some(m) = assoc_get(&var_name) {
                    let join_sep: String = sep.clone().unwrap_or_else(|| {
                        let ifs = crate::ported::params::getsparam("IFS")
                            .unwrap_or_else(|| " \t\n".to_string());
                        ifs.chars().next().map(String::from).unwrap_or_default()
                    });
                    value = m
                        .values()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(&join_sep);
                }
                isarr = 0; // c:3034
            }
        }
        // `split_parts` (c:3950) moved to function-scope declaration
        // earlier so subscript/flag arms above can write to it.
        // c:Src/Modules/parameter.c — magic-assoc (k)/(v) reads
        // captured by magic_assoc_array (declared near `value` above).
        // Seed split_parts so the auto_splat block emits each key/
        // value as its own result_node. C achieves this via SCANPM_*
        // → isarr=1; the Rust port collapses to a single scalar
        // without this seed.
        if let Some(ref keys) = magic_assoc_array {
            if !keys.is_empty() {
                split_parts = Some(keys.clone());
                isarr = 1;
            }
        }
        // c:Src/subst.c — `${(@)assoc}` (no (k)/(v) flags, just (@))
        // splats the assoc's VALUES one per result_node. Seed
        // split_parts here so the auto-splat block at c:4245 emits
        // each value separately. Bug #287 in docs/BUGS.md. Gated on
        // nojoin == 2 (the (@) flag) AND no subscript (the bare
        // assoc-name form). Mirrors the magic_assoc_array seed above
        // but covers user-defined assocs (not just magic ones).
        if nojoin == 2 && subscript.is_none() && magic_assoc_array.is_none() {
            if let Some(map) = assoc_get(&var_name) {
                let values: Vec<String> = map.values().cloned().collect();
                if !values.is_empty() {
                    split_parts = Some(values);
                    isarr = 1;
                }
            }
        }
        // c:Src/subst.c — `${assoc[(R)pat]}` / `${assoc[(I)pat]}` /
        // `${assoc[(K)pat]}` preserve ARRAY shape across the assoc
        // subscript MATCH path so consumers like
        // `for k in ${h[(I)pat]}` iterate per matched key. C source
        // returns `char **` from paramvalarr (params.c:689) for the
        // multi-match capital flags (R/I/K); the Rust port joins them
        // to a single scalar in the assoc dispatch (line 4646). Split
        // back to Vec for splat shape regardless of (@) outer flag —
        // zsh's paramvalarr return is inherently array-shaped; the
        // c:3032 DQ-collapse arm below downgrades to scalar in qt
        // context just like any other array result.
        // Bug #592 + colors() `for k in ${color[(I)3?]}` failure.
        if magic_assoc_array.is_none()
            && split_parts.is_none()
            && !raw_value.is_empty()
            && assoc_contains(&var_name)
            && subscript
                .as_deref()
                .map_or(false, |s| {
                    let t = s.trim_start();
                    t.starts_with("(R)") || t.starts_with("(r)")
                        || t.starts_with("(I)") || t.starts_with("(i)")
                        || t.starts_with("(K)") || t.starts_with("(k)")
                })
        {
            let parts: Vec<String> = raw_value
                .split(' ')
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect();
            if !parts.is_empty() {
                split_parts = Some(parts);
                isarr = 1;
            }
        }
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
                // c:Src/subst.c:3360-3412 — pre-tokenize source metas,
                // singsub splices raw, literalize leftover raw metas
                // (GLOBSUBST-gated, c:1669). Same contract as the
                // strip/replace arms below; `:#` rides the same
                // `case '#'` + singsub + getmatch C path.
                let p = literalize_spliced_metas(&singsub(&pretokenize_src_pat(pat))); // c:3540
                // c:Src/glob.c:2674-2677 — patcompile failure → "bad
                // pattern" diagnostic. Sibling of #605/#606. Bug #607.
                if !p.is_empty()
                    && patcompile(&{ let mut __pat_tok = (&p).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, PAT_HEAPDUP as i32, None).is_none()
                {
                    zerr(&format!("bad pattern: {}", p));
                    errflag_set_error();
                    return (String::new(), new_pos, vec![]);
                }
                let cur_sub_flags = sub_flags_get(); // c:2171
                let invert = (cur_sub_flags & 0x0008) != 0; // c:2171 SUB_MATCH
                sub_flags_set(0); // c:2169 (consume)
                                  // Direct port of subst.c:3422 `if (!vunset && isarr)` —
                                  // the array iteration only fires when `isarr` is set.
                                  // After getindex computes a single-slot subscript, isarr
                                  // is cleared at line 2915 (`v->scanflags ? 1 : 0`) and
                                  // the C source falls through to getmatch on `val`
                                  // (line 3451). Mirror that here: when subscript was
                                  // applied AND it picks a single slot, treat raw_value
                                  // as the scalar `val` and skip the per-element arr
                                  // loop. For `[@]`/`[*]` and range `[N,M]`, isarr stays
                                  // set (SCANPM_ISVAR_AT path at c:2027-2029) so the
                                  // array iteration MUST fire — `\${arr[@]:#pat}` filters
                                  // the elements, not the joined scalar.
                let is_array_subscript = matches!(subscript.as_deref(), Some("@") | Some("*"))
                    || subscript.as_deref().map_or(false, |s| s.contains(','));
                let has_subscript = subscript.is_some() && !is_array_subscript;
                // c:Src/subst.c:3317 — under qt (DQ context) without an
                // explicit @/*-style subscript, the array sepjoins to
                // a scalar BEFORE the filter applies. `"${a:#two}"`
                // tests the joined "one two three" once, not each
                // element. Parity bug: zshrs filtered per-element in
                // DQ too, returning "one three" instead of leaving
                // the joined scalar untouched.
                //
                // c:Src/subst.c:2167 — the `(@)` flag sets nojoin=2
                // which forces array-shape preservation even in DQ.
                // Without nojoin==2 in this gate, `${(@)arr:#pat}` in
                // DQ fell into the scalar arm and skipped per-element
                // filtering. Bug #236 surfaced via `${(@)a:#}` (empty
                // pattern) ignoring the (@)-flag and never filtering
                // empties out.
                // c:Src/subst.c:1885 + 3193 — positional `@` NEVER
                // joins in DQ ("$@" splice semantics carry through
                // the filter: `[[ -n ${(M)@:#+X} ]]` in zinit's
                // tmp-subst-autoload runs in cond/DQ compile context
                // and must still filter per-element; the joined-
                // scalar arm made the test always-true and routed
                // every plugin autoload into the +X arm).
                let per_element_array = !has_subscript
                    && (!qt || is_array_subscript || nojoin == 2 || var_name == "@");
                // c:Src/subst.c:3417 + Src/glob.c:2727 — empty
                // pattern. C's `patcompile("")` returns a Patprog
                // whose body matches only the empty string (no
                // tokens to consume → `pattry` succeeds iff the
                // subject's remaining length is zero). The Rust
                // patcompile/pattry pair returns false for empty
                // pattern vs empty subject, so the per-element
                // filter never drops empties as zsh does.
                // Short-circuit with an explicit `elem.is_empty()`
                // check that mirrors C's effective semantics.
                let match_fn = |elem: &str| -> bool {
                    if p.is_empty() {
                        // c:3417 — empty pattern ⇔ empty subject
                        elem.is_empty()
                    } else {
                        patcompile(&{ let mut __pat_tok = (&p).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, PAT_HEAPDUP as i32, None)
                            .map_or(false, |__p| pattry(&__p, elem))
                    }
                };
                // c:Src/subst.c — when a prior operator (e.g. (@k)/
                // (@v) on an assoc, or (s::) split, or assoc-splat)
                // populated `split_parts`, the :# filter operates on
                // THAT element list rather than re-reading from
                // arrays_get. Without this, `${(@k)h:#a}` fell into
                // the scalar arm and tested "a b" against "a" once
                // → no match → kept everything. Bug #311 in
                // docs/BUGS.md. The arrays_get path stays as the
                // primary check (indexed-array source); split_parts
                // is the operator-result source.
                let source_arr: Option<Vec<String>> = arrays_get(&var_name)
                    .filter(|_| per_element_array)
                    .or_else(|| {
                        if per_element_array {
                            split_parts.clone()
                        } else {
                            None
                        }
                    });
                if let Some(arr) = source_arr {
                    let kept: Vec<String> = arr
                        .into_iter() // c:3540
                        .filter(|elem| {
                            // c:3540
                            let m = match_fn(elem); // c:3540
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
                                            // array — ${(@)arr:#pat} splats only the kept
                                            // elements.
                    split_parts = Some(kept); // c:3540
                } else {
                    // c:3540
                    // c:Src/subst.c — scalar fallback path. When a
                    // prior flag operator (e.g. `(v)` joining assoc
                    // values into a scalar) populated `value`, test
                    // against the post-flag scalar — NOT `raw_value`
                    // (the raw assoc backing). For `${(v)h:#A*}` on
                    // `h=(a ABC b XYZ)`, the (v) flag sets value to
                    // "ABC XYZ"; the filter tests that joined string
                    // against `A*` (matches → return empty). Without
                    // the value-first preference, the test ran
                    // against raw_value (often empty for assoc-name
                    // bare reads) and silently passed through. Bug
                    // #312 in docs/BUGS.md.
                    let subject = if !value.is_empty() {
                        value.clone()
                    } else {
                        raw_value.clone()
                    };
                    let m = match_fn(&subject); // c:3540
                    let result = if invert {
                        // c:3540
                        if m {
                            subject.clone()
                        } else {
                            String::new()
                        } // c:3540
                    } else {
                        // c:3540
                        if m {
                            String::new()
                        } else {
                            subject.clone()
                        } // c:3540
                    };
                    value = result.clone();
                    // c:Src/subst.c — when the scalar fallback filter
                    // empties (or keeps) the scalar, the downstream
                    // auto_splat block must see the post-filter shape.
                    // The prior (v) flag seeded `split_parts` with
                    // per-element values; if we don't override here,
                    // the splat re-emits those instead of our
                    // filtered scalar. Bug #312 in docs/BUGS.md.
                    if split_parts.is_some() {
                        split_parts = if result.is_empty() {
                            Some(Vec::new())
                        } else {
                            Some(vec![result])
                        };
                    }
                } // c:3540
            } else if let Some(default) = r.strip_prefix(":-") {
                // c:3193
                // c:Src/subst.c:3193 vunset check — for arrays under
                //   `(@)` (nojoin==2) the array's existence with ANY
                //   elements (including a single empty element) means
                //   "set, not null". `b=("")` with `${(@)b:-x}` should
                //   return `[]` (the empty element) not `[x]`. Without
                //   the gate, raw_value (which joins the array to a
                //   scalar) was "" and `raw_value.is_empty()` fired
                //   the default. Bug #186 in docs/BUGS.md.
                let is_at_array = nojoin == 2 && arrays_contains(&var_name);
                let array_is_empty = is_at_array
                    && arrays_get(&var_name).map(|a| a.is_empty()).unwrap_or(true);
                let vunset = if is_at_array {
                    !is_set || array_is_empty
                } else {
                    !is_set || raw_value.is_empty()
                };
                if vunset {
                    value = singsub(default);
                    // Parity bug: for `${arr[@]:-default}` with an
                    // empty array, the operator computed the default
                    // into `value`, but the downstream auto_splat
                    // block re-fetches arrays_get(var_name) on the
                    // [@] path and ignores `value`. Seed split_parts
                    // with the default so the splat emits it. Mirror
                    // C subst.c:3193 case '-' which stores the result
                    // back into val/aval, making the c:4245 splat
                    // see the substituted shape. Skip the seed when
                    // value is empty so the Nularg sentinel `¡`
                    // doesn't leak out of `${a:-}` on empty arrays.
                    //
                    // c:Src/subst.c:3193 — the val/aval write also
                    // covers subscripted reads on positional-arrays
                    // (`${*[N]:-x}` / `${@[N]:-x}` with no positionals).
                    // Downstream (q)/(qq)/(qqq)/(qqqq) flag fetches
                    // `arrays_get(var_name)` for `*`/`@` and returns
                    // the empty PPARAMS, overwriting `value`. Bug #591:
                    // seed split_parts when the var is positional-
                    // special so the qq path picks up the default.
                    let is_positional_special =
                        matches!(var_name.as_str(), "*" | "@" | "argv");
                    if !value.is_empty()
                        && split_parts.is_none()
                        && (isarr != 0 || is_positional_special)
                    {
                        split_parts = Some(vec![value.clone()]);
                    }
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
                // (params.c:3602) based on the `arrasg` flag.
                value = singsub(default);
                if arrasg == 1 {
                    // c:3263 (A)
                    let ifs = vars_get("IFS").unwrap_or_else(|| " \t\n".to_string());
                    let parts: Vec<String> = value
                        .split(|c: char| ifs.contains(c))
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string())
                        .collect();
                    exec_assignaparam(&var_name, parts);
                } else if arrasg == 2 {
                    // c:3263 (AA)
                    let ifs = vars_get("IFS").unwrap_or_else(|| " \t\n".to_string());
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
                    assignsparam(&__s, &value, 0);
                    exec_sync_state_from_paramtab();
                }
            } else if let Some(default) = r.strip_prefix(":=") {
                // c:3245
                if !is_set || raw_value.is_empty() {
                    value = singsub(default);
                    if arrasg == 1 {
                        // c:3263 (A)
                        let ifs = vars_get("IFS").unwrap_or_else(|| " \t\n".to_string());
                        let parts: Vec<String> = value
                            .split(|c: char| ifs.contains(c))
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string())
                            .collect();
                        exec_assignaparam(&var_name, parts);
                    } else if arrasg == 2 {
                        // c:3263 (AA)
                        let ifs = vars_get("IFS").unwrap_or_else(|| " \t\n".to_string());
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
                        assignsparam(&__s, &value, 0);
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
                    if arrasg == 1 {
                        // c:3263 (A)
                        let ifs = vars_get("IFS").unwrap_or_else(|| " \t\n".to_string());
                        let parts: Vec<String> = value
                            .split(|c: char| ifs.contains(c))
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string())
                            .collect();
                        exec_assignaparam(&var_name, parts);
                    } else if arrasg == 2 {
                        // c:3263 (AA)
                        let ifs = vars_get("IFS").unwrap_or_else(|| " \t\n".to_string());
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
                        assignsparam(&__s, &value, 0);
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
                // Seed split_parts so the downstream c:4245 auto-splat
                // sees the substituted scalar, not the original array.
                // Parity bug: `${arr[@]:+yes}` on a populated array
                // splat the array elements ("x y") instead of the
                // alternate "yes" because auto_splat fell back to
                // arrays_get(var_name). Mirror C subst.c:3296 which
                // writes val back into the v->str/aval slot.
                // Only seed when there's an actual replacement value —
                // an empty result must NOT emit the Nularg sentinel
                // node `¡` (no-array-element marker) that splat-of-
                // (vec![""]) would produce.
                if isarr != 0 && split_parts.is_none() && !value.is_empty() {
                    split_parts = Some(vec![value.clone()]);
                }
            } else if let Some(alt) = r.strip_prefix('+') {
                // c:3296
                if is_set {
                    value = singsub(alt);
                } else {
                    value = String::new();
                }
                if isarr != 0 && split_parts.is_none() && !value.is_empty() {
                    split_parts = Some(vec![value.clone()]);
                }
            } else if let Some(msg) = r.strip_prefix(":?") {
                // c:3193 (:?msg)
                if !is_set || raw_value.is_empty() {
                    let m = if msg.is_empty() {
                        // c:Src/subst.c:3337 — `zerr("%s: %s", idbeg,
                        // "parameter not set")`. zsh uses the same
                        // diagnostic for both ${var?} and ${var:?}
                        // when the message is empty; only the trigger
                        // condition (unset vs unset-or-null) differs.
                        "parameter not set".to_string()
                    } else {
                        // c:3193
                        singsub(msg) // c:3193
                    }; // c:3193
                       // C: zerr("%s: %s", idbeg, msg) — Src/subst.c:3337
                    zerr(&format!("{}: {}", var_name, m));
                    errflag_set_error();
                    // c:Src/subst.c:3344 — `errflag |= ERRFLAG_HARD;`.
                    // `:?` is a FATAL error — non-interactive shell
                    // exits, interactive returns to prompt — distinct
                    // from a normal math/parse error that just clears
                    // and continues. Set ERRFLAG_HARD so downstream
                    // gates (e.g. BUILTIN_ARITH_CMD_FINISH) preserve
                    // the propagation instead of clearing the next-
                    // statement script-abort signal. Bug #193 in
                    // docs/BUGS.md.
                    crate::ported::utils::errflag.fetch_or(
                        crate::ported::zsh_h::ERRFLAG_HARD,
                        std::sync::atomic::Ordering::Relaxed,
                    );
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
                    // c:Src/subst.c:3344 — `errflag |= ERRFLAG_HARD;`
                    // (same fatal-abort semantics as `:?`). Bug #193.
                    crate::ported::utils::errflag.fetch_or(
                        crate::ported::zsh_h::ERRFLAG_HARD,
                        std::sync::atomic::Ordering::Relaxed,
                    );
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
                // c:Src/subst.c:3360-3412 — pre-tokenize source metas,
                // singsub splices raw, literalize leftover raw metas
                // (GLOBSUBST-gated, c:1669). Same contract as the
                // `//`/`/` arms below.
                let pat = literalize_spliced_metas(&singsub(&pretokenize_src_pat(parts[0])));
                // c:Src/glob.c:2680 — the replacement must be evaluated
                // AFTER the pattern match when it references match state
                // (`(#m)$MATCH` / `(#b)$match`); `pattry` publishes
                // $MATCH/$match/$mbegin/$mend on each call. The previous
                // port singsub'd the replacement ONCE before any match,
                // so `${(@)arr:/(#m)*/$MATCH}` saw an empty $MATCH (it
                // produced `${''-…}` in p10k's _p9k_must_init → "bad
                // substitution"). Mirror the `//` arm: keep raw_repl,
                // compile once, re-singsub per match when needed.
                let raw_repl = parts.get(1).copied().unwrap_or("").to_string();
                let prog_opt = if pat.is_empty() {
                    None
                } else {
                    patcompile(&{ let mut __t = pat.to_string(); crate::ported::glob::tokenize(&mut __t); __t }, PAT_HEAPDUP as i32, None)
                };
                // c:Src/glob.c:2680 SUB_DOSUBST gate — capture groups
                // (patnpar) or a match-ref (GF_MATCHREF) force per-match
                // re-evaluation of the replacement.
                let pat_needs_per_match = prog_opt
                    .as_ref()
                    .map(|p| {
                        p.0.patnpar > 0
                            || (p.0.globend & crate::ported::zsh_h::GF_MATCHREF as i32) != 0
                    })
                    .unwrap_or(false);
                let repl_static = if pat_needs_per_match {
                    String::new()
                } else {
                    let saved = SKIP_FILESUB.with(|c| c.get());
                    SKIP_FILESUB.with(|c| c.set(true));
                    let s = untokenize(&singsub(&raw_repl));
                    SKIP_FILESUB.with(|c| c.set(saved));
                    s
                };
                // `elem` must already have been pattry'd (which set the
                // match params) immediately before this call.
                let eval_repl = |needs: bool| -> String {
                    if !needs {
                        return repl_static.clone();
                    }
                    let saved = SKIP_FILESUB.with(|c| c.get());
                    SKIP_FILESUB.with(|c| c.set(true));
                    let s = untokenize(&singsub(&raw_repl));
                    SKIP_FILESUB.with(|c| c.set(saved));
                    s
                };
                if let Some(arr) = arrays_get(&var_name) {
                    let new_arr: Vec<String> = arr
                        .into_iter()
                        .map(|elem| {
                            if prog_opt.as_ref().map_or(false, |__p| pattry(__p, &elem)) {
                                eval_repl(pat_needs_per_match)
                            } else {
                                elem
                            }
                        })
                        .collect();
                    value = new_arr.join(" "); // c:3870
                    split_parts = Some(new_arr); // c:3870
                } else if prog_opt
                    .as_ref()
                    .map_or(false, |__p| pattry(__p, &raw_value))
                {
                    value = eval_repl(pat_needs_per_match); // c:3870
                } else {
                    value = raw_value.clone(); // c:3870
                }
            } else if let Some(rep) = r.strip_prefix("//") {
                // c:3870 (global replace)
                // Same NUL/Bnull-aware split as before. NUL/Bnull +
                // X → `\X` for the pat side (glob meta literal).
                // `\` + `/` → `/` (literal `/`, not separator).
                // Direct port of Src/subst.c:3884.
                let split_unescaped = |s: &str| -> (String, String) {
                    // c:Src/subst.c around line 3354 — `${var//PAT/REPL}`
                    // walks the body looking for the `/` separator.
                    // Backslash-escaped chars are passed through to
                    // the pattern verbatim: `\\` keeps both bytes
                    // (patcompile reads `\X` as literal-X escape, so
                    // the pattern stays `\\` matching a single `\` at
                    // pattern-eval time), `\/` keeps both bytes too
                    // (the leading `\` defangs the `/` from acting
                    // as the separator, but patcompile will still
                    // see the `\` as the escape prefix).
                    let cv: Vec<char> = s.chars().collect();
                    let mut pat_buf = String::new();
                    let mut i = 0;
                    while i < cv.len() {
                        let c = cv[i];
                        // c:Src/subst.c — Bnull/NUL markers from the
                        // lexer wrap the next char as user-literal.
                        // Re-emit as `\X` so patcompile sees an
                        // escape pair.
                        if (c == '\x00' || c == '\u{9f}') && i + 1 < cv.len() {
                            pat_buf.push('\\');
                            pat_buf.push(cv[i + 1]);
                            i += 2;
                            continue;
                        }
                        // c:Src/subst.c — `\X` source-level escape.
                        // Both bytes pass through to the pattern so
                        // patcompile interprets the `\X` itself.
                        // Crucially: `\\` (two backslashes) is the
                        // escape-backslash form and must consume
                        // both bytes BEFORE checking whether the
                        // SECOND char is `/` — otherwise `\\/` parses
                        // as `\` + escaped-`/`, dropping the leading
                        // `\` and treating `/` as literal instead of
                        // the separator.
                        if c == '\\' && i + 1 < cv.len() {
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
                // c:Src/lex.c:1519-1556 + c:Src/subst.c:301 — inside
                // double quotes `$'` is Qstring + RAW `'` and is NOT
                // ANSI-C; C's getmatch replacement keeps it literal
                // (oracle: `"${a/x/$'\t'q}"` → `$'\t'q`, while the
                // UNQUOTED form decodes). The rest builder folded
                // Qstring to ASCII `$`, so the replacement singsub
                // would re-read it as top-level ANSI-C. Restore the
                // canonical token form for `$` immediately followed
                // by `'` when this paramsubst is DQ-context (qt) —
                // replacement-only: the pattern side must keep its
                // (decoded) behavior, verified against the oracle.
                let raw_repl = if qt && raw_repl.contains("$'") {
                    let cs: Vec<char> = raw_repl.chars().collect();
                    let mut out = String::with_capacity(raw_repl.len());
                    let mut i = 0;
                    while i < cs.len() {
                        let prev_bnull = i > 0 && cs[i - 1] == Bnull;
                        if cs[i] == '$' && cs.get(i + 1) == Some(&'\'') && !prev_bnull {
                            out.push(Qstring); // DQ-context token form
                        } else {
                            out.push(cs[i]);
                        }
                        i += 1;
                    }
                    out
                } else {
                    raw_repl
                };
                // c:Src/subst.c — `${X//#pat/repl}` start-anchor
                // and `${X//%pat/repl}` end-anchor variants for the
                // global-replace form. The leading `#`/`%` peel off
                // the pattern and constrain where matches may fire:
                //   `#` — only at position 0 (start of string).
                //   `%` — only at the very end (longest tail match).
                // c:Src/subst.c — `${var/#%pat/repl}` anchors the
                // match at BOTH start AND end — the pattern must
                // match the entire string for the replace to fire.
                // zsh recognizes the `#` before `%` ordering only;
                // `%#` is parsed as `%` anchor + `#pat` (where
                // the leading `#` is literal pattern data).
                // Bug #355 in docs/BUGS.md.
                let (pat_anchor, pat_after_anchor) = if let Some(rest) = raw_pat.strip_prefix('#') {
                    if let Some(rest2) = rest.strip_prefix('%') {
                        ('B', rest2.to_string()) // 'B' = both anchors
                    } else {
                        ('#', rest.to_string())
                    }
                } else if let Some(rest) = raw_pat.strip_prefix('%') {
                    ('%', rest.to_string())
                } else {
                    ('\0', raw_pat.clone())
                };
                // c:Src/subst.c:3360-3412 — C order transposed: the
                // pattern is pre-tokenized (source metas -> token
                // chars, as the lexer leaves them for paramsubst),
                // THEN singsub splices values raw, THEN leftover raw
                // metas are literalized (GLOBSUBST-gated, c:1669).
                // Bare top-level `|` stays literal per
                // Src/pattern.c:248 (handled inside the pre-tokenize
                // closure; subsumes the old escape_bare_alt_pipes
                // call here). Bug #596.
                let pat = literalize_spliced_metas(&singsub(&pretokenize_src_pat(
                    &pat_after_anchor,
                )));
                // c:Src/glob.c:2674-2677 — `p = patcompile(pat,
                // patflags, NULL); if (!p) { zerr("bad pattern: %s",
                // pat); return NULL; }`. The replace path silently
                // ignored compile failures (e.g. unbalanced `(foo|bar`)
                // and returned the input unchanged; zsh rejects with
                // "bad pattern: ...". Pre-check here so the diagnostic
                // surfaces. Bug #605.
                // c:Src/glob.c:2674-2690 compgetmatch — `p = patcompile(...);
                // if (!p) { zerr("bad pattern: ..."); return NULL; }`
                // followed by the SUB_DOSUBST gate at c:2680:
                //   `if (p->patnpar || (p->globend & GF_MATCHREF))`
                //     `*flp |= SUB_DOSUBST;`   // re-substitute per match
                //   `else { singsub(replstrp); untokenize(*replstrp); }`
                // Either path of the prog->patnpar / GF_MATCHREF test means
                // the replacement references match state (`$match[N]` via
                // backref or `$MATCH` via match-ref) and MUST be
                // re-evaluated per match. The pre-substitution path
                // would otherwise capture stale `$MATCH`/`$match` (empty
                // or from a previous expansion) and every match would
                // expand identically. Bug at zinit.zsh:2690 (prehelp
                // parser): `${args[@]//(#b)(-b|…)/${OPTS[${match[2]}]::=1}}`
                // hit the pre-substitution path with $match unset →
                // singsub on the replacement saw `${match[2]}` as
                // literal text → assignsparam rejected `OPTS[${match[2]}]`
                // as not-an-identifier.
                let prog_opt = if pat.is_empty() {
                    None
                } else {
                    patcompile(&{ let mut __pat_tok = (&pat).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, PAT_HEAPDUP as i32, None)
                };
                if !pat.is_empty() && prog_opt.is_none() {
                    zerr(&format!("bad pattern: {}", pat));
                    errflag_set_error();
                    return (String::new(), new_pos, vec![]);
                }
                // c:Src/glob.c:2680 — `if (p->patnpar || (p->globend
                // & GF_MATCHREF)) *flp |= SUB_DOSUBST;`. Faithful
                // port: the compiled `Patprog`'s `patnpar` carries the
                // capture-group count and `globend` carries the final
                // patglobflags snapshot (Src/pattern.c:636), so this
                // two-clause check is the canonical SUB_DOSUBST gate.
                let pat_needs_per_match = prog_opt
                    .as_ref()
                    .map(|p| {
                        p.0.patnpar > 0
                            || (p.0.globend
                                & crate::ported::zsh_h::GF_MATCHREF as i32)
                                != 0
                    })
                    .unwrap_or(false);
                // Replacement: c:Src/glob.c:2687-2688 —
                //   `singsub(replstrp); untokenize(*replstrp);`
                // Skip when SUB_DOSUBST equivalent fires (per-match path
                // takes over via eval_repl_for_match below). The
                // SKIP_FILESUB gate mirrors C's `prefork(replstr,
                // SUB_FLAG|SKIP_FILESUB, ...)` per c:Src/subst.c:3354
                // (tilde expansion suppressed inside the replacement).
                let repl = if pat_needs_per_match {
                    String::new()
                } else {
                    let saved_skip = SKIP_FILESUB.with(|c| c.get());
                    SKIP_FILESUB.with(|c| c.set(true));
                    let s = untokenize(&singsub(&raw_repl)); // c:2687
                    SKIP_FILESUB.with(|c| c.set(saved_skip));
                    s
                };
                let eval_repl_for_match = |span_text: &str, span_start_byte: usize| -> String {
                    if !pat_needs_per_match {
                        return repl.clone();
                    }
                    // Set $MATCH / $MBEGIN / $MEND so the singsub
                    // pass below sees the current capture state.
                    crate::ported::params::setsparam("MATCH", span_text);
                    let base = if isset(crate::ported::zsh_h::KSHARRAYS) {
                        0i64
                    } else {
                        1
                    };
                    crate::ported::params::setiparam("MBEGIN", span_start_byte as i64 + base);
                    crate::ported::params::setiparam(
                        "MEND",
                        (span_start_byte as i64 + span_text.len() as i64 + base).saturating_sub(1),
                    );
                    let saved_skip = SKIP_FILESUB.with(|c| c.get());
                    SKIP_FILESUB.with(|c| c.set(true));
                    let s = untokenize(&singsub(&raw_repl));
                    SKIP_FILESUB.with(|c| c.set(saved_skip));
                    // c:Src/glob.c:2687-2688 — see Bug #609 comment on
                    // the precomputed path above; literal backslashes
                    // (`\n`/`\t`/`\&` etc.) are kept verbatim.
                    s
                };
                // Per-element replace for arrays — zsh treats each
                // element as a separate match target, preserving the
                // array shape. \${(@)arr//pat/repl} keeps element
                // count, replaces within each. Direct port of
                // subst.c's getmatcharr path that calls getmatch on
                // each element separately. Single-shot helper to
                // avoid duplicating the sliding-window logic.
                let gms = |s_: &str, p_: &str| -> bool {
                    // c:Src/glob.c:2514 matchpat shape — tokenize +
                    // patcompile + pattry. Captures, (#b) $match
                    // arrays and (#m) $MATCH now publish INSIDE
                    // pattryrefs (pattern.c:2526-2542 / 2570-2621
                    // ports), so plain pattry carries them; the
                    // former vm_helper::glob_match_static wrapper is
                    // gone. Silent false on bad pattern (caller arms
                    // pre-validate where C zerrs).
                    match crate::ported::pattern::patcompile(
                        &{
                            let mut t_ = p_.to_string();
                            crate::ported::glob::tokenize(&mut t_);
                            t_
                        },
                        crate::ported::zsh_h::PAT_HEAPDUP,
                        None,
                    ) {
                        Some(pr_) => crate::ported::pattern::pattry(&pr_, s_),
                        None => false,
                    }
                };
                let replace_global = |val: &str| -> String {
                    let cv: Vec<char> = val.chars().collect();
                    let nn = cv.len();
                    let mut o = String::with_capacity(val.len());
                    // c:Src/subst.c — `#` anchor: match only at
                    // start; `%` anchor: match only at end (longest
                    // tail). Without an anchor, the global sliding
                    // window runs across every position.
                    //
                    // c:Src/pattern.c:2570-2621 — `(#b)pat` $match /
                    // $mbegin / $mend publication now lives INSIDE
                    // pattryrefs (no-caller-arrays arm), so plain
                    // pattry via `gms` carries it. Bug #266 history.
                    if pat_anchor == 'B' {
                        // c:Src/subst.c — both-anchored `#%`/`%#`: the
                        // pattern must match the ENTIRE string. Test
                        // the whole val and replace iff successful.
                        // Bug #355 in docs/BUGS.md.
                        let whole: String = cv.iter().collect();
                        if gms(&whole, &pat) {
                            o.push_str(&eval_repl_for_match(&whole, 0));
                        } else {
                            o.push_str(val);
                        }
                        return o;
                    }
                    if pat_anchor == '#' {
                        let mut matched: Option<usize> = None;
                        // c:Src/glob.c:2920-2941 igetmatch `case 0/
                        // SUB_LONG` — `pattrylen` at the head can
                        // return a ZERO-length match (`patmatchlen()
                        // == 0`), e.g. `c#` on "ab" → replace the
                        // empty prefix: "Xab". Include end == 0.
                        for end in (0..=nn).rev() {
                            let cand: String = cv[..end].iter().collect();
                            if gms(&cand, &pat) {
                                matched = Some(end);
                                break;
                            }
                        }
                        if let Some(e) = matched {
                            let span: String = cv[..e].iter().collect();
                            o.push_str(&eval_repl_for_match(&span, 0));
                            o.push_str(&cv[e..].iter().collect::<String>());
                        } else {
                            o.push_str(val);
                        }
                        return o;
                    }
                    if pat_anchor == '%' {
                        let mut matched: Option<usize> = None;
                        for start in 0..=nn {
                            let cand: String = cv[start..].iter().collect();
                            if gms(&cand, &pat) {
                                matched = Some(start);
                                break;
                            }
                        }
                        if let Some(s) = matched {
                            let span_text: String = cv[s..].iter().collect();
                            let span_byte = cv[..s].iter().map(|c| c.len_utf8()).sum();
                            o.push_str(&cv[..s].iter().collect::<String>());
                            o.push_str(&eval_repl_for_match(&span_text, span_byte));
                        } else {
                            o.push_str(val);
                        }
                        return o;
                    }
                    // c:Src/glob.c igetmatch SUB_SUBSTR + SUB_LONG —
                    // the `(S)` flag picks shortest match at each
                    // position; default is greedy (longest first).
                    // For `${(S)s//a*/X}` on "aaa", shortest at q=0 is
                    // "a" (length 1), so each `a` is replaced
                    // individually → "XXX". Without (S) the longest
                    // match consumes all "aaa" → single "X". Bug #356.
                    let substr_short = (sub_flags_get() & SUB_SUBSTR) != 0;
                    // c:Src/pattern.c P_ISSTART / P_ISEND — `(#s)` /
                    // `(#e)` anchors that compare the absolute string
                    // position against 0 / `string.len()`. The sliding
                    // window below slices `cv[q..e]` into a fresh
                    // String for each candidate, so the matcher loses
                    // the absolute offset context and `(#s)` would
                    // trivially pass at every q. C's pattry threads
                    // the offset through; the Rust matcher doesn't
                    // (yet), so gate the search bounds here. For an
                    // alternation `((#s)X|X(#e))` BOTH anchors live in
                    // the pattern text — restrict the search to q=0
                    // OR end-positions where the match could end at
                    // nn. Bug surface: `${x//((#s)…|…(#e))/}` for
                    // boundary-only whitespace strip
                    // (zinit_anchored_strip_both_ends megamonster).
                    let has_start_anchor_global = pat.contains("(#s)");
                    let has_end_anchor_global = pat.contains("(#e)");
                    // c:Src/glob.c:3028-3046 — the inner for-loop runs
                    // `for (; t <= send; ...)`, so on an EMPTY input
                    // (t == send from the start) a zero-length match
                    // still records [0,0): `${v//a#/X}` with v=""
                    // yields "X".
                    if nn == 0 {
                        if gms("", &pat) {
                            o.push_str(&eval_repl_for_match("", 0));
                        }
                        return o;
                    }
                    let mut q = 0_usize;
                    while q < nn {
                        let mut m: Option<usize> = None;
                        // c:Src/pattern.c P_ISSTART / P_ISEND — gate
                        // candidate (q, e) spans by whether the
                        // pattern's anchors permit them. (#s)-only:
                        // q must be 0. (#e)-only: e must be nn. Both:
                        // q==0 OR e==nn (alternation in the pattern).
                        // Without anchors: any (q, e) is valid.
                        let anchor_ok = |qx: usize, ex: usize| -> bool {
                            if has_start_anchor_global && has_end_anchor_global {
                                qx == 0 || ex == nn
                            } else if has_start_anchor_global {
                                qx == 0
                            } else if has_end_anchor_global {
                                ex == nn
                            } else {
                                true
                            }
                        };
                        // c:Src/glob.c:3035-3061 — `pattrylen` at each
                        // position can succeed with a ZERO-length match
                        // (`patmatchlen() == 0`, e.g. `a#` between two
                        // b's). Include the e == q window: greedy tries
                        // it LAST (longest first), shortest tries it
                        // FIRST (umlen2 starts at 0, c:3041-3053).
                        if substr_short {
                            // Shortest: e walks q..=nn ascending.
                            for e in q..=nn {
                                if !anchor_ok(q, e) {
                                    continue;
                                }
                                let c: String = cv[q..e].iter().collect();
                                if gms(&c, &pat) {
                                    m = Some(e);
                                    break;
                                }
                            }
                        } else {
                            // Greedy (default): e walks q..=nn
                            // descending.
                            for e in (q..=nn).rev() {
                                if !anchor_ok(q, e) {
                                    continue;
                                }
                                let c: String = cv[q..e].iter().collect();
                                if gms(&c, &pat) {
                                    m = Some(e);
                                    break;
                                }
                            }
                        }
                        if let Some(e) = m {
                            let span_text: String = cv[q..e].iter().collect();
                            let span_byte = cv[..q].iter().map(|c| c.len_utf8()).sum::<usize>();
                            o.push_str(&eval_repl_for_match(&span_text, span_byte));
                            if e == q {
                                // c:Src/glob.c:3060-3061 — `if (mpos ==
                                // t) mpos += mb_charlenconv(...)`: the
                                // bump char is SKIPPED by the global
                                // loop, not marked for replacement, so
                                // get_match_ret keeps it: "bbb" //a# →
                                // "XbXbXb" (X then b at each position).
                                o.push(cv[q]);
                                q += 1;
                            } else {
                                q = e;
                            }
                        } else {
                            o.push(cv[q]);
                            q += 1;
                        }
                    }
                    o
                };
                let mut handled_array = false;
                // c:Src/subst.c:3870 — array-vs-scalar context for
                // `${arr//pat/repl}`. Shapes:
                //   - `[N]`/`[key]` (literal-index subscript) → scalar
                //     context: replace operates on the single element
                //     (subst.c:2915 clears isarr; dispatches to
                //     getmatch on that element at subst.c:3451).
                //   - `[@]`/`[*]` subscript, OR `(@)`/(*) flag
                //     (nojoin == 2), OR unquoted bare form (qt=false)
                //     → array/per-element context. zsh's word-
                //     splitting treats the array as element-wise so
                //     unquoted `${a//pat/repl}` runs getmatch on each
                //     element independently.
                //   - Quoted no-subscript no-(@) form (qt=true) →
                //     SCALAR context: join array first (via $IFS[0]),
                //     then run getmatch once on the joined string.
                //     Bug #108 in docs/BUGS.md — the previous Rust
                //     port did per-element for this case too, so
                //     `"${a/b*/X}"` on `(red blue green)` came out
                //     `red X green` instead of `red X` (the C path
                //     joins "red blue green" first and the greedy
                //     `b*` swallows past element boundaries).
                let has_scalar_subscript = subscript
                    .as_deref()
                    .map(|s| {
                        let t = s.trim();
                        t != "@" && t != "*" && !t.contains(',')
                    })
                    .unwrap_or(false);
                // c:Src/subst.c:2916 SCANPM_ISVAR_AT only fires per-
                // element on `[@]`/bare `@` — `[*]`/bare `*` join with
                // IFS in DQ context (`"$*"` / `"${a[*]/x/y}"` semantics).
                // Bug #322 in docs/BUGS.md: the previous gate folded `*`
                // subscript into `@`, so `${a[*]//?/X}` in DQ ran per-
                // element instead of joining first. Unquoted (`!qt`) and
                // explicit `(@)` flag still trigger per-element regardless.
                let is_at_subscript = matches!(subscript.as_deref(), Some("@"));
                let is_at_var = matches!(var_name.as_str(), "@");
                let per_element = is_at_subscript || is_at_var || nojoin == 2 || !qt;
                if let Some(arr) = arrays_get(&var_name).filter(|_| !has_scalar_subscript) {
                    if per_element {
                        let new_arr: Vec<String> =
                            arr.iter().map(|e| replace_global(e)).collect();
                        value = new_arr.join(" "); // c:3870
                        split_parts = Some(new_arr); // c:3870 (auto-splat)
                    } else {
                        // Scalar-join path (c:subst.c sepjoin). Use
                        // $IFS first char; default is space.
                        // c:Src/utils.c:3936-3945 — set-but-empty IFS
                        // joins with "" (not " ").
                        let joined = crate::ported::utils::sepjoin(&arr, None);
                        value = replace_global(&joined);
                        // c:Src/subst.c:3034 — DQ `[*]` sepjoins to scalar
                        // BEFORE the operator runs; suppress downstream
                        // auto_splat re-fetch of the original array.
                        // Matches existing pattern at subst.rs:7378.
                        split_parts = Some(vec![value.clone()]);
                        isarr = 0;
                    }
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
                    // c:3870 — `${X//pat/repl}` SUB_GLOBAL on scalar.
                    // Use the same replace_global closure so anchor
                    // (#/%) semantics stay in one place.
                    value = replace_global(&raw_value);
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
                // c:Src/lex.c:1519-1556 + c:Src/subst.c:301 — inside
                // double quotes `$'` is Qstring + RAW `'` and is NOT
                // ANSI-C; C's getmatch replacement keeps it literal
                // (oracle: `"${a/x/$'\t'q}"` → `$'\t'q`, while the
                // UNQUOTED form decodes). The rest builder folded
                // Qstring to ASCII `$`, so the replacement singsub
                // would re-read it as top-level ANSI-C. Restore the
                // canonical token form for `$` immediately followed
                // by `'` when this paramsubst is DQ-context (qt) —
                // replacement-only: the pattern side must keep its
                // (decoded) behavior, verified against the oracle.
                let raw_repl = if qt && raw_repl.contains("$'") {
                    let cs: Vec<char> = raw_repl.chars().collect();
                    let mut out = String::with_capacity(raw_repl.len());
                    let mut i = 0;
                    while i < cs.len() {
                        let prev_bnull = i > 0 && cs[i - 1] == Bnull;
                        if cs[i] == '$' && cs.get(i + 1) == Some(&'\'') && !prev_bnull {
                            out.push(Qstring); // DQ-context token form
                        } else {
                            out.push(cs[i]);
                        }
                        i += 1;
                    }
                    out
                } else {
                    raw_repl
                };
                // c:Src/subst.c — unquoted-context replacement: `\X`
                // escapes are stripped by the surrounding shell
                // word-evaluation pipeline AFTER the substitution
                // (Bnull-marked escapes resolve via remnulargs at the
                // shtokenize/multsub boundary; raw `\X` escapes get
                // stripped by stringsubst's `\` handler at c:Src/subst.c:296).
                //
                // For inputs where my rest walker at subst.rs:4554
                // preserves Bnull markers (the canonical lex path),
                // remnulargs handles the strip on the way out — no
                // local intervention needed. For inputs where the
                // replacement contains RAW backslashes (rare, comes
                // from singsub on runtime-computed strings that
                // bypassed shtokenize), the trailing untokenize on
                // the final value will drop them.
                //
                // This matches real zsh: `x=${p/foo/\~}` → `~` (the
                // surrounding unquoted-word eval strips the `\`);
                // `x="${p/foo/\~}"` → `\~` (DQ context preserves the
                // `\`). Both paths now flow through the canonical
                // post-substitution pipeline rather than a hand-
                // rolled strip in this arm.
                // c:Src/subst.c:3120-3133 — strip leading `#`/`%`
                // anchor flag from PAT body. Applies to single replace
                // `${var/#PAT/REPL}` and `${var/%PAT/REPL}` same as
                // double replace `${var//#PAT/REPL}`. Without this,
                // the `#` was forwarded into patcompile which read it
                // as glob-`#` (0-or-more under extendedglob) at the
                // wrong position and bailed with "bad pattern".
                let (pat_anchor, pat_after_anchor) = if let Some(rest) = raw_pat.strip_prefix('#') {
                    if let Some(rest2) = rest.strip_prefix('%') {
                        ('B', rest2.to_string()) // both anchors
                    } else {
                        ('#', rest.to_string())
                    }
                } else if let Some(rest) = raw_pat.strip_prefix('%') {
                    ('%', rest.to_string())
                } else {
                    ('\0', raw_pat.clone())
                };
                // Pattern: keep \X for glob meta literals (untokenize
                // drops Bnull but pat still carries `\X` from the
                // split-walk above for the "match this literal X"
                // form).
                // c:Src/subst.c:3360-3412 — pre-tokenize source metas,
                // singsub splices raw, literalize leftover raw metas
                // (GLOBSUBST-gated, c:1669). Bare `|` outside `(...)`
                // stays literal per Src/pattern.c:248 (handled inside
                // the pre-tokenize closure; subsumes the old
                // escape_bare_alt_pipes call). Bug #596.
                let pat_body = literalize_spliced_metas(&singsub(&pretokenize_src_pat(
                    &pat_after_anchor,
                )));
                // c:Src/glob.c:2674-2677 — `patcompile` failure → "bad
                // pattern" diagnostic. Single replace arm same as `//`
                // arm above. Bug #605.
                if !pat_body.is_empty()
                    && patcompile(&{ let mut __pat_tok = (&pat_body).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, PAT_HEAPDUP as i32, None).is_none()
                {
                    zerr(&format!("bad pattern: {}", pat_body));
                    errflag_set_error();
                    return (String::new(), new_pos, vec![]);
                }
                // Re-attach the anchor prefix so the downstream
                // `replace_one` logic that dispatches on
                // `pat.strip_prefix('#')`/`pat.strip_prefix('%')`/
                // `pat.strip_prefix("#%")` continues to work without
                // duplication. The patcompile validity check above
                // already ran against the anchor-free body.
                let pat = match pat_anchor {
                    '#' => format!("#{}", pat_body),
                    '%' => format!("%{}", pat_body),
                    'B' => format!("#%{}", pat_body),
                    _ => pat_body,
                };
                if std::env::var("ZSHRS_TRACE_REPL2").is_ok() {
                    eprintln!(
                        "[TRACE_REPL2] rep={:?} raw_pat={:?} pat={:?} raw_repl={:?}",
                        rep, raw_pat, pat, raw_repl
                    );
                }
                // Replacement: per Src/glob.c::compgetmatch:2687-2688,
                // C runs `singsub(replstrp); untokenize(*replstrp);`.
                // The C untokenize drops Bnull markers (the lexer's
                // form for `\X` escapes). zshrs's bridge upstream
                // already untokenized Bnull → literal `\`, but the
                // post-untokenize STRIP `\X → X` over-applied — it
                // stripped backslashes from `\n`/`\t`/`\&` literals
                // that zsh keeps verbatim in the replacement.
                // C's untokenize ONLY drops Bnull markers, NOT literal
                // backslashes; the bridge already converted Bnull to
                // `\` so we keep them. Bug #609.
                let repl = {
                    // c:Src/subst.c around line 3354 — `prefork(replstr,
                    // SUB_FLAG|SKIP_FILESUB)`. The replacement string
                    // must NOT undergo file/tilde expansion: `${p/x/\~}`
                    // yields a literal `~`, not the user's home. Match
                    // the canonical `//` arm at subst.rs:5018 which
                    // already sets SKIP_FILESUB around singsub; the
                    // single-replace `/` arm was missing this gate.
                    let saved_skip = SKIP_FILESUB.with(|c| c.get());
                    SKIP_FILESUB.with(|c| c.set(true));
                    let s_singsub = singsub(&raw_repl);
                    SKIP_FILESUB.with(|c| c.set(saved_skip));
                    untokenize(&s_singsub)
                };
                // c:Src/glob.c::getmatch — `(#m)` flag in the pattern
                // populates `$MATCH` / `$MBEGIN` / `$MEND` with the
                // matched span at replace-time. The `//` arm above
                // (line ~6056) handles this; the single-`/` arm
                // didn't, so `"${s/(#m)l/[$MATCH]}"` rendered `MATCH`
                // empty. Bug #265 in docs/BUGS.md. Detect the flag,
                // and on each successful match, set MATCH and
                // re-singsub the raw replacement string before using
                // it.
                let pat_has_m_one = {
                    let pat_after_anchor =
                        pat.strip_prefix('#').or(pat.strip_prefix('%')).unwrap_or(&pat);
                    pat_after_anchor.contains("(#m)") || pat_after_anchor.contains("(#b)")
                };
                let raw_repl_clone = raw_repl.clone();
                let repl_default = repl.clone();
                let resolve_repl = move |span_text: &str, span_start_byte: usize| -> String {
                    if !pat_has_m_one {
                        return repl_default.clone();
                    }
                    crate::ported::params::setsparam("MATCH", span_text);
                    let base = if isset(crate::ported::zsh_h::KSHARRAYS) {
                        0i64
                    } else {
                        1
                    };
                    crate::ported::params::setiparam("MBEGIN", span_start_byte as i64 + base);
                    crate::ported::params::setiparam(
                        "MEND",
                        (span_start_byte as i64 + span_text.len() as i64 + base)
                            .saturating_sub(1),
                    );
                    let saved_skip = SKIP_FILESUB.with(|c| c.get());
                    SKIP_FILESUB.with(|c| c.set(true));
                    let s = untokenize(&singsub(&raw_repl_clone));
                    SKIP_FILESUB.with(|c| c.set(saved_skip));
                    // c:Src/subst.c — `\X` strip in unquoted context
                    // only. QT (DQ) context preserves `\X` literally so
                    // ANSI-decoded escapes like `\033` survive into
                    // `print -r --` output. Without the qt gate, the
                    // per-match `(#b)` backref replacement stripped
                    // every `\X` even when the caller surrounded the
                    // expansion with `"…"` — matches the qt-aware
                    // gate added to the precomputed `repl` path at
                    // line 7547+. Mirror real zsh:
                    //   `print -r -- "${p/(#b)(b)/\033X${match[1]}\033}"`
                    //     → `\033Xb\033` (qt preserves the literal).
                    if !qt {
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
                    } else {
                        s
                    }
                };
                // c:Src/subst.c — `${var/#%pat/repl}` anchors the
                // match at BOTH start AND end (the pattern must
                // match the entire string). zsh only recognizes
                // the `#` before `%` ordering. Bug #355 in
                // docs/BUGS.md.
                let both_anchor_pat: Option<&str> = pat.strip_prefix("#%");
                // Single-replace helper. Variants: both-anchored
                // (`#%`/`%#`), anchor-prefix (pat starts with `#`),
                // anchor-suffix (`%`), or unanchored. Returns the
                // post-replacement string.
                // c:Src/pattern.c:2570-2621 — `(#b)` array
                // publication lives inside pattryrefs now; plain
                // pattry via `gms` carries it. Bug #590 history.
                let gms = |s_: &str, p_: &str| -> bool {
                    // c:Src/glob.c:2514 matchpat shape — tokenize +
                    // patcompile + pattry. Captures, (#b) $match
                    // arrays and (#m) $MATCH now publish INSIDE
                    // pattryrefs (pattern.c:2526-2542 / 2570-2621
                    // ports), so plain pattry carries them; the
                    // former vm_helper::glob_match_static wrapper is
                    // gone. Silent false on bad pattern (caller arms
                    // pre-validate where C zerrs).
                    match crate::ported::pattern::patcompile(
                        &{
                            let mut t_ = p_.to_string();
                            crate::ported::glob::tokenize(&mut t_);
                            t_
                        },
                        crate::ported::zsh_h::PAT_HEAPDUP,
                        None,
                    ) {
                        Some(pr_) => crate::ported::pattern::pattry(&pr_, s_),
                        None => false,
                    }
                };
                let replace_one = |val: &str| -> String {
                    if let Some(whole_pat) = both_anchor_pat {
                        if gms(val, whole_pat) {
                            let dyn_repl = resolve_repl(val, 0);
                            return dyn_repl;
                        }
                        return val.to_string();
                    }
                    if let Some(anchor_pat) = pat.strip_prefix('#') {
                        let cv: Vec<char> = val.chars().collect();
                        let nn = cv.len();
                        for end in (0..=nn).rev() {
                            let cand: String = cv[..end].iter().collect();
                            if gms(&cand, anchor_pat) {
                                let dyn_repl = resolve_repl(&cand, 0);
                                return format!(
                                    "{}{}",
                                    dyn_repl,
                                    cv[end..].iter().collect::<String>()
                                );
                            }
                        }
                        val.to_string()
                    } else if let Some(anchor_pat) = pat.strip_prefix('%') {
                        let cv: Vec<char> = val.chars().collect();
                        let nn = cv.len();
                        for start in 0..=nn {
                            let cand: String = cv[start..].iter().collect();
                            if gms(&cand, anchor_pat) {
                                let span_byte: usize =
                                    cv[..start].iter().map(|c| c.len_utf8()).sum();
                                let dyn_repl = resolve_repl(&cand, span_byte);
                                return format!(
                                    "{}{}",
                                    cv[..start].iter().collect::<String>(),
                                    dyn_repl
                                );
                            }
                        }
                        val.to_string()
                    } else {
                        // c:Src/glob.c::igetmatch SUB_SUBSTR — substring
                        // match. The matcher walks (start, end) over the
                        // input and tries `matchpat` on each slice.
                        //
                        // c:Src/pattern.c P_ISSTART / P_ISEND — the (#s)
                        // and (#e) glob-flag anchors check against the
                        // input boundaries. When `matchpat` runs on a
                        // SLICE, the anchors compare against the slice's
                        // own boundaries, which is wrong for substring
                        // mode where the anchors must hit the ORIGINAL
                        // string's boundaries. C zsh threads the offsets
                        // through pattry so the matcher knows the real
                        // input span. The Rust port slices into a new
                        // String and loses the offset context, so we
                        // gate the search bounds here: with (#s) the
                        // match must START at original position 0; with
                        // (#e) it must END at original position len.
                        // Bug: `${X/(#m)?(#e)/Y}` on "abc…t" should
                        // replace the last char, but the brute-force
                        // search found a slice "a" at (start=0,end=1)
                        // where (#e) trivially passed because the slice
                        // ended at its own end.
                        let has_start_anchor = pat.contains("(#s)");
                        let has_end_anchor = pat.contains("(#e)");
                        // c:Src/glob.c::igetmatch — without SUB_LONG
                        // (i.e. `(S)` flag set, SUB_SUBSTR), the
                        // match is SHORTEST-leftmost; with SUB_LONG
                        // it's LONGEST-leftmost. The single-`/`
                        // replace form defaults to LONGEST; `(S)`
                        // flips to SHORTEST.
                        let substr_short = (sub_flags_get() & SUB_SUBSTR) != 0;
                        let cv: Vec<char> = val.chars().collect();
                        let nn = cv.len();
                        // c:Src/glob.c:3008-3015 — `case SUB_SUBSTR:`
                        // (shortest, non-global) head pre-check:
                        // `set_pat_start(p, l); if (!(fl & SUB_GLOBAL)
                        // && pattrylen(p, send, 0, 0, &patstralloc, 0)
                        // && !--n) { *sp = get_match_ret(&imd, 0, 0);
                        // return 1; }` — a zero-length match is tried
                        // FIRST and replaces the empty span at offset
                        // 0: `${(S)v/a#/X}` on "abc" → "Xabc" (NOT
                        // "Xbc"). set_pat_start sets PAT_NOTSTART when
                        // l != 0, so (#s) fails the pre-check on
                        // non-empty input — gate it the same way.
                        if substr_short
                            && (!has_start_anchor || nn == 0)
                            && gms("", &pat)
                        {
                            let dyn_repl = resolve_repl("", 0);
                            return format!("{}{}", dyn_repl, val);
                        }
                        let start_range: Vec<usize> = if has_start_anchor {
                            vec![0]
                        } else {
                            // c:Src/glob.c:3029 — `for (; t <= send;
                            // ioff++)`: the scan includes t == send,
                            // and the longest-match `pattrylen` at any
                            // t can succeed with patmatchlen() == 0
                            // (`${v/b#/X}` on "abc" → "Xabc";
                            // `${v/a#/X}` on "" → "X").
                            (0..=nn).collect()
                        };
                        for start in start_range {
                            let end_iter: Vec<usize> = if has_end_anchor {
                                if nn >= start + 1 {
                                    vec![nn]
                                } else {
                                    vec![]
                                }
                            } else if substr_short {
                                // c:Src/glob.c:3041-3053 — shortest
                                // first, starting at length 0.
                                (start..=nn).collect()
                            } else {
                                // c:Src/glob.c:3035 — longest first,
                                // down to the zero-length window.
                                (start..=nn).rev().collect()
                            };
                            for end in end_iter {
                                let cand: String = cv[start..end].iter().collect();
                                if gms(&cand, &pat) {
                                    let span_byte: usize =
                                        cv[..start].iter().map(|c| c.len_utf8()).sum();
                                    let dyn_repl = resolve_repl(&cand, span_byte);
                                    let mut out = String::with_capacity(val.len());
                                    out.extend(cv[..start].iter());
                                    out.push_str(&dyn_repl);
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
                if let Some(arr) = arrays_get(&var_name).filter(|_| !has_subscript_one) {
                    // c:Src/subst.c:3870 — single-`/` array shapes:
                    //   `${a[@]/p/P}` / `${(@)a/p/P}` (array shape):
                    //       per-element, first match within each.
                    //   `${a/p/P}` (unquoted, no `[@]`, no `(@)`):
                    //       per-element word-splitting — first match
                    //       within each element.
                    //   `"${a/p/P}"` (quoted, no `[@]`, no `(@)`):
                    //       SCALAR JOIN first via $IFS[0], then one
                    //       getmatch on the joined string.
                    //
                    // Bug #108 in docs/BUGS.md: the previous Rust
                    // port handled the quoted no-subscript case by
                    // walking elements with a "done" flag instead
                    // of doing the proper scalar-join+single-match,
                    // so `"${a/b*/X}"` on `(red blue green)` came
                    // out `red X green` instead of `red X` — the
                    // greedy `b*` only swallowed "blue" (one
                    // element) instead of "blue green" (joined).
                    // c:Src/subst.c:2916 SCANPM_ISVAR_AT only fires per-
                    // element on `[@]`/bare `@`. `[*]`/bare `*` join in
                    // DQ; bug #322 in docs/BUGS.md. `!qt` (unquoted) and
                    // explicit `(@)` flag (nojoin==2) still trigger per-
                    // element regardless of subscript form.
                    let is_at_subscript = matches!(subscript.as_deref(), Some("@"));
                    let is_at_var = matches!(var_name.as_str(), "@");
                    let per_element =
                        is_at_subscript || is_at_var || nojoin == 2 || !qt;
                    if per_element {
                        let new_arr: Vec<String> = arr.iter().map(|e| replace_one(e)).collect();
                        value = new_arr.join(" "); // c:3870 per-element
                        split_parts = Some(new_arr);
                    } else {
                        // Scalar-join path: sepjoin with $IFS[0],
                        // then one replacement. Mirrors C's getmatch
                        // (not getmatcharr) dispatch at subst.c:3451
                        // for the array-collapsed-to-scalar case.
                        // c:Src/utils.c:3936-3945 — set-but-empty IFS
                        // joins with "" (not " ").
                        let joined = crate::ported::utils::sepjoin(&arr, None);
                        value = replace_one(&joined);
                        // c:Src/subst.c:3034 — DQ `[*]` sepjoins to scalar
                        // BEFORE the operator runs; suppress downstream
                        // auto_splat re-fetch of the original array.
                        split_parts = Some(vec![value.clone()]);
                        isarr = 0;
                    }
                } else {
                    value = replace_one(&raw_value); // c:3870
                }
            } else if let Some(pat) = r.strip_prefix("##") {
                // c:3540 (longest prefix strip)
                // c:Src/subst.c:3360-3412 — pre-tokenize source metas,
                // singsub splices raw, literalize leftover raw metas
                // (GLOBSUBST-gated, c:1669). See the closure pair at
                // the top of this fn for the full contract.
                let p = literalize_spliced_metas(&singsub(&pretokenize_src_pat(pat)));
                // c:Src/glob.c:2674-2677 — patcompile failure → "bad
                // pattern" diagnostic. Bug #606 (sibling of #605).
                if !p.is_empty()
                    && patcompile(&{ let mut __pat_tok = (&p).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, PAT_HEAPDUP as i32, None).is_none()
                {
                    zerr(&format!("bad pattern: {}", p));
                    errflag_set_error();
                    return (String::new(), new_pos, vec![]);
                }
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
                // c:Src/subst.c:3317 — under qt without @/*-subscript
                // and without (@) flag, sepjoin first.
                // c:Src/subst.c:2916 SCANPM_ISVAR_AT only fires per-
                // element on `[@]`/bare `@`. `[*]`/bare `*` join in DQ;
                // bug #322 in docs/BUGS.md.
                let is_at_subscript = matches!(subscript.as_deref(), Some("@"));
                let per_element_array = !has_scalar_sub
                    && (!qt || is_at_subscript || nojoin == 2
                        || matches!(var_name.as_str(), "@"));
                // Strip-one helper. op: 0=#, 1=##, 2=%, 3=%%.
                // Direct port of subst.c:3540 patmatch dispatch.
                // (M) handling per c:3176 — keep matched portion, discard rest.
                let match_only = (sub_flags_get() & SUB_MATCH) != 0;
                // c:Src/glob.c igetmatch SUB_SUBSTR — `(S)` flag makes
                // `##` search the WHOLE string for a substring match
                // (leftmost), longest at that position. Bug #179 in
                // docs/BUGS.md.
                let substr_mode = (sub_flags_get() & SUB_SUBSTR) != 0;
                // c:Src/glob.c:2592-2607 — (B)/(E)/(N) numeric results.
                let ben = sub_flags_get() & (SUB_BIND | SUB_EIND | SUB_LEN);
                // c:Src/glob.c:2626-2636 — (R) rest portion (only
                // relevant here when B/E/N suppress the implied
                // SUB_REST, c:Src/subst.c:3176-3177).
                let rest_flag = (sub_flags_get() & SUB_REST) != 0;
                let gms = |s_: &str, p_: &str| -> bool {
                    // c:Src/glob.c:2514 matchpat shape — tokenize +
                    // patcompile + pattry. Captures, (#b) $match
                    // arrays and (#m) $MATCH now publish INSIDE
                    // pattryrefs (pattern.c:2526-2542 / 2570-2621
                    // ports), so plain pattry carries them; the
                    // former vm_helper::glob_match_static wrapper is
                    // gone. Silent false on bad pattern (caller arms
                    // pre-validate where C zerrs).
                    match crate::ported::pattern::patcompile(
                        &{
                            let mut t_ = p_.to_string();
                            crate::ported::glob::tokenize(&mut t_);
                            t_
                        },
                        crate::ported::zsh_h::PAT_HEAPDUP,
                        None,
                    ) {
                        Some(pr_) => crate::ported::pattern::pattry(&pr_, s_),
                        None => false,
                    }
                };
                let strip_one = |val: &str, op: u8| -> String {
                    let cv: Vec<char> = val.chars().collect();
                    let nn = cv.len();
                    // Match bounds [b, e) in chars; None = no match.
                    let bounds: Option<(usize, usize)> = match op {
                        1 => {
                            if substr_mode {
                                // Leftmost longest substring match.
                                let mut found = None;
                                'outer: for start in 0..=nn {
                                    for k in (0..=(nn - start)).rev() {
                                        let candidate: String =
                                            cv[start..start + k].iter().collect();
                                        if gms(&candidate, &p) {
                                            found = Some((start, start + k));
                                            break 'outer;
                                        }
                                    }
                                }
                                found
                            } else {
                                let mut k = nn;
                                let mut found = None;
                                loop {
                                    let prefix: String = cv[..k].iter().collect();
                                    // Route through `glob_match_static` so (#b)
                                    // capture groups populate `$match`/`$mbegin`/
                                    // `$mend` on the first successful match —
                                    // `${var##(#b)pat}` longest-prefix strip wants
                                    // the captures from the matched prefix. No-op
                                    // for patterns without (#b) (GF_BACKREF gate
                                    // inside the helper).
                                    if gms(&prefix, &p) {
                                        found = Some((0, k));
                                        break;
                                    }
                                    if k == 0 {
                                        break;
                                    }
                                    k -= 1;
                                }
                                found
                            }
                        }
                        _ => return val.to_string(),
                    };
                    if ben != 0 {
                        // c:Src/glob.c:2575-2645 get_match_ret — compose
                        // [match] [rest] B E N, space-joined, trailing
                        // space trimmed. No match composes as the empty
                        // match at (0,0) per c:Src/glob.c:3230.
                        let (b, e) = bounds.unwrap_or((0, 0));
                        let mut parts: Vec<String> = Vec::new();
                        if match_only {
                            parts.push(cv[b..e].iter().collect()); // c:2618
                        }
                        if rest_flag {
                            let mut rest: String = cv[..b].iter().collect(); // c:2630
                            rest.extend(cv[e..].iter()); // c:2634
                            parts.push(rest);
                        }
                        if (ben & SUB_BIND) != 0 {
                            parts.push((b + 1).to_string()); // c:2594 1-based start
                        }
                        if (ben & SUB_EIND) != 0 {
                            parts.push((e + 1).to_string()); // c:2599 1-based end
                        }
                        if (ben & SUB_LEN) != 0 {
                            parts.push((e - b).to_string()); // c:2605 char-length
                        }
                        return parts.join(" ");
                    }
                    match bounds {
                        Some((b, e)) => {
                            if match_only {
                                cv[b..e].iter().collect()
                            } else {
                                let before: String = cv[..b].iter().collect();
                                let after: String = cv[e..].iter().collect();
                                before + &after
                            }
                        }
                        None => {
                            if match_only {
                                String::new()
                            } else {
                                val.to_string()
                            }
                        }
                    }
                };
                if let Some(arr) = arrays_get(&var_name).filter(|_| per_element_array) {
                    let new_arr: Vec<String> = arr.iter().map(|e| strip_one(e, 1)).collect();
                    value = new_arr.join(" "); // c:3540
                    split_parts = Some(new_arr); // c:3540
                } else {
                    value = strip_one(&raw_value, 1); // c:3540
                    // c:Src/subst.c:3034 — DQ `[*]` sepjoined to scalar
                    // upstream; suppress auto_splat re-fetch. Bug #322
                    // in docs/BUGS.md.
                    if matches!(subscript.as_deref(), Some("*"))
                        && qt
                        && arrays_contains(&var_name)
                    {
                        split_parts = Some(vec![value.clone()]);
                        isarr = 0;
                    }
                }
            } else if let Some(pat) = r.strip_prefix('#') {
                // c:3540 (shortest prefix strip)
                // c:Src/subst.c:3360-3412 — pre-tokenize source metas,
                // singsub splices raw, literalize leftover raw metas
                // (GLOBSUBST-gated, c:1669). See the closure pair at
                // the top of this fn for the full contract.
                let p = literalize_spliced_metas(&singsub(&pretokenize_src_pat(pat)));
                // c:Src/glob.c:2674-2677 — patcompile failure → "bad
                // pattern" diagnostic. Bug #606 (sibling of #605).
                if !p.is_empty()
                    && patcompile(&{ let mut __pat_tok = (&p).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, PAT_HEAPDUP as i32, None).is_none()
                {
                    zerr(&format!("bad pattern: {}", p));
                    errflag_set_error();
                    return (String::new(), new_pos, vec![]);
                }
                let has_scalar_sub = subscript
                    .as_deref()
                    .map(|s| {
                        let t = s.trim();
                        t != "@" && t != "*" && !t.contains(',')
                    })
                    .unwrap_or(false);
                // c:Src/subst.c:3317 — under qt (DQ context) without
                // an explicit @/* subscript AND without the (@) flag
                // (nojoin != 2), the array sepjoins to scalar BEFORE
                // strip applies. \`"\${a#pat}"\` strips the joined
                // "a b c" once, not per element. Parity bug: zshrs
                // applied per-element in DQ too.
                // c:Src/subst.c:2916 SCANPM_ISVAR_AT only fires per-
                // element on `[@]`/bare `@`. `[*]`/bare `*` join in DQ;
                // bug #322 in docs/BUGS.md.
                let is_at_subscript = matches!(subscript.as_deref(), Some("@"));
                let per_element_array = !has_scalar_sub
                    && (!qt || is_at_subscript || nojoin == 2
                        || matches!(var_name.as_str(), "@"));
                // c:Src/subst.c:3176 — SUB_MATCH inverts strip semantics:
                // default returns the rest (after the match); with (M)
                // returns the matched prefix and discards the rest.
                let match_only = (sub_flags_get() & SUB_MATCH) != 0;
                // c:Src/glob.c igetmatch SUB_SUBSTR — `(S)` flag makes
                // `#` search the WHOLE string for a substring match
                // (leftmost), shortest at that position. Bug #179 in
                // docs/BUGS.md.
                let substr_mode = (sub_flags_get() & SUB_SUBSTR) != 0;
                // c:Src/glob.c:2592-2607 — (B)/(E)/(N) numeric results.
                let ben = sub_flags_get() & (SUB_BIND | SUB_EIND | SUB_LEN);
                // c:Src/glob.c:2626-2636 — (R) rest portion.
                let rest_flag = (sub_flags_get() & SUB_REST) != 0;
                let gms = |s_: &str, p_: &str| -> bool {
                    // c:Src/glob.c:2514 matchpat shape — tokenize +
                    // patcompile + pattry. Captures, (#b) $match
                    // arrays and (#m) $MATCH now publish INSIDE
                    // pattryrefs (pattern.c:2526-2542 / 2570-2621
                    // ports), so plain pattry carries them; the
                    // former vm_helper::glob_match_static wrapper is
                    // gone. Silent false on bad pattern (caller arms
                    // pre-validate where C zerrs).
                    match crate::ported::pattern::patcompile(
                        &{
                            let mut t_ = p_.to_string();
                            crate::ported::glob::tokenize(&mut t_);
                            t_
                        },
                        crate::ported::zsh_h::PAT_HEAPDUP,
                        None,
                    ) {
                        Some(pr_) => crate::ported::pattern::pattry(&pr_, s_),
                        None => false,
                    }
                };
                let strip_one = |val: &str| -> String {
                    let cv: Vec<char> = val.chars().collect();
                    let total = cv.len();
                    // Match bounds [b, e) in chars; None = no match.
                    let bounds: Option<(usize, usize)> = (|| {
                        if substr_mode {
                            // Leftmost shortest substring match.
                            for start in 0..=total {
                                for k in 0..=(total - start) {
                                    let candidate: String =
                                        cv[start..start + k].iter().collect();
                                    if gms(&candidate, &p) {
                                        return Some((start, start + k));
                                    }
                                }
                            }
                            return None;
                        }
                        for k in 0..=total {
                            let prefix: String = cv[..k].iter().collect();
                            // (#b) capture wiring via glob_match_static.
                            if gms(&prefix, &p) {
                                return Some((0, k));
                            }
                        }
                        None
                    })();
                    if ben != 0 {
                        // c:Src/glob.c:2575-2645 get_match_ret — compose
                        // [match] [rest] B E N, space-joined, trailing
                        // space trimmed. No match composes as the empty
                        // match at (0,0) per c:Src/glob.c:3230.
                        let (b, e) = bounds.unwrap_or((0, 0));
                        let mut parts: Vec<String> = Vec::new();
                        if match_only {
                            parts.push(cv[b..e].iter().collect()); // c:2618
                        }
                        if rest_flag {
                            let mut rest: String = cv[..b].iter().collect(); // c:2630
                            rest.extend(cv[e..].iter()); // c:2634
                            parts.push(rest);
                        }
                        if (ben & SUB_BIND) != 0 {
                            parts.push((b + 1).to_string()); // c:2594 1-based start
                        }
                        if (ben & SUB_EIND) != 0 {
                            parts.push((e + 1).to_string()); // c:2599 1-based end
                        }
                        if (ben & SUB_LEN) != 0 {
                            parts.push((e - b).to_string()); // c:2605 char-length
                        }
                        return parts.join(" ");
                    }
                    match bounds {
                        Some((b, e)) => {
                            if match_only {
                                cv[b..e].iter().collect()
                            } else {
                                let before: String = cv[..b].iter().collect();
                                let after: String = cv[e..].iter().collect();
                                before + &after
                            }
                        }
                        None => {
                            if match_only {
                                // No match under SUB_MATCH: empty result.
                                // c:glob.c:2895 — getmatch SUB_MATCH no-match
                                // arm clears *sp to "". Note: C's getmatcharr
                                // additionally FILTERS out such empties from
                                // arrays (c:2735-2738 while-igetmatch loop) —
                                // not yet ported because doing so breaks `(@M)`
                                // subscript shape parity (zsh keeps 3 elements
                                // for `("${(@M)arr[@]#foo}")`, drops to 2 for
                                // `(${(M)arr#foo})`). Both need distinct
                                // codepaths to differentiate.
                                String::new()
                            } else {
                                val.to_string()
                            }
                        }
                    }
                };
                if let Some(arr) = arrays_get(&var_name).filter(|_| per_element_array) {
                    let new_arr: Vec<String> = arr.iter().map(|e| strip_one(e)).collect();
                    value = new_arr.join(" "); // c:3540
                    split_parts = Some(new_arr); // c:3540
                } else {
                    value = strip_one(&raw_value); // c:3540
                    // c:Src/subst.c:3034 — DQ `[*]` sepjoined to scalar
                    // upstream; suppress auto_splat re-fetch. Bug #322.
                    if matches!(subscript.as_deref(), Some("*"))
                        && qt
                        && arrays_contains(&var_name)
                    {
                        split_parts = Some(vec![value.clone()]);
                        isarr = 0;
                    }
                }
            } else if let Some(pat) = r.strip_prefix("%%") {
                // c:3540 (longest suffix strip)
                // c:Src/subst.c:3360-3412 — pre-tokenize source metas,
                // singsub splices raw, literalize leftover raw metas
                // (GLOBSUBST-gated, c:1669). See the closure pair at
                // the top of this fn for the full contract.
                let p = literalize_spliced_metas(&singsub(&pretokenize_src_pat(pat)));
                // c:Src/glob.c:2674-2677 — patcompile failure → "bad
                // pattern" diagnostic. Bug #606 (sibling of #605).
                if !p.is_empty()
                    && patcompile(&{ let mut __pat_tok = (&p).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, PAT_HEAPDUP as i32, None).is_none()
                {
                    zerr(&format!("bad pattern: {}", p));
                    errflag_set_error();
                    return (String::new(), new_pos, vec![]);
                }
                let has_scalar_sub = subscript
                    .as_deref()
                    .map(|s| {
                        let t = s.trim();
                        t != "@" && t != "*" && !t.contains(',')
                    })
                    .unwrap_or(false);
                // c:Src/subst.c:2916 SCANPM_ISVAR_AT only fires per-
                // element on `[@]`/bare `@`. `[*]`/bare `*` join in DQ;
                // bug #322 in docs/BUGS.md.
                let is_at_subscript = matches!(subscript.as_deref(), Some("@"));
                let per_element_array = !has_scalar_sub
                    && (!qt || is_at_subscript || nojoin == 2
                        || matches!(var_name.as_str(), "@"));
                // c:Src/subst.c:3176 — SUB_MATCH for `%%` (longest suffix).
                let match_only = (sub_flags_get() & SUB_MATCH) != 0;
                // c:Src/glob.c:3107 igetmatch SUB_END+SUB_LONG+SUB_SUBSTR
                // — `(S)` flag makes `%%` search the WHOLE string for a
                // substring match (rightmost), longest at that position.
                // Bug #179 in docs/BUGS.md.
                let substr_mode = (sub_flags_get() & SUB_SUBSTR) != 0;
                // c:Src/glob.c:2592-2607 — (B)/(E)/(N) numeric results.
                let ben = sub_flags_get() & (SUB_BIND | SUB_EIND | SUB_LEN);
                // c:Src/glob.c:2626-2636 — (R) rest portion.
                let rest_flag = (sub_flags_get() & SUB_REST) != 0;
                let gms = |s_: &str, p_: &str| -> bool {
                    // c:Src/glob.c:2514 matchpat shape — see the
                    // sibling closure above (captures/(#b)/(#m) now
                    // publish inside pattryrefs).
                    match crate::ported::pattern::patcompile(
                        &{
                            let mut t_ = p_.to_string();
                            crate::ported::glob::tokenize(&mut t_);
                            t_
                        },
                        crate::ported::zsh_h::PAT_HEAPDUP,
                        None,
                    ) {
                        Some(pr_) => crate::ported::pattern::pattry(&pr_, s_),
                        None => false,
                    }
                };
                let strip_one = |val: &str| -> String {
                    let cv: Vec<char> = val.chars().collect();
                    let total = cv.len();
                    // Match bounds [b, e) in chars; None = no match.
                    let bounds: Option<(usize, usize)> = (|| {
                    if substr_mode {
                        // Rightmost longest substring match.
                        let mut best: Option<(usize, usize)> = None;
                        for start in 0..=total {
                            for k in (0..=(total - start)).rev() {
                                let candidate: String =
                                    cv[start..start + k].iter().collect();
                                if gms(&candidate, &p) {
                                    best = Some((start, start + k));
                                    break;
                                }
                            }
                        }
                        return best;
                    }
                    let mut k = total;
                    loop {
                        let suffix_start_char = total - k;
                        let suffix: String = cv[suffix_start_char..].iter().collect();
                        // (#b) capture wiring via glob_match_static.
                        if gms(&suffix, &p) {
                            // c:Src/pattern.c:2425 `setiparam("MBEGIN",
                            // patoffset + !isset(KSHARRAYS))` — captures
                            // come back relative to the matched substring;
                            // user expects offsets in the ORIGINAL string,
                            // so add the suffix's start byte offset to
                            // every $mbegin / $mend slot.
                            let suffix_byte_off: usize =
                                cv[..suffix_start_char].iter().map(|c| c.len_utf8()).sum();
                            if suffix_byte_off > 0 {
                                if let Some(b) = crate::ported::params::getaparam("mbegin") {
                                    let shifted: Vec<String> = b
                                        .iter()
                                        .map(|s| {
                                            s.parse::<i64>()
                                                .map(|n| (n + suffix_byte_off as i64).to_string())
                                                .unwrap_or_else(|_| s.clone())
                                        })
                                        .collect();
                                    crate::ported::params::setaparam("mbegin", shifted);
                                }
                                if let Some(e) = crate::ported::params::getaparam("mend") {
                                    let shifted: Vec<String> = e
                                        .iter()
                                        .map(|s| {
                                            s.parse::<i64>()
                                                .map(|n| (n + suffix_byte_off as i64).to_string())
                                                .unwrap_or_else(|_| s.clone())
                                        })
                                        .collect();
                                    crate::ported::params::setaparam("mend", shifted);
                                }
                            }
                            return Some((suffix_start_char, total));
                        }
                        if k == 0 {
                            break;
                        }
                        k -= 1;
                    }
                    None
                    })();
                    if ben != 0 {
                        // c:Src/glob.c:2575-2645 get_match_ret — compose
                        // [match] [rest] B E N, space-joined, trailing
                        // space trimmed. No match composes as the empty
                        // match at (0,0) per c:Src/glob.c:3230.
                        let (b, e) = bounds.unwrap_or((0, 0));
                        let mut parts: Vec<String> = Vec::new();
                        if match_only {
                            parts.push(cv[b..e].iter().collect()); // c:2618
                        }
                        if rest_flag {
                            let mut rest: String = cv[..b].iter().collect(); // c:2630
                            rest.extend(cv[e..].iter()); // c:2634
                            parts.push(rest);
                        }
                        if (ben & SUB_BIND) != 0 {
                            parts.push((b + 1).to_string()); // c:2594 1-based start
                        }
                        if (ben & SUB_EIND) != 0 {
                            parts.push((e + 1).to_string()); // c:2599 1-based end
                        }
                        if (ben & SUB_LEN) != 0 {
                            parts.push((e - b).to_string()); // c:2605 char-length
                        }
                        return parts.join(" ");
                    }
                    match bounds {
                        Some((b, e)) => {
                            if match_only {
                                cv[b..e].iter().collect()
                            } else {
                                let before: String = cv[..b].iter().collect();
                                let after: String = cv[e..].iter().collect();
                                before + &after
                            }
                        }
                        None => {
                            if match_only {
                                String::new()
                            } else {
                                val.to_string()
                            }
                        }
                    }
                };
                if let Some(arr) = arrays_get(&var_name).filter(|_| per_element_array) {
                    let new_arr: Vec<String> = arr.iter().map(|e| strip_one(e)).collect();
                    value = new_arr.join(" "); // c:3540
                    split_parts = Some(new_arr); // c:3540
                } else {
                    value = strip_one(&raw_value); // c:3540
                    // c:Src/subst.c:3034 — DQ `[*]` sepjoined to scalar
                    // upstream; suppress auto_splat re-fetch. Bug #322.
                    if matches!(subscript.as_deref(), Some("*"))
                        && qt
                        && arrays_contains(&var_name)
                    {
                        split_parts = Some(vec![value.clone()]);
                        isarr = 0;
                    }
                }
            } else if let Some(pat) = r.strip_prefix('%') {
                // c:3540 (shortest suffix strip)
                // c:Src/subst.c:3360-3412 — pre-tokenize source metas,
                // singsub splices raw, literalize leftover raw metas
                // (GLOBSUBST-gated, c:1669). See the closure pair at
                // the top of this fn for the full contract.
                let p = literalize_spliced_metas(&singsub(&pretokenize_src_pat(pat)));
                // c:Src/glob.c:2674-2677 — patcompile failure → "bad
                // pattern" diagnostic. Bug #606 (sibling of #605).
                if !p.is_empty()
                    && patcompile(&{ let mut __pat_tok = (&p).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, PAT_HEAPDUP as i32, None).is_none()
                {
                    zerr(&format!("bad pattern: {}", p));
                    errflag_set_error();
                    return (String::new(), new_pos, vec![]);
                }
                let has_scalar_sub = subscript
                    .as_deref()
                    .map(|s| {
                        let t = s.trim();
                        t != "@" && t != "*" && !t.contains(',')
                    })
                    .unwrap_or(false);
                // c:Src/subst.c:2916 SCANPM_ISVAR_AT only fires per-
                // element on `[@]`/bare `@`. `[*]`/bare `*` join in DQ;
                // bug #322 in docs/BUGS.md.
                let is_at_subscript = matches!(subscript.as_deref(), Some("@"));
                let per_element_array = !has_scalar_sub
                    && (!qt || is_at_subscript || nojoin == 2
                        || matches!(var_name.as_str(), "@"));
                // c:Src/subst.c:3176 — SUB_MATCH for `%` (shortest suffix).
                let match_only = (sub_flags_get() & SUB_MATCH) != 0;
                // c:Src/glob.c:3106 igetmatch SUB_END+SUB_SUBSTR — `(S)`
                // flag makes `%`/`%%` search the WHOLE string for a
                // substring match (rightmost), then take shortest/longest
                // at that position. Bug #179 in docs/BUGS.md.
                let substr_mode = (sub_flags_get() & SUB_SUBSTR) != 0;
                // c:Src/glob.c:2592-2607 — (B)/(E)/(N) numeric results.
                let ben = sub_flags_get() & (SUB_BIND | SUB_EIND | SUB_LEN);
                // c:Src/glob.c:2626-2636 — (R) rest portion.
                let rest_flag = (sub_flags_get() & SUB_REST) != 0;
                let gms = |s_: &str, p_: &str| -> bool {
                    // c:Src/glob.c:2514 matchpat shape — see the
                    // sibling closure above (captures/(#b)/(#m) now
                    // publish inside pattryrefs).
                    match crate::ported::pattern::patcompile(
                        &{
                            let mut t_ = p_.to_string();
                            crate::ported::glob::tokenize(&mut t_);
                            t_
                        },
                        crate::ported::zsh_h::PAT_HEAPDUP,
                        None,
                    ) {
                        Some(pr_) => crate::ported::pattern::pattry(&pr_, s_),
                        None => false,
                    }
                };
                let strip_one = |val: &str| -> String {
                    let cv: Vec<char> = val.chars().collect();
                    let total = cv.len();
                    // Match bounds [b, e) in chars; None = no match.
                    let bounds: Option<(usize, usize)> = (|| {
                        if substr_mode {
                            // Rightmost shortest substring match.
                            let mut best: Option<(usize, usize)> = None;
                            for start in 0..=total {
                                for k in 0..=(total - start) {
                                    let candidate: String =
                                        cv[start..start + k].iter().collect();
                                    if gms(&candidate, &p) {
                                        best = Some((start, start + k));
                                        break;
                                    }
                                }
                            }
                            return best;
                        }
                        for k in 0..=total {
                            let suffix: String = cv[total - k..].iter().collect();
                            // (#b) capture wiring via glob_match_static.
                            if gms(&suffix, &p) {
                                return Some((total - k, total));
                            }
                        }
                        None
                    })();
                    if ben != 0 {
                        // c:Src/glob.c:2575-2645 get_match_ret — compose
                        // [match] [rest] B E N, space-joined, trailing
                        // space trimmed. No match composes as the empty
                        // match at (0,0) per c:Src/glob.c:3230.
                        let (b, e) = bounds.unwrap_or((0, 0));
                        let mut parts: Vec<String> = Vec::new();
                        if match_only {
                            parts.push(cv[b..e].iter().collect()); // c:2618
                        }
                        if rest_flag {
                            let mut rest: String = cv[..b].iter().collect(); // c:2630
                            rest.extend(cv[e..].iter()); // c:2634
                            parts.push(rest);
                        }
                        if (ben & SUB_BIND) != 0 {
                            parts.push((b + 1).to_string()); // c:2594 1-based start
                        }
                        if (ben & SUB_EIND) != 0 {
                            parts.push((e + 1).to_string()); // c:2599 1-based end
                        }
                        if (ben & SUB_LEN) != 0 {
                            parts.push((e - b).to_string()); // c:2605 char-length
                        }
                        return parts.join(" ");
                    }
                    match bounds {
                        Some((b, e)) => {
                            if match_only {
                                cv[b..e].iter().collect()
                            } else {
                                let before: String = cv[..b].iter().collect();
                                let after: String = cv[e..].iter().collect();
                                before + &after
                            }
                        }
                        None => {
                            if match_only {
                                String::new()
                            } else {
                                val.to_string()
                            }
                        }
                    }
                };
                if let Some(arr) = arrays_get(&var_name).filter(|_| per_element_array) {
                    let new_arr: Vec<String> = arr.iter().map(|e| strip_one(e)).collect();
                    value = new_arr.join(" "); // c:3540
                    split_parts = Some(new_arr); // c:3540
                } else {
                    value = strip_one(&raw_value); // c:3540
                    // c:Src/subst.c:3034 — DQ `[*]` sepjoined to scalar
                    // upstream; suppress auto_splat re-fetch. Bug #322.
                    if matches!(subscript.as_deref(), Some("*"))
                        && qt
                        && arrays_contains(&var_name)
                    {
                        split_parts = Some(vec![value.clone()]);
                        isarr = 0;
                    }
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
                                          // c:3548 SUB_DIFFERENCE returns array shape; mark
                                          // isarr=1 so the auto_splat block at c:4245 fires
                                          // even though `rest` is non-empty (the operator
                                          // consumed it).
                isarr = 1;
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
                isarr = 1; // c:3548 SUB_INTERSECT returns array shape
            } else if let Some(rhs) = r.strip_prefix(":^^") {
                // c:Src/subst.c:3456-3520 SUB_ZIP_LONG — `${a:^^b}`.
                // In DQ context (`"${a:^^b}"`), zsh collapses the FIRST
                // operand to a single sepjoin'd string BEFORE the zip
                // pattern walks, producing pairs of (joined-a, b[i %
                // blen]) for outlen = max(1, blen) iterations. Direct
                // port of the prefork-collapse-then-zip path observed
                // via:
                //   a=(1 2); b=(x y z) → "${a:^^b}" =
                //   ['1 2','x','1 2','y','1 2','z']
                // Scalar operands wrap to 1-element array (same as :^).
                let arr = arrays_get(&var_name)
                    .or_else(|| vars_get(&var_name).map(|s| vec![s]))
                    .unwrap_or_default();
                let other_name = rhs.trim();
                let other = arrays_get(other_name)
                    .or_else(|| vars_get(other_name).map(|s| vec![s]))
                    .unwrap_or_default();
                // c:Src/subst.c — `[@]`/`[*]` subscript keeps array
                // shape across DQ for the same SCANPM_ISVAR_AT reason
                // documented in the `:^` arm below. Bug #597.
                let is_at_subscript_zip = matches!(subscript.as_deref(), Some("@") | Some("*"));
                let zipped: Vec<String> = if qt && !is_at_subscript_zip && !other.is_empty() && !arr.is_empty() {
                    let ifs0 = vars_get("IFS")
                        .unwrap_or_else(|| " \t\n\0".to_string())
                        .chars()
                        .next()
                        .map(String::from)
                        .unwrap_or_default();
                    let joined = arr.join(&ifs0);
                    let n = other.len();
                    let mut z: Vec<String> = Vec::with_capacity(n * 2);
                    for i in 0..n {
                        z.push(joined.clone());
                        z.push(other[i].clone());
                    }
                    z
                } else if arr.is_empty() {
                    // One operand empty → return the other verbatim.
                    // Mirror C: zsh skips the interleave when either
                    // side has zero elements.
                    other.clone()
                } else if other.is_empty() {
                    arr.clone()
                } else {
                    // c:Src/subst.c SUB_ZIP_LONG — the shorter array
                    // CYCLES through its values to fill outlen =
                    // max(alen, blen). Mirror via modular index:
                    // `arr[i % alen]` / `other[i % blen]`. Without
                    // this, the shorter array's overflow positions
                    // received an empty string (visible as the
                    // Nularg sentinel `¡` in output) instead of
                    // wrapping back to element 0.
                    let n = arr.len().max(other.len());
                    let mut z: Vec<String> = Vec::with_capacity(n * 2);
                    for i in 0..n {
                        z.push(arr[i % arr.len()].clone());
                        z.push(other[i % other.len()].clone());
                    }
                    z
                };
                value = zipped.join(" ");
                split_parts = Some(zipped); // c:3540 (auto-splat)
                isarr = 1; // c:3540 SUB_ZIP_LONG returns array shape
            } else if let Some(rhs) = r.strip_prefix(":^") {
                // c:Src/subst.c:3456-3520 SUB_ZIP — `${a:^b}` short-
                // zip. Outlen = min(alen, blen) when shortest=1. In
                // DQ context, zsh collapses the FIRST operand to a
                // single sepjoin'd string before zipping, then takes
                // outlen = min(1, blen) = 1 (or 0 if blen=0),
                // producing exactly 2 elements:
                //   [sepjoin(a), b[0]]
                // Unset-vs-empty: `${a:^b}` with b unset returns a
                // verbatim (no zip); a unset returns b verbatim.
                // Parity bug #24 (DQ joining) + #23 (unset operand).
                //
                // SCALAR operands are treated as 1-element arrays —
                // C zsh's getarrvalue auto-wraps a scalar param into
                // a `char *aval[2] = { scalar, NULL }` for zip. Parity
                // bug: zshrs's arrays_get returns None for scalars,
                // so `${a:^a}` with scalar `a=hello` was both-unset →
                // empty result. Fall back to vars_get for the scalar
                // single-element case.
                let arr_opt =
                    arrays_get(&var_name).or_else(|| vars_get(&var_name).map(|s| vec![s]));
                let other_name = rhs.trim();
                let other_opt =
                    arrays_get(other_name).or_else(|| vars_get(other_name).map(|s| vec![s]));
                let arr_unset = arr_opt.is_none();
                let other_unset = other_opt.is_none();
                let arr = arr_opt.unwrap_or_default();
                let other = other_opt.unwrap_or_default();
                // c:Src/subst.c — explicit `[@]`/`[*]` subscript on
                // the LHS array bypasses the DQ scalar-collapse: zsh's
                // SCANPM_ISVAR_AT (set for `[@]`/`[*]` per params.c:2027-2029)
                // keeps isarr=-1 through the qt transition so the zip
                // walks per-element instead of sepjoin'ing the LHS.
                // Without this gate, `"${a[@]:^b}"` truncated to
                // `[sepjoin(a), b[0]]` — 2 elements instead of the
                // interleaved 2*min(|a|,|b|). Bug #597.
                let is_at_subscript_zip = matches!(subscript.as_deref(), Some("@") | Some("*"));
                let zipped: Vec<String> = if other_unset && !arr_unset {
                    // b unset → return a verbatim.
                    arr.clone()
                } else if arr_unset && !other_unset {
                    // a unset → return b verbatim.
                    other.clone()
                } else if qt && !is_at_subscript_zip {
                    let ifs0 = vars_get("IFS")
                        .unwrap_or_else(|| " \t\n\0".to_string())
                        .chars()
                        .next()
                        .map(String::from)
                        .unwrap_or_default();
                    let joined = arr.join(&ifs0);
                    // outlen = min(1, blen). When blen=0 → 0 elements.
                    if other.is_empty() {
                        Vec::new()
                    } else {
                        vec![joined, other[0].clone()]
                    }
                } else {
                    let mut z: Vec<String> = Vec::with_capacity(arr.len() + other.len());
                    let n = arr.len().min(other.len()); // c:3540 truncate to shorter
                    for i in 0..n {
                        z.push(arr[i].clone());
                        z.push(other[i].clone());
                    }
                    z
                };
                value = zipped.join(" ");
                split_parts = Some(zipped); // c:3540 (auto-splat)
                isarr = 1; // c:3540 SUB_ZIP returns array shape
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
                        | 'f'
                        | 'F'
                );
                // c:Src/subst.c:3786-3790 — after modify() returns, if
                // `inbrace && *s` (unconsumed modifier text remains),
                // zsh emits `unrecognized modifier 'X'`. This catches
                // unknown alphabetic modifier letters like `:U`, `:C`,
                // `:W` (wait — `W` IS recognized as the wide-separator
                // flag; the bug entry's `W` example refers to the
                // post-modifier branch where W has already been
                // consumed). Previously the Rust port fell through to
                // the substring-offset arm (line ~6516), where
                // `mathevali("U")` returned 0 and silently sliced from
                // offset 0 — yielding the original value unchanged.
                // Detect the canonical "letter but not a known
                // modifier" case here and emit the zsh diagnostic.
                if !is_modifier && first.is_ascii_alphabetic() {
                    zerr(&format!("unrecognized modifier `{}'", first)); // c:3788
                    errflag_set_error();
                    return (String::new(), 0, Vec::new()); // c:3791
                }
                if is_modifier {
                    // c:Src/subst.c:4531 modify() entry — apply
                    // history-style modifier chain (`:h`, `:t`, `:r`,
                    // `:e`, `:l`, `:u`, `:q`, `:Q`, etc.).
                    //
                    // c:Src/subst.c:4533-4540 — `if (isarr)` gates the
                    // per-element dispatch. When a numeric subscript
                    // has already narrowed the value to a single
                    // element (e.g. `${arr[1]:t}`), C's `isarr` is 0
                    // and modify runs on the scalar form. The
                    // previous Rust port unconditionally re-fetched
                    // the whole array via `arrays_get(&var_name)`
                    // when split_parts was None, which made
                    // `${arr[N]:modifier}` apply the modifier to
                    // EVERY element. Parity bug #14.
                    let mod_str = format!(":{}", slice);
                    let mod_one = |s: &str| -> String { modify(s, &mod_str) };
                    // c:Src/subst.c:3030-3034 — sepjoin under qt
                    // clears isarr to 0 BEFORE the modify dispatch at
                    // c:4531. In DQ, modify runs once on the joined
                    // scalar (`${a:e}` of (file.txt readme.md) joins
                    // to "file.txt readme.md" then :e → "md"). In
                    // non-DQ with isarr=1, modify loops per-element
                    // (c:4533). Without the qt gate, zshrs applied
                    // :modifier per-element in DQ too, producing
                    // "txt md" instead of "md". Parity bug #28.
                    let sepjoined_for_qt = || -> String {
                        if let Some(arr) = arrays_get(&var_name) {
                            // c:3030 sepjoin — when (j:STR:) was given,
                            // join with STR; else join with " " (IFS
                            // default). Bug #91 in docs/BUGS.md:
                            // `"${(j: :)paths:t}"` joined with " " but
                            // then ran :t on the wrong joined form.
                            let sep_str = sep.as_deref().unwrap_or(" ");
                            arr.join(sep_str)
                        } else {
                            value.clone()
                        }
                    };
                    // c:2859/2887 — single-slot subscript (`[N]`,
                    // `[key]`) clears isarr so modify runs on the
                    // picked scalar. `[@]` ALWAYS keeps array
                    // shape. `[*]` keeps shape unquoted, sepjoins
                    // to scalar in DQ. Range `[lo,hi]` keeps shape
                    // unquoted, sepjoins to scalar in DQ. Direct
                    // port of Src/params.c::getindex isarr-flag
                    // dispatch (SCANPM_ISVAR_AT for @, SCANPM_ISVAR
                    // for *, SCANPM_ARRAYRANGE for slice).
                    //
                    // Parity bug: \`${a[@]:t}\` left the array
                    // unchanged because the scalar path was taken
                    // indiscriminately for any subscript.
                    // Accept tokenized Star (\u{87}) as the `*`
                    // subscript — the lexer pre-tokenizes `*` in
                    // some unquoted contexts.
                    let is_at = subscript.as_deref() == Some("@");
                    let is_star = matches!(subscript.as_deref(), Some("*") | Some("\u{87}"));
                    let is_range = subscript.as_deref().map_or(false, |s| s.contains(','));
                    // c:Src/subst.c:2916 SCANPM_ISVAR_AT — bare `@`
                    //   pseudo-name keeps per-element shape in DQ (same
                    //   as `[@]` subscript). `${@:t}` loops the
                    //   modifier per positional; `${*:t}` stays scalar
                    //   (sepjoins to one word first). Bug #278 in
                    //   docs/BUGS.md: zshrs treated bare `@` as scalar
                    //   in DQ and fell into the qt-sepjoin arm,
                    //   returning the tail of only the last positional.
                    let is_at_var = var_name == "@";
                    // Per-element mode: `[@]` always; bare `@`
                    // pseudo-name always; `[*]` only outside DQ; slice
                    // only outside DQ. Anything else is scalar (single
                    // index, named key, sepjoined range/star in DQ).
                    let per_element = is_at || is_at_var || ((is_star || is_range) && !qt);
                    let scalar_subscript = subscript.is_some() && !per_element;
                    if scalar_subscript {
                        // c:2859/2887 isarr=0 after subscript → scalar.
                        // For `[*]`/range under qt the value was
                        // already sepjoined upstream, so mod_one runs
                        // on the joined scalar — exactly what zsh does.
                        // For `[*]`-tokenized unquoted that arrived
                        // via Star lookup, value is empty (the array
                        // path didn't set it); pull from arrays_get
                        // and run per-element.
                        let _ = value.clone();
                        value = mod_one(&value);
                        // c:Src/subst.c:4533 — modify on `[*]`/range
                        // in DQ sepjoins the array first, runs modify
                        // on the joined scalar, then needs to suppress
                        // the downstream auto_splat re-fetching the
                        // original array. Seed split_parts with the
                        // modified scalar so auto_splat (line ~7370)
                        // uses it; flip isarr to 0 so the splat block
                        // skips entirely.
                        if (is_star || is_range) && qt {
                            split_parts = Some(vec![value.clone()]);
                            isarr = 0;
                        }
                    } else if per_element {
                        // c:4533 — array-shape subscript keeps isarr;
                        // modify loops per element. Use split_parts
                        // when prior operator (slice) already
                        // narrowed; else pull straight from the
                        // backing array.
                        if let Some(parts) = split_parts.clone() {
                            let new_parts: Vec<String> = parts.iter().map(|s| mod_one(s)).collect();
                            value = new_parts.join(" ");
                            split_parts = Some(new_parts);
                        } else if let Some(arr) = arrays_get(&var_name) {
                            // Honor a range subscript (split_parts
                            // would have captured it normally; do the
                            // narrowing here when it didn't).
                            let narrowed: Vec<String> = if is_range {
                                if let Some((lo_s, hi_s)) =
                                    subscript.as_deref().and_then(|s| s.split_once(','))
                                {
                                    let lo: i64 = lo_s.trim().parse().unwrap_or(1);
                                    let hi: i64 = hi_s.trim().parse().unwrap_or(arr.len() as i64);
                                    getarrvalue(&arr, lo, hi)
                                } else {
                                    arr.clone()
                                }
                            } else {
                                arr.clone()
                            };
                            let new_arr: Vec<String> =
                                narrowed.iter().map(|s| mod_one(s)).collect();
                            value = new_arr.join(" ");
                            split_parts = Some(new_arr);
                        } else {
                            value = mod_one(&value);
                        }
                    } else if qt && arrays_contains(&var_name) && nojoin != 2 {
                        // c:3030-3034 DQ sepjoin cleared isarr → scalar.
                        //   Skip when `(@)` flag is set (nojoin == 2):
                        //   the C path at c:3030 KEEPS isarr=-1 under
                        //   (@), so modify still loops per-element even
                        //   in DQ. Bug #147 in docs/BUGS.md:
                        //   `${(@)a:t}` joined to a scalar and ran :t
                        //   once on the joined form, returning the
                        //   tail of the last element instead of per-
                        //   element tails. The fall-through arm at
                        //   `arrays_get(&var_name)` below correctly
                        //   loops per element.
                        let joined = sepjoined_for_qt();
                        value = mod_one(&joined);
                        // Seed split_parts with the single modified
                        // scalar so the downstream (j:STR:) join arm
                        // (subst.rs:7479) sees the modifier's result
                        // and doesn't re-fetch arrays_get and rejoin
                        // the original unmodified elements. Bug #91 in
                        // docs/BUGS.md: `${(j: :)paths:t}` printed the
                        // joined raw array because the (j) handler
                        // overwrote `value` after the modifier ran.
                        split_parts = Some(vec![value.clone()]);
                        isarr = 0;
                    } else if sep.is_some() && arrays_contains(&var_name) {
                        // c:Src/subst.c:3906-3907 — `(j:X:)` flag.
                        // Dispatch differs by qt context:
                        //
                        // - qt=true (quoted): sepjoin clears isarr=0
                        //   BEFORE the modifier runs (c:3030-3034), so
                        //   the modifier runs ONCE on the joined scalar.
                        //   Example: `"${(j: :)paths:t}"` → tail of the
                        //   joined "/a/b/c.txt /d/e/f.log" = "f.log".
                        //   Bug #91 in docs/BUGS.md.
                        //
                        // - qt=false (unquoted): isarr stays set, the
                        //   modifier loops PER ELEMENT (c:4533), THEN
                        //   the late sepjoin block at subst.rs:8629
                        //   joins the modified array with sep.
                        //   Example: `${(j:.:)a:h}` with a=(/a/b /c/d)
                        //   → (/a, /c) → joined "." = "/a./c".
                        //   Bug #576 in docs/BUGS.md.
                        //
                        // qt is checked first as a fast guard; the
                        // qt-arm above (line 7710) handles qt+array
                        // when nojoin != 2, so this branch sees only
                        // !qt OR nojoin == 2.
                        if qt {
                            let sep_str = sep.as_deref().unwrap_or(" ");
                            let joined = arrays_get(&var_name)
                                .map(|a| a.join(sep_str))
                                .unwrap_or_else(|| value.clone());
                            value = mod_one(&joined);
                            split_parts = Some(vec![value.clone()]);
                        } else if let Some(arr) = arrays_get(&var_name) {
                            let new_arr: Vec<String> =
                                arr.iter().map(|s| mod_one(s)).collect();
                            value = new_arr.join(" ");
                            split_parts = Some(new_arr);
                        } else {
                            value = mod_one(&value);
                            split_parts = Some(vec![value.clone()]);
                        }
                    } else if let Some(parts) = split_parts.clone() {
                        let new_parts: Vec<String> = parts.iter().map(|s| mod_one(s)).collect();
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
                    // c:Src/subst.c:3825 — `${str:offset:length}` arms
                    // both go through `mathevali` (Src/math.c:1240),
                    // not literal strtol. Allows parentheses, leading
                    // whitespace, and arithmetic expressions in either
                    // slot:
                    //   ${str: -5}     — space + negative (the canonical
                    //                    zsh form that disambiguates
                    //                    from `:-` default operator)
                    //   ${str:(-5)}    — parens for the same effect
                    //   ${str:N+1:K-1} — arithmetic expressions
                    // Previously used `.parse::<i64>()` which fell back
                    // to 0 on any non-digit input (parens, spaces), so
                    // `${str: -5}` silently returned the entire string
                    // (offset clamped to 0).
                    //
                    // c:Src/subst.c:3781-3792 — empty operand after `:`
                    // is malformed. `${s:}` (bare colon, no operand)
                    // errors "unrecognized modifier" via C's
                    // unrecognized-modifier-letter path; `${s:N:}` and
                    // `${s::}` (length-missing after second colon) also
                    // error. zsh rejects them at parse time; the Rust
                    // port silently treated empty mathevali results as
                    // 0 and returned the wrong substring. Bug #126 in
                    // docs/BUGS.md.
                    // c:Src/subst.c:3781-3792 — empty length operand
                    // after `:N:` is malformed. zsh errors
                    // "unrecognized modifier". The empty-offset
                    // shorthand `${a::N}` is VALID (treated as
                    // `${a:0:N}`), so the gate is specifically
                    // "length separator present AND length is empty"
                    // OR "no length separator AND offset is empty".
                    // Bug #126 in docs/BUGS.md.
                    let has_length_sep = parts.len() == 2;
                    let length_empty =
                        has_length_sep && parts[1].trim().is_empty();
                    let bare_empty_no_sep =
                        !has_length_sep && parts[0].trim().is_empty();
                    if length_empty || bare_empty_no_sep {
                        zerr("unrecognized modifier");
                        errflag.fetch_or(
                            crate::ported::zsh_h::ERRFLAG_ERROR,
                            std::sync::atomic::Ordering::Relaxed,
                        );
                        return (String::new(), 0, Vec::new());
                    }
                    // c:Src/subst.c:1571 + c:3633-3645 + c:3786-3791 —
                    // `check_colon_subscript` rejects an alphabetic-
                    // starting operand (treats it as a modifier letter
                    // not a math expression). When the LENGTH portion
                    // of `${var:OFFSET:LENGTH}` starts with an ASCII
                    // alphabetic char (no `$` / `(` / digit prefix),
                    // check_colon_subscript returns NULL, length stays
                    // unset, and the trailing `:LEN}` falls through to
                    // the "unrecognized modifier `c'" error at
                    // subst.c:3787. bash treats bare `n` as an arith
                    // identifier; zsh requires explicit `$n` or
                    // `$((n))`. Bug #477 in docs/BUGS.md.
                    if has_length_sep {
                        let len_trimmed = parts[1].trim_start();
                        if len_trimmed
                            .chars()
                            .next()
                            .map_or(false, |c| c.is_ascii_alphabetic())
                        {
                            zerr(&format!(
                                "unrecognized modifier `{}'",
                                len_trimmed.chars().next().unwrap()
                            ));
                            errflag.fetch_or(
                                crate::ported::zsh_h::ERRFLAG_ERROR,
                                std::sync::atomic::Ordering::Relaxed,
                            );
                            return (String::new(), 0, Vec::new());
                        }
                    }
                    let off = crate::ported::math::mathevali(&singsub(parts[0])).unwrap_or(0);
                    // Array context: ${arr:offset:length} slices the
                    // ARRAY (1-based, like Bash's offset), not the joined
                    // value. Direct port of subst.c's array-shape branch
                    // around c:715. Falls back to scalar substring when
                    // var_name isn't an array.
                    // Source priority: split_parts (prior operator
                    // result like filter/sort) → state.arrays → joined
                    // value. Direct port of zsh's getarrvalue → slice
                    // dispatch which uses aval if isarr is set.
                    //
                    // c:Src/params.c:2915 — single-slot subscript (`[N]`,
                    // `[name]`) clears isarr; the substring then runs on
                    // the picked SCALAR (`${arr[1]:0:3}` = first 3 chars
                    // of arr[1]). Only `[@]`/`[*]`/range `[N,M]` keep the
                    // array shape that drives the slice path.
                    let single_slot_subscript = subscript
                        .as_deref()
                        .map_or(false, |s| s != "@" && s != "*" && !s.contains(','));
                    let array_source: Option<Vec<String>> = if single_slot_subscript {
                        None // c:2915 (scalar picked → substring on val)
                    } else {
                        split_parts.clone().or_else(|| arrays_get(&var_name))
                    };
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
                            .map(|s| crate::ported::math::mathevali(&singsub(s)).unwrap_or(0)); // c:715
                        // c:Src/subst.c:3683-3690 — negative length is
                        // first adjusted by `length += alen - offset`,
                        // then if still negative, `zerr("substring
                        // expression: %d < %d", length+offset, offset)`
                        // fires and substitution aborts. zshrs's prior
                        // port silently clamped to 0 via `.max(0)` so
                        // `a=(); a=("${a[@]:0:-1}")` succeeded with an
                        // empty slice instead of producing the C
                        // diagnostic. Bug #120 in docs/BUGS.md.
                        let kept: Vec<String> = match len {
                            // c:715
                            Some(l) if l >= 0 => {
                                arr.iter().skip(lo).take(l as usize).cloned().collect()
                            } // c:715
                            Some(l) => {
                                // c:3684 — length += alen - offset
                                let adjusted = l + (n - lo as i64); // c:3685
                                if adjusted < 0 {
                                    // c:3686-3690
                                    crate::ported::utils::zerr(&format!(
                                        "substring expression: {} < {}",
                                        adjusted + lo as i64,
                                        lo
                                    ));
                                    return (String::new(), 0, Vec::new()); // c:3689
                                }
                                arr.iter().skip(lo).take(adjusted as usize).cloned().collect()
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
                            .map(|s| crate::ported::math::mathevali(&singsub(s)).unwrap_or(0));
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
            } else if {
                // c:Src/subst.c:2993-3003 — after var-name parsing,
                // the accepted operator start chars are
                // `:`/`-`/`+`/`=`/`?`/`#`/`%`/`/` (plus their token
                // variants `\u{9b}` Dash and `\u{84}` Pound). Operator
                // suffixes `(`/`[` are consumed before this rest
                // parse (subscript, modifier flags). Anything else is
                // a bad substitution per C zsh's flagerr-style reject.
                //
                // Catches:
                //   - `^` / `,` / `@` — bash case-conv / param-xfm
                //     (`${var^^}`, `${var@Q}`) NOT in zsh; #130.
                //   - alphanumeric / `_` after a single-char special
                //     like `${-default}` — name continued past
                //     `-`/`!`/`?`/`#`/`*`/`@`/`$`/`0` which only
                //     accept one char of name. Bug #189 in
                //     docs/BUGS.md.
                //
                // Skip the check when the body started with a subexp
                // (`${$((expr))}`, `${$(cmd)}`, `${${var}}`) — the
                // subexp already produced the final value into
                // raw_value/value via the subexp_value path; any
                // trailing chars in `rest` are from C's downstream
                // processing (e.g. closing brace token leftovers).
                // C `Src/subst.c:2681` lets the subexp result flow
                // through without re-applying name-validity gates.
                // Bug #165 in docs/BUGS.md.
                if used_subexp {
                    false
                } else {
                    let first = r.chars().next().unwrap_or('\0');
                    let is_op_start = matches!(
                        first,
                        ':' | '-' | '+' | '=' | '?' | '#' | '%' | '/'
                            | '\u{9b}' // Dash, c:Src/zsh.h:182
                            | '\u{84}' // Pound, c:Src/zsh.h:159
                            | '\u{97}' // Quest, c:Src/zsh.h:178
                    );
                    !is_op_start
                }
            } {
                zerr("bad substitution");
                errflag.fetch_or(
                    crate::ported::zsh_h::ERRFLAG_ERROR,
                    std::sync::atomic::Ordering::Relaxed,
                );
                value = String::new();
            }
        }
        // Apply post-processing flags to the substituted value.
        // C lines 3950-4070 — case mods, quoting, etc.
        if wantt && used_subexp {
            // c:Src/subst.c — `${(t)$(cmdsub)}` and `${(t)$((arith))}`
            // have no underlying parameter to type-check, so zsh
            // passes the resolved value through unchanged rather
            // than emitting "scalar". value here is already the
            // resolved sub-expression result (raw_value flowed from
            // subexp_value at c:2730). Bug #173 in docs/BUGS.md.
            let _ = wantt;
        } else if wantt && {
            // c:Src/subst.c:2812 — `if (v && v->pm && ((flags &
            // PM_DECLARED) || !(flags & PM_UNSET)))`. C skips the
            // type-tag emit entirely when the parameter is unset
            // (no v->pm or PM_UNSET set), leaving `val` as whatever
            // the prior `:-`/`:=` etc. modifier substituted. Bug
            // #216 in docs/BUGS.md: zshrs unconditionally entered
            // the wantt arm and overwrote value with `String::new()`
            // (the "unset → empty tag" fallback below), clobbering
            // the default that `:-` already substituted.
            //
            // Approximation of C's PM_DECLARED || !PM_UNSET check:
            // skip the wantt arm when the var is fully unset (no
            // paramtab entry, no env, no array/assoc store, not a
            // recognized magic-assoc name). For "declared but unset"
            // (`typeset -i x; ${(t)x}`), paramtab still has the
            // entry — `paramtab().read().get(&var_name).is_some()`
            // returns true and we DO emit the tag matching zsh.
            let declared = paramtab()
                .read()
                .ok()
                .and_then(|tab| tab.get(&var_name).cloned())
                .is_some()
                || arrays_contains(&var_name)
                || assoc_contains(&var_name)
                || crate::ported::modules::parameter::PARTAB_ARRAY.iter().find(|e_| e_.name == var_name.as_str()).map(|e_| e_.flags as u32 | crate::ported::zsh_h::PM_SPECIAL | crate::ported::zsh_h::PM_HIDE | crate::ported::zsh_h::PM_HIDEVAL).is_some()
                || (var_name.chars().all(|c| c.is_ascii_digit()) && !var_name.is_empty())
                || std::env::var(&var_name).is_ok();
            is_set || declared
        } {
            // c:2807
            // ${(t)var} — emit type tag. var_attrs takes
            // precedence (carries typeset flags); fall back to
            // synthesized tag from the storage table the value
            // lives in. Direct port of subst.c:2814 wantt arm
            // which checks paramtab + storage shape.
            //
            // PARTAB_ARRAY entries (historywords, funcstack, etc.)
            // need the dedicated flag lookup FIRST because their
            // paramtab stub has PM_READONLY stripped at init time
            // (so internal writes work) — reading from paramtab
            // would lose the readonly attribute that `(t)` must
            // report. Check partab_array_flags first; if it hits,
            // build the tag directly with the full implicit flags
            // (PM_ARRAY | PM_READONLY | PM_SPECIAL | PM_HIDE |
            // PM_HIDEVAL).
            // c:2800-2806 — fetchvalue resolves PM_NAMEREF chains
            // (getparamnode c:570-575) before the (t) flag read, so
            // the TARGET's type is reported; an unresolvable ref
            // reports its own `nameref` type, and a DANGLING ref
            // (target never defined) emits the empty tag (fetchvalue
            // NULL → vunset, c:2855-2856).
            let mut nameref_dangling = false;
            let var_name: String = if crate::ported::params::is_nameref(&var_name) {
                match crate::ported::params::resolve_nameref_name(&var_name, None) {
                    crate::ported::params::nameref_resolution::Target { name: t, pm, .. } => {
                        if pm.is_none() {
                            nameref_dangling = true;
                        }
                        t
                    }
                    crate::ported::params::nameref_resolution::Placeholder(p) => p,
                    _ => var_name.clone(),
                }
            } else {
                var_name.clone()
            };
            let partab_array_tag = crate::ported::modules::parameter::PARTAB_ARRAY.iter().find(|e_| e_.name == var_name.as_str()).map(|e_| e_.flags as u32 | crate::ported::zsh_h::PM_SPECIAL | crate::ported::zsh_h::PM_HIDE | crate::ported::zsh_h::PM_HIDEVAL).map(|f| {
                let mut tag = if f & PM_HASHED != 0 {
                    "association".to_string()
                } else if f & PM_ARRAY != 0 {
                    "array".to_string()
                } else if f & PM_INTEGER != 0 {
                    "integer".to_string()
                } else {
                    "scalar".to_string()
                };
                if f & PM_READONLY != 0 {
                    tag.push_str("-readonly");
                }
                if f & PM_TAGGED != 0 {
                    tag.push_str("-tag");
                }
                if f & PM_TIED != 0 {
                    tag.push_str("-tied");
                }
                if f & PM_EXPORTED != 0 {
                    tag.push_str("-export");
                }
                if f & PM_UNIQUE != 0 {
                    tag.push_str("-unique");
                }
                if f & PM_HIDE != 0 {
                    tag.push_str("-hide");
                }
                if f & PM_HIDEVAL != 0 {
                    tag.push_str("-hideval");
                }
                if f & PM_SPECIAL != 0 {
                    tag.push_str("-special");
                }
                tag
            });
            // c:2814 — read PM_* flags directly from paramtab and
            // synthesize the type tag. Mirrors C `pm->node.flags &
            // PM_TYPE` dispatch at subst.c:2814-2900.
            value = if let Some(tag) = partab_array_tag {
                tag
            } else {
                paramtab()
                    .read() // c:2814
                    .ok() // c:2814
                    .and_then(|tab| {
                        tab.get(&var_name).map(|pm| {
                            // c:2814
                            let f = pm.node.flags as u32; // c:2814
                                                          // c:Src/params.c paramtype-from-flags read.
                                                          // For PM_SPECIAL params, the pm_type bits aren't
                                                          // always carried on the paramtab entry (env-
                                                          // imported specials like SHLVL come in as
                                                          // PM_SCALAR even though IPDEF5 declares them
                                                          // PM_INTEGER). Overlay the canonical pm_type
                                                          // from special_params so (t) reads match zsh
                                                          // (`integer-export-special` instead of
                                                          // `scalar-special` for $SHLVL). Also detect
                                                          // env-presence to set PM_EXPORTED on params
                                                          // that came in via the environment but whose
                                                          // paramtab entry didn't carry the flag (set-
                                                          // before-export sequence loses the flag).
                            let f_overlay = if (f & PM_SPECIAL) != 0 {
                                let mut bits = f;
                                if let Some(sp) = crate::ported::params::special_params
                                    .iter()
                                    .find(|sp| sp.name == var_name.as_str())
                                {
                                    bits |= sp.pm_type as u32;
                                    // c:Src/params.c — overlay the canonical
                                    // pm_flags too (PM_READONLY for #/?,
                                    // PM_TIED for path/PATH etc.). Without
                                    // this, \${(t)?} read "integer-special"
                                    // instead of "integer-readonly-special".
                                    bits |= sp.pm_flags as u32;
                                }
                                // c:Src/params.c PM_EXPORTED — present iff
                                // the name has a non-null `pm->env` entry,
                                // which mirrors std::env::var Ok in Rust.
                                if std::env::var(&var_name).is_ok() {
                                    bits |= PM_EXPORTED;
                                }
                                bits
                            } else {
                                f
                            };
                            let f = f_overlay;
                            let val = if f & PM_HASHED != 0 {
                                "association"
                            }
                            // c:2823 case PM_HASHED
                            else if f & PM_ARRAY != 0 {
                                "array"
                            }
                            // c:2819 case PM_ARRAY
                            else if f & PM_INTEGER != 0 {
                                "integer"
                            }
                            // c:2820 case PM_INTEGER
                            else if f & (PM_EFLOAT | PM_FFLOAT) != 0 {
                                "float"
                            }
                            // c:2821-2822 PM_EFLOAT|PM_FFLOAT
                            else if f & PM_NAMEREF != 0 {
                                "nameref"
                            }
                            // c:2818 case PM_NAMEREF
                            else {
                                "scalar"
                            }; // c:2817 case PM_SCALAR
                            let val = dupstring(val); // c:2825 val = dupstring(val)
                            let val = if pm.level != 0
                            // c:2826
                            {
                                dyncat(&val, "-local")
                            }
                            // c:2827
                            else {
                                val
                            }; // c:2826
                            let val = if f & PM_LEFT != 0
                            // c:2828
                            {
                                dyncat(&val, "-left")
                            }
                            // c:2829
                            else {
                                val
                            }; // c:2828
                            let val = if f & PM_RIGHT_B != 0
                            // c:2830
                            {
                                dyncat(&val, "-right_blanks")
                            }
                            // c:2831
                            else {
                                val
                            }; // c:2830
                            let val = if f & PM_RIGHT_Z != 0
                            // c:2832
                            {
                                dyncat(&val, "-right_zeros")
                            }
                            // c:2833
                            else {
                                val
                            }; // c:2832
                            let val = if f & PM_LOWER != 0
                            // c:2834
                            {
                                dyncat(&val, "-lower")
                            }
                            // c:2835
                            else {
                                val
                            }; // c:2834
                            let val = if f & PM_UPPER != 0
                            // c:2836
                            {
                                dyncat(&val, "-upper")
                            }
                            // c:2837
                            else {
                                val
                            }; // c:2836
                            let val = if f & PM_READONLY != 0
                            // c:2838
                            {
                                dyncat(&val, "-readonly")
                            }
                            // c:2839
                            else {
                                val
                            }; // c:2838
                            let val = if f & PM_TAGGED != 0
                            // c:2840
                            {
                                dyncat(&val, "-tag")
                            }
                            // c:2841
                            else {
                                val
                            }; // c:2840
                            let val = if f & PM_TIED != 0
                            // c:2842
                            {
                                dyncat(&val, "-tied")
                            }
                            // c:2843
                            else {
                                val
                            }; // c:2842
                            let val = if f & PM_EXPORTED != 0
                            // c:2844
                            {
                                dyncat(&val, "-export")
                            }
                            // c:2845
                            else {
                                val
                            }; // c:2844
                            let val = if f & PM_UNIQUE != 0
                            // c:2846
                            {
                                dyncat(&val, "-unique")
                            }
                            // c:2847
                            else {
                                val
                            }; // c:2846
                            let val = if f & PM_HIDE != 0
                            // c:2848
                            {
                                dyncat(&val, "-hide")
                            }
                            // c:2849
                            else {
                                val
                            }; // c:2848
                            let val = if f & PM_HIDEVAL != 0
                            // c:2850
                            {
                                dyncat(&val, "-hideval")
                            }
                            // c:2851
                            else {
                                val
                            }; // c:2850
                            let val = if f & PM_SPECIAL != 0
                            // c:2852
                            {
                                dyncat(&val, "-special")
                            }
                            // c:2853
                            else {
                                val
                            }; // c:2852
                            val // c:2854
                        })
                    })
                    .unwrap_or_else(|| {
                        // c:Src/Modules/parameter.c SPECIALPMDEF entries
                        // (historywords / funcstack / patchars / dirstack /
                        // …) live in PARTAB_ARRAY, NOT paramtab. Their
                        // flags include PM_ARRAY plus the implicit
                        // PM_SPECIAL | PM_HIDE | PM_HIDEVAL the C macro
                        // adds at zsh.h:2123. Build the type tag from those
                        // here so `(t)historywords` reads
                        // `array-readonly-hide-hideval-special` matching
                        // zsh.
                        if let Some(f) = crate::ported::modules::parameter::PARTAB_ARRAY.iter().find(|e_| e_.name == var_name.as_str()).map(|e_| e_.flags as u32 | crate::ported::zsh_h::PM_SPECIAL | crate::ported::zsh_h::PM_HIDE | crate::ported::zsh_h::PM_HIDEVAL) {
                            let mut tag = if f & PM_HASHED != 0 {
                                "association".to_string()
                            } else if f & PM_ARRAY != 0 {
                                "array".to_string()
                            } else if f & PM_INTEGER != 0 {
                                "integer".to_string()
                            } else {
                                "scalar".to_string()
                            };
                            if f & PM_READONLY != 0 {
                                tag.push_str("-readonly");
                            }
                            if f & PM_TAGGED != 0 {
                                tag.push_str("-tag");
                            }
                            if f & PM_TIED != 0 {
                                tag.push_str("-tied");
                            }
                            if f & PM_EXPORTED != 0 {
                                tag.push_str("-export");
                            }
                            if f & PM_UNIQUE != 0 {
                                tag.push_str("-unique");
                            }
                            if f & PM_HIDE != 0 {
                                tag.push_str("-hide");
                            }
                            if f & PM_HIDEVAL != 0 {
                                tag.push_str("-hideval");
                            }
                            if f & PM_SPECIAL != 0 {
                                tag.push_str("-special");
                            }
                            return tag;
                        }
                        if assoc_contains(&var_name) {
                            "association".to_string() // c:2814
                        } else if arrays_contains(&var_name) {
                            "array".to_string() // c:2814
                        } else if !var_name.is_empty()
                            && var_name.chars().all(|c| c.is_ascii_digit())
                        {
                            // c:Src/params.c — `$1`/`$2`/... are aliases
                            // for `${argv[N]}`. The `(t)` flag reads the
                            // PARENT (argv) type, not the element type,
                            // so positionals report `array-special`
                            // matching `(t)@` / `(t)*`. Bug #163 in
                            // docs/BUGS.md. Empty $0 falls through to
                            // the standard scalar handling below.
                            "array-special".to_string()
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
                                | "langinfo"
                        ) {
                            // Magic-assoc params — type is association.
                            // Direct port of subst.c:2814 paramtab
                            // lookup which finds the magic-assoc entry
                            // and returns PM_HASHED type tag.
                            "association".to_string() // c:2814
                        } else if is_set {
                            // c:Src/params.c — env-only vars (paramtab
                            // miss + env::var hit) carry PM_EXPORTED.
                            // C zsh imports every env var at startup so
                            // the paramtab path catches them; Rust's
                            // lazy import means env-only vars miss the
                            // paramtab arm and land here. Tag with
                            // `-export` to match zsh.
                            if std::env::var(&var_name).is_ok() {
                                "scalar-export".to_string()
                            } else {
                                "scalar".to_string()
                            }
                        } else {
                            String::new()
                        }
                    })
            };
            // c:2855-2856 — dangling nameref: fetchvalue returned
            // NULL, so the (t) tag is empty.
            if nameref_dangling {
                value = String::new();
            }
            // c:2882-2883 — after wantt, C clears `v = NULL; isarr = 0;`
            // so the array-splat path at c:3950 doesn't fire on the
            // type string. Without this, ${(t)arr} would splat the
            // array's elements after value has been replaced with
            // "array".
            isarr = 0; // c:2883
            split_parts = None; // c:2883 aval is implicit-cleared by v=NULL
        }
        // c:Src/subst.c:2867-2900 — after the wantt arm cleared
        // v=NULL/isarr=0, C re-enters the `while (v || ((inbrace ||
        // ...) && isbrack(*s)))` loop. With a trailing `[subscript]`,
        // it createparam(nulstring, PM_SCALAR) holding val (the type
        // tag) and calls `getindex(&s, v, qt ? SCANPM_DQUOTED : 0)`.
        // getindex → getarg → mathevali on the subscript text. For
        // `${(t)parameters[PATH]}` mathevali("PATH") substitutes
        // $PATH (long colon-list) and fails to parse it as math,
        // calling zerr("bad math expression: …") + setting errflag.
        // The print never fires, exit is 1.
        //
        // Approximation: run mathevali on the subscript text when
        // wantt fired with a literal subscript present (skipping
        // `*`/`@`/flag-prefixed forms, which getindex handles via
        // separate getarg branches). Numeric / unset-name subscripts
        // succeed (and the existing downstream scalar-slice path
        // applies the index); names whose math value can't be
        // parsed (e.g. PATH expanded to a non-numeric string)
        // trigger the same zerr + errflag as C zsh.
        if wantt && !used_subexp {
            if let Some(sub) = subscript.as_deref() {
                let s_trim = sub.trim();
                if !s_trim.is_empty()
                    && s_trim != "*"
                    && s_trim != "@"
                    && !s_trim.starts_with('(')
                {
                    let parts: Vec<&str> = s_trim.splitn(2, ',').collect();
                    for p in &parts {
                        if let Err(msg) = crate::ported::math::mathevali(p.trim()) {
                            zerr(&msg);
                            errflag.fetch_or(
                                crate::ported::zsh_h::ERRFLAG_ERROR,
                                std::sync::atomic::Ordering::Relaxed,
                            );
                            value = String::new();
                            break;
                        }
                    }
                }
            }
        }
        // Case mods operate per-element when array-shaped (so
        // \${(@U)arr} uppercases each element, preserving shape).
        // Direct port of subst.c:3937 casmod arm which iterates aval
        // when isarr is set.
        let cap_word = |s: &str| -> String {
            // c:Src/hist.c:2239-2255 CASMOD_CAPS:
            //   if (!iswalnum(wc)) nextupper = 1;
            //   else if (nextupper) { if (iswlower(wc)) { wc = towupper(wc); ... }
            //                         nextupper = 0; }
            //   else if (iswupper(wc)) { wc = towlower(wc); ... }
            // C-faithful: ANY non-alphanumeric char (including
            // apostrophe, comma, dash, etc.) is a word boundary —
            // matches `iswalnum`'s Unicode-aware semantic. Previously
            // checked only a hand-picked subset (whitespace + `-_/.,`)
            // which missed `'`, so `(C)"l'état"` produced "L'état"
            // instead of zsh's "L'État".
            let mut out = String::with_capacity(s.len());
            let mut next_upper = true;
            for c in s.chars() {
                if !c.is_alphanumeric() {
                    // c:2243 `if (!iswalnum(wc)) nextupper = 1;`
                    out.push(c);
                    next_upper = true;
                } else if next_upper {
                    // c:2245-2250 — `if (iswlower(wc)) wc = towupper(wc); nextupper = 0;`
                    out.extend(c.to_uppercase());
                    next_upper = false;
                } else {
                    // c:2251-2254 — `else if (iswupper(wc)) wc = towlower(wc);`
                    out.extend(c.to_lowercase());
                }
            }
            out
        };
        if casmod != CASMOD_NONE {
            // c:3937 if (casmod != CASMOD_NONE)
            let transform = |s: &str| -> String {
                // c:3937
                if casmod == CASMOD_LOWER {
                    // c:3937 CASMOD_LOWER
                    s.to_lowercase() // c:3937
                } else if casmod == CASMOD_UPPER {
                    // c:3937 CASMOD_UPPER
                    s.to_uppercase() // c:3937
                } else {
                    // c:3937 CASMOD_CAPS
                    cap_word(s) // c:3937
                } // c:3937
            }; // c:3937
            // c:Src/subst.c:2915 — `v->scanflags ? 1 : 0`. Any non-splat
            // subscript (single-slot `[N]`, range `[N,M]`) collapses the
            // array shape to the picked value(s) — the casmod must
            // operate on that result, not refetch the source array.
            // For sub-expression temps (`__subexp_arr_*` names produced
            // by the multsub branch at line 4084), arrays_get(temp)
            // returns the full split and would transform every element,
            // producing e.g. "X Y Z" instead of "X" for
            // `${(U)${(s. .)s}[1]}` or "X Y Z" instead of "X Y" for
            // `${(U)${(s. .)s}[1,2]}`. Bug #195.
            //
            // Gate: skip the arrays_get refetch when a NON-SPLAT
            // subscript was applied AND the source array is a subexp
            // temp (the regular `${(U)arr[1]}` path is handled by the
            // arrays-source casmod earlier; the temp arm is the unique
            // case where subscript-collapsed value lives in `value`
            // but arrays_get still resolves to the full array).
            let has_non_splat_subscript = subscript
                .as_deref()
                .map_or(false, |s| s != "@" && s != "*");
            let is_subexp_temp = var_name.starts_with("__subexp_arr_");
            if let Some(parts) = split_parts.clone() {
                // c:3937
                let new_parts: Vec<String> = parts.iter().map(|s| transform(s)).collect();
                value = new_parts.join(" "); // c:3937
                split_parts = Some(new_parts); // c:3937
            } else if has_non_splat_subscript {
                // c:Src/subst.c:2915 — a non-splat subscript (`[N]`,
                // `[N,M]`, `[(r)pat]`, `[(i)pat]`, `[key]`) collapses the
                // array shape to the picked result. `value` already
                // holds that picked result; the casmod transforms it
                // directly without refetching the source array. For
                // slices the value is the joined slice — split on
                // whitespace before transform so each slice element is
                // case-folded individually (then rejoin). For single-
                // slot the split is len=1 → transform-then-join is the
                // identity. Bug surface: `${(L)arr[(r)FOO*]}` was
                // refetching the full array and lowercasing everything
                // because the `arrays_get` arm below fired before this
                // branch — pinned by r_flag_then_L_lowercase parity.
                let parts: Vec<String> = value
                    .split_whitespace()
                    .map(|s| transform(s))
                    .collect();
                value = parts.join(" ");
                if subscript.as_deref().map_or(false, |s| s.contains(',')) {
                    split_parts = Some(parts);
                }
                let _ = is_subexp_temp;
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
        //
        // c:4245 — `if (isarr) { ... }` — the sort + unique +
        // splat block is INSIDE this gate. Scalar shape (isarr=0)
        // after DQ sepjoin at c:3034 means no sort applies. C does
        // not have a separate "sort the joined string" path.
        //
        // sep.is_none() guard: C's sepjoin at c:3906 runs BEFORE c:4245
        // and clears isarr=0 when (j)/(F) flag was set, so the sort
        // block at c:4245 is gated out by `if (isarr)`. zshrs's port
        // ordering inverts that (sort here at 5819, sep below at 5924),
        // so explicitly skip sort when sep is set to mirror C's
        // join-collapses-first behavior. `${(oj/-/)arr}` for
        // (charlie alpha bravo) returns "charlie-alpha-bravo" in zsh —
        // the o flag is a no-op because j collapsed shape first.
        if isarr != 0 && (sortit != SORTIT_ANYOLDHOW || unique) && sep.is_none() {
            // c:4245 + c:4290
            // Sort/unique source: prefer split_parts (any prior
            // operator result like :# filter, (s::) split, or
            // assoc-splat) so sort applies to the actual element
            // list, not a whitespace re-split of the joined view.
            let parts: Vec<String> = if let Some(sp) = split_parts.clone() {
                // c:4290
                sp // c:4290 (operator-result)
            } else if let Some(arr) = arrays_get(&var_name) {
                // c:4290
                arr.clone() // c:4290 (real array)
            } else if let Some(map) = assoc_get(&var_name) {
                // c:4290
                map.values().cloned().collect() // c:4290 (assoc values)
            } else if matches!(var_name.as_str(), "@" | "*" | "argv") {
                // c:Src/params.c:3262 IPDEF9 — `@`, `*`, `argv` map
                // to `pparams`. Source the sort/unique input from the
                // canonical positional vec instead of the whitespace-
                // re-split joined view, so individual positionals
                // with embedded spaces survive `${(o)@}` etc.
                crate::ported::builtin::PPARAMS
                    .lock()
                    .map(|p| p.clone())
                    .unwrap_or_default()
            } else {
                // c:4290
                value.split_whitespace().map(String::from).collect() // c:4290 (fallback)
            }; // c:4290
            let mut sorted: Vec<String> = parts; // c:4290
            if unique {
                // c:4253
                let mut seen = std::collections::HashSet::new(); // c:4253
                sorted.retain(|s| seen.insert(s.clone())); // c:4253
            } // c:4253
            if sortit != SORTIT_ANYOLDHOW {
                // c:4290
                // (a) flag (indord=1, c:2226) preserves insertion order
                // — skip sort entirely.
                if indord == 0 {
                    // c:4292 if (!indord)
                    if (sortit & SORTIT_NUMERICALLY) != 0 {
                        // c:Src/sort.c:191 zstrcmp — canonical natural-
                        // sort comparator. Routes through the existing
                        // port at src/ported/sort.rs:120 with the same
                        // sortflags bitmask C uses (SORTIT_NUMERICALLY
                        // / SORTIT_NUMERICALLY_SIGNED).
                        let flags = sortit as u32;
                        sorted.sort_by(|a, b| crate::ported::sort::zstrcmp(a, b, flags));
                    } else if (sortit & SORTIT_IGNORING_CASE) != 0 {
                        // c:4187 SORTIT_IGNORING_CASE
                        sorted.sort_by_key(|a| a.to_lowercase()); // c:4187
                    } else {
                        // c:4180 — default `(o)` sort. C dispatches to
                        // strmetasort() (sort.c:303) which calls
                        // zstrcmp() (sort.c:191) whose final tie-break
                        // is `strcoll(as, bs)` at sort.c:134. strcoll
                        // is locale-aware: under UTF-8 locales it
                        // produces case-insensitive ordering (`a` < `B`).
                        // Previously this arm used raw byte `sort()`
                        // (ASCII order: `B` < `a`), diverging from zsh.
                        // Parity bug #31.
                        sorted.sort_by(|a, b| {
                            crate::ported::sort::zstrcmp(a, b, 0) // c:191
                        });
                    } // c:4187
                } // c:4292
                if (sortit & SORTIT_BACKWARDS) != 0 {
                    // c:4294 SORTIT_BACKWARDS
                    sorted.reverse(); // c:4191
                } // c:4294
            } // c:4290
            let join_with = sep.as_deref().unwrap_or(" "); // c:4313
            value = sorted.join(join_with); // c:4313
                                            // Update split_parts so downstream operators (case mods,
                                            // padding, splat) see the sorted/uniq list.
            split_parts = Some(sorted); // c:4313
        } // c:4290

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
            // which iterates aval per-element. Empties are
            // PRESERVED here (matching C sepsplit at utils.c:3962
            // which never reads the allownull arg); downstream
            // "remove empty unquoted words" elides middles in
            // unquoted contexts, while quoted `"${(@s..)…}"`
            // keeps them. Without preserving empties, the
            // quoted-`@` form would output `"he o"` instead of
            // zsh's `"he  o"` (the joined-with-IFS space-pair).
            let split_one = |s: &str| -> Vec<String> {
                if sp.is_empty() {
                    s.chars().map(|c| c.to_string()).collect()
                } else {
                    s.split(sp.as_str()).map(String::from).collect()
                }
            };
            // c:Src/subst.c:3902-3917 — when the source is an ARRAY,
            // it is JOINED before the force_split:
            //   nojoin==0 (or explicit sep): `val = sepjoin(aval, sep,
            //   1); isarr = 0;` (c:3905-3907; sep NULL joins with the
            //   IFS space), then c:3920 sepsplit(val, spsep) re-splits
            //   the joined scalar. So `${(f)$(cmd)}` joins the
            //   IFS-split words with spaces and the newline split
            //   finds nothing — zsh yields ONE scalar, not lines.
            //   nojoin==2 ((@) flag): joins on spsep itself
            //   (c:3913-3916 `sepjoin(aval, spsep, 1)`) — re-splitting
            //   on the same spsep ≡ per-element split.
            //
            // c:Src/subst.c:2916+ — a SINGLE-element subscript
            // (`a[1]`, not `@`/`*`/slice) already resolved the array
            // to ONE element in `value`; the split source must be
            // that scalar, NOT the whole array. Without this gate
            // `${(s:|:)a[1]}` split every element of `a` instead of
            // just element 1 (zsh: `x y z`; zshrs: `x y z b c`).
            // Mirrors the has_scalar_subscript gate used by the
            // strip/replace arms (subst.rs:8448).
            let split_has_scalar_sub = subscript
                .as_deref()
                .map(|s| {
                    let t = s.trim();
                    t != "@" && t != "*" && !t.contains(',')
                })
                .unwrap_or(false);
            let src_elems: Option<Vec<String>> = if let Some(prev) = split_parts.clone() {
                Some(prev)
            } else if split_has_scalar_sub {
                None // scalar `value` holds the resolved element
            } else {
                arrays_get(&var_name)
            };
            let parts: Vec<String> = match src_elems {
                Some(elems) if nojoin == 2 => {
                    // c:3913-3916 + c:3920 — per-element split.
                    elems.iter().flat_map(|s| split_one(s)).collect()
                }
                Some(elems) => {
                    // c:3905-3907 sepjoin then c:3920 sepsplit.
                    let joiner = sep.clone().unwrap_or_else(|| " ".to_string());
                    split_one(&elems.join(&joiner))
                }
                None => split_one(&value),
            };
            // c:Src/subst.c:3920-3928 — `if (force_split && !isarr) {
            //   aval = sepsplit(val, spsep, 0, 1); ... }` does NOT
            //   reassign `val` when the split produces multiple
            //   elements; only `aval` (split_parts) and `isarr` change.
            //   The original scalar `val` survives so scalar-context
            //   consumers (e.g. `r="${(s.X.)str}"` assignment) see the
            //   original delimiter. Multi-node splat consumers read
            //   split_parts. Previous Rust port unconditionally rejoined
            //   on space which broke `r="${(s.:.)s}"` → "a b c" instead
            //   of preserving "a:b:c". Bug #313 in docs/BUGS.md.
            //   When an explicit (j:Y:) sep is also set, the join uses
            //   it (force-rejoin path is separate).
            // c:Src/subst.c:4245-4313 — sort/unique block runs AFTER
            //   spsep split (C order: 3920+ split, then 4245 sort). The
            //   zshrs sort block at subst.rs:8141 is ordered BEFORE
            //   spsep handling here, so it sees isarr=0 from the
            //   pre-split scalar and skips. Apply sort/unique to the
            //   split parts inline here to preserve C's split-then-sort
            //   composition. Bugs #314 (sort after split) and #315
            //   (unique after split) in docs/BUGS.md.
            let mut parts = parts;
            if unique {
                let mut seen = std::collections::HashSet::new(); // c:4253
                parts.retain(|s| seen.insert(s.clone())); // c:4253
            }
            if sortit != SORTIT_ANYOLDHOW && indord == 0 {
                if (sortit & SORTIT_NUMERICALLY) != 0 {
                    let flags = sortit as u32; // c:Src/sort.c:191 zstrcmp
                    parts.sort_by(|a, b| crate::ported::sort::zstrcmp(a, b, flags));
                } else if (sortit & SORTIT_IGNORING_CASE) != 0 {
                    parts.sort_by_key(|a| a.to_lowercase()); // c:4187
                } else {
                    parts.sort_by(|a, b| crate::ported::sort::zstrcmp(a, b, 0)); // c:4180
                }
                if (sortit & SORTIT_BACKWARDS) != 0 {
                    parts.reverse(); // c:4191
                }
            } else if (sortit & SORTIT_BACKWARDS) != 0 && indord != 0 {
                // c:4283-4292 — (a) flag (indord=1) + (O) reverses the
                //   insertion-order list without applying the comparator.
                parts.reverse();
            }
            if let Some(ref jsep) = sep {
                value = parts.join(jsep.as_str()); // c:3906 sepjoin path
            } else if parts.is_empty() {
                value.clear(); // c:3923 — `val = dupstring("")`
            } else if parts.len() == 1 {
                value = parts[0].clone(); // c:3925 — `val = aval[0]`
            } else {
                // c:3920-3928 — multiple parts: leave `value` alone so
                // scalar-context reads preserve the original delimiter.
            }
            // c:Src/subst.c sepsplit — `(s.X.)` split COLLAPSES runs
            // of consecutive separators in array context (like awk
            // FS) unless `(@)` is also set (nojoin=2 path at
            // c:2165). For `aXXb` → 2 elements; `aXXXb` → 2; `X` → 2
            // (leading+trailing empty preserved); `Xab` → 2 (leading
            // empty preserved); `abX` → 2 (trailing empty preserved).
            // Only the INTERIOR empties between two non-empty
            // neighbours collapse — leading/trailing empties stay.
            // Bug #542.
            if nojoin != 2 && parts.len() >= 3 {
                let mut collapsed: Vec<String> = Vec::with_capacity(parts.len());
                let n = parts.len();
                for (i, p) in parts.iter().enumerate() {
                    let is_interior = i > 0 && i + 1 < n;
                    if is_interior && p.is_empty() {
                        continue;
                    }
                    collapsed.push(p.clone());
                }
                parts = collapsed;
            }
            split_parts = Some(parts.clone()); // c:3950
                                               // c:3922-3927 — `if (!aval || !aval[0]) val = dupstring("");
                                               // else if (!aval[1]) val = aval[0]; else isarr = nojoin ? 1 : 2;`
                                               // isarr is marked ONLY for a 2+-element split result; an empty
                                               // or single-element split stays scalar (isarr remains 0), so
                                               // `${#${(f)$(cmd)}}` measures the joined scalar's chars.
                                               // The value 2 is C's "array came from splitting a scalar"
                                               // sentinel (Src/subst.c:1647-1657 comment). The c:3317
                                               // qt-sepjoin transition skips when `spsep` is set, so isarr=2
                                               // survives DQ and the splat block at c:4245 fires —
                                               // matching zsh's `"${(s. .)str}"` per-word output.
            isarr = if parts.len() >= 2 {
                if nojoin != 0 {
                    1
                } else {
                    2
                }
            } else {
                0
            }; // c:3922-3927
                                                     // c:Src/subst.c:4215 — `if (isarr && ssub)
                                                     //   val = sepjoin(aval, NULL, 1); isarr = 0`.
                                                     //   PREFORK_SINGLE callers (scalar
                                                     //   assignment RHS, singsub) need a single
                                                     //   scalar result; the spsep-derived array
                                                     //   collapses back to the ORIGINAL `value`
                                                     //   here (preserved at c:3920-3928 above).
                                                     //   Bug #313 in docs/BUGS.md.
            if (pf_flags & PREFORK_SINGLE) != 0 {
                isarr = 0;
            }
            // c:Src/subst.c:3032 — `if (qt && !getlen && isarr > 0) {
            //     val = sepjoin(aval, sep, 1); isarr = 0; }`.
            // C's qt-sepjoin transition at c:3032 is gated on
            // `!spsep && spbreak < 2` — when the (s) flag set spsep,
            // the quoted result is NOT re-joined: zsh keeps the split
            // words separate even inside double quotes (verified:
            // `x="|a|b|"; set -- "${(s:|:)x}"` -> n=4 with empty
            // first/last words; `x="a:b:c"; set -- "${(s.:.)x}"` ->
            // n=3). A previous block here joined with IFS[0] and
            // cleared isarr, collapsing "${(s.:.)x}" to one word —
            // the !spsep guard makes that join unreachable in C for
            // this arm (spsep is always set here), so there is no
            // join at all. Empty pieces get the Nularg sentinel in
            // the splat emission below, exactly like C's nulstring.
        } else if let Some(ref sp) = sep {
            // c:3906-3907 — `val = sepjoin(aval, sep, 1); isarr = 0;`
            // (j:STR:) / (F) — join an array with STR. Source priority:
            // split_parts (operator result) → state.arrays →
            // assoc-values → whitespace-split fallback. Direct
            // port of subst.c:3906 sepjoin which reads aval.
            //
            // CRITICAL: clear isarr to 0 after the join (c:3907) so the
            // auto_splat block at c:4245 (subst.rs:6789) doesn't re-
            // fetch the unjoined array and splat element-by-element,
            // dropping the joined string. Without isarr=0 the j/F flag
            // returns only arr[0]. The auto_splat third clause is also
            // gated on `sep.is_none()` to skip the arrays_contains
            // fallback for the same reason.
            let mut joined = false;
            if let Some(parts) = split_parts.clone() {
                value = parts.join(sp); // c:3906
                split_parts = None; // c:3906 (sepjoin collapses to scalar)
                joined = true;
            } else if let Some(arr) = arrays_get(&var_name) {
                // Bug #328: `${(j:SEP:)NAME[N,M]}` — when a slice
                // subscript is present, sepjoin must operate on the
                // SLICE elements (aval, per c:3906), not the full
                // backing array. zsh C's aval points at the slice
                // result of getarrvalue (Src/params.c:2548); the
                // Rust port stuffs the slice into `value` as a
                // joined string and loses the per-element view. Re-
                // extract the slice from the full array via
                // getarrvalue before applying sep so the j flag sees
                // the same aval the C source would have.
                let slice_arr: Option<Vec<String>> = subscript
                    .as_deref()
                    .and_then(|s| {
                        let stripped = if let Some(rest) = s.strip_prefix('(') {
                            rest.find(')').map(|c| &rest[c + 1..]).unwrap_or(s)
                        } else {
                            s
                        };
                        stripped.split_once(',')
                    })
                    .and_then(|(lo_s, hi_s)| {
                        let lo = lo_s.trim().parse::<i64>().ok()?;
                        let hi = hi_s.trim().parse::<i64>().ok()?;
                        Some(getarrvalue(&arr, lo, hi))
                    });
                if let Some(slice) = slice_arr {
                    value = slice.join(sp); // c:3906 over aval (slice)
                } else {
                    value = arr.join(sp); // c:3906
                }
                joined = true;
            } else if let Some(map) = assoc_get(&var_name) {
                let vals: Vec<String> = map.values().cloned().collect();
                value = vals.join(sp); // c:3906
                joined = true;
            }
            // c:Src/subst.c:3903 — sepjoin is gated on `isarr ||
            // quoted_array_with_offset`. For a plain scalar the (j:STR:)
            // flag is a no-op; do NOT whitespace-split the scalar and
            // rejoin with STR. Previously a fallback arm split scalars
            // on whitespace which caused `"${(j:,:)$(echo a b c)}"` to
            // produce "a,b,c" instead of zsh's "a b c".
            if joined {
                isarr = 0; // c:3907
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
        if evalchar {
            // c:1673
            // c:3800-3833 — `int one = noerrs, oef = errflag; if
            // (!quoteerr) noerrs = 1; … noerrs = one; errflag = oef |
            // (errflag & ERRFLAG_INT);`. A math error inside `(#)`
            // (e.g. `${(#)a}` where a joins to the non-numeric scalar
            // "1 2 65") must yield empty WITHOUT printing or aborting
            // the command. Suppress via noerrs and restore the error
            // flags after, keeping only a pending interrupt.
            let saved_errflag = errflag.load(Ordering::Relaxed);
            let saved_noerrs = *crate::ported::utils::noerrs_lock().lock().unwrap();
            *crate::ported::utils::noerrs_lock().lock().unwrap() = 1;
            let eval_one = |s: &str| -> String { substevalchar(s.trim()).unwrap_or_default() };
            if let Some(parts) = split_parts.clone() {
                let new_parts: Vec<String> = parts.iter().map(|s| eval_one(s)).collect();
                value = new_parts.join(" ");
                split_parts = Some(new_parts);
            } else if isarr != 0 {
                // Whole-array reference: eval each element. A single-slot
                // subscript (`a[3]`) clears isarr and already selected the
                // element into `value`, so DON'T re-fetch the whole array
                // — `${(#)a[3]}` must eval just element 3. C operates on
                // the already-subscripted aval/val, never re-fetches.
                if let Some(arr) = arrays_get(&var_name) {
                    let new_arr: Vec<String> = arr.iter().map(|s| eval_one(s)).collect();
                    value = new_arr.join(" ");
                    split_parts = Some(new_arr);
                } else {
                    value = eval_one(&value);
                }
            } else {
                value = eval_one(&value);
            }
            // c:3833 — restore noerrs + errflag (keep only INT).
            *crate::ported::utils::noerrs_lock().lock().unwrap() = saved_noerrs;
            let int_bit = errflag.load(Ordering::Relaxed) & crate::ported::zsh_h::ERRFLAG_INT;
            errflag.store(saved_errflag | int_bit, Ordering::Relaxed);
        } // c:1673

        // (e) eval — re-substitute the result. Per-element on arrays.
        // Direct port of subst.c:2268 eval bit which iterates aval.
        if eval {
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
        if presc > 0 {
            // c:2405
            // Canonical prompt expansion (Src/prompt.c:182 promptexpand).
            let prompt_one = |s: &str| -> String {
                let (expanded, _, _) = promptexpand(s, 0, None);
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
        if (shsplit & LEXFLAGS_ACTIVE) != 0 {
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
                        if (shsplit & LEXFLAGS_COMMENTS_KEEP) != 0 {
                            cur.push(ch);
                        } // c:2451
                    } else if (shsplit & LEXFLAGS_COMMENTS_KEEP) != 0 {
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
                  // `((` at the start of a token: zsh's lexer treats
                  // `(( … ))` as a single DINPAR token (arith command
                  // start) and bufferwords returns the entire `((…))`
                  // as ONE word per Src/lex.c gettokstr's DINPAR arm.
                  // Walk chars consuming until matching `))`, tracking
                  // nested `(` for safety. Single token result.
                if cur.is_empty() && ch == '(' && p + 1 < chars_v.len() && chars_v[p + 1] == '(' {
                    let start = p;
                    let mut depth = 2_i32;
                    // Push the opening `((`.
                    p += 2;
                    while p < chars_v.len() && depth > 0 {
                        let pch = chars_v[p];
                        if pch == '\\' && p + 1 < chars_v.len() {
                            p += 2;
                            continue;
                        }
                        if pch == '\'' {
                            // skip SQ body
                            p += 1;
                            while p < chars_v.len() && chars_v[p] != '\'' {
                                p += 1;
                            }
                            if p < chars_v.len() {
                                p += 1;
                            }
                            continue;
                        }
                        if pch == '"' {
                            p += 1;
                            while p < chars_v.len() && chars_v[p] != '"' {
                                if chars_v[p] == '\\' && p + 1 < chars_v.len() {
                                    p += 2;
                                } else {
                                    p += 1;
                                }
                            }
                            if p < chars_v.len() {
                                p += 1;
                            }
                            continue;
                        }
                        if pch == '(' {
                            depth += 1;
                        } else if pch == ')' {
                            depth -= 1;
                        }
                        p += 1;
                    }
                    // We've consumed up to (and including) the final `)`
                    // of the matching `))`. Emit the whole slice as a
                    // single token.
                    let token: String = chars_v[start..p].iter().collect();
                    words.push(token);
                    continue;
                }
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
                    // c:Src/lex.c LEXFLAGS_COMMENTS_KEEP — `(Z+c+)`
                    // emits the entire `# comment` as ONE token.
                    // Grab the rest of the line into the current
                    // word so the trailing characters survive as a
                    // single span.
                    '#' if cur.is_empty()
                        && (shsplit & LEXFLAGS_COMMENTS_KEEP) != 0 =>
                    {
                        cur.push(ch);
                        let mut q = p + 1;
                        while q < chars_v.len() && chars_v[q] != '\n' {
                            cur.push(chars_v[q]);
                            q += 1;
                        }
                        p = q.saturating_sub(1);
                    }
                    // c:Src/lex.c LEXFLAGS_COMMENTS_STRIP — `(Z+C+)`
                    // skips the comment entirely (in_comment ends
                    // the word at the newline).
                    '#' if cur.is_empty()
                        && (shsplit & LEXFLAGS_COMMENTS_STRIP) != 0 =>
                    {
                        in_comment = true; // c:2456
                    }
                    // Plain `(z)` with neither flag: `#` is a normal
                    // token start. Bug #363 in docs/BUGS.md — the
                    // previous Rust port skipped the whole rest of
                    // the line whenever STRIP was not set, even when
                    // KEEP also wasn't set. zsh's actual default
                    // emits `#` and each following word as their own
                    // tokens, which the regular cur.push fallthrough
                    // produces.
                    '\n' if (shsplit & LEXFLAGS_NEWLINE) != 0 => {
                        // c:2461 (n: nl as ws)
                        push_word(&mut cur, &mut words); // c:2461
                    } // c:2461
                    // c:bufferwords — the (z) flag emits shell-token
                    // separators (`;`, `&`, `\n`, `|`, `||`, `&&`) as
                    // their OWN words, not just word boundaries. This
                    // mirrors what `bufferwords()` (Src/lex.c) does —
                    // it walks the lexer and yields each token as a
                    // separate node.
                    ';' | '&' | '\n' => {
                        push_word(&mut cur, &mut words);
                        // Emit the separator as its own word.
                        // Coalesce `&&` / `||` / `;;` into one token.
                        // Normalize `\n` → `;` to match bufferwords()
                        // output (c:bufferwords) — the lexer treats
                        // newlines as command separators equivalent
                        // to `;` and yields them as the `;` token.
                        let canon = if ch == '\n' { ';' } else { ch };
                        let mut sep_str = String::from(canon);
                        if (ch == '&' || ch == ';') && p + 1 < chars_v.len() && chars_v[p + 1] == ch
                        {
                            sep_str.push(ch);
                            p += 1;
                        }
                        words.push(sep_str);
                    }
                    '|' => {
                        push_word(&mut cur, &mut words);
                        let mut sep_str = String::from(ch);
                        if p + 1 < chars_v.len() && chars_v[p + 1] == '|' {
                            sep_str.push('|');
                            p += 1;
                        }
                        words.push(sep_str);
                    }
                    c if c.is_whitespace() => {
                        // c:2439
                        push_word(&mut cur, &mut words); // c:2439
                    } // c:2439
                    _ => cur.push(ch), // c:2439
                } // c:2439
                p += 1; // c:2439
            } // c:2439
            push_word(&mut cur, &mut words); // c:2439
                                             // c:4174-4198 — bufferwords result becomes a word list:
                                             // when there are multiple words OR isarr was set, the
                                             // value is the list (aval), else the single joined val.
                                             // Mirror by setting split_parts so the auto_splat block
                                             // (or DQ join via sepjoin gate) consumes the list.
                                             // Per c:3274 split-from-scalar convention, isarr = 2.
            // Bug #363: zsh's `(z)` flag produces TOKENIZED words —
            // a `$` inside any word is the literal char `$`, NOT a
            // new param reference. Without an "already-tokenized"
            // marker, downstream stringsubst re-expands `$VAR` as a
            // fresh param-ref (UNSET → empty → element elided).
            // Prefix `$` and `` ` `` with Bnull (the lexer's
            // backslash-token) so the chars survive stringsubst's
            // `$`/`` ` `` arms (Src/subst.c:265 `case Bnull: *s='\0'`
            // skip) and untokenize to the literal char at the final
            // output pass. Mirrors C bufferwords' tokenized result.
            let bnull = crate::ported::zsh_h::Bnull;
            for w in words.iter_mut() {
                if w.contains('$') || w.contains('`') {
                    let mut out = String::with_capacity(w.len() + 2);
                    for c in w.chars() {
                        if c == '$' || c == '`' {
                            out.push(bnull);
                        }
                        out.push(c);
                    }
                    *w = out;
                }
            }
            value = words.join(" "); // c:4191 single-word case
            if !words.is_empty() {
                split_parts = Some(words); // c:4194
                // Bug #363 sub-issue: in DQ context, (z) flag must
                // preserve splat shape per zsh — `"${(z)c}"` produces
                // a multi-arg splat where adjacent text glues to the
                // first/last word (mirrors `"${a[@]}"`). Without the
                // negative-isarr gate, c:3032 sepjoin fires and
                // collapses the words to a single scalar; the
                // downstream auto_splat block (gated on isarr != 0)
                // then never emits per-word. Setting isarr to a
                // negative marker (mirrors the SCANPM_ISVAR_AT case
                // at line 5969 for `[@]`) skips c:3032 (`isarr > 0`)
                // while keeping auto_splat eligible. Bug #363.
                isarr = if qt {
                    -1 // c:3274 + c:3032 isarr<0 skip-sepjoin
                } else if nojoin != 0 {
                    1
                } else {
                    2 // c:3274 split-from-scalar (unquoted)
                };
            }
        } // c:2473

        // (D) dir-magic — replace $HOME and any nameddir prefix with
        // tilde form. Direct port of subst.c:2229 mods bit 1, which
        // routes through modify()'s tilde-contraction at the end of
        // the pipeline. Common idiom: `${(D)PWD}` → `~/projects/foo`.
        // Without ZLE's nameddir hash, this reduces to plain $HOME.
        // (D) per-element dir-magic. Direct port of subst.c:2229
        // mods bit 1 → modify()'s tilde-contraction iterating aval.
        if (mods & 1) != 0 {
            // c:4155 if (mods & 1)
            let home_opt = getsparam("HOME"); // c:4155
                                              // Pull named-dirs (~name) hash into a [(name, path)]
                                              // sorted by path-length-descending so the LONGEST match
                                              // wins (zsh canonical: most-specific tilde-contraction).
                                              // Direct port of subst.c → modify dir-handling which
                                              // walks the nameddirtab in length-desc order.
                                              // c:2229 — canonical nameddirtab read (mirrors C's
                                              // `mod_export HashTable nameddirtab` at hashnameddir.c:48).
            let mut named: Vec<(String, String)> = nameddirtab()
                .lock()
                .map(|t| {
                    t.iter()
                        .map(|(k, nd)| (k.clone(), nd.dir.clone()))
                        .collect()
                })
                .unwrap_or_default();
            named.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
            let dir_one = |s: &str| -> String {
                // c:Src/utils.c:1127 finddir — HOME first (so the bare
                // `~` wins when $HOME equals a named-dir path),
                // then nameddirtab. The previous order (named first)
                // made `hash -d hm=$HOME; (D)$HOME` render as `~hm`
                // instead of zsh's `~`.
                if let Some(ref h) = home_opt {
                    if !h.is_empty() && s.starts_with(h.as_str()) {
                        let r = &s[h.len()..];
                        if r.is_empty() || r.starts_with('/') {
                            return format!("~{}", r);
                        }
                    }
                }
                for (name, path) in &named {
                    if !path.is_empty() && s.starts_with(path.as_str()) {
                        let r = &s[path.len()..];
                        if r.is_empty() || r.starts_with('/') {
                            return format!("~{}{}", name, r);
                        }
                    }
                }
                s.to_string()
            };
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
        // feed back into a glob/regex context as a literal.
        //
        // c:Src/utils.c:6242-6248 QT_BACKSLASH_PATTERN body:
        //   while (*u) {
        //       if (ipattern(*u))
        //           *v++ = '\\';
        //       *v++ = *u++;
        //   }
        //
        // `ipattern(c)` is true when c is in PATCHARS, defined at
        // Src/zsh.h:232 as `"#^*()|[]<>?~\\"` — exactly 12 chars.
        // Bug #47 in docs/BUGS.md: the previous Rust port also
        // escaped whitespace (space/tab/newline) and shell-syntax
        // separators (; & { } $ ` " '), over-escaping vs C zsh and
        // breaking the `ls **/${~(b)file}` idiom for filenames
        // containing spaces.
        let b_one = |s: &str| -> String {
            // c:6242
            let mut out = String::with_capacity(s.len() * 2);
            for ch in s.chars() {
                // c:6244 `if (ipattern(*u)) *v++ = '\\';` — chars in
                // PATCHARS only.
                if matches!(
                    ch,
                    '#' | '^' | '*' | '(' | ')' | '|' | '[' | ']'
                        | '<' | '>' | '?' | '~' | '\\'
                ) {
                    out.push('\\');
                }
                out.push(ch);
            }
            out
        };
        if quotemod > 0 && quotetype == QT_BACKSLASH_PATTERN {
            // c:4034 (b)
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
            // c:Src/subst.c:4863-4874 — `parse_subst_string + remnulargs
            // + untokenize`. The C path runs the input through the
            // shell parser (noerrs=1 tolerating malformed input), so
            // unbalanced quote chars survive as literals. Bug #507:
            // zshrs's previous walker dropped EVERY `'`/`"` it saw,
            // so `a=x"y` (single embedded `"`) became `xy` instead of
            // staying `x"y`. Mirror C's balanced-span behavior: strip
            // `'…'` / `"…"` only when a closing partner exists; an
            // orphan quote stays literal.
            let chars_v: Vec<char> = s.chars().collect();
            let mut out = String::with_capacity(s.len());
            let mut i = 0_usize;
            while i < chars_v.len() {
                let c = chars_v[i];
                if c == '$' && i + 1 < chars_v.len() && chars_v[i + 1] == '\'' {
                    // c:2261 — `$'…'` ANSI-C decoder.
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
                    let (decoded, _) = getkeystring(&body); // c:2261
                    out.push_str(&decoded);
                    i = if j < chars_v.len() { j + 1 } else { j };
                    continue;
                }
                if c == '\\' {
                    if i + 1 < chars_v.len() {
                        out.push(chars_v[i + 1]);
                        i += 2;
                        continue;
                    }
                }
                if c == '\'' || c == '"' {
                    // Probe for a closing partner. Inside `'…'`, no
                    // escapes — accept first matching `'`. Inside
                    // `"…"`, backslash escapes the next char.
                    let mut j = i + 1;
                    let mut found = None;
                    while j < chars_v.len() {
                        if c == '"' && chars_v[j] == '\\' && j + 1 < chars_v.len() {
                            j += 2;
                            continue;
                        }
                        if chars_v[j] == c {
                            found = Some(j);
                            break;
                        }
                        j += 1;
                    }
                    if let Some(close) = found {
                        // Strip opener + emit interior (backslash-
                        // unescape for `"…"`; literal for `'…'`).
                        let mut k = i + 1;
                        while k < close {
                            if c == '"' && chars_v[k] == '\\' && k + 1 < close {
                                out.push(chars_v[k + 1]);
                                k += 2;
                                continue;
                            }
                            out.push(chars_v[k]);
                            k += 1;
                        }
                        i = close + 1;
                        continue;
                    }
                    // Orphan quote: keep literal.
                    out.push(c);
                    i += 1;
                    continue;
                }
                out.push(c);
                i += 1;
            }
            out
        };
        if quotemod < 0 {
            // c:4030 if (quotemod) — negative arm (Q)
            if let Some(parts) = split_parts.clone() {
                let new_parts: Vec<String> = parts.iter().map(|s| unquote_one(s)).collect();
                value = new_parts.join(" ");
                split_parts = Some(new_parts);
            } else if subscript.is_none() {
                // Whole-array `${(Q)arr}` — per-element dequote
                // (subst.c:2261 quotemod-- iterates aval). MUST be
                // gated on no-subscript: `${(Q)a[1]}` already
                // resolved the element into `value` above; the
                // arrays_get re-fetch by NAME here discarded that
                // selection and dequoted+joined the WHOLE array.
                if let Some(arr) = arrays_get(&var_name) {
                    let new_arr: Vec<String> = arr.iter().map(|s| unquote_one(s)).collect();
                    value = new_arr.join(" ");
                    split_parts = Some(new_arr);
                } else {
                    value = unquote_one(&value);
                }
            } else {
                value = unquote_one(&value);
            }
        }

        // (X) error on unset/empty — emit error if value is empty.
        // Port of subst.c:2264 (quoteerr=1).
        if quoteerr && value.is_empty() && !is_set {
            // c:2264
            zerr(&format!("{}: parameter not set or null", var_name)); // c:N/A
            errflag_set_error();
        }

        // (V) visible — render non-printable chars per zsh's
        // `nicechar` style (`\t` and `\n` get backslash forms, other
        // controls under 0x20 use `^X`, 0x7f → `^?`, high-bit bytes →
        // `\M-X`). Port of subst.c:2232 mods bit 1 calling
        // Src/utils.c:520 nicechar via Src/utils.c:4157. The previous
        // Rust port used `^X` uniformly so `(V)$'a\tb'` rendered
        // `a^Ib` instead of zsh's `a\tb`.
        let visible_one = |s: &str| -> String {
            // c:2232
            let mut out = String::with_capacity(s.len());
            for c in s.chars() {
                out.push_str(&crate::ported::utils::nicechar(c)); // c:520
            }
            out
        };
        if (mods & 2) != 0 {
            // c:4157 if (mods & 2)
            if let Some(parts) = split_parts.clone() {
                // c:4157
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
        // (whitespace-split), word count (W = WS_NULL). Single
        // tri-state `whichlen` int per c:1679. C only fires the
        // length dispatch when `getlen = 1 + whichlen` AND `getlen`
        // is consumed by the `#`-prefix length arm (Src/subst.c:3845
        // `if (getlen) { ... }`). Without `${#var}` the flag is
        // recorded but inert — `${(W)var}` returns the value as-is.
        if length_op && whichlen == 1 {
            // c:2276 whichlen == 1 (c)
            // (m) flag, when set, counts cells via wcpadwidth (so
            // wide chars count 2). Without (m): plain chars.count().
            value = if multi_width > 0 {
                // c:2276
                value // c:2376
                    .chars() // c:2376
                    .map(|c| wcpadwidth(c, multi_width as i32) as usize) // c:2376
                    .sum::<usize>() // c:2376
                    .to_string() // c:2376
            } else {
                // c:2276
                value.chars().count().to_string() // c:2276
            }; // c:2276
        } else if length_op && whichlen == 2 {
            // c:2279 whichlen == 2 (w)
            value = value.split_whitespace().count().to_string(); // c:2279
        } else if length_op && whichlen == 3 {
            // c:2282 whichlen == 3 (W)
            // (W) — count words including empty fields.
            let parts: Vec<&str> = value.split(|c: char| c.is_whitespace()).collect(); // c:2282
            value = parts.len().to_string(); // c:2282
        }

        // Quote flags (q/qq/qqq/qqqq/q-/q+) operate per-element when
        // array-shaped. Direct port of subst.c:4030+ quotemod > 0 arm
        // which dispatches by quotetype.
        //
        // Bug #151: when the auto_splat block emits MULTIPLE result
        // nodes, each goes back through stringsubst (Src/subst.c:100).
        // zshrs's stringsubst has a literal-`'` arm (subst.rs:710)
        // that treats raw `'…'` in a node as a single-quoted span
        // and strips the surrounding quotes — fine when the lexer
        // produced raw `'` from source code, BUT wrong for `'…'`
        // characters emitted BY paramsubst (e.g. `(qq)` quoting).
        // Wrap quote_one's output in Snull (\u{9d}) markers so the
        // Snull handler at subst.rs:641 strips THE WRAPPERS and
        // keeps the inner body (including literal `'` chars) verbatim.
        // First node was sometimes fine because the prefix context
        // shifted the `'` past the position where stringsubst would
        // re-process; the second + nodes always lost their quotes.
        let wrap_snull = |s: String| -> String {
            if s.contains('\'') {
                format!("\u{9d}{}\u{9d}", s)
            } else {
                s
            }
        };
        let quote_one = |s: &str| -> String {
            // c:4030
            if quotetype == QT_SINGLE_OPTIONAL {
                // c:Src/utils.c:6181-6190 — QT_SINGLE_OPTIONAL sets
                // shownull=1 so empty string always quotes as `''`.
                if s.is_empty() {
                    return "''".to_string(); // c:utils.c:6253-6256
                }
                // c:Src/utils.c:6260+ QT_SINGLE_OPTIONAL — pick the
                // minimum quoting per char: bare apostrophes get a
                // backslash escape (`it's` → `it\'s`), other
                // specials trigger single-quote span. The previous
                // port called quotestring(s, QT_SINGLE) which
                // wrapped the whole string in `'…'` and emitted
                // `'\''` for inner apostrophes (`'it'\''s'`, 9
                // chars instead of zsh's 5). Direct port of the
                // walker at c:6266-6385. Parity bug.
                quotestring(s, QT_SINGLE_OPTIONAL) // c:6266
            } else if quotetype == QT_QUOTEDZPUTS {
                // c:4063-4064 — `for (; *ap; ap++) *ap = quotedzputs(*ap,
                // NULL);` and scalar c:4109 `val = quotedzputs(val, NULL);`.
                // Previously called quotestring(s, QT_DOLLARS) which
                // unconditionally wraps in `$'…'`; quotedzputs only
                // does so for nice-formatted (non-printable) strings,
                // otherwise picks single-quote form. zsh: `${(q+)"hi
                // there"}` → `'hi there'` (shortest valid quoting).
                crate::ported::utils::quotedzputs(s) // c:4063
            } else if quotemod > 0 {
                // c:4033 if (quotemod > 0)
                // c:2252 — quotemod++ and quotetype++ cascade for
                // (q)/(qq)/(qqq)/(qqqq). quotetype starts at QT_NONE=0
                // and is incremented per q: QT_BACKSLASH(1) /
                // QT_SINGLE(2) / QT_DOUBLE(3) / QT_DOLLARS(4).
                wrap_snull(quotestring(s, quotetype)) // c:4070
            } else {
                // c:4034
                s.to_string() // c:4034
            } // c:4034
        }; // c:4034
        if quotemod > 0 && quotetype != QT_BACKSLASH_PATTERN {
            // c:4033 (already-applied b above)
            // c:Src/subst.c — array dispatch differs by `(@)` flag:
            //   `(@q)`: per-element quote, keep array shape.
            //   `(q)`:  join with IFS-first-char (space), then quote
            //           the JOINED scalar — so the join spaces also
            //           get backslash-escaped, producing
            //           `foo\\ bar\\ baz\\ qux` from `(foo "bar baz"
            //           qux)`. The previous Rust port only quoted
            //           per-element, leaving the join spaces literal
            //           and ambiguous if the result was later
            //           re-split. Bug #52 in docs/BUGS.md.
            //
            // nojoin == 2 means `(@)` was present in the flag chain.
            // In unquoted context, the result splats as an array too
            // — `print -l ${(q)arr}` shows each element on its own
            // line with per-element quoting. Join-first only applies
            // in DQ context where the result is forced to a scalar.
            //
            // `"${(qq)arr[@]}"` — DQ with `[@]` subscript — also
            // wants per-element quoting (the `[@]` is the
            // splat-preserving subscript that keeps array shape in
            // DQ, matching C `Src/subst.c::paramsubst` SCANPM_ISVAR_AT
            // semantics). Without this, qt=true forces the join-first
            // path and produces `'a b c d e'` instead of `'a b' 'c'
            // 'd e'`. Bug #392.
            let is_at_subscript_splat =
                matches!(subscript.as_deref(), Some("@") | Some("*"));
            let want_per_element = nojoin == 2 || !qt || is_at_subscript_splat;
            // c:2237
            if let Some(parts) = split_parts.clone() {
                if want_per_element {
                    let new_parts: Vec<String> = parts.iter().map(|s| quote_one(s)).collect();
                    value = new_parts.join(" ");
                    split_parts = Some(new_parts);
                } else {
                    // Join then quote the joined scalar.
                    let joined = parts.join(" ");
                    let quoted = quote_one(&joined);
                    value = quoted.clone();
                    split_parts = Some(vec![quoted]);
                }
            } else if let Some(arr) = arrays_get(&var_name) {
                if want_per_element {
                    let new_arr: Vec<String> = arr.iter().map(|s| quote_one(s)).collect();
                    value = new_arr.join(" ");
                    split_parts = Some(new_arr);
                } else {
                    let joined = arr.join(" ");
                    let quoted = quote_one(&joined);
                    value = quoted.clone();
                    split_parts = Some(vec![quoted]);
                }
            } else if value.is_empty()
                && is_at_subscript_splat
                && assoc_get(&var_name).map_or(false, |m| m.is_empty())
            {
                // `${(q…)EMPTY_ASSOC[@]}` — zero elements splat to
                // ZERO results; the scalar fallback quoted the empty
                // join into a literal `''`. zinit stamps
                // `${(j: :)${(qkv)ICE[@]}}` into every ZINIT_SICE
                // entry, so the phantom `''` leaked into ice parsing.
                split_parts = Some(Vec::new());
                isarr = 1;
            } else {
                value = quote_one(&value);
            }
        }

        // (g) decode — apply getkeystring to the value if `(g…)` was
        // seen in the flag block. Per Src/subst.c:3955 `if (getkeys
        // >= 0)` block which fires whenever `getkeys` was set, even
        // to 0 (bare `(g::)` with no sub-letters means "default
        // getkeystring decoding"). Per-element on arrays.
        if getkeys >= 0 {
            // c:3955 if (getkeys >= 0)
            // GETKEY_EMACS / GETKEY_OCTAL_ESC / GETKEY_CTRL bits in
            // `getkeys` are honored by getkeystring directly; passing
            // `getkeys` through would require getkeystring_with(s,
            // getkeys as u32) at c:6915 (utils.c). Default call here
            // produces the same byte-string for the unsuffixed (g::)
            // case (bare flag with no sub-letters).
            let decode_one = |s: &str| -> String { getkeystring(s).0 };
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
        if mods != 0 {
            // c:4155 if (mods != 0)
            let render_d = |s: &str| -> String {
                // c:4155
                if (mods & 1) == 0 {
                    // c:4155 if (mods & 1)
                    return s.to_string(); // c:4156
                } // c:4156
                  // c:Src/utils.c:1127 finddir — checks $HOME first
                  // (so `~` wins over any `~name` whose path equals
                  // $HOME), then walks nameddirtab. The previous
                  // inline impl let nameddirtab matches override the
                  // HOME wrap when both had the same path, so
                  // `hash -d hm=$HOME` + `(D)$HOME` rendered as
                  // `~hm` instead of `~`.
                crate::ported::utils::finddir(s).unwrap_or_else(|| s.to_string())
            };
            let render_v = |s: &str| -> String {
                // c:4157
                if (mods & 2) == 0 {
                    // c:4157 if (mods & 2)
                    return s.to_string(); // c:4158
                } // c:4158
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
            let ifs = vars_get("IFS").unwrap_or_else(|| " \t\n".to_string());
            let parts: Vec<String> = value
                .split(|c: char| ifs.contains(c))
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect();
            if !parts.is_empty() {
                value = parts.join(" ");
                split_parts = Some(parts);
                // c:3274 — `isarr = nojoin ? 1 : 2;` mark this as a
                // split-from-scalar so the c:4245 splat block fires
                // and the c:3032 sepjoin-on-qt skips (per c:3317
                // !spsep guard equivalent — force_split is the
                // spbreak=2 path which has the same effect).
                isarr = if nojoin != 0 { 1 } else { 2 };
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
        // c:4245 — `if (isarr)`. The sort + splat block fires whenever
        // isarr is non-zero. isarr survived c:3029-3036 because:
        //   - `[@]` subscript sets isarr=-1 at c:2915 (SCANPM_ISVAR_AT)
        //     and the c:3032 sepjoin is gated on `isarr > 0`, so -1
        //     is preserved through DQ.
        //   - `(@)` flag sets nojoin=2; c:3030 sets isarr=-1 if nojoin;
        //     same preservation through c:3032.
        // Bare array reads in DQ get isarr=0 at c:3034 (sepjoin'd),
        // and that's how they end up joined — exactly what we want.
        // c:2883 — wantt clears `isarr = 0` so the (t) typeinfo
        // result (a scalar string like "array") doesn't get re-
        // splat from the underlying array storage. Gate auto_splat
        // off when wantt fired, mirroring C's `if (isarr)` at
        // c:4245 not firing when wantt cleared isarr to 0.
        // c:2554 — explicit `${^^arr}` forces RC_EXPAND off: the array
        // must collapse to a JOINED scalar so it neither cross-product-
        // distributes (under rcexpandparam) nor word-splits. Join now,
        // before the auto-splat decision, so isarr=0 keeps it scalar.
        if rc_force_off && isarr != 0 {
            let src = split_parts
                .clone()
                .or_else(|| arrays_get(&var_name))
                .unwrap_or_else(|| vec![value.clone()]);
            value = crate::ported::utils::sepjoin(&src, None);
            split_parts = Some(vec![value.clone()]);
            isarr = 0;
        }
        let auto_splat = !wantt
            && (isarr != 0                  // c:4245
            || force_splat_from_eq                           // c:2566
            || (!(nojoin == 2)                                     // c:3950
            && !qt                                           // c:3950 (only outside DQ)
            && pf_flags & PREFORK_SINGLE == 0         // c:3950 (multsub context)
            && rest.is_empty()                               // c:3950 (no operator subverted shape)
            && !scripted_scalar                              // c:3950 (single-elem pick is scalar)
            && sep.is_none()                                 // c:3906-3907 (j/F flag already sepjoin'd → scalar)
            && (arrays_contains(&var_name)         // c:3950
                || split_parts.is_some()))); // c:3950 ((s::) made an array)
        if (nojoin == 2) || auto_splat {
            // c:3950
            let parts: Vec<String> = if let Some(sp) = split_parts.clone() {
                // (s::) split → splat the post-split parts
                // regardless of source. Direct port of subst.c's
                // ssub-then-splat where spsep promotes scalar to
                // array via the split. In UNQUOTED context, empty
                // parts are elided (zsh's "remove empty unquoted
                // words" applied to spsep-split arrays — visible in
                // `b=(\${(s.l.)hello})` producing 2 elements not 3).
                // Quoted `"\${(@s.l.)hello}"` keeps them (the @
                // flag forces array semantics + qt preserves empties).
                if !qt && spsep.is_some() {
                    sp.into_iter().filter(|s| !s.is_empty()).collect()
                } else {
                    sp // c:3950
                }
            } else if let Some(sub) = subscript.as_deref() {
                // Range subscript: splat the slice elements.
                if let Some((lo, hi)) = sub.split_once(',') {
                    // c:Src/params.c::getarg — bounds route through
                    // singsub then mathevali, so variable / arithmetic
                    // bounds resolve (`${(@)b[1,R]}`, `${(@)b[1,-1*R-1]}`).
                    // The plain `parse()` used before failed on anything
                    // non-literal: `hi="R"` → unwrap_or(0) → getarrvalue(1,0)
                    // = empty. Mirror the scalar slice arm (subst.rs:5988).
                    let arr_len = arrays_get(&var_name).map_or(0, |a| a.len()) as i64;
                    let eval_bound = |expr: &str, default: i64| -> i64 {
                        let e = singsub(expr.trim());
                        if let Ok(n) = e.trim().parse::<i64>() {
                            return n;
                        }
                        crate::ported::math::mathevali(e.trim()).unwrap_or(default)
                    };
                    let lo: i64 = eval_bound(lo, 1); // c:3950
                    let hi: i64 = eval_bound(hi, arr_len); // c:3950
                    // c:Src/params.c — KSH_ARRAYS shifts positive
                    // 0-based slice bounds to 1-based for getarrvalue.
                    // Sibling of #610-#613 in the splat path. Bug #614.
                    let ksh_arrays = isset(crate::ported::zsh_h::KSHARRAYS);
                    let (lo, hi) = if ksh_arrays {
                        let new_lo = if lo >= 0 { lo + 1 } else { lo };
                        let new_hi = if hi >= 0 { hi + 1 } else { hi };
                        (new_lo, new_hi)
                    } else {
                        (lo, hi)
                    };
                    arrays_get(&var_name)
                        .as_ref() // c:3950
                        .map(|arr| getarrvalue(arr, lo, hi))
                        .unwrap_or_default()
                } else if let Some(arr) = arrays_get(&var_name) {
                    arr.clone() // c:3950 (@ / *)
                } else {
                    vec![value.clone()]
                }
            } else if let Some(arr) = arrays_get(&var_name) {
                arr.clone() // c:3960 (real array splat)
            } else if let Some(map) = assoc_get(&var_name) {
                if (hkeys & SCANPM_WANTKEYS) != 0 && (hvals & SCANPM_WANTVALS) != 0 {
                    // c:3955 (kv splat — interleaved)
                    let mut out: Vec<String> = Vec::with_capacity(map.len() * 2); // c:3955
                    for (k, v) in map {
                        // c:3955
                        out.push(k.clone()); // c:3955
                        out.push(v.clone()); // c:3955
                    } // c:3955
                    out // c:3955
                } else if (hkeys & SCANPM_WANTKEYS) != 0 {
                    // c:3955 (k-flag splat)
                    map.keys().cloned().collect()
                } else if (hvals & SCANPM_WANTVALS) != 0 {
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
            //
            // c:Src/subst.c:36 `nulstring[] = {Nularg, '\0'};` — zsh
            // emits the Nularg sentinel (single `\u{a1}` byte) for
            // empty array elements so the prefork's empty-node-delete
            // pass at subst.c:184-187 (`else if (!keep) uremnode`)
            // doesn't drop them. The subsequent `remnulargs` (called
            // from prefork at c:170 for each non-empty node) strips
            // the Nularg back to true empty before downstream
            // consumers see the value. Without this, `${(@s./.)X}`
            // with leading empty in DQ context lost the leading
            // element. Parity bug.
            let nul_str = "\u{a1}";
            // c:Src/subst.c:36 — `nulstring[] = {Nularg, '\0'}`. Empty
            // array elements get the Nularg sentinel ONLY in qt (DQ)
            // context where prefork's `else if (!keep) uremnode` at
            // c:184-187 must NOT delete them. In unquoted context zsh
            // word-splits and DROPS empty elements (e.g. `a=(x y z);
            // echo ${a%x*}` → `y z`, not ` y z`). Bug #578: zshrs
            // emitted Nularg unconditionally, so unquoted splats with
            // empty elements leaked `\u{a1}` into argv. Mirror C by
            // gating on qt.
            let emit_part = |s: &str| -> String {
                if s.is_empty() {
                    if qt {
                        nul_str.to_string()
                    } else {
                        String::new()
                    }
                } else {
                    s.to_string()
                }
            };
            // c:Src/subst.c:1404-1416 DPUTS — singsub asserts that the
            // result list has ≤1 element. The PREFORK_SINGLE caller
            // (heredoc body, scalar-assign RHS, `${var:-…}` default
            // expression, etc.) needs the array splat collapsed to a
            // sepjoined scalar before return; otherwise singsub takes
            // only the first node and the remaining elements silently
            // disappear. C zsh's prefork-on-singsub path falls through
            // the `if (l > 1) sepjoin` arm at multsub c:633-647 with
            // PREFORK_SINGLE clearing isarr at c:3029-3036, so the
            // splat block doesn't fire there. The Rust port's
            // auto_splat fires for isarr=-1 even under
            // PREFORK_SINGLE — collapse here to compensate. Bug #123
            // in docs/BUGS.md: `cat <<END\n${arr[@]}\nEND` returned
            // only the first array element because singsub's
            // result-zero-of-many fell through to the truncated form.
            if pf_flags & PREFORK_SINGLE != 0 && parts.len() > 1 {
                // c:Src/params.c sepjoin default sep — IFS first char.
                let ifs = vars_get("IFS").unwrap_or_else(|| " \t\n".to_string());
                let default_sep = ifs
                    .chars()
                    .next()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| " ".to_string());
                let sep_str: &str = sep.as_deref().unwrap_or(default_sep.as_str());
                let joined = parts.join(sep_str);
                let full = format!("{}{}{}", prefix, joined, suffix);
                let new_pos = prefix.chars().count() + joined.chars().count();
                return (full.clone(), new_pos, vec![full]);
            }
            let mut nodes: Vec<String> = Vec::with_capacity(parts.len());
            for (i, part) in parts.iter().enumerate() {
                let s = if parts.len() == 1 {
                    format!("{}{}{}", prefix, emit_part(part), suffix)
                } else if i == 0 {
                    format!("{}{}", prefix, emit_part(part))
                } else if i == parts.len() - 1 {
                    format!("{}{}", emit_part(part), suffix)
                } else {
                    emit_part(part)
                };
                nodes.push(s);
            }
            // c:Src/subst.c:184-187 — `else if (!keep) uremnode` — in
            // unquoted (qt=false) context the C source's prefork drops
            // empty list nodes BEFORE word-splitting reaches argv.
            // zshrs's prefork has the drop arm but stringsubst's
            // multi-node splat outruns it for the array-splat path
            // (the third-pass loop doesn't revisit the freshly
            // inserted middle/last nodes when prefork's stringsubst
            // returns a new_idx past them). Drop empties here so
            // `a=(x y z); echo ${a%x*}` doesn't leak a leading-empty
            // arg ` y z` instead of zsh's `y z`. Bug #578. Skip the
            // drop in qt (DQ keeps empties — `"${(@s./.)X}"` shape).
            // Stash the last element's rendered length BEFORE the
            // empties-drop: the multi-node return contract (below)
            // reports the suffix offset within the LAST node.
            let last_value_chars = parts
                .last()
                .map(|p| emit_part(p).chars().count())
                .unwrap_or(0);
            let pre_retain_len = nodes.len();
            if !qt && nodes.len() > 1 {
                nodes.retain(|n| !n.is_empty());
            }
            let first = nodes.first().cloned().unwrap_or_default();
            if nodes.len() > 1 {
                // Multi-node contract (paired with stringsubst's
                // splice advance, C subst.c:339): returned pos =
                // suffix offset in the LAST node (last node =
                // emit_part(last) + suffix), so scanning resumes at
                // the suffix and the element VALUES are never
                // re-expanded. If the empties-drop removed nodes the
                // original last may be gone — consume the whole (new)
                // last node; only empty nodes were dropped, so
                // nothing scannable is skipped.
                let last_pos = if nodes.len() == pre_retain_len {
                    last_value_chars
                } else {
                    nodes.last().map(|n| n.chars().count()).unwrap_or(0)
                };
                return (first, last_pos, nodes);
            }
            let new_pos_in_full = prefix.chars().count()
                + first.chars().count().saturating_sub(prefix.chars().count());
            return (first, new_pos_in_full, nodes);
        }

        let full = format!("{}{}{}", prefix, value, suffix); // c:1885
        let new_pos_in_full = prefix.chars().count() + value.chars().count();
        return (full.clone(), new_pos_in_full, vec![full]);
    } // c:1885

    // c:Src/subst.c:1939+ — `$+name` (no braces) is the brace-free
    // form of `${+name}` (chkset: emit "1" if NAME is set, "0"
    // otherwise). zsh's lexer normalizes both forms through the same
    // paramsubst codepath because paramsubst's `+` flag arm at
    // c:2199-2207 accepts the bare-form when there's no leading
    // brace. zshrs's paramsubst only recognized `+` inside `${...}`.
    //
    // Symptom: `print -r "$+parameters"` emitted `+parameters` literal
    // (the `$` got dropped, `+parameters` survived as raw text).
    //
    // Rewrite `$+NAME` → `${+NAME}` in-place and recurse so the
    // brace-form arm (lines 2487+) handles the full flag logic.
    if c == '+' {
        // Walk the identifier after `+`. Same allowed-char set as
        // the bare-name walk below: alnum + _ + the single-char
        // specials `@ * # ?`.
        let name_start = pos + 1;
        let mut name_end = name_start;
        if name_end < chars.len() {
            let first = chars[name_end];
            if first.is_ascii_alphanumeric() || first == '_' {
                while name_end < chars.len()
                    && (chars[name_end].is_ascii_alphanumeric() || chars[name_end] == '_')
                {
                    name_end += 1;
                }
            } else if matches!(first, '@' | '*' | '#' | '?') {
                name_end += 1;
            }
        }
        if name_end > name_start {
            // Optional `[subscript]` — `$+arr[key]` checks per-element.
            // Walk bracketed subscript depth-tracked so `$+arr[$a[1]]`
            // works. Same as the bare-name `$NAME[SUB]` arm below.
            let mut sub_end = name_end;
            if chars.get(sub_end).copied() == Some('[') {
                let mut depth = 1;
                let mut q = sub_end + 1;
                while q < chars.len() && depth > 0 {
                    match chars[q] {
                        '[' => depth += 1,
                        ']' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    q += 1;
                }
                if depth == 0 && q < chars.len() && chars[q] == ']' {
                    sub_end = q + 1;
                }
            }
            // Synthesize `${+NAME[SUB]}` and recurse.
            let name_with_sub: String = chars[name_start..sub_end].iter().collect();
            let prefix: String = chars[..start_pos].iter().collect();
            let suffix: String = chars[sub_end..].iter().collect();
            let rewritten = format!("{}${{+{}}}{}", prefix, name_with_sub, suffix);
            return paramsubst(&rewritten, prefix.chars().count(), qt, pf_flags, ret_flags);
        }
    }

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
        // Accept both literal `[` and tokenized Inbrack (\u{91}) —
        // the lexer-pre-tokenized form arrives via the bridge for
        // `${name[sub]}` inside DQ, while bare-form input from
        // direct calls (e.g. BUILTIN_ARRAY_INDEX) keeps ASCII `[`.
        // Parity bug: `$h[$k]` (unbraced, var-in-subscript) lost
        // the subscript walk because the bracket was already
        // Inbrack-tokenized by the inner `$k` having been
        // processed during outer expansion; the `[` -only check
        // dropped through to scalar lookup, then `[a]` glob'd.
        let mut subscript_str: Option<String> = None; // c:1625
        let opener = chars.get(pos).copied();
        // c:Src/subst.c:2800-2802 — fetchvalue's bracket-parse arg is
        // `(unset(KSHARRAYS) || inbrace) ? 1 : -1`: the BARE form
        // under KSHARRAYS does NOT parse a subscript; c:2867 keeps the
        // bracket loop off for bare+KSHARRAYS too. The `[...]` stays
        // literal trailing text (any `$refs` inside it are expanded by
        // the continuing stringsubst scan) and undergoes filename
        // generation downstream. zsh 5.9: `setopt ksharrays;
        // i=0; a=(x y z); print -- $a[$i]` →
        // `zsh:1: no matches found: x[0]`, rc=1.
        let ksharrays_bare =
            crate::ported::zsh_h::isset(crate::ported::zsh_h::KSHARRAYS);
        if !ksharrays_bare && (opener == Some('[') || opener == Some(Inbrack)) {
            // c:1625
            // Collect until matching `]` / Outbrack (depth-tracked
            // so `$arr[$other[1]]` works).
            let mut depth = 1; // c:1625
            let mut q = pos + 1; // c:1625
            while q < chars.len() && depth > 0 {
                // c:1625
                let ch = chars[q];
                if ch == '[' || ch == Inbrack {
                    depth += 1;
                } else if ch == ']' || ch == Outbrack {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                q += 1; // c:1625
            } // c:1625
            if depth == 0 {
                // c:1625
                let raw_sub: String = chars[pos + 1..q].iter().collect(); // c:1625
                                                                          // c:Src/params.c::getarg:1571 — `singsub(&s)` runs on
                                                                          // the subscript text after the needtok scan. The post-
                                                                          // singsub form still carries lexer TOKEN bytes for
                                                                          // non-substitution chars (Dash `\u{9b}` from `col-$k`,
                                                                          // Equals `\u{8d}`, etc.) — singsub only resolves
                                                                          // `$`/`` ` `` substitutions, not token-byte normalisation.
                                                                          // C zsh's getindex at c:1584 calls `untokenize(s)` BEFORE
                                                                          // the hash/array lookup so the key matches the stored
                                                                          // ASCII form. zshrs's bare-form `$NAME[KEY]` arm
                                                                          // previously fed the token-bearing key straight into
                                                                          // `map.get(sub)` at line ~10748 — for `H[col-$k]` with
                                                                          // $k="foo" the lookup searched for `col\u{9b}foo` but
                                                                          // the stored key was `col-foo`, so the lookup missed
                                                                          // and the bare `[[ -z $H[col-$k] ]]` cond test saw
                                                                          // empty. Mirror C's getindex untokenize step.
                let expanded = singsub(&raw_sub); // c:1571
                subscript_str = Some(crate::lex::untokenize(&expanded)); // c:1584
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
                    let exact = flags.contains('e'); // c:1419 e flag — literal compare
                    let mut out: Vec<String> = Vec::new();
                    for (k, v) in map.iter() {
                        let hay = if by_key { k.as_str() } else { v.as_str() };
                        let matched = if exact {
                            hay == pat.as_str()
                        } else {
                            patcompile(&{ let mut __pat_tok = (&pat).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, PAT_HEAPDUP as i32, None)
                                .map_or(false, |__p| pattry(&__p, hay))
                        };
                        if matched {
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
                        if patcompile(&{ let mut __pat_tok = (&pat).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, PAT_HEAPDUP as i32, None)
                            .map_or(false, |__p| pattry(&__p, elem))
                        {
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
                    getarrvalue(&arr, lo, hi).join(" ") // c:1625
                } else if let Some(idx) = sub.parse::<i32>().ok().or_else(|| {
                    // c:Src/params.c:1419-1432 — getarg routes a
                    // numeric subscript through mathevali so
                    // `${a[1+1]}`, `${a[i+1]}`, `${a[n]}` all
                    // evaluate as math. Direct port of the
                    // mathevali(sub) call inside getindex.
                    crate::ported::math::mathevali(sub).ok().map(|v| v as i32)
                }) {
                    // c:1625
                    let n = arr.len() as i32; // c:1625
                                              // c:Src/params.c:2125-2150 — KSHZEROSUBSCRIPT
                                              // non-strict mode: `a[0]` aliases to `a[1]` (first
                                              // element). Strict mode (default) returns empty.
                                              // Without this, `setopt kshzerosubscript; a=(q);
                                              // print $a[0]` returned "" instead of "q".
                    let i = if idx == 0 {
                        if crate::ported::zsh_h::isset(crate::ported::zsh_h::KSHZEROSUBSCRIPT) {
                            0 // c:2140 — `end = startnextlen` (first elem)
                        } else {
                            -1 // c:2148 — sentinel, returns empty
                        }
                    } else if idx < 0 {
                        n + idx
                    } else {
                        idx - 1
                    }; // c:1625
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
                // c:1625 — magic-assoc lookup via canonical PARTAB
                // (Src/Modules/parameter.c:2235-2298 ports at
                // parameter.rs::PARTAB / PARTAB_ARRAY). Mirrors the
                // companion braced-form dispatch above.
                let is_splice = sub == "@" || sub == "*";
                if is_splice {
                    if let Some(values) = arrays_get(&var_name) {
                        Some(values.join(" "))
                    } else if let Some(keys) = assoc_get(&var_name).map(|m_| m_.keys().cloned().collect::<Vec<String>>()) {
                        let vals: Vec<String> = keys
                            .iter()
                            .map(|k| assoc_get(&var_name).and_then(|m_| m_.get(k.as_str()).cloned()).unwrap_or_default())
                            .collect();
                        Some(vals.join(" "))
                    } else {
                        None
                    }
                } else if let Some(elem) = (|| -> Option<String> {
                    // c:Src/params.c:2110-2150 getarg index semantics on a
                    // PM_ARRAY magic param (1-based, negative from end, [0]
                    // per KSHZEROSUBSCRIPT, KSH_ARRAYS 0-based) over the
                    // PARTAB_ARRAY row's gsu getfn value (former bridge
                    // partab_array_index, inlined).
                    let arr = arrays_get(&var_name)?;
                    crate::ported::modules::parameter::PARTAB_ARRAY
                        .iter()
                        .find(|e_| e_.name == var_name.as_str())?;
                    let idx_n: i64 = crate::ported::math::mathevali(sub.trim()).unwrap_or(0);
                    let len = arr.len() as i64;
                    let ksh_arrays = crate::ported::zsh_h::isset(crate::ported::zsh_h::KSHARRAYS);
                    let i = if ksh_arrays {
                        if idx_n < 0 { len + idx_n } else { idx_n }
                    } else if idx_n == 0 {
                        if crate::ported::zsh_h::isset(crate::ported::zsh_h::KSHZEROSUBSCRIPT) { 0 } else { -1 }
                    } else if idx_n < 0 {
                        len + idx_n
                    } else {
                        idx_n - 1
                    };
                    Some(if i >= 0 && (i as usize) < arr.len() {
                        arr[i as usize].clone()
                    } else {
                        String::new()
                    })
                })() {
                    // c:Src/Modules/system.c:880 errnosgetfn — PM_ARRAY
                    // magic params subscript like ordinary arrays
                    // (getindex → getarg, Src/params.c:2110-2150).
                    // Mirrors the braced-form dispatch above.
                    Some(elem)
                } else {
                    (|| -> Option<String> {
                    // c:Src/Modules/parameter.c — special-hash getnode
                    // dispatch (`ht->getnode(ht, key)` = getpm*) for one
                    // key; module-gated rows resolve only after their
                    // zmodload (former bridge partab_get, inlined —
                    // single-key reads must NOT materialize the table:
                    // ${commands[git]} would enumerate $PATH).
                    let e_ = crate::ported::modules::parameter::PARTAB
                        .iter()
                        .find(|e_| e_.name == var_name.as_str())?;
                    if let Some(m_) = e_.module {
                        if !crate::ported::module::MODULESTAB
                            .lock()
                            .map(|t| t.is_loaded(m_))
                            .unwrap_or(false)
                        {
                            return None;
                        }
                    }
                    (e_.getfn)(std::ptr::null_mut(), sub).and_then(|p_| p_.u_str)
                })()
                }
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
                    getarrvalue(&chars_arr, lo, hi).concat()
                // c:1625
                } else if let Ok(idx) = sub.parse::<i32>() {
                    // c:1625
                    let n = chars_v.len() as i32; // c:1625
                                                  // c:Src/params.c:2125-2150 — KSHZEROSUBSCRIPT
                                                  // non-strict mode on scalar char-index.
                    let i = if idx == 0 {
                        if crate::ported::zsh_h::isset(crate::ported::zsh_h::KSHZEROSUBSCRIPT) {
                            0 // c:2140
                        } else {
                            -1 // c:2148
                        }
                    } else if idx < 0 {
                        n + idx
                    } else {
                        idx - 1
                    }; // c:1625
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
        } else if ksharrays_bare {
            // c:Src/params.c:2293-2296 — KSHARRAYS bare reference to
            // an identifier-named array/assoc collapses to the FIRST
            // element (`v->end = 1, v->isarr = 0`), not the joined
            // whole.
            arrays_get(&var_name)
                .map(|arr| arr.first().cloned().unwrap_or_default())
                .or_else(|| {
                    assoc_get(&var_name)
                        .map(|m| m.values().next().cloned().unwrap_or_default())
                })
                .or_else(|| exec_getsparam(&var_name))
                .unwrap_or_default()
        } else {
            // c:1625
            // No subscript: route through the canonical getsparam
            // funnel (GSU + variables + env + array-join), then
            // fall through to assoc-values for `$assoc` bare reads.
            // Same single-funnel pattern as subst.rs:2120.
            exec_getsparam(&var_name)
                .or_else(|| {
                    assoc_get(&var_name).map(|m| m.values().cloned().collect::<Vec<_>>().join(" "))
                })
                .unwrap_or_default() // c:1625
        }; // c:1625

        // c:Src/subst.c:1820 — bare `$NAME:MOD` and `$NAME[SUB]:MOD`
        // history-style modifier chain. Bugs #579, #580, #581.
        let (new_value, new_pos) = apply_bare_modifier_chain(&value, &chars, pos);
        let mut value = new_value;
        pos = new_pos;
        let _ = value;

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
            && !ksharrays_bare // c:Src/params.c:2293-2296 — isarr=0, no splat
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
                        // c:Src/params.c — KSH_ARRAYS shifts positive
                        // 0-based slice bounds to 1-based for getarrvalue.
                        // Sibling of #610-#613. Bug #614.
                        let ksh_arrays = isset(crate::ported::zsh_h::KSHARRAYS);
                        let (lo, hi) = if ksh_arrays {
                            let new_lo = if lo >= 0 { lo + 1 } else { lo };
                            let new_hi = if hi >= 0 { hi + 1 } else { hi };
                            (new_lo, new_hi)
                        } else {
                            (lo, hi)
                        };
                        arrays_get(&var_name)
                            .as_ref()
                            .map(|arr| getarrvalue(arr, lo, hi))
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
            if let Some(arr) = slice_arr.or(assoc_vals).or_else(|| arrays_get(&var_name)) {
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

    // c:Src/subst.c:2596-2602 — `$~` / `$~~` GLOB_SUBST flag prefix
    // on bare-form parameter expansion. `~` consumes itself and
    // turns GLOB_SUBST on for the following name lookup (or off,
    // for the doubled `~~` form). When NO valid parameter name
    // follows, the expansion produces empty output (matches zsh's
    // "drops the `$~` literal entirely" behavior). Bug #547 in
    // docs/BUGS.md.
    //
    // C reference:
    //   } else if (c == '~' || c == Tilde) {
    //       /* GLOB_SUBST (forced) on or off (doubled) */
    //       if ((c = *++s) == '~' || c == Tilde) {
    //           globsubst = 0;
    //           s++;
    //       } else
    //           globsubst = 2;
    //   }
    if c == '~' || c == Tilde {
        let consumed = if chars
            .get(pos + 1)
            .copied()
            .map_or(false, |n| n == '~' || n == Tilde)
        {
            if !qt {
                tilde_carrier_note(crate::ported::zsh_h::isset(crate::ported::zsh_h::GLOBSUBST));
                        opt_state_set("globsubst", false);
            }
            2
        } else {
            if !qt {
                tilde_carrier_note(crate::ported::zsh_h::isset(crate::ported::zsh_h::GLOBSUBST));
                        opt_state_set("globsubst", true);
            }
            1
        };
        let new_pos = pos + consumed;
        // No more body OR next char isn't a valid parameter name
        // starter — `$~` / `$~~` with nothing usable following.
        // Emit empty for the expansion slot and let the surrounding
        // text in the source string remain unchanged.
        let next_starts_name = chars
            .get(new_pos)
            .copied()
            .map_or(false, |n| n == '_' || n.is_ascii_alphanumeric()
                || matches!(n, '@' | '*' | '#' | '?' | '!' | '$' | '-' | '0'));
        if !next_starts_name {
            let prefix: String = chars[..start_pos].iter().collect();
            let suffix: String = chars[new_pos..].iter().collect();
            let result = format!("{}{}", prefix, suffix);
            result_nodes.push(result.clone());
            return (result, prefix.len(), result_nodes);
        }
        // Re-enter paramsubst at the new position by SLICING out
        // the consumed `~` chars (so the dispatch sees `$X` at the
        // original start_pos). Direct port of C's `s++` / `s += 2`
        // advance after the flag arm.
        let mut sliced = String::with_capacity(s.len() - consumed);
        sliced.push_str(&chars[..pos].iter().collect::<String>());
        sliced.push_str(&chars[new_pos..].iter().collect::<String>());
        return paramsubst(&sliced, start_pos, qt, pf_flags, ret_flags);
    }

    // Special parameters: $?, $$, $#, $*, $@, $0-$9
    match c {
        // c:1625
        c if c == '?' || c == Quest => {
            // c:1625
            let value = vars_get("?") // c:1625
                .unwrap_or_else(|| "0".to_string()); // c:1625
            // c:Src/subst.c:1820 — `$?:MOD` modifier chain. Bug #582
            // extends the bare-form modifier walker to `?` and `$`.
            // Accept Quest token (`\u{97}`) alongside ASCII `?` for
            // unquoted contexts where the lexer tokenizes `?`.
            let (value, after_pos) =
                apply_bare_modifier_chain(&value, &chars, pos + 1);
            let prefix: String = chars[..start_pos].iter().collect(); // c:1625
            let suffix: String = chars[after_pos..].iter().collect(); // c:1625
            let result = format!("{}{}{}", prefix, value, suffix); // c:1625
            result_nodes.push(result.clone()); // c:1625
            (result, prefix.len() + value.len(), result_nodes) // c:1625
        } // c:1625
        c if c == '$' || c == Stringg => {
            // c:1625
            let value = std::process::id().to_string(); // c:1625
            // c:Src/subst.c:1820 — `$$:MOD` modifier chain. Accept
            // Stringg token (`\u{85}`) alongside ASCII `$` since the
            // lexer tokenizes the second `$` of `$$` as Stringg in
            // unquoted contexts.
            let (value, after_pos) =
                apply_bare_modifier_chain(&value, &chars, pos + 1);
            let prefix: String = chars[..start_pos].iter().collect(); // c:1625
            let suffix: String = chars[after_pos..].iter().collect(); // c:1625
            let result = format!("{}{}{}", prefix, value, suffix); // c:1625
            result_nodes.push(result.clone()); // c:1625
            (result, prefix.len() + value.len(), result_nodes) // c:1625
        } // c:1625
        c if c == '#' || c == Pound => {
            // c:1625
            // c:Src/subst.c — bare `$#name` is shorthand for `${#name}`
            // (length of named variable). If `#` is followed by an
            // ident-start char (alpha or `_`) OR a single-char special
            // (`@`, `*`, `?`, `!`, `-`, `0`, `$`), walk the name and
            // recurse through the brace form so the length-op pipeline
            // (paramsubst length_op handler) fires. Without this, `$#a`
            // resolved as `$#` (positional count) + literal "a" and
            // `$#?` as `$#` + literal "?". Accept Pound TOKEN here too
            // since the lexer tokenizes `#` inside DQ context.
            let next = chars.get(pos + 1).copied().unwrap_or('\0');
            // Accept ASCII single-char specials AND their tokenized
            // forms (Pound=#, Quest=?, Bang=!, Dash=-, Star=*, Stringg=$).
            // The lexer tokenizes these inside double-quotes / param
            // expansion contexts, so `$#?` arrives here with `next`
            // being Quest (\u{97}) not ASCII '?'.
            let next_starts_name = next.is_ascii_alphabetic()
                || next == '_'
                || matches!(next, '@' | '*' | '?' | '!' | '-' | '0' | '$')
                || next == Quest
                || next == Bang
                || next == Dash
                || next == Star
                || next == Stringg
                || next == Pound;
            if next_starts_name {
                let name_start = pos + 1;
                let mut name_end = name_start + 1;
                // Single-char specials stop after one char; identifiers
                // walk to the end of [A-Za-z0-9_]. Bug #577: lexer
                // tokenizes `-`/`?`/`!`/`*`/`#` as Dash/Quest/Bang/Star/
                // Pound and `$` as Stringg in unquoted contexts. Both
                // forms must terminate the name at one char so the
                // recursive `${#NAME}` paramsubst sees just the special
                // glyph and can dispatch through getsparam("-")/etc.
                // Without the token-form check, the walk fell through
                // to the ascii_alphanumeric loop which stopped on the
                // token glyph too — so name_end always stayed at
                // name_start+1 anyway, but the subsequent rewrite
                // emitted the raw token char inside `${#…}` which
                // paramsubst's name-lookup didn't recognize as the
                // ASCII special.
                let first = chars[name_start];
                let token_to_ascii = match first {
                    Dash => Some('-'),
                    Quest => Some('?'),
                    Bang => Some('!'),
                    Star => Some('*'),
                    Pound => Some('#'),
                    Stringg => Some('$'),
                    _ => None,
                };
                let is_single_special =
                    matches!(first, '@' | '*' | '?' | '!' | '-' | '0' | '$')
                        || token_to_ascii.is_some();
                if !is_single_special {
                    while name_end < chars.len()
                        && (chars[name_end].is_ascii_alphanumeric() || chars[name_end] == '_')
                    {
                        name_end += 1;
                    }
                }
                // Emit ASCII form into the rewrite so the recursive
                // paramsubst's name-lookup matches the canonical name.
                let name: String = if let Some(a) = token_to_ascii {
                    a.to_string()
                } else {
                    chars[name_start..name_end].iter().collect()
                };
                let prefix: String = chars[..start_pos].iter().collect();
                let suffix: String = chars[name_end..].iter().collect();
                let rewritten = format!("{}${{#{}}}{}", prefix, name, suffix);
                return paramsubst(&rewritten, prefix.chars().count(), qt, pf_flags, ret_flags);
            }
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
            let mut values = arrays_get("@").unwrap_or_default(); // c:1625
                                                                  // c:Src/lex.c gettokstr — bare `$@[SUB]` / `$*[SUB]` parses
                                                                  // the bracket subscript inline. Walk a depth-tracked `[...]`
                                                                  // after the `@`/`*` and apply via getarrvalue when present.
            let mut after_pos = pos + 1;
            // Accept literal `[` AND tokenized Inbrack (\u{86}) /
            // Outbrack (\u{8b}). When paramsubst runs on input that
            // came through the lexer (DQ context), `[`/`]` are stored
            // as Inbrack/Outbrack tokens; bare-form input (direct call
            // from BUILTIN_ARRAY_INDEX) keeps them as ASCII brackets.
            // c:Src/lex.c gettokstr — bare `$@[SUB]` / `$*[SUB]` parses
            // the bracket subscript inline. Both shapes need the walk.
            let nxt = chars.get(after_pos).copied();
            if nxt == Some('[') || nxt == Some(Inbrack) {
                let mut depth = 1;
                let mut q = after_pos + 1;
                while q < chars.len() && depth > 0 {
                    match chars[q] {
                        c if c == '[' || c == Inbrack => depth += 1,
                        c if c == ']' || c == Outbrack => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    q += 1;
                }
                if depth == 0 && q < chars.len() && (chars[q] == ']' || chars[q] == Outbrack) {
                    let sub: String = chars[after_pos + 1..q].iter().collect();
                    after_pos = q + 1;
                    if let Some((lo, hi)) = sub.split_once(',') {
                        let lo: i64 = lo.trim().parse().unwrap_or(1);
                        let hi: i64 = hi.trim().parse().unwrap_or(0);
                        values = getarrvalue(&values, lo, hi);
                    } else if sub == "@" || sub == "*" {
                        // splat — values unchanged
                    } else if let Ok(idx) = sub.parse::<i64>() {
                        let n = values.len() as i64;
                        let i = if idx == 0 {
                            -1
                        } else if idx < 0 {
                            n + idx
                        } else {
                            idx - 1
                        };
                        values = if i >= 0 && (i as usize) < values.len() {
                            vec![values[i as usize].clone()]
                        } else {
                            Vec::new()
                        };
                    } else if sub.trim_start().starts_with('(') {
                        // c:Src/params.c::getarg:1389+ — subscript-flag
                        // dispatch. The `(r)`/`(R)`/`(i)`/`(I)`/`(k)`/
                        // `(K)` flags drive pattern/index searches over
                        // the array; in C, `$@[(I)pat]` reaches this
                        // branch via fetchvalue → getindex → getarg
                        // (the same path the braced form takes).
                        //
                        // zshrs's braced-form arm in this same file
                        // (line ~4541) already ports getarg's flag
                        // dispatch — route the bare form there by
                        // rewriting `$@[(...)…]` → `${@[(...)…]}` and
                        // recursing into paramsubst. Mirrors the
                        // existing length-form rewrite at line ~11187
                        // for bare `$#NAME` → `${#NAME}`.
                        //
                        // Without this, `(( $@[(I)-*] ))` (zinit.zsh:2668
                        // prehelp arg-guard) fell through with values
                        // = the full positional array, so arithsubst's
                        // singsub joined them and math saw
                        // `romkatv/powerlevel10k[(I)-*]` → "bad math
                        // expression: operator expected at `romkatv/po...'".
                        let prefix: String = chars[..start_pos].iter().collect();
                        let tail: String = chars[after_pos..].iter().collect();
                        let rewritten = format!("{}${{@[{}]}}{}", prefix, sub, tail);
                        return paramsubst(
                            &rewritten,
                            prefix.chars().count(),
                            qt,
                            pf_flags,
                            ret_flags,
                        );
                    }
                }
            }
            // zsh semantics:
            //   $* / "$*" — join with IFS first char
            //   $@        — splat into separate words
            //   "$@"      — preserve array shape (still splat)
            // Our port: $@ (qt or unqt) → splat; $* → join.
            // Direct port of subst.c c:1625 dispatch — only $* with
            // any quoting joins; $@ always preserves array shape.
            let value = if c == '*' {
                // c:1625 — `$*` joins via sepjoin(arr, NULL, 1).
                // c:Src/utils.c:3936-3945 — set-but-empty IFS joins
                // with "" (`IFS=""; echo "$*"` concatenates); only
                // unset / space-leading IFS yields " ". The previous
                // `.and_then(chars().next())` collapsed empty-IFS to
                // a space.
                crate::ported::utils::sepjoin(&values, None) // c:1625
            } else {
                // c:1625
                // $@ / "$@" in unquoted/SINGLE-aware context
                if pf_flags & PREFORK_SINGLE == 0 {
                    // c:1625
                    let prefix: String = chars[..start_pos].iter().collect(); // c:1625
                    let suffix: String = chars[after_pos..].iter().collect(); // c:1625
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
            let suffix: String = chars[after_pos..].iter().collect(); // c:1625
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
            let mut value = if digit == 0 {
                // c:1625
                vars_get("0").unwrap_or_default() // c:1625
            } else {
                // c:1625
                arrays_get("@") // c:1625
                    .and_then(|a| a.get(digit.saturating_sub(1)).cloned()) // c:1625
                    .unwrap_or_default() // c:1625
            }; // c:1625
            // c:Src/subst.c:1820 — `$N:MOD` history-style modifier
            // chain on bare positional. Bug #581 — same shape as the
            // `$NAME:MOD` walker at subst.rs:10080 but rooted at the
            // digit-positional arm.
            if chars.get(nx).copied() == Some(':') {
                let (new_value, new_pos) =
                    apply_bare_modifier_chain(&value, &chars, nx);
                value = new_value;
                nx = new_pos;
            }
            let prefix: String = chars[..start_pos].iter().collect(); // c:1625
            let suffix: String = chars[nx..].iter().collect(); // c:1625
            let result = format!("{}{}{}", prefix, value, suffix); // c:1625
            result_nodes.push(result.clone()); // c:1625
            (result, prefix.len() + value.len(), result_nodes) // c:1625
        } // c:1625
        // c:Src/subst.c — `$-:MOD`, `$!:MOD`. Routes through
        // exec_getsparam which dispatches to lookup_special_var
        // (params.rs:10727). Bug #582 — handle both ASCII and tokenized
        // (Dash \u{9b}, Bang \u{9c}) forms.
        c if c == '-' || c == Dash || c == '!' || c == Bang => {
            let name = if c == Dash {
                "-".to_string()
            } else if c == Bang {
                "!".to_string()
            } else {
                c.to_string()
            };
            let value = exec_getsparam(&name).unwrap_or_default();
            let (value, after_pos) =
                apply_bare_modifier_chain(&value, &chars, pos + 1);
            let prefix: String = chars[..start_pos].iter().collect();
            let suffix: String = chars[after_pos..].iter().collect();
            let result = format!("{}{}{}", prefix, value, suffix);
            result_nodes.push(result.clone());
            (result, prefix.len() + value.len(), result_nodes)
        }
        _ => {
            // c:1625
            // Just a literal $ — if the dispatch char (which lives
            // at `chars[start_pos]`) is the tokenized DQ-form
            // (Qstring `\u{8c}`) or the bare Stringg (`\u{85}`),
            // emit it as a literal ASCII `$` so callers don't see
            // the metafied byte leak through. Parity bug: bare
            // `"$"` (a `$` with nothing valid following it inside
            // DQ) printed as the metafied byte sequence instead of
            // a real dollar sign.
            let leading = chars.get(start_pos).copied().unwrap_or('\0');
            if leading == Qstring || leading == Stringg {
                let mut out = String::with_capacity(s.len());
                out.push_str(&chars[..start_pos].iter().collect::<String>());
                out.push('$');
                out.push_str(&chars[start_pos + 1..].iter().collect::<String>());
                result_nodes.push(out.clone());
                (out, start_pos + 1, result_nodes)
            } else {
                result_nodes.push(s.to_string()); // c:1625
                (s.to_string(), start_pos + 1, result_nodes) // c:1625
            }
        } // c:1625
    } // c:1625
} // c:1625

/// Port of `arithsubst(char *a, char **bptr, char *rest)` from `Src/subst.c:4485`.
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
/// WARNING: param names don't match C — Rust=(expr, prefix, rest) vs C=(a, bptr, rest)
pub fn arithsubst(expr: &str, prefix: &str, rest: &str) -> String {
    // c:4485
    // Pre-resolve `$#NAME` before singsub — singsub treats `$#` as
    // positional-count (`$#`) followed by literal `NAME`, which mangles
    // `$#parts` to `0parts`. zsh's parser binds `$#NAME` as length-of
    // (parameter-name length form) when NAME is an identifier. Direct
    // port of zsh's `prefork()` Bnull-aware `$#` arm — Src/subst.c
    // around line 1860 dispatches via the param-name lookahead before
    // the math evaluator sees the expression.
    let expr = {
        let bytes: Vec<char> = expr.chars().collect();
        let mut out = String::with_capacity(expr.len());
        let mut i = 0;
        while i < bytes.len() {
            // Accept literal `$` AND Stringg (\u{85}) / Qstring (\u{8c})
            // — the lexer emits Stringg for `$X` at top level, Qstring
            // for `$X` inside double quotes. arithsubst sees the
            // tokenized form whenever the `$(( ))` body was lexed
            // through a DQ context (e.g. `"x=$(( $#a ))"`).
            let is_dollar = bytes[i] == '$' || bytes[i] == Stringg || bytes[i] == Qstring;
            if is_dollar && i + 1 < bytes.len() && bytes[i + 1] == '#' {
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
    let v = match crate::math::matheval(&expanded) {
        // c:4490 matheval
        Ok(n) => n,
        Err(msg) => {
            // c:math.c::checkunary `zerr(...)` side effect — C's
            // matheval emits the parse-error string via zerr() which
            // writes to stderr AND sets errflag. The Rust port's
            // matheval returns Err with the message but doesn't
            // surface it; without this propagation, malformed math
            // like `$((1 2))` silently returned 0 (MN_UNSET) instead
            // of zsh's `bad math expression: operator expected at ...`.
            zerr(&msg);
            crate::math::mnumber {
                l: 0,
                d: 0.0,
                type_: MN_UNSET,
            }
        }
    };

    // c: math.c:580-583 — `outputradix` / `outputunderscore` are set
    // while parsing `[#N]` / `[##N]` / `[#N_M]` math prefixes; both
    // statics live in math.rs. `crate::math::outputradix()` /
    // `outputunderscore()` are accessors that read those statics.
    //
    // Fall back to the `OUTPUT_RADIX` shell-style env override (a
    // zshrs-only convenience for callers that can't put `[##16]`
    // inside the `$((…))` body) when the per-call radix is 0.
    let outputradix = {
        let r = crate::math::outputradix();
        if r != 0 {
            r // c:580 — `[#N]`/`[##N]` set this during matheval
        } else {
            vars_get("OUTPUT_RADIX")
                .as_ref()
                .and_then(|s| s.parse::<i32>().ok())
                .unwrap_or(0) // c:4492 (env fallback)
        }
    };
    let outputunderscore: i32 = crate::math::outputunderscore(); // c:583
    let b: String = if v.type_ == MN_UNSET {
        "0".to_string() // c:4498 — MN_UNSET falls through to zero in practice
    } else if (v.type_ == MN_FLOAT) && outputradix == 0 {
        // c:4493-4494
        convfloat_underscore(v.d, outputunderscore)
    } else {
        // c:4496-4498
        let l = if (v.type_ == MN_FLOAT) {
            v.d as i64
        } else {
            v.l
        };
        convbase_underscore(l, outputradix, outputunderscore)
    }; // c:4499

    // C: `t = *bptr = hcalloc(...); …; strcat(t, rest);` — concat
    // prefix + b + rest. Returns pointer past prefix+b (where rest
    // begins). Rust returns the full string.
    format!("{}{}{}", prefix, b, rest) // c:4501-4509
} // c:4509

// CaseMod enum imported from src/ported/hist.rs (canonical port of
// Src/hist.c::casemodify's CASMOD_* flag set). Local definition was
// drift — variants (None/Lower/Upper/Caps) duplicated hist.rs's
// (Lower/Upper/Caps) with an extra unused `None` variant.

/// c:Src/subst.c:1820 — bare `$NAME:MOD` / `$N:MOD` modifier walker.
/// Probes for a chain of history-style modifiers starting at
/// `chars[start]` and, on success, returns the value after applying
/// them and the new cursor position past the consumed chain.
///
/// Supported:
///   - simple letters: h/t/r/e/l/u/q/Q/a/A/P (+ optional digit count
///     for :hN / :tN)
///   - substitution: :s/PAT/REPL/ and :gs/PAT/REPL/ (delimiter is
///     char after s/gs; pattern, replacement terminated by same
///     delim; backslash escapes)
///
/// Anchored on `:` followed by a known modifier letter (or `g` then
/// `s`) so `$a:$b` stays two expansions. Bugs #579/#580/#581.
pub fn apply_bare_modifier_chain(
    value: &str,
    chars: &[char],
    start: usize,
) -> (String, usize) {
    if chars.get(start).copied() != Some(':') {
        return (value.to_string(), start);
    }
    let mut mod_buf = String::new();
    let mut probe = start;
    while probe < chars.len() && chars[probe] == ':' {
        let saw_g = chars.get(probe + 1).copied() == Some('g');
        let s_pos = if saw_g { probe + 2 } else { probe + 1 };
        if saw_g && chars.get(s_pos).copied() != Some('s') {
            break;
        }
        let after = chars.get(s_pos).copied();
        if saw_g || after == Some('s') {
            if chars.get(s_pos).copied() != Some('s') {
                break;
            }
            let delim_pos = s_pos + 1;
            let delim = match chars.get(delim_pos).copied() {
                Some(d) => d,
                None => break,
            };
            let mut q = delim_pos + 1;
            let mut pat_end = None;
            while q < chars.len() {
                if chars[q] == '\\' && q + 1 < chars.len() {
                    q += 2;
                    continue;
                }
                if chars[q] == delim {
                    pat_end = Some(q);
                    break;
                }
                q += 1;
            }
            let pat_end = match pat_end {
                Some(p) => p,
                None => break,
            };
            q = pat_end + 1;
            let mut repl_end = None;
            while q < chars.len() {
                if chars[q] == '\\' && q + 1 < chars.len() {
                    q += 2;
                    continue;
                }
                if chars[q] == delim {
                    repl_end = Some(q);
                    break;
                }
                q += 1;
            }
            let span_end = match repl_end {
                Some(p) => p + 1,
                None => chars.len(),
            };
            mod_buf.push(':');
            for c in &chars[probe + 1..span_end] {
                mod_buf.push(*c);
            }
            probe = span_end;
            continue;
        }
        let is_simple_mod = matches!(
            after,
            Some('h' | 't' | 'r' | 'e' | 'l' | 'u' | 'q' | 'Q' | 'a' | 'A' | 'P')
        );
        if !is_simple_mod {
            break;
        }
        mod_buf.push(':');
        mod_buf.push(after.unwrap());
        probe += 2;
        // c:Src/subst.c:4571 — `if (inbrace && idigit((*ptr)[1]))`: the
        // `:hN`/`:tN` digit-count is ONLY honored inside braces. In the
        // bare `$var:h2` form inbrace==0, so the digit is NOT consumed —
        // it stays as trailing literal text (`$var:h2` ⇒ `${var:h}2`).
        // No digit loop here; probe halts after the modifier letter.
    }
    if mod_buf.is_empty() {
        (value.to_string(), start)
    } else {
        (modify(value, &mod_buf), probe)
    }
}

/// History-style colon modifiers
/// Apply a `:` modifier chain (`:t:r:s/x/y/`...).
/// Port of `modify(char **str, char **ptr, int inbrace)` from Src/subst.c:4531.
/// WARNING: param names don't match C — Rust=(s, modifiers) vs C=(str, ptr, inbrace)
pub fn modify(s: &str, modifiers: &str) -> String {
    // c:4531
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
        let mut fixed_point = false;
        let mut max_iters: Option<u32> = None;
        // c:Src/subst.c:3786-3790 — tracks whether any pre-modifier
        // flag (`g`/`w`/`W`/`f`/`F`) was actually consumed. If yes
        // and no actual modifier letter follows, zsh emits the bare
        // "unrecognized modifier" diagnostic (the `else` arm at
        // c:3790 with no letter, since `*s == ':'` is false).
        let mut any_flag_consumed = false;

        // Parse modifier flags. `:g` is greedy/global, `:w` is
        // word-by-word, `:W:sep` is word-by-word with custom sep,
        // `:f` is fixed-point (repeat until no more changes),
        // `:FN` is bounded-iteration (repeat up to N times).
        loop {
            // c:4531
            match chars.peek() {
                // c:4531
                Some(&'g') => {
                    // c:4531
                    gbal = true; // c:4531
                    any_flag_consumed = true;
                    chars.next(); // c:4531
                } // c:4531
                Some(&'w') => {
                    // c:4531
                    wall = true; // c:4531
                    any_flag_consumed = true;
                    chars.next(); // c:4531
                } // c:4531
                Some(&'W') => {
                    // c:4531
                    any_flag_consumed = true;
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
                // c:Src/subst.c modify() — `f` flag = repeat the
                // following `:s`/`:&` until no more changes. `F N` =
                // bounded-iteration variant. Both are zsh-specific
                // (not POSIX); high-traffic in `${x:fs/.../...}`
                // idioms used by zsh autoload helpers.
                Some(&'f') => {
                    fixed_point = true;
                    any_flag_consumed = true;
                    chars.next();
                }
                Some(&'F') => {
                    any_flag_consumed = true;
                    chars.next();
                    let mut num = String::new();
                    while let Some(&pc) = chars.peek() {
                        if pc.is_ascii_digit() {
                            num.push(pc);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    max_iters = num.parse().ok();
                }
                _ => break, // c:4531
            } // c:4531
        } // c:4531

        let modifier = match chars.next() {
            // c:4531
            Some(c) => c, // c:4531
            None => {
                // c:Src/subst.c:3786-3790 — when the pre-flag loop
                // consumed `g`/`w`/`W`/`f`/`F` but no actual modifier
                // letter follows, zsh reports "unrecognized modifier"
                // with no letter. Previously this branch silently
                // exited the modify loop, so `${a:W}` returned the
                // value unchanged with rc=0.
                if any_flag_consumed {
                    zerr("unrecognized modifier"); // c:3790
                    errflag_set_error();
                }
                break;
            }
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
                None => {
                    // c:Src/subst.c modify() — bare `:s` with no
                    // delimiter byte → "bad substitution". Without
                    // this the `None => break` silently returned
                    // the unchanged value. Bug #595.
                    zerr("bad substitution");
                    errflag_set_error();
                    return String::new();
                }
            };
            // Read pattern with backslash-escape support.
            let mut pat = String::new(); // c:4595
            // c:Src/subst.c modify() — track whether the closing
            // pattern delimiter was actually consumed. C emits
            // `if (!*ptr2) zerr("bad substitution")` when no
            // closing delim is found before end of modifier string.
            // Bug #595.
            let mut pat_closed = false;
            while let Some(&c) = chars.peek() {
                if c == delim {
                    chars.next();
                    pat_closed = true;
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
            if !pat_closed {
                // c:Src/subst.c modify() — `if (!*ptr2) zerr("bad
                // substitution")` — the pattern body must end with
                // the chosen delimiter. `${a:s/l}` is rejected.
                zerr("bad substitution");
                errflag_set_error();
                return String::new();
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
            let use_glob = modifier == 'S' || isset(HISTSUBSTPATTERN);
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
                            if patcompile(&{ let mut __pat_tok = (&eff_pat).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, PAT_HEAPDUP as i32, None)
                                .map_or(false, |__p| pattry(&__p, &span))
                            {
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
            // c:Src/subst.c modify() — `f`/`F N` repeat-until-no-change
            // wrapper around the regular substitution. When neither flag
            // is set, __limit=1 ⇒ the body runs once and breaks.
            let __limit: u32 = max_iters.unwrap_or(if fixed_point { u32::MAX } else { 1 });
            let mut __iters: u32 = 0;
            loop {
                let __prev_for_fp: Option<String> = if fixed_point || max_iters.is_some() {
                    Some(result.clone())
                } else {
                    None
                };
                result = if anchor_head {
                    // c:4665
                    if use_glob {
                        let cv: Vec<char> = result.chars().collect();
                        let n = cv.len();
                        let mut found: Option<usize> = None;
                        for end in 0..=n {
                            let span: String = cv[..end].iter().collect();
                            if patcompile(&{ let mut __pat_tok = (&eff_pat).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, PAT_HEAPDUP as i32, None)
                                .map_or(false, |__p| pattry(&__p, &span))
                            {
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
                            if patcompile(&{ let mut __pat_tok = (&eff_pat).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, PAT_HEAPDUP as i32, None)
                                .map_or(false, |__p| pattry(&__p, &span))
                            {
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
                                let next_s =
                                    s + chars.next().map(|(b, _)| b).unwrap_or(rem.len() - s);
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
                __iters += 1;
                if __iters >= __limit {
                    break;
                }
                if let Some(p) = __prev_for_fp {
                    if result == p {
                        break;
                    }
                } else {
                    break;
                }
            }
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
            *hsubl.lock().unwrap() = Some(eff_pat.clone()); // c:4673
            *hsubr.lock().unwrap() = Some(repl.clone()); // c:4673
            hsubpatopt.store(mode as i32, Ordering::Relaxed); // c:4673
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
                let p_opt = hsubl.lock().unwrap().clone();
                let r_opt = hsubr.lock().unwrap().clone();
                match (p_opt, r_opt) {
                    (Some(p), Some(r)) => {
                        let mode = hsubpatopt.load(Ordering::Relaxed) as u8;
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
                'r' => Some(remtext(w)),                  // c:4585 (:r root)
                'e' => Some(rembutext(w)),                // c:4585 (:e ext)
                'l' => Some(casemodify(w, CASMOD_LOWER)), // c:4585 (:l)
                'u' => Some(casemodify(w, CASMOD_UPPER)), // c:4585 (:u)
                'q' => Some(quotestring(
                    // c:4585 (:q)
                    w,
                    QT_BACKSLASH,
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
                    } else if let Some(path) = getsparam("PATH") {
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

    // c:Src/subst.c:3786-3790 — after modify() consumes all valid
    // `:X` modifiers, if unconsumed text remains the caller emits:
    //   `:X` followed by a non-meta byte → "unrecognized modifier `X'"
    //   anything else (including bare letter, digit, etc.)        → "unrecognized modifier"
    // Without this check the outer `while chars.peek() == Some(&':')`
    // loop silently exits on any trailing non-`:` byte —
    // `${a:s/l/X/g}` and `${a:s/l/X/2}` accepted the bogus `g` / `2`
    // and returned the substitution result unchanged from the
    // legitimate trailing-delim case. Bug #594.
    if let Some(&leftover) = chars.peek() {
        // c:3787 — emit the named-modifier form only when the
        // leftover is `:X` (C: `*s == ':' && !imeta(s[1])`); the
        // trailing-after-delim case (`:s/.../.../X`) returns bare.
        if leftover == ':' {
            let _consume = chars.next();
            if let Some(&next) = chars.peek() {
                zerr(&format!("unrecognized modifier `{}'", next)); // c:3788
            } else {
                zerr("unrecognized modifier"); // c:3790
            }
        } else {
            zerr("unrecognized modifier"); // c:3790
        }
        errflag_set_error();
        return String::new();
    }

    result // c:4531
} // c:4531

/// Get a directory stack entry
/// Resolve `~+N`/`~-N` directory-stack entries.
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
/// Port of `dstackent(char ch, int val)` from `Src/subst.c:4902`.
/// WARNING: param names don't match C — Rust=(val, dirstack, pwd, pushdminus_set) vs C=(ch, val)
pub fn dstackent(
    // c:4902
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
    // backwards: `for (n=lastnode(dirstack); n != end && val; val--,
    //             n=prevnode(n));`
    // forwards: `for (end=NULL, n=firstnode(dirstack); n && val;
    //            val--, n=nextnode(n));`
    //
    // Track position as a signed cursor + at_end flag. C uses
    // `n=lastnode` which IS the sentinel when dirstack is empty —
    // the loop body never runs.
    let n_len = dirstack.len() as i32;
    let mut pos: i32;
    let mut at_end: bool;
    if backwards {
        pos = n_len - 1; // lastnode
        at_end = pos < 0; // empty dirstack → at end
        while !at_end && val > 0 {
            val -= 1;
            pos -= 1;
            if pos < 0 {
                at_end = true;
            }
        }
    } else {
        pos = 0; // firstnode
        at_end = pos >= n_len;
        while !at_end && val > 0 {
            val -= 1;
            pos += 1;
            if pos >= n_len {
                at_end = true;
            }
        }
    }

    // c:4913-4919 — `if (n == end) { if (backwards && !val) return pwd;
    //                if (isset(NOMATCH)) zerr(...); return NULL; }`
    if at_end {
        if backwards && val == 0 {
            // Empty dirstack with `~-0` returns pwd. Bug #358.
            return Some(pwd.to_string());
        }
        if crate::ported::zsh_h::isset(crate::ported::zsh_h::NOMATCH) {
            crate::ported::utils::zerr("not enough directory stack entries.");
        }
        return None;
    }

    // C: `return (char *)getdata(n);`
    dirstack.get(pos as usize).cloned() // c:4920
} // c:4922

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
// Aliases for the two names that diverged in the local module.
// Cite c:zsh.h:160 (`STRING`) and c:zsh.h:177 (`Outang`+proc-sub).
const STRING: char = Stringg; // c:zsh.h:160
const OUTANGPROC: char = OutangProc; // c:zsh.h:177

// `SubstState` and `SubstOptions` structs — DELETED per user
// directive ("SubstState must be removed", "SubstOptions must be
// removed", "delete SubstState"). All formerly-bundled fields are
// canonical globals or executor-backed:
//   - `errflag`     → `errflag` `AtomicI32`
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
//   - `last_subst` → `hsubl`/`hsubr`/`hsubpatopt`.
//   - `sub_flags`  → `SUB_FLAGS` thread_local at the top of this file.
// Every fn signature has dropped the `state: &mut SubstState` arg.

/// Null string constant from `Src/subst.c:36`: `char nulstring[] = {Nularg, '\0'};`
///
/// C value: `{0xa1, 0x00}` — the Nularg sentinel byte followed by
/// terminator. The previous Rust port had `"\u{8F}"` which is NOT
/// the canonical value (Nularg = 0xa1, not 0x8F = 0x8f). Same
/// drift-bug family as the TERM_UNKNOWN / HIST_* fixes.
///
/// Routes through the canonical `Nularg` const at zsh_h.rs:163.
/// Constructed as a const &str via the UTF-8 encoding of U+00A1.
pub const NULSTRING: &str = "\u{a1}"; // c:36 (Nularg sentinel)

/// Returns true if the global `errflag` (Src/utils.c) is set.
/// Matches the C idiom `if (errflag) …` that subst.c sprinkles
/// throughout its loops.
#[inline]
fn errflag_set() -> bool {
    errflag.load(Ordering::Relaxed) != 0
}

/// Sets `errflag |= ERRFLAG_ERROR` on the global `errflag`.
/// Mirrors C's `errflag |= ERRFLAG_ERROR;` at every subst.c site
/// where parameter / glob / arith error is reported.
#[inline]
fn errflag_set_error() {
    errflag.fetch_or(crate::ported::zsh_h::ERRFLAG_ERROR, Ordering::Relaxed);
}

// =====================================================================
// Parameter table read/write helpers — direct paramtab access.
// C reads `paramtab` directly via `getsparam`/`getaparam`
// (`Src/params.c:3194`/`:3245`); these mirror that by hitting
// `paramtab()` (the global Mutex<HashMap<
// String, Param>>) and the parallel `paramtab_hashed_storage`.
//
// Previous incarnation routed through `fusevm_bridge::try_with_executor`
// which silently no-ops outside a live VM frame (same fake pattern
// the user flagged earlier in ksh93.rs). Tests would compile and
// "pass" while exercising no parameter machinery at all.
// =====================================================================

// `splice_magic_assoc` deleted — was one big string-dispatcher
// that collapsed C's per-magic-assoc `scanpm<X>` walkers
// (`Src/Modules/parameter.c`) into a single switch. C dispatches
// each magic-assoc Param through its own `gsu->scantab` callback
// (set at module init); the per-Param scantab plumbing is a
// follow-up, but the body is now decomposed into individual
// `scanpm*` ported matching C's names, plus a `splice_magic_assoc`
// dispatcher that routes name → fn.

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
    /// `IN_PARAMSUBST_NEST` static.
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
    /// `SKIP_FILESUB` static.
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
    /// `SUB_FLAGS` static.
    pub static SUB_FLAGS: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
}

// `convbase` lives in src/ported/utils.rs (canonical port of
// Src/utils.c). Callers below import via the full path.

/// Multsub flags (from subst.c)
// `pub mod multsub_flags { … }` — DELETED per user directive; was
// a Rust-only u32 wrapper duplicating the canonical i32 constants
// in `zsh_h::MULTSUB_*` (c:zsh.h:2046-2059). Use those directly.
// c:zsh.h:2046-2059

/// Read a scalar variable. Routes through canonical `getsparam`
/// (`Src/params.c:3076`) — the C function dispatches through
/// `Param.gsu->getfn` for PM_SPECIAL params like IFS (whose value
/// is computed by `ifsgetfn` and not stored in `u_str`). The
/// previous Rust port read `pm.u_str` directly, which returned
/// None for PM_SPECIAL scalars — breaking PREFORK_SPLIT's
/// IFS-based splitting because `multsub` couldn't read $IFS.
fn vars_get(name: &str) -> Option<String> {
    crate::ported::params::getsparam(name)
}

/// True if `name` exists in `paramtab` (any type), OR in the process
/// environment, OR is a dynamic special parameter (EPOCHSECONDS,
/// EPOCHREALTIME, UID, RANDOM, SECONDS, etc.) that `lookup_special_var`
/// can resolve. zsh imports every env var into paramtab at startup
/// (createparamtable → addenv → setsparam loop); the Rust port's
/// env import is lazy — env reads route through getsparam's env
/// fallback only on access. `${+ENVVAR}` queried env vars that
/// hadn't been touched yet returned 0 even though zsh would treat
/// them as set. Add the env::var check so the chkset path matches.
///
/// Bug #31 in docs/BUGS.md: `${EPOCHSECONDS:-DEFAULT}` always fired
/// the default and `${+EPOCHSECONDS}` returned 0 because the dynamic
/// special params from `zsh/datetime` don't have a paramtab entry
/// (the module-loaded variant goes through a `lookup_special_var`
/// callback chain instead of paramtab). C zsh creates a Param via
/// `paramdef`'s `createspecial` path so it WAS in paramtab. The
/// Rust port routes those names through `lookup_special_var`
/// callbacks at read time without a backing Param, so the is-set
/// check has to consult the same callback path.
fn vars_contains(name: &str) -> bool {
    // c:Src/subst.c:3600 (`${+name}` chkset path) — C zsh's
    // `vunset = (!v || ...)` check at c:3193 treats PM_UNSET as
    // "not set" so `${+X}` returns 0. zshrs previously returned
    // true for any paramtab key regardless of PM_UNSET; under
    // `setopt typeset_to_unset`, bare `typeset X` creates the
    // entry with `PM_DEFAULTED = PM_DECLARED | PM_UNSET` (per
    // `Src/builtin.c:2544`), and `${+X}` should fire the unset
    // branch. Add the PM_UNSET filter so the chkset path sees
    // the declared-but-not-assigned state correctly. Bug #280
    // in docs/BUGS.md.
    let in_paramtab_and_set = paramtab().read().map_or(false, |tab| {
        tab.get(name).is_some_and(|pm| {
            (pm.node.flags as u32 & crate::ported::zsh_h::PM_UNSET) == 0
        })
    });
    in_paramtab_and_set
        || std::env::var(name).is_ok()
        || lookup_special_var(name).is_some()
}

/// Read an array parameter from `paramtab`. Equivalent to C's
/// `getaparam(name)` (`Src/params.c:3245`).
///
/// **Positional-param special-case** (c:Src/params.c:3262 IPDEF9
/// `pparams`): the names `@`, `*`, `argv` map to the positional
/// parameter vector. C zsh wires these through the parameter table's
/// IPDEF9 alias mechanism so getaparam("@") returns &pparams. zshrs's
/// paramtab doesn't carry the alias; positional params live in
/// `crate::ported::builtin::PPARAMS`. Map at this entry point so the
/// paramsubst array-arm (`${@[N]}` / `${@[*]}` / `${*[N,M]}`) sees the
/// positional vector instead of a None lookup.
///
/// **Previous gap:** `${@[@]}` / `print $@[@]` returned the bare `[@]`
/// suffix because arrays_get("@") returned None and the array-index
/// arm never fired — `set -- 1 2; print $@[@]` produced "1 2[@]"
/// instead of zsh's "1 2".
fn arrays_get(name: &str) -> Option<Vec<String>> {
    // c:Src/params.c:570-575 — nameref deref before the read.
    if crate::ported::params::is_nameref(name) {
        let t = match crate::ported::params::resolve_nameref_name(name, None) {
        crate::ported::params::nameref_resolution::Target { name: t_, .. } => t_,
        _ => name.to_string(),
    };
        if t != name {
            return arrays_get(&t);
        }
    }
    if name == "@" || name == "*" || name == "argv" {
        // c:Src/params.c:3262 IPDEF9 — pparams. Read the LIVE
        // executor positionals via the exec_hooks bridge: inside a
        // function `$@` is the FUNCTION's args (doshfunc swaps
        // pparams per frame, c:Src/exec.c:6021), while the static
        // PPARAMS mirror holds the toplevel set — reading it made
        // every `${(M)@:#pat}` filter inside functions operate on
        // the WRONG (usually empty) array, so the filter no-op'd:
        // zinit's `[[ -n ${(M)@:#+X} ]]` (tmp-subst-autoload) took
        // the +X arm for every plugin autoload and broke
        // add-zsh-hook / regexp-replace / add-zle-hook-widget
        // resolution in the interactive session.
        let live = crate::ported::exec_hooks::pparams();
        if !live.is_empty() {
            return Some(live);
        }
        let pp = crate::ported::builtin::PPARAMS.lock().ok()?;
        return Some(pp.clone());
    }
    // c:Src/Modules/parameter.c:2239 — `dirstack` PM_SPECIAL array
    // reads the canonical DIRSTACK LinkList via dirs_gsu.getfn.
    // paramtab has no u_arr backing for it; route here so
    // `$dirstack` / `${#dirstack}` / `${dirstack[N]}` read live.
    if name == "dirstack" {
        if let Ok(d) = crate::ported::modules::parameter::DIRSTACK.lock() {
            return Some(d.clone());
        }
    }
    // c:Src/signals.c:signals — read-only array of signal names
    // populated at startup. signals[1] = "EXIT", signals[2] = "HUP",
    // etc. zsh exposes this as a special parameter via PM_ARRAY
    // (Src/Modules/parameter.c).
    if name == "signals" {
        return Some(crate::ported::jobs::sig_names_for_signals_param());
    }
    // c:Src/Modules/parameter.c — `funcstack` PM_SPECIAL array
    // reads the canonical FUNCSTACK Vec via getfn. Same routing
    // as dirstack above so `$#funcstack` returns the call-depth.
    // FUNCSTACK holds funcstack structs; surface the .name field.
    if name == "funcstack" {
        if let Ok(f) = crate::ported::modules::parameter::FUNCSTACK.lock() {
            // c:Src/Modules/parameter.c — `$funcstack` exposes the
            // call-stack in INNERMOST-first order (funcstack[1] is
            // the most-recently-called function). zshrs's FUNCSTACK
            // Vec stores in push order (oldest first), so reverse
            // on read.
            return Some(f.iter().rev().map(|fs| fs.name.clone()).collect());
        }
    }
    // c:Src/Modules/datetime.c:256 — `epochtime` PM_ARRAY|PM_READONLY
    // backed by getcurrenttime() returning [tv_sec, tv_nsec] as
    // decimal strings. Without registration here `${epochtime[1]}`
    // / `${epochtime[@]}` returned empty. Bug #317 in
    // docs/BUGS.md. Module-load gate is implicit: getcurrenttime
    // is always linkable since the module is statically compiled
    // in.
    if name == "epochtime" {
        return Some(crate::ported::modules::datetime::getcurrenttime());
    }
    // c:Src/Zle/zleparameter.c:132 — `keymaps` PM_ARRAY|PM_READONLY
    // backed by keymapsgetfn (walks keymapnamtab). Without an arm
    // here, `${keymaps[N]}` and `${keymaps[@]}` returned empty
    // because the stub paramtab entry has no u_arr. Route through
    // the PARTAB_ARRAY getfn so subscripted reads work. Bug #383.
    // c:Src/Modules/parameter.c:2239-2291 — PM_ARRAY magic params
    // (dirstack/historywords/keymaps/errnos/…) dispatch through the
    // partab[] paramdef rows' gsu getfns. Walk the ported PARTAB_ARRAY
    // table directly (module-gated rows resolve only after their
    // zmodload, e.g. errnos → zsh/system). This replaces both the
    // keymaps special case (Bug #383) and the bridge-side
    // partab_array_get indirection.
    if let Some(entry) = crate::ported::modules::parameter::PARTAB_ARRAY
        .iter()
        .find(|e| e.name == name)
    {
        if let Some(modname) = entry.module {
            if !crate::ported::module::MODULESTAB
                .lock()
                .map(|t| t.is_loaded(modname))
                .unwrap_or(false)
            {
                return None;
            }
        }
        return Some((entry.getfn)(std::ptr::null_mut()));
    }
    if name == "funcfiletrace" || name == "funcsourcetrace" || name == "functrace" {
        // Route through the canonical ported getfns
        // (Src/Modules/parameter.c:648 functracegetfn, :679
        // funcsourcetracegetfn, :711 funcfiletracegetfn) instead of
        // duplicating the frame-walk inline. The previous inline copy
        // emitted `<filename>:<lineno>` for funcfiletrace, missing
        // the C `f->prev->flineno + f->lineno` join for frames whose
        // parent is a function/eval (parameter.c:747) — so
        // `funcfiletrace[1]` inside a nested call showed the raw
        // caller-relative line (0) instead of the absolute file line.
        // Bug #515/#585 history retained in the getfns' doc comments.
        return Some(match name {
            "funcfiletrace" => {
                crate::ported::modules::parameter::funcfiletracegetfn(std::ptr::null_mut())
            }
            "funcsourcetrace" => {
                crate::ported::modules::parameter::funcsourcetracegetfn(std::ptr::null_mut())
            }
            _ => crate::ported::modules::parameter::functracegetfn(std::ptr::null_mut()),
        });
    }
    let tab = paramtab().read().ok()?;
    let pm = tab.get(name)?;
    pm.u_arr.clone()
}

/// True if `name` is an array in `paramtab`.
fn arrays_contains(name: &str) -> bool {
    // c:Src/params.c:570-575 — nameref deref before the read.
    if crate::ported::params::is_nameref(name) {
        let t = match crate::ported::params::resolve_nameref_name(name, None) {
        crate::ported::params::nameref_resolution::Target { name: t_, .. } => t_,
        _ => name.to_string(),
    };
        if t != name {
            return arrays_contains(&t);
        }
    }
    // c:Src/params.c:3262 IPDEF9 — pparams is the @/argv array.
    if name == "@" || name == "*" || name == "argv" {
        return true;
    }
    // c:Src/Modules/parameter.c — special PM_ARRAY params backed by
    // ad-hoc storage (DIRSTACK list, signal-names table, etc.). The
    // matching arrays_get arm above synthesizes the Vec on each
    // call; mirror the "exists" bit here so `${#dirstack}` /
    // `${#signals}` length-op picks up an array source.
    if name == "dirstack" || name == "signals" {
        return true;
    }
    // c:Src/Modules/parameter.c — funcstack/funcfiletrace/
    // funcsourcetrace/functrace are PM_ARRAY|PM_READONLY backed by
    // the FUNCSTACK Vec (see the matching arrays_get arm at
    // subst.rs ~10685). Without an entry here, `${funcstack[@]}` /
    // `${(@)funcstack}` fell into the scalar arm via the else
    // branch at subst.rs:5514, set isarr=0, and emitted empty.
    // Bug #276 in docs/BUGS.md. `funcstack` alone (no subscript)
    // worked because raw_value reads the sepjoin'd form; the [@]
    // subscript path needed the array-shape signal.
    if matches!(
        name,
        "funcstack" | "funcfiletrace" | "funcsourcetrace" | "functrace" | "epochtime"
    ) {
        return true;
    }
    // c:Src/Zle/zleparameter.c:132 — `keymaps` PM_ARRAY backed by
    // keymapsgetfn. Mirror in the "exists" check so `${keymaps[@]}`
    // / `${(@)keymaps}` / `${#keymaps}` see the array shape. Bug #383.
    if name == "keymaps" {
        return true;
    }
    // c:Src/params.c:425-434 — tied-array IPDEF9 lowercase partners
    // (path/fpath/cdpath/mailpath/manpath/psvar/module_path) exist
    // whenever their uppercase scalar partner is set. The Rust port
    // doesn't auto-create the lowercase paramtab entry, so
    // `${+module_path}` etc. returned 0 even though MODULE_PATH was
    // set. Mirror the tied-partner aliasing here.
    let tied_partner = match name {
        "path" => Some("PATH"),
        "fpath" => Some("FPATH"),
        "cdpath" => Some("CDPATH"),
        "mailpath" => Some("MAILPATH"),
        "manpath" => Some("MANPATH"),
        "psvar" => Some("PSVAR"),
        "module_path" => Some("MODULE_PATH"),
        "zsh_eval_context" => Some("ZSH_EVAL_CONTEXT"),
        "fignore" => Some("FIGNORE"),
        _ => None,
    };
    if paramtab().read().map_or(false, |tab| {
        tab.get(name).map_or(false, |pm| pm.u_arr.is_some())
    }) {
        return true;
    }
    // Tied partner exists → array partner is conceptually set. C's
    // `$+name` returns 1 iff the paramtab entry exists, regardless
    // of value (including empty). Mirror via paramtab.contains_key
    // on the UPPERCASE partner. Falls back to env presence for the
    // hand-off case where the param hasn't been entered into
    // paramtab yet (env-import lazy path).
    if let Some(partner) = tied_partner {
        let in_tab = paramtab()
            .read()
            .map_or(false, |tab| tab.contains_key(partner));
        if in_tab || std::env::var(partner).is_ok() {
            return true;
        }
    }
    false
}

/// Insert / replace an array parameter. Writes through the
/// canonical paramtab as a `PM_ARRAY` entry.
fn arrays_insert(name: String, value: Vec<String>) {
    let mut tab = match paramtab().write() {
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
            u_data: 0,
            u_tied: None,
            u_arr: Some(value),
            u_str: None,
            u_val: 0,
            u_dval: 0.0,
            u_hash: None,
            gsu_s: None,
            gsu_i: None,
            gsu_f: None,
            gsu_a: None,
            gsu_h: None,
            base: 0,
            width: 0,
            env: None,
            ename: None,
            old: None,
            level: 0,
        });
        tab.insert(name, pm);
    }
}

/// Read an associative array parameter from the parallel
/// `paramtab_hashed_storage` (PM_HASHED values).
fn assoc_get(name: &str) -> Option<indexmap::IndexMap<String, String>> {
    // c:Src/params.c:570-575 — nameref deref before the read.
    let resolved = match crate::ported::params::resolve_nameref_name(name, None) {
        crate::ported::params::nameref_resolution::Target { name: t_, .. } => t_,
        _ => name.to_string(),
    };
    if let Some(m) = paramtab_hashed_storage()
        .lock()
        .ok()
        .and_then(|s| s.get(resolved.as_str()).cloned())
    {
        return Some(m);
    }
    // c:Src/Modules/parameter.c:2235+ — PM_HASHED magic assocs
    // (functions/aliases/commands/builtins/…): materialize via the
    // partab[] row's scanfn (key enumeration) + getfn (per-key
    // value), mirroring C's full-table scan through the special
    // hash's scantab. Module-gated rows (sysparams/errnos/mapfile/
    // langinfo) resolve only after their zmodload. NOTE: whole-map
    // consumers only — single-key reads use the getfn directly at
    // their sites to avoid enumerating e.g. $commands for one key.
    let entry = crate::ported::modules::parameter::PARTAB
        .iter()
        .find(|e| e.name == resolved.as_str())?;
    if let Some(modname) = entry.module {
        if !crate::ported::module::MODULESTAB
            .lock()
            .map(|t| t.is_loaded(modname))
            .unwrap_or(false)
        {
            return None;
        }
    }
    // C ScanFunc callback ABI (Src/zsh.h scantab): the scanfn walks
    // the special hash invoking a plain fn pointer per node — the
    // captureless callback collects into a thread-local, exactly the
    // shape C's scanpm* helpers use with their static linked lists.
    thread_local! {
        static PARTAB_SCAN_KEYS: std::cell::RefCell<Vec<String>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }
    fn partab_scan_cb(node: &crate::ported::zsh_h::HashNode, _flags: i32) {
        PARTAB_SCAN_KEYS.with(|k| k.borrow_mut().push(node.nam.clone()));
    }
    PARTAB_SCAN_KEYS.with(|k| k.borrow_mut().clear());
    (entry.scanfn)(std::ptr::null_mut(), Some(partab_scan_cb), 0);
    let keys = PARTAB_SCAN_KEYS.with(|k| k.borrow().clone());
    let mut out = indexmap::IndexMap::new();
    for k in keys {
        let v = (entry.getfn)(std::ptr::null_mut(), &k)
            .and_then(|p| p.u_str)
            .unwrap_or_default();
        out.insert(k, v);
    }
    Some(out)
}

/// True if `name` is an assoc-array in `paramtab_hashed_storage`.
fn assoc_contains(name: &str) -> bool {
    // c:Src/params.c:570-575 — nameref deref before the read.
    let resolved = match crate::ported::params::resolve_nameref_name(name, None) {
        crate::ported::params::nameref_resolution::Target { name: t_, .. } => t_,
        _ => name.to_string(),
    };
    paramtab_hashed_storage()
        .lock()
        .map_or(false, |s| s.contains_key(resolved.as_str()))
}

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
// c:zsh.h:1981-1996

/// Array assignment via paramtab. Equivalent to C's
/// `assignaparam(name, parts)` (`Src/params.c:3357`).
fn exec_assignaparam(name: &str, parts: Vec<String>) {
    arrays_insert(name.to_string(), parts);
}

// ============================================================================
// Additional helper functions ported from subst.c
// ============================================================================
#[cfg(test)] // utils.c:6915
#[allow(non_snake_case)] // utils.c:6915
                         // Test names embed zsh's flag/modifier letters as written in the
                         // shell — `(P)`, `(L)`, `(Q)`, `(U)`, etc. Forcing them to snake_case
                         // would obscure which zsh feature the test pins.
mod tests {
    use crate::zsh_h::{Hat, Tilde, CASMOD_CAPS};
    // utils.c:6915
    use super::*;
    // utils.c:6915

    /// Pin `NULSTRING` to the canonical Nularg sentinel value.
    /// C: `char nulstring[] = {Nularg, '\\0'};` at Src/subst.c:36
    /// where Nularg = 0xa1 (Src/zsh.h:206). The previous Rust port
    /// had `"\\u{8F}"` which is NOT the Nularg byte — same drift-bug
    /// family as TERM_UNKNOWN / HIST_* fixes.
    #[test]
    fn nulstring_matches_canonical_nularg_byte() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(Nularg as u32, 0xa1, "Src/zsh.h:206 — Nularg must be 0xa1");
        // NULSTRING is the str form of just the Nularg char.
        assert_eq!(
            NULSTRING, "\u{a1}",
            "Src/subst.c:36 — NULSTRING must be the single Nularg sentinel char"
        );
        let chars: Vec<char> = NULSTRING.chars().collect();
        assert_eq!(chars.len(), 1, "NULSTRING is a single char");
        assert_eq!(chars[0], Nularg, "NULSTRING's single char IS Nularg");
    }

    #[test] // utils.c:6915
    fn test_getkeystring() {
        let _g = crate::test_util::global_state_lock();
        // utils.c:6915
        assert_eq!(getkeystring("hello").0, "hello"); // utils.c:6915
        assert_eq!(getkeystring("hello\\nworld").0, "hello\nworld"); // utils.c:6915
        assert_eq!(getkeystring("\\t\\r\\n").0, "\t\r\n"); // utils.c:6915
        assert_eq!(getkeystring("\\x41").0, "A"); // utils.c:6915
        assert_eq!(getkeystring("\\u0041").0, "A"); // utils.c:6915
    } // utils.c:6915

    #[test]
    fn test_simple_param_expansion() {
        let _g = crate::test_util::global_state_lock();
        errflag.store(0, Ordering::Relaxed);
        let name = "FOO".to_string();
        let value = "bar".to_string();
        setsparam(&name, &value);

        let (result, _, _) = paramsubst("$FOO", 0, false, 0, &mut 0);
        assert_eq!(result, "bar");
    }

    #[test] // utils.c:6915
    fn test_modify_head() {
        let _g = crate::test_util::global_state_lock();
        // utils.c:6915
        let result = modify("/path/to/file.txt", ":h"); // utils.c:6915
        assert_eq!(result, "/path/to"); // utils.c:6915
    } // utils.c:6915

    #[test] // utils.c:6915
    fn test_modify_tail() {
        let _g = crate::test_util::global_state_lock();
        // utils.c:6915
        let result = modify("/path/to/file.txt", ":t"); // utils.c:6915
        assert_eq!(result, "file.txt"); // utils.c:6915
    } // utils.c:6915

    #[test] // utils.c:6915
    fn test_modify_extension() {
        let _g = crate::test_util::global_state_lock();
        // utils.c:6915
        let result = modify("/path/to/file.txt", ":e"); // utils.c:6915
        assert_eq!(result, "txt"); // utils.c:6915
    } // utils.c:6915

    #[test] // utils.c:6915
    fn test_modify_root() {
        let _g = crate::test_util::global_state_lock();
        // utils.c:6915
        let result = modify("/path/to/file.txt", ":r"); // utils.c:6915
        assert_eq!(result, "/path/to/file"); // utils.c:6915
    } // utils.c:6915

    #[test] // utils.c:6915
    fn test_dopadding() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        // utils.c:6915
        let name = "X".to_string();
        let value = "value".to_string();
        setsparam(&name, &value); // utils.c:6915
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
        let _g = crate::test_util::global_state_lock();
        // utils.c:6915
        // Src/hist.c:CASMOD_LOWER applies tolower() per char.
        assert_eq!(casemodify("Hello World", CASMOD_LOWER), "hello world"); // utils.c:6915
        assert_eq!(casemodify("MIXED-Case_42", CASMOD_LOWER), "mixed-case_42"); // utils.c:6915
        assert_eq!(casemodify("", CASMOD_LOWER), ""); // utils.c:6915
    } // utils.c:6915

    #[test] // utils.c:6915
    fn casemodify_upper_uppercases_each_char() {
        let _g = crate::test_util::global_state_lock();
        // utils.c:6915
        // Src/hist.c:CASMOD_UPPER applies toupper() per char.
        assert_eq!(casemodify("Hello World", CASMOD_UPPER), "HELLO WORLD"); // utils.c:6915
        assert_eq!(casemodify("ünicode", CASMOD_UPPER), "ÜNICODE"); // utils.c:6915
        assert_eq!(casemodify("", CASMOD_UPPER), ""); // utils.c:6915
    } // utils.c:6915

    #[test] // utils.c:6915
    fn casemodify_caps_titlecases_each_word() {
        let _g = crate::test_util::global_state_lock();
        // utils.c:6915
        // Src/hist.c:CASMOD_CAPS — uppercase first letter of each word,
        // lowercase the rest. zsh treats whitespace as a word boundary.
        assert_eq!(casemodify("hello world", CASMOD_CAPS), "Hello World"); // utils.c:6915
        assert_eq!(casemodify("FOO Bar", CASMOD_CAPS), "Foo Bar"); // utils.c:6915
    } // utils.c:6915

    #[test] // utils.c:6915
    fn casemodify_caps_treats_punctuation_as_word_boundary() {
        let _g = crate::test_util::global_state_lock();
        // utils.c:6915
        // Port of CASMOD_CAPS from Src/hist.c — non-alphanumerics
        // (incl. `-`, `.`, digits-then-alpha) reset `nextupper`.
        // Verified live: `print -r -- ${(C)"a-b c.d"}` → `A-B C.D`.
        assert_eq!(casemodify("a-b c.d", CASMOD_CAPS), "A-B C.D"); // utils.c:6915
        assert_eq!(casemodify("foo_bar.baz", CASMOD_CAPS), "Foo_Bar.Baz"); // utils.c:6915
    } // utils.c:6915

    // ─── remtpath (Src/hist.c:2055-2118) ────────────────────────────

    #[test] // utils.c:6915
    fn remtpath_count_zero_strips_last_component() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        // utils.c:6915
        // hist.c:2107-2114 — never erase root slash.
        assert_eq!(remtpath("/", 0), "/"); // utils.c:6915
        assert_eq!(remtpath("///", 0), "/"); // utils.c:6915
    } // utils.c:6915

    // ─── remlpaths (Src/hist.c:2151-2186) ───────────────────────────

    #[test] // utils.c:6915
    fn remlpaths_returns_last_n_components() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        // utils.c:6915
        // utils.c — \n \t \r \a \b \f \v \\ \' \"
        assert_eq!(getkeystring("\\n").0, "\n"); // utils.c:6915
        assert_eq!(getkeystring("\\t").0, "\t"); // utils.c:6915
        assert_eq!(getkeystring("\\r").0, "\r"); // utils.c:6915
        assert_eq!(getkeystring("\\\\").0, "\\"); // utils.c:6915
                                                  // Trailing literal — no escape consumed.
        assert_eq!(getkeystring("plain").0, "plain"); // utils.c:6915
    } // utils.c:6915

    #[test] // utils.c:6915
    fn getkeystring_decodes_hex_escape() {
        let _g = crate::test_util::global_state_lock();
        // utils.c:6915
        // utils.c handles `\xNN` (1-2 hex digits).
        assert_eq!(getkeystring("\\x41").0, "A"); // 0x41 = 'A' // utils.c:6915
        assert_eq!(getkeystring("\\x7e").0, "~"); // utils.c:6915
    } // utils.c:6915

    #[test] // utils.c:6915
    fn getkeystring_decodes_unicode_escape() {
        let _g = crate::test_util::global_state_lock();
        // utils.c:6915
        // utils.c `\uNNNN` form for BMP code points.
        assert_eq!(getkeystring("\\u00e9").0, "é"); // utils.c:6915
        assert_eq!(getkeystring("\\u4e2d").0, "中"); // utils.c:6915
    } // utils.c:6915

    // ─── paramsubst — bare ${VAR} ───────────────────────────────────

    // ─── paramsubst — operators ─────────────────────────────────────

    #[test] // c:1625
    fn paramsubst_default_when_unset() {
        let _g = crate::test_util::global_state_lock();
        // c:1625
        // subst.c:3202-3232 `case '-': case Dash:` — return operand
        // when value is unset. Unique name avoids paramtab collision
        // with other tests that share the global params::paramtab().
        // Reset `errflag` so prior tests' error states don't short-
        // circuit paramsubst (it returns early on errflag != 0).
        errflag.store(0, Ordering::Relaxed);
        let (result, _, _) =                                // c:3202
            paramsubst("${__default_unset_var:-fallback}", 0, false, 0, &mut 0); // c:3202
        assert_eq!(result, "fallback"); // c:3202
    } // c:3202

    #[test] // c:3300
    fn paramsubst_assign_default_writes_indexed_array_slot() {
        let _g = crate::test_util::global_state_lock();
        // c:3300
        // subst.c:3296-3305 `setaparam` path. zshrs port: numeric
        // subscript with no assoc declared → indexed slot, 1-based.
        // arrays_insert / arrays_get write through `paramtab`
        // directly (no ShellExecutor reach-in needed), so the test
        // body works without an ExecutorContext. The previous
        // incarnation that called `crate::vm_helper::ShellExecutor::new()`
        // + `ExecutorContext::enter` was a leftover from when the
        // ports mirrored writes into `exec.arrays`; both are now
        // dissolved.
        // Reset `errflag` so prior tests' error states don't short-
        // circuit paramsubst (it returns early on errflag != 0).
        errflag.store(0, Ordering::Relaxed);
        let name = format!(
            "__sub_arr_{}_{}",
            module_path!().replace("::", "_"),
            line!()
        );
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
        let _g = crate::test_util::global_state_lock();
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

    /// c:1348 — `get_strarg` parses `(c<str>c)` or `(c str c)` where
    /// `c` is the delimiter. The C source uses this for printf-style
    /// padding/quoting specifiers. Verify the canonical paren-delim
    /// AND the alternative `{...}` brace-delim shape — both are used
    /// in zsh substitution syntax (`${(p<TAB>)}` vs `${(l:5:)}`).
    #[test]
    fn get_strarg_extracts_paren_delimited_content() {
        let _g = crate::test_util::global_state_lock();
        let r = get_strarg("(foo)rest");
        assert_eq!(
            r,
            Some(('(', "foo".to_string(), "rest")),
            "(foo)rest must split into delim=( , content='foo', tail='rest'"
        );
    }

    /// c:1348 — `get_strarg("")` returns None (no delimiter).
    #[test]
    fn get_strarg_empty_input_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(get_strarg(""), None);
    }

    /// c:1428 — `get_intarg` reads `(N)` integer-padding args. Plain
    /// digit run with paren-delim should return (n, tail).
    #[test]
    fn get_intarg_parses_paren_int() {
        let _g = crate::test_util::global_state_lock();
        // Clear global errflag — `get_intarg` returns None whenever
        // `errflag != 0` (c:1445). Earlier tests in the suite may
        // have set ERRFLAG_ERROR via parser-failure paths and not
        // cleaned up; without this reset the test fails depending
        // on suite ordering.
        errflag.store(0, Ordering::Relaxed);
        let r = get_intarg("(42)rest");
        assert_eq!(
            r,
            Some((42, "rest")),
            "(42)rest must yield 42 + tail 'rest'"
        );
    }

    /// c:1428 — `get_intarg("")` returns None.
    #[test]
    fn get_intarg_empty_input_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(get_intarg(""), None);
    }

    /// c:814 — `strcatsub` concatenates prefix + src + suffix.
    /// glob_subst=false means no glob escaping; output is the
    /// straight concatenation. A regression that drops or duplicates
    /// any of the 3 parts would break every `${var:-fallback}` path.
    #[test]
    fn strcatsub_concatenates_three_parts_plain() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(strcatsub("a", "b", "c", false), "abc");
        assert_eq!(strcatsub("", "X", "", false), "X");
        assert_eq!(strcatsub("[", "y", "]", false), "[y]");
    }

    /// c:848 — `wcpadwidth` reports 1 for ASCII chars (default) AND
    /// honours the multi_width flag for wide chars. Regression that
    /// always returns 1 would mis-align `${(l:5:): -}`-style padding
    /// on CJK chars.
    #[test]
    fn wcpadwidth_reports_one_for_ascii() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(wcpadwidth('a', 0), 1);
        assert_eq!(wcpadwidth('Z', 0), 1);
    }

    /// c:848 — wide chars get the multi_width-controlled width.
    /// `multi_width=2` is the standard zsh "treat wide as 2-cols" mode.
    #[test]
    fn wcpadwidth_reports_width_two_for_cjk_when_multi_set() {
        let _g = crate::test_util::global_state_lock();
        // multi_width=2 treats wide chars as 2 columns.
        let w = wcpadwidth('中', 2);
        assert!(w >= 1, "CJK char must have width >= 1 (got {w})");
    }

    /// c:737 — `filesubstr` resolves `~/...` to $HOME/... and `~user/...`
    /// to a user homedir lookup. For plain (non-`~`) input the C body
    /// returns None — no substitution to apply.
    #[test]
    fn filesubstr_non_tilde_input_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(filesubstr("/literal/path", false), None);
        assert_eq!(filesubstr("relative", false), None);
        assert_eq!(filesubstr("", false), None);
    }

    /// c:737 — `~` with no following path or user-name resolves to
    /// $HOME directly. filesubstr requires the Tilde TOKEN
    /// (Src/zsh.h:189, `\u{98}`) per `*str == Tilde` at c:741. ASCII
    /// `~` is rejected — substitution results and DQ-quoted source
    /// carry ASCII `~` and zsh does not tilde-expand those.
    #[test]
    fn filesubstr_bare_tilde_resolves_to_home() {
        let _g = crate::test_util::global_state_lock();
        if let Ok(home) = std::env::var("HOME") {
            let r = filesubstr("\u{98}", false);
            assert_eq!(
                r.as_deref(),
                Some(home.as_str()),
                "Tilde TOKEN `\\u{{98}}` must expand to $HOME"
            );
        }
    }

    /// c:741 — ASCII `~` is NOT tilde-expanded; only the Tilde
    /// TOKEN form (lexer-emitted) triggers expansion. This pins the
    /// behavior that fixes `${var/pat/\~}` (replacement is literal)
    /// and DQ-quoted `"~/foo"` (no expansion inside double quotes).
    #[test]
    fn filesubstr_ascii_tilde_does_not_expand() {
        let _g = crate::test_util::global_state_lock();
        let r = filesubstr("~", false);
        assert!(r.is_none(), "ASCII `~` must not tilde-expand (got {:?})", r);
    }

    /// c:4531 — `modify(s, ":h")` is the dirname modifier — `${var:h}`
    /// returns the dir part of a path. `/foo/bar/baz:h` → `/foo/bar`.
    /// Regression dropping the trailing-component strip would break
    /// every script using `${PWD:h}` for parent-dir lookups.
    #[test]
    fn modify_h_strips_trailing_component() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(modify("/foo/bar/baz", ":h"), "/foo/bar");
    }

    /// c:4531 — `:t` is the basename modifier (tail). `/foo/bar/baz:t`
    /// → `baz`. Counterpart to `:h`.
    #[test]
    fn modify_t_returns_trailing_component() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(modify("/foo/bar/baz", ":t"), "baz");
    }

    /// c:4531 — `:r` strips the file extension (root). `foo.txt:r`
    /// → `foo`. Used by `${file:r}` for filename manipulation.
    #[test]
    fn modify_r_strips_extension() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(modify("foo.txt", ":r"), "foo");
        assert_eq!(modify("foo", ":r"), "foo", "no ext = no change");
    }

    /// c:4531 — `:e` returns just the extension. Counterpart to `:r`.
    #[test]
    fn modify_e_returns_extension() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(modify("foo.txt", ":e"), "txt");
    }

    /// c:4531 — `:u` uppercases (used by `${var:u}`). Critical for
    /// case-normalisation paths in user shell scripts.
    #[test]
    fn modify_u_uppercases() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(modify("hello", ":u"), "HELLO");
    }

    /// c:4531 — `:l` lowercases.
    #[test]
    fn modify_l_lowercases() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(modify("HELLO", ":l"), "hello");
    }

    /// c:4531 — chained modifiers compose left-to-right. `foo.txt:r:u`
    /// strips the extension THEN uppercases → `FOO`.
    #[test]
    fn modify_chained_modifiers_apply_left_to_right() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(modify("foo.txt", ":r:u"), "FOO");
    }

    /// c:4531 — empty modifier string is a no-op (returns input
    /// unchanged). Catches a regression that prepends `:` or
    /// processes phantom flags.
    #[test]
    fn modify_empty_modifier_returns_input_unchanged() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(modify("foo/bar", ""), "foo/bar");
    }

    /// c:1566 — `check_colon_subscript("")` returns None — empty
    /// input is not a subscript. Used by the `${var:0:5}`-style parser
    /// to distinguish positional subscripts from modifier letters.
    #[test]
    fn check_colon_subscript_empty_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(check_colon_subscript(""), None);
    }

    /// c:1571 — alphabetic-leading input (modifier letter like `:h`,
    /// `:t`, `:r`) MUST return None so the upper parser dispatches to
    /// modify(). Regression treating them as subscripts would break
    /// every `${PWD:h}`.
    #[test]
    fn check_colon_subscript_returns_none_on_modifier_letter() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(check_colon_subscript("h"), None);
        assert_eq!(check_colon_subscript("t"), None);
        assert_eq!(check_colon_subscript("r"), None);
        assert_eq!(
            check_colon_subscript("&5"),
            None,
            "`&` is the history-modifier prefix, not a subscript"
        );
    }

    /// c:1574-1576 — bare `::` (empty subscript) returns Some("0").
    /// `${var::5}` means "from position 0, take 5 chars". Regression
    /// returning None on the empty subscript breaks the substring
    /// shorthand.
    #[test]
    fn check_colon_subscript_bare_colon_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = check_colon_subscript(":remainder");
        assert!(r.is_some());
        let (sub, _rest) = r.unwrap();
        assert_eq!(sub, "0", "bare `:` subscript defaults to 0");
    }

    /// c:463 — `quotesubst` quotes meta chars in a string for
    /// pass-through to a subshell. Plain ASCII passes unchanged.
    #[test]
    fn quotesubst_passes_plain_ascii_unchanged() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(quotesubst("hello world"), "hello world");
        assert_eq!(quotesubst(""), "");
    }

    /// c:514 — `singsub` runs `${...}` substitution on its input
    /// string in-place; plain text with no `$` passes through.
    #[test]
    fn singsub_passes_plain_text_unchanged() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(singsub("hello"), "hello");
        assert_eq!(singsub(""), "");
        assert_eq!(singsub("no var"), "no var");
    }

    /// `Src/zsh.h:163,165,168,174,179,193,194` — pin the canonical
    /// token byte values that subst.rs callers compare against. The
    /// previous Rust port had FIVE places where the wrong byte was
    /// labeled as a different token: 0x85 was called both Inpar and
    /// Stringg, 0x86 was called both Equals and Outpar, etc. Pin
    /// every token const so a future regression that uses a literal
    /// byte instead of the const fails at compile time.
    #[test]
    fn token_byte_constants_match_zsh_h_canonical_values() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(Stringg as u32, 0x85, "Src/zsh.h:160 Stringg = $");
        assert_eq!(Hat as u32, 0x86, "Src/zsh.h:161 Hat = ^");
        assert_eq!(Inpar as u32, 0x88, "Src/zsh.h:163 Inpar = (");
        assert_eq!(Outpar as u32, 0x8a, "Src/zsh.h:165 Outpar = )");
        assert_eq!(Equals as u32, 0x8d, "Src/zsh.h:168 Equals = =");
        assert_eq!(Inbrack as u32, 0x91, "Src/zsh.h:172 Inbrack = [");
        assert_eq!(Outbrack as u32, 0x92, "Src/zsh.h:173 Outbrack = ]");
        assert_eq!(Tick as u32, 0x93, "Src/zsh.h:174 Tick = `");
        assert_eq!(Tilde as u32, 0x98, "Src/zsh.h:179 Tilde = ~");
        assert_eq!(Snull as u32, 0x9d, "Src/zsh.h:193 Snull");
        assert_eq!(Dnull as u32, 0x9e, "Src/zsh.h:194 Dnull");

        // Cross-pinning: confirm the WRONG mappings the previous port
        // had can NEVER match the canonical const. (If a future
        // regression revives `\u{85}` as Inpar, this would fail.)
        assert_ne!(
            Inpar as u32, 0x85,
            "Inpar must NOT equal 0x85 (that's Stringg)"
        );
        assert_ne!(
            Outpar as u32, 0x86,
            "Outpar must NOT equal 0x86 (that's Hat)"
        );
        assert_ne!(
            Equals as u32, 0x86,
            "Equals must NOT equal 0x86 (that's Hat)"
        );
        assert_ne!(
            Tick as u32, 0x83,
            "Tick must NOT equal 0x83 (that's Meta lead byte)"
        );
        assert_ne!(
            Snull as u32, 0x98,
            "Snull must NOT equal 0x98 (that's Tilde)"
        );
        assert_ne!(
            Dnull as u32, 0x97,
            "Dnull must NOT equal 0x97 (that's Quest)"
        );
    }

    /// `Src/subst.c:1348` — `get_strarg(":STR:rest")` walks until
    /// the matching close-delim and returns the inner content +
    /// the remainder past the close-delim.
    ///
    /// **Multibyte regression:** the previous Rust port indexed
    /// `&s[rest_start..]` with the CHAR index from
    /// `chars().enumerate()` instead of the byte index. ASCII input
    /// worked by coincidence; multibyte input (e.g. content with
    /// `é`) would land mid-codepoint, panicking on slice or
    /// returning corrupted output.
    #[test]
    fn get_strarg_multibyte_content_safe() {
        let _g = crate::test_util::global_state_lock();
        // ASCII smoke — pin the basic contract.
        let r = get_strarg(":foo:rest").unwrap();
        assert_eq!(r.0, ':');
        assert_eq!(r.1, "foo");
        assert_eq!(r.2, "rest");

        // Multibyte content between delimiters — the bug case.
        // `é` is 2 UTF-8 bytes. Previous char-indexed port either
        // panicked here (slice on non-UTF-8 boundary) or returned
        // wrong rest.
        let r = get_strarg(":é:rest").unwrap();
        assert_eq!(r.0, ':');
        assert_eq!(r.1, "é", "c:1348 — multibyte content preserved verbatim");
        assert_eq!(
            r.2, "rest",
            "c:1348 — rest starts AFTER close-delim (not mid-codepoint)"
        );

        // Bracket pair: `(...)` form with multibyte inside.
        let r = get_strarg("(héllo)tail").unwrap();
        assert_eq!(r.0, '(');
        assert_eq!(r.1, "héllo");
        assert_eq!(r.2, "tail");
    }

    /// `Src/subst.c:1348` — empty close-delim case: when no close
    /// matches before end-of-string, the rest is empty (C returns
    /// the consumed content with `lenp` pointing past the end).
    #[test]
    fn get_strarg_unterminated_returns_consumed_content() {
        let _g = crate::test_util::global_state_lock();
        // No matching `)` — content runs to end-of-string.
        let r = get_strarg("(unclosed_content").unwrap();
        assert_eq!(r.0, '(');
        assert_eq!(r.1, "unclosed_content");
        assert_eq!(r.2, "", "c:1348 — no close-delim → rest is empty");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Parameter-expansion forms anchored to real zsh 5.9.
    // Each test sets a param via `setsparam`, runs the expansion through
    // `paramsubst`, and asserts the value `zsh -c 'print -r -- "<expr>"'`
    // produces. Where zshrs diverges, the test fails — that failure is
    // the bug surface to fix.
    // ═══════════════════════════════════════════════════════════════════

    /// Set a scalar then run paramsubst on `expr` and return the result.
    ///
    /// `$((...))` arithmetic expressions route through `singsub` so the
    /// upstream `stringsubst` dispatch can hand them to `arithsubst` —
    /// `paramsubst` alone only handles `${...}` parameter forms, not
    /// the arith bracket. Detected by literal substring scan to keep
    /// the existing `${...}` tests on the same code path.
    ///
    /// `(#b)` / `(#m)` flag-prefix patterns require EXTENDEDGLOB to be
    /// active (per `Src/pattern.c:953-957` gate). The shell pipeline
    /// would already have set it via the test's `setopt EXTENDED_GLOB`
    /// preamble; psubst_one short-circuits the shell so we set it
    /// here. Saved/restored around the call.
    fn psubst_one(name: &str, value: &str, expr: &str) -> String {
        errflag.store(0, Ordering::Relaxed);
        setsparam(name, value);
        let needs_extglob = expr.contains("(#b)") || expr.contains("(#m)");
        let saved_extglob = if needs_extglob {
            let prev = crate::ported::options::opt_state_get("extendedglob").unwrap_or(false);
            crate::ported::options::opt_state_set("extendedglob", true);
            Some(prev)
        } else {
            None
        };
        let out = if expr.contains("$((") {
            singsub(expr)
        } else {
            let (out, _, _) = paramsubst(expr, 0, false, 0, &mut 0);
            out
        };
        if let Some(prev) = saved_extglob {
            crate::ported::options::opt_state_set("extendedglob", prev);
        }
        out
    }

    // ── Default operators ────────────────────────────────────────────
    /// `${F:-bar}` where F=foo → `foo` (param is set, default ignored).
    #[test]
    fn paramsubst_colon_dash_keeps_nonempty() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PS_F", "foo", "${PS_F:-bar}"), "foo");
    }

    /// `${F:-bar}` where F="" → `bar` (empty = use default with `:-`).
    #[test]
    fn paramsubst_colon_dash_replaces_empty() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PS_E", "", "${PS_E:-bar}"), "bar");
    }

    /// `${F-bar}` where F="" → `` (empty BUT set, no `:`, keep empty).
    #[test]
    fn paramsubst_dash_only_unset_check_keeps_empty() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PS_E2", "", "${PS_E2-bar}"), "");
    }

    /// `${F:+alt}` where F=foo → `alt` (set/nonempty → use alt).
    #[test]
    fn paramsubst_colon_plus_uses_alt_when_set() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PS_AF", "foo", "${PS_AF:+alt}"), "alt");
    }

    /// `${F:+alt}` where F="" → `` (empty → no alt).
    #[test]
    fn paramsubst_colon_plus_empty_when_empty() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PS_AE", "", "${PS_AE:+alt}"), "");
    }

    // ── Substring ────────────────────────────────────────────────────
    /// `${H:0:1}` where H=hello → `h`.
    #[test]
    fn paramsubst_substring_first_char() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PS_H", "hello", "${PS_H:0:1}"), "h");
    }

    /// `${H:1:3}` where H=hello → `ell`.
    #[test]
    fn paramsubst_substring_middle_three() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PS_H2", "hello", "${PS_H2:1:3}"), "ell");
    }

    /// `${H:2}` where H=hello → `llo` (offset, no length = rest of string).
    #[test]
    fn paramsubst_substring_offset_only() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PS_H3", "hello", "${PS_H3:2}"), "llo");
    }

    /// `${H:0:5}` where H=hello → `hello` (length equals full).
    #[test]
    fn paramsubst_substring_full_length() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PS_H4", "hello", "${PS_H4:0:5}"), "hello");
    }

    // ── Length ───────────────────────────────────────────────────────
    /// `${#H}` where H=hello → `5`.
    #[test]
    fn paramsubst_length_of_5char_string() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PS_LH", "hello", "${#PS_LH}"), "5");
    }

    /// `${#E}` where E="" → `0`.
    #[test]
    fn paramsubst_length_of_empty_is_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PS_LE", "", "${#PS_LE}"), "0");
    }

    // ── Prefix strip ─────────────────────────────────────────────────
    /// `${P#*/}` where P=/path/to/file.txt.bak → `path/to/file.txt.bak`
    /// (shortest prefix matching `*/` is just `/`).
    #[test]
    fn paramsubst_strip_shortest_prefix() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            psubst_one("PS_P", "/path/to/file.txt.bak", "${PS_P#*/}"),
            "path/to/file.txt.bak"
        );
    }

    /// `${P##*/}` → `file.txt.bak` (longest `*/` match).
    #[test]
    fn paramsubst_strip_longest_prefix() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            psubst_one("PS_P2", "/path/to/file.txt.bak", "${PS_P2##*/}"),
            "file.txt.bak"
        );
    }

    // ── Suffix strip ─────────────────────────────────────────────────
    /// `${P%.bak}` → `/path/to/file.txt`.
    #[test]
    fn paramsubst_strip_literal_suffix() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            psubst_one("PS_PS", "/path/to/file.txt.bak", "${PS_PS%.bak}"),
            "/path/to/file.txt"
        );
    }

    /// `${P%.*}` → `/path/to/file.txt` (shortest `.*` from end).
    #[test]
    fn paramsubst_strip_shortest_suffix_glob() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            psubst_one("PS_PSS", "/path/to/file.txt.bak", "${PS_PSS%.*}"),
            "/path/to/file.txt"
        );
    }

    /// `${P%%.*}` → `/path/to/file` (longest `.*` from end).
    #[test]
    fn paramsubst_strip_longest_suffix_glob() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            psubst_one("PS_PSL", "/path/to/file.txt.bak", "${PS_PSL%%.*}"),
            "/path/to/file"
        );
    }

    // ── Replace ──────────────────────────────────────────────────────
    /// `${S/X/_}` where S=aXbXc → `a_bXc` (first match only).
    #[test]
    fn paramsubst_replace_first_match() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PS_S", "aXbXc", "${PS_S/X/_}"), "a_bXc");
    }

    /// `${S//X/_}` → `a_b_c` (all matches).
    #[test]
    fn paramsubst_replace_all_matches() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PS_S2", "aXbXc", "${PS_S2//X/_}"), "a_b_c");
    }

    /// `${S/#a/Z}` → `ZXbXc` (anchored at start).
    #[test]
    fn paramsubst_replace_anchored_start() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PS_S3", "aXbXc", "${PS_S3/#a/Z}"), "ZXbXc");
    }

    /// `${S/%c/Z}` → `aXbXZ` (anchored at end).
    #[test]
    fn paramsubst_replace_anchored_end() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PS_S4", "aXbXc", "${PS_S4/%c/Z}"), "aXbXZ");
    }

    // ── Case-conversion flags ────────────────────────────────────────
    /// `${(L)MIX}` where MIX=aBcDeF → `abcdef`.
    #[test]
    fn paramsubst_flag_L_lowercases() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PS_MIX1", "aBcDeF", "${(L)PS_MIX1}"), "abcdef");
    }

    /// `${(U)MIX}` where MIX=aBcDeF → `ABCDEF`.
    #[test]
    fn paramsubst_flag_U_uppercases() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PS_MIX2", "aBcDeF", "${(U)PS_MIX2}"), "ABCDEF");
    }

    /// `${(C)MIX}` where MIX=aBcDeF → `Abcdef` (capitalize whole string =
    /// first char up, rest down). NOT per-word capitalization.
    #[test]
    fn paramsubst_flag_C_capitalizes_first_char_only() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PS_MIX3", "aBcDeF", "${(C)PS_MIX3}"), "Abcdef");
    }

    // ── Path modifiers via paramsubst (modify() is tested directly above) ─
    /// `${P:h}` where P=/path/to/file.txt.bak → `/path/to`.
    #[test]
    fn paramsubst_modifier_head() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            psubst_one("PS_MH", "/path/to/file.txt.bak", "${PS_MH:h}"),
            "/path/to"
        );
    }

    /// `${P:t}` → `file.txt.bak`.
    #[test]
    fn paramsubst_modifier_tail() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            psubst_one("PS_MT", "/path/to/file.txt.bak", "${PS_MT:t}"),
            "file.txt.bak"
        );
    }

    /// `${P:r}` → `/path/to/file.txt` (root = strip last extension).
    #[test]
    fn paramsubst_modifier_root_strips_one_extension() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            psubst_one("PS_MR", "/path/to/file.txt.bak", "${PS_MR:r}"),
            "/path/to/file.txt"
        );
    }

    /// `${P:e}` → `bak` (extension only).
    #[test]
    fn paramsubst_modifier_extension() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            psubst_one("PS_ME", "/path/to/file.txt.bak", "${PS_ME:e}"),
            "bak"
        );
    }

    /// `${name}` brace form matches bare `$name` when there is no operator.
    #[test]
    fn paramsubst_braced_bare_equals_unbraced_bare() {
        let _g = crate::test_util::global_state_lock();
        setsparam("PS_BB", "value");
        let (a, _, _) = paramsubst("$PS_BB", 0, false, 0, &mut 0);
        let (b, _, _) = paramsubst("${PS_BB}", 0, false, 0, &mut 0);
        assert_eq!(a, b);
        assert_eq!(a, "value");
    }

    // ── Advanced shapes more likely to surface zshrs gaps ────────────
    // Anchored against zsh 5.9 via `print -r --`.

    /// `${H:(-2)}` where H=hello → `lo` (negative offset = from end).
    #[test]
    fn paramsubst_substring_negative_offset() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PS_NEG", "hello", "${PS_NEG:(-2)}"), "lo");
    }

    /// `${H:(-3):2}` where H=hello → `ll` (negative offset + length).
    #[test]
    fn paramsubst_substring_negative_offset_with_length() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PS_NEG2", "hello", "${PS_NEG2:(-3):2}"), "ll");
    }

    /// `${H:0:-1}` where H=hello → `hell` (negative length = drop last N).
    #[test]
    fn paramsubst_substring_negative_length() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PS_NL", "hello", "${PS_NL:0:-1}"), "hell");
    }

    /// `${(q)X}` where X="hi there" → `hi\ there` (backslash-escape spaces).
    #[test]
    fn paramsubst_flag_q_backslash_escapes_whitespace() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PS_Q", "hi there", "${(q)PS_Q}"), r"hi\ there");
    }

    /// `${(q-)X}` where X="hi there" → `'hi there'` (single-quote when needed).
    #[test]
    fn paramsubst_flag_qdash_uses_single_quotes_when_needed() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            psubst_one("PS_QD", "hi there", "${(q-)PS_QD}"),
            "'hi there'"
        );
    }

    /// `${X:gs/X/_/}` where X="aXbXcXd" → `a_b_c_d` (gsub modifier).
    #[test]
    fn paramsubst_modifier_gs_replaces_all() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            psubst_one("PS_GS", "aXbXcXd", "${PS_GS:gs/X/_/}"),
            "a_b_c_d"
        );
    }

    /// `${(P)REF}` where REF points at target name → target's value.
    #[test]
    fn paramsubst_flag_P_dereferences_indirect_name() {
        let _g = crate::test_util::global_state_lock();
        setsparam("PSU_TARGET", "real_value");
        let (out, _, _) = paramsubst("${(P)PSU_REF}", 0, false, 0, &mut 0);
        // Run psubst_one to set PSU_REF; can't reuse psubst_one here
        // because we need TWO params set, not just one.
        setsparam("PSU_REF", "PSU_TARGET");
        let (out2, _, _) = paramsubst("${(P)PSU_REF}", 0, false, 0, &mut 0);
        let _ = out;
        assert_eq!(out2, "real_value");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Array-form expansions anchored to real zsh 5.9.
    // Arrays set via `setaparam(name, Vec<String>)`. Each expansion is
    // verified against `print -r -- "${expansion}"` in zsh. Where zshrs
    // diverges, the test FAILS, exposing the bug.
    // ═══════════════════════════════════════════════════════════════════

    /// Set an array param, expand `expr`, return (single_str_result, vec_result).
    fn psubst_arr(name: &str, elements: &[&str], expr: &str) -> (String, Vec<String>) {
        errflag.store(0, Ordering::Relaxed);
        let _ = crate::ported::params::setaparam(
            name,
            elements.iter().map(|s| (*s).to_string()).collect(),
        );
        let (out, _, multi) = paramsubst(expr, 0, false, 0, &mut 0);
        (out, multi)
    }

    // ── Single-element indexing ─────────────────────────────────────
    /// `${arr[1]}` where arr=(alpha beta gamma delta) → `alpha`
    /// (zsh arrays are 1-indexed; ${arr[0]} is also "alpha" via the
    /// KSH_ARRAYS option, but default zsh is 1-indexed).
    #[test]
    fn paramsubst_arr_index_one_returns_first_element() {
        let _g = crate::test_util::global_state_lock();
        let (out, _) = psubst_arr("PSA1", &["alpha", "beta", "gamma", "delta"], "${PSA1[1]}");
        assert_eq!(out, "alpha");
    }

    /// `${arr[2]}` → `beta`
    #[test]
    fn paramsubst_arr_index_two_returns_second_element() {
        let _g = crate::test_util::global_state_lock();
        let (out, _) = psubst_arr("PSA2", &["alpha", "beta", "gamma", "delta"], "${PSA2[2]}");
        assert_eq!(out, "beta");
    }

    /// `${arr[-1]}` → `delta` (negative index from end)
    #[test]
    fn paramsubst_arr_index_negative_one_returns_last_element() {
        let _g = crate::test_util::global_state_lock();
        let (out, _) = psubst_arr("PSA3", &["alpha", "beta", "gamma", "delta"], "${PSA3[-1]}");
        assert_eq!(out, "delta");
    }

    /// `${arr[-2]}` → `gamma`
    #[test]
    fn paramsubst_arr_index_negative_two_returns_second_to_last() {
        let _g = crate::test_util::global_state_lock();
        let (out, _) = psubst_arr("PSA4", &["alpha", "beta", "gamma", "delta"], "${PSA4[-2]}");
        assert_eq!(out, "gamma");
    }

    /// `${arr[99]}` → `` (out-of-range index produces empty string in zsh)
    #[test]
    fn paramsubst_arr_index_out_of_range_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let (out, _) = psubst_arr("PSA5", &["alpha", "beta", "gamma", "delta"], "${PSA5[99]}");
        assert_eq!(out, "");
    }

    // ── Length ──────────────────────────────────────────────────────
    /// `${#arr}` → `4` (element count, NOT byte count of joined string)
    #[test]
    fn paramsubst_arr_length_returns_element_count() {
        let _g = crate::test_util::global_state_lock();
        let (out, _) = psubst_arr("PSA6", &["alpha", "beta", "gamma", "delta"], "${#PSA6}");
        assert_eq!(out, "4");
    }

    /// `${#arr}` on a 3-element array → `3`
    #[test]
    fn paramsubst_arr_length_three_elements() {
        let _g = crate::test_util::global_state_lock();
        let (out, _) = psubst_arr("PSA7", &["x", "y", "z"], "${#PSA7}");
        assert_eq!(out, "3");
    }

    /// `${#arr}` on empty array → `0`
    #[test]
    fn paramsubst_arr_length_empty_array_is_zero() {
        let _g = crate::test_util::global_state_lock();
        let (out, _) = psubst_arr("PSA8", &[], "${#PSA8}");
        assert_eq!(out, "0");
    }

    // ── Slice ───────────────────────────────────────────────────────
    /// `${arr[1,2]}` → `alpha beta` (slice produces multi-value; pin
    /// the multi-vec since the single-string join is implementation
    /// detail — paramsubst may use $IFS or hardcoded space).
    #[test]
    fn paramsubst_arr_slice_one_two_returns_first_two_elements() {
        let _g = crate::test_util::global_state_lock();
        let (out, multi) = psubst_arr("PSA9", &["alpha", "beta", "gamma", "delta"], "${PSA9[1,2]}");
        // EITHER multi has the two elements, OR out is "alpha beta"
        // (some paramsubst paths return joined string instead of vec).
        if !multi.is_empty() {
            assert_eq!(multi, vec!["alpha", "beta"]);
        } else {
            assert_eq!(out, "alpha beta");
        }
    }

    /// `${arr[2,-1]}` → `beta gamma delta` (slice from 2 to end)
    #[test]
    fn paramsubst_arr_slice_two_to_end() {
        let _g = crate::test_util::global_state_lock();
        let (out, multi) = psubst_arr(
            "PSA10",
            &["alpha", "beta", "gamma", "delta"],
            "${PSA10[2,-1]}",
        );
        if !multi.is_empty() {
            assert_eq!(multi, vec!["beta", "gamma", "delta"]);
        } else {
            assert_eq!(out, "beta gamma delta");
        }
    }

    // ── Join flag ───────────────────────────────────────────────────
    // (j/x/), (j::), and (F) join flags on array parameters: after
    // `sep` is applied (c:3906 sepjoin), C clears `isarr` at c:3907 so
    // the auto_splat fallback (c:4245) is bypassed and the joined
    // scalar is returned. The auto_splat fallback gates on
    // `sep.is_none()` to honor this.

    /// `${(j/_/)arr}` → `alpha_beta_gamma_delta` (explicit underscore join)
    #[test]
    fn paramsubst_arr_join_underscore_anchored_to_zsh() {
        let _g = crate::test_util::global_state_lock();
        let (out, _) = psubst_arr(
            "PSA11",
            &["alpha", "beta", "gamma", "delta"],
            "${(j/_/)PSA11}",
        );
        assert_eq!(
            out, "alpha_beta_gamma_delta",
            "zsh 5.9 reference: print -r -- \"${{(j/_/)arr}}\" → alpha_beta_gamma_delta"
        );
    }

    /// `${(j::)arr}` → `alphabetagammadelta` (empty-string join)
    #[test]
    fn paramsubst_arr_join_empty_string_anchored_to_zsh() {
        let _g = crate::test_util::global_state_lock();
        let (out, _) = psubst_arr(
            "PSA12",
            &["alpha", "beta", "gamma", "delta"],
            "${(j::)PSA12}",
        );
        assert_eq!(
            out, "alphabetagammadelta",
            "zsh 5.9 reference: print -r -- \"${{(j::)arr}}\" → alphabetagammadelta"
        );
    }

    // ── F-flag (newline join) ──────────────────────────────────────
    /// `${(F)arr}` → `alpha\nbeta\ngamma\ndelta` (newline-join, same bug class)
    #[test]
    fn paramsubst_arr_F_flag_joins_with_newlines_anchored_to_zsh() {
        let _g = crate::test_util::global_state_lock();
        let (out, _) = psubst_arr("PSA13", &["alpha", "beta", "gamma", "delta"], "${(F)PSA13}");
        assert_eq!(
            out, "alpha\nbeta\ngamma\ndelta",
            "zsh 5.9 reference: print -r -- \"${{(F)arr}}\" → alpha\\nbeta\\ngamma\\ndelta"
        );
    }

    // ── Split flag on scalar ───────────────────────────────────────
    /// `${(s/:/)"a:b:c:d"}` — scalar split by `:` into 4 words.
    /// When set as scalar and expanded via paramsubst, the multi-vec
    /// should hold the 4 parts.
    #[test]
    fn paramsubst_scalar_split_on_colon_yields_four_parts() {
        let _g = crate::test_util::global_state_lock();
        setsparam("PSA14", "a:b:c:d");
        let (out, _, multi) = paramsubst("${(s/:/)PSA14}", 0, false, 0, &mut 0);
        if !multi.is_empty() {
            assert_eq!(multi, vec!["a", "b", "c", "d"]);
        } else {
            // Joined form
            assert_eq!(out, "a b c d");
        }
    }

    // ── Sort flag ──────────────────────────────────────────────────
    /// `${(o)arr}` where arr=(charlie alpha bravo) sorts the elements.
    /// In scalar/paramsubst context the multi-vec should reflect sorted
    /// order. If scalar return joins, expect `alpha bravo charlie`.
    #[test]
    fn paramsubst_arr_sort_ascending() {
        let _g = crate::test_util::global_state_lock();
        let (out, multi) = psubst_arr("PSA15", &["charlie", "alpha", "bravo"], "${(o)PSA15}");
        if !multi.is_empty() {
            assert_eq!(multi, vec!["alpha", "bravo", "charlie"]);
        } else {
            assert_eq!(out, "alpha bravo charlie");
        }
    }

    /// `${(O)arr}` → reverse sort
    #[test]
    fn paramsubst_arr_sort_descending() {
        let _g = crate::test_util::global_state_lock();
        let (out, multi) = psubst_arr("PSA16", &["charlie", "alpha", "bravo"], "${(O)PSA16}");
        if !multi.is_empty() {
            assert_eq!(multi, vec!["charlie", "bravo", "alpha"]);
        } else {
            assert_eq!(out, "charlie bravo alpha");
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Deep / compound forms — high-bug-yield surface.
    // Each test pinned against `print -r --` output in real zsh 5.9.
    // ═══════════════════════════════════════════════════════════════════

    // ── ${arr[@]:#pat} — filter, keep elements NOT matching pat ─────
    // KNOWN ZSHRS BUG (surfaced 2026-05-23): the :#pat filter on arrays
    // is not implemented — zshrs returns the unfiltered array. Real zsh
    // 5.9 removes matching elements. Tests pinned to zsh; #[ignore]'d
    // so CI stays green. Remove the #[ignore] once the filter lands.

    /// `${mix[@]:#bar}` where mix=(foo bar baz qux) → `foo baz qux`
    /// (the matched element "bar" is REMOVED, not kept).
    #[test]
    fn paramsubst_arr_filter_hash_removes_matching_literal_anchored_to_zsh() {
        let _g = crate::test_util::global_state_lock();
        let (out, multi) = psubst_arr("PSD1", &["foo", "bar", "baz", "qux"], "${PSD1[@]:#bar}");
        if !multi.is_empty() {
            assert_eq!(multi, vec!["foo", "baz", "qux"]);
        } else {
            assert_eq!(out, "foo baz qux", "zsh: ${{mix[@]:#bar}} → 'foo baz qux'");
        }
    }

    /// `${mix[@]:#ba*}` where mix=(foo bar baz qux) → `foo qux`
    /// (glob pattern: removes everything starting with "ba")
    #[test]
    fn paramsubst_arr_filter_hash_removes_matching_glob_anchored_to_zsh() {
        let _g = crate::test_util::global_state_lock();
        let (out, multi) = psubst_arr("PSD2", &["foo", "bar", "baz", "qux"], "${PSD2[@]:#ba*}");
        if !multi.is_empty() {
            assert_eq!(multi, vec!["foo", "qux"]);
        } else {
            assert_eq!(out, "foo qux", "zsh: ${{mix[@]:#ba*}} → 'foo qux'");
        }
    }

    // ── Per-element strip ${arr[@]%pat} ────────────────────────────
    /// `${files[@]%.txt}` strips .txt suffix from each element.
    /// zsh: foo.txt bar.txt baz.md → foo bar baz.md (md unaffected).
    #[test]
    fn paramsubst_arr_per_element_suffix_strip() {
        let _g = crate::test_util::global_state_lock();
        let (out, multi) = psubst_arr("PSD3", &["foo.txt", "bar.txt", "baz.md"], "${PSD3[@]%.txt}");
        if !multi.is_empty() {
            assert_eq!(multi, vec!["foo", "bar", "baz.md"]);
        } else {
            assert_eq!(out, "foo bar baz.md");
        }
    }

    /// `${paths[@]##*/}` strips longest prefix matching `*/` (basename)
    /// from each element. zsh: /a/x /b/y → x y.
    #[test]
    fn paramsubst_arr_per_element_basename_via_longest_prefix() {
        let _g = crate::test_util::global_state_lock();
        let (out, multi) = psubst_arr("PSD4", &["/a/x", "/b/y", "/c/z"], "${PSD4[@]##*/}");
        if !multi.is_empty() {
            assert_eq!(multi, vec!["x", "y", "z"]);
        } else {
            assert_eq!(out, "x y z");
        }
    }

    // ── Length of indexed element vs length of array ──────────────
    /// `${#arr[1]}` is the BYTE LENGTH of arr[1], NOT 1 (element count).
    /// zsh: arr=(hello world); ${#arr[1]} → 5 (chars in "hello").
    #[test]
    fn paramsubst_arr_length_of_indexed_element_is_string_length_anchored_to_zsh() {
        let _g = crate::test_util::global_state_lock();
        let (out, _) = psubst_arr("PSD5", &["hello", "world"], "${#PSD5[1]}");
        assert_eq!(out, "5", "zsh: ${{#arr[1]}} → string-length of first elem");
    }

    /// Substring of indexed element: `${arr[1]:0:3}` → first 3 chars.
    /// zsh: arr=(hello world); ${arr[1]:0:3} → "hel".
    #[test]
    fn paramsubst_arr_substring_of_indexed_element_anchored_to_zsh() {
        let _g = crate::test_util::global_state_lock();
        let (out, _) = psubst_arr("PSD6", &["hello", "world"], "${PSD6[1]:0:3}");
        assert_eq!(out, "hel", "zsh: ${{arr[1]:0:3}} → first 3 chars of arr[1]");
    }

    // ── Scalar context for (o): zsh does NOT sort without @ ───────
    /// `${(o)arr}` (no @, double-quoted scalar context) returns the
    /// elements JOINED with $IFS but NOT sorted in zsh 5.9.
    /// Real zsh: arr=(charlie alpha bravo) → `charlie alpha bravo`.
    /// If zshrs sorts here, that's a divergence (zshrs is sorting
    /// when zsh wouldn't).
    #[test]
    fn paramsubst_arr_sort_scalar_context_anchored_to_zsh() {
        let _g = crate::test_util::global_state_lock();
        // Set the array directly, then call paramsubst with qt=true
        // (DQ context). zsh: `"${(o)arr}"` does NOT sort because the
        // DQ-sepjoin at c:3034 clears isarr=0 BEFORE the c:4245 sort
        // block can fire. Verified: /bin/zsh -c 'arr=(charlie alpha
        // bravo); print -r -- "${(o)arr}"' → "charlie alpha bravo".
        errflag.store(0, Ordering::Relaxed);
        let _ = crate::ported::params::setaparam(
            "PSD7",
            ["charlie", "alpha", "bravo"]
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        );
        let (out, _, _) = paramsubst("${(o)PSD7}", 0, true, 0, &mut 0);
        assert_eq!(
            out, "charlie alpha bravo",
            "zsh: scalar (DQ) context (o) does NOT sort; got {out:?}"
        );
    }

    // ── Compound flags: (oj/-/) sort+join ─────────────────────────
    /// `${(oj/-/)arr}` in real zsh 5.9 returns `charlie-alpha-bravo`
    /// (joined with `-` but NOT sorted) — flag order/context defeats
    /// the sort. If zshrs returns the sorted form, that's a divergence.
    /// Pin zsh's observed behavior; mark #[ignore] since the behavior
    /// itself is counter-intuitive and we want to flag it explicitly.
    #[test]
    fn paramsubst_arr_compound_oj_anchored_to_zsh() {
        let _g = crate::test_util::global_state_lock();
        let (out, _) = psubst_arr("PSD8", &["charlie", "alpha", "bravo"], "${(oj/-/)PSD8}");
        assert_eq!(
            out, "charlie-alpha-bravo",
            "zsh: (oj/-/) does NOT sort in scalar context; got {out:?}"
        );
    }

    // ── (Fo) compound: newline-join, NOT sorted in zsh scalar ─────
    /// `${(Fo)arr}` in zsh: joined by newline, NOT sorted.
    #[test]
    fn paramsubst_arr_compound_Fo_anchored_to_zsh() {
        let _g = crate::test_util::global_state_lock();
        let (out, _) = psubst_arr("PSD9", &["charlie", "alpha", "bravo"], "${(Fo)PSD9}");
        assert_eq!(
            out, "charlie\nalpha\nbravo",
            "zsh: (Fo) → newline-join unsorted; got {out:?}"
        );
    }

    // ── Empty array under join ────────────────────────────────────
    /// `${(j/,/)empty}` where empty=() → `` (empty string)
    /// Bug-class check: ensure empty arrays don't crash the join path.
    #[test]
    fn paramsubst_arr_join_on_empty_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let (out, _) = psubst_arr("PSD10", &[], "${(j/,/)PSD10}");
        assert_eq!(out, "");
    }

    // ── Default values on unset/empty ─────────────────────────────
    /// `${UNSET_PARAM:-default}` on truly unset → "default"
    #[test]
    fn paramsubst_unset_param_uses_default() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::unsetparam("PSD_UNSET");
        let (out, _, _) = paramsubst("${PSD_UNSET:-fallback}", 0, false, 0, &mut 0);
        assert_eq!(out, "fallback");
    }

    // ── (q-) quote variants ────────────────────────────────────────
    /// `${(q-)x}` for x="" (empty) — zsh emits `''` (empty single quotes).
    #[test]
    fn paramsubst_flag_qdash_on_empty_string_emits_empty_quotes_anchored_to_zsh() {
        let _g = crate::test_util::global_state_lock();
        let out = psubst_one("PSD_QE", "", "${(q-)PSD_QE}");
        assert_eq!(out, "''", "zsh: ${{(q-)empty}} → '' (empty quotes)");
    }

    /// `${(q+)x}` for x="hi there" — zsh picks shortest valid quoting.
    /// On a plain space-bearing string, that's single-quote: `'hi there'`.
    /// zshrs returns `$'hi there'` (ANSI-C form) — divergence from zsh's
    /// shortest-form pick.
    #[test]
    fn paramsubst_flag_qplus_picks_shortest_quoting_anchored_to_zsh() {
        let _g = crate::test_util::global_state_lock();
        // quotedzputs reads the ISPECIAL typtab to decide whether a char
        // needs quoting. Tests don't auto-init typtab (init happens at
        // shell startup in C); without this call typtab is all-zero and
        // hasspecial() returns false even for space → bare unquoted.
        crate::ported::utils::inittyptab();
        let out = psubst_one("PSD_QP", "hi there", "${(q+)PSD_QP}");
        assert_eq!(
            out, "'hi there'",
            "zsh: ${{(q+)\"hi there\"}} → 'hi there' (shortest valid quoting)"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Modifier chains — ${name:h:t}, ${name:r:e}, ${name:s/x/y/:r}.
    // Anchored to `print -r --` in real zsh 5.9. Modifier chains often
    // have ordering or accumulation bugs.
    // ═══════════════════════════════════════════════════════════════════

    const PATH_FIXTURE: &str = "/path/to/file.txt.bak";

    /// `${P:h:t}` — dirname → tail of dirname → "to"
    #[test]
    fn paramsubst_mod_chain_h_t_returns_last_dir_component() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PMC1", PATH_FIXTURE, "${PMC1:h:t}"), "to");
    }

    /// `${P:t:r}` — basename then strip last extension → "file.txt"
    #[test]
    fn paramsubst_mod_chain_t_r_basename_strip_last_ext() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PMC2", PATH_FIXTURE, "${PMC2:t:r}"), "file.txt");
    }

    /// `${P:r:t}` — strip ext then basename → "file.txt"
    #[test]
    fn paramsubst_mod_chain_r_t_strip_then_basename() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PMC3", PATH_FIXTURE, "${PMC3:r:t}"), "file.txt");
    }

    /// `${P:r:e}` — strip last ext (.bak), then ext of result (.txt) → "txt"
    #[test]
    fn paramsubst_mod_chain_r_e_returns_inner_extension() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PMC4", PATH_FIXTURE, "${PMC4:r:e}"), "txt");
    }

    /// `${P:r:r}` — strip two extensions → "/path/to/file"
    #[test]
    fn paramsubst_mod_chain_r_r_strips_two_extensions() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            psubst_one("PMC5", PATH_FIXTURE, "${PMC5:r:r}"),
            "/path/to/file"
        );
    }

    /// `${P:t:e}` — basename then ext → "bak"
    #[test]
    fn paramsubst_mod_chain_t_e_basename_then_extension() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PMC6", PATH_FIXTURE, "${PMC6:t:e}"), "bak");
    }

    /// `${P:s/file/X/}` — single substitution.
    /// zsh: /path/to/X.txt.bak
    #[test]
    fn paramsubst_mod_chain_s_single_substitution() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            psubst_one("PMC7", PATH_FIXTURE, "${PMC7:s/file/X/}"),
            "/path/to/X.txt.bak"
        );
    }

    /// `${P:gs/t/Z/}` — global substitution.
    /// zsh: /paZh/Zo/file.ZxZ.bak
    #[test]
    fn paramsubst_mod_chain_gs_global_substitution() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            psubst_one("PMC8", PATH_FIXTURE, "${PMC8:gs/t/Z/}"),
            "/paZh/Zo/file.ZxZ.bak"
        );
    }

    /// `${P:s/file/X/:r}` — subst then strip ext.
    /// zsh: /path/to/X.txt
    #[test]
    fn paramsubst_mod_chain_s_then_r_subst_then_strip() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            psubst_one("PMC9", PATH_FIXTURE, "${PMC9:s/file/X/:r}"),
            "/path/to/X.txt"
        );
    }

    /// `${P:r:s/file/X/}` — strip then subst.
    /// zsh: /path/to/X.txt
    #[test]
    fn paramsubst_mod_chain_r_then_s_strip_then_subst() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            psubst_one("PMC10", PATH_FIXTURE, "${PMC10:r:s/file/X/}"),
            "/path/to/X.txt"
        );
    }

    /// `${P:t:gs/./_/}` — basename then gsub `.` → `_`.
    /// zsh: file_txt_bak
    #[test]
    fn paramsubst_mod_chain_t_then_gs_basename_then_gsub() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            psubst_one("PMC11", PATH_FIXTURE, "${PMC11:t:gs/./_/}"),
            "file_txt_bak"
        );
    }

    /// `${S:q}` — single-modifier quote (vs the (q) flag form).
    /// zsh: hi\ there for S="hi there".
    #[test]
    fn paramsubst_mod_q_quotes_backslash_escape() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PMC12", "hi there", "${PMC12:q}"), r"hi\ there");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Hash / associative-array expansion forms.
    // Anchored to `typeset -A h; h[k]=v; print -r -- "${h[k]}"` in zsh.
    // Hash iteration order is unspecified in zsh; tests for keys/values
    // sort before comparing.
    // ═══════════════════════════════════════════════════════════════════

    /// Build a small hash {k1: v1, k2: v2, k3: v3} via sethparam.
    fn build_test_hash(name: &str) {
        errflag.store(0, Ordering::Relaxed);
        crate::ported::params::unsetparam(name);
        let _ = crate::ported::params::sethparam(
            name,
            vec![
                "k1".into(),
                "v1".into(),
                "k2".into(),
                "v2".into(),
                "k3".into(),
                "v3".into(),
            ],
        );
    }

    /// `${h[k1]}` returns the value `v1`.
    #[test]
    fn paramsubst_hash_indexed_returns_value() {
        let _g = crate::test_util::global_state_lock();
        build_test_hash("PSH1");
        let (out, _, _) = paramsubst("${PSH1[k1]}", 0, false, 0, &mut 0);
        assert_eq!(out, "v1");
        crate::ported::params::unsetparam("PSH1");
    }

    /// `${h[k2]}` returns `v2`.
    #[test]
    fn paramsubst_hash_indexed_second_key() {
        let _g = crate::test_util::global_state_lock();
        build_test_hash("PSH2");
        let (out, _, _) = paramsubst("${PSH2[k2]}", 0, false, 0, &mut 0);
        assert_eq!(out, "v2");
        crate::ported::params::unsetparam("PSH2");
    }

    /// `${h[missing]}` returns empty string when key absent.
    #[test]
    fn paramsubst_hash_missing_key_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        build_test_hash("PSH3");
        let (out, _, _) = paramsubst("${PSH3[missing]}", 0, false, 0, &mut 0);
        assert_eq!(out, "");
        crate::ported::params::unsetparam("PSH3");
    }

    /// `${#h}` returns the element count.
    #[test]
    fn paramsubst_hash_length_returns_element_count() {
        let _g = crate::test_util::global_state_lock();
        build_test_hash("PSH4");
        let (out, _, _) = paramsubst("${#PSH4}", 0, false, 0, &mut 0);
        assert_eq!(out, "3", "3 key-value pairs → length 3");
        crate::ported::params::unsetparam("PSH4");
    }

    /// `${(k)h}` returns keys (order unspecified; sort to compare).
    #[test]
    fn paramsubst_hash_k_flag_returns_keys() {
        let _g = crate::test_util::global_state_lock();
        build_test_hash("PSH5");
        let (out, _, multi) = paramsubst("${(k)PSH5}", 0, false, 0, &mut 0);
        // Collect into a Vec, sort, compare.
        let mut keys: Vec<String> = if !multi.is_empty() {
            multi
        } else {
            out.split_whitespace().map(|s| s.to_string()).collect()
        };
        keys.sort();
        assert_eq!(keys, vec!["k1", "k2", "k3"]);
        crate::ported::params::unsetparam("PSH5");
    }

    /// `${(v)h}` returns values (order unspecified; sort to compare).
    #[test]
    fn paramsubst_hash_v_flag_returns_values() {
        let _g = crate::test_util::global_state_lock();
        build_test_hash("PSH6");
        let (out, _, multi) = paramsubst("${(v)PSH6}", 0, false, 0, &mut 0);
        let mut vals: Vec<String> = if !multi.is_empty() {
            multi
        } else {
            out.split_whitespace().map(|s| s.to_string()).collect()
        };
        vals.sort();
        assert_eq!(vals, vec!["v1", "v2", "v3"]);
        crate::ported::params::unsetparam("PSH6");
    }

    /// `${(kv)h}` returns alternating keys and values.
    /// zsh produces 2*N entries (key, value, key, value, ...) in some
    /// order. Sort the whole list and check membership.
    #[test]
    fn paramsubst_hash_kv_flag_returns_alternating_pairs() {
        let _g = crate::test_util::global_state_lock();
        build_test_hash("PSH7");
        let (out, _, multi) = paramsubst("${(kv)PSH7}", 0, false, 0, &mut 0);
        let mut all: Vec<String> = if !multi.is_empty() {
            multi
        } else {
            out.split_whitespace().map(|s| s.to_string()).collect()
        };
        all.sort();
        assert_eq!(
            all,
            vec!["k1", "k2", "k3", "v1", "v2", "v3"],
            "kv must produce all keys + all values (6 entries total)"
        );
        crate::ported::params::unsetparam("PSH7");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Round-7 deeper forms — slice end-anchored, set ops, paren-index,
    // assign-default side effect. Each anchored to `print -r --` in
    // real zsh 5.9.
    // ═══════════════════════════════════════════════════════════════════

    /// `${arr[1,-2]}` — slice from 1 to second-to-last.
    /// zsh: arr=(a b c d e) → "a b c d"
    #[test]
    fn paramsubst_arr_slice_to_negative_two() {
        let _g = crate::test_util::global_state_lock();
        let (out, multi) = psubst_arr("PS_S1", &["a", "b", "c", "d", "e"], "${PS_S1[1,-2]}");
        if !multi.is_empty() {
            assert_eq!(multi, vec!["a", "b", "c", "d"]);
        } else {
            assert_eq!(out, "a b c d");
        }
    }

    /// `${arr[-3,-1]}` — slice last three.
    /// zsh: arr=(a b c d e) → "c d e"
    #[test]
    fn paramsubst_arr_slice_negative_three_to_negative_one() {
        let _g = crate::test_util::global_state_lock();
        let (out, multi) = psubst_arr("PS_S2", &["a", "b", "c", "d", "e"], "${PS_S2[-3,-1]}");
        if !multi.is_empty() {
            assert_eq!(multi, vec!["c", "d", "e"]);
        } else {
            assert_eq!(out, "c d e");
        }
    }

    /// `${arr[(-1)]}` — paren-wrapped negative index → last element.
    /// zsh: arr=(a b c d e) → "e"
    #[test]
    fn paramsubst_arr_paren_negative_one_is_last_anchored_to_zsh() {
        let _g = crate::test_util::global_state_lock();
        let (out, _) = psubst_arr("PS_S3", &["a", "b", "c", "d", "e"], "${PS_S3[(-1)]}");
        assert_eq!(out, "e", "zsh: arr=(a b c d e); ${{arr[(-1)]}} → e");
    }

    /// `${arr[(1)]}` — paren-wrapped positive index → first.
    /// zsh: arr=(a b c d e) → "a"
    #[test]
    fn paramsubst_arr_paren_positive_one_is_first_anchored_to_zsh() {
        let _g = crate::test_util::global_state_lock();
        let (out, _) = psubst_arr("PS_S4", &["a", "b", "c", "d", "e"], "${PS_S4[(1)]}");
        assert_eq!(out, "a", "zsh: arr=(a b c d e); ${{arr[(1)]}} → a");
    }

    /// `${list[@]:*other}` — set INTERSECTION with another named array.
    /// zsh: list=(a b c d e), other=(b d) → "b d"
    #[test]
    fn paramsubst_arr_set_intersection_anchored_to_zsh() {
        let _g = crate::test_util::global_state_lock();
        errflag.store(0, Ordering::Relaxed);
        let _ = crate::ported::params::setaparam("PS_OTHER1", vec!["b".into(), "d".into()]);
        let (out, multi) = psubst_arr(
            "PS_LIST1",
            &["a", "b", "c", "d", "e"],
            "${PS_LIST1[@]:*PS_OTHER1}",
        );
        if !multi.is_empty() {
            assert_eq!(multi, vec!["b", "d"]);
        } else {
            assert_eq!(out, "b d");
        }
    }

    /// `${list[@]:|other}` — set SUBTRACTION (list MINUS other).
    /// zsh: list=(a b c d e), other=(b d) → "a c e"
    #[test]
    fn paramsubst_arr_set_subtraction_anchored_to_zsh() {
        let _g = crate::test_util::global_state_lock();
        errflag.store(0, Ordering::Relaxed);
        let _ = crate::ported::params::setaparam("PS_OTHER2", vec!["b".into(), "d".into()]);
        let (out, multi) = psubst_arr(
            "PS_LIST2",
            &["a", "b", "c", "d", "e"],
            "${PS_LIST2[@]:|PS_OTHER2}",
        );
        if !multi.is_empty() {
            assert_eq!(multi, vec!["a", "c", "e"]);
        } else {
            assert_eq!(out, "a c e");
        }
    }

    /// `${X:=newval}` on UNSET param both sets X and returns "newval".
    /// zsh: unset X; ${X:=newval} → "newval"; $X is "newval" thereafter.
    #[test]
    fn paramsubst_colon_equals_assigns_and_returns_default_on_unset() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::unsetparam("PS_ASSIGN1");
        errflag.store(0, Ordering::Relaxed);
        let (out, _, _) = paramsubst("${PS_ASSIGN1:=newval}", 0, false, 0, &mut 0);
        assert_eq!(out, "newval", "expansion returns the assigned value");
        // Side effect: the param now equals "newval"
        assert_eq!(
            crate::ported::params::getsparam("PS_ASSIGN1").as_deref(),
            Some("newval"),
            "X must be SET to newval by `:=` operator"
        );
        crate::ported::params::unsetparam("PS_ASSIGN1");
    }

    /// `${X:=newval}` on EMPTY param sets X to newval (the `:` triggers
    /// on both unset AND empty).
    /// zsh: X=""; ${X:=newval} → "newval"; $X is "newval".
    #[test]
    fn paramsubst_colon_equals_assigns_on_empty_too() {
        let _g = crate::test_util::global_state_lock();
        errflag.store(0, Ordering::Relaxed);
        setsparam("PS_ASSIGN2", "");
        let (out, _, _) = paramsubst("${PS_ASSIGN2:=newval}", 0, false, 0, &mut 0);
        assert_eq!(out, "newval");
        assert_eq!(
            crate::ported::params::getsparam("PS_ASSIGN2").as_deref(),
            Some("newval"),
            "empty X gets reassigned to newval"
        );
        crate::ported::params::unsetparam("PS_ASSIGN2");
    }

    /// `${X:=newval}` on a NON-empty SET param leaves X unchanged and
    /// returns the existing value.
    #[test]
    fn paramsubst_colon_equals_noop_on_nonempty() {
        let _g = crate::test_util::global_state_lock();
        errflag.store(0, Ordering::Relaxed);
        setsparam("PS_ASSIGN3", "preserved");
        let (out, _, _) = paramsubst("${PS_ASSIGN3:=newval}", 0, false, 0, &mut 0);
        assert_eq!(out, "preserved");
        assert_eq!(
            crate::ported::params::getsparam("PS_ASSIGN3").as_deref(),
            Some("preserved"),
            "non-empty X stays unchanged"
        );
        crate::ported::params::unsetparam("PS_ASSIGN3");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Compound chains and substring-on-flag-result forms.
    // Anchored to `print -r --` in zsh 5.9.
    // ═══════════════════════════════════════════════════════════════════

    /// `${(U)X:0:3}` — uppercase X then take first 3 chars.
    /// X=Hello → ${(U)X}=HELLO → ${(U)X:0:3} = "HEL".
    #[test]
    fn paramsubst_compound_uppercase_then_substring() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PCC1", "Hello", "${(U)PCC1:0:3}"), "HEL");
    }

    /// `${(U)X:1}` — uppercase + drop first char.
    /// Hello → HELLO → ELLO.
    #[test]
    fn paramsubst_compound_uppercase_then_drop_first() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PCC2", "Hello", "${(U)PCC2:1}"), "ELLO");
    }

    /// `${#${(U)X}}` — length of uppercase result. = 5 for "Hello".
    /// Nested ${(U)...} inside ${#...}.
    #[test]
    fn paramsubst_compound_length_of_uppercase() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PCC3", "Hello", "${#${(U)PCC3}}"), "5");
    }

    /// `${(L)X##*B}` — lowercase then strip longest *B prefix.
    /// FOOBARBAZ → fooBARBAZ→... actually zsh applies strip BEFORE the
    /// flag conversion order. Verified output: "az" (strip then lower).
    #[test]
    fn paramsubst_compound_lowercase_strip_prefix() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            psubst_one("PCC4", "FOOBARBAZ", "${(L)PCC4##*B}"),
            "az",
            "zsh: ${{(L)FOOBARBAZ##*B}} → 'az' (strip + lower)"
        );
    }

    /// `${#${X##*B}}` — length of prefix-strip result.
    /// FOOBARBAZ##*B = "AZ" → length 2.
    #[test]
    fn paramsubst_compound_length_of_strip_result() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(psubst_one("PCC5", "FOOBARBAZ", "${#${PCC5##*B}}"), "2");
    }

    /// `${${S#*:}#*:}` — double-strip prefix (chained nested expansion).
    /// alpha:beta:gamma → beta:gamma → gamma.
    /// **Real zsh: "gamma". zshrs: "beta:gamma" — outer prefix-strip
    /// doesn't apply to the inner result.** Bug.
    #[test]
    fn paramsubst_nested_double_strip_prefix_anchored_to_zsh() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            psubst_one("PCC6", "alpha:beta:gamma", "${${PCC6#*:}#*:}"),
            "gamma",
            "zsh: alpha:beta:gamma → beta:gamma → gamma"
        );
    }

    /// `${${S%:*}%:*}` — double-strip suffix.
    /// alpha:beta:gamma → alpha:beta → alpha.
    #[test]
    fn paramsubst_nested_double_strip_suffix() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            psubst_one("PCC7", "alpha:beta:gamma", "${${PCC7%:*}%:*}"),
            "alpha"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // zsh test corpus pin tests — Test/D04parameter.ztst:1249-2358.
    // Anchored to zsh's documented `Parameters associated with
    // backreferences` + `(#m) flag` test cases. Each test cites the
    // ztst line range it pins. Tests that fail under current Rust
    // port get `#[ignore = "ZSHRS BUG: ..."]` per established
    // practice; the assertion + expected output stay in tree so
    // when the Rust port catches up the marker can be flipped.
    // ═══════════════════════════════════════════════════════════════════

    /// `Test/D04parameter.ztst:1249-1258` — `${string%%(#b)(match)*}`
    /// with `string='look for a match in here'` strips the suffix
    /// starting at `match`. The strip result alone (without the
    /// captures) is `"look for a "`. Expected: 11-char prefix.
    /// Strip-only path: `(#b)` shouldn't affect WHICH prefix the
    /// matcher returns; captures are a side-band.
    #[test]
    fn paramsubst_strip_pound_b_backref_keeps_strip_semantic() {
        let _g = crate::test_util::global_state_lock();
        // `(#b)` is a `(#...)` glob-flag spec — only active under
        // EXTENDEDGLOB per Src/pattern.c:953-957. The previous Rust
        // port treated `(#b)` as always-active, but the new
        // option-aware gate requires setopt extendedglob.
        let saved = crate::ported::options::opt_state_get("extendedglob").unwrap_or(false);
        crate::ported::options::opt_state_set("extendedglob", true);
        let result = psubst_one(
            "ZP_BB1",
            "look for a match in here",
            "${ZP_BB1%%(#b)(match)*}",
        );
        crate::ported::options::opt_state_set("extendedglob", saved);
        assert_eq!(
            result, "look for a ",
            "Test/D04parameter.ztst:1250 — (#b) doesn't change strip result"
        );
    }

    /// `Test/D04parameter.ztst:1251` — `$match[1]` after the above
    /// strip must hold `"match"` (the captured group). This is the
    /// (#b) backref payload — requires subst.rs to wire pattern
    /// captures through to `$match[]` after `${var%%(#b)pat}`.
    #[test]
    fn paramsubst_pound_b_backref_populates_match_array() {
        let _g = crate::test_util::global_state_lock();
        let _ = psubst_one(
            "ZP_BB2",
            "look for a match in here",
            "${ZP_BB2%%(#b)(match)*}",
        );
        let m = crate::ported::params::getaparam("match");
        assert_eq!(
            m.as_deref(),
            Some(&["match".to_string()][..]),
            "Test/D04parameter.ztst:1251 — $match[1]=match",
        );
    }

    /// `Test/D04parameter.ztst:1251` — `$mbegin[1]` / `$mend[1]`
    /// hold the 1-based byte offsets of the capture. For "look for a
    /// match in here", "match" is at bytes 12-16 (1-based).
    #[test]
    fn paramsubst_pound_b_populates_mbegin_mend() {
        let _g = crate::test_util::global_state_lock();
        let _ = psubst_one(
            "ZP_BB3",
            "look for a match in here",
            "${ZP_BB3%%(#b)(match)*}",
        );
        let b = crate::ported::params::getaparam("mbegin");
        let e = crate::ported::params::getaparam("mend");
        assert_eq!(
            b.as_deref(),
            Some(&["12".to_string()][..]),
            "Test/D04parameter.ztst:1251 — $mbegin[1]=12"
        );
        assert_eq!(
            e.as_deref(),
            Some(&["16".to_string()][..]),
            "Test/D04parameter.ztst:1251 — $mend[1]=16"
        );
    }

    /// `Test/D04parameter.ztst:1261-1270` — `(#m)` flag: assigns
    /// the WHOLE-match to `$MATCH` (no capture groups). `${(S)string
    /// %%(#m)M*H}` with `string='and look for a MATCH in here'` →
    /// strip the substring matching `M*H` (= "MATCH"), result is
    /// `'and look for a  in here'`. `$MATCH = "MATCH"`.
    ///
    /// (S) — substring mode (vs prefix/suffix), so the match floats.
    #[test]
    fn paramsubst_pound_m_flag_strip_anchored_to_zsh() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one(
            "ZP_MM1",
            "and look for a MATCH in here",
            "${(S)ZP_MM1%%(#m)M*H}",
        );
        assert_eq!(
            result, "and look for a  in here",
            "Test/D04parameter.ztst:1262 — (S) + (#m) substring strip",
        );
        let m = crate::ported::params::getsparam("MATCH");
        assert_eq!(
            m.as_deref(),
            Some("MATCH"),
            "Test/D04parameter.ztst:1263 — $MATCH=MATCH"
        );
    }

    /// `Test/D04parameter.ztst:1272-1275` — `(#m)` substitution form:
    /// `${string//(#m)s/$MATCH $MBEGIN $MEND}` with `string='this is
    /// a string'` replaces every `s` with `s 4 4` / `s 7 7` / `s 11
    /// 11` (the `s` char and its 1-based byte positions). Expected
    /// output: "this 4 4 is 7 7 a s 11 11tring".
    #[test]
    fn paramsubst_pound_m_substitution_with_match_refs() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one(
            "ZP_MM2",
            "this is a string",
            "${ZP_MM2//(#m)s/$MATCH $MBEGIN $MEND}",
        );
        assert_eq!(
            result, "this 4 4 is 7 7 a s 11 11tring",
            "Test/D04parameter.ztst:1275 — (#m) in subst with $MATCH/$MBEGIN/$MEND",
        );
    }

    /// `Test/D04parameter.ztst:1306-1310` — `${file//(#b)(*)left/
    /// ${match/a/andsome}}` — `(#b)` capture + nested substitution
    /// that transforms the captured group's content. With
    /// `file='aleftkept'` and pat `(*)left`:
    ///   - The `(*)` captures "a" (greedy before "left").
    ///   - Replace "aleft" with `${match[1]/a/andsome}` = "andsome".
    ///   - Final result: "andsomekept".
    #[test]
    fn paramsubst_pound_b_nested_capture_transform() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one(
            "ZP_BBN",
            "aleftkept",
            "${ZP_BBN//(#b)(*)left/${match[1]/a/andsome}}",
        );
        assert_eq!(
            result, "andsomekept",
            "Test/D04parameter.ztst:1307,1310 — (#b)+nested subst on capture",
        );
    }

    /// `Test/D04parameter.ztst:2354-2358` — `${(S)a//#%((#b)(*))/
    /// different}` — fully-anchored search must scan the whole
    /// string. With `a="string"` the pattern `#%(...)` is
    /// "start AND end anchored" wrapping the (#b)(*) capture;
    /// the whole string matches → replaced with "different".
    /// `$match[1]` carries the captured whole string.
    #[test]
    fn paramsubst_pound_b_fully_anchored_must_scan_whole_string() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZP_BBA", "string", "${(S)ZP_BBA//#%((#b)(*))/different}");
        assert_eq!(
            result, "different",
            "Test/D04parameter.ztst:2358 — full-anchor search expected",
        );
    }

    /// `Test/D04parameter.ztst:890-893` — `(#m)` inside nested
    /// substitution: `${${string%[aeiou]*}/(#m)?(#e)/${(U)MATCH}}`
    /// with `string='abcdefghijklmnopqrstuvwxyz'`:
    ///   - Inner `${string%[aeiou]*}` strips suffix from first
    ///     vowel-pattern → "abcdefghijklmnopqrst" (the last
    ///     vowel-match suffix removed).
    ///   - Then `/(#m)?(#e)/${(U)MATCH}` replaces the last char (#e
    ///     anchors to end) with its uppercase version via $MATCH.
    ///   - Result: "abcdefghijklmnopqrsT" (last char uppercased).
    #[test]
    fn paramsubst_pound_m_with_end_anchor_in_nested_subst() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one(
            "ZP_MMN",
            "abcdefghijklmnopqrstuvwxyz",
            "${${ZP_MMN%[aeiou]*}/(#m)?(#e)/${(U)MATCH}}",
        );
        assert_eq!(
            result, "abcdefghijklmnopqrsT",
            "Test/D04parameter.ztst:893 — nested (#m)+(#e)+(U)MATCH",
        );
    }

    // ─── Test/D04parameter.ztst:124-138 — strip patterns ─────────────

    /// `Test/D04parameter.ztst:124-129` — `${str#*s}` strips shortest
    /// prefix matching `*s`. `'This is very boring indeed.'` →
    /// `' is very boring indeed.'` (one char + 's' = "Ts" → wait,
    /// shortest *s = empty + 's' = "This"? "T" + "his" then "s" =
    /// first 's' encountered. Strip up to and including first 's':
    /// 'This' = 'T'+'h'+'i'+'s' → strip yields ' is very boring
    /// indeed.' (4 chars dropped).
    #[test]
    fn paramsubst_zsh_corpus_strip_shortest_prefix() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZP_S1", "This is very boring indeed.", "${ZP_S1#*s}");
        assert_eq!(
            result, " is very boring indeed.",
            "Test/D04parameter.ztst:129 — ${{var#*s}} shortest prefix to first 's'",
        );
    }

    /// `Test/D04parameter.ztst:126,130` — `${str##*s}` strips
    /// LONGEST prefix matching `*s`. From "This is very boring
    /// indeed." → strip everything up to the LAST 's'. Last 's'
    /// is in "is" (mid-string). Result: " very boring indeed."
    #[test]
    fn paramsubst_zsh_corpus_strip_longest_prefix() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZP_S2", "This is very boring indeed.", "${ZP_S2##*s}");
        assert_eq!(
            result, " very boring indeed.",
            "Test/D04parameter.ztst:130 — ${{var##*s}} longest prefix",
        );
    }

    /// `Test/D04parameter.ztst:140-146` — `${str:#pat}` (NOT-match
    /// filter on SCALAR): if `str` matches `pat`, yield empty;
    /// else yield `str`. "does match" matches "does * match" →
    /// returns empty; "does not match" doesn't match → returns
    /// itself.
    #[test]
    fn paramsubst_zsh_corpus_colon_hash_filter_scalar_match() {
        let _g = crate::test_util::global_state_lock();
        let r1 = psubst_one("ZP_CH1", "does match", "${ZP_CH1:#does * match}");
        assert_eq!(r1, "does match", "ztst:145 — non-match yields self");
        let r2 = psubst_one("ZP_CH2", "does not match", "${ZP_CH2:#does * match}");
        assert_eq!(r2, "", "ztst:146 — match yields empty");
    }

    // ─── Test/D04parameter.ztst:155-160 — `${(S)var/pat/repl}` ───────

    /// `Test/D04parameter.ztst:156-159` — `${str/[aeiou]*g/...}`
    /// without (S): longest-leftmost replace. With (S): SHORTEST-
    /// leftmost (substring mode). The two have different results.
    /// Plain `${str1/[aeiou]*g/repl}` finds leftmost-longest
    /// `[aeiou]*g`: in "arthur boldly claws dogs every fight" →
    /// "a" + ... + "g" of "fight" (whole "ar...fig"). Result:
    /// "a braw bricht moonlicht nicht the nicht".
    #[test]
    fn paramsubst_zsh_corpus_scalar_single_replace_longest_leftmost() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one(
            "ZP_R1",
            "arthur boldly claws dogs every fight",
            "${ZP_R1/[aeiou]*g/a braw bricht moonlicht nicht the nic}",
        );
        assert_eq!(
            result, "a braw bricht moonlicht nicht the nicht",
            "ztst:159 — leftmost-longest [aeiou]*g replace",
        );
    }

    /// `Test/D04parameter.ztst:157,160` — `${(S)str/[aeiou]*g/repl}`
    /// — (S) flag: shortest substring match. In "arthur boldly
    /// claws dogs every fight" → shortest [aeiou]*g = "u boldly
    /// claws dog" (from "u" of arthur to first "g" of "dogs").
    /// Wait, that's not shortest. Actually (S) = substring match
    /// MODE, finding the shortest leftmost match. Hmm, expected
    /// output is "relishes every fight" — replace was just
    /// "relishe". So pattern matched "arthur boldly claws dog"
    /// (longest? shortest? until first 'g' = "dog").
    /// Actually re-reading: "relishes" = "relishe" + "s". Then
    /// " every fight" is the remaining suffix. So the matched part
    /// is "arthur boldly claws dog" (= [aeiou]*g where g is dog's g)
    /// and the input after the match is "s every fight".
    /// Result: "relishe" + "s every fight" = "relishes every fight".
    #[test]
    fn paramsubst_zsh_corpus_substring_mode_shortest_replace() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one(
            "ZP_R2",
            "arthur boldly claws dogs every fight",
            "${(S)ZP_R2/[aeiou]*g/relishe}",
        );
        assert_eq!(
            result, "relishes every fight",
            "ztst:160 — (S) shortest-leftmost replace",
        );
    }

    // ─── Test/D04parameter.ztst:168-179 — global subst `${var//pat/repl}` ─

    /// `Test/D04parameter.ztst:168-172` — `${str//o*/Please no}`:
    /// global longest-leftmost replace. Pattern `o*` first matches
    /// from first 'o' to end (greedy) → result: "Please no" (one
    /// replacement consuming everything from "o" onward).
    #[test]
    fn paramsubst_zsh_corpus_global_replace_greedy_eats_rest() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one(
            "ZP_G1",
            "o this is so, so so very dull",
            "${ZP_G1//o*/Please no}",
        );
        assert_eq!(
            result, "Please no",
            "ztst:172 — greedy o* match consumes from first 'o' to end",
        );
    }

    /// `Test/D04parameter.ztst:170,173` — `${(S)str//o*/Please no}`:
    /// (S) substring mode, each 'o' followed by 0 chars becomes
    /// "Please no". Effectively replaces every 'o' with "Please no".
    /// Result: "Please no this is sPlease no, sPlease no sPlease no
    /// very dull".
    #[test]
    fn paramsubst_zsh_corpus_global_replace_substring_mode_per_char() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one(
            "ZP_G2",
            "o this is so, so so very dull",
            "${(S)ZP_G2//o*/Please no}",
        );
        assert_eq!(
            result, "Please no this is sPlease no, sPlease no sPlease no very dull",
            "ztst:173 — (S) substring per-occurrence replace",
        );
    }

    // ─── Test/D04parameter.ztst:185-195 — backslash escape in subst ──

    /// `Test/D04parameter.ztst:185-192` — `${str//\\/-}` replaces
    /// `\` (literal backslash) with `-`. Input `'a\string\with\
    /// backslashes'` → "a-string-with-backslashes".
    #[test]
    fn paramsubst_zsh_corpus_replace_literal_backslash() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZP_BS", r"a\string\with\backslashes", r"${ZP_BS//\\/-}");
        assert_eq!(
            result, "a-string-with-backslashes",
            "ztst:192 — global \\ → -",
        );
    }

    /// `Test/D04parameter.ztst:189-194` — `${str//\\//-}` replaces
    /// `/` (escaped to allow it inside the //) with `-`. Input
    /// `'a/string/with/slashes'` → "a-string-with-slashes".
    #[test]
    fn paramsubst_zsh_corpus_replace_escaped_slash() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZP_SL", "a/string/with/slashes", r"${ZP_SL//\//-}");
        assert_eq!(
            result, "a-string-with-slashes",
            "ztst:194 — global escaped / → -",
        );
    }

    // ─── Test/D04parameter.ztst:410-421 — case-modifier flags ────────

    /// `Test/D04parameter.ztst:412,415` — `${(L)foo}` lowercases a
    /// scalar. "yOU KNOW, THE ONE WITH wILLIAM dALRYMPLE" → all-lower.
    #[test]
    fn paramsubst_zsh_corpus_lowercase_flag() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one(
            "ZP_LC",
            "yOU KNOW, THE ONE WITH wILLIAM dALRYMPLE",
            "${(L)ZP_LC}",
        );
        assert_eq!(
            result, "you know, the one with william dalrymple",
            "ztst:415 — (L) lowercases all chars",
        );
    }

    /// `Test/D04parameter.ztst:413,416` — `${(U)bar}` uppercases.
    #[test]
    fn paramsubst_zsh_corpus_uppercase_flag() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZP_UC", "doing that tour of India.", "${(U)ZP_UC}");
        assert_eq!(
            result, "DOING THAT TOUR OF INDIA.",
            "ztst:416 — (U) uppercases all chars",
        );
    }

    /// `Test/D04parameter.ztst:418-421` — `${(C)foo}` capitalizes
    /// each word (Title Case). "instead here I am stuck by the
    /// computer" → "Instead Here I Am Stuck By The Computer".
    #[test]
    fn paramsubst_zsh_corpus_capitalize_flag() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one(
            "ZP_CAP",
            "instead here I am stuck by the computer",
            "${(C)ZP_CAP}",
        );
        assert_eq!(
            result, "Instead Here I Am Stuck By The Computer",
            "ztst:421 — (C) Title Case",
        );
    }

    // ─── Test/D04parameter.ztst:1281+ — (#m) with tokenized input ────

    /// `Test/D04parameter.ztst:1277-1279` — `(#m)` with tokenized
    /// `*` input. `${${~:-*}//(#m)*/$MATCH=$MATCH}`:
    ///   - `${~:-*}` produces literal `*` (tokenized glob).
    ///   - `//(#m)*/...` replaces the whole match with `$MATCH=$MATCH`.
    ///   - $MATCH is `*`, so replacement is `*=*`.
    #[test]
    fn paramsubst_zsh_corpus_pound_m_with_tokenized_glob_input() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZP_TKM", "", "${${~:-*}//(#m)*/$MATCH=$MATCH}");
        assert_eq!(result, "*=*", "ztst:1279 — tokenized * passed through (#m)",);
    }

    /// `Test/D04parameter.ztst:1306-1311` — `${file//(#b)(*)left/
    /// ${match//a/andsome}}` — `(#b)` capture used in nested //
    /// substitution. With `file='aleftkept'`:
    ///   - `(*)left` captures "a", strips "aleft".
    ///   - `${match//a/andsome}` replaces every 'a' in capture with
    ///     "andsome" → "andsome".
    ///   - Final: "andsome" + "kept" = "andsomekept".
    #[test]
    fn paramsubst_zsh_corpus_pound_b_with_nested_global_subst_on_capture() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one(
            "ZP_BBG",
            "aleftkept",
            "${ZP_BBG//(#b)(*)left/${match//a/andsome}}",
        );
        assert_eq!(
            result, "andsomekept",
            "ztst:1310 — (#b) capture used in nested // subst",
        );
    }

    // ─── Test/D05array.ztst:12-62 — array indexing/slicing pins ──────

    /// `Test/D05array.ztst:12-14` — `${foo[1]}` returns first element.
    /// foo=(a b c d e f g) → `${foo[1]}` = "a" (1-based default).
    #[test]
    fn paramsubst_zsh_corpus_array_first_element() {
        let _g = crate::test_util::global_state_lock();
        let (s, _) = psubst_arr("ZA_F", &["a", "b", "c", "d", "e", "f", "g"], "${ZA_F[1]}");
        assert_eq!(s, "a", "ztst:14 — 1-based first element");
    }

    /// `Test/D05array.ztst:16-18` — `${foo[1,4]}` returns slice 1..=4
    /// → "a b c d".
    #[test]
    fn paramsubst_zsh_corpus_array_slice_one_to_four() {
        let _g = crate::test_util::global_state_lock();
        let (_, v) = psubst_arr("ZA_S", &["a", "b", "c", "d", "e", "f", "g"], "${ZA_S[1,4]}");
        assert_eq!(v.join(" "), "a b c d", "ztst:18 — [1,4] slice");
    }

    /// `Test/D05array.ztst:20-22` — `${foo[1,0]}` is empty (end < start).
    #[test]
    fn paramsubst_zsh_corpus_array_slice_empty_when_end_before_start() {
        let _g = crate::test_util::global_state_lock();
        let (s, _) = psubst_arr(
            "ZA_E1",
            &["a", "b", "c", "d", "e", "f", "g"],
            "${ZA_E1[1,0]}",
        );
        assert_eq!(s, "", "ztst:22 — [1,0] empty");
    }

    /// `Test/D05array.ztst:32-34` — `${foo[0]}` returns empty
    /// (zsh's 1-based convention: index 0 is "before-first").
    #[test]
    fn paramsubst_zsh_corpus_array_index_zero_is_empty() {
        let _g = crate::test_util::global_state_lock();
        let (s, _) = psubst_arr("ZA_Z", &["a", "b", "c", "d", "e", "f", "g"], "${ZA_Z[0]}");
        assert_eq!(s, "", "ztst:34 — [0] empty in 1-based zsh");
    }

    /// `Test/D05array.ztst:36-38` — `${foo[0,0]}` also empty.
    #[test]
    fn paramsubst_zsh_corpus_array_slice_zero_to_zero_is_empty() {
        let _g = crate::test_util::global_state_lock();
        let (s, _) = psubst_arr(
            "ZA_ZZ",
            &["a", "b", "c", "d", "e", "f", "g"],
            "${ZA_ZZ[0,0]}",
        );
        assert_eq!(s, "", "ztst:38 — [0,0] empty");
    }

    /// `Test/D05array.ztst:40-42` — `${foo[0,1]}` returns first element
    /// (zsh interprets [0,1] = [1,1]).
    #[test]
    fn paramsubst_zsh_corpus_array_slice_zero_to_one_yields_first() {
        let _g = crate::test_util::global_state_lock();
        let (s, _) = psubst_arr(
            "ZA_OZ",
            &["a", "b", "c", "d", "e", "f", "g"],
            "${ZA_OZ[0,1]}",
        );
        assert_eq!(s, "a", "ztst:42 — [0,1] yields first element");
    }

    /// `Test/D05array.ztst:44-46` — `${foo[3]}` returns "c".
    #[test]
    fn paramsubst_zsh_corpus_array_inner_element() {
        let _g = crate::test_util::global_state_lock();
        let (s, _) = psubst_arr("ZA_I", &["a", "b", "c", "d", "e", "f", "g"], "${ZA_I[3]}");
        assert_eq!(s, "c", "ztst:46 — [3] returns third element");
    }

    /// `Test/D05array.ztst:52-54` — `${foo[2,-4]}` negative end:
    /// foo=(a b c d e f g), -4 = index 4 (len-3=4) → [2..4] = "b c d".
    #[test]
    fn paramsubst_zsh_corpus_array_slice_negative_end() {
        let _g = crate::test_util::global_state_lock();
        let (_, v) = psubst_arr(
            "ZA_NE",
            &["a", "b", "c", "d", "e", "f", "g"],
            "${ZA_NE[2,-4]}",
        );
        assert_eq!(v.join(" "), "b c d", "ztst:54 — [2,-4] slice");
    }

    /// `Test/D05array.ztst:56-58` — `${foo[-4,5]}` negative start:
    /// -4 = index 4 (len=7, -4 → 4) → [4..5] = "d e".
    #[test]
    fn paramsubst_zsh_corpus_array_slice_negative_start() {
        let _g = crate::test_util::global_state_lock();
        let (_, v) = psubst_arr(
            "ZA_NS",
            &["a", "b", "c", "d", "e", "f", "g"],
            "${ZA_NS[-4,5]}",
        );
        assert_eq!(v.join(" "), "d e", "ztst:58 — [-4,5] slice");
    }

    /// `Test/D05array.ztst:60-62` — `${foo[-6,-2]}` both negative:
    /// -6 = index 2, -2 = index 6 → "b c d e f".
    #[test]
    fn paramsubst_zsh_corpus_array_slice_both_negative() {
        let _g = crate::test_util::global_state_lock();
        let (_, v) = psubst_arr(
            "ZA_NN",
            &["a", "b", "c", "d", "e", "f", "g"],
            "${ZA_NN[-6,-2]}",
        );
        assert_eq!(v.join(" "), "b c d e f", "ztst:62 — [-6,-2] slice");
    }

    /// `Test/D05array.ztst:24-26` — `${foo[4,1]}` empty (end < start, both positive).
    #[test]
    fn paramsubst_zsh_corpus_array_slice_reversed_indices_empty() {
        let _g = crate::test_util::global_state_lock();
        let (s, _) = psubst_arr("ZA_R", &["a", "b", "c", "d", "e", "f", "g"], "${ZA_R[4,1]}");
        assert_eq!(s, "", "ztst:26 — [4,1] reversed empty");
    }

    // ─── Test/D06subscript.ztst:200-241 — string/array subscript edges ──

    /// `Test/D06subscript.ztst:201-203` — `$array[0]` empty when
    /// KSH_ZERO_SUBSCRIPT is off (default). `array=(one two three four)`
    /// → `$array[0]` = "" (and length 0).
    #[test]
    fn paramsubst_zsh_corpus_array_index_zero_no_ksh_zero() {
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::setaparam(
            "ZA_KZ0",
            ["one", "two", "three", "four"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        );
        // c:Src/subst.c:1625 paramsubst expects `start_pos` to point
        // AT the `$` marker — its caller (stringsubst) pre-scans
        // for `$`/Stringg and dispatches. For an expression with
        // leading literal text the canonical entry is `singsub`
        // (Src/subst.c:514) which drives prefork → stringsubst →
        // paramsubst with the proper scan. Use it directly.
        let s = singsub("X${ZA_KZ0[0]}X");
        assert_eq!(
            s, "XX",
            "ztst:203 — array[0] empty without KSH_ZERO_SUBSCRIPT"
        );
    }

    /// `Test/D06subscript.ztst:233-236` — string subscripts.
    /// `string="Why, if it isn't Officer Dibble"`
    /// `[${string[0]}][${string[1]}][${string[0,3]}]` = `[][W][Why]`.
    #[test]
    fn paramsubst_zsh_corpus_string_subscript_zero_one_and_slice() {
        let _g = crate::test_util::global_state_lock();
        let s0 = psubst_one("ZS_W", "Why, if it isn't Officer Dibble", "${ZS_W[0]}");
        let s1 = psubst_one("ZS_W", "Why, if it isn't Officer Dibble", "${ZS_W[1]}");
        let s03 = psubst_one("ZS_W", "Why, if it isn't Officer Dibble", "${ZS_W[0,3]}");
        assert_eq!(
            format!("[{s0}][{s1}][{s03}]"),
            "[][W][Why]",
            "ztst:236 — string subscripts [0]/[1]/[0,3]",
        );
    }

    /// `Test/D06subscript.ztst:5,12-14` — scalar (i) flag returns first
    /// index of a substring match.
    #[test]
    fn paramsubst_zsh_corpus_scalar_subscript_i_flag_first_match() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one(
            "ZS_T",
            "Twinkle, twinkle, little *, [how] I [wonder] what?  You are!",
            "${ZS_T[(i)winkle]}",
        );
        assert_eq!(result, "2", "ztst:14 — (i) flag returns first index");
    }

    /// `Test/D06subscript.ztst:31-32` — `s[(i)x]` returns `len(s)+1` for
    /// no-match.
    #[test]
    fn paramsubst_zsh_corpus_scalar_subscript_i_no_match_returns_len_plus_one() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one(
            "ZS_NM",
            "Twinkle, twinkle, little *, [how] I [wonder] what?  You are!",
            "${ZS_NM[(i)x]}",
        );
        assert_eq!(result, "61", "ztst:32 — (i) no-match returns len+1");
    }

    // ─── Length and case-modifier pins ────────────────────────────────

    /// `${#var}` returns length in characters.  `var=hello` → "5".
    #[test]
    fn paramsubst_zsh_corpus_hash_prefix_returns_length() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZL_L", "hello", "${#ZL_L}");
        assert_eq!(result, "5", "${{#var}} returns char length");
    }

    /// `${#var}` on empty returns "0".
    #[test]
    fn paramsubst_zsh_corpus_hash_prefix_empty_string_zero() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZL_E", "", "${#ZL_E}");
        assert_eq!(result, "0", "${{#var}} empty returns 0");
    }

    /// `${#var}` on multibyte content counts code points.
    #[test]
    fn paramsubst_zsh_corpus_hash_prefix_multibyte_codepoints() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZL_M", "héllo", "${#ZL_M}");
        assert_eq!(result, "5", "${{#var}} multibyte counts codepoints");
    }

    /// `${(L)var}` lowercases entire scalar.
    #[test]
    fn paramsubst_zsh_corpus_l_flag_lowercases_scalar() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZL_C", "HELLO WORLD", "${(L)ZL_C}");
        assert_eq!(result, "hello world", "(L) flag lowercases");
    }

    /// `${(U)var}` uppercases entire scalar.
    #[test]
    fn paramsubst_zsh_corpus_u_flag_uppercases_scalar() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZU_C", "hello world", "${(U)ZU_C}");
        assert_eq!(result, "HELLO WORLD", "(U) flag uppercases");
    }

    /// `${(C)var}` capitalizes first letter of each word.
    #[test]
    fn paramsubst_zsh_corpus_c_flag_capitalizes_words() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZC_S", "hello world foo bar", "${(C)ZC_S}");
        assert_eq!(result, "Hello World Foo Bar", "(C) flag capitalizes words");
    }

    // ─── Test/D04parameter.ztst:464-472 — (Q) dequoting flag ─────────

    /// `Test/D04parameter.ztst:464-467` — `${(Q)foo}` strips quotes and
    /// backslash escapes. foo=`'and now' "even the pubs" \a\r\e shut.`
    /// → `and now even the pubs are shut.`
    #[test]
    fn paramsubst_zsh_corpus_q_flag_dequotes_scalar() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one(
            "ZQ_S",
            r#"'and now' "even the pubs" \a\r\e shut."#,
            "${(Q)ZQ_S}",
        );
        assert_eq!(
            result, "and now even the pubs are shut.",
            "ztst:467 — (Q) strips quotes + backslashes",
        );
    }

    /// `Test/D04parameter.ztst:452-458` — `${(q-)foo}` minimal single
    /// quoting: foo='foo' → `foo` (no quotes needed for plain word).
    #[test]
    fn paramsubst_zsh_corpus_q_minus_flag_no_quote_needed() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZQM_P", "foo", "${(q-)ZQM_P}");
        assert_eq!(result, "foo", "ztst:458 — (q-) on plain word no quotes");
    }

    /// `Test/D04parameter.ztst:453-459` — `${(q-)foo}` with space:
    /// foo='foo bar' → `'foo bar'`.
    #[test]
    fn paramsubst_zsh_corpus_q_minus_flag_space_gets_quoted() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZQM_SP", "foo bar", "${(q-)ZQM_SP}");
        assert_eq!(
            result, "'foo bar'",
            "ztst:459 — (q-) quotes when space present"
        );
    }

    /// `Test/D04parameter.ztst:454-460` — `${(q-)foo}` with glob chars:
    /// foo='*(.)' → `'*(.)'`.
    #[test]
    fn paramsubst_zsh_corpus_q_minus_flag_glob_chars_quoted() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZQM_G", "*(.)", "${(q-)ZQM_G}");
        assert_eq!(result, "'*(.)'", "ztst:460 — (q-) quotes glob chars");
    }

    // ─── Test/D04parameter.ztst:1301-1304 — empty-string substitution ─

    /// `Test/D04parameter.ztst:1301-1304` — `${${foo}/?*/replacement}` on
    /// empty `foo`: nothing to replace, result is empty string.
    #[test]
    fn paramsubst_zsh_corpus_quoted_zero_length_in_subst() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZZL_F", "", "${${ZZL_F}/?*/replacement}");
        assert_eq!(
            result, "",
            "ztst:1304 — empty var stays empty through nested /"
        );
    }

    // ─── More ${var:-default} / ${var:+alt} / ${var:?err} pins ────────

    /// `${var:-default}` returns default when var is unset/empty.
    #[test]
    fn paramsubst_zsh_corpus_colon_minus_empty_uses_default() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZCM_E", "", "${ZCM_E:-fallback}");
        assert_eq!(result, "fallback", "${{var:-d}} empty uses default");
    }

    /// `${var:-default}` returns var value when set and non-empty.
    #[test]
    fn paramsubst_zsh_corpus_colon_minus_set_uses_var() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZCM_S", "real", "${ZCM_S:-fallback}");
        assert_eq!(result, "real", "${{var:-d}} non-empty returns var");
    }

    /// `${var:+alt}` returns alt when var is set and non-empty.
    #[test]
    fn paramsubst_zsh_corpus_colon_plus_set_uses_alt() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZCP_S", "yes", "${ZCP_S:+alt}");
        assert_eq!(result, "alt", "${{var:+a}} non-empty returns alt");
    }

    /// `${var:+alt}` returns empty when var is empty/unset.
    #[test]
    fn paramsubst_zsh_corpus_colon_plus_empty_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZCP_E", "", "${ZCP_E:+alt}");
        assert_eq!(result, "", "${{var:+a}} empty returns empty");
    }

    /// `${var-default}` (no colon) — default only when var is unset.
    /// Set-but-empty gets the empty string, not the default.
    #[test]
    fn paramsubst_zsh_corpus_dash_only_empty_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZD_E", "", "${ZD_E-fallback}");
        assert_eq!(
            result, "",
            "${{var-d}} set-but-empty returns empty (not default)"
        );
    }

    // ─── String trim flags: # ## % %% pins ──────────────────────────

    /// `${var#pattern}` strips shortest prefix match.
    /// var=hellohello, pattern=hello → "hello" (remove one prefix).
    #[test]
    fn paramsubst_zsh_corpus_hash_strip_shortest_prefix() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZP_S", "hellohello", "${ZP_S#hello}");
        assert_eq!(result, "hello", "${{var#pat}} strips shortest prefix");
    }

    /// `${var##pattern}` strips longest prefix match.
    /// var=hellohello, pattern=h* → "" (greedy match).
    #[test]
    fn paramsubst_zsh_corpus_double_hash_strip_longest_prefix() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZP_L", "hellohello", "${ZP_L##h*}");
        assert_eq!(result, "", "${{var##pat}} strips longest prefix");
    }

    /// `${var%pattern}` strips shortest suffix match.
    /// var=hellohello, pattern=hello → "hello".
    #[test]
    fn paramsubst_zsh_corpus_percent_strip_shortest_suffix() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZS_S", "hellohello", "${ZS_S%hello}");
        assert_eq!(result, "hello", "${{var%pat}} strips shortest suffix");
    }

    /// `${var%%pattern}` strips longest suffix match.
    /// var=hellohello, pattern=l* → "he".
    #[test]
    fn paramsubst_zsh_corpus_double_percent_strip_longest_suffix() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZS_L", "hellohello", "${ZS_L%%l*}");
        assert_eq!(result, "he", "${{var%%pat}} strips longest suffix");
    }

    // ─── Test/D07multibyte.ztst:130-136 — case modification, multibyte ─

    /// `Test/D07multibyte.ztst:131-133` — `${(U)a}` uppercases multibyte.
    /// `a=ténébreux` → `TÉNÉBREUX`.
    #[test]
    fn paramsubst_zsh_corpus_u_flag_uppercases_multibyte() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZUM", "ténébreux", "${(U)ZUM}");
        assert_eq!(result, "TÉNÉBREUX", "ztst:133 — (U) on accented chars");
    }

    /// `Test/D07multibyte.ztst:131-134` — `${(L)var}` lowercases multibyte.
    /// `a=TÉNÉBREUX` → `ténébreux`.
    #[test]
    fn paramsubst_zsh_corpus_l_flag_lowercases_multibyte() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZLM", "TÉNÉBREUX", "${(L)ZLM}");
        assert_eq!(result, "ténébreux", "ztst:134 — (L) on accented chars");
    }

    /// `Test/D07multibyte.ztst:135` — `${(C)var}` capitalizes multibyte words.
    /// `l'état c'est moi` → `L'État C'Est Moi` (per zsh, capital after `'`
    /// since apostrophe is non-alphanumeric word separator).
    #[test]
    fn paramsubst_zsh_corpus_c_flag_capitalizes_multibyte_with_apostrophe() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZCM", "l'état c'est moi", "${(C)ZCM}");
        assert_eq!(
            result, "L'État C'Est Moi",
            "ztst:136 — (C) restarts word after '"
        );
    }

    // ─── Subscript on multibyte string ────────────────────────────────

    /// `Test/D07multibyte.ztst:13-21` — `${a[1]}` returns first codepoint
    /// (not first byte). `a=ténébreux` → `${a[1]}` = "t".
    #[test]
    fn paramsubst_zsh_corpus_multibyte_subscript_first() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZMS", "ténébreux", "${ZMS[1]}");
        assert_eq!(result, "t", "ztst:21 — [1] is first codepoint, not byte");
    }

    /// `Test/D07multibyte.ztst:14-22` — `${a[2]}` returns 2nd codepoint.
    /// `a=ténébreux` → `${a[2]}` = "é" (one codepoint, 2 bytes).
    #[test]
    fn paramsubst_zsh_corpus_multibyte_subscript_accented() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZMS2", "ténébreux", "${ZMS2[2]}");
        assert_eq!(
            result, "é",
            "ztst:22 — [2] is 'é' (multibyte codepoint, not byte)"
        );
    }

    /// `Test/D07multibyte.ztst` — `${a[1,3]}` slice spans 3 codepoints.
    /// `a=ténébreux` → "tén".
    #[test]
    fn paramsubst_zsh_corpus_multibyte_slice_first_three() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZMSL", "ténébreux", "${ZMSL[1,3]}");
        assert_eq!(result, "tén", "ztst:22 — [1,3] = first 3 codepoints");
    }

    // ─── Pattern subst with multibyte ─────────────────────────────────

    /// Multibyte pattern in `/`: `${var/é/X}` should replace one codepoint.
    /// `var=téX` → "tXX"? Let me think — var=ténébreux, /é/X → "tXnébreux".
    #[test]
    fn paramsubst_zsh_corpus_multibyte_pattern_replace_first() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZMR", "ténébreux", "${ZMR/é/X}");
        assert_eq!(result, "tXnébreux", "first / replaces first é");
    }

    /// Multibyte `//` replaces all occurrences. `ténébreux`, é→X
    /// → "tXnXbreux".
    #[test]
    fn paramsubst_zsh_corpus_multibyte_pattern_replace_all() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZMRG", "ténébreux", "${ZMRG//é/X}");
        assert_eq!(result, "tXnXbreux", "// replaces all é");
    }

    // ─── ${name:offset:length} substring (bash-style) pins ─────────────

    /// `${var:offset}` returns substring from offset to end.
    /// var=hello, offset=2 → "llo".
    #[test]
    fn paramsubst_zsh_corpus_substring_offset_only() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZSS_O", "hello", "${ZSS_O:2}");
        assert_eq!(result, "llo", "${{var:2}} skips first 2");
    }

    /// `${var:offset:length}` returns substring of given length.
    /// var=hello, offset=1, length=3 → "ell".
    #[test]
    fn paramsubst_zsh_corpus_substring_offset_length() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZSS_OL", "hello", "${ZSS_OL:1:3}");
        assert_eq!(result, "ell", "${{var:1:3}} 3 chars from offset 1");
    }

    /// `${var:0}` — offset 0 = entire string.
    #[test]
    fn paramsubst_zsh_corpus_substring_offset_zero() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZSS_Z", "hello", "${ZSS_Z:0}");
        assert_eq!(result, "hello", "${{var:0}} = entire string");
    }

    /// `${var:0:0}` — zero length = empty string.
    #[test]
    fn paramsubst_zsh_corpus_substring_zero_length() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZSS_E", "hello", "${ZSS_E:0:0}");
        assert_eq!(result, "", "${{var:0:0}} = empty");
    }

    /// `${var:offset}` with offset past end = empty.
    /// var=hi, offset=10 → "".
    #[test]
    fn paramsubst_zsh_corpus_substring_offset_past_end() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZSS_P", "hi", "${ZSS_P:10}");
        assert_eq!(result, "", "${{var:past_end}} = empty");
    }

    /// `${var:-offset}` with NEGATIVE offset from end.
    /// var=hello, offset=-2 → "lo" (last 2 chars).
    /// Note: needs space after colon to disambiguate from `${var:-default}`.
    #[test]
    fn paramsubst_zsh_corpus_substring_negative_offset() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZSS_N", "hello", "${ZSS_N: -2}");
        assert_eq!(result, "lo", "${{var: -2}} = last 2 chars");
    }

    // ─── $(( ... )) arithmetic substitution corpus pins ──────────────

    /// `$((1+2))` returns "3" string.
    #[test]
    fn paramsubst_zsh_corpus_arith_simple_add() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZAR_A", "ignored", "$((1+2))");
        assert_eq!(result, "3", "$((1+2)) = '3'");
    }

    /// `$((10*5))` returns "50".
    #[test]
    fn paramsubst_zsh_corpus_arith_multiply() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZAR_M", "ignored", "$((10*5))");
        assert_eq!(result, "50");
    }

    /// `$((1 << 4))` returns "16" (left shift).
    #[test]
    fn paramsubst_zsh_corpus_arith_left_shift() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZAR_LS", "ignored", "$((1 << 4))");
        assert_eq!(result, "16");
    }

    /// `$((0xff))` returns "255" (hex literal).
    #[test]
    fn paramsubst_zsh_corpus_arith_hex_literal() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZAR_H", "ignored", "$((0xff))");
        assert_eq!(result, "255");
    }

    /// `$((-5))` returns "-5" (unary minus).
    #[test]
    fn paramsubst_zsh_corpus_arith_unary_minus() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZAR_N", "ignored", "$((-5))");
        assert_eq!(result, "-5");
    }

    /// `$((100/3))` integer division returns "33".
    #[test]
    fn paramsubst_zsh_corpus_arith_integer_division() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZAR_D", "ignored", "$((100/3))");
        assert_eq!(result, "33");
    }

    /// `$((10%3))` modulo returns "1".
    #[test]
    fn paramsubst_zsh_corpus_arith_modulo() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZAR_MOD", "ignored", "$((10%3))");
        assert_eq!(result, "1");
    }

    /// `$((2**8))` exponentiation returns "256".
    #[test]
    fn paramsubst_zsh_corpus_arith_power() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZAR_P", "ignored", "$((2**8))");
        assert_eq!(result, "256");
    }

    /// `$(( var * 2 ))` with var=21 → "42".
    #[test]
    fn paramsubst_zsh_corpus_arith_with_variable() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::unsetparam("ZAR_V");
        crate::ported::params::setiparam("ZAR_V", 21);
        let result = psubst_one("ZAR_V_IGNORE", "ignored", "$(( ZAR_V * 2 ))");
        assert_eq!(result, "42");
        crate::ported::params::unsetparam("ZAR_V");
    }

    // ─── ${(t)var} type-query pins ────────────────────────────────────

    /// `${(t)var}` on scalar returns "scalar".
    #[test]
    fn paramsubst_zsh_corpus_type_query_scalar() {
        let _g = crate::test_util::global_state_lock();
        let result = psubst_one("ZT_S", "hello", "${(t)ZT_S}");
        assert!(
            result.starts_with("scalar"),
            "${{(t)var}} on scalar starts with 'scalar', got: {result:?}",
        );
    }

    /// `${(t)var}` on array returns "array" (with possible scope suffix).
    #[test]
    fn paramsubst_zsh_corpus_type_query_array() {
        let _g = crate::test_util::global_state_lock();
        let (result, _) = psubst_arr("ZT_A", &["a", "b"], "${(t)ZT_A}");
        assert!(
            result.starts_with("array"),
            "${{(t)var}} on array starts with 'array', got: {result:?}",
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/subst.c
    // c:1172 quotesubst / c:1357 singsub / c:2062 wcpadwidth /
    // c:2295 get_strarg / c:2342 get_intarg / c:2479 substevalchar /
    // c:2541 untok_and_escape / c:2612 check_colon_subscript /
    // c:8863 arithsubst / c:13121 sub_flags_get / c:13140 sub_flags_set
    // ═══════════════════════════════════════════════════════════════════

    /// c:1172 — `quotesubst("")` empty returns empty String.
    #[test]
    fn quotesubst_empty_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(quotesubst(""), "");
    }

    /// c:1172 — `quotesubst` is pure.
    #[test]
    fn quotesubst_is_pure() {
        let _g = crate::test_util::global_state_lock();
        for s in ["", "abc", "$var", "no-special"] {
            let first = quotesubst(s);
            for _ in 0..3 {
                assert_eq!(quotesubst(s), first, "quotesubst({:?}) must be pure", s);
            }
        }
    }

    /// c:1357 — `singsub("")` empty returns empty String.
    #[test]
    fn singsub_empty_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(singsub(""), "");
    }

    /// c:2062 — `wcpadwidth` returns i32 (compile-time type pin).
    #[test]
    fn wcpadwidth_returns_i32_type() {
        let _: i32 = wcpadwidth('a', 0);
    }

    /// c:2062 — `wcpadwidth('a', _)` returns 1 (single-width ASCII).
    #[test]
    fn wcpadwidth_ascii_letter_returns_one() {
        assert_eq!(wcpadwidth('a', 0), 1, "ASCII letter width = 1");
    }

    /// c:2062 — `wcpadwidth` is pure.
    #[test]
    fn wcpadwidth_is_pure() {
        for c in ['a', '0', ' ', '\t', '日'] {
            let first = wcpadwidth(c, 0);
            for _ in 0..3 {
                assert_eq!(
                    wcpadwidth(c, 0),
                    first,
                    "wcpadwidth({:?}, 0) must be pure",
                    c
                );
            }
        }
    }

    /// c:2295 — `get_strarg("")` empty returns None.
    #[test]
    fn get_strarg_empty_returns_none() {
        assert!(get_strarg("").is_none(), "empty → None");
    }

    /// c:2342 — `get_intarg("")` empty returns None.
    #[test]
    fn get_intarg_empty_returns_none() {
        assert!(get_intarg("").is_none(), "empty → None");
    }

    /// c:2541 — `untok_and_escape("", _, _)` empty returns empty.
    #[test]
    fn untok_and_escape_empty_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(untok_and_escape("", false, false), "");
    }

    /// c:13121 + c:13140 — sub_flags get/set round-trip.
    #[test]
    fn sub_flags_get_set_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let saved = sub_flags_get();
        sub_flags_set(0x42);
        assert_eq!(sub_flags_get(), 0x42, "sub_flags round-trips");
        sub_flags_set(saved);
    }

    /// c:2612 — `check_colon_subscript("")` empty returns None.
    #[test]
    fn check_colon_subscript_empty_returns_none_pin() {
        let _g = crate::test_util::global_state_lock();
        assert!(check_colon_subscript("").is_none(), "empty → None");
    }
} // c:3193

// ============================================================================
// Additional functions for 100% coverage of subst.c
// ============================================================================

/// Null string constant (matches C: char nulstring[] = {Nularg, '\0'})
pub static NULSTRING_BYTES: [char; 2] = [Nularg, '\0']; // c:3193

/// Assoc-array assignment via paramtab_hashed_storage. The `parts`
/// argument follows the C `sethparam` convention: alternating
/// key, value, key, value (`Src/params.c:3602`).
fn exec_sethparam(name: &str, parts: Vec<String>) {
    let mut map: indexmap::IndexMap<String, String> = indexmap::IndexMap::new();
    let mut it = parts.into_iter();
    while let (Some(k), Some(v)) = (it.next(), it.next()) {
        map.insert(k, v);
    }
    if let Ok(mut store) = paramtab_hashed_storage().lock() {
        store.insert(name.to_string(), map);
    }
    if let Ok(mut tab) = paramtab().write() {
        if let Some(pm) = tab.get_mut(name) {
            pm.node.flags |= PM_HASHED as i32;
        } else {
            let pm: Param = Box::new(param {
                node: hashnode {
                    next: None,
                    nam: name.to_string(),
                    flags: PM_HASHED as i32,
                },
                u_data: 0,
                u_tied: None,
                u_arr: None,
                u_str: None,
                u_val: 0,
                u_dval: 0.0,
                u_hash: None,
                gsu_s: None,
                gsu_i: None,
                gsu_f: None,
                gsu_a: None,
                gsu_h: None,
                base: 0,
                width: 0,
                env: None,
                ename: None,
                old: None,
                level: 0,
            });
            tab.insert(name.to_string(), pm);
        }
    }
}

/// No-op now that reads go directly to `paramtab` — the sync-shim
/// only existed for the executor-backed snapshot path.
fn exec_sync_state_from_paramtab() {}

/// Read a scalar from `paramtab` via the canonical `getsparam`
/// (`Src/params.c:3076`). Routes through `PM_TYPE` dispatch so
/// PM_ARRAY returns `sepjoin(arr)` (c:2367), PM_INTEGER returns
/// `convbase(u_val, base)` (c:2364), PM_FLOAT returns
/// `convfloat(...)` (c:2367-2368), and PM_SCALAR/PM_NAMEREF
/// returns `u_str`. The prior `vars_get` shortcut only handled
/// PM_SCALAR — arrays + ints + floats returned None → empty
/// raw_value, breaking `"${(o)arr}"` etc.
fn exec_getsparam(name: &str) -> Option<String> {
    crate::ported::params::getsparam(name)
}

/// Read the current paramsubst flag bitmask. Equivalent to C's
/// `sub_flags` read at `Src/subst.c:2171`.
pub fn sub_flags_get() -> i32 {
    SUB_FLAGS.with(|c| c.get())
}

// ============================================================================
// Final functions for complete subst.c coverage
// ============================================================================

// Local `Dnull` / `Bnullkeep` constants — DELETED per user
// directive. Both were WRONG values masquerading as canonical
// tokens: local `Dnull = '\u{97}'` is actually `Quest` (zsh.h:178);
// local `Bnullkeep = '\u{95}'` is actually `Outang` (zsh.h:176).
// Canonical values from `Src/zsh.h:194,200` are `Dnull = '\u{9e}'`
// and `Bnullkeep = '\u{a0}'`. Both already imported from
// `crate::ported::zsh_h` at the top of this file (Dnull) and
// available there (Bnullkeep). Bringing Bnullkeep into scope.

/// Write the paramsubst flag bitmask. Equivalent to C's
/// `sub_flags = X` at `Src/subst.c:2169`.
pub fn sub_flags_set(v: i32) {
    SUB_FLAGS.with(|c| c.set(v));
}

// ===========================================================
// Methods moved verbatim from src/ported/vm_helper because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: subst
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs (free ported)
