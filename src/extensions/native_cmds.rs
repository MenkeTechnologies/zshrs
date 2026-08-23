//! Host-registered native commands — builtins contributed by the *binary*
//! rather than by this library.
//!
//! `EXT_BUILTIN_NAMES` (`extensions/ext_builtins.rs`) and the daemon's
//! `ZSHRS_BUILTIN_NAMES` are both compile-time lists owned by this crate. A
//! fat binary that links sibling runtimes into the shell's address space —
//! `zshrs-native` links zvcs (`git`), arblang (`arb`) and strykelang
//! (`stryke`) — has no way to extend either: they are `const` arrays, and the
//! runtimes cannot be dependencies of this crate (zvcs depends on its own
//! vendored gitoxide by path, which makes any dependent unpublishable).
//!
//! So the binary registers them here, once, before the shell starts. A
//! registered name dispatches in-process on a direct function call: no fork,
//! no execve, no `PATH` walk, no dynamic loader — the same treatment `cat` and
//! `sort` already get from `reg_overridable!`.
//!
//! # Dispatch order
//!
//! Registration does not jump the queue. zsh resolves a command word as
//! alias → function → builtin → external (c:Src/exec.c:3038-3068), and a
//! native command sits in the *builtin* slot, after the ported builtin table:
//!
//! * a user `git() { … }` still wins, exactly as it shadows `cat` today;
//! * `command git` still reaches whatever `git` is on `PATH`, because the
//!   forced-external path never consults this registry;
//! * `builtin git` reaches the native one.
//!
//! # Registration is one-shot and start-up only
//!
//! The table is written once by the binary's `main` before the shell runs and
//! is read from every command dispatch after that, including from the worker
//! threads. It is therefore an `RwLock` whose write side is expected to be
//! uncontended: registering after startup is allowed but pointless, and no
//! path ever removes an entry — a name that answered `whence -w` one moment
//! must not vanish the next.

use std::collections::BTreeMap;
use std::sync::{OnceLock, RwLock};

/// A native command body: the full argv (argv[0] is the command name, as
/// invoked) in, a wait-status-style exit code out.
///
/// Taking argv[0] rather than only the operands is what lets a runtime keep
/// its own `argv[0]`-dependent behaviour — zvcs dispatches `git-<verb>` off
/// its own name (`dashed_subcommand`), and its diagnostics are prefixed with
/// it.
pub type NativeCmd = Box<dyn Fn(&[String]) -> i32 + Send + Sync>;

fn table() -> &'static RwLock<BTreeMap<String, NativeCmd>> {
    static TABLE: OnceLock<RwLock<BTreeMap<String, NativeCmd>>> = OnceLock::new();
    TABLE.get_or_init(|| RwLock::new(BTreeMap::new()))
}

/// Register `name` as a native command backed by `f`.
///
/// Idempotent per name in the sense that the last registration wins; the
/// binary calls this once per runtime from `main`, before the shell reads a
/// line. A poisoned lock is ignored rather than panicking — losing a builtin
/// registration must not take the shell down at startup.
pub fn register<F>(name: &str, f: F)
where
    F: Fn(&[String]) -> i32 + Send + Sync + 'static,
{
    if let Ok(mut t) = table().write() {
        t.insert(name.to_string(), Box::new(f));
    }
}

/// Is `name` a host-registered native command?
///
/// The hot path: consulted on every command word that is neither a function
/// nor a ported builtin, so it must not allocate. A read lock on a `BTreeMap`
/// of a handful of short keys is a few compares.
pub fn is_registered(name: &str) -> bool {
    table().read().map(|t| t.contains_key(name)).unwrap_or(false)
}

/// Registered *and* not masked by `disable NAME`.
///
/// The gate the two dispatch sites use. `disable` is zsh's way of taking a
/// builtin out of the way without unsetting anything (c:Src/builtin.c:541-547
/// toggles `DISABLED` on the node; this port tracks the same set in
/// `BUILTINS_DISABLED`), and it has to work on a native command for the same
/// reason it works on the `cat` shadow: it is the one escape hatch that is
/// per-shell, reversible with `enable`, and needs no change at the call site.
/// A disabled name falls through to the `PATH` binary.
pub fn is_enabled(name: &str) -> bool {
    is_registered(name)
        && !crate::ported::builtin::BUILTINS_DISABLED
            .lock()
            .map(|s| s.contains(name))
            .unwrap_or(false)
}

/// Every registered name, sorted. Feeds the `builtins` magic assoc
/// (`${(k)builtins}`), `whence -m`, and compsys's command-position
/// completion, so the shell reports the same set it will actually dispatch.
pub fn names() -> Vec<String> {
    table()
        .read()
        .map(|t| t.keys().cloned().collect())
        .unwrap_or_default()
}

/// Run `name` with `argv` (argv[0] included) if it is registered.
///
/// Returns `None` when the name is not ours, so callers fall through to their
/// existing next step — `PATH` lookup, or "command not found".
///
/// The read lock is held for the duration of the call. That is deliberate:
/// nothing ever removes an entry, and the alternative (clone the boxed closure
/// out) is not possible for a `dyn Fn`. Re-entrant dispatch — a native command
/// that runs shell code that runs another native command — takes the read lock
/// twice, which an `RwLock` grants.
pub fn dispatch(name: &str, argv: &[String]) -> Option<i32> {
    let t = table().read().ok()?;
    let f = t.get(name)?;
    Some(f(argv))
}

