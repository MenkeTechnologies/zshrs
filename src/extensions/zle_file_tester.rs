//! Port of fish-shell's `highlight/file_tester.rs` (vendor/fish/highlight/file_tester.rs).
//!
//! fish:1-4 — Support for testing whether files exist and have the correct permissions,
//! to support highlighting. Because this may perform blocking I/O, we compute results in a
//! separate thread, and provide them optimistically.
//!
//! zshrs substrate swaps (each cited at its site):
//!   * `wstr`/`WString`                → `&str`/`String`
//!   * fish expansion markers (ANY_CHAR, VARIABLE_EXPAND, …) → zsh lexer token chars
//!     (`Pound`..`Nularg`, zsh_h.rs:144-204); INTERNAL_SEPARATOR ↔ `Nularg`/quote-nulls
//!   * `unescape_string(Script|SPECIAL)` → inputs arrive as zsh *tokenized* strings from
//!     the lexer (`tokstr`); clean fragment built via `untokenize` (lex.rs:4812)
//!   * `expand_one(FAIL_ON_CMDSUBST)`  → cmdsubst-token guard + `singsub` (subst.rs:1460)
//!   * `expand_tilde`                  → `filesubstr` (subst.rs:1921, zsh Src/subst.c filesub)
//!   * `wstat`/`waccess`/`DirIter`     → `std::fs::metadata`/`libc::access`/`std::fs::read_dir`
//!   * `fpathconf(fd, _PC_CASE_SENSITIVE)` → `pathconf(path, …)` (read_dir exposes no fd)
//!
//! Also ports the three small fish helpers this file leans on, so the logic stays faithful:
//!   * `normalize_path`                — fish wutil/mod.rs:170-211
//!   * `path_apply_working_directory`  — fish path.rs:505-531
//!   * `RedirectionMode`               — fish redirection.rs:10-23
//!   * `fish_wcstoi`                   — fish wutil strtol-style base-10 integer parse

#![allow(non_snake_case)]

use crate::ported::lex::untokenize;
use crate::ported::params::getsparam;
use crate::ported::subst::{filesubstr, singsub};
use crate::ported::utils::errflag;
use crate::ported::zsh_h::{
    Bang, Bnull, Bnullkeep, Comma, Dash, Dnull, Inpar, Inparmath, Nularg, Qtick, Snull, Tick,
    Tilde,
};
use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// fish:29-30 — This is used only internally to this file, and is exposed only for testing.
/// fish:31-42 — `PathFlags`.
#[derive(Clone, Copy, Default)]
pub struct PathFlags {
    // fish:33 — The path must be to a directory.
    pub require_dir: bool,
    // fish:36 — Expand any leading tilde in the path.
    pub expand_tilde: bool,
    // fish:39 — Normalize directories before resolving, as "cd".
    pub for_cd: bool,
}

// fish:44-45 — When a file test is OK, we may also return whether this was a file.
// This is used for underlining and is dependent on the particular file test.
/// fish:47-48 — `IsFile`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct IsFile(pub bool);
/// fish:49-51 — `IsErr`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct IsErr;

/// fish:53-54 — The result of a file test.
pub type FileTestResult = Result<IsFile, IsErr>;

/// fish:redirection.rs:10-23 — `RedirectionMode`. zsh redir-type mapping happens at the
/// highlighter call site (WRITE→Overwrite, APP→Append, READ→Input, MERGEIN/MERGEOUT→Fd,
/// noclobber WRITE→NoClob). `TryInput` (`<?`) has no zsh spelling but is kept for parity.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RedirectionMode {
    /// fish:redirection.rs:12 — normal redirection: > file.txt
    Overwrite,
    /// fish:redirection.rs:14 — appending redirection: >> file.txt
    Append,
    /// fish:redirection.rs:16 — input redirection: < file.txt
    Input,
    /// fish:redirection.rs:18 — try-input redirection: <? file.txt
    TryInput,
    /// fish:redirection.rs:20 — fd redirection: 2>&1
    Fd,
    /// fish:redirection.rs:22 — noclobber redirection: >? file.txt (zsh: > with NO_CLOBBER)
    NoClob,
}

/// !!! WARNING: RUST-ONLY ADAPTER — NO DIRECT FISH COUNTERPART SHAPE !!!
/// fish's `OperationContext` (operation_context.rs) bundles an `Environment` + expansion
/// limits + a cancel checker. zshrs reads params via `getsparam` directly, so only the
/// cancel signal is carried. The cancel flag exists so a future async compute thread can
/// abandon stale requests exactly like fish's background highlight thread does.
pub struct OperationContext {
    cancel_flag: Arc<AtomicBool>,
    /// Wall-clock budget: past this instant, check_cancel() reports cancelled.
    /// fish runs its file tests on a background thread with unbounded time;
    /// zshrs computes synchronously on the ZLE thread, so every IO loop
    /// (directory walks, PATH scans, candidate validation) must be bounded
    /// or a large directory turns each keystroke into a lag spike. All the
    /// loops already poll check_cancel() — the fish shape — so the deadline
    /// composes without touching them.
    deadline: Option<std::time::Instant>,
}

impl OperationContext {
    /// fish:operation_context.rs `OperationContext::empty()` — a context that never cancels.
    pub fn empty() -> Self {
        Self {
            cancel_flag: Arc::new(AtomicBool::new(false)),
            deadline: None,
        }
    }

