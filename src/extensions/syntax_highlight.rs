//! Port of fish-shell's `highlight/highlight.rs` (vendor/fish/highlight/highlight.rs) —
//! native command-line syntax highlighting.
//!
//! fish:1 — Functions for syntax highlighting.
//!
//! The zsh script plugins (zsh-syntax-highlighting, fast-syntax-highlighting) are
//! script-level recreations of this fish engine; this ports the origin directly.
//!
//! zshrs substrate swaps (each cited at its site):
//!   * fish AST visitor            → zshrs lexer token stream (`lex_line_tokens`, the
//!     `inpush` + `LEXFLAGS_ZLE|LEXFLAGS_ACTIVE` + `ctxtlex` pattern of
//!     zle_tricky.rs:1357-1445 / zsh Src/Zle/zle_tricky.c:1157-1445), with token spans
//!     from the C word-offset arithmetic: start = zlemetall - wordbeg (lex.c:1886),
//!     end = zlemetall + 1 - inbufct (lex.c:1884)
//!   * `fish_color_*` env vars     → `$ZSH_HIGHLIGHT_STYLES[key]` (z-sy-h config compat),
//!     parsed by `match_highlight` (prompt.rs:3660, zsh Src/prompt.c:2031)
//!   * `TextFace`                  → packed `zattr` bitmap (zsh_h.rs:4295)
//!   * abbreviations               → zsh aliases (`aliastab`)
//!   * `builtin_exists`/`function::exists`/`path_get_path` → `builtintab`/`shfunctab`/
//!     `cmdnamtab` + `findcmd`
//!   * fish `%self`                → dropped (no zsh spelling)
//!   * fish `$var[slice]`          → zsh `$var[subscript]`
//!
//! The quoting rules inside `color_string_internal` are re-derived for zsh: `'...'`
//! (no escapes unless RC_QUOTES), `"..."` (`\\ \" \$ \`` + `$var`), `$'...'` (ANSI-C
//! escapes — fish's \u/\x/octal validation applies HERE), backticks, bare `\X`.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]

use crate::ported::exec::findcmd;
use crate::ported::hashtable::{aliastab_lock, cmdnamtab_lock, reswdtab_lock};
use crate::ported::lex::{
    ctxtlex, incmdpos, inredir, tok, tokstr, untokenize, LEX_LEXFLAGS, LEX_WORDBEG,
};
use crate::ported::params::{gethparam, getsparam};
use crate::ported::prompt::match_highlight;
use crate::ported::utils::getshfunc;
use crate::ported::zsh_h::{
    isset, lextok, zattr, AMPER, AMPERBANG, AUTOCD, BAR_TOK, CASE, CLOBBER, DAMPER, DBAR, DINANG,
    DINANGDASH, DOUTANG, DOUTANGAMP, DOUTANGAMPBANG, DOUTANGBANG, ENDINPUT, ENVARRAY, ENVSTRING,
    INANGAMP, INANG_TOK, INOUTANG, INPAR_TOK, INTERACTIVECOMMENTS, IS_REDIROP, LEXERR,
    LEXFLAGS_ACTIVE, LEXFLAGS_ZLE, NEWLIN, OUTANGAMP, OUTANGAMPBANG, OUTANGBANG, OUTANG_TOK,
    OUTPAR_TOK, SEMI, SEPER, STRING_LEX, TYPESET,
};
use crate::zle_file_tester::{
    expand_one_no_cmdsubst, FileTester, IsErr, IsFile, OperationContext, RedirectionMode,
};
use std::collections::{hash_map::Entry, HashMap};
use std::sync::Mutex;

/// fish:1306-1317 — Simple value type describing how a character should be highlighted.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct HighlightSpec {
    pub foreground: HighlightRole,
    pub background: HighlightRole,
    pub valid_path: bool,
    pub force_underline: bool,
}

impl HighlightSpec {
    /// fish:43-45 — `new`.
    pub fn new() -> Self {
        Self::default()
    }
    /// fish:47-53 — `with_fg_bg`.
    pub fn with_fg_bg(fg: HighlightRole, bg: HighlightRole) -> Self {
        Self {
            foreground: fg,
            background: bg,
            ..Default::default()
        }
    }
    /// fish:55-57 — `with_fg`.
    pub fn with_fg(fg: HighlightRole) -> Self {
        Self::with_fg_bg(fg, HighlightRole::normal)
    }
    /// fish:59-61 — `with_bg`.
    pub fn with_bg(bg: HighlightRole) -> Self {
        Self::with_fg_bg(HighlightRole::normal, bg)
    }
    /// fish:63-65 — `with_both`.
    pub fn with_both(role: HighlightRole) -> Self {
        Self::with_fg_bg(role, role)
    }
}

/// fish:1268-1304 — Describes the role of a span of text.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
#[repr(u8)]
pub enum HighlightRole {
    #[default]
    normal, // normal text
    error,   // error
    command, // command
    keyword,
    statement_terminator, // process separator
    param,                // command parameter (argument)
    option,               // argument starting with "-", up to a "--"
    comment,              // comment
    search_match,         // search match
    operat,               // operator
    escape,               // escape sequences
    quote,                // quoted string
    redirection,          // redirection
    autosuggestion,       // autosuggestion
    selection,

    // fish:1289-1303 — Pager support (kept for name parity; the zshrs completion
    // menu may consume these later).
    pager_progress,
    pager_background,
    pager_prefix,
    pager_completion,
    pager_description,
    pager_secondary_background,
    pager_secondary_prefix,
    pager_secondary_completion,
    pager_secondary_description,
    pager_selected_background,
    pager_selected_prefix,
    pager_selected_completion,
    pager_selected_description,
}

/// fish:692-693 — `ColorArray`: one `HighlightSpec` per character of the buffer.
pub type ColorArray = Vec<HighlightSpec>;

/// fish:1197-1228 — `get_highlight_var_name`, respelled for zsh: role →
/// `$ZSH_HIGHLIGHT_STYLES` key (z-sy-h main-highlighter config surface, so existing
/// user themes apply to the native engine unchanged). Autosuggestion is configured by
/// `$ZSH_AUTOSUGGEST_HIGHLIGHT_STYLE` (scalar) — special-cased in the resolver.
fn get_highlight_style_key(role: HighlightRole) -> &'static str {
    match role {
        HighlightRole::normal => "default",
        HighlightRole::error => "unknown-token",
        HighlightRole::command => "command",
        HighlightRole::keyword => "reserved-word",
        HighlightRole::statement_terminator => "commandseparator",
        HighlightRole::param => "default",
        HighlightRole::option => "single-hyphen-option",
        HighlightRole::comment => "comment",
        HighlightRole::search_match => "history-search-match",
        HighlightRole::operat => "globbing",
        HighlightRole::escape => "back-dollar-quoted-argument",
        HighlightRole::quote => "single-quoted-argument",
        HighlightRole::redirection => "redirection",
        HighlightRole::autosuggestion => "autosuggestion",
        HighlightRole::selection => "selection",
        // Pager roles have no z-sy-h key; resolver falls through to defaults.
        _ => "default",
    }
}

/// Built-in default style per role, used when `$ZSH_HIGHLIGHT_STYLES` doesn't override.
/// These mirror the z-sy-h main-highlighter default palette so the native engine looks
/// identical to the plugin it replaces (z-sy-h defaults: command/builtin/function/alias
/// fg=green, unknown-token fg=red, reserved-word fg=yellow, comment fg=black+bold,
/// single/double-quoted fg=yellow, dollar escapes fg=cyan, globbing fg=blue,
/// path underline; zsh-autosuggestions default: fg=8).
fn get_default_style(role: HighlightRole) -> &'static str {
    match role {
        // f-sy-h's default is `unknown-token = fg=red,bold`
        // (fast-highlight:58), and it is what this engine replaces on a
        // daily-driver rc. Measured over a pty against real zsh + f-sy-h,
        // an incomplete command word emits `\e[1m\e[31m` (bold red) and
        // this engine emitted a plain `\e[31m`, so the two diverged on
        // every partially-typed command. z-sy-h's plain `fg=red` is the
        // odd one out here; $ZSH_HIGHLIGHT_STYLES[unknown-token] still
        // overrides for anyone who wants it back.
        HighlightRole::error => "fg=red,bold",
        HighlightRole::command | HighlightRole::keyword => match role {
            HighlightRole::keyword => "fg=yellow",
            _ => "fg=green",
        },
        HighlightRole::comment => "fg=black,bold",
        HighlightRole::operat => "fg=blue",
        HighlightRole::escape => "fg=cyan",
        HighlightRole::quote => "fg=yellow",
        HighlightRole::redirection => "none",
        HighlightRole::autosuggestion => "fg=8",
        HighlightRole::selection | HighlightRole::search_match => "standout",
        _ => "none",
    }
}

// fish:1230-1266 — Table used to fetch fallback highlights in case the specified one
// wasn't set.
fn get_fallback(role: HighlightRole) -> HighlightRole {
    match role {
        HighlightRole::normal
        | HighlightRole::error
        | HighlightRole::command
        | HighlightRole::statement_terminator
        | HighlightRole::param
        | HighlightRole::search_match
        | HighlightRole::comment
        | HighlightRole::operat
        | HighlightRole::escape
        | HighlightRole::quote
        | HighlightRole::redirection
        | HighlightRole::autosuggestion
        | HighlightRole::selection
        | HighlightRole::pager_progress
        | HighlightRole::pager_background
        | HighlightRole::pager_prefix
        | HighlightRole::pager_completion
        | HighlightRole::pager_description => HighlightRole::normal,
        HighlightRole::keyword => HighlightRole::command,
        HighlightRole::option => HighlightRole::param,
        HighlightRole::pager_secondary_background => HighlightRole::pager_background,
        HighlightRole::pager_secondary_prefix | HighlightRole::pager_selected_prefix => {
            HighlightRole::pager_prefix
        }
        HighlightRole::pager_secondary_completion | HighlightRole::pager_selected_completion => {
            HighlightRole::pager_completion
        }
        HighlightRole::pager_secondary_description | HighlightRole::pager_selected_description => {
            HighlightRole::pager_description
        }
        HighlightRole::pager_selected_background => HighlightRole::search_match,
    }
}

/// fish:222-238 — `parse_text_face_for_highlight`, respelled: parse a z-sy-h style
/// string ("fg=green,bold", "none", …) into a packed `zattr` via `match_highlight`
/// (prompt.rs:3660). Returns None for empty/no-op specs so the role chain can fall
/// through, mirroring fish's `get_unless_empty` + default-face check.
fn parse_style_for_highlight(spec: &str) -> Option<zattr> {
    if spec.is_empty() {
        return None;
    }
    let (mask_on, _mask_off) = match_highlight(spec);
    Some(mask_on)
}

