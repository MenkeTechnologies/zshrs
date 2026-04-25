//! Interactive menu completion demo
//!
//! Run with: cargo run --bin menu-demo
//!
//! Completions are pulled from SQLite cache built by compinit

use compsys::{
    build_cache_from_fpath, cache::CompsysCache, compinit_lazy, get_system_fpath, zpwr_list_colors,
    Completion, CompletionGroup, MenuKeymap, MenuResult, MenuState,
};
use std::collections::HashSet;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::OnceLock;

// Marker that caches have been populated in this session
static CACHES_INITIALIZED: OnceLock<bool> = OnceLock::new();

// In-memory cache of executable names for O(1) lookups
static EXECUTABLES_SET: OnceLock<HashSet<String>> = OnceLock::new();

/// Get cache path
fn cache_path() -> PathBuf {
    std::env::var("HOME")
        .map(|h| PathBuf::from(h).join(".cache/zshrs/compsys.db"))
        .unwrap_or_else(|_| PathBuf::from("/tmp/compsys.db"))
}

/// Open or create the completion cache, ensuring all caches are populated
fn open_cache() -> CompsysCache {
    let path = cache_path();

    // Create cache directory if needed
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let mut cache = CompsysCache::open(&path).expect("Failed to open completion cache");

    // Check if completion mappings need to be built
    let (valid, count) = compinit_lazy(&cache);
    if !valid || count == 0 {
        eprintln!("Building completion cache from fpath...");
        let fpath = get_system_fpath();
        let result = build_cache_from_fpath(&fpath, &mut cache).expect("Failed to build cache");
        eprintln!(
            "Cached {} completions in {}ms",
            result.comps.len(),
            result.scan_time_ms
        );
    }

    // Ensure auxiliary caches are populated (executables, named_dirs, shell_functions)
    let _ = CACHES_INITIALIZED.get_or_init(|| {
        populate_auxiliary_caches(&mut cache);
        true
    });

    cache
}

/// Populate auxiliary caches (executables, named_dirs, shell_functions, zstyles) if empty
fn populate_auxiliary_caches(cache: &mut CompsysCache) {
    // PATH executables
    if !cache.has_executables().unwrap_or(true) {
        let executables = scan_path_executables();
        if let Err(e) = cache.set_executables_bulk(&executables) {
            eprintln!("Failed to cache executables: {}", e);
        }
    }

    // Named directories (hash -d)
    if !cache.has_named_dirs().unwrap_or(true) {
        let dirs = scan_named_directories();
        if let Err(e) = cache.set_named_dirs_bulk(&dirs) {
            eprintln!("Failed to cache named dirs: {}", e);
        }
    }

    // Shell functions (FPATH)
    if !cache.has_shell_functions().unwrap_or(true) {
        let funcs = scan_shell_functions();
        if let Err(e) = cache.set_shell_functions_bulk(&funcs) {
            eprintln!("Failed to cache shell functions: {}", e);
        }
    }

    // Zstyles from config files
    if !cache.has_zstyles().unwrap_or(true) {
        let styles = compsys::parse_zstyles_from_config();
        let bulk: Vec<(String, String, Vec<String>, bool)> = styles
            .into_iter()
            .map(|s| (s.pattern, s.style, s.values, s.eval))
            .collect();
        if let Err(e) = cache.set_zstyles_bulk(&bulk) {
            eprintln!("Failed to cache zstyles: {}", e);
        }
    }
}

/// Scan PATH for all executables (expensive, done once)
fn scan_path_executables() -> Vec<(String, String)> {
    let mut executables = Vec::new();

    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    if let Some(name) = entry.file_name().to_str() {
                        if let Ok(meta) = entry.metadata() {
                            if meta.is_file() {
                                #[cfg(unix)]
                                {
                                    use std::os::unix::fs::PermissionsExt;
                                    if meta.permissions().mode() & 0o111 != 0 {
                                        executables.push((name.to_string(), dir.to_string()));
                                    }
                                }
                                #[cfg(not(unix))]
                                {
                                    executables.push((name.to_string(), dir.to_string()));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    executables
}

/// Scan named directories from hash -d and ZPWR env vars (expensive, done once)
fn scan_named_directories() -> Vec<(String, String)> {
    let mut dirs = Vec::new();

    // Try running zsh to get hash -d output
    if let Ok(output) = std::process::Command::new("zsh")
        .args(["-c", "hash -d"])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if let Some((name, path)) = line.split_once('=') {
                    dirs.push((name.to_string(), path.to_string()));
                }
            }
        }
    }

    // Also check ZPWR env vars that define named dirs
    for (key, val) in std::env::vars() {
        if key.starts_with("ZPWR") && std::path::Path::new(&val).is_dir() {
            if !dirs.iter().any(|(n, _)| n == &key) {
                dirs.push((key, val));
            }
        }
    }

    dirs
}

/// Scan FPATH for all shell functions (expensive, done once)
fn scan_shell_functions() -> Vec<(String, String)> {
    let mut funcs = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Scan FPATH directories
    if let Ok(fpath) = std::env::var("FPATH") {
        for dir in fpath.split(':') {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    if let Some(name) = entry.file_name().to_str() {
                        if !name.starts_with('.')
                            && !name.ends_with(".zwc")
                            && seen.insert(name.to_string())
                        {
                            funcs.push((name.to_string(), format!("{}/{}", dir, name)));
                        }
                    }
                }
            }
        }
    }

    // Also check ~/.zpwr/autoload subdirectories
    if let Ok(home) = std::env::var("HOME") {
        for subdir in [
            "/.zpwr/autoload",
            "/.zpwr/autoload/common",
            "/.zpwr/autoload/darwin",
            "/.zpwr/autoload/fzf",
            "/.zpwr/autoload/comps",
            "/.zpwr/autoload/comp_utils",
            "/.zsh/functions",
            "/.config/zsh/functions",
        ] {
            let dir_path = format!("{}{}", home, subdir);
            if let Ok(entries) = std::fs::read_dir(&dir_path) {
                for entry in entries.flatten() {
                    if let Some(name) = entry.file_name().to_str() {
                        if !name.starts_with('.')
                            && !name.ends_with(".zwc")
                            && seen.insert(name.to_string())
                        {
                            funcs.push((name.to_string(), format!("{}/{}", dir_path, name)));
                        }
                    }
                }
            }
        }
    }

    funcs
}

// Global cache connection (singleton, opened once per process)
static GLOBAL_CACHE: OnceLock<std::sync::Mutex<CompsysCache>> = OnceLock::new();

/// Get or initialize the global cache (opened once per process)
fn get_cache() -> std::sync::MutexGuard<'static, CompsysCache> {
    GLOBAL_CACHE
        .get_or_init(|| std::sync::Mutex::new(open_cache()))
        .lock()
        .unwrap()
}

/// Get cache without mutex (for read-only operations in single-threaded code)
fn with_cache<F, R>(f: F) -> R
where
    F: FnOnce(&CompsysCache) -> R,
{
    let cache = GLOBAL_CACHE
        .get_or_init(|| std::sync::Mutex::new(open_cache()))
        .lock()
        .unwrap();
    f(&cache)
}

struct Editor {
    line: String,
    cursor: usize,
}

impl Editor {
    fn new() -> Self {
        Self {
            line: String::new(),
            cursor: 0,
        }
    }

