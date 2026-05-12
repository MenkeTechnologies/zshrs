//! Command intercept / advice machinery — extension; no zsh C counterpart.
#[allow(unused_imports)]
use crate::ported::exec::ShellExecutor;
#[allow(unused_imports)]
use std::collections::HashMap;

/// AOP advice type — before, after, or around.
#[derive(Debug, Clone)]
/// Aspect-oriented advice classification.
/// zshrs-original — no C zsh counterpart. C zsh's closest
/// analog is the function-wrapper hook in Src/module.c
/// (`addwrapper()`, used by `zsh/zprof`), but per-function
/// before/after/around AOP intercepts are unique to zshrs.
pub enum AdviceKind {
    /// Run code before the command executes.
    Before,
    /// Run code after the command executes. $? and INTERCEPT_MS available.
    After,
    /// Wrap the command. Code must call `intercept_proceed` to run original.
    Around,
}

/// An intercept registration.
#[derive(Debug, Clone)]
/// One AOP intercept registered against a function pattern.
/// zshrs-original — no C counterpart.
pub struct Intercept {
    /// Pattern to match command names. Supports glob: "git *", "_*", "*".
    pub pattern: String,
    /// What kind of advice.
    pub kind: AdviceKind,
    /// Shell code to execute as advice.
    pub code: String,
    /// Unique ID for removal.
    pub id: u32,
}

/// Match an intercept pattern against a command name or full command string.
/// Supports: exact match, glob ("git *", "_*", "*"), or "all".
pub(crate) fn intercept_matches(pattern: &str, cmd_name: &str, full_cmd: &str) -> bool {
    if pattern == "*" || pattern == "all" {
        return true;
    }
    if pattern == cmd_name {
        return true;
    }
    if pattern.contains('*') || pattern.contains('?') {
        if let Ok(pat) = glob::Pattern::new(pattern) {
            return pat.matches(cmd_name) || pat.matches(full_cmd);
        }
    }
    false
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: drift
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// Check intercepts for a command. Returns Some(result) if an around
    /// advice fully handled the command, None to proceed normally.
    pub(crate) fn run_intercepts(
        &mut self,
        cmd_name: &str,
        full_cmd: &str,
        args: &[String],
    ) -> Option<Result<i32, String>> {
        // Collect matching intercepts (clone to avoid borrow issues)
        let matching: Vec<Intercept> = self
            .intercepts
            .iter()
            .filter(|i| intercept_matches(&i.pattern, cmd_name, full_cmd))
            .cloned()
            .collect();

        if matching.is_empty() {
            return None;
        }

        // Set INTERCEPT_NAME and INTERCEPT_ARGS for advice code
        self.variables
            .insert("INTERCEPT_NAME".to_string(), cmd_name.to_string());
        self.variables
            .insert("INTERCEPT_ARGS".to_string(), args.join(" "));
        self.variables
            .insert("INTERCEPT_CMD".to_string(), full_cmd.to_string());

        // Run before advice
        for advice in matching
            .iter()
            .filter(|i| matches!(i.kind, AdviceKind::Before))
        {
            let _ = self.execute_advice(&advice.code);
        }

        // Check for around advice — first match wins
        let around = matching
            .iter()
            .find(|i| matches!(i.kind, AdviceKind::Around));

        let t0 = std::time::Instant::now();

        let result = if let Some(advice) = around {
            // Around advice: set INTERCEPT_PROCEED flag, run advice code.
            // If advice calls `intercept_proceed`, the original command runs.
            self.variables
                .insert("__intercept_proceed".to_string(), "0".to_string());
            let advice_result = self.execute_advice(&advice.code);

            // Check if intercept_proceed was called
            let proceeded = self
                .variables
                .get("__intercept_proceed")
                .map(|v| v == "1")
                .unwrap_or(false);

            if proceeded {
                // The original command was already executed inside the advice
                advice_result
            } else {
                // Advice didn't call proceed — command was suppressed
                advice_result
            }
        } else {
            // No around advice — run the original command.
            // We return None to let the normal dispatch continue.
            // But we still need after advice to fire, so we can't return None here
            // if there are after advices. Run the command ourselves.
            let has_after = matching.iter().any(|i| matches!(i.kind, AdviceKind::After));
            if !has_after {
                // Only before advice, no after — let normal dispatch continue
                return None;
            }

            // Has after advice — we must run the command and then run after advice
            self.run_original_command(cmd_name, args)
        };

        let elapsed = t0.elapsed();

        // Set timing variable for after advice
        let ms = elapsed.as_secs_f64() * 1000.0;
        self.variables
            .insert("INTERCEPT_MS".to_string(), format!("{:.3}", ms));
        self.variables
            .insert("INTERCEPT_US".to_string(), format!("{:.0}", ms * 1000.0));

        // Run after advice
        for advice in matching
            .iter()
            .filter(|i| matches!(i.kind, AdviceKind::After))
        {
            let _ = self.execute_advice(&advice.code);
        }

        // Clean up
        self.variables.remove("INTERCEPT_NAME");
        self.variables.remove("INTERCEPT_ARGS");
        self.variables.remove("INTERCEPT_CMD");
        self.variables.remove("INTERCEPT_MS");
        self.variables.remove("INTERCEPT_US");
        self.variables.remove("__intercept_proceed");

        Some(result)
    }
    /// Execute the original command (used by around/after intercept dispatch).
    /// Execute advice code — dispatches @ prefix to stryke (fat binary),
    /// everything else to the shell parser. No fork. Machine code speed.
    pub(crate) fn execute_advice(&mut self, code: &str) -> Result<i32, String> {
        let code = code.trim();
        if code.starts_with('@') {
            let stryke_code = code.trim_start_matches('@').trim();
            if let Some(status) = crate::try_stryke_dispatch(stryke_code) {
                self.set_last_status(status);
                return Ok(status);
            }
            // No stryke handler (thin binary) — fall through to shell
        }
        self.execute_script(code)
    }
    pub(crate) fn run_original_command(&mut self, cmd_name: &str, args: &[String]) -> Result<i32, String> {
        // Function dispatch via the compiled pipeline (functions_compiled
        // first, falls back to legacy AST recompile if needed).
        if let Some(status) = self.dispatch_function_call(cmd_name, args) {
            return Ok(status);
        }
        // External command
        self.execute_external(cmd_name, args, &[])
    }
}
// END moved-from-exec-rs
