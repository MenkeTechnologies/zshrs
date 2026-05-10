//! Textual representations of syntax trees for zshrs
//!
//! Direct port from zsh/Src/text.c
//!
//! Converts parsed shell commands back to their textual representation.
//! Used for:
//! - Displaying function definitions (`type -f`)
//! - Job text (`jobs` command)
//! - History expansion
//! - Debugging output

use crate::parse::{
    CaseTerminator, CompoundCommand, ListOp, Redirect, RedirectOp, ShellCommand, ShellWord,
    SimpleCommand,
};

/// Binary operators in `[[ ... ]]` conditions (order matches the
/// `COND_*` enum from Src/zsh.h).
/// Port of the `cond_ops[]` literals Src/text.c references inside
/// `gettext2()` (line 415) when rendering condition expressions.
pub static COND_BINARY_OPS: &[&str] = &[
    "=", "==", "!=", "<", ">", "-nt", "-ot", "-ef", "-eq", "-ne", "-lt", "-gt", "-le", "-ge", "=~",
];

/// Check whether a token is a binary `[[ ... ]]` operator.
/// Port of `is_cond_binary_op()` from Src/text.c:58.
pub fn is_cond_binary_op(s: &str) -> bool {                                  // c:58
    COND_BINARY_OPS.contains(&s)
}

/// Text formatter configuration.
/// Port of the formatting flags `getpermtext()` (Src/text.c:279)
/// and `getjobtext()` (line 315) accept — newline vs single-line,
/// job-abbreviated, expand-tab indent.
#[derive(Debug, Clone)]
pub struct TextConfig {
    /// Expand tabs to this many spaces (0 = use actual tabs)
    pub expand_tabs: i32,
    /// Include newlines (false = single line with semicolons)
    pub newlines: bool,
    /// Is job text (abbreviated output)
    pub is_job: bool,
    /// Maximum output size (for job text)
    pub max_size: Option<usize>,
}

impl Default for TextConfig {
    fn default() -> Self {
        TextConfig {
            expand_tabs: 0,
            newlines: true,
            is_job: false,
            max_size: None,
        }
    }
}

impl TextConfig {
    pub fn job_text() -> Self {
        TextConfig {
            expand_tabs: 0,
            newlines: false,
            is_job: true,
            max_size: Some(80),
        }
    }

    pub fn single_line() -> Self {
        TextConfig {
            expand_tabs: -1,
            newlines: false,
            is_job: false,
            max_size: None,
        }
    }
}

/// Text formatter for shell-command rendering.
/// Port of the `tbuf` / `tindent` / `tpending` file-statics
/// Src/text.c keeps for assembling output — `taddchr()` (line 128),
/// `taddstr()` (line 146), `taddnl()` (line 227),
/// `taddpending()` (line 89) all mutate them.
pub struct TextFormatter {
    config: TextConfig,
    buffer: String,
    indent: usize,
    pending: Option<String>,
}

impl TextFormatter {
    pub fn new(config: TextConfig) -> Self {
        TextFormatter {
            config,
            buffer: String::with_capacity(256),
            indent: 0,
            pending: None,
        }
    }

    pub fn with_indent(mut self, indent: usize) -> Self {
        self.indent = indent;
        self
    }

    /// Format a command and return the text
    pub fn format(mut self, cmd: &ShellCommand) -> String {
        self.format_command(cmd);
        self.flush_pending();
        self.buffer
    }

    /// Format a list of commands
    pub fn format_list(mut self, cmds: &[ShellCommand]) -> String {
        for (i, cmd) in cmds.iter().enumerate() {
            if i > 0 {
                self.add_separator();
            }
            self.format_command(cmd);
        }
        self.flush_pending();
        self.buffer
    }

    fn add_char(&mut self, c: char) {
        if let Some(max) = self.config.max_size {
            if self.buffer.len() >= max {
                return;
            }
        }
        self.buffer.push(c);
    }

    fn add_str(&mut self, s: &str) {
        if let Some(max) = self.config.max_size {
            if self.buffer.len() >= max {
                return;
            }
            let remaining = max - self.buffer.len();
            if s.len() > remaining {
                self.buffer.push_str(&s[..remaining]);
                return;
            }
        }

        if self.config.newlines {
            self.buffer.push_str(s);
        } else {
            for c in s.chars() {
                self.add_char(if c == '\n' { ' ' } else { c });
            }
        }
    }

    fn flush_pending(&mut self) {
        if let Some(pending) = self.pending.take() {
            self.add_char('\n');
            self.add_str(&pending);
        }
    }

    fn add_newline(&mut self, no_semicolon: bool) {
        if self.config.newlines {
            self.flush_pending();
            self.add_char('\n');
            self.add_indent();
        } else if no_semicolon {
            self.add_char(' ');
        } else {
            self.add_str("; ");
        }
    }

    fn add_indent(&mut self) {
        if self.config.expand_tabs < 0 {
            return;
        }
        for _ in 0..self.indent {
            if self.config.expand_tabs > 0 {
                for _ in 0..self.config.expand_tabs {
                    self.add_char(' ');
                }
            } else {
                self.add_char('\t');
            }
        }
    }

    fn add_separator(&mut self) {
        if self.config.newlines {
            self.add_newline(false);
        } else {
            self.add_str("; ");
        }
    }

    fn inc_indent(&mut self) {
        self.indent += 1;
    }

