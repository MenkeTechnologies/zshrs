//! Port of `_globquals` from `Completion/Zsh/Type/_globquals`.
//!
//! Full upstream body (277 lines, abridged):
//! ```text
//! sh:  1  #autoload
//! sh:  5  # Helper for completing glob qualifiers like (*.c)
//! sh: 30  if [[ -prefix [[:alpha:]] || -prefix [.\$,~^()] ]]; then
//! sh:130    quals=( single-char-qualifier list … )
//! sh:200    _describe -t globquals "glob qualifier" quals -Q -S ''
//! sh:270  fi
//! ```
//!
//! Heavy catalog of glob-qualifier letters (e.g. `-`, `@`, `*`,
//! `R`, `W`, etc.). Port emits the qualifier catalog via
//! `_describe` for the common path.

use crate::ported::exec::dispatch_function_call;
use crate::ported::params::setaparam;

const QUALS: &[&str] = &[
    "/:directory",
    "F:non-empty directory",
    ".:plain file",
    "@:symbolic link",
    "*:executable plain file",
    "%:special file",
    "r:owner-readable",
    "w:owner-writable",
    "x:owner-executable",
    "A:group-readable",
    "I:group-writable",
    "E:group-executable",
    "R:world-readable",
    "W:world-writable",
    "X:world-executable",
    "s:setuid",
    "S:setgid",
    "t:sticky bit",
    "d:specific device",
    "U:owned by current user",
    "G:owned by current group",
    "u:owned by specified user",
    "g:owned by specified group",
    "L:size in bytes (or k,m,g)",
    "m:mtime within N (days)",
    "a:atime within N",
    "c:ctime within N",
    "o:order by (n/L/l/a/m/c/d)",
    "O:reverse order",
    "N:nullglob",
    "Y:limit count",
    "-:dereference",
    "^:negate",
    "M:set MARK_DIRS",
    "T:apply suffix character per type",
    "P:prepend string",
    "e:eval qualifier",
    "+:user-defined predicate",
];

/// `_globquals` — complete a glob qualifier letter inside `(...)`.
pub fn _globquals() -> i32 {
    let q: Vec<String> = QUALS.iter().map(|s| s.to_string()).collect();
    setaparam("quals", q);
    dispatch_function_call(
        "_describe",
        &[
            "-t".to_string(),
            "globquals".to_string(),
            "glob qualifier".to_string(),
            "quals".to_string(),
            "-Q".to_string(),
            "-S".to_string(),
            "".to_string(),
        ],
    )
    .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_globquals(), 1);
    }

    #[test]
    fn publishes_quals_array() {
        let _g = crate::test_util::global_state_lock();
        let _ = _globquals();
        let q = crate::ported::params::getaparam("quals").unwrap_or_default();
        assert!(q.len() >= 30);
    }
}
