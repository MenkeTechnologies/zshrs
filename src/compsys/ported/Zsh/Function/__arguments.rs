//! Port of `__arguments` from `Completion/Zsh/Function/__arguments`.
//!
//! `__arguments` is the completion function *for* the utility function
//! `_arguments` — it fires on `_arguments -<TAB>` at the prompt to jog the
//! user's memory about `_arguments`' own option flags. It is NOT `_arguments`
//! itself; it merely delegates to the (already-ported) `_arguments` with a
//! hand-written spec list describing `_arguments`' flags.
//!
//! Full upstream body (45 lines, abridged — head is a long usage comment):
//! ```text
//! sh: 1  #compdef _arguments
//! sh:21  if (( ${words[(i)--]} < CURRENT )); then
//! sh:23    _arguments : \                       # "Deriving spec forms from --help"
//! sh:24      '*-i[…]:option name exclude pattern' \
//! sh:25      '*-s[…]:pattern and replacement as "(this that)"' \
//! sh:26      '*:helpspec (pattern\:message\:action)'
//! sh:27  else
//! sh:28    _arguments -A '-([AMO]*|[0CRSWnsw])' : \
//! sh:29      '!-n[set $NORMARG]' \
//! sh:30      '-s[…]' '-w[…]' '-W[…]' \
//! sh:33      "-C[…]" "-R[…]" "-S[…]" "-A[…]" \
//! sh:37      '-O[…]:array variable name:_parameters -g array' \
//! sh:38      "-M[…]" '-0[…]' "--[…]" \
//! sh:41      '1::optional delimiter:(\:)' \
//! sh:42      '*:spec (…)'
//! sh:44  fi
//! ```

use crate::compsys::ported::_arguments::_arguments;
use crate::ported::params::{getaparam, getiparam};

/// sh:23-26 — spec list for the `--help`-derivation branch (a `--` already
/// precedes the cursor). The leading `:` separates `_arguments` options from
/// specs.
fn help_branch_specs() -> Vec<String> {
    [
        ":",
        "*-i[specify option name exclude patterns]:option name exclude pattern",
        "*-s[specify option aliases]:pattern and replacement as \"(this that)\"",
        "*:helpspec (pattern\\:message\\:action)",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// sh:28-42 — spec list for the normal branch describing every `_arguments`
/// flag. `-A '-([AMO]*|[0CRSWnsw])'` marks the option-guard pattern; the
/// leading `:` separates options from specs.
fn flag_branch_specs() -> Vec<String> {
    [
        "-A",
        "-([AMO]*|[0CRSWnsw])",
        ":",
        "!-n[set $NORMARG]",
        "-s[enable single-letter option stacking (-x -y == -xy)]",
        "-w[(rarely needed) enable single-letter option stacking with arguments (-x X -y == -xy X)]",
        "-W[(rarely needed) enable single-letter option stacking with arguments in the same word (-x X -y == -xXy)]",
        "-C[modify $curcontext for `->action' (instead of $context)]",
        "-R[when `->action' matches, return 300]",
        "-S[honour `--' as end-of-options guard]",
        "-A[do not complete options after non-options]:pattern matching unknown options (e.g., '-*')",
        "-O[pass elements of array variable to function calls in actions]:array variable name:_parameters -g array",
        "-M[specify matchspec for completing option names and values]:matchspec for completing option names and values [ 'r\\:|[_-]=* r\\:|=*' ]",
        "-0[have ${(v)opt_args} be NUL-joined rather than colon-escaped and colon-joined]",
        "--[derive optspecs from `${command} --help' output]",
        "1::optional delimiter:(\\:)",
        "*:spec (e.g., \"(-t --to)\"*{-t+,--to=}\"[specify recipient]\\:recipient's address\\:_email_addresses)",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// sh:21 — `(( ${words[(i)--]} < CURRENT ))`: does a `--` word appear before
/// the cursor position? `${words[(i)--]}` is the 1-based index of the first
/// `--` in `$words`, or `${#words}+1` when absent.
fn dashdash_before_current(words: &[String], current: i64) -> bool {
    let idx = match words.iter().position(|w| w == "--") {
        Some(p) => (p + 1) as i64, // 1-based match position
        None => words.len() as i64 + 1,
    };
    idx < current
}

/// `__arguments` — completion for `_arguments`' own flags. Takes no arguments
/// (sh:19); dispatches to `_arguments` with one of two hard-coded spec lists.
pub fn __arguments(_args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("__arguments");
    let words = getaparam("words").unwrap_or_default();
    let current = getiparam("CURRENT");

    if dashdash_before_current(&words, current) {
        // sh:23
        _arguments(&help_branch_specs())
    } else {
        // sh:28  (TODO upstream: no support for multiple argument sets)
        _arguments(&flag_branch_specs())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashdash_detection_matches_zsh_subscript_semantics() {
        // No `--` → index = len+1, never < current.
        let w = vec!["_arguments".to_string(), "-".to_string()];
        assert!(!dashdash_before_current(&w, 2));

        // `--` at position 2 (1-based), cursor at 3 → precedes cursor.
        let w = vec!["_arguments".to_string(), "--".to_string(), "".to_string()];
        assert!(dashdash_before_current(&w, 3));

        // `--` at position 2, cursor at 2 → not strictly before (2 < 2 false).
        assert!(!dashdash_before_current(&w, 2));
    }

    #[test]
    fn help_branch_specs_lead_with_separator_and_catch_all() {
        let s = help_branch_specs();
        assert_eq!(s[0], ":");
        assert_eq!(s.last().unwrap(), "*:helpspec (pattern\\:message\\:action)");
        assert!(s.iter().any(|x| x.starts_with("*-i[")));
        assert!(s.iter().any(|x| x.starts_with("*-s[")));
    }

    #[test]
    fn flag_branch_specs_carry_option_guard_and_all_flags() {
        let s = flag_branch_specs();
        // sh:28 — the -A option guard pattern precedes the `:` separator.
        assert_eq!(s[0], "-A");
        assert_eq!(s[1], "-([AMO]*|[0CRSWnsw])");
        assert_eq!(s[2], ":");
        // Every documented flag spec is present (leading-char check).
        for lead in [
            "!-n[", "-s[", "-w[", "-W[", "-C[", "-R[", "-S[", "-A[", "-O[", "-M[", "-0[", "--[",
        ] {
            assert!(s.iter().any(|x| x.starts_with(lead)), "missing {lead}");
        }
        // Escaped colons in the final positional catch-all are preserved.
        assert!(s
            .last()
            .unwrap()
            .contains("\\:recipient's address\\:_email_addresses"));
    }
}
