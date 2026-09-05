//! Startup- and shutdown-file model for every zshrs emulation drop-in.
//!
//! **zshrs-only — no zsh C counterpart.** `Src/init.c::run_init_scripts`
//! knows exactly two startup-file sets: zsh's
//! (`zshenv`/`zprofile`/`zshrc`/`zlogin`) and a single lumped Bourne one
//! (`/etc/profile` + `~/.profile` + `$ENV`). That is enough for upstream,
//! which only ever emulates `sh`/`ksh` loosely. zshrs ships drop-ins for
//! eight shells and each has its own set, so the whole table lives here.
//!
//! Every row below was read off the shell's own manual AND verified by
//! running the reference binary with a scratch `$HOME` in which each
//! candidate file echoes its own name:
//!
//! | personality | login shell | interactive non-login | non-interactive |
//! |-------------|-------------|-----------------------|-----------------|
//! | `--bash`    | `/etc/profile`, first of `~/.bash_profile` / `~/.bash_login` / `~/.profile` | `/etc/bash.bashrc`†, `~/.bashrc` | `$BASH_ENV` |
//! | `--ksh`     | `/etc/profile`, `~/.profile`, then the interactive file | `/etc/ksh.kshrc`†, `$ENV` (default `~/.kshrc`) | — |
//! | `--mksh`    | `/etc/profile`, `~/.profile`, then the interactive file | `$ENV`, or `~/.mkshrc` when `$ENV` is unset | — |
//! | `--pdksh`   | `/etc/profile`, `~/.profile`, then the interactive file | `$ENV` (no default) | — |
//! | `--sh` / `--posix` / `--dash` / `--ash` | `/etc/profile`, `~/.profile`, then the interactive file | `$ENV` (no default) | — |
//! | `--csh`     | the non-login files, then `/etc/csh.login`, `~/.login` | `/etc/csh.cshrc`, `~/.tcshrc` or `~/.cshrc` | — |
//!
//! † existence-gated: see [`sys_file`].
//!
//! Reference runs (`printf 'true\n' | env -i HOME=… <shell> <flags>`):
//!
//! * `bash 5.3.15`: `-l -c true` → `.bash_profile`; with it removed →
//!   `.bash_login`; with both removed → `.profile`; `-i -c true` →
//!   `.bashrc`; `-i -l -c true` → `.bash_profile` and NOT `.bashrc`;
//!   `script.sh` → `$BASH_ENV`; `-l script.sh` → `.bash_profile` then
//!   `$BASH_ENV`; `-i -c true` with `$BASH_ENV` set → `.bashrc` only;
//!   `--norc -i` / `--noprofile -l` → nothing; `--rcfile F -i` and
//!   `--init-file F -i` → `F`; `-l -c exit` → `.bash_profile`,
//!   `.bash_logout`.
//! * `ksh93` (`/bin/ksh`, macOS): `-i` → `.kshrc`; `-i` with `$ENV` set →
//!   that file; `-l` → `.profile`; `-i -l` → `.profile` then `.kshrc`.
//! * `mksh 59c`: `-i` → `.mkshrc`; `-i` with `$ENV` → that file; `-l` →
//!   `.profile`; `-i -l` → `.profile` then `.mkshrc`.
//! * `dash`: `-i` → nothing without `$ENV`; `-i` with `$ENV` → that file;
//!   `-l` → `.profile`; `-i -l` with `$ENV` → `.profile` then `$ENV`.
//! * `tcsh` (macOS `/bin/csh`): `-i` → `.tcshrc`, or `.cshrc` when
//!   `.tcshrc` is absent; `-l` → `.tcshrc` then `.login`.
//!
//! pdksh itself is not installed here; its `$ENV`-with-no-default rule is
//! taken from ksh(1) on OpenBSD, which tells the user to set it by hand
//! ("`export ENV=$HOME/.kshrc`") — the sentence mksh(1) replaces with its
//! own `~/.mkshrc` fallback.
//!
//! Two deliberate divergences, both marked at their call site:
//!   * The system-wide rc files bash and ksh93 compile in (`SYS_BASHRC`,
//!     `/etc/ksh.kshrc`) are sourced when they EXIST. zshrs ships one
//!     binary to every platform and cannot bake in a packager's
//!     `-DSYS_BASHRC`; Debian/Ubuntu/Arch define it and ship the file,
//!     macOS defines neither — so "exists" reproduces both. Same
//!     reasoning as [`crate::extensions::global_rc`].
//!   * bash's `rshd`/`sshd` network-stdin case (a non-interactive shell
//!     whose stdin is a socket reads `~/.bashrc`) is not modeled.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Mutex;

/// Which shell zshrs is standing in for. Selected once by the binary's
/// CLI mode application (`--bash`, `--ksh`, an `argv[0]` symlink, …) and
/// read wherever behavior forks per drop-in.
///
/// This is deliberately NOT derived from the `emulation` bitmap:
/// `Src/init.c`'s `parseopts_setemulate` re-derives that from `argv[0]`
/// during `zsh_main`, which for zshrs is always the zshrs binary, so the
/// bitmap is reset to zsh on the interactive path. The personality is the
/// authoritative record of what the user asked for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Personality {
    /// Native zshrs, and the `--zsh` drop-in: zsh's own startup files.
    Zsh = 0,
    /// `--bash`.
    Bash = 1,
    /// `--ksh` — the ksh93 line.
    Ksh93 = 2,
    /// `--mksh` — MirBSD ksh.
    Mksh = 3,
    /// `--pdksh` — the Public Domain / OpenBSD ksh line.
    Pdksh = 4,
    /// `--sh` / `--posix`.
    Sh = 5,
    /// `--dash` / `--ash`.
    Dash = 6,
    /// `--csh`.
    Csh = 7,
}

