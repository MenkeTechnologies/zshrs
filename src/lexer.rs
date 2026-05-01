//! Zsh lexical analyzer - Direct port from zsh/Src/lex.c
//!
//! This lexer tokenizes zsh shell input into a stream of tokens.
//! It handles all zsh-specific syntax including:
//! - Single/double/dollar quotes
//! - Command substitution $(...)  and `...`
//! - Arithmetic $((...))
//! - Parameter expansion ${...}
//! - Process substitution <(...) >(...)
//! - Here documents
//! - All redirection operators
//! - Comments
//! - Continuation lines

use crate::tokens::{char_tokens, LexTok};
use std::collections::VecDeque;

/// Lexer flags controlling behavior
#[derive(Debug, Clone, Copy, Default)]
pub struct LexFlags {
    /// Parsing for ZLE (line editor) completion
    pub zle: bool,
    /// Return newlines as tokens
    pub newline: bool,
    /// Preserve comments in output
    pub comments_keep: bool,
    /// Strip comments from output
    pub comments_strip: bool,
    /// Active lexing (from bufferwords)
    pub active: bool,
}

/// Buffer state for building tokens
#[derive(Debug, Clone)]
struct LexBuf {
    data: String,
    siz: usize,
}

impl LexBuf {
    fn new() -> Self {
        LexBuf {
            data: String::with_capacity(256),
            siz: 256,
        }
    }

    fn clear(&mut self) {
        self.data.clear();
    }

    fn add(&mut self, c: char) {
        self.data.push(c);
        if self.data.len() >= self.siz {
            self.siz *= 2;
            self.data.reserve(self.siz - self.data.len());
        }
    }

    #[allow(dead_code)]
    fn add_str(&mut self, s: &str) {
        self.data.push_str(s);
    }

    fn len(&self) -> usize {
        self.data.len()
    }

    fn as_str(&self) -> &str {
        &self.data
    }

    #[allow(dead_code)]
    fn into_string(self) -> String {
        self.data
    }

    #[allow(dead_code)]
    fn last_char(&self) -> Option<char> {
        self.data.chars().last()
    }

    fn pop(&mut self) -> Option<char> {
        self.data.pop()
    }
}

/// Here-document state
#[derive(Debug, Clone)]
pub struct HereDoc {
    pub terminator: String,
    pub strip_tabs: bool,
    pub content: String,
    /// True if the terminator was originally quoted (`<<'EOF'`,
    /// `<<"EOF"`, or `<<\EOF`). Disables variable expansion / command
    /// substitution / arithmetic in the body.
    pub quoted: bool,
    /// True once `process_heredocs` has read the body. Distinct from
    /// "content is empty" because an empty heredoc legitimately has
    /// empty content.
    pub processed: bool,
}

/// The Zsh Lexer
pub struct ZshLexer<'a> {
    /// Input source
    pub(crate) input: &'a str,
    /// Current position in input
    pub(crate) pos: usize,
    /// Look-ahead buffer for ungotten characters
    unget_buf: VecDeque<char>,
    /// Current token string
    pub tokstr: Option<String>,
    /// Current token type
    pub tok: LexTok,
    /// File descriptor for redirections (e.g., 2> means fd=2)
    pub tokfd: i32,
    /// Line number at start of current token
    pub toklineno: u64,
    /// Current line number
    pub lineno: u64,
    /// Lexer has stopped (EOF or error)
    pub lexstop: bool,
    /// In command position (can accept reserved words)
    pub incmdpos: bool,
    /// In condition [[ ... ]]
    pub incond: i32,
    /// In pattern context (RHS of == != =~ in [[ ]])
    pub incondpat: bool,
    /// In case pattern
    pub incasepat: i32,
    /// In redirection
    pub inredir: bool,
    /// After 'for' keyword
    pub infor: i32,
    /// After 'repeat' keyword
    inrepeat: i32,
    /// Parsing typeset arguments
    pub intypeset: bool,
    /// Inside (( ... )) arithmetic
    dbparens: bool,
    /// Disable alias expansion
    pub noaliases: bool,
    /// Disable spelling correction
    pub nocorrect: i32,
    /// Disable comment recognition
    pub nocomments: bool,
    /// Lexer flags
    pub lexflags: LexFlags,
    /// Whether this is the first line
    pub isfirstln: bool,
    /// Whether this is the first char of command
    #[allow(dead_code)]
    isfirstch: bool,
    /// Pending here-documents
    pub heredocs: Vec<HereDoc>,
    /// Expecting heredoc terminator (0 = no, 1 = <<, 2 = <<-)
    heredoc_pending: u8,
    /// Token buffer
    lexbuf: LexBuf,
    /// After newline
    pub isnewlin: i32,
    /// Error message if any
    pub error: Option<String>,
    /// Global iteration counter for infinite loop detection
    global_iterations: usize,
    /// Recursion depth counter
    recursion_depth: usize,
    /// Raw-input capture flag — when nonzero, every char read through
    /// `hgetc` is also appended to `tokstr_raw` via zshlex_raw_add.
    /// Direct mirror of zsh/Src/lex.c:161 `lex_add_raw`. Used by
    /// skipcomm (lex.c:2082) to preserve the literal text of `$(...)`
    /// command substitutions for re-execution / display.
    pub lex_add_raw: i32,
    /// Raw-input capture buffer. Direct mirror of lex.c:165
    /// `tokstr_raw` / lex.c:166 `lexbuf_raw`. Combined into one
    /// `LexBuf` here since Rust's String tracks both the data and
    /// length internally.
    lexbuf_raw: LexBuf,
}

const MAX_LEXER_RECURSION: usize = 200;

/// Per-alias info returned by `AliasResolver::lookup_alias` and
/// `lookup_suffix_alias`. Mirrors zsh's `struct alias` fields used
/// at lex.c:1914-1943: `text` (replacement body), `in_use` (the
/// recursion-guard flag), `global` (vs command-position-only).
#[derive(Debug, Clone)]
pub struct AliasInfo {
    pub text: String,
    pub in_use: bool,
    pub global: bool,
}

/// Trait the lexer uses to look up aliases and reserved words during
/// `exalias`. Implementors typically delegate to the executor's
/// alias/reswd hash tables. Defining the trait here keeps lexer.rs
/// free of executor-specific types — same pattern zsh uses with the
/// hashtable.h opaque-handle approach against aliastab/reswdtab/
/// sufaliastab.
pub trait AliasResolver {
    /// Look up an alias by name. Returns `None` if not found, or the
    /// alias body + flags otherwise.
    fn lookup_alias(&self, name: &str) -> Option<AliasInfo>;
    /// Look up a suffix alias (e.g. `.txt → less`) by suffix only.
    fn lookup_suffix_alias(&self, suffix: &str) -> Option<AliasInfo>;
    /// Resolve a reserved word. Returns the LexTok the word should
    /// promote to (e.g. "if" → IF), or None if not a reswd.
    fn lookup_reswd(&self, name: &str) -> Option<LexTok>;
    /// Mark an alias as in-use (recursion guard). Called when an
    /// alias is about to be expanded; the matching unmark happens
    /// when the alias text has been fully consumed by the lexer.
    fn mark_in_use(&mut self, name: &str, in_use: bool);
}

/// Saved lexical state for nested-context handling. Direct port of
/// `struct lex_stack` declared in zsh/Src/zsh.h and used by
/// zsh/Src/lex.c:215-239 (`lex_context_save`) and lex.c:244-262
/// (`lex_context_restore`). Used when entering command substitution,
/// here-docs, or eval where the outer lexer state must be pushed and
/// restored after the inner parse completes.
#[derive(Debug, Clone)]
pub struct LexStack {
    pub dbparens: bool,
    pub isfirstln: bool,
    pub isfirstch: bool,
    pub lexflags: LexFlags,
    pub tok: LexTok,
    pub tokstr: Option<String>,
    pub lexbuf_data: String,
    pub lexbuf_siz: usize,
    pub lexstop: bool,
    pub toklineno: u64,
}

impl Default for LexStack {
    fn default() -> Self {
        // Mirrors lex.c:235-238 reset state after a save: tokstr / lexbuf
        // zeroed, lexbuf.siz back to the initial 256 alloc, tok to
        // ENDINPUT (the C source doesn't explicitly reset tok here but
        // the natural baseline is ENDINPUT — same as lexinit).
        LexStack {
            dbparens: false,
            isfirstln: false,
            isfirstch: false,
            lexflags: LexFlags::default(),
            tok: LexTok::Endinput,
            tokstr: None,
            lexbuf_data: String::new(),
            lexbuf_siz: 256,
            lexstop: false,
            toklineno: 0,
        }
    }
}

impl<'a> ZshLexer<'a> {
    /// Create a new lexer for the given input
    pub fn new(input: &'a str) -> Self {
        ZshLexer {
            input,
            pos: 0,
            unget_buf: VecDeque::new(),
            tokstr: None,
            tok: LexTok::Endinput,
            tokfd: -1,
            toklineno: 1,
            lineno: 1,
            lexstop: false,
            incmdpos: true,
            incond: 0,
            incondpat: false,
            incasepat: 0,
            inredir: false,
            infor: 0,
            inrepeat: 0,
            intypeset: false,
            dbparens: false,
            noaliases: false,
            nocorrect: 0,
            nocomments: false,
            lexflags: LexFlags::default(),
            isfirstln: true,
            isfirstch: true,
            heredocs: Vec::new(),
            heredoc_pending: 0,
            lexbuf: LexBuf::new(),
            isnewlin: 0,
            error: None,
            global_iterations: 0,
            recursion_depth: 0,
            lex_add_raw: 0,
            lexbuf_raw: LexBuf::new(),
        }
    }

    /// Append a char to the raw-input capture buffer. Direct port of
    /// zsh/Src/lex.c:2024-2039 `zshlex_raw_add`. Called from hgetc
    /// when `lex_add_raw` is nonzero so cmd-sub bodies (`$(...)`,
    /// `<(...)`, `>(...)`) can be replayed verbatim without re-lexing.
    pub fn zshlex_raw_add(&mut self, c: char) {
        // lex.c:2027-2028 — guard on lex_add_raw flag.
        if self.lex_add_raw == 0 {
            return;
        }
        // lex.c:2030-2038 — append to lexbuf_raw. The C source manages
        // explicit ptr/len/siz with hrealloc; Rust's String handles
        // resize automatically.
        self.lexbuf_raw.add(c);
    }

