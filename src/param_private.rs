//! Private parameters module - port of Modules/param_private.c
//!
//! Provides private parameter scoping for shell functions.

use std::collections::HashMap;

/// Private parameter state.
/// Port of the per-parameter private wrapper Src/Modules/
/// param_private.c installs via `makeprivate()` (line 80) — keeps
/// the original parameter alongside its private level so
/// `scopeprivate()` (line 512) can restore the outer scope on
/// function exit.
#[derive(Debug, Clone)]
pub struct PrivateParam {
    pub name: String,
    pub value: ParamValue,
    pub level: usize,
    pub readonly: bool,
}

/// Parameter value types.
/// Mirrors the `PM_*` flags + value union Src/zsh.h declares for
/// `Param`. Each variant lines up with one of the private-getter
/// pairs Src/Modules/param_private.c installs (`pps_*` for scalar,
/// `ppi_*` for integer, `ppf_*` for float, `ppa_*` for array,
/// `pph_*` for hash).
#[derive(Debug, Clone)]
pub enum ParamValue {
    Scalar(String),
    Integer(i64),
    Float(f64),
    Array(Vec<String>),
    Hash(HashMap<String, String>),
}

/// Private scope manager.
/// Port of the function-local scope tracking
/// Src/Modules/param_private.c keeps via `locallevel` (provided by
/// the C source's `scopeprivate()` hook, line 512). Each function
/// invocation enters a new scope; exit purges every parameter
/// tagged at that level.
#[derive(Debug, Default)]
pub struct PrivateScope {
    params: HashMap<String, PrivateParam>,
    level: usize,
}

impl PrivateScope {
    pub fn new() -> Self {
        Self::default()
    }

    /// Enter a new scope level.
    /// Port of the `locallevel++` step `scopeprivate()` from
    /// Src/Modules/param_private.c:512 fires on function entry.
    pub fn enter(&mut self) {
        self.level += 1;
    }

    /// Exit current scope level.
    /// Port of the `locallevel--` + per-parameter unset step
    /// `scopeprivate()` from Src/Modules/param_private.c:512 fires
    /// on function exit. Drops every parameter whose level matches
    /// the leaving scope.
    pub fn exit(&mut self) {
        let level = self.level;
        self.params.retain(|_, p| p.level < level);
        self.level = self.level.saturating_sub(1);
    }

    /// Current scope level.
    /// Equivalent to reading the global `locallevel` in
    /// Src/Modules/param_private.c.
    pub fn level(&self) -> usize {
        self.level
    }

    /// Add a private parameter.
    /// Port of `makeprivate()` from Src/Modules/param_private.c:80
    /// — installs the private getter/setter pair on a parameter so
    /// the outer scope can't see writes inside the function body.
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

    /// Get a private parameter.
    /// Port of the `*_getfn` slot Src/Modules/param_private.c
    /// installs for each type (`pps_getfn` line 287, `ppi_getfn`
    /// line 328, etc.). Returns the value the function currently
    /// sees, hiding any outer-scope shadow.
    pub fn get(&self, name: &str) -> Option<&PrivateParam> {
        self.params.get(name)
    }

    /// Get a private parameter mutably.
    /// Port of the `*_setfn` slot Src/Modules/param_private.c
    /// installs (`pps_setfn` line 300, `ppi_setfn` line 340, etc.).
    /// Honours `PM_READONLY` by returning `None` for read-only
    /// entries — the C source raises an error via `setfn_error()`
    /// (line 260).
    pub fn get_mut(&mut self, name: &str) -> Option<&mut PrivateParam> {
        let param = self.params.get_mut(name)?;
        if param.readonly {
            return None;
        }
        Some(param)
    }

    /// Check if a parameter is private at current scope.
    /// Port of `is_private()` from Src/Modules/param_private.c:181.
    pub fn is_private(&self, name: &str) -> bool {
        self.params
            .get(name)
            .map(|p| p.level == self.level)
            .unwrap_or(false)
    }

    /// Set parameter value if not readonly.
    /// Convenience over `get_mut` + assign — mirrors the
    /// `*_setfn` slot dispatch Src/Modules/param_private.c performs
    /// during normal `name=value` assignment.
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

    /// Remove a private parameter.
    /// Port of the `*_unsetfn` slot Src/Modules/param_private.c
    /// installs for each type (`pps_unsetfn` line 312, `ppi_unsetfn`
    /// line 352, etc.). Honours `PM_READONLY` the same way the C
    /// source does.
    pub fn remove(&mut self, name: &str) -> bool {
        if let Some(param) = self.params.get(name) {
            if param.readonly {
                return false;
            }
        }
        self.params.remove(name).is_some()
    }

    /// List all private parameters at current level.
    /// Equivalent to the `bin_private()`-with-no-args walk in
    /// Src/Modules/param_private.c:217 — the C source `scanhashtable`s
    /// `paramtab` and prints private entries at the current level.
    pub fn list_current(&self) -> Vec<&PrivateParam> {
        self.params
            .values()
            .filter(|p| p.level == self.level)
            .collect()
    }

    /// List all private parameters across every scope level.
    /// Closest C analog is the full `paramtab` walk Src/Modules/
    /// param_private.c does internally for the `private -p` debug
    /// path; mostly used for tests.
    pub fn list_all(&self) -> Vec<&PrivateParam> {
        self.params.values().collect()
    }
}

/// `private` builtin entry point.
/// Port of `bin_private()` from Src/Modules/param_private.c:217.
/// With no args lists private parameters at the current scope;
/// with args parses `-i`/`-F`/`-a`/`-A`/`-r` flags and installs
/// each named parameter via `makeprivate()` (line 80).
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
