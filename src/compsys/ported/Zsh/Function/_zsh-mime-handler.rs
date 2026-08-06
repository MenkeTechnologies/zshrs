//! Port of `_zsh-mime-handler` from `Completion/Zsh/Function/_zsh-mime-handler`.
//!
//! `#compdef zsh-mime-handler`. When completing `zsh-mime-handler FILE…`,
//! ask the real handler (`zsh-mime-handler -l`) to print the fully-quoted
//! command line it *would* execute, re-split that into `$words`, then run
//! `_normal` so completion applies to the reconstructed command instead of
//! to the handler wrapper.
//!
//! Full upstream body (20 lines verbatim):
//! ```text
//! sh: 1  #compdef zsh-mime-handler
//! sh: 7  integer end_offset=$(( ${#words} - CURRENT ))
//! sh:13  words=(${(z)"$(zsh-mime-handler -l "${(@)words[2,-1]}")"})
//! sh:15  words=("${(@Q)words}")
//! sh:17  (( CURRENT = ${#words} - end_offset ))
//! sh:19  _normal
//! ```
//!
//! sh:3-6  — the offset is kept from the END of `$words` because the handler
//! is likely to change the START of the command line (it may drop the
//! `zsh-mime-handler` prefix and substitute the resolved program), so
//! `CURRENT` is re-derived from the end, not the beginning.

use crate::compsys::ported::_normal::normal_byname;
use crate::ported::hist::bufferwords;
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam};
use std::process::Command;

/// sh:7 — `end_offset = ${#words} - CURRENT`: distance of the cursor word
/// from the end of the word array. Kept from the end because the handler
/// rewrites the front of the line (sh:3-6).
fn compute_end_offset(nwords: usize, current: i64) -> i64 {
    nwords as i64 - current
}

/// sh:13/15 — `${(z)"$(…)"}` then `"${(@Q)words}"`.
///
/// `(z)` runs the shell word-splitter (`bufferwords`) over the handler's
/// quoted output; `(Q)` then removes one level of quoting from each word.
/// The codebase's `bufferwords` port already strips quotes while it
/// tokenises (see `Src/hist.c` `(z)` callout), so the split and the
/// subsequent `(Q)` dequote collapse into this single call — the words it
/// returns are already unquoted, which is exactly what `words=("${(@Q)words}")`
/// leaves behind.
fn rebuild_words(handler_output: &str) -> Vec<String> {
    bufferwords(handler_output, 0).0
}

/// sh:17 — `CURRENT = ${#words} - end_offset` after `$words` was rebuilt.
fn recompute_current(nwords: usize, end_offset: i64) -> i64 {
    nwords as i64 - end_offset
}

/// Spawn `zsh-mime-handler -l ARGS…` and capture its stdout. On spawn
/// failure returns an empty string — zsh `$(…)` command substitution
/// likewise degrades to empty output rather than aborting the function.
fn run_handler_l(rest: &[String]) -> String {
    match Command::new("zsh-mime-handler")
        .arg("-l")
        .args(rest)
        .output()
    {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(_) => String::new(),
    }
}

/// `_zsh-mime-handler` — re-expand the handler command line and dispatch
/// `_normal` on the resolved command.
pub fn _zsh_mime_handler(_args: &[String]) -> i32 {
    // sh:7  integer end_offset=$(( ${#words} - CURRENT ))
    let words = getaparam("words").unwrap_or_default();
    let current: i64 = getsparam("CURRENT")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let end_offset = compute_end_offset(words.len(), current);

    // sh:13  words=(${(z)"$(zsh-mime-handler -l "${(@)words[2,-1]}")"})
    //   ${(@)words[2,-1]} = every element past the first (drop `zsh-mime-handler`).
    let rest: Vec<String> = if words.len() > 1 {
        words[1..].to_vec()
    } else {
        Vec::new()
    };
    let output = run_handler_l(&rest);
    // sh:13 + sh:15 — (z) split then (@Q) unquote.
    let new_words = rebuild_words(&output);

    // sh:17  (( CURRENT = ${#words} - end_offset ))
    let new_current = recompute_current(new_words.len(), end_offset);
    setaparam("words", new_words);
    let _ = setsparam("CURRENT", &new_current.to_string());

    // sh:19  _normal
    normal_byname(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn end_offset_is_distance_from_end() {
        // words=(zsh-mime-handler foo bar), CURRENT=2 -> 3 - 2 = 1.
        assert_eq!(compute_end_offset(3, 2), 1);
        // cursor on last word -> offset 0.
        assert_eq!(compute_end_offset(3, 3), 0);
    }

    #[test]
    fn recompute_current_inverts_end_offset() {
        // If the handler rebuilds to 2 words and the end offset was 1,
        // CURRENT lands on the first of those two words: 2 - 1 = 1.
        assert_eq!(recompute_current(2, 1), 1);
        // offset preserved when word count is unchanged.
        let n = 4usize;
        let off = compute_end_offset(n, 3);
        assert_eq!(recompute_current(n, off), 3);
    }

    #[test]
    fn rebuild_words_splits_and_unquotes() {
        // Handler prints a fully-quoted command line; (z)+(Q) yields the
        // executable words with quoting removed.
        let out = "open '/tmp/my file.pdf'";
        assert_eq!(
            rebuild_words(out),
            vec!["open".to_string(), "/tmp/my file.pdf".to_string()]
        );
    }

    #[test]
    fn rebuild_words_plain_line() {
        assert_eq!(
            rebuild_words("xpdf report.pdf"),
            vec!["xpdf".to_string(), "report.pdf".to_string()]
        );
    }

    #[test]
    fn rebuild_words_empty_output() {
        assert!(rebuild_words("").is_empty());
    }

    #[test]
    fn drop_first_word_selects_the_arguments() {
        // Mirrors ${(@)words[2,-1]} — everything past `zsh-mime-handler`.
        let words = vec![
            "zsh-mime-handler".to_string(),
            "a".to_string(),
            "b".to_string(),
        ];
        let rest: Vec<String> = words[1..].to_vec();
        assert_eq!(rest, vec!["a".to_string(), "b".to_string()]);
    }
}
