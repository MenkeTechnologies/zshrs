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

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

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

/// Process-global zsh drop-in flag (`zshrs --zsh` / `--zsh-compat`).
///
/// `--zsh` promises identical behaviour to `/bin/zsh`, which means the
/// zshrs-only SYNTAX extensions have to be off — not just the caches and
/// the daemon. A construct zsh's parser rejects must keep being rejected,
/// with zsh's own diagnostic, or the compat-test entrypoint is measuring a
/// different language than the one it claims to stand in for.
///
/// Currently gates the `intercept <kind> <pat> { … }` block body, whose
/// raw-span capture in the lexer has no zsh counterpart: real zsh dies with
/// "parse error near `}'" because `}` cannot be a bare argument, and under
/// this flag zshrs does too.
///
/// Set only from the binary's CLI mode-application, so it defaults `false`
/// in the library and in every embedder.
static ZSH_DROPIN: AtomicBool = AtomicBool::new(false);

/// bash `shopt -s nocasematch` — case-insensitive `[[ == ]]` / `[[ =~ ]]` /
/// `case`. It is NOT a zsh option (opt_state can't store it), so it needs its
/// own flag. Toggled by the `shopt` builtin; read by cond.rs / case matching.
static NOCASEMATCH: AtomicBool = AtomicBool::new(false);

/// Process-global "this Korn drop-in is the pdksh line, not ksh93" flag,
/// raised for `zshrs --mksh` / `--pdksh`.
///
/// `--ksh`, `--mksh` and `--pdksh` all install the same `emulate ksh`
/// option preset and are otherwise indistinguishable at run time, but the
/// two lines genuinely differ where mksh inherited pdksh behavior ksh93
/// never had. The one this currently decides is `$PIPESTATUS`:
/// mksh(1) documents it ("PIPESTATUS: An array variable holding the exit
/// statuses of the last pipeline"), ksh93 has no such parameter —
/// `mksh -c 'true|false|true; print -r -- "[${PIPESTATUS[*]}]"'` → `[0 1 0]`
/// while ksh93 prints `[]`.
static PDKSH_FAMILY: AtomicBool = AtomicBool::new(false);

/// True for a bare `zshrs --mksh` / `--pdksh`. See [`PDKSH_FAMILY`].
#[inline]
pub fn pdksh_family() -> bool {
    PDKSH_FAMILY.load(Ordering::Relaxed)
}

/// Select the pdksh/mksh line of the Korn family. Called from the binary's
/// CLI mode application; cleared for `--ksh` and every other mode.
#[inline]
pub fn set_pdksh_family(on: bool) {
    PDKSH_FAMILY.store(on, Ordering::Relaxed);
}

/// True when `shopt -s nocasematch` is active (bash case-insensitive matching).
#[inline]
pub fn nocasematch() -> bool {
    NOCASEMATCH.load(Ordering::Relaxed)
}

/// Set/clear bash `nocasematch`.
///
/// !!! WARNING: RUST-ONLY HELPER — BASH IS THE REFERENCE, NOT zsh's C !!!
/// bash(1), The Shopt Builtin: "nocasematch — If set, bash matches patterns
/// in a case-insensitive fashion when performing matching while executing
/// case or [[ conditional commands". Three consumers, two mechanisms:
///
///   * `[[ … == … ]]` / `[[ … != … ]]` and `case` arms case-fold both sides
///     at their match sites, reading the [`NOCASEMATCH`] flag directly
///     (fusevm_bridge's BUILTIN_COND_STRMATCH and `str_match`, cond.rs:462).
///   * `[[ … =~ … ]]` goes through zsh's regex module, which already has the
///     exact knob bash wants: `Src/Modules/regex.c:74` builds its regcomp
///     flags as `REG_EXTENDED | (isset(CASEMATCH) ? 0 : REG_ICASE)`. Rather
///     than bolt a second case-folding path onto the regex engine, mirror
///     the bash flag onto zsh's CASE_MATCH with the sense inverted —
///     `nocasematch` ON means CASE_MATCH OFF means REG_ICASE. CASE_MATCH has
///     exactly one reader in the tree (`src/ported/modules/regex.rs:87/109`),
///     so the mirror cannot leak into any other construct.
///
/// Under `--zsh` nothing calls this, so zsh's own CASE_MATCH is untouched.
#[inline]
pub fn set_nocasematch(on: bool) {
    NOCASEMATCH.store(on, Ordering::Relaxed);
    // Inverted mirror onto zsh CASE_MATCH — see the doc comment above.
    crate::ported::options::opt_state_set_via_alias("casematch", !on);
}

/// The bash shopt names whose behavior is carried by a zsh option with the
/// OPPOSITE sense: the shopt is ON exactly when the zsh option is OFF.
///
/// !!! WARNING: RUST-ONLY TABLE — BASH IS THE REFERENCE, NOT zsh's C !!!
/// [`BASH_SHOPTS`]'s middle column can only express a same-sense mapping,
/// and the name(s) below are negations of the zsh option that implements
/// them, so they are resolved here instead of there.
///
///   * `xpg_echo` — bash(1): "If set, the echo builtin expands
///     backslash-escape sequences by default." That is precisely zsh's
///     `NO_BSD_ECHO`: `Src/builtin.c:4754` picks escape processing unless
///     BSD_ECHO is set and `-e` was not given, and `--bash` boots with
///     BSD_ECHO on (so `echo 'a\tb'` prints the backslash, as bash does).
///     Verified against bash 5.3 for `\t`, `\n`, `\c`, `\\`, `\e`, `\0101`
///     and for the `-e` / `-E` overrides, which win over the shopt in both
///     shells.
///   * `nocaseglob` is NOT here — zsh spells it `NO_CASE_GLOB` in the
///     negative too, so `optlookup` resolves the bash spelling directly and
///     the same-sense column already works.
const BASH_SHOPTS_INVERTED_ZSH_OPT: &[(&str, &str)] = &[("xpg_echo", "bsdecho")];

/// The `shopt` rows bash treats as READ-ONLY state, not as settable flags.
///
/// !!! WARNING: RUST-ONLY TABLE — BASH IS THE REFERENCE, NOT zsh's C !!!
/// Both report a fact about how the shell was started, so bash accepts
/// `shopt -s`/`-u` on them silently — status 0, no diagnostic — and simply
/// does not change the value:
///
/// ```text
///     $ bash -c  'shopt -s login_shell; echo rc=$?; shopt -p login_shell'
///     rc=0
///     shopt -u login_shell
///     $ bash -lc 'shopt -u login_shell; shopt -p login_shell'
///     shopt -s login_shell
/// ```
///
/// zshrs let both be written, so `shopt -s login_shell` made `$BASHOPTS`
/// claim a login shell in a non-login one. Their VALUES come from the zsh
/// option that already carries the same state (`LOGIN_SHELL`,
/// `src/ported/options.rs:108`; `restricted_shell` has none since zsh
/// dropped RESTRICTED in 54181 and stays off), which is why `zshrs --bash -l`
/// now reports `shopt -s login_shell` as bash does.
const BASH_SHOPTS_READONLY: &[&str] = &["login_shell", "restricted_shell"];

/// The zsh option carrying `name`'s behavior with an inverted sense, if any.
fn bash_shopt_inverted_zsh_opt(name: &str) -> Option<&'static str> {
    BASH_SHOPTS_INVERTED_ZSH_OPT
        .iter()
        .find(|(b, _)| *b == name)
        .map(|(_, z)| *z)
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

/// The exit status a FATAL shell error leaves behind, when the emulated
/// shell disagrees with zsh's `1`.
///
/// zsh's `errflag` abort ends the list and the status is whatever `lastval`
/// held — `ERRFLAG_ERROR` is literally `1` (c:Src/zsh.h:2970), so
/// c:Src/exec.c:3001 `lastval = errflag ? errflag : cmdoutval` yields 1.
/// dash does not model errors that way: `sh_error()` calls
/// `exraise(EXERROR)`, whose handler sets `exitstatus = 2` before
/// `exitshell()`, so EVERY fatal expansion / assignment / arithmetic error
/// leaves 2 no matter what the previous status was. Measured on
/// dash 0.5.x and ash, all four fatal shapes:
///
/// ```text
/// dash -c '(set -u; : "$nope")   2>/dev/null; printf "%d\n" $?'  → 2
/// dash -c '(: "${nope:?msg}")    2>/dev/null; printf "%d\n" $?'  → 2
/// dash -c '(readonly r=1; r=2)   2>/dev/null; printf "%d\n" $?'  → 2
/// dash -c '(: $((1/0)))          2>/dev/null; printf "%d\n" $?'  → 2
/// ```
///
/// bash, ksh93 and mksh all answer `1` for the same four, which is what
/// zshrs already produces — so this returns `None` outside the dash family
/// and every other mode is untouched. `/bin/sh` is deliberately NOT covered:
/// on this platform it is bash 3.2, which answers `127`, while on Linux it
/// is dash and answers `2`; encoding either would be encoding the host, not
/// the shell.
///
/// Applied at the two places dash's `exitshell` would be reached — the
/// `( … )` boundary and the end of a `-c` script — rather than at each
/// individual `zerr` call, which is exactly where `exraise` unwinds to.
///
/// !!! RUST-ONLY EXTENSION — no zsh C counterpart !!!
#[inline]
pub fn fatal_error_status() -> Option<i32> {
    // `dash_strict` alone would also fire for the zsh-STYLE leg
    // (`--dash --zsh`), which must keep zsh's 1; `posix_faithful` is what
    // distinguishes the drop-in from it.
    if posix_faithful() && dash_strict() {
        Some(2)
    } else {
        None
    }
}

/// True for the bare `zshrs --dash` drop-in (not the zsh-style
/// `--dash --zsh` leg). Gate for dash behaviors that deviate from zsh's
/// own sh emulation.
///
/// !!! RUST-ONLY EXTENSION — no zsh C counterpart !!!
#[inline]
pub fn dash_faithful() -> bool {
    posix_faithful() && dash_strict()
}

/// The option letters dash's builtin `name` accepts, where that is narrower
/// than zsh's (`None` = keep zsh's set). dash reports any other letter as
/// "Illegal option" (status 2). Measured on /opt/homebrew/bin/dash 0.5.13.5 and
/// /bin/dash: `read -n/-s/-d`, `local -r/-a/-x` and `unalias -x` all fail.
///
/// !!! RUST-ONLY EXTENSION — no zsh C counterpart !!!
pub fn dash_builtin_optstr(name: &str) -> Option<&'static [u8]> {
    match name {
        "read" => Some(b"rp"),
        "local" => Some(b""),
        "unalias" => Some(b"a"),
        "type" => Some(b""),
        _ => None,
    }
}

/// Rewrite every dash `echo` `\0` octal escape to the three-digit `\NNN` form
/// zsh's getkeystring reads. dash's `\0` introducer takes up to THREE digits
/// after the zero (`\0101` is `A`), while zsh's consumes the zero as one of
/// the three (`\0101` is `\010` then `1`). Escaped backslashes are skipped.
///
/// !!! RUST-ONLY EXTENSION — no zsh C counterpart !!!
pub fn dash_echo_octal_normalize(s: &str) -> String {
    let b: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != '\\' || i + 1 >= b.len() {
            out.push(b[i]);
            i += 1;
        } else if b[i + 1] == '0' {
            let digits: String = b[i + 2..].iter().take(3).take_while(|c| c.is_digit(8)).collect();
            out.push_str(&format!("\\{:0>3}", digits));
            i += 2 + digits.len();
        } else {
            out.push('\\');
            out.push(b[i + 1]);
            i += 2;
        }
    }
    out
}

/// dash's arithmetic read of a variable's value: empty is 0, otherwise the whole
/// value must be a `strtoimax(.., 0)` integer (optional sign, `0x` hex, leading-`0`
/// octal or decimal) with only blanks around it.
///
/// !!! RUST-ONLY EXTENSION — no zsh C counterpart !!!
pub fn dash_arith_var(raw: &str) -> Option<i64> {
    let t = raw.trim();
    if t.is_empty() {
        return Some(0);
    }
    let (neg, body) = match t.as_bytes()[0] {
        b'-' => (true, &t[1..]),
        b'+' => (false, &t[1..]),
        _ => (false, t),
    };
    let (radix, digits) = if let Some(h) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
        (16, h)
    } else if body.len() > 1 && body.starts_with('0') {
        (8, &body[1..])
    } else {
        (10, body)
    };
    if !digits.starts_with(|c: char| c.is_ascii_alphanumeric()) {
        return None;
    }
    let v = i64::from_str_radix(digits, radix).ok()?;
    Some(if neg { -v } else { v })
}

/// dash's POSIX special builtins (parser.c `goodname` rejects them as function
/// names; `type` words them "a special shell builtin").
const DASH_SPECIAL_BUILTINS: &[&str] = &[
    ":", ".", "break", "continue", "eval", "exec", "exit", "export",
    "readonly", "return", "set", "shift", "times", "trap", "unset",
];

/// True when `name` is one of dash's POSIX special builtins.
///
/// !!! RUST-ONLY EXTENSION — no zsh C counterpart !!!
pub fn dash_special_builtin(name: &str) -> bool {
    DASH_SPECIAL_BUILTINS.contains(&name)
}

/// Every builtin dash itself provides (`command -v NAME` prints the bare name).
/// Anything else zshrs registers (`let`, `print`, `typeset`, `setopt`, ...) is
/// "not found" in dash.
const DASH_BUILTINS: &[&str] = &[
    ":", ".", "[", "alias", "bg", "break", "cd", "chdir", "command", "continue",
    "echo", "eval", "exec", "exit", "export", "false", "fc", "fg", "getopts",
    "hash", "jobs", "kill", "local", "printf", "pwd", "read", "readonly",
    "return", "set", "shift", "test", "times", "trap", "true", "type", "ulimit",
    "umask", "unalias", "unset", "wait",
];

/// Mark every zshrs builtin dash does not have as DISABLED, so the name falls
/// through to PATH lookup (`let 1+1` -> "not found", 127) exactly as in dash.
/// Called once at startup of the bare `--dash`/`--ash` drop-in.
///
/// !!! RUST-ONLY EXTENSION — no zsh C counterpart !!!
pub fn dash_hide_foreign_builtins() {
    let Ok(mut disabled) = crate::ported::builtin::BUILTINS_DISABLED.lock() else {
        return;
    };
    for name in crate::ported::builtin::createbuiltintable().keys() {
        if !DASH_BUILTINS.contains(&name.as_str()) {
            disabled.insert(name.clone());
        }
    }
}

/// dash's `number()`: an optional `+` and decimal digits that fit an int.
/// Anything else (`abc`, `-1`, `1+1`, `0x10`, empty) is "Illegal number".
///
/// !!! RUST-ONLY EXTENSION — no zsh C counterpart !!!
pub fn dash_number(s: &str) -> Option<i32> {
    let a = s.trim_start();
    let digits = a.strip_prefix('+').unwrap_or(a);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse::<i32>().ok()
}

/// True when dash's parser rejects `name` as a function name
/// ("Syntax error: Bad function name"): anything that is not
/// `[A-Za-z_][A-Za-z0-9_]*` (parser.c `goodname`), or that names a POSIX
/// special builtin (`: . break continue eval exec exit export readonly
/// return set shift times trap unset`). Measured on /opt/homebrew/bin/dash.
///
/// !!! RUST-ONLY EXTENSION — no zsh C counterpart !!!
pub fn dash_bad_function_name(name: &str) -> bool {
    let mut chars = name.chars();
    let first_ok = matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic());
    !first_ok || !chars.all(|c| c == '_' || c.is_ascii_alphanumeric()) || dash_special_builtin(name)
}

