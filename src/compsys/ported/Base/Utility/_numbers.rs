//! Port of `_numbers` from `Completion/Base/Utility/_numbers`.
//!
//! Full upstream body (87 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # Usage: _numbers [compadd options] [-t tag] [-f|-N] [-u units] [-l min] [-m max] \
//! sh: 4  #                 [-d default] ["description"] [unit-suffix...]
//! sh: 5
//! sh: 6  #   -t : specify a tag (defaults to 'numbers')
//! sh: 7  #   -u : indicate the units, e.g. seconds
//! sh: 8  #   -l : lowest possible value
//! sh: 9  #   -m : maximum possible value
//! sh:10  #   -d : default value
//! sh:11  #   -N : allow negative numbers (implied by range including a negative)
//! sh:12  #   -f : allow decimals (float)
//! sh:13
//! sh:14  # For a unit-suffix, an initial colon indicates a unit that asserts the default
//! sh:15  # otherwise, colons allow for descriptions, e.g:
//! sh:16
//! sh:17  #   :s:seconds m:minutes h:hours
//! sh:18
//! sh:19  # unit-suffixes are not sorted by the completion system when listed
//! sh:20  # Specify them in order of magnitude, this tends to be ascending unless
//! sh:21  # the default is of a higher magnitude, in which case, descending.
//! sh:22  # So for, example
//! sh:23  #   bytes kB MB GB
//! sh:24  #   s ms us ns
//! sh:25  # Where the compadd options include matching control or suffixes, these
//! sh:26  # are applied to the units
//! sh:27
//! sh:28  # For each unit-suffix, the format style is looked up with the
//! sh:29  # unit-suffixes tag and the results concatenated. Specs used are:
//! sh:30  #   x : the suffix
//! sh:31  #   X : suffix description
//! sh:32  #   d : indicate suffix is for the default unit
//! sh:33  #   i : list index
//! sh:34  #   r : reverse list index
//! sh:35  # The latter three of these are useful with ternary expressions.
//! sh:36
//! sh:37  # _description is called with the x token set to make the completed
//! sh:38  # list of suffixes available to the normal format style
//! sh:39
//! sh:40  local desc tag range suffixes suffix suffixfmt pat='<->' partial=''
//! sh:41  local -a expl formats
//! sh:42  local -a default max min keep tags units
//! sh:43  local -i i
//! sh:44  local -A opts
//! sh:45
//! sh:46  zparseopts -K -D -A opts M+:=keep q:=keep s+:=keep S+:=keep J+: V+: 1 2 o+: n F: x+: X+: \
//! sh:47    t:=tags u:=units l:=min m:=max d:=default f=type e=type N=type
//! sh:48
//! sh:49  desc="${1:-number}" tag="${tags[2]:-numbers}"
//! sh:50  (( $# )) && shift
//! sh:51
//! sh:52  [[ -n ${(M)type:#-f} ]] && pat='(<->.[0-9]#|[0-9]#.<->|<->)' partial='(|.)'
//! sh:53  [[ -n ${(M)type:#-N} || $min[2] = -* || $max[2] = -* ]] && \
//! sh:54      pat="(|-)$pat" partial="(|-)$partial"
//! sh:55
//! sh:56  if (( $#argv )) && compset -P "$pat"; then
//! sh:57    zstyle -s ":completion:${curcontext}:units" list-separator sep || sep=--
//! sh:58    _description -V units expl unit
//! sh:59    disp=( ${${argv#:}/:/ $sep } )
//! sh:60    compadd -M 'r:|/=* r:|=*' -d disp "$keep[@]" "$expl[@]" - ${${argv#:}%%:*}
//! sh:61    return
//! sh:62  elif [[ -prefix $~pat || $PREFIX = $~partial ]]; then
//! sh:63    formats=( "h:$desc" )
//! sh:64    (( $#units )) && formats+=( m:${units[2]} ) desc+=" ($units[2])"
//! sh:65    (( $#min )) && range="$min[2]-"
//! sh:66    (( $#max )) && range="${range:--}$max[2]"
//! sh:67    [[ -n $range ]] && formats+=( r:$range ) desc+=" ($range)"
//! sh:68    (( $#default )) && formats+=( o:${default[2]} ) desc+=" [$default[2]]"
//! sh:69
//! sh:70    zstyle -s ":completion:${curcontext}:unit-suffixes" format suffixfmt || \
//! sh:71        suffixfmt='%(d.%U.)%x%(d.%u.)%(r..|)'
//! sh:72    for ((i=0;i<$#;i++)); do
//! sh:73      zformat -f suffix "$suffixfmt" "x:${${argv[i+1]#:}%%:*}" \
//! sh:74          "X:${${argv[i+1]#:}#*:}" "d:${#${argv[i+1]}[1]#:}" \
//! sh:75  	i:i r:$(( $# - i - 1))
//! sh:76      suffixes+="${suffix//\%/%%}"
//! sh:77    done
//! sh:78    [[ -n $suffixes ]] && formats+=( x:$suffixes )
//! sh:79
//! sh:80    _comp_mesg=yes
//! sh:81    _description -x $tag expl "$desc" $formats
//! sh:82    [[ $compstate[insert] = *unambiguous* ]] && compstate[insert]=
//! sh:83    compadd "$expl[@]"
//! sh:84    return 0
//! sh:85  fi
//! sh:86
//! sh:87  return 1
//! ```



