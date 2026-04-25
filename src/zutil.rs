//! Zsh utility builtins - port of Modules/zutil.c
//!
//! Provides zstyle, zformat, zparseopts builtins.

use regex::Regex;
use std::collections::HashMap;

/// Style pattern with associated values
#[derive(Debug, Clone)]
pub struct StylePattern {
    pub pattern: String,
    pub weight: u64,
    pub values: Vec<String>,
    pub eval: bool,
}

impl StylePattern {
    pub fn new(pattern: &str, values: Vec<String>, eval: bool) -> Self {
        let weight = Self::calculate_weight(pattern);
        Self {
            pattern: pattern.to_string(),
            weight,
            values,
            eval,
        }
    }

    fn calculate_weight(pattern: &str) -> u64 {
        let mut weight: u64 = 0;
        let mut tmp = 2u64;
        let mut first = true;

        for ch in pattern.chars() {
            if first && ch == '*' {
                tmp = 0;
                continue;
            }
            first = false;

            if ch == '('
                || ch == '|'
                || ch == '*'
                || ch == '['
                || ch == '<'
                || ch == '?'
                || ch == '#'
                || ch == '^'
            {
                tmp = 1;
            }

            if ch == ':' {
                weight += 1 << 32;
                first = true;
                weight += tmp;
                tmp = 2;
            }
        }
        weight + tmp
    }

    pub fn matches(&self, context: &str) -> bool {
        if self.pattern == "*" {
            return true;
        }

        let regex_pattern = glob_to_regex(&self.pattern);
        if let Ok(re) = Regex::new(&regex_pattern) {
            re.is_match(context)
        } else {
            self.pattern == context
        }
    }
}

fn glob_to_regex(pattern: &str) -> String {
    let mut result = String::from("^");
    for ch in pattern.chars() {
        match ch {
            '*' => result.push_str(".*"),
            '?' => result.push('.'),
            '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' | '\\' => {
                result.push('\\');
                result.push(ch);
            }
            _ => result.push(ch),
        }
    }
    result.push('$');
    result
}

/// Style storage - maps style names to patterns
#[derive(Debug, Default)]
pub struct StyleTable {
    styles: HashMap<String, Vec<StylePattern>>,
}

impl StyleTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, pattern: &str, style: &str, values: Vec<String>, eval: bool) {
        let style_patterns = self.styles.entry(style.to_string()).or_default();

        if let Some(existing) = style_patterns.iter_mut().find(|p| p.pattern == pattern) {
            existing.values = values;
            existing.eval = eval;
        } else {
            let sp = StylePattern::new(pattern, values, eval);
            let weight = sp.weight;
            let pos = style_patterns
                .iter()
                .position(|p| p.weight < weight)
                .unwrap_or(style_patterns.len());
            style_patterns.insert(pos, sp);
        }
    }

    pub fn get(&self, context: &str, style: &str) -> Option<&[String]> {
        self.styles.get(style).and_then(|patterns| {
            patterns
                .iter()
                .find(|p| p.matches(context))
                .map(|p| p.values.as_slice())
        })
    }

    pub fn delete(&mut self, pattern: Option<&str>, style: Option<&str>) {
        match (pattern, style) {
            (None, None) => self.styles.clear(),
            (Some(pat), None) => {
                for patterns in self.styles.values_mut() {
                    patterns.retain(|p| p.pattern != pat);
                }
                self.styles.retain(|_, v| !v.is_empty());
            }
            (Some(pat), Some(sty)) => {
                if let Some(patterns) = self.styles.get_mut(sty) {
                    patterns.retain(|p| p.pattern != pat);
                    if patterns.is_empty() {
                        self.styles.remove(sty);
                    }
                }
            }
            (None, Some(sty)) => {
                self.styles.remove(sty);
            }
        }
    }

    pub fn list(&self, context: Option<&str>) -> Vec<(String, String, Vec<String>)> {
        let mut result = Vec::new();
        for (style, patterns) in &self.styles {
            for pat in patterns {
                if let Some(ctx) = context {
                    if !pat.matches(ctx) {
                        continue;
                    }
                }
                result.push((style.clone(), pat.pattern.clone(), pat.values.clone()));
            }
        }
        result
    }

    pub fn list_styles(&self) -> Vec<&str> {
        self.styles.keys().map(|s| s.as_str()).collect()
    }

    pub fn list_patterns(&self) -> Vec<&str> {
        let mut patterns = Vec::new();
        for pats in self.styles.values() {
            for pat in pats {
                if !patterns.contains(&pat.pattern.as_str()) {
                    patterns.push(pat.pattern.as_str());
                }
            }
        }
        patterns
    }

    pub fn test(&self, context: &str, style: &str, values: Option<&[&str]>) -> bool {
        if let Some(found) = self.get(context, style) {
            if let Some(test_vals) = values {
                test_vals.iter().any(|v| found.contains(&v.to_string()))
            } else {
                matches!(
                    found.first().map(|s| s.as_str()),
                    Some("true" | "yes" | "on" | "1")
                )
            }
        } else {
            false
        }
    }

    pub fn test_bool(&self, context: &str, style: &str) -> Option<bool> {
        self.get(context, style).and_then(|vals| {
            if vals.len() == 1 {
                match vals[0].as_str() {
                    "yes" | "true" | "on" | "1" => Some(true),
                    "no" | "false" | "off" | "0" => Some(false),
                    _ => None,
                }
            } else {
                None
            }
        })
    }
}

