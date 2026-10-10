//! csh control structures with block nesting.
//!
//! Every rule below was checked against /bin/tcsh. Deliberate differences
//! from tcsh are listed per construct and come from tcsh re-reading the
//! rest of a `;`-joined line when it loops or skips a block (for example
//! `foreach i (1 2); echo $i; end` runs once under tcsh); here `;` is
//! always a plain statement separator.
//!
//! Mapping:
//!   * `if (e) then` / `else if (e) then` / `else` / `endif`
//!     → `if` / `elif` / `else` / `fi`
//!   * `if (e) cmd` → `if c; then cmd; fi`. `cmd` is one pipeline: it ends
//!     at `;`, `&&`, `||` or `&`, and the operator then applies to the whole
//!     `if` (tcsh: `if (0) echo a && echo c` prints `c`).
//!   * `if e cmd` (no parentheses) → the same. The expression is the
//!     longest run of `operand (binary-op operand)*` words, with `!`, `~`
//!     and `-X` file tests as prefixes; the first word that cannot extend it
//!     starts the command (`if 1 == 1 echo a`, `if -e f echo a`).
//!   * `foreach v (w...)` / `end` → `for v in w...; do` / `done`
//!   * `while (e)` / `end` → `while c; do` / `done`
//!   * `repeat N cmd` → `repeat N; do cmd; done`
//!   * `switch (s)` … `endsw` → a one-iteration `for` loop. A `case` picks
//!     the index of the first matching label in source order (`default:`
//!     matches when reached, so later labels are only reachable by
//!     fallthrough); each label then opens a guard
//!     `if (( m && m <= k ))`, which gives csh fallthrough, and `breaksw`
//!     is `break`. The whole switch is buffered until its `endsw` because
//!     the dispatch needs every label. The switch word is globbed like any
//!     csh word: several matches give `W: Ambiguous.`, none `W: No match.`,
//!     an unquoted `{a,b}` list `W: Ambiguous.`, more than one word
//!     `Syntax Error.`.
//!   * `break` / `continue` count the `switch` loops and goto dispatchers
//!     between the statement and the csh loop (`break 2`).
//!   * `goto` / `label:` — the whole input becomes a dispatcher loop whose
//!     `case` arms are the labels (`;&` fallthrough); `goto` assigns the
//!     arm and `continue`s the dispatcher. Needs the whole input, so
//!     `finish` re-translates the retained lines when a `goto` or
//!     `onintr label` was seen. A block whose body holds a label gets its
//!     own dispatcher around that body (`foreach`/`while`/`if` branches and
//!     `case` bodies), so a `goto` from inside the body, or from a nested
//!     block, to a label of an enclosing body works, forward and backward
//!     (`goto next` to a `next:` later in the same `foreach` body). A label
//!     in an `if` branch can also be reached from outside the `if`, like
//!     tcsh: the branch is entered as if its condition had held and a later
//!     `else` skips to `endif` (`__csh_in<n>` forces the branch, `__blk<n>`
//!     is the entry arm in front of the `if`). `goto $var` is a `case` over
//!     the labels inside blocks, then the top-level dispatcher.
//!   * `label:` resets `$status` to 0, with or without a `goto`.
//!   * `onintr label` → `trap '__csh_pc=label; continue 1000' INT`:
//!     `continue 1000` unwinds to the outermost loop, which is the goto
//!     dispatcher. `onintr -` ignores INT, bare `onintr` restores it.
//!   * `$status`: tcsh resets it to 0 on entering any branch, loop body and
//!     after `endif`/`end`/`endsw`. When the input mentions `status`,
//!     `finish` re-translates with `:` inserted at those points.
//!
//! Errors tcsh raises while executing a line (`break: Not in while/foreach.`,
//! `if: Empty if.`, …) become a runtime stub (`print -ru2 -- msg; exit 1`)
//! so output produced before the line still happens, like tcsh.
//!
//! End of input inside an open block. `finish` returns `Err` with tcsh's
//! text (`then: then/endif not found.`, `foreach: end not found.`,
//! `while: end not found.`, `switch: endsw not found.`, `else: endif not
//! found.`, every one ending in `not found.`) so a prompt reader can keep
//! collecting lines (`super::translate` and the prompt reader in
//! `bins/zshrs.rs` use `finish`). `finish_eof` is for input that really
//! ended (a script file, a `-c` string): it reproduces what tcsh does — it
//! runs the body that was reached and fails only when it must skip forward
//! over the missing closer:
//!   * `if (1) then` … EOF runs silently; false with no `else` fails with
//!     `then: then/endif not found.`; a taken `then` branch that reaches
//!     `else`/`else if` fails with `else: endif not found.`.
//!   * `foreach`/`while` … EOF runs the first iteration, then the script
//!     ends (status of the last command); `continue` starts the next
//!     iteration, and after the last one fails with `continue: end not
//!     found.` (`while: end not found.` for `while`); an empty `foreach`
//!     list or a false `while` fails with `foreach: end not found.` /
//!     `while: end not found.`; `break` fails with `break: end not found.`.
//!   * `switch` … EOF runs the matched (or default) case silently; no match
//!     fails with `switch: endsw not found.`; `breaksw` fails with
//!     `breaksw: endsw not found.`.
//!
//! Not reproduced:
//!   * `goto` to a label inside a `foreach`/`while`/`switch` body from
//!     outside that body: tcsh runs the rest of the body and then fails at
//!     `end` with `end: Not in while/foreach.` (a `foreach` variable is
//!     `i: Undefined variable.`; a switch body runs to `breaksw` and skips
//!     to `endsw`). Becomes a runtime `goto: L: jumping into a block is not
//!     supported` error, exit 1; `onintr` naming a label inside a block is
//!     the same error when the `onintr` runs.
//!   * `onintr` (no argument) followed by a real SIGINT: tcsh exits 1, zsh
//!     is killed by the signal (130).
//!   * `if (0) foreach …` / `if (0) while …` / `if (0) switch …`: tcsh
//!     skips only the opener, so the body lines run unconditionally and
//!     `end` fails with `end: Not in while/foreach.` (`foreach` body:
//!     `i: Undefined variable.`; a `switch` body runs every case); here the
//!     block nests under the `if`. `if (0) if (1) then` IS reproduced (the
//!     body runs unconditionally).
//!   * The switch word checks (`Ambiguous.`, `No match.`, `Syntax Error.`)
//!     cover a literal word and a bare `$name`; a glob or list that only
//!     appears after variable expansion is not checked.

use std::collections::{HashMap, HashSet};

use super::lex::split_words;
use super::{cmds, expr, words};

/// Region id of the top level (the outermost goto dispatcher).
const TOP_REGION: usize = 0;
/// Region id of code a `goto` can never target: a switch body before its
/// first label, the dead part after a second `else`.
const NO_REGION: usize = usize::MAX;

/// Open block on the translation stack. `bid` numbers blocks in source
/// order (stable across passes); `rid` is the region (branch body) the
/// frame is currently inside, see [`Region`].
enum Frame {
    /// zsh `if`; `extra_fi` counts dead `if false; then` nests opened by
    /// repeated `else` (tcsh skips everything after a second `else`).
    If {
        has_else: bool,
        extra_fi: usize,
        bid: usize,
        rid: usize,
    },
    /// zsh `for`/`while` from csh `foreach`/`while`; `kw` names it in the
    /// "end not found" error.
    Loop {
        kw: &'static str,
        bid: usize,
        rid: usize,
    },
    /// One-iteration `for` carrying a csh `switch`.
    Switch {
        id: usize,
        bid: usize,
        rid: usize,
        labels_seen: usize,
        body_open: bool,
        body_empty: bool,
    },
    /// `if c; then` opened by a one-line `if (c) <block opener>`; closed
    /// with `fi` right after the block it guards.
    Wrap,
}

impl Frame {
    fn bid(&self) -> Option<usize> {
        match self {
            Frame::If { bid, .. } | Frame::Loop { bid, .. } | Frame::Switch { bid, .. } => {
                Some(*bid)
            }
            Frame::Wrap => None,
        }
    }

    fn rid(&self) -> Option<usize> {
        match self {
            Frame::If { rid, .. } | Frame::Loop { rid, .. } | Frame::Switch { rid, .. } => {
                Some(*rid)
            }
            Frame::Wrap => None,
        }
    }

    fn set_rid(&mut self, new: usize) {
        match self {
            Frame::If { rid, .. } | Frame::Loop { rid, .. } | Frame::Switch { rid, .. } => {
                *rid = new;
            }
            Frame::Wrap => {}
        }
    }
}

/// A body that contains goto labels and therefore runs inside its own
/// dispatcher: `pcN=''; while :; do case "$pcN" in ('') … ;& (label) … esac;
/// break; done`. `owner` is the stack index of the frame whose current
/// branch the body is.
struct Region {
    rid: usize,
    owner: usize,
}

/// A `switch` collected up to its `endsw`.
struct Buffered {
    word: String,
    /// Statements that precede the switch loop (glob check of the word).
    pre: Vec<String>,
    pieces: Vec<String>,
    nest: usize,
    bid: usize,
}

/// Which extra translation features a pass has enabled.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Modes {
    goto: bool,
    status: bool,
    /// Give label-bearing bodies their own dispatcher (needs `PassInfo`).
    regions: bool,
}

/// Facts one pass learns and a later pass consumes.
#[derive(Clone, Default)]
struct PassInfo {
    /// Region of the first occurrence of each label.
    label_region: HashMap<String, usize>,
    /// Regions from the top level down to each label's body, as
    /// `(region, bid of the owning `if`)`; the top level is first.
    label_path: HashMap<String, Vec<(usize, Option<usize>)>>,
    /// Regions that get a dispatcher.
    bearing: HashSet<usize>,
    /// For an `if` (by bid): the branch bodies a `goto` from outside the
    /// `if` can enter.
    entry: HashMap<usize, HashSet<usize>>,
    /// `Some(bids)` when the input ends inside these blocks and the pass
    /// must reproduce tcsh's end-of-file behaviour.
    eof_open: Option<HashSet<usize>>,
}

impl PassInfo {
    /// Decide which bodies get a dispatcher and which `if` branches can be
    /// entered from outside. A label's own body always gets one; when the
    /// body is a chain of `if` branches up from the enclosing body, each
    /// branch is enterable and its parent body needs an entry arm too.
    /// Returns false when no body holds a label.
    fn plan_regions(&mut self) -> bool {
        for path in self.label_path.values() {
            if path.iter().any(|&(r, _)| r == NO_REGION) {
                continue;
            }
            let Some(&(last, _)) = path.last() else {
                continue;
            };
            if last == TOP_REGION {
                continue;
            }
            self.bearing.insert(last);
            for i in (1..path.len()).rev() {
                let (rid, Some(bid)) = path[i] else {
                    break;
                };
                self.entry.entry(bid).or_default().insert(rid);
                self.bearing.insert(rid);
                self.bearing.insert(path[i - 1].0);
            }
        }
        !self.bearing.is_empty()
    }
}

/// A `case`/`default` label found by the switch pre-scan.
enum Label {
    Pat(String),
    Default,
    Bad,
}

/// Streaming translator: feed logical lines in order, then `finish`.
/// Tracks open `if`/`foreach`/`while`/`switch` blocks so `end`, `endif`,
/// `endsw`, `else`, `breaksw`, `case`, `default` map to the right zsh
/// closer.
pub struct Translator {
    stack: Vec<Frame>,
    regions: Vec<Region>,
    buf: Option<Buffered>,
    modes: Modes,
    info: PassInfo,
    is_retry: bool,
    base_indent: usize,
    raw: Vec<String>,
    start: Option<usize>,
    end: usize,
    next_id: usize,
    next_bid: usize,
    next_rid: usize,
    seen_goto: bool,
    seen_status: bool,
    last_closer: bool,
    /// `cmd |` waiting for the `while`/`foreach` it feeds (`cmd | while (…)`)
    pipe_prefix: Option<String>,
}

impl Translator {
    pub fn new() -> Self {
        Self::with_modes(
            Modes {
                goto: false,
                status: false,
                regions: false,
            },
            PassInfo::default(),
        )
    }

    fn with_modes(modes: Modes, info: PassInfo) -> Self {
        Self {
            stack: Vec::new(),
            regions: Vec::new(),
            buf: None,
            modes,
            info,
            is_retry: false,
            base_indent: if modes.goto { 2 } else { 0 },
            raw: Vec::new(),
            start: None,
            end: 0,
            next_id: 0,
            next_bid: 0,
            next_rid: TOP_REGION + 1,
            seen_goto: false,
            seen_status: false,
            last_closer: false,
            pipe_prefix: None,
        }
    }

