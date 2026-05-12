//! User-installable shell hooks — extension; no zsh C counterpart.
#[allow(unused_imports)]
use crate::ported::exec::ShellExecutor;

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: drift
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// Run hook functions (precmd, preexec, chpwd, etc.)
    /// Direct port of zsh's `runhookdef` (Src/builtin.c) — invokes any
    /// function literally named the hook (`chpwd`, `precmd`,
    /// `preexec`) AND every entry in the matching `<hook>_functions`
    /// array.
    pub fn run_hooks(&mut self, hook_name: &str) {
        // Invoke a function by running its name as a script — zsh
        // dispatch order (alias → function → builtin → external)
        // hits the function. execute_script_zsh_pipeline runs through
        // the same compile path the user-typed call would.
        let invoke = |this: &mut Self, name: &str| {
            if this.function_exists(name) {
                let _ = this.execute_script(name);
            }
        };

        // The hook function itself (`chpwd`, `precmd`, `preexec`) is
        // invoked by NAME match in zsh — no registration step. Without
        // this, top-level `chpwd() { … }` never fires.
        invoke(self, hook_name);
        // `<hook>_functions` array — canonical zsh add-zsh-hook idiom.
        // Both top-level hook handlers and `add-zsh-hook` registrations
        // end up here; we no longer carry a side ledger on ShellExecutor.
        let array_name = format!("{}_functions", hook_name);
        if let Some(funcs) = self.array(&array_name) {
            for func_name in funcs {
                invoke(self, &func_name);
            }
        }
    }
    /// Add a function to a hook. Writes to the canonical
    /// `<hook>_functions` paramtab array (the zsh `add-zsh-hook` idiom).
    pub fn add_hook(&mut self, hook_name: &str, func_name: &str) {
        let array_name = format!("{}_functions", hook_name);
        let mut arr = self.array(&array_name).unwrap_or_default();
        if !arr.iter().any(|f| f == func_name) {
            arr.push(func_name.to_string());
            crate::ported::params::setaparam(&array_name, arr);
        }
    }
    /// Remove a function from a hook. Mirrors `add-zsh-hook -d`.
    pub fn delete_hook(&mut self, hook_name: &str, func_name: &str) {
        let array_name = format!("{}_functions", hook_name);
        if let Some(mut arr) = self.array(&array_name) {
            arr.retain(|f| f != func_name);
            crate::ported::params::setaparam(&array_name, arr);
        }
    }
}
// END moved-from-exec-rs
