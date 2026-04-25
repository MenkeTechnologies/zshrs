//! Native Rust implementation of compinit - parallelized with rayon
//!
//! compinit is the slowest part of zsh startup. It:
//! 1. Scans all directories in fpath
//! 2. Reads first line of every _* file
//! 3. Parses #compdef/#autoload directives
//! 4. Registers completion functions
//!
//! This native implementation parallelizes the scanning with rayon.

use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

/// Completion definition from #compdef line
#[derive(Clone, Debug)]
pub enum CompDef {
    /// Regular command completion: #compdef cmd1 cmd2 ...
    Commands(Vec<String>),
    /// Pattern completion: #compdef -p 'pattern'
    Pattern(String),
    /// Post-pattern completion: #compdef -P 'pattern'
    PostPattern(String),
    /// Key binding: #compdef -k style key1 key2 ...
    KeyBinding { style: String, keys: Vec<String> },
    /// Widget key binding: #compdef -K widget style key
    WidgetKey {
        widget: String,
        style: String,
        key: String,
    },
}

/// Parsed completion file
#[derive(Clone, Debug)]
pub struct CompFile {
    /// Full path to the file
    pub path: PathBuf,
    /// Function name (filename without path)
    pub name: String,
    /// What this file defines
    pub def: CompFileDef,
    /// Full file body (read during scan for caching)
    pub body: Option<String>,
}

/// What a completion file defines
#[derive(Clone, Debug)]
pub enum CompFileDef {
    /// #compdef - completion function
    CompDef(CompDef),
    /// #autoload - helper function with options
    Autoload(Vec<String>),
    /// No recognized directive
    None,
}

/// Result of compinit scan
#[derive(Debug, Default)]
pub struct CompInitResult {
    /// Command -> function mapping (_comps)
    pub comps: HashMap<String, String>,
    /// Command -> service mapping (_services)
    pub services: HashMap<String, String>,
    /// Pattern -> function mapping (_patcomps)
    pub patcomps: HashMap<String, String>,
    /// Post-pattern -> function mapping (_postpatcomps)
    pub postpatcomps: HashMap<String, String>,
    /// Autoload functions with options (_compautos)
    pub compautos: HashMap<String, String>,
    /// All scanned files
    pub files: Vec<CompFile>,
    /// Scan duration
    pub scan_time_ms: u64,
    /// Number of directories scanned
    pub dirs_scanned: usize,
    /// Number of files scanned
    pub files_scanned: usize,
}

/// Parse the first line of a completion file
///
/// Handles all #compdef variants:
/// - `#compdef cmd1 cmd2` - regular commands
/// - `#compdef - cmd1 cmd2` - bare hyphen + commands (hyphen maps to '-')
/// - `#compdef -default-` - special context entries
/// - `#compdef -value-,VAR,-default-` - value context entries  
/// - `#compdef -p pattern` - pattern completions
/// - `#compdef -P pattern` - post-pattern completions
/// - `#compdef -k style key` - key bindings
/// - `#compdef -K widget style key` - widget key bindings
fn parse_first_line(line: &str) -> CompFileDef {
    let line = line.trim();

    if let Some(rest) = line.strip_prefix("#compdef") {
        let rest = rest.trim();
        if rest.is_empty() {
            return CompFileDef::None;
        }

        let parts: Vec<&str> = rest.split_whitespace().collect();
        if parts.is_empty() {
            return CompFileDef::None;
        }

        // Check for special options first
        match parts[0] {
            "-p" if parts.len() >= 2 => {
                CompFileDef::CompDef(CompDef::Pattern(parts[1].to_string()))
            }
            "-P" if parts.len() >= 2 => {
                // -P patterns go directly into _comps (not _patcomps!)
                // zsh puts these patterns as keys in _comps hash
                // Can have multiple: #compdef -P pattern1 -P pattern2 cmd1 cmd2
                let mut all_cmds = Vec::new();
                let mut i = 0;
                while i < parts.len() {
                    if parts[i] == "-P" && i + 1 < parts.len() {
                        // Pattern goes directly as a key in _comps
                        all_cmds.push(parts[i + 1].to_string());
                        i += 2;
                    } else if !parts[i].starts_with('-') || is_context_entry(parts[i]) {
                        all_cmds.push(parts[i].to_string());
                        i += 1;
                    } else {
                        i += 1;
                    }
                }
                if all_cmds.is_empty() {
                    CompFileDef::None
                } else {
                    CompFileDef::CompDef(CompDef::Commands(all_cmds))
                }
            }
            "-k" if parts.len() >= 3 => CompFileDef::CompDef(CompDef::KeyBinding {
                style: parts[1].to_string(),
                keys: parts[2..].iter().map(|s| s.to_string()).collect(),
            }),
            "-K" if parts.len() >= 4 => CompFileDef::CompDef(CompDef::WidgetKey {
                widget: parts[1].to_string(),
                style: parts[2].to_string(),
                key: parts[3].to_string(),
            }),
            _ => {
                // Parse command definitions, including:
                // - bare "-" (maps to '-' in _comps)
                // - context entries like "-default-", "-redirect-", "-value-,VAR,-default-"
                // - regular commands
                //
                // Skip actual option flags like "-n" but keep context entries
                let cmds: Vec<String> = parts
                    .iter()
                    .filter(|s| {
                        // Keep if:
                        // - bare hyphen "-"
                        // - context entry like "-foo-" or "-value-,X,-default-"
                        // - regular command (no leading hyphen)
                        **s == "-" || is_context_entry(s) || !s.starts_with('-')
                    })
                    .map(|s| s.to_string())
                    .collect();
                if cmds.is_empty() {
                    CompFileDef::None
                } else {
                    CompFileDef::CompDef(CompDef::Commands(cmds))
                }
            }
        }
    } else if let Some(rest) = line.strip_prefix("#autoload") {
        let opts: Vec<String> = rest.split_whitespace().map(|s| s.to_string()).collect();
        CompFileDef::Autoload(opts)
    } else {
        CompFileDef::None
    }
}

