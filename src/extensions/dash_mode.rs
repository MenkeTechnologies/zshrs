//! Strict-dash (Debian Almquist Shell) emulation flag — a zshrs-only
//! extension with NO zsh C counterpart.
//!
//! Upstream zsh has no `dash` personality: its option system models `sh`
//! only as a set of behavior deltas and can never *reject* a zsh syntactic
//! extension the way real dash does. dash is behaviourally `sh` for every
//! option (`shwordsplit`, `ksharrays`, `posix*`, …) and only ADDS
//! rejections of zsh-only syntax. So rather than a distinct `EMULATE_DASH`
//! bit — which would force `|| EMULATION(EMULATE_DASH)` at every one of the
//! ~25 `EMULATION(EMULATE_SH)` call sites — `emulate dash` / `zshrs --dash`
//! sets `EMULATION = EMULATE_SH` and raises this orthogonal flag.
//!
//! The lexer / parser / math / echo gates keyed off [`dash_strict`] turn
//! the following zsh extensions into the same errors real `/bin/dash`
//! produces:
//!   * `$'...'` ANSI-C quoting  → literal `$` + ordinary single quote
//!   * `<<<` here-strings       → "redirection unexpected"
//!   * `+=` compound assignment → command word ("not found")
//!   * `name=(...)` arrays       → "( unexpected" syntax error
//!   * the `[[ ]]` reserved word → ordinary command ("not found")
//!   * arith `**` / `,`          → arithmetic parse error
//!   * non-XSI `echo`            → escapes interpreted by default
//!
//! This lives in `src/extensions/` (not `src/ported/`) because it has no
//! line in zsh's C source; `src/ported/` is a faithful port only.

use std::sync::atomic::{AtomicBool, Ordering};

/// Process-global strict-dash flag. Raised by `emulate dash` (via
/// [`set_dash_strict`]) and cleared by any `emulate` to another
/// personality. Read through [`dash_strict`] at each hot-path gate.
static DASH_STRICT: AtomicBool = AtomicBool::new(false);

/// Process-global "real-shell-faithful" flag for the POSIX-family drop-in
/// modes (`zshrs --sh` / `--ksh` / `--dash`).
///
/// zsh's own `emulate sh`/`emulate ksh` only approximates the Bourne
/// shells and keeps several zsh-family behaviors that the real tools do
/// not — e.g. a trailing non-whitespace IFS separator yields a trailing
/// empty field in zsh (`IFS=:; set -- $v` on `a:b:` → 3 args) but not in
/// dash/ksh/bash (→ 2 args). When raised, zshrs matches the REAL shell
/// instead of zsh's approximation, making `zshrs --sh` strictly more
/// faithful than zsh.
///
/// Design: the bare drop-in flag (`--sh`/`--ksh`/`--dash`) raises this;
/// adding `--zsh` (`zshrs --sh --zsh`) clears it, selecting zsh-style
/// emulation instead. The runtime `emulate sh` builtin never raises it —
/// that path is zsh's feature and keeps zsh semantics. Set only from the
/// binary's CLI mode-application, so it defaults `false` in the library.
static POSIX_FAITHFUL: AtomicBool = AtomicBool::new(false);

/// Process-global bash drop-in flag (`zshrs --bash`). bash is a SUPERSET of
/// POSIX sh with syntax zsh lacks: indirect `${!var}` and case-modification
/// `${v^^}` / `${v,,}` / `${v^}` / `${v,}`. These are parsed in the subst
/// layer only when this is set, so native zsh and the other modes are
/// unaffected. Set from the binary's CLI mode-application; defaults `false`.
static BASH_MODE: AtomicBool = AtomicBool::new(false);

/// True when the shell is running in strict-dash mode (`emulate dash` or
/// `zshrs --dash`). Gates the zsh-extension rejections that make zshrs
/// match `/bin/dash` byte-for-byte.
#[inline]
pub fn dash_strict() -> bool {
    DASH_STRICT.load(Ordering::Relaxed)
}