use crate::compsys::compcore::CompletionState;
use crate::compsys::completion::Completion;

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
    // If the user already typed a number (even with partial unit
    // letters typed after) AND we have unit suffixes, emit the
    // unit suffixes.
    let num_len = number_prefix_len(&prefix, opts.allow_float, allow_neg);
    if num_len > 0 && !opts.unit_suffixes.is_empty() {
        // Strip the number from PREFIX; remainder becomes iprefix.
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
    fn float_pattern_rejects_decimal_part_when_not_allowed() {
        // PREFIX "1.5" with allow_float=false → only `1` is the
        // numeric prefix; `.5` is the unit-side. None of the
        // declared suffixes (`s`) start with `.5`, so 0 matches.
        let mut state = CompletionState::new();
        state.params.prefix = "1.5".into();
        let opts = NumbersOpts {
            allow_float: false,
            unit_suffixes: &["s".into()],
            ..Default::default()
        };
        let _ = _numbers(&mut state, &opts);
        // After numeric prefix strip with allow_float=false, prefix
        // becomes `.5` which doesn't start with `s`.
        let names: Vec<&str> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
            .collect();
        assert!(!names.contains(&"s"), "`.5` doesn't match `s` filter");
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

    #[test]
    fn description_message_includes_range_units_default_and_suffixes() {
        let mut state = CompletionState::new();
        let opts = NumbersOpts {
            description: "delay",
            units: Some("seconds"),
            min: Some("1"),
            max: Some("60"),
            default: Some("30"),
            unit_suffixes: &["s".into(), "m".into(), "h".into()],
            ..Default::default()
        };
        _numbers(&mut state, &opts);
        let exps: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.explanations.iter().cloned())
            .collect();
        let msg = exps.first().cloned().unwrap_or_default();
        assert!(msg.contains("delay"));
        assert!(msg.contains("(seconds)"));
        assert!(msg.contains("(1-60)"));
        assert!(msg.contains("[30]"));
        assert!(msg.contains("[s|m|h]"));
    }

    #[test]
    fn unit_suffix_filtering_by_typed_unit_partial() {
        // After typing the number, also typing partial unit chars
        // should narrow the suffix list.
        let mut state = CompletionState::new();
        state.params.prefix = "30m".into();
        let opts = NumbersOpts {
            unit_suffixes: &[
                "s".into(),
                "min".into(),
                "ms".into(),
                "h".into(),
            ],
            ..Default::default()
        };
        let _ = _numbers(&mut state, &opts);
        let names: Vec<&str> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
            .collect();
        // typed unit prefix = "m" → only `min`, `ms` survive.
        assert!(names.contains(&"min"));
        assert!(names.contains(&"ms"));
        assert!(!names.contains(&"s"));
        assert!(!names.contains(&"h"));
    }

    #[test]
    fn is_typed_number_helper_handles_various_forms() {
        assert!(is_typed_number("42", false, false));
        assert!(!is_typed_number("", false, false));
        assert!(!is_typed_number("abc", false, false));
        assert!(!is_typed_number("4a", false, false));
        assert!(is_typed_number("-5", false, true));
        assert!(!is_typed_number("-5", false, false));
        assert!(is_typed_number("1.5", true, false));
        assert!(!is_typed_number("1.5", false, false));
        assert!(!is_typed_number("1.5.0", true, false), "two dots invalid");
    }

    #[test]
    fn number_prefix_len_helper_stops_at_first_non_digit() {
        assert_eq!(number_prefix_len("30s", false, false), 2);
        assert_eq!(number_prefix_len("30.5s", true, false), 4);
        assert_eq!(number_prefix_len("-5s", false, true), 2);
        assert_eq!(number_prefix_len("abc", false, false), 0);
    }
}