    fn dec_indent(&mut self) {
        if self.indent > 0 {
            self.indent -= 1;
        }
    }

    fn format_command(&mut self, cmd: &ShellCommand) {
        match cmd {
            ShellCommand::Simple(simple) => self.format_simple(simple),
            ShellCommand::Pipeline(cmds, negated) => self.format_pipeline(cmds, *negated),
            ShellCommand::List(list) => self.format_list_cmd(list),
            ShellCommand::Compound(compound) => self.format_compound(compound),
            ShellCommand::FunctionDef(name, body) => self.format_function(name, body),
        }
    }

    fn format_simple(&mut self, cmd: &SimpleCommand) {
        // Assignments first
        for (name, value, is_append) in &cmd.assignments {
            self.add_str(name);
            if *is_append {
                self.add_char('+');
            }
            self.add_char('=');
            self.format_word(value);
            self.add_char(' ');
        }

        // Command and arguments
        let mut first = true;
        for word in &cmd.words {
            if !first {
                self.add_char(' ');
            }
            self.format_word(word);
            first = false;
        }

        // Redirections
        self.format_redirects(&cmd.redirects);
    }

    fn format_word(&mut self, word: &ShellWord) {
        match word {
            ShellWord::Literal(s) => self.add_str(s),
            ShellWord::Concat(parts) => {
                for part in parts {
                    self.format_word(part);
                }
            }
        }
    }

    fn format_pipeline(&mut self, cmds: &[ShellCommand], negated: bool) {
        if negated {
            self.add_str("! ");
        }
        for (i, cmd) in cmds.iter().enumerate() {
            if i > 0 {
                self.add_str(" | ");
            }
            self.format_command(cmd);
        }
    }

    fn format_list_cmd(&mut self, list: &[(ShellCommand, ListOp)]) {
        for (i, (cmd, op)) in list.iter().enumerate() {
            if i > 0 {
                match list.get(i - 1).map(|(_, o)| o) {
                    Some(ListOp::And) => self.add_str(" && "),
                    Some(ListOp::Or) => self.add_str(" || "),
                    Some(ListOp::Amp) => self.add_str(" & "),
                    Some(ListOp::Semi) | Some(ListOp::Newline) => {
                        if self.config.newlines {
                            self.add_newline(false);
                        } else {
                            self.add_str("; ");
                        }
                    }
                    None => {}
                }
            }
            self.format_command(cmd);

            // Handle trailing operator for last command
            if i == list.len() - 1 && op == &ListOp::Amp {
                self.add_str(" &")
            }
        }
    }

    fn format_compound(&mut self, compound: &CompoundCommand) {
        match compound {
            CompoundCommand::BraceGroup(cmds) => self.format_brace_group(cmds),
            CompoundCommand::Subshell(cmds) => self.format_subshell(cmds),
            CompoundCommand::If {
                conditions,
                else_part,
            } => {
                self.format_if(conditions, else_part);
            }
            CompoundCommand::For { var, words, body } => {
                self.format_for(var, words, body);
            }
            CompoundCommand::ForArith {
                init,
                cond,
                step,
                body,
            } => {
                self.format_for_arith(init, cond, step, body);
            }
            CompoundCommand::While { condition, body } => {
                self.format_while(condition, body);
            }
            CompoundCommand::Until { condition, body } => {
                self.format_until(condition, body);
            }
            CompoundCommand::Case { word, cases } => {
                self.format_case(word, cases);
            }
            CompoundCommand::Select { var, words, body } => {
                self.format_select(var, words, body);
            }
            CompoundCommand::Repeat { count, body } => {
                self.add_str("repeat ");
                self.add_str(count);
                self.add_newline(false);
                self.add_str("do");
                self.inc_indent();
                self.add_newline(false);
                for cmd in body {
                    self.format_command(cmd);
                    self.add_newline(false);
                }
                self.dec_indent();
                self.add_str("done");
            }
            CompoundCommand::Try {
                try_body,
                always_body,
            } => {
                self.add_char('{');
                self.inc_indent();
                self.add_newline(false);
                for cmd in try_body {
                    self.format_command(cmd);
                    self.add_newline(false);
                }
                self.dec_indent();
                self.add_str("} always {");
                self.inc_indent();
                self.add_newline(false);
                for cmd in always_body {
                    self.format_command(cmd);
                    self.add_newline(false);
                }
                self.dec_indent();
                self.add_char('}');
            }
            CompoundCommand::Coproc { name, body } => {
                self.add_str("coproc ");
                if let Some(n) = name {
                    self.add_str(n);
                    self.add_char(' ');
                }
                self.format_command(body);
            }
            CompoundCommand::Arith(expr) => {
                self.add_str("((");
                self.add_str(expr);
                self.add_str("))");
            }
            CompoundCommand::WithRedirects(cmd, redirects) => {
                self.format_command(cmd);
                self.format_redirects(redirects);
            }
        }
    }

    fn format_for(&mut self, var: &str, words: &Option<Vec<ShellWord>>, body: &[ShellCommand]) {
        self.add_str("for ");
        self.add_str(var);

        if let Some(word_list) = words {
            self.add_str(" in ");
            for (i, w) in word_list.iter().enumerate() {
                if i > 0 {
                    self.add_char(' ');
                }
                self.format_word(w);
            }
        }

        self.add_newline(false);
        self.add_str("do");
        self.inc_indent();
        self.add_newline(false);

        for cmd in body {
            self.format_command(cmd);
            self.add_newline(false);
        }

        self.dec_indent();
        self.add_newline(false);
        self.add_str("done");
    }

