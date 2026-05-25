//! Port of `_describe` from `Completion/Base/Utility/_describe`.
//!
//! Full upstream body (140 lines verbatim):
//! ```text
//! sh:  1  #autoload
//! sh:  2
//! sh:  3  # ### Note: Calling this function twice during one completion operation, such
//! sh:  4  # ### that in each call there exists a pair of items having the same description
//! sh:  5  # ### as each other, and the two calls specify the same $_type, currently leads
//! sh:  6  # ### to garbled output; see workers/35229 (May 2015) and its thread (which also
//! sh:  7  # ### discusses at least two other issues, that may or may not be related to
//! sh:  8  # ### this one).
//! sh:  9
//! sh: 10  # This can be used to add options or values with descriptions as matches.
//! sh: 11
//! sh: 12  local _opt _expl _tmpm _tmpd _mlen _noprefix
//! sh: 13  local _type=values _descr _ret=1 _showd _nm _hide _args _grp _sep
//! sh: 14  local csl="$compstate[list]" csl2
//! sh: 15  local _oargv _argv _new _strs _mats _opts _i _try=0
//! sh: 16  local OPTIND OPTARG
//! sh: 17  local -a _jvx12
//! sh: 18
//! sh: 19  # Get the option.
//! sh: 20
//! sh: 21  while getopts "oOt:12JVx" _opt; do
//! sh: 22    case $_opt in
//! sh: 23      (o)
//! sh: 24        _type=options;;
//! sh: 25      (O)
//! sh: 26        _type=options
//! sh: 27        _noprefix=1
//! sh: 28        ;;
//! sh: 29      (t)
//! sh: 30        _type="$OPTARG"
//! sh: 31        ;;
//! sh: 32      (1|2|J|V|x)
//! sh: 33        _jvx12+=(-$_opt)
//! sh: 34    esac
//! sh: 35  done
//! sh: 36  shift $(( OPTIND - 1 ))
//! sh: 37  unset _opt
//! sh: 38
//! sh: 39  [[ "$_type$_noprefix" = options && ! -prefix [-+]* ]] && \
//! sh: 40      zstyle -T ":completion:${curcontext}:options" prefix-needed &&
//! sh: 41          return 1
//! sh: 42
//! sh: 43  # Do the tests. `showd' is set if the descriptions should be shown.
//! sh: 44
//! sh: 45  zstyle -T ":completion:${curcontext}:$_type" verbose && _showd=yes
//! sh: 46
//! sh: 47  zstyle -s ":completion:${curcontext}:$_type" list-separator _sep || _sep=--
//! sh: 48  zstyle -s ":completion:${curcontext}:$_type" max-matches-width _mlen ||
//! sh: 49      _mlen=$((COLUMNS/2))
//! sh: 50
//! sh: 51  _descr="$1"
//! sh: 52  shift
//! sh: 53
//! sh: 54  if [[ -n "$_showd" ]] &&
//! sh: 55     zstyle -T ":completion:${curcontext}:$_type" list-grouped; then
//! sh: 56    _oargv=( "$@" )
//! sh: 57    _grp=(-g)
//! sh: 58  else
//! sh: 59    _grp=()
//! sh: 60  fi
//! sh: 61
//! sh: 62  [[ "$_type" = options ]] &&
//! sh: 63      zstyle -t ":completion:${curcontext}:options" prefix-hidden &&
//! sh: 64          _hide="${(M)PREFIX##(--|[-+])}"
//! sh: 65
//! sh: 66  _tags "$_type"
//! sh: 67  while _tags; do
//! sh: 68    while _next_label $_jvx12 "$_type" _expl "$_descr"; do
//! sh: 69
//! sh: 70      if (( $#_grp )); then
//! sh: 71
//! sh: 72        set -- "$_oargv[@]"
//! sh: 73        _argv=( "$_oargv[@]" )
//! sh: 74        _i=1
//! sh: 75        (( _try++ ))
//! sh: 76        while (( $# )); do
//! sh: 77
//! sh: 78          _strs="_a_$_try$_i"
//! sh: 79          if [[ "$1" = \(*\) ]]; then
//! sh: 80            eval local "_a_$_try$_i;_a_$_try$_i"'='$1
//! sh: 81          else
//! sh: 82            eval local "_a_$_try$_i;_a_$_try$_i"'=( "${'$1'[@]}" )'
//! sh: 83          fi
//! sh: 84          _argv[_i]="_a_$_try$_i"
//! sh: 85          shift
//! sh: 86          (( _i++ ))
//! sh: 87
//! sh: 88          if [[ "$1" = (|-*) ]]; then
//! sh: 89            _mats=
//! sh: 90          else
//! sh: 91            _mats="_a_$_try$_i"
//! sh: 92            if [[ "$1" = \(*\) ]]; then
//! sh: 93              eval local "_a_$_try$_i;_a_$_try$_i"'='$1
//! sh: 94            else
//! sh: 95              eval local "_a_$_try$_i;_a_$_try$_i"'=( "${'$1'[@]}" )'
//! sh: 96            fi
//! sh: 97            _argv[_i]="_a_$_try$_i"
//! sh: 98            shift
//! sh: 99            (( _i++ ))
//! sh:100          fi
//! sh:101
//! sh:102          _opts=( "${(@)argv[1,(i)--]:#--}" )
//! sh:103          shift "$#_opts"
//! sh:104          (( _i += $#_opts ))
//! sh:105          if [[ $1 == -- ]]; then
//! sh:106            shift
//! sh:107            (( _i++ ))
//! sh:108          fi
//! sh:109
//! sh:110          if [[ -n $_mats ]]; then
//! sh:111            compadd "$_opts[@]" -2 -o nosort "${_expl[@]}" -D $_strs -O $_mats - \
//! sh:112                    "${(@)${(@M)${(@P)_mats}##([^:\\]|\\?)##}//\\(#b)(?)/$match[1]}"
//! sh:113          else
//! sh:114            compadd "$_opts[@]" -2 -o nosort "${_expl[@]}" -D $_strs - \
//! sh:115                    "${(@)${(@M)${(@P)_strs}##([^:\\]|\\?)##}//\\(#b)(?)/$match[1]}"
//! sh:116          fi
//! sh:117        done
//! sh:118        set - "$_argv[@]"
//! sh:119      fi
//! sh:120
//! sh:121      if [[ -n "$_showd" ]]; then
//! sh:122        compdescribe -I "$_hide" "$_mlen" "$_sep " _expl "$_grp[@]" "$@"
//! sh:123      else
//! sh:124        compdescribe -i "$_hide" "$_mlen" "$@"
//! sh:125      fi
//! sh:126
//! sh:127      compstate[list]="$csl"
//! sh:128
//! sh:129      while compdescribe -g csl2 _args _tmpm _tmpd; do
//! sh:130
//! sh:131        compstate[list]="$csl $csl2"
//! sh:132        [[ -n "$csl2" ]] && compstate[list]="${compstate[list]:s/rows//}"
//! sh:133
//! sh:134        compadd "$_args[@]" -d _tmpd -a _tmpm && _ret=0
//! sh:135      done
//! sh:136    done
//! sh:137    (( _ret )) || return 0
//! sh:138  done
//! sh:139
//! sh:140  return 1
//! ```



