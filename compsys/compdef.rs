//! compdef - Register completion functions for commands
//!
//! This is the core function that wires commands to their completion functions.
//!
//! Usage:
//!   compdef _git git
//!   compdef _docker docker docker-compose
//!   compdef '_files -g "*.h"' foo
//!   compdef -p '*-hierarchical' _hierarchy
//!   compdef -d git  # delete

use crate::cache::CompsysCache;

/// compdef options
#[derive(Debug, Default)]
pub struct CompdefOpts {
    /// -a: also register autoload
    pub autoload: bool,
    /// -n: don't overwrite existing
    pub no_overwrite: bool,
    /// -e: evaluate as shell code
    pub eval: bool,
    /// -d: delete registration
    pub delete: bool,
    /// -p: pattern (tried initially)
    pub pattern_initial: bool,
    /// -P: pattern (tried finally)
    pub pattern_final: bool,
    /// -N: normal command (after -p/-P)
    pub normal_after_pattern: bool,
    /// -k: key binding style
    pub key_style: Option<String>,
    /// -K: multiple key bindings
    pub multi_key: bool,
}

impl CompdefOpts {
    pub fn parse(args: &[String]) -> (Self, Vec<String>) {
        let mut opts = Self::default();
        let mut rest = Vec::new();
        let mut i = 0;

        while i < args.len() {
            let arg = &args[i];

            if !arg.starts_with('-') || arg == "-" {
                rest.extend(args[i..].iter().cloned());
                break;
            }

            match arg.as_str() {
                "-a" => opts.autoload = true,
                "-n" => opts.no_overwrite = true,
                "-e" => opts.eval = true,
                "-d" => opts.delete = true,
                "-p" => opts.pattern_initial = true,
                "-P" => opts.pattern_final = true,
                "-N" => opts.normal_after_pattern = true,
                "-K" => opts.multi_key = true,
                "-k" => {
                    i += 1;
                    if i < args.len() {
                        opts.key_style = Some(args[i].clone());
                    }
                }
                _ => {
                    // Combined flags like -an
                    for c in arg[1..].chars() {
                        match c {
                            'a' => opts.autoload = true,
                            'n' => opts.no_overwrite = true,
                            'e' => opts.eval = true,
                            'd' => opts.delete = true,
                            'p' => opts.pattern_initial = true,
                            'P' => opts.pattern_final = true,
                            'N' => opts.normal_after_pattern = true,
                            'K' => opts.multi_key = true,
                            _ => {}
                        }
                    }
                }
            }
            i += 1;
        }

        (opts, rest)
    }
}

/// Execute compdef command
///
/// Returns: 0 on success, 1 on error
pub fn compdef_execute(cache: &mut CompsysCache, args: &[String]) -> i32 {
    if args.is_empty() {
        return 1;
    }

    let (opts, rest) = CompdefOpts::parse(args);

    if rest.is_empty() {
        return 1;
    }

    // Handle delete mode
    if opts.delete {
        for name in &rest {
            let _ = cache.delete_comp(name);
        }
        return 0;
    }

    // First arg is the function
    let function = &rest[0];
    let commands = &rest[1..];

    if commands.is_empty() {
        return 1;
    }

    // Handle service assignments (cmd=service)
    let mut mode = CompdefMode::Normal;

    for cmd in commands {
        // Check for mode switches
        if cmd == "-p" {
            mode = CompdefMode::PatternInitial;
            continue;
        }
        if cmd == "-P" {
            mode = CompdefMode::PatternFinal;
            continue;
        }
        if cmd == "-N" {
            mode = CompdefMode::Normal;
            continue;
        }

        // Handle cmd=service format
        if let Some(eq_pos) = cmd.find('=') {
            let cmd_name = &cmd[..eq_pos];
            let service = &cmd[eq_pos + 1..];

            // Register with service indirection
            if opts.no_overwrite {
                if cache.get_comp(cmd_name).unwrap_or(None).is_some() {
                    continue;
                }
            }

            // Store the service mapping
            let _ = cache.set_service(cmd_name, service);
            // Also register the comp
            let _ = cache.set_comp(cmd_name, function);
            continue;
        }

        // Register based on mode
        match mode {
            CompdefMode::Normal => {
                if opts.no_overwrite {
                    if cache.get_comp(cmd).unwrap_or(None).is_some() {
                        continue;
                    }
                }
                let _ = cache.set_comp(cmd, function);
            }
            CompdefMode::PatternInitial => {
                let _ = cache.set_patcomp(cmd, function);
            }
            CompdefMode::PatternFinal => {
                // Store with special marker for "final" patterns
                let _ = cache.set_patcomp(&format!("{}:final", cmd), function);
            }
        }

        // Handle autoload
        if opts.autoload && function.starts_with('_') {
            let _ = cache.add_autoload(function, "compdef", 0, 0);
        }
    }

    0
}

