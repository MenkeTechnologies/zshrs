//! Recursive wordcode walker — decodes a `.zwc` blob into the parser's
//! own `ZshProgram` AST so the parity harness can compare zsh-side and
//! zshrs-side ASTs through a single sexp emitter (`ast_sexp`).
//!
//! ## Why this exists alongside `src/extensions/zwc.rs`
//!
//! `zwc.rs::WordcodeDecoder` is shallow: 12 of 14 body-bearing decoders
//! (Pipe, Sublist, FuncDef, For, While, If, Case, Cond, Try, Repeat,
//! Select) return `vec![]` for their body fields. Only `decode_subsh`
//! and `decode_cursh` walk children. Adequate for autoload introspection
//! (function-name lookup, flat opcode types), but cannot serve as a
//! parity oracle.
//!
//! This module reuses `zwc::ZwcFile::load()` for file-level parsing
//! (header, function table, wordcode buffer, string table) and adds a
//! recursive walker over the wordcode array, citing
//! `~/forkedRepos/zsh/Src/parse.c` and `~/forkedRepos/zsh/Src/zsh.h`
//! for every opcode interpretation. The walker emits directly into
//! `crate::extensions::zsh_ast::ZshProgram` — the same AST type the
//! parser produces — so both sides share `crate::extensions::ast_sexp`.
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

use crate::extensions::heredoc_ast::HereDocInfo;
use crate::extensions::zsh_ast::{
    CaseArm, CaseTerm, ForList, ListFlags, SublistFlags, SublistOp, ZshAssign, ZshAssignValue,
    ZshCase, ZshCommand, ZshCond, ZshFor, ZshFuncDef, ZshIf, ZshList, ZshPipe, ZshProgram,
    ZshRedir, ZshRepeat, ZshSimple, ZshSublist, ZshTry, ZshWhile,
};
use crate::zwc::{wc_code, wc_data, ZwcFile};
use std::path::Path;