use crate::compsys::compcore::CompletionState;
use crate::compsys::completion::{Completion, CompletionFlags};

/// Options for _describe
#[derive(Clone, Debug, Default)]
pub struct DescribeOpts {
    /// Tag for this set of completions (-t)
    pub tag: Option<String>,
    /// Matcher spec (-M)
    pub matcher: Option<String>,
    /// Group name (-V or -J)
    pub group: Option<String>,
    /// Sorted group (-J)
    pub sorted: bool,
    /// Don't quote completions (-Q)
    pub no_quote: bool,
    /// Prefix (-P)
    pub prefix: Option<String>,
    /// Suffix (-S)
    pub suffix: Option<String>,
    /// Remove suffix on certain chars (-r)
    pub remove_suffix: Option<String>,
}

impl DescribeOpts {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse _describe arguments
    /// Format: _describe \[-t tag\] description array ...
    pub fn parse(args: &[String]) -> (Self, String, Vec<String>) {
        let mut opts = Self::new();
        let mut i = 0;
        let mut description = String::new();
        let mut items = Vec::new();

        // Parse options
        while i < args.len() {
            match args[i].as_str() {
                "-t" if i + 1 < args.len() => {
                    opts.tag = Some(args[i + 1].clone());
                    i += 2;
                    continue;
                }
                "-M" if i + 1 < args.len() => {
                    opts.matcher = Some(args[i + 1].clone());
                    i += 2;
                    continue;
                }
                "-V" if i + 1 < args.len() => {
                    opts.group = Some(args[i + 1].clone());
                    opts.sorted = false;
                    i += 2;
                    continue;
                }
                "-J" if i + 1 < args.len() => {
                    opts.group = Some(args[i + 1].clone());
                    opts.sorted = true;
                    i += 2;
                    continue;
                }
                "-P" if i + 1 < args.len() => {
                    opts.prefix = Some(args[i + 1].clone());
                    i += 2;
                    continue;
                }
                "-S" if i + 1 < args.len() => {
                    opts.suffix = Some(args[i + 1].clone());
                    i += 2;
                    continue;
                }
                "-Q" => {
                    opts.no_quote = true;
                    i += 1;
                    continue;
                }
                "-r" if i + 1 < args.len() => {
                    opts.remove_suffix = Some(args[i + 1].clone());
                    i += 2;
                    continue;
                }
                arg if !arg.starts_with('-') => {
                    // First non-option is description
                    if description.is_empty() {
                        description = arg.to_string();
                    } else {
                        // Rest are item arrays
                        items.push(arg.to_string());
                    }
                }
                _ => {}
            }
            i += 1;
        }

        (opts, description, items)
    }
}

