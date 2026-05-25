//! Native Rust implementation of `compdump` and `check_dump`.
//!
//! Mirrors upstream `Completion/compdump` — writes/reads the
//! `.zcompdump` cache file consulted by `compinit` for fast startup.
//!
//! Cache format (one big eval-friendly shell file):
//!   `#compdump <num_files> . <zsh_version>`  (validation header)
//!   `typeset -gHA _comps _services _patcomps _postpatcomps _compautos`
//!   `_comps=( … )` `_services=( … )` `_patcomps=( … )`
//!   `_postpatcomps=( … )` `_compautos=( … )`
//!   `autoload -Uz <fn> \ …`
//!
//! `check_dump` returns true iff the on-disk header (file count + zsh
//! version) still matches the current fpath — used as the staleness
//! check for `compinit -C` fast-path.

use rayon::prelude::*;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use super::compinit::{CompFileDef, CompInitResult};

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
pub(super) fn escape_zsh_string(s: &str) -> String {
    s.replace('\'', "'\\''")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_zsh_string() {
        assert_eq!(escape_zsh_string("hello"), "hello");
        assert_eq!(escape_zsh_string("it's"), "it'\\''s");
    }
}
