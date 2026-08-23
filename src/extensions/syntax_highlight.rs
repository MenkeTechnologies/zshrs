//! Port of fish-shell's `highlight/highlight.rs` (vendor/fish/highlight/highlight.rs) —
//! native command-line syntax highlighting.
//!
//! fish:1 — Functions for syntax highlighting.
//!
//! The zsh script plugins (zsh-syntax-highlighting, fast-syntax-highlighting) are
//! script-level recreations of this fish engine; this ports the origin directly.
//!
//! WHAT IS FISH AND WHAT IS NOT: the LEXING and the walk are fish's. The PALETTE
//! and the word-level classification are `fast-syntax-highlighting`'s, because
//! f-sy-h is the plugin this engine replaces on a daily-driver rc and the two
//! disagreed almost everywhere fish's dozen faces met f-sy-h's sixty style keys.
//! Every such site carries a `fast-highlight:NNN` citation (line numbers into
//! `~/.zinit/plugins/zdharma-continuum---fast-syntax-highlighting/fast-highlight`,
//! whose `FAST_HIGHLIGHT_STYLES` defaults start at :58) next to the `fish:NNN`
//! one it overrides. The differences are structural, not cosmetic:
//!   * fish paints glob / brace / paren / `$` characters one at a time; f-sy-h
//!     resolves the WHOLE word to one style (fast-highlight:1040-1090) and then
//!     runs a separate brackets pass that colours every bracket by nesting depth
//!     (fast-string-highlight:1-70).
//!   * fish asks "is this command valid" (a boolean); f-sy-h asks WHAT it is, in
//!     one fixed order — alias, global alias, function, builtin, command, suffix
//!     alias, reserved word, directory (fast-highlight:295-322) — and each answer
//!     has its own style key.
//!   * f-sy-h re-highlights the body of a command substitution under a SECOND
//!     palette (fast-highlight:113, the `free` theme), and runs per-command
//!     argument highlighters ("chromas", fast-highlight:171-232).
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
use crate::ported::hashtable::{aliastab_lock, cmdnamtab_lock, reswdtab_lock, sufaliastab_lock};
use crate::ported::lex::{
    ctxtlex, incmdpos, inredir, tok, tokstr, untokenize, LEX_LEXFLAGS, LEX_WORDBEG,
};
use crate::ported::params::{gethkparam, gethparam, getsparam};
use crate::ported::prompt::match_highlight;
use crate::ported::utils::getshfunc;
use crate::ported::zsh_h::{
    isset, lextok, zattr, AMPER, AMPERBANG, AUTOCD, BAR_TOK, CASE, CLOBBER, DAMPER, DBAR, DINANG,
    DINANGDASH, DINBRACK, DINPAR, DOUTANG, DOUTANGAMP, DOUTANGAMPBANG, DOUTANGBANG, DOUTBRACK,
    BANG_TOK, BARAMP, DOUTPAR, DSEMI, ENDINPUT, ENVARRAY, ENVSTRING, INANGAMP, INANG_TOK, INBRACE_TOK, INOUTANG,
    INOUTPAR, INPAR_TOK,
    INTERACTIVECOMMENTS, IS_REDIROP, LEXERR, LEXFLAGS_ACTIVE, LEXFLAGS_ZLE, NEWLIN, OUTANGAMP,
    OUTANGAMPBANG, OUTANGBANG, OUTANG_TOK, OUTBRACE_TOK, OUTPAR_TOK, SEMI, SEMIAMP, SEMIBAR, SEPER,
    STRING_LEX, TRINANG, TYPESET,
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
    /// fast-highlight:113 / :890-899 — f-sy-h flips `FAST_THEME_NAME` to the
    /// `secondary` theme (`free` by default) before re-highlighting the body of
    /// a command substitution, so `print $(ls)` shows `ls` in the secondary
    /// palette's command colour, not the primary green.  This flag selects that
    /// palette for one span; fish has no such notion.
    pub secondary: bool,
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

    // ------------------------------------------------------------------
    // fast-syntax-highlighting roles (fast-highlight:58-113, the
    // `FAST_HIGHLIGHT_STYLES` default theme).  fish has no counterpart for
    // these — its palette is a dozen faces where f-sy-h's is sixty — but
    // f-sy-h is what this engine replaces on a daily-driver rc, so every
    // style key it paints needs a role here or the two cannot agree.
    // ------------------------------------------------------------------
    precommand,          // fast-highlight:67  fg=green
    alias_,              // fast-highlight:61  fg=green
    suffix_alias,        // fast-highlight:62  fg=green
    global_alias,        // fast-highlight:63  bg=blue
    builtin_,            // fast-highlight:64  fg=green
    function_,           // fast-highlight:65  fg=green
    hashed_command,      // fast-highlight:69  fg=green
    path,                // fast-highlight:70  fg=magenta
    path_to_dir,         // fast-highlight:71  fg=magenta,underline
    globbing,            // fast-highlight:73  fg=blue,bold
    history_expansion,   // fast-highlight:75  fg=blue,bold
    double_hyphen_option, // fast-highlight:77 fg=cyan
    dquoted,             // fast-highlight:80  fg=yellow (double-quoted-argument)
    dollar_quoted,       // fast-highlight:81  fg=yellow (dollar-quoted-argument)
    dollar_in_dquote,    // fast-highlight:82  fg=cyan
    variable,            // fast-highlight:86  fg=113
    mathvar,             // fast-highlight:87  fg=blue,bold
    mathnum,             // fast-highlight:88  fg=magenta
    matherr,             // fast-highlight:89  fg=red
    assign,              // fast-highlight:84  none
    assign_array_bracket, // fast-highlight:90 fg=green
    here_string_tri,     // fast-highlight:96  fg=yellow
    here_string_text,    // fast-highlight:97  bg=18
    single_sq_bracket,   // fast-highlight:106 fg=green
    double_sq_bracket,   // fast-highlight:107 fg=green
    double_paren,        // fast-highlight:108 fg=yellow
    bracket_level_1,     // fast-highlight:103 fg=green,bold
    bracket_level_2,     // fast-highlight:104 fg=yellow,bold
    bracket_level_3,     // fast-highlight:105 fg=cyan,bold
    case_input,          // fast-highlight:99  fg=green
    case_parentheses,    // fast-highlight:100 fg=yellow
    case_condition,      // fast-highlight:101 bg=blue
    globbing_ext,        // fast-highlight:74  fg=13
    here_string_var,     // fast-highlight:98  fg=cyan,bg=18
    for_loop_variable,   // fast-highlight:92  none
    for_loop_operator,   // fast-highlight:93  fg=yellow
    for_loop_number,     // fast-highlight:94  fg=magenta
    for_loop_separator,  // fast-highlight:95  fg=yellow,bold
    correct_subtle,      // fast-highlight:109 fg=12
    incorrect_subtle,    // fast-highlight:110 fg=red
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

        // f-sy-h keys (fast-highlight:58-113).
        HighlightRole::precommand => "precommand",
        HighlightRole::alias_ => "alias",
        HighlightRole::suffix_alias => "suffix-alias",
        HighlightRole::global_alias => "global-alias",
        HighlightRole::builtin_ => "builtin",
        HighlightRole::function_ => "function",
        HighlightRole::hashed_command => "hashed-command",
        HighlightRole::path => "path",
        HighlightRole::path_to_dir => "path-to-dir",
        HighlightRole::globbing => "globbing",
        HighlightRole::history_expansion => "history-expansion",
        HighlightRole::double_hyphen_option => "double-hyphen-option",
        HighlightRole::dquoted => "double-quoted-argument",
        HighlightRole::dollar_quoted => "dollar-quoted-argument",
        HighlightRole::dollar_in_dquote => "back-or-dollar-double-quoted-argument",
        HighlightRole::variable => "variable",
        HighlightRole::mathvar => "mathvar",
        HighlightRole::mathnum => "mathnum",
        HighlightRole::matherr => "matherr",
        HighlightRole::assign => "assign",
        HighlightRole::assign_array_bracket => "assign-array-bracket",
        HighlightRole::here_string_tri => "here-string-tri",
        HighlightRole::here_string_text => "here-string-text",
        HighlightRole::single_sq_bracket => "single-sq-bracket",
        HighlightRole::double_sq_bracket => "double-sq-bracket",
        HighlightRole::double_paren => "double-paren",
        HighlightRole::bracket_level_1 => "bracket-level-1",
        HighlightRole::bracket_level_2 => "bracket-level-2",
        HighlightRole::bracket_level_3 => "bracket-level-3",
        HighlightRole::case_input => "case-input",
        HighlightRole::case_parentheses => "case-parentheses",
        HighlightRole::case_condition => "case-condition",
        HighlightRole::globbing_ext => "globbing-ext",
        HighlightRole::here_string_var => "here-string-var",
        HighlightRole::for_loop_variable => "for-loop-variable",
        HighlightRole::for_loop_operator => "for-loop-operator",
        HighlightRole::for_loop_number => "for-loop-number",
        HighlightRole::for_loop_separator => "for-loop-separator",
        HighlightRole::correct_subtle => "correct-subtle",
        HighlightRole::incorrect_subtle => "incorrect-subtle",

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
        // fast-highlight:64-69 — builtin / function / command / precommand /
        // hashed-command / alias / suffix-alias are all fg=green.
        HighlightRole::command
        | HighlightRole::precommand
        | HighlightRole::alias_
        | HighlightRole::suffix_alias
        | HighlightRole::builtin_
        | HighlightRole::function_
        | HighlightRole::hashed_command => "fg=green",
        HighlightRole::global_alias => "bg=blue", // fast-highlight:63
        HighlightRole::keyword => "fg=yellow",    // fast-highlight:59 reserved-word
        HighlightRole::comment => "fg=black,bold", // fast-highlight:85
        // Anything still routed through the generic operator role paints like a
        // glob, which is what f-sy-h does for a word carrying one.
        HighlightRole::operat | HighlightRole::globbing => "fg=blue,bold", // fast-highlight:73
        HighlightRole::escape => "fg=cyan",                               // fast-highlight:83
        HighlightRole::quote | HighlightRole::dquoted | HighlightRole::dollar_quoted => {
            "fg=yellow" // fast-highlight:79-81
        }
        HighlightRole::redirection => "none", // fast-highlight:84
        HighlightRole::autosuggestion => "fg=8",
        HighlightRole::selection | HighlightRole::search_match => "standout",
        HighlightRole::option | HighlightRole::double_hyphen_option => "fg=cyan", // :76-77
        HighlightRole::path => "fg=magenta",                                      // :70
        HighlightRole::path_to_dir => "fg=magenta,underline",                     // :71
        HighlightRole::history_expansion => "fg=blue,bold",                       // :75
        HighlightRole::dollar_in_dquote => "fg=cyan",                             // :82
        HighlightRole::variable => "fg=113",                                      // :86
        HighlightRole::mathvar => "fg=blue,bold",                                 // :87
        HighlightRole::mathnum => "fg=magenta",                                   // :88
        HighlightRole::matherr => "fg=red",                                       // :89
        HighlightRole::assign => "none",                                          // :84
        HighlightRole::assign_array_bracket => "fg=green",                        // :90
        HighlightRole::here_string_tri => "fg=yellow",                            // :96
        HighlightRole::here_string_text => "bg=18",                               // :97
        HighlightRole::single_sq_bracket | HighlightRole::double_sq_bracket => "fg=green", // :106-107
        HighlightRole::double_paren => "fg=yellow",                               // :108
        HighlightRole::bracket_level_1 => "fg=green,bold",                        // :103
        HighlightRole::bracket_level_2 => "fg=yellow,bold",                       // :104
        HighlightRole::bracket_level_3 => "fg=cyan,bold",                         // :105
        HighlightRole::case_input => "fg=green",                                  // :99
        HighlightRole::case_parentheses => "fg=yellow",                           // :100
        HighlightRole::case_condition => "bg=blue",                               // :101
        HighlightRole::globbing_ext => "fg=13",                                   // :74
        HighlightRole::here_string_var => "fg=cyan,bg=18",                        // :98
        HighlightRole::for_loop_variable => "none",                               // :92
        HighlightRole::for_loop_operator => "fg=yellow",                          // :93
        HighlightRole::for_loop_number => "fg=magenta",                           // :94
        HighlightRole::for_loop_separator => "fg=yellow,bold",                    // :95
        HighlightRole::correct_subtle => "fg=12",                                 // :109
        HighlightRole::incorrect_subtle => "fg=red",                              // :110
        _ => "none",
    }
}