/// Set (or clear) strict-dash mode. Called from `options::emulate` — set
/// for the `dash` personality, cleared for every other so a later
/// `emulate zsh` (etc.) fully leaves dash mode.
#[inline]
pub fn set_dash_strict(on: bool) {
    DASH_STRICT.store(on, Ordering::Relaxed);
}

/// True when a POSIX-family drop-in mode should match the REAL shell
/// rather than zsh's approximation of it. See [`POSIX_FAITHFUL`].
#[inline]
pub fn posix_faithful() -> bool {
    POSIX_FAITHFUL.load(Ordering::Relaxed)
}

/// True in bash drop-in mode (`zshrs --bash`). Enables bash-only param
/// expansion syntax (`${!var}` indirect, `${v^^}` case-mod). See [`BASH_MODE`].
#[inline]
pub fn bash_mode() -> bool {
    BASH_MODE.load(Ordering::Relaxed)
}

/// Set (or clear) bash drop-in mode. Called from the binary's CLI mode
/// application (raised for `--bash`, unless `--zsh` overrides).
#[inline]
pub fn set_bash_mode(on: bool) {
    BASH_MODE.store(on, Ordering::Relaxed);
}

/// Set (or clear) real-shell-faithful mode. Called from the binary's CLI
/// mode application: raised for a bare `--sh`/`--ksh`/`--dash`, cleared
/// when `--zsh` is also present (zsh-style emulation) or in any other mode.
#[inline]
pub fn set_posix_faithful(on: bool) {
    POSIX_FAITHFUL.store(on, Ordering::Relaxed);
}

/// The bash version zshrs advertises in `--bash` mode. Scripts gate features
/// on `${BASH_VERSINFO[0]}` (e.g. `>= 4` for assoc arrays / `${v^^}`), so a
/// modern 5.x keeps every feature path live. Not tied to any real build.
pub const BASH_VERSION_MAJOR: &str = "5";
pub const BASH_VERSION_MINOR: &str = "2";
pub const BASH_VERSION_PATCH: &str = "0";

/// `$BASH_VERSION` scalar, e.g. `5.2.0(1)-release`.
pub fn bash_version() -> String {
    format!(
        "{}.{}.{}(1)-release",
        BASH_VERSION_MAJOR, BASH_VERSION_MINOR, BASH_VERSION_PATCH
    )
}

/// `${BASH_VERSINFO[@]}` — the 6-element bash version array:
/// `(major minor patch build release machtype)`.
pub fn bash_versinfo() -> Vec<String> {
    vec![
        BASH_VERSION_MAJOR.to_string(),
        BASH_VERSION_MINOR.to_string(),
        BASH_VERSION_PATCH.to_string(),
        "1".to_string(),
        "release".to_string(),
        std::env::consts::ARCH.to_string(),
    ]
}

/// Resolve a bash special ARRAY name (`PIPESTATUS`, `FUNCNAME`,
/// `BASH_VERSINFO`) to its value in `--bash` mode by aliasing the zsh-native
/// special or synthesizing it. Returns `None` for any other name (or outside
/// bash mode) so callers fall through to normal array resolution.
pub fn bash_special_array(name: &str) -> Option<Vec<String>> {
    if !bash_mode() {
        return None;
    }
    match name {
        // bash PIPESTATUS ≈ zsh pipestatus (per-stage exit codes, 0-indexed).
        "PIPESTATUS" => Some(crate::ported::exec::array("pipestatus").unwrap_or_default()),
        // bash FUNCNAME ≈ zsh funcstack — call stack, innermost (current) first.
        "FUNCNAME" => crate::ported::modules::parameter::FUNCSTACK
            .lock()
            .ok()
            .map(|f| f.iter().rev().map(|fs| fs.name.clone()).collect()),
        "BASH_VERSINFO" => Some(bash_versinfo()),
        _ => None,
    }
}