    /// Build a context sharing an external cancel flag.
    pub fn with_cancel_flag(cancel_flag: Arc<AtomicBool>) -> Self {
        Self {
            cancel_flag,
            deadline: None,
        }
    }

    /// A context that self-cancels after `budget` (see `deadline` field docs).
    pub fn with_budget(budget: std::time::Duration) -> Self {
        Self {
            cancel_flag: Arc::new(AtomicBool::new(false)),
            deadline: Some(std::time::Instant::now() + budget),
        }
    }

    /// fish:operation_context.rs `check_cancel()`.
    pub fn check_cancel(&self) -> bool {
        self.cancel_flag.load(Ordering::Relaxed)
            || self
                .deadline
                .is_some_and(|d| std::time::Instant::now() >= d)
    }
}

/// fish:55-63 — `FileTester`.
pub struct FileTester<'s> {
    // fish:57-59 — The working directory, for resolving paths against.
    working_directory: String,
    // fish:60-62 — The operation context.
    ctx: &'s OperationContext,
}

impl<'s> FileTester<'s> {
    /// fish:66-72 — `new`.
    pub fn new(working_directory: String, ctx: &'s OperationContext) -> Self {
        Self {
            working_directory,
            ctx,
        }
    }

    /// fish:74-77 — Test whether a file exists and is readable.
    /// The input string 'token' is given as a *tokenized* string (as produced by the zsh
    /// lexer; fish receives an escaped-as-typed string — same information, zsh spelling).
    /// If 'prefix' is true, instead check if the path is a prefix of a valid path.
    /// Returns false on cancellation.
    pub fn test_path(&self, token: &str, prefix: bool) -> bool {
        // fish:79-83 — Skip strings exceeding PATH_MAX. See fish#7837.
        // Note some paths may exceed PATH_MAX, but this is just for highlighting.
        if token.len() > (libc::PATH_MAX as usize) {
            return false;
        }

        // fish:85-96 — fish unescapes here and re-plants '~' over HOME_DIRECTORY. zshrs
        // inputs stay tokenized: `is_potential_path` untokenizes and tilde-expands itself,
        // so the token passes through unchanged.
        is_potential_path(
            token,
            prefix,
            std::slice::from_ref(&self.working_directory),
            self.ctx,
            PathFlags {
                expand_tilde: true,
                ..Default::default()
            },
        )
    }

    // fish:110-112 — Test if the string is a prefix of a valid path we could cd into, or is
    // some other token we recognize (primarily --help).
    // If is_prefix is true, we test if the string is a prefix of a valid path we could cd into.
    /// fish:113-141 — `test_cd_path`.
    pub fn test_cd_path(&self, token: &str, is_prefix: bool) -> FileTestResult {
        let mut param = token.to_owned(); // fish:115
        if !expand_one_no_cmdsubst(&mut param) {
            // fish:116-119 — Failed expansion (e.g. may contain a command substitution).
            // Ignore it.
            return FileTestResult::Ok(IsFile(false));
        }
        // fish:120-124 — Maybe it's just --help.
        if "--help".starts_with(&param) || "-h".starts_with(&param) {
            return FileTestResult::Ok(IsFile(false));
        }
        let valid_path = is_potential_cd_path(
            &param,
            is_prefix,
            &self.working_directory,
            self.ctx,
            PathFlags {
                expand_tilde: true,
                ..Default::default()
            },
        ); // fish:125-134
           // fish:135-140 — cd into an invalid path is an error.
        if valid_path {
            Ok(IsFile(valid_path))
        } else {
            Err(IsErr)
        }
    }

