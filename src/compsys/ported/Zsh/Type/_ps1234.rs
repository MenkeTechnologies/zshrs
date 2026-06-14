//! Port of `_ps1234` from `Completion/Zsh/Type/_ps1234`.
//!
//! Full upstream body (179 lines, abridged):
//! ```text
//! sh: 1  #compdef -value-,PROMPT,-default- -value-,PROMPT2,-default- … (many prompt vars)
//! sh:10  while changed; compset -P escape-strip loops to find current incomplete %x
//! sh:30  if compset -P '%[FK]'; then  → ansi-color / terminal-color completion
//! sh:80  if compset -P '%[0-9-\\]#(\\|)\([0-9-]#[^0-9]'; then → ternary delimiter
//! sh:130  format-spec catalogs (D/d/g/i/j/L/l/S/T/t/v/V/w etc.)
//! sh:178  return ret
//! ```
//!
//! `$PROMPT` / `$PS1`-style prompt-escape completer. Skipped:
//! the complex escape-strip walk (sh:10-29) and color subcommands.
//! Emits the common `%X` format-spec catalog as a fallback.

use crate::ported::exec::dispatch_function_call;
use crate::ported::params::setaparam;

const PROMPT_SPECS: &[&str] = &[
    "%:literal % sign",
    "n:user name",
    "M:machine hostname (full)",
    "m:short hostname",
    "d:current working directory",
    "~:CWD with ~ for home",
    "h:current history line number",
    "!:history line number (alias for %h)",
    "?:exit status of last command",
    "_:status of current parser construct",
    "#:'#' for root, '%' otherwise",
    "(::ternary condition",
    "j:number of jobs",
    "L:$SHLVL",
    "D:date in yy-mm-dd",
    "T:current time (24h)",
    "t:current time (12h am/pm)",
    "*:current time (24h hms)",
    "w:day-mm name short",
    "W:month/day/yy",
    "i:current line number being executed",
    "I:current line number plus filename",
    "N:current $0",
    "x:current source filename",
    "c:trailing CWD components",
    "C:trailing CWD components (alias)",
    "/:full CWD",
    "S:start standout mode",
    "s:end standout mode",
    "U:start underline",
    "u:end underline",
    "B:start bold",
    "b:end bold",
    "F:foreground color",
    "f:end foreground",
    "K:background color",
    "k:end background",
    "{:literal-text bracket start",
    "}:literal-text bracket end",
    "G:advance by extended char width",
    "E:erase to end of display",
];

/// `_ps1234` — complete a `%X` prompt-escape spec.
pub fn _ps1234() -> i32 {
    let s: Vec<String> = PROMPT_SPECS.iter().map(|x| x.to_string()).collect();
    setaparam("ps_specs", s);
    dispatch_function_call(
        "_describe",
        &[
            "-t".to_string(),
            "prompt-escapes".to_string(),
            "prompt escape".to_string(),
            "ps_specs".to_string(),
            "-Q".to_string(),
            "-p".to_string(),
            "%".to_string(),
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
        assert_eq!(_ps1234(), 1);
    }

    #[test]
    fn publishes_ps_specs_array() {
        let _g = crate::test_util::global_state_lock();
        let _ = _ps1234();
        let s = crate::ported::params::getaparam("ps_specs").unwrap_or_default();
        assert!(s.len() >= 30);
    }
}
