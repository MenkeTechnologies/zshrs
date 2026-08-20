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

/// bash `shopt -s nocasematch` — case-insensitive `[[ == ]]` / `[[ =~ ]]` /
/// `case`. It is NOT a zsh option (opt_state can't store it), so it needs its
/// own flag. Toggled by the `shopt` builtin; read by cond.rs / case matching.
static NOCASEMATCH: AtomicBool = AtomicBool::new(false);

/// True when `shopt -s nocasematch` is active (bash case-insensitive matching).
#[inline]
pub fn nocasematch() -> bool {
    NOCASEMATCH.load(Ordering::Relaxed)
}

/// Set/clear bash `nocasematch`.
#[inline]
pub fn set_nocasematch(on: bool) {
    NOCASEMATCH.store(on, Ordering::Relaxed);
}

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

/// True when `printf` in bash mode should treat this operand to a numeric
/// conversion (`%d %i %o %u %x %X`) as an error (still prints 0, but exit
/// status 1). bash — unlike zsh/ksh/dash/sh — errors on an explicitly-supplied
/// EMPTY operand (`printf '%d' ''` → rc 1); a MISSING operand (`printf '%d'`)
/// is NOT an error, so `arg` distinguishes them: `Some("")` → true, `None` →
/// false. Non-empty junk ("abc"/"+"/"  ") already errors via mathevali in every
/// mode, so only the empty-string case is handled here. Verified vs bash 5.x.
#[inline]
pub fn bash_printf_empty_numeric_error(arg: Option<&String>) -> bool {
    bash_mode() && matches!(arg, Some(s) if s.is_empty())
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

/// Strip source-literal backslash escapes from a `${var/pat/REPL}` replacement
/// under `--bash`. bash removes a `\` before ANY char in the LITERAL
/// replacement (`\~`→`~`, `\&`→`&`, `\\`→`\`), but NOT from expanded values
/// (`${v/p/$x}` with x=`\~` stays `\~`). This runs on the tokenized replacement
/// BEFORE `singsub`, where source-literal backslashes are raw `\` (0x5c) while
/// expansion-defanging escapes (`\$`/`` \` ``/`\"`) are Bnull markers (0x9f) and
/// spliced values aren't present yet — so touching only raw `\` is exactly the
/// bash rule. A trailing lone `\` is kept.
pub fn strip_replacement_backslashes(s: &str) -> String {
    if !s.contains('\\') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{9f}' {
            // Bnull marker: the following char is an ALREADY-cooked literal (a
            // `\\`/`\$`/`` \` `` the DQ lexer defanged). Keep both bytes so it
            // survives — bash does not re-strip an already-processed backslash.
            out.push(c);
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else if c == '\\' {
            // Raw source-literal backslash: strip it, the next char is literal.
            match chars.next() {
                Some(next) => out.push(next),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
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

/// bash's `set -o` option table — the FIXED ~27 names bash accepts for
/// `set -o NAME` / `set +o NAME`, lists for `set -o` / `set +o`, and joins
/// into `$SHELLOPTS`. Each entry is `(bash name, zshrs option name)`; the
/// order is bash's own (already alphabetical), so no caller re-sorts.
///
/// Six bash names have NO zsh option behind them — `errtrace`, `functrace`,
/// `history`, `keyword`, `nolog`, `posix`. zsh's option table is a faithful
/// port and must not grow non-zsh entries (they would leak into `setopt`,
/// `${#options}` and `$options[…]` in `--zsh`), so their state lives in
/// [`BASH_ONLY_OPTS`] instead, keyed by the bash name. Both halves are read
/// back through [`bash_set_o_get`] so the listing, the query and `$SHELLOPTS`
/// all see one state.
///
/// !!! RUST-ONLY EXTENSION — no zsh C counterpart !!! zsh has no bash
/// personality; `emulate sh` only approximates it and rejects every bash-only
/// option name outright (`Src/options.c:640` `no such option`).
pub const BASH_SET_O: &[(&str, &str)] = &[
    ("allexport", "allexport"),
    ("braceexpand", "braceexpand"),
    ("emacs", "emacs"),
    ("errexit", "errexit"),
    ("errtrace", "errtrace"),
    ("functrace", "functrace"),
    ("hashall", "hashall"),
    ("histexpand", "histexpand"),
    ("history", "history"),
    ("ignoreeof", "ignoreeof"),
    ("interactive-comments", "interactivecomments"),
    ("keyword", "keyword"),
    ("monitor", "monitor"),
    ("noclobber", "noclobber"),
    ("noexec", "noexec"),
    ("noglob", "noglob"),
    ("nolog", "nolog"),
    ("notify", "notify"),
    ("nounset", "nounset"),
    ("onecmd", "singlecommand"),
    ("physical", "physical"),
    ("pipefail", "pipefail"),
    // bash's user-facing `set -o posix` toggle (off by default); NOT
    // zshrs's internal `posixbuiltins`, which is on in --bash.
    ("posix", "posix"),
    ("privileged", "privileged"),
    ("verbose", "verbose"),
    ("vi", "vi"),
    ("xtrace", "xtrace"),
];

/// The bash `set -o` names with no zsh option behind them. State-only: zshrs
/// records the flag so `set -o NAME`, the `set -o` listing and `$SHELLOPTS`
/// agree, but no behavior hangs off them yet. Kept OUT of zsh's option table
/// on purpose — see [`BASH_SET_O`].
///
/// Each defaults OFF, which is also bash's default for all six in a
/// non-interactive `bash -c` (verified: `bash -c 'set -o'`).
const BASH_ONLY_OPTS: &[(&str, &AtomicBool)] = &[
    ("errtrace", &BO_ERRTRACE),
    ("functrace", &BO_FUNCTRACE),
    ("history", &BO_HISTORY),
    ("keyword", &BO_KEYWORD),
    ("nolog", &BO_NOLOG),
    ("posix", &BO_POSIX),
];

static BO_ERRTRACE: AtomicBool = AtomicBool::new(false);
static BO_FUNCTRACE: AtomicBool = AtomicBool::new(false);
static BO_HISTORY: AtomicBool = AtomicBool::new(false);
static BO_KEYWORD: AtomicBool = AtomicBool::new(false);
static BO_NOLOG: AtomicBool = AtomicBool::new(false);
static BO_POSIX: AtomicBool = AtomicBool::new(false);

/// Read one bash `set -o` option's state by its BASH name: the bash-only
/// side-table first, else the zsh option it maps to.
pub fn bash_set_o_get(bash_name: &str) -> bool {
    if let Some((_, cell)) = BASH_ONLY_OPTS.iter().find(|(n, _)| *n == bash_name) {
        return cell.load(Ordering::Relaxed);
    }
    match BASH_SET_O.iter().find(|(b, _)| *b == bash_name) {
        Some((_, zname)) => crate::ported::options::opt_state_get(zname).unwrap_or(false),
        None => false,
    }
}

/// `set -o NAME` / `set +o NAME` in `--bash` mode.
///
/// Returns `None` when bash mode does not own the name, so the caller falls
/// through to zsh's faithful `optlookup` + `dosetopt` path (`Src/builtin.c:642`);
/// `Some(0)` when the assignment was applied.
///
/// Two of the names — `monitor` and `onecmd` — map to zsh options that
/// `dosetopt` refuses to change after startup (`Src/options.c:746`, the
/// INTERACTIVE / SHINSTDIN / SINGLECOMMAND gate). bash accepts both at any
/// time (`bash -c 'set -o monitor'` → status 0), so they are written straight
/// to the option state, which is what `dosetopt(…, force=1)` would do.
pub fn bash_set_o(bash_name: &str, on: bool) -> Option<i32> {
    if !bash_mode() {
        return None;
    }
    if let Some((_, cell)) = BASH_ONLY_OPTS.iter().find(|(n, _)| *n == bash_name) {
        cell.store(on, Ordering::Relaxed);
        return Some(0);
    }
    let (_, zname) = BASH_SET_O.iter().find(|(b, _)| *b == bash_name)?;
    // Via the alias resolver, not a raw write: several of these names are zsh
    // NEGATION aliases (`braceexpand` → NO_IGNORE_BRACES, `noglob` → NO_GLOB,
    // `nounset` → NO_UNSET, c:Src/options.c:269-280), so a raw
    // `opt_state_set("braceexpand", false)` would leave the canonical
    // `ignorebraces` slot untouched and the change would not take effect.
    crate::ported::options::opt_state_set_via_alias(zname, on);
    Some(0)
}

/// `$SHELLOPTS` in `--bash` mode: the colon-joined, alphabetically-ordered
/// list of `set -o` options currently ON (bash(1), "Shell Variables":
/// "SHELLOPTS — A colon-separated list of enabled shell options").
/// [`BASH_SET_O`] is already in bash's alphabetical order.
pub fn bash_shellopts() -> String {
    BASH_SET_O
        .iter()
        .filter(|(b, _)| bash_set_o_get(b))
        .map(|(b, _)| *b)
        .collect::<Vec<_>>()
        .join(":")
}
