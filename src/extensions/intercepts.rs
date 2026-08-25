//! Command intercept / advice machinery — extension; no zsh C counterpart.
#[allow(unused_imports)]
use crate::ported::vm_helper::ShellExecutor;
#[allow(unused_imports)]
use std::collections::HashMap;

/// AOP advice type — before, after, or around.
#[derive(Debug, Clone)]
/// Aspect-oriented advice classification.
/// zshrs-original — no C zsh counterpart. C zsh's closest
/// analog is the function-wrapper hook in Src/module.c
/// (`addwrapper()`, used by `zsh/zprof`), but per-function
/// before/after/around AOP intercepts are unique to zshrs.
pub enum AdviceKind {
    /// Run code before the command executes.
    Before,
    /// Run code after the command executes. $? and INTERCEPT_MS available.
    After,
    /// Wrap the command. Code must call `intercept_proceed` to run original.
    Around,
}

/// An intercept registration.
#[derive(Debug, Clone)]
/// One AOP intercept registered against a function pattern.
/// zshrs-original — no C counterpart.
pub struct Intercept {
    /// Pattern to match command names. Supports glob: "git *", "_*", "*".
    pub pattern: String,
    /// What kind of advice.
    pub kind: AdviceKind,
    /// Shell code to execute as advice.
    pub code: String,
    /// Unique ID for removal.
    pub id: u32,
}

// ===========================================================
// Block-body syntax — `intercept <kind> <pat> { code }`
//
// zshrs-only; no C counterpart. `}` cannot be a bare argument in
// zsh (`echo }` is "parse error near `}'"), so the documented brace
// form is not reachable through ordinary word lexing. Worse, the
// body's own operators would be eaten by the OUTER command: in
// `intercept before git { echo hi >> ~/git.log }` the `>>` lexes as
// a redirection of `intercept` itself and never reaches argv, so no
// amount of re-joining the words downstream can rebuild the body.
//
// The lexer therefore captures the span between the braces as RAW
// SOURCE and hands it over as a single STRING token, which is what
// `builtin_intercept` wants anyway — it stores advice as text and
// runs it through `execute_advice`. The parser and the grammar are
// untouched: `intercept before git { … }` becomes an ordinary
// four-word simple command.
// ===========================================================