    fn format_for_arith(&mut self, init: &str, cond: &str, step: &str, body: &[ShellCommand]) {
        self.add_str("for ((");
        self.add_str(init);
        self.add_str("; ");
        self.add_str(cond);
        self.add_str("; ");
        self.add_str(step);
        self.add_str(")) do");
        self.inc_indent();
        self.add_newline(false);

        for cmd in body {
            self.format_command(cmd);
            self.add_newline(false);
        }

        self.dec_indent();
        self.add_newline(false);
        self.add_str("done");
    }

    fn format_while(&mut self, condition: &[ShellCommand], body: &[ShellCommand]) {
        self.add_str("while ");
        self.inc_indent();

        for cmd in condition {
            self.format_command(cmd);
        }

        self.dec_indent();
        self.add_newline(false);
        self.add_str("do");
        self.inc_indent();
        self.add_newline(false);

        for cmd in body {
            self.format_command(cmd);
            self.add_newline(false);
        }

        self.dec_indent();
        self.add_newline(false);
        self.add_str("done");
    }

    fn format_until(&mut self, condition: &[ShellCommand], body: &[ShellCommand]) {
        self.add_str("until ");
        self.inc_indent();

        for cmd in condition {
            self.format_command(cmd);
        }

        self.dec_indent();
        self.add_newline(false);
        self.add_str("do");
        self.inc_indent();
        self.add_newline(false);

        for cmd in body {
            self.format_command(cmd);
            self.add_newline(false);
        }

        self.dec_indent();
        self.add_newline(false);
        self.add_str("done");
    }

    fn format_case(
        &mut self,
        word: &ShellWord,
        cases: &[(Vec<ShellWord>, Vec<ShellCommand>, CaseTerminator)],
    ) {
        self.add_str("case ");
        self.format_word(word);
        self.add_str(" in");

        if cases.is_empty() {
            if self.config.newlines {
                self.add_newline(false);
            } else {
                self.add_char(' ');
            }
            self.add_str("esac");
            return;
        }

        self.inc_indent();

        for (patterns, body, terminator) in cases {
            if self.config.newlines {
                self.add_newline(false);
            } else {
                self.add_char(' ');
            }

            self.add_str("(");
            for (i, pat) in patterns.iter().enumerate() {
                if i > 0 {
                    self.add_str(" | ");
                }
                self.format_word(pat);
            }
            self.add_str(") ");

            self.inc_indent();
            for cmd in body {
                self.format_command(cmd);
            }
            self.dec_indent();

            match terminator {
                CaseTerminator::Break => self.add_str(" ;;"),
                CaseTerminator::Fallthrough => self.add_str(" ;&"),
                CaseTerminator::Continue => self.add_str(" ;|"),
            }
        }

        self.dec_indent();
        if self.config.newlines {
            self.add_newline(false);
        } else {
            self.add_char(' ');
        }
        self.add_str("esac");
    }

    fn format_if(
        &mut self,
        conditions: &[(Vec<ShellCommand>, Vec<ShellCommand>)],
        else_part: &Option<Vec<ShellCommand>>,
    ) {
        for (i, (cond, body)) in conditions.iter().enumerate() {
            if i == 0 {
                self.add_str("if ");
            } else {
                self.dec_indent();
                self.add_newline(false);
                self.add_str("elif ");
            }

            self.inc_indent();
            for cmd in cond {
                self.format_command(cmd);
            }
            self.dec_indent();

            self.add_newline(false);
            self.add_str("then");
            self.inc_indent();
            self.add_newline(false);

            for cmd in body {
                self.format_command(cmd);
                self.add_newline(false);
            }
        }

        if let Some(else_body) = else_part {
            self.dec_indent();
            self.add_newline(false);
            self.add_str("else");
            self.inc_indent();
            self.add_newline(false);

            for cmd in else_body {
                self.format_command(cmd);
                self.add_newline(false);
            }
        }

        self.dec_indent();
        self.add_newline(false);
        self.add_str("fi");
    }

    fn format_select(&mut self, var: &str, words: &Option<Vec<ShellWord>>, body: &[ShellCommand]) {
        self.add_str("select ");
        self.add_str(var);

        if let Some(word_list) = words {
            self.add_str(" in ");
            for (i, w) in word_list.iter().enumerate() {
                if i > 0 {
                    self.add_char(' ');
                }
                self.format_word(w);
            }
        }

        self.add_newline(false);
        self.add_str("do");
        self.add_newline(false);
        self.inc_indent();

        for cmd in body {
            self.format_command(cmd);
            self.add_newline(false);
        }

        self.dec_indent();
        self.add_newline(false);
        self.add_str("done");
    }

    fn format_function(&mut self, name: &str, body: &ShellCommand) {
        self.add_str(name);
        self.add_str("() ");

        if self.config.is_job {
            self.add_str("{ ... }");
            return;
        }

        self.add_str("{");
        self.inc_indent();
        self.add_newline(true);

        self.format_command(body);

        self.dec_indent();
        self.add_newline(false);
        self.add_str("}");
    }