    // fish:143-145 — Test if the given string is a valid redirection target, and if so,
    // whether it is a path to an existing file.
    /// fish:146-231 — `test_redirection_target`.
    pub fn test_redirection_target(&self, target: &str, mode: RedirectionMode) -> FileTestResult {
        // fish:147-150 — Skip targets exceeding PATH_MAX. See fish#7837.
        if target.len() > (libc::PATH_MAX as usize) {
            return Err(IsErr);
        }
        let mut target = target.to_owned(); // fish:151
        if !expand_one_no_cmdsubst(&mut target) {
            // fish:152-155 — Could not be expanded.
            return Err(IsErr);
        }
        // fish:156-159 — Ok, we successfully expanded our target. Now verify that it works
        // with this redirection. We will probably need it as a path (but not in the case of
        // fd redirections). Note that the target is now unescaped.
        let target_path = path_apply_working_directory(&target, &self.working_directory);
        match mode {
            RedirectionMode::Fd => {
                // fish:161-168
                if target == "-" {
                    return Ok(IsFile(false));
                }
                match fish_wcstoi(&target) {
                    Ok(fd) if fd >= 0 => Ok(IsFile(false)),
                    _ => Err(IsErr),
                }
            }
            RedirectionMode::Input | RedirectionMode::TryInput => {
                // fish:170-181 — Input redirections must have a readable non-directory.
                // Note we color "try_input" files as errors if they are invalid,
                // even though it's possible to execute these (replaced via /dev/null).
                if waccess(&target_path, libc::R_OK)
                    && wstat(&target_path).is_ok_and(|md| !md.file_type().is_dir())
                {
                    Ok(IsFile(true))
                } else {
                    Err(IsErr)
                }
            }
            RedirectionMode::Overwrite | RedirectionMode::Append | RedirectionMode::NoClob => {
                // fish:182-187 — Redirections to things that are directories is definitely
                // not allowed.
                if target.ends_with('/') {
                    return Err(IsErr);
                }
                // fish:188-190 — Test whether the file exists, and whether it's writable
                // (possibly after creating it). access() returns failure if the file does
                // not exist.
                let file_exists;
                let file_is_writable;
                match wstat(&target_path) {
                    Ok(md) => {
                        // fish:193-199 — No err. We can write to it if it's not a directory
                        // and we have permission.
                        file_exists = true;
                        file_is_writable =
                            !md.file_type().is_dir() && waccess(&target_path, libc::W_OK);
                    }
                    Err(err) => {
                        if err.raw_os_error() == Some(libc::ENOENT) {
                            // fish:201-203 — File does not exist. Check if its parent
                            // directory is writable.
                            let mut parent = wdirname(&target_path);

                            // fish:205-210 — Ensure that the parent ends with the path
                            // separator. This will ensure that we get an error if the
                            // parent directory is not really a directory.
                            if !parent.ends_with('/') {
                                parent.push('/');
                            }

                            // fish:212-215 — Now the file is considered writable if the
                            // parent directory is writable.
                            file_exists = false;
                            file_is_writable = waccess(&parent, libc::W_OK);
                        } else {
                            // fish:216-221 — Other errors we treat as not writable. This
                            // includes things like ENOTDIR.
                            file_exists = false;
                            file_is_writable = false;
                        }
                    }
                }
                // fish:224-227 — NoClob means that we must not overwrite files that exist.
                if !file_is_writable || (mode == RedirectionMode::NoClob && file_exists) {
                    return Err(IsErr);
                }
                Ok(IsFile(file_exists)) // fish:228
            }
        }
    }
}

/// fish:234-238 — Tests whether the specified string cpath is the prefix of anything we
/// could cd to. directories is a list of possible parent directories (typically either the
/// working directory, or the cdpath). This does I/O!
///
/// zshrs: expects the path *tokenized* (fish expects it unescaped with expansion markers —
/// zsh token chars carry the identical "was this quoted/expandable" information).
pub fn is_potential_path(
    potential_path_fragment: &str,
    at_cursor: bool,
    directories: &[String],
    ctx: &OperationContext,
    flags: PathFlags,
) -> bool {
    // fish:246-250 — fish asserts this runs on its background thread; zshrs computes
    // synchronously in the ZLE thread (path checks are budgeted by the cancel flag), so
    // the assertion is dropped.

    if ctx.check_cancel() {
        // fish:252-254
        return false;
    }

    let require_dir = flags.require_dir; // fish:256
    let mut clean_potential_path_fragment = String::new(); // fish:257
    let mut has_magic = false; // fish:258

    let mut path_with_magic = potential_path_fragment.to_owned(); // fish:260
    if flags.expand_tilde {
        // fish:261-263 — fish expand_tilde; zsh analog is filesub (subst.rs filesubstr),
        // which expands a leading (tokenized) `~`/`~user`/`~+`/`~-` prefix.
        expand_tilde(&mut path_with_magic);
    }

    // fish:265-281 — fish scans for its expansion markers (PROCESS_EXPAND_SELF,
    // VARIABLE_EXPAND[_SINGLE], BRACE_*, ANY_CHAR, ANY_STRING[_RECURSIVE]) and skips
    // INTERNAL_SEPARATOR. zsh spelling: any lexer token char that triggers expansion or
    // globbing is magic; quote-null artifacts (Snull/Dnull/Bnull/Bnullkeep/Nularg) are the
    // INTERNAL_SEPARATOR analog; Tilde/Comma/Dash/Bang untokenize to harmless literals.
    for c in path_with_magic.chars() {
        match c {
            Snull | Dnull | Bnull | Bnullkeep | Nularg => (), // fish:278 INTERNAL_SEPARATOR
            Tilde => clean_potential_path_fragment.push('~'),
            Comma => clean_potential_path_fragment.push(','),
            Dash => clean_potential_path_fragment.push('-'),
            Bang => clean_potential_path_fragment.push('!'),
            c if ('\u{84}'..='\u{a1}').contains(&c) => {
                // fish:267-277 — remaining ITOK range (Pound..Nularg, zsh_h.rs:144-204):
                // globs (Star/Quest/Inbrack/Pound/Hat/Bar/Inang), expansions
                // (String/Qstring/Tick/Qtick/Inpar/Inbrace/Equals) — all magic.
                has_magic = true;
            }
            _ => clean_potential_path_fragment.push(c), // fish:279
        }
    }

    if has_magic || clean_potential_path_fragment.is_empty() {
        // fish:283-285
        return false;
    }

    // fish:287-289 — Don't test the same path multiple times, which can happen if the path
    // is absolute and the CDPATH contains multiple entries.
    let mut checked_paths: HashSet<String> = HashSet::new();

    // fish:291-292 — Keep a cache of which paths / filesystems are case sensitive.
    let mut case_sensitivity_cache = CaseSensitivityCache::new();

    for wd in directories {
        // fish:294
        if ctx.check_cancel() {
            // fish:295-297
            return false;
        }
        let mut abs_path = path_apply_working_directory(&clean_potential_path_fragment, wd); // fish:298
        let must_be_full_dir = abs_path.ends_with('/'); // fish:299
        if flags.for_cd {
            // fish:300-302
            abs_path = normalize_path(&abs_path, /*allow_leading_double_slashes=*/ true);
        }

        // fish:304-307 — Skip this if it's empty or we've already checked it.
        if abs_path.is_empty() || checked_paths.contains(&abs_path) {
            continue;
        }
        checked_paths.insert(abs_path.clone()); // fish:308

        // fish:310-315 — If the user is still typing the argument, we want to highlight it
        // if it's the prefix of a valid path. This means we need to potentially walk all
        // files in some directory. There are two easy cases where we can skip this:
        // 1. If the argument ends with a slash, it must be a valid directory, no prefix.
        // 2. If the cursor is not at the argument, it means the user is definitely not
        //    typing it, so we can skip the prefix-match.
        if must_be_full_dir || !at_cursor {
            // fish:316
            if let Ok(md) = wstat(&abs_path) {
                // fish:317
                if !at_cursor || md.file_type().is_dir() {
                    // fish:318-320
                    return true;
                }
            }
        } else {
            // fish:322-323 — We do not end with a slash; it does not have to be a directory.
            let dir_name = wdirname(&abs_path); // fish:324
            let filename_fragment = wbasename(&abs_path); // fish:325
            if dir_name == "/" && filename_fragment == "/" {
                // fish:326-329 — cd ///.... No autosuggestion.
                return true;
            }

            if let Ok(dir) = std::fs::read_dir(&dir_name) {
                // fish:331
                // fish:332-334 — Check if we're case insensitive.
                let do_case_insensitive =
                    fs_is_case_insensitive(&dir_name, &mut case_sensitivity_cache);

                // fish:336 — We opened the dir_name; look for a string where the base name
                // prefixes it.
                for entry in dir {
                    // fish:337
                    let Ok(entry) = entry else { continue }; // fish:338
                    if ctx.check_cancel() {
                        // fish:339-341
                        return false;
                    }

                    // fish:343-346 — Maybe skip directories.
                    if require_dir && !dir_entry_is_dir(&entry) {
                        continue;
                    }

                    let entry_name = entry.file_name().to_string_lossy().into_owned();
                    if entry_name.starts_with(&filename_fragment)
                        || (do_case_insensitive
                            && string_prefixes_string_case_insensitive(
                                &filename_fragment,
                                &entry_name,
                            ))
                    {
                        // fish:348-356
                        return true;
                    }
                }
            }
        }
    }
    false // fish:361
}