    fn insert(&mut self, c: char) {
        self.line.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    fn insert_str(&mut self, s: &str) {
        self.line.insert_str(self.cursor, s);
        self.cursor += s.len();
    }

    fn backspace(&mut self) {
        if self.cursor > 0 {
            let prev = self.line[..self.cursor]
                .chars()
                .last()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            self.cursor -= prev;
            self.line.remove(self.cursor);
        }
    }

    fn delete_word(&mut self) {
        while self.cursor > 0
            && self.line[..self.cursor]
                .chars()
                .last()
                .map(|c| c.is_whitespace())
                .unwrap_or(false)
        {
            self.backspace();
        }
        while self.cursor > 0
            && !self.line[..self.cursor]
                .chars()
                .last()
                .map(|c| c.is_whitespace())
                .unwrap_or(true)
        {
            self.backspace();
        }
    }

    fn left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= self.line[..self.cursor]
                .chars()
                .last()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
        }
    }

    fn right(&mut self) {
        if self.cursor < self.line.len() {
            self.cursor += self.line[self.cursor..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
        }
    }

    fn delete(&mut self) {
        if self.cursor < self.line.len() {
            let next = self.line[self.cursor..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
            if next > 0 {
                self.line.remove(self.cursor);
            }
        }
    }

    fn home(&mut self) {
        self.cursor = 0;
    }
    fn end(&mut self) {
        self.cursor = self.line.len();
    }
    fn clear(&mut self) {
        self.line.clear();
        self.cursor = 0;
    }

    fn words(&self) -> Vec<&str> {
        self.line.split_whitespace().collect()
    }

    fn current_word(&self) -> &str {
        let before = &self.line[..self.cursor];
        let start = before
            .rfind(char::is_whitespace)
            .map(|i| i + 1)
            .unwrap_or(0);
        &self.line[start..self.cursor]
    }

    fn replace_current_word(&mut self, new: &str) {
        let before = &self.line[..self.cursor];
        let start = before
            .rfind(char::is_whitespace)
            .map(|i| i + 1)
            .unwrap_or(0);
        let end = self.cursor
            + self.line[self.cursor..]
                .find(char::is_whitespace)
                .unwrap_or(self.line.len() - self.cursor);
        self.line.replace_range(start..end, new);
        self.cursor = start + new.len();
    }

    fn display(&self) -> String {
        format!(
            "{}\x1b[7m \x1b[0m{}",
            &self.line[..self.cursor],
            &self.line[self.cursor..]
        )
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Handle --rebuild-cache flag
    if args.iter().any(|a| a == "--rebuild-cache") {
        eprintln!("Rebuilding all caches...");
        let _ = std::fs::remove_file(cache_path());
        let _ = open_cache(); // Forces full rebuild
        eprintln!("Done!");
        return;
    }

    if args.iter().any(|a| a == "--dump") {
        dump_demo();
        return;
    }

    let term = match Terminal::new() {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "Terminal init failed: {}\nRun with --dump for non-interactive",
                e
            );
            return;
        }
    };

    let (tw, th) = term.size();
    let mut editor = Editor::new();
    let mut menu = MenuState::new();
    menu.set_term_size(tw, th);
    menu.set_available_rows(th.saturating_sub(5));
    menu.set_show_headers(true);

    // Load ZPWR zstyle colors
    let zpwr_colors = zpwr_list_colors();
    menu.set_group_colors(zpwr_colors);

    let keymap = MenuKeymap::new();
    let mut menu_selecting = false; // true = navigating menu, false = typing filters completions
    let mut buf = [0u8; 32];

    // Helper to refresh completions based on current editor state
    let refresh_completions = |menu: &mut MenuState, editor: &Editor| {
        let prefix = editor.current_word();
        menu.set_prefix(prefix);
        let groups = generate_completions(editor);
        menu.set_completions(&groups);
        if menu.count() > 0 {
            menu.start();
        }
    };

    // Initial completions
    refresh_completions(&mut menu, &editor);

    loop {
        term.clear();
        term.goto(0, 0);

        // Prompt
        term.write_str("\x1b[32m❯\x1b[0m ");
        term.write_str(&editor.display());
        term.write_str("\r\n");

        // Always show menu if there are completions
        if menu.count() > 0 {
            term.write_str(&"─".repeat(tw));
            term.write_str("\r\n");

            let r = menu.render();
            for line in &r.lines {
                term.write_str(&line.content);
                term.write_str("\r\n");
            }

            // Scrolling status line (zsh style)
            if let Some(status) = &r.status {
                term.write_str(&format!(
                    "\x1b[1;31m-<<\x1b[0m\x1b[36m{}\x1b[0m\x1b[1;31m>>-\x1b[0m",
                    status
                ));
                term.write_str("\r\n");
            }

            term.write_str(&"─".repeat(tw));
            term.write_str("\r\n");

            let (sel, total, _) = menu.status_info();
            if let Some(c) = menu.selected() {
                let mode = if menu_selecting {
                    "\x1b[32mSELECT\x1b[0m"
                } else {
                    "\x1b[33mFILTER\x1b[0m"
                };
                term.write_str(&format!(
                    "{} \x1b[7m {} \x1b[0m {}/{}",
                    mode, c.str_, sel, total
                ));
                if let Some(d) = &c.desc {
                    term.write_str(&format!(" - {}", d));
                }
            }
        } else {
            term.write_str("\r\n\x1b[90mNo matches\x1b[0m");
        }
        term.write_str("\r\n");
        term.flush();

        let n = term.read(&mut buf);
        if n == 0 {
            continue;
        }
        let input = &buf[..n];

        if input == &[3] {
            break;
        } // Ctrl+C

        // Tab toggles between filter mode and select mode
        if input == &[9] {
            if menu.count() > 0 {
                menu_selecting = !menu_selecting;
            }
            continue;
        }

        // Escape cancels selection mode or clears line
        if input == &[0x1b] && n == 1 {
            if menu_selecting {
                menu_selecting = false;
            } else {
                editor.clear();
                refresh_completions(&mut menu, &editor);
            }
            continue;
        }

        // Enter (^M only - ^J is down-history in menuselect mode)
        if input == &[13] {
            if menu_selecting && menu.count() > 0 {
                // Accept selected completion
                if let Some(c) = menu.selected() {
                    editor.replace_current_word(&c.str_.clone());
                    menu_selecting = false;
                    refresh_completions(&mut menu, &editor);
                }
            } else if !editor.line.is_empty() {
                // Execute command
                term.restore();
                println!("\n\x1b[90m$\x1b[0m {}", editor.line);

                let words = editor.words();
                if !words.is_empty() {
                    match words[0] {
                        "exit" | "quit" => return,
                        "cd" => {
                            if let Some(dir) = words.get(1) {
                                let _ = std::env::set_current_dir(dir);
                            }
                        }
                        _ => {
                            let status = std::process::Command::new(words[0])
                                .args(&words[1..])
                                .status();
                            if let Err(e) = status {
                                println!("\x1b[31mError: {}\x1b[0m", e);
                            }
                        }
                    }
                }

                editor.clear();
                term.init_raw();
                menu_selecting = false;
                refresh_completions(&mut menu, &editor);
            }
            continue;
        }

        // Zsh menuselect bindings (from bindkey -M menuselect)
        // These match your actual zsh config exactly
        if menu.count() > 0 {
            let nav_key = match input {
                // ^@ (0x00) = accept-line - accept and exit menu
                &[0x00] => Some(compsys::MenuAction::Accept),
                // ^D (0x04) = accept-and-menu-complete - accept + run NEW completion
                &[0x04] => Some(compsys::MenuAction::AcceptAndMenuComplete),
                // ^F (0x06) = accept-and-infer-next-history - accept + run NEW completion
                &[0x06] => Some(compsys::MenuAction::AcceptAndInferNextHistory),
                // ^H (0x08) = vi-backward-char - move left
                &[0x08] => Some(compsys::MenuAction::Left),
                // ^I (0x09) = vi-forward-char (Tab) - move right
                &[0x09] => Some(compsys::MenuAction::Right),
                // ^J (0x0a) = down-history - move down
                &[0x0a] => Some(compsys::MenuAction::Down),
                // ^K (0x0b) = up-history - move up
                &[0x0b] => Some(compsys::MenuAction::Up),
                // ^L (0x0c) = vi-forward-char - move right
                &[0x0c] => Some(compsys::MenuAction::Right),
                // ^M (0x0d) = .accept-line (Enter) - accept and exit
                &[0x0d] => Some(compsys::MenuAction::Accept),
                // ^N (0x0e) = vi-forward-word - page down by screenful
                &[0x0e] => Some(compsys::MenuAction::PageDown),
                // ^P (0x10) = vi-backward-word - page up by screenful
                &[0x10] => Some(compsys::MenuAction::PageUp),
                // ^S (0x13) = reverse-menu-complete - previous item
                &[0x13] => Some(compsys::MenuAction::Prev),
                // ^V (0x16) = vi-insert - toggle interactive filter mode
                &[0x16] => Some(compsys::MenuAction::ToggleInteractive),
                // ^X (0x18) = history-incremental-search-forward
                &[0x18] => Some(compsys::MenuAction::SearchForward),
                // Ctrl+B (0x02) = backward char (emacs style)
                &[0x02] => Some(compsys::MenuAction::Left),
                _ => None,
            };
            if let Some(action) = nav_key {
                menu_selecting = true;
                match menu.process_action(action) {
                    MenuResult::Accept(s) => {
                        editor.replace_current_word(&s);
                        menu_selecting = false;
                        refresh_completions(&mut menu, &editor);
                    }
                    MenuResult::AcceptAndHold(s) => {
                        // Accept current + stay in menu with next item selected
                        editor.replace_current_word(&s);
                        // Menu already advanced to next item, just continue
                    }
                    MenuResult::Cancel => {
                        menu_selecting = false;
                    }
                    MenuResult::UndoRequested => {
                        // Undo - for now just cancel
                        menu_selecting = false;
                    }
                    _ => {}
                }
                continue;
            }
        }

        if menu_selecting {
            // In select mode, use keymap for all navigation
            if let Some((action, _)) = keymap.lookup(input) {
                match menu.process_action(action) {
                    MenuResult::Accept(s) => {
                        editor.replace_current_word(&s);
                        menu_selecting = false;
                        refresh_completions(&mut menu, &editor);
                    }
                    MenuResult::AcceptAndHold(s) => {
                        // Accept current + stay in menu with next item selected
                        editor.replace_current_word(&s);
                        // Menu already advanced to next item, just continue
                    }
                    MenuResult::Cancel => {
                        menu_selecting = false;
                    }
                    MenuResult::UndoRequested => {
                        menu_selecting = false;
                    }
                    _ => {}
                }
            }
        } else {
            // In filter mode, typing updates the line and re-filters completions
            let mut need_refresh = false;

            if input == &[127] || input == &[8] {
                editor.backspace();
                need_refresh = true;
            } else if input == &[4] {
                editor.delete();
                need_refresh = true;
            } else if input == &[23] {
                editor.delete_word();
                need_refresh = true;
            } else if input == &[1] {
                editor.home();
            } else if input == &[5] {
                editor.end();
            } else if input == &[21] {
                editor.clear();
                need_refresh = true;
            } else if input.starts_with(&[0x1b, b'[']) || input.starts_with(&[0x1b, b'O']) {
                if input == &[0x1b, b'[', b'3', b'~'] {
                    editor.delete();
                    need_refresh = true;
                } else {
                    match input.last() {
                        Some(b'D') => editor.left(),
                        Some(b'C') => editor.right(),
                        Some(b'H') => editor.home(),
                        Some(b'F') => editor.end(),
                        Some(b'A') => {
                            // Up arrow in filter mode enters select mode
                            if menu.count() > 0 {
                                menu_selecting = true;
                            }
                        }
                        Some(b'B') => {
                            // Down arrow in filter mode enters select mode
                            if menu.count() > 0 {
                                menu_selecting = true;
                            }
                        }
                        _ => {}
                    }
                }
            } else if let Ok(s) = std::str::from_utf8(input) {
                for c in s.chars() {
                    if !c.is_control() {
                        editor.insert(c);
                        need_refresh = true;
                    }
                }
            }

            if need_refresh {
                refresh_completions(&mut menu, &editor);
            }
        }
    }

    term.restore();
    println!("\nBye!");
}

