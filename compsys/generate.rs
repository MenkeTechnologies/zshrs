//! Completion generation from cache
//!
//! This module provides functions to generate completions from the SQLite cache,
//! used by both the menu_demo and the main zshrs shell.

use crate::{cache::CompsysCache, Completion, CompletionGroup};
use std::collections::HashSet;
use std::sync::OnceLock;

static EXECUTABLES_SET: OnceLock<HashSet<String>> = OnceLock::new();

/// Completion context types matching zshcompsys special contexts
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CompContext {
    Command,            // -command- : command position
    Default,            // -default- : arguments
    Parameter,          // -parameter- : $VAR
    BraceParameter,     // -brace-parameter- : ${VAR}
    BraceParameterFlag, // ${(flags)...}
    Value,              // -value- : right side of =
    ArrayValue,         // -array-value- : array=()
    AssignParameter,    // -assign-parameter- : left of =
    Redirect,           // -redirect- : after >, <
    Condition,          // -condition- : inside [[ ]]
    Math,               // -math- : inside (( ))
    Subscript,          // -subscript- : array[idx]
    Tilde,              // -tilde- : ~user
    Equal,              // -equal- : =cmd
    GlobQualifier,      // *(qualifiers)
    History,            // !! history
}

/// Detect completion context from cursor position
pub fn detect_completion_context(before_cursor: &str, prefix: &str) -> CompContext {
    let trimmed = before_cursor.trim();

    if prefix.starts_with("${(") {
        return CompContext::BraceParameterFlag;
    }
    if prefix.starts_with("${") && !prefix.contains('(') {
        return CompContext::BraceParameter;
    }
    if prefix.starts_with('$') && !prefix.starts_with("${") && !prefix.starts_with("$(") {
        return CompContext::Parameter;
    }
    if prefix.contains("*(")
        || prefix.contains("?(")
        || (prefix.ends_with("(") && (before_cursor.contains('*') || before_cursor.contains('?')))
    {
        return CompContext::GlobQualifier;
    }
    if prefix.starts_with('~') && !prefix.contains('/') {
        return CompContext::Tilde;
    }
    if prefix.starts_with('=') && prefix.len() > 1 {
        return CompContext::Equal;
    }
    if prefix.starts_with('!') || prefix.starts_with("!!") {
        return CompContext::History;
    }

    let words: Vec<&str> = before_cursor.split_whitespace().collect();
    if let Some(&last) = words.last() {
        if matches!(
            last,
            ">" | "<" | ">>" | "<<" | ">&" | "<&" | ">|" | "2>" | "2>>" | "&>" | "&>>" | "<>"
        ) {
            return CompContext::Redirect;
        }
    }
    for op in &[
        ">", "<", ">>", "<<", ">&", "<&", ">|", "2>", "2>>", "&>", "&>>", "<>",
    ] {
        if trimmed.ends_with(op) {
            return CompContext::Redirect;
        }
    }
    if before_cursor.contains("((") && !before_cursor.contains("))") {
        return CompContext::Math;
    }
    if before_cursor.contains("[[") && !before_cursor.contains("]]") {
        return CompContext::Condition;
    }
    if prefix.contains('[') && !prefix.contains(']') {
        return CompContext::Subscript;
    }
    if before_cursor.contains('=') && !before_cursor.contains(' ') {
        if let Some(eq_pos) = before_cursor.rfind('=') {
            let after_eq = &before_cursor[eq_pos + 1..];
            if after_eq.is_empty() || !after_eq.contains(' ') {
                if after_eq.starts_with('(') {
                    return CompContext::ArrayValue;
                }
                return CompContext::Value;
            }
        }
    }

    let completing_command =
        words.is_empty() || (words.len() == 1 && !before_cursor.ends_with(' '));

    if completing_command {
        CompContext::Command
    } else {
        CompContext::Default
    }
}

