//! AST-walked symbol table for the zshrs LSP server. Powers cross-file
//! rename and find-references with parser-validated scoping instead of
//! the textual occurrence scan in `crate::extensions::lsp::references`.
//!
//! Mirrors the shape of `strykelang/lsp_symbols.rs` but specialized to
//! zsh's AST (`ZshFuncDef` / `ZshSimple` / `ZshAssign`). Functions and
//! top-level variable assignments are the two symbol kinds covered;
//! `local` / `typeset` declarations inside function bodies are also
//! captured as scoped locals (decl line known, scope = function name).
//!
//! Architecture:
//!
//! * [`SymbolTable::build`] parses the source with the same lexer +
//!   parser the LSP diagnostics path uses, walks the resulting
//!   `ZshProgram`, and produces:
//!     - one [`Symbol`] per declaration (function or variable)
//!     - one [`SymbolRef`] per parser-identified textual occurrence
//! * AST nodes only carry per-pipe `lineno`. To turn a `(line, name)`
//!   pair into an LSP `Range`, we re-scan the line text for the name —
//!   the same trick the stryke side uses.
//!
//! The textual fallback already in `lsp.rs` is kept; this module gives
//! the LSP the means to PREFER AST-derived ranges when the parse
//! succeeds, and only fall back to plain text when it fails (e.g. user
//! is mid-edit).

use std::collections::HashMap;

use crate::extensions::zsh_ast::{
    ZshAssign, ZshAssignValue, ZshCommand, ZshList, ZshPipe, ZshProgram, ZshSimple, ZshSublist,
};

/// Stable handle to a declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SymbolId(pub u32);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SymbolKind {
    /// `function name { ... }` or `name() { ... }`.
    Func,
    /// Top-level assignment `name=value`.
    Global,
    /// `local`/`typeset` inside a function body.
    Local,
}

#[derive(Clone, Debug)]
pub struct Symbol {
    pub id: SymbolId,
    pub name: String,
    pub kind: SymbolKind,
    /// 0-based source line of the declaration.
    pub decl_line: u32,
}

#[derive(Clone, Debug)]
pub struct SymbolRef {
    pub symbol: SymbolId,
    pub line: u32,
    pub name: String,
}

pub struct SymbolTable {
    pub symbols: Vec<Symbol>,
    pub refs: Vec<SymbolRef>,
}

impl SymbolTable {
    /// Build the table from `src`. Returns `None` if the parser fails.
    /// Callers should fall back to the textual scan in that case.
    pub fn build(src: &str) -> Option<Self> {
        // The zshrs parser uses thread-local lexer state — guard with
        // the same lock the diagnostic path uses elsewhere so concurrent
        // LSP requests don't trample each other's parse state.
        let prog = parse_locked(src)?;
        let mut builder = Builder::default();
        builder.walk_program(&prog);
        Some(SymbolTable {
            symbols: builder.symbols,
            refs: builder.refs,
        })
    }

    /// Symbol resolved at `(line, needle)`. Bare names match qualified
    /// stored names by tail (so `greet` resolves to a func declared
    /// without package qualification — same semantics as stryke).
    pub fn symbol_at(&self, line: u32, needle: &str) -> Option<SymbolId> {
        // Refs first (they're at the cursor more often than decls).
        for r in &self.refs {
            if r.line == line && r.name == needle {
                return Some(r.symbol);
            }
        }
        for s in &self.symbols {
            if s.decl_line == line && s.name == needle {
                return Some(s.id);
            }
        }
        None
    }

    /// Every (line, name) pair belonging to `id` — declaration + refs.
    pub fn occurrences(&self, id: SymbolId) -> Vec<(u32, String)> {
        let mut out: Vec<(u32, String)> = Vec::new();
        if let Some(sym) = self.symbols.iter().find(|s| s.id == id) {
            out.push((sym.decl_line, sym.name.clone()));
        }
        for r in &self.refs {
            if r.symbol == id {
                out.push((r.line, r.name.clone()));
            }
        }
        out
    }
}

/// AST-walk `src` looking for occurrences of `name` in contexts that
/// match `kind`. Returns (line, name) pairs the LSP can use for
/// cross-file rename / find-refs. Unlike [`SymbolTable::build`] this
/// doesn't require `name` to be locally declared — it answers
/// "if I rename `daemon-ping` (a function defined elsewhere), where
/// are the call sites IN THIS FILE?"
///
/// Caller resolves columns by re-scanning each line for `name`
/// (same trick `SymbolTable` itself uses — see the module-doc).
///
/// Kind semantics:
/// * `Func`   — `FuncDef.names` matching `name`, plus first-word
///              command-position uses of `name` (stripping wrappers
///              like `builtin` / `command` / `noglob` / `nocorrect`).
/// * `Global` — top-level `Assigns` writing `name`, plus `$name` /
///              `${name}` parameter refs anywhere.
/// * `Local`  — same expansion sites as Global. Cross-file callers
///              should skip Local; locals don't cross file boundaries.
pub fn find_ast_occurrences(src: &str, name: &str, kind: SymbolKind) -> Vec<u32> {
    let Some(prog) = parse_locked(src) else {
        return Vec::new();
    };
    let mut finder = OccurrenceFinder {
        target: name,
        kind,
        out: Vec::new(),
    };
    finder.walk_program(&prog);
    finder.out
}