    /// Run alias / reserved-word expansion on the just-lexed token.
    /// Direct port of zsh/Src/lex.c:1949-2021 `exalias`. Returns true
    /// if an alias was injected (the caller's loop should re-run
    /// gettok to consume the injected text).
    ///
    /// C source flow:
    ///   1. Spell-correct (lex.c:1958-1962) — disabled in zshrs.
    ///   2. If tokstr is None: set lextext from tokstrings[tok] and
    ///      checkalias against that (lex.c:1964-1969).
    ///   3. Otherwise: untokenize tokstr into a working copy (lex.c:
    ///      1971-1980).
    ///   4. ZLE word-tracking: call gotword() if LEXFLAGS_ZLE
    ///      (lex.c:1982-1991).
    ///   5. STRING tokens: try checkalias, then reservation lookup
    ///      (lex.c:1993-2015).
    ///   6. Clear inalmore (lex.c:2016).
    ///
    /// Takes an `AliasResolver` trait object so the lexer doesn't
    /// hard-depend on the executor's alias-table types. zshrs callers
    /// implement `AliasResolver` over their alias hash tables.
    pub fn exalias<R: AliasResolver>(&mut self, resolver: &mut R) -> bool {
        // lex.c:1957 — `hwend()` ends the history-word region. zshrs's
        // history layer doesn't track per-word boundaries here; no-op.

        // lex.c:1958-1962 — spell correction via spckword. zshrs
        // doesn't implement spell correction yet; documented divergence.

        // lex.c:1964-1969 — bare-token path (no tokstr).
        if self.tokstr.is_none() {
            // lex.c:1965 — `zshlextext = tokstrings[tok];` — for tokens
            // like SEMI/AMPER/etc. the canonical text comes from a
            // static table. zshrs's check_alias_for_text uses the
            // resolver directly with the token's text representation.
            if self.tok == LexTok::Newlin {
                return false;
            }
            // Use punctuation-token text; unknown tokens skip alias.
            let text = match self.tok {
                LexTok::Semi => ";",
                LexTok::Amper => "&",
                LexTok::Bar => "|",
                _ => return false,
            };
            return self.check_alias(resolver, text);
        }

        let tokstr = self.tokstr.clone().unwrap();
        // lex.c:1973-1980 — untokenize: convert the lexer's internal
        // tokenized form (Pound..ztokens shifts) into the literal
        // shell text. Call the global helper.
        let lextext = if has_token(&tokstr) {
            untokenize(&tokstr)
        } else {
            tokstr.clone()
        };

        // lex.c:1982-1991 — ZLE word-tracking for completion.
        if self.lexflags.zle {
            let zp = self.lexflags;
            self.gotword();
            // lex.c:1986-1990 — if gotword cleared lexflags, the cursor
            // word has been reached; abort exalias so completion can
            // capture the partial token unchanged.
            if zp.zle && !self.lexflags.zle {
                return false;
            }
        }

        // lex.c:1993-2015 — STRING-token alias / reswd check.
        if self.tok == LexTok::String {
            // lex.c:1995 — `checkalias()`. POSIX-aliases gate skipped
            // here (zshrs doesn't have the option flag wired).
            if self.check_alias(resolver, &lextext) {
                return true;
            }

            // lex.c:2002-2009 — reserved-word lookup. Fires when in
            // command position OR when the text is bare `}` and
            // IGNOREBRACES is unset (so `}` ends a brace block).
            if self.incmdpos || lextext == "}" {
                if let Some(rwtok) = resolver.lookup_reswd(&lextext) {
                    self.tok = rwtok;
                    if rwtok == LexTok::Repeat {
                        self.inrepeat = 1;
                    }
                    if rwtok == LexTok::Dinbrack {
                        self.incond = 1;
                    }
                }
            } else if self.incond > 0 && lextext == "]]" {
                // lex.c:2010-2012 — `]]` closes the cond expression.
                self.tok = LexTok::Doutbrack;
                self.incond = 0;
            } else if self.incond == 1 && lextext == "!" {
                // lex.c:2013-2014 — `!` inside `[[ ]]` is the BANG
                // negation, not a literal.
                self.tok = LexTok::Bang;
            }
        }

        // lex.c:2016 — `inalmore = 0;` — alias-more flag clears after
        // any non-alias token.
        // (zshrs's lexer doesn't have inalmore yet — added here would
        // require gettok to track when an alias-pushed token has more
        // text after it. Documented divergence.)

        false
    }

    /// Helper for `exalias`. Direct port of zsh/Src/lex.c:1899-1947
    /// `checkalias`. Returns true if the lookup matched (regular or
    /// suffix alias) AND the alias text was successfully injected
    /// back into the input stream for re-lexing.
    fn check_alias<R: AliasResolver>(&mut self, resolver: &mut R, lextext: &str) -> bool {
        // lex.c:1906-1907 — guard on null lextext.
        if lextext.is_empty() {
            return false;
        }

        // lex.c:1909-1911 — guard: alias expansion is disabled, or
        // POSIX aliases require the token to be a STRING and not a
        // reserved word.
        if self.noaliases {
            return false;
        }

        // lex.c:1914-1933 — regular alias lookup.
        if let Some(alias) = resolver.lookup_alias(lextext) {
            if !alias.in_use && (alias.global || (self.incmdpos && self.tok == LexTok::String)) {
                // lex.c:1918-1927 — if the next char isn't blank,
                // insert a space so the alias body can't accidentally
                // join the following word.
                if !self.lexstop {
                    if let Some(c) = self.peek() {
                        if !Self::is_blank(c) {
                            self.inject_alias_text(" ");
                        }
                    }
                }
                // lex.c:1928 — `inpush(an->text, INP_ALIAS, an);`
                self.inject_alias_text(&alias.text);
                resolver.mark_in_use(lextext, true);
                self.lexstop = false;
                return true;
            }
        }

        // lex.c:1934-1943 — suffix-alias lookup. The token must end
        // with `.SUFFIX`, the suffix name must be a registered
        // suffix-alias, AND the lexer must be in command position.
        if self.incmdpos {
            if let Some(dot_pos) = lextext.rfind('.') {
                if dot_pos > 0 && dot_pos + 1 < lextext.len() {
                    let suffix = &lextext[dot_pos + 1..];
                    if let Some(alias) = resolver.lookup_suffix_alias(suffix) {
                        if !alias.in_use {
                            // lex.c:1938-1940 — push three things in
                            // reverse: the alias text, a space, then
                            // the original word.
                            self.inject_alias_text(&alias.text);
                            self.inject_alias_text(" ");
                            self.inject_alias_text(lextext);
                            resolver.mark_in_use(suffix, true);
                            self.lexstop = false;
                            return true;
                        }
                    }
                }
            }
        }

        false
    }

    /// Push alias text back into the input stream so the lexer
    /// re-reads it. Equivalent to zsh's `inpush(text, INP_ALIAS, an)`
    /// at lex.c:1928,1938,1940. zshrs uses the existing `unget_buf`
    /// (a VecDeque<char>) to inject chars in reverse order so the
    /// next hgetc consumes them first.
    fn inject_alias_text(&mut self, text: &str) {
        // Insert at front in reverse so the first char of `text`
        // comes out first.
        for c in text.chars().rev() {
            self.unget_buf.push_front(c);
        }
    }

    /// Pop the last char from the raw-input capture buffer. Direct
    /// port of zsh/Src/lex.c:2042-2049 `zshlex_raw_back`. Called when
    /// the lexer ungets a char that was just captured raw — the raw
    /// buffer must mirror the live input so this undoes the last add.
    pub fn zshlex_raw_back(&mut self) {
        // lex.c:2045-2046 — guard.
        if self.lex_add_raw == 0 {
            return;
        }
        // lex.c:2047-2048 — `lexbuf_raw.ptr--; lexbuf_raw.len--;`
        self.lexbuf_raw.pop();
    }

    /// Mark the current raw-buffer offset (for restore later). Direct
    /// port of zsh/Src/lex.c:2052-2058 `zshlex_raw_mark`. Returns
    /// `len + offset` so callers can restore via `back_to_mark`.
    pub fn zshlex_raw_mark(&self, offset: i64) -> i64 {
        // lex.c:2055-2056 — guard.
        if self.lex_add_raw == 0 {
            return 0;
        }
        // lex.c:2057 — `return lexbuf_raw.len + offset;`
        (self.lexbuf_raw.len() as i64) + offset
    }

    /// Restore raw-buffer offset to a previously-saved mark. Direct
    /// port of zsh/Src/lex.c:2061-2068 `zshlex_raw_back_to_mark`.
    /// Truncates the raw buffer to `mark` bytes — undoes any captures
    /// since the mark was taken (used when a speculative parse fails
    /// and the lexer rolls back).
    pub fn zshlex_raw_back_to_mark(&mut self, mark: i64) {
        // lex.c:2064-2065 — guard.
        if self.lex_add_raw == 0 {
            return;
        }
        // lex.c:2066-2067 — `lexbuf_raw.ptr = tokstr_raw + mark;
        // lexbuf_raw.len = mark;` — Rust truncate handles both.
        let m = mark.max(0) as usize;
        self.lexbuf_raw.data.truncate(m);
    }

    /// Take the captured raw-input buffer, clearing it. Useful for
    /// callers that need the literal command-sub body after lexing
    /// (e.g. compile-time string capture for `$(...)`).
    pub fn take_raw_buf(&mut self) -> String {
        std::mem::take(&mut self.lexbuf_raw.data)
    }

    /// Save lexical context onto a `LexStack`. Direct port of
    /// zsh/Src/lex.c:215-239 `lex_context_save`. After save, the lexer
    /// is in a clean state suitable for parsing a nested input (command
    /// substitution body, here-doc terminator, eval'd string).
    pub fn lex_context_save(&mut self, ls: &mut LexStack) {
        // lex.c:220-233 — copy live state into the stack.
        ls.dbparens = self.dbparens;
        ls.isfirstln = self.isfirstln;
        ls.isfirstch = self.isfirstch;
        ls.lexflags = self.lexflags;
        ls.tok = self.tok;
        ls.tokstr = self.tokstr.take();
        ls.lexbuf_data = std::mem::take(&mut self.lexbuf.data);
        ls.lexbuf_siz = self.lexbuf.siz;
        ls.lexstop = self.lexstop;
        ls.toklineno = self.toklineno;

        // lex.c:235-238 — reset live state to defaults so a nested
        // parse starts from a clean slate. tokstr/lexbuf are zeroed,
        // lexbuf.siz reset to 256 (the C-source initial alloc).
        self.tokstr = None;
        self.lexbuf.data.clear();
        self.lexbuf.siz = 256;
    }

    /// Restore lexical context from a `LexStack`. Direct port of
    /// zsh/Src/lex.c:244-262 `lex_context_restore`. Inverse of
    /// `lex_context_save`. Called after the nested parse completes.
    pub fn lex_context_restore(&mut self, ls: &mut LexStack) {
        // lex.c:249-261 — copy stack state back into live fields.
        self.dbparens = ls.dbparens;
        self.isfirstln = ls.isfirstln;
        self.isfirstch = ls.isfirstch;
        self.lexflags = ls.lexflags;
        self.tok = ls.tok;
        self.tokstr = ls.tokstr.take();
        self.lexbuf.data = std::mem::take(&mut ls.lexbuf_data);
        self.lexbuf.siz = ls.lexbuf_siz;
        self.lexstop = ls.lexstop;
        self.toklineno = ls.toklineno;
    }

    /// Initialize lexical state. Direct port of zsh/Src/lex.c:440-445
    /// `lexinit`. Resets dbparens / nocorrect / lexstop and sets `tok`
    /// to ENDINPUT so the next gettok starts from a known baseline.
    /// Note: the constructor `Self::new` already sets equivalent
    /// defaults; this method exists for the rare case a caller wants
    /// to recycle a `ZshLexer` across multiple input strings.
    pub fn lexinit(&mut self) {
        // lex.c:443 — `nocorrect = dbparens = lexstop = 0;`
        self.nocorrect = 0;
        self.dbparens = false;
        self.lexstop = false;
        // lex.c:444 — `tok = ENDINPUT;`
        self.tok = LexTok::Endinput;
    }

    /// Check recursion depth; returns true if exceeded
    #[inline]
    fn check_recursion(&mut self) -> bool {
        if self.recursion_depth > MAX_LEXER_RECURSION {
            self.error = Some("lexer exceeded max recursion depth".to_string());
            self.lexstop = true;
            true
        } else {
            false
        }
    }

    /// Check and increment global iteration counter; returns true if limit exceeded
    #[inline]
    fn check_iterations(&mut self) -> bool {
        self.global_iterations += 1;
        if self.global_iterations > 50_000 {
            self.error = Some("lexer exceeded 50K iterations".to_string());
            self.lexstop = true;
            self.tok = LexTok::Lexerr;
            true
        } else {
            false
        }
    }

    /// Get next character from input
    fn hgetc(&mut self) -> Option<char> {
        if self.check_iterations() {
            return None;
        }

        // Re-read from unget_buf: increment lineno on `\n` HERE
        // too. hungetc() decremented lineno when the char was put
        // back; without a matching increment on the way out, every
        // `\n` that's ungetted-then-reread leaves lineno
        // permanently one short. Symptom: $LINENO stuck at 1 in
        // every script statement because the parser ungets the
        // separating newline once between statements.
        if let Some(c) = self.unget_buf.pop_front() {
            if c == '\n' {
                self.lineno += 1;
            }
            return Some(c);
        }

        let c = self.input[self.pos..].chars().next()?;
        self.pos += c.len_utf8();

        if c == '\n' {
            self.lineno += 1;
        }

        Some(c)
    }

    /// Put character back into input
    fn hungetc(&mut self, c: char) {
        self.unget_buf.push_front(c);
        if c == '\n' && self.lineno > 1 {
            self.lineno -= 1;
        }
        self.lexstop = false;
    }

    /// Peek at next character without consuming
    #[allow(dead_code)]
    fn peek(&mut self) -> Option<char> {
        if let Some(&c) = self.unget_buf.front() {
            return Some(c);
        }
        self.input[self.pos..].chars().next()
    }

    /// Add character to token buffer
    fn add(&mut self, c: char) {
        self.lexbuf.add(c);
    }

    /// Check if character is blank (space or tab)
    fn is_blank(c: char) -> bool {
        c == ' ' || c == '\t'
    }

