//! Substitution handling for zshrs
//!
//! Port from zsh/Src/subst.c
//!
//! Provides parameter expansion, command substitution, arithmetic expansion,
//! brace expansion, tilde expansion, and filename generation.

use std::collections::HashMap;
use std::env;
use std::process::{Command, Stdio};

/// Prefork flags
pub mod prefork {
    pub const SINGLE: u32 = 1; // Single word expected
    pub const SPLIT: u32 = 2; // Force word splitting
    pub const SHWORDSPLIT: u32 = 4; // sh-style word splitting
    pub const NOSHWORDSPLIT: u32 = 8; // Disable word splitting
    pub const ASSIGN: u32 = 16; // Assignment context
    pub const TYPESET: u32 = 32; // Typeset context
    pub const SUBEXP: u32 = 64; // Subexpression
    pub const KEY_VALUE: u32 = 128; // Key-value pair found
}

/// Perform all substitutions on a word
pub fn subst_string(
    s: &str,
    params: &HashMap<String, String>,
    opts: &SubstOptions,
) -> Result<String, String> {
    let mut result = s.to_string();

    // Tilde expansion
    result = tilde_expand(&result, opts)?;

    // Parameter expansion
    result = param_expand(&result, params, opts)?;

    // Command substitution
    result = command_subst(&result, opts)?;

    // Arithmetic expansion
    result = arith_expand(&result, params, opts)?;

    Ok(result)
}

/// Substitution options
#[derive(Clone, Debug, Default)]
pub struct SubstOptions {
    pub noglob: bool,
    pub noexec: bool,
    pub nounset: bool,
    pub word_split: bool,
    pub ignore_braces: bool,
}

/// Tilde expansion
pub fn tilde_expand(s: &str, _opts: &SubstOptions) -> Result<String, String> {
    if !s.starts_with('~') {
        return Ok(s.to_string());
    }

    let rest = &s[1..];

    // Find end of username
    let (user, suffix) = match rest.find('/') {
        Some(pos) => (&rest[..pos], &rest[pos..]),
        None => (rest, ""),
    };

    let expanded = if user.is_empty() {
        // ~ alone means $HOME
        env::var("HOME").unwrap_or_else(|_| "/".to_string())
    } else if user.starts_with('+') {
        // ~+ means $PWD
        env::var("PWD").unwrap_or_else(|_| ".".to_string())
    } else if user.starts_with('-') {
        // ~- means $OLDPWD
        env::var("OLDPWD").unwrap_or_else(|_| ".".to_string())
    } else {
        // ~user means user's home directory
        #[cfg(unix)]
        {
            get_user_home(user).unwrap_or_else(|| format!("~{}", user))
        }
        #[cfg(not(unix))]
        {
            format!("~{}", user)
        }
    };

    Ok(format!("{}{}", expanded, suffix))
}

#[cfg(unix)]
fn get_user_home(user: &str) -> Option<String> {
    use std::ffi::CString;
    unsafe {
        let c_user = CString::new(user).ok()?;
        let pw = libc::getpwnam(c_user.as_ptr());
        if pw.is_null() {
            return None;
        }
        let dir = std::ffi::CStr::from_ptr((*pw).pw_dir);
        dir.to_str().ok().map(|s| s.to_string())
    }
}

/// Parameter expansion
pub fn param_expand(
    s: &str,
    params: &HashMap<String, String>,
    opts: &SubstOptions,
) -> Result<String, String> {
    let mut result = String::new();
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '$' {
            match chars.peek() {
                Some(&'{') => {
                    // ${...} form
                    chars.next();
                    let expanded = parse_brace_param(&mut chars, params, opts)?;
                    result.push_str(&expanded);
                }
                Some(&'(') => {
                    // $(...) command substitution or $((...)) arithmetic
                    chars.next();
                    if chars.peek() == Some(&'(') {
                        // $((...)) arithmetic
                        chars.next();
                        let expr = collect_until(&mut chars, ')');
                        if chars.next() != Some(')') {
                            return Err("Missing )) in arithmetic expansion".to_string());
                        }
                        let value = eval_arith(&expr, params)?;
                        result.push_str(&value.to_string());
                    } else {
                        // $(...) command substitution
                        let cmd = collect_balanced(&mut chars, '(', ')');
                        if !opts.noexec {
                            let output = run_command(&cmd)?;
                            result.push_str(output.trim_end_matches('\n'));
                        }
                    }
                }
                Some(&c) if c.is_ascii_alphabetic() || c == '_' => {
                    // Simple $var - consume first char we peeked
                    chars.next();
                    let name = collect_varname(&mut chars);
                    let full_name = format!("{}{}", c, name);

                    if let Some(value) = params.get(&full_name) {
                        result.push_str(value);
                    } else if let Ok(value) = env::var(&full_name) {
                        result.push_str(&value);
                    } else if opts.nounset {
                        return Err(format!("{}: parameter not set", full_name));
                    }
                }
                Some(&c) if c.is_ascii_digit() => {
                    // Positional parameter
                    let mut num = String::new();
                    while let Some(&c) = chars.peek() {
                        if c.is_ascii_digit() {
                            num.push(chars.next().unwrap());
                        } else {
                            break;
                        }
                    }
                    // Positional params would be looked up here
                }
                Some(&'?') => {
                    chars.next();
                    result.push_str("0"); // Last exit status
                }
                Some(&'$') => {
                    chars.next();
                    result.push_str(&std::process::id().to_string());
                }
                Some(&'#') => {
                    chars.next();
                    result.push_str("0"); // Number of positional params
                }
                Some(&'*') | Some(&'@') => {
                    chars.next();
                    // All positional params
                }
                _ => result.push('$'),
            }
        } else {
            result.push(c);
        }
    }

    Ok(result)
}