/// Generate completions based on editor state
pub fn generate_completions(
    cache: &CompsysCache,
    line: &str,
    cursor: usize,
) -> Vec<CompletionGroup> {
    let before_cursor = &line[..cursor];
    let prefix = current_word(before_cursor);
    let words: Vec<&str> = before_cursor.split_whitespace().collect();

    let mut groups = Vec::new();
    let context = detect_completion_context(before_cursor, prefix);

    match context {
        CompContext::BraceParameterFlag => {
            let flag_prefix = prefix.trim_start_matches("${(").trim_start_matches("$(");
            groups.push(complete_parameter_flags(flag_prefix));
        }
        CompContext::BraceParameter => {
            let var_prefix = prefix.trim_start_matches("${");
            groups.push(complete_parameters(var_prefix));
        }
        CompContext::Parameter => {
            let var_prefix = prefix.trim_start_matches('$');
            groups.push(complete_parameters(var_prefix));
        }
        CompContext::GlobQualifier => {
            let qual_prefix = if let Some(idx) = prefix.rfind('(') {
                &prefix[idx + 1..]
            } else {
                ""
            };
            groups.push(complete_glob_qualifiers(qual_prefix));
        }
        CompContext::Tilde => {
            let user_prefix = prefix.trim_start_matches('~');
            groups.extend(complete_users_and_named_dirs(cache, user_prefix));
        }
        CompContext::Equal => {
            let cmd_prefix = prefix.trim_start_matches('=');
            groups.push(complete_commands_from_cache(cache, cmd_prefix));
        }
        CompContext::Redirect | CompContext::Value | CompContext::ArrayValue => {
            groups.push(complete_files(prefix, true));
        }
        CompContext::Math => {
            groups.push(complete_parameters(prefix));
            groups.push(complete_math_functions(prefix));
        }
        CompContext::Condition => {
            groups.push(complete_condition_operators(prefix));
            groups.push(complete_files(prefix, true));
        }
        CompContext::Subscript => {
            groups.push(complete_subscript_flags(prefix));
        }
        CompContext::AssignParameter => {
            groups.push(complete_parameters(prefix));
        }
        CompContext::Command => {
            groups.push(complete_commands_from_cache(cache, prefix));
            groups.push(complete_shell_functions(cache, prefix));
            groups.push(complete_builtins(prefix));
            groups.push(complete_files(prefix, true));
        }
        CompContext::Default => {
            let cmd = words.first().copied().unwrap_or("");
            let arg_num = if before_cursor.ends_with(' ') {
                words.len()
            } else {
                words.len().saturating_sub(1)
            };

            if let Ok(Some(func)) = cache.get_comp(cmd) {
                groups.extend(complete_from_cache_function(
                    cache,
                    cmd,
                    &func,
                    &words,
                    arg_num,
                    prefix,
                    before_cursor,
                ));
            } else {
                if prefix.starts_with('-') {
                    groups.push(complete_generic_options(prefix));
                }
                groups.push(complete_files(prefix, true));
            }
        }
        CompContext::History => {
            groups.push(complete_history_modifiers(prefix));
        }
    }

    groups
        .into_iter()
        .filter(|g| !g.matches.is_empty())
        .collect()
}

fn current_word(before_cursor: &str) -> &str {
    let start = before_cursor
        .rfind(char::is_whitespace)
        .map(|i| i + 1)
        .unwrap_or(0);
    &before_cursor[start..]
}

/// Complete commands from cache
pub fn complete_commands_from_cache(cache: &CompsysCache, prefix: &str) -> CompletionGroup {
    let mut group = CompletionGroup::new("command");
    group.explanation = Some("external command".to_string());

    if let Ok(executables) = cache.get_executables_prefix_fts(prefix) {
        for (name, path) in executables.into_iter().take(200) {
            let mut c = Completion::new(name);
            c.desc = Some(path);
            group.matches.push(c);
        }
    }
    group
}

