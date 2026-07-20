//! Port of `_date_formats` from `Completion/Unix/Type/_date_formats`.
//!
//! Full upstream body (112 lines, abridged):
//! ```text
//! sh:  1  #autoload
//! sh:  3  local flag ret=1; local -aU specs; local -A exclusion
//! sh:  6  exclusion=( E '[cCgGxXyY]' O '[BdeHImMSuUVwWy]' - '[OEdegHIjklmMSUz]'
//! sh:         _ '[OEdgHIjmMSUz]' 0 '[Oekl]' ^ '[OEaAbBchP]' '#' '[OEaAbBchpPrXZ]' )
//! sh: 15  compset -P '(%[0-9EO_\^#-]#[^0-9%EO_\^#-]|[^%])#'
//! sh: 16  compset -S '%*'
//! sh: 17  specs=( 'a:abbreviated day name' … '%:literal %' )   # ~45 base entries
//! sh: 60  case $OSTYPE in … esac                                # per-OS additions
//! sh: 92  if [[ $1 == zsh ]]; then specs+=( 'f:…' 'K:…' … ); fi  # zsh strftime extras
//! sh:101  for flag in ${(s..)PREFIX#%}; do
//! sh:102    (( $+exclusion[$flag] )) && specs=( ${(M)specs:#${~exclusion[$flag]}:*} )
//! sh:103  done
//! sh:105  _describe -t date-format-specifier 'date format specifier' specs \
//! sh:106      -p "${(Q)PREFIX:-%}" -S '' && ret=0
//! sh:107  [[ $1 == zsh ]] && _message -e date-format-precision 'precision for %. (1-9)'
//! sh:109  return ret
//! ```
//!
//! sh:15-16 approx — the `compset -P/-S` glob strips are dispatched to the
//! real `bin_compset`. sh:60 uses `$OSTYPE` (via the `OSTYPE` parameter).

use crate::compsys::ported::_describe::_describe;
use crate::compsys::ported::_message::_message;
use crate::ported::params::getsparam;
use crate::ported::zle::complete::bin_compset;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}
fn compset(argv: &[&str]) -> i32 {
    let v: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    bin_compset("compset", &v, &make_ops(), 0)
}

/// sh:17 — the portable base specifier table (`char:description`).
const BASE: &[&str] = &[
    "a:abbreviated day name",
    "A:full day name",
    "b:abbreviated month name",
    "h:abbreviated month name",
    "B:full month name",
    "c:preferred locale date and time",
    "C:2-digit century",
    "d:day of month (01-31)",
    "D:American format month/day/year (%m/%d/%y)",
    "e:day of month ( 1-31)",
    "F:ISO 8601 year-month-date (%Y-%m-%d)",
    "G:4-digit ISO 8601 week-based year",
    "g:2-digit ISO 8601 week-based year",
    "H:hour (00-23)",
    "I:hour (01-12)",
    "j:day of year (001-366)",
    "k:hour ( 0-23)",
    "l:hour ( 1-12)",
    "m:month (01-12)",
    "M:minute (00-59)",
    "n:newline",
    "p:locale dependent AM/PM",
    "r:locale dependent a.m. or p.m. time (%I:%M:%S %p)",
    "R:24-hour notation time (%H:%M)",
    "s:seconds since the epoch",
    "S:seconds (00-60)",
    "t:tab",
    "T:24-hour notation with seconds (%H:%M:%S)",
    "u:day of week (1-7, 1=Monday)",
    "U:week number of current year, Sunday based (00-53)",
    "V:ISO 8601 week number of current year, week 1 has 4 days in current year (01-53)",
    "w:day of week (0-6, 0=Sunday)",
    "W:week number of current year, Monday based (00-53)",
    "x:locale dependent date representation without time",
    "X:locale dependent time representation without date",
    "y:2-digit year (00-99)",
    "Y:full year",
    "z:UTC offset",
    "Z:timezone name",
    "%:literal %",
];

/// sh:6 — flag → char-class of specifiers to KEEP once that flag was typed.
fn exclusion_class(flag: char) -> Option<&'static str> {
    Some(match flag {
        'E' => "cCgGxXyY",
        'O' => "BdeHImMSuUVwWy",
        '-' => "OEdegHIjklmMSUz",
        '_' => "OEdgHIjmMSUz",
        '0' => "Oekl",
        '^' => "OEaAbBchP",
        '#' => "OEaAbBchpPrXZ",
        _ => return None,
    })
}

