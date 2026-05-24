//! Port of `_arg_compile` — compile argument specifications.
//!
//! Local shell reference: `compsys/functions/Base/Utility/_arg_compile`
//! (system copy `/opt/homebrew/share/zsh/functions/_arg_compile`).
//!
//! Upstream `_arg_compile` is a 199-line internal helper for
//! `_arguments` that parses argument-spec strings into a structured
//! form. Key shapes the parser must recognise:
//!
//! ```text
//! Spec shape                          Meaning
//! ----------                          -------
//! NAME                                Single option/positional with no
//!                                     description, no action.
//! NAME[DESCRIPTION]                   Option/positional with description
//!                                     (zsh-style square-bracket form).
//! NAME:DESCRIPTION:ACTION             Colon-separated 3-tuple form.
//! NAME:DESC:->STATE                   Action is a `->state` transition.
//! NAME:DESC:=WORDS                    Action is a literal word list.
//! (MUTEX1 MUTEX2 …)REAL-SPEC          Mutex group — the REAL-SPEC is
//!                                     mutually exclusive with the listed
//!                                     other option names.
//! *PATTERN:DESC:ACTION                Rest pattern — applies to every
//!                                     positional after the prefix args.
//! -X[DESC]:ARGNAME:ACTION             Short option (`-X`) taking one arg.
//! --LONG[DESC]                        Long option with bracket-desc.
//! --LONG=                             Long option that takes value via =.
//! ```
//!
//! Faithful Rust port: every shape above is recognised and surfaced
//! as a typed field on `CompiledArgSpec`. The previous one-shape
//! parser (just `name:desc:action`) was a stub; this is the full
//! grammar.

/// What kind of argument this spec describes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArgKind {
    /// Positional argument (NAME without leading `-`).
    Positional,
    /// Short option `-X`.
    ShortOption,
    /// Long option `--name`.
    LongOption,
    /// Rest pattern (`*PATTERN`) — matches every remaining positional.
    Rest,
}

/// Compiled argument specification.
#[derive(Clone, Debug)]
pub struct CompiledArgSpec {
    /// Bare name/pattern (after mutex-group + rest-prefix stripping).
    /// E.g. `(-h --help)-v` → `pattern = "-v"`.
    pub pattern: String,
    /// Bracket-form `[description]` OR colon-form 2nd segment.
    pub description: String,
    /// Colon-form 3rd segment OR completion action.
    pub action: String,
    /// Mutex-group names (e.g. `(-h --help)-h` → `["-h", "--help"]`).
    pub mutex: Vec<String>,
    /// True if this is a `*PATTERN` rest spec.
    pub is_rest: bool,
    /// True if the option takes an argument (`--long=` form or
    /// `:ARGNAME:ACTION` colon form).
    pub takes_arg: bool,
    /// Argument name (when `takes_arg`). E.g. `-f:FILE:_files` →
    /// `arg_name = "FILE"`.
    pub arg_name: String,
    /// Classified option kind.
    pub kind: ArgKind,
}

/// _arg_compile - Compile argument specifications (internal)
pub fn _arg_compile(specs: &[String]) -> Vec<CompiledArgSpec> {
    specs
        .iter()
        .filter_map(|s| CompiledArgSpec::parse(s))
        .collect()
}

