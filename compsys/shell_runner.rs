//! Shell function runner for completion functions
//!
//! This module provides the bridge between interpreted zsh completion functions
//! (like `_git`, `_ls`) and the native Rust compsys primitives.
//!
//! When a completion function is executed:
//! 1. The shell interpreter sets up completion state ($words, $CURRENT, etc.)
//! 2. The function body is executed
//! 3. Calls to builtins like `_arguments`, `compadd` dispatch to Rust
//! 4. Generated completions are collected and returned

use crate::{
    state::CompletionContext as StateContext, CompParams, CompState, Completion, CompletionGroup,
    CompletionReceiver, ZStyleStore,
};
use std::collections::HashMap;

/// Completion context passed to shell function runner
#[derive(Debug, Clone)]
pub struct ShellCompletionContext {
    /// The words on the command line (split)
    pub words: Vec<String>,
    /// 1-based index of current word being completed
    pub current: i32,
    /// The current prefix being completed
    pub prefix: String,
    /// The suffix after cursor
    pub suffix: String,
    /// Current context string (e.g., ":completion::complete:git:")
    pub curcontext: String,
    /// Service name (command being completed)
    pub service: String,
    /// Completion state
    pub compstate: HashMap<String, String>,
}

impl Default for ShellCompletionContext {
    fn default() -> Self {
        Self {
            words: Vec::new(),
            current: 1,
            prefix: String::new(),
            suffix: String::new(),
            curcontext: String::new(),
            service: String::new(),
            compstate: HashMap::new(),
        }
    }
}

impl ShellCompletionContext {
    /// Create context from command line
    pub fn from_command_line(line: &str, cursor_pos: usize) -> Self {
        let words: Vec<String> = line.split_whitespace().map(String::from).collect();

        // Find which word the cursor is in
        let mut current: i32 = (words.len() + 1) as i32; // Default to new word
        let mut prefix = String::new();
        let mut suffix = String::new();
        let mut found = false;
        let mut pos = 0;

        for (i, word) in words.iter().enumerate() {
            let word_start = line[pos..].find(word).map(|p| pos + p).unwrap_or(pos);
            let word_end = word_start + word.len();

            if cursor_pos >= word_start && cursor_pos <= word_end {
                current = (i + 1) as i32;
                prefix = word[..cursor_pos.saturating_sub(word_start)].to_string();
                suffix = word[cursor_pos.saturating_sub(word_start)..].to_string();
                found = true;
                break;
            }
            pos = word_end;
        }

        // If cursor is in a gap between words but before the end, it's completing a new word
        // at that position (not at the end)
        if !found && !words.is_empty() {
            // Find which gap the cursor is in
            pos = 0;
            for (i, word) in words.iter().enumerate() {
                let word_start = line[pos..].find(word).map(|p| pos + p).unwrap_or(pos);
                if cursor_pos < word_start {
                    current = (i + 1) as i32;
                    break;
                }
                pos = word_start + word.len();
            }
        }

        let service = words.first().cloned().unwrap_or_default();
        let curcontext = format!(":completion::complete:{}:", service);

        Self {
            words,
            current,
            prefix,
            suffix,
            curcontext,
            service,
            compstate: Self::default_compstate(),
        }
    }

    fn default_compstate() -> HashMap<String, String> {
        let mut state = HashMap::new();
        state.insert("context".to_string(), "command".to_string());
        state.insert("insert".to_string(), "unambiguous".to_string());
        state.insert("list".to_string(), "list".to_string());
        state
    }

    /// Convert to CompParams for native functions
    pub fn to_comp_params(&self) -> CompParams {
        CompParams {
            words: self.words.clone(),
            current: self.current,
            prefix: self.prefix.clone(),
            suffix: self.suffix.clone(),
            iprefix: String::new(),
            isuffix: String::new(),
            qiprefix: String::new(),
            qisuffix: String::new(),
            compstate: self.to_comp_state(),
        }
    }

    /// Convert to CompState
    pub fn to_comp_state(&self) -> CompState {
        CompState {
            context: StateContext::Command,
            parameter: String::new(),
            redirect: String::new(),
            quote: String::new(),
            quoting: String::new(),
            restore: String::new(),
            list: self.compstate.get("list").cloned().unwrap_or_default(),
            insert: self.compstate.get("insert").cloned().unwrap_or_default(),
            exact: String::new(),
            exact_string: String::new(),
            pattern_insert: String::new(),
            pattern_match: String::new(),
            unambiguous: String::new(),
            unambiguous_cursor: 0,
            unambiguous_positions: String::new(),
            insert_positions: String::new(),
            list_max: 0,
            last_prompt: String::new(),
            to_end: String::new(),
            old_list: String::new(),
            old_insert: String::new(),
            vared: String::new(),
            list_lines: 0,
            nmatches: 0,
            ignored: 0,
            all_quotes: String::new(),
        }
    }
}

/// Result from running a completion function
#[derive(Debug, Default)]
pub struct CompletionResult {
    /// Generated completion groups
    pub groups: Vec<CompletionGroup>,
    /// Messages to display
    pub messages: Vec<String>,
    /// Whether completion succeeded
    pub success: bool,
    /// Return status of the function
    pub status: i32,
}

/// Trait for shell interpreters that can run completion functions
pub trait CompletionRunner {
    /// Run a completion function by name
    fn run_completion_function(
        &mut self,
        func_name: &str,
        context: &ShellCompletionContext,
        zstyle: &ZStyleStore,
    ) -> CompletionResult;