thread_local! {
    /// Raised when the command word just lexed was `intercept`, so the
    /// next `{` in ARGUMENT position opens an advice body rather than
    /// starting a brace expansion. Cleared by the next command word, and
    /// by the capture itself.
    static LEX_ININTERCEPT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Note a token the lexer produced at command position, arming or
/// disarming the block-body capture.
///
/// Assignment, not a one-way set: every command word re-decides, so an
/// `intercept` that never reached a `{` cannot leave the capture armed
/// for an unrelated later command.
///
/// `quoted` is true when the original token carried Snull/Dnull/Bnull
/// quote markers — `'intercept' before git { … }` is a quoted word and
/// gets no keyword treatment, the same rule the reserved-word table
/// follows for `"if"` / `"}"` (lex.rs:3953).
pub(crate) fn note_command_word(word: &str, quoted: bool) {
    LEX_ININTERCEPT.set(word == "intercept" && !quoted);
}

/// True when the `{` the lexer is looking at opens an advice body.
///
/// False under `zshrs --zsh`: that mode promises identical behaviour to
/// `/bin/zsh`, which rejects the construct outright, so the capture must
/// stand down and let the normal path produce zsh's own diagnostic.
pub(crate) fn wants_block() -> bool {
    LEX_ININTERCEPT.get() && !crate::dash_mode::zsh_dropin()
}

/// Clear the armed state. Called once a body is captured, and whenever
/// the lexer resets its context.
pub(crate) fn disarm() {
    LEX_ININTERCEPT.set(false);
}

/// Consume the raw source of an advice body, starting just past its
/// opening `{`, and return the text between the braces.
///
/// `getc` is the lexer's own character source, so the body is taken off
/// the real input stream — a body may span lines, and on the interactive
/// REPL the continuation prompt appears exactly as it does inside `{ }`
/// anywhere else.
///
/// Brace counting alone would be wrong: a `}` inside quotes, a comment, a
/// command substitution, or a here-document is not a terminator. Each of
/// those is tracked so the scan ends on the brace a reader would pick.
///
/// Returns `None` at end of input with the body still open. The consumed
/// characters are NOT pushed back — there is nothing left to push them
/// back in front of — and the caller falls through to the ordinary path,
/// where the unterminated construct raises the usual parse error.
pub(crate) fn scan_block_body<G>(mut getc: G) -> Option<String>
where
    G: FnMut() -> Option<char>,
{
    let mut body = String::new();
    let mut depth: u32 = 1;
    // Delimiters of here-documents opened on the line being scanned. A
    // `<<EOF` body is arbitrary text — its braces, quotes and `#` are all
    // literal — so it is skipped wholesale at the next newline.
    let mut pending_heredocs: Vec<(String, bool)> = Vec::new();
    // True while the scanner is between words, where `#` starts a comment.
    let mut at_word_start = true;

    loop {
        let c = getc()?;

        match c {
            '\\' => {
                // A backslash quotes the next character anywhere outside
                // single quotes, including a brace and a newline.
                body.push(c);
                body.push(getc()?);
                at_word_start = false;
                continue;
            }
            '\'' => {
                // Single quotes take everything literally — no escapes.
                body.push(c);
                loop {
                    let q = getc()?;
                    body.push(q);
                    if q == '\'' {
                        break;
                    }
                }
                at_word_start = false;
                continue;
            }
            '"' => {
                body.push(c);
                scan_double_quoted(&mut getc, &mut body)?;
                at_word_start = false;
                continue;
            }
            '`' => {
                body.push(c);
                loop {
                    let q = getc()?;
                    body.push(q);
                    match q {
                        '\\' => body.push(getc()?),
                        '`' => break,
                        _ => {}
                    }
                }
                at_word_start = false;
                continue;
            }
            '#' if at_word_start => {
                // Comment to end of line. The newline itself is left for
                // the heredoc check below.
                body.push(c);
                loop {
                    match getc() {
                        Some('\n') => {
                            body.push('\n');
                            break;
                        }
                        Some(ch) => body.push(ch),
                        None => return None,
                    }
                }
                at_word_start = true;
                if !pending_heredocs.is_empty() {
                    drain_heredocs(&mut getc, &mut body, &mut pending_heredocs)?;
                }
                continue;
            }
            '<' => {
                body.push(c);
                let (delim, stopped_at) = scan_heredoc_intro(&mut getc, &mut body)?;
                if let Some(d) = delim {
                    pending_heredocs.push(d);
                }
                at_word_start = false;
                // Reading the delimiter word necessarily consumed the
                // character that ended it, and for `cat <<EOF` that
                // character IS the newline the here-document body starts
                // after. The main loop will never see it, so the drain has
                // to happen here or the body's text gets scanned as shell.
                if stopped_at == Some('\n') {
                    at_word_start = true;
                    if !pending_heredocs.is_empty() {
                        drain_heredocs(&mut getc, &mut body, &mut pending_heredocs)?;
                    }
                }
                continue;
            }
            '\n' => {
                body.push(c);
                at_word_start = true;
                if !pending_heredocs.is_empty() {
                    drain_heredocs(&mut getc, &mut body, &mut pending_heredocs)?;
                }
                continue;
            }
            '{' => {
                depth += 1;
                body.push(c);
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(body);
                }
                body.push(c);
            }
            _ => body.push(c),
        }
        at_word_start = c.is_whitespace() || matches!(c, ';' | '|' | '&' | '(' | ')');
    }
}