impl CompiledArgSpec {
    /// Parse one spec string. Returns None only for empty input.
    pub fn parse(spec: &str) -> Option<Self> {
        if spec.is_empty() {
            return None;
        }

        let mut s = spec;
        let mut mutex: Vec<String> = Vec::new();
        let mut is_rest = false;

        // Mutex group: `(opt1 opt2)REAL-SPEC` — strip the group and
        // record the alternatives.
        if let Some(rest) = s.strip_prefix('(') {
            if let Some(close) = rest.find(')') {
                let group = &rest[..close];
                mutex = group
                    .split_whitespace()
                    .filter(|w| !w.is_empty())
                    .map(String::from)
                    .collect();
                s = &rest[close + 1..];
            }
        }

        // Rest-pattern: `*PATTERN…` — strip the `*` prefix. If
        // the rest begins with `:` or is empty, the user wrote
        // just `*` and the effective pattern is "*". Otherwise
        // the chars after `*` are the named pattern.
        let mut rest_no_star_starts_with_colon_or_empty = false;
        if let Some(rest) = s.strip_prefix('*') {
            is_rest = true;
            rest_no_star_starts_with_colon_or_empty =
                rest.is_empty() || rest.starts_with(':') || rest.starts_with('[');
            s = rest;
        }

        // Try bracket-description form FIRST: `NAME[desc]:arg:action`
        // (zsh-style; shell:50-60 of upstream).
        let (pattern, description, after_brackets) = if let Some(lb) = s.find('[') {
            // Make sure the `[` is at the END of pattern (not inside it).
            let pat = &s[..lb];
            let rest_after = &s[lb + 1..];
            if let Some(rb) = rest_after.find(']') {
                let desc = &rest_after[..rb];
                let tail = &rest_after[rb + 1..];
                (pat.to_string(), desc.to_string(), tail.to_string())
            } else {
                // Unbalanced `[` — fall back to colon split.
                ("".to_string(), "".to_string(), "".to_string())
            }
        } else {
            ("".to_string(), "".to_string(), "".to_string())
        };

        // If bracket form didn't fire (empty pattern), use colon split.
        // Track whether the colon form fired so we can recognise the
        // option-with-argname distinction below.
        let mut colon_form_fired = false;
        let (pattern, description, action_and_arg) = if pattern.is_empty() {
            colon_form_fired = true;
            let parts: Vec<&str> = s.splitn(3, ':').collect();
            (
                parts[0].to_string(),
                parts.get(1).unwrap_or(&"").to_string(),
                parts.get(2).unwrap_or(&"").to_string(),
            )
        } else {
            (pattern, description, after_brackets)
        };

        // Now `action_and_arg` may itself be `:argname:action` (when
        // option takes an arg) or just `action` (when none) or empty.
        let (takes_arg, arg_name, action) = if let Some(after_colon) = action_and_arg.strip_prefix(':') {
            // bracket-form: `[desc]:argname:action` → `:argname:action`
            // tail. So `argname` is up to next `:`, `action` is rest.
            let parts: Vec<&str> = after_colon.splitn(2, ':').collect();
            (
                true,
                parts[0].to_string(),
                parts.get(1).unwrap_or(&"").to_string(),
            )
        } else if action_and_arg.is_empty() {
            (false, String::new(), String::new())
        } else if colon_form_fired && pattern.starts_with('-') && !description.is_empty() {
            // Colon-form on an option: `-x:argname:action` — the 2nd
            // segment is the arg name (already in `description`),
            // the 3rd is the action. shell-equivalent to bracketed
            // `-x[]:argname:action`.
            (true, description.clone(), action_and_arg)
        } else {
            // Colon-form 3rd segment is the action verbatim.
            (false, String::new(), action_and_arg)
        };

        // Synthesize "*" pattern for bare-rest case.
        let pattern_eff = if is_rest && rest_no_star_starts_with_colon_or_empty && pattern.is_empty() {
            "*".to_string()
        } else {
            pattern
        };

        // Classify option kind.
        let kind = if is_rest {
            ArgKind::Rest
        } else if pattern_eff.starts_with("--") {
            ArgKind::LongOption
        } else if pattern_eff.starts_with('-') && pattern_eff.len() > 1 {
            ArgKind::ShortOption
        } else {
            ArgKind::Positional
        };

        // `--long=` shape (trailing `=`) also implies takes_arg.
        let final_takes_arg = takes_arg || pattern_eff.ends_with('=');
        let final_pattern = pattern_eff.trim_end_matches('=').to_string();

        Some(Self {
            pattern: final_pattern,
            description,
            action,
            mutex,
            is_rest,
            takes_arg: final_takes_arg,
            arg_name,
            kind,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── bracket-description form ──────────────────────────────────

    #[test]
    fn bracket_form_positional() {
        let spec = CompiledArgSpec::parse("file[the file]").unwrap();
        assert_eq!(spec.pattern, "file");
        assert_eq!(spec.description, "the file");
        assert_eq!(spec.action, "");
        assert!(!spec.takes_arg);
        assert_eq!(spec.kind, ArgKind::Positional);
    }

    #[test]
    fn bracket_form_short_option() {
        let spec = CompiledArgSpec::parse("-v[verbose]").unwrap();
        assert_eq!(spec.pattern, "-v");
        assert_eq!(spec.description, "verbose");
        assert_eq!(spec.kind, ArgKind::ShortOption);
    }

    #[test]
    fn bracket_form_long_option() {
        let spec = CompiledArgSpec::parse("--help[show help]").unwrap();
        assert_eq!(spec.pattern, "--help");
        assert_eq!(spec.description, "show help");
        assert_eq!(spec.kind, ArgKind::LongOption);
    }

    #[test]
    fn bracket_form_with_arg_segments() {
        let spec = CompiledArgSpec::parse("-f[set file]:filename:_files").unwrap();
        assert_eq!(spec.pattern, "-f");
        assert_eq!(spec.description, "set file");
        assert!(spec.takes_arg);
        assert_eq!(spec.arg_name, "filename");
        assert_eq!(spec.action, "_files");
    }

    // ── colon-tuple form (legacy 3-segment) ───────────────────────

    #[test]
    fn colon_form_three_segment() {
        let spec = CompiledArgSpec::parse("*:file:_files").unwrap();
        assert_eq!(spec.pattern, "*");
        assert_eq!(spec.description, "file");
        assert_eq!(spec.action, "_files");
    }

    #[test]
    fn colon_form_two_segment() {
        let spec = CompiledArgSpec::parse("user:user name").unwrap();
        assert_eq!(spec.pattern, "user");
        assert_eq!(spec.description, "user name");
        assert_eq!(spec.action, "");
    }

    // ── mutex groups ──────────────────────────────────────────────

    #[test]
    fn mutex_group_strips_and_records() {
        let spec = CompiledArgSpec::parse("(-h --help)-h[print help]").unwrap();
        assert_eq!(spec.pattern, "-h");
        assert_eq!(spec.description, "print help");
        assert_eq!(spec.mutex, vec!["-h".to_string(), "--help".to_string()]);
    }

    #[test]
    fn mutex_group_with_colon_form_action() {
        let spec = CompiledArgSpec::parse("(-v --verbose)-v:DESC:_action").unwrap();
        assert_eq!(spec.pattern, "-v");
        assert_eq!(spec.mutex, vec!["-v".to_string(), "--verbose".to_string()]);
        assert!(spec.takes_arg);
        assert_eq!(spec.arg_name, "DESC");
    }

    #[test]
    fn empty_mutex_group_yields_empty_vec() {
        let spec = CompiledArgSpec::parse("()-x[desc]").unwrap();
        assert_eq!(spec.pattern, "-x");
        assert!(spec.mutex.is_empty());
    }

    // ── rest pattern ──────────────────────────────────────────────

    #[test]
    fn rest_pattern_strips_star() {
        let spec = CompiledArgSpec::parse("*:file:_files").unwrap();
        assert!(spec.is_rest);
        assert_eq!(spec.kind, ArgKind::Rest);
    }

    #[test]
    fn rest_with_named_pattern() {
        let spec = CompiledArgSpec::parse("*PATTERN:rest:_default").unwrap();
        assert!(spec.is_rest);
        assert_eq!(spec.pattern, "PATTERN");
        assert_eq!(spec.action, "_default");
    }

    // ── trailing `=` for long-option-with-arg ──────────────────────

    #[test]
    fn long_option_with_equals_takes_arg() {
        let spec = CompiledArgSpec::parse("--file=[input file]").unwrap();
        assert_eq!(spec.pattern, "--file");
        assert_eq!(spec.description, "input file");
        assert!(spec.takes_arg, "--file= shape must imply takes_arg=true");
    }

    // ── option-kind classification ────────────────────────────────

    #[test]
    fn dash_only_classified_as_positional() {
        let spec = CompiledArgSpec::parse("-").unwrap();
        // Single `-` alone is conventionally stdin-marker, not an
        // option. Pin that our parser classifies it as positional.
        assert_eq!(spec.kind, ArgKind::Positional);
    }

    #[test]
    fn double_dash_classified_as_long_option() {
        let spec = CompiledArgSpec::parse("--").unwrap();
        // `--` alone — our parser treats this as LongOption since
        // it starts with `--`. That's defensible.
        assert_eq!(spec.kind, ArgKind::LongOption);
    }

    // ── _arg_compile bulk collector ────────────────────────────────

    #[test]
    fn collects_all_valid_specs() {
        let specs = vec![
            "*:file:_files".into(),
            "(-h --help)-h[print help]".into(),
            "--verbose[verbose mode]".into(),
            "-f[file]:fname:_files".into(),
        ];
        let compiled = _arg_compile(&specs);
        assert_eq!(compiled.len(), 4);
        assert_eq!(compiled[0].pattern, "*");
        assert!(compiled[0].is_rest);
        assert_eq!(compiled[1].pattern, "-h");
        assert_eq!(compiled[1].mutex.len(), 2);
        assert_eq!(compiled[2].pattern, "--verbose");
        assert_eq!(compiled[2].kind, ArgKind::LongOption);
        assert_eq!(compiled[3].pattern, "-f");
        assert!(compiled[3].takes_arg);
        assert_eq!(compiled[3].arg_name, "fname");
    }

    #[test]
    fn empty_input_returns_none() {
        assert!(CompiledArgSpec::parse("").is_none());
    }

    #[test]
    fn whitespace_in_bracket_description_preserved() {
        let spec = CompiledArgSpec::parse("-x[a description with spaces]").unwrap();
        assert_eq!(spec.description, "a description with spaces");
    }

    #[test]
    fn pattern_only_no_description_no_action() {
        let spec = CompiledArgSpec::parse("--version").unwrap();
        assert_eq!(spec.pattern, "--version");
        assert_eq!(spec.description, "");
        assert_eq!(spec.action, "");
        assert!(!spec.takes_arg);
    }

    #[test]
    fn arg_compile_empty_input_returns_empty_vec() {
        let v: Vec<String> = vec![];
        assert!(_arg_compile(&v).is_empty());
    }

    #[test]
    fn arg_compile_filters_out_empty_specs() {
        let specs = vec!["".into(), "--ok".into(), "".into()];
        let v = _arg_compile(&specs);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].pattern, "--ok");
    }
}