    /// Check if a completion function exists
    fn has_completion_function(&self, name: &str) -> bool;

    /// Get the completion function for a command
    fn get_completer(&self, command: &str) -> Option<String>;
}

/// Builtin dispatcher - maps completion builtin calls to Rust implementations
pub struct BuiltinDispatcher {
    /// Accumulated completions
    pub receiver: CompletionReceiver,
    /// Messages
    messages: Vec<String>,
    /// ZStyle store
    zstyle: ZStyleStore,
}

impl Default for BuiltinDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl BuiltinDispatcher {
    pub fn new() -> Self {
        Self {
            receiver: CompletionReceiver::unlimited(),
            messages: Vec::new(),
            zstyle: ZStyleStore::new(),
        }
    }

    pub fn with_zstyle(zstyle: ZStyleStore) -> Self {
        Self {
            receiver: CompletionReceiver::unlimited(),
            messages: Vec::new(),
            zstyle,
        }
    }

    /// Dispatch _message
    pub fn message(&mut self, msg: &str) {
        self.messages.push(msg.to_string());
    }

    /// Dispatch zstyle lookup
    pub fn zstyle_lookup(&self, context: &str, style: &str) -> Option<String> {
        self.zstyle.lookup_str(context, style).map(String::from)
    }

    /// Begin a new completion group
    pub fn begin_group(&mut self, name: &str, sorted: bool) {
        self.receiver.begin_group(name, sorted);
    }

    /// End current group (switch back to default)
    pub fn end_group(&mut self) {
        self.receiver.begin_group("default", true);
    }

    /// Add a completion
    pub fn add_completion(&mut self, comp: Completion) {
        self.receiver.add(comp);
    }

    /// Add completions with descriptions
    pub fn add_described(
        &mut self,
        tag: &str,
        description: &str,
        items: &[(String, Option<String>)],
    ) {
        self.receiver.begin_group(tag, true);
        self.receiver.add_explanation(description);

        for (value, desc) in items {
            let mut comp = Completion::new(value);
            if let Some(d) = desc {
                comp = comp.with_description(d);
            }
            self.receiver.add(comp);
        }

        self.receiver.begin_group("default", true);
    }

    /// Add file completions
    pub fn add_files(&mut self, prefix: &str, dirs_only: bool) {
        use std::path::Path;

        let dir = if prefix.is_empty() {
            Path::new(".")
        } else if prefix.ends_with('/') {
            Path::new(prefix)
        } else {
            Path::new(prefix).parent().unwrap_or(Path::new("."))
        };

        let file_prefix = if prefix.contains('/') {
            prefix.rsplit('/').next().unwrap_or("")
        } else {
            prefix
        };

        if let Ok(entries) = std::fs::read_dir(dir) {
            let tag = if dirs_only { "directories" } else { "files" };
            self.receiver.begin_group(tag, true);

            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if !name.starts_with(file_prefix) && !file_prefix.is_empty() {
                        continue;
                    }

                    let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);

                    if dirs_only && !is_dir {
                        continue;
                    }

                    let comp = if is_dir {
                        Completion::new(name).with_suffix("/")
                    } else {
                        Completion::new(name)
                    };
                    self.receiver.add(comp);
                }
            }

            self.receiver.begin_group("default", true);
        }
    }

    /// Finalize and get results
    pub fn finish(self) -> CompletionResult {
        let groups = self.receiver.take();
        let success = groups.iter().any(|g| !g.matches.is_empty()) || !self.messages.is_empty();

        CompletionResult {
            groups,
            messages: self.messages,
            success,
            status: if success { 0 } else { 1 },
        }
    }
}

/// Helper to call external programs for completion (like `git branch`)
pub fn call_program(
    _tag: &str,
    command: &str,
    args: &[&str],
) -> Result<Vec<String>, std::io::Error> {
    use std::process::Command;

    let output = Command::new(command).args(args).output()?;

    if !output.status.success() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("{} failed with status {}", command, output.status),
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.lines().map(String::from).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_context_from_command_line() {
        let ctx = ShellCompletionContext::from_command_line("git add ", 8);
        assert_eq!(ctx.words, vec!["git", "add"]);
        assert_eq!(ctx.current, 3); // Completing 3rd word
        assert_eq!(ctx.service, "git");
    }

    #[test]
    fn test_context_mid_word() {
        // "git com" with cursor at position 6 (in the middle of "com" - after "co")
        // g=0,i=1,t=2, =3,c=4,o=5,m=6
        // Position 6 is AT "m", so prefix is "co" (positions 4,5)
        let ctx = ShellCompletionContext::from_command_line("git com", 6);
        assert_eq!(ctx.words, vec!["git", "com"]);
        assert_eq!(ctx.current, 2);
        assert_eq!(ctx.prefix, "co");
        assert_eq!(ctx.suffix, "m");
    }

    #[test]
    fn test_builtin_dispatcher() {
        let mut dispatcher = BuiltinDispatcher::new();

        dispatcher.begin_group("commands", true);
        dispatcher.add_completion(Completion::new("add"));
        dispatcher.add_completion(Completion::new("commit"));
        dispatcher.end_group();

        let result = dispatcher.finish();
        assert!(result.success);
        assert!(!result.groups.is_empty());
    }

    #[test]
    fn test_call_program() {
        // Test with a simple command that should exist
        let result = call_program("test", "echo", &["hello"]);
        assert!(result.is_ok());
        let lines = result.unwrap();
        assert_eq!(lines, vec!["hello"]);
    }
}