/// Check if a string is a zsh completion context entry
/// Context entries are like: -default-, -redirect-, -command-, -value-,VAR,-default-
/// Also handles service syntax: -redirect-,<,bunzip2=bunzip2
fn is_context_entry(s: &str) -> bool {
    if !s.starts_with('-') {
        return false;
    }
    // Strip service suffix for checking
    let base = s.split('=').next().unwrap_or(s);

    // Check if it's a known context pattern:
    // 1. Ends with '-' like -default-, -redirect-
    // 2. Contains comma (context specifiers like -redirect-,<,bunzip2 or -value-,VAR,-default-)
    // 3. But NOT single letter options like -p, -P, -k, -K, -n
    if base.len() <= 2 {
        return base == "-"; // bare hyphen is a context entry
    }

    base.ends_with('-') || base.contains(',')
}

/// Scan a single completion file - reads full body for caching
fn scan_file(path: &Path) -> Option<CompFile> {
    let name = path.file_name()?.to_string_lossy().to_string();

    // Must start with underscore
    if !name.starts_with('_') {
        return None;
    }

    // Skip certain patterns
    if name.contains(';')
        || name.contains('|')
        || name.contains('&')
        || name.ends_with('~')
        || name.ends_with(".zwc")
    {
        return None;
    }

    // Read entire file at once (will be cached in SQLite)
    let body = fs::read_to_string(path).ok()?;

    // Parse first line for directive
    let first_line = body.lines().next().unwrap_or("");
    let def = parse_first_line(first_line);

    Some(CompFile {
        path: path.to_path_buf(),
        name,
        def,
        body: Some(body),
    })
}

/// Scan a directory for completion files (parallel)
fn scan_directory(dir: &Path) -> Vec<CompFile> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();

    // Parallel scan of files within directory
    paths.par_iter().filter_map(|p| scan_file(p)).collect()
}