fn parse_brace_param(
    chars: &mut std::iter::Peekable<std::str::Chars>,
    params: &HashMap<String, String>,
    _opts: &SubstOptions,
) -> Result<String, String> {
    let mut name = String::new();
    let mut operator = None;
    let mut operand = String::new();

    // Check for special prefix operators
    let prefix = match chars.peek() {
        Some(&'#') => {
            chars.next();
            if chars.peek() == Some(&'}') {
                // ${#} - number of params
                chars.next();
                return Ok("0".to_string());
            }
            Some('#') // Length operator
        }
        Some(&'!') => {
            chars.next();
            Some('!') // Indirect expansion
        }
        _ => None,
    };

    // Collect variable name
    while let Some(&c) = chars.peek() {
        if c.is_ascii_alphanumeric() || c == '_' {
            name.push(chars.next().unwrap());
        } else {
            break;
        }
    }

    // Check for operators
    match chars.peek() {
        Some(&':') => {
            chars.next();
            match chars.peek() {
                Some(&'-') => {
                    chars.next();
                    operator = Some(":-");
                }
                Some(&'=') => {
                    chars.next();
                    operator = Some(":=");
                }
                Some(&'+') => {
                    chars.next();
                    operator = Some(":+");
                }
                Some(&'?') => {
                    chars.next();
                    operator = Some(":?");
                }
                _ => operator = Some(":"),
            }
        }
        Some(&'-') => {
            chars.next();
            operator = Some("-");
        }
        Some(&'=') => {
            chars.next();
            operator = Some("=");
        }
        Some(&'+') => {
            chars.next();
            operator = Some("+");
        }
        Some(&'?') => {
            chars.next();
            operator = Some("?");
        }
        Some(&'#') => {
            chars.next();
            operator = Some("#");
        }
        Some(&'%') => {
            chars.next();
            operator = Some("%");
        }
        Some(&'/') => {
            chars.next();
            operator = Some("/");
        }
        Some(&'^') => {
            chars.next();
            operator = Some("^");
        }
        Some(&',') => {
            chars.next();
            operator = Some(",");
        }
        _ => {}
    }

    // Collect operand until closing brace
    let mut depth = 1;
    while depth > 0 {
        match chars.next() {
            Some('{') => depth += 1,
            Some('}') => depth -= 1,
            Some(c) if depth > 0 => operand.push(c),
            None => return Err("Missing } in parameter expansion".to_string()),
            _ => {}
        }
    }

    // Get the value
    let value = params.get(&name).cloned().or_else(|| env::var(&name).ok());

    // Handle prefix operators
    if let Some('#') = prefix {
        // ${#var} - length
        return Ok(value.map(|v| v.len()).unwrap_or(0).to_string());
    }

    // Apply operator
    match operator {
        Some(":-") | Some("-") => {
            if value.as_ref().map(|v| v.is_empty()).unwrap_or(true) {
                Ok(operand)
            } else {
                Ok(value.unwrap_or_default())
            }
        }
        Some(":+") | Some("+") => {
            if value.as_ref().map(|v| !v.is_empty()).unwrap_or(false) {
                Ok(operand)
            } else {
                Ok(String::new())
            }
        }
        Some(":?") | Some("?") => {
            if value.as_ref().map(|v| v.is_empty()).unwrap_or(true) {
                Err(if operand.is_empty() {
                    format!("{}: parameter null or not set", name)
                } else {
                    operand
                })
            } else {
                Ok(value.unwrap_or_default())
            }
        }
        Some("#") => {
            // Remove shortest prefix
            if let Some(v) = value {
                Ok(remove_prefix(&v, &operand, false))
            } else {
                Ok(String::new())
            }
        }
        Some("##") => {
            // Remove longest prefix
            if let Some(v) = value {
                Ok(remove_prefix(&v, &operand, true))
            } else {
                Ok(String::new())
            }
        }
        Some("%") => {
            // Remove shortest suffix
            if let Some(v) = value {
                Ok(remove_suffix(&v, &operand, false))
            } else {
                Ok(String::new())
            }
        }
        Some("%%") => {
            // Remove longest suffix
            if let Some(v) = value {
                Ok(remove_suffix(&v, &operand, true))
            } else {
                Ok(String::new())
            }
        }
        Some("^") => {
            // Uppercase first char
            if let Some(v) = value {
                let mut c = v.chars();
                match c.next() {
                    Some(first) => Ok(first.to_uppercase().collect::<String>() + c.as_str()),
                    None => Ok(String::new()),
                }
            } else {
                Ok(String::new())
            }
        }
        Some("^^") => {
            // Uppercase all
            Ok(value.map(|v| v.to_uppercase()).unwrap_or_default())
        }
        Some(",") => {
            // Lowercase first char
            if let Some(v) = value {
                let mut c = v.chars();
                match c.next() {
                    Some(first) => Ok(first.to_lowercase().collect::<String>() + c.as_str()),
                    None => Ok(String::new()),
                }
            } else {
                Ok(String::new())
            }
        }
        Some(",,") => {
            // Lowercase all
            Ok(value.map(|v| v.to_lowercase()).unwrap_or_default())
        }
        Some("/") => {
            // Substitution
            if let Some(v) = value {
                let parts: Vec<&str> = operand.splitn(2, '/').collect();
                if parts.len() == 2 {
                    Ok(v.replacen(parts[0], parts[1], 1))
                } else {
                    Ok(v.replacen(parts[0], "", 1))
                }
            } else {
                Ok(String::new())
            }
        }
        Some("//") => {
            // Global substitution
            if let Some(v) = value {
                let parts: Vec<&str> = operand.splitn(2, '/').collect();
                if parts.len() == 2 {
                    Ok(v.replace(parts[0], parts[1]))
                } else {
                    Ok(v.replace(parts[0], ""))
                }
            } else {
                Ok(String::new())
            }
        }
        _ => Ok(value.unwrap_or_default()),
    }
}