/// The `free` theme (themes/free.ini, materialised as
/// `$FAST_WORK_DIR/secondary_theme.zsh`), which is what
/// `FAST_HIGHLIGHT_STYLES[secondary]` names by default (fast-highlight:113).
/// Used for the body of a command substitution.
fn get_secondary_default_style(role: HighlightRole) -> &'static str {
    match role {
        HighlightRole::error => "fg=red,bold",
        HighlightRole::command
        | HighlightRole::precommand
        | HighlightRole::alias_
        | HighlightRole::suffix_alias
        | HighlightRole::builtin_
        | HighlightRole::function_
        | HighlightRole::hashed_command
        | HighlightRole::case_input => "fg=180",
        HighlightRole::keyword => "fg=150",
        HighlightRole::global_alias | HighlightRole::case_condition => "bg=19",
        HighlightRole::path => "fg=166",
        HighlightRole::path_to_dir => "fg=166,underline",
        HighlightRole::operat | HighlightRole::globbing => "fg=112",
        HighlightRole::history_expansion => "fg=blue,bold",
        HighlightRole::option | HighlightRole::double_hyphen_option => "fg=110",
        HighlightRole::quote | HighlightRole::dquoted | HighlightRole::dollar_quoted => "fg=150",
        HighlightRole::escape | HighlightRole::dollar_in_dquote => "fg=110",
        HighlightRole::comment => "fg=black,bold",
        HighlightRole::variable => "none",
        HighlightRole::mathvar => "fg=blue,bold",
        HighlightRole::mathnum => "fg=166",
        HighlightRole::matherr => "fg=red",
        HighlightRole::assign_array_bracket => "fg=180",
        HighlightRole::here_string_tri => "fg=yellow",
        HighlightRole::here_string_text => "bg=19",
        HighlightRole::single_sq_bracket | HighlightRole::double_sq_bracket => "fg=180",
        HighlightRole::double_paren => "fg=150",
        HighlightRole::case_parentheses => "fg=116",
        HighlightRole::globbing_ext => "fg=118",
        HighlightRole::here_string_var => "fg=110,bg=19",
        HighlightRole::for_loop_variable => "none",
        HighlightRole::for_loop_operator => "fg=150",
        HighlightRole::for_loop_number => "fg=150",
        HighlightRole::for_loop_separator => "fg=109",
        HighlightRole::correct_subtle => "bg=55",
        HighlightRole::incorrect_subtle => "bg=52",
        HighlightRole::bracket_level_1 => "fg=130",
        HighlightRole::bracket_level_2 => "fg=70",
        HighlightRole::bracket_level_3 => "fg=69",
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
        // Every f-sy-h key carries its own default (fast-highlight:58-113 is a
        // flat `: ${FAST_HIGHLIGHT_STYLES[key]:=…}` list with no inheritance),
        // so these roles fall back to THEMSELVES: the resolver then collapses
        // the chain to [role] and lands on the built-in default rather than
        // borrowing whatever the user put in `default`.
        HighlightRole::keyword
        | HighlightRole::option
        | HighlightRole::precommand
        | HighlightRole::alias_
        | HighlightRole::suffix_alias
        | HighlightRole::global_alias
        | HighlightRole::builtin_
        | HighlightRole::function_
        | HighlightRole::hashed_command
        | HighlightRole::path
        | HighlightRole::path_to_dir
        | HighlightRole::globbing
        | HighlightRole::history_expansion
        | HighlightRole::double_hyphen_option
        | HighlightRole::dquoted
        | HighlightRole::dollar_quoted
        | HighlightRole::dollar_in_dquote
        | HighlightRole::variable
        | HighlightRole::mathvar
        | HighlightRole::mathnum
        | HighlightRole::matherr
        | HighlightRole::assign
        | HighlightRole::assign_array_bracket
        | HighlightRole::here_string_tri
        | HighlightRole::here_string_text
        | HighlightRole::single_sq_bracket
        | HighlightRole::double_sq_bracket
        | HighlightRole::double_paren
        | HighlightRole::bracket_level_1
        | HighlightRole::bracket_level_2
        | HighlightRole::bracket_level_3
        | HighlightRole::case_input
        | HighlightRole::case_parentheses
        | HighlightRole::case_condition
        | HighlightRole::globbing_ext
        | HighlightRole::here_string_var
        | HighlightRole::for_loop_variable
        | HighlightRole::for_loop_operator
        | HighlightRole::for_loop_number
        | HighlightRole::for_loop_separator
        | HighlightRole::correct_subtle
        | HighlightRole::incorrect_subtle => role,
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

/// Read `$ZSH_HIGHLIGHT_STYLES[key]`.
///
/// This looked like a flat key/value sequence and was read with
/// `gethparam(…).chunks_exact(2)` — but `gethparam` is
/// `paramvalarr(…, SCANPM_WANTVALS)` (params.rs:6300, zsh Src/params.c:3118):
/// VALUES ONLY.  The chunk walk therefore compared a style string against the
/// requested key, never matched, and every `$ZSH_HIGHLIGHT_STYLES` entry a user
/// set was silently ignored.  Keys come from `gethkparam`
/// (`SCANPM_WANTKEYS`, params.rs:6417); both scan the same hash in the same
/// order, which is exactly what `${(kv)h}` relies on.
fn zsh_highlight_styles_get(key: &str) -> Option<String> {
    let keys = gethkparam("ZSH_HIGHLIGHT_STYLES")?;
    let vals = gethparam("ZSH_HIGHLIGHT_STYLES")?;
    keys.iter()
        .zip(vals.iter())
        .find(|(k, _)| k.as_str() == key)
        .map(|(_, v)| v.clone())
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
        let secondary = highlight.secondary;
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
                } else if secondary {
                    // f-sy-h keys the secondary theme by prefixing the theme
                    // name (`$FAST_HIGHLIGHT_STYLES[freecommand]`); the z-sy-h
                    // config surface this engine reads has no theme names, so
                    // the same idea is spelled `secondary-<key>`.
                    zsh_highlight_styles_get(&format!(
                        "secondary-{}",
                        get_highlight_style_key(role)
                    ))
                } else {
                    zsh_highlight_styles_get(get_highlight_style_key(role))
                };
                if let Some(face) = configured.as_deref().and_then(parse_style_for_highlight) {
                    return face;
                }
            }
            // No user config anywhere in the chain: built-in default palette.
            let spec = if secondary {
                get_secondary_default_style(role)
            } else {
                get_default_style(role)
            };
            parse_style_for_highlight(spec).unwrap_or(0)
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

/// fast-highlight:1168 — `${+parameters[$name]}`: does a parameter of this name
/// exist?  `getsparam` alone answers only for scalars, so ask the parameter
/// table directly the way `typeset -p`'s lookup does.
fn math_name_exists(name: &str) -> bool {
    crate::ported::params::paramtab()
        .read()
        .map(|t| t.get(name).is_some())
        .unwrap_or(false)
}