    fn format_subshell(&mut self, cmds: &[ShellCommand]) {
        self.add_str("(");
        self.inc_indent();
        self.add_newline(true);

        for cmd in cmds {
            self.format_command(cmd);
            self.add_newline(false);
        }

        self.dec_indent();
        self.add_newline(false);
        self.add_str(")");
    }

    fn format_brace_group(&mut self, cmds: &[ShellCommand]) {
        self.add_str("{");
        self.inc_indent();
        self.add_newline(true);

        for cmd in cmds {
            self.format_command(cmd);
            self.add_newline(false);
        }

        self.dec_indent();
        self.add_newline(false);
        self.add_str("}");
    }

    /// Format a redirection list to a fresh string. Used by
    /// [`getredirs`] (the file-static text-buffer entry point);
    /// the trailing-space + pop dance C does is handled by the
    /// caller, so this returns the "redir1 redir2 …" body trimmed.
    pub(crate) fn format_redirects_only(mut self, redirects: &[Redirect]) -> String {
        for (i, r) in redirects.iter().enumerate() {
            if i > 0 {
                self.add_char(' ');
            }
            self.format_redirect(r);
        }
        self.buffer
    }

    fn format_redirects(&mut self, redirects: &[Redirect]) {
        if redirects.is_empty() {
            return;
        }

        self.add_char(' ');

        for redir in redirects {
            self.format_redirect(redir);
            self.add_char(' ');
        }

        // Remove trailing space
        if self.buffer.ends_with(' ') {
            self.buffer.pop();
        }
    }

    fn format_redirect(&mut self, redir: &Redirect) {
        // File descriptor variable
        if let Some(ref var) = redir.fd_var {
            self.add_char('{');
            self.add_str(var);
            self.add_char('}');
        } else if let Some(fd) = redir.fd {
            let default_fd = match redir.op {
                RedirectOp::Read
                | RedirectOp::ReadWrite
                | RedirectOp::HereDoc
                | RedirectOp::HereString
                | RedirectOp::DupRead => 0,
                _ => 1,
            };
            if fd != default_fd {
                self.add_str(&fd.to_string());
            }
        }

        // Operator
        let op = match redir.op {
            RedirectOp::Write => ">",
            RedirectOp::Clobber => ">|",
            RedirectOp::Append => ">>",
            RedirectOp::WriteBoth => "&>",
            RedirectOp::AppendBoth => "&>>",
            RedirectOp::ReadWrite => "<>",
            RedirectOp::Read => "<",
            RedirectOp::HereDoc => "<<",
            RedirectOp::HereString => "<<<",
            RedirectOp::DupRead => "<&",
            RedirectOp::DupWrite => ">&",
        };
        self.add_str(op);

        // Target
        if !matches!(redir.op, RedirectOp::DupRead | RedirectOp::DupWrite) {
            self.add_char(' ');
        }
        self.format_word(&redir.target);
    }
}

/// Port of `getpermtext()` from `Src/text.c:279`.
///
/// C body initialises the file-static text buffer (tindent =
/// start_indent, tnewlins = 1, tjob = 0, fresh tbuf), runs
/// `gettext2` over the wordcode tree, then untokenizes and
/// returns the buffer.
///
/// Rust port runs the typed-AST formatter (which carries its own
/// String) and returns its output. The file-static [`TextBuffer`]
/// singleton is used only by callers that explicitly invoke the
// get a permanent textual representation of n                             // c:275
/// `taddX` helpers (which mirror C's per-byte buffer
/// manipulation); high-level entry points like this one don't
/// touch it so they can be invoked from parallel tests safely.
pub fn getpermtext(cmd: &ShellCommand) -> String {                           // c:279
    TextFormatter::new(TextConfig::default()).format(cmd)
}

