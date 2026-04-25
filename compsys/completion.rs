//! Core completion data structures

use std::collections::HashMap;

/// Flags controlling completion behavior (maps to zsh CMF_* flags)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CompletionFlags(u32);

impl CompletionFlags {
    pub const NONE: Self = Self(0);
    /// Remove suffix when a space is typed (compadd -q)
    pub const REMOVE: Self = Self(1 << 0);
    /// This is a file completion (compadd -f)
    pub const FILE: Self = Self(1 << 1);
    /// This is a directory completion (compadd -/)
    pub const DIRECTORY: Self = Self(1 << 2);
    /// Don't list this match (compadd -n)
    pub const NOLIST: Self = Self(1 << 3);
    /// Display one match per line (compadd -l)
    pub const DISPLINE: Self = Self(1 << 4);
    /// Don't insert space after completion
    pub const NOSPACE: Self = Self(1 << 5);
    /// Quote the completion
    pub const QUOTE: Self = Self(1 << 6);
    /// This is a parameter expansion
    pub const ISPAR: Self = Self(1 << 7);
    /// Pack completions tightly
    pub const PACKED: Self = Self(1 << 8);
    /// Display in rows instead of columns
    pub const ROWS: Self = Self(1 << 9);
    /// All matches marker
    pub const ALL: Self = Self(1 << 10);
    /// Dummy/placeholder match
    pub const DUMMY: Self = Self(1 << 11);
    /// Multiple matches with same display
    pub const MULT: Self = Self(1 << 12);
    /// First of multiple matches
    pub const FMULT: Self = Self(1 << 13);
    /// Marked for deletion (internal)
    pub const DELETE: Self = Self(1 << 14);
    /// Don't quote the completion (opposite of QUOTE)
    pub const NOQUOTE: Self = Self(1 << 15);

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    pub const fn bits(self) -> u32 {
        self.0
    }
}

impl std::ops::BitOr for CompletionFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for CompletionFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl std::ops::BitAnd for CompletionFlags {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}

/*
bitflags::bitflags! {
    /// Flags controlling completion behavior (maps to zsh CMF_* flags)
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct CompletionFlags: u32 {
        /// Remove suffix when a space is typed (compadd -q)
        const REMOVE = 1 << 0;
        /// This is a file completion (compadd -f)
        const FILE = 1 << 1;
        /// This is a directory completion (compadd -/)
        const DIRECTORY = 1 << 2;
        /// Don't list this match (compadd -n)
        const NOLIST = 1 << 3;
        /// Display one match per line (compadd -l)
        const DISPLINE = 1 << 4;
        /// Don't insert space after completion
        const NOSPACE = 1 << 5;
        /// Quote the completion
        const QUOTE = 1 << 6;
        /// This is a parameter expansion
        const ISPAR = 1 << 7;
        /// Pack completions tightly
        const PACKED = 1 << 8;
        /// Display in rows instead of columns
        const ROWS = 1 << 9;
        /// All matches marker
        const ALL = 1 << 10;
        /// Dummy/placeholder match
        const DUMMY = 1 << 11;
        /// Multiple matches with same display
        const MULT = 1 << 12;
        /// First of multiple matches
        const FMULT = 1 << 13;
*/

/// A single completion match - the core data structure
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Completion {
    /// The actual string to insert (after prefixes/suffixes applied)
    pub str_: String,
    /// Original string before any transformations
    pub orig: String,
    /// Prefix to add before match (-P)
    pub pre: Option<String>,
    /// Suffix to add after match (-S)
    pub suf: Option<String>,
    /// Ignored prefix - moved from PREFIX to IPREFIX (-i)
    pub ipre: Option<String>,
    /// Ignored suffix - moved from SUFFIX to ISUFFIX (-I)  
    pub isuf: Option<String>,
    /// Path prefix (-p)
    pub ppre: Option<String>,
    /// Path suffix (-s)
    pub psuf: Option<String>,
    /// "Real" path prefix for file completions (-W)
    pub prpre: Option<String>,
    /// Display string (-d array element)
    pub disp: Option<String>,
    /// Description string (shown after completion)
    pub desc: Option<String>,
    /// Group name (-J/-V)
    pub group: Option<String>,
    /// Explanation string (-X)
    pub exp: Option<String>,
    /// Remove suffix chars (-r)
    pub rems: Option<String>,
    /// Remove suffix function (-R)
    pub remf: Option<String>,
    /// Auto-quote character
    pub autoq: Option<String>,
    /// Flags
    pub flags: CompletionFlags,
    /// Match number within group (1-indexed)
    pub rnum: i32,
    /// Global match number (1-indexed)  
    pub gnum: i32,
    /// File mode (for -f completions)
    pub mode: u32,
    /// File mode char (e.g., '/' for directory)
    pub modec: char,
}