impl Personality {
    /// The `emulate` personality name this drop-in installs — the same
    /// string the binary passes to `crate::ported::options::emulate`.
    pub fn emulate_name(self) -> &'static str {
        match self {
            Personality::Zsh => "zsh",
            // bash shares the `sh` option base; its deltas are applied
            // separately by the binary (brace expansion, BASH_REMATCH, …).
            Personality::Bash | Personality::Sh => "sh",
            Personality::Ksh93 | Personality::Mksh | Personality::Pdksh => "ksh",
            Personality::Dash => "dash",
            Personality::Csh => "csh",
        }
    }

    /// The parameter naming this shell's interactive startup file.
    /// bash reads `$BASH_ENV` (non-interactive only); every other Bourne
    /// personality reads `$ENV`; csh has no such parameter.
    fn env_param(self) -> Option<&'static str> {
        match self {
            Personality::Bash => Some("BASH_ENV"),
            Personality::Ksh93 | Personality::Mksh | Personality::Pdksh => Some("ENV"),
            Personality::Sh | Personality::Dash => Some("ENV"),
            Personality::Zsh | Personality::Csh => None,
        }
    }

    /// The file read when the shell's `$ENV`-equivalent is unset. Only
    /// the two Korn shells that document one have it.
    fn default_env_file(self) -> Option<&'static str> {
        match self {
            // ksh(1): "The default value is $HOME/.kshrc."
            Personality::Ksh93 => Some(".kshrc"),
            // mksh(1): "if unset or empty, the user mkshrc profile is
            // processed".
            Personality::Mksh => Some(".mkshrc"),
            _ => None,
        }
    }
}

/// Personality as a `u8`, so it can live in an atomic.
static PERSONALITY: AtomicU8 = AtomicU8::new(Personality::Zsh as u8);

/// True once the binary has explicitly selected a personality. Until then
/// `parseopts_setemulate` keeps its faithful `argv[0]` behavior.
static PERSONALITY_SET: AtomicBool = AtomicBool::new(false);

/// Record the drop-in the user asked for. Called once from the binary's
/// CLI mode application, before any user code runs.
pub fn set_personality(p: Personality) {
    PERSONALITY.store(p as u8, Ordering::Relaxed);
    PERSONALITY_SET.store(true, Ordering::Relaxed);
}

/// The selected drop-in, defaulting to [`Personality::Zsh`].
pub fn personality() -> Personality {
    match PERSONALITY.load(Ordering::Relaxed) {
        1 => Personality::Bash,
        2 => Personality::Ksh93,
        3 => Personality::Mksh,
        4 => Personality::Pdksh,
        5 => Personality::Sh,
        6 => Personality::Dash,
        7 => Personality::Csh,
        _ => Personality::Zsh,
    }
}

/// The `emulate` name to install, or `None` when no drop-in was selected
/// and the faithful `argv[0]` derivation should stand.
pub fn selected_emulate_name() -> Option<&'static str> {
    PERSONALITY_SET
        .load(Ordering::Relaxed)
        .then(|| personality().emulate_name())
}

/// bash `--norc`: inhibit `~/.bashrc` for an interactive non-login shell.
static NORC: AtomicBool = AtomicBool::new(false);

/// bash `--noprofile`: inhibit `/etc/profile` and the `~/.bash_profile`
/// chain for a login shell.
static NOPROFILE: AtomicBool = AtomicBool::new(false);

/// bash `--rcfile FILE` / `--init-file FILE`: read FILE in place of
/// `~/.bashrc`. `None` means the default.
static RCFILE: Mutex<Option<String>> = Mutex::new(None);

/// True when login-ness was asked for with an explicit `-l` / `--login`
/// rather than inferred from a leading `-` on `argv[0]`.
///
/// Only bash distinguishes the two. bash(1): the profile chain is read
/// "as an interactive login shell, or as a non-interactive shell with the
/// --login option" — so `bash -l -c CMD` reads it and a login shell that
/// sshd exec'd as `-bash` running `-c CMD` does NOT, even though both set
/// `shopt login_shell`. Verified against bash 5.3.15:
///
/// | invocation            | shopt login_shell | profile read |
/// |-----------------------|-------------------|--------------|
/// | `-bash -c CMD`        | on                | no           |
/// | `-bash -i -c CMD`     | on                | yes          |
/// | `bash -l -c CMD`      | on                | yes          |
/// | `bash -c CMD`         | off               | no           |
///
/// Every other shell here reads the profile for all three login rows —
/// measured on ksh93, mksh, dash and zsh 5.9 — so this gates the bash arm
/// alone.
static EXPLICIT_LOGIN: AtomicBool = AtomicBool::new(false);

/// bash's compile-time `SYS_BASHRC` on the distros that define it.
const SYS_BASHRC: &str = "/etc/bash.bashrc";

/// ksh93's system-wide interactive rc file, where the build has one.
const SYS_KSHRC: &str = "/etc/ksh.kshrc";

/// The system-wide profile every Bourne-family login shell reads first.
const SYS_PROFILE: &str = "/etc/profile";

/// The profile a PRIVILEGED Bourne-family login shell reads instead of
/// the user's own — `Src/init.c:1470` and mksh(1) both name this file.
const SYS_SUID_PROFILE: &str = "/etc/suid_profile";

/// csh's system-wide rc and login files (tcsh(1) "Startup and shutdown").
const SYS_CSHRC: &str = "/etc/csh.cshrc";
const SYS_CSH_LOGIN: &str = "/etc/csh.login";
const SYS_CSH_LOGOUT: &str = "/etc/csh.logout";