/// Format a string with specifications
pub fn zformat(format: &str, specs: &HashMap<char, String>, _presence: bool) -> String {
    let mut result = String::new();
    let mut chars = format.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '%' {
            let mut right = false;
            let mut min: Option<usize> = None;
            let mut max: Option<usize> = None;

            if chars.peek() == Some(&'-') {
                right = true;
                chars.next();
            }

            let mut num_str = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() {
                    num_str.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
            if !num_str.is_empty() {
                min = num_str.parse().ok();
            }

            if chars.peek() == Some(&'.') || chars.peek() == Some(&'(') {
                let is_ternary = chars.peek() == Some(&'(');
                if !is_ternary {
                    chars.next();
                }

                let mut max_str = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_ascii_digit() {
                        max_str.push(c);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if !max_str.is_empty() {
                    max = max_str.parse().ok();
                }
            }

            if let Some(&spec_char) = chars.peek() {
                chars.next();

                if spec_char == '(' {
                    continue;
                }

                if let Some(spec_val) = specs.get(&spec_char) {
                    let mut val = spec_val.clone();

                    if let Some(m) = max {
                        if val.len() > m {
                            val.truncate(m);
                        }
                    }

                    let out_len = min.map(|m| m.max(val.len())).unwrap_or(val.len());

                    if val.len() >= out_len {
                        result.push_str(&val[..out_len]);
                    } else {
                        let padding = out_len - val.len();
                        if right {
                            result.push_str(&" ".repeat(padding));
                            result.push_str(&val);
                        } else {
                            result.push_str(&val);
                            result.push_str(&" ".repeat(padding));
                        }
                    }
                } else if spec_char == '%' {
                    result.push('%');
                }
            }
        } else {
            result.push(ch);
        }
    }

    result
}

/// Option description for zparseopts
#[derive(Debug, Clone)]
pub struct OptDesc {
    pub name: String,
    pub takes_arg: bool,
    pub optional_arg: bool,
    pub multiple: bool,
    pub array_name: Option<String>,
}

impl OptDesc {
    pub fn parse(spec: &str) -> Option<Self> {
        if spec.is_empty() {
            return None;
        }

        let mut name = String::new();
        let mut takes_arg = false;
        let mut optional_arg = false;
        let mut multiple = false;
        let mut array_name = None;
        let mut chars = spec.chars().peekable();

        while let Some(&ch) = chars.peek() {
            if ch == '+' {
                multiple = true;
                chars.next();
                break;
            } else if ch == ':' || ch == '=' {
                break;
            } else if ch == '\\' {
                chars.next();
                if let Some(c) = chars.next() {
                    name.push(c);
                }
            } else {
                name.push(ch);
                chars.next();
            }
        }

        if name.is_empty() {
            return None;
        }

        if chars.peek() == Some(&':') {
            takes_arg = true;
            chars.next();
            if chars.peek() == Some(&':') {
                optional_arg = true;
                chars.next();
            }
        }

        if chars.peek() == Some(&'=') {
            chars.next();
            array_name = Some(chars.collect());
        }

        Some(Self {
            name,
            takes_arg,
            optional_arg,
            multiple,
            array_name,
        })
    }
}

/// Parse options from arguments
pub fn zparseopts(
    args: &[String],
    specs: &[OptDesc],
    delete: bool,
    extract: bool,
) -> Result<(HashMap<String, Vec<String>>, Vec<String>), String> {
    let mut results: HashMap<String, Vec<String>> = HashMap::new();
    let mut remaining = Vec::new();
    let mut i = 0;

    let short_opts: HashMap<char, &OptDesc> = specs
        .iter()
        .filter(|s| s.name.len() == 1)
        .map(|s| (s.name.chars().next().unwrap(), s))
        .collect();

    let long_opts: HashMap<&str, &OptDesc> = specs
        .iter()
        .filter(|s| s.name.len() > 1)
        .map(|s| (s.name.as_str(), s))
        .collect();

    while i < args.len() {
        let arg = &args[i];

        if !arg.starts_with('-') || arg == "-" {
            if extract {
                if !delete {
                    remaining.push(arg.clone());
                }
                i += 1;
                continue;
            } else {
                remaining.extend(args[i..].iter().cloned());
                break;
            }
        }

        if arg == "--" {
            i += 1;
            remaining.extend(args[i..].iter().cloned());
            break;
        }

        let opt_str = &arg[1..];

        if let Some(desc) = long_opts.get(opt_str) {
            let key = format!("-{}", desc.name);
            let entry = results.entry(key).or_default();

            if desc.takes_arg {
                if i + 1 < args.len() && !desc.optional_arg {
                    i += 1;
                    entry.push(args[i].clone());
                } else if desc.optional_arg {
                    entry.push(String::new());
                } else {
                    return Err(format!("missing argument for option: -{}", desc.name));
                }
            } else {
                entry.push(String::new());
            }
        } else if opt_str.starts_with('-') {
            let long_name = &opt_str[1..];
            if let Some((name, value)) = long_name.split_once('=') {
                if let Some(desc) = long_opts.get(name) {
                    let key = format!("-{}", desc.name);
                    results.entry(key).or_default().push(value.to_string());
                } else {
                    if !extract {
                        remaining.extend(args[i..].iter().cloned());
                        break;
                    }
                    remaining.push(arg.clone());
                }
            } else if let Some(desc) = long_opts.get(long_name) {
                let key = format!("-{}", desc.name);
                let entry = results.entry(key).or_default();

                if desc.takes_arg {
                    if i + 1 < args.len() && !desc.optional_arg {
                        i += 1;
                        entry.push(args[i].clone());
                    } else if desc.optional_arg {
                        entry.push(String::new());
                    } else {
                        return Err(format!("missing argument for option: --{}", desc.name));
                    }
                } else {
                    entry.push(String::new());
                }
            } else {
                if !extract {
                    remaining.extend(args[i..].iter().cloned());
                    break;
                }
                remaining.push(arg.clone());
            }
        } else {
            let mut j = 0;
            let chars: Vec<char> = opt_str.chars().collect();

            while j < chars.len() {
                let ch = chars[j];
                if let Some(desc) = short_opts.get(&ch) {
                    let key = format!("-{}", desc.name);
                    let entry = results.entry(key).or_default();

                    if desc.takes_arg {
                        if j + 1 < chars.len() {
                            entry.push(chars[j + 1..].iter().collect());
                            break;
                        } else if i + 1 < args.len() && !desc.optional_arg {
                            i += 1;
                            entry.push(args[i].clone());
                        } else if desc.optional_arg {
                            entry.push(String::new());
                        } else {
                            return Err(format!("missing argument for option: -{}", desc.name));
                        }
                    } else {
                        entry.push(String::new());
                    }
                } else {
                    if !extract {
                        remaining.push(arg.clone());
                        remaining.extend(args[i + 1..].iter().cloned());
                        return Ok((results, remaining));
                    }
                    break;
                }
                j += 1;
            }
        }
        i += 1;
    }

    if !delete && !extract {
        remaining = args[i..].to_vec();
    }

    Ok((results, remaining))
}

/// Align array values with a separator
pub fn zformat_align(sep: &str, values: &[&str]) -> Vec<String> {
    let mut max_pre = 0;

    for value in values {
        if let Some(pos) = value.find(':') {
            let pre_len = value[..pos].chars().filter(|c| *c != '\\').count();
            if pre_len > max_pre {
                max_pre = pre_len;
            }
        }
    }

    let mut result = Vec::new();
    for value in values {
        if let Some(pos) = value.find(':') {
            let pre = &value[..pos];
            let post = &value[pos + 1..];
            let pre_len = pre.chars().filter(|c| *c != '\\').count();
            let padding = max_pre - pre_len;

            let clean_pre: String = pre.chars().filter(|c| *c != '\\').collect();

            result.push(format!(
                "{}{}{}{}",
                clean_pre,
                " ".repeat(padding),
                sep,
                post
            ));
        } else {
            result.push(value.to_string());
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_style_pattern_weight() {
        let p1 = StylePattern::new("*", vec![], false);
        let p2 = StylePattern::new(":completion:*", vec![], false);
        let p3 = StylePattern::new(":completion:zsh:*", vec![], false);

        assert!(p3.weight > p2.weight);
        assert!(p2.weight > p1.weight);
    }

    #[test]
    fn test_style_pattern_matches() {
        let p = StylePattern::new(":completion:*", vec![], false);
        assert!(p.matches(":completion:zsh:complete"));
        assert!(!p.matches(":other:zsh"));

        let p2 = StylePattern::new("*", vec![], false);
        assert!(p2.matches("anything"));
    }

    #[test]
    fn test_style_table_set_get() {
        let mut table = StyleTable::new();
        table.set(":completion:*", "verbose", vec!["yes".to_string()], false);

        let result = table.get(":completion:zsh", "verbose");
        assert_eq!(result, Some(&["yes".to_string()][..]));

        let result = table.get(":other", "verbose");
        assert!(result.is_none());
    }

    #[test]
    fn test_style_table_priority() {
        let mut table = StyleTable::new();
        table.set("*", "menu", vec!["no".to_string()], false);
        table.set(":completion:*", "menu", vec!["yes".to_string()], false);

        let result = table.get(":completion:zsh", "menu");
        assert_eq!(result, Some(&["yes".to_string()][..]));
    }

    #[test]
    fn test_style_table_delete() {
        let mut table = StyleTable::new();
        table.set("*", "style1", vec!["val".to_string()], false);
        table.set("*", "style2", vec!["val".to_string()], false);

        table.delete(None, Some("style1"));
        assert!(table.get("test", "style1").is_none());
        assert!(table.get("test", "style2").is_some());
    }

    #[test]
    fn test_style_test_bool() {
        let mut table = StyleTable::new();
        table.set("*", "enabled", vec!["yes".to_string()], false);
        table.set("*", "disabled", vec!["no".to_string()], false);
        table.set(
            "*",
            "multiple",
            vec!["a".to_string(), "b".to_string()],
            false,
        );

        assert_eq!(table.test_bool("ctx", "enabled"), Some(true));
        assert_eq!(table.test_bool("ctx", "disabled"), Some(false));
        assert_eq!(table.test_bool("ctx", "multiple"), None);
    }

    #[test]
    fn test_zformat_basic() {
        let mut specs = HashMap::new();
        specs.insert('n', "test".to_string());
        specs.insert('v', "42".to_string());

        let result = zformat("Name: %n, Value: %v", &specs, false);
        assert_eq!(result, "Name: test, Value: 42");
    }

    #[test]
    fn test_zformat_padding() {
        let mut specs = HashMap::new();
        specs.insert('n', "hi".to_string());

        let result = zformat("[%10n]", &specs, false);
        assert_eq!(result, "[hi        ]");

        let result = zformat("[%-10n]", &specs, false);
        assert_eq!(result, "[        hi]");
    }

    #[test]
    fn test_zformat_truncate() {
        let mut specs = HashMap::new();
        specs.insert('n', "hello world".to_string());

        let result = zformat("[%.5n]", &specs, false);
        assert_eq!(result, "[hello]");
    }

    #[test]
    fn test_zformat_escape() {
        let specs = HashMap::new();
        let result = zformat("100%%", &specs, false);
        assert_eq!(result, "100%");
    }

    #[test]
    fn test_opt_desc_parse() {
        let desc = OptDesc::parse("v").unwrap();
        assert_eq!(desc.name, "v");
        assert!(!desc.takes_arg);

        let desc = OptDesc::parse("o:").unwrap();
        assert_eq!(desc.name, "o");
        assert!(desc.takes_arg);
        assert!(!desc.optional_arg);

        let desc = OptDesc::parse("o::").unwrap();
        assert!(desc.optional_arg);

        let desc = OptDesc::parse("v+").unwrap();
        assert!(desc.multiple);

        let desc = OptDesc::parse("a:=myarray").unwrap();
        assert_eq!(desc.array_name, Some("myarray".to_string()));
    }

    #[test]
    fn test_zparseopts_basic() {
        let specs = vec![OptDesc::parse("v").unwrap(), OptDesc::parse("o:").unwrap()];

        let args: Vec<String> = vec!["-v", "-o", "value", "rest"]
            .into_iter()
            .map(String::from)
            .collect();

        let (opts, remaining) = zparseopts(&args, &specs, false, false).unwrap();

        assert!(opts.contains_key("-v"));
        assert_eq!(opts.get("-o"), Some(&vec!["value".to_string()]));
        assert_eq!(remaining, vec!["rest"]);
    }

    #[test]
    fn test_zparseopts_combined() {
        let specs = vec![
            OptDesc::parse("a").unwrap(),
            OptDesc::parse("b").unwrap(),
            OptDesc::parse("c:").unwrap(),
        ];

        let args: Vec<String> = vec!["-abc", "val"].into_iter().map(String::from).collect();

        let (opts, _) = zparseopts(&args, &specs, false, false).unwrap();

        assert!(opts.contains_key("-a"));
        assert!(opts.contains_key("-b"));
        assert_eq!(opts.get("-c"), Some(&vec!["val".to_string()]));
    }

    #[test]
    fn test_zparseopts_long() {
        let specs = vec![
            OptDesc::parse("verbose").unwrap(),
            OptDesc::parse("output:").unwrap(),
        ];

        let args: Vec<String> = vec!["--verbose", "--output", "file.txt"]
            .into_iter()
            .map(String::from)
            .collect();

        let (opts, _) = zparseopts(&args, &specs, false, false).unwrap();

        assert!(opts.contains_key("-verbose"));
        assert_eq!(opts.get("-output"), Some(&vec!["file.txt".to_string()]));
    }

    #[test]
    fn test_zformat_align() {
        let values = vec!["short:desc1", "verylongname:desc2", "med:desc3"];
        let result = zformat_align(" -- ", &values);

        assert_eq!(result[0], "short        -- desc1");
        assert_eq!(result[1], "verylongname -- desc2");
        assert_eq!(result[2], "med          -- desc3");
    }
}