    /// Peek for a zsh numeric range glob shape after a `<`: returns the
    /// captured `N*-M*>` (everything *after* the leading `<`) when the
    /// upcoming chars match `[0-9]*-[0-9]*>` exactly. Otherwise returns
    /// None and leaves the input untouched.
    fn try_numeric_range_glob(&mut self) -> Option<String> {
        let mut buf: Vec<char> = Vec::new();
        // optional leading digits
        loop {
            match self.hgetc() {
                Some(c) if c.is_ascii_digit() => buf.push(c),
                Some(c) => {
                    buf.push(c);
                    break;
                }
                None => break,
            }
        }
        // last char in buf must be '-' for the range form
        if buf.last() != Some(&'-') {
            for c in buf.iter().rev() {
                self.hungetc(*c);
            }
            return None;
        }
        // optional trailing digits
        loop {
            match self.hgetc() {
                Some(c) if c.is_ascii_digit() => buf.push(c),
                Some(c) => {
                    buf.push(c);
                    break;
                }
                None => break,
            }
        }
        if buf.last() != Some(&'>') {
            for c in buf.iter().rev() {
                self.hungetc(*c);
            }
            return None;
        }
        Some(buf.into_iter().collect())
    }

    /// Check if character is blank (including other whitespace except newline)
    fn is_inblank(c: char) -> bool {
        matches!(c, ' ' | '\t' | '\x0b' | '\x0c' | '\r')
    }

    /// Check if character is a digit
    fn is_digit(c: char) -> bool {
        c.is_ascii_digit()
    }

    /// Check if character is identifier start
    #[allow(dead_code)]
    fn is_ident_start(c: char) -> bool {
        c.is_ascii_alphabetic() || c == '_'
    }

    /// Check if character is identifier continuation
    fn is_ident(c: char) -> bool {
        c.is_ascii_alphanumeric() || c == '_'
    }

    /// Main lexer entry point — fetch the next token. Direct port of
    /// zsh/Src/lex.c:265-313 `zshlex`. Loop body matches the C source
    /// `do { ... } while (tok != ENDINPUT && exalias())` at lex.c:270-276,
    /// followed by here-doc draining (lex.c:278-306), newline tracking
    /// (lex.c:307-310), and SEMI/NEWLIN→SEPER folding (lex.c:311-312).
    ///
    /// zshrs port note: `exalias()` (lex.c:1953) is not yet wired into
    /// the loop. The C source iterates as long as exalias keeps
    /// re-injecting alias text into the input buffer; zshrs's alias
    /// expansion happens post-lex in exec.rs. The loop body therefore
    /// runs once and breaks unconditionally — documented divergence.
    pub fn zshlex(&mut self) {
        // lex.c:268-269 — early-out on prior LEXERR.
        if self.tok == LexTok::Lexerr {
            return;
        }

        // Note: Do NOT reset global_iterations here - it must accumulate across all
        // zshlex calls in a parse to prevent infinite loops in the parser

        // lex.c:270-276 — gettok / exalias loop. Without exalias wired,
        // the inner body runs once and we `break` unconditionally.
        loop {
            // lex.c:271-272 — bump inrepeat counter for `repeat N {}`
            // detection.
            if self.inrepeat > 0 {
                self.inrepeat += 1;
            }
            // lex.c:273-274 — at the third token after `repeat`,
            // SHORTLOOPS / SHORTREPEAT options force back into cmd
            // position so the loop body can start. zshrs unconditionally
            // does this since the option-lookup lives in exec.rs.
            if self.inrepeat == 3 {
                self.incmdpos = true;
            }

            // lex.c:275 — `tok = gettok();`
            self.tok = self.gettok();

            // lex.c:276 — `while (tok != ENDINPUT && exalias())` —
            // when exalias re-injects alias text it returns true and
            // the loop iterates. Without exalias wired, we break.
            break;
        }

        // lex.c:277 — `nocorrect &= 1;` — clear bit 1 (lookahead-only)
        // so the persistent low bit survives but the per-word bit is
        // dropped.
        self.nocorrect &= 1;

        // lex.c:278-306 — drain pending here-documents at the start
        // of a new line. zshrs's process_heredocs reads the full body
        // and stitches it onto the matching redir token.
        if self.tok == LexTok::Newlin || self.tok == LexTok::Endinput {
            self.process_heredocs();
        }

        // lex.c:307-310 — track whether we just saw a newline.
        // C uses `inbufct` to distinguish "newline at EOF" (=1)
        // from "newline mid-input" (=-1); zshrs reads `pos < len`.
        if self.tok != LexTok::Newlin {
            self.isnewlin = 0;
        } else {
            self.isnewlin = if self.pos < self.input.len() { -1 } else { 1 };
        }

        // lex.c:311-312 — fold SEMI / NEWLIN into SEPER unless
        // LEXFLAGS_NEWLINE is set to preserve newlines (used by
        // ZLE for completion of partial lines).
        if self.tok == LexTok::Semi || (self.tok == LexTok::Newlin && !self.lexflags.newline) {
            self.tok = LexTok::Seper;
        }

        // Check for reserved words when in command position
        // Also check for "{" and "}" which are special in many contexts
        if self.tok == LexTok::String {
            if let Some(ref s) = self.tokstr {
                if s == "{" {
                    self.tok = LexTok::Inbrace;
                } else if s == "}" {
                    self.tok = LexTok::Outbrace;
                } else if self.incasepat == 0 {
                    // Skip reserved word checking in case pattern context
                    // Words like "time", "end", etc. should be patterns, not reserved words
                    self.check_reserved_word();
                }
            }
        }

        // If we were expecting a heredoc terminator, register it now
        if self.heredoc_pending > 0 && self.tok == LexTok::String {
            if let Some(ref terminator) = self.tokstr {
                let strip_tabs = self.heredoc_pending == 2;
                // Detect originally-quoted terminator (`<<'EOF'`,
                // `<<"EOF"`). The lexer wraps single-quoted text in
                // SNULL (`\u{9d}`) and double-quoted text in DNULL
                // (`\u{9e}`); plain `EOF` has neither. Quoted-terminator
                // heredocs disable variable / command-sub / arithmetic
                // expansion in the body — see `compile_redir` for the
                // expansion side.
                // Quoted terminators (`<<'EOF'`, `<<"EOF"`, `<<\EOF`)
                // disable expansion in the body. SNULL/DNULL mark
                // single/double-quoted spans; BNULL (`\u{9f}`) marks
                // any backslash-escaped char — its presence alone is
                // enough to flag the terminator as quoted (zsh's
                // `<<\EOF` shorthand for `<<'EOF'`).
                let quoted = terminator.contains('\u{9d}')
                    || terminator.contains('\u{9e}')
                    || terminator.contains('\u{9f}')
                    || terminator.starts_with('\'')
                    || terminator.starts_with('"');
                let term = terminator
                    .chars()
                    .filter(|c| {
                        *c != '\''
                            && *c != '"'
                            && *c != '\u{9d}'
                            && *c != '\u{9e}'
                            && *c != '\u{9f}'
                    })
                    .collect::<String>();
                self.heredocs.push(HereDoc {
                    terminator: term,
                    strip_tabs,
                    content: String::new(),
                    quoted,
                    processed: false,
                });
            }
            self.heredoc_pending = 0;
        }

        // Track pattern context inside [[ ... ]] - after = == != =~ the RHS is a pattern
        if self.incond > 0 {
            if let Some(ref s) = self.tokstr {
                // Check if this token is a comparison operator
                // Note: single = is also a comparison operator in [[ ]]
                // The internal marker \u{8d} is used for =
                if s == "="
                    || s == "=="
                    || s == "!="
                    || s == "=~"
                    || s == "\u{8d}"
                    || s == "\u{8d}\u{8d}"
                    || s == "!\u{8d}"
                    || s == "\u{8d}~"
                    || s == "\u{8d}\u{98}"
                {
                    self.incondpat = true;
                } else if self.incondpat {
                    // We were in pattern context, now we've consumed the pattern
                    // Reset after the pattern token is consumed
                    // But actually, pattern can span multiple tokens, so we should
                    // stay in pattern mode until ]] or && or ||
                }
            }
            // Reset pattern context on ]] or logical operators (&&, ||)
            // and grouping parens. zsh par_cond_3 (cond.c) treats
            // these as cond-pattern terminators — the next operand is
            // a fresh primary, NOT a continuation of the prior pattern.
            // Without resetting on Damper/Dbar/Inpar/Outpar, the `(`
            // after `[[ a == a && (b == b ... ` was lexed as a literal
            // glob char (incondpat=true → gettokstr) and the whole
            // remainder collapsed into one String token.
            match self.tok {
                LexTok::Doutbrack
                | LexTok::Damper
                | LexTok::Dbar
                | LexTok::Inpar
                | LexTok::Outpar
                | LexTok::Bang => {
                    self.incondpat = false;
                }
                _ => {}
            }
        } else {
            self.incondpat = false;
        }

        // Update command position for next token based on current token
        // Note: In case patterns (incasepat > 0), | is a pattern separator, not pipeline,
        // so we don't set incmdpos after Bar in that context
        match self.tok {
            LexTok::Seper
            | LexTok::Newlin
            | LexTok::Semi
            | LexTok::Dsemi
            | LexTok::Semiamp
            | LexTok::Semibar
            | LexTok::Amper
            | LexTok::Amperbang
            | LexTok::Inpar
            | LexTok::Inbrace
            | LexTok::Dbar
            | LexTok::Damper
            | LexTok::Baramp
            | LexTok::Inoutpar
            | LexTok::Doloop
            | LexTok::Then
            | LexTok::Elif
            | LexTok::Else
            | LexTok::Doutbrack
            | LexTok::Func => {
                self.incmdpos = true;
            }
            LexTok::Bar
                // In case patterns, | is a pattern separator - don't change incmdpos
                if self.incasepat <= 0 => {
                    self.incmdpos = true;
                }
            LexTok::String
            | LexTok::Typeset
            | LexTok::Envarray
            | LexTok::Outpar
            | LexTok::Case
            | LexTok::Dinbrack => {
                self.incmdpos = false;
            }
            _ => {}
        }

        // Track 'for' keyword for C-style for loop: for (( init; cond; step ))
        // When we see 'for', set infor=2 to expect the init and cond parts
        // Each Dinpar (after semicolon in arithmetic) decrements it
        if self.tok != LexTok::Dinpar {
            self.infor = if self.tok == LexTok::For { 2 } else { 0 };
        }

        // Handle redirection context
        let oldpos = self.incmdpos;
        if self.tok.is_redirop()
            || self.tok == LexTok::For
            || self.tok == LexTok::Foreach
            || self.tok == LexTok::Select
        {
            self.inredir = true;
            self.incmdpos = false;
        } else if self.inredir {
            self.incmdpos = oldpos;
            self.inredir = false;
        }
    }

    /// Process pending here-documents. Walks each heredoc whose body
    /// hasn't been filled yet (content is empty AND terminator is set),
    /// reads lines from input until the terminator, and stuffs the body
    /// into `hdoc.content` IN PLACE. The list itself is preserved so the
    /// parser can index into it after parse() finishes.
    fn process_heredocs(&mut self) {
        let n = self.heredocs.len();
        for i in 0..n {
            // Skip heredocs we've already processed AND those without
            // a terminator (early-error case). The `processed` bool
            // distinguishes "filled with empty body" from "not yet
            // visited" — both have empty `content`.
            if self.heredocs[i].processed || self.heredocs[i].terminator.is_empty() {
                continue;
            }
            let strip_tabs = self.heredocs[i].strip_tabs;
            let terminator = self.heredocs[i].terminator.clone();
            let mut content = String::new();
            let mut line_count = 0;

            loop {
                line_count += 1;
                if line_count > 10000 {
                    self.error = Some("heredoc exceeded 10000 lines".to_string());
                    self.tok = LexTok::Lexerr;
                    return;
                }

                let line = self.read_line();
                if line.is_none() {
                    self.error = Some("here document too large or unterminated".to_string());
                    self.tok = LexTok::Lexerr;
                    return;
                }

                let line = line.unwrap();
                let check_line = if strip_tabs {
                    line.trim_start_matches('\t')
                } else {
                    line.as_str()
                };

                if check_line.trim_end_matches('\n') == terminator {
                    break;
                }

                // `<<-` strips leading tabs from BODY lines too, not just
                // from terminator-match comparison. Without this, tabs in
                // here-doc content survive into stdin.
                if strip_tabs {
                    content.push_str(check_line);
                } else {
                    content.push_str(&line);
                }
            }

            self.heredocs[i].content = content;
            self.heredocs[i].processed = true;
        }
    }