fn remove_prefix(s: &str, pattern: &str, greedy: bool) -> String {
    // Simple glob-less version
    if greedy {
        for i in (0..=s.len()).rev() {
            if s[..i].ends_with(pattern) || (pattern == "*" && i > 0) {
                return s[i..].to_string();
            }
        }
    } else {
        for i in 0..=s.len() {
            if s[..i].ends_with(pattern) || (pattern == "*" && i > 0) {
                return s[i..].to_string();
            }
        }
    }
    s.to_string()
}

fn remove_suffix(s: &str, pattern: &str, greedy: bool) -> String {
    // Simple glob-less version
    if greedy {
        for i in 0..=s.len() {
            if s[i..].starts_with(pattern) || (pattern == "*" && i < s.len()) {
                return s[..i].to_string();
            }
        }
    } else {
        for i in (0..=s.len()).rev() {
            if s[i..].starts_with(pattern) || (pattern == "*" && i < s.len()) {
                return s[..i].to_string();
            }
        }
    }
    s.to_string()
}

fn collect_varname(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
    let mut name = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_alphanumeric() || c == '_' {
            name.push(chars.next().unwrap());
        } else {
            break;
        }
    }
    name
}

fn collect_until(chars: &mut std::iter::Peekable<std::str::Chars>, end: char) -> String {
    let mut result = String::new();
    while let Some(&c) = chars.peek() {
        if c == end {
            break;
        }
        result.push(chars.next().unwrap());
    }
    result
}

fn collect_balanced(
    chars: &mut std::iter::Peekable<std::str::Chars>,
    open: char,
    close: char,
) -> String {
    let mut result = String::new();
    let mut depth = 1;

    while depth > 0 {
        match chars.next() {
            Some(c) if c == open => {
                depth += 1;
                result.push(c);
            }
            Some(c) if c == close => {
                depth -= 1;
                if depth > 0 {
                    result.push(c);
                }
            }
            Some(c) => result.push(c),
            None => break,
        }
    }

    result
}

/// Command substitution
pub fn command_subst(s: &str, opts: &SubstOptions) -> Result<String, String> {
    if opts.noexec {
        return Ok(s.to_string());
    }

    let mut result = String::new();
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '`' {
            // Backtick form
            let cmd = collect_until(&mut chars, '`');
            chars.next(); // consume closing backtick
            let output = run_command(&cmd)?;
            result.push_str(output.trim_end_matches('\n'));
        } else {
            result.push(c);
        }
    }

    Ok(result)
}

fn run_command(cmd: &str) -> Result<String, String> {
    let output = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| e.to_string())?;

    String::from_utf8(output.stdout).map_err(|e| e.to_string())
}

/// Arithmetic expansion
pub fn arith_expand(
    s: &str,
    params: &HashMap<String, String>,
    opts: &SubstOptions,
) -> Result<String, String> {
    let mut result = String::new();
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '$' && chars.peek() == Some(&'[') {
            // $[...] form
            chars.next();
            let expr = collect_until(&mut chars, ']');
            chars.next(); // consume ]

            if !opts.noexec {
                let value = eval_arith(&expr, params)?;
                result.push_str(&value.to_string());
            }
        } else {
            result.push(c);
        }
    }

    Ok(result)
}