/// Read `$ZSH_HIGHLIGHT_STYLES[key]`. The assoc arrives from `gethparam` as a flat
/// key/value sequence.
fn zsh_highlight_styles_get(key: &str) -> Option<String> {
    let flat = gethparam("ZSH_HIGHLIGHT_STYLES")?;
    let mut it = flat.chunks_exact(2);
    it.find(|kv| kv[0] == key).map(|kv| kv[1].clone())
}

/// fish:131-138 — highlight_color_resolver_t resolves highlight specs (like "a
/// command") to actual attributes. It maintains a cache with no invalidation
/// mechanism. The lifetime of these should typically be one screen redraw.
#[derive(Default)]
pub struct HighlightColorResolver {
    /// fish:136-137 — `cache`.
    cache: HashMap<HighlightSpec, zattr>,
}

impl HighlightColorResolver {
    /// fish:144-147 — `new`.
    pub fn new() -> Self {
        Default::default()
    }
    /// fish:148-162 — Return a packed attribute for a given highlight spec.
    pub fn resolve_spec(&mut self, highlight: &HighlightSpec) -> zattr {
        match self.cache.entry(*highlight) {
            Entry::Occupied(e) => *e.get(),
            Entry::Vacant(e) => {
                let face = Self::resolve_spec_uncached(highlight);
                e.insert(face);
                face
            }
        }
    }
    /// fish:163-219 — `resolve_spec_uncached`.
    pub fn resolve_spec_uncached(highlight: &HighlightSpec) -> zattr {
        // fish:164-182 — role → [role, fallback, normal] chain, first configured wins.
        let resolve_role = |role: HighlightRole| -> zattr {
            let mut roles: &[HighlightRole] = &[role, get_fallback(role), HighlightRole::normal];
            for i in [2, 1] {
                if roles[i - 1] == roles[i] {
                    roles = &roles[..i];
                }
            }
            for &role in roles {
                // Autosuggestion config lives in zsh-autosuggestions' scalar param.
                let configured = if role == HighlightRole::autosuggestion {
                    getsparam("ZSH_AUTOSUGGEST_HIGHLIGHT_STYLE").filter(|s| !s.is_empty())
                } else {
                    zsh_highlight_styles_get(get_highlight_style_key(role))
                };
                if let Some(face) = configured.as_deref().and_then(parse_style_for_highlight) {
                    return face;
                }
            }
            // No user config anywhere in the chain: built-in default palette.
            parse_style_for_highlight(get_default_style(role)).unwrap_or(0)
        };
        let mut face = resolve_role(highlight.foreground);

        // fish:185-194 — background merge is a no-op here: zattr carries fg+bg in one
        // bitmap and z-sy-h specs already say "bg=…" inline; resolve background role
        // only when it differs and OR its bg bits in.
        if highlight.background != highlight.foreground {
            use crate::ported::zsh_h::{TXTBGCOLOUR, TXT_ATTR_BG_COL_MASK};
            let bg_face = resolve_role(highlight.background);
            face |= bg_face & (TXTBGCOLOUR as zattr | TXT_ATTR_BG_COL_MASK);
        }

        // fish:196-211 — valid_path modifier: merge the `path` style (z-sy-h key;
        // default underline, matching fish_color_valid_path --underline).
        if highlight.valid_path {
            let path_spec = zsh_highlight_styles_get("path").unwrap_or_default();
            let merged = if path_spec.is_empty() {
                parse_style_for_highlight("underline")
            } else {
                parse_style_for_highlight(&path_spec)
            };
            if let Some(m) = merged {
                face |= m;
            }
        }

        // fish:213-216 — force_underline.
        if highlight.force_underline {
            face |= crate::ported::zsh_h::TXTUNDERLINE as zattr;
        }

        face
    }
}

/// fish:68-91 — Given a string and list of colors of the same size, return the string
/// with ANSI escape sequences representing the colors.
pub fn colorize(text: &str, colors: &[HighlightSpec]) -> Vec<u8> {
    let chars: Vec<char> = text.chars().collect();
    assert_eq!(colors.len(), chars.len());
    let mut rv = HighlightColorResolver::new();
    let mut out: Vec<u8> = Vec::new();

    let mut last_color: Option<HighlightSpec> = None;
    for (i, &c) in chars.iter().enumerate() {
        let color = colors[i];
        if Some(color) != last_color {
            let face = rv.resolve_spec(&color);
            out.extend_from_slice(zattr_to_sgr(face).as_bytes());
            last_color = Some(color);
        }
        // fish:84-86 — reset before a trailing newline.
        if i + 1 == chars.len() && c == '\n' {
            out.extend_from_slice(b"\x1b[0m");
        }
        let mut buf = [0u8; 4];
        out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
    }
    out.extend_from_slice(b"\x1b[0m"); // fish:89
    out
}

/// !!! WARNING: RUST-ONLY HELPER — NO DIRECT FISH COUNTERPART !!!
/// fish routes attribute output through its Outputter/terminfo stack; zshrs's
/// ZLE painter consumes `zattr` directly (zle_refresh.rs `to_zattr`/`zwcputc`),
/// so this SGR string form exists only for `colorize` (batch/CLI output).
fn zattr_to_sgr(attr: zattr) -> String {
    use crate::ported::zsh_h::{
        TXTBGCOLOUR, TXTBOLDFACE, TXTFGCOLOUR, TXTSTANDOUT, TXTUNDERLINE, TXT_ATTR_BG_COL_SHIFT,
        TXT_ATTR_FG_COL_SHIFT,
    };
    let mut s = String::from("\x1b[0");
    if attr & TXTBOLDFACE as zattr != 0 {
        s.push_str(";1");
    }
    if attr & TXTSTANDOUT as zattr != 0 {
        s.push_str(";7");
    }
    if attr & TXTUNDERLINE as zattr != 0 {
        s.push_str(";4");
    }
    if attr & TXTFGCOLOUR as zattr != 0 {
        let col = (attr >> TXT_ATTR_FG_COL_SHIFT) & 0xffffff;
        s.push_str(&format!(";38;5;{}", col));
    }
    if attr & TXTBGCOLOUR as zattr != 0 {
        let col = (attr >> TXT_ATTR_BG_COL_SHIFT) & 0xffffff;
        s.push_str(&format!(";48;5;{}", col));
    }
    s.push('m');
    s
}

/// fish:93-113 — Perform syntax highlighting for the shell commands in buff. The
/// result is stored in the color array as a HighlightSpec for each character in buff.
///
/// buffstr: the buffer (metafied, as the ZLE metaline) on which to perform syntax
/// highlighting; ctx: cancellation check; io_ok: if set, allow IO which may block —
/// e.g. invalid commands may be detected; cursor: cursor position in the commandline.
pub fn highlight_shell(
    buff: &str,
    color: &mut Vec<HighlightSpec>,
    ctx: &OperationContext,
    io_ok: bool,
    cursor: Option<usize>,
) {
    // fish:110 — get_pwd_slash.
    let working_directory = getsparam("PWD").unwrap_or_else(|| ".".to_owned());
    let mut highlighter = Highlighter::new(buff, cursor, ctx, working_directory, io_ok);
    *color = highlighter.highlight();
}

/// fish:114-129 — `highlight_and_colorize`.
pub fn highlight_and_colorize(text: &str, ctx: &OperationContext) -> Vec<u8> {
    let mut colors = Vec::new();
    highlight_shell(
        text,
        &mut colors,
        ctx,
        /*io_ok=*/ false,
        /*cursor=*/ None,
    );
    colorize(text, &colors)
}

/// fish parse_constants.rs `StatementDecoration` — the zsh precommand modifiers that
/// restrict what the following command word may resolve to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StatementDecoration {
    #[default]
    None_,
    Command, // `command cmd` / `exec cmd` — externals only
    Builtin, // `builtin cmd` — builtins only
    Exec,
}

/// Per-process command-validity cache. `findcmd` walks the whole $PATH on
/// every miss, and while a command is being typed EVERY prefix is a miss
/// ("g", "gi", "git" …) — uncached, that is a full PATH stat-walk per
/// keystroke. fish eats this cost on a background thread; the synchronous
/// zshrs pass must not. Fingerprinted by $PATH so `rehash`-style changes
/// invalidate naturally; bounded so a pathological session can't grow it.
static CMD_VALID_CACHE: Mutex<Option<(String, HashMap<(String, u8), bool>)>> = Mutex::new(None);
const CMD_VALID_CACHE_MAX: usize = 8192;

pub fn command_is_valid_cached(
    cmd: &str,
    decoration: StatementDecoration,
    working_directory: &str,
) -> bool {
    // The table/PATH verdict is cacheable (fingerprinted by $PATH); the
    // implicit-cd branch depends on the cwd and stays uncached — it is a
    // single stat, the PATH walk is the expensive part.
    let path_now = getsparam("PATH").unwrap_or_default();
    let key = (cmd.to_owned(), decoration as u8);
    let cached: Option<bool> = {
        let mut guard = CMD_VALID_CACHE.lock().unwrap();
        match guard.as_mut() {
            Some((path, map)) if *path == path_now => map.get(&key).copied(),
            _ => {
                *guard = Some((path_now, HashMap::new()));
                None
            }
        }
    };
    let tables_valid = cached.unwrap_or_else(|| {
        let v = command_is_valid_tables(cmd, decoration);
        let mut guard = CMD_VALID_CACHE.lock().unwrap();
        if let Some((_, map)) = guard.as_mut() {
            if map.len() >= CMD_VALID_CACHE_MAX {
                map.clear();
            }
            map.insert(key, v);
        }
        v
    });
    if tables_valid {
        return true;
    }
    // fish:292-295 — Implicit cd (zsh: AUTO_CD), uncached (cwd-relative).
    if decoration == StatementDecoration::None_ && isset(AUTOCD) {
        let path = crate::zle_file_tester::path_apply_working_directory(cmd, working_directory);
        return std::fs::metadata(&path)
            .map(|m| m.is_dir())
            .unwrap_or(false);
    }
    false
}

/// fish:240-299 — `command_is_valid`.
pub fn command_is_valid(
    cmd: &str,
    decoration: StatementDecoration,
    working_directory: &str,
) -> bool {
    if command_is_valid_tables(cmd, decoration) {
        return true;
    }
    // fish:292-295 — Implicit cd (zsh: AUTO_CD); disabled by `command`/
    // `builtin`/`exec` decorations (fish:252-267 implicit_cd_ok).
    if decoration == StatementDecoration::None_ && isset(AUTOCD) {
        let path = crate::zle_file_tester::path_apply_working_directory(cmd, working_directory);
        if std::fs::metadata(&path)
            .map(|m| m.is_dir())
            .unwrap_or(false)
        {
            return true;
        }
    }
    // fish:297-298 — Return what we got.
    false
}

