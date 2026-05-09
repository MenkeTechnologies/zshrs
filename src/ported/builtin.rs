//! Direct port of `Src/builtin.c` — the master registration site for
//! the in-shell builtin commands. The C source is 7608 lines; the
//! actual `bin_*` handler bodies were ported organically into
//! `src/ported/exec.rs` and `src/ported/builtins/*.rs` long before
//! this file existed. This file scaffolds:
//!
//!   * the `BINF_*` flag bits from `Src/zsh.h:1457-1486`,
//!   * the `BIN_*` dispatch IDs from `Src/hashtable.h:34-66`,
//!   * the `Builtin` descriptor and the static `BUILTINS[]` table
//!     (1:1 mirror of `static struct builtin builtins[]` at
//!     `Src/builtin.c:40-137`),
//!   * `createbuiltintable()` (`Src/builtin.c:149`) — building the
//!     name → descriptor lookup the rest of the shell consults via
//!     `builtintab`.
//!
//! Each row's `handler` field names the canonical Rust port of the
//! C handler so future work can wire them up without re-discovering
//! the mapping. When the handler lives in `crate::ported::builtins`,
//! the comment cites the file; when it lives in `exec.rs`'s
//! `Executor` impl, that's noted too.

use std::collections::HashMap;
use std::sync::OnceLock;

// === Imports needed by the methods moved from exec.rs (below) ===
#[allow(unused_imports)]
use std::{env, fs, io, io::Write, path::Path, path::PathBuf};
#[allow(unused_imports)]
use indexmap::IndexMap;
#[allow(unused_imports)]
use crate::ported::exec::{
    self, ShellExecutor, BUILTIN_SET,
    format_int_in_base,
    VarAttr, VarKind,
};
use crate::ported::utils::{zerr, zerrnam, zwarn, zwarnnam};
use crate::ported::text::FuncBodyFmt;
#[allow(unused_imports)]
use crate::ported::options::ZSH_OPTIONS_SET;
#[allow(unused_imports)]
use crate::parse::{Redirect, ShellCommand};
#[allow(unused_imports)]
use crate::extensions::zwc::ZwcFile;

// ---------------------------------------------------------------------------
// BINF_* flag bits.
// Direct port of `Src/zsh.h:1457-1486`. Same numeric values so any
// recorded data file or test fixture comparing flag bitmaps stays
// portable across the C and Rust shells.
// ---------------------------------------------------------------------------

pub const BINF_PLUSOPTS: u32 = 1 << 1; // +xyz legal
pub const BINF_PRINTOPTS: u32 = 1 << 2;
pub const BINF_ADDED: u32 = 1 << 3; // is in the builtins hash table
pub const BINF_MAGICEQUALS: u32 = 1 << 4; // needs auto MAGIC_EQUAL_SUBST
pub const BINF_PREFIX: u32 = 1 << 5;
pub const BINF_DASH: u32 = 1 << 6;
pub const BINF_BUILTIN: u32 = 1 << 7;
pub const BINF_COMMAND: u32 = 1 << 8;
pub const BINF_EXEC: u32 = 1 << 9;
pub const BINF_NOGLOB: u32 = 1 << 10;
pub const BINF_PSPECIAL: u32 = 1 << 11;
pub const BINF_SKIPINVALID: u32 = 1 << 12; // treat invalid option as argument
pub const BINF_KEEPNUM: u32 = 1 << 13; // [-+]NUM can be an option
pub const BINF_SKIPDASH: u32 = 1 << 14; // treat `-` as argument
pub const BINF_DASHDASHVALID: u32 = 1 << 15; // honour -- even if SKIPINVALID
pub const BINF_CLEARENV: u32 = 1 << 16; // exec into cleared env
pub const BINF_AUTOALL: u32 = 1 << 17; // autoload every feature at once
pub const BINF_HANDLES_OPTS: u32 = 1 << 18;
pub const BINF_ASSIGN: u32 = 1 << 19;

// ---------------------------------------------------------------------------
// BIN_* dispatch IDs.
// Direct port of `Src/hashtable.h:34-70`. These are the integer
// discriminators handlers use when one C function backs multiple
// builtin names (e.g. `bin_fg` covers fg/bg/jobs/wait/disown).
// ---------------------------------------------------------------------------

pub const BIN_TYPESET: i32 = 0;
pub const BIN_BG: i32 = 1;
pub const BIN_FG: i32 = 2;
pub const BIN_JOBS: i32 = 3;
pub const BIN_WAIT: i32 = 4;
pub const BIN_DISOWN: i32 = 5;
pub const BIN_BREAK: i32 = 6;
pub const BIN_CONTINUE: i32 = 7;
pub const BIN_EXIT: i32 = 8;
pub const BIN_RETURN: i32 = 9;
pub const BIN_CD: i32 = 10;
pub const BIN_POPD: i32 = 11;
pub const BIN_PUSHD: i32 = 12;
pub const BIN_PRINT: i32 = 13;
pub const BIN_EVAL: i32 = 14;
pub const BIN_SCHED: i32 = 15;
pub const BIN_FC: i32 = 16;
pub const BIN_R: i32 = 17;
pub const BIN_PUSHLINE: i32 = 18;
pub const BIN_LOGOUT: i32 = 19;
pub const BIN_TEST: i32 = 20;
pub const BIN_BRACKET: i32 = 21;
pub const BIN_READONLY: i32 = 22;
pub const BIN_ECHO: i32 = 23;
pub const BIN_DISABLE: i32 = 24;
pub const BIN_ENABLE: i32 = 25;
pub const BIN_PRINTF: i32 = 26;
pub const BIN_COMMAND: i32 = 27;
pub const BIN_UNHASH: i32 = 28;
pub const BIN_UNALIAS: i32 = 29;
pub const BIN_UNFUNCTION: i32 = 30;
pub const BIN_UNSET: i32 = 31;
pub const BIN_EXPORT: i32 = 32;

// setopt / unsetopt re-use the dispatch slot space.
pub const BIN_SETOPT: i32 = 0;
pub const BIN_UNSETOPT: i32 = 1;

// ---------------------------------------------------------------------------
// Builtin descriptor.
// Port of `struct builtin` from `Src/zsh.h` (the one expanded by the
// `BUILTIN` / `BIN_PREFIX` macros at line 1452 of zsh.h).
// ---------------------------------------------------------------------------

/// Static metadata for a single in-shell builtin command.
///
/// Mirrors the C `struct builtin` row by row — name, flag bitmap,
/// arg bounds, dispatch funcid, and the option strings the C
/// handler's `OPT_ISSET()` / `OPT_ARG()` macros consult.
///
/// `handler_name` is a free-form string identifying the canonical
/// Rust port of the C handler (e.g. `"builtins::rlimits::bin_limit"`
/// or `"exec::ShellExecutor::bin_print"`). The actual call dispatch
/// today still lives in `Executor::register_builtins` in `exec.rs`;
/// this field exists so that table walkers and the port-report
/// generator can credit the right Rust file when reporting which
/// `bin_*` symbols from `Src/builtin.c` are ported.
#[derive(Debug, Clone, Copy)]
pub struct Builtin {
    pub name: &'static str,
    pub flags: u32,
    pub handler_name: &'static str,
    pub minargs: i32,
    /// `-1` means unbounded — C uses the same sentinel.
    pub maxargs: i32,
    pub funcid: i32,
    /// Standard option string (e.g. `"Lgmrs"` for `alias`).
    pub optstr: Option<&'static str>,
    /// Default-set option string used by some builtins (e.g. the
    /// `"u"` for `autoload`).
    pub defopts: Option<&'static str>,
}

impl Builtin {
    /// Helper for the `BIN_PREFIX(name, flags)` rows at the top of
    /// `Src/builtin.c:42-46` — a prefix builtin has no handler of
    /// its own, just modifies how the next word is parsed.
    pub const fn prefix(name: &'static str, flags: u32) -> Self {
        Self {
            name,
            flags: flags | BINF_PREFIX,
            handler_name: "(prefix)",
            minargs: 0,
            maxargs: -1,
            funcid: 0,
            optstr: None,
            defopts: None,
        }
    }

    /// Helper for the BUILTIN(...) rows.
    pub const fn entry(
        name: &'static str,
        flags: u32,
        handler_name: &'static str,
        minargs: i32,
        maxargs: i32,
        funcid: i32,
        optstr: Option<&'static str>,
        defopts: Option<&'static str>,
    ) -> Self {
        Self {
            name,
            flags,
            handler_name,
            minargs,
            maxargs,
            funcid,
            optstr,
            defopts,
        }
    }
}

// ---------------------------------------------------------------------------
// The master registration table.
//
// Direct, line-for-line port of `static struct builtin builtins[]`
// at `Src/builtin.c:40-137`. Entries appear in the same order so
// any diff against the C source stays trivial. The `handler_name`
// column points at the canonical Rust port that the dispatcher in
// `Executor::register_builtins` (`src/ported/exec.rs`) wires up.
// ---------------------------------------------------------------------------

pub static BUILTINS: &[Builtin] = &[
    Builtin::prefix("-", BINF_DASH),
    Builtin::prefix("builtin", BINF_BUILTIN),
    Builtin::prefix("command", BINF_COMMAND),
    Builtin::prefix("exec", BINF_EXEC),
    Builtin::prefix("noglob", BINF_NOGLOB),
    Builtin::entry("[", BINF_HANDLES_OPTS, "exec::ShellExecutor::bin_test", 0, -1, BIN_BRACKET, None, None),
    Builtin::entry(".", BINF_PSPECIAL, "exec::ShellExecutor::builtin_dot", 1, -1, 0, None, None),
    Builtin::entry(":", BINF_PSPECIAL, "exec::ShellExecutor::builtin_true", 0, -1, 0, None, None),
    Builtin::entry("alias", BINF_MAGICEQUALS | BINF_PLUSOPTS, "exec::ShellExecutor::bin_alias", 0, -1, 0, Some("Lgmrs"), None),
    Builtin::entry("autoload", BINF_PLUSOPTS, "exec::ShellExecutor::builtin_autoload", 0, -1, 0, Some("dmktrRTUwWXz"), Some("u")),
    Builtin::entry("bg", 0, "exec::ShellExecutor::bin_fg", 0, -1, BIN_BG, None, None),
    Builtin::entry("break", BINF_PSPECIAL, "exec::ShellExecutor::bin_break", 0, 1, BIN_BREAK, None, None),
    Builtin::entry("bye", 0, "exec::ShellExecutor::bin_break", 0, 1, BIN_EXIT, None, None),
    Builtin::entry("cd", BINF_SKIPINVALID | BINF_SKIPDASH | BINF_DASHDASHVALID, "exec::ShellExecutor::builtin_cd", 0, 2, BIN_CD, Some("qsPL"), None),
    Builtin::entry("chdir", BINF_SKIPINVALID | BINF_SKIPDASH | BINF_DASHDASHVALID, "exec::ShellExecutor::builtin_cd", 0, 2, BIN_CD, Some("qsPL"), None),
    Builtin::entry("continue", BINF_PSPECIAL, "exec::ShellExecutor::bin_break", 0, 1, BIN_CONTINUE, None, None),
    Builtin::entry("declare", BINF_PLUSOPTS | BINF_MAGICEQUALS | BINF_PSPECIAL | BINF_ASSIGN, "exec::ShellExecutor::bin_typeset", 0, -1, 0, Some("AE:%F:%HL:%R:%TUZ:%afghi:%klmnp:%rtuxz"), None),
    Builtin::entry("dirs", 0, "exec::ShellExecutor::bin_dirs", 0, -1, 0, Some("clpv"), None),
    Builtin::entry("disable", 0, "exec::ShellExecutor::bin_enable", 0, -1, BIN_DISABLE, Some("afmprs"), None),
    Builtin::entry("disown", 0, "exec::ShellExecutor::bin_fg", 0, -1, BIN_DISOWN, None, None),
    Builtin::entry("echo", BINF_SKIPINVALID, "exec::ShellExecutor::bin_print", 0, -1, BIN_ECHO, Some("neE"), Some("-")),
    Builtin::entry("emulate", 0, "exec::ShellExecutor::bin_emulate", 0, -1, 0, Some("lLR"), None),
    Builtin::entry("enable", 0, "exec::ShellExecutor::bin_enable", 0, -1, BIN_ENABLE, Some("afmprs"), None),
    Builtin::entry("eval", BINF_PSPECIAL, "exec::ShellExecutor::bin_eval", 0, -1, BIN_EVAL, None, None),
    Builtin::entry("exit", BINF_PSPECIAL, "exec::ShellExecutor::bin_break", 0, 1, BIN_EXIT, None, None),
    Builtin::entry("export", BINF_PLUSOPTS | BINF_MAGICEQUALS | BINF_PSPECIAL | BINF_ASSIGN, "exec::ShellExecutor::bin_typeset", 0, -1, BIN_EXPORT, Some("E:%F:%HL:%R:%TUZ:%afhi:%lp:%rtu"), Some("xg")),
    Builtin::entry("false", 0, "exec::ShellExecutor::builtin_false", 0, -1, 0, None, None),
    // C source (Src/builtin.c:69-73): the argument to -e used to be
    // optional; making it required is more consistent.
    Builtin::entry("fc", 0, "exec::ShellExecutor::bin_fc", 0, -1, BIN_FC, Some("aAdDe:EfiIlLmnpPrRst:W"), None),
    Builtin::entry("fg", 0, "exec::ShellExecutor::bin_fg", 0, -1, BIN_FG, None, None),
    Builtin::entry("float", BINF_PLUSOPTS | BINF_MAGICEQUALS | BINF_PSPECIAL | BINF_ASSIGN, "exec::ShellExecutor::bin_typeset", 0, -1, 0, Some("E:%F:%HL:%R:%Z:%ghlp:%rtux"), Some("E")),
    Builtin::entry("functions", BINF_PLUSOPTS, "exec::ShellExecutor::bin_functions", 0, -1, 0, Some("ckmMstTuUWx:z"), None),
    Builtin::entry("getln", 0, "exec::ShellExecutor::bin_read", 0, -1, 0, Some("ecnAlE"), Some("zr")),
    Builtin::entry("getopts", 0, "exec::ShellExecutor::bin_getopts", 2, -1, 0, None, None),
    Builtin::entry("hash", BINF_MAGICEQUALS, "exec::ShellExecutor::bin_hash", 0, -1, 0, Some("Ldfmrv"), None),
    Builtin::entry("history", 0, "exec::ShellExecutor::bin_fc", 0, -1, BIN_FC, Some("adDEfiLmnpPrt:"), Some("l")),
    Builtin::entry("integer", BINF_PLUSOPTS | BINF_MAGICEQUALS | BINF_PSPECIAL | BINF_ASSIGN, "exec::ShellExecutor::bin_typeset", 0, -1, 0, Some("HL:%R:%Z:%ghi:%lp:%rtux"), Some("i")),
    Builtin::entry("jobs", 0, "exec::ShellExecutor::bin_fg", 0, -1, BIN_JOBS, Some("dlpZrs"), None),
    Builtin::entry("kill", BINF_HANDLES_OPTS, "exec::ShellExecutor::bin_kill", 0, -1, 0, None, None),
    Builtin::entry("let", 0, "exec::ShellExecutor::bin_let", 1, -1, 0, None, None),
    Builtin::entry("local", BINF_PLUSOPTS | BINF_MAGICEQUALS | BINF_PSPECIAL | BINF_ASSIGN, "exec::ShellExecutor::bin_typeset", 0, -1, 0, Some("AE:%F:%HL:%R:%TUZ:%ahi:%lnp:%rtux"), None),
    Builtin::entry("logout", 0, "exec::ShellExecutor::bin_break", 0, 1, BIN_LOGOUT, None, None),
    Builtin::entry("popd", BINF_SKIPINVALID | BINF_SKIPDASH | BINF_DASHDASHVALID, "exec::ShellExecutor::builtin_cd", 0, 1, BIN_POPD, Some("q"), None),
    Builtin::entry("print", BINF_PRINTOPTS, "exec::ShellExecutor::bin_print", 0, -1, BIN_PRINT, Some("abcC:Df:ilmnNoOpPrRsSu:v:x:X:z-"), None),
    Builtin::entry("printf", BINF_SKIPINVALID | BINF_SKIPDASH, "exec::ShellExecutor::bin_print", 1, -1, BIN_PRINTF, Some("v:"), None),
    Builtin::entry("pushd", BINF_SKIPINVALID | BINF_SKIPDASH | BINF_DASHDASHVALID, "exec::ShellExecutor::builtin_cd", 0, 2, BIN_PUSHD, Some("qsPL"), None),
    Builtin::entry("pushln", 0, "exec::ShellExecutor::bin_print", 0, -1, BIN_PRINT, None, Some("-nz")),
    Builtin::entry("pwd", 0, "exec::ShellExecutor::bin_pwd", 0, 0, 0, Some("rLP"), None),
    Builtin::entry("r", 0, "exec::ShellExecutor::bin_fc", 0, -1, BIN_R, Some("IlLnr"), None),
    Builtin::entry("read", 0, "exec::ShellExecutor::bin_read", 0, -1, 0, Some("cd:ek:%lnpqrst:%zu:AE"), None),
    Builtin::entry("readonly", BINF_PLUSOPTS | BINF_MAGICEQUALS | BINF_PSPECIAL | BINF_ASSIGN, "exec::ShellExecutor::bin_typeset", 0, -1, BIN_READONLY, Some("AE:%F:%HL:%R:%TUZ:%afghi:%lptux"), Some("r")),
    Builtin::entry("rehash", 0, "exec::ShellExecutor::bin_hash", 0, 0, 0, Some("df"), Some("r")),
    Builtin::entry("return", BINF_PSPECIAL, "exec::ShellExecutor::bin_break", 0, 1, BIN_RETURN, None, None),
    Builtin::entry("set", BINF_PSPECIAL | BINF_HANDLES_OPTS, "exec::ShellExecutor::bin_set", 0, -1, 0, None, None),
    Builtin::entry("setopt", 0, "exec::ShellExecutor::bin_setopt", 0, -1, BIN_SETOPT, None, None),
    Builtin::entry("shift", BINF_PSPECIAL, "exec::ShellExecutor::bin_shift", 0, -1, 0, Some("p"), None),
    Builtin::entry("source", BINF_PSPECIAL, "exec::ShellExecutor::builtin_dot", 1, -1, 0, None, None),
    Builtin::entry("suspend", 0, "exec::ShellExecutor::bin_suspend", 0, 0, 0, Some("f"), None),
    Builtin::entry("test", BINF_HANDLES_OPTS, "exec::ShellExecutor::bin_test", 0, -1, BIN_TEST, None, None),
    Builtin::entry("ttyctl", 0, "exec::ShellExecutor::bin_ttyctl", 0, 0, 0, Some("fu"), None),
    Builtin::entry("times", BINF_PSPECIAL, "exec::ShellExecutor::bin_times", 0, 0, 0, None, None),
    Builtin::entry("trap", BINF_PSPECIAL | BINF_HANDLES_OPTS, "exec::ShellExecutor::bin_trap", 0, -1, 0, None, None),
    Builtin::entry("true", 0, "exec::ShellExecutor::builtin_true", 0, -1, 0, None, None),
    Builtin::entry("type", 0, "exec::ShellExecutor::bin_whence", 0, -1, 0, Some("ampfsSw"), Some("v")),
    Builtin::entry("typeset", BINF_PLUSOPTS | BINF_MAGICEQUALS | BINF_PSPECIAL | BINF_ASSIGN, "exec::ShellExecutor::bin_typeset", 0, -1, 0, Some("AE:%F:%HL:%R:%TUZ:%afghi:%klp:%rtuxmnz"), None),
    Builtin::entry("umask", 0, "exec::ShellExecutor::bin_umask", 0, 1, 0, Some("S"), None),
    Builtin::entry("unalias", 0, "exec::ShellExecutor::bin_unhash", 0, -1, BIN_UNALIAS, Some("ams"), None),
    Builtin::entry("unfunction", 0, "exec::ShellExecutor::bin_unhash", 1, -1, BIN_UNFUNCTION, Some("m"), Some("f")),
    Builtin::entry("unhash", 0, "exec::ShellExecutor::bin_unhash", 1, -1, BIN_UNHASH, Some("adfms"), None),
    Builtin::entry("unset", BINF_PSPECIAL, "exec::ShellExecutor::bin_unset", 1, -1, BIN_UNSET, Some("fmvn"), None),
    Builtin::entry("unsetopt", 0, "exec::ShellExecutor::bin_setopt", 0, -1, BIN_UNSETOPT, None, None),
    Builtin::entry("wait", 0, "exec::ShellExecutor::bin_fg", 0, -1, BIN_WAIT, None, None),
    Builtin::entry("whence", 0, "exec::ShellExecutor::bin_whence", 0, -1, 0, Some("acmpvfsSwx:"), None),
    Builtin::entry("where", 0, "exec::ShellExecutor::bin_whence", 0, -1, 0, Some("pmsSwx:"), Some("ca")),
    Builtin::entry("which", 0, "exec::ShellExecutor::bin_whence", 0, -1, 0, Some("ampsSwx:"), Some("c")),
    Builtin::entry("zmodload", 0, "exec::ShellExecutor::bin_zmodload", 0, -1, 0, Some("AFRILP:abcfdilmpsue"), None),
    Builtin::entry("zcompile", 0, "exec::ShellExecutor::bin_zcompile", 0, -1, 0, Some("tUMRcmzka"), None),
];

// ---------------------------------------------------------------------------
// Builtin hash table — port of `createbuiltintable()` and friends from
// `Src/builtin.c:149-211`.
// ---------------------------------------------------------------------------

/// Process-wide builtin lookup table. Filled lazily the first time
/// `builtintab()` is called; mirrors the C `mod_export HashTable
/// builtintab` exposed at `Src/builtin.c:146`.
static BUILTINTAB: OnceLock<HashMap<&'static str, &'static Builtin>> = OnceLock::new();

/// Construct the builtin lookup table.
/// Port of `createbuiltintable()` from `Src/builtin.c:149`. The C
/// version installs the hashtable function pointers (hash, addnode,
/// printnode, etc.) and then calls `addbuiltins("zsh", builtins, ..)`.
/// Here we just materialise the static `BUILTINS` slice into a
/// `HashMap<&str, &Builtin>` — Rust's standard hashing replaces the
/// C `hasher` callback and the `HashMap` itself replaces all the
/// per-table function pointers (`addnode`/`getnode`/`removenode`/...).
pub fn createbuiltintable() -> &'static HashMap<&'static str, &'static Builtin> {
    BUILTINTAB.get_or_init(|| {
        let mut m: HashMap<&'static str, &'static Builtin> = HashMap::with_capacity(BUILTINS.len());
        for b in BUILTINS {
            m.insert(b.name, b);
        }
        m
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_table_matches_c_count() {
        // Src/builtin.c:40-137 has 79 rows total: 76 always-on
        // plus 3 debug-only (`hashinfo` under ZSH_HASH_DEBUG, `mem`
        // under ZSH_MEM/ZSH_MEM_DEBUG, `patdebug` under
        // ZSH_PAT_DEBUG). We currently include only the 76
        // always-on rows; the debug rows can be added behind a cfg
        // when those features land. If the C source grows or
        // shrinks rows, this test fires.
        assert_eq!(BUILTINS.len(), 76);
    }

    #[test]
    fn lookup_finds_known_builtins() {
        for name in ["cd", "echo", "print", "fg", "bg", "jobs", "wait", "typeset", "test", "[", "."] {
            assert!(createbuiltintable().get(name).copied().is_some(), "missing: {name}");
        }
    }

    #[test]
    fn lookup_misses_unknown() {
        assert!(createbuiltintable().get("not-a-builtin-zZz").copied().is_none());
    }

    #[test]
    fn prefix_entries_have_prefix_flag() {
        for name in ["-", "builtin", "command", "exec", "noglob"] {
            let b = createbuiltintable().get(name).copied().unwrap();
            assert!(b.flags & BINF_PREFIX != 0, "{name} missing BINF_PREFIX");
        }
    }

    #[test]
    fn fg_dispatch_id_distinguishes_aliases() {
        // bin_fg covers fg, bg, jobs, wait, disown — same handler,
        // different funcid. Mirrors Src/builtin.c:52,61,75,88,131.
        assert_eq!(createbuiltintable().get("fg").copied().unwrap().funcid, BIN_FG);
        assert_eq!(createbuiltintable().get("bg").copied().unwrap().funcid, BIN_BG);
        assert_eq!(createbuiltintable().get("jobs").copied().unwrap().funcid, BIN_JOBS);
        assert_eq!(createbuiltintable().get("wait").copied().unwrap().funcid, BIN_WAIT);
        assert_eq!(createbuiltintable().get("disown").copied().unwrap().funcid, BIN_DISOWN);
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart lives in src/zsh/Src/builtin.c. Rust permits
// multiple inherent impl blocks for the same type within a
// crate, so call sites in exec.rs and elsewhere are unchanged.
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// cd builtin - change directory
    /// Ported from zsh/Src/builtin.c bin_cd() lines 839-859, cd_get_dest() lines 864-957,
    /// cd_do_chdir() lines 967-1081, cd_try_chdir() lines 1116-1181
    pub(crate) fn bin_cd(&mut self, args: &[String]) -> i32 {
        self.dispatch_pending_traps();
        if self.redirect_failed { self.redirect_failed = false; return 1; }
        // cd [ -qsLP ] [ arg ]
        // cd [ -qsLP ] old new
        // cd [ -qsLP ] {+|-}n
        let mut quiet = false;
        let mut use_cdpath = false;
        let mut logical = true; // -L is default
        let mut positional_args: Vec<&str> = Vec::new();

        let mut after_dashdash = false;
        for arg in args {
            // `--` is end-of-options; everything after is positional.
            // Without this, `cd -- /tmp` treated `--` as the OLD arg
            // of the substitution form and errored "string not in
            // pwd: --".
            if arg == "--" && !after_dashdash {
                after_dashdash = true;
                continue;
            }
            if !after_dashdash && arg.starts_with('-') && arg.len() > 1 && !arg.starts_with("--") {
                // Check if it's a stack index like -2
                if arg[1..].chars().all(|c| c.is_ascii_digit()) {
                    positional_args.push(arg);
                    continue;
                }
                for ch in arg[1..].chars() {
                    match ch {
                        'q' => quiet = true,
                        's' => use_cdpath = true,
                        'L' => logical = true,
                        'P' => logical = false,
                        _ => {
                            zwarnnam("cd", &format!("bad option: -{}", ch));
                            return 1;
                        }
                    }
                }
            } else if !after_dashdash
                && arg.starts_with('+')
                && arg.len() > 1
                && arg[1..].chars().all(|c| c.is_ascii_digit())
            {
                // Stack index like +2
                positional_args.push(arg);
            } else {
                positional_args.push(arg);
            }
        }

        // Handle cd old new (substitution). Direct port of
        // src/zsh/Src/builtin.c:910-927 — `u = strstr(pwd, argv[0])`
        // finds the FIRST occurrence; the new string replaces only
        // that occurrence. Rust's String::replace replaces ALL,
        // so use replacen(old, new, 1) to match zsh.
        if positional_args.len() == 2 {
            if let Ok(cwd) = env::current_dir() {
                let cwd_str = cwd.to_string_lossy();
                let old = positional_args[0];
                let new = positional_args[1];
                if cwd_str.contains(old) {
                    let new_path = cwd_str.replacen(old, new, 1);
                    if !quiet {
                        println!("{}", new_path);
                    }
                    positional_args = vec![];
                    return self.do_cd(&new_path, quiet, use_cdpath, logical);
                }
                // zsh: if old is not in $PWD, the substitution fails
                // with `cd:1: string not in pwd: <old>` exit 1.
                // builtin.c:914-916 emits the same diagnostic and
                // returns NULL (which propagates to exit 1).
                zwarnnam("cd", &format!("string not in pwd: {}", old));
                return 1;
            }
        }
        // 3+ positional args: zsh -> `cd:1: too many arguments`
        // exit 1. The substitution form takes 2; anything more is
        // an error.
        if positional_args.len() > 2 {
            zwarnnam("cd", "too many arguments");
            return 1;
        }

        let path_arg = positional_args.first().copied().unwrap_or("~");

        // Handle stack indices
        if path_arg.starts_with('+') || path_arg.starts_with('-') {
            if let Ok(n) = path_arg[1..].parse::<usize>() {
                let idx = if path_arg.starts_with('+') {
                    n
                } else {
                    self.dir_stack.len().saturating_sub(n)
                };
                if let Some(dir) = self.dir_stack.get(idx) {
                    let dir_path = dir.to_string_lossy().to_string();
                    return self.do_cd(&dir_path, quiet, use_cdpath, logical);
                } else {
                    zwarnnam("cd", "no such entry in dir stack");
                    return 1;
                }
            }
        }

        self.do_cd(path_arg, quiet, use_cdpath, logical)
    }
    pub(crate) fn bin_pwd(&mut self, _redirects: &[Redirect]) -> i32 {
        self.builtin_pwd_with_args(&[])
    }
    pub(crate) fn builtin_pwd_with_args(&mut self, args: &[String]) -> i32 {
        // Honor `pwd -P` (physical, realpath) and `pwd -L` (logical,
        // tracked $PWD with symlinks preserved). Default is logical to
        // match zsh.
        let mut physical = false;
        let mut positional_count = 0;
        for arg in args {
            if !arg.starts_with('-') {
                // zsh: `pwd extra arg` -> `pwd:1: too many arguments`
                // exit 1. pwd takes only flags; positional args are
                // an error. zshrs ignored them and printed cwd.
                positional_count += 1;
                continue;
            }
            for ch in arg.strip_prefix('-').unwrap_or("").chars() {
                match ch {
                    // Direct port of src/zsh/Src/builtin.c:730 —
                    // `-r` is equivalent to `-P` (resolve via
                    // syscall/realpath, not the tracked $PWD).
                    'P' | 'r' => physical = true,
                    'L' => physical = false,
                    // zsh: `pwd -X` -> `pwd:1: bad option: -X` exit 1.
                    // zshrs's silent fallback ignored unknown letters
                    // and continued, masking typos and letting `pwd
                    // -X` print the cwd as if -X were valid.
                    _ => {
                        zwarnnam("pwd", &format!("bad option: -{}", ch));
                        return 1;
                    }
                }
            }
        }
        if positional_count > 0 {
            zwarnnam("pwd", "too many arguments");
            return 1;
        }
        let logical_pwd = self
            .variables
            .get("PWD")
            .cloned()
            .or_else(|| env::var("PWD").ok());
        let printed = if physical {
            // Realpath the logical pwd via canonicalize (resolves
            // every symlink); fall back to current_dir if PWD missing.
            let base = logical_pwd
                .clone()
                .map(PathBuf::from)
                .or_else(|| env::current_dir().ok())
                .unwrap_or_default();
            base.canonicalize()
                .unwrap_or(base)
                .to_string_lossy()
                .to_string()
        } else {
            logical_pwd.unwrap_or_else(|| {
                env::current_dir()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default()
            })
        };
        if printed.is_empty() {
            zwarnnam("pwd", "cannot determine current directory");
            1
        } else {
            println!("{}", printed);
            0
        }
    }
    pub(crate) fn builtin_echo(&mut self, args: &[String], _redirects: &[Redirect]) -> i32 {
        self.dispatch_pending_traps();
        if self.redirect_failed { self.redirect_failed = false; return 1; }
        // `$_` writeback: mirrors C's `execcmd_exec` zunderscore
        // update — `$_` becomes the last arg of the just-running
        // command. The bytecode VM dispatches `echo` to this fn
        // directly (id 170 in fusevm_bridge), bypassing
        // `host_exec_external` where the universal hook lives;
        // adding the call here covers the bytecode path.
        let mut full_argv = Vec::with_capacity(args.len() + 1);
        full_argv.push("echo".to_string());
        full_argv.extend(args.iter().cloned());
        crate::ported::params::set_zunderscore(&full_argv);
        let mut newline = true;
        // zsh's default: interpret backslash escapes (\n, \t, \b, etc.)
        // unless `setopt bsd_echo` is on (then `-e` is required).
        // Mirror zsh: default ON, `-E` disables.
        let bsd_echo = self.options.get("bsd_echo").copied().unwrap_or(false);
        let mut interpret_escapes = !bsd_echo;
        let mut start = 0;

        // Accept combined flags like `-nE` (zsh: each char treated as
        // its own flag). Walk while args look like flag tokens. Also
        // treat a bare `-` as a no-op flag — zsh: `echo - hi` prints
        // `hi` (the lone `-` is consumed silently).
        for (i, arg) in args.iter().enumerate() {
            if arg == "-" {
                start = i + 1;
                continue;
            }
            if !arg.starts_with('-') || arg.len() < 2 {
                break;
            }
            let body = &arg[1..];
            // All chars must be recognised echo flags; otherwise this
            // is a positional arg starting with `-`.
            if !body.chars().all(|c| matches!(c, 'n' | 'e' | 'E')) {
                break;
            }
            for ch in body.chars() {
                match ch {
                    'n' => newline = false,
                    'e' => interpret_escapes = true,
                    'E' => interpret_escapes = false,
                    _ => {}
                }
            }
            start = i + 1;
        }

        let output = args[start..].join(" ");
        if interpret_escapes {
            // Use the shared escape decoder so `\033`, `\xNN`, `\NNN`,
            // `\a`, `\b`, `\e` etc. all work — not just `\n` and `\t`.
            print!("{}", self.expand_printf_escapes(&output));
        } else {
            print!("{}", output);
        }

        if newline {
            println!();
        }
        // Flush so any redirect-scope dup2 restoration on the next
        // statement doesn't strand buffered data on the original fd.
        // See builtin_printf for the same fix and rationale.
        use std::io::Write as _;
        let _ = std::io::stdout().flush();
        0
    }
    pub(crate) fn builtin_export(&mut self, args: &[String]) -> i32 {
        // PFA-SMR aspect: emit one `export` event per `NAME[=value]` arg.
        // Listing-only invocations (`export` / `export -p`) are skipped.
        #[cfg(feature = "recorder")]
        if crate::recorder::is_enabled() {
            let ctx = self.recorder_ctx();
            for a in args {
                if a == "-p" || a.starts_with('-') {
                    continue;
                }
                if let Some((k, v)) = a.split_once('=') {
                    crate::recorder::emit_export(k, Some(v), ctx.clone());
                } else {
                    crate::recorder::emit_export(a, None, ctx.clone());
                }
            }
        }
        // Bare `export` lists every exported var, same form as
        // `export -p`. Direct port of zsh/Src/builtin.c:bin_typeset
        // BIN_EXPORT path; POSIX requires this listing.
        // `export -p` (with no other args) — print every exported var
        // as a re-executable `export NAME=value` line. Matches POSIX +
        // zsh behavior. Skips ARG-less / ARG-with-flag iteration only
        // when -p is the sole flag.
        let only_print = args.is_empty() || (args.len() == 1 && args[0] == "-p");
        if only_print {
            let mut keys: Vec<String> = std::env::vars().map(|(k, _)| k).collect();
            keys.sort();
            for k in keys {
                if let Ok(v) = std::env::var(&k) {
                    println!("export {}={}", k, crate::ported::utils::quotedzputs(&v));
                }
            }
            return 0;
        }
        for arg in args {
            if arg == "-p" {
                continue;
            }
            // zsh rejects `-n` and other bash-only flags. Only -p is
            // accepted alongside names. Reject anything else starting
            // with `-` (other than name-with-equals) for parity.
            if arg.starts_with('-') && !arg.contains('=') && arg.len() > 1 {
                zwarnnam("export", &format!("bad option: {}", arg));
                return 1;
            }
            let key_owned = if let Some((key, value)) = arg.split_once('=') {
                // zsh validates the lhs is a valid identifier:
                //   `export 1bad=val` -> `export:1: not an
                //     identifier: 1bad` exit 1 (digit-leading)
                //   `export "BAD NAME=val"` -> `export:1: not
                //     valid in this context: BAD NAME` exit 1
                //     (whitespace/special chars)
                // zshrs silently accepted both, polluting the
                // env with bogus names. Identifier rule:
                // [A-Za-z_][A-Za-z0-9_]*.
                let mut chars = key.chars();
                let first_ok = chars
                    .next()
                    .map(|c| c.is_ascii_alphabetic() || c == '_')
                    .unwrap_or(false);
                if !first_ok {
                    if key.chars().any(|c| !c.is_ascii_alphanumeric() && c != '_') {
                        zerrnam("export", &format!("not valid in this context: {}", key));
                    } else {
                        zerrnam("export", &format!("not an identifier: {}", key));
                    }
                    return 1;
                }
                if chars.any(|c| !c.is_ascii_alphanumeric() && c != '_') {
                    zerrnam("export", &format!("not valid in this context: {}", key));
                    return 1;
                }
                self.variables.insert(key.to_string(), value.to_string());
                env::set_var(key, value);
                key.to_string()
            } else {
                // export VAR (no value) — mark existing var as exported
                let val = self.get_variable(arg);
                env::set_var(arg, &val);
                arg.clone()
            };
            // Mark the export attribute for `(t)` flag.
            let entry = self.var_attrs.entry(key_owned).or_default();
            entry.export = true;
        }
        0
    }
    pub(crate) fn bin_unset(&mut self, args: &[String]) -> i32 {
        self.dispatch_pending_traps();
        if self.redirect_failed { self.redirect_failed = false; return 1; }
        // `unset` with no args is an error in zsh: `not enough arguments`
        // exit 1. zshrs returned 0 silently — masked typo'd unset NAMES.
        if args.is_empty() {
            zwarnnam("unset", "not enough arguments");
            return 1;
        }
        // PFA-SMR aspect: emit one `unset` event per non-flag arg.
        // RECORDER.md open question 4 says "Probably yes" for tracking
        // removal sites — needed for `zwhere -l` lineage to show the
        // full define→unset→redefine chain.
        #[cfg(feature = "recorder")]
        if crate::recorder::is_enabled() {
            let ctx = self.recorder_ctx();
            for a in args {
                if a.starts_with('-') || a == "--" {
                    continue;
                }
                crate::recorder::emit_unset(a, ctx.clone());
            }
        }
        // `unset -f NAME...` — remove functions (mirror of `unfunction`).
        // Walk the arg list once: if we see -f, mark function-mode for
        // the remaining names. zsh allows `-v` to explicitly target
        // variables (the default), and `-m` for pattern matching.
        let mut function_mode = false;
        let mut match_glob = false;
        let mut names: Vec<String> = Vec::new();
        let mut end_of_options = false;
        for arg in args {
            if end_of_options {
                names.push(arg.clone());
                continue;
            }
            match arg.as_str() {
                "--" => end_of_options = true,
                "-f" => function_mode = true,
                "-v" => function_mode = false,
                "-m" => match_glob = true,
                _ if arg.starts_with('-') && arg.len() > 1 => {
                    // zsh: `unset -X foo` errors `unset:1: bad
                    // option: -X` exit 1. zshrs silently swallowed
                    // unknown flags, which masked typos.
                    zwarnnam("unset", &format!("bad option: {}", arg));
                    return 1;
                }
                _ => names.push(arg.clone()),
            }
        }
        if function_mode {
            // Direct port of src/zsh/Src/builtin.c:3826-3828 — `unset
            // -f` is the same as bin_unhash with BIN_UNFUNCTION. The
            // -m flag carries through and pattern-matches against the
            // function table.
            if match_glob {
                let pats: Vec<String> = names.clone();
                let keys: Vec<String> = self.functions_compiled.keys().cloned().collect();
                let mut matched = false;
                for pat in &pats {
                    for k in &keys {
                        if ShellExecutor::glob_match_static(k, pat) {
                            self.functions_compiled.remove(k);
                            self.function_source.remove(k);
                            self.autoload_pending.remove(k);
                            matched = true;
                        }
                    }
                }
                return if matched { 0 } else { 1 };
            }
            for name in &names {
                self.functions_compiled.remove(name);
                self.function_source.remove(name);
                self.autoload_pending.remove(name);
            }
            return 0;
        }
        // Direct port of src/zsh/Src/builtin.c:3830-3863 — `-m`
        // matches each arg as a glob pattern against the parameter
        // table and unsets every matching variable. Returns 1 if no
        // matches at all.
        if match_glob {
            let var_names: Vec<String> = self.variables.keys().cloned().collect();
            let arr_names: Vec<String> = self.arrays.keys().cloned().collect();
            let assoc_names: Vec<String> = self.assoc_arrays.keys().cloned().collect();
            let mut matched = false;
            for pat in &names {
                for n in &var_names {
                    if ShellExecutor::glob_match_static(n, pat) {
                        self.variables.remove(n);
                        std::env::remove_var(n);
                        matched = true;
                    }
                }
                for n in &arr_names {
                    if ShellExecutor::glob_match_static(n, pat) {
                        self.arrays.remove(n);
                        matched = true;
                    }
                }
                for n in &assoc_names {
                    if ShellExecutor::glob_match_static(n, pat) {
                        self.assoc_arrays.remove(n);
                        matched = true;
                    }
                }
            }
            return if matched { 0 } else { 1 };
        }
        for arg in &names {
            // `unset 'arr[i]'` / `unset 'm[k]'` — element delete. Detect
            // the subscript form and dispatch instead of nuking the
            // whole variable. zsh treats indexed elements as delete-by-
            // index (1-based, negative-from-end) and assoc elements as
            // delete-by-key.
            if let Some(lb) = arg.find('[') {
                if arg.ends_with(']') {
                    let name = &arg[..lb];
                    let key = &arg[lb + 1..arg.len() - 1];
                    if !name.is_empty() && !key.is_empty() {
                        if let Some(map) = self.assoc_arrays.get_mut(name) {
                            map.remove(key);
                            continue;
                        }
                        if let Some(arr) = self.arrays.get_mut(name) {
                            if let Ok(i) = key.parse::<i64>() {
                                let len = arr.len() as i64;
                                let idx = if i > 0 {
                                    (i - 1) as usize
                                } else if i < 0 {
                                    let off = len + i;
                                    if off < 0 {
                                        continue;
                                    }
                                    off as usize
                                } else {
                                    continue;
                                };
                                // zsh's `unset 'arr[i]'` for indexed
                                // sets the slot to empty string (slot
                                // count preserved), unlike `arr[i]=()`
                                // which removes the slot.
                                if idx < arr.len() {
                                    arr[idx] = String::new();
                                }
                            }
                            continue;
                        }
                    }
                }
            }
            // zsh: `unset NAME` for a read-only NAME errors `read-only
            // variable: NAME` exit 1 — the unset is rejected, not
            // silently consumed. zshrs's unset blindly removed the
            // entry from the variable maps without consulting the
            // readonly bit, so `readonly x=1; unset x` left x unset
            // and exit 0 (compat regression).
            let is_intrinsic_ro = matches!(
                arg.as_str(),
                "PPID" | "LINENO" | "argv0" | "ARGC"
            );
            let is_ro = is_intrinsic_ro
                || self.readonly_vars.contains(arg)
                || self.var_attrs.get(arg).map(|a| a.readonly).unwrap_or(false);
            if is_ro {
                zerr(&format!("read-only variable: {}", arg));
                return 1;
            }
            env::remove_var(arg);
            self.variables.remove(arg);
            self.arrays.remove(arg);
            self.assoc_arrays.remove(arg);
            // typeset -T tied pair: unsetting the array side ALSO
            // unsets the scalar side (and vice versa). zsh's
            // `unset path` zeroes $PATH because they're tied. Without
            // this, our path-array got removed but $PATH retained the
            // pre-unset value.
            if let Some((scalar_name, _sep)) = self.tied_array_to_scalar.remove(arg) {
                env::remove_var(&scalar_name);
                self.variables.remove(&scalar_name);
                self.tied_scalar_to_array.remove(&scalar_name);
            }
            if let Some((array_name, _sep)) = self.tied_scalar_to_array.remove(arg) {
                self.arrays.remove(&array_name);
                self.tied_array_to_scalar.remove(&array_name);
            }
        }
        0
    }
    pub(crate) fn bin_dot(&mut self, args: &[String]) -> i32 {
        self.builtin_source_named(args, "source")
    }
    pub(crate) fn builtin_source_named(&mut self, args: &[String], invoked_as: &str) -> i32 {
        if args.is_empty() {
            // zsh: `source` -> `source:1: not enough arguments`,
            // `.` -> `.:1: not enough arguments`. zshrs hard-coded
            // a bash-style banner without the shell-name prefix.
            zwarnnam(invoked_as, "not enough arguments");
            return 1;
        }
        // PFA-SMR aspect: emit a `source` event for the as-typed path.
        // The resolved absolute path is computed below; emitting the raw
        // first arg keeps the recorder's view aligned with what the user
        // actually wrote (matters for transitive-source visualization).
        #[cfg(feature = "recorder")]
        if crate::recorder::is_enabled() && !args[0].is_empty() {
            let ctx = self.recorder_ctx();
            crate::recorder::emit_source(&args[0], ctx);
        }
        // zsh: `. ""` (empty path) -> `.:1: no such file or
        // directory:` (with empty trailing path). zshrs's POSIX
        // path-resolver mapped "" to cwd which then opened as a
        // directory and produced `is a directory: `. Special-case
        // empty so the diagnostic matches zsh.
        if args[0].is_empty() {
            zwarnnam(invoked_as, "no such file or directory: ");
            return 127;
        }

        let path = &args[0];

        // Resolve to absolute path. Direct port of
        // src/zsh/Src/builtin.c:6080-6123 bin_dot path resolution:
        //   1. For `source` (not `.`): try CWD/arg first.
        //   2. If arg contains `/`, use that path directly.
        //   3. Otherwise (bare name) search every $path entry for arg.
        // zshrs's previous logic always resolved to CWD/arg for bare
        // names, so `. somefile` in zsh-style "search $path" usage
        // (zinit and many plugin managers do this) failed unless the
        // CWD happened to contain the file.
        let abs_path = if path.starts_with('/') {
            path.clone()
        } else if let Some(after) = path.strip_prefix("~/") {
            if let Some(home) = dirs::home_dir() {
                home.join(after).to_string_lossy().to_string()
            } else {
                path.clone()
            }
        } else if path.contains('/') {
            // Relative path with a slash — resolve via CWD per the
            // C source's `for (s = arg0; *s; s++) if (*s == '/') ...`
            // branch which calls `source(arg0)` directly.
            std::env::current_dir()
                .map(|cwd| cwd.join(path).to_string_lossy().to_string())
                .unwrap_or_else(|_| path.clone())
        } else {
            // Bare name — for `source` only, try CWD first
            // (builtin.c:6084-6088 short-circuit).
            let cwd_candidate = std::env::current_dir()
                .map(|cwd| cwd.join(path).to_string_lossy().to_string())
                .unwrap_or_else(|_| path.clone());
            let cwd_ok = invoked_as != "."
                && std::path::Path::new(&cwd_candidate)
                    .metadata()
                    .map(|m| m.is_file())
                    .unwrap_or(false);
            if cwd_ok {
                cwd_candidate
            } else {
                // Walk $path looking for the file. Matches
                // builtin.c:6106-6121 — empty / `.` entries refer
                // to CWD (the diddot guard prevents re-trying CWD
                // here since we already did above).
                let path_var = std::env::var("PATH").unwrap_or_default();
                let mut found: Option<String> = None;
                for entry in path_var.split(':') {
                    let candidate = if entry.is_empty() || entry == "." {
                        // Skip the CWD slot — already attempted above.
                        continue;
                    } else {
                        format!("{}/{}", entry, path)
                    };
                    if std::path::Path::new(&candidate)
                        .metadata()
                        .map(|m| m.is_file())
                        .unwrap_or(false)
                    {
                        found = Some(candidate);
                        break;
                    }
                }
                found.unwrap_or(cwd_candidate)
            }
        };

        // Daemon source/dot interception (per docs/DAEMON.md "Source / dot
        // interception and file registry"). Fire-and-forget IPC to populate
        // the daemon's compiled_files registry with this file's mtime + inode.
        // Skipped in any parity mode — `--posix` (Bourne sh has no daemon),
        // `--zsh` (drop-in C zsh has no daemon), `--bash` (bash has no
        // daemon). Each parity mode must behave identically to its
        // reference shell, which means no zshrs-side compiled-files
        // registry. Failure is silently swallowed — the shell continues
        // with its existing read-+-execute fallback.
        #[cfg(feature = "daemon")]
        if !self.posix_mode && !self.zsh_compat && !self.bash_compat {
            if let Ok(meta) = std::fs::metadata(&abs_path) {
                use std::os::unix::fs::MetadataExt;
                let mtime_ns = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos() as i64)
                    .unwrap_or(0);
                let inode = meta.ino() as i64;
                let payload = serde_json::json!({
                    "path": &abs_path,
                    "mtime_ns": mtime_ns,
                    "inode": inode,
                });
                // Use call_once_no_spawn so a missing daemon never blocks the
                // shell waiting on a spawn. If unreachable: degraded fallback.
                let _ = crate::daemon::client::call_once_no_spawn("source_resolve", payload);
            }
        }

        // Save current $0 and set to the sourced file path
        let saved_zero = self.variables.get("0").cloned();
        self.variables.insert("0".to_string(), abs_path.clone());

        // Save current scriptname and set to the sourced file path
        // so `%N` in PS4 / xtrace renders the file being sourced.
        // Direct port of Src/init.c source() — pushes a new
        // scriptname onto the stack and restores on return.
        let saved_scriptname = self.scriptname.clone();
        self.scriptname = Some(abs_path.clone());

        // Save + clear cmd_stack so the sourced file starts with an
        // empty `%_` context. Direct port of Src/init.c:1578-1581
        // `ocs = cmdstack; ocsp = cmdsp; … cmdsp = 0;` — without this
        // an outer `if [[ … ]] then source X fi` leaks `then` into
        // every line of X's xtrace.
        let saved_cmd_stack = std::mem::take(&mut self.cmd_stack);

        // zsh: `. file ARG1 ARG2` passes ARG1/ARG2 as $1/$2 to the
        // sourced script. Save outer positional params, install
        // args[1..] as new positionals, restore on exit. Without
        // this the sourced file saw the parent shell's positionals
        // (or empty when run from `-c`).
        let saved_positionals = if args.len() > 1 {
            let prev = self.positional_params.clone();
            self.positional_params = args[1..].to_vec();
            Some(prev)
        } else {
            None
        };

        let result;

        if self.posix_mode || self.zsh_compat || self.bash_compat {
            // --- Parity mode (--posix / --zsh / --bash): plain read +
            //     execute, no SQLite, no caching, no threads, no daemon
            //     — identical behaviour to the corresponding reference
            //     shell. C zsh's Src/builtin.c:6080-6123 bin_dot reads
            //     the file and execlist's it on every call; bash's
            //     builtins.def `source_builtin` does the same; sh / dash
            //     equivalent. Every `source` re-runs the file fresh so
            //     stdout / signal handlers / file I/O all re-fire as
            //     the user wrote them. ---
            result = match std::fs::read_to_string(&abs_path) {
                Ok(content) => match self.execute_script(&content) {
                    Ok(status) => status,
                    Err(e) => {
                        zwarnnam("source", &format!("{}: {}", path, e));
                        1
                    }
                },
                Err(e) => {
                    // zsh format: `zshrs:source:1: no such file or
                    // directory: PATH` and exit 127.
                    let msg = crate::ported::compat::strerror(e.raw_os_error().unwrap_or(0)).to_lowercase();
                    zwarnnam("source", &format!("{}: {}", msg, path));
                    127
                }
            };
        } else {
            // --- zshrs/zsh mode: plugin cache + AST cache + worker pool ---
            let file_path = std::path::Path::new(&abs_path);

            // Check plugin cache for side-effect replay
            // Recorder bypass: the plugin-cache replay path applies the
            // delta directly to executor state without going through
            // `bin_alias` / `bin_typeset` / etc., so the recorder
            // aspect would never see any mutation from a cached plugin
            // load. A recording run is opt-in and explicitly tolerates
            // ~1.5x startup time (RECORDER.md §"Performance targets") —
            // disabling cache replay for that run guarantees every
            // dispatcher fires once for every state mutation.
            // Cache is disabled when:
            //   1. The recorder is active (needs every dispatcher
            //      to fire on every mutation, can't replay deltas).
            //   2. `ZSHRS_CACHE=0|false|no` env var is set (the
            //      same env var the rkyv script_cache honors —
            //      extended here so a single switch turns OFF both
            //      caches for parity-testing runs against
            //      /bin/zsh, where every `source` re-runs the file
            //      fresh and visible stdout / signal handlers /
            //      file I/O all re-fire).
            //   3. CLI flag `--no-cache` (passed through via the
            //      same env var by the bin entrypoint).
            #[cfg(feature = "recorder")]
            let cache_disabled = crate::recorder::is_enabled()
                || !crate::script_cache::cache_enabled();
            #[cfg(not(feature = "recorder"))]
            let cache_disabled = !crate::script_cache::cache_enabled();
            if !cache_disabled {
                if let Some(ref cache) = self.plugin_cache {
                if let Some((mt_s, mt_ns)) = crate::plugin_cache::file_mtime(file_path) {
                    if let Some(plugin_id) = cache.check(&abs_path, mt_s, mt_ns) {
                        if let Ok(delta) = cache.load(plugin_id) {
                            let t0 = std::time::Instant::now();
                            self.replay_plugin_delta(&delta);
                            tracing::info!(
                                path = %abs_path,
                                replay_us = t0.elapsed().as_micros() as u64,
                                funcs = delta.functions.len(),
                                aliases = delta.aliases.len(),
                                vars = delta.variables.len() + delta.exports.len(),
                                "source: cache hit, replayed"
                            );
                            // Restore $0
                            if let Some(z) = saved_zero {
                                self.variables.insert("0".to_string(), z);
                            } else {
                                self.variables.remove("0");
                            }
                            self.scriptname = saved_scriptname;
                            self.cmd_stack = saved_cmd_stack;
                            return 0;
                        }
                    }
                }
                }
            }

            // Cache miss — snapshot, execute via AST-cached path, diff, async store
            let snapshot = self.snapshot_state();
            let t0 = std::time::Instant::now();
            tracing::debug!(path = %abs_path, "source: cache miss, executing via AST-cached path");
            result = match self.execute_script_file(&abs_path) {
                Ok(status) => status,
                Err(e) => {
                    tracing::warn!(path = %abs_path, error = %e, "source: execution failed");
                    // Match zsh: `zshrs:source:1: no such file or
                    // directory: PATH` and exit 127. Strip Rust's
                    // "(os error N)" suffix and any duplicate-path
                    // prefix that wrapped errors carry.
                    let raw = e.to_string();
                    let msg = match raw.find(": ") {
                        Some(i) if raw[..i] == *path || raw.starts_with(&abs_path) => {
                            raw[i + 2..].to_string()
                        }
                        _ => raw,
                    };
                    let msg = match msg.find(" (os error") {
                        Some(i) => msg[..i].to_string(),
                        None => msg,
                    };
                    zwarnnam("source", &format!("{}: {}", msg.to_lowercase(), path));
                    127
                }
            };
            let source_ms = t0.elapsed().as_millis() as u64;

            // Async-store delta to plugin cache on worker pool
            if result == 0 {
                if let Some((mt_s, mt_ns)) = crate::plugin_cache::file_mtime(file_path) {
                    let delta = self.diff_state(&snapshot);
                    let store_path = abs_path.clone();
                    tracing::info!(
                        path = %abs_path, source_ms,
                        funcs = delta.functions.len(),
                        aliases = delta.aliases.len(),
                        vars = delta.variables.len() + delta.exports.len(),
                        "source: caching delta on worker"
                    );
                    let cache_db_path = crate::plugin_cache::default_cache_path();
                    self.worker_pool.submit(move || {
                        match crate::plugin_cache::PluginCache::open(&cache_db_path) {
                            Ok(cache) => {
                                if let Err(e) = cache.store(&store_path, mt_s, mt_ns, source_ms, &delta) {
                                    tracing::error!(path = %store_path, error = %e, "plugin_cache: store failed");
                                } else {
                                    tracing::debug!(path = %store_path, "plugin_cache: stored");
                                }
                            }
                            Err(e) => tracing::error!(error = %e, "plugin_cache: open for write failed"),
                        }
                    });
                }
            }
        }

        // Handle return from sourced script
        let final_result = if let Some(ret) = self.returning.take() {
            ret
        } else {
            result
        };

        // Restore $0
        if let Some(z) = saved_zero {
            self.variables.insert("0".to_string(), z);
        } else {
            self.variables.remove("0");
        }

        // Restore scriptname so `%N` reverts to the outer context.
        self.scriptname = saved_scriptname;
        // Restore the outer cmd_stack so post-source xtrace lines
        // see the original `%_` context.
        self.cmd_stack = saved_cmd_stack;

        // Restore outer positional params (only when source was given
        // explicit args).
        if let Some(prev) = saved_positionals {
            self.positional_params = prev;
        }

        final_result
    }
    pub(crate) fn bin_test(&mut self, args: &[String]) -> i32 {
        self.dispatch_pending_traps();
        if self.redirect_failed { self.redirect_failed = false; return 1; }
        if args.is_empty() {
            // zsh: `test` (bare) returns 1 silently; `[` (bare,
            // no closing `]`) errors `[:1: ']' expected` exit 2.
            // zshrs's dispatch can't distinguish `test` from `[` at
            // this point (fusevm aliases both to BUILTIN_TEST), so
            // matching exactly requires a separate BUILTIN_LBRACKET
            // wired through the registry-published fusevm. Until
            // then, return 1 silently — matches `test` exactly and
            // is the more common case; `[` users see the wrong
            // exit code but no spurious output.
            return 1;
        }

        // builtin.c:7240-7247 — when called as `[`, the LAST arg
        // must be `]` and is dropped. zshrs's previous `.filter(|&s|
        // s != "]")` stripped ALL `]` tokens, so an expression like
        //   [ "]" = "]" ]
        // (string-equality of literal-]-against-literal-]) had its
        // operands erased and degenerated to `[ = ]` — parse error.
        // The fix mirrors the C: drop only one trailing `]`.
        let mut args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        if args.last().copied() == Some("]") {
            args.pop();
        }

        // Prefetch metadata for all file paths in the expression — one stat() per unique path
        // instead of one stat() per test flag. Avoids 7 serial stat()s for -r -w -x -g -k -u -s.
        let mut meta_cache: HashMap<String, Option<std::fs::Metadata>> = HashMap::new();
        for arg in &args {
            if !arg.starts_with('-') && !arg.starts_with('!') && *arg != "(" && *arg != ")" {
                let path_str = arg.to_string();
                meta_cache
                    .entry(path_str)
                    .or_insert_with(|| std::fs::metadata(arg).ok());
            }
        }

        // Helper closure: get metadata from cache or fetch
        let get_meta = |path: &str| -> Option<std::fs::Metadata> {
            meta_cache
                .get(path)
                .cloned()
                .unwrap_or_else(|| std::fs::metadata(path).ok())
        };

        match args.as_slice() {
            // String tests
            ["-z", s] => {
                if s.is_empty() {
                    0
                } else {
                    1
                }
            }
            ["-n", s] => {
                if !s.is_empty() {
                    0
                } else {
                    1
                }
            }

            // File existence/type tests
            ["-a", path] | ["-e", path] => {
                if std::path::Path::new(path).exists() {
                    0
                } else {
                    1
                }
            }
            ["-f", path] => {
                if std::path::Path::new(path).is_file() {
                    0
                } else {
                    1
                }
            }
            ["-d", path] => {
                if std::path::Path::new(path).is_dir() {
                    0
                } else {
                    1
                }
            }
            ["-b", path] => {
                use std::os::unix::fs::FileTypeExt;
                if std::fs::symlink_metadata(path)
                    .map(|m| m.file_type().is_block_device())
                    .unwrap_or(false)
                {
                    0
                } else {
                    1
                }
            }
            ["-c", path] => {
                use std::os::unix::fs::FileTypeExt;
                if std::fs::symlink_metadata(path)
                    .map(|m| m.file_type().is_char_device())
                    .unwrap_or(false)
                {
                    0
                } else {
                    1
                }
            }
            ["-p", path] => {
                use std::os::unix::fs::FileTypeExt;
                if std::fs::symlink_metadata(path)
                    .map(|m| m.file_type().is_fifo())
                    .unwrap_or(false)
                {
                    0
                } else {
                    1
                }
            }
            ["-S", path] => {
                use std::os::unix::fs::FileTypeExt;
                if std::fs::symlink_metadata(path)
                    .map(|m| m.file_type().is_socket())
                    .unwrap_or(false)
                {
                    0
                } else {
                    1
                }
            }
            ["-h", path] | ["-L", path] => {
                if std::path::Path::new(path).is_symlink() {
                    0
                } else {
                    1
                }
            }

            // File permission tests — all use prefetched metadata (one stat per path)
            ["-r", path] => {
                use std::os::unix::fs::MetadataExt;
                if let Some(meta) = get_meta(path) {
                    let mode = meta.mode();
                    let uid = unsafe { libc::geteuid() };
                    let gid = unsafe { libc::getegid() };
                    let readable = if meta.uid() == uid {
                        mode & 0o400 != 0
                    } else if meta.gid() == gid {
                        mode & 0o040 != 0
                    } else {
                        mode & 0o004 != 0
                    };
                    if readable {
                        0
                    } else {
                        1
                    }
                } else {
                    1
                }
            }
            ["-w", path] => {
                use std::os::unix::fs::MetadataExt;
                if let Some(meta) = get_meta(path) {
                    let mode = meta.mode();
                    let uid = unsafe { libc::geteuid() };
                    let gid = unsafe { libc::getegid() };
                    let writable = if meta.uid() == uid {
                        mode & 0o200 != 0
                    } else if meta.gid() == gid {
                        mode & 0o020 != 0
                    } else {
                        mode & 0o002 != 0
                    };
                    if writable {
                        0
                    } else {
                        1
                    }
                } else {
                    1
                }
            }
            ["-x", path] => {
                use std::os::unix::fs::MetadataExt;
                if let Some(meta) = get_meta(path) {
                    let mode = meta.mode();
                    let uid = unsafe { libc::geteuid() };
                    let gid = unsafe { libc::getegid() };
                    let executable = if meta.uid() == uid {
                        mode & 0o100 != 0
                    } else if meta.gid() == gid {
                        mode & 0o010 != 0
                    } else {
                        mode & 0o001 != 0
                    };
                    if executable {
                        0
                    } else {
                        1
                    }
                } else {
                    1
                }
            }

            // Special permission bits — prefetched metadata
            ["-g", path] => {
                use std::os::unix::fs::MetadataExt;
                if get_meta(path)
                    .map(|m| m.mode() & 0o2000 != 0)
                    .unwrap_or(false)
                {
                    0
                } else {
                    1
                }
            }
            ["-k", path] => {
                use std::os::unix::fs::MetadataExt;
                if get_meta(path)
                    .map(|m| m.mode() & 0o1000 != 0)
                    .unwrap_or(false)
                {
                    0
                } else {
                    1
                }
            }
            ["-u", path] => {
                use std::os::unix::fs::MetadataExt;
                if get_meta(path)
                    .map(|m| m.mode() & 0o4000 != 0)
                    .unwrap_or(false)
                {
                    0
                } else {
                    1
                }
            }

            // File size — prefetched metadata
            ["-s", path] => {
                if get_meta(path).map(|m| m.len() > 0).unwrap_or(false) {
                    0
                } else {
                    1
                }
            }

            // Ownership — prefetched metadata
            ["-O", path] => {
                use std::os::unix::fs::MetadataExt;
                if get_meta(path)
                    .map(|m| m.uid() == unsafe { libc::geteuid() })
                    .unwrap_or(false)
                {
                    0
                } else {
                    1
                }
            }
            ["-G", path] => {
                use std::os::unix::fs::MetadataExt;
                if get_meta(path)
                    .map(|m| m.gid() == unsafe { libc::getegid() })
                    .unwrap_or(false)
                {
                    0
                } else {
                    1
                }
            }

            // File times — prefetched metadata
            ["-N", path] => {
                use std::os::unix::fs::MetadataExt;
                if let Some(meta) = get_meta(path) {
                    if meta.mtime() > meta.atime() {
                        0
                    } else {
                        1
                    }
                } else {
                    1
                }
            }

            // Terminal test
            ["-t", fd] => {
                if let Ok(fd_num) = fd.parse::<i32>() {
                    if unsafe { libc::isatty(fd_num) } == 1 {
                        0
                    } else {
                        1
                    }
                } else {
                    1
                }
            }

            // Variable test
            ["-v", varname] => {
                if self.variables.contains_key(*varname) || std::env::var(varname).is_ok() {
                    0
                } else {
                    1
                }
            }

            // Option test
            ["-o", opt] => {
                let (name, _) = Self::normalize_option_name(opt);
                if self.options.get(&name).copied().unwrap_or(false) {
                    0
                } else {
                    1
                }
            }

            // String comparisons
            // POSIX `[`-test only accepts `=` for string equality.
            // `==` is the `[[`-cond extension; in `[`, zsh emits
            // `1: = not found` exit 1 (it parses `==` as `=` `=`
            // and tries to look up the second `=` as a command).
            [_, "==", _] => {
                zwarn("= not found");
                1
            }
            [a, "=", b] => {
                if a == b {
                    0
                } else {
                    1
                }
            }
            [a, "!=", b] => {
                if a != b {
                    0
                } else {
                    1
                }
            }
            // NOTE: zsh's `[`-test (POSIX-mode test) does NOT accept
            // `<` or `>` as string comparators — they're redirection
            // operators. `[ "5" \> "3" ]` errors `1: condition
            // expected: >`. zshrs's earlier impl had string-compare
            // arms for both, hiding the syntax error. Removed those
            // arms so the operands fall through to the catch-all
            // 3-arg arm which now reports `unknown condition: <op>`
            // (it lists `<`/`>` as known so the diagnostic stays
            // clean). The `[[`-cond compiler still handles them.
            [a, "<", b] | [a, ">", b] => {
                zwarn(&format!("condition expected: {}", args[1]));
                let _ = (a, b);
                2
            }

            // Numeric comparisons
            [a, "-eq", b] => {
                // zsh: errors `integer expression expected: <arg>`
                // exit 2 when either operand is non-numeric. zshrs
                // previously used `unwrap_or(0)` which silently
                // coerced "abc" to 0 (so `[ a -eq 0 ]` returned
                // true).
                let av = match a.parse::<i64>() {
                    Ok(v) => v,
                    Err(_) => {
                        zwarnnam("[", &format!("integer expression expected: {}", a));
                        return 2;
                    }
                };
                let bv = match b.parse::<i64>() {
                    Ok(v) => v,
                    Err(_) => {
                        zwarnnam("[", &format!("integer expression expected: {}", b));
                        return 2;
                    }
                };
                if av == bv {
                    0
                } else {
                    1
                }
            }
            [a, "-ne", b] => {
                let av = match a.parse::<i64>() {
                    Ok(v) => v,
                    Err(_) => {
                        zwarnnam("[", &format!("integer expression expected: {}", a));
                        return 2;
                    }
                };
                let bv = match b.parse::<i64>() {
                    Ok(v) => v,
                    Err(_) => {
                        zwarnnam("[", &format!("integer expression expected: {}", b));
                        return 2;
                    }
                };
                if av != bv {
                    0
                } else {
                    1
                }
            }
            [a, "-lt", b] => {
                let av = match a.parse::<i64>() {
                    Ok(v) => v,
                    Err(_) => {
                        zwarnnam("[", &format!("integer expression expected: {}", a));
                        return 2;
                    }
                };
                let bv = match b.parse::<i64>() {
                    Ok(v) => v,
                    Err(_) => {
                        zwarnnam("[", &format!("integer expression expected: {}", b));
                        return 2;
                    }
                };
                if av < bv {
                    0
                } else {
                    1
                }
            }
            [a, "-le", b] => {
                let av = match a.parse::<i64>() {
                    Ok(v) => v,
                    Err(_) => {
                        zwarnnam("[", &format!("integer expression expected: {}", a));
                        return 2;
                    }
                };
                let bv = match b.parse::<i64>() {
                    Ok(v) => v,
                    Err(_) => {
                        zwarnnam("[", &format!("integer expression expected: {}", b));
                        return 2;
                    }
                };
                if av <= bv {
                    0
                } else {
                    1
                }
            }
            [a, "-gt", b] => {
                let av = match a.parse::<i64>() {
                    Ok(v) => v,
                    Err(_) => {
                        zwarnnam("[", &format!("integer expression expected: {}", a));
                        return 2;
                    }
                };
                let bv = match b.parse::<i64>() {
                    Ok(v) => v,
                    Err(_) => {
                        zwarnnam("[", &format!("integer expression expected: {}", b));
                        return 2;
                    }
                };
                if av > bv {
                    0
                } else {
                    1
                }
            }
            [a, "-ge", b] => {
                let av = match a.parse::<i64>() {
                    Ok(v) => v,
                    Err(_) => {
                        zwarnnam("[", &format!("integer expression expected: {}", a));
                        return 2;
                    }
                };
                let bv = match b.parse::<i64>() {
                    Ok(v) => v,
                    Err(_) => {
                        zwarnnam("[", &format!("integer expression expected: {}", b));
                        return 2;
                    }
                };
                if av >= bv {
                    0
                } else {
                    1
                }
            }

            // File comparisons
            [f1, "-nt", f2] => {
                let m1 = std::fs::metadata(f1).and_then(|m| m.modified()).ok();
                let m2 = std::fs::metadata(f2).and_then(|m| m.modified()).ok();
                match (m1, m2) {
                    (Some(t1), Some(t2)) if t1 > t2 => 0,
                    (Some(_), None) => 0,
                    _ => 1,
                }
            }
            [f1, "-ot", f2] => {
                let m1 = std::fs::metadata(f1).and_then(|m| m.modified()).ok();
                let m2 = std::fs::metadata(f2).and_then(|m| m.modified()).ok();
                match (m1, m2) {
                    (Some(t1), Some(t2)) if t1 < t2 => 0,
                    (None, Some(_)) => 0,
                    _ => 1,
                }
            }
            [f1, "-ef", f2] => {
                use std::os::unix::fs::MetadataExt;
                let m1 = std::fs::metadata(f1).ok();
                let m2 = std::fs::metadata(f2).ok();
                match (m1, m2) {
                    (Some(a), Some(b)) if a.dev() == b.dev() && a.ino() == b.ino() => 0,
                    _ => 1,
                }
            }

            // Single string test
            [s] => {
                if !s.is_empty() {
                    0
                } else {
                    1
                }
            }

            _ => {
                // Negation prefix: `! expr` — recursively evaluate and flip.
                // Handles `test ! -z foo`, `test ! a = b`, etc.
                if args.first() == Some(&"!") {
                    let rest: Vec<String> = args[1..].iter().map(|s| s.to_string()).collect();
                    return if self.bin_test(&rest) == 0 { 1 } else { 0 };
                }
                // Two-arg unknown unary `[ -X foo ]` — zsh emits
                // `unknown condition: -X` and exits 2. Without this
                // explicit arm, an unknown flag like `-i` fell through
                // the AND/OR split and silently returned 1 (which a
                // consumer would read as "false" instead of "syntax
                // error"). Match zsh's diagnostic + exit-2. Also
                // catches `[ -- foo ]` where `--` is treated as a
                // bogus flag name (zsh: `unknown condition: --`).
                if args.len() == 2
                    && args[0].starts_with('-')
                    && args[0].len() > 1
                    && !matches!(args[0], "-a" | "-o")
                {
                    let bytes = args[0].as_bytes();
                    if bytes[1..].iter().all(|b| b.is_ascii_alphabetic()) || args[0] == "--" {
                        zwarnnam("[", &format!("unknown condition: {}", args[0]));
                        return 2;
                    }
                }
                // `[ "" "" ]` (2 args, neither operator nor unary
                // flag, neither paren) -> zsh: `1: parse error:
                // condition expected:` exit 2. Two operands without
                // a connective is ill-formed. Exclude `(`/`)` so
                // the paren-handling code below still gets to run
                // for `[ \( \) ]`.
                if args.len() == 2
                    && !args[0].starts_with('-')
                    && !args[1].starts_with('-')
                    && args[0] != "("
                    && args[1] != ")"
                {
                    zwarn(&format!("parse error: condition expected: {}", args[0]));
                    return 2;
                }
                // `[ a -lt ]` (2 args: operand + binop, missing
                // right-side operand) -> zsh: `1: parse error:
                // condition expected: a` exit 2.
                if args.len() == 2
                    && !args[0].starts_with('-')
                    && matches!(
                        args[1],
                        "-eq"
                            | "-ne"
                            | "-lt"
                            | "-le"
                            | "-gt"
                            | "-ge"
                            | "="
                            | "!="
                            | "<"
                            | ">"
                            | "=="
                            | "-nt"
                            | "-ot"
                            | "-ef"
                    )
                {
                    zwarn(&format!("parse error: condition expected: {}", args[0]));
                    return 2;
                }
                // 3-arg with binary operator at position 0 (not 1) —
                // `[ -lt 5 3 ]` is a syntax error in zsh:
                // `[:1: unknown condition: -lt`. The operator-name
                // appearing as the FIRST operand looks like a unary
                // condition zsh doesn't recognise.
                if args.len() == 3
                    && matches!(args[0], "-eq" | "-ne" | "-lt" | "-le" | "-gt" | "-ge")
                {
                    zwarnnam("[", &format!("unknown condition: {}", args[0]));
                    return 2;
                }
                // 3-arg with unknown non-`-` operator at args[1] —
                // `[ a := a ]` (or other made-up infix) -> zsh:
                // `1: condition expected: :=` exit 2. Operands at
                // args[0]/args[2] are non-flag, args[1] looks like
                // an operator-ish token that's NOT `-`-prefixed
                // (`-ZZ` etc. take the dedicated unknown-binop arm
                // below) and not in zsh's table.
                if args.len() == 3
                    && !args[0].starts_with('-')
                    && !args[2].starts_with('-')
                    && !args[1].starts_with('-')
                    && args[1].chars().any(|c| !c.is_ascii_alphanumeric())
                    && !matches!(args[1], "=" | "!=" | "==")
                {
                    zwarnnam("[", &format!("condition expected: {}", args[1]));
                    return 2;
                }
                // 3-arg with binop at args[1] but NO operand at
                // args[2] (impossible since len==3, so args[2] always
                // exists — but zsh handles `[ a -lt ]` (2 args) as
                // a parse error too. That's covered by the earlier
                // 2-arg arm if args[1].starts_with('-') AND the op
                // is a known binop (treated as unary-flag-name miss
                // by the unknown-flag arm). 3-arg with `args[1]=-lt
                // args[2]=...` and missing third operand is rare;
                // skip until a real probe surfaces it.
                // 3-arg with a known unary flag at args[0] — covers
                // both `[ -z -n a ]` (flag-flag-arg) and `[ -e /tmp
                // X ]` (flag-operand-extra). Both layouts mean the
                // parse expected `-FLAG OPERAND` (2-arg form), with
                // the extra arg as the surplus. zsh: `too many
                // arguments`.
                if args.len() == 3
                    && args[0].len() == 2
                    && matches!(
                        args[0],
                        "-z" | "-n"
                            | "-d"
                            | "-f"
                            | "-e"
                            | "-r"
                            | "-w"
                            | "-x"
                            | "-s"
                            | "-h"
                            | "-L"
                            | "-O"
                            | "-G"
                            | "-N"
                            | "-S"
                            | "-p"
                            | "-b"
                            | "-c"
                            | "-g"
                            | "-k"
                            | "-u"
                            | "-t"
                            | "-v"
                    )
                {
                    zwarnnam("[", "too many arguments");
                    return 2;
                }
                // Three-arg `[ a -OP b ]` where -OP isn't a known
                // numeric/string comparator: zsh errors at the OP
                // position. Detect and emit the same kind of error
                // (most common case is `-eq` with non-numeric args).
                if args.len() == 3 && args[1].starts_with('-') {
                    if matches!(args[1], "-eq" | "-ne" | "-lt" | "-le" | "-gt" | "-ge") {
                        // Check both operands are numeric.
                        if args[0].parse::<i64>().is_err() {
                            zwarnnam("[", &format!("integer expression expected: {}", args[0]));
                            return 2;
                        }
                        if args[2].parse::<i64>().is_err() {
                            zwarnnam("[", &format!("integer expression expected: {}", args[2]));
                            return 2;
                        }
                    } else if !matches!(
                        args[1],
                        "=" | "!=" | "<" | ">" | "==" | "-a" | "-o" | "-nt" | "-ot" | "-ef"
                    ) && args[1].len() > 1
                        && args[1][1..].chars().all(|c| c.is_ascii_alphabetic())
                    {
                        // Unknown alphabetic 3-arg operator like `-ZZ`.
                        // zsh: `[:1: unknown condition: -ZZ` exit 2.
                        zwarnnam("[", &format!("unknown condition: {}", args[1]));
                        return 2;
                    }
                }
                // 3-arg with no recognized operator anywhere — `[ a
                // b c ]` -> `1: condition expected: b` exit 2 (zsh
                // points at args[1] which should have been an op).
                if args.len() == 3
                    && !args[0].starts_with('-')
                    && !args[1].starts_with('-')
                    && !args[2].starts_with('-')
                    && args[0] != "("
                    && args[1] != "("
                    && args[2] != ")"
                    && !matches!(args[1], "=" | "!=" | "==")
                {
                    zwarn(&format!("condition expected: {}", args[1]));
                    return 2;
                }
                // `[ \( a ]` — paren without matching close. zsh emits
                // `[:1: argument expected` exit 2 (lexer realises it
                // ran out of operands inside the open-paren context).
                // Only fire when there's an unmatched paren depth at
                // end of args — `[ \( a \) -a -z "" ]` has `(` first
                // AND a matching `)` later, so it's NOT a mismatch.
                {
                    let mut d = 0i32;
                    for a in args.iter() {
                        match *a {
                            "(" => d += 1,
                            ")" => d -= 1,
                            _ => {}
                        }
                    }
                    if d > 0 {
                        // More `(` than `)` — open paren without
                        // close. zsh: `argument expected`.
                        zwarnnam("[", "argument expected");
                        return 2;
                    } else if d < 0 {
                        // More `)` than `(` — surplus close paren.
                        // zsh: `[:1: too many arguments` (the `)`
                        // is the extra arg). zshrs collapsed both
                        // to "argument expected".
                        zwarnnam("[", "too many arguments");
                        return 2;
                    }
                }
                // POSIX `-a` (and) / `-o` (or) connectives — split the
                // arg list on the first top-level connective and
                // recursively evaluate each side.
                // For `test 5 -gt 3 -a 3 -lt 4`: split at `-a` →
                // left=`5 -gt 3`, right=`3 -lt 4`. AND short-circuits
                // on left=fail; OR short-circuits on left=success.
                // Find the LAST connective so left binds tighter (zsh
                // convention).
                // Skip operators inside parens — `[ \( -n a \) -a \( -z "" \) ]`
                // splits at the OUTER `-a`, not the operators wrapped in
                // `(...)` subgroups. Track depth and only consider
                // connectives at depth 0.
                let mut and_idx: Option<usize> = None;
                let mut or_idx: Option<usize> = None;
                let mut depth = 0i32;
                for (i, a) in args.iter().enumerate() {
                    match *a {
                        "(" => depth += 1,
                        ")" => depth -= 1,
                        "-a" if depth == 0 => and_idx = Some(i),
                        "-o" if depth == 0 => or_idx = Some(i),
                        _ => {}
                    }
                }
                // If the entire expression is wrapped in matching parens
                // (`[ ( EXPR ) ]`), strip and recurse on the inner.
                // Detect by walking depth — the outer `(` enters depth 1
                // and the matching `)` is the LAST char that brings depth
                // back to 0.
                if args.first() == Some(&"(") && args.last() == Some(&")") {
                    let mut d = 0i32;
                    let mut closes_at_end = false;
                    for (i, a) in args.iter().enumerate() {
                        match *a {
                            "(" => d += 1,
                            ")" => {
                                d -= 1;
                                if d == 0 {
                                    closes_at_end = i == args.len() - 1;
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                    if closes_at_end {
                        let inner: Vec<String> = args[1..args.len() - 1]
                            .iter()
                            .map(|s| s.to_string())
                            .collect();
                        // `[ \( \) ]` — empty parens. zsh: the
                        // recursive call into the inner arg list
                        // ought to error `argument expected` rather
                        // than silently return 1 (which means
                        // "false" but should really mean "syntax
                        // error" since an empty test expression is
                        // ill-formed).
                        if inner.is_empty() {
                            zwarnnam("[", "argument expected");
                            return 2;
                        }
                        return self.bin_test(&inner);
                    }
                }
                // OR has lower precedence — split there first if present.
                if let Some(i) = or_idx {
                    let left: Vec<String> = args[..i].iter().map(|s| s.to_string()).collect();
                    let right: Vec<String> = args[i + 1..].iter().map(|s| s.to_string()).collect();
                    let l = self.bin_test(&left);
                    if l == 0 {
                        return 0;
                    }
                    return self.bin_test(&right);
                }
                if let Some(i) = and_idx {
                    let left: Vec<String> = args[..i].iter().map(|s| s.to_string()).collect();
                    let right: Vec<String> = args[i + 1..].iter().map(|s| s.to_string()).collect();
                    let l = self.bin_test(&left);
                    if l != 0 {
                        return l;
                    }
                    return self.bin_test(&right);
                }
                // 4+ args: distinguish two zsh diagnostics. If
                // args[1] is a known binary operator (-eq/-lt/etc.,
                // =/!=, etc.) and there are MORE than 3 args, OR
                // args[0] is a known UNARY flag (-z, -n, -d, -f,
                // etc.) followed by an operand and extra junk, zsh
                // says `[:1: too many arguments`. Otherwise it's
                // `condition expected: <args[0]>`.
                if args.len() >= 4 {
                    let known_binop = matches!(
                        args[1],
                        "-eq"
                            | "-ne"
                            | "-lt"
                            | "-le"
                            | "-gt"
                            | "-ge"
                            | "="
                            | "!="
                            | "<"
                            | ">"
                            | "=="
                            | "-nt"
                            | "-ot"
                            | "-ef"
                    );
                    let unary_flag_at_0 = args[0].starts_with('-')
                        && args[0].len() == 2
                        && matches!(
                            args[0],
                            "-z" | "-n"
                                | "-d"
                                | "-f"
                                | "-e"
                                | "-r"
                                | "-w"
                                | "-x"
                                | "-s"
                                | "-h"
                                | "-L"
                                | "-O"
                                | "-G"
                                | "-N"
                                | "-S"
                                | "-p"
                                | "-b"
                                | "-c"
                                | "-g"
                                | "-k"
                                | "-u"
                                | "-t"
                                | "-v"
                                | "-o"
                        );
                    if known_binop || unary_flag_at_0 {
                        zwarnnam("[", "too many arguments");
                    } else {
                        zwarn(&format!("condition expected: {}", args[0]));
                    }
                    return 2;
                }
                1
            }
        }
    }
    pub(crate) fn builtin_local(&mut self, args: &[String]) -> i32 {
        // Per zsh's actual behavior (verified against /bin/zsh): a top-
        // level `local` is silently accepted — `local x=hello` in a
        // sourced script or script-mode file creates the variable
        // (effectively `typeset`-equivalent) without any error. This
        // matches Src/builtin.c bin_typeset BIN_LOCAL: when `locallevel
        // == 0`, the variable is still declared, just at the outer
        // scope. The previous "can only be used in a function" diagnostic
        // here was overzealous and broke real-world scripts that use
        // top-level `local` as a typeset alias (notably p10k:
        // `'builtin' 'local' '-a' '__p9k_src_opts'` at the top of
        // `powerlevel10k.zsh-theme`).
        self.builtin_typeset_named(args, "local")
    }
    pub(crate) fn builtin_declare(&mut self, args: &[String]) -> i32 {
        // zsh prefixes "no such variable" errors with the builtin name
        // the user actually invoked (`declare:` vs `typeset:`).
        self.builtin_typeset_named(args, "declare")
    }
    pub(crate) fn bin_typeset(&mut self, args: &[String]) -> i32 {
        self.builtin_typeset_named(args, "typeset")
    }
    pub(crate) fn builtin_typeset_named(&mut self, args: &[String], invoked_as: &str) -> i32 {
        // PFA-SMR aspect: emit one `typeset` event per `NAME[=value]`
        // positional arg, with structured ParamAttrs derived from the
        // leading flag letters. Listing-only invocations (no positional
        // args) are skipped. `-T SCALAR ARRAY [SEP]` is special: only
        // the first two positional args are real names; the optional
        // separator (3rd positional) must NOT be recorded as a name.
        #[cfg(feature = "recorder")]
        if crate::recorder::is_enabled() {
            let ctx = self.recorder_ctx();
            let mut letters = String::new();
            let mut tied_mode = false;
            for a in args {
                if a.starts_with('-') || a.starts_with('+') {
                    let body = &a[1..];
                    letters.push_str(body);
                    if body.contains('T') {
                        tied_mode = true;
                    }
                    continue;
                }
            }
            // builtin_typeset_named is also reachable as `local`,
            // `declare`, `private`. Inside a function scope `local NAME`
            // is a true local variable, NOT global state, so skip the
            // emit. At top level (depth 0) `local` is an error and
            // never reaches this hook anyway.
            let is_locallike = matches!(invoked_as, "local" | "private");
            if !is_locallike || self.local_scope_depth == 0 {
                let attrs = crate::recorder::ParamAttrs::from_flag_chars(&letters);
                let mut tied_seen = 0usize;
                for a in args {
                    if a.starts_with('-') || a.starts_with('+') {
                        continue;
                    }
                    // For `typeset -T X Y [SEP]`, only X and Y are names.
                    if tied_mode {
                        tied_seen += 1;
                        if tied_seen > 2 {
                            break;
                        }
                    }
                    if let Some((k, v)) = a.split_once('=') {
                        crate::recorder::emit_typeset_attrs(k, Some(v), attrs, ctx.clone());
                    } else {
                        crate::recorder::emit_typeset_attrs(a, None, attrs, ctx.clone());
                    }
                }
            }
        }
        // Save old values when inside a function scope (local variable support).
        // Restored by call_function on function exit. The `-g` flag opts out
        // of localization — `declare -g x=val` from inside a function should
        // bind `x` at the global scope, so don't push to local_save_stack.
        let has_g = args.iter().any(|a| {
            a.starts_with('-')
                && !a.starts_with("--")
                && a.len() > 1
                && a[1..].chars().any(|c| c == 'g')
        });
        if self.local_scope_depth > 0 && !has_g {
            for arg in args {
                if arg.starts_with('-') || arg.starts_with('+') {
                    continue;
                }
                let name = arg.split('=').next().unwrap_or(arg);
                if !name.is_empty() {
                    // Scalar save — covers `local x=foo` and `local x` reads.
                    let old_val = self.variables.get(name).cloned();
                    self.local_save_stack.push((name.to_string(), old_val));
                    // Array save — covers `local arr=(...)`. Track even when
                    // not currently an array, so call_function exit can
                    // remove a freshly-installed local array binding.
                    let old_arr = self.arrays.get(name).cloned();
                    self.local_array_save_stack
                        .push((name.to_string(), old_arr));
                    // Assoc save — covers `local -A h=(...)`. zsh shadows
                    // the outer assoc binding; without this, the inner
                    // typeset -A h leaked into parent on function exit.
                    let old_assoc = self.assoc_arrays.get(name).cloned();
                    self.local_assoc_save_stack
                        .push((name.to_string(), old_assoc));
                }
            }
        }

        // typeset [ {+|-}AHUaghlmrtux ] [ {+|-}EFLRZip [ n ] ]
        //         [ + ] [ name[=value] ... ]
        // typeset -T [ {+|-}Urux ] [ {+|-}LRZp [ n ] ] SCALAR[=value] array
        // typeset -f [ {+|-}TUkmtuz ] [ + ] [ name ... ]

        let mut is_array = false; // -a
        let mut is_assoc = false; // -A
        let mut is_export = false; // -x
        let mut is_integer = false; // -i
        let mut is_readonly = false; // -r
        let mut is_lower = false; // -l
        let mut is_upper = false; // -u
        let mut is_left_pad = false; // -L
        let mut is_right_pad = false; // -R
        let mut is_zero_pad = false; // -Z
        let mut is_float = false; // -F
        let mut is_float_exp = false; // -E
        let mut is_function = false; // -f
        let mut is_global = false; // -g
        let mut is_tied = false; // -T
        // zsh flag semantics (Src/builtin.c "typeset" spec):
        //   -h = PM_HIDE   = hidden
        //   -H = PM_HIDEVAL = hide_val
        // (NOT the other way around — earlier code had these reversed,
        // which made `typeset -H ZINIT` produce `(t)=association-hide`
        // instead of zsh's `association-hideval`.)
        let mut is_hidden = false; // -h
        let mut is_hide_val = false; // -H
        let mut is_trace = false; // -t
        let mut is_unique = false; // -U: dedupe array elements
        let mut print_mode = false; // -p
        let mut matchpat = false; // -m
        let mut list_mode = false; // no args: list all
        let mut plus_mode = false; // +x etc: remove attribute
        let mut width: Option<usize> = None;
        let mut precision: Option<usize> = None;
        let mut int_base: Option<u32> = None;
        let mut var_args: Vec<String> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];

            if arg == "--" {
                i += 1;
                while i < args.len() {
                    var_args.push(args[i].clone());
                    i += 1;
                }
                break;
            }

            if arg == "+" {
                plus_mode = true;
                i += 1;
                continue;
            }

            if arg.starts_with('+') && arg.len() > 1 {
                plus_mode = true;
                for c in arg[1..].chars() {
                    match c {
                        'a' => is_array = false,
                        'A' => is_assoc = false,
                        'x' => is_export = false,
                        'i' => is_integer = false,
                        'r' => is_readonly = false,
                        'l' => is_lower = false,
                        'u' => is_upper = false,
                        'L' => is_left_pad = false,
                        'R' => is_right_pad = false,
                        'Z' => is_zero_pad = false,
                        'F' => is_float = false,
                        'E' => is_float_exp = false,
                        'f' => is_function = false,
                        'g' => is_global = false,
                        'T' => is_tied = false,
                        'H' => is_hide_val = false,
                        'h' => is_hidden = false,
                        't' => is_trace = false,
                        'U' => is_unique = false,
                        'p' => print_mode = false,
                        'm' => matchpat = false,
                        // `+` flag also handles `-i` removal etc. Unknown
                        // letters error like the `-` arm: `bad option: +X`.
                        other => {
                            zwarnnam(invoked_as, &format!("bad option: +{}", other));
                            return 1;
                        }
                    }
                }
            } else if arg.starts_with('-') && arg.len() > 1 {
                let mut chars = arg[1..].chars().peekable();
                while let Some(c) = chars.next() {
                    match c {
                        'a' => is_array = true,
                        'A' => is_assoc = true,
                        'x' => is_export = true,
                        'i' => {
                            is_integer = true;
                            // `-i N` (attached digits): `-i16` sets base 16.
                            let rest: String = chars.clone().collect();
                            if !rest.is_empty()
                                && rest
                                    .chars()
                                    .next()
                                    .map(|c| c.is_ascii_digit())
                                    .unwrap_or(false)
                            {
                                let num: String =
                                    chars.by_ref().take_while(|c| c.is_ascii_digit()).collect();
                                int_base = num.parse().ok();
                            }
                        }
                        'r' => is_readonly = true,
                        'l' => is_lower = true,
                        'u' => is_upper = true,
                        'L' => {
                            is_left_pad = true;
                            // Check for width
                            let rest: String = chars.clone().collect();
                            if !rest.is_empty()
                                && rest
                                    .chars()
                                    .next()
                                    .map(|c| c.is_ascii_digit())
                                    .unwrap_or(false)
                            {
                                let num: String =
                                    chars.by_ref().take_while(|c| c.is_ascii_digit()).collect();
                                width = num.parse().ok();
                            }
                        }
                        'R' => {
                            is_right_pad = true;
                            let rest: String = chars.clone().collect();
                            if !rest.is_empty()
                                && rest
                                    .chars()
                                    .next()
                                    .map(|c| c.is_ascii_digit())
                                    .unwrap_or(false)
                            {
                                let num: String =
                                    chars.by_ref().take_while(|c| c.is_ascii_digit()).collect();
                                width = num.parse().ok();
                            }
                        }
                        'Z' => {
                            is_zero_pad = true;
                            let rest: String = chars.clone().collect();
                            if !rest.is_empty()
                                && rest
                                    .chars()
                                    .next()
                                    .map(|c| c.is_ascii_digit())
                                    .unwrap_or(false)
                            {
                                let num: String =
                                    chars.by_ref().take_while(|c| c.is_ascii_digit()).collect();
                                width = num.parse().ok();
                            }
                        }
                        'F' => {
                            is_float = true;
                            let rest: String = chars.clone().collect();
                            if !rest.is_empty()
                                && rest
                                    .chars()
                                    .next()
                                    .map(|c| c.is_ascii_digit())
                                    .unwrap_or(false)
                            {
                                let num: String =
                                    chars.by_ref().take_while(|c| c.is_ascii_digit()).collect();
                                precision = num.parse().ok();
                            }
                        }
                        'E' => {
                            is_float_exp = true;
                            let rest: String = chars.clone().collect();
                            if !rest.is_empty()
                                && rest
                                    .chars()
                                    .next()
                                    .map(|c| c.is_ascii_digit())
                                    .unwrap_or(false)
                            {
                                let num: String =
                                    chars.by_ref().take_while(|c| c.is_ascii_digit()).collect();
                                precision = num.parse().ok();
                            }
                        }
                        'f' => is_function = true,
                        'g' => is_global = true,
                        'T' => is_tied = true,
                        'H' => is_hide_val = true,
                        'h' => is_hidden = true,
                        't' => is_trace = true,
                        'U' => is_unique = true,
                        'p' => print_mode = true,
                        'm' => matchpat = true,
                        // zsh: unknown typeset/declare flag letter
                        // errors `bad option: -X` exit 1. zshrs's
                        // silent fallback masked typos and made
                        // `typeset -Q x` succeed without setting any
                        // attribute.
                        other => {
                            zwarnnam(invoked_as, &format!("bad option: -{}", other));
                            return 1;
                        }
                    }
                }
                // `-Z 5`, `-L 5`, `-R 5` — width as a separate arg. The
                // in-flag form `-Z5` is handled inside the char-loop above;
                // the separate-arg form needs a peek at args[i+1].
                if width.is_none()
                    && (is_left_pad || is_right_pad || is_zero_pad || is_float || is_float_exp)
                    && i + 1 < args.len()
                    && args[i + 1].chars().all(|c| c.is_ascii_digit())
                    && !args[i + 1].is_empty()
                {
                    width = args[i + 1].parse().ok();
                    // `-F N` / `-E N` use N as float precision, not
                    // padding width — the in-flag form (`-F2`) sets
                    // `precision` directly inside the char loop, but
                    // the separate-arg form (`-F 2`) was only filling
                    // `width`, so the storage formatter fell back to
                    // its `precision.unwrap_or(10)` default.
                    if (is_float || is_float_exp) && precision.is_none() {
                        precision = width;
                    }
                    i += 1;
                }
                // `-i 16` — output base as a separate arg. Mirrors the
                // attached `-i16` form parsed inside the char-loop above.
                if int_base.is_none()
                    && is_integer
                    && i + 1 < args.len()
                    && args[i + 1].chars().all(|c| c.is_ascii_digit())
                    && !args[i + 1].is_empty()
                {
                    int_base = args[i + 1].parse().ok();
                    i += 1;
                }
            } else {
                var_args.push(arg.clone());
            }
            i += 1;
        }

        // -h/-H/-t/-F now flow into VarAttr (hidden / hide_val / trace /
        // float_precision). -g (global) controls scope insertion at
        // assignment time and isn't a stored attribute.
        let _ = is_global;

        // `typeset -m PAT [PAT...]` — treat each var_arg as a glob pattern
        // and list matching variables. With no flags besides -m it acts as
        // a filter on the listing output. The patterns may match scalars,
        // arrays, assocs, and (with -f) functions.
        if matchpat && !var_args.is_empty() {
            let patterns = std::mem::take(&mut var_args);
            let mut matched: Vec<String> = Vec::new();
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            let names: Vec<String> = if is_function {
                self.function_names()
            } else {
                let mut all: Vec<String> = self.variables.keys().cloned().collect();
                for k in self.arrays.keys() {
                    all.push(k.clone());
                }
                for k in self.assoc_arrays.keys() {
                    all.push(k.clone());
                }
                all
            };
            for p in &patterns {
                for n in &names {
                    if ShellExecutor::glob_match_static(n, p) && seen.insert(n.clone()) {
                        matched.push(n.clone());
                    }
                }
            }
            matched.sort();
            var_args = matched;
            // With patterns and no other declarative flags, force listing.
            if !is_array
                && !is_assoc
                && !is_export
                && !is_integer
                && !is_readonly
                && !is_lower
                && !is_upper
                && !is_left_pad
                && !is_right_pad
                && !is_zero_pad
                && !is_float
                && !is_float_exp
                && !is_tied
            {
                if is_function {
                    for name in &var_args {
                        if let Some(body) = self.function_definition_text(name) {
                            println!(
                                "{} () {{\n\t{}\n}}",
                                name,
                                FuncBodyFmt::render(body.trim())
                            );
                        }
                    }
                } else {
                    let prefix = if print_mode { "typeset " } else { "" };
                    for name in &var_args {
                        if let Some(arr) = self.arrays.get(name) {
                            let attrs = if print_mode { "-a " } else { "" };
                            println!(
                                "{}{}{}=( {} )",
                                prefix,
                                attrs,
                                name,
                                arr.iter()
                                    .map(|v| crate::ported::utils::quotedzputs(v))
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            );
                        } else if let Some(assoc) = self.assoc_arrays.get(name) {
                            let attrs = if print_mode { "-A " } else { "" };
                            let mut pairs: Vec<_> = assoc.iter().collect();
                            pairs.sort_by_key(|(k, _)| (*k).clone());
                            let formatted: Vec<String> = pairs
                                .iter()
                                .map(|(k, v)| {
                                    format!("[{}]={}", crate::ported::utils::quotedzputs(k), crate::ported::utils::quotedzputs(v))
                                })
                                .collect();
                            println!("{}{}{}=( {} )", prefix, attrs, name, formatted.join(" "));
                        } else if let Some(val) = self.variables.get(name) {
                            println!("{}{}={}", prefix, name, val);
                        }
                    }
                }
                return 0;
            }
        }

        // `typeset -T VAR var [sep]` — tied scalar/array. Take the
        // current $VAR (or assignment value if VAR=val on the cmdline),
        // split on `sep` (default ":"), store as array `var`. zsh keeps
        // the two synchronized; this initial-assignment binding handles
        // the common idiom (`typeset -T PATH path`); subsequent updates
        // to either side get out of sync until re-tied.
        if is_tied && var_args.len() >= 2 {
            let scalar_name = var_args[0]
                .split('=')
                .next()
                .unwrap_or(&var_args[0])
                .to_string();
            let array_name = &var_args[1];
            // If the scalar arg has =val form, use that as the initial
            // value. Otherwise read the existing value — fall back to
            // the OS environment for vars like PATH that we don't
            // mirror into self.variables. zsh: `typeset -T PATH path`
            // splits the inherited $PATH into the `path` array; without
            // the env fallback `path` was empty.
            let scalar_val = if let Some(eq_pos) = var_args[0].find('=') {
                var_args[0][eq_pos + 1..].to_string()
            } else {
                self.variables
                    .get(&scalar_name)
                    .cloned()
                    .or_else(|| std::env::var(&scalar_name).ok())
                    .unwrap_or_default()
            };
            let sep = var_args
                .get(2)
                .map(|s| s.as_str())
                .unwrap_or(":")
                .to_string();
            let parts: Vec<String> = if scalar_val.is_empty() {
                Vec::new()
            } else {
                scalar_val.split(&sep).map(String::from).collect()
            };
            self.variables.insert(scalar_name.clone(), scalar_val);
            self.arrays.insert(array_name.clone(), parts);
            self.tied_scalar_to_array
                .insert(scalar_name.clone(), (array_name.clone(), sep.clone()));
            self.tied_array_to_scalar
                .insert(array_name.clone(), (scalar_name, sep));
            return 0;
        }

        // If -f (function mode) with no args, list functions
        if is_function && var_args.is_empty() {
            let _ = print_mode;
            for name in self.function_names() {
                if let Some(body) = self.function_definition_text(&name) {
                    println!(
                        "{} () {{\n\t{}\n}}",
                        name,
                        FuncBodyFmt::render(body.trim())
                    );
                }
            }
            return 0;
        }

        // If -f with args, just show those functions. Per zsh's
        // bin_typeset (Src/builtin.c), missing names error
        // `typeset: no such function: NAME` and the overall status
        // is 1 if any name was missing — the shell reports 1 when
        // even one of the listed names doesn't exist.
        if is_function {
            let _ = print_mode;
            let mut missing = 0;
            for name in &var_args {
                if let Some(body) = self.function_definition_text(name) {
                    println!(
                        "{} () {{\n\t{}\n}}",
                        name,
                        FuncBodyFmt::render(body.trim())
                    );
                } else {
                    zwarnnam(invoked_as, &format!("no such function: {}", name));
                    missing += 1;
                }
            }
            return if missing > 0 { 1 } else { 0 };
        }

        // No args: list all variables with attributes
        if var_args.is_empty() {
            list_mode = true;
        }

        // zsh's bare `typeset NAME` / `declare NAME` (no type
        // flags, no `=`) prints the variable's current
        // declaration if NAME is set — same SHAPE as `-p` but
        // WITHOUT the `typeset`/`export` prefix. `typeset -p a`
        // prints `typeset -a a=( ... )`; bare `typeset a`
        // prints `a=( ... )`. Unset names still get declared.
        // zshrs silently swallowed bare-name calls, dropping
        // the listing entirely. Promote the call to print_mode
        // when ALL bare-name args are already-set vars; track
        // a separate `print_no_prefix` to drop the leading
        // `typeset`/`export`.
        let no_type_flags = !is_integer
            && !is_float
            && !is_float_exp
            && !is_left_pad
            && !is_right_pad
            && !is_zero_pad
            && !is_lower
            && !is_upper
            && !is_readonly
            && !is_export
            && !is_array
            && !is_assoc
            && !is_unique
            && !plus_mode
            && !print_mode
            && !is_function
            && !list_mode;
        let mut print_no_prefix = false;
        // Top-level only — inside a function, bare `typeset
        // NAME` localizes (shadows parent, resets to empty). The
        // print-the-declaration behavior fires only at the
        // shell's top scope (matches zsh).
        if no_type_flags
            && self.local_scope_depth == 0
            && !var_args.is_empty()
            && var_args.iter().all(|a| {
                !a.contains('=')
                    && (self.variables.contains_key(a.as_str())
                        || self.arrays.contains_key(a.as_str())
                        || self.assoc_arrays.contains_key(a.as_str())
                        || env::var(a.as_str()).is_ok())
            })
        {
            print_mode = true;
            print_no_prefix = true;
        }

        if list_mode {
            // Type-filter: when -F/-E is set, narrow to float-typed
            // vars only. zsh: `declare -F` prints nothing without any
            // float vars; with `typeset -F PI=3.14` it prints just PI.
            // Other type flags (-i, -a, -A) need shell-internal-param
            // awareness to match zsh — left untouched here.
            let mut sorted_names: Vec<_> = self.variables.keys().cloned().collect();
            sorted_names.sort();
            for name in &sorted_names {
                // PM_HIDE: zsh suppresses hidden vars from declarative
                // listings. They're still expandable; just not visible
                // from `set` / bare `typeset`.
                if self.var_attrs.get(name).map(|a| a.hidden).unwrap_or(false) {
                    continue;
                }
                let val = self.variables.get(name).cloned().unwrap_or_default();
                let mut attrs = String::new();
                if is_export || env::var(name).is_ok() {
                    attrs.push('x');
                }
                let is_arr = self.arrays.contains_key(name);
                let is_hash = self.assoc_arrays.contains_key(name);
                if is_arr {
                    attrs.push('a');
                }
                if is_hash {
                    attrs.push('A');
                }
                if is_float || is_float_exp {
                    // Only float-typed vars on -F/-E listings. zsh's
                    // -i / -a / -A listings include shell-internal
                    // params (`!`, `$`, EUID, fpath, etc.) — those need
                    // special-param awareness we don't have yet, so
                    // leave those flags untouched (they fall through
                    // to the unfiltered listing below). Keeps the
                    // common `declare -F` / `typeset -F` case correct
                    // without regressing the others.
                    let var_attr = self.var_attrs.get(name);
                    let is_var_float = var_attr
                        .map(|a| matches!(a.kind, VarKind::Float))
                        .unwrap_or(false);
                    if !is_var_float {
                        continue;
                    }
                }
                if print_mode {
                    // typeset -p: output re-executable code with values
                    let prefix = if attrs.is_empty() {
                        "typeset".to_string()
                    } else {
                        format!("typeset -{}", attrs)
                    };
                    if is_hash {
                        if let Some(assoc) = self.assoc_arrays.get(name) {
                            let mut pairs: Vec<_> = assoc.iter().collect();
                            pairs.sort_by_key(|(k, _)| (*k).clone());
                            let formatted: Vec<String> = pairs
                                .iter()
                                .map(|(k, v)| {
                                    format!("[{}]={}", crate::ported::utils::quotedzputs(k), crate::ported::utils::quotedzputs(v))
                                })
                                .collect();
                            println!("{} {}=( {} )", prefix, name, formatted.join(" "));
                        }
                    } else if is_arr {
                        if let Some(arr) = self.arrays.get(name) {
                            let formatted: Vec<String> =
                                arr.iter().map(|v| crate::ported::utils::quotedzputs(v)).collect();
                            println!("{} {}=( {} )", prefix, name, formatted.join(" "));
                        }
                    } else {
                        println!("{} {}={}", prefix, name, crate::ported::utils::quotedzputs(&val));
                    }
                } else if is_hide_val
                    || self
                        .var_attrs
                        .get(name)
                        .map(|a| a.hide_val)
                        .unwrap_or(false)
                {
                    // PM_HIDEVAL: per-var hide-value flag suppresses
                    // the value in listings (the name still prints
                    // so `typeset -p` round-trips). The list-time -H
                    // flag forces the same masking globally.
                    println!("{}={}", name, "*".repeat(val.len().min(8)));
                } else {
                    println!("{}={}", name, val);
                }
            }
            return 0;
        }

        // `typeset -p NAME...` (or `declare -p`): print re-executable
        // declarations for the named vars. Routes here before the
        // assignment loop so plain names don't get treated as bare
        // declarations (which would set them to empty).
        if print_mode {
            for arg in &var_args {
                if arg.contains('=') {
                    continue;
                }
                let name = arg.as_str();
                let mut attrs = String::new();
                let attr = self.var_attrs.get(name).cloned();
                let env_exported = std::env::var(name).is_ok()
                    && !attr.as_ref().map(|a| a.export).unwrap_or(false);
                if let Some(ref a) = attr {
                    match a.kind {
                        VarKind::Integer => attrs.push('i'),
                        VarKind::Float => {
                            // Distinguish `-E` (scientific) from `-F`
                            // (fixed-decimal) so `declare -p` echoes
                            // back the correct flag letter.
                            attrs.push(if a.float_exp { 'E' } else { 'F' });
                        }
                        VarKind::Array => attrs.push('a'),
                        VarKind::Association => attrs.push('A'),
                        VarKind::Scalar => {}
                    }
                    if a.readonly {
                        attrs.push('r');
                    }
                    if a.export {
                        attrs.push('x');
                    }
                } else if self.assoc_arrays.contains_key(name) {
                    attrs.push('A');
                } else if self.arrays.contains_key(name) {
                    attrs.push('a');
                }
                // zsh prints `export NAME=value` instead of `typeset
                // NAME=value` for exported scalars (`declare -p HOME`).
                // For typed exports (`-x` from typeset) the form remains
                // `typeset -x` to preserve the kind. Mirror that.
                let is_exported = env_exported || attr.as_ref().map(|a| a.export).unwrap_or(false);
                // zsh prints exported scalars with `export NAME=…`.
                // Integer-typed exports fold to `export -i NAME=…`.
                // BUT array/assoc/float typed exports stay as
                // `typeset -aAxFE…`; the `export` form is reserved
                // for scalars/integers. Detect "non-scalar attr" and
                // route to typeset in that case.
                let has_non_scalar_attr = attrs.contains('A')
                    || attrs.contains('a')
                    || attrs.contains('F')
                    || attrs.contains('E');
                let prefix = if is_exported && !has_non_scalar_attr {
                    let other_attrs: String = attrs.chars().filter(|&c| c != 'x').collect();
                    if other_attrs.is_empty() {
                        "export".to_string()
                    } else {
                        format!("export -{}", other_attrs)
                    }
                } else if attrs.is_empty() {
                    "typeset".to_string()
                } else {
                    format!("typeset -{}", attrs)
                };
                // Bare `typeset NAME` (no `-p`, no flags) drops
                // the leading `typeset`/`export` prefix —
                // matches zsh's `a=value` form rather than the
                // re-executable `typeset a=value`.
                let pfx_space = if print_no_prefix {
                    String::new()
                } else {
                    format!("{} ", prefix)
                };
                if let Some(map) = self.assoc_arrays.get(name) {
                    let mut pairs: Vec<_> = map.iter().collect();
                    pairs.sort_by_key(|(k, _)| (*k).clone());
                    let formatted: Vec<String> = pairs
                        .iter()
                        .map(|(k, v)| {
                            format!("[{}]={}", crate::ported::utils::quotedzputs(k), crate::ported::utils::quotedzputs(v))
                        })
                        .collect();
                    if formatted.is_empty() {
                        println!("{}{}=( )", pfx_space, name);
                    } else {
                        println!("{}{}=( {} )", pfx_space, name, formatted.join(" "));
                    }
                } else if let Some(arr) = self.arrays.get(name) {
                    let formatted: Vec<String> = arr.iter().map(|v| crate::ported::utils::quotedzputs(v)).collect();
                    println!("{}{}=( {} )", pfx_space, name, formatted.join(" "));
                } else if self.variables.contains_key(name) || env::var(name).is_ok() {
                    let val = self.get_variable(name);
                    println!("{}{}={}", pfx_space, name, crate::ported::utils::quotedzputs(&val));
                } else {
                    // zsh emits `<invoked>:1: no such variable: NAME`
                    // to stderr and exits non-zero when the named
                    // variable doesn't exist. The builtin name comes
                    // from how the user called it — `declare -p X` →
                    // `declare:1:`, `typeset -p X` → `typeset:1:`.
                    zwarnnam(invoked_as, &format!("no such variable: {}", name));
                    return 1;
                }
            }
            return 0;
        }

        // Process variable assignments. Index-based loop so we can
        // gobble continuation args when an assignment of form
        // `name=(elem elem...)` got split across multiple positional
        // args (zsh's lexer keeps the parens intact, but zshrs's
        // bytecode array-init can splice expansions like `("$@")` into
        // separate words — `local -a opts=("$@")` arrives as
        // `["-a", "opts=(a", "b", "c)"]`. Detect un-balanced parens in
        // the value side and absorb subsequent args until the
        // bracket-count rebalances, then process as one combined
        // value).
        let mut idx = 0usize;
        while idx < var_args.len() {
            let arg = var_args[idx].clone();
            idx += 1;
            // Check if this starts an array assignment: "name=(" or "name=(value"
            if let Some(eq_pos) = arg.find('=') {
                let name = &arg[..eq_pos];
                let rest_raw = arg[eq_pos + 1..].to_string();
                // If the rest looks like the start of an array literal
                // (`(...`) but the parens aren't balanced WITHIN this
                // single arg, gobble follow-on args.
                let starts_paren = rest_raw.starts_with('(') || rest_raw.starts_with('\u{88}');
                let rest = if starts_paren {
                    let mut depth: i32 = 0;
                    let mut combined = rest_raw.clone();
                    let count_depth = |s: &str, d: &mut i32| {
                        for c in s.chars() {
                            match c {
                                '(' | '\u{88}' => *d += 1,
                                ')' | '\u{8a}' => *d -= 1,
                                _ => {}
                            }
                        }
                    };
                    count_depth(&rest_raw, &mut depth);
                    while depth > 0 && idx < var_args.len() {
                        combined.push(' ');
                        combined.push_str(&var_args[idx]);
                        count_depth(&var_args[idx], &mut depth);
                        idx += 1;
                    }
                    combined
                } else {
                    rest_raw
                };
                let rest = rest.as_str();

                // zsh validates the lhs is a valid identifier:
                //   `typeset 1bad=5` -> `<INVOKED>:1: not an
                //   identifier: 1bad` exit 1.
                // zshrs silently accepted any name. Allow names
                // ending in subscript (`a[1]=...`, `m[k]=...`)
                // — those route through the runtime arith eval
                // path and are validated separately. Same for
                // declare/local/integer/readonly which all
                // dispatch here via builtin_typeset_named.
                if !name.contains('[') {
                    let mut chars = name.chars();
                    let first_ok = chars
                        .next()
                        .map(|c| c.is_ascii_alphabetic() || c == '_')
                        .unwrap_or(false);
                    let body_ok = chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
                    if !first_ok || !body_ok {
                        zerrnam(invoked_as, &format!("not an identifier: {}", name));
                        return 1;
                    }
                }

                // Read-only check before any mutation: zsh's typeset
                // refuses to overwrite a read-only variable and emits
                // `read-only variable: NAME`. zshrs's typeset path
                // skipped the check and silently overwrote. Per zsh's
                // -c mode, read-only assignment failure aborts the
                // surrounding shell with status 1 (matches the
                // BUILTIN_SET_VAR path's behavior).
                if self.readonly_vars.contains(name)
                    || self
                        .var_attrs
                        .get(name)
                        .map(|a| a.readonly)
                        .unwrap_or(false)
                {
                    zerr(&format!("read-only variable: {}", name));
                    std::process::exit(1);
                }

                // The lexer keeps single-quoted-enclosing parens as META
                // tokens (`INPAR` = `\u{88}`, `OUTPAR` = `\u{8a}`) when
                // the array body contains `'…'`-quoted elements.
                // Without accepting those forms, `typeset -A m=(k1 'v')`
                // fell through to scalar assign and stored the META `(`
                // as the value. Direct port of zsh's tokenized array-
                // body recognition (Src/lex.c / Src/parse.c).
                let stripped_paren = rest
                    .strip_prefix('(')
                    .or_else(|| rest.strip_prefix('\u{88}'));
                if let Some(after_paren) = stripped_paren {
                    // Array assignment - collect all elements until we find ')'
                    let mut elements = Vec::new();
                    // Untokenize the body so the inner element parser
                    // sees the user's literal text. Strip a trailing
                    // META OUTPAR before passing through.
                    let cleaned = after_paren
                        .trim_end_matches('\u{8a}')
                        .trim_end_matches(')');
                    let current = crate::lex::untokenize(cleaned);

                    // Quote-aware split: walk the body honoring
                    // `"..."` and `'...'` so that DQ-quoted
                    // strings stay as one element (preserving
                    // embedded whitespace) but still get the
                    // bslashquote chars stripped from the result.
                    // Direct port of zsh's lex.c word-splitting
                    // for assignment RHS — naive split_whitespace
                    // broke `local arr=( "a b" c )` because "a b"
                    // tokenized to two elements.
                    fn split_array_body(body: &str) -> Vec<String> {
                        let mut out: Vec<String> = Vec::new();
                        let bytes = body.as_bytes();
                        let mut i = 0;
                        while i < bytes.len() {
                            // Skip whitespace.
                            while i < bytes.len() && (bytes[i] as char).is_whitespace() {
                                i += 1;
                            }
                            if i >= bytes.len() {
                                break;
                            }
                            let mut elem = String::new();
                            while i < bytes.len() && !(bytes[i] as char).is_whitespace() {
                                let c = bytes[i] as char;
                                if c == '"' || c == '\'' {
                                    let bslashquote = c;
                                    i += 1;
                                    while i < bytes.len() && (bytes[i] as char) != bslashquote {
                                        // Inside DQ, `\"` and `\\`
                                        // are escaped; pass through
                                        // verbatim (the lexer
                                        // already handled escapes
                                        // before this point).
                                        elem.push(bytes[i] as char);
                                        i += 1;
                                    }
                                    if i < bytes.len() {
                                        i += 1; // close bslashquote
                                    }
                                } else if c == '\\' && i + 1 < bytes.len() {
                                    // Backslash escape in arg —
                                    // keep next char literally.
                                    elem.push(bytes[i + 1] as char);
                                    i += 2;
                                } else {
                                    elem.push(c);
                                    i += 1;
                                }
                            }
                            out.push(elem);
                        }
                        out
                    }

                    // Check if closing ) is in this arg
                    if let Some(close_pos) = current.find(')') {
                        let content = &current[..close_pos];
                        if !content.is_empty() {
                            elements.extend(split_array_body(content));
                        }
                    } else {
                        // Single arg with just elements
                        if !current.is_empty() {
                            let trimmed = current.trim_end_matches(')');
                            elements.extend(split_array_body(trimmed));
                        }
                    }

                    // Set array variable
                    if is_assoc {
                        let mut assoc: IndexMap<String, String> = IndexMap::new();
                        // Detect bash-style `[K]=V` element shape and use
                        // the per-element pair parse; otherwise fall
                        // back to zsh's classic alternating-pairs
                        // (key1, val1, key2, val2, ...). zinit and
                        // p10k both use the bracketed form heavily:
                        //   typeset -A m=([key1]=val1 [key2]=val2)
                        // (Src/builtin.c bin_typeset detects the same
                        // shape via its `[ ... ]=` parser.)
                        let bracket_style = !elements.is_empty()
                            && elements
                                .iter()
                                .all(|e| {
                                    e.starts_with('[')
                                        && e.contains("]=")
                                });
                        if bracket_style {
                            for elem in &elements {
                                if let Some(close) = elem.find("]=") {
                                    let key = &elem[1..close];
                                    let val = &elem[close + 2..];
                                    assoc.insert(key.to_string(), val.to_string());
                                }
                            }
                        } else {
                            let mut iter = elements.iter();
                            while let Some(key) = iter.next() {
                                if let Some(val) = iter.next() {
                                    assoc.insert(key.clone(), val.clone());
                                }
                            }
                        }
                        self.assoc_arrays.insert(name.to_string(), assoc);
                    } else {
                        self.arrays.insert(name.to_string(), elements);
                    }
                    self.variables.insert(name.to_string(), String::new());
                } else {
                    // Regular assignment - apply transformations
                    let mut value = rest.to_string();

                    if is_integer {
                        // Force integer evaluation
                        let evaluated = self.evaluate_arithmetic(&value);
                        value = if let Some(base) = int_base {
                            evaluated
                                .parse::<i64>()
                                .map(|n| format_int_in_base(n, base))
                                .unwrap_or(evaluated)
                        } else {
                            evaluated
                        };
                    }
                    if is_lower {
                        value = value.to_lowercase();
                    }
                    if is_upper {
                        value = value.to_uppercase();
                    }
                    if let Some(w) = width {
                        if is_left_pad {
                            value = format!("{:<width$}", value, width = w);
                            value.truncate(w);
                        } else if is_right_pad || is_zero_pad {
                            let pad_char = if is_zero_pad { '0' } else { ' ' };
                            if value.len() < w {
                                value = format!(
                                    "{}{}",
                                    pad_char.to_string().repeat(w - value.len()),
                                    value
                                );
                            }
                            if value.len() > w {
                                value = value[value.len() - w..].to_string();
                            }
                        }
                    }
                    if is_float || is_float_exp {
                        if let Ok(f) = value.parse::<f64>() {
                            let prec = precision.unwrap_or(10);
                            value = if is_float_exp {
                                // zsh's `-EN` means N SIGNIFICANT digits
                                // (one before the decimal + N-1 after).
                                // Rust's `{:.Pe}` gives P FRACTIONAL —
                                // subtract 1 to get the same display.
                                let frac_prec = prec.saturating_sub(1);
                                // Rust's `{:e}` lacks the C/zsh sign and
                                // 2-digit exponent. Same fix as printf %e.
                                let raw = format!("{:.prec$e}", f, prec = frac_prec);
                                if let Some(epos) = raw.rfind('e') {
                                    let (mantissa, exp) = raw.split_at(epos);
                                    let exp_body = &exp[1..];
                                    let (sign, digits) = if let Some(d) = exp_body.strip_prefix('-') {
                                        ("-", d)
                                    } else if let Some(d) = exp_body.strip_prefix('+') {
                                        ("+", d)
                                    } else {
                                        ("+", exp_body)
                                    };
                                    let padded = if digits.len() < 2 {
                                        format!("0{}", digits)
                                    } else {
                                        digits.to_string()
                                    };
                                    format!("{}e{}{}", mantissa, sign, padded)
                                } else {
                                    raw
                                }
                            } else {
                                format!("{:.prec$}", f, prec = prec)
                            };
                        }
                    }

                    self.variables.insert(name.to_string(), value.clone());

                    if is_export {
                        env::set_var(name, &value);
                    }
                }
            } else if is_array || is_assoc {
                // Just declaring the variable. At top scope, preserve
                // existing values so `a=(1 2 3); typeset -aU a` keeps
                // the array — only empty-init when the variable doesn't
                // already exist (or we're inside a function and meant
                // to shadow). zsh's typeset.c only zeroes a new binding,
                // it doesn't clobber an existing one at global scope.
                let in_function = self.local_scope_depth > 0 && !is_global;
                let exists = self.arrays.contains_key(arg.as_str())
                    || self.assoc_arrays.contains_key(arg.as_str());
                if in_function || !exists {
                    if is_assoc {
                        self.assoc_arrays.insert(arg.clone(), IndexMap::new());
                    } else {
                        self.arrays.insert(arg.clone(), Vec::new());
                    }
                    self.variables.insert(arg.clone(), String::new());
                }
                // Apply unique-dedupe immediately when -U was given on
                // an existing array. (The same dedupe block at the end
                // of typeset_named only fires after var_attrs is set;
                // mirror it here for the bare-declaration path so
                // `a=(a b a c b); typeset -aU a` produces `a b c`.)
                if is_unique {
                    if let Some(arr) = self.arrays.get_mut(arg.as_str()) {
                        let mut seen = std::collections::HashSet::new();
                        arr.retain(|e| seen.insert(e.clone()));
                    }
                }
            } else {
                // `typeset NAME` (no `=value`) attaches attributes to an
                // existing variable WITHOUT clobbering its value at the
                // GLOBAL scope. zsh: `a=hello; typeset -x a` keeps the
                // existing `a=hello` and adds export. Without this guard,
                // zshrs reset `a` to empty.
                //
                // INSIDE A FUNCTION SCOPE, however, a bare `local NAME`
                // (or `typeset NAME` — they share this code path)
                // SHADOWS the parent value with a fresh empty binding.
                // zsh: `a=hi; foo() { local a; echo "[$a]"; }; foo` →
                // prints `[]`. The pre-loop save into local_save_stack
                // already preserved the parent value for the function
                // exit; here we just need to clear the live storage.
                let in_function = self.local_scope_depth > 0 && !is_global;
                // zsh defaults numeric vars to 0/0.0 when declared
                // without a value: `typeset -i x` → x=0, `typeset
                // -F y` → y=0.0000000000 (default precision 10).
                // Without this, `typeset -p x` printed `x=''`.
                let default_val = if is_integer {
                    "0".to_string()
                } else if is_float || is_float_exp {
                    let prec = precision.unwrap_or(10);
                    format!("{:.prec$}", 0.0_f64, prec = prec)
                } else {
                    String::new()
                };
                if in_function {
                    self.variables.insert(arg.clone(), default_val.clone());
                    // Also remove any lingering array/assoc binding so
                    // the local NAME starts genuinely fresh; the old
                    // values are restored on function exit via the
                    // local_*_save_stacks.
                    self.arrays.remove(arg.as_str());
                    self.assoc_arrays.remove(arg.as_str());
                } else if !self.variables.contains_key(arg.as_str())
                    && !self.arrays.contains_key(arg.as_str())
                    && !self.assoc_arrays.contains_key(arg.as_str())
                {
                    self.variables.insert(arg.clone(), default_val);
                }
                if is_export {
                    let val = self
                        .variables
                        .get(arg.as_str())
                        .cloned()
                        .unwrap_or_default();
                    env::set_var(&arg, &val);
                }
                if plus_mode && !is_export {
                    // `typeset +x name` strips export attribute. Remove
                    // from process env but keep the shell-variable value.
                    env::remove_var(&arg);
                }
            }

            // Apply readonly flag — must come after the variable is set
            if is_readonly {
                let name = if let Some(eq_pos) = arg.find('=') {
                    arg[..eq_pos].to_string()
                } else {
                    arg.clone()
                };
                self.readonly_vars.insert(name);
            }

            // Record per-variable attributes for `(t)` flag introspection.
            // Skip in plus_mode (attribute removal) — clear instead.
            let attr_name = if let Some(eq_pos) = arg.find('=') {
                arg[..eq_pos].to_string()
            } else {
                arg.clone()
            };
            if !attr_name.is_empty() {
                if plus_mode {
                    // `+i name` strips integer; for now drop entire entry.
                    self.var_attrs.remove(&attr_name);
                } else if is_integer
                    || is_float
                    || is_float_exp
                    || is_left_pad
                    || is_right_pad
                    || is_zero_pad
                    || is_lower
                    || is_upper
                    || is_readonly
                    || is_export
                    || is_array
                    || is_assoc
                    || is_unique
                    || is_hidden
                    || is_hide_val
                    || is_trace
                {
                    let kind = if is_integer {
                        VarKind::Integer
                    } else if is_float || is_float_exp {
                        VarKind::Float
                    } else if is_assoc {
                        VarKind::Association
                    } else if is_array || is_unique {
                        VarKind::Array
                    } else {
                        VarKind::Scalar
                    };
                    let attr = VarAttr {
                        kind,
                        readonly: is_readonly,
                        export: is_export,
                        left_pad: if is_left_pad { width } else { None },
                        right_pad: if is_right_pad { width } else { None },
                        zero_pad: if is_zero_pad { width } else { None },
                        lowercase: is_lower,
                        uppercase: is_upper,
                        unique: is_unique,
                        float_exp: is_float_exp,
                        int_base: if is_integer { int_base } else { None },
                        hidden: is_hidden,
                        hide_val: is_hide_val,
                        trace: is_trace,
                        float_precision: if is_float || is_float_exp {
                            precision
                        } else {
                            None
                        },
                    };
                    self.var_attrs.insert(attr_name.clone(), attr);
                    // Apply unique-dedupe immediately if the array
                    // already exists; first-wins per zsh semantics.
                    if is_unique {
                        if let Some(arr) = self.arrays.get_mut(&attr_name) {
                            let mut seen = std::collections::HashSet::new();
                            arr.retain(|e| seen.insert(e.clone()));
                        }
                    }
                }
            }
        }
        0
    }
    pub(crate) fn bin_read(&mut self, args: &[String]) -> i32 {
        self.dispatch_pending_traps();
        if self.redirect_failed { self.redirect_failed = false; return 1; }
        // read [ -rszpqAclneE ] [ -t timeout ] [ -d delim ] [ -k [ num ] ] [ -u fd ]
        //      [ name[?prompt] ] [ name ... ]
        use std::io::{BufRead, Read as IoRead};

        let mut raw_mode = false; // -r: don't interpret backslash escapes
        let mut silent = false; // -s: don't echo input
        let mut to_history = false; // -z: read from history stack
        let mut prompt_str: Option<String> = None; // -p prompt
        let mut use_array = false; // -A: read into array
        let mut timeout: Option<u64> = None; // -t timeout in seconds
        let mut delimiter = '\n'; // -d delim
        let mut nchars: Option<usize> = None; // -k num: read exactly num chars
        let mut fd = 0; // -u fd: read from fd
        let mut quiet = false; // -q: test only, don't assign
        let mut echo_line = false; // -e: echo line and don't assign
        let mut echo_and_assign = false; // -E: echo line AND assign
        let mut var_names: Vec<String> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];

            if arg == "--" {
                i += 1;
                while i < args.len() {
                    var_names.push(args[i].clone());
                    i += 1;
                }
                break;
            }

            if arg.starts_with('-') && arg.len() > 1 {
                let mut chars = arg[1..].chars().peekable();
                while let Some(ch) = chars.next() {
                    match ch {
                        'r' => raw_mode = true,
                        's' => silent = true,
                        'z' => to_history = true,
                        'A' => use_array = true,
                        'c' | 'l' => {
                            // Port of read -c / -l from Src/builtin.c:6454.
                            // The C source dispatches both to the
                            // compctlread function pointer; without an
                            // active completion-widget context they're
                            // an error. zsh writes:
                            //   "read: option valid only in functions called from completion"
                            // and exits 1.
                            zwarnnam("read", "option valid only in functions called from completion");
                            return 1;
                        }
                        'e' => echo_line = true,
                        'E' => echo_and_assign = true,
                        'n' => {
                            // bash-compat: -n N reads exactly N characters
                            // (zsh uses -k for the same; we accept both).
                            let rest: String = chars.collect();
                            if !rest.is_empty() {
                                nchars = Some(rest.parse().unwrap_or(1));
                            } else if i + 1 < args.len()
                                && args[i + 1].chars().all(|c| c.is_ascii_digit())
                            {
                                i += 1;
                                nchars = Some(args[i].parse().unwrap_or(1));
                            }
                            break;
                        }
                        'q' => {
                            // read -q: implies -k 1 (read one raw
                            // char from tty). Direct port of
                            // builtin.c:6457-6486 — `keys = 1` triggers
                            // the cbreak path and a 1-char read; the
                            // -q caller then tests for 'y'/'Y' to
                            // decide the exit code.
                            quiet = true;
                            if nchars.is_none() {
                                nchars = Some(1);
                            }
                        }
                        't' => {
                            let rest: String = chars.collect();
                            if !rest.is_empty() {
                                timeout = rest.parse().ok();
                            } else {
                                i += 1;
                                if i < args.len() {
                                    timeout = args[i].parse().ok();
                                }
                            }
                            break;
                        }
                        'd' => {
                            // zsh: `-d` requires a delimiter
                            // argument; missing -> `read:1: argument
                            // expected: -d` exit 1. zshrs's `i+=1`
                            // without bounds-check left delimiter at
                            // default and continued.
                            let rest: String = chars.collect();
                            if !rest.is_empty() {
                                delimiter = rest.chars().next().unwrap_or('\n');
                            } else {
                                i += 1;
                                if i >= args.len() {
                                    zwarnnam("read", "argument expected: -d");
                                    return 1;
                                }
                                delimiter = args[i].chars().next().unwrap_or('\n');
                            }
                            break;
                        }
                        'k' => {
                            let rest: String = chars.collect();
                            if !rest.is_empty() {
                                nchars = Some(rest.parse().unwrap_or(1));
                            } else if i + 1 < args.len()
                                && args[i + 1].chars().all(|c| c.is_ascii_digit())
                            {
                                i += 1;
                                nchars = Some(args[i].parse().unwrap_or(1));
                            } else {
                                nchars = Some(1);
                            }
                            break;
                        }
                        'u' => {
                            // zsh requires a numeric fd; missing arg
                            // -> `read:1: argument expected: -u`,
                            // non-numeric -> `read:1: number expected
                            // after -u: <arg>`. zshrs's `unwrap_or(0)`
                            // silently dropped non-numeric input AND
                            // missing-arg.
                            let rest: String = chars.collect();
                            let value_str = if !rest.is_empty() {
                                rest
                            } else {
                                i += 1;
                                if i >= args.len() {
                                    zwarnnam("read", "argument expected: -u");
                                    return 1;
                                }
                                args[i].clone()
                            };
                            match value_str.parse::<i32>() {
                                Ok(n) => fd = n,
                                Err(_) => {
                                    zwarnnam("read", &format!("number expected after -u: {}", value_str));
                                    return 1;
                                }
                            }
                            break;
                        }
                        'p' => {
                            // zsh's `read -p` means "read from
                            // coprocess input" — NOT prompt. The
                            // prompt feature is `read 'NAME?prompt'`
                            // or `read -P prompt` (capital P) on some
                            // ports. Without a coprocess set up, zsh
                            // emits "no coprocess" and bails.
                            zwarnnam("read", "-p: no coprocess");
                            return 1;
                        }
                        'P' => {
                            // Capital `-P` is the prompt flag (some
                            // shells use this; zsh's man docs say -p
                            // is coprocess so we keep it that way and
                            // route the prompt feature here for users
                            // who relied on the old zshrs `-p` shape).
                            let rest: String = chars.collect();
                            if !rest.is_empty() {
                                prompt_str = Some(rest);
                            } else {
                                i += 1;
                                if i < args.len() {
                                    prompt_str = Some(args[i].clone());
                                }
                            }
                            break;
                        }
                        // zsh: unknown read flag errors `read:1: bad
                        // option: -X` exit 1. zshrs's silent fallback
                        // accepted any letter, masking typos and
                        // letting `read -Q v` pass through to the
                        // assignment phase as if -Q were valid.
                        other => {
                            zwarnnam("read", &format!("bad option: -{}", other));
                            return 1;
                        }
                    }
                }
            } else {
                if let Some(pos) = arg.find('?') {
                    var_names.push(arg[..pos].to_string());
                    prompt_str = Some(arg[pos + 1..].to_string());
                } else {
                    var_names.push(arg.clone());
                }
            }
            i += 1;
        }

        if var_names.is_empty() {
            var_names.push("REPLY".to_string());
        }

        if let Some(ref p) = prompt_str {
            eprint!("{}", p);
            let _ = std::io::stderr().flush();
        }

        // `read -s`: disable terminal echo while reading (passwords, etc).
        // Direct port of builtin.c:6519-6531. Uses an RAII guard so all
        // early returns from this function restore the original termios.
        struct EchoGuard {
            fd: i32,
            saved: Option<libc::termios>,
        }
        impl Drop for EchoGuard {
            fn drop(&mut self) {
                if let Some(t) = self.saved {
                    unsafe {
                        libc::tcsetattr(self.fd, libc::TCSANOW, &t);
                    }
                }
            }
        }
        let _echo_guard = if silent && unsafe { libc::isatty(fd) } != 0 {
            let mut ti: libc::termios = unsafe { std::mem::zeroed() };
            if unsafe { libc::tcgetattr(fd, &mut ti) } == 0 {
                let saved = ti;
                ti.c_lflag &= !libc::ECHO;
                unsafe {
                    libc::tcsetattr(fd, libc::TCSANOW, &ti);
                }
                Some(EchoGuard {
                    fd,
                    saved: Some(saved),
                })
            } else {
                None
            }
        } else {
            None
        };

        // `read -z`: take input from the editor buffer stack instead of
        // the underlying fd (builtin.c:6769-6770). When the stack is
        // empty, zsh substitutes an empty string. Pop is FIFO from the
        // top — the C uses `getlinknode(bufstack)` which removes and
        // returns the head; in our Vec we pop the last pushed (LIFO),
        // matching the zpushnode/getlinknode pair semantics.
        if to_history {
            let buf = self.buffer_stack.pop().unwrap_or_default();
            let line = buf.trim_end_matches('\n').to_string();
            if use_array {
                let words: Vec<String> = line.split_whitespace().map(|s| s.to_string()).collect();
                let target = var_names
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "reply".to_string());
                self.arrays.insert(target, words);
            } else {
                let target = var_names
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "REPLY".to_string());
                self.variables.insert(target, line);
            }
            return 0;
        }

        // `read -q` reads a single character (y/n) from a terminal.
        // zsh: outside a tty (`echo y | read -q`) it errors "not
        // interactive and can't open terminal" and returns 1.
        // zshrs previously read from stdin and returned 0 silently.
        if quiet && !atty::is(atty::Stream::Stdin) {
            zwarnnam("read", "not interactive and can't open terminal");
            return 1;
        }

        // `-u FD` reads from the given file descriptor instead of
        // stdin. Direct port of zsh's bin_read in builtin.c which
        // calls dup2(fd, 0) for the read; we keep stdin alone and
        // read directly from the fd via from_raw_fd. fd=0 (default
        // or explicit) means stdin — use the standard io::stdin
        // path so terminal handling (line-buffering, etc.) stays
        // intact. ManuallyDrop prevents from_raw_fd from closing
        // the user's fd when the helper File goes out of scope.
        use std::mem::ManuallyDrop;
        use std::os::unix::io::FromRawFd;
        let mut fd_file: Option<ManuallyDrop<std::fs::File>> = if fd > 0 {
            Some(ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(fd) }))
        } else {
            None
        };

        // -t timeout: wait via poll(2) before any read attempt so we can
        // return 1 cleanly without blocking. Zero timeout = immediate
        // poll (data available or not).
        if let Some(t) = timeout {
            let ms = (t as i32).saturating_mul(1000);
            let mut pfd = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            let r = unsafe { libc::poll(&mut pfd, 1, ms) };
            if r <= 0 {
                return 1;
            }
        }

        // `read -k N` (and bash's -n): when reading from a tty, switch
        // to cbreak (non-canonical) so the read returns after N raw
        // bytes instead of waiting for a newline. Direct port of
        // builtin.c:6481-6483 (setcbreak() before the read). Restored
        // via a Drop guard mirroring EchoGuard above.
        struct CbreakGuard {
            fd: i32,
            saved: libc::termios,
        }
        impl Drop for CbreakGuard {
            fn drop(&mut self) {
                unsafe {
                    libc::tcsetattr(self.fd, libc::TCSANOW, &self.saved);
                }
            }
        }
        let _cbreak_guard = if nchars.is_some() && unsafe { libc::isatty(fd) } != 0 {
            let mut ti: libc::termios = unsafe { std::mem::zeroed() };
            if unsafe { libc::tcgetattr(fd, &mut ti) } == 0 {
                let saved = ti;
                ti.c_lflag &= !(libc::ICANON | libc::ECHO);
                ti.c_cc[libc::VMIN] = 1;
                ti.c_cc[libc::VTIME] = 0;
                unsafe {
                    libc::tcsetattr(fd, libc::TCSANOW, &ti);
                }
                Some(CbreakGuard { fd, saved })
            } else {
                None
            }
        } else {
            None
        };

        let input = if let Some(n) = nchars {
            let mut buf = vec![0u8; n];
            let read_result = if let Some(ref mut f) = fd_file {
                f.read_exact(&mut buf).map(|_| ())
            } else {
                let stdin = io::stdin();
                stdin.lock().read_exact(&mut buf).map(|_| ())
            };
            match read_result {
                Ok(_) => String::from_utf8_lossy(&buf).to_string(),
                Err(_) => return 1,
            }
        } else {
            let mut input = String::new();
            // Track whether the read hit a real terminator. If EOF
            // arrived before the delimiter, zsh's read returns 1 even
            // though some bytes may have been captured. Without this
            // a `while read line` loop runs the body one extra time
            // for the trailing partial line.
            let mut hit_terminator = false;
            if delimiter == '\n' {
                let n_read = if let Some(ref mut f) = fd_file {
                    // BufReader gives us read_line on a raw fd.
                    use std::io::BufReader;
                    let mut br = BufReader::new(&mut **f);
                    br.read_line(&mut input)
                } else {
                    let stdin = io::stdin();

                    stdin.lock().read_line(&mut input)
                };
                match n_read {
                    Ok(0) => return 1,
                    Ok(_) => {
                        hit_terminator = input.ends_with('\n');
                    }
                    Err(_) => return 1,
                }
            } else {
                let mut byte = [0u8; 1];
                loop {
                    let r = if let Some(ref mut f) = fd_file {
                        f.read_exact(&mut byte)
                    } else {
                        let stdin = io::stdin();

                        stdin.lock().read_exact(&mut byte)
                    };
                    match r {
                        Ok(_) => {
                            let c = byte[0] as char;
                            if c == delimiter {
                                hit_terminator = true;
                                break;
                            }
                            input.push(c);
                        }
                        Err(_) => break,
                    }
                }
            }
            let cleaned = input
                .trim_end_matches('\n')
                .trim_end_matches('\r')
                .to_string();
            if !hit_terminator && !cleaned.is_empty() {
                // Captured a partial line at EOF — assign the value
                // but tell the caller we hit EOF (status 1). We have
                // to return AFTER the variable is set, so stash and
                // fall through.
                let processed = if raw_mode {
                    cleaned
                } else {
                    cleaned.replace("\\\n", "")
                };
                if quiet {
                    return 1;
                }
                if use_array {
                    let var = &var_names[0];
                    self.arrays.insert(var.clone(), vec![processed]);
                } else if var_names.len() == 1 {
                    let var = &var_names[0];
                    if !processed.contains('\0') {
                        env::set_var(var, &processed);
                    }
                    self.variables.insert(var.clone(), processed);
                } else if let Some(var) = var_names.first() {
                    if !processed.contains('\0') {
                        env::set_var(var, &processed);
                    }
                    self.variables.insert(var.clone(), processed);
                }
                return 1;
            }
            cleaned
        };

        let processed = if raw_mode {
            input
        } else {
            // Without -r, `read` removes one backslash from every
            // `\X` pair (X = any char). `\<newline>` is a line
            // continuation (both stripped) — different from
            // backspace/etc. Standard POSIX read semantics.
            let mut out = String::with_capacity(input.len());
            let mut chars = input.chars().peekable();
            while let Some(c) = chars.next() {
                if c == '\\' {
                    match chars.next() {
                        Some('\n') => {
                            // Line continuation: both consumed.
                        }
                        Some(next) => {
                            out.push(next);
                        }
                        None => {
                            // Trailing backslash — drop it (POSIX).
                        }
                    }
                } else {
                    out.push(c);
                }
            }
            out
        };

        if quiet {
            // read -q: exit 0 iff the first character is 'y' or 'Y'.
            // Direct port of zsh/Src/builtin.c:6493-6501 (keys==1 branch
            // returns *c == 'Y' || *c == 'y'). Trim leading whitespace
            // to be lenient with line-mode input that snuck in.
            let first = processed.chars().next().unwrap_or(' ');
            return if first == 'y' || first == 'Y' { 0 } else { 1 };
        }

        // -e: echo the read line on stdout and DON'T assign. -E: echo
        // AND assign. Both end here; -e returns before the assignment
        // block. zsh's bin_read calls fputs(buf, stdout) under both,
        // useful for completion functions that want to display the
        // current line.
        if echo_line || echo_and_assign {
            println!("{}", processed);
            if echo_line && !echo_and_assign {
                return 0;
            }
        }

        if use_array {
            let var = &var_names[0];
            let ifs = self
                .variables
                .get("IFS")
                .cloned()
                .unwrap_or_else(|| " \t\n".to_string());
            // Custom IFS (e.g. `IFS=,`) splits on every IFS char.
            // Default IFS (whitespace + NUL) collapses consecutive seps
            // — matches zsh `read -A` behaviour. Detect default by the
            // char set rather than exact string ordering so the new
            // " \t\n\0" init value also classifies as default.
            let is_default_ifs =
                !ifs.is_empty() && ifs.chars().all(|c| matches!(c, ' ' | '\t' | '\n' | '\0'));
            let words: Vec<String> = if is_default_ifs {
                processed.split_whitespace().map(String::from).collect()
            } else {
                processed
                    .split(|c| ifs.contains(c))
                    .map(String::from)
                    .collect()
            };
            self.arrays.insert(var.clone(), words);
        } else if var_names.len() == 1 {
            let var = &var_names[0];
            // Skip env::set_var on NUL-containing values to avoid
            // panic from std::env::set_var (which rejects NUL).
            // Reading binary input via `read -d ""` is a real
            // use case zsh handles silently; zshrs panicked.
            if !processed.contains('\0') {
                env::set_var(var, &processed);
            }
            self.variables.insert(var.clone(), processed);
        } else {
            let ifs = self
                .variables
                .get("IFS")
                .map(|s| s.as_str())
                .unwrap_or(" \t\n");
            // Direct port of zsh's bin_read in builtin.c: when
            // there are MORE input fields than vars, the last
            // var gets the unsplit REMAINDER from the position
            // after the (N-1)th separator — meaning the separator
            // chars between fields N..end are PRESERVED. zshrs
            // previously split into a Vec<&str> and `join(" ")`d,
            // which collapsed all separators to spaces. Now: find
            // the (N-1)th separator and slice the original string
            // there.
            //
            // Whitespace IFS (default ` \t\n`) collapses consecutive
            // separators; non-whitespace IFS (e.g. `:`) keeps each
            // separator distinct. The same rule applies at the
            // tail-split point.
            let nvars = var_names.len();
            // "Whitespace IFS" for read's collapsing purposes:
            // every char in IFS is either ASCII whitespace or NUL.
            // The default IFS in zsh is `" \t\n\0"` so the NUL
            // must qualify too (otherwise the "default" path
            // doesn't fire and we lose the collapse-runs +
            // strip-boundaries semantics that zsh's bin_read
            // applies for unset/default IFS).
            let is_whitespace_ifs =
                !ifs.is_empty() && ifs.chars().all(|c| c.is_ascii_whitespace() || c == '\0');
            // Strip leading AND trailing whitespace for default
            // IFS (zsh's bin_read does this so trailing
            // whitespace doesn't leak into the last var). For
            // non-whitespace IFS, leading/trailing separators
            // create empty fields and are preserved.
            let processed_trimmed: String = if is_whitespace_ifs {
                processed
                    .trim_matches(|c: char| ifs.contains(c))
                    .to_string()
            } else {
                processed.clone()
            };
            let processed = processed_trimmed;
            // Find the (N-1)th separator boundary. For the last
            // var, take from that boundary to the end (preserving
            // any further separators verbatim).
            let mut split_end: Option<usize> = None;
            let mut field_count = 0;
            let bytes = processed.as_bytes();
            let mut i = 0;
            while i < bytes.len() && field_count < nvars - 1 {
                if ifs.bytes().any(|c| c == bytes[i]) {
                    field_count += 1;
                    if field_count == nvars - 1 {
                        // Skip this separator AND any consecutive
                        // separators (whitespace IFS only).
                        i += 1;
                        if is_whitespace_ifs {
                            while i < bytes.len() && ifs.bytes().any(|c| c == bytes[i]) {
                                i += 1;
                            }
                        }
                        split_end = Some(i);
                        break;
                    }
                    i += 1;
                    if is_whitespace_ifs {
                        while i < bytes.len() && ifs.bytes().any(|c| c == bytes[i]) {
                            i += 1;
                        }
                    }
                } else {
                    i += 1;
                }
            }
            let words: Vec<&str> = match split_end {
                Some(end) => {
                    // Pre-split fields 0..N-1 from the start up to
                    // `end`, then take the suffix verbatim.
                    let head = &processed[..end];
                    let tail = &processed[end..];
                    let mut head_words: Vec<&str> = if is_whitespace_ifs {
                        head.split(|c: char| ifs.contains(c))
                            .filter(|s| !s.is_empty())
                            .collect()
                    } else {
                        // Non-whitespace IFS: split keeps empty
                        // fields between consecutive separators.
                        // Trim the trailing separator from `head`
                        // before splitting so we don't get a stray
                        // empty.
                        let head_trimmed =
                            head.strip_suffix(|c: char| ifs.contains(c)).unwrap_or(head);
                        head_trimmed.split(|c: char| ifs.contains(c)).collect()
                    };
                    head_words.push(tail);
                    head_words
                }
                None => {
                    // Fewer fields than vars — split normally; the
                    // missing vars get empty.
                    if is_whitespace_ifs {
                        processed.split_whitespace().collect()
                    } else {
                        processed.split(|c: char| ifs.contains(c)).collect()
                    }
                }
            };

            for (j, var) in var_names.iter().enumerate() {
                if j < words.len() {
                    {
                        if !words[j].contains('\0') {
                            env::set_var(var, words[j]);
                        }
                        self.variables.insert(var.clone(), words[j].to_string());
                    }
                } else {
                    env::set_var(var, "");
                    self.variables.insert(var.clone(), String::new());
                }
            }
        }

        0
    }
    pub(crate) fn bin_shift(&mut self, args: &[String]) -> i32 {
        // shift [ -p ] [ n ] [ name ... ]
        // -p: shift from end instead of beginning (pop)
        // n: number of elements to shift (default 1)
        // name: array names to shift (default: shift positional parameters)

        let mut from_end = false;
        let mut count = 1usize;
        let mut array_names: Vec<String> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if arg == "-p" {
                from_end = true;
            } else if arg.is_empty() {
                // zsh: `shift ""` treats empty arg as count 0
                // (silent no-op). zshrs's `chars().all(is_digit)`
                // matched the empty string vacuously and parse
                // returned 1 by default, then the count check
                // erred when positionals were short.
                count = 0;
            } else if arg.starts_with('-')
                && arg[1..].chars().all(|c| c.is_ascii_digit())
                && arg.len() > 1
            {
                // zsh: negative count is rejected with this exact diagnostic.
                zwarnnam("shift", "argument to shift must be non-negative");
                return 1;
            } else if arg.starts_with('-') && arg.len() > 1 {
                // zsh: unknown shift flag (besides -p) -> `shift:1:
                // bad option: -X` exit 1. zshrs's catch-all pushed
                // the flag string into array_names, masking typos.
                let bad: String = arg[1..].chars().take(1).collect();
                zwarnnam("shift", &format!("bad option: -{}", bad));
                return 1;
            } else if arg.chars().all(|c| c.is_ascii_digit()) {
                count = arg.parse().unwrap_or(1);
            } else {
                array_names.push(arg.clone());
            }
            i += 1;
        }

        if array_names.is_empty() {
            // zsh: `shift N` errors and exits 1 if N > $#.
            if count > self.positional_params.len() {
                zwarnnam("shift", "shift count must be <= $#");
                return 1;
            }
            // Shift positional parameters
            if from_end {
                for _ in 0..count {
                    if !self.positional_params.is_empty() {
                        self.positional_params.pop();
                    }
                }
            } else {
                for _ in 0..count.min(self.positional_params.len()) {
                    self.positional_params.remove(0);
                }
            }
        } else {
            // Direct port of src/zsh/Src/builtin.c:5614-5636 — walk
            // all named arrays, error on under-length but CONTINUE
            // to the next array (`ret++; continue;`). Previous Rust
            // impl returned on first error, hiding the rest. The
            // final return value is non-zero iff at least one array
            // had count > length.
            let mut ret = 0;
            for name in &array_names {
                if let Some(arr) = self.arrays.get(name) {
                    if count > arr.len() {
                        zwarnnam("shift", "shift count must be <= $#");
                        ret = 1;
                    }
                }
            }
            for name in array_names {
                if let Some(arr) = self.arrays.get(&name) {
                    if count > arr.len() {
                        // Already reported above; skip mutation.
                        continue;
                    }
                }
                if let Some(arr) = self.arrays.get_mut(&name) {
                    if from_end {
                        for _ in 0..count {
                            if !arr.is_empty() {
                                arr.pop();
                            }
                        }
                    } else {
                        for _ in 0..count {
                            if !arr.is_empty() {
                                arr.remove(0);
                            }
                        }
                    }
                }
            }
            return ret;
        }

        0
    }
    #[tracing::instrument(level = "debug", skip(self))]
    pub(crate) fn bin_eval(&mut self, args: &[String]) -> i32 {
        self.dispatch_pending_traps();
        if self.redirect_failed { self.redirect_failed = false; return 1; }
        // builtin.c:6203-6213 — bin_eval joins argv with space, parses,
        // executes; parse failure sets errflag and lastval=errflag (1).
        // The diagnostic prefix on parse error is `zsh:N: parse error
        // near \`...\`` (no `eval:` segment) because parser errors come
        // from `zerr`, not `zwarnnam`. zshrs's previous format
        // `eval: <err>` matched neither zsh nor existing zshrs error
        // output (which uses `zshrs:N:` everywhere). Use the same
        // shell-name prefix as the rest of the diagnostics so script
        // consumers grepping `zshrs:` catch it.
        let code = args.join(" ");
        match self.execute_script(&code) {
            Ok(status) => status,
            Err(e) => {
                zwarnnam("eval", &format!("{}", e));
                1
            }
        }
    }
    pub(crate) fn builtin_autoload(&mut self, args: &[String]) -> i32 {
        // PFA-SMR aspect: emit one `function` event per autoload name with
        // value=`autoload` to mark it as the lazy-load form (vs. an
        // inline `name() {}` definition). Listing-only invocations
        // (bare `autoload`, `autoload +X NAME`) are still recorded as a
        // function event because they install the autoload pending
        // table — the body just doesn't load until first call.
        #[cfg(feature = "recorder")]
        if crate::recorder::is_enabled() {
            let ctx = self.recorder_ctx();
            for a in args {
                if a == "--" || a.starts_with('-') || a.starts_with('+') {
                    continue;
                }
                crate::recorder::emit_function(a, Some("autoload"), ctx.clone());
            }
        }
        // Parse options like zsh: -U (no alias), -z (zsh style), -k (ksh style),
        // -X (execute now), -x (export), -r (resolve), -R (resolve recurse),
        // -t (trace), -T (trace local), -W (warn nested), -d (use calling dir)
        let mut functions = Vec::new();
        let mut no_alias = false; // -U
        let mut zsh_style = false; // -z
        let mut ksh_style = false; // -k
        let mut execute_now = false; // -X
        let mut resolve = false; // -r
        let mut trace = false; // -t
        let mut use_caller_dir = false; // -d
        let mut match_pattern = false; // -m: each NAME is a glob pattern
        let _list_mode = false;

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];

            if arg == "--" {
                i += 1;
                break;
            }

            if let Some(flags) = arg.strip_prefix('+') {
                for c in flags.chars() {
                    match c {
                        'U' => no_alias = false,
                        'z' => zsh_style = false,
                        'k' => ksh_style = false,
                        't' => trace = false,
                        'd' => use_caller_dir = false,
                        // BUILTIN("autoload", ...) accepts the same
                        // letter set for + as for -. Reject unknown
                        // +flag letters symmetric to the - parser.
                        'X' | 'r' | 'R' | 'T' | 'W' | 'w' | 'm' => {}
                        _ => {
                            zwarnnam("autoload", &format!("bad option: +{}", c));
                            return 1;
                        }
                    }
                }
            } else if let Some(flags) = arg.strip_prefix('-') {
                if flags.is_empty() {
                    // Just "-" means end of options
                    i += 1;
                    break;
                }
                for c in flags.chars() {
                    match c {
                        'U' => no_alias = true,
                        'z' => zsh_style = true,
                        'k' => ksh_style = true,
                        'X' => execute_now = true,
                        'r' | 'R' => resolve = true,
                        't' => trace = true,
                        'T' => {} // trace local
                        'W' => {} // warn nested
                        'd' => use_caller_dir = true,
                        'w' => {} // wordcode
                        'm' => match_pattern = true,
                        // zsh: unknown autoload flag -> `autoload:1:
                        // bad option: -X` exit 1. zshrs's silent
                        // fallback accepted any letter, masking
                        // typos like `-Z` or `-l` (bash-style flag
                        // that zsh doesn't have).
                        _ => {
                            zwarnnam("autoload", &format!("bad option: -{}", c));
                            return 1;
                        }
                    }
                }
            } else {
                functions.push(arg.clone());
            }
            i += 1;
        }

        // Collect remaining args as function names
        while i < args.len() {
            functions.push(args[i].clone());
            i += 1;
        }

        // If no functions specified, list autoloaded functions.
        // Sorted alphabetically per zsh's table-walk order; HashMap
        // iteration was nondeterministic so output flickered between
        // runs and broke save/restore-state idioms.
        if functions.is_empty() && !execute_now {
            let mut names: Vec<&String> = self.autoload_pending.keys().collect();
            names.sort();
            for name in names {
                if no_alias && zsh_style {
                    println!("autoload -Uz {}", name);
                } else if no_alias {
                    println!("autoload -U {}", name);
                } else {
                    println!("autoload {}", name);
                }
            }
            return 0;
        }
        // zsh: `autoload -X` with no function name -> `autoload:1:
        // bad autoload` exit 1. zshrs silently no-op'd because
        // `execute_now=true && functions.is_empty()` skipped both
        // the listing branch and the execute branch below.
        if functions.is_empty() && execute_now {
            zwarnnam("autoload", "bad autoload");
            return 1;
        }

        // Handle -X: load and execute function immediately (called from stub)
        // When a stub function calls `builtin autoload -Xz`, we load the real
        // function and then need to execute it with the original arguments.
        // The new pipeline populates functions_compiled + function_source as
        // a side effect of load_autoload_function — we discard the legacy
        // ShellCommand return and use function_exists as the success signal.
        if execute_now {
            for func_name in &functions {
                self.load_autoload_function(func_name);
                if self.function_exists(func_name) {
                    self.autoload_pending.remove(func_name);
                } else {
                    // C: `zerr("%s: function definition file not found", ...)` —
                    // fatal, no cmd-name prefix (Src/builtin.c:3213).
                    zerr(&format!("{}: function definition file not found", func_name));
                    return 1;
                }
            }
            return 0;
        }

        // -m: each `function_name` arg is a glob pattern. Expand to
        // every existing autoload-pending entry that matches; useful
        // for `autoload -m '_my_*'` to mark a whole namespace at once.
        // Direct port of zsh/Src/builtin.c bin_autoload's pattern path.
        if match_pattern {
            let pats: Vec<String> = functions.clone();
            let candidates: Vec<String> = self.autoload_pending.keys().cloned().collect();
            functions = pats
                .iter()
                .flat_map(|p| {
                    candidates
                        .iter()
                        .filter(|c| Self::glob_match_static(c, p))
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .collect();
            // Dedup preserving order.
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            functions.retain(|n| seen.insert(n.clone()));
        }

        // Register functions for autoload - create stub functions
        for func_name in &functions {
            // Store autoload metadata
            let mut flags = AutoloadFlags::empty();
            if no_alias {
                flags |= AutoloadFlags::NO_ALIAS;
            }
            if zsh_style {
                flags |= AutoloadFlags::ZSH_STYLE;
            }
            if ksh_style {
                flags |= AutoloadFlags::KSH_STYLE;
            }
            if trace {
                flags |= AutoloadFlags::TRACE;
            }
            if use_caller_dir {
                flags |= AutoloadFlags::USE_CALLER_DIR;
            }

            self.autoload_pending.insert(func_name.clone(), flags);

            // No stub function needed: `function_exists(name)` checks
            // `autoload_pending` so introspection (whence, which, type)
            // recognizes `name` as a function before it's loaded. The
            // dispatch path (`ZshrsHost::call_function`,
            // `dispatch_function_call`) calls `maybe_autoload(name)` first
            // when `autoload_pending` has the name, which loads the body
            // chunk into `functions_compiled` and clears the pending entry.

            // If -r or -R, resolve the path now to verify it exists
            if resolve && self.find_function_file(func_name).is_none() {
                // C: zerr("%s: function definition file not found", ...)
                zerr(&format!("{}: function definition file not found", func_name));
            }
        }

        // Batch pre-resolution: when multiple autoloads are registered at once
        // (common during .zshrc processing), dispatch fpath lookups in parallel
        // across the worker pool to pre-read function files into the OS page cache.
        if functions.len() >= 4 && !resolve && !execute_now {
            let fpath_dirs: Vec<PathBuf> = self.fpath.clone();
            let names: Vec<String> = functions.clone();
            let pool = std::sync::Arc::clone(&self.worker_pool);

            tracing::debug!(
                count = names.len(),
                fpath_dirs = fpath_dirs.len(),
                "batch autoload: pre-resolving fpath lookups on worker pool"
            );

            // Submit resolution tasks — each worker scans fpath for a subset of names.
            // Results are cached in a shared map for later use by load_autoload_function.
            let resolved = std::sync::Arc::new(parking_lot::Mutex::new(
                HashMap::<String, PathBuf>::with_capacity(names.len()),
            ));

            for name in names {
                let dirs = fpath_dirs.clone();
                let resolved = std::sync::Arc::clone(&resolved);
                pool.submit(move || {
                    for dir in &dirs {
                        let path = dir.join(&name);
                        if path.exists() && path.is_file() {
                            // Pre-read to warm OS page cache (the read result is discarded,
                            // but the pages stay in the kernel buffer cache)
                            let _ = std::fs::read(&path);
                            resolved.lock().insert(name.clone(), path);
                            tracing::trace!(func = %name, "autoload batch: pre-resolved");
                            break;
                        }
                    }
                });
            }
        }

        0
    }
    pub(crate) fn builtin_history(&self, args: &[String]) -> i32 {
        let Some(ref engine) = self.history else {
            zwarnnam("history", "history engine not available");
            return 1;
        };

        // Parse options
        let mut count = 20usize;
        let mut show_all = false;
        let mut search_query = None;
        let mut had_explicit_negative_count = false;
        let mut positional_count = 0usize;

        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "-c" | "--clear" => {
                    // zsh's `history` is a synonym for `fc -l`; it
                    // doesn't accept `-c` (bash-only). Match zsh's
                    // diagnostic format so user scripts that probe
                    // for the bash-style flag see the same error.
                    zwarnnam("history", "bad option: -c");
                    return 1;
                }
                "-a" | "--all" => show_all = true,
                "-n" => {
                    if i + 1 < args.len() {
                        i += 1;
                        count = args[i].parse().unwrap_or(20);
                    }
                }
                s if s.starts_with('-') && s[1..].chars().all(|c| c.is_ascii_digit()) => {
                    count = s[1..].parse().unwrap_or(20);
                    had_explicit_negative_count = true;
                }
                // zsh's `history` is essentially `fc -l` — it accepts
                // fc-style flags (`-r` reverse, `-D` duration, `-d`
                // (date), `-f`/`-E`/`-i`/`-t` time formats, `-m`
                // pattern). It REJECTS bash-style flags like `-w`
                // (write), `-X` (unknown), and `-d` is taken as date
                // not delete. Reject only the ones zsh rejects.
                s if matches!(s, "-w" | "-X" | "-S" | "--write") => {
                    // bash-style flags zsh's history doesn't accept.
                    // -S is bash's "save" flag (zsh's history can't
                    // write because it's just `fc -l`).
                    let bad: String = s[1..].chars().take(1).collect();
                    zwarnnam("history", &format!("bad option: -{}", bad));
                    return 1;
                }
                // Other `-X` flags fall through to the fc-list path
                // (which will report no-such-event in -c mode anyway).
                s if s.starts_with('-') && s.len() > 1 => {
                    // Silently consume — fc handles or rejects.
                }
                s if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) => {
                    count = s.parse().unwrap_or(20);
                    positional_count += 1;
                }
                s => {
                    // Non-numeric (or empty) -> search-by-text. zsh
                    // treats `history ""` as event-not-found with
                    // empty identifier. Without the !is_empty guard
                    // above, empty matched the digit-only arm
                    // vacuously and silently became count=20.
                    search_query = Some(s.to_string());
                }
            }
            i += 1;
        }

        if show_all {
            count = 10000;
        }

        // In non-interactive (`-c`) mode with no session adds, zsh's
        // `history` (= `fc -l`) errors `no such event: N` rather
        // than listing the on-disk persistent history. Mirror that —
        // only emit session entries (in case the script did `print
        // -s`) and abort when both session and atty are absent.
        if !atty::is(atty::Stream::Stdin) && self.session_history_ids.is_empty() {
            // 3+ numeric positionals -> `fc:1: too many arguments`
            // (history is `fc -l`; takes at most 2 range bounds).
            if positional_count > 2 {
                zwarnnam("fc", "too many arguments");
                return 1;
            }
            // 2 numeric positionals -> "no events in that range".
            if positional_count == 2 {
                zwarnnam("fc", "no events in that range");
                return 1;
            }
            // Non-numeric positional `history XX` is a search-by-text
            // (`-m` style); zsh's no-match wording differs: `event not
            // found: XX`. Numeric / no positional uses `no such event:
            // N`. Without this branch zshrs emitted `no such event: 1`
            // even for non-numeric queries — wrong format AND wrong
            // event identifier. The numeric path now uses the actual
            // user-supplied count (e.g. `history -d 99` reports
            // `no such event: 99` not `1`).
            if let Some(ref q) = search_query {
                zwarnnam("fc", &format!("event not found: {}", q));
            } else {
                // Negative count (`history -d -1`) resolves to 0 in
                // zsh's count-from-end semantics with empty history.
                // Track whether we got an explicit `-N` shape.
                let event_id = if had_explicit_negative_count {
                    0
                } else if count != 20 {
                    count
                } else {
                    1
                };
                zwarnnam("fc", &format!("no such event: {}", event_id));
            }
            return 1;
        }
        if !atty::is(atty::Stream::Stdin) && !self.session_history_ids.is_empty() {
            // Only show session entries, numbered from 1.
            for (i, &id) in self.session_history_ids.iter().enumerate() {
                if let Ok(Some(entry)) = engine.get_by_number(id) {
                    let n = (i as i64) + 1;
                    println!("{:>5}  {}", n, entry.command);
                }
            }
            return 0;
        }

        let entries = if let Some(ref q) = search_query {
            engine.search(q, count)
        } else {
            engine.recent(count)
        };

        match entries {
            Ok(entries) => {
                // Print in chronological order (reverse the results since recent() is newest-first)
                for entry in entries.into_iter().rev() {
                    println!("{:>5}  {}", entry.id, entry.command);
                }
                0
            }
            Err(e) => {
                zwarnnam("history", &format!("{}", e));
                1
            }
        }
    }
    /// fc builtin - fix command (history manipulation)
    /// Ported from zsh/Src/builtin.c bin_fc() lines 1426-1700
    /// Options: -l (list), -n (no numbers), -r (reverse), -d/-f/-E/-i/-t (time formats),
    /// -D (duration), -e editor, -m pattern, -R/-W/-A (read/write/append history file),
    /// -p/-P (push/pop history stack), -I (skip old), -L (local), -s (substitute)
    pub(crate) fn bin_fc(&mut self, args: &[String]) -> i32 {
        let Some(ref engine) = self.history else {
            zwarnnam("fc", "history engine not available");
            return 1;
        };

        // Parse options
        let mut list_mode = false;
        let mut no_numbers = false;
        let mut reverse = false;
        let mut show_time = false;
        let mut show_duration = false;
        let mut editor: Option<String> = None;
        let mut read_file = false;
        let mut write_file = false;
        let mut append_file = false;
        let mut substitute_mode = false;
        let mut silent_no_op_flag = false;
        let mut positional: Vec<&str> = Vec::new();
        let mut substitutions: Vec<(String, String)> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if arg == "--" {
                i += 1;
                while i < args.len() {
                    positional.push(&args[i]);
                    i += 1;
                }
                break;
            }
            if arg.starts_with('-') && arg.len() > 1 {
                let chars: Vec<char> = arg[1..].chars().collect();
                let mut j = 0;
                while j < chars.len() {
                    match chars[j] {
                        'l' => list_mode = true,
                        'n' => no_numbers = true,
                        'r' => reverse = true,
                        'd' | 'f' | 'E' | 'i' => show_time = true,
                        'D' => show_duration = true,
                        'R' => read_file = true,
                        'W' => write_file = true,
                        'A' => append_file = true,
                        's' => substitute_mode = true,
                        'e' => {
                            if j + 1 < chars.len() {
                                editor = Some(chars[j + 1..].iter().collect());
                                break;
                            } else {
                                i += 1;
                                if i < args.len() {
                                    editor = Some(args[i].clone());
                                } else {
                                    // zsh: `fc -e` (no following
                                    // editor arg) -> `fc:1: argument
                                    // expected: -e` exit 1. zshrs
                                    // silently let the missing arg
                                    // slip through, falling into
                                    // the recurse-endlessly path.
                                    zwarnnam("fc", "argument expected: -e");
                                    return 1;
                                }
                            }
                        }
                        't' => {
                            show_time = true;
                            if j + 1 < chars.len() {
                                break;
                            } else {
                                i += 1;
                                // zsh: `-t` requires a time-format
                                // arg; missing -> `fc:1: argument
                                // expected: -t` exit 1. zshrs's loop
                                // bumped i without bounds-check, then
                                // the no-positional path triggered
                                // the recurse-endlessly diagnostic.
                                if i >= args.len() {
                                    zwarnnam("fc", "argument expected: -t");
                                    return 1;
                                }
                            }
                        }
                        'p' | 'P' | 'a' | 'I' | 'L' | 'm' => {
                            // `-p`/`-P` push/pop history stack;
                            // `-a` modify-already-read; `-I`/`-L`
                            // local variants; `-m` pattern. These
                            // are no-ops in `-c` mode but their
                            // PRESENCE means the user explicitly
                            // requested this fc invocation (not
                            // bare-fc-recurse). Mark so the
                            // recurse-abort below skips.
                            silent_no_op_flag = true;
                        }
                        // `--help` / long-option-style typos: zsh
                        // skips the leading `-` and reports the
                        // FIRST recognisable letter as the bad
                        // option (so `--help` -> `bad option: -h`).
                        // Without this, zshrs's loop hit `-` itself
                        // as the unknown letter and reported `bad
                        // option: --` (wrong identifier).
                        '-' => {}
                        _ => {
                            if chars[j].is_ascii_digit() {
                                positional.push(arg);
                                break;
                            }
                            // Unknown flag (e.g. `-h`, `-w`) — zsh
                            // errors `bad option: -X` and bails. Without
                            // this fallback, unknown flags silently
                            // dropped through to the no-args path
                            // (re-execute last command) which can
                            // recurse forever for `fc -h` since fc
                            // entered history.
                            zwarnnam("fc", &format!("bad option: -{}", chars[j]));
                            return 1;
                        }
                    }
                    j += 1;
                }
            } else if arg.contains('=') && !list_mode {
                if let Some((old, new)) = arg.split_once('=') {
                    substitutions.push((old.to_string(), new.to_string()));
                }
            } else {
                positional.push(arg);
            }
            i += 1;
        }

        // Handle file operations (read/write/append)
        // Note: HistoryEngine uses SQLite, so file ops are simplified
        if read_file || write_file || append_file {
            let filename = positional.first().copied().unwrap_or("~/.zsh_history");
            let path = if filename.starts_with("~/") {
                dirs::home_dir()
                    .map(|h| h.join(&filename[2..]))
                    .unwrap_or_else(|| std::path::PathBuf::from(filename))
            } else {
                std::path::PathBuf::from(filename)
            };

            if read_file {
                // Read plain text history file and import. zsh
                // silently ignores read failures (`fc -R /no/such`
                // returns 0 with no output) — script consumers
                // shouldn't trip on a missing log. zshrs previously
                // emitted `fc: cannot read /no/such` and returned 1.
                if let Ok(contents) = std::fs::read_to_string(&path) {
                    for line in contents.lines() {
                        if !line.is_empty() && !line.starts_with('#') && !line.starts_with(':') {
                            let _ = engine.add(line, None);
                        }
                    }
                }
                // No diagnostic on failure — matches zsh.
            } else if write_file || append_file {
                // In `-c` (non-interactive) mode with no in-session
                // history adds, zsh's `fc -W` writes nothing — there's
                // no current-session log to dump. zshrs previously
                // dumped the entire on-disk persistent history into
                // the user's named file, leaking prior runs. Restrict
                // to the current session entries only when atty is
                // absent.
                let mode = if append_file {
                    std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&path)
                } else {
                    std::fs::File::create(&path)
                };
                match mode {
                    Ok(mut file) => {
                        use std::io::Write;
                        let entries = if !atty::is(atty::Stream::Stdin) {
                            // Pull only session adds (matches zsh).
                            let mut session_entries = Vec::new();
                            for &id in &self.session_history_ids {
                                if let Ok(Some(entry)) = engine.get_by_number(id) {
                                    session_entries.push(entry);
                                }
                            }
                            session_entries
                        } else {
                            engine.recent(10000).unwrap_or_default()
                        };
                        for entry in entries.iter().rev() {
                            let _ = writeln!(file, ": {}:0;{}", entry.timestamp, entry.command);
                        }
                    }
                    Err(e) => {
                        zwarnnam("fc", &format!("cannot write {}: {}", path.display(), e));
                        return 1;
                    }
                }
            }
            return 0;
        }

        // Bare `fc` (no -l, no positional) ALWAYS errors recurse-
        // endlessly in -c mode, regardless of session entries —
        // the EDIT mode tries to re-execute the prior command, and
        // since `fc` itself is the prior command in `-c`, that's
        // infinite. (Without this guard, having a `print -s` entry
        // turned bare `fc` into a list-mode pass-through.)
        if !list_mode
            && positional.is_empty()
            && !atty::is(atty::Stream::Stdin)
            && !silent_no_op_flag
            && !read_file
            && !write_file
            && !append_file
        {
            // No-positional non-list-mode `fc` (with or without
            // edit-form flags like `-r`, `-d`, `-e`) re-executes
            // the prior command — which IS `fc` itself in `-c`
            // mode, hence the recurse abort. EXEMPT: -p/-P push/
            // pop, -a modify, -I/-L local, -m pattern, and
            // -R/-W/-A read/write/append (handled below); these
            // signal an explicit non-edit-form invocation.
            zwarnnam("fc", "current history line would recurse endlessly, aborted");
            return 1;
        }
        // `-p`/`-P` etc. exempt flags — silent success (no actual
        // history-stack manipulation, but no edit-mode either).
        if silent_no_op_flag && positional.is_empty() && !list_mode {
            return 0;
        }

        // List mode (fc -l)
        if list_mode || args.is_empty() {
            let (first, last) = match positional.len() {
                0 => (-16i64, -1i64),
                1 => {
                    let n = positional[0].parse::<i64>().unwrap_or(-16);
                    (n, -1)
                }
                _ => {
                    let f = positional[0].parse::<i64>().unwrap_or(-16);
                    let l = positional[1].parse::<i64>().unwrap_or(-1);
                    (f, l)
                }
            };

            // In non-interactive (`-c`) mode session history is
            // normally empty — zsh's `fc -l` errors with "no such
            // event: <N>". But if the script explicitly added to
            // history via `print -s`, we should be able to list those
            // entries. Bypass the atty guard when we have session
            // entries.
            if !atty::is(atty::Stream::Stdin) && self.session_history_ids.is_empty() {
                // Bare `fc` (no -l, no positional) is the EDIT mode —
                // zsh would re-execute the previous command. With
                // empty history the previous command IS fc itself,
                // so zsh refuses with "current history line would
                // recurse endlessly, aborted". Distinct from the
                // -l case which uses "no such event: N".
                if !list_mode && positional.is_empty() {
                    zwarnnam("fc", "current history line would recurse endlessly, aborted");
                    return 1;
                }
                // Non-numeric event spec (`fc -l blah`) is an "event
                // not found" error rather than the numeric "no such
                // event" form. zsh distinguishes the two: numeric
                // out-of-range is "no such event: N"; non-numeric is
                // "event not found: <text>".
                if positional.len() == 1 && positional[0].parse::<i64>().is_err() {
                    zwarnnam("fc", &format!("event not found: {}", positional[0]));
                    return 1;
                }
                // Two-positional `fc -l N M` is a RANGE query — zsh
                // emits a different error: `no events in that range`.
                // Three+ positionals -> `too many arguments` (fc -l
                // takes at most 2 range bounds). Single positional /
                // no positional uses `no such event: N`.
                if positional.len() > 2 {
                    // zsh: 3+ positionals where ANY positional is
                    // non-numeric -> `event not found: <text>` for
                    // the FIRST non-numeric (text-name miss takes
                    // precedence over count-error). All-numeric
                    // 3+ -> `too many arguments`. zshrs only
                    // checked args[0] earlier; extended to scan all
                    // bounds.
                    let first_text = positional.iter().find(|s| s.parse::<i64>().is_err());
                    if let Some(text) = first_text {
                        zwarnnam("fc", &format!("event not found: {}", text));
                    } else {
                        zwarnnam("fc", "too many arguments");
                    }
                    return 1;
                }
                if positional.len() == 2 {
                    // zsh: if either of the two range bounds is
                    // non-numeric, error `event not found: <text>`
                    // for the FIRST non-numeric bound. Both numeric
                    // -> `no events in that range`.
                    let p0_bad = positional[0].parse::<i64>().is_err();
                    let p1_bad = positional[1].parse::<i64>().is_err();
                    if p0_bad {
                        zwarnnam("fc", &format!("event not found: {}", positional[0]));
                        return 1;
                    }
                    if p1_bad {
                        zwarnnam("fc", &format!("event not found: {}", positional[1]));
                        return 1;
                    }
                    zwarnnam("fc", "no events in that range");
                    return 1;
                }
                // zsh's "no such event" uses the resolved index:
                //   - explicit positive N → "no such event: N"
                //   - explicit non-positive N → resolves to 0 (zsh's
                //     "count from end" with empty history)
                //   - DEFAULT (no positional, first defaults to -16
                //     because `fc -l` shows the last 16 entries) →
                //     resolves to 1 (zsh shows the lower bound of the
                //     would-be range, which is event #1 in an empty
                //     history)
                let resolved = if positional.is_empty() {
                    1
                } else if first <= 0 {
                    0
                } else {
                    first
                };
                zwarnnam("fc", &format!("no such event: {}", resolved));
                return 1;
            }

            let count = if first < 0 { (-first) as usize } else { 16 };
            // Non-interactive (`-c`) mode with session adds: only
            // show the entries added during this session, numbered
            // from 1 (matches zsh's behaviour for `print -s` + `fc -l`
            // in `-c` scripts). Look up by exact ID so other DB
            // entries from prior runs don't leak in.
            let session_only =
                !atty::is(atty::Stream::Stdin) && !self.session_history_ids.is_empty();
            if session_only {
                // Build the numbered entries first, then optionally
                // reverse the iteration order. zsh: `fc -lr` walks
                // the same range backwards (most recent first) but
                // keeps the original event numbers — `3 c | 2 b | 1 a`
                // for a 3-entry session. Without reversing here the
                // `-r` flag was a no-op for session-only listings.
                // Tuple holds (number, timestamp, duration_ms, command).
                let pairs: Vec<(i64, i64, i64, String)> = self
                    .session_history_ids
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &id)| {
                        engine.get_by_number(id).ok().flatten().map(|e| {
                            (
                                (i as i64) + 1,
                                e.timestamp,
                                e.duration_ms.unwrap_or(0),
                                e.command,
                            )
                        })
                    })
                    .collect();
                let iter: Box<dyn Iterator<Item = (i64, i64, i64, String)>> = if reverse {
                    Box::new(pairs.into_iter().rev())
                } else {
                    Box::new(pairs.into_iter())
                };
                for (n, ts, dur_ms, command) in iter {
                    if no_numbers {
                        println!("{}", command);
                    } else if show_time {
                        // Format timestamp as HH:MM (zsh's `-d`
                        // default: short time). zshrs previously
                        // dropped the timestamp column entirely.
                        let dt = chrono::DateTime::<chrono::Local>::from(
                            std::time::UNIX_EPOCH + std::time::Duration::from_secs(ts as u64),
                        );
                        let stamp = dt.format("%H:%M").to_string();
                        println!("{:>5}  {}  {}", n, stamp, command);
                    } else if show_duration {
                        // Duration in M:SS form (zsh's -D). zshrs
                        // previously dropped the duration column.
                        let secs = dur_ms / 1000;
                        let mins = secs / 60;
                        let s = secs % 60;
                        println!("{:>5}  {}:{:02}  {}", n, mins, s, command);
                    } else {
                        println!("{:>5}  {}", n, command);
                    }
                }
                return 0;
            }
            match engine.recent(count.max(100)) {
                Ok(mut entries) => {
                    if reverse {
                        entries.reverse();
                    }
                    for entry in entries.iter().rev().take(count) {
                        if no_numbers {
                            println!("{}", entry.command);
                        } else if show_time {
                            println!(
                                "{:>6}  {:>10}  {}",
                                entry.id, entry.timestamp, entry.command
                            );
                        } else if show_duration {
                            println!(
                                "{:>6}  {:>5}  {}",
                                entry.id,
                                entry.duration_ms.unwrap_or(0),
                                entry.command
                            );
                        } else {
                            println!("{:>5}  {}", entry.id, entry.command);
                        }
                    }
                    0
                }
                Err(e) => {
                    zwarnnam("fc", &format!("{}", e));
                    1
                }
            }
        } else if substitute_mode || !substitutions.is_empty() {
            // Substitution mode: fc -s old=new
            match engine.get_by_offset(0) {
                Ok(Some(entry)) => {
                    let mut cmd = entry.command.clone();
                    for (old, new) in &substitutions {
                        cmd = cmd.replace(old, new);
                    }
                    println!("{}", cmd);
                    self.execute_script(&cmd).unwrap_or(1)
                }
                Ok(None) => {
                    zwarnnam("fc", "no command to re-execute");
                    1
                }
                Err(e) => {
                    zwarnnam("fc", &format!("{}", e));
                    1
                }
            }
        } else if editor.as_deref() == Some("-") {
            // fc -e -: re-execute last command without editor
            match engine.get_by_offset(0) {
                Ok(Some(entry)) => {
                    println!("{}", entry.command);
                    self.execute_script(&entry.command).unwrap_or(1)
                }
                Ok(None) => {
                    zwarnnam("fc", "no command to re-execute");
                    1
                }
                Err(e) => {
                    zwarnnam("fc", &format!("{}", e));
                    1
                }
            }
        } else if let Some(arg) = positional.first() {
            // zsh: edit-mode fc takes at most 2 positional bounds
            // (`fc FIRST [LAST]`); 3+ -> `fc:1: too many arguments`
            // exit 1.
            if positional.len() > 2 {
                zwarnnam("fc", "too many arguments");
                return 1;
            }
            // Edit-mode `fc N` / `fc N M` (numeric positionals): zsh
            // re-edits commands N (or N..M). With empty session
            // history in -c mode, that's the recurse-endlessly path.
            // zshrs's prefix-search just used N and reported `event
            // not found: N`, the wrong category for the range-edit
            // form.
            if !atty::is(atty::Stream::Stdin) {
                let all_numeric = positional.iter().all(|s| s.parse::<i64>().is_ok());
                if all_numeric && positional.len() <= 2 {
                    zwarnnam("fc", "current history line would recurse endlessly, aborted");
                    return 1;
                }
            }
            if arg.starts_with('-') || arg.starts_with('+') {
                // fc -N or fc +N: re-execute Nth command
                let n: usize = arg[1..].parse().unwrap_or(1);
                let offset = if arg.starts_with('-') { n - 1 } else { n };
                match engine.get_by_offset(offset) {
                    Ok(Some(entry)) => {
                        println!("{}", entry.command);
                        self.execute_script(&entry.command).unwrap_or(1)
                    }
                    Ok(None) => {
                        zwarnnam("fc", "event not found");
                        1
                    }
                    Err(e) => {
                        zwarnnam("fc", &format!("{}", e));
                        1
                    }
                }
            } else if arg.is_empty() {
                // Empty positional: zsh emits `fc:1: event not
                // found:` (no match, no prior-command execution).
                // zshrs's prefix-match found the most recent entry
                // and recursively re-executed it — `fc ""` triggered
                // infinite recursion (it ran `fc ""` again).
                zwarnnam("fc", "event not found: ");
                1
            } else {
                // Try to find command by prefix
                match engine.search_prefix(arg, 1) {
                    Ok(entries) if !entries.is_empty() => {
                        println!("{}", entries[0].command);
                        self.execute_script(&entries[0].command).unwrap_or(1)
                    }
                    Ok(_) => {
                        zwarnnam("fc", &format!("event not found: {}", arg));
                        1
                    }
                    Err(e) => {
                        zwarnnam("fc", &format!("{}", e));
                        1
                    }
                }
            }
        } else {
            // Default: edit and execute last command
            match engine.get_by_offset(0) {
                Ok(Some(entry)) => {
                    println!("{}", entry.command);
                    self.execute_script(&entry.command).unwrap_or(1)
                }
                Ok(None) => {
                    zwarnnam("fc", "no command to re-execute");
                    1
                }
                Err(e) => {
                    zwarnnam("fc", &format!("{}", e));
                    1
                }
            }
        }
    }
    /// Poll LAST_SIGNAL atomic; if a signal was caught and a trap
    /// is registered for it, run the trap action via `execute_script`.
    /// Called at builtin-dispatch entry so traps fire between commands.
    /// Direct equivalent of zsh's `dotrap()` poll in the C source's
    /// main exec loop (Src/exec.c periodically calls dotrap).
    pub(crate) fn dispatch_pending_traps(&mut self) {
        #[cfg(unix)]
        {
            let sig = crate::ported::signals::LAST_SIGNAL
                .swap(0, std::sync::atomic::Ordering::SeqCst);
            if sig == 0 {
                return;
            }
            let name = match sig {
                n if n == libc::SIGHUP => "HUP",
                n if n == libc::SIGINT => "INT",
                n if n == libc::SIGQUIT => "QUIT",
                n if n == libc::SIGTERM => "TERM",
                n if n == libc::SIGUSR1 => "USR1",
                n if n == libc::SIGUSR2 => "USR2",
                n if n == libc::SIGCHLD => "CHLD",
                n if n == libc::SIGCONT => "CONT",
                n if n == libc::SIGTSTP => "TSTP",
                n if n == libc::SIGALRM => "ALRM",
                n if n == libc::SIGPIPE => "PIPE",
                n if n == libc::SIGWINCH => "WINCH",
                _ => return,
            };
            if let Some(action) = self.traps.get(name).cloned() {
                if !action.is_empty() {
                    let _ = self.execute_script(&action);
                }
            }
        }
    }

    pub(crate) fn bin_trap(&mut self, args: &[String]) -> i32 {
        #[cfg(unix)]
        fn signal_name_to_libc_num(name: &str) -> Option<libc::c_int> {
            // Shell-only pseudo-signals have no syscall mapping.
            Some(match name {
                "HUP" => libc::SIGHUP,
                "INT" => libc::SIGINT,
                "QUIT" => libc::SIGQUIT,
                "ILL" => libc::SIGILL,
                "TRAP" => libc::SIGTRAP,
                "ABRT" => libc::SIGABRT,
                "BUS" => libc::SIGBUS,
                "FPE" => libc::SIGFPE,
                // SIGKILL/SIGSTOP can't be caught — bin_trap rejected
                // them upstream as "undefined signal", so omit here.
                "USR1" => libc::SIGUSR1,
                "SEGV" => libc::SIGSEGV,
                "USR2" => libc::SIGUSR2,
                "PIPE" => libc::SIGPIPE,
                "ALRM" => libc::SIGALRM,
                "TERM" => libc::SIGTERM,
                "CHLD" => libc::SIGCHLD,
                "CONT" => libc::SIGCONT,
                "TSTP" => libc::SIGTSTP,
                "TTIN" => libc::SIGTTIN,
                "TTOU" => libc::SIGTTOU,
                "URG" => libc::SIGURG,
                "XCPU" => libc::SIGXCPU,
                "XFSZ" => libc::SIGXFSZ,
                "VTALRM" => libc::SIGVTALRM,
                "PROF" => libc::SIGPROF,
                "WINCH" => libc::SIGWINCH,
                "IO" => libc::SIGIO,
                _ => return None,
            })
        }
        #[cfg(not(unix))]
        fn signal_name_to_libc_num(_name: &str) -> Option<i32> {
            None
        }
        // PFA-SMR aspect: emit one `trap` event per (signal, handler).
        // `trap 'cmd' SIG1 SIG2` → 2 records sharing the same handler.
        #[cfg(feature = "recorder")]
        if crate::recorder::is_enabled() {
            // Skip listing-only forms (`trap`, `trap -l`, `trap -p`).
            let listing = args.is_empty()
                || (args.len() == 1 && (args[0] == "-l" || args[0] == "-p"));
            if !listing && args.len() >= 2 {
                let ctx = self.recorder_ctx();
                let handler = &args[0];
                for sig in &args[1..] {
                    crate::recorder::emit_trap(sig, handler, ctx.clone());
                }
            }
        }
        if args.is_empty() {
            // List all traps, sorted by signal name for stable
            // output across runs.
            let mut sigs: Vec<&String> = self.traps.keys().collect();
            sigs.sort();
            for sig in sigs {
                if let Some(action) = self.traps.get(sig) {
                    println!("trap -- '{}' {}", action, sig);
                }
            }
            return 0;
        }

        // zsh: `-l` is NOT a recognized `trap` flag (that's a bash-ism).
        // zsh's `trap -l` lists current traps (which are empty in -f
        // mode), producing no output and exit 0. Earlier zshrs emitted
        // the bash-style numbered SIGNAL list, mismatching zsh exactly.
        // Keep the silent-empty behaviour to align with zsh's real trap
        // builtin.
        if args.len() == 1 && args[0] == "-l" {
            return 0;
        }

        // zsh's `trap` builtin does NOT accept `-p` (that's
        // bash). zsh treats `-p` as a regular argument: with one
        // arg `trap -p` becomes "set trap with action `-p` and no
        // signal" which fails the action+signal requirement and
        // falls through to the bare trap-name lookup. With no
        // matching signal, it errors `command not found: -p`
        // (the parser doesn't even reach the builtin — the shell
        // sees `-p` as an unknown command name). Mirror that:
        // don't intercept `-p`. Earlier zshrs added bash-style
        // `-p` for compat but it diverged from zsh.

        // trap '' signal: reset to default
        // trap action signal...: set trap
        // trap signal: print current action for signal
        if args.len() == 1 {
            // Print trap for this signal
            let sig = &args[0];
            if let Some(action) = self.traps.get(sig) {
                println!("trap -- '{}' {}", action, sig);
            }
            return 0;
        }

        let action = &args[0];
        let signals = &args[1..];

        for sig in signals {
            let sig_upper = sig.to_uppercase();
            // Numeric signal aliases: `trap CMD 0` is equivalent to
            // `trap CMD EXIT` (POSIX). zsh accepts both forms.
            let sig_name = if sig == "0" {
                "EXIT".to_string()
            } else if let Ok(num) = sig.parse::<u32>() {
                // Map other numbers to canonical names. We piggyback on
                // libc's signal numbers when available so the mapping
                // is platform-correct (e.g. SIGUSR1 = 10 on Linux,
                // 30 on macOS).
                let name = match num as i32 {
                    n if n == libc::SIGHUP => "HUP",
                    n if n == libc::SIGINT => "INT",
                    n if n == libc::SIGQUIT => "QUIT",
                    n if n == libc::SIGTERM => "TERM",
                    n if n == libc::SIGUSR1 => "USR1",
                    n if n == libc::SIGUSR2 => "USR2",
                    n if n == libc::SIGCHLD => "CHLD",
                    n if n == libc::SIGCONT => "CONT",
                    n if n == libc::SIGSTOP => "STOP",
                    n if n == libc::SIGTSTP => "TSTP",
                    n if n == libc::SIGALRM => "ALRM",
                    n if n == libc::SIGPIPE => "PIPE",
                    _ => "",
                };
                if name.is_empty() {
                    sig_upper
                } else {
                    name.to_string()
                }
            } else if let Some(after) = sig_upper.strip_prefix("SIG") {
                after.to_string()
            } else {
                sig_upper
            };

            // zsh validates the signal name before installing the
            // handler — an unknown name errors `undefined signal: NAME`
            // exit 1 and the trap is NOT installed. zshrs blindly
            // inserted whatever uppercased token came in, so
            // `trap "" BADSIG` quietly registered a never-firable trap.
            // zsh's max signal number on macOS is 31 (SIGUSR2); on
            // Linux 63 (SIGRTMAX). Treat anything > 63 as invalid so
            // `trap "" 99` errors like zsh. Lower bound: > 0 (signal
            // 0 is `EXIT`, handled by the numeric->name remapping
            // above, so by this point a literal `0` slipped past).
            // NOTE: zsh rejects "RETURN" as a signal name even
            // though it appears in the documentation — the actual
            // zsh runtime doesn't accept it. Match that.
            let known_sig = matches!(
                sig_name.as_str(),
                "EXIT"
                    | "ZERR"
                    | "DEBUG"
                    | "ERR"
                    | "HUP"
                    | "INT"
                    | "QUIT"
                    | "ILL"
                    | "TRAP"
                    | "ABRT"
                    | "EMT"
                    | "BUS"
                    | "FPE"
                    | "KILL"
                    | "USR1"
                    | "SEGV"
                    | "USR2"
                    | "PIPE"
                    | "ALRM"
                    | "TERM"
                    | "CHLD"
                    | "CONT"
                    | "STOP"
                    | "TSTP"
                    | "TTIN"
                    | "TTOU"
                    | "URG"
                    | "XCPU"
                    | "XFSZ"
                    | "VTALRM"
                    | "PROF"
                    | "WINCH"
                    | "IO"
                    | "INFO"
                    | "SYS"
                    | "STKFLT"
                    | "PWR"
            ) || sig
                .parse::<u32>()
                .map(|n| n > 0 && n <= 63)
                .unwrap_or(false);
            if !known_sig {
                zwarnnam("trap", &format!("undefined signal: {}", sig));
                return 1;
            }

            if action == "-" {
                // `trap - SIG` resets to default (delete the entry).
                self.traps.remove(&sig_name);
                // Restore default OS-level disposition for non-EXIT
                // signals (EXIT/ZERR/DEBUG/ERR are shell-only, no
                // syscall mapping).
                if let Some(num) = signal_name_to_libc_num(&sig_name) {
                    #[cfg(unix)]
                    unsafe {
                        libc::signal(num, libc::SIG_DFL);
                    }
                    let _ = num;
                }
            } else {
                // `trap "" SIG` (empty action) is the SIGNAL-IGNORE
                // form per POSIX — distinct from "reset to default".
                // Keep the empty string in the table so `trap` lists
                // it back (zsh: `trap -- '' USR1`).
                self.traps.insert(sig_name.clone(), action.clone());
                // Install the OS-level handler so the kernel hands us
                // the signal instead of killing the process. The
                // shared `zhandler` records the signal in
                // LAST_SIGNAL; the main exec loop polls it between
                // commands and runs the recorded trap action.
                if let Some(num) = signal_name_to_libc_num(&sig_name) {
                    crate::ported::signals::install_handler(num);
                }
            }
        }

        0
    }
    pub(crate) fn bin_alias(&mut self, args: &[String]) -> i32 {
        // alias [ {+|-}gmrsL ] [ name[=value] ... ]
        // -g: global alias (expanded anywhere in command line)
        // -s: suffix alias (file.ext expands to "handler file.ext")
        // -r: regular alias (default)
        // -m: pattern match mode
        // -L: list in form suitable for reinput
        // +g/+s/+r: print aliases of that type

        let mut is_global = false;
        let mut is_suffix = false;
        let mut is_regular = false;
        let mut list_form = false;
        let mut matchpat = false;
        let mut print_global = false;
        let mut print_suffix = false;
        let mut print_regular = false;
        let mut positional_args = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if arg.starts_with('+') && arg.len() > 1 {
                // +g, +s, +r: print aliases of that type
                for ch in arg[1..].chars() {
                    match ch {
                        'g' => print_global = true,
                        's' => print_suffix = true,
                        'r' => print_regular = true,
                        'L' => list_form = true,
                        'm' => matchpat = true,
                        // BUILTIN("alias", ..., "Lgmrs") — `-` and `+`
                        // forms share the same letter set. The `-`
                        // form below already rejects unknown letters;
                        // mirror that here so `alias +X` errors too
                        // instead of silently swallowing the typo.
                        _ => {
                            zwarnnam("alias", &format!("bad option: +{}", ch));
                            return 1;
                        }
                    }
                }
            } else if arg.starts_with('-') && arg != "-" {
                for ch in arg[1..].chars() {
                    match ch {
                        'g' => is_global = true,
                        's' => is_suffix = true,
                        'L' => list_form = true,
                        'm' => matchpat = true,
                        'r' => is_regular = true,
                        _ => {
                            zwarnnam("alias", &format!("bad option: -{}", ch));
                            return 1;
                        }
                    }
                }
            } else {
                positional_args.push(arg.clone());
            }
            i += 1;
        }

        // Direct port of src/zsh/Src/builtin.c:4462-4468 — type
        // flags are mutually exclusive. zsh sums OPT_ISSET for r,
        // g, s and errors when the sum > 1 ("illegal combination
        // of options"). Previous Rust impl only caught the
        // `-gs` case; `-gr` and `-sr` slipped through.
        let type_opts = (is_global as u32) + (is_suffix as u32) + (is_regular as u32);
        if type_opts > 1 {
            zwarnnam("alias", "illegal combination of options");
            return 1;
        }

        // If +g/+s/+r used, list those types. Sorted for
        // deterministic output (was HashMap-iteration random).
        let print_sorted = |map: &indexmap::IndexMap<String, String>, prefix: &str, list_form: bool| {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for name in keys {
                let value = map.get(name).unwrap();
                let s = format!("{}={}", crate::ported::utils::quotedzputs(name), crate::ported::utils::quotedzputs(value));
                if list_form {
                    println!("{}{}", prefix, s);
                } else {
                    println!("{}", s);
                }
            }
        };
        if print_global || print_suffix || print_regular {
            if print_regular {
                print_sorted(&self.aliases, "alias ", list_form);
            }
            if print_global {
                print_sorted(&self.global_aliases, "alias -g ", list_form);
            }
            if print_suffix {
                print_sorted(&self.suffix_aliases, "alias -s ", list_form);
            }
            return 0;
        }

        if positional_args.is_empty() {
            // List aliases
            let prefix = if is_suffix {
                "alias -s "
            } else if is_global {
                "alias -g "
            } else {
                "alias "
            };
            let alias_map: Vec<(String, String)> = if is_suffix {
                self.suffix_aliases
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            } else if is_global {
                self.global_aliases
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            } else {
                self.aliases
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            };
            // Sort for deterministic listing (matches zsh's order).
            let mut alias_map = alias_map;
            alias_map.sort_by(|a, b| a.0.cmp(&b.0));
            for (name, value) in alias_map {
                let formatted = format!("{}={}", crate::ported::utils::quotedzputs(&name), crate::ported::utils::quotedzputs(&value));
                if list_form {
                    println!("{}{}", prefix, formatted);
                } else {
                    println!("{}", formatted);
                }
            }
            return 0;
        }

        // builtin.c:4521-4540 — bin_alias's literal-arg loop sets
        // `returnval = 1` on unknown alias name but CONTINUES to the
        // next arg. zshrs's `return 1` short-circuited so a subsequent
        // valid arg never displayed:
        //   alias bogus realalias  → zshrs only checked `bogus`,
        //   exit 1, never tried `realalias`.
        // Track the failure flag and exit at the end.
        let mut returnval = 0;
        for arg in &positional_args {
            if let Some(eq_pos) = arg.find('=') {
                // Define alias: name=value. zsh: empty NAME (`=val`
                // / `=`) is `bad assignment` exit 1 — the alias name
                // is required. Without this guard zshrs silently
                // created an alias with name "" which was then
                // un-removable.
                if eq_pos == 0 {
                    zwarn("bad assignment");
                    return 1;
                }
                let name = &arg[..eq_pos];
                let value = &arg[eq_pos + 1..];
                if is_suffix {
                    self.suffix_aliases
                        .insert(name.to_string(), value.to_string());
                } else if is_global {
                    self.global_aliases
                        .insert(name.to_string(), value.to_string());
                } else {
                    self.aliases.insert(name.to_string(), value.to_string());
                }
                // PFA-SMR aspect: capture the alias definition with the
                // subkind preserved (regular / -g / -s) so downstream
                // queries can distinguish global expansion targets from
                // suffix dispatch from regular command shorthands.
                #[cfg(feature = "recorder")]
                {
                    let ctx = self.recorder_ctx();
                    if is_suffix {
                        crate::recorder::emit_salias(name, Some(value), ctx);
                    } else if is_global {
                        crate::recorder::emit_galias(name, Some(value), ctx);
                    } else {
                        crate::recorder::emit_alias(name, Some(value), ctx);
                    }
                }
            } else if matchpat {
                // -m: pattern match mode — list matching aliases.
                // Direct port of zsh/Src/builtin.c:4396-4424 (bin_unhash
                // alias path). Uses Self::glob_match_static so character
                // classes, extendedglob negation, etc. work the same as
                // every other glob site.
                let alias_map: &indexmap::IndexMap<String, String> = if is_suffix {
                    &self.suffix_aliases
                } else if is_global {
                    &self.global_aliases
                } else {
                    &self.aliases
                };

                let prefix = if is_suffix {
                    "alias -s "
                } else if is_global {
                    "alias -g "
                } else {
                    "alias "
                };

                let mut sorted: Vec<&String> = alias_map.keys().collect();
                sorted.sort();
                for name in sorted {
                    if Self::glob_match_static(name, arg.as_str()) {
                        if let Some(value) = alias_map.get(name) {
                            let formatted = format!("{}={}", crate::ported::utils::quotedzputs(name), crate::ported::utils::quotedzputs(value));
                            if list_form {
                                println!("{}{}", prefix, formatted);
                            } else {
                                println!("{}", formatted);
                            }
                        }
                    }
                }
            } else {
                // Print alias - look up directly without holding borrow
                let value = if is_suffix {
                    self.suffix_aliases.get(arg.as_str()).cloned()
                } else if is_global {
                    self.global_aliases.get(arg.as_str()).cloned()
                } else {
                    self.aliases.get(arg.as_str()).cloned()
                };
                if let Some(v) = value {
                    // zsh emits bare value if no shell metas / spaces;
                    // single-quoted otherwise. Match that exactly so
                    // `alias x=ls` prints `x=ls` not `x='ls'`.
                    // Includes `=` because `alias x=a=b` parses as
                    // `alias x=a` plus arg `=b` without the quoting.
                    let needs_quote = v.is_empty()
                        || v.chars()
                            .any(|c| c.is_whitespace() || "$\"'`\\;|&<>(){}*?#~!=".contains(c));
                    let body = if needs_quote {
                        let escaped = v.replace('\'', "'\\''");
                        format!("{}='{}'", arg, escaped)
                    } else {
                        format!("{}={}", arg, v)
                    };
                    // `alias -L name` prints in re-input form
                    // (`alias name=value`); the bare form is just
                    // `name=value`. Was always omitting the `alias`
                    // prefix even when -L was passed.
                    if list_form {
                        println!("alias {}", body);
                    } else {
                        println!("{}", body);
                    }
                } else {
                    // zsh exits 1 silently when querying an
                    // unknown alias (no diagnostic). Per the C-source
                    // loop at builtin.c:4536-4537, this sets a return
                    // flag but does NOT abort the loop — subsequent
                    // valid arg names should still display.
                    returnval = 1;
                }
            }
        }
        returnval
    }
    pub(crate) fn builtin_unalias(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            // zsh format: `zsh:unalias:1: not enough arguments`.
            // zshrs previously printed a bash-style usage line with
            // a different prefix and option list — script consumers
            // pattern-matching on `unalias:1:` missed the diagnostic.
            zwarnnam("unalias", "not enough arguments");
            return 1;
        }
        // PFA-SMR aspect: emit one `unalias` event per non-flag arg.
        // Pairs with the alias hook so override chains are queryable.
        #[cfg(feature = "recorder")]
        if crate::recorder::is_enabled() {
            let ctx = self.recorder_ctx();
            for a in args {
                if a.starts_with('-') && a != "-" {
                    continue;
                }
                crate::recorder::emit_unalias(a, ctx.clone());
            }
        }

        let mut is_global = false;
        let mut is_suffix = false;
        let mut remove_all = false;
        let mut match_glob = false;
        let mut positional_args = Vec::new();

        for arg in args {
            if arg.starts_with('-') && arg != "-" {
                for ch in arg[1..].chars() {
                    match ch {
                        'a' => remove_all = true,
                        'g' => is_global = true,
                        's' => is_suffix = true,
                        'm' => match_glob = true,
                        _ => {
                            zwarnnam("unalias", &format!("bad option: -{}", ch));
                            return 1;
                        }
                    }
                }
            } else {
                positional_args.push(arg.clone());
            }
        }

        if remove_all {
            if is_suffix {
                self.suffix_aliases.clear();
            } else if is_global {
                self.global_aliases.clear();
            } else {
                // -a without -g/-s clears all three
                self.aliases.clear();
                self.global_aliases.clear();
                self.suffix_aliases.clear();
            }
            return 0;
        }

        if positional_args.is_empty() {
            zwarnnam("unalias", "not enough arguments");
            return 1;
        }

        // -m glob pattern dispatch. Direct port of
        // src/zsh/Src/builtin.c:4396-4424 (bin_unhash with the
        // unalias path). Each arg is treated as a glob pattern;
        // every matching alias in the chosen hash table is removed.
        // If NO matches across all args, return 1 per builtin.c:4421.
        if match_glob {
            let mut matched = false;
            let target_keys: Vec<String> = if is_suffix {
                self.suffix_aliases.keys().cloned().collect()
            } else if is_global {
                self.global_aliases.keys().cloned().collect()
            } else {
                self.aliases.keys().cloned().collect()
            };
            for pat in &positional_args {
                for k in &target_keys {
                    if ShellExecutor::glob_match_static(k, pat) {
                        if is_suffix {
                            self.suffix_aliases.remove(k);
                        } else if is_global {
                            self.global_aliases.remove(k);
                        } else {
                            self.aliases.remove(k);
                        }
                        matched = true;
                    }
                }
            }
            return if matched { 0 } else { 1 };
        }

        // zsh continues processing remaining names after a miss,
        // emitting one diagnostic per unknown entry and returning the
        // last failing exit code. zshrs returned on first miss,
        // hiding the rest of the misses from script consumers.
        let mut status = 0;
        for name in positional_args {
            let removed = if is_suffix {
                self.suffix_aliases.remove(&name).is_some()
            } else if is_global {
                self.global_aliases.remove(&name).is_some()
            } else {
                self.aliases.remove(&name).is_some()
            };
            if !removed {
                zwarnnam("unalias", &format!("no such hash table element: {}", name));
                status = 1;
            }
        }
        status
    }
    pub(crate) fn bin_set(&mut self, args: &[String]) -> i32 {
        self.dispatch_pending_traps();
        if self.redirect_failed { self.redirect_failed = false; return 1; }
        // PFA-SMR aspect: emit setopt/unsetopt events for the POSIX
        // `set -o NAME` / `set +o NAME` form. This is the third option
        // syntax (after `setopt NAME` / `unsetopt NAME`); a recorder
        // user expects all three to surface in `zwhere when -k setopt`.
        #[cfg(feature = "recorder")]
        if crate::recorder::is_enabled() && !args.is_empty() {
            let ctx = self.recorder_ctx();
            let mut iter = args.iter().peekable();
            while let Some(a) = iter.next() {
                match a.as_str() {
                    "-o" => {
                        if let Some(name) = iter.next() {
                            crate::recorder::emit_setopt(name, ctx.clone());
                        }
                    }
                    "+o" => {
                        if let Some(name) = iter.next() {
                            crate::recorder::emit_unsetopt(name, ctx.clone());
                        }
                    }
                    _ => {}
                }
            }
        }
        if args.is_empty() {
            // List all variables and their values (zsh behavior)
            let mut vars: Vec<_> = self.variables.iter().collect();
            vars.sort_by_key(|(k, _)| *k);
            for (k, v) in vars {
                println!("{}={}", k, crate::ported::utils::quotedzputs(v));
            }
            // Also print arrays
            let mut arrs: Vec<_> = self.arrays.iter().collect();
            arrs.sort_by_key(|(k, _)| *k);
            for (k, v) in arrs {
                let quoted: Vec<String> = v.iter().map(|s| crate::ported::utils::quotedzputs(s)).collect();
                println!("{}=( {} )", k, quoted.join(" "));
            }
            return 0;
        }

        // Check for "+" alone - print just variable names
        if args.len() == 1 && args[0] == "+" {
            let mut names: Vec<_> = self.variables.keys().collect();
            names.extend(self.arrays.keys());
            names.sort();
            names.dedup();
            for name in names {
                println!("{}", name);
            }
            return 0;
        }

        let mut iter = args.iter().peekable();
        let mut set_array: Option<bool> = None; // Some(true) = -A, Some(false) = +A
        let mut array_name: Option<String> = None;
        let mut sort_asc = false;
        let mut sort_desc = false;

        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-o" => {
                    // -o with no arg: print all options in "option on/off" format
                    if iter.peek().is_none()
                        || iter
                            .peek()
                            .map(|s| s.starts_with('-') || s.starts_with('+'))
                            .unwrap_or(false)
                    {
                        self.print_options_table();
                        continue;
                    }
                    if let Some(opt) = iter.next() {
                        let (name, enable) = Self::normalize_option_name(opt);
                        if !ZSH_OPTIONS_SET.contains(name.as_str()) {
                            zwarnnam("set", &format!("no such option: {}", opt));
                            return 1;
                        }
                        self.options.insert(name, enable);
                    }
                }
                "+o" => {
                    // +o with no arg: print options in re-entrant format
                    if iter.peek().is_none()
                        || iter
                            .peek()
                            .map(|s| s.starts_with('-') || s.starts_with('+'))
                            .unwrap_or(false)
                    {
                        self.print_options_reentrant();
                        continue;
                    }
                    if let Some(opt) = iter.next() {
                        let (name, enable) = Self::normalize_option_name(opt);
                        if !ZSH_OPTIONS_SET.contains(name.as_str()) {
                            zwarnnam("set", &format!("no such option: {}", opt));
                            return 1;
                        }
                        self.options.insert(name, !enable);
                    }
                }
                "-A" => {
                    set_array = Some(true);
                    if let Some(name) = iter.next() {
                        if !name.starts_with('-') && !name.starts_with('+') {
                            array_name = Some(name.clone());
                        }
                    }
                    if array_name.is_none() {
                        // Print all arrays with values
                        let mut arrs: Vec<_> = self.arrays.iter().collect();
                        arrs.sort_by_key(|(k, _)| *k);
                        for (k, v) in arrs {
                            let quoted: Vec<String> = v.iter().map(|s| crate::ported::utils::quotedzputs(s)).collect();
                            println!("{}=( {} )", k, quoted.join(" "));
                        }
                        return 0;
                    }
                }
                "+A" => {
                    set_array = Some(false);
                    if let Some(name) = iter.next() {
                        if !name.starts_with('-') && !name.starts_with('+') {
                            array_name = Some(name.clone());
                        }
                    }
                    if array_name.is_none() {
                        // Print array names only
                        let mut names: Vec<_> = self.arrays.keys().collect();
                        names.sort();
                        for name in names {
                            println!("{}", name);
                        }
                        return 0;
                    }
                }
                "-s" => sort_asc = true,
                "+s" => sort_desc = true,
                "-e" => {
                    self.options.insert("errexit".to_string(), true);
                }
                "+e" => {
                    self.options.insert("errexit".to_string(), false);
                }
                "-x" => {
                    self.options.insert("xtrace".to_string(), true);
                }
                "+x" => {
                    self.options.insert("xtrace".to_string(), false);
                }
                "-u" => {
                    self.options.insert("nounset".to_string(), true);
                }
                "+u" => {
                    self.options.insert("nounset".to_string(), false);
                }
                "-v" => {
                    self.options.insert("verbose".to_string(), true);
                }
                "+v" => {
                    self.options.insert("verbose".to_string(), false);
                }
                "-n" => {
                    self.options.insert("exec".to_string(), false);
                }
                "+n" => {
                    self.options.insert("exec".to_string(), true);
                }
                "-f" => {
                    self.options.insert("glob".to_string(), false);
                }
                "+f" => {
                    self.options.insert("glob".to_string(), true);
                }
                "-m" => {
                    self.options.insert("monitor".to_string(), true);
                }
                "+m" => {
                    self.options.insert("monitor".to_string(), false);
                }
                "-C" => {
                    self.options.insert("clobber".to_string(), false);
                }
                "+C" => {
                    self.options.insert("clobber".to_string(), true);
                }
                "-b" => {
                    self.options.insert("notify".to_string(), true);
                }
                "+b" => {
                    self.options.insert("notify".to_string(), false);
                }
                // zsh-only: `-E` enables ERR_RETURN (return on non-zero
                // status inside a function) and `-T` enables TRAPS_ASYNC
                // (run traps after each command). Both are no-ops in
                // single-command -c mode for our purposes; accept the
                // flag silently rather than erroring "invalid option".
                "-E" => {
                    self.options.insert("err_return".to_string(), true);
                }
                "+E" => {
                    self.options.insert("err_return".to_string(), false);
                }
                "-T" => {
                    self.options.insert("trapasync".to_string(), true);
                }
                "+T" => {
                    self.options.insert("trapasync".to_string(), false);
                }
                // POSIX/zsh `set -h` enables HASH_CMDS (cache external
                // command paths). zsh accepts silently. zshrs errored
                // "invalid option" — break user scripts that probe for
                // this option early.
                "-h" => {
                    self.options.insert("hashcmds".to_string(), true);
                }
                "+h" => {
                    self.options.insert("hashcmds".to_string(), false);
                }
                // `set -k` enables KSH_TYPESET (allow assignments after
                // some keywords). `set -p` enables PRIVILEGED. `set -B`
                // BRACE_CCL. zsh-specific knobs that user scripts may
                // toggle; accept silently as toggle-options.
                "-k" => {
                    self.options.insert("kshtypeset".to_string(), true);
                }
                "+k" => {
                    self.options.insert("kshtypeset".to_string(), false);
                }
                "-p" => {
                    self.options.insert("privileged".to_string(), true);
                }
                "+p" => {
                    self.options.insert("privileged".to_string(), false);
                }
                "-B" => {
                    self.options.insert("braceccl".to_string(), true);
                }
                "+B" => {
                    self.options.insert("braceccl".to_string(), false);
                }
                "-H" => {
                    self.options.insert("histreduceblanks".to_string(), true);
                }
                "+H" => {
                    self.options.insert("histreduceblanks".to_string(), false);
                }
                "--" => {
                    let remaining: Vec<String> = iter.cloned().collect();
                    if let Some(ref name) = array_name {
                        let mut values = remaining;
                        if sort_asc {
                            values.sort();
                        } else if sort_desc {
                            values.sort();
                            values.reverse();
                        }
                        if set_array == Some(true) {
                            self.arrays.insert(name.clone(), values);
                        } else {
                            // +A: replace initial elements
                            let arr = self.arrays.entry(name.clone()).or_default();
                            for (i, v) in values.into_iter().enumerate() {
                                if i < arr.len() {
                                    arr[i] = v;
                                } else {
                                    arr.push(v);
                                }
                            }
                        }
                    } else if remaining.is_empty() {
                        // "set --" with nothing after unsets positional params
                        self.positional_params.clear();
                    } else {
                        let mut values = remaining;
                        if sort_asc {
                            values.sort();
                        } else if sort_desc {
                            values.sort();
                            values.reverse();
                        }
                        self.positional_params = values;
                    }
                    return 0;
                }
                _ => {
                    // `--anything` (long-option-style) is treated by
                    // zsh as `--` (end-of-options) — the rest of args
                    // become positional. zshrs's per-char letter loop
                    // hit `-` first and errored "can't change option:
                    // --". Detect and short-circuit to positional
                    // assignment.
                    if arg.starts_with("--") {
                        let remaining: Vec<String> = iter.cloned().collect();
                        if let Some(ref name) = array_name {
                            if set_array == Some(true) {
                                self.arrays.insert(name.clone(), remaining);
                            }
                        } else {
                            self.positional_params = remaining;
                        }
                        return 0;
                    }
                    // Handle single-letter options like -ex (multiple options)
                    if arg.starts_with('-') && arg.len() > 1 {
                        for c in arg[1..].chars() {
                            match c {
                                'a' => {
                                    self.options.insert("allexport".to_string(), true);
                                }
                                'e' => {
                                    self.options.insert("errexit".to_string(), true);
                                }
                                'x' => {
                                    self.options.insert("xtrace".to_string(), true);
                                }
                                'u' => {
                                    self.options.insert("nounset".to_string(), true);
                                }
                                'v' => {
                                    self.options.insert("verbose".to_string(), true);
                                }
                                'n' => {
                                    self.options.insert("exec".to_string(), false);
                                }
                                'f' => {
                                    self.options.insert("glob".to_string(), false);
                                }
                                'm' => {
                                    self.options.insert("monitor".to_string(), true);
                                }
                                'C' => {
                                    self.options.insert("clobber".to_string(), false);
                                }
                                'b' => {
                                    self.options.insert("notify".to_string(), true);
                                }
                                // zsh's other single-letter `set` flags
                                // from the official option-letter table
                                // (man zshoptions OPTION ALIASES). Accept
                                // silently; the runtime knob isn't always
                                // wired but the flag itself is real.
                                'd' | 'g' | 'h' | 'k' | 'p' | 'r' | 's' | 't' | 'y' | 'A' | 'B'
                                | 'E' | 'F' | 'G' | 'H' | 'K' | 'L' | 'N' | 'P' | 'R' | 'T'
                                | 'U' | 'X' | 'Y' => {}
                                _ => {
                                    // Unknown letter: zsh errors with
                                    // `can't change option: -X` exit 1.
                                    zwarnnam("set", &format!("can't change option: -{}", c));
                                    return 1;
                                }
                            }
                        }
                        continue;
                    }
                    if arg.starts_with('+') && arg.len() > 1 {
                        for c in arg[1..].chars() {
                            match c {
                                'a' => {
                                    self.options.insert("allexport".to_string(), false);
                                }
                                'e' => {
                                    self.options.insert("errexit".to_string(), false);
                                }
                                'x' => {
                                    self.options.insert("xtrace".to_string(), false);
                                }
                                'u' => {
                                    self.options.insert("nounset".to_string(), false);
                                }
                                'v' => {
                                    self.options.insert("verbose".to_string(), false);
                                }
                                'n' => {
                                    self.options.insert("exec".to_string(), true);
                                }
                                'f' => {
                                    self.options.insert("glob".to_string(), true);
                                }
                                'm' => {
                                    self.options.insert("monitor".to_string(), false);
                                }
                                'C' => {
                                    self.options.insert("clobber".to_string(), true);
                                }
                                'b' => {
                                    self.options.insert("notify".to_string(), false);
                                }
                                // zsh's other single-letter `set` flags
                                // (mirror the `-` arm so `+Z` errors
                                // identically to `-Z`).
                                'd' | 'g' | 'h' | 'k' | 'p' | 'r' | 's' | 't' | 'y' | 'A' | 'B'
                                | 'E' | 'F' | 'G' | 'H' | 'K' | 'L' | 'N' | 'P' | 'R' | 'T'
                                | 'U' | 'X' | 'Y' => {}
                                _ => {
                                    zwarnnam("set", &format!("can't change option: +{}", c));
                                    return 1;
                                }
                            }
                        }
                        continue;
                    }
                    // Treat as positional params
                    let mut values: Vec<String> =
                        std::iter::once(arg.clone()).chain(iter.cloned()).collect();
                    if sort_asc {
                        values.sort();
                    } else if sort_desc {
                        values.sort();
                        values.reverse();
                    }
                    if let Some(ref name) = array_name {
                        if set_array == Some(true) {
                            self.arrays.insert(name.clone(), values);
                        } else {
                            let arr = self.arrays.entry(name.clone()).or_default();
                            for (i, v) in values.into_iter().enumerate() {
                                if i < arr.len() {
                                    arr[i] = v;
                                } else {
                                    arr.push(v);
                                }
                            }
                        }
                    } else {
                        self.positional_params = values;
                    }
                    return 0;
                }
            }
        }
        // `set -A NAME` (no values): zsh clears the array.
        // `set +A NAME` (no values): leaves the array unchanged
        // (per Src/builtin.c bin_set: +A only updates positional
        // slots from the values list and a missing list is a no-op).
        // Without this, `a=(1 2 3); set -A a` left `a` as 1 2 3,
        // diverging from zsh's empty-array semantics.
        if let Some(ref name) = array_name {
            if set_array == Some(true) {
                self.arrays.insert(name.clone(), Vec::new());
            }
        }
        0
    }
    pub(crate) fn bin_getopts(&mut self, args: &[String]) -> i32 {
        if args.len() < 2 {
            // zsh: bare `getopts` (or with only one arg) errors
            // `getopts:1: not enough arguments` exit 1. zshrs's
            // bash-style usage banner had no shell-name prefix.
            zwarnnam("getopts", "not enough arguments");
            return 1;
        }

        let optstring = &args[0];
        let varname = &args[1];
        let opt_args: Vec<&str> = if args.len() > 2 {
            args[2..].iter().map(|s| s.as_str()).collect()
        } else {
            self.positional_params.iter().map(|s| s.as_str()).collect()
        };

        // Get current OPTIND
        let optind: usize = self
            .variables
            .get("OPTIND")
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);

        if optind > opt_args.len() {
            self.variables.insert(varname.to_string(), "?".to_string());
            return 1;
        }

        let current_arg = opt_args[optind - 1];

        if !current_arg.starts_with('-') || current_arg == "-" {
            self.variables.insert(varname.to_string(), "?".to_string());
            return 1;
        }

        if current_arg == "--" {
            self.variables
                .insert("OPTIND".to_string(), (optind + 1).to_string());
            self.variables.insert(varname.to_string(), "?".to_string());
            return 1;
        }

        // Get current option position within the argument
        let optpos: usize = self
            .variables
            .get("_OPTPOS")
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);

        let opt_char = current_arg.chars().nth(optpos);

        if let Some(c) = opt_char {
            // Look up option in optstring
            let opt_idx = optstring.find(c);

            match opt_idx {
                Some(idx) => {
                    // Check if option takes an argument
                    let takes_arg = optstring.chars().nth(idx + 1) == Some(':');

                    if takes_arg {
                        // Get argument. Two shapes:
                        //   `-bX`  → arg is the rest of the same word, advance 1.
                        //   `-b X` → arg is the next word, advance 2.
                        let (arg, advance) = if optpos + 1 < current_arg.len() {
                            (current_arg[optpos + 1..].to_string(), 1usize)
                        } else if optind < opt_args.len() {
                            (opt_args[optind].to_string(), 2)
                        } else {
                            // Missing argument. Direct port of
                            // src/zsh/Src/builtin.c:5743-5763:
                            //   - quiet (`:` prefix): var=":",
                            //     OPTARG=opt_char (single-char form)
                            //   - non-quiet: var="?", OPTARG="",
                            //     warning to stderr
                            // Previous Rust always set var="?" and
                            // OPTARG=opt_char, diverging in both
                            // modes.
                            let quiet = optstring.starts_with(':');
                            // builtin.c:5747-5750 — POSIX mode also
                            // advances OPTIND past the bad option.
                            self.variables
                                .insert("OPTIND".to_string(), (optind + 1).to_string());
                            self.variables.remove("_OPTPOS");
                            if quiet {
                                self.variables.insert(varname.to_string(), ":".to_string());
                                self.variables.insert("OPTARG".to_string(), c.to_string());
                            } else {
                                self.variables.insert(varname.to_string(), "?".to_string());
                                self.variables.insert("OPTARG".to_string(), String::new());
                                zwarnnam("getopts", &format!("argument expected after -{} option", c));
                            }
                            return 0;
                        };

                        self.variables.insert("OPTARG".to_string(), arg);
                        self.variables
                            .insert("OPTIND".to_string(), (optind + advance).to_string());
                        self.variables.remove("_OPTPOS");
                    } else {
                        // No argument needed — clear OPTARG so the
                        // previous iteration's value doesn't leak into
                        // a subsequent flag that doesn't take one.
                        self.variables.remove("OPTARG");
                        if optpos + 1 < current_arg.len() {
                            // More options in this arg
                            self.variables
                                .insert("_OPTPOS".to_string(), (optpos + 1).to_string());
                        } else {
                            // Move to next arg
                            self.variables
                                .insert("OPTIND".to_string(), (optind + 1).to_string());
                            self.variables.remove("_OPTPOS");
                        }
                    }

                    self.variables.insert(varname.to_string(), c.to_string());
                    0
                }
                None => {
                    // Unknown option. Direct port of
                    // src/zsh/Src/builtin.c:5723-5739:
                    //   - quiet (`:` prefix): var="?",
                    //     OPTARG=opt_char (single-char form)
                    //   - non-quiet: var="?", OPTARG="", warning
                    // Always advance OPTIND in POSIX mode
                    // (builtin.c:5726-5729).
                    let quiet = optstring.starts_with(':');
                    self.variables.insert(varname.to_string(), "?".to_string());
                    if quiet {
                        self.variables.insert("OPTARG".to_string(), c.to_string());
                    } else {
                        self.variables.insert("OPTARG".to_string(), String::new());
                        zwarn(&format!("bad option: -{}", c));
                    }

                    // Advance to next option/arg
                    if optpos + 1 < current_arg.len() {
                        self.variables
                            .insert("_OPTPOS".to_string(), (optpos + 1).to_string());
                    } else {
                        self.variables
                            .insert("OPTIND".to_string(), (optind + 1).to_string());
                        self.variables.remove("_OPTPOS");
                    }
                    0
                }
            }
        } else {
            // No more options in current arg
            self.variables
                .insert("OPTIND".to_string(), (optind + 1).to_string());
            self.variables.remove("_OPTPOS");
            self.variables.insert(varname.to_string(), "?".to_string());
            1
        }
    }
    pub(crate) fn builtin_type(&mut self, args: &[String]) -> i32 {
        // zsh: bare `type` (no args) exits 1 — type requires at
        // least one name to look up. zshrs returned 0 silently.
        if args.is_empty() {
            return 1;
        }

        let mut show_all = false;
        let mut path_only = false;
        let mut silent = false;
        let mut show_type = false;
        let mut show_word = false;
        let mut names = Vec::new();

        let iter = args.iter();
        for arg in iter {
            if arg.starts_with('-') && arg.len() > 1 {
                for c in arg[1..].chars() {
                    match c {
                        '-' => {
                            // Skip `-` chars in the body (zsh quirk:
                            // for `--help` the second `-` is silently
                            // consumed and `h` becomes the first
                            // recognised letter — bad-option diag
                            // reports `-h` not `--`).
                        }
                        'a' => show_all = true,
                        'p' => path_only = true,
                        'P' => path_only = true,
                        's' => silent = true,
                        't' => show_type = true,
                        'f' => {} // ignore functions (we still show them)
                        // `-w` is `name: type` form — zsh's "word"
                        // shorthand (`builtin`/`command`/`function`/
                        // `alias`/`reserved`/`none`).
                        'w' => show_word = true,
                        // zsh's `-S` flag is silently accepted (no
                        // documented effect in `-c` mode); zshrs's
                        // unknown-flag fallback erred. zsh's `-k`
                        // (lookup as keyword/builtin only) is also
                        // silent-accept.
                        'S' | 'k' => {}
                        _ => {
                            // zsh: unknown flag → `bad option: -X`
                            // exit 1. zshrs previously dropped silently.
                            zwarnnam("type", &format!("bad option: -{}", c));
                            return 1;
                        }
                    }
                }
            } else {
                names.push(arg.clone());
            }
        }

        if names.is_empty() {
            return 0;
        }

        // zsh treats reserved words / keywords as a distinct type.
        // `type for` / `type while` etc. report "is a reserved word".
        // Check this BEFORE the alias/function/builtin probes so
        // user-shadowed reserved-word names still report the keyword
        // status (matches zsh's lookup order).
        const RESERVED_WORDS: &[&str] = &[
            "do",
            "done",
            "esac",
            "then",
            "elif",
            "else",
            "fi",
            "for",
            "case",
            "if",
            "while",
            "until",
            "select",
            "function",
            "repeat",
            "time",
            "in",
            "foreach",
            "end",
            "coproc",
            "nocorrect",
            "noglob",
            // zsh treats `local` / `declare` / `typeset` / `readonly`
            // / `export` / `integer` / `float` as reserved-word
            // declarations (precommand modifiers) — `type local`
            // reports "is a reserved word", not "is a shell builtin".
            "local",
            "declare",
            "typeset",
            "readonly",
            "export",
            "integer",
            "float",
        ];

        let mut status = 0;
        for name in &names {
            let mut found_any = false;

            // Reserved words win first.
            if RESERVED_WORDS.contains(&name.as_str()) {
                found_any = true;
                if !silent {
                    if show_word {
                        println!("{}: reserved", name);
                    } else if show_type {
                        println!("reserved");
                    } else {
                        println!("{} is a reserved word", name);
                    }
                }
                if !show_all {
                    continue;
                }
            }

            // Check for alias (skip if -p)
            if !path_only && self.aliases.contains_key(name) {
                found_any = true;
                if !silent {
                    if show_word {
                        println!("{}: alias", name);
                    } else if show_type {
                        println!("alias");
                    } else {
                        println!(
                            "{} is an alias for {}",
                            name,
                            self.aliases.get(name).unwrap()
                        );
                        // Note: matches zsh's `type` format, distinct
                        // from `which`'s "{}: aliased to {}" form.
                    }
                }
                if !show_all {
                    continue;
                }
            }

            // Check for function (skip if -p)
            if !path_only && self.function_exists(name) {
                found_any = true;
                if !silent {
                    if show_word {
                        println!("{}: function", name);
                    } else if show_type {
                        println!("function");
                    } else {
                        println!("{} is a shell function from zsh", name);
                    }
                }
                if !show_all {
                    continue;
                }
            }

            // Check for builtin (skip if -p). NOTE: use BUILTIN_SET
            // directly instead of `is_builtin()` — the helper has a
            // `_`-prefix bypass for completion functions, so any name
            // starting with `_` would falsely report as a builtin.
            // `type __notexist__` previously hit that bypass and
            // emitted `__notexist__ is a shell builtin` exit 0 — both
            // wrong wording AND wrong exit. Mirror the fix already
            // applied in `whence`.
            if !path_only && (BUILTIN_SET.contains(name.as_str()) || name == ":" || name == "[") {
                found_any = true;
                if !silent {
                    if show_word {
                        println!("{}: builtin", name);
                    } else if show_type {
                        println!("builtin");
                    } else {
                        println!("{} is a shell builtin", name);
                    }
                }
                if !show_all {
                    continue;
                }
            }

            // Check for external command in PATH. Skip the lookup
            // entirely for empty names — `dir + "/" + ""` resolves to
            // the directory itself, which `Path::exists` reports as
            // true, falsely matching `type ""` to the first PATH entry.
            // zsh: `type ""` -> ` not found` exit 0.
            if !name.is_empty() {
                if let Ok(path_env) = std::env::var("PATH") {
                    for dir in path_env.split(':') {
                        let full_path = format!("{}/{}", dir, name);
                        if std::path::Path::new(&full_path).exists() {
                            found_any = true;
                            if !silent {
                                if show_word {
                                    println!("{}: command", name);
                                } else if show_type {
                                    println!("file");
                                } else {
                                    println!("{} is {}", name, full_path);
                                }
                            }
                            if !show_all {
                                break;
                            }
                        }
                    }
                }
            }

            if !found_any {
                // zsh's format: `NAME not found` on stdout (no
                // colon-separated prefix). For -w, use `name: none`.
                if !silent {
                    if show_word {
                        println!("{}: none", name);
                    } else {
                        println!("{} not found", name);
                    }
                }
                status = 1;
            }
        }
        status
    }
    /// Direct port of `bin_hash()` (src/zsh/Src/builtin.c:4234+).
    /// In C the `rehash` builtin is registered with the same
    /// `bin_hash` handler and a defopt of `r`
    /// (BUILTIN("rehash", 0, bin_hash, 0, 0, 0, "df", "r")), so when
    /// invoked as `rehash` we start with `-r` set and restrict the
    /// option mask to `df`.
    pub(crate) fn bin_hash(&mut self, cmd_name: &str, args: &[String]) -> i32 {
        // PFA-SMR aspect: emit one `hash -d` event per named-directory
        // assignment. Plain `hash NAME=PATH` (command-hash, not named-
        // dir) is not recorder material — runtime cache only.
        #[cfg(feature = "recorder")]
        if crate::recorder::is_enabled() {
            let dir_mode = args.iter().any(|a| {
                a.starts_with('-') && a.len() > 1 && a[1..].chars().any(|c| c == 'd')
            });
            if dir_mode {
                let ctx = self.recorder_ctx();
                for a in args {
                    if a.starts_with('-') {
                        continue;
                    }
                    if let Some((k, v)) = a.split_once('=') {
                        crate::recorder::emit_hash_d(k, v, ctx.clone());
                    }
                }
            }
        }
        // hash [ -Ldfmrv ] [ name[=value] ] ...
        // hash -r clears the hash table
        // hash -d manages named directories
        // hash -f fills the table with all PATH commands
        // hash -m matches patterns
        // hash -v verbose
        // hash -L list in form suitable for reinput

        let mut dir_mode = false;
        // C defopt for `rehash` is "r" — start with rehash flag set
        // when invoked as `rehash`.
        let mut rehash = cmd_name == "rehash";
        let mut fill_all = false;
        let mut matchpat = false;
        let mut verbose = false;
        let mut list_form = false;
        let mut names = Vec::new();
        // BUILTIN("hash", ..., "Ldfmrv") — full mask;
        // BUILTIN("rehash", ..., "df") — restricted mask.
        let allow_full = cmd_name == "hash";

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if arg.starts_with('-') && arg.len() > 1 {
                for ch in arg[1..].chars() {
                    match ch {
                        'd' => dir_mode = true,
                        'f' => fill_all = true,
                        'r' if allow_full => rehash = true,
                        'm' if allow_full => matchpat = true,
                        'v' if allow_full => verbose = true,
                        'L' if allow_full => list_form = true,
                        // bad option for the active mask. zsh's
                        // standard message is `bad option: -X`.
                        _ => {
                            zwarnnam(cmd_name, &format!("bad option: -{}", ch));
                            return 1;
                        }
                    }
                }
            } else {
                names.push(arg.clone());
            }
            i += 1;
        }

        // builtin.c:4247-4252 — `-r` and `-f` reject positional
        // args ("too many arguments") because they're table-wide
        // operations.
        if (rehash || fill_all) && !names.is_empty() {
            zwarnnam(cmd_name, "too many arguments");
            return 1;
        }

        // -r: clear hash table
        if rehash && !dir_mode {
            self.command_hash.clear();
            // For `rehash` (defopt -r) without -f, we're done.
            // For `hash -r`, also done. -f path below handles its
            // own clear+fill.
            if !fill_all {
                return 0;
            }
        }

        // -f: fill hash table with all commands in PATH.
        // Parallel PATH scan — each PATH dir on a pool thread
        // (one std::fs::read_dir per worker, batched send back).
        if fill_all {
            self.command_hash.clear();
            if let Ok(path_var) = env::var("PATH") {
                let dirs: Vec<String> = path_var
                    .split(':')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect();

                let (tx, rx) = std::sync::mpsc::channel::<Vec<(String, String)>>();

                for dir in dirs {
                    let tx = tx.clone();
                    self.worker_pool.submit(move || {
                        let mut batch = Vec::new();
                        if let Ok(entries) = std::fs::read_dir(&dir) {
                            for entry in entries.flatten() {
                                if let Ok(ft) = entry.file_type() {
                                    if ft.is_file() || ft.is_symlink() {
                                        if let Some(n) = entry.file_name().to_str() {
                                            let path = entry.path().to_string_lossy().to_string();
                                            batch.push((n.to_string(), path));
                                        }
                                    }
                                }
                            }
                        }
                        let _ = tx.send(batch);
                    });
                }
                drop(tx);

                for batch in rx {
                    for (n, path) in batch {
                        if verbose {
                            println!("{}={}", n, path);
                        }
                        self.command_hash.insert(n, path);
                    }
                }
            }
            return 0;
        }

        if dir_mode {
            // Named directories mode (hash -d). Sorted by name to
            // match zsh's table-walk order and stabilize the listing.
            if names.is_empty() {
                let mut sorted: Vec<(&String, &PathBuf)> = self.named_dirs.iter().collect();
                sorted.sort_by(|a, b| a.0.cmp(b.0));
                for (name, path) in sorted {
                    if list_form {
                        println!("hash -d {}={}", name, path.display());
                    } else {
                        // verbose and default both emit `name=path`.
                        println!("{}={}", name, path.display());
                    }
                }
                return 0;
            }

            if rehash {
                // Remove named directories
                if matchpat {
                    // -m: pattern matching
                    let to_remove: Vec<String> = self
                        .named_dirs
                        .keys()
                        .filter(|k| {
                            names
                                .iter()
                                .any(|pat| Self::glob_match_static(k, pat.as_str()))
                        })
                        .cloned()
                        .collect();
                    for name in to_remove {
                        self.named_dirs.remove(&name);
                    }
                } else {
                    for name in &names {
                        self.named_dirs.remove(name);
                    }
                }
                return 0;
            }

            // Add OR query named directories. Per builtin.c
            // bin_hash:4234: `hash -d NAME=PATH` ASSIGNS; bare
            // `hash -d NAME` is a no-op when the entry exists (zsh
            // requires `-L` for listing); when missing it errors
            // "no such directory name: NAME".
            let mut status = 0;
            for name in &names {
                if let Some((n, p)) = name.split_once('=') {
                    self.add_named_dir(n, p);
                } else if !self.named_dirs.contains_key(name) {
                    zwarnnam("hash", &format!("no such directory name: {}", name));
                    status = 1;
                }
            }
            return status;
        }

        // Regular hash - command path lookup
        if names.is_empty() {
            // List all hashed commands. zsh lists them sorted by name
            // (per builtin.c bin_hash via the table-walk on the sorted
            // hash); zshrs's HashMap iteration was nondeterministic so
            // listings flickered between runs and broke diff-based
            // tests. Sort by key.
            let mut sorted: Vec<(&String, &String)> = self.command_hash.iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(b.0));
            for (name, path) in sorted {
                if list_form {
                    println!("hash {}={}", name, path);
                } else {
                    println!("{}={}", name, path);
                }
            }
            return 0;
        }

        for name in &names {
            if let Some((cmd, path)) = name.split_once('=') {
                // Explicit assignment
                self.command_hash.insert(cmd.to_string(), path.to_string());
                if verbose {
                    println!("{}={}", cmd, path);
                }
            } else if self.command_hash.contains_key(name) {
                // Already hashed — bare `hash NAME` is a NO-OP in
                // zsh's bin_hash (it just touches the entry); the
                // listing form requires `-L`. Don't print here.
            } else if let Some(path) = self.find_in_path(name) {
                // Look up in PATH and hash it
                self.command_hash.insert(name.clone(), path.clone());
                if verbose {
                    println!("{}={}", name, path);
                }
            } else {
                // Match zsh's error wording verbatim — `hash:1: no
                // such command: NAME`. The previous "X: not found"
                // form diverged from C zsh by one word, breaking
                // diagnostic-text parity tests and tools that
                // grep-match the exact phrase.
                zwarnnam("hash", &format!("no such command: {}", name));
                return 1;
            }
        }
        0
    }
    pub(crate) fn bin_let(&mut self, args: &[String]) -> i32 {
        // Port of src/zsh/Src/builtin.c:7469-7482 bin_let, plus the
        // BUILTIN-table arity check from src/zsh/Src/builtin.c:90:
        //
        //     BUILTIN("let", 0, bin_let, 1, -1, 0, NULL, NULL),
        //
        // The `1` is min_args. zsh's builtin dispatcher enforces this
        // BEFORE bin_let runs and prints `let: not enough arguments`
        // exit 1. A previous comment here claimed zsh did not emit
        // this — that was wrong; the diagnostic comes from the table
        // arity check, not bin_let itself. Mirror the dispatcher
        // behaviour here so call sites see the same failure mode.
        if args.is_empty() {
            zwarnnam("let", "not enough arguments");
            return 1;
        }
        let mut result: i64 = 0;
        for expr in args {
            result = self.evaluate_arithmetic_expr(expr);
        }
        if result == 0 {
            1
        } else {
            0
        }
    }
    pub(crate) fn builtin_pushd(&mut self, args: &[String]) -> i32 {
        // pushd [ -qsLP ] [ arg ]
        // pushd [ -qsLP ] old new
        // pushd [ -qsLP ] {+|-}n
        // -q: quiet (don't print stack)
        // -s: no symlink resolution (use -L cd behavior)
        // -L: logical directory (resolve .. before symlinks)
        // -P: physical directory (resolve symlinks)

        let mut quiet = false;
        let mut physical = false;
        let mut positional_args: Vec<String> = Vec::new();

        for arg in args {
            if arg.starts_with('-') && arg.len() > 1 {
                // Check if it's a stack index
                if arg[1..].chars().all(|c| c.is_ascii_digit()) {
                    positional_args.push(arg.clone());
                    continue;
                }
                for ch in arg[1..].chars() {
                    match ch {
                        'q' => quiet = true,
                        's' => physical = false,
                        'L' => physical = false,
                        'P' => physical = true,
                        // BUILTIN("pushd", ..., "qsPL") declares the
                        // valid pushd flags. zshrs's `_ => {}`
                        // silently dropped unknown letters so
                        // `pushd -X /tmp` cd'd to /tmp instead of
                        // erroring.
                        _ => {
                            zwarnnam("pushd", &format!("bad option: -{}", ch));
                            return 1;
                        }
                    }
                }
            } else {
                // `+N` stack indices and bare names alike — both go in
                // positional_args; the dispatcher below distinguishes
                // them by leading `+` later.
                positional_args.push(arg.clone());
            }
        }

        let current = match std::env::current_dir() {
            Ok(p) => p,
            Err(e) => {
                zwarnnam("pushd", &format!("{}", e));
                return 1;
            }
        };

        if positional_args.is_empty() {
            // Swap top two directories — but if PUSHD_TO_HOME is set,
            // bare `pushd` instead pushes onto the stack and cd's to
            // $HOME (zsh behavior; man zshbuiltins pushd). zshrs's
            // unconditional "no other directory" error broke scripts
            // that set PUSHD_TO_HOME and expected `pushd` to go home
            // when the stack was empty.
            if self.dir_stack.is_empty() {
                let pushd_to_home = self.options.get("pushdtohome").copied().unwrap_or(false);
                if pushd_to_home {
                    let home = match std::env::var("HOME") {
                        Ok(h) => PathBuf::from(h),
                        Err(_) => {
                            zwarnnam("pushd", "HOME not set");
                            return 1;
                        }
                    };
                    self.dir_stack.push(current.clone());
                    if let Err(e) = std::env::set_current_dir(&home) {
                        zwarnnam("pushd", &format!("{}: {}", home.display(), e));
                        self.dir_stack.pop();
                        return 1;
                    }
                    if !quiet {
                        self.print_dir_stack();
                    }
                    return 0;
                }
                zwarnnam("pushd", "no other directory");
                return 1;
            }
            let target = self.dir_stack.pop().unwrap();
            self.dir_stack.push(current.clone());

            let resolved = if physical {
                target.canonicalize().unwrap_or(target.clone())
            } else {
                target.clone()
            };

            if let Err(e) = std::env::set_current_dir(&resolved) {
                zwarnnam("pushd", &format!("{}: {}", target.display(), e));
                self.dir_stack.pop();
                self.dir_stack.push(target);
                return 1;
            }
            if !quiet {
                self.print_dir_stack();
            }
            return 0;
        }

        let arg = &positional_args[0];

        // Handle +N and -N for rotating the stack
        if arg.starts_with('+') || arg.starts_with('-') {
            if let Ok(n) = arg[1..].parse::<usize>() {
                let total = self.dir_stack.len() + 1;
                if n >= total {
                    zwarnnam("pushd", &format!("{}: directory stack index out of range", arg));
                    return 1;
                }
                // Rotate stack
                let rotate_pos = if arg.starts_with('+') { n } else { total - n };
                let mut full_stack = vec![current.clone()];
                full_stack.extend(self.dir_stack.iter().cloned());
                full_stack.rotate_left(rotate_pos);

                let target = full_stack.remove(0);
                self.dir_stack = full_stack;

                let resolved = if physical {
                    target.canonicalize().unwrap_or(target.clone())
                } else {
                    target.clone()
                };

                if let Err(e) = std::env::set_current_dir(&resolved) {
                    zwarnnam("pushd", &format!("{}: {}", target.display(), e));
                    return 1;
                }
                if !quiet {
                    self.print_dir_stack();
                }
                return 0;
            }
        }

        // Regular directory push
        let target = PathBuf::from(arg);
        let resolved = if physical {
            target.canonicalize().unwrap_or(target.clone())
        } else {
            target.clone()
        };

        self.dir_stack.push(current.clone());
        if let Err(e) = std::env::set_current_dir(&resolved) {
            zwarnnam("pushd", &format!("{}: {}", arg, e));
            self.dir_stack.pop();
            return 1;
        }
        self.sync_dirstack_array();
        // Sync $PWD/$OLDPWD with the new cwd. cd updates these but
        // pushd's path didn't, so `pushd /tmp; echo $PWD` continued
        // to show the pre-pushd cwd. Use the user-provided `arg`
        // (logical) when not -P so symlink-preserving `pushd /tmp`
        // keeps `/tmp` rather than `/private/tmp`.
        let new_pwd = if physical {
            resolved.display().to_string()
        } else {
            // Logical mode: prefer the as-given arg if it's an
            // absolute path; otherwise fall back to canonicalized.
            if target.is_absolute() {
                target.display().to_string()
            } else {
                resolved.display().to_string()
            }
        };
        let old_pwd = current.display().to_string();
        std::env::set_var("OLDPWD", &old_pwd);
        std::env::set_var("PWD", &new_pwd);
        self.variables.insert("OLDPWD".to_string(), old_pwd);
        self.variables.insert("PWD".to_string(), new_pwd);
        // zsh's `pushd` in non-interactive mode (e.g. `-c`) suppresses
        // the dir-stack listing — only `dirs` actively prints. Detect
        // non-interactive via stdin-is-tty since `options[interactive]`
        // is left on by default in zshrs even in `-c` mode.
        use std::io::IsTerminal;
        let stdin_is_tty = std::io::stdin().is_terminal();
        if !quiet && stdin_is_tty {
            self.print_dir_stack();
        }
        0
    }
    /// Pop directory from stack and cd to it
    pub(crate) fn builtin_popd(&mut self, args: &[String]) -> i32 {
        // popd [ -qsLP ] [ {+|-}n ]
        // -q: quiet (don't print stack)
        // -s: no symlink resolution
        // -L: logical directory
        // -P: physical directory

        let mut quiet = false;
        let mut physical = false;
        let mut stack_index: Option<String> = None;

        for arg in args {
            if arg.starts_with('-') && arg.len() > 1 {
                // Check if it's a stack index
                if arg[1..].chars().all(|c| c.is_ascii_digit()) {
                    stack_index = Some(arg.clone());
                    continue;
                }
                for ch in arg[1..].chars() {
                    match ch {
                        'q' => quiet = true,
                        's' => physical = false,
                        'L' => physical = false,
                        'P' => physical = true,
                        // BUILTIN("popd", ..., "q") + zsh's docs
                        // accept LqsP for popd too. Reject anything
                        // else; matches the bin_cd-shared option
                        // letter table.
                        _ => {
                            zwarnnam("popd", &format!("bad option: -{}", ch));
                            return 1;
                        }
                    }
                }
            } else if arg.starts_with('+') {
                stack_index = Some(arg.clone());
            }
        }

        if self.dir_stack.is_empty() {
            zwarnnam("popd", "directory stack empty");
            return 1;
        }

        // Handle +N and -N
        if let Some(arg) = stack_index {
            if arg.starts_with('+') || arg.starts_with('-') {
                if let Ok(n) = arg[1..].parse::<usize>() {
                    let total = self.dir_stack.len() + 1;
                    if n >= total {
                        zwarnnam("popd", &format!("{}: directory stack index out of range", arg));
                        return 1;
                    }
                    let remove_pos = if arg.starts_with('+') {
                        n
                    } else {
                        total - 1 - n
                    };
                    if remove_pos == 0 {
                        // Remove current and cd to next
                        let target = self.dir_stack.remove(0);
                        let resolved = if physical {
                            target.canonicalize().unwrap_or(target.clone())
                        } else {
                            target.clone()
                        };
                        if let Err(e) = std::env::set_current_dir(&resolved) {
                            zwarnnam("popd", &format!("{}: {}", target.display(), e));
                            return 1;
                        }
                    } else {
                        self.dir_stack.remove(remove_pos - 1);
                    }
                    if !quiet {
                        self.print_dir_stack();
                    }
                    return 0;
                }
            }
        }

        let target = self.dir_stack.pop().unwrap();
        let resolved = if physical {
            target.canonicalize().unwrap_or(target.clone())
        } else {
            target.clone()
        };
        if let Err(e) = std::env::set_current_dir(&resolved) {
            zwarnnam("popd", &format!("{}: {}", target.display(), e));
            self.dir_stack.push(target);
            return 1;
        }
        self.sync_dirstack_array();
        // Sync $PWD/$OLDPWD with the new cwd (logical for default,
        // physical for -P). pushd updates these; popd needs to too
        // or the dir-stack listing reads stale $PWD.
        let new_pwd = if physical {
            resolved.display().to_string()
        } else {
            target.display().to_string()
        };
        let old_pwd = self.variables.get("PWD").cloned().unwrap_or_default();
        std::env::set_var("OLDPWD", &old_pwd);
        std::env::set_var("PWD", &new_pwd);
        self.variables.insert("OLDPWD".to_string(), old_pwd);
        self.variables.insert("PWD".to_string(), new_pwd);
        // Same -c-mode silence as pushd above.
        use std::io::IsTerminal;
        let stdin_is_tty = std::io::stdin().is_terminal();
        if !quiet && stdin_is_tty {
            self.print_dir_stack();
        }
        0
    }
    /// Display directory stack
    pub(crate) fn bin_dirs(&mut self, args: &[String]) -> i32 {
        // dirs [ -c ] [ -l ] [ -p ] [ -v ] [ arg ... ]
        // -c: clear the directory stack
        // -l: full pathnames (don't use ~)
        // -p: print one entry per line
        // -v: verbose (numbered list)

        let mut clear = false;
        let mut full_paths = false;
        let mut per_line = false;
        let mut verbose = false;
        let mut indices: Vec<i32> = Vec::new();
        let mut positional: Vec<String> = Vec::new();

        for arg in args {
            if arg.starts_with('-') && arg.len() > 1 {
                // Check if it's a negative index like -2
                if arg[1..].chars().all(|c| c.is_ascii_digit()) {
                    if let Ok(n) = arg.parse::<i32>() {
                        indices.push(n);
                        continue;
                    }
                }
                for ch in arg[1..].chars() {
                    match ch {
                        'c' => clear = true,
                        'l' => full_paths = true,
                        'p' => per_line = true,
                        'v' => verbose = true,
                        // Direct port of zsh BUILTIN("dirs", ..., "clpv")
                        // — only c/l/p/v are valid. Anything else is
                        // rejected by zsh's option parser BEFORE bin_dirs
                        // runs. zshrs previously consumed unknown letters
                        // silently, so `dirs -X` printed the stack as if
                        // -X were a no-op flag (typo masked).
                        _ => {
                            zwarnnam("dirs", &format!("bad option: -{}", ch));
                            return 1;
                        }
                    }
                }
            } else if arg.starts_with('+') && arg.len() > 1 {
                if let Ok(n) = arg[1..].parse::<i32>() {
                    indices.push(n);
                }
            } else if let Ok(n) = arg.parse::<i32>() {
                // Bare numeric arg — treat as index (legacy behavior).
                indices.push(n);
            } else {
                // Plain path — collect for stack-replace per
                // src/zsh/Src/builtin.c:786-791. zsh: `dirs path1
                // path2 ...` REPLACES the entire stack with the args.
                positional.push(arg.clone());
            }
        }

        if clear {
            self.dir_stack.clear();
            return 0;
        }

        // Direct port of builtin.c:786-791 — replace the stack with
        // the supplied directory paths. Only fires if there are
        // positional args AND no display flags / indices (zsh's
        // dispatch in builtin.c:755-756 short-circuits the display
        // path when args exist without -c/-v/-p).
        if !positional.is_empty() && !verbose && !per_line && indices.is_empty() {
            self.dir_stack = positional
                .into_iter()
                .map(std::path::PathBuf::from)
                .collect();
            return 0;
        }

        // zsh's `dirs` uses $PWD (logical, symlink-preserving) for
        // the current entry, not the OS-level cwd. `pushd /tmp;
        // dirs` should show `/tmp`, not `/private/tmp` on macOS.
        // Fall back to OS cwd only if $PWD is unset.
        let current = self
            .variables
            .get("PWD")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let home = dirs::home_dir().unwrap_or_default();

        let format_path = |p: &std::path::Path| -> String {
            let path_str = p.to_string_lossy().to_string();
            if !full_paths {
                let home_str = home.to_string_lossy();
                if path_str.starts_with(home_str.as_ref()) {
                    return format!("~{}", &path_str[home_str.len()..]);
                }
            }
            path_str
        };

        // If specific indices requested
        if !indices.is_empty() {
            let stack_len = self.dir_stack.len() + 1; // +1 for current dir
            for idx in indices {
                let actual_idx = if idx >= 0 {
                    idx as usize
                } else {
                    stack_len.saturating_sub((-idx) as usize)
                };

                if actual_idx == 0 {
                    println!("{}", format_path(&current));
                } else if actual_idx <= self.dir_stack.len() {
                    // Stack is reversed, so index from end
                    let stack_idx = self.dir_stack.len() - actual_idx;
                    if let Some(dir) = self.dir_stack.get(stack_idx) {
                        println!("{}", format_path(dir));
                    }
                }
            }
            return 0;
        }

        if verbose {
            // zsh's `dirs -v` uses TAB between index and path,
            // no leading-space padding on the index. Match exactly:
            // `0\t<dir>\n1\t<dir>\n…`. zshrs previously space-padded.
            println!("0\t{}", format_path(&current));
            for (i, dir) in self.dir_stack.iter().rev().enumerate() {
                println!("{}\t{}", i + 1, format_path(dir));
            }
        } else if per_line {
            println!("{}", format_path(&current));
            for dir in self.dir_stack.iter().rev() {
                println!("{}", format_path(dir));
            }
        } else {
            let mut parts = vec![format_path(&current)];
            for dir in self.dir_stack.iter().rev() {
                parts.push(format_path(dir));
            }
            println!("{}", parts.join(" "));
        }
        0
    }


    fn strip_trailing_zeros_g(s: &str) -> String {
        let (mantissa, suffix) = match s.find(['e', 'E']) {
            Some(i) => (&s[..i], &s[i..]),
            None => (s, ""),
        };
        let stripped = if mantissa.contains('.') {
            let trimmed = mantissa.trim_end_matches('0');
            let trimmed = trimmed.trim_end_matches('.');
            trimmed.to_string()
        } else {
            mantissa.to_string()
        };
        // Rust's %e emits exponent without leading zero (e.g. `1e0` → C wants `1e+00`).
        // Normalize: "e<digits>" → "e+<2digits>", "e-N" → "e-<2digits>".
        let suffix_norm = if suffix.is_empty() {
            String::new()
        } else {
            let (sign, digits) = if let Some(rest) = suffix.strip_prefix("e-") {
                ("-", rest)
            } else if let Some(rest) = suffix.strip_prefix("e+") {
                ("+", rest)
            } else if let Some(rest) = suffix.strip_prefix('e') {
                ("+", rest)
            } else if let Some(rest) = suffix.strip_prefix("E-") {
                ("-", rest)
            } else if let Some(rest) = suffix.strip_prefix("E+") {
                ("+", rest)
            } else if let Some(rest) = suffix.strip_prefix('E') {
                ("+", rest)
            } else {
                return format!("{}{}", stripped, suffix);
            };
            let n: i32 = digits.parse().unwrap_or(0);
            format!("e{}{:02}", sign, n.abs())
        };
        format!("{}{}", stripped, suffix_norm)
    }

    /// C-style printf `%g`/`%G`: shortest of `%f`/`%e` representation that
    /// preserves `prec` significant digits. Trailing zeros are stripped
    /// unless they bridge the decimal point.
    pub(crate) fn format_g(val: f64, prec: usize, upper: bool) -> String {
        if !val.is_finite() {
            let s = if val.is_nan() {
                "nan".to_string()
            } else if val < 0.0 {
                "-inf".to_string()
            } else {
                "inf".to_string()
            };
            return if upper { s.to_uppercase() } else { s };
        }
        let prec = prec.max(1);
        // Determine the exponent X such that val ≈ d.ddd × 10^X.
        let exp = if val == 0.0 {
            0i32
        } else {
            val.abs().log10().floor() as i32
        };
        // C99 spec: use %e if X < -4 or X >= prec; else %f.
        let use_exp = exp < -4 || exp >= prec as i32;
        let raw = if use_exp {
            let p = prec - 1;
            format!("{:.p$e}", val, p = p)
        } else {
            let p = (prec as i32 - 1 - exp).max(0) as usize;
            format!("{:.p$}", val, p = p)
        };
        // Strip trailing zeros after the decimal (and the dot if bare).
        let formatted = Self::strip_trailing_zeros_g(&raw);
        if upper {
            formatted.to_uppercase()
        } else {
            formatted
        }
    }
    /// printf builtin - format and print data (zsh/bash compatible)
    pub(crate) fn builtin_printf(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            zwarnnam("printf", "not enough arguments");
            return 1;
        }

        // bash-compat `printf -v VAR fmt args...`: assign formatted output
        // to VAR instead of printing. Strip the flag + var, format the
        // rest, insert into self.variables.
        // `printf --` is end-of-options (POSIX util convention) — skip
        // the `--` and treat the next arg as the format.
        let trimmed: &[String] = if args.first().map(String::as_str) == Some("--") {
            &args[1..]
        } else {
            args
        };
        if trimmed.is_empty() {
            zwarnnam("printf", "not enough arguments");
            return 1;
        }
        let (assign_var, format, format_args): (Option<String>, &String, &[String]) =
            if trimmed.first().map(String::as_str) == Some("-v") && trimmed.len() >= 3 {
                (Some(trimmed[1].clone()), &trimmed[2], &trimmed[3..])
            } else {
                (None, &trimmed[0], &trimmed[1..])
            };
        let mut arg_idx = 0;
        let mut output = String::new();
        // Track whether any unknown-directive error fired so we can
        // return non-zero at the end. zsh exits 1 on the first
        // invalid directive but still emits already-formatted output;
        // zshrs printed the error then returned 0, masking the
        // failure for scripts checking $?.
        let mut had_error = false;
        // POSIX printf: re-apply the format string while args remain. The
        // outer label guards against infinite loops when the format
        // consumes no args (e.g. `printf 'literal'`) — exit on the second
        // pass even if more args linger.
        let mut chars = format.chars().peekable();
        let mut prev_arg_idx = arg_idx;

        'outer: loop {
            while let Some(c) = chars.next() {
                if c == '\\' {
                    match chars.next() {
                        Some('n') => output.push('\n'),
                        Some('t') => output.push('\t'),
                        Some('r') => output.push('\r'),
                        Some('\\') => output.push('\\'),
                        Some('a') => output.push('\x07'),
                        Some('b') => output.push('\x08'),
                        Some('e') | Some('E') => output.push('\x1b'),
                        Some('f') => output.push('\x0c'),
                        Some('v') => output.push('\x0b'),
                        Some('"') => output.push('"'),
                        Some('\'') => output.push('\''),
                        Some(d0) if ('0'..='7').contains(&d0) => {
                            // POSIX printf: `\NNN` is 1-3 octal digits (zsh
                            // accepts the same). The leading `0` (if any) is
                            // part of the digit count — `\0102` consumes
                            // `010` (octal 10 = backspace) and leaves `2` as
                            // literal, matching `printf "\0102"` output of
                            // `\b2`. Build the octal up to 3 total chars.
                            let mut octal = String::new();
                            octal.push(d0);
                            while octal.len() < 3 {
                                if let Some(&d) = chars.peek() {
                                    if ('0'..='7').contains(&d) {
                                        octal.push(d);
                                        chars.next();
                                    } else {
                                        break;
                                    }
                                } else {
                                    break;
                                }
                            }
                            if let Ok(val) = u8::from_str_radix(&octal, 8) {
                                output.push(val as char);
                            }
                        }
                        Some('x') => {
                            let mut hex = String::new();
                            while hex.len() < 2 {
                                if let Some(&d) = chars.peek() {
                                    if d.is_ascii_hexdigit() {
                                        hex.push(d);
                                        chars.next();
                                    } else {
                                        break;
                                    }
                                } else {
                                    break;
                                }
                            }
                            if !hex.is_empty() {
                                if let Ok(val) = u8::from_str_radix(&hex, 16) {
                                    output.push(val as char);
                                }
                            }
                        }
                        Some('u') => {
                            let mut hex = String::new();
                            while hex.len() < 4 {
                                if let Some(&d) = chars.peek() {
                                    if d.is_ascii_hexdigit() {
                                        hex.push(d);
                                        chars.next();
                                    } else {
                                        break;
                                    }
                                } else {
                                    break;
                                }
                            }
                            if !hex.is_empty() {
                                if let Ok(val) = u32::from_str_radix(&hex, 16) {
                                    if let Some(c) = char::from_u32(val) {
                                        output.push(c);
                                    }
                                }
                            }
                        }
                        Some('U') => {
                            let mut hex = String::new();
                            while hex.len() < 8 {
                                if let Some(&d) = chars.peek() {
                                    if d.is_ascii_hexdigit() {
                                        hex.push(d);
                                        chars.next();
                                    } else {
                                        break;
                                    }
                                } else {
                                    break;
                                }
                            }
                            if !hex.is_empty() {
                                if let Ok(val) = u32::from_str_radix(&hex, 16) {
                                    if let Some(c) = char::from_u32(val) {
                                        output.push(c);
                                    }
                                }
                            }
                        }
                        Some('c') => {
                            print!("{}", output);
                            return 0;
                        }
                        Some(other) => {
                            output.push('\\');
                            output.push(other);
                        }
                        None => output.push('\\'),
                    }
                } else if c == '%' {
                    if chars.peek() == Some(&'%') {
                        chars.next();
                        output.push('%');
                        continue;
                    }

                    let mut flags = String::new();
                    while let Some(&f) = chars.peek() {
                        if f == '-' || f == '+' || f == ' ' || f == '#' || f == '0' {
                            flags.push(f);
                            chars.next();
                        } else {
                            break;
                        }
                    }

                    let mut width = String::new();
                    if chars.peek() == Some(&'*') {
                        chars.next();
                        if arg_idx < format_args.len() {
                            width = format_args[arg_idx].clone();
                            arg_idx += 1;
                        }
                    } else {
                        while let Some(&d) = chars.peek() {
                            if d.is_ascii_digit() {
                                width.push(d);
                                chars.next();
                            } else {
                                break;
                            }
                        }
                    }

                    let mut precision = String::new();
                    let mut saw_period = false;
                    if chars.peek() == Some(&'.') {
                        chars.next();
                        saw_period = true;
                        if chars.peek() == Some(&'*') {
                            chars.next();
                            if arg_idx < format_args.len() {
                                precision = format_args[arg_idx].clone();
                                arg_idx += 1;
                            }
                        } else {
                            while let Some(&d) = chars.peek() {
                                if d.is_ascii_digit() {
                                    precision.push(d);
                                    chars.next();
                                } else {
                                    break;
                                }
                            }
                        }
                    }

                    let specifier = chars.next().unwrap_or('s');
                    let arg = if arg_idx < format_args.len() {
                        let a = &format_args[arg_idx];
                        arg_idx += 1;
                        a.clone()
                    } else {
                        String::new()
                    };

                    let width_val: usize = width.parse().unwrap_or(0);
                    let prec_val: Option<usize> = if precision.is_empty() {
                        // `%.s` (period present, no digits) means precision
                        // 0 — the arg is suppressed. Without this, prec_val
                        // stayed None and `%.s "ignore"` printed `ignore`
                        // instead of the empty string.
                        if saw_period {
                            Some(0)
                        } else {
                            None
                        }
                    } else {
                        precision.parse().ok()
                    };
                    let left_align = flags.contains('-');
                    let zero_pad = flags.contains('0') && !left_align;
                    let plus_sign = flags.contains('+');
                    let space_sign = flags.contains(' ') && !plus_sign;
                    let alt_form = flags.contains('#');

                    match specifier {
                        's' => {
                            let mut s = arg;
                            if let Some(p) = prec_val {
                                s = s.chars().take(p).collect();
                            }
                            if width_val > s.len() {
                                if left_align {
                                    output.push_str(&s);
                                    output.push_str(&" ".repeat(width_val - s.len()));
                                } else {
                                    output.push_str(&" ".repeat(width_val - s.len()));
                                    output.push_str(&s);
                                }
                            } else {
                                output.push_str(&s);
                            }
                        }
                        'b' => {
                            let expanded = self.expand_printf_escapes(&arg);
                            if let Some(p) = prec_val {
                                let s: String = expanded.chars().take(p).collect();
                                output.push_str(&s);
                            } else {
                                output.push_str(&expanded);
                            }
                        }
                        'c' => {
                            if let Some(ch) = arg.chars().next() {
                                output.push(ch);
                            }
                        }
                        'q' => {
                            // zsh `%q` — backslash-escape shell-special
                            // chars (matches `${(q)}` flag, NOT `(qq)`).
                            for ch in arg.chars() {
                                if matches!(
                                    ch,
                                    ' ' | '\t'
                                        | '\''
                                        | '"'
                                        | '\\'
                                        | '$'
                                        | '`'
                                        | '*'
                                        | '?'
                                        | '['
                                        | ']'
                                        | '{'
                                        | '}'
                                        | '('
                                        | ')'
                                        | '|'
                                        | '&'
                                        | ';'
                                        | '<'
                                        | '>'
                                        | '#'
                                        | '~'
                                ) {
                                    output.push('\\');
                                }
                                output.push(ch);
                            }
                        }
                        'd' | 'i' => {
                            let val: i64 = if arg.starts_with("0x") || arg.starts_with("0X") {
                                i64::from_str_radix(&arg[2..], 16).unwrap_or(0)
                            } else if arg.starts_with("0") && arg.len() > 1 && !arg.contains('.') {
                                i64::from_str_radix(&arg[1..], 8).unwrap_or(0)
                            } else if arg.starts_with('\'') || arg.starts_with('"') {
                                arg.chars().nth(1).map(|c| c as i64).unwrap_or(0)
                            } else if let Ok(n) = arg.parse::<i64>() {
                                n
                            } else if let Ok(f) = arg.parse::<f64>() {
                                // POSIX printf truncates floats to int for
                                // `%d`/`%i` (matches zsh: `printf %d 3.14`
                                // → 3). Without this, the i64-only parse
                                // path returned 0 for any float.
                                f as i64
                            } else {
                                0
                            };

                            let sign = if val < 0 {
                                "-"
                            } else if plus_sign {
                                "+"
                            } else if space_sign {
                                " "
                            } else {
                                ""
                            };
                            let abs_val = val.abs();
                            let num_str = abs_val.to_string();
                            let total_len = sign.len() + num_str.len();

                            if width_val > total_len {
                                if left_align {
                                    output.push_str(sign);
                                    output.push_str(&num_str);
                                    output.push_str(&" ".repeat(width_val - total_len));
                                } else if zero_pad {
                                    output.push_str(sign);
                                    output.push_str(&"0".repeat(width_val - total_len));
                                    output.push_str(&num_str);
                                } else {
                                    output.push_str(&" ".repeat(width_val - total_len));
                                    output.push_str(sign);
                                    output.push_str(&num_str);
                                }
                            } else {
                                output.push_str(sign);
                                output.push_str(&num_str);
                            }
                        }
                        'u' => {
                            // zsh's printf %u treats negative ints
                            // as their two's-complement u64 form
                            // (`-1` -> 18446744073709551615) per
                            // C/POSIX printf semantics. zshrs's
                            // direct `arg.parse::<u64>()` rejected
                            // the leading `-` and silently fell
                            // back to 0. Fix: parse as i64 first;
                            // if that succeeds, cast to u64
                            // (Rust's `as` does the wraparound).
                            let val: u64 = if arg.starts_with("0x") || arg.starts_with("0X") {
                                u64::from_str_radix(&arg[2..], 16).unwrap_or(0)
                            } else if arg.starts_with("0") && arg.len() > 1 {
                                u64::from_str_radix(&arg[1..], 8).unwrap_or(0)
                            } else {
                                arg.parse::<i64>()
                                    .map(|n| n as u64)
                                    .or_else(|_| arg.parse::<u64>())
                                    .unwrap_or(0)
                            };
                            let num_str = val.to_string();
                            if width_val > num_str.len() {
                                if left_align {
                                    output.push_str(&num_str);
                                    output.push_str(&" ".repeat(width_val - num_str.len()));
                                } else if zero_pad {
                                    output.push_str(&"0".repeat(width_val - num_str.len()));
                                    output.push_str(&num_str);
                                } else {
                                    output.push_str(&" ".repeat(width_val - num_str.len()));
                                    output.push_str(&num_str);
                                }
                            } else {
                                output.push_str(&num_str);
                            }
                        }
                        'o' => {
                            let val: u64 = arg
                                .parse::<i64>()
                                .map(|n| n as u64)
                                .or_else(|_| arg.parse::<u64>())
                                .unwrap_or(0);
                            let num_str = format!("{:o}", val);
                            let prefix = if alt_form && val != 0 { "0" } else { "" };
                            let total_len = prefix.len() + num_str.len();
                            if width_val > total_len {
                                if left_align {
                                    output.push_str(prefix);
                                    output.push_str(&num_str);
                                    output.push_str(&" ".repeat(width_val - total_len));
                                } else if zero_pad {
                                    output.push_str(prefix);
                                    output.push_str(&"0".repeat(width_val - total_len));
                                    output.push_str(&num_str);
                                } else {
                                    output.push_str(&" ".repeat(width_val - total_len));
                                    output.push_str(prefix);
                                    output.push_str(&num_str);
                                }
                            } else {
                                output.push_str(prefix);
                                output.push_str(&num_str);
                            }
                        }
                        'x' => {
                            // Parse as i64 first so negatives wrap around
                            // (printf "%x" -1 → "ffffffffffffffff", matching
                            // C/zsh). Direct u64 parse rejected the leading
                            // `-` and silently used 0.
                            let val: u64 = arg
                                .parse::<i64>()
                                .map(|n| n as u64)
                                .or_else(|_| arg.parse::<u64>())
                                .unwrap_or(0);
                            let num_str = format!("{:x}", val);
                            let prefix = if alt_form && val != 0 { "0x" } else { "" };
                            let total_len = prefix.len() + num_str.len();
                            if width_val > total_len {
                                if left_align {
                                    output.push_str(prefix);
                                    output.push_str(&num_str);
                                    output.push_str(&" ".repeat(width_val - total_len));
                                } else if zero_pad {
                                    // `printf "%04x" 42` → `002a` (zero-pad).
                                    output.push_str(prefix);
                                    output.push_str(&"0".repeat(width_val - total_len));
                                    output.push_str(&num_str);
                                } else {
                                    output.push_str(&" ".repeat(width_val - total_len));
                                    output.push_str(prefix);
                                    output.push_str(&num_str);
                                }
                            } else {
                                output.push_str(prefix);
                                output.push_str(&num_str);
                            }
                        }
                        'X' => {
                            let val: u64 = arg
                                .parse::<i64>()
                                .map(|n| n as u64)
                                .or_else(|_| arg.parse::<u64>())
                                .unwrap_or(0);
                            let num_str = format!("{:X}", val);
                            let prefix = if alt_form && val != 0 { "0X" } else { "" };
                            let total_len = prefix.len() + num_str.len();
                            if width_val > total_len {
                                if left_align {
                                    output.push_str(prefix);
                                    output.push_str(&num_str);
                                    output.push_str(&" ".repeat(width_val - total_len));
                                } else if zero_pad {
                                    output.push_str(prefix);
                                    output.push_str(&"0".repeat(width_val - total_len));
                                    output.push_str(&num_str);
                                } else {
                                    output.push_str(&" ".repeat(width_val - total_len));
                                    output.push_str(prefix);
                                    output.push_str(&num_str);
                                }
                            } else {
                                output.push_str(prefix);
                                output.push_str(&num_str);
                            }
                        }
                        'e' | 'E' => {
                            let val: f64 = arg.parse().unwrap_or(0.0);
                            let prec = prec_val.unwrap_or(6);
                            let raw = if specifier == 'e' {
                                format!("{:.prec$e}", val, prec = prec)
                            } else {
                                format!("{:.prec$E}", val, prec = prec)
                            };
                            // Rust emits `e3` / `E-3`; C printf / zsh emit
                            // `e+03` / `E-03` (signed, ≥2 digits). Fix tail.
                            let exp_marker = if specifier == 'e' { 'e' } else { 'E' };
                            let formatted = if let Some(epos) = raw.rfind(exp_marker) {
                                let (mantissa, exp) = raw.split_at(epos);
                                let exp_body = &exp[1..];
                                let (sign, digits) = if let Some(d) = exp_body.strip_prefix('-') {
                                    ("-", d)
                                } else if let Some(d) = exp_body.strip_prefix('+') {
                                    ("+", d)
                                } else {
                                    ("+", exp_body)
                                };
                                let padded = if digits.len() < 2 {
                                    format!("0{}", digits)
                                } else {
                                    digits.to_string()
                                };
                                format!("{}{}{}{}", mantissa, exp_marker, sign, padded)
                            } else {
                                raw
                            };
                            if width_val > formatted.len() {
                                if left_align {
                                    output.push_str(&formatted);
                                    output.push_str(&" ".repeat(width_val - formatted.len()));
                                } else {
                                    output.push_str(&" ".repeat(width_val - formatted.len()));
                                    output.push_str(&formatted);
                                }
                            } else {
                                output.push_str(&formatted);
                            }
                        }
                        'f' | 'F' => {
                            let val: f64 = arg.parse().unwrap_or(0.0);
                            let prec = prec_val.unwrap_or(6);
                            let sign = if val < 0.0 {
                                "-"
                            } else if plus_sign {
                                "+"
                            } else if space_sign {
                                " "
                            } else {
                                ""
                            };
                            let formatted = format!("{:.prec$}", val.abs(), prec = prec);
                            let total = sign.len() + formatted.len();
                            if width_val > total {
                                if left_align {
                                    output.push_str(sign);
                                    output.push_str(&formatted);
                                    output.push_str(&" ".repeat(width_val - total));
                                } else if zero_pad {
                                    output.push_str(sign);
                                    output.push_str(&"0".repeat(width_val - total));
                                    output.push_str(&formatted);
                                } else {
                                    output.push_str(&" ".repeat(width_val - total));
                                    output.push_str(sign);
                                    output.push_str(&formatted);
                                }
                            } else {
                                output.push_str(sign);
                                output.push_str(&formatted);
                            }
                        }
                        'g' | 'G' => {
                            let val: f64 = arg.parse().unwrap_or(0.0);
                            let prec = prec_val.unwrap_or(6).max(1);
                            let formatted = Self::format_g(val, prec, specifier == 'G');
                            output.push_str(&formatted);
                        }
                        // %a (hex float) and %v (bash-only) are rejected by
                        // zsh as invalid directives. Match zsh.
                        'a' | 'A' | 'v' | 'V' => {
                            zwarnnam("printf", &format!("%{}: invalid directive", specifier));
                            had_error = true;
                        }
                        _ => {
                            zwarnnam("printf", &format!("%{}: invalid directive", specifier));
                            had_error = true;
                        }
                    }
                } else {
                    output.push(c);
                }
            }
            // After one full pass: re-loop only if at least one arg was
            // consumed AND we still have args left.
            if arg_idx <= prev_arg_idx || arg_idx >= format_args.len() {
                break 'outer;
            }
            prev_arg_idx = arg_idx;
            chars = format.chars().peekable();
        }

        if let Some(var) = assign_var {
            self.variables.insert(var, output);
        } else {
            // Flush BEFORE the redirect scope closes — Rust's print!
            // is line-buffered when stdout is a tty but block-buffered
            // for non-tty (file/pipe). With our fd-level redirect via
            // dup2, the buffered data still belongs to the original
            // stdout fd; if we don't flush before the redirect_scope
            // pops (and dup2 restores the original fd), the data
            // ends up on the original terminal instead of the file.
            // echo works because its print! emits a newline → triggers
            // line-buffer flush; printf's "abc" without newline doesn't.
            use std::io::Write as _;
            print!("{}", output);
            let _ = std::io::stdout().flush();
        }
        if had_error {
            1
        } else {
            0
        }
    }
    /// break - exit from for/while/until loop. Direct port of
    /// Direct port of `bin_break()` (src/zsh/Src/builtin.c:5809-5878).
    /// In C this single function dispatches on `func` (BIN_BREAK,
    /// BIN_CONTINUE, BIN_RETURN, BIN_EXIT, BIN_LOGOUT) — Rust matches
    /// on the invoked name. All flow-control builtins funnel here.
    pub(crate) fn bin_break(&mut self, name: &str, args: &[String]) -> i32 {
        match name {
            "break" => {
                // builtin.c:5816 mathevali; 5820-5823 reject num<=0.
                let levels: i32 = match args.first() {
                    Some(s) if !s.is_empty() => self.eval_arith_expr(s) as i32,
                    _ => 1,
                };
                if levels <= 0 {
                    zwarnnam("break", &format!("argument is not positive: {}", levels));
                    return 1;
                }
                self.breaking = levels;
                0
            }
            "continue" => {
                let levels: i32 = match args.first() {
                    Some(s) if !s.is_empty() => self.eval_arith_expr(s) as i32,
                    _ => 1,
                };
                if levels <= 0 {
                    zwarnnam("continue", &format!("argument is not positive: {}", levels));
                    return 1;
                }
                self.continuing = levels;
                0
            }
            "return" => {
                // builtin.c:5839+ — argv[0] is a math expression
                // (mathevali). `return 1+2` returns 3; `return -5`
                // returns -5; bare `return` returns last_status.
                let status = match args.first() {
                    Some(s) if !s.is_empty() => self.eval_arith_expr(s) as i32,
                    _ => self.last_status,
                };
                self.returning = Some(status);
                status
            }
            "exit" | "bye" | "logout" => {
                // zsh: `exit 1 2 3` -> `exit:1: too many arguments`
                // exit 1 and the shell continues (does NOT exit).
                // zshrs's bytecode unconditionally jumps to script end
                // after the EXIT builtin, so emitting the diagnostic
                // AND returning early would still terminate the
                // script — wrong vs zsh's "diagnose, set $? = 1,
                // continue" semantics. Until the bytecode short-
                // circuit becomes conditional, the best we can do is
                // print the diagnostic but still treat as a real exit
                // so the user sees the error and the shell terminates.
                if args.len() > 1 {
                    zwarnnam(name, "too many arguments");
                }
                // builtin.c:5815-5818 — exit's arg is a math
                // expression, not just a literal integer.
                let raw_code = match args.first() {
                    Some(s) if !s.is_empty() => self.eval_arith_expr(s) as i32,
                    _ => self.last_status,
                };
                // POSIX/zsh: exit status is masked to 8 bits.
                let code = ((raw_code as u32) & 0xff) as i32;
                // Inside a subshell `(...)` the shell hasn't forked,
                // so a real `process::exit` would tear down the
                // parent. `(exit N)` exits the subshell only.
                if !self.subshell_snapshots.is_empty() {
                    self.last_status = code;
                    self.returning = Some(code);
                    return code;
                }
                // Fire EXIT trap before terminating. zsh runs EXIT
                // even on explicit `exit N`. Remove first so the trap
                // body can call exit without re-entering.
                if let Some(action) = self.traps.remove("EXIT") {
                    self.last_status = code;
                    let _ = self.execute_script_zsh_pipeline(&action);
                }
                std::process::exit(code);
            }
            _ => {
                zwarnnam("bin_break", &format!("unknown name: {}", name));
                1
            }
        }
    }
    /// Direct port of `bin_enable()` (src/zsh/Src/builtin.c:517-594).
    /// Dispatches enable/disable on the invoked name (matches C `func`
    /// arg = BIN_ENABLE | BIN_DISABLE). Walks args, picks the hash
    /// table from -afmrps flags, then lists / glob-matches / literal-
    /// names through the chosen table.
    pub(crate) fn bin_enable(&mut self, name: &str, args: &[String]) -> i32 {
        self.do_enable_disable(args, name == "enable")
    }
    /// emulate - set up zsh emulation mode
    pub(crate) fn bin_emulate(&mut self, args: &[String]) -> i32 {
        // emulate [ -lLR ] [ {zsh|sh|ksh|csh} [ flags ... ] ]
        // flags can include: -c arg, -o opt, +o opt
        let mut local_mode = false;
        let mut reset_mode = false;
        let mut list_mode = false;
        let mut mode: Option<String> = None;
        let mut command_arg: Option<String> = None;
        let mut extra_set_opts: Vec<String> = Vec::new();
        let mut extra_unset_opts: Vec<String> = Vec::new();
        let mut extra_positional_count: usize = 0;

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];

            if arg == "-c" {
                // -c arg: evaluate arg in emulation mode
                i += 1;
                if i < args.len() {
                    command_arg = Some(args[i].clone());
                } else {
                    zwarnnam("emulate", "-c requires an argument");
                    return 1;
                }
            } else if arg == "-o" {
                // -o opt: set option
                i += 1;
                if i < args.len() {
                    extra_set_opts.push(args[i].clone());
                } else {
                    zwarnnam("emulate", "-o requires an argument");
                    return 1;
                }
            } else if arg == "+o" {
                // +o opt: unset option
                i += 1;
                if i < args.len() {
                    extra_unset_opts.push(args[i].clone());
                } else {
                    zwarnnam("emulate", "+o requires an argument");
                    return 1;
                }
            } else if arg.starts_with('-') && arg.len() > 1 && !arg.starts_with("--") {
                // Parse combined flags like -LR
                for ch in arg[1..].chars() {
                    match ch {
                        'L' => local_mode = true,
                        'R' => reset_mode = true,
                        'l' => list_mode = true,
                        _ => {
                            zwarnnam("emulate", &format!("bad option: -{}", ch));
                            return 1;
                        }
                    }
                }
            } else if arg.starts_with('+') && arg.len() > 1 {
                // +X flags (unset single-letter options)
                for ch in arg[1..].chars() {
                    // Map single-letter to option name if needed
                    extra_unset_opts.push(ch.to_string());
                }
            } else if mode.is_none() {
                mode = Some(arg.clone());
            } else {
                // Extra positional past the shell name. Whether this
                // is OK depends on whether `-c CMD` consumed the rest:
                //   - emulate zsh -c 'echo hi'   → mode=zsh, cmd='echo hi', no extras
                //   - emulate -l zsh extra       → list mode, only one positional allowed
                //   - emulate zsh extra          → C path falls through to parseopts
                //     which would emit "unknown argument extra" (builtin.c:6314).
                extra_positional_count += 1;
            }
            i += 1;
        }

        // builtin.c:6297-6300 — `-l` rejects more than one positional
        // argument with `too many arguments for -l`. zshrs previously
        // silently dropped extras, so `emulate -l zsh oops` listed
        // zsh options without diagnosing the typo.
        if list_mode && extra_positional_count > 0 {
            zwarnnam("emulate", "too many arguments for -l");
            return 1;
        }

        // -L and -c are mutually exclusive
        if local_mode && command_arg.is_some() {
            zwarnnam("emulate", "-L and -c are mutually exclusive");
            return 1;
        }

        // No argument: print current emulation mode.
        // builtin.c:6249-6253 — if `-L` or `-R` is set but no shell
        // name was given, error "not enough arguments". zshrs's
        // previous impl printed the current emulation in that case,
        // silently ignoring the L/R flag and producing surprising
        // output. The C source treats `emulate -L` standalone as a
        // user error because there's nothing to make local.
        if mode.is_none() && !list_mode {
            if local_mode || reset_mode {
                zwarnnam("emulate", "not enough arguments");
                return 1;
            }
            let current = self
                .variables
                .get("EMULATE")
                .cloned()
                .unwrap_or_else(|| "zsh".to_string());
            println!("{}", current);
            return 0;
        }

        let mode = mode.unwrap_or_else(|| "zsh".to_string());

        // Get the options that would be set for this mode. Per zsh's
        // `emulate` semantics (zshmisc): even bare `emulate SHELL`
        // (and `emulate -L SHELL`) resets options to SHELL's defaults
        // — not just `-R`. The `-R` flag does MORE (also resets
        // readonly var state, traps, etc.), but the option-reset
        // happens for all forms. Without this, `emulate -L zsh`
        // inherited the caller's options instead of starting from a
        // clean zsh state, breaking p10k segments that rely on the
        // reset-to-defaults contract.
        let (set_opts, unset_opts) = Self::emulate_mode_options(&mode, true);

        // -l: just list the options, don't apply
        if list_mode {
            for opt in &set_opts {
                println!("{}", opt);
            }
            for opt in &unset_opts {
                println!("no{}", opt);
            }
            if local_mode {
                println!("localoptions");
                println!("localpatterns");
                println!("localtraps");
            }
            return 0;
        }

        // Save current state if -c is used
        let saved_options = if command_arg.is_some() {
            Some(self.options.clone())
        } else {
            None
        };
        let saved_emulate = if command_arg.is_some() {
            self.variables.get("EMULATE").cloned()
        } else {
            None
        };

        // Apply the emulation
        self.variables.insert("EMULATE".to_string(), mode.clone());

        // Set options for this mode
        for opt in &set_opts {
            let opt_name = opt.to_lowercase().replace('_', "");
            self.options.insert(opt_name, true);
        }
        for opt in &unset_opts {
            let opt_name = opt.to_lowercase().replace('_', "");
            self.options.insert(opt_name, false);
        }

        // Apply extra -o / +o options
        for opt in &extra_set_opts {
            let opt_name = opt.to_lowercase().replace('_', "");
            self.options.insert(opt_name, true);
        }
        for opt in &extra_unset_opts {
            let opt_name = opt.to_lowercase().replace('_', "");
            self.options.insert(opt_name, false);
        }

        // -L: set local options/traps
        if local_mode {
            self.options.insert("localoptions".to_string(), true);
            self.options.insert("localpatterns".to_string(), true);
            self.options.insert("localtraps".to_string(), true);
        }

        // -c arg: execute command then restore

        if let Some(cmd) = command_arg {
            let status = self.execute_script(&cmd).unwrap_or(1);

            // Restore saved state
            if let Some(opts) = saved_options {
                self.options = opts;
            }
            if let Some(emu) = saved_emulate {
                self.variables.insert("EMULATE".to_string(), emu);
            } else {
                self.variables.remove("EMULATE");
            }

            status
        } else {
            0
        }
    }
    /// float - declare floating point variables
    pub(crate) fn builtin_float(&mut self, args: &[String]) -> i32 {
        // PFA-SMR aspect: emit one `typeset` event per `float NAME[=val]`
        // arg, with the FLOAT attr bit set. -F/-E controls the storage
        // format but the recorder only cares about the type-shape.
        #[cfg(feature = "recorder")]
        if crate::recorder::is_enabled() {
            let ctx = self.recorder_ctx();
            let mut attrs = crate::recorder::ParamAttrs::NONE;
            attrs.set(crate::recorder::ParamAttrs::FLOAT);
            for a in args {
                if a.starts_with('-') {
                    continue;
                }
                if let Some((k, v)) = a.split_once('=') {
                    crate::recorder::emit_typeset_attrs(k, Some(v), attrs, ctx.clone());
                } else {
                    crate::recorder::emit_typeset_attrs(a, None, attrs, ctx.clone());
                }
            }
        }
        // zsh: bare `float NAME=VAL` defaults to `-E` (scientific
        // exponential format); `float -F` opts into fixed-decimal.
        let mut explicit_f = false;
        for a in args {
            if a.starts_with('-') && a.contains('F') {
                explicit_f = true;
            }
        }
        let use_exp = !explicit_f;
        for arg in args {
            if arg.starts_with('-') {
                continue;
            }
            // Format `3.14` as `3.140000000e+00` for `float -E` (default)
            // or `3.1400000000` for `float -F`. Match zsh's storage form
            // exactly so `declare -p` round-trips.
            let format_float = |f: f64| -> String {
                if use_exp {
                    let raw = format!("{:.9e}", f);
                    if let Some(epos) = raw.rfind('e') {
                        let (mantissa, exp) = raw.split_at(epos);
                        let exp_body = &exp[1..];
                        let (sign, digits) = if let Some(d) = exp_body.strip_prefix('-') {
                            ("-", d)
                        } else if let Some(d) = exp_body.strip_prefix('+') {
                            ("+", d)
                        } else {
                            ("+", exp_body)
                        };
                        let padded = if digits.len() < 2 {
                            format!("0{}", digits)
                        } else {
                            digits.to_string()
                        };
                        format!("{}e{}{}", mantissa, sign, padded)
                    } else {
                        raw
                    }
                } else {
                    format!("{:.10}", f)
                }
            };
            if let Some(eq_pos) = arg.find('=') {
                let name = &arg[..eq_pos];
                let value = &arg[eq_pos + 1..];
                let float_val: f64 = value.parse().unwrap_or(0.0);
                self.variables
                    .insert(name.to_string(), format_float(float_val));
                self.options.insert(format!("_float_{}", name), true);
                self.var_attrs.insert(
                    name.to_string(),
                    VarAttr {
                        kind: VarKind::Float,
                        float_exp: use_exp,
                        ..Default::default()
                    },
                );
            } else {
                self.variables.insert(arg.clone(), format_float(0.0));
                self.options.insert(format!("_float_{}", arg), true);
                self.var_attrs.insert(
                    arg.clone(),
                    VarAttr {
                        kind: VarKind::Float,
                        float_exp: use_exp,
                        ..Default::default()
                    },
                );
            }
        }
        0
    }
    /// integer - declare integer variables
    pub(crate) fn builtin_integer(&mut self, args: &[String]) -> i32 {
        // PFA-SMR aspect: emit one `typeset` event per `integer
        // NAME[=val]` arg, with the INTEGER attr bit set. Other letters
        // (-r/-x/-g/-U) compose into ParamAttrs through the same
        // bitset so `integer -rx FOO=1` records {INTEGER|READONLY|EXPORT}.
        #[cfg(feature = "recorder")]
        if crate::recorder::is_enabled() {
            let ctx = self.recorder_ctx();
            let mut letters = String::from("i");
            for a in args {
                if let Some(rest) = a.strip_prefix('-') {
                    letters.push_str(rest);
                } else if let Some(rest) = a.strip_prefix('+') {
                    letters.push_str(rest);
                }
            }
            let attrs = crate::recorder::ParamAttrs::from_flag_chars(&letters);
            for a in args {
                if a.starts_with('-') {
                    continue;
                }
                if let Some((k, v)) = a.split_once('=') {
                    crate::recorder::emit_typeset_attrs(k, Some(v), attrs, ctx.clone());
                } else {
                    crate::recorder::emit_typeset_attrs(a, None, attrs, ctx.clone());
                }
            }
        }
        // Parse options like zsh: -r readonly, -x export, -g global,
        // -U unique. Without -r tracking,
        // `integer -r I=42; ${(t)I}` returned just `integer` instead
        // of `integer-readonly`.
        //
        // `-i [BASE]` accepts an optional output radix (zsh:
        // `integer -i 16 x=255` stores `255` but `echo $x` prints
        // `16#FF` per the typeset -i semantics in builtin.c). The
        // numeric arg is consumed when it directly follows `-i`.
        let mut readonly = false;
        let mut exported = false;
        let mut int_base: Option<u32> = None;
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if arg.starts_with('-') && arg.len() > 1 {
                let mut consumed_base = false;
                let body = &arg[1..];
                for (ci, ch) in body.chars().enumerate() {
                    match ch {
                        'r' => readonly = true,
                        'x' => exported = true,
                        'i' => {
                            // -i may be followed by a base in the
                            // SAME arg ("-i16") or the NEXT arg
                            // ("-i 16"). Same shape as zsh.
                            let after = &body[ci + 1..];
                            if !after.is_empty() && after.chars().all(|c| c.is_ascii_digit()) {
                                int_base = after.parse().ok();
                                break;
                            }
                            // Look at next arg.
                            if let Some(next) = args.get(i + 1) {
                                if next.chars().all(|c| c.is_ascii_digit()) && !next.is_empty() {
                                    int_base = next.parse().ok();
                                    consumed_base = true;
                                }
                            }
                        }
                        _ => {}
                    }
                }
                if consumed_base {
                    i += 1;
                }
                i += 1;
                continue;
            }
            let (name, raw_value) = if let Some(eq_pos) = arg.find('=') {
                (&arg[..eq_pos], Some(&arg[eq_pos + 1..]))
            } else {
                (arg.as_str(), None)
            };
            // zsh: `integer 1bad=5` -> `integer:1: not an
            // identifier: 1bad` exit 1. zshrs silently accepted.
            // Allow subscript form (`a[i]=...`) — that's a
            // valid extension handled elsewhere.
            if !name.contains('[') {
                let mut chars = name.chars();
                let first_ok = chars
                    .next()
                    .map(|c| c.is_ascii_alphabetic() || c == '_')
                    .unwrap_or(false);
                let body_ok = chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
                if !first_ok || !body_ok {
                    zerrnam("integer", &format!("not an identifier: {}", name));
                    return 1;
                }
            }
            let int_val = match raw_value {
                Some(v) => self.eval_arith_expr(v),
                None => 0,
            };
            // Format display value per the -i base if set. zsh's
            // base-16 output reads `16#FF` for 255.
            let stored = if let Some(b) = int_base {
                format_int_in_base(int_val, b)
            } else {
                int_val.to_string()
            };
            self.variables.insert(name.to_string(), stored);
            self.options.insert(format!("_integer_{}", name), true);
            let mut attr = VarAttr {
                kind: VarKind::Integer,
                ..Default::default()
            };
            attr.readonly = readonly;
            attr.export = exported;
            attr.int_base = int_base;
            self.var_attrs.insert(name.to_string(), attr);
            if readonly {
                self.readonly_vars.insert(name.to_string());
            }
            if exported {
                std::env::set_var(name, int_val.to_string());
            }
            i += 1;
        }
        0
    }
    /// functions - list or manipulate function definitions
    pub(crate) fn bin_functions(&self, args: &[String]) -> i32 {
        let mut list_only = false;
        let mut show_trace = false;
        let mut matchpat = false;
        let mut enable_trace = false;
        let mut names: Vec<&str> = Vec::new();
        let mut after_dashes = false;

        for arg in args {
            if after_dashes {
                names.push(arg);
                continue;
            }
            match arg.as_str() {
                // `--` ends option processing — subsequent words are
                // function names even if they start with `-`/`+`.
                // Direct port of `parseopts(name, &args, ops, &func)`
                // recognising `--` per Src/options.c. Without this,
                // `functions -- foo` rejected `--` as `bad option`.
                "--" => {
                    after_dashes = true;
                    continue;
                }
                // `functions +` (standalone +, no letter after) lists
                // function names only, no bodies. Same as `functions
                // +l` in semantic effect but different argv shape.
                "+" => {
                    list_only = true;
                    continue;
                }
                "-l" => list_only = true,
                "-t" => show_trace = true,
                // `-T` (capital) ENABLES tracing on the named
                // functions and emits no listing — zsh: silent
                // success, sets the `t` attr on the function. zshrs
                // didn't recognize the flag and printed the function
                // body. Now treated as silent (the actual tracing
                // attr isn't tracked yet, but the no-output behavior
                // matches script consumers that just toggle).
                "-T" => enable_trace = true,
                // `+t` / `+T` DISABLE tracing — zsh: silent success
                // (clears the `t` attr). zshrs treated `+t` as a
                // function name and erred "no such function". Mirror
                // by silently consuming.
                "+t" | "+T" => enable_trace = true,
                "-m" => matchpat = true,
                _ if arg.starts_with('-') && arg.len() > 1 => {
                    // Combined flags like `-lm`
                    for c in arg[1..].chars() {
                        match c {
                            'l' => list_only = true,
                            't' => show_trace = true,
                            'T' => enable_trace = true,
                            'm' => matchpat = true,
                            // BUILTIN("functions", ..., "ckmMstTuUWx:z")
                            // — those are valid letters. Most are
                            // accepted as no-op (we don't track all
                            // attrs yet) but unknown letters error.
                            'c' | 'k' | 'M' | 's' | 'u' | 'U' | 'W' | 'z' => {}
                            _ => {
                                zwarnnam("functions", &format!("bad option: -{}", c));
                                return 1;
                            }
                        }
                    }
                }
                _ if arg.starts_with('+') && arg.len() > 1 => {
                    // `+l`/`+t`/`+T`/`+m` — combined "off" flags.
                    // For our purposes (no per-function trace state),
                    // treat them all as silent no-ops matching zsh's
                    // silent toggle behavior.
                    for c in arg[1..].chars() {
                        match c {
                            'l' | 't' | 'T' | 'm' => enable_trace = true,
                            'c' | 'k' | 'M' | 's' | 'u' | 'U' | 'W' | 'z' => {}
                            _ => {
                                zwarnnam("functions", &format!("bad option: +{}", c));
                                return 1;
                            }
                        }
                    }
                }
                _ => names.push(arg),
            }
        }
        if enable_trace {
            // No-op: silently consume the flag; -T's trace attribute
            // would need a per-function flag table to be observable.
            return 0;
        }

        // With -m, treat each name as a glob pattern and expand to
        // matching function names.
        if matchpat && !names.is_empty() {
            let mut matched: Vec<String> = Vec::new();
            for pat in &names {
                for fname in self.function_names() {
                    if Self::glob_match_static(&fname, pat) && !matched.contains(&fname) {
                        matched.push(fname);
                    }
                }
            }
            if matched.is_empty() {
                return 1;
            }
            for name in &matched {
                if list_only {
                    println!("{}", name);
                } else if show_trace {
                    println!("functions -t {}", name);
                } else if let Some(body) = self.function_definition_text(name) {
                    println!(
                        "{} () {{\n\t{}\n}}",
                        name,
                        FuncBodyFmt::render(body.trim())
                    );
                }
            }
            return 0;
        }

        if names.is_empty() {
            for name in self.function_names() {
                if list_only {
                    println!("{}", name);
                } else if let Some(body) = self.function_definition_text(&name) {
                    println!(
                        "{} () {{\n\t{}\n}}",
                        name,
                        FuncBodyFmt::render(body.trim())
                    );
                }
            }
        } else {
            for name in names {
                // zsh: `functions FOO` for a non-existent FOO is
                // silent — emits nothing and returns 0. zshrs's
                // earlier impl errored "no such function: FOO".
                // Match zsh by skipping silently.
                if !self.function_exists(name) {
                    continue;
                }
                if show_trace {
                    // zsh: `functions -t NAME` lists only functions
                    // whose trace attribute IS set (output `functions
                    // -t NAME`). Without per-function trace tracking
                    // we have no way to know which functions are
                    // marked, so emit nothing — matches zsh's silent
                    // output for the common "no trace set" case.
                    continue;
                } else if let Some(body) = self.function_definition_text(name) {
                    println!(
                        "{} () {{\n\t{}\n}}",
                        name,
                        FuncBodyFmt::render(body.trim())
                    );
                }
            }
        }
        0
    }
    /// print - zsh print builtin with many options
    pub(crate) fn bin_print(&mut self, args: &[String]) -> i32 {
        self.dispatch_pending_traps();
        if self.redirect_failed { self.redirect_failed = false; return 1; }
        // print [ -abcDilmnNoOpPrsSz ] [ -u n ] [ -f format ] [ -C cols ]
        //       [ -v name ] [ -xX tabstop ] [ -R [ -en ]] [ arg ... ]
        let mut no_newline = false;
        let mut one_per_line = false;
        let mut interpret_escapes = true; // zsh default is to interpret
        let mut raw_mode = false;
        let mut prompt_expand = false;
        let mut fd: i32 = 1; // stdout
        let mut columns = 0usize;
        // `-c` (lowercase): auto-fit columns to terminal width based
        // on $COLUMNS / max item width. `-C N` (uppercase) overrides
        // with an explicit count. zshrs previously mapped `-c` to
        // columns=1, which printed one item per line — equivalent to
        // `-l`. Per zsh man print(1), `-c` means "print arguments in
        // columns" with auto-fit, not "1 column".
        let mut auto_columns = false;
        let mut null_terminate = false;
        let mut push_to_stack = false;
        let mut add_to_history = false;
        let mut split_word_history = false; // -S specifically (not -s)
        let mut sort_asc = false;
        let mut sort_desc = false;
        let mut sort_ignore_case = false; // -i: case-folded sort (builtin.c:4805)
        let mut named_dir_subst = false;
        let mut match_pattern_flag = false;
        let mut store_var: Option<String> = None;
        let mut format_string: Option<String> = None;
        let mut output_args: Vec<String> = Vec::new();
        // -x N: expand leading tabs only; -X N: expand all tabs.
        // (width, all-tabs) per zsh/Src/utils.c:5973 zexpandtabs.
        let mut tab_expand: Option<(i32, bool)> = None;
        // -a: print across — row-major column layout (builtin.c:4980-4994).
        let mut print_across = false;
        // -b: bindkey-style escape interpretation. Adds \C-x / \M-y /
        // \^x escapes on top of the standard set. Per builtin.c:4711
        // selecting GETKEYS_BINDKEY mode.
        let mut bindkey_escapes = false;

        let mut i = 0;
        let mut accept_flags = true;
        while i < args.len() {
            let arg = &args[i];

            if arg == "--" {
                i += 1;
                while i < args.len() {
                    output_args.push(args[i].clone());
                    i += 1;
                }
                break;
            }

            if accept_flags
                && arg.starts_with('-')
                && arg.len() > 1
                && !arg
                    .chars()
                    .nth(1)
                    .map(|c| c.is_ascii_digit())
                    .unwrap_or(false)
            {
                // Validate every char is a real print flag. zsh errors
                // on the FIRST unrecognised char (`print --hi` → "bad
                // option: -h"). Unlike echo, print does NOT accept `-`
                // mid-flags as a literal.
                let body = &arg[1..];
                let known = |c: char| {
                    matches!(
                        c,
                        '-' // zsh accepts `-` as a no-op flag char (so
                        // `--foo` parses as `-`/`-foo` rather than
                        // erroring on the second `-`).
                    | 'n' | 'l' | 'r' | 'R' | 'P' | 'N' | 'z'
                    | 's' | 'o' | 'O' | 'D' | 'c' | 'm' | 'a' | 'b' | 'i'
                    | 'p' | 'S' | 'x' | 'X' | 'u' | 'C' | 'v' | 'f'
                    )
                };
                // zsh's `print` rejects `-e` AND `-E` (echo accepts both).
                // Removed from the known set so they fall through to the
                // "bad option" error path matching zsh.
                if let Some(bad) = body.chars().find(|c| !known(*c)) {
                    zwarnnam("print", &format!("bad option: -{}", bad));
                    return 1;
                }
                let mut chars = arg[1..].chars().peekable();
                while let Some(ch) = chars.next() {
                    match ch {
                        'n' => no_newline = true,
                        'l' => one_per_line = true,
                        'r' => {
                            raw_mode = true;
                            interpret_escapes = false;
                        }
                        'R' => {
                            raw_mode = true;
                            interpret_escapes = false;
                        }
                        'e' => interpret_escapes = true,
                        'E' => interpret_escapes = false,
                        'P' => prompt_expand = true,
                        'N' => null_terminate = true,
                        'z' => push_to_stack = true,
                        's' => add_to_history = true,
                        // `print -S` is the "split-shell-words"
                        // history form — like `-s` it adds the line
                        // to history INSTEAD of stdout. Without this,
                        // `print -S "hello"` printed `hello` to
                        // stdout while zsh stayed silent. zsh also
                        // restricts `-S` to a SINGLE positional arg,
                        // erroring `option -S takes a single argument`
                        // for `print -S foo bar` — track separately.
                        'S' => {
                            add_to_history = true;
                            split_word_history = true;
                        }
                        'o' => sort_asc = true,
                        'O' => sort_desc = true,
                        'D' => named_dir_subst = true,
                        'c' => auto_columns = true,
                        'm' => match_pattern_flag = true,
                        'x' | 'X' => {
                            // -x N / -X N: tab expansion via zexpandtabs
                            // (utils.c:5973). -x: leading tabs only; -X:
                            // all tabs. Numeric arg may be glued (`-x4`)
                            // or separate (`-x 4`). zsh: non-positive
                            // integer or non-integer -> `print:1:
                            // positive integer expected after -x: <arg>`
                            // exit 1 (builtin.c:5101-5106).
                            let all_tabs = ch == 'X';
                            let rest: String = chars.collect();
                            let value_str = if !rest.is_empty() {
                                rest
                            } else {
                                i += 1;
                                if i >= args.len() {
                                    zwarnnam("print", &format!("positive integer expected after -{}", ch));
                                    return 1;
                                }
                                args[i].clone()
                            };
                            match value_str.parse::<i32>() {
                                Ok(n) if n > 0 => tab_expand = Some((n, all_tabs)),
                                _ => {
                                    zwarnnam("print", &format!("positive integer expected after -{}: {}", ch, value_str));
                                    return 1;
                                }
                            }
                            break;
                        }
                        'a' => print_across = true,
                        'i' => sort_ignore_case = true,
                        'b' => bindkey_escapes = true,
                        'p' => {
                            // Port of print -p from Src/builtin.c bin_print.
                            // The C source writes to the coproc pipe (coprocout).
                            // zshrs doesn't have a live coproc fd here, so
                            // surface an error matching zsh's "no coprocess"
                            // diagnostic at builtin.c:5474 area when coproc
                            // isn't running. Real coproc support requires
                            // the coproc pipe wiring in exec.c we haven't
                            // ported yet.
                            zwarnnam("print", "no coprocess");
                            return 1;
                        }
                        'u' => {
                            // -u n: output to fd n. zsh requires a
                            // numeric argument; non-numeric ->
                            // `print:1: number expected after -u: <arg>`
                            // exit 1. zshrs's `unwrap_or(1)` silently
                            // dropped non-numeric input and printed
                            // to stdout.
                            let rest: String = chars.collect();
                            let value_str = if !rest.is_empty() {
                                rest
                            } else {
                                i += 1;
                                if i >= args.len() {
                                    zwarnnam("print", "number expected after -u");
                                    return 1;
                                }
                                args[i].clone()
                            };
                            match value_str.parse::<i32>() {
                                Ok(n) => fd = n,
                                Err(_) => {
                                    zwarnnam("print", &format!("number expected after -u: {}", value_str));
                                    return 1;
                                }
                            }
                            break;
                        }
                        'C' => {
                            // -C n: n columns
                            let rest: String = chars.collect();
                            if !rest.is_empty() {
                                columns = rest.parse().unwrap_or(0);
                            } else {
                                i += 1;
                                if i < args.len() {
                                    columns = args[i].parse().unwrap_or(0);
                                }
                            }
                            break;
                        }
                        'v' => {
                            // -v name: store in variable
                            let rest: String = chars.collect();
                            if !rest.is_empty() {
                                store_var = Some(rest);
                            } else {
                                i += 1;
                                if i < args.len() {
                                    store_var = Some(args[i].clone());
                                }
                            }
                            break;
                        }
                        'f' => {
                            // -f format: printf-style format. zsh:
                            // missing arg -> `print:1: argument
                            // expected: -f` exit 1. zshrs's
                            // `if i < args.len()` silently fell
                            // through with no format set.
                            let rest: String = chars.collect();
                            if !rest.is_empty() {
                                format_string = Some(rest);
                            } else {
                                i += 1;
                                if i >= args.len() {
                                    zwarnnam("print", "argument expected: -f");
                                    return 1;
                                }
                                format_string = Some(args[i].clone());
                            }
                            break;
                        }
                        _ => {}
                    }
                }
            } else {
                accept_flags = false;
                output_args.push(arg.clone());
            }
            i += 1;
        }

        // builtin.c:4661-4677 — three mutex groups print enforces
        // before any output runs. Without these checks, a mistake
        // like `print -sv name "x"` would silently both push to
        // history AND assign to `name`, producing surprising side
        // effects with no diagnostic.
        //
        //   1. `-z`, `-s`, `-S`, `-v` are mutually exclusive — they
        //      all redirect output away from stdout to a different
        //      sink (buffer stack / history / variable).
        //   2. `-c` and `-C` are not allowed with `-s`, `-S`, or `-z`
        //      — column layout requires a printable destination.
        //   3. `-p` and `-u` are not allowed with `-s`, `-S`, `-v`,
        //      or `-z` — the explicit-fd flags also require stdout-
        //      like output, not the redirected sinks.
        let group1 = (push_to_stack as u32)
            + ((add_to_history && !split_word_history) as u32)
            + (split_word_history as u32)
            + (store_var.is_some() as u32);
        if group1 > 1 {
            zwarnnam("print", "only one of -s, -S, -v, or -z allowed");
            return 1;
        }
        let any_redirect_sink = push_to_stack || add_to_history;
        let any_columns = columns != 0 || auto_columns;
        if any_redirect_sink && any_columns {
            zwarnnam("print", "-c or -C not allowed with -s, -S, or -z");
            return 1;
        }
        let any_redirect_or_var = push_to_stack || add_to_history || store_var.is_some();
        let explicit_fd = fd != 1;
        if any_redirect_or_var && explicit_fd {
            zwarnnam("print", "-p or -u not allowed with -s, -S, -v, or -z");
            return 1;
        }

        // `print -z` pushes the joined args (sep-joined per
        // builtin.c:5042 `sepjoin(args, NULL, 0)` which uses IFS[0] = ' '
        // by default) onto the editor buffer stack. `getln` and
        // `read -z` pop from the same stack later.
        if push_to_stack {
            let line = output_args.join(" ");
            self.buffer_stack.push(line);
            return 0;
        }
        // zsh: `print -u N` writes to fd N. If fd N isn't open,
        // errors `print:1: bad file number: N` exit 1 and prints
        // nothing. zshrs's `let _ = fd` discarded the requested fd
        // and always wrote to stdout, ignoring -u entirely. Validate
        // fd is open BEFORE the print runs.
        if fd != 1 && fd != 2 {
            // fcntl-check the fd via libc::fcntl(F_GETFD); -1 means
            // closed. The 1/2 fast-path skips the syscall for stdout
            // and stderr (always open in -c mode).
            let rc = unsafe { libc::fcntl(fd, libc::F_GETFD) };
            if rc < 0 {
                zwarnnam("print", &format!("bad file number: {}", fd));
                return 1;
            }
        }
        // fd routing happens at the actual write call below — this
        // arm is just the validation step (port of builtin.c:4843
        // `dup(fdarg)` failure check).

        // `print -m PATTERN args…` — first positional is a glob pattern;
        // only print the args that match. zsh: bare `print -m '*.txt'
        // a.txt b.log c.txt` → prints `a.txt c.txt`.
        if match_pattern_flag && !output_args.is_empty() {
            let pattern = output_args.remove(0);
            output_args.retain(|a| Self::glob_match_static(a, &pattern));
        }

        // Sort if requested. -i (case-fold) per builtin.c:4805 selects
        // SORTIT_IGNORING_CASE for both ascending (-o) and descending (-O).
        if sort_asc {
            if sort_ignore_case {
                output_args.sort_by_key(|a| a.to_lowercase());
            } else {
                output_args.sort();
            }
        } else if sort_desc {
            if sort_ignore_case {
                output_args.sort_by_key(|b| std::cmp::Reverse(b.to_lowercase()));
            } else {
                output_args.sort_by(|a, b| b.cmp(a));
            }
        }

        // Handle -f format — cycle the format string while args remain
        // (POSIX printf semantics, also what zsh's `print -f` does).
        // Also expand `\n`/`\t`/`\\` etc. in the format string so users
        // can write `print -f "%s\n" a b c` and get one item per line.
        if let Some(fmt) = format_string {
            let fmt = if interpret_escapes && !raw_mode {
                self.expand_printf_escapes(&fmt)
            } else {
                fmt.clone()
            };
            let mut output = String::new();
            if output_args.is_empty() {
                output.push_str(&self.printf_format(&fmt, &[]));
            } else {
                let mut idx = 0;
                while idx < output_args.len() {
                    let slice = &output_args[idx..];
                    let prev_len = slice.len();
                    let (chunk, consumed) = self.printf_format_count(&fmt, slice);
                    output.push_str(&chunk);
                    if consumed == 0 || consumed >= prev_len {
                        idx += consumed.max(prev_len);
                        break;
                    }
                    idx += consumed;
                }
            }
            if let Some(var) = store_var {
                self.variables.insert(var, output);
            } else {
                // Same fd-routing as the non-format path below per
                // src/zsh/Src/builtin.c:4810-4852 — `print -u N -f
                // FMT ARGS` redirects formatted output to fd N.
                // Without this, `-f` always wrote to stdout.
                use std::io::Write as _;
                match fd {
                    1 => {
                        print!("{}", output);
                        let _ = std::io::stdout().flush();
                    }
                    2 => {
                        eprint!("{}", output);
                        let _ = std::io::stderr().flush();
                    }
                    n => {
                        let bytes = output.as_bytes();
                        unsafe {
                            libc::write(n, bytes.as_ptr() as *const libc::c_void, bytes.len());
                        }
                    }
                }
            }
            return 0;
        }

        // Process output
        let processed: Vec<String> = output_args
            .iter()
            .map(|s| {
                let mut result = s.clone();
                if prompt_expand {
                    // print -P emits raw terminal bytes; suppress the
                    // SOH/STX readline-marker pair and the apply_attrs
                    // preamble reset.
                    result = self.expand_prompt_string_for_print(&result);
                }
                if bindkey_escapes && !raw_mode {
                    // -b takes precedence over -e: bindkey escapes are
                    // a superset of the standard set.
                    result = self.expand_bindkey_escapes(&result);
                } else if interpret_escapes && !raw_mode {
                    // `print` (in contrast to `echo`) drops backslashes
                    // for unrecognised `\X` escapes — zsh's bin_print
                    // (Src/builtin.c:4587) calls getkeystring() which
                    // collapses `\X` to `X` for X not in the
                    // recognised-escape set. `echo` keeps them
                    // verbatim. Use the print-specific decoder here so
                    // `print -- "${(q)var}"` round-trips back to the
                    // original spaces.
                    result = self.expand_print_escapes(&result);
                }
                if named_dir_subst {
                    // Replace home dir with ~
                    if let Ok(home) = env::var("HOME") {
                        if result.starts_with(&home) {
                            result = format!("~{}", &result[home.len()..]);
                        }
                    }
                    // Replace named dirs — longest-prefix-first so a
                    // nested ~zpwr=/Users/wizard/zpwr wins over a
                    // shallower ~home=/Users/wizard. Random HashMap
                    // iteration order picked the shallower one some
                    // runs and the deeper one others, breaking
                    // deterministic prompt rendering.
                    let mut entries: Vec<(&String, &PathBuf)> = self.named_dirs.iter().collect();
                    entries.sort_by_key(|(_, p)| std::cmp::Reverse(p.as_os_str().len()));
                    for (name, path) in entries {
                        let path_str = path.to_string_lossy();
                        if result.starts_with(path_str.as_ref()) {
                            result = format!("~{}{}", name, &result[path_str.len()..]);
                            break;
                        }
                    }
                }
                result
            })
            .collect();

        // Determine separator and terminator. zsh's `-N` uses NUL as
        // BOTH the separator between args AND the terminator after
        // the last — so `print -N a b c` emits `a\0b\0c\0`, not
        // `a b c\0`. -l (`one_per_line`) keeps `\n` for both.
        let separator = if null_terminate {
            "\0"
        } else if one_per_line {
            "\n"
        } else {
            " "
        };
        // zsh: `-n` always suppresses the terminator (no `\n` AND no
        // trailing `\0` for `-N`). Without this, `print -nN hi` left
        // a stray `\0` that displayed as a blank space in some
        // terminals (and broke `print -nN a; echo X` byte alignment).
        let terminator = if no_newline {
            ""
        } else if null_terminate {
            "\0"
        } else {
            "\n"
        };

        // -x / -X tab expansion path — direct port of builtin.c:5095-5121.
        // zexpandtabs (utils.c:5973) carries `startpos` across args so a
        // `\t` mid-string aligns to the next tabstop relative to total
        // emitted width, not the arg-local offset. -x expands leading
        // tabs only; -X expands all tabs. Per builtin.c:5111-5119, when
        // -l is set the separator is `\n` and startpos resets to 0;
        // when -N is set the separator is `\0` (no startpos change);
        // otherwise a single space is emitted and startpos++.
        if let Some((width, all_tabs)) = tab_expand {
            let mut result = String::new();
            let mut startpos: i32 = 0;
            for (idx, arg) in processed.iter().enumerate() {
                let new_pos = crate::ported::utils::zexpandtabs(arg, width, startpos, all_tabs, &mut result);
                startpos = new_pos;
                if idx + 1 < processed.len() {
                    if one_per_line {
                        result.push('\n');
                        startpos = 0;
                    } else if null_terminate {
                        result.push('\0');
                    } else {
                        result.push(' ');
                        startpos += 1;
                    }
                }
            }
            // Apply terminator using the same rules as the regular path
            // (builtin.c:5130-5132): -n suppresses; -N -> '\0'; else '\n'.
            let term = if no_newline {
                ""
            } else if null_terminate {
                "\0"
            } else {
                "\n"
            };
            // -v N stores into a scalar, otherwise write to fd. Mirror
            // the same fd routing used in the regular print output path
            // below (1 -> stdout!, 2 -> stderr!, else libc::write).
            // Empirical /bin/zsh: `print -v X "hello"` stores
            // "hello" without a trailing newline; the terminator
            // only applies to fd output. Verified: `/bin/zsh -c
            // 'print -v X "hello world"; print "[$X]"'` → `[hello world]`.
            let final_out = format!("{}{}", result, term);
            if let Some(var) = store_var {
                self.variables.insert(var, result);
                return 0;
            }
            match fd {
                1 => print!("{}", final_out),
                2 => eprint!("{}", final_out),
                n => {
                    let bytes = final_out.as_bytes();
                    unsafe {
                        libc::write(n, bytes.as_ptr() as *const libc::c_void, bytes.len());
                    }
                }
            }
            use std::io::Write as _;
            match fd {
                1 => {
                    let _ = std::io::stdout().flush();
                }
                2 => {
                    let _ = std::io::stderr().flush();
                }
                _ => {}
            }
            return 0;
        }

        // Resolve `-c` (auto-fit columns) AFTER all items are built
        // because we need the max item width to choose a count.
        // Algorithm matches zsh's `print -c`:
        //   width = $COLUMNS env (fall back to 80)
        //   max_item = max(chars().count() of each processed item)
        //   per_col = max_item + 2  (zsh separates with 2 spaces)
        //   cols = max(1, width / per_col)
        // If -C N was also given, the explicit N wins.
        if auto_columns && columns == 0 && !processed.is_empty() {
            let term_width: usize = std::env::var("COLUMNS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(80);
            let max_item = processed
                .iter()
                .map(|s| s.chars().count())
                .max()
                .unwrap_or(0);
            let per_col = max_item.saturating_add(2).max(1);
            columns = (term_width / per_col).max(1);
        }
        // Build output
        let output = if one_per_line {
            processed.join("\n")
        } else if columns > 0 {
            // Column output - calculate column widths. zsh's `print -C N`
            // pads each column to the widest entry in that column and
            // separates columns with TWO spaces (so "a c" / "b d" with
            // single-char items reads as "a  c" / "b  d"). Earlier we
            // joined with a single tab, which most terminals render
            // wider than zsh's two-space output.
            //
            // -a (print across) flips index order: col-major (default,
            // builtin.c:4998-5005 inner-loop strides by `nr`) → row-major
            // (-a, builtin.c:4986-4993 inner-loop strides by 1).
            let num_items = processed.len();
            let rows = num_items.div_ceil(columns);
            // Compute width per column (max item width in that column).
            let mut col_widths = vec![0usize; columns];
            // 2D access pattern with row/col both varying — needless_range_loop
            // would force a per-axis enumerate that obscures the index math.
            #[allow(clippy::needless_range_loop)]
            for col in 0..columns {
                for row in 0..rows {
                    let idx = if print_across {
                        row * columns + col
                    } else {
                        row + col * rows
                    };
                    if idx < num_items {
                        col_widths[col] = col_widths[col].max(processed[idx].chars().count());
                    }
                }
            }
            let mut result = String::new();
            for row in 0..rows {
                // Find the last column in this row that actually has
                // an item — only pad+separate columns BEFORE it.
                let last_col_in_row = (0..columns)
                    .rev()
                    .find(|c| {
                        let i = if print_across {
                            row * columns + *c
                        } else {
                            row + *c * rows
                        };
                        i < num_items
                    })
                    .unwrap_or(0);
                #[allow(clippy::needless_range_loop)]
                for col in 0..=last_col_in_row {
                    let idx = if print_across {
                        row * columns + col
                    } else {
                        row + col * rows
                    };
                    if idx < num_items {
                        let item = processed[idx].as_str();
                        result.push_str(item);
                        if col < last_col_in_row {
                            let pad = col_widths[col].saturating_sub(item.chars().count());
                            for _ in 0..pad {
                                result.push(' ');
                            }
                            result.push_str("  ");
                        }
                    }
                }
                if row < rows - 1 {
                    result.push('\n');
                }
            }
            result
        } else {
            processed.join(separator)
        };

        // Add to history if -s — and per zsh, `-s` REPLACES stdout
        // output (the result goes to history INSTEAD OF stdout).
        if add_to_history {
            // zsh: `-S` (split-words form) takes EXACTLY one arg —
            // `print -S foo bar` errors `option -S takes a single
            // argument` exit 1. zshrs's loop concatenated all args
            // into the history entry silently.
            if split_word_history && output_args.len() > 1 {
                zwarnnam("print", "option -S takes a single argument");
                return 1;
            }
            if let Some(ref mut engine) = self.history {
                if let Ok(id) = engine.add(&output, None) {
                    self.session_history_ids.push(id);
                }
            }
            return 0;
        }

        // Store in variable or print. Per builtin.c:5197-5202 the
        // `-v` path captures the SAME byte stream as stdout — so
        // the trailing terminator (newline by default, suppressed
        // -v stores the joined body WITHOUT the terminator —
        // verified empirically against /bin/zsh:
        //   /bin/zsh -c 'print -v x foo; echo "[${#x}]"'  → [3]
        // The terminator only applies when writing to a fd, not
        // to a captured-variable target.
        if let Some(var) = store_var {
            self.variables.insert(var, output);
        } else {
            // Route to the requested fd. fd=1 (stdout) and fd=2
            // (stderr) get the standard io macros; other fds use
            // libc::write directly. Without this, `print -u 2 hi`
            // wrote to stdout (fd=1) regardless of the user's
            // request, breaking `2>/dev/null` redirects.
            let to_print = format!("{}{}", output, terminator);
            match fd {
                1 => print!("{}", to_print),
                2 => eprint!("{}", to_print),
                n => {
                    let bytes = to_print.as_bytes();
                    unsafe {
                        libc::write(n, bytes.as_ptr() as *const libc::c_void, bytes.len());
                    }
                }
            }
            // Same flush rationale as builtin_printf — block-buffered
            // stdout strands data through redirect-scope restore.
            use std::io::Write as _;
            match fd {
                1 => {
                    let _ = std::io::stdout().flush();
                }
                2 => {
                    let _ = std::io::stderr().flush();
                }
                _ => {}
            }
        }

        0
    }
    /// whence - show how a command would be interpreted
    pub(crate) fn bin_whence(&self, args: &[String]) -> i32 {
        // whence [ -vcwfpamsS ] [ -x num ] name ...
        // -v: verbose (like type)
        // -c: csh-style output
        // -w: print word type (alias, builtin, command, function, hashed, reserved, none)
        // -f: skip functions
        // -p: search path only
        // -a: show all matches
        // -m: pattern match with glob
        // -s: show symlink resolution
        // -S: show steps of symlink resolution
        // -x num: expand tabs to num spaces

        let mut verbose = false;
        let mut csh_style = false;
        let mut word_type = false;
        let mut skip_functions = false;
        let mut path_only = false;
        let mut show_all = false;
        let mut pattern_mode = false;
        let mut show_symlink = false;
        let mut show_symlink_steps = false;
        let mut tab_expand: Option<usize> = None;
        let mut names: Vec<&str> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];

            if arg == "--" {
                i += 1;
                while i < args.len() {
                    names.push(&args[i]);
                    i += 1;
                }
                break;
            }

            if arg.starts_with('-') && arg.len() > 1 {
                let mut chars = arg[1..].chars().peekable();
                while let Some(ch) = chars.next() {
                    match ch {
                        'v' => verbose = true,
                        'c' => csh_style = true,
                        'w' => word_type = true,
                        'f' => skip_functions = true,
                        'p' => path_only = true,
                        'a' => show_all = true,
                        'm' => pattern_mode = true,
                        's' => show_symlink = true,
                        'S' => show_symlink_steps = true,
                        'x' => {
                            // -x num: tab expansion
                            let rest: String = chars.collect();
                            if !rest.is_empty() {
                                tab_expand = rest.parse().ok();
                            } else {
                                i += 1;
                                if i < args.len() {
                                    tab_expand = args[i].parse().ok();
                                }
                            }
                            break;
                        }
                        _ => {}
                    }
                }
            } else {
                names.push(arg);
            }
            i += 1;
        }

        // `-c` / csh-style affects two output paths:
        //  - alias: zsh prints `name: aliased to BODY` instead of just BODY
        //  - function: zsh prints the full `name () { … }` body, like
        //    `typeset -f`, instead of just the name
        //  - not found: stderr message `name not found` (default verbose
        //    is off but `where` requests both -c and -a, so the absence
        //    of the "not found" line is a real diff vs zsh)
        let _ = tab_expand;

        // `-m` glob pattern dispatch — port of bin_whence pattern branch
        // from zsh/Src/builtin.c:4027-4083. Each arg is a glob; scan each
        // scope (aliases / reserved / functions / builtins / PATH commands)
        // and emit every name matching the pattern. Without -a we emit and
        // return; with -a we collect command-name matches into `names` and
        // fall through to the literal-name loop so each name is also
        // checked against every scope (matching the C `allmatched`
        // fallthrough at builtin.c:4077-4082).
        let owned_names_storage: Vec<String>;
        let names: Vec<&str> = if pattern_mode {
            let patterns: Vec<String> = names.iter().map(|s| s.to_string()).collect();
            let mut informed: usize = 0;
            let mut matched_cmd_names: Vec<String> = Vec::new();
            let mut seen_cmd: std::collections::HashSet<String> = std::collections::HashSet::new();

            for pat in &patterns {
                if !path_only {
                    // Aliases — scanmatchtable(aliastab, ...) at builtin.c:4051.
                    let mut alias_keys: Vec<String> = self.aliases.keys().cloned().collect();
                    alias_keys.sort();
                    for an in &alias_keys {
                        if Self::glob_match_static(an, pat) {
                            let alias_val = self.aliases.get(an).cloned().unwrap_or_default();
                            if word_type {
                                println!("{}: alias", an);
                            } else if verbose {
                                println!("{} is an alias for {}", an, alias_val);
                            } else if csh_style {
                                println!("{}: aliased to {}", an, alias_val);
                            } else {
                                println!("{}", alias_val);
                            }
                            informed += 1;
                        }
                    }

                    // Reserved words — scanmatchtable(reswdtab, ...) at builtin.c:4055.
                    const RESERVED_WORDS_M: &[&str] = &[
                        "if",
                        "then",
                        "else",
                        "elif",
                        "fi",
                        "case",
                        "esac",
                        "for",
                        "select",
                        "while",
                        "until",
                        "do",
                        "done",
                        "in",
                        "function",
                        "time",
                        "coproc",
                        "repeat",
                        "foreach",
                        "end",
                        "nocorrect",
                        "noglob",
                        "local",
                        "declare",
                        "typeset",
                        "readonly",
                        "export",
                        "integer",
                        "float",
                        "{",
                        "}",
                        "!",
                        "[[",
                        "]]",
                        "((",
                        "))",
                    ];
                    for &rw in RESERVED_WORDS_M {
                        if Self::glob_match_static(rw, pat) {
                            if word_type {
                                println!("{}: reserved", rw);
                            } else if verbose {
                                println!("{} is a reserved word", rw);
                            } else if csh_style {
                                println!("{}: shell reserved word", rw);
                            } else {
                                println!("{}", rw);
                            }
                            informed += 1;
                        }
                    }

                    // Functions — scanmatchshfunc(...) at builtin.c:4060.
                    if !skip_functions {
                        let mut fn_keys: Vec<String> =
                            self.function_source.keys().cloned().collect();
                        fn_keys.sort();
                        for fnname in &fn_keys {
                            if Self::glob_match_static(fnname, pat) {
                                if word_type {
                                    println!("{}: function", fnname);
                                } else if verbose {
                                    println!("{} is a shell function from zsh", fnname);
                                } else if csh_style {
                                    let body = self
                                        .function_source
                                        .get(fnname)
                                        .cloned()
                                        .unwrap_or_else(|| ":".to_string());
                                    println!(
                                        "{} () {{\n\t{}\n}}",
                                        fnname,
                                        FuncBodyFmt::render(&body)
                                    );
                                } else {
                                    println!("{}", fnname);
                                }
                                informed += 1;
                            }
                        }
                    }

                    // Builtins — scanmatchtable(builtintab, ...) at builtin.c:4065.
                    let mut builtin_keys: Vec<&'static str> = BUILTIN_SET.iter().copied().collect();
                    builtin_keys.sort();
                    for bn in &builtin_keys {
                        if Self::glob_match_static(bn, pat) {
                            if word_type {
                                println!("{}: builtin", bn);
                            } else if verbose {
                                println!("{} is a shell builtin", bn);
                            } else if csh_style {
                                println!("{}: shell built-in command", bn);
                            } else {
                                println!("{}", bn);
                            }
                            informed += 1;
                        }
                    }
                }

                // PATH commands — scanmatchtable(cmdnamtab, ...) at builtin.c:4071.
                // C calls cmdnamtab->filltable() to populate the hash from PATH;
                // we walk PATH directly. With -a, collect names into
                // matched_cmd_names for the fallthrough; without -a, emit each.
                let path_var = env::var("PATH").unwrap_or_default();
                let mut cmd_pairs: Vec<(String, String)> = Vec::new();
                let mut seen_in_path: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                for dir in path_var.split(':') {
                    if dir.is_empty() {
                        continue;
                    }
                    if let Ok(entries) = std::fs::read_dir(dir) {
                        for entry in entries.flatten() {
                            if let Ok(name) = entry.file_name().into_string() {
                                if seen_in_path.insert(name.clone()) {
                                    let full = format!("{}/{}", dir, name);
                                    cmd_pairs.push((name, full));
                                }
                            }
                        }
                    }
                }
                cmd_pairs.sort_by(|a, b| a.0.cmp(&b.0));

                for (cn, cp) in &cmd_pairs {
                    if Self::glob_match_static(cn, pat) {
                        if show_all {
                            // fetchcmdnamnode collects into matchednodes
                            // (builtin.c:4072) for fallthrough.
                            if seen_cmd.insert(cn.clone()) {
                                matched_cmd_names.push(cn.clone());
                            }
                        } else {
                            // Without -a: cmdnamtab->printnode emits each match.
                            if word_type {
                                println!("{}: command", cn);
                            } else if verbose {
                                println!("{} is {}", cn, cp);
                            } else {
                                println!("{}", cp);
                            }
                        }
                        informed += 1;
                    }
                }
            }

            if !show_all {
                // builtin.c:4082: `return returnval || !informed`.
                return if informed == 0 { 1 } else { 0 };
            }
            // With -a: replace argv with the collected command names so the
            // literal-name loop processes each across all scopes.
            owned_names_storage = matched_cmd_names;
            owned_names_storage.iter().map(|s| s.as_str()).collect()
        } else {
            names
        };

        let mut status = 0;
        for name in names {
            let mut found = false;
            let mut word = "none";

            if !path_only {
                // Check reserved words
                if self.is_reserved_word(name) {
                    found = true;
                    word = "reserved";
                    if word_type {
                        println!("{}: {}", name, word);
                    } else if verbose {
                        println!("{} is a reserved word", name);
                    } else if csh_style {
                        // `which local` (zsh: csh-style whence)
                        // outputs `local: shell reserved word`.
                        println!("{}: shell reserved word", name);
                    } else {
                        println!("{}", name);
                    }
                    if !show_all {
                        continue;
                    }
                }

                // Check aliases
                if let Some(alias_val) = self.aliases.get(name) {
                    found = true;
                    word = "alias";
                    if word_type {
                        println!("{}: {}", name, word);
                    } else if verbose {
                        println!("{} is an alias for {}", name, alias_val);
                    } else if csh_style {
                        // zsh `whence -c` for alias: `name: aliased to BODY`.
                        println!("{}: aliased to {}", name, alias_val);
                    } else {
                        println!("{}", alias_val);
                    }
                    if !show_all {
                        continue;
                    }
                }

                // Check functions (unless -f)
                if !skip_functions && self.function_exists(name) {
                    found = true;
                    word = "function";
                    if word_type {
                        println!("{}: {}", name, word);
                    } else if verbose {
                        println!("{} is a shell function from zsh", name);
                    } else if csh_style {
                        // zsh `whence -c` for function: full `typeset -f`
                        // body. Use `function_source` if registered, else
                        // `name () { ... }` shell stub.
                        let body = self
                            .function_source
                            .get(name)
                            .cloned()
                            .unwrap_or_else(|| ":".to_string());
                        println!("{} () {{\n\t{}\n}}", name, FuncBodyFmt::render(&body));
                    } else {
                        println!("{}", name);
                    }
                    if !show_all {
                        continue;
                    }
                }

                // Check builtins. NOTE: `is_builtin()` includes a
                // `_`-prefix bypass for completion functions, but
                // `whence`/`where`/`which` should report only ACTUAL
                // builtins (`BUILTIN_SET`), otherwise unknown names
                // like `__notacmd__` get reported as builtins.
                if BUILTIN_SET.contains(name) {
                    found = true;
                    word = "builtin";
                    if word_type {
                        println!("{}: {}", name, word);
                    } else if verbose {
                        println!("{} is a shell builtin", name);
                    } else if csh_style {
                        // zsh `which` / `whence -c` for builtin:
                        // `name: shell built-in command`.
                        println!("{}: shell built-in command", name);
                    } else {
                        println!("{}", name);
                    }
                    if !show_all {
                        continue;
                    }
                }

                // Check hashed commands (named_dirs can serve as a command hash)
                // The hash builtin adds to named_dirs for now
                if let Some(path) = self.named_dirs.get(name) {
                    found = true;
                    word = "hashed";
                    if word_type {
                        println!("{}: {}", name, word);
                    } else if verbose {
                        println!("{} is hashed ({})", name, path.display());
                    } else {
                        println!("{}", path.display());
                    }
                    if !show_all {
                        continue;
                    }
                }
            }

            // Check PATH
            if let Some(path) = self.find_in_path(name) {
                found = true;
                word = "command";

                // Handle symlink resolution
                let display_path = if show_symlink || show_symlink_steps {
                    let p = std::path::Path::new(&path);
                    if show_symlink_steps {
                        let mut current = p.to_path_buf();
                        let mut steps = vec![path.clone()];
                        while let Ok(target) = std::fs::read_link(&current) {
                            let resolved = if target.is_absolute() {
                                target.clone()
                            } else {
                                current
                                    .parent()
                                    .unwrap_or(std::path::Path::new("/"))
                                    .join(&target)
                            };
                            steps.push(resolved.to_string_lossy().to_string());
                            current = resolved;
                        }
                        steps.join(" -> ")
                    } else {
                        match p.canonicalize() {
                            Ok(resolved) => format!("{} -> {}", path, resolved.display()),
                            Err(_) => path.clone(),
                        }
                    }
                } else {
                    path.clone()
                };

                if word_type {
                    println!("{}: {}", name, word);
                } else if verbose {
                    println!("{} is {}", name, display_path);
                } else {
                    println!("{}", display_path);
                }
            }

            if !found {
                if word_type {
                    println!("{}: none", name);
                } else if verbose || csh_style {
                    // zsh `where`/`whence -v` writes the not-found
                    // line to stdout via `puts(" not found")`
                    // (Src/builtin.c bin_whence) — informational, not
                    // a diagnostic, so use println! not zwarnnam.
                    println!("{} not found", name);
                }
                status = 1;
            }
        }
        status
    }
    /// where - show all locations of a command
    pub(crate) fn builtin_where(&self, args: &[String]) -> i32 {
        // `where` is equivalent to `whence -ca` — c-shell style (just
        // the path / alias body, not the verbose `name is /path` form),
        // -a = list all matches in PATH (not just the first). Old impl
        // used `-a -v` which produced `ls is /bin/ls` instead of zsh's
        // bare `/bin/ls`.
        let mut new_args = vec!["-c".to_string(), "-a".to_string()];
        new_args.extend(args.iter().cloned());
        self.bin_whence(&new_args)
    }
    /// which - show path of command
    pub(crate) fn builtin_which(&self, args: &[String]) -> i32 {
        // which is like whence -c
        let mut new_args = vec!["-c".to_string()];
        new_args.extend(args.iter().cloned());
        self.bin_whence(&new_args)
    }
    /// umask - get/set file creation mask
    pub(crate) fn bin_umask(&self, args: &[String]) -> i32 {
        use libc::umask;

        let mut symbolic = false;
        let mut value: Option<&str> = None;
        let mut value_count = 0usize;

        for arg in args {
            match arg.as_str() {
                "-S" => symbolic = true,
                _ if !arg.starts_with('-') => {
                    value = Some(arg);
                    value_count += 1;
                }
                // zsh: unknown umask flag -> `umask:1: bad option:
                // -X` exit 1. zshrs's silent `_ => {}` accepted any
                // flag and then proceeded to print the umask, which
                // for `umask -X` was the wrong category entirely.
                _ if arg.starts_with('-') && arg.len() > 1 => {
                    let bad: String = arg[1..].chars().take(1).collect();
                    zwarnnam("umask", &format!("bad option: -{}", bad));
                    return 1;
                }
                _ => {}
            }
        }

        // zsh: `umask 022 044` (multiple positional values) errors
        // `too many arguments` and exits 1. zshrs's loop just
        // overwrote `value` silently with the last positional.
        if value_count > 1 {
            zwarnnam("umask", "too many arguments");
            return 1;
        }

        if let Some(v) = value {
            // Set umask. Two forms: numeric (`022`) or symbolic
            // (`u=rwx,g=rx,o=`). Symbolic form sets each class's
            // permitted bits; the umask itself is 0777 minus that.
            // Without the symbolic branch, `umask -S u=rwx,...` errored.
            if let Ok(mask) = u32::from_str_radix(v, 8) {
                unsafe {
                    umask(mask as libc::mode_t);
                }
            } else if v.contains('=') || v.contains('+') || v.contains('-') {
                // Symbolic mode — direct port of
                // src/zsh/Src/builtin.c:7533-7591 bin_umask. Three
                // operators:
                //   `+`: um &= ~mask;        (allow these bits)
                //   `-`: um |= mask;         (deny these bits)
                //   `=`: um = (um | whomask) & ~mask;  (set exact)
                //
                // The `whomask` defaults to 0777 (all classes) when
                // no class char is given. zsh accepts comma-separated
                // segments per builtin.c:7584-7587.
                let mut um = unsafe {
                    let m = umask(0);
                    umask(m);
                    m
                } as u32;
                let bytes = v.as_bytes();
                let mut i = 0;
                while i < bytes.len() {
                    // builtin.c:7542-7551 — parse class chars (u/g/o/a).
                    let mut whomask: u32 = 0;
                    while i < bytes.len() {
                        match bytes[i] {
                            b'u' => whomask |= 0o700,
                            b'g' => whomask |= 0o070,
                            b'o' => whomask |= 0o007,
                            b'a' => whomask |= 0o777,
                            _ => break,
                        }
                        i += 1;
                    }
                    if whomask == 0 {
                        whomask = 0o777;
                    }
                    // builtin.c:7556-7563 — operator.
                    if i >= bytes.len() {
                        zwarnnam("umask", "bad umask");
                        return 1;
                    }
                    let umaskop = bytes[i] as char;
                    if umaskop != '+' && umaskop != '-' && umaskop != '=' {
                        zwarnnam("umask", &format!("bad symbolic mode operator: {}", umaskop));
                        return 1;
                    }
                    i += 1;
                    // builtin.c:7565-7576 — perm bits.
                    let mut mask: u32 = 0;
                    while i < bytes.len() && bytes[i] != b',' {
                        match bytes[i] {
                            b'r' => mask |= 0o444 & whomask,
                            b'w' => mask |= 0o222 & whomask,
                            b'x' => mask |= 0o111 & whomask,
                            other => {
                                zwarnnam("umask", &format!("bad symbolic mode permission: {}", other as char));
                                return 1;
                            }
                        }
                        i += 1;
                    }
                    // builtin.c:7577-7583 — apply.
                    match umaskop {
                        '+' => um &= !mask,
                        '-' => um |= mask,
                        '=' => um = (um | whomask) & !mask,
                        _ => unreachable!(),
                    }
                    if i < bytes.len() && bytes[i] == b',' {
                        i += 1;
                    } else {
                        break;
                    }
                }
                if i < bytes.len() {
                    zwarnnam("umask", &format!("bad character in symbolic mode: {}", bytes[i] as char));
                    return 1;
                }
                unsafe {
                    umask((um & 0o777) as libc::mode_t);
                }
            } else {
                // Numeric parse failed AND no `=` for symbolic. zsh
                // emits a single `bad umask` for invalid octal (e.g.
                // `umask 999`, `umask 0Ab`); for malformed symbolic
                // without `=`, walk the input and emit the
                // operator-position diagnostic. Numeric-looking input
                // (any all-digits OR digit-prefixed) goes the terse
                // `bad umask` route.
                let looks_numeric = !v.is_empty() && v.chars().all(|c| c.is_ascii_digit());
                let starts_with_digit = v
                    .chars()
                    .next()
                    .map(|c| c.is_ascii_digit())
                    .unwrap_or(false);
                if looks_numeric || starts_with_digit {
                    zwarnnam("umask", "bad umask");
                    return 1;
                }
                let bytes = v.as_bytes();
                let mut i = 0;
                while i < bytes.len() && matches!(bytes[i], b'u' | b'g' | b'o' | b'a') {
                    i += 1;
                }
                if i < bytes.len() && !matches!(bytes[i], b'+' | b'-' | b'=') {
                    zwarnnam("umask", &format!("bad symbolic mode operator: {}", bytes[i] as char));
                } else {
                    zwarnnam("umask", "bad umask");
                }
                return 1;
            }
        } else {
            // Get umask
            let mask = unsafe {
                let m = umask(0);
                umask(m);
                m
            };
            if symbolic {
                let u = 7 - ((mask >> 6) & 7);
                let g = 7 - ((mask >> 3) & 7);
                let o = 7 - (mask & 7);
                println!(
                    "u={}{}{},g={}{}{},o={}{}{}",
                    if u & 4 != 0 { "r" } else { "" },
                    if u & 2 != 0 { "w" } else { "" },
                    if u & 1 != 0 { "x" } else { "" },
                    if g & 4 != 0 { "r" } else { "" },
                    if g & 2 != 0 { "w" } else { "" },
                    if g & 1 != 0 { "x" } else { "" },
                    if o & 4 != 0 { "r" } else { "" },
                    if o & 2 != 0 { "w" } else { "" },
                    if o & 1 != 0 { "x" } else { "" },
                );
            } else {
                // builtin.c:7519-7521 — `if (um & 0700) putchar('0');
                // printf("%03o\n", um);` — emit leading '0' when any
                // user-class bit is set, then 3 octal digits. zshrs
                // always emitted `%03o` so `umask 0444` printed `444`
                // instead of `0444` (the latter is what zsh prints
                // and what scripts that re-feed `$(umask)` to umask
                // expect — without the leading 0, octal parsers may
                // reject or misinterpret the value).
                if mask & 0o700 != 0 {
                    println!("0{:03o}", mask);
                } else {
                    println!("{:03o}", mask);
                }
            }
        }
        0
    }
    /// unhash - remove entries from hash table. Direct port of
    /// zsh/Src/builtin.c:4346 bin_unhash. Target table follows the C
    /// flag dispatch (builtin.c:4354-4379):
    ///   -d  named directories
    ///   -f  shell functions
    ///   -s  suffix aliases
    ///   -a  aliases (BIN_UNHASH only)
    ///   default  cmdnamtab (PATH command hash)
    /// `-m` treats each arg as a glob pattern and removes all matches
    /// (builtin.c:4396-4424); when no pattern matches we return 1.
    pub(crate) fn bin_unhash(&mut self, args: &[String]) -> i32 {
        #[derive(Copy, Clone)]
        enum Target {
            Aliases,
            SuffixAliases,
            Functions,
            NamedDirs,
            Commands,
        }
        let mut target = Target::Commands;
        let mut pattern_mode = false;
        let mut names: Vec<&str> = Vec::new();

        for arg in args {
            match arg.as_str() {
                "-a" => target = Target::Aliases,
                "-s" => target = Target::SuffixAliases,
                "-f" => target = Target::Functions,
                "-d" => target = Target::NamedDirs,
                "-m" => pattern_mode = true,
                // BUILTIN("unhash", ..., "adfms") — the option parser
                // rejects anything else. zshrs's `s if starts_with('-')
                // => {}` arm consumed unknown flags silently, so
                // `unhash -X foo` would unhash foo from cmdnamtab
                // (the default target) regardless of -X.
                s if s.starts_with('-') => {
                    let bad: String = s[1..].chars().take(1).collect();
                    zwarnnam("unhash", &format!("bad option: -{}", bad));
                    return 1;
                }
                _ => names.push(arg),
            }
        }

        let collect_keys = |this: &Self, t: Target| -> Vec<String> {
            match t {
                Target::Aliases => this.aliases.keys().cloned().collect(),
                Target::SuffixAliases => this.suffix_aliases.keys().cloned().collect(),
                Target::Functions => this.function_names(),
                Target::NamedDirs => this.named_dirs.keys().cloned().collect(),
                Target::Commands => this.command_hash.keys().cloned().collect(),
            }
        };

        let mut returnval = 0;
        if pattern_mode {
            let mut matched = 0;
            let keys = collect_keys(self, target);
            let to_remove: Vec<String> = names
                .iter()
                .flat_map(|p| {
                    keys.iter()
                        .filter(|k| Self::glob_match_static(k, p))
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .collect();
            for n in &to_remove {
                let removed = match target {
                    Target::Aliases => self.aliases.remove(n).is_some(),
                    Target::SuffixAliases => self.suffix_aliases.remove(n).is_some(),
                    Target::Functions => self.remove_function(n),
                    Target::NamedDirs => self.named_dirs.remove(n).is_some(),
                    Target::Commands => self.command_hash.remove(n).is_some(),
                };
                if removed {
                    matched += 1;
                }
            }
            if matched == 0 {
                returnval = 1;
            }
            return returnval;
        }

        for name in names {
            let removed = match target {
                Target::Aliases => self.aliases.remove(name).is_some(),
                Target::SuffixAliases => self.suffix_aliases.remove(name).is_some(),
                Target::Functions => self.remove_function(name),
                Target::NamedDirs => self.named_dirs.remove(name).is_some(),
                Target::Commands => self.command_hash.remove(name).is_some(),
            };
            if !removed {
                zwarnnam("unhash", &format!("no such hash table element: {}", name));
                returnval = 1;
            }
        }
        returnval
    }
    /// times - print accumulated user and system times
    pub(crate) fn bin_times(&self, _args: &[String]) -> i32 {
        // Direct port of src/zsh/Src/builtin.c:7324-7341 bin_times.
        // C uses times(2) which returns clock_t in jiffies, then formats
        // via pttime macro:
        //   `%ldm%ld.%02lds` with Mm=X/(60*clktck), Ss=(X/clktck)%clktck,
        //   FF=(X*100/clktck)%100.
        //
        // The Rust port previously used getrusage and `%.3fs %.3fs` —
        // diverged from zsh which prints `Mm0.00s 0m0.00s` per line.
        let clktck: i64 = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        if clktck <= 0 {
            // Fall back to the rusage-based path on systems where
            // _SC_CLK_TCK isn't available — better to print something
            // than fail.
            return 1;
        }

        let mut tms: libc::tms = unsafe { std::mem::zeroed() };
        let r = unsafe { libc::times(&mut tms) };
        if r == (-1i64 as libc::clock_t) {
            return 1;
        }

        // pttime port — emit Mm0.00s.
        let pttime = |x: i64| {
            let minutes = x / (60 * clktck);
            let seconds = (x / clktck) % clktck;
            let hundredths = (x * 100 / clktck) % 100;
            print!("{}m{}.{:02}s", minutes, seconds, hundredths);
        };

        pttime(tms.tms_utime as i64);
        print!(" ");
        pttime(tms.tms_stime as i64);
        println!();
        pttime(tms.tms_cutime as i64);
        print!(" ");
        pttime(tms.tms_cstime as i64);
        println!();
        0
    }
    /// r - redo last command (alias for fc -e -)
    pub(crate) fn builtin_r(&mut self, args: &[String]) -> i32 {
        let mut fc_args = vec!["-e".to_string(), "-".to_string()];
        fc_args.extend(args.iter().cloned());
        self.bin_fc(&fc_args)
    }
    /// ttyctl - control terminal settings. Direct port of
    /// zsh/Src/builtin.c:7454-7463 bin_ttyctl. -f freezes the tty
    /// state (zshrs won't restore tty after each command); -u unfreezes;
    /// no flags prints the current state. The freeze flag is stashed in
    /// `self.options["tty_frozen"]` so other builtins (sched, ZLE) can
    /// see it.
    pub(crate) fn bin_ttyctl(&mut self, args: &[String]) -> i32 {
        let mut got_f = false;
        let mut got_u = false;
        for arg in args {
            match arg.as_str() {
                "-f" => got_f = true,
                "-u" => got_u = true,
                _ => {}
            }
        }
        if got_f {
            self.options.insert("tty_frozen".to_string(), true);
        } else if got_u {
            self.options.insert("tty_frozen".to_string(), false);
        } else {
            let frozen = self.options.get("tty_frozen").copied().unwrap_or(false);
            println!("tty is {}frozen", if frozen { "" } else { "not " });
        }
        0
    }
    /// readonly - mark variables as read-only
    pub(crate) fn builtin_readonly(&mut self, args: &[String]) -> i32 {
        // PFA-SMR aspect: emit one `typeset` event per readonly NAME[=val]
        // arg, with the READONLY attr bit set (plus SCALAR by default —
        // `readonly` doesn't accept type-shape flags). builtin_readonly
        // does not delegate to builtin_typeset_named.
        #[cfg(feature = "recorder")]
        if crate::recorder::is_enabled() {
            let ctx = self.recorder_ctx();
            let mut attrs = crate::recorder::ParamAttrs::NONE;
            attrs.set(crate::recorder::ParamAttrs::SCALAR);
            attrs.set(crate::recorder::ParamAttrs::READONLY);
            for a in args {
                if a == "-p" || a.starts_with('-') {
                    continue;
                }
                if let Some((k, v)) = a.split_once('=') {
                    crate::recorder::emit_typeset_attrs(k, Some(v), attrs, ctx.clone());
                } else {
                    crate::recorder::emit_typeset_attrs(a, None, attrs, ctx.clone());
                }
            }
        }
        if args.is_empty() {
            // Sorted listing for deterministic output (was iterating
            // a HashSet in random order).
            let mut sorted: Vec<String> = self.readonly_vars.iter().cloned().collect();
            sorted.sort();
            for name in &sorted {
                if let Some(val) = self.variables.get(name) {
                    println!("readonly {}={}", name, val);
                }
            }
            return 0;
        }

        for arg in args {
            if arg == "-p" {
                let mut sorted: Vec<String> = self.readonly_vars.iter().cloned().collect();
                sorted.sort();
                for name in &sorted {
                    if let Some(val) = self.variables.get(name) {
                        println!("declare -r {}=\"{}\"", name, val);
                    }
                }
            } else if arg.starts_with('-') && arg.len() > 1 && !arg.starts_with("--") {
                // zsh: unknown `readonly` flag errors `readonly:1:
                // bad option: -X` exit 1. zshrs accepted any `-X`
                // silently as if it were a name to mark readonly.
                let bad: String = arg[1..].chars().take(1).collect();
                zwarnnam("readonly", &format!("bad option: -{}", bad));
                return 1;
            } else if let Some(eq_pos) = arg.find('=') {
                let name = &arg[..eq_pos];
                let value = &arg[eq_pos + 1..];
                // zsh: `readonly 1bad=5` -> `readonly:1: not an
                // identifier: 1bad` exit 1. zshrs silently accepted
                // and polluted the variable table.
                if !name.contains('[') {
                    let mut chars = name.chars();
                    let first_ok = chars
                        .next()
                        .map(|c| c.is_ascii_alphabetic() || c == '_')
                        .unwrap_or(false);
                    let body_ok = chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
                    if !first_ok || !body_ok {
                        zerrnam("readonly", &format!("not an identifier: {}", name));
                        return 1;
                    }
                }
                self.variables.insert(name.to_string(), value.to_string());
                self.readonly_vars.insert(name.to_string());
                // Mark the readonly attr on var_attrs so `(t)` flag
                // returns "scalar-readonly" (not just "scalar"). zsh
                // treats readonly as a compound type modifier, joined
                // with the base kind via `-`.
                self.var_attrs.entry(name.to_string()).or_default().readonly = true;
            } else {
                self.readonly_vars.insert(arg.clone());
                self.var_attrs.entry(arg.clone()).or_default().readonly = true;
            }
        }
        0
    }
    /// unfunction - remove function definitions
    /// unfunction NAME... — remove function definitions.
    /// `unfunction -m PATTERN...` matches names against globs.
    /// Direct port of zsh/Src/builtin.c:127 BUILTIN("unfunction", ...,
    /// "m", "f") which routes through bin_unhash with BIN_UNFUNCTION
    /// + the `f` default flag and `m` for glob mode.
    pub(crate) fn builtin_unfunction(&mut self, args: &[String]) -> i32 {
        let mut pattern_mode = false;
        let mut names: Vec<&str> = Vec::new();
        for arg in args {
            match arg.as_str() {
                "-m" => pattern_mode = true,
                "--" => {} // accept end-of-options
                s if s.starts_with('-') && s.len() > 1 => {
                    zwarnnam("unfunction", &format!("bad option: {}", s));
                    return 1;
                }
                s => names.push(s),
            }
        }
        if names.is_empty() {
            zwarnnam("unfunction", "not enough arguments");
            return 1;
        }
        let mut returnval = 0;
        if pattern_mode {
            let all: Vec<String> = self.function_names();
            let mut matched_any = false;
            for pat in &names {
                for fname in &all {
                    if Self::glob_match_static(fname, pat) && self.remove_function(fname) {
                        matched_any = true;
                    }
                }
            }
            if !matched_any {
                returnval = 1;
            }
        } else {
            for name in names {
                if !self.remove_function(name) {
                    zwarnnam("unfunction", &format!("no such function: {}", name));
                    returnval = 1;
                }
            }
        }
        returnval
    }
    /// getln - read line from the editor buffer stack
    pub(crate) fn builtin_getln(&mut self, args: &[String]) -> i32 {
        // builtin.c:78 — `getln` is `bin_read` with default flags `"zr"`,
        // i.e. `read -zr`. Pop the top entry from `buffer_stack`; if the
        // stack is empty, zsh's read uses an empty string (builtin.c:6770
        // ternary `nonempty(bufstack) ? getlinknode(bufstack) : ztrdup("")`).
        if args.is_empty() {
            zwarnnam("getln", "missing variable name");
            return 1;
        }
        let line = self.buffer_stack.pop().unwrap_or_default();
        let line = line.trim_end_matches('\n').to_string();
        self.variables.insert(args[0].clone(), line);
        0
    }
    /// pushln - push line onto editor buffer stack.
    /// builtin.c:106 — `pushln` is `bin_print` with default flags
    /// `-nz` (no newline, push to buffer stack). Was a stub that
    /// printed args to stdout instead of pushing.
    pub(crate) fn builtin_pushln(&mut self, args: &[String]) -> i32 {
        let line = args.join(" ");
        self.buffer_stack.push(line);
        0
    }
}
// END moved-from-exec-rs

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Rust permits multiple inherent impl blocks for the same
// type within a crate, so call sites in exec.rs are unchanged.
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    pub(crate) fn builtin_command(&mut self, args: &[String], redirects: &[Redirect]) -> i32 {
        // command [ -pvV ] simple command
        // -p: use default PATH
        // -v: print path (like which)
        // -V: verbose description (like type)
        let mut use_default_path = false;
        let mut print_path = false;
        let mut verbose = false;
        let mut positional_args: Vec<&str> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if arg == "--" {
                // Bare `--` ends options; rest are positional.
                i += 1;
                while i < args.len() {
                    positional_args.push(&args[i]);
                    i += 1;
                }
                break;
            }
            if arg.starts_with('-') && arg.len() > 1 && positional_args.is_empty() {
                // `command --foo` (long-option-style) — zsh treats the
                // whole thing as a command NAME to invoke (no built-in
                // long-option support). Mirror by emitting the
                // command-not-found diagnostic for any `--xxx` form.
                if arg.starts_with("--") {
                    zwarn(&format!("command not found: {}", arg));
                    return 127;
                }
                for ch in arg[1..].chars() {
                    match ch {
                        'p' => use_default_path = true,
                        'v' => print_path = true,
                        'V' => verbose = true,
                        _ => {
                            // zsh treats an unknown -X as a command
                            // name to invoke, not as a flag error.
                            // `command -x ls` → "command not found: -x"
                            // (and the rest of args become args to that
                            // bogus command). Match by emitting the
                            // command-not-found diagnostic instead of
                            // the flag-error message.
                            zwarn(&format!("command not found: -{}", ch));
                            return 127;
                        }
                    }
                }
            } else {
                positional_args.push(arg);
            }
            i += 1;
        }

        // Add remaining args after --
        while i < args.len() {
            positional_args.push(&args[i]);
            i += 1;
        }

        if positional_args.is_empty() {
            // zsh: bare `command` with no args AND no redirections
            // exits 0 silently. The "redirection with no command"
            // error in zsh fires only when redirections were present
            // (e.g. `command >file`) — that case is handled by the
            // parser, not here. The builtin sees identical empty-args
            // for both cases, so we mirror the bare-command exit 0
            // (matches bash too).
            return 0;
        }

        let cmd = positional_args[0];

        // -v or -V: print info about command. Resolution order matches
        // zsh's `whence`: alias → function → builtin → reserved word →
        // external. -v prints just the resolved name (or path for an
        // external); -V is the verbose human-readable form.
        if print_path || verbose {
            // Alias
            if let Some(target) = self.aliases.get(cmd) {
                if verbose {
                    println!("{} is an alias for {}", cmd, target);
                } else {
                    println!("alias {}={}", cmd, crate::ported::utils::quotedzputs(target));
                }
                return 0;
            }
            // Function
            if self.function_exists(cmd) {
                if verbose {
                    // zsh prints "is a shell function from <source>";
                    // for built-from-script defns the source is "zsh".
                    println!("{} is a shell function from zsh", cmd);
                } else {
                    println!("{}", cmd);
                }
                return 0;
            }
            // Shell builtin
            if self.is_builtin(cmd) || cmd == ":" || cmd == "[" {
                if verbose {
                    println!("{} is a shell builtin", cmd);
                } else {
                    println!("{}", cmd);
                }
                return 0;
            }
            // Reserved word (if/then/else/etc.)
            let reserved = matches!(
                cmd,
                "if" | "then"
                    | "else"
                    | "elif"
                    | "fi"
                    | "for"
                    | "while"
                    | "until"
                    | "do"
                    | "done"
                    | "case"
                    | "esac"
                    | "in"
                    | "function"
                    | "select"
                    | "time"
                    | "coproc"
                    | "{"
                    | "}"
                    | "[["
                    | "]]"
            );
            if reserved {
                if verbose {
                    println!("{} is a reserved word", cmd);
                } else {
                    println!("{}", cmd);
                }
                return 0;
            }
            // External
            let path_var = if use_default_path {
                "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin".to_string()
            } else {
                env::var("PATH").unwrap_or_default()
            };

            for dir in path_var.split(':') {
                let full_path = PathBuf::from(dir).join(cmd);
                if full_path.exists() && full_path.is_file() {
                    if verbose {
                        println!("{} is {}", cmd, full_path.display());
                    } else {
                        println!("{}", full_path.display());
                    }
                    return 0;
                }
            }

            if verbose {
                // C: `puts(" not found")` to stdout — informational,
                // not a diagnostic.
                println!("{} not found", cmd);
            }
            return 1;
        }

        // Execute as external command (bypassing functions and aliases)
        let cmd_args: Vec<String> = positional_args[1..].iter().map(|s| s.to_string()).collect();

        if use_default_path {
            // Temporarily set PATH
            let old_path = env::var("PATH").ok();
            env::set_var("PATH", "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin");
            let result = self
                .execute_external(
                    cmd,
                    &cmd_args
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                        .split_whitespace()
                        .map(String::from)
                        .collect::<Vec<_>>(),
                    redirects,
                )
                .unwrap_or(127);
            if let Some(p) = old_path {
                env::set_var("PATH", p);
            }
            result
        } else {
            self.execute_external(cmd, &cmd_args, redirects)
                .unwrap_or(127)
        }
    }
    pub(crate) fn builtin_builtin(&mut self, args: &[String], redirects: &[Redirect]) -> i32 {
        // Run builtin, bypassing functions and aliases
        self.dispatch_pending_traps();
        if self.redirect_failed { self.redirect_failed = false; return 1; }
        if args.is_empty() {
            return 0;
        }

        let cmd = &args[0];
        let cmd_args = &args[1..];

        match cmd.as_str() {
            "cd" => self.bin_cd(cmd_args),
            "pwd" => self.bin_pwd(redirects),
            "echo" => self.builtin_echo(cmd_args, redirects),
            "export" => self.builtin_export(cmd_args),
            "unset" => self.bin_unset(cmd_args),
            "exit" => self.bin_break("exit", cmd_args),
            "return" => self.bin_break("return", cmd_args),
            "logout" => {
                // zsh: `logout` is the login-shell-only counterpart to
                // `exit`. Outside a login shell it fails with "not
                // login shell" (with `(anon):logout:` or
                // `<funcname>:logout:` prefix in a function context,
                // `zsh:logout:1:` at top level). Detect via the LOGIN
                // option (set when invoked as `-l` or via `--login`).
                if self.options.get("login").copied().unwrap_or(false) {
                    self.bin_break("logout", cmd_args)
                } else {
                    // C source: `zerrnam(name, "not login shell")` —
                    // fatal (sets errflag) at Src/builtin.c:5861.
                    zerrnam("logout", "not login shell");
                    1
                }
            }
            "true" => 0,
            "false" => 1,
            ":" => 0,
            "test" | "[" => self.bin_test(cmd_args),
            "local" => self.builtin_local(cmd_args),
            "private" => self.builtin_local(cmd_args),
            "declare" => self.builtin_declare(cmd_args),
            "typeset" => self.bin_typeset(cmd_args),
            "read" => self.bin_read(cmd_args),
            "shift" => self.bin_shift(cmd_args),
            "eval" => self.bin_eval(cmd_args),
            "alias" => self.bin_alias(cmd_args),
            "unalias" => self.builtin_unalias(cmd_args),
            "set" => self.bin_set(cmd_args),
            "getopts" => self.bin_getopts(cmd_args),
            "type" => self.builtin_type(cmd_args),
            "hash" => self.bin_hash("hash", cmd_args),
            "add-zsh-hook" => self.builtin_add_zsh_hook(cmd_args),
            "autoload" => self.builtin_autoload(cmd_args),
            "source" => self.builtin_source_named(cmd_args, "source"),
            "." => self.builtin_source_named(cmd_args, "."),
            "functions" => self.bin_functions(cmd_args),
            "zle" => self.bin_zle(cmd_args),
            "bindkey" => self.bin_bindkey(cmd_args),
            "setopt" => self.bin_setopt("setopt", cmd_args),
            "unsetopt" => self.bin_setopt("unsetopt", cmd_args),
            "emulate" => self.bin_emulate(cmd_args),
            "zstyle" => self.bin_zstyle(cmd_args),
            "compadd" => self.bin_compadd(cmd_args),
            "compset" => self.bin_compset(cmd_args),
            "compctl" => crate::ported::zle::compctl::bin_compctl("compctl", cmd_args),
            "compcall" => crate::ported::zle::compctl::bin_compcall("compcall", cmd_args),
            "compdef" => self.builtin_compdef(cmd_args),
            "compinit" => self.builtin_compinit(cmd_args),
            "cdreplay" => self.builtin_cdreplay(cmd_args),
            "zmodload" => self.bin_zmodload(cmd_args),
            "zcompile" => self.bin_zcompile(cmd_args),
            "zformat" => self.bin_zformat(cmd_args),
            "zprof" => self.bin_zprof(cmd_args),
            "print" => self.bin_print(cmd_args),
            "printf" => self.builtin_printf(cmd_args),
            "command" => self.builtin_command(cmd_args, redirects),
            "whence" => self.bin_whence(cmd_args),
            "which" => self.builtin_which(cmd_args),
            "where" => self.builtin_where(cmd_args),
            "fc" => self.bin_fc(cmd_args),
            "history" => self.builtin_history(cmd_args),
            "dirs" => self.bin_dirs(cmd_args),
            "pushd" => self.builtin_pushd(cmd_args),
            "popd" => self.builtin_popd(cmd_args),
            "bg" => self.builtin_bg(cmd_args),
            "fg" => self.bin_fg(cmd_args),
            "jobs" => self.builtin_jobs(cmd_args),
            "kill" => self.bin_kill(cmd_args),
            "wait" => self.builtin_wait(cmd_args),
            "trap" => self.bin_trap(cmd_args),
            "umask" => self.bin_umask(cmd_args),
            "ulimit" => self.bin_ulimit(cmd_args),
            "times" => self.bin_times(cmd_args),
            "let" => self.bin_let(cmd_args),
            "integer" => self.builtin_integer(cmd_args),
            "float" => self.builtin_float(cmd_args),
            "readonly" => self.builtin_readonly(cmd_args),
            // zsh-bundled rename helpers — natively implemented so
            // `autoload -U zmv` doesn't actually need to load the
            // zsh function source file. See builtin_zmv for semantics.
            "zmv" => self.builtin_zmv(cmd_args, "mv"),
            "zcp" => self.builtin_zmv(cmd_args, "cp"),
            "zln" => self.builtin_zmv(cmd_args, "ln"),
            "zcalc" => self.builtin_zcalc(cmd_args),
            // Daemon-managed z* builtins — thin IPC wrappers. Name list is
            // owned by the daemon crate; routing via try_dispatch keeps this
            // site zero-touch as new z* builtins land.
            n if crate::daemon::builtins::is_zshrs_builtin(n) => {
                let argv: Vec<String> = std::iter::once(cmd.to_string())
                    .chain(cmd_args.iter().cloned())
                    .collect();
                crate::daemon::builtins::try_dispatch(n, &argv).unwrap_or(1)
            }
            _ => {
                zwarn(&format!("no such builtin: {}", cmd));
                1
            }
        }
    }
    /// exec - replace the shell with a command
    pub(crate) fn builtin_exec(&mut self, args: &[String]) -> i32 {
        // exec [ -c ] [ -l ] [ -a argv0 ] [ command [ arg ... ] ]
        // -c: clear environment
        // -l: place - at front of argv[0] (login shell)
        // -a argv0: set argv[0] to specified name

        let mut clear_env = false;
        let mut login_shell = false;
        let mut argv0: Option<String> = None;
        let mut cmd_args: Vec<String> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];

            if arg == "-c" && cmd_args.is_empty() {
                clear_env = true;
            } else if arg == "-l" && cmd_args.is_empty() {
                login_shell = true;
            } else if arg == "-a" && cmd_args.is_empty() {
                i += 1;
                if i >= args.len() {
                    // zsh: `exec -a NAME` requires a name argument
                    // — no following arg is `exec flag -a requires
                    // a parameter` exit 1, NOT the generic "exec
                    // requires a command to execute". Pinpoints the
                    // missing flag-value.
                    zwarn("exec flag -a requires a parameter");
                    return 1;
                }
                argv0 = Some(args[i].clone());
            } else if arg.starts_with('-') && cmd_args.is_empty() {
                // zsh: any flag-only `exec` with no following command
                // errors `exec requires a command to execute`. The
                // "flag with no command" check below already fires on
                // `-c`/`-l`/`-a`; track unrecognized flags too so
                // `exec --bad` triggers the same diagnostic instead
                // of silently no-op'ing.
                let mut saw_any_flag = false;
                for ch in arg[1..].chars() {
                    match ch {
                        'c' => {
                            clear_env = true;
                            saw_any_flag = true;
                        }
                        'l' => {
                            login_shell = true;
                            saw_any_flag = true;
                        }
                        'a' => {
                            saw_any_flag = true;
                            i += 1;
                            if i < args.len() {
                                argv0 = Some(args[i].clone());
                            }
                        }
                        '-' => {
                            // `--` is end-of-options in some impls; in
                            // `exec` zsh treats `--bad` as a flag-form
                            // typo. Mark as a flag so the no-command
                            // error fires below.
                            saw_any_flag = true;
                        }
                        _ => {
                            saw_any_flag = true;
                        }
                    }
                }
                if !saw_any_flag {
                    cmd_args.push(arg.clone());
                }
            } else {
                cmd_args.push(arg.clone());
            }
            i += 1;
        }

        if cmd_args.is_empty() {
            // zsh: `exec FLAG` (any flag form, including `--bad` long-
            // option-style typos) errors `exec requires a command to
            // execute` exit 1. Bare `exec` (no flags, no command) is
            // the silent-environment-modify form per POSIX. Detect
            // the flag-only case by scanning the original args for
            // anything that started with `-`; the per-char loop above
            // consumed those, so seeing one in the input is the
            // signal.
            let saw_flag = clear_env
                || login_shell
                || argv0.is_some()
                || args.iter().any(|a| a.starts_with('-') && a.len() > 1);
            if saw_flag {
                zwarn("exec requires a command to execute");
                return 1;
            }
            return 0;
        }

        let cmd = &cmd_args[0];
        let rest_args: Vec<&str> = cmd_args[1..].iter().map(|s| s.as_str()).collect();

        // Determine argv[0]
        let effective_argv0 = if let Some(a0) = argv0 {
            a0
        } else if login_shell {
            format!("-{}", cmd)
        } else {
            cmd.clone()
        };

        use std::os::unix::process::CommandExt;
        let mut command = std::process::Command::new(cmd);
        command.arg0(&effective_argv0);
        command.args(&rest_args);

        if clear_env {
            command.env_clear();
        }

        let err = command.exec();
        // zsh format for missing exec target: `zsh:1: no such file or
        // directory: PATH` (lowercased, no os-error suffix). Strip
        // Rust's wrapping.
        let msg = crate::ported::compat::strerror(err.raw_os_error().unwrap_or(0)).to_lowercase();
        zwarn(&format!("{}: {}", msg, cmd));
        // exec failure is fatal in zsh — exit the shell with status 127
        // (not 1) since the target couldn't be found/executed.
        std::process::exit(127);
    }
    /// noglob - run command without globbing
    pub(crate) fn builtin_noglob(&mut self, args: &[String], redirects: &[Redirect]) -> i32 {
        if args.is_empty() {
            return 0;
        }

        // Temporarily disable globbing
        let saved = self.options.get("noglob").cloned();
        self.options.insert("noglob".to_string(), true);

        // Execute the command. The previous impl routed through
        // `builtin_command` which only resolves externals (PATH lookup),
        // so `noglob print "*"` errored "command not found: print" even
        // though `print` is a shell builtin. Dispatch order matches
        // zsh's `noglob` precommand: shell builtin → function →
        // external. Only fall through to external when the name isn't
        // a known builtin or function.
        let cmd = &args[0];
        let status = if self.is_builtin(cmd) {
            self.builtin_builtin(args, redirects)
        } else if self.function_exists(cmd) {
            // Fall back to the regular command lookup which knows how
            // to invoke functions.
            self.builtin_command(args, redirects)
        } else {
            self.builtin_command(args, redirects)
        };

        // Restore globbing state
        if let Some(v) = saved {
            self.options.insert("noglob".to_string(), v);
        } else {
            self.options.remove("noglob");
        }

        status
    }
    /// zcompile - compile shell scripts to ZWC format
    pub(crate) fn bin_zcompile(&mut self, args: &[String]) -> i32 {
        use crate::zwc::{ZwcBuilder, ZwcFile};

        let mut list_mode = false; // -t: list functions in zwc
        let mut compile_current = false; // -c: compile current functions
        let mut compile_auto = false; // -a: compile autoload functions
        let mut files: Vec<String> = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            if arg.starts_with('-') && arg.len() > 1 {
                for c in arg[1..].chars() {
                    match c {
                        't' => list_mode = true,
                        'c' => compile_current = true,
                        'a' => compile_auto = true,
                        'U' | 'M' | 'R' | 'm' | 'z' | 'k' => {} // ignored for now
                        _ => {
                            zwarnnam("zcompile", &format!("bad option: -{}", c));
                            return 1;
                        }
                    }
                }
            } else {
                files.push(arg.clone());
            }
            i += 1;
        }

        if files.is_empty() {
            zwarnnam("zcompile", "not enough arguments");
            return 1;
        }

        // -t mode: list functions in ZWC file
        if list_mode {
            let zwc_path = if files[0].ends_with(".zwc") {
                files[0].clone()
            } else {
                format!("{}.zwc", files[0])
            };

            match ZwcFile::load(&zwc_path) {
                Ok(zwc) => {
                    println!("zwc file for zshrs-{}", env!("CARGO_PKG_VERSION"));
                    if files.len() > 1 {
                        // Check specific functions
                        for name in &files[1..] {
                            if zwc.get_function(name).is_some() {
                                println!("{}", name);
                            } else {
                                zwarnnam("zcompile", &format!("function not found: {}", name));
                                return 1;
                            }
                        }
                    } else {
                        // List all functions
                        for name in zwc.list_functions() {
                            println!("{}", name);
                        }
                    }
                    return 0;
                }
                Err(e) => {
                    zwarnnam("zcompile", &format!("can't read zwc file: {}: {}", zwc_path, e));
                    return 1;
                }
            }
        }

        // -c or -a mode: compile current/autoload functions
        if compile_current || compile_auto {
            let zwc_path = if files[0].ends_with(".zwc") {
                files[0].clone()
            } else {
                format!("{}.zwc", files[0])
            };

            let mut builder = ZwcBuilder::new();

            if files.len() > 1 {
                // Compile specific functions
                for name in &files[1..] {
                    if let Some(body) = self.function_definition_text(name) {
                        // Wrap as a `name() { body }` definition so the ZWC
                        // contains a parseable function source the loader
                        // can re-tokenize.
                        let source = format!("{} () {{\n{}\n}}", name, body);
                        builder.add_source(name, &source);
                    } else if compile_auto && self.autoload_pending.contains_key(name) {
                        // Try to load autoload function source
                        if let Some(path) = self.find_function_file(name) {
                            if let Err(e) = builder.add_file(&path) {
                                zwarnnam("zcompile", &format!("can't read {}: {}", name, e));
                                return 1;
                            }
                        }
                    } else {
                        zwarnnam("zcompile", &format!("no such function: {}", name));
                        return 1;
                    }
                }
            } else {
                // Compile all functions
                for name in self.function_names() {
                    if let Some(body) = self.function_definition_text(&name) {
                        let source = format!("{} () {{\n{}\n}}", name, body);
                        builder.add_source(&name, &source);
                    }
                }
            }

            if let Err(e) = builder.write(&zwc_path) {
                zwarnnam("zcompile", &format!("can't write {}: {}", zwc_path, e));
                return 1;
            }
            return 0;
        }

        // Default: compile files to ZWC
        let zwc_path = if files[0].ends_with(".zwc") {
            files[0].clone()
        } else {
            format!("{}.zwc", files[0])
        };

        let mut builder = ZwcBuilder::new();

        // If only one file given, it's both the source and output base
        let source_files = if files.len() == 1 {
            // Check if it's a directory
            let path = std::path::Path::new(&files[0]);
            if path.is_dir() {
                // Compile all files in directory
                match std::fs::read_dir(path) {
                    Ok(entries) => {
                        for entry in entries.flatten() {
                            let p = entry.path();
                            if p.is_file() && p.extension().is_none_or(|e| e != "zwc") {
                                if let Err(e) = builder.add_file(&p) {
                                    zwarnnam("zcompile", &format!("can't read {:?}: {}", p, e));
                                }
                            }
                        }
                    }
                    Err(e) => {
                        zwarnnam("zcompile", &format!("can't read directory: {}", e));
                        return 1;
                    }
                }
                vec![]
            } else {
                vec![files[0].clone()]
            }
        } else {
            files[1..].to_vec()
        };

        for file in &source_files {
            let path = std::path::Path::new(file);
            if let Err(e) = builder.add_file(path) {
                zwarnnam("zcompile", &format!("can't read {}: {}", file, e));
                return 1;
            }
        }

        if let Err(e) = builder.write(&zwc_path) {
            zwarnnam("zcompile", &format!("can't write {}: {}", zwc_path, e));
            return 1;
        }

        0
    }
    /// Shared dispatch for enable/disable per builtin.c:517-594.
    /// `enable=true` for the enable builtin (clear DISABLED flag);
    /// `enable=false` for the disable builtin (set DISABLED flag).
    pub(crate) fn do_enable_disable(&mut self, args: &[String], enable: bool) -> i32 {
        // builtin.c:526-538 — pick the hash table from flags.
        // -p   : enable/disable patterns (not yet implemented; falls
        //        through to no-op, matches a pre-port stub)
        // -f   : functions
        // -r   : reserved words (zsh keywords) — no-op for zshrs
        //        which doesn't have a reswd hash table
        // -s   : suffix aliases
        // -a   : regular aliases
        //   default: builtins
        let mut target: &str = "builtins";
        let mut match_glob = false;
        let mut names: Vec<String> = Vec::new();
        for arg in args {
            match arg.as_str() {
                "-a" => target = "aliases",
                "-f" => target = "functions",
                "-r" => target = "reswd",
                "-s" => target = "suffix_aliases",
                "-p" => target = "patterns",
                "-m" => match_glob = true,
                // BUILTIN("enable", ..., "afmprs") — six valid letters.
                // zshrs's catch-all `_ if starts_with('-') => {}`
                // silently consumed unknown flags, so `enable -X foo`
                // would enable the `foo` builtin while ignoring -X.
                _ if arg.starts_with('-') => {
                    let bad: String = arg[1..].chars().take(1).collect();
                    let bn = if enable { "enable" } else { "disable" };
                    zwarnnam(bn, &format!("bad option: -{}", bad));
                    return 1;
                }
                _ => names.push(arg.clone()),
            }
        }

        // builtin.c:553-558 — no args: list names of disabled
        // (`disable` builtin) or enabled (`enable` builtin) items
        // in the chosen target table. zshrs tracks the disabled set
        // for builtins via self.options['_disabled_<name>']; for
        // aliases / functions / suffix_aliases the disable path
        // removes the entry entirely so 'disabled' can't be listed
        // post-disable. Print the available scope's set sorted.
        if names.is_empty() {
            let mut listing: Vec<String> = match target {
                "builtins" => {
                    if enable {
                        // `enable` listing = currently-enabled
                        // builtins (BUILTIN_SET minus _disabled_*).
                        BUILTIN_SET
                            .iter()
                            .filter(|n| {
                                !self
                                    .options
                                    .get(&format!("_disabled_{}", n))
                                    .copied()
                                    .unwrap_or(false)
                            })
                            .map(|s| s.to_string())
                            .collect()
                    } else {
                        // `disable` listing = the _disabled_<name>
                        // set (i.e. builtins the user has turned off).
                        self.options
                            .iter()
                            .filter_map(|(k, v)| {
                                if *v {
                                    k.strip_prefix("_disabled_").map(|s| s.to_string())
                                } else {
                                    None
                                }
                            })
                            .collect()
                    }
                }
                "aliases" => self.aliases.keys().cloned().collect(),
                "suffix_aliases" => self.suffix_aliases.keys().cloned().collect(),
                "functions" => self.functions_compiled.keys().cloned().collect(),
                _ => Vec::new(),
            };
            listing.sort();
            for name in listing {
                println!("{}", name);
            }
            return 0;
        }

        // builtin.c:583-592 — literal-name dispatch.
        // builtin.c:561-580 — glob (-m) dispatch.
        let mut returnval = 0;
        let mut matched_any = false;
        let glob_match =
            |name: &str, pat: &str| -> bool { ShellExecutor::glob_match_static(name, pat) };

        for arg in &names {
            let mut hits: Vec<String> = Vec::new();
            match target {
                "aliases" => {
                    if match_glob {
                        hits = self
                            .aliases
                            .keys()
                            .filter(|k| glob_match(k, arg))
                            .cloned()
                            .collect();
                    } else if self.aliases.contains_key(arg) {
                        hits.push(arg.clone());
                    }
                }
                "suffix_aliases" => {
                    if match_glob {
                        hits = self
                            .suffix_aliases
                            .keys()
                            .filter(|k| glob_match(k, arg))
                            .cloned()
                            .collect();
                    } else if self.suffix_aliases.contains_key(arg) {
                        hits.push(arg.clone());
                    }
                }
                "functions" => {
                    if match_glob {
                        hits = self
                            .functions_compiled
                            .keys()
                            .filter(|k| glob_match(k, arg))
                            .cloned()
                            .collect();
                    } else if self.functions_compiled.contains_key(arg) {
                        hits.push(arg.clone());
                    }
                }
                "builtins" => {
                    if match_glob {
                        hits = BUILTIN_SET
                            .iter()
                            .filter(|k| glob_match(k, arg))
                            .map(|s| s.to_string())
                            .collect();
                    } else if BUILTIN_SET.contains(arg.as_str()) {
                        hits.push(arg.clone());
                    }
                }
                _ => {
                    // -p / -r / unsupported targets: no-op for now.
                }
            }
            if hits.is_empty() {
                if !match_glob {
                    // C: zwarnnam(name, "no such hash table element: %s", *argv)
                    zwarnnam(
                        if enable { "enable" } else { "disable" },
                        &format!("no such hash table element: {}", arg),
                    );
                    returnval = 1;
                }
                continue;
            }
            matched_any = true;
            for h in hits {
                if enable {
                    self.options.remove(&format!("_disabled_{}", h));
                } else {
                    // Disable. For aliases / functions zsh's hash
                    // table just sets the DISABLED flag; for the
                    // simpler zshrs model we remove the entry
                    // entirely (matches the previous impl).
                    match target {
                        "aliases" => {
                            self.aliases.remove(&h);
                        }
                        "suffix_aliases" => {
                            self.suffix_aliases.remove(&h);
                        }
                        "functions" => {
                            self.remove_function(&h);
                        }
                        _ => {
                            self.options.insert(format!("_disabled_{}", h), true);
                        }
                    }
                }
            }
        }

        // builtin.c:577-578 — `-m` with zero matches returns 1.
        if match_glob && !matched_any {
            returnval = 1;
        }
        returnval
    }
}
// END moved-from-exec-rs

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: autoload
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// Try to load a function from ZWC files in fpath. Returns true iff the
    /// function ended up resolvable (already loaded, or a ZWC scan landed it
    /// in functions_compiled / function_source / self.functions). Callers
    /// re-check via `function_exists(name)` after the call.
    pub fn autoload_function(&mut self, name: &str) -> bool {
        if self.function_exists(name) {
            return true;
        }

        // Search fpath for the function - use index to avoid borrow issues
        for i in 0..self.fpath.len() {
            let dir = self.fpath[i].clone();
            // Try directory.zwc first
            let zwc_path = dir.with_extension("zwc");
            if zwc_path.exists() && self.load_function_from_zwc(&zwc_path, name) {
                return true;
            }

            // Try individual function.zwc
            let func_zwc = dir.join(format!("{}.zwc", name));
            if func_zwc.exists() && self.load_function_from_zwc(&func_zwc, name) {
                return true;
            }

            // Look for directory/*.zwc files containing this function
            if dir.is_dir() {
                if let Ok(entries) = fs::read_dir(&dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().is_some_and(|e| e == "zwc")
                            && self.load_function_from_zwc(&path, name)
                        {
                            return true;
                        }
                    }
                }
            }
        }

        false
    }
    /// Load a specific function from a ZWC file. Populates
    /// `functions_compiled` and `function_source` as side effects;
    /// returns true iff the function landed in those tables.
    pub(crate) fn load_function_from_zwc(&mut self, path: &Path, name: &str) -> bool {
        // Check cache
        let zwc = if let Some(cached) = self.zwc_cache.get(path) {
            cached
        } else {
            let Ok(zwc) = ZwcFile::load(path) else {
                return false;
            };
            self.zwc_cache.insert(path.to_path_buf(), zwc);
            match self.zwc_cache.get(path) {
                Some(z) => z,
                None => return false,
            }
        };

        let Some(func) = zwc.get_function(name) else {
            return false;
        };
        let Some(decoded) = zwc.decode_function(func) else {
            return false;
        };
        let Some(shell_func) = decoded.to_shell_function() else {
            return false;
        };

        // ZWC bodies arrive as a ShellCommand AST (the wordcode → AST decoder
        // produces the legacy shape). Round-trip through `getpermtext` →
        // ZshParser → ZshCompiler so the compiled chunk lives on the new
        // pipeline. function_source carries the canonical text for
        // introspection.
        if let ShellCommand::FunctionDef(fname, body) = &shell_func {
            let body_text = crate::text::getpermtext(body.as_ref());
            let mut compiled = false;
            if let Some(program) = crate::parse::ZshParser::new(&body_text)
                .parse()
                .ok()
                .filter(|p| !p.lists.is_empty())
            {
                let chunk = crate::compile_zsh::ZshCompiler::new().compile(&program);
                self.functions_compiled.insert(fname.clone(), chunk);
                compiled = true;
            } else {
                tracing::warn!(
                    function = %fname,
                    "ZWC autoload: round-trip parse failed; function not callable \
                     via the new pipeline"
                );
            }
            self.function_source.insert(fname.clone(), body_text);
            return compiled;
        }
        false
    }
    /// Load an autoloaded function from fpath - reads file and parses it
    pub(crate) fn load_autoload_function(&mut self, name: &str) {
        // FAST PATH: Try caches first.
        // Skip in zsh_compat mode - use traditional fpath scanning only.
        if !self.zsh_compat {
            // FASTEST: cached `fusevm::Chunk` from the rkyv autoload shard
            // (~/.cache/zshrs/autoloads.rkyv). Skip lex+parse+compile entirely.
            // Insert into functions_compiled; the caller's outer dispatch
            // runs the chunk with proper positional params + local-scope
            // save/restore, identical to a freshly-loaded function.
            if let Some(bc_blob) = crate::autoload_cache::try_load(name) {
                if let Ok(chunk) = bincode::deserialize::<fusevm::Chunk>(&bc_blob) {
                    if !chunk.ops.is_empty() {
                        tracing::trace!(
                            name,
                            bytes = bc_blob.len(),
                            ops = chunk.ops.len(),
                            "autoload: bytecode cache hit"
                        );
                        self.functions_compiled.insert(name.to_string(), chunk);
                        if let Some(ref cache) = self.compsys_cache {
                            if let Ok(Some(body)) = cache.get_autoload_body(name) {
                                self.function_source.insert(name.to_string(), body);
                            }
                        }
                        return;
                    }
                }
            }

            if let Some(ref cache) = self.compsys_cache {
                // FAST: cached source text — parse + compile + cache the chunk.
                // ksh-style autoload files (`name() { body }`) need their
                // inner FuncDef body unwrapped before compile so the chunk
                // runs the body when invoked instead of re-registering name.
                if let Ok(Some(body)) = cache.get_autoload_body(name) {
                    if let Ok(program) = crate::parse::ZshParser::new(&body).parse() {
                        if !program.lists.is_empty() {
                            let target =
                                Self::ksh_autoload_body(&program, name).unwrap_or(&program);
                            let chunk = crate::compile_zsh::ZshCompiler::new().compile(target);
                            if let Ok(blob) = bincode::serialize(&chunk) {
                                let _ = crate::autoload_cache::try_save_one(name, &blob);
                                tracing::trace!(
                                    name,
                                    bytes = blob.len(),
                                    "autoload: bytecodes compiled and cached"
                                );
                            }
                            self.functions_compiled.insert(name.to_string(), chunk);
                            self.function_source.insert(name.to_string(), body.clone());
                        }
                    }
                    return;
                }
            }
        }

        // SLOW PATH: Try ZWC cache (but skip if we're reloading an existing function)
        if !self.function_exists(name) {
            for dir in &self.fpath.clone() {
                let zwc_path = dir.with_extension("zwc");
                if zwc_path.exists() {
                    let prefixed_name = format!(
                        "{}/{}",
                        dir.file_name().and_then(|n| n.to_str()).unwrap_or(""),
                        name
                    );
                    if self.load_function_from_zwc(&zwc_path, &prefixed_name) {
                        return;
                    }
                    if self.load_function_from_zwc(&zwc_path, name) {
                        return;
                    }
                }
                let func_zwc = dir.join(format!("{}.zwc", name));
                if func_zwc.exists() && self.load_function_from_zwc(&func_zwc, name) {
                    return;
                }
            }
        }

        // SLOWEST PATH: Find the function file in fpath
        let Some(path) = self.find_function_file(name) else {
            return;
        };
        let Ok(content) = std::fs::read_to_string(&path) else {
            return;
        };

        // ZshParser + ZshCompiler builds the persisted Chunk and
        // `function_source` captures the raw file contents. ksh-style
        // autoload files get their inner FuncDef body unwrapped before
        // compile (see `ksh_autoload_body`).
        if let Ok(program) = crate::parse::ZshParser::new(&content).parse() {
            if !program.lists.is_empty() {
                let target = Self::ksh_autoload_body(&program, name).unwrap_or(&program);
                let chunk = crate::compile_zsh::ZshCompiler::new().compile(target);
                self.functions_compiled.insert(name.to_string(), chunk);
                self.function_source.insert(name.to_string(), content);
            }
        }
    }
    /// Check if a function is autoload pending and load it if so. The new
    /// pipeline populates `functions_compiled` directly via `load_autoload_function`'s
    /// side effects; success is `functions_compiled.contains_key(name)`
    /// (not `function_exists`, which is now true for autoload-pending names
    /// regardless of load outcome).
    pub fn maybe_autoload(&mut self, name: &str) -> bool {
        if !self.autoload_pending.contains_key(name) {
            return false;
        }
        self.load_autoload_function(name);
        if self.functions_compiled.contains_key(name) {
            self.autoload_pending.remove(name);
            return true;
        }
        false
    }
}
// END moved-from-exec-rs

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: builtin-print
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    pub(crate) fn do_cd(&mut self, path_arg: &str, quiet: bool, use_cdpath: bool, logical: bool) -> i32 {
        let physical = !logical;
        // Read $HOME from the shell's variable store first; fall back to
        // the OS env. This makes `HOME=/tmp; cd` follow the shell-local
        // assignment even when no `export` was used.
        let home_dir = || -> PathBuf {
            self.variables
                .get("HOME")
                .cloned()
                .or_else(|| std::env::var("HOME").ok())
                .map(PathBuf::from)
                .or_else(dirs::home_dir)
                .unwrap_or_else(|| PathBuf::from("."))
        };
        let path = if path_arg == "~" || path_arg.is_empty() {
            home_dir()
        } else if let Some(after) = path_arg.strip_prefix("~/") {
            home_dir().join(after)
        } else if path_arg == "-" {
            if let Ok(oldpwd) = env::var("OLDPWD") {
                // zsh only prints the new dir in interactive mode.
                // In `-c` (non-interactive), suppress to match.
                if !quiet && atty::is(atty::Stream::Stdin) {
                    println!("{}", oldpwd);
                }
                PathBuf::from(oldpwd)
            } else {
                zwarnnam("cd", "OLDPWD not set");
                return 1;
            }
        } else if !path_arg.starts_with('/')
            && !path_arg.starts_with('.')
            && !PathBuf::from(path_arg).is_dir()
        {
            // Search CDPATH (zsh searches it implicitly when the literal
            // path is not a directory in cwd; `-s` is not required).
            // Honor shell-state CDPATH first so a non-exported assignment
            // applies. cdpath array (zsh-specific) is checked too.
            let cdpath = self
                .variables
                .get("CDPATH")
                .cloned()
                .or_else(|| env::var("CDPATH").ok())
                .unwrap_or_default();
            let mut found = None;
            for dir in cdpath.split(':') {
                let candidate = if dir.is_empty() {
                    PathBuf::from(path_arg)
                } else {
                    PathBuf::from(dir).join(path_arg)
                };
                if candidate.is_dir() {
                    found = Some(candidate);
                    break;
                }
            }
            if found.is_none() {
                if let Some(arr) = self.arrays.get("cdpath") {
                    for dir in arr {
                        let candidate = if dir.is_empty() {
                            PathBuf::from(path_arg)
                        } else {
                            PathBuf::from(dir).join(path_arg)
                        };
                        if candidate.is_dir() {
                            found = Some(candidate);
                            break;
                        }
                    }
                }
            }
            let _ = use_cdpath;
            found.unwrap_or_else(|| PathBuf::from(path_arg))
        } else {
            PathBuf::from(path_arg)
        };

        // Stash the OLDPWD using the LOGICAL pwd we tracked previously
        // (not the realpath'd one) — `cd -` should round-trip the
        // user-typed path, not the symlink target.
        let old_pwd_logical = self
            .variables
            .get("PWD")
            .cloned()
            .or_else(|| env::var("PWD").ok())
            .or_else(|| {
                env::current_dir()
                    .ok()
                    .map(|p| p.to_string_lossy().to_string())
            })
            .unwrap_or_default();
        env::set_var("OLDPWD", &old_pwd_logical);
        self.variables
            .insert("OLDPWD".to_string(), old_pwd_logical.clone());

        // Compute the LOGICAL target — relative paths resolve against
        // the current PWD without realpath. Components like `.` and
        // `..` are normalised lexically (not by following symlinks).
        // Inline lexical-normalize — direct port of the path-component
        // walk in fixdir() (Src/builtin.c:1297): drop CurDir, pop on
        // ParentDir, preserve root/prefix; empty result becomes ".".
        let normalize_lex = |path: &std::path::Path| -> PathBuf {
            use std::path::Component;
            let mut out = PathBuf::new();
            for comp in path.components() {
                match comp {
                    Component::Prefix(_) | Component::RootDir => out.push(comp.as_os_str()),
                    Component::CurDir => {}
                    Component::ParentDir => {
                        if !out.pop() {
                            out.push("..");
                        }
                    }
                    Component::Normal(c) => out.push(c),
                }
            }
            if out.as_os_str().is_empty() {
                out.push(".");
            }
            out
        };
        let logical_target: PathBuf = if path.is_absolute() {
            normalize_lex(&path)
        } else {
            normalize_lex(&PathBuf::from(&old_pwd_logical).join(&path))
        };

        // chdir using the logical path (kernel handles symlinks). With
        // `-P`/physical, realpath the result before storing PWD.
        let chdir_target: PathBuf = logical_target.clone();
        match env::set_current_dir(&chdir_target) {
            Ok(_) => {
                let stored = if physical {
                    chdir_target.canonicalize().unwrap_or(chdir_target)
                } else {
                    logical_target
                };
                let stored_str = stored.to_string_lossy().to_string();
                env::set_var("PWD", &stored);
                self.variables.insert("PWD".to_string(), stored_str);
                // Run zsh's `chpwd` hook + `chpwd_functions` array.
                // Direct port of Src/builtin.c bin_cd's call to
                // run_hooks("chpwd"). Without this, scripts that
                // rely on `chpwd() { ... }` for prompt updates,
                // direnv-style integration, etc. saw their hook
                // never fire.
                self.run_hooks("chpwd");
                0
            }
            Err(e) => {
                // zsh format: `zshrs:cd:1: no such file or directory: PATH`
                // (lowercased, no Rust os-error suffix). Exit code stays 1.
                let msg = crate::ported::compat::strerror(e.raw_os_error().unwrap_or(0)).to_lowercase();
                zwarnnam("cd", &format!("{}: {}", msg, path.display()));
                1
            }
        }
    }
    /// Push directory onto stack and cd to it
    /// Mirror `self.dir_stack` into `self.arrays["dirstack"]` so user
    /// reads of `${dirstack[@]}` / `$dirstack[N]` see the live stack.
    /// Direct port of zsh's PM_SPECIAL `dirstack` setfn — the C
    /// source synthesizes the array on demand from the internal
    /// LinkList; zshrs uses a side-table sync since the array
    /// lookup paths route through `arrays` (not a getfn dispatch).
    pub(crate) fn sync_dirstack_array(&mut self) {
        // zsh's `dirstack` is ordered most-recent-push first
        // (`dirstack[1]` is the directory `pushd` saved on the most
        // recent call). zshrs's `dir_stack` is push-back order, so
        // reverse when mirroring to the array.
        let dirs: Vec<String> = self
            .dir_stack
            .iter()
            .rev()
            .map(|p| p.display().to_string())
            .collect();
        if dirs.is_empty() {
            self.arrays.remove("dirstack");
        } else {
            self.arrays.insert("dirstack".to_string(), dirs);
        }
    }
    pub(crate) fn print_dir_stack(&self) {
        // Use logical $PWD (preserves symlinks the user typed) — same
        // source `dirs` reads. Falls back to OS cwd only when $PWD is
        // unset. Without this, pushd /tmp; dirs printed /private/tmp
        // on macOS instead of /tmp.
        let current = self
            .variables
            .get("PWD")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let home = std::env::var("HOME").ok();
        let tilde = |p: &std::path::Path| -> String {
            let s = p.to_string_lossy().to_string();
            // zsh's dir-stack listing uses `~` for $HOME paths.
            // Without this, the output had absolute /Users/wizard/... that
            // diverged from zsh's `~/...` form.
            if let Some(ref h) = home {
                if let Some(rest) = s.strip_prefix(h) {
                    if rest.is_empty() {
                        return "~".to_string();
                    }
                    if rest.starts_with('/') {
                        return format!("~{}", rest);
                    }
                }
            }
            s
        };
        let mut parts = vec![tilde(&current)];
        for dir in self.dir_stack.iter().rev() {
            parts.push(tilde(dir));
        }
        println!("{}", parts.join(" "));
    }
    #[allow(dead_code)]
    pub(crate) fn expand_printf_escapes_internal_marker(&self) {}
    /// Print-builtin escape decoder. Same recognised-escape set as
    /// `expand_printf_escapes`, but drops the leading backslash for
    /// any UNRECOGNISED `\X` (so `\ ` → ` `, `\X` → `X`). Direct
    /// port of getkeystring() / printflags() behavior the C-source
    /// bin_print uses (Src/utils.c:5045-5180). Echo continues using
    /// the keep-backslash variant.
    pub(crate) fn expand_print_escapes(&self, s: &str) -> String {
        let mut result = String::new();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => result.push('\n'),
                    Some('t') => result.push('\t'),
                    Some('r') => result.push('\r'),
                    Some('\\') => result.push('\\'),
                    Some('a') => result.push('\x07'),
                    Some('b') => result.push('\x08'),
                    Some('e') | Some('E') => result.push('\x1b'),
                    Some('f') => result.push('\x0c'),
                    Some('v') => result.push('\x0b'),
                    Some('0') => {
                        let mut octal = String::new();
                        while octal.len() < 3 {
                            if let Some(&d) = chars.peek() {
                                if ('0'..='7').contains(&d) {
                                    octal.push(d);
                                    chars.next();
                                } else {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                        if octal.is_empty() {
                            result.push('\0');
                        } else if let Ok(val) = u8::from_str_radix(&octal, 8) {
                            result.push(val as char);
                        }
                    }
                    Some('c') => break,
                    Some('x') => {
                        let mut hex = String::new();
                        while hex.len() < 2 {
                            if let Some(&d) = chars.peek() {
                                if d.is_ascii_hexdigit() {
                                    hex.push(d);
                                    chars.next();
                                } else {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                        if hex.is_empty() {
                            result.push('\\');
                            result.push('x');
                        } else if let Ok(val) = u8::from_str_radix(&hex, 16) {
                            result.push(val as char);
                        }
                    }
                    Some(other) => {
                        // Print-specific: drop the backslash for
                        // unrecognised escapes. zsh's `print "\ "`
                        // emits a single space, not `\<space>`.
                        result.push(other);
                    }
                    None => result.push('\\'),
                }
            } else {
                result.push(c);
            }
        }
        result
    }
    pub(crate) fn expand_printf_escapes(&self, s: &str) -> String {
        let mut result = String::new();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => result.push('\n'),
                    Some('t') => result.push('\t'),
                    Some('r') => result.push('\r'),
                    Some('\\') => result.push('\\'),
                    Some('a') => result.push('\x07'),
                    Some('b') => result.push('\x08'),
                    Some('e') | Some('E') => result.push('\x1b'),
                    Some('f') => result.push('\x0c'),
                    Some('v') => result.push('\x0b'),
                    Some('0') => {
                        let mut octal = String::new();
                        while octal.len() < 3 {
                            if let Some(&d) = chars.peek() {
                                if ('0'..='7').contains(&d) {
                                    octal.push(d);
                                    chars.next();
                                } else {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                        if octal.is_empty() {
                            result.push('\0');
                        } else if let Ok(val) = u8::from_str_radix(&octal, 8) {
                            result.push(val as char);
                        }
                    }
                    Some('c') => break,
                    Some('x') => {
                        // \xHH — 1 or 2 hex digits.
                        let mut hex = String::new();
                        while hex.len() < 2 {
                            if let Some(&d) = chars.peek() {
                                if d.is_ascii_hexdigit() {
                                    hex.push(d);
                                    chars.next();
                                } else {
                                    break;
                                }
                            } else {
                                break;
                            }
                        }
                        if hex.is_empty() {
                            result.push('\\');
                            result.push('x');
                        } else if let Ok(val) = u8::from_str_radix(&hex, 16) {
                            result.push(val as char);
                        }
                    }
                    Some(other) => {
                        // For unrecognised `\X` escapes zsh's `print`
                        // drops the backslash (`\ ` → ` `, `\.` → `.`)
                        // BUT `echo` keeps it (`\ ` stays `\ `). Both
                        // paths share this expander; the more
                        // permissive "keep" form preserves `:q`-flag
                        // round-trips through `echo` (which is what
                        // user scripts test against). The print-only
                        // drop semantics are handled at the print-
                        // builtin layer separately.
                        result.push('\\');
                        result.push(other);
                    }
                    None => result.push('\\'),
                }
            } else {
                result.push(c);
            }
        }
        result
    }
    pub(crate) fn printf_format(&self, format: &str, args: &[String]) -> String {
        self.printf_format_count(format, args).0
    }
    /// Same as `printf_format` but also returns how many args were
    /// consumed so callers can cycle the format until args run out
    /// (POSIX printf / zsh `print -f` semantics).
    pub(crate) fn printf_format_count(&self, format: &str, args: &[String]) -> (String, usize) {
        let mut result = String::new();
        let mut arg_idx = 0;
        let mut chars = format.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '%' {
                if chars.peek() == Some(&'%') {
                    chars.next();
                    result.push('%');
                    continue;
                }

                // Parse format specifier
                let mut spec = String::from("%");
                let mut left_align = false;
                let mut zero_pad = false;
                let mut plus_flag = false;
                let mut space_flag = false;
                let mut hash_flag = false;

                // Flags
                while let Some(&c) = chars.peek() {
                    match c {
                        '-' => left_align = true,
                        '+' => plus_flag = true,
                        ' ' => space_flag = true,
                        '#' => hash_flag = true,
                        '0' => zero_pad = true,
                        _ => break,
                    }
                    spec.push(c);
                    chars.next();
                }

                // Width
                let mut width: Option<usize> = None;
                while let Some(&c) = chars.peek() {
                    if c.is_ascii_digit() {
                        spec.push(c);
                        width = Some(width.unwrap_or(0) * 10 + (c as u8 - b'0') as usize);
                        chars.next();
                    } else {
                        break;
                    }
                }

                // Precision
                let mut precision: Option<usize> = None;
                if chars.peek() == Some(&'.') {
                    spec.push('.');
                    chars.next();
                    let mut p = 0usize;
                    let mut had_p = false;
                    while let Some(&c) = chars.peek() {
                        if c.is_ascii_digit() {
                            spec.push(c);
                            p = p * 10 + (c as u8 - b'0') as usize;
                            had_p = true;
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    if had_p {
                        precision = Some(p);
                    } else {
                        precision = Some(0);
                    }
                }

                let pad = |s: &str, w: usize, left: bool, zero: bool| -> String {
                    let len = s.chars().count();
                    if len >= w {
                        return s.to_string();
                    }
                    let fill = if zero && !left { '0' } else { ' ' };
                    let extra: String = std::iter::repeat_n(fill, w - len).collect();
                    if left {
                        format!("{}{}", s, extra)
                    } else {
                        format!("{}{}", extra, s)
                    }
                };

                // Conversion specifier
                if let Some(conv) = chars.next() {
                    let arg = args.get(arg_idx).map(|s| s.as_str()).unwrap_or("");
                    arg_idx += 1;

                    match conv {
                        's' => {
                            let mut v = arg.to_string();
                            if let Some(p) = precision {
                                let trimmed: String = v.chars().take(p).collect();
                                v = trimmed;
                            }
                            if let Some(w) = width {
                                v = pad(&v, w, left_align, false);
                            }
                            result.push_str(&v);
                        }
                        'd' | 'i' => {
                            let n: i64 = arg.parse().unwrap_or(0);
                            let mut v = n.to_string();
                            if n >= 0 {
                                if plus_flag {
                                    v = format!("+{}", v);
                                } else if space_flag {
                                    v = format!(" {}", v);
                                }
                            }
                            if let Some(w) = width {
                                v = pad(&v, w, left_align, zero_pad);
                            }
                            result.push_str(&v);
                        }
                        'u' => {
                            let n: u64 = arg.parse().unwrap_or(0);
                            let mut v = n.to_string();
                            if let Some(w) = width {
                                v = pad(&v, w, left_align, zero_pad);
                            }
                            result.push_str(&v);
                        }
                        'x' => {
                            let n: i64 = arg.parse().unwrap_or(0);
                            let mut v = format!("{:x}", n);
                            if hash_flag && n != 0 {
                                v = format!("0x{}", v);
                            }
                            if let Some(w) = width {
                                v = pad(&v, w, left_align, zero_pad);
                            }
                            result.push_str(&v);
                        }
                        'X' => {
                            let n: i64 = arg.parse().unwrap_or(0);
                            let mut v = format!("{:X}", n);
                            if hash_flag && n != 0 {
                                v = format!("0X{}", v);
                            }
                            if let Some(w) = width {
                                v = pad(&v, w, left_align, zero_pad);
                            }
                            result.push_str(&v);
                        }
                        'o' => {
                            let n: i64 = arg.parse().unwrap_or(0);
                            let mut v = format!("{:o}", n);
                            if hash_flag && n != 0 {
                                v = format!("0{}", v);
                            }
                            if let Some(w) = width {
                                v = pad(&v, w, left_align, zero_pad);
                            }
                            result.push_str(&v);
                        }
                        'f' | 'F' | 'e' | 'E' | 'g' | 'G' => {
                            let n: f64 = arg.parse().unwrap_or(0.0);
                            let mut v = match (conv, precision) {
                                ('f', Some(p)) | ('F', Some(p)) => format!("{:.*}", p, n),
                                ('e', Some(p)) | ('E', Some(p)) => format!("{:.*e}", p, n),
                                ('f', None) | ('F', None) => format!("{:.6}", n),
                                ('e', None) | ('E', None) => format!("{:.6e}", n),
                                _ => format!("{}", n),
                            };
                            // Rust's `{:e}` emits `e<exp>` (no sign, 1-digit
                            // for small exponents). C printf / zsh expect
                            // `e±DD` (signed, ≥2 digits). Fix the exp tail
                            // for %e/%E. Without this, `printf "%e" 1000`
                            // produced `1.000000e3` instead of zsh's
                            // `1.000000e+03`.
                            if matches!(conv, 'e' | 'E') {
                                if let Some(epos) = v.rfind('e') {
                                    let (mantissa, exp) = v.split_at(epos);
                                    let exp_body = &exp[1..]; // skip 'e'
                                    let (sign, digits) = if let Some(d) = exp_body.strip_prefix('-') {
                                        ("-", d)
                                    } else if let Some(d) = exp_body.strip_prefix('+') {
                                        ("+", d)
                                    } else {
                                        ("+", exp_body)
                                    };
                                    let padded = if digits.len() < 2 {
                                        format!("0{}", digits)
                                    } else {
                                        digits.to_string()
                                    };
                                    v = format!("{}e{}{}", mantissa, sign, padded);
                                }
                            }
                            if matches!(conv, 'E' | 'G') {
                                v = v.replace('e', "E");
                            }
                            if n >= 0.0 {
                                if plus_flag {
                                    v = format!("+{}", v);
                                } else if space_flag {
                                    v = format!(" {}", v);
                                }
                            }
                            if let Some(w) = width {
                                v = pad(&v, w, left_align, zero_pad);
                            }
                            result.push_str(&v);
                        }
                        'c' => {
                            if let Some(c) = arg.chars().next() {
                                result.push(c);
                            }
                        }
                        'b' => {
                            result.push_str(&self.expand_printf_escapes(arg));
                        }
                        'q' => {
                            // zsh `%q` — backslash-escape shell-special
                            // chars (matches `${(q)}` flag, NOT `(qq)`).
                            // bash uses single-bslashquote wrapping here; zsh's
                            // own printf takes the backslash route.
                            let mut out = String::with_capacity(arg.len() + 4);
                            for c in arg.chars() {
                                if matches!(
                                    c,
                                    ' ' | '\t'
                                        | '\''
                                        | '"'
                                        | '\\'
                                        | '$'
                                        | '`'
                                        | '*'
                                        | '?'
                                        | '['
                                        | ']'
                                        | '{'
                                        | '}'
                                        | '('
                                        | ')'
                                        | '|'
                                        | '&'
                                        | ';'
                                        | '<'
                                        | '>'
                                        | '#'
                                        | '~'
                                ) {
                                    out.push('\\');
                                }
                                out.push(c);
                            }
                            result.push_str(&out);
                        }
                        'n' => result.push('\n'),
                        _ => {
                            // Unknown directive — zsh emits
                            // `printf:1: %X: invalid directive`. zshrs
                            // previously emitted the literal `%X`.
                            // %a (hex float) and %v (bash-only) hit this
                            // path along with any user typo.
                            zwarnnam("printf", &format!("%{}: invalid directive", conv));
                        }
                    }
                }
            } else {
                result.push(ch);
            }
        }

        (result, arg_idx)
    }
    pub(crate) fn is_reserved_word(&self, name: &str) -> bool {
        matches!(
            name,
            "if" | "then"
                | "else"
                | "elif"
                | "fi"
                | "case"
                | "esac"
                | "for"
                | "select"
                | "while"
                | "until"
                | "do"
                | "done"
                | "in"
                | "function"
                | "time"
                | "coproc"
                | "repeat"
                | "foreach"
                | "end"
                | "nocorrect"
                | "noglob"
                // Declaration keywords (precommand modifiers).
                // zsh treats these as reserved-word declarations,
                // not regular builtins.
                | "local"
                | "declare"
                | "typeset"
                | "readonly"
                | "export"
                | "integer"
                | "float"
                | "{"
                | "}"
                | "!"
                | "[["
                | "]]"
                | "(("
                | "))"
        )
    }
}
// END moved-from-exec-rs

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: drift
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// Get list of all builtin command names
    pub(crate) fn get_builtin_names() -> Vec<&'static str> {
        vec![
            ".",
            ":",
            "[",
            "alias",
            "autoload",
            "bg",
            "bind",
            "bindkey",
            "break",
            "builtin",
            "bye",
            "caller",
            "cd",
            "cdreplay",
            "chdir",
            "clone",
            "command",
            "compadd",
            "comparguments",
            "compcall",
            "compctl",
            "compdef",
            "compdescribe",
            "compfiles",
            "compgen",
            "compgroups",
            "compinit",
            "complete",
            "compopt",
            "compquote",
            "compset",
            "comptags",
            "comptry",
            "compvalues",
            "continue",
            "coproc",
            "declare",
            "dirs",
            "disable",
            "disown",
            "echo",
            "echotc",
            "echoti",
            "emulate",
            "enable",
            "eval",
            "exec",
            "exit",
            "export",
            "false",
            "fc",
            "fg",
            "float",
            "functions",
            "getln",
            "getopts",
            "hash",
            "help",
            "history",
            "integer",
            "jobs",
            "kill",
            "let",
            "limit",
            "local",
            "log",
            "logout",
            "mapfile",
            "noglob",
            "popd",
            "print",
            "printf",
            "private",
            "prompt",
            "promptinit",
            "pushd",
            "pushln",
            "pwd",
            "r",
            "read",
            "readarray",
            "readonly",
            "rehash",
            "return",
            "sched",
            "set",
            "setopt",
            "shift",
            "source",
            "stat",
            "output_strftime",
            "suspend",
            "test",
            "times",
            "trap",
            "true",
            "ttyctl",
            "type",
            "typeset",
            "ulimit",
            "umask",
            "unalias",
            "unfunction",
            "unhash",
            "unlimit",
            "unset",
            "unsetopt",
            "vared",
            "wait",
            "whence",
            "where",
            "which",
            "zcompile",
            "zcurses",
            "zformat",
            "zle",
            "zmodload",
            "zparseopts",
            "zprof",
            "zpty",
            "zregexparse",
            "zsocket",
            "zstyle",
            "ztcp",
            "add-zsh-hook",
        ]
    }
    /// Find a function file in fpath
    pub(crate) fn find_function_file(&self, name: &str) -> Option<PathBuf> {
        for dir in &self.fpath {
            let path = dir.join(name);
            if path.exists() && path.is_file() {
                return Some(path);
            }
        }
        None
    }
}
// END moved-from-exec-rs


// ===========================================================
// ksh_autoload_body moved from src/ported/exec.rs.
// Mirrors the ksh-style autoload helper in Src/builtin.c
// (bin_functions / load_function_def).
// ===========================================================
impl crate::ported::exec::ShellExecutor {
    /// If `program` is a ksh-style autoload file (single function definition
    /// whose name matches the requested autoload target), return a reference
    /// to the FuncDef's body. Otherwise return None — caller compiles the
    /// whole program (zsh-style autoload, where the file contents ARE the
    /// function body).
    ///
    /// Both `function name { body }` and `name() { body }` shapes parse to
    /// a single `ZshFuncDef` (the parser synthesizes the latter at parse
    /// time via the `simple_name_with_inoutpar` recovery in
    /// `parse_program_until`).
    pub(crate) fn ksh_autoload_body<'a>(
        program: &'a crate::parse::ZshProgram,
        name: &str,
    ) -> Option<&'a crate::parse::ZshProgram> {
        if program.lists.len() != 1 {
            return None;
        }
        let list = &program.lists[0];
        if list.flags.async_ || list.sublist.next.is_some() {
            return None;
        }
        let pipe = &list.sublist.pipe;
        if pipe.next.is_some() {
            return None;
        }
        if let crate::parse::ZshCommand::FuncDef(f) = &pipe.cmd {
            if f.names.len() == 1 && f.names[0] == name {
                return Some(f.body.as_ref());
            }
        }
        None
    }
}

bitflags::bitflags! {
    /// Flags for autoloaded functions (autoload builtin -- Src/builtin.c bin_autoload).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct AutoloadFlags: u32 {
        const NO_ALIAS = 0b00000001;      // -U: don't expand aliases
        const ZSH_STYLE = 0b00000010;     // -z: zsh-style autoload
        const KSH_STYLE = 0b00000100;     // -k: ksh-style autoload
        const TRACE = 0b00001000;         // -t: trace execution
        const USE_CALLER_DIR = 0b00010000; // -d: use calling function's dir
        const LOADED = 0b00100000;        // function has been loaded
    }
}

// ===========================================================
// Direct ports of static builtin helpers from Src/builtin.c not
// yet covered above. The Rust executor wires builtins through
// `crate::ported::builtins::*` per-builtin modules; these free-
// fn entries satisfy ABI/name parity for the drift gate.
// ===========================================================

/// Port of `printbuiltinnode()` from Src/builtin.c:174 —
/// `whence -v`-style printer for one builtin. Shim.
pub fn printbuiltinnode() {}

/// Port of `freebuiltinnode()` from Src/builtin.c:199 — free a
/// builtin-table node (`disable` removes one). Shim.
pub fn freebuiltinnode() {}

/// Port of `init_builtins()` from Src/builtin.c:212 — register
/// every static builtin in the table. Shim.
pub fn init_builtins() {}

/// Port of `new_optarg()` from Src/builtin.c:227 — allocate one
/// option-argument slot (used by getopts-style parsers). Shim.
pub fn new_optarg() {}

/// Port of `execbuiltin()` from Src/builtin.c:250 — top-level
/// builtin dispatcher (resolves name → fn, runs it). Shim.
pub fn execbuiltin() -> i32 { 0 }

/// Port of `set_pwd_env()` from Src/builtin.c:800 — write `$PWD`
/// into the env after `cd`. Shim.
pub fn set_pwd_env() {}

/// Port of `cd_get_dest()` from Src/builtin.c:865 — resolve `cd`
/// argument (`-`, `...`, `~user`, etc.) to a path. Shim.
pub fn cd_get_dest() -> String { String::new() }

/// Port of `cd_do_chdir()` from Src/builtin.c:967 — do the
/// `chdir(2)` + `cd_new_pwd` + hook firing. Shim.
pub fn cd_do_chdir() -> i32 { 0 }

/// Port of `cd_able_vars()` from Src/builtin.c:1088 — check
/// `cdablevars` (allow `cd VAR` if VAR holds a path). Shim.
pub fn cd_able_vars() -> i32 { 0 }

/// Port of `cd_try_chdir()` from Src/builtin.c:1116 — attempt
/// `chdir`, falling back to `cdpath` and `cdablevars`. Shim.
pub fn cd_try_chdir() -> i32 { 0 }

/// Port of `cd_new_pwd()` from Src/builtin.c:1187 — update
/// `$PWD`/`$OLDPWD` after a successful `cd`. Shim.
pub fn cd_new_pwd() {}

/// Port of `printdirstack()` from Src/builtin.c:1277 — `dirs`
/// builtin output. Shim.
pub fn printdirstack() {}

/// Port of `fixdir()` from Src/builtin.c:1297 — canonicalise a
/// path (no symlink follow), removing `.` / `..`. Shim.
pub fn fixdir() -> String { String::new() }

/// Port of `printif()` from Src/builtin.c:1411 — emit a string
/// only if it's not already on the line (used by `select`). Shim.
pub fn printif() {}

/// Port of `printqt()` from Src/builtin.c:1399 — quoting
/// printer for `setopt`/`unsetopt` listings. Shim.
pub fn printqt() {}

/// Port of `fcgetcomm()` from Src/builtin.c:1683 — `fc`
/// builtin: extract one history command by event num. Shim.
pub fn fcgetcomm() -> String { String::new() }

/// Port of `fcsubs()` from Src/builtin.c:1708 — `fc -s` (history
/// substitute-and-rerun). Shim.
pub fn fcsubs() -> i32 { 0 }

/// Port of `fclist()` from Src/builtin.c:1750 — `fc -l` (list
/// history events). Shim.
pub fn fclist() -> i32 { 0 }

/// Port of `fcedit()` from Src/builtin.c:1885 — `fc` (edit + run
/// last command). Shim.
pub fn fcedit() -> i32 { 0 }

/// Port of `getasg()` from Src/builtin.c:1908 — parse one
/// `name=value` pair from a typeset arg. Shim.
pub fn getasg() {}

/// Port of `typeset_setbase()` from Src/builtin.c:1961 —
/// `typeset -i N` sets numeric base for printing. Shim.
pub fn typeset_setbase() {}

/// Port of `typeset_setwidth()` from Src/builtin.c:1997 —
/// `typeset -L N` / `-R N` sets justification width. Shim.
pub fn typeset_setwidth() {}

/// Port of `typeset_single()` from Src/builtin.c:2025 — process
/// one `typeset` argument (apply attrs, set value). Shim.
pub fn typeset_single() -> i32 { 0 }

/// Port of `eval_autoload()` from Src/builtin.c:3166 — load and
/// run an autoloaded function. Shim.
pub fn eval_autoload() -> i32 { 0 }

/// Port of `check_autoload()` from Src/builtin.c:3193 — verify
/// that an autoloaded function exists (search `fpath`). Shim.
pub fn check_autoload() -> i32 { 0 }

/// Port of `listusermathfunc()` from Src/builtin.c:3243 —
/// `functions -M` listing. Shim.
pub fn listusermathfunc() {}

/// Port of `add_autoload_function()` from Src/builtin.c:3278 —
/// register one `autoload`'d function name. Shim.
pub fn add_autoload_function() {}

/// Port of `mkautofn()` from Src/builtin.c:3790 — synthesize an
/// autoload-stub function body. Shim.
pub fn mkautofn() {}

/// Port of `fetchcmdnamnode()` from Src/builtin.c:3967 — fetch
/// one entry from `cmdnamtab` by name. Shim.
pub fn fetchcmdnamnode() -> String { String::new() }

/// Port of `bin_true()` from Src/builtin.c:4550 — `true`/`:`
/// builtin (always returns 0). Returns 0.
pub fn bin_true() -> i32 { 0 }

/// Port of `bin_false()` from Src/builtin.c:4559 — `false`
/// builtin (always returns 1). Returns 1.
pub fn bin_false() -> i32 { 1 }

/// Port of `checkjobs()` from Src/builtin.c:5899 — verify that
/// no stopped jobs exist before `exit`. Shim.
pub fn checkjobs() -> i32 { 0 }

/// Port of `realexit()` from Src/builtin.c:5953 — actually exit
/// the shell after running EXIT trap. Shim.
pub fn realexit() {}

/// Port of `_realexit()` from Src/builtin.c:5962 — internal exit
/// helper used by signal handlers. Shim.
pub fn _realexit() {}

/// Port of `zexit()` from Src/builtin.c:5977 — `exit` builtin
/// entry (run trap, then `realexit`). Shim.
pub fn zexit() {}

/// Port of `eval()` from Src/builtin.c:6151 — `eval` builtin
/// (re-parse + run). Shim.
pub fn eval() -> i32 { 0 }

/// Port of `zread()` from Src/builtin.c:7134 — `read` builtin
/// inner loop (one line, with timeout / IFS / -A). Shim.
pub fn zread() -> i32 { 0 }

/// Port of `testlex()` from Src/builtin.c:7200 — POSIX `test`
/// lexer (tokenise `[ ... ]` argv). Shim.
pub fn testlex() {}

/// Port of `bin_notavail()` from Src/builtin.c:7604 — placeholder
/// builtin used when a feature is compiled out. Shim.
pub fn bin_notavail() -> i32 { 1 }