fn eval_arith(expr: &str, _params: &HashMap<String, String>) -> Result<i64, String> {
    // Simple arithmetic evaluation
    // This would use the math module in practice
    let expr = expr.trim();

    // Handle simple integers
    if let Ok(n) = expr.parse::<i64>() {
        return Ok(n);
    }

    // Try simple expression
    if let Some(pos) = expr.find('+') {
        let left = expr[..pos]
            .trim()
            .parse::<i64>()
            .map_err(|e| e.to_string())?;
        let right = expr[pos + 1..]
            .trim()
            .parse::<i64>()
            .map_err(|e| e.to_string())?;
        return Ok(left + right);
    }
    if let Some(pos) = expr.rfind('-') {
        if pos > 0 {
            let left = expr[..pos]
                .trim()
                .parse::<i64>()
                .map_err(|e| e.to_string())?;
            let right = expr[pos + 1..]
                .trim()
                .parse::<i64>()
                .map_err(|e| e.to_string())?;
            return Ok(left - right);
        }
    }
    if let Some(pos) = expr.find('*') {
        let left = expr[..pos]
            .trim()
            .parse::<i64>()
            .map_err(|e| e.to_string())?;
        let right = expr[pos + 1..]
            .trim()
            .parse::<i64>()
            .map_err(|e| e.to_string())?;
        return Ok(left * right);
    }
    if let Some(pos) = expr.find('/') {
        let left = expr[..pos]
            .trim()
            .parse::<i64>()
            .map_err(|e| e.to_string())?;
        let right = expr[pos + 1..]
            .trim()
            .parse::<i64>()
            .map_err(|e| e.to_string())?;
        if right == 0 {
            return Err("division by zero".to_string());
        }
        return Ok(left / right);
    }

    Err(format!("Invalid arithmetic expression: {}", expr))
}

/// Brace expansion
pub fn brace_expand(s: &str) -> Vec<String> {
    if !s.contains('{') {
        return vec![s.to_string()];
    }

    let mut results = vec![String::new()];
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '{' {
            let content = collect_balanced(&mut chars, '{', '}');
            let alternatives: Vec<&str> = content.split(',').collect();

            if alternatives.len() > 1 {
                results = results
                    .iter()
                    .flat_map(|prefix| {
                        alternatives
                            .iter()
                            .map(|alt| format!("{}{}", prefix, alt))
                            .collect::<Vec<_>>()
                    })
                    .collect();
            } else if let Some((start, end)) = parse_range(&content) {
                results = results
                    .iter()
                    .flat_map(|prefix| {
                        (start..=end)
                            .map(|n| format!("{}{}", prefix, n))
                            .collect::<Vec<_>>()
                    })
                    .collect();
            } else {
                for r in &mut results {
                    r.push('{');
                    r.push_str(&content);
                    r.push('}');
                }
            }
        } else {
            for r in &mut results {
                r.push(c);
            }
        }
    }

    results
}

fn parse_range(s: &str) -> Option<(i32, i32)> {
    let parts: Vec<&str> = s.splitn(2, "..").collect();
    if parts.len() != 2 {
        return None;
    }
    let start = parts[0].parse().ok()?;
    let end = parts[1].parse().ok()?;
    Some((start, end))
}

/// Remove trailing path component(s)
/// Port from remtpath() in zsh/Src/subst.c
pub fn remtpath(path: &str, count: usize) -> String {
    let mut result = path.to_string();
    for _ in 0..count {
        if let Some(pos) = result.rfind('/') {
            if pos == 0 {
                result = "/".to_string();
            } else {
                result = result[..pos].to_string();
            }
        } else {
            result = ".".to_string();
            break;
        }
    }
    result
}

/// Remove leading path component(s)
/// Port from remlpaths() in zsh/Src/subst.c
pub fn remlpaths(path: &str, count: usize) -> String {
    let mut result = path;
    for _ in 0..count {
        if let Some(pos) = result.find('/') {
            result = &result[pos + 1..];
        } else {
            return result.to_string();
        }
    }
    result.to_string()
}

/// Remove text after last dot (extension)
/// Port from remtext() in zsh/Src/subst.c
pub fn remtext(path: &str) -> String {
    if let Some(slash_pos) = path.rfind('/') {
        let filename = &path[slash_pos + 1..];
        if let Some(dot_pos) = filename.rfind('.') {
            if dot_pos > 0 {
                return format!("{}{}", &path[..=slash_pos], &filename[..dot_pos]);
            }
        }
        path.to_string()
    } else if let Some(dot_pos) = path.rfind('.') {
        if dot_pos > 0 {
            return path[..dot_pos].to_string();
        }
        path.to_string()
    } else {
        path.to_string()
    }
}