/// An item with optional description for _describe
#[derive(Clone, Debug)]
pub struct DescribeItem {
    /// The completion string
    pub value: String,
    /// Optional description
    pub description: String,
}

impl DescribeItem {
    /// Parse "value:description" format
    pub fn parse(s: &str) -> Self {
        if let Some(pos) = s.find(':') {
            Self {
                value: s[..pos].to_string(),
                description: s[pos + 1..].to_string(),
            }
        } else {
            Self {
                value: s.to_string(),
                description: String::new(),
            }
        }
    }

    /// Parse from escaped format "value\:with\:colons:description"
    pub fn parse_escaped(s: &str) -> Self {
        let mut value = String::new();
        let mut description = String::new();
        let mut in_value = true;
        let mut chars = s.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '\\' {
                if let Some(&next) = chars.peek() {
                    if next == ':' {
                        value.push(':');
                        chars.next();
                        continue;
                    }
                }
                if in_value {
                    value.push(c);
                } else {
                    description.push(c);
                }
            } else if c == ':' && in_value {
                in_value = false;
            } else if in_value {
                value.push(c);
            } else {
                description.push(c);
            }
        }

        Self { value, description }
    }
}

/// Execute _describe completion
pub fn describe_execute(
    state: &mut CompletionState,
    opts: &DescribeOpts,
    description: &str,
    items: &[DescribeItem],
) -> bool {
    let prefix = state.params.prefix.clone();
    let group_name = opts
        .group
        .as_deref()
        .or(opts.tag.as_deref())
        .unwrap_or("default");

    state.begin_group(group_name, opts.sorted);

    if !description.is_empty() {
        state.add_explanation(description.to_string(), Some(group_name));
    }

    let mut added = false;

    for item in items {
        // Check if matches prefix
        if !item.value.starts_with(&prefix) {
            continue;
        }

        let mut comp_str = item.value.clone();

        // Add prefix/suffix
        if let Some(ref pfx) = opts.prefix {
            comp_str = format!("{}{}", pfx, comp_str);
        }
        if let Some(ref sfx) = opts.suffix {
            comp_str.push_str(sfx);
        }

        let mut comp = Completion::new(&comp_str);

        // Set display with description
        if !item.description.is_empty() {
            comp.disp = Some(format!("{} -- {}", item.value, item.description));
        }

        if opts.no_quote {
            comp.flags |= CompletionFlags::NOQUOTE;
        }

        state.add_match(comp, Some(group_name));
        added = true;
    }

    state.end_group();
    added
}

