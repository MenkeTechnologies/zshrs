//! Faithful recursive wordcode decoder + canonical sexp emitter.
//!
//! Used by `tests/parity_harness.rs` to compare zsh's parser output
//! (via `.zwc` wordcode) against zshrs's parser output (via `ast_sexp`).
//!
//! ## Why this exists alongside `src/zwc.rs`
//!
//! The pre-existing `zwc.rs::WordcodeDecoder` is shallow: 12 of 14
//! body-bearing decoders (Pipe, Sublist, FuncDef, For, While, If, Case,
//! Cond, Try, Repeat, Select) return `vec![]` for their body fields.
//! Only `decode_subsh` and `decode_cursh` walk children. That decoder is
//! adequate for callers that only need function names + flat opcode
//! types (autoload introspection), but cannot serve as a parity oracle.
//!
//! This module reuses `zwc::ZwcFile::load()` for file-level parsing
//! (header, function table, wordcode buffer, string table) but
//! implements its OWN recursive walker over the wordcode array, citing
//! `~/forkedRepos/zsh/Src/parse.c` and `~/forkedRepos/zsh/Src/zsh.h`
//! for every opcode interpretation.
//!
//! ## References
//!
//! - `zsh.h:888-1038` — WC_*/WCB_* opcode + bit-field layout
//! - `parse.c:771-817` — `par_list`/`par_list1` emission pattern
//! - `parse.c:825-869` — `par_sublist`/`par_sublist2` AND/OR chains
//! - `parse.c:894-944` — `par_pline` pipe-stage emission
//! - `parse.c:958-1085` — `par_cmd` dispatch
//! - `parse.c:1087-1198` — `par_for` (3 forms: list, c-style, pparam)
//! - `parse.c:1209-1407` — `par_case`
//! - `parse.c:1411-1519` — `par_if` (head + elif chain + else)
//! - `parse.c:1521-1567` — `par_while`/`par_until`
//! - `parse.c:1565-1617` — `par_repeat`
//! - `parse.c:1619-1670` — `par_subsh`/`par_cursh`
//! - `parse.c:1672-1779` — `par_funcdef`
//! - `parse.c:1836-2228` — `par_simple` (assigns, words, redirs, named-fd)
//! - `parse.c:2229-2345` — `par_redir`

#![allow(clippy::too_many_arguments)]

use crate::zwc::{wc_code, wc_data, ZwcFile};
use std::path::Path;

// ---------------------------------------------------------------------------
// Opcode constants — `zsh.h:888-909`
// ---------------------------------------------------------------------------

const WC_END: u32 = 0;
const WC_LIST: u32 = 1;
const WC_SUBLIST: u32 = 2;
const WC_PIPE: u32 = 3;
const WC_REDIR: u32 = 4;
const WC_ASSIGN: u32 = 5;
const WC_SIMPLE: u32 = 6;
const WC_TYPESET: u32 = 7;
const WC_SUBSH: u32 = 8;
const WC_CURSH: u32 = 9;
const WC_TIMED: u32 = 10;
const WC_FUNCDEF: u32 = 11;
const WC_FOR: u32 = 12;
const WC_SELECT: u32 = 13;
const WC_WHILE: u32 = 14;
const WC_REPEAT: u32 = 15;
const WC_CASE: u32 = 16;
const WC_IF: u32 = 17;
const WC_COND: u32 = 18;
const WC_ARITH: u32 = 19;
const WC_AUTOFN: u32 = 20;
const WC_TRY: u32 = 21;

// `zsh.h:645-648, 921-922` — list flags
const Z_SYNC: u32 = 1 << 1;
const Z_ASYNC: u32 = 1 << 2;
const Z_DISOWN: u32 = 1 << 3;
const Z_END: u32 = 1 << 4;
const Z_SIMPLE: u32 = 1 << 5;
const WC_LIST_FREE: u32 = 6;

// `zsh.h:927-936` — sublist
const WC_SUBLIST_END: u32 = 0;
const WC_SUBLIST_AND: u32 = 1;
const WC_SUBLIST_OR: u32 = 2;
const WC_SUBLIST_COPROC: u32 = 4;
const WC_SUBLIST_NOT: u32 = 8;
const WC_SUBLIST_SIMPLE: u32 = 16;
const WC_SUBLIST_FREE: u32 = 5;

// `zsh.h:940-944` — pipe
const WC_PIPE_END: u32 = 0;
const WC_PIPE_MID: u32 = 1;

// `zsh.h:397-401` — redir mask bits
const REDIR_TYPE_MASK: u32 = 0x1f;
const REDIR_VARID_MASK: u32 = 0x20;
const REDIR_FROM_HEREDOC_MASK: u32 = 0x40;

// `zsh.h:376-395` — REDIR_* enum order
const REDIR_WRITE: u32 = 0;
const REDIR_WRITENOW: u32 = 1;
const REDIR_APP: u32 = 2;
const REDIR_APPNOW: u32 = 3;
const REDIR_ERRWRITE: u32 = 4;
const REDIR_ERRWRITENOW: u32 = 5;
const REDIR_ERRAPP: u32 = 6;
const REDIR_ERRAPPNOW: u32 = 7;
const REDIR_READWRITE: u32 = 8;
const REDIR_READ: u32 = 9;
const REDIR_HEREDOC: u32 = 10;
const REDIR_HEREDOCDASH: u32 = 11;
const REDIR_HERESTR: u32 = 12;
const REDIR_MERGEIN: u32 = 13;
const REDIR_MERGEOUT: u32 = 14;
const _REDIR_CLOSE: u32 = 15;
const REDIR_INPIPE: u32 = 16;
const REDIR_OUTPIPE: u32 = 17;

// `zsh.h:957-967` — assign
const WC_ASSIGN_SCALAR: u32 = 0;
const WC_ASSIGN_ARRAY: u32 = 1;
const WC_ASSIGN_INC: u32 = 1; // bit-1 set in TYPE2 = `+=`

// `zsh.h:1014-1020` — for
const WC_FOR_PPARAM: u32 = 0;
const WC_FOR_LIST: u32 = 1;
const WC_FOR_COND: u32 = 2;

// `zsh.h:1022-1027` — select
const WC_SELECT_PPARAM: u32 = 0;
const WC_SELECT_LIST: u32 = 1;

// `zsh.h:1029-1034` — while
const WC_WHILE_WHILE: u32 = 0;
const WC_WHILE_UNTIL: u32 = 1;

// `zsh.h:1043-1049` — case
const WC_CASE_HEAD: u32 = 0;
const WC_CASE_OR: u32 = 1;
const WC_CASE_AND: u32 = 2;
const WC_CASE_TESTAND: u32 = 3;
const WC_CASE_FREE: u32 = 3;

// `zsh.h:1051-1057` — if
const WC_IF_HEAD: u32 = 0;
const WC_IF_IF: u32 = 1;
const WC_IF_ELIF: u32 = 2;
const WC_IF_ELSE: u32 = 3;

// `zsh.h:1058-1059` — cond
const WC_COND_TYPE_MASK: u32 = 127;
const _WC_COND_FREE: u32 = 7;