/// Remove everything but the extension
/// Port from rembutext() in zsh/Src/subst.c
pub fn rembutext(path: &str) -> String {
    let filename = if let Some(slash_pos) = path.rfind('/') {
        &path[slash_pos + 1..]
    } else {
        path
    };

    if let Some(dot_pos) = filename.rfind('.') {
        if dot_pos > 0 && dot_pos < filename.len() - 1 {
            return filename[dot_pos + 1..].to_string();
        }
    }
    String::new()
}

/// Get the tail (filename) part of a path
pub fn path_tail(path: &str) -> String {
    if let Some(pos) = path.rfind('/') {
        path[pos + 1..].to_string()
    } else {
        path.to_string()
    }
}

/// Get the head (directory) part of a path
pub fn path_head(path: &str) -> String {
    remtpath(path, 1)
}

/// Case modification modes
/// Port from CASMOD_* in zsh.h
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CaseMod {
    Lower,
    Upper,
    Caps,
}

/// Modify case of a string
/// Port from casemodify() in zsh/Src/subst.c
pub fn casemodify(s: &str, mode: CaseMod) -> String {
    match mode {
        CaseMod::Lower => s.to_lowercase(),
        CaseMod::Upper => s.to_uppercase(),
        CaseMod::Caps => {
            let mut result = String::with_capacity(s.len());
            let mut cap_next = true;
            for c in s.chars() {
                if c.is_whitespace() || !c.is_alphabetic() {
                    result.push(c);
                    cap_next = true;
                } else if cap_next {
                    for uc in c.to_uppercase() {
                        result.push(uc);
                    }
                    cap_next = false;
                } else {
                    for lc in c.to_lowercase() {
                        result.push(lc);
                    }
                }
            }
            result
        }
    }
}

/// Convert path to absolute path
/// Port from chabspath() in zsh/Src/subst.c
pub fn chabspath(path: &str) -> String {
    if path.starts_with('/') {
        return clean_path(path);
    }

    let cwd = env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "/".to_string());

    clean_path(&format!("{}/{}", cwd, path))
}

/// Clean up path by removing redundant components
fn clean_path(path: &str) -> String {
    let mut components: Vec<&str> = Vec::new();

    for part in path.split('/') {
        match part {
            "" | "." => continue,
            ".." => {
                if !components.is_empty() && components.last() != Some(&"..") {
                    components.pop();
                } else if !path.starts_with('/') {
                    components.push("..");
                }
            }
            p => components.push(p),
        }
    }

    if path.starts_with('/') {
        format!("/{}", components.join("/"))
    } else if components.is_empty() {
        ".".to_string()
    } else {
        components.join("/")
    }
}

/// Perform single substitution (no word splitting)
/// Port from singsub() in zsh/Src/subst.c
pub fn singsub(s: &str, params: &HashMap<String, String>) -> Result<String, String> {
    let opts = SubstOptions::default();
    subst_string(s, params, &opts)
}

/// Perform multiple substitution with word splitting
/// Port from multsub() in zsh/Src/subst.c
pub fn multsub(s: &str, params: &HashMap<String, String>) -> Result<Vec<String>, String> {
    let mut opts = SubstOptions::default();
    opts.word_split = true;

    let expanded = subst_string(s, params, &opts)?;

    // Split on IFS
    let ifs = params.get("IFS").map(|s| s.as_str()).unwrap_or(" \t\n");

    Ok(expanded
        .split(|c: char| ifs.contains(c))
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect())
}

/// Untokenize a string (remove internal tokens)
/// Port from untokenize() in zsh/Src/subst.c
pub fn untokenize(s: &str) -> String {
    s.to_string()
}

/// Remove null arguments
/// Port from remnulargs() in zsh/Src/subst.c
pub fn remnulargs(s: &str) -> String {
    s.to_string()
}

