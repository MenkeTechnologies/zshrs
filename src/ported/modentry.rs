//! Module entry point dispatch
//!
//! Port from zsh/Src/modentry.c (43 lines)
//!
//! In C, this is the dlopen entry point that dispatches setup/boot/cleanup/finish
//! calls to loaded modules. In Rust, all modules are statically compiled,
//! so this provides the ModuleLifecycle trait dispatch instead.

use crate::module::ModuleLifecycle;

/// Module entry operations.
/// Port of the integer `boot` parameter to `modentry()` from
/// Src/modentry.c — the C source dispatches `0`/`1`/`2`/`3`/`4`/`5`
/// to `setup_`/`boot_`/`cleanup_`/`finish_`/`features_`/`enables_`.
/// We give each integer a typed name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModOp {
    Setup = 0,
    Boot = 1,
    Cleanup = 2,
    Finish = 3,
    Features = 4,
    Enables = 5,
}

impl ModOp {
    pub fn from_int(n: i32) -> Option<Self> {
        match n {
            0 => Some(ModOp::Setup),
            1 => Some(ModOp::Boot),
            2 => Some(ModOp::Cleanup),
            3 => Some(ModOp::Finish),
            4 => Some(ModOp::Features),
            5 => Some(ModOp::Enables),
            _ => None,
        }
    }
}

/// Dispatch a module lifecycle operation.
/// Port of `modentry()` from Src/modentry.c — the C source's switch
/// on the `boot` integer that the dlopened module's entry point
/// uses to route into `setup_` / `boot_` / `cleanup_` / `finish_` /
/// `features_` / `enables_`. Default branch in the C source emits
/// `zerr("bad call to modentry")`; we ignore unknown ops as a
/// no-op since `ModOp` is exhaustive at the type level.
pub fn modentry(op: ModOp, module: &mut dyn ModuleLifecycle) -> i32 {
    match op {
        ModOp::Setup => module.setup(),
        ModOp::Boot => module.boot(),
        ModOp::Cleanup => module.cleanup(),
        ModOp::Finish => module.finish(),
        ModOp::Features | ModOp::Enables => 0,
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
    fn test_modentry_dispatch() {
        let mut m = TestModule { booted: false };
        assert_eq!(modentry(ModOp::Setup, &mut m), 0);
        assert!(!m.booted);
        assert_eq!(modentry(ModOp::Boot, &mut m), 0);
        assert!(m.booted);
    }

    #[test]
    fn test_modop_from_int() {
        assert_eq!(ModOp::from_int(0), Some(ModOp::Setup));
        assert_eq!(ModOp::from_int(1), Some(ModOp::Boot));
        assert_eq!(ModOp::from_int(6), None);
    }
}
