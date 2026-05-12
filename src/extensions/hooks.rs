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
        if let Some(funcs) = self.hook_functions.get(hook_name).cloned() {
            for func_name in funcs {
                invoke(self, &func_name);
            }
        }
        // `<hook>_functions` array — zsh stdlib + add-zsh-hook idiom.
        let array_name = format!("{}_functions", hook_name);
        if let Some(funcs) = self.array(&array_name) {
            for func_name in funcs {
                invoke(self, &func_name);
            }
        }
    }
    /// Add a function to a hook
    pub fn add_hook(&mut self, hook_name: &str, func_name: &str) {
        self.hook_functions
            .entry(hook_name.to_string())
            .or_default()
            .push(func_name.to_string());
    }
}
// END moved-from-exec-rs