// fish:364-365 — Given a string, return whether it prefixes a path that we could cd into.
// Expects path to be unescaped (zshrs: tokenized, see is_potential_path).
/// fish:366-402 — `is_potential_cd_path`.
pub fn is_potential_cd_path(
    path: &str,
    at_cursor: bool,
    working_directory: &str,
    ctx: &OperationContext,
    mut flags: PathFlags,
) -> bool {
    let mut directories: Vec<String> = vec![]; // fish:374

    if path.starts_with("./") {
        // fish:376-378 — Ignore the CDPATH in this case; just use the working directory.
        directories.push(working_directory.to_owned());
    } else {
        // fish:380-384 — Get the CDPATH. zsh spelling: colon-split $CDPATH (same read the
        // cd builtin does, builtin.rs:2179-2183).
        let cdpath_str = getsparam("CDPATH").unwrap_or_default();
        let mut pathsv: Vec<String> = if cdpath_str.is_empty() {
            vec![".".to_owned()]
        } else {
            cdpath_str.split(':').map(str::to_owned).collect()
        };
        // fish:386-387 — The current $PWD is always valid.
        pathsv.push(".".to_owned());

        for mut next_path in pathsv {
            // fish:389
            if next_path.is_empty() {
                // fish:390-392
                next_path = ".".to_owned();
            }
            // fish:393-394 — Ensure that we use the working directory for relative cdpaths
            // like ".".
            directories.push(path_apply_working_directory(&next_path, working_directory));
        }
    }

    // fish:398-401 — Call is_potential_path with all of these directories.
    flags.require_dir = true;
    flags.for_cd = true;
    is_potential_path(path, at_cursor, &directories, ctx, flags)
}

/// fish:404-410 — Determine if the filesystem containing the given path is case insensitive
/// for lookups regardless of whether it preserves the case when saving a pathname.
///
/// Returns:
///     false: the filesystem is not case insensitive
///     true: the file system is case insensitive
pub type CaseSensitivityCache = HashMap<String, bool>;

/// fish:412-427 — `fs_is_case_insensitive`. Adaptation: fish asks `fpathconf` on the open
/// DirIter fd; `std::fs::read_dir` exposes no fd, so ask `pathconf` on the path instead —
/// identical answer for the same filesystem object.
#[cfg(any(target_os = "macos", target_os = "ios"))]
fn fs_is_case_insensitive(path: &str, case_sensitivity_cache: &mut CaseSensitivityCache) -> bool {
    if let Some(cached) = case_sensitivity_cache.get(path) {
        // fish:418-420
        return *cached;
    }
    // fish:421-423 — Ask the system. A -1 value means error (so assume case sensitive), a 1
    // value means case sensitive, and a 0 value means case insensitive.
    let Ok(cpath) = CString::new(path) else {
        return false;
    };
    let ret = unsafe { libc::pathconf(cpath.as_ptr(), libc::_PC_CASE_SENSITIVE) };
    let icase = ret == 0; // fish:424
    case_sensitivity_cache.insert(path.to_owned(), icase); // fish:425
    icase // fish:426
}

