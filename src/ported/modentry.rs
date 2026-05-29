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
pub fn modentry(boot: i32, module: &mut dyn ModuleLifecycle) -> i32 {
    // c:7
    match boot {
        0 => module.setup(),   // c:14
        1 => module.boot(),    // c:18
        2 => module.cleanup(), // c:22
        3 => module.finish(),  // c:26
        4 | 5 => 0,            // c:30,34 features_/enables_
        _ => 1,                // c:38-40 zerr default
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
        let _g = crate::test_util::global_state_lock();
        let mut m = TestModule { booted: false };
        assert_eq!(modentry(0, &mut m), 0); // c:14 setup_
        assert!(!m.booted);
        assert_eq!(modentry(1, &mut m), 0); // c:18 boot_
        assert!(m.booted);
    }

    #[test]
    fn modentry_unknown_op_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let mut m = TestModule { booted: false };
        assert_eq!(modentry(6, &mut m), 1); // c:38 default
        assert_eq!(modentry(-1, &mut m), 1);
    }

    /// c:22 — `cleanup_` op dispatches to module.cleanup(). Default
    /// ModuleLifecycle::cleanup returns 0 unless overridden.
    #[test]
    fn modentry_cleanup_op_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let mut m = TestModule { booted: false };
        assert_eq!(modentry(2, &mut m), 0);
    }

    /// c:26 — `finish_` op dispatches to module.finish().
    #[test]
    fn modentry_finish_op_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let mut m = TestModule { booted: false };
        assert_eq!(modentry(3, &mut m), 0);
    }

    /// c:30/34 — `features_` / `enables_` ops short-circuit to 0
    /// without invoking the module trait (the module-feature ledger
    /// is canonicalised in modulestab, not in per-module hooks).
    #[test]
    fn modentry_features_enables_short_circuit_zero() {
        let _g = crate::test_util::global_state_lock();
        let mut m = TestModule { booted: false };
        assert_eq!(modentry(4, &mut m), 0); // c:30 features_
        assert_eq!(modentry(5, &mut m), 0); // c:34 enables_
                                            // Confirm boot() wasn't invoked as side effect.
        assert!(!m.booted);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/modentry.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:38 — `modentry` rejects MANY out-of-range op codes with 1.
    #[test]
    fn modentry_far_out_of_range_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let mut m = TestModule { booted: false };
        for op in [-100, -50, 7, 10, 100, i32::MAX, i32::MIN] {
            assert_eq!(modentry(op, &mut m), 1, "op {} must return 1", op);
        }
    }

    /// c:14 — `setup_` op (boot=0) dispatches without invoking boot().
    /// Pin orthogonality: setup ≠ boot.
    #[test]
    fn modentry_setup_does_not_invoke_boot() {
        let _g = crate::test_util::global_state_lock();
        let mut m = TestModule { booted: false };
        assert_eq!(modentry(0, &mut m), 0);
        assert!(!m.booted, "setup must NOT trigger boot side effect");
    }

    /// c:18 — `boot_` op (boot=1) sets booted=true on our TestModule.
    /// Pin: side effect is observable.
    #[test]
    fn modentry_boot_triggers_module_boot() {
        let _g = crate::test_util::global_state_lock();
        let mut m = TestModule { booted: false };
        assert_eq!(modentry(1, &mut m), 0);
        assert!(m.booted, "boot op must invoke module.boot()");
    }

    /// c:7 — `modentry` is deterministic for known ops on a stateless
    /// fresh module instance.
    #[test]
    fn modentry_is_deterministic_for_known_ops() {
        let _g = crate::test_util::global_state_lock();
        for op in [0, 2, 3, 4, 5, 6, -1] {
            let mut m = TestModule { booted: false };
            let first = modentry(op, &mut m);
            // Second call on fresh module should give same return.
            let mut m2 = TestModule { booted: false };
            let second = modentry(op, &mut m2);
            assert_eq!(first, second, "op {} must be pure", op);
        }
    }

    /// c:7 — every defined op (0..=5) returns 0 (no error).
    #[test]
    fn modentry_known_ops_return_zero() {
        let _g = crate::test_util::global_state_lock();
        for op in 0..=5 {
            let mut m = TestModule { booted: false };
            assert_eq!(modentry(op, &mut m), 0, "op {} must succeed", op);
        }
    }

    /// c:38-40 — unknown op returns 1 for boundary values around 5/6.
    #[test]
    fn modentry_boundary_op_6_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let mut m = TestModule { booted: false };
        assert_eq!(modentry(6, &mut m), 1, "op=6 just past valid range → 1");
    }
}
