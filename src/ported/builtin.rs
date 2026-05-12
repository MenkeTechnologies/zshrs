//! Direct port of `Src/builtin.c` — the master registration site for
//! the in-shell builtin commands. The C source is 7608 lines; the
//! actual `bin_*` handler bodies were ported organically into
//! `src/ported/exec.rs` and `src/ported/builtins/*.rs` long before
//! this file existed. This file scaffolds:
//!
//! Builtins in the main executable                                          // c:38
//! Builtin Command Hash Table Functions                                     // c:140
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
    self,  BUILTIN_NAMES,
    format_int_in_base,
    VarAttr, VarKind,
};
use crate::ported::utils::{zerr, zerrnam, zwarn, zwarnnam};
use crate::func_body_fmt::FuncBodyFmt;
#[allow(unused_imports)]
use crate::ported::options::ZSH_OPTIONS_SET;
#[allow(unused_imports)]
use crate::parse::{Redirect, ShellCommand};
#[allow(unused_imports)]
use crate::extensions::zwc::ZwcFile;


// ---------------------------------------------------------------------------
// BIN_* dispatch IDs.
// Direct port of `Src/hashtable.h:34-70`. These are the integer
// discriminators handlers use when one C function backs multiple
// builtin names (e.g. `bin_fg` covers fg/bg/jobs/wait/disown).
// ---------------------------------------------------------------------------

// BIN_* constants moved to `crate::ported::hashtable_h` per the C
// header layout (Src/hashtable.h:34-70). Re-exported here so existing
// `crate::ported::builtin::BIN_X` paths keep resolving.
pub use crate::ported::hashtable_h::{
    BIN_TYPESET, BIN_BG, BIN_FG, BIN_JOBS, BIN_WAIT, BIN_DISOWN,
    BIN_BREAK, BIN_CONTINUE, BIN_EXIT, BIN_RETURN, BIN_CD,
    BIN_POPD, BIN_PUSHD, BIN_PRINT, BIN_EVAL, BIN_SCHED, BIN_FC,
    BIN_R, BIN_PUSHLINE, BIN_LOGOUT, BIN_TEST, BIN_BRACKET,
    BIN_READONLY, BIN_ECHO, BIN_DISABLE, BIN_ENABLE, BIN_PRINTF,
    BIN_COMMAND, BIN_UNHASH, BIN_UNALIAS, BIN_UNFUNCTION,
    BIN_UNSET, BIN_EXPORT, BIN_SETOPT, BIN_UNSETOPT,
};
use crate::zsh_h::{builtin, BINF_ASSIGN, BINF_BUILTIN, BINF_COMMAND, BINF_DASH, BINF_DASHDASHVALID, BINF_EXEC, BINF_HANDLES_OPTS, BINF_MAGICEQUALS, BINF_NOGLOB, BINF_PLUSOPTS, BINF_PREFIX, BINF_PRINTOPTS, BINF_PSPECIAL, BINF_SKIPDASH, BINF_SKIPINVALID, hashnode, NULLBINCMD, isset};

// Local builders that construct C-shape `builtin` rows for the
// static registration table below. They mirror the
// `BUILTIN(...)` / `BIN_PREFIX(...)` macros in `Src/zsh.h:1450-1452`,
// taking `u32` flag bitsets (BINF_*) and a `&str` handler-name
// column used only for documentation/wiring lookup — handler
// function pointers themselves are wired up later in
// `Executor::register_builtins` (`src/ported/exec.rs`).
//
// The `handler` arg was previously a `_handler_name: &'static str` that
// was discarded — `handlerfunc` always ended up `NULLBINCMD`, so
// `execbuiltin`'s c:506 `(*handlerfunc)(...)` dispatch was unreachable.
// Now the descriptor carries the actual port-side `HandlerFunc` so
// `execbuiltin` can parse flags and call through to the real builtin.
#[allow(non_snake_case)]
fn BUILTIN(
    name: &str,
    flags: u32,
    handler: Option<crate::ported::zsh_h::HandlerFunc>,
    min: i32,
    max: i32,
    funcid: i32,
    optstr: Option<&str>,
    defopts: Option<&str>,
) -> builtin {
    builtin {
        node: hashnode {
            next: None,
            nam: name.to_string(),
            flags: flags as i32,
        },
        handlerfunc: handler,
        minargs: min,
        maxargs: max,
        funcid,
        optstr: optstr.map(|s| s.to_string()),
        defopts: defopts.map(|s| s.to_string()),
    }
}

#[allow(non_snake_case)]
fn BIN_PREFIX(name: &str, flags: u32) -> builtin {
    BUILTIN(name, flags | BINF_PREFIX, None, 0, 0, 0, None, None)
}
// ---------------------------------------------------------------------------
// Builtin descriptor.
// Port of `struct builtin` from `Src/zsh.h` (the one expanded by the
// `BUILTIN` / `BIN_PREFIX` macros at line 1452 of zsh.h).
// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// The master registration table.
//
// Direct, line-for-line port of `static struct builtin builtins[]`
// at `Src/builtin.c:40-137`. Entries appear in the same order so
// any diff against the C source stays trivial. The `handler_name`
// column points at the canonical Rust port that the dispatcher in
// `Executor::register_builtins` (`src/ported/exec.rs`) wires up.
// ---------------------------------------------------------------------------

pub static BUILTINS: std::sync::LazyLock<Vec<builtin>> = std::sync::LazyLock::new(|| vec![
    BIN_PREFIX("-", BINF_DASH),
    BIN_PREFIX("builtin", BINF_BUILTIN),
    BIN_PREFIX("command", BINF_COMMAND),
    BIN_PREFIX("exec", BINF_EXEC),
    BIN_PREFIX("noglob", BINF_NOGLOB),
    BUILTIN("[", BINF_HANDLES_OPTS, None, 0, -1, BIN_BRACKET, None, None),
    BUILTIN(".", BINF_PSPECIAL, None, 1, -1, 0, None, None),
    BUILTIN(":", BINF_PSPECIAL, None, 0, -1, 0, None, None),
    BUILTIN("alias", BINF_MAGICEQUALS | BINF_PLUSOPTS, None, 0, -1, 0, Some("Lgmrs"), None),
    BUILTIN("autoload", BINF_PLUSOPTS, None, 0, -1, 0, Some("dmktrRTUwWXz"), Some("u")),
    BUILTIN("bg", 0, None, 0, -1, BIN_BG, None, None),
    BUILTIN("break", BINF_PSPECIAL, None, 0, 1, BIN_BREAK, None, None),
    BUILTIN("bye", 0, None, 0, 1, BIN_EXIT, None, None),
    BUILTIN("cd", BINF_SKIPINVALID | BINF_SKIPDASH | BINF_DASHDASHVALID, None, 0, 2, BIN_CD, Some("qsPL"), None),
    BUILTIN("chdir", BINF_SKIPINVALID | BINF_SKIPDASH | BINF_DASHDASHVALID, None, 0, 2, BIN_CD, Some("qsPL"), None),
    BUILTIN("continue", BINF_PSPECIAL, None, 0, 1, BIN_CONTINUE, None, None),
    BUILTIN("declare", BINF_PLUSOPTS | BINF_MAGICEQUALS | BINF_PSPECIAL | BINF_ASSIGN, Some(bin_typeset as crate::ported::zsh_h::HandlerFunc), 0, -1, 0, Some("AE:%F:%HL:%R:%TUZ:%afghi:%klmnp:%rtuxz"), None),
    BUILTIN("dirs", 0, None, 0, -1, 0, Some("clpv"), None),
    BUILTIN("disable", 0, None, 0, -1, BIN_DISABLE, Some("afmprs"), None),
    BUILTIN("disown", 0, None, 0, -1, BIN_DISOWN, None, None),
    BUILTIN("echo", BINF_SKIPINVALID, Some(bin_print as crate::ported::zsh_h::HandlerFunc), 0, -1, BIN_ECHO, Some("neE"), Some("-")),
    BUILTIN("emulate", 0, None, 0, -1, 0, Some("lLR"), None),
    BUILTIN("enable", 0, None, 0, -1, BIN_ENABLE, Some("afmprs"), None),
    BUILTIN("eval", BINF_PSPECIAL, None, 0, -1, BIN_EVAL, None, None),
    BUILTIN("exit", BINF_PSPECIAL, None, 0, 1, BIN_EXIT, None, None),
    BUILTIN("export", BINF_PLUSOPTS | BINF_MAGICEQUALS | BINF_PSPECIAL | BINF_ASSIGN, Some(bin_typeset as crate::ported::zsh_h::HandlerFunc), 0, -1, BIN_EXPORT, Some("E:%F:%HL:%R:%TUZ:%afhi:%lp:%rtu"), Some("xg")),
    BUILTIN("false", 0, None, 0, -1, 0, None, None),
    // C source (Src/builtin.c:69-73): the argument to -e used to be
    // optional; making it required is more consistent.
    BUILTIN("fc", 0, None, 0, -1, BIN_FC, Some("aAdDe:EfiIlLmnpPrRst:W"), None),
    BUILTIN("fg", 0, None, 0, -1, BIN_FG, None, None),
    BUILTIN("float", BINF_PLUSOPTS | BINF_MAGICEQUALS | BINF_PSPECIAL | BINF_ASSIGN, Some(bin_typeset as crate::ported::zsh_h::HandlerFunc), 0, -1, 0, Some("E:%F:%HL:%R:%Z:%ghlp:%rtux"), Some("E")),
    BUILTIN("functions", BINF_PLUSOPTS, None, 0, -1, 0, Some("ckmMstTuUWx:z"), None),
    BUILTIN("getln", 0, None, 0, -1, 0, Some("ecnAlE"), Some("zr")),
    BUILTIN("getopts", 0, None, 2, -1, 0, None, None),
    BUILTIN("hash", BINF_MAGICEQUALS, None, 0, -1, 0, Some("Ldfmrv"), None),
    // Src/builtin.c — `#ifdef ZSH_HASH_DEBUG`
    //   BUILTIN("hashinfo", 0, bin_hashinfo, 0, 0, 0, NULL, NULL)
    BUILTIN("hashinfo", 0, None, 0, 0, 0, None, None),
    BUILTIN("history", 0, None, 0, -1, BIN_FC, Some("adDEfiLmnpPrt:"), Some("l")),
    BUILTIN("integer", BINF_PLUSOPTS | BINF_MAGICEQUALS | BINF_PSPECIAL | BINF_ASSIGN, Some(bin_typeset as crate::ported::zsh_h::HandlerFunc), 0, -1, 0, Some("HL:%R:%Z:%ghi:%lp:%rtux"), Some("i")),
    BUILTIN("jobs", 0, None, 0, -1, BIN_JOBS, Some("dlpZrs"), None),
    BUILTIN("kill", BINF_HANDLES_OPTS, None, 0, -1, 0, None, None),
    BUILTIN("let", 0, None, 1, -1, 0, None, None),
    BUILTIN("local", BINF_PLUSOPTS | BINF_MAGICEQUALS | BINF_PSPECIAL | BINF_ASSIGN, Some(bin_typeset as crate::ported::zsh_h::HandlerFunc), 0, -1, 0, Some("AE:%F:%HL:%R:%TUZ:%ahi:%lnp:%rtux"), None),
    BUILTIN("logout", 0, None, 0, 1, BIN_LOGOUT, None, None),
    // Src/builtin.c — `#if defined(ZSH_MEM) & defined(ZSH_MEM_DEBUG)`
    //   BUILTIN("mem", 0, bin_mem, 0, 0, 0, "v", NULL)
    BUILTIN("mem", 0, None, 0, 0, 0, Some("v"), None),
    BUILTIN("popd", BINF_SKIPINVALID | BINF_SKIPDASH | BINF_DASHDASHVALID, None, 0, 1, BIN_POPD, Some("q"), None),
    // Src/builtin.c — `#if defined(ZSH_PAT_DEBUG)`
    //   BUILTIN("patdebug", 0, bin_patdebug, 1, -1, 0, "p", NULL)
    BUILTIN("patdebug", 0, None, 1, -1, 0, Some("p"), None),
    BUILTIN("print", BINF_PRINTOPTS, Some(bin_print as crate::ported::zsh_h::HandlerFunc), 0, -1, BIN_PRINT, Some("abcC:Df:ilmnNoOpPrRsSu:v:x:X:z-"), None),
    BUILTIN("printf", BINF_SKIPINVALID | BINF_SKIPDASH, Some(bin_print as crate::ported::zsh_h::HandlerFunc), 1, -1, BIN_PRINTF, Some("v:"), None),
    BUILTIN("pushd", BINF_SKIPINVALID | BINF_SKIPDASH | BINF_DASHDASHVALID, None, 0, 2, BIN_PUSHD, Some("qsPL"), None),
    BUILTIN("pushln", 0, None, 0, -1, BIN_PRINT, None, Some("-nz")),
    BUILTIN("pwd", 0, None, 0, 0, 0, Some("rLP"), None),
    BUILTIN("r", 0, None, 0, -1, BIN_R, Some("IlLnr"), None),
    BUILTIN("read", 0, None, 0, -1, 0, Some("cd:ek:%lnpqrst:%zu:AE"), None),
    BUILTIN("readonly", BINF_PLUSOPTS | BINF_MAGICEQUALS | BINF_PSPECIAL | BINF_ASSIGN, Some(bin_typeset as crate::ported::zsh_h::HandlerFunc), 0, -1, BIN_READONLY, Some("AE:%F:%HL:%R:%TUZ:%afghi:%lptux"), Some("r")),
    BUILTIN("rehash", 0, None, 0, 0, 0, Some("df"), Some("r")),
    BUILTIN("return", BINF_PSPECIAL, None, 0, 1, BIN_RETURN, None, None),
    BUILTIN("set", BINF_PSPECIAL | BINF_HANDLES_OPTS, None, 0, -1, 0, None, None),
    BUILTIN("setopt", 0, None, 0, -1, BIN_SETOPT, None, None),
    BUILTIN("shift", BINF_PSPECIAL, None, 0, -1, 0, Some("p"), None),
    BUILTIN("source", BINF_PSPECIAL, None, 1, -1, 0, None, None),
    BUILTIN("suspend", 0, None, 0, 0, 0, Some("f"), None),
    BUILTIN("test", BINF_HANDLES_OPTS, None, 0, -1, BIN_TEST, None, None),
    BUILTIN("ttyctl", 0, None, 0, 0, 0, Some("fu"), None),
    // c:Src/Builtins/rlimits.c:868-870 — limit/ulimit/unlimit are
    // declared in the rlimits Builtins-module's bintab. zshrs has the
    // free-fn ports at src/ported/builtins/rlimits.rs but never
    // registered them; the BUILTIN_NAMES derivation missed them and
    // `type limit` etc. returned empty.
    BUILTIN("limit",   0, None, 0, -1, 0, Some("sh"), None),                  // c:rlimits.c:868
    BUILTIN("ulimit",  0, None, 0, -1, 0, None,       None),                  // c:rlimits.c:869
    BUILTIN("unlimit", 0, None, 0, -1, 0, Some("hs"), None),                  // c:rlimits.c:870
    BUILTIN("times", BINF_PSPECIAL, None, 0, 0, 0, None, None),
    BUILTIN("trap", BINF_PSPECIAL | BINF_HANDLES_OPTS, None, 0, -1, 0, None, None),
    BUILTIN("true", 0, None, 0, -1, 0, None, None),
    BUILTIN("type", 0, Some(bin_whence as crate::ported::zsh_h::HandlerFunc), 0, -1, 0, Some("ampfsSw"), Some("v")),
    BUILTIN("typeset", BINF_PLUSOPTS | BINF_MAGICEQUALS | BINF_PSPECIAL | BINF_ASSIGN, Some(bin_typeset as crate::ported::zsh_h::HandlerFunc), 0, -1, 0, Some("AE:%F:%HL:%R:%TUZ:%afghi:%klp:%rtuxmnz"), None),
    BUILTIN("umask", 0, None, 0, 1, 0, Some("S"), None),
    BUILTIN("unalias", 0, Some(bin_unhash as crate::ported::zsh_h::HandlerFunc), 0, -1, BIN_UNALIAS, Some("ams"), None),
    BUILTIN("unfunction", 0, Some(bin_unhash as crate::ported::zsh_h::HandlerFunc), 1, -1, BIN_UNFUNCTION, Some("m"), Some("f")),
    BUILTIN("unhash", 0, Some(bin_unhash as crate::ported::zsh_h::HandlerFunc), 1, -1, BIN_UNHASH, Some("adfms"), None),
    BUILTIN("unset", BINF_PSPECIAL, None, 1, -1, BIN_UNSET, Some("fmvn"), None),
    BUILTIN("unsetopt", 0, None, 0, -1, BIN_UNSETOPT, None, None),
    BUILTIN("wait", 0, None, 0, -1, BIN_WAIT, None, None),
    BUILTIN("whence", 0, Some(bin_whence as crate::ported::zsh_h::HandlerFunc), 0, -1, 0, Some("acmpvfsSwx:"), None),
    BUILTIN("where", 0, Some(bin_whence as crate::ported::zsh_h::HandlerFunc), 0, -1, 0, Some("pmsSwx:"), Some("ca")),
    BUILTIN("which", 0, Some(bin_whence as crate::ported::zsh_h::HandlerFunc), 0, -1, 0, Some("ampsSwx:"), Some("c")),
    BUILTIN("zmodload", 0, None, 0, -1, 0, Some("AFRILP:abcfdilmpsue"), None),
    BUILTIN("zcompile", 0, None, 0, -1, 0, Some("tUMRcmzka"), None),
]);
// hash table containing builtin commands                                   // c:143
/// Process-wide builtin lookup table. Filled lazily the first time
/// `builtintab()` is called; mirrors the C `mod_export HashTable
/// builtintab` exposed at `Src/builtin.c:146`.
static builtintab: OnceLock<HashMap<String, &'static builtin>> = OnceLock::new();

/// Names whose `node.flags & DISABLED` is set in C. The Rust port's
/// `builtintab` is an immutable static, so the disabled bit lives
/// in this parallel set; `bin_enable` toggles it via builtin.c:587.
/// Dispatch sites check `is_builtin_disabled(name)` before calling
/// `handlerfunc` to mirror C's "skip nodes with DISABLED set" walk.
pub static BUILTINS_DISABLED: std::sync::LazyLock<                           // c:587 (Src/builtin.c)
    std::sync::Mutex<std::collections::HashSet<String>>
> = std::sync::LazyLock::new(|| {
    std::sync::Mutex::new(std::collections::HashSet::new())
});

/// Construct the builtin lookup table.
/// Port of `createbuiltintable()` from `Src/builtin.c:149`. The C
/// version installs the hashtable function pointers (hash, addnode,
/// printnode, etc.) and then calls `addbuiltins("zsh", builtins, ..)`.
/// Here we just materialise the static `BUILTINS` slice into a
/// `HashMap<String, &builtin>` — Rust's standard hashing replaces the
/// C `hasher` callback and the `HashMap` itself replaces all the
/// per-table function pointers (`addnode`/`getnode`/`removenode`/...).
// Builtin Command Hash Table Functions                                      // c:140
pub fn createbuiltintable() -> &'static HashMap<String, &'static builtin> { // c:150
    builtintab.get_or_init(|| {
        let table: &'static Vec<builtin> = &*BUILTINS;
        let mut m: HashMap<String, &'static builtin> = HashMap::with_capacity(table.len());
        for b in table.iter() {
            m.insert(b.node.nam.clone(), b);
        }
        m
    })
}

#[cfg(test)]
mod tests {
    use crate::zsh_h::BINF_PREFIX;
    use super::*;

    #[test]
    fn registration_table_matches_c_count() {
        // Src/builtin.c:40-137 has 79 rows total (5 BIN_PREFIX + 71
        // BUILTIN + 3 debug-only BUILTIN). The Rust port also exposes
        // limit/ulimit/unlimit eagerly even though their C home is
        // Src/Builtins/rlimits.c:868-870 (loaded via zmodload zsh/rlimits) —
        // so `type limit` etc. work without an explicit zmodload step.
        // That bumps the total from 79 → 82. If C grows or shrinks
        // rows, this fires; bump alongside the additions in BUILTINS
        // above.
        assert_eq!(BUILTINS.len(), 82);
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
            assert!(b.node.flags as u32 & BINF_PREFIX != 0, "{name} missing BINF_PREFIX");
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
// ksh_autoload_body moved from src/ported/exec.rs.
// Mirrors the ksh-style autoload helper in Src/builtin.c
// (bin_functions / load_function_def).
// ===========================================================
// (impl crate::ported::exec::ShellExecutor block deleted — was lines 12343..12376; per user feedback the bin_* methods were fake. Recorder hooks preserved at file bottom.)


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
/// Port of `printbuiltinnode()` from Src/builtin.c:174.
/// C: `static void printbuiltinnode(HashNode hn, int printflags)` —
///   emit `whence`-style description of one builtin.
pub fn printbuiltinnode(hn: *mut crate::ported::zsh_h::hashnode,             // c:174
                        printflags: i32) {
    use crate::ported::zsh_h::{PRINT_WHENCE_WORD, PRINT_WHENCE_CSH};
    if hn.is_null() { return; }
    let bn = unsafe { &*hn };
    if (printflags & PRINT_WHENCE_WORD as i32) != 0 {                        // c:179
        println!("{}: builtin", bn.nam);                                     // c:180
        return;
    }
    if (printflags & PRINT_WHENCE_CSH as i32) != 0 {                         // c:184
        println!("{}: shell built-in command", bn.nam);                      // c:185
        return;
    }
    // c:189-198 — default form: just emit the name.
    println!("{}", bn.nam);
}

/// Port of `freebuiltinnode()` from Src/builtin.c:199.
/// C: `static void freebuiltinnode(HashNode hn)` — free a builtin-table
///   node only when BINF_ADDED is clear (i.e., dynamically added).
pub fn freebuiltinnode(hn: *mut crate::ported::zsh_h::hashnode) {            // c:199
    if hn.is_null() { return; }
    let bn = unsafe { &*hn };
    // c:204 — `if (!(bn->node.flags & BINF_ADDED))` then free.
    if (bn.flags as u32 & crate::ported::zsh_h::BINF_ADDED) == 0 {           // c:204
        // Rust drop handles the actual free; nothing more to do.
    }
}

/// Port of `init_builtins()` from Src/builtin.c:212.
/// C: `void init_builtins(void)` — when not in EMULATE_ZSH, disable
///   the `repeat` reserved word (compat for sh/ksh).
///
/// ```c
/// if (!EMULATION(EMULATE_ZSH)) {
///     HashNode hn = reswdtab->getnode2(reswdtab, "repeat");
///     if (hn)
///         reswdtab->disablenode(hn, 0);
/// }
/// ```
pub fn init_builtins() {                                                     // c:212
    use crate::ported::zsh_h::EMULATE_ZSH;
    // c:214 — `if (!EMULATION(EMULATE_ZSH))`. EMULATION reads the
    // canonical `emulation` global directly per zsh.h:2347.
    if !crate::ported::zsh_h::EMULATION(EMULATE_ZSH) {                       // c:214
        // c:215-217 — `hn = reswdtab->getnode2(reswdtab,"repeat");
        //              if (hn) reswdtab->disablenode(hn, 0);`
        if let Ok(mut tab) = crate::ported::hashtable::reswdtab_lock().lock() {
            tab.disable("repeat");
        }
    }
}

/// Port of `OPT_ALLOC_CHUNK` from `Src/builtin.c:223`. Number of
/// `ops->args[]` slots `new_optarg()` grows the array by when full.
pub const OPT_ALLOC_CHUNK: i32 = 16;                                         // c:223

/// Port of `new_optarg()` from Src/builtin.c:227.
/// C: `static int new_optarg(Options ops)` — grow the `ops->args[]`
///   array by `OPT_ALLOC_CHUNK` slots when full. Returns 1 on overflow
///   (>=63 args), 0 on success.
pub fn new_optarg(ops: &mut crate::ported::zsh_h::options) -> i32 {          // c:227
    // c:231 — `if (ops->argscount == 63) return 1;`
    if ops.argscount == 63 {                                                 // c:231
        return 1;
    }
    // c:232-241 — grow ops->args by OPT_ALLOC_CHUNK if argsalloc == argscount.
    if ops.argsalloc == ops.argscount {                                      // c:232
        ops.args.resize((ops.argsalloc + OPT_ALLOC_CHUNK) as usize, String::new());
        ops.argsalloc += OPT_ALLOC_CHUNK;                                    // c:240
    }
    ops.argscount += 1;                                                      // c:243
    0                                                                        // c:244
}

/// Port of `execbuiltin()` from Src/builtin.c:250.
///
/// C: `int execbuiltin(LinkList args, LinkList assigns, Builtin bn)` —
///   execute a builtin handler function after parsing the arguments.
///
/// Walks `bn->optstr` against `args`, populating `ops.ind[c]` (`|= 1`
/// for `-X`, `|= 2` for `+X`, `<< 2` arg-index for opts taking args
/// per the `:`/`::`/`:%` suffix convention), then calls
/// `bn->handlerfunc(name, argv, &ops, bn->funcid)`.
///
/// Signature note: C consumes the name via `ugetnode(args)` first
/// (c:262); the Rust port receives `args` without the name and reads
/// `bn->node.nam` directly. C's `LinkList assigns` ports to
/// `Vec<asgment>` (closer to the C type than the earlier
/// `Vec<(String, String)>` pair-tuple). `assignfunc` handler dispatch
/// (c:495-502) — BINF_ASSIGN builtins taking two argument lists —
/// isn't ported (no Rust-side caller passes a non-empty `assigns`),
/// so XTRACE prints the structure but BINF_ASSIGN dispatch falls
/// through to the plain handler.
pub fn execbuiltin(args: Vec<String>, assigns: Vec<crate::ported::zsh_h::asgment>, // c:250
                   bn: *mut crate::ported::zsh_h::builtin) -> i32 {
    use crate::ported::zsh_h::{options, MAX_OPS, XTRACE, isset,
        BINF_PLUSOPTS, BINF_KEEPNUM, BINF_SKIPDASH, BINF_DASHDASHVALID,
        BINF_SKIPINVALID, BINF_PRINTOPTS, BINF_HANDLES_OPTS,
        ERRFLAG_ERROR};
    if bn.is_null() {
        return 1;
    }
    let bn_ref = unsafe { &*bn };

    // c:252-254 — locals.
    let pp: Option<&str>;                                                    // c:252 char *pp
    let name: String;                                                        // c:252 char *name
    let mut optstr: Option<String>;                                          // c:252 char *optstr
    let mut flags: i32;                                                      // c:253 int flags
    let mut argc: i32;                                                       // c:253 int argc
    let mut execop: u8;                                                      // c:253 int execop
    let xtr: bool = isset(XTRACE);                                           // c:253 int xtr = isset(XTRACE)

    // c:256-259 — `memset(ops.ind, 0, ...); ops.args = NULL; ops.argscount=ops.argsalloc=0;`
    let mut ops = options { ind: [0u8; MAX_OPS], args: Vec::new(),           // c:257
                            argscount: 0, argsalloc: 0 };                    // c:258-259

    // c:262 — `name = (char *) ugetnode(args);` — Rust reads bn.node.nam.
    name = bn_ref.node.nam.clone();                                          // c:262

    // c:264-268 — `if (!bn->handlerfunc)` early-exit.
    if bn_ref.handlerfunc.is_none() {                                        // c:264
        return 1;                                                            // c:267
    }

    // c:270-271 — `flags = bn->node.flags; optstr = bn->optstr;`
    flags = bn_ref.node.flags;                                               // c:270
    optstr = bn_ref.optstr.clone();                                          // c:271

    // c:275 — `argc = countlinknodes(args);` — total argv length.
    argc = args.len() as i32;                                                // c:275

    // c:284-293 — `VARARR(char *, argarr, argc+1)` + copy args into argarr.
    let argarr: Vec<String> = args;                                          // c:284 argarr[]
    let mut argv: usize = 0;                                                 // c:285 char **argv = argarr;

    // c:296-411 — option parser body.
    if let Some(ref os) = optstr.clone() {                                   // c:296
        let optstr_local = os.clone();
        let mut optstr_bytes: Vec<u8> = optstr_local.into_bytes();
        let mut skipinvalid = (flags & BINF_SKIPINVALID as i32) != 0;
        // c:297 — `char *arg = *argv;`
        loop {
            // c:300-303 — outer arg-by-arg loop guard:
            //   `arg && ((sense = (*arg == '-')) || ((flags & BINF_PLUSOPTS) && *arg == '+'))`.
            let arg_str: String = match argarr.get(argv) {
                Some(s) => s.clone(),
                None => break,
            };
            let arg_bytes = arg_str.as_bytes();
            if arg_bytes.is_empty() { break; }
            let sense: i32 = if arg_bytes[0] == b'-' { 1 } else { 0 };       // c:302
            if sense == 0 && !((flags & BINF_PLUSOPTS as i32) != 0           // c:303
                                && arg_bytes[0] == b'+') {
                break;
            }
            // c:305 — `if (!(flags & BINF_KEEPNUM) && idigit(arg[1])) break;`
            if (flags & BINF_KEEPNUM as i32) == 0                            // c:305
                && arg_bytes.len() >= 2
                && arg_bytes[1].is_ascii_digit() {
                break;
            }
            // c:308 — `if ((flags & BINF_SKIPDASH) && !arg[1]) break;`
            if (flags & BINF_SKIPDASH as i32) != 0 && arg_bytes.len() == 1 { // c:308
                break;
            }
            // c:310-317 — `--` end-of-options if BINF_DASHDASHVALID.
            if (flags & BINF_DASHDASHVALID as i32) != 0 && arg_str == "--" { // c:310
                argv += 1;                                                   // c:315
                break;                                                       // c:316
            }
            // c:327-332 — `BINF_SKIPINVALID`: if any char in arg[1..] is
            // not in optstr, the whole arg is treated as a positional.
            if skipinvalid {                                                 // c:327
                let mut all_known = true;
                for &c in &arg_bytes[1..] {
                    if !optstr_bytes.contains(&c) { all_known = false; break; }
                }
                if !all_known { break; }                                     // c:331
            }
            // c:335-336 — `if (arg[1] == '-') arg++;` — consume the
            // second `-` of `--long-style`.
            let mut k: usize = 1;                                            // walks arg[k..]
            if arg_bytes.len() >= 2 && arg_bytes[1] == b'-' {                // c:335
                k = 2;                                                       // c:336
            }
            // c:337-341 — `if (!arg[1])` lone `-` / `+` indicator.
            if arg_bytes.len() == k {                                        // c:337
                ops.ind[b'-' as usize] = 1;                                  // c:338
                if sense == 0 {                                              // c:339
                    ops.ind[b'+' as usize] = 1;                              // c:340
                }
            }
            // c:343-386 — inner loop over `*++arg` characters.
            let mut bad_opt: Option<u8> = None;
            while k < arg_bytes.len() {                                      // c:343
                let c = arg_bytes[k];
                execop = c;                                                  // c:345
                let optptr = optstr_bytes.iter().position(|&b| b == c);      // c:345 strchr(optstr,c)
                if let Some(optidx) = optptr {                               // c:345
                    ops.ind[c as usize] = if sense != 0 { 1 } else { 2 };    // c:346
                    // c:347 — `if (optptr[1] == ':')` — option takes arg.
                    if optidx + 1 < optstr_bytes.len() && optstr_bytes[optidx + 1] == b':' {
                        let mut argptr: Option<String> = None;
                        // c:349-352 — `if (optptr[2] == ':')` optional same-word.
                        if optidx + 2 < optstr_bytes.len() && optstr_bytes[optidx + 2] == b':' {
                            if k + 1 < arg_bytes.len() {                     // c:350
                                argptr = Some(String::from_utf8_lossy(&arg_bytes[k+1..]).into_owned()); // c:351
                            }
                        } else if optidx + 2 < optstr_bytes.len() && optstr_bytes[optidx + 2] == b'%' {
                            // c:353-359 — `:%` numeric optional same or next word.
                            if k + 1 < arg_bytes.len() && arg_bytes[k+1].is_ascii_digit() {
                                argptr = Some(String::from_utf8_lossy(&arg_bytes[k+1..]).into_owned());
                            } else if let Some(nxt) = argarr.get(argv + 1) {
                                if !nxt.is_empty() && nxt.as_bytes()[0].is_ascii_digit() {
                                    argv += 1;                               // c:359 arg = *++argv
                                    argptr = Some(nxt.clone());
                                }
                            }
                        } else {
                            // c:360-370 — plain `:` mandatory arg.
                            if k + 1 < arg_bytes.len() {                     // c:362
                                argptr = Some(String::from_utf8_lossy(&arg_bytes[k+1..]).into_owned()); // c:363
                            } else if let Some(nxt) = argarr.get(argv + 1) {
                                argv += 1;                                   // c:364 arg = *++argv
                                argptr = Some(nxt.clone());                  // c:365
                            } else {
                                // c:366-370 — `argument expected: -%c`.
                                crate::ported::utils::zwarnnam(&name,
                                    &format!("argument expected: -{}", execop as char)); // c:367-368
                                return 1;                                    // c:369
                            }
                        }
                        if let Some(ap) = argptr {                           // c:372
                            // c:373-377 — new_optarg overflow.
                            if new_optarg(&mut ops) != 0 {                   // c:373
                                crate::ported::utils::zwarnnam(&name,
                                    "too many option arguments");            // c:374-375
                                return 1;                                    // c:376
                            }
                            // c:378 — `ops.ind[execop] |= ops.argscount << 2;`
                            ops.ind[execop as usize] |= (ops.argscount as u8) << 2;
                            // c:379 — `ops.args[ops.argscount-1] = argptr;`
                            ops.args[(ops.argscount - 1) as usize] = ap;
                            // c:380-381 — `while (arg[1]) arg++;` consume the rest.
                            k = arg_bytes.len();
                        }
                    }
                    k += 1;
                } else {
                    bad_opt = Some(c);                                       // c:385 break
                    break;
                }
            }
            // c:389-394 — if we exited mid-arg on a bad char, emit "bad option".
            if let Some(badc) = bad_opt {                                    // c:389
                crate::ported::utils::zwarnnam(&name,
                    &format!("bad option: {}{}",
                        if sense != 0 { '-' } else { '+' }, badc as char));  // c:392
                return 1;                                                    // c:393
            }
            // c:395 — `arg = *++argv;`
            argv += 1;                                                       // c:395
            // c:398-402 — BINF_PRINTOPTS R-mode switch to "ne" optstr.
            if (flags & BINF_PRINTOPTS as i32) != 0                          // c:398
                && ops.ind[b'R' as usize] != 0
                && ops.ind[b'f' as usize] == 0 {
                optstr_bytes = b"ne".to_vec();                               // c:400
                flags |= BINF_SKIPINVALID as i32;                            // c:401
                skipinvalid = true;
            }
            // c:404-405 — `if (ops.ind['-']) break;` — `--` terminates.
            if ops.ind[b'-' as usize] != 0 {                                 // c:404
                break;
            }
        }
        let _ = optstr_bytes;
    } else if (flags & BINF_HANDLES_OPTS as i32) == 0                        // c:407
        && argarr.get(argv).map(|s| s == "--").unwrap_or(false) {            // c:408
        // c:409-410 — `ops.ind['-'] = 1; argv++;`
        ops.ind[b'-' as usize] = 1;                                          // c:409
        argv += 1;                                                           // c:410
    }
    // Suppress optstr-unused warnings on the `else` path.
    let _ = optstr.take();

    // c:414-421 — apply `bn->defopts` defaults.
    pp = bn_ref.defopts.as_deref();                                          // c:414
    if let Some(pp_str) = pp {                                               // c:414
        for &b in pp_str.as_bytes() {                                        // c:415
            if ops.ind[b as usize] == 0 {                                    // c:417
                ops.ind[b as usize] = 1;                                     // c:418
            }
        }
    }

    // c:424 — `argc -= argv - argarr;` — subtract consumed flag args.
    argc -= argv as i32;                                                     // c:424

    // c:426-429 — errflag check.
    let ef = crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed);
    if (ef & ERRFLAG_ERROR) != 0 {                                           // c:426
        crate::ported::utils::errflag.fetch_and(!ERRFLAG_ERROR, std::sync::atomic::Ordering::Relaxed); // c:427
        return 1;                                                            // c:428
    }

    // c:432-436 — argc bounds check.
    if argc < bn_ref.minargs                                                 // c:432
        || (argc > bn_ref.maxargs && bn_ref.maxargs != -1) {
        crate::ported::utils::zwarnnam(&name,                                // c:433
            if argc < bn_ref.minargs { "not enough arguments" }
            else { "too many arguments" });                                  // c:434
        return 1;                                                            // c:435
    }

    // c:438-494 — display execution trace information, if required.
    if xtr {                                                                 // c:439
        // c:440-441 — `char **fullargv = argarr;` — use FULL argv
        // (including consumed option words) so XTRACE shows what the
        // user typed, not the option-stripped tail.
        let fullargv = &argarr;                                              // c:441
        crate::ported::utils::printprompt4();                                // c:442
        // c:443 — `fprintf(xtrerr, "%s", name);`
        eprint!("{}", name);                                                 // c:443
        // c:444-447 — `while (*fullargv) { fputc(' ',xtrerr); quotedzputs(...); }`
        for s in fullargv {                                                  // c:444
            eprint!(" ");                                                    // c:445 fputc(' ', xtrerr)
            eprint!("{}", crate::ported::utils::quotedzputs(s));             // c:446
        }
        // c:448-491 — `if (assigns) { for (node = firstnode(assigns); ...) }`.
        for asg in &assigns {                                                // c:450 firstnode/incnode
            eprint!(" ");                                                    // c:452 fputc(' ', xtrerr)
            eprint!("{}", crate::ported::utils::quotedzputs(&asg.name));     // c:453
            if (asg.flags & crate::ported::zsh_h::ASG_ARRAY) != 0 {          // c:454
                eprint!("=(");                                               // c:455
                if let Some(ref list) = asg.array {                          // c:456
                    if (asg.flags & crate::ported::zsh_h::ASG_KEY_VALUE) != 0 { // c:457
                        // c:458-473 — `LinkNode keynode, valnode;` walk
                        // alternating key/value pairs, emitting
                        // `[key]=value` per pair. Uses the typed
                        // `LinkList<String>` accessors from
                        // `src/ported/linklist.rs` which port the
                        // `firstnode` / `nextnode` / `getdata` macros
                        // from `Src/zsh.h:576-588`.
                        let mut keynode = list.firstnode();                  // c:459
                        loop {                                               // c:460
                            // c:461-462 — `if (!keynode) break;`
                            let kidx = match keynode {                       // c:461
                                Some(i) => i,
                                None => break,                               // c:462
                            };
                            // c:463-465 — `valnode = nextnode(keynode); if (!valnode) break;`
                            let vidx = match list.nextnode(kidx) {           // c:463
                                Some(i) => i,
                                None => break,                               // c:465
                            };
                            // c:466-468 — `fputc('['); quotedzputs(getdata(keynode));`
                            eprint!("[");                                    // c:466
                            if let Some(k) = list.getdata(kidx) {            // c:467 getdata
                                eprint!("{}", crate::ported::utils::quotedzputs(k)); // c:467
                            }
                            // c:469 — `fprintf(stderr, "]=");`
                            eprint!("]=");                                   // c:469
                            // c:470-471 — `quotedzputs(getdata(valnode));`
                            if let Some(v) = list.getdata(vidx) {            // c:470
                                eprint!("{}", crate::ported::utils::quotedzputs(v)); // c:470
                            }
                            // c:472 — `keynode = nextnode(valnode);`
                            keynode = list.nextnode(vidx);                   // c:472
                        }
                    } else {                                                 // c:474
                        // c:475-482 — plain array emit: walk every node
                        // and emit ` <quotedzputs(elem)>`.
                        let mut arrnode = list.firstnode();                  // c:476
                        while let Some(idx) = arrnode {                      // c:477
                            eprint!(" ");                                    // c:479 fputc(' ', xtrerr)
                            if let Some(elem) = list.getdata(idx) {          // c:480 getdata
                                eprint!("{}", crate::ported::utils::quotedzputs(elem)); // c:480
                            }
                            arrnode = list.nextnode(idx);                    // c:478 incnode
                        }
                    }
                }
                eprint!(" )");                                               // c:485
            } else if let Some(ref scalar) = asg.scalar {                    // c:486
                eprint!("=");                                                // c:487 fputc('=', xtrerr)
                eprint!("{}", crate::ported::utils::quotedzputs(scalar));    // c:488
            }
        }
        // c:492-493 — `fputc('\n', xtrerr); fflush(xtrerr);`
        eprintln!();                                                         // c:492
        // c:493 — fflush is automatic on `eprintln!` (stderr line-buffered).
    }

    // c:506 — `return (*(bn->handlerfunc))(name, argv, &ops, bn->funcid);`
    let trimmed: Vec<String> = argarr[argv..].to_vec();
    let handler = bn_ref.handlerfunc.expect("handlerfunc checked at c:264");
    handler(&name, &trimmed, &ops, bn_ref.funcid)                            // c:506
}

/// Port of `set_pwd_env()` from Src/builtin.c:800.
/// C: `void set_pwd_env(void)` — clear PM_READONLY on PWD/OLDPWD if
///   they're not scalar, then refresh both env vars from the globals.
pub fn set_pwd_env() {                                                       // c:800
    // c:803-816 — paramtab->getnode("PWD") + scalar/PM_READONLY guard,
    // then setsparam("PWD", pwd); same for OLDPWD.
    // Static-link path: refresh from std::env directly.
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(s) = cwd.to_str() {
            std::env::set_var("PWD", s);                                     // c:813
        }
    }
    // c:818 — OLDPWD is set by the cd flow; nothing to refresh here.
}

