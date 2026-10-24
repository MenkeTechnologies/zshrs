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

/// `complete` builtin per-command spec (bash-style), bash's `COMPSPEC`
/// (pcomplete.h).
#[derive(Debug, Clone, Default)]
pub struct CompSpec {
    pub actions: Vec<String>,       // action names (`COMPACTS`): -a, -A function, …
    pub options: Vec<String>,       // -o names (`COMPOPTS`)
    pub wordlist: Option<String>,   // -W wordlist
    pub function: Option<String>,   // -F function
    pub command: Option<String>,    // -C command
    pub globpat: Option<String>,    // -G glob
    pub prefix: Option<String>,     // -P prefix
    pub suffix: Option<String>,     // -S suffix
    pub filterpat: Option<String>,  // -X filterpat
}

/// The pseudo-command keys `complete -D` / `-E` / `-I` store their specs
/// under (bash pcomplete.h:109-111).
pub const EMPTYCMD: &str = "_EmptycmD_";
/// See [`EMPTYCMD`].
pub const DEFAULTCMD: &str = "_DefaultCmD_";
/// See [`EMPTYCMD`].
pub const INITIALWORD: &str = "_InitialWorD_";

/// bash complete.def `compacts[]`: action name and its single-letter
/// option (0 = only reachable as `-A name`), in the table's order, which is
/// the order `complete -p` prints them in.
pub const COMPACTS: &[(&str, u8)] = &[
    ("alias", b'a'),
    ("arrayvar", 0),
    ("binding", 0),
    ("builtin", b'b'),
    ("command", b'c'),
    ("directory", b'd'),
    ("disabled", 0),
    ("enabled", 0),
    ("export", b'e'),
    ("file", b'f'),
    ("function", 0),
    ("helptopic", 0),
    ("hostname", 0),
    ("group", b'g'),
    ("job", b'j'),
    ("keyword", b'k'),
    ("running", 0),
    ("service", b's'),
    ("setopt", 0),
    ("shopt", 0),
    ("signal", 0),
    ("stopped", 0),
    ("user", b'u'),
    ("variable", b'v'),
];

/// bash complete.def `compopts[]`, in print order.
pub const COMPOPTS: &[&str] = &[
    "bashdefault",
    "default",
    "dirnames",
    "filenames",
    "fullquote",
    "noquote",
    "nosort",
    "nospace",
    "plusdirs",
];

/// bash's `prog_completes` hash table (pcomplib.c, hashlib.c), kept as
/// buckets so `complete -p` walks it in bash's order: bucket 0 upward,
/// newest entry first within a bucket.
#[derive(Debug, Clone)]
pub struct ProgcompTable {
    buckets: Vec<Vec<(String, CompSpec, u32)>>,
    nentries: usize,
    // `prog_completes != 0`: the table exists from the first insert on.
    created: bool,
}

impl Default for ProgcompTable {
    fn default() -> Self {
        // pcomplib.c:40 `COMPLETE_HASH_BUCKETS 512`.
        ProgcompTable {
            buckets: vec![Vec::new(); 512],
            nentries: 0,
            created: false,
        }
    }
}

impl ProgcompTable {
    /// hashlib.c:208 `hash_string` — FNV-1 over the C `char`s, so a high
    /// byte enters sign-extended wherever `char` is signed.
    fn hash_string(s: &str) -> u32 {
        let mut i: u32 = 2166136261; // FNV_OFFSET
        for &b in s.as_bytes() {
            i = i.wrapping_mul(16777619); // FNV_PRIME, written as shifts in C
            i ^= (b as libc::c_char) as i32 as u32;
        }
        i
    }

    fn bucket(&self, h: u32) -> usize {
        (h as usize) & (self.buckets.len() - 1)
    }

    pub fn get(&self, cmd: &str) -> Option<&CompSpec> {
        let h = Self::hash_string(cmd);
        self.buckets[self.bucket(h)]
            .iter()
            .find(|(k, _, kh)| *kh == h && k == cmd)
            .map(|(_, cs, _)| cs)
    }

    /// pcomplib.c:171 `progcomp_insert` → hashlib.c:318 `hash_insert`: an
    /// existing key keeps its place and gets the new spec.
    pub fn insert(&mut self, cmd: &str, cs: CompSpec) {
        let h = Self::hash_string(cmd);
        let b = self.bucket(h);
        if let Some(e) = self.buckets[b].iter_mut().find(|(k, _, kh)| *kh == h && k == cmd) {
            e.1 = cs;
            return;
        }
        // hashlib.c:41 HASH_SHOULDGROW → hash_grow ×4 (c:154), rehashing
        // old buckets in order with head insertion (c:139-147).
        if self.nentries >= self.buckets.len() * 2 {
            let nsize = self.buckets.len() * 4;
            let old = std::mem::replace(&mut self.buckets, vec![Vec::new(); nsize]);
            for e in old.into_iter().flatten() {
                let nb = self.bucket(e.2);
                self.buckets[nb].insert(0, e);
            }
        }
        self.created = true; // c:178-179 progcomp_create
        let b = self.bucket(h);
        self.buckets[b].insert(0, (cmd.to_string(), cs, h));
        self.nentries += 1;
    }

    /// pcomplib.c:151 `progcomp_remove`; false when there was no entry.
    pub fn remove(&mut self, cmd: &str) -> bool {
        if !self.created {
            return true; // c:155-156 `if (prog_completes == 0) return 1;`
        }
        let h = Self::hash_string(cmd);
        let b = self.bucket(h);
        match self.buckets[b].iter().position(|(k, _, kh)| *kh == h && k == cmd) {
            Some(p) => {
                self.buckets[b].remove(p);
                self.nentries -= 1;
                true
            }
            None => false,
        }
    }

    /// pcomplib.c:136 `progcomp_flush` (the bucket count is kept).
    pub fn flush(&mut self) {
        self.buckets.iter_mut().for_each(Vec::clear);
        self.nentries = 0;
    }

    pub fn is_empty(&self) -> bool {
        self.nentries == 0
    }

    /// hashlib.c:397 `hash_walk` order.
    pub fn walk(&self) -> impl Iterator<Item = (&String, &CompSpec)> {
        self.buckets.iter().flatten().map(|(k, cs, _)| (k, cs))
    }
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