/// bash's `~/.bash_profile` → `~/.bash_login` → `~/.profile` chain, in
/// the order bash tries them. The FIRST readable one wins; the rest are
/// skipped even if they exist.
const BASH_PROFILE_CHAIN: [&str; 3] = [".bash_profile", ".bash_login", ".profile"];

/// tcsh(1): "first ~/.tcshrc (+) or, if ~/.tcshrc is not found, ~/.cshrc".
const CSH_RC_CHAIN: [&str; 2] = [".tcshrc", ".cshrc"];

/// Record that login-ness came from an explicit `-l` / `--login`.
#[inline]
pub fn set_explicit_login(on: bool) {
    EXPLICIT_LOGIN.store(on, Ordering::Relaxed);
}

/// True when `-l` / `--login` was given explicitly. See [`EXPLICIT_LOGIN`].
#[inline]
pub fn explicit_login() -> bool {
    EXPLICIT_LOGIN.load(Ordering::Relaxed)
}

/// Set/clear `--norc`.
#[inline]
pub fn set_norc(on: bool) {
    NORC.store(on, Ordering::Relaxed);
}

/// True when `--norc` was given.
#[inline]
pub fn norc() -> bool {
    NORC.load(Ordering::Relaxed)
}

/// Set/clear `--noprofile`.
#[inline]
pub fn set_noprofile(on: bool) {
    NOPROFILE.store(on, Ordering::Relaxed);
}

/// True when `--noprofile` was given.
#[inline]
pub fn noprofile() -> bool {
    NOPROFILE.load(Ordering::Relaxed)
}

/// Record `--rcfile FILE` / `--init-file FILE`.
pub fn set_rcfile(path: &str) {
    if let Ok(mut slot) = RCFILE.lock() {
        *slot = Some(path.to_string());
    }
}

/// The `--rcfile` override, if one was given.
pub fn rcfile() -> Option<String> {
    RCFILE.lock().ok().and_then(|slot| slot.clone())
}

/// `$HOME` as the shell sees it (paramtab, not the OS environment — a
/// startup file that reassigns `HOME` must be visible to the next one).
fn home() -> Option<PathBuf> {
    crate::ported::params::getsparam("HOME").map(PathBuf::from)
}

/// A system-wide file, included only when it is actually there. The
/// shells that compile these in (`SYS_BASHRC`, `/etc/ksh.kshrc`) ship
/// them alongside; a platform whose packager left the define out also
/// has no file, so existence reproduces both halves.
fn sys_file(path: &str) -> Option<PathBuf> {
    let p = PathBuf::from(path);
    p.exists().then_some(p)
}

/// The first readable member of `chain` under `home`.
///
/// Readable, not merely present: bash skips a startup file it cannot
/// open (and reports an error only when it exists but is unreadable),
/// and tcsh's `~/.tcshrc` → `~/.cshrc` fallback works the same way.
fn first_readable(home: Option<&Path>, chain: &[&str]) -> Option<PathBuf> {
    let h = home?;
    chain
        .iter()
        .map(|n| h.join(n))
        .find(|p| std::fs::File::open(p).is_ok())
}

/// This personality's `$ENV`-equivalent, word-expanded, or its documented
/// default when the parameter is unset.
///
/// ksh(1) and mksh(1) both specify parameter, command, arithmetic and
/// tilde substitution on the value; bash(1) says the same of `$BASH_ENV`
/// ("expands its value if it appears there … but does not use the value
/// of the PATH variable to search for the filename"). The expansion
/// mirrors zsh's own `$ENV` handling (`parsestr` + `singsub`,
/// `Src/init.c:1459`).
fn env_file(p: Personality, home: Option<&Path>) -> Option<PathBuf> {
    let raw = p
        .env_param()
        .and_then(crate::ported::params::getsparam)
        .filter(|v| !v.is_empty());
    match raw {
        Some(raw) => {
            let expanded = if raw.contains('$') || raw.contains('`') || raw.starts_with('~') {
                crate::ported::lex::untokenize(&crate::ported::subst::singsub(&raw))
            } else {
                raw
            };
            (!expanded.is_empty()).then(|| PathBuf::from(expanded))
        }
        None => p
            .default_env_file()
            .and_then(|name| home.map(|h| h.join(name))),
    }
}

/// Everything [`files_for`] needs to know about the shell it is deciding
/// for. Passing it in keeps the ordering rules testable without a booted
/// parameter table.
struct Ctx<'a> {
    home: Option<&'a Path>,
    /// The resolved `$ENV` / `$BASH_ENV` file, already defaulted.
    env_file: Option<PathBuf>,
    is_login: bool,
    is_interactive: bool,
    privileged: bool,
    /// Login-ness came from `-l` / `--login`, not from `argv[0]`'s dash.
    explicit_login: bool,
}

/// The `/etc/profile` + `~/.profile` opening every Bourne-family login
/// shell shares. A privileged shell takes `/etc/suid_profile` and no
/// user file — `Src/init.c:1470` and mksh(1) "A privileged shell then
/// processes the suid profile".
fn bourne_profile(c: &Ctx, out: &mut Vec<PathBuf>) {
    if c.privileged {
        out.extend(sys_file(SYS_SUID_PROFILE));
        return;
    }
    out.push(PathBuf::from(SYS_PROFILE));
    out.extend(c.home.map(|h| h.join(".profile")));
}