/// Port of `getjobtext()` from `Src/text.c:315`.
///
// get a representation of n in a job text buffer                          // c:311
/// C body uses a static `jbuf[JOBTEXTSIZE]` buffer in single-line
/// (tnewlins = 0, tjob = 1) mode. Rust port routes through
/// TextConfig::job_text without touching the global TextBuffer.
pub fn getjobtext(cmd: &ShellCommand) -> String {                            // c:315
    TextFormatter::new(TextConfig::job_text()).format(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simple_cmd(words: &[&str]) -> ShellCommand {
        ShellCommand::Simple(SimpleCommand {
            words: words
                .iter()
                .map(|s| ShellWord::Literal(s.to_string()))
                .collect(),
            assignments: vec![],
            redirects: vec![],
        })
    }

    #[test]
    fn test_simple_command() {
        let cmd = simple_cmd(&["echo", "hello"]);
        assert_eq!(getpermtext(&cmd), "echo hello");
    }

    #[test]
    fn test_pipeline() {
        let pipeline = ShellCommand::Pipeline(
            vec![
                simple_cmd(&["cat", "file"]),
                simple_cmd(&["grep", "pattern"]),
            ],
            false,
        );
        assert_eq!(getpermtext(&pipeline), "cat file | grep pattern");
    }

    #[test]
    fn test_negated_pipeline() {
        let pipeline = ShellCommand::Pipeline(vec![simple_cmd(&["test", "-f", "file"])], true);
        assert_eq!(getpermtext(&pipeline), "! test -f file");
    }

    #[test]
    fn test_and_list() {
        let list = ShellCommand::List(vec![
            (simple_cmd(&["test", "-f", "file"]), ListOp::And),
            (simple_cmd(&["cat", "file"]), ListOp::Semi),
        ]);
        let text = getpermtext(&list);
        assert!(text.contains("&&"));
    }

    #[test]
    fn test_or_list() {
        let list = ShellCommand::List(vec![
            (simple_cmd(&["test", "-f", "file"]), ListOp::Or),
            (simple_cmd(&["echo", "not found"]), ListOp::Semi),
        ]);
        let text = getpermtext(&list);
        assert!(text.contains("||"));
    }

    #[test]
    fn test_subshell() {
        let cmd =
            ShellCommand::Compound(CompoundCommand::Subshell(vec![simple_cmd(&["echo", "hi"])]));
        let text = getpermtext(&cmd);
        assert!(text.contains("("));
        assert!(text.contains(")"));
        assert!(text.contains("echo hi"));
    }

    #[test]
    fn test_brace_group() {
        let cmd = ShellCommand::Compound(CompoundCommand::BraceGroup(vec![simple_cmd(&[
            "echo", "hi",
        ])]));
        let text = getpermtext(&cmd);
        assert!(text.contains("{"));
        assert!(text.contains("}"));
    }

    #[test]
    fn test_job_text() {
        let cmd = simple_cmd(&["very", "long", "command", "with", "many", "arguments"]);
        let job_text = getjobtext(&cmd);
        assert!(job_text.len() <= 80);
    }

    #[test]
    fn test_single_line() {
        let cmd = ShellCommand::Compound(CompoundCommand::BraceGroup(vec![
            simple_cmd(&["echo", "a"]),
            simple_cmd(&["echo", "b"]),
        ]));
        let text = TextFormatter::new(TextConfig::single_line()).format(&cmd);
        assert!(!text.contains('\n'));
        assert!(text.contains(';'));
    }

    #[test]
    fn test_is_cond_binary_op() {
        assert!(is_cond_binary_op("="));
        assert!(is_cond_binary_op("-eq"));
        assert!(is_cond_binary_op("-nt"));
        assert!(!is_cond_binary_op("-f"));
        assert!(!is_cond_binary_op("foo"));
    }

    #[test]
    fn test_redirect_output() {
        let cmd = ShellCommand::Simple(SimpleCommand {
            words: vec![
                ShellWord::Literal("echo".to_string()),
                ShellWord::Literal("hello".to_string()),
            ],
            assignments: vec![],
            redirects: vec![Redirect {
                fd: Some(1),
                op: RedirectOp::Write,
                target: ShellWord::Literal("file.txt".to_string()),
                heredoc_content: None,
                fd_var: None,
            }],
        });
        let text = getpermtext(&cmd);
        assert!(text.contains("> file.txt"));
    }

    // -------------------------------------------------------------
    // text-buffer file-static singleton tests.
    //
    // Serialise via TBUF_TEST_LOCK because the buffer is process-wide.
    // -------------------------------------------------------------
    static TBUF_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_taddchr_appends() {
        let _g = TBUF_TEST_LOCK.lock();
        text_buffer_reset(true, false, 0);
        taddchr(b'x' as i32);
        taddchr(b'y' as i32);
        let b = text_buffer_lock().lock().unwrap();
        assert_eq!(b.buf, "xy");
    }

    #[test]
    fn test_taddstr_with_newlins_keeps_newlines() {
        let _g = TBUF_TEST_LOCK.lock();
        text_buffer_reset(true, false, 0);
        taddstr("a\nb");
        let b = text_buffer_lock().lock().unwrap();
        assert_eq!(b.buf, "a\nb");
    }

    #[test]
    fn test_taddstr_job_mode_flattens_newlines() {
        let _g = TBUF_TEST_LOCK.lock();
        text_buffer_reset(false, true, 0);
        taddstr("a\nb");
        let b = text_buffer_lock().lock().unwrap();
        assert_eq!(b.buf, "a b");
    }

    #[test]
    fn test_dec_tindent_clamps_at_zero() {
        let _g = TBUF_TEST_LOCK.lock();
        text_buffer_reset(true, false, 2);
        dec_tindent();
        dec_tindent();
        dec_tindent(); // would go negative — clamp at 0 per C DPUTS branch.
        let b = text_buffer_lock().lock().unwrap();
        assert_eq!(b.indent, 0);
    }

    #[test]
    fn test_taddpending_buffers_until_tdopending() {
        let _g = TBUF_TEST_LOCK.lock();
        text_buffer_reset(true, false, 0);
        taddpending("EOF\n", "body line\n");
        {
            let b = text_buffer_lock().lock().unwrap();
            assert_eq!(b.buf, "");
            assert!(b.pending.as_deref() == Some("EOF\nbody line\n"));
        }
        tdopending();
        let b = text_buffer_lock().lock().unwrap();
        assert!(b.buf.contains("EOF"));
        assert!(b.pending.is_none());
    }

    #[test]
    fn test_taddnl_no_semicolon_when_flat() {
        let _g = TBUF_TEST_LOCK.lock();
        text_buffer_reset(false, false, 0);
        taddstr("foo");
        taddnl(0);
        taddstr("bar");
        let b = text_buffer_lock().lock().unwrap();
        assert_eq!(b.buf, "foo; bar");
        // Same with no_semicolon=1 → just space.
        drop(b);
        text_buffer_reset(false, false, 0);
        taddstr("foo");
        taddnl(1);
        taddstr("bar");
        let b = text_buffer_lock().lock().unwrap();
        assert_eq!(b.buf, "foo bar");
    }

    #[test]
    fn test_taddassign_simple() {
        let _g = TBUF_TEST_LOCK.lock();
        text_buffer_reset(true, false, 0);
        taddassign("PATH", Some("/usr/bin"), false, false);
        let b = text_buffer_lock().lock().unwrap();
        assert!(b.buf.starts_with("PATH=/usr/bin"));
    }

    #[test]
    fn test_taddassign_augment() {
        let _g = TBUF_TEST_LOCK.lock();
        text_buffer_reset(true, false, 0);
        taddassign("ARR", Some("(a b)"), true, false);
        let b = text_buffer_lock().lock().unwrap();
        assert!(b.buf.starts_with("ARR+=(a b)"));
    }

    #[test]
    fn test_zoutputtab_writes_tab_or_spaces() {
        let _g = TBUF_TEST_LOCK.lock();
        text_buffer_reset(true, false, 0);
        // expand_tabs = 0 → literal tab
        let mut buf: Vec<u8> = Vec::new();
        zoutputtab(&mut buf).unwrap();
        assert_eq!(buf, b"\t");
        // expand_tabs = 4 → 4 spaces
        text_buffer_lock().lock().unwrap().expand_tabs = 4;
        let mut buf: Vec<u8> = Vec::new();
        zoutputtab(&mut buf).unwrap();
        assert_eq!(buf, b"    ");
        // expand_tabs = -1 → no output
        text_buffer_lock().lock().unwrap().expand_tabs = -1;
        let mut buf: Vec<u8> = Vec::new();
        zoutputtab(&mut buf).unwrap();
        assert_eq!(buf, b"");
        text_buffer_lock().lock().unwrap().expand_tabs = 0;
    }
}

// ===========================================================
// Free fns moved verbatim from src/ported/exec.rs.
// ===========================================================
// BEGIN moved-from-exec-rs (free fns)
/// Format a function body the way zsh's `typeset -f` / `functions`
/// display it: each top-level statement on its own line (split on `;`
/// and `\n`), trailing semicolons stripped, no empty lines. Matches
/// `/bin/zsh -f -c 'f() { echo a; echo b; }; typeset -f f'` output:
///   f () {
///   (tab)echo a
///   (tab)echo b
///   }
/// Render a function body as zsh source.
/// Specialized variant of `getpermtext` (Src/text.c:279) for the
/// function-body subset (used by `functions`, `which -x`).
/// Source-text input variant (raw body string) — distinct from
/// the AST-based `getpermtext(&ShellCommand)` overload used when
/// the parsed tree is available.
pub struct FuncBodyFmt;

impl FuncBodyFmt {
pub fn render(body: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut depth_paren = 0i32;
    let mut depth_brace = 0i32;
    let mut in_squote = false;
    let mut in_dquote = false;
    let mut prev = '\0';
    for c in body.chars() {
        let escaped = prev == '\\';
        match c {
            '\'' if !in_dquote && !escaped => {
                in_squote = !in_squote;
                current.push(c);
            }
            '"' if !in_squote && !escaped => {
                in_dquote = !in_dquote;
                current.push(c);
            }
            '(' | '[' | '{' if !in_squote && !in_dquote => {
                if c == '{' {
                    depth_brace += 1;
                } else {
                    depth_paren += 1;
                }
                current.push(c);
            }
            ')' | ']' | '}' if !in_squote && !in_dquote => {
                if c == '}' {
                    depth_brace -= 1;
                } else {
                    depth_paren -= 1;
                }
                current.push(c);
            }
            ';' | '\n' if !in_squote && !in_dquote && depth_paren == 0 && depth_brace == 0 => {
                let t = current.trim().to_string();
                if !t.is_empty() {
                    lines.push(t);
                }
                current.clear();
            }
            _ => current.push(c),
        }
        prev = c;
    }
    let t = current.trim().to_string();
    if !t.is_empty() {
        lines.push(t);
    }
    lines.join("\n\t")
}
}  // impl FuncBodyFmt
// END moved-from-exec-rs (free fns)

// ===========================================================
// AST-text-buffer helpers — direct ports of the file-static
// text-builder routines from Src/text.c. In zsh these accumulate
// text into the globals `tbuf`/`tptr`/`tlim`/`tindent`/`tpending`/
// `tnewlins`/`tjob`/`text_expand_tabs` during AST decompilation
// (used by `whence -v`, job-text, fc, the `printprompt` debug
// path).
//
// Rust port: the C file-statics are reproduced as a single
// `TextBuffer` struct held inside a OnceLock<Mutex<…>>. Each fn
// below mutates the singleton, matching the C signatures byte-
// for-byte. The bodies match C's `Src/text.c:<line>` ports cited
// in each doc.
//
// `tpush` and `gettext2` operate on C's `Estate`/`wordcode`
// AST walker which zshrs doesn't have (the Rust formatter renders
// `ShellCommand` instead). Those keep the C signature shape but
// route through the Rust formatter where possible.
// ===========================================================

/// File-static state holder mirroring the text-buffer globals in
/// `Src/text.c:30+`.
///
/// Fields map 1:1 to C globals:
/// - `buf`     ↔ `tbuf` (heap string buffer being filled)
/// - `indent`  ↔ `tindent` (current indent depth in tabs)
/// - `pending` ↔ `tpending` (here-doc strings deferred until next \n)
/// - `newlins` ↔ `tnewlins` (true → emit real newlines+indent;
///                          false → flatten to "; "/" ")
/// - `job`     ↔ `tjob` (true while building job-text via
///                       `getjobtext`, false for permtext)
/// - `expand_tabs` ↔ `text_expand_tabs` (-1 = no tabs at all,
///                       0 = literal tab, N>0 = N spaces)
#[derive(Default)]
pub struct TextBuffer {
    pub buf: String,
    pub indent: i32,
    pub pending: Option<String>,
    pub newlins: bool,
    pub job: bool,
    pub expand_tabs: i32,
}

/// Singleton accessor for the text-buffer state.
///
/// Mirrors the file-static globals around line 30 of Src/text.c.
/// Lazily initialised on first use. Recovers from poisoning so a
/// panicking test in this module doesn't cascade-fail every other
/// test that grabs the lock.
pub fn text_buffer_lock() -> &'static std::sync::Mutex<TextBuffer> {
    static TBUF: std::sync::OnceLock<std::sync::Mutex<TextBuffer>> = std::sync::OnceLock::new();
    let m = TBUF.get_or_init(|| std::sync::Mutex::new(TextBuffer::default()));
    m.clear_poison();
    m
}