#[derive(Debug, Clone, Copy)]
enum CompdefMode {
    Normal,
    PatternInitial,
    PatternFinal,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_opts() {
        let args: Vec<String> = vec!["-an", "_git", "git"]
            .into_iter()
            .map(String::from)
            .collect();
        let (opts, rest) = CompdefOpts::parse(&args);
        assert!(opts.autoload);
        assert!(opts.no_overwrite);
        assert_eq!(rest, vec!["_git", "git"]);
    }

    #[test]
    fn test_parse_delete() {
        let args: Vec<String> = vec!["-d", "git", "docker"]
            .into_iter()
            .map(String::from)
            .collect();
        let (opts, rest) = CompdefOpts::parse(&args);
        assert!(opts.delete);
        assert_eq!(rest, vec!["git", "docker"]);
    }

    #[test]
    fn test_compdef_basic() {
        let mut cache = CompsysCache::memory().unwrap();

        let args: Vec<String> = vec!["_git", "git", "git-commit", "git-push"]
            .into_iter()
            .map(String::from)
            .collect();

        let ret = compdef_execute(&mut cache, &args);
        assert_eq!(ret, 0);

        assert_eq!(cache.get_comp("git").unwrap(), Some("_git".to_string()));
        assert_eq!(
            cache.get_comp("git-commit").unwrap(),
            Some("_git".to_string())
        );
        assert_eq!(
            cache.get_comp("git-push").unwrap(),
            Some("_git".to_string())
        );
    }

    #[test]
    fn test_compdef_service() {
        let mut cache = CompsysCache::memory().unwrap();

        let args: Vec<String> = vec!["_git", "hub=git"]
            .into_iter()
            .map(String::from)
            .collect();

        let ret = compdef_execute(&mut cache, &args);
        assert_eq!(ret, 0);

        assert_eq!(cache.get_comp("hub").unwrap(), Some("_git".to_string()));
        assert_eq!(cache.get_service("hub").unwrap(), Some("git".to_string()));
    }

    #[test]
    fn test_compdef_pattern() {
        let mut cache = CompsysCache::memory().unwrap();

        let args: Vec<String> = vec!["_git", "-p", "git-*"]
            .into_iter()
            .map(String::from)
            .collect();

        let ret = compdef_execute(&mut cache, &args);
        assert_eq!(ret, 0);

        assert_eq!(
            cache.find_patcomp("git-foo").unwrap(),
            Some("_git".to_string())
        );
    }

    #[test]
    fn test_compdef_no_overwrite() {
        let mut cache = CompsysCache::memory().unwrap();

        // First registration
        let args: Vec<String> = vec!["_git", "git"].into_iter().map(String::from).collect();
        compdef_execute(&mut cache, &args);

        // Try to overwrite with -n
        let args: Vec<String> = vec!["-n", "_other", "git"]
            .into_iter()
            .map(String::from)
            .collect();
        compdef_execute(&mut cache, &args);

        // Should still be _git
        assert_eq!(cache.get_comp("git").unwrap(), Some("_git".to_string()));
    }

    #[test]
    fn test_compdef_delete() {
        let mut cache = CompsysCache::memory().unwrap();

        // Register
        let args: Vec<String> = vec!["_git", "git"].into_iter().map(String::from).collect();
        compdef_execute(&mut cache, &args);
        assert_eq!(cache.get_comp("git").unwrap(), Some("_git".to_string()));

        // Delete
        let args: Vec<String> = vec!["-d", "git"].into_iter().map(String::from).collect();
        compdef_execute(&mut cache, &args);
        assert_eq!(cache.get_comp("git").unwrap(), None);
    }
}