/// fish:246-290 — the table/PATH checks of `command_is_valid` (everything except
/// the cwd-dependent implicit-cd branch, split out so the cache can hold them).
fn command_is_valid_tables(cmd: &str, decoration: StatementDecoration) -> bool {
    // fish:246-267 — Determine which types we check, based on the decoration.
    let mut builtin_ok = true;
    let mut function_ok = true;
    // fish's abbreviations are zsh's aliases.
    let mut alias_ok = true;
    let mut command_ok = true;
    if matches!(
        decoration,
        StatementDecoration::Command | StatementDecoration::Exec
    ) {
        builtin_ok = false;
        function_ok = false;
        alias_ok = false;
        command_ok = true;
    } else if decoration == StatementDecoration::Builtin {
        builtin_ok = true;
        function_ok = false;
        alias_ok = false;
        command_ok = false;
    }

    // fish:269-270 — Check them.
    let mut is_valid = false;

    // fish:272-275 — Builtins. (Reserved words resolve at the lexer level and never
    // reach here as STRING tokens, so no reswdtab check is needed.)
    if !is_valid && builtin_ok {
        is_valid = crate::ported::builtin::createbuiltintable().contains_key(cmd)
            // zshrs's extension builtins (provenance, dbview, zcache, …)
            // are NOT in createbuiltintable — they live in
            // EXT_BUILTIN_NAMES and dispatch through ext_builtins. The
            // highlighter asked only the core table, so every one of them
            // painted as an unknown token even though `whence -w
            // provenance` says `builtin` and `${+builtins[provenance]}`
            // is 1 in the same shell.
            //
            // `builtin_in_builtintab` is the predicate every OTHER
            // consumer asks (ext_builtins.rs:207-228), and it already
            // honours the `--zsh` / ZSHRS_HIDE_EXT_BUILTINS gate, so a
            // zshrs-original builtin correctly goes back to unknown-token
            // under emulation, where it is hidden from the namespace.
            // NOTE: `builtin_in_builtintab` alone is NOT a membership
            // test — `builtin_owning_module` returns None for an unknown
            // name and its `None => true` arm then reports every string
            // as available. Membership in EXT_BUILTIN_NAMES has to come
            // first; the availability call adds the `disable`/module and
            // ZSHRS_HIDE_EXT_BUILTINS gates on top.
            || (crate::ext_builtins::EXT_BUILTIN_NAMES.contains(&cmd)
                && crate::ext_builtins::builtin_in_builtintab(cmd));
    }

    // fish:277-280 — Functions.
    if !is_valid && function_ok {
        is_valid = getshfunc(cmd).is_some();
    }

    // fish:282-285 — Aliases (fish: abbreviations).
    if !is_valid && alias_ok {
        is_valid = aliastab_lock()
            .read()
            .map(|t| t.get(cmd).is_some())
            .unwrap_or(false);
    }

    // fish:287-290 — Regular commands: hashed table first, then a PATH walk.
    if !is_valid && command_ok {
        is_valid = cmdnamtab_lock()
            .read()
            .map(|t| t.get(cmd).is_some())
            .unwrap_or(false)
            || findcmd(cmd, 0, 0).is_some();
    }

    is_valid
}

/// fish:301-308 — `has_expand_reserved`: does the string still carry expansion
/// markers? zsh spelling: any lexer token char left in the ITOK range.
fn has_expand_reserved(s: &str) -> bool {
    s.chars().any(|wc| ('\u{84}'..='\u{a1}').contains(&wc))
}

/// fish:310-341 — Parse a command line. Return the first command, and the first
/// argument to that command (as a string), if any. This is used to validate
/// autosuggestions. fish parses an AST; zshrs walks the same token stream the
/// highlighter uses (continue_after_error + accept_incomplete are what
/// LEXFLAGS_ACTIVE provides).
pub fn autosuggest_parse_command(buff: &str) -> Option<(String, String)> {
    let toks = lex_line_tokens(buff);
    let mut cmd: Option<String> = None;
    let mut arg = String::new(); // fish:329
    for t in &toks {
        if t.tok == STRING_LEX {
            match &cmd {
                None if t.cmdpos => {
                    // fish:328 — expand the command word.
                    let text = t.clean_text();
                    let mut expanded = t.text.clone().unwrap_or_default();
                    if expand_one_no_cmdsubst(&mut expanded) && !expanded.is_empty() {
                        cmd = Some(expanded);
                    } else {
                        cmd = Some(text);
                    }
                }
                None => (),
                Some(_) => {
                    // fish:330-335 — Check if the first argument or redirection is,
                    // in fact, an argument.
                    if !t.in_redir {
                        arg = t.clean_text();
                    }
                    break;
                }
            }
        } else if cmd.is_some() {
            break; // separator/redirection ends the first statement's argument scan
        }
    }
    cmd.map(|c| (c, arg)) // fish:337
}

/// fish:342-345 — `is_veritable_cd`: it's really `cd`, not something wrapping cd
/// (fish checks the completion wrap map; the zsh spelling is an alias named cd).
pub fn is_veritable_cd(expanded_command: &str) -> bool {
    expanded_command == "cd"
        && aliastab_lock()
            .read()
            .map(|t| t.get("cd").is_none())
            .unwrap_or(true)
}

/// fish:347-356 — Given an item from the history which is a proposed autosuggestion,
/// return whether the autosuggestion is valid. It may not be valid if e.g. it is
/// attempting to cd into a directory which does not exist.
///
/// zsh adaptation: fish history items carry `required_paths` metadata; zsh history
/// has none, so callers pass an empty slice and only the cd/command checks apply.
pub fn autosuggest_validate_from_history(
    item_commandline: &str,
    required_paths: &[String],
    working_directory: &str,
    ctx: &OperationContext,
) -> bool {
    // fish:357 — background-thread assertion dropped (synchronous compute).
    // fish:359-364 — the multi-command suggested_range trim is handled by the caller
    // (zshrs suggests whole history lines only).

    // fish:366-372 — Parse the string.
    let Some((parsed_command, mut cd_dir)) = autosuggest_parse_command(item_commandline) else {
        // This is for autosuggestions which are not decorated commands, e.g. function
        // declarations.
        return true;
    };

    // fish:374-390 — We handle cd specially.
    if is_veritable_cd(&parsed_command) && !cd_dir.is_empty() {
        if expand_one_no_cmdsubst(&mut cd_dir) {
            if "--help".starts_with(&cd_dir) || "-h".starts_with(&cd_dir) {
                // fish:379-383 — cd --help is always valid.
                return true;
            } else {
                // fish:384-389 — Check the directory target, respecting CDPATH.
                // Permit the autosuggestion if the path is valid and not our directory.
                return crate::zle_file_tester::is_potential_cd_path(
                    &cd_dir,
                    /*at_cursor=*/ false,
                    working_directory,
                    ctx,
                    Default::default(),
                );
            }
        }
    }

    // fish:392-398 — Not handled specially. Is the command valid?
    let cmd_ok = command_is_valid_cached(
        &parsed_command,
        StatementDecoration::None_,
        working_directory,
    );
    if !cmd_ok {
        return false;
    }

    // fish:400-403 — Did the historical command have arguments that look like paths,
    // which aren't paths now?
    if !required_paths.is_empty() {
        let tester = FileTester::new(working_directory.to_owned(), ctx);
        if !required_paths.iter().all(|p| tester.test_path(p, false)) {
            return false;
        }
    }

    true // fish:405
}

/// zsh `$var` name characters (fish:common valid_var_name_char, zsh spelling:
/// alphanumeric or underscore — Src/params.c iident).
fn valid_var_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn valid_var_name(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with(|c: char| c.is_ascii_digit())
        && s.chars().all(valid_var_name_char)
}

/// zsh single-character special parameters ($?, $#, $$, $!, $@, $*, $-, $0..$9…).
fn is_special_param_char(c: char) -> bool {
    matches!(c, '?' | '#' | '$' | '!' | '@' | '*' | '-' | '_') || c.is_ascii_digit()
}

/// fish:408-471 — Highlights the variable starting with '$', setting colors within
/// the 'colors' array. Returns the number of characters consumed.
///
/// zsh respelling: `$name`, `$name[subscript]`, `${...}` (balanced), single-char
/// specials. fish's `$$var` chain and slice loop map to zsh's subscript span.
fn color_variable(inp: &[char], colors: &mut [HighlightSpec]) -> usize {
    assert_eq!(inp[0], '$');

    let at = |i: usize| -> char { inp.get(i).copied().unwrap_or('\0') };

    // fish:413-429 — Handle an initial run of $s.
    let mut idx = 0;
    let mut dollar_count = 0;
    while at(idx) == '$' {
        // Our color depends on the next char.
        let next = at(idx + 1);
        if next == '$' || valid_var_name_char(next) || is_special_param_char(next) {
            colors[idx] = HighlightSpec::with_fg(HighlightRole::operat);
        } else if next == '(' || next == '{' || next == '\'' {
            // zsh: $(cmdsub), ${param}, $'ansi-quote' — the '$' is an operator and the
            // construct is handled by the caller / string scanner.
            colors[idx] = HighlightSpec::with_fg(HighlightRole::operat);
            return idx + 1;
        } else {
            colors[idx] = HighlightSpec::with_fg(HighlightRole::error);
        }
        idx += 1;
        dollar_count += 1;
    }

    // Single-char special param ($?, $#, …) — consume exactly one char.
    if idx == dollar_count && !valid_var_name_char(at(idx)) && is_special_param_char(at(idx)) {
        colors[idx] = HighlightSpec::with_fg(HighlightRole::operat);
        return idx + 1;
    }

    // fish:431-445 — Handle a sequence of variable characters.
    // It may contain an escaped newline - see fish#8444.
    loop {
        if valid_var_name_char(at(idx)) {
            colors[idx] = HighlightSpec::with_fg(HighlightRole::operat);
            idx += 1;
        } else if at(idx) == '\\' && at(idx + 1) == '\n' {
            colors[idx] = HighlightSpec::with_fg(HighlightRole::operat);
            idx += 1;
            colors[idx] = HighlightSpec::with_fg(HighlightRole::operat);
            idx += 1;
        } else {
            break;
        }
    }

    // fish:447-469 — Handle a subscript (fish: slice), up to dollar_count of them.
    // Note that we currently don't do any validation of the subscript's contents.
    for _slice_count in 0..dollar_count {
        match subscript_length(&inp[idx..]) {
            Some(slice_len) if slice_len > 0 => {
                colors[idx] = HighlightSpec::with_fg(HighlightRole::operat);
                colors[idx + slice_len - 1] = HighlightSpec::with_fg(HighlightRole::operat);
                idx += slice_len;
            }
            Some(_slice_len) => {
                // not a subscript
                break;
            }
            None => {
                // fish:460-467 — Syntax error: color the variable + the subscript
                // start red.
                colors[..=idx].fill(HighlightSpec::with_fg(HighlightRole::error));
                break;
            }
        }
    }
    idx
}