    /// Read a line from input (returns partial line at EOF)
    fn read_line(&mut self) -> Option<String> {
        let mut line = String::new();

        loop {
            match self.hgetc() {
                Some(c) => {
                    line.push(c);
                    if c == '\n' {
                        break;
                    }
                }
                None => {
                    // EOF - return partial line if any
                    if line.is_empty() {
                        return None;
                    }
                    break;
                }
            }
        }

        Some(line)
    }

    /// Get the next token. Direct port of zsh/Src/lex.c:613-936
    /// `gettok`. Reads characters from the input via hgetc, dispatches
    /// on the leading char through lexact1[]/lexact2[] tables (zshrs
    /// uses inline `match` in lex_initial / lex_inang / lex_outang
    /// since Rust pattern-matching subsumes the table dispatch).
    ///
    /// Structural divergence from C: the giant ~322-line C switch
    /// statement at lex.c:725-936 is split into helper methods in
    /// Rust (lex_initial = LX1_OTHER plus the punctuation cases,
    /// lex_inang / lex_outang for the < and > arms). The flow is
    /// equivalent — same chars consumed, same tokens emitted — but
    /// the source-level layout differs. C's table-driven dispatch
    /// would Rust-port as `match c { '\\' => ..., '\n' => ..., ... }`
    /// which is what the helpers ultimately do.
    fn gettok(&mut self) -> LexTok {
        // lex.c:621 — `tokstr = NULL;` reset before each token.
        self.tokstr = None;
        // (zshrs-specific: tokfd reset lives here too — C does it
        // implicitly via the `peekfd = -1` local at lex.c:617 used
        // only when a digit-prefix redirection is detected.)
        self.tokfd = -1;

        // lex.c:622 — `while (iblank(c = hgetc()) && !lexstop);` —
        // skip leading blanks (space/tab, NOT newline).
        let mut ws_iterations = 0;
        loop {
            ws_iterations += 1;
            if ws_iterations > 100_000 {
                self.error = Some("gettok: infinite loop in whitespace skip".to_string());
                return LexTok::Lexerr;
            }
            let c = match self.hgetc() {
                Some(c) => c,
                None => {
                    // lex.c:624-625 — lexstop set, return ENDINPUT
                    // (or LEXERR if errflag is set elsewhere).
                    self.lexstop = true;
                    return if self.error.is_some() {
                        LexTok::Lexerr
                    } else {
                        LexTok::Endinput
                    };
                }
            };

            if !Self::is_blank(c) {
                self.hungetc(c);
                break;
            }
        }

        let c = match self.hgetc() {
            Some(c) => c,
            None => {
                self.lexstop = true;
                return LexTok::Endinput;
            }
        };

        // lex.c:623 — `toklineno = lineno;`
        self.toklineno = self.lineno;
        // lex.c:626 — `isfirstln = 0;` once we've consumed any non-
        // blank.
        self.isfirstln = false;

        // lex.c:631-648 — dbparens (inside `(( … ))`) special path:
        // call dquote_parse with `;` or `)` as the end-char and
        // either return DINPAR (continue for-loop arith) or DOUTPAR
        // (close the arith block) or LEXERR.
        if self.dbparens {
            return self.lex_arith(c);
        }

        // lex.c:649-668 — digit prefix on a redirection: `2> file`
        // treats `2` as the fd to redirect, not a literal arg. Three
        // shapes: `N>`/`N<` (single redir), `N&>` (errwrite), or
        // anything else (push back, treat as literal digit).
        if Self::is_digit(c) {
            let d = self.hgetc();
            match d {
                Some('&') => {
                    let e = self.hgetc();
                    if e == Some('>') {
                        // lex.c:653-657 — `N&>` shape detected.
                        self.tokfd = (c as u8 - b'0') as i32;
                        self.hungetc('>');
                        return self.lex_initial('&');
                    }
                    // lex.c:658-661 — not `N&>`, push everything back.
                    if let Some(e) = e {
                        self.hungetc(e);
                    }
                    self.hungetc('&');
                }
                Some('>') | Some('<') => {
                    // lex.c:662-664 — `N>` or `N<` shape detected.
                    self.tokfd = (c as u8 - b'0') as i32;
                    return self.lex_initial(d.unwrap());
                }
                Some(d) => {
                    // lex.c:665-668 — not a redir prefix, push back.
                    self.hungetc(d);
                }
                None => {}
            }
            self.lexstop = false;
        }

        // lex.c:670-936 — main dispatch on the leading char. zshrs
        // delegates to lex_initial which holds the equivalent of
        // lex.c's `switch (lexact1[c])` plus the gettokstr fallback
        // for LX1_OTHER.
        self.lex_initial(c)
    }

    /// Lex (( ... )) arithmetic expression
    fn lex_arith(&mut self, c: char) -> LexTok {
        self.lexbuf.clear();
        self.hungetc(c);

        let end_char = if self.infor > 0 { ';' } else { ')' };
        if self.dquote_parse(end_char, false).is_err() {
            return LexTok::Lexerr;
        }

        self.tokstr = Some(self.lexbuf.as_str().to_string());

        if !self.lexstop && self.infor > 0 {
            self.infor -= 1;
            return LexTok::Dinpar;
        }

        // Check for closing ))
        match self.hgetc() {
            Some(')') => {
                self.dbparens = false;
                LexTok::Doutpar
            }
            c => {
                if let Some(c) = c {
                    self.hungetc(c);
                }
                LexTok::Lexerr
            }
        }
    }

    /// Handle initial character of token
    fn lex_initial(&mut self, c: char) -> LexTok {
        // Handle comments
        if c == '#' && !self.nocomments {
            return self.lex_comment();
        }

        match c {
            '\\' => {
                let d = self.hgetc();
                if d == Some('\n') {
                    // Line continuation - get next token
                    return self.gettok();
                }
                if let Some(d) = d {
                    self.hungetc(d);
                }
                self.lexstop = false;
                self.gettokstr(c, false)
            }

            '\n' => LexTok::Newlin,

            ';' => {
                let d = self.hgetc();
                match d {
                    Some(';') => LexTok::Dsemi,
                    Some('&') => LexTok::Semiamp,
                    Some('|') => LexTok::Semibar,
                    _ => {
                        if let Some(d) = d {
                            self.hungetc(d);
                        }
                        self.lexstop = false;
                        LexTok::Semi
                    }
                }
            }

            '&' => {
                let d = self.hgetc();
                match d {
                    Some('&') => LexTok::Damper,
                    Some('!') | Some('|') => LexTok::Amperbang,
                    Some('>') => {
                        self.tokfd = self.tokfd.max(0);
                        let e = self.hgetc();
                        match e {
                            Some('!') | Some('|') => LexTok::Outangampbang,
                            Some('>') => {
                                let f = self.hgetc();
                                match f {
                                    Some('!') | Some('|') => LexTok::Doutangampbang,
                                    _ => {
                                        if let Some(f) = f {
                                            self.hungetc(f);
                                        }
                                        self.lexstop = false;
                                        LexTok::Doutangamp
                                    }
                                }
                            }
                            _ => {
                                if let Some(e) = e {
                                    self.hungetc(e);
                                }
                                self.lexstop = false;
                                LexTok::Ampoutang
                            }
                        }
                    }
                    _ => {
                        if let Some(d) = d {
                            self.hungetc(d);
                        }
                        self.lexstop = false;
                        LexTok::Amper
                    }
                }
            }

            '|' => {
                let d = self.hgetc();
                match d {
                    Some('|') if self.incasepat <= 0 => LexTok::Dbar,
                    Some('&') => LexTok::Baramp,
                    _ => {
                        if let Some(d) = d {
                            self.hungetc(d);
                        }
                        self.lexstop = false;
                        LexTok::Bar
                    }
                }
            }

            '(' => {
                let d = self.hgetc();
                match d {
                    Some('(') => {
                        if self.infor > 0 {
                            self.dbparens = true;
                            return LexTok::Dinpar;
                        }
                        if self.incmdpos {
                            // Could be (( arithmetic )) or ( subshell )
                            self.lexbuf.clear();
                            match self.cmd_or_math() {
                                CmdOrMath::Math => {
                                    self.tokstr = Some(self.lexbuf.as_str().to_string());
                                    return LexTok::Dinpar;
                                }
                                CmdOrMath::Cmd => {
                                    self.tokstr = None;
                                    return LexTok::Inpar;
                                }
                                CmdOrMath::Err => return LexTok::Lexerr,
                            }
                        }
                        self.hungetc('(');
                        self.lexstop = false;
                        self.gettokstr('(', false)
                    }
                    Some(')') => LexTok::Inoutpar,
                    _ => {
                        if let Some(d) = d {
                            self.hungetc(d);
                        }
                        self.lexstop = false;
                        // In pattern context (after == != =~ in [[ ]]), ( is part of pattern
                        // In case pattern context, ( at start is optional delimiter, not pattern
                        // incasepat == 1 means "at start of pattern", > 1 means "inside pattern"
                        if self.incondpat || self.incasepat > 1 {
                            self.gettokstr('(', false)
                        } else if self.incond == 1 || self.incmdpos || self.incasepat == 1 {
                            LexTok::Inpar
                        } else {
                            self.gettokstr('(', false)
                        }
                    }
                }
            }

            ')' => LexTok::Outpar,

            '{' => {
                // { is a command group only if followed by whitespace,
                // newline, or `}` (the empty-block form `{}`). zsh
                // treats `{}` as an empty compound — `foo() {}` is a
                // valid no-op function. Without `}` in this list,
                // `{}` got consumed as one literal token and ran as a
                // command, failing "command not found: {}".
                // The empty `{}` is also recognised AFTER a function
                // header `name()` even when `incmdpos` got cleared by
                // the preceding Outpar — peek for `}` regardless and
                // treat as Inbrace so `foo() {}` parses as a no-op
                // function body.
                let next = self.hgetc();
                let next_is_close = matches!(next, Some('}'));
                if self.incmdpos {
                    let is_brace_group = match next {
                        Some(' ') | Some('\t') | Some('\n') | Some('}') | None => true,
                        _ => false,
                    };
                    if let Some(ch) = next {
                        self.hungetc(ch);
                    }
                    if is_brace_group {
                        self.tokstr = Some("{".to_string());
                        LexTok::Inbrace
                    } else {
                        self.gettokstr(c, false)
                    }
                } else if next_is_close {
                    // `{}` empty block in non-cmd position (function
                    // body after `()`). Treat as Inbrace; the parser
                    // will follow with Outbrace.
                    if let Some(ch) = next {
                        self.hungetc(ch);
                    }
                    self.tokstr = Some("{".to_string());
                    LexTok::Inbrace
                } else {
                    if let Some(ch) = next {
                        self.hungetc(ch);
                    }
                    self.gettokstr(c, false)
                }
            }

            '}' => {
                // } at start of token is always Outbrace (ends command group)
                // Inside a word, } would be handled by gettokstr but we never reach here mid-word
                self.tokstr = Some("}".to_string());
                LexTok::Outbrace
            }

            '[' => {
                // [[ is a conditional expression start
                // [ can also be a command (test builtin) or array subscript
                // In case patterns (incasepat > 0), [ is part of glob pattern like [yY]
                if self.incasepat > 0 {
                    self.gettokstr(c, false)
                } else if self.incmdpos {
                    let next = self.hgetc();
                    if next == Some('[') {
                        // [[ - double bracket conditional
                        self.tokstr = Some("[[".to_string());
                        self.incond = 1;
                        return LexTok::Dinbrack;
                    }
                    // Single [ - either test command or start of glob pattern
                    if let Some(ch) = next {
                        self.hungetc(ch);
                    }
                    self.tokstr = Some("[".to_string());
                    LexTok::String
                } else {
                    self.gettokstr(c, false)
                }
            }

            ']' => {
                // ]] ends a conditional expression started by [[
                if self.incond > 0 {
                    let next = self.hgetc();
                    if next == Some(']') {
                        self.tokstr = Some("]]".to_string());
                        self.incond = 0;
                        return LexTok::Doutbrack;
                    }
                    if let Some(ch) = next {
                        self.hungetc(ch);
                    }
                }
                self.gettokstr(c, false)
            }

            '<' => {
                // In pattern context, < is literal (e.g., <-> in glob)
                if self.incondpat || self.incasepat > 0 {
                    self.gettokstr(c, false)
                } else {
                    self.lex_inang()
                }
            }

            '>' => {
                // In pattern context, > is literal
                if self.incondpat || self.incasepat > 0 {
                    self.gettokstr(c, false)
                } else {
                    self.lex_outang()
                }
            }

            _ => self.gettokstr(c, false),
        }
    }