/// fish:428-437 — Other platforms don't have _PC_CASE_SENSITIVE.
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
fn fs_is_case_insensitive(
    _path: &str,
    _case_sensitivity_cache: &mut CaseSensitivityCache,
) -> bool {
    false
}

// ---------------------------------------------------------------------------
// Ported fish helpers this file depends on (each from its own fish home).
// ---------------------------------------------------------------------------

/// fish:wutil/mod.rs:170-211 — `normalize_path`: lexical path normalization (collapses
/// `.`/`..`/repeated slashes) as "cd" does, without touching the filesystem.
pub fn normalize_path(path: &str, allow_leading_double_slashes: bool) -> String {
    // fish:wutil/mod.rs:171-179 — Count the leading slashes.
    let sep = '/';
    let mut leading_slashes: usize = 0;
    for c in path.chars() {
        if c != sep {
            break;
        }
        leading_slashes += 1;
    }

    let comps: Vec<&str> = path.split(sep).collect(); // fish:wutil/mod.rs:181
    let mut new_comps: Vec<&str> = Vec::new(); // fish:wutil/mod.rs:182
    for comp in comps {
        // fish:wutil/mod.rs:183
        if comp.is_empty() || comp == "." {
            // fish:wutil/mod.rs:184-185
            continue;
        } else if comp != ".." {
            // fish:wutil/mod.rs:186-187
            new_comps.push(comp);
        } else if !new_comps.is_empty() && *new_comps.last().unwrap() != ".." {
            // fish:wutil/mod.rs:188-190 — '..' with a real path component, drop that path
            // component.
            new_comps.pop();
        } else if leading_slashes == 0 {
            // fish:wutil/mod.rs:191-193 — We underflowed the .. and are a relative (not
            // absolute) path.
            new_comps.push("..");
        }
    }
    let mut result = new_comps.join(&sep.to_string()); // fish:wutil/mod.rs:196
                                                       // fish:wutil/mod.rs:197-198 — If we don't allow leading double slashes, collapse them
                                                       // to 1 if there are any.
    let mut numslashes = if leading_slashes > 0 { 1 } else { 0 };
    // fish:wutil/mod.rs:199-203 — If we do, prepend one or two leading slashes.
    // Yes, three+ slashes are collapsed to one. (!)
    if allow_leading_double_slashes && leading_slashes == 2 {
        numslashes = 2;
    }
    for _ in 0..numslashes {
        result.insert(0, sep); // fish:wutil/mod.rs:204-206
    }
    // fish:wutil/mod.rs:207-210 — Ensure ./ normalizes to . and not empty.
    if result.is_empty() {
        result.push('.');
    }
    result
}

/// fish:path.rs:505-531 — `path_apply_working_directory`: resolve `path` against
/// `working_directory` unless it is empty or absolute.
pub fn path_apply_working_directory(path: &str, working_directory: &str) -> String {
    if path.is_empty() || working_directory.is_empty() {
        // fish:path.rs:506-508
        return path.to_owned();
    }

    // fish:path.rs:510-512 — We're going to make sure that if we want to prepend the wd,
    // that the string has no leading "/". (fish also exempts HOME_DIRECTORY; the zsh tilde
    // is expanded before this point, and an unexpanded literal '~' resolves relative.)
    let prepend_wd = !path.starts_with('/');

    if !prepend_wd {
        // fish:path.rs:514-517 — No need to prepend the wd, so just return the path we were
        // given.
        return path.to_owned();
    }

    // fish:path.rs:519-523 — Remove up to one "./".
    let mut path_component = path.to_owned();
    if path_component.starts_with("./") {
        path_component.replace_range(0..2, "");
    }

    // fish:path.rs:525-528 — Removing leading /s.
    while path_component.starts_with('/') {
        path_component.replace_range(0..1, "");
    }

    // fish:path.rs:530-531 — Construct and return a new path.
    let mut result = working_directory.to_owned();
    if !result.ends_with('/') {
        result.push('/');
    }
    result.push_str(&path_component);
    result
}

/// fish:wutil `fish_wcstoi` — strtol-style base-10 parse: optional leading whitespace,
/// optional single sign, one or more digits, NO trailing garbage, overflow is an error.
/// ("We are base 10, despite the leading 0" — fish:692.)
pub fn fish_wcstoi(s: &str) -> Result<i32, ()> {
    let t = s.trim_start_matches([' ', '\t']);
    let (neg, digits) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(());
    }
    let mut acc: i64 = 0;
    for b in digits.bytes() {
        acc = acc.checked_mul(10).ok_or(())?;
        acc = acc.checked_add((b - b'0') as i64).ok_or(())?;
        if acc > i32::MAX as i64 + 1 {
            return Err(());
        }
    }
    let signed = if neg { -acc } else { acc };
    i32::try_from(signed).map_err(|_| ())
}

// ---------------------------------------------------------------------------
// zsh-substrate adapters (each replaces a fish API named in the header).
// ---------------------------------------------------------------------------