/// Pad string to specified width (from subst.c dopadding lines 892-1332)
pub fn dopadding(
    s: &str,
    prenum: usize,
    postnum: usize,
    preone: Option<&str>,
    postone: Option<&str>,
    premul: Option<&str>,
    postmul: Option<&str>,
) -> String {
    let default_pad = " ";
    let preone = preone.unwrap_or("");
    let postone = postone.unwrap_or("");
    let premul = if premul.map(|s| s.is_empty()).unwrap_or(true) {
        default_pad
    } else {
        premul.unwrap()
    };
    let postmul = if postmul.map(|s| s.is_empty()).unwrap_or(true) {
        default_pad
    } else {
        postmul.unwrap()
    };

    let slen = s.chars().count();

    if prenum + postnum == slen {
        return s.to_string();
    }

    let mut result = String::new();

    if prenum > 0 {
        let f = prenum.saturating_sub(slen);
        if f == 0 {
            // String is longer than prenum, truncate from left
            let skip = slen - prenum;
            result.extend(s.chars().skip(skip));
        } else {
            // Need to pad on left
            let mut pad_needed = f.saturating_sub(preone.chars().count());

            // Add repeated premul padding
            while pad_needed > 0 {
                let plen = premul.chars().count();
                if pad_needed >= plen {
                    result.push_str(premul);
                    pad_needed -= plen;
                } else {
                    // Partial pad
                    result.extend(premul.chars().take(pad_needed));
                    pad_needed = 0;
                }
            }

            // Add preone
            if !preone.is_empty() && f >= preone.chars().count() {
                result.push_str(preone);
            } else if !preone.is_empty() {
                // Truncate preone
                let skip = preone.chars().count() - f;
                result.extend(preone.chars().skip(skip));
            }

            // Add the string
            result.push_str(s);
        }
    } else if postnum > 0 {
        let f = postnum.saturating_sub(slen);
        if f == 0 {
            // String is longer than postnum, truncate from right
            result.extend(s.chars().take(postnum));
        } else {
            // Add the string
            result.push_str(s);

            // Add postone
            if !postone.is_empty() {
                if f >= postone.chars().count() {
                    result.push_str(postone);
                } else {
                    result.extend(postone.chars().take(f));
                }
            }

            // Add repeated postmul padding
            let mut pad_needed = f.saturating_sub(postone.chars().count());
            while pad_needed > 0 {
                let plen = postmul.chars().count();
                if pad_needed >= plen {
                    result.push_str(postmul);
                    pad_needed -= plen;
                } else {
                    result.extend(postmul.chars().take(pad_needed));
                    pad_needed = 0;
                }
            }
        }
    } else {
        result.push_str(s);
    }

    result
}

/// Get delimited string argument (from subst.c get_strarg lines 1346-1417)
pub fn get_strarg(s: &str) -> Option<(&str, char)> {
    let mut chars = s.chars();
    let delim = chars.next()?;

    let end_delim = match delim {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '<' => '>',
        _ => delim,
    };

    let rest: String = chars.collect();
    if let Some(pos) = rest.find(end_delim) {
        Some((&s[1..pos + 1], end_delim))
    } else {
        None
    }
}

/// Do =foo substitution (from subst.c equalsubstr lines 714-733)
pub fn equalsubstr(cmd: &str) -> Option<String> {
    crate::utils::find_in_path(cmd).and_then(|p| p.to_str().map(|s| s.to_string()))
}

/// File substitution - tilde and equals (from subst.c filesubstr lines 736-807)
pub fn filesubstr(name: &str, assign: bool) -> Option<String> {
    if name.starts_with('~') {
        let rest = &name[1..];

        // ~ alone
        if rest.is_empty() || rest.starts_with('/') {
            let home = std::env::var("HOME").unwrap_or_default();
            return Some(format!("{}{}", home, rest));
        }

        // ~+
        if rest.starts_with('+') && (rest.len() == 1 || rest.chars().nth(1) == Some('/')) {
            let pwd = std::env::var("PWD").unwrap_or_else(|_| ".".to_string());
            return Some(format!("{}{}", pwd, &rest[1..]));
        }

        // ~-
        if rest.starts_with('-') && (rest.len() == 1 || rest.chars().nth(1) == Some('/')) {
            let oldpwd = std::env::var("OLDPWD").unwrap_or_else(|_| ".".to_string());
            return Some(format!("{}{}", oldpwd, &rest[1..]));
        }

        // ~user
        let (user, suffix) = match rest.find('/') {
            Some(pos) => (&rest[..pos], &rest[pos..]),
            None => (rest, ""),
        };

        #[cfg(unix)]
        {
            if let Some(home) = crate::subst::get_user_home(user) {
                return Some(format!("{}{}", home, suffix));
            }
        }
    } else if name.starts_with('=') && name.len() > 1 {
        // =cmd substitution
        if let Some(path) = equalsubstr(&name[1..]) {
            return Some(path);
        }
    }

    None
}

/// Subst eval char - evaluate numeric expression to character (from subst.c substevalchar lines 1489-1520)
pub fn substevalchar(expr: &str) -> Option<char> {
    let value: i64 = expr.parse().ok()?;
    if value < 0 || value > 0x10FFFF {
        return None;
    }
    char::from_u32(value as u32)
}

/// Check if string is a subscript or length after colon (from subst.c check_colon_subscript lines 1565-1599)
pub fn check_colon_subscript(s: &str) -> Option<(String, &str)> {
    if s.is_empty() || s.starts_with(|c: char| c.is_alphabetic()) || s.starts_with('&') {
        return None;
    }

    if s.starts_with(':') {
        return Some(("0".to_string(), s));
    }

    // Find the end of the subscript expression
    let end = s.find(':').unwrap_or(s.len());
    let expr = &s[..end];
    let rest = &s[end..];

    Some((expr.to_string(), rest))
}

/// Apply offset and length to array (from subst.c ${PARAM:offset:length} handling)
pub fn array_slice(arr: &[String], offset: i64, length: Option<i64>) -> Vec<String> {
    let len = arr.len() as i64;

    let offset = if offset < 0 {
        (len + offset).max(0) as usize
    } else {
        (offset as usize).min(arr.len())
    };

    let length = match length {
        Some(l) if l < 0 => (len - offset as i64 + l).max(0) as usize,
        Some(l) => l.max(0) as usize,
        None => arr.len().saturating_sub(offset),
    };

    arr.iter().skip(offset).take(length).cloned().collect()
}