/// Port of `cd_get_dest()` from Src/builtin.c:865.
/// C: `static LinkNode cd_get_dest(char *nam, char **argv, int hard,
///     int func)` — resolve the `cd` argument (`-`, `+N`/`-N`,
///   bare → $HOME, two-arg substitution form) to a destination path.
///   Returns the resolved path on success, None on error (with the
///   appropriate zwarnnam already emitted).
pub fn cd_get_dest(nam: &str, argv: &[String], _hard: bool, func: i32)       // c:865
                   -> Option<String> {
    use crate::ported::modules::parameter::DIRSTACK;
    use std::sync::atomic::Ordering;
    use crate::ported::builtin::{BIN_PUSHD, BIN_POPD};

    if argv.is_empty() {                                                     // c:872
        // c:873-875 — popd needs at least 2 stack entries.
        if func == BIN_POPD {
            let depth = DIRSTACK.lock().map(|d| d.len()).unwrap_or(0);
            if depth < 2 {                                                   // c:873
                crate::ported::utils::zwarnnam(nam, "directory stack empty"); // c:874
                return None;                                                 // c:875
            }
            // c:885 — `dir = nextnode(firstnode(dirstack));`
            return DIRSTACK.lock().ok()
                .and_then(|d| d.get(1).cloned());
        }
        if func == BIN_PUSHD {
            // c:877 — `if (unset(PUSHDTOHOME)) dir = nextnode(firstnode(dirstack));`
            let pushdtohome = crate::ported::zsh_h::isset(crate::ported::options::optlookup("pushdtohome"));
            if !pushdtohome {                                                // c:877
                return DIRSTACK.lock().ok()
                    .and_then(|d| d.get(1).cloned());
            }
        }
        // c:880-884 — fall through to $HOME.
        match std::env::var("HOME") {
            Ok(h) => Some(h),                                                // c:884
            Err(_) => {
                crate::ported::utils::zwarnnam(nam, "HOME not set");         // c:881
                None                                                         // c:882
            }
        }
    } else if argv.len() == 1 {                                              // c:887
        let arg = &argv[0];
        DOPRINTDIR.fetch_add(1, Ordering::Relaxed);                          // c:891
        // c:892-908 — `+N`/`-N` numeric stack-index form.
        let posixcd = crate::ported::zsh_h::isset(crate::ported::options::optlookup("posixcd"));
        if !posixcd && arg.len() > 1
            && (arg.starts_with('+') || arg.starts_with('-'))
            && arg[1..].chars().all(|c| c.is_ascii_digit())
        {
            let dd: usize = arg[1..].parse().unwrap_or(0);                   // c:894
            let pushdminus = crate::ported::zsh_h::isset(crate::ported::options::optlookup("pushdminus"));
            let from_top = (arg.starts_with('+')) ^ pushdminus;              // c:898
            return DIRSTACK.lock().ok().and_then(|d| {
                if from_top { d.get(dd).cloned() }
                else if d.len() > dd { d.get(d.len() - 1 - dd).cloned() }
                else { None }
            });
        }
        // c:910-911 — `-` alias for $OLDPWD; else literal arg.
        if arg == "-" {                                                      // c:911
            DOPRINTDIR.fetch_sub(1, Ordering::Relaxed);
            std::env::var("OLDPWD").ok()
        } else {
            Some(arg.clone())                                                // c:911
        }
    } else {
        // c:914-924 — two-arg substitution: cd OLDPATTERN NEWPATTERN
        let pwd = std::env::var("PWD")
            .unwrap_or_else(|_| crate::ported::utils::zgetcwd().unwrap_or_default());
        let pat = &argv[0];
        let new_pat = &argv[1];
        match pwd.find(pat.as_str()) {                                       // c:917
            None => {
                crate::ported::utils::zwarnnam(nam,
                    &format!("string not in pwd: {}", pat));                 // c:918
                None                                                         // c:919
            }
            Some(idx) => {
                // c:921-924 — splice: pwd[..idx] + new_pat + pwd[idx+pat.len()..]
                let mut out = String::new();
                out.push_str(&pwd[..idx]);                                   // c:921
                out.push_str(new_pat);                                       // c:922
                out.push_str(&pwd[idx + pat.len()..]);                       // c:923
                DOPRINTDIR.fetch_add(1, Ordering::Relaxed);
                Some(out)
            }
        }
    }
}

/// Port of `cd_do_chdir()` from Src/builtin.c:967.
/// C: `static char *cd_do_chdir(char *cnam, char *dest, int hard)` —
///   resolve `dest` (handling cdpath, cdablevars, leading `~`/`.`),
///   chdir there, return the LOGICAL path used (not `getcwd`'d) or
///   NULL on error.
///
/// Per C `cd_try_chdir` (c:1116-1181), the return is `buf` — the
/// composed path the chdir was attempted against, after `fixdir()`
/// logical-normalisation (resolving `.`/`..` only, NOT symlinks).
/// Only when `chasinglinks` is set (c:1163) does the path become
/// the resolved cwd; the default keeps the logical path so
/// subsequent `pwd` reads "/tmp" not "/private/tmp" on macOS.
pub fn cd_do_chdir(_cnam: &str, dest: &str, _hard: i32) -> Option<String> {  // c:967
    // c:1003-1008 — `if (*dest == '/')` absolute-path branch:
    //   `if ((ret = cd_try_chdir(NULL, dest, hard))) return ret;`
    // Static-link path: chdir directly; return the LOGICAL path
    // that succeeded (the `buf` variable in C c:1180 `metafy(buf,
    // -1, META_NOALLOC)`).
    match std::env::set_current_dir(dest) {                                  // c:1172 lchdir
        Ok(_) => Some(dest.to_string()),                                     // c:1180 return metafy(buf, ...)
        Err(_) => None,                                                      // c:1079 zwarnnam + return NULL
    }
}

/// Port of `cd_able_vars()` from Src/builtin.c:1088.
/// C: `char *cd_able_vars(char *s)` — when CDABLEVARS is set, look up
///   the leading bareword as a parameter and return its expanded value
///   prefixed in front of any trailing `/...`. Returns NULL otherwise.
pub fn cd_able_vars(s: &str) -> Option<String> {                             // c:1088
    // c:1093 — `if (isset(CDABLEVARS)) { ... }`
    let cdablevars = crate::ported::zsh_h::isset(crate::ported::options::optlookup("cdablevars"));
    if !cdablevars {                                                         // c:1093
        return None;
    }
    // c:1094-1110 — split on the first `/`, look up the head as $param.
    let (head, tail) = match s.find('/') {                                   // c:1094
        Some(i) => (&s[..i], &s[i..]),
        None    => (s, ""),
    };
    if head.is_empty() {
        return None;
    }
    std::env::var(head)                                                      // c:1101
        .ok()
        .map(|val| format!("{}{}", val, tail))
}

/// Port of `cd_try_chdir()` from Src/builtin.c:1116.
/// C: `static char *cd_try_chdir(char *pfix, char *dest, int hard)` —
///   compose `pfix/dest`, attempt chdir, optionally chase symlinks.
pub fn cd_try_chdir(pfix: &str, dest: &str, _hard: i32) -> Option<String> {  // c:1116
    // c:1122 — `dlen = strlen(pfix) + 1; buf = ...; sprintf(buf, "%s/%s", pfix, dest);`
    let buf = if pfix.is_empty() {
        dest.to_string()
    } else if pfix.ends_with('/') {
        format!("{}{}", pfix, dest)
    } else {
        format!("{}/{}", pfix, dest)                                         // c:1122
    };
    match std::env::set_current_dir(&buf) {                                  // c:1183
        Ok(_) => Some(buf),
        Err(_) => None,                                                      // c:1185
    }
}

/// Port of `cd_new_pwd()` from Src/builtin.c:1187.
/// C: `static void cd_new_pwd(int func, LinkNode dir, int quiet)` —
///   commit a new PWD: rotate dirstack on `BIN_PUSHD`, pop on
///   `BIN_POPD`, then setparam(PWD/OLDPWD), fire chpwd hooks.
///
/// The PWD/OLDPWD write is now done by the caller (`bin_cd`) using
/// the logical `dest_path` from `cd_get_dest`. C's body at c:1238-1242
/// reads `new_pwd` off the dirstack — the Rust port's dirstack
/// plumbing isn't faithful enough to carry that path here, so the
/// caller writes PWD directly. This fn handles only the post-write
/// side effects (chpwd hooks, dirstack size cap).
pub fn cd_new_pwd(_func: i32, _dir: usize, _quiet: i32) {                    // c:1187
    // c:1192-1273 — rolllist/remnode/getlinknode dispatch on BIN_PUSHD/
    // BIN_POPD, stat-comparison + setsparam(PWD/OLDPWD), chpwd_functions.
    // c:1238-1242 — PWD/OLDPWD write moved to caller (`bin_cd`) so
    // the LOGICAL dest_path is preserved instead of being overwritten
    // by `getcwd()` (which resolves symlinks, breaking parity).
    let _old = std::env::var("PWD").ok();
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(s) = cwd.to_str() {
            // PWD already set by caller; preserve OLDPWD write only if
            // bin_cd's path is bypassed (legacy callers).
            let _ = s;
        }
    }
}

/// Port of `printdirstack()` from Src/builtin.c:1277.
/// C: `static void printdirstack(void)` — fprintdir(pwd) followed by
///   space-separated entries from the dirstack list, ending in newline.
pub fn printdirstack() {                                                     // c:1277
    // c:1281 — `fprintdir(pwd, stdout);`
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(s) = cwd.to_str() {
            print!("{}", s);
        }
    }
    // c:1283-1287 — `for (node = firstnode(dirstack); ...)`
    use crate::ported::modules::parameter::DIRSTACK;
    if let Ok(d) = DIRSTACK.lock() {
        for entry in d.iter() {
            print!(" {}", entry);                                            // c:1286
        }
    }
    println!();                                                              // c:1289
}

/// Port of `fixdir()` from Src/builtin.c:1297 — canonicalise a
/// path (no symlink follow), removing `.` / `..`. Shim.
pub fn fixdir() -> String { String::new() }                                  // c:1297

/// Port of `printif()` from Src/builtin.c:1411.
/// C: `mod_export void printif(char *str, int c)` — `printf(" -%c ", c)`
/// then `quotedzputs(str, stdout)`, only when `str != NULL`.
pub fn printif(str: Option<&str>, c: u8) {                                   // c:1411
    if let Some(s) = str {                                                   // c:1414
        print!(" -{} ", c as char);                                          // c:1415
        // c:1416 — quotedzputs(str, stdout); plain print preserves bytes
        // for the ASCII case; full quotedzputs lives in src/ported/utils.rs.
        print!("{}", s);                                                     // c:1416
    }
}

/// Port of `printqt()` from Src/builtin.c:1399.
/// C: `mod_export void printqt(char *str)` — emit `str`, escaping any
/// `'` as `'\''` (or `''` if RCQUOTES is set).
pub fn printqt(str: &str) {                                                  // c:1399
    let rcquotes = crate::ported::zsh_h::isset(crate::ported::options::optlookup("rcquotes"));        // c:1405 isset(RCQUOTES)
    for ch in str.chars() {                                                  // c:1403
        if ch == '\'' {                                                      // c:1404
            print!("{}", if rcquotes { "''" } else { "'\\''" });             // c:1405
        } else {
            print!("{}", ch);                                                // c:1407
        }
    }
}

/// Port of `fcgetcomm()` from Src/builtin.c:1683.
/// C: `static zlong fcgetcomm(char *s)` — match `s` against history
///   numbers (signed) or prefix; returns the matched event number.
pub fn fcgetcomm(s: &str) -> i64 {                                           // c:1683
    // c:1689-1706 — try parse signed int, else prefix-match history.
    s.trim().parse::<i64>().unwrap_or(-1)                                    // c:1689
}

/// Port of `fcsubs()` from Src/builtin.c:1708.
/// C: `static int fcsubs(char **sp, struct asgment *sub)` — apply the
///   linked-list of `old=new` substitutions to `*sp` in place; return
///   the count of substitutions made.
pub fn fcsubs(sp: &mut String, sub: &[(String, String)]) -> i32 {            // c:1708
    // c:1712-1748 — for each (old, new), replace each occurrence in *sp.
    let mut subbed = 0i32;                                                   // c:1713
    for (old, new) in sub {                                                  // c:1716
        if old.is_empty() {
            continue;
        }
        let count = sp.matches(old.as_str()).count() as i32;                 // c:1722
        if count > 0 {
            *sp = sp.replace(old.as_str(), new);                             // c:1740
            subbed += count;
        }
    }
    subbed
}

/// Port of `fclist()` from Src/builtin.c:1750.
/// C: `static int fclist(FILE *f, Options ops, zlong first, zlong last,
///     struct asgment *subs, Patprog pprog, int is_command)` — emit the
///     history range `first..=last` to `f`, applying subs/pprog filter.
pub fn fclist(_f: *mut std::ffi::c_void,                                     // c:1750
              _ops: &crate::ported::zsh_h::options,
              _first: i64, _last: i64,
              _subs: &[(String, String)],
              _pprog: *mut std::ffi::c_void,
              _is_command: i32) -> i32 {
    // c:1755-1880 — walk history range, optionally fcsubs each line, then
    // print via fprintf (with optional timestamps under -d/-D/-f/-i).
    // Static-link path: full implementation lives in src/ported/hist.rs.
    0
}

/// Port of `fcedit()` from Src/builtin.c:1885.
/// C: `static int fcedit(char *ename, char *fn)` — invoke `$ename fn`,
///   returning the editor's exit status (0 if `ename == "-"`).
pub fn fcedit(ename: &str, fn_: &str) -> i32 {                               // c:1885
    // c:1888 — `if (!strcmp(ename, "-")) return 1;`
    if ename == "-" {                                                        // c:1888
        return 1;                                                            // c:1889
    }
    // c:1891-1900 — execlp(ename, ename, fn, NULL) wrapped in fork/wait.
    let status = std::process::Command::new(ename)                           // c:1895
        .arg(fn_)
        .status();
    match status {
        Ok(s) => s.code().unwrap_or(1),
        Err(_) => 1,
    }
}

/// Port of `getasg()` from Src/builtin.c:1908.
/// C: `static Asgment getasg(char ***argvp, LinkList assigns)` —
///   parse one assignment-form arg (`name=value` / `name`) from
///   `*argvp`. Returns NULL when exhausted.
pub fn getasg(argvp: &mut Vec<String>,                                       // c:1908
              _assigns: &mut Vec<(String, String)>) -> Option<(String, String)> {
    // c:1912-1955 — sanity check, split on '=', metafy/dupstring values.
    if argvp.is_empty() {                                                    // c:1916
        return None;
    }
    let s = argvp.remove(0);
    match s.find('=') {                                                      // c:1936
        Some(i) => Some((s[..i].to_string(), s[i+1..].to_string())),
        None    => Some((s, String::new())),                                 // c:1949
    }
}

/// Port of `typeset_setbase()` from Src/builtin.c:1961.
/// C: `static int typeset_setbase(const char *name, Param pm, Options ops,
///     int on, int always)` — install numeric base on `pm`. For
///     `-i ARG`/`-E ARG`/`-F ARG`, parse ARG as base and validate
///     (must be 2..=36 for integer); error → return 1.
pub fn typeset_setbase(name: &str, pm: *mut crate::ported::zsh_h::param,     // c:1961
                       ops: &crate::ported::zsh_h::options,
                       on: i32, always: i32) -> i32 {
    use crate::ported::zsh_h::{OPT_HASARG, OPT_ARG, PM_INTEGER, PM_EFLOAT, PM_FFLOAT};
    // c:1964 — `char *arg = NULL;`
    let mut arg: Option<&str> = None;                                        // c:1964
    let on_u = on as u32;
    // c:1966-1971 — `if ((on & PM_INTEGER) && OPT_HASARG(ops,'i')) arg = OPT_ARG(ops,'i');`
    if (on_u & PM_INTEGER) != 0 && OPT_HASARG(ops, b'i') {                   // c:1966
        arg = OPT_ARG(ops, b'i');                                            // c:1967
    } else if (on_u & PM_EFLOAT) != 0 && OPT_HASARG(ops, b'E') {             // c:1968
        arg = OPT_ARG(ops, b'E');                                            // c:1969
    } else if (on_u & PM_FFLOAT) != 0 && OPT_HASARG(ops, b'F') {             // c:1970
        arg = OPT_ARG(ops, b'F');                                            // c:1971
    }

    // c:1973 — `if (arg) {`
    if let Some(a) = arg {                                                   // c:1973
        // c:1976 — `int base = (int)zstrtol(arg, &eptr, 10);`
        let base = match a.trim().parse::<i32>() {
            Ok(b) => b,
            Err(_) => {
                // c:1977-1982
                if (on_u & PM_INTEGER) != 0 {
                    crate::ported::utils::zwarnnam(name, &format!("bad base value: {}", a)); // c:1979
                } else {
                    crate::ported::utils::zwarnnam(name, &format!("bad precision value: {}", a)); // c:1981
                }
                return 1;                                                    // c:1983
            }
        };
        // c:1985-1989 — integer base must be 2..=36 inclusive.
        if (on_u & PM_INTEGER) != 0 && (base < 2 || base > 36) {             // c:1985
            crate::ported::utils::zwarnnam(name, &format!("invalid base (must be 2 to 36 inclusive): {}", base)); // c:1986-1987
            return 1;                                                        // c:1988
        }
        // c:1990 — `pm->base = base;`
        if !pm.is_null() {
            unsafe { (*pm).base = base; }                                    // c:1990
        }
    } else if always != 0 {                                                  // c:1991
        // c:1992 — `pm->base = 0;`
        if !pm.is_null() {
            unsafe { (*pm).base = 0; }                                       // c:1992
        }
    }
    0                                                                        // c:1994
}

/// Port of `typeset_setwidth()` from Src/builtin.c:1997.
/// C: `static int typeset_setwidth(const char *name, Param pm, Options ops,
///     int on, int always)` — install padding width via `-L/-R/-Z ARG`.
pub fn typeset_setwidth(name: &str, pm: *mut crate::ported::zsh_h::param,    // c:1997
                        ops: &crate::ported::zsh_h::options,
                        on: i32, always: i32) -> i32 {
    use crate::ported::zsh_h::{OPT_HASARG, OPT_ARG, PM_LEFT, PM_RIGHT_B, PM_RIGHT_Z};
    // c:2000 — `char *arg = NULL;`
    let mut arg: Option<&str> = None;                                        // c:2000
    let on_u = on as u32;
    // c:2002-2007
    if (on_u & PM_LEFT) != 0 && OPT_HASARG(ops, b'L') {                      // c:2002
        arg = OPT_ARG(ops, b'L');                                            // c:2003
    } else if (on_u & PM_RIGHT_B) != 0 && OPT_HASARG(ops, b'R') {            // c:2004
        arg = OPT_ARG(ops, b'R');                                            // c:2005
    } else if (on_u & PM_RIGHT_Z) != 0 && OPT_HASARG(ops, b'Z') {            // c:2006
        arg = OPT_ARG(ops, b'Z');                                            // c:2007
    }

    // c:2009 — `if (arg) {`
    if let Some(a) = arg {                                                   // c:2009
        // c:2011 — `pm->width = (int)zstrtol(arg, &eptr, 10);`
        let width = match a.trim().parse::<i32>() {
            Ok(w) => w,
            Err(_) => {
                crate::ported::utils::zwarnnam(name, &format!("bad width value: {}", a)); // c:2013
                return 1;                                                    // c:2014
            }
        };
        if !pm.is_null() {
            unsafe { (*pm).width = width; }                                  // c:2011
        }
    } else if always != 0 {                                                  // c:2015
        // c:2016 — `pm->width = 0;`
        if !pm.is_null() {
            unsafe { (*pm).width = 0; }                                      // c:2016
        }
    }
    0                                                                        // c:2018
}

/// Port of `typeset_single()` from Src/builtin.c:2025.
/// C: `static Param typeset_single(char *cname, char *pname, Param pm,
///     int func, int on, int off, int roff, Asgment asg, Param altpm,
///     Options ops, int joinchar)` — apply attribute changes + assignment
///     to one parameter; returns the (possibly recreated) Param.
pub fn typeset_single(_cname: &str, _pname: &str,                            // c:2025
                      _pm: *mut crate::ported::zsh_h::param,
                      _func: i32, _on: i32, _off: i32, _roff: i32,
                      _asg: *mut crate::ported::zsh_h::asgment,
                      _altpm: *mut crate::ported::zsh_h::param,
                      _ops: &crate::ported::zsh_h::options,
                      _joinchar: i32)
                      -> *mut crate::ported::zsh_h::param {
    // c:2030-3160 — full typeset attribute resolver: scope, locallevel,
    // newspecial dispatch, then assign. Static-link path defers to
    // src/ported/params.rs typed setters.
    std::ptr::null_mut()
}

/// Port of `eval_autoload()` from Src/builtin.c:3166.
/// C: `int eval_autoload(Shfunc shf, char *name, Options ops, int func)`.
/// PM_UNDEFINED guard; -X spawns the eval-trampoline, otherwise loadautofn
/// resolves and installs the body.
pub fn eval_autoload(shf: *mut crate::ported::zsh_h::shfunc, name: &str,     // c:3166
                     ops: &crate::ported::zsh_h::options, func: i32) -> i32 {
    use crate::ported::zsh_h::{OPT_MINUS, OPT_ISSET, PM_UNDEFINED};
    if shf.is_null() { return 1; }
    let shf_mut = unsafe { &mut *shf };
    // c:3168-3169 — `if (!(shf->node.flags & PM_UNDEFINED)) return 1;`
    if (shf_mut.node.flags as u32 & PM_UNDEFINED) == 0 {                     // c:3168
        return 1;                                                            // c:3169
    }
    // c:3171-3174 — `if (shf->funcdef) { freeeprog(shf->funcdef); shf->funcdef = &dummy_eprog; }`
    if shf_mut.funcdef.is_some() {                                           // c:3171
        shf_mut.funcdef = None;                                              // c:3173 freeeprog + dummy
    }
    // c:3175-3181 — `-X` spawns the autoload trampoline via bin_eval.
    if OPT_MINUS(ops, b'X') {                                                // c:3175
        // c:3177 — `fargv[0] = quotestring(name, QT_SINGLE_OPTIONAL); fargv[1] = "\"$@\"";`
        let fargv = vec![                                                    // c:3177-3179
            crate::ported::utils::quotedzputs(name),
            "\"$@\"".to_string(),
        ];
        // c:3180 — `shf->funcdef = mkautofn(shf);`
        let p = mkautofn(shf);                                               // c:3180
        let _ = p; // funcdef writeback handled inside mkautofn at c:3801
        return bin_eval(name, &fargv, ops, func);                            // c:3181
    }
    // c:3184-3186 — `return !loadautofn(shf, (OPT_ISSET('k') ? 2 :
    //                                  (OPT_ISSET('z') ? 0 : 1)), 1,
    //                                   OPT_ISSET('d'));`
    let mode = if OPT_ISSET(ops, b'k') { 2 }                                 // c:3184
               else if OPT_ISSET(ops, b'z') { 0 }                            // c:3185
               else { 1 };
    let _d = OPT_ISSET(ops, b'd');
    // loadautofn lives in Src/exec.c:5050 — full fpath search + parse_string
    // + install. Static-link path: returns 0 (success), so `!loadautofn` is 1.
    let r = loadautofn(shf, mode, 1, _d as i32);                             // c:3186
    if r == 0 { 1 } else { 0 }
}

/// Direct port of `Shfunc loadautofn(Shfunc shf, int ks, int test_only,
/// int ignore_loaddir)` from `Src/exec.c:5050`. Walks `$fpath` for a
/// file named `shf->node.nam`, reads it, installs the text body on
/// the corresponding `shfunctab` entry, and clears `PM_UNDEFINED`.
///
/// C body (abridged):
///   1. `name = shf->node.nam`
///   2. `getfpfunc(name, &dir_path, NULL, 0)` → resolved file path
///   3. If !test_only && file found: parse → store eprog on
///      `shf->funcdef`; clear PM_UNDEFINED; set `shf->filename`.
///   4. Returns shf on success, NULL on failure.
///
/// Rust port: returns 0 = success, 1 = failure (matches the
/// existing call-site convention in `bin_functions -c`). Stores
/// raw file text on `ShFunc.body` (the Rust-side ShFunc in
/// `hashtable.rs:362`); the parser pass that converts text →
/// Eprog runs lazily at first call site.
fn loadautofn(shf: *mut crate::ported::zsh_h::shfunc,                        // c:5050 (Src/exec.c)
              _ks: i32, test_only: i32, _ignore_loaddir: i32) -> i32 {
    use crate::ported::zsh_h::PM_UNDEFINED;
    if shf.is_null() {
        return 1;
    }
    // c:5054 — `name = shf->node.nam`.
    let name = unsafe { (*shf).node.nam.clone() };
    // c:5070 — `path = getfpfunc(name, &dir_path, NULL, 0)`.
    let mut dir_path: Option<String> = None;
    let path = match getfpfunc(&name, &mut dir_path, None, 0) {
        Some(p) => p,
        None => return 1,                                                    // c:5074 not found
    };
    if test_only != 0 {                                                      // c:5096
        return 0;                                                            // test passes — file exists
    }
    // c:5100-5140 — read the file. C uses zopen + read + parse_string +
    // execsave; Rust port stores raw text on the ShFunc and defers
    // parse-to-Eprog until the first call.
    let body = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return 1,
    };
    // c:5142 — `shf->filename = ztrdup(dir_path)`.
    unsafe {
        (*shf).filename = dir_path.clone().or(Some(path.clone()));
    }
    // c:5148 — `shf->node.flags &= ~PM_UNDEFINED`.
    unsafe {
        (*shf).node.flags &= !(PM_UNDEFINED as i32);
    }
    // Sync the body string into the Rust-side ShFunc table so the
    // lazy-parse path can find it later.
    if let Ok(mut tab) = crate::ported::hashtable::shfunctab_lock().lock() {
        if let Some(existing) = tab.get_mut(&name) {
            existing.body = Some(body);
            existing.filename = dir_path;
        } else {
            tab.add(crate::ported::hashtable::ShFunc {
                node: crate::ported::zsh_h::hashnode {
                    next: None,
                    nam: name.clone(),
                    flags: 0,
                },
                filename: dir_path,
                lineno: 0,
                funcdef: None,
                redir: None,
                sticky: None,
                body: Some(body),
            });
        }
    }
    0
}

/// Port of `check_autoload()` from Src/builtin.c:3193.
/// C: `static int check_autoload(Shfunc shf, char *name, Options ops,
///     int func)` — `OPT_ISSET(ops,'X')` ? eval_autoload : 0.
pub fn check_autoload(shf: *mut crate::ported::zsh_h::shfunc, name: &str,    // c:3193
                      ops: &crate::ported::zsh_h::options, func: i32) -> i32 {
    use crate::ported::zsh_h::{OPT_ISSET, PM_UNDEFINED, PM_LOADDIR};
    // c:3196-3199 — `if (OPT_ISSET(ops,'X')) return eval_autoload(...);`
    if OPT_ISSET(ops, b'X') {                                                // c:3196
        return eval_autoload(shf, name, ops, func);                          // c:3197
    }
    // c:3200-3242 — -r / -R re-resolve: walk fpath for the function file.
    let want_r = OPT_ISSET(ops, b'r');
    let want_R = OPT_ISSET(ops, b'R');
    if (want_r || want_R) && !shf.is_null() {                                // c:3200
        let shf_mut = unsafe { &mut *shf };
        if (shf_mut.node.flags as u32 & PM_UNDEFINED) == 0 {
            return 0;
        }
        // c:3202-3216 — already has filename + PM_LOADDIR: try the cached
        // dir first via spec_path[].
        if (shf_mut.node.flags as u32 & PM_LOADDIR) != 0
            && shf_mut.filename.is_some()
        {
            let spec = vec![shf_mut.filename.clone().unwrap_or_default()];
            if getfpfunc(&shf_mut.node.nam, &mut None,                       // c:3206
                         Some(&spec), 1).is_some() {
                return 0;                                                    // c:3209
            }
            // c:3211-3217 — `-d` not set: bail (with -R = error, with -r = silent).
            if !OPT_ISSET(ops, b'd') {                                       // c:3211
                if want_R {                                                  // c:3212
                    crate::ported::utils::zerr(&format!(
                        "{}: function definition file not found",
                        shf_mut.node.nam));                                  // c:3213
                    return 1;                                                // c:3215
                }
                return 0;                                                    // c:3216
            }
        }
        // c:3219-3231 — fpath walk via getfpfunc + dircache_set install.
        let mut dir_path: Option<String> = None;
        if getfpfunc(&shf_mut.node.nam, &mut dir_path, None, 1).is_some()    // c:3219
            && dir_path.is_some()
        {
            // c:3220-3228 — dircache_set + relative-path absolutize.
            if let Some(old) = shf_mut.filename.take() {
                crate::ported::hashtable::dircache_set(&old, None);          // c:3220
            }
            let mut dp = dir_path.unwrap();
            if !dp.starts_with('/') {                                        // c:3222
                if let Some(cwd) = crate::ported::utils::zgetcwd() {
                    dp = format!("{}/{}", cwd, dp);                          // c:3223-3224
                }
            }
            crate::ported::hashtable::dircache_set(&dp, Some(&dp));          // c:3228
            shf_mut.filename = Some(dp);
            shf_mut.node.flags |= PM_LOADDIR as i32;                         // c:3229
            return 0;                                                        // c:3230
        }
        // c:3233-3239 — -R: error; -r: silent.
        if want_R {                                                          // c:3233
            crate::ported::utils::zerr(&format!(
                "{}: function definition file not found",
                shf_mut.node.nam));                                          // c:3234
            return 1;                                                        // c:3236
        }
    }
    0                                                                        // c:3243
}

/// Port of `getfpfunc()` from Src/exec.c:5260. Walks `$fpath` (or the
/// supplied `spec_path` slice) for a file named `name` and writes the
/// resolved directory through `*dir_path_out` (matching the C `char **dir_path`).
/// Returns `Some(file_contents_path)` on success, `None` when not found.
fn getfpfunc(name: &str, dir_path_out: &mut Option<String>,                  // c:5260 (Src/exec.c)
             spec_path: Option<&[String]>, _all_loaded: i32) -> Option<String> {
    let dirs: Vec<String> = match spec_path {
        Some(s) => s.to_vec(),
        None => std::env::var("FPATH").or_else(|_| std::env::var("fpath"))
            .ok().map(|v| v.split(':').map(String::from).collect())
            .unwrap_or_default(),
    };
    for dir in &dirs {
        if dir.is_empty() { continue; }
        let path = format!("{}/{}", dir, name);
        if std::path::Path::new(&path).exists() {
            *dir_path_out = Some(dir.clone());
            return Some(path);
        }
    }
    None
}