/// AST-walk `src` looking for `source X` / `. X` / `zsource X`
/// commands. Resolves each X against `base_dir` (the dir of the
/// file being walked) and returns the absolute paths that actually
/// exist on disk.
///
/// Path resolution:
///   * Leading `~/...` — expanded against `$HOME`.
///   * Absolute (`/`, `~`) — taken as-is.
///   * Relative — joined to `base_dir`.
///
/// Skipped (intentionally — they require shell runtime state we
/// don't have at LSP time):
///   * `$ENV/X` / `${PATH_DIRS}/X` expansion.
///   * `$fpath`-relative autoload resolution.
///   * Glob expansion (`source ~/*.zsh`).
///
/// Used by `references` and `rename` for source-chain following —
/// the active file's deps get walked even when they live outside
/// the LSP workspace root (e.g. `source ~/.zshrc` from a project
/// `init.zsh`).
pub fn collect_sourced_paths(src: &str, base_dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Some(prog) = parse_locked(src) else {
        return Vec::new();
    };
    let mut finder = SourceFinder {
        out: Vec::new(),
    };
    finder.walk_program(&prog);
    // Resolve every collected path against `base_dir` + $HOME.
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let mut resolved: Vec<std::path::PathBuf> = Vec::new();
    for raw in finder.out {
        let untok = crate::ported::lex::untokenize(&raw);
        // Drop quoting if present (parser keeps literal text).
        let trimmed = untok.trim_matches(|c| c == '"' || c == '\'');
        if trimmed.is_empty() {
            continue;
        }
        let path = if let Some(rest) = trimmed.strip_prefix("~/") {
            match &home {
                Some(h) => h.join(rest),
                None => continue,
            }
        } else if trimmed.starts_with('/') {
            std::path::PathBuf::from(trimmed)
        } else if trimmed.contains('$') {
            // Skip env-var paths we can't resolve at LSP time.
            continue;
        } else {
            base_dir.join(trimmed)
        };
        // Canonicalize when possible so symlinks dedup; fall back to
        // the original path otherwise (canonicalize fails on missing
        // files, which we DO want to skip).
        if let Ok(canon) = std::fs::canonicalize(&path) {
            resolved.push(canon);
        }
    }
    resolved
}

struct SourceFinder {
    out: Vec<String>,
}

impl SourceFinder {
    fn walk_program(&mut self, p: &ZshProgram) {
        for list in &p.lists {
            self.walk_sublist(&list.sublist);
        }
    }
    fn walk_sublist(&mut self, sl: &ZshSublist) {
        self.walk_pipe(&sl.pipe);
        if let Some((_, next)) = &sl.next {
            self.walk_sublist(next);
        }
    }
    fn walk_pipe(&mut self, pipe: &ZshPipe) {
        self.walk_command(&pipe.cmd);
        if let Some(next) = &pipe.next {
            self.walk_pipe(next);
        }
    }
    fn walk_command(&mut self, cmd: &ZshCommand) {
        match cmd {
            ZshCommand::Simple(s) => {
                // `source X` / `. X` / `zsource X` — first word matches
                // one of the source-loader spellings, second word is
                // the path.
                if let Some(verb) = s.words.first() {
                    let v = crate::ported::lex::untokenize(verb);
                    if matches!(v.as_str(), "source" | "." | "zsource") {
                        if let Some(arg) = s.words.get(1) {
                            self.out.push(arg.clone());
                        }
                    }
                }
            }
            ZshCommand::FuncDef(fd) => self.walk_program(&fd.body),
            ZshCommand::Subsh(p) | ZshCommand::Cursh(p) => self.walk_program(p),
            ZshCommand::For(f) => self.walk_program(&f.body),
            ZshCommand::Case(c) => {
                for arm in &c.arms {
                    self.walk_program(&arm.body);
                }
            }
            ZshCommand::If(iff) => {
                self.walk_program(&iff.cond);
                self.walk_program(&iff.then);
                for (cond, body) in &iff.elif {
                    self.walk_program(cond);
                    self.walk_program(body);
                }
                if let Some(els) = &iff.else_ {
                    self.walk_program(els);
                }
            }
            ZshCommand::While(w) | ZshCommand::Until(w) => {
                self.walk_program(&w.cond);
                self.walk_program(&w.body);
            }
            ZshCommand::Repeat(r) => self.walk_program(&r.body),
            ZshCommand::Try(t) => {
                self.walk_program(&t.try_block);
                self.walk_program(&t.always);
            }
            ZshCommand::Redirected(inner, _) => self.walk_command(inner),
            ZshCommand::Time(Some(sl)) => self.walk_sublist(sl),
            ZshCommand::Time(None) | ZshCommand::Cond(_) | ZshCommand::Arith(_) => {}
        }
    }
}

struct OccurrenceFinder<'a> {
    target: &'a str,
    kind: SymbolKind,
    out: Vec<u32>,
}