/// fish parse_util `slice_length`, zsh respelling: length of a balanced
/// `[...]` subscript starting at input, 0 if input doesn't open one, None if
/// unbalanced.
fn subscript_length(inp: &[char]) -> Option<usize> {
    if inp.first() != Some(&'[') {
        return Some(0);
    }
    let mut depth = 0usize;
    for (i, &c) in inp.iter().enumerate() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => (),
        }
    }
    None
}

/// fish:473-691 — This function is a disaster badly in need of refactoring (fish's
/// words, kept for the record). It colors an argument or command, without regard to
/// command substitutions. Quoting rules are zsh's; structure is fish's.
pub fn color_string_internal(
    buffstr: &str,
    base_color: HighlightSpec,
    colors: &mut [HighlightSpec],
) {
    // fish:476-485 — Clarify what we expect.
    assert!(
        [
            HighlightSpec::with_fg(HighlightRole::param),
            HighlightSpec::with_fg(HighlightRole::option),
            HighlightSpec::with_fg(HighlightRole::command)
        ]
        .contains(&base_color),
        "Unexpected base color"
    );
    let chars: Vec<char> = buffstr.chars().collect();
    let buff_len = chars.len();
    colors.fill(base_color);

    // fish:489-493 — fish's %self special-case has no zsh spelling; dropped.

    #[derive(Eq, PartialEq)]
    enum Mode {
        unquoted,
        single_quoted,
        double_quoted,
        dollar_quoted, // zsh $'...' — fish's \u/\x/octal validation applies here
        backtick,      // zsh `...`
    }
    let mut mode = Mode::unquoted;
    let mut unclosed_quote_offset = None;
    let mut bracket_count = 0;
    let mut in_pos = 0;
    while in_pos < buff_len {
        let c = chars[in_pos];
        match mode {
            Mode::unquoted => {
                if c == '\\' {
                    // fish:509-593 — bare backslash escape. zsh unquoted `\X` quotes
                    // X literally (no \n/\t expansion outside $'…'), so color the
                    // pair as escape; a trailing lone backslash is a continuation.
                    let backslash_pos = in_pos;
                    let fill_end = if in_pos + 1 < buff_len {
                        in_pos + 2
                    } else {
                        in_pos + 1
                    };
                    colors[backslash_pos..fill_end]
                        .fill(HighlightSpec::with_fg(HighlightRole::escape));
                    in_pos += 1; // skip the escaped char; loop tail adds one more
                } else {
                    // fish:594-634 — Not a backslash.
                    match c {
                        '~' if in_pos == 0 => {
                            colors[in_pos] = HighlightSpec::with_fg(HighlightRole::operat);
                        }
                        '$' if chars.get(in_pos + 1) == Some(&'\'') => {
                            // zsh $'...' — enter dollar-quote mode; color both opener
                            // chars as quote.
                            colors[in_pos] = HighlightSpec::with_fg(HighlightRole::quote);
                            colors[in_pos + 1] = HighlightSpec::with_fg(HighlightRole::quote);
                            unclosed_quote_offset = Some(in_pos);
                            in_pos += 1;
                            mode = Mode::dollar_quoted;
                        }
                        '$' => {
                            assert!(in_pos < buff_len);
                            in_pos += color_variable(&chars[in_pos..], &mut colors[in_pos..]);
                            // fish:603-604 — Subtract one to account for the upcoming
                            // loop increment.
                            in_pos -= 1;
                        }
                        '`' => {
                            colors[in_pos] = HighlightSpec::with_fg(HighlightRole::operat);
                            unclosed_quote_offset = Some(in_pos);
                            mode = Mode::backtick;
                        }
                        '?' | '*' | '(' | ')' => {
                            // fish:606-611 — globs and grouping.
                            colors[in_pos] = HighlightSpec::with_fg(HighlightRole::operat);
                        }
                        '{' => {
                            colors[in_pos] = HighlightSpec::with_fg(HighlightRole::operat);
                            bracket_count += 1;
                        }
                        '}' => {
                            colors[in_pos] = HighlightSpec::with_fg(HighlightRole::operat);
                            bracket_count -= 1;
                        }
                        ',' if bracket_count > 0 => {
                            colors[in_pos] = HighlightSpec::with_fg(HighlightRole::operat);
                        }
                        '[' | ']' => {
                            // zsh char classes / subscripts glob
                            colors[in_pos] = HighlightSpec::with_fg(HighlightRole::operat);
                        }
                        '\'' => {
                            colors[in_pos] = HighlightSpec::with_fg(HighlightRole::quote);
                            unclosed_quote_offset = Some(in_pos);
                            mode = Mode::single_quoted;
                        }
                        '"' => {
                            colors[in_pos] = HighlightSpec::with_fg(HighlightRole::quote);
                            unclosed_quote_offset = Some(in_pos);
                            mode = Mode::double_quoted;
                        }
                        _ => (), // fish:633 — we ignore all other characters
                    }
                }
            }
            // fish:637-653 — single quoted string, i.e 'foo'. zsh: NO escapes inside
            // single quotes; `''` inside is a literal ' only under RC_QUOTES (handled
            // as close+reopen, which colors identically).
            Mode::single_quoted => {
                colors[in_pos] = HighlightSpec::with_fg(HighlightRole::quote);
                if c == '\'' {
                    mode = Mode::unquoted;
                }
            }
            // fish:654-682 — double quoted string, i.e. "foo".
            Mode::double_quoted => {
                // fish:656-660 — subscripts are colored in advance, past `in_pos`,
                // and we don't want to overwrite that.
                if colors[in_pos] == base_color {
                    colors[in_pos] = HighlightSpec::with_fg(HighlightRole::quote);
                }
                match c {
                    '"' => {
                        mode = Mode::unquoted;
                    }
                    '\\' if in_pos + 1 < buff_len => {
                        // fish:665-674 — zsh dquote escapes: \\ \" \$ \`.
                        let escaped_char = chars[in_pos + 1];
                        if matches!(escaped_char, '\\' | '"' | '$' | '`' | '\n') {
                            colors[in_pos] = HighlightSpec::with_fg(HighlightRole::escape);
                            colors[in_pos + 1] = HighlightSpec::with_fg(HighlightRole::escape);
                            in_pos += 1; // skip over backslash
                        }
                    }
                    '$' => {
                        in_pos += color_variable(&chars[in_pos..], &mut colors[in_pos..]);
                        // fish:677-678 — Subtract one to account for the upcoming
                        // increment in the loop.
                        in_pos -= 1;
                    }
                    _ => (), // fish:680 — we ignore all other characters
                }
            }
            // zsh $'...' — ANSI-C quoting. fish's escape-sequence validation
            // (fish:538-591) applies in THIS mode for zsh.
            Mode::dollar_quoted => {
                colors[in_pos] = HighlightSpec::with_fg(HighlightRole::quote);
                if c == '\\' && in_pos + 1 < buff_len {
                    let mut fill_color = HighlightRole::escape;
                    let backslash_pos = in_pos;
                    let mut fill_end = backslash_pos;
                    in_pos += 1;
                    let escaped_char = chars[in_pos];
                    if "abcefnrtv\\'\"?".contains(escaped_char) {
                        fill_end = in_pos + 1;
                    } else if "uUxX01234567".contains(escaped_char) {
                        // fish:538-591 — numeric escapes with base/width/max
                        // validation.
                        let mut res: u32 = 0;
                        let mut chars_max = 2;
                        let mut base = 16;
                        let mut max_val = 0x7f_u32; // ASCII_MAX

                        match escaped_char {
                            'u' => {
                                chars_max = 4;
                                max_val = 0xFFFF; // UCS2_MAX
                                in_pos += 1;
                            }
                            'U' => {
                                chars_max = 8;
                                // fish:551-553 — Don't exceed the largest Unicode
                                // code point - see fish#1107.
                                max_val = 0x10FFFF;
                                in_pos += 1;
                            }
                            'x' | 'X' => {
                                max_val = 0xFF;
                                in_pos += 1;
                            }
                            _ => {
                                // fish:560-564 — a digit like \12.
                                base = 8;
                                chars_max = 3;
                            }
                        }

                        // fish:567-577 — Consume.
                        for _i in 0..chars_max {
                            if in_pos == buff_len {
                                break;
                            }
                            let Some(d) = chars[in_pos].to_digit(base) else {
                                break;
                            };
                            res = res.saturating_mul(base).saturating_add(d);
                            in_pos += 1;
                        }
                        // fish:578-581 — in_pos is now at the first character that
                        // could not be converted (or buff_len).
                        fill_end = in_pos;

                        // fish:583-586 — It's an error if we exceeded the max value.
                        if res > max_val {
                            fill_color = HighlightRole::error;
                        }

                        // fish:588-590 — Subtract one so the loop increment moves to
                        // the next character.
                        in_pos -= 1;
                    } else {
                        fill_end = in_pos + 1;
                    }
                    if fill_end > backslash_pos {
                        colors[backslash_pos..fill_end.min(buff_len)]
                            .fill(HighlightSpec::with_fg(fill_color));
                    } else {
                        colors[backslash_pos] = HighlightSpec::with_fg(fill_color);
                        colors[in_pos.min(buff_len - 1)] = HighlightSpec::with_fg(fill_color);
                    }
                } else if c == '\'' {
                    mode = Mode::unquoted;
                }
            }
            // zsh `...` backquote command substitution: color content as quote-ish,
            // delimiters as operators.
            Mode::backtick => {
                if c == '`' {
                    colors[in_pos] = HighlightSpec::with_fg(HighlightRole::operat);
                    mode = Mode::unquoted;
                } else {
                    colors[in_pos] = HighlightSpec::with_fg(HighlightRole::quote);
                }
            }
        }
        in_pos += 1;
    }

    // fish:687-690 — Error on unclosed quotes.
    if mode != Mode::unquoted {
        colors[unclosed_quote_offset.unwrap()] = HighlightSpec::with_fg(HighlightRole::error);
    }
}