/// Reset the text-buffer state. Mirrors the inline init blocks at
/// `Src/text.c:298` (`getpermtext`) and `Src/text.c:333`
/// (`getjobtext`) — `tindent`, `tpending`, `tnewlins`, `tjob` all
/// re-seeded before each formatting pass.
pub fn text_buffer_reset(newlins: bool, job: bool, indent: i32) {
    let mut b = text_buffer_lock().lock().expect("text buffer poisoned");
    b.buf.clear();
    b.indent = indent;
    b.pending = None;
    b.newlins = newlins;
    b.job = job;
    // expand_tabs is preserved (controlled separately by the
    // `text_expand_tabs` global zsh exposes via tput / `printf`).
}

/// Port of `dec_tindent()` from `Src/text.c:69`.
///
/// C body:
/// ```c
/// DPUTS(tindent == 0, "attempting to decrement tindent below zero");
/// if (tindent > 0) tindent--;
/// ```
pub fn dec_tindent() {
    let mut b = text_buffer_lock().lock().expect("text buffer poisoned");
    if b.indent > 0 {
        b.indent -= 1;
    }
}

/// Port of `taddpending()` from `Src/text.c:88`.
///
/// C body buffers a here-doc terminator + body pair. On the next
/// significant newline ([`tdopending`]) the buffered string is
/// emitted prefixed with `\n`. Multiple calls concatenate (each
/// preceded by `\n`).
pub fn taddpending(str1: &str, str2: &str) {
    let mut b = text_buffer_lock().lock().expect("text buffer poisoned");
    let combined = format!("{}{}", str1, str2);
    match b.pending.as_mut() {
        Some(existing) => {
            existing.push('\n');
            existing.push_str(&combined);
        }
        None => b.pending = Some(combined),
    }
}

