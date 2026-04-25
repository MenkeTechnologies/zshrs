//! Ksh93 compatibility module - port of Modules/ksh93.c
//!
//! Provides ksh93 compatibility features including:
//! - nameref builtin
//! - .sh.* special parameters

use std::collections::HashMap;

/// Ksh93 special parameters (.sh.*)
#[derive(Debug, Default)]
pub struct Ksh93Params {
    pub file: Option<String>,
    pub lineno: i64,
    pub fun: Option<String>,
    pub level: i64,
    pub subshell: i64,
    pub version: String,
    pub name: Option<String>,
    pub subscript: Option<String>,
    pub edchar: Option<String>,
    pub edmode: String,
    pub edcol: Option<i64>,
    pub edtext: Option<String>,
    pub command: Option<String>,
    pub value: Option<String>,
    pub match_arr: Vec<String>,
}

impl Ksh93Params {
    pub fn new() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            ..Default::default()
        }
    }

    /// Get a parameter by name
    pub fn get(&self, name: &str) -> Option<String> {
        match name {
            ".sh.file" => self.file.clone(),
            ".sh.lineno" => Some(self.lineno.to_string()),
            ".sh.fun" => self.fun.clone(),
            ".sh.level" => Some(self.level.to_string()),
            ".sh.subshell" => Some(self.subshell.to_string()),
            ".sh.version" => Some(self.version.clone()),
            ".sh.name" => self.name.clone(),
            ".sh.subscript" => self.subscript.clone(),
            ".sh.edchar" => self.edchar.clone(),
            ".sh.edmode" => Some(self.edmode.clone()),
            ".sh.edcol" => self.edcol.map(|n| n.to_string()),
            ".sh.edtext" => self.edtext.clone(),
            ".sh.command" => self.command.clone(),
            ".sh.value" => self.value.clone(),
            ".sh.match" => {
                if self.match_arr.is_empty() {
                    None
                } else {
                    Some(self.match_arr.join(" "))
                }
            }
            _ => None,
        }
    }

    /// Set a parameter by name
    pub fn set(&mut self, name: &str, value: &str) -> bool {
        match name {
            ".sh.edchar" => {
                self.edchar = Some(value.to_string());
                true
            }
            ".sh.value" => {
                self.value = Some(value.to_string());
                true
            }
            _ => false,
        }
    }

    /// Update function context
    pub fn enter_function(&mut self, name: &str, file: Option<&str>, lineno: i64) {
        self.level += 1;
        self.fun = Some(name.to_string());
        self.file = file.map(|s| s.to_string());
        self.lineno = lineno;
    }

    /// Exit function context
    pub fn exit_function(&mut self) {
        self.level = (self.level - 1).max(0);
        self.fun = None;
    }

    /// Enter subshell
    pub fn enter_subshell(&mut self) {
        self.subshell += 1;
    }

    /// Exit subshell
    pub fn exit_subshell(&mut self) {
        self.subshell = (self.subshell - 1).max(0);
    }

    /// Set match array
    pub fn set_match(&mut self, full: Option<&str>, captures: &[Option<String>]) {
        self.match_arr.clear();
        if let Some(m) = full {
            self.match_arr.push(m.to_string());
        }
        for cap in captures {
            if let Some(c) = cap {
                self.match_arr.push(c.clone());
            }
        }
    }

    /// Get all parameters as hash
    pub fn to_hash(&self) -> HashMap<String, String> {
        let mut map = HashMap::new();
        for name in &[
            ".sh.file",
            ".sh.lineno",
            ".sh.fun",
            ".sh.level",
            ".sh.subshell",
            ".sh.version",
            ".sh.name",
            ".sh.subscript",
            ".sh.edchar",
            ".sh.edmode",
            ".sh.edcol",
            ".sh.edtext",
            ".sh.command",
            ".sh.value",
            ".sh.match",
        ] {
            if let Some(v) = self.get(name) {
                map.insert(name.to_string(), v);
            }
        }
        map
    }
}

