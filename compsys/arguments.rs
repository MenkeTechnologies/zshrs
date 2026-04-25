//! Native Rust implementation of _arguments
//!
//! This replaces the ~590 line zsh shell function with native Rust for speed.
//! The _arguments function is the most commonly used completion helper -
//! it parses option specifications and generates completions.

use crate::compcore::CompletionState;
use crate::completion::{Completion, CompletionFlags};
use crate::state::CompParams;
use std::collections::{HashMap, HashSet};

/// Option argument requirement
#[derive(Clone, Debug, PartialEq)]
pub enum ArgRequirement {
    /// No argument
    None,
    /// Required argument (`:`)
    Required,
    /// Optional argument (`::`)
    Optional,
}

/// Option type
#[derive(Clone, Debug, PartialEq)]
pub enum OptType {
    /// Short option: -x
    Short,
    /// Long option: --foo
    Long,
    /// Old-style long: -foo (single dash)
    OldLong,
    /// Plus option: +x
    Plus,
}

/// A parsed option specification
#[derive(Clone, Debug)]
pub struct OptSpec {
    /// The option string without leading dashes (e.g., "verbose", "v")
    pub name: String,
    /// Option type
    pub opt_type: OptType,
    /// Description shown in completion menu
    pub description: String,
    /// Argument requirement
    pub arg_req: ArgRequirement,
    /// Argument description/name (e.g., "FILE")
    pub arg_name: String,
    /// Action to complete the argument (e.g., "_files", "(yes no)")
    pub action: String,
    /// Options that are mutually exclusive with this one
    pub excludes: Vec<String>,
    /// Can this option be repeated?
    pub repeated: bool,
}

impl OptSpec {
    /// Get the full option string with dashes
    pub fn full_name(&self) -> String {
        match self.opt_type {
            OptType::Short => format!("-{}", self.name),
            OptType::Long => format!("--{}", self.name),
            OptType::OldLong => format!("-{}", self.name),
            OptType::Plus => format!("+{}", self.name),
        }
    }
}

/// A parsed positional argument specification
#[derive(Clone, Debug)]
pub struct ArgSpec {
    /// Position (1-based, 0 means "rest")
    pub position: usize,
    /// Description
    pub description: String,
    /// Action to complete
    pub action: String,
    /// Is this a "rest" argument (*:)?
    pub rest: bool,
}

/// Parsed _arguments specification
#[derive(Clone, Debug, Default)]
pub struct ArgumentsSpec {
    /// Option specifications
    pub options: Vec<OptSpec>,
    /// Positional argument specifications
    pub arguments: Vec<ArgSpec>,
    /// Whether -s (single-letter options can be combined) is set
    pub single_dash_combine: bool,
    /// Whether -S (don't complete options after --) is set
    pub no_opts_after_ddash: bool,
    /// Whether -A (complete options after first non-option) is set
    pub opts_anywhere: bool,
    /// Whether -W (options take arguments in next word) is set
    pub arg_in_next_word: bool,
}

