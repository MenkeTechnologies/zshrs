//! Port of `_module_math_func` from
//! `Completion/Zsh/Type/_module_math_func`.
//!
//! Full upstream body (12 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local mod
//! sh: 4  local -a funcs alts
//! sh: 5  local -a modules=( example mathfunc system random )
//! sh: 6
//! sh: 7  for mod in $modules; do
//! sh: 8    funcs=( ${${${(f)"$(zmodload -Fl zsh/$mod 2>/dev/null)"}:#^+f:*}##+f:} )
//! sh: 9    alts+=( "module-math-functions.${mod}:math function from zsh/${mod}:compadd -S '(' $funcs" )
//! sh:10  done
//! sh:11
//! sh:12  _alternative $alts
//! ```
//!
//! `zmodload -Fl zsh/$mod` lists features in module `zsh/$mod` —
//! `+f:NAME` entries are math functions. Parsing skipped (would
//! require executing zmodload); we emit empty `funcs` for each
//! module — `_alternative` still receives the four group specs but
//! with no candidates to add. Production zsh-side `_alternative`
//! handles empty groups gracefully.

use crate::ported::exec::dispatch_function_call;

const MODULES: &[&str] = &["example", "mathfunc", "system", "random"];

/// `_module_math_func` — complete math functions from
/// `zsh/{example,mathfunc,system,random}` modules.
pub fn _module_math_func() -> i32 {
    // sh:7-10
    let alts: Vec<String> = MODULES
        .iter()
        .map(|m| {
            format!(
                "module-math-functions.{}:math function from zsh/{}:compadd -S '('",
                m, m
            )
        })
        .collect();

    // sh:12
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
}