// `zsh.h:660-680` — cond types
const COND_NOT: u32 = 0;
const COND_AND: u32 = 1;
const COND_OR: u32 = 2;

// ---------------------------------------------------------------------------
// AST shape (mirrors `parser::ZshCommand` so sexp emit can match)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct WcProgram {
    pub lists: Vec<WcList>,
}

#[derive(Debug, Clone)]
pub struct WcList {
    pub flags: WcListFlags,
    pub sublist: WcSublist,
}

#[derive(Debug, Clone, Copy)]
pub struct WcListFlags {
    pub async_: bool,
    pub disown: bool,
}

#[derive(Debug, Clone)]
pub struct WcSublist {
    pub flags: WcSublistFlags,
    pub pipe: WcPipe,
    pub next: Option<(WcSublistOp, Box<WcSublist>)>,
}

#[derive(Debug, Clone, Copy)]
pub struct WcSublistFlags {
    pub not: bool,
    pub coproc: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WcSublistOp {
    And,
    Or,
}

#[derive(Debug, Clone)]
pub struct WcPipe {
    pub cmd: WcCommand,
    pub next: Option<Box<WcPipe>>,
}

#[derive(Debug, Clone)]
pub enum WcCommand {
    Simple(WcSimple),
    Subsh(Box<WcProgram>),
    Cursh(Box<WcProgram>),
    For(WcFor),
    Case(WcCase),
    If(WcIf),
    While(WcWhile),
    Until(WcWhile),
    Repeat(WcRepeat),
    FuncDef(WcFuncDef),
    Time(Option<Box<WcSublist>>),
    Cond(WcCond),
    Arith(String),
    Try(WcTry),
    AutoFn,
}

#[derive(Debug, Clone, Default)]
pub struct WcSimple {
    pub assigns: Vec<WcAssign>,
    pub words: Vec<String>,
    pub redirs: Vec<WcRedir>,
}

#[derive(Debug, Clone)]
pub struct WcAssign {
    pub name: String,
    pub value: WcAssignValue,
    pub append: bool,
}

#[derive(Debug, Clone)]
pub enum WcAssignValue {
    Scalar(String),
    Array(Vec<String>),
}

#[derive(Debug, Clone)]
pub struct WcRedir {
    pub rtype: u32,
    pub fd: i32,
    pub name: String,
    pub varid: Option<String>,
    pub heredoc: Option<WcHeredoc>,
}

#[derive(Debug, Clone)]
pub struct WcHeredoc {
    pub terminator: String,
    pub quoted: bool,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct WcFor {
    pub var: String,
    pub list: WcForList,
    pub body: Box<WcProgram>,
    pub is_select: bool,
}

#[derive(Debug, Clone)]
pub enum WcForList {
    Words(Vec<String>),
    CStyle {
        init: String,
        cond: String,
        step: String,
    },
    Positional,
}

#[derive(Debug, Clone)]
pub struct WcCase {
    pub word: String,
    pub arms: Vec<WcCaseArm>,
}

#[derive(Debug, Clone)]
pub struct WcCaseArm {
    pub patterns: Vec<String>,
    pub body: WcProgram,
    pub terminator: u32,
}

#[derive(Debug, Clone)]
pub struct WcIf {
    pub cond: WcProgram,
    pub then: WcProgram,
    pub elif: Vec<(WcProgram, WcProgram)>,
    pub else_: Option<WcProgram>,
}

#[derive(Debug, Clone)]
pub struct WcWhile {
    pub cond: WcProgram,
    pub body: WcProgram,
}

#[derive(Debug, Clone)]
pub struct WcRepeat {
    pub count: String,
    pub body: WcProgram,
}

#[derive(Debug, Clone)]
pub struct WcFuncDef {
    pub names: Vec<String>,
    pub body: WcProgram,
}

#[derive(Debug, Clone)]
pub enum WcCond {
    Not(Box<WcCond>),
    And(Box<WcCond>, Box<WcCond>),
    Or(Box<WcCond>, Box<WcCond>),
    Unary(String, String),
    Binary(String, String, String),
}

#[derive(Debug, Clone)]
pub struct WcTry {
    pub try_block: WcProgram,
    pub always: WcProgram,
}

// ---------------------------------------------------------------------------
// Walker
// ---------------------------------------------------------------------------

struct Walker<'a> {
    code: &'a [u32],
    strings: &'a [u8],
    strs_base: usize,
    pos: usize,
}

impl<'a> Walker<'a> {
    fn new(code: &'a [u32], strings: &'a [u8], strs_base: usize) -> Self {
        Self {
            code,
            strings,
            strs_base,
            pos: 0,
        }
    }

    fn peek(&self) -> Option<u32> {
        self.code.get(self.pos).copied()
    }

    fn next(&mut self) -> Option<u32> {
        let v = self.code.get(self.pos).copied();
        if v.is_some() {
            self.pos += 1;
        }
        v
    }

    /// Decode one wordcode-encoded string, per `parse.c` `ecstrcode` /
    /// `ecrawstr`. Short strings pack 1-3 chars in the upper bits;
    /// long strings reference the string table at `strs_base + (wc>>2)`.
    fn read_string(&mut self) -> String {
        let wc = self.next().unwrap_or(0);
        self.decode_string_word(wc)
    }

    fn decode_string_word(&self, wc: u32) -> String {
        // Sentinel: 6 (or 7) → empty (`parse.c` ecstrcode).
        if wc == 6 || wc == 7 {
            return String::new();
        }
        if (wc & 2) != 0 {
            // Short: 1-3 bytes packed in bits 3-10, 11-18, 19-26.
            // Pack the raw bytes then run them through untokenize so the
            // result matches the long-string path (string_at also calls
            // untokenize on the raw bytes). Without this, short strings
            // like `$x` (`\x85x`) and `-f` (`\x9bf`) leak the raw token
            // bytes (Pound, String, Dash, …) into the AST sexp.
            let mut bytes: Vec<u8> = Vec::new();
            for shift in [3, 11, 19] {
                let c = ((wc >> shift) & 0xff) as u8;
                if c == 0 {
                    break;
                }
                bytes.push(c);
            }
            crate::zwc::untokenize(&bytes)
        } else {
            // Long: byte offset into strings table.
            let offset = self.strs_base + (wc >> 2) as usize;
            self.string_at(offset)
        }
    }

    fn string_at(&self, offset: usize) -> String {
        if offset >= self.strings.len() {
            return String::new();
        }
        let end = self.strings[offset..]
            .iter()
            .position(|&b| b == 0)
            .map(|p| offset + p)
            .unwrap_or(self.strings.len());
        // Untokenize zsh's internal markers (Bnull, Snull, Star, etc.) back
        // to source bytes. Reuse zwc.rs's helper.
        let raw = &self.strings[offset..end];
        crate::zwc::untokenize(raw)
    }