impl ArgumentsSpec {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse a single option spec string
    ///
    /// Format: `[exclusions]opt[description]:arg-name:action`
    /// Examples:
    /// - `-v[verbose mode]`
    /// - `--help[show help]`
    /// - `(-v --verbose)'{-v,--verbose}'[be verbose]`
    /// - `*-d[debug mode]` (repeatable)
    /// - `--file=[file to use]:filename:_files`
    pub fn parse_opt_spec(spec: &str) -> Option<OptSpec> {
        let spec = spec.trim();
        if spec.is_empty() {
            return None;
        }

        let mut chars = spec.chars().peekable();

        // Parse exclusions: (opt1 opt2)
        let mut excludes = Vec::new();
        if chars.peek() == Some(&'(') {
            chars.next(); // consume '('
            let mut excl = String::new();
            while let Some(c) = chars.next() {
                if c == ')' {
                    if !excl.is_empty() {
                        excludes.push(excl);
                    }
                    break;
                } else if c.is_whitespace() {
                    if !excl.is_empty() {
                        excludes.push(excl);
                        excl = String::new();
                    }
                } else {
                    excl.push(c);
                }
            }
        }

        // Check for repeatability: *
        let repeated = chars.peek() == Some(&'*');
        if repeated {
            chars.next();
        }

        // Collect remaining string
        let rest: String = chars.collect();
        let rest = rest.trim();

        // Handle brace expansion for multiple options: {-v,--verbose}
        // For now, just take the first one
        let opt_str = if rest.starts_with('{') {
            if let Some(end) = rest.find('}') {
                let inside = &rest[1..end];
                inside.split(',').next().unwrap_or("").trim()
            } else {
                rest
            }
        } else if rest.starts_with('\'') || rest.starts_with('"') {
            // Quoted option
            let quote = rest.chars().next().unwrap();
            if let Some(end) = rest[1..].find(quote) {
                &rest[1..end + 1]
            } else {
                &rest[1..]
            }
        } else {
            rest
        };

        // Determine option type and name
        let (opt_type, name_start) = if opt_str.starts_with("--") {
            (OptType::Long, 2)
        } else if opt_str.starts_with('-') {
            if opt_str.len() > 2
                && opt_str
                    .chars()
                    .nth(2)
                    .map(|c| c.is_alphanumeric())
                    .unwrap_or(false)
            {
                (OptType::OldLong, 1)
            } else {
                (OptType::Short, 1)
            }
        } else if opt_str.starts_with('+') {
            (OptType::Plus, 1)
        } else {
            return None;
        };

        // Find where the option name ends
        let opt_part = &opt_str[name_start..];
        let name_end = opt_part
            .find(|c: char| c == '[' || c == '=' || c == ':' || c == '+' || c == '-')
            .unwrap_or(opt_part.len());
        let name = opt_part[..name_end].to_string();

        if name.is_empty() {
            return None;
        }

        // Parse description in [brackets]
        let mut description = String::new();
        if let Some(bracket_start) = opt_str.find('[') {
            if let Some(bracket_end) = opt_str[bracket_start..].find(']') {
                description = opt_str[bracket_start + 1..bracket_start + bracket_end].to_string();
            }
        }

        // Determine argument requirement from = or :
        let has_equal = opt_str.contains("=-") || opt_str.contains("=");
        let (arg_req, arg_name, action) = if has_equal || rest.contains(':') {
            // Find the part after the option spec
            let after_bracket = if let Some(pos) = rest.find(']') {
                &rest[pos + 1..]
            } else {
                // Find after the option name
                let after_name = name_end + name_start;
                if after_name < rest.len() {
                    &rest[after_name..]
                } else {
                    ""
                }
            };

            // Check for =-  (optional with =)
            let optional = opt_str.contains("=-") || after_bracket.starts_with("::");

            // Parse :arg-name:action
            let parts: Vec<&str> = after_bracket
                .trim_start_matches(':')
                .splitn(2, ':')
                .collect();
            let arg_name = parts.first().unwrap_or(&"").trim().to_string();
            let action = parts.get(1).unwrap_or(&"").trim().to_string();

            let req = if optional || after_bracket.starts_with("::") {
                ArgRequirement::Optional
            } else if !arg_name.is_empty() || !action.is_empty() || has_equal {
                ArgRequirement::Required
            } else {
                ArgRequirement::None
            };

            (req, arg_name, action)
        } else {
            (ArgRequirement::None, String::new(), String::new())
        };

        Some(OptSpec {
            name,
            opt_type,
            description,
            arg_req,
            arg_name,
            action,
            excludes,
            repeated,
        })
    }

    /// Parse a positional argument spec
    ///
    /// Format: `N:description:action` or `*:description:action`
    pub fn parse_arg_spec(spec: &str) -> Option<ArgSpec> {
        let spec = spec.trim();
        if spec.is_empty() {
            return None;
        }

        // Check for *: (rest arguments)
        let (rest, remaining) = if spec.starts_with('*') {
            (true, &spec[1..])
        } else {
            (false, spec)
        };

        // Must start with :
        if !remaining.starts_with(':') {
            // Could be N:desc:action format
            if let Some(colon_pos) = remaining.find(':') {
                let num_part = &remaining[..colon_pos];
                if let Ok(pos) = num_part.parse::<usize>() {
                    let after_num = &remaining[colon_pos + 1..];
                    let parts: Vec<&str> = after_num.splitn(2, ':').collect();
                    return Some(ArgSpec {
                        position: pos,
                        description: parts.first().unwrap_or(&"").to_string(),
                        action: parts.get(1).unwrap_or(&"").to_string(),
                        rest: false,
                    });
                }
            }
            return None;
        }

        let after_colon = &remaining[1..];
        let parts: Vec<&str> = after_colon.splitn(2, ':').collect();

        Some(ArgSpec {
            position: 0, // Will be set based on order
            description: parts.first().unwrap_or(&"").to_string(),
            action: parts.get(1).unwrap_or(&"").to_string(),
            rest,
        })
    }

