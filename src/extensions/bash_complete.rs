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
    /// `name` field.
    pub name: String,
    /// `matches` field.
    pub matches: Vec<CompMatch>,
    /// `explanation` field.
    pub explanation: Option<String>,
    /// `sorted` field.
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

#[cfg(test)]
mod tests {
    use super::*;

    // === CompSpec: bash-style `complete` builtin spec defaults ===

    #[test]
    fn comp_spec_default_has_empty_actions_and_no_callbacks() {
        let s = CompSpec::default();
        assert!(
            s.actions.is_empty(),
            "default actions must be empty (no -a/-b/-c flags applied yet)"
        );
        assert!(s.wordlist.is_none(), "no -W wordlist by default");
        assert!(s.function.is_none(), "no -F function by default");
        assert!(s.command.is_none(), "no -C command by default");
        assert!(s.globpat.is_none(), "no -G glob by default");
        assert!(s.prefix.is_none(), "no -P prefix by default");
        assert!(s.suffix.is_none(), "no -S suffix by default");
    }

    #[test]
    fn comp_spec_clone_is_deep_independent() {
        // CompSpec derives Clone — mutating the clone must not bleed
        // back into the original. Catches a future move to `Rc` /
        // `Arc` field types that would share state.
        let mut a = CompSpec::default();
        a.actions.push("file".to_string());
        a.function = Some("__git_complete".to_string());
        let b = a.clone();
        a.actions.push("dir".to_string());
        a.function = Some("__docker_complete".to_string());
        assert_eq!(b.actions, vec!["file"]);
        assert_eq!(b.function.as_deref(), Some("__git_complete"));
    }

    #[test]
    fn comp_spec_actions_preserve_insertion_order() {
        // Bash `complete -A file -A directory -A user` records each
        // action in flag-order — CompSpec uses Vec to honor that.
        let mut s = CompSpec::default();
        for a in ["file", "directory", "user"] {
            s.actions.push(a.to_string());
        }
        assert_eq!(s.actions, vec!["file", "directory", "user"]);
    }

    // === CompMatch: per-candidate flags + override defaults ===

    #[test]
    fn comp_match_default_empty_word_and_no_flags() {
        let m = CompMatch::default();
        assert_eq!(m.word, "");
        assert!(m.display.is_none());
        assert!(m.prefix.is_none());
        assert!(m.suffix.is_none());
        assert!(m.hidden_prefix.is_none());
        assert!(m.hidden_suffix.is_none());
        assert!(m.ignored_prefix.is_none());
        assert!(m.ignored_suffix.is_none());
        assert!(m.group.is_none());
        assert!(m.description.is_none());
        assert!(m.remove_suffix.is_none());
        assert!(
            !m.file_match,
            "file_match must default to false (no -f flag)"
        );
        assert!(
            !m.quote_match,
            "quote_match must default to false (no -q flag)"
        );
    }

    #[test]
    fn comp_match_word_round_trip_preserves_value() {
        let m = CompMatch {
            word: "git-status".to_string(),
            description: Some("show working tree".to_string()),
            file_match: false,
            quote_match: true,
            ..Default::default()
        };
        assert_eq!(m.word, "git-status");
        assert_eq!(m.description.as_deref(), Some("show working tree"));
        assert!(m.quote_match);
    }

    // === CompGroup: ordering / grouping container defaults ===

    #[test]
    fn comp_group_default_has_no_name_no_matches_unsorted() {
        let g = CompGroup::default();
        assert_eq!(g.name, "");
        assert!(g.matches.is_empty());
        assert!(g.explanation.is_none());
        assert!(
            !g.sorted,
            "default groups are unsorted (callers opt in via -J)"
        );
    }

    #[test]
    fn comp_group_can_carry_multiple_matches() {
        let g = CompGroup {
            name: "files".to_string(),
            matches: vec![
                CompMatch {
                    word: "a.txt".to_string(),
                    file_match: true,
                    ..Default::default()
                },
                CompMatch {
                    word: "b.txt".to_string(),
                    file_match: true,
                    ..Default::default()
                },
            ],
            explanation: Some("text files".to_string()),
            sorted: true,
        };
        assert_eq!(g.matches.len(), 2);
        assert_eq!(g.matches[0].word, "a.txt");
        assert!(g.matches.iter().all(|m| m.file_match));
        assert!(g.sorted);
    }

    // === CompState: per-completion mutable state defaults ===

    #[test]
    fn comp_state_default_all_strings_empty_all_ints_zero() {
        let s = CompState::default();
        // Every String field defaults to "" — verify a representative
        // subset across param/list/quoting/etc. families.
        assert_eq!(s.context, "");
        assert_eq!(s.exact, "");
        assert_eq!(s.insert, "");
        assert_eq!(s.list, "");
        assert_eq!(s.parameter, "");
        assert_eq!(s.quoting, "");
        assert_eq!(s.unambiguous, "");
        assert_eq!(s.vared, "");
        // Every i32 field defaults to 0.
        assert_eq!(s.ignored, 0);
        assert_eq!(s.list_lines, 0);
        assert_eq!(s.list_max, 0);
        assert_eq!(s.nmatches, 0);
        assert_eq!(s.unambiguous_cursor, 0);
    }

    #[test]
    fn comp_state_clone_independence() {
        // Same independence guarantee as CompSpec — protects against
        // a future regression to a shared-state field type.
        let mut a = CompState::default();
        a.nmatches = 5;
        a.insert = "foo".to_string();
        let b = a.clone();
        a.nmatches = 99;
        a.insert = "bar".to_string();
        assert_eq!(b.nmatches, 5);
        assert_eq!(b.insert, "foo");
    }

    #[test]
    fn comp_state_int_fields_round_trip_negative_values() {
        // Some compstate ints are signed by design (e.g.
        // unambiguous_cursor can carry -1 for "none"). Verify the
        // field type carries the sign through.
        let s = CompState {
            unambiguous_cursor: -1,
            nmatches: -42,
            ..Default::default()
        };
        assert_eq!(s.unambiguous_cursor, -1);
        assert_eq!(s.nmatches, -42);
    }
}