impl<'a> OccurrenceFinder<'a> {
    fn walk_program(&mut self, p: &ZshProgram) {
        for list in &p.lists {
            self.walk_sublist(&list.sublist);
        }
    }
    fn walk_sublist(&mut self, sl: &ZshSublist) {
        self.walk_pipe(&sl.pipe);
        if let Some((_, next)) = &sl.next {
            self.walk_sublist(next);
        }
    }
    fn walk_pipe(&mut self, pipe: &ZshPipe) {
        let line0 = pipe.lineno.saturating_sub(1) as u32;
        self.walk_command(&pipe.cmd, line0);
        if let Some(next) = &pipe.next {
            self.walk_pipe(next);
        }
    }
    fn walk_command(&mut self, cmd: &ZshCommand, line: u32) {
        match cmd {
            ZshCommand::FuncDef(fd) => {
                if matches!(self.kind, SymbolKind::Func) {
                    for n in &fd.names {
                        // Names from the parser carry zsh internal
                        // token bytes (e.g. `Dash` 0x9b for `-`). The
                        // canonical char-level untokenize maps those
                        // back to the printable form so `daemon-ping`
                        // round-trips. Without this `n == target`
                        // misses any name containing `-`, `*`, `?` etc.
                        let nu = crate::ported::lex::untokenize(n);
                        if nu == self.target {
                            self.out.push(line);
                        }
                    }
                }
                self.walk_program(&fd.body);
            }
            ZshCommand::Simple(s) => self.walk_simple(s, line),
            ZshCommand::Subsh(p) | ZshCommand::Cursh(p) => self.walk_program(p),
            ZshCommand::For(f) => self.walk_program(&f.body),
            ZshCommand::Case(c) => {
                for arm in &c.arms {
                    self.walk_program(&arm.body);
                }
            }
            ZshCommand::If(iff) => {
                self.walk_program(&iff.cond);
                self.walk_program(&iff.then);
                for (cond, body) in &iff.elif {
                    self.walk_program(cond);
                    self.walk_program(body);
                }
                if let Some(els) = &iff.else_ {
                    self.walk_program(els);
                }
            }
            ZshCommand::While(w) | ZshCommand::Until(w) => {
                self.walk_program(&w.cond);
                self.walk_program(&w.body);
            }
            ZshCommand::Repeat(r) => self.walk_program(&r.body),
            ZshCommand::Try(t) => {
                self.walk_program(&t.try_block);
                self.walk_program(&t.always);
            }
            ZshCommand::Redirected(inner, _) => self.walk_command(inner, line),
            ZshCommand::Time(Some(sl)) => self.walk_sublist(sl),
            ZshCommand::Time(None) | ZshCommand::Cond(_) | ZshCommand::Arith(_) => {}
        }
    }
    fn walk_simple(&mut self, s: &ZshSimple, line: u32) {
        // Globals / Locals: leading `x=value` assignments.
        if matches!(self.kind, SymbolKind::Global | SymbolKind::Local) {
            for a in &s.assigns {
                let nu = crate::ported::lex::untokenize(&a.name);
                if nu == self.target {
                    self.out.push(line);
                }
                if let ZshAssignValue::Scalar(v) = &a.value {
                    self.scan_dollar_refs(v, line);
                } else if let ZshAssignValue::Array(v) = &a.value {
                    for item in v {
                        self.scan_dollar_refs(item, line);
                    }
                }
            }
            // `local NAME=…` / `typeset NAME=…` decl form.
            let is_local_kw = s
                .words
                .first()
                .map(|w| matches!(w.as_str(), "local" | "typeset" | "declare" | "private"))
                .unwrap_or(false);
            if is_local_kw {
                for w in s.words.iter().skip(1) {
                    let raw = w.as_str();
                    if raw.starts_with('-') {
                        continue;
                    }
                    let name_part = match raw.find('=') {
                        Some(i) => &raw[..i],
                        None => raw,
                    };
                    if name_part == self.target {
                        self.out.push(line);
                    }
                }
            }
        }
        // Funcs: first-word command-position use, plus alias/autoload
        // declarations (both bind callable names that the LSP treats
        // as functions for rename/find-refs).
        if matches!(self.kind, SymbolKind::Func) {
            let is_alias = s.words.first().map(|w| w.as_str() == "alias").unwrap_or(false);
            let is_autoload = s.words.first().map(|w| w.as_str() == "autoload").unwrap_or(false);
            if is_alias || is_autoload {
                for w in s.words.iter().skip(1) {
                    // Untokenize first so `=` (tokenized as Equals 0x8d)
                    // and quote markers are visible to the splitter.
                    let untok_word = crate::ported::lex::untokenize(w);
                    if untok_word.starts_with('-')
                        || (is_autoload && untok_word.starts_with('+'))
                    {
                        continue;
                    }
                    let name_part = if is_alias {
                        match untok_word.find('=') {
                            Some(i) => &untok_word[..i],
                            None => untok_word.as_str(),
                        }
                    } else {
                        untok_word.as_str()
                    };
                    if name_part == self.target {
                        self.out.push(line);
                    }
                }
            }
            if let Some(first) = s.words.first() {
                let callee = if matches!(
                    first.as_str(),
                    "local" | "typeset" | "declare" | "private" | "alias" | "autoload"
                ) {
                    None
                } else if matches!(
                    first.as_str(),
                    "builtin" | "command" | "exec" | "noglob" | "nocorrect"
                ) {
                    s.words.get(1)
                } else {
                    Some(first)
                };
                if let Some(name) = callee {
                    // Untokenize so `daemon-ping` matches the parser's
                    // `daemon\u{9b}ping` storage form.
                    let nu = crate::ported::lex::untokenize(name);
                    if nu == self.target {
                        self.out.push(line);
                    }
                }
            }
        }
        // Param refs in every word, for Global / Local kinds.
        if matches!(self.kind, SymbolKind::Global | SymbolKind::Local) {
            for w in &s.words {
                self.scan_dollar_refs(w, line);
            }
        }
    }
    fn scan_dollar_refs(&mut self, word: &str, line: u32) {
        // Char-level canonical untokenize so `$VAR` inside `"..."`
        // survives the multi-byte UTF-8 split that bit `zwc::untokenize`.
        let cleaned = crate::ported::lex::untokenize(word);
        let bytes = cleaned.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] != b'$' {
                i += 1;
                continue;
            }
            i += 1;
            if i >= bytes.len() {
                break;
            }
            let braced = bytes[i] == b'{';
            if braced {
                i += 1;
                // Skip a `${(flags)NAME...}` flag prefix. zsh parameter
                // expansion accepts a parenthesized flag group right
                // after `${` (e.g. `${(U)NAME}` upcase, `${(@f)str}`
                // split-on-newlines, `${(Pkv@)name}` indirect+kv). Walk
                // past one balanced `(...)` if present so the name
                // reader below lands on the identifier.
                if i < bytes.len() && bytes[i] == b'(' {
                    let mut depth = 1i32;
                    i += 1;
                    while i < bytes.len() && depth > 0 {
                        match bytes[i] {
                            b'(' => depth += 1,
                            b')' => depth -= 1,
                            _ => {}
                        }
                        i += 1;
                    }
                }
            }
            let start = i;
            while i < bytes.len() {
                let c = bytes[i];
                if c.is_ascii_alphanumeric() || c == b'_' {
                    i += 1;
                } else {
                    break;
                }
            }
            if i > start {
                if let Ok(name) = std::str::from_utf8(&bytes[start..i]) {
                    if name == self.target {
                        self.out.push(line);
                    }
                }
            }
            if braced {
                while i < bytes.len() && bytes[i] != b'}' {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
            }
        }
    }
}