    /// Translate one logical line (comments stripped, continuations joined,
    /// `;` NOT yet split — a one-line `if (e) cmd1; cmd2` is one line).
    pub fn feed(&mut self, line: &str, out: &mut String) -> Result<(), String> {
        if self.start.is_none() {
            self.start = Some(out.len());
        }
        self.raw.push(line.to_string());
        if let Some(body) = line.strip_prefix(super::lex::HEREDOC_RAW) {
            return self.heredoc_line(body, line, out);
        }
        if line.contains("status") {
            self.seen_status = true;
        }
        if self.buf.is_none() {
            let parts = super::lex::split_unquoted(line, '|');
            if let [left, right] = &parts[..] {
                let right = right.trim();
                if starts_with_word(right, "while") || starts_with_word(right, "foreach") {
                    let text = cmds::translate_line(left.trim())?;
                    self.pipe_prefix = Some(format!("{text} | "));
                    self.piece(right, out)?;
                    self.end = out.len();
                    return Ok(());
                }
            }
        }
        let stmts = split_stmts(line);
        if self.buf.is_none() && stmts.iter().all(|s| !is_control_head(s.trim())) {
            // No keyword anywhere: csh list semantics (`a; b &` backgrounds the
            // whole list, `repeat`-free redirects) belong to `cmds`, which
            // needs the line unsplit.
            let text = cmds::translate_line(line.trim())?;
            self.emit(out, &text);
        } else {
            for stmt in stmts {
                let stmt = stmt.trim();
                if !stmt.is_empty() {
                    self.piece(stmt, out)?;
                }
            }
        }
        self.end = out.len();
        Ok(())
    }

    /// One here-document body line or terminator: literal text, so it goes out
    /// at column 0 (a terminator must start its line) and is never parsed. A
    /// switch being collected holds it with its other pieces.
    fn heredoc_line(&mut self, body: &str, raw: &str, out: &mut String) -> Result<(), String> {
        if let Some(b) = self.buf.as_mut() {
            b.pieces.push(raw.to_string());
        } else {
            out.push_str(body);
            out.push('\n');
        }
        self.end = out.len();
        Ok(())
    }

    /// Called when a prompt reader asks whether the input is complete: an
    /// unterminated block is an `Err` with tcsh's text (`then:
    /// then/endif not found.`, `foreach: end not found.`, …), always ending
    /// in `not found.`, so the caller keeps reading lines. When a `goto`,
    /// `onintr label` or a `status` mention was seen, the first-pass output
    /// is replaced by a pass with the matching feature enabled.
    pub fn finish(&mut self, out: &mut String) -> Result<(), String> {
        let intact = self.output_intact(out);
        self.finish_pass(out)?;
        let want = self.wanted_modes();
        if self.is_retry || want == self.modes || !intact {
            return Ok(());
        }
        self.retranslate(out, None)
    }

    /// Called when the input really ended (script file, `-c` string). A
    /// block still open is translated the way tcsh runs it: the reached
    /// body executes and an error is raised only where tcsh has to skip
    /// forward over the missing `endif`/`end`/`endsw` (see the module
    /// docs). Without an open block this is `finish`.
    pub fn finish_eof(&mut self, out: &mut String) -> Result<(), String> {
        let mut open: HashSet<usize> = self.stack.iter().filter_map(Frame::bid).collect();
        if let Some(b) = &self.buf {
            open.insert(b.bid);
        }
        if open.is_empty() {
            return self.finish(out);
        }
        if self.is_retry || !self.output_intact(out) {
            return self.finish_pass(out);
        }
        self.retranslate(out, Some(open))
    }

    /// `out` still ends exactly where the last `feed` left it, so the text
    /// this translator produced can be replaced.
    fn output_intact(&self, out: &String) -> bool {
        out.len() == self.end && self.start.is_some_and(|s| s <= out.len())
    }

    fn wanted_modes(&self) -> Modes {
        Modes {
            goto: self.seen_goto,
            status: self.seen_status,
            regions: false,
        }
    }

    /// Replace this translator's output by a translation of the retained
    /// lines with the features the first pass found a need for. A goto
    /// pass that finds labels inside blocks is followed by a pass that
    /// gives those blocks their dispatchers.
    fn retranslate(
        &mut self,
        out: &mut String,
        eof_open: Option<HashSet<usize>>,
    ) -> Result<(), String> {
        let raw = std::mem::take(&mut self.raw);
        let want = self.wanted_modes();
        let info = PassInfo {
            eof_open,
            ..PassInfo::default()
        };
        let (mut text, mut info) = Self::run_pass(&raw, want, info)?;
        if want.goto {
            if info.plan_regions() {
                let with_regions = Modes {
                    regions: true,
                    ..want
                };
                text = Self::run_pass(&raw, with_regions, info)?.0;
            }
        }
        out.truncate(self.start.unwrap_or(0));
        out.push_str(&text);
        Ok(())
    }

    /// Translate `raw` once with `modes`; returns the text and what the
    /// pass learned.
    fn run_pass(
        raw: &[String],
        modes: Modes,
        info: PassInfo,
    ) -> Result<(String, PassInfo), String> {
        let mut sub = Self::with_modes(modes, info);
        sub.is_retry = true;
        let mut text = String::new();
        if modes.goto {
            text.push_str("__csh_pc='#start'\nwhile :; do\n  case $__csh_pc in\n  ('#start')\n");
        }
        for line in raw {
            sub.feed(line, &mut text)?;
        }
        sub.finish_pass(&mut text)?;
        Ok((text, sub.info))
    }

    /// End-of-input checks and trailers for this pass.
    fn finish_pass(&mut self, out: &mut String) -> Result<(), String> {
        if self.info.eof_open.is_some() {
            self.close_open_blocks(out)?;
        } else {
            if self.buf.is_some() {
                return Err("switch: endsw not found.".to_string());
            }
            if let Some(msg) = self.open_block_error() {
                return Err(msg.to_string());
            }
        }
        // tcsh leaves status 0 after a block closer; zsh keeps the last
        // body status, which would become the script's exit status.
        if self.last_closer {
            self.line(out, 0, ":");
            self.last_closer = false;
        }
        if self.modes.goto {
            out.push_str("    ;;\n");
            out.push_str("  (*) print -ru2 -- \"${__csh_pc}: label not found.\"; exit 1 ;;\n");
            out.push_str("  esac\n  break\ndone\n");
        }
        Ok(())
    }

