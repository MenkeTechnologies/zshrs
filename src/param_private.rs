//! Private parameters module - port of Modules/param_private.c
//!
//! Provides private parameter scoping for shell functions.

use std::collections::HashMap;

/// Private parameter state
#[derive(Debug, Clone)]
pub struct PrivateParam {
    pub name: String,
    pub value: ParamValue,
    pub level: usize,
    pub readonly: bool,
}

/// Parameter value types
#[derive(Debug, Clone)]
pub enum ParamValue {
    Scalar(String),
    Integer(i64),
    Float(f64),
    Array(Vec<String>),
    Hash(HashMap<String, String>),
}

/// Private scope manager
#[derive(Debug, Default)]
pub struct PrivateScope {
    params: HashMap<String, PrivateParam>,
    level: usize,
}

impl PrivateScope {
    pub fn new() -> Self {
        Self::default()
    }

    /// Enter a new scope level
    pub fn enter(&mut self) {
        self.level += 1;
    }

    /// Exit current scope level
    pub fn exit(&mut self) {
        let level = self.level;
        self.params.retain(|_, p| p.level < level);
        self.level = self.level.saturating_sub(1);
    }

    /// Current scope level
    pub fn level(&self) -> usize {
        self.level
    }

    /// Add a private parameter
    pub fn add(&mut self, name: &str, value: ParamValue, readonly: bool) -> bool {
        if let Some(existing) = self.params.get(name) {
            if existing.readonly {
                return false;
            }
        }

        self.params.insert(
            name.to_string(),
            PrivateParam {
                name: name.to_string(),
                value,
                level: self.level,
                readonly,
            },
        );

        true
    }

    /// Get a private parameter
    pub fn get(&self, name: &str) -> Option<&PrivateParam> {
        self.params.get(name)
    }

    /// Get a private parameter mutably
    pub fn get_mut(&mut self, name: &str) -> Option<&mut PrivateParam> {
        let param = self.params.get_mut(name)?;
        if param.readonly {
            return None;
        }
        Some(param)
    }

    /// Check if a parameter is private at current scope
    pub fn is_private(&self, name: &str) -> bool {
        self.params
            .get(name)
            .map(|p| p.level == self.level)
            .unwrap_or(false)
    }

    /// Set parameter value if not readonly
    pub fn set(&mut self, name: &str, value: ParamValue) -> bool {
        if let Some(param) = self.params.get_mut(name) {
            if param.readonly {
                return false;
            }
            param.value = value;
            return true;
        }
        false
    }

    /// Remove a private parameter
    pub fn remove(&mut self, name: &str) -> bool {
        if let Some(param) = self.params.get(name) {
            if param.readonly {
                return false;
            }
        }
        self.params.remove(name).is_some()
    }

    /// List all private parameters at current level
    pub fn list_current(&self) -> Vec<&PrivateParam> {
        self.params
            .values()
            .filter(|p| p.level == self.level)
            .collect()
    }

    /// List all private parameters
    pub fn list_all(&self) -> Vec<&PrivateParam> {
        self.params.values().collect()
    }
}

