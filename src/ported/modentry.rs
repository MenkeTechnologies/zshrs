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

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/modentry.c c:7 switch.
    // ═══════════════════════════════════════════════════════════════════

    /// c:7 — `modentry` repeated calls don't leak state between modules.
    /// Pin module-isolation: a sequence of modentry calls on module A
    /// must not poison subsequent modentry calls on module B.
    #[test]
    fn modentry_module_isolation() {
        let _g = crate::test_util::global_state_lock();
        let mut a = TestModule { booted: false };
        assert_eq!(modentry(1, &mut a), 0); // boot A
        assert!(a.booted);

        let mut b = TestModule { booted: false };
        assert_eq!(modentry(0, &mut b), 0); // setup B
        assert!(!b.booted, "B must NOT be booted via A's modentry");
    }

    /// c:14 — calling setup multiple times is safe (no side-effect lock).
    /// ModuleLifecycle::setup default returns 0.
    #[test]
    fn modentry_setup_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let mut m = TestModule { booted: false };
        for _ in 0..10 {
            assert_eq!(modentry(0, &mut m), 0);
        }
        assert!(!m.booted);
    }

    /// c:22 — calling cleanup multiple times is safe.
    #[test]
    fn modentry_cleanup_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let mut m = TestModule { booted: false };
        for _ in 0..10 {
            assert_eq!(modentry(2, &mut m), 0);
        }
    }

    /// c:7 — return value is always either 0 or 1, never anything else.
    #[test]
    fn modentry_return_value_in_0_or_1_for_all_inputs() {
        let _g = crate::test_util::global_state_lock();
        for op in -5..=15 {
            let mut m = TestModule { booted: false };
            let r = modentry(op, &mut m);
            assert!(r == 0 || r == 1, "modentry({}) = {} not in {{0,1}}", op, r);
        }
    }

    /// c:7 — the lifecycle sequence setup→boot→cleanup→finish all succeed
    /// in order without leaking unknown-op error.
    #[test]
    fn modentry_canonical_lifecycle_sequence_all_zero() {
        let _g = crate::test_util::global_state_lock();
        let mut m = TestModule { booted: false };
        assert_eq!(modentry(0, &mut m), 0); // setup
        assert_eq!(modentry(1, &mut m), 0); // boot
        assert_eq!(modentry(2, &mut m), 0); // cleanup
        assert_eq!(modentry(3, &mut m), 0); // finish
        assert!(m.booted, "boot side effect must survive cleanup/finish");
    }

    /// c:30/34 — features_/enables_ ops do NOT call any trait method
    /// (they short-circuit pre-dispatch per the c:30,34 case arms).
    /// Verified by ensuring boot side effect is NOT triggered.
    #[test]
    fn modentry_features_op_does_not_call_module_boot() {
        let _g = crate::test_util::global_state_lock();
        let mut m = TestModule { booted: false };
        assert_eq!(modentry(4, &mut m), 0);
        assert!(!m.booted, "features_ op MUST NOT side-effect boot");
        assert_eq!(modentry(5, &mut m), 0);
        assert!(!m.booted, "enables_ op MUST NOT side-effect boot");
    }

    /// c:38 — out-of-range op doesn't call any trait method.
    #[test]
    fn modentry_unknown_op_does_not_invoke_trait() {
        let _g = crate::test_util::global_state_lock();
        let mut m = TestModule { booted: false };
        for op in [99, 1000, -42] {
            assert_eq!(modentry(op, &mut m), 1);
            assert!(!m.booted, "op {} must NOT trigger boot", op);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/modentry.c
    // c:7 modentry — type/value pins beyond the existing 0-5 sweep
    // ═══════════════════════════════════════════════════════════════════

    /// c:7 — `modentry` returns i32 (compile-time type pin).
    #[test]
    fn modentry_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let mut m = TestModule { booted: false };
        let _: i32 = modentry(0, &mut m);
    }

    /// c:7 — `modentry` op=0 always returns 0 (success).
    #[test]
    fn modentry_setup_op_always_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            let mut m = TestModule { booted: false };
            assert_eq!(modentry(0, &mut m), 0, "op=0 (setup) → 0");
        }
    }

    /// c:7 — `modentry` op=3 (finish) always returns 0.
    #[test]
    fn modentry_finish_op_always_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            let mut m = TestModule { booted: false };
            assert_eq!(modentry(3, &mut m), 0, "op=3 (finish) → 0");
        }
    }

    /// c:7 — `modentry` op=4 (features) returns 0 + does NOT touch trait.
    #[test]
    fn modentry_features_op_returns_zero_no_side_effect() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            let mut m = TestModule { booted: false };
            assert_eq!(modentry(4, &mut m), 0);
            assert!(!m.booted, "features op (4) must NOT trigger boot");
        }
    }

    /// c:7 — `modentry` op=5 (enables) returns 0 + does NOT touch trait.
    #[test]
    fn modentry_enables_op_returns_zero_no_side_effect() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            let mut m = TestModule { booted: false };
            assert_eq!(modentry(5, &mut m), 0);
            assert!(!m.booted, "enables op (5) must NOT trigger boot");
        }
    }

    /// c:7 — `modentry` boot side-effect persists across subsequent calls.
    #[test]
    fn modentry_boot_side_effect_persists() {
        let _g = crate::test_util::global_state_lock();
        let mut m = TestModule { booted: false };
        modentry(1, &mut m); // boot
        assert!(m.booted, "boot triggered");
        modentry(2, &mut m); // cleanup
        assert!(m.booted, "boot side-effect persists past cleanup");
        modentry(3, &mut m); // finish
        assert!(m.booted, "boot side-effect persists past finish");
    }

    /// c:7 — `modentry` does NOT decrement m.booted on subsequent boot calls.
    /// (TestModule sets booted=true unconditionally; bool stays true.)
    #[test]
    fn modentry_boot_sticky_true_across_calls() {
        let _g = crate::test_util::global_state_lock();
        let mut m = TestModule { booted: false };
        for _ in 0..5 {
            modentry(1, &mut m);
        }
        assert!(m.booted, "booted stays true across repeated boot calls");
    }

    /// c:38 — `modentry(MAX-1)` is in unknown-op range (returns 1).
    #[test]
    fn modentry_max_minus_one_unknown_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let mut m = TestModule { booted: false };
        assert_eq!(modentry(i32::MAX - 1, &mut m), 1,
            "MAX-1 op → 1 (unknown)");
    }

    /// c:7 — `modentry` for op=5 returns same result as op=4 (both feature-side).
    #[test]
    fn modentry_features_enables_both_zero() {
        let _g = crate::test_util::global_state_lock();
        for op in [4, 5] {
            let mut m = TestModule { booted: false };
            assert_eq!(modentry(op, &mut m), 0,
                "feature-side op {} returns 0", op);
        }
    }

    /// c:7 — `modentry` for op=2 (cleanup) without prior boot is safe.
    #[test]
    fn modentry_cleanup_without_prior_boot_safe() {
        let _g = crate::test_util::global_state_lock();
        let mut m = TestModule { booted: false };
        assert_eq!(modentry(2, &mut m), 0, "cleanup without boot → 0");
        assert!(!m.booted, "boot side-effect not triggered by cleanup");
    }
}