    /// tcsh message for the innermost unterminated block, if any.
    fn open_block_error(&self) -> Option<&'static str> {
        self.stack.iter().rev().find_map(|f| match f {
            Frame::If { has_else: true, .. } => Some("else: endif not found."),
            Frame::If { .. } => Some("then: then/endif not found."),
            Frame::Loop { kw: "while", .. } => Some("while: end not found."),
            Frame::Loop { .. } => Some("foreach: end not found."),
            Frame::Switch { .. } => Some("switch: endsw not found."),
            Frame::Wrap => None,
        })
    }

    /// True when the block `bid` is still open at the end of the input
    /// (only known in an end-of-file pass).
    fn open_at_eof(&self, bid: usize) -> bool {
        self.info.eof_open.as_ref().is_some_and(|s| s.contains(&bid))
    }

    // ---- output helpers -------------------------------------------------

    /// Append `text` as one line at nesting `level`.
    fn line(&self, out: &mut String, level: usize, text: &str) {
        out.push_str(&"  ".repeat(self.base_indent + level));
        out.push_str(text);
        out.push('\n');
    }

    /// Indent level of statements inside the innermost frame: one per open
    /// block, two per `switch` (its loop and the label guard), two per
    /// active goto dispatcher (its `while` and `case`).
    fn depth(&self) -> usize {
        let frames: usize = self
            .stack
            .iter()
            .map(|f| if matches!(f, Frame::Switch { .. }) { 2 } else { 1 })
            .sum();
        frames + 2 * self.regions.len()
    }

    fn body_level(&self) -> usize {
        self.depth()
    }

    /// Emit a statement in the current body. Inside a `switch` before its
    /// first label the statement is dead (tcsh skips to the first `case`),
    /// so it goes under `if false`.
    fn emit(&mut self, out: &mut String, text: &str) {
        let level = self.depth().saturating_sub(1);
        let mut dead_open = false;
        if let Some(Frame::Switch {
            body_open,
            body_empty,
            ..
        }) = self.stack.last_mut()
        {
            if !*body_open {
                *body_open = true;
                dead_open = true;
            }
            *body_empty = false;
        }
        if dead_open {
            self.line(out, level, "if false; then");
        }
        let level = self.body_level();
        match self.pipe_prefix.take() {
            Some(prefix) => self.line(out, level, &format!("{prefix}{text}")),
            None => self.line(out, level, text),
        }
        self.last_closer = false;
    }

    /// Emit a runtime error stub.
    fn emit_stub(&mut self, out: &mut String, msg: &str) {
        self.emit(out, &stub(msg));
    }

    /// Work done after a block closer was emitted: close `Wrap` guards,
    /// reset status under `$status` mode, remember for the exit status.
    fn after_close(&mut self, out: &mut String) {
        while matches!(self.stack.last(), Some(Frame::Wrap)) {
            self.stack.pop();
            let level = self.depth();
            self.line(out, level, "fi");
        }
        if self.modes.status {
            let level = self.body_level();
            self.line(out, level, ":");
            self.last_closer = false;
        } else {
            self.last_closer = true;
        }
    }

    // ---- regions ---------------------------------------------------------

    fn alloc_bid(&mut self) -> usize {
        self.next_bid += 1;
        self.next_bid - 1
    }

    fn alloc_rid(&mut self) -> usize {
        self.next_rid += 1;
        self.next_rid - 1
    }

    /// Region the next statement belongs to.
    fn cur_rid(&self) -> usize {
        self.stack
            .iter()
            .rev()
            .find_map(Frame::rid)
            .unwrap_or(TOP_REGION)
    }

    /// Start a new branch body in the innermost frame (`rid` fresh) and
    /// give it a dispatcher when a label lives in it.
    fn enter_branch(&mut self, out: &mut String, rid: usize) {
        if let Some(f) = self.stack.last_mut() {
            f.set_rid(rid);
        }
        self.open_region(out);
    }

    /// Open the dispatcher of the current branch body when it holds a
    /// label (second pass only). Statements of the body then sit one
    /// `while`/`case` deeper.
    fn open_region(&mut self, out: &mut String) {
        let rid = self.cur_rid();
        if !self.modes.regions || !self.info.bearing.contains(&rid) {
            return;
        }
        let level = self.depth();
        let var = pc_var(rid);
        // A body entered by a `goto` from outside already holds its label.
        match self.entry_flag(rid) {
            Some(flag) => {
                self.line(out, level, &format!("[[ -n ${flag} ]] || {var}=''"));
                self.line(out, level, &format!("{flag}=''"));
            }
            None => self.line(out, level, &format!("{var}=''")),
        }
        self.line(out, level, "while :; do");
        self.line(out, level + 1, &format!("case \"${var}\" in"));
        self.line(out, level + 1, "('')");
        self.regions.push(Region {
            rid,
            owner: self.stack.len().saturating_sub(1),
        });
    }

    /// Close the dispatcher of the innermost frame's current branch body,
    /// if it has one.
    fn close_region(&mut self, out: &mut String) {
        let owner = self.stack.len().saturating_sub(1);
        if self.regions.last().map(|r| r.owner) != Some(owner) || self.stack.is_empty() {
            return;
        }
        self.regions.pop();
        let level = self.depth();
        self.line(out, level + 2, ";;");
        self.line(out, level + 1, "esac");
        self.line(out, level + 1, "break");
        self.line(out, level, "done");
    }

    /// Variable that forces the branch `rid` of its `if` (set by a `goto`
    /// from outside), when that branch can be entered that way.
    fn entry_flag(&self, rid: usize) -> Option<String> {
        match self.stack.last() {
            Some(Frame::If { bid, .. })
                if self.info.entry.get(bid).is_some_and(|s| s.contains(&rid)) =>
            {
                Some(format!("__csh_in{bid}"))
            }
            _ => None,
        }
    }

    /// Condition of branch `rid` of the `if` numbered `bid`. A `goto` into
    /// the `if` sets `__csh_in<bid>` to the branch to run: that branch is
    /// taken without evaluating anything and every other condition fails.
    fn branch_cond(&self, bid: usize, rid: usize, cond: &str) -> String {
        let Some(entry) = self.info.entry.get(&bid) else {
            return cond.to_string();
        };
        let flag = format!("__csh_in{bid}");
        let plain = format!("[[ -z ${flag} ]] && {{ {cond}; }}");
        if entry.contains(&rid) {
            format!("{{ [[ ${flag} == {rid} ]] || {{ {plain}; }}; }}")
        } else {
            format!("{{ {plain}; }}")
        }
    }

    /// `;&` plus the case arm `(name)` at the current body level of a
    /// dispatcher: statements that follow are reached by a `goto name`.
    fn emit_arm(&mut self, out: &mut String, name: &str) {
        let level = self.base_indent + self.depth();
        out.push_str(&"  ".repeat(level));
        out.push_str(";&\n");
        out.push_str(&"  ".repeat(level - 1));
        out.push_str(&format!("({})\n", sh_quote(name)));
        self.last_closer = false;
    }

    /// True when the current body runs inside a dispatcher.
    fn in_dispatcher(&self) -> bool {
        let rid = self.cur_rid();
        rid == TOP_REGION
            || (self.modes.regions && self.regions.last().is_some_and(|r| r.rid == rid))
    }

    /// Dispatcher loops (and open frames' own loops) that sit between a
    /// statement and the dispatcher of the frame at stack index `owner`
    /// (`None`: the top-level dispatcher).
    fn loops_inside(&self, owner: Option<usize>) -> usize {
        let from = owner.map_or(0, |o| o + 1);
        let frames = self.stack[from..]
            .iter()
            .filter(|f| matches!(f, Frame::Loop { .. } | Frame::Switch { .. }))
            .count();
        let regions = self.regions.iter().filter(|r| r.owner >= from).count();
        frames + regions
    }

    /// Number of active dispatchers owned by frame `k`.
    fn regions_of(&self, k: usize) -> usize {
        self.regions.iter().filter(|r| r.owner == k).count()
    }

    // ---- end of input inside open blocks ----------------------------------

    /// Close every block still open at the end of the input the way tcsh
    /// ends the script (module docs, "End of input inside an open block").
    fn close_open_blocks(&mut self, out: &mut String) -> Result<(), String> {
        if let Some(b) = self.buf.take() {
            self.emit_switch(b, out)?;
        }
        while let Some(frame) = self.stack.last() {
            match frame {
                Frame::Wrap => {
                    self.stack.pop();
                    let level = self.depth();
                    self.line(out, level, "fi");
                }
                Frame::If { .. } => self.eof_close_if(out),
                Frame::Loop { .. } => self.eof_close_loop(out),
                Frame::Switch { .. } => self.close_switch(out, true),
            }
        }
        Ok(())
    }

    /// An `if` that ends with the input: a branch that was taken just
    /// ends; nothing taken is tcsh skipping to an `endif` that is not there.
    fn eof_close_if(&mut self, out: &mut String) {
        self.close_region(out);
        let Some(Frame::If {
            has_else, extra_fi, ..
        }) = self.stack.last()
        else {
            return;
        };
        let (has_else, extra) = (*has_else, *extra_fi);
        let level = self.depth() - 1;
        if !has_else {
            self.line(out, level, "else");
            self.line(out, level + 1, &stub("then: then/endif not found."));
        }
        for _ in 0..=extra {
            self.line(out, level, "fi");
        }
        self.stack.pop();
    }

    /// A loop that ends with the input: falling off the end of the first
    /// iteration ends the script; `continue` reaches the next iteration, and
    /// running out of iterations is tcsh skipping to a missing `end`.
    fn eof_close_loop(&mut self, out: &mut String) {
        self.close_region(out);
        let Some(Frame::Loop { kw, bid, .. }) = self.stack.last() else {
            return;
        };
        let (kw, bid) = (*kw, *bid);
        let level = self.depth() - 1;
        self.line(out, level + 1, "exit $?");
        self.line(out, level, "done");
        self.stack.pop();
        if kw == "while" {
            self.emit_stub(out, "while: end not found.");
        } else {
            let ran = format!("__csh_ran{bid}");
            let level = self.body_level();
            self.line(out, level, &format!("if (( {ran} )); then"));
            self.line(out, level + 1, &stub("continue: end not found."));
            self.line(out, level, "else");
            self.line(out, level + 1, &stub("foreach: end not found."));
            self.line(out, level, "fi");
        }
    }

    // ---- statement dispatch --------------------------------------------

    fn piece(&mut self, stmt: &str, out: &mut String) -> Result<(), String> {
        if let Some(body) = stmt.strip_prefix(super::lex::HEREDOC_RAW) {
            out.push_str(body);
            out.push('\n');
            return Ok(());
        }
        if self.buf.is_some() {
            return self.buffer_piece(stmt, out);
        }
        let (head, rest) = split_head(stmt);
        match head {
            "if" => self.kw_if(rest, out),
            "else" => self.kw_else(rest, out),
            "endif" => self.kw_endif(rest, out),
            "foreach" => self.kw_foreach(rest, out),
            "while" => self.kw_while(rest, out),
            "end" => self.kw_end(rest, out),
            "switch" => self.kw_switch(rest, out),
            "case" | "default" | "default:" => self.kw_label(out),
            "endsw" => self.kw_endsw(out),
            h if h.len() > 1 && h.ends_with(':') => self.kw_goto_label(h, rest, out),
            _ => {
                let text = self.simple_command(stmt)?;
                self.emit(out, &text);
                Ok(())
            }
        }
    }

    /// One command that does not open a block, as zsh text. Used for plain
    /// statements and for the command of a one-line `if`/`repeat`.
    fn simple_command(&mut self, stmt: &str) -> Result<String, String> {
        let (head, rest) = split_head(stmt);
        match head {
            "break" | "continue" => {
                if !rest.is_empty() {
                    return Ok(stub(&format!("{head}: Too many arguments.")));
                }
                Ok(self.loop_jump(head))
            }
            "breaksw" => {
                if !rest.is_empty() {
                    return Ok(stub("breaksw: Too many arguments."));
                }
                Ok(self.switch_break())
            }
            "goto" => self.goto_text(rest),
            "onintr" => self.onintr_text(rest),
            "repeat" => self.repeat_text(rest),
            "if" => match if_parts(rest) {
                Err(m) => Ok(stub(m)),
                Ok((inner, after)) => {
                    let after = after.trim();
                    if after.is_empty() {
                        Ok(stub("if: Empty if."))
                    } else if after == "then" || starts_with_word(after, "then") {
                        Ok(stub("if: Improper then."))
                    } else {
                        self.if_oneline_text(&inner, after)
                    }
                }
            },
            _ => cmds::translate_line(stmt),
        }
    }

    /// `if c; then body; fi` plus the `&&`/`||`/`&` tail that follows the
    /// command on the csh line.
    fn if_oneline_text(&mut self, cond: &str, cmd: &str) -> Result<String, String> {
        let c = expr::translate_condition(cond)?;
        let (head, tail) = cut_list(cmd);
        let head = head.trim();
        if head.is_empty() {
            return Ok(stub("if: Empty if."));
        }
        let body = self.simple_command(head)?;
        let text = compose_if(&c, &body);
        self.attach_tail(text, tail)
    }

    fn repeat_text(&mut self, rest: &str) -> Result<String, String> {
        let (count, cmd) = split_head(rest);
        if count.is_empty() || cmd.is_empty() {
            return Ok(stub("repeat: Too few arguments."));
        }
        let literal = !count.contains('$') && !count.contains('`');
        if literal && !count.chars().all(|c| c.is_ascii_digit()) {
            return Ok(stub("repeat: Badly formed number."));
        }
        let (head, tail) = cut_list(cmd);
        let body = self.simple_command(head.trim())?;
        let text = format!("repeat {}; do {body}; done", words::translate_word(count));
        self.attach_tail(text, tail)
    }

    /// Join the operator that ended a one-line command onto its statement.
    fn attach_tail(&mut self, text: String, tail: Option<(&str, &str)>) -> Result<String, String> {
        let Some((op, rest)) = tail else {
            return Ok(text);
        };
        let rest = rest.trim();
        if op == "&" {
            let mut s = format!("{text} &");
            if !rest.is_empty() {
                s.push('\n');
                s.push_str(&self.simple_command(rest)?);
            }
            return Ok(s);
        }
        if rest.is_empty() {
            return Ok(text);
        }
        Ok(format!("{text} {op} {}", self.simple_command(rest)?))
    }

    // ---- break / continue / breaksw / goto -----------------------------

    /// `break`/`continue` for the innermost csh loop; `switch` loops and
    /// goto dispatchers in between are skipped with a count argument.
    fn loop_jump(&self, kw: &str) -> String {
        let mut skipped = 0;
        for (k, f) in self.stack.iter().enumerate().rev() {
            let d = self.regions_of(k);
            match f {
                Frame::Loop { bid, .. } => {
                    // The loop's closer never comes: tcsh skips to the end
                    // of the input looking for it.
                    if kw == "break" && self.open_at_eof(*bid) {
                        return stub("break: end not found.");
                    }
                    return jump(kw, skipped + d + 1);
                }
                Frame::Switch { .. } => skipped += 1 + d,
                Frame::If { .. } => skipped += d,
                Frame::Wrap => {}
            }
        }
        stub(&format!("{kw}: Not in while/foreach."))
    }

    /// `breaksw`: leave the innermost switch loop, through any csh loops
    /// and dispatchers opened inside the case body.
    fn switch_break(&self) -> String {
        let mut skipped = 0;
        for (k, f) in self.stack.iter().enumerate().rev() {
            let d = self.regions_of(k);
            match f {
                Frame::Switch { bid, .. } => {
                    if self.open_at_eof(*bid) {
                        return stub("breaksw: endsw not found.");
                    }
                    return jump("break", skipped + d + 1);
                }
                Frame::Loop { .. } => skipped += 1 + d,
                Frame::If { .. } => skipped += d,
                Frame::Wrap => {}
            }
        }
        stub("breaksw: endsw not found.")
    }

    fn goto_text(&mut self, rest: &str) -> Result<String, String> {
        let args = split_words(rest);
        match args.len() {
            0 => return Ok(stub("goto: Too few arguments.")),
            1 => {}
            _ => return Ok(stub("goto: Too many arguments.")),
        }
        self.seen_goto = true;
        if !self.modes.goto {
            return Ok(stub("goto: not available in this pass"));
        }
        let word = &args[0];
        if word.contains('$') || word.contains('`') {
            return Ok(self.computed_goto(&words::translate_word(word)));
        }
        match self.goto_plan(word) {
            Some((assign, depth)) => Ok(format!("{assign}; {}", jump("continue", depth + 1))),
            None => Ok(stub(&format!(
                "goto: {word}: jumping into a block is not supported"
            ))),
        }
    }

    /// `goto $var`: the label is known only at run time. Labels inside
    /// blocks get a `case` arm each with the plan `goto_plan` computes for
    /// this statement; anything else is handed to the top-level dispatcher.
    fn computed_goto(&self, value: &str) -> String {
        let top_depth = self.loops_inside(None);
        let fallback = format!("__csh_pc={value}; {}", jump("continue", top_depth + 1));
        if !self.modes.regions {
            return fallback;
        }
        let mut labels: Vec<&String> = self
            .info
            .label_path
            .iter()
            .filter(|(_, p)| p.last().is_some_and(|&(r, _)| r != TOP_REGION))
            .map(|(l, _)| l)
            .collect();
        labels.sort();
        let mut text = format!("case {value} in");
        for l in labels {
            let arm = match self.goto_plan(l) {
                Some((assign, depth)) => format!("{assign}; {}", jump("continue", depth + 1)),
                None => stub(&format!("goto: {l}: jumping into a block is not supported")),
            };
            text.push_str(&format!(" ({}) {arm};;", sh_quote(l)));
        }
        text.push_str(&format!(" (*) {fallback};; esac"));
        text
    }

    /// `(region, bid of the owning if)` for every body from the top level to
    /// the current statement.
    fn region_path(&self) -> Vec<(usize, Option<usize>)> {
        let steps = self.stack.iter().filter_map(|f| match f {
            Frame::If { rid, bid, .. } => Some((*rid, Some(*bid))),
            Frame::Loop { rid, .. } | Frame::Switch { rid, .. } => Some((*rid, None)),
            Frame::Wrap => None,
        });
        std::iter::once((TOP_REGION, None)).chain(steps).collect()
    }

    /// How a `goto label` reaches its label: the assignments that select
    /// the arm in each dispatcher on the way, and the number of zsh loops
    /// between the statement and the dispatcher to `continue`. `None` when
    /// the label sits in a block this statement cannot enter.
    ///
    /// A label in an enclosing body just sets that body's variable. A label
    /// in an `if` branch that does not enclose the statement is entered
    /// from the nearest enclosing body: it jumps to the entry arm of the
    /// outermost `if`, and each `if` on the way is told which branch to run
    /// (`branch_cond`). Loop and switch bodies cannot be entered.
    fn goto_plan(&self, label: &str) -> Option<(String, usize)> {
        let top = || {
            let assign = format!("__csh_pc={}", sh_quote(label));
            Some((assign, self.loops_inside(None)))
        };
        let Some(path) = self.info.label_path.get(label) else {
            return top();
        };
        let &(last, _) = path.last()?;
        if last == TOP_REGION || !self.modes.regions {
            // The first goto pass is replaced once the regions are known.
            return top();
        }
        let active = |rid: usize| rid == TOP_REGION || self.regions.iter().any(|r| r.rid == rid);
        let j = path.iter().rposition(|&(rid, _)| active(rid))?;
        let owner = self.regions.iter().find(|r| r.rid == path[j].0).map(|r| r.owner);
        let loops = self.loops_inside(owner);
        // Bodies below `j` are entered through their `if`s; a loop or
        // switch body cannot be.
        let mut assigns = Vec::new();
        for i in j + 1..path.len() {
            let (rid, Some(bid)) = path[i] else {
                return None;
            };
            assigns.push(format!("__csh_in{bid}={rid}"));
        }
        for i in j..path.len() {
            let target = match path.get(i + 1) {
                Some(&(_, Some(bid))) => sh_quote(&format!("__blk{bid}")),
                _ => sh_quote(label),
            };
            assigns.push(format!("{}={target}", pc_var(path[i].0)));
        }
        Some((assigns.join("; "), loops))
    }

    /// `onintr`: `-` ignores INT, no argument restores it, a label jumps
    /// to that label from wherever the signal arrives.
    fn onintr_text(&mut self, rest: &str) -> Result<String, String> {
        let args = split_words(rest);
        match args.len() {
            0 => return Ok("trap - INT".to_string()),
            1 => {}
            _ => return Ok(stub("onintr: Too many arguments.")),
        }
        let word = &args[0];
        if word == "-" {
            return Ok("trap '' INT".to_string());
        }
        self.seen_goto = true;
        if !self.modes.goto {
            return Ok(":".to_string());
        }
        let value = if word.contains('$') || word.contains('`') {
            words::translate_word(word)
        } else {
            match self.info.label_region.get(word.as_str()) {
                None | Some(&TOP_REGION) => {}
                Some(_) => {
                    return Ok(stub(&format!(
                        "onintr: {word}: jumping into a block is not supported"
                    )));
                }
            }
            sh_quote(word)
        };
        // `continue 1000` leaves every loop up to the outermost one, which
        // is the goto dispatcher.
        let handler = format!("__csh_pc={value}; continue 1000");
        Ok(format!("trap {} INT", sh_quote(&handler)))
    }

    /// `label:` on a line of its own.
    fn kw_goto_label(&mut self, head: &str, rest: &str, out: &mut String) -> Result<(), String> {
        if !rest.is_empty() {
            self.emit_stub(out, &format!("{head}: Too many arguments."));
            return Ok(());
        }
        let name = &head[..head.len() - 1];
        if !self.modes.goto {
            // No dispatcher, but the label line still resets `$status`.
            if self.modes.status {
                self.emit(out, ":");
            }
            return Ok(());
        }
        let rid = self.cur_rid();
        let path = self.region_path();
        self.info.label_region.entry(name.to_string()).or_insert(rid);
        self.info.label_path.entry(name.to_string()).or_insert(path);
        if self.in_dispatcher() {
            self.emit_arm(out, name);
            // A label line is a command that succeeds: it resets `$status`.
            if self.modes.status {
                let level = self.depth();
                self.line(out, level, ":");
            }
        }
        Ok(())
    }

    // ---- if / else / endif ---------------------------------------------

    fn kw_if(&mut self, rest: &str, out: &mut String) -> Result<(), String> {
        if rest.is_empty() {
            self.emit_stub(out, "if: Too few arguments.");
            return Ok(());
        }
        let (inner, after) = match if_parts(rest) {
            Ok(g) => g,
            Err(m) => {
                self.emit_stub(out, m);
                return Ok(());
            }
        };
        let after = after.trim();
        if after.is_empty() {
            self.emit_stub(out, "if: Empty if.");
        } else if after == "then" {
            let c = expr::translate_condition(&inner)?;
            self.open_if(out, &c);
        } else if starts_with_word(after, "then") {
            self.emit_stub(out, "if: Improper then.");
        } else if let Some(conds) = if_chain(&inner, after) {
            // `if (c1) if (c2) then`: when c1 is false tcsh skips only the
            // one-line `if`, so the body lines run unconditionally.
            let mut text = String::new();
            for c in &conds[..conds.len() - 1] {
                text.push_str(&format!("! {{ {}; }} || ", expr::translate_condition(c)?));
            }
            text.push_str(&expr::translate_condition(&conds[conds.len() - 1])?);
            self.open_if(out, &text);
        } else if self.opens_block(after) {
            let c = expr::translate_condition(&inner)?;
            self.emit(out, &format!("if {c}; then"));
            self.stack.push(Frame::Wrap);
            self.piece(after, out)?;
        } else {
            let text = self.if_oneline_text(&inner, after)?;
            self.emit(out, &text);
        }
        Ok(())
    }

    /// Emit the opening `if … then` line and push its frame.
    fn open_if(&mut self, out: &mut String, cond: &str) {
        let (bid, rid) = (self.alloc_bid(), self.alloc_rid());
        if self.info.entry.contains_key(&bid) && self.in_dispatcher() {
            self.emit_arm(out, &format!("__blk{bid}"));
        }
        let cond = self.branch_cond(bid, rid, cond);
        self.emit(out, &format!("if {cond}; then"));
        self.stack.push(Frame::If {
            has_else: false,
            extra_fi: 0,
            bid,
            rid,
        });
        self.open_region(out);
    }

    /// True when `stmt` (the command of a one-line `if`) starts a block
    /// that a later `end`/`endsw`/`endif` closes.
    fn opens_block(&self, stmt: &str) -> bool {
        let (head, rest) = split_head(stmt);
        match head {
            "foreach" | "while" | "switch" => true,
            "if" => match if_parts(rest) {
                Ok((_, after)) => {
                    let after = after.trim();
                    after == "then" || (!after.is_empty() && self.opens_block(after))
                }
                Err(_) => false,
            },
            _ => false,
        }
    }

    fn kw_else(&mut self, rest: &str, out: &mut String) -> Result<(), String> {
        if !matches!(self.stack.last(), Some(Frame::If { .. })) {
            self.emit_stub(out, "else: endif not found.");
            return Ok(());
        }
        self.close_region(out);
        let (has_else, bid, level) = match self.stack.last() {
            Some(Frame::If { has_else, bid, .. }) => (*has_else, *bid, self.depth() - 1),
            _ => return Ok(()),
        };
        // A taken `then` branch that reaches `else` makes tcsh skip to an
        // `endif`; when the input ends first that is an error.
        if !has_else && self.open_at_eof(bid) {
            self.line(out, level + 1, &stub("else: endif not found."));
        }
        // `else if (c) then` continues the chain.
        let (head, r2) = split_head(rest);
        if head == "if" {
            if let Ok((inner, after)) = if_parts(r2) {
                if after.trim() == "then" {
                    if has_else {
                        return self.dead_else(out);
                    }
                    let c = expr::translate_condition(&inner)?;
                    let c = if self.modes.status { format!(":; {c}") } else { c };
                    let rid = self.alloc_rid();
                    let c = match self.stack.last().and_then(Frame::bid) {
                        Some(bid) => self.branch_cond(bid, rid, &c),
                        None => c,
                    };
                    self.line(out, level, &format!("elif {c}; then"));
                    self.last_closer = false;
                    self.enter_branch(out, rid);
                    return Ok(());
                }
            }
        }
        if has_else {
            self.dead_else(out)?;
        } else {
            self.line(out, level, "else");
            if let Some(Frame::If { has_else, .. }) = self.stack.last_mut() {
                *has_else = true;
            }
            let rid = self.alloc_rid();
            self.enter_branch(out, rid);
            if self.modes.status {
                let l = self.body_level();
                self.line(out, l, ":");
            }
            self.last_closer = false;
        }
        if !rest.is_empty() {
            self.piece(rest, out)?;
        }
        Ok(())
    }

    /// A second `else` in one `if`: tcsh skips to `endif`, so the branch
    /// never runs; zsh forbids a second `else`, so it becomes `if false`.
    fn dead_else(&mut self, out: &mut String) -> Result<(), String> {
        let level = self.depth();
        self.line(out, level, "if false; then");
        if let Some(f) = self.stack.last_mut() {
            f.set_rid(NO_REGION);
            if let Frame::If { extra_fi, .. } = f {
                *extra_fi += 1;
            }
        }
        Ok(())
    }

    fn kw_endif(&mut self, rest: &str, out: &mut String) -> Result<(), String> {
        if !rest.is_empty() {
            self.emit_stub(out, "endif: Too many arguments.");
            return Ok(());
        }
        if matches!(self.stack.last(), Some(Frame::If { .. })) {
            self.close_region(out);
            let extra = match self.stack.last() {
                Some(Frame::If { extra_fi, .. }) => *extra_fi,
                _ => 0,
            };
            let level = self.depth() - 1;
            for _ in 0..=extra {
                self.line(out, level, "fi");
            }
            self.stack.pop();
            self.after_close(out);
        }
        // A stray `endif` is a no-op in tcsh.
        Ok(())
    }

    // ---- foreach / while / end -----------------------------------------

    fn kw_foreach(&mut self, rest: &str, out: &mut String) -> Result<(), String> {
        let args = split_words(rest);
        if args.len() < 2 {
            self.emit_stub(out, "foreach: Too few arguments.");
            return Ok(());
        }
        let var = &args[0];
        if !var.chars().next().is_some_and(|c| c.is_alphabetic()) {
            self.emit_stub(out, "foreach: Variable name must begin with a letter.");
            return Ok(());
        }
        if !var.chars().all(|c| c.is_alphanumeric() || c == '_') {
            self.emit_stub(out, "foreach: Invalid variable");
            return Ok(());
        }
        let list = &args[1];
        if !list.starts_with('(') || args.len() > 2 {
            self.emit_stub(out, "foreach: Words not parenthesized.");
            return Ok(());
        }
        if !list.ends_with(')') {
            self.emit_stub(out, "Too many ('s.");
            return Ok(());
        }
        let inner = &list[1..list.len() - 1];
        let items: Vec<String> = split_words(inner)
            .iter()
            .map(|w| words::translate_word_checked(w))
            .collect();
        let words_text = if items.is_empty() {
            String::new()
        } else {
            format!(" {}", items.join(" "))
        };
        // tcsh expands the list first and ends the script with
        // `foreach: No match.` when every pattern of it fails.
        let globs: Vec<String> = split_words(inner)
            .iter()
            .filter(|w| super::cmds::is_glob_word(w))
            .map(|w| words::translate_word(w))
            .collect();
        if !globs.is_empty() {
            let probe = format!(
                "{{ [[ ! -o cshnullglob ]] || () {{ setopt localoptions nullglob; local -a _g; _g=({}); (( $#_g )); }} \
|| {{ print -u2 -r -- 'foreach: No match.'; [[ -o interactive ]] || exit 1; false; }}; }}",
                globs.join(" ")
            );
            self.emit(out, &probe);
        }
        let (bid, rid) = (self.alloc_bid(), self.alloc_rid());
        // An unterminated foreach tells "no iteration" from "ran out of
        // iterations" through this flag (module docs).
        let ran_flag = self.open_at_eof(bid).then(|| format!("__csh_ran{bid}"));
        if let Some(flag) = &ran_flag {
            self.emit(out, &format!("{flag}=0"));
        }
        self.emit(out, &format!("for {var} in{words_text}; do"));
        self.stack.push(Frame::Loop {
            kw: "foreach",
            bid,
            rid,
        });
        self.open_region(out);
        if let Some(flag) = ran_flag {
            let l = self.body_level();
            self.line(out, l, &format!("{flag}=1"));
        }
        // A csh variable is a list: the loop variable is a one-element array
        // so that `$f[1]` and the `:h`/`:t` modifiers (which index it) work.
        let l = self.body_level();
        self.line(out, l, &format!("{var}=(\"${var}\")"));
        if self.modes.status {
            let l = self.body_level();
            self.line(out, l, ":");
        }
        Ok(())
    }

    fn kw_while(&mut self, rest: &str, out: &mut String) -> Result<(), String> {
        if rest.is_empty() {
            self.emit_stub(out, "while: Too few arguments.");
            return Ok(());
        }
        let group = if rest.starts_with('(') {
            paren_group(rest)
        } else {
            Err("while: Expression Syntax.")
        };
        match group {
            Err(m) => self.emit_stub(out, m),
            Ok((_, after)) if !after.trim().is_empty() => {
                self.emit_stub(out, "while: Expression Syntax.");
            }
            Ok((inner, _)) => {
                let c = expr::for_command(&expr::translate_condition(inner)?, "while");
                self.emit(out, &format!("while {c}; do"));
                let (bid, rid) = (self.alloc_bid(), self.alloc_rid());
                self.stack.push(Frame::Loop {
                    kw: "while",
                    bid,
                    rid,
                });
                self.open_region(out);
            }
        }
        Ok(())
    }

    fn kw_end(&mut self, rest: &str, out: &mut String) -> Result<(), String> {
        if !rest.is_empty() {
            self.emit_stub(out, "end: Too many arguments.");
            return Ok(());
        }
        if matches!(self.stack.last(), Some(Frame::Loop { .. })) {
            self.close_region(out);
            let level = self.depth() - 1;
            self.line(out, level, "done");
            self.stack.pop();
            self.after_close(out);
        } else {
            self.emit_stub(out, "end: Not in while/foreach.");
        }
        Ok(())
    }

    // ---- switch ---------------------------------------------------------

    fn kw_switch(&mut self, rest: &str, out: &mut String) -> Result<(), String> {
        if rest.is_empty() {
            self.emit_stub(out, "switch: Too few arguments.");
            return Ok(());
        }
        let group = if rest.starts_with('(') {
            paren_group(rest)
        } else {
            Err("Syntax Error.")
        };
        let inner = match group {
            Ok((inner, after)) if after.trim().is_empty() => inner,
            Ok(_) => {
                self.emit_stub(out, "Syntax Error.");
                return Ok(());
            }
            Err(m) => {
                self.emit_stub(out, m);
                return Ok(());
            }
        };
        let args = split_words(inner);
        let mut pre = Vec::new();
        let word = match args.len() {
            0 => "''".to_string(),
            1 => self.switch_word(&args[0], &mut pre),
            _ => {
                self.emit_stub(out, "Syntax Error.");
                return Ok(());
            }
        };
        let bid = self.alloc_bid();
        self.buf = Some(Buffered {
            word,
            pre,
            pieces: Vec::new(),
            nest: 0,
            bid,
        });
        Ok(())
    }

    /// The zsh word a switch tests. tcsh globs it like any word: an
    /// unquoted `{a,b}` list or several glob matches are `Ambiguous.`, no
    /// match is `No match.`. The checks go to `pre`.
    fn switch_word(&self, w: &str, pre: &mut Vec<String>) -> String {
        let translated = words::translate_word(w);
        if let Some(name) = bare_variable(w) {
            // csh variables are lists: undefined is an error, more than one
            // word is not a single switch word.
            let z = words::zsh_var_name(name);
            let undefined = stub(&format!("{name}: Undefined variable."));
            pre.push(format!("(( ${{+{z}}} )) || {{ {undefined}; }}"));
            // `"${(@)v}"` is one word per element for a list and exactly
            // one for the scalar a `foreach` variable is.
            pre.push(format!("__csh_g=(\"${{(@){z}}}\")"));
            pre.push(format!("(( $#__csh_g > 1 )) && {{ {}; }}", stub("Syntax Error.")));
            return translated;
        }
        if has_brace_list(w) {
            pre.push(stub(&format!("{w}: Ambiguous.")));
            return translated;
        }
        if !has_unquoted_glob(w) {
            return translated;
        }
        pre.push(format!("__csh_g=({translated}(N))"));
        pre.push(format!(
            "(( $#__csh_g > 1 )) && {{ {}; }}",
            stub(&format!("{w}: Ambiguous."))
        ));
        pre.push(format!(
            "(( $#__csh_g )) || {{ {}; }}",
            stub(&format!("{w}: No match."))
        ));
        "${__csh_g[1]}".to_string()
    }

    /// While a switch is buffered: collect statements until the matching
    /// `endsw`, then emit the whole switch.
    fn buffer_piece(&mut self, stmt: &str, out: &mut String) -> Result<(), String> {
        let kw = leading_kw(stmt);
        let Some(b) = self.buf.as_mut() else {
            return Ok(());
        };
        match kw.as_str() {
            "switch" => b.nest += 1,
            "endsw" if b.nest > 0 => b.nest -= 1,
            "endsw" => {
                let Some(b) = self.buf.take() else {
                    return Ok(());
                };
                self.emit_switch(b, out)?;
                return self.piece(stmt, out);
            }
            _ => {}
        }
        b.pieces.push(stmt.to_string());
        Ok(())
    }

    /// Emit the dispatch for a buffered switch and replay its body.
    fn emit_switch(&mut self, b: Buffered, out: &mut String) -> Result<(), String> {
        let labels = prescan_labels(&b.pieces);
        let id = self.next_id;
        self.next_id += 1;
        let m = format!("__csh_m{id}");
        for text in &b.pre {
            self.emit(out, text);
        }
        let level = self.depth();
        self.emit(out, "for __csh_sw in 1; do");
        self.line(out, level + 1, &format!("{m}=0"));
        self.line(out, level + 1, &format!("case {} in", b.word));
        // tcsh tests labels in source order and `default` matches the moment
        // it is reached, so labels after the first `default` are only
        // reachable by fallthrough.
        for (i, l) in labels.iter().enumerate() {
            match l {
                Label::Pat(p) => {
                    self.line(out, level + 1, &format!("({}) {m}={};;", case_pattern(p), i + 1));
                }
                Label::Default => {
                    self.line(out, level + 1, &format!("(*) {m}={};;", i + 1));
                    break;
                }
                Label::Bad => {}
            }
        }
        self.line(out, level + 1, "esac");
        self.stack.push(Frame::Switch {
            id,
            bid: b.bid,
            rid: NO_REGION,
            labels_seen: 0,
            body_open: false,
            body_empty: true,
        });
        for p in &b.pieces {
            self.piece(p, out)?;
        }
        Ok(())
    }

    /// `case pat:` / `default:` — closes the previous label's guard and
    /// opens this one. Text after the colon is dropped, as tcsh does.
    fn kw_label(&mut self, out: &mut String) -> Result<(), String> {
        if matches!(self.stack.last(), Some(Frame::Switch { .. })) {
            self.close_region(out);
            let level = self.depth().saturating_sub(1);
            if let Some(Frame::Switch {
                id,
                labels_seen,
                body_open,
                body_empty,
                ..
            }) = self.stack.last_mut()
            {
                *labels_seen += 1;
                let (m, k) = (format!("__csh_m{id}"), *labels_seen);
                let (was_open, was_empty) = (*body_open, *body_empty);
                *body_open = true;
                *body_empty = true;
                if was_open {
                    if was_empty {
                        self.line(out, level + 1, ":");
                    }
                    self.line(out, level, "fi");
                }
                self.line(out, level, &format!("if (( {m} && {m} <= {k} )); then"));
                self.last_closer = false;
            }
            let rid = self.alloc_rid();
            self.enter_branch(out, rid);
            return Ok(());
        }
        // A label inside a nested block still occupies an index.
        for f in self.stack.iter_mut().rev() {
            if let Frame::Switch { labels_seen, .. } = f {
                *labels_seen += 1;
                break;
            }
        }
        Ok(())
    }

    fn kw_endsw(&mut self, out: &mut String) -> Result<(), String> {
        if matches!(self.stack.last(), Some(Frame::Switch { .. })) {
            self.close_switch(out, false);
            self.after_close(out);
        }
        Ok(())
    }

    /// Close the innermost switch's guard and loop. `at_eof`: the `endsw`
    /// never came, so a switch that matched no label is tcsh skipping to a
    /// missing `endsw`.
    fn close_switch(&mut self, out: &mut String, at_eof: bool) {
        self.close_region(out);
        let Some(Frame::Switch {
            id,
            body_open,
            body_empty,
            ..
        }) = self.stack.last()
        else {
            return;
        };
        let (id, open, empty) = (*id, *body_open, *body_empty);
        let level = self.depth() - 1;
        if open {
            if empty {
                self.line(out, level + 1, ":");
            }
            self.line(out, level, "fi");
        }
        if at_eof {
            let m = format!("__csh_m{id}");
            self.line(
                out,
                level,
                &format!("(( {m} )) || {{ {}; }}", stub("switch: endsw not found.")),
            );
        }
        self.line(out, level - 1, "done");
        self.stack.pop();
    }
}

