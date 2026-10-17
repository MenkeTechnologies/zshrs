//! Rust-only utility (NOT a port — lives outside `src/ported/` by design).
//!
//! Original line numbers for code that runs out of a `.zwc` dump.
//!
//! C executes a dump's wordcode directly. Every pipeline in it carries the
//! line it was parsed on: `WCB_PIPE(type, toklineno + 1)` (`Src/parse.c:911`,
//! `:935`, `:944`), and the simple-command forms keep the same value as a bare
//! word after the list/sublist header (`set_list_code` / `set_sublist_code`,
//! `Src/parse.c:734-762`). `execpline` and `execsimple` load it back with
//! `lineno = WC_PIPE_LINENO(pcode) - 1` (`Src/exec.c:2057`) and
//! `lineno = code - 1` (`Src/exec.c:1356`), so `$LINENO`, error prefixes and
//! `$funcfiletrace` inside a dump-loaded function or a sourced `.zwc` report
//! the ORIGINAL source lines.
//!
//! zshrs runs a dump by deparsing it with `getpermtext` and re-parsing the
//! text. `gettext2` is a pretty-printer: it drops blank lines and comments,
//! splits `a; b` onto two lines and breaks `if X; then` into `if X` NEWLINE
//! `then`, so the re-parsed pipelines got the deparse's line numbers.
//!
//! This module records, while `gettext2` walks the wordcode, each pipeline's
//! stored line value together with how many newlines the deparse had emitted
//! before it, then rewrites `ZshPipe::lineno` in the re-parsed AST so every
//! pipeline carries the line C would have loaded. The AST is visited in the
//! same order `gettext2` walks the wordcode (text order == wordcode order);
//! when the counts disagree nothing is rewritten.

use crate::extensions::zsh_ast::{ZshCommand, ZshPipe, ZshProgram, ZshSublist};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::Mutex;

/// One pipeline as `gettext2` met it: the newlines already in the deparse
/// buffer, and the wordcode's stored line value (`toklineno + 1`, 0 = none).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
#[archive(check_bytes)]
pub struct PipeLine {
    pub newlines_before: u32,
    pub field: u32,
}

thread_local! {
    static RECORD: RefCell<Option<Vec<PipeLine>>> = const { RefCell::new(None) };
    /// (bytes of the deparse buffer already scanned, newlines in them).
    static SCAN: Cell<(usize, u32)> = const { Cell::new((0, 0)) };
    /// Lines the next `compile_script_isolated_at` parse is rewritten with.
    static ARMED: RefCell<Option<Vec<PipeLine>>> = const { RefCell::new(None) };
}

/// Lines of each function body that was installed from a dump deparse,
/// by function name. Set at load time, read at the call that compiles it.
static AUTOLOAD_LINES: Mutex<Option<HashMap<String, Vec<PipeLine>>>> = Mutex::new(None);

/// `getpermtext(prog, NULL, 0)` plus the pipeline line table of the text.
pub fn deparse(prog: crate::ported::zsh_h::eprog) -> (String, Vec<PipeLine>) {
    RECORD.with(|r| *r.borrow_mut() = Some(Vec::new()));
    SCAN.with(|s| s.set((0, 0)));
    let text = crate::ported::text::getpermtext(Box::new(prog), None, 0);
    let lines = RECORD.with(|r| r.borrow_mut().take()).unwrap_or_default();
    (text, lines)
}

/// True while [`deparse`] is collecting; checked by `gettext2` before it
/// touches the buffer.
pub fn recording() -> bool {
    RECORD.with(|r| r.borrow().is_some())
}

/// Called by `gettext2` at each pipeline's line word, with the deparse buffer
/// as it stands (the pipeline's own text not yet written).
pub fn note(buf: &[u8], field: u32) {
    let (scanned, mut count) = SCAN.with(|s| s.get());
    let from = if buf.len() < scanned {
        count = 0;
        0
    } else {
        scanned
    };
    count += buf[from..].iter().filter(|&&b| b == b'\n').count() as u32;
    SCAN.with(|s| s.set((buf.len(), count)));
    RECORD.with(|r| {
        if let Some(v) = r.borrow_mut().as_mut() {
            v.push(PipeLine {
                newlines_before: count,
                field,
            });
        }
    });
}

/// Remember (`Some`) or forget (`None`) the line table of `name`'s
/// dump-deparsed body.
pub fn note_autoload(name: &str, lines: Option<Vec<PipeLine>>) {
    let mut slot = AUTOLOAD_LINES.lock().unwrap_or_else(|e| e.into_inner());
    match lines {
        Some(l) => {
            slot.get_or_insert_with(HashMap::new).insert(name.to_string(), l);
        }
        None => {
            if let Some(map) = slot.as_mut() {
                map.remove(name);
            }
        }
    }
}

/// The line table [`note_autoload`] recorded for `name`.
pub fn autoload_lines(name: &str) -> Option<Vec<PipeLine>> {
    let slot = AUTOLOAD_LINES.lock().unwrap_or_else(|e| e.into_inner());
    slot.as_ref().and_then(|m| m.get(name).cloned())
}

/// Arms the line table for the next parse in `compile_script_isolated_at`
/// for the life of the guard.
pub struct Armed;

impl Armed {
    pub fn new(lines: Option<Vec<PipeLine>>) -> Self {
        ARMED.with(|a| *a.borrow_mut() = lines);
        Armed
    }
}

impl Drop for Armed {
    fn drop(&mut self) {
        ARMED.with(|a| *a.borrow_mut() = None);
    }
}