/// Port of `tdopending()` from `Src/text.c:113`.
///
/// C body:
/// ```c
/// if (tpending) {
///     taddchr('\n');
///     taddstr(tpending);
///     zsfree(tpending);
///     tpending = NULL;
/// }
/// ```
pub fn tdopending() {
    let drained = {
        let mut b = text_buffer_lock().lock().expect("text buffer poisoned");
        b.pending.take()
    };
    if let Some(s) = drained {
        taddchr(b'\n' as i32);
        taddstr(&s);
    }
}

/// Port of `taddchr()` from `Src/text.c:127`.
///
/// C body:
/// ```c
/// *tptr++ = c;
/// if (tptr == tlim) { tbuf = zrealloc(tbuf, tsiz *= 2); ... }
/// ```
///
/// Rust port: appends to `String`, which auto-grows. The realloc-
/// on-overflow logic from C is implicit.
// add a character to the text buffer                                       // c:124
pub fn taddchr(c: i32) {
    let mut b = text_buffer_lock().lock().expect("text buffer poisoned");
    if let Some(ch) = char::from_u32(c as u32) {
        b.buf.push(ch);
    }
}

/// Port of `taddstr()` from `Src/text.c:145`.
///
/// C body appends with newline-flatten semantics: when
/// `tnewlins == 0` (job-text mode), `\n` becomes ' '.
// add a string to the text buffer                                          // c:142
pub fn taddstr(s: &str) {
    let mut b = text_buffer_lock().lock().expect("text buffer poisoned");
    if b.newlins {
        b.buf.push_str(s);
    } else {
        for c in s.chars() {
            b.buf.push(if c == '\n' { ' ' } else { c });
        }
    }
}

/// Port of `taddlist()` from `Src/text.c:170`.
///
/// C body emits `num` words from the wordcode stream, space-
/// separated; trailing space is removed via `tptr--`.
///
/// WARNING: the wordcode `Estate` walker isn't ported. Callers
/// that hold their own list of strings should use `taddlist_strs`
/// instead. This entry preserves the C signature shape — taking
/// a slice of strings as a stand-in for the wordcode iteration.
pub fn taddlist(words: &[String]) {
    if words.is_empty() {
        return;
    }
    let mut b = text_buffer_lock().lock().expect("text buffer poisoned");
    let mut first = true;
    for w in words {
        if !first {
            b.buf.push(' ');
        }
        b.buf.push_str(w);
        first = false;
    }
}

