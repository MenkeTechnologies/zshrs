//! Shared type definitions for the Base/ completion subsystem.
//!
//! Historically this module hosted both the type definitions
//! (`MainCompleteState`, `TagManager`, `CompletionContext`,
//! `CompleterResult`, `Value`, `Alternative`) AND inline ports of
//! every Base/ shell function (`_main_complete`, `_normal`,
//! `_alternative`, `_values`, `_description`, `_message`, etc.).
//! The per-function bodies have now been extracted into
//! `compsys/ported/Base/{Core,Completer,Utility}/_NAME.rs` to mirror
//! zsh's `Completion/Base/{Core,Completer,Utility,Widget}/` taxonomy
//! one-file-per-shell-function.
//!
//! What lives here now:
//!   * `MainCompleteState` — global state object passed through every
//!     completer.
//!   * `CompletionContext` — current `:completion:…:…:` context.
//!   * `TagManager` — tag offering / requested / wanted accounting.
//!   * `CompleterResult` — `Matched` / `NoMatch` / `Skip`.
//!   * `Value`, `Alternative` — spec parsers shared by `_values` /
//!     `_alternative`.
//!   * Re-exports of every extracted Base/ fn so external callers
//!     spelled `compsys::base::_main_complete` (and the
//!     `compsys::base::{…}` re-exports in `compsys/lib.rs`) continue
//!     to resolve.

use std::collections::{HashMap, HashSet};

// =============================================================================
// Core context / state types
// =============================================================================

/// Completion context for tag-based completion
#[derive(Clone, Debug, Default)]
pub struct CompletionContext {
    /// Current context string (e.g., ":completion::complete:git:")
    pub context: String,
    /// Current completer being used
    pub completer: String,
    /// Completer index (1-based)
    pub completer_num: usize,
    /// Matcher specification
    pub matcher: String,
    /// Matcher index (1-based)
    pub matcher_num: usize,
}

// TagManager deleted — dup of zsh's tag-set machinery exposed via
// `bin_comptags` at `src/ported/zle/computil.rs:6364`. Engine ports
// that need tag-order accounting should call
// `crate::ported::zle::computil::bin_comptags("comptags", &argv, &ops, 0)`
// directly.

/// Result from a completer function
#[derive(Clone, Debug)]
pub enum CompleterResult {
    /// Matches were added
    Matched,
    /// No matches, but not an error
    NoMatch,
    /// Skip remaining completers
    Skip,
}

/// Completer function type
pub type CompleterFn = fn(&mut MainCompleteState) -> CompleterResult;

/// State for _main_complete
#[derive(Debug)]
pub struct MainCompleteState {
    // `comp: CompletionState` field removed alongside compcore.rs
    // deletion. Engine ports must now read/write shell-side state in
    // `src/ported/zle/compcore.rs` directly.
    // `tags: TagManager` field removed alongside TagManager deletion.
    // Engine ports call bin_comptags directly.
    /// Context
    pub ctx: CompletionContext,
    /// Completers to use
    pub completers: Vec<String>,
    /// Last completion info
    pub lastcomp: HashMap<String, String>,
    /// Pre-completion functions
    pub prefuncs: Vec<String>,
    /// Post-completion functions
    pub postfuncs: Vec<String>,
    /// `$compcontext` shell variable — user-supplied context override
    /// read by `_complete` (`Completion/Base/Completer/_complete:14`).
    /// Empty string means unset.
    pub compcontext: String,
    /// `${(t)compcontext}` type marker — set to "array" or
    /// "association" by callers that wired `compcontext` to a typed
    /// value (matches shell:16 `[[ "${(t)compcontext}" = *array* ]]`
    /// and shell:21 `*assoc*` branches).
    pub compcontext_type: String,
    /// When `compcontext_type == "array"`, the array elements.
    pub compcontext_array: Vec<String>,
    /// When `compcontext_type == "association"`, the assoc keys/values.
    pub compcontext_assoc: Vec<(String, String)>,
    /// Return value
    pub ret: i32,
}

impl MainCompleteState {
    pub fn new(_line: &str, _cursor: usize) -> Self {
        Self {
            ctx: CompletionContext::default(),
            completers: vec!["_complete".to_string(), "_ignored".to_string()],
            lastcomp: HashMap::new(),
            prefuncs: Vec::new(),
            postfuncs: Vec::new(),
            compcontext: String::new(),
            compcontext_type: String::new(),
            compcontext_array: Vec::new(),
            compcontext_assoc: Vec::new(),
            ret: 1,
        }
    }

    /// Get the current context string for zstyle lookups
    pub fn context_string(&self) -> String {
        format!(":completion:{}:{}:", self.ctx.context, self.ctx.completer)
    }
}

// =============================================================================
// Spec types used by _alternative / _values
// =============================================================================
/// Value with optional argument for _values
#[derive(Clone, Debug)]
pub struct Value {
    pub name: String,
    pub description: String,
    pub has_arg: bool,
    pub arg_description: String,
    pub action: String,
}

