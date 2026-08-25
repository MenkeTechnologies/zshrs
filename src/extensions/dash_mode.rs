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
    ("login_shell", None, false),
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
    if canon == "nocasematch" {
        set_nocasematch(on);
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
        if zsh_opt.is_some() {
            bash_shopt_set(name, *default_on);
        }
    }
}

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
