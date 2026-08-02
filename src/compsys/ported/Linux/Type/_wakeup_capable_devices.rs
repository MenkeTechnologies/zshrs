//! Port of `_wakeup_capable_devices` from
//! `Completion/Linux/Type/_wakeup_capable_devices`.
//!
//! Full upstream body (16 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local ret=1 item devline expl
//! sh: 4  typeset -a desc
//! sh: 6  _call_program wakeup-capable-devices acpitool -w 2> /dev/null |
//! sh: 7  while read devline; do
//! sh: 8    [[ -n ${devline:#(#b)([0-9]#).[[:space:]]#([^[:space:]]#)[[:space:]]#[0-9]#[[:space:]]#(*)} ]] && continue
//! sh: 9    zformat -f item "${match[1]}:%8d (currently ${match[3]})" d:${match[2]}
//! sh:10    desc+=$item
//! sh:11  done
//! sh:13  _describe -t wakeup-capable-devices 'wakeup capable device' desc "$@" && ret=0
//! sh:15  return ret
//! ```

use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_describe::_describe;
use crate::ported::modules::zutil::zformat_substring;
use crate::ported::params::{getsparam, setaparam};
use regex::Regex;
use std::collections::HashMap;
use std::sync::OnceLock;

/// sh:8 — `${devline:#(#b)([0-9]#).[[:space:]]#([^[:space:]]#)[[:space:]]#[0-9]#[[:space:]]#(*)}`
///
/// zsh glob translation (`#` = zero-or-more, `.` = literal dot, `?` would be
/// "any char" but is not used here):
///   group1 `[0-9]#`          -> `([0-9]*)`   device index
///   literal `.`              -> `\.`
///   `[[:space:]]#`           -> `[[:space:]]*`
///   group2 `[^[:space:]]#`   -> `([^[:space:]]*)`  device name
///   `[[:space:]]#`           -> `[[:space:]]*`
///   `[0-9]#` (uncaptured)    -> `[0-9]*`
///   `[[:space:]]#`           -> `[[:space:]]*`
///   group3 `*`               -> `(.*)`       rest of line
///
/// `${var:#pattern}` requires a *full* match of `pattern` against `var`
/// (anchored both ends), so the translated regex is anchored `^...$`.
fn line_pattern() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^([0-9]*)\.[[:space:]]*([^[:space:]]*)[[:space:]]*[0-9]*[[:space:]]*(.*)$")
            .expect("valid _wakeup_capable_devices line regex")
    })
}

/// sh:8-9 — parse one `devline`; `None` when the line doesn't match (the
/// shell's `continue`), `Some(item)` when it does (the zformat'd entry).
fn parse_devline(devline: &str) -> Option<String> {
    let caps = line_pattern().captures(devline)?; // sh:8 `continue` on no match
    let match1 = &caps[1]; // device index
    let match2 = &caps[2]; // device name
    let match3 = &caps[3]; // trailing status text

    // sh:9  zformat -f item "${match[1]}:%8d (currently ${match[3]})" d:${match[2]}
    //   The `${match[1]}`/`${match[3]}` pieces are shell-expanded into the
    //   format string *before* zformat runs; only `%8d` is a live zformat
    //   substitution (spec 'd' -> match[2], min-width 8, left-justified
    //   with trailing-space padding — no `-` flag was given).
    let format = format!("{}:%8d (currently {})", match1, match3);
    let mut specs: HashMap<char, String> = HashMap::new();
    specs.insert('d', match2.to_string());
    Some(zformat_substring(&format, &specs, false))
}

/// `_wakeup_capable_devices` — offer devices reported by `acpitool -w` as
/// wakeup-capable, annotated with their current wakeup status.
pub fn _wakeup_capable_devices(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_wakeup_capable_devices");
    let mut ret = 1; // sh:3

    // sh:6  _call_program wakeup-capable-devices acpitool -w 2> /dev/null
    let mut desc: Vec<String> = Vec::new();
    if _call_program(&[
        "wakeup-capable-devices".to_string(),
        "acpitool -w".to_string(),
    ]) == 0
    {
        let output = getsparam("REPLY").unwrap_or_default();
        // sh:7  while read devline; do ... done
        for devline in output.lines() {
            if let Some(item) = parse_devline(devline) {
                desc.push(item); // sh:10
            }
            // sh:8 non-match -> `continue`, i.e. skip pushing.
        }
    }

    // sh:4  typeset -a desc — publish the array so `_describe` (mirroring
    //   the shell's `resolve_array_arg`-by-name convention) can look it up.
    setaparam("desc", desc);

    // sh:13  _describe -t wakeup-capable-devices 'wakeup capable device' desc "$@" && ret=0
    let mut describe_argv: Vec<String> = vec![
        "-t".to_string(),
        "wakeup-capable-devices".to_string(),
        "wakeup capable device".to_string(),
        "desc".to_string(),
    ];
    describe_argv.extend(args.iter().cloned());
    if _describe(&describe_argv) == 0 {
        ret = 0;
    }

    ret // sh:15
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_devline_matches_and_formats() {
        // "0. wlan0    1    enabled" — index=0, name=wlan0, trailing=enabled.
        // sh:8 pattern `([0-9]#).` requires the literal `.` (zsh glob `.` is
        // a literal char, not "any char") after the index, matching acpitool's
        // "N. NAME ..." output; without it the line fails to match.
        let item = parse_devline("0. wlan0    1    enabled");
        assert!(item.is_some());
        let item = item.unwrap();
        assert!(item.starts_with("0:"));
        assert!(item.contains("wlan0"));
        assert!(item.ends_with("(currently enabled)"));
    }

    #[test]
    fn parse_devline_pads_device_name_to_width_eight() {
        // sh:9 `%8d` (no `-` flag) left-justifies the device-name spec,
        // padding with trailing spaces to a minimum field width of 8.
        // sh:8 requires the literal `.` after the index (see above); "3. x …".
        let item = parse_devline("3. x    0    disabled").unwrap();
        // "3:" + 8-wide field "x       " + " (currently disabled)"
        assert!(item.starts_with("3:"));
        let after_colon = &item["3:".len()..];
        let field_and_rest: Vec<&str> = after_colon.splitn(2, " (currently").collect();
        assert_eq!(field_and_rest[0], "x       ");
        assert_eq!(field_and_rest[0].len(), 8);
    }

    #[test]
    fn parse_devline_no_index_still_matches_since_digits_are_optional() {
        // `[0-9]#` (zero-or-more) means an empty index is a valid match.
        let item = parse_devline(".    eth0    enabled");
        assert!(item.is_some());
    }

    #[test]
    fn returns_one_when_call_program_fails() {
        // sh:6 `_call_program … 2>/dev/null` failing (e.g. acpitool absent)
        // leaves `desc` empty; `_describe` on an empty array fails, so
        // `ret` stays at its sh:3 initial value of 1.
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(_wakeup_capable_devices(&[]), 1);
    }
}