/// `_date_formats [zsh]` — complete strftime-style format specifiers.
pub fn _date_formats(args: &[String]) -> i32 {
    let is_zsh = args.first().map(|s| s.as_str()) == Some("zsh");
    let ostype = getsparam("OSTYPE").unwrap_or_default();

    // sh:15-16
    let _ = compset(&["-P", r"(%[0-9EO_\^#-]#[^0-9%EO_\^#-]|[^%])#"]);
    let _ = compset(&["-S", "%*"]);

    // sh:17 — base table.
    let mut specs: Vec<String> = BASE.iter().map(|s| s.to_string()).collect();

    // sh:60 — per-OS additions (matched against $OSTYPE).
    let is = |p: &str| ostype.starts_with(p);
    let solaris_ge11 = ostype
        .strip_prefix("solaris2.")
        .and_then(|v| v.split('.').next())
        .and_then(|n| n.parse::<u32>().ok())
        .map(|n| n >= 11)
        .unwrap_or(false);
    if is("linux-gnu") && !is_zsh {
        specs.push("N:fractional part of seconds since epoch, in nanoseconds".to_string());
    }
    if is("freebsd") || is("dragonfly") || is("darwin") || is("linux-gnu") || solaris_ge11 {
        specs.extend(
            [
                "E:alternate representation",
                "O:alternative format modifier",
                "-:don't pad numeric values",
                "0:left pad numeric values with zeroes",
                "_:left pad numeric values with spaces",
            ]
            .iter()
            .map(|s| s.to_string()),
        );
    }
    if is("linux-gnu") || solaris_ge11 {
        specs.extend(
            [
                "#:swap case of alphabetic characters",
                "^:convert lowercase characters to uppercase",
                "P:lower case locale dependent am/pm",
            ]
            .iter()
            .map(|s| s.to_string()),
        );
    }
    if is("freebsd") || is("dragonfly") || is("darwin") || is("openbsd") || is("netbsd") {
        specs.push("v:date in short form (%e-%b-%Y)".to_string());
    }
    if solaris_ge11 || is("freebsd") || is("dragonfly") || is("darwin") || is("openbsd") {
        specs.push("+:localized representation of date and time".to_string());
    }

    // sh:92 — zsh's own strftime extras.
    if is_zsh {
        specs.extend(
            [
                "f:day of month (1-31)",
                "K:hour (0-23)",
                "L:hour (0-12)",
                "N:fractional part of seconds since epoch, in nanoseconds (%9.)",
                ".:fractional part of seconds since epoch",
                "-:don't pad numeric values",
            ]
            .iter()
            .map(|s| s.to_string()),
        );
    }

    // -aU dedup (sh:3) — keep first occurrence of each entry.
    let mut seen = std::collections::HashSet::new();
    specs.retain(|s| seen.insert(s.clone()));

    // sh:101-103 — narrow by already-typed modifier flags.
    let prefix = getsparam("PREFIX").unwrap_or_default();
    for flag in prefix.trim_start_matches('%').chars() {
        if let Some(class) = exclusion_class(flag) {
            specs.retain(|s| s.chars().next().map(|c| class.contains(c)).unwrap_or(false));
        }
    }

    // sh:105-106  _describe -t date-format-specifier 'date format specifier' specs -p PREFIX -S ''
    let prefix_p = if prefix.is_empty() {
        "%".to_string()
    } else {
        prefix.clone()
    };
    // `_describe` reads the spec list from a named array parameter.
    crate::ported::params::setaparam("specs", specs);
    let d: Vec<String> = vec![
        "-t".to_string(),
        "date-format-specifier".to_string(),
        "date format specifier".to_string(),
        "specs".to_string(),
        "-p".to_string(),
        prefix_p,
        "-S".to_string(),
        String::new(),
    ];
    let mut ret = 1;
    if _describe(&d) == 0 {
        ret = 0;
    }

    // sh:107
    if is_zsh {
        let _ = _message(&[
            "-e".to_string(),
            "date-format-precision".to_string(),
            "precision for %. (1-9)".to_string(),
        ]);
    }
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusion_classes_known() {
        assert_eq!(exclusion_class('E'), Some("cCgGxXyY"));
        assert_eq!(exclusion_class('z'), None);
    }

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(_date_formats(&[]), 1);
    }
}