    /// Top-level: walk a complete program until WC_END.
    /// Maps to `parse.c::par_event` driving `par_list`.
    fn decode_program(&mut self) -> WcProgram {
        let mut lists = Vec::new();
        while let Some(wc) = self.peek() {
            let code = wc_code(wc);
            if code == WC_END {
                self.next();
                break;
            }
            if code != WC_LIST {
                // Defensive: shouldn't happen at program-level, but bail
                // gracefully rather than infinite-loop.
                break;
            }
            lists.push(self.decode_list());
        }
        WcProgram { lists }
    }

    /// Sub-program inside a known-bounded region (subsh body, for body, etc.).
    /// Reads WC_LIST entries until pos == end_pos, until WC_END, OR until
    /// a list with Z_END flag is consumed (per execlist's `if (ltype &
    /// Z_END) break;` at exec.c:1626 — Z_END marks the natural boundary
    /// between cond/body programs in WC_IF / WC_WHILE etc.).
    fn decode_program_until(&mut self, end_pos: usize) -> WcProgram {
        let mut lists = Vec::new();
        while self.pos < end_pos {
            let wc = match self.peek() {
                Some(w) => w,
                None => break,
            };
            let code = wc_code(wc);
            if code == WC_END {
                self.next();
                break;
            }
            if code != WC_LIST {
                break;
            }
            // Inspect Z_END flag BEFORE consuming the list (decode_list
            // reads the WC_LIST header and we need the flag to decide
            // whether to terminate the program after it).
            let type_bits = wc_data(wc) & ((1 << WC_LIST_FREE) - 1);
            let is_z_end = (type_bits & Z_END) != 0;
            lists.push(self.decode_list());
            if is_z_end {
                break;
            }
        }
        WcProgram { lists }
    }

    /// `parse.c:771-817` par_list / par_list1 — emits `WCB_LIST(type, skip)`.
    /// Z_SIMPLE shortcut: body is `[lineno, single_pipe_body]` directly,
    /// with no inner WC_SUBLIST. Otherwise body starts with WC_SUBLIST.
    fn decode_list(&mut self) -> WcList {
        let header = match self.next() {
            Some(h) if wc_code(h) == WC_LIST => h,
            _ => {
                return WcList {
                    flags: WcListFlags { async_: false, disown: false },
                    sublist: WcSublist {
                        flags: WcSublistFlags { not: false, coproc: false },
                        pipe: WcPipe { cmd: WcCommand::Simple(WcSimple::default()), next: None },
                        next: None,
                    },
                };
            }
        };
        let data = wc_data(header);
        let type_bits = data & ((1 << WC_LIST_FREE) - 1);
        let _skip = data >> WC_LIST_FREE;

        let async_ = (type_bits & Z_ASYNC) != 0;
        let disown = (type_bits & Z_DISOWN) != 0;
        let is_simple = (type_bits & Z_SIMPLE) != 0;

        let sublist = if is_simple {
            // Z_SIMPLE shortcut: lineno, then a single pipe stage body
            // (possibly with redirs/assigns), no WC_SUBLIST wrapper.
            let _lineno = self.next();
            // The body that follows is the inside of a single Pipe.
            // Synthesize a Sublist+Pipe wrapper so canonical AST matches.
            let cmd = self.decode_command_body();
            WcSublist {
                flags: WcSublistFlags {
                    not: false,
                    coproc: false,
                },
                pipe: WcPipe { cmd, next: None },
                next: None,
            }
        } else {
            self.decode_sublist()
        };

        WcList {
            flags: WcListFlags { async_, disown },
            sublist,
        }
    }

    /// `parse.c:825-869` par_sublist — emits chained `WCB_SUBLIST` ops with
    /// types END/AND/OR. SUBLIST_SIMPLE flag means body is single-pipe
    /// shortcut (lineno + body without inner WC_PIPE).
    fn decode_sublist(&mut self) -> WcSublist {
        // Peek (don't advance) — if not WC_SUBLIST, the parent's expectations
        // are wrong. Don't consume; let the outer loop decide what to do.
        let header = match self.peek() {
            Some(h) if wc_code(h) == WC_SUBLIST => {
                self.next();
                h
            }
            _ => {
                return WcSublist {
                    flags: WcSublistFlags { not: false, coproc: false },
                    pipe: WcPipe { cmd: WcCommand::Simple(WcSimple::default()), next: None },
                    next: None,
                };
            }
        };
        let data = wc_data(header);
        let stype = data & 3;
        let flags_bits = data & 0x1c;
        let _skip = data >> WC_SUBLIST_FREE;

        let not = (flags_bits & WC_SUBLIST_NOT) != 0;
        let coproc = (flags_bits & WC_SUBLIST_COPROC) != 0;
        let is_simple = (flags_bits & WC_SUBLIST_SIMPLE) != 0;

        let pipe = if is_simple {
            let _lineno = self.next();
            let cmd = self.decode_command_body();
            WcPipe { cmd, next: None }
        } else {
            self.decode_pipe()
        };

        let next = match stype {
            WC_SUBLIST_AND => Some((WcSublistOp::And, Box::new(self.decode_sublist()))),
            WC_SUBLIST_OR => Some((WcSublistOp::Or, Box::new(self.decode_sublist()))),
            _ => None, // WC_SUBLIST_END
        };

        WcSublist {
            flags: WcSublistFlags { not, coproc },
            pipe,
            next,
        }
    }

    /// `parse.c:894-944` par_pline — chained `WCB_PIPE(MID|END, lineno)`.
    /// MID stages have an extra skip-count word inserted at p+1 (per
    /// parse.c:912-913 ecispace) — consume it. END stages have no extra.
    fn decode_pipe(&mut self) -> WcPipe {
        let header = match self.peek() {
            Some(h) if wc_code(h) == WC_PIPE => {
                self.next();
                h
            }
            _ => {
                return WcPipe {
                    cmd: WcCommand::Simple(WcSimple::default()),
                    next: None,
                };
            }
        };
        let data = wc_data(header);
        let ptype = data & 1;
        // lineno = data >> 1 — discarded (not part of canonical AST).

        if ptype == WC_PIPE_MID {
            // par_pline (parse.c:912) inserts ecused-1-p as skip count.
            let _stage_skip = self.next();
        }

        let cmd = self.decode_command_body();

        let next = if ptype == WC_PIPE_MID {
            Some(Box::new(self.decode_pipe()))
        } else {
            None
        };

        WcPipe { cmd, next }
    }

