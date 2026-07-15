//! Port of `_module_math_func` from
//! `Completion/Zsh/Type/_module_math_func`.
//!
//! Full upstream body (12 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local mod
//! sh: 4  local -a funcs alts
//! sh: 5  local -a modules=( example mathfunc system )
//! sh: 6
//! sh: 7  for mod in $modules; do
//! sh: 8    funcs=( ${${${(f)"$(zmodload -Fl zsh/$mod 2>/dev/null)"}:#^+f:*}##+f:} )
//! sh: 9    alts+=( "module-math-functions.${mod}:math function from zsh/${mod}:compadd -S '(' $funcs" )
//! sh:10  done
//! sh:11
//! sh:12  _alternative $alts
//! ```
//!
//! `zmodload -Fl zsh/$mod` lists the features of module `zsh/$mod`,
//! one per line, as `<onoff><type>:<name>` (e.g. `+f:abs`, `-b:example`).
//! The source keeps only enabled math functions — the `:#^+f:*` filter
//! discards every line that does NOT match `+f:*` — then `##+f:` strips
//! the prefix, leaving bare function names.
//!
//! Rather than shelling out to a command substitution to capture the
//! builtin's stdout, this port queries the module feature tables
//! directly via [`features_module`] + [`enables_module`] (the exact
//! data `zmodload -Fl` would print) and applies the identical
//! `+f:` filter. Enabled math features are found only for modules whose
//! features are enabled (loaded) at completion time — matching upstream,
//! where an unloaded module's `zmodload -Fl` (with stderr silenced)
//! yields nothing and `funcs` stays empty.

use crate::ported::exec::dispatch_function_call;
use crate::ported::module::{enables_module, features_module, MODULESTAB};

// sh:5 — local -a modules=( example mathfunc system )
const MODULES: &[&str] = &["example", "mathfunc", "system"];

/// sh:8 — reproduce `${${${(f)"$(zmodload -Fl zsh/$mod 2>/dev/null)"}:#^+f:*}##+f:}`.
///   Query `zsh/$mod`'s features; keep enabled (`+`) math-function
///   (`f:`) features; return their bare names.
fn module_math_funcs(m: &str) -> Vec<String> {
    let modname = format!("zsh/{}", m);
    let mut table = match MODULESTAB.lock() {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let table = &mut *table;
    let mut features: Vec<String> = Vec::new();
    // 2>/dev/null — a module that doesn't support features contributes
    //   nothing (features_module returns non-zero).
    if features_module(table, &modname, &mut features) != 0 {
        return Vec::new();
    }
    let mut enables_opt: Option<Vec<i32>> = None;
    let _ = enables_module(table, &modname, &mut enables_opt);
    let enables = enables_opt.unwrap_or_else(|| vec![0; features.len()]);

    let mut funcs = Vec::new();
    for (f, en) in features.iter().zip(enables.iter()) {
        // `:#^+f:*` keeps entries printed as `+f:<name>`, i.e. enabled
        //   features whose type prefix is `f:`; `##+f:` strips the
        //   prefix, leaving `<name>`.
        if *en != 0 {
            if let Some(name) = f.strip_prefix("f:") {
                funcs.push(name.to_string());
            }
        }
    }
    funcs
}

/// sh:7-10 — build the `$alts` array. Split out so behaviour is
///   testable without a live completion executor.
fn build_alts() -> Vec<String> {
    let mut alts: Vec<String> = Vec::new();
    // sh:7 — for mod in $modules; do
    for m in MODULES {
        // sh:8 — funcs=( ... )
        let funcs = module_math_funcs(m);
        // sh:9 — alts+=( "module-math-functions.${mod}:math function
        //         from zsh/${mod}:compadd -S '(' $funcs" ). The action
        //         field is executed by `_alternative`; it is the literal
        //         `compadd -S '(' <funcs>` the source builds.
        let mut action = String::from("compadd -S '('");
        for f in &funcs {
            action.push(' ');
            action.push_str(f);
        }
        alts.push(format!(
            "module-math-functions.{}:math function from zsh/{}:{}",
            m, m, action
        ));
    }
    alts
}

/// `_module_math_func` — complete math functions from
/// `zsh/{example,mathfunc,system}` modules.
pub fn _module_math_func() -> i32 {
    // sh:3-4 — local mod; local -a funcs alts (funcs/alts built below).
    let alts = build_alts();
    // sh:12 — _alternative $alts
    dispatch_function_call("_alternative", &alts).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_module_math_func(), 1);
    }

    #[test]
    fn alts_cover_exact_source_modules() {
        // sh:5,9 — one alt per module, in order, with the source's
        //   `module-math-functions.<mod>` tag, human description, and a
        //   `compadd -S '('` action. Pins the fix: the fabricated
        //   `random` module is gone and the action is the real
        //   `compadd -S '('` (not a bare parameter name).
        let _g = crate::test_util::global_state_lock();
        let alts = build_alts();
        assert_eq!(alts.len(), MODULES.len());
        for (spec, m) in alts.iter().zip(MODULES.iter()) {
            assert!(
                spec.starts_with(&format!("module-math-functions.{}:", m)),
                "spec {:?} must be tagged module-math-functions.{}",
                spec,
                m
            );
            assert!(
                spec.contains(&format!("math function from zsh/{}:", m)),
                "spec {:?} must carry the zsh/{} description",
                spec,
                m
            );
            assert!(
                spec.contains("compadd -S '('"),
                "spec {:?} action must run compadd -S '(' (executed action field)",
                spec
            );
        }
        assert!(
            !alts.iter().any(|s| s.contains("random")),
            "fabricated `random` module must not appear"
        );
    }
}