    /// Parse full _arguments specification
    pub fn parse(args: &[String]) -> Self {
        let mut spec = Self::new();
        let mut arg_position = 1;

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];

            // Handle _arguments options
            match arg.as_str() {
                "-s" => spec.single_dash_combine = true,
                "-S" => spec.no_opts_after_ddash = true,
                "-A" => spec.opts_anywhere = true,
                "-W" => spec.arg_in_next_word = true,
                "-C" | "-R" | "-n" | "-w" => {
                    // Flags we recognize but don't need to store
                }
                "-O" | "-M" => {
                    // These take an argument
                    i += 1;
                }
                "--" => {
                    // Everything after -- is from --help parsing (skip for native impl)
                    break;
                }
                ":" => {
                    // Separator, ignore
                }
                _ => {
                    // Actual spec
                    if arg.starts_with('-')
                        || arg.starts_with('+')
                        || arg.starts_with('(')
                        || arg.starts_with('*')
                            && args.get(i).map(|s| s.contains('-')).unwrap_or(false)
                    {
                        // Option spec
                        if let Some(opt) = Self::parse_opt_spec(arg) {
                            spec.options.push(opt);
                        }
                    } else if arg.starts_with(':')
                        || arg.starts_with('*')
                        || arg
                            .chars()
                            .next()
                            .map(|c| c.is_ascii_digit())
                            .unwrap_or(false)
                    {
                        // Argument spec
                        if let Some(mut arg_spec) = Self::parse_arg_spec(arg) {
                            if arg_spec.position == 0 && !arg_spec.rest {
                                arg_spec.position = arg_position;
                                arg_position += 1;
                            }
                            spec.arguments.push(arg_spec);
                        }
                    } else {
                        // Try as option spec anyway
                        if let Some(opt) = Self::parse_opt_spec(arg) {
                            spec.options.push(opt);
                        }
                    }
                }
            }
            i += 1;
        }

        spec
    }
}

/// State for _arguments completion
#[derive(Debug)]
pub struct ArgumentsState<'a> {
    /// The parsed specification
    pub spec: &'a ArgumentsSpec,
    /// Current completion parameters
    pub params: &'a CompParams,
    /// Options that have been used
    pub used_options: HashSet<String>,
    /// Current positional argument index
    pub arg_index: usize,
    /// Whether we've seen --
    pub seen_ddash: bool,
    /// Parsed opt_args (option -> value)
    pub opt_args: HashMap<String, String>,
}

impl<'a> ArgumentsState<'a> {
    pub fn new(spec: &'a ArgumentsSpec, params: &'a CompParams) -> Self {
        let mut state = Self {
            spec,
            params,
            used_options: HashSet::new(),
            arg_index: 0,
            seen_ddash: false,
            opt_args: HashMap::new(),
        };
        state.analyze_words();
        state
    }

    /// Analyze the command line to determine state
    fn analyze_words(&mut self) {
        let current = self.params.current as usize;
        for (i, word) in self.params.words.iter().enumerate() {
            if i == 0 {
                continue; // Skip command name
            }
            if i >= current {
                break; // Don't analyze beyond cursor
            }

            if word == "--" {
                self.seen_ddash = true;
                continue;
            }

            if !self.seen_ddash && (word.starts_with('-') || word.starts_with('+')) {
                // It's an option
                let opt_name = word.trim_start_matches('-').trim_start_matches('+');

                // Find matching option spec
                for opt in &self.spec.options {
                    if opt.name == opt_name || opt.full_name() == *word {
                        self.used_options.insert(opt.full_name());

                        // Mark excludes as used too
                        for excl in &opt.excludes {
                            self.used_options.insert(excl.clone());
                        }

                        // If option takes argument, next word might be it
                        if opt.arg_req != ArgRequirement::None && i + 1 < current {
                            if let Some(next) = self.params.words.get(i + 1) {
                                if !next.starts_with('-') {
                                    self.opt_args.insert(opt.full_name(), next.clone());
                                }
                            }
                        }
                        break;
                    }
                }
            } else if self.seen_ddash || !word.starts_with('-') {
                // Positional argument
                self.arg_index += 1;
            }
        }
    }

    /// Get available options (not yet used, not excluded)
    pub fn available_options(&self) -> Vec<&OptSpec> {
        self.spec
            .options
            .iter()
            .filter(|opt| {
                let full = opt.full_name();
                (opt.repeated || !self.used_options.contains(&full))
                    && !opt.excludes.iter().any(|e| self.used_options.contains(e))
            })
            .collect()
    }