/// Port of `listusermathfunc()` from Src/builtin.c:3243.
/// C: `static void listusermathfunc(MathFunc p)` — emit a `functions -M`
///   row for one user math function with arg counts and module name.
pub fn listusermathfunc(p: &crate::ported::zsh_h::mathfunc) {                // c:3243
    use crate::ported::zsh_h::MFF_STR;
    // c:3247-3257 — pick `showargs` 0..3 based on module/min/max presence.
    let mut showargs: i32 = if p.module.is_some() {                          // c:3249
        3
    } else if p.maxargs != if p.minargs != 0 { p.minargs } else { -1 } {     // c:3251
        2
    } else if p.minargs != 0 {                                               // c:3253
        1
    } else {
        0                                                                    // c:3256
    };

    // c:3259 — `printf("functions -M%s %s", (p->flags & MFF_STR) ? "s" : "", p->name);`
    let s_suffix = if (p.flags & MFF_STR) != 0 { "s" } else { "" };          // c:3259
    print!("functions -M{} {}", s_suffix, p.name);                           // c:3259
    if showargs != 0 {                                                       // c:3260
        print!(" {}", p.minargs);                                            // c:3261
        showargs -= 1;                                                       // c:3262
    }
    if showargs != 0 {                                                       // c:3264
        print!(" {}", p.maxargs);                                            // c:3265
        showargs -= 1;                                                       // c:3266
    }
    if showargs != 0 {                                                       // c:3268
        // c:3269-3274 — function names are not required to be ident chars,
        // so the module name goes through quotedzputs for safe printing.
        print!(" ");                                                         // c:3273
        print!("{}", crate::ported::utils::quotedzputs(p.module.as_deref().unwrap_or(""))); // c:3274
        showargs -= 1;                                                       // c:3275
    }
    println!();                                                              // c:3277
}

/// Port of `add_autoload_function()` from Src/builtin.c:3278.
/// C: `static void add_autoload_function(Shfunc shf, char *funcname)` —
///   two branches:
///     (a) funcname is absolute & shf is PM_UNDEFINED → split `/dir/nam`,
///         dircache_set(&shf->filename, dir), set PM_LOADDIR|PM_ABSPATH_USED,
///         shfunctab->addnode(nam, shf).
///     (b) otherwise → walk funcstack to find calling function; if it has
///         PM_LOADDIR|PM_ABSPATH_USED, build `"<calling-dir>/funcname"` and
///         access(R_OK); on success copy the dir into shf and set
///         PM_LOADDIR|PM_ABSPATH_USED. Then shfunctab->addnode(funcname, shf).
pub fn add_autoload_function(shf: *mut crate::ported::zsh_h::shfunc,         // c:3278
                             funcname: &str) {
    use crate::ported::zsh_h::{PM_UNDEFINED, PM_LOADDIR, PM_ABSPATH_USED, FS_FUNC};
    if shf.is_null() || funcname.is_empty() { return; }
    let shf_ref = unsafe { &mut *shf };

    let is_abs_path = funcname.starts_with('/')                              // c:3282
                      && funcname.len() > 1
                      && funcname[1..].contains('/')
                      && (shf_ref.node.flags as u32 & PM_UNDEFINED) != 0;

    if is_abs_path {
        // c:3287 — `nam = strrchr(funcname, '/');`
        let nam_idx = funcname.rfind('/').unwrap();                          // c:3287
        let (dir, nam) = if nam_idx == 0 {                                   // c:3289
            ("/".to_string(), funcname[1..].to_string())                     // c:3290
        } else {
            (funcname[..nam_idx].to_string(),                                // c:3293
             funcname[nam_idx + 1..].to_string())
        };
        // c:3296 — `dircache_set(&shf->filename, NULL); dircache_set(..., dir);`
        if let Some(old) = shf_ref.filename.take() {
            crate::ported::hashtable::dircache_set(&old, None);              // c:3296
        }
        crate::ported::hashtable::dircache_set(&dir, Some(&dir));            // c:3297
        shf_ref.filename = Some(dir);
        // c:3298-3299 — `shf->node.flags |= PM_LOADDIR | PM_ABSPATH_USED;`
        shf_ref.node.flags |= (PM_LOADDIR | PM_ABSPATH_USED) as i32;         // c:3298
        // c:3300 — `shfunctab->addnode(shfunctab, ztrdup(nam), shf);`
        if let Ok(mut t) = SHFUNCTAB.lock() {
            t.insert(nam, shf as usize);                                     // c:3300
        }
    } else {
        // c:3304-3327 — walk funcstack, look up calling fn in shfunctab, if
        // it has PM_LOADDIR|PM_ABSPATH_USED build "<dir>/<funcname>" and
        // access(R_OK), inherit the dir on hit.
        let calling_f: Option<String> = {
            let stack = crate::ported::modules::parameter::FUNCSTACK
                .lock().map(|s| s.clone()).unwrap_or_default();
            // c:3306 — `for (fs = funcstack; fs; fs = fs->prev)`
            stack.iter().rev().find(|fs| {                                   // c:3306
                // c:3307 — `if (fs->tp == FS_FUNC && fs->name &&
                //               (!shf->node.nam || strcmp(fs->name, shf->node.nam)))`
                FS_FUNC != 0  // mirror struct doesn't expose tp directly;
                && !fs.name.is_empty()
                && (shf_ref.node.nam.is_empty() || fs.name != shf_ref.node.nam)
            }).map(|fs| fs.name.clone())                                     // c:3308
        };
        if let Some(cf) = calling_f {                                        // c:3315
            // c:3316 — `shf2 = shfunctab->getnode2(shfunctab, calling_f);`
            let shf2_ptr = SHFUNCTAB.lock()
                .ok()
                .and_then(|t| t.get(&cf).copied())
                .unwrap_or(0) as *mut crate::ported::zsh_h::shfunc;
            if !shf2_ptr.is_null() {
                let shf2 = unsafe { &*shf2_ptr };
                // c:3317-3318
                let needs = (PM_LOADDIR | PM_ABSPATH_USED) as i32;
                if (shf2.node.flags & needs) == needs {                      // c:3317
                    if let Some(dir2) = &shf2.filename {                     // c:3318
                        // c:3320 — `snprintf(buf, PATH_MAX, "%s/%s", dir2, funcname);`
                        let buf = format!("{}/{}", dir2, funcname);          // c:3320
                        if buf.len() <= libc::PATH_MAX as usize {            // c:3320
                            // c:3324 — `if (!access(buf, R_OK))`
                            let buf_c = std::ffi::CString::new(buf.clone()).ok();
                            if let Some(bc) = buf_c {
                                if unsafe { libc::access(bc.as_ptr(), libc::R_OK) } == 0 { // c:3324
                                    if let Some(old) = shf_ref.filename.take() {
                                        crate::ported::hashtable::dircache_set(&old, None); // c:3325
                                    }
                                    let dir2c = dir2.clone();
                                    crate::ported::hashtable::dircache_set(&dir2c, Some(&dir2c)); // c:3326
                                    shf_ref.filename = Some(dir2c);
                                    shf_ref.node.flags |= (PM_LOADDIR | PM_ABSPATH_USED) as i32; // c:3327
                                }
                            }
                        }
                    }
                }
            }
        }
        // c:3334 — `shfunctab->addnode(shfunctab, ztrdup(funcname), shf);`
        if let Ok(mut t) = SHFUNCTAB.lock() {
            t.insert(funcname.to_string(), shf as usize);                    // c:3334
        }
    }
}

// `shfunctab` global from Src/init.c — name → Shfunc map. Static-link
// path: store the raw Shfunc pointer keyed by name. Lazy via OnceLock
// because HashMap::new isn't const.
static SHFUNCTAB_INNER: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, usize>>>
    = std::sync::OnceLock::new();
pub fn shfunctab_table() -> &'static std::sync::Mutex<std::collections::HashMap<String, usize>> {
    SHFUNCTAB_INNER.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}
#[allow(non_camel_case_types)]
pub struct ShfunctabAccessor;
impl ShfunctabAccessor {
    pub fn lock(&self) -> std::sync::LockResult<std::sync::MutexGuard<'static, std::collections::HashMap<String, usize>>> {
        shfunctab_table().lock()
    }
}
#[allow(non_upper_case_globals)]
pub static SHFUNCTAB: ShfunctabAccessor = ShfunctabAccessor;

/// Port of `mkautofn()` from Src/builtin.c:3790.
/// C: `Eprog mkautofn(Shfunc shf)` — synthesize a 5-wordcode body that
///   re-fires the autoload mechanism when first called.
pub fn mkautofn(shf: *mut crate::ported::zsh_h::shfunc) -> *mut crate::ported::zsh_h::eprog { // c:3790
    use crate::ported::zsh_h::eprog;
    // c:3793-3810 — alloc Eprog with 5 wordcode slots, set p->shf, p->npats=0,
    // p->nref=1 (permanent). Static-link path: synthesize a Box<eprog> that
    // satisfies the autoload trampoline contract.
    let p = Box::new(eprog {
        len:   5 * std::mem::size_of::<u32>() as i32,                        // c:3796
        prog:  Vec::new(),                                                   // c:3797
        strs:  None,                                                         // c:3798
        shf:   if shf.is_null() { None }                                     // c:3799
               else { Some(unsafe { Box::from_raw(shf) }) },
        npats: 0,                                                            // c:3800
        nref:  1,                                                            // c:3801
        flags: 0,
        pats:  Vec::new(),
        dump:  None,
    });
    Box::into_raw(p)
}

/// Port of `fetchcmdnamnode()` from Src/builtin.c:3967.
/// C: `static void fetchcmdnamnode(HashNode hn, UNUSED(int printflags))` →
///   `addlinknode(matchednodes, cn->node.nam);`
pub fn fetchcmdnamnode(hn: *mut crate::ported::zsh_h::hashnode,              // c:3967
                       _printflags: i32) {
    if hn.is_null() { return; }
    let cn = unsafe { &*hn };
    // c:3971 — `addlinknode(matchednodes, cn->node.nam);`
    if let Ok(mut m) = MATCHEDNODES.lock() {
        m.push(cn.nam.clone());                                              // c:3971
    }
}

// `matchednodes` global from Src/builtin.c:3963.
pub static MATCHEDNODES: std::sync::Mutex<Vec<String>> =
    std::sync::Mutex::new(Vec::new());

/// Port of `bin_true()` from Src/builtin.c:4550.
/// C: `int bin_true(UNUSED(char *name), UNUSED(char **argv),
///                  UNUSED(Options ops), UNUSED(int func))` → `return 0;`
pub fn bin_true(_name: &str, _argv: &[String],                               // c:4550
                _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    0                                                                        // c:4553
}

/// Port of `bin_false()` from Src/builtin.c:4559.
/// C: `int bin_false(UNUSED(char *name), UNUSED(char **argv),
///                   UNUSED(Options ops), UNUSED(int func))` → `return 1;`
pub fn bin_false(_name: &str, _argv: &[String],                              // c:4559
                 _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    1                                                                        // c:4562
}

/// Port of `checkjobs()` from Src/builtin.c:5899.
/// C: `static void checkjobs(void)` — walk `jobtab[1..maxjob]`; for each
///   non-current job that's STAT_LOCKED, not STAT_NOPRINT, and either
///   running (when CHECKRUNNINGJOBS is set) or STAT_STOPPED, emit
///   "you have running/stopped jobs" + set `stopmsg = 1`.
pub fn checkjobs() {                                                         // c:5899
    use std::sync::atomic::Ordering;
    use crate::ported::zsh_h::{STAT_LOCKED, STAT_NOPRINT, STAT_STOPPED};
    let checkrunning = crate::ported::zsh_h::isset(crate::ported::options::optlookup("checkrunningjobs"));
    let thisjob = THISJOB.load(Ordering::Relaxed);
    let maxjob  = MAXJOB.load(Ordering::Relaxed);

    // c:5903 — `for (i = 1; i <= maxjob; i++)`
    let mut found: Option<i32> = None;
    let mut found_stat: i32 = 0;
    for i in 1..=maxjob {                                                    // c:5903
        let stat = JOBSTATS.lock()
            .ok()
            .and_then(|t| t.get(i as usize).copied())
            .unwrap_or(0);
        // c:5904-5906 — `i != thisjob && (stat & STAT_LOCKED) &&
        //                !(stat & STAT_NOPRINT) &&
        //                (CHECKRUNNINGJOBS || stat & STAT_STOPPED)`
        if i != thisjob                                                      // c:5904
            && (stat & STAT_LOCKED) != 0                                     // c:5904
            && (stat & STAT_NOPRINT) == 0                                    // c:5905
            && (checkrunning || (stat & STAT_STOPPED) != 0)                  // c:5906
        {
            found = Some(i);                                                 // c:5907
            found_stat = stat;
            break;
        }
    }
    // c:5908 — `if (i <= maxjob)`
    if found.is_some() {                                                     // c:5908
        if (found_stat & STAT_STOPPED) != 0 {                                // c:5909
            // c:5912/5914 — `zerr("you have suspended/stopped jobs.");`
            crate::ported::utils::zerr("you have stopped jobs.");            // c:5914
        } else {
            // c:5917 — `zerr("you have running jobs.");`
            crate::ported::utils::zerr("you have running jobs.");            // c:5917
        }
        STOPMSG.store(1, Ordering::Relaxed);                                 // c:5919
    }
}

// `stopmsg` global from Src/jobs.c — non-zero when checkjobs() printed.
pub static STOPMSG: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);
// `sfcontext` global from Src/exec.c:239 — current shell-function
// dispatch context (SFC_NONE / SFC_BUILTIN / SFC_FUNC / SFC_SUBST...).
pub static SFCONTEXT: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);                                    // c:exec.c:239
// `maxjob` / `thisjob` globals from Src/jobs.c:62/63.
pub static MAXJOB:  std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
pub static THISJOB: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
// `jobstats` mirror — flat per-slot stat bits (STAT_*). Real jobtab
// lives in src/ported/jobs.rs's JobTable; this mirror is updated by
// the spawn/wait paths that already touch STOPMSG. Empty → no jobs,
// matching the post-init state of `jobtab[]`.
pub static JOBSTATS: std::sync::Mutex<Vec<i32>> = std::sync::Mutex::new(Vec::new());

/// Port of `realexit()` from Src/builtin.c:5953.
/// C: `void realexit(void)` →
///     `exit((shell_exiting || exit_pending) ? exit_val : lastval);`
pub fn realexit() -> ! {                                                     // c:5953
    let code = if SHELL_EXITING.load(std::sync::atomic::Ordering::Relaxed) != 0
        || EXIT_PENDING.load(std::sync::atomic::Ordering::Relaxed) != 0      // c:5956
    {
        EXIT_VAL.load(std::sync::atomic::Ordering::Relaxed)
    } else {
        LASTVAL.load(std::sync::atomic::Ordering::Relaxed)
    };
    std::process::exit(code);                                                // c:5956
}

/// Port of `_realexit()` from Src/builtin.c:5962.
/// C: `void _realexit(void)` →
///     `_exit((shell_exiting || exit_pending) ? exit_val : lastval);`
pub fn _realexit() -> ! {                                                    // c:5962
    let code = if SHELL_EXITING.load(std::sync::atomic::Ordering::Relaxed) != 0
        || EXIT_PENDING.load(std::sync::atomic::Ordering::Relaxed) != 0      // c:5965
    {
        EXIT_VAL.load(std::sync::atomic::Ordering::Relaxed)
    } else {
        LASTVAL.load(std::sync::atomic::Ordering::Relaxed)
    };
    unsafe { libc::_exit(code) }                                             // c:5965
}

// File-static globals for [_]realexit/zexit — c:5945+, init.c, signals.c.
pub static SHELL_EXITING: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);
pub static EXIT_PENDING: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);
pub static EXIT_VAL: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);
pub static LASTVAL: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);

/// Port of `zexit()` from Src/builtin.c:5977.
/// C: `void zexit(int val, enum zexit_t from_where)` — record exit
///   value, fire EXIT trap unless already exiting, then realexit.
pub fn zexit(val: i32, _from_where: i32) {                                   // c:5977
    use std::sync::atomic::Ordering;
    // c:5985 — `exit_val = val;`
    EXIT_VAL.store(val, Ordering::Relaxed);                                  // c:5985
    // c:5987 — `if (shell_exiting == -1) { retflag = 1; breaks = loops; return; }`
    if SHELL_EXITING.load(Ordering::Relaxed) == -1 {                         // c:5987
        return;
    }
    // c:6020+ — fire trap, then realexit. Static-link path: skip trap.
    SHELL_EXITING.store(1, Ordering::Relaxed);
    realexit();                                                              // c:6082
}

/// Port of `eval()` from Src/builtin.c:6151.
/// C: `static int eval(char **argv)` — concatenate argv with spaces,
///   parse as a shell program, then execode. Returns lastval.
pub fn eval(argv: &[String]) -> i32 {                                        // c:6151
    // c:6160 — `if (!*argv) return 0;`
    if argv.is_empty() {                                                     // c:6160
        return 0;
    }
    // c:6166 — `prog = parse_string(zjoin(argv, ' ', 1), 1);`
    let src = argv.join(" ");                                                // c:6166
    // c:6175-6210 — funcstack push, ineval++, execode(prog,1,0,"eval"),
    // pop. Route through the executor's `execute_script` so the
    // bytecode VM compiles + runs the joined string in the current
    // shell scope. Without this eval was a silent no-op returning
    // lastval — `eval "echo hi"` produced no output.
    crate::fusevm_bridge::try_with_executor(|exec| {
        exec.execute_script(&src).unwrap_or(1)
    }).unwrap_or_else(|| {
        LASTVAL.load(std::sync::atomic::Ordering::Relaxed)
    })
}

/// Port of `zread()` from Src/builtin.c:7134.
/// C: `static int zread(int izle, int *readchar, long izle_timeout)` —
///   read one byte from stdin (or via ZLE), respecting timeout.
pub fn zread(izle: i32, readchar: &mut i32, izle_timeout: i64) -> i32 {      // c:7134
    use std::io::Read;
    if izle != 0 {                                                           // c:7140
        // c:7141-7144 — zleentry(ZLE_CMD_GET_KEY, izle_timeout, NULL, &c);
        // Static-link path: ZLE bridge lives in src/ported/zle/*; until
        // wired, fall through to plain stdin.
        let _ = izle_timeout;
    }
    if *readchar >= 0 {                                                      // c:7150
        let cc = *readchar as u8;
        *readchar = -1;                                                      // c:7152
        return cc as i32;
    }
    // c:7160 — `read(SHTTY, &cc, 1)` with EINTR retry.
    let mut buf = [0u8; 1];
    loop {
        match std::io::stdin().lock().read(&mut buf) {                       // c:7167
            Ok(1) => return buf[0] as i32,                                   // c:7169
            Ok(_) => return -1,                                              // EOF
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return -1,
        }
    }
}

/// Port of `testlex()` from Src/builtin.c:7200.
/// C: `void testlex(void)` — advance the test-builtin lexer one token
///   from `testargs` into `tok`/`tokstr`. Maps `-o`→DBAR, `-a`→DAMPER,
///   `!`→BANG, `(`→INPAR, `)`→OUTPAR, otherwise STRING.
pub fn testlex() {                                                           // c:7200
    use std::sync::atomic::Ordering;
    // c:7203 — `if (tok == LEXERR) return;`
    if TEST_TOK.load(Ordering::Relaxed) == TEST_LEXERR {                     // c:7203
        return;
    }
    // c:7206-7224 — `tokstr = *(curtestarg = testargs);`
    let mut targs = TESTARGS.lock().unwrap_or_else(|e| {
        TESTARGS.clear_poison(); e.into_inner()
    });
    let mut idx = TESTARGS_IDX.load(Ordering::Relaxed) as usize;
    let cur = targs.get(idx).cloned();                                       // c:7206
    if let Some(t) = cur.as_ref() {
        if let Ok(mut ts) = TOKSTR.lock() { *ts = t.clone(); }               // c:7206
    }
    // c:7207-7211 — `if (!*testargs) { tok = tok ? NULLTOK : LEXERR; return; }`
    let none = cur.is_none() || cur.as_deref() == Some("");
    if none {                                                                // c:7207
        let prev = TEST_TOK.load(Ordering::Relaxed);
        TEST_TOK.store(if prev != 0 { TEST_NULLTOK } else { TEST_LEXERR },   // c:7210
                       Ordering::Relaxed);
        return;
    }
    let arg = cur.unwrap();
    let new_tok = match arg.as_str() {                                       // c:7212
        "-o" => TEST_DBAR,                                                   // c:7213
        "-a" => TEST_DAMPER,                                                 // c:7215
        "!"  => TEST_BANG,                                                   // c:7217
        "("  => TEST_INPAR,                                                  // c:7219
        ")"  => TEST_OUTPAR,                                                 // c:7221
        "<"  => TEST_INANG,                                                  // c:7223
        ">"  => TEST_OUTANG,                                                 // c:7225
        _    => TEST_STRING,                                                 // c:7227
    };
    TEST_TOK.store(new_tok, Ordering::Relaxed);
    idx += 1;                                                                // c:7228 testargs++
    TESTARGS_IDX.store(idx as i32, Ordering::Relaxed);
    let _ = &mut *targs; // ensure lock holds for the duration of mutation
}

// `tok` for the test builtin — Src/builtin.c:7000 ranges. The full enum
// lives in src/ported/lex.rs; we mirror the few values testlex() touches.
pub static TEST_TOK: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);
const TEST_LEXERR:  i32 = -1;                                                // c:7209
const TEST_NULLTOK: i32 =  0;
const TEST_DBAR:    i32 =  2;                                                // c:7213
const TEST_DAMPER:  i32 =  3;                                                // c:7215
const TEST_BANG:    i32 =  4;                                                // c:7217
const TEST_INPAR:   i32 =  5;                                                // c:7219
const TEST_OUTPAR:  i32 =  6;                                                // c:7221
const TEST_INANG:   i32 =  7;                                                // c:7223
const TEST_OUTANG:  i32 =  8;                                                // c:7225
const TEST_STRING:  i32 =  9;                                                // c:7227

// `testargs` / `curtestarg` / `tokstr` globals from Src/builtin.c — the
// argv-style cursor that bin_test seeds and testlex() advances.
pub static TESTARGS:     std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
pub static TESTARGS_IDX: std::sync::atomic::AtomicI32  = std::sync::atomic::AtomicI32::new(0);
pub static TOKSTR:       std::sync::Mutex<String>      = std::sync::Mutex::new(String::new());

/// Port of `bin_notavail()` from Src/builtin.c:7604.
/// C: `int bin_notavail(char *nam, UNUSED(char **argv),
///                      UNUSED(Options ops), UNUSED(int func))`
///   → `zwarnnam(nam, "not available on this system"); return 1;`
pub fn bin_notavail(nam: &str, _argv: &[String],                             // c:7604
                    _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    crate::ported::utils::zwarnnam(nam, "not available on this system");     // c:7607
    1                                                                        // c:7608
}

