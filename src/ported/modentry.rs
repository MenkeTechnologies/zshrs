//! Module entry point dispatch
//!
//! Port from zsh/Src/modentry.c (43 lines).
//!
//! In C, this is the dlopen entry point that dispatches setup/boot/cleanup/finish
//! calls to loaded modules. In Rust, all modules are statically compiled,
//! so this provides the ModuleLifecycle trait dispatch instead.
//!
//! C signature: `int modentry(int boot, Module m, void *ptr)`. The `boot`
//! int is the op selector — `0=setup_`, `1=boot_`, `2=cleanup_`,
//! `3=finish_`, `4=features_`, `5=enables_` per Src/modentry.c switch.

use crate::module::ModuleLifecycle;

/// Port of `modentry(int boot, Module m, void *ptr)` from Src/modentry.c:7. Direct port of the
/// C `int modentry(int boot, Module m, void *ptr)` switch — `boot`
/// selects which lifecycle function to dispatch. Unknown values
/// return 1 (matches C's `zerr("bad call to modentry"); return 1`).
/// WARNING: param names don't match C — Rust=(boot, module) vs C=(boot, m, ptr)
pub fn modentry(boot: i32, module: &mut dyn ModuleLifecycle) -> i32 {       // c:7
    match boot {
        0 => module.setup(),                                                 // c:14
        1 => module.boot(),                                                  // c:18
        2 => module.cleanup(),                                                // c:22
        3 => module.finish(),                                                 // c:26
        4 | 5 => 0,                                                           // c:30,34 features_/enables_
        _ => 1,                                                               // c:38-40 zerr default
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestModule {
        booted: bool,
    }
    impl ModuleLifecycle for TestModule {
        fn boot(&mut self) -> i32 {
            self.booted = true;
            0
        }
    }

    #[test]
    fn modentry_dispatches_setup_and_boot() {
        let mut m = TestModule { booted: false };
        assert_eq!(modentry(0, &mut m), 0);     // c:14 setup_
        assert!(!m.booted);
        assert_eq!(modentry(1, &mut m), 0);     // c:18 boot_
        assert!(m.booted);
    }

    #[test]
    fn modentry_unknown_op_returns_one() {
        let mut m = TestModule { booted: false };
        assert_eq!(modentry(6, &mut m), 1);     // c:38 default
        assert_eq!(modentry(-1, &mut m), 1);
    }

    /// c:22 — `cleanup_` op dispatches to module.cleanup(). Default
    /// ModuleLifecycle::cleanup returns 0 unless overridden.
    #[test]
    fn modentry_cleanup_op_returns_zero() {
        let mut m = TestModule { booted: false };
        assert_eq!(modentry(2, &mut m), 0);
    }

    /// c:26 — `finish_` op dispatches to module.finish().
    #[test]
    fn modentry_finish_op_returns_zero() {
        let mut m = TestModule { booted: false };
        assert_eq!(modentry(3, &mut m), 0);
    }

    /// c:30/34 — `features_` / `enables_` ops short-circuit to 0
    /// without invoking the module trait (the module-feature ledger
    /// is canonicalised in modulestab, not in per-module hooks).
    #[test]
    fn modentry_features_enables_short_circuit_zero() {
        let mut m = TestModule { booted: false };
        assert_eq!(modentry(4, &mut m), 0);     // c:30 features_
        assert_eq!(modentry(5, &mut m), 0);     // c:34 enables_
        // Confirm boot() wasn't invoked as side effect.
        assert!(!m.booted);
    }
}