    /// Decode the command body of a Pipe stage. zsh wordcode emits the
    /// command's leading opcode (WC_SIMPLE / WC_SUBSH / WC_FOR / etc.),
    /// optionally with surrounding redirs/assigns for Simple. Per
    /// `parse.c:958-1085` par_cmd dispatch.
    fn decode_command_body(&mut self) -> WcCommand {
        // For SIMPLE, zsh emits assigns + redirs interleaved with WC_SIMPLE.
        // Other compound commands have the command-leading opcode first.
        // Use lookahead to discriminate.
        //
        // Strategy: read a pre-amble of WC_ASSIGN/WC_REDIR ops; on the first
        // non-assign-non-redir op, dispatch based on its WC_*. If it's
        // WC_SIMPLE, finish gathering the trailing redirs into the simple's
        // redirs list. If it's a compound (WC_FOR etc.), pass through.

        let mut leading_assigns: Vec<WcAssign> = Vec::new();
        let mut leading_redirs: Vec<WcRedir> = Vec::new();

        loop {
            let wc = match self.peek() {
                Some(w) => w,
                None => return WcCommand::Simple(WcSimple::default()),
            };
            let code = wc_code(wc);
            match code {
                WC_ASSIGN => {
                    self.next();
                    leading_assigns.push(self.decode_assign(wc_data(wc)));
                }
                WC_REDIR => {
                    self.next();
                    leading_redirs.push(self.decode_redir(wc_data(wc)));
                }
                WC_SIMPLE => {
                    self.next();
                    let argc = wc_data(wc) as usize;
                    let mut words = Vec::with_capacity(argc);
                    for _ in 0..argc {
                        words.push(self.read_string());
                    }
                    // Trailing redirs (zsh encodes them after the WC_SIMPLE
                    // args within the same pipe stage).
                    while let Some(next_wc) = self.peek() {
                        if wc_code(next_wc) != WC_REDIR {
                            break;
                        }
                        self.next();
                        leading_redirs.push(self.decode_redir(wc_data(next_wc)));
                    }
                    return WcCommand::Simple(WcSimple {
                        assigns: leading_assigns,
                        words,
                        redirs: leading_redirs,
                    });
                }
                WC_TYPESET => {
                    // typeset: like SIMPLE but with assigns appended after
                    // the args. Conservatively render as SIMPLE for now.
                    self.next();
                    let argc = wc_data(wc) as usize;
                    let mut words = Vec::with_capacity(argc);
                    for _ in 0..argc {
                        words.push(self.read_string());
                    }
                    let num_assigns = self.next().unwrap_or(0) as usize;
                    for _ in 0..num_assigns {
                        if let Some(aw) = self.peek() {
                            if wc_code(aw) == WC_ASSIGN {
                                self.next();
                                leading_assigns.push(self.decode_assign(wc_data(aw)));
                            } else {
                                break;
                            }
                        }
                    }
                    return WcCommand::Simple(WcSimple {
                        assigns: leading_assigns,
                        words,
                        redirs: leading_redirs,
                    });
                }
                _ => {
                    // Compound command. Decode it; preserve any leading
                    // redirs by wrapping (Redirected ...) — but for now
                    // most compound forms have no leading redirs; if they
                    // do, the parser audit will surface that.
                    let cmd = self.decode_compound();
                    if leading_redirs.is_empty() && leading_assigns.is_empty() {
                        return cmd;
                    }
                    // Best-effort: leading redirs/assigns on a compound cmd
                    // are unusual in real source; record on a synthetic
                    // wrapper. The AST side wraps via ZshCommand::Redirected.
                    return cmd; // assigns/redirs lost; will surface as a
                                // diff if real cases exist.
                }
            }
        }
    }

    fn decode_compound(&mut self) -> WcCommand {
        let wc = self.next().expect("compound command opcode");
        let code = wc_code(wc);
        let data = wc_data(wc);
        match code {
            WC_SUBSH => WcCommand::Subsh(Box::new(self.decode_subsh(data))),
            WC_CURSH => WcCommand::Cursh(Box::new(self.decode_cursh(data))),
            WC_FOR => self.decode_for(data, false),
            WC_SELECT => self.decode_select(data),
            WC_WHILE => self.decode_while(data),
            WC_REPEAT => WcCommand::Repeat(self.decode_repeat(data)),
            WC_IF => WcCommand::If(self.decode_if(data)),
            WC_CASE => WcCommand::Case(self.decode_case(data)),
            WC_FUNCDEF => WcCommand::FuncDef(self.decode_funcdef(data)),
            WC_TIMED => self.decode_timed(data),
            WC_COND => WcCommand::Cond(self.decode_cond_expr(data)),
            WC_ARITH => {
                let expr = self.read_string();
                WcCommand::Arith(expr)
            }
            WC_AUTOFN => WcCommand::AutoFn,
            WC_TRY => WcCommand::Try(self.decode_try(data)),
            _ => WcCommand::Simple(WcSimple::default()),
        }
    }

    /// `parse.c::par_redir` — emits WCB_REDIR(type|flags) + fd + target_str
    /// + [varid_str?] + [heredoc 2 words?].
    fn decode_redir(&mut self, data: u32) -> WcRedir {
        let rtype = data & REDIR_TYPE_MASK;
        let has_varid = (data & REDIR_VARID_MASK) != 0;
        let from_heredoc = (data & REDIR_FROM_HEREDOC_MASK) != 0;
        let fd = self.next().unwrap_or(0) as i32;
        let name = self.read_string();
        let varid = if has_varid {
            Some(self.read_string())
        } else {
            None
        };
        let heredoc = if from_heredoc {
            // parse.c stores 2 extra words for heredoc (terminator + body
            // string codes). Read and decode.
            let term_wc = self.next().unwrap_or(0);
            let body_wc = self.next().unwrap_or(0);
            let terminator = self.decode_string_word(term_wc);
            let content = self.decode_string_word(body_wc);
            // `quoted` flag isn't directly encoded in wordcode here — it
            // affects expansion at runtime, not the AST shape. Zshrs's AST
            // tracks it in `HereDocInfo.quoted`. For parity, render as
            // Unquoted on this side; if zshrs ever stores Quoted heredocs
            // verbatim differently, that's a real divergence to surface.
            Some(WcHeredoc {
                terminator,
                quoted: false,
                content,
            })
        } else {
            None
        };
        WcRedir {
            rtype,
            fd,
            name,
            varid,
            heredoc,
        }
    }

    /// `parse.c::par_assign` — WCB_ASSIGN(scalar_or_array, append, num).
    fn decode_assign(&mut self, data: u32) -> WcAssign {
        let is_array = (data & 1) == WC_ASSIGN_ARRAY;
        let append = ((data >> 1) & 1) == WC_ASSIGN_INC;
        let num = (data >> 2) as usize;
        let name = self.read_string();
        let value = if is_array {
            let mut vs = Vec::with_capacity(num);
            for _ in 0..num {
                vs.push(self.read_string());
            }
            WcAssignValue::Array(vs)
        } else {
            WcAssignValue::Scalar(self.read_string())
        };
        WcAssign {
            name,
            value,
            append,
        }
    }

    /// `parse.c:1619-1632` par_subsh — emits WC_SUBSH at p, an EXTRA word
    /// at p+1 (reserved for optional `always` block / WC_TRY wrapper),
    /// then the body lists, terminated by WCB_END().
    fn decode_subsh(&mut self, skip: u32) -> WcProgram {
        let end_pos = self.pos + skip as usize;
        // Consume the reserved-for-try word at p+1.
        let _ = self.next();
        let prog = self.decode_program_until(end_pos);
        self.pos = end_pos.min(self.code.len());
        prog
    }