// ---------------------------------------------------------------------------
// Token-stream driver — the zshrs replacement for fish's AST visit.
// ---------------------------------------------------------------------------

/// One lexed token with its source span. Spans are char indices into the (metafied)
/// input line.
#[derive(Clone, Debug)]
pub struct TokSpan {
    pub tok: lextok,
    pub text: Option<String>,
    pub start: usize,
    pub end: usize,
    /// incmdpos() sampled BEFORE this token was lexed — is it in command position?
    pub cmdpos: bool,
    /// inredir() sampled before this token — is it a redirection target?
    pub in_redir: bool,
}

impl TokSpan {
    /// Untokenized, quote-null-stripped text (what the user "meant").
    pub fn clean_text(&self) -> String {
        let mut s = self.text.clone().unwrap_or_default();
        crate::ported::glob::remnulargs(&mut s);
        untokenize(&s)
    }
}

/// Lex a (metafied) line into spanned tokens, tolerating in-progress constructs.
///
/// Direct reuse of the completion machinery's lex pattern —
/// zle_tricky.rs:1357-1445 (zsh Src/Zle/zle_tricky.c:1157-1290):
/// `zcontext_save` → `LEXFLAGS_ZLE|LEXFLAGS_ACTIVE` → `inpush(dupstrspace(line))` →
/// `strinbeg(0)` → `ctxtlex()` loop → `strinend`/`inpop` → `zcontext_restore`.
///
/// Span arithmetic is the C completion formulas with addedx = 0:
///   start = zlemetall - wordbeg          (lex.c:1886 `nwb`)
///   end   = zlemetall + 1 - inbufct      (lex.c:1884 `nwe`)
///
/// ZLEMETACS is parked past every reachable `nwe` while lexing so `gotword`
/// (lex.rs:3042, lex.c:1882-1895) never clears LEXFLAGS_ZLE mid-line — wordbeg must
/// keep updating for EVERY token, not just up to a completion cursor. The completion
/// globals are saved/restored around the walk.
pub fn lex_line_tokens(line: &str) -> Vec<TokSpan> {
    use crate::ported::zle::compcore::{ADDEDX, ZLEMETACS, ZLEMETALL};
    use std::sync::atomic::Ordering;

    let ll = line.chars().count() as i32;
    let mut out: Vec<TokSpan> = Vec::new();

    // Save completion globals we're about to borrow.
    let saved_cs = ZLEMETACS.load(Ordering::SeqCst);
    let saved_ll = ZLEMETALL.load(Ordering::SeqCst);
    let saved_addedx = ADDEDX.load(Ordering::SeqCst);

    crate::ported::context::zcontext_save(); // c:1169
                                             // LEX_UNGET_BUF isolation: hgetc drains this Rust-only side channel
                                             // BEFORE every input frame (lex.rs hgetc), and `$(...)` bodies use
                                             // it as a deliberate cross-context handoff — so zcontext_save
                                             // leaves it alone. This walk must isolate it manually: the
                                             // suspended outer parse's ungot chars must not be consumed as line
                                             // content, and the walk's own ungets must not leak back out.
    let saved_unget: std::collections::VecDeque<char> =
        crate::ported::lex::LEX_UNGET_BUF.with_borrow_mut(std::mem::take);
    ZLEMETALL.store(ll, Ordering::SeqCst);
    ZLEMETACS.store(ll + 5, Ordering::SeqCst); // park beyond max nwe = ll + 1
    ADDEDX.store(0, Ordering::SeqCst);

    // c:1170 — see zle_tricky.rs:1369-1375 for why ACTIVE is OR'd in (tolerate
    // unterminated quote/backtick/brace while the user is mid-word).
    LEX_LEXFLAGS.set(LEXFLAGS_ZLE | LEXFLAGS_ACTIVE);
    crate::ported::input::inpush(&crate::ported::zle::zle_tricky::dupstrspace(line), 0, None); // c:1171
    crate::ported::hist::strinbeg(0); // c:1172

    // No-progress guard: on an unterminated construct the lexer can re-yield
    // the same (retyped) LEXERR token without consuming input — inbufct stops
    // moving and the loop would spin, accumulating TokSpans without bound
    // (observed as a runaway-memory hang on `echo 'unterminated`). C's
    // completion loop never hits this because gotword ends its walk at the
    // cursor; this walk covers the whole line, so it must self-terminate.
    let mut prev_inbufct = i32::MIN;
    loop {
        let cmdpos_before = incmdpos();
        let inredir_before = inredir();
        ctxtlex(); // c:1213

        // c:1215-1227 — LEXERR fixup: odd Snull/Dnull count means an unterminated
        // quote; treat as STRING so the in-progress word still gets colored.
        let mut tokv = tok();
        let was_lexerr = tokv == LEXERR;
        if tokv == LEXERR {
            match tokstr() {
                None => break,
                Some(ts) => {
                    use crate::ported::zsh_h::{Dnull, Snull};
                    let jcnt = ts.chars().filter(|&c| c == Snull || c == Dnull).count();
                    if jcnt & 1 == 1 {
                        tokv = STRING_LEX;
                    }
                }
            }
        }

        if tokv == ENDINPUT {
            break; // c:1273
        }

        let inbufct = crate::ported::input::inbufct.with(|c| c.get());
        let wordbeg = LEX_WORDBEG.get();
        let start = (ll - wordbeg).clamp(0, ll) as usize; // lex.c:1886
        let end = (ll + 1 - inbufct).clamp(start as i32, ll) as usize; // lex.c:1884

        let no_progress = inbufct == prev_inbufct;
        prev_inbufct = inbufct;

        out.push(TokSpan {
            tok: tokv,
            text: tokstr(),
            start,
            end,
            cmdpos: cmdpos_before,
            in_redir: inredir_before,
        });

        // A LEXERR (even one retyped to STRING) that consumed no input can
        // never make progress; neither can any token stream longer than the
        // line has characters. Both are hard stops, not errors.
        if (was_lexerr && no_progress) || out.len() > (ll as usize + 8) {
            break;
        }
        if was_lexerr && inbufct <= 1 {
            break; // trailing dupstrspace space is all that remains
        }
    }

    crate::ported::hist::strinend(); // c:1608
    crate::ported::input::inpop(); // c:1609
    crate::ported::context::zcontext_restore(); // c:1745
                                                // Put the suspended parse's ungot chars back, discarding anything
                                                // this walk left behind (see the isolation note at the top).
    crate::ported::lex::LEX_UNGET_BUF.with_borrow_mut(|b| *b = saved_unget);

    ZLEMETACS.store(saved_cs, Ordering::SeqCst);
    ZLEMETALL.store(saved_ll, Ordering::SeqCst);
    ADDEDX.store(saved_addedx, Ordering::SeqCst);

    out
}

/// fish:695-715 — Syntax highlighter helper.
pub struct Highlighter<'s> {
    // fish:697-698 — The string we're highlighting (metafied line).
    buff: &'s str,
    buff_chars: Vec<char>,
    // fish:699-700 — The position of the cursor within the string.
    cursor: Option<usize>,
    // fish:701-702 — The operation context.
    ctx: &'s OperationContext,
    // fish:703-704 — Whether it's OK to do I/O.
    io_ok: bool,
    // fish:705-706 — Working directory.
    working_directory: String,
    // fish:707-708 — Our component for testing strings for being potential file paths.
    file_tester: FileTester<'s>,
    // fish:709-710 — The resulting colors.
    color_array: ColorArray,
    // fish:711-713 — A stack of variables that the current commandline probably
    // defines. We mark redirections as valid if they use one of these variables, to
    // avoid marking valid targets as error.
    pending_variables: Vec<String>,
    done: bool,
}

impl<'s> Highlighter<'s> {
    /// fish:718-738 — `new`.
    pub fn new(
        buff: &'s str,
        cursor: Option<usize>,
        ctx: &'s OperationContext,
        working_directory: String,
        can_do_io: bool,
    ) -> Self {
        let file_tester = FileTester::new(working_directory.clone(), ctx);
        Self {
            buff,
            buff_chars: buff.chars().collect(),
            cursor,
            ctx,
            io_ok: can_do_io,
            working_directory,
            file_tester,
            color_array: vec![],
            pending_variables: vec![],
            done: false,
        }
    }

    /// fish:739-788 — `highlight`.
    pub fn highlight(&mut self) -> ColorArray {
        assert!(!self.done);
        self.done = true;

        self.color_array
            .resize(self.buff_chars.len(), HighlightSpec::default());

        // fish:752-761 — parse. The lexer flags LEXFLAGS_ZLE|ACTIVE inside
        // lex_line_tokens are the zsh spelling of fish's continue_after_error +
        // accept_incomplete_tokens + leave_unterminated.
        let toks = lex_line_tokens(self.buff);

        self.visit_tokens(&toks);
        if self.ctx.check_cancel() {
            return std::mem::take(&mut self.color_array);
        }

        // fish:768-772 — Color every comment. The zsh lexer consumes comments as
        // whitespace, so recover them from inter-token gaps: an unquoted '#' at a
        // word boundary starts a comment when INTERACTIVE_COMMENTS is set.
        if isset(INTERACTIVECOMMENTS) {
            self.color_gap_comments(&toks);
        }

        // fish:782-785 — Color every error range: a LEXERR token that survived the
        // unterminated-quote fixup is a real lex error.
        for t in toks.iter().filter(|t| t.tok == LEXERR) {
            self.color_span(t.start, self.buff_chars.len(), HighlightRole::error);
        }

        std::mem::take(&mut self.color_array)
    }

    fn io_still_ok(&self) -> bool {
        // fish:797-799
        self.io_ok && !self.ctx.check_cancel()
    }

    // fish:876-880 — Colors a range with a given color.
    fn color_span(&mut self, start: usize, end: usize, role: HighlightRole) {
        let end = end.min(self.color_array.len());
        if start < end {
            self.color_array[start..end].fill(HighlightSpec::with_fg(role));
        }
    }

    /// Source text of a span.
    fn span_text(&self, t: &TokSpan) -> String {
        self.buff_chars[t.start.min(self.buff_chars.len())..t.end.min(self.buff_chars.len())]
            .iter()
            .collect()
    }

