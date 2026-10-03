//! Port of `_suffix_alias_files` from
//! `Completion/Zsh/Type/_suffix_alias_files`.
//!
//! Full upstream body (zsh-5.9.1 tag, 22 lines):
//! ```text
//! sh: 1  #autoload
//! sh: 3  # Complete files for which a suffix alias exists.
//! sh: 5  local expl pat
//! sh: 7  (( ${#saliases} )) || return 1
//! sh: 9  if (( ${#saliases} == 1 )); then
//! sh:10      pat="*.${(kq)saliases}"
//! sh:11  else
//! sh:12      local -a tmpa
//! sh:13      # This is so we can quote the alias names against expansion
//! sh:14      # without quoting the `|' which needs to be active in the pattern
//! sh:15      # --- remember that an alias name can be pretty much anything.
//! sh:16      tmpa=(${(kq)saliases})
//! sh:17      pat="*.(${(kj.|.)tmpa})"
//! sh:18  fi
//! sh:19
//! sh:20  # _wanted is called for us by _command_names
//! sh:21  _path_files "$@" -g $pat
//! ```
//!
//! The `[[ -o autocd ]] || pat+='(#q^/)'` line is zsh workers/50307,
//! which is on the dev branch only; zsh 5.9.1/5.9.2 do not have it.

use crate::compsys::ported::shared::dispatch_action_command;
use crate::ported::params::getaparam;
use crate::ported::utils::quotestring;
use crate::ported::zsh_h::QT_BACKSLASH;

/// `_suffix_alias_files` — complete file paths that match a
/// suffix-alias suffix.
pub fn _suffix_alias_files(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_suffix_alias_files");
    // sh:7,10,16 read the KEYS of `saliases` (`${#saliases}`,
    // `${(kq)saliases}`). For the magic assoc that is gethkparam, C's
    // `paramvalarr(..., SCANPM_WANTKEYS)` (c:Src/params.c:3131-3140), which
    // also materializes the autoload stub (c:Src/params.c:589-594). If a
    // caller's scope hides the special behind a plain array, `(k)` on an
    // array yields its elements (zsh 5.9.2: `a=(x y); echo ${(k)a}` prints
    // `x y`), so every element is a key.
    let keys: Vec<String> = crate::ported::params::gethkparam("saliases")
        .or_else(|| getaparam("saliases"))
        .unwrap_or_default();

    // sh:7
    if keys.is_empty() {
        return 1;
    }

    // sh:10 / sh:16 — the `(q)` flag: backslash-quote each alias name so a
    // pattern character in it matches literally.
    let quoted: Vec<String> = keys.iter().map(|k| quotestring(k, QT_BACKSLASH)).collect();

    // sh:9-18
    let pat = if quoted.len() == 1 {
        format!("*.{}", quoted[0]) // sh:10
    } else {
        format!("*.({})", quoted.join("|")) // sh:17
    };

    // sh:21
    let mut argv: Vec<String> = args.to_vec();
    argv.push("-g".to_string());
    argv.push(pat);
    // sh:21 is a COMMAND WORD, so `dispatch_action_command`
    // (shared.rs:1407) resolves it exactly as `execcmd` does:
    // shfunc/port/plugin (c:Src/exec.c:3105-3109), then builtin, then
    // `$PATH`, then c:903's `command not found` with c:908's 127.
    dispatch_action_command("_path_files", &argv, 21)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::setaparam;

    /// sh:7 — `(( ${#saliases} )) || return 1`. Reference zsh 5.9.2 with no
    /// suffix alias defined returns 1 from a `zle -C` widget body, before
    /// `_path_files` is ever reached. `$saliases` reads `sufaliastab`
    /// (c:Src/Modules/parameter.c scanpmsaliases), so the table itself is
    /// emptied for the call and restored after.
    #[test]
    fn no_saliases_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let tab = crate::ported::hashtable::sufaliastab_lock();
        let saved = tab.read().expect("sufaliastab poisoned").snapshot();
        tab.write()
            .expect("sufaliastab poisoned")
            .restore(crate::ported::hashtable::alias_table::new());
        let r = _suffix_alias_files(&[]);
        tab.write().expect("sufaliastab poisoned").restore(saved);
        assert_eq!(r, 1);
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