/// Initialize the completion system by scanning fpath
///
/// This is the main entry point - replaces the zsh compinit function.
/// Uses rayon for parallel directory and file scanning.
pub fn compinit(fpath: &[PathBuf]) -> CompInitResult {
    let start = Instant::now();

    // Track seen function names (first one wins)
    let seen: Mutex<HashSet<String>> = Mutex::new(HashSet::new());

    // Parallel scan of all directories
    let all_files: Vec<CompFile> = fpath
        .par_iter()
        .filter(|dir| dir.as_os_str() != "." && dir.exists())
        .flat_map(|dir| scan_directory(dir))
        .filter(|f| {
            let mut seen = seen.lock().unwrap();
            if seen.contains(&f.name) {
                false
            } else {
                seen.insert(f.name.clone());
                true
            }
        })
        .collect();

    let files_scanned = all_files.len();
    let dirs_scanned = fpath.len();

    // Build the result maps
    let mut result = CompInitResult {
        scan_time_ms: start.elapsed().as_millis() as u64,
        dirs_scanned,
        files_scanned,
        ..Default::default()
    };

    for file in &all_files {
        match &file.def {
            CompFileDef::CompDef(compdef) => {
                match compdef {
                    CompDef::Commands(cmds) => {
                        for cmd in cmds {
                            // Handle service syntax: cmd=service
                            if let Some(eq_pos) = cmd.find('=') {
                                let cmd_name = &cmd[..eq_pos];
                                let service = &cmd[eq_pos + 1..];
                                result.comps.insert(cmd_name.to_string(), file.name.clone());
                                result
                                    .services
                                    .insert(cmd_name.to_string(), service.to_string());
                            } else {
                                result.comps.insert(cmd.clone(), file.name.clone());
                            }
                        }
                    }
                    CompDef::Pattern(pat) => {
                        // -p patterns go to BOTH _comps and _patcomps (zsh behavior)
                        result.comps.insert(pat.clone(), file.name.clone());
                        result.patcomps.insert(pat.clone(), file.name.clone());
                    }
                    CompDef::PostPattern(pat) => {
                        result.postpatcomps.insert(pat.clone(), file.name.clone());
                    }
                    CompDef::KeyBinding { .. } | CompDef::WidgetKey { .. } => {
                        // Key bindings need to be handled by the shell
                    }
                }
            }
            CompFileDef::Autoload(opts) => {
                let opts_str = opts.join(" ");
                result.compautos.insert(file.name.clone(), opts_str);
            }
            CompFileDef::None => {}
        }
    }

    result.files = all_files;
    result
}

/// Dump the compinit state to a cache file
pub fn compdump(
    result: &CompInitResult,
    dump_path: &Path,
    zsh_version: &str,
) -> std::io::Result<()> {
    use std::io::Write;

    let mut file = File::create(dump_path)?;

    // Header line: #compdump <num_files> . <zsh_version>
    writeln!(file, "#compdump {} . {}", result.files_scanned, zsh_version)?;

    // Dump _comps
    writeln!(
        file,
        "typeset -gHA _comps _services _patcomps _postpatcomps _compautos"
    )?;
    writeln!(file, "_comps=(")?;
    for (cmd, func) in &result.comps {
        writeln!(
            file,
            "  '{}' '{}'",
            escape_zsh_string(cmd),
            escape_zsh_string(func)
        )?;
    }
    writeln!(file, ")")?;

    // Dump _services
    writeln!(file, "_services=(")?;
    for (cmd, svc) in &result.services {
        writeln!(
            file,
            "  '{}' '{}'",
            escape_zsh_string(cmd),
            escape_zsh_string(svc)
        )?;
    }
    writeln!(file, ")")?;

    // Dump _patcomps
    writeln!(file, "_patcomps=(")?;
    for (pat, func) in &result.patcomps {
        writeln!(
            file,
            "  '{}' '{}'",
            escape_zsh_string(pat),
            escape_zsh_string(func)
        )?;
    }
    writeln!(file, ")")?;

    // Dump _postpatcomps
    writeln!(file, "_postpatcomps=(")?;
    for (pat, func) in &result.postpatcomps {
        writeln!(
            file,
            "  '{}' '{}'",
            escape_zsh_string(pat),
            escape_zsh_string(func)
        )?;
    }
    writeln!(file, ")")?;

    // Dump _compautos
    writeln!(file, "_compautos=(")?;
    for (name, opts) in &result.compautos {
        writeln!(
            file,
            "  '{}' '{}'",
            escape_zsh_string(name),
            escape_zsh_string(opts)
        )?;
    }
    writeln!(file, ")")?;

    // Autoload all completion functions
    writeln!(file, "autoload -Uz \\")?;
    for file_info in &result.files {
        if matches!(file_info.def, CompFileDef::CompDef(_)) {
            writeln!(file, "  {} \\", file_info.name)?;
        }
    }
    writeln!(file)?;

    Ok(())
}