/// Port of `taddassign()` from `Src/text.c:184`.
///
/// Emits `name=value` (or `name+=value`) to the buffer. For array
/// assignments emits `name=(v1 v2 …)`. The `typeset` flag enables
/// the typeset-style "name only" emission for `WC_ASSIGN_INC`.
pub fn taddassign(name: &str, value: Option<&str>, augment: bool, typeset: bool) {
    let mut b = text_buffer_lock().lock().expect("text buffer poisoned");
    b.buf.push_str(name);
    if augment {
        if typeset {
            b.buf.push(' ');
            return;
        }
        b.buf.push('+');
    }
    b.buf.push('=');
    drop(b);
    if let Some(v) = value {
        taddstr(v);
        let mut b = text_buffer_lock().lock().expect("text buffer poisoned");
        b.buf.push(' ');
    }
}

/// Port of `taddassignlist()` from `Src/text.c:213`.
///
/// C body emits `count` consecutive assignments from the
/// wordcode stream; the Rust port takes the resolved
/// `(name, value, augment)` triples.
pub fn taddassignlist(assigns: &[(String, Option<String>, bool)]) {
    if !assigns.is_empty() {
        taddchr(b' ' as i32);
    }
    for (name, value, augment) in assigns {
        taddassign(name, value.as_deref(), *augment, true);
    }
}

/// Port of `taddnl()` from `Src/text.c:227`.
///
/// C body:
/// ```c
/// if (tnewlins) {
///     tdopending(); taddchr('\n');
///     for (t0 = 0; t0 != tindent; t0++)
///         taddchr(text_expand_tabs ? ' '×N : '\t');
/// } else if (no_semicolon) taddstr(" ");
/// else taddstr("; ");
/// ```
pub fn taddnl(no_semicolon: i32) {
    let (newlins, indent, expand_tabs) = {
        let b = text_buffer_lock().lock().expect("text buffer poisoned");
        (b.newlins, b.indent, b.expand_tabs)
    };
    if newlins {
        tdopending();
        taddchr(b'\n' as i32);
        for _ in 0..indent {
            if expand_tabs >= 0 {
                if expand_tabs > 0 {
                    for _ in 0..expand_tabs {
                        taddchr(b' ' as i32);
                    }
                } else {
                    taddchr(b'\t' as i32);
                }
            }
        }
    } else if no_semicolon != 0 {
        taddstr(" ");
    } else {
        taddstr("; ");
    }
}

/// Port of `zoutputtab()` from `Src/text.c:263`.
///
/// C body emits a tab to `outf`, expanded to spaces when
/// `text_expand_tabs > 0`. Used by `getpermtext` consumers that
/// need to align their own output with the formatter's indent
/// rules.
///
/// Rust port writes to a writeable target (typically stdout)
/// matching the same expansion rules.
pub fn zoutputtab<W: std::io::Write>(outf: &mut W) -> std::io::Result<()> {
    let expand_tabs = text_buffer_lock()
        .lock()
        .expect("text buffer poisoned")
        .expand_tabs;
    if expand_tabs < 0 {
        return Ok(());
    }
    if expand_tabs > 0 {
        let spaces = vec![b' '; expand_tabs as usize];
        outf.write_all(&spaces)
    } else {
        outf.write_all(b"\t")
    }
}

/// Port of `tpush()` from `Src/text.c:396`.
///
/// C body pushes a `Tstack` frame for the recursive `gettext2`
/// walker — used when entering a nested wordcode region.
///
/// WARNING: zshrs's text formatter walks `ShellCommand` AST
/// recursively (Rust's stack), so the explicit Tstack isn't
/// kept. The fn is provided for C name parity; it just bumps
/// the indent counter via `dec_tindent`'s inverse.
pub fn tpush(increment: i32) {
    if increment != 0 {
        let mut b = text_buffer_lock().lock().expect("text buffer poisoned");
        b.indent = b.indent.saturating_add(1);
    }
}

/// Port of `gettext2()` from `Src/text.c:415`.
///
/// C body is the master AST→text walker for the wordcode
/// representation: dispatches on `WC_*` codes, calls all the
/// `taddX` helpers above, and pushes Tstack frames for nested
/// regions.
///
/// WARNING: zshrs uses `ShellCommand` AST + `TextFormatter`
/// rather than wordcode + Estate. The real walker is
/// [`getpermtext`] above, which builds the same output through
/// the typed AST. This entry preserves the C name; calling it
/// is equivalent to "the formatter has already run via
/// getpermtext" — so it's a finalise-buffer no-op.
pub fn gettext2() {}

/// Port of `getredirs()` from `Src/text.c:1019`.
///
/// C body emits each Redir node from the linked list using the
/// fixed `fstr[]` table for the operator string. Rust port takes
/// a slice of `Redirect` AST nodes and routes through the same
/// formatter the rest of text.rs uses.
///
/// Per the C source, the buffer is space-padded then the trailing
/// space is decremented (`tptr--`); same here via `pop()`.
pub fn getredirs(redirs: &[Redirect]) {
    taddchr(b' ' as i32);
    let formatter = TextFormatter::new(TextConfig::default());
    let snippet = formatter.format_redirects_only(redirs);
    taddstr(&snippet);
    let mut b = text_buffer_lock().lock().expect("text buffer poisoned");
    if b.buf.ends_with(' ') {
        b.buf.pop();
    }
}
