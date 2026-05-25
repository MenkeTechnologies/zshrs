//! Port of `_globqual_delims` from `Completion/Zsh/Type/_globqual_delims`.
//!
//! Full upstream body (24 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # Helper for _globquals.  Sets delim to delimiter to match.
//! sh: 4
//! sh: 5  # don't restore special parameters
//! sh: 6  compstate[restore]=no
//! sh: 7
//! sh: 8  delim=$PREFIX[1]
//! sh: 9  compset -p 1
//! sh:10
//! sh:11  # One of matching brackets?
//! sh:12  # These don't actually work: the parser gets very confused.
//! sh:13  local matchl="<({[" matchr=">)}]"
//! sh:14  integer ind=${matchl[(I)$delim]}
//! sh:15
//! sh:16  (( ind )) && delim=$matchr[ind]
//! sh:17
//! sh:18  if compset -P "[^$delim]#$delim"; then
//! sh:19    # Completely matched.
//! sh:20    return 0
//! sh:21  else
//! sh:22    # Still in delimiter
//! sh:23    return 1
//! sh:24  fi
//! ```
//!
//! The fn picks the OPEN delim char (`<`, `(`, `{`, `[` map to their
//! matching close; anything else is the close itself), then checks
//! whether `PREFIX` already contains a closing delim somewhere.
//!
//! Strict Rust port: returns the (open, close, already_closed) tuple
//! based on inspecting state.params.prefix. Mutates `iprefix`/`prefix`
//! to mirror the shell's `compset -p 1` (chew the open delim).



use crate::compsys::compcore::CompletionState;

/// Outcome of `_globqual_delims`.
#[derive(Debug, PartialEq, Eq)]
pub struct GlobQualDelimsResult {
    /// The delimiter char the user opened with.
    pub open_delim: char,
    /// The matching close-delim char (`<` → `>`, etc., else == open).
    pub close_delim: char,
    /// True iff `PREFIX` already contains the closing delim
    /// somewhere — i.e. the qualifier sub-expression is closed and
    /// the caller can stop completing within it.
    pub already_closed: bool,
}

/// `_globqual_delims` — resolve the qualifier delimiter pair from
/// `state.params.prefix`. Returns None when prefix is empty.
pub fn _globqual_delims(state: &mut CompletionState) -> Option<GlobQualDelimsResult> {
    let prefix = state.params.prefix.clone();
    let open = prefix.chars().next()?;
    // shell `compset -p 1` — chew first char.
    state
        .params
        .iprefix
        .push_str(&prefix[..open.len_utf8()]);
    state.params.prefix = prefix[open.len_utf8()..].to_string();

    // Map open delim to close delim where applicable.
    let close = match open {
        '<' => '>',
        '(' => ')',
        '{' => '}',
        '[' => ']',
        c => c,
    };

    // shell `compset -P "[^$delim]#$delim"` — check if the close
    // delim appears in PREFIX (after consuming the open).
    let already_closed = state.params.prefix.contains(close);
    Some(GlobQualDelimsResult {
        open_delim: open,
        close_delim: close,
        already_closed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paren_open_resolves_to_close_paren() {
        let mut state = CompletionState::new();
        state.params.prefix = "(abc".into();
        let r = _globqual_delims(&mut state).unwrap();
        assert_eq!(r.open_delim, '(');
        assert_eq!(r.close_delim, ')');
        assert!(!r.already_closed);
    }

    #[test]
    fn bracket_open_resolves_to_close_bracket() {
        let mut state = CompletionState::new();
        state.params.prefix = "[abc".into();
        let r = _globqual_delims(&mut state).unwrap();
        assert_eq!(r.close_delim, ']');
    }

    #[test]
    fn slash_open_uses_slash_as_close_too() {
        let mut state = CompletionState::new();
        state.params.prefix = "/abc".into();
        let r = _globqual_delims(&mut state).unwrap();
        assert_eq!(r.open_delim, '/');
        assert_eq!(r.close_delim, '/');
    }

    #[test]
    fn closing_delim_in_prefix_marks_already_closed() {
        let mut state = CompletionState::new();
        state.params.prefix = "(abc)def".into();
        let r = _globqual_delims(&mut state).unwrap();
        assert!(r.already_closed);
    }

    #[test]
    fn prefix_chewed_into_iprefix() {
        let mut state = CompletionState::new();
        state.params.prefix = "(abc".into();
        let _ = _globqual_delims(&mut state);
        assert_eq!(state.params.iprefix, "(");
        assert_eq!(state.params.prefix, "abc");
    }

    #[test]
    fn empty_prefix_returns_none() {
        let mut state = CompletionState::new();
        assert!(_globqual_delims(&mut state).is_none());
    }

    #[test]
    fn angle_bracket_pair() {
        let mut state = CompletionState::new();
        state.params.prefix = "<rest".into();
        let r = _globqual_delims(&mut state).unwrap();
        assert_eq!(r.open_delim, '<');
        assert_eq!(r.close_delim, '>');
    }
}