/// The ordered startup files one personality reads, given how the shell
/// was invoked. Pure: every filesystem-independent rule is decided from
/// `c` alone, so the table above is exercised directly by the tests.
fn files_for(p: Personality, c: &Ctx) -> Vec<PathBuf> {
    let mut files = Vec::new();
    match p {
        // Handled by the faithful `Src/init.c` port, not here.
        Personality::Zsh => {}

        Personality::Bash => {
            // bash(1): "If the shell is started with the effective user
            // (group) id not equal to the real user (group) id, and the
            // -p option is not supplied, no startup files are read".
            // bash reads no suid profile — unlike the Korn/Bourne line.
            if c.privileged {
                return files;
            }
            // bash(1): the profile chain belongs to "an interactive login
            // shell, or a non-interactive shell with the --login option".
            // A shell sshd exec'd as `-bash` to run `-c CMD` is a login
            // shell by `shopt login_shell` and still reads no profile —
            // see EXPLICIT_LOGIN for the measured table.
            if c.is_login && (c.is_interactive || c.explicit_login) {
                if !noprofile() {
                    files.push(PathBuf::from(SYS_PROFILE));
                    files.extend(first_readable(c.home, &BASH_PROFILE_CHAIN));
                }
            } else if !c.is_login && c.is_interactive && !norc() {
                files.extend(sys_file(SYS_BASHRC));
                match rcfile() {
                    // bash resolves `--rcfile` against `$PWD`, not
                    // `$HOME`, so a bare name is used verbatim.
                    Some(f) => files.push(PathBuf::from(f)),
                    None => files.extend(c.home.map(|h| h.join(".bashrc"))),
                }
            }
            // A NON-interactive shell also reads $BASH_ENV — including a
            // non-interactive LOGIN shell, which reads it after the
            // profile chain (`bash -l script.sh` sources `.bash_profile`
            // then `$BASH_ENV`). An interactive shell never does.
            if !c.is_interactive {
                files.extend(c.env_file.clone());
            }
        }

        // The Korn and Bourne lines share one shape and differ only in
        // what `$ENV` defaults to (resolved by the caller) and whether a
        // system-wide interactive rc file exists. Unlike bash, an
        // interactive LOGIN shell reads BOTH the profile and the
        // interactive file — `ksh -i -l` sources `.profile` then
        // `.kshrc`, and `dash -i -l` sources `.profile` then `$ENV`.
        Personality::Ksh93
        | Personality::Mksh
        | Personality::Pdksh
        | Personality::Sh
        | Personality::Dash => {
            if c.is_login {
                bourne_profile(c, &mut files);
            }
            if c.is_interactive && !c.privileged {
                if p == Personality::Ksh93 {
                    files.extend(sys_file(SYS_KSHRC));
                }
                files.extend(c.env_file.clone());
            }
        }

        // tcsh(1): "A login shell begins by executing commands from the
        // system files /etc/csh.cshrc and /etc/csh.login. It then
        // executes commands from files in the user's home directory:
        // first ~/.tcshrc (+) or, if ~/.tcshrc is not found, ~/.cshrc,
        // … then ~/.login". "Non-login shells read only /etc/csh.cshrc
        // and ~/.tcshrc or ~/.cshrc on startup." csh reads its rc file
        // whether or not the shell is interactive.
        Personality::Csh => {
            if c.privileged {
                return files;
            }
            files.extend(sys_file(SYS_CSHRC));
            files.extend(first_readable(c.home, &CSH_RC_CHAIN));
            if c.is_login {
                files.extend(sys_file(SYS_CSH_LOGIN));
                files.extend(c.home.map(|h| h.join(".login")));
            }
        }
    }
    files
}

/// The ordered startup files the selected drop-in reads. Returned rather
/// than sourced so the library's `run_init_scripts` hook and the
/// binary's `-c` / script-file dispatch drive the SAME list through their
/// own sourcing machinery.
///
/// Paths come back unfiltered by existence: the caller sources what is
/// there and ignores what is not, as every one of these shells does.
pub fn startup_files(is_login: bool, is_interactive: bool, privileged: bool) -> Vec<PathBuf> {
    let p = personality();
    let home = home();
    files_for(
        p,
        &Ctx {
            home: home.as_deref(),
            env_file: env_file(p, home.as_deref()),
            is_login,
            is_interactive,
            privileged,
            explicit_login: explicit_login(),
        },
    )
}

/// The ordered files read when a LOGIN shell exits, given whether the
/// shell is interactive.
///
/// bash(1): "~/.bash_logout". tcsh(1) reads `~/.logout` and
/// `/etc/csh.logout`. The Korn and Bourne shells document no logout
/// file, and zsh's `.zlogout` stays with the faithful port in
/// `Src/builtin.c::zexit`.
pub fn logout_files(is_interactive: bool) -> Vec<PathBuf> {
    let home = home();
    match personality() {
        // bash reads it for a non-interactive login shell too, as long as
        // the shell left through the `exit` builtin — `bash -l -c exit`
        // sources `~/.bash_logout`, `bash -l -c true` does not.
        Personality::Bash => home.map(|h| h.join(".bash_logout")).into_iter().collect(),
        // tcsh needs the login shell to be INTERACTIVE: `csh -csh -c exit`
        // reads `~/.cshrc` and stops there, never `~/.logout`.
        Personality::Csh if is_interactive => home
            .map(|h| h.join(".logout"))
            .into_iter()
            .chain(sys_file(SYS_CSH_LOGOUT))
            .collect(),
        _ => Vec::new(),
    }
}