/// Apply offset and length to string (from subst.c ${PARAM:offset:length} handling)
pub fn string_slice(s: &str, offset: i64, length: Option<i64>) -> String {
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len() as i64;

    let offset = if offset < 0 {
        (len + offset).max(0) as usize
    } else {
        (offset as usize).min(chars.len())
    };

    let length = match length {
        Some(l) if l < 0 => (len - offset as i64 + l).max(0) as usize,
        Some(l) => l.max(0) as usize,
        None => chars.len().saturating_sub(offset),
    };

    chars.iter().skip(offset).take(length).collect()
}

/// Array union (from subst.c ${array|other})
pub fn array_union(arr1: &[String], arr2: &[String]) -> Vec<String> {
    use std::collections::HashSet;
    let set2: HashSet<_> = arr2.iter().collect();

    let mut result: Vec<String> = arr1.to_vec();
    for item in arr2 {
        if !result.contains(item) {
            result.push(item.clone());
        }
    }
    result
}

/// Array intersection (from subst.c ${array*other})
pub fn array_intersection(arr1: &[String], arr2: &[String]) -> Vec<String> {
    use std::collections::HashSet;
    let set2: HashSet<_> = arr2.iter().collect();

    arr1.iter()
        .filter(|item| set2.contains(item))
        .cloned()
        .collect()
}

/// Array difference (from subst.c ${array|other} with negation)
pub fn array_difference(arr1: &[String], arr2: &[String]) -> Vec<String> {
    use std::collections::HashSet;
    let set2: HashSet<_> = arr2.iter().collect();

    arr1.iter()
        .filter(|item| !set2.contains(item))
        .cloned()
        .collect()
}

/// Zip arrays together (from subst.c ${array:^other})
pub fn array_zip(arr1: &[String], arr2: &[String], shortest: bool) -> Vec<String> {
    let len = if shortest {
        arr1.len().min(arr2.len())
    } else {
        arr1.len().max(arr2.len())
    };

    let mut result = Vec::with_capacity(len * 2);
    for i in 0..len {
        let v1 = arr1.get(i % arr1.len()).cloned().unwrap_or_default();
        let v2 = arr2.get(i % arr2.len()).cloned().unwrap_or_default();
        result.push(v1);
        result.push(v2);
    }
    result
}

/// Unique array elements (from subst.c (u) flag)
pub fn array_unique(arr: &[String]) -> Vec<String> {
    use std::collections::HashSet;
    let mut seen = HashSet::new();
    arr.iter()
        .filter(|item| seen.insert(item.as_str()))
        .cloned()
        .collect()
}

/// Reverse array (from subst.c (O) flag with 'a')
pub fn array_reverse(arr: &[String]) -> Vec<String> {
    arr.iter().rev().cloned().collect()
}

/// Sort array (from subst.c (o) flag)
pub fn array_sort(arr: &[String], reverse: bool, numeric: bool) -> Vec<String> {
    let mut result = arr.to_vec();
    if numeric {
        result.sort_by(|a, b| {
            let na: f64 = a.parse().unwrap_or(0.0);
            let nb: f64 = b.parse().unwrap_or(0.0);
            na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
        });
    } else {
        result.sort();
    }
    if reverse {
        result.reverse();
    }
    result
}

/// Pattern filter (from subst.c ${array:#pattern})
pub fn array_filter_pattern(arr: &[String], pattern: &str, invert: bool) -> Vec<String> {
    arr.iter()
        .filter(|item| {
            let matches = crate::glob::pattern_match(pattern, item, false, true);
            if invert {
                matches
            } else {
                !matches
            }
        })
        .cloned()
        .collect()
}

/// Search and replace in array (from subst.c ${array/pat/repl})
pub fn array_replace(
    arr: &[String],
    pattern: &str,
    replacement: &str,
    global: bool,
) -> Vec<String> {
    arr.iter()
        .map(|item| {
            if global {
                item.replace(pattern, replacement)
            } else {
                item.replacen(pattern, replacement, 1)
            }
        })
        .collect()
}

