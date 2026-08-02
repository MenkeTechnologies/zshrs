//! Port of `_suffix_alias_files` from
//! `Completion/Zsh/Type/_suffix_alias_files`.
//!
//! Full upstream body (22 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  # Complete files for which a suffix alias exists.
//! sh: 5  local expl pat
//! sh: 7  (( ${#saliases} )) || return 1
//! sh: 9  if (( ${#saliases} == 1 )); then
//! sh:10      pat="*.${(kq)saliases}"
//! sh:11  else
//! sh:12      local -a tmpa
//! sh:13      tmpa=(${(kq)saliases})
//! sh:14      pat="*.(${(kj.|.)tmpa})"
//! sh:15  fi
//! sh:16  [[ -o autocd ]] || pat+='(#q^/)'
//! sh:18  # _wanted is called for us by _command_names
//! sh:19  _path_files "$@" -g $pat
//! ```
//!
//! `saliases` is the shell-side suffix-alias assoc array. Keys are
//! the suffixes (`.gz`, `.tar`, etc.). When AUTOCD is off, append
//! the `(#q^/)` glob qualifier to exclude directories.

use crate::ported::exec::dispatch_function_call;
use crate::ported::params::getaparam;
use crate::ported::zsh_h::{isset, AUTOCD};

/// `_suffix_alias_files` — complete file paths that match a
/// suffix-alias suffix.
pub fn _suffix_alias_files(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_suffix_alias_files");
    // saliases is stored as a flat array of key/value pairs in our
    //   port. Extract every other element (the keys).
    let saliases = getaparam("saliases").unwrap_or_default();
    let keys: Vec<&String> = saliases.iter().step_by(2).collect();

    // sh:7
    if keys.is_empty() {
        return 1;
    }

    // sh:9-15
    let pat = if keys.len() == 1 {
        format!("*.{}", keys[0])
    } else {
        let joined = keys
            .iter()
            .map(|k| k.as_str())
            .collect::<Vec<_>>()
            .join("|");
        format!("*.({})", joined)
    };

    // sh:16
    let pat = if isset(AUTOCD) {
        pat
    } else {
        format!("{}(#q^/)", pat)
    };

    // sh:19
    let mut argv: Vec<String> = args.to_vec();
    argv.push("-g".to_string());
    argv.push(pat);
    dispatch_function_call("_path_files", &argv).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::setaparam;

    #[test]
    fn no_saliases_returns_one() {
        let _g = crate::test_util::global_state_lock();
        setaparam("saliases", Vec::new());
        assert_eq!(_suffix_alias_files(&[]), 1);
    }

    #[test]
    fn populates_glob_pattern_from_saliases_keys() {
        // Verify the pattern construction logic by setting a fake
        //   saliases (key/value pairs) and ensuring the function
        //   reaches the dispatch step without panic.
        let _g = crate::test_util::global_state_lock();
        setaparam(
            "saliases",
            vec![
                "gz".to_string(),
                "gunzip".to_string(),
                "tar".to_string(),
                "tar".to_string(),
            ],
        );
        let _ = _suffix_alias_files(&[]);
    }
}