/// Consume the armed table (if any) and rewrite `program`'s pipeline lines.
pub fn apply_armed(program: &mut ZshProgram) {
    if let Some(lines) = ARMED.with(|a| a.borrow_mut().take()) {
        apply(program, &lines);
    }
}

/// A pipeline found in the AST: its line slot and the index (into the same
/// list) of the function definition whose body it sits in, if any.
struct Slot<'a> {
    lineno: &'a mut u64,
    frame: Option<usize>,
}

/// Rewrite every pipeline line in `program` from `lines`.
///
/// A zsh-style autoload is compiled as `name() {\n<body>\n}`, one pipeline
/// more than the dumped body; that wrapper is skipped and the body matched.
pub fn apply(program: &mut ZshProgram, lines: &[PipeLine]) {
    let total = {
        let mut all = Vec::new();
        walk_program(program, None, &mut all);
        all.len()
    };
    let target = match &mut program.lists[..] {
        [only] if total == lines.len() + 1 && is_wrapper(&only.sublist) => {
            match &mut only.sublist.pipe.cmd {
                ZshCommand::FuncDef(fd) => &mut *fd.body,
                _ => return,
            }
        }
        _ => program,
    };
    let mut slots = Vec::new();
    walk_program(target, None, &mut slots);
    if slots.len() != lines.len() {
        tracing::debug!(
            ast = slots.len(),
            wordcode = lines.len(),
            "zwc lineno: pipeline counts differ, lines left as deparsed"
        );
        return;
    }
    // The line each slot ends up with. A function header always precedes the
    // pipelines of its body, so its entry is final when they read it.
    let mut new_lines: Vec<i64> = Vec::with_capacity(slots.len());
    for (slot, line) in slots.into_iter().zip(lines) {
        let ast = *slot.lineno as i64;
        let field = line.field as i64;
        // C runs this pipeline at `field - 1` (Src/exec.c:2057). A function
        // body counts from 0 at its header (`lineno = 0`, Src/parse.c:1679 /
        // 2071) and zshrs reports a body line relative to the header's
        // pipeline, so a body pipeline sits `field - 1` past the header's
        // final line. At the top, the deparse placed the pipeline
        // `newlines_before` lines in where the source had it `field - 2`.
        let new = match (line.field, slot.frame) {
            (0, _) => ast,
            (_, Some(f)) => new_lines[f] + field - 1,
            (_, None) => ast - line.newlines_before as i64 + field - 2,
        }
        .max(0);
        *slot.lineno = new as u64;
        new_lines.push(new);
    }
}

fn is_wrapper(s: &ZshSublist) -> bool {
    s.next.is_none() && s.pipe.next.is_none() && matches!(s.pipe.cmd, ZshCommand::FuncDef(_))
}

fn walk_program<'a>(p: &'a mut ZshProgram, frame: Option<usize>, out: &mut Vec<Slot<'a>>) {
    for list in &mut p.lists {
        walk_sublist(&mut list.sublist, frame, out);
    }
}

fn walk_sublist<'a>(s: &'a mut ZshSublist, frame: Option<usize>, out: &mut Vec<Slot<'a>>) {
    walk_pipe(&mut s.pipe, frame, out);
    if let Some((_, next)) = &mut s.next {
        walk_sublist(next, frame, out);
    }
}

fn walk_pipe<'a>(p: &'a mut ZshPipe, frame: Option<usize>, out: &mut Vec<Slot<'a>>) {
    let here = out.len();
    out.push(Slot {
        lineno: &mut p.lineno,
        frame,
    });
    walk_command(&mut p.cmd, frame, here, out);
    if let Some(next) = &mut p.next {
        walk_pipe(next, frame, out);
    }
}

fn walk_command<'a>(
    c: &'a mut ZshCommand,
    frame: Option<usize>,
    here: usize,
    out: &mut Vec<Slot<'a>>,
) {
    match c {
        ZshCommand::Simple(_) | ZshCommand::Cond(_) | ZshCommand::Arith(_) => {}
        ZshCommand::Subsh(p) | ZshCommand::Cursh(p) => walk_program(p, frame, out),
        ZshCommand::For(f) => walk_program(&mut f.body, frame, out),
        ZshCommand::Case(cs) => {
            for arm in &mut cs.arms {
                walk_program(&mut arm.body, frame, out);
            }
        }
        ZshCommand::If(i) => {
            walk_program(&mut i.cond, frame, out);
            walk_program(&mut i.then, frame, out);
            for (c, b) in &mut i.elif {
                walk_program(c, frame, out);
                walk_program(b, frame, out);
            }
            if let Some(e) = &mut i.else_ {
                walk_program(e, frame, out);
            }
        }
        ZshCommand::While(w) | ZshCommand::Until(w) => {
            walk_program(&mut w.cond, frame, out);
            walk_program(&mut w.body, frame, out);
        }
        ZshCommand::Repeat(r) => walk_program(&mut r.body, frame, out),
        ZshCommand::FuncDef(fd) => walk_program(&mut fd.body, Some(here), out),
        ZshCommand::Time(s) => {
            if let Some(s) = s {
                walk_sublist(s, frame, out);
            }
        }
        ZshCommand::Try(t) => {
            walk_program(&mut t.try_block, frame, out);
            walk_program(&mut t.always, frame, out);
        }
        ZshCommand::Redirected(inner, _) => walk_command(inner, frame, here, out),
    }
}