/// Nameref options
#[derive(Debug, Default, Clone)]
pub struct NamerefOptions {
    pub global: bool,
    pub print: bool,
    pub readonly: bool,
    pub unset: bool,
}

/// Execute nameref builtin
pub fn builtin_nameref(args: &[&str], options: &NamerefOptions) -> (i32, String) {
    if args.is_empty() {
        if options.print {
            return (0, String::new());
        }
        return (1, "nameref: variable name required\n".to_string());
    }

    let name = args[0];

    if !is_valid_identifier(name) {
        return (1, format!("nameref: {}: invalid variable name\n", name));
    }

    if args.len() < 2 {
        if options.unset {
            return (0, String::new());
        }
        return (1, format!("nameref: {}: reference target required\n", name));
    }

    let target = args[1];

    if !is_valid_identifier(target) {
        return (
            1,
            format!("nameref: {}: invalid reference target\n", target),
        );
    }

    (0, String::new())
}

fn is_valid_identifier(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }

    let mut chars = s.chars();
    let first = chars.next().unwrap();

    if !first.is_alphabetic() && first != '_' {
        return false;
    }

    chars.all(|c| c.is_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ksh93_params_new() {
        let params = Ksh93Params::new();
        assert!(!params.version.is_empty());
        assert_eq!(params.level, 0);
    }

    #[test]
    fn test_ksh93_params_get() {
        let params = Ksh93Params::new();
        assert!(params.get(".sh.version").is_some());
        assert!(params.get(".sh.invalid").is_none());
    }

    #[test]
    fn test_ksh93_params_enter_function() {
        let mut params = Ksh93Params::new();
        params.enter_function("test", Some("test.zsh"), 10);
        assert_eq!(params.level, 1);
        assert_eq!(params.fun, Some("test".to_string()));
        assert_eq!(params.lineno, 10);
    }

    #[test]
    fn test_ksh93_params_exit_function() {
        let mut params = Ksh93Params::new();
        params.enter_function("test", None, 1);
        params.exit_function();
        assert_eq!(params.level, 0);
        assert!(params.fun.is_none());
    }

    #[test]
    fn test_ksh93_params_subshell() {
        let mut params = Ksh93Params::new();
        params.enter_subshell();
        assert_eq!(params.subshell, 1);
        params.exit_subshell();
        assert_eq!(params.subshell, 0);
    }

    #[test]
    fn test_ksh93_params_set_match() {
        let mut params = Ksh93Params::new();
        params.set_match(
            Some("hello"),
            &[Some("h".to_string()), Some("ello".to_string())],
        );
        assert_eq!(params.match_arr.len(), 3);
    }

    #[test]
    fn test_is_valid_identifier() {
        assert!(is_valid_identifier("foo"));
        assert!(is_valid_identifier("_bar"));
        assert!(is_valid_identifier("foo123"));
        assert!(!is_valid_identifier(""));
        assert!(!is_valid_identifier("123"));
        assert!(!is_valid_identifier("foo-bar"));
    }

    #[test]
    fn test_builtin_nameref_no_args() {
        let options = NamerefOptions::default();
        let (status, _) = builtin_nameref(&[], &options);
        assert_eq!(status, 1);
    }

    #[test]
    fn test_builtin_nameref_no_target() {
        let options = NamerefOptions::default();
        let (status, _) = builtin_nameref(&["foo"], &options);
        assert_eq!(status, 1);
    }

    #[test]
    fn test_builtin_nameref_valid() {
        let options = NamerefOptions::default();
        let (status, _) = builtin_nameref(&["foo", "bar"], &options);
        assert_eq!(status, 0);
    }

    #[test]
    fn test_builtin_nameref_invalid_name() {
        let options = NamerefOptions::default();
        let (status, _) = builtin_nameref(&["123", "bar"], &options);
        assert_eq!(status, 1);
    }
}
