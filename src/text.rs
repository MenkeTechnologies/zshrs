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

use crate::parser::{
    CaseTerminator, CompoundCommand, CondExpr, ListOp, Redirect, RedirectOp, ShellCommand,
    ShellWord, SimpleCommand,
};

/// Binary operators in conditions (order matches COND_STREQ et seq.)
pub static COND_BINARY_OPS: &[&str] = &[
    "=", "==", "!=", "<", ">", "-nt", "-ot", "-ef", "-eq", "-ne", "-lt", "-gt", "-le", "-ge", "=~",
];

/// Check if a string is a condition binary operator
pub fn is_cond_binary_op(s: &str) -> bool {
    COND_BINARY_OPS.contains(&s)
}

/// Text formatter configuration
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

/// Text formatter for shell commands
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
            ShellWord::SingleQuoted(s) => {
                self.add_char('\'');
                self.add_str(s);
                self.add_char('\'');
            }
            ShellWord::DoubleQuoted(parts) => {
                self.add_char('"');
                for part in parts {
                    self.format_word(part);
                }
                self.add_char('"');
            }
            ShellWord::Variable(name) => {
                self.add_char('$');
                self.add_str(name);
            }
            ShellWord::VariableBraced(name, modifier) => {
                self.add_str("${");
                self.add_str(name);
                if modifier.is_some() {
                    self.add_str("..."); // Simplified
                }
                self.add_char('}');
            }
            ShellWord::ArrayVar(name, _idx) => {
                self.add_str("${");
                self.add_str(name);
                self.add_str("[...]}");
            }
            ShellWord::CommandSub(cmd) => {
                self.add_str("$(");
                self.format_command(cmd);
                self.add_char(')');
            }
            ShellWord::ProcessSubIn(cmd) => {
                self.add_str("<(");
                self.format_command(cmd);
                self.add_char(')');
            }
            ShellWord::ProcessSubOut(cmd) => {
                self.add_str(">(");
                self.format_command(cmd);
                self.add_char(')');
            }
            ShellWord::ArithSub(expr) => {
                self.add_str("$((");
                self.add_str(expr);
                self.add_str("))");
            }
            ShellWord::ArrayLiteral(words) => {
                self.add_char('(');
                for (i, w) in words.iter().enumerate() {
                    if i > 0 {
                        self.add_char(' ');
                    }
                    self.format_word(w);
                }
                self.add_char(')');
            }
            ShellWord::Glob(pattern) => self.add_str(pattern),
            ShellWord::Tilde(user) => {
                self.add_char('~');
                if let Some(u) = user {
                    self.add_str(u);
                }
            }
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
            if i == list.len() - 1 {
                match op {
                    ListOp::Amp => self.add_str(" &"),
                    _ => {}
                }
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
            CompoundCommand::Cond(expr) => {
                self.add_str("[[ ");
                self.format_cond_expr(expr);
                self.add_str(" ]]");
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

    fn format_cond_expr(&mut self, expr: &CondExpr) {
        match expr {
            CondExpr::Not(inner) => {
                self.add_str("! ");
                self.format_cond_expr(inner);
            }
            CondExpr::And(left, right) => {
                self.format_cond_expr(left);
                self.add_str(" && ");
                self.format_cond_expr(right);
            }
            CondExpr::Or(left, right) => {
                self.format_cond_expr(left);
                self.add_str(" || ");
                self.format_cond_expr(right);
            }
            // File tests
            CondExpr::FileExists(w) => {
                self.add_str("-e ");
                self.format_word(w);
            }
            CondExpr::FileRegular(w) => {
                self.add_str("-f ");
                self.format_word(w);
            }
            CondExpr::FileDirectory(w) => {
                self.add_str("-d ");
                self.format_word(w);
            }
            CondExpr::FileSymlink(w) => {
                self.add_str("-L ");
                self.format_word(w);
            }
            CondExpr::FileReadable(w) => {
                self.add_str("-r ");
                self.format_word(w);
            }
            CondExpr::FileWritable(w) => {
                self.add_str("-w ");
                self.format_word(w);
            }
            CondExpr::FileExecutable(w) => {
                self.add_str("-x ");
                self.format_word(w);
            }
            CondExpr::FileNonEmpty(w) => {
                self.add_str("-s ");
                self.format_word(w);
            }
            // String tests
            CondExpr::StringEmpty(w) => {
                self.add_str("-z ");
                self.format_word(w);
            }
            CondExpr::StringNonEmpty(w) => {
                self.add_str("-n ");
                self.format_word(w);
            }
            CondExpr::StringEqual(l, r) => {
                self.format_word(l);
                self.add_str(" == ");
                self.format_word(r);
            }
            CondExpr::StringNotEqual(l, r) => {
                self.format_word(l);
                self.add_str(" != ");
                self.format_word(r);
            }
            CondExpr::StringMatch(l, r) => {
                self.format_word(l);
                self.add_str(" =~ ");
                self.format_word(r);
            }
            CondExpr::StringLess(l, r) => {
                self.format_word(l);
                self.add_str(" < ");
                self.format_word(r);
            }
            CondExpr::StringGreater(l, r) => {
                self.format_word(l);
                self.add_str(" > ");
                self.format_word(r);
            }
            // Numeric tests
            CondExpr::NumEqual(l, r) => {
                self.format_word(l);
                self.add_str(" -eq ");
                self.format_word(r);
            }
            CondExpr::NumNotEqual(l, r) => {
                self.format_word(l);
                self.add_str(" -ne ");
                self.format_word(r);
            }
            CondExpr::NumLess(l, r) => {
                self.format_word(l);
                self.add_str(" -lt ");
                self.format_word(r);
            }
            CondExpr::NumLessEqual(l, r) => {
                self.format_word(l);
                self.add_str(" -le ");
                self.format_word(r);
            }
            CondExpr::NumGreater(l, r) => {
                self.format_word(l);
                self.add_str(" -gt ");
                self.format_word(r);
            }
            CondExpr::NumGreaterEqual(l, r) => {
                self.format_word(l);
                self.add_str(" -ge ");
                self.format_word(r);
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

/// Get a permanent textual representation of a command
pub fn getpermtext(cmd: &ShellCommand) -> String {
    TextFormatter::new(TextConfig::default()).format(cmd)
}

/// Get a permanent textual representation with custom indent
pub fn getpermtext_indent(cmd: &ShellCommand, indent: usize) -> String {
    TextFormatter::new(TextConfig::default())
        .with_indent(indent)
        .format(cmd)
}

/// Get a representation suitable for job text (abbreviated, single line)
pub fn getjobtext(cmd: &ShellCommand) -> String {
    TextFormatter::new(TextConfig::job_text()).format(cmd)
}

/// Get a single-line representation
pub fn getsingleline(cmd: &ShellCommand) -> String {
    TextFormatter::new(TextConfig::single_line()).format(cmd)
}

/// Format a list of commands
pub fn format_commands(cmds: &[ShellCommand], config: TextConfig) -> String {
    TextFormatter::new(config).format_list(cmds)
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
        let text = getsingleline(&cmd);
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
}
