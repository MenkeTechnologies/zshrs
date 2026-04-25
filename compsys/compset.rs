//! compset builtin implementation
//!
//! compset manipulates PREFIX, SUFFIX, IPREFIX, ISUFFIX and words array.
//! See zshcompwid(1) for full documentation.

use super::state::CompParams;

/// compset operation type
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompsetOp {
    /// -p N: ignore first N characters of PREFIX
    PrefixNum(usize),
    /// -P pattern: ignore PREFIX matching pattern
    PrefixPat(String),
    /// -s N: ignore last N characters of SUFFIX  
    SuffixNum(usize),
    /// -S pattern: ignore SUFFIX matching pattern
    SuffixPat(String),
    /// -n begin end: restrict words to numeric range
    RangeNum(i32, i32),
    /// -N begin [end]: restrict words to pattern range
    RangePat(String, Option<String>),
    /// -q: quote removal (set up for subword completion)
    Quote,
}

impl CompsetOp {
    /// Parse compset arguments into operation
    pub fn parse(args: &[String]) -> Result<Self, String> {
        if args.is_empty() {
            return Err("compset: missing option".to_string());
        }

        let opt = &args[0];
        if !opt.starts_with('-') || opt.len() < 2 {
            return Err(format!("compset: invalid option: {}", opt));
        }

        let flag = opt.chars().nth(1).unwrap();

        // Check for pasted argument (-pN instead of -p N)
        let pasted = if opt.len() > 2 {
            Some(opt[2..].to_string())
        } else {
            None
        };

        match flag {
            'p' => {
                let val = pasted
                    .or_else(|| args.get(1).cloned())
                    .ok_or("compset -p: missing argument")?;
                let n: usize = val
                    .parse()
                    .map_err(|_| format!("compset -p: invalid number: {}", val))?;
                Ok(CompsetOp::PrefixNum(n))
            }
            'P' => {
                let val = pasted
                    .or_else(|| args.get(1).cloned())
                    .ok_or("compset -P: missing argument")?;
                Ok(CompsetOp::PrefixPat(val))
            }
            's' => {
                let val = pasted
                    .or_else(|| args.get(1).cloned())
                    .ok_or("compset -s: missing argument")?;
                let n: usize = val
                    .parse()
                    .map_err(|_| format!("compset -s: invalid number: {}", val))?;
                Ok(CompsetOp::SuffixNum(n))
            }
            'S' => {
                let val = pasted
                    .or_else(|| args.get(1).cloned())
                    .ok_or("compset -S: missing argument")?;
                Ok(CompsetOp::SuffixPat(val))
            }
            'n' => {
                let has_pasted = pasted.is_some();
                let arg1 = pasted
                    .or_else(|| args.get(1).cloned())
                    .ok_or("compset -n: missing begin argument")?;
                let arg2 = args
                    .get(if has_pasted { 1 } else { 2 })
                    .cloned()
                    .unwrap_or_else(|| "-1".to_string());
                let begin: i32 = arg1
                    .parse()
                    .map_err(|_| format!("compset -n: invalid number: {}", arg1))?;
                let end: i32 = arg2
                    .parse()
                    .map_err(|_| format!("compset -n: invalid number: {}", arg2))?;
                Ok(CompsetOp::RangeNum(begin, end))
            }
            'N' => {
                let has_pasted = pasted.is_some();
                let arg1 = pasted
                    .or_else(|| args.get(1).cloned())
                    .ok_or("compset -N: missing pattern argument")?;
                let arg2 = args.get(if has_pasted { 1 } else { 2 }).cloned();
                Ok(CompsetOp::RangePat(arg1, arg2))
            }
            'q' => Ok(CompsetOp::Quote),
            _ => Err(format!("compset: unknown option: -{}", flag)),
        }
    }
}