    fn decode_cursh(&mut self, skip: u32) -> WcProgram {
        let end_pos = self.pos + skip as usize;
        // Same as par_subsh — extra word at p+1 (parse.c:1626).
        let _ = self.next();
        let prog = self.decode_program_until(end_pos);
        self.pos = end_pos.min(self.code.len());
        prog
    }

    /// `parse.c:1087-1198` par_for — three forms by `WC_FOR_TYPE`. Layout:
    /// - WC_FOR_LIST: n_vars (count), var_strcodes×n, n_iter_words, iter_strcodes×n
    /// - WC_FOR_COND: init/cond/step strcodes (no count words)
    /// - WC_FOR_PPARAM: n_vars (count), var_strcodes×n
    ///
    /// Followed by body lists.
    fn decode_for(&mut self, data: u32, _unused: bool) -> WcCommand {
        let ftype = data & 3;
        let skip = (data >> 2) as usize;
        let end_pos = self.pos + skip;
        let (var, list) = match ftype {
            WC_FOR_LIST => {
                let n_vars = self.next().unwrap_or(0) as usize;
                let mut vars = Vec::with_capacity(n_vars);
                for _ in 0..n_vars {
                    vars.push(self.read_string());
                }
                let n_iter = self.next().unwrap_or(0) as usize;
                let mut ws = Vec::with_capacity(n_iter);
                for _ in 0..n_iter {
                    ws.push(self.read_string());
                }
                // ZshFor.var is single — take first; multi-var for loops
                // aren't represented in the current AST, so multi-var
                // sources will surface as parity diffs.
                let var = vars.into_iter().next().unwrap_or_default();
                (var, WcForList::Words(ws))
            }
            WC_FOR_COND => {
                let init = self.read_string();
                let cond = self.read_string();
                let step = self.read_string();
                (
                    String::new(),
                    WcForList::CStyle { init, cond, step },
                )
            }
            _ => {
                // WC_FOR_PPARAM: n_vars (count), var_strcodes×n.
                let n_vars = self.next().unwrap_or(0) as usize;
                let mut vars = Vec::with_capacity(n_vars);
                for _ in 0..n_vars {
                    vars.push(self.read_string());
                }
                let var = vars.into_iter().next().unwrap_or_default();
                (var, WcForList::Positional)
            }
        };
        let body = self.decode_program_until(end_pos);
        self.pos = end_pos.min(self.code.len());
        WcCommand::For(WcFor {
            var,
            list,
            body: Box::new(body),
            is_select: false,
        })
    }

    fn decode_select(&mut self, data: u32) -> WcCommand {
        // par_for path with sel=1 — same layout as for-list/for-pparam,
        // minus the n_vars count slot for SELECT_PPARAM (par_for line
        // 1124: `if (!sel) np = ecadd(0)`). For SELECT, vars are emitted
        // raw without a count.
        let stype = data & 1;
        let skip = (data >> 1) as usize;
        let end_pos = self.pos + skip;
        let var = self.read_string();
        let list = if stype == WC_SELECT_LIST {
            let n = self.next().unwrap_or(0) as usize;
            let mut ws = Vec::with_capacity(n);
            for _ in 0..n {
                ws.push(self.read_string());
            }
            WcForList::Words(ws)
        } else {
            WcForList::Positional
        };
        let body = self.decode_program_until(end_pos);
        self.pos = end_pos.min(self.code.len());
        WcCommand::For(WcFor {
            var,
            list,
            body: Box::new(body),
            is_select: true,
        })
    }

    /// `parse.c:1521` par_while — body is [cond_program] [body_program].
    fn decode_while(&mut self, data: u32) -> WcCommand {
        let until = (data & 1) == WC_WHILE_UNTIL;
        let skip = (data >> 1) as usize;
        let end_pos = self.pos + skip;
        // The wordcode lays out: cond-list ... body-list ... but they're
        // separated only by structural markers. Both are full programs,
        // each terminating in WC_END. Read two programs back-to-back.
        let cond = self.decode_program_to_end();
        let body = self.decode_program_until(end_pos);
        self.pos = end_pos.min(self.code.len());
        let w = WcWhile { cond, body };
        if until {
            WcCommand::Until(w)
        } else {
            WcCommand::While(w)
        }
    }

    fn decode_repeat(&mut self, data: u32) -> WcRepeat {
        let skip = data as usize;
        let end_pos = self.pos + skip;
        let count = self.read_string();
        let body = self.decode_program_until(end_pos);
        self.pos = end_pos.min(self.code.len());
        WcRepeat { count, body }
    }

    /// `parse.c:1411-1519` par_if — chain of WC_IF entries:
    /// HEAD opens, IF head clause, ELIF for each elif, ELSE for the else.
    /// Each entry has its own skip count to its body terminator.
    fn decode_if(&mut self, data: u32) -> WcIf {
        // The data of the leading WC_IF holds the type and skip-to-end-of-if.
        let head_type = data & 3;
        let head_skip = (data >> 2) as usize;
        let if_end = self.pos + head_skip;

        // After WC_IF(HEAD), zsh emits subsequent WC_IF(IF), WC_IF(ELIF)*,
        // WC_IF(ELSE)? entries until reaching if_end.
        let _ = head_type; // HEAD is always the wrapper; substructure follows
        let mut cond = WcProgram { lists: vec![] };
        let mut then = WcProgram { lists: vec![] };
        let mut elif: Vec<(WcProgram, WcProgram)> = Vec::new();
        let mut else_: Option<WcProgram> = None;

        while self.pos < if_end {
            let wc = match self.peek() {
                Some(w) => w,
                None => break,
            };
            if wc_code(wc) != WC_IF {
                break;
            }
            self.next();
            let entry_data = wc_data(wc);
            let entry_type = entry_data & 3;
            let entry_skip = (entry_data >> 2) as usize;
            let entry_end = self.pos + entry_skip;

            match entry_type {
                WC_IF_IF => {
                    // followed by cond program then body program
                    cond = self.decode_program_to_end();
                    then = self.decode_program_until(entry_end);
                }
                WC_IF_ELIF => {
                    let c = self.decode_program_to_end();
                    let b = self.decode_program_until(entry_end);
                    elif.push((c, b));
                }
                WC_IF_ELSE => {
                    let b = self.decode_program_until(entry_end);
                    else_ = Some(b);
                }
                _ => {
                    // Unknown; consume entry's wordcode.
                }
            }
            self.pos = entry_end.min(self.code.len());
        }
        self.pos = if_end.min(self.code.len());

        WcIf {
            cond,
            then,
            elif,
            else_,
        }
    }

