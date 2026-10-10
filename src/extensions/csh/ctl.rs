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
//!   * `foreach v (w...)` / `end` → `for v in w...; do` / `done`
//!   * `while (e)` / `end` → `while c; do` / `done`
//!   * `repeat N cmd` → `repeat N; do cmd; done`
//!   * `switch (s)` … `endsw` → a one-iteration `for` loop. A `case` picks
//!     the index of the first matching label in source order (`default:`
//!     matches when reached, so later labels are only reachable by
//!     fallthrough); each label then opens a guard
//!     `if (( m && m <= k ))`, which gives csh fallthrough, and `breaksw`
//!     is `break`. The whole switch is buffered until its `endsw` because
//!     the dispatch needs every label.
//!   * `break` / `continue` count the `switch` loops between the statement
//!     and the csh loop (`break 2`).
//!   * `goto` / `label:` — labels at top level become `case` arms of a
//!     dispatcher loop with `;&` fallthrough; `goto` assigns the label and
//!     `continue`s the dispatcher. This needs the whole input, so `finish`
//!     re-translates the retained lines when a `goto` was seen.
//!   * `$status`: tcsh resets it to 0 on entering any branch, loop body and
//!     after `endif`/`end`/`endsw`. When the input mentions `status`,
//!     `finish` re-translates with `:` inserted at those points.
//!
//! Errors tcsh raises while executing a line (`break: Not in while/foreach.`,
//! `if: Empty if.`, …) become a runtime stub (`print -ru2 -- msg; exit 1`)
//! so output produced before the line still happens, like tcsh.
//!
//! Not supported: `onintr label` (Err), `goto` into a label that sits inside
//! a block (Err), `if expr cmd` without parentheses (stub, tcsh accepts it),
//! the tcsh quirk where `if (0) foreach …` / `if (0) if (1) then` run the
//! following block lines unconditionally (here the block nests under the
//! outer `if`).

use super::lex::split_words;
use super::{cmds, expr, words};

/// Open block on the translation stack.
enum Frame {
    /// zsh `if`; `extra_fi` counts dead `if false; then` nests opened by
    /// repeated `else` (tcsh skips everything after a second `else`).
    If { has_else: bool, extra_fi: usize },
    /// zsh `for`/`while` from csh `foreach`/`while`; `kw` names it in the
    /// "end not found" error.
    Loop { kw: &'static str },
    /// One-iteration `for` carrying a csh `switch`.
    Switch {
        id: usize,
        labels_seen: usize,
        body_open: bool,
        body_empty: bool,
    },
    /// `if c; then` opened by a one-line `if (c) <block opener>`; closed
    /// with `fi` right after the block it guards.
    Wrap,
}

/// A `switch` collected up to its `endsw`.
struct Buffered {
    word: String,
    pieces: Vec<String>,
    nest: usize,
}

/// Which extra translation features a pass has enabled.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Modes {
    goto: bool,
    status: bool,
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
    buf: Option<Buffered>,
    modes: Modes,
    is_retry: bool,
    base_indent: usize,
    raw: Vec<String>,
    start: Option<usize>,
    end: usize,
    next_id: usize,
    seen_goto: bool,
    seen_status: bool,
    last_closer: bool,
    goto_targets: Vec<String>,
    inner_labels: Vec<String>,
}

impl Translator {
    pub fn new() -> Self {
        Self::with_modes(Modes {
            goto: false,
            status: false,
        })
    }

    fn with_modes(modes: Modes) -> Self {
        Self {
            stack: Vec::new(),
            buf: None,
            modes,
            is_retry: false,
            base_indent: if modes.goto { 2 } else { 0 },
            raw: Vec::new(),
            start: None,
            end: 0,
            next_id: 0,
            seen_goto: false,
            seen_status: false,
            last_closer: false,
            goto_targets: Vec::new(),
            inner_labels: Vec::new(),
        }
    }