/// Wrap the parser call in lex-state save/restore and translate parser
/// failure into `None`. Returns the parsed `ZshProgram` on success.
/// The lexer + `errflag` are shared global state (a port of zsh's C
/// globals); callers in test contexts hold `test_util::global_state_lock`
/// across SymbolTable::build to serialize, which is why
/// [`SymbolTable::build`]'s doc says "build under lock".
fn parse_locked(src: &str) -> Option<ZshProgram> {
    use crate::utils::{errflag, ERRFLAG_ERROR};
    use std::sync::atomic::Ordering;
    crate::lex::lex_init(src);
    let saved = errflag.load(Ordering::Relaxed);
    errflag.fetch_and(!ERRFLAG_ERROR, Ordering::Relaxed);
    let prog = crate::parse::parse();
    let had_err = (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0;
    errflag.store(saved, Ordering::Relaxed);
    if had_err {
        return None;
    }
    Some(prog)
}

#[derive(Default)]
struct Builder {
    symbols: Vec<Symbol>,
    refs: Vec<SymbolRef>,
    next_sym: u32,
    /// Name → id of every function declared at the program level. Used
    /// by the simple-command walker to decide whether `words[0]` is a
    /// function call (ref) vs. an external command (ignored).
    func_index: HashMap<String, SymbolId>,
    /// Name → id of every top-level Global assignment. Used by
    /// `scan_dollar_refs` to resolve `$VAR` / `${VAR…}` references
    /// back to the declared global. Without this index, Global refs
    /// across the file are silently dropped — only the decl line ends
    /// up in `occurrences(id)`, and Find Usages / Rename miss every
    /// use site.
    global_index: HashMap<String, SymbolId>,
    /// Local-decl symbols visible in the currently-walking function
    /// body. Cleared on function exit.
    local_stack: Vec<HashMap<String, SymbolId>>,
}

impl Builder {
    fn fresh_id(&mut self) -> SymbolId {
        let id = SymbolId(self.next_sym);
        self.next_sym += 1;
        id
    }

    fn declare(&mut self, name: &str, kind: SymbolKind, line: u32) -> SymbolId {
        // Names from the parser may carry zsh internal token bytes
        // (e.g. `-` → 0x9b `Dash`). Store the printable form so
        // user-visible names round-trip cleanly and `symbol_at` /
        // `func_index` / local-scope lookups hit on the user's
        // cursor word.
        let name = crate::ported::lex::untokenize(name);
        let id = self.fresh_id();
        self.symbols.push(Symbol {
            id,
            name: name.clone(),
            kind: kind.clone(),
            decl_line: line,
        });
        match kind {
            SymbolKind::Func => {
                self.func_index.insert(name.clone(), id);
            }
            SymbolKind::Global => {
                self.global_index.insert(name.clone(), id);
            }
            SymbolKind::Local => {
                if let Some(top) = self.local_stack.last_mut() {
                    top.insert(name, id);
                }
            }
        }
        id
    }

    fn record_ref(&mut self, name: &str, line: u32) {
        // Compare untokenized form so a parser-stored `daemon\u{9b}ping`
        // call-site word resolves against the func_index's printable
        // key `daemon-ping`.
        let name = crate::ported::lex::untokenize(name);
        // Locals shadow globals; check the local scope stack first.
        for scope in self.local_stack.iter().rev() {
            if let Some(&id) = scope.get(name.as_str()) {
                self.refs.push(SymbolRef {
                    symbol: id,
                    line,
                    name,
                });
                return;
            }
        }
        // Then globals (top-level `name=value` assignments).
        if let Some(&id) = self.global_index.get(name.as_str()) {
            self.refs.push(SymbolRef {
                symbol: id,
                line,
                name,
            });
            return;
        }
        // Finally functions (command-position usages).
        if let Some(&id) = self.func_index.get(name.as_str()) {
            self.refs.push(SymbolRef {
                symbol: id,
                line,
                name,
            });
        }
    }

    fn walk_program(&mut self, p: &ZshProgram) {
        for list in &p.lists {
            self.walk_list(list);
        }
    }

    fn walk_list(&mut self, list: &ZshList) {
        self.walk_sublist(&list.sublist);
    }

    fn walk_sublist(&mut self, sl: &ZshSublist) {
        self.walk_pipe(&sl.pipe);
        if let Some((_, next)) = &sl.next {
            self.walk_sublist(next);
        }
    }

    fn walk_pipe(&mut self, pipe: &ZshPipe) {
        let line0 = pipe.lineno.saturating_sub(1) as u32;
        self.walk_command(&pipe.cmd, line0);
        if let Some(next) = &pipe.next {
            self.walk_pipe(next);
        }
    }

    fn walk_command(&mut self, cmd: &ZshCommand, line: u32) {
        match cmd {
            ZshCommand::FuncDef(fd) => {
                // `function f g h { body }` declares three function
                // names sharing one body. Emit a symbol per name.
                for n in &fd.names {
                    let _id = self.declare(n, SymbolKind::Func, line);
                }
                self.local_stack.push(HashMap::new());
                self.walk_program(&fd.body);
                self.local_stack.pop();
            }
            ZshCommand::Simple(s) => self.walk_simple(s, line),
            ZshCommand::Subsh(p) | ZshCommand::Cursh(p) => self.walk_program(p),
            ZshCommand::For(f) => self.walk_program(&f.body),
            ZshCommand::Case(c) => {
                for arm in &c.arms {
                    self.walk_program(&arm.body);
                }
            }
            ZshCommand::If(iff) => {
                self.walk_program(&iff.cond);
                self.walk_program(&iff.then);
                for (cond, body) in &iff.elif {
                    self.walk_program(cond);
                    self.walk_program(body);
                }
                if let Some(els) = &iff.else_ {
                    self.walk_program(els);
                }
            }
            ZshCommand::While(w) | ZshCommand::Until(w) => {
                self.walk_program(&w.cond);
                self.walk_program(&w.body);
            }
            ZshCommand::Repeat(r) => self.walk_program(&r.body),
            ZshCommand::Try(t) => {
                self.walk_program(&t.try_block);
                self.walk_program(&t.always);
            }
            ZshCommand::Redirected(inner, _) => self.walk_command(inner, line),
            ZshCommand::Time(Some(sl)) => self.walk_sublist(sl),
            ZshCommand::Time(None) | ZshCommand::Cond(_) | ZshCommand::Arith(_) => {}
        }
    }

    fn walk_simple(&mut self, s: &ZshSimple, line: u32) {
        let is_local_keyword = s
            .words
            .first()
            .map(|w| matches!(w.as_str(), "local" | "typeset" | "declare" | "private"))
            .unwrap_or(false);
        let is_alias_keyword = s
            .words
            .first()
            .map(|w| w.as_str() == "alias")
            .unwrap_or(false);
        let is_autoload_keyword = s
            .words
            .first()
            .map(|w| w.as_str() == "autoload")
            .unwrap_or(false);
        let in_function = !self.local_stack.is_empty();

        // `alias name='value'` / `alias name=value …` — register every
        // `name=…` token after the `alias` keyword as a Func-kind decl
        // (aliases are callable like functions; for the LSP they
        // participate in goto-def / find-refs / rename identically).
        //
        // Untokenize the word FIRST so the `=` separator can be found
        // — the parser stores it as `Equals` (0x8d) inside the word,
        // and `find('=')` on the tokenized form misses it.
        if is_alias_keyword {
            for w in s.words.iter().skip(1) {
                let untok_word = crate::ported::lex::untokenize(w);
                if untok_word.starts_with('-') {
                    continue;
                }
                let name_part = match untok_word.find('=') {
                    Some(i) => &untok_word[..i],
                    None => untok_word.as_str(),
                };
                if !name_part.is_empty()
                    && name_part
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                {
                    self.declare(name_part, SymbolKind::Func, line);
                }
            }
        }

        // `autoload -U funcname [more …]` — every non-flag word names
        // a lazy-loaded function. Treat as Func decls so call sites
        // resolve back here.
        if is_autoload_keyword {
            for w in s.words.iter().skip(1) {
                let raw = w.as_str();
                if raw.starts_with('-') || raw.starts_with('+') {
                    continue;
                }
                let untok = crate::ported::lex::untokenize(raw);
                if !untok.is_empty()
                    && untok
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                {
                    self.declare(&untok, SymbolKind::Func, line);
                }
            }
        }

        // Form 1: leading assignments like `x=1 some_cmd`. The parser
        // splits these into `s.assigns`. At program scope they're
        // global; in a function body they're scoped local.
        for a in &s.assigns {
            let kind = if in_function {
                SymbolKind::Local
            } else {
                SymbolKind::Global
            };
            self.declare(&a.name, kind, line);
            if let ZshAssignValue::Scalar(v) = &a.value {
                self.scan_dollar_refs(v, line);
            } else if let ZshAssignValue::Array(v) = &a.value {
                for item in v {
                    self.scan_dollar_refs(item, line);
                }
            }
        }

        // Form 2: `local name=value` / `typeset name=value` / `local name`.
        // Here `local` is `words[0]` (the command); each remaining
        // `name=value` or bare `name` is treated as a Local
        // declaration. Multiple decls on one line are supported.
        if is_local_keyword {
            for w in s.words.iter().skip(1) {
                let raw = w.as_str();
                // Skip option flags (e.g. `-i`, `-a`, `-r`); zsh's
                // typeset accepts a flag-bag here.
                if raw.starts_with('-') {
                    continue;
                }
                let name_part = match raw.find('=') {
                    Some(i) => &raw[..i],
                    None => raw,
                };
                if !name_part.is_empty()
                    && name_part
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_')
                {
                    self.declare(name_part, SymbolKind::Local, line);
                }
                if let Some(i) = raw.find('=') {
                    self.scan_dollar_refs(&raw[i + 1..], line);
                }
            }
        }

        // First non-wrapper word is the command name.
        if let Some(first) = s.words.first() {
            let callee = if is_local_keyword {
                // `local`/`typeset` aren't user-defined funcs; don't
                // record a func-call ref for `local` itself.
                None
            } else if matches!(
                first.as_str(),
                "builtin" | "command" | "exec" | "noglob" | "nocorrect"
            ) {
                s.words.get(1)
            } else {
                Some(first)
            };
            if let Some(name) = callee {
                if !name.is_empty() {
                    self.record_ref(name, line);
                }
            }
            // Scan every word for `$var` mentions regardless of form.
            for w in &s.words {
                self.scan_dollar_refs(w, line);
            }
        }
    }

    /// Scan a word for `$name` / `${name}` parameter expansions and
    /// record refs for each. Crude but matches what zsh actually does
    /// — full param expansion (`${name:-default}` etc.) all start with
    /// the same `$name` shape we care about.
    ///
    /// The parser stores word strings with zsh's internal token bytes
    /// (e.g. `$` ↔ `\u{85}` aka `Stringg`). `untokenize` reverts those
    /// back to the printable form so we can match the literal `$`.
    fn scan_dollar_refs(&mut self, word: &str, line: u32) {
        // Route through the char-level canonical port (`ported::lex::
        // untokenize`, port of `Src/exec.c:2077`). The byte-level
        // `zwc::untokenize` is for raw-byte .zwc-file paths; a Rust
        // `String` encodes each token char (`\u{85}` Stringg, `\u{8c}`
        // Qstring, etc.) as a 2-byte UTF-8 sequence, which byte-level
        // processing splits — Qstring would be silently dropped, hiding
        // `$VAR` refs inside double-quoted words from this scanner.
        let cleaned = crate::ported::lex::untokenize(word);
        let bytes = cleaned.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] != b'$' {
                i += 1;
                continue;
            }
            i += 1;
            if i >= bytes.len() {
                break;
            }
            // Optional `{` for `${name}` form. Stop at the closing `}`
            // / `:` / `-` / `=` / `+` / `?` since those open param-
            // expansion operators after the name.
            let braced = bytes[i] == b'{';
            if braced {
                i += 1;
                // Skip a `${(flags)NAME...}` flag prefix. zsh parameter
                // expansion accepts a parenthesized flag group right
                // after `${` (e.g. `${(U)NAME}` upcase, `${(@f)str}`
                // split-on-newlines, `${(Pkv@)name}` indirect+kv). Walk
                // past one balanced `(...)` if present so the name
                // reader below lands on the identifier.
                if i < bytes.len() && bytes[i] == b'(' {
                    let mut depth = 1i32;
                    i += 1;
                    while i < bytes.len() && depth > 0 {
                        match bytes[i] {
                            b'(' => depth += 1,
                            b')' => depth -= 1,
                            _ => {}
                        }
                        i += 1;
                    }
                }
            }
            let start = i;
            while i < bytes.len() {
                let c = bytes[i];
                if c.is_ascii_alphanumeric() || c == b'_' {
                    i += 1;
                } else {
                    break;
                }
            }
            if i > start {
                if let Ok(name) = std::str::from_utf8(&bytes[start..i]) {
                    self.record_ref(name, line);
                }
            }
            // Skip past the rest of the expansion. Cheap heuristic:
            // when braced, walk to the matching `}`.
            if braced {
                while i < bytes.len() && bytes[i] != b'}' {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_ast_occurrences_func_decl_with_hyphen() {
        let _g = crate::test_util::global_state_lock();
        let src = "function daemon-ping() {\n    echo hi\n}\n";
        // Diagnostic: dump what the parser actually stores so the
        // expected-name comparison can be validated.
        let table = SymbolTable::build(src);
        let dump: Vec<(String, String)> = table
            .as_ref()
            .map(|t| {
                t.symbols
                    .iter()
                    .map(|s| (s.name.clone(), format!("{:?}", s.kind)))
                    .collect()
            })
            .unwrap_or_default();
        let lines = find_ast_occurrences(src, "daemon-ping", SymbolKind::Func);
        assert!(
            lines.contains(&0),
            "expected line 0 (the FuncDef decl).\n  AST symbols: {:?}\n  occurrences: {:?}",
            dump,
            lines
        );
    }

    #[test]
    fn alias_decl_recorded_as_func() {
        let _g = crate::test_util::global_state_lock();
        let src = "alias myalias='echo hello'\nmyalias\n";
        let t = SymbolTable::build(src).expect("parse ok");
        let dump: Vec<_> = t.symbols.iter().map(|s| (&s.name, &s.kind, s.decl_line)).collect();
        let funcs: Vec<_> = t
            .symbols
            .iter()
            .filter(|s| matches!(s.kind, SymbolKind::Func) && s.name == "myalias")
            .collect();
        assert_eq!(funcs.len(), 1, "alias not recorded. table: {:?}", dump);
        assert_eq!(funcs[0].decl_line, 0, "decl line: {:?}", dump);
    }

    #[test]
    fn autoload_decl_recorded_as_func() {
        let _g = crate::test_util::global_state_lock();
        let src = "autoload -U callback_helper\ncallback_helper\n";
        let t = SymbolTable::build(src).expect("parse ok");
        let funcs: Vec<_> = t
            .symbols
            .iter()
            .filter(|s| matches!(s.kind, SymbolKind::Func) && s.name == "callback_helper")
            .collect();
        assert_eq!(funcs.len(), 1);
        // Should also have a ref to the call site.
        let refs: Vec<_> = t.refs.iter().filter(|r| r.name == "callback_helper").collect();
        assert!(!refs.is_empty(), "no ref recorded for call: {:?}", t.refs);
    }

    #[test]
    fn global_param_refs_resolve_in_dq_string() {
        let _g = crate::test_util::global_state_lock();
        let src = "FOO=42\necho \"$FOO is set\"\necho \"${FOO:-x}\"\n";
        let t = SymbolTable::build(src).expect("parse ok");
        let foo_id = t
            .symbols
            .iter()
            .find(|s| s.name == "FOO" && matches!(s.kind, SymbolKind::Global))
            .map(|s| s.id)
            .expect("FOO declared");
        let refs: Vec<_> = t.refs.iter().filter(|r| r.symbol == foo_id).collect();
        assert_eq!(refs.len(), 2, "two interp refs expected: {:?}", t.refs);
    }

    #[test]
    fn find_ast_occurrences_global_assignment_and_refs() {
        let _g = crate::test_util::global_state_lock();
        // Top-level Global decl + two `$VAR` refs in different word
        // positions. The dq-string interp ref is the one the textual
        // path would have skipped via the inside-string mask.
        let src = "DAEMON_URL=http://example.com\n\
                   echo \"$DAEMON_URL is the URL\"\n\
                   curl \"$DAEMON_URL/api\"\n";
        let lines = find_ast_occurrences(src, "DAEMON_URL", SymbolKind::Global);
        assert!(lines.contains(&0), "decl line: {:?}", lines);
        assert!(lines.contains(&1), "dq-interp ref: {:?}", lines);
        assert!(lines.contains(&2), "second dq-interp ref: {:?}", lines);
    }

    #[test]
    fn find_ast_occurrences_func_kind_ignores_dollar_refs() {
        let _g = crate::test_util::global_state_lock();
        // A `$daemon-ping` ref (impossible in real zsh but defensive)
        // and a function-call `daemon-ping` must not be conflated
        // when asking for kind=Func.
        let src = "function daemon-ping() { :; }\ndaemon-ping arg\n";
        let lines = find_ast_occurrences(src, "daemon-ping", SymbolKind::Func);
        // Two: decl + call. No `$`-ref entry.
        assert_eq!(lines, vec![0, 1], "{:?}", lines);
    }

    #[test]
    fn find_ast_occurrences_global_kind_ignores_func_calls() {
        let _g = crate::test_util::global_state_lock();
        // Asking for Global occurrences of `foo` must skip the
        // command-position call `foo` and only catch the assignment
        // / `$foo` ref forms.
        let src = "foo=1\nfoo bar\necho $foo\n";
        let lines = find_ast_occurrences(src, "foo", SymbolKind::Global);
        // Lines 0 (assign) and 2 ($foo). Line 1 (call) NOT included.
        assert!(lines.contains(&0), "assign: {:?}", lines);
        assert!(lines.contains(&2), "$foo ref: {:?}", lines);
        assert!(!lines.contains(&1), "func-call leaked: {:?}", lines);
    }

    #[test]
    fn find_ast_occurrences_func_call_with_hyphen() {
        let _g = crate::test_util::global_state_lock();
        let src = "function daemon-ping() { :; }\ndaemon-ping\ndaemon-ping arg\n";
        let lines = find_ast_occurrences(src, "daemon-ping", SymbolKind::Func);
        // Expect 3: decl at 0, call at 1, call at 2.
        assert!(lines.contains(&0), "decl line: {:?}", lines);
        assert!(lines.contains(&1), "call line 1: {:?}", lines);
        assert!(lines.contains(&2), "call line 2: {:?}", lines);
    }

    #[test]
    fn func_decl_recorded() {
        let _g = crate::test_util::global_state_lock();
        let t = SymbolTable::build("function greet { echo hi }\n").expect("parse ok");
        let funcs: Vec<_> = t
            .symbols
            .iter()
            .filter(|s| matches!(s.kind, SymbolKind::Func))
            .collect();
        assert_eq!(funcs.len(), 1);
        assert_eq!(funcs[0].name, "greet");
        assert_eq!(funcs[0].decl_line, 0);
    }

    #[test]
    fn func_call_is_recorded_as_ref() {
        let _g = crate::test_util::global_state_lock();
        let src = "function greet { echo hi }\ngreet\ngreet world\n";
        let t = SymbolTable::build(src).expect("parse ok");
        let func_id = t
            .symbols
            .iter()
            .find(|s| matches!(s.kind, SymbolKind::Func) && s.name == "greet")
            .map(|s| s.id)
            .expect("greet found");
        let refs: Vec<_> = t.refs.iter().filter(|r| r.symbol == func_id).collect();
        assert_eq!(refs.len(), 2, "two call sites recorded: {:?}", t.refs);
    }

    #[test]
    fn external_command_is_not_recorded_as_func_ref() {
        let _g = crate::test_util::global_state_lock();
        // `echo` is not a user-defined function — it has no symbol in
        // the table, so `record_ref` should drop it entirely.
        let t = SymbolTable::build("echo hi\n").expect("parse ok");
        assert!(
            t.refs.is_empty(),
            "no refs for unknown command: {:?}",
            t.refs
        );
    }

    #[test]
    fn global_assignment_records_a_global_symbol() {
        let _g = crate::test_util::global_state_lock();
        let t = SymbolTable::build("my_var=42\n").expect("parse ok");
        let g: Vec<_> = t
            .symbols
            .iter()
            .filter(|s| matches!(s.kind, SymbolKind::Global))
            .collect();
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].name, "my_var");
    }

    #[test]
    fn local_inside_function_records_local() {
        let _g = crate::test_util::global_state_lock();
        let t = SymbolTable::build("function f { local x=1; echo $x }\n").expect("parse ok");
        let locals: Vec<_> = t
            .symbols
            .iter()
            .filter(|s| matches!(s.kind, SymbolKind::Local))
            .collect();
        assert!(
            locals.iter().any(|s| s.name == "x"),
            "local x recorded: {:?}",
            t.symbols
        );
    }

    #[test]
    fn dollar_var_ref_is_resolved_to_local() {
        let _g = crate::test_util::global_state_lock();
        let t = SymbolTable::build("function f { local x=1; echo $x }\n").expect("parse ok");
        let x_id = t
            .symbols
            .iter()
            .find(|s| s.name == "x" && matches!(s.kind, SymbolKind::Local))
            .map(|s| s.id)
            .expect("x found");
        assert!(
            t.refs.iter().any(|r| r.symbol == x_id && r.name == "x"),
            "$x resolved to the local: {:?}",
            t.refs
        );
    }

    #[test]
    fn symbol_at_finds_decl_line() {
        let _g = crate::test_util::global_state_lock();
        let src = "function greet { echo hi }\n";
        let t = SymbolTable::build(src).expect("parse ok");
        let id = t.symbol_at(0, "greet").expect("resolved");
        let sym = t.symbols.iter().find(|s| s.id == id).unwrap();
        assert_eq!(sym.name, "greet");
        assert!(matches!(sym.kind, SymbolKind::Func));
    }

    #[test]
    fn symbol_at_finds_call_site() {
        let _g = crate::test_util::global_state_lock();
        let src = "function greet { echo hi }\ngreet world\n";
        let t = SymbolTable::build(src).expect("parse ok");
        // Line 1 (0-based) is the `greet world` call.
        let id = t.symbol_at(1, "greet").expect("call resolved");
        let sym = t.symbols.iter().find(|s| s.id == id).unwrap();
        assert_eq!(sym.name, "greet");
        assert!(matches!(sym.kind, SymbolKind::Func));
    }
}