    /// The zshrs replacement for fish's NodeVisitor::visit (fish:1157-1181):
    /// walk the token stream, tracking command position, decoration, cd, `--`,
    /// assignments and redirections.
    fn visit_tokens(&mut self, toks: &[TokSpan]) {
        let mut decoration = StatementDecoration::None_;
        // A precommand modifier (`command`, `builtin`, `exec`, `noglob`,
        // `nocorrect`) is itself the cmdpos word, so the REAL command word
        // that follows it is not flagged cmdpos by the lexer. fish's comment
        // at the decoration arms says "keep looking for the real command
        // word"; without this flag that never happened, so `command id` left
        // `id` unvisited — neither validated nor coloured (an invalid word
        // like `command zzqwx` was equally uncoloured, so the error case was
        // invisible too).
        let mut after_decoration = false;
        let mut expanded_cmd = String::new();
        let mut is_cd = false;
        let mut is_typeset = false;
        let mut have_dashdash = false;
        let mut i = 0;
        while i < toks.len() {
            if self.ctx.check_cancel() {
                return;
            }
            let t = &toks[i];
            let tokv = t.tok;

            if IS_REDIROP(tokv) {
                // fish:962-1014 — redirection operator + target.
                let target = toks.get(i + 1).filter(|n| n.tok == STRING_LEX);
                self.visit_redirection(t, target);
                if target.is_some() {
                    i += 2;
                } else {
                    i += 1;
                }
                continue;
            }

            match tokv {
                SEPER | NEWLIN | SEMI | AMPER | AMPERBANG | BAR_TOK => {
                    // fish:912-917 — End/Pipe/Background → statement_terminator.
                    self.color_span(t.start, t.end, HighlightRole::statement_terminator);
                    decoration = StatementDecoration::None_;
                    expanded_cmd.clear();
                    is_cd = false;
                    is_typeset = false;
                    have_dashdash = false;
                }
                DBAR | DAMPER => {
                    // fish:921 — AndAnd/OrOr → operator.
                    self.color_span(t.start, t.end, HighlightRole::operat);
                    decoration = StatementDecoration::None_;
                    expanded_cmd.clear();
                    is_cd = false;
                    is_typeset = false;
                    have_dashdash = false;
                }
                INPAR_TOK | OUTPAR_TOK | INOUTPAR_LOCAL => {
                    self.color_span(t.start, t.end, HighlightRole::operat);
                }
                ENVSTRING => {
                    // fish:1016-1025 — visit_variable_assignment.
                    self.visit_variable_assignment(t);
                }
                ENVARRAY => {
                    // zsh `a=(…)`: name colored like an assignment; parens follow as
                    // INPAR/OUTPAR tokens.
                    self.visit_variable_assignment(t);
                }
                STRING_LEX => {
                    if (t.cmdpos || after_decoration) && expanded_cmd.is_empty() {
                        // fish:1032-1073 — visit_decorated_statement (command word).
                        let clean = t.clean_text();
                        match clean.as_str() {
                            // fish:1033-1036 — color any decoration and keep looking
                            // for the real command word.
                            "command" => {
                                decoration = StatementDecoration::Command;
                                // f-sy-h paints every precommand modifier as
                                // a COMMAND (measured: SGR 32 for command /
                                // builtin / exec / noglob / nocorrect). fish
                                // uses its keyword face here, which rendered
                                // them the colour of a control-flow word.
                                self.color_span(t.start, t.end, HighlightRole::command);
                                after_decoration = true;
                            }
                            "builtin" => {
                                decoration = StatementDecoration::Builtin;
                                // f-sy-h paints every precommand modifier as
                                // a COMMAND (measured: SGR 32 for command /
                                // builtin / exec / noglob / nocorrect). fish
                                // uses its keyword face here, which rendered
                                // them the colour of a control-flow word.
                                self.color_span(t.start, t.end, HighlightRole::command);
                                after_decoration = true;
                            }
                            "exec" => {
                                decoration = StatementDecoration::Exec;
                                // f-sy-h paints every precommand modifier as
                                // a COMMAND (measured: SGR 32 for command /
                                // builtin / exec / noglob / nocorrect). fish
                                // uses its keyword face here, which rendered
                                // them the colour of a control-flow word.
                                self.color_span(t.start, t.end, HighlightRole::command);
                                after_decoration = true;
                            }
                            "noglob" | "nocorrect" => {
                                self.color_span(t.start, t.end, HighlightRole::command);
                                after_decoration = true;
                            }
                            _ => {
                                self.visit_command_word(t, &clean, decoration);
                                after_decoration = false;
                                expanded_cmd = clean;
                                is_cd = is_veritable_cd(&expanded_cmd);
                                is_typeset = matches!(
                                    expanded_cmd.as_str(),
                                    "typeset"
                                        | "local"
                                        | "declare"
                                        | "export"
                                        | "readonly"
                                        | "integer"
                                        | "float"
                                );
                            }
                        }
                    } else {
                        // fish:932-960 — visit_argument.
                        if is_typeset {
                            // fish:1078-1088 — `set` (zsh: typeset family) defines
                            // variables; remember them for redirection validation.
                            let arg = t.clean_text();
                            let name = arg.split('=').next().unwrap_or("").to_owned();
                            if valid_var_name(&name) {
                                self.pending_variables.push(name);
                            }
                        }
                        self.visit_argument(t, is_cd, !have_dashdash);
                        if self.span_text(t) == "--" {
                            have_dashdash = true; // fish:1091-1093
                        }
                    }
                }
                tokv if (CASE..=TYPESET).contains(&tokv) => {
                    // fish:887-911 — visit_keyword: reserved words.
                    //
                    // EXCEPT the typeset family. `declare`, `export`,
                    // `float`, `integer`, `local`, `readonly` and
                    // `typeset` all lex to the single TYPESET token
                    // (hashtable.rs RESWDS), and while zsh does list
                    // them in `$reswords`, they are reserved only so the
                    // parser can treat their arguments as assignments —
                    // as hashtable.rs:862-864 already puts it, they
                    // "really live as builtins, so a reserved word
                    // inventory should exclude them".
                    //
                    // fast-syntax-highlighting agrees and paints them as
                    // commands. Measured against f-sy-h in real zsh over
                    // a pty, final colour of the completed word:
                    //   declare/typeset/local/export/readonly/integer
                    //                              -> SGR 32 (green)
                    //   if/while                   -> SGR 33 (yellow)
                    // This engine painted the whole range yellow, so
                    // every declaration command came out the colour of
                    // a control-flow keyword.
                    let role = if tokv == TYPESET {
                        HighlightRole::command
                    } else {
                        HighlightRole::keyword
                    };
                    self.color_span(t.start, t.end, role);
                    if tokv == TYPESET {
                        is_typeset = true;
                    }
                }
                LEXERR => {
                    self.color_span(t.start, t.end, HighlightRole::error);
                }
                _ => {
                    // fish:928 — default: leave normal.
                }
            }
            i += 1;
        }
    }

    // fish:801-811 + 1038-1073 — Color a command word after validity checking.
    fn visit_command_word(&mut self, t: &TokSpan, clean: &str, decoration: StatementDecoration) {
        // Reserved words arrive as their own token types; aliases as STRING that
        // checkalias() already expanded. What reaches here is a plain command word.
        let mut is_valid_cmd = false;
        if !self.io_still_ok() {
            // fish:1047-1049 — We cannot check if the command is invalid, so just
            // assume it's valid.
            is_valid_cmd = true;
        } else {
            // fish:1052-1065 — Check to see if the command is valid.
            // Try expanding it. If we cannot, it's an error.
            let mut expanded = t.text.clone().unwrap_or_default();
            let expanded_ok = expand_one_no_cmdsubst(&mut expanded);
            let cmd = if expanded_ok && !expanded.is_empty() {
                expanded
            } else {
                clean.to_owned()
            };
            if !has_expand_reserved(&cmd) {
                is_valid_cmd = command_is_valid_cached(&cmd, decoration, &self.working_directory);
            }
        }

        // fish:1068-1073 — Color our statement.
        if is_valid_cmd {
            let start = t.start;
            let end = t.end.min(self.color_array.len());
            let src: String = self.span_text(t);
            color_string_internal(
                &src,
                HighlightSpec::with_fg(HighlightRole::command),
                &mut self.color_array[start..end],
            );
        } else {
            self.color_span(t.start, t.end, HighlightRole::error);
        }
    }

    // fish:812-871 + 932-960 — Visit an argument, perhaps knowing that our command
    // is cd.
    fn visit_argument(&mut self, t: &TokSpan, cmd_is_cd: bool, options_allowed: bool) {
        let start = t.start;
        let end = t.end.min(self.color_array.len());
        if start >= end {
            return;
        }
        let src: String = self.span_text(t);

        // fish:820-833 — Color this argument without concern for command
        // substitutions.
        let base = if options_allowed && src.starts_with('-') {
            HighlightRole::option
        } else {
            HighlightRole::param
        };
        color_string_internal(
            &src,
            HighlightSpec::with_fg(base),
            &mut self.color_array[start..end],
        );

        // fish:835-870 — Now do command substitutions: locate `$(…)` spans in the
        // source and highlight the contents recursively.
        let src_chars: Vec<char> = src.chars().collect();
        let mut scan = 0usize;
        while let Some((open, close)) = locate_cmdsubst_span(&src_chars, scan) {
            // fish:846-851 — highlight the parens (closing paren may be missing on
            // an in-progress line).
            self.color_span(start + open, start + open + 2, HighlightRole::operat);
            let inner_start = open + 2;
            let inner_end = close.unwrap_or(src_chars.len());
            if let Some(c) = close {
                self.color_span(start + c, start + c + 1, HighlightRole::operat);
            }
            if inner_end > inner_start {
                // fish:853-869 — Highlight it recursively.
                let arg_cursor = self.cursor.map(|c| c.wrapping_sub(start + inner_start));
                let inner_src: String = src_chars[inner_start..inner_end].iter().collect();
                let mut cmdsub_highlighter = Highlighter::new(
                    &inner_src,
                    arg_cursor,
                    self.ctx,
                    self.working_directory.clone(),
                    self.io_still_ok(),
                );
                let subcolors = cmdsub_highlighter.highlight();
                let dst_lo = (start + inner_start).min(self.color_array.len());
                let dst_hi = (start + inner_end).min(self.color_array.len());
                let n = dst_hi - dst_lo;
                self.color_array[dst_lo..dst_hi].copy_from_slice(&subcolors[..n]);
            }
            scan = inner_end + 1;
            if scan >= src_chars.len() {
                break;
            }
        }

        if !self.io_still_ok() {
            return; // fish:935-937
        }

        // fish:939-959 — Underline every valid path.
        let is_prefix = self.cursor.is_some_and(|c| (t.start..=t.end).contains(&c));
        let token = t.text.clone().unwrap_or_default();
        let test_result = if cmd_is_cd {
            self.file_tester.test_cd_path(&token, is_prefix)
        } else {
            let is_path = self.file_tester.test_path(&token, is_prefix);
            Ok(IsFile(is_path))
        };
        match test_result {
            Ok(IsFile(false)) => (),
            Ok(IsFile(true)) => {
                for i in start..end {
                    self.color_array[i].valid_path = true;
                }
            }
            Err(IsErr) => self.color_span(start, end, HighlightRole::error),
        }
    }