fn generate_completions(editor: &Editor) -> Vec<CompletionGroup> {
    let line = &editor.line;
    let prefix = editor.current_word();

    // Parse words before cursor
    let before_cursor = &line[..editor.cursor];
    let words: Vec<&str> = before_cursor.split_whitespace().collect();

    let mut groups = Vec::new();
    let cache = open_cache();

    // Detect context based on cursor position (zshcompsys special contexts)
    let context = detect_completion_context(before_cursor, prefix);

    match context {
        // -brace-parameter- context: ${VAR with flags
        CompContext::BraceParameterFlag => {
            let flag_prefix = prefix.trim_start_matches("${(").trim_start_matches("$(");
            groups.push(complete_parameter_flags(flag_prefix));
        }
        // -brace-parameter- context: ${VAR
        CompContext::BraceParameter => {
            let var_prefix = prefix.trim_start_matches("${");
            groups.push(complete_parameters(var_prefix));
        }
        // -parameter- context: $VAR
        CompContext::Parameter => {
            let var_prefix = prefix.trim_start_matches('$');
            groups.push(complete_parameters(var_prefix));
        }
        // Glob qualifier context: *(qualifiers)
        CompContext::GlobQualifier => {
            let qual_prefix = if let Some(idx) = prefix.rfind('(') {
                &prefix[idx + 1..]
            } else {
                ""
            };
            groups.push(complete_glob_qualifiers(qual_prefix));
        }
        // -tilde- context: ~user or ~NAMED_DIR
        CompContext::Tilde => {
            let user_prefix = prefix.trim_start_matches('~');
            groups.extend(complete_users_and_named_dirs(user_prefix));
        }
        // -equal- context: =cmd (command path expansion)
        CompContext::Equal => {
            let cmd_prefix = prefix.trim_start_matches('=');
            groups.push(complete_commands_from_cache(&cache, cmd_prefix));
        }
        // -redirect- context: after >, <, >>, etc.
        CompContext::Redirect => {
            groups.push(complete_files(prefix, true));
        }
        // -math- context: inside (( ))
        CompContext::Math => {
            groups.push(complete_parameters(prefix));
            groups.push(complete_math_functions(prefix));
        }
        // -condition- context: inside [[ ]]
        CompContext::Condition => {
            groups.push(complete_condition_operators(prefix));
            groups.push(complete_files(prefix, true));
        }
        // -subscript- context: array[subscript]
        CompContext::Subscript => {
            groups.push(complete_subscript_flags(prefix));
        }
        // -value- context: VAR=value (right side of assignment)
        CompContext::Value => {
            groups.push(complete_files(prefix, true));
        }
        // -assign-parameter- context: left side of assignment
        CompContext::AssignParameter => {
            groups.push(complete_parameters(prefix));
        }
        // -array-value- context: array assignment
        CompContext::ArrayValue => {
            groups.push(complete_files(prefix, true));
        }
        // -command- context: command position
        CompContext::Command => {
            groups.push(complete_commands_from_cache(&cache, prefix));
            groups.push(complete_shell_functions(prefix));
            groups.push(complete_builtins(prefix));
            groups.push(complete_files(prefix, true));
            // Like _megacomplete: add last commands from history at command position
            groups.push(complete_last_commands(prefix));
        }
        // -default- context: arguments to commands
        CompContext::Default => {
            let cmd = words.first().copied().unwrap_or("");
            let arg_num = if before_cursor.ends_with(' ') {
                words.len()
            } else {
                words.len().saturating_sub(1)
            };

            // Look up completion function in cache
            if let Ok(Some(func)) = cache.get_comp(cmd) {
                groups.extend(complete_from_cache_function(
                    &cache,
                    cmd,
                    &func,
                    &words,
                    arg_num,
                    prefix,
                    before_cursor,
                ));
            } else {
                // No completion function - just offer files
                if prefix.starts_with('-') {
                    groups.push(complete_generic_options(prefix));
                }
                groups.push(complete_files(prefix, true));
            }
            // Like _megacomplete: add last command args as fallback
            groups.push(complete_last_command_args(prefix));
        }
        // History expansion context: !!
        CompContext::History => {
            groups.push(complete_history_modifiers(prefix));
        }
    }

    groups
        .into_iter()
        .filter(|g| !g.matches.is_empty())
        .collect()
}