/// Complete shell functions from cache
pub fn complete_shell_functions(cache: &CompsysCache, prefix: &str) -> CompletionGroup {
    let mut group = CompletionGroup::new("function");
    group.explanation = Some("shell function".to_string());

    if let Ok(funcs) = cache.get_shell_functions_prefix(prefix) {
        for (name, source) in funcs.into_iter().take(100) {
            let mut c = Completion::new(name);
            c.desc = Some(source);
            group.matches.push(c);
        }
    }
    group
}

/// Complete builtins
pub fn complete_builtins(prefix: &str) -> CompletionGroup {
    let mut group = CompletionGroup::new("builtin");
    group.explanation = Some("shell builtin".to_string());

    let builtins = [
        (".", "source file"),
        (":", "null command"),
        ("alias", "define alias"),
        ("autoload", "autoload function"),
        ("bg", "background job"),
        ("bindkey", "key bindings"),
        ("break", "exit loop"),
        ("builtin", "run builtin"),
        ("cd", "change directory"),
        ("chdir", "change directory"),
        ("command", "run command"),
        ("compctl", "completion control"),
        ("compadd", "add completions"),
        ("compdef", "define completion"),
        ("compset", "modify completion"),
        ("continue", "next iteration"),
        ("declare", "declare variable"),
        ("dirs", "directory stack"),
        ("disown", "disown job"),
        ("echo", "print arguments"),
        ("emulate", "emulation mode"),
        ("enable", "enable builtin"),
        ("eval", "evaluate arguments"),
        ("exec", "replace shell"),
        ("exit", "exit shell"),
        ("export", "export variable"),
        ("false", "return false"),
        ("fc", "fix command"),
        ("fg", "foreground job"),
        ("float", "float variable"),
        ("functions", "list functions"),
        ("getln", "get line"),
        ("getopts", "parse options"),
        ("hash", "hash commands"),
        ("history", "command history"),
        ("integer", "integer variable"),
        ("jobs", "list jobs"),
        ("kill", "send signal"),
        ("let", "arithmetic"),
        ("limit", "resource limits"),
        ("local", "local variable"),
        ("log", "log message"),
        ("logout", "logout shell"),
        ("noglob", "no globbing"),
        ("popd", "pop directory"),
        ("print", "print output"),
        ("printf", "formatted print"),
        ("pushd", "push directory"),
        ("pushln", "push line"),
        ("pwd", "print directory"),
        ("read", "read input"),
        ("readonly", "readonly variable"),
        ("rehash", "rehash commands"),
        ("return", "return from function"),
        ("sched", "schedule command"),
        ("set", "set options"),
        ("setopt", "set option"),
        ("shift", "shift parameters"),
        ("source", "source file"),
        ("suspend", "suspend shell"),
        ("test", "test condition"),
        ("times", "process times"),
        ("trap", "signal trap"),
        ("true", "return true"),
        ("ttyctl", "tty control"),
        ("type", "command type"),
        ("typeset", "declare variable"),
        ("ulimit", "resource limits"),
        ("umask", "file mask"),
        ("unalias", "remove alias"),
        ("unfunction", "remove function"),
        ("unhash", "remove hash"),
        ("unlimit", "remove limit"),
        ("unset", "unset variable"),
        ("unsetopt", "unset option"),
        ("vared", "edit variable"),
        ("wait", "wait for jobs"),
        ("whence", "command location"),
        ("where", "command locations"),
        ("which", "command path"),
        ("zcompile", "compile file"),
        ("zformat", "format string"),
        ("zle", "line editor"),
        ("zmodload", "load module"),
        ("zparseopts", "parse options"),
        ("zprof", "profiling"),
        ("zpty", "pseudo terminal"),
        ("zregexparse", "regex parse"),
        ("zsocket", "socket ops"),
        ("zstat", "file stats"),
        ("zstyle", "style lookup"),
    ];

    let prefix_lower = prefix.to_lowercase();
    for (name, desc) in builtins {
        if name.starts_with(&prefix_lower) {
            let mut c = Completion::new(name.to_string());
            c.desc = Some(desc.to_string());
            group.matches.push(c);
        }
    }
    group
}

