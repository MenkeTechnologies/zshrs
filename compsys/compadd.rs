//! compadd builtin implementation
//!
//! compadd is the core builtin for adding completion matches.
//! See zshcompwid(1) for full documentation.

use super::completion::{Completion, CompletionFlags, CompletionReceiver};
// TODO: use super::matching::MatchSpec for -M option support
use super::state::CompParams;

/// Options parsed from compadd arguments
#[derive(Clone, Debug, Default)]
pub struct CompadOpts {
    /// -P: prefix to show before match
    pub prefix: Option<String>,
    /// -S: suffix to show after match
    pub suffix: Option<String>,
    /// -p: path prefix
    pub path_prefix: Option<String>,
    /// -s: path suffix
    pub path_suffix: Option<String>,
    /// -i: ignored prefix (moves to IPREFIX)
    pub ignored_prefix: Option<String>,
    /// -I: ignored suffix (moves to ISUFFIX)
    pub ignored_suffix: Option<String>,
    /// -W: "real" path prefix for file completions
    pub real_path_prefix: Option<String>,
    /// -d: array name containing display strings
    pub display_array: Option<String>,
    /// -J: sorted group name
    pub group_sorted: Option<String>,
    /// -V: unsorted group name  
    pub group_unsorted: Option<String>,
    /// -X: explanation string
    pub explanation: Option<String>,
    /// -x: message (always shown)
    pub message: Option<String>,
    /// -M: match specification
    pub match_spec: Option<String>,
    /// -F: array of suffixes to ignore (fignore)
    pub ignore_array: Option<String>,
    /// -r: remove suffix on these chars
    pub remove_suffix: Option<String>,
    /// -R: remove suffix function
    pub remove_func: Option<String>,
    /// -A: array to store matches
    pub array_param: Option<String>,
    /// -O: array to store original strings
    pub orig_param: Option<String>,
    /// -D: array(s) to filter in parallel
    pub filter_arrays: Vec<String>,
    /// -E: add N dummy matches
    pub dummies: i32,
    /// -o: ordering options
    pub order: Option<String>,
    /// -q: remove suffix on space
    pub remove_on_space: bool,
    /// -Q: quote match
    pub quote: bool,
    /// -f: file completion
    pub file: bool,
    /// -e: parameter expansion
    pub param_expansion: bool,
    /// -a: array expansion
    pub array: bool,
    /// -k: hash key expansion
    pub keys: bool,
    /// -n: don't list (hidden match)
    pub nolist: bool,
    /// -U: don't match against PREFIX
    pub no_match: bool,
    /// -l: display one per line
    pub displine: bool,
    /// -C: add all matches to list
    pub all: bool,
    /// -1: unique all (remove all duplicates)
    pub unique_all: bool,
    /// -2: unique consecutive (remove consecutive duplicates)
    pub unique_consecutive: bool,
}