/// Execute compset operation, modifying params in place
/// Returns 0 on success, 1 on failure (e.g., pattern didn't match)
pub fn compset_execute(op: &CompsetOp, params: &mut CompParams) -> i32 {
    match op {
        CompsetOp::PrefixNum(n) => {
            let n = *n;
            if n > params.prefix.len() {
                return 1;
            }
            let removed: String = params.prefix.chars().take(n).collect();
            params.iprefix.push_str(&removed);
            params.prefix = params.prefix.chars().skip(n).collect();
            0
        }

        CompsetOp::PrefixPat(pattern) => {
            // Match pattern against prefix
            // Pattern syntax: basic glob with * and ?
            // If pattern starts with *, match shortest; if ends with *, match longest from start
            if let Some(matched) = match_prefix_pattern(&params.prefix, pattern) {
                params.iprefix.push_str(&matched);
                params.prefix = params.prefix[matched.len()..].to_string();
                0
            } else {
                1
            }
        }

        CompsetOp::SuffixNum(n) => {
            let n = *n;
            if n > params.suffix.len() {
                return 1;
            }
            let suffix_len = params.suffix.len();
            let removed: String = params.suffix.chars().skip(suffix_len - n).collect();
            params.suffix = params.suffix.chars().take(suffix_len - n).collect();
            params.isuffix = format!("{}{}", removed, params.isuffix);
            0
        }

        CompsetOp::SuffixPat(pattern) => {
            if let Some(matched) = match_suffix_pattern(&params.suffix, pattern) {
                let suffix_len = params.suffix.len();
                let matched_len = matched.len();
                params.suffix = params.suffix[..suffix_len - matched_len].to_string();
                params.isuffix = format!("{}{}", matched, params.isuffix);
                0
            } else {
                1
            }
        }

        CompsetOp::RangeNum(begin, end) => {
            let len = params.words.len() as i32;
            let current = params.current;

            // Convert to 0-based, handling negative indices
            let b = if *begin < 0 { len + begin } else { begin - 1 };
            let e = if *end < 0 { len + end } else { end - 1 };

            if b < 0 || e < 0 || b > e || b >= len {
                return 1;
            }

            // Check if current word is in range
            let current_0 = current - 1;
            if current_0 < b || current_0 > e {
                return 1;
            }

            // Restrict words array
            let b_usize = b as usize;
            let e_usize = (e + 1) as usize;
            params.words = params.words[b_usize..e_usize].to_vec();
            params.current = current - b;
            0
        }

        CompsetOp::RangePat(begin_pat, end_pat) => {
            let current = params.current as usize;
            if current == 0 || current > params.words.len() {
                return 1;
            }

            // Search backward from current for begin pattern
            let mut begin_idx = None;
            for i in (0..current).rev() {
                if glob_match(&params.words[i], begin_pat) {
                    begin_idx = Some(i + 1); // Start after the match
                    break;
                }
            }
            let begin_idx = begin_idx.unwrap_or(0);

            // Search forward from current for end pattern (if provided)
            let end_idx = if let Some(ref ep) = end_pat {
                let mut found = None;
                for i in current..params.words.len() {
                    if glob_match(&params.words[i], ep) {
                        found = Some(i); // End before the match
                        break;
                    }
                }
                found.unwrap_or(params.words.len())
            } else {
                params.words.len()
            };

            if begin_idx >= end_idx {
                return 1;
            }

            params.words = params.words[begin_idx..end_idx].to_vec();
            params.current = (current - begin_idx + 1) as i32;
            0
        }

        CompsetOp::Quote => {
            // Quote removal - split current word as if it were a command line
            // This is used for completing inside quoted strings
            // For now, basic implementation
            let word = format!("{}{}", params.prefix, params.suffix);

            // Try to detect and strip quotes
            let (new_prefix, new_suffix, quote_char) = strip_quotes(&word, params.prefix.len());

            if quote_char.is_some() {
                params.prefix = new_prefix;
                params.suffix = new_suffix;
                params.compstate.quote = quote_char.map(|c| c.to_string()).unwrap_or_default();
                0
            } else {
                1
            }
        }
    }
}

/// Match a pattern against a prefix, returning the matched portion
fn match_prefix_pattern(text: &str, pattern: &str) -> Option<String> {
    // Handle patterns like:
    // *: - match up to and including first :
    // [^/]# - match everything except /
    // etc.

    // Simple implementation: if pattern ends with a literal char, find first occurrence
    if pattern.starts_with('*') && pattern.len() > 1 {
        let suffix = &pattern[1..];
        if let Some(pos) = text.find(suffix) {
            return Some(text[..pos + suffix.len()].to_string());
        }
    }

    // Pattern with # at end means match one or more
    if pattern.ends_with('#') {
        let base = &pattern[..pattern.len() - 1];
        // For [^x] patterns
        if base.starts_with("[^") && base.ends_with(']') {
            let exclude_char = base.chars().nth(2)?;
            let mut end = 0;
            for (i, c) in text.char_indices() {
                if c == exclude_char {
                    break;
                }
                end = i + c.len_utf8();
            }
            if end > 0 {
                return Some(text[..end].to_string());
            }
        }
    }

    // Literal prefix match
    if !pattern.contains('*') && !pattern.contains('?') && !pattern.contains('[') {
        if text.starts_with(pattern) {
            return Some(pattern.to_string());
        }
    }

    // Full glob match
    if glob_match(text, pattern) {
        return Some(text.to_string());
    }

    None
}