impl Value {
    /// Parse "name\[description\]:arg-desc:action" format
    pub fn parse(spec: &str) -> Option<Self> {
        let spec = spec.trim();
        if spec.is_empty() {
            return None;
        }

        // Parse name[description]
        let (name, rest) = if let Some(bracket_start) = spec.find('[') {
            if let Some(bracket_end) = spec[bracket_start..].find(']') {
                let name = spec[..bracket_start].to_string();
                let desc = spec[bracket_start + 1..bracket_start + bracket_end].to_string();
                let rest = &spec[bracket_start + bracket_end + 1..];
                (name, (desc, rest))
            } else {
                (spec.to_string(), (String::new(), ""))
            }
        } else if let Some(colon) = spec.find(':') {
            (spec[..colon].to_string(), (String::new(), &spec[colon..]))
        } else {
            (spec.to_string(), (String::new(), ""))
        };

        let (description, rest) = rest;

        // Parse :arg-desc:action
        let (has_arg, arg_description, action) = if let Some(after_colon) = rest.strip_prefix(':') {
            let parts: Vec<&str> = after_colon.splitn(2, ':').collect();
            (
                true,
                parts.first().unwrap_or(&"").to_string(),
                parts.get(1).unwrap_or(&"").to_string(),
            )
        } else {
            (false, String::new(), String::new())
        };

        Some(Self {
            name,
            description,
            has_arg,
            arg_description,
            action,
        })
    }
}

// =============================================================================
// Back-compat re-exports — every Base/ fn that used to live here now
// resolves to its per-file port under `crate::compsys::ported::`. Names match the
// upstream zsh shell-function names. Pre-rename aliases
// (`completer_correct` → `_correct`, `next_label` → `_next_label`,
// `message` → `_message`, `_description` → `base_description` already
// at the lib.rs layer) are kept so `compsys/lib.rs`'s `pub use
// base::{…}` continues to compile.
// =============================================================================

pub use crate::compsys::ported::{
    _all_labels, _alternative, _approximate, _complete, _description, _dispatch, _ignored,
    _main_complete, _message, _multi_parts, _next_label, _normal, _sep_parts, _values, _wanted,
    get_ignored_patterns, is_ignored,
};

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_value_parse() {
        let val = Value::parse("debug[enable debugging]").unwrap();
        assert_eq!(val.name, "debug");
        assert_eq!(val.description, "enable debugging");
        assert!(!val.has_arg);

        let val = Value::parse("level[set level]:number:").unwrap();
        assert_eq!(val.name, "level");
        assert!(val.has_arg);
        assert_eq!(val.arg_description, "number");
    }

    #[test]
    fn test_value_parse_no_description() {
        let val = Value::parse("verbose").unwrap();
        assert_eq!(val.name, "verbose");
        assert_eq!(val.description, "");
        assert!(!val.has_arg);
    }

    #[test]
    fn test_value_parse_with_action() {
        let val = Value::parse("file[select file]:filename:_files").unwrap();
        assert_eq!(val.name, "file");
        assert_eq!(val.description, "select file");
        assert!(val.has_arg);
        assert_eq!(val.arg_description, "filename");
        assert_eq!(val.action, "_files");
    }

    #[test]
    fn test_context_string() {
        let mut state = MainCompleteState::new("git checkout", 12);
        state.ctx.context = "complete".to_string();
        state.ctx.completer = "complete".to_string();
        assert_eq!(state.context_string(), ":completion:complete:complete:");
    }

    #[test]
    fn test_is_ignored() {
        let patterns = vec![
            "*.pyc".to_string(),
            "__pycache__".to_string(),
            ".git*".to_string(),
        ];

        assert!(is_ignored("file.pyc", &patterns));
        assert!(is_ignored("__pycache__", &patterns));
        assert!(is_ignored(".git", &patterns));
        assert!(is_ignored(".gitignore", &patterns));
        assert!(!is_ignored("main.py", &patterns));
        assert!(!is_ignored("git", &patterns));
    }

    #[test]
    fn test_is_ignored_empty_patterns() {
        let patterns: Vec<String> = vec![];
        assert!(!is_ignored("anything", &patterns));
    }


    #[test]
    fn test_completer_result_variants() {
        let matched = CompleterResult::Matched;
        let no_match = CompleterResult::NoMatch;
        let skip = CompleterResult::Skip;

        // Just verify they're distinct (for match arms)
        assert!(matches!(matched, CompleterResult::Matched));
        assert!(matches!(no_match, CompleterResult::NoMatch));
        assert!(matches!(skip, CompleterResult::Skip));
    }

    #[test]
    fn test_completion_context_default() {
        let ctx = CompletionContext::default();
        assert_eq!(ctx.context, "");
        assert_eq!(ctx.completer, "");
        assert_eq!(ctx.completer_num, 0);
        assert_eq!(ctx.matcher, "");
        assert_eq!(ctx.matcher_num, 0);
    }
}