impl CompadOpts {
    /// Parse compadd arguments
    pub fn parse(args: &[String]) -> Result<(Self, Vec<String>), String> {
        let mut opts = Self::default();
        let mut matches = Vec::new();
        let mut i = 0;

        while i < args.len() {
            let arg = &args[i];

            if arg == "--" {
                // End of options
                matches.extend(args[i + 1..].iter().cloned());
                break;
            }

            if !arg.starts_with('-') || arg == "-" {
                // Not an option, start of matches
                matches.extend(args[i..].iter().cloned());
                break;
            }

            let chars: Vec<char> = arg[1..].chars().collect();
            let mut j = 0;

            while j < chars.len() {
                let ch = chars[j];
                j += 1;

                match ch {
                    'P' => {
                        opts.prefix = Some(get_opt_arg(&chars, &mut j, &args, &mut i)?);
                    }
                    'S' => {
                        opts.suffix = Some(get_opt_arg(&chars, &mut j, &args, &mut i)?);
                    }
                    'p' => {
                        opts.path_prefix = Some(get_opt_arg(&chars, &mut j, &args, &mut i)?);
                    }
                    's' => {
                        opts.path_suffix = Some(get_opt_arg(&chars, &mut j, &args, &mut i)?);
                    }
                    'i' => {
                        opts.ignored_prefix = Some(get_opt_arg(&chars, &mut j, &args, &mut i)?);
                    }
                    'I' => {
                        opts.ignored_suffix = Some(get_opt_arg(&chars, &mut j, &args, &mut i)?);
                    }
                    'W' => {
                        opts.real_path_prefix = Some(get_opt_arg(&chars, &mut j, &args, &mut i)?);
                    }
                    'd' => {
                        opts.display_array = Some(get_opt_arg(&chars, &mut j, &args, &mut i)?);
                    }
                    'J' => {
                        opts.group_sorted = Some(get_opt_arg(&chars, &mut j, &args, &mut i)?);
                    }
                    'V' => {
                        opts.group_unsorted = Some(get_opt_arg(&chars, &mut j, &args, &mut i)?);
                    }
                    'X' => {
                        opts.explanation = Some(get_opt_arg(&chars, &mut j, &args, &mut i)?);
                    }
                    'x' => {
                        opts.message = Some(get_opt_arg(&chars, &mut j, &args, &mut i)?);
                    }
                    'M' => {
                        opts.match_spec = Some(get_opt_arg(&chars, &mut j, &args, &mut i)?);
                    }
                    'F' => {
                        opts.ignore_array = Some(get_opt_arg(&chars, &mut j, &args, &mut i)?);
                    }
                    'r' => {
                        opts.remove_suffix = Some(get_opt_arg(&chars, &mut j, &args, &mut i)?);
                    }
                    'R' => {
                        opts.remove_func = Some(get_opt_arg(&chars, &mut j, &args, &mut i)?);
                    }
                    'A' => {
                        opts.array_param = Some(get_opt_arg(&chars, &mut j, &args, &mut i)?);
                    }
                    'O' => {
                        opts.orig_param = Some(get_opt_arg(&chars, &mut j, &args, &mut i)?);
                    }
                    'D' => {
                        opts.filter_arrays
                            .push(get_opt_arg(&chars, &mut j, &args, &mut i)?);
                    }
                    'E' => {
                        let val = get_opt_arg(&chars, &mut j, &args, &mut i)?;
                        opts.dummies = val.parse().map_err(|_| "invalid number for -E")?;
                    }
                    'o' => {
                        // -o with optional argument
                        if j < chars.len() {
                            opts.order = Some(chars[j..].iter().collect());
                            j = chars.len();
                        } else if i + 1 < args.len() && !args[i + 1].starts_with('-') {
                            i += 1;
                            opts.order = Some(args[i].clone());
                        } else {
                            opts.order = Some("match".to_string());
                        }
                    }
                    'q' => opts.remove_on_space = true,
                    'Q' => opts.quote = true,
                    'f' => opts.file = true,
                    'e' => opts.param_expansion = true,
                    'a' => opts.array = true,
                    'k' => {
                        opts.array = true;
                        opts.keys = true;
                    }
                    'n' => opts.nolist = true,
                    'U' => opts.no_match = true,
                    'l' => opts.displine = true,
                    'C' => opts.all = true,
                    '1' => opts.unique_all = true,
                    '2' => opts.unique_consecutive = true,
                    _ => {
                        return Err(format!("unknown option: -{}", ch));
                    }
                }
            }
            i += 1;
        }

        Ok((opts, matches))
    }

    /// Get the group name (sorted takes precedence)
    pub fn group_name(&self) -> Option<&str> {
        self.group_sorted
            .as_deref()
            .or(self.group_unsorted.as_deref())
    }

    /// Is this group sorted?
    pub fn is_sorted(&self) -> bool {
        self.group_sorted.is_some() || self.group_unsorted.is_none()
    }

    /// Build completion flags from options
    pub fn to_flags(&self) -> CompletionFlags {
        let mut flags = CompletionFlags::empty();
        if self.remove_on_space {
            flags |= CompletionFlags::REMOVE;
        }
        if self.file {
            flags |= CompletionFlags::FILE;
        }
        if self.nolist {
            flags |= CompletionFlags::NOLIST;
        }
        if self.displine {
            flags |= CompletionFlags::DISPLINE;
        }
        if self.quote {
            flags |= CompletionFlags::QUOTE;
        }
        if self.param_expansion {
            flags |= CompletionFlags::ISPAR;
        }
        flags
    }
}

/// Helper to get option argument (either pasted or next arg)
fn get_opt_arg(
    chars: &[char],
    j: &mut usize,
    args: &[String],
    i: &mut usize,
) -> Result<String, String> {
    if *j < chars.len() {
        // Pasted argument: -Xfoo
        let val: String = chars[*j..].iter().collect();
        *j = chars.len();
        Ok(val)
    } else if *i + 1 < args.len() {
        // Separate argument: -X foo
        *i += 1;
        Ok(args[*i].clone())
    } else {
        Err("missing argument".to_string())
    }
}