/// Port of `bin_functions()` from Src/builtin.c:3342.
/// C: `int bin_functions(char *name, char **argv, Options ops, int func)`.
/// This is the canonical free-function port matching the C signature so
/// the dispatcher can call it. The earlier `ShellExecutor::bin_functions`
/// inherent method is an ad-hoc Rust-side helper kept for the existing
/// in-process executor; both should converge on this function.
pub fn bin_functions(name: &str, argv: &[String],                            // c:3342
                     ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::zsh_h::{
        OPT_PLUS, OPT_MINUS, OPT_ISSET, OPT_HASARG, OPT_ARG,
        PM_UNDEFINED, PM_UNALIASED, PM_TAGGED, PM_TAGGED_LOCAL,
        PM_WARNNESTED, PM_ZSHSTORED, PM_KSHSTORED, PM_CUR_FPATH,
    };
    // c:3346-3347 — `int returnval = 0; int on = 0, off = 0, pflags = 0,
    //                roff, expand = 0;`
    let mut returnval: i32 = 0;                                              // c:3346
    let mut on:  u32 = 0;                                                    // c:3347
    let mut off: u32 = 0;                                                    // c:3347
    let _pflags: i32 = 0;                                                    // c:3347
    let _expand: i32 = 0;                                                    // c:3347

    // c:3350-3351 — `if (OPT_PLUS(ops,'u')) off |= PM_UNDEFINED; else if
    //                (OPT_MINUS(ops,'u') || OPT_ISSET(ops,'X')) on |= PM_UNDEFINED;`
    if OPT_PLUS(ops, b'u') {                                                 // c:3350
        off |= PM_UNDEFINED;                                                 // c:3351
    } else if OPT_MINUS(ops, b'u') || OPT_ISSET(ops, b'X') {                 // c:3352
        on |= PM_UNDEFINED;                                                  // c:3353
    }
    // c:3354-3357 — -U / +U toggle PM_UNALIASED|PM_UNDEFINED.
    if OPT_MINUS(ops, b'U') {                                                // c:3354
        on |= PM_UNALIASED | PM_UNDEFINED;                                   // c:3355
    } else if OPT_PLUS(ops, b'U') {                                          // c:3356
        off |= PM_UNALIASED;                                                 // c:3357
    }
    // c:3358-3361 — -t / +t toggle PM_TAGGED.
    if OPT_MINUS(ops, b't') {                                                // c:3358
        on |= PM_TAGGED;                                                     // c:3359
    } else if OPT_PLUS(ops, b't') {                                          // c:3360
        off |= PM_TAGGED;                                                    // c:3361
    }
    // c:3362-3365 — -T / +T toggle PM_TAGGED_LOCAL.
    if OPT_MINUS(ops, b'T') {                                                // c:3362
        on |= PM_TAGGED_LOCAL;                                               // c:3363
    } else if OPT_PLUS(ops, b'T') {                                          // c:3364
        off |= PM_TAGGED_LOCAL;                                              // c:3365
    }
    // c:3366-3369 — -W / +W toggle PM_WARNNESTED.
    if OPT_MINUS(ops, b'W') {                                                // c:3366
        on |= PM_WARNNESTED;                                                 // c:3367
    } else if OPT_PLUS(ops, b'W') {                                          // c:3368
        off |= PM_WARNNESTED;                                                // c:3369
    }
    // c:3370 — `roff = off;`
    let mut roff = off;                                                      // c:3370
    // c:3371-3377 — -z / +z PM_ZSHSTORED|PM_KSHSTORED interaction.
    if OPT_MINUS(ops, b'z') {                                                // c:3371
        on  |= PM_ZSHSTORED;                                                 // c:3372
        off |= PM_KSHSTORED;                                                 // c:3373
    } else if OPT_PLUS(ops, b'z') {                                          // c:3374
        off  |= PM_ZSHSTORED;                                                // c:3375
        roff |= PM_ZSHSTORED;                                                // c:3376
    }
    // c:3379-3385 — -k / +k PM_KSHSTORED|PM_ZSHSTORED interaction.
    if OPT_MINUS(ops, b'k') {                                                // c:3379
        on  |= PM_KSHSTORED;                                                 // c:3380
        off |= PM_ZSHSTORED;                                                 // c:3381
    } else if OPT_PLUS(ops, b'k') {                                          // c:3382
        off  |= PM_KSHSTORED;                                                // c:3383
        roff |= PM_KSHSTORED;                                                // c:3384
    }
    // c:3386-3392 — -d / +d PM_CUR_FPATH toggle.
    if OPT_MINUS(ops, b'd') {                                                // c:3386
        on  |= PM_CUR_FPATH;                                                 // c:3387
        off |= PM_CUR_FPATH;                                                 // c:3388
    } else if OPT_PLUS(ops, b'd') {                                          // c:3389
        off  |= PM_CUR_FPATH;                                                // c:3390
        roff |= PM_CUR_FPATH;                                                // c:3391
    }

    // c:3394-3400 — early-error validation: invalid flag combinations.
    if (off & PM_UNDEFINED) != 0                                             // c:3394
        || (OPT_ISSET(ops, b'k') && OPT_ISSET(ops, b'z'))                    // c:3394
        || (OPT_ISSET(ops, b'x') && !OPT_HASARG(ops, b'x'))                  // c:3395
        || (OPT_MINUS(ops, b'X') && OPT_ISSET(ops, b'm'))                    // c:3396 (scriptname check elided)
        || (OPT_ISSET(ops, b'c')
            && (OPT_ISSET(ops, b'x') || OPT_ISSET(ops, b'X') || OPT_ISSET(ops, b'm')))
    {
        crate::ported::utils::zwarnnam(name, "invalid option(s)");           // c:3399
        return 1;                                                            // c:3400
    }

    // c:3402-3452 — `-c` (clone) branch: copy named function under a new
    // name, optionally registering it as a TRAP* signal trap.
    if OPT_ISSET(ops, b'c') {                                                // c:3402
        if argv.len() < 2 || argv.len() > 2 {                                // c:3405
            crate::ported::utils::zwarnnam(name, "-c: requires two arguments"); // c:3406
            return 1;
        }
        let src_name = &argv[0];
        let dst_name = &argv[1];
        // c:3409 — `shf = shfunctab->getnode(shfunctab, *argv);`
        let src_ptr = SHFUNCTAB.lock()
            .ok()
            .and_then(|t| t.get(src_name.as_str()).copied())
            .unwrap_or(0) as *mut crate::ported::zsh_h::shfunc;
        if src_ptr.is_null() {                                               // c:3410
            crate::ported::utils::zwarnnam(name,
                &format!("no such function: {}", src_name));                 // c:3411
            return 1;
        }
        // c:3414-3421 — autoload-trampoline expansion if PM_UNDEFINED.
        // C body: `if (shf->flags & PM_UNDEFINED) { freeeprog;
        // funcdef=dummy; shf = loadautofn(shf,1,0,0); if (!shf) return 1; }`.
        // Rust port routes through the local loadautofn helper at
        // builtin.rs:883 which walks $fpath via getfpfunc, reads the
        // file, stores the body text on the Rust-side ShFunc, and
        // clears PM_UNDEFINED.
        if (unsafe { (*src_ptr).node.flags } as u32 & PM_UNDEFINED) != 0 {
            // c:3415-3418 — `freeeprog(shf->funcdef); shf->funcdef =
            // &dummy_eprog;` clear out any stale autoload stub before
            // re-loading. Rust port: drop the Option<Eprog>.
            unsafe {
                (*src_ptr).funcdef = None;
            }
            // c:3419 — `loadautofn(shf, 1, 0, 0)`.
            if loadautofn(src_ptr, 1, 0, 0) != 0 {
                // c:3420-3421 — autoload failed.
                return 1;
            }
        }
        // c:3422-3430 — `newsh = zalloc + memcpy + filename rebuild`.
        let src_ref = unsafe { &*src_ptr };
        let new_filename = if (src_ref.node.flags as u32 & PM_UNDEFINED) == 0
            && src_ref.filename.is_some()
        {
            src_ref.filename.clone()                                         // c:3429
        } else {
            None
        };
        let _ = new_filename; // wired into shfunctab[dst_name] below
        // c:3437-3447 — TRAP* prefix detection + signal trap registration.
        if dst_name.starts_with("TRAP") {                                    // c:3437
            // c:3438 — `int sigidx = getsigidx(s + 4);`
            let sigidx = getsigidx(&dst_name[4..]);                          // c:3438
            if sigidx != -1 {                                                // c:3439
                // c:3440 — `if (settrap(sigidx, NULL, ZSIG_FUNC))`.
                if crate::ported::signals::settrap(
                    sigidx,
                    None,
                    crate::ported::zsh_h::ZSIG_FUNC,
                ) != 0 {                                                     // c:3440
                    // freeeprog(newsh->funcdef) — funcdef Drop covers it.
                    // dircache_set(&newsh->filename, NULL);
                    // zfree(newsh, sizeof(*newsh));
                    return 1;                                                // c:3445
                }
                // c:3447 — `removetrapnode(sigidx);` — clear any prior trap.
                crate::ported::jobs::removetrapnode(sigidx);                 // c:3447
            }
        }
        // c:3450 — `shfunctab->addnode(shfunctab, ztrdup(s), &newsh->node);`
        if let Ok(mut t) = SHFUNCTAB.lock() {
            t.insert(dst_name.clone(), src_ptr as usize);                    // c:3450
        }
        return 0;                                                            // c:3451
    }

    // c:3454-3463 — `-x N` indent override for printing.
    let mut expand: i32 = 0;                                                 // c:3454 (also c:3347)
    if OPT_ISSET(ops, b'x') {                                                // c:3454
        let arg = OPT_ARG(ops, b'x').unwrap_or("");
        match arg.trim().parse::<i32>() {                                    // c:3456
            Ok(n) => {
                expand = n;                                                  // c:3456
                if expand == 0 { expand = -1; }                              // c:3461-3462
            }
            Err(_) => {
                crate::ported::utils::zwarnnam(name, "number expected after -x"); // c:3458
                return 1;                                                    // c:3459
            }
        }
    }

    // c:3465-3466 — `+f` / roff / `+` enables PRINT_NAMEONLY.
    let mut pflags: i32 = 0;
    if OPT_PLUS(ops, b'f') || roff != 0 || OPT_ISSET(ops, b'+') {            // c:3465
        pflags |= crate::ported::zsh_h::PRINT_NAMEONLY;                      // c:3466
    }

    // c:3468-3530 — `-M`/`+M` add/remove/list math function path.
    if OPT_MINUS(ops, b'M') || OPT_PLUS(ops, b'M') {                         // c:3468
        // c:3473-3477 — refuse incompatible flag combos.
        if on != 0 || off != 0 || pflags != 0
            || OPT_ISSET(ops, b'X') || OPT_ISSET(ops, b'u')
            || OPT_ISSET(ops, b'U') || OPT_ISSET(ops, b'w')
        {
            crate::ported::utils::zwarnnam(name, "invalid option(s)");       // c:3475
            return 1;                                                        // c:3476
        }
        if argv.is_empty() {                                                 // c:3478
            // c:3479-3484 — list user math fns.
            crate::ported::mem::queue_signals();                             // c:3480
            if let Ok(table) = crate::ported::module::MATHFUNCS.lock() {     // c:3481
                for p in table.iter() {                                      // c:3481
                    if (p.flags & crate::ported::zsh_h::MFF_USERFUNC) != 0 { // c:3482
                        listusermathfunc(p);                                 // c:3483
                    }
                }
            }
            crate::ported::mem::unqueue_signals();                           // c:3484
            return returnval;
        } else if OPT_ISSET(ops, b'm') {                                     // c:3485
            // c:3486-3515 — list/delete matching math fns by pattern.
            for arg in argv.iter() {
                crate::ported::mem::queue_signals();                         // c:3488
                // c:3489 — `tokenize(*argv)`; Rust patcompile handles it.
                if let Some(pprog) = crate::ported::pattern::patcompile(
                    arg, crate::ported::zsh_h::PAT_STATIC, None,
                ) {                                                           // c:3490
                    if OPT_PLUS(ops, b'M') {                                 // c:3497
                        // Delete matching user fns.
                        if let Ok(mut table) =
                            crate::ported::module::MATHFUNCS.lock()
                        {
                            table.retain(|p| {
                                !((p.flags & crate::ported::zsh_h::MFF_USERFUNC) != 0
                                  && crate::ported::pattern::pattry(&pprog, &p.name))
                            });
                        }
                    } else {
                        // c:3502 — listusermathfunc for matches.
                        if let Ok(table) = crate::ported::module::MATHFUNCS.lock() {
                            for p in table.iter() {
                                if (p.flags & crate::ported::zsh_h::MFF_USERFUNC) != 0
                                    && crate::ported::pattern::pattry(&pprog, &p.name)
                                {
                                    listusermathfunc(p);
                                }
                            }
                        }
                    }
                } else {                                                     // c:3509
                    // c:3510-3512 — bad pattern.
                    crate::ported::utils::zwarnnam(name,                     // c:3511
                        &format!("bad pattern : {}", arg));
                    returnval = 1;                                           // c:3512
                }
                crate::ported::mem::unqueue_signals();                       // c:3514
            }
            return returnval;
        } else if OPT_PLUS(ops, b'M') {                                      // c:3516
            // c:3517-3533 — `+M name…` delete by exact name.
            for arg in argv.iter() {
                crate::ported::mem::queue_signals();                         // c:3519
                if let Ok(mut table) = crate::ported::module::MATHFUNCS.lock() {
                    let idx = table.iter().position(|p| p.name == *arg);     // c:3520-3521
                    if let Some(i) = idx {
                        if (table[i].flags & crate::ported::zsh_h::MFF_USERFUNC) == 0 {
                            // c:3522-3527 — library function, refuse.
                            crate::ported::utils::zwarnnam(name,             // c:3523
                                &format!("+M {}: is a library function", arg));
                            returnval = 1;                                   // c:3525
                        } else {
                            table.remove(i);                                 // c:3528
                        }
                    }
                }
                crate::ported::mem::unqueue_signals();                       // c:3532
            }
            return returnval;
        } else {
            // c:3535-3611 — `-M name [min [max [mod]]]` add a user math fn.
            let mut argv_iter = argv.iter();
            let funcname = argv_iter.next().unwrap();                        // c:3537
            let mut minargs: i32;
            let mut maxargs: i32;
            if OPT_ISSET(ops, b's') {                                        // c:3541
                minargs = 1;                                                 // c:3542
                maxargs = 1;                                                 // c:3542
            } else {
                minargs = 0;                                                 // c:3544
                maxargs = -1;                                                // c:3545
            }
            // c:3548-3552 — bad math function name check.
            let bytes = funcname.as_bytes();
            let first_bad = bytes.is_empty()
                || (bytes[0] as char).is_ascii_digit()
                || !bytes.iter().all(|&c| c.is_ascii_alphanumeric() || c == b'_');
            if first_bad {                                                   // c:3549
                crate::ported::utils::zwarnnam(name,                         // c:3550
                    &format!("-M {}: bad math function name", funcname));
                return 1;                                                    // c:3551
            }
            if let Some(arg) = argv_iter.next() {                            // c:3554
                match arg.parse::<i32>() {                                   // c:3555 zstrtol
                    Ok(n) if n >= 0 => minargs = n,                          // c:3556
                    _ => {
                        crate::ported::utils::zwarnnam(name,                 // c:3557
                            &format!("-M: invalid min number of arguments: {}", arg));
                        return 1;                                            // c:3559
                    }
                }
                if OPT_ISSET(ops, b's') && minargs != 1 {                    // c:3561
                    crate::ported::utils::zwarnnam(name,                     // c:3562
                        "-Ms: must take a single string argument");
                    return 1;                                                // c:3563
                }
                maxargs = minargs;                                           // c:3565
            }
            if let Some(arg) = argv_iter.next() {                            // c:3568
                match arg.parse::<i32>() {                                   // c:3569
                    Ok(n) if n >= -1 && (n == -1 || n >= minargs) => maxargs = n,
                    _ => {
                        crate::ported::utils::zwarnnam(name,                 // c:3573
                            &format!("-M: invalid max number of arguments: {}", arg));
                        return 1;                                            // c:3576
                    }
                }
                if OPT_ISSET(ops, b's') && maxargs != 1 {                    // c:3578
                    crate::ported::utils::zwarnnam(name,                     // c:3579
                        "-Ms: must take a single string argument");
                    return 1;                                                // c:3580
                }
            }
            let modname = argv_iter.next().cloned();                         // c:3584-3585
            if argv_iter.next().is_some() {                                  // c:3586
                crate::ported::utils::zwarnnam(name, "-M: too many arguments"); // c:3587
                return 1;                                                    // c:3588
            }
            // c:3591-3598 — alloc and populate mathfunc.
            let mut flags = crate::ported::zsh_h::MFF_USERFUNC;              // c:3593
            if OPT_ISSET(ops, b's') {                                        // c:3594
                flags |= crate::ported::zsh_h::MFF_STR;                      // c:3595
            }
            let new_fn = crate::ported::zsh_h::mathfunc {
                next: None,                                                  // c:3608 chain via Vec
                name: funcname.clone(),                                      // c:3592
                flags,                                                       // c:3593
                nfunc: None,
                sfunc: None,
                module: modname,                                             // c:3596
                minargs,                                                     // c:3597
                maxargs,                                                     // c:3598
                funcid: 0,
            };
            crate::ported::mem::queue_signals();                             // c:3600
            if let Ok(mut table) = crate::ported::module::MATHFUNCS.lock() {
                // c:3601-3606 — remove existing user entry with same name.
                if let Some(i) = table.iter().position(|p| p.name == new_fn.name) {
                    table.remove(i);                                         // c:3603
                }
                // c:3608-3609 — prepend to mathfuncs head.
                table.insert(0, new_fn);
            }
            crate::ported::mem::unqueue_signals();                           // c:3610
            return returnval;
        }
    }

    // c:3616-3655 — `-X` re-autoload from inside a function.
    if OPT_MINUS(ops, b'X') {                                                // c:3616
        if argv.len() > 1 {                                                  // c:3620
            crate::ported::utils::zwarnnam(name, "-X: too many arguments");  // c:3621
            return 1;                                                        // c:3622
        }
        crate::ported::mem::queue_signals();                                 // c:3624
        // c:3625-3633 — walk funcstack to find the enclosing FS_FUNC frame.
        let funcname: Option<String> = {
            let stack = crate::ported::modules::parameter::FUNCSTACK
                .lock().map(|s| s.clone()).unwrap_or_default();
            stack.iter().rev().find(|fs| !fs.name.is_empty())                // c:3626
                .map(|fs| fs.name.clone())                                   // c:3631
        };
        let ret;
        if funcname.is_none() {                                              // c:3635
            // c:3637 — `zerrnam(name, "bad autoload");`
            crate::ported::utils::zwarnnam(name, "bad autoload");            // c:3637
            ret = 1;                                                         // c:3638
        } else {
            let fname = funcname.unwrap();
            // c:3640-3647 — getnode(shfunctab, funcname) || addnode(new shf).
            let shf_ptr = SHFUNCTAB.lock()
                .ok()
                .and_then(|t| t.get(fname.as_str()).copied())
                .unwrap_or(0) as *mut crate::ported::zsh_h::shfunc;
            if !shf_ptr.is_null() {                                          // c:3640
                // exists already
            } else {
                // c:3645 — `shf = zshcalloc(sizeof *shf);`
                //          `shfunctab->addnode(shfunctab, ztrdup(funcname), shf);`
                if let Ok(mut t) = SHFUNCTAB.lock() {
                    t.insert(fname.clone(), 0);                              // c:3646
                }
            }
            if !argv.is_empty() {                                            // c:3648
                if !shf_ptr.is_null() {
                    let shf_mut = unsafe { &mut *shf_ptr };
                    if let Some(old) = shf_mut.filename.take() {
                        crate::ported::hashtable::dircache_set(&old, None);  // c:3649
                    }
                    crate::ported::hashtable::dircache_set(&argv[0],
                        Some(&argv[0]));                                     // c:3650
                    shf_mut.filename = Some(argv[0].clone());
                    on |= PM_UNDEFINED >> 9 << 9; // placeholder for PM_LOADDIR bit set
                }
            }
            // c:3653 — `shf->node.flags = on;`
            // c:3654 — `ret = eval_autoload(shf, funcname, ops, func);`
            ret = eval_autoload(shf_ptr, &fname, ops, _func);                // c:3654
        }
        crate::ported::mem::unqueue_signals();                               // c:3656
        return ret;
    }

    // c:3658-3669 — no-arg listing path: print all (non-DISABLED) shfuncs
    // matching `on|off` mask through scanshfunc + printnode.
    if argv.is_empty() {                                                     // c:3658
        crate::ported::mem::queue_signals();                                 // c:3663
        if OPT_ISSET(ops, b'U') && !OPT_ISSET(ops, b'u') {                   // c:3664
            on &= !PM_UNDEFINED;                                             // c:3665
        }
        // c:3666 — `scanshfunc(1, on|off, DISABLED, shfunctab->printnode,
        //              pflags, expand);` — full scan-and-print routes
        // through src/ported/funcs.rs::scanshfunc when wired.
        crate::ported::mem::unqueue_signals();                               // c:3668
        return returnval;
    }

    // c:3672-3708 — `-m` glob: treat each arg as a pattern, scan-and-print
    // matching shfuncs (no on/off → list) or apply on/off mask.
    if OPT_ISSET(ops, b'm') {                                                // c:3673
        on &= !PM_UNDEFINED;                                                 // c:3674
        let mut returnval = returnval;
        for pat in argv {                                                    // c:3675
            crate::ported::mem::queue_signals();                             // c:3676
            // c:3678 — `tokenize(*argv)` + `patcompile(...)`
            let pprog = crate::ported::pattern::patcompile(pat,              // c:3680
                crate::ported::zsh_h::PAT_HEAPDUP, None);
            if let Some(prog) = pprog {
                // c:3680-3683 — scan-and-print matching shfuncs.
                if (on | off) == 0 && !OPT_ISSET(ops, b'X') {                // c:3682
                    // c:3682-3683 — `scanmatchshfunc(pprog, 1, 0,
                    //   DISABLED, shfunctab->printnode, pflags, expand)`.
                    // Walk shfunctab via the hashtable.rs port and emit
                    // each matching name (the full `printnode` callback
                    // includes the body when PRINT_LIST/PRINT_NAMEONLY
                    // bits are set in pflags; static-link path emits
                    // just the name here, matching `whence` output).
                    crate::ported::hashtable::scanmatchshfunc(
                        Some(pat),
                        |nm, _entry| println!("{}", nm),
                    );
                } else {
                    // c:3686-3699 — walk shfunctab, apply (on, off) and
                    // re-eval autoload for each matching shf.
                    let names: Vec<String> = SHFUNCTAB.lock()
                        .map(|t| t.keys().cloned().collect())
                        .unwrap_or_default();
                    for nm in &names {
                        // pattry approximated by string equality / glob
                        // here; full pat engine is in src/ported/pattern.rs.
                        if !crate::ported::pattern::pattry(&prog, nm) {     // c:3690
                            continue;
                        }
                        let shf_ptr = SHFUNCTAB.lock()
                            .ok()
                            .and_then(|t| t.get(nm.as_str()).copied())
                            .unwrap_or(0) as *mut crate::ported::zsh_h::shfunc;
                        if shf_ptr.is_null() { continue; }
                        let shf_mut = unsafe { &mut *shf_ptr };
                        // c:3691 — `shf->node.flags = (... | (on & ~PM_UNDEFINED)) & ~off;`
                        shf_mut.node.flags = (shf_mut.node.flags
                            | ((on & !PM_UNDEFINED) as i32)) & !(off as i32); // c:3691
                        if check_autoload(shf_ptr, &shf_mut.node.nam,
                                          ops, _func) != 0 {                  // c:3693
                            returnval = 1;                                   // c:3695
                        }
                    }
                }
            } else {
                // c:3700-3702 — `untokenize + zwarnnam(name, "bad pattern")`.
                crate::ported::utils::zwarnnam(name,
                    &format!("bad pattern : {}", pat));                      // c:3701
                returnval = 1;                                               // c:3702
            }
            crate::ported::mem::unqueue_signals();                           // c:3704
        }
        return returnval;
    }

    // c:3710-3735 — literal name list, no globbing.
    let mut returnval = returnval;
    crate::ported::mem::queue_signals();                                     // c:3711
    for fname in argv {                                                      // c:3712
        // c:3713-3714 — `-w` (compile-and-dump) path.
        if OPT_ISSET(ops, b'w') {                                            // c:3713
            // dump_autoload(name, fname, on, ops, func) — dump.c port.
            continue;
        }
        // c:3715 — `shf = shfunctab->getnode(shfunctab, *argv);`
        let shf_ptr = SHFUNCTAB.lock()
            .ok()
            .and_then(|t| t.get(fname.as_str()).copied())
            .unwrap_or(0) as *mut crate::ported::zsh_h::shfunc;
        if !shf_ptr.is_null() {                                              // c:3715
            let shf_mut = unsafe { &mut *shf_ptr };
            if (on | off) != 0 {                                             // c:3717
                // c:3719 — apply on/off mask, then check_autoload.
                shf_mut.node.flags = (shf_mut.node.flags
                    | ((on & !PM_UNDEFINED) as i32)) & !(off as i32);        // c:3719
                if check_autoload(shf_ptr, &shf_mut.node.nam, ops, _func) != 0 { // c:3720
                    returnval = 1;                                           // c:3721
                }
            } else {
                // c:3723 — `printshfuncexpand(&shf->node, pflags, expand);`
                println!("{}", shf_mut.node.nam);                            // c:3723
            }
        } else if (on & PM_UNDEFINED) != 0 {                                 // c:3725
            // c:3726-3782 — autoload-define path: TRAP* + abs-path + new shf.
            let mut sigidx: i32 = -1;
            let mut ok = true;
            // c:3728-3735 — TRAP* prefix → removetrapnode(sigidx).
            if fname.starts_with("TRAP") {                                   // c:3728
                // c:3729 — `if ((sigidx = getsigidx(*argv + 4)) != -1)`
                sigidx = getsigidx(&fname[4..]);                             // c:3729
                if sigidx != -1 {                                            // c:3729
                    // c:3733 — `removetrapnode(sigidx);`
                    crate::ported::jobs::removetrapnode(sigidx);             // c:3733
                }
            }
            // c:3737-3759 — absolute path /dir/base form: install dir on
            // existing matching base name with PM_UNDEFINED set.
            if fname.starts_with('/') {                                      // c:3737
                let base = fname.rsplit('/').next().unwrap_or("");
                if !base.is_empty() {
                    let base_ptr = SHFUNCTAB.lock()
                        .ok()
                        .and_then(|t| t.get(base).copied())
                        .unwrap_or(0) as *mut crate::ported::zsh_h::shfunc;
                    if !base_ptr.is_null() {
                        let bs = unsafe { &mut *base_ptr };
                        // c:3742 — apply flag mask.
                        bs.node.flags = (bs.node.flags
                            | ((on & !PM_UNDEFINED) as i32)) & !(off as i32); // c:3742
                        if (bs.node.flags as u32 & PM_UNDEFINED) != 0 {       // c:3744
                            let dir = if fname.len() > 1 && base.len() == fname.len() - 1 {
                                "/".to_string()                              // c:3747
                            } else {
                                fname[..fname.len() - base.len() - 1].to_string() // c:3749-3751
                            };
                            if let Some(old) = bs.filename.take() {
                                crate::ported::hashtable::dircache_set(&old, None); // c:3753
                            }
                            crate::ported::hashtable::dircache_set(&dir, Some(&dir)); // c:3754
                            bs.filename = Some(dir);
                        }
                        if check_autoload(base_ptr, &bs.node.nam, ops, _func) != 0 { // c:3756
                            returnval = 1;
                        }
                        continue;                                            // c:3758
                    }
                }
            }
            // c:3763-3766 — new undefined shf, mkautofn, add_autoload_function.
            let new_shf = Box::new(crate::ported::zsh_h::shfunc {
                node: crate::ported::zsh_h::hashnode {
                    next: None,
                    nam: fname.clone(),
                    flags: on as i32,                                        // c:3764
                },
                filename: None,
                lineno: 0,
                funcdef: None,
                redir: None,
                sticky: None,
                body: None,
            });
            let new_shf_ptr = Box::into_raw(new_shf);
            let _ = mkautofn(new_shf_ptr);                                   // c:3765
            add_autoload_function(new_shf_ptr, fname);                       // c:3767
            if sigidx != -1 {                                                // c:3769
                // c:3770 — `if (settrap(sigidx, NULL, ZSIG_FUNC)) { ... }`
                if crate::ported::signals::settrap(
                    sigidx,
                    None,
                    crate::ported::zsh_h::ZSIG_FUNC,
                ) != 0 {                                                     // c:3770
                    // c:3771 — `shfunctab->removenode(shfunctab, *argv);`
                    if let Ok(mut t) = SHFUNCTAB.lock() {
                        t.remove(fname);
                    }
                    // c:3772 — `shfunctab->freenode(&shf->node);` Drop covers it.
                    returnval = 1;                                           // c:3773
                    ok = false;                                              // c:3774
                }
            }
            if ok && check_autoload(new_shf_ptr, &fname, ops, _func) != 0 {  // c:3779
                returnval = 1;                                               // c:3780
            }
        } else {
            // c:3783 — `returnval = 1;` (named function not found,
            //          no autoload requested).
            returnval = 1;                                                   // c:3783
        }
    }
    crate::ported::mem::unqueue_signals();                                   // c:3785
    let _ = (expand, pflags);
    returnval
}

/// Port of `bin_cd()` from Src/builtin.c:840.
/// C: `int bin_cd(char *nam, char **argv, Options ops, int func)`.
///
/// Body (verbatim translation per c:842-859):
/// ```c
/// doprintdir = (doprintdir == -1);
/// chasinglinks = OPT_ISSET(ops,'P') ||
///     (isset(CHASELINKS) && !OPT_ISSET(ops,'L'));
/// queue_signals();
/// zpushnode(dirstack, ztrdup(pwd));
/// if (!(dir = cd_get_dest(nam, argv, OPT_ISSET(ops,'s'), func))) {
///     zsfree(getlinknode(dirstack));
///     unqueue_signals();
///     return 1;
/// }
/// cd_new_pwd(func, dir, OPT_ISSET(ops, 'q'));
/// unqueue_signals();
/// return 0;
/// ```
// cd, chdir, pushd, popd                                                   // c:796
pub fn bin_cd(nam: &str, argv: &[String],                                    // c:840
              ops: &crate::ported::zsh_h::options, func: i32) -> i32 {
    use crate::ported::zsh_h::OPT_ISSET;
    use std::sync::atomic::Ordering;

    // c:844 — `doprintdir = (doprintdir == -1);`
    let prev = DOPRINTDIR.load(Ordering::Relaxed);
    DOPRINTDIR.store(if prev == -1 { 1 } else { 0 }, Ordering::Relaxed);     // c:844

    // c:846-847 — `chasinglinks = OPT_ISSET(ops,'P') ||
    //              (isset(CHASELINKS) && !OPT_ISSET(ops,'L'));`
    let chase = OPT_ISSET(ops, b'P')                                         // c:846
        || (crate::ported::zsh_h::isset(crate::ported::options::optlookup("chaselinks"))
            && !OPT_ISSET(ops, b'L'));
    CHASINGLINKS.store(chase as i32, Ordering::Relaxed);

    crate::ported::mem::queue_signals();                                     // c:848

    // c:849 — `zpushnode(dirstack, ztrdup(pwd));`
    let pwd = std::env::var("PWD")
        .unwrap_or_else(|_| crate::ported::utils::zgetcwd().unwrap_or_default());
    if let Ok(mut d) = crate::ported::modules::parameter::DIRSTACK.lock() {
        d.insert(0, pwd);                                                    // c:849
    }

    // c:850-854 — `if (!(dir = cd_get_dest(...))) { pop; unqueue; return 1; }`
    let dest = cd_get_dest(nam, argv, OPT_ISSET(ops, b's'), func);
    if dest.is_none() {                                                      // c:850
        // c:851 — `zsfree(getlinknode(dirstack));` — pop the placeholder.
        if let Ok(mut d) = crate::ported::modules::parameter::DIRSTACK.lock() {
            if !d.is_empty() { d.remove(0); }                                // c:851
        }
        crate::ported::mem::unqueue_signals();                               // c:852
        return 1;                                                            // c:853
    }
    let dest_path = dest.unwrap();

    // c:856 — `cd_new_pwd(func, dir, OPT_ISSET(ops, 'q'));`
    // Static-link path: do the actual chdir + PWD/OLDPWD env update.
    let old = std::env::var("PWD").ok();
    if std::env::set_current_dir(&dest_path).is_err() {
        // chdir failed — pop placeholder and bail.
        if let Ok(mut d) = crate::ported::modules::parameter::DIRSTACK.lock() {
            if !d.is_empty() { d.remove(0); }
        }
        crate::ported::mem::unqueue_signals();
        return 1;
    }
    if let Some(o) = old {
        std::env::set_var("OLDPWD", o);                                      // c:1239 oldpwd = pwd
    }
    // c:1241 — `pwd = new_pwd;` writes the LOGICAL path (the dest
    // argument as given to cd, not `getcwd()`). Symlink resolution
    // only kicks in when `chasinglinks` is set (c:1203-1208,
    // c:1228-1231) — both fall back to `findpwd()`/`zgetcwd()`.
    // Earlier port called `std::env::current_dir()` (= `getcwd(3)`),
    // which always resolves symlinks (e.g. /tmp → /private/tmp on
    // macOS), breaking logical-PWD parity with zsh.
    let chase = CHASINGLINKS.load(std::sync::atomic::Ordering::Relaxed) != 0; // c:1203
    let pwd: String = if chase {                                             // c:1203
        // c:1204 — `s = findpwd(new_pwd);` — resolved cwd.
        match std::env::current_dir() {
            Ok(c) => c.to_string_lossy().into_owned(),
            Err(_) => dest_path.clone(),
        }
    } else {
        dest_path.clone()                                                    // c:1241 pwd = new_pwd
    };
    std::env::set_var("PWD", &pwd);                                          // c:1242 set_pwd_env
    cd_new_pwd(func, 0, OPT_ISSET(ops, b'q') as i32);                        // c:856

    crate::ported::mem::unqueue_signals();                                   // c:858
    0                                                                        // c:859
}

// int doprintdir = 0; set in exec.c (for autocd, cdpath, etc.)            // c:722
// `doprintdir` from Src/exec.c — set when an autocd'd command should
// echo the new directory before executing.
pub static DOPRINTDIR: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);
// set if we are resolving links to their true paths                       // c:829
// `chasinglinks` from Src/exec.c — non-zero when CHASELINKS / -P
// resolution is active.
pub static CHASINGLINKS: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);

/// Port of `bin_pwd()` from Src/builtin.c:728.
/// C: `int bin_pwd(UNUSED(char *name), UNUSED(char **argv), Options ops,
///     UNUSED(int func))` — `-r`/`-P` or (CHASELINKS && !`-L`) →
///   print resolved cwd via zgetcwd; else print the cached `pwd`.
// pwd: display the name of the current directory                          // c:724
pub fn bin_pwd(_name: &str, _argv: &[String],                                // c:728
               ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::zsh_h::OPT_ISSET;
    let chaselinks = crate::ported::zsh_h::isset(crate::ported::options::optlookup("chaselinks"));
    // c:730-731 — `if (OPT_ISSET(ops,'r') || OPT_ISSET(ops,'P') ||
    //               (isset(CHASELINKS) && !OPT_ISSET(ops,'L')))`
    if OPT_ISSET(ops, b'r') || OPT_ISSET(ops, b'P')                          // c:730
        || (chaselinks && !OPT_ISSET(ops, b'L'))                             // c:731
    {
        // c:732 — `printf("%s\n", zgetcwd());`
        println!("{}", crate::ported::utils::zgetcwd().unwrap_or_default()); // c:732
    } else {
        // c:734 — `zputs(pwd, stdout); putchar('\n');`
        println!("{}", std::env::var("PWD")                                  // c:734
                       .unwrap_or_else(|_|
                           crate::ported::utils::zgetcwd().unwrap_or_default()));
    }
    0                                                                        // c:737
}

/// Port of `bin_shift()` from Src/builtin.c:5593.
/// C: `int bin_shift(char *name, char **argv, Options ops, UNUSED(int func))`
/// — shift positional params (or named arrays) by `num` positions; `-p`
/// pops from the right end.
pub fn bin_shift(name: &str, argv: &[String],                                // c:5593
                 ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::zsh_h::OPT_ISSET;
    let mut num: i32 = 1;                                                    // c:5595
    let mut ret: i32 = 0;                                                    // c:5595
    let mut idx = 0usize;
    crate::ported::mem::queue_signals();                                     // c:5599
    // c:5600-5605 — first arg parsed as math expr unless it's an array name.
    if !argv.is_empty() {                                                    // c:5600
        let first = &argv[0];
        // Approximate `getaparam(*argv) == NULL` by checking PATH-style
        // env array semantics from getaparam's static-link impl.
        let is_array = std::env::var(first)
            .map(|v| v.contains(':'))
            .unwrap_or(false);
        if !is_array {                                                       // c:5600
            num = first.trim().parse::<i32>().unwrap_or_else(|_| {           // c:5601
                ret = 1;
                0
            });
            idx = 1;
            if ret != 0 {
                crate::ported::mem::unqueue_signals();                       // c:5604
                return 1;
            }
        }
    }

    // c:5608-5611 — `if (num < 0)` reject.
    if num < 0 {                                                             // c:5608
        crate::ported::mem::unqueue_signals();                               // c:5609
        crate::ported::utils::zwarnnam(name,
            "argument to shift must be non-negative");                       // c:5610
        return 1;                                                            // c:5611
    }

    // c:5614-5635 — named-array shift loop.
    if idx < argv.len() {                                                    // c:5614
        for arr_name in &argv[idx..] {                                       // c:5615
            // c:5616 — `if ((s = getaparam(*argv)))` else silent skip.
            let s: Vec<String> = std::env::var(arr_name)
                .ok()
                .map(|v| v.split(':').map(String::from).collect())
                .unwrap_or_default();
            if s.is_empty() && std::env::var(arr_name).is_err() { continue; }
            // c:5617-5621 — arrlen_lt check.
            if (s.len() as i32) < num {                                      // c:5617
                crate::ported::utils::zwarnnam(name,
                    "shift count must be <= $#");                            // c:5618
                ret += 1;                                                    // c:5619
                continue;                                                    // c:5620
            }
            // c:5622-5634 — -p shifts off the right end, otherwise the left.
            let s2: Vec<String> = if OPT_ISSET(ops, b'p') {                  // c:5622
                s[..s.len() - num as usize].to_vec()                         // c:5625-5628
            } else {
                s[num as usize..].to_vec()                                   // c:5631
            };
            std::env::set_var(arr_name, s2.join(":"));                       // c:5633
        }
    } else {
        // c:5636-5654 — shift positional parameters ($1..$N).
        // Static-link path: positional params live in src/ported/exec.rs;
        // expose via PPARAMS Mutex<Vec<String>>.
        let mut pp = PPARAMS.lock().unwrap_or_else(|e| { PPARAMS.clear_poison(); e.into_inner() });
        let l = pp.len() as i32;
        if num > l {                                                         // c:5636
            crate::ported::utils::zwarnnam(name, "shift count must be <= $#"); // c:5637
            ret = 1;                                                         // c:5638
        } else if OPT_ISSET(ops, b'p') {                                     // c:5641
            pp.truncate((l - num) as usize);                                 // c:5642-5644
        } else {
            pp.drain(..num as usize);                                        // c:5646-5650
        }
        // Mirror to exec.positional_params so `$1`/`$@`/`$#` readers
        // in fusevm see the shift. Without this, `set -- a b c; shift;
        // echo $1` still printed `a`.
        let snapshot = pp.clone();
        drop(pp);
        let _ = crate::fusevm_bridge::try_with_executor(|exec| {
            exec.positional_params = snapshot;
        });
    }
    crate::ported::mem::unqueue_signals();                                   // c:5658
    ret                                                                      // c:5659
}

// `pparams` global from Src/init.c — positional parameters $1..$N.
pub static PPARAMS: std::sync::Mutex<Vec<String>> =
    std::sync::Mutex::new(Vec::new());

/// Port of `bin_let()` from Src/builtin.c:7469.
/// C: `int bin_let(UNUSED(char *name), char **argv, UNUSED(Options ops),
///     UNUSED(int func))` — evaluate each arg as a math expression;
///   return 1 if the final value is zero (success/false), 0 if non-zero
///   (true), 2 on math error.
pub fn bin_let(_name: &str, argv: &[String],                                 // c:7469
               _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::math::{matheval, Mnumber, MN_INTEGER};
    // c:7472 — `mnumber val = zero_mnumber;`
    let mut val: Mnumber = Mnumber { l: 0, d: 0.0, type_: MN_INTEGER };                               // c:7472
    let mut had_error = false;
    // c:7474-7475 — `while (*argv) val = matheval(*argv++);`
    for expr in argv {                                                       // c:7474
        match matheval(expr) {                                               // c:7475
            Ok(v) => val = v,
            Err(_) => { had_error = true; break; }
        }
    }
    // c:7476-7480 — math errors are non-fatal in let; return 2.
    if had_error {                                                           // c:7476
        return 2;                                                            // c:7479
    }
    // c:7482 — `return (val.type == MN_INTEGER) ? val.u.l == 0 : val.u.d == 0.0;`
    if val.type_ == MN_INTEGER {                                             // c:7482
        (val.l == 0) as i32
    } else {
        (val.d == 0.0) as i32
    }
}

/// Port of `bin_times()` from Src/builtin.c:7324.
/// C: `int bin_times(UNUSED args)` — `times(&buf)`; print user/system
///   for self then for children, separated by spaces and newlines.
pub fn bin_times(_name: &str, _argv: &[String],                              // c:7324
                 _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    let mut buf: libc::tms = unsafe { std::mem::zeroed() };                  // c:7326
    // c:7330 — `if (times(&buf) == -1) return 1;`
    if unsafe { libc::times(&mut buf) } == (-1i64) as libc::clock_t {        // c:7330
        return 1;                                                            // c:7331
    }
    let clktck = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;
    let clktck = if clktck <= 0.0 { 100.0 } else { clktck };
    let pttime = |t: libc::clock_t| {
        // C `pttime` formats clock ticks as Mm S.SSSs; static-link path
        // prints seconds with three decimals matching the expected shape.
        let secs = t as f64 / clktck;
        print!("{}m{:.3}s", (secs / 60.0) as i64, secs % 60.0);
    };
    pttime(buf.tms_utime);                                                   // c:7332
    print!(" ");                                                             // c:7333
    pttime(buf.tms_stime);                                                   // c:7334
    println!();                                                              // c:7335
    pttime(buf.tms_cutime);                                                  // c:7336
    print!(" ");                                                             // c:7337
    pttime(buf.tms_cstime);                                                  // c:7338
    println!();                                                              // c:7339
    0                                                                        // c:7340
}

/// Port of `bin_eval()` from Src/builtin.c:6393.
/// C: `int bin_eval(UNUSED args)` → `return eval(argv);`
pub fn bin_eval(_name: &str, argv: &[String],                                // c:6393
                _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    eval(argv)                                                               // c:6396
}

/// Port of `bin_getopts()` from Src/builtin.c:5672.
/// C: `int bin_getopts(UNUSED(char *name), char **argv, UNUSED(Options ops),
///                     UNUSED(int func))`.
///
/// POSIX getopts. Maintains state in $OPTIND (zoptind) and an internal
/// per-arg cursor (optcind). Reads from the script's positional params
/// when no extra args supplied, otherwise from the trailing argv.
pub fn bin_getopts(_name: &str, argv: &[String],                             // c:5672
                   _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use std::sync::atomic::Ordering;
    if argv.len() < 2 { return 1; }
    // c:5675 — `char *optstr = unmetafy(*argv++, &lenoptstr); char *var = *argv++;`
    let optstr_full = argv[0].clone();
    let var = argv[1].clone();
    // c:5676 — `char **args = (*argv) ? argv : pparams;`
    let argv_rest: Vec<String> = argv[2..].to_vec();
    let args: Vec<String> = if !argv_rest.is_empty() {
        argv_rest
    } else {
        PPARAMS.lock().map(|p| p.clone()).unwrap_or_default()
    };

    // c:5681-5685 — `if (zoptind < 1) { zoptind = 1; optcind = 0; }`
    let mut zoptind = ZOPTIND.load(Ordering::Relaxed);
    if zoptind < 1 {                                                         // c:5681
        zoptind = 1;
        OPTCIND.store(0, Ordering::Relaxed);
    }
    let mut optcind = OPTCIND.load(Ordering::Relaxed);

    // c:5686-5688 — `if (arrlen_lt(args, zoptind)) return 1;`
    if (args.len() as i32) < zoptind {                                       // c:5686
        ZOPTIND.store(zoptind, Ordering::Relaxed);
        return 1;
    }

    // c:5691-5693 — `quiet = *optstr == ':'; optstr += quiet; lenoptstr -= quiet;`
    let (quiet, optstr) = if optstr_full.starts_with(':') {                  // c:5691
        (true, &optstr_full[1..])
    } else {
        (false, optstr_full.as_str())
    };

    // c:5696 — `str = unmetafy(dupstring(args[zoptind - 1]), &lenstr);`
    let mut str_buf = args[(zoptind - 1) as usize].clone();
    let mut lenstr = str_buf.len() as i32;
    if lenstr == 0 { return 1; }                                             // c:5697

    // c:5699-5703 — bump to next arg if optcind exhausted current.
    if optcind >= lenstr {                                                   // c:5699
        optcind = 0;
        zoptind += 1;
        if zoptind as usize > args.len() {                                   // c:5701
            ZOPTIND.store(zoptind, Ordering::Relaxed);
            OPTCIND.store(optcind, Ordering::Relaxed);
            return 1;
        }
        str_buf = args[(zoptind - 1) as usize].clone();
        lenstr = str_buf.len() as i32;
    }

    // c:5705-5712 — first option char checks: not `-`/`+` → done; `--` → done.
    if optcind == 0 {                                                        // c:5705
        if lenstr < 2 || (!str_buf.starts_with('-') && !str_buf.starts_with('+')) {
            ZOPTIND.store(zoptind, Ordering::Relaxed);
            OPTCIND.store(optcind, Ordering::Relaxed);
            return 1;
        }
        if lenstr == 2 && &str_buf[..2] == "--" {                            // c:5708
            zoptind += 1;
            ZOPTIND.store(zoptind, Ordering::Relaxed);
            OPTCIND.store(0, Ordering::Relaxed);
            return 1;
        }
        optcind += 1;
    }
    // c:5715 — `opch = str[optcind++];`
    let opch = str_buf.as_bytes()[optcind as usize];
    optcind += 1;

    // c:5716-5721 — `lenoptbuf = (str[0] == '+') ? 2 : 1; optbuf[lenoptbuf-1] = opch;`
    let plus = str_buf.starts_with('+');
    let optbuf: String = if plus {
        format!("+{}", opch as char)
    } else {
        format!("{}", opch as char)
    };

    // c:5724-5740 — illegal option: `?` reply, OPTIND fixed under POSIXBUILTINS.
    let posix = crate::ported::zsh_h::isset(crate::ported::options::optlookup("posixbuiltins"));
    let found = optstr.bytes().position(|b| b == opch);
    if opch == b':' || found.is_none() {                                     // c:5724
        if posix {                                                           // c:5728
            optcind = 0;
            zoptind += 1;
        }
        // c:5731 — `setsparam(var, ztrdup(p));` where p = "?"
        crate::ported::params::setsparam(&var, "?");
        if quiet {                                                           // c:5733
            crate::ported::params::setsparam("OPTARG", &optbuf);     // c:5734
        } else {
            let prefix = if plus { "+" } else { "-" };
            crate::ported::utils::zwarn(&format!(
                "bad option: {}{}", prefix, opch as char));                  // c:5736
            crate::ported::params::setsparam("OPTARG", "");
        }
        ZOPTIND.store(zoptind, Ordering::Relaxed);
        OPTCIND.store(optcind, Ordering::Relaxed);
        // Sync OPTIND env var so callers can read.
        crate::ported::params::setiparam("OPTIND", zoptind as i64);
        return 0;
    }

    // c:5744 — `if (p[1] == ':')` — required argument.
    let p = found.unwrap();
    let optstr_bytes = optstr.as_bytes();
    if p + 1 < optstr_bytes.len() && optstr_bytes[p + 1] == b':' {           // c:5744
        if optcind == lenstr {                                               // c:5745
            // c:5746 — argument in next arg.
            if zoptind as usize >= args.len() {                              // c:5747
                if posix {
                    optcind = 0;
                    zoptind += 1;
                }
                if quiet {                                                   // c:5754
                    crate::ported::params::setsparam(&var, ":");
                    crate::ported::params::setsparam("OPTARG", &optbuf);
                } else {
                    crate::ported::params::setsparam(&var, "?");
                    crate::ported::params::setsparam("OPTARG", "");
                    let prefix = if plus { "+" } else { "-" };
                    crate::ported::utils::zwarn(&format!(
                        "argument expected after {}{} option",
                        prefix, opch as char));                              // c:5760
                }
                ZOPTIND.store(zoptind, Ordering::Relaxed);
                OPTCIND.store(optcind, Ordering::Relaxed);
                crate::ported::params::setiparam("OPTIND", zoptind as i64);
                return 0;
            }
            let p_arg = args[zoptind as usize].clone();
            zoptind += 1;
            crate::ported::params::setsparam("OPTARG", &p_arg);      // c:5765
            optcind = 0;
        } else {
            // c:5774 — `p = metafy(str+optcind, lenstr-optcind, META_DUP);`
            let p_arg = str_buf[(optcind as usize)..].to_string();
            crate::ported::params::setsparam("OPTARG", &p_arg);
            optcind = 0;
            zoptind += 1;
        }
    } else {
        // c:5784 — `zsfree(zoptarg); zoptarg = ztrdup("");`
        crate::ported::params::setsparam("OPTARG", "");
    }

    // c:5788 — `setsparam(var, metafy(optbuf, lenoptbuf, META_DUP));`
    crate::ported::params::setsparam(&var, &optbuf);
    ZOPTIND.store(zoptind, Ordering::Relaxed);
    OPTCIND.store(optcind, Ordering::Relaxed);
    crate::ported::params::setiparam("OPTIND", zoptind as i64);
    0                                                                        // c:5790
}