/// Completion context types matching zshcompsys special contexts
#[derive(Debug, Clone, Copy, PartialEq)]
enum CompContext {
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
fn detect_completion_context(before_cursor: &str, prefix: &str) -> CompContext {
    let trimmed = before_cursor.trim();

    // Check for parameter flag context ${(
    if prefix.starts_with("${(") {
        return CompContext::BraceParameterFlag;
    }

    // Check for brace parameter ${VAR
    if prefix.starts_with("${") && !prefix.contains('(') {
        return CompContext::BraceParameter;
    }

    // Check for parameter $VAR
    if prefix.starts_with('$') && !prefix.starts_with("${") && !prefix.starts_with("$(") {
        return CompContext::Parameter;
    }

    // Check for glob qualifier *(
    if prefix.contains("*(")
        || prefix.contains("?(")
        || (prefix.ends_with("(") && (before_cursor.contains('*') || before_cursor.contains('?')))
    {
        return CompContext::GlobQualifier;
    }

    // Check for tilde expansion ~user
    if prefix.starts_with('~') && !prefix.contains('/') {
        return CompContext::Tilde;
    }

    // Check for equal expansion =cmd
    if prefix.starts_with('=') && prefix.len() > 1 {
        return CompContext::Equal;
    }

    // Check for history expansion
    if prefix.starts_with('!') || prefix.starts_with("!!") {
        return CompContext::History;
    }

    // Check for redirect context (previous word is >, <, >>, etc.)
    let words: Vec<&str> = before_cursor.split_whitespace().collect();
    if let Some(&last) = words.last() {
        if matches!(
            last,
            ">" | "<" | ">>" | "<<" | ">&" | "<&" | ">|" | "2>" | "2>>" | "&>" | "&>>" | "<>"
        ) {
            return CompContext::Redirect;
        }
    }
    // Also check if before_cursor ends with redirect operator
    for op in &[
        ">", "<", ">>", "<<", ">&", "<&", ">|", "2>", "2>>", "&>", "&>>", "<>",
    ] {
        if trimmed.ends_with(op) {
            return CompContext::Redirect;
        }
    }

    // Check for math context (( ))
    if before_cursor.contains("((") && !before_cursor.contains("))") {
        return CompContext::Math;
    }

    // Check for condition context [[ ]]
    if before_cursor.contains("[[") && !before_cursor.contains("]]") {
        return CompContext::Condition;
    }

    // Check for subscript context array[
    if prefix.contains('[') && !prefix.contains(']') {
        return CompContext::Subscript;
    }

    // Check for assignment context VAR=
    if before_cursor.contains('=') && !before_cursor.contains(' ') {
        // Check if we're on the right side (value) or left side (parameter name)
        if let Some(eq_pos) = before_cursor.rfind('=') {
            let after_eq = &before_cursor[eq_pos + 1..];
            if after_eq.is_empty() || !after_eq.contains(' ') {
                // Check for array assignment
                if after_eq.starts_with('(') {
                    return CompContext::ArrayValue;
                }
                return CompContext::Value;
            }
        }
    }

    // Check if completing the left side of an assignment (no = yet but looks like param name)
    if !trimmed.contains(' ')
        && !trimmed.contains('=')
        && !trimmed.is_empty()
        && trimmed.chars().all(|c| c.is_alphanumeric() || c == '_')
    {
        // Could be either command or assignment - default to command
        return CompContext::Command;
    }

    // Determine if we're completing the command or an argument
    let completing_command =
        words.is_empty() || (words.len() == 1 && !before_cursor.ends_with(' '));

    if completing_command {
        CompContext::Command
    } else {
        CompContext::Default
    }
}

/// Complete users and named directories for tilde expansion
fn complete_users_and_named_dirs(prefix: &str) -> Vec<CompletionGroup> {
    let mut groups = Vec::new();

    // Named directories from SQLite cache (already sorted)
    let cache = get_cache();
    let named_dirs = if prefix.is_empty() {
        cache.get_named_dirs().unwrap_or_default()
    } else {
        cache.get_named_dirs_prefix(prefix).unwrap_or_default()
    };
    drop(cache); // Release lock

    if !named_dirs.is_empty() {
        let mut nd_group = CompletionGroup::new("named directory");
        nd_group.explanation = Some("named directory".to_string());

        for (name, path) in named_dirs {
            let mut c = Completion::new(format!("~{}", name));
            c.desc = Some(path);
            nd_group.matches.push(c);
        }
        // Already sorted by SQLite
        groups.push(nd_group);
    }

    // Users from /etc/passwd (needs Rust-side sort)
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

    // Also add current user
    if let Ok(user) = std::env::var("USER") {
        if user.to_lowercase().starts_with(&prefix.to_lowercase()) {
            let exists = user_group
                .matches
                .iter()
                .any(|c| c.str_ == format!("~{}", user));
            if !exists {
                let mut c = Completion::new(format!("~{}", user));
                if let Ok(home) = std::env::var("HOME") {
                    c.desc = Some(home);
                }
                user_group.matches.push(c);
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

/// Complete math functions for (( )) context
fn complete_math_functions(prefix: &str) -> CompletionGroup {
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
        ("fabs", "floating absolute value"),
        ("floor", "floor"),
        ("int", "integer part"),
        ("log", "natural logarithm"),
        ("log10", "base-10 logarithm"),
        ("rand", "random number"),
        ("sin", "sine"),
        ("sinh", "hyperbolic sine"),
        ("sqrt", "square root"),
        ("tan", "tangent"),
        ("tanh", "hyperbolic tangent"),
    ];

    let prefix_lower = prefix.to_lowercase();
    for (func, desc) in functions {
        if func.starts_with(&prefix_lower) {
            let mut c = Completion::new(func.to_string());
            c.desc = Some(desc.to_string());
            group.matches.push(c);
        }
    }
    group
}

/// Complete condition operators for [[ ]] context
fn complete_condition_operators(prefix: &str) -> CompletionGroup {
    let mut group = CompletionGroup::new("condition operator");
    group.explanation = Some("condition operator".to_string());

    let operators = [
        ("-a", "file exists"),
        ("-b", "block special file"),
        ("-c", "character special file"),
        ("-d", "directory"),
        ("-e", "file exists"),
        ("-f", "regular file"),
        ("-g", "setgid bit set"),
        ("-h", "symbolic link"),
        ("-k", "sticky bit set"),
        ("-n", "string length > 0"),
        ("-o", "option is set"),
        ("-p", "named pipe"),
        ("-r", "readable"),
        ("-s", "file size > 0"),
        ("-t", "fd is a tty"),
        ("-u", "setuid bit set"),
        ("-v", "variable is set"),
        ("-w", "writable"),
        ("-x", "executable"),
        ("-z", "string length == 0"),
        ("-L", "symbolic link"),
        ("-N", "modified since read"),
        ("-O", "owned by EUID"),
        ("-G", "owned by EGID"),
        ("-S", "socket"),
        ("-eq", "equal (numeric)"),
        ("-ne", "not equal (numeric)"),
        ("-lt", "less than (numeric)"),
        ("-le", "less or equal (numeric)"),
        ("-gt", "greater than (numeric)"),
        ("-ge", "greater or equal (numeric)"),
        ("==", "string equal"),
        ("!=", "string not equal"),
        ("=~", "regex match"),
        ("-nt", "newer than"),
        ("-ot", "older than"),
        ("-ef", "same file"),
    ];

    for (op, desc) in operators {
        if op.starts_with(prefix) {
            let mut c = Completion::new(op.to_string());
            c.desc = Some(desc.to_string());
            group.matches.push(c);
        }
    }
    group
}

/// Complete subscript flags for array[subscript] context  
fn complete_subscript_flags(prefix: &str) -> CompletionGroup {
    let mut group = CompletionGroup::new("subscript flag");
    group.explanation = Some("subscript flag".to_string());

    let flags = [
        ("@", "all elements as separate words"),
        ("*", "all elements as single word"),
        ("#", "number of elements"),
        ("(i)", "first index of match"),
        ("(I)", "last index of match"),
        ("(k)", "keys of associative array"),
        ("(v)", "values of associative array"),
        ("(r)", "reverse subscript"),
        ("(R)", "reverse subscript from end"),
        ("(w)", "word subscript"),
        ("(s)", "subscript with separator"),
        ("(e)", "exact match subscript"),
    ];

    let prefix_stripped = prefix.trim_start_matches('[');
    for (flag, desc) in flags {
        if flag.starts_with(prefix_stripped) {
            let mut c = Completion::new(flag.to_string());
            c.desc = Some(desc.to_string());
            group.matches.push(c);
        }
    }
    group
}

/// Complete history modifiers for !! context
fn complete_history_modifiers(prefix: &str) -> CompletionGroup {
    let mut group = CompletionGroup::new("history modifier");
    group.explanation = Some("history modifier".to_string());

    let modifiers = [
        ("!!", "previous command"),
        ("!$", "last argument of previous command"),
        ("!^", "first argument of previous command"),
        ("!*", "all arguments of previous command"),
        ("!n", "command number n"),
        ("!-n", "n commands back"),
        ("!str", "most recent command starting with str"),
        ("!?str", "most recent command containing str"),
        (":h", "head (dirname)"),
        (":t", "tail (basename)"),
        (":r", "root (remove extension)"),
        (":e", "extension"),
        (":p", "print without executing"),
        (":q", "quote"),
        (":x", "quote and split"),
        (":s/old/new", "substitute"),
        (":gs/old/new", "global substitute"),
        (":&", "repeat last substitution"),
        (":a", "absolute path"),
        (":A", "resolve symlinks"),
        (":l", "lowercase"),
        (":u", "uppercase"),
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

/// Complete shell parameters from environment
fn complete_parameters(prefix: &str) -> CompletionGroup {
    let mut group = CompletionGroup::new("parameter");
    group.explanation = Some("parameter".to_string());

    let prefix_upper = prefix.to_uppercase();

    for (key, value) in std::env::vars() {
        if key.to_uppercase().starts_with(&prefix_upper) {
            let mut c = Completion::new(key.clone());
            // Truncate long values for display
            let display_val: String = value.chars().take(60).collect();
            if value.len() > 60 {
                c.desc = Some(format!("{}...", display_val));
            } else {
                c.desc = Some(display_val);
            }
            group.matches.push(c);
        }
    }

    // Sort alphabetically
    group.matches.sort_by(|a, b| a.str_.cmp(&b.str_));
    group
}

/// Generic options for commands without completion functions
fn complete_generic_options(prefix: &str) -> CompletionGroup {
    let mut group = CompletionGroup::new("option");
    group.explanation = Some("common option".to_string());

    let options = ["--help", "--version", "-h", "-v", "-V"];
    let prefix_lower = prefix.to_lowercase();

    for opt in options {
        if opt.to_lowercase().starts_with(&prefix_lower) {
            group.matches.push(Completion::new(opt.to_string()));
        }
    }
    group
}

/// Complete zsh parameter expansion flags ${(flags)...}
fn complete_parameter_flags(prefix: &str) -> CompletionGroup {
    let mut group = CompletionGroup::new("parameter flag");
    group.explanation = Some("parameter flag".to_string());

    let flags: &[(&str, &str)] = &[
        ("-", "sort decimal integers numerically"),
        ("@", "prevent double-quoted joining of arrays"),
        ("*", "enable extended globs for pattern"),
        ("#", "interpret numeric expression as character code"),
        ("%", "expand prompt sequences"),
        ("~", "treat strings in parameter flag arguments as patterns"),
        ("0", "split words on null bytes"),
        ("A", "assign as an array parameter"),
        ("a", "sort in array index order (with O to reverse)"),
        ("b", "backslash quote pattern characters only"),
        (
            "B",
            "include index of beginning of match in #, % expressions",
        ),
        ("C", "capitalize words"),
        ("c", "count characters in an array (with ${(c)#...})"),
        ("D", "perform directory name abbreviation"),
        (
            "E",
            "include index of one past end of match in #, % expressions",
        ),
        ("e", "perform single-word shell expansions"),
        ("F", "join arrays with newlines"),
        ("f", "split the result on newlines"),
        ("g", "process echo array sequences (needs options)"),
        ("I", "search <argument>th match in #, %, / expressions"),
        ("i", "sort case-insensitively"),
        ("j", "join arrays with specified string"),
        ("k", "substitute keys of associative arrays"),
        ("l", "left-pad resulting words"),
        ("L", "lower case all letters"),
        ("m", "count multibyte width in padding calculation"),
        ("M", "include matched portion in #, % expressions"),
        ("N", "include length of match in #, % expressions"),
        ("n", "sort decimal integers numerically"),
        ("O", "sort in descending order"),
        ("o", "sort in ascending order"),
        ("P", "interpret result as parameter name"),
        ("p", "recognize print escape sequences"),
        ("Q", "remove one level of quotes"),
        ("q", "quote special characters"),
        ("r", "right-pad resulting words"),
        ("S", "split on substrings of separator (needs s)"),
        ("s", "split on specified separator"),
        ("t", "substitute type of parameter"),
        ("U", "unique - remove duplicate elements"),
        ("u", "expand only first occurrence in substitution"),
        ("V", "make special characters visible"),
        ("v", "substitute values of associative arrays"),
        ("w", "count words in array or string"),
        ("W", "count words including empty words"),
        ("X", "report parsing errors and eXit"),
        ("z", "split words using shell parsing"),
        ("Z", "split words using full shell parsing"),
    ];

    for (flag, desc) in flags {
        if prefix.is_empty() || flag.starts_with(prefix) {
            let mut c = Completion::new(flag.to_string());
            c.desc = Some(desc.to_string());
            group.matches.push(c);
        }
    }
    group
}

/// Complete zsh glob qualifiers *(qualifier)
fn complete_glob_qualifiers(prefix: &str) -> CompletionGroup {
    let mut group = CompletionGroup::new("glob qualifier");
    group.explanation = Some("glob qualifier".to_string());

    let qualifiers: &[(&str, &str)] = &[
        // File type qualifiers
        (".", "regular files"),
        ("/", "directories"),
        ("@", "symbolic links"),
        ("=", "sockets"),
        ("p", "named pipes (FIFOs)"),
        ("*", "executable plain files"),
        ("%", "device files (character or block special)"),
        ("%b", "block special files"),
        ("%c", "character special files"),
        // Permissions
        ("r", "owner-readable files"),
        ("w", "owner-writable files"),
        ("x", "owner-executable files"),
        ("A", "group-readable files"),
        ("I", "group-writable files"),
        ("E", "group-executable files"),
        ("R", "world-readable files"),
        ("W", "world-writable files"),
        ("X", "world-executable files"),
        ("s", "setuid files"),
        ("S", "setgid files"),
        ("t", "sticky bit set"),
        // Ownership
        ("U", "owned by EUID"),
        ("G", "owned by EGID"),
        ("u", "owned by given user (u:uid or u:name)"),
        ("g", "owned by given group (g:gid or g:name)"),
        // Size
        ("L", "size filter (Lk, Lm, L+n, L-n)"),
        ("Lk", "size in kilobytes"),
        ("Lm", "size in megabytes"),
        ("LM", "size in megabytes (same as Lm)"),
        ("Lg", "size in gigabytes"),
        // Time
        ("m", "modification time (mh, md, mw, mM)"),
        ("mh", "modified within n hours"),
        ("md", "modified within n days (alias: m)"),
        ("mw", "modified within n weeks"),
        ("mM", "modified within n months"),
        ("a", "access time (ah, ad, aw, aM)"),
        ("c", "inode change time (ch, cd, cw, cM)"),
        // Special
        ("N", "NULL_GLOB - set nullglob for this pattern"),
        ("D", "GLOB_DOTS - match files starting with ."),
        ("n", "numeric glob sort"),
        ("o", "sort order (on, oL, om, oa, oc)"),
        ("on", "sort by name"),
        ("oL", "sort by size (length)"),
        ("om", "sort by modification time"),
        ("oa", "sort by access time"),
        ("oc", "sort by inode change time"),
        ("od", "sort by directory depth"),
        ("O", "reverse sort order (same suffixes as o)"),
        ("^", "negate following qualifier"),
        ("-", "follow symlinks"),
        ("M", "mark directories with trailing /"),
        ("T", "mark types with trailing character"),
        ("F", "non-empty directories"),
        ("e", "execute code as test (e:'code':)"),
        ("+", "execute shell function as test (+func)"),
        ("[", "select range by index ([1], [1,3], [-1])"),
        ("f", "permission filter (f:spec:)"),
        ("P", "prepend string to each match"),
        ("Y", "short-circuit limit (Y1 = first match only)"),
    ];

    for (qual, desc) in qualifiers {
        if prefix.is_empty() || qual.starts_with(prefix) {
            let mut c = Completion::new(qual.to_string());
            c.desc = Some(desc.to_string());
            group.matches.push(c);
        }
    }
    group
}

/// Get cached executable names (loaded once per process)
fn get_executables_set(cache: &CompsysCache) -> &'static HashSet<String> {
    EXECUTABLES_SET.get_or_init(|| cache.get_executable_names().unwrap_or_default())
}

/// Complete commands from the SQLite cache using prefix search
fn complete_commands_from_cache(cache: &CompsysCache, prefix: &str) -> CompletionGroup {
    let mut group = CompletionGroup::new("command");
    group.explanation = Some("command".to_string());

    // Get cached executable names (O(1) per-process, loaded once)
    let executables = get_executables_set(cache);

    // Use prefix search on comps
    let comps = cache.comps_prefix(prefix).unwrap_or_default();

    for (cmd, func) in comps {
        // Skip special context entries
        if cmd.starts_with('-')
            || cmd.contains('(')
            || cmd.contains('*')
            || cmd.contains('{')
            || cmd.contains('\'')
            || cmd.contains('$')
        {
            continue;
        }
        // O(1) HashSet lookup
        if executables.contains(&cmd) {
            let mut c = Completion::new(cmd);
            c.desc = Some(func);
            group.matches.push(c);
        }
    }

    // Limit
    if group.matches.len() > 500 {
        group.matches.truncate(500);
    }

    group
}

/// Complete arguments based on cached completion function
/// All completions are parsed from the actual completion files - no hardcoding
fn complete_from_cache_function(
    cache: &CompsysCache,
    _cmd: &str,
    func: &str,
    _words: &[&str],
    arg_num: usize,
    prefix: &str,
    _before_cursor: &str,
) -> Vec<CompletionGroup> {
    let mut groups = Vec::new();

    if prefix.starts_with('-') {
        // Complete options from the completion file
        if let Some(opts_group) = complete_options_from_cache(cache, func, prefix) {
            groups.push(opts_group);
        }
    } else if arg_num == 1 {
        // Complete subcommands from the completion file
        if let Some(subs_group) = complete_subcommands_from_cache(cache, func, prefix) {
            groups.push(subs_group);
        }
    }

    // Files as fallback
    groups.push(complete_files(prefix, true));
    groups
}

/// Parse options from a completion file in the cache
fn complete_options_from_cache(
    cache: &CompsysCache,
    func: &str,
    prefix: &str,
) -> Option<CompletionGroup> {
    // Get the autoload stub to find the file path
    let stub = cache.get_autoload(func).ok()??;

    // Read and parse the completion file
    let content = std::fs::read_to_string(&stub.source).ok()?;

    let mut group = CompletionGroup::new("option");
    group.explanation = Some(format!("option (from {})", func));

    let prefix_lower = prefix.to_lowercase();

    // Parse _arguments style options from the file
    // Patterns:
    //   '--option[description]'
    //   '(-x --exclude)--option[description]'
    //   '--option=[description]:arg:action'
    //   '-o[description]'
    for line in content.lines() {
        let line = line.trim();

        // Skip non-argument lines
        if !line.contains('[') || line.starts_with('#') {
            continue;
        }

        // Extract options from _arguments patterns
        // Look for quoted option specs like '--foo[desc]' or '(-x)--foo[desc]'
        for segment in line.split('\'') {
            if let Some(opt) = parse_option_spec(segment) {
                if opt.0.to_lowercase().starts_with(&prefix_lower) {
                    let mut c = Completion::new(opt.0);
                    if !opt.1.is_empty() {
                        c.desc = Some(opt.1);
                    }
                    group.matches.push(c);
                }
            }
        }
    }

    // Deduplicate and sort
    group.matches.sort_by(|a, b| a.str_.cmp(&b.str_));
    group.matches.dedup_by(|a, b| a.str_ == b.str_);

    if group.matches.is_empty() {
        None
    } else {
        Some(group)
    }
}

/// Parse subcommands from a completion file
/// Conservative parsing - only extract clearly defined subcommands
fn complete_subcommands_from_cache(
    cache: &CompsysCache,
    func: &str,
    prefix: &str,
) -> Option<CompletionGroup> {
    let stub = cache.get_autoload(func).ok()??;
    let content = std::fs::read_to_string(&stub.source).ok()?;

    let mut group = CompletionGroup::new("subcommand");
    group.explanation = Some(format!("subcommand (from {})", func));

    let prefix_lower = prefix.to_lowercase();

    // Parse subcommands from common patterns in completion files
    // Focus on clearly structured definitions to avoid garbage

    for line in content.lines() {
        let line = line.trim();

        // Skip comments and empty lines
        if line.starts_with('#') || line.is_empty() {
            continue;
        }

        // Pattern 1: _describe with inline array '(cmd1:desc1 cmd2:desc2)'
        // Example: _describe 'command' '(build:Build\ project run:Run\ program)'
        if line.contains("_describe") && line.contains("'(") {
            if let Some(start) = line.find("'(") {
                let rest = &line[start + 2..];
                if let Some(end) = rest.find(")'") {
                    let items_str = &rest[..end];
                    for item in items_str.split_whitespace() {
                        let item = item.trim();
                        if item.is_empty() || item.contains('$') || item.contains('{') {
                            continue;
                        }
                        // Parse "cmd:description" or "cmd:desc\ with\ escapes"
                        let (cmd, desc) = if let Some(colon) = item.find(':') {
                            let c = &item[..colon];
                            let d = item[colon + 1..].replace("\\ ", " ");
                            (c.to_string(), d)
                        } else {
                            (item.to_string(), String::new())
                        };

                        // Validate: cmd should be alphanumeric with dashes
                        if !cmd.is_empty()
                            && cmd
                                .chars()
                                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
                            && cmd.to_lowercase().starts_with(&prefix_lower)
                        {
                            let mut c = Completion::new(cmd);
                            if !desc.is_empty() {
                                c.desc = Some(desc);
                            }
                            group.matches.push(c);
                        }
                    }
                }
            }
        }

        // Pattern 2: Array definition with quoted strings
        // Example: local commands=("build:Build project" "run:Run program")
        // or: _values 'command' 'build[Build the project]' 'run[Run the program]'
        if line.contains("_values") {
            // Parse _values style: _values 'tag' 'cmd1[desc]' 'cmd2[desc]'
            for segment in line.split('\'') {
                let segment = segment.trim();
                if segment.is_empty()
                    || segment.starts_with('-')
                    || !segment
                        .chars()
                        .next()
                        .map(|c| c.is_alphanumeric())
                        .unwrap_or(false)
                {
                    continue;
                }

                // Parse cmd[description] format
                if let Some(bracket) = segment.find('[') {
                    let cmd = &segment[..bracket];
                    let desc_end = segment.find(']').unwrap_or(segment.len());
                    let desc = &segment[bracket + 1..desc_end];

                    if !cmd.is_empty()
                        && cmd
                            .chars()
                            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
                        && cmd.to_lowercase().starts_with(&prefix_lower)
                    {
                        let mut c = Completion::new(cmd.to_string());
                        if !desc.is_empty() {
                            c.desc = Some(desc.to_string());
                        }
                        group.matches.push(c);
                    }
                }
            }
        }
    }

    // Deduplicate and sort
    group.matches.sort_by(|a, b| a.str_.cmp(&b.str_));
    group.matches.dedup_by(|a, b| a.str_ == b.str_);

    if group.matches.is_empty() {
        None
    } else {
        Some(group)
    }
}

/// Parse a single option spec like '--foo[description]' or '(-x --exclude)--foo[desc]'
fn parse_option_spec(spec: &str) -> Option<(String, String)> {
    let spec = spec.trim();

    // Skip if doesn't look like an option
    if !spec.contains('-') {
        return None;
    }

    // Find the actual option (skip exclusion groups)
    let opt_start = if spec.starts_with('(') {
        // Skip exclusion group like (-x --exclude)
        spec.find(')')?.checked_add(1)?
    } else {
        0
    };

    let rest = &spec[opt_start..];

    // Must start with - or --
    if !rest.starts_with('-') {
        return None;
    }

    // Find option name end (at '[' or '=' or ':' or space)
    let opt_end = rest
        .find(|c| c == '[' || c == '=' || c == ':' || c == ' ')
        .unwrap_or(rest.len());

    let opt_name = rest[..opt_end].trim_end_matches(|c| c == '+' || c == '=');

    if opt_name.is_empty() || opt_name == "-" || opt_name == "--" {
        return None;
    }

    // Extract description from [...]
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

fn complete_aliases(prefix: &str) -> CompletionGroup {
    let mut group = CompletionGroup::new("alias");
    group.explanation = Some("alias".to_string());

    let prefix_lower = prefix.to_lowercase();

    // Read aliases from common alias files
    let alias_files = [
        std::env::var("HOME").ok().map(|h| format!("{}/.zshrc", h)),
        std::env::var("HOME")
            .ok()
            .map(|h| format!("{}/.zpwr/local/.tokens.sh", h)),
        std::env::var("HOME")
            .ok()
            .map(|h| format!("{}/.bash_aliases", h)),
        std::env::var("HOME")
            .ok()
            .map(|h| format!("{}/.aliases", h)),
    ];

    let mut seen = std::collections::HashSet::new();

    for path_opt in alias_files.iter().flatten() {
        if let Ok(content) = std::fs::read_to_string(path_opt) {
            for line in content.lines() {
                let line = line.trim();
                // Match: alias name=value or alias name='value' or alias name="value"
                if let Some(rest) = line.strip_prefix("alias ") {
                    if let Some(eq) = rest.find('=') {
                        let name = rest[..eq].trim();
                        if name.to_lowercase().starts_with(&prefix_lower)
                            && seen.insert(name.to_string())
                        {
                            let value = rest[eq + 1..].trim().trim_matches('\'').trim_matches('"');
                            let mut c = Completion::new(name.to_string());
                            c.desc = Some(value.chars().take(40).collect());
                            group.matches.push(c);
                        }
                    }
                }
            }
        }
    }

    // Common aliases if we didn't find any
    if group.matches.is_empty() {
        let common = [
            ("ll", "ls -la"),
            ("la", "ls -A"),
            ("l", "ls -CF"),
            ("..", "cd .."),
            ("...", "cd ../.."),
            ("g", "git"),
            ("ga", "git add"),
            ("gc", "git commit"),
            ("gp", "git push"),
            ("gd", "git diff"),
            ("gs", "git status"),
            ("gl", "git log"),
            ("gco", "git checkout"),
        ];
        for (name, value) in common {
            if name.to_lowercase().starts_with(&prefix_lower) {
                let mut c = Completion::new(name.to_string());
                c.desc = Some(value.to_string());
                group.matches.push(c);
            }
        }
    }

    group.matches.sort_by(|a, b| a.str_.cmp(&b.str_));
    group
}

fn complete_shell_functions(prefix: &str) -> CompletionGroup {
    let mut group = CompletionGroup::new("shell function");
    group.explanation = Some("shell function".to_string());

    let prefix_lower = prefix.to_lowercase();
    let cache = get_cache();

    // Get shell functions from SQLite cache
    if let Ok(funcs) = cache.get_shell_functions_prefix(prefix) {
        for (name, source) in funcs {
            let mut c = Completion::new(name);
            c.desc = Some(source);
            group.matches.push(c);
        }
    } else if let Ok(funcs) = cache.get_shell_functions() {
        // Fallback: filter manually
        for (name, source) in funcs {
            if name.to_lowercase().starts_with(&prefix_lower) {
                let mut c = Completion::new(name);
                c.desc = Some(source);
                group.matches.push(c);
            }
        }
    }

    // SQLite returns sorted, no Rust-side sort needed
    group
}

fn complete_builtins(prefix: &str) -> CompletionGroup {
    let mut group = CompletionGroup::new("builtin command");
    group.explanation = Some("builtin command".to_string());

    let prefix_lower = prefix.to_lowercase();

    // Zsh builtins
    let builtins = [
        ".",
        ":",
        "[",
        "alias",
        "autoload",
        "bg",
        "bindkey",
        "break",
        "builtin",
        "bye",
        "cap",
        "cd",
        "chdir",
        "clone",
        "command",
        "comparguments",
        "compcall",
        "compctl",
        "compdescribe",
        "compfiles",
        "compgroups",
        "compquote",
        "comptags",
        "comptry",
        "compvalues",
        "continue",
        "declare",
        "dirs",
        "disable",
        "disown",
        "echo",
        "echotc",
        "echoti",
        "emulate",
        "enable",
        "eval",
        "exec",
        "exit",
        "export",
        "false",
        "fc",
        "fg",
        "float",
        "functions",
        "getcap",
        "getln",
        "getopts",
        "hash",
        "history",
        "integer",
        "jobs",
        "kill",
        "let",
        "limit",
        "local",
        "log",
        "logout",
        "noglob",
        "popd",
        "print",
        "printf",
        "private",
        "pushd",
        "pushln",
        "pwd",
        "r",
        "read",
        "readonly",
        "rehash",
        "return",
        "sched",
        "set",
        "setcap",
        "setopt",
        "shift",
        "source",
        "stat",
        "suspend",
        "test",
        "times",
        "trap",
        "true",
        "ttyctl",
        "type",
        "typeset",
        "ulimit",
        "umask",
        "unalias",
        "unfunction",
        "unhash",
        "unlimit",
        "unset",
        "unsetopt",
        "vared",
        "wait",
        "whence",
        "where",
        "which",
        "zcompile",
        "zformat",
        "zftp",
        "zle",
        "zmodload",
        "zparseopts",
        "zprof",
        "zpty",
        "zregexparse",
        "zsocket",
        "zstat",
        "zstyle",
        "ztcp",
    ];

    for cmd in builtins {
        if cmd.to_lowercase().starts_with(&prefix_lower) {
            group.matches.push(Completion::new(cmd.to_string()));
        }
    }

    group.matches.sort_by(|a, b| a.str_.cmp(&b.str_));
    group
}

/// Complete from recent history commands (last-ten tag from _megacomplete)
fn complete_last_commands(prefix: &str) -> CompletionGroup {
    let mut group = CompletionGroup::new("last commands");
    group.explanation = Some("last commands".to_string());

    let prefix_lower = prefix.to_lowercase();

    // Read from zsh history file
    let histfile = std::env::var("HISTFILE").unwrap_or_else(|_| {
        std::env::var("HOME")
            .map(|h| format!("{}/.zsh_history", h))
            .unwrap_or_default()
    });

    if let Ok(content) = std::fs::read_to_string(&histfile) {
        let mut seen = std::collections::HashSet::new();
        // Read last 200 lines, extract commands
        for line in content.lines().rev().take(200) {
            // Zsh extended history format: : timestamp:0;command
            let cmd = if line.starts_with(':') {
                line.split(';').nth(1).unwrap_or(line)
            } else {
                line
            };

            let cmd = cmd.trim();
            if !cmd.is_empty()
                && cmd.to_lowercase().starts_with(&prefix_lower)
                && seen.insert(cmd.to_string())
            {
                group.matches.push(Completion::new(cmd.to_string()));
                if group.matches.len() >= 20 {
                    break;
                }
            }
        }
    }

    group
}

/// Complete from last command arguments (last-line tag from _megacomplete)
fn complete_last_command_args(prefix: &str) -> CompletionGroup {
    let mut group = CompletionGroup::new("last command args");
    group.explanation = Some("last command args".to_string());

    let prefix_lower = prefix.to_lowercase();

    // Read last command from history
    let histfile = std::env::var("HISTFILE").unwrap_or_else(|_| {
        std::env::var("HOME")
            .map(|h| format!("{}/.zsh_history", h))
            .unwrap_or_default()
    });

    if let Ok(content) = std::fs::read_to_string(&histfile) {
        // Get the last non-empty line
        if let Some(line) = content.lines().rev().find(|l| !l.trim().is_empty()) {
            let cmd = if line.starts_with(':') {
                line.split(';').nth(1).unwrap_or(line)
            } else {
                line
            };

            // Split into words and offer each as completion
            let mut seen = std::collections::HashSet::new();
            for word in cmd.split_whitespace() {
                if word.to_lowercase().starts_with(&prefix_lower) && seen.insert(word.to_string()) {
                    group.matches.push(Completion::new(word.to_string()));
                }
            }

            // Also offer the full command with various wrappings (like _megacomplete)
            let cmd = cmd.trim();
            if !cmd.is_empty() && cmd.to_lowercase().starts_with(&prefix_lower) {
                if seen.insert(cmd.to_string()) {
                    group.matches.push(Completion::new(cmd.to_string()));
                }
                // Quoted forms
                let quoted = format!("\"{}\"", cmd);
                if quoted.to_lowercase().starts_with(&prefix_lower) && seen.insert(quoted.clone()) {
                    group.matches.push(Completion::new(quoted));
                }
                // Command substitution form
                let subst = format!("$({})", cmd);
                if subst.to_lowercase().starts_with(&prefix_lower) && seen.insert(subst.clone()) {
                    group.matches.push(Completion::new(subst));
                }
            }
        }
    }

    group
}

fn complete_external_commands(prefix: &str) -> CompletionGroup {
    let mut group = CompletionGroup::new("external command");
    group.explanation = Some("external command".to_string());

    let prefix_lower = prefix.to_lowercase();
    let mut seen = std::collections::HashSet::new();

    // Scan ALL PATH directories for executables
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    if let Some(name) = entry.file_name().to_str() {
                        if name.to_lowercase().starts_with(&prefix_lower)
                            && seen.insert(name.to_string())
                        {
                            group.matches.push(Completion::new(name.to_string()));
                        }
                    }
                }
            }
        }
    }

    group.matches.sort_by(|a, b| a.str_.cmp(&b.str_));
    group
}

fn complete_directories(prefix: &str) -> CompletionGroup {
    let mut group = CompletionGroup::new("directory");
    group.explanation = Some("directory".to_string());

    let (dir, file_prefix) = if prefix.contains('/') {
        let idx = prefix.rfind('/').unwrap();
        let dir = if idx == 0 { "/" } else { &prefix[..idx] };
        (dir.to_string(), &prefix[idx + 1..])
    } else {
        (".".to_string(), prefix)
    };

    let dir_path = if dir.starts_with('~') {
        if let Some(home) = std::env::var("HOME").ok() {
            dir.replacen('~', &home, 1)
        } else {
            dir.clone()
        }
    } else {
        dir.clone()
    };

    if let Ok(entries) = std::fs::read_dir(&dir_path) {
        let prefix_lower = file_prefix.to_lowercase();
        for entry in entries.flatten() {
            if let Ok(ft) = entry.file_type() {
                if !ft.is_dir() {
                    continue;
                }
            }
            if let Some(name) = entry.file_name().to_str() {
                if name.to_lowercase().starts_with(&prefix_lower) || file_prefix.is_empty() {
                    let display = if dir == "." {
                        format!("{}/", name)
                    } else if dir.ends_with('/') {
                        format!("{}{}/", dir, name)
                    } else {
                        format!("{}/{}/", dir, name)
                    };
                    group.matches.push(Completion::new(display));
                }
            }
        }
    }

    group.matches.sort_by(|a, b| a.str_.cmp(&b.str_));
    group
}

fn complete_files(prefix: &str, include_dirs: bool) -> CompletionGroup {
    let mut group = CompletionGroup::new("files");
    group.explanation = Some("files".to_string());

    let (dir, file_prefix) = if prefix.contains('/') {
        let idx = prefix.rfind('/').unwrap();
        let dir = if idx == 0 { "/" } else { &prefix[..idx] };
        (dir.to_string(), &prefix[idx + 1..])
    } else {
        (".".to_string(), prefix)
    };

    let dir_path = if dir.starts_with('~') {
        if let Some(home) = std::env::var("HOME").ok() {
            dir.replacen('~', &home, 1)
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
                if name.to_lowercase().starts_with(&prefix_lower) || file_prefix.is_empty() {
                    let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    if !include_dirs && is_dir {
                        continue;
                    }

                    let display = if dir == "." {
                        name.to_string()
                    } else if dir.ends_with('/') {
                        format!("{}{}", dir, name)
                    } else {
                        format!("{}/{}", dir, name)
                    };

                    let mut comp = Completion::new(if is_dir {
                        format!("{}/", display)
                    } else {
                        display.clone()
                    });
                    if is_dir {
                        comp.modec = '/';
                    } else {
                        // Check if executable
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            if let Ok(meta) = entry.metadata() {
                                if meta.permissions().mode() & 0o111 != 0 {
                                    comp.modec = '*';
                                }
                            }
                        }
                        // Check if symlink
                        if entry.file_type().map(|t| t.is_symlink()).unwrap_or(false) {
                            comp.modec = '@';
                        }
                    }
                    group.matches.push(comp);
                }
            }
        }
    }