/// Check if dump file is valid and can be used
pub fn check_dump(dump_path: &Path, fpath: &[PathBuf], zsh_version: &str) -> bool {
    let file = match File::open(dump_path) {
        Ok(f) => f,
        Err(_) => return false,
    };

    let mut reader = BufReader::new(file);
    let mut first_line = String::new();
    if reader.read_line(&mut first_line).is_err() {
        return false;
    }

    // Parse header: #compdump <num_files> . <version>
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() < 4 || parts[0] != "#compdump" {
        return false;
    }

    let stored_count: usize = match parts[1].parse() {
        Ok(n) => n,
        Err(_) => return false,
    };

    let stored_version = parts[3];

    // Quick count of files in fpath
    let current_count: usize = fpath
        .par_iter()
        .filter(|dir| dir.as_os_str() != "." && dir.exists())
        .map(|dir| {
            fs::read_dir(dir)
                .map(|entries| {
                    entries
                        .filter_map(|e| e.ok())
                        .filter(|e| e.file_name().to_string_lossy().starts_with('_'))
                        .count()
                })
                .unwrap_or(0)
        })
        .sum();

    stored_count == current_count && stored_version == zsh_version
}

/// Escape a string for zsh single quotes
fn escape_zsh_string(s: &str) -> String {
    s.replace('\'', "'\\''")
}

/// Build SQLite cache from fpath scan
///
/// This is the main entry point for initializing the completion system.
/// It scans fpath directories, parses #compdef directives, and populates
/// the SQLite cache for fast lookups.
pub fn build_cache_from_fpath(
    fpath: &[PathBuf],
    cache: &mut crate::cache::CompsysCache,
) -> std::io::Result<CompInitResult> {
    use std::time::Instant;

    let t0 = Instant::now();
    let result = compinit(fpath);
    let scan_time = t0.elapsed();

    let t1 = Instant::now();

    // Populate comps table (_comps hash)
    let comps: Vec<(String, String)> = result
        .comps
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    cache
        .set_comps_bulk(&comps)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

    // Populate services table (_services hash)
    let services: Vec<(String, String)> = result
        .services
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    cache
        .set_services_bulk(&services)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

    // Populate patcomps table (_patcomps hash)
    for (pattern, function) in &result.patcomps {
        cache
            .set_patcomp(pattern, function)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    }

    // Populate postpatcomps (stored in patcomps with a marker, or separate table if needed)
    // For now, we'll merge them into patcomps
    for (pattern, function) in &result.postpatcomps {
        cache
            .set_patcomp(pattern, function)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    }

    let comps_time = t1.elapsed();
    let t2 = Instant::now();

    // Populate autoloads table with function bodies for instant loading
    // Bodies were already read during parallel scan - no extra I/O here
    let autoloads: Vec<(String, String, String)> = result
        .files
        .iter()
        .filter(|f| matches!(f.def, CompFileDef::CompDef(_) | CompFileDef::Autoload(_)))
        .filter_map(|f| {
            let path_str = f.path.to_string_lossy().to_string();
            let body = f.body.as_ref()?.clone();
            Some((f.name.clone(), path_str, body))
        })
        .collect();
    cache
        .add_autoloads_with_bodies_bulk(&autoloads)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

    let autoloads_time = t2.elapsed();

    // Timing logged by caller in exec.rs via tracing

    Ok(result)
}

/// Load _comps from existing cache (instantaneous)
///
/// Returns a CompInitResult populated from the SQLite cache without rescanning fpath.
/// Use this after the cache has been built with `build_cache_from_fpath`.
///
/// This is the equivalent of `compinit -C` with a valid zcompdump - it skips
/// the fpath scan entirely and just loads from cache.
pub fn load_from_cache(cache: &crate::cache::CompsysCache) -> std::io::Result<CompInitResult> {
    use std::time::Instant;
    let start = Instant::now();

    let mut result = CompInitResult::default();

    // Load comps - single query
    result.comps = cache
        .get_all_comps()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;

    // Load patcomps - single query
    for (pat, func) in cache
        .patcomps_kv()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?
    {
        result.patcomps.insert(pat, func);
    }

    // Services are loaded on-demand via cache.get_service() - no need to preload
    // This matches zsh behavior where $_services is lazily populated

    result.scan_time_ms = start.elapsed().as_millis() as u64;
    result.files_scanned = result.comps.len();

    Ok(result)
}