/// Complete files and directories
pub fn complete_files(prefix: &str, include_hidden: bool) -> CompletionGroup {
    let mut group = CompletionGroup::new("file");
    group.explanation = Some("file".to_string());

    let (dir, file_prefix) = if prefix.contains('/') {
        let idx = prefix.rfind('/').unwrap();
        let dir = if idx == 0 { "/" } else { &prefix[..idx] };
        (dir.to_string(), &prefix[idx + 1..])
    } else {
        (".".to_string(), prefix)
    };

    let dir_path = if dir.starts_with('~') {
        if let Some(home) = dirs::home_dir() {
            dir.replacen('~', &home.to_string_lossy(), 1)
        } else {
            dir.clone()
        }
    } else {
        dir.clone()
    };

    if let Ok(entries) = std::fs::read_dir(&dir_path) {
        let prefix_lower = file_prefix.to_lowercase();
        for entry in entries.take(200).flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if !include_hidden && name.starts_with('.') && !file_prefix.starts_with('.') {
                    continue;
                }
                if name.to_lowercase().starts_with(&prefix_lower) || file_prefix.is_empty() {
                    let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    let display = if dir == "." {
                        name.to_string()
                    } else if dir.ends_with('/') {
                        format!("{}{}", dir, name)
                    } else {
                        format!("{}/{}", dir, name)
                    };
                    let value = if is_dir {
                        format!("{}/", display)
                    } else {
                        display
                    };
                    let mut c = Completion::new(value);
                    if is_dir {
                        c.desc = Some("directory".to_string());
                    }
                    group.matches.push(c);
                }
            }
        }
    }
    group
}

/// Complete from a cached completion function
pub fn complete_from_cache_function(
    cache: &CompsysCache,
    cmd: &str,
    func: &str,
    words: &[&str],
    arg_num: usize,
    prefix: &str,
    _before_cursor: &str,
) -> Vec<CompletionGroup> {
    let mut groups = Vec::new();

    // Try to get the autoload stub to parse options from
    if let Ok(Some(stub)) = cache.get_autoload(func) {
        if let Ok(content) = std::fs::read_to_string(&stub.source) {
            // Parse _arguments specs from the function
            let mut opt_group = CompletionGroup::new("option");
            opt_group.explanation = Some(format!("{} option", cmd));

            for line in content.lines() {
                let line = line.trim();
                if line.starts_with('#') || !line.contains('[') {
                    continue;
                }
                // Parse option specifications like '-l[long listing]'
                for segment in line.split('\'') {
                    if let Some((opt, desc)) = parse_option_spec(segment) {
                        if opt.starts_with(prefix) || prefix.is_empty() {
                            let mut c = Completion::new(opt);
                            if !desc.is_empty() {
                                c.desc = Some(desc);
                            }
                            opt_group.matches.push(c);
                        }
                    }
                }
            }

            if !opt_group.matches.is_empty() {
                groups.push(opt_group);
            }
        }
    }

    // Git subcommands
    if cmd == "git" && arg_num == 1 {
        let mut sub_group = CompletionGroup::new("subcommand");
        sub_group.explanation = Some("git command".to_string());

        let subcommands = [
            ("add", "add files to index"),
            ("bisect", "binary search"),
            ("branch", "list/create branches"),
            ("checkout", "switch branches"),
            ("clone", "clone repository"),
            ("commit", "record changes"),
            ("diff", "show changes"),
            ("fetch", "download objects"),
            ("grep", "search files"),
            ("init", "create repository"),
            ("log", "show commits"),
            ("merge", "join branches"),
            ("mv", "move files"),
            ("pull", "fetch and merge"),
            ("push", "update remote"),
            ("rebase", "reapply commits"),
            ("reset", "reset HEAD"),
            ("restore", "restore files"),
            ("rm", "remove files"),
            ("show", "show objects"),
            ("stash", "stash changes"),
            ("status", "show status"),
            ("switch", "switch branches"),
            ("tag", "manage tags"),
        ];

        for (name, desc) in subcommands {
            if name.starts_with(prefix) || prefix.is_empty() {
                let mut c = Completion::new(name.to_string());
                c.desc = Some(desc.to_string());
                sub_group.matches.push(c);
            }
        }
        groups.push(sub_group);
    }

    // Cargo subcommands
    if cmd == "cargo" && arg_num == 1 {
        let mut sub_group = CompletionGroup::new("subcommand");
        sub_group.explanation = Some("cargo command".to_string());

        let subcommands = [
            ("build", "compile package"),
            ("check", "check package"),
            ("clean", "remove artifacts"),
            ("doc", "build documentation"),
            ("new", "create new package"),
            ("init", "init in directory"),
            ("run", "run binary"),
            ("test", "run tests"),
            ("bench", "run benchmarks"),
            ("update", "update deps"),
            ("search", "search crates"),
            ("publish", "publish crate"),
            ("install", "install binary"),
            ("uninstall", "remove binary"),
            ("add", "add dependency"),
            ("remove", "remove dependency"),
            ("fmt", "format code"),
            ("clippy", "lint code"),
            ("tree", "show dep tree"),
            ("fix", "auto-fix warnings"),
        ];

        for (name, desc) in subcommands {
            if name.starts_with(prefix) || prefix.is_empty() {
                let mut c = Completion::new(name.to_string());
                c.desc = Some(desc.to_string());
                sub_group.matches.push(c);
            }
        }
        groups.push(sub_group);
    }

    // Add file completions as fallback
    groups.push(complete_files(prefix, false));

    groups
}

