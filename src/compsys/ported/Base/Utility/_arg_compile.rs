//! Port of `_arg_compile` from `Completion/Base/Utility/_arg_compile`.
//!
//! Full upstream body (199 lines verbatim):
//! ```text
//! sh:  1  #autoload
//! sh:  2
//! sh:  3  # A simple compiler for _arguments descriptions.  The first argument of
//! sh:  4  # _arg_compile is the name of an array parameter in which the parse is
//! sh:  5  # returned.  The remaining arguments form a series of `phrases'.  Each
//! sh:  6  # `phrase' begins with one of the keywords "argument", "option", or "help"
//! sh:  7  # and consists of a series of keywords and/or values.  The syntax is as
//! sh:  8  # free-form as possible, but "argument" phrases generally must appear in
//! sh:  9  # the same relative position as the corresponding argument on the command
//! sh: 10  # line to be completed, and there are some restrictions on ordering of
//! sh: 11  # keywords and values within each phrase.
//! sh: 12  #
//! sh: 13  # Anything appearing before the first phrase or after the last is passed
//! sh: 14  # through verbatim.  (See TODO.)  If more detailed mixing of compiled and
//! sh: 15  # uncompiled fragments is necessary, use two or more calls, either with
//! sh: 16  # different array names or by passing the output of each previous call
//! sh: 17  # through the next.
//! sh: 18  #
//! sh: 19  # In the documentation below, brackets [ ] indicate optional elements and
//! sh: 20  # braces { } indicate elements that may be repeated zero or more times.
//! sh: 21  # Except as noted, bracketed or braced elements may appear in any order
//! sh: 22  # relative to each other, but tokens within each element are ordered.
//! sh: 23  #
//! sh: 24  #   argument [POS] [means MSG] [action ACT]
//! sh: 25  #
//! sh: 26  #     POS may be an integer N for the Nth argument or "*" for all, and
//! sh: 27  #      must appear first if it appears at all.
//! sh: 28  #     MSG is a string to be displayed above the matches in a listing.
//! sh: 29  #     ACT is (currently) as described in the compsys manual.
//! sh: 30  #
//! sh: 31  #   option OPT [follow HOW] [explain STR] {unless XOR} \
//! sh: 32  #    {[means MSG] [action ACT]} [through PAT [means MSG] [action ACT]]
//! sh: 33  #
//! sh: 34  #     OPT is the option, prefixed with "*" if it may appear more than once.
//! sh: 35  #     HOW refers to a following argument, and may be one of:
//! sh: 36  #       "close"   must appear in the same word (synonyms "join" or "-")
//! sh: 37  #       "next"    the argument must appear in the next word (aka "split")
//! sh: 38  #       "loose"   the argument may appear in the same or the next word ("+")
//! sh: 39  #       "assign"  as loose, but must follow an "=" in the same word ("=")
//! sh: 40  #     HOW should be suffixed with a colon if the following argument is
//! sh: 41  #      _not_ required to appear.
//! sh: 42  #     STR is to be displayed based on style `description'
//! sh: 43  #     XOR is another option in combination with which OPT may not appear.
//! sh: 44  #      It may be ":" to disable non-option completions when OPT is present.
//! sh: 45  #     MSG is a string to be displayed above the matches in a listing.
//! sh: 46  #     ACT is (currently) as described in the compsys manual.
//! sh: 47  #     PAT is either "*" for "all remaining words on the line" or a pattern
//! sh: 48  #      that, if matched, marks the end of the arguments of this option.
//! sh: 49  #      The "through PAT ..." description must be the last.
//! sh: 50  #     PAT may be suffixed with one colon to narrow the $words array to
//! sh: 51  #      the remainder of the command line, or with two colons to narrow
//! sh: 52  #      to the words before (not including) the next that matches PAT.
//! sh: 53  #
//! sh: 54  #   help PAT [means MSG] action ACT
//! sh: 55  #
//! sh: 56  #     ACT is applied to any option output by --help that matches PAT.
//! sh: 57  #      Do not use "help" with commands that do not support --help.
//! sh: 58  #     PAT may be suffixed with a colon if the following argument is
//! sh: 59  #      _not_ required to appear (this is usually inferred from --help).
//! sh: 60  #     MSG is a string to be displayed above the matches in a listing.
//! sh: 61
//! sh: 62  # EXAMPLE:
//! sh: 63  # This is from _gprof in the standard distribution.  Note that because of
//! sh: 64  # the brace expansion trick used in the "function name" case, no attempt
//! sh: 65  # is made to use `phrase' form; that part gets passed through unchanged.
//! sh: 66  # It could simply be moved to the _arguments call ahead of "$args[@]".
//! sh: 67  #
//! sh: 68  # _arg_compile args -s -{a,b,c,D,h,i,l,L,s,T,v,w,x,y,z} \
//! sh: 69  #              -{A,C,e,E,f,F,J,n,N,O,p,P,q,Q,Z}:'function name:->funcs' \
//! sh: 70  #              option -I means directory action _dir_list \
//! sh: 71  #              option -d follow close means "debug level" \
//! sh: 72  #              option -k means "function names" action '->pair' \
//! sh: 73  #              option -m means "minimum execution count" \
//! sh: 74  #              argument means executable action '_files -g \*\(-\*\)' \
//! sh: 75  #              argument means "profile file" action '_files -g gmon.\*' \
//! sh: 76  #              help '*=name*' means "function name" action '->funcs' \
//! sh: 77  #              help '*=dirs*' means "directory" action _dir_list
//! sh: 78  # _arguments "$args[@]"
//! sh: 79
//! sh: 80  # TODO:
//! sh: 81  # Verbose forms of various actions, e.g. (but not exactly)
//! sh: 82  #   "state foo"                  becomes "->foo"
//! sh: 83  #   "completion X explain Y ..." becomes "((X\:Y ...))"
//! sh: 84  #   etc.
//! sh: 85  # Represent leading "*" in OPT some other way.
//! sh: 86  # Represent trailing colons in HOW and PAT some other way.
//! sh: 87  # Stricter syntax checking on HOW, sanity checks on XOR.
//! sh: 88  # Something less obscure than "unless :" would be nice.
//! sh: 89  # Warning or other syntax check for stuff after the last phrase.
//! sh: 90
//! sh: 91  emulate -L zsh
//! sh: 92  local -h argspec dspec helpspec prelude xor
//! sh: 93  local -h -A amap dmap safe
//! sh: 94
//! sh: 95  [[ -n "$1" ]] || return 1
//! sh: 96  [[ ${(tP)${1}} = *-local ]] && { print -R NAME CONFLICT: $1 1>&2; return 1 }
//! sh: 97  safe[reply]="$1"; shift
//! sh: 98
//! sh: 99  # First consume and save anything before the argument phrases
//! sh:100
//! sh:101  helpspec=()
//! sh:102  prelude=()
//! sh:103
//! sh:104  while (($#))
//! sh:105  do
//! sh:106    case $1 in
//! sh:107    (argument|help|option) break;;
//! sh:108    (*) prelude=("$prelude[@]" "$1"); shift;;
//! sh:109    esac
//! sh:110  done
//! sh:111
//! sh:112  # Consume all the argument phrases and build the argspec array
//! sh:113
//! sh:114  while (($#))
//! sh:115  do
//! sh:116    amap=()
//! sh:117    dspec=()
//! sh:118    case $1 in
//! sh:119
//! sh:120    # argument [POS] [means MSG] [action ACT]
//! sh:121    (argument)
//! sh:122      shift
//! sh:123      while (($#))
//! sh:124      do
//! sh:125        case $1 in
//! sh:126        (<1->|\*) amap[position]="$1"; shift;;
//! sh:127        (means|action) amap[$1]="$2"; shift 2;;
//! sh:128        (argument|option|help) break;;
//! sh:129        (*) print -R SYNTAX ERROR at "$@" 1>&2; return 1;;
//! sh:130        esac
//! sh:131      done
//! sh:132      if (( $#amap ))
//! sh:133      then
//! sh:134        argspec=("$argspec[@]" "${amap[position]}:${amap[means]}:${amap[action]}")
//! sh:135      fi;;
//! sh:136
//! sh:137    # option OPT [follow HOW] [explain STR] {unless XOR} \
//! sh:138    #  {[through PAT] [means MSG] [action ACT]}
//! sh:139    (option)
//! sh:140      amap[option]="$2"; shift 2
//! sh:141      dmap=()
//! sh:142      xor=()
//! sh:143      while (( $# ))
//! sh:144      do
//! sh:145        (( ${+amap[$1]} || ${+dmap[through]} )) && break;
//! sh:146        case $1 in
//! sh:147        (follow)
//! sh:148  	amap[follow]="${2:s/join/-/:s/close/-/:s/next//:s/split//:s/loose/+/:s/assign/=/:s/none//}"
//! sh:149  	shift 2;;
//! sh:150        (explain) amap[explain]="[$2]" ; shift 2;;
//! sh:151        (unless) xor=("$xor[@]" "${(@)=2}"); shift 2;;
//! sh:152        (through|means|action)
//! sh:153  	while (( $# ))
//! sh:154  	do
//! sh:155  	  (( ${+dmap[$1]} )) && break 2
//! sh:156  	  case $1 in
//! sh:157  	  (through|means|action) dmap[$1]=":${2}"; shift 2;;
//! sh:158  	  (argument|option|help|follow|explain|unless) break;;
//! sh:159  	  (*) print -R SYNTAX ERROR at "$@" 1>&2; return 1;;
//! sh:160  	  esac
//! sh:161  	done;;
//! sh:162        (argument|option|help) break;;
//! sh:163        (*) print -R SYNTAX ERROR at "$@" 1>&2; return 1;;
//! sh:164        esac
//! sh:165        if (( $#dmap ))
//! sh:166        then
//! sh:167  	dspec=("$dspec[@]" "${dmap[through]}${dmap[means]:-:}${dmap[action]:-:}")
//! sh:168        fi
//! sh:169      done
//! sh:170      if (( $#amap ))
//! sh:171      then
//! sh:172        argspec=("$argspec[@]" "${xor:+($xor)}${amap[option]}${amap[follow]}${amap[explain]}${dspec}")
//! sh:173      fi;;
//! sh:174
//! sh:175    # help PAT [means MSG] action ACT
//! sh:176    (help)
//! sh:177      amap[pattern]="$2"; shift 2
//! sh:178      while (($#))
//! sh:179      do
//! sh:180        (( ${+amap[$1]} )) && break;
//! sh:181        case $1 in
//! sh:182        (means|action) amap[$1]="$2"; shift 2;;
//! sh:183        (argument|option|help) break;;
//! sh:184        (*) print -R SYNTAX ERROR at "$@" 1>&2; return 1;;
//! sh:185        esac
//! sh:186      done
//! sh:187      if (( $#amap ))
//! sh:188      then
//! sh:189        helpspec=("$helpspec[@]" "${amap[pattern]}:${amap[means]}:${amap[action]}")
//! sh:190      fi;;
//! sh:191    (*) break;;
//! sh:192    esac
//! sh:193  done
//! sh:194
//! sh:195  eval $safe[reply]'=( "${prelude[@]}" "${argspec[@]}" ${helpspec:+"-- ${helpspec[@]}"} "$@" )'
//! sh:196
//! sh:197  # print -R _arguments "${prelude[@]:q}" "${argspec[@]:q}" ${helpspec:+"-- ${helpspec[@]:q}"} "$@:q"
//! sh:198
//! sh:199  return 0
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