impl Default for Translator {
    fn default() -> Self {
        Self::new()
    }
}

// ---- free helpers ---------------------------------------------------------

/// Single-quote `s` for zsh.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// zsh text that prints a csh error to stderr and aborts, as tcsh does for
/// an error raised while running a script.
pub(crate) fn stub(msg: &str) -> String {
    format!("print -ru2 -- {}; exit 1", sh_quote(msg))
}

/// `break`/`continue` leaving `n` loops.
fn jump(kw: &str, n: usize) -> String {
    if n > 1 {
        format!("{kw} {n}")
    } else {
        kw.to_string()
    }
}

/// `word` followed by whitespace at the start of `s`.
fn starts_with_word(s: &str, word: &str) -> bool {
    s.strip_prefix(word)
        .is_some_and(|r| r.starts_with(char::is_whitespace))
}

/// `if c; then body; fi` on one line when `body` allows it.
fn compose_if(cond: &str, body: &str) -> String {
    if body.contains('\n') || body.ends_with('&') || body.ends_with(';') {
        format!("if {cond}; then\n  {body}\nfi")
    } else {
        format!("if {cond}; then {body}; fi")
    }
}

/// First word of a statement and the trimmed remainder. A `(` ends the
/// word, so `if(1)then` and `switch(a)` split like tcsh's lexer does.
fn split_head(stmt: &str) -> (&str, &str) {
    let end = stmt
        .find(|c: char| c.is_whitespace() || c == '(')
        .unwrap_or(stmt.len());
    (&stmt[..end], stmt[end..].trim_start())
}