/// Execute compadd with parsed options
pub fn compadd_execute(
    opts: &CompadOpts,
    match_strings: &[String],
    params: &CompParams,
    receiver: &mut CompletionReceiver,
    get_array: impl Fn(&str) -> Option<Vec<String>>,
) -> i32 {
    // Set up group
    let group_name = opts.group_name().unwrap_or("default");
    receiver.begin_group(group_name, opts.is_sorted());

    // Add explanation/message
    if let Some(ref exp) = opts.explanation {
        receiver.add_explanation(exp.clone());
    }
    if let Some(ref msg) = opts.message {
        receiver.add_explanation(msg.clone());
    }

    // Get display strings if -d specified
    let displays: Option<Vec<String>> =
        opts.display_array.as_ref().and_then(|name| get_array(name));

    // Get ignore patterns if -F specified
    let ignores: Option<Vec<String>> = opts.ignore_array.as_ref().and_then(|name| get_array(name));

    let base_flags = opts.to_flags();
    let mut added = 0;

    // Process each match
    for (idx, m) in match_strings.iter().enumerate() {
        // Check against PREFIX unless -U
        if !opts.no_match {
            if !matches_prefix(m, &params.prefix, opts.match_spec.as_deref()) {
                continue;
            }
        }

        // Check against ignore patterns
        if let Some(ref igns) = ignores {
            if should_ignore(m, igns) {
                continue;
            }
        }

        let mut comp = Completion::new(m.clone());
        comp.orig = m.clone();
        comp.flags = base_flags;

        // Apply prefixes/suffixes
        comp.pre = opts.prefix.clone();
        comp.suf = opts.suffix.clone();
        comp.ppre = opts.path_prefix.clone();
        comp.psuf = opts.path_suffix.clone();
        comp.ipre = opts.ignored_prefix.clone();
        comp.isuf = opts.ignored_suffix.clone();
        comp.prpre = opts.real_path_prefix.clone();

        // Apply display string
        if let Some(ref disps) = displays {
            if let Some(d) = disps.get(idx) {
                comp.disp = Some(d.clone());
            }
        }

        // Apply remove suffix settings
        comp.rems = opts.remove_suffix.clone();
        comp.remf = opts.remove_func.clone();

        // Set group
        comp.group = Some(group_name.to_string());

        if !receiver.add(comp) {
            // Hit limit
            break;
        }
        added += 1;
    }

    // Add dummies if requested
    for _ in 0..opts.dummies {
        let mut comp = Completion::new("");
        comp.flags = base_flags | CompletionFlags::DUMMY;
        receiver.add(comp);
    }

    if added == 0 && opts.dummies == 0 {
        1 // No matches added
    } else {
        0 // Success
    }
}

/// Check if match string matches the prefix (basic implementation)
fn matches_prefix(match_str: &str, prefix: &str, _match_spec: Option<&str>) -> bool {
    // TODO: implement full matcher-control specs (-M)
    // For now, simple prefix matching
    if prefix.is_empty() {
        return true;
    }
    match_str.to_lowercase().starts_with(&prefix.to_lowercase())
}

/// Check if match should be ignored based on fignore patterns
fn should_ignore(match_str: &str, ignores: &[String]) -> bool {
    for pattern in ignores {
        if pattern.starts_with("?*") {
            // ?*.ext means any file ending in .ext
            let suffix = &pattern[2..];
            if match_str.ends_with(suffix) {
                return true;
            }
        } else if match_str.ends_with(pattern) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_basic() {
        let args: Vec<String> = vec!["-P", "[", "-S", "]", "foo", "bar"]
            .into_iter()
            .map(String::from)
            .collect();
        let (opts, matches) = CompadOpts::parse(&args).unwrap();
        assert_eq!(opts.prefix, Some("[".to_string()));
        assert_eq!(opts.suffix, Some("]".to_string()));
        assert_eq!(matches, vec!["foo", "bar"]);
    }

    #[test]
    fn test_parse_pasted() {
        let args: Vec<String> = vec!["-Pprefix", "-Ssuffix", "match"]
            .into_iter()
            .map(String::from)
            .collect();
        let (opts, matches) = CompadOpts::parse(&args).unwrap();
        assert_eq!(opts.prefix, Some("prefix".to_string()));
        assert_eq!(opts.suffix, Some("suffix".to_string()));
        assert_eq!(matches, vec!["match"]);
    }

    #[test]
    fn test_parse_flags() {
        let args: Vec<String> = vec!["-qfnU", "match"]
            .into_iter()
            .map(String::from)
            .collect();
        let (opts, matches) = CompadOpts::parse(&args).unwrap();
        assert!(opts.remove_on_space);
        assert!(opts.file);
        assert!(opts.nolist);
        assert!(opts.no_match);
        assert_eq!(matches, vec!["match"]);
    }

    #[test]
    fn test_parse_double_dash() {
        let args: Vec<String> = vec!["-P", "x", "--", "-foo", "-bar"]
            .into_iter()
            .map(String::from)
            .collect();
        let (opts, matches) = CompadOpts::parse(&args).unwrap();
        assert_eq!(opts.prefix, Some("x".to_string()));
        assert_eq!(matches, vec!["-foo", "-bar"]);
    }

    #[test]
    fn test_parse_group() {
        let args: Vec<String> = vec!["-J", "mygroup", "a", "b"]
            .into_iter()
            .map(String::from)
            .collect();
        let (opts, matches) = CompadOpts::parse(&args).unwrap();
        assert_eq!(opts.group_name(), Some("mygroup"));
        assert!(opts.is_sorted());
        assert_eq!(matches, vec!["a", "b"]);

        let args: Vec<String> = vec!["-V", "unsorted", "x"]
            .into_iter()
            .map(String::from)
            .collect();
        let (opts, _) = CompadOpts::parse(&args).unwrap();
        assert_eq!(opts.group_name(), Some("unsorted"));
        assert!(!opts.is_sorted());
    }
}