/// Copy a double-quoted string, stopping after its closing quote.
///
/// Backslash still escapes inside double quotes, and `$( … )` may contain
/// arbitrary shell — including more quotes and braces — so the nested
/// command substitution is copied through by paren depth rather than
/// scanned for a terminator.
fn scan_double_quoted<G>(getc: &mut G, body: &mut String) -> Option<()>
where
    G: FnMut() -> Option<char>,
{
    loop {
        let c = getc()?;
        body.push(c);
        match c {
            '\\' => body.push(getc()?),
            '"' => return Some(()),
            '$' => {
                let n = getc()?;
                body.push(n);
                if n == '(' {
                    let mut depth = 1;
                    while depth > 0 {
                        let q = getc()?;
                        body.push(q);
                        match q {
                            '\\' => body.push(getc()?),
                            '(' => depth += 1,
                            ')' => depth -= 1,
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

/// After a `<`, decide whether this is a here-document and if so return
/// its delimiter plus whether `<<-` tab-stripping is in effect.
///
/// `<<<` is a here-STRING: an ordinary word follows, with no body to skip.
#[allow(clippy::type_complexity)]
fn scan_heredoc_intro<G>(
    getc: &mut G,
    body: &mut String,
) -> Option<(Option<(String, bool)>, Option<char>)>
where
    G: FnMut() -> Option<char>,
{
    let second = getc()?;
    body.push(second);
    if second != '<' {
        return Some((None, Some(second)));
    }
    let mut third = getc()?;
    body.push(third);
    if third == '<' {
        // here-string
        return Some((None, Some(third)));
    }
    let strip_tabs = third == '-';
    if strip_tabs {
        third = getc()?;
        body.push(third);
    }
    // Skip blanks between the operator and the delimiter word.
    let mut c = third;
    while c == ' ' || c == '\t' {
        c = getc()?;
        body.push(c);
    }
    // The delimiter may be quoted; the quotes are not part of it.
    let mut delim = String::new();
    // The character the delimiter word ended on, handed back so the caller
    // knows whether the here-document body starts immediately.
    let mut stopped_at: Option<char> = None;
    loop {
        match c {
            '\'' | '"' => {
                let close = c;
                loop {
                    let q = getc()?;
                    body.push(q);
                    if q == close {
                        break;
                    }
                    delim.push(q);
                }
            }
            '\\' => {
                let q = getc()?;
                body.push(q);
                delim.push(q);
            }
            _ if c.is_whitespace() || c == ';' || c == '&' || c == '|' || c == ')' => {
                stopped_at = Some(c);
                break;
            }
            _ => delim.push(c),
        }
        match getc() {
            Some(n) => {
                c = n;
                body.push(n);
            }
            None => break,
        }
    }
    if delim.is_empty() {
        Some((None, stopped_at))
    } else {
        Some((Some((delim, strip_tabs)), stopped_at))
    }
}

/// Copy here-document bodies verbatim, one per pending delimiter.
///
/// Everything up to the terminator line is literal text: braces, quotes
/// and `#` inside it must not steer the scan.
fn drain_heredocs<G>(
    getc: &mut G,
    body: &mut String,
    pending: &mut Vec<(String, bool)>,
) -> Option<()>
where
    G: FnMut() -> Option<char>,
{
    for (delim, strip_tabs) in pending.drain(..) {
        loop {
            let mut line = String::new();
            let mut hit_eof = true;
            while let Some(c) = getc() {
                if c == '\n' {
                    hit_eof = false;
                    break;
                }
                line.push(c);
            }
            body.push_str(&line);
            if !hit_eof {
                body.push('\n');
            }
            let candidate = if strip_tabs {
                line.trim_start_matches('\t')
            } else {
                line.as_str()
            };
            if candidate == delim {
                break;
            }
            if hit_eof {
                // zsh warns and treats EOF as the terminator; the body ends
                // here either way.
                return Some(());
            }
        }
    }
    Some(())
}

/// Match an intercept pattern against a command name or full command string.
/// Supports: exact match, glob ("git *", "_*", "*"), or "all".
pub(crate) fn intercept_matches(pattern: &str, cmd_name: &str, full_cmd: &str) -> bool {
    if pattern == "*" || pattern == "all" {
        return true;
    }
    if pattern == cmd_name {
        return true;
    }
    if pattern.contains('*') || pattern.contains('?') {
        if let Ok(pat) = glob::Pattern::new(pattern) {
            return pat.matches(cmd_name) || pat.matches(full_cmd);
        }
    }
    false
}

// ===========================================================
// Methods moved verbatim from src/ported/vm_helper because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: drift
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::vm_helper::ShellExecutor {
    /// Check intercepts for a command. Returns Some(result) if an around
    /// advice fully handled the command, None to proceed normally.
    pub(crate) fn run_intercepts(
        &mut self,
        cmd_name: &str,
        full_cmd: &str,
        args: &[String],
    ) -> Option<Result<i32, String>> {
        // Collect matching intercepts (clone to avoid borrow issues)
        let matching: Vec<Intercept> = self
            .intercepts
            .iter()
            .filter(|i| intercept_matches(&i.pattern, cmd_name, full_cmd))
            .cloned()
            .collect();

        if matching.is_empty() {
            return None;
        }

        // Set INTERCEPT_NAME and INTERCEPT_ARGS for advice code
        self.set_scalar("INTERCEPT_NAME".to_string(), cmd_name.to_string());
        self.set_scalar("INTERCEPT_ARGS".to_string(), args.join(" "));
        self.set_scalar("INTERCEPT_CMD".to_string(), full_cmd.to_string());

        // Run before advice
        for advice in matching
            .iter()
            .filter(|i| matches!(i.kind, AdviceKind::Before))
        {
            let _ = self.execute_advice(&advice.code);
        }

        // Check for around advice — first match wins
        let around = matching
            .iter()
            .find(|i| matches!(i.kind, AdviceKind::Around));

        let t0 = std::time::Instant::now();

        let result = if let Some(advice) = around {
            // Around advice: set INTERCEPT_PROCEED flag, run advice code.
            // If advice calls `intercept_proceed`, the original command runs.
            self.set_scalar("__intercept_proceed".to_string(), "0".to_string());
            let advice_result = self.execute_advice(&advice.code);

            // Check if intercept_proceed was called
            let proceeded = self
                .scalar("__intercept_proceed")
                .map(|v| v == "1")
                .unwrap_or(false);

            if proceeded {
                // The original command was already executed inside the advice
                advice_result
            } else {
                // Advice didn't call proceed — command was suppressed
                advice_result
            }
        } else {
            // No around advice — run the original command.
            // We return None to let the normal dispatch continue.
            // But we still need after advice to fire, so we can't return None here
            // if there are after advices. Run the command ourselves.
            let has_after = matching.iter().any(|i| matches!(i.kind, AdviceKind::After));
            if !has_after {
                // Only before advice, no after — let normal dispatch continue
                return None;
            }

            // Has after advice — we must run the command and then run after advice
            self.run_original_command(cmd_name, args)
        };

        let elapsed = t0.elapsed();

        // Set timing variable for after advice
        let ms = elapsed.as_secs_f64() * 1000.0;
        self.set_scalar("INTERCEPT_MS".to_string(), format!("{:.3}", ms));
        self.set_scalar("INTERCEPT_US".to_string(), format!("{:.0}", ms * 1000.0));
        // The intercepted command's exit status, for `after` advice that
        // wants to branch on success. `$?` alone is not enough: the advice
        // body's own first command overwrites it, and a `before` advice that
        // ran earlier has already moved it. An advice that failed to run at
        // all reports 1, matching what the shell would have left behind.
        self.set_scalar(
            "INTERCEPT_STATUS".to_string(),
            match &result {
                Ok(st) => st.to_string(),
                Err(_) => "1".to_string(),
            },
        );

        // Run after advice
        for advice in matching
            .iter()
            .filter(|i| matches!(i.kind, AdviceKind::After))
        {
            let _ = self.execute_advice(&advice.code);
        }

        // Clean up
        self.unset_scalar("INTERCEPT_NAME");
        self.unset_scalar("INTERCEPT_ARGS");
        self.unset_scalar("INTERCEPT_CMD");
        self.unset_scalar("INTERCEPT_MS");
        self.unset_scalar("INTERCEPT_US");
        self.unset_scalar("INTERCEPT_STATUS");
        self.unset_scalar("__intercept_proceed");

        Some(result)
    }
    /// Execute the original command (used by around/after intercept dispatch).
    /// Execute advice code — dispatches @ prefix to stryke (fat binary),
    /// everything else to the shell parser. No fork. Machine code speed.
    pub(crate) fn execute_advice(&mut self, code: &str) -> Result<i32, String> {
        let code = code.trim();
        if code.starts_with('@') {
            let stryke_code = code.trim_start_matches('@').trim();
            if let Some(status) = crate::try_stryke_dispatch(stryke_code) {
                self.set_last_status(status);
                return Ok(status);
            }
            // No stryke handler (thin binary) — fall through to shell
        }
        self.execute_script(code)
    }
    pub(crate) fn run_original_command(
        &mut self,
        cmd_name: &str,
        args: &[String],
    ) -> Result<i32, String> {
        // Function dispatch via the compiled pipeline (functions_compiled
        // first, falls back to legacy AST recompile if needed).
        if let Some(status) = self.dispatch_function_call(cmd_name, args) {
            return Ok(status);
        }
        // External command
        self.execute_external(cmd_name, args, &[])
    }
}
// END moved-from-exec-rs

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn star_matches_anything() {
        assert!(intercept_matches("*", "anything", "anything --here"));
        assert!(intercept_matches("*", "", ""));
    }

    #[test]
    fn all_matches_anything() {
        assert!(intercept_matches("all", "ls", "ls -la"));
        assert!(intercept_matches("all", "git", "git status"));
    }

    #[test]
    fn exact_match_on_cmd_name() {
        assert!(intercept_matches("git", "git", "git push"));
        assert!(intercept_matches("ls", "ls", "ls -la"));
    }

    #[test]
    fn exact_pattern_does_not_match_different_name() {
        assert!(!intercept_matches("git", "svn", "svn diff"));
        assert!(!intercept_matches("ls", "lsof", "lsof -p 1"));
    }

    #[test]
    fn glob_star_matches_prefix() {
        // "git *" should match the full command line like "git push origin".
        assert!(intercept_matches("git *", "git", "git push origin"));
    }

    #[test]
    fn glob_star_underscore_prefix_matches_completion_funcs() {
        // "_*" is the canonical zsh pattern for completion functions.
        assert!(intercept_matches("_*", "_files", "_files"));
        assert!(intercept_matches("_*", "_describe", "_describe"));
    }

    #[test]
    fn glob_star_does_not_match_non_prefix() {
        assert!(!intercept_matches("_*", "files", "files"));
    }

    #[test]
    fn question_mark_glob_matches_single_char() {
        assert!(intercept_matches("l?", "ls", "ls"));
        assert!(!intercept_matches("l?", "lsof", "lsof"));
    }

    #[test]
    fn unmatched_pattern_without_glob_chars_returns_false() {
        assert!(!intercept_matches("nope", "git", "git push"));
    }

    #[test]
    fn invalid_glob_pattern_returns_false() {
        // `[` with no closing bracket is invalid; should not panic and not match.
        // Pattern with `[` triggers neither the `*` shortcut nor exact match,
        // but it also contains no `*` or `?`, so we never reach glob parsing.
        assert!(!intercept_matches("[invalid", "git", "git push"));
    }

    #[test]
    fn empty_pattern_does_not_match_non_empty_cmd() {
        assert!(!intercept_matches("", "ls", "ls -la"));
    }

    #[test]
    fn empty_pattern_matches_empty_cmd_exactly() {
        // Falls through to the `pattern == cmd_name` check.
        assert!(intercept_matches("", "", ""));
    }

    #[test]
    fn advice_kind_variants_round_trip_clone() {
        let b = AdviceKind::Before;
        let a = AdviceKind::After;
        let r = AdviceKind::Around;
        assert!(matches!(b.clone(), AdviceKind::Before));
        assert!(matches!(a.clone(), AdviceKind::After));
        assert!(matches!(r.clone(), AdviceKind::Around));
    }

    #[test]
    fn intercept_struct_clone_preserves_fields() {
        let i = Intercept {
            pattern: "git *".into(),
            kind: AdviceKind::Before,
            code: "echo before".into(),
            id: 42,
        };
        let c = i.clone();
        assert_eq!(c.pattern, "git *");
        assert!(matches!(c.kind, AdviceKind::Before));
        assert_eq!(c.code, "echo before");
        assert_eq!(c.id, 42);
    }
}