/// Match a pattern against a suffix, returning the matched portion
fn match_suffix_pattern(text: &str, pattern: &str) -> Option<String> {
    // Mirror of prefix pattern matching but from the end

    if pattern.ends_with('*') && pattern.len() > 1 {
        let prefix = &pattern[..pattern.len() - 1];
        if let Some(pos) = text.rfind(prefix) {
            return Some(text[pos..].to_string());
        }
    }

    if !pattern.contains('*') && !pattern.contains('?') && !pattern.contains('[') {
        if text.ends_with(pattern) {
            return Some(pattern.to_string());
        }
    }

    if glob_match(text, pattern) {
        return Some(text.to_string());
    }

    None
}

/// Simple glob matching
pub fn glob_match(text: &str, pattern: &str) -> bool {
    let text_chars: Vec<char> = text.chars().collect();
    let pat_chars: Vec<char> = pattern.chars().collect();
    glob_match_impl(&text_chars, &pat_chars)
}

fn glob_match_impl(text: &[char], pattern: &[char]) -> bool {
    let mut ti = 0;
    let mut pi = 0;
    let mut star_pi = None;
    let mut star_ti = 0;

    while ti < text.len() {
        if pi < pattern.len() && (pattern[pi] == '?' || pattern[pi] == text[ti]) {
            ti += 1;
            pi += 1;
        } else if pi < pattern.len() && pattern[pi] == '*' {
            star_pi = Some(pi);
            star_ti = ti;
            pi += 1;
        } else if let Some(sp) = star_pi {
            pi = sp + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }

    while pi < pattern.len() && pattern[pi] == '*' {
        pi += 1;
    }

    pi == pattern.len()
}

/// Strip quotes from a word, returning (prefix, suffix, quote_char)
fn strip_quotes(word: &str, cursor_offset: usize) -> (String, String, Option<char>) {
    let chars: Vec<char> = word.chars().collect();

    if chars.is_empty() {
        return (String::new(), String::new(), None);
    }

    let first = chars[0];
    let last = *chars.last().unwrap();

    // Check for quotes
    let quote = match first {
        '\'' | '"' => {
            if chars.len() > 1 && last == first {
                Some(first)
            } else if chars.len() > 1 {
                Some(first)
            } else {
                None
            }
        }
        '$' if chars.len() > 1 && chars[1] == '\'' => Some('$'),
        _ => None,
    };

    if let Some(q) = quote {
        let start = if q == '$' { 2 } else { 1 };
        let end = if chars.len() > start
            && chars.last() == Some(&q.to_string().chars().last().unwrap_or(q))
        {
            chars.len() - 1
        } else {
            chars.len()
        };

        let inner: String = chars[start..end].iter().collect();
        let adj_offset = cursor_offset.saturating_sub(start);
        let prefix: String = inner.chars().take(adj_offset).collect();
        let suffix: String = inner.chars().skip(adj_offset).collect();

        (prefix, suffix, Some(q))
    } else {
        let prefix: String = chars.iter().take(cursor_offset).collect();
        let suffix: String = chars.iter().skip(cursor_offset).collect();
        (prefix, suffix, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_prefix_num() {
        let args = vec!["-p".to_string(), "3".to_string()];
        let op = CompsetOp::parse(&args).unwrap();
        assert_eq!(op, CompsetOp::PrefixNum(3));
    }

    #[test]
    fn test_parse_prefix_pat() {
        let args = vec!["-P".to_string(), "*:".to_string()];
        let op = CompsetOp::parse(&args).unwrap();
        assert_eq!(op, CompsetOp::PrefixPat("*:".to_string()));
    }

    #[test]
    fn test_parse_pasted() {
        let args = vec!["-p5".to_string()];
        let op = CompsetOp::parse(&args).unwrap();
        assert_eq!(op, CompsetOp::PrefixNum(5));
    }

    #[test]
    fn test_execute_prefix_num() {
        let mut params = CompParams::new();
        params.prefix = "foobar".to_string();

        let op = CompsetOp::PrefixNum(3);
        let result = compset_execute(&op, &mut params);

        assert_eq!(result, 0);
        assert_eq!(params.prefix, "bar");
        assert_eq!(params.iprefix, "foo");
    }

    #[test]
    fn test_execute_suffix_num() {
        let mut params = CompParams::new();
        params.suffix = "foobar".to_string();

        let op = CompsetOp::SuffixNum(3);
        let result = compset_execute(&op, &mut params);

        assert_eq!(result, 0);
        assert_eq!(params.suffix, "foo");
        assert_eq!(params.isuffix, "bar");
    }

    #[test]
    fn test_glob_match() {
        assert!(glob_match("foobar", "foo*"));
        assert!(glob_match("foobar", "*bar"));
        assert!(glob_match("foobar", "f?ob?r"));
        assert!(glob_match("foobar", "*"));
        assert!(!glob_match("foobar", "baz*"));
    }

    #[test]
    fn test_prefix_pattern() {
        assert_eq!(
            match_prefix_pattern("user:host:path", "*:"),
            Some("user:".to_string())
        );
    }
}

// =============================================================================
// compquote builtin
// =============================================================================

/// compquote options
#[derive(Clone, Debug, Default)]
pub struct CompquoteOpts {
    /// -p: quote special characters in PREFIX/SUFFIX for patterns
    pub pattern: bool,
}

impl CompquoteOpts {
    pub fn parse(args: &[String]) -> Result<(Self, Vec<String>), String> {
        let mut opts = Self::default();
        let mut names = Vec::new();
        let mut i = 0;

        while i < args.len() {
            let arg = &args[i];
            if arg == "-p" {
                opts.pattern = true;
            } else if arg.starts_with('-') {
                return Err(format!("compquote: unknown option: {}", arg));
            } else {
                names.push(arg.clone());
            }
            i += 1;
        }

        Ok((opts, names))
    }
}

/// Execute compquote builtin
///
/// compquote quotes special characters in the values of named parameters
/// so they can be used as completion matches. This is needed when the
/// values contain characters that would be interpreted by the shell.
pub fn compquote_execute(params: &mut CompParams, opts: &CompquoteOpts, names: &[String]) -> bool {
    for name in names {
        match name.as_str() {
            "PREFIX" => {
                params.prefix = quote_for_completion(&params.prefix, opts.pattern);
            }
            "SUFFIX" => {
                params.suffix = quote_for_completion(&params.suffix, opts.pattern);
            }
            "IPREFIX" => {
                params.iprefix = quote_for_completion(&params.iprefix, opts.pattern);
            }
            "ISUFFIX" => {
                params.isuffix = quote_for_completion(&params.isuffix, opts.pattern);
            }
            _ => {
                // For other parameters, we'd need access to the shell's parameter table
                // In practice, compquote is mainly used for PREFIX/SUFFIX
            }
        }
    }
    true
}

/// Quote special characters for use in completion
fn quote_for_completion(s: &str, for_pattern: bool) -> String {
    let mut result = String::with_capacity(s.len() * 2);

    for c in s.chars() {
        if for_pattern {
            // Quote pattern metacharacters: * ? [ ] # ~ ^
            match c {
                '*' | '?' | '[' | ']' | '#' | '~' | '^' => {
                    result.push('\\');
                }
                _ => {}
            }
        }
        // Always quote shell metacharacters: \ ' " $ ` ! { } ( ) | & ; < > space tab newline
        match c {
            '\\' | '\'' | '"' | '$' | '`' | '!' | '{' | '}' | '(' | ')' | '|' | '&' | ';' | '<'
            | '>' | ' ' | '\t' | '\n' => {
                result.push('\\');
            }
            _ => {}
        }
        result.push(c);
    }

    result
}

// =============================================================================
// compcall builtin (compctl compatibility)
// =============================================================================

/// compcall options
#[derive(Clone, Debug, Default)]
pub struct CompcallOpts {
    /// -T: use completion for command specified by words[1] (like -T flag to compctl)
    pub use_command: bool,
    /// -D: use default completion
    pub use_default: bool,
}

impl CompcallOpts {
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut opts = Self::default();

        for arg in args {
            match arg.as_str() {
                "-T" => opts.use_command = true,
                "-D" => opts.use_default = true,
                _ if arg.starts_with('-') => {
                    return Err(format!("compcall: unknown option: {}", arg));
                }
                _ => {}
            }
        }

        Ok(opts)
    }
}

/// Execute compcall builtin
///
/// compcall allows calling completions defined with the old compctl builtin
/// from within a completion widget. This provides backward compatibility.
///
/// Returns true if completions were added, false otherwise.
pub fn compcall_execute(_params: &CompParams, _opts: &CompcallOpts) -> bool {
    // compcall invokes the old compctl completion system
    // For now, we return false as we don't support compctl
    // In a full implementation, this would:
    // 1. Look up the compctl definition for the current command
    // 2. Generate completions according to that definition
    // 3. Add them via compadd
    false
}
