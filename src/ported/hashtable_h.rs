//! Direct port of `Src/hashtable.h` — header file for hash table
//! handling code.
//!
//! This file mirrors the entire `hashtable.h` header content. The
//! implementation lives in `crate::ported::hashtable` (port of
//! `Src/hashtable.c`).
//!
//! Per Src/hashtable.h:30-67 — the BIN_* dispatch IDs are used by
//! handler functions that handle more than one builtin (e.g.
//! `bin_break` handles BIN_BREAK / BIN_CONTINUE / BIN_RETURN /
//! BIN_EXIT / BIN_LOGOUT). Builtins not overloaded (like
//! `compctl`) don't get a number.

// =====================================================================
// Builtin function numbers — Src/hashtable.h:34-66.
// =====================================================================

pub const BIN_TYPESET:    i32 = 0;                                       // c:34
pub const BIN_BG:         i32 = 1;                                       // c:35
pub const BIN_FG:         i32 = 2;                                       // c:36
pub const BIN_JOBS:       i32 = 3;                                       // c:37
pub const BIN_WAIT:       i32 = 4;                                       // c:38
pub const BIN_DISOWN:     i32 = 5;                                       // c:39
pub const BIN_BREAK:      i32 = 6;                                       // c:40
pub const BIN_CONTINUE:   i32 = 7;                                       // c:41
pub const BIN_EXIT:       i32 = 8;                                       // c:42
pub const BIN_RETURN:     i32 = 9;                                       // c:43
pub const BIN_CD:         i32 = 10;                                      // c:44
pub const BIN_POPD:       i32 = 11;                                      // c:45
pub const BIN_PUSHD:      i32 = 12;                                      // c:46
pub const BIN_PRINT:      i32 = 13;                                      // c:47
pub const BIN_EVAL:       i32 = 14;                                      // c:48
pub const BIN_SCHED:      i32 = 15;                                      // c:49
pub const BIN_FC:         i32 = 16;                                      // c:50
pub const BIN_R:          i32 = 17;                                      // c:51
pub const BIN_PUSHLINE:   i32 = 18;                                      // c:52
pub const BIN_LOGOUT:     i32 = 19;                                      // c:53
pub const BIN_TEST:       i32 = 20;                                      // c:54
pub const BIN_BRACKET:    i32 = 21;                                      // c:55
pub const BIN_READONLY:   i32 = 22;                                      // c:56
pub const BIN_ECHO:       i32 = 23;                                      // c:57
pub const BIN_DISABLE:    i32 = 24;                                      // c:58
pub const BIN_ENABLE:     i32 = 25;                                      // c:59
pub const BIN_PRINTF:     i32 = 26;                                      // c:60
pub const BIN_COMMAND:    i32 = 27;                                      // c:61
pub const BIN_UNHASH:     i32 = 28;                                      // c:62
pub const BIN_UNALIAS:    i32 = 29;                                      // c:63
pub const BIN_UNFUNCTION: i32 = 30;                                      // c:64
pub const BIN_UNSET:      i32 = 31;                                      // c:65
pub const BIN_EXPORT:     i32 = 32;                                      // c:66

// c:69-70 — These currently depend on being 0 and 1.
pub const BIN_SETOPT:    i32 = 0;                                        // c:69
pub const BIN_UNSETOPT:  i32 = 1;                                        // c:70