/// Re-apply the selected drop-in's option deltas on top of the `emulate`
/// preset it shares with another shell.
///
/// Must be called after EVERY `options::emulate()` that installs the
/// personality, because that call resets the option table wholesale. The
/// binary applies these once during CLI mode selection, but
/// `Src/init.c`'s `parseopts_setemulate` re-derives and re-installs the
/// emulation inside `zsh_main` — which only the INTERACTIVE path reaches.
/// So the deltas used to survive on the `-c` and script-file paths and be
/// silently wiped for an interactive shell: `zshrs --bash -i` ran with
/// brace expansion off, no `$BASH_REMATCH`, and `PS1` rendered as raw `%`
/// sequences, which is exactly the configuration a login shell gets.
///
/// Safe to call more than once, and always before any user startup file
/// runs, so a user's own `setopt` / `shopt` still wins.
pub fn apply_personality_option_deltas() {
    use crate::ported::options::opt_state_set;
    let p = personality();
    if p == Personality::Zsh {
        return;
    }
    // EVERY drop-in: zsh marks a partial last line with an inverse `%`
    // before printing the prompt (PROMPT_SP). No Bourne-family shell does
    // — measured on bash, ksh93, mksh, dash and /bin/sh — and the marker
    // plus its clear-to-end-of-line padding is the most visible thing on
    // the screen, so it is off for all of them.
    opt_state_set("promptsp", false);
    if p != Personality::Bash {
        return;
    }
    // bash is a SUPERSET of POSIX sh: unlike `emulate sh` (which sets
    // IGNORE_BRACES), bash performs brace expansion — `echo {a,b}` → `a b`.
    opt_state_set("ignorebraces", false);
    // bash always populates `$BASH_REMATCH` after `[[ str =~ re ]]`.
    opt_state_set("bashrematch", true);
    // zshrs renders a bash prompt by translating its backslash escapes
    // into `%` sequences (see `crate::extensions::bash_prompt`), so the
    // `%` pass must be on even though `emulate sh` turns it off. A
    // literal `%` is doubled by the translator, so nothing is lost.
    opt_state_set("promptpercent", true);
    // bash's `promptvars` shopt (on by default) expands parameters and
    // command substitutions in the prompt at display time — zsh's
    // PROMPT_SUBST.
    opt_state_set("promptsubst", true);
    // bash-only param syntax (`${!var}`, `${v^^}`) and the `shopt`
    // defaults for the rows backed by a real zsh option.
    crate::dash_mode::set_bash_mode(true);
    crate::dash_mode::bash_shopt_apply_defaults();
}

/// Whether this drop-in writes the bracketed-paste enable/disable pair
/// (`\e[?2004h` / `\e[?2004l`) around its line editing.
///
/// Measured on this machine by driving each shell under a pty and
/// counting `?2004h`: bash 5.3 → 1, zsh 5.9 → 1, ksh93 → 0, mksh → 0,
/// dash → 0, `/bin/sh` → 0, tcsh → 0. zshrs's ZLE sends it
/// unconditionally, which put a pair of sequences on the wire that the
/// emulated shell never sends.
// (Tested from tests/startup_file_parity.rs, not here: asserting these
// means flipping the process-global personality, and the lib test binary
// runs its tests in parallel threads that share it — doing it inline made
// 74 `ported::prompt` tests take the bash translation path.)
pub fn emits_bracketed_paste() -> bool {
    matches!(personality(), Personality::Zsh | Personality::Bash)
}

/// Whether this drop-in writes the OSC 133 shell-integration marker
/// before each prompt. It is a zsh feature — bash does not send one, and
/// neither does zsh 5.9 with its default `.term.extensions` — so only
/// native zshrs keeps it.
pub fn emits_integration_prompt() -> bool {
    personality() == Personality::Zsh
}

/// ksh93 defines NO default aliases in the maintained line.
///
/// This one is version-split, and the split is real: AT&T ksh93u+ 2012
/// (what Apple ships as `/bin/ksh`) defines 19 — `r`, `functions`,
/// `integer`, `type`, `hash`, `history`, `source`, … — while ksh93u+m
/// 1.0.10 2024 (Homebrew's `ksh93`, the maintained fork) defines none.
///
/// zshrs follows the maintained line, for two reasons. It is what a
/// current install gives you, and the failure modes are asymmetric:
/// omitting an alias leaves a command resolving to its builtin, while
/// inventing one SHADOWS a real command — aliasing `type`, `hash`,
/// `source` or `r` when the user's ksh would not is the more surprising
/// error of the two.
const KSH93_ALIASES: &[(&str, &str)] = &[];

/// mksh's built-in aliases, captured from `mksh -c alias`. The values
/// really do carry TWO backslashes: mksh does not double a backslash when
/// listing (`alias foo='a\b'` lists as `foo='a\b'`), so `\\builtin` in
/// the listing is the stored value, not an escape of it.
const MKSH_ALIASES: &[(&str, &str)] = &[
    ("autoload", r"\builtin typeset -fu"),
    ("functions", r"\builtin typeset -f"),
    ("hash", r"\builtin alias -t"),
    ("history", r"\builtin fc -l"),
    ("integer", r"\builtin typeset -i"),
    ("local", r"\builtin typeset"),
    ("login", r"\builtin exec login"),
    ("nameref", r"\builtin typeset -n"),
    ("nohup", "nohup "),
    ("r", r"\builtin fc -e -"),
    ("type", r"\builtin whence -v"),
];