/// fish:file_tester.rs:116/152 — `expand_one(&mut s, ExpandFlags::FAIL_ON_CMDSUBST, ctx, None)`.
/// zsh analog: `singsub` (subst.rs:1460, zsh Src/subst.c:514 — single-word prefork), after
/// refusing any token that would *execute code* during highlighting: Tick/Qtick (`` ` ``),
/// Inpar (`$(…)`/`(…)`) and Inparmath (`$((…))`, which can run `x++` side effects).
/// errflag is saved/restored so a failed expansion never poisons the editor session.
pub fn expand_one_no_cmdsubst(param: &mut String) -> bool {
    if param
        .chars()
        .any(|c| c == Tick || c == Qtick || c == Inpar || c == Inparmath)
    {
        return false; // FAIL_ON_CMDSUBST
    }
    let saved_errflag = errflag.load(Ordering::Relaxed);
    let expanded = singsub(param);
    let failed = errflag.load(Ordering::Relaxed) != saved_errflag;
    errflag.store(saved_errflag, Ordering::Relaxed);
    if failed {
        return false;
    }
    // singsub output can still carry token/quote-null chars; produce the plain string the
    // fish call sites expect (fish's expand_one returns an unescaped string).
    *param = untokenize(&expanded);
    true
}

/// fish:expand.rs `expand_tilde` — zsh analog `filesubstr` (subst.rs:1921, zsh Src/subst.c
/// filesub): expands a leading tokenized `~`-prefix. A plain leading '~' (already-clean
/// caller input, e.g. tests) is promoted to the Tilde token first, matching what the lexer
/// would have produced for an unquoted tilde.
fn expand_tilde(input: &mut String) {
    let mut s = input.clone();
    if let Some(rest) = s.strip_prefix('~') {
        s = format!("{Tilde}{rest}");
    }
    if s.starts_with(Tilde) {
        if let Some(expanded) = filesubstr(&s, false) {
            *input = expanded;
        }
    }
}

/// fish:wutil `waccess` — access(2) on a path. Returns true when the mode is permitted.
fn waccess(path: &str, mode: libc::c_int) -> bool {
    let Ok(cpath) = CString::new(path) else {
        return false;
    };
    unsafe { libc::access(cpath.as_ptr(), mode) == 0 }
}

/// fish:wutil `wstat` — stat(2), following symlinks.
fn wstat(path: &str) -> std::io::Result<std::fs::Metadata> {
    std::fs::metadata(path)
}

/// fish:wutil `wdirname` — POSIX dirname(3) semantics ("///" → "/", "a" → ".").
fn wdirname(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return "/".to_owned(); // all slashes (or empty → fish returns ".")
    }
    match trimmed.rfind('/') {
        None => ".".to_owned(),
        Some(0) => "/".to_owned(),
        Some(idx) => trimmed[..idx].trim_end_matches('/').to_owned(),
    }
}

/// fish:wutil `wbasename` — POSIX basename(3) semantics ("///" → "/", "a/b/" → "b").
fn wbasename(path: &str) -> String {
    if path.is_empty() {
        return ".".to_owned();
    }
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return "/".to_owned(); // all slashes
    }
    match trimmed.rfind('/') {
        None => trimmed.to_owned(),
        Some(idx) => trimmed[idx + 1..].to_owned(),
    }
}

/// fish:wutil/dir_iter.rs:78-80 — `DirEntry::is_dir` resolves the entry type (via d_type
/// when available, stat otherwise). std spelling: file_type + follow symlinks via metadata.
fn dir_entry_is_dir(entry: &std::fs::DirEntry) -> bool {
    match entry.file_type() {
        Ok(ft) if ft.is_dir() => true,
        Ok(ft) if ft.is_symlink() => std::fs::metadata(entry.path())
            .map(|md| md.is_dir())
            .unwrap_or(false),
        _ => false,
    }
}

/// fish:wcstringutil `string_prefixes_string_case_insensitive` — is `proposed_prefix` a
/// case-insensitive prefix of `value`?
fn string_prefixes_string_case_insensitive(proposed_prefix: &str, value: &str) -> bool {
    let mut vc = value.chars();
    for pc in proposed_prefix.chars() {
        match vc.next() {
            Some(c) if c.to_lowercase().eq(pc.to_lowercase()) => (),
            _ => return false,
        }
    }
    true
}