    group.matches.sort_by(|a, b| {
        let a_dir = a.str_.ends_with('/');
        let b_dir = b.str_.ends_with('/');
        if a_dir != b_dir {
            return b_dir.cmp(&a_dir);
        }
        a.str_.cmp(&b.str_)
    });
    group
}

fn dump_demo() {
    println!("=== Menu Completion Demo (zsh-style) ===\n");

    // Initialize cache and show stats
    let cache = open_cache();
    let stats = cache.stats().unwrap();
    println!("SQLite Cache Stats:");
    println!("  comps: {} command completions", stats.comps);
    println!("  autoloads: {} functions", stats.autoloads);
    println!("  patcomps: {} patterns", stats.patcomps);
    println!("  services: {} services", stats.services);
    println!();

    let mut menu = MenuState::new();
    menu.set_term_size(180, 60);
    menu.set_available_rows(5000);
    menu.set_show_headers(true);

    let zpwr_colors = zpwr_list_colors();
    menu.set_group_colors(zpwr_colors);

    // Test 1: git <TAB> - show git subcommands
    println!("--- git <TAB> - git subcommands with descriptions ---\n");
    let mut editor = Editor::new();
    for c in "git ".chars() {
        editor.insert(c);
    }
    let groups = generate_completions(&editor);

    for g in &groups {
        println!("  {} : {} matches", g.name, g.matches.len());
    }
    println!();

    menu.set_prefix(editor.current_word());
    menu.set_completions(&groups);
    menu.start();
    let r = menu.render();
    println!("Input: '{}' ({} matches)\n", editor.line, menu.count());

    for line in r.lines.iter().take(20) {
        println!("{}", line.content);
    }

    // Test 2: git checkout <TAB> - show branches
    println!("\n--- git checkout <TAB> - branches ---\n");
    editor.clear();
    for c in "git checkout ".chars() {
        editor.insert(c);
    }
    let groups = generate_completions(&editor);

    for g in &groups {
        println!("  {} : {} matches", g.name, g.matches.len());
    }
    println!();

    menu.set_prefix(editor.current_word());
    menu.set_completions(&groups);
    menu.start();
    let r = menu.render();
    println!("Input: '{}' ({} matches)\n", editor.line, menu.count());

    for line in r.lines.iter().take(10) {
        println!("{}", line.content);
    }

    // Test 3: git add <TAB> - show modified files
    println!("\n--- git add <TAB> - modified files ---\n");
    editor.clear();
    for c in "git add ".chars() {
        editor.insert(c);
    }
    let groups = generate_completions(&editor);

    for g in &groups {
        println!("  {} : {} matches", g.name, g.matches.len());
    }
    println!();

    menu.set_prefix(editor.current_word());
    menu.set_completions(&groups);
    menu.start();
    let r = menu.render();
    println!("Input: '{}' ({} matches)\n", editor.line, menu.count());

    for line in r.lines.iter().take(10) {
        println!("{}", line.content);
    }

    // Test 4: Command completion with prefix
    println!("\n--- a<TAB> - command completion (multiple groups) ---\n");
    editor.clear();
    editor.insert('a');
    let groups = generate_completions(&editor);

    for g in &groups {
        println!("  {} : {} matches", g.name, g.matches.len());
    }
    println!();

    menu.set_prefix(editor.current_word());
    menu.set_completions(&groups);
    menu.start();
    let r = menu.render();
    println!("Input: '{}' ({} matches)\n", editor.line, menu.count());

    let mut lines_shown = 0;
    for line in &r.lines {
        if line.content.contains("-<<") || lines_shown < 3 {
            println!("{}", line.content);
            if !line.content.contains("-<<") {
                lines_shown += 1;
            }
        } else if line.content.contains("-<<") {
            lines_shown = 0;
        }
    }

    // Test 5: docker <TAB> - management and common commands
    println!("\n--- docker <TAB> - docker commands (2 sections) ---\n");
    editor.clear();
    for c in "docker ".chars() {
        editor.insert(c);
    }
    let groups = generate_completions(&editor);

    for g in &groups {
        println!("  {} : {} matches", g.name, g.matches.len());
    }
    println!();

    menu.set_prefix(editor.current_word());
    menu.set_completions(&groups);
    menu.start();
    let r = menu.render();
    println!("Input: '{}' ({} matches)\n", editor.line, menu.count());

    for line in r.lines.iter().take(15) {
        println!("{}", line.content);
    }

    // Test 6: zsh -<TAB> - zsh options (3 sections)
    println!("\n--- zsh -<TAB> - zsh options (3 sections) ---\n");
    editor.clear();
    for c in "zsh -".chars() {
        editor.insert(c);
    }
    let groups = generate_completions(&editor);

    for g in &groups {
        println!("  {} : {} matches", g.name, g.matches.len());
    }
    println!();

    menu.set_prefix(editor.current_word());
    menu.set_completions(&groups);
    menu.start();
    let r = menu.render();
    println!("Input: '{}' ({} matches)\n", editor.line, menu.count());

    for line in r.lines.iter().take(20) {
        println!("{}", line.content);
    }

    // Test 7: zsh --emulate <TAB> - emulation modes (parsed from _zsh)
    println!("\n--- zsh --emulate <TAB> - emulation modes (from _zsh) ---\n");
    editor.clear();
    for c in "zsh --emulate ".chars() {
        editor.insert(c);
    }
    let groups = generate_completions(&editor);

    for g in &groups {
        println!("  {} : {} matches", g.name, g.matches.len());
    }
    println!();

    menu.set_prefix(editor.current_word());
    menu.set_completions(&groups);
    menu.start();
    let r = menu.render();
    println!("Input: '{}' ({} matches)\n", editor.line, menu.count());

    for line in &r.lines {
        println!("{}", line.content);
    }

    // Test 8: zpwr <TAB> - subcommands parsed from _zpwr
    println!("\n--- zpwr <TAB> - subcommands from _zpwr ---\n");
    editor.clear();
    for c in "zpwr ".chars() {
        editor.insert(c);
    }
    let groups = generate_completions(&editor);

    for g in &groups {
        println!("  {} : {} matches", g.name, g.matches.len());
    }
    println!();

    menu.set_prefix(editor.current_word());
    menu.set_completions(&groups);
    menu.start();
    let r = menu.render();
    println!("Input: '{}' ({} matches)\n", editor.line, menu.count());

    for line in r.lines.iter().take(20) {
        println!("{}", line.content);
    }

    // Test 9: bash --<TAB> - options parsed from _bash (NOT zsh options!)
    println!("\n--- bash --<TAB> - options from _bash ---\n");
    editor.clear();
    for c in "bash --".chars() {
        editor.insert(c);
    }
    let groups = generate_completions(&editor);

    for g in &groups {
        println!("  {} : {} matches", g.name, g.matches.len());
    }
    println!();

    menu.set_prefix(editor.current_word());
    menu.set_completions(&groups);
    menu.start();
    let r = menu.render();
    println!("Input: '{}' ({} matches)\n", editor.line, menu.count());

    for line in r.lines.iter().take(15) {
        println!("{}", line.content);
    }

    // Test 10: echo $HO<TAB> - parameter completion
    println!("\n--- echo $HO<TAB> - parameter completion ---\n");
    editor.clear();
    for c in "echo $HO".chars() {
        editor.insert(c);
    }
    let groups = generate_completions(&editor);

    for g in &groups {
        println!("  {} : {} matches", g.name, g.matches.len());
    }
    println!();

    menu.set_prefix(editor.current_word());
    menu.set_completions(&groups);
    menu.start();
    let r = menu.render();
    println!("Input: '{}' ({} matches)\n", editor.line, menu.count());

    for line in r.lines.iter().take(15) {
        println!("{}", line.content);
    }

    // Test 11: echo $<TAB> - all parameters
    println!("\n--- echo $<TAB> - all parameters (env vars) ---\n");
    editor.clear();
    for c in "echo $".chars() {
        editor.insert(c);
    }
    let groups = generate_completions(&editor);

    for g in &groups {
        println!("  {} : {} matches", g.name, g.matches.len());
    }
    println!();

    menu.set_prefix(editor.current_word());
    menu.set_completions(&groups);
    menu.start();
    let r = menu.render();
    println!("Input: '{}' ({} matches)\n", editor.line, menu.count());

    for line in r.lines.iter().take(15) {
        println!("{}", line.content);
    }

    // Test 12: echo ${(<TAB> - parameter flags
    println!("\n--- echo ${{(<TAB> - parameter expansion flags ---\n");
    editor.clear();
    for c in "echo ${(".chars() {
        editor.insert(c);
    }
    let groups = generate_completions(&editor);

    for g in &groups {
        println!("  {} : {} matches", g.name, g.matches.len());
    }
    println!();

    menu.set_prefix(editor.current_word());
    menu.set_completions(&groups);
    menu.start();
    let r = menu.render();
    println!("Input: '{}' ({} matches)\n", editor.line, menu.count());

    for line in r.lines.iter().take(20) {
        println!("{}", line.content);
    }

    // Test 13: *(<TAB> - glob qualifiers
    println!("\n--- *(<TAB> - glob qualifiers ---\n");
    editor.clear();
    for c in "*(".chars() {
        editor.insert(c);
    }
    let groups = generate_completions(&editor);

    for g in &groups {
        println!("  {} : {} matches", g.name, g.matches.len());
    }
    println!();

    menu.set_prefix(editor.current_word());
    menu.set_completions(&groups);
    menu.start();
    let r = menu.render();
    println!("Input: '{}' ({} matches)\n", editor.line, menu.count());

    for line in r.lines.iter().take(25) {
        println!("{}", line.content);
    }

    // Test 14: Scrolling test - parameters with limited viewport
    println!("\n--- Scrolling Test: $<TAB> with 10 row viewport ---\n");
    let mut scroll_menu = MenuState::new();
    scroll_menu.set_term_size(180, 60);
    scroll_menu.set_available_rows(10); // Limited viewport to trigger scrolling
    scroll_menu.set_show_headers(true);
    scroll_menu.set_group_colors(zpwr_list_colors());

    editor.clear();
    for c in "echo $".chars() {
        editor.insert(c);
    }
    let groups = generate_completions(&editor);

    scroll_menu.set_prefix(editor.current_word());
    scroll_menu.set_completions(&groups);
    scroll_menu.start();

    // Render at top
    let r = scroll_menu.render();
    println!(
        "Viewport 10 rows, {} total matches, {} total rows",
        r.total_matches, r.total_rows
    );
    println!("Scrollable: {}", r.scrollable);
    if let Some(status) = &r.status {
        println!("Status: {}", status);
    }
    println!();

    for line in &r.lines {
        println!("{}", line.content);
    }

    // Navigate to middle
    println!("\n--- After navigating down 50 times ---\n");
    for _ in 0..50 {
        scroll_menu.navigate(compsys::MenuMotion::Down);
    }
    let r = scroll_menu.render();
    if let Some(status) = &r.status {
        println!("Status: {}", status);
    }
    for line in &r.lines {
        println!("{}", line.content);
    }

    // Navigate to bottom
    println!("\n--- After navigating to end ---\n");
    scroll_menu.navigate(compsys::MenuMotion::Last);
    let r = scroll_menu.render();
    if let Some(status) = &r.status {
        println!("Status: {}", status);
    }
    for line in &r.lines {
        println!("{}", line.content);
    }

    // Test 15: echo ~Z<TAB> - named directories from hash -d
    println!("\n--- echo ~Z<TAB> - named directories (hash -d) ---\n");
    editor.clear();
    for c in "echo ~Z".chars() {
        editor.insert(c);
    }
    let groups = generate_completions(&editor);

    for g in &groups {
        println!("  {} : {} matches", g.name, g.matches.len());
    }
    println!();

    menu.set_prefix(editor.current_word());
    menu.set_completions(&groups);
    menu.start();
    let r = menu.render();
    println!("Input: '{}' ({} matches)\n", editor.line, menu.count());

    for line in r.lines.iter().take(25) {
        println!("{}", line.content);
    }

    // Test 16: echo ~<TAB> - all named directories and users
    println!("\n--- echo ~<TAB> - all named directories and users ---\n");
    editor.clear();
    for c in "echo ~".chars() {
        editor.insert(c);
    }
    let groups = generate_completions(&editor);

    for g in &groups {
        println!("  {} : {} matches", g.name, g.matches.len());
    }
    println!();

    menu.set_prefix(editor.current_word());
    menu.set_completions(&groups);
    menu.start();
    let r = menu.render();
    println!("Input: '{}' ({} matches)\n", editor.line, menu.count());

    for line in r.lines.iter().take(30) {
        println!("{}", line.content);
    }

    // Test 17: zpwr<TAB> - shell functions (ZPWR autoloaded functions)
    println!("\n--- zpwr<TAB> - shell functions from FPATH ---\n");
    editor.clear();
    for c in "zpwr".chars() {
        editor.insert(c);
    }
    let groups = generate_completions(&editor);

    for g in &groups {
        println!("  {} : {} matches", g.name, g.matches.len());
    }
    println!();

    menu.set_prefix(editor.current_word());
    menu.set_completions(&groups);
    menu.start();
    let r = menu.render();
    println!("Input: '{}' ({} matches)\n", editor.line, menu.count());

    for line in r.lines.iter().take(30) {
        println!("{}", line.content);
    }

    // Test 18: z<TAB> - commands (only existing executables, not stale cache entries)
    println!("\n--- z<TAB> - commands (only existing executables) ---\n");
    editor.clear();
    editor.insert('z');
    let groups = generate_completions(&editor);

    for g in &groups {
        println!("  {} : {} matches", g.name, g.matches.len());
    }
    println!();

    menu.set_prefix(editor.current_word());
    menu.set_completions(&groups);
    menu.start();
    let r = menu.render();
    println!("Input: '{}' ({} matches)\n", editor.line, menu.count());

    for line in r.lines.iter().take(30) {
        println!("{}", line.content);
    }
}