/// The aliases this personality starts with, or `None` to leave zsh's own
/// defaults (`run-help`, `which-command`) in place.
///
/// Measured with `<shell> -c alias` in an empty environment: bash 0,
/// dash 0, `/bin/sh` 0, ksh93 19, mksh 11, zsh 2. zshrs installed zsh's
/// two in every mode, so `alias` under `--bash` listed `run-help=man`
/// (bash has none) and under `--ksh` listed neither `r` nor `functions`
/// (ksh has both).
pub fn default_aliases() -> Option<&'static [(&'static str, &'static str)]> {
    match personality() {
        Personality::Zsh => None,
        Personality::Ksh93 => Some(KSH93_ALIASES),
        Personality::Mksh | Personality::Pdksh => Some(MKSH_ALIASES),
        // bash, dash/ash, sh/posix and csh define no aliases at all.
        _ => Some(&[]),
    }
}

/// Replace the alias table's contents with this personality's defaults.
/// Called once during CLI mode selection, before any startup file runs,
/// so a user's own `alias` still wins.
pub fn install_default_aliases() {
    let Some(defaults) = default_aliases() else {
        return;
    };
    let Ok(mut tab) = crate::ported::hashtable::aliastab_lock().write() else {
        return;
    };
    // Drop zsh's `run-help` / `which-command`, then install this shell's
    // own set. `clear` + `add` is the table's existing API; the drop-in
    // owns the whole default set, not a delta on zsh's.
    tab.clear();
    for (name, text) in defaults {
        tab.add(crate::ported::hashtable::createaliasnode(name, text, 0));
    }
}

/// Single-quote a value the way bash's `alias` / `declare -p` listings
/// do: wrap in `'…'` and render an embedded quote as `'\''`.
pub fn single_quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for c in value.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}

/// True when this drop-in owns its own startup files, i.e. the faithful
/// `run_init_scripts` port must NOT run for it.
pub fn overrides_zsh_startup() -> bool {
    personality() != Personality::Zsh
}

/// Source the selected drop-in's startup files. Library-side entry
/// point, called from the `run_init_scripts` hook.
pub fn run_init_scripts() {
    // `-f` / `unsetopt rcs` suppresses every startup file, the same way
    // bash's `--norc` + `--noprofile` do.
    if !crate::ported::zsh_h::isset(crate::ported::zsh_h::RCS) {
        return;
    }
    let files = startup_files(
        crate::ported::zsh_h::islogin(),
        crate::ported::zsh_h::interact(),
        crate::ported::zsh_h::isset(crate::ported::zsh_h::PRIVILEGED),
    );
    for f in files {
        let _ = crate::ported::init::source(&f.to_string_lossy());
    }
}