    /// Lex comment
    fn lex_comment(&mut self) -> LexTok {
        if self.lexflags.comments_keep {
            self.lexbuf.clear();
            self.add('#');
        }

        loop {
            let c = self.hgetc();
            match c {
                Some('\n') | None => break,
                Some(c) => {
                    if self.lexflags.comments_keep {
                        self.add(c);
                    }
                }
            }
        }

        if self.lexflags.comments_keep {
            self.tokstr = Some(self.lexbuf.as_str().to_string());
            if !self.lexstop {
                self.hungetc('\n');
            }
            return LexTok::String;
        }

        if self.lexflags.comments_strip && self.lexstop {
            return LexTok::Endinput;
        }

        LexTok::Newlin
    }

    /// Lex < and variants
    fn lex_inang(&mut self) -> LexTok {
        let d = self.hgetc();
        match d {
            Some('(') => {
                // Process substitution <(...)
                self.hungetc('(');
                self.lexstop = false;
                self.gettokstr('<', false)
            }
            Some('>') => LexTok::Inoutang,
            Some('<') => {
                let e = self.hgetc();
                match e {
                    Some('(') => {
                        self.hungetc('(');
                        self.hungetc('<');
                        LexTok::Inang
                    }
                    Some('<') => LexTok::Trinang,
                    Some('-') => {
                        self.heredoc_pending = 2; // <<- expects terminator next
                        LexTok::Dinangdash
                    }
                    _ => {
                        if let Some(e) = e {
                            self.hungetc(e);
                        }
                        self.lexstop = false;
                        self.heredoc_pending = 1; // << expects terminator next
                        LexTok::Dinang
                    }
                }
            }
            Some('&') => LexTok::Inangamp,
            _ => {
                if let Some(d) = d {
                    self.hungetc(d);
                }
                self.lexstop = false;
                LexTok::Inang
            }
        }
    }

    /// Lex > and variants
    fn lex_outang(&mut self) -> LexTok {
        let d = self.hgetc();
        match d {
            Some('(') => {
                // Process substitution >(...)
                self.hungetc('(');
                self.lexstop = false;
                self.gettokstr('>', false)
            }
            Some('&') => {
                let e = self.hgetc();
                match e {
                    Some('!') | Some('|') => LexTok::Outangampbang,
                    _ => {
                        if let Some(e) = e {
                            self.hungetc(e);
                        }
                        self.lexstop = false;
                        LexTok::Outangamp
                    }
                }
            }
            Some('!') | Some('|') => LexTok::Outangbang,
            Some('>') => {
                let e = self.hgetc();
                match e {
                    Some('&') => {
                        let f = self.hgetc();
                        match f {
                            Some('!') | Some('|') => LexTok::Doutangampbang,
                            _ => {
                                if let Some(f) = f {
                                    self.hungetc(f);
                                }
                                self.lexstop = false;
                                LexTok::Doutangamp
                            }
                        }
                    }
                    Some('!') | Some('|') => LexTok::Doutangbang,
                    Some('(') => {
                        self.hungetc('(');
                        self.hungetc('>');
                        LexTok::Outang
                    }
                    _ => {
                        if let Some(e) = e {
                            self.hungetc(e);
                        }
                        self.lexstop = false;
                        LexTok::Doutang
                    }
                }
            }
            _ => {
                if let Some(d) = d {
                    self.hungetc(d);
                }
                self.lexstop = false;
                LexTok::Outang
            }
        }
    }

    /// Get rest of token string
    fn gettokstr(&mut self, c: char, sub: bool) -> LexTok {
        let mut bct = 0; // brace count
        let mut pct = 0; // parenthesis count
        let mut brct = 0; // bracket count
        let mut in_brace_param = 0;
        let mut peek = LexTok::String;
        let mut intpos = 1;
        let mut unmatched = '\0';
        let mut c = c;
        const MAX_ITERATIONS: usize = 100_000;
        let mut iterations = 0;

        if !sub {
            self.lexbuf.clear();
        }

        loop {
            iterations += 1;
            if iterations > MAX_ITERATIONS {
                self.error = Some("gettokstr exceeded maximum iterations".to_string());
                return LexTok::Lexerr;
            }

            let inbl = Self::is_inblank(c);

            if inbl && in_brace_param == 0 && pct == 0 {
                // Whitespace outside brace param ends token
                break;
            }

            match c {
                // Whitespace is handled above for most cases
                ')' => {
                    if in_brace_param > 0 || sub {
                        self.add(char_tokens::OUTPAR);
                    } else if pct > 0 {
                        pct -= 1;
                        self.add(char_tokens::OUTPAR);
                    } else {
                        break;
                    }
                }

                '|' => {
                    if pct == 0 && in_brace_param == 0 {
                        if sub {
                            self.add(c);
                        } else {
                            break;
                        }
                    } else {
                        self.add(char_tokens::BAR);
                    }
                }

                '$' => {
                    let e = self.hgetc();
                    match e {
                        Some('\\') => {
                            let f = self.hgetc();
                            if f != Some('\n') {
                                if let Some(f) = f {
                                    self.hungetc(f);
                                }
                                self.hungetc('\\');
                                self.add(char_tokens::STRING);
                            } else {
                                // Line continuation after $
                                continue;
                            }
                        }
                        Some('[') => {
                            // $[...] arithmetic
                            self.add(char_tokens::STRING);
                            self.add(char_tokens::INBRACK);
                            if self.dquote_parse(']', sub).is_err() {
                                peek = LexTok::Lexerr;
                                break;
                            }
                            self.add(char_tokens::OUTBRACK);
                        }
                        Some('(') => {
                            // $(...) or $((...))
                            self.add(char_tokens::STRING);
                            match self.cmd_or_math_sub() {
                                CmdOrMath::Cmd => self.add(char_tokens::OUTPAR),
                                CmdOrMath::Math => self.add(char_tokens::OUTPARMATH),
                                CmdOrMath::Err => {
                                    peek = LexTok::Lexerr;
                                    break;
                                }
                            }
                        }
                        Some('{') => {
                            self.add(c);
                            self.add(char_tokens::INBRACE);
                            bct += 1;
                            if in_brace_param == 0 {
                                in_brace_param = bct;
                            }
                        }
                        _ => {
                            if let Some(e) = e {
                                self.hungetc(e);
                            }
                            self.lexstop = false;
                            self.add(char_tokens::STRING);
                        }
                    }
                }

                '[' => {
                    if in_brace_param == 0 {
                        brct += 1;
                    }
                    self.add(char_tokens::INBRACK);
                }

                ']' => {
                    if in_brace_param == 0 && brct > 0 {
                        brct -= 1;
                    }
                    self.add(char_tokens::OUTBRACK);
                }

                '(' => {
                    if in_brace_param == 0 {
                        pct += 1;
                    }
                    self.add(char_tokens::INPAR);
                }

                '{' => {
                    // Track braces for both ${...} param expansion and {...} brace expansion
                    bct += 1;
                    self.add(c);
                }

                '}' => {
                    if in_brace_param > 0 {
                        if bct == in_brace_param {
                            in_brace_param = 0;
                        }
                        bct -= 1;
                        self.add(char_tokens::OUTBRACE);
                    } else if bct > 0 {
                        // Closing a brace expansion like {a,b}
                        bct -= 1;
                        self.add(c);
                    } else {
                        break;
                    }
                }

                '>' => {
                    // In pattern context (incondpat), > is literal
                    if in_brace_param > 0 || sub || self.incondpat || self.incasepat > 0 {
                        self.add(c);
                    } else {
                        let e = self.hgetc();
                        if e != Some('(') {
                            if let Some(e) = e {
                                self.hungetc(e);
                            }
                            self.lexstop = false;
                            break;
                        }
                        // >(...)
                        self.add(char_tokens::OUTANGPROC);
                        if self.skip_command_sub().is_err() {
                            peek = LexTok::Lexerr;
                            break;
                        }
                        self.add(char_tokens::OUTPAR);
                    }
                }

                '<' => {
                    // In pattern context (incondpat), < is literal
                    if in_brace_param > 0 || sub || self.incondpat || self.incasepat > 0 {
                        self.add(c);
                    } else if let Some(range_chars) = self.try_numeric_range_glob() {
                        // zsh numeric range glob `<N-M>`, `<->`, `<N->`,
                        // `<-M>`. When `<` mid-word matches that exact
                        // shape, swallow it into the word instead of
                        // breaking out for redirection.
                        self.add(c);
                        for ch in range_chars.chars() {
                            self.add(ch);
                        }
                    } else {
                        let e = self.hgetc();
                        if e != Some('(') {
                            if let Some(e) = e {
                                self.hungetc(e);
                            }
                            self.lexstop = false;
                            break;
                        }
                        // <(...)
                        self.add(char_tokens::INANG);
                        if self.skip_command_sub().is_err() {
                            peek = LexTok::Lexerr;
                            break;
                        }
                        self.add(char_tokens::OUTPAR);
                    }
                }

                '=' => {
                    if !sub {
                        if intpos > 0 {
                            // At start of token, check for =(...) process substitution
                            let e = self.hgetc();
                            if e == Some('(') {
                                self.add(char_tokens::EQUALS);
                                if self.skip_command_sub().is_err() {
                                    peek = LexTok::Lexerr;
                                    break;
                                }
                                self.add(char_tokens::OUTPAR);
                            } else {
                                if let Some(e) = e {
                                    self.hungetc(e);
                                }
                                self.lexstop = false;
                                self.add(char_tokens::EQUALS);
                            }
                        } else if peek != LexTok::Envstring
                            && (self.incmdpos || self.intypeset)
                            && bct == 0
                            && brct == 0
                            && self.incasepat == 0
                        {
                            // Check for VAR=value assignment (but not in case pattern context)
                            let tok_so_far = self.lexbuf.as_str().to_string();
                            if self.is_valid_assignment_target(&tok_so_far) {
                                let next = self.hgetc();
                                if next == Some('(') {
                                    // VAR=(...) array assignment - include '=' in tokstr
                                    self.add(char_tokens::EQUALS);
                                    self.tokstr = Some(self.lexbuf.as_str().to_string());
                                    return LexTok::Envarray;
                                }
                                if let Some(next) = next {
                                    self.hungetc(next);
                                }
                                self.lexstop = false;
                                peek = LexTok::Envstring;
                                intpos = 2;
                                self.add(char_tokens::EQUALS);
                            } else {
                                self.add(char_tokens::EQUALS);
                            }
                        } else {
                            self.add(char_tokens::EQUALS);
                        }
                    } else {
                        self.add(char_tokens::EQUALS);
                    }
                }

                '\\' => {
                    let next = self.hgetc();
                    if next == Some('\n') {
                        // Line continuation
                        let next = self.hgetc();
                        if let Some(next) = next {
                            c = next;
                            continue;
                        }
                        break;
                    } else {
                        self.add(char_tokens::BNULL);
                        if let Some(next) = next {
                            self.add(next);
                        }
                    }
                }

                '\'' => {
                    // Single quoted string - everything literal until '
                    self.add(char_tokens::SNULL);
                    loop {
                        let ch = self.hgetc();
                        match ch {
                            Some('\'') => break,
                            Some(ch) => self.add(ch),
                            None => {
                                self.lexstop = true;
                                unmatched = '\'';
                                peek = LexTok::Lexerr;
                                break;
                            }
                        }
                    }
                    if unmatched != '\0' {
                        break;
                    }
                    self.add(char_tokens::SNULL);
                }

                '"' => {
                    // Double quoted string
                    self.add(char_tokens::DNULL);
                    if self.dquote_parse('"', sub).is_err() {
                        unmatched = '"';
                        if !self.lexflags.active {
                            peek = LexTok::Lexerr;
                        }
                        break;
                    }
                    self.add(char_tokens::DNULL);
                }

                '`' => {
                    // Backtick command substitution
                    self.add(char_tokens::TICK);
                    loop {
                        let ch = self.hgetc();
                        match ch {
                            Some('`') => break,
                            Some('\\') => {
                                let next = self.hgetc();
                                match next {
                                    Some('\n') => continue, // Line continuation
                                    Some(c) if c == '`' || c == '\\' || c == '$' => {
                                        self.add(char_tokens::BNULL);
                                        self.add(c);
                                    }
                                    Some(c) => {
                                        self.add('\\');
                                        self.add(c);
                                    }
                                    None => break,
                                }
                            }
                            Some(ch) => self.add(ch),
                            None => {
                                self.lexstop = true;
                                unmatched = '`';
                                peek = LexTok::Lexerr;
                                break;
                            }
                        }
                    }
                    if unmatched != '\0' {
                        break;
                    }
                    self.add(char_tokens::TICK);
                }

                '~' => {
                    self.add(char_tokens::TILDE);
                }

                '#' => {
                    self.add(char_tokens::POUND);
                }

                '^' => {
                    self.add(char_tokens::HAT);
                }

                '*' => {
                    self.add(char_tokens::STAR);
                }

                '?' => {
                    self.add(char_tokens::QUEST);
                }

                ',' if bct > in_brace_param => {
                    self.add(char_tokens::COMMA);
                }

                '-' => {
                    self.add(char_tokens::DASH);
                }

                '!' if brct > 0 => {
                    self.add(char_tokens::BANG);
                }

                // Terminators
                '\n' | ';' | '&' => {
                    break;
                }

                _ => {
                    self.add(c);
                }
            }

            c = match self.hgetc() {
                Some(c) => c,
                None => {
                    self.lexstop = true;
                    break;
                }
            };

            if intpos > 0 {
                intpos -= 1;
            }
        }

        // Put back the character that ended the token
        if !self.lexstop {
            self.hungetc(c);
        }

        if unmatched != '\0' && !self.lexflags.active {
            self.error = Some(format!("unmatched {}", unmatched));
        }

        if in_brace_param > 0 {
            self.error = Some("closing brace expected".to_string());
        }

        self.tokstr = Some(self.lexbuf.as_str().to_string());
        peek
    }

