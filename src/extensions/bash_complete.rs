//! Rust-original types for the bash-style `complete` builtin extension.
//!
//! These types back the `complete` / `compopt` / `compgen` builtins
//! that `src/extensions/ext_builtins.rs` exposes. They are NOT ports
//! of any zsh C source — zsh's native `compdef` / `compctl` / `compsys`
//! uses completely different types (`Cmatch` at comp_h.rs:334,
//! `Cmgroup` at comp_h.rs:269, `Compctl` at compctl_h.rs:198).
//!
//! Previous home was `src/ported/zle/computil.rs` which violated the
//! "no Rust-only types in ported files" invariant; moved here so the
//! ported zle/ tree stays a faithful C-source mirror.

/// `complete` builtin per-command spec (bash-style).
#[derive(Debug, Clone, Default)]
pub struct CompSpec {
    pub actions: Vec<String>,     // -a, -b, -c, etc.
    pub wordlist: Option<String>, // -W wordlist
    pub function: Option<String>, // -F function
    pub command: Option<String>,  // -C command
    pub globpat: Option<String>,  // -G glob
    pub prefix: Option<String>,   // -P prefix
    pub suffix: Option<String>,   // -S suffix
}

/// One completion-match candidate surfaced by the `complete` builtin.
#[derive(Debug, Clone, Default)]
pub struct CompMatch {
    pub word: String,                   // The actual completion word
    pub display: Option<String>,        // Display string (-d)
    pub prefix: Option<String>,         // -P prefix (inserted but not part of match)
    pub suffix: Option<String>,         // -S suffix (inserted but not part of match)
    pub hidden_prefix: Option<String>,  // -p hidden prefix
    pub hidden_suffix: Option<String>,  // -s hidden suffix
    pub ignored_prefix: Option<String>, // -i ignored prefix
    pub ignored_suffix: Option<String>, // -I ignored suffix
    pub group: Option<String>,          // -J/-V group name
    pub description: Option<String>,    // -X explanation
    pub remove_suffix: Option<String>,  // -r remove chars
    pub file_match: bool,               // -f flag
    pub quote_match: bool,              // -q flag
}

/// Completion group bundling multiple match candidates.
#[derive(Debug, Clone, Default)]
pub struct CompGroup {
    pub name: String,
    pub matches: Vec<CompMatch>,
    pub explanation: Option<String>,
    pub sorted: bool,
}

/// Per-completion state tracked across `complete` builtin invocations.
#[derive(Debug, Clone, Default)]
pub struct CompState {
    pub context: String,               // completion context
    pub exact: String,                 // exact match handling
    pub exact_string: String,          // the exact string if matched
    pub ignored: i32,                  // number of ignored matches
    pub insert: String,                // what to insert
    pub insert_positions: String,      // cursor positions after insert
    pub last_prompt: String,           // whether to return to last prompt
    pub list: String,                  // listing style
    pub list_lines: i32,               // number of lines for listing
    pub list_max: i32,                 // max matches to list
    pub nmatches: i32,                 // number of matches
    pub old_insert: String,            // previous insert value
    pub old_list: String,              // previous list value
    pub parameter: String,             // parameter being completed
    pub pattern_insert: String,        // pattern insert mode
    pub matchpat: String,              // pattern matching mode
    pub bslashquote: String,           // quoting type
    pub quoting: String,               // current quoting
    pub redirect: String,              // redirection type
    pub restore: String,               // restore mode
    pub to_end: String,                // move to end mode
    pub unambiguous: String,           // unambiguous prefix
    pub unambiguous_cursor: i32,       // cursor pos in unambiguous
    pub unambiguous_positions: String, // positions in unambiguous
    pub vared: String,                 // vared context
}