    /// `parse.c:1209-1407` par_case + `text.c:721-820` walker.
    /// HEAD encoding: `WC_CASE(HEAD, skip)` + word_strcode. Empty case has
    /// `pos >= end` immediately. Non-empty: each arm is
    /// `WC_CASE(arm_type, skip) n_alts (pat_strcode npats_word)×n_alts body...`.
    /// Multi-alternative arms (`(a|b|c)`) emit n_alts > 1.
    fn decode_case(&mut self, data: u32) -> WcCase {
        let _head_type = data & 7;
        let head_skip = (data >> WC_CASE_FREE) as usize;
        let case_end = self.pos + head_skip;
        let word = self.read_string();
        let mut arms: Vec<WcCaseArm> = Vec::new();
        while self.pos < case_end {
            let wc = match self.peek() {
                Some(w) => w,
                None => break,
            };
            if wc_code(wc) != WC_CASE {
                break;
            }
            self.next();
            let arm_data = wc_data(wc);
            let arm_type = arm_data & 7;
            let arm_skip = (arm_data >> WC_CASE_FREE) as usize;
            let arm_end = self.pos + arm_skip;
            // n_alts then (pat, npats) per alt. text.c:743-748.
            let n_alts = self.next().unwrap_or(0) as usize;
            let mut patterns = Vec::with_capacity(n_alts);
            for _ in 0..n_alts {
                patterns.push(self.read_string());
                let _ = self.next(); // ecnpats per pattern
            }
            let body = self.decode_program_until(arm_end);
            self.pos = arm_end.min(self.code.len());
            arms.push(WcCaseArm {
                patterns,
                body,
                terminator: arm_type,
            });
        }
        self.pos = case_end.min(self.code.len());
        WcCase { word, arms }
    }

    /// `parse.c:1672-1779` par_funcdef — names, metadata, body.
    /// Function body strings live at a NEW base offset (per `ecssub`/
    /// `ecsoffs` save/restore around par_list at parse.c:1739-1741).
    /// The metadata `strs_offset` tells us how much to advance strs_base
    /// for the duration of the body decode.
    fn decode_funcdef(&mut self, skip: u32) -> WcFuncDef {
        let end_pos = self.pos + skip as usize;
        let num = self.next().unwrap_or(0) as usize;
        let mut names = Vec::with_capacity(num);
        for _ in 0..num {
            names.push(self.read_string());
        }
        // Metadata words (parse.c:1751-1754):
        //   strs_offset = so - oecssub  (start of body's local strs)
        //   strs_len    = ecsoffs - so
        //   npats       = pattern count
        //   tracing     = -T flag
        let strs_offset = self.next().unwrap_or(0) as usize;
        let _strs_len = self.next();
        let _npats = self.next();
        let _tracing = self.next();

        let saved_base = self.strs_base;
        self.strs_base = saved_base + strs_offset;
        let body = self.decode_program_until(end_pos);
        self.strs_base = saved_base;
        self.pos = end_pos.min(self.code.len());
        WcFuncDef { names, body }
    }

    fn decode_timed(&mut self, data: u32) -> WcCommand {
        if data == 1 {
            // WC_TIMED_PIPE — a sublist follows.
            let sl = self.decode_sublist();
            WcCommand::Time(Some(Box::new(sl)))
        } else {
            WcCommand::Time(None)
        }
    }

    /// `parse.c::par_subsh` always-block branch + `text.c:984-1009`.
    /// Layout: outer WC_TRY(total_skip), inner WC_TRY(try_body_skip) at p+1,
    /// try body, then always body. Both halves are patched as WC_TRY by
    /// par_subsh (parse.c:1659/1661).
    fn decode_try(&mut self, outer_skip: u32) -> WcTry {
        let outer_end = self.pos + outer_skip as usize;
        // Inner WC_TRY at p+1: its data field is the try-body skip count.
        let inner_wc = self.next().unwrap_or(0);
        let inner_skip = wc_data(inner_wc) as usize;
        let try_end = self.pos + inner_skip;
        let try_block = self.decode_program_until(try_end);
        self.pos = try_end.min(self.code.len());
        let always = self.decode_program_until(outer_end);
        self.pos = outer_end.min(self.code.len());
        WcTry { try_block, always }
    }

    /// `parse.c::par_cond_double` / `par_cond_triple` — cond opcode encoding.
    ///
    /// | cond_type      | data | layout after WC_COND header |
    /// |---|---|---|
    /// | COND_NOT (0)   | 0    | recursive WC_COND for inner |
    /// | COND_AND (1)   | skip | left WC_COND, right WC_COND |
    /// | COND_OR  (2)   | skip | left WC_COND, right WC_COND |
    /// | COND_STREQ (3) | 0    | 2 strs + 1 ecnpats word |
    /// | COND_STRDEQ(4) | 0    | 2 strs + 1 ecnpats word |
    /// | COND_STRNEQ(5) | 0    | 2 strs + 1 ecnpats word |
    /// | COND_STRLT (6) | 0    | 2 strs + 1 ecnpats word |
    /// | COND_STRGTR(7) | 0    | 2 strs + 1 ecnpats word |
    /// | 8..16 (numeric)| 0    | 2 strs |
    /// | COND_REGEX(17) | 0    | 2 strs |
    /// | COND_MOD  (18) | 1 or 2 | 1+data strings (op_name + operand(s)) |
    /// | COND_MODI (19) | 0    | 3 strs (op + a + c) |
    /// | ASCII letter   | 0    | 1 str (unary file test like -f, -d) |
    fn decode_cond_expr(&mut self, data: u32) -> WcCond {
        let ctype = data & WC_COND_TYPE_MASK;
        let high = data >> 7;
        match ctype {
            COND_NOT => {
                let inner = self.read_inner_cond();
                WcCond::Not(Box::new(inner))
            }
            COND_AND => {
                let a = self.read_inner_cond();
                let b = self.read_inner_cond();
                WcCond::And(Box::new(a), Box::new(b))
            }
            COND_OR => {
                let a = self.read_inner_cond();
                let b = self.read_inner_cond();
                WcCond::Or(Box::new(a), Box::new(b))
            }
            3..=7 => {
                // STREQ, STRDEQ, STRNEQ, STRLT, STRGTR — 2 strs + ecnpats
                let x = self.read_string();
                let y = self.read_string();
                let _ecnpats = self.next();
                WcCond::Binary(x, cond_op_name(ctype).to_string(), y)
            }
            8..=17 => {
                // -nt, -ot, -ef, -eq, -ne, -lt, -gt, -le, -ge, =~ — 2 strs only
                let x = self.read_string();
                let y = self.read_string();
                WcCond::Binary(x, cond_op_name(ctype).to_string(), y)
            }
            18 => {
                // COND_MOD: data=high holds operand count (1 or 2).
                let op = self.read_string();
                let a = self.read_string();
                if high == 2 {
                    let b = self.read_string();
                    WcCond::Binary(a, op, b)
                } else {
                    WcCond::Unary(op, a)
                }
            }
            19 => {
                // COND_MODI: 3 strs (op, a, c)
                let op = self.read_string();
                let a = self.read_string();
                let b = self.read_string();
                WcCond::Binary(a, op, b)
            }
            _ => {
                // ASCII-letter unary file test — cond_type is the letter byte.
                let x = self.read_string();
                let mut op = String::from("-");
                op.push(ctype as u8 as char);
                WcCond::Unary(op, x)
            }
        }
    }

    fn read_inner_cond(&mut self) -> WcCond {
        match self.peek() {
            Some(w) if wc_code(w) == WC_COND => {
                self.next();
                self.decode_cond_expr(wc_data(w))
            }
            _ => WcCond::Unary(String::new(), String::new()),
        }
    }