    /// Check if a string is a valid assignment target (identifier or array ref)
    fn is_valid_assignment_target(&self, s: &str) -> bool {
        let mut chars = s.chars().peekable();

        // Check for leading digit (invalid)
        if let Some(&c) = chars.peek() {
            if c.is_ascii_digit() {
                // Could be array index, check rest
                while let Some(&c) = chars.peek() {
                    if !c.is_ascii_digit() {
                        break;
                    }
                    chars.next();
                }
                return chars.peek().is_none();
            }
        }

        // Check identifier
        let mut has_ident = false;
        while let Some(&c) = chars.peek() {
            if c == char_tokens::INBRACK || c == '[' {
                break;
            }
            if c == '+' {
                // foo+=value
                chars.next();
                return chars.peek().is_none() || chars.peek() == Some(&'=');
            }
            if !Self::is_ident(c) && c != char_tokens::STRING && !char_tokens::is_token(c) {
                return false;
            }
            has_ident = true;
            chars.next();
        }

        has_ident
    }

    /// Parse the body of a double-quoted string (or any context that
    /// uses double-quote tokenization — `(( ))`, `${...}`, `$( ( ) )`).
    /// Direct port of zsh/Src/lex.c:1486-1693 `dquote_parse`. Reads
    /// chars until `endchar` is seen at depth 0, handling escapes,
    /// `${...}` parameter substitutions, `$(...)` and backtick command
    /// substitutions, `$((...))` arithmetic, and inner double-quoted
    /// strings. The `sub` flag toggles substitution-context tokens
    /// (lex.c:1487 `int sub` argument).
    ///
    /// zshrs port note: the recursion guard at the top is a Rust
    /// safety net; the C source relies on the runtime stack. Inner
    /// logic delegates to `dquote_parse_inner` which holds the actual
    /// per-char state machine matching lex.c:1495-1692.
    fn dquote_parse(&mut self, endchar: char, sub: bool) -> Result<(), ()> {
        self.recursion_depth += 1;
        if self.check_recursion() {
            self.recursion_depth -= 1;
            return Err(());
        }

        let result = self.dquote_parse_inner(endchar, sub);
        self.recursion_depth -= 1;
        result
    }

    fn dquote_parse_inner(&mut self, endchar: char, sub: bool) -> Result<(), ()> {
        let mut pct = 0; // parenthesis count
        let mut brct = 0; // bracket count
        let mut bct = 0; // brace count (for ${...})
        let mut intick = false; // inside backtick
        let is_math = endchar == ')' || endchar == ']' || self.infor > 0;
        const MAX_ITERATIONS: usize = 100_000;
        let mut iterations = 0;

        loop {
            iterations += 1;
            if iterations > MAX_ITERATIONS {
                self.error = Some("dquote_parse exceeded maximum iterations".to_string());
                return Err(());
            }
            let c = self.hgetc();
            let c = match c {
                Some(c) if c == endchar && !intick && bct == 0 => {
                    if is_math && (pct > 0 || brct > 0) {
                        self.add(c);
                        if c == ')' {
                            pct -= 1;
                        } else if c == ']' {
                            brct -= 1;
                        }
                        continue;
                    }
                    return Ok(());
                }
                Some(c) => c,
                None => {
                    self.lexstop = true;
                    return Err(());
                }
            };

            match c {
                '\\' => {
                    let next = self.hgetc();
                    match next {
                        Some('\n') if !sub => continue, // Line continuation
                        Some(c)
                            if c == '$'
                                || c == '\\'
                                || (c == '}' && !intick && bct > 0)
                                || c == endchar
                                || c == '`'
                                || (endchar == ']'
                                    && (c == '['
                                        || c == ']'
                                        || c == '('
                                        || c == ')'
                                        || c == '{'
                                        || c == '}'
                                        || (c == '"' && sub))) =>
                        {
                            self.add(char_tokens::BNULL);
                            self.add(c);
                        }
                        Some(c) => {
                            self.add('\\');
                            self.hungetc(c);
                            continue;
                        }
                        None => {
                            self.add('\\');
                        }
                    }
                }

                '$' => {
                    if intick {
                        self.add(c);
                        continue;
                    }
                    let next = self.hgetc();
                    match next {
                        Some('(') => {
                            self.add(char_tokens::QSTRING);
                            match self.cmd_or_math_sub() {
                                CmdOrMath::Cmd => self.add(char_tokens::OUTPAR),
                                CmdOrMath::Math => self.add(char_tokens::OUTPARMATH),
                                CmdOrMath::Err => return Err(()),
                            }
                        }
                        Some('[') => {
                            self.add(char_tokens::STRING);
                            self.add(char_tokens::INBRACK);
                            self.dquote_parse(']', sub)?;
                            self.add(char_tokens::OUTBRACK);
                        }
                        Some('{') => {
                            self.add(char_tokens::QSTRING);
                            self.add(char_tokens::INBRACE);
                            bct += 1;
                        }
                        Some('$') => {
                            self.add(char_tokens::QSTRING);
                            self.add('$');
                        }
                        _ => {
                            if let Some(next) = next {
                                self.hungetc(next);
                            }
                            self.lexstop = false;
                            self.add(char_tokens::QSTRING);
                        }
                    }
                }

                '}' => {
                    if intick || bct == 0 {
                        self.add(c);
                    } else {
                        self.add(char_tokens::OUTBRACE);
                        bct -= 1;
                    }
                }

                '`' => {
                    self.add(char_tokens::QTICK);
                    intick = !intick;
                }

                '(' => {
                    if !is_math || bct == 0 {
                        pct += 1;
                    }
                    self.add(c);
                }

                ')' => {
                    if !is_math || bct == 0 {
                        if pct == 0 && is_math {
                            return Err(());
                        }
                        pct -= 1;
                    }
                    self.add(c);
                }

                '[' => {
                    if !is_math || bct == 0 {
                        brct += 1;
                    }
                    self.add(c);
                }

                ']' => {
                    if !is_math || bct == 0 {
                        if brct == 0 && is_math {
                            return Err(());
                        }
                        brct -= 1;
                    }
                    self.add(c);
                }

                '"' => {
                    if intick || (endchar != '"' && bct == 0) {
                        self.add(c);
                    } else if bct > 0 {
                        self.add(char_tokens::DNULL);
                        self.dquote_parse('"', sub)?;
                        self.add(char_tokens::DNULL);
                    } else {
                        return Err(());
                    }
                }

                _ => {
                    self.add(c);
                }
            }
        }
    }

    /// Determine if (( is arithmetic or command
    /// Decide whether `( ... )` after a `$` is a math expression
    /// `$((...))` or a command substitution `$(...)`. Direct port of
    /// zsh/Src/lex.c:495-532 `cmd_or_math`. Tries dquote_parse first;
    /// if it succeeds AND the next char is `)` (closing the second
    /// paren of `(( ))`), it's math. Otherwise rewinds and treats as
    /// a command substitution.
    fn cmd_or_math(&mut self) -> CmdOrMath {
        let oldlen = self.lexbuf.len();

        self.add(char_tokens::INPAR);
        self.add('(');

        if self.dquote_parse(')', false).is_err() {
            // Back up and try as command
            while self.lexbuf.len() > oldlen {
                if let Some(c) = self.lexbuf.pop() {
                    self.hungetc(c);
                }
            }
            self.hungetc('(');
            self.lexstop = false;
            return if self.skip_command_sub().is_err() {
                CmdOrMath::Err
            } else {
                CmdOrMath::Cmd
            };
        }

        // Check for closing )
        let c = self.hgetc();
        if c == Some(')') {
            self.add(')');
            return CmdOrMath::Math;
        }

        // Not math, back up
        if let Some(c) = c {
            self.hungetc(c);
        }
        self.lexstop = false;

        // Back up token
        while self.lexbuf.len() > oldlen {
            if let Some(c) = self.lexbuf.pop() {
                self.hungetc(c);
            }
        }
        self.hungetc('(');

        if self.skip_command_sub().is_err() {
            CmdOrMath::Err
        } else {
            CmdOrMath::Cmd
        }
    }

    /// Parse `$(...)` or `$((...))` after the `$` has been consumed.
    /// Direct port of zsh/Src/lex.c:540-573 `cmd_or_math_sub`. Reads
    /// the next char to discriminate: a leading `(` plus successful
    /// math parse via `cmd_or_math` → arithmetic substitution (with
    /// the open-paren retroactively rewritten to Inparmath); else
    /// command substitution via skip_command_sub.
    fn cmd_or_math_sub(&mut self) -> CmdOrMath {
        const MAX_CONTINUATIONS: usize = 10_000;
        let mut continuations = 0;

        loop {
            continuations += 1;
            if continuations > MAX_CONTINUATIONS {
                self.error = Some("cmd_or_math_sub: too many line continuations".to_string());
                return CmdOrMath::Err;
            }

            let c = self.hgetc();
            if c == Some('\\') {
                let c2 = self.hgetc();
                if c2 != Some('\n') {
                    if let Some(c2) = c2 {
                        self.hungetc(c2);
                    }
                    self.hungetc('\\');
                    self.lexstop = false;
                    return if self.skip_command_sub().is_err() {
                        CmdOrMath::Err
                    } else {
                        CmdOrMath::Cmd
                    };
                }
                // Line continuation, try again (loop instead of recursion)
                continue;
            }

            // Not a line continuation, process normally
            if c == Some('(') {
                // Might be $((...))
                let lexpos = self.lexbuf.len();
                self.add(char_tokens::INPAR);
                self.add('(');

                if self.dquote_parse(')', false).is_ok() {
                    let c2 = self.hgetc();
                    if c2 == Some(')') {
                        self.add(')');
                        return CmdOrMath::Math;
                    }
                    if let Some(c2) = c2 {
                        self.hungetc(c2);
                    }
                }

                // Not math, restore and parse as command
                while self.lexbuf.len() > lexpos {
                    if let Some(ch) = self.lexbuf.pop() {
                        self.hungetc(ch);
                    }
                }
                self.hungetc('(');
                self.lexstop = false;
            } else {
                if let Some(c) = c {
                    self.hungetc(c);
                }
                self.lexstop = false;
            }

            return if self.skip_command_sub().is_err() {
                CmdOrMath::Err
            } else {
                CmdOrMath::Cmd
            };
        }
    }

