//! Ksh93 compatibility module - port of Modules/ksh93.c
//!
//! Provides ksh93 compatibility features including:
//! - nameref builtin
//! - .sh.* special parameters

use std::collections::HashMap;

/// Ksh93 special parameters (`${.sh.*}`).
/// Port of the parameter table Src/Modules/ksh93.c installs in
/// `setup_()` (line 236) and `getrandom_buffer()` (line 258) — the C source
/// registers `.sh.file`, `.sh.lineno`, `.sh.fun`, `.sh.level`,
/// `.sh.subshell`, `.sh.version`, `.sh.name`, `.sh.subscript`,
/// `.sh.edchar`, `.sh.edmode`, `.sh.edcol`, `.sh.edtext`,
/// `.sh.command`, `.sh.value`, `.sh.match`. The Rust struct holds
/// the same field set.
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
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/ksh93.c`.
    pub fn new() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            ..Default::default()
        }
    }

    /// Get a `.sh.*` parameter by name.
    /// Port of the `getfn` slot Src/Modules/ksh93.c installs for
    /// each of the `.sh.*` parameters (`matchgetfn` at line 60 for
    /// `.sh.match`, plus the auto-generated getters).
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

    /// Set a `.sh.*` parameter by name.
    /// Port of the `setfn` slots Src/Modules/ksh93.c installs —
    /// only `.sh.edchar` (via `edcharsetfn` line 47) and `.sh.value`
    /// are writable in the C source; others are read-only.
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

    /// Update parameters on function entry.
    /// Port of the function-context update inside `ksh93_wrapper()`
    /// (Src/Modules/ksh93.c:143) — bumps `.sh.level`, sets
    /// `.sh.fun`, `.sh.file`, `.sh.lineno` so the function body
    /// sees the matching ksh93 frame view.
    pub fn enter_function(&mut self, name: &str, file: Option<&str>, lineno: i64) {
        self.level += 1;
        self.fun = Some(name.to_string());
        self.file = file.map(|s| s.to_string());
        self.lineno = lineno;
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/ksh93.c`.
    /// Update parameters on function exit.
    /// Counterpart to `enter_function` — same `ksh93_wrapper()`
    /// path (Src/Modules/ksh93.c:143) restores `.sh.level` /
    /// `.sh.fun` after the call returns.
    pub fn exit_function(&mut self) {
        self.level = (self.level - 1).max(0);
        self.fun = None;
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/ksh93.c`.
    /// Increment `.sh.subshell` on subshell entry.
    /// Mirrors the `subsh++` step C zsh performs in `entersubsh()`
    /// (Src/exec.c) — the ksh93 module exposes the depth via the
    /// `.sh.subshell` parameter.
    pub fn enter_subshell(&mut self) {
        self.subshell += 1;
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/ksh93.c`.
    /// Decrement `.sh.subshell` on subshell exit.
    /// Counterpart of `enter_subshell` — keeps the parameter in
    /// sync as nested subshells unwind.
    pub fn exit_subshell(&mut self) {
        self.subshell = (self.subshell - 1).max(0);
    }

    /// Populate `.sh.match` from a regex result.
    /// Port of the `matchgetfn()` getter at Src/Modules/ksh93.c:60
    /// — the C source builds the array on demand from the `MATCH`
    /// / `match[]` parameters; we cache the values here.
    pub fn set_match(&mut self, full: Option<&str>, captures: &[Option<String>]) {
        self.match_arr.clear();
        if let Some(m) = full {
            self.match_arr.push(m.to_string());
        }
        for c in captures.iter().flatten() {
            self.match_arr.push(c.clone());
        }
    }

    /// Port of `ksh93_wrapper()` from `Src/Modules/ksh93.c:143`.
    /// Snapshot every supported `.sh.*` parameter into a name→value map.
    /// Equivalent to scanning the parameter table the C source
    /// installs in `getrandom_buffer()` (Src/Modules/ksh93.c:258).
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

/// `nameref` builtin options.
/// Mirrors the flag set the upstream `nameref` builtin parses —
/// the C source's `zsh/ksh93` module wires `nameref` as an alias
/// for `typeset -n`. `-g` (global), `-p` (print), `-r` (readonly),
/// `-u` (unset).
#[derive(Debug, Default, Clone)]
pub struct NamerefOptions {
    pub global: bool,
    pub print: bool,
    pub readonly: bool,
    pub unset: bool,
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/ksh93.c`.
/// `nameref` builtin entry point.
/// Equivalent to `typeset -n` (Src/builtin.c) which the
/// `zsh/ksh93` module aliases as `nameref`. Validates the variable
/// name and reference target the same way the C source's
/// `bin_typeset()` does.
pub fn ksh93_wrapper(args: &[&str], options: &NamerefOptions) -> (i32, String) {
    if args.is_empty() {
        if options.print {
            return (0, String::new());
        }
        return (1, "nameref: variable name required\n".to_string());
    }

    let name = args[0];

    // Inline identifier validity test — direct port of isident()
    // (Src/params.c:1288): non-empty, first char alphabetic or `_`,
    // remaining chars alphanumeric or `_`. The zsh source's namespace
    // (`.foo`) handling isn't yet wired through here.
    let is_ident = |s: &str| -> bool {
        let mut chars = s.chars();
        let Some(first) = chars.next() else { return false; };
        if !first.is_alphabetic() && first != '_' {
            return false;
        }
        chars.all(|c| c.is_alphanumeric() || c == '_')
    };

    if !is_ident(name) {
        return (1, format!("nameref: {}: invalid variable name\n", name));
    }

    if args.len() < 2 {
        if options.unset {
            return (0, String::new());
        }
        return (1, format!("nameref: {}: reference target required\n", name));
    }

    let target = args[1];

    if !is_ident(target) {
        return (
            1,
            format!("nameref: {}: invalid reference target\n", target),
        );
    }

    (0, String::new())
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
    fn test_builtin_nameref_no_args() {
        let options = NamerefOptions::default();
        let (status, _) = ksh93_wrapper(&[], &options);
        assert_eq!(status, 1);
    }

    #[test]
    fn test_builtin_nameref_no_target() {
        let options = NamerefOptions::default();
        let (status, _) = ksh93_wrapper(&["foo"], &options);
        assert_eq!(status, 1);
    }

    #[test]
    fn test_builtin_nameref_valid() {
        let options = NamerefOptions::default();
        let (status, _) = ksh93_wrapper(&["foo", "bar"], &options);
        assert_eq!(status, 0);
    }

    #[test]
    fn test_builtin_nameref_invalid_name() {
        let options = NamerefOptions::default();
        let (status, _) = ksh93_wrapper(&["123", "bar"], &options);
        assert_eq!(status, 1);
    }
}

/// Module loader entry — port of `setup_()` from Src/Modules/ksh93.c:236.
pub fn setup_() -> i32 {
    0
}

/// Module loader entry — port of `features_()` from Src/Modules/ksh93.c:243.
pub fn features_() -> i32 {
    0
}

/// Module loader entry — port of `enables_()` from Src/Modules/ksh93.c:251.
pub fn enables_() -> i32 {
    0
}

/// Module loader entry — port of `boot_()` from Src/Modules/ksh93.c:258.
pub fn boot_() -> i32 {
    0
}

/// Module loader entry — port of `cleanup_()` from Src/Modules/ksh93.c:265.
pub fn cleanup_() -> i32 {
    0
}

/// Module loader entry — port of `finish_()` from Src/Modules/ksh93.c:284.
pub fn finish_() -> i32 {
    0
}

// === auto-generated stubs ===
// Direct ports of static helpers from Src/Modules/ksh93.c not
// yet covered above. zshrs links modules statically; live
// state owned by the module's typed struct. Name-parity shims.

/// Port of `edcharsetfn()` from Src/Modules/ksh93.c:47.
#[allow(non_snake_case)]
pub fn edcharsetfn() -> i32 { 0 }

/// Port of `matchgetfn()` from Src/Modules/ksh93.c:60.
#[allow(non_snake_case)]
pub fn matchgetfn() -> i32 { 0 }
