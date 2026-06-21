use crate::flog::flog;
use std::cell::{Ref, RefMut};
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::MutexGuard;
/// `RelaxedAtomicBool` — see fields for layout.
#[derive(Debug, Default)]
pub struct RelaxedAtomicBool(AtomicBool);

impl RelaxedAtomicBool {
    /// `new` — see implementation.
    pub const fn new(value: bool) -> Self {
        Self(AtomicBool::new(value))
    }
    /// `load` — see implementation.
    pub fn load(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
    /// `store` — see implementation.
    pub fn store(&self, value: bool) {
        self.0.store(value, Ordering::Relaxed);
    }
    /// `swap` — see implementation.
    pub fn swap(&self, value: bool) -> bool {
        self.0.swap(value, Ordering::Relaxed)
    }
}

impl Clone for RelaxedAtomicBool {
    fn clone(&self) -> Self {
        Self(AtomicBool::new(self.load()))
    }
}

/// An atomic reference type, allowing &'static values to be stored.
/// This uses relaxed ordering - it's intended for string literals.
/// Note that because string literals are fat pointers, we can't store one
/// directly in an AtomicPtr, so we store a pointer to the string literal instead!
pub struct AtomicRef<T: ?Sized + 'static>(AtomicPtr<&'static T>);

impl<T: ?Sized> AtomicRef<T> {
    /// `new` — see implementation.
    pub const fn new(value: &'static &'static T) -> Self {
        Self(AtomicPtr::new(std::ptr::from_ref(value).cast_mut()))
    }
    /// `load` — see implementation.
    pub fn load(&self) -> &'static T {
        unsafe { *self.0.load(Ordering::Relaxed) }
    }
    /// `store` — see implementation.
    pub fn store(&self, value: &'static &'static T) {
        self.0
            .store(std::ptr::from_ref(value).cast_mut(), Ordering::Relaxed);
    }
}
/// `DebugRef` — see fields for layout.
pub struct DebugRef<'a, T>(Ref<'a, T>);

impl<'a, T> DebugRef<'a, T> {
    /// `new` — see implementation.
    pub fn new(r: Ref<'a, T>) -> Self {
        flog!(
            refcell,
            "CREATE DebugRef",
            std::backtrace::Backtrace::capture()
        );
        Self(r)
    }
}

impl<'a, T> Drop for DebugRef<'a, T> {
    fn drop(&mut self) {
        flog!(
            refcell,
            "DROP DebugRef",
            std::backtrace::Backtrace::capture()
        );
    }
}

impl<'a, T> std::ops::Deref for DebugRef<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
/// `DebugRefMut` — see fields for layout.
pub struct DebugRefMut<'a, T>(RefMut<'a, T>);

impl<'a, T> DebugRefMut<'a, T> {
    /// `new` — see implementation.
    pub fn new(r: RefMut<'a, T>) -> Self {
        flog!(
            refcell,
            "CREATE DebugRefMut",
            std::backtrace::Backtrace::capture()
        );
        Self(r)
    }
}

impl<'a, T> Drop for DebugRefMut<'a, T> {
    fn drop(&mut self) {
        flog!(
            refcell,
            "DROP DebugRefMut",
            std::backtrace::Backtrace::capture()
        );
    }
}
impl<'a, T> std::ops::Deref for DebugRefMut<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<'a, T> std::ops::DerefMut for DebugRefMut<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
/// `DebugMutexGuard` — see fields for layout.
pub struct DebugMutexGuard<'a, T>(MutexGuard<'a, T>);

impl<'a, T> DebugMutexGuard<'a, T> {
    /// `new` — see implementation.
    pub fn new(r: MutexGuard<'a, T>) -> Self {
        flog!(
            refcell,
            "CREATE DebugMutexGuard",
            std::backtrace::Backtrace::capture()
        );
        Self(r)
    }
}

impl<'a, T> Drop for DebugMutexGuard<'a, T> {
    fn drop(&mut self) {
        flog!(
            refcell,
            "DROP DebugMutexGuard",
            std::backtrace::Backtrace::capture()
        );
    }
}

impl<'a, T> std::ops::Deref for DebugMutexGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl<'a, T> std::ops::DerefMut for DebugMutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