/// `s` starts with `(`; return the text inside the matching `)` and what
/// follows it.
fn paren_group(s: &str) -> Result<(&str, &str), &'static str> {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut it = s.char_indices();
    while let Some((i, c)) = it.next() {
        if c == '\\' {
            it.next();
            continue;
        }
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '\'' | '"' | '`' => quote = Some(c),
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok((&s[1..i], &s[i + 1..]));
                    }
                }
                _ => {}
            },
        }
    }
    Err("Too many ('s.")
}

/// Positions of characters outside quotes, backslash escapes, parentheses
/// and `{ ... }` expression braces. A `{` only opens a group when followed
/// by whitespace and a `}` only closes one when preceded by whitespace, so
/// `${x}` and `{a,b}` are plain text.
fn top_level(s: &str) -> Vec<(usize, char)> {
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let mut out = Vec::new();
    let mut quote: Option<char> = None;
    let (mut parens, mut braces) = (0i32, 0i32);
    let mut i = 0;
    while i < chars.len() {
        let (pos, c) = chars[i];
        if c == '\\' {
            i += 2;
            continue;
        }
        let next_space = chars.get(i + 1).map_or(true, |&(_, n)| n.is_whitespace());
        let prev_space = i > 0 && chars[i - 1].1.is_whitespace();
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '\'' | '"' | '`' => quote = Some(c),
                '(' => parens += 1,
                ')' => parens -= 1,
                '{' if next_space => braces += 1,
                '}' if braces > 0 && prev_space => braces -= 1,
                _ => {
                    if parens <= 0 && braces == 0 {
                        out.push((pos, c));
                    }
                }
            },
        }
        i += 1;
    }
    out
}

/// Split a logical line into statements on top-level `;`.
fn split_stmts(line: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut from = 0;
    for (pos, c) in top_level(line) {
        if c == ';' {
            parts.push(&line[from..pos]);
            from = pos + 1;
        }
    }
    parts.push(&line[from..]);
    parts
}

/// Cut a one-line command at the first top-level `&&`, `||` or `&`
/// (a `&` that is not part of `>&`, `|&`, `>>&`). The command before the
/// operator is the one pipeline the `if`/`repeat` governs.
fn cut_list(cmd: &str) -> (&str, Option<(&'static str, &str)>) {
    for (pos, c) in top_level(cmd) {
        let after = &cmd[pos + 1..];
        match c {
            '&' if after.starts_with('&') => return (&cmd[..pos], Some(("&&", &after[1..]))),
            '|' if after.starts_with('|') => return (&cmd[..pos], Some(("||", &after[1..]))),
            '&' => {
                let prev = cmd[..pos].chars().last();
                if !matches!(prev, Some('>') | Some('|')) {
                    return (&cmd[..pos], Some(("&", after)));
                }
            }
            _ => {}
        }
    }
    (cmd, None)
}

/// Head keyword of a statement after peeling one-line `if (c)` prefixes.
fn leading_kw(stmt: &str) -> String {
    let mut cur = stmt.to_string();
    loop {
        let (head, rest) = split_head(&cur);
        if head == "if" {
            if let Ok((_, after)) = if_parts(rest) {
                let after = after.trim();
                if !after.is_empty() && after != "then" {
                    cur = after.to_string();
                    continue;
                }
            }
        }
        return head.to_string();
    }
}

/// The glob of a `case` label: text up to the first unquoted `:`.
fn parse_case_pattern(rest: &str) -> Option<String> {
    let mut quote: Option<char> = None;
    let mut it = rest.char_indices();
    while let Some((i, c)) = it.next() {
        if c == '\\' {
            it.next();
            continue;
        }
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '\'' | '"' => quote = Some(c),
                ':' => {
                    let pat = rest[..i].trim();
                    return if pat.is_empty() {
                        None
                    } else {
                        Some(pat.to_string())
                    };
                }
                _ => {}
            },
        }
    }
    None
}

/// Labels of the outermost switch in `pieces`, in source order.
fn prescan_labels(pieces: &[String]) -> Vec<Label> {
    let mut labels = Vec::new();
    let mut nest = 0usize;
    for p in pieces {
        let kw = leading_kw(p);
        match kw.as_str() {
            "switch" => nest += 1,
            "endsw" => nest = nest.saturating_sub(1),
            "default" | "default:" if nest == 0 => labels.push(Label::Default),
            "case" if nest == 0 => {
                let (_, rest) = split_head(p);
                labels.push(match parse_case_pattern(rest) {
                    Some(pat) => Label::Pat(pat),
                    None => Label::Bad,
                });
            }
            _ => {}
        }
    }
    labels
}

/// zsh `case` pattern for a csh label. A parameter expansion in csh is
/// matched as a glob; zsh would match it literally, so it gets `${~…}`.
fn case_pattern(pat: &str) -> String {
    let w = words::translate_word(pat);
    let quoted = w.starts_with('"') || w.starts_with('\'');
    if !quoted && (w.contains('$') || w.contains('`')) {
        format!("${{~${{:-{w}}}}}")
    } else {
        w
    }
}


/// True when `stmt` starts with a word this layer handles itself (block
/// keywords, jumps, `repeat`, `onintr`, `label:`).
fn is_control_head(stmt: &str) -> bool {
    let (head, _) = split_head(stmt);
    matches!(
        head,
        "if" | "else"
            | "endif"
            | "foreach"
            | "while"
            | "end"
            | "switch"
            | "case"
            | "default"
            | "default:"
            | "endsw"
            | "break"
            | "continue"
            | "breaksw"
            | "goto"
            | "onintr"
            | "repeat"
    ) || (head.len() > 1 && head.ends_with(':'))
}

/// Variable holding the label a `goto` selected for region `rid`.
fn pc_var(rid: usize) -> String {
    if rid == TOP_REGION {
        "__csh_pc".to_string()
    } else {
        format!("__csh_pc{rid}")
    }
}

/// Binary operators of a parenthesis-less `if` expression, as separate
/// words. `&&`, `||`, `|` and `&` are absent: tcsh splits the line into a
/// command list at them before `if` sees it (`if -f a && -r a echo x` is
/// `if -f a` followed by a list, which fails with `if: Empty if.`). `<` and `>`
/// (and `<=`, `>=`, `<<`, `>>`) are redirections wherever they stand.
const BINARY_OPS: &[&str] = &[
    "^", "==", "!=", "=~", "!~", "+", "-", "*", "/", "%",
];