    /// Skip over `(...)` for command-style substitutions: `$(...)`,
    /// `<(...)`, `>(...)`. Direct port of zsh/Src/lex.c:2080-end
    /// `skipcomm`. Per the C source comment: "we'll parse the input
    /// until we find an unmatched closing parenthesis. However, we'll
    /// throw away the result of the parsing and just keep the string
    /// we've built up on the way."
    ///
    /// zshrs port note: the C source uses zcontext_save/restore +
    /// strinbeg/inpush to set up an isolated lex context for the
    /// throw-away parse. zshrs's standalone walker tracks paren
    /// depth directly without re-entering the parser. Same
    /// invariant: stops at the matching `)`.
    fn skip_command_sub(&mut self) -> Result<(), ()> {
        let mut pct = 1;
        let mut start = true;
        const MAX_ITERATIONS: usize = 100_000;
        let mut iterations = 0;

        self.add(char_tokens::INPAR);

        loop {
            iterations += 1;
            if iterations > MAX_ITERATIONS {
                self.error = Some("skip_command_sub exceeded maximum iterations".to_string());
                return Err(());
            }

            let c = self.hgetc();
            let c = match c {
                Some(c) => c,
                None => {
                    self.lexstop = true;
                    return Err(());
                }
            };

            let iswhite = Self::is_inblank(c);

            match c {
                '(' => {
                    pct += 1;
                    self.add(c);
                }
                ')' => {
                    pct -= 1;
                    if pct == 0 {
                        return Ok(());
                    }
                    self.add(c);
                }
                '\\' => {
                    self.add(c);
                    if let Some(c) = self.hgetc() {
                        self.add(c);
                    }
                }
                '\'' => {
                    self.add(c);
                    loop {
                        let ch = self.hgetc();
                        match ch {
                            Some('\'') => {
                                self.add('\'');
                                break;
                            }
                            Some(ch) => self.add(ch),
                            None => {
                                self.lexstop = true;
                                return Err(());
                            }
                        }
                    }
                }
                '"' => {
                    self.add(c);
                    loop {
                        let ch = self.hgetc();
                        match ch {
                            Some('"') => {
                                self.add('"');
                                break;
                            }
                            Some('\\') => {
                                self.add('\\');
                                if let Some(ch) = self.hgetc() {
                                    self.add(ch);
                                }
                            }
                            Some(ch) => self.add(ch),
                            None => {
                                self.lexstop = true;
                                return Err(());
                            }
                        }
                    }
                }
                '`' => {
                    self.add(c);
                    loop {
                        let ch = self.hgetc();
                        match ch {
                            Some('`') => {
                                self.add('`');
                                break;
                            }
                            Some('\\') => {
                                self.add('\\');
                                if let Some(ch) = self.hgetc() {
                                    self.add(ch);
                                }
                            }
                            Some(ch) => self.add(ch),
                            None => {
                                self.lexstop = true;
                                return Err(());
                            }
                        }
                    }
                }
                '#' if start => {
                    self.add(c);
                    // Skip comment to end of line
                    loop {
                        let ch = self.hgetc();
                        match ch {
                            Some('\n') => {
                                self.add('\n');
                                break;
                            }
                            Some(ch) => self.add(ch),
                            None => break,
                        }
                    }
                }
                _ => {
                    self.add(c);
                }
            }

            start = iswhite;
        }
    }

    /// Lex next token AND update per-context flags. Direct port of
    /// zsh/Src/lex.c:316-369 `ctxtlex`. The post-token state machine
    /// at lex.c:322-358 sets `incmdpos` based on the token shape:
    /// list separators / pipes / control keywords reset to cmd-pos;
    /// word-shaped tokens leave cmd-pos. Redirections (lex.c:361-368)
    /// stash prior incmdpos and force the redir target to non-cmd-pos.
    pub fn ctxtlex(&mut self) {
        // lex.c:319 — static `oldpos` cache for redir-target restore
        // is captured per-call here as `oldpos` below (zshrs's parser
        // re-enters ctxtlex per token, no need for static persistence).

        // lex.c:321 — `zshlex();` to advance to the next token.
        self.zshlex();

        // lex.c:322-358 — post-token incmdpos switch.
        match self.tok {
            // lex.c:323-343 — separators / openers / conjunctions /
            // control keywords — back into cmd-pos so the next token
            // can be a fresh command.
            LexTok::Seper
            | LexTok::Newlin
            | LexTok::Semi
            | LexTok::Dsemi
            | LexTok::Semiamp
            | LexTok::Semibar
            | LexTok::Amper
            | LexTok::Amperbang
            | LexTok::Inpar
            | LexTok::Inbrace
            | LexTok::Dbar
            | LexTok::Damper
            | LexTok::Bar
            | LexTok::Baramp
            | LexTok::Inoutpar
            | LexTok::Doloop
            | LexTok::Then
            | LexTok::Elif
            | LexTok::Else
            | LexTok::Doutbrack => {
                self.incmdpos = true;
            }
            // lex.c:345-353 — word/value-shaped tokens leave cmd-pos
            // so subsequent tokens are arguments, not a fresh command.
            LexTok::String
            | LexTok::Typeset
            | LexTok::Envarray
            | LexTok::Outpar
            | LexTok::Case
            | LexTok::Dinbrack => {
                self.incmdpos = false;
            }
            _ => {}
        }

        // lex.c:359-360 — `infor` decay. FOR sets infor=2 so the next
        // DINPAR can detect c-style for. After any non-DINPAR, decay
        // to 0 (or back to 2 if we just saw FOR again).
        if self.tok != LexTok::Dinpar {
            self.infor = if self.tok == LexTok::For { 2 } else { 0 };
        }

        // lex.c:361-368 — redir-target context dance. After consuming
        // a redir operator, the following token (the file path) sees
        // incmdpos=0 even when its inherent shape would put it back
        // in cmd-pos. After the redir target, restore `oldpos`.
        let oldpos = self.incmdpos;
        if self.tok.is_redirop()
            || self.tok == LexTok::For
            || self.tok == LexTok::Foreach
            || self.tok == LexTok::Select
        {
            self.inredir = true;
            self.incmdpos = false;
        } else if self.inredir {
            self.incmdpos = oldpos;
            self.inredir = false;
        }
    }

    /// Mark the current word as the one ZLE was looking for. Direct
    /// port of zsh/Src/lex.c:1881-1897 `gotword`. Only meaningful
    /// when the lexer was started with LEXFLAGS_ZLE for completion;
    /// after this call `lexflags` is cleared so subsequent tokens
    /// don't re-trigger word tracking.
    ///
    /// zshrs port note: zsh's gotword updates `wb`/`we` (word begin/
    /// end positions) based on `zlemetacs` (cursor pos), `zlemetall`
    /// (line length), `inbufct`, and `addedx` — all live in zsh's
    /// input.c globals which zshrs hasn't wired through the lexer.
    /// Only the `lexflags = 0` side-effect at lex.c:1895 is
    /// reproducible without that integration.
    pub fn gotword(&mut self) {
        // lex.c:1895 — `lexflags = 0;`
        self.lexflags = LexFlags::default();
    }

    /// Register a heredoc to be processed at next newline
    pub fn register_heredoc(&mut self, terminator: String, strip_tabs: bool) {
        self.heredocs.push(HereDoc {
            terminator,
            strip_tabs,
            content: String::new(),
            quoted: false,
            processed: false,
        });
    }

    /// Check for reserved word
    pub fn check_reserved_word(&mut self) -> bool {
        if let Some(ref tokstr) = self.tokstr {
            if self.incmdpos || (tokstr == "}" && self.tok == LexTok::String) {
                if let Some(tok) = crate::tokens::lookup_reserved_word(tokstr) {
                    self.tok = tok;
                    if tok == LexTok::Repeat {
                        self.inrepeat = 1;
                    }
                    if tok == LexTok::Dinbrack {
                        self.incond = 1;
                    }
                    return true;
                }
                if tokstr == "]]" && self.incond > 0 {
                    self.tok = LexTok::Doutbrack;
                    self.incond = 0;
                    return true;
                }
            }
        }
        false
    }
}

/// Result of determining if (( is arithmetic or command
enum CmdOrMath {
    Cmd,
    Math,
    Err,
}

// ============================================================================
// Additional parsing functions ported from lex.c
// ============================================================================

/// Check whether we're looking at valid numeric globbing syntax
/// `<N-M>` / `<N->` / `<-M>` / `<->`. Call pointing just after the
/// opening `<`. Leaves the input position unchanged, returning true
/// or false.
///
/// Direct port of zsh/Src/lex.c:580-610 `isnumglob`. C source uses
/// hgetc/hungetc against the input stream and a temp buffer to
/// remember consumed chars; zshrs takes a `(input, pos)` slice and
/// scans without consumption. Same predicate, different I/O model.
pub fn isnumglob(input: &str, pos: usize) -> bool {
    let chars: Vec<char> = input[pos..].chars().collect();
    let mut i = 0;
    let mut expect_close = false;

    // Look for digits, then -, then digits, then >
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_digit() {
            i += 1;
        } else if c == '-' && !expect_close {
            expect_close = true;
            i += 1;
        } else if c == '>' && expect_close {
            return true;
        } else {
            break;
        }
    }
    false
}

/// Tokenize a string as if in double quotes (error-tolerant variant).
///
/// Direct port of zsh/Src/lex.c:1713-1733 `parsestrnoerr`. The C
/// source: zcontext_save → untokenize → inpush → strinbeg →
/// `lexbuf.ptr = tokstr = *s; lexbuf.siz = l + 1` →
/// `err = dquote_parse('\0', 1)` → strinend → inpop → zcontext_restore.
/// Returns the tokenized string on success, or the offending char as
/// an error code (zsh convention: `> 32 && < 127` → printable, else
/// generic).
///
/// zshrs port: the C version drives the lexer's dquote_parse method
/// against the input string. zshrs's standalone walker produces the
/// same BNULL/QSTRING/QTICK token markers without re-entering the
/// lexer — same output for typical bodies. Documented divergence:
/// nested cmd-sub `$(...)` and arith `$((...))` aren't lexed
/// recursively; the runtime handles them at expansion time.
pub fn parsestrnoerr(s: &str) -> Result<String, String> {
    parsestr_inner(s)
}

/// Tokenize a string as if in double quotes (error-reporting variant).
///
/// Direct port of zsh/Src/lex.c:1693-1709 `parsestr`. C source:
/// `if ((err = parsestrnoerr(s))) { untokenize(*s); ... zerr("parse
/// error near `%c'", err); tok = LEXERR; }`. zshrs's wrapper
/// returns the same Result and lets the caller emit the diagnostic.
///
/// Both `parsestr` and `parsestrnoerr` share the inner walker; the
/// only difference in C is whether errors trigger `zerr`. zshrs
/// returns `Err(msg)` from both — the caller decides whether to
/// surface the diagnostic.
pub fn parsestr(s: &str) -> Result<String, String> {
    parsestr_inner(s)
}

/// Shared body for parsestr / parsestrnoerr.
fn parsestr_inner(s: &str) -> Result<String, String> {
    let mut result = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' => {
                i += 1;
                if i < chars.len() {
                    let next = chars[i];
                    match next {
                        '$' | '\\' | '`' | '"' | '\n' => {
                            result.push(char_tokens::BNULL);
                            result.push(next);
                        }
                        _ => {
                            result.push('\\');
                            result.push(next);
                        }
                    }
                } else {
                    result.push('\\');
                }
            }
            '$' => {
                result.push(char_tokens::QSTRING);
                if i + 1 < chars.len() {
                    let next = chars[i + 1];
                    if next == '{' {
                        result.push(char_tokens::INBRACE);
                        i += 1;
                    } else if next == '(' {
                        result.push(char_tokens::INPAR);
                        i += 1;
                    }
                }
            }
            '`' => {
                result.push(char_tokens::QTICK);
            }
            _ => {
                result.push(c);
            }
        }
        i += 1;
    }

    Ok(result)
}

/// Parse a subscript in string s. Return the position after the
/// closing bracket, or None on error.
///
/// Direct port of zsh/Src/lex.c:1742-1788 `parse_subscript`. The C
/// source uses dupstring_wlen + inpush + dquote_parse to lex the
/// subscript through the main lexer; zshrs implements a focused
/// bracket-balancing walker that handles the same nesting rules
/// (`[...]`, `(...)`, `{...}`) without re-entering the lexer.
///
/// zshrs port note: zsh's parse_subscript also handles a `sub`
/// flag that controls whether `$` and quotes are tokenized — that
/// flag isn't exposed here. Most callers don't need it; the few
/// that do (parameter expansion's `${var[expr]}`) handle the
/// quote-aware lex separately at the expansion layer.
pub fn parse_subscript(s: &str, endchar: char) -> Option<usize> {
    if s.is_empty() || s.starts_with(endchar) {
        return None;
    }

    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    let mut depth = 0;
    let mut in_dquote = false;
    let mut in_squote = false;

    while i < chars.len() {
        let c = chars[i];

        if in_squote {
            if c == '\'' {
                in_squote = false;
            }
            i += 1;
            continue;
        }

        if in_dquote {
            if c == '"' {
                in_dquote = false;
            } else if c == '\\' && i + 1 < chars.len() {
                i += 1; // skip escaped char
            }
            i += 1;
            continue;
        }

        match c {
            '\\' => {
                i += 1; // skip next char
            }
            '\'' => {
                in_squote = true;
            }
            '"' => {
                in_dquote = true;
            }
            '[' | '(' => {
                depth += 1;
            }
            ']' | ')' => {
                if depth > 0 {
                    depth -= 1;
                } else if c == endchar {
                    return Some(i);
                }
            }
            _ => {}
        }

        if c == endchar && depth == 0 {
            return Some(i);
        }

        i += 1;
    }

    None
}