    /// Decode one program until WC_END or a list with Z_END flag.
    fn decode_program_to_end(&mut self) -> WcProgram {
        let mut lists = Vec::new();
        while let Some(wc) = self.peek() {
            let code = wc_code(wc);
            if code == WC_END {
                self.next();
                break;
            }
            if code != WC_LIST {
                break;
            }
            let type_bits = wc_data(wc) & ((1 << WC_LIST_FREE) - 1);
            let is_z_end = (type_bits & Z_END) != 0;
            lists.push(self.decode_list());
            if is_z_end {
                break;
            }
        }
        WcProgram { lists }
    }
}

/// Map a `COND_*` numeric op back to its source-form string. Mirrors
/// `parse.c::par_cond` reverse mapping. Used only for canonical sexp
/// rendering — runtime cond execution lives elsewhere.
fn cond_op_name(t: u32) -> &'static str {
    match t {
        3 => "=",
        4 => "==",
        5 => "!=",
        6 => "<",
        7 => ">",
        8 => "-nt",
        9 => "-ot",
        10 => "-ef",
        11 => "-eq",
        12 => "-ne",
        13 => "-lt",
        14 => "-gt",
        15 => "-le",
        16 => "-ge",
        17 => "=~",
        // Unary tests: not numeric mapping in the usual sense; surface as
        // their COND_* index. Real source-form recovery would require the
        // module-system unary-op table, which lives outside parse.c.
        _ => "?",
    }
}

// ---------------------------------------------------------------------------
// Public entry: load .zwc + decode + render to canonical sexp
// ---------------------------------------------------------------------------

/// Decode the wordcode of a single function (or top-level script) into
/// a `WcProgram` tree. Reuses `zwc.rs::ZwcFile::load` for file structure.
pub fn decode_zwc_file<P: AsRef<Path>>(path: P) -> std::io::Result<Vec<(String, WcProgram)>> {
    let zwc = ZwcFile::load(path)?;
    let mut out = Vec::new();
    let header_words = zwc.header.header_len as usize;
    for func in &zwc.functions {
        let start_idx = (func.start as usize).saturating_sub(header_words);
        if start_idx >= zwc.wordcode.len() {
            continue;
        }
        let func_code = &zwc.wordcode[start_idx..];
        // Strings live as appended bytes after the wordcode of this function.
        // We materialize the function's words back into bytes so byte-offset
        // string lookups (long strings) resolve correctly.
        let mut sb: Vec<u8> = Vec::with_capacity(func_code.len() * 4);
        for &w in func_code {
            sb.extend_from_slice(&w.to_ne_bytes());
        }
        let mut walker = Walker::new(func_code, &sb, func.strs_offset as usize);
        let prog = walker.decode_program();
        out.push((func.name.clone(), prog));
    }
    Ok(out)
}

/// Convenience: load and decode the FIRST function (typical for a
/// `zcompile out.zwc input.zsh` script-style dump where there's exactly
/// one synthetic top-level function).
pub fn decode_zwc_first<P: AsRef<Path>>(path: P) -> std::io::Result<Option<WcProgram>> {
    let all = decode_zwc_file(path)?;
    Ok(all.into_iter().next().map(|(_, p)| p))
}

// ---------------------------------------------------------------------------
// Canonical sexp emitter — must produce IDENTICAL output to ast_sexp
// ---------------------------------------------------------------------------

pub fn wc_to_sexp(prog: &WcProgram) -> String {
    let mut out = String::new();
    emit_program(prog, &mut out);
    out
}

fn emit_program(p: &WcProgram, out: &mut String) {
    out.push_str("(Program");
    for l in &p.lists {
        out.push(' ');
        emit_list(l, out);
    }
    out.push(')');
}

fn emit_list(l: &WcList, out: &mut String) {
    out.push_str("(List ");
    let s = match (l.flags.async_, l.flags.disown) {
        (false, _) => "Sync",
        (true, false) => "Async",
        (true, true) => "Disown",
    };
    out.push_str(s);
    out.push(' ');
    emit_sublist(&l.sublist, out);
    out.push(')');
}

fn emit_sublist(sl: &WcSublist, out: &mut String) {
    out.push_str("(Sublist (");
    let mut first = true;
    if sl.flags.not {
        out.push_str("Not");
        first = false;
    }
    if sl.flags.coproc {
        if !first {
            out.push(' ');
        }
        out.push_str("Coproc");
    }
    out.push_str(") ");
    emit_pipe(&sl.pipe, out);
    if let Some((op, next)) = &sl.next {
        out.push(' ');
        out.push_str(match op {
            WcSublistOp::And => "And",
            WcSublistOp::Or => "Or",
        });
        out.push(' ');
        emit_sublist(next, out);
    }
    out.push(')');
}

fn emit_pipe(p: &WcPipe, out: &mut String) {
    out.push_str("(Pipe ");
    emit_cmd(&p.cmd, out);
    if let Some(next) = &p.next {
        out.push_str(" Pipe ");
        emit_pipe(next, out);
    }
    out.push(')');
}

fn emit_cmd(c: &WcCommand, out: &mut String) {
    match c {
        WcCommand::Simple(s) => emit_simple(s, out),
        WcCommand::Subsh(p) => {
            out.push_str("(Subsh ");
            emit_program(p, out);
            out.push(')');
        }
        WcCommand::Cursh(p) => {
            out.push_str("(Cursh ");
            emit_program(p, out);
            out.push(')');
        }
        WcCommand::For(f) => {
            let tag = if f.is_select { "Select" } else { "For" };
            out.push('(');
            out.push_str(tag);
            out.push(' ');
            emit_str(&f.var, out);
            out.push(' ');
            match &f.list {
                WcForList::Words(ws) => {
                    out.push_str("(Words");
                    for w in ws {
                        out.push(' ');
                        emit_str(w, out);
                    }
                    out.push(')');
                }
                WcForList::CStyle { init, cond, step } => {
                    out.push_str("(CStyle ");
                    emit_str(init, out);
                    out.push(' ');
                    emit_str(cond, out);
                    out.push(' ');
                    emit_str(step, out);
                    out.push(')');
                }
                WcForList::Positional => out.push_str("Positional"),
            }
            out.push(' ');
            emit_program(&f.body, out);
            out.push(')');
        }
        WcCommand::Case(c) => {
            out.push_str("(Case ");
            emit_str(&c.word, out);
            for arm in &c.arms {
                out.push_str(" (Arm (");
                let mut first = true;
                for p in &arm.patterns {
                    if !first {
                        out.push(' ');
                    }
                    emit_str(p, out);
                    first = false;
                }
                out.push_str(") ");
                out.push_str(match arm.terminator {
                    WC_CASE_OR => "Break",
                    WC_CASE_AND => "Continue",
                    WC_CASE_TESTAND => "TestNext",
                    _ => "Break",
                });
                out.push(' ');
                emit_program(&arm.body, out);
                out.push(')');
            }
            out.push(')');
        }
        WcCommand::If(i) => {
            out.push_str("(If ");
            emit_program(&i.cond, out);
            out.push(' ');
            emit_program(&i.then, out);
            for (c, b) in &i.elif {
                out.push_str(" (Elif ");
                emit_program(c, out);
                out.push(' ');
                emit_program(b, out);
                out.push(')');
            }
            if let Some(eb) = &i.else_ {
                out.push_str(" (Else ");
                emit_program(eb, out);
                out.push(')');
            }
            out.push(')');
        }
        WcCommand::While(w) => {
            out.push_str("(While ");
            emit_program(&w.cond, out);
            out.push(' ');
            emit_program(&w.body, out);
            out.push(')');
        }
        WcCommand::Until(w) => {
            out.push_str("(Until ");
            emit_program(&w.cond, out);
            out.push(' ');
            emit_program(&w.body, out);
            out.push(')');
        }
        WcCommand::Repeat(r) => {
            out.push_str("(Repeat ");
            emit_str(&r.count, out);
            out.push(' ');
            emit_program(&r.body, out);
            out.push(')');
        }
        WcCommand::FuncDef(f) => {
            out.push_str("(FuncDef (");
            let mut first = true;
            for n in &f.names {
                if !first {
                    out.push(' ');
                }
                emit_str(n, out);
                first = false;
            }
            out.push_str(") ");
            emit_program(&f.body, out);
            out.push(')');
        }
        WcCommand::Time(opt) => {
            out.push_str("(Time");
            if let Some(sl) = opt {
                out.push(' ');
                emit_sublist(sl, out);
            }
            out.push(')');
        }
        WcCommand::Cond(c) => {
            out.push_str("(Cond ");
            emit_cond(c, out);
            out.push(')');
        }
        WcCommand::Arith(s) => {
            out.push_str("(Arith ");
            emit_str(s, out);
            out.push(')');
        }
        WcCommand::Try(t) => {
            out.push_str("(Try ");
            emit_program(&t.try_block, out);
            out.push(' ');
            emit_program(&t.always, out);
            out.push(')');
        }
        WcCommand::AutoFn => out.push_str("(AutoFn)"),
    }
}