// Canonical opcode / flag / redir constants live in `ported::zsh_h`
// (faithful port of `Src/zsh.h`). Re-import as u32 (matching the
// `wordcode` type) so they participate in bitwise ops with `wc_data()`
// output without inline casts.
use crate::ported::zsh_h::{
    WC_ARITH, WC_ASSIGN, WC_ASSIGN_ARRAY, WC_ASSIGN_INC, WC_AUTOFN, WC_CASE, WC_CASE_AND,
    WC_CASE_FREE, WC_CASE_OR, WC_CASE_TESTAND, WC_COND, WC_CURSH, WC_END, WC_FOR, WC_FOR_COND,
    WC_FOR_LIST, WC_FUNCDEF, WC_IF, WC_IF_ELIF, WC_IF_ELSE, WC_IF_IF, WC_LIST, WC_LIST_FREE,
    WC_PIPE, WC_PIPE_MID, WC_REDIR, WC_REPEAT, WC_SELECT, WC_SELECT_LIST, WC_SIMPLE, WC_SUBLIST,
    WC_SUBLIST_AND, WC_SUBLIST_FREE, WC_SUBLIST_NOT, WC_SUBLIST_OR, WC_SUBLIST_SIMPLE, WC_SUBSH,
    WC_TIMED, WC_TRY, WC_TYPESET, WC_WHILE, WC_WHILE_UNTIL,
};
// i32-typed in zsh_h (since C uses int for these flag/enum values).
// Rebind to u32 for bitwise ops against `wordcode` data.
const Z_ASYNC: u32 = crate::ported::zsh_h::Z_ASYNC as u32;
const Z_DISOWN: u32 = crate::ported::zsh_h::Z_DISOWN as u32;
const Z_END: u32 = crate::ported::zsh_h::Z_END as u32;
const Z_SIMPLE: u32 = crate::ported::zsh_h::Z_SIMPLE as u32;
const REDIR_TYPE_MASK: u32 = crate::ported::zsh_h::REDIR_TYPE_MASK as u32;
const REDIR_VARID_MASK: u32 = crate::ported::zsh_h::REDIR_VARID_MASK as u32;
const REDIR_FROM_HEREDOC_MASK: u32 = crate::ported::zsh_h::REDIR_FROM_HEREDOC_MASK as u32;
const COND_NOT: u32 = crate::ported::zsh_h::COND_NOT as u32;
const COND_AND: u32 = crate::ported::zsh_h::COND_AND as u32;
const COND_OR: u32 = crate::ported::zsh_h::COND_OR as u32;
// `zsh.h:1058` — `WC_COND_TYPE(C) = wc_data(C) & 127`. No standalone
// const in zsh_h; the macro inlines the literal. Kept local.
const WC_COND_TYPE_MASK: u32 = 127;

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
            // Keep the raw token bytes, widened byte→char (token bytes
            // 0x84..=0xa1 become the same `char` the zshrs lexer stores
            // in tokstr). Canonicalization to source text happens in
            // `ast_sexp::emit_str` via `ported::lex::untokenize`
            // (port of Src/exec.c:2077 + Src/lex.c:38 ztokens) — the
            // SAME function the zshrs-parser side goes through, so both
            // sides of the parity harness render tokens identically.
            // The previous pre-untokenize here used the byte-level
            // `zwc::untokenize`, which diverged from the canonical
            // table (no Qstring/Qtick/OutangProc arms, Bnull dropped
            // instead of `\`, no `$'...'` decode) and produced false
            // parity divergences.
            let mut s = String::new();
            for shift in [3, 11, 19] {
                let c = ((wc >> shift) & 0xff) as u8;
                if c == 0 {
                    break;
                }
                s.push(c as char);
            }
            s
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
        // Keep token bytes, widened byte→char — canonicalization happens
        // in `ast_sexp::emit_str` via `ported::lex::untokenize` so both
        // parity-harness sides share one untokenizer (see
        // decode_string_word above).
        let raw = &self.strings[offset..end];
        raw.iter().map(|&b| b as char).collect()
    }

    /// Top-level: walk a complete program until WC_END.
    /// Maps to `parse.c::par_event` driving `par_list`.
    fn decode_program(&mut self) -> ZshProgram {
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
        ZshProgram { lists }
    }

    /// Sub-program inside a known-bounded region (subsh body, for body, etc.).
    /// Reads WC_LIST entries until pos == end_pos, until WC_END, OR until
    /// a list with Z_END flag is consumed (per execlist's `if (ltype &
    /// Z_END) break;` at exec.c:1626 — Z_END marks the natural boundary
    /// between cond/body programs in WC_IF / WC_WHILE etc.).
    fn decode_program_until(&mut self, end_pos: usize) -> ZshProgram {
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
        ZshProgram { lists }
    }

    /// Decode one program until WC_END or a list with Z_END flag.
    fn decode_program_to_end(&mut self) -> ZshProgram {
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
        ZshProgram { lists }
    }

    /// `parse.c:771-817` par_list / par_list1 — emits `WCB_LIST(type, skip)`.
    /// Z_SIMPLE shortcut: body is `[lineno, single_pipe_body]` directly,
    /// with no inner WC_SUBLIST. Otherwise body starts with WC_SUBLIST.
    fn decode_list(&mut self) -> ZshList {
        let header = match self.next() {
            Some(h) if wc_code(h) == WC_LIST => h,
            _ => {
                return ZshList {
                    sublist: empty_sublist(),
                    flags: ListFlags::default(),
                };
            }
        };
        let data = wc_data(header);
        let type_bits = data & ((1 << WC_LIST_FREE) - 1);

        let async_ = (type_bits & Z_ASYNC) != 0;
        let disown = (type_bits & Z_DISOWN) != 0;
        let is_simple = (type_bits & Z_SIMPLE) != 0;

        let sublist = if is_simple {
            // Z_SIMPLE shortcut: lineno, then a single pipe stage body
            // (possibly with redirs/assigns), no WC_SUBLIST wrapper.
            let lineno = self.next().unwrap_or(0);
            // The body that follows is the inside of a single Pipe.
            // Synthesize a Sublist+Pipe wrapper so canonical AST matches.
            let cmd = self.decode_command_body();
            ZshSublist {
                pipe: ZshPipe {
                    cmd,
                    next: None,
                    lineno: lineno as u64,
                    merge_stderr: false,
                },
                next: None,
                flags: SublistFlags::default(),
            }
        } else {
            self.decode_sublist()
        };

        ZshList {
            sublist,
            flags: ListFlags { async_, disown },
        }
    }

    /// `parse.c:825-869` par_sublist — emits chained `WCB_SUBLIST` ops with
    /// types END/AND/OR. SUBLIST_SIMPLE flag means body is single-pipe
    /// shortcut (lineno + body without inner WC_PIPE).
    fn decode_sublist(&mut self) -> ZshSublist {
        let header = match self.peek() {
            Some(h) if wc_code(h) == WC_SUBLIST => {
                self.next();
                h
            }
            _ => return empty_sublist(),
        };
        let data = wc_data(header);
        let stype = data & 3;
        let flags_bits = data & 0x1c;
        let _skip = data >> WC_SUBLIST_FREE;

        let not = (flags_bits & WC_SUBLIST_NOT) != 0;
        let coproc = (flags_bits & WC_SUBLIST_FREE) != 0 && (flags_bits & 4) != 0;
        let _ = coproc; // recompute below to use canonical const
        let coproc = (flags_bits & crate::ported::zsh_h::WC_SUBLIST_COPROC) != 0;
        let is_simple = (flags_bits & WC_SUBLIST_SIMPLE) != 0;

        let pipe = if is_simple {
            let lineno = self.next().unwrap_or(0);
            let cmd = self.decode_command_body();
            ZshPipe {
                cmd,
                next: None,
                lineno: lineno as u64,
                merge_stderr: false,
            }
        } else {
            self.decode_pipe()
        };

        let next = match stype {
            x if x == WC_SUBLIST_AND => Some((SublistOp::And, Box::new(self.decode_sublist()))),
            x if x == WC_SUBLIST_OR => Some((SublistOp::Or, Box::new(self.decode_sublist()))),
            _ => None, // WC_SUBLIST_END
        };

        ZshSublist {
            pipe,
            next,
            flags: SublistFlags { coproc, not },
        }
    }

    /// `parse.c:894-944` par_pline — chained `WCB_PIPE(MID|END, lineno)`.
    /// MID stages have an extra skip-count word inserted at p+1 (per
    /// parse.c:912-913 ecispace) — consume it. END stages have no extra.
    fn decode_pipe(&mut self) -> ZshPipe {
        let header = match self.peek() {
            Some(h) if wc_code(h) == WC_PIPE => {
                self.next();
                h
            }
            _ => {
                return ZshPipe {
                    cmd: ZshCommand::Simple(ZshSimple {
                        assigns: vec![],
                        words: vec![],
                        redirs: vec![],
                    }),
                    next: None,
                    lineno: 0,
                    merge_stderr: false,
                };
            }
        };
        let data = wc_data(header);
        let ptype = data & 1;
        let lineno = (data >> 1) as u64;

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

        ZshPipe {
            cmd,
            next,
            lineno,
            merge_stderr: false,
        }
    }

    /// Decode the command body of a Pipe stage. Per `parse.c:958-1085`
    /// par_cmd dispatch: SIMPLE has leading assigns/redirs interleaved
    /// with WC_SIMPLE; other compounds have the command-leading opcode
    /// first.
    fn decode_command_body(&mut self) -> ZshCommand {
        let mut leading_assigns: Vec<ZshAssign> = Vec::new();
        let mut leading_redirs: Vec<ZshRedir> = Vec::new();

        loop {
            let wc = match self.peek() {
                Some(w) => w,
                None => {
                    return ZshCommand::Simple(ZshSimple {
                        assigns: leading_assigns,
                        words: vec![],
                        redirs: leading_redirs,
                    });
                }
            };
            let code = wc_code(wc);
            match code {
                x if x == WC_ASSIGN => {
                    self.next();
                    leading_assigns.push(self.decode_assign(wc_data(wc)));
                }
                x if x == WC_REDIR => {
                    self.next();
                    leading_redirs.push(self.decode_redir(wc_data(wc)));
                }
                x if x == WC_SIMPLE => {
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
                    return ZshCommand::Simple(ZshSimple {
                        assigns: leading_assigns,
                        words,
                        redirs: leading_redirs,
                    });
                }
                x if x == WC_TYPESET => {
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
                    return ZshCommand::Simple(ZshSimple {
                        assigns: leading_assigns,
                        words,
                        redirs: leading_redirs,
                    });
                }
                _ => {
                    // Compound command. Decode it; if there were leading
                    // redirs, wrap the result in ZshCommand::Redirected.
                    let cmd = self.decode_compound();
                    if leading_redirs.is_empty() {
                        return cmd;
                    }
                    return ZshCommand::Redirected(Box::new(cmd), leading_redirs);
                }
            }
        }
    }

    fn decode_compound(&mut self) -> ZshCommand {
        let wc = match self.next() {
            Some(w) => w,
            None => {
                return ZshCommand::Simple(ZshSimple {
                    assigns: vec![],
                    words: vec![],
                    redirs: vec![],
                });
            }
        };
        let code = wc_code(wc);
        let data = wc_data(wc);
        match code {
            x if x == WC_SUBSH => ZshCommand::Subsh(Box::new(self.decode_subsh(data))),
            x if x == WC_CURSH => ZshCommand::Cursh(Box::new(self.decode_cursh(data))),
            x if x == WC_FOR => self.decode_for(data),
            x if x == WC_SELECT => self.decode_select(data),
            x if x == WC_WHILE => self.decode_while(data),
            x if x == WC_REPEAT => ZshCommand::Repeat(self.decode_repeat(data)),
            x if x == WC_IF => ZshCommand::If(self.decode_if(data)),
            x if x == WC_CASE => ZshCommand::Case(self.decode_case(data)),
            x if x == WC_FUNCDEF => ZshCommand::FuncDef(self.decode_funcdef(data)),
            x if x == WC_TIMED => self.decode_timed(data),
            x if x == WC_COND => ZshCommand::Cond(self.decode_cond_expr(data)),
            x if x == WC_ARITH => ZshCommand::Arith(self.read_string()),
            x if x == WC_AUTOFN => {
                // Autoload-stub function body (zsh.h: WC_AUTOFN). Stored
                // in the shfunc's eprog, not in compiled scripts. No
                // ZshCommand variant — emit a placeholder Simple so parity
                // diff surfaces if it ever appears in a script blob.
                ZshCommand::Simple(ZshSimple {
                    assigns: vec![],
                    words: vec![],
                    redirs: vec![],
                })
            }
            x if x == WC_TRY => ZshCommand::Try(self.decode_try(data)),
            _ => ZshCommand::Simple(ZshSimple {
                assigns: vec![],
                words: vec![],
                redirs: vec![],
            }),
        }
    }

    /// `parse.c::par_redir` — emits WCB_REDIR(type|flags) + fd + target_str
    /// + [varid_str?] + [heredoc 2 words?].
    fn decode_redir(&mut self, data: u32) -> ZshRedir {
        let rtype = (data & REDIR_TYPE_MASK) as i32;
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
            // affects expansion at runtime, not the AST shape. zshrs's
            // AST tracks it in `HereDocInfo.quoted`. For parity, render
            // as unquoted on this side; if zshrs ever stores Quoted
            // heredocs verbatim differently, that's a real divergence to
            // surface.
            Some(HereDocInfo {
                content,
                terminator,
                quoted: false,
            })
        } else {
            None
        };
        ZshRedir {
            rtype,
            fd,
            name,
            heredoc,
            varid,
            heredoc_idx: None,
        }
    }

    /// `parse.c::par_assign` — WCB_ASSIGN(scalar_or_array, append, num).
    fn decode_assign(&mut self, data: u32) -> ZshAssign {
        let is_array = (data & 1) == WC_ASSIGN_ARRAY;
        let append = ((data >> 1) & 1) == WC_ASSIGN_INC;
        let num = (data >> 2) as usize;
        let name = self.read_string();
        let value = if is_array {
            let mut vs = Vec::with_capacity(num);
            for _ in 0..num {
                vs.push(self.read_string());
            }
            ZshAssignValue::Array(vs)
        } else {
            ZshAssignValue::Scalar(self.read_string())
        };
        ZshAssign {
            name,
            value,
            append,
        }
    }

    /// `parse.c:1619-1632` par_subsh — emits WC_SUBSH at p, an EXTRA word
    /// at p+1 (reserved for optional `always` block / WC_TRY wrapper),
    /// then the body lists, terminated by WCB_END().
    fn decode_subsh(&mut self, skip: u32) -> ZshProgram {
        let end_pos = self.pos + skip as usize;
        // Consume the reserved-for-try word at p+1.
        let _ = self.next();
        let prog = self.decode_program_until(end_pos);
        self.pos = end_pos.min(self.code.len());
        prog
    }

    fn decode_cursh(&mut self, skip: u32) -> ZshProgram {
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
    fn decode_for(&mut self, data: u32) -> ZshCommand {
        let ftype = data & 3;
        let skip = (data >> 2) as usize;
        let end_pos = self.pos + skip;
        let (var, list) = match ftype {
            x if x == WC_FOR_LIST => {
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
                (var, ForList::Words(ws))
            }
            x if x == WC_FOR_COND => {
                let init = self.read_string();
                let cond = self.read_string();
                let step = self.read_string();
                (String::new(), ForList::CStyle { init, cond, step })
            }
            _ => {
                // WC_FOR_PPARAM: n_vars (count), var_strcodes×n.
                let n_vars = self.next().unwrap_or(0) as usize;
                let mut vars = Vec::with_capacity(n_vars);
                for _ in 0..n_vars {
                    vars.push(self.read_string());
                }
                let var = vars.into_iter().next().unwrap_or_default();
                (var, ForList::Positional)
            }
        };
        let body = self.decode_program_until(end_pos);
        self.pos = end_pos.min(self.code.len());
        ZshCommand::For(ZshFor {
            var,
            list,
            body: Box::new(body),
            is_select: false,
        })
    }

    fn decode_select(&mut self, data: u32) -> ZshCommand {
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
            ForList::Words(ws)
        } else {
            ForList::Positional
        };
        let body = self.decode_program_until(end_pos);
        self.pos = end_pos.min(self.code.len());
        ZshCommand::For(ZshFor {
            var,
            list,
            body: Box::new(body),
            is_select: true,
        })
    }

    /// `parse.c:1521` par_while — body is [cond_program] [body_program].
    fn decode_while(&mut self, data: u32) -> ZshCommand {
        let until = (data & 1) == WC_WHILE_UNTIL;
        let skip = (data >> 1) as usize;
        let end_pos = self.pos + skip;
        // The wordcode lays out: cond-list ... body-list ... separated
        // only by structural markers. Both are full programs, each
        // terminating in WC_END. Read two programs back-to-back.
        let cond = self.decode_program_to_end();
        let body = self.decode_program_until(end_pos);
        self.pos = end_pos.min(self.code.len());
        ZshCommand::While(ZshWhile {
            cond: Box::new(cond),
            body: Box::new(body),
            until,
        })
    }

    fn decode_repeat(&mut self, data: u32) -> ZshRepeat {
        let skip = data as usize;
        let end_pos = self.pos + skip;
        let count = self.read_string();
        let body = self.decode_program_until(end_pos);
        self.pos = end_pos.min(self.code.len());
        ZshRepeat {
            count,
            body: Box::new(body),
        }
    }

    /// `parse.c:1411-1519` par_if — chain of WC_IF entries:
    /// HEAD opens, IF head clause, ELIF for each elif, ELSE for the else.
    /// Each entry has its own skip count to its body terminator.
    fn decode_if(&mut self, data: u32) -> ZshIf {
        let head_skip = (data >> 2) as usize;
        let if_end = self.pos + head_skip;

        let mut cond = ZshProgram { lists: vec![] };
        let mut then = ZshProgram { lists: vec![] };
        let mut elif: Vec<(ZshProgram, ZshProgram)> = Vec::new();
        let mut else_: Option<Box<ZshProgram>> = None;

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
                x if x == WC_IF_IF => {
                    cond = self.decode_program_to_end();
                    then = self.decode_program_until(entry_end);
                }
                x if x == WC_IF_ELIF => {
                    let c = self.decode_program_to_end();
                    let b = self.decode_program_until(entry_end);
                    elif.push((c, b));
                }
                x if x == WC_IF_ELSE => {
                    let b = self.decode_program_until(entry_end);
                    else_ = Some(Box::new(b));
                }
                _ => {}
            }
            self.pos = entry_end.min(self.code.len());
        }
        self.pos = if_end.min(self.code.len());

        ZshIf {
            cond: Box::new(cond),
            then: Box::new(then),
            elif,
            else_,
        }
    }

    /// `parse.c:1209-1407` par_case + `text.c:721-820` walker.
    /// HEAD encoding: `WC_CASE(HEAD, skip)` + word_strcode. Empty case has
    /// `pos >= end` immediately. Non-empty: each arm is
    /// `WC_CASE(arm_type, skip) n_alts (pat_strcode npats_word)×n_alts body...`.
    /// Multi-alternative arms (`(a|b|c)`) emit n_alts > 1.
    fn decode_case(&mut self, data: u32) -> ZshCase {
        let head_skip = (data >> WC_CASE_FREE) as usize;
        let case_end = self.pos + head_skip;
        let word = self.read_string();
        let mut arms: Vec<CaseArm> = Vec::new();
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
            let terminator = match arm_type {
                x if x == WC_CASE_OR => CaseTerm::Break,
                x if x == WC_CASE_AND => CaseTerm::Continue,
                x if x == WC_CASE_TESTAND => CaseTerm::TestNext,
                _ => CaseTerm::Break,
            };
            arms.push(CaseArm {
                patterns,
                body,
                terminator,
            });
        }
        self.pos = case_end.min(self.code.len());
        ZshCase { word, arms }
    }

    /// `parse.c:1672-1779` par_funcdef — names, metadata, body.
    /// Function body strings live at a NEW base offset (per `ecssub`/
    /// `ecsoffs` save/restore around par_list at parse.c:1739-1741).
    /// The metadata `strs_offset` tells us how much to advance strs_base
    /// for the duration of the body decode.
    fn decode_funcdef(&mut self, skip: u32) -> ZshFuncDef {
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
        let tracing_word = self.next().unwrap_or(0);

        let saved_base = self.strs_base;
        self.strs_base = saved_base + strs_offset;
        let body = self.decode_program_until(end_pos);
        self.strs_base = saved_base;
        self.pos = end_pos.min(self.code.len());
        ZshFuncDef {
            names,
            body: Box::new(body),
            tracing: tracing_word != 0,
            auto_call_args: None,
            body_source: None,
        }
    }

    fn decode_timed(&mut self, data: u32) -> ZshCommand {
        if data == 1 {
            // WC_TIMED_PIPE — a sublist follows.
            let sl = self.decode_sublist();
            ZshCommand::Time(Some(Box::new(sl)))
        } else {
            ZshCommand::Time(None)
        }
    }

    /// `parse.c::par_subsh` always-block branch + `text.c:984-1009`.
    /// Layout: outer WC_TRY(total_skip), inner WC_TRY(try_body_skip) at p+1,
    /// try body, then always body. Both halves are patched as WC_TRY by
    /// par_subsh (parse.c:1659/1661).
    fn decode_try(&mut self, outer_skip: u32) -> ZshTry {
        let outer_end = self.pos + outer_skip as usize;
        // Inner WC_TRY at p+1: its data field is the try-body skip count.
        let inner_wc = self.next().unwrap_or(0);
        let inner_skip = wc_data(inner_wc) as usize;
        let try_end = self.pos + inner_skip;
        let try_block = self.decode_program_until(try_end);
        self.pos = try_end.min(self.code.len());
        let always = self.decode_program_until(outer_end);
        self.pos = outer_end.min(self.code.len());
        ZshTry {
            try_block: Box::new(try_block),
            always: Box::new(always),
        }
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
    fn decode_cond_expr(&mut self, data: u32) -> ZshCond {
        let ctype = data & WC_COND_TYPE_MASK;
        let high = data >> 7;
        match ctype {
            x if x == COND_NOT => {
                let inner = self.read_inner_cond();
                ZshCond::Not(Box::new(inner))
            }
            x if x == COND_AND => {
                let a = self.read_inner_cond();
                let b = self.read_inner_cond();
                ZshCond::And(Box::new(a), Box::new(b))
            }
            x if x == COND_OR => {
                let a = self.read_inner_cond();
                let b = self.read_inner_cond();
                ZshCond::Or(Box::new(a), Box::new(b))
            }
            3..=7 => {
                // STREQ, STRDEQ, STRNEQ, STRLT, STRGTR — 2 strs + ecnpats
                let x = self.read_string();
                let y = self.read_string();
                let _ecnpats = self.next();
                ZshCond::Binary(x, cond_op_name(ctype).to_string(), y)
            }
            8..=16 => {
                // -nt, -ot, -ef, -eq, -ne, -lt, -gt, -le, -ge — 2 strs only
                let x = self.read_string();
                let y = self.read_string();
                ZshCond::Binary(x, cond_op_name(ctype).to_string(), y)
            }
            17 => {
                // =~ regex — distinct ZshCond variant.
                let x = self.read_string();
                let y = self.read_string();
                ZshCond::Regex(x, y)
            }
            18 => {
                // COND_MOD: data=high holds operand count (1 or 2).
                let op = self.read_string();
                let a = self.read_string();
                if high == 2 {
                    let b = self.read_string();
                    ZshCond::Binary(a, op, b)
                } else {
                    ZshCond::Unary(op, a)
                }
            }
            19 => {
                // COND_MODI: 3 strs (op, a, c)
                let op = self.read_string();
                let a = self.read_string();
                let b = self.read_string();
                ZshCond::Binary(a, op, b)
            }
            _ => {
                // ASCII-letter unary file test — cond_type is the letter byte.
                let x = self.read_string();
                let mut op = String::from("-");
                op.push(ctype as u8 as char);
                ZshCond::Unary(op, x)
            }
        }
    }

    fn read_inner_cond(&mut self) -> ZshCond {
        match self.peek() {
            Some(w) if wc_code(w) == WC_COND => {
                self.next();
                self.decode_cond_expr(wc_data(w))
            }
            _ => ZshCond::Unary(String::new(), String::new()),
        }
    }
}