fn parse_option_spec(spec: &str) -> Option<(String, String)> {
    let spec = spec.trim();
    if !spec.contains('-') {
        return None;
    }
    let opt_start = if spec.starts_with('(') {
        spec.find(')')?.checked_add(1)?
    } else {
        0
    };
    let rest = &spec[opt_start..];
    if !rest.starts_with('-') {
        return None;
    }
    let opt_end = rest
        .find(|c| c == '[' || c == '=' || c == ':' || c == ' ')
        .unwrap_or(rest.len());
    let opt_name = rest[..opt_end].trim_end_matches(|c| c == '+' || c == '=');
    if opt_name.is_empty() || opt_name == "-" || opt_name == "--" {
        return None;
    }
    let desc = if let Some(bracket_start) = rest.find('[') {
        if let Some(bracket_end) = rest[bracket_start..].find(']') {
            rest[bracket_start + 1..bracket_start + bracket_end].to_string()
        } else {
            String::new()
        }
    } else {
        String::new()
    };
    Some((opt_name.to_string(), desc))
}

/// Complete parameters (environment variables)
pub fn complete_parameters(prefix: &str) -> CompletionGroup {
    let mut group = CompletionGroup::new("parameter");
    group.explanation = Some("parameter".to_string());

    let prefix_upper = prefix.to_uppercase();
    for (key, value) in std::env::vars() {
        if key.to_uppercase().starts_with(&prefix_upper) {
            let mut c = Completion::new(key);
            let truncated = if value.len() > 40 {
                format!("{}...", &value[..40])
            } else {
                value
            };
            c.desc = Some(truncated);
            group.matches.push(c);
        }
    }
    group.matches.sort_by(|a, b| a.str_.cmp(&b.str_));
    group
}