/// Source the selected drop-in's logout files on the way out of a login
/// shell. Called from `zexit`, where zsh reads `.zlogout`.
pub fn run_logout_scripts() {
    if !crate::ported::zsh_h::islogin() {
        return;
    }
    if !crate::ported::zsh_h::isset(crate::ported::zsh_h::RCS) {
        return;
    }
    for f in logout_files(crate::ported::zsh_h::interact()) {
        let _ = crate::ported::init::source(&f.to_string_lossy());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialize the process-global flag mutations these tests perform.
    static GUARD: Mutex<()> = Mutex::new(());

    /// Reset every flag so one test's `--norc` cannot leak into the next.
    fn reset() {
        set_norc(false);
        set_noprofile(false);
        if let Ok(mut slot) = RCFILE.lock() {
            *slot = None;
        }
    }

    /// A `$HOME` holding the named files, so "first readable wins" is
    /// exercised against a real directory.
    fn fake_home(files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        for f in files {
            std::fs::write(dir.path().join(f), "").expect("write");
        }
        dir
    }

    fn ctx<'a>(home: &'a Path, env: Option<&str>, login: bool, interactive: bool) -> Ctx<'a> {
        Ctx {
            home: Some(home),
            env_file: env.map(PathBuf::from),
            is_login: login,
            is_interactive: interactive,
            privileged: false,
            // The curated rows below all describe an EXPLICIT `-l`, which
            // is the shape `bash -l` / `ksh -l` documents. The implicit
            // `argv[0]`-dash form has its own test.
            explicit_login: true,
        }
    }

    fn names(files: Vec<PathBuf>) -> Vec<String> {
        files
            .into_iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect()
    }

    /// The non-system files, as bare names — so an assertion reads like
    /// the shell manual instead of like a tempdir path.
    ///
    /// `/etc/…` entries are dropped because the system-wide rows are
    /// existence-gated and therefore platform-dependent: `/etc/csh.cshrc`
    /// is present on macOS, `/etc/bash.bashrc` on Debian, `/etc/profile`
    /// almost everywhere. Their presence is asserted separately, by path,
    /// where it is part of the rule under test.
    fn tails(files: Vec<PathBuf>) -> Vec<String> {
        files
            .into_iter()
            .filter(|p| !p.starts_with("/etc"))
            .map(|p| {
                p.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            })
            .collect()
    }

    /// bash: a login shell reads the profile chain and NEVER `~/.bashrc`
    /// — bash 5.3.15 `-i -l -c true` sources `.bash_profile` alone.
    #[test]
    fn bash_login_reads_profile_not_bashrc() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let home = fake_home(&[".bash_profile", ".bash_login", ".profile", ".bashrc"]);
        let files = files_for(Personality::Bash, &ctx(home.path(), None, true, true));
        assert!(
            names(files.clone()).contains(&SYS_PROFILE.to_string()),
            "a login bash reads /etc/profile first"
        );
        assert_eq!(tails(files), vec![".bash_profile".to_string()]);
    }

    /// bash: the profile chain falls through in the documented order.
    #[test]
    fn bash_profile_chain_falls_through_in_order() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        for (present, expected) in [
            (vec![".bash_login", ".profile"], ".bash_login"),
            (vec![".profile"], ".profile"),
        ] {
            let home = fake_home(&present);
            assert_eq!(
                tails(files_for(
                    Personality::Bash,
                    &ctx(home.path(), None, true, true)
                ))
                .last()
                .map(String::as_str),
                Some(expected),
                "with {present:?} present, bash reads {expected}"
            );
        }
        let home = fake_home(&[]);
        assert_eq!(
            names(files_for(
                Personality::Bash,
                &ctx(home.path(), None, true, true)
            )),
            vec![SYS_PROFILE.to_string()],
            "an empty home leaves a login shell with /etc/profile alone"
        );
    }

    /// bash: an interactive non-login shell reads `~/.bashrc`, no
    /// profile, and never a zsh file.
    #[test]
    fn bash_interactive_nonlogin_reads_bashrc_only() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let home = fake_home(&[".bashrc", ".bash_profile", ".zshrc"]);
        let files = tails(files_for(
            Personality::Bash,
            &ctx(home.path(), None, false, true),
        ));
        assert!(files.contains(&".bashrc".to_string()), "got {files:?}");
        assert!(!files.iter().any(|f| f == ".bash_profile"), "got {files:?}");
        assert!(!files.iter().any(|f| f.contains("zsh")), "got {files:?}");
    }

    /// bash: `--norc` and `--noprofile` each empty their own phase.
    #[test]
    fn bash_norc_and_noprofile_suppress_their_phase() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let home = fake_home(&[".bashrc", ".bash_profile"]);
        set_norc(true);
        assert!(files_for(Personality::Bash, &ctx(home.path(), None, false, true)).is_empty());
        reset();
        set_noprofile(true);
        assert!(files_for(Personality::Bash, &ctx(home.path(), None, true, true)).is_empty());
        reset();
    }

    /// bash: `--rcfile FILE` REPLACES `~/.bashrc`.
    #[test]
    fn bash_rcfile_overrides_bashrc() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let home = fake_home(&[".bashrc"]);
        set_rcfile("/tmp/zshrs-alt-bashrc");
        let files = names(files_for(
            Personality::Bash,
            &ctx(home.path(), None, false, true),
        ));
        assert!(
            files.iter().any(|f| f == "/tmp/zshrs-alt-bashrc"),
            "got {files:?}"
        );
        assert!(
            !files.iter().any(|f| f.ends_with("/.bashrc")),
            "got {files:?}"
        );
        reset();
    }

    /// bash: `$BASH_ENV` is non-interactive-only, and a non-interactive
    /// login shell reads it AFTER the profile chain.
    #[test]
    fn bash_env_is_non_interactive_only_and_last() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let home = fake_home(&[".bashrc", ".bash_profile"]);
        let probe = "/tmp/zshrs-env-probe";
        assert_eq!(
            tails(files_for(
                Personality::Bash,
                &ctx(home.path(), Some(probe), true, false)
            )),
            vec![".bash_profile".to_string(), "zshrs-env-probe".to_string()],
            "profile chain first, $BASH_ENV last"
        );
        assert_eq!(
            names(files_for(
                Personality::Bash,
                &ctx(home.path(), Some(probe), false, false)
            )),
            vec![probe.to_string()],
        );
        assert!(
            !names(files_for(
                Personality::Bash,
                &ctx(home.path(), Some(probe), false, true)
            ))
            .iter()
            .any(|f| f == probe),
            "an interactive bash never reads $BASH_ENV"
        );
    }

    /// The Korn/Bourne line: an interactive LOGIN shell reads the
    /// profile AND the interactive file — unlike bash. Verified with
    /// `ksh -i -l` (`.profile`, `.kshrc`) and `dash -i -l` with `$ENV`
    /// set (`.profile`, `$ENV`).
    #[test]
    fn korn_and_bourne_interactive_login_read_both() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let home = fake_home(&[".profile"]);
        let probe = "/tmp/zshrs-env-probe";
        for p in [
            Personality::Ksh93,
            Personality::Mksh,
            Personality::Pdksh,
            Personality::Sh,
            Personality::Dash,
        ] {
            let files = files_for(p, &ctx(home.path(), Some(probe), true, true));
            assert!(
                names(files.clone()).contains(&SYS_PROFILE.to_string()),
                "{p:?} interactive login reads /etc/profile"
            );
            assert_eq!(
                tails(files),
                vec![".profile".to_string(), "zshrs-env-probe".to_string()],
                "{p:?} interactive login reads ~/.profile, then $ENV"
            );
        }
    }

    /// The Korn/Bourne line reads NO interactive file when the shell is
    /// not interactive — `dash script.sh` sources nothing.
    #[test]
    fn korn_and_bourne_non_interactive_read_no_rc() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let home = fake_home(&[".profile", ".kshrc", ".mkshrc"]);
        let probe = "/tmp/zshrs-env-probe";
        for p in [Personality::Ksh93, Personality::Mksh, Personality::Dash] {
            assert!(
                files_for(p, &ctx(home.path(), Some(probe), false, false)).is_empty(),
                "{p:?} non-interactive non-login reads nothing"
            );
        }
    }

    /// `$ENV` defaults: ksh93 → `~/.kshrc`, mksh → `~/.mkshrc`, and
    /// pdksh / sh / dash have none.
    #[test]
    fn env_defaults_match_each_korn_line() {
        let home = fake_home(&[]);
        assert_eq!(
            env_file(Personality::Ksh93, Some(home.path())),
            Some(home.path().join(".kshrc"))
        );
        assert_eq!(
            env_file(Personality::Mksh, Some(home.path())),
            Some(home.path().join(".mkshrc"))
        );
        for p in [Personality::Pdksh, Personality::Sh, Personality::Dash] {
            assert_eq!(
                env_file(p, Some(home.path())),
                None,
                "{p:?} documents no default for $ENV"
            );
        }
    }

    /// csh reads its rc file whether or not the shell is a login shell,
    /// prefers `~/.tcshrc` over `~/.cshrc`, and appends `~/.login` for a
    /// login shell — tcsh(1), and `csh -l` → `.tcshrc`, `.login`.
    #[test]
    fn csh_reads_cshrc_always_and_login_after() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let home = fake_home(&[".tcshrc", ".cshrc", ".login"]);
        assert_eq!(
            tails(files_for(
                Personality::Csh,
                &ctx(home.path(), None, false, true)
            )),
            vec![".tcshrc".to_string()],
        );
        assert_eq!(
            tails(files_for(
                Personality::Csh,
                &ctx(home.path(), None, true, true)
            )),
            vec![".tcshrc".to_string(), ".login".to_string()],
        );
        let no_tcshrc = fake_home(&[".cshrc"]);
        assert_eq!(
            tails(files_for(
                Personality::Csh,
                &ctx(no_tcshrc.path(), None, false, true)
            )),
            vec![".cshrc".to_string()],
            "~/.cshrc is the fallback when ~/.tcshrc is absent"
        );
    }

    /// A privileged shell reads no user file. bash reads nothing at all;
    /// the Korn/Bourne line takes `/etc/suid_profile` instead of
    /// `/etc/profile` + `~/.profile`.
    #[test]
    fn privileged_shell_reads_no_user_file() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let home = fake_home(&[".bashrc", ".bash_profile", ".profile", ".kshrc", ".cshrc"]);
        for p in [
            Personality::Bash,
            Personality::Ksh93,
            Personality::Mksh,
            Personality::Pdksh,
            Personality::Sh,
            Personality::Dash,
            Personality::Csh,
        ] {
            for (login, interactive) in [(true, true), (true, false), (false, true), (false, false)]
            {
                let files = names(files_for(
                    p,
                    &Ctx {
                        home: Some(home.path()),
                        env_file: Some(PathBuf::from("/tmp/zshrs-env-probe")),
                        is_login: login,
                        is_interactive: interactive,
                        privileged: true,
                        explicit_login: true,
                    },
                ));
                assert!(
                    files.iter().all(|f| f.starts_with("/etc/")),
                    "{p:?} privileged (login={login}, interactive={interactive}) read {files:?}"
                );
            }
        }
    }

    /// zsh keeps the faithful `Src/init.c` port; this module contributes
    /// nothing for it, and `overrides_zsh_startup` says so.
    #[test]
    fn zsh_personality_contributes_nothing() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let home = fake_home(&[".zshrc", ".profile", ".bashrc"]);
        assert!(files_for(
            Personality::Zsh,
            &ctx(home.path(), Some("/tmp/zshrs-env-probe"), true, true)
        )
        .is_empty());
        assert_eq!(Personality::Zsh.emulate_name(), "zsh");
    }

    /// Each drop-in installs the emulation its binary-side mode selection picks.
    #[test]
    fn emulate_names_match_the_cli_modes() {
        assert_eq!(Personality::Bash.emulate_name(), "sh");
        assert_eq!(Personality::Sh.emulate_name(), "sh");
        assert_eq!(Personality::Ksh93.emulate_name(), "ksh");
        assert_eq!(Personality::Mksh.emulate_name(), "ksh");
        assert_eq!(Personality::Pdksh.emulate_name(), "ksh");
        assert_eq!(Personality::Dash.emulate_name(), "dash");
        assert_eq!(Personality::Csh.emulate_name(), "csh");
    }

    /// bash alone distinguishes an IMPLICIT login shell (a leading `-` on
    /// `argv[0]`, which is how login(1) and sshd start one) from an
    /// explicit `-l`. Non-interactive + implicit reads no profile; every
    /// other shell reads it in all three login shapes.
    #[test]
    fn implicit_login_reads_no_bash_profile_but_does_for_the_others() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let home = fake_home(&[".bash_profile", ".bashrc", ".profile", ".kshrc"]);
        let implicit = |p: Personality, interactive: bool| {
            tails(files_for(
                p,
                &Ctx {
                    home: Some(home.path()),
                    env_file: None,
                    is_login: true,
                    is_interactive: interactive,
                    privileged: false,
                    explicit_login: false,
                },
            ))
        };
        // bash: non-interactive implicit login reads NOTHING …
        assert!(
            implicit(Personality::Bash, false).is_empty(),
            "`-bash -c CMD` reads no startup file, got {:?}",
            implicit(Personality::Bash, false)
        );
        // … but the interactive form reads the profile chain.
        assert_eq!(
            implicit(Personality::Bash, true),
            vec![".bash_profile".to_string()],
        );
        // The Korn/Bourne line reads ~/.profile either way.
        for p in [Personality::Ksh93, Personality::Sh, Personality::Dash] {
            assert!(
                implicit(p, false).contains(&".profile".to_string()),
                "{p:?} reads ~/.profile for a non-interactive implicit login shell"
            );
        }
    }

    /// The atomic round-trips every variant.
    #[test]
    fn personality_round_trips_through_the_atomic() {
        let _g = GUARD.lock().unwrap_or_else(|e| e.into_inner());
        let saved = personality();
        for p in [
            Personality::Zsh,
            Personality::Bash,
            Personality::Ksh93,
            Personality::Mksh,
            Personality::Pdksh,
            Personality::Sh,
            Personality::Dash,
            Personality::Csh,
        ] {
            set_personality(p);
            assert_eq!(personality(), p);
            assert_eq!(selected_emulate_name(), Some(p.emulate_name()));
        }
        set_personality(saved);
    }
}