    // fish:962-1014 — visit_redirection: operator token + optional target token.
    fn visit_redirection(&mut self, op: &TokSpan, target: Option<&TokSpan>) {
        // fish:968-980 — Color the operator part like 2>.
        self.color_span(op.start, op.end, HighlightRole::redirection);

        let Some(target) = target else { return };
        let target_text = target.text.clone().unwrap_or_default();

        // Heredoc delimiters are words, not files.
        if matches!(op.tok, DINANG | DINANGDASH) {
            self.color_span(target.start, target.end, HighlightRole::redirection);
            return;
        }

        // fish:982-988 — Check if the argument contains a command substitution. If
        // so, highlight it as a param and don't try to do any other validation.
        if target_text.contains(crate::ported::zsh_h::Tick)
            || target_text.contains(crate::ported::zsh_h::Qtick)
            || target_text.contains(crate::ported::zsh_h::Inpar)
        {
            self.visit_argument(target, false, true);
            return;
        }

        let mode = redir_tok_mode(op.tok, &target.clean_text());

        // fish:989-1007 — No command substitution, so we can highlight the target
        // file or fd.
        let (role, file_exists) = if !self.io_still_ok() {
            // fish:991-994 — I/O is disallowed, so we don't have much hope of
            // catching anything but gross errors. Assume it's valid.
            (HighlightRole::redirection, false)
        } else if contains_pending_variable(&self.pending_variables, &target_text) {
            // fish:995-997 — Target uses a variable defined by the current
            // commandline. Assume it's valid.
            (HighlightRole::redirection, false)
        } else {
            // fish:998-1006 — Validate the redirection target.
            if let Ok(IsFile(file_exists)) =
                self.file_tester.test_redirection_target(&target_text, mode)
            {
                (HighlightRole::redirection, file_exists)
            } else {
                (HighlightRole::error, false)
            }
        };
        self.color_span(target.start, target.end, role);
        if file_exists {
            // fish:1009-1013
            for i in target.start..target.end.min(self.color_array.len()) {
                self.color_array[i].valid_path = true;
            }
        }
    }

    // fish:1016-1025 — visit_variable_assignment.
    fn visit_variable_assignment(&mut self, t: &TokSpan) {
        let start = t.start;
        let end = t.end.min(self.color_array.len());
        if start >= end {
            return;
        }
        let src: String = self.span_text(t);
        color_string_internal(
            &src,
            HighlightSpec::with_fg(HighlightRole::param),
            &mut self.color_array[start..end],
        );
        // fish:1018-1024 — Highlight the '=' in variable assignments as an operator,
        // and remember the variable for redirection validation.
        if let Some(offset) = src.chars().position(|c| c == '=') {
            if start + offset < self.color_array.len() {
                self.color_array[start + offset] = HighlightSpec::with_fg(HighlightRole::operat);
            }
            let var_name: String = src.chars().take(offset).collect();
            if valid_var_name(&var_name) {
                self.pending_variables.push(var_name);
            }
        }
    }

    /// fish:768-772 adaptation — recover comment spans from inter-token gaps (the
    /// zsh lexer consumes comments as whitespace under INTERACTIVE_COMMENTS).
    fn color_gap_comments(&mut self, toks: &[TokSpan]) {
        let len = self.buff_chars.len();
        let mut gaps: Vec<(usize, usize)> = Vec::new();
        let mut prev_end = 0usize;
        for t in toks {
            if t.start > prev_end {
                gaps.push((prev_end, t.start.min(len)));
            }
            prev_end = prev_end.max(t.end);
        }
        if prev_end < len {
            gaps.push((prev_end, len));
        }
        for (lo, hi) in gaps {
            let mut j = lo;
            while j < hi {
                let c = self.buff_chars[j];
                if c == '#' {
                    // comment runs to end of line within this gap
                    let eol = self.buff_chars[j..hi]
                        .iter()
                        .position(|&c| c == '\n')
                        .map(|p| j + p)
                        .unwrap_or(hi);
                    self.color_span(j, eol, HighlightRole::comment);
                    j = eol;
                }
                j += 1;
            }
        }
    }
}

/// fish:1124-1132 — Return whether a string contains a command substitution;
/// respelled as a char scanner locating `$(`…`)` with paren balance. Returns
/// (open_index_of_'$', Some(close_index)) — close is None when unterminated.
fn locate_cmdsubst_span(chars: &[char], from: usize) -> Option<(usize, Option<usize>)> {
    let mut i = from;
    let mut in_squote = false;
    while i + 1 < chars.len() {
        let c = chars[i];
        if in_squote {
            if c == '\'' {
                in_squote = false;
            }
        } else if c == '\'' {
            in_squote = true;
        } else if c == '\\' {
            i += 1;
        } else if c == '$' && chars[i + 1] == '(' {
            // Found the opener. Find the balanced closer.
            let mut depth = 0i32;
            let mut j = i + 1;
            while j < chars.len() {
                match chars[j] {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some((i, Some(j)));
                        }
                    }
                    _ => (),
                }
                j += 1;
            }
            return Some((i, None));
        }
        i += 1;
    }
    None
}

/// fish:1134-1155 — `contains_pending_variable`.
fn contains_pending_variable(pending_variables: &[String], haystack: &str) -> bool {
    let hay: Vec<char> = haystack.chars().collect();
    for var_name in pending_variables {
        let needle: Vec<char> = var_name.chars().collect();
        if needle.is_empty() || hay.len() < needle.len() {
            continue;
        }
        let mut nextpos = 0usize;
        while nextpos + needle.len() <= hay.len() {
            let Some(relpos) = hay[nextpos..]
                .windows(needle.len())
                .position(|w| w == needle.as_slice())
            else {
                break;
            };
            let pos = nextpos + relpos;
            nextpos = pos + 1;
            if pos == 0 || hay[pos - 1] != '$' {
                continue; // fish:1144-1146
            }
            let end = pos + needle.len();
            if end < hay.len() && valid_var_name_char(hay[end]) {
                continue; // fish:1147-1150
            }
            return true;
        }
    }
    false
}

/// Map a zsh redirection token (+ its target text) to the fish RedirectionMode the
/// FileTester validates (fish tokenizer.rs PipeOrRedir::mode equivalent).
fn redir_tok_mode(tokv: lextok, target: &str) -> RedirectionMode {
    let looks_fd = target == "-" || target.chars().all(|c| c.is_ascii_digit());
    match tokv {
        OUTANG_TOK => {
            if isset(CLOBBER) {
                RedirectionMode::Overwrite
            } else {
                RedirectionMode::NoClob
            }
        }
        OUTANGBANG => RedirectionMode::Overwrite,
        DOUTANG | DOUTANGBANG => RedirectionMode::Append,
        INANG_TOK => RedirectionMode::Input,
        INOUTANG => RedirectionMode::Overwrite, // <> creates rw
        INANGAMP => RedirectionMode::Fd,
        OUTANGAMP | OUTANGAMPBANG => {
            if looks_fd {
                RedirectionMode::Fd
            } else {
                RedirectionMode::Overwrite // >& file duplicates both streams to file
            }
        }
        DOUTANGAMP | DOUTANGAMPBANG => {
            if looks_fd {
                RedirectionMode::Fd
            } else {
                RedirectionMode::Append
            }
        }
        crate::ported::zsh_h::AMPOUTANG => RedirectionMode::Overwrite,
        _ => RedirectionMode::Input,
    }
}

// Local alias: `(` `)` in `x=( … )` array assignments arrive as INPAR/OUTPAR; some
// grammars also use INOUTPAR for `()` in function definitions.
use crate::ported::zsh_h::INOUTPAR as INOUTPAR_LOCAL;