/// fast-highlight:1273 — `: ${expanded_path:=${(Q)~__arg}}`: quote removal plus
/// tilde expansion, and NOTHING that can execute. Returns None when the word
/// still carries an expansion or globbing marker (f-sy-h's `${(Q)~…}` leaves
/// those unresolved and the `-e`/`-d` tests then fail).
///
/// This must NOT go through `expand_one_no_cmdsubst`: that calls `singsub`,
/// which runs a command substitution mid-keystroke — typing
/// `print "a $(ls) b"` executed `l` at the moment the buffer read
/// `print "a $(l`.
fn fsh_expand_for_path(tokenized: &str) -> Option<String> {
    use crate::ported::zsh_h::{Bnull, Bnullkeep, Comma, Dash, Dnull, Nularg, Snull, Tilde};
    let mut s = tokenized.to_owned();
    // `=cmd` — zsh EQUALS expansion. `${(Q)~=ls}` yields the command's path, so
    // f-sy-h styles `ls =ls` with a magenta `=ls`.
    if let Some(rest) = s.strip_prefix(crate::ported::zsh_h::Equals) {
        return crate::ported::exec::findcmd(rest, 0, 0);
    }
    if s.starts_with(Tilde) {
        // Must go through the QuietErrs-wrapped helper: a bare `filesubstr`
        // raises zsh's errflag for `~nosuchuser`, and an errflag raised while
        // ZLE is repainting aborts the read loop (typing `ls ~root` froze the
        // line at `ls ~`).
        crate::zle_file_tester::expand_tilde_quiet(&mut s);
    }
    // zle_file_tester.rs:330-350 (`is_potential_path`) — the same
    // quote-null-is-fine / anything-else-in-the-ITOK-range-is-magic split.
    let mut out = String::new();
    for c in s.chars() {
        match c {
            Snull | Dnull | Bnull | Bnullkeep | Nularg => (),
            Tilde => out.push('~'),
            Comma => out.push(','),
            Dash => out.push('-'),
            c if ('\u{84}'..='\u{a1}').contains(&c) => return None,
            c => out.push(c),
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// The subset of f-sy-h's chroma table (fast-highlight:171-232) that this
/// engine implements.  The other 32 chromas — `git`, `grep`, `awk`, `docker`,
/// `ssh`, `make`, `printf`, the `-subcommand.ch` family and the rest — are not
/// ported: each is a separate argument-grammar script, and several of them
/// shell out or write files on every keystroke.
#[derive(Clone, Copy, PartialEq)]
pub enum Chroma {
    Autoload, // →chroma/-autoload.ch
    Source,   // →chroma/-source.ch  (`source` and `.`)
    Printf,   // →chroma/-printf.ch
}

impl Chroma {
    fn for_command(cmd: &str) -> Option<Chroma> {
        match cmd {
            "autoload" => Some(Chroma::Autoload),
            "source" | "." => Some(Chroma::Source),
            "printf" => Some(Chroma::Printf),
            _ => None,
        }
    }
}

/// -printf.ch:63 — the conversion pattern
/// `%[#+ 0-]#[0-9]#([.][0-9]#)(#c0,1)[diouxXfFeEgGaAcsb]`, as (start, end)
/// char ranges within a printf format word.
fn printf_conversions(w: &[char]) -> Vec<(usize, usize)> {
    const FLAGS: &str = "#+ 0-";
    const CONV: &str = "diouxXfFeEgGaAcsb";
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < w.len() {
        if w[i] != '%' {
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i + 1;
        while j < w.len() && FLAGS.contains(w[j]) {
            j += 1;
        }
        while j < w.len() && w[j].is_ascii_digit() {
            j += 1;
        }
        if j < w.len() && w[j] == '.' {
            j += 1;
            while j < w.len() && w[j].is_ascii_digit() {
                j += 1;
            }
        }
        if j < w.len() && CONV.contains(w[j]) {
            out.push((start, j + 1));
            i = j + 1;
        } else {
            i = start + 1;
        }
    }
    out
}

/// fast-highlight:884 — does the (tokenized) word contain a live quote?  The
/// lexer answers this for us: a quote that survived as a `Snull`/`Dnull` token
/// really opened a quoted section, while a `\"` is a plain character.
fn fsh_has_quote(tokenized: &str) -> bool {
    use crate::ported::zsh_h::{Dnull, Snull};
    tokenized.chars().any(|c| c == Snull || c == Dnull)
}

/// fast-highlight:1045 — `*([^\\]##|"(#b)"|"(#B)"|"(#m)"|"(#c")*`: does the word
/// carry an EXTENDED glob operator?
fn fsh_is_globbing_ext(word: &str) -> bool {
    if word.contains("(#b)") || word.contains("(#B)") || word.contains("(#m)") || word.contains("(#c")
    {
        return true;
    }
    // A `##` that is not backslash-escaped.
    let c: Vec<char> = word.chars().collect();
    (1..c.len()).any(|i| c[i] == '#' && c[i - 1] == '#' && (i < 2 || c[i - 2] != '\\'))
}

/// fast-highlight:562-563 — the option words that `command` / `exec` accept
/// while still leaving the NEXT word in command position: a `-` followed only
/// by characters from `set` (`-pvV-` for `command`, `-cla-` for `exec`).
fn fsh_afp_option(word: &str, set: &str) -> bool {
    let Some(rest) = word.strip_prefix('-') else {
        return false;
    };
    !rest.is_empty() && rest.chars().all(|c| set.contains(c))
}

/// fast-highlight:121-130 — the value-1 entries of
/// `__FAST_HIGHLIGHT_TOKEN_TYPES` (`builtin`, `command`, `exec`, `nocorrect`,
/// `noglob`, `pkexec`), plus `sudo`/`doas`, which fast-highlight:620-623
/// special-cases into the same `precommand` style with the same
/// "next word is still a command" effect.
///
/// fish's equivalent set is only `command`/`builtin`/`exec` (its
/// `StatementDecoration`), so `sudo ls` left `ls` uncoloured and `nocorrect`
/// took the reserved-word face.
pub fn fsh_is_precommand(word: &str) -> bool {
    matches!(
        word,
        "builtin" | "command" | "exec" | "nocorrect" | "noglob" | "pkexec" | "sudo" | "doas"
    )
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
fn color_variable(inp: &[char], colors: &mut [HighlightSpec], role: HighlightRole) -> usize {
    assert_eq!(inp[0], '$');

    let at = |i: usize| -> char { inp.get(i).copied().unwrap_or('\0') };

    // fish:413-429 — Handle an initial run of $s.
    let mut idx = 0;
    let mut dollar_count = 0;
    while at(idx) == '$' {
        // Our color depends on the next char.
        let next = at(idx + 1);
        if next == '$' || valid_var_name_char(next) || is_special_param_char(next) {
            colors[idx] = HighlightSpec::with_fg(role);
        } else if next == '(' || next == '{' || next == '\'' {
            // zsh: $(cmdsub), ${param}, $'ansi-quote' — the '$' is an operator and the
            // construct is handled by the caller / string scanner.
            colors[idx] = HighlightSpec::with_fg(role);
            return idx + 1;
        } else {
            colors[idx] = HighlightSpec::with_fg(HighlightRole::error);
        }
        idx += 1;
        dollar_count += 1;
    }

    // Single-char special param ($?, $#, …) — consume exactly one char.
    if idx == dollar_count && !valid_var_name_char(at(idx)) && is_special_param_char(at(idx)) {
        colors[idx] = HighlightSpec::with_fg(role);
        return idx + 1;
    }

    // fish:431-445 — Handle a sequence of variable characters.
    // It may contain an escaped newline - see fish#8444.
    loop {
        if valid_var_name_char(at(idx)) {
            colors[idx] = HighlightSpec::with_fg(role);
            idx += 1;
        } else if at(idx) == '\\' && at(idx + 1) == '\n' {
            colors[idx] = HighlightSpec::with_fg(role);
            idx += 1;
            colors[idx] = HighlightSpec::with_fg(role);
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
                colors[idx] = HighlightSpec::with_fg(role);
                colors[idx + slice_len - 1] = HighlightSpec::with_fg(role);
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
    // fish restricts this to three faces; f-sy-h resolves a word to one of a
    // dozen styles (alias, path, globbing, variable, assign, …) before the
    // string scanner runs, so the base can be any of them.  What must NOT reach
    // here is a face the scanner would overwrite on every character.
    assert!(
        !matches!(
            base_color.foreground,
            HighlightRole::quote
                | HighlightRole::dquoted
                | HighlightRole::dollar_quoted
                | HighlightRole::escape
        ),
        "Unexpected base color"
    );
    let chars: Vec<char> = buffstr.chars().collect();
    let buff_len = chars.len();
    colors.fill(base_color);

    // fish:489-493 — fish's %self special-case has no zsh spelling; dropped.

    #[derive(Eq, PartialEq)]
    #[allow(dead_code)]
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
                    // fish:509-593 colours a bare `\X` with its escape face.
                    // f-sy-h does not: outside a quote the backslash is part of
                    // the word and takes the word's own style
                    // (fast-highlight:1040-1090 has no backslash arm), so
                    // `print a\ b` was cyan here and plain in f-sy-h.  Still
                    // skip the escaped character so a `\'` or `\"` cannot open
                    // a quote.
                    in_pos += 1; // skip the escaped char; loop tail adds one more
                } else {
                    // fish:594-634 — Not a backslash.
                    //
                    // Everything fish paints per-character out here — `~`, `$`,
                    // `?`, `*`, `(`, `)`, `{`, `}`, `,`, `[`, `]` — is decided
                    // by f-sy-h at WORD granularity instead
                    // (fast-highlight:1040-1090: the whole word is `globbing`
                    // or `variable` or `path`), with the brackets pass painting
                    // bracket characters afterwards.  Painting them here as
                    // operators is what made `ls *.c` colour only the `*` and
                    // `print ${HOME}` colour only the punctuation.
                    match c {
                        '$' if chars.get(in_pos + 1) == Some(&'\'') => {
                            // zsh $'...' — enter dollar-quote mode; color both opener
                            // chars as quote (fast-highlight:879-882).
                            colors[in_pos] = HighlightSpec::with_fg(HighlightRole::dollar_quoted);
                            colors[in_pos + 1] =
                                HighlightSpec::with_fg(HighlightRole::dollar_quoted);
                            unclosed_quote_offset = Some(in_pos);
                            in_pos += 1;
                            mode = Mode::dollar_quoted;
                        }
                        '`' => {
                            // fast-highlight:81 — `back-quoted-argument` is
                            // `none`; only the contents are re-highlighted.
                            unclosed_quote_offset = Some(in_pos);
                            mode = Mode::backtick;
                        }
                        '\'' => {
                            colors[in_pos] = HighlightSpec::with_fg(HighlightRole::quote);
                            unclosed_quote_offset = Some(in_pos);
                            mode = Mode::single_quoted;
                        }
                        '"' => {
                            // fast-highlight:80 — the opening quote is part of
                            // the `double-quoted-argument` run, not the
                            // single-quoted one.
                            colors[in_pos] = HighlightSpec::with_fg(HighlightRole::dquoted);
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
                    // fast-highlight:80 — `double-quoted-argument` is its own
                    // style key, distinct from `single-quoted-argument`.
                    colors[in_pos] = HighlightSpec::with_fg(HighlightRole::dquoted);
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
                        // fast-highlight:82 / :1358 — a `$…` inside `"…"` takes
                        // `back-or-dollar-double-quoted-argument` (cyan), not
                        // the generic operator face.
                        in_pos += color_variable(
                            &chars[in_pos..],
                            &mut colors[in_pos..],
                            HighlightRole::dollar_in_dquote,
                        );
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
                // fast-highlight:81 — `dollar-quoted-argument`.
                colors[in_pos] = HighlightSpec::with_fg(HighlightRole::dollar_quoted);
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
                        let first_digit = in_pos;
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

                        // fish:583-586 errors when the escape exceeds the code
                        // point maximum.  f-sy-h's `-fast-highlight-dollar-string`
                        // (fast-highlight:1198-1230) validates only the SHAPE —
                        // `\x`/`\X`/`\u`/`\U` must be followed by at least one hex
                        // digit — and never the value, so `$'\U110000'` is a
                        // plain escape there and was red here.
                        let _ = max_val;
                        if res == 0 && in_pos == first_digit && matches!(escaped_char, 'u' | 'U' | 'x' | 'X') {
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
                // fast-highlight:81 — `back-quoted-argument` is `none`: the
                // ticks and their contents keep the word's own base face here,
                // and the caller recurses into the body.  fish painted the ticks
                // as operators and the body as a quoted string.
                if c == '`' {
                    mode = Mode::unquoted;
                }
            }
        }
        in_pos += 1;
    }

    // fish:687-690 marks the opening quote of an unterminated string with its
    // error face.  f-sy-h does not: an in-progress `print 'abc` is styled
    // `single-quoted-argument` all the way to the cursor
    // (fast-highlight:884-960 has no unterminated branch), and flagging it red
    // meant every string was red until its closing quote was typed.
    let _ = (mode, unclosed_quote_offset);
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

    // Alias expansion has to be OFF for this walk. `checkalias()` pushes the
    // alias BODY into the input stack, so `inbufct` (which the span arithmetic
    // subtracts from) counts the expansion's characters, not the typed word's:
    // a 7-character `myalias` came back as a 6-character span and the last
    // character stayed uncoloured. f-sy-h never expands either — it looks the
    // word up in `$aliases` and styles it (fast-highlight:300-303 / :668-676).
    let saved_noaliases = crate::ported::lex::noaliases();
    crate::ported::lex::set_noaliases(true);

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

    crate::ported::lex::set_noaliases(saved_noaliases);

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
    /// fast-string-highlight:24 `_FAST_COMPLEX_BRACKETS` — char offsets of
    /// brackets the main pass already styled structurally (`[[`, `]]`, `((`,
    /// `))`, `[`, `]`, the parens of an array assignment).  The brackets pass
    /// skips these instead of repainting them by nesting level.
    complex_brackets: Vec<usize>,
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
            complex_brackets: vec![],
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

        // fast-syntax-highlighting.plugin.zsh:96-98 — after the main pass,
        // `-fast-highlight-string-process` repaints every bracket by nesting
        // level.  fish has nothing like it; without it `(ls)`, `{ ls }`,
        // `${HOME}`, `$path[1]` and `*(.)` all came out the wrong colour.
        self.highlight_brackets();

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
        // fast-highlight:385/478/492 — `highlight_glob` starts at 1 for every
        // command and is cleared by `noglob`, so `noglob echo *` leaves the
        // `*` plain instead of painting it as a glob.
        let mut highlight_glob = true;
        // fast-highlight:1004-1008 — `<<<` sets BIT_here_string (128) on the
        // NEXT word, which is then painted `here-string-text` rather than
        // treated as a filename.
        let mut here_string_word = false;
        // fast-highlight:701-710 — `name=(` opens an array assignment; the
        // matching `)` is an `assign-array-bracket`, not a subshell paren.
        let mut in_array_assignment = false;
        // fast-highlight:659-664 — `for` and `function` put a NAME, not a
        // command, in the next word slot, and `case`'s next word is the
        // subject expression.  Without this the zsh lexer's `incmdpos` flag
        // routes `in` (of `for i in …`) and `f` (of `function f`) through the
        // command-word validator, which paints them unknown-token red.
        let mut name_word = false;
        // fast-highlight:659-660 — BIT_for: from `for` up to the `;`, every word
        // is a loop variable or list element, never a command.
        let mut for_mode = false;
        // fast-highlight:662-663 / :488-500 — the `case` word ladder:
        // subject → `in` → pattern → `)` → body → `;;` → pattern …
        #[derive(PartialEq, Clone, Copy)]
        enum CaseState {
            None_,
            Subject,
            In,
            Pattern,
            Body,
        }
        let mut case_state = CaseState::None_;
        // fast-highlight:543-557 — after `sudo`/`doas`, `-x` words stay options
        // and command position slides along; the flags in `-Cgprtu` swallow one
        // argument first (BIT_sudo_arg).
        let mut sudo_opt = false;
        let mut sudo_arg = false;
        // fast-highlight:561-563 (BIT_afpcmd) — `command -pvV…` / `exec -cla…`
        // likewise keep command position.
        let mut afp: Option<String> = None;
        // fast-highlight:1188-1199 — `repeat` swallows its count word like a
        // redirection target and leaves `this_word = 3`, which makes the NEXT
        // word resolve as a command but style an UNRESOLVED one `default`
        // instead of `unknown-token` (fast-highlight:795-796, `this_word & 14`).
        let mut repeat_count = false;
        let mut soft_unknown = false;
        // fast-highlight:653-654 — inside `[[ … ]]` nothing is a command word.
        let mut in_cond = false;
        // fast-highlight:612-615 — `always` is only a reserved word right after
        // the `}` that closed a `{ … }` block.
        let mut after_close_brace = false;
        // fast-highlight:686 — `eval` sets BIT_eval (256); fast-highlight:884-899
        // then re-highlights each quoted argument's BODY under the secondary
        // theme, leaving the quotes themselves unstyled.
        let mut eval_mode = false;
        // fast-highlight:171-232 — `FAST_HIGHLIGHT[chroma-<cmd>]`: per-command
        // argument highlighters.  Only the two that a zshrc is built out of are
        // ported (see `Chroma`); the other 32 are not.
        let mut chroma: Option<Chroma> = None;
        let mut chroma_words = 0usize;
        let mut chroma_skip = false;
        let mut i = 0;
        while i < toks.len() {
            if self.ctx.check_cancel() {
                return;
            }
            let t = &toks[i];
            let tokv = t.tok;

            if IS_REDIROP(tokv) {
                // fast-highlight:1004-1008 — `<<<` is a here-string triple, not
                // a redirection to a file: the operator takes `here-string-tri`
                // and the word after it is text, not a filename.
                if tokv == TRINANG {
                    self.color_span(t.start, t.end, HighlightRole::here_string_tri);
                    here_string_word = true;
                    i += 1;
                    continue;
                }
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
                // fish:912-921 — End/Pipe/Background/AndAnd/OrOr.
                //
                // fast-highlight:152-158 lists `|`, `||`, `;`, `&`, `&&`, `|&`,
                // `&!`, `&|` as ONE class (__arg_type 3) painted
                // `commandseparator`, whose default is `none`
                // (fast-highlight:68) — fish splits them into a terminator face
                // and an operator face, and this engine's operator face is the
                // glob colour, so `ls && ls` came out with a blue `&&`.
                SEPER | NEWLIN | SEMI | AMPER | AMPERBANG | BAR_TOK | BARAMP | DBAR | DAMPER => {
                    self.color_span(t.start, t.end, HighlightRole::statement_terminator);
                    decoration = StatementDecoration::None_;
                    expanded_cmd.clear();
                    is_cd = false;
                    is_typeset = false;
                    have_dashdash = false;
                    highlight_glob = true; // fast-highlight:492
                    name_word = false;
                    for_mode = false; // fast-highlight:482
                    sudo_opt = false;
                    sudo_arg = false;
                    afp = None;
                    repeat_count = false;
                    soft_unknown = false;
                    after_close_brace = false;
                    eval_mode = false;
                    chroma = None;
                    chroma_words = 0;
                    chroma_skip = false;
                }
                DSEMI | SEMIAMP | SEMIBAR => {
                    // fast-highlight:1058-1060 — `;;` / `;&` / `;|` end a case
                    // item and put the next word back in pattern position.
                    if case_state == CaseState::Body {
                        case_state = CaseState::Pattern;
                    }
                    expanded_cmd.clear();
                    is_cd = false;
                    is_typeset = false;
                    have_dashdash = false;
                    highlight_glob = true;
                }
                INPAR_TOK | OUTPAR_TOK => {
                    // fast-highlight:785-793 — a subshell paren is styled
                    // `reserved-word`; fast-highlight:840-847 — the `)` closing
                    // an array assignment is an `assign-array-bracket`.  Both
                    // are then overpainted by the brackets pass unless they were
                    // recorded as "complex" (fast-string-highlight:24).
                    if tokv == OUTPAR_TOK && in_array_assignment {
                        // fast-highlight:840-847 — the `)` closing an array
                        // assignment.
                        in_array_assignment = false;
                        self.color_span(t.start, t.end, HighlightRole::assign_array_bracket);
                        self.complex_brackets.push(t.start);
                    } else if tokv == OUTPAR_TOK && case_state == CaseState::Pattern {
                        // fast-highlight:100 — the `)` closing a case pattern is
                        // `case-parentheses`, and it is not a bracket pair the
                        // brackets pass should claim.
                        case_state = CaseState::Body;
                        // fast-highlight:491-493 — the word after `)` is back in
                        // command position; zsh's lexer does not set `incmdpos`
                        // there, so hand it over explicitly.
                        after_decoration = true;
                        self.color_span(t.start, t.end, HighlightRole::case_parentheses);
                        self.complex_brackets.push(t.start);
                    } else {
                        self.color_span(t.start, t.end, HighlightRole::keyword);
                    }
                }
                ENVSTRING => {
                    // fish:1016-1025 — visit_variable_assignment.
                    self.visit_variable_assignment(t);
                }
                ENVARRAY => {
                    // zsh `a=(…)`: the ENVARRAY span is `name=(`. fast-highlight:
                    // 701-710 paints the word `assign` and the `(` itself
                    // `assign-array-bracket`, and records the paren as complex so
                    // the brackets pass leaves it alone.
                    self.visit_variable_assignment(t);
                    self.mark_assign_parens(t, &mut in_array_assignment);
                }
                STRING_LEX if here_string_word => {
                    // fast-highlight:1115-1118 — the word after `<<<` is
                    // `here-string-text`, then `-fast-highlight-string` paints
                    // any `$…` inside it `here-string-var`.
                    here_string_word = false;
                    self.color_span(t.start, t.end, HighlightRole::here_string_text);
                    let n = self.color_array.len();
                    let (lo, hi) = (t.start.min(n), t.end.min(n));
                    let body: Vec<char> = self.buff_chars[lo..hi].to_vec();
                    let mut k = 0usize;
                    while k < body.len() {
                        if body[k] == '$' {
                            k += color_variable(
                                &body[k..],
                                &mut self.color_array[lo + k..hi],
                                HighlightRole::here_string_var,
                            );
                        } else {
                            k += 1;
                        }
                    }
                }
                STRING_LEX if case_state == CaseState::Subject => {
                    // fast-highlight:99 — the word after `case` is `case-input`.
                    case_state = CaseState::In;
                    self.color_span(t.start, t.end, HighlightRole::case_input);
                }
                STRING_LEX if case_state == CaseState::In => {
                    // The literal `in` of `case … in`; f-sy-h resolves it through
                    // `-fast-highlight-main-type`, which finds it in `$reswords`
                    // (fast-highlight:312-313) and paints `reserved-word`.
                    case_state = CaseState::Pattern;
                    self.color_span(t.start, t.end, HighlightRole::keyword);
                }
                STRING_LEX if case_state == CaseState::Pattern => {
                    // fast-highlight:101 — `case-condition`, a background style.
                    let n = self.color_array.len();
                    let (lo, hi) = (t.start.min(n), t.end.min(n));
                    self.color_array[lo..hi]
                        .fill(HighlightSpec::with_bg(HighlightRole::case_condition));
                }
                STRING_LEX if for_mode || name_word => {
                    // fast-highlight:659-664 — a `for` list element, a `for` /
                    // `function` NAME and the `in` of a for-list are never
                    // command words, but they DO take the ordinary non-command
                    // dispatch (`this_word & 14` at fast-highlight:793), so a
                    // glob in `for f in *.c` still paints as a glob.
                    name_word = false;
                    self.visit_argument(t, false, true, highlight_glob);
                }
                STRING_LEX if repeat_count => {
                    // fast-highlight:1191-1198 — the repeat-count word is
                    // consumed like a redirection target: no styling at all.
                    repeat_count = false;
                    soft_unknown = true;
                    after_decoration = true;
                }
                STRING_LEX if sudo_arg => {
                    // fast-highlight:559-560 — the argument of `sudo -u`, `-g`, …
                    sudo_arg = false;
                    sudo_opt = true;
                    after_decoration = true;
                }
                STRING_LEX if sudo_opt && t.clean_text().starts_with('-') => {
                    // fast-highlight:546-557
                    let clean = t.clean_text();
                    self.color_span(
                        t.start,
                        t.end,
                        if clean.starts_with("--") {
                            HighlightRole::double_hyphen_option
                        } else {
                            HighlightRole::option
                        },
                    );
                    if matches!(clean.as_str(), "-C" | "-g" | "-p" | "-r" | "-t" | "-u") {
                        sudo_arg = true;
                        sudo_opt = false;
                    }
                    after_decoration = true;
                }
                STRING_LEX
                    if afp.as_deref() == Some("command")
                        && fsh_afp_option(&t.clean_text(), "pvV-") =>
                {
                    // fast-highlight:562 — `command -p|-v|-V|--`
                    self.color_span(t.start, t.end, HighlightRole::option);
                    after_decoration = true;
                }
                STRING_LEX
                    if afp.as_deref() == Some("exec") && fsh_afp_option(&t.clean_text(), "cla-") =>
                {
                    // fast-highlight:563 — `exec -c|-l|-a|-`
                    self.color_span(t.start, t.end, HighlightRole::option);
                    after_decoration = true;
                }
                STRING_LEX if after_close_brace && t.clean_text() == "always" => {
                    // fast-highlight:612-615 — the `always` of a try-always
                    // block: "de facto a reserved word, although not de jure".
                    self.color_span(t.start, t.end, HighlightRole::keyword);
                    after_decoration = true;
                }
                STRING_LEX if after_close_brace && t.clean_text() == "{" => {
                    // fast-highlight:471 — after a delimiter (`}`, `))`, `]]`)
                    // a `{` is on command position again. zsh's lexer hands this
                    // one back as a plain STRING (it is past `always`, so its
                    // `incmdpos` is already false), so the brace has to be
                    // recognised here or the block's first word never gets
                    // validated.
                    after_close_brace = false;
                    self.color_span(t.start, t.end, HighlightRole::keyword);
                    expanded_cmd.clear();
                    after_decoration = true;
                }
                STRING_LEX => {
                    after_close_brace = false;
                    if in_cond {
                        // fast-highlight:813 — inside `[[ … ]]` every word takes
                        // the non-command dispatch.
                        self.visit_argument(t, false, true, highlight_glob);
                    } else if (t.cmdpos || after_decoration) && expanded_cmd.is_empty() {
                        // fish:1032-1073 — visit_decorated_statement (command word).
                        let clean = t.clean_text();
                        if clean == "noglob" {
                            highlight_glob = false; // fast-highlight:478
                        }
                        // fish:1033-1036 — color any decoration and keep looking
                        // for the real command word.
                        //
                        // fast-highlight:616-623 — a precommand modifier takes
                        // the `precommand` style and hands command position to
                        // the next word.  The set is the value-1 entries of
                        // `__FAST_HIGHLIGHT_TOKEN_TYPES` (fast-highlight:
                        // 125-130) plus `sudo`/`doas` (fast-highlight:620).
                        if toks.get(i + 1).map(|n| n.tok) == Some(INOUTPAR) {
                            // fast-highlight:865-868 — `name()` is a function
                            // DEFINITION: the `()` arm does `reply[-1]=()`,
                            // dropping the unknown-token style the name would
                            // otherwise have taken (the function does not exist
                            // yet, so validating it always fails).
                            after_decoration = false;
                        } else if fsh_is_precommand(&clean) {
                            decoration = match clean.as_str() {
                                "command" => StatementDecoration::Command,
                                "builtin" => StatementDecoration::Builtin,
                                "exec" => StatementDecoration::Exec,
                                _ => decoration,
                            };
                            if matches!(clean.as_str(), "sudo" | "doas") {
                                sudo_opt = true; // fast-highlight:620-623
                            }
                            if matches!(clean.as_str(), "command" | "exec") {
                                afp = Some(clean.clone()); // fast-highlight:617
                            }
                            self.color_span(t.start, t.end, HighlightRole::precommand);
                            after_decoration = true;
                        } else {
                            let soft = std::mem::take(&mut soft_unknown);
                            self.visit_command_word_soft(t, &clean, decoration, soft);
                            after_decoration = false;
                            expanded_cmd = clean;
                            is_cd = is_veritable_cd(&expanded_cmd);
                            eval_mode = expanded_cmd == "eval"; // fast-highlight:686
                            chroma = Chroma::for_command(&expanded_cmd);
                            chroma_words = 0;
                            chroma_skip = false;
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
                        if let Some(c) = chroma {
                            if self.visit_chroma_argument(t, c, &mut chroma_words, &mut chroma_skip) {
                                i += 1;
                                continue;
                            }
                            self.visit_argument(t, is_cd, !have_dashdash, highlight_glob);
                        } else if eval_mode && self.visit_eval_argument(t) {
                            // handled: the quoted body was re-highlighted
                        } else if is_typeset {
                            // fast-highlight:667 / :698 — `typeset` and friends
                            // push 'T' onto `braces_stack`, and while it is
                            // there EVERY following word is an `assign`
                            // (default `none`), options included: f-sy-h paints
                            // `typeset -g x` with a plain `-g`, not a cyan one.
                            self.color_span(t.start, t.end, HighlightRole::assign);
                            self.mark_assign_parens(t, &mut in_array_assignment);
                        } else {
                            self.visit_argument(t, is_cd, !have_dashdash, highlight_glob);
                        }
                        if self.span_text(t) == "--" {
                            have_dashdash = true; // fish:1091-1093
                        }
                    }
                }
                DINBRACK | DOUTBRACK => {
                    // fast-highlight:640 / :820 — `[[` and `]]` have their own
                    // style key, and both are recorded as "complex" brackets so
                    // the brackets pass does not repaint them
                    // (fast-highlight:653-654, :826-827).
                    self.color_span(t.start, t.end, HighlightRole::double_sq_bracket);
                    self.complex_brackets.push(t.start);
                    self.complex_brackets.push(t.start + 1);
                    in_cond = tokv == DINBRACK;
                }
                INOUTPAR => {
                    // fast-highlight:860-868 — `()`, an anonymous function or a
                    // function-definition marker: `reserved-word`, and recorded
                    // complex so the brackets pass does not claim it.
                    self.color_span(t.start, t.end, HighlightRole::keyword);
                    self.complex_brackets.push(t.start);
                    self.complex_brackets.push(t.start + 1);
                    expanded_cmd.clear();
                    after_decoration = true;
                }
                DINPAR | DOUTPAR => {
                    // fast-highlight:765-780 — `((` and `))` are `double-paren`,
                    // recorded complex so the brackets pass leaves them alone,
                    // and the expression between them goes through
                    // `-fast-highlight-math-string` (fast-highlight:771).
                    //
                    // zsh's lexer does NOT hand this back as one token: a
                    // `for ((i=0;i<3;i++))` header arrives as DINPAR `((`,
                    // DINPAR `i=0;`, DINPAR `i<3;`, DOUTPAR `i++))` — one token
                    // per `;`-separated clause, only the first carrying the
                    // opening `((` and only the last the closing `))`.  Both
                    // delimiters therefore have to be located in the source
                    // rather than assumed at the token edges.
                    let n = self.buff_chars.len();
                    let mut body_lo = t.start.min(n);
                    let mut body_end = t.end.min(n);
                    if self.buff_chars.get(body_lo) == Some(&'(')
                        && self.buff_chars.get(body_lo + 1) == Some(&'(')
                    {
                        self.color_span(body_lo, body_lo + 2, HighlightRole::double_paren);
                        self.complex_brackets.push(body_lo);
                        self.complex_brackets.push(body_lo + 1);
                        body_lo += 2;
                    }
                    {
                        let closer = if body_end >= body_lo + 2
                            && self.buff_chars[body_end - 2] == ')'
                            && self.buff_chars[body_end - 1] == ')'
                        {
                            body_end -= 2;
                            Some(body_end)
                        } else if self.buff_chars.get(body_end) == Some(&')')
                            && self.buff_chars.get(body_end + 1) == Some(&')')
                        {
                            Some(body_end)
                        } else {
                            None
                        };
                        if for_mode {
                            // fast-highlight:1010-1044 — a `((…))` right after
                            // `for` is a C-style loop header ('F' on
                            // `braces_stack`), styled with the for-loop keys
                            // rather than the math ones: identifiers plain,
                            // operator runs yellow, numbers magenta, the `;`
                            // separators yellow+bold.  The closing `))` is part
                            // of the trailing operator run there, so it is NOT
                            // repainted `double-paren`.
                            self.highlight_for_loop(body_lo, body_end);
                        } else {
                            self.highlight_math(body_lo, body_end);
                        }
                        if let Some(c) = closer {
                            self.color_span(c, c + 2, HighlightRole::double_paren);
                            self.complex_brackets.push(c);
                            self.complex_brackets.push(c + 1);
                        }
                    }
                }
                INBRACE_TOK | OUTBRACE_TOK => {
                    // fast-highlight:641-648 — `{` / `}` as a command word are
                    // `reserved-word`; they are NOT complex, so the brackets
                    // pass paints them by nesting level on top.
                    self.color_span(t.start, t.end, HighlightRole::keyword);
                    // fast-highlight:646-648 — `}` sets BIT_always (16) and `{`
                    // opens a fresh command position.
                    expanded_cmd.clear();
                    is_typeset = false;
                    after_decoration = tokv == INBRACE_TOK;
                    after_close_brace = tokv == OUTBRACE_TOK;
                }
                BANG_TOK => {
                    // fast-highlight:148 — `!` is a reserved word (control flow),
                    // and the word after it is still a command.
                    self.color_span(t.start, t.end, HighlightRole::keyword);
                    after_decoration = true;
                }
                tokv if (CASE..=TYPESET).contains(&tokv) => {
                    // fish:887-911 — visit_keyword: reserved words.
                    //
                    // fast-highlight:611-700 resolves a command-position word in
                    // ONE fixed order, and `reserved` is near the END of it
                    // (fast-highlight:295-322 `-fast-highlight-main-type`):
                    //   precommand → alias → global alias → function → builtin
                    //   → command → suffix alias → reserved
                    // So a zsh reserved word that ALSO resolves earlier takes
                    // the earlier style: `nocorrect` is a precommand
                    // (fast-highlight:128) and paints green, and `time` paints
                    // green wherever /usr/bin/time exists because `$+commands`
                    // is consulted before `$reswords`.  The whole typeset family
                    // lands on `builtin` the same way — `declare`, `export`,
                    // `float`, `integer`, `local`, `readonly` and `typeset` all
                    // lex to the single TYPESET token (hashtable.rs RESWDS) and
                    // all are real builtins, which is why f-sy-h greens them
                    // while `if`/`while` stay yellow.
                    let clean = t.clean_text();
                    let role = if fsh_is_precommand(&clean) {
                        after_decoration = true;
                        HighlightRole::precommand
                    } else {
                        self.classify_command_word(&clean)
                            .unwrap_or(HighlightRole::keyword)
                    };
                    self.color_span(t.start, t.end, role);
                    // fast-highlight:659-666 — `for` and `case` are followed by
                    // a name / subject, and zsh's `function` likewise.
                    match clean.as_str() {
                        "for" | "foreach" | "select" => for_mode = true,
                        "case" => case_state = CaseState::Subject,
                        "esac" => case_state = CaseState::None_,
                        "function" => name_word = true,
                        "repeat" => repeat_count = true, // fast-highlight:1188
                        _ => (),
                    }
                    if fsh_is_precommand(&clean) && matches!(clean.as_str(), "nocorrect" | "noglob")
                    {
                        // no extra state: these take no options
                    }
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

    /// fast-highlight:295-340 — `-fast-highlight-main-type`, respelled onto the
    /// zsh hash tables this shell already owns, in f-sy-h's EXACT lookup order:
    ///
    ///   alias → global alias → function → builtin → command → suffix alias →
    ///   reserved → dirpath
    ///
    /// Returns the style role for the word, or None when nothing claims it (the
    /// caller decides between `unknown-token` and `reserved-word`).
    ///
    /// fish asks a boolean `command_is_valid` and paints one `command` face; that
    /// cannot reproduce f-sy-h, which gives `alias`, `global alias`, `suffix
    /// alias` and `dirpath` each their own style key — a global alias is
    /// `bg=blue`, not green, and a bare `foo.txt` with a suffix alias defined is
    /// green, not an unknown token.
    fn classify_command_word(&self, word: &str) -> Option<HighlightRole> {
        if word.is_empty() {
            return None;
        }
        // fast-highlight:300-303 — aliases first, split by kind.
        if let Ok(tab) = aliastab_lock().read() {
            if let Some(node) = tab.get(word) {
                use crate::ported::zsh_h::ALIAS_GLOBAL;
                return Some(if node.node.flags & ALIAS_GLOBAL as i32 != 0 {
                    HighlightRole::global_alias
                } else if word.len() > 1 && word[1..].contains('=') {
                    // fast-highlight:670-673 — the "insane alias" (`a=b`).
                    HighlightRole::error
                } else {
                    HighlightRole::alias_
                });
            }
        }
        // fast-highlight:304-305 — functions.
        if getshfunc(word).is_some() {
            return Some(HighlightRole::function_);
        }
        // fast-highlight:306-307 — builtins.  `[` has its own style key
        // (fast-highlight:679-682).
        if crate::ported::builtin::createbuiltintable().contains_key(word)
            // zshrs's extension builtins (provenance, dbview, zcache, …) are
            // NOT in createbuiltintable — they live in EXT_BUILTIN_NAMES and
            // dispatch through ext_builtins, so asking only the core table
            // painted every one of them as an unknown token even though
            // `whence -w provenance` says `builtin`.
            //
            // NOTE: `builtin_in_builtintab` alone is NOT a membership test —
            // `builtin_owning_module` returns None for an unknown name and its
            // `None => true` arm then reports every string as available.
            // Membership in EXT_BUILTIN_NAMES has to come first; the
            // availability call adds the `disable`/module and
            // ZSHRS_HIDE_EXT_BUILTINS gates on top.
            || (crate::ext_builtins::EXT_BUILTIN_NAMES.contains(&word)
                && crate::ext_builtins::builtin_in_builtintab(word))
        {
            return Some(if word == "[" {
                HighlightRole::single_sq_bracket
            } else {
                HighlightRole::builtin_
            });
        }
        // fast-highlight:308-309 — external commands (`$+commands`, i.e. the
        // hashed table plus a $PATH walk).
        if cmdnamtab_lock()
            .read()
            .map(|t| t.get(word).is_some())
            .unwrap_or(false)
            || findcmd(word, 0, 0).is_some()
        {
            return Some(HighlightRole::command);
        }
        // fast-highlight:310-311 — suffix aliases, keyed by the extension.
        if let Some((_, ext)) = word.rsplit_once('.') {
            if !ext.is_empty()
                && sufaliastab_lock()
                    .read()
                    .map(|t| t.get(ext).is_some())
                    .unwrap_or(false)
            {
                return Some(HighlightRole::suffix_alias);
            }
        }
        // fast-highlight:312-313 — reserved words.
        if reswdtab_lock()
            .read()
            .map(|t| t.get(word).is_some())
            .unwrap_or(false)
        {
            return Some(HighlightRole::keyword);
        }
        // fast-highlight:327-335 — a directory is `dirpath`, styled
        // `path-to-dir` (fast-highlight:694).
        let dir = crate::zle_file_tester::path_apply_working_directory(word, &self.working_directory);
        if std::fs::metadata(&dir).map(|m| m.is_dir()).unwrap_or(false) {
            return Some(HighlightRole::path_to_dir);
        }
        None
    }

    /// fast-highlight:795-796 — a command word reached with `this_word & 14`
    /// (the `repeat`-count aftermath) styles an UNRESOLVED word `default`, not
    /// `unknown-token`: `repeat 2 zzqwx` leaves `zzqwx` plain while
    /// `repeat 2 print` still greens `print`.
    fn visit_command_word_soft(
        &mut self,
        t: &TokSpan,
        clean: &str,
        decoration: StatementDecoration,
        soft: bool,
    ) {
        if !soft {
            return self.visit_command_word(t, clean, decoration);
        }
        if let Some(role) = self.classify_command_word(clean) {
            if role != HighlightRole::error {
                self.color_span(t.start, t.end, role);
            }
        }
    }

    // fish:801-811 + 1038-1073 — Color a command word after validity checking.
    fn visit_command_word(&mut self, t: &TokSpan, clean: &str, _decoration: StatementDecoration) {
        // Reserved words arrive as their own token types; aliases as STRING that
        // checkalias() already expanded. What reaches here is a plain command word.
        if !self.io_still_ok() {
            // fish:1047-1049 — We cannot check if the command is invalid, so just
            // assume it's valid.
            self.color_span(t.start, t.end, HighlightRole::command);
            return;
        }
        // fish:1052-1065 — Check to see if the command is valid.
        // Try expanding it. If we cannot, it's an error.
        let mut expanded = t.text.clone().unwrap_or_default();
        let expanded_ok = expand_one_no_cmdsubst(&mut expanded);
        let cmd = if expanded_ok && !expanded.is_empty() {
            expanded
        } else {
            clean.to_owned()
        };
        // fast-highlight:743-747 — a history expansion in command position
        // (`sudo !!`) is `history-expansion`, checked before the type lookup.
        let histchar = crate::ported::hist::bangchar.load(std::sync::atomic::Ordering::SeqCst);
        if histchar != 0 {
            let mut it = clean.chars();
            if it.next() == char::from_u32(histchar as u32) && it.next().is_some() {
                self.color_span(t.start, t.end, HighlightRole::history_expansion);
                return;
            }
        }

        // fast-highlight:638-696 — the resolved KIND picks the style; f-sy-h has
        // no single "is it valid" question.  A word still carrying expansion
        // markers can't be resolved, so it is assumed good (fish:1063).
        let role = if has_expand_reserved(&cmd) {
            Some(HighlightRole::command)
        } else {
            self.classify_command_word(&cmd)
        };

        // fish:1068-1073 — Color our statement.
        match role {
            Some(role) => {
                let start = t.start;
                let end = t.end.min(self.color_array.len());
                let src: String = self.span_text(t);
                color_string_internal(
                    &src,
                    HighlightSpec::with_fg(role),
                    &mut self.color_array[start..end],
                );
            }
            None => self.color_span(t.start, t.end, HighlightRole::error),
        }
        if role == Some(HighlightRole::single_sq_bracket) {
            // fast-highlight:681 — `[` is recorded complex so the brackets pass
            // leaves it alone.
            self.complex_brackets.push(t.start);
        }
    }

    // fish:812-871 + 932-960 — Visit an argument, perhaps knowing that our command
    // is cd.
    //
    // The dispatch order is f-sy-h's non-command-word `case` arm
    // (fast-highlight:869-1090), which resolves the WHOLE word to one style
    // rather than painting glob/brace/paren characters individually the way
    // fish does:
    //   `--`/`--opt` → double-hyphen-option   (fast-highlight:869-878)
    //   `-opt`       → single-hyphen-option   (fast-highlight:881)
    //   quoted       → the quote styles       (fast-highlight:884-960)
    //   `*`/`?` word → globbing               (fast-highlight:1047-1048)
    //   `$…`         → variable               (fast-highlight:1049-1050)
    //   `!…`         → history-expansion      (fast-highlight:1062-1063)
    //   global alias → global-alias           (fast-highlight:1068-1069)
    //   existing path→ path / path-to-dir     (fast-highlight:1279-1280)
    fn visit_argument(
        &mut self,
        t: &TokSpan,
        cmd_is_cd: bool,
        options_allowed: bool,
        highlight_glob: bool,
    ) {
        let start = t.start;
        let end = t.end.min(self.color_array.len());
        if start >= end {
            return;
        }
        let src: String = self.span_text(t);
        let clean = t.clean_text();
        // The TOKENIZED text: the lexer has already turned every live glob
        // character into its token (Star, Quest, …) and left quoted ones as
        // plain characters, which is exactly the question
        // `[[ $__arg = ([*?]*|*[^\\][*?]*) ]]` (fast-highlight:1047) asks.  The
        // literal span cannot answer it — `ls '*'` looks identical to `ls *`.
        let tokenized = t.text.clone().unwrap_or_default();

        // fast-highlight:729-737 / :955-965 — `$((…))`: the `$` takes the
        // in-string dollar style, the `((` and `))` are `double-paren` (and
        // complex, so the brackets pass skips them), and the expression between
        // them goes through the math highlighter.
        if clean.starts_with("$((") {
            self.color_span(start, start + 1, HighlightRole::dollar_in_dquote);
            self.color_span(start + 1, start + 3, HighlightRole::double_paren);
            self.complex_brackets.push(start + 1);
            self.complex_brackets.push(start + 2);
            let mut body_end = end;
            if clean.ends_with("))") && end >= start + 5 {
                body_end = end - 2;
                self.color_span(body_end, end, HighlightRole::double_paren);
                self.complex_brackets.push(body_end);
                self.complex_brackets.push(body_end + 1);
            }
            self.highlight_math(start + 3, body_end);
            return;
        }

        // fast-highlight:869-881 — options, gated by a preceding bare `--`.
        let base = if options_allowed && clean.starts_with("--") {
            HighlightRole::double_hyphen_option
        } else if options_allowed && clean.starts_with('-') {
            // fast-highlight:881 — the pattern is `'-'*`, so a bare `-` counts.
            HighlightRole::option
        } else if fsh_has_quote(&tokenized) {
            // fast-highlight:884 — the `[\"\']*|…` arm: once a word contains an
            // unescaped quote, f-sy-h hands the WHOLE word to
            // `-fast-highlight-string` and never applies a word-level style, so
            // the unquoted remainder stays `default`.  Painting `variable` under
            // it left the `$`, `f` and `}` of `${(f)"$(ls)"}` at 113.
            HighlightRole::param
        } else if clean == "]" {
            // fast-highlight:833-836 — the `]` of a `[ … ]` test.
            self.complex_brackets.push(t.start);
            HighlightRole::single_sq_bracket
        } else if let Some(role) = self.fsh_plain_word_role(&clean, &tokenized, highlight_glob) {
            role
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
            // fish:846-851 paints the `$(` and `)` with its operator face.
            // f-sy-h leaves them to the brackets pass (fast-string-highlight:60),
            // which gives them `bracket-level-N`, while the leading `$` keeps the
            // word's own `variable` style (fast-highlight:1049-1050).
            let inner_start = open + 2;
            let inner_end = close.unwrap_or(src_chars.len());
            // fast-highlight:803 (`$__arg = \$\([^\(]*`) and the recursion in
            // `-fast-highlight-string`: only a substitution with NO nested paren
            // is re-highlighted.  `print $(print $(ls))` leaves the outer body
            // on the word's own `variable` style and recurses into `$(ls)` only.
            let nested = src_chars[inner_start..inner_end].contains(&'(');
            if nested {
                scan = inner_start;
                continue;
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
                let mut subcolors = cmdsub_highlighter.highlight();
                // fast-highlight:890-899 — the recursion happens under the
                // SECONDARY theme.
                for c in subcolors.iter_mut() {
                    c.secondary = true;
                }
                let dst_lo = (start + inner_start).min(self.color_array.len());
                let dst_hi = (start + inner_end).min(self.color_array.len());
                // Overlay only the cells the sub-pass actually styled.  f-sy-h
                // appends region entries on top of the word's own style, so the
                // gaps between the nested command's tokens keep the outer
                // `variable` colour (`ls $(print /tmp)` — the space between
                // `print` and `/tmp` is 113, not default).
                for (k, sub) in subcolors.iter().take(dst_hi - dst_lo).enumerate() {
                    if sub.foreground != HighlightRole::normal
                        || sub.background != HighlightRole::normal
                    {
                        self.color_array[dst_lo + k] = *sub;
                    }
                }
            }
            scan = inner_end + 1;
            if scan >= src_chars.len() {
                break;
            }
        }

        // fast-highlight:1244-1258 — a backquoted substitution is re-highlighted
        // the same way (its ticks keep the `back-quoted-argument` style, which is
        // `none`).  fish has no backtick syntax at all, so nothing here recursed
        // and `print \`ls\`` left `ls` uncoloured.
        self.highlight_backticks(&src_chars, start);

        let _ = cmd_is_cd; // fish:939-959 cd-path validation — see below.
    }

    /// fast-highlight:171-232 — one of f-sy-h's per-command argument
    /// highlighters ("chromas"), for the two commands a zshrc is mostly made of.
    /// Returns true when the chroma styled the word itself.
    fn visit_chroma_argument(
        &mut self,
        t: &TokSpan,
        c: Chroma,
        words: &mut usize,
        skip: &mut bool,
    ) -> bool {
        let clean = t.clean_text();
        if c == Chroma::Printf {
            // -printf.ch:44-53 — options are NOT styled by the chroma (they
            // pass through), and `-v` swallows the variable name after it.
            if clean.starts_with('-') {
                *skip = clean == "-v";
                return false;
            }
            if *skip {
                *skip = false;
                return false;
            }
        }
        // -autoload.ch:52-55 / -source.ch:41-45 — options keep their own style.
        let opt_lead = match c {
            Chroma::Autoload => clean.starts_with('-') || clean.starts_with('+'),
            Chroma::Source => clean.starts_with('-'),
            Chroma::Printf => false,
        };
        if opt_lead {
            self.color_span(
                t.start,
                t.end,
                if clean.starts_with("--") {
                    HighlightRole::double_hyphen_option
                } else {
                    HighlightRole::option
                },
            );
            return true;
        }
        *words += 1;
        match c {
            Chroma::Autoload => {
                // -autoload.ch:62-72 — a name found under $fpath is
                // `correct-subtle`, one that is not is `incorrect-subtle`.
                // Words that begin with `$`, `/` or a backtick are skipped.
                if clean.starts_with('$') || clean.starts_with('/') || clean.starts_with('`') {
                    return false;
                }
                let found = crate::ported::params::getaparam("fpath")
                    .unwrap_or_default()
                    .iter()
                    .any(|dir| std::path::Path::new(&format!("{dir}/{clean}")).exists());
                self.color_span(
                    t.start,
                    t.end,
                    if found {
                        HighlightRole::correct_subtle
                    } else {
                        HighlightRole::incorrect_subtle
                    },
                );
                true
            }
            Chroma::Source => {
                // -source.ch:50-58 — only the FIRST non-option word is checked;
                // the second and later pass through to the ordinary dispatch.
                //
                // f-sy-h decides `correct-subtle` vs `incorrect-subtle` by
                // COPYING the file into $FAST_WORK_DIR and running `zcompile`
                // on it — on every keystroke.  That side effect has no place on
                // zshrs's ZLE path, so the file's mere existence stands in for
                // "compiles": a readable file is `correct-subtle` and a missing
                // one falls through (which is also what f-sy-h does when the
                // copy fails, -source.ch:54).  The one case that differs is an
                // existing file with a syntax error, which f-sy-h reddens.
                if *words != 1 {
                    return false;
                }
                let Some(path) = fsh_expand_for_path(&t.text.clone().unwrap_or_default()) else {
                    return false;
                };
                let abs = crate::zle_file_tester::path_apply_working_directory(
                    &path,
                    &self.working_directory,
                );
                if std::fs::metadata(&abs).map(|m| m.is_file()).unwrap_or(false) {
                    self.color_span(t.start, t.end, HighlightRole::correct_subtle);
                    return true;
                }
                false
            }
            Chroma::Printf => {
                // -printf.ch:54-70 — only the FIRST non-option word is the
                // format: it takes its quote style, and every `%`-conversion
                // inside it is painted `mathnum`.
                if *words != 1 {
                    return false;
                }
                let n = self.buff_chars.len();
                let (lo, hi) = (t.start.min(n), t.end.min(n));
                let q = self.buff_chars.get(lo).copied();
                if q == Some('"') {
                    self.color_span(lo, hi, HighlightRole::dquoted);
                } else if q == Some('\'') {
                    self.color_span(lo, hi, HighlightRole::quote);
                }
                for (a, b) in printf_conversions(&self.buff_chars[lo..hi]) {
                    self.color_span(lo + a, lo + b, HighlightRole::mathnum);
                }
                true
            }
        }
    }

    /// fast-highlight:884-899 — `eval 'print hi'`: the quoted argument's BODY is
    /// re-highlighted as a command line under the secondary theme, and the
    /// quotes themselves keep the `recursive-base` style (unset in the default
    /// theme, hence plain).  Returns false when the argument is not quoted, so
    /// the caller falls back to the ordinary dispatch.
    fn visit_eval_argument(&mut self, t: &TokSpan) -> bool {
        let n = self.buff_chars.len();
        let (lo, hi) = (t.start.min(n), t.end.min(n));
        if hi <= lo + 1 {
            return false;
        }
        let q = self.buff_chars[lo];
        if (q != '\'' && q != '"') || self.buff_chars[hi - 1] != q {
            return false;
        }
        let inner: String = self.buff_chars[lo + 1..hi - 1].iter().collect();
        if inner.is_empty() {
            return true;
        }
        let mut sub = Highlighter::new(
            &inner,
            None,
            self.ctx,
            self.working_directory.clone(),
            self.io_still_ok(),
        );
        let mut subcolors = sub.highlight();
        for c in subcolors.iter_mut() {
            c.secondary = true;
        }
        for (k, s) in subcolors.iter().enumerate() {
            if s.foreground != HighlightRole::normal || s.background != HighlightRole::normal {
                self.color_array[lo + 1 + k] = *s;
            }
        }
        true
    }

    /// Re-highlight the body of every ``…`` span in an argument, under the
    /// secondary theme (fast-highlight:1244-1258).
    fn highlight_backticks(&mut self, src_chars: &[char], start: usize) {
        let mut i = 0usize;
        let mut in_squote = false;
        while i < src_chars.len() {
            match src_chars[i] {
                '\\' => i += 1,
                '\'' => in_squote = !in_squote,
                '`' if !in_squote => {
                    let body_lo = i + 1;
                    let mut j = body_lo;
                    while j < src_chars.len() && src_chars[j] != '`' {
                        if src_chars[j] == '\\' {
                            j += 1;
                        }
                        j += 1;
                    }
                    let body_hi = j.min(src_chars.len());
                    if body_hi > body_lo {
                        let inner: String = src_chars[body_lo..body_hi].iter().collect();
                        let mut sub = Highlighter::new(
                            &inner,
                            None,
                            self.ctx,
                            self.working_directory.clone(),
                            self.io_still_ok(),
                        );
                        let mut subcolors = sub.highlight();
                        for c in subcolors.iter_mut() {
                            c.secondary = true;
                        }
                        let dst_lo = (start + body_lo).min(self.color_array.len());
                        let dst_hi = (start + body_hi).min(self.color_array.len());
                        for (k, sub) in subcolors.iter().take(dst_hi - dst_lo).enumerate() {
                            if sub.foreground != HighlightRole::normal
                                || sub.background != HighlightRole::normal
                            {
                                self.color_array[dst_lo + k] = *sub;
                            }
                        }
                    }
                    i = body_hi;
                }
                _ => (),
            }
            i += 1;
        }
    }

    /// fast-highlight:1040-1090 — the tail of the non-command-word dispatch:
    /// glob, variable, history expansion, global alias, path.  Returns None when
    /// nothing claims the word (f-sy-h leaves it `default`).
    ///
    /// `src` is the still-tokenized span (so a glob char is a lexer token, not a
    /// literal `*`); `clean` is what the user typed.
    fn fsh_plain_word_role(
        &self,
        clean: &str,
        src: &str,
        highlight_glob: bool,
    ) -> Option<HighlightRole> {
        if clean.is_empty() {
            return None;
        }
        // fast-highlight:1045-1046 — an extended-glob word (`(#b)`, `(#B)`,
        // `(#m)`, `(#c…)` or a bare `##`) is `globbing-ext`, checked BEFORE the
        // plain glob test.
        if fsh_is_globbing_ext(clean) {
            return Some(if highlight_glob {
                HighlightRole::globbing_ext
            } else {
                HighlightRole::param
            });
        }
        // fast-highlight:1047-1048 — `[[ $__arg = ([*?]*|*[^\\][*?]*) ]]`: a word
        // carrying an UNQUOTED `*` or `?` is a glob, and the whole word takes the
        // glob style.  The zsh lexer already answered "was it quoted" for us by
        // turning the live ones into Star/Quest tokens.
        {
            use crate::ported::zsh_h::{Quest, Star};
            if src.chars().any(|c| c == Star || c == Quest) {
                return Some(if highlight_glob {
                    HighlightRole::globbing
                } else {
                    HighlightRole::param
                });
            }
        }
        // fast-highlight:1049-1050 — a word STARTING with `$` is a variable.
        if clean.starts_with('$') {
            return Some(HighlightRole::variable);
        }
        // fast-highlight:1062-1063 — history expansion (`!!`, `!ls`, `!$`).
        let histchar =
            crate::ported::hist::bangchar.load(std::sync::atomic::Ordering::SeqCst);
        if histchar != 0 {
            let mut it = clean.chars();
            if it.next() == char::from_u32(histchar as u32) && it.next().is_some() {
                return Some(HighlightRole::history_expansion);
            }
        }
        // fast-highlight:1068-1069 — a global alias used as an argument.
        if let Ok(tab) = aliastab_lock().read() {
            if let Some(node) = tab.get(clean) {
                use crate::ported::zsh_h::ALIAS_GLOBAL;
                if node.node.flags & ALIAS_GLOBAL as i32 != 0 {
                    return Some(HighlightRole::global_alias);
                }
            }
        }
        // fast-highlight:1279-1280 (`-fast-highlight-check-path`) — a directory
        // is `path-to-dir`, anything else that EXISTS is `path`.  Note f-sy-h
        // tests `-d`/`-e` on the expanded word and nothing else: it has no
        // notion of fish's "is a prefix of a valid path", which is why fish
        // underlined a bare `c` in `arr=(a b c)` whenever a `c*` file existed.
        if !self.io_still_ok() {
            return None; // fish:935-937
        }
        let expanded = fsh_expand_for_path(src)?;
        let abs =
            crate::zle_file_tester::path_apply_working_directory(&expanded, &self.working_directory);
        match std::fs::metadata(&abs) {
            Ok(md) if md.is_dir() => Some(HighlightRole::path_to_dir),
            Ok(_) => Some(HighlightRole::path),
            Err(_) => None,
        }
    }

    // fish:962-1014 — visit_redirection: operator token + optional target token.
    fn visit_redirection(&mut self, op: &TokSpan, target: Option<&TokSpan>) {
        // fish:968-980 — Color the operator part like 2>.
        self.color_span(op.start, op.end, HighlightRole::redirection);

        let Some(target) = target else { return };
        let target_text = target.text.clone().unwrap_or_default();


        // fish:982-1013 validates the target and paints an unwritable / missing
        // one `unknown-token`.  f-sy-h has no such check: a redirection target
        // runs through the ordinary word dispatch (fast-highlight:1040-1090), so
        // `cat < /etc/hosts` is a magenta path and `ls > /nope` is plain.
        let _ = (&target_text, redir_tok_mode(op.tok, &target.clean_text()));
        self.visit_argument(target, false, /*options_allowed=*/ true, true);
    }

    // fish:1016-1025 — visit_variable_assignment.
    fn visit_variable_assignment(&mut self, t: &TokSpan) {
        let start = t.start;
        let end = t.end.min(self.color_array.len());
        if start >= end {
            return;
        }
        let src: String = self.span_text(t);
        // fast-highlight:698-699 — the whole assignment word takes the `assign`
        // style, whose default is `none` (fast-highlight:84).  fish:1018-1024
        // paints the `=` with its operator face, which this engine renders as
        // the glob colour, so every `FOO=bar` grew a blue `=`.
        color_string_internal(
            &src,
            HighlightSpec::with_fg(HighlightRole::assign),
            &mut self.color_array[start..end],
        );
        // fast-highlight:729-737 — `NAME=$((…))`: the arithmetic part of an
        // assignment gets the same `$` / `((` / math / `))` treatment as a bare
        // `$((…))` argument.
        if let Some(eq) = src.chars().position(|c| c == '=') {
            let rest: String = src.chars().skip(eq + 1).collect();
            if rest.starts_with("$((") {
                let lo = start + eq + 1;
                self.color_span(lo, lo + 1, HighlightRole::dollar_in_dquote);
                self.color_span(lo + 1, lo + 3, HighlightRole::double_paren);
                self.complex_brackets.push(lo + 1);
                self.complex_brackets.push(lo + 2);
                let mut body_end = end;
                if rest.ends_with("))") && end >= lo + 5 {
                    body_end = end - 2;
                    self.color_span(body_end, end, HighlightRole::double_paren);
                    self.complex_brackets.push(body_end);
                    self.complex_brackets.push(body_end + 1);
                }
                self.highlight_math(lo + 3, body_end);
            }
        }

        // fish:1022-1024 — remember the variable for redirection validation.
        if let Some(offset) = src.chars().position(|c| c == '=') {
            let var_name: String = src.chars().take(offset).collect();
            if valid_var_name(&var_name) {
                self.pending_variables.push(var_name);
            }
        }
    }

    /// Port of the `braces_stack = F*` arm of fast-highlight:1012-1044 — the
    /// C-style `for ((init; cond; step))` header.  Four independent scans, in
    /// f-sy-h's order, each overwriting the previous:
    ///   identifiers → `for-loop-variable`, operator runs → `for-loop-operator`,
    ///   digit runs → `for-loop-number`, and a trailing `;` →
    ///   `for-loop-separator`.
    fn highlight_for_loop(&mut self, lo: usize, hi: usize) {
        let hi = hi.min(self.buff_chars.len());
        if lo >= hi {
            return;
        }
        const OPS: &str = "+<>=:*|&^~-";
        let mut i = lo;
        while i < hi {
            let c = self.buff_chars[i];
            if c.is_alphabetic() || c == '_' {
                let mut j = i;
                while j < hi && (self.buff_chars[j].is_alphanumeric() || self.buff_chars[j] == '_') {
                    j += 1;
                }
                self.color_span(i, j, HighlightRole::for_loop_variable);
                i = j;
            } else if c.is_ascii_digit() {
                let mut j = i;
                while j < hi && self.buff_chars[j].is_ascii_digit() {
                    j += 1;
                }
                self.color_span(i, j, HighlightRole::for_loop_number);
                i = j;
            } else if OPS.contains(c) {
                let mut j = i;
                while j < hi && OPS.contains(self.buff_chars[j]) {
                    j += 1;
                }
                self.color_span(i, j, HighlightRole::for_loop_operator);
                i = j;
            } else if c == ';' {
                self.color_span(i, i + 1, HighlightRole::for_loop_separator);
                i += 1;
            } else {
                i += 1;
            }
        }
    }

    /// fast-highlight:701-710 — inside an assignment word, the `(` opening an
    /// array value (and the `)` closing it, when both land in the same token)
    /// are `assign-array-bracket` and are recorded complex so the brackets pass
    /// leaves them at that flat green instead of the bold bracket-level colour.
    fn mark_assign_parens(&mut self, t: &TokSpan, in_array_assignment: &mut bool) {
        let n = self.buff_chars.len();
        let (lo, hi) = (t.start.min(n), t.end.min(n));
        let Some(eq) = self.buff_chars[lo..hi].iter().position(|&c| c == '=') else {
            return;
        };
        let Some(open) = self.buff_chars[lo + eq..hi].iter().position(|&c| c == '(') else {
            return;
        };
        let open = lo + eq + open;
        if open != lo + eq + 1 {
            return; // not `NAME=(`
        }
        self.color_span(open, open + 1, HighlightRole::assign_array_bracket);
        self.complex_brackets.push(open);
        match self.buff_chars[open..hi].iter().rposition(|&c| c == ')') {
            Some(rel) => {
                let close = open + rel;
                self.color_span(close, close + 1, HighlightRole::assign_array_bracket);
                self.complex_brackets.push(close);
            }
            None => *in_array_assignment = true,
        }
    }

    /// Port of `-fast-highlight-math-string` (fast-highlight:1160-1190).
    ///
    /// It walks a math expression and styles only three kinds of token, leaving
    /// operators and whitespace alone:
    ///   * a run of digits           → `mathnum`
    ///   * a bare identifier         → `mathvar` if the parameter exists,
    ///                                 `matherr` if it does not
    ///   * a `$name` / `${name}`     → `back-or-dollar-double-quoted-argument`
    ///                                 if set, `matherr` if not
    ///
    /// fish has no math highlighting at all; it colours a `$((…))` body with the
    /// generic error face the moment the arithmetic does not parse, which is why
    /// `print $((1+2))` came out entirely red.
    fn highlight_math(&mut self, lo: usize, hi: usize) {
        let hi = hi.min(self.buff_chars.len());
        if lo >= hi {
            return;
        }
        let mut i = lo;
        while i < hi {
            let c = self.buff_chars[i];
            if c.is_ascii_digit() {
                let mut j = i;
                while j < hi && (self.buff_chars[j].is_ascii_digit() || self.buff_chars[j] == '.') {
                    j += 1;
                }
                self.color_span(i, j, HighlightRole::mathnum); // fast-highlight:1166
                i = j;
            } else if c.is_alphabetic() || c == '_' {
                let mut j = i;
                while j < hi && (self.buff_chars[j].is_alphanumeric() || self.buff_chars[j] == '_') {
                    j += 1;
                }
                let name: String = self.buff_chars[i..j].iter().collect();
                let role = if math_name_exists(&name) || self.pending_variables.contains(&name) {
                    HighlightRole::mathvar // fast-highlight:1168
                } else {
                    HighlightRole::matherr // fast-highlight:1169
                };
                self.color_span(i, j, role);
                i = j;
            } else if c == '$' {
                let mut j = i + 1;
                let braced = self.buff_chars.get(j) == Some(&'{');
                if braced {
                    j += 1;
                }
                let name_lo = j;
                while j < hi && (self.buff_chars[j].is_alphanumeric() || self.buff_chars[j] == '_') {
                    j += 1;
                }
                let name: String = self.buff_chars[name_lo..j].iter().collect();
                if braced && self.buff_chars.get(j) == Some(&'}') {
                    j += 1;
                }
                let role = if !name.is_empty()
                    && (math_name_exists(&name) || self.pending_variables.contains(&name))
                {
                    HighlightRole::dollar_in_dquote // fast-highlight:1175
                } else {
                    HighlightRole::matherr // fast-highlight:1177
                };
                self.color_span(i, j, role);
                i = j;
            } else {
                i += 1;
            }
        }
    }

    /// Port of `-fast-highlight-string-process` (fast-string-highlight:1-70),
    /// the brackets highlighter that `FAST_HIGHLIGHT[use_brackets]` turns on by
    /// default (fast-highlight:168).
    ///
    /// It walks the buffer tracking `'`, `"` and `$'` quoting plus backslash
    /// escapes, pushes `(`/`{`/`[` onto a level stack, pairs each closer with
    /// the top of the stack, then paints every bracket position: a matched pair
    /// gets `bracket-level-N` where N is `((level-1) %% 3) + 1`
    /// (fast-string-highlight:60), and an unmatched bracket gets
    /// `unknown-token` (fast-string-highlight:60, the `||` branch).
    fn highlight_brackets(&mut self) {
        let chars = &self.buff_chars;
        let n = chars.len();
        // fast-string-highlight:11 — pos → level, and the matched-pair set.
        let mut pos_level: Vec<(usize, i32)> = Vec::new();
        let mut stack: Vec<usize> = Vec::new();
        let mut paired: Vec<bool> = Vec::new();
        // Quoting state: fast-string-highlight:17-56.
        #[derive(PartialEq)]
        enum Q {
            None_,
            Single,
            Double,
            DollarSingle,
        }
        let mut quoting = Q::None_;
        let mut i = 0usize;
        while i < n {
            let c = chars[i];
            if c == '\\' {
                // fast-string-highlight:18-21 — a backslash escapes the next
                // char, except inside `'…'`.
                if quoting == Q::Single {
                    i += 1;
                } else {
                    i += 2;
                }
                continue;
            }
            match c {
                '"' if quoting != Q::Single && quoting != Q::DollarSingle => {
                    quoting = if quoting == Q::Double { Q::None_ } else { Q::Double };
                }
                '\'' if quoting != Q::Double => {
                    quoting = match quoting {
                        Q::Single | Q::DollarSingle => Q::None_,
                        // fast-string-highlight:50-54 — `$'` opens an ANSI-C
                        // quote, which keeps its own escape rules.
                        _ if i > 0 && chars[i - 1] == '$' => Q::DollarSingle,
                        _ => Q::Single,
                    };
                }
                '(' | '{' | '[' if quoting == Q::None_ && !self.complex_brackets.contains(&i) => {
                    stack.push(i);
                    pos_level.push((i, stack.len() as i32));
                    paired.push(false);
                }
                ')' | '}' | ']' if quoting == Q::None_ && !self.complex_brackets.contains(&i) => {
                    if let Some(open) = stack.pop() {
                        pos_level.push((i, stack.len() as i32 + 1));
                        paired.push(false);
                        let opener = chars[open];
                        // fast-string-highlight:29 — `pair_map` match.
                        let matches = matches!((opener, c), ('(', ')') | ('{', '}') | ('[', ']'));
                        if matches {
                            let last = paired.len() - 1;
                            paired[last] = true;
                            if let Some(k) = pos_level.iter().position(|&(p, _)| p == open) {
                                paired[k] = true;
                            }
                        }
                    } else {
                        // fast-string-highlight:35 — level -1: unmatched closer.
                        pos_level.push((i, -1));
                        paired.push(false);
                    }
                }
                _ => (),
            }
            i += 1;
        }
        for (k, &(pos, level)) in pos_level.iter().enumerate() {
            let role = if paired[k] {
                match (level - 1).rem_euclid(3) + 1 {
                    1 => HighlightRole::bracket_level_1,
                    2 => HighlightRole::bracket_level_2,
                    _ => HighlightRole::bracket_level_3,
                }
            } else {
                HighlightRole::error
            };
            self.color_span(pos, pos + 1, role);
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


    // ======================================================================
    // fast-syntax-highlighting parity.
    //
    // Every test below pins a case where fish's palette or structure
    // DISAGREED with f-sy-h and the difference was measured over a pty
    // (both shells driven side by side, compared on rendered cell
    // attributes).  The f-sy-h line that decides each case is cited at the
    // assertion.
    // ======================================================================

    fn roles(line: &str) -> Vec<HighlightRole> {
        let ctx = OperationContext::empty();
        let mut colors = Vec::new();
        highlight_shell(line, &mut colors, &ctx, /*io_ok=*/ true, None);
        colors.iter().map(|c| c.foreground).collect()
    }

    fn role_of(line: &str, word: &str) -> HighlightRole {
        let at = line.find(word).expect("word not in line");
        let at = line[..at].chars().count();
        roles(line)[at]
    }

    /// fast-highlight:121-130 + :620 — the precommand set. The NEGATIVE half is
    /// the point: a prefix or a superstring of a precommand is an ordinary word,
    /// and treating it as one would hand command position to the wrong token.
    #[test]
    fn precommand_table_is_exact_membership() {
        for yes in [
            "builtin", "command", "exec", "nocorrect", "noglob", "pkexec", "sudo", "doas",
        ] {
            assert!(fsh_is_precommand(yes), "{yes} is a precommand");
        }
        for no in ["", "sud", "commander", "execute", "noglobber", "Sudo", "-"] {
            assert!(!fsh_is_precommand(no), "{no} is NOT a precommand");
        }
    }

    /// fast-highlight:562-563 — `command -pvV--` / `exec -cla-` keep command
    /// position; anything else after them does not.
    #[test]
    fn afp_option_predicate_both_directions() {
        assert!(fsh_afp_option("-v", "pvV-"));
        assert!(fsh_afp_option("-pv", "pvV-"));
        assert!(fsh_afp_option("--", "pvV-"));
        assert!(fsh_afp_option("-a", "cla-"));
        assert!(!fsh_afp_option("-x", "pvV-"), "-x is not a `command` option");
        assert!(!fsh_afp_option("-", "pvV-"), "a bare - has no option letters");
        assert!(!fsh_afp_option("v", "pvV-"), "no leading hyphen");
        assert!(!fsh_afp_option("", "pvV-"));
    }

    /// fast-highlight:1045 — `globbing-ext` detection. The escaped `\##` case is
    /// the negative half: f-sy-h's pattern is `[^\\]##`, so a backslash in front
    /// keeps it an ordinary word.
    #[test]
    fn globbing_ext_predicate_both_directions() {
        for yes in ["(#b)a*", "(#B)x", "(#m)y", "(#c1,3)z", "a##b"] {
            assert!(fsh_is_globbing_ext(yes), "{yes} carries an extended glob");
        }
        for no in ["", "a#b", "plain", "*.c", "a\\##b"] {
            assert!(!fsh_is_globbing_ext(no), "{no} has no extended glob");
        }
    }

    /// -printf.ch:63 — the `%`-conversion scanner. The negatives matter: a bare
    /// `%` or an unterminated conversion must not be painted.
    #[test]
    fn printf_conversion_scanner_both_directions() {
        let scan = |s: &str| {
            let c: Vec<char> = s.chars().collect();
            printf_conversions(&c)
        };
        assert_eq!(scan("'%s'"), vec![(1, 3)]);
        assert_eq!(scan("\"%s=%d\\n\""), vec![(1, 3), (4, 6)]);
        assert_eq!(scan("'%-10.3f'"), vec![(1, 8)]);
        assert!(scan("no conversions here").is_empty());
        assert!(scan("100%").is_empty(), "a trailing %% converts nothing");
        assert!(scan("%z").is_empty(), "z is not a conversion letter");
        assert!(scan("%").is_empty());
    }

    /// fast-highlight:295-322 — `-fast-highlight-main-type` resolves a
    /// command-position word in ONE order, and `reserved` sits near the END of
    /// it. fish gives every reserved word its keyword face; f-sy-h gives
    /// `nocorrect` the precommand face (it is in the precommand table) and the
    /// typeset family the builtin face, while control flow stays reserved.
    #[test]
    fn reserved_words_resolve_through_the_type_ladder() {
        let _g = lock();
        assert_eq!(role_of("if true; then :; fi", "if"), HighlightRole::keyword);
        assert_eq!(role_of("while :; do :; done", "while"), HighlightRole::keyword);
        assert_eq!(
            role_of("nocorrect ls", "nocorrect"),
            HighlightRole::precommand
        );
        for decl in ["typeset", "declare", "local", "export", "readonly", "integer"] {
            let line = format!("{decl} x");
            assert_eq!(
                role_of(&line, decl),
                HighlightRole::builtin_,
                "{decl} is a builtin before it is a reserved word"
            );
        }
    }

    /// fast-highlight:616-623 — a precommand hands command position to the next
    /// word. fish's decoration arm is gated on the lexer's `cmdpos` flag, which
    /// the MODIFIER holds, so the real command word was never visited: `sudo ls`
    /// left `ls` uncoloured and `command zzqwx` never went red.
    #[test]
    fn precommand_passes_command_position_along() {
        let _g = lock();
        assert_eq!(role_of("sudo ls", "sudo"), HighlightRole::precommand);
        assert_ne!(
            role_of("sudo ls", "ls"),
            HighlightRole::param,
            "the word after sudo is a command"
        );
        assert_eq!(
            role_of("command zzqwx_not_a_command", "zzqwx_not_a_command"),
            HighlightRole::error,
            "and an invalid one still reports"
        );
        // fast-highlight:546-557 — `sudo -u NAME cmd`: the flag is an option,
        // NAME is its argument, and command position survives both.
        assert_eq!(role_of("sudo -u root ls", "-u"), HighlightRole::option);
        assert_eq!(role_of("sudo -u root ls", "root"), HighlightRole::normal);
        assert_ne!(role_of("sudo -u root ls", "ls"), HighlightRole::param);
    }

    /// fast-highlight:1047-1048 — a word carrying a LIVE `*`/`?` is `globbing`
    /// whole, not one painted character. The quoted half is the negative: the
    /// lexer leaves a quoted `*` as a plain character, so `ls '*.c'` is a string.
    #[test]
    fn glob_word_is_whole_word_and_quoting_defeats_it() {
        let _g = lock();
        let r = roles("ls *.c");
        assert!(
            r[3..6].iter().all(|&x| x == HighlightRole::globbing),
            "the whole word globs, not just the star: {r:?}"
        );
        let q = roles("ls '*.c'");
        assert!(
            !q.iter().any(|&x| x == HighlightRole::globbing),
            "a quoted glob is a string: {q:?}"
        );
        // fast-highlight:478 — `noglob` turns the whole thing off.
        let n = roles("noglob print *.c");
        assert!(
            !n.iter().any(|&x| x == HighlightRole::globbing),
            "noglob suppresses glob styling: {n:?}"
        );
    }

    /// fast-highlight:76-77 — options have their own cyan style, and a bare `--`
    /// switches it off for the rest of the command (fish:1091-1093).
    #[test]
    fn option_words_take_the_option_styles() {
        let _g = lock();
        assert_eq!(role_of("ls -l", "-l"), HighlightRole::option);
        assert_eq!(role_of("ls -", "-"), HighlightRole::option);
        assert_eq!(
            role_of("ls --color", "--color"),
            HighlightRole::double_hyphen_option
        );
        let after = roles("ls -- -l");
        assert_eq!(
            after[6],
            HighlightRole::param,
            "after a bare -- nothing is an option: {after:?}"
        );
    }

    /// fast-string-highlight:60 — the brackets pass. A matched pair takes
    /// `bracket-level-N` by nesting depth; an unmatched one takes
    /// `unknown-token`. fish has no such pass and painted every bracket with its
    /// generic operator face.
    #[test]
    fn brackets_are_levelled_and_unmatched_ones_report() {
        let _g = lock();
        let r = roles("(ls)");
        assert_eq!(r[0], HighlightRole::bracket_level_1);
        assert_eq!(r[3], HighlightRole::bracket_level_1);
        let n = roles("print ${(U)HOME}");
        assert_eq!(n[7], HighlightRole::bracket_level_1, "{{ is level 1");
        assert_eq!(n[8], HighlightRole::bracket_level_2, "( inside is level 2");
        assert_eq!(n[10], HighlightRole::bracket_level_2);
        assert_eq!(n[15], HighlightRole::bracket_level_1);
        let bad = roles("print (");
        assert_eq!(
            bad[6],
            HighlightRole::error,
            "an unmatched bracket is unknown-token: {bad:?}"
        );
    }

    /// fast-highlight:152-158 — `|`, `||`, `;`, `&`, `&&` are ONE class painted
    /// `commandseparator` (default `none`). fish splits them and routed `&&`
    /// through its operator face, which this engine renders as the glob colour.
    #[test]
    fn every_separator_is_a_commandseparator() {
        let _g = lock();
        for (line, at) in [("ls && ls", 3), ("ls || ls", 3), ("ls; ls", 2), ("ls | ls", 3)] {
            let r = roles(line);
            assert_eq!(
                r[at],
                HighlightRole::statement_terminator,
                "{line} at {at}: {r:?}"
            );
        }
    }

    /// fast-highlight:1279-1280 — a path is styled only when it EXISTS: `-d`
    /// gives `path-to-dir`, `-e` gives `path`, and nothing else is styled.
    /// fish additionally underlines any word that is a PREFIX of an existing
    /// path while the cursor is on it, which f-sy-h never does — that is what
    /// underlined the bare `c` of `arr=(a b c)` whenever a `c*` file existed.
    #[test]
    fn paths_are_styled_only_when_they_exist() {
        let _g = lock();
        assert_eq!(
            role_of("cat /etc/hosts", "/etc/hosts"),
            HighlightRole::path
        );
        assert_eq!(role_of("cat /tmp", "/tmp"), HighlightRole::path_to_dir);
        assert_eq!(
            role_of("cat /nonexistent_zzqwx", "/nonexistent_zzqwx"),
            HighlightRole::param,
            "a path that does not exist is a plain word"
        );
        assert_eq!(
            role_of("cat /etc/host", "/etc/host"),
            HighlightRole::param,
            "a PREFIX of an existing path is not a path"
        );
    }

    /// fast-highlight:86 vs :82 — `$var` is `variable` (fg=113) on its own and
    /// `back-or-dollar-double-quoted-argument` (cyan) inside `"…"`. fish used one
    /// operator face for both.
    #[test]
    fn variable_role_depends_on_quoting() {
        let _g = lock();
        assert_eq!(role_of("print $HOME", "$HOME"), HighlightRole::variable);
        assert_eq!(
            role_of("print \"a $HOME b\"", "$HOME"),
            HighlightRole::dollar_in_dquote
        );
    }

    /// fish:687-690 marks the opening quote of an unterminated string with its
    /// error face; f-sy-h styles the in-progress string normally, so every
    /// string was red until its closing quote was typed.
    #[test]
    fn an_unterminated_quote_is_not_an_error() {
        let _g = lock();
        let r = roles("print 'abc");
        assert_eq!(r[6], HighlightRole::quote, "{r:?}");
        assert!(!r.iter().any(|&x| x == HighlightRole::error), "{r:?}");
        let d = roles("print \"abc");
        assert_eq!(d[6], HighlightRole::dquoted, "{d:?}");
        assert!(!d.iter().any(|&x| x == HighlightRole::error), "{d:?}");
    }

    /// fast-highlight:75 / :743-747 / :1062-1063 — history expansion has its own
    /// style, in argument AND command position (`sudo !!`), and a lone `!` is
    /// NOT one (f-sy-h requires `-n ${__arg[2]}`).
    #[test]
    fn history_expansion_needs_a_word_after_the_bang() {
        let _g = lock();
        assert_eq!(
            role_of("print !!", "!!"),
            HighlightRole::history_expansion
        );
        assert_eq!(
            role_of("sudo !!", "!!"),
            HighlightRole::history_expansion,
            "also in command position"
        );
        assert_ne!(
            role_of("! ls", "!"),
            HighlightRole::history_expansion,
            "a lone ! is the negation reserved word, not an expansion"
        );
    }

    /// fast-highlight:113 + :890-899 — the body of a command substitution is
    /// re-highlighted under the SECONDARY theme. The nesting rule is the other
    /// half: fast-highlight:803 recurses only into a `$(` with no nested paren,
    /// so the OUTER body of `$(print $(ls))` keeps the word's own style.
    #[test]
    fn command_substitution_body_uses_the_secondary_theme() {
        let _g = lock();
        let ctx = OperationContext::empty();
        let mut colors = Vec::new();
        highlight_shell("print $(ls)", &mut colors, &ctx, true, None);
        assert!(colors[8].secondary, "the nested command is secondary");
        assert!(!colors[6].secondary, "the leading $ is not");

        let mut nested = Vec::new();
        highlight_shell("print $(print $(ls))", &mut nested, &ctx, true, None);
        assert!(
            !nested[8].secondary,
            "an outer body with a nested paren is not recursed: {:?}",
            &nested[6..14]
        );
        assert!(nested[16].secondary, "the innermost one is");
    }

    /// fast-highlight:1160-1190 — math highlighting. `mathvar` vs `matherr` is
    /// the predicate: an existing parameter is blue, an unknown one is red.
    #[test]
    fn math_numbers_and_variables_split_on_existence() {
        let _g = lock();
        let r = roles("print $((1+2))");
        assert_eq!(r[9], HighlightRole::mathnum, "{r:?}");
        assert_eq!(r[11], HighlightRole::mathnum, "{r:?}");
        assert_eq!(r[7], HighlightRole::double_paren, "{r:?}");
        crate::ported::params::setsparam("zzmath_defined", "1");
        assert_eq!(
            role_of("print $((zzmath_defined))", "zzmath_defined"),
            HighlightRole::mathvar,
            "a defined parameter is a mathvar"
        );
        crate::ported::params::unsetparam("zzmath_defined");
        assert_eq!(
            role_of("print $((zzqwx_no_such_param))", "zzqwx_no_such_param"),
            HighlightRole::matherr,
            "an unset name is a math error"
        );
    }

    /// fast-highlight:96-98 — `<<<` is `here-string-tri` and the word after it is
    /// `here-string-text`, not a filename to validate.
    #[test]
    fn here_string_operator_and_text() {
        let _g = lock();
        assert_eq!(role_of("cat <<< hi", "<<<"), HighlightRole::here_string_tri);
        assert_eq!(
            role_of("cat <<< hi", "hi"),
            HighlightRole::here_string_text
        );
        assert_eq!(
            role_of("cat <<< $HOME", "$HOME"),
            HighlightRole::here_string_var
        );
    }

    /// fast-highlight:667 / :698 — while `braces_stack` carries 'T', every word
    /// after a typeset-family command is an `assign`, options included.
    #[test]
    fn typeset_arguments_are_assignments_not_options() {
        let _g = lock();
        assert_eq!(role_of("typeset -g x", "-g"), HighlightRole::assign);
        assert_eq!(role_of("declare -a arr", "-a"), HighlightRole::assign);
        assert_eq!(
            role_of("ls -g", "-g"),
            HighlightRole::option,
            "outside typeset the same word IS an option"
        );
    }

    /// -autoload.ch:62-72 — the autoload chroma: a name found under $fpath is
    /// `correct-subtle`, one that is not is `incorrect-subtle`.
    #[test]
    fn autoload_chroma_splits_on_fpath() {
        let _g = lock();
        let dir = std::env::temp_dir().join("zshrs_hl_fpath_test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("zzfn_present"), "zzfn_present() { : }").unwrap();
        crate::ported::params::setaparam("fpath", vec![dir.to_string_lossy().into_owned()]);
        assert_eq!(
            role_of("autoload -Uz zzfn_present", "zzfn_present"),
            HighlightRole::correct_subtle
        );
        assert_eq!(
            role_of("autoload -Uz zzfn_absent", "zzfn_absent"),
            HighlightRole::incorrect_subtle
        );
        assert_eq!(
            role_of("autoload -Uz zzfn_present", "-Uz"),
            HighlightRole::option
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The configuration contract: `$ZSH_HIGHLIGHT_STYLES[key]` must still beat
    /// every built-in default this port introduced.
    #[test]
    fn zsh_highlight_styles_overrides_the_new_defaults() {
        let _g = lock();
        let plain = HighlightColorResolver::resolve_spec_uncached(&HighlightSpec::with_fg(
            HighlightRole::globbing,
        ));
        crate::ported::params::sethparam(
            "ZSH_HIGHLIGHT_STYLES",
            vec!["globbing".into(), "fg=magenta".into()],
        );
        let themed = HighlightColorResolver::resolve_spec_uncached(&HighlightSpec::with_fg(
            HighlightRole::globbing,
        ));
        crate::ported::params::unsetparam("ZSH_HIGHLIGHT_STYLES");
        assert_ne!(
            plain, themed,
            "the user's globbing style must beat the built-in fg=blue,bold"
        );
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
        // "d — double-quoted-argument (fast-highlight:80), a DIFFERENT style
        // key from single-quoted-argument
        assert_eq!(roles[4], HighlightRole::dquoted);
        assert_eq!(roles[5], HighlightRole::dquoted);
        // $V inside dquotes — back-or-dollar-double-quoted-argument
        // (fast-highlight:82), not the generic operator face
        assert_eq!(roles[6], HighlightRole::dollar_in_dquote);
        assert_eq!(roles[7], HighlightRole::dollar_in_dquote);
        // closing "
        assert_eq!(roles[8], HighlightRole::dquoted);
        // Outside a quote f-sy-h decides at WORD granularity
        // (fast-highlight:1047-1050), so the string scanner leaves the trailing
        // `$X*` on the base face; painting `$`/`X`/`*` individually here is what
        // made `ls *.c` colour only the `*`.
        assert_eq!(roles[9], HighlightRole::param);
        assert_eq!(roles[10], HighlightRole::param);
        assert_eq!(roles[11], HighlightRole::param);
    }

    #[test]
    fn color_string_unclosed_quote_keeps_the_quote_style() {
        let _g = lock();
        // fish:687-690 flags the opening quote of an unterminated string with
        // its error face. f-sy-h has no unterminated branch at all
        // (fast-highlight:884-960): the in-progress string keeps
        // `single-quoted-argument`, which is what a user sees for every string
        // they have not finished typing.
        let s = "'unclosed";
        let n = s.chars().count();
        let mut colors = vec![HighlightSpec::default(); n];
        color_string_internal(s, HighlightSpec::with_fg(HighlightRole::param), &mut colors);
        assert!(colors.iter().all(|c| c.foreground == HighlightRole::quote));
    }

    #[test]
    fn color_string_dollar_quote_escapes() {
        let _g = lock();
        // $'\x41' — a valid escape.
        let s = "$'\\x41'";
        let n = s.chars().count();
        let mut colors = vec![HighlightSpec::default(); n];
        color_string_internal(s, HighlightSpec::with_fg(HighlightRole::param), &mut colors);
        let roles: Vec<HighlightRole> = colors.iter().map(|c| c.foreground).collect();
        assert_eq!(roles[2], HighlightRole::escape, "roles {roles:?}");

        // fish:583-586 errors when a numeric escape exceeds the code point
        // maximum. `-fast-highlight-dollar-string` (fast-highlight:1198-1230)
        // validates only the SHAPE, never the value, so an over-large escape is
        // an ordinary escape there.
        let s2 = "$'\\U110000'";
        let n2 = s2.chars().count();
        let mut colors2 = vec![HighlightSpec::default(); n2];
        color_string_internal(
            s2,
            HighlightSpec::with_fg(HighlightRole::param),
            &mut colors2,
        );
        assert_eq!(colors2[2].foreground, HighlightRole::escape, "{colors2:?}");

        // The shape check that DOES fire: `\x` with no hex digit after it
        // (fast-highlight:1221-1224).
        let s3 = "$'\\x'";
        let n3 = s3.chars().count();
        let mut colors3 = vec![HighlightSpec::default(); n3];
        color_string_internal(
            s3,
            HighlightSpec::with_fg(HighlightRole::param),
            &mut colors3,
        );
        assert_eq!(colors3[2].foreground, HighlightRole::error, "{colors3:?}");
    }

    #[test]
    fn color_variable_subscript() {
        let _g = lock();
        let s: Vec<char> = "$arr[1]x".chars().collect();
        let mut colors = vec![HighlightSpec::default(); s.len()];
        let consumed = color_variable(&s, &mut colors, HighlightRole::variable);
        // $arr[1] consumed; trailing x not.
        assert_eq!(consumed, 7);
        assert_eq!(colors[0].foreground, HighlightRole::variable);
        assert_eq!(colors[4].foreground, HighlightRole::variable); // [
        assert_eq!(colors[6].foreground, HighlightRole::variable); // ]
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
                HighlightRole::builtin_,
                "{name} is an extension builtin and must colour as a builtin"
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

        // `echo` resolves via builtintab. fast-highlight:306-307 gives a builtin
        // its OWN style key (`builtin`), ahead of `command` in the resolution
        // order — both default to fg=green but a theme can separate them.
        let line = "echo hi";
        let mut colors = Vec::new();
        highlight_shell(line, &mut colors, &ctx, /*io_ok=*/ true, None);
        assert_eq!(colors.len(), line.chars().count());
        assert_eq!(
            colors[0].foreground,
            HighlightRole::builtin_,
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

    /// fast-highlight:698-699 — an assignment word is ONE style, `assign`,
    /// whose default is `none` (fast-highlight:84). fish:1018-1024 singles the
    /// `=` out with its operator face, which this engine renders as the glob
    /// colour, so every `FOO=bar` grew a blue `=` that f-sy-h never draws.
    #[test]
    fn highlight_assignment_is_one_flat_assign_span() {
        let _g = lock();
        let ctx = OperationContext::empty();
        let line = "FOO=bar echo x";
        let mut colors = Vec::new();
        highlight_shell(line, &mut colors, &ctx, true, None);
        let eq = line.chars().position(|c| c == '=').unwrap();
        for i in 0..line.chars().position(|c| c == ' ').unwrap() {
            assert_eq!(colors[i].foreground, HighlightRole::assign, "{colors:?}");
        }
        assert_eq!(colors[eq].foreground, HighlightRole::assign, "{colors:?}");
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

