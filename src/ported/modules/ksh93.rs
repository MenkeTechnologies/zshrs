//! Ksh93 compatibility module — port of `Src/Modules/ksh93.c`.
//!
//! C source has zero `struct ...` / `enum ...` definitions. Rust
//! port matches: zero types. The `.sh.*` parameters are registered
//! via the C `partab[]` paramdef array (ksh93.c:200+); each entry
//! is a `struct paramdef` (defined in zsh.h, not a ksh93-private
//! type) wired to a getfn/setfn pair.
//!
//! Functions in C:
//!   - `edcharsetfn`   — setfn for `.sh.edchar` (c:47-58)
//!   - `matchgetfn`    — getfn for `.sh.match` (c:60-90)
//!   - `ksh93_wrapper` — per-function wrapper installed via
//!                       `addwrapper(m, wrapper)` (c:142-...)
//!   - `setup_/features_/enables_/boot_/cleanup_/finish_` — module
//!                       loader hooks.

/// Port of static helper `edcharsetfn()` from
/// `Src/Modules/ksh93.c:47`. The setfn callback for ksh93's
/// `.sh.edchar` param. C body is intentionally empty (just `;`)
/// — the comment at ksh93.c:48-55 explains a faithful ksh
/// emulation would need to intercept `$KEYS` before widget lookup
/// (similar to `bindkey -s`) and register `SIGKEYBD`. Upstream zsh
/// left it as a placeholder for future work.
///
/// C signature: `static void edcharsetfn(Param pm, char *x)`.
/// Rust port matches: takes the param ref + new value, does
/// nothing.
pub fn edcharsetfn(_pm_name: &str, _value: &str) {                       // c:47
    // Intentional no-op — ksh93.c:56 is just `;`.
}

/// Port of static helper `matchgetfn()` from
/// `Src/Modules/ksh93.c:60`. The getfn callback for ksh93's
/// `.sh.match` array. Reads zsh's `match` array via `getaparam`,
/// then under `KSHARRAYS` prepends `$MATCH` as element 0;
/// otherwise returns the array as-is.
///
/// C signature: `static char **matchgetfn(Param pm)`.
/// Rust port returns a fresh `Vec<String>` since the param-node
/// `u.arr` mutation in C lives in the param-table layer; this
/// entry produces the value the param machinery then assigns.
pub fn matchgetfn() -> Vec<String> {                                     // c:60
    // c:62 — `char **zsh_match = getaparam("match");`
    let zsh_match: Vec<String> = std::env::var("match")
        .ok()
        .map(|s| s.split(' ').map(|t| t.to_string()).collect())
        .unwrap_or_default();
    // c:71 — `if (isset(KSHARRAYS)) ...`
    let kshari = std::env::var("KSHARRAYS").map(|v| !v.is_empty()).unwrap_or(false);
    // c:67-68 — `if (pm->u.arr) freearray(pm->u.arr);` (param-table side)
    if zsh_match.is_empty() {                                            // c:80
        if kshari {
            // c:80 — `pm->u.arr = mkarray(ztrdup(getsparam("MATCH")));`
            return vec![std::env::var("MATCH").unwrap_or_default()];
        }
        return Vec::new();                                               // c:82 NULL
    }
    if kshari {                                                          // c:71
        // c:73-77 — prepend $MATCH as element 0.
        let mut out = Vec::with_capacity(zsh_match.len() + 1);
        out.push(std::env::var("MATCH").unwrap_or_default());            // c:75
        out.extend(zsh_match);                                           // c:76
        out
    } else {
        zsh_match                                                        // c:78 zarrdup
    }
}