/// Execute private builtin
pub fn builtin_private(args: &[&str], scope: &mut PrivateScope) -> (i32, String) {
    if args.is_empty() {
        let params = scope.list_current();
        if params.is_empty() {
            return (0, String::new());
        }

        let mut output = String::new();
        for p in params {
            let type_str = match &p.value {
                ParamValue::Scalar(_) => "",
                ParamValue::Integer(_) => "-i ",
                ParamValue::Float(_) => "-F ",
                ParamValue::Array(_) => "-a ",
                ParamValue::Hash(_) => "-A ",
            };
            let readonly = if p.readonly { "-r " } else { "" };
            output.push_str(&format!("private {}{}{}\n", type_str, readonly, p.name));
        }

        return (0, output);
    }

    let mut i = 0;
    let mut param_type = ParamValue::Scalar(String::new());
    let mut readonly = false;

    while i < args.len() && args[i].starts_with('-') {
        match args[i] {
            "-i" => param_type = ParamValue::Integer(0),
            "-F" => param_type = ParamValue::Float(0.0),
            "-a" => param_type = ParamValue::Array(Vec::new()),
            "-A" => param_type = ParamValue::Hash(HashMap::new()),
            "-r" => readonly = true,
            _ => {}
        }
        i += 1;
    }

    if i >= args.len() {
        return (1, "private: parameter name required\n".to_string());
    }

    for arg in &args[i..] {
        if let Some((name, value)) = arg.split_once('=') {
            let val = match &param_type {
                ParamValue::Scalar(_) => ParamValue::Scalar(value.to_string()),
                ParamValue::Integer(_) => ParamValue::Integer(value.parse().unwrap_or(0)),
                ParamValue::Float(_) => ParamValue::Float(value.parse().unwrap_or(0.0)),
                ParamValue::Array(_) => {
                    ParamValue::Array(value.split_whitespace().map(|s| s.to_string()).collect())
                }
                ParamValue::Hash(_) => {
                    let mut map = HashMap::new();
                    for pair in value.split_whitespace() {
                        if let Some((k, v)) = pair.split_once('=') {
                            map.insert(k.to_string(), v.to_string());
                        }
                    }
                    ParamValue::Hash(map)
                }
            };

            if !scope.add(name, val, readonly) {
                return (1, format!("private: read-only variable: {}\n", name));
            }
        } else {
            let val = match &param_type {
                ParamValue::Scalar(_) => ParamValue::Scalar(String::new()),
                ParamValue::Integer(_) => ParamValue::Integer(0),
                ParamValue::Float(_) => ParamValue::Float(0.0),
                ParamValue::Array(_) => ParamValue::Array(Vec::new()),
                ParamValue::Hash(_) => ParamValue::Hash(HashMap::new()),
            };

            if !scope.add(arg, val, readonly) {
                return (1, format!("private: read-only variable: {}\n", arg));
            }
        }
    }

    (0, String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_private_scope_new() {
        let scope = PrivateScope::new();
        assert_eq!(scope.level(), 0);
    }

    #[test]
    fn test_private_scope_enter_exit() {
        let mut scope = PrivateScope::new();
        scope.enter();
        assert_eq!(scope.level(), 1);
        scope.exit();
        assert_eq!(scope.level(), 0);
    }

    #[test]
    fn test_private_scope_add_get() {
        let mut scope = PrivateScope::new();
        scope.enter();
        scope.add("foo", ParamValue::Scalar("bar".to_string()), false);
        assert!(scope.get("foo").is_some());
    }

    #[test]
    fn test_private_scope_readonly() {
        let mut scope = PrivateScope::new();
        scope.enter();
        scope.add("foo", ParamValue::Scalar("bar".to_string()), true);
        assert!(scope.get("foo").is_some());
        assert!(scope.get_mut("foo").is_none());
    }

    #[test]
    fn test_private_scope_exit_removes() {
        let mut scope = PrivateScope::new();
        scope.enter();
        scope.add("foo", ParamValue::Scalar("bar".to_string()), false);
        scope.exit();
        assert!(scope.get("foo").is_none());
    }

    #[test]
    fn test_builtin_private_no_args() {
        let mut scope = PrivateScope::new();
        scope.enter();
        let (status, _) = builtin_private(&[], &mut scope);
        assert_eq!(status, 0);
    }

    #[test]
    fn test_builtin_private_scalar() {
        let mut scope = PrivateScope::new();
        scope.enter();
        let (status, _) = builtin_private(&["foo=bar"], &mut scope);
        assert_eq!(status, 0);
        assert!(scope.get("foo").is_some());
    }

    #[test]
    fn test_builtin_private_integer() {
        let mut scope = PrivateScope::new();
        scope.enter();
        let (status, _) = builtin_private(&["-i", "foo=42"], &mut scope);
        assert_eq!(status, 0);
        if let Some(p) = scope.get("foo") {
            assert!(matches!(p.value, ParamValue::Integer(42)));
        }
    }
}
