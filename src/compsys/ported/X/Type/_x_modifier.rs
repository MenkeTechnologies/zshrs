//! Port of `_x_modifier` from `Completion/X/Type/_x_modifier`.
//!
//! Full upstream body (8 lines verbatim):
//! ```text
//! sh:1  #autoload
//! sh:3  local expl
//! sh:5  _wanted modifiers expl modifier \
//! sh:6      compadd "$@" -M 'm:{a-z}={A-Z}' - \
//! sh:7              Shift Lock Control Mod1 Mod2 Mod3 Mod4 Mod5
//! ```

use crate::compsys::ported::_wanted::wanted_byname;

/// sh:7 — the fixed list of X keyboard modifier names.
const MODIFIERS: &[&str] = &[
    "Shift", "Lock", "Control", "Mod1", "Mod2", "Mod3", "Mod4", "Mod5",
];

/// `_x_modifier` — complete X keyboard modifier names (`Shift`, `Lock`,
/// `Control`, `Mod1`..`Mod5`), matched case-insensitively (`m:{a-z}={A-Z}`).
pub fn _x_modifier(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_x_modifier");
    // sh:5-7  _wanted modifiers expl modifier \
    //             compadd "$@" -M 'm:{a-z}={A-Z}' - Shift Lock Control Mod1 Mod2 Mod3 Mod4 Mod5
    let mut wanted_argv: Vec<String> = vec![
        "modifiers".to_string(),
        "expl".to_string(),
        "modifier".to_string(),
        "compadd".to_string(),
    ];
    wanted_argv.extend(args.iter().cloned());
    wanted_argv.push("-M".to_string());
    wanted_argv.push("m:{a-z}={A-Z}".to_string());
    wanted_argv.push("-".to_string());
    wanted_argv.extend(MODIFIERS.iter().map(|s| s.to_string()));
    wanted_byname(&wanted_argv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = _x_modifier(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 1);
    }

    #[test]
    fn passes_through_extra_args_and_full_modifier_list() {
        // sh:6 — "$@" is spliced between `compadd` and `-M`; the fixed
        //   modifier list follows `-` unconditionally.
        let mut wanted_argv: Vec<String> = vec![
            "modifiers".to_string(),
            "expl".to_string(),
            "modifier".to_string(),
            "compadd".to_string(),
        ];
        let extra = vec!["-V".to_string(), "grp".to_string()];
        wanted_argv.extend(extra.iter().cloned());
        wanted_argv.push("-M".to_string());
        wanted_argv.push("m:{a-z}={A-Z}".to_string());
        wanted_argv.push("-".to_string());
        wanted_argv.extend(MODIFIERS.iter().map(|s| s.to_string()));

        assert_eq!(
            wanted_argv,
            vec![
                "modifiers",
                "expl",
                "modifier",
                "compadd",
                "-V",
                "grp",
                "-M",
                "m:{a-z}={A-Z}",
                "-",
                "Shift",
                "Lock",
                "Control",
                "Mod1",
                "Mod2",
                "Mod3",
                "Mod4",
                "Mod5",
            ]
        );
    }

    #[test]
    fn modifier_list_matches_upstream_order() {
        assert_eq!(
            MODIFIERS,
            &["Shift", "Lock", "Control", "Mod1", "Mod2", "Mod3", "Mod4", "Mod5"]
        );
    }
}
