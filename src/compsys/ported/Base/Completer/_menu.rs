//! Port of `_menu` from `Completion/Base/Completer/_menu`.
//!
//! Full upstream body (23 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  [[ _matcher_num -gt 1 ]] && return 1
//! sh: 4
//! sh: 5  # This completer is an example showing how menu completion can be
//! sh: 6  # implemented with the new completion system.
//! sh: 7  # Use this one before the normal _complete completer, as in:
//! sh: 8  #
//! sh: 9  #   zstyle ":completion:::::" completer _menu _complete
//! sh:10
//! sh:11  if [[ -n "$compstate[old_list]" ]]; then
//! sh:12
//! sh:13    # We have an old list, keep it and insert the next match.
//! sh:14
//! sh:15    compstate[old_list]=keep
//! sh:16    compstate[insert]=$((compstate[old_insert]+1))
//! sh:17  else
//! sh:18    # No old list, make completion insert the first match.
//! sh:19
//! sh:20    compstate[insert]=1
//! sh:21  fi
//! sh:22
//! sh:23  return 1
//! ```
//!
//! `$compstate` is a PM_HASHED special parameter created by
//! `makecompparams()` in `src/ported/zle/complete.rs:1492`. Real
//! hash-subscript access goes through
//! `crate::ported::zle::compcore::{get_compstate_str, set_compstate_str}`
//! — both back onto the shared `paramtab_hashed_storage()` table
//! (`src/ported/params.rs:1913`) where PM_HASHED param values actually
//! live. The shell function uses no `local` declarations, so neither
//! does the port.

use crate::ported::params::getiparam;
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};

/// `_menu` — menu-completion completer. Returns the shell's `return 1`
/// to defer match emission to the next completer in the chain.
pub fn _menu() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_menu");
    // sh:3  [[ _matcher_num -gt 1 ]] && return 1
    //   `$_matcher_num` is a top-level shell variable (NOT a
    //   `$compstate` sub-key); > 1 means we're not the first matcher
    //   pass — defer.
    if getiparam("_matcher_num") > 1 {
        return 1;
    }

    // sh:11  if [[ -n "$compstate[old_list]" ]]; then
    let old_list = get_compstate_str("old_list").unwrap_or_default();
    if !old_list.is_empty() {
        // sh:15  compstate[old_list]=keep
        set_compstate_str("old_list", "keep");
        // sh:16  compstate[insert]=$((compstate[old_insert]+1))
        //   compstate values are strings even when numeric in
        //   contexts; parse-then-increment-then-stringify.
        let old_insert: i64 = get_compstate_str("old_insert")
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        set_compstate_str("insert", &(old_insert + 1).to_string());
    } else {
        // sh:20  compstate[insert]=1
        set_compstate_str("insert", "1");
    }

    // sh:23  return 1
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::setiparam;

    /// Wipe any leftover `$compstate` entries between tests by clearing
    /// the keys this completer touches. The hashed-storage backing for
    /// `compstate` is shared global state, so each test must reset
    /// inputs it cares about.
    fn reset_compstate_keys() {
        set_compstate_str("old_list", "");
        set_compstate_str("insert", "");
        set_compstate_str("old_insert", "");
    }

    #[test]
    fn defers_when_matcher_num_gt_one() {
        // sh:3 — if `_matcher_num > 1`, return 1 immediately without
        // touching `$compstate`.
        let _g = crate::test_util::global_state_lock();
        reset_compstate_keys();
        setiparam("_matcher_num", 5);
        set_compstate_str("old_list", "preserved");
        let r = _menu();
        assert_eq!(r, 1);
        assert_eq!(get_compstate_str("old_list").as_deref(), Some("preserved"));
        // Cleanup.
        reset_compstate_keys();
        setiparam("_matcher_num", 0);
    }

    #[test]
    fn no_old_list_sets_insert_to_one() {
        // sh:17-20 — `_matcher_num <= 1` AND `old_list` empty → set
        // `insert=1`.
        let _g = crate::test_util::global_state_lock();
        reset_compstate_keys();
        setiparam("_matcher_num", 1);
        let r = _menu();
        assert_eq!(r, 1);
        assert_eq!(get_compstate_str("insert").as_deref(), Some("1"));
        reset_compstate_keys();
    }

    #[test]
    fn with_old_list_keeps_and_advances_insert() {
        // sh:11-16 — `old_list` non-empty → set to "keep" and
        // advance `insert` past `old_insert`.
        let _g = crate::test_util::global_state_lock();
        reset_compstate_keys();
        setiparam("_matcher_num", 1);
        set_compstate_str("old_list", "yes");
        set_compstate_str("old_insert", "7");
        let r = _menu();
        assert_eq!(r, 1);
        assert_eq!(get_compstate_str("old_list").as_deref(), Some("keep"));
        assert_eq!(
            get_compstate_str("insert").as_deref(),
            Some("8"),
            "sh:16 insert = old_insert + 1"
        );
        reset_compstate_keys();
    }

    #[test]
    fn old_insert_zero_yields_insert_one() {
        // Edge case: `old_insert == 0` → `insert = 0 + 1 = 1`.
        let _g = crate::test_util::global_state_lock();
        reset_compstate_keys();
        setiparam("_matcher_num", 1);
        set_compstate_str("old_list", "yes");
        set_compstate_str("old_insert", "0");
        let r = _menu();
        assert_eq!(r, 1);
        assert_eq!(get_compstate_str("insert").as_deref(), Some("1"));
        reset_compstate_keys();
    }

    #[test]
    fn old_insert_missing_yields_insert_one() {
        // Edge case: `old_insert` never set → parse returns None →
        // default 0 → `insert = 1`.
        let _g = crate::test_util::global_state_lock();
        reset_compstate_keys();
        setiparam("_matcher_num", 1);
        set_compstate_str("old_list", "yes");
        // old_insert intentionally NOT set.
        let r = _menu();
        assert_eq!(r, 1);
        assert_eq!(get_compstate_str("insert").as_deref(), Some("1"));
        reset_compstate_keys();
    }
}