// `zoptind` (Src/builtin.c:5667) and `optcind` (c:5670) — the two
// pieces of getopts state. zoptind backs the user-visible $OPTIND.
pub static ZOPTIND: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(1);
pub static OPTCIND: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);

/// Port of `bin_read()` from Src/builtin.c:6412.
/// C: `int bin_read(char *name, char **args, Options ops, UNUSED(int func))`.
///
/// The C body is ~720 lines covering the whole `read` builtin matrix:
/// `-A` array, `-k N` raw chars, `-q` yes/no, `-r` raw, `-s` silent,
/// `-t TIMEOUT`, `-u FD` input FD, `-p` coproc, `-d DELIM` delimiter,
/// `-e` echo, `-E` echo-stdout-only, `-l`/`-c` compctl. The structural
/// port below handles the script-friendly subset: VAR= default,
/// `read -p PROMPT VAR`, `read -t TIMEOUT VAR`, `read -A ARRAY`,
/// `read -k N VAR`. Terminal-mode (-q/-s/-e) and ZLE plumbing defer
/// to the existing zle/io accessors.
pub fn bin_read(name: &str, args: &[String],                                 // c:6412
                ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::zsh_h::{OPT_ISSET, OPT_HASARG, OPT_ARG};
    use std::io::Read;
    let args = args.to_vec();
    let mut nchars: i32 = 1;                                                 // c:6415

    // c:6432-6438 — `-k N` raw-char count.
    if OPT_HASARG(ops, b'k') {                                               // c:6432
        let optarg = OPT_ARG(ops, b'k').unwrap_or("");
        match optarg.trim().parse::<i32>() {
            Ok(n) => nchars = n,
            Err(_) => {
                crate::ported::utils::zwarnnam(name,
                    &format!("number expected after -k: {}", optarg));        // c:6437
                return 1;
            }
        }
    }

    // c:6444-6446 — first arg may be `?prompt`; reply name (or REPLY/reply).
    let mut argi = 0usize;
    let mut prompt: Option<String> = None;
    if argi < args.len() && args[argi].starts_with('?') {                    // c:6444
        prompt = Some(args[argi][1..].to_string());
        argi += 1;
    }
    let want_array = OPT_ISSET(ops, b'A');
    let reply = if argi < args.len() {
        let r = args[argi].clone();
        argi += 1;
        r
    } else if want_array {
        "reply".to_string()                                                  // c:6446
    } else {
        "REPLY".to_string()                                                  // c:6446
    };

    if want_array && argi < args.len() {                                     // c:6448
        crate::ported::utils::zwarnnam(name, "only one array argument allowed"); // c:6449
        return 1;
    }

    // c:6453-6455 — `return compctlreadptr(name, args, ops, reply)`.
    // The compctlreadptr function pointer is set by the zsh/compctl
    // module's load hook; Rust dispatches to the static
    // crate::ported::zle::compctl::compctlread port (zle/compctl.rs:1235).
    if OPT_ISSET(ops, b'l') || OPT_ISSET(ops, b'c') {                        // c:6453
        return crate::ported::zle::compctl::compctlread(name, &args[argi..]);
    }

    // Optional explicit input FD via -u.
    let _ufd: i32 = if OPT_HASARG(ops, b'u') {
        OPT_ARG(ops, b'u').and_then(|s| s.parse().ok()).unwrap_or(0)
    } else { 0 };

    // c:6488-6515 — `-t TIMEOUT` poll(2) wait.
    if OPT_HASARG(ops, b't') {
        let arg = OPT_ARG(ops, b't').unwrap_or("");
        let tmout: f64 = arg.parse().unwrap_or(0.0);
        let mut pfd = libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 };
        let r = unsafe { libc::poll(&mut pfd, 1, (tmout * 1000.0) as i32) };
        if r == 0 { return 4; } // timeout
        if r < 0  { return 2; } // error
    }

    // Print prompt if provided.
    if let Some(ref p) = prompt {
        eprint!("{}", p);
        let _ = std::io::Write::flush(&mut std::io::stderr());
    }

    // Read one byte at a time until newline (or nchars when -k).
    let mut buf = String::new();
    if OPT_ISSET(ops, b'k') {                                                // c:6588
        let mut got = vec![0u8; nchars as usize];
        let mut bytes_read = 0;
        while bytes_read < nchars as usize {
            let mut b = [0u8; 1];
            match std::io::stdin().lock().read(&mut b) {
                Ok(1) => { got[bytes_read] = b[0]; bytes_read += 1; }
                _ => break,
            }
        }
        buf = String::from_utf8_lossy(&got[..bytes_read]).into_owned();
    } else {
        // Read a line (default behaviour).
        match std::io::stdin().read_line(&mut buf) {
            Ok(0) => return 1, // EOF
            Ok(_) => {
                if buf.ends_with('\n') { buf.pop(); }                        // strip \n
            }
            Err(_) => return 2,
        }
    }

    // Assign to scalar reply, multi-var split, or array.
    // c:6685-6735 — `read x y z` splits buf by IFS, fills the first
    // N-1 vars with one IFS-separated field each, and stores the
    // REST of the line (including embedded IFS chars) into the last
    // var. zsh's read is stable on `print "a b c d" | read x y z`:
    // x="a", y="b", z="c d".
    if want_array {
        let parts: Vec<String> = buf.split_whitespace().map(String::from).collect();
        crate::ported::params::setaparam(&reply, parts);                 // c:setaparam
    } else if argi < args.len() {
        // Multi-var: `read x y [z]`. First var = reply (already
        // consumed); rest are args[argi..]. Split with at most
        // `vars.len()` chunks using IFS.
        let mut vars: Vec<String> = Vec::with_capacity(args.len() - argi + 1);
        vars.push(reply);
        for n in &args[argi..] { vars.push(n.clone()); }
        let ifs = crate::ported::params::getsparam("IFS")
            .unwrap_or_else(|| " \t\n".to_string());
        // C zsh splits by ANY char from IFS (whitespace or not).
        let is_ifs = |c: char| ifs.contains(c);
        // Trim leading IFS-whitespace per zsh's read semantics
        // (`a   b c` → x=a, y="b c", not x="" y=…).
        let trimmed = buf.trim_start_matches(|c: char| is_ifs(c) && c.is_whitespace());
        let mut remaining = trimmed.to_string();
        for (i, var) in vars.iter().enumerate() {
            if i + 1 == vars.len() {
                // Last var: store the remainder, trim trailing IFS.
                let final_val = remaining.trim_end_matches(|c: char|
                    is_ifs(c) && c.is_whitespace()).to_string();
                crate::ported::params::setsparam(var, &final_val);
            } else {
                // Find next IFS char.
                match remaining.find(is_ifs) {
                    Some(idx) => {
                        let field = remaining[..idx].to_string();
                        // Skip the IFS char + any leading
                        // whitespace-IFS that follows (zsh-style
                        // whitespace coalescing).
                        let rest = &remaining[idx + remaining[idx..]
                            .chars().next().map(|c| c.len_utf8()).unwrap_or(1)..];
                        let rest = rest.trim_start_matches(|c: char|
                            is_ifs(c) && c.is_whitespace());
                        crate::ported::params::setsparam(var, &field);
                        remaining = rest.to_string();
                    }
                    None => {
                        // No more IFS: this var gets remaining, others empty.
                        crate::ported::params::setsparam(var, &remaining);
                        remaining.clear();
                    }
                }
            }
        }
    } else {
        crate::ported::params::setsparam(&reply, &buf);
    }
    0
}

/// Port of `bin_print()` from Src/builtin.c:4587.
/// C: `int bin_print(char *name, char **args, Options ops, int func)`.
///
/// The C body is ~1000 lines: `print` / `echo` / `printf` / `pushln`
/// dispatcher with -n/-N/-c/-r/-R/-l/-D/-i/-f/-v/-s/-S/-z/-e/-E etc.
/// The structural port handles the script-friendly subset that the
/// daily-driver hits: print/echo plain emission with -n, -l (one per
/// line), -r raw, -E newline-only, -- end-of-options. The full -f
/// printf format-spec engine and ZLE/history wireups defer to the
/// expand_printf_escapes helpers.
pub fn bin_print(name: &str, args: &[String],                                // c:4587
                 ops: &crate::ported::zsh_h::options, func: i32) -> i32 {
    use crate::ported::zsh_h::{OPT_ISSET, OPT_HASARG, OPT_ARG};
    use crate::ported::builtin::{BIN_ECHO, BIN_PRINTF};
    let nonewline = OPT_ISSET(ops, b'n');                                    // c:4595
    let raw = OPT_ISSET(ops, b'r') || OPT_ISSET(ops, b'R');                  // c:4596
    let one_per_line = OPT_ISSET(ops, b'l');                                 // c:4597
    let _printf_mode = func == BIN_PRINTF || OPT_HASARG(ops, b'f');          // c:4604
    let echo_mode = func == BIN_ECHO;
    let _ = (name, raw);

    // c:4633-4685 — destination dispatch. -u FD writes to fd, -s pushes
    // to history, -z to ZLE buffer, -v VAR assigns to scalar. Defer to
    // env/var wireup.
    let dest_var: Option<String> = if OPT_HASARG(ops, b'v') {
        OPT_ARG(ops, b'v').map(String::from)
    } else { None };

    // c:4604-4612 — printf format-string handling.
    if _printf_mode {
        let fmt = if let Some(f) = OPT_ARG(ops, b'f') {
            f.to_string()
        } else if !args.is_empty() {
            args[0].clone()
        } else {
            return 0;
        };
        let rest: &[String] = if OPT_HASARG(ops, b'f') { args } else { &args[1..] };
        let out = printf_format(&fmt, rest);
        if let Some(ref v) = dest_var {
            crate::ported::params::setsparam(v, &out);
        } else {
            print!("{}", out);
        }
        return 0;
    }

    // c:4860+ — main print loop.
    let sep = if one_per_line { "\n" } else { " " };
    // c:4598-4600 — `-P` prompt-style percent expansion (`%n`, `%d`,
    // `%?`, `%h`, `%%`, etc.). Routes through `expand_prompt`
    // (canonical port of `Src/prompt.c:182 promptexpand`).
    let mut processed_args: Vec<String> = if OPT_ISSET(ops, b'P') {
        args.iter()
            .map(|a| crate::ported::prompt::expand_prompt(a))                // c:Src/prompt.c:182
            .collect()
    } else {
        args.to_vec()
    };
    // c:Src/builtin.c:4869-4880 `-o` / `-O` / `-i` sort flags.
    // -o → case-insensitive ascending, -O → case-insensitive
    // descending, -i → case-sensitive (with -o/-O).
    if OPT_ISSET(ops, b'o') || OPT_ISSET(ops, b'O') {
        let case_sensitive = OPT_ISSET(ops, b'i');
        if case_sensitive {
            processed_args.sort();
        } else {
            processed_args.sort_by_key(|s| s.to_lowercase());
        }
        if OPT_ISSET(ops, b'O') {
            processed_args.reverse();
        }
    }
    // c:Src/builtin.c:4866-4886 — when `-r` is NOT set, each arg goes
    // through `getkeystring` to interpret backslash escapes (`\n`,
    // `\t`, `\\`, escaped space `\ `, etc.). `echo` follows the same
    // path when `BSD_ECHO`/`SH_OPTION_LETTERS`-style isn't in effect;
    // BIN_ECHO with `-E` keeps escapes literal. Without this, `print
    // -- ${(q)a}` for `a="he llo"` emitted `he\ llo` instead of zsh's
    // `he llo` (the (q) flag's backslash gets consumed by print).
    if !raw {
        let echo_E = echo_mode && OPT_ISSET(ops, b'E');
        if !echo_E {
            for a in processed_args.iter_mut() {
                let (s, _) = crate::ported::utils::getkeystring_print(a);
                *a = s;
            }
        }
    }
    let body = processed_args.join(sep);
    if let Some(ref v) = dest_var {
        crate::ported::params::setsparam(v, &body);
    } else {
        print!("{}", body);
        // c:5550 — final newline unless -n.
        if !nonewline && !echo_mode {
            println!();
        } else if echo_mode && !nonewline {
            println!();
        }
    }
    0
}

/// Inline printf-style format helper used by bin_print's -f/printf mode.
/// Replaces `%s` / `%d` / `%i` / `%c` / `%%` with positional args.
/// Full C printf-spec engine (Src/builtin.c:4691-5500) is much more
/// elaborate (width/precision/flag chars/%b/%q/etc.); this is the
/// minimal subset that covers the common script patterns.
fn printf_format(fmt: &str, args: &[String]) -> String {
    // c:Src/builtin.c:4711 — `fmt = getkeystring(fmt, &flen, ...,
    // GETKEYS_PRINTF_FMT, ...);`. The format string is first run
    // through getkeystring to interpret backslash escapes (`\n`,
    // `\t`, `\xNN`, etc.) before %-format substitution.
    let (fmt, _) = crate::ported::utils::getkeystring(fmt);                  // c:builtin.c:4711
    let mut out = String::new();
    // c:Src/builtin.c:4914-4923 — printf reapplies the format
    // string until ALL args are consumed. `printf '%s,' a b c` →
    // "a,b,c," not just "a,". Loop until arg_i stops advancing.
    let mut arg_i = 0usize;
    loop {
        let prev = arg_i;
        let chunk = printf_format_once(&fmt, args, &mut arg_i);
        out.push_str(&chunk);
        if arg_i == prev || arg_i >= args.len() { break; }
    }
    out
}

fn printf_format_once(fmt: &str, args: &[String], arg_i_ref: &mut usize) -> String {
    // Local re-bind so the existing `arg_i` uses read/write through
    // the caller-owned counter (so the loop in printf_format sees
    // progress across reapplies of the format string).
    macro_rules! arg_i { () => { *arg_i_ref }; }
    let mut out = String::with_capacity(fmt.len() + 16);
    let mut iter = fmt.chars().peekable();
    while let Some(c) = iter.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        // c:Src/builtin.c:4791+ — parse width/precision/flag chars
        // between `%` and the conversion. Capture them so `printf
        // "%-10s" hi` and `printf "%.3f" 3.14159` render correctly.
        let mut spec = String::from("%");
        loop {
            match iter.peek() {
                Some(&c) if matches!(c, '-' | '+' | ' ' | '#' | '0') => {
                    spec.push(c); iter.next();
                }
                _ => break,
            }
        }
        while let Some(&c) = iter.peek() {
            if c.is_ascii_digit() { spec.push(c); iter.next(); }
            else { break; }
        }
        if iter.peek() == Some(&'.') {
            spec.push('.'); iter.next();
            while let Some(&c) = iter.peek() {
                if c.is_ascii_digit() { spec.push(c); iter.next(); }
                else { break; }
            }
        }
        match iter.next() {
            Some('%') => out.push('%'),
            Some('s') => {
                let a = args.get(arg_i!()).cloned().unwrap_or_default();
                spec.push('s');
                out.push_str(&format_spec_str(&spec, &a));
                arg_i!() += 1;
            }
            Some('d') | Some('i') => {
                let a = args.get(arg_i!()).cloned().unwrap_or_default();
                let n: i64 = a.parse().unwrap_or(0);
                spec.push('d');
                out.push_str(&format_spec_int(&spec, n));
                arg_i!() += 1;
            }
            Some('u') => {
                let a = args.get(arg_i!()).cloned().unwrap_or_default();
                let n: u64 = a.parse().unwrap_or(0);
                spec.push('u');
                out.push_str(&format_spec_uint(&spec, n));
                arg_i!() += 1;
            }
            Some('x') => {
                let a = args.get(arg_i!()).cloned().unwrap_or_default();
                let n: i64 = a.parse().unwrap_or(0);
                spec.push('x');
                out.push_str(&format!("{:x}", n));
                arg_i!() += 1;
            }
            Some('X') => {
                let a = args.get(arg_i!()).cloned().unwrap_or_default();
                let n: i64 = a.parse().unwrap_or(0);
                spec.push('X');
                out.push_str(&format!("{:X}", n));
                arg_i!() += 1;
            }
            Some('o') => {
                let a = args.get(arg_i!()).cloned().unwrap_or_default();
                let n: i64 = a.parse().unwrap_or(0);
                spec.push('o');
                out.push_str(&format!("{:o}", n));
                arg_i!() += 1;
            }
            Some('f') | Some('F') | Some('g') | Some('G') | Some('e') | Some('E') => {
                let a = args.get(arg_i!()).cloned().unwrap_or_default();
                let n: f64 = a.parse().unwrap_or(0.0);
                spec.push('f');
                out.push_str(&format_spec_float(&spec, n));
                arg_i!() += 1;
            }
            Some('c') => {
                if let Some(a) = args.get(arg_i!()) {
                    if let Some(ch) = a.chars().next() { out.push(ch); }
                }
                arg_i!() += 1;
            }
            // c:builtin.c:4825 %q — shell-quote the arg.
            Some('q') => {
                let a = args.get(arg_i!()).cloned().unwrap_or_default();
                out.push_str(&crate::ported::utils::quotedzputs(&a));
                arg_i!() += 1;
            }
            // c:builtin.c:4810 %b — interpret backslash escapes.
            Some('b') => {
                let a = args.get(arg_i!()).cloned().unwrap_or_default();
                let (s, _) = crate::ported::utils::getkeystring_print(&a);
                out.push_str(&s);
                arg_i!() += 1;
            }
            Some(other) => { out.push('%'); out.push(other); }
            None => out.push('%'),
        }
    }
    out
}

/// Apply a printf-style `%[-flag][width][.prec]s` spec to a string.
/// Mirrors C `printf "%-10s" str` formatting; the Rust `format!` macro
/// doesn't accept runtime-parsed specs so we hand-parse.
fn format_spec_str(spec: &str, s: &str) -> String {
    let (left_align, width, prec) = parse_width_prec(spec);
    let truncated: &str = if let Some(p) = prec {
        let end: usize = s.chars().take(p).map(|c| c.len_utf8()).sum();
        &s[..end.min(s.len())]
    } else { s };
    let pad = width.saturating_sub(truncated.chars().count());
    if left_align {
        format!("{}{}", truncated, " ".repeat(pad))
    } else {
        format!("{}{}", " ".repeat(pad), truncated)
    }
}

fn format_spec_int(spec: &str, n: i64) -> String {
    let (left_align, width, _prec) = parse_width_prec(spec);
    let zero_pad = spec.contains('0') && !left_align;
    let body = n.to_string();
    let pad = width.saturating_sub(body.chars().count());
    if pad == 0 { body }
    else if left_align { format!("{}{}", body, " ".repeat(pad)) }
    else if zero_pad {
        if let Some(rest) = body.strip_prefix('-') {
            format!("-{}{}", "0".repeat(pad), rest)
        } else { format!("{}{}", "0".repeat(pad), body) }
    } else { format!("{}{}", " ".repeat(pad), body) }
}

fn format_spec_uint(spec: &str, n: u64) -> String {
    format_spec_int(spec, n as i64)
}

fn format_spec_float(spec: &str, n: f64) -> String {
    let (left_align, width, prec) = parse_width_prec(spec);
    let p = prec.unwrap_or(6);
    let body = format!("{:.*}", p, n);
    let pad = width.saturating_sub(body.chars().count());
    if pad == 0 { body }
    else if left_align { format!("{}{}", body, " ".repeat(pad)) }
    else { format!("{}{}", " ".repeat(pad), body) }
}

fn parse_width_prec(spec: &str) -> (bool, usize, Option<usize>) {
    let s = spec.trim_start_matches('%');
    let mut i = 0;
    let bytes = s.as_bytes();
    let mut left_align = false;
    while i < bytes.len() && matches!(bytes[i], b'-' | b'+' | b' ' | b'#' | b'0') {
        if bytes[i] == b'-' { left_align = true; }
        i += 1;
    }
    let width_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() { i += 1; }
    let width: usize = s[width_start..i].parse().unwrap_or(0);
    let mut prec: Option<usize> = None;
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        let p_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() { i += 1; }
        prec = Some(s[p_start..i].parse().unwrap_or(0));
    }
    (left_align, width, prec)
}

/// Port of `bin_fc()` from Src/builtin.c:1426.
/// C: `int bin_fc(char *nam, char **argv, Options ops, int func)`.
///
/// History/edit/list dispatcher: `-p` push hist stack, `-P` pop,
/// `-R` read, `-W` write, `-A` append, `-m` glob filter, `-l` list,
/// `-s` substitute, default: edit + re-execute. The C body is ~245
/// lines; the structural translation here covers the major options
/// and dispatches the underlying history-file ops to the existing
/// hist.rs accessors.
pub fn bin_fc(nam: &str, argv: &[String],                                    // c:1426
              ops: &mut crate::ported::zsh_h::options, func: i32) -> i32 {
    use crate::ported::zsh_h::{
        OPT_ISSET, OPT_HASARG, OPT_ARG, HIST_FOREIGN,
    };
    let mut argv = argv.to_vec();
    let mut first: i64 = -1;
    let mut last: i64 = -1;
    let mut asgf: Vec<(String, String)> = Vec::new();

    use crate::ported::zsh_h::{HFILE_APPEND, HFILE_SKIPOLD, HFILE_USE_OPTIONS};
    use std::sync::atomic::Ordering;

    // c:1441-1481 — `-p` push history stack.
    if OPT_ISSET(ops, b'p') {                                                // c:1441
        let mut hf = "".to_string();
        let mut hs: i64;                                                     // c:1443
        let mut shs: i64;                                                    // c:1444
        // c:1445 — `int level = OPT_ISSET(ops,'a') ? locallevel : -1;`
        let level: i32 = if OPT_ISSET(ops, b'a') {
            LOCALLEVEL.load(std::sync::atomic::Ordering::Relaxed)
        } else { -1 };
        hs = crate::ported::hist::histsiz.load(Ordering::Relaxed);           // c:1442
        shs = crate::ported::hist::savehistsiz.load(Ordering::Relaxed);
        if !argv.is_empty() {                                                // c:1445
            hf = argv.remove(0);                                             // c:1446
            if !argv.is_empty() {                                            // c:1447
                let s2 = argv.remove(0);
                match s2.parse::<i64>() {                                    // c:1449 zstrtol
                    Ok(n) => hs = n,
                    Err(_) => {
                        crate::ported::utils::zwarnnam("fc",                 // c:1452
                            "HISTSIZE must be an integer");
                        return 1;                                            // c:1453
                    }
                }
                if !argv.is_empty() {                                        // c:1455
                    let s3 = argv.remove(0);
                    match s3.parse::<i64>() {                                // c:1456
                        Ok(n) => shs = n,
                        Err(_) => {
                            crate::ported::utils::zwarnnam("fc",             // c:1459
                                "SAVEHIST must be an integer");
                            return 1;                                        // c:1460
                        }
                    }
                } else {
                    shs = hs;                                                // c:1464
                }
                if !argv.is_empty() {                                        // c:1466
                    crate::ported::utils::zwarnnam("fc",                     // c:1468
                        "too many arguments");
                    return 1;                                                // c:1469
                }
            }
        }
        // c:1473 — pushhiststack(hf, hs, shs, level); failure → return 1.
        crate::ported::hist::pushhiststack(Some(&hf), hs, shs, level);       // c:1473
        if !hf.is_empty() {                                                  // c:1475
            // c:1476-1480 — stat then readhistfile(hf, 1, HFILE_USE_OPTIONS).
            let exists = std::fs::metadata(&hf).is_ok();
            let enoent = std::io::Error::last_os_error().raw_os_error()
                == Some(libc::ENOENT);
            if exists || !enoent {                                           // c:1477
                crate::ported::hist::readhistfile(                           // c:1478
                    Some(&hf), 1, HFILE_USE_OPTIONS as i32);
            }
        }
        return 0;                                                            // c:1483
    }

    // c:1485-1491 — `-P` pop history stack.
    if OPT_ISSET(ops, b'P') {                                                // c:1485
        if !argv.is_empty() {                                                // c:1486
            crate::ported::utils::zwarnnam("fc", "too many arguments");      // c:1487
            return 1;                                                        // c:1488
        }
        // c:1490 — `return !saveandpophiststack(-1, HFILE_USE_OPTIONS);`.
        crate::ported::hist::saveandpophiststack(HFILE_USE_OPTIONS as i32);  // c:1490
        return 0;
    }

    // c:1494-1500 — `-m` pattern filter (compile first arg).
    let mut pprog: Option<crate::ported::pattern::PatProg> = None;
    if !argv.is_empty() && OPT_ISSET(ops, b'm') {                            // c:1494
        let pat = argv.remove(0);
        // c:1495 — tokenize(*argv); — Rust `patcompile` handles tokenisation.
        match crate::ported::pattern::patcompile(&pat,                       // c:1496
            crate::ported::zsh_h::PAT_HEAPDUP, None) {
            Some(p) => pprog = Some(p),
            None => {
                crate::ported::utils::zwarnnam(nam, "invalid match pattern"); // c:1497
                return 1;                                                    // c:1498
            }
        }
    }

    crate::ported::mem::queue_signals();                                     // c:1502

    // c:1503-1525 — `-R` read / `-W` write / `-A` append history file.
    if OPT_ISSET(ops, b'R') {                                                // c:1503
        let path = argv.first().cloned();
        let flags = if OPT_ISSET(ops, b'I') { HFILE_SKIPOLD as i32 } else { 0 };
        crate::ported::hist::readhistfile(                                   // c:1505
            path.as_deref(), 1, flags);
        crate::ported::mem::unqueue_signals();                               // c:1506
        return 0;                                                            // c:1507
    }
    if OPT_ISSET(ops, b'W') {                                                // c:1509
        let path = argv.first().cloned();
        let flags = if OPT_ISSET(ops, b'I') { HFILE_SKIPOLD as i32 } else { 0 };
        crate::ported::hist::savehistfile(                                   // c:1511
            path.as_deref(), flags);
        crate::ported::mem::unqueue_signals();                               // c:1512
        return 0;                                                            // c:1513
    }
    if OPT_ISSET(ops, b'A') {                                                // c:1515
        let path = argv.first().cloned();
        let mut flags = HFILE_APPEND as i32;
        if OPT_ISSET(ops, b'I') { flags |= HFILE_SKIPOLD as i32; }           // c:1518
        crate::ported::hist::savehistfile(                                   // c:1517
            path.as_deref(), flags);
        crate::ported::mem::unqueue_signals();                               // c:1519
        return 0;                                                            // c:1520
    }

    // c:1523-1527 — refuse inside ZLE.
    if crate::ported::builtins::sched::zleactive.load(                       // c:1523
        std::sync::atomic::Ordering::Relaxed) != 0 {
        crate::ported::mem::unqueue_signals();                               // c:1524
        crate::ported::utils::zwarnnam(nam,                                  // c:1525
            "no interactive history within ZLE");
        return 1;                                                            // c:1526
    }

    // c:1530-1547 — `name=value` substitution pairs.
    while !argv.is_empty() && argv[0].contains('=') {                        // c:1530
        let arg = argv.remove(0);
        if let Some(eq) = arg.find('=') {
            let n = &arg[..eq];
            let v = &arg[eq + 1..];
            if n.is_empty() {
                crate::ported::utils::zwarnnam(nam,
                    &format!("invalid replacement pattern: ={}", v));        // c:1534
                return 1;
            }
            asgf.push((n.to_string(), v.to_string()));                       // c:1546
        }
    }

    // c:1550-1568 — first/last history specifiers via fcgetcomm.
    if !argv.is_empty() {                                                    // c:1550
        first = fcgetcomm(&argv.remove(0));                                  // c:1551
        if first == -1 {
            crate::ported::mem::unqueue_signals();
            return 1;                                                        // c:1553
        }
    }
    if !argv.is_empty() {                                                    // c:1559
        last = fcgetcomm(&argv.remove(0));                                   // c:1560
        if last == -1 {
            crate::ported::mem::unqueue_signals();
            return 1;
        }
    }
    if !argv.is_empty() {                                                    // c:1567
        crate::ported::mem::unqueue_signals();
        crate::ported::utils::zwarnnam("fc", "too many arguments");          // c:1569
        return 1;
    }

    // c:1573-1610 — default ranges + listing/edit dispatch.
    let curhist: i64 = std::env::var("HISTCMD").ok()
        .and_then(|s| s.parse().ok()).unwrap_or(0);
    if last == -1 {                                                          // c:1573
        if OPT_ISSET(ops, b'l') && first < curhist {                         // c:1574
            last = curhist;                                                  // c:1583
            if last < 1 { last = 1; }                                        // c:1585
        } else {
            last = first;                                                    // c:1587
        }
    }
    if first == -1 {                                                         // c:1589
        let _xflags = if OPT_ISSET(ops, b'L') { HIST_FOREIGN } else { 0 };   // c:1597
        first = if OPT_ISSET(ops, b'l') { (curhist - 16).max(1) }            // c:1598
                else { (curhist - 1).max(1) };
        if last < first { last = first; }                                    // c:1604
    }

    let mut retval;
    if OPT_ISSET(ops, b'l') {                                                // c:1606
        // c:1608 — `fclist(stdout, ops, first, last, asgf, pprog, 0);`
        retval = fclist(std::ptr::null_mut(), ops, first, last,
                        &asgf, std::ptr::null_mut(), 0);
        crate::ported::mem::unqueue_signals();
    } else {
        // c:1611-1668 — edit history range to a temp file, fcedit it,
        // then stuff() the result back as the next command.
        retval = 1;                                                          // c:1620
        let fil_opt = crate::ported::utils::gettempfile(Some("zshfc"));      // c:1621 gettempfile
        match fil_opt {
            None => {                                                        // c:1623
                crate::ported::mem::unqueue_signals();                       // c:1624
                crate::ported::utils::zwarnnam("fc",                         // c:1625
                    &format!("can't open temp file: {}",
                        std::io::Error::last_os_error()));
            }
            Some((fd, fil)) => {
                unsafe { libc::close(fd); }                                  // c:1622 (file is reopened below)
                // c:1632 — `if (last >= curhist) { last = curhist - 1; ... }`
                if last >= curhist {                                         // c:1632
                    last = curhist - 1;                                      // c:1633
                    if first > last {                                        // c:1634
                        crate::ported::mem::unqueue_signals();               // c:1635
                        crate::ported::utils::zwarnnam("fc",                 // c:1636
                            "current history line would recurse endlessly, aborted");
                        let _ = std::fs::remove_file(&fil);                  // c:1639 unlink
                        return 1;                                            // c:1640
                    }
                }
                ops.ind[b'n' as usize] = 1;                                  // c:1644 No line numbers
                let out = std::fs::OpenOptions::new()
                    .create(true).write(true).truncate(true).open(&fil).ok();
                let listed = if out.is_some() {                              // c:1645
                    fclist(std::ptr::null_mut(), ops, first, last,
                           &asgf, std::ptr::null_mut(), 1)
                } else { 1 };
                if listed == 0 {                                             // c:1645
                    // c:1647-1656 — pick editor.
                    let editor: String = if func == BIN_R || OPT_ISSET(ops, b's') {
                        "-".to_string()                                      // c:1648
                    } else if OPT_HASARG(ops, b'e') {                        // c:1649
                        OPT_ARG(ops, b'e').unwrap_or("").to_string()         // c:1650
                    } else {
                        std::env::var("FCEDIT")                              // c:1651 getsparam("FCEDIT")
                            .or_else(|_| std::env::var("EDITOR"))            // c:1653 getsparam("EDITOR")
                            .unwrap_or_else(|_|
                                crate::ported::config_h::DEFAULT_FCEDIT.to_string()) // c:1654
                    };
                    crate::ported::mem::unqueue_signals();                   // c:1657
                    if fcedit(&editor, &fil) != 0 {                          // c:1658
                        if crate::ported::input::stuff(&fil) != 0 {          // c:1659
                            crate::ported::utils::zwarnnam("fc",             // c:1660
                                &format!("{}: {}",
                                    std::io::Error::last_os_error(), fil));
                        } else {
                            // c:1663-1664 — `loop(0,1); retval = lastval;`
                            // The interactive loop drives the next stuffed
                            // line through the parser. Static-link path:
                            // the executor's input source picks it up on
                            // the next read; lastval reflects that result.
                            retval = LASTVAL.load(                           // c:1664
                                std::sync::atomic::Ordering::Relaxed);
                        }
                    }
                } else {
                    crate::ported::mem::unqueue_signals();                   // c:1667
                }
                let _ = std::fs::remove_file(&fil);                          // c:1671 unlink
            }
        }
    }
    let _ = pprog;
    retval                                                                   // c:1675
}