/// Index just past one operand starting at word `i`: `!`/`~` prefixes,
/// then an optional `-X` file test, then one word.
fn operand_end(ws: &[String], mut i: usize) -> Option<usize> {
    while i < ws.len() && matches!(ws[i].as_str(), "!" | "~") {
        i += 1;
    }
    let is_file_test = |w: &str| {
        let b = w.as_bytes();
        b.len() == 2 && b[0] == b'-' && b[1].is_ascii_alphabetic()
    };
    if i < ws.len() && is_file_test(&ws[i]) {
        i += 1;
    }
    (i < ws.len()).then_some(i + 1)
}

/// Number of leading words of `ws` that form one csh expression:
/// `operand (binary-op operand)*`. The first word that cannot extend it
/// belongs to the command of a parenthesis-less `if`.
fn expr_word_count(ws: &[String]) -> usize {
    let Some(mut i) = operand_end(ws, 0) else {
        return 0;
    };
    while i < ws.len() && BINARY_OPS.contains(&ws[i].as_str()) {
        match operand_end(ws, i + 1) {
            Some(j) => i = j,
            None => break,
        }
    }
    i
}

/// Split the text after `if` into (expression, rest). `(e) rest` gives the
/// parenthesised group; without a parenthesis the expression is found by
/// [`expr_word_count`], like tcsh's `if 1 == 1 echo a`.
fn if_parts(rest: &str) -> Result<(String, String), &'static str> {
    if rest.starts_with('(') {
        let (inner, after) = paren_group(rest)?;
        return Ok((inner.to_string(), after.to_string()));
    }
    // Redirections may stand anywhere on the line; they belong to the command.
    let mut ws = Vec::new();
    let mut redirs: Vec<String> = Vec::new();
    let mut it = split_words(rest).into_iter();
    while let Some(w) = it.next() {
        if w.starts_with(['<', '>']) {
            let bare_op = w.chars().all(|c| matches!(c, '<' | '>' | '&' | '!'));
            redirs.push(w);
            if bare_op {
                redirs.extend(it.next());
            }
        } else {
            ws.push(w);
        }
    }
    let n = expr_word_count(&ws);
    if n == 0 {
        return Err("if: Expression Syntax.");
    }
    let expr = ws[..n].join(" ");
    let mut after = ws[n..].join(" ");
    if after.starts_with(['&', '|']) {
        // `&&`, `||`, `|`, `&` end the `if` command: nothing to run.
        after.clear();
    } else if !after.is_empty() && !redirs.is_empty() {
        after.push(' ');
        after.push_str(&redirs.join(" "));
    }
    Ok((expr, after))
}

/// For `if (c1) if (c2) … if (cN) then` the conditions c1..cN; `None` when
/// the chain of one-line `if`s does not end in `then`.
fn if_chain(inner: &str, after: &str) -> Option<Vec<String>> {
    let mut conds = vec![inner.to_string()];
    let mut cur = after.trim().to_string();
    loop {
        let (head, rest) = split_head(&cur);
        if head != "if" {
            return None;
        }
        let (c, a) = if_parts(rest).ok()?;
        conds.push(c);
        let a = a.trim().to_string();
        if a == "then" {
            return Some(conds);
        }
        if a.is_empty() {
            return None;
        }
        cur = a;
    }
}

/// An unquoted `{a,b}` list (a `{}` or `{a}` stays literal in csh).
fn has_brace_list(w: &str) -> bool {
    // 0: brace without comma, 1: with comma, 2: `${…}`.
    let mut open: Vec<u8> = Vec::new();
    let mut quote: Option<char> = None;
    let mut prev = ' ';
    let mut chars = w.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            chars.next();
            prev = c;
            continue;
        }
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '\'' | '"' | '`' => quote = Some(c),
                '{' => open.push(if prev == '$' { 2 } else { 0 }),
                ',' => {
                    if let Some(f) = open.last_mut().filter(|f| **f == 0) {
                        *f = 1;
                    }
                }
                '}' => {
                    if open.pop() == Some(1) {
                        return true;
                    }
                }
                _ => {}
            },
        }
        prev = c;
    }
    false
}

/// An unquoted `*`, `?` or `[` in a word without variable or command
/// substitution.
fn has_unquoted_glob(w: &str) -> bool {
    if w.contains('$') || w.contains('`') {
        return false;
    }
    let mut quote: Option<char> = None;
    let mut chars = w.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            chars.next();
            continue;
        }
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '\'' | '"' => quote = Some(c),
                '*' | '?' | '[' => return true,
                _ => {}
            },
        }
    }
    false
}