// ---------------------------------------------------------------------------
// fish:439-889 — tests, ported. Filesystem fixtures use tempfile (dev-dep);
// permission-denial assertions are skipped for euid 0 (root ignores mode bits,
// e.g. CI containers).
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::{
        is_potential_path, FileTester, IsErr, IsFile, OperationContext, PathFlags,
        RedirectionMode,
    };
    use std::fs::{self, create_dir_all, File, Permissions};
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::PathBuf;

    fn is_root() -> bool {
        unsafe { libc::geteuid() == 0 }
    }

    // fish:456-476 — `TempDirWithCtx`.
    struct TempDirWithCtx {
        tempdir: tempfile::TempDir,
        ctx: OperationContext,
    }

    impl TempDirWithCtx {
        fn new() -> TempDirWithCtx {
            TempDirWithCtx {
                tempdir: tempfile::tempdir().unwrap(),
                ctx: OperationContext::empty(),
            }
        }

        fn filepath(&self, name: &str) -> PathBuf {
            self.tempdir.path().join(name)
        }

        fn file_tester(&self) -> FileTester<'_> {
            FileTester::new(
                self.tempdir.path().to_string_lossy().into_owned(),
                &self.ctx,
            )
        }
    }

    // fish:478-525 — `test_ispath`.
    #[test]
    fn test_ispath() {
        let temp = TempDirWithCtx::new();
        let tester = temp.file_tester();

        let file_path = temp.filepath("file.txt");
        File::create(file_path).unwrap();

        assert!(tester.test_path("file.txt", false));
        assert!(tester.test_path("file.txt", true));
        assert!(!tester.test_path("fi", false));
        assert!(tester.test_path("fi", true));
        assert!(!tester.test_path("file.txt-more", false));
        assert!(!tester.test_path("file.txt-more", true));
        assert!(!tester.test_path("ffiledfk.txt", false));
        assert!(!tester.test_path("ffiledfk.txt", true));

        // fish:510 — Directories are also files.
        let dir_path = temp.filepath("somedir");
        create_dir_all(dir_path).unwrap();

        assert!(tester.test_path("somedir", false));
        assert!(tester.test_path("somedir", true));
        assert!(!tester.test_path("some", false));
        assert!(tester.test_path("some", true));
    }

    // fish:527-555 — `test_iscdpath`.
    #[test]
    fn test_iscdpath() {
        let temp = TempDirWithCtx::new();
        let tester = temp.file_tester();

        // fish:531-533 — Note cd (unlike file paths) should report IsErr for invalid cd
        // paths, rather than IsFile(false).
        let dir_path = temp.filepath("somedir");
        create_dir_all(dir_path).unwrap();

        assert_eq!(tester.test_cd_path("somedir", false), Ok(IsFile(true)));
        assert_eq!(tester.test_cd_path("somedir", true), Ok(IsFile(true)));
        assert_eq!(tester.test_cd_path("some", false), Err(IsErr));
        assert_eq!(tester.test_cd_path("some", true), Ok(IsFile(true)));
        assert_eq!(tester.test_cd_path("notdir", false), Err(IsErr));
        assert_eq!(tester.test_cd_path("notdir", true), Err(IsErr));
    }

    // fish:557-743 — `test_redirections`.
    #[test]
    fn test_redirections() {
        // fish:559 — Note we use is_ok and is_err since we don't care about the IsFile part.
        let temp = TempDirWithCtx::new();
        let tester = temp.file_tester();
        let file_path = temp.filepath("file.txt");
        File::create(&file_path).unwrap();

        let dir_path = temp.filepath("somedir");
        create_dir_all(&dir_path).unwrap();

        // fish:568-570 — Normal redirection.
        assert_eq!(
            tester.test_redirection_target("file.txt", RedirectionMode::Input),
            Ok(IsFile(true))
        );

        // fish:572-577 — Can't redirect from a missing file.
        assert_eq!(
            tester.test_redirection_target("notfile.txt", RedirectionMode::Input),
            Err(IsErr)
        );
        assert_eq!(
            tester.test_redirection_target("bogus_path/file.txt", RedirectionMode::Input),
            Err(IsErr)
        );

        // fish:579-581 — Can't redirect from a directory.
        assert_eq!(
            tester.test_redirection_target("somedir", RedirectionMode::Input),
            Err(IsErr)
        );

        // fish:583-590 — Can't redirect from an unreadable file.
        if !is_root() {
            fs::set_permissions(&file_path, Permissions::from_mode(0o200)).unwrap();
            assert_eq!(
                tester.test_redirection_target("file.txt", RedirectionMode::Input),
                Err(IsErr)
            );
            fs::set_permissions(&file_path, Permissions::from_mode(0o600)).unwrap();
        }

        // fish:592-599 — try_input syntax highlighting reports an error even though the
        // command will succeed.
        assert_eq!(
            tester.test_redirection_target("file.txt", RedirectionMode::TryInput),
            Ok(IsFile(true))
        );
        assert_eq!(
            tester.test_redirection_target("notfile.txt", RedirectionMode::TryInput),
            Err(IsErr)
        );
        assert_eq!(
            tester.test_redirection_target("bogus_path/file.txt", RedirectionMode::TryInput),
            Err(IsErr)
        );

        // fish:601-604 — Overwrite an existing file.
        assert_eq!(
            tester.test_redirection_target("file.txt", RedirectionMode::Overwrite),
            Ok(IsFile(true))
        );

        // fish:606-608 — Append to an existing file.
        assert_eq!(
            tester.test_redirection_target("file.txt", RedirectionMode::Append),
            Ok(IsFile(true))
        );

        // fish:610-612 — Write to a missing file.
        assert_eq!(
            tester.test_redirection_target("newfile.txt", RedirectionMode::Overwrite),
            Ok(IsFile(false))
        );

        // fish:614-616 — No-clobber write to existing file should fail.
        assert_eq!(
            tester.test_redirection_target("file.txt", RedirectionMode::NoClob),
            Err(IsErr)
        );

        // fish:618-620 — No-clobber write to missing file should succeed.
        assert_eq!(
            tester.test_redirection_target("unique.txt", RedirectionMode::NoClob),
            Ok(IsFile(false))
        );

        let write_modes = &[
            RedirectionMode::Overwrite,
            RedirectionMode::Append,
            RedirectionMode::NoClob,
        ];

        // fish:628-636 — Can't write to a directory.
        for mode in write_modes {
            assert_eq!(
                tester.test_redirection_target("somedir", *mode),
                Err(IsErr),
                "Should not be able to write to a directory with mode {:?}",
                mode
            );
        }

        if !is_root() {
            // fish:638-648 — Can't write without write permissions.
            fs::set_permissions(&file_path, Permissions::from_mode(0o400)).unwrap(); // Read-only.
            for mode in write_modes {
                assert_eq!(
                    tester.test_redirection_target("file.txt", *mode),
                    Err(IsErr),
                    "Should not be able to write to a read-only file with mode {:?}",
                    mode
                );
            }
            fs::set_permissions(&file_path, Permissions::from_mode(0o600)).unwrap(); // Restore permissions.

            // fish:650-664 — Writing into a directory without write permissions (loop
            // through all modes).
            fs::set_permissions(&dir_path, Permissions::from_mode(0o500)).unwrap(); // Read and execute, no write.
            for mode in write_modes {
                assert_eq!(
                    tester.test_redirection_target("somedir/newfile.txt", *mode),
                    Err(IsErr),
                    "Should not be able to create/write in a read-only directory with mode {:?}",
                    mode
                );
            }
            fs::set_permissions(&dir_path, Permissions::from_mode(0o700)).unwrap();
            // Restore permissions.
        }

        // fish:666-704 — Test fd redirections.
        for good in ["-", "0", "1", "2", "3", "500", "000", "01", "07"] {
            assert_eq!(
                tester.test_redirection_target(good, RedirectionMode::Fd),
                Ok(IsFile(false)),
                "fd target {:?} should be valid",
                good
            );
        }

        // fish:706-742 — Invalid fd redirections.
        for bad in [
            "0x2",
            "0x3F",
            "0F",
            "-1",
            "-0009",
            "--",
            "derp",
            "123boo",
            "18446744073709551616",
        ] {
            assert_eq!(
                tester.test_redirection_target(bad, RedirectionMode::Fd),
                Err(IsErr),
                "fd target {:?} should be invalid",
                bad
            );
        }
    }

    // fish:745-888 — `test_is_potential_path`. Adapted to tempdir-rooted absolute wds
    // (fish uses a cwd-relative "test/" fixture tree).
    #[test]
    fn test_is_potential_path() {
        let temp = TempDirWithCtx::new();
        let root = temp.tempdir.path();

        // fish:749-751 — Directories.
        create_dir_all(root.join("is_potential_path_test/alpha")).unwrap();
        create_dir_all(root.join("is_potential_path_test/beta")).unwrap();

        // fish:753-755 — Files.
        fs::write(root.join("is_potential_path_test/aardvark"), []).unwrap();
        fs::write(root.join("is_potential_path_test/gamma"), []).unwrap();

        let wd = root
            .join("is_potential_path_test")
            .to_string_lossy()
            .into_owned()
            + "/";
        let root_str = root.to_string_lossy().into_owned();
        let wds = [root_str.clone(), wd];

        let ctx = OperationContext::empty();

        let path_require_dir = PathFlags {
            require_dir: true,
            ..Default::default()
        };

        assert!(is_potential_path("al", true, &wds[1..], &ctx, path_require_dir));
        assert!(is_potential_path("alpha/", true, &wds[1..], &ctx, path_require_dir));
        assert!(is_potential_path("aard", true, &wds[1..], &ctx, PathFlags::default()));
        assert!(!is_potential_path("aard", false, &wds[1..], &ctx, PathFlags::default()));
        assert!(!is_potential_path(
            "alp/",
            true,
            &wds[1..],
            &ctx,
            PathFlags {
                require_dir: true,
                for_cd: true,
                ..Default::default()
            }
        ));

        assert!(!is_potential_path("balpha/", true, &wds[1..], &ctx, path_require_dir));
        assert!(!is_potential_path("aard", true, &wds[1..], &ctx, path_require_dir));
        assert!(!is_potential_path("aarde", true, &wds[1..], &ctx, path_require_dir));
        assert!(!is_potential_path("aarde", true, &wds[1..], &ctx, PathFlags::default()));

        assert!(is_potential_path(
            "is_potential_path_test/aardvark",
            true,
            &wds[..1],
            &ctx,
            PathFlags::default()
        ));
        assert!(is_potential_path(
            "is_potential_path_test/al",
            true,
            &wds[..1],
            &ctx,
            path_require_dir
        ));
        assert!(is_potential_path(
            "is_potential_path_test/aardv",
            true,
            &wds[..1],
            &ctx,
            PathFlags::default()
        ));

        assert!(!is_potential_path(
            "is_potential_path_test/aardvark",
            true,
            &wds[..1],
            &ctx,
            path_require_dir
        ));
        assert!(!is_potential_path(
            "is_potential_path_test/al/",
            true,
            &wds[..1],
            &ctx,
            PathFlags::default()
        ));
        assert!(!is_potential_path(
            "is_potential_path_test/ar",
            true,
            &wds[..1],
            &ctx,
            PathFlags::default()
        ));
        assert!(is_potential_path("/usr", true, &wds[..1], &ctx, path_require_dir));
    }
}