    /// Get the current positional argument spec, if any
    pub fn current_arg_spec(&self) -> Option<&ArgSpec> {
        // Find rest argument
        if let Some(rest) = self.spec.arguments.iter().find(|a| a.rest) {
            if self.arg_index >= self.spec.arguments.iter().filter(|a| !a.rest).count() {
                return Some(rest);
            }
        }

        // Find by position
        self.spec
            .arguments
            .iter()
            .find(|a| a.position == self.arg_index + 1)
    }

    /// Check if we're completing an option's argument
    pub fn completing_option_arg(&self) -> Option<&OptSpec> {
        let current = self.params.current as usize;
        if current < 2 {
            return None;
        }

        let prev_word = &self.params.words[current - 2];

        // Check if previous word was an option that takes an argument
        for opt in &self.spec.options {
            if opt.arg_req != ArgRequirement::None {
                if opt.full_name() == *prev_word {
                    return Some(opt);
                }
            }
        }

        // Check for --opt=value form
        let current = self.params.current_word();
        if let Some(eq_pos) = current.find('=') {
            let opt_part = &current[..eq_pos];
            for opt in &self.spec.options {
                if opt.full_name() == opt_part {
                    return Some(opt);
                }
            }
        }

        None
    }
}

/// Result of analyzing _arguments state
#[derive(Debug)]
pub struct ArgumentsAnalysis {
    /// Action to run for option argument (if any)
    pub opt_action: Option<String>,
    /// Action to run for positional argument (if any)
    pub arg_action: Option<String>,
    /// Available options to complete
    pub available_opts: Vec<OptSpec>,
    /// Whether we've seen --
    pub seen_ddash: bool,
    /// Current prefix
    pub prefix: String,
}

/// Analyze _arguments state without borrowing CompletionState mutably
pub fn arguments_analyze(params: &CompParams, spec: &ArgumentsSpec) -> ArgumentsAnalysis {
    let args_state = ArgumentsState::new(spec, params);

    let opt_action = args_state
        .completing_option_arg()
        .filter(|opt| !opt.action.is_empty())
        .map(|opt| opt.action.clone());

    let arg_action = args_state
        .current_arg_spec()
        .filter(|arg| !arg.action.is_empty())
        .map(|arg| arg.action.clone());

    let available_opts: Vec<OptSpec> = args_state
        .available_options()
        .into_iter()
        .cloned()
        .collect();

    ArgumentsAnalysis {
        opt_action,
        arg_action,
        available_opts,
        seen_ddash: args_state.seen_ddash,
        prefix: params.prefix.clone(),
    }
}

/// Execute _arguments completion
pub fn arguments_execute(
    state: &mut CompletionState,
    spec: &ArgumentsSpec,
    action_handler: impl Fn(&str, &mut CompletionState),
) -> bool {
    // Analyze first without mutable borrow
    let analysis = arguments_analyze(&state.params, spec);
    let mut added = false;

    // Check if completing option argument
    if let Some(action) = &analysis.opt_action {
        action_handler(action, state);
        return true;
    }

    // Check if completing positional argument
    if let Some(action) = &analysis.arg_action {
        action_handler(action, state);
        return true;
    }

    // Complete options if prefix starts with - or +
    if analysis.prefix.is_empty()
        || analysis.prefix.starts_with('-')
        || analysis.prefix.starts_with('+')
    {
        if !analysis.seen_ddash || !spec.no_opts_after_ddash {
            state.begin_group("options", true);

            for opt in &analysis.available_opts {
                let full = opt.full_name();
                if full.starts_with(&analysis.prefix) {
                    let mut comp = Completion::new(&full);

                    if !opt.description.is_empty() {
                        comp.disp = Some(format!("{} -- {}", full, opt.description));
                    }

                    // Add = suffix for options that take arguments with =
                    if opt.arg_req == ArgRequirement::Required && opt.opt_type == OptType::Long {
                        comp.suf = Some("=".to_string());
                        comp.flags |= CompletionFlags::NOSPACE;
                    }

                    state.add_match(comp, Some("options"));
                    added = true;
                }
            }

            state.end_group();
        }
    }

    added
}