fn emit_cond(c: &WcCond, out: &mut String) {
    match c {
        WcCond::Not(inner) => {
            out.push_str("(Not ");
            emit_cond(inner, out);
            out.push(')');
        }
        WcCond::And(a, b) => {
            out.push_str("(And ");
            emit_cond(a, out);
            out.push(' ');
            emit_cond(b, out);
            out.push(')');
        }
        WcCond::Or(a, b) => {
            out.push_str("(Or ");
            emit_cond(a, out);
            out.push(' ');
            emit_cond(b, out);
            out.push(')');
        }
        WcCond::Unary(op, x) => {
            out.push_str("(Unary ");
            emit_str(op, out);
            out.push(' ');
            emit_str(x, out);
            out.push(')');
        }
        WcCond::Binary(x, op, y) => {
            out.push_str("(Binary ");
            emit_str(x, out);
            out.push(' ');
            emit_str(op, out);
            out.push(' ');
            emit_str(y, out);
            out.push(')');
        }
    }
}

fn emit_simple(s: &WcSimple, out: &mut String) {
    out.push_str("(Simple (Assigns");
    for a in &s.assigns {
        out.push(' ');
        out.push('(');
        out.push_str(if a.append { "Append " } else { "Set " });
        emit_str(&a.name, out);
        out.push(' ');
        match &a.value {
            WcAssignValue::Scalar(v) => {
                out.push_str("(Scalar ");
                emit_str(v, out);
                out.push(')');
            }
            WcAssignValue::Array(vs) => {
                out.push_str("(Array");
                for v in vs {
                    out.push(' ');
                    emit_str(v, out);
                }
                out.push(')');
            }
        }
        out.push(')');
    }
    out.push_str(") (Words");
    for w in &s.words {
        out.push(' ');
        emit_str(w, out);
    }
    out.push_str(") (Redirs");
    for r in &s.redirs {
        out.push(' ');
        emit_redir(r, out);
    }
    out.push_str("))");
}

fn emit_redir(r: &WcRedir, out: &mut String) {
    out.push('(');
    out.push_str(redir_tag(r.rtype));
    out.push(' ');
    out.push_str(&r.fd.to_string());
    out.push(' ');
    emit_str(&r.name, out);
    if let Some(v) = &r.varid {
        out.push(' ');
        emit_str(v, out);
    }
    if let Some(h) = &r.heredoc {
        out.push_str(" (Heredoc ");
        emit_str(&h.terminator, out);
        out.push(' ');
        out.push_str(if h.quoted { "Quoted" } else { "Unquoted" });
        out.push(' ');
        emit_str(&h.content, out);
        out.push(')');
    }
    out.push(')');
}

fn redir_tag(t: u32) -> &'static str {
    match t {
        REDIR_WRITE => "Write",
        REDIR_WRITENOW => "Writenow",
        REDIR_APP => "Append",
        REDIR_APPNOW => "Appendnow",
        REDIR_READ => "Read",
        REDIR_READWRITE => "ReadWrite",
        REDIR_HEREDOC => "Heredoc",
        REDIR_HEREDOCDASH => "HeredocDash",
        REDIR_HERESTR => "Herestr",
        REDIR_MERGEIN => "MergeIn",
        REDIR_MERGEOUT => "MergeOut",
        REDIR_ERRWRITE => "ErrWrite",
        REDIR_ERRWRITENOW => "ErrWritenow",
        REDIR_ERRAPP => "ErrAppend",
        REDIR_ERRAPPNOW => "ErrAppendnow",
        REDIR_INPIPE => "InPipe",
        REDIR_OUTPIPE => "OutPipe",
        _ => "Write",
    }
}

fn emit_str(s: &str, out: &mut String) {
    out.push('"');
    for b in s.bytes() {
        match b {
            b'\\' => out.push_str("\\\\"),
            b'"' => out.push_str("\\\""),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            0x20..=0x7e => out.push(b as char),
            _ => {
                use std::fmt::Write;
                let _ = write!(out, "\\x{:02x}", b);
            }
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    fn compile_via_zsh(src: &str) -> std::io::Result<(TempDir, std::path::PathBuf)> {
        let tmp = TempDir::new()?;
        let in_path = tmp.path().join("in.zsh");
        let out_path = tmp.path().join("out.zwc");
        std::fs::write(&in_path, src)?;
        let status = Command::new("zsh")
            .args([
                "-fc",
                &format!(
                    "zcompile {} {}",
                    out_path.to_str().unwrap(),
                    in_path.to_str().unwrap()
                ),
            ])
            .status()?;
        if !status.success() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("zcompile failed with exit {:?}", status.code()),
            ));
        }
        Ok((tmp, out_path))
    }

    #[test]
    fn loads_simple_echo() {
        let (_tmp, zwc) = compile_via_zsh("echo hello\n").expect("compile");
        let prog = decode_zwc_first(&zwc)
            .expect("load")
            .expect("first function");
        let s = wc_to_sexp(&prog);
        assert!(s.contains(r#"(Words "echo" "hello")"#), "got: {}", s);
    }
}