impl Completion {
    pub fn new(word: impl Into<String>) -> Self {
        Self {
            str_: word.into(),
            ..Default::default()
        }
    }

    pub fn with_display(mut self, disp: impl Into<String>) -> Self {
        self.disp = Some(disp.into());
        self
    }

    pub fn with_description(mut self, exp: impl Into<String>) -> Self {
        self.exp = Some(exp.into());
        self
    }

    pub fn with_prefix(mut self, pre: impl Into<String>) -> Self {
        self.pre = Some(pre.into());
        self
    }

    pub fn with_suffix(mut self, suf: impl Into<String>) -> Self {
        self.suf = Some(suf.into());
        self
    }

    pub fn with_flags(mut self, flags: CompletionFlags) -> Self {
        self.flags = flags;
        self
    }

    /// Returns the string to display in completion list
    pub fn display_str(&self) -> &str {
        self.disp.as_deref().unwrap_or(&self.str_)
    }

    /// Returns the full string to insert (with prefixes/suffixes)
    pub fn insert_str(&self) -> String {
        let mut result = String::new();
        if let Some(ref pre) = self.pre {
            result.push_str(pre);
        }
        if let Some(ref ppre) = self.ppre {
            result.push_str(ppre);
        }
        result.push_str(&self.str_);
        if let Some(ref psuf) = self.psuf {
            result.push_str(psuf);
        }
        if let Some(ref suf) = self.suf {
            result.push_str(suf);
        }
        result
    }
}

/// Flags for completion groups (maps to zsh CGF_* flags)
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GroupFlags(u32);

impl GroupFlags {
    pub const NONE: Self = Self(0);
    /// Pack columns tightly (LIST_PACKED)
    pub const PACKED: Self = Self(1 << 0);
    /// Fill rows first instead of columns (LIST_ROWS_FIRST)
    pub const ROWS_FIRST: Self = Self(1 << 1);
    /// Has display lines (multiline entries)
    pub const HAS_DISPLINE: Self = Self(1 << 2);
    /// Has files (show type indicators)
    pub const FILES: Self = Self(1 << 3);
    /// Lines mode (not columns)
    pub const LINES: Self = Self(1 << 4);

    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

impl std::ops::BitOr for GroupFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for GroupFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// A group of completions (zsh Cmgroup)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompletionGroup {
    /// Group name
    pub name: String,
    /// Matches in this group
    pub matches: Vec<Completion>,
    /// Explanation strings
    pub explanations: Vec<String>,
    /// Explanation for the group header
    pub explanation: Option<String>,
    /// Whether group is sorted (-J) or unsorted (-V)
    pub sorted: bool,
    /// Group flags
    pub flags: GroupFlags,
    /// Number of matches to display (excludes hidden)
    pub lcount: usize,
}

impl CompletionGroup {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            sorted: true,
            ..Default::default()
        }
    }

    pub fn new_unsorted(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            sorted: false,
            ..Default::default()
        }
    }

    pub fn add_match(&mut self, m: Completion) {
        if !m.flags.contains(CompletionFlags::NOLIST) {
            self.lcount += 1;
        }
        self.matches.push(m);
    }

    pub fn add_explanation(&mut self, exp: impl Into<String>) {
        self.explanations.push(exp.into());
    }
}

/// List of completions with limit support
pub struct CompletionReceiver {
    groups: HashMap<String, CompletionGroup>,
    current_group: String,
    limit: usize,
    count: usize,
}

impl CompletionReceiver {
    pub fn new(limit: usize) -> Self {
        let mut groups = HashMap::new();
        groups.insert("default".to_string(), CompletionGroup::new("default"));
        Self {
            groups,
            current_group: "default".to_string(),
            limit,
            count: 0,
        }
    }

    pub fn unlimited() -> Self {
        Self::new(usize::MAX)
    }

    /// Begin a new group or switch to existing one
    pub fn begin_group(&mut self, name: impl Into<String>, sorted: bool) {
        let name = name.into();
        self.groups.entry(name.clone()).or_insert_with(|| {
            if sorted {
                CompletionGroup::new(&name)
            } else {
                CompletionGroup::new_unsorted(&name)
            }
        });
        self.current_group = name;
    }