/// Complete parameter flags for ${(flags)...}
pub fn complete_parameter_flags(prefix: &str) -> CompletionGroup {
    let mut group = CompletionGroup::new("parameter flag");
    group.explanation = Some("parameter expansion flag".to_string());

    let flags = [
        ("@", "array expansion"),
        ("A", "create assoc array"),
        ("a", "sort array"),
        ("c", "count characters"),
        ("C", "capitalize"),
        ("D", "named dir subst"),
        ("e", "expand escapes"),
        ("f", "split on newlines"),
        ("F", "join with newlines"),
        ("g", "glob patterns"),
        ("i", "sort case-insensitive"),
        ("j", "join words"),
        ("k", "assoc keys"),
        ("L", "lowercase"),
        ("n", "sort numerically"),
        ("o", "sort ascending"),
        ("O", "sort descending"),
        ("P", "dereference"),
        ("q", "quote special"),
        ("Q", "strip quotes"),
        ("s", "split on chars"),
        ("S", "shell quoting"),
        ("t", "type of param"),
        ("u", "unique elements"),
        ("U", "uppercase"),
        ("v", "assoc values"),
        ("V", "visible chars"),
        ("w", "count words"),
        ("W", "count words (alt)"),
        ("z", "split like shell"),
        ("Z", "split with quoting"),
    ];

    for (flag, desc) in flags {
        if flag.starts_with(prefix) || prefix.is_empty() {
            let mut c = Completion::new(flag.to_string());
            c.desc = Some(desc.to_string());
            group.matches.push(c);
        }
    }
    group
}

/// Complete glob qualifiers
pub fn complete_glob_qualifiers(prefix: &str) -> CompletionGroup {
    let mut group = CompletionGroup::new("glob qualifier");
    group.explanation = Some("glob qualifier".to_string());

    let qualifiers = [
        ("/", "directories"),
        (".", "plain files"),
        ("@", "symbolic links"),
        ("=", "sockets"),
        ("p", "named pipes"),
        ("*", "executables"),
        ("%", "device files"),
        ("r", "readable"),
        ("w", "writable"),
        ("x", "executable"),
        ("R", "world-readable"),
        ("W", "world-writable"),
        ("X", "world-executable"),
        ("s", "setuid"),
        ("S", "setgid"),
        ("t", "sticky bit"),
        ("U", "owned by EUID"),
        ("G", "owned by EGID"),
        ("u", "owned by user"),
        ("g", "owned by group"),
        ("a", "access time"),
        ("m", "modification time"),
        ("c", "inode change time"),
        ("L", "file size"),
        ("^", "negate"),
        ("-", "follow symlinks"),
        ("M", "mark directories"),
        ("T", "mark types"),
        ("N", "null glob"),
        ("D", "glob dots"),
        ("n", "numeric sort"),
        ("o", "sort order"),
        ("O", "reverse sort"),
        ("[", "subscript"),
        ("e", "execute code"),
        ("+", "glob function"),
    ];

    for (qual, desc) in qualifiers {
        if qual.starts_with(prefix) || prefix.is_empty() {
            let mut c = Completion::new(qual.to_string());
            c.desc = Some(desc.to_string());
            group.matches.push(c);
        }
    }
    group
}

/// Complete users and named directories
pub fn complete_users_and_named_dirs(cache: &CompsysCache, prefix: &str) -> Vec<CompletionGroup> {
    let mut groups = Vec::new();

    // Named directories from cache
    let named_dirs = if prefix.is_empty() {
        cache.get_named_dirs().unwrap_or_default()
    } else {
        cache.get_named_dirs_prefix(prefix).unwrap_or_default()
    };

    if !named_dirs.is_empty() {
        let mut nd_group = CompletionGroup::new("named directory");
        nd_group.explanation = Some("named directory".to_string());

        for (name, path) in named_dirs {
            let mut c = Completion::new(format!("~{}", name));
            c.desc = Some(path);
            nd_group.matches.push(c);
        }
        groups.push(nd_group);
    }

    // Users from /etc/passwd
    let mut user_group = CompletionGroup::new("user");
    user_group.explanation = Some("user".to_string());

    if let Ok(content) = std::fs::read_to_string("/etc/passwd") {
        let prefix_lower = prefix.to_lowercase();
        for line in content.lines() {
            if let Some(user) = line.split(':').next() {
                if user.to_lowercase().starts_with(&prefix_lower) {
                    let mut c = Completion::new(format!("~{}", user));
                    let parts: Vec<&str> = line.split(':').collect();
                    if parts.len() > 5 {
                        c.desc = Some(parts[5].to_string());
                    }
                    user_group.matches.push(c);
                }
            }
        }
    }

    user_group.matches.sort_by(|a, b| a.str_.cmp(&b.str_));
    user_group.matches.dedup_by(|a, b| a.str_ == b.str_);
    if !user_group.matches.is_empty() {
        groups.push(user_group);
    }

    groups
}