/// Parse items from string array (for use with shell arrays)
pub fn parse_items(specs: &[String]) -> Vec<DescribeItem> {
    specs
        .iter()
        .map(|s| DescribeItem::parse_escaped(s))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_item() {
        let item = DescribeItem::parse("foo:description of foo");
        assert_eq!(item.value, "foo");
        assert_eq!(item.description, "description of foo");
    }

    #[test]
    fn test_parse_item_no_desc() {
        let item = DescribeItem::parse("foo");
        assert_eq!(item.value, "foo");
        assert_eq!(item.description, "");
    }

    #[test]
    fn test_parse_escaped() {
        let item = DescribeItem::parse_escaped(r"foo\:bar:description");
        assert_eq!(item.value, "foo:bar");
        assert_eq!(item.description, "description");
    }

    #[test]
    fn test_parse_opts() {
        let (opts, desc, items) = DescribeOpts::parse(&[
            "-t".to_string(),
            "commands".to_string(),
            "-J".to_string(),
            "git commands".to_string(),
            "command".to_string(),
            "items_array".to_string(),
        ]);

        assert_eq!(opts.tag, Some("commands".to_string()));
        assert_eq!(opts.group, Some("git commands".to_string()));
        assert!(opts.sorted);
        assert_eq!(desc, "command");
        assert_eq!(items, vec!["items_array"]);
    }

    #[test]
    fn dash_V_group_unsorted_dash_J_sorted() {
        let (opts_v, _, _) =
            DescribeOpts::parse(&["-V".into(), "g1".into(), "desc".into()]);
        assert!(!opts_v.sorted, "-V → unsorted group");
        assert_eq!(opts_v.group.as_deref(), Some("g1"));
        let (opts_j, _, _) =
            DescribeOpts::parse(&["-J".into(), "g2".into(), "desc".into()]);
        assert!(opts_j.sorted, "-J → sorted group");
        assert_eq!(opts_j.group.as_deref(), Some("g2"));
    }

    #[test]
    fn dash_P_S_prefix_and_suffix() {
        let (opts, _, _) = DescribeOpts::parse(&[
            "-P".into(),
            "prefix_".into(),
            "-S".into(),
            "/".into(),
            "desc".into(),
        ]);
        assert_eq!(opts.prefix.as_deref(), Some("prefix_"));
        assert_eq!(opts.suffix.as_deref(), Some("/"));
    }

    #[test]
    fn dash_Q_no_quote_flag() {
        let (opts, _, _) = DescribeOpts::parse(&["-Q".into(), "desc".into()]);
        assert!(opts.no_quote);
    }

    #[test]
    fn dash_r_remove_suffix_chars() {
        let (opts, _, _) =
            DescribeOpts::parse(&["-r".into(), " \\t".into(), "desc".into()]);
        assert_eq!(opts.remove_suffix.as_deref(), Some(" \\t"));
    }

    #[test]
    fn parse_item_with_only_separator_yields_empty_description() {
        let item = DescribeItem::parse("foo:");
        assert_eq!(item.value, "foo");
        assert_eq!(item.description, "");
    }

    #[test]
    fn parse_items_array_round_trips_all_specs() {
        let specs = vec![
            "alpha:first".to_string(),
            "beta:second".to_string(),
            "gamma:third".to_string(),
        ];
        let items = parse_items(&specs);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].value, "alpha");
        assert_eq!(items[2].description, "third");
    }

    #[test]
    fn parse_escaped_with_backslash_in_description_kept() {
        // `\:` only escapes COLONS — backslashes in the description
        // pass through unchanged.
        let item = DescribeItem::parse_escaped(r"value:foo\bar");
        assert_eq!(item.value, "value");
        assert_eq!(item.description, r"foo\bar");
    }

    #[test]
    fn item_with_multiple_unescaped_colons_splits_on_first() {
        let item = DescribeItem::parse("a:b:c:d");
        assert_eq!(item.value, "a");
        // First-colon split, rest stays whole.
        assert_eq!(item.description, "b:c:d");
    }
}