/// `name` of a word that is exactly `$name` or `${name}` for a plain
/// identifier (not `$status`, not a subscript or `$#`/`$?` form).
fn bare_variable(w: &str) -> Option<&str> {
    let name = w
        .strip_prefix("${")
        .and_then(|r| r.strip_suffix('}'))
        .or_else(|| w.strip_prefix('$'))?;
    let ident = name.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_alphanumeric() || c == '_');
    (ident && name != "status").then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr(src: &str) -> String {
        super::super::translate(src).unwrap_or_else(|e| panic!("translate failed: {e}"))
    }

    /// Translate and run under `/bin/zsh -f`; None when zsh is unavailable.
    /// Returns (stdout, stderr, exit status).
    fn run(src: &str) -> Option<(String, String, i32)> {
        let zsh = ["/bin/zsh", "/usr/bin/zsh", "/usr/local/bin/zsh"]
            .into_iter()
            .find(|p| std::path::Path::new(p).exists())?;
        let code = tr(src);
        let o = std::process::Command::new(zsh)
            .args(["-f", "-c", &code])
            .output()
            .ok()?;
        Some((
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
            o.status.code().unwrap_or(-1),
        ))
    }

    /// Expected stdout, but only when a zsh was found to run it.
    fn check(src: &str, want: &str) {
        if let Some((o, e, _)) = run(src) {
            assert_eq!(o, want, "script:\n{src}\nstderr: {e}\nzsh:\n{}", tr(src));
        }
    }

    #[test]
    fn one_line_if_ends_at_pipeline_and_list_operator_wraps_the_if() {
        // tcsh: `if (0) echo a && echo c` prints c; `if (1) false || echo c` prints c.
        check("if (\"\") echo a && echo c; echo b", "c\nb\n");
        check("if (1) echo a && echo c", "a\nc\n");
        check("if (1) false || echo c", "c\n");
        check("if (\"\") echo a || echo c", "");
        check("if (1) echo a | cat; echo b", "a\nb\n");
        check("if (\"\") echo a | cat; echo b", "b\n");
    }

    #[test]
    fn one_line_if_background_applies_to_the_whole_if() {
        let z = tr("if (1) echo a & echo b");
        assert!(z.contains("fi &\n"), "{z}");
    }

    #[test]
    fn semicolon_joined_block_and_closer_followed_by_command() {
        check("if (1) then; echo a; endif", "a\n");
        check("if (\"\") then; echo a; else; echo b; endif; echo c", "b\nc\n");
        check("foreach i (1 2); echo $i; end; echo post", "1\n2\npost\n");
    }

    #[test]
    fn else_if_chain_and_else_command_forms() {
        check("if (\"\") then\necho a\nelse if (1) then\necho b\nelse\necho c\nendif", "b\n");
        check("if (\"\") then\necho a\nelse if (\"\") then\necho b\nelse\necho c\nendif", "c\n");
        check("if (\"\") then\necho a\nelse echo b\nendif", "b\n");
        // `else if (c) cmd` is an else branch holding a one-line if.
        check("if (\"\") then\necho a\nelse if (1) echo b\necho c\nendif\necho d", "b\nc\nd\n");
        check(
            "if (\"\") then\nif (\"\") then\necho no\nelse\necho inner\nendif\nelse\necho outer\nendif",
            "outer\n",
        );
    }

    #[test]
    fn second_else_never_runs() {
        check("if (\"\") then\necho a\nelse\necho b\nelse\necho c\nendif", "b\n");
        check("if (1) then\nelse\necho a\nelse\necho b\nendif", "");
    }

    #[test]
    fn one_line_if_with_block_opener_nests_the_block() {
        check("if (1) foreach i (1 2)\necho $i\nend\necho z", "1\n2\nz\n");
        check("if (1) switch (b)\ncase a:\necho A\nbreaksw\ncase b:\necho B\nbreaksw\nendsw\necho z", "B\nz\n");
        check("if (1) if (1) echo x", "x\n");
    }

    #[test]
    fn foreach_variable_survives_and_empty_list_runs_nothing() {
        check("foreach i (1 2 3)\nend\necho $i", "3\n");
        check("foreach i ()\necho no\nend\necho z", "z\n");
        check("foreach i (a b)\nforeach j (1 2)\nif (1) then\ncontinue\nendif\necho no\nend\nend\necho z", "z\n");
    }

    #[test]
    fn while_with_break_and_repeat() {
        check("while (1)\necho a\nbreak\nend\necho ok", "a\nok\n");
        check("while (\"\")\necho no\nend\necho z", "z\n");
        check("repeat 2 echo hi", "hi\nhi\n");
        check("repeat 0 echo a\necho z", "z\n");
        check("repeat 2 echo a && echo b", "a\na\nb\n");
        check("repeat 2 echo a | cat", "a\na\n");
    }

    #[test]
    fn switch_fallthrough_stops_at_breaksw() {
        let sw = "switch (a)\ncase a:\necho A\ncase b:\necho B\nbreaksw\ncase c:\necho C\nendsw";
        check(sw, "A\nB\n");
        check(&sw.replace("(a)", "(c)"), "C\n");
        check(&sw.replace("(a)", "(z)"), "");
        check("switch (a)\ncase a:\ncase b:\necho AB\nendsw", "AB\n");
    }

    #[test]
    fn switch_default_matches_when_reached_in_source_order() {
        // tcsh: default before a matching later label wins, then falls through.
        let sw = "switch (zz)\ndefault:\necho D\ncase a:\necho A\nbreaksw\ncase zz:\necho Z\nendsw";
        check(sw, "D\nA\n");
        check(&sw.replace("(zz)", "(qq)"), "D\nA\n");
        check("switch (a)\ncase a:\necho A\ndefault:\necho D\nendsw", "A\nD\n");
    }

    #[test]
    fn switch_case_patterns_are_globs_and_case_sensitive() {
        check("switch (foo.c)\ncase *.h:\necho h\nbreaksw\ncase *.[cC]:\necho c\nbreaksw\nendsw", "c\n");
        check("switch (a)\ncase A:\necho upper\nbreaksw\nendsw\necho after", "after\n");
        check("switch (\"a b\")\ncase \"a b\":\necho spc\nbreaksw\nendsw", "spc\n");
        // Text after the label colon is dropped by tcsh.
        check("switch (a)\ncase a: echo hi\necho next\nbreaksw\nendsw", "next\n");
    }

    #[test]
    fn break_and_continue_inside_switch_target_the_csh_loop() {
        let body = |kw: &str| {
            format!("foreach i (1 2 3)\nswitch ($i)\ncase 2:\n{kw}\ndefault:\necho $i\nendsw\nend\necho z")
        };
        check(&body("continue"), "1\n3\nz\n");
        check(&body("break"), "1\nz\n");
    }

    #[test]
    fn breaksw_leaves_the_switch_from_nested_blocks() {
        check(
            "switch (a)\ncase a:\nif (1) then\necho in\nbreaksw\nendif\necho nr\nendsw\necho after",
            "in\nafter\n",
        );
        check(
            "switch (a)\ncase a:\nforeach i (1 2)\necho $i\nbreaksw\nend\necho nr\nendsw\necho after",
            "1\nafter\n",
        );
    }

    #[test]
    fn nested_switches_keep_separate_dispatch() {
        check(
            "switch (a)\ncase a:\nswitch (b)\ncase b:\necho inner\nbreaksw\nendsw\necho outer\nbreaksw\ncase c:\necho no\nendsw",
            "inner\nouter\n",
        );
    }

    #[test]
    fn break_continue_breaksw_outside_a_block_raise_tcsh_errors() {
        for (src, msg) in [
            ("break", "break: Not in while/foreach."),
            ("continue", "continue: Not in while/foreach."),
            ("breaksw", "breaksw: endsw not found."),
            ("end", "end: Not in while/foreach."),
            ("else", "else: endif not found."),
        ] {
            if let Some((o, e, rc)) = run(&format!("echo pre\n{src}\necho post")) {
                assert_eq!(o, "pre\n", "{src}");
                assert!(e.contains(msg), "{src}: {e}");
                assert_eq!(rc, 1, "{src}");
            }
        }
        // Not executed, so no error.
        check("if (\"\") break\necho ok", "ok\n");
    }

    #[test]
    fn stray_endif_and_endsw_are_noops() {
        check("endif\necho a\nendsw\ncase a:\ndefault:\necho b", "a\nb\n");
    }

    #[test]
    fn syntax_errors_become_runtime_stubs_after_earlier_output() {
        for (src, msg) in [
            ("if (1)", "if: Empty if."),
            ("if", "if: Too few arguments."),
            ("if (1) then echo", "if: Improper then."),
            ("foreach", "foreach: Too few arguments."),
            ("foreach i 1 2", "foreach: Words not parenthesized."),
            ("foreach 1x (a)", "foreach: Variable name must begin with a letter."),
            ("while", "while: Too few arguments."),
            ("switch", "switch: Too few arguments."),
            ("switch (a b)", "Syntax Error."),
            ("repeat", "repeat: Too few arguments."),
            ("repeat x echo a", "repeat: Badly formed number."),
            ("goto", "goto: Too few arguments."),
            ("goto a b", "goto: Too many arguments."),
            ("endif x", "endif: Too many arguments."),
            ("end x", "end: Too many arguments."),
            ("done: echo x", "done:: Too many arguments."),
        ] {
            if let Some((o, e, rc)) = run(&format!("echo pre\n{src}\necho post")) {
                assert_eq!(o, "pre\n", "{src}");
                assert!(e.contains(msg), "{src}: {e}");
                assert_eq!(rc, 1, "{src}");
            }
        }
    }

    #[test]
    fn unterminated_blocks_report_tcsh_text() {
        let err = |src: &str| super::super::translate_partial(src).unwrap_err();
        assert_eq!(err("if (1) then\necho a"), "then: then/endif not found.");
        assert_eq!(err("if (1) then\necho a\nelse\necho b"), "else: endif not found.");
        assert_eq!(err("foreach i (1)\necho $i"), "foreach: end not found.");
        assert_eq!(err("while (1)\nbreak"), "while: end not found.");
        assert_eq!(err("switch (a)\ncase a:\necho a"), "switch: endsw not found.");
        // The innermost open block is the one reported.
        assert_eq!(err("foreach i (1)\nif (1) then"), "then: then/endif not found.");
        assert_eq!(err("if (1) then\nswitch (a)\ncase a:\nendif"), "switch: endsw not found.");
    }

    #[test]
    fn goto_backward_forward_and_out_of_loops() {
        check("echo a\ngoto done\necho b\ndone:\necho c", "a\nc\n");
        check("echo 1\ngoto L\nL:\necho 2\ngoto L2\necho skip\nL2:\necho 3", "1\n2\n3\n");
        check("if (1) then\ngoto L\nendif\necho skip\nL:\necho L", "L\n");
        check("foreach i (1 2 3)\nif (1) goto out\necho $i\nend\nout:\necho out", "out\n");
        check("foreach i (a b)\nswitch ($i)\ncase a:\ngoto out\nendsw\nend\nout:\necho out", "out\n");
        check("echo a\ngoto top2\ntop2:\necho b\nif (\"\") goto top2\necho c", "a\nb\nc\n");
    }

    #[test]
    fn goto_to_missing_label_errors_when_executed() {
        if let Some((o, e, rc)) = run("echo a\ngoto nolabel") {
            assert_eq!(o, "a\n");
            assert!(e.contains("nolabel: label not found."), "{e}");
            assert_eq!(rc, 1);
        }
        check("if (\"\") goto nolabel\necho ok", "ok\n");
    }

    #[test]
    fn goto_to_a_label_in_an_enclosing_loop_body_keeps_the_loop() {
        // tcsh: forward and backward jumps inside a foreach/while body leave
        // the loop running.
        check(
            "foreach i (1 2 3)\nif ($i == 2) goto skip\necho body $i\nskip:\necho tail $i\nend\necho done",
            "body 1\ntail 1\ntail 2\nbody 3\ntail 3\ndone\n",
        );
        check(
            "set n = 0\nforeach i (1 2)\nagain:\n@ n++\necho i=$i n=$n\nif ($n < 3) goto again\nend\necho n=$n",
            "i=1 n=1\ni=1 n=2\ni=1 n=3\ni=2 n=4\nn=4\n",
        );
        // From an inner loop to a label after it in the outer body.
        check(
            "foreach i (1 2)\nforeach j (a b)\nif ($j == b) goto nexti\necho $i$j\nend\nnexti:\necho -- $i\nend",
            "1a\n-- 1\n2a\n-- 2\n",
        );
        check(
            "set n = 0\nwhile ($n < 3)\n@ n++\nif ($n == 2) goto skip\necho n=$n\nskip:\nend\necho done",
            "n=1\nn=3\ndone\n",
        );
    }

    #[test]
    fn break_and_continue_count_the_goto_dispatchers_of_their_bodies() {
        check(
            "set i = 0\nwhile ($i < 6)\n@ i++\nif ($i == 2) continue\nmark:\nif ($i == 5) break\necho i=$i\nend\necho out=$i",
            "i=1\ni=3\ni=4\nout=5\n",
        );
        check(
            "foreach x (a b)\nswitch ($x)\ncase a:\necho a1\ngoto over\necho never\nover:\necho a2\nbreaksw\ncase b:\necho b\nbreaksw\nendsw\nend",
            "a1\na2\nb\n",
        );
        // breaksw out of an if inside a case body that holds a label.
        check(
            "switch (a)\ncase a:\nif (1) then\nL:\necho in\nbreaksw\nendif\necho nr\nendsw\necho after",
            "in\nafter\n",
        );
    }

    #[test]
    fn goto_into_an_if_branch_from_outside_runs_the_branch() {
        // tcsh enters the branch as if its condition had held; reaching the
        // following `else` then skips to `endif`.
        check("echo a\ngoto L\nif (0) then\necho no\nL:\necho in\nendif\necho after", "a\nin\nafter\n");
        check("goto L\nif (1) then\necho no\nelse\nL:\necho in\nendif\necho after", "in\nafter\n");
        check(
            "goto L\nif (0) then\necho no\nL:\necho in\nelse\necho else\nendif\necho after",
            "in\nafter\n",
        );
        // From the else branch back into the then branch of the same if.
        check(
            "if (\"\") then\necho no\ninthen:\necho in-then\nelse\necho in-else\ngoto inthen\nendif\necho after",
            "in-else\nin-then\nafter\n",
        );
        // Through two nested ifs.
        check(
            "goto L\nif (0) then\nif (0) then\nL:\necho deep\nendif\nendif\necho after",
            "deep\nafter\n",
        );
    }

    #[test]
    fn goto_into_a_loop_or_switch_body_from_outside_is_a_runtime_error() {
        // tcsh runs the body and then fails at `end`; not reproduced.
        for body in [
            "goto L\nforeach i (1)\nL:\necho in\nend",
            "goto L\nwhile (0)\nL:\necho in\nend",
            "goto L\nswitch (x)\ncase x:\nL:\necho in\nendsw",
        ] {
            if let Some((o, e, rc)) = run(&format!("echo pre\n{body}\necho post")) {
                assert_eq!(o, "pre\n", "{body}");
                assert!(e.contains("goto: L: jumping into a block is not supported"), "{body}: {e}");
                assert_eq!(rc, 1, "{body}");
            }
        }
        // The dead part of a goto to a label that exists nowhere still
        // reports tcsh's text.
        if let Some((_, e, rc)) = run("goto nolabel") {
            assert!(e.contains("nolabel: label not found."), "{e}");
            assert_eq!(rc, 1);
        }
    }

    #[test]
    fn computed_goto_reaches_labels_inside_bodies() {
        check(
            "foreach step (one two)\ngoto $step\none:\necho first\ngoto cont\ntwo:\necho second\ncont:\nend",
            "first\nsecond\n",
        );
        check("set where = beta\ngoto $where\nalpha:\necho alpha\nbeta:\necho beta", "beta\n");
    }

    #[test]
    fn label_resets_status() {
        check("false\nL:\necho st=$status", "st=0\n");
        check("set n = 0\nif (1) then\ntop:\n@ n++\nfalse\necho st=$status\nif ($n < 2) goto top\nendif\necho end=$status", "st=1\nst=1\nend=0\n");
    }

    #[test]
    fn onintr_label_runs_the_handler_wherever_the_signal_lands() {
        check("onintr cleanup\necho work\nkill -INT $$\necho never\ncleanup:\necho cleaned", "work\ncleaned\n");
        // From inside nested loops and a switch.
        check(
            "onintr out\nforeach i (1 2 3)\nswitch ($i)\ncase 2:\nkill -INT $$\ndefault:\necho $i\nendsw\nend\necho never\nout:\necho stopped at $i",
            "1\nstopped at 2\n",
        );
        if let Some((o, _, rc)) = run("onintr cleanup\necho a\nkill -INT $$\ncleanup:\necho c\nexit 5") {
            assert_eq!((o.as_str(), rc), ("a\nc\n", 5));
        }
        // `onintr -` ignores the signal.
        check("onintr -\nkill -INT $$\necho survived", "survived\n");
        // A missing label fails like goto.
        if let Some((_, e, rc)) = run("onintr nolabel\nkill -INT $$\nsleep 0") {
            assert!(e.contains("nolabel: label not found."), "{e}");
            assert_eq!(rc, 1);
        }
    }

    #[test]
    fn onintr_forms_translate_to_traps() {
        let z = tr("onintr -\nonintr");
        assert!(z.contains("trap '' INT") && z.contains("trap - INT"), "{z}");
        let z = tr("onintr cleanup\ncleanup:\necho c");
        assert!(z.contains("trap '__csh_pc='\\''cleanup'\\''; continue 1000' INT"), "{z}");
    }

    #[test]
    fn if_without_parentheses_parses_the_longest_expression() {
        check("if 1 echo a\necho post", "a\npost\n");
        check("if 0 echo a\necho post", "post\n");
        check("if 1 == 1 echo eq\nif 1 != 1 echo ne\nif ! 0 echo not", "eq\nnot\n");
        check("set n = 5\nif $n == 5 then\necho five\nelse\necho other\nendif", "five\n");
        check("set n = 5\nif $n == 1 then\necho one\nelse if $n =~ 5 then\necho m\nendif", "m\n");
        check("if -e /usr echo usr\nif ! -e /nonexistent_zz echo missing", "usr\nmissing\n");
        // `&&` ends the `if` command before the expression grows.
        if let Some((o, e, rc)) = run("echo pre\nif -e /usr && -d /usr echo x\necho post") {
            assert_eq!((o.as_str(), rc), ("pre\n", 1));
            assert!(e.contains("if: Empty if."), "{e}");
        }
        // A redirection stands anywhere and applies to the command.
        if let Some((o, _, _)) = run("set n = 1\nif $n > /dev/null echo hidden\necho shown") {
            assert_eq!(o, "shown\n");
        }
    }

    #[test]
    fn expression_split_matches_the_grammar() {
        let parts = |s: &str| if_parts(s).unwrap();
        assert_eq!(parts("1 echo a"), ("1".to_string(), "echo a".to_string()));
        assert_eq!(parts("$a == 1 echo a"), ("$a == 1".to_string(), "echo a".to_string()));
        assert_eq!(parts("-e f echo a"), ("-e f".to_string(), "echo a".to_string()));
        assert_eq!(parts("! -d f then"), ("! -d f".to_string(), "then".to_string()));
        assert_eq!(parts("$a + 1 == 3 echo"), ("$a + 1 == 3".to_string(), "echo".to_string()));
        assert_eq!(parts("1 && 1 echo"), ("1".to_string(), String::new()));
        assert_eq!(parts("(a) echo"), ("a".to_string(), " echo".to_string()));
        assert!(if_parts("").is_err());
    }

    #[test]
    fn nested_one_line_ifs_ending_in_then_fold_their_conditions() {
        // tcsh: with the first condition false only the one-line `if` is
        // skipped, so the block lines run unconditionally.
        check("if (\"\") if (1) then\necho ran\nendif\necho after", "ran\nafter\n");
        check("if (1) if (\"\") then\necho no\nendif\necho after", "after\n");
        check("if (1) if (1) then\necho both\nelse\necho no\nendif", "both\n");
        check("if (1) if (1) if (\"\") then\necho no\nendif\necho z", "z\n");
    }

    #[test]
    fn switch_word_is_globbed_and_checked_like_tcsh() {
        let dir = std::env::temp_dir().join(format!("csh_ctl_sw_{}", std::process::id()));
        let d = dir.display();
        let pre = format!("rm -rf {d}\nmkdir -p {d}\ncd {d}\ntouch one.c two.c three.h\n");
        check(&format!("{pre}switch (t*.h)\ncase three.h:\necho single\nendsw"), "single\n");
        for (word, msg) in [("*.c", "*.c: Ambiguous."), ("zz*", "zz*: No match."), ("{a,b}", "{a,b}: Ambiguous.")] {
            if let Some((o, e, rc)) = run(&format!("{pre}echo pre\nswitch ({word})\ncase x:\nendsw\necho post")) {
                assert_eq!((o.as_str(), rc), ("pre\n", 1), "{word}");
                assert!(e.contains(msg), "{word}: {e}");
            }
        }
        // Quoted globs and `{a}` stay literal.
        check("switch (\"*\")\ncase \"*\":\necho star\nendsw", "star\n");
        check("switch ({a})\ncase {a}:\necho lit\nendsw", "lit\n");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn switch_on_a_variable_checks_definition_and_word_count() {
        check("set v = ()\nswitch ($v)\ndefault:\necho empty\nendsw", "empty\n");
        check("set v = (a b)\nswitch (\"$v\")\ncase \"a b\":\necho joined\nendsw", "joined\n");
        check("set v = (a b)\nswitch ($v[2])\ncase b:\necho second\nendsw", "second\n");
        for (src, msg) in [
            ("set v = (a b)\nswitch ($v)\ndefault:\nendsw", "Syntax Error."),
            ("switch ($nope)\ndefault:\nendsw", "nope: Undefined variable."),
        ] {
            if let Some((o, e, rc)) = run(&format!("echo pre\n{src}\necho post")) {
                assert_eq!((o.as_str(), rc), ("pre\n", 1), "{src}");
                assert!(e.contains(msg), "{src}: {e}");
            }
        }
        // A foreach variable is a one-word scalar in zsh.
        check("foreach f (a.c b.h)\nswitch ($f)\ncase *.c:\necho c\nbreaksw\ndefault:\necho other\nendsw\nend", "c\nother\n");
    }

    /// Translate with end-of-file semantics and run under `zsh -f`.
    fn run_eof(src: &str) -> Option<(String, String, i32)> {
        let zsh = ["/bin/zsh", "/usr/bin/zsh", "/usr/local/bin/zsh"]
            .into_iter()
            .find(|p| std::path::Path::new(p).exists())?;
        let mut t = Translator::new();
        let mut out = String::new();
        for l in super::super::lex::logical_lines(src) {
            t.feed(&l, &mut out).unwrap_or_else(|e| panic!("feed: {e}"));
        }
        t.finish_eof(&mut out).unwrap_or_else(|e| panic!("finish_eof: {e}"));
        let o = std::process::Command::new(zsh)
            .args(["-f", "-c", &out])
            .output()
            .ok()?;
        Some((
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
            o.status.code().unwrap_or(-1),
        ))
    }

    /// Expected (stdout, stderr fragment, status) of an input that ends in
    /// an open block; every case was recorded from /bin/tcsh.
    fn check_eof(src: &str, out: &str, err: &str, rc: i32) {
        if let Some((o, e, r)) = run_eof(src) {
            assert_eq!(o, out, "stdout of:\n{src}\nstderr: {e}");
            assert!(e.contains(err), "stderr of:\n{src}\n{e}");
            assert_eq!(r, rc, "status of:\n{src}\nstderr: {e}");
        }
    }

    #[test]
    fn if_open_at_eof() {
        check_eof("echo pre\nif (1) then\necho yes", "pre\nyes\n", "", 0);
        check_eof("echo pre\nif (0) then\necho no\necho post", "pre\n", "then: then/endif not found.", 1);
        check_eof("echo pre\nif (0) then\necho a\nelse\necho b", "pre\nb\n", "", 0);
        check_eof("echo pre\nif (0) then\necho a\nelse if (1) then\necho b", "pre\nb\n", "", 0);
        check_eof("echo pre\nif (0) then\necho a\nelse if (0) then\necho b", "pre\n", "then: then/endif not found.", 1);
        // A taken branch that reaches `else` must skip to the missing endif.
        check_eof("echo pre\nif (1) then\necho a\nelse\necho b", "pre\na\n", "else: endif not found.", 1);
        check_eof("echo pre\nif (1) then\necho a\nelse if (1) then\necho b", "pre\na\n", "else: endif not found.", 1);
        // Closed inner if, open outer.
        check_eof("if (1) then\nif (1) then\necho a\nelse\necho b\nendif\nexit 4", "a\n", "", 4);
        // The last command's status is the script's.
        check_eof("if (1) then\necho a\nfalse", "a\n", "", 1);
    }

    #[test]
    fn loops_open_at_eof() {
        // First iteration runs, then the input ends.
        check_eof("echo pre\nforeach i (1 2)\necho $i", "pre\n1\n", "", 0);
        check_eof("echo pre\nwhile (1)\necho w", "pre\nw\n", "", 0);
        check_eof("foreach i (1 2)\necho $i\nfalse", "1\n", "", 1);
        // continue runs the next iteration; past the last one tcsh fails.
        check_eof("foreach i (1 2)\necho $i\ncontinue", "1\n2\n", "continue: end not found.", 1);
        check_eof(
            "foreach i (1 2)\necho $i\nif ($i == 1) continue\necho post $i",
            "1\n2\npost 2\n",
            "",
            0,
        );
        check_eof(
            "set n = 0\nwhile ($n < 2)\n@ n++\necho $n\ncontinue",
            "1\n2\n",
            "while: end not found.",
            1,
        );
        // Nothing to run, or break: tcsh skips to the missing end.
        check_eof("echo pre\nforeach i ()\necho body", "pre\n", "foreach: end not found.", 1);
        check_eof("echo pre\nwhile (0)\necho x", "pre\n", "while: end not found.", 1);
        check_eof("foreach i (1 2 3)\necho $i\nbreak", "1\n", "break: end not found.", 1);
        check_eof("while (1)\nif (1) then\necho x\nbreak", "x\n", "break: end not found.", 1);
        // A closed inner loop still iterates; the open outer one ends.
        check_eof("foreach i (1 2)\nforeach j (a b)\necho $i$j\nend\necho o$i", "1a\n1b\no1\n", "", 0);
        check_eof("foreach i (1 2)\nforeach j (a b)\necho $i$j\ncontinue", "1a\n1b\n", "continue: end not found.", 1);
    }

    #[test]
    fn switch_open_at_eof() {
        check_eof("echo pre\nswitch (a)\ncase a:\necho A", "pre\nA\n", "", 0);
        check_eof("echo pre\nswitch (b)\ncase a:\necho A", "pre\n", "switch: endsw not found.", 1);
        check_eof("switch (z)\ncase a:\necho A\ndefault:\necho D", "D\n", "", 0);
        check_eof("switch (a)\ncase a:\necho A\ncase b:\necho B", "A\nB\n", "", 0);
        check_eof("switch (a)\ncase a:\necho A\nbreaksw", "A\n", "breaksw: endsw not found.", 1);
        check_eof("foreach i (1 2)\nswitch ($i)\ncase 1:\necho one", "one\n", "", 0);
    }

    #[test]
    fn eof_semantics_combine_with_goto_and_status() {
        check_eof(
            "echo start\ngoto L\necho no\nL:\nforeach i (1 2)\necho $i",
            "start\n1\n",
            "",
            0,
        );
        check_eof(
            "false\nif (1) then\necho st=$status\nforeach i (a b)\necho $i $status\ncontinue",
            "st=0\na 0\nb 0\n",
            "continue: end not found.",
            1,
        );
    }

    #[test]
    fn finish_eof_without_an_open_block_is_finish() {
        let (mut a, mut b) = (String::new(), String::new());
        let (mut ta, mut tb) = (Translator::new(), Translator::new());
        for l in ["if (1) then", "echo a", "endif", "foreach i (1)", "false", "end"] {
            ta.feed(l, &mut a).unwrap();
            tb.feed(l, &mut b).unwrap();
        }
        ta.finish(&mut a).unwrap();
        tb.finish_eof(&mut b).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn every_opener_left_open_is_a_not_found_error_until_closed() {
        // `bins/zshrs.rs::csh_line` keeps reading while the error ends in
        // "not found." and runs the text once it translates.
        let blocks: [(&[&str], &str); 8] = [
            (&["if (1) then", "echo a", "endif"], "then: then/endif not found."),
            (&["if (1) then", "echo a", "else", "echo b", "endif"], "else: endif not found."),
            (&["foreach i (1 2)", "echo $i", "end"], "foreach: end not found."),
            (&["while (0)", "echo x", "end"], "while: end not found."),
            (&["switch (a)", "case a:", "echo a", "breaksw", "endsw"], "switch: endsw not found."),
            (&["if (1) foreach i (1 2)", "echo $i", "end"], "foreach: end not found."),
            (&["foreach i (1)", "if (1) then", "echo x", "endif", "end"], "then: then/endif not found."),
            (&["onintr c", "foreach i (1)", "echo $i", "end", "c:", "echo c"], "foreach: end not found."),
        ];
        for (lines, _) in blocks {
            let whole = super::super::translate_partial(&lines.join("\n")).unwrap();
            let mut seen_err = false;
            for k in 1..lines.len() {
                let prefix = lines[..k].join("\n");
                match super::super::translate_partial(&prefix) {
                    Ok(_) => {
                        // Only closed prefixes translate (a label line or a
                        // block that already ended).
                        assert!(!prefix.ends_with("else"), "{prefix}");
                    }
                    Err(e) => {
                        seen_err = true;
                        assert!(e.ends_with("not found."), "{prefix}: {e}");
                    }
                }
            }
            assert!(seen_err, "{lines:?}");
            assert!(!whole.is_empty());
        }
        // The message names the innermost open block.
        let err = |src: &str| super::super::translate_partial(src).unwrap_err();
        assert_eq!(err("if (1) then\necho a\nelse"), "else: endif not found.");
        assert_eq!(err("if (1) then\nforeach i (1)\nif (1) then"), "then: then/endif not found.");
        assert_eq!(err("while (1)\nswitch (a)\ncase a:"), "switch: endsw not found.");
    }

    #[test]
    fn accumulated_lines_translate_to_the_same_text_once_closed() {
        let lines = ["foreach i (1 2)", "if ($i == 1) then", "echo one", "else", "echo other", "endif", "end"];
        let full = super::super::translate(&lines.join("\n")).unwrap();
        let mut pending = String::new();
        let mut text = None;
        for l in lines {
            if !pending.is_empty() {
                pending.push('\n');
            }
            pending.push_str(l);
            match super::super::translate(&pending) {
                Ok(t) => text = Some(t),
                Err(e) => assert!(e.ends_with("not found."), "{e}"),
            }
        }
        assert_eq!(text.as_deref(), Some(full.as_str()));
        check(&lines.join("\n"), "one\nother\n");
    }

    #[test]
    fn same_variable_nesting_exit_and_brace_conditions() {
        check("foreach i (a b)\nforeach i (1 2)\necho in $i\nend\necho out $i\nend", "in 1\nin 2\nout 2\nin 1\nin 2\nout 2\n");
        if let Some((o, _, rc)) = run("foreach i (1 2 3)\nif ($i == 2) then\necho bye\nexit 7\nendif\necho $i\nend\necho never") {
            assert_eq!((o.as_str(), rc), ("1\nbye\n", 7));
        }
        check(
            "set n = 0\nwhile ({ test $n -lt 3 })\necho n=$n\n@ n++\nend\nif (! { false }) echo nf",
            "n=0\nn=1\nn=2\nnf\n",
        );
        check("repeat 3 echo x > /dev/null\necho ok", "ok\n");
        check("false | true\necho s1=$status\ntrue | false\necho s2=$status", "s1=0\ns2=1\n");
    }

    #[test]
    fn status_is_zero_inside_branches_and_after_blocks() {
        check("false\nif (1) then\necho st=$status\nendif", "st=0\n");
        check("false\nif (\"\") then\nelse\necho st=$status\nendif", "st=0\n");
        check("false\nif (\"\") then\nelse if (1) then\necho st=$status\nendif", "st=0\n");
        check("foreach i (1)\nfalse\nend\necho st=$status", "st=0\n");
        check("false\nforeach i (1)\necho in=$status\nend", "in=0\n");
        check("false\nswitch (a)\ncase a:\necho sw=$status\nendsw\necho after=$status", "sw=0\nafter=0\n");
        check("if (1) then\nfalse\nendif\necho st=$status", "st=0\n");
        // A bare command keeps its status.
        check("false\necho st=$status", "st=1\n");
    }

    #[test]
    fn script_exit_status_is_zero_after_a_closing_block() {
        if let Some((_, _, rc)) = run("foreach i (1)\nfalse\nend") {
            assert_eq!(rc, 0);
        }
        if let Some((_, _, rc)) = run("echo a\nfalse") {
            assert_eq!(rc, 1);
        }
        if let Some((_, _, rc)) = run("if (1) then\nexit 3\nendif") {
            assert_eq!(rc, 3);
        }
    }

    #[test]
    fn plain_scripts_get_no_dispatcher_or_status_noise() {
        let z = tr("if (1) then\necho a\nendif\nforeach i (1 2)\necho $i\nend");
        assert!(!z.contains("__csh_pc"), "{z}");
        assert!(z.contains("if ") && z.contains("fi\n") && z.contains("for i in 1 2; do"), "{z}");
        assert!(z.contains("done\n"), "{z}");
    }

    #[test]
    fn statement_splitting_honours_quotes_parens_and_expression_braces() {
        assert_eq!(split_stmts("a; b ';' c; (d; e)"), vec!["a", " b ';' c", " (d; e)"]);
        assert_eq!(
            split_stmts("if { a; b } then; echo ${x}; echo {p,q}"),
            vec!["if { a; b } then", " echo ${x}", " echo {p,q}"]
        );
        assert_eq!(split_stmts(r"echo a\;b; echo c"), vec![r"echo a\;b", " echo c"]);
    }

    #[test]
    fn one_line_command_cut_points() {
        assert_eq!(cut_list("echo a | cat"), ("echo a | cat", None));
        assert_eq!(cut_list("echo a |& cat"), ("echo a |& cat", None));
        assert_eq!(cut_list("echo a >& f"), ("echo a >& f", None));
        assert_eq!(cut_list("echo a && echo b"), ("echo a ", Some(("&&", " echo b"))));
        assert_eq!(cut_list("echo a || echo b"), ("echo a ", Some(("||", " echo b"))));
        assert_eq!(cut_list("sleep 1 & echo b"), ("sleep 1 ", Some(("&", " echo b"))));
        assert_eq!(cut_list("(a && b)"), ("(a && b)", None));
        assert_eq!(cut_list("echo 'a && b'"), ("echo 'a && b'", None));
    }

    #[test]
    fn parenthesis_groups_and_heads() {
        assert_eq!(paren_group("(a (b) c) rest"), Ok(("a (b) c", " rest")));
        assert_eq!(paren_group("(\")\" == x) y"), Ok(("\")\" == x", " y")));
        assert_eq!(paren_group("(a"), Err("Too many ('s."));
        assert_eq!(split_head("if(1)then"), ("if", "(1)then"));
        assert_eq!(split_head("foreach  i (a)"), ("foreach", "i (a)"));
    }
}