/// Complete math functions
pub fn complete_math_functions(prefix: &str) -> CompletionGroup {
    let mut group = CompletionGroup::new("math function");
    group.explanation = Some("math function".to_string());

    let functions = [
        ("abs", "absolute value"),
        ("acos", "arc cosine"),
        ("asin", "arc sine"),
        ("atan", "arc tangent"),
        ("cbrt", "cube root"),
        ("ceil", "ceiling"),
        ("cos", "cosine"),
        ("cosh", "hyperbolic cosine"),
        ("exp", "exponential"),
        ("floor", "floor"),
        ("log", "natural log"),
        ("log10", "base-10 log"),
        ("sin", "sine"),
        ("sinh", "hyperbolic sine"),
        ("sqrt", "square root"),
        ("tan", "tangent"),
    ];

    for (func, desc) in functions {
        if func.starts_with(prefix) || prefix.is_empty() {
            let mut c = Completion::new(func.to_string());
            c.desc = Some(desc.to_string());
            group.matches.push(c);
        }
    }
    group
}

/// Complete condition operators
pub fn complete_condition_operators(prefix: &str) -> CompletionGroup {
    let mut group = CompletionGroup::new("condition");
    group.explanation = Some("condition operator".to_string());

    let operators = [
        ("-a", "file exists"),
        ("-b", "block device"),
        ("-c", "character device"),
        ("-d", "directory"),
        ("-e", "exists"),
        ("-f", "regular file"),
        ("-g", "setgid"),
        ("-h", "symbolic link"),
        ("-k", "sticky bit"),
        ("-n", "non-empty string"),
        ("-o", "option set"),
        ("-p", "named pipe"),
        ("-r", "readable"),
        ("-s", "non-empty file"),
        ("-t", "terminal"),
        ("-u", "setuid"),
        ("-w", "writable"),
        ("-x", "executable"),
        ("-z", "empty string"),
        ("-L", "symbolic link"),
        ("-N", "modified since read"),
        ("-O", "owned by EUID"),
        ("-G", "owned by EGID"),
        ("-S", "socket"),
        ("-nt", "newer than"),
        ("-ot", "older than"),
        ("-ef", "same file"),
        ("-eq", "equal"),
        ("-ne", "not equal"),
        ("-lt", "less than"),
        ("-le", "less or equal"),
        ("-gt", "greater than"),
        ("-ge", "greater or equal"),
    ];

    for (op, desc) in operators {
        if op.starts_with(prefix) || prefix.is_empty() {
            let mut c = Completion::new(op.to_string());
            c.desc = Some(desc.to_string());
            group.matches.push(c);
        }
    }
    group
}

/// Complete subscript flags
pub fn complete_subscript_flags(prefix: &str) -> CompletionGroup {
    let mut group = CompletionGroup::new("subscript");
    group.explanation = Some("subscript flag".to_string());

    let flags = [
        ("@", "all elements"),
        ("*", "all as string"),
        ("#", "array length"),
        ("k", "keys"),
        ("v", "values"),
        ("K", "keys reversed"),
        ("V", "values reversed"),
    ];

    for (flag, desc) in flags {
        if flag.starts_with(prefix) || prefix.is_empty() {
            let mut c = Completion::new(flag.to_string());
            c.desc = Some(desc.to_string());
            group.matches.push(c);
        }
    }
    group
}