fn empty_sublist() -> ZshSublist {
    ZshSublist {
        pipe: ZshPipe {
            cmd: ZshCommand::Simple(ZshSimple {
                assigns: vec![],
                words: vec![],
                redirs: vec![],
            }),
            next: None,
            lineno: 0,
            merge_stderr: false,
        },
        next: None,
        flags: SublistFlags::default(),
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
// Public entry: load .zwc + decode into ZshProgram (parser AST type)
// ---------------------------------------------------------------------------

/// Decode an entire `.zwc` file into typed `ZshProgram`s.
/// zshrs-original tooling — used by the parity harness to compare
/// zsh's wordcode output against zshrs's parser via the single
/// `ast_sexp::ast_to_sexp` emitter. C zsh has no equivalent decoder;
/// it just re-evaluates from wordcode.
pub fn decode_zwc_file<P: AsRef<Path>>(path: P) -> std::io::Result<Vec<(String, ZshProgram)>> {
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

/// Decode the first function out of a `.zwc` file.
/// zshrs-original tooling — convenience for fixtures.
pub fn decode_zwc_first<P: AsRef<Path>>(path: P) -> std::io::Result<Option<ZshProgram>> {
    let all = decode_zwc_file(path)?;
    Ok(all.into_iter().next().map(|(_, p)| p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::ast_sexp::ast_to_sexp;
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
            return Err(std::io::Error::other(format!(
                "zcompile failed with exit {:?}",
                status.code()
            )));
        }
        Ok((tmp, out_path))
    }

    #[test]
    fn loads_simple_echo() {
        let _g = crate::test_util::global_state_lock();
        let (_tmp, zwc) = compile_via_zsh("echo hello\n").expect("compile");
        let prog = decode_zwc_first(&zwc)
            .expect("load")
            .expect("first function");
        let s = ast_to_sexp(&prog);
        assert!(
            s.contains(r#""echo""#) && s.contains(r#""hello""#),
            "got: {}",
            s
        );
    }
}
