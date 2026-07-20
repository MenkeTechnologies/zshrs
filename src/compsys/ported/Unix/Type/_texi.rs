//! Port of `_texi` from `Completion/Unix/Type/_texi`.
//!
//! Full upstream body (6 lines verbatim):
//! ```text
//! sh:1  #compdef -P (texi(2*|ndex))
//! sh:2
//! sh:3  local expl
//! sh:4
//! sh:5  _description files expl 'texinfo file'
//! sh:6  _files "$@" "$expl[@]" -g '*.(texinfo|texi)(-.)'
//! ```

use crate::compsys::ported::_description::_description;
use crate::compsys::ported::_files::_files;
use crate::ported::params::getaparam;

/// `_texi` — complete texinfo source files (`*.texinfo` / `*.texi`).
pub fn _texi(args: &[String]) -> i32 {
    // sh:5
    let _ = _description(&[
        "files".to_string(),
        "expl".to_string(),
        "texinfo file".to_string(),
    ]);
    // sh:6  _files "$@" "$expl[@]" -g '*.(texinfo|texi)(-.)'
    let mut a: Vec<String> = args.to_vec();
    a.extend(getaparam("expl").unwrap_or_default());
    a.push("-g".to_string());
    a.push("*.(texinfo|texi)(-.)".to_string());
    _files(&a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_texi(&[]), 1);
    }
}