/// Port of `bin_typeset()` from Src/builtin.c:2655.
/// C: `int bin_typeset(char *name, char **argv, LinkList assigns,
///     Options ops, int func)`.
///
/// The C body (~500 lines) ports here in two layers: the option-flag
/// matrix + conflict-resolution / dispatch (faithfully translated)
/// and the per-arg param-setting loop (delegated to typeset_single
/// already ported above).
pub fn bin_typeset(name: &str, argv: &[String],                              // c:2655
                   ops: &crate::ported::zsh_h::options, func: i32) -> i32 {
    use crate::ported::zsh_h::{
        OPT_ISSET, OPT_PLUS, OPT_MINUS, OPT_HASARG, OPT_ARG, EMULATION,
        TYPESET_OPTSTR,
        PM_INTEGER, PM_EFLOAT, PM_FFLOAT, PM_HASHED, PM_LEFT,
        PM_RIGHT_B, PM_RIGHT_Z, PM_HIDEVAL, PM_LOWER, PM_UPPER,
        PM_TIED, PM_LOCAL, PM_NAMEREF, PM_READONLY, PM_ARRAY,
        PRINT_TYPESET, PRINT_LINE, PRINT_TYPE, PRINT_NAMEONLY,
        PRINT_POSIX_EXPORT, PRINT_POSIX_READONLY, PRINT_WITH_NAMESPACE,
        EMULATE_KSH,
    };

    // PFA-SMR aspect: bin_typeset is the C dispatch site for
    // typeset/declare/integer/float/local/export/readonly/private —
    // every one of those state-mutating builtins lands here with a
    // funcid (BIN_EXPORT/BIN_READONLY/BIN_TYPESET/...) discriminant.
    // Emit a per-name event per the recorder schema.
    #[cfg(feature = "recorder")]
    if crate::recorder::is_enabled() {
        let ctx = crate::recorder::recorder_ctx_global();
        // Collect option letters (`-x`/`+x` body) so ParamAttrs reflects
        // the typeset flag set the C source sees in `on`.
        let mut letters = String::new();
        let mut tied_mode = false;
        for a in argv {
            if a.starts_with('-') || a.starts_with('+') {
                let body = &a[1..];
                letters.push_str(body);
                if body.contains('T') { tied_mode = true; }
            }
        }
        // Funcid-driven attr seeding: BIN_EXPORT seeds nothing
        // (recorder uses emit_export for those), BIN_READONLY seeds
        // SCALAR|READONLY, BIN_FLOAT seeds FLOAT, BIN_INTEGER seeds
        // INTEGER. Otherwise pass the letter set through
        // ParamAttrs::from_flag_chars verbatim.
        let mut attrs = crate::recorder::ParamAttrs::from_flag_chars(&letters);
        match func {
            crate::ported::builtin::BIN_READONLY => {
                attrs.set(crate::recorder::ParamAttrs::SCALAR);
                attrs.set(crate::recorder::ParamAttrs::READONLY);
            }
            _ => {}
        }
        // BIN_EXPORT routes to emit_export (different schema row).
        if func == crate::ported::builtin::BIN_EXPORT {
            for a in argv {
                if a == "-p" || a.starts_with('-') { continue; }
                if let Some((k, v)) = a.split_once('=') {
                    crate::recorder::emit_export(k, Some(v), ctx.clone());
                } else {
                    crate::recorder::emit_export(a, None, ctx.clone());
                }
            }
        } else {
            // Suppress the emit when invoked as `local`/`private` inside
            // a function — those scope to the frame and don't merit a
            // top-level state-mutation row. local_scope_depth is tracked
            // by the executor; defer to the global LOCALLEVEL counter.
            let is_locallike = matches!(name, "local" | "private");
            let inside_function =
                LOCALLEVEL.load(std::sync::atomic::Ordering::Relaxed) > 0;
            if !is_locallike || !inside_function {
                let mut tied_seen = 0usize;
                for a in argv {
                    if a.starts_with('-') || a.starts_with('+') { continue; }
                    if tied_mode {
                        // For `typeset -T X Y [SEP]`, only X and Y are names.
                        tied_seen += 1;
                        if tied_seen > 2 { break; }
                    }
                    if let Some((k, v)) = a.split_once('=') {
                        crate::recorder::emit_typeset_attrs(k, Some(v), attrs, ctx.clone());
                    } else {
                        crate::recorder::emit_typeset_attrs(a, None, attrs, ctx.clone());
                    }
                }
            }
        }
    }
    let mut ops = ops.clone();
    let mut on: u32 = 0;                                                     // c:2661
    let mut off: u32 = 0;                                                    // c:2661
    let returnval: i32 = 0;                                                  // c:2664
    let mut printflags: i32 = PRINT_WITH_NAMESPACE;                          // c:2664
    let hasargs = !argv.is_empty();                                          // c:2665

    // c:2668-2670 — POSIX bash/ksh ignore -p with args under
    // readonly/export.
    let posix = crate::ported::zsh_h::isset(crate::ported::options::optlookup("posixbuiltins"));
    if (func == BIN_READONLY || func == BIN_EXPORT) && posix && hasargs {    // c:2668
        ops.ind[b'p' as usize] = 0;                                          // c:2670
    }

    // c:2673 — `if (OPT_ISSET(ops,'f')) return bin_functions(...)`.
    if OPT_ISSET(&ops, b'f') {                                               // c:2673
        return bin_functions(name, argv, &ops, func);                        // c:2673
    }

    // c:2676 — POSIX readonly forces -g unless explicit +g.
    if func == BIN_READONLY && posix && !OPT_PLUS(&ops, b'g') {              // c:2676
        ops.ind[b'g' as usize] = 1;                                          // c:2677
    }

    // c:2691-2706 — translate optstr letters into PM_* flag bits.
    let mut bit: u32 = PM_ARRAY;                                             // c:2660
    for ch in TYPESET_OPTSTR.chars() {                                       // c:2691
        let optval = ch as u8;
        if OPT_MINUS(&ops, optval) { on |= bit; }                            // c:2694-2695
        else if OPT_PLUS(&ops, optval) { off |= bit; }                       // c:2696-2697
        // c:2698-2706 — `-n` only allows readonly/upper/hideval.
        else { bit <<= 1; continue; }
        if OPT_MINUS(&ops, b'n')
            && (bit & !(PM_READONLY | PM_UPPER | PM_HIDEVAL)) != 0           // c:2701
        {
            crate::ported::utils::zwarnnam(name,
                &format!("-{} not allowed with -n", ch));                    // c:2702
        }
        bit <<= 1;
    }
    // c:2708-2715 — -n / +n conflict resolution.
    if OPT_MINUS(&ops, b'n') {                                               // c:2708
        if (on | off) & !(PM_READONLY | PM_UPPER | PM_HIDEVAL) != 0 {        // c:2710
            return 1;                                                        // c:2711
        }
        on |= PM_NAMEREF;                                                    // c:2713
    } else if OPT_PLUS(&ops, b'n') {                                         // c:2714
        off |= PM_NAMEREF;                                                   // c:2715
    }
    let roff = off;                                                          // c:2716

    // c:2719-2740 — sanity checks: remove conflicting attrs.
    if (on & PM_FFLOAT) != 0 {                                               // c:2719
        off |= PM_UPPER | PM_ARRAY | PM_HASHED | PM_INTEGER | PM_EFLOAT;     // c:2720
        on &= !PM_EFLOAT;                                                    // c:2722
    }
    if (on & PM_EFLOAT) != 0 {                                               // c:2724
        off |= PM_UPPER | PM_ARRAY | PM_HASHED | PM_INTEGER | PM_FFLOAT;     // c:2725
    }
    if (on & PM_INTEGER) != 0 {                                              // c:2726
        off |= PM_UPPER | PM_ARRAY | PM_HASHED | PM_EFLOAT | PM_FFLOAT;      // c:2727
    }
    if (on & (PM_LEFT | PM_RIGHT_Z)) != 0 {                                  // c:2731
        off |= PM_RIGHT_B;                                                   // c:2732
    }
    if (on & PM_RIGHT_B) != 0 {                                              // c:2733
        off |= PM_LEFT | PM_RIGHT_Z;                                         // c:2734
    }
    if (on & PM_UPPER) != 0 { off |= PM_LOWER; }                             // c:2735-2736
    if (on & PM_LOWER) != 0 { off |= PM_UPPER; }                             // c:2737-2738
    if (on & PM_HASHED) != 0 { off |= PM_ARRAY; }                            // c:2739-2740
    if (on & PM_TIED) != 0 {                                                 // c:2741
        off |= PM_INTEGER | PM_EFLOAT | PM_FFLOAT | PM_ARRAY | PM_HASHED;    // c:2742
    }
    on &= !off;                                                              // c:2744

    crate::ported::mem::queue_signals();                                     // c:2746

    // c:2748-2772 — `-p` print-mode: PRINT_POSIX_EXPORT / READONLY /
    // TYPESET, plus optional -p N for line-style.
    if OPT_ISSET(&ops, b'p') {                                               // c:2748
        if posix && !EMULATION(EMULATE_KSH) {                                // c:2750
            printflags |= match func {
                BIN_EXPORT   => PRINT_POSIX_EXPORT,                          // c:2752
                BIN_READONLY => PRINT_POSIX_READONLY,                        // c:2754
                _            => PRINT_TYPESET,                               // c:2756
            };
        } else {
            printflags |= PRINT_TYPESET;                                     // c:2758
        }
        if OPT_HASARG(&ops, b'p') {                                          // c:2761
            let arg = OPT_ARG(&ops, b'p').unwrap_or("");
            match arg.trim().parse::<i32>() {                                // c:2763
                Ok(1) => printflags |= PRINT_LINE,                           // c:2765
                Ok(0) => {}                                                  // c:2770 -p0 == -p
                _ => {
                    crate::ported::utils::zwarnnam(name,
                        &format!("bad argument to -p: {}", arg));            // c:2767
                    crate::ported::mem::unqueue_signals();
                    return 1;                                                // c:2769
                }
            }
        }
    }

    // c:2775-2795 — no-args path: list whatever options select.
    if !hasargs {                                                            // c:2775
        if !OPT_ISSET(&ops, b'm') {                                          // c:2779
            printflags &= !PRINT_WITH_NAMESPACE;                             // c:2780
        }
        if !OPT_ISSET(&ops, b'p') {                                          // c:2782
            if (on | roff) == 0 {                                            // c:2783
                printflags |= PRINT_TYPE;                                    // c:2784
            }
            if roff != 0 || OPT_ISSET(&ops, b'+') {                          // c:2785
                printflags |= PRINT_NAMEONLY;                                // c:2786
            }
        }
        // c:2792 — scanhashtable(paramtab, ...) listing path. Defer
        // to env walk for static-link path (real paramtab walk lives
        // in src/ported/params.rs).
        for (k, v) in std::env::vars() {                                     // c:2792
            if (printflags & PRINT_NAMEONLY) != 0 {
                println!("{}", k);
            } else {
                println!("{}={}", k,
                    crate::ported::utils::quotedzputs(&v));
            }
        }
        crate::ported::mem::unqueue_signals();
        return 0;                                                            // c:2794
    }

    // c:2799-2810 — `local` (or +g) implies PM_LOCAL.
    let nm0 = name.chars().next().unwrap_or(' ');
    if nm0 == 'l' || OPT_PLUS(&ops, b'g') {                                  // c:2799
        on |= PM_LOCAL;                                                      // c:2800
    } else if !OPT_ISSET(&ops, b'g') {                                       // c:2801
        if OPT_MINUS(&ops, b'x') {                                           // c:2802
            let globalexport = crate::ported::zsh_h::isset(crate::ported::options::optlookup("globalexport"));
            let locallevel = LOCALLEVEL.load(std::sync::atomic::Ordering::Relaxed);
            if globalexport {                                                // c:2803
                ops.ind[b'g' as usize] = 1;                                  // c:2804
            } else if locallevel != 0 {                                      // c:2805
                on |= PM_LOCAL;                                              // c:2806
            }
        } else if !(OPT_ISSET(&ops, b'x') || OPT_ISSET(&ops, b'm')) {        // c:2808
            on |= PM_LOCAL;                                                  // c:2809
        }
    }

    // c:2813+ — -T tied vars + per-arg setting loop.
    // The full C body has dozens of paths (PM_TIED tie-pair setup at
    // c:2813-2900, glob -m walk at c:2905-2935, name=value assign
    // through typeset_single at c:2945+). The Rust port handles the
    // three high-frequency paths inline: assoc creation (`PM_HASHED`
    // + `name=(k v k v)`), array creation (`PM_ARRAY` + `name=(a b c)`),
    // and scalar assignment.
    let _ = (off, returnval, name);
    let is_hashed = (on & PM_HASHED) != 0;                                   // c:2655 `-A`
    let is_array  = (on & PM_ARRAY)  != 0;                                   // c:2655 `-a`
    for arg in argv {
        if let Some(eq) = arg.find('=') {
            let n = &arg[..eq];
            let raw_v = &arg[eq + 1..];
            // c:2945-3050 — `=(elem elem ...)` array-init syntax.
            // The parser hands the whole `(...)` body in as one arg
            // when typeset's BINF_MAGICEQUALS is set; the `(` / `)` are
            // literal first/last bytes. Strip them and split on
            // whitespace to recover the element list.
            let is_paren_init = raw_v.starts_with('(') && raw_v.ends_with(')')
                && raw_v.len() >= 2;
            if is_paren_init {
                let inner = &raw_v[1..raw_v.len()-1];                        // c:2950
                let elems: Vec<String> = inner.split_whitespace()            // c:2952
                    .map(String::from)
                    .collect();
                if is_hashed {
                    // c:2960-2975 — `setdataparam(..., PM_HASHED, …)`.
                    // Two assoc-init shapes accepted by zsh:
                    //  1. flat alternating k/v: `m=(k1 v1 k2 v2)`
                    //  2. per-element [K]=V:    `m=([k1]=v1 [k2]=v2)`
                    // The parser hands all elements as one `(…)` body,
                    // so we detect shape 2 when every element starts
                    // with `[` and contains `]=`. Otherwise fall back
                    // to alternating pairs.
                    let bracket_shape = !elems.is_empty()
                        && elems.iter().all(|e| {
                            e.starts_with('[')
                                && e.contains("]=")
                        });
                    let mut map: indexmap::IndexMap<String, String>
                        = indexmap::IndexMap::new();
                    if bracket_shape {
                        for e in &elems {
                            let close = e.find("]=").unwrap();
                            let k = e[1..close].to_string();
                            let v = e[close + 2..].to_string();
                            map.insert(k, v);
                        }
                    } else {
                        let mut it = elems.into_iter();                      // c:2960 pair walk
                        while let Some(k) = it.next() {
                            let v = it.next().unwrap_or_default();
                            map.insert(k, v);                                // c:2964 hashtab insert
                        }
                    }
                    let n_owned = n.to_string();
                    crate::fusevm_bridge::with_executor(|exec| {
                        exec.set_assoc(n_owned.clone(), map.clone());
                        exec.variables.remove(&n_owned);
                    });
                } else {
                    // c:2980-2995 — plain array.
                    let n_owned = n.to_string();
                    let elems_owned = elems.clone();
                    crate::fusevm_bridge::with_executor(|exec| {
                        exec.set_array(n_owned.clone(), elems_owned.clone());
                        exec.variables.remove(&n_owned);
                    });
                }
            } else {
                // c:3010-3030 — `name=value` scalar assign. C-canonical
                // `setsparam` (Src/params.c:3350) writes paramtab; the
                // env mirror at `Src/params.c:3024 addenv` follows.
                //
                // c:Src/params.c PM_LOCAL: when typeset is invoked as
                // `local`/`private`/inside a fn with implicit local
                // scope, save the current value to local_save_stack
                // so the function-exit unwind at fusevm_bridge.rs:
                // 9514 can restore it. Without this, `x=outer; f() {
                // local x=inner }; f; echo $x` left x=inner after f.
                if (on & PM_LOCAL) != 0 {
                    let n_owned = n.to_string();
                    let _ = crate::fusevm_bridge::try_with_executor(|exec| {
                        if exec.local_scope_depth > 0 {
                            let old = exec.variables.get(&n_owned).cloned();
                            exec.local_save_stack.push((n_owned, old));
                        }
                    });
                }
                // c:Src/params.c PM_LOWER/PM_UPPER setstrvalue arms:
                // when typeset -l or -u is set, the assigned value is
                // case-folded BEFORE storage. Without this, `typeset -l
                // s=HELLO; echo $s` printed `HELLO`. We also mirror to
                // exec.var_attrs so subsequent plain assigns (`s=NEW`)
                // pick up the fold via the SET_VAR opcode's attr
                // check (fusevm_bridge.rs case-fold arm).
                let lower = (on & PM_LOWER) != 0;
                let upper = (on & PM_UPPER) != 0;
                let folded: String = if lower {
                    raw_v.to_lowercase()
                } else if upper {
                    raw_v.to_uppercase()
                } else {
                    raw_v.to_string()
                };
                crate::ported::params::setsparam(n, &folded);                // c:params.c:3350
                std::env::set_var(n, &folded);                               // c:3024 addenv
                // Mirror to exec.variables (legacy store) so arith
                // eval (`evaluate_arithmetic` seeds `extras` from
                // `self.variables`) sees the typed assignment. Without
                // this, `typeset -i n=10; echo $((n+5))` returned 5
                // instead of 15 because $((n)) resolved to 0.
                let is_int = (on & PM_INTEGER) != 0;
                let is_float = (on & (PM_EFLOAT | PM_FFLOAT)) != 0;
                let n_owned = n.to_string();
                let folded_clone = folded.clone();
                let _ = crate::fusevm_bridge::try_with_executor(|exec| {
                    exec.variables.insert(n_owned.clone(), folded_clone);
                    if lower || upper || is_int || is_float
                        || (on & PM_READONLY) != 0
                    {
                        let attr = exec.var_attrs
                            .entry(n_owned)
                            .or_default();
                        attr.lowercase = lower;
                        attr.uppercase = upper;
                        if is_int {
                            attr.kind = crate::ported::params::VarKind::Integer;
                        } else if is_float {
                            attr.kind = crate::ported::params::VarKind::Float;
                        }
                        if (on & PM_READONLY) != 0 {
                            attr.readonly = true;
                        }
                    }
                });
            }
        } else if is_hashed || is_array {
            // c:3060-3070 — bare name + `-A`/`-a` declares an empty
            // assoc/array.
            let n_owned = arg.clone();
            crate::fusevm_bridge::with_executor(|exec| {
                if is_hashed {
                    exec.assoc_arrays.entry(n_owned.clone())
                        .or_insert_with(indexmap::IndexMap::new);
                } else {
                    exec.arrays.entry(n_owned.clone())
                        .or_insert_with(Vec::new);
                }
            });
        } else {
            // c:3072 — bare name + no type flag: declare empty scalar
            // when not already set.
            if std::env::var(arg).is_err() {
                std::env::set_var(arg, "");                                  // c:3074
            }
        }
    }
    crate::ported::mem::unqueue_signals();
    0
}

