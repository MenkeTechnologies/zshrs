//! Port of `_numbers` (zsh Completion/Base/Utility/_numbers, 87 lines).
//!
//! Local shell reference: `compsys/functions/Base/Utility/_numbers`.
//!
//! Critical insight the previous stub got backwards: `_numbers` does
//! **not** enumerate numbers as completion matches. Enumerating
//! `min..max` is a DoS vector — try `_numbers -l 0 -m 1_000_000_000`.
//! What `_numbers` actually does:
//!
//!   - **Suffix-completion mode**: if the user has already typed a
//!     numeric prefix AND the caller provided unit-suffixes
//!     (`s m h` / `bytes kB MB GB` / …), emit the unit suffixes as
//!     matches so the user can pick `s` / `min` / `h` after typing
//!     `30` → `30s`.
//!   - **Entry mode** (no number typed yet): emit no matches but
//!     attach a `_description -x` MESSAGE giving the user the
//!     contextual range, units, default, and suffix list. The
//!     compsys list-display shows this so the user knows what
//!     range to type into.
//!
//! Flags:
//!   `-t tag`    — tag override (default `numbers`)
//!   `-u units`  — unit label (e.g. `seconds`)
//!   `-l min`    — minimum
//!   `-m max`    — maximum
//!   `-d def`    — default value
//!   `-N`        — allow negatives
//!   `-f`        — allow decimals (floats)
//!
//! Plus the unit-suffixes positional list (`s m h` etc.).

use crate::compcore::CompletionState;
use crate::completion::Completion;

pub struct NumbersOpts<'a> {
    pub tag: &'a str,
    pub description: &'a str,
    pub units: Option<&'a str>,
    pub min: Option<&'a str>,
    pub max: Option<&'a str>,
    pub default: Option<&'a str>,
    /// `-N` flag — allow leading `-`.
    pub allow_negative: bool,
    /// `-f` flag — allow `[0-9]*.[0-9]*` form.
    pub allow_float: bool,
    /// Positional unit-suffix list. Each entry is `[colon-leading]suf[:desc]`.
    /// Leading colon marks the default-unit entry (shell %d format spec).
    pub unit_suffixes: &'a [String],
}

impl<'a> Default for NumbersOpts<'a> {
    fn default() -> Self {
        Self {
            tag: "numbers",
            description: "number",
            units: None,
            min: None,
            max: None,
            default: None,
            allow_negative: false,
            allow_float: false,
            unit_suffixes: &[],
        }
    }
}

pub fn _numbers(state: &mut CompletionState, opts: &NumbersOpts<'_>) -> bool {
    // Auto-detect negative from min/max having a `-` prefix (shell:50).
    let allow_neg = opts.allow_negative
        || opts.min.map(|s| s.starts_with('-')).unwrap_or(false)
        || opts.max.map(|s| s.starts_with('-')).unwrap_or(false);

    let prefix = state.params.prefix.clone();

    // ── Suffix-completion mode (shell:52-58) ──────────────────────────
    // If the user already typed a number AND we have unit suffixes,
    // emit the unit suffixes.
    let typed_number = is_typed_number(&prefix, opts.allow_float, allow_neg);
    if typed_number && !opts.unit_suffixes.is_empty() {
        // Strip the number from PREFIX; remainder becomes iprefix.
        let num_len = number_prefix_len(&prefix, opts.allow_float, allow_neg);
        state.params.iprefix.push_str(&prefix[..num_len]);
        state.params.prefix = prefix[num_len..].to_string();

        state.begin_group("units", false);
        for raw in opts.unit_suffixes {
            // Strip leading `:` (default-unit marker) and trailing `:desc`.
            let body = raw.strip_prefix(':').unwrap_or(raw);
            let (suf, desc) = match body.find(':') {
                Some(i) => (&body[..i], Some(&body[i + 1..])),
                None => (body, None),
            };
            if !suf.starts_with(&state.params.prefix) {
                continue;
            }
            let mut c = Completion::new(suf.to_string());
            if let Some(d) = desc {
                c.disp = Some(format!("{} {}", suf, d));
            }
            state.add_match(c, Some("units"));
        }
        state.end_group();
        return state.nmatches > 0;
    }

    // ── Entry mode (shell:60-83) ──────────────────────────────────────
    // No number typed yet (or no units to complete). Emit a single
    // description message giving the user the context.
    let mut parts: Vec<String> = Vec::new();
    parts.push(opts.description.to_string());
    if let Some(u) = opts.units {
        parts.push(format!("({})", u));
    }
    let range = match (opts.min, opts.max) {
        (Some(lo), Some(hi)) => Some(format!("{}-{}", lo, hi)),
        (Some(lo), None) => Some(format!("{}-", lo)),
        (None, Some(hi)) => Some(format!("-{}", hi)),
        (None, None) => None,
    };
    if let Some(r) = range {
        parts.push(format!("({})", r));
    }
    if let Some(d) = opts.default {
        parts.push(format!("[{}]", d));
    }
    if !opts.unit_suffixes.is_empty() {
        let labels: Vec<&str> = opts
            .unit_suffixes
            .iter()
            .map(|s| {
                let body = s.strip_prefix(':').unwrap_or(s);
                match body.find(':') {
                    Some(i) => &body[..i],
                    None => body,
                }
            })
            .collect();
        parts.push(format!("[{}]", labels.join("|")));
    }
    let msg = parts.join(" ");
    // add_explanation only bumps nmessages when the named group
    // exists, so create it first (empty — _numbers doesn't enumerate).
    state.begin_group(opts.tag, true);
    state.end_group();
    state.add_explanation(msg, Some(opts.tag));
    // shell:81 `compstate[insert]=` — block unambiguous-insertion so
    // the message is actually shown. We don't have full compstate
    // wiring; the explanation surfaces through the list display.
    true
}