struct Terminal {
    orig: libc::termios,
}

impl Terminal {
    fn new() -> io::Result<Self> {
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(0, &mut t) != 0 {
                return Err(io::Error::last_os_error());
            }
            let orig = t;
            t.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
            t.c_iflag &= !(libc::IXON | libc::ICRNL);
            t.c_cc[libc::VMIN] = 0;
            t.c_cc[libc::VTIME] = 1;
            if libc::tcsetattr(0, libc::TCSAFLUSH, &t) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self { orig })
        }
    }
    fn restore(&self) {
        unsafe {
            libc::tcsetattr(0, libc::TCSAFLUSH, &self.orig);
        }
    }
    fn init_raw(&self) {
        unsafe {
            let mut t = self.orig;
            t.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
            t.c_iflag &= !(libc::IXON | libc::ICRNL);
            t.c_cc[libc::VMIN] = 0;
            t.c_cc[libc::VTIME] = 1;
            libc::tcsetattr(0, libc::TCSAFLUSH, &t);
        }
    }
    fn size(&self) -> (usize, usize) {
        unsafe {
            let mut w: libc::winsize = std::mem::zeroed();
            if libc::ioctl(1, libc::TIOCGWINSZ, &mut w) == 0 {
                (w.ws_col as usize, w.ws_row as usize)
            } else {
                (80, 24)
            }
        }
    }
    fn clear(&self) {
        print!("\x1b[2J");
    }
    fn goto(&self, _x: usize, y: usize) {
        print!("\x1b[{};1H", y + 1);
    }
    fn write_str(&self, s: &str) {
        print!("{}", s);
    }
    fn flush(&self) {
        let _ = io::stdout().flush();
    }
    fn read(&self, buf: &mut [u8]) -> usize {
        io::stdin().read(buf).unwrap_or(0)
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.restore();
    }
}