/// Complete generic options when no specific completion exists
pub fn complete_generic_options(prefix: &str) -> CompletionGroup {
    let mut group = CompletionGroup::new("option");
    group.explanation = Some("option".to_string());

    let options = [
        ("--help", "show help"),
        ("--version", "show version"),
        ("-h", "help"),
        ("-v", "verbose"),
        ("-V", "version"),
        ("-q", "quiet"),
        ("-f", "force"),
        ("-r", "recursive"),
        ("-n", "dry run"),
        ("-i", "interactive"),
    ];

    for (opt, desc) in options {
        if opt.starts_with(prefix) {
            let mut c = Completion::new(opt.to_string());
            c.desc = Some(desc.to_string());
            group.matches.push(c);
        }
    }
    group
}

/// Complete history modifiers
pub fn complete_history_modifiers(prefix: &str) -> CompletionGroup {
    let mut group = CompletionGroup::new("history");
    group.explanation = Some("history modifier".to_string());

    let modifiers = [
        ("!", "previous command"),
        ("!!", "last command"),
        ("!$", "last argument"),
        ("!^", "first argument"),
        ("!*", "all arguments"),
        ("!:0", "command name"),
        ("!:n", "nth argument"),
        ("!:n-m", "arguments n to m"),
        ("!:-n", "first n args"),
        ("!:n*", "args from n"),
        ("!#", "current line"),
        ("!?str", "search for str"),
    ];

    for (mod_, desc) in modifiers {
        if mod_.starts_with(prefix) || prefix.is_empty() {
            let mut c = Completion::new(mod_.to_string());
            c.desc = Some(desc.to_string());
            group.matches.push(c);
        }
    }
    group
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_option_spec() {
        // Basic option
        assert_eq!(
            parse_option_spec("-l[long listing]"),
            Some(("-l".to_string(), "long listing".to_string()))
        );

        // With exclusion group
        assert_eq!(
            parse_option_spec("(-A)-a[list entries starting with .]"),
            Some(("-a".to_string(), "list entries starting with .".to_string()))
        );

        // Long option
        assert_eq!(
            parse_option_spec("--help[show help]"),
            Some(("--help".to_string(), "show help".to_string()))
        );

        // Option without description
        assert_eq!(
            parse_option_spec("-v"),
            Some(("-v".to_string(), "".to_string()))
        );

        // Multiple exclusions
        assert_eq!(
            parse_option_spec("(-l -g -1 -C -m -x)-l[long listing]"),
            Some(("-l".to_string(), "long listing".to_string()))
        );

        // Not an option
        assert_eq!(parse_option_spec("something else"), None);
        assert_eq!(parse_option_spec("*: :_files"), None);
    }

    #[test]
    fn test_generate_completions_ls() {
        let cache_path = crate::cache::default_cache_path();

        if !cache_path.exists() {
            eprintln!("Skipping test: cache not found at {:?}", cache_path);
            return;
        }

        let cache = CompsysCache::open(&cache_path).unwrap();

        // Test "ls -" completion
        let groups = generate_completions(&cache, "ls -", 4);

        eprintln!("Got {} groups for 'ls -'", groups.len());
        for group in &groups {
            eprintln!("  Group '{}': {} matches", group.name, group.matches.len());
            for m in group.matches.iter().take(5) {
                eprintln!("    {} - {:?}", m.str_, m.desc);
            }
        }

        // Should have at least one group with options
        assert!(!groups.is_empty(), "Should have completion groups");

        // Find the option group
        let opt_group = groups.iter().find(|g| g.name == "option");
        assert!(opt_group.is_some(), "Should have an option group");

        let opt_group = opt_group.unwrap();
        assert!(
            !opt_group.matches.is_empty(),
            "Option group should have matches"
        );

        // Check for common ls options
        let opt_names: Vec<&str> = opt_group.matches.iter().map(|m| m.str_.as_str()).collect();
        assert!(
            opt_names.contains(&"-l"),
            "Should have -l option, got: {:?}",
            opt_names
        );
        assert!(
            opt_names.contains(&"-a"),
            "Should have -a option, got: {:?}",
            opt_names
        );
    }
}