/// Test-only removal.
///
/// Production never removes an entry — a name that answered `whence -w` one
/// moment must not vanish the next — but the table is process-global and the
/// `${(k)builtins}` scan reads it, so a test that registers a probe name has
/// to take it back out or the next test in the same process sees a builtin
/// nobody registered.
#[cfg(test)]
pub(crate) fn unregister(name: &str) {
    if let Ok(mut t) = table().write() {
        t.remove(name);
    }
}

thread_local! {
    /// Set while a `command NAME …` precommand is dispatching NAME.
    ///
    /// `command` means "not the function, not the builtin — the thing on
    /// `PATH`" (c:Src/exec.c:3275-3278, the `BINF_COMMAND && !POSIXBUILTINS`
    /// arm that clears the builtin node). zshrs already honours that for its
    /// own in-process shadows: `PATH= command cat` reports `command not
    /// found: cat` rather than running the built-in `cat`.
    ///
    /// A native command has to answer the same way, and the site that catches
    /// it — `execute_external_bg` — is the very site `command` dispatches
    /// through, so the two are indistinguishable without this flag. The
    /// `command` handler raises it for the duration of that one call.
    ///
    /// Thread-local because it describes one invocation in flight, and
    /// commands run on worker threads.
    static FORCED_EXTERNAL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Restores the previous `command`-prefix state on drop, so a nested
/// dispatch (a `command` inside a function a native command ran) unwinds
/// correctly instead of leaving the flag stuck on.
pub struct ForcedExternalGuard(bool);

impl Drop for ForcedExternalGuard {
    fn drop(&mut self) {
        FORCED_EXTERNAL.with(|f| f.set(self.0));
    }
}

/// Mark the current invocation as `command`-forced for as long as the
/// returned guard lives.
#[must_use]
pub fn force_external() -> ForcedExternalGuard {
    ForcedExternalGuard(FORCED_EXTERNAL.with(|f| f.replace(true)))
}

/// True while a `command NAME` precommand is dispatching NAME, i.e. while the
/// user has explicitly asked for the `PATH` binary rather than this table.
pub fn is_forced_external() -> bool {
    FORCED_EXTERNAL.with(std::cell::Cell::get)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The registry answers for a name it was given, and only for that name.
    /// Names are test-unique because the table is process-global and the test
    /// binary runs many tests in one process.
    #[test]
    fn registered_name_dispatches_with_full_argv() {
        static SEEN: AtomicUsize = AtomicUsize::new(0);
        register("zshrs_test_native_dispatch", |argv| {
            SEEN.store(argv.len(), Ordering::SeqCst);
            assert_eq!(argv[0], "zshrs_test_native_dispatch");
            7
        });

        assert!(is_registered("zshrs_test_native_dispatch"));
        assert!(!is_registered("zshrs_test_native_never_registered"));

        let argv = [
            "zshrs_test_native_dispatch".to_string(),
            "--flag".to_string(),
        ];
        assert_eq!(dispatch("zshrs_test_native_dispatch", &argv), Some(7));
        assert_eq!(SEEN.load(Ordering::SeqCst), 2);
        // An unregistered name must fall through rather than answer.
        assert_eq!(dispatch("zshrs_test_native_never_registered", &argv), None);

        unregister("zshrs_test_native_dispatch");
        assert!(!is_registered("zshrs_test_native_dispatch"));
    }

    /// `disable NAME` masks a native command without unregistering it, and
    /// `enable NAME` takes it back — the shell falls through to `PATH` in
    /// between. Exercised through the same `BUILTINS_DISABLED` set that
    /// `bin_enable` writes.
    #[test]
    fn disable_masks_dispatch_and_enable_restores() {
        register("zshrs_test_native_disable", |_| 0);
        assert!(is_enabled("zshrs_test_native_disable"));

        crate::ported::builtin::BUILTINS_DISABLED
            .lock()
            .unwrap()
            .insert("zshrs_test_native_disable".to_string());
        assert!(is_registered("zshrs_test_native_disable"));
        assert!(!is_enabled("zshrs_test_native_disable"));

        crate::ported::builtin::BUILTINS_DISABLED
            .lock()
            .unwrap()
            .remove("zshrs_test_native_disable");
        assert!(is_enabled("zshrs_test_native_disable"));

        unregister("zshrs_test_native_disable");
    }

    /// The `command NAME` guard is scoped and nests: an inner scope restores
    /// the outer state on drop rather than clearing the flag outright.
    #[test]
    fn forced_external_guard_restores_previous_state() {
        assert!(!is_forced_external());
        {
            let _outer = force_external();
            assert!(is_forced_external());
            {
                let _inner = force_external();
                assert!(is_forced_external());
            }
            // Still inside the outer `command` dispatch.
            assert!(is_forced_external());
        }
        assert!(!is_forced_external());
    }
}