/// Fast check if compinit is needed
///
/// Returns the number of completion entries in cache, or 0 if cache is empty/invalid.
/// Use this to decide whether to run full compinit or load_from_cache.
pub fn cache_entry_count(cache: &crate::cache::CompsysCache) -> usize {
    cache.comp_count().unwrap_or(0) as usize
}

/// Lazy compinit - validates cache exists but doesn't load into memory
///
/// This is the fastest option for shell startup. It just verifies the cache
/// is valid and returns immediately. Actual lookups happen via cache.get_comp().
///
/// Returns (is_valid, entry_count) in microseconds.
pub fn compinit_lazy(cache: &crate::cache::CompsysCache) -> (bool, usize) {
    let count = cache.comp_count().unwrap_or(0) as usize;
    (count > 0, count)
}

/// Check if cache is valid and up-to-date
///
/// Returns true if cache exists and has entries, false if cache needs to be rebuilt.
pub fn cache_is_valid(cache: &crate::cache::CompsysCache) -> bool {
    cache.comp_count().unwrap_or(0) > 0
}

/// Get system fpath from environment or defaults
pub fn get_system_fpath() -> Vec<PathBuf> {
    // Try FPATH env var first
    if let Ok(fpath_str) = std::env::var("FPATH") {
        if !fpath_str.is_empty() {
            return fpath_str.split(':').map(PathBuf::from).collect();
        }
    }

    // Default paths for common systems
    let mut paths = Vec::new();

    // macOS Homebrew
    for base in &["/opt/homebrew", "/usr/local"] {
        paths.push(PathBuf::from(format!("{}/share/zsh/site-functions", base)));
        paths.push(PathBuf::from(format!("{}/share/zsh/functions", base)));
    }

    // System zsh
    for version in &["5.9", "5.8", "5.7"] {
        paths.push(PathBuf::from(format!(
            "/usr/share/zsh/{}/functions",
            version
        )));
    }
    paths.push(PathBuf::from("/usr/share/zsh/functions"));
    paths.push(PathBuf::from("/usr/share/zsh/site-functions"));

    // Zinit/zplugin common paths
    if let Ok(home) = std::env::var("HOME") {
        paths.push(PathBuf::from(format!("{}/.zinit/completions", home)));
        paths.push(PathBuf::from(format!("{}/.zplugin/completions", home)));
        paths.push(PathBuf::from(format!(
            "{}/.local/share/zsh/site-functions",
            home
        )));
    }

    // Filter to existing directories
    paths.into_iter().filter(|p| p.exists()).collect()
}

/// Options for compinit
#[derive(Clone, Debug, Default)]
pub struct CompInitOpts {
    /// Dump file path (-d)
    pub dump_file: Option<PathBuf>,
    /// Skip dump (-D)
    pub no_dump: bool,
    /// Skip security check (-C)
    pub no_check: bool,
    /// Ignore insecure dirs (-i)
    pub ignore_insecure: bool,
    /// Use insecure dirs (-u)
    pub use_insecure: bool,
}