#[cfg(test)]
mod tests {
    use super::*;

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        crate::test_util::global_state_lock()
    }

    fn spans_of(line: &str) -> Vec<(String, i32)> {
        lex_line_tokens(line)
            .iter()
            .map(|t| {
                (
                    line.chars()
                        .skip(t.start)
                        .take(t.end - t.start)
                        .collect::<String>(),
                    t.tok,
                )
            })
            .collect()
    }

    /// The highlight walk must leave NO residue in the Rust-only
    /// `LEX_UNGET_BUF` side channel: `hgetc` (lex.rs:4209) drains it
    /// BEFORE every input frame, so leftover chars from a per-keystroke
    /// highlight pass are read by the NEXT real interactive parse —
    /// `{ print ok }` / `( print sub )` / every brace-body function
    /// definition typed interactively died with "parse error near `}'"
    /// while the fish engines were enabled (ZSHRS_NATIVE_ZLE_FX=0 made
    /// it vanish). The walk must also put back what the SUSPENDED outer
    /// parse had ungot before it blocked in zleread.
    #[test]
    fn lex_line_tokens_leaves_unget_buf_untouched() {
        use crate::ported::lex::LEX_UNGET_BUF;
        let _g = lock();
        for line in [
            "{ print ok }",
            "( print sub )",
            "function q(){ print fq }",
            "echo hi",
        ] {
            // Outer-parse residue that must survive the walk verbatim.
            LEX_UNGET_BUF.with_borrow_mut(|b| {
                b.clear();
                b.push_back('X');
                b.push_back('\n');
            });
            let _ = lex_line_tokens(line);
            let after: Vec<char> = LEX_UNGET_BUF.with_borrow(|b| b.iter().copied().collect());
            assert_eq!(
                after,
                vec!['X', '\n'],
                "LEX_UNGET_BUF corrupted by highlight walk of {line:?}"
            );
        }
        LEX_UNGET_BUF.with_borrow_mut(|b| b.clear());
    }

    /// Token spans must slice the source exactly — the C nwb/nwe formulas.
    #[test]
    fn lex_spans_match_source() {
        let _g = lock();
        let spans = spans_of("echo hello world");
        let words: Vec<&str> = spans
            .iter()
            .filter(|(_, t)| *t == STRING_LEX)
            .map(|(s, _)| s.as_str())
            .collect();
        assert_eq!(words, vec!["echo", "hello", "world"], "spans {spans:?}");
    }

    #[test]
    fn lex_spans_operators() {
        let _g = lock();
        let toks = lex_line_tokens("a && b || c");
        let ops: Vec<i32> = toks.iter().map(|t| t.tok).collect();
        assert!(ops.contains(&DAMPER), "toks {ops:?}");
        assert!(ops.contains(&DBAR), "toks {ops:?}");
        // Operator spans point at the operator text.
        let damper = toks.iter().find(|t| t.tok == DAMPER).unwrap();
        let src: String = "a && b || c"
            .chars()
            .skip(damper.start)
            .take(damper.end - damper.start)
            .collect();
        assert_eq!(src.trim(), "&&");
    }

    #[test]
    fn lex_spans_cmdpos_flag() {
        let _g = lock();
        let toks = lex_line_tokens("echo foo; ls bar");
        let strings: Vec<(String, bool)> = toks
            .iter()
            .filter(|t| t.tok == STRING_LEX)
            .map(|t| (t.clean_text(), t.cmdpos))
            .collect();
        assert_eq!(
            strings,
            vec![
                ("echo".to_owned(), true),
                ("foo".to_owned(), false),
                ("ls".to_owned(), true),
                ("bar".to_owned(), false)
            ]
        );
    }

    /// Unterminated quote at cursor: LEXFLAGS_ACTIVE keeps it a STRING (the
    /// zle_tricky c:1215-1227 fixup).
    #[test]
    fn lex_tolerates_unterminated_quote() {
        let _g = lock();
        let toks = lex_line_tokens("echo 'in progress");
        assert!(
            toks.iter().filter(|t| t.tok == STRING_LEX).count() >= 2,
            "unterminated quote must still lex as STRING: {:?}",
            toks.iter().map(|t| t.tok).collect::<Vec<_>>()
        );
    }

    // fish:475-691 — color_string_internal coverage, zsh quoting.
    #[test]
    fn color_string_quotes_and_vars() {
        let _g = lock();
        let s = "a'q'\"d$V\"$X*";
        let n = s.chars().count();
        let mut colors = vec![HighlightSpec::default(); n];
        color_string_internal(s, HighlightSpec::with_fg(HighlightRole::param), &mut colors);
        let roles: Vec<HighlightRole> = colors.iter().map(|c| c.foreground).collect();
        // a
        assert_eq!(roles[0], HighlightRole::param);
        // 'q'
        assert_eq!(roles[1], HighlightRole::quote);
        assert_eq!(roles[2], HighlightRole::quote);
        assert_eq!(roles[3], HighlightRole::quote);
        // "d — quote
        assert_eq!(roles[4], HighlightRole::quote);
        assert_eq!(roles[5], HighlightRole::quote);
        // $V inside dquotes — operator
        assert_eq!(roles[6], HighlightRole::operat);
        assert_eq!(roles[7], HighlightRole::operat);
        // closing "
        assert_eq!(roles[8], HighlightRole::quote);
        // $X unquoted — operator
        assert_eq!(roles[9], HighlightRole::operat);
        assert_eq!(roles[10], HighlightRole::operat);
        // * glob — operator
        assert_eq!(roles[11], HighlightRole::operat);
    }

    #[test]
    fn color_string_unclosed_quote_is_error() {
        let _g = lock();
        let s = "'unclosed";
        let n = s.chars().count();
        let mut colors = vec![HighlightSpec::default(); n];
        color_string_internal(s, HighlightSpec::with_fg(HighlightRole::param), &mut colors);
        assert_eq!(colors[0].foreground, HighlightRole::error); // fish:687-690
    }

    #[test]
    fn color_string_dollar_quote_escapes() {
        let _g = lock();
        // $'\x41' valid escape; $'\U110000' exceeds max → error (fish:583-586).
        let s = "$'\\x41'";
        let n = s.chars().count();
        let mut colors = vec![HighlightSpec::default(); n];
        color_string_internal(s, HighlightSpec::with_fg(HighlightRole::param), &mut colors);
        let roles: Vec<HighlightRole> = colors.iter().map(|c| c.foreground).collect();
        assert_eq!(roles[2], HighlightRole::escape, "roles {roles:?}");

        let s2 = "$'\\U110000'";
        let n2 = s2.chars().count();
        let mut colors2 = vec![HighlightSpec::default(); n2];
        color_string_internal(
            s2,
            HighlightSpec::with_fg(HighlightRole::param),
            &mut colors2,
        );
        assert_eq!(colors2[2].foreground, HighlightRole::error);
    }

    #[test]
    fn color_variable_subscript() {
        let _g = lock();
        let s: Vec<char> = "$arr[1]x".chars().collect();
        let mut colors = vec![HighlightSpec::default(); s.len()];
        let consumed = color_variable(&s, &mut colors);
        // $arr[1] consumed; trailing x not.
        assert_eq!(consumed, 7);
        assert_eq!(colors[0].foreground, HighlightRole::operat);
        assert_eq!(colors[4].foreground, HighlightRole::operat); // [
        assert_eq!(colors[6].foreground, HighlightRole::operat); // ]
    }

    /// zshrs's EXTENSION builtins (`provenance`, `dbview`, `zcache`, …)
    /// live in EXT_BUILTIN_NAMES, not in `createbuiltintable`. The command
    /// check consulted only the core table, so every one of them painted
    /// as an unknown token even though `whence -w provenance` reports
    /// `builtin` and `${+builtins[provenance]}` is 1 in the same shell.
    ///
    /// The negative half matters just as much: `builtin_in_builtintab` is
    /// NOT a membership test on its own — `builtin_owning_module` returns
    /// None for an unknown name and its `None => true` arm reports every
    /// string as available. Calling it without the EXT_BUILTIN_NAMES
    /// membership check first turned literally every word green.
    #[test]
    fn extension_builtins_are_commands_but_unknown_words_are_not() {
        let _g = lock();
        let ctx = OperationContext::empty();

        for name in ["provenance", "dbview"] {
            let line = format!("{name} -m x");
            let mut colors = Vec::new();
            highlight_shell(&line, &mut colors, &ctx, true, None);
            assert_eq!(
                colors[0].foreground,
                HighlightRole::command,
                "{name} is an extension builtin and must colour as a command"
            );
        }

        // A near-miss prefix of a real extension builtin must NOT pass.
        for bogus in ["provenanc", "zzqwx", "dbvie"] {
            let line = format!("{bogus} arg");
            let mut colors = Vec::new();
            highlight_shell(&line, &mut colors, &ctx, true, None);
            assert_eq!(
                colors[0].foreground,
                HighlightRole::error,
                "{bogus} is not a command and must colour as unknown-token"
            );
        }
    }

    // fish:1342-1821 — the highlight_shell integration checks, zsh syntax.
    #[test]
    fn highlight_valid_and_invalid_command() {
        let _g = lock();
        let ctx = OperationContext::empty();

        // `echo` resolves via builtintab → command color.
        let line = "echo hi";
        let mut colors = Vec::new();
        highlight_shell(line, &mut colors, &ctx, /*io_ok=*/ true, None);
        assert_eq!(colors.len(), line.chars().count());
        assert_eq!(
            colors[0].foreground,
            HighlightRole::command,
            "colors {colors:?}"
        );

        // Nonexistent command → error color.
        let line2 = "definitely_not_a_cmd_zshrs_x hi";
        let mut colors2 = Vec::new();
        highlight_shell(line2, &mut colors2, &ctx, /*io_ok=*/ true, None);
        assert_eq!(colors2[0].foreground, HighlightRole::error);
        // argument keeps param color
        let arg_pos = line2.chars().count() - 1;
        assert_eq!(colors2[arg_pos].foreground, HighlightRole::param);
    }

    #[test]
    fn highlight_reserved_word_and_separator() {
        let _g = lock();
        let ctx = OperationContext::empty();
        let line = "if true; then echo x; fi";
        let mut colors = Vec::new();
        highlight_shell(line, &mut colors, &ctx, true, None);
        // `if` keyword
        assert_eq!(colors[0].foreground, HighlightRole::keyword, "{colors:?}");
        // `;` statement terminator
        let semi = line.chars().position(|c| c == ';').unwrap();
        assert_eq!(colors[semi].foreground, HighlightRole::statement_terminator);
    }

    #[test]
    fn highlight_assignment_equals_operator() {
        let _g = lock();
        let ctx = OperationContext::empty();
        let line = "FOO=bar echo x";
        let mut colors = Vec::new();
        highlight_shell(line, &mut colors, &ctx, true, None);
        let eq = line.chars().position(|c| c == '=').unwrap();
        assert_eq!(colors[eq].foreground, HighlightRole::operat, "{colors:?}");
    }

    #[test]
    fn highlight_io_off_assumes_valid() {
        let _g = lock();
        let ctx = OperationContext::empty();
        // fish:1047-1049 — io_ok=false: no validity IO, command assumed valid.
        let line = "definitely_not_a_cmd_zshrs_x";
        let mut colors = Vec::new();
        highlight_shell(line, &mut colors, &ctx, /*io_ok=*/ false, None);
        assert_eq!(colors[0].foreground, HighlightRole::command);
    }

    #[test]
    fn resolver_default_palette() {
        let _g = lock();
        // Without user config, the resolver must produce non-zero attrs for the
        // roles with non-"none" defaults.
        let attr = HighlightColorResolver::resolve_spec_uncached(&HighlightSpec::with_fg(
            HighlightRole::command,
        ));
        assert_ne!(attr, 0, "command role must default to a visible style");
        let err = HighlightColorResolver::resolve_spec_uncached(&HighlightSpec::with_fg(
            HighlightRole::error,
        ));
        assert_ne!(err, 0);
        assert_ne!(attr, err, "command and error styles must differ");
    }

    #[test]
    fn contains_pending_variable_matches_dollar_use() {
        // fish:1134-1155
        let vars = vec!["x".to_owned()];
        assert!(contains_pending_variable(&vars, "$x"));
        assert!(contains_pending_variable(&vars, "dir/$x/log"));
        assert!(!contains_pending_variable(&vars, "x"));
        assert!(!contains_pending_variable(&vars, "$xy"));
    }

    #[test]
    fn locate_cmdsubst_span_finds_nested() {
        let chars: Vec<char> = "a$(b $(c) d)e".chars().collect();
        let (open, close) = locate_cmdsubst_span(&chars, 0).unwrap();
        assert_eq!(open, 1);
        assert_eq!(close, Some(11));
        // unterminated
        let chars2: Vec<char> = "a$(b".chars().collect();
        let (o2, c2) = locate_cmdsubst_span(&chars2, 0).unwrap();
        assert_eq!(o2, 1);
        assert_eq!(c2, None);
    }
}