    /// Add a completion to the current group
    pub fn add(&mut self, comp: Completion) -> bool {
        if self.count >= self.limit {
            return false;
        }
        self.count += 1;
        if let Some(group) = self.groups.get_mut(&self.current_group) {
            group.add_match(comp);
        }
        true
    }

    /// Add explanation to current group
    pub fn add_explanation(&mut self, exp: impl Into<String>) {
        if let Some(group) = self.groups.get_mut(&self.current_group) {
            group.add_explanation(exp);
        }
    }

    /// Total number of matches across all groups
    pub fn total_count(&self) -> usize {
        self.count
    }

    /// Get all groups
    pub fn groups(&self) -> &HashMap<String, CompletionGroup> {
        &self.groups
    }

    /// Take all completions, consuming self
    pub fn take(self) -> Vec<CompletionGroup> {
        self.groups.into_values().collect()
    }

    /// Get flat list of all matches
    pub fn all_matches(&self) -> Vec<&Completion> {
        self.groups
            .values()
            .flat_map(|g| g.matches.iter())
            .collect()
    }

    /// Get flat list of all completions (owned)
    pub fn all_completions(&self) -> Vec<Completion> {
        self.groups
            .values()
            .flat_map(|g| g.matches.clone())
            .collect()
    }
}

/// Type alias for a list of completions
pub type CompletionList = Vec<Completion>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_completion_flags_empty() {
        let flags = CompletionFlags::empty();
        assert_eq!(flags.bits(), 0);
        assert!(!flags.contains(CompletionFlags::FILE));
    }

    #[test]
    fn test_completion_flags_contains() {
        let flags = CompletionFlags::FILE | CompletionFlags::DIRECTORY;
        assert!(flags.contains(CompletionFlags::FILE));
        assert!(flags.contains(CompletionFlags::DIRECTORY));
        assert!(!flags.contains(CompletionFlags::REMOVE));
    }

    #[test]
    fn test_completion_flags_bitor() {
        let mut flags = CompletionFlags::FILE;
        flags |= CompletionFlags::NOSPACE;
        assert!(flags.contains(CompletionFlags::FILE));
        assert!(flags.contains(CompletionFlags::NOSPACE));
    }

    #[test]
    fn test_completion_flags_bitand() {
        let flags1 = CompletionFlags::FILE | CompletionFlags::DIRECTORY;
        let flags2 = CompletionFlags::FILE | CompletionFlags::NOSPACE;
        let result = flags1 & flags2;
        assert!(result.contains(CompletionFlags::FILE));
        assert!(!result.contains(CompletionFlags::DIRECTORY));
        assert!(!result.contains(CompletionFlags::NOSPACE));
    }

    #[test]
    fn test_completion_new() {
        let comp = Completion::new("test");
        assert_eq!(comp.str_, "test");
        assert!(comp.orig.is_empty()); // orig is String, not Option
        assert_eq!(comp.pre, None);
        assert_eq!(comp.suf, None);
        assert_eq!(comp.exp, None);
        assert_eq!(comp.disp, None);
        assert_eq!(comp.flags, CompletionFlags::NONE);
    }

    #[test]
    fn test_completion_with_description() {
        let comp = Completion::new("checkout").with_description("checkout a branch");
        assert_eq!(comp.exp, Some("checkout a branch".to_string()));
    }

    #[test]
    fn test_completion_with_flags() {
        let comp = Completion::new("dir/").with_flags(CompletionFlags::DIRECTORY);
        assert!(comp.flags.contains(CompletionFlags::DIRECTORY));
    }

    #[test]
    fn test_completion_display_str() {
        let comp = Completion::new("test");
        assert_eq!(comp.display_str(), "test");

        let mut comp2 = Completion::new("test");
        comp2.disp = Some("Test Display".to_string());
        assert_eq!(comp2.display_str(), "Test Display");
    }

    #[test]
    fn test_completion_insert_str() {
        let mut comp = Completion::new("test");
        assert_eq!(comp.insert_str(), "test");

        comp.pre = Some("pre-".to_string());
        comp.suf = Some("-suf".to_string());
        assert_eq!(comp.insert_str(), "pre-test-suf");
    }

    #[test]
    fn test_completion_group_new() {
        let group = CompletionGroup::new("files");
        assert_eq!(group.name, "files");
        assert!(group.matches.is_empty());
        assert!(group.sorted);
        assert!(group.explanation.is_none());
    }

    #[test]
    fn test_completion_group_new_unsorted() {
        let group = CompletionGroup::new_unsorted("history");
        assert!(!group.sorted);
    }

    #[test]
    fn test_completion_group_explanations() {
        let mut group = CompletionGroup::new("files");
        group.add_explanation("file completions");
        assert!(group.explanations.contains(&"file completions".to_string()));
    }

    #[test]
    fn test_completion_group_add_match() {
        let mut group = CompletionGroup::new("test");
        group.add_match(Completion::new("foo"));
        group.add_match(Completion::new("bar"));

        assert_eq!(group.matches.len(), 2);
        assert_eq!(group.matches[0].str_, "foo");
        assert_eq!(group.matches[1].str_, "bar");
    }

    #[test]
    fn test_completion_group_add_explanation() {
        let mut group = CompletionGroup::new("test");
        group.add_explanation("Select an option");

        // add_explanation adds to the explanations vec
        assert!(group.explanations.contains(&"Select an option".to_string()));
    }

    #[test]
    fn test_group_flags_default() {
        let flags = GroupFlags::default();
        assert!(!flags.contains(GroupFlags::PACKED));
        assert!(!flags.contains(GroupFlags::ROWS_FIRST));
        assert!(!flags.contains(GroupFlags::HAS_DISPLINE));
    }

    #[test]
    fn test_completion_receiver_new() {
        let receiver = CompletionReceiver::unlimited();
        assert_eq!(receiver.total_count(), 0);
        // Has default group
        assert_eq!(receiver.groups().len(), 1);
    }

    #[test]
    fn test_completion_receiver_add() {
        let mut receiver = CompletionReceiver::unlimited();
        receiver.begin_group("files", true);
        receiver.add(Completion::new("test.txt"));

        assert_eq!(receiver.total_count(), 1);
        assert!(receiver.groups().contains_key("files"));
    }

    #[test]
    fn test_completion_receiver_multiple_groups() {
        let mut receiver = CompletionReceiver::unlimited();

        receiver.begin_group("files", true);
        receiver.add(Completion::new("file1.txt"));
        receiver.add(Completion::new("file2.txt"));

        receiver.begin_group("directories", true);
        receiver.add(Completion::new("dir1/"));

        assert_eq!(receiver.total_count(), 3);
        // 3 groups: default + files + directories
        assert_eq!(receiver.groups().len(), 3);
    }

    #[test]
    fn test_completion_receiver_all_matches() {
        let mut receiver = CompletionReceiver::unlimited();

        receiver.begin_group("a", true);
        receiver.add(Completion::new("alpha"));

        receiver.begin_group("b", true);
        receiver.add(Completion::new("beta"));

        let matches = receiver.all_matches();
        assert_eq!(matches.len(), 2);
    }

    #[test]
    fn test_completion_receiver_take() {
        let mut receiver = CompletionReceiver::unlimited();
        receiver.begin_group("test", true);
        receiver.add(Completion::new("item"));

        let groups = receiver.take();
        // default group + test group
        assert!(groups.len() >= 1);
    }

    #[test]
    fn test_completion_with_all_fields() {
        let mut comp = Completion::new("main");
        comp.orig = "original".to_string();
        comp.pre = Some("pre".to_string());
        comp.suf = Some("suf".to_string());
        comp.exp = Some("description".to_string());
        comp.disp = Some("display".to_string());
        comp.flags = CompletionFlags::FILE | CompletionFlags::REMOVE;
        comp.rnum = 5;
        comp.gnum = 10;
        comp.modec = '/';

        assert_eq!(comp.orig, "original");
        assert_eq!(comp.modec, '/');
        assert!(comp.flags.contains(CompletionFlags::FILE));
    }

    #[test]
    fn test_completion_flags_all_flags() {
        // Verify all flags are distinct
        let all_flags = [
            CompletionFlags::REMOVE,
            CompletionFlags::FILE,
            CompletionFlags::DIRECTORY,
            CompletionFlags::NOLIST,
            CompletionFlags::DISPLINE,
            CompletionFlags::NOSPACE,
            CompletionFlags::QUOTE,
            CompletionFlags::ISPAR,
            CompletionFlags::PACKED,
            CompletionFlags::ROWS,
            CompletionFlags::ALL,
            CompletionFlags::DUMMY,
            CompletionFlags::MULT,
            CompletionFlags::FMULT,
            CompletionFlags::DELETE,
            CompletionFlags::NOQUOTE,
        ];

        for (i, flag1) in all_flags.iter().enumerate() {
            for (j, flag2) in all_flags.iter().enumerate() {
                if i != j {
                    assert_ne!(
                        flag1.bits(),
                        flag2.bits(),
                        "Flags at {} and {} have same bits",
                        i,
                        j
                    );
                }
            }
        }
    }
}