impl CompInitOpts {
    /// Parse compinit arguments
    pub fn parse(args: &[String]) -> Self {
        let mut opts = Self::default();
        let mut i = 0;

        while i < args.len() {
            match args[i].as_str() {
                "-d" => {
                    if i + 1 < args.len() && !args[i + 1].starts_with('-') {
                        opts.dump_file = Some(PathBuf::from(&args[i + 1]));
                        i += 1;
                    }
                }
                "-D" => opts.no_dump = true,
                "-C" => opts.no_check = true,
                "-i" => opts.ignore_insecure = true,
                "-u" => opts.use_insecure = true,
                _ => {}
            }
            i += 1;
        }

        opts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_compdef_commands() {
        let def = parse_first_line("#compdef git svn hg");
        match def {
            CompFileDef::CompDef(CompDef::Commands(cmds)) => {
                assert_eq!(cmds, vec!["git", "svn", "hg"]);
            }
            _ => panic!("Expected Commands"),
        }
    }

    #[test]
    fn test_parse_compdef_pattern() {
        let def = parse_first_line("#compdef -p 'c*'");
        match def {
            CompFileDef::CompDef(CompDef::Pattern(pat)) => {
                assert_eq!(pat, "'c*'");
            }
            _ => panic!("Expected Pattern"),
        }
    }

    #[test]
    fn test_parse_autoload() {
        let def = parse_first_line("#autoload -U -z");
        match def {
            CompFileDef::Autoload(opts) => {
                assert_eq!(opts, vec!["-U", "-z"]);
            }
            _ => panic!("Expected Autoload"),
        }
    }

    #[test]
    fn test_parse_compdef_key() {
        let def = parse_first_line("#compdef -k complete-word ^X^C");
        match def {
            CompFileDef::CompDef(CompDef::KeyBinding { style, keys }) => {
                assert_eq!(style, "complete-word");
                assert_eq!(keys, vec!["^X^C"]);
            }
            _ => panic!("Expected KeyBinding"),
        }
    }

    #[test]
    fn test_escape_zsh_string() {
        assert_eq!(escape_zsh_string("hello"), "hello");
        assert_eq!(escape_zsh_string("it's"), "it'\\''s");
    }

    #[test]
    fn test_parse_compdef_redirect_context() {
        // _bzip2 line: has regular commands + context entries with services
        let def = parse_first_line("#compdef bzip2 bunzip2 bzcat=bunzip2 bzip2recover -redirect-,<,bunzip2=bunzip2 -redirect-,>,bzip2=bunzip2 -redirect-,<,bzip2=bzip2");
        match def {
            CompFileDef::CompDef(CompDef::Commands(cmds)) => {
                // Should contain all entries
                assert!(cmds.contains(&"bzip2".to_string()), "missing bzip2");
                assert!(cmds.contains(&"bunzip2".to_string()), "missing bunzip2");
                assert!(
                    cmds.contains(&"bzcat=bunzip2".to_string()),
                    "missing bzcat=bunzip2"
                );
                assert!(
                    cmds.contains(&"bzip2recover".to_string()),
                    "missing bzip2recover"
                );
                assert!(
                    cmds.contains(&"-redirect-,<,bunzip2=bunzip2".to_string()),
                    "missing redirect bunzip2"
                );
                assert!(
                    cmds.contains(&"-redirect-,>,bzip2=bunzip2".to_string()),
                    "missing redirect >,bzip2"
                );
                assert!(
                    cmds.contains(&"-redirect-,<,bzip2=bzip2".to_string()),
                    "missing redirect <,bzip2"
                );
                assert_eq!(cmds.len(), 7, "cmds: {:?}", cmds);
            }
            other => panic!("Expected Commands, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_compdef_context_entries() {
        // -default- style entries
        let def = parse_first_line("#compdef -default-");
        match def {
            CompFileDef::CompDef(CompDef::Commands(cmds)) => {
                assert_eq!(cmds, vec!["-default-"]);
            }
            other => panic!("Expected Commands, got {:?}", other),
        }

        // bare hyphen + commands
        let def = parse_first_line("#compdef - nohup eval time");
        match def {
            CompFileDef::CompDef(CompDef::Commands(cmds)) => {
                assert!(cmds.contains(&"-".to_string()));
                assert!(cmds.contains(&"nohup".to_string()));
                assert!(cmds.contains(&"eval".to_string()));
                assert!(cmds.contains(&"time".to_string()));
            }
            other => panic!("Expected Commands, got {:?}", other),
        }

        // -value- entries
        let def = parse_first_line("#compdef -value- -array-value- -value-,-default-,-default-");
        match def {
            CompFileDef::CompDef(CompDef::Commands(cmds)) => {
                assert!(cmds.contains(&"-value-".to_string()));
                assert!(cmds.contains(&"-array-value-".to_string()));
                assert!(cmds.contains(&"-value-,-default-,-default-".to_string()));
            }
            other => panic!("Expected Commands, got {:?}", other),
        }
    }

    #[test]
    fn test_is_context_entry() {
        assert!(is_context_entry("-default-"));
        assert!(is_context_entry("-redirect-"));
        assert!(is_context_entry("-value-,DISPLAY,-default-"));
        assert!(is_context_entry("-redirect-,<,bunzip2=bunzip2"));
        assert!(is_context_entry("-redirect-,>,bzip2"));
        assert!(!is_context_entry("-p")); // option flag, not context
        assert!(!is_context_entry("-P")); // option flag
        assert!(!is_context_entry("git")); // regular command
    }
}