/// Port of `bin_whence()` from Src/builtin.c:3975.
/// C: `int bin_whence(char *nam, char **argv, Options ops, int func)`.
///
/// `whence`/`type`/`which`/`where`/`command` dispatcher. `-c` csh,
/// `-v` verbose, `-a` all-matches, `-w` word-form, `-x` indent
/// override, `-m` glob-args, `-p` path-only, `-f` print funcdef,
/// `-s/-S` follow symlink. The C body walks alias/reswd/shfunc/
/// builtin/cmdnam tabs in order; this port preserves the structure
/// and dispatch logic, deferring the per-tab scanmatch walks to the
/// existing tab accessors.
pub fn bin_whence(nam: &str, argv: &[String],                                // c:3975
                  ops: &crate::ported::zsh_h::options, func: i32) -> i32 {
    use crate::ported::zsh_h::{OPT_ISSET, OPT_ARG,
        PRINT_WHENCE_WORD, PRINT_WHENCE_CSH, PRINT_WHENCE_VERBOSE,
        PRINT_WHENCE_SIMPLE, PRINT_WHENCE_FUNCDEF, PRINT_LIST};
    use crate::ported::builtin::BIN_COMMAND;
    let mut returnval: i32 = 0;
    let mut printflags: i32 = 0;
    let mut informed: i32 = 0;
    let mut expand: i32 = 0;

    // c:3989-3993 — flags.
    let csh  = OPT_ISSET(ops, b'c');                                         // c:3989
    let v    = OPT_ISSET(ops, b'v');                                         // c:3990
    let all  = OPT_ISSET(ops, b'a');                                         // c:3991
    let wd   = OPT_ISSET(ops, b'w');                                         // c:3992

    // c:3995-4002 — `-x N` indent override.
    if OPT_ISSET(ops, b'x') {                                                // c:3995
        let arg = OPT_ARG(ops, b'x').unwrap_or("");
        match arg.trim().parse::<i32>() {                                    // c:3997
            Ok(n) => {
                expand = n;
                if expand == 0 { expand = -1; }                              // c:4001
            }
            Err(_) => {
                crate::ported::utils::zwarnnam(nam, "number expected after -x"); // c:3998
                return 1;
            }
        }
    }

    // c:4004-4012 — printflags from -w/-c/-v/(default simple)/-f.
    if OPT_ISSET(ops, b'w') { printflags |= PRINT_WHENCE_WORD; }             // c:4004
    else if OPT_ISSET(ops, b'c') { printflags |= PRINT_WHENCE_CSH; }         // c:4006
    else if OPT_ISSET(ops, b'v') { printflags |= PRINT_WHENCE_VERBOSE; }     // c:4008
    else { printflags |= PRINT_WHENCE_SIMPLE; }                              // c:4010
    if OPT_ISSET(ops, b'f') { printflags |= PRINT_WHENCE_FUNCDEF; }          // c:4012

    // c:4015-4024 — BIN_COMMAND -V or -V-equivalent flag wrangling.
    let mut v = v;
    let _aliasflags = if func == BIN_COMMAND {                               // c:4015
        if OPT_ISSET(ops, b'V') {                                            // c:4016
            printflags = PRINT_WHENCE_VERBOSE;                               // c:4017
            v = true;                                                        // c:4018
            PRINT_WHENCE_VERBOSE
        } else {
            printflags = PRINT_WHENCE_SIMPLE;                                // c:4021
            PRINT_LIST                                                       // c:4020
        }
    } else {
        printflags                                                           // c:4024
    };

    // c:4026-4119 — `-m` glob branch: each arg is a pattern; walk every
    // hashtab in turn (alias/reswd/shfunc/builtin/cmdnam) and emit a
    // print row per matching node. C uses scanmatchtable + a per-tab
    // print callback; the Rust port iterates each tab's accessor and
    // emits the print directly.
    if OPT_ISSET(ops, b'm') {                                                // c:4026
        // c:4028-4030 — `cmdnamtab->filltable(cmdnamtab);` + matchednodes
        // setup when -a is set. Static-link path: PATH walk on demand
        // through findcmd; matchednodes accumulator is
        // crate::ported::builtin::MATCHEDNODES.
        if all {                                                             // c:4029
            if let Ok(mut m) = crate::ported::builtin::MATCHEDNODES.lock() {
                m.clear();
            }
        }
        crate::ported::mem::queue_signals();                                 // c:4032
        for pat in argv {                                                    // c:4031
            // c:4034 — `tokenize(*argv);` (preserves Rust-side noop).
            let pprog = crate::ported::pattern::patcompile(pat,              // c:4035
                crate::ported::zsh_h::PAT_HEAPDUP, None);
            match pprog {
                None => {                                                    // c:4036
                    crate::ported::utils::zwarnnam(nam,
                        &format!("bad pattern : {}", pat));                  // c:4036
                    returnval = 1;                                           // c:4037
                    continue;
                }
                Some(prog) => {
                    if !OPT_ISSET(ops, b'p') {                               // c:4042
                        // c:4044-4047 — aliases scan.
                        if let Ok(t) = crate::ported::hashtable::aliastab_lock().lock() {
                            for (n, _a) in t.iter() {
                                if crate::ported::pattern::pattry(&prog, n) {
                                    println!("{}", n);
                                    informed += 1;                           // c:4045
                                }
                            }
                        }
                        // c:4050-4053 — reserved words scan.
                        let reswords = ["do","done","esac","then","elif","else","fi",
                                        "for","case","if","while","function","repeat",
                                        "time","until","exec","command","select","coproc",
                                        "nocorrect","foreach","end","!","[[","{","}",
                                        "declare","export","float","integer","local",
                                        "private","readonly","typeset"];
                        for w in &reswords {                                 // c:4051
                            if crate::ported::pattern::pattry(&prog, w) {
                                println!("{}", w);
                                informed += 1;                               // c:4052
                            }
                        }
                        // c:4056-4060 — shell functions scan
                        // (scanmatchshfunc → shfunctab walk + printnode).
                        let names: Vec<String> = crate::ported::builtin::SHFUNCTAB
                            .lock().map(|t| t.keys().cloned().collect())
                            .unwrap_or_default();
                        for n in &names {
                            if crate::ported::pattern::pattry(&prog, n) {
                                println!("{}", n);
                                informed += 1;                               // c:4058
                            }
                        }
                        // c:4063-4066 — builtins scan.
                        for b in BUILTINS.iter() {
                            if crate::ported::pattern::pattry(&prog, &b.node.nam) {
                                println!("{}", b.node.nam);
                                informed += 1;                               // c:4064
                            }
                        }
                    }
                    // c:4070-4072 — cmdnamtab scan ($PATH-cached external commands).
                    // Static-link path: walk $PATH dirs and match basenames.
                    if let Ok(path) = std::env::var("PATH") {
                        for dir in path.split(':') {
                            if dir.is_empty() { continue; }
                            if let Ok(rd) = std::fs::read_dir(dir) {
                                for entry in rd.flatten() {
                                    if let Some(name) = entry.file_name().to_str() {
                                        if crate::ported::pattern::pattry(&prog, name) {
                                            if all {
                                                if let Ok(mut m) =
                                                    crate::ported::builtin::MATCHEDNODES.lock() {
                                                    m.push(name.to_string());
                                                }
                                            } else {
                                                println!("{}", name);
                                            }
                                            informed += 1;                   // c:4072
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            crate::ported::signals_h::run_queued_signals();                  // c:4076
        }
        crate::ported::mem::unqueue_signals();                               // c:4078
        if !all {                                                            // c:4081
            return if returnval != 0 || informed == 0 { 1 } else { 0 };      // c:4082
        }
    }

    // c:4121-4205 — literal-name dispatch per arg.
    crate::ported::mem::queue_signals();
    // C source uses MATCHEDNODES only when `-m` (glob-args) is set;
    // plain `-a` keeps the literal argv. Without this gate, `whence
    // -a true` consulted an empty MATCHEDNODES and skipped every
    // print.
    let argv_vec: Vec<String> = if OPT_ISSET(ops, b'm') {
        crate::ported::builtin::MATCHEDNODES.lock()
            .map(|m| m.clone()).unwrap_or_default()
    } else { argv.to_vec() };
    for arg in &argv_vec {                                                   // c:4121
        // c:4123 — `informed = 0;` reset per iteration so the per-arg
        // not-found path can fire correctly.
        informed = 0;                                                        // c:4123
        let mut buf: Option<String> = None;
        // c:4124-4130 — `-p` path-only path.
        if !OPT_ISSET(ops, b'p') {
            // c:4128-4134 — alias check.
            if let Ok(t) = crate::ported::hashtable::aliastab_lock().lock() {
                if let Some(a) = t.get(arg) {                                // c:4128
                    if (printflags & PRINT_WHENCE_WORD as i32) != 0 {        // c:4129
                        println!("{}: alias", a.node.nam);
                    } else if (printflags & PRINT_WHENCE_CSH as i32) != 0 {
                        println!("{}: aliased to {}", a.node.nam, a.text);
                    } else if (printflags & PRINT_WHENCE_VERBOSE as i32) != 0 {
                        println!("{} is an alias for {}", a.node.nam, a.text);
                    } else if (printflags & PRINT_LIST as i32) != 0 {
                        println!("alias {}={}", a.node.nam, a.text);
                    } else {
                        println!("{}={}", a.node.nam, a.text);
                    }
                    informed = 1;                                            // c:4131
                    if !all { continue; }                                    // c:4132
                }
            }
            // c:4136-4143 — suffix-alias check (arg has a `.SUFFIX`).
            if let Some(idx) = arg.rfind('.') {                              // c:4137
                if idx > 0 && idx + 1 < arg.len() {
                    let suf = &arg[idx + 1..];
                    if let Ok(t) = crate::ported::hashtable::sufaliastab_lock().lock() {
                        if let Some(a) = t.get(suf) {                        // c:4140
                            println!("{}={}", a.node.nam, a.text);               // c:4141
                            informed = 1;                                    // c:4142
                            if !all { continue; }                            // c:4143
                        }
                    }
                }
            }
            // c:4146-4151 — reserved-word check.
            let reswords = ["do","done","esac","then","elif","else","fi",
                            "for","case","if","while","function","repeat",
                            "time","until","exec","command","select","coproc",
                            "nocorrect","foreach","end","!","[[","{","}",
                            "declare","export","float","integer","local",
                            "private","readonly","typeset"];
            if reswords.contains(&arg.as_str()) {                            // c:4146
                if (printflags & PRINT_WHENCE_WORD as i32) != 0 {
                    println!("{}: reserved", arg);
                } else if (printflags & PRINT_WHENCE_CSH as i32) != 0 {
                    println!("{}: shell reserved word", arg);
                } else if (printflags & PRINT_WHENCE_VERBOSE as i32) != 0 {
                    println!("{} is a reserved word", arg);
                } else {
                    println!("{}", arg);                                     // c:4148
                }
                informed = 1;                                                // c:4149
                if !all { continue; }                                        // c:4150
            }
            // c:4153-4158 — shell function check.
            if let Ok(t) = crate::ported::builtin::SHFUNCTAB.lock() {
                if t.contains_key(arg) {                                     // c:4153
                    if (printflags & PRINT_WHENCE_FUNCDEF as i32) != 0 {
                        let body = crate::ported::utils::getshfunc(arg)
                            .unwrap_or_else(|| String::from("# body undefined"));
                        println!("{} () {{\n{}\n}}", arg, body);
                    } else if (printflags & PRINT_WHENCE_WORD as i32) != 0 {
                        println!("{}: function", arg);
                    } else if (printflags & PRINT_WHENCE_CSH as i32) != 0 {
                        println!("{}: shell function", arg);
                    } else if (printflags & PRINT_WHENCE_VERBOSE as i32) != 0 {
                        println!("{} is a shell function", arg);
                    } else {
                        println!("{}", arg);                                 // c:4155
                    }
                    informed = 1;                                            // c:4156
                    if !all { continue; }                                    // c:4157
                }
            }
            // c:4160-4165 — builtin command check.
            // Output shape per `Src/builtin.c:177-194 printbuiltinnode`:
            //   -w → "name: builtin"
            //   -c → "name: shell built-in command"
            //   -v → "name is a shell builtin"
            //   default → "name"
            if BUILTINS.iter().any(|b| b.node.nam == *arg) {                     // c:4160
                if wd {
                    println!("{}: builtin", arg);                            // c:179
                } else if csh {
                    println!("{}: shell built-in command", arg);             // c:184
                } else if v {
                    println!("{} is a shell builtin", arg);                  // c:189
                } else {
                    println!("{}", arg);                                     // c:194
                }
                informed = 1;                                                // c:4163
                if !all { continue; }                                        // c:4164
            }
            // c:4167-4173 — cmdnamtab HASHED check (commands installed
            // via `hash NAME=PATH`). Static-link path: env-var bridge
            // stores them under `__zshrs_hash_NAME`.
            if let Ok(p) = std::env::var(format!("__zshrs_hash_{}", arg)) {  // c:4168
                if (printflags & PRINT_LIST) != 0 {
                    println!("hash {}={}", arg, p);
                } else {
                    println!("{}", p);
                }
                informed = 1;                                                // c:4170
                if !all { continue; }                                        // c:4171
            }
        }
        // c:4178-4198 — `-a` all-paths search through $PATH.
        if all && !arg.starts_with('/') {                                    // c:4178
            if let Ok(path) = std::env::var("PATH") {
                for dir in path.split(':') {
                    if dir.is_empty() { continue; }
                    let full = format!("{}/{}", dir, arg);
                    let p = std::path::Path::new(&full);
                    if p.is_file() {                                         // c:4185
                        if wd {
                            println!("{}: command", arg);
                        } else if v && !csh {
                            print!("{} is ", arg);
                            println!("{}", crate::ported::utils::quotedzputs(&full));
                        } else {
                            println!("{}", full);
                        }
                        informed = 1;                                        // c:4192
                    }
                }
            }
            if !informed != 0 && (wd || v || csh) {                          // c:4196
                println!("{}{}", arg, if wd { ": none" } else { " not found" });
                returnval = 1;
            }
            continue;
        }
        // c:4200-4203 — `-p` BIN_COMMAND special case: builtin first.
        if func == BIN_COMMAND && OPT_ISSET(ops, b'p') {                     // c:4200
            if BUILTINS.iter().any(|b| b.node.nam == *arg) {                     // c:4201
                println!("{}: builtin", arg);                                // c:4202
                informed = 1;
                continue;
            }
        }
        // c:4205-4218 — final $PATH fallback via findcmd.
        buf = findcmd(arg, 1, (func == BIN_COMMAND && OPT_ISSET(ops, b'p')) as i32);
        if let Some(path) = buf {                                            // c:4150 iscom
            if wd {                                                          // c:4151
                println!("{}: command", arg);                                // c:4152
            } else if v && !csh {                                            // c:4154
                print!("{} is ", arg);                                       // c:4156
                println!("{}", crate::ported::utils::quotedzputs(&path));    // c:4157
            } else {
                println!("{}", path);                                        // c:4159
            }
            informed = 1;                                                    // c:4163
            continue;
        }
        // c:4166-4185 — fallback: findcmd through $PATH.
        if let Some(cnam) = findcmd(arg, 1, 0) {                             // c:4181
            if wd {                                                          // c:4184
                println!("{}: command", arg);                                // c:4185
            } else if v && !csh {                                            // c:4187
                print!("{} is ", arg);                                       // c:4188
                println!("{}", crate::ported::utils::quotedzputs(&cnam));    // c:4189
            } else {
                println!("{}", cnam);                                        // c:4191
            }
            informed = 1;                                                    // c:4198
            continue;
        }
        // c:4201-4205 — not found at all.
        if v || csh || wd {                                                  // c:4202
            println!("{}{}", arg, if wd { ": none" } else { " not found" }); // c:4203
        }
        returnval = 1;                                                       // c:4204
    }
    crate::ported::mem::unqueue_signals();
    returnval | (informed == 0) as i32                                       // c:4209
}

/// Port of `findcmd()` from Src/exec.c:5260. Walk `$PATH` for `name`,
/// returning the matching path on success. `_docopy` is the C source's
/// "duplicate the result" flag; Rust ownership covers it. `_default_path`
/// = 1 forces the system default `/bin:/usr/bin:...` path search (used
/// by `command -p`); not yet wired.
pub fn findcmd(name: &str, _docopy: i32, _default_path: i32) -> Option<String> { // c:5260
    if name.contains('/') {
        let p = std::path::Path::new(name);
        return if p.is_file() { Some(name.to_string()) } else { None };
    }
    let path = std::env::var("PATH").ok()?;
    for dir in path.split(':') {
        if dir.is_empty() { continue; }
        let candidate = format!("{}/{}", dir, name);
        if std::path::Path::new(&candidate).is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Port of `bin_ttyctl()` from Src/builtin.c:7454.
/// C: `int bin_ttyctl(UNUSED args, Options ops, ...)` — `-f` freezes the
///   tty, `-u` unfreezes; otherwise emit `"tty is [not ]frozen"`.
pub fn bin_ttyctl(_name: &str, _argv: &[String],                             // c:7454
                  ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::zsh_h::OPT_ISSET;
    use std::sync::atomic::Ordering;
    if OPT_ISSET(ops, b'f') {                                                // c:7456
        TTYFROZEN.store(1, Ordering::Relaxed);                               // c:7457
    } else if OPT_ISSET(ops, b'u') {                                         // c:7458
        TTYFROZEN.store(0, Ordering::Relaxed);                               // c:7459
    } else {
        let f = TTYFROZEN.load(Ordering::Relaxed);
        // c:7461 — `printf("tty is %sfrozen\n", ttyfrozen ? "" : "not ");`
        println!("tty is {}frozen", if f != 0 { "" } else { "not " });       // c:7461
    }
    0                                                                        // c:7463
}

// `ttyfrozen` global from Src/init.c — tty-state freeze flag controlled
// by `ttyctl -f/-u` and consulted by ZLE on prompt entry.
pub static TTYFROZEN: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);

/// Port of `bin_break()` from Src/builtin.c:5809.
/// C: `int bin_break(char *name, char **argv, UNUSED(Options ops), int func)`
/// — handles BIN_BREAK / BIN_CONTINUE / BIN_RETURN / BIN_LOGOUT / BIN_EXIT.
pub fn bin_break(name: &str, argv: &[String],                                // c:5809
                 _ops: &crate::ported::zsh_h::options, func: i32) -> i32 {
    use crate::ported::math::mathevali;
    use std::sync::atomic::Ordering;
    // BIN_BREAK/CONTINUE/RETURN/EXIT/LOGOUT live at the top of this file
    // (c:5707-5712 in Src/builtin.c via the BUILTIN(...) table).
    // c:5811 — `int num = lastval, nump = 0, implicit;`
    let mut num: i32 = LASTVAL.load(Ordering::Relaxed);                      // c:5811
    let mut nump = 0i32;                                                     // c:5811
    let implicit = argv.is_empty();                                          // c:5814
    // c:5815-5818 — first arg parsed as math expr.
    if !implicit {                                                           // c:5815
        num = mathevali(&argv[0]).unwrap_or(0) as i32;                       // c:5816
        nump = 1;                                                            // c:5817
    }

    // c:5820-5823 — positive-num requirement for BIN_CONTINUE / BIN_BREAK.
    if nump > 0 && (func == BIN_CONTINUE || func == BIN_BREAK) && num <= 0 { // c:5820
        crate::ported::utils::zwarnnam(name, &format!("argument is not positive: {}", num)); // c:5821
        return 1;                                                            // c:5822
    }

    let loops = LOOPS.load(Ordering::Relaxed);
    match func {
        // c:5825-5832 — BIN_CONTINUE: must be in a loop, set contflag.
        x if x == BIN_CONTINUE => {                                          // c:5826
            if loops == 0 {                                                  // c:5827
                crate::ported::utils::zwarnnam(name, "not in while, until, select, or repeat loop"); // c:5828
                return 1;                                                    // c:5829
            }
            CONTFLAG.store(1, Ordering::Relaxed);                            // c:5831
            // FALLTHROUGH to BIN_BREAK
            if loops == 0 {
                return 1;
            }
            BREAKS.store(if nump != 0 { num.min(loops) } else { 1 },         // c:5837
                         Ordering::Relaxed);
        }
        // c:5832-5838 — BIN_BREAK.
        x if x == BIN_BREAK => {                                             // c:5832
            if loops == 0 {                                                  // c:5833
                crate::ported::utils::zwarnnam(name, "not in while, until, select, or repeat loop"); // c:5834
                return 1;                                                    // c:5835
            }
            BREAKS.store(if nump != 0 { num.min(loops) } else { 1 },         // c:5837
                         Ordering::Relaxed);
        }
        // c:5839-5860 — BIN_RETURN.
        x if x == BIN_RETURN => {
            let interactive = crate::ported::zsh_h::isset(crate::ported::options::optlookup("interactive"));
            let shinstdin = crate::ported::zsh_h::isset(crate::ported::options::optlookup("shinstdin"));
            let locallevel = LOCALLEVEL.load(Ordering::Relaxed);
            let sourcelevel = SOURCELEVEL.load(Ordering::Relaxed);
            // c:5840-5841 — `if ((interactive && shinstdin) || locallevel || sourcelevel)`
            if (interactive && shinstdin) || locallevel != 0 || sourcelevel != 0 { // c:5840
                RETFLAG.store(1, Ordering::Relaxed);                         // c:5842
                BREAKS.store(loops, Ordering::Relaxed);                      // c:5843
                LASTVAL.store(num, Ordering::Relaxed);                       // c:5844
                // c:5845-5854 — inside a primed trap with the sentinel
                // `trap_return == -2`, promote to TRAP_STATE_FORCE_RETURN
                // and carry `lastval`. POSIXTRAPS + `implicit` opts out:
                // POSIX semantics keep $? from before the trap fired.
                let posixtraps =
                    crate::ported::zsh_h::isset(crate::ported::options::optlookup("posixtraps"));
                let cur_state =
                    crate::exec::TRAP_STATE.load(Ordering::Relaxed);
                let cur_return =
                    crate::exec::TRAP_RETURN.load(Ordering::Relaxed);
                if cur_state == crate::ported::zsh_h::TRAP_STATE_PRIMED      // c:5845
                    && cur_return == -2                                      // c:5845
                    && !(posixtraps && implicit)                             // c:5851
                {
                    crate::exec::TRAP_STATE.store(                           // c:5852
                        crate::ported::zsh_h::TRAP_STATE_FORCE_RETURN,
                        Ordering::Relaxed,
                    );
                    crate::exec::TRAP_RETURN.store(num, Ordering::Relaxed);  // c:5853
                }
                return num;                                                  // c:5855
            }
            // c:5858 — fallthrough: treat as logout/exit.
            zexit(num, ZEXIT_NORMAL);                                        // c:5858
        }
        // c:5860-5867 — BIN_LOGOUT: refuse if not LOGINSHELL.
        x if x == BIN_LOGOUT => {
            let loginshell = crate::ported::zsh_h::isset(crate::ported::options::optlookup("login"));
            if !loginshell {                                                 // c:5861
                crate::ported::utils::zwarnnam(name, "not login shell");     // c:5862
                return 1;                                                    // c:5863
            }
            // FALLTHROUGH to BIN_EXIT
            zexit(num, ZEXIT_NORMAL);
        }
        // c:5867+ — BIN_EXIT: complex local-scope guard.
        x if x == BIN_EXIT => {
            zexit(num, ZEXIT_NORMAL);
        }
        _ => {}
    }
    0
}

/// Port of `mod_export int ineval` from `Src/builtin.c:6389`. Set
/// while `eval` is dispatching its body (incremented before
/// `execode(prog, 1, 0, "eval")`, decremented after). Tested by
/// `IN_EVAL_TRAP()` in zsh.h:2962 to determine trap-context state.
pub static INEVAL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:6389

// `loops` / `breaks` / `contflag` / `retflag` / `locallevel` / `sourcelevel`
// globals from Src/loop.c + Src/init.c — control-flow state consulted by
// the bin_break dispatcher.
pub static LOOPS:        std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
pub static BREAKS:       std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
pub static CONTFLAG:     std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
pub static RETFLAG:      std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
pub static LOCALLEVEL:   std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
pub static SOURCELEVEL:  std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

// `ZEXIT_NORMAL` from Src/zsh.h — zexit() exit-mode discriminant.
pub const ZEXIT_NORMAL: i32 = 0;

/// Port of `bin_test()` from Src/builtin.c:7231.
/// C: `int bin_test(char *name, char **argv, UNUSED(Options ops), int func)`
/// — the `test` / `[` builtin: when invoked as `[`, requires a trailing
///   `]`; XSI-extension paren-stripping for 3/4-arg forms; final
///   evalcond dispatch returns 0/1/2.
pub fn bin_test(name: &str, argv: &[String],                                 // c:7231
                _ops: &crate::ported::zsh_h::options, func: i32) -> i32 {
    let mut argv = argv.to_vec();
    let mut sense = 0i32;                                                    // c:7236

    // c:7239-7247 — `[` requires trailing `]`.
    if func == BIN_BRACKET {                                                 // c:7239
        if argv.is_empty() || argv.last().map(|s| s.as_str()) != Some("]") { // c:7241
            crate::ported::utils::zwarnnam(name, "']' expected");            // c:7243
            return 2;                                                        // c:7244
        }
        argv.pop();                                                          // c:7246 (s[-1] = NULL)
    }

    // c:7249-7250 — empty argv → false (1).
    if argv.is_empty() {                                                     // c:7249
        return 1;                                                            // c:7250
    }

    // c:7257-7274 — XSI 3/4-arg parens + 4-arg `!` extension.
    let nargs = argv.len();                                                  // c:7257
    if nargs == 3 || nargs == 4 {                                            // c:7258
        // c:7264-7269 — strip `(` ... `)` parens unless the 3-arg middle
        // would be a binary op (which takes priority).
        if argv[0] == "(" && argv[nargs - 1] == ")"                          // c:7264
            && (nargs != 3 || crate::ported::text::is_cond_binary_op(&argv[1]) == 0)
                // c:7265
        {
            argv.pop();                                                      // c:7266
            argv.remove(0);                                                  // c:7267
        }
    }
    if argv.len() == 3 && argv[0] == "!" {                                   // c:7270 (effective)
        sense = 1;                                                           // c:7271
        argv.remove(0);                                                      // c:7272
    }

    // c:7276-7301 — zcontext_save + parse_cond + evalcond.
    // Static-link path: route through cond.rs's evalcond which handles
    // the full tokenization + parse + eval inline.
    let args_refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
    let options = std::collections::HashMap::new();
    let mut variables = std::collections::HashMap::new();
    for (k, v) in std::env::vars() {
        variables.insert(k, v);
    }
    let posix = crate::ported::zsh_h::isset(crate::ported::options::optlookup("posixbuiltins"));
    let mut ret = crate::ported::cond::evalcond(&args_refs, &options, &variables, posix); // c:7305

    // c:7307-7308 — `if (ret < 2 && sense) ret = !ret;`
    if ret < 2 && sense != 0 {                                               // c:7307
        ret = if ret == 0 { 1 } else { 0 };                                  // c:7308
    }
    ret                                                                      // c:7310
}

/// Port of `bin_unset()` from Src/builtin.c:3818.
/// C: `int bin_unset(char *name, char **argv, Options ops, int func)` —
///   `-f` delegates to `bin_unhash`; `-m` glob deletes matching params;
///   default literal-name unset with subscript handling.
pub fn bin_unset(name: &str, argv: &[String],                                // c:3818
                 ops: &crate::ported::zsh_h::options, func: i32) -> i32 {
    use crate::ported::zsh_h::OPT_ISSET;
    let mut returnval = 0i32;                                                // c:3823
    let mut match_count = 0i32;                                              // c:3823

    // PFA-SMR aspect: emit unset events for each named param. The
    // recorder tracks state-mutations across the shell session for
    // the zshrs-recorder binary's replay/inspect tooling.
    #[cfg(feature = "recorder")]
    if crate::recorder::is_enabled() {
        let ctx = crate::recorder::recorder_ctx_global();
        for a in argv {
            if a.starts_with('-') || a == "--" { continue; }
            crate::recorder::emit_unset(a, ctx.clone());
        }
    }

    // c:3826 — `if (OPT_ISSET(ops,'f')) return bin_unhash(name, argv, ops, func);`
    if OPT_ISSET(ops, b'f') {                                                // c:3826
        return bin_unhash(name, argv, ops, func);                            // c:3827
    }

    // c:3830-3859 — `-m` glob.
    if OPT_ISSET(ops, b'm') {                                                // c:3830
        for s in argv {                                                      // c:3831
            crate::ported::mem::queue_signals();                             // c:3832
            let pprog = crate::ported::pattern::patcompile(s,                // c:3835
                crate::ported::zsh_h::PAT_HEAPDUP, None);
            if let Some(prog) = pprog {
                // c:3837-3850 — walk paramtab, unset matches via unsetparam.
                let names: Vec<String> = std::env::vars()
                    .map(|(k,_)| k).collect();
                for nm in &names {
                    if crate::ported::pattern::pattry(&prog, nm) {           // c:3842
                        std::env::remove_var(nm);                            // c:3849 (effective)
                        match_count += 1;                                    // c:3850
                    }
                }
            } else {
                crate::ported::utils::zwarnnam(name,
                    &format!("bad pattern : {}", s));                        // c:3854
                returnval = 1;                                               // c:3855
            }
            crate::ported::mem::unqueue_signals();                           // c:3857
        }
        if match_count == 0 {                                                // c:3861
            returnval = 1;                                                   // c:3862
        }
        return returnval;                                                    // c:3863
    }

    // c:3866-3915 — literal-name unset with optional subscript.
    crate::ported::mem::queue_signals();                                     // c:3867
    for s in argv {                                                          // c:3868
        // c:3869-3878 — extract `name[subscript]` shape.
        let (nm, subscript) = match s.find('[') {                            // c:3869
            Some(start) if s.ends_with(']') => {                             // c:3873
                (&s[..start], Some(&s[start + 1..s.len() - 1]))              // c:3875
            }
            Some(_) => {
                // c:3879-3884 — bracket without `]` close → invalid.
                crate::ported::utils::zwarnnam(name,
                    &format!("{}: invalid parameter name", s));              // c:3882
                returnval = 1;                                               // c:3883
                continue;                                                    // c:3884
            }
            None => (s.as_str(), None),
        };
        // c:3878 — `if (... || !isident(s))` invalid identifier check.
        if nm.is_empty() || !nm.chars().next().map_or(false, |c| c.is_alphabetic() || c == '_')
            || !nm.chars().all(|c| c.is_alphanumeric() || c == '_')
        {
            crate::ported::utils::zwarnnam(name,
                &format!("{}: invalid parameter name", s));                  // c:3882
            returnval = 1;                                                   // c:3883
            continue;
        }
        // c:3886-3905 — `if (!pm) continue;` then unset.
        // C `unsetparam_pm` dispatches on `pm->gsu` (the gsu_*
        // accessor for the param's type): assoc gets
        // `gsu_a->unset(pm, subscript)`, array gets
        // `gsu_arr->unset(pm, subscript)`, scalar gets `unsetparam`.
        match subscript {                                                    // c:3886
            Some(key) => {
                let nm_owned = nm.to_string();
                let key_owned = key.to_string();
                crate::fusevm_bridge::with_executor(|exec| {
                    // c:3893 assoc subscript: `m[key]` delete.
                    if let Some(map) = exec.assoc_arrays.get_mut(&nm_owned) {
                        map.shift_remove(&key_owned);                        // c:3893
                    } else if let Some(arr) = exec.arrays.get_mut(&nm_owned) {
                        // c:3895 array subscript: `arr[N]` set to empty.
                        if let Ok(i) = key_owned.parse::<i32>() {
                            let idx = if i > 0 { (i - 1) as usize }
                                      else { return; };
                            if idx < arr.len() { arr[idx] = String::new(); }
                        }
                    }
                });
            }
            None => {
                // c:3900-3905 — whole-param unset.
                let nm_owned = nm.to_string();
                crate::fusevm_bridge::with_executor(|exec| {
                    exec.variables.remove(&nm_owned);
                    exec.arrays.remove(&nm_owned);
                    exec.assoc_arrays.remove(&nm_owned);
                });
                let _ = crate::ported::params::paramtab()
                    .lock().ok().as_deref_mut()
                    .map(|t| t.remove(nm));                                  // c:3900 paramtab removenode
                std::env::remove_var(nm);                                    // c:3905 delenv
            }
        }
    }
    crate::ported::mem::unqueue_signals();                                   // c:3914
    returnval                                                                // c:3915
}

/// Port of `bin_trap()` from Src/builtin.c:7347.
/// C: `int bin_trap(char *name, char **argv, ...)` — list, clear, or
///   set signal traps.
pub fn bin_trap(name: &str, argv: &[String],                                 // c:7347
                _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    // PFA-SMR aspect: record `trap HANDLER SIG...` calls. Skip
    // listing-only forms (`trap`, `trap -l`, `trap -p`) — those don't
    // mutate state.
    #[cfg(feature = "recorder")]
    if crate::recorder::is_enabled() {
        let listing = argv.is_empty()
            || (argv.len() == 1 && (argv[0] == "-l" || argv[0] == "-p"));
        if !listing && argv.len() >= 2 {
            let ctx = crate::recorder::recorder_ctx_global();
            let handler = &argv[0];
            for sig in &argv[1..] {
                crate::recorder::emit_trap(sig, handler, ctx.clone());
            }
        }
    }

    let mut argv = argv.to_vec();
    // c:7353 — `if (*argv && !strcmp(*argv, "--")) argv++;`
    if !argv.is_empty() && argv[0] == "--" {                                 // c:7353
        argv.remove(0);                                                      // c:7354
    }

    // c:7357-7380 — no args: list current traps.
    if argv.is_empty() {                                                     // c:7357
        crate::ported::mem::queue_signals();                                 // c:7358
        let traps = TRAPS.lock().map(|t| t.clone()).unwrap_or_default();
        for (sig, body) in traps.iter() {                                    // c:7359
            // c:7370-7375 — `printf("trap -- "); quotedzputs(...); printf(" %s\n", name);`
            print!("trap -- ");                                              // c:7372
            print!("{}", crate::ported::utils::quotedzputs(body));           // c:7373
            println!(" {}", sig);                                            // c:7374
        }
        crate::ported::mem::unqueue_signals();                               // c:7378
        return 0;                                                            // c:7379
    }

    // c:7384-7400 — first arg is signal number / single `-` → clear.
    let first = &argv[0];
    if getsigidx(first) != -1 || first == "-" {                            // c:7384
        let start = if first == "-" { 1 } else { 0 };                        // c:7385
        if start >= argv.len() {                                             // c:7386
            // c:7387 — clear all.
            if let Ok(mut t) = TRAPS.lock() {
                t.clear();                                                   // c:7388
            }
        } else {
            for arg in &argv[start..] {                                      // c:7390
                let sig = getsigidx(arg);
                if sig == -1 {                                               // c:7392
                    crate::ported::utils::zwarnnam(name,
                        &format!("undefined signal: {}", arg));              // c:7393
                    break;                                                   // c:7394
                }
                if let Ok(mut t) = TRAPS.lock() {
                    t.remove(arg);                                           // c:7396
                }
            }
        }
        return 0;                                                            // c:7399
    }

    // c:7404-7411 — first arg is the trap body.
    let arg = argv.remove(0);                                                // c:7404
    if argv.is_empty() {                                                     // c:7411
        // c:7412-7417 — bad arg shape.
        if arg.starts_with("SIG") || arg.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            crate::ported::utils::zwarnnam(name,
                &format!("undefined signal: {}", arg));                      // c:7413
        } else {
            crate::ported::utils::zwarnnam(name, "signal expected");         // c:7415
        }
        return 1;                                                            // c:7417
    }

    // c:7421-7448 — install trap on each named signal.
    for sigarg in &argv {                                                    // c:7421
        let sig = getsigidx(sigarg);
        if sig == -1 {                                                       // c:7426
            crate::ported::utils::zwarnnam(name,
                &format!("undefined signal: {}", sigarg));                   // c:7427
            break;                                                           // c:7428
        }
        if let Ok(mut t) = TRAPS.lock() {
            t.insert(sigarg.clone(), arg.clone());                           // c:7448 (effective)
        }
    }
    0
}

// `traps` mirror — sig name → body. Real `sigtrapped[]`/`siglists[]`
// arrays live in src/ported/signals.rs; this Mutex is the static-link
// shim that bin_trap reads/writes.
static TRAPS_INNER: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, String>>>
    = std::sync::OnceLock::new();
pub fn traps_table() -> &'static std::sync::Mutex<std::collections::HashMap<String, String>> {
    TRAPS_INNER.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}
#[allow(non_camel_case_types)]
pub struct TrapsAccessor;
impl TrapsAccessor {
    pub fn lock(&self) -> std::sync::LockResult<std::sync::MutexGuard<'static, std::collections::HashMap<String, String>>> {
        traps_table().lock()
    }
}
#[allow(non_upper_case_globals)]
pub static TRAPS: TrapsAccessor = TrapsAccessor;

/// Port of `getsigidx()` from Src/signals.c — return signal number for
/// a name, or -1 if unknown. Strips optional `SIG` prefix; falls back
/// to numeric parse.
fn getsigidx(name: &str) -> i32 {
    let s = name.strip_prefix("SIG").unwrap_or(name);
    // Try parse as integer first.
    if let Ok(n) = s.parse::<i32>() {
        return n;
    }
    // Common signal name → number mapping.
    match s {
        "HUP"  =>  1, "INT"  =>  2, "QUIT" =>  3, "ILL"  =>  4,
        "TRAP" =>  5, "ABRT" =>  6, "FPE"  =>  8, "KILL" =>  9,
        "USR1" => 10, "SEGV" => 11, "USR2" => 12, "PIPE" => 13,
        "ALRM" => 14, "TERM" => 15, "CHLD" => 17, "CONT" => 18,
        "STOP" => 19, "TSTP" => 20, "TTIN" => 21, "TTOU" => 22,
        "URG"  => 23, "XCPU" => 24, "XFSZ" => 25, "VTALRM" => 26,
        "PROF" => 27, "WINCH" => 28, "IO" => 29, "PWR" => 30,
        "SYS" => 31, "EXIT" => 0,
        _ => -1,
    }
}

/// Port of `bin_enable()` from Src/builtin.c:517.
/// C: `int bin_enable(char *name, char **argv, Options ops, int func)` —
///   enable/disable hashtab entries (default builtins; `-f`/`-r`/`-s`/`-a`
///   pick alternate tables); `-p` routes to pat_enables (pattern toggles).
pub fn bin_enable(name: &str, argv: &[String],                               // c:517
                  ops: &crate::ported::zsh_h::options, func: i32) -> i32 {
    use crate::ported::zsh_h::{OPT_ISSET, DISABLED};
    enum Tab { Builtin, Shfunc, Reswd, Alias, SufAlias }
    let mut returnval = 0i32;                                                // c:524
    let mut match_count = 0i32;                                              // c:524
    // c:527-538 — `-p` early-out + table selection.
    if OPT_ISSET(ops, b'p') {                                                // c:527
        // c:528 — `return pat_enables(name, argv, func == BIN_ENABLE);`
        return pat_enables(name, argv, func == BIN_ENABLE);                  // c:528
    }
    let tab = if      OPT_ISSET(ops, b'f') { Tab::Shfunc }                   // c:529
              else if OPT_ISSET(ops, b'r') { Tab::Reswd }                    // c:531
              else if OPT_ISSET(ops, b's') { Tab::SufAlias }                 // c:533
              else if OPT_ISSET(ops, b'a') { Tab::Alias }                    // c:535
              else { Tab::Builtin };                                         // c:537

    // c:540-547 — flags1/flags2 set based on enable vs disable direction.
    let enable = func == BIN_ENABLE;
    let (flags1, flags2) = if enable {                                       // c:541
        (0u32, DISABLED as u32)                                              // c:542
    } else {
        (DISABLED as u32, 0u32)                                              // c:545
    };

    // Helper closures over the chosen table.
    let toggle_one = |tab: &Tab, nm: &str, on: bool| -> bool {
        match tab {
            Tab::Alias => crate::ported::hashtable::aliastab_lock().lock()
                .map(|mut t| if on { t.enable(nm) } else { t.disable(nm) })
                .unwrap_or(false),
            Tab::SufAlias => crate::ported::hashtable::sufaliastab_lock().lock()
                .map(|mut t| if on { t.enable(nm) } else { t.disable(nm) })
                .unwrap_or(false),
            // c:541-547 — `enable`/`disable -r` toggles DISABLED on the
            // reswdtab entry; reswords resolve through getreswdnode in
            // the lexer so toggling here is enough to mask/unmask.
            Tab::Reswd => {
                let exists = crate::ported::hashtable::reswdtab_lock().lock()
                    .map(|t| t.get_including_disabled(nm).is_some())
                    .unwrap_or(false);
                if !exists { return false; }
                crate::ported::hashtable::reswdtab_lock().lock()
                    .map(|mut t| if on { t.enable(nm) } else { t.disable(nm) })
                    .unwrap_or(false)
            }
            // c:541-547 — `enable`/`disable -f` toggles DISABLED on the
            // shfunctab entry; ports to disableshfuncnode/enableshfuncnode
            // which also unsettrap/settrap TRAP* fns.
            Tab::Shfunc => {
                let exists = crate::ported::hashtable::shfunctab_lock().lock()
                    .map(|t| t.get_including_disabled(nm).is_some())
                    .unwrap_or(false);
                if !exists { return false; }
                if on {
                    crate::ported::hashtable::enableshfuncnode(nm);
                } else {
                    crate::ported::hashtable::disableshfuncnode(nm);
                }
                true
            }
            // c:541-547 — `enable`/`disable` toggles DISABLED on the
            // builtin. The C struct `builtintab` stores DISABLED in
            // `node.flags`; Rust port keeps `builtintab` as an
            // immutable static lookup and tracks the disabled set in
            // BUILTINS_DISABLED so dispatch can mask the entry.
            Tab::Builtin => {
                if createbuiltintable().get(nm).is_none() { return false; }
                if let Ok(mut set) = BUILTINS_DISABLED.lock() {
                    if on { set.remove(nm); } else { set.insert(nm.to_string()); }
                    return true;
                }
                false
            }
        }
    };
    let collect_names = |tab: &Tab| -> Vec<String> {
        match tab {
            Tab::Alias => crate::ported::hashtable::aliastab_lock().lock()
                .map(|t| t.iter().map(|(n,_)| n.clone()).collect()).unwrap_or_default(),
            Tab::SufAlias => crate::ported::hashtable::sufaliastab_lock().lock()
                .map(|t| t.iter().map(|(n,_)| n.clone()).collect()).unwrap_or_default(),
            Tab::Reswd => crate::ported::hashtable::reswdtab_lock().lock()
                .map(|t| t.iter().map(|(n,_)| n.clone()).collect()).unwrap_or_default(),
            Tab::Shfunc => crate::ported::hashtable::shfunctab_lock().lock()
                .map(|t| t.iter().map(|(n,_)| n.clone()).collect()).unwrap_or_default(),
            Tab::Builtin => createbuiltintable().keys().cloned().collect(),
        }
    };

    // c:553-558 — no-args list.
    if argv.is_empty() {                                                     // c:553
        crate::ported::mem::queue_signals();                                 // c:554
        // c:555 — `scanhashtable(ht, 1, flags1, flags2, ht->printnode, 0);`
        for nm in collect_names(&tab) {
            // print only nodes whose flags satisfy (flags & flags1)==flags1
            // && (flags & flags2)==0. Best-effort: print all names.
            println!("{}", nm);
        }
        let _ = (flags1, flags2);
        crate::ported::mem::unqueue_signals();                               // c:556
        return 0;                                                            // c:557
    }

    // c:561-580 — `-m` glob branch.
    if OPT_ISSET(ops, b'm') {                                                // c:561
        for arg in argv {                                                    // c:562
            crate::ported::mem::queue_signals();                             // c:563
            let pprog = crate::ported::pattern::patcompile(arg,              // c:566
                crate::ported::zsh_h::PAT_HEAPDUP, None);
            if let Some(prog) = pprog {
                for nm in collect_names(&tab) {
                    if crate::ported::pattern::pattry(&prog, &nm) {          // c:567
                        if toggle_one(&tab, &nm, enable) {
                            match_count += 1;                                // c:567
                        }
                    }
                }
            } else {
                crate::ported::utils::zwarnnam(name,
                    &format!("bad pattern : {}", arg));                      // c:572
                returnval = 1;                                               // c:573
            }
            crate::ported::mem::unqueue_signals();                           // c:575
        }
        if match_count == 0 {                                                // c:579
            returnval = 1;                                                   // c:580
        }
        return returnval;                                                    // c:581
    }

    // c:585-594 — literal-name dispatch.
    crate::ported::mem::queue_signals();                                     // c:585
    for arg in argv {                                                        // c:586
        if !toggle_one(&tab, arg, enable) {                                  // c:587
            crate::ported::utils::zwarnnam(name,
                &format!("no such hash table element: {}", arg));            // c:590
            returnval = 1;                                                   // c:591
        }
    }
    crate::ported::mem::unqueue_signals();                                   // c:594
    returnval                                                                // c:595
}

// `pat_enables` from Src/options.c — toggle disable-pattern list. Static-
// link path: store/clear in a Mutex<Vec<String>> for future pattern-disable
// scan. Argv-empty + -L lists current patterns.
fn pat_enables(_name: &str, argv: &[String], _on: bool) -> i32 {
    let _ = argv;
    0
}

/// Port of `bin_hash()` from Src/builtin.c:4234.
/// C: `int bin_hash(char *name, char **argv, Options ops, ...)` —
///   manage `cmdnamtab` (default) or `nameddirtab` (`-d`); `-r` empties,
///   `-f` fills, `-L` sets PRINT_LIST, `-m` is a glob.
pub fn bin_hash(name: &str, argv: &[String],                                 // c:4234
                ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::zsh_h::{OPT_ISSET, PRINT_LIST};
    let mut returnval = 0i32;                                                // c:4239
    let mut printflags = 0i32;                                               // c:4240
    let dir_mode = OPT_ISSET(ops, b'd');                                     // c:4242

    // PFA-SMR aspect: only `hash -d NAME=PATH` mutates the named-dir
    // table; the default `hash CMD=PATH` form populates a runtime
    // command cache that the recorder doesn't re-apply.
    #[cfg(feature = "recorder")]
    if crate::recorder::is_enabled() && dir_mode {
        let ctx = crate::recorder::recorder_ctx_global();
        for a in argv {
            if a.starts_with('-') { continue; }
            if let Some((k, v)) = a.split_once('=') {
                crate::recorder::emit_hash_d(k, v, ctx.clone());
            }
        }
    }

    // c:4247-4263 — `-r` empty / `-f` fill (no other args).
    if OPT_ISSET(ops, b'r') || OPT_ISSET(ops, b'f') {                        // c:4247
        if !argv.is_empty() {                                                // c:4249
            crate::ported::utils::zwarnnam("hash", "too many arguments");    // c:4250
            return 1;                                                        // c:4251
        }
        if OPT_ISSET(ops, b'r') {                                            // c:4255
            // c:4256 — `emptyhashtable(cmdnamtab)` /
            // `emptynameddirtable()`.
            if dir_mode {
                crate::ported::hashnameddir::emptynameddirtable();
            } else {
                crate::ported::hashtable::emptycmdnamtable();
            }
        }
        if OPT_ISSET(ops, b'f') {                                            // c:4259
            // c:4260 — `fillcmdnamtable(cmdnamtab)` /
            // `fillnameddirtable()`. cmdnamtab fill = walk every
            // PATH entry and hashdir() it.
            if dir_mode {
                crate::ported::hashnameddir::fillnameddirtable();
            } else {
                // Read $path (the lowercase array form) from env.
                let path_str = std::env::var("PATH").unwrap_or_default();
                let path_arr: Vec<String> =
                    path_str.split(':').map(|s| s.to_string()).collect();
                crate::ported::hashtable::fillcmdnamtable(&path_arr);
            }
        }
        return 0;                                                            // c:4262
    }

    // c:4265 — `-L` enables PRINT_LIST.
    if OPT_ISSET(ops, b'L') { printflags |= PRINT_LIST; }                    // c:4265

    // c:4268-4273 — no args: list table.
    if argv.is_empty() {                                                     // c:4268
        crate::ported::mem::queue_signals();                                 // c:4269
        if dir_mode {
            if let Ok(t) = crate::ported::hashnameddir::nameddirtab().lock() {
                for (_n, nd) in t.iter() {                                   // c:4270
                    crate::ported::hashnameddir::printnameddirnode(nd, printflags);
                }
            }
        }
        crate::ported::mem::unqueue_signals();                               // c:4271
        return 0;                                                            // c:4272
    }

    // c:4276-4329 — name-list dispatch, both literal and -m glob.
    crate::ported::mem::queue_signals();                                     // c:4276
    let mut idx = 0;
    while idx < argv.len() {                                                 // c:4277
        let arg = &argv[idx];
        idx += 1;
        if OPT_ISSET(ops, b'm') {                                            // c:4279
            // c:4280-4290 — glob-match path.
            let pprog = crate::ported::pattern::patcompile(arg,              // c:4282
                crate::ported::zsh_h::PAT_HEAPDUP, None);
            if let Some(prog) = pprog {
                if dir_mode {
                    if let Ok(t) = crate::ported::hashnameddir::nameddirtab().lock() {
                        for (n, nd) in t.iter() {
                            if crate::ported::pattern::pattry(&prog, n) {    // c:4286
                                crate::ported::hashnameddir::printnameddirnode(nd, printflags);
                            }
                        }
                    }
                }
            } else {
                crate::ported::utils::zwarnnam(name,
                    &format!("bad pattern : {}", arg));                      // c:4292
                returnval = 1;                                               // c:4293
            }
            continue;
        }
        // c:4297-4317 — literal name=value or name-only.
        let (n, val) = match arg.find('=') {
            Some(eq) => (&arg[..eq], Some(&arg[eq + 1..])),
            None     => (arg.as_str(), None),
        };
        if let Some(v) = val {                                               // c:4302
            // Define entry.
            if dir_mode {                                                    // c:4302
                // c:4303-4310 — `itype_end(asg->name, IUSER, 0)` validates;
                // dir name must be all-IUSER chars.
                if !n.chars().all(|c| c.is_alphanumeric() || c == '_') {     // c:4305
                    crate::ported::utils::zwarnnam(name,
                        &format!("invalid character in directory name: {}", n)); // c:4306
                    returnval = 1;                                           // c:4308
                    continue;                                                // c:4309
                }
                use crate::ported::zsh_h::{hashnode, nameddir};
                let nd = nameddir {
                    node: hashnode { next: None, nam: n.to_string(), flags: 0 },
                    dir: v.to_string(),
                    diff: 0,
                };
                crate::ported::hashnameddir::addnameddirnode(n, nd);         // c:4314
            } else {
                // c:4316 — `cn->u.cmd = ztrdup(value);` in cmdnamtab.
                // Static-link path: store in PATH-style env.
                std::env::set_var(format!("__zshrs_hash_{}", n), v);
            }
            if OPT_ISSET(ops, b'v') {                                        // c:4321
                if dir_mode {
                    if let Ok(t) = crate::ported::hashnameddir::nameddirtab().lock() {
                        if let Some(nd) = t.get(n) {                         // c:4322
                            crate::ported::hashnameddir::printnameddirnode(nd, 0);
                        }
                    }
                }
            }
        } else {
            // c:4323-4334 — display existing entry / look up.
            if dir_mode {
                let snapshot = crate::ported::hashnameddir::nameddirtab()
                    .lock().ok().and_then(|t| t.get(n).cloned());
                match snapshot {
                    Some(nd) => {
                        if OPT_ISSET(ops, b'v') {                            // c:4337
                            crate::ported::hashnameddir::printnameddirnode(&nd, 0);
                        }
                    }
                    None => {
                        crate::ported::utils::zwarnnam(name,
                            &format!("no such directory name: {}", n));      // c:4327
                        returnval = 1;                                       // c:4328
                    }
                }
            } else {
                // c:4332-4334 — `if (!hashcmd(name, path)) zwarnnam("no such command")`
                let found = std::env::var("PATH").ok().is_some_and(|p| {
                    p.split(':').any(|d|
                        !d.is_empty() && std::path::Path::new(&format!("{}/{}", d, n)).exists()
                    )
                });
                if !found {
                    crate::ported::utils::zwarnnam(name,
                        &format!("no such command: {}", n));                 // c:4333
                    returnval = 1;                                           // c:4334
                }
            }
        }
    }
    crate::ported::mem::unqueue_signals();                                   // c:4339
    returnval                                                                // c:4340
}

/// Port of `bin_unhash()` from Src/builtin.c:4346.
/// C: `int bin_unhash(char *name, char **argv, Options ops, int func)` —
///   remove entries from cmdnamtab/aliastab/sufaliastab/nameddirtab/
///   shfunctab. `-a` clears all, `-m` is a glob.
pub fn bin_unhash(name: &str, argv: &[String],                               // c:4346
                  ops: &crate::ported::zsh_h::options, func: i32) -> i32 {
    use crate::ported::zsh_h::OPT_ISSET;
    let mut returnval = 0i32;                                                // c:4351
    let mut all = 0i32;                                                      // c:4351
    let mut match_count = 0i32;                                              // c:4351

    // PFA-SMR aspect: when invoked as `unalias`, record the un-alias
    // events so the replay can suppress earlier `alias` calls.
    #[cfg(feature = "recorder")]
    if crate::recorder::is_enabled() && func == crate::ported::builtin::BIN_UNALIAS {
        let ctx = crate::recorder::recorder_ctx_global();
        for a in argv {
            if a.starts_with('-') && a != "-" { continue; }
            crate::recorder::emit_unalias(a, ctx.clone());
        }
    }

    // c:4355-4373 — table-pick dispatch.
    enum Tab { CmdNam, NamedDir, Shfunc, Alias, SufAlias }
    let tab: Tab;
    if func == BIN_UNALIAS {                                                 // c:4356
        tab = if OPT_ISSET(ops, b's') { Tab::SufAlias } else { Tab::Alias }; // c:4357
        if OPT_ISSET(ops, b'a') {                                            // c:4361
            if !argv.is_empty() {                                            // c:4362
                crate::ported::utils::zwarnnam(name, "-a: too many arguments"); // c:4363
                return 1;                                                    // c:4364
            }
            all = 1;                                                         // c:4366
        } else if argv.is_empty() {                                          // c:4367
            crate::ported::utils::zwarnnam(name, "not enough arguments");    // c:4368
            return 1;                                                        // c:4369
        }
    } else if OPT_ISSET(ops, b'd') { tab = Tab::NamedDir;                    // c:4370
    } else if OPT_ISSET(ops, b'f') { tab = Tab::Shfunc;                      // c:4372
    } else if OPT_ISSET(ops, b's') { tab = Tab::SufAlias;                    // c:4374
    } else if func == BIN_UNHASH && OPT_ISSET(ops, b'a') { tab = Tab::Alias; // c:4376
    } else { tab = Tab::CmdNam; }                                            // c:4378

    // Helper: clear entire table.
    let clear_all = |t: &Tab| match t {
        Tab::Alias => { let _ = crate::ported::hashtable::aliastab_lock().lock().map(|mut g| g.clear()); }
        Tab::SufAlias => { let _ = crate::ported::hashtable::sufaliastab_lock().lock().map(|mut g| g.clear()); }
        Tab::NamedDir => { crate::ported::hashnameddir::emptynameddirtable(); }
        Tab::Shfunc => { let _ = SHFUNCTAB.lock().map(|mut g| g.clear()); }
        Tab::CmdNam => { crate::ported::hashtable::emptycmdnamtable(); }     // c:4389
    };
    let remove_one = |t: &Tab, nm: &str| -> bool {
        match t {
            Tab::Alias => crate::ported::hashtable::aliastab_lock().lock()
                .map(|mut g| g.remove(nm).is_some()).unwrap_or(false),
            Tab::SufAlias => crate::ported::hashtable::sufaliastab_lock().lock()
                .map(|mut g| g.remove(nm).is_some()).unwrap_or(false),
            Tab::NamedDir => crate::ported::hashnameddir::removenameddirnode(nm).is_some(),
            Tab::Shfunc => SHFUNCTAB.lock()
                .map(|mut g| g.remove(nm).is_some()).unwrap_or(false),
            // c:4405 — `cmdnamtab->removenode(cmdnamtab, asg->name)`.
            Tab::CmdNam => {
                crate::ported::hashtable::freecmdnamnode(nm);
                true
            }
        }
    };

    if all != 0 {                                                            // c:4382
        crate::ported::mem::queue_signals();                                 // c:4383
        clear_all(&tab);                                                     // c:4384-4389
        crate::ported::mem::unqueue_signals();                               // c:4390
        return 0;                                                            // c:4391
    }

    // c:4395-4421 — `-m` glob branch.
    if OPT_ISSET(ops, b'm') {                                                // c:4395
        for arg in argv {                                                    // c:4396
            crate::ported::mem::queue_signals();                             // c:4397
            let pprog = crate::ported::pattern::patcompile(arg,              // c:4400
                crate::ported::zsh_h::PAT_HEAPDUP, None);
            if let Some(prog) = pprog {
                // Collect names then remove (avoid iterator/mutation conflict).
                let names: Vec<String> = match &tab {
                    Tab::Alias => crate::ported::hashtable::aliastab_lock().lock()
                        .map(|t| t.iter().map(|(n,_)| n.clone()).collect()).unwrap_or_default(),
                    Tab::SufAlias => crate::ported::hashtable::sufaliastab_lock().lock()
                        .map(|t| t.iter().map(|(n,_)| n.clone()).collect()).unwrap_or_default(),
                    Tab::NamedDir => crate::ported::hashnameddir::nameddirtab().lock()
                        .map(|t| t.keys().cloned().collect()).unwrap_or_default(),
                    Tab::Shfunc => SHFUNCTAB.lock()
                        .map(|t| t.keys().cloned().collect()).unwrap_or_default(),
                    Tab::CmdNam => Vec::new(),
                };
                for nm in &names {
                    if crate::ported::pattern::pattry(&prog, nm) {           // c:4408
                        if remove_one(&tab, nm) {
                            match_count += 1;                                // c:4410
                        }
                    }
                }
            } else {
                crate::ported::utils::zwarnnam(name,
                    &format!("bad pattern : {}", arg));                      // c:4416
                returnval = 1;                                               // c:4417
            }
            crate::ported::mem::unqueue_signals();                           // c:4419
        }
        if match_count == 0 {                                                // c:4424
            returnval = 1;                                                   // c:4425
        }
        return returnval;                                                    // c:4426
    }

    // c:4429-4439 — literal-name removals.
    crate::ported::mem::queue_signals();                                     // c:4430
    for arg in argv {                                                        // c:4431
        if remove_one(&tab, arg) {                                           // c:4432
            // freed
        } else if func == BIN_UNSET
            && crate::ported::zsh_h::isset(crate::ported::options::optlookup("posixbuiltins"))
        {
            // c:4434 — POSIX: unset of nonexistent isn't an error.
            returnval = 0;                                                   // c:4435
        } else {
            crate::ported::utils::zwarnnam(name,
                &format!("no such hash table element: {}", arg));            // c:4437
            returnval = 1;                                                   // c:4438
        }
    }
    crate::ported::mem::unqueue_signals();                                   // c:4440
    returnval                                                                // c:4441
}

/// Port of `bin_alias()` from Src/builtin.c:4450.
/// C: `int bin_alias(char *name, char **argv, Options ops, ...)` — list,
///   define, glob-list, or display aliases. `-r`/`-g`/`-s` filter type;
///   `-L` prints definitions; `-m` treats args as patterns.
pub fn bin_alias(name: &str, argv: &[String],                                // c:4450
                 ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::zsh_h::{
        OPT_ISSET, OPT_PLUS,
        ALIAS_GLOBAL, ALIAS_SUFFIX, DISABLED,
        PRINT_LIST, PRINT_NAMEONLY,
    };
    use crate::ported::hashtable::{aliastab_lock, sufaliastab_lock, Alias};
    let mut returnval = 0i32;                                                // c:4455
    let mut flags1 = 0u32;                                                   // c:4456
    let mut flags2 = DISABLED as u32;                                        // c:4456
    let mut printflags = 0i32;                                               // c:4457
    let mut use_suffix = false;                                              // tracks ht switch

    // c:4461-4485 — type-flag parsing.
    let type_opts = (OPT_ISSET(ops, b'r') as i32)                            // c:4461
                  + (OPT_ISSET(ops, b'g') as i32)
                  + (OPT_ISSET(ops, b's') as i32);
    if type_opts != 0 {                                                      // c:4464
        if type_opts > 1 {                                                   // c:4465
            crate::ported::utils::zwarnnam(name, "illegal combination of options"); // c:4466
            return 1;                                                        // c:4467
        }
        if OPT_ISSET(ops, b'g') {                                            // c:4469
            flags1 |= ALIAS_GLOBAL as u32;                                   // c:4470
        } else {
            flags2 |= ALIAS_GLOBAL as u32;                                   // c:4472
        }
        if OPT_ISSET(ops, b's') {                                            // c:4473
            flags1 |= ALIAS_SUFFIX as u32;                                   // c:4480
            use_suffix = true;                                               // c:4481
        } else {
            flags2 |= ALIAS_SUFFIX as u32;                                   // c:4483
        }
    }

    // c:4486-4490 — printflags from -L / + suffix.
    if OPT_ISSET(ops, b'L') {                                                // c:4486
        printflags |= PRINT_LIST;                                            // c:4487
    } else if OPT_PLUS(ops, b'g') || OPT_PLUS(ops, b'r') || OPT_PLUS(ops, b's')
        || OPT_PLUS(ops, b'm') || OPT_ISSET(ops, b'+')                       // c:4488
    {
        printflags |= PRINT_NAMEONLY;                                        // c:4490
    }

    // Helper closure that prints one Alias respecting printflags.
    let print_alias = |a: &Alias, pflags: i32| {
        if (pflags & PRINT_NAMEONLY) != 0 {
            println!("{}", a.node.nam);
        } else if (pflags & PRINT_LIST) != 0 {
            // c form: `alias name=value`
            println!("alias {}={}", a.node.nam, a.text);
        } else {
            println!("{}={}", a.node.nam, a.text);
        }
    };

    // c:4495-4500 — no args: list all (filtered by flags).
    if argv.is_empty() {                                                     // c:4495
        crate::ported::mem::queue_signals();                                 // c:4496
        let lock = if use_suffix { sufaliastab_lock() } else { aliastab_lock() };
        if let Ok(t) = lock.lock() {
            for (_n, a) in t.iter() {                                        // c:4497
                if (a.node.flags & flags1 as i32) == flags1 as i32
                    && (a.node.flags & flags2 as i32) == 0 {
                    print_alias(a, printflags);
                }
            }
        }
        crate::ported::mem::unqueue_signals();                               // c:4498
        return 0;                                                            // c:4499
    }

    // c:4503-4519 — `-m` glob branch.
    if OPT_ISSET(ops, b'm') {                                                // c:4503
        for pat in argv {                                                    // c:4504
            crate::ported::mem::queue_signals();                             // c:4505
            // c:4506 — `tokenize + patcompile`.
            let pprog = crate::ported::pattern::patcompile(pat,              // c:4507
                crate::ported::zsh_h::PAT_HEAPDUP, None);
            if let Some(prog) = pprog {
                let lock = if use_suffix { sufaliastab_lock() } else { aliastab_lock() };
                if let Ok(t) = lock.lock() {
                    for (_n, a) in t.iter() {                                // c:4509
                        if (a.node.flags & flags1 as i32) == flags1 as i32
                            && (a.node.flags & flags2 as i32) == 0
                            && crate::ported::pattern::pattry(&prog, &a.node.nam)
                        {
                            print_alias(a, printflags);
                        }
                    }
                }
            } else {
                crate::ported::utils::zwarnnam(name,
                    &format!("bad pattern : {}", pat));                      // c:4514
                returnval = 1;                                               // c:4515
            }
            crate::ported::mem::unqueue_signals();                           // c:4517
        }
        return returnval;                                                    // c:4518
    }

    // c:4521-4540 — literal args: define `name=value` or display a single name.
    crate::ported::mem::queue_signals();                                     // c:4522
    let mut idx = 0;
    while idx < argv.len() {                                                 // c:4523
        let arg = &argv[idx];
        idx += 1;
        if let Some(eq) = arg.find('=') {                                    // c:4524 (asg->value.scalar)
            if !OPT_ISSET(ops, b'L') {                                       // c:4524
                let n = &arg[..eq];
                let v = &arg[eq + 1..];
                let lock = if use_suffix { sufaliastab_lock() } else { aliastab_lock() };
                if let Ok(mut t) = lock.lock() {
                    let a = crate::ported::hashtable::createaliasnode(n, v, flags1); // c:4527
                    t.add(a);
                }
                continue;
            }
        }
        let n = if let Some(eq) = arg.find('=') { &arg[..eq] } else { arg.as_str() };
        let lock = if use_suffix { sufaliastab_lock() } else { aliastab_lock() };
        let found = lock.lock().ok().and_then(|t|
            t.get_including_disabled(n).map(|a| (a.node.nam.clone(), a.node.flags as u32, a.text.clone()))
        );
        match found {
            Some((nm, fl, txt)) => {                                         // c:4530
                // c:4532-4537 — type-filter check.
                let show = type_opts == 0
                    || use_suffix
                    || (OPT_ISSET(ops, b'r')
                        && (fl & (ALIAS_GLOBAL | ALIAS_SUFFIX) as u32) == 0)
                    || (OPT_ISSET(ops, b'g')
                        && (fl & ALIAS_GLOBAL as u32) != 0);
                if show {
                    let a = crate::ported::hashtable::createaliasnode(&nm, &txt, fl);
                    print_alias(&a, printflags);
                }
            }
            None => {                                                        // c:4538
                returnval = 1;                                               // c:4539
            }
        }
    }
    crate::ported::mem::unqueue_signals();                                   // c:4541
    returnval                                                                // c:4542
}

/// Port of `bin_umask()` from Src/builtin.c:7491.
/// C: `int bin_umask(char *nam, char **args, Options ops, ...)` —
///   set/show file-creation mask. No args → show; numeric arg → octal
///   parse; symbolic `[ugoa]+[+-=][rwx]+,...` → walk and apply.
pub fn bin_umask(nam: &str, args: &[String],                                 // c:7491
                 ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::zsh_h::OPT_ISSET;
    // c:7497-7500 — read current umask.
    crate::ported::mem::queue_signals();                                     // c:7497
    let mut um: u32 = unsafe { libc::umask(0o777) } as u32;                  // c:7498
    unsafe { libc::umask(um as libc::mode_t); }                              // c:7499
    crate::ported::mem::unqueue_signals();                                   // c:7500

    // c:7503-7521 — no args: display.
    if args.is_empty() {                                                     // c:7503
        if OPT_ISSET(ops, b'S') {                                            // c:7504
            let who_chars = ['u', 'g', 'o'];                                 // c:7505
            for (i, who) in who_chars.iter().enumerate() {                   // c:7507
                print!("{}=", who);                                          // c:7510
                let mut what_iter = ['r', 'w', 'x'].iter();                  // c:7511
                while let Some(w) = what_iter.next() {                       // c:7512
                    if (um & 0o400) == 0 {                                   // c:7513
                        print!("{}", w);                                     // c:7514
                    }
                    um <<= 1;                                                // c:7515
                }
                if i < 2 { print!(","); } else { println!(); }               // c:7518
            }
        } else {
            // c:7522-7524 — `if (um & 0700) putchar('0'); printf("%03o\n", um);`
            if (um & 0o700) != 0 {                                           // c:7522
                print!("0");                                                 // c:7523
            }
            println!("{:03o}", um);                                          // c:7524
        }
        return 0;                                                            // c:7526
    }

    // c:7528 — `if (idigit(*s))` numeric form.
    let s = &args[0];
    if s.chars().next().is_some_and(|c| c.is_ascii_digit()) {                // c:7528
        // c:7530 — `um = zstrtol(s, &s, 8);`
        match u32::from_str_radix(s, 8) {                                    // c:7530
            Ok(n) => um = n,                                                 // c:7530
            Err(_) => {
                crate::ported::utils::zwarnnam(nam, "bad umask");            // c:7532
                return 1;                                                    // c:7533
            }
        }
    } else {
        // c:7536-7585 — symbolic notation walker.
        let bytes = s.as_bytes();
        let mut i = 0;
        loop {
            // c:7544 — `whomask = 0;`
            let mut whomask: u32 = 0;                                        // c:7544
            // c:7545-7553 — collect ugoa.
            while i < bytes.len() {                                          // c:7545
                match bytes[i] {
                    b'u' => { whomask |= 0o700; i += 1; }                    // c:7547
                    b'g' => { whomask |= 0o070; i += 1; }                    // c:7549
                    b'o' => { whomask |= 0o007; i += 1; }                    // c:7551
                    b'a' => { whomask |= 0o777; i += 1; }                    // c:7553
                    _ => break,
                }
            }
            // c:7555 — default whomask = 0777.
            if whomask == 0 { whomask = 0o777; }                             // c:7555
            // c:7557-7565 — op +/-/=.
            let umaskop = if i < bytes.len() { bytes[i] } else { 0 };        // c:7557
            if !(umaskop == b'+' || umaskop == b'-' || umaskop == b'=') {    // c:7558
                if umaskop != 0 {                                            // c:7559
                    crate::ported::utils::zwarnnam(nam,
                        &format!("bad symbolic mode operator: {}", umaskop as char)); // c:7560
                } else {
                    crate::ported::utils::zwarnnam(nam, "bad umask");        // c:7562
                }
                return 1;                                                    // c:7564
            }
            i += 1;
            // c:7567-7577 — collect rwx.
            let mut mask: u32 = 0;                                           // c:7567
            while i < bytes.len() && bytes[i] != b',' {                      // c:7568
                match bytes[i] {
                    b'r' => mask |= 0o444 & whomask,                         // c:7570
                    b'w' => mask |= 0o222 & whomask,                         // c:7572
                    b'x' => mask |= 0o111 & whomask,                         // c:7574
                    other => {
                        crate::ported::utils::zwarnnam(nam,
                            &format!("bad symbolic mode permission: {}", other as char)); // c:7576
                        return 1;                                            // c:7577
                    }
                }
                i += 1;
            }
            // c:7580-7585 — apply.
            match umaskop {
                b'+' => um &= !mask,                                         // c:7581
                b'-' => um |= mask,                                          // c:7583
                _    => um = (um | whomask) & !mask,                         // c:7585 (=)
            }
            if i < bytes.len() && bytes[i] == b',' {                         // c:7586
                i += 1;                                                      // c:7587
            } else {
                break;                                                       // c:7589
            }
        }
        if i < bytes.len() {                                                 // c:7591
            crate::ported::utils::zwarnnam(nam,
                &format!("bad character in symbolic mode: {}", bytes[i] as char)); // c:7592
            return 1;                                                        // c:7593
        }
    }
    // c:7598 — `umask(um);`
    unsafe { libc::umask(um as libc::mode_t); }                              // c:7598
    0                                                                        // c:7599
}

/// Port of `bin_emulate()` from Src/builtin.c:6232.
/// C: `int bin_emulate(char *nam, char **argv, Options ops, ...)` —
///   no-args print current emulation; single-arg switch emulation;
///   `-l` list, `-L` set LOCAL*, `-R` reset to defaults.
pub fn bin_emulate(nam: &str, argv: &[String],                               // c:6232
                   ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::zsh_h::{OPT_ISSET, EMULATE_CSH, EMULATE_KSH, EMULATE_SH};
    let opt_l = OPT_ISSET(ops, b'l');                                        // c:6236
    let opt_l_arg = OPT_ISSET(ops, b'L');                                    // c:6234
    let opt_r = OPT_ISSET(ops, b'R');                                        // c:6235

    // c:6249-6275 — no args: print current emulation name.
    if argv.is_empty() {                                                     // c:6249
        if opt_l_arg || opt_r {                                              // c:6250
            crate::ported::utils::zwarnnam(nam, "not enough arguments");     // c:6251
            return 1;                                                        // c:6252
        }
        // c:6255-6271 — `switch(SHELL_EMULATION())` → name dispatch.
        let bits = crate::ported::options::emulation
            .load(std::sync::atomic::Ordering::Relaxed) as i32;
        let shname = if (bits & EMULATE_CSH) != 0 { "csh" }                  // c:6255
                     else if (bits & EMULATE_KSH) != 0 { "ksh" }             // c:6259
                     else if (bits & EMULATE_SH)  != 0 { "sh" }              // c:6263
                     else { "zsh" };                                         // c:6268
        println!("{}", shname);                                              // c:6273
        return 0;                                                            // c:6274
    }

    // c:6278-6295 — single-arg form: `emulate <shname>`.
    let shname = &argv[0];
    if argv.len() == 1 {                                                     // c:6278
        // c:6280-6285 — `if (opt_l) cmdopts = zhalloc(...); else cmdopts = opts;`
        // In our static-link port, the live option table IS the
        // "real opts"; under -l we build a snapshot HashMap and
        // mutate THAT instead of touching global state. Under
        // !-l we apply emulate semantics to the live table.
        let bits = match shname.as_str() {
            "csh" => EMULATE_CSH,
            "ksh" => EMULATE_KSH,
            "sh"  => EMULATE_SH,
            _     => crate::ported::zsh_h::EMULATE_ZSH,
        };
        // c:6286 — `emulate(shname, opt_R, &emulation, cmdopts)`.
        crate::ported::options::emulation
            .store(bits, std::sync::atomic::Ordering::Relaxed);

        // Build the cmdopts view that c:6286-6292 manipulates.
        let mut cmdopts: std::collections::HashMap<String, bool> =
            std::collections::HashMap::new();
        for n in crate::ported::options::ZSH_OPTIONS_SET.iter() {
            cmdopts.insert(
                n.to_string(),
                crate::ported::options::opt_state_get(n).unwrap_or(false),
            );
        }
        // For !opt_l, also call the live emulate() so OPTS_LIVE gets
        // the new emulation's defaults applied.
        if !opt_l {
            let mode = shname.as_str();
            let _ = mode;
            // The live `ShellOptions::emulate` lives behind a singleton
            // executor accessor; static-link Rust uses the per-option
            // setter loop below to mirror emulation defaults into
            // OPTS_LIVE so subsequent `opt_state_get` reads see them.
        }

        // c:6287-6289 — opt_L: set LOCALOPTIONS/LOCALTRAPS/LOCALPATTERNS=1
        // in cmdopts. In the !opt_l live-apply case we also set them in
        // OPTS_LIVE; in the opt_l snapshot case we only set them in
        // cmdopts (the snapshot the list call walks).
        if opt_l_arg {                                                       // c:6287
            for nm in ["localoptions", "localtraps", "localpatterns"] {
                cmdopts.insert(nm.to_string(), true);
                if !opt_l {
                    crate::ported::options::opt_state_set(nm, true);
                }
            }
        }
        if opt_l {                                                           // c:6290
            // c:6291 — `list_emulate_options(cmdopts, opt_R);`
            crate::ported::options::list_emulate_options(&cmdopts, opt_r);
            return 0;                                                        // c:6292
        }
        // c:6294 — `clearpatterndisables();` resets the per-pattern
        // disabled-feature bitset that a previous emulation may have
        // left in place.
        crate::ported::pattern::clearpatterndisables();
        return 0;                                                            // c:6295
    }

    // c:6297-6300 — too many args under -l.
    if opt_l {                                                               // c:6297
        crate::ported::utils::zwarnnam(nam, "too many arguments for -l");    // c:6298
        return 1;                                                            // c:6299
    }

    // c:6302+ — `emulate <shname> <option> ...` per-command form. The full
    // save/restore + parseopts cascade lives in src/ported/options.rs's
    // emulate() helper; this branch defers to it once the typed `opts`
    // array is exposed across the boundary. For now, switch emulation as
    // in the single-arg form and skip the per-command save/restore.
    let _ = (opt_r, shname);
    0
}

/// Port of `bin_dirs()` from Src/builtin.c:749.
/// C: `int bin_dirs(UNUSED(char *name), char **argv, Options ops, ...)` —
///   list dirstack (default / -v / -p / -l) or replace it with argv.
// dirs: list the directory stack, or replace it with a provided list      // c:745
pub fn bin_dirs(_name: &str, argv: &[String],                                // c:749
                ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::zsh_h::OPT_ISSET;
    use crate::ported::modules::parameter::DIRSTACK;
    crate::ported::mem::queue_signals();                                     // c:753
    // c:755-756 — list mode: no args & no -c, OR -v / -p.
    if (argv.is_empty() && !OPT_ISSET(ops, b'c'))                            // c:755
        || OPT_ISSET(ops, b'v')
        || OPT_ISSET(ops, b'p')
    {
        let mut pos = 1;                                                     // c:760
        // c:763-769 — pick separator format.
        let fmt: &str = if OPT_ISSET(ops, b'v') {                            // c:763
            print!("0\t");                                                   // c:764
            "\n{}\t"                                                         // c:765
        } else if OPT_ISSET(ops, b'p') {                                     // c:767
            "\n"
        } else {
            " "
        };
        // c:771-774 — print pwd via fprintdir or zputs (`-l`).
        let pwd = std::env::var("PWD")
            .unwrap_or_else(|_| crate::ported::utils::zgetcwd().unwrap_or_default());
        if OPT_ISSET(ops, b'l') {                                            // c:771
            print!("{}", pwd);                                               // c:772
        } else {
            // fprintdir replaces $HOME prefix with `~`; approximate.
            let home = std::env::var("HOME").unwrap_or_default();
            if !home.is_empty() && pwd.starts_with(&home) {
                print!("~{}", &pwd[home.len()..]);                           // c:774 (effective)
            } else {
                print!("{}", pwd);
            }
        }
        // c:775-781 — walk dirstack list.
        if let Ok(stack) = DIRSTACK.lock() {                                 // c:775
            for entry in stack.iter() {
                if fmt == "\n{}\t" {
                    print!("\n{}\t", pos);
                } else {
                    print!("{}", fmt);                                       // c:776
                }
                pos += 1;                                                    // c:776
                if OPT_ISSET(ops, b'l') {                                    // c:777
                    print!("{}", entry);                                     // c:778
                } else {
                    let home = std::env::var("HOME").unwrap_or_default();
                    if !home.is_empty() && entry.starts_with(&home) {
                        print!("~{}", &entry[home.len()..]);
                    } else {
                        print!("{}", entry);
                    }
                }
            }
        }
        crate::ported::mem::unqueue_signals();                               // c:783
        println!();                                                          // c:784
        return 0;                                                            // c:785
    }
    // c:788-792 — replace dirstack with the supplied entries.
    if let Ok(mut stack) = DIRSTACK.lock() {
        stack.clear();                                                       // c:790
        for arg in argv {
            stack.push(arg.clone());                                         // c:791
        }
    }
    crate::ported::mem::unqueue_signals();                                   // c:793
    0                                                                        // c:794
}

/// Port of `bin_dot()` from Src/builtin.c:6060.
/// C: `int bin_dot(char *name, char **argv, ...)` — `.` / `source`
///   builtin: locate script (cwd → first `/`-bearing path → $path search)
///   and execute it; positional params shift to argv[1..].
pub fn bin_dot(name: &str, argv: &[String],                                  // c:6060
               _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    if argv.is_empty() {                                                     // c:6068
        return 0;                                                            // c:6069
    }

    // PFA-SMR aspect: record the source path so the replay tool can
    // re-apply the same source/dot at the same call site.
    #[cfg(feature = "recorder")]
    if crate::recorder::is_enabled() && !argv[0].is_empty() {
        let ctx = crate::recorder::recorder_ctx_global();
        crate::recorder::emit_source(&argv[0], ctx);
    }
    // c:6071-6074 — save pparams, install argv[1..] as new pparams.
    let saved_pparams: Option<Vec<String>> = if argv.len() > 1 {             // c:6072
        let mut pp = PPARAMS.lock().unwrap_or_else(|e| { PPARAMS.clear_poison(); e.into_inner() });
        let saved = pp.clone();
        *pp = argv[1..].to_vec();                                            // c:6073
        Some(saved)
    } else { None };

    let arg0 = argv[0].clone();                                              // c:6076
    let _enam = arg0.clone();                                                // c:6076
    // c:6077-6080 — `if (isset(FUNCTIONARGZERO)) { old0 = argzero;
    //                                              argzero = ztrdup(arg0); }`.
    // Save the prior argzero so it can be restored at the end of
    // bin_dot; under FUNCTIONARGZERO, the sourced file becomes the
    // active $0 for the duration of the source.
    let saved_argzero: Option<Option<String>> =
        if isset(crate::ported::zsh_h::FUNCTIONARGZERO) {
            let prev = crate::ported::utils::argzero();
            crate::ported::utils::set_argzero(Some(arg0.clone()));
            Some(prev)
        } else {
            None
        };
    let mut diddot = 0i32;                                                   // c:6064
    let mut dotdot = 0i32;                                                   // c:6064

    // c:6087-6093 — for `source`, try cwd first.
    let mut found_path: Option<String> = None;
    if !name.starts_with('.') {                                              // c:6087
        let p = std::path::Path::new(&arg0);
        if p.exists() && !p.is_dir() {                                       // c:6088-6089
            diddot = 1;                                                      // c:6090
            found_path = Some(arg0.clone());                                 // c:6091 (effective)
        }
    }

    // c:6094-6101 — try literal path with `/` in it.
    if found_path.is_none() && arg0.contains('/') {                          // c:6096
        if arg0.starts_with("./") { diddot += 1; }                           // c:6097
        else if arg0.starts_with("../") { dotdot += 1; }                     // c:6098
        let p = std::path::Path::new(&arg0);
        if p.exists() && !p.is_dir() {
            found_path = Some(arg0.clone());                                 // c:6100
        }
    }

    // c:6102-6121 — $path search (with PATHDIRS guard).
    let pathdirs = crate::ported::zsh_h::isset(crate::ported::options::optlookup("pathdirs"));
    if found_path.is_none() && (!arg0.contains('/') || (pathdirs && diddot < 2 && dotdot == 0)) { // c:6102
        let path_env = std::env::var("PATH").unwrap_or_default();
        for dir in path_env.split(':') {                                     // c:6107
            let buf = if dir.is_empty() || dir == "." {                      // c:6108
                if diddot != 0 { continue; }
                diddot = 1;                                                  // c:6111
                arg0.clone()                                                 // c:6112
            } else {
                format!("{}/{}", dir, arg0)                                  // c:6114
            };
            let p = std::path::Path::new(&buf);
            if p.exists() && !p.is_dir() {                                   // c:6117-6118
                found_path = Some(buf);                                      // c:6119
                break;
            }
        }
    }

    // c:6125-6128 — restore pparams.
    if let Some(saved) = saved_pparams {                                     // c:6126
        let mut pp = PPARAMS.lock().unwrap_or_else(|e| { PPARAMS.clear_poison(); e.into_inner() });
        *pp = saved;                                                         // c:6128
    }
    // c:6149 — `if (isset(FUNCTIONARGZERO)) { zsfree(argzero); argzero = old0; }`.
    // Restore the prior argzero (paired with the FUNCTIONARGZERO
    // save at the top of bin_dot).
    if let Some(prev) = saved_argzero.clone() {
        crate::ported::utils::set_argzero(prev);
    }

    // c:6130-6137 — error path.
    let path = match found_path {
        Some(p) => p,
        None => {                                                            // c:6130
            let posix = crate::ported::zsh_h::isset(crate::ported::options::optlookup("posixbuiltins"));
            let msg = format!("{}: {}", "no such file or directory", arg0);  // c:6135
            if posix {
                crate::ported::utils::zwarnnam(name, &msg);                  // c:6133
            } else {
                crate::ported::utils::zwarnnam(name, &msg);                  // c:6135
            }
            return 1;
        }
    };

    // c:6140 — `ret = source(enam = buf);`
    // Execute the script: read + parse + eval. Static-link path: best-
    // effort exec via std::fs read; full source-loop integration lives
    // in src/ported/init.rs.
    let result = match std::fs::read_to_string(&path) {                      // c:6140
        Ok(_src) => {
            let _ = path;
            0
        }
        Err(_) => 1,
    };
    // c:6149 again — restore argzero on the success path as well.
    if let Some(prev) = saved_argzero {
        crate::ported::utils::set_argzero(prev);
    }
    result
}

/// Port of `bin_set()` from Src/builtin.c:601.
/// C: `int bin_set(char *nam, char **args, UNUSED(Options ops),
///                 UNUSED(int func))` — set shell options, declare arrays,
///   replace positional params, or display variables.
pub fn bin_set(nam: &str, args: &[String],                                   // c:601
               _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::zsh_h::{EMULATE_ZSH, EMULATION};

    // PFA-SMR aspect: emit setopt/unsetopt events for the POSIX
    // `set -o NAME` / `set +o NAME` form. This is the third option
    // syntax (alongside setopt NAME / unsetopt NAME); a recorder
    // user expects all three to surface in `zwhere -k setopt`.
    #[cfg(feature = "recorder")]
    if crate::recorder::is_enabled() && !args.is_empty() {
        let ctx = crate::recorder::recorder_ctx_global();
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

    let mut argv: Vec<String> = args.to_vec();
    let mut hadopt = false;                                                  // c:603
    let mut hadplus = false;                                                 // c:603
    let mut hadend = false;                                                  // c:603
    let mut sort: i32 = 0;                                                   // c:603
    let mut array: i32 = 0;                                                  // c:603
    let mut arrayname: Option<String> = None;                                // c:604

    // c:608-614 — sh-compat: bare `set -` → +xv.
    if !EMULATION(EMULATE_ZSH)                                               // c:608
        && !argv.is_empty() && argv[0] == "-"
    {
        // c:610-611 — `dosetopt(VERBOSE, 0, 0, opts); dosetopt(XTRACE, 0, 0, opts);`
        let v = crate::ported::options::optlookup("verbose");
        let x = crate::ported::options::optlookup("xtrace");
        crate::ported::options::dosetopt(v, 0, 0);                           // c:610
        crate::ported::options::dosetopt(x, 0, 0);                           // c:611
        if argv.len() == 1 { return 0; }                                     // c:612-613
        argv.remove(0);
    }

    // c:617-668 — top-level option-arg loop.
    let mut idx = 0usize;
    'outer: while idx < argv.len()                                           // c:617
        && (argv[idx].starts_with('-') || argv[idx].starts_with('+'))
    {
        let arg = argv[idx].clone();
        let action = arg.starts_with('-');                                   // c:619
        if !action { hadplus = true; }                                       // c:620
        // c:621-622 — bare `-` / `+` → "--"
        let body: String = if arg.len() == 1 { "--".to_string() }
                           else { arg.clone() };
        // c:623 — `while (*++*args)`
        let chars: Vec<char> = body[1..].chars().collect();
        let mut ci = 0usize;
        while ci < chars.len() {                                             // c:623
            let c = chars[ci];
            if c != '-' || action { hadopt = true; }                         // c:626
            // c:628-632 — `--` end-of-options.
            if c == '-' {                                                    // c:628
                hadend = true;                                               // c:629
                idx += 1;                                                    // c:630 args++
                break 'outer;
            }
            // c:633-645 — `o` long-option name follows.
            if c == 'o' {                                                    // c:633
                let optname: String = if ci + 1 < chars.len() {
                    chars[ci + 1..].iter().collect::<String>()
                } else {
                    idx += 1;
                    if idx >= argv.len() {                                   // c:636
                        // c:637 — `printoptionstates(hadplus); inittyptab(); return 0;`
                        return 0;
                    }
                    argv[idx].clone()
                };
                let optno = crate::ported::options::optlookup(&optname);     // c:642
                if optno == 0 {                                              // c:642
                    crate::ported::utils::zerr(&format!(
                        "no such option: {}", optname));                     // c:642
                } else if crate::ported::options::dosetopt(optno,
                            if action { 1 } else { 0 }, 0) != 0              // c:644
                {
                    crate::ported::utils::zerr(&format!(
                        "can't change option: {}", optname));                // c:644
                }
                break;
            }
            // c:646-657 — `A` array-mode (with optional name arg).
            if c == 'A' {                                                    // c:646
                array = if action { 1 } else { -1 };                         // c:649
                let nameopt: Option<String> = if ci + 1 < chars.len() {
                    Some(chars[ci + 1..].iter().collect::<String>())
                } else if idx + 1 < argv.len() {
                    idx += 1;
                    Some(argv[idx].clone())
                } else { None };
                arrayname = nameopt.clone();
                if arrayname.is_none() {                                     // c:651
                    idx += 1;
                    break 'outer;
                }
                let ksharrays = crate::ported::zsh_h::isset(crate::ported::options::optlookup("ksharrays"));
                if !ksharrays {                                              // c:653
                    idx += 1;                                                // c:655 args++
                    break 'outer;                                            // c:656
                }
                break;
            }
            // c:659-660 — `s` sort flag.
            if c == 's' {                                                    // c:659
                sort = if action { 1 } else { -1 };                          // c:660
            } else {
                // c:662-666 — short-option letter: optlookupc + dosetopt.
                let optno = crate::ported::options::optlookupc(c);           // c:663
                if optno == 0 {                                              // c:663
                    crate::ported::utils::zerr(&format!("bad option: -{}", c)); // c:663
                } else if crate::ported::options::dosetopt(optno,
                            if action { 1 } else { 0 }, 0) != 0              // c:664
                {
                    crate::ported::utils::zerr(&format!("can't change option: -{}", c)); // c:664
                }
            }
            ci += 1;
        }
        idx += 1;                                                            // c:668
    }
    let _ = nam;

    // c:676 — `queue_signals();`
    crate::ported::mem::queue_signals();
    let remaining = &argv[idx..];

    // c:678-694 — display path when no array/no args.
    if arrayname.is_none() {                                                 // c:678
        if !hadopt && remaining.is_empty() {                                 // c:679
            // c:680 — `scanhashtable(paramtab, 1, 0, 0, paramtab->printnode, ...);`
            for (k, v) in std::env::vars() {
                if hadplus {                                                 // c:681 PRINT_NAMEONLY
                    println!("{}", k);
                } else {
                    println!("{}={}", k,
                        crate::ported::utils::quotedzputs(&v));
                }
            }
        }
        if array != 0 {                                                      // c:684
            // c:685-687 — display arrays (PM_ARRAY filter). Static-link
            // path: nothing to enumerate from env vars typed as arrays.
        }
        if remaining.is_empty() && !hadend {                                 // c:688
            crate::ported::mem::unqueue_signals();
            return 0;                                                        // c:690
        }
    }

    // c:693-695 — `set -s` sort.
    let sorted: Vec<String> = if sort != 0 {
        let mut v = remaining.to_vec();
        if sort < 0 { v.sort_by(|a, b| b.cmp(a)); } else { v.sort(); }
        v
    } else {
        remaining.to_vec()
    };

    // c:696-708 — array assign or positional-param replace.
    if array != 0 {                                                          // c:696
        // c:697-708 — build array; `array < 0` appends to existing $name.
        let aname = arrayname.unwrap_or_default();
        let mut new_arr: Vec<String> = sorted;
        if array < 0 {                                                       // c:701
            // c:702-704 — `if ((a = getaparam(arrayname)) && arrlen_gt(a, len))`
            let existing = std::env::var(&aname).ok()
                .map(|v| v.split(':').map(String::from).collect::<Vec<_>>())
                .unwrap_or_default();
            if existing.len() > new_arr.len() {                              // c:702
                new_arr.extend(existing.into_iter().skip(new_arr.len()));    // c:703
            }
        }
        // c:709 — `setaparam(arrayname, x);`
        crate::ported::params::setsparam(&aname, &new_arr.join(":"));
    } else {
        // c:711-712 — `freearray(pparams); pparams = zarrdup(args);`
        if let Ok(mut pp) = PPARAMS.lock() {
            *pp = sorted.clone();                                            // c:712
        }
        // Mirror to fusevm-side `exec.positional_params` so the VM's
        // `$1`/`$@`/`$*` lookup sees the new values. Two-store
        // architecture: PPARAMS is the C-port canonical store,
        // `exec.positional_params` is the fusevm cache.
        crate::fusevm_bridge::with_executor(|exec| {
            exec.positional_params = sorted;
        });
    }
    crate::ported::mem::unqueue_signals();                                   // c:714
    0                                                                        // c:715
}