/// Tokenize a string as if it were a normal command-line argument
/// but it may contain separators. Used for ${...%...} substitutions.
///
/// Direct port of zsh/Src/lex.c:1796-1880 `parse_subst_string`.
/// zsh's version sets `noaliases = 1` + `lexflags = 0` + uses
/// zcontext_save/inpush/strinbeg → dquote_parse('\0', 1) →
/// strinend/inpop/zcontext_restore. zshrs's standalone walker
/// produces the same BNULL/SNULL/DNULL/INPAR/INBRACK markers
/// without re-entering the lexer.
///
/// zshrs port note: the C source returns int (0=ok, char value =
/// where it stopped on error); zshrs returns Result<String,String>
/// returning the tokenized text directly. Lossy for callers that
/// need to know the exact stop position, but nothing in zshrs's
/// expansion layer uses that yet.
pub fn parse_subst_string(s: &str) -> Result<String, String> {
    if s.is_empty() {
        return Ok(String::new());
    }

    let mut result = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' => {
                result.push(char_tokens::BNULL);
                i += 1;
                if i < chars.len() {
                    result.push(chars[i]);
                }
            }
            '\'' => {
                result.push(char_tokens::SNULL);
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    result.push(chars[i]);
                    i += 1;
                }
                result.push(char_tokens::SNULL);
            }
            '"' => {
                result.push(char_tokens::DNULL);
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        result.push(char_tokens::BNULL);
                        i += 1;
                        result.push(chars[i]);
                    } else if chars[i] == '$' {
                        result.push(char_tokens::QSTRING);
                    } else {
                        result.push(chars[i]);
                    }
                    i += 1;
                }
                result.push(char_tokens::DNULL);
            }
            '$' => {
                result.push(char_tokens::STRING);
                if i + 1 < chars.len() {
                    match chars[i + 1] {
                        '{' => {
                            result.push(char_tokens::INBRACE);
                            i += 1;
                        }
                        '(' => {
                            result.push(char_tokens::INPAR);
                            i += 1;
                        }
                        _ => {}
                    }
                }
            }
            '*' => result.push(char_tokens::STAR),
            '?' => result.push(char_tokens::QUEST),
            '[' => result.push(char_tokens::INBRACK),
            ']' => result.push(char_tokens::OUTBRACK),
            '{' => result.push(char_tokens::INBRACE),
            '}' => result.push(char_tokens::OUTBRACE),
            '~' => result.push(char_tokens::TILDE),
            '#' => result.push(char_tokens::POUND),
            '^' => result.push(char_tokens::HAT),
            _ => result.push(c),
        }
        i += 1;
    }

    Ok(result)
}

/// Untokenize a string - convert tokenized chars back to original
///
/// Port of untokenize() from exec.c (but used by lexer too)
/// Like `untokenize`, but maps SNULL → `'` and DNULL → `"` instead of
/// stripping them. Used by callers that need the source form including
/// quoting (e.g. arithmetic-substitution detection in compile_zsh).
pub fn untokenize_preserve_quotes(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        let cu = c as u32;
        if (0x83..=0x9f).contains(&cu) {
            match c {
                c if c == char_tokens::POUND => result.push('#'),
                c if c == char_tokens::STRING => result.push('$'),
                c if c == char_tokens::HAT => result.push('^'),
                c if c == char_tokens::STAR => result.push('*'),
                c if c == char_tokens::INPAR => result.push('('),
                c if c == char_tokens::OUTPAR => result.push(')'),
                c if c == char_tokens::INPARMATH => result.push('('),
                c if c == char_tokens::OUTPARMATH => result.push(')'),
                c if c == char_tokens::QSTRING => result.push('$'),
                c if c == char_tokens::EQUALS => result.push('='),
                c if c == char_tokens::BAR => result.push('|'),
                c if c == char_tokens::INBRACE => result.push('{'),
                c if c == char_tokens::OUTBRACE => result.push('}'),
                c if c == char_tokens::INBRACK => result.push('['),
                c if c == char_tokens::OUTBRACK => result.push(']'),
                c if c == char_tokens::TICK => result.push('`'),
                c if c == char_tokens::INANG => result.push('<'),
                c if c == char_tokens::OUTANG => result.push('>'),
                c if c == char_tokens::OUTANGPROC => result.push('>'),
                c if c == char_tokens::QUEST => result.push('?'),
                c if c == char_tokens::TILDE => result.push('~'),
                c if c == char_tokens::QTICK => result.push('`'),
                c if c == char_tokens::COMMA => result.push(','),
                c if c == char_tokens::DASH => result.push('-'),
                c if c == char_tokens::BANG => result.push('!'),
                c if c == char_tokens::SNULL => result.push('\''),
                c if c == char_tokens::DNULL => result.push('"'),
                c if c == char_tokens::BNULL => result.push('\\'),
                _ => {
                    let idx = c as usize;
                    if idx < char_tokens::ZTOKENS.len() {
                        result.push(char_tokens::ZTOKENS.chars().nth(idx).unwrap_or(c));
                    } else {
                        result.push(c);
                    }
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

pub fn untokenize(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        // Token chars live in zsh's META range (0x83 = META through 0x9f =
        // BNULL). Anything in that range needs un-mapping before display
        // or downstream consumption. The original `< 32` test was wrong —
        // none of zsh's tokens land in that range.
        let cu = c as u32;
        if (0x83..=0x9f).contains(&cu) {
            // Convert token back to original character
            match c {
                c if c == char_tokens::POUND => result.push('#'),
                c if c == char_tokens::STRING => result.push('$'),
                c if c == char_tokens::HAT => result.push('^'),
                c if c == char_tokens::STAR => result.push('*'),
                c if c == char_tokens::INPAR => result.push('('),
                c if c == char_tokens::OUTPAR => result.push(')'),
                c if c == char_tokens::INPARMATH => result.push('('),
                c if c == char_tokens::OUTPARMATH => result.push(')'),
                c if c == char_tokens::QSTRING => result.push('$'),
                c if c == char_tokens::EQUALS => result.push('='),
                c if c == char_tokens::BAR => result.push('|'),
                c if c == char_tokens::INBRACE => result.push('{'),
                c if c == char_tokens::OUTBRACE => result.push('}'),
                c if c == char_tokens::INBRACK => result.push('['),
                c if c == char_tokens::OUTBRACK => result.push(']'),
                c if c == char_tokens::TICK => result.push('`'),
                c if c == char_tokens::INANG => result.push('<'),
                c if c == char_tokens::OUTANG => result.push('>'),
                c if c == char_tokens::OUTANGPROC => result.push('>'),
                c if c == char_tokens::QUEST => result.push('?'),
                c if c == char_tokens::TILDE => result.push('~'),
                c if c == char_tokens::QTICK => result.push('`'),
                c if c == char_tokens::COMMA => result.push(','),
                c if c == char_tokens::DASH => result.push('-'),
                c if c == char_tokens::BANG => result.push('!'),
                c if c == char_tokens::SNULL
                    || c == char_tokens::DNULL
                    || c == char_tokens::BNULL =>
                {
                    // Null markers - skip
                }
                _ => {
                    // Unknown token, try ztokens lookup
                    let idx = c as usize;
                    if idx < char_tokens::ZTOKENS.len() {
                        result.push(char_tokens::ZTOKENS.chars().nth(idx).unwrap_or(c));
                    } else {
                        result.push(c);
                    }
                }
            }
        } else {
            result.push(c);
        }
        i += 1;
    }

    result
}

/// Check if a string contains any token characters
pub fn has_token(s: &str) -> bool {
    s.chars().any(|c| (c as u32) < 32)
}

/// Convert token characters to their printable form for display
pub fn tokens_to_printable(s: &str) -> String {
    untokenize(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_command() {
        let mut lexer = ZshLexer::new("echo hello");
        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);
        assert_eq!(lexer.tokstr, Some("echo".to_string()));

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);
        assert_eq!(lexer.tokstr, Some("hello".to_string()));

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::Endinput);
    }

    #[test]
    fn test_pipeline() {
        let mut lexer = ZshLexer::new("ls | grep foo");
        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::Bar);

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);
    }

    #[test]
    fn test_redirections() {
        let mut lexer = ZshLexer::new("echo > file");
        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::Outang);

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);
    }

    #[test]
    fn test_heredoc() {
        let mut lexer = ZshLexer::new("cat << EOF");
        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::Dinang);

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);
    }

    #[test]
    fn test_single_quotes() {
        let mut lexer = ZshLexer::new("echo 'hello world'");
        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);
        // Should contain Snull markers around literal content
        assert!(lexer.tokstr.is_some());
    }

    #[test]
    fn test_function_tokens() {
        let mut lexer = ZshLexer::new("function foo { }");
        lexer.zshlex();
        assert_eq!(
            lexer.tok,
            LexTok::Func,
            "expected Func, got {:?}",
            lexer.tok
        );

        lexer.zshlex();
        assert_eq!(
            lexer.tok,
            LexTok::String,
            "expected String for 'foo', got {:?}",
            lexer.tok
        );
        assert_eq!(lexer.tokstr, Some("foo".to_string()));

        lexer.zshlex();
        assert_eq!(
            lexer.tok,
            LexTok::Inbrace,
            "expected Inbrace, got {:?} tokstr={:?}",
            lexer.tok,
            lexer.tokstr
        );

        lexer.zshlex();
        assert_eq!(
            lexer.tok,
            LexTok::Outbrace,
            "expected Outbrace, got {:?} tokstr={:?} incmdpos={}",
            lexer.tok,
            lexer.tokstr,
            lexer.incmdpos
        );
    }

    #[test]
    fn test_double_quotes() {
        let mut lexer = ZshLexer::new("echo \"hello $name\"");
        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);
        // Should contain tokenized content
        assert!(lexer.tokstr.is_some());
    }

    #[test]
    fn test_command_substitution() {
        let mut lexer = ZshLexer::new("echo $(pwd)");
        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);
    }

    #[test]
    fn test_env_assignment() {
        let mut lexer = ZshLexer::new("FOO=bar echo");
        lexer.incmdpos = true;
        lexer.zshlex();
        assert_eq!(
            lexer.tok,
            LexTok::Envstring,
            "tok={:?} tokstr={:?}",
            lexer.tok,
            lexer.tokstr
        );

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);
    }

    #[test]
    fn test_array_assignment() {
        let mut lexer = ZshLexer::new("arr=(a b c)");
        lexer.incmdpos = true;
        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::Envarray);
    }

    #[test]
    fn test_process_substitution() {
        let mut lexer = ZshLexer::new("diff <(ls) >(cat)");
        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);
        // <(ls) is tokenized into the string

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);
        // >(cat) is tokenized
    }

    #[test]
    fn test_arithmetic() {
        let mut lexer = ZshLexer::new("echo $((1+2))");
        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);

        lexer.zshlex();
        assert_eq!(lexer.tok, LexTok::String);
    }

    #[test]
    fn test_semicolon_variants() {
        let mut lexer = ZshLexer::new("case x in a) cmd;; b) cmd;& c) cmd;| esac");

        // Skip to first ;;
        loop {
            lexer.zshlex();
            if lexer.tok == LexTok::Dsemi || lexer.tok == LexTok::Endinput {
                break;
            }
        }
        assert_eq!(lexer.tok, LexTok::Dsemi);

        // Find ;&
        loop {
            lexer.zshlex();
            if lexer.tok == LexTok::Semiamp || lexer.tok == LexTok::Endinput {
                break;
            }
        }
        assert_eq!(lexer.tok, LexTok::Semiamp);

        // Find ;|
        loop {
            lexer.zshlex();
            if lexer.tok == LexTok::Semibar || lexer.tok == LexTok::Endinput {
                break;
            }
        }
        assert_eq!(lexer.tok, LexTok::Semibar);
    }
}