/// Case modification (from subst.c (L), (U) flags)
pub fn modify_case(s: &str, mode: CaseMode) -> String {
    match mode {
        CaseMode::Lower => s.to_lowercase(),
        CaseMode::Upper => s.to_uppercase(),
        CaseMode::Capitalize => {
            let mut chars = s.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().chain(chars).collect(),
            }
        }
        CaseMode::CapitalizeWords => s
            .split_whitespace()
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    None => String::new(),
                    Some(c) => c.to_uppercase().chain(chars).collect(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

#[derive(Clone, Copy, Debug)]
pub enum CaseMode {
    Lower,
    Upper,
    Capitalize,
    CapitalizeWords,
}

/// Type info for parameter (from subst.c (t) flag)
pub fn param_type_info(value: &ParamValue) -> String {
    use crate::params::{flags, ParamValue};
    match value {
        ParamValue::Scalar(_) => "scalar".to_string(),
        ParamValue::Integer(_) => "integer".to_string(),
        ParamValue::Float(_) => "float".to_string(),
        ParamValue::Array(_) => "array".to_string(),
        ParamValue::Assoc(_) => "association".to_string(),
        ParamValue::Unset => "undefined".to_string(),
    }
}

use crate::params::ParamValue;

/// Subscript flags handling (from subst.c subscript parsing)
#[derive(Default, Clone, Debug)]
pub struct SubscriptFlags {
    pub reverse: bool,    // (r) flag
    pub words: bool,      // (w) flag
    pub chars: bool,      // (c) flag
    pub match_once: bool, // default vs (R) flag
}

/// Apply subscript to string (from subst.c getstrvalue)
pub fn apply_subscript_string(s: &str, start: i64, end: i64, flags: &SubscriptFlags) -> String {
    if flags.words {
        let words: Vec<&str> = s.split_whitespace().collect();
        return apply_subscript_array(
            &words.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            start,
            end,
        )
        .join(" ");
    }

    let chars: Vec<char> = s.chars().collect();
    let len = chars.len() as i64;

    let (start, end) = normalize_indices(start, end, len);

    chars[start..end].iter().collect()
}

/// Apply subscript to array (from subst.c getarrvalue)
pub fn apply_subscript_array(arr: &[String], start: i64, end: i64) -> Vec<String> {
    let len = arr.len() as i64;
    let (start, end) = normalize_indices(start, end, len);
    arr[start..end].to_vec()
}

fn normalize_indices(start: i64, end: i64, len: i64) -> (usize, usize) {
    let start = if start < 0 { len + start + 1 } else { start };
    let end = if end < 0 { len + end + 1 } else { end };
    let start = ((start.max(1) - 1) as usize).min(len as usize);
    let end = (end.max(0) as usize).min(len as usize);
    (start, end.max(start))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tilde_expand() {
        let opts = SubstOptions::default();
        let result = tilde_expand("~", &opts).unwrap();
        assert!(!result.starts_with('~'));
    }

    #[test]
    fn test_param_expand_simple() {
        let mut params = HashMap::new();
        params.insert("FOO".to_string(), "bar".to_string());

        let opts = SubstOptions::default();
        let result = param_expand("$FOO", &params, &opts).unwrap();
        assert_eq!(result, "bar");
    }

    #[test]
    fn test_param_expand_default() {
        let params = HashMap::new();
        let opts = SubstOptions::default();

        let result = param_expand("${UNDEFINED:-default}", &params, &opts).unwrap();
        assert_eq!(result, "default");
    }

    #[test]
    fn test_brace_expand_alternatives() {
        let results = brace_expand("file.{txt,md,rs}");
        assert_eq!(results.len(), 3);
        assert!(results.contains(&"file.txt".to_string()));
        assert!(results.contains(&"file.md".to_string()));
        assert!(results.contains(&"file.rs".to_string()));
    }

    #[test]
    fn test_brace_expand_range() {
        let results = brace_expand("file{1..3}");
        assert_eq!(results.len(), 3);
        assert!(results.contains(&"file1".to_string()));
        assert!(results.contains(&"file2".to_string()));
        assert!(results.contains(&"file3".to_string()));
    }

    #[test]
    fn test_remtpath() {
        assert_eq!(remtpath("/a/b/c", 1), "/a/b");
        assert_eq!(remtpath("/a/b/c", 2), "/a");
        assert_eq!(remtpath("foo", 1), ".");
    }

    #[test]
    fn test_remlpaths() {
        assert_eq!(remlpaths("/a/b/c", 1), "a/b/c");
        assert_eq!(remlpaths("a/b/c", 2), "c");
    }

    #[test]
    fn test_remtext() {
        assert_eq!(remtext("file.txt"), "file");
        assert_eq!(remtext("/path/to/file.txt"), "/path/to/file");
        assert_eq!(remtext("noext"), "noext");
    }

    #[test]
    fn test_rembutext() {
        assert_eq!(rembutext("file.txt"), "txt");
        assert_eq!(rembutext("/path/to/file.rs"), "rs");
        assert_eq!(rembutext("noext"), "");
    }

    #[test]
    fn test_casemodify() {
        assert_eq!(casemodify("Hello World", CaseMod::Lower), "hello world");
        assert_eq!(casemodify("Hello World", CaseMod::Upper), "HELLO WORLD");
        assert_eq!(casemodify("hello world", CaseMod::Caps), "Hello World");
    }

    #[test]
    fn test_clean_path() {
        assert_eq!(chabspath("/a/b/../c"), "/a/c");
        assert_eq!(chabspath("/a/./b/c"), "/a/b/c");
    }
}