    /// Translate one logical line (comments stripped, continuations joined,
    /// `;` NOT yet split — a one-line `if (e) cmd1; cmd2` is one line).
    pub fn feed(&mut self, line: &str, out: &mut String) -> Result<(), String> {
        if self.start.is_none() {
            self.start = Some(out.len());
        }
        self.raw.push(line.to_string());
        if line.contains("status") {
            self.seen_status = true;
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

    /// Called at end of input; errors for unterminated blocks with tcsh's
    /// text (`then/endif not found.`, `end not found.`, `endsw not found.`).
    /// When a `goto` or a `status` mention was seen, the first-pass output is
    /// replaced by a second pass with the matching feature enabled.
    pub fn finish(&mut self, out: &mut String) -> Result<(), String> {
        let intact = out.len() == self.end && self.start.is_some_and(|s| s <= out.len());
        self.finish_pass(out)?;
        let want = Modes {
            goto: self.seen_goto,
            status: self.seen_status,
        };
        if self.is_retry || want == self.modes || !intact {
            return Ok(());
        }
        let mut sub = Self::with_modes(want);
        sub.is_retry = true;
        let mut text = String::new();
        if want.goto {
            text.push_str("__csh_pc='#start'\nwhile :; do\n  case $__csh_pc in\n  ('#start')\n");
        }
        for line in std::mem::take(&mut self.raw) {
            sub.feed(&line, &mut text)?;
        }
        sub.finish_pass(&mut text)?;
        out.truncate(self.start.unwrap_or(0));
        out.push_str(&text);
        Ok(())
    }

    /// End-of-input checks and trailers for this pass.
    fn finish_pass(&mut self, out: &mut String) -> Result<(), String> {
        if self.buf.is_some() {
            return Err("switch: endsw not found.".to_string());
        }
        if let Some(msg) = self.open_block_error() {
            return Err(msg.to_string());
        }
        if let Some(t) = self.goto_targets.iter().find(|t| self.inner_labels.contains(t)) {
            return Err(format!("goto: label {t}: inside a block is not supported"));
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
            Frame::Loop { kw: "while" } => Some("while: end not found."),
            Frame::Loop { .. } => Some("foreach: end not found."),
            Frame::Switch { .. } => Some("switch: endsw not found."),
            Frame::Wrap => None,
        })
    }

    // ---- output helpers -------------------------------------------------

    /// Append `text` as one line at nesting `level`.
    fn line(&self, out: &mut String, level: usize, text: &str) {
        out.push_str(&"  ".repeat(self.base_indent + level));
        out.push_str(text);
        out.push('\n');
    }

    /// Indent level of statements inside the innermost frame: one per open
    /// block, two per `switch` (its loop and the label guard).
    fn depth(&self) -> usize {
        self.stack
            .iter()
            .map(|f| if matches!(f, Frame::Switch { .. }) { 2 } else { 1 })
            .sum()
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
        self.line(out, level, text);
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

    // ---- statement dispatch --------------------------------------------

    fn piece(&mut self, stmt: &str, out: &mut String) -> Result<(), String> {
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
            "onintr" => match rest {
                "" => Ok("trap - INT".to_string()),
                "-" => Ok("trap '' INT".to_string()),
                _ => Err("onintr: interrupt handler label is not supported".to_string()),
            },
            "repeat" => self.repeat_text(rest),
            "if" => {
                if !rest.starts_with('(') {
                    return Ok(stub("if: Expression Syntax."));
                }
                match paren_group(rest) {
                    Err(m) => Ok(stub(m)),
                    Ok((inner, after)) => {
                        let after = after.trim();
                        if after.is_empty() {
                            Ok(stub("if: Empty if."))
                        } else if after == "then" || starts_with_word(after, "then") {
                            Ok(stub("if: Improper then."))
                        } else {
                            self.if_oneline_text(inner, after)
                        }
                    }
                }
            }
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

    /// `break`/`continue` for the innermost csh loop; `switch` loops in
    /// between are skipped with a count argument.
    fn loop_jump(&self, kw: &str) -> String {
        let mut switches = 0;
        for f in self.stack.iter().rev() {
            match f {
                Frame::Loop { .. } => return jump(kw, switches + 1),
                Frame::Switch { .. } => switches += 1,
                _ => {}
            }
        }
        stub(&format!("{kw}: Not in while/foreach."))
    }

    /// `breaksw`: leave the innermost switch loop, through any csh loops
    /// opened inside the case body.
    fn switch_break(&self) -> String {
        let mut loops = 0;
        for f in self.stack.iter().rev() {
            match f {
                Frame::Switch { .. } => return jump("break", loops + 1),
                Frame::Loop { .. } => loops += 1,
                _ => {}
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
        let value = if word.contains('$') || word.contains('`') {
            words::translate_word(word)
        } else {
            self.goto_targets.push(word.clone());
            sh_quote(word)
        };
        // Every csh loop and switch loop between here and the dispatcher.
        let depth = self
            .stack
            .iter()
            .filter(|f| matches!(f, Frame::Loop { .. } | Frame::Switch { .. }))
            .count();
        Ok(format!("__csh_pc={value}; {}", jump("continue", depth + 1)))
    }

    /// `label:` on a line of its own.
    fn kw_goto_label(&mut self, head: &str, rest: &str, out: &mut String) -> Result<(), String> {
        if !rest.is_empty() {
            self.emit_stub(out, &format!("{head}: Too many arguments."));
            return Ok(());
        }
        let name = &head[..head.len() - 1];
        if !self.modes.goto {
            return Ok(());
        }
        if !self.stack.is_empty() {
            self.inner_labels.push(name.to_string());
            return Ok(());
        }
        out.push_str("    ;&\n");
        out.push_str(&format!("  ({})\n", sh_quote(name)));
        self.last_closer = false;
        Ok(())
    }

    // ---- if / else / endif ---------------------------------------------

    fn kw_if(&mut self, rest: &str, out: &mut String) -> Result<(), String> {
        if rest.is_empty() {
            self.emit_stub(out, "if: Too few arguments.");
            return Ok(());
        }
        if !rest.starts_with('(') {
            self.emit_stub(out, "if: Expression Syntax.");
            return Ok(());
        }
        let (inner, after) = match paren_group(rest) {
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
            let c = expr::translate_condition(inner)?;
            self.emit(out, &format!("if {c}; then"));
            self.stack.push(Frame::If {
                has_else: false,
                extra_fi: 0,
            });
        } else if starts_with_word(after, "then") {
            self.emit_stub(out, "if: Improper then.");
        } else if self.opens_block(after) {
            let c = expr::translate_condition(inner)?;
            self.emit(out, &format!("if {c}; then"));
            self.stack.push(Frame::Wrap);
            self.piece(after, out)?;
        } else {
            let text = self.if_oneline_text(inner, after)?;
            self.emit(out, &text);
        }
        Ok(())
    }

    /// True when `stmt` (the command of a one-line `if`) starts a block
    /// that a later `end`/`endsw`/`endif` closes.
    fn opens_block(&self, stmt: &str) -> bool {
        let (head, rest) = split_head(stmt);
        match head {
            "foreach" | "while" | "switch" => true,
            "if" if rest.starts_with('(') => match paren_group(rest) {
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
        let (has_else, level) = match self.stack.last() {
            Some(Frame::If { has_else, .. }) => (*has_else, self.depth() - 1),
            _ => {
                self.emit_stub(out, "else: endif not found.");
                return Ok(());
            }
        };
        // `else if (c) then` continues the chain.
        let (head, r2) = split_head(rest);
        if head == "if" && r2.starts_with('(') {
            if let Ok((inner, after)) = paren_group(r2) {
                if after.trim() == "then" {
                    if has_else {
                        return self.dead_else(out);
                    }
                    let c = expr::translate_condition(inner)?;
                    let c = if self.modes.status { format!(":; {c}") } else { c };
                    self.line(out, level, &format!("elif {c}; then"));
                    self.last_closer = false;
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
        if let Some(Frame::If { extra_fi, .. }) = self.stack.last_mut() {
            *extra_fi += 1;
        }
        Ok(())
    }

    fn kw_endif(&mut self, rest: &str, out: &mut String) -> Result<(), String> {
        if !rest.is_empty() {
            self.emit_stub(out, "endif: Too many arguments.");
            return Ok(());
        }
        if let Some(Frame::If { extra_fi, .. }) = self.stack.last() {
            let extra = *extra_fi;
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
            .map(|w| words::translate_word(w))
            .collect();
        let words_text = if items.is_empty() {
            String::new()
        } else {
            format!(" {}", items.join(" "))
        };
        self.emit(out, &format!("for {var} in{words_text}; do"));
        self.stack.push(Frame::Loop { kw: "foreach" });
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
                let c = expr::translate_condition(inner)?;
                self.emit(out, &format!("while {c}; do"));
                self.stack.push(Frame::Loop { kw: "while" });
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
        let word = match args.len() {
            0 => "''".to_string(),
            1 => words::translate_word(&args[0]),
            _ => {
                self.emit_stub(out, "Syntax Error.");
                return Ok(());
            }
        };
        self.buf = Some(Buffered {
            word,
            pieces: Vec::new(),
            nest: 0,
        });
        Ok(())
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
                let b = self.buf.take().unwrap_or(Buffered {
                    word: String::new(),
                    pieces: Vec::new(),
                    nest: 0,
                });
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
        let level = self.depth().saturating_sub(1);
        if let Some(Frame::Switch {
            id,
            labels_seen,
            body_open,
            body_empty,
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
        if let Some(Frame::Switch {
            body_open,
            body_empty,
            ..
        }) = self.stack.last()
        {
            let (open, empty) = (*body_open, *body_empty);
            let level = self.depth() - 1;
            if open {
                if empty {
                    self.line(out, level + 1, ":");
                }
                self.line(out, level, "fi");
            }
            self.line(out, level - 1, "done");
            self.stack.pop();
            self.after_close(out);
        }
        Ok(())
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
fn stub(msg: &str) -> String {
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
    let mut s = stmt;
    loop {
        let (head, rest) = split_head(s);
        if head == "if" && rest.starts_with('(') {
            if let Ok((_, after)) = paren_group(rest) {
                let after = after.trim();
                if !after.is_empty() && after != "then" {
                    s = after;
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
        let err = |src: &str| super::super::translate(src).unwrap_err();
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
    fn goto_into_a_block_is_rejected() {
        let e = super::super::translate("if (1) then\nL:\necho a\nendif\ngoto L").unwrap_err();
        assert!(e.contains("inside a block"), "{e}");
    }

    #[test]
    fn onintr_forms() {
        let z = tr("onintr -\nonintr");
        assert!(z.contains("trap '' INT") && z.contains("trap - INT"), "{z}");
        assert!(super::super::translate("onintr cleanup").is_err());
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