/// Simple action parser for common patterns
pub fn parse_action(action: &str) -> ActionType {
    let action = action.trim();

    if action.is_empty() || action == " " {
        ActionType::Message(String::new())
    } else if action.starts_with("((") && action.ends_with("))") {
        // ((opt1\:desc1 opt2\:desc2))
        let inner = &action[2..action.len() - 2];
        let items: Vec<(String, String)> = inner
            .split_whitespace()
            .filter_map(|s| {
                let parts: Vec<&str> = s.splitn(2, "\\:").collect();
                if parts.is_empty() {
                    None
                } else {
                    Some((
                        parts[0].to_string(),
                        parts.get(1).unwrap_or(&"").to_string(),
                    ))
                }
            })
            .collect();
        ActionType::Literal(items)
    } else if action.starts_with('(') && action.ends_with(')') {
        // (val1 val2 val3)
        let inner = &action[1..action.len() - 1];
        let items: Vec<String> = inner.split_whitespace().map(String::from).collect();
        ActionType::Values(items)
    } else if action.starts_with('{') && action.ends_with('}') {
        // {eval code}
        ActionType::Eval(action[1..action.len() - 1].to_string())
    } else if action.starts_with("->") {
        // ->state
        ActionType::State(action[2..].trim().to_string())
    } else if action.starts_with('_') {
        // _function
        ActionType::Function(action.to_string())
    } else {
        ActionType::Function(action.to_string())
    }
}

/// Types of completion actions
#[derive(Clone, Debug)]
pub enum ActionType {
    /// Just show a message
    Message(String),
    /// Literal values with descriptions: ((val1\:desc1 val2\:desc2))
    Literal(Vec<(String, String)>),
    /// Simple values: (val1 val2)
    Values(Vec<String>),
    /// Evaluate shell code: {code}
    Eval(String),
    /// Transition to state: ->state
    State(String),
    /// Call completion function: _files
    Function(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_short_opt() {
        let opt = ArgumentsSpec::parse_opt_spec("-v[verbose mode]").unwrap();
        assert_eq!(opt.name, "v");
        assert_eq!(opt.opt_type, OptType::Short);
        assert_eq!(opt.description, "verbose mode");
        assert_eq!(opt.arg_req, ArgRequirement::None);
    }

    #[test]
    fn test_parse_long_opt() {
        let opt = ArgumentsSpec::parse_opt_spec("--help[show help message]").unwrap();
        assert_eq!(opt.name, "help");
        assert_eq!(opt.opt_type, OptType::Long);
        assert_eq!(opt.description, "show help message");
    }

    #[test]
    fn test_parse_opt_with_arg() {
        let opt =
            ArgumentsSpec::parse_opt_spec("--file=[file to process]:filename:_files").unwrap();
        assert_eq!(opt.name, "file");
        assert_eq!(opt.arg_req, ArgRequirement::Required);
        assert_eq!(opt.arg_name, "filename");
        assert_eq!(opt.action, "_files");
    }

    #[test]
    fn test_parse_opt_with_exclusions() {
        let opt = ArgumentsSpec::parse_opt_spec("(-q --quiet)--verbose[be verbose]").unwrap();
        assert_eq!(opt.name, "verbose");
        assert_eq!(opt.excludes, vec!["-q", "--quiet"]);
    }

    #[test]
    fn test_parse_repeated_opt() {
        let opt = ArgumentsSpec::parse_opt_spec("*-v[increase verbosity]").unwrap();
        assert_eq!(opt.name, "v");
        assert!(opt.repeated);
    }

    #[test]
    fn test_parse_arg_spec() {
        let arg = ArgumentsSpec::parse_arg_spec(":source file:_files").unwrap();
        assert_eq!(arg.description, "source file");
        assert_eq!(arg.action, "_files");
        assert!(!arg.rest);
    }

    #[test]
    fn test_parse_rest_arg() {
        let arg = ArgumentsSpec::parse_arg_spec("*:input files:_files").unwrap();
        assert_eq!(arg.description, "input files");
        assert!(arg.rest);
    }

    #[test]
    fn test_parse_full_spec() {
        let args = vec![
            "-v[verbose]".to_string(),
            "--help[show help]".to_string(),
            "--file=[input file]:file:_files".to_string(),
            ":output:_files".to_string(),
        ];
        let spec = ArgumentsSpec::parse(&args);

        assert_eq!(spec.options.len(), 3);
        assert_eq!(spec.arguments.len(), 1);
    }

    #[test]
    fn test_action_parser() {
        assert!(matches!(parse_action("_files"), ActionType::Function(_)));
        assert!(matches!(parse_action("(yes no)"), ActionType::Values(_)));
        assert!(matches!(
            parse_action("((y\\:yes n\\:no))"),
            ActionType::Literal(_)
        ));
        assert!(matches!(parse_action("->state"), ActionType::State(_)));
    }
}