/// Port of `ksh93_wrapper()` from `Src/Modules/ksh93.c:142`. The
/// per-function wrapper hook — installed by `boot_()` via
/// `addwrapper(m, wrapper)`. Runs at every shell-function call
/// when the module is loaded; sets up the `.sh.command` /
/// `.sh.edcol` / etc. local namerefs, then calls `runshfunc()`
/// to actually run the function body.
///
/// C signature: `static int ksh93_wrapper(Eprog prog, FuncWrap w,
///                                         char *name)`. Returns 1
/// when not in EMULATE_KSH so the wrapper chain falls through to
/// the next entry (c:148-149).
///
/// **Partial port — see TODO.md.** The full body needs:
///   - `EMULATION(EMULATE_KSH)` check (c:148)
///   - `getiparam(".sh.level")` + funcstack walk (c:146,151)
///   - `queue_signals()` / `unqueue_signals()` (c:156)
///   - `createparam(".sh.command", LOCAL_NAMEREF)` for each `.sh.*`
///     local nameref (c:159+)
///   - `runshfunc(prog, w, name)` to execute the wrapped function
///   - `++locallevel` / `--locallevel` book-keeping (c:157)
/// All of these depend on param-table + funcstack + locallevel
/// machinery that isn't yet wired through to a free-fn signature
/// like this. The Rust port returns 1 to mean "skip this wrapper"
/// — matching the EMULATE_KSH-not-set arm of the C body.
pub fn ksh93_wrapper(_prog: i32, _w: i32, _name: &str) -> i32 {          // c:142
    1                                                                    // c:149 not EMULATE_KSH
}

/// Port of `setup_()` from `Src/Modules/ksh93.c:236`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn setup_() -> i32 {                                                 // c:236
    0                                                                    // c:239
}

/// Port of `features_()` from `Src/Modules/ksh93.c:243`. C body is
/// `*features = featuresarray(m, &module_features); return 0;`.
/// Static-link path: 0.
pub fn features_() -> i32 {                                              // c:243
    0                                                                    // c:247
}

/// Port of `enables_()` from `Src/Modules/ksh93.c:251`. C body is
/// `return handlefeatures(m, &module_features, enables);`.
/// Static-link path: 0.
pub fn enables_() -> i32 {                                               // c:251
    0                                                                    // c:254
}

/// Port of `boot_()` from `Src/Modules/ksh93.c:258`. C body is
/// `return addwrapper(m, wrapper);` — registers the per-function
/// `ksh93_wrapper` callback. zshrs's function dispatch wires the
/// ksh93 wrapper directly through `crate::ported::exec` when
/// emulation is set; the loader hook is a no-op.
pub fn boot_() -> i32 {                                                  // c:258
    0                                                                    // c:262
}

/// Port of `cleanup_()` from `Src/Modules/ksh93.c:265`. C body
/// (lines 266-281):
///   1. `deletewrapper(m, wrapper)` — remove the function wrapper.
///   2. Walk the per-module paramdef table; for each `PM_NAMEREF`
///      param defined here, clear the NAMEREF flag on any live
///      paramtab node so `deleteparamdef()` doesn't see a
///      lingering nameref.
///   3. `setfeatureenables(m, &module_features, NULL)` — disable
///      every feature.
///
/// Static-link path: zshrs never unloads ksh93. 0 success.
pub fn cleanup_() -> i32 {                                               // c:265
    0                                                                    // c:281
}

/// Port of `finish_()` from `Src/Modules/ksh93.c:284`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn finish_() -> i32 {                                                // c:284
    0                                                                    // c:287
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ksh93_wrapper_returns_one_when_not_emulate_ksh() {
        // C c:148-149: `if (!EMULATION(EMULATE_KSH)) return 1;`
        // The Rust port currently always takes that branch (full
        // port pending param-table integration).
        assert_eq!(ksh93_wrapper(0, 0, "foo"), 1);
    }

    #[test]
    fn matchgetfn_empty_returns_empty() {
        // With $match unset, returns empty Vec (c:82 NULL branch).
        std::env::remove_var("match");
        std::env::remove_var("KSHARRAYS");
        let v = matchgetfn();
        assert!(v.is_empty());
    }

    #[test]
    fn module_loaders_return_zero() {
        assert_eq!(setup_(), 0);
        assert_eq!(features_(), 0);
        assert_eq!(enables_(), 0);
        assert_eq!(boot_(), 0);
        assert_eq!(cleanup_(), 0);
        assert_eq!(finish_(), 0);
    }
}