fn is_typed_number(s: &str, float: bool, neg: bool) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut chars = s.chars().peekable();
    if neg && chars.peek() == Some(&'-') {
        chars.next();
    }
    let mut saw_digit = false;
    let mut saw_dot = false;
    for c in chars {
        if c.is_ascii_digit() {
            saw_digit = true;
        } else if float && c == '.' && !saw_dot {
            saw_dot = true;
        } else {
            return false;
        }
    }
    saw_digit
}

fn number_prefix_len(s: &str, float: bool, neg: bool) -> usize {
    let mut len = 0;
    let mut chars = s.chars();
    if neg {
        if let Some('-') = chars.clone().next() {
            len += 1;
            chars.next();
        }
    }
    let mut saw_dot = false;
    for c in chars {
        if c.is_ascii_digit() {
            len += c.len_utf8();
        } else if float && c == '.' && !saw_dot {
            saw_dot = true;
            len += c.len_utf8();
        } else {
            break;
        }
    }
    len
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_typed_number_emits_message_not_matches() {
        let mut state = CompletionState::new();
        let opts = NumbersOpts {
            description: "delay",
            min: Some("0"),
            max: Some("60"),
            units: Some("seconds"),
            unit_suffixes: &["s".to_string(), "ms".to_string()],
            ..Default::default()
        };
        let ok = _numbers(&mut state, &opts);
        assert!(ok);
        // No actual match completions in entry mode.
        let total_matches: usize = state.groups.iter().map(|g| g.matches.len()).sum();
        assert_eq!(total_matches, 0, "_numbers must NOT enumerate numbers");
        // But at least one explanation was added.
        assert!(state.nmessages >= 1, "expected description message");
    }

    #[test]
    fn typed_number_with_suffixes_completes_units() {
        let mut state = CompletionState::new();
        state.params.prefix = "30".into();
        let opts = NumbersOpts {
            description: "delay",
            unit_suffixes: &[
                ":s:seconds".to_string(),
                "m:minutes".to_string(),
                "h:hours".to_string(),
            ],
            ..Default::default()
        };
        let ok = _numbers(&mut state, &opts);
        assert!(ok);
        let names: Vec<&str> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["s", "m", "h"], "got {names:?}");
        // PREFIX chewing: 30 went to iprefix; prefix is empty.
        assert_eq!(state.params.iprefix, "30");
        assert_eq!(state.params.prefix, "");
    }

    #[test]
    fn float_pattern_recognises_decimals() {
        let mut state = CompletionState::new();
        state.params.prefix = "1.5".into();
        let opts = NumbersOpts {
            allow_float: true,
            unit_suffixes: &["s".into()],
            ..Default::default()
        };
        let ok = _numbers(&mut state, &opts);
        assert!(ok);
        assert_eq!(state.params.iprefix, "1.5");
    }

    #[test]
    fn float_pattern_rejects_decimal_when_not_allowed() {
        // PREFIX "1.5" with allow_float=false → NOT a typed number
        // → fall to entry mode (no unit completion, message-only).
        let mut state = CompletionState::new();
        state.params.prefix = "1.5".into();
        let opts = NumbersOpts {
            allow_float: false,
            unit_suffixes: &["s".into()],
            ..Default::default()
        };
        let ok = _numbers(&mut state, &opts);
        assert!(ok);
        let total_matches: usize = state.groups.iter().map(|g| g.matches.len()).sum();
        assert_eq!(total_matches, 0);
    }

    #[test]
    fn negatives_inferred_from_min_with_dash() {
        let mut state = CompletionState::new();
        state.params.prefix = "-5".into();
        let opts = NumbersOpts {
            min: Some("-10"),
            max: Some("10"),
            unit_suffixes: &["dB".into()],
            ..Default::default()
        };
        let ok = _numbers(&mut state, &opts);
        assert!(ok);
        // -5 IS a typed number once neg inferred from min.
        assert_eq!(state.params.iprefix, "-5");
    }

    #[test]
    fn no_dos_on_huge_range() {
        // Pre-port stub would have allocated and emitted 1e9 strings.
        let mut state = CompletionState::new();
        let opts = NumbersOpts {
            min: Some("0"),
            max: Some("1000000000"),
            ..Default::default()
        };
        let ok = _numbers(&mut state, &opts);
        assert!(ok);
        let total: usize = state.groups.iter().map(|g| g.matches.len()).sum();
        assert!(total < 100, "should NOT enumerate huge ranges, got {total}");
    }
}