/// True in a bare Korn drop-in — `zshrs --ksh`, `--mksh` or `--pdksh`.
///
/// Composed rather than stored: [`posix_faithful`] is raised only by a
/// BARE POSIX-family drop-in flag (cleared by `--zsh` and never set by the
/// runtime `emulate` builtin), and `EMULATION(EMULATE_KSH)` picks the Korn
/// leg out of that family. So this is false in `--zsh`, in native zshrs,
/// under `emulate ksh` typed at a zsh prompt, and in `--sh`/`--dash`/
/// `--bash`. Using the option/emulation bit ALONE would be wrong: a zsh
/// user who runs `emulate ksh` must keep zsh's behavior.
#[inline]
pub fn korn_mode() -> bool {
    posix_faithful() && crate::ported::zsh_h::EMULATION(crate::ported::zsh_h::EMULATE_KSH)
}

/// True for a bare `zshrs --ksh` (the ksh93 line), false for the pdksh line
/// (`--mksh` / `--pdksh`) and for every non-Korn mode.
#[inline]
pub fn ksh93_mode() -> bool {
    korn_mode() && !pdksh_family()
}

/// True when the shell being emulated has SPARSE indexed arrays — bash and
/// the whole Korn family.
///
/// bash(1), Arrays: "Arrays are assigned to using compound assignments …
/// Indexed array assignments do not require anything but *subscript*=*value*
/// … arrays are sparse, i.e. you do not have to define all the indices."
/// mksh(1) and ksh(1) match: `mksh -c 'a=(x y z); a[5]=q; print -r --
/// "${!a[@]}"'` → `0 1 2 5` and `${#a[@]}` → 4, exactly like bash.
///
/// zsh arrays are DENSE, so `a[5]=q` pads indices 3 and 4 — hence the hole
/// side-table in [`crate::bash_arrays`]. This predicate is the write-side
/// gate for that table; the read side keys off whether holes exist at all,
/// so widening this automatically widens `${#a[@]}` / `${!a[@]}` /
/// `"${a[*]}"` / `typeset -p` with no further change.
#[inline]
pub fn sparse_arrays() -> bool {
    bash_mode() || korn_mode()
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

/// Values of a ksh93 brace range `{from..to[..step]}`, verified against
/// ksh93u+m 1.0.10. Without a step the direction comes from the endpoints.
/// With a step its SIGN is the direction (`{10..1..-4}` -> 10 6 2, `{e..a..2}`
/// -> e): the start is always emitted and the walk stops at the first value
/// past `to` in that direction, so a step pointing away from `to` yields
/// just the start.
pub fn ksh93_range_values(from: i64, to: i64, step: Option<i64>) -> Vec<i64> {
    let step = step.unwrap_or(if from <= to { 1 } else { -1 });
    let mut vals = vec![from];
    let mut v = from + step;
    while (step > 0 && v <= to) || (step < 0 && v >= to) {
        vals.push(v);
        v += step;
    }
    vals
}

/// True in zsh drop-in mode (`zshrs --zsh` / `--zsh-compat`). Gates OFF the
/// zshrs-only syntax extensions so the compat entrypoint parses exactly what
/// `/bin/zsh` parses. See [`ZSH_DROPIN`].
#[inline]
pub fn zsh_dropin() -> bool {
    ZSH_DROPIN.load(Ordering::Relaxed)
}

/// Set (or clear) zsh drop-in mode. Called from the binary's CLI mode
/// application (raised for `--zsh` / `--zsh-compat`).
#[inline]
pub fn set_zsh_dropin(on: bool) {
    ZSH_DROPIN.store(on, Ordering::Relaxed);
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
    if !s.contains('\\') && !s.contains('\u{e19f}') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{e19f}' {
            // Bnull marker: the following char is an ALREADY-cooked literal (a
            // `\\`/`\$`/`` \` `` the DQ lexer defanged). Keep both bytes so it
            // survives — bash does not re-strip an already-processed backslash.
            // bash patsub_replacement: a lexer-cooked `\&` is a literal `&`.
            if chars.clone().next() == Some('&')
                && bash_shopt_get("patsub_replacement") == Some(true)
            {
                chars.next();
                out.push(PATSUB_LITERAL_AMP);
                continue;
            }
            out.push(c);
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else if c == '\\' {
            // Raw source-literal backslash: strip it, the next char is literal.
            match chars.next() {
                // bash patsub_replacement: `\&` is a literal `&` that the later
                // `&` -> matched-text pass (`bash_patsub`) must leave alone.
                Some('&') if bash_shopt_get("patsub_replacement") == Some(true) => {
                    out.push(PATSUB_LITERAL_AMP)
                }
                Some(next) => out.push(next),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// True when the shell is executing a script file or stdin rather than a
/// `-c` string: only then does bash add the outermost `main` frame to
/// `FUNCNAME` / `BASH_SOURCE` / `BASH_LINENO`.
fn bash_running_script() -> bool {
    crate::ported::params::getsparam("ZSH_EXECUTION_STRING").map_or(true, |c| c.is_empty())
}

/// The name bash reports for the top-level script (`$0` at top level).
fn bash_script_name() -> String {
    crate::ported::params::getsparam("ZSH_ARGZERO").unwrap_or_default()
}

/// Resolve a bash special ARRAY name (`PIPESTATUS`, `FUNCNAME`,
/// `BASH_VERSINFO`) to its value in `--bash` mode by aliasing the zsh-native
/// special or synthesizing it. Returns `None` for any other name (or outside
/// bash mode) so callers fall through to normal array resolution.
pub fn bash_special_array(name: &str) -> Option<Vec<String>> {
    // `$PIPESTATUS` is not bash's alone: mksh(1) documents it verbatim —
    // "PIPESTATUS: An array variable holding the exit statuses of the last
    // pipeline" — and `mksh -c 'true|false|true; print -r --
    // "[${PIPESTATUS[*]}]"'` prints `[0 1 0]`. ksh93 has NO such parameter
    // (same command prints `[]`), so this widens to the pdksh line only,
    // not to `--ksh`. `FUNCNAME` and `BASH_VERSINFO` stay bash-only —
    // neither Korn shell has them.
    if !bash_mode() && !(name == "PIPESTATUS" && pdksh_family()) {
        return None;
    }
    match name {
        // Under the Korn emulation `pipestatus` is an ordinary parameter, not
        // the special (zsh `emulate ksh` leaves it unset after a pipeline), so
        // the pdksh line reads the C globals the pipeline wrote — what
        // pipestatgetfn walks (c:Src/params.c:5269-5276).
        "PIPESTATUS" if !bash_mode() => {
            use crate::ported::jobs::{NUMPIPESTATS, PIPESTATS};
            let n = NUMPIPESTATS
                .get()
                .and_then(|m| m.lock().ok().map(|g| *g))
                .unwrap_or(0);
            let stats = PIPESTATS
                .get()
                .and_then(|m| m.lock().ok().map(|p| p[..n].to_vec()))
                .unwrap_or_default();
            Some(stats.iter().map(|s| s.to_string()).collect())
        }
        // bash PIPESTATUS ≈ zsh pipestatus (per-stage exit codes, 0-indexed).
        "PIPESTATUS" => Some(crate::ported::exec::array("pipestatus").unwrap_or_default()),
        // bash FUNCNAME ≈ zsh funcstack — call stack, innermost (current) first.
        // ...plus `main` as the outermost frame when running a script (not `-c`).
        "FUNCNAME" => crate::ported::modules::parameter::FUNCSTACK.lock().ok().map(|f| {
            // FUNCNAME exists only while a function runs; a sourced file is a "source" frame.
            if !f.iter().any(|fs| fs.tp == crate::ported::zsh_h::FS_FUNC) {
                return Vec::new();
            }
            let mut names: Vec<String> = f
                .iter()
                .rev()
                .map(|fs| if fs.tp == crate::ported::zsh_h::FS_SOURCE { "source".to_string() } else { fs.name.clone() })
                .collect();
            if bash_running_script() {
                names.push("main".to_string());
            }
            names
        }),
        // bash BASH_SOURCE: the file each frame was defined in, then the script itself.
        "BASH_SOURCE" => crate::ported::modules::parameter::FUNCSTACK.lock().ok().map(|f| {
            let script = bash_script_name();
            let mut files: Vec<String> = f
                .iter()
                .rev()
                .map(|fs| fs.filename.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| script.clone()))
                .collect();
            if bash_running_script() {
                files.push(script);
            }
            files
        }),
        // bash BASH_LINENO: the line each frame was called from, then 0 for the script.
        "BASH_LINENO" => crate::ported::modules::parameter::FUNCSTACK.lock().ok().map(|f| {
            let mut lines: Vec<String> = f.iter().rev().map(|fs| fs.lineno.to_string()).collect();
            if bash_running_script() {
                lines.push("0".to_string());
            }
            lines
        }),
        "BASH_VERSINFO" => Some(bash_versinfo()),
        _ => None,
    }
}

/// bash's `shopt` option table: `(bash name, zshrs option key, bash default)`.
///
/// The name list and the defaults are bash 5.3's own — `bash -c shopt`
/// prints exactly these 59 rows in this order, and `shopt` lists them
/// alphabetically, so no caller re-sorts. bash rejects anything outside the
/// list: `bash -c 'shopt -p zznope'` → "shopt: zznope: invalid shell option
/// name", status 1.
///
/// The middle field is where the state LIVES:
///   * `Some(opt)` — a real zsh option carries the behavior, so the flag is
///     read and written through `opt_state`. Twelve names zsh already has
///     under the same spelling (`optlookup` is underscore- and case-blind),
///     plus two renames whose behavior zsh implements under a different
///     name: bash `extglob` is zsh `kshglob` (identical `@()`/`*()`/`+()`/
///     `?()`/`!()` ksh patterns), and bash `failglob` — "if a pattern fails
///     to match, an error message is printed and the command is not
///     executed" (bash(1) The Shopt Builtin) — is zsh `nomatch`.
///   * `None` — bash-only, no zsh option behind it. zsh's option table is a
///     faithful port and must not grow non-zsh rows (they would leak into
///     `setopt` / `${#options}` under `--zsh`), so the state lives in
///     [`BASH_ONLY_SHOPTS`] keyed by the bash name, seeded from the default
///     in this table.
///
/// !!! RUST-ONLY EXTENSION — no zsh C counterpart !!!
pub const BASH_SHOPTS: &[(&str, Option<&str>, bool)] = &[
    ("array_expand_once", None, false),
    ("assoc_expand_once", None, false),
    ("autocd", Some("autocd"), false),
    ("bash_source_fullpath", None, false),
    ("cdable_vars", Some("cdable_vars"), false),
    ("cdspell", None, false),
    ("checkhash", None, false),
    ("checkjobs", Some("checkjobs"), false),
    ("checkwinsize", None, true),
    ("cmdhist", None, true),
    ("compat31", None, false),
    ("compat32", None, false),
    ("compat40", None, false),
    ("compat41", None, false),
    ("compat42", None, false),
    ("compat43", None, false),
    ("compat44", None, false),
    ("complete_fullquote", None, true),
    ("direxpand", None, false),
    ("dirspell", None, false),
    ("dotglob", Some("dotglob"), false),
    ("execfail", None, false),
    ("expand_aliases", None, false),
    ("extdebug", None, false),
    ("extglob", Some("kshglob"), false),
    ("extquote", None, true),
    ("failglob", Some("nomatch"), false),
    ("force_fignore", None, true),
    ("globasciiranges", None, true),
    ("globskipdots", None, true),
    ("globstar", None, false),
    ("gnu_errfmt", None, false),
    ("histappend", Some("histappend"), false),
    ("histreedit", None, false),
    ("histverify", Some("histverify"), false),
    ("hostcomplete", None, true),
    ("huponexit", None, false),
    ("inherit_errexit", None, false),
    ("interactive_comments", Some("interactive_comments"), true),
    ("lastpipe", None, false),
    ("lithist", None, false),
    ("localvar_inherit", None, false),
    ("localvar_unset", None, false),
    // Backed by zsh's LOGINSHELL, which is what `argv[0]`'s leading dash
    // and `-l` both set — bash(1) calls this one "not settable": it
    // REPORTS how the shell was started rather than configuring it. It is
    // therefore excluded from `bash_shopt_apply_defaults` below, which
    // would otherwise stamp `false` over a real login shell's state.
    ("login_shell", Some("loginshell"), false),
    ("mailwarn", Some("mailwarn"), false),
    ("no_empty_cmd_completion", None, false),
    ("nocaseglob", Some("nocaseglob"), false),
    ("nocasematch", Some("nocasematch"), false),
    ("noexpand_translation", None, false),
    ("nullglob", Some("nullglob"), false),
    ("patsub_replacement", None, true),
    ("progcomp", None, true),
    ("progcomp_alias", None, false),
    ("promptvars", Some("promptvars"), true),
    ("restricted_shell", None, false),
    ("shift_verbose", None, false),
    ("sourcepath", None, true),
    ("varredir_close", None, false),
    ("xpg_echo", None, false),
];

thread_local! {
    /// Live state for the `BASH_SHOPTS` rows with no zsh option behind them
    /// (`None` in the middle column). Absent key ⇒ the table's default.
    static BASH_ONLY_SHOPTS: RefCell<HashMap<&'static str, bool>> =
        RefCell::new(HashMap::new());
}

/// The `BASH_SHOPTS` row for `name`, or `None` when bash would reject the
/// name outright ("invalid shell option name", status 1).
pub fn bash_shopt_row(name: &str) -> Option<(&'static str, Option<&'static str>, bool)> {
    BASH_SHOPTS.iter().copied().find(|(n, _, _)| *n == name)
}

/// Read one bash `shopt` flag. `None` when the name is not a bash shopt.
pub fn bash_shopt_get(name: &str) -> Option<bool> {
    let (canon, zsh_opt, default_on) = bash_shopt_row(name)?;
    // `nocasematch` is not an option at all in zsh — `opt_state` cannot
    // store it — so it keeps its own flag (see NOCASEMATCH).
    if canon == "nocasematch" {
        return Some(nocasematch());
    }
    // Inverted-sense rows (`xpg_echo` ↔ zsh BSD_ECHO) — see
    // BASH_SHOPTS_INVERTED_ZSH_OPT.
    if let Some(zopt) = bash_shopt_inverted_zsh_opt(canon) {
        return Some(!crate::ported::options::opt_state_get(zopt).unwrap_or(!default_on));
    }
    Some(match zsh_opt {
        Some(opt) => crate::ported::options::opt_state_get(opt).unwrap_or(default_on),
        None => BASH_ONLY_SHOPTS
            .with(|m| m.borrow().get(canon).copied())
            .unwrap_or(default_on),
    })
}

/// Write one bash `shopt` flag. Returns false when the name is not a bash
/// shopt (caller emits bash's "invalid shell option name" and exits 1).
pub fn bash_shopt_set(name: &str, on: bool) -> bool {
    let Some((canon, zsh_opt, _)) = bash_shopt_row(name) else {
        return false;
    };
    // Read-only state rows — accepted and ignored, as bash does. See
    // BASH_SHOPTS_READONLY.
    if BASH_SHOPTS_READONLY.contains(&canon) {
        return true;
    }
    if canon == "nocasematch" {
        set_nocasematch(on);
        return true;
    }
    // Inverted-sense rows (`xpg_echo` ↔ zsh BSD_ECHO) — see
    // BASH_SHOPTS_INVERTED_ZSH_OPT.
    if let Some(zopt) = bash_shopt_inverted_zsh_opt(canon) {
        crate::ported::options::opt_state_set_via_alias(zopt, !on);
        return true;
    }
    match zsh_opt {
        // Through the alias-aware setter: `opt_state_get` canonicalises via
        // `optlookup`, so a raw write under the bash spelling
        // (`cdable_vars`) would land in a slot the read never consults
        // (`cdablevars`) — `shopt -s cdable_vars; shopt -p cdable_vars`
        // reported `-u`.
        Some(opt) => {
            crate::ported::options::opt_state_set_via_alias(opt, on);
        }
        None => BASH_ONLY_SHOPTS.with(|m| {
            m.borrow_mut().insert(canon, on);
        }),
    }
    true
}

/// Install bash's `shopt` defaults for every row whose state lives in a
/// REAL zsh option.
///
/// The bash-only rows default correctly on their own (`BASH_ONLY_SHOPTS`
/// falls back to the table), but the twelve zsh-backed ones inherit zsh's
/// default, which is not always bash's: zsh's `histappend`
/// (`APPEND_HISTORY`) is ON where bash's is OFF, so `shopt -p histappend`
/// reported `shopt -s histappend` / status 0 against bash's
/// `shopt -u histappend` / status 1 — and the shell really did append where
/// bash truncates.
///
/// Called once from the binary's `--bash` mode application, BEFORE any user
/// code runs, so a script's own `setopt`/`shopt` still wins.
pub fn bash_shopt_apply_defaults() {
    for (name, zsh_opt, default_on) in BASH_SHOPTS {
        // Derived rows report how the shell was STARTED; they have no
        // configurable default to stamp. `login_shell` is set from
        // `argv[0]`'s leading dash / `-l` before any user code runs, and
        // writing the table default here would report a real login shell
        // as a non-login one.
        if DERIVED_SHOPTS.contains(name) {
            continue;
        }
        // Inverted rows (`xpg_echo`) live in a zsh option too, so they need
        // the same seeding even though the middle column is `None`.
        if zsh_opt.is_some() || bash_shopt_inverted_zsh_opt(name).is_some() {
            bash_shopt_set(name, *default_on);
        }
    }
}

/// shopt rows whose value is derived from how the shell was invoked, not
/// from a default. bash(1) lists these under "not settable".
const DERIVED_SHOPTS: &[&str] = &["login_shell", "restricted_shell"];

/// `$BASHOPTS` — bash(1): the shopt options "valid as an argument for the
/// -s option to shopt", colon-separated, in the table's (alphabetical)
/// order. Only the ENABLED ones appear.
pub fn bash_shoptsopts() -> String {
    BASH_SHOPTS
        .iter()
        .filter(|(n, _, _)| bash_shopt_get(n).unwrap_or(false))
        .map(|(n, _, _)| *n)
        .collect::<Vec<_>>()
        .join(":")
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

/// The bash `set` option letter -> `set -o` name, for the letters zsh either
/// lacks or reads differently (`set -h`, `-B`, `-E`, `-H`, `-P`, `-T`, `-k`, `-t`).
const BASH_SET_LETTERS: &[(char, &str)] = &[
    ('h', "hashall"),
    ('B', "braceexpand"),
    ('E', "errtrace"),
    ('H', "histexpand"),
    ('P', "physical"),
    ('T', "functrace"),
    ('k', "keyword"),
    ('t', "onecmd"),
];

/// `set -L` / `set +L` for a bash-only letter. Returns false when `letter` is
/// not one of them (the caller then takes zsh's letter table).
pub fn bash_set_letter(letter: char, on: bool) -> bool {
    bash_mode()
        && BASH_SET_LETTERS
            .iter()
            .find(|(l, _)| *l == letter)
            .is_some_and(|(_, name)| bash_set_o(name, on).is_some())
}

/// `$-` in `--bash` mode: bash's flag letters, lowercase alphabetical (with
/// `i`), then the uppercase ones, then `c` / `s` for the invocation kind.
/// `bash -c 'echo $-'` is `hBc`.
pub fn bash_dollar_dash() -> String {
    const ORDER: &[(char, &str)] = &[
        ('a', "allexport"),
        ('b', "notify"),
        ('e', "errexit"),
        ('f', "noglob"),
        ('h', "hashall"),
        ('i', ""),
        ('k', "keyword"),
        ('m', "monitor"),
        ('n', "noexec"),
        ('p', "privileged"),
        ('t', "onecmd"),
        ('u', "nounset"),
        ('v', "verbose"),
        ('x', "xtrace"),
        ('B', "braceexpand"),
        ('C', "noclobber"),
        ('E', "errtrace"),
        ('H', "histexpand"),
        ('P', "physical"),
        ('T', "functrace"),
    ];
    use crate::ported::zsh_h::{isset, INTERACTIVE, SHINSTDIN};
    let mut out: String = ORDER
        .iter()
        .filter(|(l, name)| if *l == 'i' { isset(INTERACTIVE) } else { bash_set_o_get(name) })
        .map(|(l, _)| *l)
        .collect();
    if crate::ported::params::getsparam("ZSH_EXECUTION_STRING").is_some_and(|c| !c.is_empty()) {
        out.push('c');
    }
    if isset(SHINSTDIN) {
        out.push('s');
    }
    out
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

// ===========================================================================
// !!! WARNING: RUST-ONLY HELPERS — NO zsh C COUNTERPART !!!
//
// Output-format and bookkeeping deltas between zsh and the REAL Bourne-family
// shells that `zshrs --bash` / `--ksh` / `--mksh` / `--pdksh` / `--sh` /
// `--dash` / `--ash` stand in for. zsh's C source is NOT the spec for any of
// these — each was measured against the actual reference binary and the
// observed output is quoted at the definition.
//
// Every predicate below hangs off `posix_faithful()`, which the binary raises
// only for a BARE drop-in flag, so all of them are false in native zshrs, in
// `--zsh`, under a runtime `emulate sh`, and in the zsh-STYLE cross-emulation
// legs (`--sh --zsh` / `--ksh --zsh`). zsh's own formats cannot regress.
// ===========================================================================

/// `umask` with no arguments prints a FIXED four-octal-digit mask in the real
/// Bourne-family shells; zsh prints three digits and emits the leading `0`
/// only when the owner field is non-zero (`Src/builtin.c:7522-7524` —
/// `if (um & 0700) putchar('0'); printf("%03o\n", um);`).
///
/// Measured (`<shell> -c 'umask 022; umask'`):
///
/// ```text
/// bash 5.3 → 0022    dash → 0022    ksh93 → 0022    ash → 0022
/// /bin/sh  → 0022    mksh → 022     zsh   → 022
/// ```
///
/// mksh is the outlier — it uses zsh's conditional-zero form — so the pdksh
/// line is excluded. Confirmed across the range: `000`/`007`/`022`/`077` gain
/// the zero, `0700`/`0777` already have it, and `umask -S` is byte-identical
/// in every shell, so the `-S` arm is untouched.
#[inline]
pub fn umask_four_digit() -> bool {
    posix_faithful() && !pdksh_family()
}

// ---------------------------------------------------------------------------
// getopts: $OPTIND reporting, the end-of-options contract, and reset detection
// ---------------------------------------------------------------------------

/// Last value zshrs wrote to `$OPTIND`, and the internal `zoptind` it came
/// from. `bin_getopts` re-reads `$OPTIND` at entry (scripts reset it to start
/// a new parse), so a reported value carrying the emulation bias has to be
/// translated back before it is used as an argument index. Both start at -1,
/// which no report can produce, so the first call always looks like a script
/// assignment.
static GETOPTS_REPORTED: AtomicI32 = AtomicI32::new(-1);
/// Internal `zoptind` matching [`GETOPTS_REPORTED`].
static GETOPTS_INTERNAL: AtomicI32 = AtomicI32::new(-1);

/// The value the emulated shell would show in `$OPTIND`, given zsh's internal
/// parse state — and the point where the two families disagree.
///
/// zsh advances `zoptind` LAZILY: it names the argument currently being
/// scanned and is bumped only on the NEXT `getopts` call, when the cursor
/// `optcind` is found past the end of that argument
/// (`Src/builtin.c:5699-5703`). Every real Bourne shell instead reports the
/// index of the next argument as soon as the current one is finished. zsh's
/// own answer is 1 where all of them say 2 — a genuine zsh-vs-POSIX semantic
/// difference, not a port bug, so it is corrected only in the drop-in modes:
///
/// ```text
/// $ for s in bash dash ksh mksh ash zsh; do $s -c \
///     'OPTIND=1; getopts "ab" o -b; printf "%s\n" "$OPTIND"'; done
/// 2  2  2  2  2  1
/// ```
///
/// The families split mid-CLUSTER (`-ab`). bash and ksh93 keep reporting the
/// cluster's own index until it is exhausted; dash, ash and mksh report the
/// next index as soon as any character has been consumed:
///
/// ```text
/// $ for s in bash ksh dash ash mksh; do $s -c \
///     'OPTIND=1; getopts "ab" o -ab; printf "%s\n" "$OPTIND"'; done
/// 1  1  2  2  2
/// ```
///
/// `optcind == 0` means "positioned at an argument boundary" — nothing was
/// taken from the current word (a fresh call, an exhausted-and-advanced
/// cursor, or the post-`OPTARG` reset at `Src/builtin.c:5771-5772`) — and
/// every shell, zsh included, reports `zoptind` unchanged there.
///
/// `--sh` follows the bash rule: `/bin/sh` is bash on this platform (and on
/// the RHEL-family Linuxes), and the two rules differ only mid-cluster.
///
/// Records the pair so [`getopts_optind_internal`] can undo the bias on the
/// next call. Outside the drop-in modes it returns `zoptind` untouched and
/// records nothing.
pub fn getopts_optind_report(zoptind: i32, optcind: i32, lenstr: i32) -> i32 {
    if !posix_faithful() {
        return zoptind;
    }
    if optcind == 0 {
        // Argument boundary: the value shown IS the internal index. Record the
        // identity pair so a stale mid-cluster pair (`-abval`: a -> 2 from 1)
        // cannot alias this value back onto the earlier index on the next call.
        GETOPTS_REPORTED.store(zoptind, Ordering::Relaxed);
        GETOPTS_INTERNAL.store(zoptind, Ordering::Relaxed);
        return zoptind;
    }
    let eager = dash_strict() || pdksh_family();
    let reported = if eager || optcind >= lenstr {
        zoptind + 1
    } else {
        zoptind
    };
    GETOPTS_REPORTED.store(reported, Ordering::Relaxed);
    GETOPTS_INTERNAL.store(zoptind, Ordering::Relaxed);
    reported
}

/// Translate the `$OPTIND` a script can see back into the internal `zoptind`
/// `bin_getopts` left off at. Only the exact value zshrs itself last reported
/// is translated; anything else is a script assignment (`OPTIND=1`) and is
/// passed through so the reset still works. Identity outside the drop-in modes.
pub fn getopts_optind_internal(param_value: i64) -> i64 {
    if !posix_faithful() {
        return param_value;
    }
    let reported = GETOPTS_REPORTED.load(Ordering::Relaxed);
    if reported >= 0 && param_value == reported as i64 {
        return GETOPTS_INTERNAL.load(Ordering::Relaxed) as i64;
    }
    param_value
}

/// True when a script's own write to `$OPTIND` must ALSO rewind the
/// within-argument cursor, so the next `getopts` re-scans that argument from
/// its first character.
///
/// Every POSIX shell measured treats a CHANGED `$OPTIND` as a full reset,
/// dash included — its `getoptsreset()` drops the within-argument offset when
/// the variable is assigned. zsh instead rewinds only when the new value is 1
/// AND its internal index was elsewhere. Measured on this host:
///
/// ```text
/// $ <shell> -c 'OPTIND=1; getopts "ab" o -ab; OPTIND=1; getopts "ab" o -ab;
///               printf "%s/%s\n" "$o" "$OPTIND"'
/// dash a/2      bash a/1      mksh a/2
/// ```
///
/// What separates the families is an assignment of the SAME value, which bash
/// treats as a reset (it hooks the assignment) while dash and mksh do not
/// (they compare values):
///
/// ```text
/// $ <shell> -c 'OPTIND=1; getopts "ab" o -ab; OPTIND=$OPTIND;
///               getopts "ab" o -ab; printf "%s/%s\n" "$o" "$OPTIND"'
/// dash ?/2      bash a/1      mksh ?/2
/// ```
///
/// The value comparison below is therefore right for the dash family too; it
/// used to be suppressed by a `dash_strict()` guard on the strength of the
/// first table having been recorded as `dash/ash → ?/2`, which no dash tested
/// here produces.
///
/// `raw_param` is `$OPTIND` exactly as the parameter table holds it, BEFORE
/// [`getopts_optind_internal`] removes the reporting bias; anything other than
/// the value zshrs last wrote is a script assignment. False outside the
/// drop-in modes and in the dash family, so zsh's own rule
/// (`Src/builtin.c:5681-5685`) stands alone there.
///
/// KNOWN GAP: bash hooks the ASSIGNMENT rather than comparing values, so it
/// also resets when the assigned value equals the one it last reported —
/// visible only mid-cluster (`getopts ab o -ab; OPTIND=1; getopts ab o -ab`
/// → bash `a`, here `b`). Catching that needs an `$OPTIND` assignment hook in
/// the parameter table; pinned by an ignored parity test rather than faked.
pub fn getopts_optind_user_reset(raw_param: i64) -> bool {
    if !posix_faithful() {
        return false;
    }
    raw_param != GETOPTS_REPORTED.load(Ordering::Relaxed) as i64
}

/// POSIX end-of-options bookkeeping for `getopts`, which zsh does not do.
///
/// XCU `getopts`: "When the end of options is encountered, the getopts utility
/// shall exit with a return value greater than zero; … and *name* shall be set
/// to the question-mark character." zsh leaves *name* holding whatever it held
/// before:
///
/// ```text
/// $ <shell> -c 'o=INIT; OPTIND=1; getopts "ab" o --; printf "[%s]\n" "$o"'
/// bash/ksh/mksh/dash/ash → [?]        zsh → [INIT]
/// ```
///
/// bash, ksh93 and mksh also CLEAR `$OPTARG` on that return; dash and ash
/// leave the last option's argument in place, and so does zsh:
///
/// ```text
/// $ <shell> -c 'OPTIND=1; for i in 1 2 3; do getopts "a:b" o -b -a W; done;
///               printf "[%s]\n" "$OPTARG"'
/// bash/ksh/mksh → []      dash/ash → [W]      zsh → [W]
/// ```
///
/// Call at every "no more options" (`return 1`) exit of `bin_getopts`. No-op
/// outside the drop-in modes.
pub fn getopts_end_of_options(var: &str) {
    if !posix_faithful() {
        return;
    }
    crate::ported::params::setsparam(var, "?");
    if !dash_strict() {
        crate::ported::params::setsparam("OPTARG", "");
    }
}

// ---------------------------------------------------------------------------
// `trap` with no arguments — the listing format
// ---------------------------------------------------------------------------

/// One `trap -- <body> <SIG>` listing line in the emulated shell's own format,
/// or `None` when zsh's format (`Src/builtin.c:7370-7375`) applies unchanged.
///
/// Three deltas, each measured rather than derived from zsh's C:
///
/// 1. **Raw body, not a re-deparse.** zsh compiles the trap body to an Eprog
///    and renders THAT back for the listing (`getpermtext`), so the text is
///    canonicalised. Every other shell echoes the string it was given:
///
///    ```text
///    $ trap 'printf a; printf b' INT; trap
///    zsh                    → trap -- $'printf a\nprintf b' INT
///    bash/dash/ash/ksh/mksh → trap -- 'printf a; printf b' <SIG>
///    ```
///
/// 2. **Quoting.** zsh and the Korn pair quote only when the body needs it, so
///    `trap ':' HUP` lists as `trap -- : HUP` there and as `trap -- ':' HUP`
///    in bash, dash, ash and `/bin/sh`. The escapes differ too — measured with
///    the bodies `printf 'a b'`, `'x`, `x'`, `a'b'c` and `a\b'`:
///
///    ```text
///    body        bash / /bin/sh   dash / ash        ksh93          mksh
///    printf 'a b' 'printf '\''a b'\''' 'printf '"'"'a b'"'" $'printf \'a b\'' 'printf '\''a b'\'
///    'x           ''\''x'         ''"'"'x'          $'\'x'         \''x'
///    x'           'x'\'''         'x'"'"            $'x\''         'x'\'
///    a\b'         'a\b'\'''       'a\b'"'"          $'a\\b\''      'a\b'\'
///    ```
///
///    bash keeps empty runs at BOTH ends; dash and ash drop a trailing empty
///    one but keep a leading one; ksh93 switches to ANSI-C `$'…'` as soon as
///    the body holds an apostrophe; mksh is byte-identical to zsh's
///    `quotedzputs`, empty-run trimming included.
///
/// 3. **`SIG`-prefixed names in bash only.** `bash -c "trap ':' HUP; trap"`
///    prints `trap -- ':' SIGHUP`, while dash, ash, `/bin/sh`, ksh93 and mksh
///    all print the bare `HUP`. bash leaves the pseudo-signals alone —
///    `EXIT`, `DEBUG`, `ERR` and `RETURN` print unprefixed — so only names
///    resolving to a real signal number are prefixed.
///
/// An empty body is `''` in every shell including zsh, so it needs no special
/// case beyond forcing the quotes.
pub fn trap_listing_line(signame: &str, body: &str) -> Option<String> {
    if !posix_faithful() {
        return None;
    }
    let quoted = if korn_mode() && !pdksh_family() && body.contains('\'') {
        // ksh93's ANSI-C form: backslash doubles, apostrophe is escaped.
        format!("$'{}'", body.replace('\\', "\\\\").replace('\'', "\\'"))
    } else if korn_mode() {
        // ksh93 without an apostrophe, and mksh always: quote only when
        // needed, exactly like zsh — but over the RAW body.
        crate::ported::utils::quotedzputs(body)
    } else if dash_strict() {
        // dash / ash close the quote, emit `"'"`, reopen — then drop a
        // TRAILING empty reopened run (a leading one is kept).
        let mut q = format!("'{}'", body.replace('\'', "'\"'\"'"));
        // An IGNORED trap has an empty body and lists as `trap -- '' SIG` in
        // every shell, so the trim must not eat the whole quoted string.
        if q.len() > 2 && q.ends_with("''") {
            q.truncate(q.len() - 2);
        }
        q
    } else {
        // bash and /bin/sh: `'` → `'\''`, no empty-run trimming at either end.
        format!("'{}'", body.replace('\'', "'\\''"))
    };
    let name = if bash_mode() && real_signal_name(signame) {
        format!("SIG{}", signame)
    } else {
        signame.to_string()
    };
    Some(format!("trap -- {} {}", quoted, name))
}

/// True when `name` is a real signal (1..=SIGCOUNT) rather than one of the
/// pseudo-signals zsh keeps in the same table (`EXIT` at index 0, `ZERR` and
/// `DEBUG` above `SIGCOUNT` — `Src/signals.h:34-46`). Only real signals take
/// bash's `SIG` prefix.
fn real_signal_name(name: &str) -> bool {
    match crate::ported::jobs::getsigidx(name) {
        Some(idx) => idx >= 1 && idx <= crate::ported::signals_h::SIGCOUNT,
        None => false,
    }
}

/// ksh93 lists `trap` entries in DESCENDING signal-number order, where zsh,
/// bash, dash, ash and mksh all walk the table upwards:
///
/// ```text
/// $ <shell> -c "trap ':' EXIT INT HUP USR1 TERM QUIT; trap"
/// ksh93                 zsh / bash / dash / ash / mksh
///   trap -- : USR1        trap -- : EXIT
///   trap -- : TERM        trap -- : HUP     (bash: SIGHUP, …)
///   trap -- : QUIT        trap -- : INT
///   trap -- : INT         trap -- : QUIT
///   trap -- : HUP         trap -- : TERM
///   trap -- : EXIT        trap -- : USR1
/// ```
///
/// True only for a bare `--ksh`; the pdksh line (`--mksh` / `--pdksh`) matches
/// zsh's ascending walk (`Src/builtin.c:7358` — `for (sig = 0;
/// sig < TRAPCOUNT; sig++)`), and so does every other mode.
#[inline]
pub fn trap_listing_descending() -> bool {
    korn_mode() && !pdksh_family()
}

// ---------------------------------------------------------------------------
// `type` / `command -V` / `whence -v` on a shell function
// ---------------------------------------------------------------------------

/// The verbose one-line description of a shell function in the emulated
/// shell's wording, or `None` to keep zsh's.
///
/// zsh appends the file the function was loaded from
/// (`Src/hashtable.c:927-941` — `" is a shell function"` then `" from "` and
/// the filename), which for a `-c` script is the shell's own name, so
/// `zsh -fc 'f() { :; }; type f'` prints `f is a shell function from zsh`.
/// No other shell says anything of the kind:
///
/// ```text
/// bash 5.3 → f is a function          (then the definition, see below)
/// /bin/sh  → f is a function          (then the definition)
/// ksh93    → f is a function
/// mksh     → f is a function
/// dash     → f is a shell function
/// ash      → f is a shell function
/// ```
pub fn function_whence_verbose(name: &str) -> Option<String> {
    if !posix_faithful() {
        return None;
    }
    if dash_strict() {
        Some(format!("{} is a shell function", name))
    } else {
        Some(format!("{} is a function", name))
    }
}

/// True when the emulated shell follows its `type` / `command -V` header line
/// with the function's DEFINITION. bash does; ksh93, mksh, dash and ash do not:
///
/// ```text
/// $ bash -c 'f() { :; }; type f'      $ ksh -c 'f() { :; }; type f'
/// f is a function                     f is a function
/// f ()
/// {
///     :
/// }
/// ```
///
/// `/bin/sh` is bash on this platform and behaves the same, so it is included.
#[inline]
pub fn function_whence_prints_body() -> bool {
    posix_faithful() && !korn_mode() && !dash_strict()
}

/// Private-use stand-in for a backslash-escaped `&` between
/// `strip_replacement_backslashes` and [`bash_patsub`].
const PATSUB_LITERAL_AMP: char = '\u{F8FE}';

/// bash 5.2 `${v/pat/rep}` with `shopt patsub_replacement` (default on): an
/// unescaped `&` in the replacement stands for the matched text and `\&` is a
/// literal `&`. Identity outside `--bash` or with the shopt off.
pub fn bash_patsub(repl: String, matched: &str) -> String {
    if !bash_mode() || bash_shopt_get("patsub_replacement") != Some(true) {
        return repl;
    }
    if !repl.contains('&') && !repl.contains(PATSUB_LITERAL_AMP) {
        return repl;
    }
    let mut out = String::with_capacity(repl.len() + matched.len());
    for c in repl.chars() {
        match c {
            '&' => out.push_str(matched),
            PATSUB_LITERAL_AMP => out.push('&'),
            _ => out.push(c),
        }
    }
    out
}

/// Render a stored function body as bash lists it (`declare -f` / `type`).
pub fn bash_function_listing(name: &str, body_src: Option<&str>) -> String {
    // Stored source re-parsed and deparsed, one statement per line (the same
    // rendering bin_trap uses for a trap body).
    let rendered = body_src
        .filter(|b| !b.is_empty())
        .and_then(|b| crate::ported::exec::parse_string(b, 1))
        .map(|prog| crate::ported::text::getpermtext(Box::new(prog), None, 0))
        .unwrap_or_default();
    bash_function_body(name, &rendered)
}

/// Re-lay a deparsed function body in bash's `type` format.
///
/// bash re-prints the parsed function from its own AST (`make_command_string`):
/// the header is `NAME () ` then `{ ` — each with ONE trailing space —
/// statements are indented four spaces per level, and every line but the last
/// carries a trailing `;`.
///
/// ```text
/// $ bash -c 'f() { echo a; echo b; }; type f' | cat -e
/// f is a function$
/// f () $
/// { $
///     echo a;$
///     echo b$
/// }$
/// ```
///
/// `body_lines` is zsh's own rendering of the body (one statement per line,
/// one leading TAB per nesting level, keywords on their own lines).
/// `BashDeparse` re-lays `if`/`for`/`while`/`until`/`select`/`case`, `( )`,
/// `{ }` and nested function definitions in bash's layout; a body with a
/// shape it does not model (here-documents) falls back to the flat
/// one-line-per-statement layout.
pub fn bash_function_body(name: &str, body_lines: &str) -> String {
    let mut out = format!("{} () \n{{ \n", name);
    match BashDeparse::parse(body_lines) {
        Some(nodes) => out.push_str(&BashDeparse::render(&nodes, 1, true)),
        None => {
            let lines: Vec<&str> = body_lines.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                let depth = line.chars().take_while(|c| *c == '\t').count();
                let text = line.trim_start_matches('\t');
                let last = i + 1 == lines.len();
                out.push_str(&" ".repeat(4 * (depth + 1)));
                out.push_str(text);
                out.push_str(if last { "\n" } else { ";\n" });
            }
        }
    }
    out.push_str("}\n");
    out
}

/// bash expands braces BEFORE parameters (`x=; echo $x{,a}` -> `$x $xa`, both
/// empty and dropped), zsh after (`a`). For a tokenized command word that
/// carries both a brace list and an expansion, run `xpandbraces` on the source
/// text with every opaque span (`$name`, `${..}`, `$(..)`, `` `..` ``, quotes)
/// masked by a placeholder, then restore the spans in each alternative.
/// Returns the alternatives (empty unquoted ones dropped), or `None` when the
/// word needs no pre-splitting.
pub fn bash_presplit_braces(word: &str) -> Option<Vec<String>> {
    use crate::ported::zsh_h::{
        Bnull, Comma, Dnull, Inbrace, Inpar, Inparmath, Outbrace, Outpar, Outparmath, Qstring,
        Qtick, Snull, Stringg, Tick,
    };
    if !bash_mode()
        || !word.contains(Inbrace)
        || !word.contains(Comma)
        || !word.chars().any(|c| matches!(c, Stringg | Qstring | Tick | Qtick))
    {
        return None;
    }
    let ch: Vec<char> = word.chars().collect();
    let mut holes: Vec<String> = Vec::new();
    let mut masked = String::new();
    let mut hole = |text: String, masked: &mut String| {
        masked.push(char::from_u32(0xF0000 + holes.len() as u32).unwrap_or('\u{f0000}'));
        holes.push(text);
    };
    let mut i = 0;
    while i < ch.len() {
        let c = ch[i];
        // End (exclusive) of the span an opener at `i` opens.
        let end = match c {
            Snull | Dnull | Tick | Qtick => {
                (i + 1..ch.len()).find(|&j| ch[j] == c).map_or(ch.len(), |j| j + 1)
            }
            Bnull => (i + 2).min(ch.len()),
            Stringg | Qstring => match ch.get(i + 1) {
                Some(&Inbrace) | Some(&Inpar) | Some(&Inparmath) => {
                    let mut depth = 0i32;
                    let mut j = i + 1;
                    while j < ch.len() {
                        match ch[j] {
                            Inbrace | Inpar => depth += 1,
                            Inparmath => depth += 2,
                            Outbrace | Outpar => depth -= 1,
                            Outparmath => depth -= 2,
                            _ => {}
                        }
                        j += 1;
                        if depth <= 0 {
                            break;
                        }
                    }
                    j
                }
                Some(n) if n.is_ascii_alphanumeric() || *n == '_' => {
                    let mut j = i + 1;
                    while j < ch.len() && (ch[j].is_ascii_alphanumeric() || ch[j] == '_') {
                        j += 1;
                    }
                    j
                }
                Some(_) => i + 2,
                None => i + 1,
            },
            _ => i,
        };
        if end > i {
            hole(ch[i..end].iter().collect(), &mut masked);
            i = end;
        } else {
            masked.push(c);
            i += 1;
        }
    }
    let parts = crate::ported::glob::xpandbraces(&masked, false);
    if parts.len() == 1 && parts[0] == masked {
        return None;
    }
    Some(
        parts
            .into_iter()
            .filter(|p| !p.is_empty())
            .map(|p| {
                p.chars()
                    .map(|c| match (c as u32).checked_sub(0xF0000) {
                        Some(k) if (k as usize) < holes.len() => holes[k as usize].clone(),
                        _ => c.to_string(),
                    })
                    .collect()
            })
            .collect(),
    )
}

/// One command of a function body, as bash's `print_cmd.c` lays it out.
enum BashNode {
    Simple(String),
    If { cond: String, then: Vec<BashNode>, els: Option<Vec<BashNode>>, closer: String },
    Loop { kw: &'static str, cond: String, body: Vec<BashNode>, closer: String },
    For { head: String, body: Vec<BashNode>, closer: String },
    Case { subject: String, arms: Vec<(String, Vec<BashNode>, String)>, closer: String },
    Subshell { body: Vec<BashNode>, closer: String },
    Group { body: Vec<BashNode>, closer: String },
    Function { name: String, body: Vec<BashNode> },
}

/// Re-lays zsh's deparsed function body (one command per line, keywords on
/// their own lines) in bash's layout: `if C; then` on one line, `for`/`do`
/// split, `case` arms with `;;` on its own line, `( a; b )` subshells.
/// Parses the keyword structure of the zsh text; any shape it does not
/// recognise yields `None` and the caller falls back to the flat layout.
struct BashDeparse {
    lines: Vec<String>,
    pos: usize,
}

const ARM_END: char = '\u{1}';

impl BashDeparse {
    fn parse(body_lines: &str) -> Option<Vec<BashNode>> {
        let mut lines: Vec<String> = Vec::new();
        for raw in body_lines.lines() {
            let t = raw.trim();
            if t.contains("<<") || (t.ends_with('{') && t != "{" && !t.ends_with(" () {")) {
                return None;
            }
            match [" ;;&", " ;&", " ;;"].iter().find(|e| t.ends_with(**e)) {
                Some(e) => {
                    lines.push(t[..t.len() - e.len()].to_string());
                    lines.push(format!("{ARM_END}{}", e.trim()));
                }
                None => lines.push(t.to_string()),
            }
        }
        let mut p = BashDeparse { lines, pos: 0 };
        let nodes = p.block()?;
        (p.pos == p.lines.len()).then_some(nodes)
    }

    fn peek(&self) -> Option<&str> {
        self.lines.get(self.pos).map(String::as_str)
    }

    fn first_word(line: &str) -> &str {
        line.split_whitespace().next().unwrap_or("")
    }

    fn is_stop(line: &str) -> bool {
        line.starts_with(ARM_END)
            || line.starts_with('}')
            || line.starts_with(')')
            || matches!(Self::first_word(line), "fi" | "done" | "esac" | "else" | "elif" | "then" | "do")
    }

    /// Statements up to (not including) the next stop line.
    fn block(&mut self) -> Option<Vec<BashNode>> {
        let mut out = Vec::new();
        while let Some(line) = self.peek() {
            if Self::is_stop(line) {
                break;
            }
            out.push(self.node()?);
        }
        Some(out)
    }

    fn expect_word(&mut self, w: &str) -> Option<()> {
        (self.peek()? == w).then(|| self.pos += 1)
    }

    /// A closer line (`fi`, `done > f`, `}`, `)`): returns its trailing redirections.
    fn closer(&mut self, word: &str) -> Option<String> {
        let line = self.peek()?.to_string();
        let rest = line.strip_prefix(word)?;
        if !rest.is_empty() && !rest.starts_with(' ') {
            return None;
        }
        self.pos += 1;
        Some(rest.to_string())
    }

    fn node(&mut self) -> Option<BashNode> {
        let line = self.peek()?.to_string();
        self.pos += 1;
        if let Some(cond) = line.strip_prefix("if ") {
            return self.if_rest(cond.to_string());
        }
        for kw in ["while", "until"] {
            if let Some(cond) = line.strip_prefix(&format!("{kw} ")) {
                self.expect_word("do")?;
                let body = self.block()?;
                let closer = self.closer("done")?;
                return Some(BashNode::Loop {
                    kw: if kw == "while" { "while" } else { "until" },
                    cond: cond.to_string(),
                    body,
                    closer,
                });
            }
        }
        if line.starts_with("for ((") && line.ends_with(" do") {
            let body = self.block()?;
            let closer = self.closer("done")?;
            return Some(BashNode::For { head: line[..line.len() - 3].to_string(), body, closer });
        }
        if line.starts_with("for ") || line.starts_with("select ") {
            self.expect_word("do")?;
            let body = self.block()?;
            let closer = self.closer("done")?;
            return Some(BashNode::For { head: line, body, closer });
        }
        if let Some(rest) = line.strip_prefix("case ") {
            let subject = rest.strip_suffix(" in")?.to_string();
            let mut arms = Vec::new();
            while self.peek()?.starts_with('(') {
                let arm = self.peek()?.to_string();
                let close = arm.find(") ").or_else(|| arm.ends_with(')').then(|| arm.len() - 1))?;
                let pat = arm[1..close].to_string();
                let first = arm[close + 1..].trim().to_string();
                if first.is_empty() {
                    self.pos += 1;
                } else {
                    self.lines[self.pos] = first;
                }
                let body = self.block()?;
                let term = self.peek()?.strip_prefix(ARM_END)?.to_string();
                self.pos += 1;
                arms.push((pat, body, term));
            }
            let closer = self.closer("esac")?;
            return Some(BashNode::Case { subject, arms, closer });
        }
        if let Some(name) = line.strip_suffix(" () {") {
            let body = self.block()?;
            self.closer("}")?;
            return Some(BashNode::Function { name: name.to_string(), body });
        }
        if line == "(" || line == "{" {
            let body = self.block()?;
            let closer = self.closer(if line == "(" { ")" } else { "}" })?;
            return Some(if line == "(" {
                BashNode::Subshell { body, closer }
            } else {
                BashNode::Group { body, closer }
            });
        }
        Some(BashNode::Simple(line))
    }

    fn if_rest(&mut self, cond: String) -> Option<BashNode> {
        self.expect_word("then")?;
        let then = self.block()?;
        let mut els = None;
        match self.peek() {
            Some(l) if l.starts_with("elif ") => {
                // bash prints `elif` as `else` plus a nested `if`.
                let nested = l["elif ".len()..].to_string();
                self.pos += 1;
                els = Some(vec![self.if_rest(nested)?]);
                return Some(BashNode::If { cond, then, els, closer: String::new() });
            }
            Some("else") => {
                self.pos += 1;
                els = Some(self.block()?);
            }
            _ => {}
        }
        let closer = self.closer("fi")?;
        Some(BashNode::If { cond, then, els, closer })
    }

    /// Lay `nodes` out at `indent` levels; `bare_last` drops the `;` after
    /// the final command (function, `{ }`, subshell and case-arm bodies).
    fn render(nodes: &[BashNode], indent: usize, bare_last: bool) -> String {
        let mut out = String::new();
        for (i, n) in nodes.iter().enumerate() {
            let term = if bare_last && i + 1 == nodes.len() { "" } else { ";" };
            out.push_str(&Self::render_one(n, indent, term));
        }
        out
    }

    fn render_one(n: &BashNode, indent: usize, term: &str) -> String {
        let ind = "    ".repeat(indent);
        match n {
            BashNode::Simple(s) => {
                let term = if s.ends_with('&') { "" } else { term };
                format!("{ind}{s}{term}\n")
            }
            BashNode::If { cond, then, els, closer } => {
                let mut o = format!("{ind}if {cond}; then\n{}", Self::render(then, indent + 1, false));
                if let Some(e) = els {
                    o.push_str(&format!("{ind}else\n{}", Self::render(e, indent + 1, false)));
                }
                o.push_str(&format!("{ind}fi{closer}{term}\n"));
                o
            }
            BashNode::Loop { kw, cond, body, closer } => format!(
                "{ind}{kw} {cond}; do\n{}{ind}done{closer}{term}\n",
                Self::render(body, indent + 1, false)
            ),
            BashNode::For { head, body, closer } => {
                let semi = if head.starts_with("for ((") { "" } else { ";" };
                format!(
                    "{ind}{head}{semi}\n{ind}do\n{}{ind}done{closer}{term}\n",
                    Self::render(body, indent + 1, false)
                )
            }
            BashNode::Case { subject, arms, closer } => {
                let mut o = format!("{ind}case {subject} in \n");
                for (pat, body, t) in arms {
                    o.push_str(&format!(
                        "{ind}    {pat})\n{}{ind}    {t}\n",
                        Self::render(body, indent + 2, true)
                    ));
                }
                o.push_str(&format!("{ind}esac{closer}{term}\n"));
                o
            }
            BashNode::Subshell { body, closer } => {
                let inner = Self::render(body, indent, true);
                let inner = inner.trim_end_matches('\n');
                let inner = inner.strip_prefix(ind.as_str()).unwrap_or(inner);
                format!("{ind}( {inner} ){closer}{term}\n")
            }
            BashNode::Function { name, body } => format!(
                "{ind}function {name} () \n{ind}{{ \n{}{ind}}}{term}\n",
                Self::render(body, indent + 1, true)
            ),
            BashNode::Group { body, closer } => {
                format!("{ind}{{ \n{}{ind}}}{closer}{term}\n", Self::render(body, indent + 1, true))
            }
        }
    }
}

/// True when replacing the positional parameters (`set -- …`) must also
/// rewind `getopts`, and when a write to `$OPTIND` must NOT.
///
/// dash and ash key `getopts` off `shellparam.optind` / `shellparam.optoff`,
/// which `setparam()` resets and an `$OPTIND` assignment does not reach — the
/// exact opposite of bash and the Korn shells:
///
/// ```text
/// $ <shell> -c 'getopts "ab" o -a; set -- -b -a; getopts "ab" o;
///               printf "%s %s\n" "$o" "$OPTIND"'
/// dash/ash → b 2        bash/ksh/mksh → a 3        zsh → a 2
/// ```
///
/// Only the `set --` half is acted on here; the `$OPTIND`-write half stays a
/// pinned gap (see the ignored parity test) because zsh routes the parse
/// index through the parameter itself.
#[inline]
pub fn getopts_reset_on_set_positional() -> bool {
    posix_faithful() && dash_strict()
}

/// Rewind the `getopts` cursor recorded by [`getopts_optind_report`], so a
/// later `$OPTIND` read is not translated against a stale pair.
pub fn getopts_forget_reported() {
    GETOPTS_REPORTED.store(-1, Ordering::Relaxed);
    GETOPTS_INTERNAL.store(-1, Ordering::Relaxed);
}

// ===========================================================================
// bash `${parameter@operator}` parameter transformations
// ===========================================================================
//
// !!! WARNING: RUST-ONLY HELPER !!!
//
// Nothing below has a zsh C counterpart, so nothing below carries a `// c:NNN`
// citation.  `${var@Q}`, `${var@A}`, `${var@K}`, `${var@k}`, `${var@a}`,
// `${var@E}` and the `@U` / `@L` / `@u` case operators are bash extensions;
// zsh answers "bad substitution" for every one of them:
//
// ```text
// $ zsh -fc 'v=abc; echo "${v@A}"'
// zsh:1: bad substitution
// ```
//
// THE REFERENCE IS BASH ITSELF (GNU bash 5.3.15), not `Src/subst.c`.  Every
// rule encoded here was established by running bash and recording its output;
// the transcripts are pinned as parity tests in
// `tests/parity/bash_param_transform.rs`.  Callers must gate on
// [`bash_mode`] so `--zsh` keeps answering "bad substitution".

/// bash's `isprint()` test, as `ansic_shouldquote` (lib/sh/strtrans.c) uses
/// it: C0 controls and DEL are unprintable, everything else — including
/// non-ASCII — is printable.  `${v@Q}` on `héllo` answers `'héllo'`, not a
/// `$'…'` byte dump, which is what fixes the boundary here.
fn bash_isprint(c: char) -> bool {
    let u = c as u32;
    !(u < 0x20 || u == 0x7f)
}

/// bash `ansic_shouldquote` — true when the value holds a character that
/// cannot survive `'…'` quoting and so forces the `$'…'` form.
pub fn bash_ansic_shouldquote(s: &str) -> bool {
    s.chars().any(|c| !bash_isprint(c))
}

/// bash `ansic_quote` — render `s` as a `$'…'` literal.
///
/// ```text
/// $ bash -c $'v=$\'\\a\\v\\b\\f\\n\\r\\t\\e\\001\\177\'; printf "<%s>" "${v@Q}"'
/// <$'\a\v\b\f\n\r\t\E\001\177'>
/// ```
pub fn bash_ansic_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    out.push_str("$'");
    for c in s.chars() {
        match c {
            '\u{7}' => out.push_str("\\a"),
            '\u{8}' => out.push_str("\\b"),
            '\u{b}' => out.push_str("\\v"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{1b}' => out.push_str("\\E"),
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            _ if bash_isprint(c) => out.push(c),
            // Unprintable and not one of the named escapes: bash emits one
            // three-digit octal escape per BYTE (`\001`, `\177`).
            _ => {
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).as_bytes() {
                    out.push_str(&format!("\\{:03o}", b));
                }
            }
        }
    }
    out.push('\'');
    out
}

/// bash `sh_single_quote` — `'…'`, with an embedded `'` written `'\''`.
///
/// ```text
/// $ bash -c $'v="it\'s"; printf "<%s>" "${v@Q}"'
/// <'it'\''s'>
/// ```
pub fn bash_single_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// bash `sh_double_quote` — `"…"`, backslash-escaping only the four
/// characters that stay live inside double quotes (`CBSDQUOTE`).
pub fn bash_double_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if matches!(c, '"' | '\\' | '$' | '`') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// bash `sh_quote_reusable` — what `${v@Q}` and the SCALAR form of `${v@A}`
/// emit: `$'…'` when the value needs it, otherwise `'…'`.
pub fn bash_quote_reusable(s: &str) -> String {
    if bash_ansic_shouldquote(s) {
        bash_ansic_quote(s)
    } else {
        bash_single_quote(s)
    }
}

/// The form bash uses for one ELEMENT inside a `name=(…)` assignment or a
/// `@K` key/value pair: `$'…'` when unprintable, otherwise DOUBLE quotes.
///
/// ```text
/// $ bash -c 'a=(x "y z"); printf "<%s>" "${a[*]@K}"'
/// <0 "x" 1 "y z">
/// ```
pub fn bash_quote_element(s: &str) -> String {
    if bash_ansic_shouldquote(s) {
        bash_ansic_quote(s)
    } else {
        bash_double_quote(s)
    }
}

/// bash `sh_contains_shell_metas` — decides whether an associative-array KEY
/// has to be quoted inside `@A` / `@K` output.  Measured char-by-char against
/// bash 5.3: `- . + , : / @ % = # ~` stay bare mid-word, the rest quote.
pub fn bash_contains_shell_metas(s: &str) -> bool {
    let b: Vec<char> = s.chars().collect();
    for (i, &c) in b.iter().enumerate() {
        match c {
            ' ' | '\t' | '\n' => return true,
            '\'' | '"' | '\\' => return true,
            '|' | '&' | ';' => return true,
            '(' | ')' | '<' | '>' => return true,
            '!' | '{' | '}' => return true,
            '*' | '[' | '?' | ']' => return true,
            '^' => return true,
            '$' | '`' => return true,
            // Tilde expansion only fires at the start of a word or right
            // after `=` / `:`, so a mid-word `~` needs no quoting.
            '~' => {
                if i == 0 || b[i - 1] == '=' || b[i - 1] == ':' {
                    return true;
                }
            }
            // A comment only starts a word.
            '#' => {
                if i == 0 {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// The form bash uses for an associative-array key inside `@A` / `@K`.
pub fn bash_quote_assoc_key(s: &str) -> String {
    if bash_ansic_shouldquote(s) {
        bash_ansic_quote(s)
    } else if bash_contains_shell_metas(s) {
        bash_double_quote(s)
    } else {
        s.to_string()
    }
}

/// The shape a `${var@A}` / `@K` / `@k` transformation is looking at.
///
/// `Indexed` carries the LIVE indices, so a sparse bash array recreates as
/// `declare -a a=([0]="x" [5]="z")` rather than renumbering its holes away.
pub enum BashParamShape<'a> {
    /// The reference resolved to nothing at all.
    Unset,
    /// A scalar, or an array/assoc reduced to one element by a subscript.
    Scalar(&'a str),
    /// A whole indexed array: `(index, value)` in index order.
    Indexed(&'a [(usize, String)]),
    /// A whole associative array: `(key, value)`.
    Assoc(&'a [(String, String)]),
}

/// bash `${var@A}` — the assignment statement that would recreate the
/// parameter with its attributes.
///
/// `attrs` is the attribute-letter string [`bash_attr_letters`] produces; an
/// empty one means bash omits the `declare` word entirely.
///
/// ```text
/// $ bash -c 'v=abc;          printf "<%s>" "${v@A}"'    → <v='abc'>
/// $ bash -c 'declare -i n=5; printf "<%s>" "${n@A}"'    → <declare -i n='5'>
/// $ bash -c 'a=(x "y z");    printf "<%s>" "${a[*]@A}"' → <declare -a a=([0]="x" [1]="y z")>
/// $ bash -c 'declare -A h=([q]=1); printf "<%s>" "${h[*]@A}"'
///                                                       → <declare -A h=([q]="1" )>
/// ```
pub fn bash_assignment_string(name: &str, attrs: &str, shape: &BashParamShape) -> String {
    let decl = if attrs.is_empty() {
        String::new()
    } else {
        format!("declare -{} ", attrs)
    };
    match shape {
        // An unset reference recreates as a bare declaration with no `=`.
        BashParamShape::Unset => format!("{}{}", decl, name),
        BashParamShape::Scalar(v) => format!("{}{}={}", decl, name, bash_quote_reusable(v)),
        BashParamShape::Indexed(items) => {
            let body = items
                .iter()
                .map(|(i, v)| format!("[{}]={}", i, bash_quote_element(v)))
                .collect::<Vec<_>>()
                .join(" ");
            format!("{}{}=({})", decl, name, body)
        }
        // bash's assoc writer emits a trailing space after EVERY pair, so the
        // body ends `… )` — reproduced verbatim.  Keys are bracketed here,
        // unlike in the `@K` pair form.
        BashParamShape::Assoc(items) => {
            let mut body = String::new();
            for (k, v) in items.iter() {
                body.push('[');
                body.push_str(&bash_quote_assoc_key(k));
                body.push_str("]=");
                body.push_str(&bash_quote_element(v));
                body.push(' ');
            }
            format!("{}{}=({})", decl, name, body)
        }
    }
}

/// bash `${var@K}` — like `@Q` for a scalar; for an array, `key "value"`
/// pairs in ONE word.
///
/// ```text
/// $ bash -c 'a=(x "y z");          printf "<%s>" "${a[@]@K}"' → <0 "x" 1 "y z">
/// $ bash -c 'declare -A h=([q]=1); printf "<%s>" "${h[*]@K}"' → <q "1" >
/// ```
pub fn bash_kvpair_string(shape: &BashParamShape) -> String {
    match shape {
        BashParamShape::Unset => String::new(),
        BashParamShape::Scalar(v) => bash_quote_reusable(v),
        BashParamShape::Indexed(items) => items
            .iter()
            .map(|(i, v)| format!("{} {}", i, bash_quote_element(v)))
            .collect::<Vec<_>>()
            .join(" "),
        BashParamShape::Assoc(items) => {
            let mut out = String::new();
            for (k, v) in items.iter() {
                out.push_str(&bash_quote_assoc_key(k));
                out.push(' ');
                out.push_str(&bash_quote_element(v));
                out.push(' ');
            }
            out
        }
    }
}

/// bash `${var@k}` — the `@K` pairs with NO quoting, each key and each value
/// its own word.
///
/// ```text
/// $ bash -c 'a=(x "y z"); printf "<%s>" "${a[@]@k}"' → <0><x><1><y z>
/// ```
pub fn bash_kv_words(shape: &BashParamShape) -> Vec<String> {
    match shape {
        BashParamShape::Unset => Vec::new(),
        BashParamShape::Scalar(v) => vec![bash_quote_reusable(v)],
        BashParamShape::Indexed(items) => items
            .iter()
            .flat_map(|(i, v)| [i.to_string(), v.clone()])
            .collect(),
        BashParamShape::Assoc(items) => items
            .iter()
            .flat_map(|(k, v)| [k.clone(), v.clone()])
            .collect(),
    }
}

/// Split a `${var[@]@A}` result into words the way bash does.
///
/// bash hands the assignment statement back through IFS field splitting, but
/// with quote regions skipped, so `declare -a a=([0]="x" [1]="y z")` becomes
/// three words and the parenthesised body stays whole.  Measured:
///
/// ```text
/// $ bash -c 'a=(x "y z");            printf "<%s>" "${a[@]@A}"'
/// <declare><-a><a=([0]="x" [1]="y z")>
/// $ bash -c 'a=(x "y z"); IFS=":";   printf "<%s>" "${a[@]@A}"'
/// <declare -a a=([0]="x" [1]="y z")>
/// $ bash -c 'a=(x "y z"); IFS="a";   printf "<%s>" "${a[@]@A}"'
/// <decl><re ->< ><=([0]="x" [1]="y z")>
/// ```
///
/// An IFS that is unset OR empty falls back to `" \t\n"` — the empty case is
/// bash's own, and is why `IFS=""` above still splits.
///
/// A parenthesised region is protected too, which is what keeps the whole
/// `a=(…)` body one word even though its interior is full of spaces:
///
/// ```text
/// $ bash -c 'a=(x y); IFS="0"; printf "<%s>" "${a[@]@A}"'
/// <declare -a a=([0]="x" [1]="y")>          # the `0`s live inside the parens
/// $ bash -c 'a=(x y); IFS="[";  printf "<%s>" "${a[@]@A}"'
/// <declare -a a=([0]="x" [1]="y")>
/// ```
pub fn bash_split_ifs_quote_aware(s: &str, ifs: Option<&str>) -> Vec<String> {
    let seps: Vec<char> = match ifs {
        Some(v) if !v.is_empty() => v.chars().collect(),
        _ => vec![' ', '\t', '\n'],
    };
    let is_sep = |c: char| seps.contains(&c);
    let is_ws_sep = |c: char| is_sep(c) && (c == ' ' || c == '\t' || c == '\n');

    let cs: Vec<char> = s.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut started = false;
    let mut depth = 0usize; // unquoted `(` … `)` nesting — never split inside
    let mut i = 0usize;
    while i < cs.len() {
        let c = cs[i];
        if c == '(' {
            depth += 1;
            cur.push(c);
            started = true;
            i += 1;
            continue;
        }
        if c == ')' {
            depth = depth.saturating_sub(1);
            cur.push(c);
            started = true;
            i += 1;
            continue;
        }
        if c == '\'' {
            // Single-quoted region (also the tail of `$'…'`): nothing inside
            // it splits and no backslash ends it early.
            cur.push(c);
            started = true;
            i += 1;
            while i < cs.len() && cs[i] != '\'' {
                cur.push(cs[i]);
                i += 1;
            }
            if i < cs.len() {
                cur.push('\'');
                i += 1;
            }
            continue;
        }
        if c == '"' {
            cur.push(c);
            started = true;
            i += 1;
            while i < cs.len() && cs[i] != '"' {
                if cs[i] == '\\' && i + 1 < cs.len() {
                    cur.push(cs[i]);
                    i += 1;
                }
                cur.push(cs[i]);
                i += 1;
            }
            if i < cs.len() {
                cur.push('"');
                i += 1;
            }
            continue;
        }
        if c == '\\' && i + 1 < cs.len() {
            cur.push(c);
            cur.push(cs[i + 1]);
            started = true;
            i += 2;
            continue;
        }
        if is_sep(c) && depth == 0 {
            if started {
                out.push(std::mem::take(&mut cur));
                started = false;
            }
            if is_ws_sep(c) {
                // Runs of IFS whitespace collapse into one separator.
                while i + 1 < cs.len() && is_ws_sep(cs[i + 1]) {
                    i += 1;
                }
            }
            i += 1;
            continue;
        }
        cur.push(c);
        started = true;
        i += 1;
    }
    if started {
        out.push(cur);
    }
    out
}

/// bash `${var@a}` — the parameter's attribute flags as a letter string, and
/// the same letters `${var@A}` puts after `declare -`.
///
/// The ORDER is bash's, measured:
///
/// ```text
/// $ bash -c 'declare -air z=(1 2); printf "<%s>" "${z@a}"'  → <air>
/// $ bash -c 'declare -xi  q=3;     printf "<%s>" "${q@a}"'  → <ix>
/// $ bash -c 'declare -A   h; h[x]=1; printf "<%s>" "${h[@]@a}"' → <A>
/// $ bash -c 'v=abc;                printf "<%s>" "${v@a}"'  → <>
/// ```
pub fn bash_attr_letters(flags: u32) -> String {
    use crate::ported::zsh_h::{
        PM_ARRAY, PM_EXPORTED, PM_HASHED, PM_INTEGER, PM_LOWER, PM_READONLY, PM_UPPER,
    };
    let mut attrs = String::new();
    if flags & PM_HASHED != 0 {
        attrs.push('A');
    } else if flags & PM_ARRAY != 0 {
        attrs.push('a');
    }
    if flags & PM_INTEGER != 0 {
        attrs.push('i');
    }
    if flags & PM_READONLY != 0 {
        attrs.push('r');
    }
    if flags & PM_EXPORTED != 0 {
        attrs.push('x');
    }
    if flags & PM_UPPER != 0 {
        attrs.push('u');
    }
    if flags & PM_LOWER != 0 {
        attrs.push('l');
    }
    attrs
}

/// Rebuild the scalar `val` after a bash `${var@OP}` transformation ran
/// ELEMENT-WISE over an array reference.
///
/// A `"${a[*]…}"` reference was already collapsed to one word before the
/// transform ran, so its result has to be re-joined with the same separator
/// (`$IFS[0]`, or an explicit `(j:X:)`); a `"${a[@]…}"` one still splats from
/// `split_parts` and only needs a placeholder join, which mirrors what the
/// `casmod` arm in `subst.rs` does.
///
/// ```text
/// $ bash -c 'a=(x "y z"); IFS=":"; printf "<%s>" "${a[*]@Q}"'  → <'x':'y z'>
/// ```
pub fn bash_rejoin_elems(parts: &[String], dq_collapsed: bool, sep: Option<&str>) -> String {
    if dq_collapsed {
        crate::ported::utils::sepjoin(parts, sep)
    } else {
        parts.join(" ")
    }
}

/// The raw `PM_*` flag word of a named parameter, or 0 when it has none.
/// Feeds [`bash_attr_letters`] for `${v@a}` and `${v@A}`.
pub fn bash_param_flags(name: &str) -> u32 {
    crate::ported::params::paramtab()
        .read()
        .ok()
        .and_then(|t| t.get(name).map(|p| p.node.flags as u32))
        .unwrap_or(0)
}

/// ksh93's `printf %q` quoting of one argument.
///
/// !!! EMULATION-ONLY (no C counterpart) !!! zsh backslash-escapes
/// (`a\ b`); ksh93u+m wraps in single quotes (`'a b'`), switches to the
/// ANSI-C `$'…'` form when the argument holds a control character or an
/// apostrophe, and leaves `a=b`, `a!b`, `a,b`, `a%b`, `a+b`, `a@b`, `a^b`
/// and non-ASCII text bare. Measured against ksh93u+m 1.0.10 with every
/// ASCII punctuation character between two letters, plus leading `=`.
pub fn ksh93_printf_q(arg: &str) -> String {
    if arg.is_empty() {
        return "''".to_string();
    }
    if arg.chars().any(|c| c.is_control()) || arg.contains('\'') {
        let mut out = String::from("$'");
        for c in arg.chars() {
            match c {
                '\\' => out.push_str("\\\\"),
                '\'' => out.push_str("\\'"),
                '\t' => out.push_str("\\t"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                c if c.is_control() => out.push_str(&format!("\\x{:02x}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('\'');
        return out;
    }
    const SPECIAL: &str = " \"#$&()*;<>?[\\]`{|}~";
    if arg.starts_with('=') || arg.chars().any(|c| SPECIAL.contains(c)) {
        return format!("'{arg}'");
    }
    arg.to_string()
}

/// ksh93 `print` / `echo -e` backslash escapes, as measured on
/// /opt/homebrew/bin/ksh (93u+m). The set is narrower than zsh's
/// GETKEYS_PRINT/ECHO: `\a \b \E \f \n \r \t \v \\`, `\0` plus up to three
/// octal digits, and `\u` plus exactly four hex digits. `\c` ends the output
/// (the returned flag is true: drop the rest and the newline). Everything
/// else — `\e`, `\x41`, `\101`, `\1`, `\U…`, `\"`, `\q`, a trailing `\` —
/// keeps its backslash. Bytes >= 0x80 are metafied like getkeystring's.
///
/// !!! RUST-ONLY EXTENSION — no zsh C counterpart !!!
pub fn ksh93_print_escapes(s: &str) -> (String, bool) {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c != '\\' || i + 1 >= chars.len() {
            out.push(c);
            i += 1;
            continue;
        }
        let simple = match chars[i + 1] {
            'a' => Some('\x07'),
            'b' => Some('\x08'),
            'E' => Some('\x1b'),
            'f' => Some('\x0c'),
            'n' => Some('\n'),
            'r' => Some('\r'),
            't' => Some('\t'),
            'v' => Some('\x0b'),
            '\\' => Some('\\'),
            _ => None,
        };
        if let Some(ch) = simple {
            out.push(ch);
            i += 2;
        } else if chars[i + 1] == 'c' {
            return (out, true);
        } else if chars[i + 1] == '0' {
            let digits: Vec<u32> = chars[i + 2..]
                .iter()
                .take(3)
                .map_while(|d| d.to_digit(8))
                .collect();
            let byte = (digits.iter().fold(0, |acc, d| acc * 8 + d) & 0xff) as u8;
            if byte < 0x80 {
                out.push(byte as char);
            } else {
                out.push('\u{83}'); // Meta
                out.push(char::from(byte ^ 32));
            }
            i += 2 + digits.len();
        } else if chars[i + 1] == 'u'
            && chars.len() >= i + 6
            && chars[i + 2..i + 6].iter().all(|d| d.is_ascii_hexdigit())
        {
            let hex: String = chars[i + 2..i + 6].iter().collect();
            out.push(char::from_u32(u32::from_str_radix(&hex, 16).unwrap_or(0xfffd)).unwrap_or('\u{fffd}'));
            i += 6;
        } else {
            out.push('\\');
            i += 1;
        }
    }
    (out, false)
}

/// Names currently defined with the ksh `function name { … }` keyword (as
/// opposed to the POSIX `name() { … }` spelling). Written at definition time
/// by [`note_function_spelling`].
static KEYWORD_FUNCS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Record which spelling defined `name`; a redefinition replaces the old entry.
///
/// !!! RUST-ONLY EXTENSION — no zsh C counterpart !!!
pub fn note_function_spelling(name: &str, keyword: bool) {
    let mut names = KEYWORD_FUNCS.lock().unwrap_or_else(|e| e.into_inner());
    names.retain(|n| n != name);
    if keyword {
        names.push(name.to_string());
    }
}

/// True while the innermost running shell function is a ksh93 `function name
/// { … }` definition. ksh93 gives only that spelling its own EXIT-trap scope
/// (the trap fires when the function returns); a `name() { … }` function's
/// EXIT trap is the shell's and fires at shell exit — which is zsh's
/// POSIX_TRAPS behavior, so the keyword spelling is the one to special-case.
///
/// !!! RUST-ONLY EXTENSION — no zsh C counterpart !!!
pub fn in_ksh93_keyword_function() -> bool {
    ksh93_mode() && innermost_function_is_keyword_spelled() == Some(true)
}

/// True while the innermost running shell function is a POSIX-spelled
/// `name() { … }` definition under `--ksh`. ksh93 gives that spelling no
/// local scope: `f() { typeset x=1; }; f; print $x` prints `1`, so `typeset`
/// and its aliases act as `typeset -g` there (the `function name { … }`
/// spelling keeps zsh's local scope).
///
/// !!! RUST-ONLY EXTENSION — no zsh C counterpart !!!
pub fn in_ksh93_posix_function() -> bool {
    ksh93_mode() && innermost_function_is_keyword_spelled() == Some(false)
}

/// `Some(true)` when the innermost running function was defined with the
/// `function` keyword, `Some(false)` for `name()`, `None` outside a function.
fn innermost_function_is_keyword_spelled() -> Option<bool> {
    let top = crate::ported::modules::parameter::FUNCSTACK
        .lock()
        .ok()
        .and_then(|stk| {
            stk.last()
                .filter(|f| f.tp == crate::ported::zsh_h::FS_FUNC)
                .map(|f| f.name.clone())
        });
    top.map(|name| {
        KEYWORD_FUNCS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&name)
    })
}

/// Scope guard for a bash command substitution body.
///
/// bash(1) `set -e`: "Subshells spawned to execute command substitutions
/// inherit the value of the -e option from the parent shell. When not in
/// posix mode, bash clears the -e option in such subshells" — unless
/// `shopt -s inherit_errexit`. So `set -e; echo $(false; echo hi)` prints
/// `hi`. zsh (and ksh) keep ERREXIT inside the substitution. Restores the
/// option on drop.
///
/// !!! RUST-ONLY EXTENSION — no zsh C counterpart !!!
pub struct BashCmdsubstErrexit {
    armed: bool,
}

impl BashCmdsubstErrexit {
    pub fn enter() -> Self {
        use crate::ported::zsh_h::{isset, ERREXIT};
        let armed = bash_mode()
            && isset(ERREXIT)
            && !bash_shopt_get("inherit_errexit").unwrap_or(false);
        if armed {
            crate::ported::options::dosetopt(ERREXIT, 0, 1);
        }
        Self { armed }
    }
}

impl Drop for BashCmdsubstErrexit {
    fn drop(&mut self) {
        if self.armed {
            crate::ported::options::dosetopt(crate::ported::zsh_h::ERREXIT, 1, 1);
        }
    }
}

/// C `oldpwd` global (Src/params.c:75). `cd -` and `~-` read THIS, not the
/// `$OLDPWD` parameter: assigning `OLDPWD=x` leaves it alone (Src/builtin.c:909
/// `: oldpwd`, Src/subst.c:755). `None` until startup or the first `cd` sets it.
static OLDPWD_GLOBAL: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// `oldpwd = <val>` (c:Src/builtin.c:1238-1239, c:Src/init.c:1255-1259).
pub fn set_oldpwd_global(val: &str) {
    *OLDPWD_GLOBAL.lock().unwrap_or_else(|e| e.into_inner()) = Some(val.to_string());
}

/// What `cd -` / `~-` resolve to. zsh reads the `oldpwd` global; the real
/// POSIX-family shells (dash, ksh93, bash) read `$OLDPWD` itself, so an
/// assignment to it redirects `cd -` there.
///
/// !!! RUST-ONLY ADAPTER — the posix_faithful branch has no zsh C counterpart !!!
pub fn oldpwd_cd_target() -> Option<String> {
    let param = || crate::ported::params::getsparam("OLDPWD");
    if posix_faithful() {
        return param();
    }
    OLDPWD_GLOBAL
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .or_else(param)
}

/// True when a blank at bracket depth `brct` must NOT end the token: the word
/// so far (`lexbuf`) is `NAME[` in command position (`incmdpos`) and the shell
/// is bash or a Korn drop-in, whose assignments take a blank-bearing subscript
/// (`h[a b]=1`, `a[1 + 1]=x`). zsh itself splits the word at the blank, so
/// every other personality keeps Src/lex.c:958.
///
/// !!! RUST-ONLY EXTENSION — no zsh C counterpart !!!
pub fn korn_subscript_blank(brct: i32, incmdpos: bool, lexbuf: &str) -> bool {
    if brct <= 0 || !incmdpos || !(bash_mode() || korn_mode()) {
        return false;
    }
    let name_len = lexbuf
        .find(|c: char| !(c == '_' || c.is_ascii_alphanumeric()))
        .unwrap_or(lexbuf.len());
    name_len > 0
        && !lexbuf.starts_with(|c: char| c.is_ascii_digit())
        && lexbuf[name_len..].starts_with(crate::ported::zsh_h::Inbrack)
}

/// ksh93 `printf %f/%e/%g` rounds an exactly-representable tie away from zero
/// (`printf '%.0f' 0.5 2.5` -> `1 3`, `%.1f 0.25` -> `0.3`), where libc and
/// Rust round the tie to even. Nudge such a tie one ulp away from zero so the
/// ordinary formatter rounds it up; every non-tie and every other mode is
/// returned unchanged. `prec` is the directive's precision (`None` = default).
pub fn ksh93_round_ties_away(n: f64, conv: char, prec: Option<usize>) -> f64 {
    if !ksh93_mode() || n == 0.0 || !n.is_finite() {
        return n;
    }
    let sci = matches!(conv, 'e' | 'E' | 'g' | 'G');
    let p = match conv {
        'g' | 'G' => prec.unwrap_or(6).max(1) - 1,
        _ => prec.unwrap_or(6),
    };
    // Exact decimal expansion with enough extra digits to expose any non-tie.
    let exact = if sci { format!("{:.*e}", p + 40, n) } else { format!("{:.*}", p + 40, n) };
    let mantissa = exact.split('e').next().unwrap_or("");
    let frac = mantissa.split_once('.').map_or("", |(_, f)| f);
    let is_tie = frac.as_bytes().get(p) == Some(&b'5') && frac[p + 1..].bytes().all(|b| b == b'0');
    if is_tie { f64::from_bits(n.to_bits() + 1) } else { n }
}

/// Index of a single-element subscript assignment `a[KEY]=v` for the sparse
/// side-table: a plain number, or (Korn/bash arithmetic subscript) a
/// side-effect-free expression such as `1+1` or a variable name. ksh93 and
/// mksh keep `a[1+1]=x` sparse (`${!a[@]}` -> `2`), same as `a[2]=x`.
/// Expressions with `++`/`--`/`=`/`,`/`?` or any other operator are not
/// evaluated here (a second evaluation would repeat the side effect), and a
/// negative result has no table slot.
pub fn sparse_subscript_index(key: &str) -> Option<usize> {
    let key = key.trim();
    if let Ok(i) = key.parse::<usize>() {
        return Some(i);
    }
    let pure = !key.is_empty()
        && key.chars().all(|c| c.is_ascii_alphanumeric() || " _+-*/%()".contains(c))
        && !key.contains("++")
        && !key.contains("--");
    if !pure {
        return None;
    }
    let v = crate::ported::math::mathevali(key).ok()?;
    usize::try_from(v).ok()
}

/// True for the body of a ksh93 stepped CHARACTER range `{a..e..2}`: two
/// single-character endpoints and a signed integer step. zsh's `hasbraces`
/// (Src/glob.c:2042) only accepts a stepped range when the endpoints are
/// numeric, but ksh93 expands `{a..e..2}` -> `a c e`.
pub fn ksh93_char_step_range(body: &str) -> bool {
    if !ksh93_mode() {
        return false;
    }
    let norm: String = body.chars().map(|c| if c == '\u{e19b}' { '-' } else { c }).collect();
    let parts: Vec<&str> = norm.split("..").collect();
    parts.len() == 3
        && parts[..2].iter().all(|p| p.chars().count() == 1)
        && parts[2].parse::<i64>().is_ok()
}

/// mksh's quoting for `typeset -p` values: bare when every character is
/// alphanumeric, non-ASCII or one of `_-./:,+@%^!{}~`; otherwise POSIX
/// single quotes (`'\''` for an embedded quote); control characters switch to
/// the `$'..'` form. Measured against mksh R59 over the ASCII punctuation set.
pub fn mksh_quote(v: &str) -> String {
    if v.chars().any(|c| c.is_ascii_control()) {
        let mut out = String::from("$'");
        for c in v.chars() {
            match c {
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                '\r' => out.push_str("\\r"),
                '\'' => out.push_str("\\'"),
                '\\' => out.push_str("\\\\"),
                c if c.is_ascii_control() => out.push_str(&format!("\\{:03o}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('\'');
        return out;
    }
    let bare = |c: char| !c.is_ascii() || c.is_ascii_alphanumeric() || "_-./:,+@%^!{}~".contains(c);
    if !v.is_empty() && v.chars().all(bare) {
        v.to_string()
    } else {
        format!("'{}'", v.replace('\'', "'\\''"))
    }
}

/// Lines of mksh's `typeset -p NAME` for a SET scalar or indexed array.
/// Attribute words come in mksh's fixed order `-i -x -r -t -L|-R -Z -l|-u`
/// (width/base glued to `-L`/`-R`; a base-N integer shows its value as
/// `N#digits` under a bare `-i`). Arrays print `set -A NAME` followed by one
/// `typeset NAME[i]=v` per live element, holes skipped.
///
/// !!! RUST-ONLY EXTENSION — no zsh C counterpart !!!
pub fn pdksh_typeset_p(
    name: &str,
    flags: u32,
    width: i32,
    value: &str,
    elems: Option<Vec<(usize, String)>>,
) -> Vec<String> {
    use crate::ported::zsh_h::{
        PM_EXPORTED, PM_INTEGER, PM_LEFT, PM_LOWER, PM_READONLY, PM_RIGHT_B, PM_RIGHT_Z, PM_TAGGED,
        PM_UPPER,
    };
    if let Some(elems) = elems {
        let mut lines = vec![format!("set -A {name}")];
        lines.extend(elems.into_iter().map(|(i, v)| format!("typeset {name}[{i}]={}", mksh_quote(&v))));
        return lines;
    }
    let mut attrs: Vec<String> = Vec::new();
    for (flag, word) in [(PM_INTEGER, "-i"), (PM_EXPORTED, "-x"), (PM_READONLY, "-r"), (PM_TAGGED, "-t")] {
        if flags & flag != 0 {
            attrs.push(word.to_string());
        }
    }
    if flags & PM_LEFT != 0 {
        attrs.push(format!("-L{width}"));
    }
    if flags & (PM_RIGHT_B | PM_RIGHT_Z) != 0 {
        attrs.push(format!("-R{width}"));
    }
    if flags & PM_RIGHT_Z != 0 {
        attrs.push("-Z".to_string());
    }
    for (flag, word) in [(PM_LOWER, "-l"), (PM_UPPER, "-u")] {
        if flags & flag != 0 {
            attrs.push(word.to_string());
        }
    }
    let head = if attrs.is_empty() { "typeset".to_string() } else { format!("typeset {}", attrs.join(" ")) };
    let int_text = |s: &str| {
        let (base, digits) = s.split_once('#').unwrap_or(("", s));
        let digits_ok = |d: &str| !d.is_empty() && d.trim_start_matches('-').bytes().all(|b| b.is_ascii_alphanumeric());
        digits_ok(digits) && (base.is_empty() || base.trim_start_matches('-').bytes().all(|b| b.is_ascii_digit()))
    };
    let shown = if flags & PM_INTEGER != 0 && int_text(value) { value.to_string() } else { mksh_quote(value) };
    vec![format!("{head} {name}={shown}")]
}

/// mksh `${v@Q}`: the `typeset -p` quoting, except a value holding control
/// characters is wrapped in `$'..'` with its bytes left RAW (mksh R59 does not
/// escape them there: a tab stays a tab, a newline stays a newline).
pub fn mksh_at_q(v: &str) -> String {
    if v.chars().any(|c| c.is_ascii_control()) {
        format!("$'{}'", v.replace('\'', "\\'"))
    } else {
        mksh_quote(v)
    }
}

/// `$KSH_VERSION` of the emulated Korn shell: ksh93u+m 1.0.10 for `--ksh`,
/// mksh R59 for the pdksh line (the reference binaries this mode is measured
/// against).
pub fn ksh_version_string() -> &'static str {
    if pdksh_family() {
        "@(#)MIRBSD KSH R59 2020/10/31"
    } else {
        "Version AJM 93u+m/1.0.10 2024-08-01"
    }
}

/// dash's `ulimit` (miscbltin.c `ulimitcmd`): `[-HSa] [-tfdscmlpnv] [limit]`,
/// its own fixed resource table, `%-20s ` listing for `-a`, and "Illegal
/// option" (status 2) for every other letter. zsh's table (`-u`, `-T`, ...)
/// differs, so the bare drop-in does not go through `bin_ulimit`.
///
/// Platform-specific extras dash adds on Linux (`-w -i -q -e -r`) are not
/// carried: only the ten resources common to dash on every host are.
///
/// !!! RUST-ONLY EXTENSION — no zsh C counterpart !!!
#[cfg(unix)]
pub fn dash_ulimit(argv: &[String]) -> i32 {
    // (name, resource, factor, option letter) — dash's `limits[]` order.
    let limits: [(&str, libc::c_int, libc::rlim_t, char); 10] = [
        ("time(seconds)", libc::RLIMIT_CPU as _, 1, 't'),
        ("file(blocks)", libc::RLIMIT_FSIZE as _, 512, 'f'),
        ("data(kbytes)", libc::RLIMIT_DATA as _, 1024, 'd'),
        ("stack(kbytes)", libc::RLIMIT_STACK as _, 1024, 's'),
        ("coredump(blocks)", libc::RLIMIT_CORE as _, 512, 'c'),
        ("memory(kbytes)", libc::RLIMIT_RSS as _, 1024, 'm'),
        ("locked memory(kbytes)", libc::RLIMIT_MEMLOCK as _, 1024, 'l'),
        ("process", libc::RLIMIT_NPROC as _, 1, 'p'),
        ("nofiles", libc::RLIMIT_NOFILE as _, 1, 'n'),
        ("vmemory(kbytes)", libc::RLIMIT_AS as _, 1024, 'v'),
    ];
    let (mut soft, mut hard, mut all) = (true, true, false);
    let mut what = 'f';
    let mut i = 0;
    while let Some(arg) = argv.get(i) {
        if arg == "--" {
            i += 1;
            break;
        }
        if !arg.starts_with('-') || arg.len() == 1 {
            break;
        }
        for c in arg[1..].chars() {
            match c {
                'H' => (soft, hard) = (false, true),
                'S' => (soft, hard) = (true, false),
                'a' => all = true,
                c if limits.iter().any(|l| l.3 == c) => what = c,
                c => {
                    crate::ported::utils::zwarnnam("ulimit", &format!("Illegal option -{c}"));
                    return 2;
                }
            }
        }
        i += 1;
    }
    let value = argv.get(i);
    let get = |res: libc::c_int| -> libc::rlimit {
        let mut r = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        // SAFETY: `r` is a valid out-pointer for the duration of the call.
        unsafe { libc::getrlimit(res as _, &mut r) };
        r
    };
    let show = |r: &libc::rlimit, factor: libc::rlim_t| -> String {
        let v = if soft { r.rlim_cur } else { r.rlim_max };
        if v == libc::RLIM_INFINITY {
            "unlimited".to_string()
        } else {
            (v / factor).to_string()
        }
    };
    if let Some(v) = value {
        if all || argv.get(i + 1).is_some() {
            crate::ported::utils::zwarnnam("ulimit", "too many arguments");
            return 2;
        }
        let (_, res, factor, _) = limits.iter().find(|l| l.3 == what).copied().unwrap();
        let new = if v == "unlimited" {
            libc::RLIM_INFINITY
        } else if !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()) {
            match v.parse::<libc::rlim_t>().ok().and_then(|n| n.checked_mul(factor)) {
                Some(n) => n,
                None => {
                    crate::ported::utils::zwarnnam("ulimit", "bad number");
                    return 2;
                }
            }
        } else {
            crate::ported::utils::zwarnnam("ulimit", "bad number");
            return 2;
        };
        let mut r = get(res);
        if soft {
            r.rlim_cur = new;
        }
        if hard {
            r.rlim_max = new;
        }
        // SAFETY: `r` is a valid rlimit for the duration of the call.
        if unsafe { libc::setrlimit(res as _, &r) } < 0 {
            crate::ported::utils::zwarnnam(
                "ulimit",
                &format!("error setting limit ({})", std::io::Error::last_os_error()),
            );
            return 2;
        }
        return 0;
    }
    if all {
        for (name, res, factor, _) in &limits {
            println!("{:<20} {}", name, show(&get(*res), *factor));
        }
    } else {
        let (_, res, factor, _) = limits.iter().find(|l| l.3 == what).copied().unwrap();
        println!("{}", show(&get(res), factor));
    }
    0
}

/// Double the backslash of every `\c` in a printf format so zsh's
/// `getkeystring` (which reads `\c` as "stop output") emits `\c` literally,
/// as dash's printf does. An already-escaped `\\` pair is passed through.
///
/// !!! RUST-ONLY EXTENSION — no zsh C counterpart !!!
pub fn dash_printf_keep_backslash_c(fmt: &str) -> String {
    let mut out = String::with_capacity(fmt.len() + 2);
    let mut chars = fmt.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('c') => out.push_str("\\\\c"),
            Some(n) => {
                out.push('\\');
                out.push(n);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Skip the separators that may follow a `&&` / `||` / `|` / `|&` operator.
///
/// dash allows only newlines there: a `;` or the end of input is
/// `Syntax error: ";" unexpected` / `end of file unexpected`. Returns false
/// after reporting that error; every other mode skips any separator, as zsh
/// does.
///
/// !!! RUST-ONLY EXTENSION — no zsh C counterpart !!!
pub fn dash_skip_operator_separators() -> bool {
    use crate::ported::lex::{set_tok, tok, zshlex, LEX_ISNEWLIN};
    use crate::ported::zsh_h::{ENDINPUT, LEXERR, NEWLIN, SEPER};
    let faithful = dash_faithful();
    while tok() == NEWLIN
        || (tok() == SEPER && (!faithful || LEX_ISNEWLIN.with(|c| c.get()) != 0))
    {
        zshlex();
    }
    if !faithful {
        return true;
    }
    match tok() {
        SEPER => crate::ported::utils::zerr("Syntax error: \";\" unexpected"),
        ENDINPUT => crate::ported::utils::zerr("Syntax error: end of file unexpected"),
        _ => return true,
    }
    set_tok(LEXERR);
    false
}

// ---------------------------------------------------------------------------
// dash `test` / `[` — a port of dash's src/bltin/test.c (`testcmd`, `oexpr`,
// `aexpr`, `nexpr`, `primary`, `binop`, `filstat`, `t_lex`, `isoperand`).
// zsh's `bin_test` grammar (Src/builtin.c:7231, Src/parse.c par_cond) is a
// different parser with different corner cases (`-a`/`-o`/`( )` with odd
// operand counts, `-v`, unsigned-looking numbers, missing-file `-nt`/`-ot`),
// so the bare drop-in runs dash's own algorithm.
// ---------------------------------------------------------------------------

/// dash `test` operator numbers (test.c `enum token`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum DashTok {
    Eoi,
    Operand,
    FilRd,
    FilWr,
    FilEx,
    FilExist,
    FilReg,
    FilDir,
    FilCdev,
    FilBdev,
    FilFifo,
    FilSuid,
    FilSgid,
    FilStck,
    FilGz,
    FilTt,
    StrEz,
    StrNz,
    FilSym,
    FilUid,
    FilGid,
    FilSock,
    StrEq,
    StrNe,
    StrLt,
    StrGt,
    IntEq,
    IntNe,
    IntGe,
    IntGt,
    IntLe,
    IntLt,
    FilNt,
    FilOt,
    FilEq,
    Not,
    And,
    Or,
    LParen,
    RParen,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DashOpType {
    Unop,
    Binop,
    BUnop,
    BBinop,
    Paren,
}

/// test.c `ops[]`.
const DASH_TEST_OPS: &[(&str, DashTok, DashOpType)] = &[
    ("-r", DashTok::FilRd, DashOpType::Unop),
    ("-w", DashTok::FilWr, DashOpType::Unop),
    ("-x", DashTok::FilEx, DashOpType::Unop),
    ("-e", DashTok::FilExist, DashOpType::Unop),
    ("-f", DashTok::FilReg, DashOpType::Unop),
    ("-d", DashTok::FilDir, DashOpType::Unop),
    ("-c", DashTok::FilCdev, DashOpType::Unop),
    ("-b", DashTok::FilBdev, DashOpType::Unop),
    ("-p", DashTok::FilFifo, DashOpType::Unop),
    ("-u", DashTok::FilSuid, DashOpType::Unop),
    ("-g", DashTok::FilSgid, DashOpType::Unop),
    ("-k", DashTok::FilStck, DashOpType::Unop),
    ("-s", DashTok::FilGz, DashOpType::Unop),
    ("-t", DashTok::FilTt, DashOpType::Unop),
    ("-z", DashTok::StrEz, DashOpType::Unop),
    ("-n", DashTok::StrNz, DashOpType::Unop),
    ("-h", DashTok::FilSym, DashOpType::Unop),
    ("-O", DashTok::FilUid, DashOpType::Unop),
    ("-G", DashTok::FilGid, DashOpType::Unop),
    ("-L", DashTok::FilSym, DashOpType::Unop),
    ("-S", DashTok::FilSock, DashOpType::Unop),
    ("=", DashTok::StrEq, DashOpType::Binop),
    ("!=", DashTok::StrNe, DashOpType::Binop),
    ("<", DashTok::StrLt, DashOpType::Binop),
    (">", DashTok::StrGt, DashOpType::Binop),
    ("-eq", DashTok::IntEq, DashOpType::Binop),
    ("-ne", DashTok::IntNe, DashOpType::Binop),
    ("-ge", DashTok::IntGe, DashOpType::Binop),
    ("-gt", DashTok::IntGt, DashOpType::Binop),
    ("-le", DashTok::IntLe, DashOpType::Binop),
    ("-lt", DashTok::IntLt, DashOpType::Binop),
    ("-nt", DashTok::FilNt, DashOpType::Binop),
    ("-ot", DashTok::FilOt, DashOpType::Binop),
    ("-ef", DashTok::FilEq, DashOpType::Binop),
    ("!", DashTok::Not, DashOpType::BUnop),
    ("-a", DashTok::And, DashOpType::BBinop),
    ("-o", DashTok::Or, DashOpType::BBinop),
    ("(", DashTok::LParen, DashOpType::Paren),
    (")", DashTok::RParen, DashOpType::Paren),
];

type DashOp = (&'static str, DashTok, DashOpType);

fn dash_getop(s: &str) -> Option<&'static DashOp> {
    DASH_TEST_OPS.iter().find(|o| o.0 == s)
}

/// dash `atomax10`: optional blanks, sign and decimal digits filling the whole
/// string (overflow saturates, as strtoimax does); anything else is
/// "Illegal number".
fn dash_getn(s: &str) -> Result<i64, String> {
    let t = s.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let (neg, digits) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("Illegal number: {s}"));
    }
    let mag = digits.bytes().try_fold(0i64, |a, b| a.checked_mul(10)?.checked_add((b - b'0') as i64));
    Ok(match (mag, neg) {
        (Some(m), false) => m,
        (Some(m), true) => -m,
        (None, false) => i64::MAX,
        (None, true) => i64::MIN,
    })
}

struct DashTest {
    argv: Vec<String>,
    wp: usize,
    wp_op: Option<&'static DashOp>,
}

impl DashTest {
    fn at(&self, i: usize) -> Option<&str> {
        self.argv.get(i).map(String::as_str)
    }

    /// test.c `isoperand`.
    fn isoperand(&self, tp: usize) -> bool {
        let Some(s) = self.at(tp + 1) else { return true };
        if self.at(tp + 2).is_none() {
            return false;
        }
        dash_getop(s).is_some_and(|o| o.2 == DashOpType::Binop)
    }

    /// test.c `t_lex`.
    fn t_lex(&mut self, tp: usize) -> DashTok {
        let Some(s) = self.at(tp) else {
            self.wp_op = None;
            return DashTok::Eoi;
        };
        if let Some(op) = dash_getop(s) {
            if !(op.2 == DashOpType::Unop && self.isoperand(tp))
                && !(op.1 == DashTok::LParen && self.at(tp + 1).is_none())
            {
                self.wp_op = Some(op);
                return op.1;
            }
        }
        self.wp_op = None;
        DashTok::Operand
    }

    fn oexpr(&mut self, mut n: DashTok) -> Result<bool, String> {
        let mut res = false;
        loop {
            res |= self.aexpr(n)?;
            if self.at(self.wp).is_none() {
                break;
            }
            n = self.t_lex(self.wp + 1);
            if n != DashTok::Or {
                break;
            }
            self.wp += 2;
            n = self.t_lex(self.wp);
        }
        Ok(res)
    }

    fn aexpr(&mut self, mut n: DashTok) -> Result<bool, String> {
        let mut res = true;
        loop {
            if !self.nexpr(n)? {
                res = false;
            }
            if self.at(self.wp).is_none() {
                break;
            }
            n = self.t_lex(self.wp + 1);
            if n != DashTok::And {
                break;
            }
            self.wp += 2;
            n = self.t_lex(self.wp);
        }
        Ok(res)
    }

    fn nexpr(&mut self, n: DashTok) -> Result<bool, String> {
        if n != DashTok::Not {
            return self.primary(n);
        }
        let n = self.t_lex(self.wp + 1);
        if n != DashTok::Eoi {
            self.wp += 1;
        }
        Ok(!self.nexpr(n)?)
    }

    fn primary(&mut self, n: DashTok) -> Result<bool, String> {
        if n == DashTok::Eoi {
            return Ok(false);
        }
        if n == DashTok::LParen {
            self.wp += 1;
            let nn = self.t_lex(self.wp);
            if nn == DashTok::RParen {
                return Ok(false);
            }
            let res = self.oexpr(nn)?;
            self.wp += 1;
            if self.t_lex(self.wp) != DashTok::RParen {
                return Err("closing paren expected".to_string());
            }
            return Ok(res);
        }
        if let Some(op) = self.wp_op.filter(|o| o.2 == DashOpType::Unop) {
            self.wp += 1;
            let Some(arg) = self.at(self.wp).map(str::to_owned) else {
                return Err(format!("{}: argument expected", op.0));
            };
            return Ok(match n {
                DashTok::StrEz => arg.is_empty(),
                DashTok::StrNz => !arg.is_empty(),
                // SAFETY: isatty takes any int; a bad fd just reports 0.
                DashTok::FilTt => unsafe { libc::isatty(dash_getn(&arg)? as libc::c_int) != 0 },
                DashTok::FilRd => dash_access(&arg, libc::R_OK),
                DashTok::FilWr => dash_access(&arg, libc::W_OK),
                DashTok::FilEx => dash_access(&arg, libc::X_OK),
                _ => dash_filstat(&arg, n),
            });
        }
        self.t_lex(self.wp + 1);
        if self.wp_op.is_some_and(|o| o.2 == DashOpType::Binop) {
            return self.binop();
        }
        Ok(self.at(self.wp).is_some_and(|s| !s.is_empty()))
    }

    fn binop(&mut self) -> Result<bool, String> {
        let a = self.at(self.wp).unwrap_or("").to_owned();
        self.wp += 1;
        self.t_lex(self.wp);
        let Some(op) = self.wp_op else { return Ok(false) };
        self.wp += 1;
        let Some(b) = self.at(self.wp).map(str::to_owned) else {
            return Err(format!("{}: argument expected", op.0));
        };
        Ok(match op.1 {
            DashTok::StrEq => a == b,
            DashTok::StrNe => a != b,
            DashTok::StrLt => dash_strcoll(&a, &b) < 0,
            DashTok::StrGt => dash_strcoll(&a, &b) > 0,
            DashTok::IntEq => dash_getn(&a)? == dash_getn(&b)?,
            DashTok::IntNe => dash_getn(&a)? != dash_getn(&b)?,
            DashTok::IntGe => dash_getn(&a)? >= dash_getn(&b)?,
            DashTok::IntGt => dash_getn(&a)? > dash_getn(&b)?,
            DashTok::IntLe => dash_getn(&a)? <= dash_getn(&b)?,
            DashTok::IntLt => dash_getn(&a)? < dash_getn(&b)?,
            DashTok::FilNt => dash_newerf(&a, &b),
            DashTok::FilOt => dash_olderf(&a, &b),
            DashTok::FilEq => dash_equalf(&a, &b),
            _ => false,
        })
    }
}

fn dash_strcoll(a: &str, b: &str) -> i32 {
    let (Ok(a), Ok(b)) = (std::ffi::CString::new(a), std::ffi::CString::new(b)) else {
        return 0;
    };
    // SAFETY: both pointers are valid NUL-terminated strings.
    unsafe { libc::strcoll(a.as_ptr(), b.as_ptr()) }
}

/// `faccessat(AT_FDCWD, path, mode, AT_EACCESS)`.
fn dash_access(path: &str, mode: libc::c_int) -> bool {
    let Ok(p) = std::ffi::CString::new(path) else { return false };
    // SAFETY: `p` is a valid NUL-terminated string.
    unsafe { libc::faccessat(libc::AT_FDCWD, p.as_ptr(), mode, libc::AT_EACCESS) == 0 }
}

/// test.c `filstat`.
fn dash_filstat(path: &str, mode: DashTok) -> bool {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let md = if mode == DashTok::FilSym {
        std::fs::symlink_metadata(path)
    } else {
        std::fs::metadata(path)
    };
    let Ok(md) = md else { return false };
    let ft = md.file_type();
    let m = md.mode();
    match mode {
        DashTok::FilExist => true,
        DashTok::FilReg => ft.is_file(),
        DashTok::FilDir => ft.is_dir(),
        DashTok::FilCdev => ft.is_char_device(),
        DashTok::FilBdev => ft.is_block_device(),
        DashTok::FilFifo => ft.is_fifo(),
        DashTok::FilSock => ft.is_socket(),
        DashTok::FilSym => ft.is_symlink(),
        DashTok::FilSuid => m & libc::S_ISUID as u32 != 0,
        DashTok::FilSgid => m & libc::S_ISGID as u32 != 0,
        DashTok::FilStck => m & libc::S_ISVTX as u32 != 0,
        DashTok::FilGz => md.size() != 0,
        // SAFETY: geteuid/getegid have no preconditions.
        DashTok::FilUid => md.uid() == unsafe { libc::geteuid() },
        DashTok::FilGid => md.gid() == unsafe { libc::getegid() },
        _ => true,
    }
}

fn dash_mtime(path: &str) -> Option<(i64, i64)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).ok().map(|m| (m.mtime(), m.mtime_nsec()))
}

/// test.c `newerf`: a missing FIRST file is never newer, a missing second is.
fn dash_newerf(f1: &str, f2: &str) -> bool {
    let Some(a) = dash_mtime(f1) else { return false };
    dash_mtime(f2).map_or(true, |b| a > b)
}

/// test.c `olderf`: a missing SECOND file is never newer, a missing first is.
fn dash_olderf(f1: &str, f2: &str) -> bool {
    let Some(b) = dash_mtime(f2) else { return false };
    dash_mtime(f1).map_or(true, |a| a < b)
}

/// test.c `equalf`: same device and inode.
fn dash_equalf(f1: &str, f2: &str) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::metadata(f1), std::fs::metadata(f2)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

/// dash `testcmd` (test.c): `name` is `test` or `[`; `args` exclude the name.
/// Returns the exit status: 0 true, 1 false, 2 for a diagnosed error.
///
/// !!! RUST-ONLY EXTENSION — no zsh C counterpart !!!
pub fn dash_test(name: &str, args: &[String]) -> i32 {
    let mut v: Vec<String> = Vec::with_capacity(args.len() + 1);
    v.push(name.to_string());
    v.extend(args.iter().cloned());
    let mut argc = v.len();
    if name.starts_with('[') {
        argc -= 1;
        if v[argc] != "]" {
            crate::ported::utils::zwarnnam(name, "missing ]");
            return 2;
        }
        v.truncate(argc);
    }
    let mut t = DashTest { argv: v, wp: 0, wp_op: None };
    let mut res = true; // C: `int res = 1`, flipped by `res ^= oexpr()`.
    let mut base = 0usize;
    let n;
    loop {
        base += 1;
        argc -= 1;
        if argc < 1 {
            return i32::from(res);
        }
        if argc == 3
            && t.at(base + 1).and_then(dash_getop).is_some_and(|o| o.2 == DashOpType::Binop)
        {
            n = DashTok::Operand;
            break;
        }
        if argc == 3 || argc == 4 {
            if t.at(base) == Some("(") && t.at(base + argc - 1) == Some(")") {
                t.argv.truncate(base + argc - 1);
                base += 1;
                argc -= 1;
            } else if t.at(base) == Some("!") {
                res = false;
                continue;
            }
        }
        n = t.t_lex(base);
        break;
    }
    t.wp = base;
    let r = match t.oexpr(n) {
        Ok(r) => r,
        Err(msg) => {
            crate::ported::utils::zwarnnam(name, &msg);
            return 2;
        }
    };
    // C: `res ^= oexpr(n)` with `res` starting at 1 (false) or flipped to 0 by `!`.
    let status = res ^ r;
    if t.at(t.wp).is_some() && t.at(t.wp + 1).is_some() {
        crate::ported::utils::zwarnnam(name, &format!("{}: unexpected operator", t.argv[t.wp]));
        return 2;
    }
    i32::from(status)
}
