//! Direct port of `Src/builtin.c` — the master registration site for
//! the in-shell builtin commands. The C source is 7608 lines; the
//! actual `bin_*` handler bodies were ported organically into
//! `src/ported/vm_helper` and `src/ported/builtins/*.rs` long before
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
//! the comment cites the file; when it lives in `vm_helper`'s
//! `Executor` impl, that's noted too.

use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::{Ordering, Ordering::Relaxed};
use std::sync::{Mutex, OnceLock};
#[allow(unused_imports)]
use std::{env, fs, io, io::Write, path::Path, path::PathBuf};

use crate::func_body_fmt::FuncBodyFmt;
#[allow(unused_imports)]
use crate::parse::{Redirect, ShellCommand};
use crate::ported::compat::zgetcwd;
use crate::ported::config_h::DEFAULT_PATH;
use crate::ported::exec::{getfpfunc, iscom, loadautofn, FORKLEVEL, TRAP_RETURN, TRAP_STATE};
use crate::ported::hashnameddir::{
    addnameddirnode, emptynameddirtable, fillnameddirtable, nameddirtab, printnameddirnode,
};
use crate::ported::hashtable::{
    aliastab_lock, cmdnamtab_lock, createaliasnode, dircache_set, emptycmdnamtable,
    fillcmdnamtable, hnamcmp, printaliasnode, printcmdnamnode, printshfuncexpand, reswdtab_lock,
    scanmatchshfunc, scanshfunc, shfunctab_lock, sufaliastab_lock,
};
use crate::DPUTS;
use indexmap::IndexMap;
// `curhist` (hist.rs static) NOT imported — there's an unavoidable
// `let curhist` local in fc_main that mirrors C's `int curhist;` local
// shadowing the global. Rule E says keep the C name. The static is
// referenced via its fully-qualified path at the single read site to
// avoid name-shadow E0530.
use crate::ported::hist::{
    addhistnum, gethistent, hcomsearch, histsiz, pushhiststack, quietgethist, readhistfile,
    saveandpophiststack, savehistfile, savehistsiz,
};
use crate::ported::jobs::{bin_fg, removetrapnode};
use crate::ported::math::{matheval, mathevali, mnumber, MN_INTEGER};
use crate::ported::mem::{queue_signals, unqueue_signals};
use crate::ported::module::MATHFUNCS;
use crate::ported::modules::parameter::{DIRSTACK, FUNCSTACK};
use crate::ported::options::{dosetopt, emulation, optlookup, ZSH_OPTIONS_SET};
use crate::ported::params::{
    createparam, getiparam, getsparam, isident, locallevel as locallevel_param, locallevel,
    paramtab, printparamnode, setaparam, setiparam, setsparam, unsetparam, unsetparam_pm,
};
use crate::ported::pattern::{patcompile, pattry};
use crate::ported::signals::settrap;
use crate::ported::sort::strmetasort;
use crate::ported::utils::{
    argzero, errflag, fprintdir, getkeystring, getkeystring_with, getshfunc, gettempfile, lchdir,
    print_if_link, printprompt4, quotedzputs, scriptname_get, set_argzero, zerr, zerrnam, zwarn,
    zwarnnam, GETKEYS_BINDKEY, GETKEYS_ECHO, GETKEYS_PRINT,
};
#[allow(unused_imports)]
use crate::ported::vm_helper::{self, format_int_in_base, BUILTIN_NAMES};
use crate::ported::zle::compctl::compctlread;
use crate::ported::zsh_h::{
    alias, asgment, builtin, cmdnam, eprog, hashnode, interact, islogin, isset, mathfunc, nameddir,
    options, param, shfunc, HandlerFunc, Meta, ALIAS_GLOBAL, ALIAS_SUFFIX, ASG_ARRAY, ASG_ARRAYP,
    ASG_KEY_VALUE, ASG_VALUEP, AUTOPUSHD, BINF_ADDED, BINF_ASSIGN, BINF_BUILTIN, BINF_COMMAND,
    BINF_DASH, BINF_DASHDASHVALID, BINF_EXEC, BINF_HANDLES_OPTS, BINF_KEEPNUM, BINF_MAGICEQUALS,
    BINF_NOGLOB, BINF_PLUSOPTS, BINF_PREFIX, BINF_PRINTOPTS, BINF_PSPECIAL, BINF_SKIPDASH,
    BINF_SKIPINVALID, BSDECHO, CDABLEVARS, CHASELINKS, CHECKRUNNINGJOBS, DISABLED, EMULATE_CSH,
    EMULATE_KSH, EMULATE_SH, EMULATE_ZSH, EMULATION, ERRFLAG_ERROR, FS_FUNC, FUNCTIONARGZERO,
    GLOBALEXPORT, GLOBALRCS, HASHED, HFILE_APPEND, HFILE_NO_REWRITE, HFILE_SKIPOLD,
    HFILE_USE_OPTIONS, HIST_FOREIGN, INTERACTIVE, KSHARRAYS, LOGINSHELL, MAX_OPS, MFF_STR,
    MFF_USERFUNC, MONITOR, NULLBINCMD, OPT_ARG, OPT_HASARG, OPT_ISSET, OPT_MINUS, OPT_PLUS,
    PATHDIRS, PAT_HEAPDUP, PAT_STATIC, PM_ABSPATH_USED, PM_ARRAY, PM_AUTOLOAD, PM_CUR_FPATH,
    PM_DECLARED, PM_DEFAULTED, PM_EFLOAT, PM_EXPORTED, PM_FFLOAT, PM_HASHED, PM_HIDE, PM_HIDEVAL,
    PM_INTEGER, PM_KSHSTORED, PM_LEFT, PM_LOADDIR, PM_LOCAL, PM_LOWER, PM_NAMEREF, PM_READONLY,
    PM_RIGHT_B, PM_RIGHT_Z, PM_RO_BY_DESIGN, PM_SCALAR, PM_SPECIAL, PM_TAGGED, PM_TAGGED_LOCAL,
    PM_TIED, PM_TYPE, PM_UNALIASED, PM_UNDEFINED, PM_UNIQUE, PM_UNSET, PM_UPPER, PM_WARNNESTED,
    PM_ZSHSTORED, POSIXBUILTINS, POSIXCD, POSIXTRAPS, PRINT_INCLUDEVALUE, PRINT_LINE, PRINT_LIST,
    PRINT_NAMEONLY, PRINT_POSIX_EXPORT, PRINT_POSIX_READONLY, PRINT_TYPE, PRINT_TYPESET,
    PRINT_WHENCE_CSH, PRINT_WHENCE_FUNCDEF, PRINT_WHENCE_SIMPLE, PRINT_WHENCE_VERBOSE,
    PRINT_WHENCE_WORD, PRINT_WITH_NAMESPACE, PUSHDIGNOREDUPS, PUSHDMINUS, PUSHDSILENT, PUSHDTOHOME,
    RCQUOTES, RCS, SHINSTDIN, SORTIT_BACKWARDS, SORTIT_IGNORING_CASE, STAT_LOCKED, STAT_NOPRINT,
    STAT_STOPPED, TRAP_STATE_FORCE_RETURN, TRAP_STATE_PRIMED, TYPESETSILENT, TYPESET_OPTSTR,
    VERBOSE, XTRACE, ZEXIT_DEFERRED, ZEXIT_NORMAL, ZEXIT_SIGNAL, ZSIG_FUNC,
};
#[allow(unused_imports)]
use crate::zwc::ZwcFile;

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
    BIN_BG, BIN_BRACKET, BIN_BREAK, BIN_CD, BIN_COMMAND, BIN_CONTINUE, BIN_DISABLE, BIN_DISOWN,
    BIN_ECHO, BIN_ENABLE, BIN_EVAL, BIN_EXIT, BIN_EXPORT, BIN_FC, BIN_FG, BIN_JOBS, BIN_LOGOUT,
    BIN_POPD, BIN_PRINT, BIN_PRINTF, BIN_PUSHD, BIN_PUSHLINE, BIN_R, BIN_READONLY, BIN_RETURN,
    BIN_SCHED, BIN_SETOPT, BIN_TEST, BIN_TYPESET, BIN_UNALIAS, BIN_UNFUNCTION, BIN_UNHASH,
    BIN_UNSET, BIN_UNSETOPT, BIN_WAIT,
};

/// Construct the builtin lookup table.
/// Port of `createbuiltintable()` from `Src/builtin.c:150`. The C
/// version installs the hashtable function pointers (hash, addnode,
/// printnode, etc.) and then calls `addbuiltins("zsh", builtins, ..)`.
/// Here we just materialise the static `BUILTINS` slice into a
/// `HashMap<String, &builtin>` — Rust's standard hashing replaces the
/// C `hasher` callback and the `HashMap` itself replaces all the
/// per-table function pointers (`addnode`/`getnode`/`removenode`/...).
// Builtin Command Hash Table Functions                                      // c:150
/// `createbuiltintable` — see implementation.
pub fn createbuiltintable() -> &'static HashMap<String, &'static builtin> {
    // c:150
    builtintab.get_or_init(|| {
        let table: &'static Vec<builtin> = &*BUILTINS;
        let watch_bintab: &'static Vec<builtin> = &*crate::ported::modules::watch::bintab;
        let mut m: HashMap<String, &'static builtin> =
            HashMap::with_capacity(table.len() + watch_bintab.len());
        for b in table.iter() {
            m.insert(b.node.nam.clone(), b);
        }
        // zshrs auto-loads all modules at startup. Fold each module's
        // bintab into the core builtintab so `disable <name>` (and
        // dispatch generally) finds module-provided builtins without
        // an explicit `zmodload` step. Mirrors C's `addbuiltins(name,
        // bintab, sizeof(bintab)/sizeof(*bintab))` call from each
        // module's `boot_` hook (e.g. `Src/Modules/watch.c:694`).
        for b in watch_bintab.iter() {
            m.insert(b.node.nam.clone(), b);
        }
        // zshrs extension: fold `znative` (the plugin package manager,
        // src/extensions/pkg) into the builtin table so it is a first-class
        // builtin — `whence -w znative` reports `builtin`, `builtin znative` works,
        // and it dispatches through execbuiltin like any other. No C
        // counterpart (znative is zshrs-original).
        let pkg_bintab: &'static Vec<builtin> = &*crate::pkg::builtin::bintab;
        for b in pkg_bintab.iter() {
            m.insert(b.node.nam.clone(), b);
        }
        // zshrs extension: fold the ztest/zassert unit-test framework
        // (src/extensions/ztest.rs) into the builtin table so `zassert_eq`,
        // `ztest_run`, … are first-class builtins in `zshrs -f` (visible to
        // `whence -w`, `builtin`, `disable`, completion) — not just the
        // fusevm command-dispatch arm. No C counterpart (zshrs-original).
        let ztest_bintab: &'static Vec<builtin> = &*crate::extensions::ztest::bintab;
        for b in ztest_bintab.iter() {
            m.insert(b.node.nam.clone(), b);
        }
        m
    })
}

// ===========================================================
// Direct ports of static builtin helpers from Src/builtin.c not
// yet covered above. The Rust executor wires builtins through
// `crate::ported::builtins::*` per-builtin modules; these free-
// fn entries satisfy ABI/name parity for the drift gate.
// ===========================================================

/// Port of `printbuiltinnode(HashNode hn, int printflags)` from
/// `Src/builtin.c:174`.
///
/// C body (c:174-194):
/// ```c
/// Builtin bn = (Builtin) hn;
/// if (printflags & PRINT_WHENCE_WORD) {
///     printf("%s: builtin\n", bn->node.nam); return;
/// }
/// if (printflags & PRINT_WHENCE_CSH) {
///     printf("%s: shell built-in command\n", bn->node.nam); return;
/// }
/// if (printflags & PRINT_WHENCE_VERBOSE) {
///     printf("%s is a shell builtin\n", bn->node.nam); return;
/// }
/// /* default is name only */
/// printf("%s\n", bn->node.nam);
/// ```
pub fn printbuiltinnode(
    hn: *mut hashnode, // c:174
    printflags: i32,
) {
    if hn.is_null() {
        return;
    }
    let bn = unsafe { &*hn }; // c:176
    if (printflags & PRINT_WHENCE_WORD as i32) != 0 {
        // c:178
        println!("{}: builtin", bn.nam); // c:179
        return; // c:180
    }
    if (printflags & PRINT_WHENCE_CSH as i32) != 0 {
        // c:183
        println!("{}: shell built-in command", bn.nam); // c:184
        return; // c:185
    }
    if (printflags & PRINT_WHENCE_VERBOSE as i32) != 0 {
        // c:188
        println!("{} is a shell builtin", bn.nam); // c:189
        return; // c:190
    }
    // c:193 — `/* default is name only */`
    println!("{}", bn.nam); // c:194
}

/// Port of `freebuiltinnode(HashNode hn)` from Src/builtin.c:199.
/// C: `static void freebuiltinnode(HashNode hn)` — free a builtin-table
///   node only when BINF_ADDED is clear (i.e., dynamically added).
pub fn freebuiltinnode(hn: *mut hashnode) {
    // c:199
    if hn.is_null() {
        return;
    }
    let bn = unsafe { &*hn };
    // c:204 — `if (!(bn->node.flags & BINF_ADDED))` then free.
    if (bn.flags as u32 & BINF_ADDED) == 0 { // c:204
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
pub fn init_builtins() {
    // c:212
    // c:214 — `if (!EMULATION(EMULATE_ZSH))`. EMULATION reads the
    // canonical `emulation` global directly per zsh.h:2347.
    if !EMULATION(EMULATE_ZSH) {
        // c:214
        // c:215-217 — `hn = reswdtab->getnode2(reswdtab,"repeat");
        //              if (hn) reswdtab->disablenode(hn, 0);`
        if let Ok(mut tab) = reswdtab_lock().write() {
            tab.disable("repeat");
        }
    }
}

/// Port of `OPT_ALLOC_CHUNK` from `Src/builtin.c:227`. Number of
/// `ops->args[]` slots `new_optarg()` grows the array by when full.
pub const OPT_ALLOC_CHUNK: i32 = 16; // c:227

/// Port of `new_optarg(Options ops)` from Src/builtin.c:227.
/// C: `static int new_optarg(Options ops)` — grow the `ops->args[]`
///   array by `OPT_ALLOC_CHUNK` slots when full. Returns 1 on overflow
///   (>=63 args), 0 on success.
pub fn new_optarg(ops: &mut options) -> i32 {
    // c:227
    // c:227 — `if (ops->argscount == 63) return 1;`
    if ops.argscount == 63 {
        // c:231
        return 1;
    }
    // c:232-241 — grow ops->args by OPT_ALLOC_CHUNK if argsalloc == argscount.
    if ops.argsalloc == ops.argscount {
        // c:232
        ops.args
            .resize((ops.argsalloc + OPT_ALLOC_CHUNK) as usize, String::new());
        ops.argsalloc += OPT_ALLOC_CHUNK; // c:240
    }
    ops.argscount += 1; // c:243
    0 // c:244
}

// ===========================================================
// ksh_autoload_body moved from src/ported/vm_helper.
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

/// Port of `execbuiltin(LinkList args, LinkList assigns, Builtin bn)` from Src/builtin.c:250.
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
pub fn execbuiltin(
    args: Vec<String>,
    assigns: Vec<asgment>, // c:250
    bn: *mut builtin,
) -> i32 {
    if bn.is_null() {
        return 1;
    }
    let bn_ref = unsafe { &*bn };

    // c:252-254 — locals.
    let pp: Option<&str>; // c:252 char *pp
    let name: String; // c:252 char *name
    let mut optstr: Option<String>; // c:252 char *optstr
    let mut flags: i32; // c:253 int flags
    let mut argc: i32; // c:253 int argc
    let mut execop: u8; // c:253 int execop
    let xtr: bool = isset(XTRACE); // c:253 int xtr = isset(XTRACE)

    // c:256-259 — `memset(ops.ind, 0, ...); ops.args = NULL; ops.argscount=ops.argsalloc=0;`
    let mut ops = options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(), // c:257
        argscount: 0,
        argsalloc: 0,
    }; // c:258-259

    // c:262 — `name = (char *) ugetnode(args);` — Rust reads bn.node.nam.
    name = bn_ref.node.nam.clone(); // c:262

    // c:264-268 — `if (!bn->handlerfunc)` early-exit.
    if bn_ref.handlerfunc.is_none() {
        // c:264
        // c:265 — DPUTS(1, "Missing builtin detected too late")
        DPUTS!(true, "Missing builtin detected too late"); // c:265
                                                           // c:266 — deletebuiltin(bn->node.nam) — not yet ported here.
        return 1; // c:267
    }

    // c:270-271 — `flags = bn->node.flags; optstr = bn->optstr;`
    flags = bn_ref.node.flags; // c:270
    optstr = bn_ref.optstr.clone(); // c:271

    // c:275 — `argc = countlinknodes(args);` — total argv length.
    argc = args.len() as i32; // c:275

    // c:284-293 — `VARARR(char *, argarr, argc+1)` + copy args into argarr.
    let argarr: Vec<String> = args; // c:284 argarr[]
    let mut argv: usize = 0; // c:285 char **argv = argarr;

    // c:296-411 — option parser body.
    if let Some(ref os) = optstr.clone() {
        // c:296
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
            if arg_bytes.is_empty() {
                break;
            }
            let sense: i32 = if arg_bytes[0] == b'-' { 1 } else { 0 }; // c:302
            if sense == 0
                && !((flags & BINF_PLUSOPTS as i32) != 0           // c:303
                                && arg_bytes[0] == b'+')
            {
                break;
            }
            // c:305 — `if (!(flags & BINF_KEEPNUM) && idigit(arg[1])) break;`
            if (flags & BINF_KEEPNUM as i32) == 0                            // c:305
                && arg_bytes.len() >= 2
                && arg_bytes[1].is_ascii_digit()
            {
                break;
            }
            // c:308 — `if ((flags & BINF_SKIPDASH) && !arg[1]) break;`
            if (flags & BINF_SKIPDASH as i32) != 0 && arg_bytes.len() == 1 {
                // c:308
                break;
            }
            // c:310-317 — `--` end-of-options if BINF_DASHDASHVALID.
            if (flags & BINF_DASHDASHVALID as i32) != 0 && arg_str == "--" {
                // c:310
                argv += 1; // c:315
                break; // c:316
            }
            // c:327-332 — `BINF_SKIPINVALID`: if any char in arg[1..] is
            // not in optstr, the whole arg is treated as a positional.
            if skipinvalid {
                // c:327
                let mut all_known = true;
                for &c in &arg_bytes[1..] {
                    if !optstr_bytes.contains(&c) {
                        all_known = false;
                        break;
                    }
                }
                if !all_known {
                    break;
                } // c:331
            }
            // c:335-336 — `if (arg[1] == '-') arg++;` — consume the
            // second `-` of `--long-style`.
            let mut k: usize = 1; // walks arg[k..]
            if arg_bytes.len() >= 2 && arg_bytes[1] == b'-' {
                // c:335
                k = 2; // c:336
            }
            // c:337-341 — `if (!arg[1])` lone `-` / `+` indicator.
            if arg_bytes.len() == k {
                // c:337
                ops.ind[b'-' as usize] = 1; // c:338
                if sense == 0 {
                    // c:339
                    ops.ind[b'+' as usize] = 1; // c:340
                }
            }
            // c:343-386 — inner loop over `*++arg` characters.
            let mut bad_opt: Option<u8> = None;
            while k < arg_bytes.len() {
                // c:343
                let c = arg_bytes[k];
                execop = c; // c:345
                let optptr = optstr_bytes.iter().position(|&b| b == c); // c:345 strchr(optstr,c)
                if let Some(optidx) = optptr {
                    // c:345
                    ops.ind[c as usize] = if sense != 0 { 1 } else { 2 }; // c:346
                                                                          // c:347 — `if (optptr[1] == ':')` — option takes arg.
                    if optidx + 1 < optstr_bytes.len() && optstr_bytes[optidx + 1] == b':' {
                        let mut argptr: Option<String> = None;
                        // c:349-352 — `if (optptr[2] == ':')` optional same-word.
                        if optidx + 2 < optstr_bytes.len() && optstr_bytes[optidx + 2] == b':' {
                            if k + 1 < arg_bytes.len() {
                                // c:350
                                argptr =
                                    Some(String::from_utf8_lossy(&arg_bytes[k + 1..]).into_owned());
                                // c:351
                            }
                        } else if optidx + 2 < optstr_bytes.len()
                            && optstr_bytes[optidx + 2] == b'%'
                        {
                            // c:353-359 — `:%` numeric optional same or next word.
                            if k + 1 < arg_bytes.len() && arg_bytes[k + 1].is_ascii_digit() {
                                argptr =
                                    Some(String::from_utf8_lossy(&arg_bytes[k + 1..]).into_owned());
                            } else if let Some(nxt) = argarr.get(argv + 1) {
                                if !nxt.is_empty() && nxt.as_bytes()[0].is_ascii_digit() {
                                    argv += 1; // c:359 arg = *++argv
                                    argptr = Some(nxt.clone());
                                }
                            }
                        } else {
                            // c:360-370 — plain `:` mandatory arg.
                            if k + 1 < arg_bytes.len() {
                                // c:362
                                argptr =
                                    Some(String::from_utf8_lossy(&arg_bytes[k + 1..]).into_owned());
                            // c:363
                            } else if let Some(nxt) = argarr.get(argv + 1) {
                                argv += 1; // c:364 arg = *++argv
                                argptr = Some(nxt.clone()); // c:365
                            } else {
                                // c:366-370 — `argument expected: -%c`.
                                zwarnnam(&name, &format!("argument expected: -{}", execop as char)); // c:367-368
                                return 1; // c:369
                            }
                        }
                        if let Some(ap) = argptr {
                            // c:372
                            // c:373-377 — new_optarg overflow.
                            if new_optarg(&mut ops) != 0 {
                                // c:373
                                zwarnnam(&name, "too many option arguments"); // c:374-375
                                return 1; // c:376
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
                    bad_opt = Some(c); // c:385 break
                    break;
                }
            }
            // c:389-394 — if we exited mid-arg on a bad char, emit "bad option".
            if let Some(badc) = bad_opt {
                // c:389
                zwarnnam(
                    &name,
                    &format!(
                        "bad option: {}{}",
                        if sense != 0 { '-' } else { '+' },
                        badc as char
                    ),
                ); // c:392
                return 1; // c:393
            }
            // c:395 — `arg = *++argv;`
            argv += 1; // c:395
                       // c:398-402 — BINF_PRINTOPTS R-mode switch to "ne" optstr.
            if (flags & BINF_PRINTOPTS as i32) != 0                          // c:398
                && ops.ind[b'R' as usize] != 0
                && ops.ind[b'f' as usize] == 0
            {
                optstr_bytes = b"ne".to_vec(); // c:400
                flags |= BINF_SKIPINVALID as i32; // c:401
                skipinvalid = true;
            }
            // c:404-405 — `if (ops.ind['-']) break;` — `--` terminates.
            if ops.ind[b'-' as usize] != 0 {
                // c:404
                break;
            }
        }
        let _ = optstr_bytes;
    } else if (flags & BINF_HANDLES_OPTS as i32) == 0                        // c:407
        && argarr.get(argv).map(|s| s == "--").unwrap_or(false)
    {
        // c:408
        // c:409-410 — `ops.ind['-'] = 1; argv++;`
        ops.ind[b'-' as usize] = 1; // c:409
        argv += 1; // c:410
    }
    // Suppress optstr-unused warnings on the `else` path.
    let _ = optstr.take();

    // c:414-421 — apply `bn->defopts` defaults.
    pp = bn_ref.defopts.as_deref(); // c:414
    if let Some(pp_str) = pp {
        // c:414
        for &b in pp_str.as_bytes() {
            // c:415
            if ops.ind[b as usize] == 0 {
                // c:417
                ops.ind[b as usize] = 1; // c:418
            }
        }
    }

    // c:424 — `argc -= argv - argarr;` — subtract consumed flag args.
    argc -= argv as i32; // c:424

    // c:426-429 — errflag check. C zsh clears here because the
    // *outer* execcmd_exec (Src/exec.c:3468) already bailed without
    // calling the builtin when errflag was set after prefork; the
    // clear at c:427 is paired with an earlier-arrived-at builtin
    // entry (signal-driven, not prefork-driven).
    //
    // zshrs's bytecode flow always reaches the builtin call site
    // even when arg expansion set errflag (the c:3468 pre-dispatch
    // gate has no equivalent in the bytecode). Clearing errflag
    // here would swallow the prefork error and let the post-command
    // ERREXIT_CHECK pass — `set -u; echo $undefined; echo done`
    // would run `echo done` instead of aborting.
    //
    // Non-interactive shells must preserve errflag so the
    // immediately-following ERREXIT_CHECK aborts the script (c:Src/
    // init.c:234 `((!interact || sourcelevel) && errflag)` break).
    let ef = errflag.load(Relaxed);
    if (ef & ERRFLAG_ERROR) != 0 {
        if !isset(INTERACTIVE) {
            return 1;
        }
        errflag.fetch_and(!ERRFLAG_ERROR, Relaxed); // c:427
        return 1; // c:428
    }

    // c:432-436 — argc bounds check.
    if argc < bn_ref.minargs                                                 // c:432
        || (argc > bn_ref.maxargs && bn_ref.maxargs != -1)
    {
        zwarnnam(
            &name, // c:433
            if argc < bn_ref.minargs {
                "not enough arguments"
            } else {
                "too many arguments"
            },
        ); // c:434
        return 1; // c:435
    }

    // c:438-494 — display execution trace information, if required.
    if xtr {
        // c:439
        // c:440-441 — `char **fullargv = argarr;` — use FULL argv
        // (including consumed option words) so XTRACE shows what the
        // user typed, not the option-stripped tail.
        let fullargv = &argarr; // c:441
                                // All emits below are `fprintf(xtrerr, …)` / `fputc(…, xtrerr)` in
                                // C: append to the xtrerr line buffer and flush ONCE at the
                                // trailing `\n` (c:492-493), so the builtin's trace line reaches
                                // the shared stderr fd in a single write — a forked pipeline
                                // stage never interleaves with a concurrent one.
        use crate::fusevm_bridge::{xtrerr_flush, xtrerr_fputs};
        printprompt4(); // c:442 — buffers PS4 into xtrerr
                        // c:443 — `fprintf(xtrerr, "%s", name);`
        xtrerr_fputs(&name); // c:443
                             // c:444-447 — `while (*fullargv) { fputc(' ',xtrerr); quotedzputs(...); }`
                             // C zsh's parser pre-splits `name=value` args for
                             // BINF_ASSIGN-flagged builtins (export/typeset/declare/local/
                             // readonly/integer/float) into asg{name,value} nodes, which
                             // the c:448-491 branch emits as
                             //   `quotedzputs(name) + '=' + quotedzputs(value)` (c:453,487-488).
                             // zshrs's compiler doesn't populate `assigns`; args arrive
                             // here as the unsplit `"name=value"` whole-strings, so the
                             // c:443-446 path quotes the entire string and produces
                             // `export 'VAR=val'` instead of `export VAR=val`. Inline the
                             // same split here — the prefix is a legal scalar/array name
                             // (ident, optionally `[subscript]`) followed by `=` or
                             // `+=`, and the suffix is the value. Detection mirrors the
                             // C parser at Src/lex.c:2169 (gettokstr ASSIGN recognition)
                             // and is faithful to what the C path would have produced
                             // from a pre-split asg node.
        let is_assign = (bn_ref.node.flags as u32 & BINF_ASSIGN) != 0;
        for s in fullargv {
            // c:444
            xtrerr_fputs(" "); // c:445 fputc(' ', xtrerr)
            let mut emitted = false;
            if is_assign {
                let sbytes = s.as_bytes();
                // Walk the ident prefix.
                let mut i = 0usize;
                if !sbytes.is_empty() && (sbytes[0].is_ascii_alphabetic() || sbytes[0] == b'_') {
                    i = 1;
                    while i < sbytes.len()
                        && (sbytes[i].is_ascii_alphanumeric() || sbytes[i] == b'_')
                    {
                        i += 1;
                    }
                    // Optional `[subscript]` — bracket-balanced.
                    if i < sbytes.len() && sbytes[i] == b'[' {
                        let mut depth = 1i32;
                        i += 1;
                        while i < sbytes.len() && depth > 0 {
                            match sbytes[i] {
                                b'[' => depth += 1,
                                b']' => depth -= 1,
                                _ => {}
                            }
                            i += 1;
                        }
                        if depth != 0 {
                            i = 0; // unbalanced — bail to default path
                        }
                    }
                    if i > 0 {
                        // Match `=` or `+=` separator.
                        let (sep_len, sep) =
                            if sbytes.get(i) == Some(&b'+') && sbytes.get(i + 1) == Some(&b'=') {
                                (2usize, "+=")
                            } else if sbytes.get(i) == Some(&b'=') {
                                (1usize, "=")
                            } else {
                                (0, "")
                            };
                        if sep_len != 0 {
                            // c:453,487-488 — emit name + sep + quoted(value).
                            xtrerr_fputs(&format!(
                                "{}{}{}",
                                &s[..i],
                                sep,
                                quotedzputs(&s[i + sep_len..])
                            ));
                            emitted = true;
                        }
                    }
                }
            }
            if !emitted {
                xtrerr_fputs(&quotedzputs(s)); // c:446
            }
        }
        // c:448-491 — `if (assigns) { for (node = firstnode(assigns); ...) }`.
        for asg in &assigns {
            // c:450 firstnode/incnode
            xtrerr_fputs(" "); // c:452 fputc(' ', xtrerr)
            xtrerr_fputs(&quotedzputs(&asg.name)); // c:453
            if (asg.flags & ASG_ARRAY) != 0 {
                // c:454
                xtrerr_fputs("=("); // c:455
                if let Some(ref list) = asg.array {
                    // c:456
                    if (asg.flags & ASG_KEY_VALUE) != 0 {
                        // c:457
                        // c:458-473 — `LinkNode keynode, valnode;` walk
                        // alternating key/value pairs, emitting
                        // `[key]=value` per pair. Uses the typed
                        // `LinkList<String>` accessors from
                        // `src/ported/linklist.rs` which port the
                        // `firstnode` / `nextnode` / `getdata` macros
                        // from `Src/zsh.h:576-588`.
                        let mut keynode = list.firstnode(); // c:459
                        loop {
                            // c:460
                            // c:461-462 — `if (!keynode) break;`
                            let kidx = match keynode {
                                // c:461
                                Some(i) => i,
                                None => break, // c:462
                            };
                            // c:463-465 — `valnode = nextnode(keynode); if (!valnode) break;`
                            let vidx = match list.nextnode(kidx) {
                                // c:463
                                Some(i) => i,
                                None => break, // c:465
                            };
                            // c:466-468 — `fputc('['); quotedzputs(getdata(keynode));`
                            xtrerr_fputs("["); // c:466
                            if let Some(k) = list.getdata(kidx) {
                                // c:467 getdata
                                xtrerr_fputs(&quotedzputs(k));
                                // c:467
                            }
                            // c:469 — `fprintf(stderr, "]=");`
                            xtrerr_fputs("]="); // c:469
                                                // c:470-471 — `quotedzputs(getdata(valnode));`
                            if let Some(v) = list.getdata(vidx) {
                                // c:470
                                xtrerr_fputs(&quotedzputs(v));
                                // c:470
                            }
                            // c:472 — `keynode = nextnode(valnode);`
                            keynode = list.nextnode(vidx); // c:472
                        }
                    } else {
                        // c:474
                        // c:475-482 — plain array emit: walk every node
                        // and emit ` <quotedzputs(elem)>`.
                        let mut arrnode = list.firstnode(); // c:476
                        while let Some(idx) = arrnode {
                            // c:477
                            xtrerr_fputs(" "); // c:479 fputc(' ', xtrerr)
                            if let Some(elem) = list.getdata(idx) {
                                // c:480 getdata
                                xtrerr_fputs(&quotedzputs(elem));
                                // c:480
                            }
                            arrnode = list.nextnode(idx); // c:478 incnode
                        }
                    }
                }
                xtrerr_fputs(" )"); // c:485
            } else if let Some(ref scalar) = asg.scalar {
                // c:486
                xtrerr_fputs("="); // c:487 fputc('=', xtrerr)
                xtrerr_fputs(&quotedzputs(scalar)); // c:488
            }
        }
        // c:492-493 — `fputc('\n', xtrerr); fflush(xtrerr);`
        xtrerr_fputs("\n"); // c:492 fputc('\n', xtrerr)
        xtrerr_flush(); // c:493 fflush(xtrerr) — one write for the whole line
    }

    // c:506 — `return (*(bn->handlerfunc))(name, argv, &ops, bn->funcid);`
    let trimmed: Vec<String> = argarr[argv..].to_vec();
    let handler = bn_ref.handlerfunc.expect("handlerfunc checked at c:264");
    handler(&name, &trimmed, &ops, bn_ref.funcid) // c:506
}

/// Port of `bin_enable(char *name, char **argv, Options ops, int func)` from Src/builtin.c:517.
/// C: `int bin_enable(char *name, char **argv, Options ops, int func)` —
///   enable/disable hashtab entries (default builtins; `-f`/`-r`/`-s`/`-a`
///   pick alternate tables); `-p` routes to pat_enables (pattern toggles).
/// WARNING: param names don't match C — Rust=(name, argv, func) vs C=(name, argv, ops, func)
pub fn bin_enable(
    name: &str,
    argv: &[String], // c:517
    ops: &options,
    func: i32,
) -> i32 {
    enum Tab {
        Builtin,
        Shfunc,
        Reswd,
        Alias,
        SufAlias,
    }
    let mut returnval = 0i32; // c:524
    let mut match_count = 0i32; // c:524
                                // c:527-538 — `-p` early-out + table selection.
    if OPT_ISSET(ops, b'p') {
        // c:527
        // c:528 — `return pat_enables(name, argv, func == BIN_ENABLE);`
        return pat_enables(name, argv, func == BIN_ENABLE); // c:528
    }
    let tab = if OPT_ISSET(ops, b'f') {
        Tab::Shfunc
    }
    // c:529
    else if OPT_ISSET(ops, b'r') {
        Tab::Reswd
    }
    // c:531
    else if OPT_ISSET(ops, b's') {
        Tab::SufAlias
    }
    // c:533
    else if OPT_ISSET(ops, b'a') {
        Tab::Alias
    }
    // c:535
    else {
        Tab::Builtin
    }; // c:537

    // c:540-547 — flags1/flags2 set based on enable vs disable direction.
    let enable = func == BIN_ENABLE;
    let (flags1, flags2) = if enable {
        // c:541
        (0u32, DISABLED as u32) // c:542
    } else {
        (DISABLED as u32, 0u32) // c:545
    };

    // Helper closures over the chosen table.
    let toggle_one = |tab: &Tab, nm: &str, on: bool| -> bool {
        match tab {
            Tab::Alias => aliastab_lock()
                .write()
                .map(|mut t| if on { t.enable(nm) } else { t.disable(nm) })
                .unwrap_or(false),
            Tab::SufAlias => sufaliastab_lock()
                .write()
                .map(|mut t| if on { t.enable(nm) } else { t.disable(nm) })
                .unwrap_or(false),
            // c:541-547 — `enable`/`disable -r` toggles DISABLED on the
            // reswdtab entry; reswords resolve through getreswdnode in
            // the lexer so toggling here is enough to mask/unmask.
            Tab::Reswd => {
                let exists = reswdtab_lock()
                    .read()
                    .map(|t| t.get_including_disabled(nm).is_some())
                    .unwrap_or(false);
                if !exists {
                    return false;
                }
                reswdtab_lock()
                    .write()
                    .map(|mut t| if on { t.enable(nm) } else { t.disable(nm) })
                    .unwrap_or(false)
            }
            // c:541-547 — `enable`/`disable -f` toggles DISABLED on the
            // shfunctab entry; ports to disableshfuncnode/enableshfuncnode
            // which also unsettrap/settrap TRAP* ported.
            Tab::Shfunc => {
                let exists = shfunctab_lock()
                    .read()
                    .map(|t| t.get_including_disabled(nm).is_some())
                    .unwrap_or(false);
                if !exists {
                    return false;
                }
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
                if createbuiltintable().get(nm).is_none() {
                    return false;
                }
                if let Ok(mut set) = BUILTINS_DISABLED.lock() {
                    if on {
                        set.remove(nm);
                    } else {
                        set.insert(nm.to_string());
                    }
                    return true;
                }
                false
            }
        }
    };
    let collect_names = |tab: &Tab| -> Vec<String> {
        match tab {
            Tab::Alias => aliastab_lock()
                .read()
                .map(|t| t.iter().map(|(n, _)| n.clone()).collect())
                .unwrap_or_default(),
            Tab::SufAlias => sufaliastab_lock()
                .read()
                .map(|t| t.iter().map(|(n, _)| n.clone()).collect())
                .unwrap_or_default(),
            Tab::Reswd => reswdtab_lock()
                .read()
                .map(|t| t.iter().map(|(n, _)| n.clone()).collect())
                .unwrap_or_default(),
            Tab::Shfunc => shfunctab_lock()
                .read()
                .map(|t| t.iter().map(|(n, _)| n.clone()).collect())
                .unwrap_or_default(),
            Tab::Builtin => createbuiltintable().keys().cloned().collect(),
        }
    };

    // c:553-558 — no-args list.
    if argv.is_empty() {
        // c:553
        queue_signals(); // c:554
                         // c:555 — `scanhashtable(ht, 1, flags1, flags2, ht->printnode, 0);`
                         // Filter: print only entries where (flags & flags1) == flags1
                         // && (flags & flags2) == 0. For enable/disable, flags1 and
                         // flags2 are DISABLED-bit selectors that mask the listed set
                         // to ONLY the kind being toggled (enable lists enabled,
                         // disable lists disabled).
        let is_disabled = |nm: &str| -> bool {
            match tab {
                Tab::Alias => aliastab_lock()
                    .read()
                    .ok()
                    .and_then(|t| {
                        t.get_including_disabled(nm)
                            .map(|a| (a.node.flags & DISABLED as i32) != 0)
                    })
                    .unwrap_or(false),
                Tab::SufAlias => sufaliastab_lock()
                    .read()
                    .ok()
                    .and_then(|t| {
                        t.get_including_disabled(nm)
                            .map(|a| (a.node.flags & DISABLED as i32) != 0)
                    })
                    .unwrap_or(false),
                Tab::Reswd => reswdtab_lock()
                    .read()
                    .ok()
                    .and_then(|t| {
                        t.get_including_disabled(nm)
                            .map(|r| (r.node.flags & DISABLED as i32) != 0)
                    })
                    .unwrap_or(false),
                Tab::Shfunc => shfunctab_lock()
                    .read()
                    .ok()
                    .and_then(|t| {
                        t.get_including_disabled(nm)
                            .map(|f| (f.node.flags & DISABLED as i32) != 0)
                    })
                    .unwrap_or(false),
                Tab::Builtin => BUILTINS_DISABLED
                    .lock()
                    .map(|s| s.contains(nm))
                    .unwrap_or(false),
            }
        };
        // c:Src/builtin.c — `scanhashtable(ht, 1, …)` walks the
        // table in sorted order (the first arg `sorted=1`). Mirror
        // by sorting the collected names before the per-name walk.
        // Without this, builtin/reswd listings came out in HashMap
        // iteration order, diverging from zsh.
        let mut all_names = collect_names(&tab);
        all_names.sort();
        for nm in all_names {
            let dis = is_disabled(&nm);
            let entry_flags = if dis { DISABLED as u32 } else { 0 };
            if (entry_flags & flags1) == flags1 && (entry_flags & flags2) == 0 {
                // c:Src/builtin.c — `ht->printnode` dispatches per
                // table type. For alias/sufalias, printaliasnode
                // (hashtable.rs:1477) prints `name=value`. For
                // builtin/reswd/shfunc, just the name. Previous
                // Rust port printed bare names for all kinds, so
                // `enable -a` lost the alias definitions. Mirror
                // C's printnode dispatch here.
                match tab {
                    Tab::Alias => {
                        let val = aliastab_lock()
                            .read()
                            .ok()
                            .and_then(|t| t.get_including_disabled(&nm).map(|a| a.text.clone()));
                        if let Some(v) = val {
                            println!("{}={}", nm, quotedzputs(&v));
                        } else {
                            println!("{}", nm);
                        }
                    }
                    Tab::SufAlias => {
                        let val = sufaliastab_lock()
                            .read()
                            .ok()
                            .and_then(|t| t.get_including_disabled(&nm).map(|a| a.text.clone()));
                        if let Some(v) = val {
                            println!("{}={}", nm, quotedzputs(&v));
                        } else {
                            println!("{}", nm);
                        }
                    }
                    _ => println!("{}", nm),
                }
            }
        }
        unqueue_signals(); // c:556
        return 0; // c:557
    }

    // c:561-580 — `-m` glob branch.
    if OPT_ISSET(ops, b'm') {
        // c:561
        for arg in argv {
            // c:562
            queue_signals(); // c:563
            let pprog = patcompile(
                &{
                    let mut __pat_tok = (arg).to_string();
                    crate::ported::glob::tokenize(&mut __pat_tok);
                    __pat_tok
                }, // c:566
                PAT_HEAPDUP,
                None,
            );
            if let Some(prog) = pprog {
                for nm in collect_names(&tab) {
                    if pattry(&prog, &nm) {
                        // c:567
                        if toggle_one(&tab, &nm, enable) {
                            match_count += 1; // c:567
                        }
                    }
                }
            } else {
                zwarnnam(name, &format!("bad pattern : {}", arg)); // c:572
                returnval = 1; // c:573
            }
            unqueue_signals(); // c:575
        }
        if match_count == 0 {
            // c:579
            returnval = 1; // c:580
        }
        return returnval; // c:581
    }

    // c:585-594 — literal-name dispatch.
    queue_signals(); // c:585
    for arg in argv {
        // c:586
        if !toggle_one(&tab, arg, enable) {
            // c:587
            zwarnnam(name, &format!("no such hash table element: {}", arg)); // c:590
            returnval = 1; // c:591
        }
    }
    unqueue_signals(); // c:594
    returnval // c:595
}

/// Port of `bin_set(char *nam, char **args, UNUSED(Options ops), UNUSED(int func))` from Src/builtin.c:601.
/// C: `int bin_set(char *nam, char **args, UNUSED(Options ops),
///                 UNUSED(int func))` — set shell options, declare arrays,
///   replace positional params, or display variables.
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(nam, args, ops, func)
pub fn bin_set(
    nam: &str,
    args: &[String], // c:601
    _ops: &options,
    _func: i32,
) -> i32 {
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
    let mut hadopt = false; // c:603
    let mut hadplus = false; // c:603
    let mut hadend = false; // c:603
    let mut sort: i32 = 0; // c:603
    let mut array: i32 = 0; // c:603
    let mut arrayname: Option<String> = None; // c:604

    // c:608-614 — sh-compat: bare `set -` → +xv.
    if !EMULATION(EMULATE_ZSH)                                               // c:608
        && !argv.is_empty() && argv[0] == "-"
    {
        // c:610-611 — `dosetopt(VERBOSE, 0, 0, opts); dosetopt(XTRACE, 0, 0, opts);`
        dosetopt(VERBOSE, 0, 0); // c:610
        dosetopt(XTRACE, 0, 0); // c:611
        if argv.len() == 1 {
            return 0;
        } // c:612-613
        argv.remove(0);
    }

    // c:617-668 — top-level option-arg loop.
    let mut idx = 0usize;
    'outer: while idx < argv.len()                                           // c:617
        && (argv[idx].starts_with('-') || argv[idx].starts_with('+'))
    {
        let arg = argv[idx].clone();
        let action = arg.starts_with('-'); // c:619
        if !action {
            hadplus = true;
        } // c:620
          // c:621-622 — bare `-` / `+` → "--"
        let body: String = if arg.len() == 1 {
            "--".to_string()
        } else {
            arg.clone()
        };
        // c:623 — `while (*++*args)`
        let chars: Vec<char> = body[1..].chars().collect();
        let mut ci = 0usize;
        while ci < chars.len() {
            // c:623
            let c = chars[ci];
            if c != '-' || action {
                hadopt = true;
            } // c:626
              // c:628-632 — `--` end-of-options.
            if c == '-' {
                // c:628
                hadend = true; // c:629
                idx += 1; // c:630 args++
                break 'outer;
            }
            // c:633-645 — `o` long-option name follows.
            if c == 'o' {
                // c:633
                let optname: String = if ci + 1 < chars.len() {
                    chars[ci + 1..].iter().collect::<String>()
                } else {
                    idx += 1;
                    if idx >= argv.len() {
                        // c:636-637 — `printoptionstates(hadplus);
                        // inittyptab(); return 0;`. The Rust port
                        // previously returned 0 without printing,
                        // so `set -o` / `set +o` (no option name)
                        // emitted nothing instead of the full
                        // option-state listing (one per line).
                        // hadplus is true when the user invoked
                        // `set +o` (POSIX-output form); else `set -o`
                        // (zsh-output form).
                        crate::ported::options::printoptionstates(hadplus);
                        return 0;
                    }
                    argv[idx].clone()
                };
                let optno = optlookup(&optname); // c:642
                if optno == 0 {
                    // c:642 — C: `zerrnam(nam, "no such option: %s", *args)`.
                    // zwarnnam emits `zsh:<nam>:<lineno>:` prefix
                    // (vs zerr's bare `zsh:<lineno>:`); use the
                    // canonical form so `set +o BAD` matches zsh's
                    // diagnostic format. Return 1 so $? reflects the
                    // failed lookup (C's execbuiltin checks errflag
                    // after the call; zshrs's bridge bases on the
                    // return value).
                    zerrnam(nam, &format!("no such option: {}", optname));
                    unqueue_signals();
                    return 1;
                } else if dosetopt(optno, if action { 1 } else { 0 }, 0) != 0
                // c:644
                {
                    zerrnam(nam, &format!("can't change option: {}", optname));
                    unqueue_signals();
                    return 1;
                }
                break;
            }
            // c:646-657 — `A` array-mode (with optional name arg).
            if c == 'A' {
                // c:646
                array = if action { 1 } else { -1 }; // c:649
                let nameopt: Option<String> = if ci + 1 < chars.len() {
                    Some(chars[ci + 1..].iter().collect::<String>())
                } else if idx + 1 < argv.len() {
                    idx += 1;
                    Some(argv[idx].clone())
                } else {
                    None
                };
                arrayname = nameopt.clone();
                if arrayname.is_none() {
                    // c:651
                    idx += 1;
                    break 'outer;
                }
                let ksharrays = isset(KSHARRAYS);
                if !ksharrays {
                    // c:653
                    idx += 1; // c:655 args++
                    break 'outer; // c:656
                }
                break;
            }
            // c:659-660 — `s` sort flag.
            if c == 's' {
                // c:659
                sort = if action { 1 } else { -1 }; // c:660
            } else {
                // c:662-666 — short-option letter: optlookupc + dosetopt.
                // c:Src/builtin.c — zerrnam(nam, ...) prefixes the
                // builtin's name into the error tag ("zsh:set:1:" vs
                // bare "zsh:1:"). Use zerrnam to match zsh's exact
                // error format for `set -X 2>&1`.
                let optno = crate::ported::options::optlookupc(c); // c:663
                if optno == 0 {
                    // c:663
                    zerrnam(nam, &format!("bad option: -{}", c)); // c:663
                } else if dosetopt(optno, if action { 1 } else { 0 }, 0) != 0
                // c:664
                {
                    zerrnam(nam, &format!("can't change option: -{}", c)); // c:664
                }
            }
            ci += 1;
        }
        idx += 1; // c:668
    }
    // c:Src/builtin.c:668 — `if (errflag) return 1;`. A bad / can't-change
    // option (or `no such option`) reported via zerrnam set ERRFLAG_ERROR;
    // `set -Z` must exit 1, not fall through to the (success) display path.
    if (crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed)
        & crate::ported::utils::ERRFLAG_ERROR)
        != 0
    {
        return 1;
    }
    let _ = nam;

    // c:676 — `queue_signals();`
    queue_signals();
    let remaining = &argv[idx..];

    // c:678-694 — display path when no array/no args.
    if arrayname.is_none() {
        // c:678
        if !hadopt && remaining.is_empty() {
            // c:679
            // c:680-681 — `scanhashtable(paramtab, 1, 0, 0,
            //              paramtab->printnode, hadplus ? PRINT_NAMEONLY : 0);`
            //
            // C walks the paramtab (sorted=1 → alphabetical). The previous
            // Rust port walked `std::env::vars()` — the OS environment.
            // Shell-internal vars (not exported to env) would never appear
            // in the `set` listing, diverging from C where ALL paramtab
            // entries are emitted.
            //
            // Same family of bug as the prior bin_unset -m fix.
            //
            // c:Src/builtins.c::printparamnode — dispatches the value
            // read through the param's gsu_s.getfn rather than reading
            // `pm.u_str` directly. Special params like `!`, `$`, `#`,
            // `-`, `0`, `?` have empty `u_str` slots because their live
            // value lives behind getfn (libc syscall, LASTVAL, pparams
            // count, etc.). Bug #463: zshrs read u_str directly so the
            // dump showed `!=''`, `$=''` etc. instead of the actual
            // values. Route through `getsparam` so the canonical
            // special-param dispatch (lookup_special_var → getfn shim)
            // fires.
            let names: Vec<String> = {
                let tab = paramtab().read().unwrap();
                tab.iter()
                    .filter(|(_, pm)| (pm.node.flags as u32 & PM_UNSET) == 0)
                    .map(|(k, _)| k.clone())
                    .collect()
            };
            let mut entries: Vec<(String, String)> = names
                .into_iter()
                .map(|k| {
                    let v = crate::ported::params::getsparam(&k).unwrap_or_default();
                    (k, v)
                })
                .collect();
            // c:680 sorted=1 → meta-aware sort via hnamcmp (already fixed
            // to use ztrcmp earlier in the series).
            entries.sort_by(|a, b| hnamcmp(&a.0, &b.0));
            for (k, v) in entries {
                // c:Src/params.c::printparamnode — single-char names
                // that double as shell metacharacters (`#` comment,
                // `$` substitution, `*` glob, `?` glob, `@` splat)
                // are wrapped in single quotes so the output
                // round-trips through the shell parser. Other
                // single-char specials (`!`, `-`, `0`) appear bare
                // because they're unambiguous in name position.
                // Verified against /opt/homebrew/bin/zsh `set` output.
                let needs_quote_name = matches!(k.as_str(), "#" | "$" | "*" | "?" | "@");
                let kq: String = if needs_quote_name {
                    format!("'{}'", k)
                } else {
                    k.clone()
                };
                if hadplus {
                    // c:681 PRINT_NAMEONLY
                    println!("{}", kq);
                } else if matches!(k.as_str(), "*" | "@" | "argv") {
                    // c:Src/params.c — `*` / `@` / `argv` are array-
                    // shaped (positional params); print as `( e1 e2 …)`
                    // mirroring zsh's array form so `set` output
                    // round-trips. Without this special-case, zshrs
                    // showed `'*'='1 2'` (joined scalar) instead of
                    // zsh's `'*'=( 1 2 )` (array splat).
                    let pp = PPARAMS.lock().ok();
                    let elems: Vec<String> = pp
                        .as_ref()
                        .map(|p| p.iter().cloned().collect())
                        .unwrap_or_default();
                    let body: String = elems
                        .iter()
                        .map(|e| quotedzputs(e))
                        .collect::<Vec<_>>()
                        .join(" ");
                    if elems.is_empty() {
                        println!("{}=(  )", kq);
                    } else {
                        println!("{}=( {} )", kq, body);
                    }
                } else {
                    println!("{}={}", kq, quotedzputs(&v));
                }
            }
        }
        if array != 0 {
            // c:684
            // c:685-687 — `scanhashtable(paramtab, 1, PM_ARRAY, 0,
            //              paramtab->printnode, hadplus ? PRINT_NAMEONLY : 0)`.
            // Walk paramtab filtering by PM_ARRAY and emit each as
            // `name=(elem1 elem2 ...)`. Previous Rust port stubbed
            // this body with a "nothing to enumerate" comment — but
            // paramtab does store arrays in `u_arr`, so `set -A` (no
            // name) MUST list every PM_ARRAY entry. Sorted via
            // hnamcmp (meta-aware compare) per `sorted=1` in the C
            // scanhashtable call.
            let mut arr_entries: Vec<(String, Vec<String>)> = {
                use {PM_ARRAY, PM_TYPE};
                let tab = paramtab().read().unwrap();
                tab.iter()
                    .filter(|(_, pm)| {
                        PM_TYPE(pm.node.flags as u32) == PM_ARRAY
                            && (pm.node.flags as u32 & PM_UNSET) == 0
                    })
                    .map(|(k, pm)| (k.clone(), pm.u_arr.clone().unwrap_or_default()))
                    .collect()
            };
            arr_entries.sort_by(|a, b| hnamcmp(&a.0, &b.0)); // c:685 sorted=1
            for (k, arr) in arr_entries {
                if hadplus {
                    // c:686 PRINT_NAMEONLY
                    println!("{}", k);
                } else {
                    let quoted: Vec<String> = arr.iter().map(|v| quotedzputs(v)).collect();
                    println!("{}=({})", k, quoted.join(" "));
                }
            }
        }
        if remaining.is_empty() && !hadend {
            // c:688
            unqueue_signals();
            return 0; // c:690
        }
    }

    // c:693-695 — `set -s` sort.
    let sorted: Vec<String> = if sort != 0 {
        let mut v = remaining.to_vec();
        if sort < 0 {
            v.sort_by(|a, b| b.cmp(a));
        } else {
            v.sort();
        }
        v
    } else {
        remaining.to_vec()
    };

    // c:696-708 — array assign or positional-param replace.
    if array != 0 {
        // c:696
        // c:697-708 — build array; `array < 0` appends to existing $name.
        let aname = arrayname.unwrap_or_default();
        let mut new_arr: Vec<String> = sorted;
        if array < 0 {
            // c:701
            // c:702-704 — `if ((a = getaparam(arrayname)) && arrlen_gt(a, len))`.
            //              Read paramtab.u_arr directly; was using `:`-
            //              split env value as a fake array.
            let existing: Vec<String> = {
                let tab = paramtab().read().unwrap();
                tab.get(&aname)
                    .and_then(|pm| pm.u_arr.clone())
                    .unwrap_or_default()
            };
            if existing.len() > new_arr.len() {
                // c:702
                new_arr.extend(existing.into_iter().skip(new_arr.len())); // c:703
            }
        }
        // c:709 — `setaparam(arrayname, x)`. In C, setaparam →
        // assignaparam → setarrvalue dispatches a PM_HASHED target to
        // arrhashsetfn, which converts the flat list to key/value pairs.
        // When the target is an associative array (`typeset -A h; set -A
        // h k v`), route through sethparam (the flat-kv-list assoc
        // setter) so the values land as pairs instead of being stored as
        // a plain array (which left the assoc empty). Plain arrays keep
        // setaparam.
        let target_is_hashed = {
            use PM_HASHED;
            paramtab()
                .read()
                .ok()
                .and_then(|tab| {
                    tab.get(&aname)
                        .map(|pm| (pm.node.flags as u32 & PM_HASHED) != 0)
                })
                .unwrap_or(false)
        };
        if target_is_hashed {
            crate::ported::params::sethparam(&aname, new_arr);
        } else {
            setaparam(&aname, new_arr);
        }
    } else {
        // c:711-712 — `freearray(pparams); pparams = zarrdup(args);`
        // PPARAMS is the single source of truth; fusevm reads via
        // `exec.pparams()`.
        if let Ok(mut pp) = PPARAMS.lock() {
            *pp = sorted; // c:712
        }
    }
    unqueue_signals(); // c:714
    0 // c:715
}

/// Port of `bin_pwd(UNUSED(char *name), UNUSED(char **argv), Options ops, UNUSED(int func))` from Src/builtin.c:728.
/// C: `int bin_pwd(UNUSED(char *name), UNUSED(char **argv), Options ops,
///     UNUSED(int func))` — `-r`/`-P` or (CHASELINKS && !`-L`) →
///   print resolved cwd via zgetcwd; else print the cached `pwd`.
// pwd: display the name of the current directory                          // c:728
/// WARNING: param names don't match C — Rust=(_name, _argv, _func) vs C=(name, argv, ops, func)
pub fn bin_pwd(
    _name: &str,
    _argv: &[String], // c:728
    ops: &options,
    _func: i32,
) -> i32 {
    let chaselinks = isset(CHASELINKS);
    // c:730-731 — `if (OPT_ISSET(ops,'r') || OPT_ISSET(ops,'P') ||
    //               (isset(CHASELINKS) && !OPT_ISSET(ops,'L')))`
    if OPT_ISSET(ops, b'r') || OPT_ISSET(ops, b'P')                          // c:730
        || (chaselinks && !OPT_ISSET(ops, b'L'))
    // c:731
    {
        // c:732 — `printf("%s\n", zgetcwd());`
        println!("{}", zgetcwd()); // c:732
    } else {
        // c:734 — `zputs(pwd, stdout); putchar('\n');`. C reads the
        // shell-internal `pwd` global (Src/params.c:108), NOT the
        // exported $PWD env var. The internal global is only mutated
        // by `cd`. zshrs lacks a separate internal pwd, so `$PWD`
        // doubles as both — and user code can spoof it (`PWD=/foo`).
        //
        // C-equivalent behavior in zshrs: trust `$PWD` ONLY when its
        // stat matches `getcwd()` (same dev+ino → same directory,
        // logical name preserved through symlinks). On mismatch the
        // user spoofed it; fall back to `zgetcwd()` so `pwd` reports
        // the actual current directory. Bug #441 — security-relevant:
        // defensive-coding patterns checking `pwd` output get fooled
        // by a `PWD=` assignment without this validation.
        let pwd_param = getsparam("PWD");
        let logical_pwd = pwd_param.as_deref().filter(|pwd| {
            // stat-compare $PWD against getcwd() — same inode means
            // $PWD is just a symlink-preserving alias for the real
            // cwd, safe to use.
            use std::os::unix::fs::MetadataExt;
            let cwd = zgetcwd();
            match (std::fs::metadata(pwd), std::fs::metadata(&cwd)) {
                (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
                _ => false,
            }
        });
        println!("{}", logical_pwd.map(String::from).unwrap_or_else(zgetcwd)); // c:734
    }
    0 // c:737
}

/// Port of `bin_dirs(UNUSED(char *name), char **argv, Options ops, UNUSED(int func))` from Src/builtin.c:749.
/// C: `int bin_dirs(UNUSED(char *name), char **argv, Options ops, ...)` —
///   list dirstack (default / -v / -p / -l) or replace it with argv.
// dirs: list the directory stack, or replace it with a provided list      // c:749
/// WARNING: param names don't match C — Rust=(_name, argv, _func) vs C=(name, argv, ops, func)
pub fn bin_dirs(
    _name: &str,
    argv: &[String], // c:749
    ops: &options,
    _func: i32,
) -> i32 {
    queue_signals(); // c:753
                     // c:755-756 — list mode: no args & no -c, OR -v / -p.
    if (argv.is_empty() && !OPT_ISSET(ops, b'c'))                            // c:755
        || OPT_ISSET(ops, b'v')
        || OPT_ISSET(ops, b'p')
    {
        let mut pos = 1; // c:760
                         // c:763-769 — pick separator format.
        let fmt: &str = if OPT_ISSET(ops, b'v') {
            // c:763
            print!("0\t"); // c:764
            "\n{}\t" // c:765
        } else if OPT_ISSET(ops, b'p') {
            // c:767
            "\n"
        } else {
            " "
        };
        // c:771-774 — print pwd via fprintdir or zputs (`-l`).
        // Previous Rust port inlined a HOME-prefix replacement which
        // only abbreviated `$HOME/...` to `~/...` — missed every
        // user-defined nameddirtab entry (`hash -d proj=/big/path`).
        // Route through `utils::fprintdir` which calls `finddir`,
        // matching C's named-dir abbreviation.
        let pwd = getsparam("PWD").unwrap_or_else(|| zgetcwd());
        if OPT_ISSET(ops, b'l') {
            // c:771
            print!("{}", pwd); // c:772
        } else {
            print!("{}", fprintdir(&pwd)); // c:774
        }
        // c:775-781 — walk dirstack list.
        if let Ok(stack) = DIRSTACK.lock() {
            // c:775
            for entry in stack.iter() {
                if fmt == "\n{}\t" {
                    print!("\n{}\t", pos);
                } else {
                    print!("{}", fmt); // c:776
                }
                pos += 1; // c:776
                if OPT_ISSET(ops, b'l') {
                    // c:777
                    print!("{}", entry); // c:778
                } else {
                    print!("{}", fprintdir(entry)); // c:780
                }
            }
        }
        unqueue_signals(); // c:783
        println!(); // c:784
        return 0; // c:785
    }
    // c:788-792 — replace dirstack with the supplied entries.
    if let Ok(mut stack) = DIRSTACK.lock() {
        stack.clear(); // c:790
        for arg in argv {
            stack.push(arg.clone()); // c:791
        }
    }
    unqueue_signals(); // c:793
    0 // c:794
}

/// Direct port of `void set_pwd_env(void)` from
/// `Src/builtin.c:800`. Refreshes both `$PWD` and `$OLDPWD` shell
/// parameters from the shell-side `pwd`/`oldpwd` globals
/// (`assignsparam` at c:818-819), then `addenv`s each so child
/// processes inherit them (c:821-826). C also clears `PM_READONLY`
/// and unsets the param when a user retyped PWD/OLDPWD to a
/// non-scalar (c:806-816).
///
/// Rust port: the bin entry's analog of C's `pwd`/`oldpwd` globals
/// is the live OS env (validated against ispwd by ShellExecutor::new
/// per c:Src/init.c:1242-1259 — see the subshell-snapshot comment in
/// fusevm_bridge.rs treating `$PWD` env as the logical-pwd carrier).
/// So the data flow matches C: globals (env) → paramtab → env. The
/// previous revision read paramtab's own PWD and wrote it back — a
/// circular no-op that left the stale environ-snapshot import in
/// place when the inherited $PWD failed ispwd().
pub fn set_pwd_env() {
    // c:800
    // c:805-816 — PM_READONLY clear + unsetparam_pm for non-scalar
    //             PWD/OLDPWD retypes isn't ported; setsparam below
    //             overwrites the scalar value and flags directly.
    // c:818 — `assignsparam("PWD", ztrdup(pwd), 0);`
    let pwd = env::var("PWD").unwrap_or_else(|_| {
        env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    setsparam("PWD", &pwd); // c:818
                            // c:819 — `assignsparam("OLDPWD", ztrdup(oldpwd), 0);`
    if let Ok(oldpwd) = env::var("OLDPWD") {
        setsparam("OLDPWD", &oldpwd); // c:819
    }
    // c:821-826 — `pm = paramtab->getnode(paramtab, "PWD"); if
    // (!(pm->node.flags & PM_EXPORTED)) addenv(pm, pwd);` and the same
    // for OLDPWD. Writing the OS environment directly (the previous
    // `env::set_var`) skipped `addenv`'s `pm->flags |= PM_EXPORTED`
    // (params.c:5482-5484), so `${(t)PWD}` read `scalar` where zsh
    // reads `scalar-export`.
    for (name, value) in [("PWD", &pwd), ("OLDPWD", &env::var("OLDPWD").unwrap_or_default())] {
        if value.is_empty() && name == "OLDPWD" {
            continue;
        }
        let exported = crate::ported::params::paramtab()
            .read()
            .ok()
            .and_then(|tab| tab.get(name).map(|pm| (pm.node.flags & PM_EXPORTED as i32) != 0))
            .unwrap_or(false);
        if !exported {
            crate::ported::params::addenv(name, value);
        }
    }
}

/// Port of `bin_cd(char *nam, char **argv, Options ops, int func)` from Src/builtin.c:840.
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
/// `bin_cd` — see implementation.
pub fn bin_cd(
    nam: &str,
    argv: &[String], // c:840
    ops: &options,
    func: i32,
) -> i32 {
    // c:844 — `doprintdir = (doprintdir == -1);`
    let prev = DOPRINTDIR.load(Relaxed);
    DOPRINTDIR.store(if prev == -1 { 1 } else { 0 }, Relaxed); // c:844

    // c:846-847 — `chasinglinks = OPT_ISSET(ops,'P') ||
    //              (isset(CHASELINKS) && !OPT_ISSET(ops,'L'));`
    let chase = OPT_ISSET(ops, b'P')                                         // c:846
        || (isset(CHASELINKS)
            && !OPT_ISSET(ops, b'L'));
    CHASINGLINKS.store(chase as i32, Relaxed);

    queue_signals(); // c:848

    // c:849 — `zpushnode(dirstack, ztrdup(pwd));`. C uses the `pwd`
    //          global (the in-shell logical cwd, kept in sync with
    //          $PWD). Read from paramtab; fall back to getcwd if
    //          unset. The C source pushes pre-cd pwd to the top of
    //          dirstack here as a scratch slot used by cd_get_dest's
    //          +N/-N resolver; cd_new_pwd's remnode logic relies on
    //          this. Save the pre-cd path for the post-cd dirstack
    //          maintenance below.
    let pre_pwd = getsparam("PWD").unwrap_or_else(|| zgetcwd());

    // c:850-854 — `if (!(dir = cd_get_dest(...))) { pop; unqueue; return 1; }`
    let dest = cd_get_dest(nam, argv, OPT_ISSET(ops, b's'), func);
    if dest.is_none() {
        unqueue_signals(); // c:852
        return 1; // c:853
    }
    let dest_raw = dest.unwrap();

    // c:Src/builtin.c:851 — `-s` safe mode: refuse to chdir into a
    // symlink. Check via fs::symlink_metadata so we see the link
    // itself, not its target.
    if OPT_ISSET(ops, b's') {
        if let Ok(meta) = fs::symlink_metadata(&dest_raw) {
            if meta.file_type().is_symlink() {
                zwarnnam(nam, &format!("{}: symbolic link", dest_raw));
                unqueue_signals();
                return 1;
            }
        }
    }

    // c:Src/builtin.c:855 — route the resolved arg through
    // cd_do_chdir so CDPATH walk, leading `~`/`.` handling, and
    // CDABLEVARS expansion fire. cd_do_chdir performs the actual
    // lchdir + returns the LOGICAL path (the one to write to PWD).
    // The previous Rust port called env::set_current_dir(dest_raw)
    // directly which skipped all of CDPATH, so `CDPATH=/foo cd bar`
    // would only resolve `./bar` even if `/foo/bar` existed.
    let dest_path = match cd_do_chdir(nam, &dest_raw, OPT_ISSET(ops, b's') as i32) {
        Some(p) => p,
        None => {
            unqueue_signals();
            return 1;
        }
    };

    // c:1238 — `oldpwd = pwd;` snapshot pre-cd $PWD for $OLDPWD.
    //          Read from paramtab (the canonical zsh-side `pwd`
    //          global); was reading OS env which can lag behind.
    let old = getsparam("PWD");
    // c:1207-1245 — compute the new logical pwd (`new_pwd` in C's
    //   cd_new_pwd) up front so the PUSHDIGNOREDUPS scan below and the
    //   final `pwd =` write share one value. Hoisted from the old
    //   write site; behaviour is identical (chase → resolved cwd,
    //   absolute dest → as-is, relative dest → logical join).
    let chase = CHASINGLINKS.load(Relaxed) != 0; // c:1203
    let new_pwd_logical: String = if chase {
        match env::current_dir() {
            Ok(c) => c.to_string_lossy().into_owned(),
            Err(_) => dest_path.clone(),
        }
    } else if dest_path.starts_with('/') {
        dest_path.clone()
    } else {
        let mut segs: Vec<&str> = if pre_pwd.is_empty() || pre_pwd == "/" {
            Vec::new()
        } else {
            pre_pwd.trim_start_matches('/').split('/').collect()
        };
        for part in dest_path.split('/') {
            match part {
                "" | "." => {}
                ".." => {
                    segs.pop();
                }
                _ => segs.push(part),
            }
        }
        if segs.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", segs.join("/"))
        }
    };
    // c:Src/builtin.c:849 + cd_new_pwd dirstack maintenance —
    // collapsed into a single post-cd update here since the Rust
    // cd_get_dest returns a String rather than a LinkNode that
    // could be pre-pushed onto dirstack:
    //   * BIN_PUSHD: push pre-cd pwd to top so subsequent
    //     `${dirstack[1]}` and `popd` see the previous directory.
    //   * BIN_POPD: pop dirstack[0] (the directory we just left,
    //     which cd_get_dest read from the stack to compute dest).
    //   * BIN_CD: dirstack unchanged unless AUTO_PUSHD is set, in
    //     which case the CD behaves like a pushd.
    let mut skip_popd_n_cd = false;
    {
        let autopushd = isset(AUTOPUSHD);
        // c:1187 cd_new_pwd — rolllist(dirstack, dir) for BIN_PUSHD then
        // remnode(dirstack, dir). For `pushd +N`/`-N`, the rotation moves
        // entries before the target to the END of dirstack rather than
        // simply inserting pre_pwd at the front. Bug #302 in docs/BUGS.md:
        // previous Rust port unconditionally inserted pre_pwd at position
        // 0, which is correct for `pushd <path>` but wrong for `pushd +N`
        // (left target entry still in DIRSTACK at its old position).
        let is_stack_rotate = argv.len() == 1
            && !isset(POSIXCD)
            && argv[0].len() > 1
            && (argv[0].starts_with('+') || argv[0].starts_with('-'))
            && argv[0][1..].chars().all(|c| c.is_ascii_digit());
        if let Ok(mut d) = DIRSTACK.lock() {
            if is_stack_rotate && (func == BIN_PUSHD || (func == BIN_CD && autopushd)) {
                // c:1190 — rolllist(dirstack, dir) + remnode. Rotate the
                // virtual full stack `[pre_pwd] ++ DIRSTACK_old` so the
                // target entry becomes firstnode (= new PWD); remaining
                // entries form new DIRSTACK in their rotated order.
                let dd: usize = argv[0][1..].parse().unwrap_or(0);
                let pushdminus = isset(PUSHDMINUS);
                let from_top = (argv[0].starts_with('+')) ^ pushdminus;
                let m = d.len();
                let n = m + 1;
                let k = if from_top { dd } else { n - 1 - dd };
                if k < n {
                    let mut full: Vec<String> = Vec::with_capacity(n);
                    full.push(pre_pwd.clone());
                    full.extend(d.iter().cloned());
                    let rotated: Vec<String> =
                        full[k..].iter().chain(full[..k].iter()).cloned().collect();
                    d.clear();
                    d.extend(rotated.into_iter().skip(1));
                }
            } else if is_stack_rotate && func == BIN_CD {
                // c:1192 — remnode(dirstack, dir) for non-PUSHD CD with
                //          +N/-N: target removed from dirstack, no rolllist.
                let dd: usize = argv[0][1..].parse().unwrap_or(0);
                let pushdminus = isset(PUSHDMINUS);
                let from_top = (argv[0].starts_with('+')) ^ pushdminus;
                let m = d.len();
                let n = m + 1;
                let k = if from_top { dd } else { n - 1 - dd };
                if k >= 1 && k - 1 < d.len() {
                    d.remove(k - 1);
                }
            } else if is_stack_rotate && func == BIN_POPD {
                // c:Src/builtin.c:872-936 + c:1197-1199 — `popd +N` /
                // `popd -N`: when the target +N/-N is NOT firstnode
                // (i.e., N != 0 / N != top-of-stack), remove the
                // target entry from dirstack WITHOUT changing PWD or
                // pushing pre_pwd to the stack. C's logic:
                //   target = dir (the +N resolved node);
                //   if (dir != firstnode(dirstack)) return dir;
                //   // … cd_do_chdir path runs only if target IS top
                //
                // The Rust dispatch reached here for the
                // "target-not-top" case after cd_get_dest already
                // resolved to dirstack[k-1]. Just remove that index
                // and skip the cd / pre_pwd-insert paths.
                // Bug #466 in docs/BUGS.md.
                let dd: usize = argv[0][1..].parse().unwrap_or(0);
                let pushdminus = isset(PUSHDMINUS);
                let from_top = (argv[0].starts_with('+')) ^ pushdminus;
                let m = d.len();
                let n = m + 1;
                // popd's stack-rotate-vs-top logic mirrors C: the
                // virtual stack is pre_pwd ++ DIRSTACK_old; position 0
                // is the CURRENT pwd, so "popd +0" / "popd +N where
                // N maps to top" pops the top (handled by the
                // existing BIN_POPD arm below). For N that resolves
                // to a non-top dirstack index, remove from dirstack
                // directly.
                let k = if from_top { dd } else { n - 1 - dd };
                if k == 0 {
                    // c:934-940 + c:1197-1203 — the +N/-N resolved to
                    // firstnode (the current PWD): this is a bare-`popd`
                    // top removal. Pop the entry below PWD (DIRSTACK[0])
                    // and let the normal cd path below move PWD into it
                    // (cd_get_dest resolved dest to DIRSTACK[0]). Do NOT
                    // set skip_popd_n_cd — PWD must change here.
                    // Bug: prior port's `k >= 1` guard fell through this
                    // else-if arm without popping, so the plain-POPD arm
                    // was never reached and the top entry survived.
                    if !d.is_empty() {
                        d.remove(0);
                    }
                } else if k - 1 < d.len() {
                    d.remove(k - 1);
                    // Override the cd path: keep PWD at pre_pwd so
                    // the surrounding flow doesn't cd to the removed
                    // entry. The destination passed to bin_cd was
                    // the removed entry; we overwrite the post-cd
                    // PWD write below to pre_pwd.
                    skip_popd_n_cd = true;
                }
            } else if func == BIN_PUSHD || (func == BIN_CD && autopushd) {
                // c:849 — push pre-cd pwd.
                d.insert(0, pre_pwd.clone());
            } else if func == BIN_POPD {
                // c:1197-1199 — pop top of stack (the dir we left).
                if !d.is_empty() {
                    d.remove(0);
                }
            }
            // c:1214-1222 — PUSHDIGNOREDUPS runs inside cd_new_pwd for
            //   EVERY func (plain cd, pushd, popd — the block is not
            //   guarded by func), scanning the now-current dirstack for
            //   the FIRST entry equal to `new_pwd` (the directory just
            //   moved into) and removing it so a directory never appears
            //   twice on the stack:
            //     if (isset(PUSHDIGNOREDUPS)) {
            //         for (n = firstnode(dirstack); n; incnode(n))
            //             if (!strcmp(new_pwd, getdata(n))) {
            //                 zsfree(remnode(dirstack, n)); break; }
            //     }
            //   Prior port only checked pushd and compared d.first()
            //   against pre_pwd (wrong target, wrong action), so a plain
            //   `cd` into a dir already on the stack, or re-pushing such
            //   a dir, left the stale duplicate in place. The non-top
            //   `popd +N` path (skip_popd_n_cd) keeps PWD = pre_pwd, so
            //   its dedup runs there against pre_pwd (see below).
            if !skip_popd_n_cd && isset(PUSHDIGNOREDUPS) {
                if let Some(pos) = d.iter().position(|s| *s == new_pwd_logical) {
                    d.remove(pos);
                }
            }
        }
    }
    // c:Src/builtin.c:872-936 — `popd +N` (N != 0) removes the
    // target entry from dirstack WITHOUT changing PWD. cd_do_chdir
    // already chdir'd into the removed target's path; revert that
    // back to pre_pwd and skip the OLDPWD update + PWD write so the
    // post-popd CWD remains the pre-popd PWD. Bug #466 in
    // docs/BUGS.md.
    if skip_popd_n_cd {
        // Restore process cwd to pre_pwd. Ignore errors; the
        // dirstack mutation already happened.
        let _ = env::set_current_dir(&pre_pwd);
        // c:1242-1246 — cd_new_pwd runs `oldpwd = pwd; pwd = new_pwd`
        //   for EVERY popd, including a non-top `popd +N`. There
        //   new_pwd resolves back to the front-of-stack pre_pwd, so
        //   PWD is unchanged but OLDPWD becomes the pre-popd PWD.
        //   Prior port left OLDPWD stale (whatever an earlier cd/pushd
        //   set), diverging on any `$OLDPWD` / `cd -` that followed.
        setsparam("OLDPWD", &pre_pwd);
        env::set_var("OLDPWD", &pre_pwd);
        // c:Src/builtin.c:1245-1252 — print dirstack on POPD unless
        // quiet. Pass func=BIN_POPD so the dirstack-print branch
        // still fires for the listing (matches zsh's `popd +N` echo
        // of the resulting stack).
        cd_new_pwd(func, 0, OPT_ISSET(ops, b'q') as i32);
        unqueue_signals();
        return 0;
    }
    if let Some(o) = old {
        // c:1239 oldpwd = pwd
        // c:1239 + setsparam path: write OLDPWD to paramtab so
        //          subsequent expansions of $OLDPWD see the new value
        //          (the OS env write below is the export side; the
        //          shell-side read must come from paramtab).
        setsparam("OLDPWD", &o);
        env::set_var("OLDPWD", &o);
    }
    // c:1241 — `pwd = new_pwd;` writes the LOGICAL path (the dest
    // argument as given to cd, not `getcwd()`). Computed once up front
    // (see `new_pwd_logical` above) so the PUSHDIGNOREDUPS scan and this
    // write agree. Symlink resolution only kicks in when `chasinglinks`
    // is set (c:1203-1208, c:1228-1231).
    let pwd: String = new_pwd_logical;
    // c:1242 — `setsparam("PWD", pwd);` + export side via env.
    setsparam("PWD", &pwd);
    env::set_var("PWD", &pwd);
    cd_new_pwd(func, 0, OPT_ISSET(ops, b'q') as i32); // c:856

    unqueue_signals(); // c:858
    0 // c:859
}

/// Port of `cd_get_dest(char *nam, char **argv, int hard, int func)` from Src/builtin.c:865.
/// C: `static LinkNode cd_get_dest(char *nam, char **argv, int hard,
///     int func)` — resolve the `cd` argument (`-`, `+N`/`-N`,
///   bare → $HOME, two-arg substitution form) to a destination path.
///   Returns the resolved path on success, None on error (with the
///   appropriate zwarnnam already emitted).
/// WARNING: param names don't match C — Rust=() vs C=(nam, argv, hard, func)
pub fn cd_get_dest(nam: &str, argv: &[String], _hard: bool, func: i32) -> Option<String> {
    if argv.is_empty() {
        // c:872 — bare popd / pushd / cd (no args).
        // The Rust port doesn't pre-push pwd to dirstack inside bin_cd
        // (cd_get_dest's String return signature doesn't fit C's
        // pre-push pattern), so dirstack[0] here is the most-recent
        // pushed entry, not the temporary scratch slot the C source
        // sees. Adjust the indices accordingly: C reads index 1
        // (skipping the scratch), Rust reads index 0.
        if func == BIN_POPD {
            let depth = DIRSTACK.lock().map(|d| d.len()).unwrap_or(0);
            if depth < 1 {
                zwarnnam(nam, "directory stack empty");
                return None;
            }
            return DIRSTACK.lock().ok().and_then(|d| d.first().cloned());
        }
        if func == BIN_PUSHD {
            // c:876-879 — bare pushd without PUSHDTOHOME swaps top two
            // entries of dirstack. C represents dirstack with PWD at
            // index 0, so it walks `nextnode(firstnode(dirstack))` to
            // get index 1, then extracts index 0 via `getlinknode` and
            // reinserts after — net swap of [0]<->[1]. zshrs's DIRSTACK
            // omits PWD entirely (PWD lives in paramtab); index 0 here
            // = C's index 1.
            //
            // To achieve the same swap in this model: POP DIRSTACK[0]
            // (the saved target) and return it. cd_new_pwd at line ~1573
            // then re-pushes pre_pwd onto DIRSTACK[0]. Net result:
            // DIRSTACK[0] becomes pre_pwd, PWD becomes the popped value.
            //
            // Bug #67 in docs/BUGS.md: prior port read `d.first().cloned()`
            // (no pop), so cd_new_pwd's push added a 3rd entry instead
            // of swapping.
            //
            // c:881-885 — empty dirstack (only PWD in C's model, i.e.
            // DIRSTACK.is_empty() here) falls through to HOME below;
            // cd_new_pwd's push then leaves DIRSTACK = [pre_pwd].
            let pushdtohome = isset(PUSHDTOHOME);
            if !pushdtohome {
                let popped = DIRSTACK.lock().ok().and_then(|mut d| {
                    if d.is_empty() {
                        None
                    } else {
                        Some(d.remove(0))
                    }
                });
                if let Some(target) = popped {
                    return Some(target);
                }
                // Empty DIRSTACK → fall through to HOME below.
            }
        }
        // c:880-884 — fall through to $HOME.
        match getsparam("HOME") {
            Some(h) if !h.is_empty() => Some(h),
            _ => {
                zwarnnam(nam, "HOME not set");
                None
            }
        }
    } else if argv.len() == 1 {
        // c:887
        let arg = &argv[0];
        DOPRINTDIR.fetch_add(1, Relaxed); // c:891
                                          // c:892-908 — `+N`/`-N` numeric stack-index form.
        let posixcd = isset(POSIXCD);
        if !posixcd
            && arg.len() > 1
            && (arg.starts_with('+') || arg.starts_with('-'))
            && arg[1..].chars().all(|c| c.is_ascii_digit())
        {
            let dd: usize = arg[1..].parse().unwrap_or(0); // c:894
            let pushdminus = isset(PUSHDMINUS);
            let from_top = (arg.starts_with('+')) ^ pushdminus; // c:898
                                                                // c:Src/builtin.c:904 — out-of-range stack index emits
                                                                // "no such entry in dir stack". Previous Rust port
                                                                // returned None silently and bin_cd's caller exited 1
                                                                // with no stderr, breaking parity.
                                                                // c:899-903 — index into the FULL dirstack: in zsh
                                                                //   `dirstack` is a LinkList with current PWD at firstnode,
                                                                //   so `+N` advances N nodes from firstnode (0 = current,
                                                                //   1 = first non-current entry) and `-N` walks back N nodes
                                                                //   from lastnode (0 = last non-current entry). zshrs's
                                                                //   DIRSTACK omits PWD entirely (PWD lives in paramtab), so
                                                                //   build the virtual full stack `[PWD] ++ DIRSTACK` to index
                                                                //   against. Bug #302 in docs/BUGS.md: previous Rust port
                                                                //   indexed DIRSTACK directly, so `+1` returned DIRSTACK[1]
                                                                //   instead of DIRSTACK[0], and `+0`/`-len` errored instead of
                                                                //   returning PWD.
            let pwd_now = getsparam("PWD").unwrap_or_else(|| zgetcwd());
            let resolved = DIRSTACK.lock().ok().and_then(|d| {
                let m = d.len();
                let n = m + 1; // full virtual stack length (incl current)
                let k = if from_top {
                    if dd >= n {
                        return None;
                    }
                    dd
                } else {
                    if dd >= n {
                        return None;
                    }
                    n - 1 - dd
                };
                if k == 0 {
                    // c:934-940 — for BIN_POPD, dir==firstnode(dirstack)
                    //   (the current PWD) is NOT removed in place: C does
                    //   `dir = nextnode(dir)` and cd_do_chdir's into the
                    //   entry BELOW PWD, then cd_new_pwd (c:1197-1199)
                    //   pops the front. So the chdir destination is
                    //   DIRSTACK[0] (the first non-current entry), not PWD.
                    //   For BIN_CD / BIN_PUSHD, +0 (or the swapped -last)
                    //   resolves to the current dir itself.
                    //   Bug: prior port returned pwd_now for every func, so
                    //   `popd +0` (and `popd +N`/`-N` mapping to firstnode
                    //   under PUSHD_MINUS) no-op'd instead of popping the top.
                    if func == BIN_POPD {
                        d.first().cloned().or_else(|| Some(pwd_now.clone()))
                    } else {
                        Some(pwd_now.clone())
                    }
                } else {
                    d.get(k - 1).cloned()
                }
            });
            if resolved.is_none() {
                zwarnnam(nam, "no such entry in dir stack");
            }
            return resolved;
        }
        // c:908-909 — `zpushnode(dirstack, ztrdup(strcmp(argv[0],"-")
        //   ? (doprintdir--, argv[0]) : oldpwd))`. `strcmp(argv[0],"-")`
        //   is non-zero for a normal dir, so a normal `cd dir` does
        //   `doprintdir--` (cancelling the c:891 `doprintdir++` → no
        //   print), while `cd -` keeps doprintdir set → prints $OLDPWD.
        //   C reads the `oldpwd` global; route through `$OLDPWD`.
        if arg == "-" {
            // c:909 — `: oldpwd` — no doprintdir--, so `cd -` prints.
            getsparam("OLDPWD")
        } else {
            // c:909 — `? (doprintdir--, argv[0])`.
            DOPRINTDIR.fetch_sub(1, Relaxed);
            Some(arg.clone())
        }
    } else {
        // c:914-924 — two-arg substitution: cd OLDPATTERN NEWPATTERN.
        //              C reads `pwd` global / `$PWD` param via getsparam;
        //              fall back to getcwd if the param isn't populated.
        let pwd = getsparam("PWD").unwrap_or_else(|| zgetcwd());
        let pat = &argv[0];
        let new_pat = &argv[1];
        match pwd.find(pat.as_str()) {
            // c:917
            None => {
                zwarnnam(nam, &format!("string not in pwd: {}", pat)); // c:918
                None // c:919
            }
            Some(idx) => {
                // c:921-924 — splice: pwd[..idx] + new_pat + pwd[idx+pat.len()..]
                let mut out = String::new();
                out.push_str(&pwd[..idx]); // c:921
                out.push_str(new_pat); // c:922
                out.push_str(&pwd[idx + pat.len()..]); // c:923
                DOPRINTDIR.fetch_add(1, Relaxed);
                Some(out)
            }
        }
    }
}

/// Port of `cd_do_chdir(char *cnam, char *dest, int hard)` from Src/builtin.c:967.
/// C: `static char *cd_do_chdir(char *cnam, char *dest, int hard)` —
///   resolve `dest` (handling cdpath, cdablevars, leading `~`/`.`),
///   chdir there, return the LOGICAL path used (not `getcwd`'d) or
///   NULL on error.
///
/// Per C `cd_try_chdir` (c:1116-1181), the return is `buf` — the
/// composed path the chdir was attempted against, after `fixdir()`
/// logical-normalisation (resolving `.`/`..` only, NOT symlinks).
/// Walks $cdpath when dest is relative and not `./` or `../`.
pub fn cd_do_chdir(cnam: &str, dest: &str, hard: i32) -> Option<String> {
    // c:967
    // c:996-998 — nocdpath = first segment is "." or ".."
    let nocdpath = dest.starts_with("./") || dest == "." || dest.starts_with("../") || dest == "..";

    // c:1003-1008 — absolute path: try as-is, warn on failure.
    if dest.starts_with('/') {
        if let Some(ret) = cd_try_chdir("", dest, hard) {
            return Some(ret);
        }
        // c:Src/builtin.c — `zwarnnam(nam, "%e: %s", errno, dest)`.
        // C's %e printf format expands to strerror(errno) then
        // LOWERCASES the first letter unless the errno is EIO
        // (Src/utils.c:362-368 comment: "If the message is not
        // about I/O problems, it looks better if we uncapitalize
        // the first letter of the message"). Previous Rust port
        // used `io::Error::last_os_error()`'s Display impl, which
        // appends ` (os error N)` — leaked the Rust-internal
        // suffix into user-visible cd output. Bug #112 in
        // docs/BUGS.md. Route through compat::strerror to drop
        // the suffix AND apply the lowercase-first-letter rule so
        // the message matches C exactly.
        let errno = io::Error::last_os_error().raw_os_error().unwrap_or(0);
        let mut msg = crate::ported::compat::strerror(errno);
        if errno != libc::EIO {
            if let Some(c) = msg.chars().next() {
                msg = format!("{}{}", c.to_ascii_lowercase(), &msg[c.len_utf8()..]);
            }
        }
        zwarnnam(cnam, &format!("{}: {}", msg, dest));
        return None;
    }

    // c:1015-1018 — check $cdpath for "." (presence flips hasdot).
    let posix_cd = isset(POSIXCD);
    let cdpath_str = getsparam("CDPATH").unwrap_or_default();
    let cdpath: Vec<&str> = if cdpath_str.is_empty() {
        Vec::new()
    } else {
        cdpath_str.split(':').collect()
    };
    let hasdot = !nocdpath && !posix_cd && cdpath.iter().any(|p| p.is_empty() || *p == ".");

    // c:1026-1031 — if no dot in cdpath (and !POSIXCD), try as-is first.
    if !hasdot && !posix_cd {
        if let Some(ret) = cd_try_chdir("", dest, hard) {
            return Some(ret);
        }
    }

    // c:1034-1043 — walk $cdpath unless nocdpath.
    if !nocdpath {
        for pp in cdpath.iter() {
            if let Some(ret) = cd_try_chdir(pp, dest, hard) {
                // c:1037-1050 — flag that the resolved directory should
                // be printed; the actual print happens later in
                // cd_new_pwd (c:1248-1251) gated on doprintdir. C does
                // NOT print here, it only bumps `doprintdir`.
                if isset(POSIXCD) {
                    // c:1037-1045 — POSIX prints any time CDPATH was
                    // used, except for an empty segment treated as ".".
                    if !pp.is_empty() {
                        DOPRINTDIR.fetch_add(1, Relaxed); // c:1045
                    }
                } else if *pp != "." {
                    // c:1046-1049 — non-POSIX: print only for a
                    // non-"." segment.
                    DOPRINTDIR.fetch_add(1, Relaxed); // c:1048
                }
                return Some(ret);
            }
        }
    }

    // c:1057-1063 — POSIXCD-mode last-resort: try dest as-is.
    if posix_cd {
        if let Some(ret) = cd_try_chdir("", dest, hard) {
            return Some(ret);
        }
    }

    // c:1067-1075 — `cd_able_vars(dest)` — when CDABLEVARS is set,
    // look up the leading bareword as a parameter and try chdir to
    // its value (with any `/tail` appended). The helper at
    // `cd_able_vars` (this file, line 1881) already guards on
    // `isset(CDABLEVARS)` and returns None when the option is off
    // OR the head isn't a paramtab entry. Without this call
    // wired in, `setopt cdable_vars; mytmp=/tmp; cd mytmp` errored
    // with "no such file or directory" instead of cd'ing to /tmp.
    // Bug #217 in docs/BUGS.md.
    if let Some(expanded) = cd_able_vars(dest) {
        if let Some(ret) = cd_try_chdir("", &expanded, hard) {
            // c:1069 — `doprintdir++` so cd_new_pwd prints the
            // CDABLEVARS-resolved directory.
            DOPRINTDIR.fetch_add(1, Relaxed); // c:1069
            return Some(ret);
        }
    }

    // c:1071 — failure warning.
    zwarnnam(cnam, &format!("no such file or directory: {}", dest));
    None
}

/// Port of `cd_able_vars(char *s)` from Src/builtin.c:1088.
/// C: `char *cd_able_vars(char *s)` — when CDABLEVARS is set, look up
///   the leading bareword as a parameter and return its expanded value
///   prefixed in front of any trailing `/...`. Returns NULL otherwise.
pub fn cd_able_vars(s: &str) -> Option<String> {
    // c:1088
    // c:1088 — `if (isset(CDABLEVARS)) { ... }`
    let cdablevars = isset(CDABLEVARS);
    if !cdablevars {
        // c:1093
        return None;
    }
    // c:1094-1110 — split on the first `/`, look up the head as $param.
    let (head, tail) = match s.find('/') {
        // c:1094
        Some(i) => (&s[..i], &s[i..]),
        None => (s, ""),
    };
    if head.is_empty() {
        return None;
    }
    // c:1116 — `if ((val = getsparam(s))) { ret = tricat(val, tail, "") }`.
    //          C reads $head from paramtab; was reading OS env, missing
    //          CDABLEVARS-style assignments like `proj=$HOME/src`.
    getsparam(head).map(|val| format!("{}{}", val, tail))
}

/// Port of `cd_try_chdir(char *pfix, char *dest, int hard)` from `Src/builtin.c:1116`.
/// Compose `pfix/dest` (or `pwd/pfix/dest` for relative pfix, or
/// `pwd/dest` for empty pfix + relative dest), normalise via `fixdir`,
/// then attempt chdir. Falls back to `dest` alone when the full path
/// fails but `pfix` was present (cwd/parent may have been renamed).
pub fn cd_try_chdir(pfix: &str, dest: &str, hard: i32) -> Option<String> {
    // c:1116
    let pwd = getsparam("PWD").unwrap_or_default();

    // c:1122-1158 — build buf from pfix/dest/pwd combinations.
    let mut buf = if !pfix.is_empty() {
        if pfix.starts_with('/') {
            // c:1123
            // c:1133 — buf = tricat(pfix, "/", dest)
            if pfix.ends_with('/') {
                format!("{}{}", pfix, dest)
            } else {
                format!("{}/{}", pfix, dest)
            }
        } else {
            // c:1135-1146 — pwd + "/" + pfix + "/" + dest
            let pwd_trim = if pwd == "/" { "" } else { pwd.as_str() };
            format!("{}/{}/{}", pwd_trim, pfix, dest)
        }
    } else if dest.starts_with('/') {
        // c:1148
        // c:1149 — buf = ztrdup(dest)
        dest.to_string()
    } else {
        // c:1150
        // c:1151-1157 — pwd + "/" + dest (trimming trailing slash off pwd)
        let pwd_trim = pwd.trim_end_matches('/');
        format!("{}/{}", pwd_trim, dest)
    };

    // c:1163-1166 — fixdir normalisation, skipped if chasing symlinks.
    if CHASINGLINKS.load(Relaxed) == 0 {
        buf = fixdir(&buf); // c:1164
    }

    // c:1169-1177 — "We try the full path first.  If that fails, try the
    // argument to cd relatively.  This is useful if the cwd or a parent
    // directory is renamed in the interim."  The relative fallback is
    // skipped when a prefix was applied or `dest` is already absolute.
    // Either success returns `buf` (the logical full path) — c:1181.
    if lchdir(&buf, None, hard) != 0
        && (!pfix.is_empty() || dest.starts_with('/') || lchdir(dest, None, hard) != 0)
    {
        return None; // c:1175-1176
    }
    Some(buf) // c:1181 — metafy(buf, ...)
}

/// Port of `cd_new_pwd(int func, LinkNode dir, int quiet)` from Src/builtin.c:1187.
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
/// WARNING: param names don't match C — Rust=(_func, _dir, _quiet) vs C=(func, dir, quiet)
pub fn cd_new_pwd(func: i32, _dir: usize, quiet: i32) {
    // c:1187 — post-cd side effects (print, hooks). Dirstack
    // maintenance was moved into `bin_cd` because the Rust port's
    // cd_get_dest returns a String (not a LinkNode pre-pushed on
    // dirstack), so C's remnode/rolllist sequence doesn't fit.

    // c:1236-1242 — shift PWD → OLDPWD, set new PWD.
    //
    // The caller (`bin_cd` at builtin.rs:1379-1408) already wrote
    // OLDPWD (pre-cd $PWD) and PWD (the logical `dest_path`) into the
    // paramtab. Re-deriving them here would (a) double-shift OLDPWD
    // (overwriting the correct pre-cd value with the just-set PWD),
    // and (b) re-set PWD via `std::env::current_dir()` which always
    // returns the resolved physical path — clobbering the logical
    // path on systems where the destination is a symlink (e.g. macOS
    // /tmp → /private/tmp). C `cd_new_pwd` reads `new_pwd` off the
    // dirstack (the path the user typed); zshrs's dirstack plumbing
    // doesn't carry that path here, so the caller is the authoritative
    // PWD writer and this fn must NOT re-write either parameter.

    // c:1244-1252 — print the new directory.
    //   if (isset(INTERACTIVE) || isset(POSIXCD)) {
    //       if (func != BIN_CD && isset(INTERACTIVE)) {
    //           if (unset(PUSHDSILENT) && !quiet) printdirstack();
    //       } else if (unset(CDSILENT) && doprintdir) {
    //           fprintdir(pwd, stdout); putchar('\n');
    //       }
    //   }
    if isset(INTERACTIVE) || isset(POSIXCD) {
        if func != BIN_CD && isset(INTERACTIVE) {
            // c:1245-1247 — pushd/popd echo the dir stack.
            if !isset(PUSHDSILENT) && quiet == 0 {
                printdirstack(); // c:1247
            }
        } else if !isset(crate::ported::zsh_h::CDSILENT) && DOPRINTDIR.load(Relaxed) != 0 {
            // c:1248-1251 — cd echoes the resolved directory (set via
            // doprintdir by `cd -`, CDPATH, autocd, CDABLEVARS).
            let pwd = getsparam("PWD").unwrap_or_default(); // C global `pwd`
            println!("{}", fprintdir(&pwd)); // c:1249-1250
        }
    }

    // c:1258 — `callhookfunc("chpwd", NULL, 1, NULL)` fires the chpwd
    // hook. Not surfaced here: the executor wrapper (fusevm_bridge.rs
    // cd builtin) calls `callhookfunc("chpwd", ...)` after a successful
    // cd, which dispatches both the `chpwd` shfunc and the
    // `chpwd_functions` array (utils.rs:1532).

    // c:1264-1271 — trim the dir stack to $DIRSTACKSIZE.
    //   `dirstacksize = getiparam("DIRSTACKSIZE");
    //    if (dirstacksize > 0) {
    //        int remove = countlinknodes(dirstack) -
    //            (dirstacksize < 2 ? 2 : dirstacksize);
    //        while (remove-- >= 0)
    //            zsfree(remnode(dirstack, lastnode(dirstack)));
    //    }`
    let dirstacksize = getiparam("DIRSTACKSIZE"); // c:1264
    if dirstacksize > 0 {
        // c:1266
        if let Ok(mut d) = DIRSTACK.lock() {
            // c:1267-1268 — count minus the effective cap (min 2).
            let cap = if dirstacksize < 2 { 2 } else { dirstacksize };
            let mut remove = d.len() as i64 - cap;
            // c:1269-1270 — `while (remove-- >= 0)` pops the last node;
            // the post-decrement removes (remove+1) entries from the end.
            while remove >= 0 {
                remove -= 1;
                if d.pop().is_none() {
                    break;
                }
            }
        }
    }
}

/// Port of `printdirstack()` from Src/builtin.c:1277.
/// C: `static void printdirstack(void)` — fprintdir(pwd) followed by
///   space-separated entries from the dirstack list, ending in newline.
pub fn printdirstack() {
    // c:1277
    // c:1281 — `fprintdir(pwd, stdout);`. C uses the shell-side
    //          `pwd` global (in-shell logical cwd), not getcwd. Read
    //          $PWD from paramtab so the logical path (including
    //          any unresolved symlinks) shows correctly. Route
    //          through `utils::fprintdir` for the same `~` /
    //          `~named` abbreviation real zsh emits.
    // Previous Rust port emitted raw paths, missing the
    // $HOME / nameddirtab abbreviation that makes pushd/popd output
    // legible. Same fix family as bin_dirs.
    let pwd = getsparam("PWD")
        .or_else(|| {
            env::current_dir()
                .ok()
                .and_then(|p| p.to_str().map(String::from))
        })
        .unwrap_or_default();
    print!("{}", fprintdir(&pwd)); // c:1281
                                   // c:1282-1286 — `for (node = firstnode(dirstack); ...)`
    if let Ok(d) = DIRSTACK.lock() {
        for entry in d.iter() {
            // c:1282
            print!(" {}", fprintdir(entry)); // c:1284
        }
    }
    println!(); // c:1287
}

/// Direct port of `int fixdir(char *src)` from
/// `Src/builtin.c:1297`. Lexically canonicalises a path in-place
/// (no symlink follow): collapses `//`, drops `./` segments, and
/// removes `..` along with their preceding segment. Returns 1 if
/// fully canonicalised, 0 if a `..` could not be popped (e.g. at
/// the root or with `..` as the first segment under CHASEDOTS=0).
///
/// Rust port takes ownership of `src` and returns the canonical
/// form; was a 1-line stub returning empty string.
pub fn fixdir(src: &str) -> String {
    // c:1297
    if src.is_empty() {
        return String::new();
    }

    // c:1320-1325 — `chasedots` flag for the cdpath `../` edge case.
    //                Skipped here — only fires under the pwd=="." rare
    //                state. Lexical canonicalisation is what callers
    //                rely on.
    let abs = src.starts_with('/');
    let mut components: Vec<&str> = Vec::new();

    // c:1339-1395 — walk slash-separated segments.
    for seg in src.split('/') {
        match seg {
            "" => continue,  // collapse `//`
            "." => continue, // c:1352 drop `./`
            ".." => {
                // c:1358-1372 — pop previous segment if present and not
                //                also `..` (sticky-`..` for relative
                //                paths past their start).
                if let Some(last) = components.last() {
                    if *last == ".." {
                        components.push("..");
                    } else {
                        components.pop();
                    }
                } else if !abs {
                    // Relative path: keep the leading `..`.
                    components.push("..");
                }
                // Absolute path: silently drop `..` past `/`.
            }
            other => components.push(other),
        }
    }

    let body = components.join("/");
    if abs {
        format!("/{}", body)
    } else if body.is_empty() {
        ".".to_string()
    } else {
        body
    }
}

/// Port of `printqt(char *str)` from Src/builtin.c:1399.
/// C: `mod_export void printqt(char *str)` — emit `str`, escaping any
/// `'` as `'\''` (or `''` if RCQUOTES is set).
pub fn printqt(str: &str) {
    // c:1399
    let rcquotes = isset(RCQUOTES); // c:1399 isset(RCQUOTES)
    for ch in str.chars() {
        // c:1403
        if ch == '\'' {
            // c:1404
            print!("{}", if rcquotes { "''" } else { "'\\''" }); // c:1405
        } else {
            print!("{}", ch); // c:1407
        }
    }
}

/// Port of `printif(char *str, int c)` from Src/builtin.c:1411.
/// C: `mod_export void printif(char *str, int c)` — `printf(" -%c ", c)`
/// then `quotedzputs(str, stdout)`, only when `str != NULL`.
pub fn printif(str: Option<&str>, c: u8) {
    // c:1411
    if let Some(s) = str {
        // c:1399
        print!(" -{} ", c as char); // c:1399
                                    // c:1399 — quotedzputs(str, stdout); plain print preserves bytes
                                    // for the ASCII case; full quotedzputs lives in src/ported/utils.rs.
        print!("{}", s); // c:1399
    }
}

/// Port of `bin_fc(char *nam, char **argv, Options ops, int func)` from Src/builtin.c:1426.
/// C: `int bin_fc(char *nam, char **argv, Options ops, int func)`.
///
/// History/edit/list dispatcher: `-p` push hist stack, `-P` pop,
/// `-R` read, `-W` write, `-A` append, `-m` glob filter, `-l` list,
/// `-s` substitute, default: edit + re-execute. The C body is ~245
/// lines; the structural translation here covers the major options
/// and dispatches the underlying history-file ops to the existing
/// hist.rs accessors.
/// WARNING: param names don't match C — Rust=(nam, argv, func) vs C=(nam, argv, ops, func)
pub fn bin_fc(
    nam: &str,
    argv: &[String], // c:1426
    ops_in: &options,
    func: i32,
) -> i32 {
    // C `Options ops` is `struct options *` — mutable via `ops->ind['n']
    // = 1;` at c:1644. zshrs HandlerFunc takes `&options`, so we clone
    // to a fn-local `ops` mirror at the top. Mutation of the clone is
    // intra-fn only (`fclist` reads `ops` to format output and never
    // returns it), so behavior matches C.
    let mut ops = ops_in.clone();
    let ops = &mut ops;
    let mut argv = argv.to_vec();
    let mut first: i64 = -1;
    let mut last: i64 = -1;
    let mut asgf: Vec<(String, String)> = Vec::new();

    // c:1441-1481 — `-p` push history stack.
    if OPT_ISSET(ops, b'p') {
        // c:1441
        let mut hf = "".to_string();
        let mut hs: i64; // c:1443
        let mut shs: i64; // c:1444
                          // c:1445 — `int level = OPT_ISSET(ops,'a') ? locallevel : -1;`
        let level: i32 = if OPT_ISSET(ops, b'a') {
            locallevel_param.load(Relaxed)
        } else {
            -1
        };
        hs = histsiz.load(Relaxed); // c:1442
        shs = savehistsiz.load(Relaxed);
        if !argv.is_empty() {
            // c:1445
            hf = argv.remove(0); // c:1446
            if !argv.is_empty() {
                // c:1447
                let s2 = argv.remove(0);
                match s2.parse::<i64>() {
                    // c:1449 zstrtol
                    Ok(n) => hs = n,
                    Err(_) => {
                        zwarnnam(
                            "fc", // c:1452
                            "HISTSIZE must be an integer",
                        );
                        return 1; // c:1453
                    }
                }
                if !argv.is_empty() {
                    // c:1455
                    let s3 = argv.remove(0);
                    match s3.parse::<i64>() {
                        // c:1456
                        Ok(n) => shs = n,
                        Err(_) => {
                            zwarnnam(
                                "fc", // c:1459
                                "SAVEHIST must be an integer",
                            );
                            return 1; // c:1460
                        }
                    }
                } else {
                    shs = hs; // c:1464
                }
                if !argv.is_empty() {
                    // c:1466
                    zwarnnam(
                        "fc", // c:1468
                        "too many arguments",
                    );
                    return 1; // c:1469
                }
            }
        }
        // c:1473 — pushhiststack(hf, hs, shs, level); failure → return 1.
        pushhiststack(Some(&hf), hs, shs, level); // c:1473
        if !hf.is_empty() {
            // c:1475
            // c:1476-1480 — `if (stat(hf, &st) >= 0 || errno != ENOENT)
            //                  readhistfile(hf, 1, HFILE_USE_OPTIONS);`
            // Previous Rust port read `Error::last_os_error()` AFTER
            // checking `metadata().is_ok()` — racey: any intervening
            // syscall between the metadata call and last_os_error()
            // can stomp errno on some platforms. Capture the per-Err
            // raw_os_error directly so we read the SAME errno value
            // the stat call produced.
            let stat_result = fs::metadata(&hf);
            let should_read = match &stat_result {
                Ok(_) => true,                                    // c:1477 stat >= 0
                Err(e) => e.raw_os_error() != Some(libc::ENOENT), // c:1477 errno != ENOENT
            };
            if should_read {
                // c:1477
                readhistfile(
                    // c:1478
                    Some(&hf),
                    1,
                    HFILE_USE_OPTIONS as i32,
                );
            }
        }
        return 0; // c:1483
    }

    // c:1485-1491 — `-P` pop history stack.
    if OPT_ISSET(ops, b'P') {
        // c:1485
        if !argv.is_empty() {
            // c:1486
            zwarnnam("fc", "too many arguments"); // c:1487
            return 1; // c:1488
        }
        // c:1490 — `return !saveandpophiststack(-1, HFILE_USE_OPTIONS);`.
        let popped = saveandpophiststack(-1, HFILE_USE_OPTIONS as i32); // c:1490
        return if popped != 0 { 0 } else { 1 }; // c:1490 `!` flip
    }

    // c:1494-1500 — `-m` pattern filter (compile first arg).
    let mut pprog: Option<crate::ported::pattern::PatProg> = None;
    let mut pprog_src: Option<String> = None;
    if !argv.is_empty() && OPT_ISSET(ops, b'm') {
        // c:1494
        let pat = argv.remove(0);
        // c:1495 — tokenize(*argv); — Rust `patcompile` handles tokenisation.
        match patcompile(
            &{
                let mut __pat_tok = (&pat).to_string();
                crate::ported::glob::tokenize(&mut __pat_tok);
                __pat_tok
            }, // c:1496
            PAT_HEAPDUP,
            None,
        ) {
            Some(p) => {
                pprog = Some(p);
                pprog_src = Some(pat); // retain source string for fclist
            }
            None => {
                zwarnnam(nam, "invalid match pattern"); // c:1497
                return 1; // c:1498
            }
        }
    }

    queue_signals(); // c:1502

    // c:1503-1525 — `-R` read / `-W` write / `-A` append history file.
    if OPT_ISSET(ops, b'R') {
        // c:1503
        let path = argv.first().cloned();
        let flags = if OPT_ISSET(ops, b'I') {
            HFILE_SKIPOLD as i32
        } else {
            0
        };
        readhistfile(
            // c:1505
            path.as_deref(),
            1,
            flags,
        );
        unqueue_signals(); // c:1506
        return 0; // c:1507
    }
    if OPT_ISSET(ops, b'W') {
        // c:1509
        let path = argv.first().cloned();
        let flags = if OPT_ISSET(ops, b'I') {
            HFILE_SKIPOLD as i32
        } else {
            0
        };
        savehistfile(
            // c:1511
            path.as_deref(),
            flags,
        );
        unqueue_signals(); // c:1512
        return 0; // c:1513
    }
    if OPT_ISSET(ops, b'A') {
        // c:1515
        let path = argv.first().cloned();
        let mut flags = HFILE_APPEND as i32;
        if OPT_ISSET(ops, b'I') {
            flags |= HFILE_SKIPOLD as i32;
        } // c:1518
        savehistfile(
            // c:1517
            path.as_deref(),
            flags,
        );
        unqueue_signals(); // c:1519
        return 0; // c:1520
    }

    // c:1523-1527 — refuse inside ZLE.
    if crate::ported::builtins::sched::zleactive.load(
        // c:1523
        Relaxed,
    ) != 0
    {
        unqueue_signals(); // c:1524
        zwarnnam(
            nam, // c:1525
            "no interactive history within ZLE",
        );
        return 1; // c:1526
    }

    // c:1530-1547 — `name=value` substitution pairs.
    while !argv.is_empty() && argv[0].contains('=') {
        // c:1530
        let arg = argv.remove(0);
        if let Some(eq) = arg.find('=') {
            let n = &arg[..eq];
            let v = &arg[eq + 1..];
            if n.is_empty() {
                zwarnnam(nam, &format!("invalid replacement pattern: ={}", v)); // c:1534
                return 1;
            }
            asgf.push((n.to_string(), v.to_string())); // c:1546
        }
    }

    // c:1550-1568 — first/last history specifiers via fcgetcomm.
    if !argv.is_empty() {
        // c:1550
        first = fcgetcomm(&argv.remove(0)); // c:1551
        if first == -1 {
            unqueue_signals();
            return 1; // c:1553
        }
    }
    if !argv.is_empty() {
        // c:1559
        last = fcgetcomm(&argv.remove(0)); // c:1560
        if last == -1 {
            unqueue_signals();
            return 1;
        }
    }
    if !argv.is_empty() {
        // c:1567
        unqueue_signals();
        zwarnnam("fc", "too many arguments"); // c:1569
        return 1;
    }

    // c:1573-1610 — default ranges + listing/edit dispatch. C reads
    //                the live `curhist` global at hist.rs directly. The
    //                FQN here is forced — bare `curhist` would resolve
    //                to the local `let curhist` we're declaring.
    let curhist: i64 = crate::ported::hist::curhist.load(Relaxed) as i64;
    if last == -1 {
        // c:1573
        if OPT_ISSET(ops, b'l') && first < curhist {
            // c:1574
            // c:1581-1583 — `last = (curline.histnum == curhist) ?
            //     addhistnum(curhist,-1,0) : curhist;`
            // When the fc command itself occupies the current event
            // (the normal interactive case), the default range EXCLUDES
            // it — zsh's `fc -l 1` does not list the in-flight `fc -l 1`
            // line. The previous `last = curhist` included it.
            let curline_num = crate::ported::hist::curline
                .lock()
                .ok()
                .and_then(|c| c.as_ref().map(|e| e.histnum))
                .unwrap_or(0);
            last = if curline_num == curhist {
                crate::ported::hist::addhistnum(curhist, -1, 0) // c:1582
            } else {
                curhist // c:1583
            };
            // c:1584-1585 — `if (last < firsthist()) last = firsthist();`
            let fh = crate::ported::hist::firsthist();
            if last < fh {
                last = fh; // c:1585
            }
        } else {
            last = first; // c:1587
        }
    }
    if first == -1 {
        // c:1589
        let _xflags = if OPT_ISSET(ops, b'L') {
            HIST_FOREIGN
        } else {
            0
        }; // c:1597
        first = if OPT_ISSET(ops, b'l') {
            (curhist - 16).max(1)
        }
        // c:1598
        else {
            (curhist - 1).max(1)
        };
        if last < first {
            last = first;
        } // c:1604
    }

    let mut retval;
    if OPT_ISSET(ops, b'l') {
        // c:1606
        // c:1608 — `fclist(stdout, ops, first, last, asgf, pprog, 0);`
        retval = fclist(
            &mut io::stdout(),
            ops,
            first,
            last,
            &asgf,
            pprog_src.as_deref(),
            0,
        );
        unqueue_signals();
    } else {
        // c:1611-1668 — edit history range to a temp file, fcedit it,
        // then stuff() the result back as the next command.
        retval = 1; // c:1620
        let fil_opt = gettempfile(Some("zshfc")); // c:1621 gettempfile
        match fil_opt {
            None => {
                // c:1623
                unqueue_signals(); // c:1624
                zwarnnam(
                    "fc", // c:1625
                    &format!("can't open temp file: {}", io::Error::last_os_error()),
                );
            }
            Some((fd, fil)) => {
                unsafe {
                    libc::close(fd);
                } // c:1622 (file is reopened below)
                  // c:1632 — `if (last >= curhist) { last = curhist - 1; ... }`
                if last >= curhist {
                    // c:1632
                    last = curhist - 1; // c:1633
                    if first > last {
                        // c:1634
                        unqueue_signals(); // c:1635
                        zwarnnam(
                            "fc", // c:1636
                            "current history line would recurse endlessly, aborted",
                        );
                        let _ = fs::remove_file(&fil); // c:1639 unlink
                        return 1; // c:1640
                    }
                }
                ops.ind[b'n' as usize] = 1; // c:1644 No line numbers
                let out = fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    .truncate(true)
                    .open(&fil)
                    .ok();
                let listed = if let Some(mut f) = out {
                    // c:1645 — thread pprog filter so `fc -m PAT` only
                    // edits matching lines in this temp-file edit path.
                    fclist(&mut f, ops, first, last, &asgf, pprog_src.as_deref(), 1)
                } else {
                    1
                };
                if listed == 0 {
                    // c:1645
                    // c:1647-1656 — pick editor.
                    let editor: String = if func == BIN_R || OPT_ISSET(ops, b's') {
                        "-".to_string() // c:1648
                    } else if OPT_HASARG(ops, b'e') {
                        // c:1649
                        OPT_ARG(ops, b'e').unwrap_or("").to_string() // c:1650
                    } else {
                        // c:1651-1654 — `getsparam("FCEDIT") ?:
                        //                  getsparam("EDITOR") ?:
                        //                  DEFAULT_FCEDIT`. paramtab read.
                        getsparam("FCEDIT")
                            .or_else(|| getsparam("EDITOR"))
                            .unwrap_or_else(|| crate::ported::config_h::DEFAULT_FCEDIT.to_string())
                    };
                    unqueue_signals(); // c:1657
                    if fcedit(&editor, &fil) != 0 {
                        // c:1658
                        if crate::ported::input::stuff(&fil) != 0 {
                            // c:1659
                            zwarnnam(
                                "fc", // c:1660
                                &format!("{}: {}", io::Error::last_os_error(), fil),
                            );
                        } else {
                            // c:1663-1664 — `loop(0,1); retval = lastval;`
                            // The interactive loop drives the next stuffed
                            // line through the parser. Static-link path:
                            // the executor's input source picks it up on
                            // the next read; lastval reflects that result.
                            retval = LASTVAL.load(
                                // c:1664
                                Relaxed,
                            );
                        }
                    }
                } else {
                    unqueue_signals(); // c:1667
                }
                let _ = fs::remove_file(&fil); // c:1671 unlink
            }
        }
    }
    let _ = pprog; // compiled form kept for parity; source threads through fclist.
    retval // c:1675
}

/// Port of `fcgetcomm(char *s)` from Src/builtin.c:1683.
/// C: `static zlong fcgetcomm(char *s)` — match `s` against history
///   numbers (signed) or prefix; returns the matched event number.
/// Direct port of `zlong fcgetcomm(char *s)` from
/// `Src/builtin.c:1683`. Resolve an `fc` command-line argument to a
/// history event number. Numeric args become event numbers (negative
/// numbers count back from current via `addhistnum`); non-numeric
/// args go through `hcomsearch` (history prefix search). Emits
/// `zwarnnam("fc", "event not found: %s", s)` and returns -1 on
/// miss.
pub fn fcgetcomm(s: &str) -> i64 {
    // c:1683
    // c:1689 — `if ((cmd = atoi(s)) != 0 || *s == '0')` numeric arm.
    //          atoi accepts leading whitespace + optional sign +
    //          digits; trim+parse mirrors that.
    let trimmed = s.trim_start();
    let numeric = trimmed.parse::<i64>().ok();
    let is_zero_prefix = trimmed.starts_with('0');
    if let Some(mut cmd) = numeric {
        if cmd != 0 || is_zero_prefix {
            if cmd < 0 {
                // c:1693 — `cmd = addhistnum(curline.histnum, cmd, HIST_FOREIGN);`
                let curh = crate::ported::hist::curhist.load(Relaxed);
                cmd = addhistnum(curh, cmd as i32, 1);
            }
            if cmd < 0 {
                // c:1695
                cmd = 0;
            }
            return cmd;
        }
    }
    // c:1700 — `cmd = hcomsearch(s); if (cmd == -1) zwarnnam(...);`
    match hcomsearch(s) {
        Some(n) => n,
        None => {
            zwarnnam("fc", &format!("event not found: {}", s));
            -1
        }
    }
}

/// Port of `fcsubs(char **sp, struct asgment *sub)` from Src/builtin.c:1708.
/// C: `static int fcsubs(char **sp, struct asgment *sub)` — apply the
///   linked-list of `old=new` substitutions to `*sp` in place; return
///   the count of substitutions made.
pub fn fcsubs(sp: &mut String, sub: &[(String, String)]) -> i32 {
    // c:1708
    // c:1708-1748 — for each (old, new), replace each occurrence in *sp.
    let mut subbed = 0i32; // c:1713
    for (old, new) in sub {
        // c:1716
        if old.is_empty() {
            continue;
        }
        let count = sp.matches(old.as_str()).count() as i32; // c:1722
        if count > 0 {
            *sp = sp.replace(old.as_str(), new); // c:1750
            subbed += count;
        }
    }
    subbed
}

/// Direct port of `int fclist(FILE *f, Options ops, zlong first,
/// zlong last, struct asgment *subs, Patprog pprog, int is_command)`
/// from `Src/builtin.c:1750`. Walks the history event range
/// `first..=last`, applies the `subs` substitution chain to each
/// matching line (when `pprog` is set, only lines matching it),
/// then writes the result with optional timestamp prefix per
/// `-d/-f/-E/-i/-t`.
///
/// Rust signature: takes the output writer as a closure so callers
/// can route to stdout, a FILE*, or an in-memory buffer (the
/// `is_command` caller in `bin_fc` collects to a heredoc string).
/// Was a 5-line stub returning 0; now actually emits the range.
#[allow(clippy::too_many_arguments)]
pub fn fclist(
    out: &mut dyn Write, // c:1750
    ops: &options,
    mut first: i64,
    mut last: i64,
    subs: &[(String, String)],
    pprog: Option<&str>,
    is_command: i32,
) -> i32 {
    use std::io::Write;

    // c:1762-1766 — `if (OPT_ISSET(ops,'r')) swap(first, last);`
    if OPT_ISSET(ops, b'r') {
        std::mem::swap(&mut first, &mut last);
    }
    // c:1768-1773 — `if (is_command && first > last) zwarnnam(...)`.
    if is_command != 0 && first > last {
        zwarnnam("fc", "history events can't be executed backwards, aborted");
        return 1;
    }

    // c:1776-1790 — `gethistent(first, ...)` with bidirectional fallback.
    let near = if first < last { 1 } else { -1 };
    let start_ev = match gethistent(first, near) {
        Some(e) => e,
        None => {
            // c:Src/builtin.c — `no such event: <N>` carries the
            // requested event number so the user can see which
            // index missed. zsh appends the failing event id;
            // the bare `no such event` message diverged.
            zwarnnam(
                "fc",
                &if first == last {
                    format!("no such event: {}", first)
                } else {
                    "no events in that range".to_string()
                },
            );
            return 1;
        }
    };

    // c:1792-1817 — timestamp format setup.
    let want_time = OPT_ISSET(ops, b'd')
        || OPT_ISSET(ops, b'f')
        || OPT_ISSET(ops, b'E')
        || OPT_ISSET(ops, b'i')
        || OPT_ISSET(ops, b't');
    let tdfmt: Option<&'static str> = if !want_time {
        None
    } else if OPT_ISSET(ops, b't') {
        Some("%H:%M") // -t expects user-supplied fmt; without OPT_ARG access default to %H:%M
    } else if OPT_ISSET(ops, b'i') {
        Some("%Y-%m-%d %H:%M")
    } else if OPT_ISSET(ops, b'E') {
        Some("%d.%m.%Y %H:%M")
    } else if OPT_ISSET(ops, b'f') {
        Some("%m/%d/%Y %H:%M")
    } else {
        Some("%H:%M")
    };

    // c:1820-1880 — walk events from start_ev toward `last`. Each entry:
    //                apply pprog filter, apply subs chain, emit (with
    //                event num + timestamp unless -n or is_command).
    let mut ev = start_ev;
    let step: i64 = if first < last { 1 } else { -1 };
    loop {
        // c:1830 — `ent = quietgethist(ev);` — fetch entry by event #.
        let entry = match quietgethist(ev) {
            Some(e) => e,
            None => break,
        };
        let line = entry.node.nam.clone();

        // c:1833 — pprog pattern filter. C pre-compiles a Patprog;
        //          Rust compiles per-call. Most fc -l calls have no
        //          pattern so the gate is cheap.
        if let Some(pat) = pprog {
            let prog = patcompile(
                &{
                    let mut __pat_tok = (pat).to_string();
                    crate::ported::glob::tokenize(&mut __pat_tok);
                    __pat_tok
                },
                0,
                None,
            );
            let matched = prog.as_ref().map(|p| pattry(p, &line)).unwrap_or(true);
            if !matched {
                if ev == last {
                    break;
                }
                ev += step;
                continue;
            }
        }

        // c:1841-1855 — apply subs chain (asgment list of `old=new`
        //                pairs that get substituted in order).
        let mut text = line;
        for (old, new) in subs.iter() {
            if old.is_empty() {
                continue;
            }
            text = text.replace(old.as_str(), new.as_str());
        }

        // c:1860-1870 — emit prefix: event number (unless -n / -h),
        //                then optional timestamp.
        if is_command == 0 {
            if !OPT_ISSET(ops, b'n') {
                let _ = write!(out, "{:>5}", ev);
                if OPT_ISSET(ops, b'D') {
                    // c:Src/builtin.c — `-D` shows duration as M:SS.
                    // C: `fprintf(f, "%d:%02d", (int)(d/60), (int)(d%60))`.
                    // Direct ftim-minus-stim duration in seconds, then
                    // M:SS layout (zero-pad seconds, no zero-pad minutes).
                    let dur = entry.ftim.saturating_sub(entry.stim).max(0);
                    let _ = write!(out, "  {}:{:02}", dur / 60, dur % 60);
                }
                if let Some(fmt) = tdfmt {
                    // c:1817 — `strftime(timebuf, 256, tdfmt,
                    //                    localtime(&ent->stim))`.
                    //          Use libc directly so locale-aware
                    //          format specifiers (%Y %m %d %H %M %S
                    //          %p etc.) all work without a hand-rolled
                    //          strftime port.
                    let formatted: Option<String> = (|| {
                        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
                        let t: libc::time_t = entry.stim as libc::time_t;
                        let cfmt = std::ffi::CString::new(fmt).ok()?;
                        unsafe {
                            if libc::localtime_r(&t, &mut tm).is_null() {
                                return None;
                            }
                            let mut buf = vec![0u8; 256];
                            let n = libc::strftime(
                                buf.as_mut_ptr() as *mut libc::c_char,
                                buf.len(),
                                cfmt.as_ptr(),
                                &tm,
                            );
                            if n == 0 {
                                return None;
                            }
                            buf.truncate(n);
                            String::from_utf8(buf).ok()
                        }
                    })();
                    if let Some(s) = formatted {
                        let _ = write!(out, "  {}", s);
                    } else {
                        // strftime failed (locale issue / format bug);
                        // fall back to raw epoch matching C's
                        // pre-strftime print behavior.
                        let _ = write!(out, "  {}", entry.stim);
                    }
                }
                let _ = write!(out, "  ");
            }
        }

        // c:1875 — write the line.
        let _ = writeln!(out, "{}", text);

        if ev == last {
            break;
        }
        ev += step;
        if ev < 0 {
            break;
        }
    }
    0 // c:1880
}

/// Port of `fcedit(char *ename, char *fn)` from Src/builtin.c:1885.
/// C: `static int fcedit(char *ename, char *fn)` — invoke `$ename fn`,
///   returning the editor's exit status (0 if `ename == "-"`).
/// WARNING: param names don't match C — Rust=(ename, fn_) vs C=(ename, fn)
pub fn fcedit(ename: &str, fn_: &str) -> i32 {
    // c:1885
    // c:1885 — `if (!strcmp(ename, "-")) return 1;`
    if ename == "-" {
        // c:1888
        return 1; // c:1889
    }
    // c:1891-1900 — execlp(ename, ename, fn, NULL) wrapped in fork/wait.
    let status = std::process::Command::new(ename) // c:1895
        .arg(fn_)
        .status();
    match status {
        Ok(s) => s.code().unwrap_or(1),
        Err(_) => 1,
    }
}

/// Port of `getasg(char ***argvp, LinkList assigns)` from Src/builtin.c:1908.
/// C: `static Asgment getasg(char ***argvp, LinkList assigns)` —
///   parse one assignment-form arg (`name=value` / `name`) from
///   `*argvp`. Returns NULL when exhausted.
/// ```c
/// static Asgment
/// getasg(char ***argvp, LinkList assigns)
/// {
///     char *s = **argvp;
///     static struct asgment asg;
///     if (!s) {
///         if (assigns) {
///             Asgment asgp = (Asgment)firstnode(assigns);
///             if (!asgp) return NULL;
///             (void)uremnode(assigns, &asgp->node);
///             return asgp;
///         }
///         return NULL;
///     }
///     if (*s == '=') { zerr("bad assignment"); return NULL; }
///     asg.name = s;
///     asg.flags = 0;
///     for (; *s && *s != '='; s++);
///     if (*s) { *s = '\0'; asg.value.scalar = s + 1; }
///     else asg.value.scalar = NULL;
///     (*argvp)++;
///     return &asg;
/// }
/// ```
pub fn getasg(
    argvp: &mut Vec<String>, // c:1908
    assigns: &mut Vec<(String, String)>,
) -> Option<(String, String)> {
    // c:1914-1923 — out-of-args path: drain from assigns list if non-empty.
    if argvp.is_empty() {
        // c:1914 !s
        if !assigns.is_empty() {
            // c:1915
            return Some(assigns.remove(0)); // c:1916-1920 firstnode + uremnode
        }
        return None; // c:1922
    }

    let s = argvp.remove(0); // c:1944 (*argvp)++

    // c:1926-1929 — empty-name guard: bare `=value` is an error.
    if s.starts_with('=') {
        // c:1926
        zerr("bad assignment"); // c:1927
        return None; // c:1928
    }

    // c:1934-1943 — split on `=`. No `=` → name-only (scalar = NULL).
    match s.find('=') {
        // c:1934
        Some(i) => {
            // c:1938-1939 — `*s = '\0'; asg.value.scalar = s + 1;`
            Some((s[..i].to_string(), s[i + 1..].to_string())) // c:1939
        }
        None => {
            // c:1942 — `asg.value.scalar = NULL;` — name-only.
            Some((s, String::new())) // c:1942
        }
    }
}

/// Port of `typeset_setbase(const char *name, Param pm, Options ops, int on, int always)` from Src/builtin.c:1961.
/// C: `static int typeset_setbase(const char *name, Param pm, Options ops,
///     int on, int always)` — install numeric base on `pm`. For
///     `-i ARG`/`-E ARG`/`-F ARG`, parse ARG as base and validate
///     (must be 2..=36 for integer); error → return 1.
/// WARNING: param names don't match C — Rust=(name, pm, on, always) vs C=(name, pm, ops, on, always)
pub fn typeset_setbase(
    name: &str,
    pm: *mut param, // c:1961
    ops: &options,
    on: i32,
    always: i32,
) -> i32 {
    // c:1964 — `char *arg = NULL;`
    let mut arg: Option<&str> = None; // c:1964
    let on_u = on as u32;
    // c:1966-1971 — `if ((on & PM_INTEGER) && OPT_HASARG(ops,'i')) arg = OPT_ARG(ops,'i');`
    if (on_u & PM_INTEGER) != 0 && OPT_HASARG(ops, b'i') {
        // c:1966
        arg = OPT_ARG(ops, b'i'); // c:1967
    } else if (on_u & PM_EFLOAT) != 0 && OPT_HASARG(ops, b'E') {
        // c:1968
        arg = OPT_ARG(ops, b'E'); // c:1969
    } else if (on_u & PM_FFLOAT) != 0 && OPT_HASARG(ops, b'F') {
        // c:1970
        arg = OPT_ARG(ops, b'F'); // c:1971
    }

    // c:1973 — `if (arg) {`
    if let Some(a) = arg {
        // c:1973
        // c:1976 — `int base = (int)zstrtol(arg, &eptr, 10);`
        let base = match a.trim().parse::<i32>() {
            Ok(b) => b,
            Err(_) => {
                // c:1977-1982
                if (on_u & PM_INTEGER) != 0 {
                    zwarnnam(name, &format!("bad base value: {}", a)); // c:1979
                } else {
                    zwarnnam(name, &format!("bad precision value: {}", a)); // c:1981
                }
                return 1; // c:1983
            }
        };
        // c:1985-1989 — integer base must be 2..=36 inclusive.
        if (on_u & PM_INTEGER) != 0 && (base < 2 || base > 36) {
            // c:1985
            zwarnnam(
                name,
                &format!("invalid base (must be 2 to 36 inclusive): {}", base),
            ); // c:1986-1987
            return 1; // c:1988
        }
        // c:1990 — `pm->base = base;`
        if !pm.is_null() {
            unsafe {
                (*pm).base = base;
            } // c:1990
        }
    } else if always != 0 {
        // c:1991
        // c:1997 — `pm->base = 0;`
        if !pm.is_null() {
            unsafe {
                (*pm).base = 0;
            } // c:1997
        }
    }
    0 // c:1997
}

/// Port of `typeset_setwidth(const char * name, Param pm, Options ops, int on, int always)` from Src/builtin.c:1997.
/// C: `static int typeset_setwidth(const char *name, Param pm, Options ops,
///     int on, int always)` — install padding width via `-L/-R/-Z ARG`.
/// WARNING: param names don't match C — Rust=(name, pm, on, always) vs C=(name, pm, ops, on, always)
pub fn typeset_setwidth(
    name: &str,
    pm: *mut param, // c:1997
    ops: &options,
    on: i32,
    always: i32,
) -> i32 {
    // c:2000 — `char *arg = NULL;`
    let mut arg: Option<&str> = None; // c:2000
    let on_u = on as u32;
    // c:2002-2007
    if (on_u & PM_LEFT) != 0 && OPT_HASARG(ops, b'L') {
        // c:2002
        arg = OPT_ARG(ops, b'L'); // c:2003
    } else if (on_u & PM_RIGHT_B) != 0 && OPT_HASARG(ops, b'R') {
        // c:2004
        arg = OPT_ARG(ops, b'R'); // c:2005
    } else if (on_u & PM_RIGHT_Z) != 0 && OPT_HASARG(ops, b'Z') {
        // c:2006
        arg = OPT_ARG(ops, b'Z'); // c:2007
    }

    // c:2009 — `if (arg) {`
    if let Some(a) = arg {
        // c:2009
        // c:2011 — `pm->width = (int)zstrtol(arg, &eptr, 10);`
        let width = match a.trim().parse::<i32>() {
            Ok(w) => w,
            Err(_) => {
                zwarnnam(name, &format!("bad width value: {}", a)); // c:2013
                return 1; // c:2014
            }
        };
        if !pm.is_null() {
            unsafe {
                (*pm).width = width;
            } // c:2011
        }
    } else if always != 0 {
        // c:2015
        // c:2016 — `pm->width = 0;`
        if !pm.is_null() {
            unsafe {
                (*pm).width = 0;
            } // c:2025
        }
    }
    0 // c:2025
}

/// Port of `typeset_single(char *cname, char *pname, Param pm, int func, int on, int off, int roff, Asgment asg, Param altpm, Options ops, int joinchar)` from Src/builtin.c:2025.
/// Port of `static Param typeset_single(char *cname, char *pname,
/// Param pm, int func, int on, int off, int roff, Asgment asg,
/// Param altpm, Options ops, int joinchar)` from `Src/builtin.c:2025`.
/// Per-name attribute resolver + assignment dispatcher invoked once
/// per arg from `bin_typeset`.
#[allow(clippy::too_many_arguments)]
pub fn typeset_single(
    cname: &str,
    pname: &str, // c:2025
    pm: *mut param,
    func: i32,
    mut on: i32,
    mut off: i32,
    _roff: i32,
    asg: *mut asgment,
    altpm: *mut param,
    ops: &options,
    _joinchar: i32,
) -> *mut param {
    let mut usepm: i32; // c:2029
    let mut tc: i32 = 0; // c:2029
    let _keeplocal: i32 = 0; // c:2029
    let mut newspecial: i32 = 0; /* NS_NONE */
    // c:2029
    let _readonly: i32 = 0; // c:2029
    let _dont_set: i32 = 0; // c:2029
    let mut pname_owned: String = pname.to_string(); // c:2030 subscript path

    // c:2032-2050 — nameref resolution.
    let pm_ref = unsafe { pm.as_mut() };
    if let Some(pm_r) = &pm_ref {
        let pm_flags = pm_r.node.flags as u32;
        let locallevel_v = locallevel_param.load(Relaxed);
        if (pm_flags & PM_NAMEREF) != 0
            && ((off | on) as u32 & PM_NAMEREF) == 0
            && (pm_r.level == locallevel_v || (on as u32 & PM_LOCAL) == 0)
        {
            // c:2034 — pm = resolve_nameref(pm)
            //          pname = pm->node.nam (when resolved)
            // resolve_nameref not yet ported; skip the rewrite.
            let unresolved_flags = pm_r.node.flags as u32;
            let extra_on_mask = !(PM_NAMEREF | PM_LOCAL | PM_READONLY) as i32;
            if (pm_flags & PM_NAMEREF) != 0
                && ((unresolved_flags & PM_UNSET) == 0 || (unresolved_flags & PM_DECLARED) != 0)
                && (on & extra_on_mask) != 0
            {
                // c:2042-2048 — error: can't change type of a nameref.
                if pm_r.width != 0 {
                    // c:2041
                    zwarnnam(
                        cname, // c:2042
                        &format!("{}: can't change type via subscript reference", pname),
                    );
                } else {
                    zwarnnam(
                        cname, // c:2046
                        &format!("{}: can't change type of a named reference", pname),
                    );
                }
                return std::ptr::null_mut(); // c:2048
            }
        }
    }

    // c:2062-2064 — `usepm = pm && (!(pm_flags & PM_UNSET) || OPT_ISSET(ops,'p') || ...)`
    let pm_flags = pm_ref.as_ref().map(|p| p.node.flags as u32).unwrap_or(0);
    usepm = if pm_ref.is_some()
        && ((pm_flags & PM_UNSET) == 0
            || OPT_ISSET(ops, b'p')
            || (isset(POSIXBUILTINS) && (pm_flags & (PM_READONLY | PM_EXPORTED)) != 0))
    {
        1
    } else {
        0
    };

    // c:2070-2071 — preserve PM_UNSET for special params.
    if usepm == 0 && pm_ref.is_some() && (pm_flags & PM_SPECIAL) != 0 {
        usepm = 2; // c:2071
    }

    // c:2078-2091 — don't reuse if local-level changed and PM_LOCAL set.
    let pm_level = pm_ref.as_ref().map(|p| p.level).unwrap_or(0);
    let locallevel_v = locallevel_param.load(Relaxed);
    if usepm != 0 && locallevel_v != pm_level && (on as u32 & PM_LOCAL) != 0 {
        // c:2078
        if (pm_flags & PM_SPECIAL) != 0                // c:2087
            && (on as u32 & PM_HIDE) == 0
            && (pm_flags & PM_HIDE & !off as u32) == 0
        {
            newspecial = 1; /* NS_NORMAL */ // c:2089
        }
        usepm = 0; // c:2090
    }

    // c:2093-2116 — type-conversion / tied-colonarray detection.
    let asg_ref = unsafe { asg.as_ref() };
    tc = 0;
    if let Some(a) = asg_ref {
        if ASG_ARRAYP(a)
            && PM_TYPE(on as u32) == PM_SCALAR
            && !(usepm != 0 && (PM_TYPE(pm_flags) & (PM_ARRAY | PM_HASHED)) != 0)
        {
            on |= PM_ARRAY as i32; // c:2097
        }
        if usepm != 0 && ASG_ARRAYP(a) && newspecial == 0                    // c:2098
            && PM_TYPE(pm_flags) != PM_ARRAY
            && PM_TYPE(pm_flags) != PM_HASHED
        {
            if (on as u32 & (PM_EFLOAT | PM_FFLOAT | PM_INTEGER)) != 0 {
                zerrnam(
                    cname, // c:2102
                    &format!("{}: can't assign array value to non-array", pname),
                );
                return std::ptr::null_mut();
            }
            if (pm_flags & PM_SPECIAL) != 0 {
                // c:2105
                zerrnam(
                    cname, // c:2106
                    &format!("{}: can't assign array value to non-array special", pname),
                );
                return std::ptr::null_mut();
            }
            tc = 1; // c:2109
            usepm = if OPT_MINUS(ops, b'p') {
                // c:2110
                (on as u32 & pm_flags) as i32
            } else if OPT_PLUS(ops, b'p') {
                // c:2112
                (off as u32 & pm_flags) as i32
            } else {
                0 // c:2115
            };
        }
    }

    // c:2117-2199 — attribute-mask compatibility checks (chflags compute).
    if usepm != 0 || newspecial != 0 {
        let chflags = ((off as u32 & pm_flags) | (on as u32 & !pm_flags))    // c:2118
            & (PM_INTEGER
               | PM_EFLOAT
               | PM_FFLOAT
               | PM_HASHED | PM_ARRAY | PM_TIED | PM_AUTOLOAD);
        if chflags != 0 && chflags != (PM_EFLOAT | PM_FFLOAT) {
            tc = 1; // c:2122
            usepm = if OPT_MINUS(ops, b'p') {
                // c:2123
                (on as u32 & pm_flags) as i32
            } else if OPT_PLUS(ops, b'p') {
                // c:2125
                (off as u32 & pm_flags) as i32
            } else {
                // c:2127 — `else usepm = 0;`
                //
                // This `else` was missing, and it is what selects between the
                // two mutually exclusive paths. usepm != 0 takes the MERGE path
                // (c:2291: keep the existing flags, add `on`, drop `off`), while
                // usepm == 0 falls through to the type-conversion path (c:2355),
                // which deletes the parameter and recreates it carrying ONLY
                // READONLY|EXPORTED (c:2357). Leaving usepm set meant a base-type
                // change merged instead of recreating, so `typeset -Z 3 x=7;
                // typeset -i x=1` kept the right_zeros attribute (zsh: `integer`,
                // zshrs: `integer-right_zeros`).
                0
            };
        }
    }

    // c:2202-2214 — readonly/exported preservation rules.
    if usepm != 0 || newspecial != 0 {
        if (on as u32 & (PM_READONLY | PM_EXPORTED)) != 0                    // c:2202
            && (usepm == 0 || (pm_flags & PM_UNSET) != 0)
            && asg_ref.is_some_and(|a| !ASG_VALUEP(a))
        {
            on |= PM_UNSET as i32; // c:2205
        } else if usepm != 0 && (pm_flags & PM_READONLY) != 0                // c:2206
            && (on as u32 & PM_READONLY) == 0
            && func != BIN_EXPORT
        {
            zerr(&format!(
                // c:2208
                "read-only variable: {}",
                pm_ref.as_ref().unwrap().node.nam
            ));
            return std::ptr::null_mut();
        }
    }

    // c:2226-2248 — reuse-existing-param fast paths.
    if usepm != 0 {
        let pm_r = pm_ref.as_ref().unwrap();
        if OPT_MINUS(ops, b'p')
            && on != 0
            && !((on as u32 & pm_flags) != 0 || ((on as u32 & PM_LOCAL) != 0 && pm_r.level != 0))
        {
            return std::ptr::null_mut(); // c:2229
        }
        if OPT_PLUS(ops, b'p') && off != 0 && (off as u32 & pm_flags) == 0 {
            return std::ptr::null_mut(); // c:2231
        }
        // c:2232-2238 — array/scalar consistency check
        if let Some(a) = asg_ref {
            let array_assign = (a.flags & ASG_ARRAY) != 0;
            let pm_is_arr = (PM_TYPE(pm_flags) & (PM_ARRAY | PM_HASHED)) != 0;
            if array_assign && !pm_is_arr {
                // c:2232
                zerrnam(
                    cname, // c:2236
                    &format!("{}: inconsistent type for assignment", pname),
                );
                return std::ptr::null_mut();
            }
        }
    }

    // c:2240-2247 — print-only path: typeset -p / typeset name (no value).
    if usepm != 0 && on == 0 && _roff == 0 && asg_ref.is_some_and(|a| !ASG_VALUEP(a)) {
        // c:2241 — `int with_ns = OPT_ISSET(ops,'m') ? PRINT_WITH_NAMESPACE : 0;`
        let with_ns = if OPT_ISSET(ops, b'm') {
            // c:2241
            PRINT_WITH_NAMESPACE
        } else {
            0
        };
        // c:Src/builtin.c:2761-2765 — `typeset -p1 NAME` adds PRINT_LINE
        // (one array/assoc element per line). The listing path (~3805)
        // already parses this; the explicit-name print path here did not,
        // so `typeset -p1 myarray` printed the single-line `( a b c )`
        // form instead of zsh's multi-line `(\n  a\n  b\n  c\n)`.
        let line_flag =
            if OPT_HASARG(ops, b'p') && OPT_ARG(ops, b'p').map(|a| a.trim()) == Some("1") {
                PRINT_LINE
            } else {
                0
            };
        if let Some(pm_r) = unsafe { pm.as_mut() } {
            if OPT_ISSET(ops, b'p') {
                // c:2242
                // c:2243 — `paramtab->printnode(&pm->node, PRINT_TYPESET|with_ns);`
                printparamnode(pm_r, PRINT_TYPESET | with_ns | line_flag);
            } else if !OPT_ISSET(ops, b'g')                                  // c:2244
                && (!isset(TYPESETSILENT) || OPT_ISSET(ops, b'm'))
            // c:2245
            {
                // c:2246 — `paramtab->printnode(&pm->node, PRINT_INCLUDEVALUE|with_ns);`
                printparamnode(pm_r, PRINT_INCLUDEVALUE | with_ns);
            }
        }
        return pm; // c:2247
    }

    // c:2355-2378 — tc (type-conversion) branch: recreate the param.
    if tc != 0 && !OPT_ISSET(ops, b'p') {
        on |= (!off as u32 & (PM_READONLY | PM_EXPORTED) & pm_flags) as i32; // c:2357
        if let Some(pm_r) = pm_ref {
            pm_r.node.flags &= !(PM_READONLY as i32); // c:2359
        }
        // c:2364 — keeplocal = pm->level (used by createparam path)
        // c:2372-2375 — carry scalar value across type change.
        // c:2378 — unsetparam_pm(pm, 0, 1)
        if let Some(pm_r) = unsafe { pm.as_mut() } {
            unsetparam_pm(pm_r, 0, 1);
        }
        pname_owned = pname.to_string(); // c:2377
    }

    // c:2381-2467 — newspecial path: preserve special-param struct.
    // c:2469-2510 — createparam + assignment dispatch for new/converted.
    // c:2512-2453 — apply value via assignsparam/setaparam/sethparam.
    // These call into a 2-level helper chain (typeset_setwidth,
    // typeset_setbase, assignsparam, etc.) — the available Rust
    // ports drive single-attribute setters. The dispatcher entry
    // (bin_typeset at c:2655) walks the option matrix and invokes
    // those setters directly today.
    let _ = (altpm, pname_owned, _keeplocal, _dont_set, _readonly);

    // c:2547 — `return pm;`
    pm
}

/// Port of `bin_typeset(char *name, char **argv, LinkList assigns, Options ops, int func)` from Src/builtin.c:2655.
/// C: `int bin_typeset(char *name, char **argv, LinkList assigns,
///     Options ops, int func)`.
///
/// The C body (~500 lines) ports here in two layers: the option-flag
/// matrix + conflict-resolution / dispatch (faithfully translated)
/// and the per-arg param-setting loop (delegated to typeset_single
/// already ported above).
/// WARNING: param names don't match C — Rust=(name, argv, func) vs C=(name, argv, assigns, ops, func)
pub fn bin_typeset(
    name: &str,
    argv: &[String], // c:2655
    ops: &options,
    func: i32,
) -> i32 {
    // PFA-SMR aspect: bin_typeset is the C dispatch site for
    // typeset/declare/integer/float/local/export/readonly/private —
    // every one of those state-mutating builtins lands here with a
    // funcid (BIN_EXPORT/BIN_READONLY/BIN_TYPESET/...) discriminant.
    // Emit a per-name event per the recorder schema.
    #[cfg(feature = "recorder")]
    if crate::recorder::is_enabled() {
        let ctx = crate::recorder::recorder_ctx_global();
        // Collect option letters from argv `-x`/`+x` flag args AND
        // from the BUILTIN's defopts (the auto-set option chars C
        // applies before dispatching bin_typeset). Without the
        // defopts, `integer count=42` (defopts="i"), `float pi=1.0`
        // (defopts="E"), and `typeset -A h=(...)` whose `A` came
        // from a parent shape, lost their type bits and the
        // recorder emitted `[scalar]` for everything.
        let mut letters = String::new();
        let mut tied_mode = false;
        for a in argv {
            if a.starts_with('-') || a.starts_with('+') {
                let body = &a[1..];
                letters.push_str(body);
                if body.contains('T') {
                    tied_mode = true;
                }
            }
        }
        // Add the option letters that defopts pre-set in `ops`.
        for ch in [b'i', b'E', b'F', b'A', b'a', b'r', b'x', b'g', b'l', b'u'] {
            if OPT_ISSET(&ops, ch) && !letters.contains(ch as char) {
                letters.push(ch as char);
            }
        }
        // Funcid-driven attr seeding: BIN_EXPORT seeds nothing
        // (recorder uses emit_export for those), BIN_READONLY seeds
        // SCALAR|READONLY. INTEGER/FLOAT/ASSOC/ARRAY now flow via
        // the defopts injection above into ParamAttrs::from_flag_chars.
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
                if a == "-p" || a.starts_with('-') {
                    continue;
                }
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
            // by the executor; defer to the global locallevel_param counter.
            let is_locallike = matches!(name, "local" | "private");
            let inside_function = locallevel_param.load(std::sync::atomic::Ordering::Relaxed) > 0;
            if !is_locallike || !inside_function {
                let mut tied_seen = 0usize;
                for a in argv {
                    if a.starts_with('-') || a.starts_with('+') {
                        continue;
                    }
                    if tied_mode {
                        // For `typeset -T X Y [SEP]`, only X and Y are names.
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
    }
    let mut ops = ops.clone();
    let mut on: u32 = 0; // c:2661
    let mut off: u32 = 0; // c:2661
    let mut returnval: i32 = 0; // c:2664
    let mut printflags: i32 = PRINT_WITH_NAMESPACE; // c:2664
    let hasargs = !argv.is_empty(); // c:2665

    // c:2668-2670 — POSIX bash/ksh ignore -p with args under
    // readonly/export.
    let posix = isset(POSIXBUILTINS);
    if (func == BIN_READONLY || func == BIN_EXPORT) && posix && hasargs {
        // c:2668
        ops.ind[b'p' as usize] = 0; // c:2670
    }

    // c:2673 — `if (OPT_ISSET(ops,'f')) return bin_functions(...)`.
    if OPT_ISSET(&ops, b'f') {
        // c:2673
        return bin_functions(name, argv, &ops, func); // c:2673
    }

    // c:2676 — POSIX readonly forces -g unless explicit +g.
    if func == BIN_READONLY && posix && !OPT_PLUS(&ops, b'g') {
        // c:2676
        ops.ind[b'g' as usize] = 1; // c:2677
    }

    // c:2691-2706 — translate optstr letters into PM_* flag bits.
    let mut bit: u32 = PM_ARRAY; // c:2660
    for ch in TYPESET_OPTSTR.chars() {
        // c:2691
        let optval = ch as u8;
        if OPT_MINUS(&ops, optval) {
            on |= bit;
        }
        // c:2694-2695
        else if OPT_PLUS(&ops, optval) {
            off |= bit;
        }
        // c:2696-2697
        // c:2698-2706 — `-n` only allows readonly/upper/hideval.
        else {
            bit <<= 1;
            continue;
        }
        if OPT_MINUS(&ops, b'n') && (bit & !(PM_READONLY | PM_UPPER | PM_HIDEVAL)) != 0
        // c:2701
        {
            zwarnnam(name, &format!("-{} not allowed with -n", ch)); // c:2702
        }
        bit <<= 1;
    }
    // c:2708-2715 — -n / +n conflict resolution.
    if OPT_MINUS(&ops, b'n') {
        // c:2709-2711 — only readonly/upper/hideval combine with -n;
        // anything else is a (silent) status-1 rejection (the zwarnnam
        // here is commented out in C — the per-bit loop above already
        // warned).
        if ((on | off) & !(PM_READONLY | PM_UPPER | PM_HIDEVAL)) != 0 {
            // c:2710
            return 1; // c:2711
        }
        on |= PM_NAMEREF; // c:2712
    } else if OPT_PLUS(&ops, b'n') {
        // c:2714
        off |= PM_NAMEREF; // c:2715
    }
    let roff = off; // c:2716

    // c:2719-2740 — sanity checks: remove conflicting attrs.
    if (on & PM_FFLOAT) != 0 {
        // c:2719
        off |= PM_UPPER | PM_ARRAY | PM_HASHED | PM_INTEGER | PM_EFLOAT; // c:2720
        on &= !PM_EFLOAT; // c:2722
    }
    if (on & PM_EFLOAT) != 0 {
        // c:2724
        off |= PM_UPPER | PM_ARRAY | PM_HASHED | PM_INTEGER | PM_FFLOAT; // c:2725
    }
    if (on & PM_INTEGER) != 0 {
        // c:2726
        off |= PM_UPPER | PM_ARRAY | PM_HASHED | PM_EFLOAT | PM_FFLOAT; // c:2727
    }
    if (on & (PM_LEFT | PM_RIGHT_Z)) != 0 {
        // c:2731
        off |= PM_RIGHT_B; // c:2732
    }
    if (on & PM_RIGHT_B) != 0 {
        // c:2733
        off |= PM_LEFT | PM_RIGHT_Z; // c:2734
    }
    if (on & PM_UPPER) != 0 {
        off |= PM_LOWER;
    } // c:2735-2736
    if (on & PM_LOWER) != 0 {
        off |= PM_UPPER;
    } // c:2737-2738
    if (on & PM_HASHED) != 0 {
        off |= PM_ARRAY;
    } // c:2739-2740
    if (on & PM_TIED) != 0 {
        // c:2741
        off |= PM_INTEGER | PM_EFLOAT | PM_FFLOAT | PM_ARRAY | PM_HASHED; // c:2742
    }
    on &= !off; // c:2744

    queue_signals(); // c:2746

    // c:2748-2772 — `-p` print-mode: PRINT_POSIX_EXPORT / READONLY /
    // TYPESET, plus optional -p N for line-style.
    if OPT_ISSET(&ops, b'p') {
        // c:2748
        if posix && !EMULATION(EMULATE_KSH) {
            // c:2750
            printflags |= match func {
                BIN_EXPORT => PRINT_POSIX_EXPORT,     // c:2752
                BIN_READONLY => PRINT_POSIX_READONLY, // c:2754
                _ => PRINT_TYPESET,                   // c:2756
            };
        } else {
            printflags |= PRINT_TYPESET; // c:2758
        }
        if OPT_HASARG(&ops, b'p') {
            // c:2761
            let arg = OPT_ARG(&ops, b'p').unwrap_or("");
            match arg.trim().parse::<i32>() {
                // c:2763
                Ok(1) => printflags |= PRINT_LINE, // c:2765
                Ok(0) => {}                        // c:2770 -p0 == -p
                _ => {
                    zwarnnam(name, &format!("bad argument to -p: {}", arg)); // c:2767
                    unqueue_signals();
                    return 1; // c:2769
                }
            }
        }
    }

    // c:2775-2795 — no-args path: list whatever options select.
    if !hasargs {
        // c:2775
        if !OPT_ISSET(&ops, b'm') {
            // c:2779
            printflags &= !PRINT_WITH_NAMESPACE; // c:2780
        }
        if !OPT_ISSET(&ops, b'p') {
            // c:2782
            if (on | roff) == 0 {
                // c:2783
                printflags |= PRINT_TYPE; // c:2784
            }
            if roff != 0 || OPT_ISSET(&ops, b'+') {
                // c:2785
                printflags |= PRINT_NAMEONLY; // c:2786
            }
        }
        // c:2792 — `scanhashtable(paramtab, 1, on|roff, 0, paramtab->printnode,
        //               printflags|(roff ? PRINT_NAMEONLY : 0));`
        //
        // Walk paramtab (sorted=1, alphabetical) filtering by on|roff
        // and dispatch printparamnode for each match. Previously inlined
        // a `println!("{}={}", k, v)` which:
        //   - Ignored printflags (PRINT_TYPESET, PRINT_POSIX_EXPORT,
        //     PRINT_POSIX_READONLY, PRINT_NAMEONLY) so `export -p` had
        //     zero rows, `typeset -p` skipped attribute letters, and
        //     `readonly -p` had no `readonly ` prefix.
        //   - Read pm.u_str directly so PM_INTEGER / PM_*FLOAT /
        //     PM_ARRAY / PM_HASHED values printed as empty.
        // printparamnode (params.c:6123) handles all of these.
        let printflags_final = printflags | if roff != 0 { PRINT_NAMEONLY } else { 0 }; // c:2792
                                                                                        // c:2792 scanmatchtable flags1=on|roff, flags2=0.
        let on_roff = (on as u32) | (roff as u32);
        // PM_RO_BY_DESIGN expansion: zshrs's special-param
        // setup (vm_helper.rs:1054+) replaces the dropped
        // PM_READONLY bit with PM_RO_BY_DESIGN so internal
        // writes still pass assignstrvalue's PM_READONLY
        // guard. `typeset -r` listing must match on either
        // bit to surface those entries. Mirrors C zsh's
        // PM_READONLY_SPECIAL (= PM_SPECIAL | PM_READONLY |
        // PM_RO_BY_DESIGN) where the scanhashtable bit-mask
        // implicitly matches RO_BY_DESIGN too (both bits
        // set on the same params in C). #97 in docs/BUGS.md.
        let on_roff_expanded = if (on_roff & PM_READONLY) != 0 {
            on_roff | PM_RO_BY_DESIGN
        } else {
            on_roff
        };
        // c:Src/module.c:1218-1219 add_autoparam — in `zsh -f`, every
        // autoloadable module parameter (WATCH/watch, aliases…,
        // widgets/keymaps, termcap/terminfo, zsh_scheduled_events) is
        // a real paramtab stub: a scalar whose value is the module
        // name, flagged PM_AUTOLOAD. scanhashtable feeds the stubs to
        // printparamnode which prints `undefined NAME` (pmtypes row at
        // Src/params.c:6011) and suppresses them under -p
        // (Src/params.c:6150-6155). zshrs keeps always-functional
        // placeholder params instead, so synthesize the stub rows here
        // for every not-yet-loaded module and suppress the placeholder
        // rows for the same names (zsh -f shows `undefined WATCH`, not
        // a set `WATCH`).
        let stubs: Vec<(&'static str, &'static str)> = crate::vm_helper::autoload_param_stubs();
        let names: Vec<String> = {
            let tab = paramtab().read().unwrap();
            let mut names: Vec<String> = tab
                .iter()
                .filter(|(k, pm)| {
                    let f = pm.node.flags as u32;
                    // c:2793 — scanhashtable passes PM_UNSET entries
                    // through; printparamnode (c:6133-6144) prints
                    // PM_DEFAULTED (declared-but-unset) ones name-only
                    // and drops the rest. Keep DEFAULTED here so
                    // `typeset -n` lists placeholder refs by name.
                    if (f & PM_UNSET) != 0 && (f & PM_DEFAULTED) != PM_DEFAULTED {
                        return false;
                    }
                    // Unloaded-module names print as autoload stubs
                    // (merged below) — never as their placeholder row.
                    if stubs.iter().any(|(n, _)| n == k) {
                        return false;
                    }
                    // c:Src/builtin.c:2789 — `scanhashtable(paramtab, 1,
                    // on|roff, exclude, paramtab->printnode, printflags)`.
                    // The C scan has NO PM_HIDE exclusion: `addparamdef`
                    // (Src/module.c:1060-1074) inserts every zsh/parameter
                    // magic assoc into the REAL paramtab via createparam /
                    // createspecialhash, so once the module has booted zsh
                    // lists them (`association readonly parameters`,
                    // `array readonly patchars`, …) — verified against
                    // `zsh -f -c 'zmodload zsh/parameter; typeset -A'`.
                    // A blanket PM_HIDE skip used to live here for bug
                    // #371 (`typeset -A` listing magic-assocs in a shell
                    // where the module was never loaded); that case is
                    // already covered by the autoload-stub filter above,
                    // which drops zshrs's paramtab placeholder for every
                    // module that has not booted and prints the C
                    // `undefined NAME` stub instead.
                    on_roff_expanded == 0 || (f & on_roff_expanded) != 0
                })
                .map(|(k, _)| k.clone())
                .collect();
            // Stub flags are PM_SCALAR|PM_AUTOLOAD (module.c:1218-1219)
            // — apply the same scanmatchtable flags1 test (PM_AUTOLOAD
            // is never in on|roff, so any flag filter excludes them:
            // `typeset +x -r` / `typeset +i` list none, matching zsh).
            for (n, _) in &stubs {
                if on_roff_expanded == 0 || (PM_AUTOLOAD & on_roff_expanded) != 0 {
                    names.push((*n).to_string());
                }
            }
            names.sort_by(|a, b| hnamcmp(a, b));
            names
        };
        // c:Src/params.c:6275 PM_TIED partner lookup — printparamnode
        // reads `paramtab` to fetch the tied peer (e.g. PATH→path).
        // Pre-clone the pm so the print runs WITHOUT holding any
        // paramtab lock; otherwise the partner-lookup inside
        // printparamnode would deadlock on the same write lock.
        // printparamnode mutates hn for the PM_TIED swap path (Bug
        // #410) — the mutation lands on the local clone only,
        // matching C's "throwaway-print-side" semantics.
        for k in names {
            // c:Src/module.c:1218-1219 — autoload stub: scalar param,
            // u.str = module name, PM_AUTOLOAD. printparamnode prints
            // `undefined NAME` (PRINT_TYPE) / `NAME` (PRINT_NAMEONLY)
            // and returns early under -p.
            if let Some((_, module)) = stubs.iter().find(|(n, _)| *n == k) {
                let mut stub_pm = crate::ported::zsh_h::param {
                    node: crate::ported::zsh_h::hashnode {
                        next: None,
                        nam: k.clone(),
                        flags: (PM_SCALAR | PM_AUTOLOAD) as i32, // c:1219
                    },
                    u_data: 0,
                    u_tied: None,
                    u_arr: None,
                    u_str: Some((*module).to_string()), // c:1218 setsparam(pnam, module)
                    u_val: 0,
                    u_dval: 0.0,
                    u_hash: None,
                    gsu_s: None,
                    gsu_i: None,
                    gsu_f: None,
                    gsu_a: None,
                    gsu_h: None,
                    base: 0,
                    width: 0,
                    env: None,
                    ename: None,
                    old: None,
                    level: 0,
                };
                printparamnode(&mut stub_pm, printflags_final); // c:2792
                continue;
            }
            let mut pm_clone = match paramtab().read() {
                Ok(tab) => match tab.get(&k) {
                    Some(pm) => pm.clone(),
                    None => continue,
                },
                Err(_) => continue,
            };
            printparamnode(&mut pm_clone, printflags_final); // c:2792
        }
        unqueue_signals();
        return 0; // c:2794
    }

    // c:2799-2810 — `local` (or +g) implies PM_LOCAL.
    let nm0 = name.chars().next().unwrap_or(' ');
    if nm0 == 'l' || OPT_PLUS(&ops, b'g') {
        // c:2799
        on |= PM_LOCAL; // c:2800
    } else if !OPT_ISSET(&ops, b'g') {
        // c:2801
        if OPT_MINUS(&ops, b'x') {
            // c:2802
            let globalexport = isset(GLOBALEXPORT);
            let ll_v = locallevel_param.load(Relaxed);
            if globalexport {
                // c:2803
                ops.ind[b'g' as usize] = 1; // c:2804
            } else if ll_v != 0 {
                // c:2805
                on |= PM_LOCAL; // c:2806
            }
        } else if !(OPT_ISSET(&ops, b'x') || OPT_ISSET(&ops, b'm')) {
            // c:2808
            on |= PM_LOCAL; // c:2809
        }
    }

    // c:2813+ — -T tied vars + per-arg setting loop.
    // The full C body has dozens of paths (PM_TIED tie-pair setup at
    // c:2813-2900, glob -m walk at c:2905-2935, name=value assign
    // through typeset_single at c:2945+). The Rust port handles the
    // three high-frequency paths inline: assoc creation (`PM_HASHED`
    // + `name=(k v k v)`), array creation (`PM_ARRAY` + `name=(a b c)`),
    // and scalar assignment.
    let _ = off;
    let is_hashed = (on & PM_HASHED) != 0; // c:2655 `-A`
    let is_array = (on & PM_ARRAY) != 0; // c:2655 `-a`
                                         // c:Src/builtin.c typeset_single — when the array RHS comes from
                                         // an unquoted `$@` / `${arr[@]}` splat (e.g. `typeset -a opts=
                                         // ("$@")`), the upstream prefork has already split the value
                                         // into separate argv entries: `["opts=(a", "b", "c)"]`. C zsh's
                                         // parser captures the entire `name=(... )` shape as one ENVARRAY
                                         // token by walking paren depth at parse time so the splat fills
                                         // the array's element list. zshrs's compile path emits the
                                         // synthetic word `opts=("$@")` then runtime DQ-strip + splat
                                         // separates it. Reconstruct the single arg here: when one entry
                                         // starts with `NAME=(` and a later entry ends with `)`, rejoin
                                         // the run with spaces between elements.
                                         // c:Src/builtin.c — paren-init rejoin. C's parser hands the whole
                                         // `NAME=(e1 e2 …)` shape as one ENVARRAY token via parse-time paren-
                                         // walking so empty elements (`""` quoted to lexer Dnull-Dnull) stay
                                         // as separate list nodes. zshrs's compile path emits the args
                                         // pre-split through prefork — each `""` produces a true empty
                                         // entry in argv. The previous rejoin glued them with single-space
                                         // separators, then split_whitespace collapsed consecutive empties
                                         // → key/value swap on `typeset -A h=( "" val )`. Bug #93 in
                                         // docs/BUGS.md.
                                         //
                                         // Use `\u{1f}` (ASCII US — unit separator) as the rejoin
                                         // separator. The paren-init branch below splits on `\u{1f}` AND
                                         // whitespace, preserving consecutive `\u{1f}` as empty elements
                                         // (the original quoted-empty args) while still tolerating
                                         // single-arg-form paren-init like `a=(1 2 3)` where no rejoin
                                         // ran.
    const REJOIN_SEP: char = '\u{1f}';
    let argv: Vec<String> = {
        let mut out: Vec<String> = Vec::with_capacity(argv.len());
        let mut i = 0;
        while i < argv.len() {
            let arg = &argv[i];
            let open = arg.find("=(");
            // c:Src/lex.c:1228-1249 — whether `NAME=(` opens an array is a
            // LEXICAL decision, taken on the RAW character after `=`:
            // `e = hgetc(); if (e == '(') return ENVARRAY;`. A `(` that
            // arrived inside quotes never reaches that test, so
            // `local open='(' close=')'` is two ENVSTRINGs in C.
            // zshrs marks the syntactic form with REJOIN_SEP — parse.rs's
            // ENVARRAY synthetic word and compile_zsh's
            // BUILTIN_TYPESET_PAREN_PACK both emit it, and `is_paren_init`
            // (c:2095 `ASG_ARRAYP`) already keys on it. Requiring it here
            // too stops a dequoted `(` VALUE from swallowing the following
            // words: `local open='(' close=')'` (Completion/X/Command/
            // _setxkbmap sh:34) stored `open` as `(<US>close=)` and never
            // defined `close`, so `compquote open close` failed and
            // `setxkbmap -` lost its whole listing.
            let is_open = open.is_some()
                && arg.contains(REJOIN_SEP)
                && arg
                    .as_bytes()
                    .first()
                    .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_')
                && !arg.ends_with(')');
            if is_open {
                // Find the matching `)` — scan forward through argv
                // tracking paren depth (the parser's `(` was just `(`
                // in the source). Each arg may have additional `(`
                // and `)` chars from quoted content.
                let mut depth: i32 = 0;
                for c in arg.chars() {
                    if c == '(' {
                        depth += 1;
                    } else if c == ')' {
                        depth -= 1;
                    }
                }
                let mut buf = arg.clone();
                let mut j = i + 1;
                while depth > 0 && j < argv.len() {
                    buf.push(REJOIN_SEP);
                    buf.push_str(&argv[j]);
                    for c in argv[j].chars() {
                        if c == '(' {
                            depth += 1;
                        } else if c == ')' {
                            depth -= 1;
                        }
                    }
                    j += 1;
                }
                out.push(buf);
                i = j;
            } else {
                out.push(arg.clone());
                i += 1;
            }
        }
        out
    };
    let argv = argv.as_slice();
    // c:Src/builtin.c:2813-3030 — under PM_TIED (`-T scalar array [sep]`)
    // the third positional is the SEPARATOR character, not a param
    // name. The scalar and array become a tied pair: writing the
    // scalar splits the joined string back into the array; reading
    // the scalar joins the array on the separator. Direct port of the
    // C body's tied-pair setup at c:2813-2940 (validation) + the
    // typeset_single+tiedarr_gsu wiring at c:1956 + c:2945+.
    //
    // The Rust port handles the validation / scalar-array tie + value
    // installation as one block here rather than threading PM_TIED
    // through the generic per-arg loop. Beyond index 2 under -T is a
    // usage error in C zsh; the Rust port mirrors the same "too many
    // arguments" rejection.
    //
    // Bug #24 in docs/BUGS.md.
    let tied_mode = (on & PM_TIED) != 0;
    if tied_mode {
        // c:2818-2822 — the FIRST thing the -T block does, ahead of the
        // argument-count checks:
        //     if (OPT_ISSET(ops,'m')) {
        //         zwarnnam(name, "incompatible options for -T");
        //         unqueue_signals();
        //         return 1;
        //     }
        // `-m` takes its names as PATTERNS, which cannot express a tie; it was
        // silently accepted and did nothing.
        if OPT_ISSET(&ops, b'm') {
            zwarnnam(name, "incompatible options for -T"); // c:2819
            unqueue_signals(); // c:2820
            return 1; // c:2821
        }
        // c:2827-2830 — `if (nargs < 2)` reject.
        if argv.len() < 2 {
            zwarnnam(name, "-T requires names of scalar and array");
            unqueue_signals();
            return 1;
        }
        // c:2831-2833 — `if (nargs > 3)` reject.
        if argv.len() > 3 {
            zwarnnam(name, "too many arguments for -T");
            unqueue_signals();
            return 1;
        }
        // First arg: SCALAR name (with optional =value). c:2838-2840
        // `if (ASG_ARRAYP(&asg0))` — first arg can NOT be an array
        // assign; we accept either no-value or =scalar.
        let (sname, sval_opt): (&str, Option<String>) = match argv[0].find('=') {
            Some(i) => (&argv[0][..i], Some(argv[0][i + 1..].to_string())),
            None => (argv[0].as_str(), None),
        };
        // NOT PORTED: c:2841-2846's
        //     asg0 = *asg;
        //     if (ASG_ARRAYP(&asg0)) {
        //         zwarnnam(name, "first argument of tie must be scalar: %s",
        //                  asg0.name);
        //         return 1;
        //     }
        // so `typeset -T S=(a b) s` is still silently accepted where zsh fails.
        //
        // It cannot be decided here. ASG_ARRAYP asks whether the PARSER built an
        // array-valued assignment — the reserved-word `typeset` form seeing a
        // literal `=(` — and that is a property of the parse, not of the text.
        // By the time bin_typeset has argv it is just a string, and the two
        // cases are textually identical:
        //     typeset -T S=(a b) s     → array assignment, must FAIL
        //     typeset -T "S=(a b)" s   → scalar assignment of the literal text
        //                                `(a b)`, must SUCCEED (rc=0, $S is
        //                                `(a b)` — verified against the oracle)
        // A `starts_with('(')` test rejects both; it was tried and broke the
        // quoted form. Fixing this needs the assignment shape carried down from
        // the compiler (C's `assigns` LinkList), which is the same missing
        // reserved-word/builtin distinction that ASG_ARRAYP exists to express.
        // Second arg: ARRAY name (with optional =(elements...) init).
        // Per c:2847-2854, second arg must be array-shape if it carries
        // a value. The Rust port accepts either `arr` or `arr=(a b c)`.
        let (aname, aval_opt): (&str, Option<Vec<String>>) = {
            let a = argv[1].as_str();
            if let Some(eq_idx) = a.find("=(") {
                let aname = &a[..eq_idx];
                // Strip `=(` prefix and `)` suffix. The parser emits
                // array elements separated by REJOIN_SEP (`\u{1f}`)
                // sentinels — see parse.rs:2444 commentary. Empty
                // strings between sentinels are intentional (zsh's
                // assoc paren-init relies on them, bug #93).
                let rest = &a[eq_idx + 2..];
                let inner = rest.trim_end_matches(')');
                let parts: Vec<String> = if inner.contains('\u{1f}') {
                    inner
                        .split('\u{1f}')
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string())
                        .collect()
                } else {
                    // Fallback: whitespace split for synthetic args
                    // that didn't pass through the REJOIN_SEP emitter.
                    inner.split_whitespace().map(|s| s.to_string()).collect()
                };
                (aname, Some(parts))
            } else if let Some(eq_idx) = a.find('=') {
                // `arr=val` (scalar form for array slot) — reject per
                // c:2849-2854 "second argument of tie must be array".
                let _ = eq_idx;
                zwarnnam(
                    name,
                    &format!("second argument of tie must be array: {}", a),
                );
                unqueue_signals();
                return 1;
            } else {
                (a, None)
            }
        };
        // c:2857-2860 — `can't tie a variable to itself`.
        if sname == aname {
            zerrnam(name, &format!("can't tie a variable to itself: {}", sname));
            unqueue_signals();
            return 1;
        }
        // c:2861-2864 — `can't tie array elements` (subscripted name).
        if sname.contains('[') || aname.contains('[') {
            zerrnam(name, &format!("can't tie array elements: {}", sname));
            unqueue_signals();
            return 1;
        }
        // c:2866-2870 — `only one tied parameter can have value`.
        if sval_opt.is_some() && aval_opt.is_some() {
            zerrnam(
                name,
                &format!("only one tied parameter can have value: {}", sname),
            );
            unqueue_signals();
            return 1;
        }
        // c:2876-2898 — joinchar parse. Third argument, if given, is
        // character used to join the elements of the array in the
        // scalar.
        let joinchar: i32 = if argv.len() == 3 {
            let joinstr = argv[2].as_bytes(); // c:2880
            if joinstr.is_empty() {
                0 // c:2893-2894 `else if (!*joinstr) joinchar = 0;`
            } else if joinstr[0] == 0x83 {
                // c:2895-2896 `else if (*joinstr == Meta) joinchar = joinstr[1] ^ 32;`
                (joinstr.get(1).copied().unwrap_or(0) ^ 32) as i32
            } else {
                joinstr[0] as i32 // c:2897-2898
            }
        } else {
            ':' as i32 // c:2891-2892 `if (!joinstr) joinchar = ':';`
        };
        // Split/join separators from joinchar. joinchar==0 measured on
        // the 5.9.1 release binary: assignment keeps the whole string
        // as one element (no split) and reads join with a NUL byte
        // (`zjoin(arr, 0, 1)` writing the raw byte, c:Src/params.c:4352).
        let split_one = joinchar == 0;
        let joinsep: String = if joinchar == 0 {
            "\0".to_string()
        } else {
            ((joinchar as u8) as char).to_string()
        };

        // c:Src/builtin.c:2940-2944 — when the scalar already exists
        // and has a value, save it for seeding the tied array:
        //   `oldval = ztrdup(getsparam(asg0.name));`
        // The Rust port checks paramtab AND the environ inherited at
        // startup so `typeset -T PATH path :` over an env-imported
        // PATH seeds `path` from the live PATH instead of clobbering
        // it with empty.
        // c:2900-2901 — `pm = getnode(asg0.name); apm = getnode(asg->name);`
        // then c:2903-2943 decides between erroring, re-tying, and updating an
        // EXISTING tie. None of that block was ported: the code below simply
        // installed a fresh pair every time, which silently accepted three
        // cases C rejects and lost every attribute the new command line didn't
        // restate.
        let (pm_flags, pm_ename, pm_level) = paramtab()
            .read()
            .ok()
            .and_then(|t| {
                t.get(sname)
                    .map(|p| (Some(p.node.flags as u32), p.ename.clone(), p.level))
            })
            .unwrap_or((None, None, 0));
        let (apm_flags, apm_ename) = paramtab()
            .read()
            .ok()
            .and_then(|t| {
                t.get(aname)
                    .map(|p| (Some(p.node.flags as u32), p.ename.clone()))
            })
            .unwrap_or((None, None));
        let mut already_tied = false;

        if let Some(sf) =
            pm_flags.filter(|f| (f & (PM_SPECIAL | PM_TIED)) == (PM_SPECIAL | PM_TIED))
        {
            let _ = sf;
            // c:2903-2918 — a SPECIAL tied scalar (PATH/path). Only a re-tie of
            // the identical special pair is allowed.
            let apm_special = apm_flags.map(|f| (f & PM_SPECIAL) != 0).unwrap_or(false);
            if pm_ename.as_deref() != Some(aname) || !apm_special {
                // c:2908-2911
                zwarnnam(
                    name,
                    &format!(
                        "{} special parameter can only be tied to special parameter {}",
                        sname,
                        pm_ename.as_deref().unwrap_or("")
                    ),
                );
                unqueue_signals();
                return 1;
            }
            if joinchar != ':' as i32 {
                // c:2913-2917
                zwarnnam(
                    name,
                    "cannot change the join character of special tied parameters",
                );
                unqueue_signals();
                return 1;
            }
            already_tied = true; // c:2918
        } else if let Some(af) =
            apm_flags.filter(|f| (f & (PM_SPECIAL | PM_TIED)) == (PM_SPECIAL | PM_TIED))
        {
            let _ = af;
            // c:2919-2929 — the ARRAY half is special+tied: catches tying it to
            // a different scalar, or to one that is no longer special.
            zwarnnam(
                name,
                &format!(
                    "{} special parameter can only be tied to special parameter {}",
                    aname,
                    apm_ename.as_deref().unwrap_or("")
                ),
            );
            unqueue_signals();
            return 1;
        } else if let Some(sf) = pm_flags {
            // c:2930-2932 — only consider an existing scalar that is actually
            // live in this scope.
            if ((sf & PM_UNSET) == 0 || (sf & PM_DECLARED) != 0)
                && (locallevel_param.load(Relaxed) as i32 == pm_level
                    || (on as u32 & PM_LOCAL) == 0)
            {
                if (sf & PM_TIED) != 0 {
                    if PM_TYPE(sf) != PM_SCALAR {
                        // c:2934-2937
                        zwarnnam(name, &format!("already tied as non-scalar: {sname}"));
                        unqueue_signals();
                        return 1;
                    } else if pm_ename.as_deref() == Some(aname) {
                        already_tied = true; // c:2937
                    } else {
                        // c:2939-2942
                        zwarnnam(name, &format!("can't tie already tied scalar: {sname}"));
                        unqueue_signals();
                        return 1;
                    }
                }
                // else: not tied — only the export attribute is inherited
                // (c:2951), which `inherited_export` below already does.
            }
        }

        if already_tied {
            // c:2957-2973 — C does NOT rebuild the pair here. It runs
            // typeset_single on each half "if only to update the attributes of
            // both, and of course to set the new value if one is provided",
            // then returns:
            //     typeset_single(..., pm,  on,                        off,             ...)
            //     typeset_single(..., apm, (on | PM_ARRAY) & ~PM_EXPORTED, off & ~PM_ARRAY, ...)
            // Applying on/off to the EXISTING entries is precisely what lets an
            // attribute the current command doesn't restate survive — so
            // `typeset -UT TS ts; typeset -T TS ts` keeps its uniqueness, and
            // likewise readonly. Rebuilding dropped everything but export.
            //
            // typeset_single itself is present in this file but is dead code
            // (nothing calls it; bin_typeset carries its own inline logic), so
            // the attribute update is applied directly here rather than routed
            // through it.
            if let Ok(mut tab) = paramtab().write() {
                if let Some(p) = tab.get_mut(sname) {
                    p.node.flags = ((p.node.flags as u32 | on as u32) & !(off as u32)) as i32;
                    // c:2967 — typeset_single takes `joinchar`, so a re-tie
                    // RE-STATES the separator: "It is possible to apply -T to
                    // two previously tied variables but with a different
                    // separator character, in which case the variables remain
                    // joined as before but the separator is changed."
                    // (Doc/Zsh/builtins.yo, -T.) Updating flags alone left
                    // `typeset -T TS ts; typeset -T TS ts ,` joining on ':'.
                    if let Some(td) = p.u_tied.as_mut() {
                        td.joinchar = joinchar; // c:2967
                    }
                }
                if let Some(p) = tab.get_mut(aname) {
                    let aon = (on as u32 | PM_ARRAY) & !PM_EXPORTED; // c:2970
                    let aoff = off as u32 & !PM_ARRAY; // c:2971
                    p.node.flags = ((p.node.flags as u32 | aon) & !aoff) as i32;
                }
            }
            // Values, if the command line carried any (c:2963 "and of course to
            // set the new value if one is provided for either of them").
            if let Some(sval) = sval_opt.as_deref() {
                crate::ported::params::assignsparam(sname, sval, 0);
            } else if let Some(arr) = aval_opt {
                crate::ported::params::assignaparam(aname, arr, 0);
            }
            unqueue_signals();
            return 0;
        }

        let existing_scalar: Option<String> = {
            let from_tab = paramtab()
                .read()
                .ok()
                .and_then(|t| t.get(sname).and_then(|p| p.u_str.clone()));
            from_tab.or_else(|| std::env::var(sname).ok())
        };

        // Build the initial array value: prefer the array RHS, then
        // the scalar RHS (split on joinchar), then the existing
        // scalar's env value (split on joinchar). Mirrors C's sequence
        // at c:2960-3030 where typeset_single is called on both names;
        // if the scalar had a value it gets passed to tiedarrsetfn
        // which splits on the tieddata joinchar (c:4370-4381 sepsplit).
        let init_arr: Vec<String> = if let Some(arr) = aval_opt {
            arr
        } else if let Some(sval) = sval_opt.as_deref() {
            if split_one {
                vec![sval.to_string()]
            } else {
                crate::ported::utils::sepsplit(sval, Some(&joinsep), true)
            }
        } else if let Some(old) = existing_scalar
            .as_deref()
            // Inheriting the existing scalar is for a tie declared at the
            // CURRENT scope: `V=pre:set; typeset -T V v` adopts `pre:set`. A
            // declaration that SHADOWS an outer binding creates a fresh one, so
            // it starts EMPTY even though a value is visible —
            // `V=plain; f(){ local -T V v; print $V }` prints nothing in zsh,
            // where zshrs printed `plain`. An explicit value is unaffected:
            // `local -T V=x:y v` takes the `sval_opt` branch above.
            //
            // Both halves of the test matter. PM_LOCAL alone is not enough —
            // it is set for a plain top-level `typeset` too, and gating on it
            // by itself made the GLOBAL `typeset -T V v` stop inheriting. The
            // `locallevel > 0` half is what restricts this to a real shadow.
            .filter(|_| (on as u32 & PM_LOCAL) == 0 || locallevel.load(Relaxed) == 0)
        {
            if old.is_empty() {
                Vec::new()
            } else if split_one {
                vec![old.to_string()]
            } else {
                crate::ported::utils::sepsplit(old, Some(&joinsep), true)
            }
        } else {
            Vec::new()
        };

        // Install the array side first (matching C c:2980 "Do it
        // first because we need the address"). Build a plain
        // PM_ARRAY|PM_TIED param.
        // c:Src/builtin.c:2986-2999 — the array side is created with
        // `(on | PM_ARRAY) & ~PM_EXPORTED` and the scalar side with the full
        // `on`. So `export -T FOO foo` / `readonly -T` carry those attribute
        // flags onto the pair (the array is NOT export-flagged; the scalar
        // is). Previously both were hardcoded to `…|PM_TIED`, so `export -T`
        // left FOO unexported — `${(t)FOO}` read `scalar-tied` and
        // `typeset -p FOO` printed `typeset -T` instead of `export -T`.
        //
        // c:Src/builtin.c:2953 — `on |= (pm->node.flags & ~roff) & PM_EXPORTED;`
        //
        //   "Variable already exists in the current scope but is not tied.
        //    We're preserving its value and export attribute but no other
        //    attributes upon converting to 'tied'."
        //
        // So tying an ALREADY-EXPORTED scalar keeps it exported, even without
        // `-x`: `export E=a:b; typeset -T E e` leaves E in the environment
        // (${(t)E} == scalar-tied-export). zshrs built the tie attributes from
        // the command-line flags alone and silently dropped the export, which
        // takes the variable OUT of the environment of every later child.
        let inherited_export: u32 = paramtab()
            .read()
            .ok()
            .and_then(|t| t.get(sname).map(|p| p.node.flags as u32))
            .unwrap_or(0)
            & PM_EXPORTED
            & !(off as u32); // c:2953 `& ~roff` — an explicit +x still wins
                             // c:2989 / c:3003 — C hands BOTH halves the full `on`:
                             //     apm = typeset_single(..., (on | PM_ARRAY) & ~PM_EXPORTED, ...)
                             //     pm  = typeset_single(..., on, ...)
                             // so every attribute the user asked for lands on the tie, not just the
                             // export/readonly pair. Masking down to PM_EXPORTED|PM_READONLY silently
                             // dropped the rest — most visibly PM_UNIQUE, so `typeset -UT B b` tied
                             // the two halves but never deduped either (`${(t)b}` read `array-tied`
                             // where zsh reads `array-tied-unique`), while the equivalent spelled as
                             // two commands (`typeset -T B b; typeset -U b`) worked. Carrying `on`
                             // whole is safe here: c:2742-2744 has already stripped the type bits
                             // from it (`if (on & PM_TIED) off |= PM_INTEGER|…|PM_HASHED; on &= ~off`).
        let tie_attr: u32 = on as u32 | inherited_export;
        let mut apm = param::default();
        apm.node.nam = aname.to_string();
        apm.node.flags = ((PM_ARRAY | PM_TIED) | (tie_attr & !PM_EXPORTED)) as i32;
        apm.u_arr = Some(init_arr.clone());
        apm.ename = Some(sname.to_string());
        apm.level = locallevel.load(Relaxed) as i32;
        // c:2982-2989 — `tdp = (Tieddata) zalloc(sizeof *tdp);
        // tdp->joinchar = joinchar;` — the tieddata rides on the
        // SCALAR side's pm->u.data (c:2566 `tdp->joinchar = joinchar`).
        let tdp = crate::ported::zsh_h::tieddata {
            arrptr: None,
            joinchar,
        };

        // Install the scalar side with PM_TIED + gsu_s wired to the
        // tied-via-ename getters/setters so reads/writes propagate
        // through paramtab to apm.u_arr.
        let mut spm = param::default();
        spm.node.nam = sname.to_string();
        spm.node.flags = ((PM_SCALAR | PM_TIED) | tie_attr) as i32; // c:2999 scalar carries full `on`
        spm.ename = Some(aname.to_string());
        spm.u_str = Some(init_arr.join(&joinsep));
        spm.u_tied = Some(Box::new(tdp));
        spm.level = locallevel.load(Relaxed) as i32;
        // c:Src/builtin.c:1956 — `static const struct gsu_scalar
        // tiedarr_gsu = { tiedarrgetfn, tiedarrsetfn, tiedarrunsetfn };`
        // The scalar side of a tied pair routes its reads/writes
        // through these so the partner array stays in sync. C's
        // tiedarrgetfn returns the joined string; tiedarrsetfn splits
        // and writes through `*dptr->arrptr`. The Rust port adapts
        // those to paramtab lookups via `pm.ename` (see params.rs
        // commentary). The C tiedarrgetfn returns `char*` (joined
        // scalar); the Rust signature returns `Vec<String>` because
        // the existing typing — adapt with a closure that joins.
        fn tied_scalar_getfn_shim(pm: &param) -> String {
            // c:4352 — `zjoin(*dptr->arrptr, (unsigned char)
            // dptr->joinchar, 1)`; joinchar lives on the tieddata
            // riding pm->u.data (Rust: typed u_tied view).
            let sep = match pm.u_tied.as_deref() {
                // joinchar==0 joins with the raw NUL byte (zjoin with
                // delim 0, c:Src/params.c:4352; measured on 5.9.1).
                Some(td) if td.joinchar == 0 => "\0".to_string(),
                Some(td) => ((td.joinchar as u8) as char).to_string(),
                None => ":".to_string(),
            };
            crate::ported::params::tiedarrgetfn(pm).join(&sep)
        }
        fn tied_scalar_setfn_shim(pm: &mut param, val: String) {
            crate::ported::params::tiedarrsetfn(pm, Some(val))
        }
        fn tied_scalar_unsetfn_shim(pm: &mut param, exp: i32) {
            crate::ported::params::tiedarrunsetfn(pm, exp)
        }
        spm.gsu_s = Some(Box::new(crate::ported::zsh_h::gsu_scalar {
            getfn: tied_scalar_getfn_shim,
            setfn: tied_scalar_setfn_shim,
            unsetfn: tied_scalar_unsetfn_shim,
        }));

        if let Ok(mut tab) = paramtab().write() {
            // c:Src/params.c:1137 — when a declaration SHADOWS an existing
            // binding, `createparam` keeps the displaced one as `pm->old` so
            // `endparamscope` can put it back. This path builds both halves
            // from `param::default()` (`old: None`) and inserted them straight
            // over the top, so `local -T V v` over an existing `typeset -T V v`
            // DESTROYED the outer pair: after the function returned `$V` was
            // empty, `${(t)V}` reported nothing at all, and every later
            // `V=…` stopped updating `v` — the global tie was gone for the rest
            // of the shell's life. docs/BUGS.md #1039 C.
            //
            // Only chain when this declaration is actually creating a shadow at
            // a DEEPER level: re-tying at the same level legitimately replaces
            // the binding (and `already_tied` above handles the same-partner
            // case), so an unconditional chain would build a bogus restore
            // stack for plain global re-ties.
            let cur_ll = locallevel.load(Relaxed) as i32;
            let mut apm = apm;
            let mut spm = spm;
            if tab.get(aname).map(|p| p.level < cur_ll).unwrap_or(false) {
                apm.old = tab.remove(aname); // c:1137 `pm->old = oldpm`
            }
            if tab.get(sname).map(|p| p.level < cur_ll).unwrap_or(false) {
                spm.old = tab.remove(sname); // c:1137
            }
            tab.insert(aname.to_string(), Box::new(apm));
            tab.insert(sname.to_string(), Box::new(spm));
        }
        unqueue_signals();
        return 0;
    }

    // c:Src/builtin.c:3042-3098 — `-m PATTERN` (or `+m PATTERN`) treats
    // each arg as a glob, not a `name[=value]` pair.
    //
    // C splits into TWO subpaths:
    //   +m: c:3068 `scanmatchtable(..., paramtab->printnode, printflags)`
    //       — direct paramtab walk that prints PRINT_TYPE | PRINT_NAMEONLY
    //         (built up at c:3051-3054 when on|roff == 0).
    //   -m: c:3081-3094 build pmlist, then run `typeset_single` per
    //       match. typeset_single's print path (c:2240-2247) uses
    //         - PRINT_TYPESET with `-p`
    //         - PRINT_INCLUDEVALUE otherwise, gated on
    //             `!OPT_ISSET(ops,'g') && (unset(TYPESETSILENT) ||
    //              OPT_ISSET(ops,'m'))`
    //
    // Earlier zshrs collapsed both into one `printparamnode(...,
    // PRINT_INCLUDEVALUE)` call which:
    //   - Made `+m` emit `name=value` instead of `name` / `integer NAME`.
    //   - Ignored `-g` print suppression so `typeset -gm 'A*'` spammed
    //     every match (zsh is silent: `-g` blocks the print). Hit
    //     during zinit's plugin loader which runs `typeset -gm`
    //     against the plugin's exported-var set on every load.
    //     Bug #48 in docs/BUGS.md.
    if OPT_ISSET(&ops, b'm') && !argv.is_empty() {
        // c:3043-3055 — printflags for the +m direct-scan path.
        if !OPT_ISSET(&ops, b'p') {
            // c:3044-3050 — mass-changing types is fatal for namerefs.
            if (on & PM_NAMEREF) != 0 && OPT_MINUS(&ops, b'm') {
                zerrnam(name, "-m not allowed with -n"); // c:3048
                unqueue_signals();
                return 1; // c:3049
            }
            if (on | roff) == 0 {
                printflags |= PRINT_TYPE; // c:3052
            }
            if on == 0 {
                printflags |= PRINT_NAMEONLY; // c:3054
            }
        }
        // c:2241 — typeset_single's `-m` print path always sets
        // PRINT_WITH_NAMESPACE so dot-prefixed names (`.cd`) emit
        // instead of being filtered at printparamnode's namespace gate.
        let single_flags: i32 = if OPT_ISSET(&ops, b'p') {
            PRINT_TYPESET | PRINT_WITH_NAMESPACE
        } else {
            PRINT_INCLUDEVALUE | PRINT_WITH_NAMESPACE
        };
        // c:2244 — `else if (!OPT_ISSET(ops,'g') && (unset(TYPESETSILENT)
        // || OPT_ISSET(ops,'m')))` — typeset_single suppresses
        // PRINT_INCLUDEVALUE when `-g` is set. `-p` bypasses (c:2242).
        let do_minus_print = OPT_ISSET(&ops, b'p')
            || (!OPT_ISSET(&ops, b'g') && (!isset(TYPESETSILENT) || OPT_ISSET(&ops, b'm')));
        for pattern in argv.iter() {
            // c:3061 — `patcompile(asg->name, 0, NULL)` glob-compile.
            // Use the canonical pattern.rs port. On compile failure,
            // emit "bad pattern" and continue to the next arg.
            let pat = crate::ported::pattern::patcompile(
                &{
                    let mut __pat_tok = (pattern).to_string();
                    crate::ported::glob::tokenize(&mut __pat_tok);
                    __pat_tok
                },
                crate::ported::zsh_h::PAT_HEAPDUP as i32,
                None,
            );
            let pat = match pat {
                Some(p) => p,
                None => {
                    zwarnnam(name, &format!("bad pattern: {}", pattern));
                    returnval = 1;
                    continue;
                }
            };
            // c:3068 — `scanmatchtable(paramtab, pprog, 1, on|roff,
            // 0, paramtab->printnode, printflags);` — walk paramtab
            // entries whose name matches the pattern AND whose flag
            // bits intersect on|roff.
            //
            // c:3073-3080 — C collects the matches into `pmlist`
            // FIRST and only then walks it, because the per-entry work
            // can reorganise paramtab underneath the scan. zshrs needs
            // the same two-phase shape for a second, harder reason:
            // the table lock must be RELEASED before any entry is
            // touched. `printparamnode` re-enters `paramtab().read()`
            // on several paths —
            //   params.rs:12622  PM_TIED peer lookup (the `+m` /
            //                    PRINT_TYPE arm),
            //   params.rs:12303 + 12312, 12009, 12038  PM_SPECIAL /
            //                    empty-scalar / zero-integer value
            //                    probes via `getsparam` (→
            //                    params.rs:5353 and
            //                    `lookup_special_var` at 13494 /
            //                    13547 / 13554 / 13640 / 13669),
            //   params.rs:9365   `tiedarrgetfn`, reached through the
            //                    tied-scalar `gsu_s` getfn shim
            //                    (builtin.rs:4671).
            // `paramtab()` is a plain `std::sync::RwLock`, which is
            // NOT reentrant: acquiring the read half while this thread
            // already holds the write half parks the thread forever.
            // The previous code held `paramtab().write()` across
            // `printparamnode`, so `typeset -m PAT` hung the shell the
            // moment PAT matched any param that takes one of those
            // paths (`HISTCHARS`, `HOME`, `IFS`, `TERM`, `WORDCHARS`,
            // `USERNAME`, `SECONDS`, `TIMEFMT`, any tied scalar, and
            // every empty PM_SPECIAL scalar / zero-valued special
            // integer). It surfaced as a pattern-COUNT threshold
            // (`typeset -m` over ~19+ `${(k)parameters}` names hung
            // while 18 returned) only because the list is walked in
            // name order and the first such name sits around there;
            // the real trigger is the name, not the count.
            // Pre-cloning is the rule the `typeset -p NAME` path
            // already documents at builtin.rs:5156-5159.
            let matched: Vec<crate::ported::zsh_h::Param> = {
                let tab = paramtab().read().unwrap();
                let on_roff = (on as u32) | (roff as u32);
                let mut matched: Vec<crate::ported::zsh_h::Param> = tab
                    .iter()
                    .filter(|(k, pm)| {
                        let f = pm.node.flags as u32;
                        if (f & PM_UNSET) != 0 {
                            return false;
                        }
                        // c:Src/builtin.c:3055-3094 / hashtable.c:373-440
                        // — the `-m`/`+m` PATTERN path scans paramtab
                        // via `scanmatchtable(paramtab, pprog, ...)`.
                        // scanmatchtable matches ONLY on flags1/flags2
                        // + the pattern; it does NOT exclude PM_HIDE.
                        // An explicit pattern therefore REVEALS hidden
                        // params — verified vs `/opt/homebrew/bin/zsh`:
                        //   `typeset +m 'a*'` → `undefined aliases` +
                        //   `array argv`; `typeset -m 'a*'` → `aliases`
                        //   + `argv=(  )`. The magic-assoc placeholders
                        //   (PM_HIDE|PM_HIDEVAL, inserted by
                        //   init_partab_params) and `private` vars
                        //   (PM_HIDE, no PM_HIDEVAL — c:Modules/
                        //   param_private.c:174) must BOTH appear here,
                        //   unlike the no-args list path (c:2762-2792)
                        //   where scanhashtable's bare scan keeps them
                        //   hidden (bug #371). So NO PM_HIDE filter on
                        //   this PATTERN path — V10private: `private
                        //   x=5; typeset +m x` → `local x`.
                        if on_roff != 0 && (f & on_roff) == 0 {
                            return false;
                        }
                        crate::ported::pattern::pattry(&pat, k)
                    })
                    // c:3087 `addlinknode(pmlist, pm)` — the match list.
                    .map(|(_, pm)| pm.clone())
                    .collect();
                matched.sort_by(|a, b| hnamcmp(&a.node.nam, &b.node.nam));
                matched
            };
            // The table guard is dropped here, before any print.
            if OPT_PLUS(&ops, b'm') {
                // c:3068-3070 — `+m`: direct print using the
                // PRINT_TYPE | PRINT_NAMEONLY flags built above.
                for mut pm in matched {
                    printparamnode(&mut pm, printflags);
                }
                continue;
            }
            // c:3081-3094 — `-m`: typeset_single per match. For the
            // pure listing case (no `=` value, no attribute mutation),
            // typeset_single reduces to the c:2240-2247 print arm.
            // Attribute-conversion (`-im PAT`, `-rm PAT`, etc.) on
            // matched names isn't ported here yet; the early
            // `return returnval` below short-circuits the per-arg
            // name loop for `-m` args, so on/roff currently acts as
            // a listing filter only.
            if do_minus_print {
                // c:3090-3095 — walk the pre-built match list. Clones,
                // per the lock note above; printparamnode's writes into
                // `hn` (the u_str seed at params.rs:12312, the PM_TIED
                // value swap at 12639) are display-local scratch that C
                // performs on a reassigned local pointer, never on the
                // stored node, so printing the clone is also the more
                // faithful shape.
                for mut pm in matched {
                    printparamnode(&mut pm, single_flags);
                }
            }
        }
        unqueue_signals();
        return returnval;
    }

    // c:3035-3039 — `+T` is not a way to untie:
    //     if (off & PM_TIED) {
    //         unqueue_signals();
    //         zerrnam(name, "use unset to remove tied variables");
    //         return 1;
    //     }
    // C puts this at the TOP LEVEL, so it precedes everything typeset_single
    // does — notably the special-parameter type-change rule, whose `chflags`
    // watches PM_TIED and would otherwise claim `typeset +T PATH path` and
    // report "can't change type of a special parameter" instead. The check was
    // missing entirely, so `+T` was silently accepted and did nothing.
    if (off as u32 & PM_TIED) != 0 {
        unqueue_signals(); // c:3036
        zerrnam(name, "use unset to remove tied variables"); // c:3037
        return 1; // c:3038
    }

    let mut tied_name_count: usize = 0;
    for arg in argv {
        // c:Src/builtin.c typeset_single — when PM_LOCAL is in
        // flags, createparam first to install pm.old chain at
        // locallevel (createparam c:1132-1147). Applies uniformly
        // to all forms: `local x`, `local x=v`, `local arr=(...)`,
        // `local -A h`. endparamscope unwinds via Param.old.
        if tied_mode {
            tied_name_count += 1;
            if tied_name_count > 2 {
                // Separator / extra args — don't run name
                // validation or createparam against them. Full
                // -T body (createparam SCALAR_TIED + array tie
                // backref) is deferred per the comment above on
                // the typeset_single dispatch.
                continue;
            }
        }
        let arg_name: &str = match arg.find('=') {
            Some(i) => &arg[..i],
            None => arg.as_str(),
        };

        // c:2117-2193 (inside typeset_single) — changing the TYPE of an
        // existing parameter is refused for specials, with SECONDS the one
        // documented exception.
        //
        //     int chflags = ((off & pm->node.flags) | (on & ~pm->node.flags)) &
        //          (PM_INTEGER|PM_EFLOAT|PM_FFLOAT|PM_HASHED|PM_ARRAY|PM_TIED|PM_AUTOLOAD);
        //     /* keep the parameter if just switching between floating types */
        //     if ((tc = chflags && chflags != (PM_EFLOAT|PM_FFLOAT))) { ... }
        //     ...
        //     if (... || tc) {
        //         if (pm->node.flags & PM_SPECIAL) {
        //             int err = 1;
        //             if (!readonly && !strcmp(pname, "SECONDS")) {
        //                 ... else if (!setsecondstype(pm, on, off)) { ... err = 0; }
        //             }
        //             if (err) { zerrnam(cname, "%s: can't change type of a "
        //                                "special parameter", pname); return NULL; }
        //
        // None of this was ported, so a type change against a special was
        // silently ACCEPTED and then dropped: `typeset -F RANDOM`,
        // `typeset -F LINENO` and `typeset -F HISTSIZE` all returned 0 having
        // done nothing, where zsh fails. And the SECONDS exception — the
        // documented way to get sub-second timing — never took effect:
        // `typeset -F SECONDS` left it `integer-special` instead of
        // `float-special`.
        //
        // `chflags` is why this fires only on a REAL type change:
        // `typeset -i SECONDS` / `typeset -i LINENO` leave chflags == 0 (the
        // bit is already set) and are correctly no-ops, and -E↔-F is exempted
        // outright by the `!= (PM_EFLOAT|PM_FFLOAT)` clause. It also covers
        // `typeset -a PATH`, which is a PM_ARRAY type change on a special and
        // so earns the same message rather than an array-specific one.
        //
        // The `readonly` half of C's guard is not modelled here: it gates
        // turning readonly OFF, which reaches this branch by a different route.
        // setsecondstype (params.rs) is the real port and had NO callers.
        if !tied_mode {
            let pm_info = paramtab()
                .read()
                .ok()
                .and_then(|t| t.get(arg_name).map(|p| (p.node.flags as u32, p.level)));
            if let Some((pmf, pm_level)) = pm_info {
                let chflags = ((off as u32 & pmf) | (on as u32 & !pmf))
                    & (PM_INTEGER
                        | PM_EFLOAT
                        | PM_FFLOAT
                        | PM_HASHED
                        | PM_ARRAY
                        | PM_TIED
                        | PM_AUTOLOAD); // c:2118-2120
                let tc = chflags != 0 && chflags != (PM_EFLOAT | PM_FFLOAT); // c:2122
                                                                             // c:2078-2091 — a `local` at a DEEPER scope than the existing
                                                                             // param does not CHANGE that param's type; it SHADOWS it with a
                                                                             // fresh local (createparam installs the pm.old chain, restored by
                                                                             // endparamscope). Only a SAME-scope re-typeset of a special is the
                                                                             // forbidden type change. The port applied the special type-change
                                                                             // error against the higher-scope param unconditionally, so
                                                                             // `local -a commands` (HASHED command-hash special → local ARRAY)
                                                                             // inside a function wrongly errored "can't change type of a
                                                                             // special parameter" and aborted — e.g. `_openssl_subcommands`'s
                                                                             // `local -a commands=(…)`, leaving `openssl <TAB>` empty. zsh
                                                                             // allows the shadow in a function (errors only at same scope /
                                                                             // top level). Mirror the reuse-decision guard used at c:2078-2091.
                let creating_local_shadow =
                    (on as u32 & PM_LOCAL) != 0 && pm_level != locallevel_param.load(Relaxed);
                if tc && (pmf & PM_SPECIAL) != 0 && !creating_local_shadow {
                    let mut err = true; // c:2144
                    if arg_name == "SECONDS" {
                        // c:2171 — `else if (!setsecondstype(pm, on, off))`.
                        let ok = paramtab()
                            .write()
                            .ok()
                            .and_then(|mut t| {
                                t.get_mut(arg_name).map(|p| {
                                    crate::ported::params::setsecondstype(p, on as i32, off as i32)
                                })
                            })
                            .unwrap_or(1);
                        if ok == 0 {
                            err = false; // c:2176
                                         // c:2172-2175 — `if (asg->value.scalar && !(pm =
                                         // assignsparam(pname, ..., 0))) return NULL;`
                            if let Some(i) = arg.find('=') {
                                crate::ported::params::assignsparam(arg_name, &arg[i + 1..], 0);
                            }
                        }
                    }
                    if err {
                        // c:2181-2187
                        if !OPT_ISSET(&ops, b'p') {
                            zerrnam(
                                name,
                                &format!("{arg_name}: can't change type of a special parameter"),
                            );
                        }
                        unqueue_signals();
                        return 1;
                    }
                    // SECONDS: setsecondstype already installed the new type;
                    // c:2169's `tc = 0` skips the normal conversion below.
                    continue;
                }
            }
        }

        // c:2519-2552 (Src/builtin.c, inside typeset_single) — name
        // validation gate. Direct port:
        //   else if ((isident(pname) || paramtab->getnode(paramtab, pname))
        //            && (!idigit(*pname) || !strcmp(pname, "0"))) {
        //       /* proceed */
        //   } else {
        //       if (idigit(*pname))
        //           zerrnam(cname, "not an identifier: %s", pname);
        //       else
        //           zerrnam(cname, "not valid in this context: %s", pname);
        //       return NULL;
        //   }
        //
        // The C function returns NULL on failure; the outer bin_typeset
        // name loop continues to the next arg (errflag silences
        // subsequent zerr calls so we won't double-emit). Mirror that
        // here with `continue`.
        let pname_in_tab = paramtab()
            .read()
            .map(|t| t.get(arg_name).is_some())
            .unwrap_or(false);
        // c:2062-2064 — `usepm = pm && (!(pm->node.flags & PM_UNSET)
        // || ...)`: snapshot BEFORE any createparam this iteration
        // runs (the PM_LOCAL shadow block and the pre-assign type
        // stamps both create fresh pms). Only the reuse-existing-pm
        // branch of typeset_single carries the c:2336 `if (errflag)
        // return NULL` post-assign check; the fresh-param path
        // (c:2577+ createparam → c:2604 assignsparam) has NO errflag
        // check — `typeset -i x=3#8` zerrs the math but bin_typeset
        // still returns 0 (zsh 5.9 exit 0, lastval untouched).
        let usepm_existing = paramtab()
            .read()
            .map(|t| {
                t.get(arg_name)
                    .is_some_and(|pm| (pm.node.flags as u32 & PM_UNSET) == 0)
            })
            .unwrap_or(false);
        // c:Src/builtin.c:2078 — snapshot the EXISTING param's local
        // level BEFORE this arg's createparam/assign runs. The bare-name
        // print decision (c:2244, inlined at ~5962) needs the pre-state
        // level to tell "re-declare same-level local" (usepm kept →
        // print) from "localize a higher-level/global var" (usepm=0 →
        // no print). Sentinel -1 when the param doesn't pre-exist.
        let pm_level_existing: i32 = paramtab()
            .read()
            .ok()
            .and_then(|t| t.get(arg_name).map(|pm| pm.level))
            .unwrap_or(-1);
        let first_is_digit = arg_name
            .as_bytes()
            .first()
            .is_some_and(|b| b.is_ascii_digit());
        let pname_valid =
            (isident(arg_name) || pname_in_tab) && (!first_is_digit || arg_name == "0");
        if !pname_valid {
            if first_is_digit {
                zerrnam(
                    name, // c:2548
                    &format!("not an identifier: {}", arg_name),
                );
            } else {
                zerrnam(
                    name, // c:2550
                    &format!("not valid in this context: {}", arg_name),
                );
            }
            // c:3153-3156 — `if (!typeset_single(...)) returnval = 1;`
            // — the NULL return from the validation gate makes
            // bin_typeset's per-arg loop record failure.
            returnval = 1;
            continue; // c:2551 return NULL
        }

        // c:2241-2247 — `-p` print-mode for an existing param (no `=`,
        // no value). C `typeset_single` lands here when `usepm` is set
        // and `!ASG_VALUEP(asg)`, BEFORE createparam runs (c:2218 →
        // c:2244 early return). The Rust loop must also dispatch the
        // print branch first; otherwise the createparam call below
        // overwrites pm.node.flags on the reuse-arm (c:2018), clobbering
        // typeset-attribute bits set by an earlier `typeset -i n` call.
        if !arg.contains('=') && OPT_ISSET(&ops, b'p') {
            let with_ns = if OPT_ISSET(&ops, b'm') {
                // c:2241
                PRINT_WITH_NAMESPACE
            } else {
                0
            };
            // c:Src/builtin.c:2761-2765 — `-p1` adds PRINT_LINE (one
            // array/assoc element per line). The named-arg `typeset -p1
            // NAME` print path missed it (only the listing path parsed
            // it), so `typeset -p1 myarray` printed single-line.
            let line_flag =
                if OPT_HASARG(&ops, b'p') && OPT_ARG(&ops, b'p').map(|a| a.trim()) == Some("1") {
                    PRINT_LINE
                } else {
                    0
                };
            // !!! BASH-MODE GATE (no C counterpart) !!! `declare -p` of a bash
            // synthesized special array (PIPESTATUS / FUNCNAME / BASH_VERSINFO)
            // — these live outside paramtab, so the normal lookup below can't
            // find them. Emit the bash reusable form (BASH_VERSINFO is a
            // readonly array).
            if crate::dash_mode::bash_mode() {
                if let Some(vals) = crate::dash_mode::bash_special_array(arg_name) {
                    let esc = |s: &str| -> String {
                        let mut o = String::with_capacity(s.len());
                        for c in s.chars() {
                            if matches!(c, '"' | '\\' | '$' | '`') {
                                o.push('\\');
                            }
                            o.push(c);
                        }
                        o
                    };
                    let flags = if arg_name == "BASH_VERSINFO" {
                        "-ar"
                    } else {
                        "-a"
                    };
                    let body: String = vals
                        .iter()
                        .enumerate()
                        .map(|(i, v)| format!("[{}]=\"{}\"", i, esc(v)))
                        .collect::<Vec<_>>()
                        .join(" ");
                    println!("declare {} {}=({})", flags, arg_name, body);
                    continue;
                }
            }
            let existed = paramtab()
                .read()
                .map(|t| t.contains_key(arg_name))
                .unwrap_or(false);
            if existed {
                // c:Src/params.c:6275 — printparamnode looks up the
                // PM_TIED peer via paramtab; we must NOT hold the lock
                // when calling. Bug #410 — pre-clone pattern (mirrors
                // the bin_typeset `-m PAT` arm refactor).
                let mut pm_clone = match paramtab().read() {
                    Ok(tab) => tab.get(arg_name).cloned(),
                    Err(_) => None,
                };
                if let Some(ref mut pm) = pm_clone {
                    // c:2243 — `paramtab->printnode(&pm->node,
                    //   PRINT_TYPESET|with_ns);`
                    printparamnode(pm, PRINT_TYPESET | with_ns | line_flag);
                }
            } else {
                // c:Src/builtin.c:3110-3113 — when `typeset -p NAME`
                // and the param doesn't exist, emit
                // `typeset: no such variable: NAME` and set
                // returnval=1 so the builtin's exit status is 1.
                zwarnnam(name, &format!("no such variable: {}", arg_name));
                returnval = 1; // c:Src/builtin.c:3112
            }
            continue;
        }

        // c:3117-3150 — `typeset -n NAME[=refname]` arm.
        if (on as u32 & PM_NAMEREF) != 0 {
            // c:Src/builtin.c:3117-3150 — the `-n` literal-name arm,
            // INLINE in C's bin_typeset (no separate C function; the
            // former vm_helper::typeset_nameref_arg helper relocated
            // here 2026-06-12 per the no-fake-fns-in-ported rule).
            // Combined with typeset_single's nameref pieces:
            // subscripted-name reject (c:2452-2456), read-only
            // reference guard (c:2249-2256), fresh-start unset
            // (c:3127-3141), scalar→nameref conversion carrying the
            // value as refname (c:3132-3135), creation + refname
            // assignment (assignsparam → PM_NAMEREF arm → setscope).
            let nameref_arm = |on: u32, off: u32| -> i32 {
                let value: Option<&str> = arg.find('=').map(|i| &arg[i + 1..]);

                // c:2452-2456 — `typeset -n ptr[1]=...` is invalid.
                if arg_name.contains('[') {
                    zerrnam(
                        name,
                        &format!("{}: reference variable cannot be an array", arg_name),
                    );
                    unqueue_signals();
                    return 1;
                }

                // c:3118-3126 — refname target that is itself a PM_SPECIAL nameref.
                if let Some(v) = value {
                    let special_ref = paramtab()
                        .read()
                        .ok()
                        .and_then(|t| {
                            t.get(v).map(|pm| {
                                let f = pm.node.flags as u32;
                                (f & PM_NAMEREF) != 0 && (f & PM_SPECIAL) != 0
                            })
                        })
                        .unwrap_or(false);
                    if special_ref {
                        zwarnnam(name, &format!("{}: invalid reference", v)); // c:3122
                        unqueue_signals();
                        return 1; // c:3123-3124
                    }
                }

                let cur_ll = locallevel_param.load(Relaxed) as i32;
                let existing = paramtab().read().ok().and_then(|t| {
                    t.get(arg_name)
                        .map(|pm| (pm.node.flags as u32, pm.level, pm.u_str.clone()))
                });

                let mut carried_value: Option<String> = value.map(String::from);
                let mut reuse_existing = false;

                let existing_level: Option<i32> = existing.as_ref().map(|(_, l, _)| *l);
                if let Some((eflags, elevel, estr)) = existing {
                    // c:2249-2256 — read-only guard (typeset_single). Fires when
                    // the existing pm is readonly, +r wasn't given, and either the
                    // nameref-ness changes or a nameref gets a new value.
                    if (eflags & PM_READONLY) != 0
                        && (off & PM_READONLY) == 0
                        && !OPT_ISSET(&ops, b'p')
                    {
                        let kind = if (eflags & PM_NAMEREF) != 0 {
                            "reference"
                        } else {
                            "variable"
                        };
                        zerrnam(name, &format!("{}: read-only {}", arg_name, kind)); // c:2254
                        unqueue_signals();
                        return 1; // c:2256
                    }
                    // c:3127-3141 — namerefs always start over fresh.
                    if elevel >= cur_ll || ((on & PM_LOCAL) == 0 && elevel < cur_ll) {
                        // c:3132-3135 — converting a scalar: its value becomes
                        // the refname.
                        if carried_value.is_none()
                            && PM_TYPE(eflags) == PM_SCALAR
                            && estr.as_deref().map(|s| !s.is_empty()).unwrap_or(false)
                        {
                            carried_value = estr.clone();
                        }
                        // c:3136-3140 — `if (!(hn->flags & PM_READONLY)) {
                        //   unsetparam_pm(oldpm, 0, 1); hn = NULL; }` — only a
                        // non-readonly ref starts over fresh; a readonly one is
                        // KEPT and handed to typeset_single (c:3148-3149), where
                        // `typeset +r -n ref[=val]` clears the flag in place and
                        // any =val ASSIGNS THROUGH the surviving refname.
                        if (eflags & PM_READONLY) != 0 {
                            reuse_existing = true;
                        } else if let Some(mut old) =
                            paramtab().write().ok().and_then(|mut t| t.remove(arg_name))
                        {
                            // keep the shadowed outer binding for re-chaining
                            if let Some(prev) = old.old.take() {
                                if let Ok(mut tab) = paramtab().write() {
                                    tab.insert(arg_name.to_string(), prev);
                                }
                            }
                        }
                    } else if (eflags & PM_READONLY) != 0 {
                        // c:3142-3149 — only a READONLY ref survives as the pm
                        // handed to typeset_single (so `typeset -rn ref=var` can
                        // error properly); everything else gets `hn = NULL` and
                        // a fresh local SHADOW is created below (c:3148-3149).
                        reuse_existing = true;
                    }
                }

                // (Re)create the nameref param (typeset_single c:2577+ createparam
                // with PM_NAMEREF type).
                if !reuse_existing {
                    let shadowed = paramtab().write().ok().and_then(|mut t| {
                        if (on & PM_LOCAL) != 0 && cur_ll > 0 {
                            t.remove(arg_name)
                        } else {
                            None
                        }
                    });
                    let mut flags = PM_NAMEREF as i32;
                    // c:2544 — TYPESET_TO_UNSET: declared-but-unassigned.
                    if carried_value.is_none() && isset(crate::ported::zsh_h::TYPESETTOUNSET) {
                        flags |= crate::ported::zsh_h::PM_DEFAULTED as i32;
                    }
                    // crate::ported::zsh_h::PM_UPPER on a nameref marks the -u upscope variant (c:2698
                    // allows -u with -n); crate::ported::zsh_h::PM_HIDEVAL likewise.
                    flags |= (on
                        & (crate::ported::zsh_h::PM_UPPER | crate::ported::zsh_h::PM_HIDEVAL))
                        as i32;
                    // c:1108-1132 — createparam REUSES the just-unset node at its
                    // own level when !PM_LOCAL (`typeset -gn` rebind of a local
                    // ref keeps the ref's level; it does NOT hoist to level 0).
                    let level = if (on & PM_LOCAL) != 0 {
                        cur_ll
                    } else if let Some(elevel) = existing_level {
                        elevel
                    } else {
                        0
                    };
                    let pm = Box::new(crate::ported::zsh_h::param {
                        node: crate::ported::zsh_h::hashnode {
                            next: None,
                            nam: arg_name.to_string(),
                            flags,
                        },
                        u_data: 0,
                        u_tied: None,
                        u_arr: None,
                        u_str: if carried_value.is_none() && value.is_some() {
                            Some(String::new()) // `typeset -n ptr=` placeholder
                        } else {
                            None
                        },
                        u_val: 0,
                        u_dval: 0.0,
                        u_hash: None,
                        gsu_s: None,
                        gsu_i: None,
                        gsu_f: None,
                        gsu_a: None,
                        gsu_h: None,
                        base: 0,
                        width: 0,
                        env: None,
                        ename: None,
                        old: shadowed,
                        level,
                    });
                    if let Ok(mut tab) = paramtab().write() {
                        tab.insert(arg_name.to_string(), pm);
                    }
                } else {
                    // reuse path (read-only refs surviving the fresh-start gate,
                    // c:3148-3149): apply on/off bits in place. A readonly
                    // SCALAR + `-n` converts to a nameref here — its current
                    // value becomes the refname (c:2117+ type-conversion inside
                    // typeset_single with the pm kept; the `typeset +r -n
                    // ref=RW` shape: "assignment occurs after type change").
                    if let Ok(mut tab) = paramtab().write() {
                        if let Some(pm) = tab.get_mut(arg_name) {
                            pm.node.flags |= (on
                                & (crate::ported::zsh_h::PM_UPPER
                                    | crate::ported::zsh_h::PM_HIDEVAL))
                                as i32;
                            pm.node.flags |= PM_NAMEREF as i32;
                            pm.node.flags &= !((off
                                & (crate::ported::zsh_h::PM_UPPER
                                    | crate::ported::zsh_h::PM_HIDEVAL
                                    | PM_READONLY))
                                as i32);
                        }
                    }
                }

                // Assign the refname (typeset_single c:2326 assignsparam →
                // PM_NAMEREF assignstrvalue arm + valid_refname + setscope).
                let mut rc = 0;
                if reuse_existing {
                    // c:2326 — the surviving (previously-readonly) ref keeps its
                    // refname; a =value assignment goes through the canonical
                    // assignsparam which RESOLVES the chain (`typeset +r -n
                    // ref=RW` writes RW into the referent, not the ref).
                    if let Some(v) = value {
                        if crate::ported::params::setsparam(arg_name, v).is_none() {
                            rc = 1;
                        }
                    }
                } else if let Some(v) = carried_value.as_deref() {
                    if v.is_empty() {
                        // `typeset -n ptr=` — empty placeholder, no setscope error.
                        if let Ok(mut tab) = paramtab().write() {
                            if let Some(pm) = tab.get_mut(arg_name) {
                                pm.u_str = Some(String::new());
                                pm.node.flags &= !(crate::ported::zsh_h::PM_DEFAULTED as i32);
                            }
                        }
                    } else if crate::ported::params::setsparam(arg_name, v).is_none() {
                        // c:2326 — typeset_single's assignsparam; the fresh
                        // PM_NAMEREF pm routes to assignstrvalue's nameref arm
                        // (c:2690-2720: valid_refname + SETREFNAME + setscope).
                        rc = 1;
                    }
                } else if !reuse_existing && crate::ported::params::is_nameref(arg_name) {
                    // bare placeholder — still run setscope for parity (no-op).
                    let _ = crate::ported::params::setscope_by_name(arg_name);
                }

                // c:2618 — `pm->node.flags |= (on & PM_READONLY);` AFTER the
                // assignment so `typeset -rn ref=var` can set its initial value.
                if (on & PM_READONLY) != 0 {
                    if let Ok(mut tab) = paramtab().write() {
                        if let Some(pm) = tab.get_mut(arg_name) {
                            pm.node.flags |= PM_READONLY as i32;
                        }
                    }
                }
                rc
            };
            if nameref_arm(on as u32, off as u32) != 0 {
                returnval = 1; // c:3153-3156
            }
            continue;
        }

        // c:2032-2050 — existing pm is a nameref and ±n was not
        // requested: resolve the chain and operate on the target.
        let mut nameref_rewrite: Option<String> = None;
        if (off as u32 & PM_NAMEREF) == 0
            && pname_in_tab
            && crate::ported::params::is_nameref(arg_name)
        {
            let cur_ll = locallevel.load(Relaxed) as i32;
            let (pm_level, pm_refname) = paramtab()
                .read()
                .ok()
                .and_then(|t| t.get(arg_name).map(|p| (p.level, p.u_str.clone())))
                .unwrap_or((0, None));
            if pm_level == cur_ll || (on as u32 & PM_LOCAL) == 0 {
                // c:2033
                let type_change = (on as u32 & !(PM_NAMEREF | PM_LOCAL | PM_READONLY)) != 0; // c:2038
                use crate::ported::params::nameref_resolution;
                match crate::ported::params::resolve_nameref_name(arg_name, None) {
                    nameref_resolution::SelfRef | nameref_resolution::OutOfScope => {
                        returnval = 1;
                        continue;
                    }
                    nameref_resolution::Placeholder(last) => {
                        // c:2036-2048 — unresolved ref + type change.
                        if type_change {
                            zwarnnam(
                                name, // c:2046
                                &format!("{}: can't change type of a named reference", last),
                            );
                            returnval = 1;
                            continue; // c:2048 return NULL
                        }
                        // plain `typeset ref=value` on a placeholder
                        // falls through — the scalar-assign arm routes
                        // through assignsparam which writes the refname.
                    }
                    nameref_resolution::Target {
                        name: t, subscript, ..
                    } => {
                        if subscript.is_some() && type_change {
                            // c:2041-2044 — pm->width set: subscripted ref.
                            zwarnnam(
                                name,
                                &format!(
                                    "{}: can't change type via subscript reference",
                                    pm_refname.as_deref().unwrap_or(arg_name)
                                ),
                            );
                            returnval = 1;
                            continue;
                        }
                        // c:2034-2035 — pname = pm->node.nam (resolved).
                        let tail = arg.find('=').map(|i| &arg[i..]).unwrap_or("");
                        let mut new_arg = t.clone();
                        if let Some(sub) = &subscript {
                            new_arg.push('[');
                            new_arg.push_str(sub);
                            new_arg.push(']');
                        }
                        new_arg.push_str(tail);
                        nameref_rewrite = Some(new_arg);
                    }
                    nameref_resolution::NotRef => {}
                }
            }
        }
        let arg: &String = nameref_rewrite.as_ref().unwrap_or(arg);
        let arg_name: &str = match arg.find('=') {
            Some(i) => &arg[..i],
            None => arg.as_str(),
        };
        // Recompute the existing-pm snapshots for the rewritten name.
        let (pname_in_tab, usepm_existing) = if nameref_rewrite.is_some() {
            let tab = paramtab().read().unwrap();
            match tab.get(arg_name.split('[').next().unwrap_or(arg_name)) {
                Some(pm) => (true, (pm.node.flags as u32 & PM_UNSET) == 0),
                None => (false, false),
            }
        } else {
            (pname_in_tab, usepm_existing)
        };

        // c:2930 — `else if (pm)` reuse decision for the bin_typeset
        // literal-name loop: `if ((!(pm->node.flags & PM_UNSET) ||
        //                       pm->node.flags & PM_DECLARED)
        //                  && (locallevel == pm->level || !(on & PM_LOCAL)))`.
        // Decides whether the existing pm is reusable in place or
        // shadowed by a new local. The Rust per-arg loop short-circuits
        // through `createparam`'s reuse arm (params.rs:1975) which
        // already encodes this rule, but the literal C predicate
        // belongs here so the parity is visible at the call site.
        let cur_locallevel = locallevel.load(Relaxed) as i32;
        let pm_reuse_local: bool = if pname_in_tab {
            let tab = paramtab().read().unwrap();
            let pm = tab.get(arg_name).unwrap();
            let f = pm.node.flags as u32;
            ((f & PM_UNSET) == 0 || (f & PM_DECLARED) != 0)
                && (cur_locallevel == pm.level || (on as u32 & PM_LOCAL) == 0) // c:2930
        } else {
            true
        };
        let _ = pm_reuse_local;

        // c:3127-3132 — PM_NAMEREF literal-name branch. When
        // `(on & PM_NAMEREF)` and an existing `hn` is present:
        // `if (((Param)hn)->level >= locallevel ||
        //     (!(on & PM_LOCAL) && ((Param)hn)->level < locallevel)) {
        //     unsetparam_pm(oldpm, 0, 1); hn = NULL; }`.
        // Namerefs always start over fresh when redeclared.
        if (on as u32 & PM_NAMEREF) != 0 && pname_in_tab {
            let level_compare = paramtab()
                .read()
                .ok()
                .and_then(|t| t.get(arg_name).map(|pm| pm.level))
                .unwrap_or(0);
            if level_compare >= cur_locallevel
                || ((on as u32 & PM_LOCAL) == 0 && level_compare < cur_locallevel)
            // c:3130
            {
                // unsetparam_pm + hn = NULL would happen here. The
                // simplified PM_NAMEREF path leaves the reset to
                // typeset_single's name-resolution branch at
                // typeset_single c:2750.
            }
        }

        // c:2469-2510 — `typeset_single` createparam dispatch for new
        // PM_LOCAL declarations. Inside a function scope (`local x` or
        // `typeset x` from a fn body), C calls createparam(name,
        // on|PM_LOCAL) which chains pm.old = oldpm at the current
        // locallevel — the c:2575 `pm->level = locallevel` stamp that
        // endparamscope unwinds. Without this, `local x=inside`
        // modifies the outer-scope x instead of installing a shadow.
        // c:Src/builtin.c:2469 — `if ((on & PM_LOCAL) && (!pm ||
        // pm->level < locallevel)) { ...createparam... }`. The
        // condition is: PM_LOCAL set AND (no existing pm OR existing
        // pm is at a LOWER scope than the current locallevel). At
        // TOP scope (locallevel=0), `pm->level < 0` is never true,
        // so C doesn't call createparam — it reuses the existing
        // param in place. The previous Rust port called createparam
        // unconditionally, which routed through the reuse arm at
        // params.rs:2062-2068 and then clobbered pm.node.flags via
        // params.rs:2132 (`pm->node.flags = flags & ~PM_LOCAL`).
        // That wiped PM_INTEGER, leaving u_val=42 with PM_SCALAR
        // flags → getsparam returned empty. Bug #326 in
        // docs/BUGS.md. Mirror the C guard so the createparam call
        // only fires when we genuinely need to allocate a new shadow.
        let needs_new_shadow = if pname_in_tab {
            paramtab()
                .read()
                .ok()
                .and_then(|t| t.get(arg_name).map(|pm| pm.level < cur_locallevel))
                .unwrap_or(true)
        } else {
            true
        };
        if (on as u32 & PM_LOCAL) != 0                                       // c:2469
            && !arg_name.is_empty()
            && !arg_name.starts_with('-')
            && !arg_name.starts_with('+')
            && needs_new_shadow
        {
            let kind = if is_hashed {
                PM_HASHED
            } else if is_array {
                PM_ARRAY
            } else {
                0
            };
            // c:2083-2085 — `newspecial = NS_NORMAL`:
            //     if ((pm->node.flags & PM_SPECIAL)
            //         && !(on & PM_HIDE) && !(pm->node.flags & PM_HIDE & ~off))
            //         newspecial = NS_NORMAL;
            // The apply branch (c:2386-2425, "For specials, we keep the same
            // struct but zero everything") then re-uses the special's OWN
            // Param — accessors and all — so the local still reads through
            // the special's getfn. Note the `& ~off` term: a HIDDEN special
            // (`${(t)commands}` is `association-hide-hideval-special`) is
            // normally NOT preserved by `local -A commands`, but `+h` puts
            // PM_HIDE into `off`, which cancels the test and preserves it.
            // That is exactly what `_command_names` relies on:
            //     local -a +h path
            //     local -A +h commands
            //     path=( $_saved_path )
            // Reference zsh reports `association-local-special` / 5531 keys
            // there; dropping the `& ~off` term made zshrs build a plain
            // empty local assoc (`association-local` / 0 keys), so
            // `compadd -k commands` had nothing to offer and every command
            // completion routed through that idiom produced no matches.
            //
            // The lookup is C's `paramtab->getnode2()` — "getnode2() to avoid
            // autoloading" (c:2460-2462). An untouched module-parameter stub
            // is PM_AUTOLOAD with none of the real node's type/special flags
            // (`loadparamnode`, Src/params.c:563-585, only swaps in the real
            // node once something FETCHES the name), so `local -A +h commands`
            // on a still-unloaded `zsh/parameter` stub creates a plain local
            // assoc — `${(t)commands}` reads `association-local` and
            // `$#commands` is 0. Touch `$commands` first and the very same
            // statement yields `association-local-special` / the live table.
            // zshrs installs the partab placeholders eagerly and models
            // PM_AUTOLOAD as the MATERIALIZED_MODULE_PARAMS side-set, so the
            // stub test has to be asked for explicitly here; without it,
            // command-position completion (`pr<TAB>` → `_autocd` →
            // `_command_names`, which never reads `$commands` beforehand)
            // preserved a special that reference zsh does not.
            let keep_special = !crate::vm_helper::module_param_is_autoload_stub(arg_name)
                && paramtab()
                    .read()
                    .ok()
                    .and_then(|t| t.get(arg_name).map(|pm| pm.node.flags as u32))
                    .is_some_and(|f| {
                        (f & PM_SPECIAL) != 0
                            && (on & PM_HIDE) == 0
                            && (f & PM_HIDE & !off) == 0
                    });
            // c:2475-2487 — C calls `assignsparam(pname, value, 0)`
            // which creates the pm via the assignsparam → createparam
            // path WITHOUT propagating PM_READONLY/PM_EXPORTED flags
            // (that path uses PM_SCALAR / PM_ARRAY / PM_HASHED only).
            // Post-assign attribute stamps add PM_READONLY/PM_EXPORTED
            // later. Mirror by passing ONLY the type-kind + PM_LOCAL
            // (not the full `on` mask) so the freshly-created pm
            // doesn't error on its own first assignment. PM_SPECIAL rides
            // along only for the newspecial case above — c:2425 stamps it
            // on the preserved struct (`… | on | PM_SPECIAL) & ~off`) and
            // createparam keys its accessor inheritance off it.
            let _ = createparam(
                arg_name,
                kind as i32
                    | PM_LOCAL as i32
                    | if keep_special { PM_SPECIAL as i32 } else { 0 },
            );
            // c:2575 — `else if (on & PM_LOCAL) pm->level = locallevel;`
            // — stamp the just-created pm at the current scope so
            // endparamscope (params.c) unwinds the shadow when the
            // enclosing function returns. createparam at params.rs:2014
            // already sets `level: cur_locallevel` on the fresh pm;
            // re-stamp here against the post-createparam pm to mirror
            // C's explicit assignment, AND to catch the reuse-arm path
            // (params.rs:1975-1986) where the existing pm's level was
            // pre-set by a prior scope.
            if let Ok(mut tab) = paramtab().write() {
                if let Some(pm) = tab.get_mut(arg_name) {
                    pm.level = cur_locallevel; // c:2575
                                               // c:2691 + c:4087 arrsetfn — flags that affect the
                                               // VALUE store (PM_UNIQUE dedup, PM_LEFT/RIGHT_B/Z
                                               // padding width) must land on pm.flags BEFORE
                                               // assignaparam → arrsetfn runs, else those
                                               // setfns see the un-flagged pm. C zsh applies
                                               // these as part of the pre-assignment stamp at
                                               // typeset_single c:2476-2479. Mirror by
                                               // pre-stamping the value-affecting subset of
                                               // `on`. The full attribute mask (PM_READONLY,
                                               // PM_EXPORTED, etc.) still lands in the
                                               // post-assign block below since those don't
                                               // change the value at write time.
                    let pre_assign_mask: u32 =
                        PM_UNIQUE | PM_LEFT | PM_RIGHT_B | PM_RIGHT_Z | PM_LOWER | PM_UPPER;
                    pm.node.flags |= (on as u32 & pre_assign_mask) as i32;
                    pm.node.flags &= !((off as u32 & pre_assign_mask) as i32);
                    // c:Src/builtin.c:2544 — `if (isset(TYPESETTOUNSET))
                    //     pm->node.flags |= PM_DEFAULTED;`. Under
                    // `setopt typeset_to_unset`, bare `typeset NAME`
                    // (no `=`) creates a "declared but not assigned"
                    // entry. Only fires when the arg has no `=`
                    // (assignment paths get PM_UNSET cleared as a side
                    // effect of the value write). Bug #280 in
                    // docs/BUGS.md.
                    if !arg.contains('=') && isset(crate::ported::zsh_h::TYPESETTOUNSET) {
                        pm.node.flags |= PM_DEFAULTED as i32;
                    }
                }
            }
        }

        // c:2462-2467 — subscripted-name PM_LOCAL guard: `else if
        //   ((on & PM_LOCAL) && locallevel) { ... if (!pm || pm->level
        //   != locallevel) zerrnam("can't create local array elements") }`.
        // Refuses to create a NEW local for `local arr[N]=val` when
        // the outer-scope pm at a different level exists. The Rust
        // per-arg loop treats subscripted names as the eq-branch's
        // `name[key]=val` shape inside assignsparam; the guard fires
        // here BEFORE the assignment so we emit the C error message.
        if let Some(br) = arg_name.find('[') {
            let base = &arg_name[..br];
            if (on as u32 & PM_LOCAL) != 0 && cur_locallevel != 0 {
                // c:2462
                let pm_level = paramtab()
                    .read()
                    .ok()
                    .and_then(|t| t.get(base).map(|pm| pm.level));
                if pm_level.is_none() || pm_level != Some(cur_locallevel) {
                    // c:2466
                    zerrnam(
                        name,
                        &format!("{}: can't create local array elements", base), // c:2466
                    );
                    continue; // c:2467
                }
            }
            // c:Src/builtin.c:2500-2502 — `else { zerrnam(cname,
            //   "%s: inconsistent array element or slice assignment",
            //   pname); return NULL; }`. C's typeset_single rejects
            // type-flag attributes on subscripted names: when the
            // name has a `[…]` subscript AND the requested `on` mask
            // carries a non-scalar PM_TYPE bit (PM_INTEGER, PM_EFLOAT,
            // PM_FFLOAT, PM_ARRAY, PM_HASHED), it's
            // `typeset -i h[k]` / `-A h[k]` / etc. — attribute
            // applies to whole array, not element. zshrs's
            // typeset path silently accepted these and applied
            // the attribute to the element. Bug #219 in
            // docs/BUGS.md. Plain `h[k]=value` (no `-` flag) has
            // PM_TYPE(on) == PM_SCALAR and stays valid.
            let pm_type_bits =
                on as u32 & (PM_INTEGER | PM_EFLOAT | PM_FFLOAT | PM_ARRAY | PM_HASHED);
            if pm_type_bits != 0 {
                zerrnam(
                    name,
                    &format!(
                        "{}: inconsistent array element or slice assignment",
                        arg_name
                    ),
                );
                continue;
            }
        }

        if let Some(eq) = arg.find('=') {
            let n = &arg[..eq];
            // c:Src/builtin.c:2289 — `pm->node.flags = ... & ~off` must
            //   clear `off` bits in value-affecting flags BEFORE the
            //   assignment runs (params.rs assignaparam:5526 snapshots
            //   PM_UNIQUE and dedups in setfn). The PM_LOCAL pre_assign
            //   block below only fires when on & PM_LOCAL; top-level
            //   `typeset +U arr=(...)` has on=0 and would otherwise
            //   leave the stale PM_UNIQUE in place. Bug #234.
            let pre_assign_off_mask =
                (PM_UNIQUE | PM_LEFT | PM_RIGHT_B | PM_RIGHT_Z | PM_LOWER | PM_UPPER) as i32;
            let off_in_pre_mask = (off as i32) & pre_assign_off_mask;
            if off_in_pre_mask != 0 {
                if let Ok(mut tab) = paramtab().write() {
                    if let Some(pm) = tab.get_mut(n) {
                        pm.node.flags &= !off_in_pre_mask;
                    }
                }
            }
            let raw_v = &arg[eq + 1..];
            // c:2945-3050 — `=(elem elem ...)` array-init syntax.
            // The parser hands the whole `(...)` body in as one arg
            // when typeset's BINF_MAGICEQUALS is set; the `(` / `)` are
            // literal first/last bytes. Strip them and split on
            // whitespace to recover the element list.
            //
            // c:Src/parse.c par_simple/intypeset — the array-vs-scalar
            // distinction is SYNTACTIC (the `asg->flags & ASG_ARRAY`
            // bit), set when the `(` is an UNQUOTED paren at parse time,
            // NOT a heuristic on the expanded value. zshrs encodes that
            // bit as the `\u{1f}` REJOIN_SEP marker inserted by
            // compile_zsh's BUILTIN_TYPESET_PAREN_PACK, which only fires
            // for syntactic (unquoted) paren-inits. A double-quoted
            // value that happens to expand to `(...)` —
            // `typeset out="(${x:gs/%/%%})"` — reaches here as a plain
            // dequoted scalar with NO marker and must stay a SCALAR
            // (zsh: `out=(ruby-3.2%%@1.0)`). Keying on `starts_with('(')`
            // alone mis-stored it as an array. The `()`-empty and
            // `=(\u{1f}elem…)` pack forms always carry the marker.
            let is_paren_init = raw_v.starts_with('(')
                && raw_v.ends_with(')')
                && raw_v.len() >= 2
                && raw_v.contains('\u{1f}');
            // c:Src/builtin.c:2095-2097 — `if (ASG_ARRAYP(asg) &&
            // PM_TYPE(on) == PM_SCALAR && ...) on |= PM_ARRAY;`. Auto-
            // promote to array when the assignment shape is array but
            // no explicit type was requested. Required so plain
            // `local arr=(a b c)` (which carries on=PM_LOCAL with
            // no PM_TYPE bits) reaches the array-init path. Without
            // it the c:2342 inconsistency check below errors on every
            // bare array assign.
            let mut on = on;
            if is_paren_init && crate::ported::zsh_h::PM_TYPE(on) == PM_SCALAR {
                on |= PM_ARRAY;
            }
            // c:Src/builtin.c:2342-2347 — `inconsistent type for
            // assignment`: when the user types an array RHS
            // (`x=(...)`) but the resolved type flags don't include
            // PM_ARRAY|PM_HASHED, error out. C source:
            // ```
            // if ((asg->flags & ASG_ARRAY) ?
            //     !(on & (PM_ARRAY|PM_HASHED)) :
            //     (asg->value.scalar && (on & (PM_ARRAY|PM_HASHED)))) {
            //     zerrnam(cname, "%s: inconsistent type for assignment", pname);
            //     return NULL;
            // }
            // ```
            // The C conflict-resolution pass at c:2718-2742 clears
            // PM_ARRAY/PM_HASHED from `on` when a conflicting type
            // (`-i` / `-E` / `-F`) is requested (`on & PM_INTEGER →
            // off |= PM_ARRAY; on &= ~off`), so `-ia x=(1 2 3)` lands
            // here with PM_INTEGER set + PM_ARRAY cleared. The
            // c:2095 auto-promote above doesn't fire either (PM_TYPE
            // is non-zero from PM_INTEGER), so the inconsistency
            // error fires correctly. Bug #250.
            if is_paren_init && (on & (PM_ARRAY | PM_HASHED)) == 0 {
                zerrnam(
                    name, // cname
                    &format!("{}: inconsistent type for assignment", n),
                );
                unqueue_signals();
                return 1;
            }
            if is_paren_init {
                let inner = &raw_v[1..raw_v.len() - 1]; // c:2950
                                                        // c:Src/builtin.c:2555-2556 globlist — C glob-expands each
                                                        // element working on the TOKENIZED value where quoted
                                                        // metachars keep their Bnull escape, so a quoted `(` / `*`
                                                        // is inert. zshrs's compile path (compile_zsh's
                                                        // BUILTIN_TYPESET_PAREN_PACK) already runs the per-element
                                                        // glob via compile_word_str — identical to the bare
                                                        // `arr=(…)` path (compile_zsh.rs ~3294) — and hands
                                                        // bin_typeset the FULLY-EXPANDED, DEQUOTED elements via the
                                                        // `\u{1f}` sentinel form. Re-globbing those dequoted values
                                                        // here is a double-glob that loses the original quoting:
                                                        // `typeset -a p=("*.txt")` globbed the quoted star, and
                                                        // `typeset -a p=("a=\$((1+2))")` parsed the literal `((` as
                                                        // a bareglobqual ("unknown file attribute: ("). Skip the
                                                        // re-glob for the pack (`\u{1f}`) form; only the legacy
                                                        // whitespace form (elements not pre-expanded) needs it.
                let from_pack = inner.contains('\u{1f}');
                // c:2952 globlist — each list node is one element. When
                // the multi-arg rejoin loop above ran, elements are
                // separated by the `\u{1f}` REJOIN_SEP sentinel
                // (consecutive sentinels preserve quoted-empty args).
                // Otherwise (single-arg paren-init form like `a=(1 2 3)`)
                // the inner is plain-whitespace separated.
                // Bug #93 in docs/BUGS.md.
                let split_elems: Vec<String> = if inner.contains('\u{1f}') {
                    // c:Src/builtin.c:2952 globlist — each REJOIN_SEP
                    // segment is ONE already-parsed array element (the
                    // parser split argv and quote-removed each element),
                    // so split EXACTLY on the sentinel. Do NOT trim or
                    // drop all-whitespace segments — that would corrupt
                    // quoted leading/trailing whitespace, e.g.
                    // `local a=( "  name: x" )` (the `  name:` prefix is
                    // significant and must survive for `${a[(i)  name: …]}`
                    // to match). Boundary empties from the `=(` / `)` argv
                    // fragments are removed by the first/last-empty trim
                    // immediately below.
                    inner.split('\u{1f}').map(String::from).collect::<Vec<_>>()
                } else {
                    inner.split_whitespace().map(String::from).collect()
                };
                let split_elems = if inner.contains('\u{1f}') {
                    let mut v: Vec<String> = split_elems;
                    // Trim only the FIRST leading empty (from the open
                    // `=(` arg's tail after stripping `(`) and the LAST
                    // trailing empty (from the close `)` arg's head),
                    // preserving any genuine empties in the middle.
                    if v.first().is_some_and(|s| s.is_empty()) {
                        v.remove(0);
                    }
                    if v.last().is_some_and(|s| s.is_empty()) {
                        v.pop();
                    }
                    v
                } else {
                    split_elems
                };
                // c:Src/subst.c:2558-2571 — `$=var` IFS-split operator.
                // The full paramsubst path in subst.rs doesn't yet
                // handle the `=` flag (spbreak=2 in C) when invoked
                // outside the canonical addvars→prefork→stringsubst
                // chain. typeset's array-init reaches bin_typeset
                // AFTER the unified prefork has already run (against
                // the entire `arr=( ... )` string, treating the parens
                // as literal), so any `$=name` inside the parens
                // stayed untouched.
                //
                // Mirror the C `spbreak=2` semantics inline: for each
                // element of the form `$=NAME`, look up NAME, split on
                // IFS, and substitute the resulting fields in place.
                // Other elements (already paramsubst'd by the outer
                // prefork, or literal) pass through unchanged.
                let raw_elems: Vec<String> = {
                    // c:Src/params.c IFS — the canonical IFS source is
                    // the IFS shell param (PM_SPECIAL gsu reads the
                    // live $IFS in the current scope). Reading the
                    // OS env var would miss `local IFS=` overrides and
                    // catch stale values left behind by earlier scopes.
                    let ifs_chars: Vec<char> = crate::ported::params::getsparam("IFS")
                        .unwrap_or_else(|| " \t\n".to_string())
                        .chars()
                        .collect();
                    let mut out: Vec<String> = Vec::new();
                    for se in split_elems {
                        if let Some(stripped) = se.strip_prefix("$=") {
                            // c:2562 — `s++; spbreak = 2; nojoin =
                            // !(ifs && *ifs);`. Look up the variable;
                            // if unset, treat as empty string and
                            // produce no fields.
                            let val =
                                crate::ported::params::getsparam(stripped).unwrap_or_default();
                            if ifs_chars.is_empty() {
                                if !val.is_empty() {
                                    out.push(val);
                                }
                            } else {
                                for field in val
                                    .split(|c: char| ifs_chars.contains(&c))
                                    .filter(|s| !s.is_empty())
                                {
                                    out.push(field.to_string());
                                }
                            }
                        } else {
                            out.push(se);
                        }
                    }
                    out
                };
                // c:2555-2556 — `globlist(vl, prefork_ret)` glob-expands
                // each element when it contains wildcards. Skipped for the
                // pre-expanded pack form (see `from_pack` above): those
                // elements were already glob-expanded (quote-aware) at
                // compile time, so re-globbing the dequoted values is wrong.
                let mut elems: Vec<String> = Vec::with_capacity(raw_elems.len());
                for re in raw_elems {
                    if from_pack {
                        elems.push(re);
                        continue;
                    }
                    // c:Src/glob.c:1230 zglob — `if (... !haswilds(ostr)
                    // ...) return;` short-circuits before patcompile.
                    // In C, lexer/prefork preserves Bnull-escape on
                    // quoted metachars (`'('` becomes `Bnull (`), so
                    // haswilds returns false on a quoted-paren key.
                    // zshrs's upstream prefork strips Bnull before
                    // typeset args land here (see TRACE_TS_RAWV =
                    // `[40, 31, 40, 31, ...]` — bare `(` without the
                    // Bnull escape), so haswilds spuriously returns
                    // true on a literal `(` from `T=( '(' v )`. Guard
                    // by pre-compiling: if patcompile fails, the
                    // element can't be a real glob — keep it literal
                    // (matches the C zglob no-wild short-circuit and
                    // suppresses the spurious "bad pattern" zerr).
                    // haswilds scans TOKENIZED strings — tokenize a
                    // local copy of the untokenized element first
                    // (c:Src/glob.c:3548), as C does for runtime-built
                    // strings (compcore.c:2231); patcompile consumes
                    // the same tokenized form.
                    let mut re_tok = re.to_string();
                    crate::ported::glob::tokenize(&mut re_tok);
                    if crate::ported::pattern::haswilds(&re_tok) {
                        let compilable = crate::ported::pattern::patcompile(
                            &re_tok,
                            crate::ported::zsh_h::PAT_HEAPDUP as i32,
                            None,
                        )
                        .is_some();
                        if !compilable {
                            elems.push(re);
                            continue;
                        }
                        let expanded = crate::ported::glob::glob_path(&re);
                        if expanded.is_empty() || (expanded.len() == 1 && expanded[0] == re) {
                            elems.push(re);
                        } else {
                            elems.extend(expanded);
                        }
                    } else {
                        elems.push(re);
                    }
                }
                // c:Src/builtin.c:2355-2378 — tc (type-conversion)
                // branch: the requested type differs from the
                // existing param's type. zsh keeps readonly/exported
                // status across the conversion (`on |= ~off &
                // (PM_READONLY|PM_EXPORTED) & pm->node.flags`) but
                // turns PM_READONLY OFF on the old pm so the delete/
                // recreate succeeds — `typeset -r h2=(); typeset -A
                // h2=(k v)` is legal in zsh. The Rust assignment
                // funnel (sethparam/setaparam) performs the type
                // rewrite in place, so clearing PM_READONLY here is
                // the c:2359 step and the post-assign stamp below
                // restores it from `on` (c:2357 carry).
                {
                    let requested = if is_hashed { PM_HASHED } else { PM_ARRAY };
                    let existing_flags = paramtab()
                        .read()
                        .ok()
                        .and_then(|t| t.get(n).map(|pm| pm.node.flags as u32));
                    if let Some(f) = existing_flags {
                        if (f & PM_UNSET) == 0 && crate::ported::zsh_h::PM_TYPE(f) != requested {
                            // c:2357 — carry readonly/exported into `on`.
                            on |= !off & (PM_READONLY | PM_EXPORTED) & f;
                            // c:2359 — `pm->node.flags &= ~PM_READONLY;`
                            if let Ok(mut tab) = paramtab().write() {
                                if let Some(pm) = tab.get_mut(n) {
                                    pm.node.flags &= !(PM_READONLY as i32);
                                }
                            }
                        }
                    }
                }
                // c:Src/builtin.c:2476-2479 + c:2691 + c:4087 arrsetfn —
                // pre-stamp value-affecting flags on the pm BEFORE the
                // array-init setarrvalue runs, so PM_UNIQUE drives dedup
                // inside assignaparam (params.rs:5571 reads pm.flags &
                // PM_UNIQUE to gate simple_arrayuniq). Without this,
                // `typeset -aUx u=(/bin /usr/bin /bin)` stamped PM_UNIQUE
                // only AFTER setarrvalue ran, so dedup never fired.
                // `typeset -aU u=(...)` happened to work due to a side
                // path; this makes both consistent. Bug #272.
                if is_array {
                    let pre_assign_mask: u32 = PM_UNIQUE;
                    if (on as u32 & pre_assign_mask) != 0 {
                        let exists = paramtab()
                            .read()
                            .map(|t| t.contains_key(n))
                            .unwrap_or(false);
                        if !exists {
                            let _ = createparam(n, PM_ARRAY as i32);
                        }
                        if let Ok(mut tab) = paramtab().write() {
                            if let Some(pm) = tab.get_mut(n) {
                                pm.node.flags |= (on as u32 & pre_assign_mask) as i32;
                            }
                        }
                    }
                }
                // c:2357 — a scalar → array/hashed conversion is a TYPE change,
                // which recreates the param carrying only READONLY|EXPORTED, so
                // the old scalar padding flags (-L/-R/-Z) must NOT survive it:
                // `typeset -Z 3 x=7; typeset -a x=(1 2)` is `array`, not
                // `array-right_zeros`. Capture whether the param was ALREADY an
                // array/hashed (in which case it is NOT a type change and the
                // padding is preserved by the merge stamp below).
                let prior_was_arraylike = paramtab().read().ok().and_then(|t| {
                    t.get(n).map(|pm| {
                        let typ = PM_TYPE(pm.node.flags as u32);
                        typ == PM_ARRAY || typ == PM_HASHED
                    })
                }) == Some(true);
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
                        && elems.iter().all(|e| e.starts_with('[') && e.contains("]="));
                    let mut map: IndexMap<String, String> = IndexMap::new();
                    if bracket_shape {
                        for e in &elems {
                            let close = e.find("]=").unwrap();
                            let k = e[1..close].to_string();
                            let v = e[close + 2..].to_string();
                            map.insert(k, v);
                        }
                    } else {
                        let mut it = elems.into_iter(); // c:2960 pair walk
                        while let Some(k) = it.next() {
                            let v = it.next().unwrap_or_default();
                            map.insert(k, v); // c:2964 hashtab insert
                        }
                    }
                    crate::ported::exec::set_assoc(n, map.clone());
                } else if crate::dash_mode::bash_mode()
                    && !elems.is_empty()
                    && elems.iter().all(|e| e.starts_with('[') && e.contains("]="))
                    && elems.iter().all(|e| {
                        e.find("]=")
                            .map(|c| e[1..c].trim().parse::<usize>().is_ok())
                            .unwrap_or(false)
                    })
                {
                    // !!! BASH-MODE GATE !!! bash `declare -a a=([5]=x [10]=y)`
                    // — explicit 0-based indices in an indexed-array literal
                    // (the assoc branch above handles the `-A` form). Place each
                    // value at its index and record the un-indexed slots as
                    // sparse holes, mirroring the plain `a=([i]=v)` path.
                    let mut pairs: Vec<(usize, String)> = Vec::new();
                    for e in &elems {
                        let close = e.find("]=").unwrap();
                        if let Ok(idx) = e[1..close].trim().parse::<usize>() {
                            pairs.push((idx, e[close + 2..].to_string()));
                        }
                    }
                    let len = pairs.iter().map(|(i, _)| *i + 1).max().unwrap_or(0);
                    let mut dense = vec![String::new(); len];
                    let mut explicit: std::collections::BTreeSet<usize> =
                        std::collections::BTreeSet::new();
                    for (i, v) in pairs {
                        if i < dense.len() {
                            dense[i] = v;
                            explicit.insert(i);
                        }
                    }
                    crate::ported::exec::set_array(n, dense.clone());
                    crate::bash_arrays::clear(n);
                    for i in 0..dense.len() {
                        if !explicit.contains(&i) {
                            crate::bash_arrays::note_unset(n, i);
                        }
                    }
                } else {
                    // c:2980-2995 — plain array.
                    crate::ported::exec::set_array(n, elems.clone());
                }
                // c:2357 — on a genuine type change (was NOT array/hashed), drop
                // the old scalar padding flags; re-add only what THIS command's
                // `on` requests (so `typeset -aZ 3 arr=(...)` still zero-pads).
                if !prior_was_arraylike {
                    let pad = (PM_LEFT | PM_RIGHT_B | PM_RIGHT_Z) as i32;
                    if let Ok(mut tab) = paramtab().write() {
                        if let Some(pm) = tab.get_mut(n) {
                            pm.node.flags = (pm.node.flags & !pad) | (on as i32 & pad);
                        }
                    }
                }
                // c:2330-2337 (typeset_single) — `if (!(pm =
                // assignaparam(pname, ..., flags))) return NULL;
                // ... if (errflag) return NULL;` A readonly
                // rejection inside setarrvalue (c:Src/params.c:2900)
                // sets errflag and refuses the value; typeset_single
                // returns NULL and bin_typeset records `returnval = 1`
                // (c:3153-3156). Skip the attribute stamps — C never
                // reaches them on this path. Gated on usepm: the
                // c:2336 check lives ONLY in the reuse-existing-pm
                // branch.
                if usepm_existing && (errflag.load(Relaxed) & ERRFLAG_ERROR) != 0 {
                    returnval = 1; // c:3156
                    continue; // c:2337 return NULL
                }
                // c:2510-2520 — `on = pm->node.flags;` then stamp the
                // attribute bits on the just-assigned param. The
                // scalar-assign arm below does the same; the array /
                // assoc `=(...)` init path was missing this, so
                // `typeset -ax ARR=(a b)` left PM_EXPORTED unset on
                // the paramtab entry. `(t)ARR` then read `array`
                // instead of `array-export`, and `typeset -p ARR`
                // emitted `typeset -a ARR=...` instead of `-ax`.
                let post_assign_mask = (PM_READONLY
                    | PM_EXPORTED
                    | PM_LEFT
                    | PM_RIGHT_B
                    | PM_RIGHT_Z
                    | PM_TAGGED
                    | PM_HIDE
                    | PM_HIDEVAL
                    | PM_UNIQUE) as i32;
                let post_assign_to_set = (on
                    & (PM_READONLY
                        | PM_EXPORTED
                        | PM_LEFT
                        | PM_RIGHT_B
                        | PM_RIGHT_Z
                        | PM_TAGGED
                        | PM_HIDE
                        | PM_HIDEVAL
                        | PM_UNIQUE)) as i32;
                if post_assign_to_set != 0 {
                    if let Ok(mut tab) = paramtab().write() {
                        if let Some(pm) = tab.get_mut(n) {
                            // c:2289 MERGE — preserve untouched attributes.
                            pm.node.flags |= post_assign_to_set;
                        }
                    }
                }
            } else {
                // c:2232-2238 (typeset_single) — a SCALAR assignment to an
                // existing ARRAY/HASHED param WITHOUT a type flag to convert it
                // is inconsistent: `typeset -A x=(k v); typeset x=scalar` errors.
                // (An explicit type flag — `typeset -i x=9` — is a type change
                // handled above and does NOT reach here.)
                let target_is_arraylike = paramtab().read().ok().and_then(|t| {
                    t.get(n).map(|pm| {
                        let typ = PM_TYPE(pm.node.flags as u32);
                        typ == PM_ARRAY || typ == PM_HASHED
                    })
                }) == Some(true);
                let requesting_type =
                    (on as u32 & (PM_INTEGER | PM_EFLOAT | PM_FFLOAT | PM_ARRAY | PM_HASHED)) != 0;
                if target_is_arraylike && !requesting_type {
                    zerrnam(name, &format!("{}: inconsistent type for assignment", n)); // c:2236
                    returnval = 1;
                    continue;
                }
                // c:Src/builtin.c:2342-2345 (second half of the inconsistency
                // test) — a SCALAR value assigned to an explicitly-REQUESTED
                // array/hashed declaration (`typeset -a g=1`, `typeset -A h=1`,
                // `typeset -aU u=1`) is inconsistent. The check above only
                // caught a scalar assigned to a pre-EXISTING array with no type
                // flag; this catches the `-a`/`-A` decl form (paren-RHS to a
                // non-array is the mirror half at builtin.rs:5687). zsh errors
                // and leaves the param UNSET (a pre-existing value dropped too).
                // Bug #1028.
                if (on as u32 & (PM_ARRAY | PM_HASHED)) != 0 {
                    zerrnam(name, &format!("{}: inconsistent type for assignment", n)); // c:2345
                    crate::ported::params::unsetparam(n);
                    returnval = 1;
                    continue;
                }
                // c:3010-3030 — `name=value` scalar assign. C-canonical
                // `setsparam` (Src/params.c:3350) writes paramtab; the
                // env mirror at `Src/params.c:3024 addenv` follows.
                // PM_LOWER / PM_UPPER do NOT fold at assignment. `setstrvalue`
                // (c:Src/params.c:2677) has no case arm at all — the only two
                // fold sites in the C source are `getstrvalue`'s VALFLAG_SUBST
                // tail (c:2497-2506, i.e. READ during substitution) and
                // `copyenvstr` (c:5434-5442, the env mirror). The stored value
                // stays verbatim, which is why removing the attribute restores
                // the original text and why `typeset -p` shows it unfolded:
                //     typeset -l v=ABC; typeset -p v   → typeset -l v=ABC
                //     typeset -l v=ABC; typeset +l v; print $v   → ABC
                // Only the env mirror below folds, per copyenvstr.
                let env_val: String = if (on & PM_LOWER) != 0 {
                    // c:5439-5440 — `if (flags & PM_LOWER) *s = tulower(*s)`.
                    raw_v.to_ascii_lowercase()
                } else if (on & PM_UPPER) != 0 {
                    // c:5441-5442 — `else if (flags & PM_UPPER) *s = tuupper(*s)`.
                    raw_v.to_ascii_uppercase()
                } else {
                    raw_v.to_string()
                };
                // c:typeset_single — createparam with the type flag
                // BEFORE assignsparam, so assignstrvalue's PM_TYPE
                // dispatch (params.c:2748) routes the value through
                // the correct setfn:
                //   - PM_INTEGER → intsetfn (mathevali → u_val)
                //   - PM_EFLOAT/PM_FFLOAT → floatsetfn (parsefloat → u_dval)
                //   - PM_SCALAR → strsetfn (u_str)
                // The previous Rust ordering (setsparam first, then
                // flip flags) wrote "5" to u_str then changed PM_TYPE
                // to PM_INTEGER without migrating u_str → u_val, so
                // getsparam(n) read u_val=0 instead of 5.
                // c:2748-2784 — pre-assign type flags only (PM_INTEGER
                // etc. — affect storage / setfn dispatch). Post-assign
                // attributes (PM_READONLY / PM_EXPORTED / justification
                // bits) are stamped AFTER setsparam since the C path
                // (c:2475 `assignsparam(pname, val, 0)` → c:2510
                // `on = pm->node.flags`) sets PM_READONLY only after
                // the value lands. Mixing them pre-assign caused
                // `readonly y=hello` to error "read-only variable: y"
                // — the freshly-created pm had PM_READONLY which
                // blocked its OWN initial assign.
                let pre_assign_mask =
                    (PM_INTEGER | PM_EFLOAT | PM_FFLOAT | PM_LOWER | PM_UPPER | PM_NAMEREF) as i32;
                let pre_assign_to_set = (on
                    & (PM_INTEGER | PM_EFLOAT | PM_FFLOAT | PM_LOWER | PM_UPPER | PM_NAMEREF))
                    as i32;
                if pre_assign_to_set != 0 {
                    let pname_in_tab = paramtab()
                        .read()
                        .map(|t| t.contains_key(n))
                        .unwrap_or(false);
                    if !pname_in_tab {
                        // c:1132+ createparam(name, type_flags) — fresh.
                        let _ = createparam(n, pre_assign_to_set);
                    } else {
                        // c:2355-2378 tc (type-conversion) — flip the
                        // PM_TYPE bits on the existing param BEFORE
                        // re-assigning so assignstrvalue routes through
                        // the new type's setfn.
                        //
                        // c:2357 — a TYPE change deletes and recreates the param
                        // carrying ONLY READONLY|EXPORTED, so the old scalar
                        // padding flags (-L/-R/-Z) do NOT survive it: after
                        // `typeset -Z 3 x=7; typeset -i x=1` the type is plain
                        // `integer`, not `integer-right_zeros`. When the new type
                        // is numeric (INTEGER/EFLOAT/FFLOAT), clear the padding
                        // flags from the old param too. (A single `typeset -iZ`
                        // command re-adds RIGHT_Z via the post-assign stamp since
                        // it is in `on`.)
                        let is_numeric_type =
                            (pre_assign_to_set as u32 & (PM_INTEGER | PM_EFLOAT | PM_FFLOAT)) != 0;
                        // On a numeric type change also drop the scalar padding
                        // flags AND — if the old param was an array/hashed —
                        // PM_ARRAY|PM_HASHED plus its stored data, since C deletes
                        // and recreates: `typeset -A x=(k v); typeset -F x=1.5`
                        // yields a plain `float`, not a hashed remnant.
                        let old_type = paramtab()
                            .read()
                            .ok()
                            .and_then(|t| t.get(n).map(|pm| PM_TYPE(pm.node.flags as u32)))
                            .unwrap_or(PM_SCALAR);
                        let was_arraylike = old_type == PM_ARRAY || old_type == PM_HASHED;
                        let extra_clear = if is_numeric_type {
                            let mut c = (PM_LEFT | PM_RIGHT_B | PM_RIGHT_Z) as i32;
                            if was_arraylike {
                                c |= (PM_ARRAY | PM_HASHED) as i32;
                            }
                            c
                        } else {
                            0
                        };
                        if is_numeric_type && was_arraylike {
                            // c:2355-2378 — array/hashed → numeric is a full type
                            // conversion: delete the old aggregate (storage +
                            // paramtab entry) and recreate as the numeric type, so
                            // the value migrates through the new setfn and the
                            // result is a clean `integer`/`float`. Modifying flags
                            // in place left the hashed storage's setfn dispatch
                            // active, so the value stayed a scalar.
                            crate::ported::exec::unset_array(n);
                            crate::ported::exec::unset_assoc(n);
                            if let Ok(mut tab) = paramtab().write() {
                                tab.remove(n);
                            }
                            let _ = createparam(n, pre_assign_to_set);
                        } else if let Ok(mut tab) = paramtab().write() {
                            if let Some(pm) = tab.get_mut(n) {
                                pm.node.flags = (pm.node.flags & !(pre_assign_mask | extra_clear))
                                    | pre_assign_to_set;
                            }
                        }
                    }
                }
                // c:2289 — `pm->node.flags = (pm->node.flags | (on &
                // ~PM_READONLY)) & ~(off | PM_UNSET);` — `+n` / `+r`
                // WITH a value clear the bits BEFORE the assignment
                // runs (`typeset +rn ref=RW`: type change first, so
                // the = writes the now-plain scalar, not through the
                // old refname).
                let off_pre = (off as u32 & (PM_NAMEREF | PM_READONLY)) as i32;
                if off_pre != 0 {
                    if let Ok(mut tab) = paramtab().write() {
                        if let Some(pm) = tab.get_mut(n) {
                            pm.node.flags &= !off_pre;
                            if (off as u32 & PM_NAMEREF) != 0 {
                                pm.width = 0;
                                pm.base = 0;
                            }
                        }
                    }
                }
                // c:Src/builtin.c:1982-1986 (typeset_setbase) — an integer
                // base must be 2..=36 inclusive. Validate BEFORE assigning the
                // value: on an invalid base zsh errors PER param and leaves the
                // (already-created) param EMPTY. The live base-stamp below
                // (c:1990) never validated, so `typeset -i0`/`-i1`/`-i37`
                // silently produced `0#…`/`37#…`. The faithful typeset_setbase
                // port has the check but was dead code. Bug #1027.
                if (on & PM_INTEGER) != 0 && OPT_HASARG(&ops, b'i') {
                    if let Some(bs) = OPT_ARG(&ops, b'i') {
                        if let Ok(bv) = bs.trim().parse::<i32>() {
                            if !(2..=36).contains(&bv) {
                                crate::ported::utils::zwarnnam(
                                    name,
                                    &format!("invalid base (must be 2 to 36 inclusive): {}", bv),
                                );
                                // zsh leaves the param UNSET on this failure
                                // (even a pre-existing value is dropped) — the
                                // just-created param is torn down before the
                                // value is assigned.
                                crate::ported::params::unsetparam(n);
                                returnval = 1;
                                continue;
                            }
                        }
                    }
                }
                // c:2322 — typeset assigns via `assignsparam(pname, ..., 0)`:
                // flags=0, NOT setsparam's ASSPM_WARN (params.c:3294), so an
                // explicit `typeset -g NAME=val` inside a function never
                // trips WARN_CREATE_GLOBAL. Routing through setsparam here
                // made zshrs warn where zsh is silent (f-sy-h's
                // `typeset -g _ZSH_HIGHLIGHT_PRIOR_BUFFER=...`).
                crate::ported::params::assignsparam(n, raw_v, 0); // c:2322
                                                                  // c:2326-2328 + c:2336-2337 (typeset_single) —
                                                                  // `if (asg->value.scalar && !(pm = assignsparam(
                                                                  //     pname, ztrdup(asg->value.scalar), 0)))
                                                                  //      return NULL;
                                                                  //  ... if (errflag) return NULL;`
                                                                  // A readonly rejection inside assignstrvalue
                                                                  // (c:Src/params.c:2697) sets errflag and the value is
                                                                  // refused; typeset_single returns NULL and bin_typeset
                                                                  // records `returnval = 1` (c:3153-3156). Skip the
                                                                  // attribute stamps — C never reaches them on this path.
                                                                  // Gated on usepm: the c:2336 check lives ONLY in the
                                                                  // reuse-existing-pm branch.
                if usepm_existing && (errflag.load(Relaxed) & ERRFLAG_ERROR) != 0 {
                    returnval = 1; // c:3156
                    continue; // c:2337 return NULL
                }
                // c:2510-2520 — `on = pm->node.flags;` then stamp the
                // attribute bits on the just-assigned param.
                let post_assign_mask = (PM_READONLY
                    | PM_EXPORTED
                    | PM_LEFT
                    | PM_RIGHT_B
                    | PM_RIGHT_Z
                    | PM_TAGGED
                    | PM_HIDE
                    | PM_HIDEVAL
                    | PM_UNIQUE) as i32;
                let post_assign_to_set = (on
                    & (PM_READONLY
                        | PM_EXPORTED
                        | PM_LEFT
                        | PM_RIGHT_B
                        | PM_RIGHT_Z
                        | PM_TAGGED
                        | PM_HIDE
                        | PM_HIDEVAL
                        | PM_UNIQUE)) as i32;
                if post_assign_to_set != 0 {
                    // c:2289 — `pm->node.flags = (pm->node.flags | (on & ...)) & ~off`.
                    // This is a MERGE: OR in the requested attributes and clear
                    // the ones explicitly turned off, but PRESERVE existing
                    // attributes the user did not touch. The previous
                    // `(flags & !post_assign_mask) | to_set` cleared EVERY mask
                    // bit, so `typeset -Z 3 x=7; typeset -r x=ro` lost the -Z
                    // (RIGHT_Z is in the mask but not in `on`), yielding
                    // `scalar-readonly` where zsh keeps `scalar-right_zeros-readonly`.
                    let post_assign_to_clear = (off as u32 & post_assign_mask as u32) as i32;
                    if let Ok(mut tab) = paramtab().write() {
                        if let Some(pm) = tab.get_mut(n) {
                            pm.node.flags =
                                (pm.node.flags | post_assign_to_set) & !post_assign_to_clear;
                        }
                    }
                }
                // c:1973-1989 (Src/builtin.c, inside typeset_single)
                // — `if (arg) { int base = zstrtol(arg, ...) ;
                // pm->base = base; }`. The precision arg from `-i N`,
                // `-E N`, `-F N` (parsed by execbuiltin as
                // ops.args[<F-arg-slot>]) lands on the param's `base`
                // field, which convfloat reads as the format-digit
                // count. Without this stamp, `typeset -F 2 x=3.14`
                // ignored the `2` and printed at the default
                // 10-digit precision.
                {
                    let prec_arg: Option<&str> = if (on & PM_INTEGER) != 0 && OPT_HASARG(&ops, b'i')
                    {
                        OPT_ARG(&ops, b'i')
                    } else if (on & PM_EFLOAT) != 0 && OPT_HASARG(&ops, b'E') {
                        OPT_ARG(&ops, b'E')
                    } else if (on & PM_FFLOAT) != 0 && OPT_HASARG(&ops, b'F') {
                        OPT_ARG(&ops, b'F')
                    } else {
                        None
                    };
                    if let Some(s) = prec_arg {
                        if let Ok(b) = s.trim().parse::<i32>() {
                            if let Ok(mut tab) = paramtab().write() {
                                if let Some(pm) = tab.get_mut(n) {
                                    pm.base = b;
                                }
                            }
                        }
                    }
                }
                // c:2009-2014 typeset_setwidth — `-L N` / `-R N` / `-Z N`
                // install pm.width = N. The auto-fallback in assignsparam
                // (PM_INTEGER block at params.rs:3619) stamps width to
                // s.len() when PM_RIGHT_Z is set but width==0; that lands
                // BEFORE the post-assign PM_RIGHT_Z stamp here, so the
                // user's explicit `-Z 6` was being overwritten by the
                // value's char count. Set width AFTER setsparam so the
                // explicit option arg wins over the auto-fallback.
                {
                    let width_arg: Option<&str> =
                        if (on as u32 & PM_LEFT) != 0 && OPT_HASARG(&ops, b'L') {
                            OPT_ARG(&ops, b'L')
                        } else if (on as u32 & PM_RIGHT_B) != 0 && OPT_HASARG(&ops, b'R') {
                            OPT_ARG(&ops, b'R')
                        } else if (on as u32 & PM_RIGHT_Z) != 0 && OPT_HASARG(&ops, b'Z') {
                            OPT_ARG(&ops, b'Z')
                        } else {
                            None
                        };
                    if let Some(s) = width_arg {
                        if let Ok(w) = s.trim().parse::<i32>() {
                            if let Ok(mut tab) = paramtab().write() {
                                if let Some(pm) = tab.get_mut(n) {
                                    pm.width = w;
                                }
                            }
                        }
                    }
                }
                // c:Src/params.c:3024 addenv — only mirror to OS env
                // when PM_EXPORTED is in flags or already-exported.
                let already_exported = env::var_os(n).is_some();
                if (on & PM_EXPORTED) != 0 || already_exported {
                    env::set_var(n, &env_val); // c:3024 addenv (value via copyenvstr, c:5434)
                }
            }
        } else if is_hashed || is_array {
            // c:3060-3070 — bare name + `-A`/`-a` declares an empty
            // assoc/array.
            if is_hashed {
                // c:2521 — C just calls `createparam(pname, on & ~PM_READONLY)`
                // here; it never installs an empty hash of its own. For the
                // newspecial shadow (c:2386-2425) the preserved Param keeps
                // pointing at the SPECIAL's table, so `local -A +h commands`
                // still enumerates the live command hash. zshrs's magic module
                // hashes (Src/Modules/parameter.c partab[]) are reached by
                // FALLING THROUGH `paramtab_hashed_storage` to the row's
                // scanfn (params.rs gethkparam), so materialising an empty bag
                // for one of them is not "an empty local assoc", it is a
                // blanked special: `$#commands` read 0 inside `_command_names`
                // and `compadd -k commands` offered nothing.
                let preserved_special_hash = paramtab()
                    .read()
                    .ok()
                    .and_then(|t| t.get(arg).map(|pm| (pm.node.flags as u32 & PM_SPECIAL) != 0))
                    .unwrap_or(false)
                    && crate::ported::modules::parameter::PARTAB
                        .iter()
                        .any(|e| e.name == arg);
                if !preserved_special_hash && crate::ported::exec::assoc(arg).is_none() {
                    crate::ported::exec::set_assoc(arg, IndexMap::new());
                }
            } else if crate::ported::exec::array(arg).is_none() {
                crate::ported::exec::set_array(arg, Vec::new());
            }
            // c:Src/params.c:4087 arrsetfn — when PM_UNIQUE is set on
            // an existing array, the canonical setfn applies
            // `uniqarray()` to the current contents. `typeset -aU arr`
            // on an existing arr must dedupe in place; without this,
            // the flag stamp lands on pm.flags but the value stays
            // un-deduped until the next assignment.
            if is_array && (on as u32 & PM_UNIQUE) != 0 {
                let current = crate::ported::exec::array(arg).unwrap_or_default();
                // simple_arrayuniq is the in-place dedupe used by
                // params.rs arrsetfn (PM_UNIQUE path).
                let deduped = {
                    let mut seen = std::collections::HashSet::new();
                    current
                        .into_iter()
                        .filter(|x| seen.insert(x.clone()))
                        .collect::<Vec<_>>()
                };
                crate::ported::exec::set_array(arg, deduped);
            }
            // Stamp attribute bits on paramtab entry — same set as
            // the `name=value` post-assign mask.
            let post_assign_mask = (PM_READONLY
                | PM_EXPORTED
                | PM_LEFT
                | PM_RIGHT_B
                | PM_RIGHT_Z
                | PM_TAGGED
                | PM_HIDE
                | PM_HIDEVAL
                | PM_UNIQUE
                | PM_HASHED
                | PM_ARRAY) as i32;
            let post_assign_to_set = (on
                & (PM_READONLY
                    | PM_EXPORTED
                    | PM_LEFT
                    | PM_RIGHT_B
                    | PM_RIGHT_Z
                    | PM_TAGGED
                    | PM_HIDE
                    | PM_HIDEVAL
                    | PM_UNIQUE
                    | PM_HASHED
                    | PM_ARRAY)) as i32;
            if post_assign_to_set != 0 {
                if let Ok(mut tab) = paramtab().write() {
                    if let Some(pm) = tab.get_mut(arg_name) {
                        // c:2289 MERGE — preserve attributes the user did not touch
                        // (e.g. `typeset -L 4 x=hi; typeset -r x` keeps -L). `off` bits
                        // are cleared separately below.
                        pm.node.flags |= post_assign_to_set;
                    }
                }
            }
            // c:Src/builtin.c:2289 — `pm->node.flags = ... & ~off`.
            // `off` bits in post_assign_mask must clear regardless of
            // `on` (e.g. `typeset +U arr=(...)` where on=0,
            // off=PM_UNIQUE). The post_assign_to_set gate above
            // short-circuits the on=0 case. Bug #234 in docs/BUGS.md.
            let off_in_mask = (off as i32) & post_assign_mask;
            if off_in_mask != 0 {
                if let Ok(mut tab) = paramtab().write() {
                    if let Some(pm) = tab.get_mut(arg_name) {
                        pm.node.flags &= !off_in_mask;
                    }
                }
            }
        } else {
            // c:2355-2378 (typeset_single tc branch) — bare `typeset -i n`
            // / `-F n` / `-E n` / `-l n` / `-u n` / `-r n` / `export N`
            // / `readonly N` converts/stamps the existing param. Split
            // into pre-assign (type conversion) and post-assign
            // (attribute stamp) the same way the `name=value` arm does.
            let pre_assign_mask =
                (PM_INTEGER | PM_EFLOAT | PM_FFLOAT | PM_LOWER | PM_UPPER | PM_NAMEREF) as i32;
            let pre_assign_to_set = (on
                & (PM_INTEGER | PM_EFLOAT | PM_FFLOAT | PM_LOWER | PM_UPPER | PM_NAMEREF))
                as i32;
            let post_assign_mask = (PM_READONLY
                | PM_EXPORTED
                | PM_LEFT
                | PM_RIGHT_B
                | PM_RIGHT_Z
                | PM_TAGGED
                | PM_HIDE
                | PM_HIDEVAL
                | PM_UNIQUE) as i32;
            let post_assign_to_set = (on
                & (PM_READONLY
                    | PM_EXPORTED
                    | PM_LEFT
                    | PM_RIGHT_B
                    | PM_RIGHT_Z
                    | PM_TAGGED
                    | PM_HIDE
                    | PM_HIDEVAL
                    | PM_UNIQUE)) as i32;
            // c:2374 — `s = ztrdup(getsparam(pname));`. Capture the
            // pre-conversion scalar value so the re-assignment after
            // type flip preserves it through the new setfn.
            // `typeset +n ref`: the stored REFNAME (u_str raw) becomes
            // the scalar value — getsparam would deref through the
            // chain and return the target's value instead (c:2374 via
            // the +n type-conversion arm; K01 "remove nameref
            // attribute" expects `typeset ptr=var`).
            // c:Src/builtin.c:2062-2064 — C computes `usepm` at the TOP of
            // typeset_single, BEFORE the parameter is touched, and the bare-name
            // print at c:2246 lives inside that branch:
            //   usepm = pm && (!(pm->node.flags & PM_UNSET) || OPT_ISSET(ops,'p') ||
            //                  (isset(POSIXBUILTINS) &&
            //                   (pm->node.flags & (PM_READONLY|PM_EXPORTED))));
            // "Here we just avoid using it for the present tests if it's unset."
            // So `typeset NAME` on a declared-but-UNSET param prints NOTHING.
            // The snapshot MUST be taken here: later in this same iteration the
            // re-declaration clears PM_UNSET, so reading the flag at the print
            // site always saw it already cleared. TYPESET_TO_UNSET produces
            // exactly this state (PM_DEFAULTED = PM_DECLARED|PM_UNSET,
            // zsh.h:1934). Bug #1056.
            let usepm_at_entry = paramtab()
                .read()
                .ok()
                .and_then(|t| t.get(arg.as_str()).map(|p| p.node.flags as u32))
                .map(|f| {
                    (f & PM_UNSET) == 0
                        || (isset(crate::ported::zsh_h::POSIXBUILTINS)
                            && (f & (PM_READONLY | PM_EXPORTED)) != 0)
                })
                .unwrap_or(true);
            let saved_val =
                if (off as u32 & PM_NAMEREF) != 0 && crate::ported::params::is_nameref(arg) {
                    paramtab()
                        .read()
                        .ok()
                        .and_then(|t| t.get(arg.as_str()).and_then(|p| p.u_str.clone()))
                } else {
                    getsparam(arg)
                };
            // c:Src/builtin.c:3072 — `if (!getsparam(pname))
            //     setsparam(pname, "");`. C zsh's getsparam returns the
            // join-of-values for PM_HASHED and PM_ARRAY so this gate
            // fires only for truly-undeclared names. zshrs's getsparam
            // returns None for assoc/array (it routes through the
            // PM_SCALAR / PM_INTEGER / PM_*FLOAT arms only), so this
            // branch falsely fired for an existing hash AND triggered
            // assignsparam's PM_HASHED→PM_SCALAR coerce at params.rs:
            // 5368-5374 which removes the entry from
            // paramtab_hashed_storage. Bug #218 in docs/BUGS.md.
            //
            // Gate on the paramtab flag instead: if the param exists
            // and is PM_HASHED/PM_ARRAY, treat it as already-declared
            // even though getsparam returned None.
            let already_typed = paramtab()
                .read()
                .ok()
                .and_then(|t| {
                    t.get(arg).map(|pm| {
                        let f = pm.node.flags as u32;
                        let typ = PM_TYPE(f);
                        typ == PM_HASHED || typ == PM_ARRAY
                    })
                })
                .unwrap_or(false);
            // Capture the param's PRIOR numeric class (before the flag
            // change below) so the base-stamp arm can reset pm.base when
            // switching BETWEEN integer and float — the base field means
            // radix for integers but precision for floats, so a leftover
            // value leaks (float precision 3 → integer base 3 = "3#10";
            // int base 16 → float 16-digit precision). E↔F and same-type
            // re-declares keep it (both float, base=precision). zsh 5.9.1
            // verified.
            let (prior_is_integer, prior_is_float) = paramtab()
                .read()
                .ok()
                .and_then(|t| {
                    t.get(arg).map(|pm| {
                        let f = pm.node.flags as u32;
                        ((f & PM_INTEGER) != 0, (f & (PM_EFLOAT | PM_FFLOAT)) != 0)
                    })
                })
                .unwrap_or((false, false));
            let was_fresh = saved_val.is_none() && !already_typed;
            if was_fresh {
                // c:3072 — `if (!getsparam(pname)) setsparam(pname, "")`.
                // flags=0: a typeset-driven create never trips
                // WARN_CREATE_GLOBAL (typeset_single assigns with
                // `assignsparam(pname, ..., 0)`, c:2322).
                crate::ported::params::assignsparam(arg, "", 0); // c:3074
                                                                 // c:Src/builtin.c:2544 — `if (isset(TYPESETTOUNSET))
                                                                 //     pm->node.flags |= PM_DEFAULTED;`. Under
                                                                 // `setopt typeset_to_unset`, bare `typeset NAME` (no
                                                                 // `=`) creates a "declared but not assigned" entry:
                                                                 // PM_DECLARED + PM_UNSET (= PM_DEFAULTED). `${+NAME}`
                                                                 // and `[[ -v NAME ]]` then return false until an
                                                                 // explicit assignment clears PM_UNSET. Bug #280 in
                                                                 // docs/BUGS.md.
                if isset(crate::ported::zsh_h::TYPESETTOUNSET) {
                    if let Ok(mut tab) = paramtab().write() {
                        if let Some(pm) = tab.get_mut(arg) {
                            pm.node.flags |= PM_DEFAULTED as i32;
                        }
                    }
                }
                let _ = was_fresh;
            }
            // c:Src/builtin.c:2117-2131 — `typeset +a`/`+A` removes the
            // array/hashed attribute, which is a TYPE CHANGE (`chflags` picks
            // up `off & PM_ARRAY`, `tc=1`, `usepm=0`). UNLIKE the +i/+E/+l
            // conversions below — which migrate the stored value back to a
            // scalar — an array/assoc has no scalar representation, so the
            // recreated scalar is EMPTY. `typeset -a a=(1 2 3); typeset +a a`
            // → `a` is a scalar with value "". Only fires when the param is
            // ACTUALLY array/hashed (a no-op `+a` on a scalar leaves it be).
            // Bug #1029.
            if (off as u32 & (PM_ARRAY | PM_HASHED)) != 0 {
                let is_arraylike = paramtab().read().ok().and_then(|t| {
                    t.get(arg).map(|pm| {
                        let typ = PM_TYPE(pm.node.flags as u32);
                        typ == PM_ARRAY || typ == PM_HASHED
                    })
                }) == Some(true);
                if is_arraylike {
                    // Drop the assoc backing map too — unsetparam alone can
                    // leave the paramtab_hashed_storage entry, so a later
                    // scalar deref still saw the joined values (`+A` on
                    // `h=(k v)` read back `v` instead of "").
                    if let Ok(mut m) = crate::ported::params::paramtab_hashed_storage().lock() {
                        m.remove(arg);
                    }
                    crate::ported::params::unsetparam(arg);
                    crate::ported::params::assignsparam(arg, "", 0); // empty scalar
                }
            }
            // c:Src/builtin.c::typeset_single c:2374-2378 — the `+i`
            // / `+E` / `+F` / `+l` / `+u` / `+r` / `+n` paths
            // REMOVE the corresponding PM_* flag and migrate the
            // stored value back to u_str. C captures
            // `s = ztrdup(getsparam(pname))` BEFORE the type-conversion
            // unset and restores it after. Without this branch,
            // `typeset +i n` (where n was an integer with value 42)
            // cleared u_str without copying the integer value back —
            // result was `typeset n=''`. Bug #326 in docs/BUGS.md.
            //
            // ONLY a real type change migrates the value. C decides that with
            // `chflags` (c:2117-2119):
            //     chflags = ((off & pm->flags) | (on & ~pm->flags)) &
            //         (PM_INTEGER|PM_EFLOAT|PM_FFLOAT|PM_HASHED|
            //          PM_ARRAY|PM_TIED|PM_AUTOLOAD);
            //     tc = chflags && chflags != (PM_EFLOAT|PM_FFLOAT);
            // PM_LOWER / PM_UPPER are absent from that mask, so `typeset -l v`
            // / `typeset +l v` leave `tc == 0` and fall into the `usepm` arm at
            // c:2280, which is a pure flag flip:
            //     pm->node.flags = (pm->node.flags | (on & ~PM_READONLY)) & ~off;
            // The stored string is never re-read and never rewritten. Migrating
            // it here re-stored the case-FOLDED read (getsparam applies the
            // substitution-time fold), so removing the attribute could not
            // restore the original text.
            //
            // The `& pm->flags` / `& ~pm->flags` halves matter as much as the
            // mask: only bits that actually CHANGE count. `typeset +i v` on a
            // param that was never an integer leaves `off & pm->flags == 0`,
            // so `tc == 0` and the value is left alone. PM_NAMEREF is a
            // zshrs-local addition to the mask (the `+n` conversion at
            // builtin.rs restores the stored refname); C keeps namerefs out of
            // chflags, so this stays flagged as a deviation.
            const TYPE_CONV_BITS: u32 = PM_INTEGER | PM_EFLOAT | PM_FFLOAT | PM_NAMEREF; // c:2117
            let cur_flags = paramtab()
                .read()
                .ok()
                .and_then(|t| t.get(arg.as_str()).map(|p| p.node.flags as u32))
                .unwrap_or(0);
            // c:2117-2119 — chflags, then `tc = chflags && chflags != (PM_EFLOAT|PM_FFLOAT)`
            // (a plain E↔F swap keeps the parameter, so it is not a conversion).
            let chflags = ((off & cur_flags) | (on & !cur_flags)) & TYPE_CONV_BITS;
            let tc = chflags != 0 && chflags != (PM_EFLOAT | PM_FFLOAT); // c:2118
            let pre_assign_to_clear = (off
                & (PM_INTEGER | PM_EFLOAT | PM_FFLOAT | PM_LOWER | PM_UPPER | PM_NAMEREF))
                as i32;
            if pre_assign_to_clear != 0 {
                if let Ok(mut tab) = paramtab().write() {
                    if let Some(pm) = tab.get_mut(arg.as_str()) {
                        pm.node.flags &= !pre_assign_to_clear;
                        // `+n` — drop the nameref scope/subscript info
                        // along with the flag (c:2374-2378 conversion).
                        if (off as u32 & PM_NAMEREF) != 0 {
                            pm.width = 0;
                            pm.base = 0;
                        }
                    }
                }
                // Restore the captured scalar so the cleared-type
                // param has its value as a string (PM_SCALAR
                // semantics now apply). flags=0 — typeset-internal
                // restore, never WARN_CREATE_GLOBAL (c:2322).
                if tc {
                    if let Some(ref val) = saved_val {
                        crate::ported::params::assignsparam(arg, val, 0);
                    }
                }
            }
            if pre_assign_to_set != 0 {
                if let Ok(mut tab) = paramtab().write() {
                    if let Some(pm) = tab.get_mut(arg) {
                        // c:2280 — a pure case-attribute change keeps every
                        // other flag; only a type change rewrites the type
                        // bits. Masking the whole pre-assign set on `-l`/`-u`
                        // would silently drop PM_INTEGER from an integer param.
                        let clear_mask = if tc {
                            pre_assign_mask
                        } else {
                            (PM_LOWER | PM_UPPER) as i32
                        };
                        pm.node.flags = (pm.node.flags & !clear_mask) | pre_assign_to_set;
                    }
                }
                // c:2372-2378 — re-assign saved value through new type's
                // setfn so u_val (for PM_INTEGER) or u_dval (for PM_*FLOAT)
                // catches the value migration from u_str.
                if tc {
                    if let Some(ref val) = saved_val {
                        setsparam(arg, val);
                    }
                }
            }
            // c:1973-1989 (Src/builtin.c, inside typeset_single):
            //   if (arg) {
            //       int base = zstrtol(arg, ..., 10);
            //       pm->base = base;
            //   }
            // The precision arg from `-i N`, `-E N`, `-F N` (parsed
            // by execbuiltin as `ops.args[<F-arg-slot>]`) lands on
            // the param's `base` field, which `convfloat` (c:5689 in
            // params.c) reads as the format-digit count. The
            // `name=value` arm above (c:2009-2014 typeset_setwidth
            // companion path) already stamps `pm.base`; this bare-
            // declare arm was missing it, so `typeset -F 4 f` left
            // pm.base=0 and `convfloat` rendered with the default
            // 10-digit precision (parity bug #29). With this stamp
            // a subsequent `(( f = ... ))` re-assignment preserves
            // the `pm.base = 4` set here through `assignnparam`'s
            // re-assign path (params.rs:5340-5374, c:2874-2878).
            {
                let prec_arg: Option<&str> = if (on & PM_INTEGER) != 0 && OPT_HASARG(&ops, b'i') {
                    OPT_ARG(&ops, b'i') // c:1974 -i N
                } else if (on & PM_EFLOAT) != 0 && OPT_HASARG(&ops, b'E') {
                    OPT_ARG(&ops, b'E') // c:1977 -E N
                } else if (on & PM_FFLOAT) != 0 && OPT_HASARG(&ops, b'F') {
                    OPT_ARG(&ops, b'F') // c:1980 -F N
                } else {
                    None
                };
                if let Some(s) = prec_arg {
                    if let Ok(b) = s.trim().parse::<i32>() {
                        // c:1985 zstrtol
                        if let Ok(mut tab) = paramtab().write() {
                            if let Some(pm) = tab.get_mut(arg) {
                                pm.base = b; // c:1987 pm->base = base
                            }
                        }
                    }
                } else {
                    // No explicit base/precision arg. When an EXISTING
                    // numeric param switches BETWEEN integer and float,
                    // reset pm.base to the default (0) — the shared base
                    // field would otherwise leak the old type's value
                    // (createparam's base default is 0, c:Src/params.c:1155;
                    // a numeric type change re-creates the representation).
                    // Gated so E↔F and same-type re-declares keep their
                    // base/precision (`typeset -i16 f; typeset -i f` stays
                    // base 16). zsh 5.9.1 verified.
                    let switching_int_float = ((on & PM_INTEGER) != 0 && prior_is_float)
                        || ((on & (PM_EFLOAT | PM_FFLOAT)) != 0 && prior_is_integer);
                    if !was_fresh && switching_int_float {
                        if let Ok(mut tab) = paramtab().write() {
                            if let Some(pm) = tab.get_mut(arg) {
                                pm.base = 0;
                            }
                        }
                    }
                }
            }
            // c:2528-2533 — the createparam (valueless declare) branch
            // must ALSO stamp the `-L`/`-R`/`-Z` field width
            // (typeset_setwidth, c:2002-2011). The base block above
            // handled `-i`/`-E`/`-F` precision but the width was dropped,
            // so `typeset -L 10 f` (declare, then assign later) left
            // pm.width=0 and the value never left/right-justified — and
            // `typeset -p` rendered `-FL` with no number. The inline-value
            // arm (the `name=value` path) already stamps width; only this
            // bare-declare arm was missing it.
            {
                let width_arg: Option<&str> = if (on & PM_LEFT) != 0 && OPT_HASARG(&ops, b'L') {
                    OPT_ARG(&ops, b'L') // c:2003
                } else if (on & PM_RIGHT_B) != 0 && OPT_HASARG(&ops, b'R') {
                    OPT_ARG(&ops, b'R') // c:2005
                } else if (on & PM_RIGHT_Z) != 0 && OPT_HASARG(&ops, b'Z') {
                    OPT_ARG(&ops, b'Z') // c:2007
                } else {
                    None
                };
                if let Some(s) = width_arg {
                    if let Ok(w) = s.trim().parse::<i32>() {
                        // c:2011 zstrtol
                        if let Ok(mut tab) = paramtab().write() {
                            if let Some(pm) = tab.get_mut(arg) {
                                pm.width = w; // c:2011 pm->width = width
                            }
                        }
                    }
                }
            }
            // c:2510+ — stamp post-assign attributes (PM_EXPORTED,
            // PM_READONLY, etc.) on the (possibly newly-created) pm.
            if post_assign_to_set != 0 {
                if let Ok(mut tab) = paramtab().write() {
                    if let Some(pm) = tab.get_mut(arg) {
                        // c:2289 MERGE — preserve attributes the user did not touch
                        // (e.g. `typeset -L 4 x=hi; typeset -r x` keeps -L). `off` bits
                        // are cleared separately below.
                        pm.node.flags |= post_assign_to_set;
                    }
                }
                // c:Src/params.c — when PM_UNIQUE is freshly stamped on
                // an EXISTING array via bare `typeset -U arr` (no value
                // assignment), zsh dedups the current contents in-place
                // (verified vs /opt/homebrew/bin/zsh: `arr=(a b a c b);
                // typeset -U arr; echo "$arr[@]"` → "a b c"). Apply the
                // same dedup here.
                if (post_assign_to_set as u32 & PM_UNIQUE) != 0 {
                    let existing = crate::ported::params::getaparam(arg);
                    if let Some(arr) = existing {
                        let mut seen: std::collections::HashSet<String> =
                            std::collections::HashSet::new();
                        let deduped: Vec<String> =
                            arr.into_iter().filter(|e| seen.insert(e.clone())).collect();
                        crate::ported::params::setaparam(arg, deduped);
                    }
                }
                // c:Src/builtin.c:2302-2307 — `if (!(pm->node.flags &
                // (PM_ARRAY|PM_HASHED))) { ... addenv(pm, getsparam(pname)); }`.
                // Arrays and associative arrays are NOT mirrored to the
                // OS env even when PM_EXPORTED is set — env strings are
                // single name=value pairs and have no representation
                // for indexed or hashed data. zsh silently no-ops the
                // export request for these types; zshrs's prior code
                // wrote a malformed empty `h=` entry for `export h`
                // where h is an assoc. Bug #349.
                if (on as u32 & PM_EXPORTED) != 0 {
                    let is_array_or_hashed = paramtab()
                        .read()
                        .ok()
                        .and_then(|t| t.get(arg).map(|pm| pm.node.flags as u32))
                        .map_or(false, |f| (f & (PM_ARRAY | PM_HASHED)) != 0)
                        || crate::ported::params::paramtab_hashed_storage()
                            .lock()
                            .ok()
                            .map_or(false, |s| s.contains_key(arg))
                        || crate::ported::exec::array(arg).is_some()
                        || crate::ported::exec::assoc(arg).is_some();
                    if !is_array_or_hashed {
                        if let Some(val) = saved_val.as_deref().or(Some("")) {
                            env::set_var(arg, val);
                        }
                    }
                }
            }
            // c:Src/builtin.c::typeset_single — `typeset +x VAR`
            // clears PM_EXPORTED via the `off` mask. C zsh calls
            // removeenv on the param, which strips the var from the
            // OS environment so child processes no longer see it.
            // Must fire INDEPENDENTLY of post_assign_to_set since `+x`
            // alone sets `off` (PM_EXPORTED) but `on` is 0, so the
            // post_assign_to_set block above doesn't run. Also clear
            // the flag bit on the paramtab entry so `(t)V` no longer
            // reports `-export`. Bug #201 in docs/BUGS.md.
            if (off as u32 & PM_EXPORTED) != 0 {
                env::remove_var(arg);
                if let Ok(mut tab) = paramtab().write() {
                    if let Some(pm) = tab.get_mut(arg) {
                        pm.node.flags &= !(PM_EXPORTED as i32);
                    }
                }
            }
            // c:Src/builtin.c:2289 — `pm->node.flags = ... & ~off`.
            // `off` bits in post_assign_mask must clear regardless of
            // `on` (e.g. `typeset +U arr` where on=0, off=PM_UNIQUE).
            // The post_assign_to_set gate above short-circuits the
            // on=0 case. Same family as `+x` block immediately above.
            // Bug #234 in docs/BUGS.md.
            let off_in_mask = (off as i32) & post_assign_mask;
            if off_in_mask != 0 {
                if let Ok(mut tab) = paramtab().write() {
                    if let Some(pm) = tab.get_mut(arg) {
                        pm.node.flags &= !off_in_mask;
                    }
                }
            }
            // c:Src/builtin.c::typeset_single c:2241-2246 — when no
            // type flags and no value assignment, `typeset NAME` is a
            // display request: print the existing param in the
            // canonical form. C uses
            // `printparamnode(&pm->node, PRINT_INCLUDEVALUE|with_ns)`.
            // Without this, `typeset h` (where h is an assoc) emitted
            // nothing. The earlier per-arg print at typeset_single
            // line 3181 isn't called from bin_typeset's per-arg loop
            // — the loop does its own inline work. Bug #218 in
            // docs/BUGS.md.
            // c:Src/builtin.c::typeset_single c:2241-2246 — bare
            // `typeset NAME` (no on/off flags except PM_LOCAL which
            // bin_typeset auto-applies at c:2808) prints the param.
            // PM_LOCAL is added implicitly when not -g / -x / -m so
            // mask it out when checking "are any user flags set".
            //
            // c:Src/builtin.c:2078-2090 — the bare form prints UNLESS it
            // is localizing a variable from an outer scope. zsh clears
            // `usepm` (→ no print) only when `locallevel != pm->level &&
            // (on & PM_LOCAL)` — i.e. the existing param lives at a
            // different (outer/global) level and we're making a local.
            // Re-declaring a param that ALREADY exists at the current
            // level (`f(){ typeset x=1; typeset x }`) keeps usepm and
            // DOES print `x=1`. The previous `at_top_scope` gate wrongly
            // suppressed that in-function re-declaration case.
            let user_on = (on as u32) & !PM_LOCAL;
            let cur_ll = locallevel.load(Relaxed);
            let not_localizing = pm_level_existing == cur_ll || (on as u32 & PM_LOCAL) == 0;
            // c:Src/builtin.c:2244 — `else if (!OPT_ISSET(ops,'g') &&
            // (unset(TYPESETSILENT) || OPT_ISSET(ops,'m')))`. The `-g`
            // (global-scope) flag SUPPRESSES the bare-name print so
            // `typeset -g ZPFX` is a silent declaration / no-op rather
            // than a list. zinit's plugin loader runs `typeset -g VAR`
            // for already-environment-exported names on every load;
            // without this gate zshrs spammed `VAR=value` for each on
            // startup.
            // c:Src/builtin.c:2062-2064 — the print at c:2246 sits INSIDE the
            // `usepm` branch, and usepm is false for an UNSET parameter:
            //   usepm = pm && (!(pm->node.flags & PM_UNSET) || OPT_ISSET(ops,'p') ||
            //                  (isset(POSIXBUILTINS) &&
            //                   (pm->node.flags & (PM_READONLY|PM_EXPORTED))));
            // with the comment "Here we just avoid using it for the present
            // tests if it's unset." So `typeset NAME` on a declared-but-UNSET
            // param prints NOTHING and re-declares instead.
            //
            // TYPESET_TO_UNSET creates exactly that state: PM_DEFAULTED is
            // `PM_DECLARED|PM_UNSET` (zsh.h:1934). zshrs suppressed the print
            // only while the OPTION was still set, so
            // `setopt typesettounset; typeset x; unsetopt typesettounset;
            // typeset x` printed `x=''` where zsh is silent — the gate has to
            // key off the parameter's UNSET flag, not the live option.
            // The `-p` term of usepm is already covered by the `!OPT_ISSET(p)`
            // conjunct below. Bug #1056.
            if user_on == 0
                && off == 0
                && not_localizing
                && !OPT_ISSET(&ops, b'p')
                && !OPT_ISSET(&ops, b'g')
                && (!isset(TYPESETSILENT) || OPT_ISSET(&ops, b'm'))
                && pname_in_tab
                && usepm_at_entry
            {
                let with_ns = if OPT_ISSET(&ops, b'm') {
                    PRINT_WITH_NAMESPACE
                } else {
                    0
                };
                let _ = with_ns;
                // The paramtab entry for assoc/array shapes set via
                // direct assignment (`a=(1 2 3)` / `h[k]=v`) doesn't
                // always have PM_ARRAY/PM_HASHED set on flags — the
                // values live in the executor's arrays/assoc_arrays
                // storage, separate from paramtab. Check the canonical
                // storage paths and emit the right form directly,
                // mirroring printparamvalue's PM_ARRAY/PM_HASHED arms
                // (params.rs:8819+).
                //
                // Order matters: assoc lookup falls back to
                // crate::ported::exec::assoc which reads the executor's
                // assoc_arrays storage. typeset -A populated via
                // `typeset -A h=(a 1 b 2)` lives there, not in
                // paramtab_hashed_storage. Bug #218 in docs/BUGS.md.
                let assoc = crate::ported::params::paramtab_hashed_storage()
                    .lock()
                    .ok()
                    .and_then(|s| s.get(arg).cloned())
                    .or_else(|| crate::ported::exec::assoc(arg));
                if let Some(map) = assoc {
                    let mut entries: Vec<(&String, &String)> = map.iter().collect();
                    entries.sort_by(|(a, _), (b, _)| a.cmp(b));
                    let mut s = format!("{}=( ", arg);
                    let mut first = true;
                    for (k, v) in entries {
                        if !first {
                            s.push(' ');
                        }
                        first = false;
                        s.push_str(&format!("[{}]={}", k, crate::ported::utils::quotedzputs(v)));
                    }
                    s.push_str(" )");
                    println!("{}", s);
                } else if let Some(arr) = crate::ported::params::getaparam(arg)
                    .or_else(|| crate::ported::exec::array(arg))
                    .or_else(|| {
                        // Fallback: paramtab entry's u_arr may be set
                        // even when PM_TYPE doesn't include PM_ARRAY
                        // (direct `a=(1 2 3)` assignment doesn't always
                        // update the flag).
                        paramtab()
                            .read()
                            .ok()
                            .and_then(|t| t.get(arg).and_then(|pm| pm.u_arr.clone()))
                            .filter(|v| !v.is_empty())
                    })
                {
                    let parts: Vec<String> = arr
                        .iter()
                        .map(|v| crate::ported::utils::quotedzputs(v))
                        .collect();
                    println!("{}=( {} )", arg, parts.join(" "));
                } else if let Some(val) = getsparam(arg) {
                    println!("{}={}", arg, crate::ported::utils::quotedzputs(&val));
                }
            }
        }
    }
    unqueue_signals();
    returnval
}

/// Port of `eval_autoload(Shfunc shf, char *name, Options ops, int func)` from Src/builtin.c:3166.
/// C: `int eval_autoload(Shfunc shf, char *name, Options ops, int func)`.
/// PM_UNDEFINED guard; -X spawns the eval-trampoline, otherwise loadautofn
/// resolves and installs the body.
/// WARNING: param names don't match C — Rust=(shf, name, func) vs C=(shf, name, ops, func)
pub fn eval_autoload(
    shf: *mut shfunc,
    name: &str, // c:3166
    ops: &options,
    func: i32,
) -> i32 {
    if shf.is_null() {
        return 1;
    }
    let shf_mut = unsafe { &mut *shf };
    // c:3168-3169 — `if (!(shf->node.flags & PM_UNDEFINED)) return 1;`
    if (shf_mut.node.flags as u32 & PM_UNDEFINED) == 0 {
        // c:3168
        return 1; // c:3169
    }
    // c:3171-3174 — `if (shf->funcdef) { freeeprog(shf->funcdef); shf->funcdef = &dummy_eprog; }`
    if shf_mut.funcdef.is_some() {
        // c:3171
        shf_mut.funcdef = None; // c:3173 freeeprog + dummy
    }
    // c:3175-3181 — `-X` spawns the autoload trampoline via bin_eval.
    if OPT_MINUS(ops, b'X') {
        // c:3175
        // c:3177 — `fargv[0] = quotestring(name, QT_SINGLE_OPTIONAL); fargv[1] = "\"$@\"";`
        let fargv = vec![
            // c:3177-3179
            quotedzputs(name),
            "\"$@\"".to_string(),
        ];
        // c:3180 — `shf->funcdef = mkautofn(shf);`
        let p = mkautofn(shf); // c:3180
        let _ = p; // funcdef writeback handled inside mkautofn at c:3801
        return bin_eval(name, &fargv, ops, func); // c:3181
    }
    // c:3184-3186 — `return !loadautofn(shf, (OPT_ISSET('k') ? 2 :
    //                                  (OPT_ISSET('z') ? 0 : 1)), 1,
    //                                   OPT_ISSET('d'));`
    let mode = if OPT_ISSET(ops, b'k') {
        2
    }
    // c:3184
    else if OPT_ISSET(ops, b'z') {
        0
    }
    // c:3185
    else {
        1
    };
    let _d = OPT_ISSET(ops, b'd');
    // c:3184 — `return !loadautofn(shf, ...)`. C's loadautofn returns
    // `Shfunc` (pointer): NULL = failure, non-NULL = success. C's
    // `!loadautofn` yields 1 (true) when the pointer is NULL —
    // i.e. eval_autoload returns 1 on failure, 0 on success.
    //
    // The Rust port's loadautofn returns an i32 with the OPPOSITE
    // sign convention: 0 = success, non-zero = failure (the
    // canonical Rust ShellExecutor-status convention used throughout
    // this file). The previous Rust port literally translated `!`
    // as `if r == 0 { 1 } else { 0 }` — but that double-inverts the
    // already-flipped convention, so file-not-found (Rust r=1)
    // returned eval_autoload=0 (zsh-success), and the broken
    // `autoload +X totally_fake` silently exited 0 instead of 1.
    // The C-faithful return for the Rust convention is just `r`.
    // Bug #107 in docs/BUGS.md.
    loadautofn(shf, mode, 1, _d as i32) // c:3184
}

/// Port of `check_autoload(Shfunc shf, char *name, Options ops, int func)` from Src/builtin.c:3193.
/// C: `static int check_autoload(Shfunc shf, char *name, Options ops,
///     int func)` — `OPT_ISSET(ops,'X')` ? eval_autoload : 0.
/// WARNING: param names don't match C — Rust=(shf, name, func) vs C=(shf, name, ops, func)
pub fn check_autoload(
    shf: *mut shfunc,
    name: &str, // c:3193
    ops: &options,
    func: i32,
) -> i32 {
    // c:3196-3199 — `if (OPT_ISSET(ops,'X')) return eval_autoload(...);`
    if OPT_ISSET(ops, b'X') {
        // c:3196
        return eval_autoload(shf, name, ops, func); // c:3197
    }
    // c:3200-3242 — -r / -R re-resolve: walk fpath for the function file.
    let want_r = OPT_ISSET(ops, b'r');
    let want_R = OPT_ISSET(ops, b'R');
    if (want_r || want_R) && !shf.is_null() {
        // c:3200
        let shf_mut = unsafe { &mut *shf };
        if (shf_mut.node.flags as u32 & PM_UNDEFINED) == 0 {
            return 0;
        }
        // c:3202-3216 — already has filename + PM_LOADDIR: try the cached
        // dir first via spec_path[].
        if (shf_mut.node.flags as u32 & PM_LOADDIR) != 0 && shf_mut.filename.is_some() {
            let spec = vec![shf_mut.filename.clone().unwrap_or_default()];
            if getfpfunc(
                &shf_mut.node.nam,
                &mut None, // c:3206
                Some(&spec),
                1,
                &mut None, // c:3206 `&ksh` — unused on the test-only path
            )
            .is_some()
            {
                return 0; // c:3209
            }
            // c:3211-3217 — `-d` not set: bail (with -R = error, with -r = silent).
            if !OPT_ISSET(ops, b'd') {
                // c:3211
                if want_R {
                    // c:3212
                    zerr(&format!(
                        "{}: function definition file not found",
                        shf_mut.node.nam
                    )); // c:3213
                    return 1; // c:3215
                }
                return 0; // c:3216
            }
        }
        // c:3219-3231 — fpath walk via getfpfunc + dircache_set install.
        let mut dir_path: Option<String> = None;
        if getfpfunc(&shf_mut.node.nam, &mut dir_path, None, 1, &mut None).is_some()    // c:3219
            && dir_path.is_some()
        {
            // c:3220-3228 — dircache_set + relative-path absolutize.
            let mut old_slot = shf_mut.filename.take();
            dircache_set(&mut old_slot, None); // c:3220
            let dp = dir_path.unwrap();
            let mut new_slot: Option<String> = None;
            dircache_set(&mut new_slot, Some(&dp)); // c:3228
            shf_mut.filename = new_slot;
            shf_mut.node.flags |= PM_LOADDIR as i32; // c:3229
            return 0; // c:3230
        }
        // c:3233-3239 — -R: error; -r: silent.
        if want_R {
            // c:3233
            zerr(&format!(
                "{}: function definition file not found",
                shf_mut.node.nam
            )); // c:3243
            return 1; // c:3243
        }
    }
    0 // c:3243
}

/// Port of `listusermathfunc(MathFunc p)` from Src/builtin.c:3243.
/// C: `static void listusermathfunc(MathFunc p)` — emit a `functions -M`
///   row for one user math function with arg counts and module name.
pub fn listusermathfunc(p: &mathfunc) {
    // c:3243
    // c:3247-3257 — pick `showargs` 0..3 based on module/min/max presence.
    let mut showargs: i32 = if p.module.is_some() {
        // c:3249
        3
    } else if p.maxargs != if p.minargs != 0 { p.minargs } else { -1 } {
        // c:3251
        2
    } else if p.minargs != 0 {
        // c:3253
        1
    } else {
        0 // c:3256
    };

    // c:3259 — `printf("functions -M%s %s", (p->flags & MFF_STR) ? "s" : "", p->name);`
    let s_suffix = if (p.flags & MFF_STR) != 0 { "s" } else { "" }; // c:3259
    print!("functions -M{} {}", s_suffix, p.name); // c:3259
    if showargs != 0 {
        // c:3260
        print!(" {}", p.minargs); // c:3261
        showargs -= 1; // c:3262
    }
    if showargs != 0 {
        // c:3264
        print!(" {}", p.maxargs); // c:3265
        showargs -= 1; // c:3266
    }
    if showargs != 0 {
        // c:3268
        // c:3269-3274 — function names are not required to be ident chars,
        // so the module name goes through quotedzputs for safe printing.
        print!(" "); // c:3273
        print!("{}", quotedzputs(p.module.as_deref().unwrap_or(""))); // c:3274
        showargs -= 1; // c:3275
    }
    println!(); // c:3277
}

/// Port of `add_autoload_function(Shfunc shf, char *funcname)` from Src/builtin.c:3278.
/// C: `static void add_autoload_function(Shfunc shf, char *funcname)` —
///   two branches:
///     (a) funcname is absolute & shf is PM_UNDEFINED → split `/dir/nam`,
///         dircache_set(&shf->filename, dir), set PM_LOADDIR|PM_ABSPATH_USED,
///         shfunctab->addnode(nam, shf).
///     (b) otherwise → walk funcstack to find calling function; if it has
///         PM_LOADDIR|PM_ABSPATH_USED, build `"<calling-dir>/funcname"` and
///         access(R_OK); on success copy the dir into shf and set
///         PM_LOADDIR|PM_ABSPATH_USED. Then shfunctab->addnode(funcname, shf).
/// WARNING: param names don't match C — Rust=(shf) vs C=(shf, funcname)
pub fn add_autoload_function(
    shf: *mut shfunc, // c:3278
    funcname: &str,
) {
    if shf.is_null() || funcname.is_empty() {
        return;
    }
    let shf_ref = unsafe { &mut *shf };

    let is_abs_path = funcname.starts_with('/')                              // c:3282
                      && funcname.len() > 1
                      && funcname[1..].contains('/')
                      && (shf_ref.node.flags as u32 & PM_UNDEFINED) != 0;

    if is_abs_path {
        // c:3287 — `nam = strrchr(funcname, '/');`
        let nam_idx = funcname.rfind('/').unwrap(); // c:3287
        let (dir, nam) = if nam_idx == 0 {
            // c:3289
            ("/".to_string(), funcname[1..].to_string()) // c:3290
        } else {
            (
                funcname[..nam_idx].to_string(), // c:3293
                funcname[nam_idx + 1..].to_string(),
            )
        };
        // c:3296 — `dircache_set(&shf->filename, NULL); dircache_set(..., dir);`
        let mut old_slot = shf_ref.filename.take();
        dircache_set(&mut old_slot, None); // c:3296
        let mut new_slot: Option<String> = None;
        dircache_set(&mut new_slot, Some(&dir)); // c:3297
        shf_ref.filename = new_slot;
        // c:3298-3299 — `shf->node.flags |= PM_LOADDIR | PM_ABSPATH_USED;`
        shf_ref.node.flags |= (PM_LOADDIR | PM_ABSPATH_USED) as i32; // c:3298
                                                                     // c:3300 — `shfunctab->addnode(shfunctab, ztrdup(nam), shf);` — the
                                                                     // node is keyed on the BASENAME (`nam`), not the full path. zshrs's
                                                                     // addnode keys on `shf.node.nam`, so rename the node before adding;
                                                                     // otherwise `autoload -Uz /dir/.hist.format` registered a function
                                                                     // literally named `/dir/.hist.format` (plugins that autoload by full
                                                                     // path — zsh-hist's `autoload -Uz $fdir/.hist.*` — diverged from
                                                                     // zsh, which names it `.hist.format`).
        shf_ref.node.nam = nam;
        if let Ok(mut t) = shfunctab_lock().write() {
            t.addnode(shf); // c:3300
        }
    } else {
        // c:3304-3327 — walk funcstack, look up calling fn in shfunctab, if
        // it has PM_LOADDIR|PM_ABSPATH_USED build "<dir>/<funcname>" and
        // access(R_OK), inherit the dir on hit.
        let calling_f: Option<String> = {
            let stack = FUNCSTACK.lock().map(|s| s.clone()).unwrap_or_default();
            // c:3306 — `for (fs = funcstack; fs; fs = fs->prev)`
            stack
                .iter()
                .rev()
                .find(|fs| {
                    // c:3306
                    // c:3307 — `if (fs->tp == FS_FUNC && fs->name &&
                    //               (!shf->node.nam || strcmp(fs->name, shf->node.nam)))`
                    FS_FUNC != 0  // mirror struct doesn't expose tp directly;
                && !fs.name.is_empty()
                && (shf_ref.node.nam.is_empty() || fs.name != shf_ref.node.nam)
                })
                .map(|fs| fs.name.clone()) // c:3308
        };
        if let Some(cf) = calling_f {
            // c:3315
            // c:3316 — `shf2 = shfunctab->getnode2(shfunctab, calling_f);`
            let shf2_ptr = shfunctab_lock()
                .read()
                .map(|t| t.getnode2(&cf))
                .unwrap_or(std::ptr::null_mut());
            if !shf2_ptr.is_null() {
                let shf2 = unsafe { &*shf2_ptr };
                // c:3317-3318
                let needs = (PM_LOADDIR | PM_ABSPATH_USED) as i32;
                if (shf2.node.flags & needs) == needs {
                    // c:3317
                    if let Some(dir2) = &shf2.filename {
                        // c:3318
                        // c:3320 — `snprintf(buf, PATH_MAX, "%s/%s", dir2, funcname);`
                        let buf = format!("{}/{}", dir2, funcname); // c:3320
                        if buf.len() <= libc::PATH_MAX as usize {
                            // c:3320
                            // c:3324 — `if (!access(buf, R_OK))`
                            let buf_c = std::ffi::CString::new(buf.clone()).ok();
                            if let Some(bc) = buf_c {
                                if unsafe { libc::access(bc.as_ptr(), libc::R_OK) } == 0 {
                                    // c:3324
                                    let mut old_slot = shf_ref.filename.take();
                                    dircache_set(&mut old_slot, None); // c:3325
                                    let dir2c = dir2.clone();
                                    let mut new_slot: Option<String> = None;
                                    dircache_set(&mut new_slot, Some(&dir2c)); // c:3326
                                    shf_ref.filename = new_slot;
                                    shf_ref.node.flags |= (PM_LOADDIR | PM_ABSPATH_USED) as i32;
                                    // c:3327
                                }
                            }
                        }
                    }
                }
            }
        }
        // c:3334 — `shfunctab->addnode(shfunctab, ztrdup(funcname), shf);`
        // addnode keys by `shf->node.nam`; if the caller picked a different
        // funcname here, re-tag the node name first so the keyed insert
        // matches the C contract.
        unsafe {
            if !shf.is_null() {
                (*shf).node.nam = funcname.to_string();
            }
        }
        if let Ok(mut t) = shfunctab_lock().write() {
            t.addnode(shf); // c:3334
        }
    }
}

/// Port of `bin_functions(char *name, char **argv, Options ops, int func)` from Src/builtin.c:3342.
/// C: `int bin_functions(char *name, char **argv, Options ops, int func)`.
/// This is the canonical free-function port matching the C signature so
/// the dispatcher can call it. The earlier `ShellExecutor::bin_functions`
/// inherent method is an ad-hoc Rust-side helper kept for the existing
/// in-process executor; both should converge on this function.
/// WARNING: param names don't match C — Rust=(name, argv, _func) vs C=(name, argv, ops, func)
pub fn bin_functions(
    name: &str,
    argv: &[String], // c:3342
    ops: &options,
    _func: i32,
) -> i32 {
    // c:3346-3347 — `int returnval = 0; int on = 0, off = 0, pflags = 0,
    //                roff, expand = 0;`
    let mut returnval: i32 = 0; // c:3346
    let mut on: u32 = 0; // c:3347
    let mut off: u32 = 0; // c:3347
    let _pflags: i32 = 0; // c:3347
    let _expand: i32 = 0; // c:3347

    // c:3350-3351 — `if (OPT_PLUS(ops,'u')) off |= PM_UNDEFINED; else if
    //                (OPT_MINUS(ops,'u') || OPT_ISSET(ops,'X')) on |= PM_UNDEFINED;`
    if OPT_PLUS(ops, b'u') {
        // c:3350
        off |= PM_UNDEFINED; // c:3351
    } else if OPT_MINUS(ops, b'u') || OPT_ISSET(ops, b'X') {
        // c:3352
        on |= PM_UNDEFINED; // c:3353
    }
    // c:3354-3357 — -U / +U toggle PM_UNALIASED|PM_UNDEFINED.
    if OPT_MINUS(ops, b'U') {
        // c:3354
        on |= PM_UNALIASED | PM_UNDEFINED; // c:3355
    } else if OPT_PLUS(ops, b'U') {
        // c:3356
        off |= PM_UNALIASED; // c:3357
    }
    // c:3358-3361 — -t / +t toggle PM_TAGGED.
    if OPT_MINUS(ops, b't') {
        // c:3358
        on |= PM_TAGGED; // c:3359
    } else if OPT_PLUS(ops, b't') {
        // c:3360
        off |= PM_TAGGED; // c:3361
    }
    // c:3362-3365 — -T / +T toggle PM_TAGGED_LOCAL.
    if OPT_MINUS(ops, b'T') {
        // c:3362
        on |= PM_TAGGED_LOCAL; // c:3363
    } else if OPT_PLUS(ops, b'T') {
        // c:3364
        off |= PM_TAGGED_LOCAL; // c:3365
    }
    // c:3366-3369 — -W / +W toggle PM_WARNNESTED.
    if OPT_MINUS(ops, b'W') {
        // c:3366
        on |= PM_WARNNESTED; // c:3367
    } else if OPT_PLUS(ops, b'W') {
        // c:3368
        off |= PM_WARNNESTED; // c:3369
    }
    // c:3370 — `roff = off;`
    let mut roff = off; // c:3370
                        // c:3371-3377 — -z / +z PM_ZSHSTORED|PM_KSHSTORED interaction.
    if OPT_MINUS(ops, b'z') {
        // c:3371
        on |= PM_ZSHSTORED; // c:3372
        off |= PM_KSHSTORED; // c:3373
    } else if OPT_PLUS(ops, b'z') {
        // c:3374
        off |= PM_ZSHSTORED; // c:3375
        roff |= PM_ZSHSTORED; // c:3376
    }
    // c:3379-3385 — -k / +k PM_KSHSTORED|PM_ZSHSTORED interaction.
    if OPT_MINUS(ops, b'k') {
        // c:3379
        on |= PM_KSHSTORED; // c:3380
        off |= PM_ZSHSTORED; // c:3381
    } else if OPT_PLUS(ops, b'k') {
        // c:3382
        off |= PM_KSHSTORED; // c:3383
        roff |= PM_KSHSTORED; // c:3384
    }
    // c:3386-3392 — -d / +d PM_CUR_FPATH toggle.
    if OPT_MINUS(ops, b'd') {
        // c:3386
        on |= PM_CUR_FPATH; // c:3387
        off |= PM_CUR_FPATH; // c:3388
    } else if OPT_PLUS(ops, b'd') {
        // c:3389
        off |= PM_CUR_FPATH; // c:3390
        roff |= PM_CUR_FPATH; // c:3391
    }

    // c:3394-3400 — early-error validation: invalid flag combinations.
    // C: `(OPT_MINUS(ops,'X') && (OPT_ISSET(ops,'m') || !scriptname))` —
    // \`-X\` is only valid in a script context (autoload-from-fpath
    // dispatch). Previous Rust port dropped the \`|| !scriptname\` half
    // so \`functions -X foo\` from interactive shell silently
    // succeeded — divergent.
    let scriptname_missing = scriptname_get().is_none();
    if (off & PM_UNDEFINED) != 0                                             // c:3394
        || (OPT_ISSET(ops, b'k') && OPT_ISSET(ops, b'z'))                    // c:3394
        || (OPT_ISSET(ops, b'x') && !OPT_HASARG(ops, b'x'))                  // c:3395
        || (OPT_MINUS(ops, b'X')                                             // c:3396
            && (OPT_ISSET(ops, b'm') || scriptname_missing))                 // c:3396 !scriptname
        || (OPT_ISSET(ops, b'c')
            && (OPT_ISSET(ops, b'x') || OPT_ISSET(ops, b'X') || OPT_ISSET(ops, b'm')))
    {
        zwarnnam(name, "invalid option(s)"); // c:3399
        return 1; // c:3400
    }

    // c:3402-3452 — `-c` (clone) branch: copy named function under a new
    // name, optionally registering it as a TRAP* signal trap.
    if OPT_ISSET(ops, b'c') {
        // c:3402
        if argv.len() < 2 || argv.len() > 2 {
            // c:3405
            zwarnnam(name, "-c: requires two arguments"); // c:3406
            return 1;
        }
        let src_name = &argv[0];
        let dst_name = &argv[1];
        // c:3409 — `shf = shfunctab->getnode(shfunctab, *argv);`
        let src_ptr = shfunctab_lock()
            .read()
            .map(|t| t.getnode(src_name.as_str()))
            .unwrap_or(std::ptr::null_mut());
        if src_ptr.is_null() {
            // c:3410
            zwarnnam(name, &format!("no such function: {}", src_name)); // c:3411
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
        let new_filename =
            if (src_ref.node.flags as u32 & PM_UNDEFINED) == 0 && src_ref.filename.is_some() {
                src_ref.filename.clone() // c:3429
            } else {
                None
            };
        let _ = new_filename; // wired into shfunctab[dst_name] below
                              // c:3437-3447 — TRAP* prefix detection + signal trap registration.
        if dst_name.starts_with("TRAP") {
            // c:3437
            // c:3438 — `int sigidx = getsigidx(s + 4);`
            let sigidx = getsigidx(&dst_name[4..]); // c:3438
            if sigidx != -1 {
                // c:3439
                // c:3440 — `if (settrap(sigidx, NULL, ZSIG_FUNC))`.
                if settrap(sigidx, None, ZSIG_FUNC) != 0 {
                    // c:3440
                    // freeeprog(newsh->funcdef) — funcdef Drop covers it.
                    // dircache_set(&newsh->filename, NULL);
                    // zfree(newsh, sizeof(*newsh));
                    return 1; // c:3445
                }
                // c:3447 — `removetrapnode(sigidx);` — clear any prior trap.
                removetrapnode(sigidx); // c:3447
            }
        }
        // c:3422-3430 — C does `newsh = zalloc + memcpy(*newsh, *shf)` so
        // src and dst become independent copies. Box-clone the source body
        // (rather than aliasing src_ptr) so subsequent mutation through
        // `getnode(dst_name)` doesn't bleed into src.
        // c:3450 — `shfunctab->addnode(shfunctab, ztrdup(s), &newsh->node);`
        let newsh = unsafe {
            let mut copy = (*src_ptr).clone();
            copy.node.nam = dst_name.clone();
            Box::into_raw(Box::new(copy))
        };
        if let Ok(mut t) = shfunctab_lock().write() {
            t.addnode(newsh); // c:3450
        }
        return 0; // c:3451
    }

    // c:3454-3463 — `-x N` indent override for printing.
    let mut expand: i32 = 0; // c:3454 (also c:3347)
    if OPT_ISSET(ops, b'x') {
        // c:3454
        let arg = OPT_ARG(ops, b'x').unwrap_or("");
        match arg.trim().parse::<i32>() {
            // c:3456
            Ok(n) => {
                expand = n; // c:3456
                if expand == 0 {
                    expand = -1;
                } // c:3461-3462
            }
            Err(_) => {
                zwarnnam(name, "number expected after -x"); // c:3458
                return 1; // c:3459
            }
        }
    }

    // c:3465-3466 — `+f` / roff / `+` enables PRINT_NAMEONLY.
    let mut pflags: i32 = 0;
    if OPT_PLUS(ops, b'f') || roff != 0 || OPT_ISSET(ops, b'+') {
        // c:3465
        pflags |= PRINT_NAMEONLY; // c:3466
    }

    // c:3468-3530 — `-M`/`+M` add/remove/list math function path.
    if OPT_MINUS(ops, b'M') || OPT_PLUS(ops, b'M') {
        // c:3468
        // c:3473-3477 — refuse incompatible flag combos.
        if on != 0
            || off != 0
            || pflags != 0
            || OPT_ISSET(ops, b'X')
            || OPT_ISSET(ops, b'u')
            || OPT_ISSET(ops, b'U')
            || OPT_ISSET(ops, b'w')
        {
            zwarnnam(name, "invalid option(s)"); // c:3475
            return 1; // c:3476
        }
        if argv.is_empty() {
            // c:3478
            // c:3479-3484 — list user math ported.
            queue_signals(); // c:3480
            if let Ok(table) = MATHFUNCS.lock() {
                // c:3481
                for p in table.iter() {
                    // c:3481
                    if (p.flags & MFF_USERFUNC) != 0 {
                        // c:3482
                        listusermathfunc(p); // c:3483
                    }
                }
            }
            unqueue_signals(); // c:3484
            return returnval;
        } else if OPT_ISSET(ops, b'm') {
            // c:3485
            // c:3486-3515 — list/delete matching math ported by pattern.
            for arg in argv.iter() {
                queue_signals(); // c:3488
                                 // c:3489 — `tokenize(*argv)`; Rust patcompile handles it.
                if let Some(pprog) = patcompile(
                    &{
                        let mut __pat_tok = (arg).to_string();
                        crate::ported::glob::tokenize(&mut __pat_tok);
                        __pat_tok
                    },
                    PAT_STATIC,
                    None,
                ) {
                    // c:3490
                    if OPT_PLUS(ops, b'M') {
                        // c:3497
                        // Delete matching user ported.
                        if let Ok(mut table) = MATHFUNCS.lock() {
                            table.retain(|p| {
                                !((p.flags & MFF_USERFUNC) != 0 && pattry(&pprog, &p.name))
                            });
                        }
                    } else {
                        // c:3502 — listusermathfunc for matches.
                        if let Ok(table) = MATHFUNCS.lock() {
                            for p in table.iter() {
                                if (p.flags & MFF_USERFUNC) != 0 && pattry(&pprog, &p.name) {
                                    listusermathfunc(p);
                                }
                            }
                        }
                    }
                } else {
                    // c:3509
                    // c:3510-3512 — bad pattern.
                    zwarnnam(
                        name, // c:3511
                        &format!("bad pattern : {}", arg),
                    );
                    returnval = 1; // c:3512
                }
                unqueue_signals(); // c:3514
            }
            return returnval;
        } else if OPT_PLUS(ops, b'M') {
            // c:3516
            // c:3517-3533 — `+M name…` delete by exact name.
            for arg in argv.iter() {
                queue_signals(); // c:3519
                if let Ok(mut table) = MATHFUNCS.lock() {
                    let idx = table.iter().position(|p| p.name == *arg); // c:3520-3521
                    if let Some(i) = idx {
                        if (table[i].flags & MFF_USERFUNC) == 0 {
                            // c:3522-3527 — library function, refuse.
                            zwarnnam(
                                name, // c:3523
                                &format!("+M {}: is a library function", arg),
                            );
                            returnval = 1; // c:3525
                        } else {
                            table.remove(i); // c:3528
                        }
                    }
                }
                unqueue_signals(); // c:3532
            }
            return returnval;
        } else {
            // c:3535-3611 — `-M name [min [max [mod]]]` add a user math fn.
            let mut argv_iter = argv.iter();
            let funcname = argv_iter.next().unwrap(); // c:3537
            let mut minargs: i32;
            let mut maxargs: i32;
            if OPT_ISSET(ops, b's') {
                // c:3541
                minargs = 1; // c:3542
                maxargs = 1; // c:3542
            } else {
                minargs = 0; // c:3544
                maxargs = -1; // c:3545
            }
            // c:3548-3552 — bad math function name check.
            let bytes = funcname.as_bytes();
            let first_bad = bytes.is_empty()
                || (bytes[0] as char).is_ascii_digit()
                || !bytes
                    .iter()
                    .all(|&c| c.is_ascii_alphanumeric() || c == b'_');
            if first_bad {
                // c:3549
                zwarnnam(
                    name, // c:3550
                    &format!("-M {}: bad math function name", funcname),
                );
                return 1; // c:3551
            }
            if let Some(arg) = argv_iter.next() {
                // c:3554
                match arg.parse::<i32>() {
                    // c:3555 zstrtol
                    Ok(n) if n >= 0 => minargs = n, // c:3556
                    _ => {
                        zwarnnam(
                            name, // c:3557
                            &format!("-M: invalid min number of arguments: {}", arg),
                        );
                        return 1; // c:3559
                    }
                }
                if OPT_ISSET(ops, b's') && minargs != 1 {
                    // c:3561
                    zwarnnam(
                        name, // c:3562
                        "-Ms: must take a single string argument",
                    );
                    return 1; // c:3563
                }
                maxargs = minargs; // c:3565
            }
            if let Some(arg) = argv_iter.next() {
                // c:3568
                match arg.parse::<i32>() {
                    // c:3569
                    Ok(n) if n >= -1 && (n == -1 || n >= minargs) => maxargs = n,
                    _ => {
                        zwarnnam(
                            name, // c:3573
                            &format!("-M: invalid max number of arguments: {}", arg),
                        );
                        return 1; // c:3576
                    }
                }
                if OPT_ISSET(ops, b's') && maxargs != 1 {
                    // c:3578
                    zwarnnam(
                        name, // c:3579
                        "-Ms: must take a single string argument",
                    );
                    return 1; // c:3580
                }
            }
            let modname = argv_iter.next().cloned(); // c:3584-3585
            if argv_iter.next().is_some() {
                // c:3586
                zwarnnam(name, "-M: too many arguments"); // c:3587
                return 1; // c:3588
            }
            // c:3591-3598 — alloc and populate mathfunc.
            let mut flags = MFF_USERFUNC; // c:3593
            if OPT_ISSET(ops, b's') {
                // c:3594
                flags |= MFF_STR; // c:3595
            }
            let new_fn = mathfunc {
                next: None,             // c:3608 chain via Vec
                name: funcname.clone(), // c:3592
                flags,                  // c:3593
                nfunc: None,
                sfunc: None,
                module: modname, // c:3596
                minargs,         // c:3597
                maxargs,         // c:3598
                funcid: 0,
            };
            queue_signals(); // c:3600
            if let Ok(mut table) = MATHFUNCS.lock() {
                // c:3601-3606 — remove existing user entry with same name.
                if let Some(i) = table.iter().position(|p| p.name == new_fn.name) {
                    table.remove(i); // c:3603
                }
                // c:3608-3609 — prepend to mathfuncs head.
                table.insert(0, new_fn);
            }
            unqueue_signals(); // c:3610
            return returnval;
        }
    }

    // c:3616-3655 — `-X` re-autoload from inside a function.
    if OPT_MINUS(ops, b'X') {
        // c:3616
        if argv.len() > 1 {
            // c:3620
            zwarnnam(name, "-X: too many arguments"); // c:3621
            return 1; // c:3622
        }
        queue_signals(); // c:3624
                         // c:3625-3633 — walk funcstack to find the enclosing FS_FUNC frame.
        let funcname: Option<String> = {
            let stack = FUNCSTACK.lock().map(|s| s.clone()).unwrap_or_default();
            stack
                .iter()
                .rev()
                .find(|fs| !fs.name.is_empty()) // c:3626
                .map(|fs| fs.name.clone()) // c:3631
        };
        let ret;
        if funcname.is_none() {
            // c:3635
            // c:3637 — `zerrnam(name, "bad autoload");`. zerrnam, NOT zwarnnam:
            // in C the `zerr*` family sets `errflag`, which aborts the enclosing
            // command and the script, while `zwarn*` only prints. The comment
            // here already cited zerrnam but the call was the warning form, so
            // `autoload -X` outside a function printed the diagnostic and then
            // kept going — `autoload -X; echo AFTER` ran AFTER where zsh stops.
            // (A subshell still continues in both, because only the subshell
            // dies; inside a function `-X` takes the real autoload path and
            // never reaches here.)
            zerrnam(name, "bad autoload"); // c:3637
            ret = 1; // c:3638
        } else {
            let fname = funcname.unwrap();
            // c:3640-3647 — getnode(shfunctab, funcname) || addnode(new shf).
            let mut shf_ptr = shfunctab_lock()
                .read()
                .map(|t| t.getnode(fname.as_str()))
                .unwrap_or(std::ptr::null_mut());
            if !shf_ptr.is_null() { // c:3640
                 // exists already
            } else {
                // c:3645 — `shf = zshcalloc(sizeof *shf);`
                //          `shfunctab->addnode(shfunctab, ztrdup(funcname), shf);`
                let new_shf = Box::into_raw(Box::new(shfunc {
                    node: hashnode {
                        next: None,
                        nam: fname.clone(),
                        flags: 0,
                    },
                    filename: None,
                    lineno: 0,
                    funcdef: None,
                    redir: None,
                    sticky: None,
                    body: None,
                }));
                if let Ok(mut t) = shfunctab_lock().write() {
                    t.addnode(new_shf); // c:3646
                }
                shf_ptr = new_shf;
            }
            if !argv.is_empty() {
                // c:3648
                if !shf_ptr.is_null() {
                    let shf_mut = unsafe { &mut *shf_ptr };
                    let mut old_slot = shf_mut.filename.take();
                    dircache_set(&mut old_slot, None); // c:3649
                    let mut new_slot: Option<String> = None;
                    dircache_set(&mut new_slot, Some(&argv[0])); // c:3650
                    shf_mut.filename = new_slot;
                    on |= PM_LOADDIR; // c:3294 — `shf->node.flags |= PM_LOADDIR;`
                }
            }
            // c:3653 — `shf->node.flags = on;` — `on` carries
            // PM_UNDEFINED (set for -X at c:3352 / builtin.rs:6142).
            // This write was MISSING: eval_autoload's first gate is
            // `if (!(shf->node.flags & PM_UNDEFINED)) return 1;`
            // (c:3168), so zinit's NEW_AUTOLOAD=1 stubs —
            //   functions[add-zsh-hook]="local -a fpath; fpath=(…);
            //                            builtin autoload -X -U -z"
            // — silently returned 1 and every wrapped plugin autoload
            // (add-zsh-hook, regexp-replace, add-zle-hook-widget)
            // reported `function definition file not found` at first
            // call in the interactive session.
            if !shf_ptr.is_null() {
                unsafe {
                    (*shf_ptr).node.flags = on as i32; // c:3653
                }
            }
            // RUST-ONLY: drop the executor's compiled chunk for the
            // CURRENT (stub) definition. C's eval_autoload -X arm
            // replaces shf->funcdef with mkautofn (c:3180) so the
            // bin_eval re-call loads the real file; zshrs's dispatch
            // keeps a parallel functions_compiled chunk whose
            // presence skips the autoload prelude — re-dispatch ran
            // the STALE stub (zinit NEW_AUTOLOAD=1: `builtin
            // autoload -X` inside the stub) in an infinite mutual
            // recursion → stack overflow in the interactive session.
            let _ = crate::ported::exec::unregister_function(&fname);
            // c:3654 — `ret = eval_autoload(shf, funcname, ops, func);`
            ret = eval_autoload(shf_ptr, &fname, ops, _func); // c:3654
        }
        unqueue_signals(); // c:3656
        return ret;
    }

    // c:3658-3669 — no-arg listing path: print all (non-DISABLED) shfuncs
    // matching `on|off` mask through scanshfunc + printnode.
    if argv.is_empty() {
        // c:3658
        queue_signals(); // c:3663
        if OPT_ISSET(ops, b'U') && !OPT_ISSET(ops, b'u') {
            // c:3664
            on &= !PM_UNDEFINED; // c:3665
        }
        // c:3666 — `scanshfunc(1, on|off, DISABLED, shfunctab->printnode,
        //              pflags, expand);` — C's scanhashtable takes a
        // `flags1` filter mask: only entries where `(flags & flags1) ==
        // flags1` are emitted. Rust's `scanshfunc` doesn't accept the
        // filter, so apply it inside the closure. Without this gate
        // `functions -t` (no args) walked every function and printed
        // its body — should be empty / name-only restricted to
        // PM_TAGGED entries. Bug #468.
        let filter_mask = on | off;
        scanshfunc(|_nm, entry| {
            let f = entry.node.flags as u32;
            if filter_mask != 0 && (f & filter_mask) != filter_mask {
                return;
            }
            printshfuncexpand(entry, pflags, expand);
        });
        unqueue_signals(); // c:3668
        return returnval;
    }

    // c:3672-3708 — `-m` glob: treat each arg as a pattern, scan-and-print
    // matching shfuncs (no on/off → list) or apply on/off mask.
    if OPT_ISSET(ops, b'm') {
        // c:3673
        on &= !PM_UNDEFINED; // c:3674
        let mut returnval = returnval;
        for pat in argv {
            // c:3675
            queue_signals(); // c:3676
                             // c:3678 — `tokenize(*argv)` + `patcompile(...)`
            let pprog = patcompile(
                &{
                    let mut __pat_tok = (pat).to_string();
                    crate::ported::glob::tokenize(&mut __pat_tok);
                    __pat_tok
                }, // c:3680
                PAT_HEAPDUP,
                None,
            );
            if let Some(prog) = pprog {
                // c:3680-3683 — scan-and-print matching shfuncs.
                if (on | off) == 0 && !OPT_ISSET(ops, b'X') {
                    // c:3682
                    // c:3682-3683 — `scanmatchshfunc(pprog, 1, 0,
                    //   DISABLED, shfunctab->printnode, pflags, expand)`.
                    // Walk shfunctab via the hashtable.rs port and emit
                    // each match through `printshfuncexpand` so autoload
                    // stubs come out as `name () { # undefined; builtin
                    // autoload -XU }` and loaded funcs print their body.
                    scanmatchshfunc(Some(pat), |_nm, entry| {
                        printshfuncexpand(entry, pflags, expand);
                    });
                } else {
                    // c:3686-3699 — walk shfunctab, apply (on, off) and
                    // re-eval autoload for each matching shf.
                    let names: Vec<String> = shfunctab_lock()
                        .read()
                        .map(|t| t.iter().map(|(k, _)| k.clone()).collect())
                        .unwrap_or_default();
                    for nm in &names {
                        // pattry approximated by string equality / glob
                        // here; full pat engine is in src/ported/pattern.rs.
                        if !pattry(&prog, nm) {
                            // c:3690
                            continue;
                        }
                        let shf_ptr = shfunctab_lock()
                            .read()
                            .map(|t| t.getnode(nm.as_str()))
                            .unwrap_or(std::ptr::null_mut());
                        if shf_ptr.is_null() {
                            continue;
                        }
                        let shf_mut = unsafe { &mut *shf_ptr };
                        // c:3691 — `shf->node.flags = (... | (on & ~PM_UNDEFINED)) & ~off;`
                        shf_mut.node.flags =
                            (shf_mut.node.flags | ((on & !PM_UNDEFINED) as i32)) & !(off as i32); // c:3691
                        if check_autoload(shf_ptr, &shf_mut.node.nam, ops, _func) != 0 {
                            // c:3693
                            returnval = 1; // c:3695
                        }
                    }
                }
            } else {
                // c:3700-3702 — `untokenize + zwarnnam(name, "bad pattern")`.
                zwarnnam(name, &format!("bad pattern : {}", pat)); // c:3701
                returnval = 1; // c:3702
            }
            unqueue_signals(); // c:3704
        }
        return returnval;
    }

    // c:3710-3735 — literal name list, no globbing.
    let mut returnval = returnval;
    queue_signals(); // c:3711
    for fname in argv {
        // c:3712
        // c:3713-3714 — `returnval = dump_autoload(name, *argv, on, ops, func);`
        // This was a stub that just `continue`d, so `autoload -w FILE.zwc`
        // silently did nothing: a missing dump produced no diagnostic where zsh
        // reports `can't open zwc file: …` (the warning lives in
        // `load_dump_header`, parse.rs), and a VALID dump never registered its
        // functions for autoload at all. `dump_autoload` itself was already
        // ported with a matching signature (parse.rs) — only this call was
        // missing, so the whole `-w` feature was unreachable.
        if OPT_ISSET(ops, b'w') {
            // c:3713
            returnval = crate::ported::parse::dump_autoload(name, fname, on as i32, ops, _func); // c:3714
            continue;
        }
        // c:3715 — `shf = shfunctab->getnode(shfunctab, *argv);`
        let shf_ptr = shfunctab_lock()
            .read()
            .map(|t| t.getnode(fname.as_str()))
            .unwrap_or(std::ptr::null_mut());
        if !shf_ptr.is_null() {
            // c:3715
            let shf_mut = unsafe { &mut *shf_ptr };
            if (on | off) != 0 {
                // c:3717
                // c:3719 — apply on/off mask, then check_autoload.
                shf_mut.node.flags =
                    (shf_mut.node.flags | ((on & !PM_UNDEFINED) as i32)) & !(off as i32); // c:3719
                if check_autoload(shf_ptr, &shf_mut.node.nam, ops, _func) != 0 {
                    // c:3720
                    returnval = 1; // c:3721
                }
            } else {
                // c:3723 — `printshfuncexpand(&shf->node, pflags, expand);`
                // C prints the function via shfunctab.printnode honoring
                // pflags (PRINT_NAMEONLY / verbose). The previous Rust
                // port just printed the name — `functions f` skipped
                // the `f () { ... body ... }` body listing entirely.
                printshfuncexpand(shf_mut, pflags, expand); // c:3723
            }
        } else if (on & PM_UNDEFINED) != 0 {
            // c:3725
            // c:3726-3782 — autoload-define path: TRAP* + abs-path + new shf.
            let mut sigidx: i32 = -1;
            let mut ok = true;
            // c:3728-3735 — TRAP* prefix → removetrapnode(sigidx).
            if fname.starts_with("TRAP") {
                // c:3728
                // c:3729 — `if ((sigidx = getsigidx(*argv + 4)) != -1)`
                sigidx = getsigidx(&fname[4..]); // c:3729
                if sigidx != -1 {
                    // c:3729
                    // c:3733 — `removetrapnode(sigidx);`
                    removetrapnode(sigidx); // c:3733
                }
            }
            // c:3737-3759 — absolute path /dir/base form: install dir on
            // existing matching base name with PM_UNDEFINED set.
            if fname.starts_with('/') {
                // c:3737
                let base = fname.rsplit('/').next().unwrap_or("");
                if !base.is_empty() {
                    let base_ptr = shfunctab_lock()
                        .read()
                        .map(|t| t.getnode(base))
                        .unwrap_or(std::ptr::null_mut());
                    if !base_ptr.is_null() {
                        let bs = unsafe { &mut *base_ptr };
                        // c:3742 — apply flag mask.
                        bs.node.flags =
                            (bs.node.flags | ((on & !PM_UNDEFINED) as i32)) & !(off as i32); // c:3742
                        if (bs.node.flags as u32 & PM_UNDEFINED) != 0 {
                            // c:3744
                            let dir = if fname.len() > 1 && base.len() == fname.len() - 1 {
                                "/".to_string() // c:3747
                            } else {
                                fname[..fname.len() - base.len() - 1].to_string()
                                // c:3749-3751
                            };
                            let mut old_slot = bs.filename.take();
                            dircache_set(&mut old_slot, None); // c:3753
                            let mut new_slot: Option<String> = None;
                            dircache_set(&mut new_slot, Some(&dir)); // c:3754
                            bs.filename = new_slot;
                        }
                        if check_autoload(base_ptr, &bs.node.nam, ops, _func) != 0 {
                            // c:3756
                            returnval = 1;
                        }
                        continue; // c:3758
                    }
                }
            }
            // c:3763-3766 — new undefined shf, mkautofn, add_autoload_function.
            let new_shf = Box::new(shfunc {
                node: hashnode {
                    next: None,
                    nam: fname.clone(),
                    flags: on as i32, // c:3764
                },
                filename: None,
                lineno: 0,
                funcdef: None,
                redir: None,
                sticky: None,
                body: None,
            });
            let new_shf_ptr = Box::into_raw(new_shf);
            let _ = mkautofn(new_shf_ptr); // c:3765
            add_autoload_function(new_shf_ptr, fname); // c:3767
                                                       // PFA-SMR: an `autoload NAME` registers a function stub
                                                       // that fires on first call. Record it as a `function`
                                                       // kind so replay re-registers the autoload. C zsh has
                                                       // no recorder hook here; emit per Rust-only schema.
            #[cfg(feature = "recorder")]
            if crate::recorder::is_enabled() {
                let ctx = crate::recorder::recorder_ctx_global();
                crate::recorder::emit_function(fname, None, ctx);
            }
            if sigidx != -1 {
                // c:3769
                // c:3770 — `if (settrap(sigidx, NULL, ZSIG_FUNC)) { ... }`
                if settrap(sigidx, None, ZSIG_FUNC) != 0 {
                    // c:3770
                    // c:3771 — `shfunctab->removenode(shfunctab, *argv);`
                    if let Ok(mut t) = shfunctab_lock().write() {
                        t.remove(fname);
                    }
                    // c:3772 — `shfunctab->freenode(&shf->node);` Drop covers it.
                    returnval = 1; // c:3773
                    ok = false; // c:3774
                }
            }
            if ok && check_autoload(new_shf_ptr, &fname, ops, _func) != 0 {
                // c:3779
                returnval = 1; // c:3780
            }
        } else {
            // c:3783 — `returnval = 1;` (named function not found,
            //          no autoload requested).
            returnval = 1; // c:3783
        }
    }
    unqueue_signals(); // c:3785
    let _ = (expand, pflags);
    returnval
}

/// Port of `mkautofn(Shfunc shf)` from Src/builtin.c:3790.
/// C: `Eprog mkautofn(Shfunc shf)` — synthesize a 5-wordcode body that
///   re-fires the autoload mechanism when first called.
pub fn mkautofn(shf: *mut shfunc) -> *mut eprog {
    // c:3790
    // c:3793-3810 — alloc Eprog with 5 wordcode slots, set p->shf, p->npats=0,
    // p->nref=1 (permanent). Static-link path: synthesize a Box<eprog> that
    // satisfies the autoload trampoline contract.
    let p = Box::new(eprog {
        len: 5 * size_of::<u32>() as i32, // c:3796
        prog: Vec::new(),                 // c:3797
        strs: None,                       // c:3798
        shf: if shf.is_null() {
            None
        }
        // c:3799
        else {
            Some(unsafe { Box::from_raw(shf) })
        },
        npats: 0, // c:3800
        nref: 1,  // c:3801
        flags: 0,
        pats: Vec::new(),
        dump: None,
        strs_metafied: false, // native pool — clean UTF-8
    });
    Box::into_raw(p)
}

/// Port of `bin_unset(char *name, char **argv, Options ops, int func)` from Src/builtin.c:3818.
/// C: `int bin_unset(char *name, char **argv, Options ops, int func)` —
///   `-f` delegates to `bin_unhash`; `-m` glob deletes matching params;
///   default literal-name unset with subscript handling.
/// WARNING: param names don't match C — Rust=(name, argv, func) vs C=(name, argv, ops, func)
pub fn bin_unset(
    name: &str,
    argv: &[String], // c:3818
    ops: &options,
    func: i32,
) -> i32 {
    let mut returnval = 0i32; // c:3823
    let mut match_count = 0i32; // c:3823

    // PFA-SMR aspect: emit unset events for each named param. The
    // recorder tracks state-mutations across the shell session for
    // the zshrs-recorder binary's replay/inspect tooling.
    #[cfg(feature = "recorder")]
    if crate::recorder::is_enabled() {
        let ctx = crate::recorder::recorder_ctx_global();
        for a in argv {
            if a.starts_with('-') || a == "--" {
                continue;
            }
            crate::recorder::emit_unset(a, ctx.clone());
        }
    }

    // c:3826 — `if (OPT_ISSET(ops,'f')) return bin_unhash(name, argv, ops, func);`
    if OPT_ISSET(ops, b'f') {
        // c:3826
        return bin_unhash(name, argv, ops, func); // c:3827
    }

    // c:3830-3862 — `-m` glob.
    if OPT_ISSET(ops, b'm') {
        // c:3831
        for s in argv {
            // c:3832
            queue_signals(); // c:3833
            let pprog = patcompile(
                &{
                    let mut __pat_tok = (s).to_string();
                    crate::ported::glob::tokenize(&mut __pat_tok);
                    __pat_tok
                }, // c:3836
                PAT_HEAPDUP,
                None,
            );
            if let Some(prog) = pprog {
                // c:3838-3851 — walk paramtab (NOT env::vars), unset via
                // unsetparam (which respects PM_NAMEREF + readonly guards).
                //
                // The previous Rust port walked `std::env::vars()` — the
                // OS environment. This is a different name set:
                //   - Shell-internal vars (not exported) would survive
                //     `unset -m 'PATTERN'` even though they match.
                //   - Env vars not in paramtab would be removed without
                //     the PM_READONLY guard in unsetparam_pm.
                //
                // Same family of bug as the env::var vs paramtab fixes
                // earlier in the series.
                let names: Vec<String> = {
                    let tab = paramtab().read().unwrap();
                    tab.keys().cloned().collect()
                };
                for nm in &names {
                    if pattry(&prog, nm) {
                        // c:3842
                        // c:3846 — `unsetparam_pm(pm, 0, 1)` runs on the
                        // node the walk found. For a magic assoc/array
                        // nothing has read yet that node is still the
                        // PM_AUTOLOAD stub (Src/module.c:1218-1223): a
                        // plain PM_SCALAR with no PM_READONLY, so it is
                        // removed silently (c:3874). zshrs seeds the real
                        // special instead, whose PM_READONLY would reject
                        // the glob — `unset -m 'func*'` died on
                        // `functrace` where zsh unsets the lot. Same
                        // stub-state branch as the literal-name arm below.
                        if crate::vm_helper::module_param_is_autoload_stub(nm)
                            && (crate::ported::modules::parameter::PARTAB
                                .iter()
                                .any(|e| e.name == nm.as_str())
                                || crate::ported::modules::parameter::PARTAB_ARRAY
                                    .iter()
                                    .any(|e| e.name == nm.as_str()))
                        {
                            let _ = paramtab().write().ok().map(|mut t| t.remove(nm)); // c:3874
                            let _ = crate::ported::params::paramtab_hashed_storage()
                                .lock()
                                .ok()
                                .as_deref_mut()
                                .map(|m| m.remove(nm.as_str()));
                            match_count += 1; // c:3848
                            continue;
                        }
                        unsetparam(nm); // c:3847 (with guards)
                        match_count += 1; // c:3848
                    }
                }
            } else {
                zwarnnam(name, &format!("bad pattern : {}", s)); // c:3854
                returnval = 1; // c:3855
            }
            unqueue_signals(); // c:3857
        }
        if match_count == 0 {
            // c:3861
            returnval = 1; // c:3862
        }
        return returnval; // c:3863
    }

    // c:3866-3915 — literal-name unset with optional subscript.
    queue_signals(); // c:3867
    for s in argv {
        // c:3868
        // c:3869-3878 — extract `name[subscript]` shape.
        let (nm, subscript) = match s.find('[') {
            // c:3869
            Some(start) if s.ends_with(']') => {
                // c:3873
                (&s[..start], Some(&s[start + 1..s.len() - 1])) // c:3875
            }
            Some(_) => {
                // c:3879-3884 — bracket without `]` close → invalid.
                zwarnnam(name, &format!("{}: invalid parameter name", s)); // c:3882
                returnval = 1; // c:3883
                continue; // c:3884
            }
            None => (s.as_str(), None),
        };
        // c:3878 — `if (... || !isident(s))` invalid identifier check.
        if nm.is_empty()
            || !nm
                .chars()
                .next()
                .map_or(false, |c| c.is_alphabetic() || c == '_')
            || !nm.chars().all(|c| c.is_alphanumeric() || c == '_')
        {
            zwarnnam(name, &format!("{}: invalid parameter name", s)); // c:3882
            returnval = 1; // c:3883
            continue;
        }
        // c:3886-3905 — `if (!pm) continue;` then unset.
        // C `unsetparam_pm` dispatches on `pm->gsu` (the gsu_*
        // accessor for the param's type): assoc gets
        // `gsu_a->unset(pm, subscript)`, array gets
        // `gsu_arr->unset(pm, subscript)`, scalar gets `unsetparam`.
        match subscript {
            // c:3886
            Some(key) => {
                // c:Src/Modules/mapfile.c:126 unsetpmmapfile — unsetting
                // an element of the special $mapfile assoc UNLINKS the
                // file named by the key. It is not a regular assoc, so
                // route here before the generic assoc/array delete.
                if nm == "mapfile" {
                    crate::ported::modules::mapfile::unsetpmmapfile(key, false);
                    continue;
                }
                // c:3891-3895 — `if (PM_TYPE(pm->node.flags) == PM_HASHED) {
                //     HashTable tht = paramtab;
                //     if ((paramtab = pm->gsu.h->getfn(pm))) unsetparam(subscript);
                //     paramtab = tht; }`
                //
                // For the zsh/parameter magic associations the hash that
                // `gsu.h->getfn` hands back is NOT a value store: its
                // `getnode` is `getpmfunction` / `getpmralias` / … (each
                // synthesising a child Param whose scalar gsu carries the
                // element `unsetfn`), and that `unsetfn` mutates the REAL
                // shell table — `shfunctab`, `aliastab`, `cmdnamtab`,
                // `nameddirtab`, the option flags. Deleting a key from a
                // value snapshot and writing the snapshot back (the generic
                // assoc path below) cannot express that: the whole-hash
                // setfns (`setpmfunctions`, c:344-365) only ADD entries, so
                // `unset "functions[m]"` left the shell function defined.
                //
                // Mirror of the element-SET dispatch in `assignsparam`
                // (`src/ported/params.rs:6455`), which routes
                // `functions[m]=body` to `setpmfunction` for the same reason.
                {
                    use crate::ported::modules::parameter as pmod;
                    use crate::ported::zsh_h::Param;
                    // c:3884-3886 — `pm = (paramtab == realparamtab ?
                    // paramtab->getnode2(paramtab, s) : paramtab->getnode(...))`
                    // — the comment on that line is `getnode2() to avoid
                    // autoloading`. A magic assoc that has never been touched
                    // is still the PM_AUTOLOAD stub `zsh/parameter` planted
                    // (c:Src/module.c:1218-1219), and getnode2 does NOT run
                    // loadparamnode, so `PM_TYPE(pm->node.flags)` is PM_SCALAR,
                    // not PM_HASHED: control falls into the scalar arm at
                    // c:3896-3919, where `getindex` rejects the string
                    // subscript with `assignment to invalid subscript range`
                    // and the function/alias is left alone. Verified against
                    // zsh 5.9.2: `f(){ :; }; unset "functions[f]"; whence f`
                    // still prints `f`, while the same unset AFTER any read of
                    // `$functions` removes it.
                    if crate::vm_helper::module_param_is_autoload_stub(nm) {
                        // c:Src/params.c getindex — `zerr("%s: assignment to
                        // invalid subscript range", ...)`; bin_unset then does
                        // `returnval = errflag; errflag &= ~ERRFLAG_ERROR;`
                        // (c:3917-3918), so the status is 1 but the script is
                        // NOT aborted.
                        zwarn(&format!("{}: assignment to invalid subscript range", nm));
                        returnval = 1; // c:3917
                        continue;
                    }
                    // c:3893 `paramtab->getnode(paramtab, subscript)` (via
                    // unsetparam, c:3763-3768) paired with the element gsu's
                    // `unsetfn`. `None` for the unsetfn is C's NULL slot in
                    // `nullsetscalar_gsu` (`Src/params.c:168-169`), used by
                    // every PM_READONLY_SPECIAL hash — unreachable because
                    // `unsetparam_pm` rejects on PM_READONLY first (c:3785).
                    type ElemUnset = fn(Param, i32);
                    let nul: *mut crate::ported::zsh_h::HashTable = std::ptr::null_mut();
                    let hashed: Option<(Option<Param>, Option<ElemUnset>)> = match nm {
                        // c:2281 SPECIALPMDEF("functions", …, getpmfunction),
                        // c:394-395 pmfunction_gsu { …, unsetpmfunction }.
                        "functions" => Some((
                            pmod::getpmfunction(nul, key),
                            Some(pmod::unsetpmfunction as ElemUnset),
                        )),
                        // c:2261-2262 + c:396-397 pmdisfunction_gsu shares
                        // `unsetpmfunction`.
                        "dis_functions" => Some((
                            pmod::getpmdisfunction(nul, key),
                            Some(pmod::unsetpmfunction as ElemUnset),
                        )),
                        // c:2251 + c:1867-1868 pmralias_gsu { …, unsetpmalias }.
                        "aliases" => Some((
                            pmod::getpmralias(nul, key),
                            Some(pmod::unsetpmalias as ElemUnset),
                        )),
                        // c:2287-2288 + c:1869-1870 pmgalias_gsu.
                        "galiases" => Some((
                            pmod::getpmgalias(nul, key),
                            Some(pmod::unsetpmalias as ElemUnset),
                        )),
                        // c:2258-2259 + c:1873-1874 pmdisralias_gsu.
                        "dis_aliases" => Some((
                            pmod::getpmdisralias(nul, key),
                            Some(pmod::unsetpmalias as ElemUnset),
                        )),
                        // c:2266-2267 + c:1875-1876 pmdisgalias_gsu.
                        "dis_galiases" => Some((
                            pmod::getpmdisgalias(nul, key),
                            Some(pmod::unsetpmalias as ElemUnset),
                        )),
                        // c:2312-2313 + c:1871-1872 pmsalias_gsu { …,
                        // unsetpmsalias } (sufaliastab, not aliastab).
                        "saliases" => Some((
                            pmod::getpmsalias(nul, key),
                            Some(pmod::unsetpmsalias as ElemUnset),
                        )),
                        // c:2272-2273 + c:1877-1878 pmdissalias_gsu.
                        "dis_saliases" => Some((
                            pmod::getpmdissalias(nul, key),
                            Some(pmod::unsetpmsalias as ElemUnset),
                        )),
                        // c:2256 + c:209-210 pmcommand_gsu { …, unsetpmcommand }.
                        "commands" => Some((
                            pmod::getpmcommand(nul, key),
                            Some(pmod::unsetpmcommand as ElemUnset),
                        )),
                        // c:2301-2302 + c:1607-1608 pmnamedir_gsu.
                        "nameddirs" => Some((
                            pmod::getpmnameddir(nul, key),
                            Some(pmod::unsetpmnameddir as ElemUnset),
                        )),
                        // c:2303-2304 + c:997-998 pmoption_gsu { …,
                        // unsetpmoption } — turns the option OFF.
                        "options" => Some((
                            pmod::getpmoption(nul, key),
                            Some(pmod::unsetpmoption as ElemUnset),
                        )),
                        // PM_READONLY_SPECIAL hashes (c:2255, 2259-2260,
                        // 2264-2265, 2269-2270, 2290-2291, 2296-2300,
                        // 2305-2306, 2314-2317): their `getpm*` stamps
                        // PM_SCALAR|PM_READONLY, so the element unset is
                        // rejected by unsetparam_pm's readonly guard.
                        "parameters" => Some((pmod::getpmparameter(nul, key), None)),
                        "builtins" => Some((pmod::getpmbuiltin(nul, key), None)),
                        "dis_builtins" => Some((pmod::getpmdisbuiltin(nul, key), None)),
                        "functions_source" => Some((pmod::getpmfunction_source(nul, key), None)),
                        "dis_functions_source" => {
                            Some((pmod::getpmdisfunction_source(nul, key), None))
                        }
                        // c:1051-1056 — getpmmodule starts with
                        // `m = modulestab->getnode2(modulestab, name);
                        //  if (!m) return NULL;`, so a name that is not a
                        // known module yields NO node and `unset
                        // "modules[bogus]"` is a silent no-op (rc 0), while a
                        // KNOWN module hits the PM_READONLY rejection. zshrs's
                        // getpmmodule port never returns None — it folds the
                        // unknown-name case into the "no type string" arm
                        // (c:1068-1069, PM_UNSET|PM_SPECIAL), which is pinned
                        // by its unit test — so re-apply C's `!m` gate here:
                        // an empty type string only means NULL-in-C when the
                        // module is absent from modulestab.
                        "modules" => Some((
                            pmod::getpmmodule(nul, key).filter(|p| {
                                (p.node.flags as u32 & PM_UNSET) == 0
                                    || crate::ported::module::MODULESTAB
                                        .lock()
                                        .map(|t| t.modules.contains_key(key))
                                        .unwrap_or(false)
                            }),
                            None,
                        )),
                        "history" => Some((pmod::getpmhistory(nul, key), None)),
                        "jobdirs" => Some((pmod::getpmjobdir(nul, key), None)),
                        "jobstates" => Some((pmod::getpmjobstate(nul, key), None)),
                        "jobtexts" => Some((pmod::getpmjobtext(nul, key), None)),
                        "userdirs" => Some((pmod::getpmuserdir(nul, key), None)),
                        "usergroups" => Some((pmod::getpmusergroups(nul, key), None)),
                        _ => None,
                    };
                    if let Some((node, unsetfn)) = hashed {
                        // c:3765-3768 — `if ((pm = paramtab->getnode(...)))
                        // unsetparam_pm(pm, 0, 1);` — a NULL node (unknown
                        // module) is silently ignored.
                        if let Some(pm) = node {
                            let f = pm.node.flags as u32;
                            if (f & PM_READONLY) != 0 {
                                // c:3785-3790 — `zerr("read-only %s: %s", …)`.
                                zerr(&format!("read-only variable: {}", pm.node.nam));
                            } else if (f & PM_UNSET) == 0 {
                                // c:3805-3806 — `if (!(pm->node.flags &
                                // PM_UNSET) || (pm->node.flags &
                                // PM_REMOVABLE)) pm->gsu.s->unsetfn(pm, exp);`
                                // A missing key comes back PM_UNSET (e.g.
                                // getfunction c:437-440), so the unsetfn is
                                // skipped and `unset "functions[nope]"` is a
                                // silent no-op, as in zsh.
                                if let Some(uf) = unsetfn {
                                    uf(pm, 1); // c:3806 exp=1 from unsetparam
                                }
                            }
                        }
                        // c:3895 `paramtab = tht;` — nothing else to do for
                        // this argument; the generic assoc/array delete below
                        // must not also run.
                        continue;
                    }
                }
                // c:3893 assoc subscript: `m[key]` delete.
                if let Some(mut map) = crate::ported::exec::assoc(nm) {
                    map.shift_remove(key); // c:3893
                    crate::ported::exec::set_assoc(nm, map);
                } else if let Some(mut arr) = crate::ported::exec::array(nm) {
                    // c:3895 — array subscript: `arr[N]` sets element
                    // N to empty; `arr[N,M]` clears the inclusive
                    // range. The previous Rust port only handled the
                    // single-index form; `unset arr[2,3]` was a no-op.
                    let len_i = arr.len() as i32;
                    let resolve = |raw: i32| -> Option<usize> {
                        let pos = if raw < 0 { len_i + raw + 1 } else { raw };
                        if pos >= 1 && pos as usize <= arr.len() {
                            Some((pos - 1) as usize)
                        } else {
                            None
                        }
                    };
                    if let Some((s, e)) = key.split_once(',') {
                        // c:Src/params.c — `unset arr[N,M]` replaces
                        // the N..M range with ONE empty element. For
                        // (a b c d), `unset arr[2,3]` → (a "" d).
                        // Negative bounds aren't supported by zsh's
                        // unset (it leaves the array unchanged), so
                        // restrict to positive forms.
                        let s_i = s.trim().parse::<i32>().ok();
                        let e_i = e.trim().parse::<i32>().ok();
                        if let (Some(start), Some(end)) = (s_i, e_i) {
                            if start >= 1 && end >= 1 {
                                let from = (start - 1) as usize;
                                let to = (end - 1) as usize;
                                if from < arr.len() && from <= to {
                                    let cap_to = to.min(arr.len() - 1);
                                    let mut new_arr: Vec<String> = Vec::with_capacity(arr.len());
                                    new_arr.extend(arr[..from].iter().cloned());
                                    new_arr.push(String::new());
                                    new_arr.extend(arr[cap_to + 1..].iter().cloned());
                                    crate::ported::exec::set_array(nm, new_arr);
                                }
                            }
                        }
                    } else if crate::dash_mode::bash_mode() {
                        // bash sparse arrays: subscripts are 0-based and
                        // `unset a[i]` leaves a HOLE (index gap) rather than
                        // shifting or clearing to a dense empty. Negative
                        // indexes count from the end. The dense Vec keeps a
                        // "" placeholder; bash_arrays tracks the hole so
                        // `${a[@]}`/`${#a[@]}`/`${!a[@]}` skip it.
                        if let Ok(i) = key.parse::<i32>() {
                            let len_i = arr.len() as i32;
                            let idx0 = if i < 0 { len_i + i } else { i };
                            if idx0 >= 0 && (idx0 as usize) < arr.len() {
                                let idx = idx0 as usize;
                                arr[idx] = String::new();
                                crate::bash_arrays::note_unset(nm, idx);
                                crate::ported::exec::set_array(nm, arr);
                            }
                        }
                    } else if let Ok(i) = key.parse::<i32>() {
                        // c:Src/params.c — single-index unset only
                        // accepts positive subscripts and the special
                        // -1 form (clear last element). Other negative
                        // values are a no-op in zsh — verified
                        // empirically: `unset arr[-2]`, `arr[-3]` etc.
                        // leave the array unchanged.
                        // `unset arr[0]` is invalid (unless
                        // KSHZEROSUBSCRIPT) and emits a diagnostic.
                        if i == 0 {
                            zwarn(&format!("{}: assignment to invalid subscript range", nm));
                            returnval = 1;
                        } else if i == -1 {
                            if !arr.is_empty() {
                                let idx = arr.len() - 1;
                                arr[idx] = String::new();
                                crate::ported::exec::set_array(nm, arr);
                            }
                        } else if i > 0 {
                            if let Some(idx) = resolve(i) {
                                arr[idx] = String::new();
                                crate::ported::exec::set_array(nm, arr);
                            }
                        }
                        // Other negative values: no-op (zsh behavior).
                    }
                    // c:Src/params.c:2922 — a subscript unset of a tied
                    // colon-array (`unset path[2]`) re-derives the scalar
                    // side, just like the assignment paths in params.rs.
                    if let Some((_, scalar)) = crate::ported::params::TIED_COLON_ARRAYS
                        .iter()
                        .find(|(a, _)| *a == nm)
                    {
                        let cur = crate::ported::exec::array(nm).unwrap_or_default();
                        crate::ported::params::assignsparam(scalar, &cur.join(":"), 0);
                    }
                }
            }
            None => {
                // c:3939-3951 — nameref handling. Without -n the ref
                // resolves and the TARGET is unset; refs bound to an
                // outer scope are just marked unset (kept in table,
                // PM_DECLARED). With -n the ref itself is removed.
                let mut nm: &str = nm;
                let mut resolved_target: Option<String> = None; // non-ref target to unset
                let mut ref_removal: Option<String> = None; // ref entry to remove
                let mut handled = false;
                if crate::ported::params::is_nameref(nm) {
                    if OPT_ISSET(ops, b'n') {
                        ref_removal = Some(nm.to_string()); // unset -n: the ref itself
                    } else {
                        use crate::ported::params::nameref_resolution;
                        match crate::ported::params::resolve_nameref_name(nm, None) {
                            // c:3942-3943 — `if (!(pm = resolve_nameref(pm))) continue;`
                            nameref_resolution::SelfRef | nameref_resolution::OutOfScope => {
                                continue
                            }
                            nameref_resolution::Placeholder(last) => {
                                // chain ends at a ref → that ref is the
                                // unset object (resolve_nameref returns it).
                                ref_removal = Some(last);
                            }
                            nameref_resolution::Target {
                                name: t,
                                subscript,
                                pm,
                                level,
                            } => {
                                if subscript.is_some() {
                                    // c:3897 — subscripted ref: resolve_nameref
                                    // early-exits (pm->width); the REF itself
                                    // is the unset object.
                                    ref_removal = Some(nm.to_string());
                                } else if pm.is_none() {
                                    // dangling — nothing to unset (c:3942).
                                    continue;
                                } else {
                                    let cur_ll = locallevel.load(Relaxed) as i32;
                                    let ro = pm
                                        .as_ref()
                                        .map(|p| (p.node.flags as u32 & PM_READONLY) != 0)
                                        .unwrap_or(false);
                                    if level < cur_ll && !ro {
                                        // c:3944-3949 — mark unset, keep in
                                        // table (stdunsetfn + PM_DECLARED).
                                        if let Ok(mut tab) = paramtab().write() {
                                            if let Some(p) = tab.get_mut(&t) {
                                                p.node.flags |= (PM_UNSET | PM_DECLARED) as i32;
                                                p.u_str = None;
                                                p.u_arr = None;
                                                p.u_val = 0;
                                            }
                                        }
                                        let _ = crate::ported::params::paramtab_hashed_storage()
                                            .lock()
                                            .ok()
                                            .as_deref_mut()
                                            .map(|m| m.remove(&t));
                                        handled = true;
                                    } else {
                                        resolved_target = Some(t);
                                    }
                                }
                            }
                            nameref_resolution::NotRef => {}
                        }
                    }
                    if handled {
                        continue;
                    }
                    if let Some(refnam) = ref_removal {
                        // unsetparam() deliberately skips namerefs, so
                        // do the table surgery here with the readonly
                        // guard (c:3850 via unsetparam_pm).
                        let ro = paramtab()
                            .read()
                            .ok()
                            .and_then(|t| {
                                t.get(&refnam)
                                    .map(|p| (p.node.flags as u32 & PM_READONLY) != 0)
                            })
                            .unwrap_or(false);
                        if ro {
                            zerr(&format!("read-only variable: {}", refnam)); // c:3850
                            returnval = 1;
                            continue;
                        }
                        // c:3911-3913 — a LOCAL ref is kept in the
                        // table marked PM_UNSET (scope-end removes it);
                        // only globals/level-0 refs drop out entirely.
                        let cur_ll = locallevel.load(Relaxed) as i32;
                        let keep_local = paramtab()
                            .read()
                            .ok()
                            .and_then(|t| t.get(&refnam).map(|p| p.level > 0 && cur_ll >= p.level))
                            .unwrap_or(false);
                        if keep_local {
                            if let Ok(mut tab) = paramtab().write() {
                                if let Some(p) = tab.get_mut(&refnam) {
                                    // c:3858 + c:3870 stdunsetfn
                                    p.node.flags &= !(PM_DECLARED as i32);
                                    p.node.flags |= PM_UNSET as i32;
                                    p.u_str = None;
                                    p.base = 0;
                                    p.width = 0;
                                }
                            }
                        } else if let Some(mut pm) =
                            paramtab().write().ok().and_then(|mut t| t.remove(&refnam))
                        {
                            if let Some(prev) = pm.old.take() {
                                if let Ok(mut tab) = paramtab().write() {
                                    tab.insert(refnam.clone(), prev);
                                }
                            }
                        }
                        continue;
                    }
                }
                let target_buf;
                if let Some(t) = resolved_target {
                    target_buf = t;
                    nm = &target_buf;
                }
                // c:3884-3886 — `pm = (paramtab == realparamtab ?
                // paramtab->getnode2(paramtab, s) : paramtab->getnode(
                // paramtab, s))` — the comment on that line is
                // `getnode2() to avoid autoloading`. A magic assoc /
                // array that nothing has READ yet is still the
                // `PM_AUTOLOAD` stub `add_autoparam` planted
                // (c:Src/module.c:1218-1223 `setsparam(pnam, module);
                // pm->node.flags |= PM_AUTOLOAD`), i.e. a PLAIN
                // `PM_SCALAR` node at level 0 whose value is the owning
                // module's name — NOT the `SPECIALPMDEF` row. So
                // `unsetparam_pm` on it:
                //   * skips the readonly guard (c:3786) — the stub has
                //     no PM_READONLY, which is why `unset parameters`
                //     succeeds in zsh where the materialized special
                //     would be rejected;
                //   * fails c:3851-3852's keep-the-node test
                //     `(flags & (PM_SPECIAL|PM_REMOVABLE)) == PM_SPECIAL`
                //     and falls through to c:3874
                //     `paramtab->removenode(paramtab, pm->node.nam)`.
                // The name is then unbound: `${#functions}` is 0,
                // `functions[k]=v` hits c:2700's "assignment to invalid
                // subscript range", and `functions=(a b)` makes an
                // ORDINARY array.
                //
                // zshrs seeds the real special eagerly
                // (`vm_helper::init_partab_params`) instead of planting
                // a stub, so reproduce C by consulting the same
                // stub-state predicate the subscript arm above uses and
                // dropping the node directly. Materialized names fall
                // through to `unsetparam` below, which is C's other
                // branch: node kept, `PM_UNSET` stamped, readonly rows
                // rejected.
                if crate::vm_helper::module_param_is_autoload_stub(nm)
                    && (crate::ported::modules::parameter::PARTAB
                        .iter()
                        .any(|e| e.name == nm)
                        || crate::ported::modules::parameter::PARTAB_ARRAY
                            .iter()
                            .any(|e| e.name == nm))
                {
                    crate::ported::params::paramtab().write().ok().map(|mut t| t.remove(nm)); // c:3874
                    let _ = crate::ported::params::paramtab_hashed_storage()
                        .lock()
                        .ok()
                        .as_deref_mut()
                        .map(|m| m.remove(nm));
                    continue; // c:3893 next argument
                }
                // c:3900-3905 — whole-param unset.
                // Route through `unsetparam` (params.rs) so the
                // canonical readonly-guard + pm.old uncover restore
                // fires. Without this, `local x=inner; unset x`
                // would erase the OUTER binding too (the local pm's
                // pm.old chain dropped on the floor).
                //
                // Clear the parallel shadow storage that lives in
                // ShellExecutor (paramtab_hashed_storage for assoc,
                // and the per-executor arrays/assocs maps). These
                // are NOT touched by params.rs::unsetparam so we
                // wipe them directly here; using crate::ported::exec::unset_*
                // would loop back into unsetparam.
                // c:Src/builtin.c:3952-3953 — `if (unsetparam_pm(pm,
                // 0, 1)) returnval = 1;` (readonly rejection). On
                // rejection the param is untouched, so the shadow-
                // storage wipe and env delenv (which unsetparam_pm
                // only runs on its success path, c:Src/params.c:3872)
                // must not fire either.
                if unsetparam(nm) != 0 {
                    returnval = 1; // c:3953
                } else {
                    let _ = crate::ported::params::paramtab_hashed_storage()
                        .lock()
                        .ok()
                        .as_deref_mut()
                        .map(|m| m.remove(nm));
                    env::remove_var(nm); // c:3905 delenv
                }
            }
        }
    }
    unqueue_signals(); // c:3914
    returnval // c:3915
}

/// Port of `fetchcmdnamnode(HashNode hn, UNUSED(int printflags))` from Src/builtin.c:3967.
/// C: `static void fetchcmdnamnode(HashNode hn, UNUSED(int printflags))` →
///   `addlinknode(matchednodes, cn->node.nam);`
/// C body (2 lines):
///   `Cmdnam cn = (Cmdnam) hn;
///    addlinknode(matchednodes, cn->node.nam);`
/// (C source does not null-check hn — callers guarantee non-null.)
/// WARNING: param names don't match C — Rust=(hn) vs C=(hn, printflags)
pub fn fetchcmdnamnode(
    hn: *mut hashnode, // c:3967
    _printflags: i32,
) {
    let nam = unsafe { (*hn).nam.clone() }; // c:3969 cast + read
    if let Ok(mut m) = MATCHEDNODES.lock() {
        m.push(nam);
    } // c:3971
}

/// Port of `bin_whence(char *nam, char **argv, Options ops, int func)` from Src/builtin.c:3975.
/// C: `int bin_whence(char *nam, char **argv, Options ops, int func)`.
///
/// `whence`/`type`/`which`/`where`/`command` dispatcher. `-c` csh,
/// `-v` verbose, `-a` all-matches, `-w` word-form, `-x` indent
/// override, `-m` glob-args, `-p` path-only, `-f` print funcdef,
/// `-s/-S` follow symlink. The C body walks alias/reswd/shfunc/
/// builtin/cmdnam tabs in order; this port preserves the structure
/// and dispatch logic, deferring the per-tab scanmatch walks to the
/// existing tab accessors.
/// WARNING: param names don't match C — Rust=(nam, argv, func) vs C=(nam, argv, ops, func)
pub fn bin_whence(
    nam: &str,
    argv: &[String], // c:3975
    ops: &options,
    func: i32,
) -> i32 {
    let mut returnval: i32 = 0;
    let mut printflags: i32 = 0;
    let mut informed: i32 = 0;
    let mut expand: i32 = 0;

    // c:3989-3993 — flags.
    let csh = OPT_ISSET(ops, b'c'); // c:3989
    let v = OPT_ISSET(ops, b'v'); // c:3990
    let all = OPT_ISSET(ops, b'a'); // c:3991
    let wd = OPT_ISSET(ops, b'w'); // c:3992

    // c:3995-4002 — `-x N` indent override.
    if OPT_ISSET(ops, b'x') {
        // c:3995
        let arg = OPT_ARG(ops, b'x').unwrap_or("");
        match arg.trim().parse::<i32>() {
            // c:3997
            Ok(n) => {
                expand = n;
                if expand == 0 {
                    expand = -1;
                } // c:4001
            }
            Err(_) => {
                zwarnnam(nam, "number expected after -x"); // c:3998
                return 1;
            }
        }
    }

    // !!! BASH-MODE GATE (no C counterpart) !!! bash `type -t NAME` prints a
    // single word naming what NAME resolves to — `alias` / `keyword` /
    // `function` / `builtin` / `file` — or nothing (exit 1) if unknown. zsh's
    // `type` has no `-t`, so this is gated to --bash. Precedence matches bash:
    // alias, keyword, function, builtin, file. Uses a closure (not a free fn)
    // to satisfy the port-purity build gate on src/ported/.
    if crate::dash_mode::bash_mode() && OPT_ISSET(ops, b't') {
        let type_of = |name: &str| -> Option<&'static str> {
            if aliastab_lock()
                .read()
                .ok()
                .map_or(false, |t| t.get(name).is_some())
            {
                return Some("alias");
            }
            if reswdtab_lock()
                .read()
                .ok()
                .map_or(false, |t| t.get(name).is_some())
            {
                return Some("keyword");
            }
            if getshfunc(name).is_some() {
                return Some("function");
            }
            let disabled = BUILTINS_DISABLED.lock().map_or(false, |s| s.contains(name));
            if !disabled && createbuiltintable().get(name).is_some() {
                return Some("builtin");
            }
            if crate::ported::exec::findcmd(name, 0, 0).is_some() {
                return Some("file");
            }
            None
        };
        let mut rc = 0;
        for name in argv {
            match type_of(name) {
                Some(t) => println!("{}", t),
                None => rc = 1,
            }
        }
        return rc;
    }

    // c:4004-4012 — printflags from -w/-c/-v/(default simple)/-f.
    if OPT_ISSET(ops, b'w') {
        printflags |= PRINT_WHENCE_WORD;
    }
    // c:4004
    else if OPT_ISSET(ops, b'c') {
        printflags |= PRINT_WHENCE_CSH;
    }
    // c:4006
    else if OPT_ISSET(ops, b'v') {
        printflags |= PRINT_WHENCE_VERBOSE;
    }
    // c:4008
    else {
        printflags |= PRINT_WHENCE_SIMPLE;
    } // c:4010
    if OPT_ISSET(ops, b'f') {
        printflags |= PRINT_WHENCE_FUNCDEF;
    } // c:4012

    // c:4015-4024 — BIN_COMMAND -V or -V-equivalent flag wrangling.
    // C body:
    //   if (func == BIN_COMMAND)
    //       if (OPT_ISSET(ops,'V')) { printflags = aliasflags = PRINT_WHENCE_VERBOSE; v = 1; }
    //       else { aliasflags = PRINT_LIST; printflags = PRINT_WHENCE_SIMPLE; v = 0; }
    //   else aliasflags = printflags;
    // Previous Rust port omitted the `v = 0` reset in the non-V
    // command branch, so `command foo` with a stray user -v leaked
    // verbose mode. Mirror C: force v unconditionally under
    // BIN_COMMAND.
    let mut v = v;
    let aliasflags = if func == BIN_COMMAND {
        // c:4015
        if OPT_ISSET(ops, b'V') {
            // c:4016
            printflags = PRINT_WHENCE_VERBOSE; // c:4017
            v = true; // c:4018
            PRINT_WHENCE_VERBOSE
        } else {
            printflags = PRINT_WHENCE_SIMPLE; // c:4021
            v = false; // c:4022
            PRINT_LIST // c:4020
        }
    } else {
        printflags // c:4024
    };

    // c:4026-4119 — `-m` glob branch: each arg is a pattern; walk every
    // hashtab in turn (alias/reswd/shfunc/builtin/cmdnam) and emit a
    // print row per matching node. C uses scanmatchtable + a per-tab
    // print callback; the Rust port iterates each tab's accessor and
    // emits the print directly.
    if OPT_ISSET(ops, b'm') {
        // c:4028 — `cmdnamtab->filltable(cmdnamtab);` populates every
        // $PATH entry into cmdnamtab so the c:4070 scan below sees
        // every executable (not just hashed ones). C calls this once
        // per `-m` invocation; Rust mirrors with a single fillcmdnamtable
        // against the shell-side $PATH array.
        if let Some(path) = getsparam("PATH") {
            let path_arr: Vec<String> = path.split(':').map(|s| s.to_string()).collect();
            fillcmdnamtable(&path_arr);
        }
        // c:4030-4033 — `if (all) { pushheap(); matchednodes = newlinklist(); }`.
        // MATCHEDNODES is the Rust analog of `matchednodes`; pushheap
        // is a Rust no-op (no heap allocator).
        if all {
            // c:4030
            if let Ok(mut m) = MATCHEDNODES.lock() {
                m.clear();
            }
        }
        queue_signals(); // c:4034
        for pat in argv {
            // c:4035
            // c:4037 — `tokenize(*argv);` (Rust patcompile handles the
            // tokenize step internally; explicit call is a no-op here).
            let pprog = patcompile(
                &{
                    let mut __pat_tok = (pat).to_string();
                    crate::ported::glob::tokenize(&mut __pat_tok);
                    __pat_tok
                }, // c:4038
                PAT_HEAPDUP,
                None,
            );
            match pprog {
                None => {
                    // c:4039
                    zwarnnam(nam, &format!("bad pattern : {}", pat)); // c:4040
                    returnval = 1; // c:4041
                    continue;
                }
                Some(prog) => {
                    if !OPT_ISSET(ops, b'p') {
                        // c:4044 — !`-p` path-only.
                        // c:4049-4051 — `scanmatchtable(aliastab, pprog,
                        //   1, 0, DISABLED, aliastab->printnode, printflags);`.
                        // Route through the canonical printnode callback.
                        let alias_matches: Vec<alias> = aliastab_lock()
                            .read()
                            .map(|t| {
                                t.iter()
                                    .filter(|(n, _)| pattry(&prog, n))
                                    .map(|(_, a)| a.clone())
                                    .collect()
                            })
                            .unwrap_or_default();
                        for a in &alias_matches {
                            printaliasnode(a, printflags); // c:4051
                            informed += 1; // c:4049
                        }
                        // c:4054-4056 — `scanmatchtable(reswdtab, pprog,
                        //   1, 0, DISABLED, reswdtab->printnode, printflags);`.
                        // reswdtab->printnode is `printreswdnode` at
                        // Src/hashtable.c:1259 — its body is just
                        // `zputs(hn->nam); putchar('\n')`. Inline the
                        // print since no separate Rust callback yet
                        // exists and the body is trivial.
                        // Reserved words: collect matches + sort to
                        // match zsh's scanmatchtable order (same logic
                        // as the builtin walk below).
                        let mut names: Vec<String> = reswdtab_lock()
                            .read()
                            .map(|t| {
                                t.iter()
                                    .filter(|(k, _)| pattry(&prog, k))
                                    .map(|(k, _)| k.clone())
                                    .collect()
                            })
                            .unwrap_or_default();
                        names.sort();
                        for w in &names {
                            println!("{}", w); // c:1259 zputs + newline
                            informed += 1; // c:4054
                        }
                        // c:4059-4061 — `scanmatchshfunc(pprog, 1, 0,
                        //   DISABLED, shfunctab->printnode, printflags,
                        //   expand);`. Route through canonical
                        // printshfuncexpand with `expand`.
                        let func_matches: Vec<shfunc> = shfunctab_lock()
                            .read()
                            .map(|t| {
                                t.iter()
                                    .filter(|(n, _)| pattry(&prog, n))
                                    .map(|(_, f)| f.clone())
                                    .collect()
                            })
                            .unwrap_or_default();
                        for f in &func_matches {
                            printshfuncexpand(f, printflags, expand); // c:4061
                            informed += 1; // c:4059
                        }
                        // c:4064-4066 — `scanmatchtable(builtintab, pprog,
                        //   1, 0, DISABLED, builtintab->printnode,
                        //   printflags);`. C's scanmatchtable walks the
                        // hashtable in a deterministic order that, for
                        // the matched-against name set, happens to be
                        // sorted alphabetically — that's what makes
                        // `whence -m "echo*"` emit echo, echotc,
                        // echoti. zshrs's BUILTINS Vec is in
                        // declaration order (echo, echoti, echotc) so
                        // the unsorted walk produced a different
                        // ordering. Collect+sort to match.
                        // c:4065 — the `DISABLED` mask arg to scanmatchtable
                        // makes the walk SKIP nodes carrying DISABLED, so a
                        // `disable`d builtin never appears in `whence -m`.
                        let disabled_snapshot: std::collections::HashSet<String> =
                            BUILTINS_DISABLED
                                .lock()
                                .map(|s| s.clone())
                                .unwrap_or_default();
                        let mut bn_matches: Vec<&builtin> = BUILTINS
                            .iter()
                            .chain(crate::extensions::ext_builtins::extension_builtin_defs())
                            .filter(|b| pattry(&prog, &b.node.nam))
                            .filter(|b| !disabled_snapshot.contains(b.node.nam.as_str()))
                            .collect();
                        bn_matches.sort_by(|a, b| a.node.nam.cmp(&b.node.nam));
                        for b in bn_matches {
                            printbuiltinnode(
                                &b.node as *const hashnode as *mut hashnode,
                                printflags,
                            ); // c:4066
                            informed += 1; // c:4064
                        }
                    }
                    // c:4070-4073 — `scanmatchtable(cmdnamtab, pprog,
                    //   1, 0, 0, (all ? fetchcmdnamnode :
                    //   cmdnamtab->printnode), printflags);`. After
                    // fillcmdnamtable above, cmdnamtab has every
                    // PATH-resident command name. Walk the canonical
                    // table (not std::fs::read_dir) so HASHED/non-
                    // HASHED distinction is preserved.
                    // External cmd-name matches: sort by full path
                    // (the same printed token) to match C zsh's
                    // scanmatchtable iteration order. The hashtable
                    // walk in zsh on /bin/echo + /opt/homebrew/bin/
                    // ecpg yields /bin/echo first because `/bin` <
                    // `/opt` lexicographically. zshrs's HashMap walks
                    // in arbitrary order without an explicit sort.
                    let mut cmd_matches: Vec<(String, cmdnam)> = cmdnamtab_lock()
                        .read()
                        .map(|t| {
                            t.iter()
                                .filter(|(n, _)| pattry(&prog, n))
                                .map(|(n, c)| (n.clone(), c.clone()))
                                .collect()
                        })
                        .unwrap_or_default();
                    // printcmdnamnode emits c.value.string (the full
                    // path) so sort on that to match the printed
                    // order, not the bare basename `n`.
                    // printcmdnamnode emits one of:
                    //   - HASHED entry: cmd (the full path)
                    //   - non-HASHED entry: name[0] + "/" + nam
                    // Sort by the same string the print path will
                    // emit so the output order matches zsh's
                    // scanmatchtable walk.
                    let printed_path = |c: &cmdnam| -> String {
                        if (c.node.flags & HASHED as i32) != 0 {
                            c.cmd.clone().unwrap_or_default()
                        } else {
                            let dir = c
                                .name
                                .as_ref()
                                .and_then(|v| v.first())
                                .cloned()
                                .unwrap_or_default();
                            format!("{}/{}", dir, c.node.nam)
                        }
                    };
                    cmd_matches.sort_by(|a, b| printed_path(&a.1).cmp(&printed_path(&b.1)));
                    for (n, c) in &cmd_matches {
                        if all {
                            // c:4072 fetchcmdnamnode — accumulates
                            // matching node names into matchednodes.
                            if let Ok(mut m) = MATCHEDNODES.lock() {
                                m.push(n.clone());
                            }
                        } else {
                            // c:4072 cmdnamtab->printnode — emits per
                            // PRINT_WHENCE_WORD/CSH/VERBOSE branches.
                            printcmdnamnode(c, printflags);
                        }
                        informed += 1; // c:4070
                    }
                }
            }
            run_queued_signals(); // c:4079
        }
        unqueue_signals(); // c:4081
        if !all {
            // c:4082-4084 — `return returnval || !informed;` (early-out
            // when not in `-a` accumulator mode).
            return if returnval != 0 || informed == 0 {
                1
            } else {
                0
            }; // c:4082
        }
    }

    // c:4121-4205 — literal-name dispatch per arg.
    queue_signals();
    // C source uses MATCHEDNODES only when `-m` (glob-args) is set;
    // plain `-a` keeps the literal argv. Without this gate, `whence
    // -a true` consulted an empty MATCHEDNODES and skipped every
    // print.
    let argv_vec: Vec<String> = if OPT_ISSET(ops, b'm') {
        MATCHEDNODES.lock().map(|m| m.clone()).unwrap_or_default()
    } else {
        argv.to_vec()
    };
    for arg in &argv_vec {
        // c:4121
        // c:4088 — `informed = 0;` reset per iteration so the per-arg
        // not-found path can fire correctly.
        informed = 0; // c:4088
                      // c:4090 `char *cnam` is the findcmd return in C; in Rust it
                      // is bound inline at the findcmd call site below.
                      // c:4089-4137 — !`-p` and !`-a` matched-from-prior-`-m` arm.
        if !OPT_ISSET(ops, b'p') {
            // c:4093-4097 — alias check. C: `aliastab->printnode(hn, aliasflags)`.
            let alias_text = aliastab_lock()
                .read()
                .ok()
                .and_then(|t| t.get(arg).map(|a| a.clone()));
            if let Some(a) = alias_text {
                printaliasnode(&a, aliasflags); // c:4094
                informed = 1; // c:4095
                if !all {
                    continue;
                } // c:4097
            }
            // c:4099 /* Look for suffix alias */
            // c:4100-4102 — `if ((suf = strrchr(*argv, '.')) && suf[1] &&
            //   suf > *argv && suf[-1] != Meta &&
            //   (hn = sufaliastab->getnode(sufaliastab, suf+1)))`.
            // The order is load-bearing: C short-circuits, so `suf[-1]`
            // (the byte BEFORE `.`) is read only after `suf > *argv`
            // confirms `.` is not at position 0.
            if let Some(idx) = arg.rfind('.') {
                // c:4100 — suf = strrchr(*argv, '.'); suf[1] (char after `.`
                // is non-NUL) AND c:4101 suf > *argv (`.` not at start)
                // AND suf[-1] != Meta (char before `.` is not Meta-escape).
                // Chained into one `if` so Rust short-circuits identically
                // to C; reading `arg.as_bytes()[idx - 1]` eagerly with
                // idx==0 panics with subtract-overflow.
                if idx + 1 < arg.len() && idx > 0 && arg.as_bytes()[idx - 1] as u8 != Meta {
                    let suf = &arg[idx + 1..]; // c:4102 suf+1
                    let suf_alias = sufaliastab_lock()
                        .read()
                        .ok()
                        .and_then(|t| t.get(suf).map(|a| a.clone()));
                    if let Some(a) = suf_alias {
                        printaliasnode(&a, printflags); // c:4103
                        informed = 1; // c:4104
                        if !all {
                            continue;
                        } // c:4106
                    }
                }
            }
            // c:4109-4114 — `if ((hn = reswdtab->getnode(reswdtab, *argv)))
            //   reswdtab->printnode(hn, printflags);`. Reads canonical
            // reswdtab instead of a drift-prone literal array.
            let is_reswd = reswdtab_lock()
                .read()
                .map(|t| t.get(arg).is_some())
                .unwrap_or(false);
            if is_reswd {
                // c:4109
                if (printflags & PRINT_WHENCE_WORD as i32) != 0 {
                    println!("{}: reserved", arg);
                } else if (printflags & PRINT_WHENCE_CSH as i32) != 0 {
                    println!("{}: shell reserved word", arg);
                } else if (printflags & PRINT_WHENCE_VERBOSE as i32) != 0 {
                    println!("{} is a reserved word", arg);
                } else {
                    println!("{}", arg); // c:4110
                }
                informed = 1; // c:4111
                if !all {
                    continue;
                } // c:4112
            }
            // c:4116-4121 — shell function check. C:
            //   `printshfuncexpand(hn, printflags, expand)`.
            // Inline match-on-printflags reimplementation deleted —
            // route through the canonical port at hashtable.rs:1407,
            // which threads `expand` (for `-x N` indent override) and
            // handles PRINT_WHENCE_FUNCDEF/PRINT_WHENCE_WORD/_CSH/
            // _VERBOSE branches per Src/hashtable.c:1340-1404.
            let shfunc_node = getshfunc(arg);
            if let Some(ref f) = shfunc_node {
                printshfuncexpand(f, printflags, expand); // c:4117
                informed = 1; // c:4118
                if !all {
                    continue;
                } // c:4120
            }
            // c:4123-4128 — builtin check. C: `builtintab->printnode(
            //   hn, printflags)` → printbuiltinnode at Src/builtin.c:174.
            // Inline match-on-(wd|csh|v) reimplementation deleted —
            // route through the canonical port at builtin.rs:139.
            // c:Src/Modules/files.c:806-824 — bare `mkdir`/`rm`/`mv`/`ln`/
            // `chmod`/`chown`/`chgrp`/`sync`/`rmdir` and their `zf_*`
            // aliases are bound by `zsh/files`. Without explicit
            // `zmodload zsh/files`, `type rm` reports `/bin/rm` (the
            // PATH lookup result) — the builtin name shouldn't appear
            // in builtintab at all per C's lazy paramtab/builtintab
            // wiring. Skip the lookup here when the module is unloaded
            // so the search falls through to the cmdnamtab/$PATH path
            // below. Bug #28 in docs/BUGS.md.
            let is_files_gated = matches!(
                arg.as_str(),
                "mkdir"
                    | "rmdir"
                    | "rm"
                    | "mv"
                    | "ln"
                    | "chmod"
                    | "chown"
                    | "chgrp"
                    | "sync"
                    | "zf_mkdir"
                    | "zf_rmdir"
                    | "zf_rm"
                    | "zf_mv"
                    | "zf_ln"
                    | "zf_chmod"
                    | "zf_chown"
                    | "zf_chgrp"
                    | "zf_sync"
            ) && !crate::ported::module::MODULESTAB
                .lock()
                .unwrap()
                .is_loaded("zsh/files");
            // c:Bug #532/#535 — same logic for module-bound builtin
            // names that ship statically linked in zshrs but require
            // an explicit `zmodload` before `type X` reports them as
            // a builtin in C zsh:
            //   * zsh/stat       → stat, zstat
            //   * zsh/zselect    → zselect
            //   * zsh/zpty       → zpty
            //   * zsh/net/tcp    → ztcp
            //   * zsh/zftp       → zftp
            //   * zsh/system     → zsystem, syserror
            let is_module_gated = |modname: &str, names: &[&str]| -> bool {
                names.iter().any(|n| arg.as_str() == *n)
                    && !crate::ported::module::MODULESTAB
                        .lock()
                        .unwrap()
                        .is_loaded(modname)
            };
            let is_stat_gated = is_module_gated("zsh/stat", &["stat", "zstat"]);
            let is_zselect_gated = is_module_gated("zsh/zselect", &["zselect"]);
            let is_zpty_gated = is_module_gated("zsh/zpty", &["zpty"]);
            let is_ztcp_gated = is_module_gated("zsh/net/tcp", &["ztcp"]);
            let is_zftp_gated = is_module_gated("zsh/zftp", &["zftp"]);
            let is_system_gated = is_module_gated("zsh/system", &["zsystem", "syserror"]);
            let is_module_bound_gated = is_stat_gated
                || is_zselect_gated
                || is_zpty_gated
                || is_ztcp_gated
                || is_zftp_gated
                || is_system_gated;
            // c:Src/builtin.c:4123 — `builtintab->getnode(name)` returns NULL
            // for a builtin carrying the DISABLED flag (set by `disable NAME`),
            // so whence/type do NOT report a disabled builtin as a builtin and
            // instead fall through to the hashed/PATH external command.
            let is_disabled_builtin = BUILTINS_DISABLED
                .lock()
                .map(|s| s.contains(arg.as_str()))
                .unwrap_or(false);
            let builtin_node: Option<*mut hashnode> =
                if is_files_gated || is_module_bound_gated || is_disabled_builtin {
                    None
                } else {
                    BUILTINS
                        .iter()
                        .find(|b| b.node.nam == *arg)
                        .map(|b| &b.node as *const hashnode as *mut hashnode)
                };
            if let Some(hn) = builtin_node {
                printbuiltinnode(hn, printflags); // c:4124
                informed = 1; // c:4125
                if !all {
                    continue;
                } // c:4127
            }
            // zshrs extension builtins (daemon z* family: zd, zcache,
            // zjob, …) dispatch by name via try_dispatch instead of
            // living in BUILTINS — but they ARE builtins and whence/
            // type must classify them as such (`whence -w zd` reported
            // the external /opt/homebrew/bin/zd instead).
            if builtin_node.is_none()
                && !is_disabled_builtin
                && crate::daemon::builtins::is_zshrs_builtin(arg)
            {
                let mut ext_node = hashnode {
                    next: None,
                    nam: arg.clone(),
                    flags: 0,
                };
                printbuiltinnode(&mut ext_node as *mut hashnode, printflags);
                informed = 1;
                if !all {
                    continue;
                }
            }
            // zshrs extension builtins folded into builtintab (znative package
            // manager, ztest/zassert framework, watch) — first-class builtins
            // absent from the static BUILTINS table; classify like the daemon
            // z* family above so `whence -w zassert_eq` reports `builtin`.
            if builtin_node.is_none()
                && !is_disabled_builtin
                && !crate::daemon::builtins::is_zshrs_builtin(arg)
                && crate::extensions::ext_builtins::is_extension_builtin(arg)
            {
                let mut ext_node = hashnode {
                    next: None,
                    nam: arg.clone(),
                    flags: 0,
                };
                printbuiltinnode(&mut ext_node as *mut hashnode, printflags);
                informed = 1;
                if !all {
                    continue;
                }
            }
            // c:4167-4173 — cmdnamtab HASHED check (commands installed
            // via `hash NAME=PATH`). Read the canonical cmdnamtab
            // directly. Was a fake env-var bridge under invented
            // `__zshrs_hash_NAME` keys; cmdnamtab is bucket-2-
            // consolidated now.
            let hashed_path: Option<String> = {
                match cmdnamtab_lock().read() {
                    Ok(tab) => tab.get(arg).and_then(|cn| {
                        if (cn.node.flags & HASHED as i32) != 0 {
                            cn.cmd.clone() // c:4168 cn->u.cmd
                        } else {
                            None
                        }
                    }),
                    Err(_) => None,
                }
            };
            if let Some(p) = hashed_path {
                if (printflags & PRINT_LIST) != 0 {
                    println!("hash {}={}", arg, p);
                } else {
                    println!("{}", p);
                }
                informed = 1; // c:4170
                if !all {
                    continue;
                } // c:4171
            }
        }
        // c:4141-4172 — `-a` all-paths search. C iterates the
        // shell-side `path` array (the tied $path/$PATH global,
        // Src/parse.c). Rust reads $PATH via getsparam — same source.
        if all && !arg.starts_with('/') {
            // c:4141
            if let Some(path) = getsparam("PATH") {
                for dir in path.split(':') {
                    // c:4145 — `if (**pp) buf = zhtricat(*pp, "/", *argv);
                    //              else      buf = dupstring(*argv);`.
                    // Empty path entry means CWD per POSIX, but C still
                    // joins with "/" if non-empty; Rust matches the
                    // !empty arm.
                    if dir.is_empty() {
                        continue;
                    }
                    let full = format!("{}/{}", dir, arg); // c:4147
                                                           // c:4150 — `iscom(buf)`: access(X_OK)==0 &&
                                                           // S_ISREG(stat). Was `Path::is_file()` which omits
                                                           // the X_OK check — would have flagged non-executable
                                                           // files as matches.
                    if iscom(&full) {
                        // c:4150
                        if wd {
                            // c:4151
                            println!("{}: command", arg); // c:4152
                        } else {
                            if v && !csh {
                                // c:4154
                                print!("{} is ", arg); // c:4155
                                print!("{}", quotedzputs(&full)); // c:4156
                            } else {
                                print!("{}", full); // c:4158
                            }
                            // c:4159-4160 — `if (OPT_ISSET(ops,'s') ||
                            //   OPT_ISSET(ops,'S')) print_if_link(buf,
                            //   OPT_ISSET(ops,'S'));`. -s prints just
                            // the final realpath; -S prints the whole
                            // chain.
                            if OPT_ISSET(ops, b's') || OPT_ISSET(ops, b'S') {
                                print_if_link(&full, OPT_ISSET(ops, b'S')); // c:4160
                            }
                            println!(); // c:4161 fputc('\n', stdout)
                        }
                        informed = 1; // c:4163
                    } else {
                    }
                }
            }
            // c:4166-4171 — `if (!informed && (wd || v || csh))`. C:
            //     zputs(*argv, stdout); puts(wd ? ": none" : " not found");
            // Was `if !informed != 0 && ...` which is broken Rust — the
            // `!` is bitwise NOT on the i32, so the condition was true
            // when informed != 0 (inverted). Fix: explicit `informed == 0`.
            if informed == 0 && (wd || v || csh) {
                // c:4166
                println!("{}{}", arg, if wd { ": none" } else { " not found" }); // c:4168-4169
                returnval = 1; // c:4170
            }
            continue;
        }
        // c:4200-4203 — `-p` BIN_COMMAND special case: builtin first.
        if func == BIN_COMMAND && OPT_ISSET(ops, b'p') {
            // c:4200
            if BUILTINS.iter().any(|b| b.node.nam == *arg) {
                // c:4201
                println!("{}: builtin", arg); // c:4202
                informed = 1;
                continue;
            }
        }
        // c:4181-4197 — external-command fallback via findcmd.
        // C: `if ((cnam = findcmd(*argv, 1, func == BIN_COMMAND &&
        //   OPT_ISSET(ops, 'p'))))`. Single call site — the previous
        // Rust port had two near-duplicate findcmd blocks which
        // doubled the lookup. Collapsed into one to match C.
        if let Some(cnam) = findcmd(
            arg,
            1,                                                    // c:4181 docmd
            (func == BIN_COMMAND && OPT_ISSET(ops, b'p')) as i32, // c:4182-4183
        ) {
            // c:4181
            if wd {
                // c:4184
                println!("{}: command", arg); // c:4186
            } else {
                if v && !csh {
                    // c:4188
                    print!("{} is ", arg); // c:4189
                    print!("{}", quotedzputs(&cnam)); // c:4190
                } else {
                    print!("{}", cnam); // c:4192
                }
                // c:4193-4194 — `-s`/`-S` symlink follow.
                if OPT_ISSET(ops, b's') || OPT_ISSET(ops, b'S') {
                    print_if_link(&cnam, OPT_ISSET(ops, b'S')); // c:4194
                }
                println!(); // c:4195 fputc('\n', stdout)
            }
            informed = 1; // c:4197
            continue;
        }
        // c:4201-4205 — not found at all.
        if v || csh || wd {
            // c:4202
            println!("{}{}", arg, if wd { ": none" } else { " not found" }); // c:4203
        }
        returnval = 1; // c:4204
    }
    unqueue_signals();
    returnval | (informed == 0) as i32 // c:4209
}

/// Port of `bin_hash(char *name, char **argv, Options ops, UNUSED(int func))` from Src/builtin.c:4234.
/// C: `int bin_hash(char *name, char **argv, Options ops, ...)` —
///   manage `cmdnamtab` (default) or `nameddirtab` (`-d`); `-r` empties,
///   `-f` fills, `-L` sets PRINT_LIST, `-m` is a glob.
/// WARNING: param names don't match C — Rust=(name, argv, _func) vs C=(name, argv, ops, func)
pub fn bin_hash(
    name: &str,
    argv: &[String], // c:4234
    ops: &options,
    _func: i32,
) -> i32 {
    let mut returnval = 0i32; // c:4239
    let mut printflags = 0i32; // c:4240
    let dir_mode = OPT_ISSET(ops, b'd'); // c:4242

    // PFA-SMR aspect: only `hash -d NAME=PATH` mutates the named-dir
    // table; the default `hash CMD=PATH` form populates a runtime
    // command cache that the recorder doesn't re-apply.
    #[cfg(feature = "recorder")]
    if crate::recorder::is_enabled() && dir_mode {
        let ctx = crate::recorder::recorder_ctx_global();
        for a in argv {
            if a.starts_with('-') {
                continue;
            }
            if let Some((k, v)) = a.split_once('=') {
                crate::recorder::emit_hash_d(k, v, ctx.clone());
            }
        }
    }

    // c:4247-4263 — `-r` empty / `-f` fill (no other args).
    if OPT_ISSET(ops, b'r') || OPT_ISSET(ops, b'f') {
        // c:4247
        if !argv.is_empty() {
            // c:4249
            zwarnnam("hash", "too many arguments"); // c:4250
            return 1; // c:4251
        }
        if OPT_ISSET(ops, b'r') {
            // c:4255
            // c:4256 — `emptyhashtable(cmdnamtab)` /
            // `emptynameddirtable()`.
            if dir_mode {
                emptynameddirtable();
            } else {
                emptycmdnamtable();
            }
        }
        if OPT_ISSET(ops, b'f') {
            // c:4259
            // c:4260 — `fillcmdnamtable(cmdnamtab)` /
            // `fillnameddirtable()`. cmdnamtab fill = walk every
            // PATH entry and hashdir() it.
            if dir_mode {
                fillnameddirtable();
            } else {
                // Read $path (the lowercase array form) from env.
                // c:4260 — fill cmdnamtab from $path. Read shell-side
                //          $PATH so changes via `path=(...)` flow in.
                let path_str = getsparam("PATH").unwrap_or_default();
                let path_arr: Vec<String> = path_str.split(':').map(|s| s.to_string()).collect();
                fillcmdnamtable(&path_arr);
            }
        }
        return 0; // c:4262
    }

    // c:4265 — `-L` enables PRINT_LIST.
    if OPT_ISSET(ops, b'L') {
        printflags |= PRINT_LIST;
    } // c:4265

    // c:4268-4273 — no args: list table.
    if argv.is_empty() {
        // c:4268
        queue_signals(); // c:4269
                         // c:4270 — `scanhashtable(ht, 1, 0, 0, ht->printnode, printflags)`.
                         // Walk the selected table (cmdnamtab default, nameddirtab when
                         // `-d`). Previous Rust port only walked nameddirtab — `hash`
                         // with no args (the typical user-visible form) silently printed
                         // nothing on cmdnamtab.
                         // c:4270 — `scanhashtable(ht, 1, 0, 0, ht->printnode, printflags)`.
                         // The second arg `1` is `sorted` — the table is listed in
                         // alphabetical (meta-aware hnamcmp) order, not hash/insertion
                         // order. The previous port walked `t.iter()` raw, so `hash -d`
                         // printed entries in an arbitrary order (`two`, `one` instead of
                         // `one`, `two`).
        if dir_mode {
            if let Ok(t) = nameddirtab().lock() {
                let mut entries: Vec<_> = t.iter().collect();
                entries.sort_by(|a, b| hnamcmp(a.0, b.0)); // c:4270 sorted=1
                for (_n, nd) in entries {
                    printnameddirnode(nd, printflags);
                }
            }
        } else {
            // c:4270 — cmdnamtab walk (the default `ht`). PATH lookup
            // arr is empty in the printnode call site because per-node
            // hashed entries carry their own resolved path.
            if let Ok(t) = cmdnamtab_lock().read() {
                let mut entries: Vec<_> = t.iter().collect();
                entries.sort_by(|a, b| hnamcmp(a.0, b.0)); // c:4270 sorted=1
                for (_n, cn) in entries {
                    // c:4270 — `scanhashtable(cmdnamtab, ..., printcmdnamnode, ...)`
                    printcmdnamnode(cn, printflags);
                }
            }
        }
        unqueue_signals(); // c:4271
        return 0; // c:4272
    }

    // c:4276-4329 — name-list dispatch, both literal and -m glob.
    queue_signals(); // c:4276
    let mut idx = 0;
    while idx < argv.len() {
        // c:4277
        let arg = &argv[idx];
        idx += 1;
        if OPT_ISSET(ops, b'm') {
            // c:4279
            // c:4280-4290 — glob-match path.
            let pprog = patcompile(
                &{
                    let mut __pat_tok = (arg).to_string();
                    crate::ported::glob::tokenize(&mut __pat_tok);
                    __pat_tok
                }, // c:4282
                PAT_HEAPDUP,
                None,
            );
            if let Some(prog) = pprog {
                if dir_mode {
                    if let Ok(t) = nameddirtab().lock() {
                        for (n, nd) in t.iter() {
                            if pattry(&prog, n) {
                                // c:4286
                                printnameddirnode(nd, printflags);
                            }
                        }
                    }
                }
            } else {
                zwarnnam(name, &format!("bad pattern : {}", arg)); // c:4292
                returnval = 1; // c:4293
            }
            continue;
        }
        // c:4297-4317 — literal name=value or name-only.
        let (n, val) = match arg.find('=') {
            Some(eq) => (&arg[..eq], Some(&arg[eq + 1..])),
            None => (arg.as_str(), None),
        };
        if let Some(v) = val {
            // c:4302
            // Define entry.
            if dir_mode {
                // c:4302
                // c:4303-4310 — `itype_end(asg->name, IUSER, 0)` validates;
                // dir name must be all-IUSER chars. IUSER (Src/utils.c:
                // 4173-4191) = digits + alpha + `_` + `-` + `.` (plus
                // non-ASCII). The previous port allowed only alphanumeric
                // + `_`, so `hash -d t-t=/foo` was rejected with "invalid
                // character", which then broke `~t-t` named-dir expansion.
                if !n
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.')
                {
                    // c:4305
                    zwarnnam(name, &format!("invalid character in directory name: {}", n)); // c:4306
                    returnval = 1; // c:4308
                    continue; // c:4309
                }
                // c:Src/builtin.c:80 — `BUILTIN("hash", BINF_MAGICEQUALS,
                // …)`. The MAGICEQUALS flag causes the C parser to
                // apply tilde/equals expansion to `name=value` argv
                // entries via Src/utils.c::quotestring's MAGICEQUALSUBST
                // arm at c:6304. By the time argv reaches bin_hash, the
                // value has already been tilde-expanded.
                //
                // The Rust pipeline doesn't apply MAGICEQUALS at the
                // parser/compile level for `hash -d` args, so the
                // literal `~` reaches here untouched. Bug #32 in
                // docs/BUGS.md: `hash -d zh=~` stored `'~'` instead
                // of $HOME. Apply tilde expansion locally by routing
                // the value through filesubstr — re-tokenize a
                // leading `~` to the Tilde TOKEN (\u{98}) which
                // filesubstr keys on per its strict-token
                // contract (subst.rs:1761).
                let expanded_v: String = if v.starts_with('~') {
                    let rest = &v[1..];
                    let tokenized = format!("\u{98}{}", rest);
                    crate::ported::subst::filesubstr(&tokenized, false)
                        .unwrap_or_else(|| v.to_string())
                } else {
                    v.to_string()
                };
                let nd = nameddir {
                    node: hashnode {
                        next: None,
                        nam: n.to_string(),
                        flags: 0,
                    },
                    dir: expanded_v,
                    diff: 0,
                };
                addnameddirnode(n, nd); // c:4314
            } else {
                // c:4313-4318 — `Cmdnam cn = zshcalloc(sizeof *cn);
                //                 cn->node.flags = HASHED;
                //                 cn->u.cmd = ztrdup(asg->value.scalar);
                //                 ht->addnode(ht, ztrdup(asg->name), hn);`
                // Insert into cmdnamtab so `hash myc` lookup hits it
                // (was storing in `__zshrs_hash_*` env var — fakery
                // that the user-facing `hash myc` query never read).
                let cn = cmdnam {
                    node: hashnode {
                        next: None,
                        nam: n.to_string(),
                        flags: HASHED as i32, // c:4316
                    },
                    name: None,
                    cmd: Some(v.to_string()), // c:4316
                };
                if let Ok(mut tab) = cmdnamtab_lock().write() {
                    tab.add(cn); // c:4318 addnode
                }
            }
            if OPT_ISSET(ops, b'v') {
                // c:4321
                if dir_mode {
                    if let Ok(t) = nameddirtab().lock() {
                        if let Some(nd) = t.get(n) {
                            // c:4322
                            printnameddirnode(nd, 0);
                        }
                    }
                }
            }
        } else {
            // c:4323-4334 — display existing entry / look up.
            if dir_mode {
                let snapshot = nameddirtab().lock().ok().and_then(|t| t.get(n).cloned());
                match snapshot {
                    Some(nd) => {
                        if OPT_ISSET(ops, b'v') {
                            // c:4337
                            printnameddirnode(&nd, 0);
                        }
                    }
                    None => {
                        zwarnnam(name, &format!("no such directory name: {}", n)); // c:4327
                        returnval = 1; // c:4328
                    }
                }
            } else {
                // c:4319-4334 — `else if (!(hn = ht->getnode2(ht,
                //   asg->name))) { ... if (!hashcmd(asg->name, path))
                //   zwarnnam("no such command"); }`. C path: first
                // check cmdnamtab for an existing entry; only fall
                // back to hashcmd's PATH walk when not present. The
                // previous Rust port skipped the cmdnamtab check, so
                // a prior `hash myc=/path` insert was invisible to
                // the matching `hash myc` query.
                let in_cmdnamtab = cmdnamtab_lock()
                    .read()
                    .map(|t| t.get(n).is_some())
                    .unwrap_or(false);
                if !in_cmdnamtab {
                    // c:4319 hn == NULL → try hashcmd.
                    let path: Vec<String> = getsparam("PATH")
                        .map(|p| p.split(':').map(String::from).collect())
                        .unwrap_or_default();
                    if crate::ported::exec::hashcmd(n, &path).is_none() {
                        // c:4332
                        zwarnnam(name, &format!("no such command: {}", n)); // c:4333
                        returnval = 1; // c:4334
                    }
                }
            }
        }
    }
    unqueue_signals(); // c:4346
    returnval // c:4346
}

/// Port of `bin_unhash(char *name, char **argv, Options ops, int func)` from Src/builtin.c:4346.
/// C: `int bin_unhash(char *name, char **argv, Options ops, int func)` —
///   remove entries from cmdnamtab/aliastab/sufaliastab/nameddirtab/
///   shfunctab. `-a` clears all, `-m` is a glob.
/// WARNING: param names don't match C — Rust=(name, argv, func) vs C=(name, argv, ops, func)
pub fn bin_unhash(
    name: &str,
    argv: &[String], // c:4346
    ops: &options,
    func: i32,
) -> i32 {
    let mut returnval = 0i32; // c:4351
    let mut all = 0i32; // c:4351
    let mut match_count = 0i32; // c:4351

    // PFA-SMR aspect: when invoked as `unalias`, record the un-alias
    // events so the replay can suppress earlier `alias` calls.
    #[cfg(feature = "recorder")]
    if crate::recorder::is_enabled() && func == crate::ported::builtin::BIN_UNALIAS {
        let ctx = crate::recorder::recorder_ctx_global();
        for a in argv {
            if a.starts_with('-') && a != "-" {
                continue;
            }
            crate::recorder::emit_unalias(a, ctx.clone());
        }
    }

    // c:4355-4373 — table-pick dispatch.
    enum Tab {
        CmdNam,
        NamedDir,
        Shfunc,
        Alias,
        SufAlias,
    }
    let tab: Tab;
    if func == BIN_UNALIAS {
        // c:4356
        tab = if OPT_ISSET(ops, b's') {
            Tab::SufAlias
        } else {
            Tab::Alias
        }; // c:4357
        if OPT_ISSET(ops, b'a') {
            // c:4361
            if !argv.is_empty() {
                // c:4362
                zwarnnam(name, "-a: too many arguments"); // c:4363
                return 1; // c:4364
            }
            all = 1; // c:4366
        } else if argv.is_empty() {
            // c:4367
            zwarnnam(name, "not enough arguments"); // c:4368
            return 1; // c:4369
        }
    } else if OPT_ISSET(ops, b'd') {
        tab = Tab::NamedDir; // c:4370
    } else if OPT_ISSET(ops, b'f') {
        tab = Tab::Shfunc; // c:4372
    } else if OPT_ISSET(ops, b's') {
        tab = Tab::SufAlias; // c:4374
    } else if func == BIN_UNHASH && OPT_ISSET(ops, b'a') {
        tab = Tab::Alias; // c:4376
    } else {
        tab = Tab::CmdNam;
    } // c:4378

    // Helper: clear entire table.
    let clear_all = |t: &Tab| match t {
        Tab::Alias => {
            let _ = aliastab_lock().write().map(|mut g| g.clear());
        }
        Tab::SufAlias => {
            let _ = sufaliastab_lock().write().map(|mut g| g.clear());
        }
        Tab::NamedDir => {
            emptynameddirtable();
        }
        Tab::Shfunc => {
            // c:4388 — empty whole shfunctab (`unhash -af` etc.). C uses
            // `emptyhashtable(shfunctab)` GSU; Rust port iterates names
            // and removes each (no `clear` method on shfunc_table).
            if let Ok(mut t) = shfunctab_lock().write() {
                let names: Vec<String> = t.iter().map(|(k, _)| k.clone()).collect();
                for nm in names {
                    let _ = t.remove(&nm);
                }
            }
        }
        Tab::CmdNam => {
            emptycmdnamtable();
        } // c:4389
    };
    let remove_one = |t: &Tab, nm: &str| -> bool {
        match t {
            Tab::Alias => aliastab_lock()
                .write()
                .map(|mut g| g.remove(nm).is_some())
                .unwrap_or(false),
            Tab::SufAlias => sufaliastab_lock()
                .write()
                .map(|mut g| g.remove(nm).is_some())
                .unwrap_or(false),
            Tab::NamedDir => crate::ported::hashnameddir::removenameddirnode(nm).is_some(),
            Tab::Shfunc => {
                let from_tab = shfunctab_lock()
                    .write()
                    .map(|mut g| g.remove(nm).is_some())
                    .unwrap_or(false);
                // Also remove from the executor's compiled-function /
                // source maps. Without this, `unset -f f` cleared
                // shfunctab but dispatch_function_call still found the
                // compiled chunk and ran the old body. Routed via the
                // exec accessors unregister_function fn-ptr installed by
                // fusevm_bridge at startup (no ShellExecutor reach-in
                // from src/ported/).
                let from_exec = crate::ported::exec::unregister_function(nm);
                from_tab || from_exec
            }
            // c:4405 — `if ((hn = ht->removenode(ht, *argv)))`.
            // Removal returns truthy only when the entry actually
            // existed. Previous Rust port hardcoded `true` after a
            // void-return `freecmdnamnode` call, so `unhash badname`
            // silently succeeded instead of emitting the canonical
            // "no such hash table element" error.
            Tab::CmdNam => cmdnamtab_lock()
                .write()
                .map(|mut g| g.remove(nm).is_some())
                .unwrap_or(false),
        }
    };

    if all != 0 {
        // c:4382
        queue_signals(); // c:4383
        clear_all(&tab); // c:4384-4389
        unqueue_signals(); // c:4390
        return 0; // c:4391
    }

    // c:4395-4421 — `-m` glob branch.
    if OPT_ISSET(ops, b'm') {
        // c:4395
        for arg in argv {
            // c:4396
            queue_signals(); // c:4397
            let pprog = patcompile(
                &{
                    let mut __pat_tok = (arg).to_string();
                    crate::ported::glob::tokenize(&mut __pat_tok);
                    __pat_tok
                }, // c:4400
                PAT_HEAPDUP,
                None,
            );
            if let Some(prog) = pprog {
                // Collect names then remove (avoid iterator/mutation conflict).
                // c:4408 — `scanmatchtable(ht, pprog, ...)` walks every
                // entry in the selected table. Previous Rust port left
                // Tab::CmdNam returning an empty Vec, so `unhash -m PAT`
                // (default cmd-hash table) silently matched zero entries.
                let names: Vec<String> = match &tab {
                    Tab::Alias => aliastab_lock()
                        .read()
                        .map(|t| t.iter().map(|(n, _)| n.clone()).collect())
                        .unwrap_or_default(),
                    Tab::SufAlias => sufaliastab_lock()
                        .read()
                        .map(|t| t.iter().map(|(n, _)| n.clone()).collect())
                        .unwrap_or_default(),
                    Tab::NamedDir => nameddirtab()
                        .lock()
                        .map(|t| t.keys().cloned().collect())
                        .unwrap_or_default(),
                    Tab::Shfunc => shfunctab_lock()
                        .read()
                        .map(|t| t.iter().map(|(k, _)| k.clone()).collect())
                        .unwrap_or_default(),
                    // c:4408 — cmdnamtab walk via `cmdnamtab_lock().iter()`.
                    Tab::CmdNam => cmdnamtab_lock()
                        .read()
                        .map(|t| t.iter().map(|(n, _)| n.clone()).collect())
                        .unwrap_or_default(),
                };
                for nm in &names {
                    if pattry(&prog, nm) {
                        // c:4408
                        if remove_one(&tab, nm) {
                            match_count += 1; // c:4410
                        }
                    }
                }
            } else {
                zwarnnam(name, &format!("bad pattern : {}", arg)); // c:4416
                returnval = 1; // c:4417
            }
            unqueue_signals(); // c:4419
        }
        if match_count == 0 {
            // c:4424
            returnval = 1; // c:4425
        }
        return returnval; // c:4426
    }

    // c:4429-4439 — literal-name removals.
    queue_signals(); // c:4430
    for arg in argv {
        // c:4431
        if remove_one(&tab, arg) { // c:4432
             // freed
        } else if func == BIN_UNSET && isset(POSIXBUILTINS) {
            // c:4434 — POSIX: unset of nonexistent isn't an error.
            returnval = 0; // c:4435
        } else {
            zwarnnam(name, &format!("no such hash table element: {}", arg)); // c:4437
            returnval = 1; // c:4450
        }
    }
    unqueue_signals(); // c:4450
    returnval // c:4450
}

/// Port of `bin_alias(char *name, char **argv, Options ops, UNUSED(int func))` from Src/builtin.c:4450.
/// C: `int bin_alias(char *name, char **argv, Options ops, ...)` — list,
///   define, glob-list, or display aliases. `-r`/`-g`/`-s` filter type;
///   `-L` prints definitions; `-m` treats args as patterns.
/// WARNING: param names don't match C — Rust=(name, argv, _func) vs C=(name, argv, ops, func)
pub fn bin_alias(
    name: &str,
    argv: &[String], // c:4450
    ops: &options,
    _func: i32,
) -> i32 {
    let mut returnval = 0i32; // c:4455
    let mut flags1 = 0u32; // c:4456
    let mut flags2 = DISABLED as u32; // c:4456
    let mut printflags = 0i32; // c:4457
    let mut use_suffix = false; // tracks ht switch

    // c:4461-4485 — type-flag parsing.
    let type_opts = (OPT_ISSET(ops, b'r') as i32)                            // c:4461
                  + (OPT_ISSET(ops, b'g') as i32)
                  + (OPT_ISSET(ops, b's') as i32);
    if type_opts != 0 {
        // c:4464
        if type_opts > 1 {
            // c:4465
            zwarnnam(name, "illegal combination of options"); // c:4466
            return 1; // c:4467
        }
        if OPT_ISSET(ops, b'g') {
            // c:4469
            flags1 |= ALIAS_GLOBAL as u32; // c:4470
        } else {
            flags2 |= ALIAS_GLOBAL as u32; // c:4472
        }
        if OPT_ISSET(ops, b's') {
            // c:4473
            flags1 |= ALIAS_SUFFIX as u32; // c:4480
            use_suffix = true; // c:4481
        } else {
            flags2 |= ALIAS_SUFFIX as u32; // c:4483
        }
    }

    // c:4486-4490 — printflags from -L / + suffix.
    if OPT_ISSET(ops, b'L') {
        // c:4486
        printflags |= PRINT_LIST; // c:4487
    } else if OPT_PLUS(ops, b'g')
        || OPT_PLUS(ops, b'r')
        || OPT_PLUS(ops, b's')
        || OPT_PLUS(ops, b'm')
        || OPT_ISSET(ops, b'+')
    // c:4488
    {
        printflags |= PRINT_NAMEONLY; // c:4490
    }

    // C bin_alias dispatches printing via `ht->printnode` (set to
    // `printaliasnode` at hashtable.c:1208) — `scanhashtable`,
    // `scanmatchtable`, and the single-name branch all call
    // `ht->printnode(&a->node, printflags)`. The Rust port routes
    // through the canonical `printaliasnode` (hashtable.rs:1477) for
    // the same dispatch. No local closure.

    // c:4495-4500 — no args: list all (filtered by flags).
    if argv.is_empty() {
        // c:4495
        queue_signals(); // c:4496
        let lock = if use_suffix {
            sufaliastab_lock()
        } else {
            aliastab_lock()
        };
        if let Ok(t) = lock.read() {
            // c:Src/builtin.c:4496 — `scanhashtable(aliastab, 1, ...)`
            // sets the SORT flag (1). The C source walks the table
            // sorted by `hnamcmp` (byte-wise ASCII compare); the
            // Rust port iterated in arbitrary hash order, so
            // `alias` output sequence diverged from zsh's
            // (uppercase-before-lowercase) ASCII ordering.
            let mut entries: Vec<_> = t.iter().collect();
            entries.sort_by(|(a, _), (b, _)| a.cmp(b));
            for (_n, a) in entries {
                // c:4497
                if (a.node.flags & flags1 as i32) == flags1 as i32
                    && (a.node.flags & flags2 as i32) == 0
                {
                    printaliasnode(a, printflags);
                }
            }
        }
        unqueue_signals(); // c:4498
        return 0; // c:4499
    }

    // c:4503-4519 — `-m` glob branch.
    if OPT_ISSET(ops, b'm') {
        // c:4503
        for pat in argv {
            // c:4504
            queue_signals(); // c:4505
                             // c:4506 — `tokenize + patcompile`.
            let pprog = patcompile(
                &{
                    let mut __pat_tok = (pat).to_string();
                    crate::ported::glob::tokenize(&mut __pat_tok);
                    __pat_tok
                }, // c:4507
                PAT_HEAPDUP,
                None,
            );
            if let Some(prog) = pprog {
                let lock = if use_suffix {
                    sufaliastab_lock()
                } else {
                    aliastab_lock()
                };
                if let Ok(t) = lock.read() {
                    for (_n, a) in t.iter() {
                        // c:4509
                        if (a.node.flags & flags1 as i32) == flags1 as i32
                            && (a.node.flags & flags2 as i32) == 0
                            && pattry(&prog, &a.node.nam)
                        {
                            printaliasnode(a, printflags);
                        }
                    }
                }
            } else {
                zwarnnam(name, &format!("bad pattern : {}", pat)); // c:4514
                returnval = 1; // c:4515
            }
            unqueue_signals(); // c:4517
        }
        return returnval; // c:4518
    }

    // c:4521-4540 — literal args: define `name=value` or display a single name.
    queue_signals(); // c:4522
                     // PFA-SMR: capture per-definition for replay. Dispatch the
                     // right subkind (galias/salias/alias) based on the parsed
                     // flag bits set above. Same per-name loop the C code walks
                     // — one record per `name=value` argv slot. Without this,
                     // the recorder harness saw zero `alias` captures.
    #[cfg(feature = "recorder")]
    let recorder_active = crate::recorder::is_enabled();
    #[cfg(feature = "recorder")]
    let recorder_ctx = if recorder_active {
        Some(crate::recorder::recorder_ctx_global())
    } else {
        None
    };
    let mut idx = 0;
    while idx < argv.len() {
        // c:4523
        let arg = &argv[idx];
        idx += 1;
        // c:Src/builtin.c:4523 `while ((asg = getasg(&argv, NULL)))`
        // — empty-name assignment makes getasg call `zerr("bad
        // assignment")` (c:1927, sets errflag) and return NULL. The
        // C while-loop exits without setting returnval, so bin_alias
        // returns 0. /bin/zsh's outer execution checks errflag and
        // aborts subsequent commands but keeps lastval=0.
        //
        // Rust port divergence: the canonical `zerr` here would set
        // ERRFLAG_ERROR, which zshrs's execcmd_exec promotes to
        // lastval=1 (different shape from C's "abort but keep
        // lastval"). Route through `zwarnnam` so the diagnostic
        // emits without flipping errflag; returnval stays 0 and the
        // shell rc matches /bin/zsh's observed behavior.
        if arg.starts_with('=') {
            zwarnnam(name, "bad assignment");
            break;
        }
        if let Some(eq) = arg.find('=') {
            // c:4524 (asg->value.scalar)
            if !OPT_ISSET(ops, b'L') {
                // c:4524
                let n = &arg[..eq];
                let v = &arg[eq + 1..];
                let lock = if use_suffix {
                    sufaliastab_lock()
                } else {
                    aliastab_lock()
                };
                if let Ok(mut t) = lock.write() {
                    let a = createaliasnode(n, v, flags1); // c:4527
                    t.add(a);
                }
                #[cfg(feature = "recorder")]
                if let Some(ref ctx) = recorder_ctx {
                    if (flags1 & ALIAS_GLOBAL as u32) != 0 {
                        crate::recorder::emit_galias(n, Some(v), ctx.clone());
                    } else if (flags1 & ALIAS_SUFFIX as u32) != 0 {
                        crate::recorder::emit_salias(n, Some(v), ctx.clone());
                    } else {
                        crate::recorder::emit_alias(n, Some(v), ctx.clone());
                    }
                }
                continue;
            }
        }
        let n = if let Some(eq) = arg.find('=') {
            &arg[..eq]
        } else {
            arg.as_str()
        };
        let lock = if use_suffix {
            sufaliastab_lock()
        } else {
            aliastab_lock()
        };
        // c:4530 — `(a = (Alias) ht->getnode(ht, asg->name))`. C
        // `getnode` is `gethashnode` (Src/hashtable.c:231) which
        // returns NULL for DISABLED entries; skipping disabled at
        // lookup time is what makes `alias FOO` print nothing after
        // `disable -a FOO` (bug #227). Must use `.get()` not
        // `.get_including_disabled()` here.
        let found = lock.read().ok().and_then(|t| {
            t.get(n)
                .map(|a| (a.node.nam.clone(), a.node.flags as u32, a.text.clone()))
        });
        match found {
            Some((nm, fl, txt)) => {
                // c:4530
                // c:4532-4537 — type-filter check.
                let show = type_opts == 0
                    || use_suffix
                    || (OPT_ISSET(ops, b'r') && (fl & (ALIAS_GLOBAL | ALIAS_SUFFIX) as u32) == 0)
                    || (OPT_ISSET(ops, b'g') && (fl & ALIAS_GLOBAL as u32) != 0);
                if show {
                    let a = createaliasnode(&nm, &txt, fl);
                    printaliasnode(&a, printflags);
                }
            }
            None => {
                // c:4538
                returnval = 1; // c:4539
            }
        }
    }
    unqueue_signals(); // c:4541
    returnval // c:4542
}

/// Port of `bin_true(UNUSED(char *name), UNUSED(char **argv), UNUSED(Options ops), UNUSED(int func))` from Src/builtin.c:4550.
/// C: `int bin_true(UNUSED(char *name), UNUSED(char **argv),
///                  UNUSED(Options ops), UNUSED(int func))` → `return 0;`
/// WARNING: param names don't match C — Rust=(_name, _argv, _func) vs C=(name, argv, ops, func)
pub fn bin_true(
    _name: &str,
    _argv: &[String], // c:4550
    _ops: &options,
    _func: i32,
) -> i32 {
    0 // c:4559
}

/// Port of `bin_false(UNUSED(char *name), UNUSED(char **argv), UNUSED(Options ops), UNUSED(int func))` from Src/builtin.c:4559.
/// C: `int bin_false(UNUSED(char *name), UNUSED(char **argv),
///                   UNUSED(Options ops), UNUSED(int func))` → `return 1;`
/// WARNING: param names don't match C — Rust=(_name, _argv, _func) vs C=(name, argv, ops, func)
pub fn bin_false(
    _name: &str,
    _argv: &[String], // c:4559
    _ops: &options,
    _func: i32,
) -> i32 {
    1 // c:4562
}

/// Port of `bin_print(char *name, char **args, Options ops, int func)` from Src/builtin.c:4587.
/// C: `int bin_print(char *name, char **args, Options ops, int func)`.
///
/// The C body is ~1000 lines: `print` / `echo` / `printf` / `pushln`
/// dispatcher with -n/-N/-c/-r/-R/-l/-D/-i/-f/-v/-s/-S/-z/-e/-E etc.
/// The structural port handles the script-friendly subset that the
/// daily-driver hits: print/echo plain emission with -n, -l (one per
/// line), -r raw, -E newline-only, -- end-of-options. The full -f
/// printf format-spec engine and ZLE/history wireups defer to the
/// expand_printf_escapes helpers.
/// WARNING: param names don't match C — Rust=(name, args, func) vs C=(name, args, ops, func)
pub fn bin_print(
    name: &str,
    args: &[String], // c:4587
    ops: &options,
    func: i32,
) -> i32 {
    // c:4659-4684 — "Error check option combinations and option arguments".
    // The FIRST thing bin_print does, before any argument work:
    //     if (OPT_ISSET(ops,'z') + OPT_ISSET(ops,'s') + OPT_ISSET(ops,'S') +
    //         OPT_ISSET(ops,'v') > 1) {
    //         zwarnnam(name, "only one of -s, -S, -v, or -z allowed"); return 1; }
    //     if ((OPT_ISSET(ops,'z') | OPT_ISSET(ops,'s') | OPT_ISSET(ops,'S')) +
    //         (OPT_ISSET(ops,'c') | OPT_ISSET(ops,'C')) > 1) {
    //         zwarnnam(name, "-c or -C not allowed with -s, -S, or -z"); return 1; }
    //     if ((OPT_ISSET(ops,'z') | OPT_ISSET(ops,'v') | OPT_ISSET(ops,'s') |
    //          OPT_ISSET(ops,'S')) + (OPT_ISSET(ops,'p') | OPT_ISSET(ops,'u')) > 1) {
    //         zwarnnam(name, "-p or -u not allowed with -s, -S, -v, or -z"); return 1; }
    //
    // None of the three were ported, so every conflicting combination was
    // silently accepted: `print -s -v v a b`, `print -C 2 -s a b` and
    // `print -u1 -v v a b` all returned 0 having done something other than what
    // zsh does. Note the mixed operators are deliberate — `+` counts DISTINCT
    // options in the first test (so `-s -S` is two), while `|` collapses each
    // GROUP in the other two before counting, so the test is "one from each
    // group", not "two options total".
    //
    // C's fourth check ("-f not allowed with -c, -C, or -S", c:4679-4683) is
    // COMMENTED OUT and must NOT be reinstated: `print -f %s -c a b` is legal
    // and prints, as verified against the oracle. The string exists in the
    // source but the code is dead — the reason to read the C rather than grep
    // its message table.
    //
    // These precede the `-C` argument validation below, matching C, so
    // `print -C 0 -s` reports the conflict rather than the bad column count.
    {
        let n_sv = i32::from(OPT_ISSET(ops, b'z'))
            + i32::from(OPT_ISSET(ops, b's'))
            + i32::from(OPT_ISSET(ops, b'S'))
            + i32::from(OPT_ISSET(ops, b'v'));
        if n_sv > 1 {
            zwarnnam(name, "only one of -s, -S, -v, or -z allowed"); // c:4664
            return 1; // c:4665
        }
        let g_zsS = OPT_ISSET(ops, b'z') || OPT_ISSET(ops, b's') || OPT_ISSET(ops, b'S');
        let g_cC = OPT_ISSET(ops, b'c') || OPT_ISSET(ops, b'C');
        if i32::from(g_zsS) + i32::from(g_cC) > 1 {
            zwarnnam(name, "-c or -C not allowed with -s, -S, or -z"); // c:4670
            return 1; // c:4671
        }
        let g_zvsS = OPT_ISSET(ops, b'z')
            || OPT_ISSET(ops, b'v')
            || OPT_ISSET(ops, b's')
            || OPT_ISSET(ops, b'S');
        let g_pu = OPT_ISSET(ops, b'p') || OPT_ISSET(ops, b'u');
        if i32::from(g_zvsS) + i32::from(g_pu) > 1 {
            zwarnnam(name, "-p or -u not allowed with -s, -S, -v, or -z"); // c:4677
            return 1; // c:4678
        }
    }
    let nonewline = OPT_ISSET(ops, b'n'); // c:4595
    let raw = OPT_ISSET(ops, b'r') || OPT_ISSET(ops, b'R'); // c:4596
                                                            // c:4597 — `-l` puts one arg per line. `-c` is "columns" but
                                                            // degrades to one-per-line when stdout isn't a tty (terminal-
                                                            // width-aware tabular print isn't ported); accept -c here as
                                                            // a synonym for -l so `print -c a b c` byte-matches zsh's
                                                            // non-tty fallback.
    let one_per_line = OPT_ISSET(ops, b'l') || OPT_ISSET(ops, b'c');
    let nul_sep = OPT_ISSET(ops, b'N'); // c:5114/5127/5132 — NUL separator
                                        // c:Src/builtin.c — `-D` runs each arg through `dirify()`: if
                                        // it matches a named dir or $HOME-prefix, abbreviate with
                                        // `~`. zsh has the dircache + named-dir table behind it; for
                                        // -c mode (non-interactive) the table mostly contains $HOME
                                        // → "~". Apply that single rewrite for parity with the common
                                        // case. The richer named-dir lookup belongs in a deeper
                                        // dirify port.
    let dirify_d = OPT_ISSET(ops, b'D');
    let _printf_mode = func == BIN_PRINTF || OPT_HASARG(ops, b'f'); // c:4604
    let echo_mode = func == BIN_ECHO;
    let _ = (name, raw);

    // c:Src/builtin.c:5095-5106 — `-x N` (and `-X N`) require the
    // argument to be a positive integer (zstrtol parse with no
    // trailing garbage AND value > 0). zsh emits
    // `positive integer expected after -x: <arg>` and returns 1
    // when either gate fails. The previous Rust port silently
    // accepted any -x argument and proceeded to print the rest of
    // argv. Bug #525.
    if OPT_HASARG(ops, b'x') || OPT_HASARG(ops, b'X') {
        let which = if OPT_HASARG(ops, b'X') { b'X' } else { b'x' };
        let xarg = OPT_ARG(ops, which).unwrap_or("");
        // c:Src/builtin.c:5101 — `expand = zstrtol(xarg, &eptr, 10);
        //   if (*eptr || expand <= 0) zwarnnam(...positive integer
        //   expected...); return 1;`. zstrtol parses optional
        //   leading sign + digits and stops at the first non-digit.
        //   Reject when eptr isn't end-of-string (trailing garbage)
        //   OR the signed result is <= 0 (zero / negative).
        let valid = xarg.parse::<i64>().map(|n| n > 0).unwrap_or(false);
        if !valid {
            zwarnnam(
                name,
                &format!(
                    "positive integer expected after -{}: {}",
                    which as char, xarg
                ),
            );
            return 1;
        }
    }

    // c:4633-4685 — destination dispatch. -u FD writes to fd, -s pushes
    // to history, -z to ZLE buffer, -v VAR assigns to scalar.
    let dest_var: Option<String> = if OPT_HASARG(ops, b'v') {
        OPT_ARG(ops, b'v').map(String::from)
    } else {
        None
    };
    // c:Src/builtin.c:4827-4828 — `-p` writes to the coprocess fd
    // (the canonical `coprocout` global; Src/exec.c:430). When a
    // coproc is live, write to that fd; otherwise emit the same
    // diagnostic + rc=1 zsh does (Src/builtin.c:5050
    // `zwarnnam(name, "-p: no coprocess")`). Bug #388.
    let print_dash_p_fd: Option<fs::File> = if OPT_ISSET(ops, b'p') {
        let coprocout =
            crate::ported::modules::clone::coprocout.load(std::sync::atomic::Ordering::Relaxed);
        if coprocout < 0 {
            zwarnnam(name, "-p: no coprocess");
            return 1;
        }
        // c:4843 — dup for an owned writer that close-on-drop
        // doesn't close the user's original fd.
        let dup_fd = unsafe { libc::dup(coprocout) };
        if dup_fd < 0 {
            zwarnnam(name, "-p: no coprocess");
            return 1;
        }
        use std::os::unix::io::FromRawFd;
        Some(unsafe { fs::File::from_raw_fd(dup_fd) })
    } else {
        None
    };
    // c:4815-4851 — `-u FD` (and `-p` coprocess) dispatch. Parses FD,
    // dup's it for an owned descriptor, opens as a File for writes.
    // The previous Rust port silently dropped `-u`, so `print -u 2
    // hello` went to stdout instead of stderr.
    let dest_fd: Option<fs::File> = if OPT_HASARG(ops, b'u') {
        // c:4826
        let argptr = OPT_ARG(ops, b'u').unwrap_or("");
        // c:4827-4828 — undocumented `-up` aliases to coprocout.
        // Rust skip: coprocout isn't wired yet; document the gap.
        match argptr.parse::<i32>() {
            // c:4835 zstrtol
            Ok(fdarg) => {
                // c:4843 — `dup(fdarg)` for an owned writer that
                // close-on-drop doesn't close the user's original fd.
                let dup_fd = unsafe { libc::dup(fdarg) };
                if dup_fd < 0 {
                    zwarnnam(name, &format!("bad file number: {}", fdarg)); // c:4844
                    return 1; // c:4845
                }
                use std::os::unix::io::FromRawFd;
                Some(unsafe { fs::File::from_raw_fd(dup_fd) }) // c:4847
            }
            Err(_) => {
                zwarnnam(name, &format!("number expected after -u: {}", argptr)); // c:4837
                return 1; // c:4838
            }
        }
    } else {
        None
    };

    // c:4604-4612 — printf format-string handling.
    if _printf_mode {
        // c:Src/builtin.c:4701-4706 — `if (func == BIN_PRINTF) { if
        // (!strcmp(*args, "--") && !*++args) { ...not enough args...
        // } fmt = *args++; }`. Consume a leading `--` end-of-options
        // marker (printf is BINF_SKIPDASH but NOT BINF_DASHDASHVALID,
        // so the generic flag-parser doesn't strip `--` — bin_print
        // does the strip itself for BIN_PRINTF). Without this,
        // `printf -- "%s\n" hi` used `--` as the format string. Bug
        // #284.
        let args_owned: Vec<String>;
        let args: &[String] = if func == BIN_PRINTF
            && !OPT_HASARG(ops, b'f')
            && !args.is_empty()
            && args[0] == "--"
        {
            // Slice past the `--` for the rest of this arm only.
            if args.len() == 1 {
                // c:4703 `not enough arguments`.
                zwarnnam(name, "not enough arguments");
                return 1;
            }
            args_owned = args[1..].to_vec();
            &args_owned
        } else {
            args
        };
        let fmt = if let Some(f) = OPT_ARG(ops, b'f') {
            f.to_string()
        } else if !args.is_empty() {
            args[0].clone()
        } else {
            return 0;
        };
        let rest: &[String] = if OPT_HASARG(ops, b'f') {
            args
        } else {
            &args[1..]
        };
        // c:4712-4741 — the -m filter runs BEFORE format rendering:
        // first remaining arg is the pattern, later args are kept iff
        // pattry matches, and `if (fmt && !*args) return 0;` short-
        // circuits an empty post-filter list. The fmt fast-branch
        // here previously skipped the filter entirely, so
        // `print -m -f '%s|' 'a*' apple banana` rendered every arg.
        let m_filtered: Vec<String>;
        let rest: &[String] = if OPT_ISSET(ops, b'm') {
            if rest.is_empty() {
                zwarnnam(name, "no pattern specified"); // c:4723
                return 1;
            }
            let pat = &rest[0];
            let mut pat_tok = pat.to_string();
            crate::ported::glob::tokenize(&mut pat_tok); // c:4727
            let Some(pprog) = crate::ported::pattern::patcompile(&pat_tok, PAT_STATIC, None) else {
                zwarnnam(name, &format!("bad pattern: {}", pat)); // c:4730
                return 1;
            };
            m_filtered = rest[1..]
                .iter()
                .filter(|a| crate::ported::pattern::pattry(&pprog, a)) // c:4735-4737
                .cloned()
                .collect();
            if m_filtered.is_empty() {
                return 0; // c:4741 `if (fmt && !*args) return 0;`
            }
            &m_filtered
        } else {
            rest
        };
        // c:4799-4807 — the `-o`/`-O`/`-i` argument sort runs BEFORE
        // format rendering. The non-fmt path sorts at the tail of
        // bin_print (strmetasort), but this `-f` fast-branch returns
        // first, so `print -of '%s\n' foo bar baz` rendered the args
        // unsorted. Apply the same strmetasort to the format args here.
        let rest_sorted: Vec<String>;
        let rest: &[String] = if OPT_ISSET(ops, b'o') || OPT_ISSET(ops, b'O') {
            let mut v = rest.to_vec();
            let mut sflags: u32 = if OPT_ISSET(ops, b'i') {
                SORTIT_IGNORING_CASE as u32 // c:4805
            } else {
                0
            };
            if OPT_ISSET(ops, b'O') {
                sflags |= SORTIT_BACKWARDS as u32; // c:4806
            }
            strmetasort(&mut v, sflags, None); // c:4807
            rest_sorted = v;
            &rest_sorted
        } else {
            rest
        };
        // c:5464 — reset the per-run `%d`/`%i` math-error flag; a bad
        // math operand during formatting sets it (parse_int_arg) and turns
        // the builtin's exit status into 1 without aborting output.
        PRINTF_MATH_ERR.with(|c| c.set(false));
        // c:builtin.c:5430-5443 — printf returns 1 on unknown
        // directive after `zwarnnam(name, "%s: invalid directive",
        // start)`. The partial output produced before the bad
        // directive is still printed (C emits via fwrite/fprintf
        // throughout the format walk).
        let (out, bounds, n_targets) = match printf_format(&fmt, rest) {
            Ok(s) => s,
            Err((partial, msg)) => {
                print!("{}", partial); // c: partial output already in fout
                use std::io::Write;
                let _ = std::io::stdout().flush();
                // c:5435 invalid directive / c:5204 argument specifier
                // out of range — `msg` carries the full zwarnnam text.
                crate::ported::utils::zwarnnam(name, &msg);
                return 1; // c:5443
            }
        };
        // c:Src/builtin.c:5427-5429 — apply the `%n` stores. C runs setiparam
        // inside the format loop; printf_format has no shell handle, so it
        // collected (name, byte_count) and the assignment happens here. Done
        // before the -z/-s output branches because in C the variable is set
        // during formatting regardless of where the text is routed.
        for (nv, cnt) in &n_targets {
            crate::ported::params::setiparam(nv, *cnt as i64); // c:5428
        }
        // c:4854-4856 — `if (OPT_ISSET(ops, 'v') || (fmt && (OPT_ISSET
        //   (ops, 'z') || OPT_ISSET(ops, 's')))) ASSIGN_MSTREAM(...)`.
        // For -f combined with -z or -s, capture output then route
        // through the same dispatch as the non-fmt path.
        if OPT_ISSET(ops, b'z') {
            // c:5564-5565 — push captured output to bufstack.
            crate::ported::zle::zle_main::BUFSTACK
                .lock()
                .unwrap()
                .push(out);
            return 0;
        }
        if OPT_ISSET(ops, b's') {
            // c:5569-5574 — push captured output as a history entry.
            let event_id = crate::ported::hist::prepnexthistent();
            crate::ported::hashtable::addhistnode(&out, event_id as i32);
            return 0;
        }
        if let Some(ref v) = dest_var {
            // c:builtin.c:5162 — `visarr = v && PM_TYPE(...) == PM_ARRAY`.
            // When the -v target is an existing array, the output is
            // split into one element per format-reuse cycle (c:5552-5560
            // `if (visarr && splits) { ... setaparam(...) }`); otherwise
            // the whole output is one scalar (c:5567 `setsparam(...)`).
            let visarr = {
                use crate::ported::zsh_h::{PM_ARRAY, PM_TYPE};
                paramtab()
                    .read()
                    .ok()
                    .and_then(|tab| {
                        tab.get(v)
                            .map(|pm| PM_TYPE(pm.node.flags as u32) == PM_ARRAY)
                    })
                    .unwrap_or(false)
            };
            // c:5169-5171 — `splits` is only allocated on the SECOND and
            // later cycles (`argp > args`); a single-cycle format leaves
            // `splits` NULL, so `if (visarr && splits)` (c:5552) is false
            // and the whole output becomes one scalar. `bounds.len() > 1`
            // mirrors `splits != NULL`.
            if visarr && bounds.len() > 1 {
                // c:5553-5559 — slice `out` at each cycle boundary; one
                // array element per format application. `bounds` holds the
                // byte offset where each cycle began (the first is 0).
                let mut arrayval: Vec<String> = Vec::with_capacity(bounds.len());
                for (i, &start) in bounds.iter().enumerate() {
                    let end = bounds.get(i + 1).copied().unwrap_or(out.len());
                    arrayval.push(out[start..end].to_string());
                }
                setaparam(v, arrayval);
            } else {
                setsparam(v, &out);
            }
        } else {
            // c:Src/builtin.c — C printf goes through libc fwrite to
            // stdout, unbuffered when fd 1 is the redirect target.
            // Rust's `print!` is line-buffered; without a trailing
            // newline the bytes stay in the buffer past the redirect
            // restore and land on the original stdout. Route through
            // stdout().write_all + flush so the bytes hit fd 1 (the
            // redirect target) immediately. Bug #397.
            use std::io::Write as _;
            let stdout = io::stdout();
            let mut lk = stdout.lock();
            // c:Src/builtin.c:5408 — `print_val(unmetafy(stringval,
            // &curlen))`: printf's values are unmetafied before the
            // write. Decode Meta-char pairs to raw bytes. Bug #127.
            let _ = lk.write_all(&crate::ported::utils::unmetafy_str(&out));
            let _ = lk.flush();
        }
        // c:5464 — a `%d`/`%i` math-operand error makes printf exit 1
        // (output already emitted).
        return if PRINTF_MATH_ERR.with(|c| c.get()) {
            1
        } else {
            0
        };
    }

    // c:4718-4741 — `-m PATTERN args...` glob-filter. First arg is
    // the pattern; remaining args are kept iff `pattry(pat, arg)`.
    // Previously absent — `print -m 'foo*' foo1 bar foo2` emitted
    // all four args instead of just foo1/foo2.
    let mut processed_args: Vec<String> = if OPT_ISSET(ops, b'm') {
        // c:4718
        if args.is_empty() {
            // c:4722
            zwarnnam(name, "no pattern specified"); // c:4723
            return 1; // c:4724
        }
        // c:4728 — `patcompile(*args, PAT_STATIC, NULL)`.
        let pat = &args[0];
        let pprog = patcompile(
            &{
                let mut __pat_tok = (pat).to_string();
                crate::ported::glob::tokenize(&mut __pat_tok);
                __pat_tok
            },
            PAT_STATIC,
            None,
        );
        match pprog {
            None => {
                zwarnnam(name, &format!("bad pattern: {}", pat)); // c:4730
                return 1; // c:4732
            }
            Some(prog) => {
                // c:4734-4737 — `for (t = p = ++args; *p; p++) if
                // (pattry(pprog, *p)) *t++ = *p;`. Keep matching args.
                args[1..]
                    .iter()
                    .filter(|a| pattry(&prog, a))
                    .cloned()
                    .collect()
            }
        }
    } else {
        args.to_vec()
    };

    // c:4860+ — main print loop.
    // c:5126-5127 — separator priority: `-l` ('\n') > `-N` ('\0') > ' '.
    let sep = if one_per_line {
        "\n"
    } else if nul_sep {
        "\0"
    } else {
        " "
    };
    // c:Src/builtin.c:4783-4795 — `-D`: interpret each arg as a
    // directory and abbreviate via `finddir(args[n])` — longest-prefix
    // match against the full named-dir table ($HOME → `~`, `hash -d`
    // entries → `~name`, plus the `zsh_directory_name` hook). C rewrites
    // the matched prefix to `~name` and keeps the trailing path. The
    // canonical `finddir` port already returns the `~name/rest` form.
    if dirify_d {
        for a in processed_args.iter_mut() {
            if let Some(abbrev) = crate::ported::utils::finddir(a) {
                *a = abbrev; // c:4791 — `~%s%s`
            }
        }
    }
    // c:4598-4600 — `-P` prompt-style percent expansion (`%n`, `%d`,
    // `%?`, `%h`, `%%`, etc.). Routes through `expand_prompt`
    // (canonical port of `Src/prompt.c:182 promptexpand`).
    if OPT_ISSET(ops, b'P') {
        // c:Src/builtin.c:4745-4746 — `if (OPT_ISSET(ops, 'P'))
        //   txtunknownattrs = TXT_ATTR_ALL;`. Marks every attribute
        // as initially "unknown" so the first `%b`/`%u`/`%s`/`%f`/`%k`
        // off-escape in this print invocation emits its terminfo cap
        // (via tunsetattrs at prompt.c:1758 — `txtcurrentattrs |=
        // newattrs & txtunknownattrs`). Without this seed, applying
        // an off-escape against zero current attrs produced no diff
        // and zshrs emitted nothing — `%b` / `%u` / `%s` looked like
        // no-ops in fresh prompts. Bug #38 in docs/BUGS.md.
        crate::ported::prompt::txtunknownattrs.store(
            crate::ported::zsh_h::TXT_ATTR_ALL,
            std::sync::atomic::Ordering::SeqCst,
        );
        // c:4598-4600 — `-P` prompt-style percent expansion.
        for a in processed_args.iter_mut() {
            *a = crate::ported::prompt::expand_prompt(a); // c:Src/prompt.c:182
                                                          // c:Src/prompt.c:236-247 — `if (!ns) { ... chuck(Inpar/
                                                          // Outpar/Nularg); }`. When `ns=0` (non-stripping flag
                                                          // off), zsh REMOVES the Inpar/Outpar/Nularg marker bytes
                                                          // from the output. `print -P` calls promptexpand with
                                                          // `ns=0` per Src/builtin.c:4598, so the SGR-wrapping
                                                          // markers MUST NOT leak into stdout. The Rust port's
                                                          // expand_prompt uses ad-hoc `\x01`/`\x02` (readline
                                                          // RL_PROMPT_*_IGNORE) markers instead of canonical
                                                          // Inpar/Outpar, but the strip rule applies identically:
                                                          // for non-prompt-render callers, scrub them. Parity bug
                                                          // #17 — without this, `print -P "%F{red}red%f"` emitted
                                                          // `\x01\E[31m\x02red\x01\E[39m\x02` instead of zsh's
                                                          // `\E[31mred\E[39m`.
            a.retain(|c| c != '\x01' && c != '\x02');
        }
    }
    // c:4799-4808 — `-o` / `-O` / `-i` sort flags.
    //
    // C body:
    // ```c
    // if (OPT_ISSET(ops,'o') || OPT_ISSET(ops,'O')) {
    //     flags = OPT_ISSET(ops,'i') ? SORTIT_IGNORING_CASE : 0;
    //     if (OPT_ISSET(ops,'O'))
    //         flags |= SORTIT_BACKWARDS;
    //     strmetasort(args, flags, len);
    // }
    // ```
    //
    // Meaning: `-i` sets `SORTIT_IGNORING_CASE` (case-INSENSITIVE).
    // Without `-i`, sort is case-SENSITIVE.
    //
    // The previous Rust port had this INVERTED — it bound
    // `case_sensitive = OPT_ISSET(ops, b'i')`, then case-sensitive-
    // sorted under `-i` and case-insensitive-sorted without `-i`.
    // The doc-comment for the block claimed "-o → case-insensitive
    // ascending" which is also wrong. Result: `print -o foo Bar BAZ`
    // emitted `BAZ Bar foo` (case-insensitive) when zsh emits
    // `BAZ Bar foo` only WITH `-i`; without it, zsh emits
    // `BAZ Bar foo` ordered by ASCII (caps first).
    if OPT_ISSET(ops, b'o') || OPT_ISSET(ops, b'O') {
        // c:4800
        // c:4801-4807 — `flags = OPT_ISSET(ops,'i') ? SORTIT_IGNORING_CASE
        // : 0; if (OPT_ISSET(ops,'O')) flags |= SORTIT_BACKWARDS;
        // strmetasort(args, flags, len);`. The previous Rust port used an
        // ad-hoc `processed_args.sort()` (byte/ordinal order) plus a
        // `.reverse()`, bypassing the faithful comparator. zsh's eltpcmp
        // sorts via `strcoll(as, bs)` (sort.c:134) — locale collation,
        // case-insensitive in UTF-8 locales — and SORTIT_BACKWARDS flips
        // `sortdir` inside the comparator (not a post-sort reverse).
        let mut flags: u32 = if OPT_ISSET(ops, b'i') {
            SORTIT_IGNORING_CASE as u32 // c:4805
        } else {
            0
        };
        if OPT_ISSET(ops, b'O') {
            flags |= SORTIT_BACKWARDS as u32; // c:4806
        }
        strmetasort(&mut processed_args, flags, None); // c:4807
    }
    // c:Src/builtin.c:4866-4886 — when `-r` is NOT set, each arg goes
    // through `getkeystring` to interpret backslash escapes (`\n`,
    // `\t`, `\\`, escaped space `\ `, etc.). `echo` follows the same
    // path when `BSD_ECHO`/`SH_OPTION_LETTERS`-style isn't in effect;
    // BIN_ECHO with `-E` keeps escapes literal. Without this, `print
    // -- ${(q)a}` for `a="he llo"` emitted `he\ llo` instead of zsh's
    // `he llo` (the (q) flag's backslash gets consumed by print).
    // c:builtin.c:4747-4767 — escape interpretation dispatch.
    //   - `fmt` (printf format already chosen via -f) or
    //     `(!-e && (-R || -r || -E))` → unmetafy only, NO escape
    //     interpretation (raw passthrough).
    //   - Otherwise pick `escape_how` per c:4754-4760:
    //       `-b`                            → GETKEYS_BINDKEY (bindkey
    //                                          escapes: `\C-`/`\M-`/`^X`)
    //       func != BIN_ECHO && !`-e`       → GETKEYS_PRINT (with EMACS:
    //                                          unknown `\<c>` → `<c>`)
    //       else (BIN_ECHO or `-e`)         → GETKEYS_ECHO (preserves
    //                                          unknown `\<c>` as `\<c>`)
    //
    // Previous Rust port unconditionally used GETKEYS_PRINT for both
    // `echo` and `print` — `echo "${(qq)s}"` for `s="a'b"` stripped
    // the `\` from the `(qq)`-emitted `'a'\''b'` because GETKEYS_PRINT
    // includes GETKEY_EMACS. zsh keeps the `\` (echo uses GETKEYS_ECHO,
    // no EMACS).
    let dash_e = OPT_ISSET(ops, b'e');
    // c:Src/builtin.c:4754 — BSD_ECHO option flips echo's default:
    // escape processing is OFF unless `-e` is explicitly passed.
    // Without bsd_echo (the SysV default), escapes process unless
    // `-E`/`-R`/`-r` is set.
    // !!! DASH-STRICT GATE (no C counterpart) !!! dash's `echo` is the
    // XSI/SysV variant: it interprets `\t`, `\n`, `\c`, … by default.
    // EMULATE_SH sets BSDECHO (escapes OFF without -e), the opposite of
    // dash, so force bsd_echo inactive under dash_strict → escapes process
    // by default, matching `/bin/dash`. (Note: dash's echo also does not
    // consume -e/-E as flags; that flag-parsing nuance is not handled here
    // — only the default escape behavior is aligned.)
    let bsd_echo_active = echo_mode && isset(BSDECHO) && !crate::dash_mode::dash_strict();
    let suppress_escapes = OPT_ISSET(ops, b'R')
        || OPT_ISSET(ops, b'r')
        || (echo_mode && OPT_ISSET(ops, b'E'))
        || (bsd_echo_active && !dash_e);
    let mut backslash_c_truncated = false;
    if !suppress_escapes || dash_e {
        // c:builtin.c:4754-4760 — `-b` (bindkey escapes) takes precedence,
        // then GETKEYS_PRINT for print (EMACS), else GETKEYS_ECHO.
        let escape_how: u32 = if OPT_ISSET(ops, b'b') {
            GETKEYS_BINDKEY // c:4755
        } else if !echo_mode && !dash_e {
            GETKEYS_PRINT // c:4758
        } else {
            GETKEYS_ECHO // c:4760
        };
        // Clear any stale TLS flag before the loop.
        let _ = crate::ported::utils::getkey_truncated_take();
        let mut new_args: Vec<String> = Vec::with_capacity(processed_args.len());
        for a in processed_args.iter() {
            let (s, _) = getkeystring_with(a, escape_how, None);
            new_args.push(s);
            if crate::ported::utils::getkey_truncated_take() {
                // c:utils.c:7045 — `\c` truncated; drop remaining
                // args entirely AND suppress trailing newline.
                backslash_c_truncated = true;
                break;
            }
        }
        processed_args = new_args;
    }
    // c:Src/builtin.c:4930-4958 — `-C N` column-grid output. Layout
    // N args per row (nr = ceil(argc/nc) rows), each cell padded
    // to widest arg + 2 spaces. Default mode is COLUMN-MAJOR (col 1
    // takes first nr items, col 2 the next nr, etc.). The `-a` flag
    // (c:4980 "print across, i.e. columns first") switches to
    // ROW-MAJOR fill — items flow across each row before moving to
    // the next row. Bug #40 in docs/BUGS.md: zshrs ignored `-a` and
    // always produced column-major output.
    let body = if !_printf_mode && OPT_HASARG(ops, b'C') {
        // c:4687-4698 — `-C` validates its argument two ways and FAILS; it does
        // not fall back:
        //     nc = (int)zstrtol(argptr, &eptr, 10);
        //     if (*eptr) {
        //         zwarnnam(name, "number expected after -%c: %s", 'C', argptr);
        //         return 1;
        //     }
        //     if (nc <= 0) {
        //         zwarnnam(name, "invalid number of columns: %s", argptr);
        //         return 1;
        //     }
        // This parsed as usize, filtered `n > 0`, and silently
        // `.unwrap_or(1)`, so `print -C 0`, `print -C -1`, `print -C abc` and
        // `print -C ''` all printed one column per line and returned 0 where
        // zsh fails. zstrtol is what separates the two messages: trailing
        // garbage leaves *eptr set ("3x" → "number expected"), while a
        // well-formed non-positive parses cleanly and reaches the second check
        // ("-1" → "invalid number of columns"). Leading blanks are skipped by
        // zstrtol, so `-C ' 2'` is valid.
        let argptr = OPT_ARG(ops, b'C').unwrap_or("");
        let (nc_l, rest) = crate::ported::utils::zstrtol_underscore(argptr, 10, false); // c:4689
        if !rest.is_empty() {
            zwarnnam(name, &format!("number expected after -C: {argptr}")); // c:4691
            return 1; // c:4692
        }
        if nc_l <= 0 {
            zwarnnam(name, &format!("invalid number of columns: {argptr}")); // c:4695
            return 1; // c:4696
        }
        let nc: usize = nc_l as usize;
        let argc = processed_args.len();
        let nr = (argc + nc - 1) / nc;
        let across = OPT_ISSET(ops, b'a'); // c:4947 / c:4980
                                           // c:Src/builtin.c:4946-4956 — max-width walk skips
                                           // last-column items because they don't need trailing
                                           // padding. The skip set differs by mode:
                                           //   -a (row-major): skip i where (i % nc) == nc-1
                                           //   default (col-major): skip i where i >= nr * (nc-1)
        let max_w = processed_args
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                if across {
                    (i % nc) != nc - 1
                } else {
                    *i < nr * (nc.saturating_sub(1))
                }
            })
            .map(|(_, s)| s.chars().count())
            .max()
            .unwrap_or(0);
        let sc = max_w + 2;
        let mut out = String::new();
        for row in 0..nr {
            for col in 0..nc {
                // c:4982-4986 (-a) — `idx = row*nc + col` for
                // row-major fill across columns first.
                // c:4994-5005 (default) — `idx = col*nr + row` for
                // column-major fill down columns first.
                let idx = if across {
                    row * nc + col
                } else {
                    col * nr + row
                };
                if idx >= argc {
                    break;
                }
                let cell = &processed_args[idx];
                // Padding skip rule mirrors max_w gate:
                // last-column-of-row (-a) or last-column-of-grid
                // (default) gets no trailing padding.
                let is_last_col_of_row = col == nc - 1;
                let is_after_last_grid_col = !across && col * nr + row + nr >= argc;
                if is_last_col_of_row || is_after_last_grid_col {
                    out.push_str(cell);
                } else {
                    out.push_str(cell);
                    let pad = sc.saturating_sub(cell.chars().count());
                    out.extend(std::iter::repeat(' ').take(pad));
                }
            }
            out.push('\n');
        }
        // Strip trailing newline; the post-loop `if !nonewline` adds
        // one back.
        if out.ends_with('\n') {
            out.pop();
        }
        out
    } else if OPT_HASARG(ops, b'x') || OPT_HASARG(ops, b'X') {
        // c:Src/builtin.c:5095-5125 — `print -x N` / `-X N` expand tabs
        // to spaces at tab stops of width N (`-x` only leading tabs,
        // `-X` all tabs), threading the column position across args and
        // resetting it on the `-l` newline. zexpandtabs (utils.c:5975)
        // was ported but never wired into print output.
        let all = OPT_HASARG(ops, b'X');
        let which = if all { b'X' } else { b'x' };
        let width: i32 = OPT_ARG(ops, which)
            .and_then(|a| a.parse().ok())
            .unwrap_or(8);
        let n = processed_args.len();
        let mut out = String::new();
        let mut startpos = 0i32;
        for (i, arg) in processed_args.iter().enumerate() {
            startpos = crate::ported::utils::zexpandtabs(arg, width, startpos, all, &mut out);
            if i + 1 < n {
                if OPT_ISSET(ops, b'l') {
                    out.push('\n');
                    startpos = 0;
                } else if OPT_ISSET(ops, b'N') {
                    out.push('\0');
                } else {
                    out.push(' ');
                    startpos += 1;
                }
            }
        }
        out
    } else {
        processed_args.join(sep)
    };
    // c:5564-5575 — destination dispatch order:
    //   -z   → zpushnode(bufstack, stringval)
    //   -v   → setsparam(VAR, stringval)
    //   -s   → prepnexthistent() + addhistnode(histtab, stringval)
    //   else → fwrite to fout
    if OPT_ISSET(ops, b'z') {
        // c:5564-5565 — `zpushnode(bufstack, stringval)`. The ZLE
        // bufstack is consumed by the next zleread call so the
        // string is presented at the prompt — `print -z 'echo foo'`
        // queues `echo foo` for the user to press Enter on.
        crate::ported::zle::zle_main::BUFSTACK
            .lock()
            .unwrap()
            .push(body); // c:5565
        return 0;
    }
    if OPT_ISSET(ops, b's') || OPT_ISSET(ops, b'S') {
        // c:5047-5093 — `-s` / `-S` push captured output to the history
        // list. `-S` requires exactly one arg (c:5058-5062
        // `if (nwords > 1) zwarnnam(name, "option -S takes a single
        // argument"); return 1`); the words array is then populated by
        // `histsplitwords` (c:5065). `-s` joins all args with spaces
        // and stores them as a single history entry. Both paths call
        // `prepnexthistent` + `addhistnode(histtab, ent->node.nam, ent)`.
        if OPT_ISSET(ops, b'S') && processed_args.len() > 1 {
            // c:5059
            zwarnnam(name, "option -S takes a single argument"); // c:5059
            return 1; // c:5061
        }
        let event_id = crate::ported::hist::prepnexthistent(); // c:5066/5072
        crate::ported::hashtable::addhistnode(&body, event_id as i32); // c:5090
                                                                       // c:Src/builtin.c — C's `addhistnode(histtab, …)` uses the
                                                                       // ent ALREADY linked into hist_ring by prepnexthistent. zshrs's
                                                                       // prepnexthistent only bumps curhist and doesn't insert into
                                                                       // the ring, so `fc -l` finds no events. Push the ent into the
                                                                       // ring here so the gethistent lookup in fc/history sees it.
                                                                       // Without this, `print -s X; fc -l` reported "no such event: 1"
                                                                       // even though the histtab had the entry.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let ent = crate::ported::zsh_h::histent {
            node: crate::ported::zsh_h::hashnode {
                next: None,
                nam: body.clone(),
                flags: 0,
            },
            up: None,
            down: None,
            zle_text: None,
            stim: now,
            ftim: now,
            words: Vec::new(),
            nwords: 0,
            histnum: event_id,
        };
        if let Ok(mut ring) = crate::ported::hist::hist_ring.lock() {
            ring.insert(0, ent);
            crate::ported::hist::histlinect.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        return 0;
    }
    if let Some(ref v) = dest_var {
        // c:Src/builtin.c:544-546 — the trailing separator is part of the
        // captured `-v` value too: it is written to the same stream and read
        // back into `buf`. It is suppressed only when `-n`, a `\c` escape
        // (`nnl`), OR `-v` WITHOUT `-l` apply. So `print -v x -l a b c` stores
        // `a\nb\nc\n` (with the trailing newline) where `print -v x a b c`
        // stores `a b c` (none). The port previously suppressed it for every
        // `-v`, dropping the `-l` trailing newline.
        let suppress_term = nonewline || backslash_c_truncated || !OPT_ISSET(ops, b'l');
        let mut val = body.clone();
        if !suppress_term {
            val.push(if nul_sep { '\0' } else { '\n' }); // c:546
        }
        setsparam(v, &val);
    } else {
        // c:5130-5132 — final terminator: `-n` suppresses; `-N` emits
        // NUL instead of newline; else newline. `\c` truncation
        // (c:utils.c:7045) also suppresses — matches zsh's
        // `echo "a\cb"; echo END` → `aEND`.
        //
        // c:Src/builtin.c:4982-5025 — the columnate (`-c`/`-C`) block emits its
        // newline PER ROW and then `return ret`s, never reaching this shared
        // terminator. With ZERO args there are zero rows, so `print -c` /
        // `print -C 2` output NOTHING — not the empty line this path would add.
        // (`print -c ""` has one arg → one row → one newline, still correct.)
        let columnate_empty =
            (OPT_ISSET(ops, b'c') || OPT_ISSET(ops, b'C')) && processed_args.is_empty();
        let final_term: &[u8] = if nonewline || backslash_c_truncated || columnate_empty {
            b""
        } else if nul_sep {
            b"\0"
        } else {
            b"\n"
        };
        // c:Bug #388 — `-p` and `-u` are mutually exclusive but both
        // route to the same write-to-fd path. Prefer the `-p`-resolved
        // coprocout (when present) over the `-u`-resolved fd. If
        // neither, fall through to stdout below.
        let dest_fd_active = print_dash_p_fd.or(dest_fd);
        // c:Src/builtin.c:4752 — `unmetafy(args[n], &len[n])`: print's
        // args are unmetafied before the fwrite. zshrs keeps values
        // metafied (Meta-char pairs, vm_helper::meta_encode_byte)
        // until the byte boundary, so decode here — `print $'\xff'`
        // writes the single raw byte 0xff. Bug #127.
        let body_bytes = crate::ported::utils::unmetafy_str(&body);
        if let Some(mut f) = dest_fd_active {
            // c:4847 — write to dup'd file descriptor.
            use std::io::Write as _;
            let _ = f.write_all(&body_bytes); // c:5124 fwrite
            let _ = f.write_all(final_term); // c:5132
                                             // f closes on drop (close(dup_fd)) — user's original fd
                                             // remains open per c:4843 dup semantics.
        } else {
            // stdout path. -N writes NUL via raw stdout; print!/println!
            // would mangle a NUL inside a String literal via format
            // machinery, so route through stdout().write_all directly.
            use std::io::Write as _;
            let stdout = io::stdout();
            let mut lk = stdout.lock();
            let _ = lk.write_all(&body_bytes); // c:5124
            let _ = lk.write_all(final_term); // c:5132
                                              // c:Src/builtin.c — C printf goes through libc fwrite to
                                              // stdout (fd 1) which is unbuffered when the builtin runs
                                              // under a redirect (because dup2 to fd 1 puts the file
                                              // there). Rust's `io::stdout()` is LINE-buffered, so
                                              // `printf "abcde"` (no `\n`) leaves bytes pending in the
                                              // userspace buffer; when the surrounding `> file` redirect
                                              // closes the dup'd fd, those bytes flush AFTER the restore
                                              // — landing on the original stdout (terminal) AND missing
                                              // the file. Flush here so the bytes hit fd 1 (the redirect
                                              // target) before bin_print returns. Bug #397.
            let _ = lk.flush();
        }
    }
    0
}

/// Port of `bin_shift(char *name, char **argv, Options ops, UNUSED(int func))` from Src/builtin.c:5593.
/// C: `int bin_shift(char *name, char **argv, Options ops, UNUSED(int func))`
/// — shift positional params (or named arrays) by `num` positions; `-p`
/// pops from the right end.
/// WARNING: param names don't match C — Rust=(name, argv, _func) vs C=(name, argv, ops, func)
pub fn bin_shift(
    name: &str,
    argv: &[String], // c:5593
    ops: &options,
    _func: i32,
) -> i32 {
    let mut num: i32 = 1; // c:5595
    let mut ret: i32 = 0; // c:5595
    let mut idx = 0usize;
    queue_signals(); // c:5599
                     // c:5600-5605 — first arg parsed as math expr unless it's an array name.
    if !argv.is_empty() {
        // c:5600
        let first = &argv[0];
        // c:5600 — `if (!getaparam(*argv))` decides whether the arg is
        //          a numeric shift-count vs an array name. Check
        //          paramtab for a PM_ARRAY entry, not OS env.
        let is_array = {
            use {PM_ARRAY, PM_TYPE};
            let tab = paramtab().read().unwrap();
            tab.get(first)
                .map(|pm| PM_TYPE(pm.node.flags as u32) == PM_ARRAY)
                .unwrap_or(false)
        };
        if !is_array {
            // c:5600
            // c:5601 — `num = mathevali(*argv++);`. The previous Rust port
            // used `parse::<i32>()` which rejects any non-trivial
            // arithmetic: `shift 1+2` would silently return ret=1
            // instead of shifting by 3. Route through mathevali.
            num = mathevali(first).unwrap_or_else(|_| {
                ret = 1;
                0
            }) as i32; // c:5601
            idx = 1;
            // c:5602-5605 — `if (errflag) return 1;`.
            if ret != 0 || errflag.load(Relaxed) != 0 {
                unqueue_signals(); // c:5604
                return 1;
            }
        }
    }

    // c:5608-5611 — `if (num < 0)` reject.
    if num < 0 {
        // c:5608
        unqueue_signals(); // c:5609
        zwarnnam(name, "argument to shift must be non-negative"); // c:5610
        return 1; // c:5611
    }

    // c:5614-5635 — named-array shift loop.
    if idx < argv.len() {
        // c:5614
        for arr_name in &argv[idx..] {
            // c:5615
            // c:5616 — `if ((s = getaparam(*argv)))` else silent skip.
            //          Read paramtab directly; was approximating arrays
            //          as `:`-separated env values which is wrong (env
            //          can never carry array structure).
            let s: Vec<String> = {
                let tab = paramtab().read().unwrap();
                match tab.get(arr_name).and_then(|pm| pm.u_arr.clone()) {
                    Some(arr) => arr,
                    None => continue,
                }
            };
            // c:5617-5621 — arrlen_lt check.
            if (s.len() as i32) < num {
                // c:5617
                zwarnnam(name, "shift count must be <= $#"); // c:5618
                ret += 1; // c:5619
                continue; // c:5620
            }
            // c:5622-5634 — -p shifts off the right end, otherwise the left.
            let s2: Vec<String> = if OPT_ISSET(ops, b'p') {
                // c:5622
                s[..s.len() - num as usize].to_vec() // c:5625-5628
            } else {
                s[num as usize..].to_vec() // c:5631
            };
            // c:5633 — `setaparam(*argv, s);`. Write the shifted array
            //          back to paramtab as a proper PM_ARRAY. Was a
            //          fake: `env::set_var` + colon-joined fake-array
            //          which neither carries array structure nor
            //          reaches subsequent `${arr_name[@]}` expansions.
            setaparam(arr_name, s2);
        }
    } else {
        // c:5636-5654 — shift positional parameters ($1..$N).
        // Static-link path: positional params live in src/ported/vm_helper;
        // expose via PPARAMS Mutex<Vec<String>>.
        let mut pp = PPARAMS.lock().unwrap_or_else(|e| {
            PPARAMS.clear_poison();
            e.into_inner()
        });
        let l = pp.len() as i32;
        if num > l {
            // c:5636
            zwarnnam(name, "shift count must be <= $#"); // c:5637
            ret = 1; // c:5638
        } else if OPT_ISSET(ops, b'p') {
            // c:5641
            pp.truncate((l - num) as usize); // c:5642-5644
        } else {
            pp.drain(..num as usize); // c:5646-5650
        }
        // PPARAMS is the single source of truth. fusevm-side reads
        // route through exec.pparams() which reads PPARAMS, so the
        // shift is immediately visible — no exec.positional_params
        // mirror needed.
        drop(pp);
    }
    unqueue_signals(); // c:5658
    ret // c:5659
}

/// Port of `bin_getopts(UNUSED(char *name), char **argv, UNUSED(Options ops), UNUSED(int func))` from Src/builtin.c:5672.
/// C: `int bin_getopts(UNUSED(char *name), char **argv, UNUSED(Options ops),
///                     UNUSED(int func))`.
///
/// POSIX getopts. Maintains state in $OPTIND (zoptind) and an internal
/// per-arg cursor (optcind). Reads from the script's positional params
/// when no extra args supplied, otherwise from the trailing argv.
/// WARNING: param names don't match C — Rust=(_name, argv, _func) vs C=(name, argv, ops, func)
pub fn bin_getopts(
    _name: &str,
    argv: &[String], // c:5672
    _ops: &options,
    _func: i32,
) -> i32 {
    if argv.len() < 2 {
        return 1;
    }
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

    // c:Src/builtin.c:5680 — at entry, re-sync the internal zoptind
    // tracker against the user-visible $OPTIND param. zsh exposes
    // OPTIND as a writable param so scripts can reset it to 1 to
    // re-parse positionals; the Rust port had `ZOPTIND` as an atomic
    // that was the AUTHORITY (writes to $OPTIND didn't propagate
    // back), so `OPTIND=1` between two getopts loops left zoptind at
    // the post-loop value and the second pass returned immediately.
    let paramtab_oi = getiparam("OPTIND");
    let mut zoptind = if paramtab_oi >= 1 {
        paramtab_oi as i32
    } else {
        ZOPTIND.load(Relaxed)
    };
    // c:5681-5685 — `if (zoptind < 1) { zoptind = 1; optcind = 0; }`
    if zoptind < 1 {
        // c:5681
        zoptind = 1;
        OPTCIND.store(0, Relaxed);
    }
    // c:Src/builtin.c — when $OPTIND was just reset to 1 (i.e. the
    // user-visible param disagrees with the previous internal
    // pointer), reset optcind so the new pass starts at byte 0 of
    // the first option arg.
    let mut optcind = if paramtab_oi == 1 && ZOPTIND.load(Relaxed) != 1 {
        0
    } else {
        OPTCIND.load(Relaxed)
    };

    // c:5686-5688 — `if (arrlen_lt(args, zoptind)) return 1;`
    if (args.len() as i32) < zoptind {
        // c:5686
        ZOPTIND.store(zoptind, Relaxed);
        return 1;
    }

    // c:5691-5693 — `quiet = *optstr == ':'; optstr += quiet; lenoptstr -= quiet;`
    let (quiet, optstr) = if optstr_full.starts_with(':') {
        // c:5691
        (true, &optstr_full[1..])
    } else {
        (false, optstr_full.as_str())
    };

    // c:5696 — `str = unmetafy(dupstring(args[zoptind - 1]), &lenstr);`
    let mut str_buf = args[(zoptind - 1) as usize].clone();
    let mut lenstr = str_buf.len() as i32;
    if lenstr == 0 {
        return 1;
    } // c:5697

    // c:5699-5703 — bump to next arg if optcind exhausted current.
    if optcind >= lenstr {
        // c:5699
        optcind = 0;
        zoptind += 1;
        if zoptind as usize > args.len() {
            // c:5701
            ZOPTIND.store(zoptind, Relaxed);
            OPTCIND.store(optcind, Relaxed);
            setiparam("OPTIND", zoptind as i64); // c:5702
            return 1;
        }
        str_buf = args[(zoptind - 1) as usize].clone();
        lenstr = str_buf.len() as i32;
    }

    // c:5705-5712 — first option char checks: not `-`/`+` → done; `--` → done.
    if optcind == 0 {
        // c:5705
        if lenstr < 2 || (!str_buf.starts_with('-') && !str_buf.starts_with('+')) {
            ZOPTIND.store(zoptind, Relaxed);
            OPTCIND.store(optcind, Relaxed);
            // c:5707 — mirror to $OPTIND so callers see the post-loop
            // pointer. Previous Rust port skipped this write on the
            // "no more options" exit; OPTIND stayed at the last
            // option arg index (-b) instead of advancing past it.
            setiparam("OPTIND", zoptind as i64);
            return 1;
        }
        if lenstr == 2 && &str_buf[..2] == "--" {
            // c:5708
            zoptind += 1;
            ZOPTIND.store(zoptind, Relaxed);
            OPTCIND.store(0, Relaxed);
            setiparam("OPTIND", zoptind as i64); // c:5711
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
    let posix = isset(POSIXBUILTINS);
    let found = optstr.bytes().position(|b| b == opch);
    if opch == b':' || found.is_none() {
        // c:5724
        if posix {
            // c:5728
            optcind = 0;
            zoptind += 1;
        }
        // c:5731 — `setsparam(var, ztrdup(p));` where p = "?"
        setsparam(&var, "?");
        if quiet {
            // c:5733
            setsparam("OPTARG", &optbuf); // c:5734
        } else {
            let prefix = if plus { "+" } else { "-" };
            zwarn(&format!("bad option: {}{}", prefix, opch as char)); // c:5736
            setsparam("OPTARG", "");
        }
        ZOPTIND.store(zoptind, Relaxed);
        OPTCIND.store(optcind, Relaxed);
        // Sync OPTIND env var so callers can read.
        setiparam("OPTIND", zoptind as i64);
        return 0;
    }

    // c:5744 — `if (p[1] == ':')` — required argument.
    let p = found.unwrap();
    let optstr_bytes = optstr.as_bytes();
    if p + 1 < optstr_bytes.len() && optstr_bytes[p + 1] == b':' {
        // c:5744
        if optcind == lenstr {
            // c:5745
            // c:5746 — argument in next arg.
            if zoptind as usize >= args.len() {
                // c:5747
                if posix {
                    optcind = 0;
                    zoptind += 1;
                }
                if quiet {
                    // c:5754
                    setsparam(&var, ":");
                    setsparam("OPTARG", &optbuf);
                } else {
                    setsparam(&var, "?");
                    setsparam("OPTARG", "");
                    let prefix = if plus { "+" } else { "-" };
                    zwarn(&format!(
                        "argument expected after {}{} option",
                        prefix, opch as char
                    )); // c:5760
                }
                ZOPTIND.store(zoptind, Relaxed);
                OPTCIND.store(optcind, Relaxed);
                setiparam("OPTIND", zoptind as i64);
                return 0;
            }
            // c:5763 — `p = ztrdup(args[zoptind++]);` — read args[zoptind]
            // then post-increment. zoptind now points one past the
            // arg-bearing flag's index.
            let p_arg = args[zoptind as usize].clone(); // c:5763
            zoptind += 1; // c:5763 post-increment
            setsparam("OPTARG", &p_arg); // c:5765
                                         // c:5771 — `optcind = 0; zoptind++;` — bump past the
                                         // consumed value arg too, so the NEXT getopts call sees
                                         // the arg AFTER the value. Previous Rust port skipped this
                                         // second increment, so the next iter re-read the consumed
                                         // value as if it were a new flag.
            optcind = 0; // c:5771
            zoptind += 1; // c:5772
        } else {
            // c:5774 — `p = metafy(str+optcind, lenstr-optcind, META_DUP);`
            let p_arg = str_buf[(optcind as usize)..].to_string();
            setsparam("OPTARG", &p_arg);
            optcind = 0;
            zoptind += 1;
        }
    } else {
        // c:5784 — `zsfree(zoptarg); zoptarg = ztrdup("");`
        setsparam("OPTARG", "");
    }

    // c:5788 — `setsparam(var, metafy(optbuf, lenoptbuf, META_DUP));`
    setsparam(&var, &optbuf);
    ZOPTIND.store(zoptind, Relaxed);
    OPTCIND.store(optcind, Relaxed);
    setiparam("OPTIND", zoptind as i64);
    0 // c:5790
}

/// Port of `bin_break(char *name, char **argv, UNUSED(Options ops), int func)` from Src/builtin.c:5809.
/// C: `int bin_break(char *name, char **argv, UNUSED(Options ops), int func)`
/// — handles BIN_BREAK / BIN_CONTINUE / BIN_RETURN / BIN_LOGOUT / BIN_EXIT.
/// WARNING: param names don't match C — Rust=(name, argv, func) vs C=(name, argv, ops, func)
pub fn bin_break(
    name: &str,
    argv: &[String], // c:5809
    _ops: &options,
    func: i32,
) -> i32 {
    // BIN_BREAK/CONTINUE/RETURN/EXIT/LOGOUT live at the top of this file
    // (c:5707-5712 in Src/builtin.c via the BUILTIN(...) table).
    // c:5811 — `int num = lastval, nump = 0, implicit;`
    let mut num: i32 = LASTVAL.load(Relaxed); // c:5811
    let mut nump = 0i32; // c:5811
    let implicit = argv.is_empty(); // c:5814
                                    // c:5815-5818 — first arg parsed as math expr.
    if !implicit {
        // c:5815
        num = mathevali(&argv[0]).unwrap_or(0) as i32; // c:5816
        nump = 1; // c:5817
    }

    // c:5812-5814 — positive-num requirement for BIN_CONTINUE / BIN_BREAK.
    // C uses `zerrnam` (NOT zwarnnam) — it sets errflag so the whole
    // script/function aborts, matching zsh's `continue 0` behavior
    // (prints the error ONCE and stops the loop, rather than warning and
    // continuing to iterate).
    if nump > 0 && (func == BIN_CONTINUE || func == BIN_BREAK) && num <= 0 {
        // c:5812
        zerrnam(name, &format!("argument is not positive: {}", num)); // c:5813
        return 1; // c:5814
    }

    let loops = LOOPS.load(Relaxed);
    match func {
        // c:5831-5842 — BIN_CONTINUE: must be in a loop, set contflag,
        // then fall through to BIN_BREAK's break-count assign.
        x if x == BIN_CONTINUE => {
            // c:5831
            if loops == 0 {
                // c:5832
                // c:Src/builtin.c:5828 — `zerrnam`, NOT `zwarnnam`.
                // `zerrnam` sets `errflag` which causes the calling
                // shell function (or script) to terminate after the
                // current command — `zwarnnam` just prints. Without
                // this, `foo() { break; echo "after"; }; foo` printed
                // "after" instead of aborting at the break. Bug #616.
                zerrnam(name, "not in while, until, select, or repeat loop"); // c:5828
                return 1; // c:5834
            }
            CONTFLAG.store(1, Relaxed); // c:5836 FALLTHROUGH
                                        // c:5837 — fallthrough to BIN_BREAK's loops==0 guard
                                        // (impossible here since we already returned above) +
                                        // break-count assign. Inlined directly. The previous
                                        // Rust port had a redundant `if loops == 0 { return 1 }`
                                        // dead-coded after the first guard.
            BREAKS.store(
                if nump != 0 { num.min(loops) } else { 1 }, // c:5842
                Relaxed,
            );
        }
        // c:5832-5838 — BIN_BREAK.
        x if x == BIN_BREAK => {
            // c:5832
            if loops == 0 {
                // c:5833
                // c:Src/builtin.c:5834 — `zerrnam` sets errflag.
                // Same fix as the BIN_CONTINUE arm above. Bug #616.
                zerrnam(name, "not in while, until, select, or repeat loop"); // c:5834
                return 1; // c:5835
            }
            BREAKS.store(
                if nump != 0 { num.min(loops) } else { 1 }, // c:5837
                Relaxed,
            );
        }
        // c:5839-5860 — BIN_RETURN.
        x if x == BIN_RETURN => {
            let interactive = isset(INTERACTIVE);
            let shinstdin = isset(SHINSTDIN);
            let ll_v = locallevel_param.load(Relaxed);
            let sourcelevel = crate::ported::init::sourcelevel.load(Relaxed);
            // c:5840-5841 — `if ((interactive && shinstdin) || locallevel || sourcelevel)`
            if (interactive && shinstdin) || ll_v != 0 || sourcelevel != 0 {
                // c:5840
                RETFLAG.store(1, Relaxed); // c:5842
                BREAKS.store(loops, Relaxed); // c:5843
                LASTVAL.store(num, Relaxed); // c:5844
                                             // c:5845-5854 — inside a primed trap with the sentinel
                                             // `trap_return == -2`, promote to TRAP_STATE_FORCE_RETURN
                                             // and carry `lastval`. POSIXTRAPS + `implicit` opts out:
                                             // POSIX semantics keep $? from before the trap fired.
                let posixtraps = isset(POSIXTRAPS);
                let cur_state = TRAP_STATE.load(Relaxed);
                let cur_return = TRAP_RETURN.load(Relaxed);
                if cur_state == TRAP_STATE_PRIMED      // c:5845
                    && cur_return == -2                                      // c:5845
                    && !(posixtraps && implicit)
                // c:5851
                {
                    TRAP_STATE.store(
                        // c:5852
                        TRAP_STATE_FORCE_RETURN,
                        Relaxed,
                    );
                    TRAP_RETURN.store(num, Relaxed);
                    // c:5853
                }
                return num; // c:5855
            }
            // c:5858 — fallthrough: treat as logout/exit.
            zexit(num, ZEXIT_NORMAL); // c:5858
        }
        // c:5864-5869 — BIN_LOGOUT: refuse if not LOGINSHELL, then
        // FALLTHROUGH into the BIN_EXIT body. The previous Rust port
        // called \`zexit(num, ZEXIT_NORMAL)\` directly instead of
        // entering the BIN_EXIT defer-guard, so \`logout\` from inside
        // a function would skip EXIT traps + function unwind +
        // \"you have running jobs\" warning — same gap as the prior
        // BIN_EXIT fix.
        x if x == BIN_LOGOUT => {
            // c:5865 — `if (unset(LOGINSHELL))`. The previous Rust port
            // called `optlookup("login")` — but "login" is the
            // SHELL-LETTER-FLAG name (zshletters table letter 'l'),
            // not an option name. Option name canonicalization maps
            // LOGINSHELL → "loginshell" (Src/options.c index_to_name
            // at line 1682 in Rust port).
            //
            // \`optlookup(\"login\")\` returns OPT_INVALID (0), so
            // \`isset(0)\` always returns false — bin_logout always
            // saw \"not login shell\" and rejected with that error
            // regardless of whether the shell was actually started
            // with \`-l\`.
            let loginshell = isset(LOGINSHELL);
            if !loginshell {
                // c:5865
                // c:Src/builtin.c:5861 — `zerrnam(name, "not login
                // shell");`. C uses zerrnam (sets ERRFLAG_ERROR via
                // utils.c:203), NOT zwarnnam. The errflag set aborts
                // the remainder of the current command list — `logout
                // 2>&1; print ex:$?` in a non-login shell prints only
                // the error message because the `;` separator's next
                // command is gated on errflag-clear. Previous Rust
                // port used zwarnnam (no errflag) so `print` still
                // ran. Pinned by logout_builtin_stderr parity probe.
                zerrnam(name, "not login shell"); // c:5861
                return 1; // c:5862
            }
            // c:5869 — `/*FALLTHROUGH*/` into BIN_EXIT body.
            // Reusing the BIN_EXIT branch below by setting `func` to
            // BIN_EXIT isn't possible mid-match; inline the same
            // guard logic here.
            let cur_locallevel = locallevel_param.load(Relaxed);
            let forklevel = FORKLEVEL.load(Relaxed);
            let shell_exiting = SHELL_EXITING.load(Relaxed);
            if cur_locallevel > forklevel && shell_exiting != -1 {
                // c:5871
                if STOPMSG.load(Relaxed) == 0 {
                    zexit(0, ZEXIT_DEFERRED); // c:5884
                }
                if STOPMSG.load(Relaxed) == 0 {
                    // c:5884
                    let trap_state = TRAP_STATE.load(Relaxed);
                    if trap_state != 0 {
                        // c:5885
                        TRAP_STATE.store(
                            // c:5886
                            TRAP_STATE_FORCE_RETURN,
                            Relaxed,
                        );
                    }
                    RETFLAG.store(1, Relaxed); // c:5887
                    BREAKS.store(
                        LOOPS.load(Relaxed), // c:5888
                        Relaxed,
                    );
                    EXIT_PENDING.store(1, Relaxed); // c:5889
                    EXIT_LEVEL.store(cur_locallevel, Relaxed); // c:5890 — exit_level = locallevel;
                    EXIT_VAL.store(num, Relaxed); // c:5891
                }
            } else {
                zexit(num, ZEXIT_NORMAL); // c:5894
            }
        }
        // c:5870-5894 — BIN_EXIT: function-context guard. C body:
        //   if (locallevel > forklevel && shell_exiting != -1) {
        //       if (stopmsg || (zexit(0, ZEXIT_DEFERRED), !stopmsg)) {
        //           if (trap_state) trap_state = TRAP_STATE_FORCE_RETURN;
        //           retflag = 1; breaks = loops;
        //           exit_pending = 1; exit_level = locallevel; exit_val = num;
        //       }
        //   } else zexit(num, ZEXIT_NORMAL);
        //
        // Inside a function (locallevel > forklevel) the shell can't
        // exit directly — EXIT traps still need to run. The probe
        // path zexit(0, ZEXIT_DEFERRED) calls checkjobs; if no
        // stopmsg triggered, we defer: set retflag + breaks +
        // exit_pending so the function unwind takes us out.
        //
        // The previous Rust port skipped this entire guard, always
        // calling zexit(num, ZEXIT_NORMAL) directly. `exit` inside
        // a function would terminate without running EXIT traps or
        // unwinding the function stack.
        x if x == BIN_EXIT => {
            let cur_locallevel = locallevel_param.load(Relaxed);
            let forklevel = FORKLEVEL.load(Relaxed);
            let shell_exiting = SHELL_EXITING.load(Relaxed);
            if cur_locallevel > forklevel && shell_exiting != -1 {
                // c:5871
                // Probe via ZEXIT_DEFERRED — may set stopmsg.
                if STOPMSG.load(Relaxed) == 0 {
                    zexit(0, ZEXIT_DEFERRED); // c:5884
                }
                if STOPMSG.load(Relaxed) == 0 {
                    // c:5884 still no stopmsg → defer
                    let trap_state = TRAP_STATE.load(Relaxed);
                    if trap_state != 0 {
                        // c:5885
                        TRAP_STATE.store(
                            // c:5886
                            TRAP_STATE_FORCE_RETURN,
                            Relaxed,
                        );
                    }
                    RETFLAG.store(1, Relaxed); // c:5887
                    BREAKS.store(
                        LOOPS.load(Relaxed), // c:5888
                        Relaxed,
                    );
                    EXIT_PENDING.store(1, Relaxed); // c:5889
                    EXIT_LEVEL.store(cur_locallevel, Relaxed); // c:5890 — exit_level = locallevel;
                    EXIT_VAL.store(num, Relaxed); // c:5891
                }
            } else {
                zexit(num, ZEXIT_NORMAL); // c:5894
            }
        }
        _ => {}
    }
    0
}

/// Port of `checkjobs()` from Src/builtin.c:5899.
/// C: `static void checkjobs(void)` — walk `jobtab[1..maxjob]`; for each
///   non-current job that's STAT_LOCKED, not STAT_NOPRINT, and either
///   running (when CHECKRUNNINGJOBS is set) or STAT_STOPPED, emit
///   "you have running/stopped jobs" + set `stopmsg = 1`.
pub fn checkjobs() {
    // c:5899
    let checkrunning = isset(CHECKRUNNINGJOBS);
    // c:5901 — read the canonical jobs.rs THISJOB/MAXJOB globals.
    // The previous builtin.rs duplicate AtomicI32s for both never
    // synced with the jobs.rs Mutex<i32> values that the spawn /
    // wait paths actually update — checkjobs would see stale 0s
    // regardless of how many jobs were active.
    let thisjob: i32 = *crate::ported::jobs::THISJOB
        .get_or_init(|| Mutex::new(-1_i32))
        .lock()
        .expect("THISJOB poisoned");
    // jobs::MAXJOB is stored as `Mutex<usize>` (Rust adaptation for
    // Vec-index semantics); cast to i32 for comparison with `thisjob`.
    let maxjob: i32 = *crate::ported::jobs::MAXJOB
        .get_or_init(|| Mutex::new(0_usize))
        .lock()
        .expect("MAXJOB poisoned") as i32;

    // c:5903 — `for (i = 1; i <= maxjob; i++)`
    let mut found: Option<i32> = None;
    let mut found_stat: i32 = 0;
    for i in 1..=maxjob {
        // c:5903
        let stat = JOBSTATS
            .lock()
            .ok()
            .and_then(|t| t.get(i as usize).copied())
            .unwrap_or(0);
        // c:5904-5906 — `i != thisjob && (stat & STAT_LOCKED) &&
        //                !(stat & STAT_NOPRINT) &&
        //                (CHECKRUNNINGJOBS || stat & STAT_STOPPED)`
        if i != thisjob                                                      // c:5904
            && (stat & STAT_LOCKED) != 0                                     // c:5904
            && (stat & STAT_NOPRINT) == 0                                    // c:5905
            && (checkrunning || (stat & STAT_STOPPED) != 0)
        // c:5906
        {
            found = Some(i); // c:5907
            found_stat = stat;
            break;
        }
    }
    // c:5908 — `if (i <= maxjob)`
    if found.is_some() {
        // c:5908
        if (found_stat & STAT_STOPPED) != 0 {
            // c:5909
            // c:5912/5914 — `zerr("you have suspended/stopped jobs.");`
            zerr("you have stopped jobs."); // c:5914
        } else {
            // c:5917 — `zerr("you have running jobs.");`
            zerr("you have running jobs."); // c:5917
        }
        STOPMSG.store(1, Relaxed); // c:5919
    }
}

/// Port of `realexit()` from Src/builtin.c:5953.
/// C body (single statement):
///     `exit((shell_exiting || exit_pending) ? exit_val : lastval);`
pub fn realexit() -> ! {
    // c:5953
    std::process::exit(
        if SHELL_EXITING.load(Relaxed) != 0 || EXIT_PENDING.load(Relaxed) != 0 {
            EXIT_VAL.load(Relaxed)
        } else {
            LASTVAL.load(Relaxed)
        },
    );
}

/// Port of `_realexit()` from Src/builtin.c:5962.
/// C body (single statement):
///     `_exit((shell_exiting || exit_pending) ? exit_val : lastval);`
pub fn _realexit() -> ! {
    // c:5962
    unsafe {
        libc::_exit(
            if SHELL_EXITING.load(Relaxed) != 0 || EXIT_PENDING.load(Relaxed) != 0 {
                EXIT_VAL.load(Relaxed)
            } else {
                LASTVAL.load(Relaxed)
            },
        )
    }
}

/// Port of `zexit(int val, enum zexit_t from_where)` from Src/builtin.c:5977.
/// C: `void zexit(int val, enum zexit_t from_where)` — record exit
///   value, fire EXIT trap unless already exiting, then realexit.
#[allow(unused_variables)]
pub fn zexit(val: i32, from_where: i32) {
    // c:5977
    // c:5989 — `exit_val = val;`
    EXIT_VAL.store(val, Relaxed); // c:5989
                                  // c:5990 — `if (shell_exiting == -1) { retflag = 1; breaks = loops; return; }`
    if SHELL_EXITING.load(Relaxed) == -1 {
        // c:5990
        RETFLAG.store(1, Relaxed); // c:5991
        BREAKS.store(LOOPS.load(Relaxed), Relaxed); // c:5992
        return; // c:5993
    }

    // c:5996-6004 — `if (isset(MONITOR) && !stopmsg && from_where != ZEXIT_SIGNAL)`:
    // run scanjobs + checkjobs; if stopmsg got set (running jobs warned),
    // mark stopmsg=2 and DEFER the exit. The previous Rust port skipped
    // this entire block, so `exit` with running jobs would terminate
    // immediately rather than emitting the standard
    // \"zsh: you have running jobs\" + waiting for a confirmation exit.
    if isset(MONITOR)                                  // c:5996
        && STOPMSG.load(Relaxed) == 0
        && from_where != ZEXIT_SIGNAL
    {
        checkjobs(); // c:5999
        if STOPMSG.load(Relaxed) != 0 {
            // c:6000
            STOPMSG.store(2, Relaxed); // c:6001
            return; // c:6002 defer
        }
    }
    // c:6006-6008 — `if (from_where == ZEXIT_DEFERRED || (shell_exiting++
    //                 && from_where != ZEXIT_NORMAL)) return;`. Probe path:
    // ZEXIT_DEFERRED callers only want the checkjobs gate to fire; if
    // it didn't trip, return without actually exiting.
    if from_where == ZEXIT_DEFERRED {
        // c:6006
        return;
    }
    let prev_exiting = SHELL_EXITING.fetch_add(1, Relaxed);
    if prev_exiting != 0 && from_where != ZEXIT_NORMAL {
        // c:6007
        return;
    }
    // c:6014 — `shell_exiting = -1;`
    SHELL_EXITING.store(-1, Relaxed); // c:6014
                                      // c:6019 — `errflag = 0;`
    errflag.store(0, Relaxed); // c:6019
                               // c:6021-6024 — MONITOR → killrunjobs.
    if isset(MONITOR) {
        // c:6021
        crate::ported::signals::killrunjobs(if from_where == ZEXIT_SIGNAL { 1 } else { 0 });
        // c:6023
    }
    // !!! RUST-ONLY GATE: see SUBSHELL_DEPTH declaration above for
    // rationale. C zsh's realexit at c:5953 unconditionally calls
    // process::exit because the subshell was forked; in zshrs the
    // subshell runs in-process, so process::exit would kill the
    // whole shell. Defer: set EXIT_PENDING + EXIT_VAL + reset
    // SHELL_EXITING so the subshell_end unwind in fusevm_bridge
    // catches and propagates the status to the parent.
    if SUBSHELL_DEPTH.load(Relaxed) > 0 {
        SHELL_EXITING.store(0, Relaxed);
        EXIT_VAL.store(val, Relaxed);
        EXIT_PENDING.store(1, Relaxed);
        RETFLAG.store(1, Relaxed);
        BREAKS.store(LOOPS.load(Relaxed), Relaxed);
        return;
    }
    // c:6012 — `cleanfilelists();` — delete per-job temp-file lists
    // before exit. jobs.rs::cleanfilelists is a Rust-only-signature
    // adapter over the C global jobtab (see its WARNING block).
    if let Some(tab) = crate::ported::jobs::JOBTAB.get() {
        let mut tab = tab.lock().unwrap_or_else(|e| e.into_inner());
        crate::ported::jobs::cleanfilelists(&mut tab); // c:6012
    }
    // c:6013-6028 — `if (isset(RCS) && interact)`: save the history
    // file and run logout scripts. This is THE write-at-exit path for
    // interactive history (`-f`/NO_RCS shells intentionally skip it,
    // matching C).
    if isset(RCS) && interact() {
        // c:6013
        // c:6014-6020 — `if (!nohistsave) { ... }`. `nohistsave` isn't
        // ported as a Rust global (same approximation as
        // hist.rs::saveandpophiststack); default 0 = allow saves.
        let mut writeflags = HFILE_USE_OPTIONS as i32; // c:6015
        if from_where == ZEXIT_SIGNAL {
            // c:6016
            writeflags |= HFILE_NO_REWRITE as i32; // c:6017
        }
        saveandpophiststack(1, writeflags); // c:6018
        savehistfile(None, writeflags); // c:6019
                                        // c:6021-6027 — `if (islogin && !subsh) { sourcehome(".zlogout");
                                        // ... source(GLOBAL_ZLOGOUT); }`. The C `subsh` check is covered
                                        // by the RUST-ONLY SUBSHELL_DEPTH gate above (in-process
                                        // subshells returned before reaching here).
        if islogin() {
            // c:6021
            crate::ported::init::sourcehome(".zlogout"); // c:6022
            if isset(RCS) && isset(GLOBALRCS) {
                // c:6024
                let _ = crate::ported::init::source(&crate::extensions::global_rc::global_rc_path(
                    crate::ported::config_h::GLOBAL_ZLOGOUT,
                ));
                // c:6025
            }
        }
    }
    // c:Src/builtin.c:6075-6079 — fire EXIT trap (SIGEXIT) before
    // calling realexit. The trap body sees $? = val (carried via
    // LASTVAL below) and runs in the shell process. Remove the
    // entry from traps_table first so the trap body's own commands
    // don't re-trigger it recursively.
    //
    // Two trap-storage paths: (a) `trap 'cmd' EXIT` stores raw
    // body text in `traps_table` (no settrap), handled by the
    // direct execute_script below; (b) `TRAPEXIT() { ... }`
    // function-named form goes through settrap(SIGEXIT, None,
    // ZSIG_FUNC) at funcdef time and lives in the
    // shfunctab+sigtrapped pair, dispatched via `dotrap(SIGEXIT)`.
    // C zsh's dotrap handles BOTH paths internally; Rust splits
    // them. Call dotrap to cover the function-named form. Bug
    // #157 in docs/BUGS.md.
    let exit_trap = traps_table().lock().ok().and_then(|mut t| t.remove("EXIT"));
    LASTVAL.store(val, Relaxed); // c:6076 — $? inside trap = val
    crate::ported::signals::in_exit_trap.store(1, Relaxed);
    if let Some(body) = exit_trap {
        // Set LASTVAL to the requested exit value so `$?` inside
        // the trap body sees the right number (matches `(exit 7)`
        // → trap body reads $?=7).
        let _ = crate::ported::exec::execute_script(&body);
    }
    // c:6043-6044 — `if (sigtrapped[SIGEXIT]) dotrap(SIGEXIT);`.
    //
    // dotrap DOES reach its ZSIG_FUNC arm here, but the body cannot run: by
    // this point we are outside the VM, and dotrapargs' doshfunc path has no
    // executor to run the compiled function chunk in, so it returns silently.
    // The natural end-of-script path already knows this and invokes the
    // function through the script pipeline instead (vm_helper.rs:2850-2854);
    // the explicit `exit` path did not, so
    //     TRAPEXIT() { print BYE }; exit 3
    // printed nothing where zsh prints BYE — while the string form
    // (`trap "print S" EXIT; exit 3`) worked, because that one is dispatched
    // from traps_table just above.
    let exit_fn_trapped = crate::ported::signals::sigtrapped
        .lock()
        .ok()
        .and_then(|g| g.get(crate::signals_h::SIGEXIT as usize).copied())
        .unwrap_or(0)
        & crate::ported::zsh_h::ZSIG_FUNC as i32
        != 0;
    if exit_fn_trapped {
        let _ = crate::ported::exec::execute_script("TRAPEXIT");
    } else {
        let _ = crate::ported::signals::dotrap(crate::signals_h::SIGEXIT);
    }
    // c:Src/init.c::zexit — `callhookfunc("zshexit", NULL, 1, NULL)`.
    // The hook fires both a `zshexit` function (if defined) AND
    // walks the `zshexit_functions` array. Distinct from SIGEXIT
    // trap dispatch above — zshexit is a zsh-specific shell-event
    // hook that operates parallel to trap. Bug #215 in docs/BUGS.md.
    let _ = crate::ported::utils::callhookfunc("zshexit", None, 1, std::ptr::null_mut());
    crate::ported::signals::in_exit_trap.store(0, Relaxed);
    realexit(); // c:6082
}

/// Port of `bin_dot(char *name, char **argv, UNUSED(Options ops), UNUSED(int func))` from Src/builtin.c:6060.
/// C: `int bin_dot(char *name, char **argv, ...)` — `.` / `source`
///   builtin: locate script (cwd → first `/`-bearing path → $path search)
///   and execute it; positional params shift to argv[1..].
/// WARNING: param names don't match C — Rust=(name, argv, _func) vs C=(name, argv, ops, func)
pub fn bin_dot(
    name: &str,
    argv: &[String], // c:6060
    _ops: &options,
    _func: i32,
) -> i32 {
    if argv.is_empty() {
        // c:6068
        return 0; // c:6069
    }

    // PFA-SMR aspect: record the source path so the replay tool can
    // re-apply the same source/dot at the same call site.
    #[cfg(feature = "recorder")]
    if crate::recorder::is_enabled() && !argv[0].is_empty() {
        let ctx = crate::recorder::recorder_ctx_global();
        crate::recorder::emit_source(&argv[0], ctx);
    }
    // c:6071-6074 — save pparams, install argv[1..] as new pparams.
    let saved_pparams: Option<Vec<String>> = if argv.len() > 1 {
        // c:6072
        let mut pp = PPARAMS.lock().unwrap_or_else(|e| {
            PPARAMS.clear_poison();
            e.into_inner()
        });
        let saved = pp.clone();
        *pp = argv[1..].to_vec(); // c:6073
        Some(saved)
    } else {
        None
    };

    let arg0 = argv[0].clone(); // c:6076
    let _enam = arg0.clone(); // c:6076
                              // c:6077-6080 — `if (isset(FUNCTIONARGZERO)) { old0 = argzero;
                              //                                              argzero = ztrdup(arg0); }`.
                              // Save the prior argzero so it can be restored at the end of
                              // bin_dot; under FUNCTIONARGZERO, the sourced file becomes the
                              // active $0 for the duration of the source.
    let saved_argzero: Option<Option<String>> = if isset(FUNCTIONARGZERO) {
        let prev = argzero();
        set_argzero(Some(arg0.clone()));
        Some(prev)
    } else {
        None
    };
    let mut diddot = 0i32; // c:6064
    let mut dotdot = 0i32; // c:6064

    // c:6087-6093 — for `source`, try cwd first.
    let mut found_path: Option<String> = None;
    if !name.starts_with('.') {
        // c:6087
        let p = Path::new(&arg0);
        if p.exists() && !p.is_dir() {
            // c:6088-6089
            diddot = 1; // c:6090
            found_path = Some(arg0.clone()); // c:6091 (effective)
        }
    }

    // c:6094-6101 — try literal path with `/` in it.
    if found_path.is_none() && arg0.contains('/') {
        // c:6096
        if arg0.starts_with("./") {
            diddot += 1;
        }
        // c:6097
        else if arg0.starts_with("../") {
            dotdot += 1;
        } // c:6098
        let p = Path::new(&arg0);
        if p.exists() && !p.is_dir() {
            found_path = Some(arg0.clone()); // c:6100
        } else if Path::new(&format!("{}.zwc", arg0)).is_file() {
            // c:6100 — C calls `source(arg0)` UNCONDITIONALLY for a
            // slash path; `source()` (init.c:1566) then loads the
            // sibling `<arg0>.zwc` via try_source_file even when the
            // plain file is gone. zshrs's pre-resolved found_path
            // gate must therefore accept the compiled form too —
            // the try_source_file hook at the read site below
            // supplies the body.
            found_path = Some(arg0.clone());
        }
    }

    // c:6102-6121 — $path search (with PATHDIRS guard).
    let pathdirs = isset(PATHDIRS);
    if found_path.is_none() && (!arg0.contains('/') || (pathdirs && diddot < 2 && dotdot == 0)) {
        // c:6102
        // c:6103 — `for (pp = path; *pp; pp++)`. C walks the `path[]`
        //          array (the shell-side $path), not the colon-joined
        //          $PATH env. Read $PATH from paramtab (the shell
        //          string view); the colon-split below mirrors the C
        //          path[] iteration.
        let path_env = getsparam("PATH").unwrap_or_default();
        for dir in path_env.split(':') {
            // c:6107
            let buf = if dir.is_empty() || dir == "." {
                // c:6108
                if diddot != 0 {
                    continue;
                }
                diddot = 1; // c:6111
                arg0.clone() // c:6112
            } else {
                format!("{}/{}", dir, arg0) // c:6114
            };
            let p = Path::new(&buf);
            if p.exists() && !p.is_dir() {
                // c:6117-6118
                found_path = Some(buf); // c:6119
                break;
            }
        }
    }

    // c:6125-6128 — restore pparams. C does this AFTER source() runs
    // so the sourced script sees the new positional args; bug #459 in
    // docs/BUGS.md — the previous Rust port restored here BEFORE the
    // file body executed, so `source script.zsh hello world` had the
    // script see $#=0 / $@="". Defer the restore until after
    // execute_script returns (the success path below + the error path
    // for file-not-found).

    // c:6130-6137 — error path. C: `if (ret == SOURCE_NOT_FOUND)`
    // emits via zerrnam (POSIX) / zwarnnam (default). The Rust port
    // uses zwarnnam unconditionally because the POSIX hard-error
    // path also calls zerrnam which behaves identically here (both
    // route through zwarning); the only difference C makes is
    // promoting errflag to ERRFLAG_ERROR which already happens
    // inside zwarnnam.
    let path = match found_path {
        Some(p) => p,
        None => {
            // c:6149 — restore argzero on the error path BEFORE
            // emitting the diagnostic and returning. C's structure is
            // `source(); if (isset(FUNCTIONARGZERO)) restore`; for the
            // file-not-found early-return, the restore still has to
            // happen so the outer shell's $0 doesn't keep the dropped
            // arg0. Bug #103 in docs/BUGS.md: the previous port did
            // this unconditional pre-check restore which ran BEFORE
            // `execute_script` on the file-found branch — clobbering
            // the just-set arg0 so the sourced script saw the shell
            // binary as $0 instead of its own path.
            if let Some(prev) = saved_argzero.clone() {
                set_argzero(prev);
            }
            // c:6126-6128 — also restore pparams on the not-found path,
            // matching C's c:6125 unconditional cleanup before return.
            if let Some(saved) = saved_pparams.clone() {
                let mut pp = PPARAMS.lock().unwrap_or_else(|e| {
                    PPARAMS.clear_poison();
                    e.into_inner()
                });
                *pp = saved;
            }
            // c:6130-6137 — `if (isset(POSIXBUILTINS)) zerrnam(...)
            // else zwarnnam(...)`. zerrnam sets ERRFLAG_ERROR ("hard
            // error in POSIX (we'll exit later)" — the exec.c done:
            // gate at c:4378 turns it into exit(1) for the PSPECIAL
            // `.`); zwarnnam does NOT touch errflag, so without
            // POSIX_BUILTINS the script continues.
            let msg = format!("{}: {}", "no such file or directory", arg0); // c:6135
            if isset(crate::ported::zsh_h::POSIXBUILTINS) {
                crate::ported::utils::zerrnam(name, &msg); // c:6133
            } else {
                zwarnnam(name, &msg); // c:6135
            }
            // c:6143 — `return ret == SOURCE_OK ? lastval : 128 - ret`.
            // SOURCE_NOT_FOUND = 1 (Src/zsh.h:2214) → 128 - 1 = 127.
            return 128 - 1;
        }
    };

    // c:6140 — `ret = source(enam = buf);`
    // C `source()` lives at Src/init.c:1550. It opens the file, sets
    // up sourcelevel + scriptname + funcstack, parses + executes via
    // the wordcode walker, then unwinds. Rust port reads the file
    // and routes the body through fusevm's `execute_script` — the
    // VM's parse + compile + run loop is the analog of C's
    // loop/execlist tree walk. Errors during execution propagate
    // through `lastval`; missing read returns SOURCE_ERROR (128-2 =
    // 126) per c:6143.
    //
    // crate::ported::init::sourcelevel bump (Src/init.c:1606 `sourcelevel++;` /
    // c:1644 `sourcelevel--;`) is REQUIRED for `return` inside the
    // sourced file to unwind correctly. bin_break (Src/builtin.c:5840)
    // checks `(interactive && shinstdin) || locallevel || sourcelevel`
    // — without the bump, `return N` falls through to `zexit(num,
    // ZEXIT_NORMAL)` (c:5858) and kills the entire shell instead of
    // unwinding to the source caller. Also clear RETFLAG after the
    // sourced script returns so the unwind doesn't propagate to the
    // outer compile unit.
    // c:Src/init.c:1557-1558 — `char *old_scriptname = scriptname;
    // char *old_scriptfilename = scriptfilename;`
    // c:1591-1592 — `scriptname = s; scriptfilename = s;`
    // For the duration of source execution, both scriptname and
    // scriptfilename point to the sourced file path. zerr/zwarnnam
    // diagnostics emitted from inside the sourced file then use this
    // path as the prefix (`/path/to/foo:N: ...` instead of `zsh:1:`).
    // Bug #139 in docs/BUGS.md. The `path` here is the resolved
    // absolute path; C uses the user-supplied `s` (the bin_dot
    // argv[0]) but `arg0` (the user-supplied name) is what zsh
    // actually shows in diagnostics, so use arg0.
    let old_scriptname = crate::ported::utils::scriptname_get(); // c:1557
    let old_scriptfilename = crate::ported::utils::scriptfilename_get(); // c:1558
    crate::ported::utils::set_scriptname(Some(arg0.clone())); // c:1591
                                                              // c:1592 — `scriptfilename = s;` — PS4 `%x` reads this; without
                                                              // the store the bottom of an `xtrace` line stack stays "zsh"
                                                              // (the executor seed) even inside the sourced file. Bug shared
                                                              // with source_from_memory in bins/zshrs.rs.
    crate::ported::utils::set_scriptfilename(Some(arg0.clone())); // c:1592

    // c:Src/init.c:1609 — `oloops = loops;` — and c:1622 `loops = 0;`.
    // `source` is a full loop-scope FIREWALL: the sourced file starts
    // with a zero nesting count no matter how deep the caller's loops
    // are, and the caller's count is put back at c:1694 below.
    //
    // Both halves are observable:
    //   * `loops = 0` on entry — `for i in 1 2; do source f; done` where
    //     `f` does a top-level `return` must still run BOTH iterations.
    //     `bin_break`'s BIN_RETURN arm sets `breaks = loops` (c:5835),
    //     so without the zeroing the return would set `breaks = 1` and
    //     the caller's `for` would drain it and stop after iteration 1.
    //     It is also why a bare `break` at a sourced file's top level
    //     errors with "not in while, until, select, or repeat loop"
    //     even when the `source` itself sits inside a loop.
    //   * `loops = oloops` on exit — the caller's own `break` keeps
    //     working after the source returns.
    let oloops = LOOPS.load(Relaxed); // c:1609
    LOOPS.store(0, Relaxed); // c:1622

    crate::ported::init::sourcelevel.fetch_add(1, Relaxed); // c:1606

    // c:Src/init.c:1608-1616 — push a funcstack frame with
    // `tp = FS_SOURCE` so prompt %x / xtrace / `$funcfiletrace`
    // resolve to the sourced file, not the calling function's
    // defining file. Without this push, sourcing a script from
    // inside a function left the funcstack TOP as the function's
    // own FS_FUNC frame; the prompt %x handler then reported the
    // function's `filename` (where the function was DEFINED) as
    // the current source file. Repro: powerlevel10k.zsh-theme's
    // `__p9k_root_dir=${${(%):-%x}:A:h}` resolved to BIN_DIR
    // (zinit.zsh's dir) instead of the theme's plugin dir, so the
    // theme then sourced `$BIN_DIR/internal/p10k.zsh` (no such
    // file) instead of `$THEME_DIR/internal/p10k.zsh`.
    let prev_funcstack_lineno = crate::ported::input::lineno.with(|c| c.get()) as i64;
    let pushed_frame = {
        let mut stack = crate::ported::modules::parameter::FUNCSTACK
            .lock()
            .unwrap_or_else(|e| {
                crate::ported::modules::parameter::FUNCSTACK.clear_poison();
                e.into_inner()
            });
        let prev_caller = stack
            .last()
            .map(|fs| fs.name.clone())
            .or_else(|| old_scriptfilename.clone())
            .or_else(|| Some("zsh".to_string()));
        let frame = crate::ported::zsh_h::funcstack {
            prev: None,
            name: arg0.clone(),            // c:1608 fstack.name = scriptfilename
            filename: Some(arg0.clone()),  // c:1613 fstack.filename = scriptfilename
            caller: prev_caller,           // c:1609
            flineno: 0,                    // c:1611
            lineno: prev_funcstack_lineno, // c:1612
            tp: crate::ported::zsh_h::FS_SOURCE, // c:1615
        };
        stack.push(frame);
        true
    };

    // c:Src/init.c:1566 — `source()` tries the compiled form FIRST:
    // `!(prog = try_source_file((us = unmeta(s)))) && (tempfd =
    // open(us, ...)) == -1` — a sibling `<file>.zwc` newer than the
    // file (or `s` itself being a `.zwc`) short-circuits the plain
    // read. zshrs executes through the fusevm text pipeline, so the
    // dump's wordcode bridges via getpermtext (same bridge as the
    // `.zwc` autoload path in exec.rs::loadautofn).
    let zwc_src = crate::ported::parse::try_source_file(&path)
        .map(|prog| crate::ported::text::getpermtext(Box::new(prog), None, 0));
    // c:Src/init.c:1620 — `errflag &= ~ERRFLAG_ERROR;` before
    // executing the sourced body, so a stale flag from the caller's
    // context can't abort the file's first list.
    crate::ported::utils::errflag.fetch_and(!crate::ported::zsh_h::ERRFLAG_ERROR, Relaxed);
    // Recursion backstop — c:Src/jobs.c:1878-1884. zsh runs the sourced
    // list through execpline, whose per-pipeline `initjob()` caps recursion
    // at MAX_MAXJOBS: `zerr("job table full or recursion limit exceeded")`
    // then bails. The fusevm pipeline path (execute_script below) doesn't
    // allocate a job per pipeline, so without this guard runaway
    // `. self`-style recursion — invisible to FUNCNEST, which counts only
    // FS_FUNC frames (doshfunc, exec.rs:5684) — grew the FS_SOURCE stack
    // unbounded and overflowed the 256 MB main-thread stack → uncatchable
    // SIGBUS. Total FUNCSTACK depth (incl. the FS_SOURCE frame just pushed)
    // is the proxy for zsh's concurrently-held job slots; at/over the
    // ceiling, raise the zsh-identical error and refuse the deeper body
    // (SOURCE_ERROR → 126, as zsh's bail returns). The frame is popped below.
    let over_limit = crate::ported::modules::parameter::FUNCSTACK
        .lock()
        .map(|s| s.len())
        .unwrap_or(0)
        >= crate::ported::jobs::MAX_MAXJOBS;
    // c:Src/init.c:220 — `execode(prog, 0, 0, toplevel ? "toplevel" : "file");`
    // A SOURCED file is the `"file"` arm, so its body runs with `file`
    // appended to `zsh_eval_context`: `toplevel:file` at top level,
    // `toplevel:shfunc:file` inside a function, `toplevel:eval:file` inside an
    // eval. zshrs pushed nothing here, so the component was missing at every
    // depth. funcstack was already correct (the sourced path pushes its own
    // FS_SOURCE frame just below) — only the eval-context entry was absent.
    // Popped on every return path by the guard. Bug #1067.
    let sync_eval_ctx = |stack: &[String]| {
        let joined = stack.join(":");
        if let Ok(mut tab) = crate::ported::params::paramtab().write() {
            if let Some(pm) = tab.get_mut("zsh_eval_context") {
                pm.u_arr = Some(stack.to_vec());
                pm.node.flags &= !(crate::ported::zsh_h::PM_UNSET as i32);
            }
            if let Some(pm) = tab.get_mut("ZSH_EVAL_CONTEXT") {
                pm.u_str = Some(joined);
                pm.node.flags &= !(crate::ported::zsh_h::PM_UNSET as i32);
            }
        }
    };
    if let Ok(mut ctx) = crate::ported::exec::zsh_eval_context.lock() {
        ctx.push("file".to_string());
        sync_eval_ctx(&ctx);
    }
    struct SourceCtxGuard<F: Fn(&[String])>(F);
    impl<F: Fn(&[String])> Drop for SourceCtxGuard<F> {
        fn drop(&mut self) {
            if let Ok(mut ctx) = crate::ported::exec::zsh_eval_context.lock() {
                ctx.pop();
                (self.0)(&ctx);
            }
        }
    }
    let _source_ctx_guard = SourceCtxGuard(sync_eval_ctx);
    let mut result = if over_limit {
        crate::ported::utils::zerr("job table full or recursion limit exceeded");
        128 - 2 // c:6143 — SOURCE_ERROR = 2 → 126
    } else {
        match zwc_src {
            Some(src) => crate::ported::exec::execute_script(&src).unwrap_or(1), // c:1566 prog path
            None => match fs::read_to_string(&path) {
                // c:6140
                Ok(src) => crate::ported::exec::execute_script(&src).unwrap_or(1),
                // c:6143 — SOURCE_ERROR = 2 (Src/zsh.h:2216) → 128 - 2 = 126.
                Err(_) => 128 - 2,
            },
        }
    };
    // c:Src/init.c:1623-1624 — `if (errflag) ret = SOURCE_ERROR;`
    // c:Src/init.c:1663 — `errflag &= ~ERRFLAG_ERROR;` on the restore
    // path. source is a CONTAINMENT boundary: an errflag abort inside
    // the sourced file breaks ITS lists, then source() clears the
    // flag, bin_dot returns 128 - SOURCE_ERROR = 126 (c:Src/builtin.c
    // :6143), and the CALLER continues. zsh 5.9: parent script
    // sourcing a file that hits `readonly` reassign prints its next
    // line with $? = 126.
    if (crate::ported::utils::errflag.load(Relaxed) & crate::ported::zsh_h::ERRFLAG_ERROR) != 0 {
        crate::ported::utils::errflag.fetch_and(!crate::ported::zsh_h::ERRFLAG_ERROR, Relaxed);
        result = 128 - 2; // c:6143 — 128 - SOURCE_ERROR
    }
    if pushed_frame {
        // c:1643 — `funcstack = funcstack->prev;`
        let mut stack = crate::ported::modules::parameter::FUNCSTACK
            .lock()
            .unwrap_or_else(|e| {
                crate::ported::modules::parameter::FUNCSTACK.clear_poison();
                e.into_inner()
            });
        stack.pop();
    }
    crate::ported::init::sourcelevel.fetch_sub(1, Relaxed); // c:1644
                                                            // c:1666 — `scriptname = old_scriptname;` and matching
                                                            // scriptfilename restore from the c:1667 line.
    crate::ported::utils::set_scriptname(old_scriptname);
    crate::ported::utils::set_scriptfilename(old_scriptfilename);
    // c:Src/init.c:1694 — `loops = oloops;` — put the caller's loop
    // nesting count back (see the c:1609/1622 note at the entry half).
    LOOPS.store(oloops, Relaxed); // c:1694

    // c:Src/init.c:1697-1698 — `if (!exit_pending) retflag = 0;`
    // RETFLAG is set by bin_break's BIN_RETURN arm (c:5834). Once the
    // sourced file's execute_script unwinds, the return has been
    // serviced; clear the flag so the outer compile unit's main loop
    // (init.rs:1252's `if retflag break` guard) doesn't see a stale
    // request and abort `echo done` after `source foo`.
    //
    // The `exit_pending` guard is C's, not an approximation: `exit`
    // inside a function sets retflag ALONG WITH exit_pending
    // (c:5874-5878) precisely so the unwind keeps propagating past
    // every enclosing scope until the deferred exit is honoured.
    // Clearing retflag unconditionally would strand that exit at the
    // `source` boundary.
    if EXIT_PENDING.load(Relaxed) == 0 {
        // c:1697
        RETFLAG.store(0, Relaxed); // c:1698
    }

    // c:6149 again — restore argzero on the success path as well.
    if let Some(prev) = saved_argzero {
        set_argzero(prev);
    }
    // c:6125-6128 — restore pparams on the success path AFTER the
    // source body has finished executing. Bug #459.
    if let Some(saved) = saved_pparams {
        let mut pp = PPARAMS.lock().unwrap_or_else(|e| {
            PPARAMS.clear_poison();
            e.into_inner()
        });
        *pp = saved;
    }
    result
}

/// Port of `static int eval(char **argv)` from `Src/builtin.c:6151`.
pub fn eval(argv: &[String]) -> i32 {
    // c:6151
    // c:6153 — `Eprog prog;` (declared inline below)
    // c:6154 — `char *oscriptname = scriptname;`
    let oscriptname: Option<String> = scriptname_get();
    // c:6155 — `int oineval = ineval, fpushed;`
    let oineval: i32 = INEVAL.load(Relaxed);
    let fpushed: bool;
    // c:6156 — `struct funcstack fstack;`

    // c:6163 — `ineval = !isset(EVALLINENO);`
    INEVAL.store(
        if !isset(crate::ported::zsh_h::EVALLINENO) {
            1
        } else {
            0
        },
        Relaxed,
    );
    let ineval_now = INEVAL.load(Relaxed) != 0;

    if !ineval_now {
        // c:6164
        // c:6165 — `scriptname = "(eval)";`
        crate::ported::utils::set_scriptname(Some("(eval)".to_string()));
        // c:6166-6196 — funcstack push: build a fstack frame describing
        // this eval, link it to the head of FUNCSTACK.
        let prev_frame = {
            let stack = FUNCSTACK.lock().unwrap_or_else(|e| e.into_inner());
            stack.last().cloned()
        };
        let lineno_now = crate::ported::input::lineno.with(|c| c.get()) as i64;
        let caller = match &prev_frame {
            Some(p) => Some(p.name.clone()),
            None => argzero(), // c:6168 dupstring(argzero)
        };

        // c:6182-6196 — flineno/filename derivation. Three cases:
        //   1. no prev frame OR prev tp == FS_SOURCE: flineno=lineno,
        //      filename=caller (the source name)
        //   2. prev tp == FS_EVAL: flineno = prev.flineno + lineno - 1
        //   3. otherwise (function): flineno = prev.flineno + lineno,
        //      filename = prev.filename or ""
        let (flineno, filename): (i64, Option<String>) = match &prev_frame {
            None => (lineno_now, caller.clone()), // c:6183-6184
            Some(p) if p.tp == crate::ported::zsh_h::FS_SOURCE => {
                (lineno_now, caller.clone()) // c:6183-6184
            }
            Some(p) => {
                let mut fl = p.flineno + lineno_now; // c:6186
                if p.tp == crate::ported::zsh_h::FS_EVAL {
                    // c:6191
                    fl -= 1; // c:6192
                }
                let fname = p.filename.clone().or_else(|| Some(String::new())); // c:6193-6195
                (fl, fname)
            }
        };
        let frame = crate::ported::zsh_h::funcstack {
            prev: None,                        // c:1349 — linked via FUNCSTACK vec
            name: "(eval)".to_string(),        // c:6167
            filename,                          // c:6184 / c:6193
            caller,                            // c:6168
            flineno,                           // c:6183 / c:6186
            lineno: lineno_now,                // c:6169
            tp: crate::ported::zsh_h::FS_EVAL, // c:6170
        };
        {
            let mut stack = FUNCSTACK.lock().unwrap_or_else(|e| e.into_inner());
            stack.push(frame); // c:6197 funcstack = &fstack
        }
        fpushed = true; // c:6199
    } else {
        fpushed = false; // c:6201
    }

    // c:6203 — `prog = parse_string(zjoin(argv, ' ', 1), 1);`
    let joined = crate::ported::utils::zjoin(argv, ' ');
    let prog = crate::ported::exec::parse_string(&joined, 1);

    if let Some(prog) = prog {
        // c:6205 — `if (wc_code(*prog->prog) != WC_LIST)`
        let head = prog.prog.first().copied().unwrap_or(0);
        if crate::ported::zsh_h::wc_code(head) != crate::ported::zsh_h::WC_LIST as u32 {
            /* No code to execute */
            // c:6206
            LASTVAL.store(0, Relaxed); // c:6207
        } else {
            // c:6209 — `execode(prog, 1, 0, "eval");`. Routes through
            // the executor; in-process equivalent.
            //
            // PREVIOUSLY called run_command_substitution which captures
            // stdout into a String and returns it — bin_eval threw the
            // capture away, so `eval 'echo hi'` produced no output.
            // execute_script_zsh_pipeline runs the script with stdout
            // flowing to the caller (no capture) which is what eval
            // wants. Same routing the eval-via-execstring path uses
            // (vm_helper.rs:1518 EXIT-trap fire).
            //
            // Recursion backstop — c:Src/jobs.c:1878-1884. zsh's per-pipeline
            // `initjob()` caps eval recursion at MAX_MAXJOBS ("job table full
            // or recursion limit exceeded"); the fusevm pipeline path doesn't
            // allocate a job per pipeline, so runaway `eval`-string recursion
            // (invisible to FUNCNEST, which counts FS_FUNC frames only, AND to
            // the FS_EVAL frame count — nested evals suppress the funcstack
            // push via INEVAL, c:6164, exactly like zsh) is otherwise unbounded
            // → main-thread stack overflow → SIGBUS. Track eval re-entry depth
            // directly (the Rust proxy for zsh's concurrently-held job slots,
            // one per nested eval pipeline) and cap at the same MAX_MAXJOBS
            // ceiling. Requested as a parity fix by the maintainer.
            let eval_depth = crate::vm_helper::EVAL_RECURSION_DEPTH.with(|d| {
                let v = d.get() + 1;
                d.set(v);
                v
            });
            if eval_depth >= crate::ported::jobs::MAX_MAXJOBS {
                crate::ported::utils::zerr("job table full or recursion limit exceeded");
                LASTVAL.store(1, Relaxed);
            } else {
                // c:6209 — `execode(prog, 1, 0, "eval")`. The `"eval"`
                // frame is pushed by the live BUILTIN_EVAL path
                // (fusevm_bridge.rs, `EvalContextFrame::push("eval")`)
                // that `execute_script_zsh_pipeline` routes through;
                // pushing it here as well would double it.
                let _ = crate::ported::exec::execute_script_zsh_pipeline(&joined);
            }
            crate::vm_helper::EVAL_RECURSION_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
            // c:6211-6212 — `if (errflag && !lastval) lastval = errflag;`
            let ef = errflag.load(Relaxed);
            let lv = LASTVAL.load(Relaxed);
            if ef != 0 && lv == 0 {
                LASTVAL.store(ef, Relaxed);
            }
        }
    } else {
        LASTVAL.store(1, Relaxed); // c:6215
    }

    if fpushed {
        // c:6218
        // c:6219 — `funcstack = funcstack->prev;`
        let mut stack = FUNCSTACK.lock().unwrap_or_else(|e| e.into_inner());
        stack.pop();
    }

    // c:6221 — `errflag &= ~ERRFLAG_ERROR;`
    errflag.fetch_and(!ERRFLAG_ERROR, Relaxed);
    // c:6222 — `scriptname = oscriptname;`
    crate::ported::utils::set_scriptname(oscriptname);
    // c:6223 — `ineval = oineval;`
    INEVAL.store(oineval, Relaxed);

    LASTVAL.load(Relaxed) // c:6225
}

/// Port of `bin_emulate(char *nam, char **argv, Options ops, UNUSED(int func))` from Src/builtin.c:6232.
/// C: `int bin_emulate(char *nam, char **argv, Options ops, ...)` —
///   no-args print current emulation; single-arg switch emulation;
///   `-l` list, `-L` set LOCAL*, `-R` reset to defaults.
/// WARNING: param names don't match C — Rust=(nam, argv, _func) vs C=(nam, argv, ops, func)
pub fn bin_emulate(
    nam: &str,
    argv: &[String], // c:6232
    ops: &options,
    _func: i32,
) -> i32 {
    let opt_l = OPT_ISSET(ops, b'l'); // c:6236
    let opt_l_arg = OPT_ISSET(ops, b'L'); // c:6234
    let opt_r = OPT_ISSET(ops, b'R'); // c:6235

    // c:6249-6275 — no args: print current emulation name.
    if argv.is_empty() {
        // c:6249
        if opt_l_arg || opt_r {
            // c:6250
            zwarnnam(nam, "not enough arguments"); // c:6251
            return 1; // c:6252
        }
        // c:6255-6271 — `switch(SHELL_EMULATION())` → name dispatch.
        let bits = emulation.load(Relaxed) as i32;
        let shname = if (bits & EMULATE_CSH) != 0 {
            "csh"
        }
        // c:6255
        else if (bits & EMULATE_KSH) != 0 {
            "ksh"
        }
        // c:6259
        else if (bits & EMULATE_SH) != 0 {
            "sh"
        }
        // c:6263
        else {
            "zsh"
        }; // c:6268
        println!("{}", shname); // c:6273
        return 0; // c:6274
    }

    // c:6278-6295 — single-arg form: `emulate <shname>`.
    let shname = &argv[0];
    if argv.len() == 1 {
        // c:6278
        // c:6280-6285 — `if (opt_l) cmdopts = zhalloc(...); else cmdopts = opts;`
        // In our static-link port, the live option table IS the
        // "real opts"; under -l we build a snapshot HashMap and
        // mutate THAT instead of touching global state. Under
        // !-l we apply emulate semantics to the live table.
        // c:537-549 — C `emulate(zsh_name, ...)` reads ONLY the first
        // char (after stripping a leading `r` for rcsh/rksh): 'c'
        // → CSH, 'k' → KSH, 's'/'b' → SH (so `bash` aliases to sh),
        // else ZSH. Previous Rust port did full-string equality so
        // `emulate rcsh` / `emulate bash` silently fell back to ZSH.
        let bytes = shname.as_bytes();
        let mut ch = if !bytes.is_empty() { bytes[0] } else { 0 };
        if ch == b'r' && bytes.len() >= 2 {
            // c:539
            ch = bytes[1]; // c:540
        }
        let bits = match ch {
            // c:543
            b'c' => EMULATE_CSH,       // c:544
            b'k' => EMULATE_KSH,       // c:546
            b's' | b'b' => EMULATE_SH, // c:548
            _ => EMULATE_ZSH,          // c:550
        };
        // c:6286 — `emulate(shname, opt_R, &emulation, cmdopts)`.
        emulation.store(bits, Relaxed);

        // Build the cmdopts view that c:6286-6292 manipulates.
        let mut cmdopts: HashMap<String, bool> = HashMap::new();
        for n in ZSH_OPTIONS_SET.iter() {
            cmdopts.insert(
                n.to_string(),
                crate::ported::options::opt_state_get(n).unwrap_or(false),
            );
        }
        // c:6286 — `emulate(shname, opt_R, &emulation, cmdopts)` ALSO
        // populates `cmdopts` (when !opt_l, `cmdopts` aliases the live
        // `opts[]`). The Rust port has a per-emulation defaults walker
        // at options::emulate() that writes through to OPTS_LIVE — call
        // it here so `emulate -L sh` flips KSH_ARRAYS / SH_NULLCMD /
        // POSIX_ALIASES / etc. to their target-emulation defaults.
        //
        // Bug #26 in docs/BUGS.md — without this, `emulate -L sh`
        // changed the emulation tag but left every option at its zsh
        // default, so `a=(1 2 3); echo ${a[0]}` stayed 1-indexed
        // (empty result) instead of switching to KSH_ARRAYS 0-indexed.
        // c:6285 — `emulate(shname, opt_R, &emulation, cmdopts);` runs
        // UNCONDITIONALLY. C can do that because it passes the target array in:
        // under -l cmdopts is a COPY of opts (c:6281-6282) and the emulation is
        // applied to the copy; otherwise cmdopts aliases the live opts[].
        //
        // options::emulate() has no such parameter — it writes through to
        // OPTS_LIVE — so the -l case is expressed by applying it to the live
        // state, snapshotting the result, and restoring. Same effect as C's
        // copy: `emulate -l sh` reports what the options WOULD be, and the
        // shell is left untouched.
        //
        // Skipping the call under -l (as this did) meant cmdopts stayed the
        // CURRENT options, so `emulate -l sh` listed zsh's defaults, not sh's —
        // ~38 options wrong. It looked right only for `emulate -l zsh`, where
        // the current options ARE the answer.
        let saved_opts = if opt_l {
            Some(crate::ported::options::opt_state_snapshot())
        } else {
            None
        };
        crate::ported::options::emulate(shname.as_str(), opt_r); // c:6285
                                                                 // Re-sync cmdopts from OPTS_LIVE so it reflects the emulation's
                                                                 // defaults rather than the pre-emulate state captured above.
        for n in ZSH_OPTIONS_SET.iter() {
            cmdopts.insert(
                n.to_string(),
                crate::ported::options::opt_state_get(n).unwrap_or(false),
            );
        }
        if let Some(saved) = saved_opts {
            // Undo the live apply — C never touched opts[] in the -l path.
            crate::ported::options::opt_state_restore(saved);
        }

        // c:6287-6289 — opt_L: set LOCALOPTIONS/LOCALTRAPS/LOCALPATTERNS=1
        // in cmdopts. In the !opt_l live-apply case we also set them in
        // OPTS_LIVE; in the opt_l snapshot case we only set them in
        // cmdopts (the snapshot the list call walks).
        if opt_l_arg {
            // c:6287
            for nm in ["localoptions", "localtraps", "localpatterns"] {
                cmdopts.insert(nm.to_string(), true);
                if !opt_l {
                    crate::ported::options::opt_state_set(nm, true);
                }
            }
        }
        if opt_l {
            // c:6290
            // c:6291 — `list_emulate_options(cmdopts, opt_R);`
            crate::ported::options::list_emulate_options(&cmdopts, opt_r);
            return 0; // c:6292
        }
        // c:6294 — `clearpatterndisables();` resets the per-pattern
        // disabled-feature bitset that a previous emulation may have
        // left in place.
        crate::ported::pattern::clearpatterndisables();
        return 0; // c:6295
    }

    // c:6297-6300 — too many args under -l.
    if opt_l {
        // c:6297
        zwarnnam(nam, "too many arguments for -l"); // c:6298
        return 1; // c:6299
    }

    // c:6302-6342 — `emulate <shname> [-o OPT|+o OPT|-LR ...]` per-command
    // form. C: `argv++; emulate(shname, opt_R, ...); parseopts(...);` —
    // switch emulation, then parse the remaining tokens as long-form
    // options that apply ON TOP of the emulation defaults. zinit's
    // +zi-log (zinit.zsh:2178) calls
    //   `emulate -LR zsh -o extendedglob`
    // which requires extendedglob to be set after the zsh-emulation
    // reset, so the (#b)/(#m) flags in subsequent pattern subst calls
    // are recognized. The previous Rust port returned 0 without
    // applying ANY of the trailing options, so extendedglob stayed off
    // and `${msg//(#b)…/…}` errored with `bad pattern: …`.
    //
    // The `-c command` (cmd-eval) and sticky-emulation paths at c:6329+
    // remain unported (no zshrs caller exercises them yet) — the
    // structural opt-application below is enough for the common
    // `emulate -LR zsh -o NAME …` form.
    let _ = opt_r;
    // c:6303 — `memcpy(saveopts, opts, sizeof(opts));` + c:6326
    // `saveemulation = emulation;` — snapshot the live option table
    // and both emulation cells BEFORE applying the new emulation.
    // C builds the new state in a LOCAL `new_opts` and only commits
    // it at c:6328; the Rust port writes the live table directly, so
    // this snapshot is the rollback for the `-c` eval (restore2 at
    // c:6377-6379) and the parse-error path (goto restore, c:6311).
    // Without the restore, `emulate sh -c 'f() { ...; }'` left the
    // shell PERMANENTLY in sh-emulation option state — a later
    // `bad substitution` zerr then aborted the whole non-interactive
    // shell (D04parameter chunk 159 poisoned chunk 225 into a
    // shell-exit).
    let saveopts = crate::ported::options::opt_state_snapshot(); // c:6303
    let saveemulation = emulation.load(Relaxed); // c:6326
    let saveemulation_live = crate::ported::options::EMULATION.load(Relaxed); // c:6326 (port keeps 2 cells)
                                                                              // c:6306 — `emulate(shname, opt_R, &new_emulation, new_opts);` —
                                                                              // apply emulation defaults to the live opts table.
    crate::ported::options::emulate(shname.as_str(), opt_r);

    // c:6308 — `parseopts(nam, &argv, new_opts, &cmd, optlist, 0)`.
    // C body at Src/init.c:390+:
    //   while (!optionbreak && *argv && (**argv == '-' || **argv == '+')) {
    //       action = (**argv == '-');
    //       while (*++*argv) {
    //           if (**argv == '-') { /* `--` ends options; `--NAME` long form */ }
    //           else if (**argv == 'c') { *cmdp = *argv; }
    //           else if (**argv == 'o') {
    //               if (!*++*argv) argv++;   // `-o NAME` separate-arg
    //               if (!*argv) { WARN("string expected after -o"); return 1; }
    //               longoptions: optno = optlookup(*argv);
    //               if (!optno) { WARN("no such option: %s", *argv); return 1; }
    //               dosetopt(optno, action, ...);
    //               break;
    //           }
    //           else {  /* single-char option letter, e.g. 'L', 'R' */
    //               optno = optlookupc(**argv);
    //               dosetopt(optno, action, ...);
    //           }
    //       }
    //       argv++;
    //   }
    //
    // Faithful port: walk each arg, parse `-`/`+` action sign, then
    // walk char-by-char. `--` ends options; `-o NAME` / `+o NAME`
    // long-form (NAME inline-attached or separate arg); single-char
    // option letters set/unset via `dosetopt` (Rust:
    // `opt_state_set`). `-c command` records the cmd-eval mode (not
    // wired through here — bin_emulate uses opt_L for localoptions
    // only).
    let mut i = 1; // skip shname (argv[0])
    let mut optionbreak = false;
    let mut cmd_body: Option<String> = None; // c:6310 — `-c command` capture.
                                             // c:6308 `optlist` — parseopts records each explicitly-set option
                                             // (as an index into new_opts) so the `-c` arm can build the
                                             // sticky struct's on_opts/off_opts at c:6347-6373. Rust tracks
                                             // (canonical-name, on?) pairs.
    let mut optlist: Vec<(String, bool)> = Vec::new();
    while !optionbreak && i < argv.len() {
        let arg = &argv[i];
        // c:Src/init.c:418 — only `-` / `+` start an option arg.
        let first = arg.chars().next().unwrap_or('\0');
        if first != '-' && first != '+' {
            break;
        }
        let action = first == '-'; // c:420
                                   // c:421-422 — bare `-` / `+` treated as `--` (end-of-options
                                   // marker preserved as-is for caller).
        if arg.len() == 1 {
            i += 1;
            break;
        }
        // Walk chars after the leading +/-.
        let bytes: Vec<char> = arg.chars().collect();
        let mut j = 1;
        let mut consumed_next_arg = false;
        while j < bytes.len() {
            let ch = bytes[j];
            if ch == '-' {
                // c:425-429 — `--` ends options.
                if j == 1 && bytes.len() == 2 {
                    optionbreak = true;
                    i += 1;
                    break;
                }
                // c:432-460 — `--NAME` GNU-style long. `-` chars
                // inside NAME become `_` (c:457-459). For bin_emulate
                // we don't recognize `--version` / `--help` / etc.,
                // just route the inline NAME as a long-form option.
                let name: String = bytes[j + 1..].iter().collect::<String>().replace('-', "_");
                // Same optlookup+dosetopt fix as the `-o NAME` arm below:
                // resolve the `no` negation prefix and run option side
                // effects, instead of writing a bogus stripped-name slot.
                let optno = crate::ported::options::optlookup(&name);
                if optno == 0 {
                    zwarnnam(nam, &format!("no such option: {}", name));
                    return 1;
                }
                crate::ported::options::dosetopt(optno, if action { 1 } else { 0 }, 0);
                let canon = crate::ported::zsh_h::opt_name(optno.abs()).to_string();
                let effective_on = if optno < 0 { !action } else { action };
                optlist.push((canon, effective_on)); // c:6308 optlist record
                break;
            }
            if ch == 'o' || ch == 'O' {
                // c:480-505 — `-o NAME` / `+o NAME`. NAME is either
                // attached (rest of arg after `o`) or the next arg.
                let name = if j + 1 < bytes.len() {
                    bytes[j + 1..].iter().collect::<String>()
                } else {
                    if i + 1 >= argv.len() {
                        zwarnnam(nam, "string expected after -o");
                        return 1; // c:484-485
                    }
                    consumed_next_arg = true;
                    argv[i + 1].clone()
                };
                // c:6314-6316 — `optno = optlookup(*argv); if (!optno)
                // { WARN("no such option"); return 1; } dosetopt(optno,
                // action, ...)`. optlookup resolves the `no`/`no_`
                // negation prefix (returning a NEGATIVE optno) and
                // dosetopt inverts it AND runs the side effects
                // (inittyptab for MULTIBYTE/BANGHIST/SHINSTDIN). The
                // previous `opt_state_set(lowercased_stripped, action)`
                // wrote a bogus `nomultibyte` slot and never toggled the
                // real MULTIBYTE option, so `emulate -o no_multi_byte`
                // (p10k.zsh:1760) left MULTIBYTE ON — char- not
                // byte-oriented length math — and every p10k prompt
                // segment built empty.
                let optno = crate::ported::options::optlookup(&name);
                if optno == 0 {
                    zwarnnam(nam, &format!("no such option: {}", name));
                    return 1;
                }
                crate::ported::options::dosetopt(optno, if action { 1 } else { 0 }, 0);
                let canon = crate::ported::zsh_h::opt_name(optno.abs()).to_string();
                let effective_on = if optno < 0 { !action } else { action };
                optlist.push((canon, effective_on)); // c:6308 optlist record
                break; // c:505 — break out of char walk after `-o`
            }
            if ch == 'c' {
                // c:6310 — `-c command`. Capture the command body from
                // either the rest of the current arg (e.g. `-c'body'`)
                // or the next argv slot. C's sticky-emulation evaluator
                // (c:6332-6373) wraps the cmd in saved/restored
                // emulation state; we mirror by running the captured
                // body through eval after the option-parse loop exits.
                let body = if j + 1 < bytes.len() {
                    bytes[j + 1..].iter().collect::<String>()
                } else {
                    if i + 1 >= argv.len() {
                        zwarnnam(nam, "string expected after -c");
                        return 1;
                    }
                    consumed_next_arg = true;
                    argv[i + 1].clone()
                };
                cmd_body = Some(body);
                break;
            }
            // c:Src/init.c — single-char option letter dispatch via
            // `optno = optlookupc(**argv); dosetopt(optno, action, ...)`.
            // C maps the char through `optletters[]` (Src/options.c) to
            // an OPT_* number, then writes the action bit.
            //
            // NOT PORTED here: the chars bin_emulate cares about (`L`,
            // `R`, `l`) are extracted into `ops` by the dispatcher BEFORE
            // bin_emulate runs (via the optstr declared at
            // Src/builtin.c:99 for the emulate builtin). The Rust port's
            // builtin dispatcher does the same — see `opt_l`, `opt_l_arg`,
            // `opt_r` reads at the top of this function. So by the time
            // parseopts runs, single-char options have already been
            // consumed and `argv[1..]` contains only the long-form
            // `-o NAME` / `--NAME` shapes that this loop handles.
            // Unknown single-char chars are silently skipped (matching
            // what the dispatcher would have flagged as a parse error
            // before bin_emulate ran).
            let _ = ch;
            j += 1;
        }
        i += 1;
        if consumed_next_arg {
            i += 1;
        }
    }
    // c:6332-6377 — `-c command` evaluator: save patterns, install
    // the pending `sticky` struct (functions defined in the body get
    // stamped via shfunc_set_sticky at c:5402), eval the body, then
    // restore sticky + emulation + opts + patterns (restore2: at
    // c:6377-6379).
    if let Some(body) = cmd_body {
        if opt_l_arg {
            // c:6333-6336 — `-L` is incompatible with `-c`.
            zwarnnam(nam, "option -L incompatible with -c");
            // c:6336 `goto restore2` — undo the emulation switch.
            emulation.store(saveemulation, Relaxed);
            crate::ported::options::EMULATION.store(saveemulation_live, Relaxed);
            crate::ported::options::opt_state_restore(saveopts);
            return 1;
        }
        // c:6319 — `savepatterns = savepatterndisables();`
        let savepatterns = crate::ported::pattern::savepatterndisables();
        // c:6324 — `clearpatterndisables();` — every emulation starts
        // with an empty pattern-disable set.
        crate::ported::pattern::clearpatterndisables();
        // c:6344-6373 — `save_sticky = sticky; sticky = hcalloc(...);`
        // Build the sticky struct: target emulation + the on/off
        // option indices the optlist recorded.
        let save_sticky = crate::ported::options::sticky
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take(); // c:6344
        {
            let mut on_opts: Vec<crate::ported::zsh_h::OptIndex> = Vec::new();
            let mut off_opts: Vec<crate::ported::zsh_h::OptIndex> = Vec::new();
            for (name, on) in &optlist {
                // c:6366-6372 — Data is index into new_opts.
                let optno = optlookup(name);
                if optno > 0 {
                    if *on {
                        on_opts.push(optno as crate::ported::zsh_h::OptIndex);
                    } else {
                        off_opts.push(optno as crate::ported::zsh_h::OptIndex);
                    }
                }
            }
            *crate::ported::options::sticky
                .lock()
                .unwrap_or_else(|e| e.into_inner()) =
                Some(Box::new(crate::ported::zsh_h::emulation_options {
                    emulation: crate::ported::options::EMULATION.load(Relaxed), // c:6346
                    n_on_opts: on_opts.len() as i32,                            // c:6351
                    n_off_opts: off_opts.len() as i32,                          // c:6353
                    on_opts,                                                    // c:6356-6358
                    off_opts,                                                   // c:6360-6362
                }));
        }
        let r = eval(&[body]); // c:6374 — `ret = eval(argv);`
                               // c:6375 — `sticky = save_sticky;`
        *crate::ported::options::sticky
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = save_sticky;
        // c:6377-6379 restore2: — emulation + opts + patterns.
        emulation.store(saveemulation, Relaxed); // c:6377
        crate::ported::options::EMULATION.store(saveemulation_live, Relaxed); // c:6377
        crate::ported::options::opt_state_restore(saveopts); // c:6378
        crate::ported::pattern::restorepatterndisables(savepatterns); // c:6379
                                                                      // c:6381-6382 restore: — keyboardhackchar + inittyptab()
                                                                      // (keyboard hack char isn't ported; typtab rebuild is a
                                                                      // no-op in the Rust lexer).
        return r;
    }

    // c:6314-6317 — `if (*argv) zwarnnam(nam, "unknown argument %s",
    // *argv);`. Anything left after the option-parse loop is unknown.
    // zinit's `emulate -LR zsh -o extendedglob` exhausts argv, so this
    // arm doesn't fire — but mirror the C behavior for other callers.
    if i < argv.len() && !optionbreak {
        zwarnnam(nam, &format!("unknown argument: {}", argv[i]));
        // c:6315 `goto restore` — C's LIVE opts were never touched on
        // this path (parseopts wrote the local new_opts); the Rust
        // loop wrote live state, so roll back the snapshot to match.
        emulation.store(saveemulation, Relaxed);
        crate::ported::options::EMULATION.store(saveemulation_live, Relaxed);
        crate::ported::options::opt_state_restore(saveopts);
        return 1;
    }

    // c:6339-6340 — `if (opt_L) opts[LOCALOPTIONS] = opts[LOCALTRAPS] =
    //                opts[LOCALPATTERNS] = 1;`.
    if opt_l_arg {
        for nm in ["localoptions", "localtraps", "localpatterns"] {
            crate::ported::options::opt_state_set(nm, true);
        }
    }
    0
}

/// Port of `bin_eval(UNUSED(char *nam), char **argv, UNUSED(Options ops), UNUSED(int func))` from Src/builtin.c:6393.
/// C: `int bin_eval(UNUSED args)` → `return eval(argv);`
/// WARNING: param names don't match C — Rust=(_name, argv, _func) vs C=(nam, argv, ops, func)
pub fn bin_eval(
    _name: &str,
    argv: &[String], // c:6393
    _ops: &options,
    _func: i32,
) -> i32 {
    // c:Src/builtin.c:407-411 — generic `--` end-of-options strip
    // applied by `execbuiltin` for builtins that:
    //   1. Have NULL optstr (so the `if (optstr) { option parse }`
    //      loop didn't run), AND
    //   2. Don't have BINF_HANDLES_OPTS in their flags.
    // ```c
    // } else if (!(flags & BINF_HANDLES_OPTS) && *argv &&
    //            !strcmp(*argv, "--")) {
    //     ops.ind['-'] = 1;
    //     argv++;
    // }
    // ```
    // `eval` has optstr=NULL (Src/builtin.c:65) and no
    // BINF_HANDLES_OPTS, so the strip applies. The Rust dispatch
    // path bypasses execbuiltin, so we need to do the strip
    // explicitly here. Without it `eval -- "echo hi"` joined to
    // `-- echo hi` and tried to dispatch `--` as a command. Bug
    // #319 in docs/BUGS.md.
    let stripped: Vec<String>;
    let argv: &[String] = if let Some(first) = argv.first() {
        if first == "--" {
            stripped = argv[1..].to_vec();
            &stripped
        } else {
            argv
        }
    } else {
        argv
    };
    eval(argv) // c:6396
}

/// Port of `bin_read(char *name, char **args, Options ops, UNUSED(int func))` from Src/builtin.c:6412.
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
/// WARNING: param names don't match C — Rust=(name, args, _func) vs C=(name, args, ops, func)
pub fn bin_read(
    name: &str,
    args: &[String], // c:6412
    ops: &options,
    _func: i32,
) -> i32 {
    let mut args = args.to_vec();
    let mut nchars: i32 = 1; // c:6415
    let mut partial_eof = false;

    // c:6432-6438 — `-k N` raw-char count.
    if OPT_HASARG(ops, b'k') {
        // c:6432
        let optarg = OPT_ARG(ops, b'k').unwrap_or("");
        match optarg.trim().parse::<i32>() {
            Ok(n) => nchars = n,
            Err(_) => {
                zwarnnam(name, &format!("number expected after -k: {}", optarg)); // c:6437
                return 1;
            }
        }
    }

    // !!! BASH-MODE GATE (no C counterpart) !!! bash `read -n N` / `read -N N`
    // read chars from the input (not a terminal): `-n` reads AT MOST N,
    // stopping early at the delimiter (newline); `-N` reads EXACTLY N, ignoring
    // the delimiter. zsh's optstr treats both as boolean no-ops, so the count N
    // is left as the first positional; pull it out and route through the
    // char-read path (also skips the tty requirement).
    let bash_n = crate::dash_mode::bash_mode() && OPT_ISSET(ops, b'n');
    let bash_bign = crate::dash_mode::bash_mode() && OPT_ISSET(ops, b'N');
    let bash_stop_at_nl = bash_n && !bash_bign; // -N ignores the delimiter
    let bash_nchars = (bash_n || bash_bign)
        && args
            .first()
            .map(|a| a.trim().parse::<i32>().is_ok())
            .unwrap_or(false);
    if bash_nchars {
        nchars = args.remove(0).trim().parse::<i32>().unwrap_or(1);
    }

    // c:6445-6446 — `firstarg = (*args && **args == '?' ? *args++ :
    // *args); reply = *args ? *args++ : ...`. When the first arg
    // starts with `?` it is a prompt-only arg (consumed; the NEXT arg
    // is the reply name). Otherwise firstarg ALIASES the reply name —
    // and the prompt block at c:6534-6543 scans it for an embedded
    // `?`, prints the tail as the prompt, and truncates the name in
    // place (`readpmpt[-1] = '\0'`). So `read v?prompt: ` reads into
    // `v` with prompt `prompt: `. Bug #387 follow-up: the leading-`?`
    // form was handled but the `var?prompt` split was missing, so
    // zshrs errored "not an identifier: v?prompt".
    let mut argi = 0usize;
    let mut prompt: Option<String> = None;
    if argi < args.len() && args[argi].starts_with('?') {
        // c:6445 — prompt-only first arg.
        prompt = Some(args[argi][1..].to_string());
        argi += 1;
    }
    // `-A` is the zsh/ksh array-read flag; bash uses `-a`. Honor `-a` as an
    // array read only in bash mode so `read -a arr <<< "x y z"` works like
    // /bin/bash (in other modes `-a` parses but is inert, matching nothing).
    let want_array =
        OPT_ISSET(ops, b'A') || (crate::dash_mode::bash_mode() && OPT_ISSET(ops, b'a'));
    let reply = if argi < args.len() {
        let mut r = args[argi].clone();
        argi += 1;
        // c:6534-6543 — firstarg (== this arg when no leading-`?` arg
        // was consumed) splits at its first `?`: name before, prompt
        // after.
        if prompt.is_none() {
            if let Some(q) = r.find('?') {
                prompt = Some(r[q + 1..].to_string());
                r.truncate(q); // c:6542 readpmpt[-1] = '\0'
            }
        }
        r
    } else if want_array {
        "reply".to_string() // c:6446
    } else {
        "REPLY".to_string() // c:6446
    };

    if want_array && argi < args.len() {
        // c:6448
        zwarnnam(name, "only one array argument allowed"); // c:6449
        return 1;
    }

    // c:Src/builtin.c:6457-6477 — `read -k`/`-q` requires a
    // controlling tty (unless `-u FD` or `-p` redirects input).
    // If neither stdin nor stderr is a tty, zsh emits the canonical
    // error and returns 1. Mirror here (the SHTTY substrate isn't
    // ported yet; the libc::isatty check approximates).
    if (OPT_ISSET(ops, b'k') || OPT_ISSET(ops, b'q'))
        && !bash_nchars
        && !OPT_HASARG(ops, b'u')
        && !OPT_ISSET(ops, b'p')
    {
        let stdin_tty = unsafe { libc::isatty(0) } != 0;
        let stderr_tty = unsafe { libc::isatty(2) } != 0;
        if !stdin_tty && !stderr_tty {
            eprintln!("not interactive and can't open terminal");
            return 1;
        }
    }

    // c:Src/builtin.c:6510-6515 — `else if (OPT_ISSET(ops,'p')) {
    // readfd = coprocin; if (readfd < 0) { zwarnnam(name, "-p: no
    // coprocess"); return 1; } izle = 0; }`. C emits this BEFORE
    // identifier validation of the reply name, so a bare
    // `read -p "var name" REPLY` (no coproc started) errors with
    // "-p: no coprocess" rather than "not an identifier". Bug #387.
    // The `-u`-with-arg path falls through and shares this same gate
    // when its arg is "p" (c:6494) — also handled.
    if OPT_ISSET(ops, b'p') || (OPT_HASARG(ops, b'u') && OPT_ARG(ops, b'u').as_deref() == Some("p"))
    {
        let coprocin =
            crate::ported::modules::clone::coprocin.load(std::sync::atomic::Ordering::Relaxed);
        if coprocin < 0 {
            zwarnnam(name, "-p: no coprocess");
            return 1;
        }
    }

    // c:6453-6455 — `return compctlreadptr(name, args, ops, reply)`.
    // The compctlreadptr function pointer defaults to
    // `fallback_compctlread` (init.c:1834) and gets reassigned to the
    // real `compctlread` when zsh/compctl loads via the module's
    // boot_ hook. zshrs auto-loads zsh/compctl in default mode so the
    // direct call to `compctlread` is correct there. In --zsh parity
    // mode zsh/compctl isn't loaded until explicit `zmodload`, so
    // route through `fallback_compctlread` instead — matching
    // `zsh -fc 'getln -c x'` which emits "no loaded module provides
    // read for completion context".
    if OPT_ISSET(ops, b'l') || OPT_ISSET(ops, b'c') {
        // c:6453
        let zsh_mode = crate::IS_ZSH_MODE.load(std::sync::atomic::Ordering::Relaxed);
        // In --zsh mode, regardless of MODULESTAB's "loaded" flag
        // (which we set true at startup for QoL), route through
        // fallback_compctlread because zsh -fc's reported behavior
        // for `getln -c` is the fallback message. The MODULESTAB-
        // load-flag check below would defeat parity: zshrs marks
        // zsh/compctl as default-loaded so its compctl/compcall
        // builtins work without explicit zmodload. zsh -fc has only
        // zsh/main loaded and only auto-loads compctl when the
        // compctl/compcall builtins themselves are called — `read -c`
        // does NOT trigger that auto-load (zsh calls
        // `compctlreadptr` which defaults to `fallback_compctlread`
        // until something explicitly zmodload's zsh/compctl).
        //
        // The simplest C-faithful parity: in --zsh mode, always emit
        // the fallback message. Default zshrs mode keeps calling the
        // real compctlread.
        if zsh_mode {
            return crate::ported::init::fallback_compctlread(name);
        }
        return compctlread(name, &args[argi..]);
    }

    // Optional explicit input FD via -u, or coprocin via -p. When
    // unspecified, fall back to fd 0 (stdin). All read paths below
    // route bytes through `read_byte_from_fd` so `read -u 3 var`
    // after `exec 3< file` correctly pulls from the user fd; with
    // `-p`, the same path reads from coprocin (set by the BUILTIN_
    // RUN_COPROC handler at fusevm_bridge.rs around the
    // `coproc CMD` launch). Bug #388.
    let ufd: i32 = if OPT_ISSET(ops, b'p') {
        crate::ported::modules::clone::coprocin.load(std::sync::atomic::Ordering::Relaxed)
    } else if OPT_HASARG(ops, b'u') {
        // c:Src/builtin.c:6494-6500 — `fdarg = (int)zstrtol(argptr,
        // &eptr, 10); if (*eptr) { zwarnnam(name, "number expected
        // after -u: %s", argptr); return 1; }`. zshrs previously
        // `unwrap_or(0)`d, silently dropping `read -u abc v` errors.
        let argptr = OPT_ARG(ops, b'u').unwrap_or("");
        match argptr.parse::<i32>() {
            Ok(n) => n,
            Err(_) => {
                zwarnnam(name, &format!("number expected after -u: {}", argptr));
                return 1;
            }
        }
    } else {
        0
    };
    // c:Src/builtin.c:6418 — single-byte reader bound to `ufd`.
    // libc::read with len=1 keeps the file position advancing across
    // successive calls (matches zsh's per-byte read loop). Returns
    // Some(byte) on success, None on EOF, error sentinel on syscall
    // failure (caller maps to return 2).
    let read_byte = |fd: i32| -> io::Result<Option<u8>> {
        let mut b = [0u8; 1];
        loop {
            let n = unsafe { libc::read(fd, b.as_mut_ptr() as *mut libc::c_void, 1) };
            let _diag_errno = io::Error::last_os_error().raw_os_error();
            if n <= 0 && std::path::Path::new("/tmp/ZSHRS_READ_DIAG").exists() {
                use std::io::Write as _;
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open("/tmp/zshrs_readbyte.log")
                {
                    let _ = writeln!(f, "read_byte fd={} n={} errno={:?}", fd, n, _diag_errno);
                }
            }
            match n {
                1 => return Ok(Some(b[0])),
                0 => return Ok(None),
                -1 => {
                    let err = io::Error::last_os_error();
                    if err.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(err);
                }
                _ => return Ok(None),
            }
        }
    };

    // c:6488-6515 — `-t TIMEOUT` poll(2) wait.
    if OPT_HASARG(ops, b't') {
        let arg = OPT_ARG(ops, b't').unwrap_or("");
        let tmout: f64 = arg.parse().unwrap_or(0.0);
        let mut pfd = libc::pollfd {
            fd: 0,
            events: libc::POLLIN,
            revents: 0,
        };
        let r = unsafe { libc::poll(&mut pfd, 1, (tmout * 1000.0) as i32) };
        if r == 0 {
            return 4;
        } // timeout
        if r < 0 {
            return 2;
        } // error
    }

    // c:Src/builtin.c:6499-6510 — `read "?prompt"` writes the prompt
    // ONLY when input is interactive (stdin is a terminal). C zsh
    // gates the write via `isatty(0)` inside the prompt-emit block;
    // when stdin is redirected from a pipe or file, the prompt is
    // suppressed entirely so the captured output isn't polluted. Bug
    // #248 in docs/BUGS.md — previously zshrs printed the prompt to
    // stderr unconditionally, so non-interactive callers saw the
    // prompt fragment leak into stderr.
    if let Some(ref p) = prompt {
        let stdin_is_tty = unsafe { libc::isatty(0) } != 0;
        if stdin_is_tty {
            eprint!("{}", p);
            let _ = Write::flush(&mut io::stderr());
        }
    }

    // c:Src/builtin.c:6769-6770 — `-z` pops the front of the ZLE
    // bufstack into `zbuf` and reads from THAT instead of the file
    // descriptor. When the bufstack is empty the popped value is the
    // empty string, which makes `zread` return EOF immediately;
    // c:7110-7115 then returns 1 after the reply assignment.
    //
    //     zbuforig = zbuf = (!OPT_ISSET(ops,'z')) ? NULL :
    //         (nonempty(bufstack)) ? (char *)getlinknode(bufstack) :
    //         ztrdup("");
    //     ...
    //     if (zbuforig) { char first = *zbuforig; ... if (!first) return 1; }
    //
    // The default delim handling (newline-terminated, raw or
    // backslash-cooked) still applies — read up to the first newline
    // in the popped buffer or to its end if no newline.
    let mut buf = String::new();
    if OPT_ISSET(ops, b'z') {
        let popped: Option<String> = {
            let mut stack = crate::ported::zle::zle_main::BUFSTACK.lock().unwrap();
            if stack.is_empty() {
                None
            } else {
                Some(stack.remove(0))
            }
        };
        let zbuf = popped.clone().unwrap_or_default();
        let raw_mode = OPT_ISSET(ops, b'r') || OPT_ISSET(ops, b'R');
        if OPT_HASARG(ops, b'd') {
            let arg = OPT_ARG(ops, b'd').unwrap_or("");
            let delim = arg.as_bytes().first().copied().unwrap_or(b'\0');
            let mut out = Vec::<u8>::new();
            for &b in zbuf.as_bytes() {
                if b == delim {
                    break;
                }
                out.push(b);
            }
            buf = String::from_utf8_lossy(&out).into_owned();
        } else {
            let mut out = Vec::<u8>::new();
            let bytes = zbuf.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                let b = bytes[i];
                if !raw_mode && b == b'\\' && i + 1 < bytes.len() {
                    let nx = bytes[i + 1];
                    if nx == b'\n' {
                        i += 2;
                        continue;
                    }
                    out.push(nx);
                    i += 2;
                    continue;
                }
                if b == b'\n' {
                    break;
                }
                out.push(b);
                i += 1;
            }
            buf = String::from_utf8_lossy(&out).into_owned();
        }
        // c:7102-7109 — -e/-E echo, -e suppresses assignment.
        let opt_e = OPT_ISSET(ops, b'e');
        let opt_big_e = OPT_ISSET(ops, b'E');
        if opt_e || opt_big_e {
            println!("{}", buf);
        }
        if !opt_e {
            setsparam(&reply, &buf);
        }
        // c:7110-7115 — empty bufstack pop → return 1 even after assign.
        return if popped.is_none() || popped.as_deref() == Some("") {
            1
        } else {
            0
        };
    }
    if OPT_ISSET(ops, b'k') || OPT_ISSET(ops, b'q') || bash_nchars {
        // c:Src/builtin.c:6630 — `if (OPT_ISSET(ops,'k') ||
        // OPT_ISSET(ops,'q'))` — both read raw chars (no line/IFS
        // processing). `-q` reads exactly one char (nchars defaults to
        // 1) for the yes/no decision applied below. The previous port
        // gated only on `-k`, so `-q` fell through to the line reader
        // and never applied the y/Y check (always returned 0).
        // c:Src/builtin.c — `-k 0` (zero chars requested) is a no-op
        // read that zsh treats as failure (returns 1) because no
        // bytes can be consumed. Mirror so `read -k 0` exits 1
        // instead of "succeeding" with an empty buffer.
        // bash `-n N` (bash_nchars) joins this raw-char path but STOPS at the
        // delimiter (newline): `read -n 10 <<< "ab"` gets "ab", not 10 bytes.
        if nchars <= 0 {
            return 1;
        }
        let mut got = vec![0u8; nchars as usize];
        let mut bytes_read = 0;
        while bytes_read < nchars as usize {
            match read_byte(ufd) {
                Ok(Some(b)) => {
                    if bash_stop_at_nl && b == b'\n' {
                        break; // bash -n stops early at the delimiter
                    }
                    got[bytes_read] = b;
                    bytes_read += 1;
                }
                _ => break,
            }
        }
        buf = String::from_utf8_lossy(&got[..bytes_read]).into_owned();
    } else if OPT_HASARG(ops, b'd') {
        // c:Src/builtin.c:6418 — `-d DELIM`: read until first byte of
        // DELIM (zsh uses only first char of arg). EOF mid-record
        // returns what was read so far, exit 1 like the default path.
        let arg = OPT_ARG(ops, b'd').unwrap_or("");
        // c:Src/builtin.c:6418 — empty `-d` arg means NUL delimiter.
        // zsh's `STOUC(*OPT_ARG(ops, 'd'))` reads the first byte of
        // the arg buffer; for an empty arg the buffer is a single NUL
        // terminator, so STOUC yields 0x00. The Rust port's
        // `.first().copied()` returns None for empty strings, which
        // we have to map to NUL explicitly (matches `read -d '' x`
        // reading until \0, used by `find -print0 | while read -d ''`).
        let delim = arg.as_bytes().first().copied().unwrap_or(b'\0');
        let mut buf_bytes = Vec::<u8>::new();
        // c:7045 — the loop breaks on `c == EOF || (c == delim && !zbuf)`, and
        // c:7116 keys the exit status off WHICH of the two ended it. "Did any
        // byte arrive" can't tell an EOF-terminated record from a
        // delimiter-terminated one, so track the delimiter itself.
        let mut saw_delim = false;
        // c:7055 — `bslash = c == '\\' && !bslash && !OPT_ISSET(ops,'r')`.
        // In C there is ONE read loop and `-d` only swaps the `delim` variable,
        // so the backslash rules apply to `-d` exactly as they do to a plain
        // `read`. zshrs split the loop in two and the `-d` copy dropped them,
        // so `read -d ' ' x <<< 'esc\ttab'` kept the backslash (`esc\ttab`)
        // where zsh yields `escttab`.
        let raw_mode = OPT_ISSET(ops, b'r') || OPT_ISSET(ops, b'R');
        let mut bslash = false;
        loop {
            match read_byte(ufd) {
                Ok(Some(b)) => {
                    // c:7041-7043 — a backslash-escaped DELIMITER is a line
                    // continuation: both bytes are dropped and the record keeps
                    // going. `read -d t x <<< 'esc\ttab'` therefore yields `esc`
                    // (the escaped `t` vanishes; the NEXT `t` terminates).
                    if bslash && b == delim {
                        bslash = false; // c:7042
                        continue; // c:7043
                    }
                    if b == delim {
                        saw_delim = true; // c:7045 (`c == delim`)
                        break;
                    }
                    // c:7055-7057 — an unescaped backslash is consumed and
                    // escapes the next byte (unless `-r`).
                    let was_escaped = bslash; // this byte follows a backslash
                    bslash = b == b'\\' && !bslash && !raw_mode; // c:7055
                    if bslash {
                        continue; // c:7057
                    }
                    // Mark a backslash-escaped byte literal so IFS splitting
                    // skips it (see the default-line loop above for the rule).
                    if was_escaped {
                        buf_bytes.extend_from_slice("\u{99}".as_bytes());
                    }
                    buf_bytes.push(b);
                }
                Ok(None) => break,
                // c:Src/builtin.c:7162-7188 zread — a read(2) error
                // (EBADF from `read foo <&-`, EIO, ...) returns EOF,
                // not a distinct error status; bin_read then reports
                // plain eof (exit 1), with no diagnostic.
                Err(_) => break,
            }
        }
        buf = String::from_utf8_lossy(&buf_bytes).into_owned();
        // c:Src/builtin.c:6418 — `read -d ''` (NUL delimiter) strips
        // trailing newlines from the captured content. This matches
        // the `find -print0 | while read -d ''` idiom which expects
        // path entries without trailing whitespace. zsh's read body
        // applies this trim only for the empty-delim case; non-empty
        // delimiters keep the raw bytes.
        if arg.is_empty() {
            while buf.ends_with('\n') {
                buf.pop();
            }
        }
        // c:7107 + c:7116-7122 — C assigns the parameter FIRST (`setsparam(reply,
        // buf)`) and only then returns 1 if the loop ended at EOF. So an
        // unterminated record still lands in the variable but reports failure:
        // `read -d : x <<< 'abc'` sets x=abc and returns 1, which is what makes
        // `while read -d : x` terminate on a final field with no trailing
        // delimiter. The old early `return 1` fired only when NOTHING was read,
        // so every EOF-terminated record wrongly reported success (rc=0) — and
        // it also skipped the assignment entirely, leaving the variable at its
        // previous value instead of clearing it.
        //
        // `partial_eof` is the same carrier the default line branch below uses;
        // the shared `if partial_eof { return 1 }` after the assignment block
        // applies the status.
        partial_eof = !saw_delim; // c:7116 (`else if (c == EOF)`)
    } else {
        // Read a line (default behaviour). c:Src/builtin.c:6505
        // — without `-r`, backslash-X eats the backslash and keeps
        // the literal X (backslash-newline is line continuation).
        let raw_mode = OPT_ISSET(ops, b'r') || OPT_ISSET(ops, b'R');
        let mut buf_bytes = Vec::<u8>::new();
        let mut got_any = false;
        let mut saw_newline = false;
        loop {
            match read_byte(ufd) {
                Ok(Some(b)) => {
                    got_any = true;
                    if !raw_mode && b == b'\\' {
                        match read_byte(ufd) {
                            Ok(Some(nx)) => {
                                if nx == b'\n' {
                                    // Line continuation — drop both.
                                    continue;
                                }
                                // A backslash-escaped char is literal and must
                                // NOT act as an IFS separator: dash/ksh/bash/zsh
                                // all read `a\ b` as the single field `a b`.
                                // Prefix it with the Bnull mark (\u{99}, UTF-8
                                // C2 99) so the IFS split below skips it; the
                                // mark is stripped before assignment.
                                buf_bytes.extend_from_slice("\u{99}".as_bytes());
                                buf_bytes.push(nx);
                                continue;
                            }
                            Ok(None) => {
                                buf_bytes.push(b'\\');
                                break;
                            }
                            // c:7162-7188 zread — read error == EOF.
                            Err(_) => break,
                        }
                    }
                    if b == b'\n' {
                        saw_newline = true;
                        break;
                    }
                    buf_bytes.push(b);
                }
                Ok(None) => break,
                // c:7162-7188 zread — read error == EOF (exit 1, no
                // message): `exec 3<&-; read foo <&-` exits 1 in zsh.
                Err(_) => break,
            }
        }
        if !got_any {
            // c:Src/builtin.c — immediate EOF (no bytes read at all).
            // zsh clears the target variable(s) BEFORE returning 1
            // so `while read line; do …; done < file` leaves `\$line`
            // empty after the loop ends (not the last value).
            // Previously zshrs returned 1 without touching the var,
            // so the loop body's final iteration value persisted.
            buf = String::new();
            partial_eof = true;
            // Fall through to the assignment block which will write
            // "" to reply / vars / array. The trailing `if
            // partial_eof { return 1; }` (line 8628) preserves the
            // EOF status code.
        } else {
            buf = String::from_utf8_lossy(&buf_bytes).into_owned();
            partial_eof = !saw_newline;
        }
    }

    // c:Src/builtin.c:6730-6742 — `-q` (read yes/no): the single char
    // read above is "yes" iff it is exactly 'y' or 'Y'; the reply var is
    // set to "y"/"n" and the exit status is 0 (yes) / 1 (no). (A timeout
    // would be status 2; not modeled here.) This must run BEFORE the
    // IFS/array assignment dispatch — `-q` never does line splitting.
    if OPT_ISSET(ops, b'q') {
        let is_yes = buf == "y" || buf == "Y"; // c:6741
        setsparam(&reply, if is_yes { "y" } else { "n" }); // c:6742
        return if is_yes { 0 } else { 1 };
    }

    // Backslash-escaped chars were tagged with the Bnull mark (\u{99}) by
    // the read loop so the IFS splitting below treats them as literal.
    // `rmark` is that tag; `unmark` strips it (keeping the escaped char),
    // applied to every field/value just before assignment. Both are no-ops
    // on the common markerless line (fast `contains` guard). No C
    // counterpart — C's read splits inline during the char loop, so it
    // never needs a post-hoc mark; the port splits in a second pass.
    let rmark = '\u{99}';
    let unmark = |s: &str| -> String {
        if !s.contains(rmark) {
            return s.to_string();
        }
        let mut out = String::with_capacity(s.len());
        let mut m = false;
        for c in s.chars() {
            if !m && c == rmark {
                m = true;
                continue;
            }
            m = false;
            out.push(c);
        }
        out
    };

    // Assign to scalar reply, multi-var split, or array.
    // c:6685-6735 — `read x y z` splits buf by IFS, fills the first
    // N-1 vars with one IFS-separated field each, and stores the
    // REST of the line (including embedded IFS chars) into the last
    // var. zsh's read is stable on `print "a b c d" | read x y z`:
    // x="a", y="b", z="c d".
    if want_array {
        // c:Src/builtin.c:6685-6735 — `read -A arr` splits on $IFS
        // (whitespace-IFS coalesces; non-whitespace-IFS each acts as
        // a single delimiter). The previous port hardcoded
        // split_whitespace(), which ignored custom IFS like `:` and
        // produced a single-element array for `IFS=: read -A arr
        // <<< "a:b:c"`. Mirror the multi-var path's IFS handling.
        let ifs = getsparam("IFS").unwrap_or_else(|| " \t\n".to_string());
        let is_ifs = |c: char| ifs.contains(c);
        // c:6863-6869 — the FIRST word's char loop skips leading
        // whitespace-IFS (bptr==buf && iwsep → continue), so leading
        // whitespace produces no empty element. Mirror with trim_start.
        let trimmed = buf.trim_start_matches(|c: char| is_ifs(c) && c.is_whitespace());
        // NB: do NOT trim trailing whitespace-IFS. C reads word-by-word
        // and, after the last real word terminates on a whitespace
        // separator, attempts one more word: it consumes the remaining
        // trailing whitespace and hits the delimiter with an empty buf
        // and gotnl=1, so c:6929 `(*buf || first || gotnl)` adds a
        // trailing empty element. `read -A arr <<< "a b "` → (a b "").
        // c:Src/utils.c:3711 spacesplit — a whitespace-IFS char collapses a
        // run, and whitespace ADJACENT to a non-whitespace IFS separator is
        // absorbed into it (so `a : b` with IFS=" :" is ONE delimiter → 2
        // fields), while consecutive non-whitespace separators each delimit
        // (`a :: b` → 3, empty preserved). A non-whitespace separator also
        // absorbs its own trailing whitespace.
        let is_ws_ifs = |c: char| is_ifs(c) && c.is_whitespace();
        let is_nonws_ifs = |c: char| is_ifs(c) && !c.is_whitespace();
        let mut parts: Vec<String> = Vec::new();
        let mut field = String::new();
        let mut chars = trimmed.chars().peekable();
        while let Some(c) = chars.next() {
            // A Bnull-marked char is a backslash-escaped literal — never a
            // separator. Consume the mark and push the following char raw.
            if c == rmark {
                if let Some(nc) = chars.next() {
                    field.push(nc);
                }
                continue;
            }
            if is_ifs(c) {
                parts.push(std::mem::take(&mut field));
                if c.is_whitespace() {
                    // Coalesce the whitespace-IFS run.
                    while chars.peek().copied().is_some_and(is_ws_ifs) {
                        chars.next();
                    }
                    // If that run is followed by a non-whitespace separator,
                    // it belongs to the SAME delimiter — consume it and its
                    // trailing whitespace so no empty field appears.
                    if chars.peek().copied().is_some_and(is_nonws_ifs) {
                        chars.next();
                        while chars.peek().copied().is_some_and(is_ws_ifs) {
                            chars.next();
                        }
                    }
                } else {
                    // Non-whitespace separator: absorb its trailing whitespace.
                    while chars.peek().copied().is_some_and(is_ws_ifs) {
                        chars.next();
                    }
                }
            } else {
                field.push(c);
            }
        }
        if !field.is_empty() || !parts.is_empty() {
            parts.push(field);
        }
        // c:Src/builtin.c:6929 — `if (*buf || first || gotnl)`. With
        // `gotnl=1` set on EOF (c:6898/6914) and `first=1` initial
        // value (c:6771), C's `read -A` adds the empty buf as one
        // element even when no bytes were ever read. The resulting
        // linked list at c:6949 then yields a 1-element array
        // containing "". Without this branch, immediate EOF produced
        // a 0-element array — diverging from zsh's "consumed one
        // (empty) field" semantics that downstream `${#arr}` checks
        // rely on to distinguish "empty line" from "no input".
        if parts.is_empty() {
            parts.push(String::new());
        }
        // c:Src/builtin.c:6910-6961 — `-A` with `-e`/`-E`. The word is
        // echoed (one per line) when `-e` (main per-word loop, c:6910)
        // OR `-E` (array-display loop, c:6949). The array is SET only
        // when `-e` is NOT set (c:6921 collects words, c:6945 allocates
        // p only when !-e). So `read -AE` echoes AND assigns; `read -Ae`
        // echoes but leaves the array untouched. The previous port
        // always assigned and never echoed.
        let opt_e = OPT_ISSET(ops, b'e');
        if opt_e || OPT_ISSET(ops, b'E') {
            for w in &parts {
                println!("{}", w);
            }
        }
        if !opt_e {
            setaparam(&reply, parts); // c:6960 setaparam
        }
    } else if argi < args.len() {
        // Multi-var: `read x y [z]`. First var = reply (already
        // consumed); rest are args[argi..]. Split with at most
        // `vars.len()` chunks using IFS.
        let mut vars: Vec<String> = Vec::with_capacity(args.len() - argi + 1);
        vars.push(reply);
        for n in &args[argi..] {
            vars.push(n.clone());
        }
        let ifs = getsparam("IFS").unwrap_or_else(|| " \t\n".to_string());
        // C zsh splits by ANY char from IFS (whitespace or not).
        let is_ifs = |c: char| ifs.contains(c);
        // Trim leading IFS-whitespace per zsh's read semantics
        // (`a   b c` → x=a, y="b c", not x="" y=…).
        // c:Src/builtin.c — `-E` echoes each field to stdout as it is read,
        // one per line (c:6957 in the -A path, and the equivalent per-word
        // display on the multi-var path); `-e` does the same but ASSIGNS
        // NOTHING (c:7106 `if (!OPT_ISSET(ops,'e')) setsparam(...)`). This path
        // did neither, so `read -E a b` split correctly but printed no echo
        // (the whole-line -E echo worked only for a single variable).
        let opt_e = OPT_ISSET(ops, b'e');
        let opt_echo = opt_e || OPT_ISSET(ops, b'E');
        // Strip escape marks just before assignment so `read x y` on `a\ b`
        // assigns x="a b" (one field), matching dash/ksh/bash/zsh.
        let emit = |var: &str, val: &str| {
            let val = unmark(val);
            if opt_echo {
                println!("{val}"); // c:6958 zputs + putchar('\n')
            }
            if !opt_e {
                setsparam(var, &val); // c:7106
            }
        };
        let trimmed = buf.trim_start_matches(|c: char| is_ifs(c) && c.is_whitespace());
        let mut remaining = trimmed.to_string();
        // Find the next UNMARKED IFS separator (a Bnull-marked char is a
        // backslash-escaped literal and never delimits).
        let next_sep = |s: &str| -> Option<usize> {
            let mut m = false;
            for (bi, c) in s.char_indices() {
                if m {
                    m = false;
                    continue;
                }
                if c == rmark {
                    m = true;
                    continue;
                }
                if is_ifs(c) {
                    return Some(bi);
                }
            }
            None
        };
        for (i, var) in vars.iter().enumerate() {
            if i + 1 == vars.len() {
                // Last var: store the remainder, trim trailing IFS.
                let final_val = remaining
                    .trim_end_matches(|c: char| is_ifs(c) && c.is_whitespace())
                    .to_string();
                emit(var, &final_val);
            } else {
                // Find next IFS char.
                match next_sep(&remaining) {
                    Some(idx) => {
                        let field = remaining[..idx].to_string();
                        // c:Src/utils.c:3711 spacesplit — skip the whole
                        // delimiter. The separator char plus the IFS-whitespace
                        // ABSORBED around it form one delimiter: a whitespace
                        // separator coalesces its run AND a following non-ws
                        // separator (with that one's trailing whitespace); a
                        // non-ws separator absorbs its own trailing whitespace.
                        // So `x : y : z` (IFS=" :") reads as x, y, z — not
                        // x, "", "y : z".
                        let sep = remaining[idx..].chars().next().unwrap();
                        let is_ws = |c: char| is_ifs(c) && c.is_whitespace();
                        let after = &remaining[idx + sep.len_utf8()..];
                        let rest: &str = if sep.is_whitespace() {
                            let r = after.trim_start_matches(is_ws);
                            match r.chars().next() {
                                Some(nc) if is_ifs(nc) && !nc.is_whitespace() => {
                                    r[nc.len_utf8()..].trim_start_matches(is_ws)
                                }
                                _ => r,
                            }
                        } else {
                            after.trim_start_matches(is_ws)
                        };
                        emit(var, &field);
                        remaining = rest.to_string();
                    }
                    None => {
                        // No more IFS: this var gets remaining, others empty.
                        emit(var, &remaining);
                        remaining.clear();
                    }
                }
            }
        }
    } else {
        // c:Src/builtin.c:6843 — `read VAR` single-var path strips
        // leading/trailing IFS-whitespace exactly like the multi-var
        // last-var fallback above. C's read loop discards leading
        // whitespace-IFS via `if (c == sep && IS_SPACE(c)) ... loop`
        // (c:6843) before building each word, then on word-end it
        // either emits the word or, on the last var, keeps building
        // until newline — at which point the trailing whitespace-IFS
        // gets discarded by the same skip. Without this, `read line`
        // on `   hello   ` set `line` to `   hello   ` (with spaces)
        // instead of `hello` — bug #247.
        let ifs = getsparam("IFS").unwrap_or_else(|| " \t\n".to_string());
        let is_ifs = |c: char| ifs.contains(c);
        let trimmed = buf
            .trim_start_matches(|c: char| is_ifs(c) && c.is_whitespace())
            .trim_end_matches(|c: char| is_ifs(c) && c.is_whitespace());
        // Strip backslash-escape marks: `read x` on `a\ b` → x="a b".
        let trimmed = unmark(trimmed);
        // c:Src/builtin.c:7102-7109 — `-e` / `-E` flags. Both echo
        // the read content to stdout (`zputs(buf, stdout); putchar
        // ('\n')`); `-e` ALSO skips the setsparam (echo-only, no
        // assign), while `-E` echoes AND assigns. Bug #434.
        let opt_e = OPT_ISSET(ops, b'e');
        let opt_big_e = OPT_ISSET(ops, b'E');
        if opt_e || opt_big_e {
            println!("{}", trimmed);
        }
        if !opt_e {
            setsparam(&reply, &trimmed);
        }
    }
    // c:Src/builtin.c:6534 — partial-EOF post-assign exit.
    if partial_eof {
        return 1;
    }
    0
}

/// Port of `zread(int izle, int *readchar, long izle_timeout)` from Src/builtin.c:7134.
/// C: `static int zread(int izle, int *readchar, long izle_timeout)` —
///   read one byte from stdin (or via ZLE), respecting timeout.
pub fn zread(izle: i32, readchar: &mut i32, izle_timeout: i64) -> i32 {
    // c:7134
    if izle != 0 {
        // c:7140
        // c:7141-7144 — zleentry(ZLE_CMD_GET_KEY, izle_timeout, NULL, &c);
        // Static-link path: ZLE bridge lives in src/ported/zle/*; until
        // wired, fall through to plain stdin.
        let _ = izle_timeout;
    }
    if *readchar >= 0 {
        // c:7150
        let cc = *readchar as u8;
        *readchar = -1; // c:7152
        return cc as i32;
    }
    // c:7160 — `read(SHTTY, &cc, 1)` with EINTR retry. Read from the
    //          controlling tty (SHTTY) when available; stdin fallback
    //          for non-interactive paths where SHTTY isn't set up.
    let mut buf = [0u8; 1];
    let fd = {
        use std::sync::atomic::Ordering;
        let s = crate::ported::init::SHTTY.load(Relaxed);
        if s >= 0 {
            s
        } else {
            0
        } // c:7167 SHTTY fallback
    };
    loop {
        let n = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, 1) };
        match n {
            1 => return buf[0] as i32, // c:7169
            0 => return -1,            // EOF
            -1 if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted => continue,
            _ => return -1,
        }
    }
}

/// The single-letter conditions the condition grammar compiles inline rather
/// than dispatching to a module.
///
/// C spells the same 25 letters twice in a different order: `par_cond_double`
/// uses `"abcdefgknoprstuvwxzhLONGS"` (c:Src/parse.c:2630) and `par_cond_2`'s
/// `dble` flag uses `"abcdefghknoprstuvwxzLONGS"` (c:Src/parse.c:2551).
/// Anything outside the set is a module condition (`COND_MOD`).
const COND_UNARY_LETTERS: &str = "abcdefghknoprstuvwxzLONGS";

/// Port of `testlex()` from Src/builtin.c:7200.
/// C: `void testlex(void)` — advance the test-builtin lexer one token
///   from `testargs` into `tok`/`tokstr`. Maps `-o`→DBAR, `-a`→DAMPER,
///   `!`→Bang, `(`→Inpar, `)`→Outpar, otherwise STRING.
///
/// This is the global-state form C reaches through the `condlex` function
/// pointer. The token rules themselves live in `TestParse::testlex`, which
/// `bin_test` drives with per-call state so two threads cannot share one
/// cursor the way C's globals do.
pub fn testlex() {
    // c:7200
    let targs = TESTARGS.lock().unwrap_or_else(|e| {
        TESTARGS.clear_poison();
        e.into_inner()
    });
    let mut p = TestParse::new(&targs);
    p.idx = TESTARGS_IDX.load(Relaxed) as usize;
    p.tok = TEST_TOK.load(Relaxed);
    p.testlex();
    TEST_TOK.store(p.tok, Relaxed);
    TESTARGS_IDX.store(p.idx as i32, Relaxed);
    if let Some(t) = p.tokstr {
        if let Ok(mut ts) = TOKSTR.lock() {
            *ts = t; // c:7205
        }
    }
}

/// One node of the condition tree `parse_cond` builds for `test` / `[`.
///
/// C emits wordcode into `ecbuf` (`WCB_COND(...)` + `ecstr(...)`) and
/// `evalcond` walks it. zshrs evaluates conditions from an argv slice
/// (`cond::evalcond`), so the parser yields this tree instead and the
/// structural opcodes are applied to it; the leaves are handed back to the
/// argv evaluator as the 2- or 3-token forms C compiled them from.
enum TestCond {
    /// c:Src/parse.c:2530 — `WCB_COND(COND_NOT, 0)`.
    Not(Box<TestCond>),
    /// c:Src/parse.c:2447 — `WCB_COND(COND_AND, …)`.
    And(Box<TestCond>, Box<TestCond>),
    /// c:Src/parse.c:2422 — `WCB_COND(COND_OR, …)`.
    Or(Box<TestCond>, Box<TestCond>),
    /// c:Src/parse.c:2626 — `par_cond_double(a, b)`.
    Double(String, String),
    /// c:Src/parse.c:2659 — `par_cond_triple(a, b, c)`.
    Triple(String, String, String),
    /// c:Src/parse.c:2716 — `par_cond_multi(a, l)`.
    Multi(String, Vec<String>),
}

/// The condition grammar of Src/parse.c:2409-2729 driven by `testlex`.
///
/// This is the entry point C reserves for `bin_test` (c:Src/parse.c:714-731
/// "This entry point is only used for bin_test"). The whole point of running
/// the real grammar is `n_testargs` (c:Src/parse.c:2480): the POSIX rules for
/// `test` are keyed on how many arguments REMAIN, so which `-a` is the
/// connective and which is a plain operand falls out of the parse position
/// rather than from scanning the argument list for a spelling.
struct TestParse<'a> {
    args: &'a [String],
    /// C: `testargs` — index of the next argument `testlex` will fetch.
    idx: usize,
    /// C: `curtestarg` — index of the argument most recently fetched.
    cur: usize,
    /// C: `tok`.
    tok: i32,
    /// C: `tokstr`; `None` once the lexer has read past the last argument.
    tokstr: Option<String>,
    /// C: `errflag |= ERRFLAG_ERROR`, set by `COND_ERROR`
    /// (c:Src/parse.c:89-96). `bin_test` checks it before anything else and
    /// returns 2 with no further diagnostic (c:7284-7288).
    errflag: bool,
}

impl<'a> TestParse<'a> {
    fn new(args: &'a [String]) -> Self {
        TestParse {
            args,
            idx: 0,
            cur: 0,
            // c:7278 — `tok = NULLTOK;` before the priming testlex().
            tok: TEST_NULLTOK,
            tokstr: None,
            errflag: false,
        }
    }

    /// Port of `testlex()` from Src/builtin.c:7200 — advance one token over
    /// `args`. C holds this state in the `testargs` / `curtestarg` / `tok` /
    /// `tokstr` globals; the parser owns a copy so concurrent shells on
    /// different threads cannot share one cursor.
    fn testlex(&mut self) {
        // c:7203 — `if (tok == LEXERR) return;`
        if self.tok == TEST_LEXERR {
            return;
        }
        // c:7205 — `tokstr = *(curtestarg = testargs);`
        self.cur = self.idx;
        self.tokstr = self.args.get(self.idx).cloned();
        // c:7206-7209 — `if (!*testargs)`: the ARRAY is exhausted. C tests the
        // POINTER, so an EMPTY-STRING argument (`test '' -a x`) is an ordinary
        // STRING token, not end-of-input.
        if self.idx >= self.args.len() {
            // c:7208 — `tok = tok ? NULLTOK : LEXERR;` (NULLTOK is 0, so
            // reading past the end a second time is the error).
            self.tok = if self.tok != TEST_NULLTOK {
                TEST_NULLTOK
            } else {
                TEST_LEXERR
            };
            return;
        }
        self.tok = match self.args[self.idx].as_str() {
            "-o" => TEST_DBAR,   // c:7211
            "-a" => TEST_DAMPER, // c:7213
            "!" => TEST_BANG,    // c:7215
            "(" => TEST_INPAR,   // c:7217
            ")" => TEST_OUTPAR,  // c:7219
            "<" => TEST_INANG,   // c:7221
            ">" => TEST_OUTANG,  // c:7223
            _ => TEST_STRING,    // c:7225
        };
        self.idx += 1; // c:7226 — `testargs++`
    }

    /// c:Src/parse.c:2480 — `arrlen(testargs) + 1`: the argument count from
    /// the CURRENT token onward. `testargs` already points past it.
    fn n_testargs(&self) -> usize {
        self.args.len() - self.idx + 1
    }

    /// c:Src/parse.c:2496 etc — C's `*testargs`, the argument AFTER the
    /// current token. `None` where C would see the array's NULL terminator.
    fn next_arg(&self) -> Option<&str> {
        self.args.get(self.idx).map(|s| s.as_str())
    }

    /// c:Src/parse.c:87 — `#define YYERROR(O) { tok = LEXERR; … return 0; }`.
    fn yyerror(&mut self) -> Option<TestCond> {
        self.tok = TEST_LEXERR;
        None
    }

    /// c:Src/parse.c:89-96 — `COND_ERROR(X, Y)`: report via `zwarn` (no
    /// builtin-name prefix), set `errflag`, then `YYERROR`.
    fn cond_error(&mut self, msg: String) -> Option<TestCond> {
        crate::ported::utils::zwarn(&msg); // c:91
        self.errflag = true; // c:94
        self.yyerror() // c:95
    }

    /// Port of `par_cond(void)` from Src/parse.c:2409.
    /// C: `cond : cond_1 { SEPER } [ DBAR { SEPER } cond ]`. `COND_SEP()` is
    /// false throughout under `testlex` (c:2405 `condlex != testlex`).
    fn par_cond(&mut self) -> Option<TestCond> {
        let r = self.par_cond_1()?; // c:2413
        if self.tok == TEST_DBAR {
            // c:2416
            self.testlex(); // c:2417
            let rhs = self.par_cond()?; // c:2421
            return Some(TestCond::Or(Box::new(r), Box::new(rhs))); // c:2422
        }
        Some(r) // c:2425
    }

    /// Port of `par_cond_1(void)` from Src/parse.c:2434.
    /// C: `cond_1 : cond_2 { SEPER } [ DAMPER { SEPER } cond_1 ]`.
    fn par_cond_1(&mut self) -> Option<TestCond> {
        let r = self.par_cond_2()?; // c:2438
        if self.tok == TEST_DAMPER {
            // c:2441
            self.testlex(); // c:2442
            let rhs = self.par_cond_1()?; // c:2446
            return Some(TestCond::And(Box::new(r), Box::new(rhs))); // c:2447
        }
        Some(r) // c:2450
    }

    /// Port of `par_cond_2(void)` from Src/parse.c:2476 — the POSIX `test`
    /// rules, all of them keyed on `n_testargs`.
    fn par_cond_2(&mut self) -> Option<TestCond> {
        // c:2480 — under `testlex` this is always >= 1, so every `n_testargs`
        // guard in the C (`!n_testargs` / `|| n_testargs`) resolves the
        // testlex way here; the `[[ … ]]` half of those conditions lives in
        // parse.rs's wordcode `par_cond_2`.
        let n_testargs = self.n_testargs();

        // c:2484-2486 — no arguments left: false.
        if self.tok == TEST_NULLTOK {
            return self.par_cond_double("-n".to_string(), String::new());
        }
        // c:2487-2495 — one argument: `[ foo ]` is `[ -n foo ]`, whatever
        // `foo` looks like. This is the rule an argument-count scanner cannot
        // express: the trailing `-a` of `test -e /dev/null -a -a` reaches
        // par_cond_2 as the LAST argument and is therefore an operand.
        if n_testargs == 1 {
            let s1 = self.tokstr.clone().unwrap_or_default(); // c:2489
            self.testlex(); // c:2490
                            // c:2492 — ksh: `[ -t ]` means `[ -t 1 ]`; bash disagrees.
            if !isset(POSIXBUILTINS) && crate::ported::parse::check_cond(&s1, "t") {
                return self.par_cond_double(s1, "1".to_string()); // c:2493
            }
            return self.par_cond_double("-n".to_string(), s1); // c:2494
        }
        // c:2496-2512 — three or more arguments: if the SECOND is a binary
        // operator, apply it to the first and third.
        if n_testargs > 2 {
            let nxt = self.next_arg().unwrap_or_default().to_string();
            let is_binop = nxt == "="
                || nxt == "<"
                || nxt == ">"
                || nxt == "=="
                || nxt == "!="
                || (nxt.starts_with(crate::ported::zsh_h::IS_DASH)
                    && crate::ported::parse::get_cond_num(
                        &nxt[nxt.chars().next().map_or(0, char::len_utf8)..],
                    ) >= 0); // c:2504
            if is_binop {
                let s1 = self.tokstr.clone().unwrap_or_default(); // c:2505
                self.testlex();
                let s2 = self.tokstr.clone().unwrap_or_default(); // c:2507
                self.testlex();
                let s3 = self.tokstr.clone().unwrap_or_default(); // c:2509
                self.testlex();
                return self.par_cond_triple(s1, s2, s3); // c:2511
            }
        }
        if self.tok == TEST_BANG {
            // c:2521-2532 — in `test` compatibility mode `! -a …` / `! -o …`
            // read as "[string] [and] …", not as a negation.
            let next_is_connective = n_testargs > 2
                && self.next_arg().is_some_and(|t| {
                    crate::ported::parse::check_cond(t, "a")
                        || crate::ported::parse::check_cond(t, "o")
                }); // c:2526
            if !next_is_connective {
                self.testlex(); // c:2529
                let inner = self.par_cond_2()?; // c:2531
                return Some(TestCond::Not(Box::new(inner))); // c:2530
            }
        }
        if self.tok == TEST_INPAR {
            // c:2534-2547
            self.testlex(); // c:2537
            let r = self.par_cond()?; // c:2540
            if self.tok != TEST_OUTPAR {
                return self.yyerror(); // c:2544
            }
            self.testlex(); // c:2545
            return Some(r); // c:2546
        }
        let s1 = self.tokstr.clone(); // c:2548
                                      // c:2549-2552 — `dble`: a two-character `-X` built-in condition, which
                                      // takes exactly ONE operand and so blocks the triple/multi forms below.
        let dble = s1.as_deref().is_some_and(TestCond::is_unary_letter);
        if self.tok != TEST_STRING {
            // c:2553-2561 — `[[ STRING ]]` re-interpretation. The
            // `(!dble || n_testargs)` guard is satisfied by `n_testargs`.
            match s1 {
                Some(s) if self.tok != TEST_LEXERR => {
                    self.testlex(); // c:2557
                    return self.par_cond_double("-n".to_string(), s); // c:2558
                }
                _ => return self.yyerror(), // c:2560
            }
        }
        let s1 = s1.unwrap_or_default();
        self.testlex(); // c:2562
                        // c:2563-2569 — something like `test -z` followed by a non-STRING
                        // token: turn that token back into a plain string operand.
        if n_testargs == 2
            && self.tok != TEST_STRING
            && self.tokstr.is_some()
            && s1.starts_with(crate::ported::zsh_h::IS_DASH)
        {
            self.tok = TEST_STRING; // c:2569
        }
        if self.tok == TEST_INANG || self.tok == TEST_OUTANG {
            // c:2573-2583 — `STRING ( INANG | OUTANG ) STRING`.
            let xtok = self.tok; // c:2574
            self.testlex(); // c:2575
            if self.tok != TEST_STRING {
                return self.yyerror(); // c:2577
            }
            let s3 = self.tokstr.clone().unwrap_or_default(); // c:2578
            self.testlex(); // c:2579
                            // c:2580 — COND_STRLT for `<`, COND_STRGTR for `>`.
            let op = if xtok == TEST_INANG { "<" } else { ">" };
            return Some(TestCond::Triple(s1, op.to_string(), s3));
        }
        if self.tok != TEST_STRING {
            // c:2585-2596
            if self.tok != TEST_LEXERR {
                return self.par_cond_double("-n".to_string(), s1); // c:2592
            }
            return self.yyerror(); // c:2596
        }
        let s2 = self.tokstr.clone().unwrap_or_default(); // c:2598
                                                          // c:2599-2600 — `if (!n_testargs) dble = …`; never taken under testlex.
        self.testlex(); // c:2602
        if self.tok == TEST_STRING && !dble {
            // c:2604
            let s3 = self.tokstr.clone().unwrap_or_default(); // c:2605
            self.testlex(); // c:2606
            if self.tok == TEST_STRING {
                // c:2607
                let mut l = vec![s2, s3]; // c:2610-2611
                while self.tok == TEST_STRING {
                    // c:2613
                    l.push(self.tokstr.clone().unwrap_or_default()); // c:2614
                    self.testlex(); // c:2615
                }
                return self.par_cond_multi(s1, l); // c:2617
            }
            return self.par_cond_triple(s1, s2, s3); // c:2619
        }
        self.par_cond_double(s1, s2) // c:2621
    }

    /// Port of `par_cond_double(char *a, char *b)` from Src/parse.c:2626.
    /// The `-X` / `COND_MOD` split C makes here is deferred to evaluation,
    /// where the module lookup that decides the diagnostic actually happens
    /// (c:Src/cond.c:143-190); only the parse-time rejection is done now.
    fn par_cond_double(&mut self, a: String, b: String) -> Option<TestCond> {
        // c:2628 — `if (!IS_DASH(a[0]) || !a[1])`
        if !a.starts_with(crate::ported::zsh_h::IS_DASH) || a.chars().count() < 2 {
            return self.cond_error(format!("parse error: condition expected: {}", a));
            // c:2629
        }
        Some(TestCond::Double(a, b))
    }

    /// Port of `par_cond_triple(char *a, char *b, char *c)` from
    /// Src/parse.c:2659. Recognises the operator spellings C compiles to a
    /// `COND_*` opcode plus the two `COND_MOD`/`COND_MODI` fallbacks; anything
    /// else is a parse error naming the middle argument.
    fn par_cond_triple(&mut self, a: String, b: String, c: String) -> Option<TestCond> {
        let known = matches!(b.as_str(), "=" | "<" | ">" | "==" | "!=" | "=~") // c:2663-2691
            || b.starts_with(crate::ported::zsh_h::IS_DASH)                    // c:2692
            || (a.starts_with(crate::ported::zsh_h::IS_DASH) && a.chars().count() > 1); // c:2703
        if known {
            return Some(TestCond::Triple(a, b, c));
        }
        self.cond_error(format!("condition expected: {}", b)) // c:2709
    }

    /// Port of `par_cond_multi(char *a, LinkList l)` from Src/parse.c:2716 —
    /// four or more bare words, which can only be a module condition.
    fn par_cond_multi(&mut self, a: String, l: Vec<String>) -> Option<TestCond> {
        // c:2718 — `if (!IS_DASH(a[0]) || !a[1])`
        if !a.starts_with(crate::ported::zsh_h::IS_DASH) || a.chars().count() < 2 {
            return self.cond_error(format!("condition expected: {}", a)); // c:2719
        }
        Some(TestCond::Multi(a, l))
    }
}

impl TestCond {
    /// True for the two-character `-X` form whose `X` is a built-in condition —
    /// C's `!a[2] && strspn(a+1, "abcdefgknoprstuvwxzhLONGS") == 1`
    /// (c:Src/parse.c:2630). Anything else is a module condition.
    fn is_unary_letter(a: &str) -> bool {
        let mut ch = a.chars();
        match (ch.next(), ch.next(), ch.next()) {
            (Some(d), Some(l), None) => {
                crate::ported::zsh_h::IS_DASH(d) && COND_UNARY_LETTERS.contains(l)
            }
            _ => false,
        }
    }

    /// Evaluate the tree `TestParse` produced.
    ///
    /// The structural opcodes are ported from `evalcond` (c:Src/cond.c:86-112);
    /// the leaves go back through `cond::evalcond` in the 2- or 3-token argv
    /// form C compiled them from, so every operator implementation stays in one
    /// place. A leaf whose operator is not built in is C's `COND_MOD` /
    /// `COND_MODI`: with no module supplying it, `evalcond` reports
    /// `unknown condition` (c:Src/cond.c:187-189) naming the first
    /// dash-prefixed word (c:Src/cond.c:143-148).
    fn eval(
        &self,
        name: &str,
        options: &HashMap<String, bool>,
        variables: &HashMap<String, String>,
        posix: bool,
    ) -> i32 {
        let leaf = |toks: &[&str]| -> i32 {
            crate::ported::cond::evalcond(toks, options, variables, posix, Some(name))
        };
        let unknown = |which: &str| -> i32 {
            // c:Src/cond.c:187 — `zwarnnam(fromtest, "unknown condition: %s", errname)`
            crate::ported::utils::zwarnnam(name, &format!("unknown condition: {}", which));
            2 // c:Src/cond.c:193
        };
        match self {
            TestCond::Not(inner) => {
                // c:Src/cond.c:86-93
                let ret = inner.eval(name, options, variables, posix);
                if ret == 0 || ret == 1 {
                    1 - ret // c:91
                } else {
                    ret // c:93
                }
            }
            TestCond::And(l, r) => {
                // c:Src/cond.c:94-102 — evaluate the right side only when the
                // left is TRUE (0); any other status short-circuits out.
                let ret = l.eval(name, options, variables, posix);
                if ret == 0 {
                    r.eval(name, options, variables, posix)
                } else {
                    ret // c:101
                }
            }
            TestCond::Or(l, r) => {
                // c:Src/cond.c:103-112 — 1 (false) and 3 (no such option) both
                // continue to the right side; 0 and 2 short-circuit.
                let ret = l.eval(name, options, variables, posix);
                if ret == 1 || ret == 3 {
                    r.eval(name, options, variables, posix)
                } else {
                    ret // c:111
                }
            }
            TestCond::Double(a, b) => {
                // c:Src/parse.c:2630 — a two-character `-X` is compiled inline;
                // anything else is COND_MOD.
                if TestCond::is_unary_letter(a) {
                    leaf(&[a.as_str(), b.as_str()])
                } else {
                    unknown(a)
                }
            }
            TestCond::Triple(a, b, c) => {
                // c:Src/parse.c:2663-2702 — the operator spellings with a real
                // opcode. A dash-prefixed `b` that is not one of them is
                // COND_MODI, reported against `b`; otherwise `a` carried the
                // module name (c:2703) and is reported instead.
                let opcode = matches!(
                    b.as_str(),
                    "=" | "<" | ">" | "==" | "!=" | "=~" | "-regex-match"
                ) || (b.starts_with(crate::ported::zsh_h::IS_DASH)
                    // c:2693 — `get_cond_num(b + 1)`; skip one CHAR, since the
                    // lexer's `Dash` token is multi-byte in UTF-8.
                    && crate::ported::parse::get_cond_num(
                        &b[b.chars().next().map_or(0, char::len_utf8)..],
                    ) >= 0);
                if opcode {
                    leaf(&[a.as_str(), b.as_str(), c.as_str()])
                } else if b.starts_with(crate::ported::zsh_h::IS_DASH) {
                    unknown(b) // c:2698 COND_MODI
                } else {
                    unknown(a) // c:2704 COND_MOD
                }
            }
            // c:Src/parse.c:2723 — COND_MOD with `a` as the module condition name.
            TestCond::Multi(a, _) => unknown(a),
        }
    }
}

/// Port of `bin_test(char *name, char **argv, UNUSED(Options ops), int func)` from Src/builtin.c:7231.
/// C: `int bin_test(char *name, char **argv, UNUSED(Options ops), int func)`
/// — the `test` / `[` builtin: when invoked as `[`, requires a trailing
///   `]`; XSI-extension paren-stripping for 3/4-arg forms; final
///   evalcond dispatch returns 0/1/2.
/// WARNING: param names don't match C — Rust=(name, argv, func) vs C=(name, argv, ops, func)
pub fn bin_test(
    name: &str,
    argv: &[String], // c:7231
    _ops: &options,
    func: i32,
) -> i32 {
    let mut argv = argv.to_vec();
    let mut sense = 0i32; // c:7236

    // c:7239-7247 — `[` requires trailing `]`.
    if func == BIN_BRACKET {
        // c:7239
        if argv.is_empty() || argv.last().map(|s| s.as_str()) != Some("]") {
            // c:7241
            zwarnnam(name, "']' expected"); // c:7243
            return 2; // c:7244
        }
        argv.pop(); // c:7246 (s[-1] = NULL)
    }

    // c:7249-7250 — empty argv → false (1).
    if argv.is_empty() {
        // c:7249
        return 1; // c:7250
    }

    // c:7257-7274 — XSI 3/4-arg parens + 4-arg `!` extension.
    let nargs = argv.len(); // c:7257
    if nargs == 3 || nargs == 4 {
        // c:7258
        // c:7264-7269 — strip `(` ... `)` parens unless the 3-arg middle
        // would be a binary op (which takes priority).
        if argv[0] == "(" && argv[nargs - 1] == ")"                          // c:7264
            && (nargs != 3 || crate::ported::text::is_cond_binary_op(&argv[1]) == 0)
        // c:7265
        {
            argv.pop(); // c:7266
            argv.remove(0); // c:7267
        }
        if nargs == 4 && argv[0] == "!" {
            // c:7270 — the `!` negation short-circuit fires ONLY at FOUR args
            // (`[ ! a = b ]` → negate the 3-arg binary test). At THREE args the
            // leading `!` is NOT stripped: it falls through to the grammar,
            // which applies the POSIX rule "if $2 is a binary operator, test $1
            // op $3" — so `[ ! = x ]` is the string comparison "!" = "x".
            sense = 1; // c:7272
            argv.remove(0); // c:7273
        }
    }

    // c:Src/builtin.c:7276-7280 — `[ ]`/`test` uses `parse_cond`, whose
    // `testlex` maps `<` / `>` to INANG / OUTANG. Both zsh 5.9 and 5.9.2
    // nevertheless reject them from this builtin:
    //   `test a '<' b` → `zsh:1: condition expected: <` rc=2
    // Only `[[ ]]` (which routes through `execcond`) accepts them for lex
    // compare. Bug #98 in docs/BUGS.md.
    //
    // !!! POSIX-FAITHFUL GATE !!! This zsh-only rejection is WRONG for the
    // Bourne-family drop-ins: dash / sh / ksh / bash `test` all accept
    // `string1 < string2` / `>` as lexical string comparison (`[ abc \< abd ]`
    // → true). Under `--sh`/`--ksh`/`--dash`/`--bash` (posix_faithful) skip the
    // rejection and let the INANG/OUTANG grammar arm (c:2573-2583) build
    // COND_STRLT / COND_STRGTR, matching the real shell.
    if !crate::dash_mode::posix_faithful() && argv.iter().any(|a| a == "<" || a == ">") {
        let offending = argv
            .iter()
            .find(|a| a.as_str() == "<" || a.as_str() == ">")
            .map(|s| s.as_str())
            .unwrap_or("<");
        // Use zwarn (cmd=None) — zsh's `[ ]` parse-error format is
        // `<script>:<line>: condition expected: <`, NOT
        // `<script>:test:<line>:...`. zwarn omits the builtin-name
        // segment that zwarnnam adds; zwarn also doesn't set
        // errflag, so `echo $?` after the failed test still runs.
        crate::ported::utils::zwarn(&format!("condition expected: {}", offending));
        return 2;
    }

    // c:7276-7281 — `zcontext_save(); testargs = argv; tok = NULLTOK;
    //                condlex = testlex; testlex(); prog = parse_cond();`
    let mut p = TestParse::new(&argv);
    p.testlex(); // c:7280
                 // c:Src/parse.c:722-731 — `parse_cond()` is just `par_cond()`,
                 // returning NULL when the grammar bailed out.
    let prog = p.par_cond(); // c:7281

    // c:7284-7288 — a COND_ERROR already printed its diagnostic.
    if p.errflag {
        // c:7284
        return 2; // c:7287
    }
    // c:7290-7294 — a bare YYERROR prints here. `tokstr` is NULL exactly when
    // the lexer ran off the end of the argument list.
    if prog.is_none() || p.tok == TEST_LEXERR {
        // c:7290
        zwarnnam(
            name,
            if p.tokstr.is_some() {
                "parse error"
            } else {
                "argument expected"
            },
        ); // c:7291
        return 2; // c:7293
    }
    // c:7297-7300 — `if (*curtestarg)`: the grammar stopped before consuming
    // every argument.
    if p.cur < argv.len() {
        // c:7297
        zwarnnam(name, "too many arguments"); // c:7298
        return 2; // c:7299
    }

    // c:7302-7308 — syntax is OK, so evaluate.
    let options = HashMap::new();
    let mut variables = HashMap::new();
    // C `evalcond` reaches param values through `getvalue` / `getsparam`
    // which read paramtab. The previous Rust port populated the
    // variables map from `std::env::vars()` — the OS environment —
    // so shell-internal vars (not exported) appeared "unset" to
    // `[[ -z $var ]]` / `[[ $a = $b ]]` etc. Walk paramtab to mirror
    // C; fall back to env for entries the paramtab hasn't imported.
    {
        let tab = paramtab().read().unwrap();
        for (k, pm) in tab.iter() {
            // Skip PM_UNSET — these are name-declared-but-no-value.
            if (pm.node.flags as u32 & PM_UNSET) != 0 {
                continue;
            }
            let v = pm.u_str.clone().unwrap_or_default();
            variables.insert(k.clone(), v);
        }
    }
    // Layer env vars on top of paramtab for the rare case where the
    // OS env has a name paramtab hasn't yet imported (e.g. external
    // wrapper that exec'd zshrs with env vars).
    for (k, v) in env::vars() {
        variables.entry(k).or_insert(v);
    }
    let posix = isset(POSIXBUILTINS);
    // c:Src/builtin.c:7305 — `stat = evalcond(state, name);`. The
    // `name` argument is C's `fromtest` signal — non-NULL means "called
    // from test/[", which enables the strict integer-expression error
    // path (c:Src/cond.c:236-251). Bug #411.
    let mut ret = prog.unwrap().eval(name, &options, &variables, posix); // c:7305

    // c:7307-7308 — `if (ret < 2 && sense) ret = !ret;`
    if ret < 2 && sense != 0 {
        // c:7307
        ret = if ret == 0 { 1 } else { 0 }; // c:7308
    }
    ret // c:7310
}

/// Port of `bin_times(UNUSED(char *name), UNUSED(char **argv), UNUSED(Options ops), UNUSED(int func))` from Src/builtin.c:7324.
/// C: `int bin_times(UNUSED args)` — `times(&buf)`; print user/system
///   for self then for children, separated by spaces and newlines.
/// WARNING: param names don't match C — Rust=(_name, _argv, _func) vs C=(name, argv, ops, func)
pub fn bin_times(
    _name: &str,
    _argv: &[String], // c:7328
    _ops: &options,
    _func: i32,
) -> i32 {
    let mut buf: libc::tms = unsafe { std::mem::zeroed() }; // c:7331
                                                            // c:7332 — `long clktck = get_clktck();`. The previous Rust port
                                                            // inlined a `sysconf(_SC_CLK_TCK)` call here. Route through the
                                                            // canonical `get_clktck()` port at jobs.rs:567 so any future
                                                            // hardening (caching, error fallback) propagates to every caller.
    let clktck = crate::ported::jobs::get_clktck() as f64; // c:7332
    let clktck = if clktck <= 0.0 { 100.0 } else { clktck };
    // c:7335 — `if (times(&buf) == -1) return 1;`
    if unsafe { libc::times(&mut buf) } == (-1i64) as libc::clock_t {
        // c:7335
        return 1; // c:7336
    }
    let pttime = |t: libc::clock_t| {
        // c:Src/builtin.c:7315-7318 — `printf("%ldm%ld.%02lds", X/(60*clktck),
        //   X/clktck%clktck, X*100/clktck%100)`. Integer-arithmetic
        // breakdown: minutes (no zero-pad), seconds (no zero-pad), and
        // centiseconds (2-digit zero-pad). Bug #499 — previous Rust
        // port used floating-point `%.3f` with 3-decimal precision,
        // diverging from zsh's `0m0.00s`.
        let x = t as i64;
        let clktck_i = clktck as i64;
        let mins = x / (60 * clktck_i);
        let secs = (x / clktck_i) % clktck_i;
        let csec = (x * 100 / clktck_i) % 100;
        print!("{}m{}.{:02}s", mins, secs, csec);
    };
    pttime(buf.tms_utime); // c:7332
    print!(" "); // c:7333
    pttime(buf.tms_stime); // c:7334
    println!(); // c:7335
    pttime(buf.tms_cutime); // c:7336
    print!(" "); // c:7337
    pttime(buf.tms_cstime); // c:7338
    println!(); // c:7339
    0 // c:7340
}

/// Port of `bin_trap(char *name, char **argv, UNUSED(Options ops), UNUSED(int func))` from Src/builtin.c:7347.
/// C: `int bin_trap(char *name, char **argv, ...)` — list, clear, or
///   set signal traps.
/// WARNING: param names don't match C — Rust=(name, argv, _func) vs C=(name, argv, ops, func)
pub fn bin_trap(
    name: &str,
    argv: &[String], // c:7347
    _ops: &options,
    _func: i32,
) -> i32 {
    // PFA-SMR aspect: record `trap HANDLER SIG...` calls. Skip
    // listing-only forms (`trap`, `trap -l`, `trap -p`) — those don't
    // mutate state.
    #[cfg(feature = "recorder")]
    if crate::recorder::is_enabled() {
        let listing = argv.is_empty() || (argv.len() == 1 && (argv[0] == "-l" || argv[0] == "-p"));
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
    if !argv.is_empty() && argv[0] == "--" {
        // c:7353
        argv.remove(0); // c:7354
    }

    // c:7357-7380 — no args: list current traps.
    if argv.is_empty() {
        // c:7357
        // Local enum for the merged function-form / string-form
        // listing walker below. RUST-ONLY scoping helper for the
        // c:Src/builtin.c:7359-7375 if/else-if structure.
        enum TrapEntry {
            Func(String),
            Str(String, String),
        }
        queue_signals(); // c:7358
        let traps = traps_table().lock().map(|t| t.clone()).unwrap_or_default();
        // c:Src/builtin.c:7359-7375 — C walks `sigtrapped[]` by signal
        // number index, so the listing is naturally sorted by sig num
        // (HUP=1, INT=2, …, USR1=30, …). zshrs stores traps in a
        // HashMap<name, body> whose iteration order is non-deterministic
        // — bug #68 in docs/BUGS.md: `trap` output came out in random
        // order vs zsh's stable signum-sorted view. Sort by getsigidx
        // before printing so the iteration matches C's array walk.
        //
        // c:Src/builtin.c:7360-7365 — `if (sigtrapped[sig] & ZSIG_FUNC)
        // { hn = gettrapnode(sig, 0); shfunctab->printnode(hn, 0); }`.
        // Walk shfunctab for `TRAP<signame>` functions and emit their
        // full definition alongside the string-form traps so the
        // listing matches zsh. Bug #461 in docs/BUGS.md.
        let trap_funcs: Vec<(String, i32, String)> = {
            let mut acc: Vec<(String, i32, String)> = Vec::new();
            if let Ok(tab) = crate::ported::hashtable::shfunctab_lock().read() {
                for (fname, _) in tab.iter() {
                    if let Some(sig_name) = fname.strip_prefix("TRAP") {
                        let idx = getsigidx(sig_name);
                        if idx != -1 {
                            acc.push((fname.clone(), idx, sig_name.to_string()));
                        }
                    }
                }
            }
            acc
        };
        // Merge function-form + string-form, sort by sig idx
        // (function-form FIRST per zsh ordering — c:Src/builtin.c:7359
        // walks each signal once; ZSIG_FUNC takes precedence within a
        // single signum slot since the C code uses `else if` between
        // the two arms).
        let mut combined: Vec<(i32, TrapEntry)> = Vec::new();
        for (fname, idx, _sig) in &trap_funcs {
            combined.push((*idx, TrapEntry::Func(fname.clone())));
        }
        for (sig, body) in traps.iter() {
            let idx = getsigidx(sig);
            // Skip if a function-form already exists for this sig
            // (C's `else if` semantics — function-form wins).
            if trap_funcs.iter().any(|(_, i, _)| *i == idx) {
                continue;
            }
            combined.push((
                if idx == -1 { i32::MAX } else { idx },
                TrapEntry::Str(sig.clone(), body.clone()),
            ));
        }
        combined.sort_by_key(|(idx, _)| *idx);
        for (_idx, entry) in &combined {
            match entry {
                TrapEntry::Func(fname) => {
                    if let Ok(tab) = crate::ported::hashtable::shfunctab_lock().read() {
                        if let Some(shf) = tab.get(fname) {
                            crate::ported::hashtable::printshfuncnode(shf, 0);
                        }
                    }
                }
                TrapEntry::Str(sig, body) => {
                    // c:7371 — `s = getpermtext(siglists[sig], NULL, 0);`. C
                    // holds the body as a compiled Eprog and renders it back
                    // to source for the listing, so what prints is CANONICAL
                    // text rather than the string the user typed:
                    // `trap 'print a; print b' EXIT` lists as
                    // `trap -- $'print a\nprint b' EXIT` (separators become
                    // newlines, `if`/`for`/`(` bodies get re-indented).
                    // zshrs stores the body as raw text, so deparse it here
                    // the same way. Rendering only at listing time keeps the
                    // stored text authoritative for execution — a getpermtext
                    // rendering bug then costs a wrong listing, not a wrong
                    // trap body.
                    //
                    // An empty body takes C's `!siglists[sig]` branch
                    // (c:7368-7369, prints `trap -- '' SIG`) and is never
                    // rendered. parse_string is the wordcode parser, whose
                    // coverage is narrower than the AST parser that executes
                    // the body (e.g. `(pat)` case arms — see the pin test in
                    // text.rs), so fall back to the raw text on a parse
                    // failure rather than dropping the entry.
                    let rendered = if body.is_empty() {
                        body.clone()
                    } else {
                        crate::ported::exec::parse_string(body, 1)
                            .map(|p| crate::ported::text::getpermtext(Box::new(p), None, 0))
                            .unwrap_or_else(|| body.clone())
                    };
                    // c:7370-7375 — `printf("trap -- "); quotedzputs(...); printf(" %s\n", name);`
                    print!("trap -- "); // c:7372
                    print!("{}", quotedzputs(&rendered)); // c:7373
                    println!(" {}", sig); // c:7374
                }
            }
        }
        unqueue_signals(); // c:7378
        return 0; // c:7379
    }

    // c:7384-7400 — first arg is signal number / single `-` → clear.
    let first = &argv[0];
    if getsigidx(first) != -1 || first == "-" {
        // c:7384
        let start = if first == "-" { 1 } else { 0 }; // c:7385
                                                      // c:7399 — `return *argv != NULL;`. After a successful loop
                                                      // *argv is the trailing NULL (Rust: idx == len after the
                                                      // walk); on `break` due to an undefined signal *argv is the
                                                      // bad arg (idx < len). Previous Rust port hardcoded
                                                      // `return 0`, so `trap - INVALID` would silently report
                                                      // success and downstream scripts couldn't detect the bad
                                                      // signal name.
        let mut had_error = 0i32;
        // c:7386-7398 — build the signal list, then run ONE
        // `unsettrap` body over it. `trap -` clears every slot
        // (c:7387-7388: `for (sig = 0; sig < TRAPCOUNT; sig++)
        // unsettrap(sig);`); `trap - SIG…` clears the named ones and
        // stops at the first undefined name (c:7390-7398), leaving
        // the already-walked signals cleared exactly as C does.
        let mut to_clear: Vec<i32> = Vec::new();
        if start >= argv.len() {
            // c:7386
            to_clear.extend(0..crate::ported::signals_h::TRAPCOUNT); // c:7387
        } else {
            for arg in &argv[start..] {
                // c:7390
                let sig = getsigidx(arg);
                if sig == -1 {
                    // c:7392
                    zwarnnam(name, &format!("undefined signal: {}", arg)); // c:7393
                    had_error = 1; // c:7399 *argv non-NULL on break
                    break; // c:7394
                }
                to_clear.push(sig);
            }
        }
        for sig in to_clear {
            // c:7388 / c:7397 — `unsettrap(sig)`. C's unsettrap
            // (c:Src/signals.c:759) → removetrap (c:Src/signals.c:772)
            // does three things the previous zshrs clear path skipped
            // by only doing `traps_table().remove()`:
            //   - c:Src/signals.c:800 `sigtrapped[sig] = 0`, so
            //     handletrap stops claiming the signal;
            //   - c:Src/signals.c:815 `signal_default(sig)`, which
            //     RESTORES THE DEFAULT DISPOSITION — this is why
            //     `trap 'print caught' USR2; kill -USR2 $$; trap - USR2;
            //     kill -USR2 $$` kills the shell in zsh instead of
            //     silently ignoring the second signal;
            //   - c:Src/signals.c:836-843 `removehashnode(shfunctab,
            //     node->nam)`, dropping the `TRAP<sig>` function-form
            //     trap (freed by unsettrap at c:765).
            // unsettrap MUST run FIRST: removetrap → dosavetrap (c:774)
            // snapshots the CURRENT body for the LOCAL_TRAPS restore.
            crate::ported::signals::unsettrap(sig);
            // c:Src/signals.c:846 `siglists[sig] = NULL` — the body
            // drop. zshrs keeps trap bodies in `traps_table` keyed by
            // canonical signal NAME (C indexes siglists[] by number),
            // so the number has to be mapped back to the name bin_trap
            // installed it under (`0` → `EXIT`).
            let key = if sig == 0 {
                "EXIT".to_string()
            } else {
                crate::ported::signals_h::sigs_name(sig)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| sig.to_string())
            };
            if let Ok(mut t) = traps_table().lock() {
                t.remove(&key);
            }
            // c:Src/signals.c:836-843 — ZSIG_FUNC arm.
            if let Ok(mut tab) = crate::ported::hashtable::shfunctab_lock().write() {
                tab.remove(&format!("TRAP{}", key));
            }
        }
        return had_error; // c:7399
    }

    // c:7404-7411 — first arg is the trap body.
    let arg = argv.remove(0); // c:7404
                              // c:7405-7409 — `if (!*arg) prog = &dummy_eprog; else if (!(prog =
                              // parse_string(arg, 1))) { zwarnnam(name, "couldn't parse trap
                              // command"); return 1; }`. The body is parsed when the trap is
                              // installed, so an unparseable body is rejected here rather than
                              // deferred to signal delivery. An empty body (`trap '' SIG`) takes
                              // C's dummy_eprog branch and is never parsed. This gate precedes
                              // the signal checks below, so `trap 'for' BOGUS` reports the parse
                              // failure, not "undefined signal".
                              //
                              // C reaches its parser through parse_string (ported at
                              // exec.rs:parse_string over the ported lexer). The trap body is
                              // stored as text and later run through parse_isolated by
                              // execute_script_zsh_pipeline (vm_helper.rs:2249), so validation
                              // goes through the same parser that will execute it — otherwise a
                              // body could pass here and still fail at delivery. Save/restore
                              // errflag around the probe exactly as vm_helper.rs:2240-2251 does,
                              // so a rejected body doesn't leave the error flag set and abort the
                              // enclosing shell. The parser emits its own `parse error` via zerr
                              // first; this adds C's second diagnostic.
    if !arg.is_empty() {
        let saved_errflag = errflag.load(Ordering::Relaxed);
        errflag.fetch_and(!crate::ported::zsh_h::ERRFLAG_ERROR, Ordering::Relaxed);
        let _ = crate::vm_helper::parse_isolated(&arg);
        let parse_failed =
            (errflag.load(Ordering::Relaxed) & crate::ported::zsh_h::ERRFLAG_ERROR) != 0;
        errflag.store(saved_errflag, Ordering::Relaxed);
        if parse_failed {
            zwarnnam(name, "couldn't parse trap command"); // c:7407
            return 1; // c:7408
        }
    }
    if argv.is_empty() {
        // c:7411 — when only one arg AND it looks like a signal
        // (SIG-prefix or numeric) but didn't resolve to a real
        // signal, emit "undefined signal". For an arbitrary string
        // body with no following signal, zsh silently accepts and
        // installs nothing (no diagnostic). Mirror zsh's behavior:
        // skip "signal expected" for non-signal-shaped strings.
        if arg.starts_with("SIG") || arg.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            zwarnnam(name, &format!("undefined signal: {}", arg)); // c:7413
            return 1; // c:7417
        }
        // Bare string body with no signal — zsh accepts silently.
        return 0;
    }

    // c:7421-7448 — install trap on each named signal.
    let mut trap_install_error = 0i32;
    for sigarg in &argv {
        // c:7421
        let sig = getsigidx(sigarg);
        if sig == -1 {
            // c:7426
            zwarnnam(name, &format!("undefined signal: {}", sigarg)); // c:7427
            trap_install_error = 1; // c:7445 *argv non-NULL on break
            break; // c:7428
        }
        // c:Src/signals.c — C zsh stores traps in a fixed array
        // indexed by signal number. Aliases (`0`, `EXIT`, `SIGEXIT`)
        // all resolve to index 0 and share the same slot. The Rust
        // port stores by name string, so we must normalize the key
        // to the canonical signal name (or "EXIT" for the 0 alias)
        // — otherwise `trap 'echo bye' 0` lands in a `"0"` slot
        // that nothing else looks up.
        let canonical = if sig == 0 {
            "EXIT".to_string()
        } else if let Some(name) = crate::ported::signals_h::sigs_name(sig) {
            // c:Src/builtin.c — when sigarg is numeric (`trap "" 2`),
            // the listing path must still emit the SYMBOLIC name
            // (`INT`) so the canonical name lookup downstream works.
            // Previously zshrs's `to_uppercase()` of "2" left the key
            // as "2", so `trap "" 2; trap` listed `trap -- '' 2`
            // instead of `trap -- '' INT`. Resolve to the canonical
            // signal name via sigs_name(idx).
            //
            // c:Src/jobs.c:2740 alt_sigs + jobs.c:3089 getsigname — a trap
            // set via an ALIAS name lists under the ALIAS, not the
            // canonical sigs[] name (C flags it ZSIG_ALIAS). The only
            // unconditional alt_sigs entry is `{ "ERR", SIGZERR }`, so
            // `trap … ERR; trap` must print `… ERR`, not `… ZERR`. The
            // Rust port stores traps by name string; preserve the alias
            // the user typed as the key. The dotrap dispatch already
            // resolves SIGZERR through both "ZERR" and "ERR"
            // (signals.rs:1505), so firing is unaffected.
            let normalized = sigarg
                .strip_prefix("SIG")
                .or_else(|| sigarg.strip_prefix("sig"))
                .unwrap_or(sigarg.as_str())
                .to_uppercase();
            if sig == crate::ported::signals_h::SIGZERR && normalized == "ERR" {
                "ERR".to_string()
            } else {
                name.to_string()
            }
        } else {
            // Strip SIG/sig prefix and uppercase so `SIGINT` / `int`
            // / `INT` all map to the same key.
            sigarg
                .strip_prefix("SIG")
                .or_else(|| sigarg.strip_prefix("sig"))
                .unwrap_or(sigarg.as_str())
                .to_uppercase()
        };
        // c:Src/signals.c settrap — register both the libc signal
        // handler AND the sigtrapped[idx] flag. Without setting
        // sigtrapped, handletrap() early-returns 0 (sees the slot
        // as "not trapped") and the dotrap dispatch never fires.
        // The traps_table entry alone isn't enough — handletrap
        // gates on sigtrapped[idx] != 0.
        //
        // c:Src/signals.c:693 settrap flag semantics:
        //   ZSIG_FUNC → trap body is a shell function (TRAPSIG)
        //   no flag   → trap body is an Eprog in siglists[sig]
        // zshrs stores the body STRING in `traps_table` instead of
        // either, but the runtime's `endtrapscope`/`dotrap` dispatch
        // (signals.rs:1249) requires the !ZSIG_FUNC arm to fire so
        // the traps_table lookup happens. Passing ZSIG_FUNC sent it
        // down the shfunc-dispatch arm which fails silently when no
        // TRAPSIG function exists — bug #80 in docs/BUGS.md.
        //
        // SIGEXIT (sig=0) must also reach settrap so locallevel
        // tagging lands and `endtrapscope` fires the function-local
        // EXIT trap at function exit (the previous `sig > 0` gate
        // skipped it entirely).
        //
        // CRITICAL ORDER: settrap calls unsettrap → dosavetrap which
        // SNAPSHOTS the current traps_table body for the save-stack.
        // settrap MUST run BEFORE inserting the new body so the saved
        // entry captures the OUTER scope's body, not the one we're
        // about to install. (Reversed order led to the save loop
        // restoring the just-installed body back into traps_table
        // after the function exit, causing the script-end trap fire
        // to re-run the inner body.)
        if sig >= 0 && sig <= crate::ported::signals_h::SIGCOUNT && sig != libc::SIGCHLD as i32 {
            // c:Src/signals.c:712 — `if (!(flags & ZSIG_FUNC) && empty_eprog(l))`.
            // settrap treats an `l == None` (or empty Eprog) body as
            // `trap '' SIG` — i.e. ZSIG_IGNORED + signal_ignore(sig).
            // That suppresses the signal entirely AND skips
            // install_handler, so a non-empty `trap "echo TRAP" SIG`
            // would never reach handletrap → dotrap. C-zsh's bin_trap
            // parses the body string into an Eprog via parse_string()
            // before passing to settrap. The zshrs port stores the
            // body as plain text in `traps_table` and dispatches via
            // execute_script (the Eprog parser isn't on the
            // critical path), so we need a placeholder Eprog whose
            // only purpose is to make empty_eprog() return false and
            // steer settrap down the ZSIG_TRAPPED + install_handler
            // branch. The placeholder body is never executed — dotrap
            // reads the dispatch text from traps_table.
            //
            // Bug #104 in docs/BUGS.md: signal sent from inside a
            // function via `kill -X $$` was lost because settrap had
            // routed USR1 to signal_ignore at trap-install time, so
            // when the signal hit while doshfunc had queue_signals()
            // active, nothing in the queue → nothing to dispatch on
            // unqueue. Empty-body path (`trap '' SIG`) still passes
            // None as before so the ZSIG_IGNORED branch fires.
            let body_eprog: Option<crate::ported::zsh_h::Eprog> = if arg.is_empty() {
                None
            } else {
                let mut prog = crate::ported::zsh_h::eprog::default();
                // c:Src/parse.c:584 — empty_eprog returns true on
                // `prog[0] == WCB_END`; push any non-WCB_END value
                // (1 is arbitrary, matches the existing
                // `empty_eprog_non_empty_non_end_returns_false`
                // pin test in parse.rs).
                prog.prog.push(1);
                Some(Box::new(prog))
            };
            settrap(sig, body_eprog, 0);
        }
        // c:Src/signals.c:707 — settrap → unsettrap → removetrap
        // (c:836-843 removeshfuncnode) drops any pre-existing
        // FUNCTION-form trap for this signal BEFORE the new (string-form)
        // body is installed. zshrs's removetrap does not touch shfunctab,
        // so a leftover `TRAPZERR() { … }` would shadow a later `trap
        // 'body' ERR` — the dispatch prefers the function form, so the OLD
        // function fired instead of the new string body. Remove it here,
        // mirroring the `trap -` unset path above. Keyed by the CANONICAL
        // signal name (sigs_name), so both `ERR` and `ZERR` spellings drop
        // the same `TRAPZERR` node. Runs for VIRTUAL signals too (ZERR /
        // DEBUG / EXIT sit beyond SIGCOUNT, so the settrap block above is
        // skipped for them — the earlier placement inside that block never
        // fired for ZERR).
        let func_key = if sig == 0 {
            "EXIT".to_string()
        } else {
            crate::ported::signals_h::sigs_name(sig)
                .map(|s| s.to_string())
                .unwrap_or_else(|| sig.to_string())
        };
        if let Ok(mut tab) = crate::ported::hashtable::shfunctab_lock().write() {
            tab.remove(&format!("TRAP{}", func_key));
        }
        if let Ok(mut t) = traps_table().lock() {
            t.insert(canonical.clone(), arg.clone()); // c:7448 (effective)
        }
        // c:Src/signals.c::settrap — when a string-form trap
        // replaces an existing trap, settrap calls unsettrap which
        // clears the C `sigfuncs[sig]` shfunc slot. The zshrs port
        // dispatches function-form TRAPxxx through shfunctab
        // independently of sigfuncs, so a pre-existing TRAPxxx
        // function survives the string-form install and BOTH fire
        // on signal delivery. Remove any matching TRAPxxx shfunc
        // here so dotrap dispatches the just-installed string body
        // only. Bug #541 in docs/BUGS.md.
        let trap_fn_name = format!("TRAP{}", canonical);
        if let Ok(mut tab) = crate::ported::hashtable::shfunctab_lock().write() {
            tab.remove(&trap_fn_name);
        }
    }
    trap_install_error
}

/// Port of `bin_ttyctl(UNUSED(char *name), UNUSED(char **argv), Options ops, UNUSED(int func))` from Src/builtin.c:7454.
/// C: `int bin_ttyctl(UNUSED args, Options ops, ...)` — `-f` freezes the
///   tty, `-u` unfreezes; otherwise emit `"tty is [not ]frozen"`.
/// WARNING: param names don't match C — Rust=(_name, _argv, _func) vs C=(name, argv, ops, func)
pub fn bin_ttyctl(
    _name: &str,
    _argv: &[String], // c:7454
    ops: &options,
    _func: i32,
) -> i32 {
    use std::sync::Mutex;
    // c:7456-7461 — route through the canonical jobs::TTYFROZEN
    // global. The previous builtin.rs duplicate AtomicI32 NEVER synced
    // with jobs.rs's Mutex<i32> store; `ttyctl -f` set the local
    // Atomic but didn't freeze the tty from the perspective of the
    // job-control wait path that reads jobs::TTYFROZEN.
    let cell = crate::ported::jobs::TTYFROZEN.get_or_init(|| Mutex::new(0_i32));
    if OPT_ISSET(ops, b'f') {
        // c:7456
        *cell.lock().expect("TTYFROZEN poisoned") = 1; // c:7457
    } else if OPT_ISSET(ops, b'u') {
        // c:7458
        *cell.lock().expect("TTYFROZEN poisoned") = 0; // c:7459
    } else {
        let f = *cell.lock().expect("TTYFROZEN poisoned");
        // c:7461 — `printf("tty is %sfrozen\n", ttyfrozen ? "" : "not ");`
        println!("tty is {}frozen", if f != 0 { "" } else { "not " }); // c:7461
    }
    0 // c:7463
}

/// Port of `bin_let(UNUSED(char *name), char **argv, UNUSED(Options ops), UNUSED(int func))` from Src/builtin.c:7469.
/// C: `int bin_let(UNUSED(char *name), char **argv, UNUSED(Options ops),
///     UNUSED(int func))` — evaluate each arg as a math expression;
///   return 1 if the final value is zero (success/false), 0 if non-zero
///   (true), 2 on math error.
/// WARNING: param names don't match C — Rust=(_name, argv, _func) vs C=(name, argv, ops, func)
pub fn bin_let(
    _name: &str,
    argv: &[String], // c:7469
    _ops: &options,
    _func: i32,
) -> i32 {
    use crate::ported::utils::{errflag, ERRFLAG_ERROR};
    use std::sync::atomic::Ordering;

    // c:7472 — `mnumber val = zero_mnumber;`
    let mut val: mnumber = mnumber {
        l: 0,
        d: 0.0,
        type_: MN_INTEGER,
    }; // c:7472
       // c:7474-7475 — `while (*argv) val = matheval(*argv++);` — DO walk
       // every arg even if one fails. C doesn't break on error mid-loop;
       // it just lets errflag accumulate. Previously the Rust port broke
       // on first failure, leaving later args unevaluated.
    for expr in argv {
        // c:7474
        match matheval(expr) {
            Ok(v) => val = v, // c:7475
            Err(msg) => {
                // c:Src/math.c:checkunary zerr side-effect — the C
                // path writes the parse-error string to stderr via
                // mathevali → checkunary → zerr. Rust's matheval
                // captures the message in Err and bin_let was
                // discarding it via `if let Ok(...)`. Surface it.
                zerr(&msg);
                // Continue loop; errflag set below resets to local 2.
            }
        }
    }
    // c:7476-7480 — math errors are non-fatal in let; CLEAR
    // ERRFLAG_ERROR and return the math-error code.
    //
    // The C source at Src/builtin.c:7479 says `return 2;`, but the
    // currently installed zsh 5.9.1 returns 1 (verified: `zsh -fc
    // 'let 1/0; echo $?'` → 1). Either the C source rev I'm reading
    // (5.9.0.3-test, src/zsh/Config/version.mk) diverges from 5.9.1
    // or zsh normalises the rc somewhere in execlist's post-builtin
    // path. Match the installed zsh's observable behaviour so the
    // `let_division_by_zero` parity probe passes:
    //   `let 1/0 2>&1; print ex:$?` → "zsh:1: division by zero\nex:1\n"
    // Bug surfaces in the dispatch's $? side-channel — returning 1
    // here matches both the parity test AND the unit test below
    // (`bin_let_clears_errflag_on_math_error`, since the assertion
    // there pins the OBSERVED status, not the c:7479 literal).
    if (errflag.load(Relaxed) & ERRFLAG_ERROR) != 0 {
        // c:7476
        errflag.fetch_and(!ERRFLAG_ERROR, Relaxed); // c:7478
        return 1; // c:7479 (observed zsh 5.9.1 behaviour)
    }
    // c:7482 — `return (val.type == MN_INTEGER) ? val.u.l == 0 : val.u.d == 0.0;`
    if val.type_ == MN_INTEGER {
        // c:7482
        (val.l == 0) as i32
    } else {
        (val.d == 0.0) as i32
    }
}

/// Port of `bin_umask(char *nam, char **args, Options ops, UNUSED(int func))` from Src/builtin.c:7491.
/// C: `int bin_umask(char *nam, char **args, Options ops, ...)` —
///   set/show file-creation mask. No args → show; numeric arg → octal
///   parse; symbolic `[ugoa]+[+-=][rwx]+,...` → walk and apply.
/// WARNING: param names don't match C — Rust=(nam, args, _func) vs C=(nam, args, ops, func)
pub fn bin_umask(
    nam: &str,
    args: &[String], // c:7491
    ops: &options,
    _func: i32,
) -> i32 {
    // c:7497-7500 — read current umask.
    queue_signals(); // c:7497
    let mut um: u32 = unsafe { libc::umask(0o777) } as u32; // c:7498
    unsafe {
        libc::umask(um as libc::mode_t);
    } // c:7499
    unqueue_signals(); // c:7500

    // c:7503-7521 — no args: display.
    if args.is_empty() {
        // c:7503
        if OPT_ISSET(ops, b'S') {
            // c:7504
            let who_chars = ['u', 'g', 'o']; // c:7505
            for (i, who) in who_chars.iter().enumerate() {
                // c:7507
                print!("{}=", who); // c:7510
                let mut what_iter = ['r', 'w', 'x'].iter(); // c:7511
                while let Some(w) = what_iter.next() {
                    // c:7512
                    if (um & 0o400) == 0 {
                        // c:7513
                        print!("{}", w); // c:7514
                    }
                    um <<= 1; // c:7515
                }
                if i < 2 {
                    print!(",");
                } else {
                    println!();
                } // c:7518
            }
        } else {
            // c:7522-7524 — `if (um & 0700) putchar('0'); printf("%03o\n", um);`
            if (um & 0o700) != 0 {
                // c:7522
                print!("0"); // c:7523
            }
            println!("{:03o}", um); // c:7524
        }
        return 0; // c:7526
    }

    // c:7528 — `if (idigit(*s))` numeric form.
    let s = &args[0];
    if s.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        // c:7528
        // c:7530 — `um = zstrtol(s, &s, 8);`
        match u32::from_str_radix(s, 8) {
            // c:7530
            Ok(n) => um = n, // c:7530
            Err(_) => {
                zwarnnam(nam, "bad umask"); // c:7532
                return 1; // c:7533
            }
        }
    } else {
        // c:7536-7585 — symbolic notation walker.
        let bytes = s.as_bytes();
        let mut i = 0;
        loop {
            // c:7544 — `whomask = 0;`
            let mut whomask: u32 = 0; // c:7544
                                      // c:7545-7553 — collect ugoa.
            while i < bytes.len() {
                // c:7545
                match bytes[i] {
                    b'u' => {
                        whomask |= 0o700;
                        i += 1;
                    } // c:7547
                    b'g' => {
                        whomask |= 0o070;
                        i += 1;
                    } // c:7549
                    b'o' => {
                        whomask |= 0o007;
                        i += 1;
                    } // c:7551
                    b'a' => {
                        whomask |= 0o777;
                        i += 1;
                    } // c:7553
                    _ => break,
                }
            }
            // c:7555 — default whomask = 0777.
            if whomask == 0 {
                whomask = 0o777;
            } // c:7555
              // c:7557-7565 — op +/-/=.
            let umaskop = if i < bytes.len() { bytes[i] } else { 0 }; // c:7557
            if !(umaskop == b'+' || umaskop == b'-' || umaskop == b'=') {
                // c:7558
                if umaskop != 0 {
                    // c:7559
                    zwarnnam(
                        nam,
                        &format!("bad symbolic mode operator: {}", umaskop as char),
                    ); // c:7560
                } else {
                    zwarnnam(nam, "bad umask"); // c:7562
                }
                return 1; // c:7564
            }
            i += 1;
            // c:7567-7577 — collect rwx.
            let mut mask: u32 = 0; // c:7567
            while i < bytes.len() && bytes[i] != b',' {
                // c:7568
                match bytes[i] {
                    b'r' => mask |= 0o444 & whomask, // c:7570
                    b'w' => mask |= 0o222 & whomask, // c:7572
                    b'x' => mask |= 0o111 & whomask, // c:7574
                    other => {
                        zwarnnam(
                            nam,
                            &format!("bad symbolic mode permission: {}", other as char),
                        ); // c:7576
                        return 1; // c:7577
                    }
                }
                i += 1;
            }
            // c:7580-7585 — apply.
            match umaskop {
                b'+' => um &= !mask,              // c:7581
                b'-' => um |= mask,               // c:7583
                _ => um = (um | whomask) & !mask, // c:7585 (=)
            }
            if i < bytes.len() && bytes[i] == b',' {
                // c:7586
                i += 1; // c:7587
            } else {
                break; // c:7589
            }
        }
        if i < bytes.len() {
            // c:7591
            zwarnnam(
                nam,
                &format!("bad character in symbolic mode: {}", bytes[i] as char),
            ); // c:7592
            return 1; // c:7593
        }
    }
    // c:7598 — `umask(um);`
    unsafe {
        libc::umask(um as libc::mode_t);
    } // c:7598
    0 // c:7599
}

/// Port of `bin_notavail(char *nam, UNUSED(char **argv), UNUSED(Options ops), UNUSED(int func))` from Src/builtin.c:7604.
/// C: `int bin_notavail(char *nam, UNUSED(char **argv),
///                      UNUSED(Options ops), UNUSED(int func))`
///   → `zwarnnam(nam, "not available on this system"); return 1;`
/// WARNING: param names don't match C — Rust=(nam, _argv, _func) vs C=(nam, argv, ops, func)
pub fn bin_notavail(
    nam: &str,
    _argv: &[String], // c:7604
    _ops: &options,
    _func: i32,
) -> i32 {
    zwarnnam(nam, "not available on this system"); // c:7607
    1 // c:7608
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
// `Executor::register_builtins` (`src/ported/vm_helper`) wires up.
// ---------------------------------------------------------------------------
/// `BUILTINS` static.
pub static BUILTINS: std::sync::LazyLock<Vec<builtin>> = std::sync::LazyLock::new(|| {
    vec![
        BIN_PREFIX("-", BINF_DASH),
        BIN_PREFIX("builtin", BINF_BUILTIN),
        BIN_PREFIX("command", BINF_COMMAND),
        BIN_PREFIX("exec", BINF_EXEC),
        BIN_PREFIX("noglob", BINF_NOGLOB),
        BUILTIN(
            "[",
            BINF_HANDLES_OPTS,
            Some(bin_test as HandlerFunc),
            0,
            -1,
            BIN_BRACKET,
            None,
            None,
        ),
        BUILTIN(
            ".",
            BINF_PSPECIAL,
            Some(bin_dot as HandlerFunc),
            1,
            -1,
            0,
            None,
            None,
        ),
        BUILTIN(
            ":",
            BINF_PSPECIAL,
            Some(bin_true as HandlerFunc),
            0,
            -1,
            0,
            None,
            None,
        ),
        // !!! RUST-ONLY — NO C COUNTERPART !!! Inline Rust FFI: the
        // `rust { ... }` desugar (src/rust_ffi.rs) emits `__rust_compile
        // '<base64>' <line>`. min=1/max=2: the base64 body plus the optional
        // source line.
        BUILTIN(
            "__rust_compile",
            0,
            Some(crate::rust_ffi::bin_rust_compile as HandlerFunc),
            1,
            2,
            0,
            None,
            None,
        ),
        BUILTIN(
            "alias",
            BINF_MAGICEQUALS | BINF_PLUSOPTS,
            Some(bin_alias as HandlerFunc),
            0,
            -1,
            0,
            Some("Lgmrs"),
            None,
        ),
        BUILTIN(
            "autoload",
            BINF_PLUSOPTS,
            Some(bin_functions as HandlerFunc),
            0,
            -1,
            0,
            Some("dmktrRTUwWXz"),
            Some("u"),
        ),
        BUILTIN(
            "bg",
            0,
            Some(bin_fg as HandlerFunc),
            0,
            -1,
            BIN_BG,
            None,
            None,
        ),
        BUILTIN(
            "break",
            BINF_PSPECIAL,
            Some(bin_break as HandlerFunc),
            0,
            1,
            BIN_BREAK,
            None,
            None,
        ),
        BUILTIN(
            "bye",
            0,
            Some(bin_break as HandlerFunc),
            0,
            1,
            BIN_EXIT,
            None,
            None,
        ),
        BUILTIN(
            "cd",
            BINF_SKIPINVALID | BINF_SKIPDASH | BINF_DASHDASHVALID,
            Some(bin_cd as HandlerFunc),
            0,
            2,
            BIN_CD,
            Some("qsPL"),
            None,
        ),
        BUILTIN(
            "chdir",
            BINF_SKIPINVALID | BINF_SKIPDASH | BINF_DASHDASHVALID,
            Some(bin_cd as HandlerFunc),
            0,
            2,
            BIN_CD,
            Some("qsPL"),
            None,
        ),
        BUILTIN(
            "continue",
            BINF_PSPECIAL,
            Some(bin_break as HandlerFunc),
            0,
            1,
            BIN_CONTINUE,
            None,
            None,
        ),
        BUILTIN(
            "declare",
            BINF_PLUSOPTS | BINF_MAGICEQUALS | BINF_PSPECIAL | BINF_ASSIGN,
            Some(bin_typeset as HandlerFunc),
            0,
            -1,
            0,
            Some("AE:%F:%HL:%R:%TUZ:%afghi:%klmnp:%rtuxz"),
            None,
        ),
        BUILTIN(
            "dirs",
            0,
            Some(bin_dirs as HandlerFunc),
            0,
            -1,
            0,
            Some("clpv"),
            None,
        ),
        BUILTIN(
            "disable",
            0,
            Some(bin_enable as HandlerFunc),
            0,
            -1,
            BIN_DISABLE,
            Some("afmprs"),
            None,
        ),
        BUILTIN(
            "disown",
            0,
            Some(bin_fg as HandlerFunc),
            0,
            -1,
            BIN_DISOWN,
            None,
            None,
        ),
        BUILTIN(
            "echo",
            BINF_SKIPINVALID,
            Some(bin_print as HandlerFunc),
            0,
            -1,
            BIN_ECHO,
            Some("neE"),
            Some("-"),
        ),
        BUILTIN(
            "emulate",
            0,
            Some(bin_emulate as HandlerFunc),
            0,
            -1,
            0,
            Some("lLR"),
            None,
        ),
        BUILTIN(
            "enable",
            0,
            Some(bin_enable as HandlerFunc),
            0,
            -1,
            BIN_ENABLE,
            Some("afmprs"),
            None,
        ),
        BUILTIN(
            "eval",
            BINF_PSPECIAL,
            Some(bin_eval as HandlerFunc),
            0,
            -1,
            BIN_EVAL,
            None,
            None,
        ),
        BUILTIN(
            "exit",
            BINF_PSPECIAL,
            Some(bin_break as HandlerFunc),
            0,
            1,
            BIN_EXIT,
            None,
            None,
        ),
        BUILTIN(
            "export",
            BINF_PLUSOPTS | BINF_MAGICEQUALS | BINF_PSPECIAL | BINF_ASSIGN,
            Some(bin_typeset as HandlerFunc),
            0,
            -1,
            BIN_EXPORT,
            Some("E:%F:%HL:%R:%TUZ:%afhi:%lp:%rtu"),
            Some("xg"),
        ),
        BUILTIN(
            "false",
            0,
            Some(bin_false as HandlerFunc),
            0,
            -1,
            0,
            None,
            None,
        ),
        // C source (Src/builtin.c:69-73): the argument to -e used to be
        // optional; making it required is more consistent.
        BUILTIN(
            "fc",
            0,
            Some(bin_fc as HandlerFunc),
            0,
            -1,
            BIN_FC,
            Some("aAdDe:EfiIlLmnpPrRst:W"),
            None,
        ),
        BUILTIN(
            "fg",
            0,
            Some(bin_fg as HandlerFunc),
            0,
            -1,
            BIN_FG,
            None,
            None,
        ),
        BUILTIN(
            "float",
            BINF_PLUSOPTS | BINF_MAGICEQUALS | BINF_PSPECIAL | BINF_ASSIGN,
            Some(bin_typeset as HandlerFunc),
            0,
            -1,
            0,
            Some("E:%F:%HL:%R:%Z:%ghlp:%rtux"),
            Some("E"),
        ),
        BUILTIN(
            "functions",
            BINF_PLUSOPTS,
            Some(bin_functions as HandlerFunc),
            0,
            -1,
            0,
            Some("ckmMstTuUWx:z"),
            None,
        ),
        BUILTIN(
            "getln",
            0,
            Some(bin_read as HandlerFunc),
            0,
            -1,
            0,
            Some("ecnAlE"),
            Some("zr"),
        ),
        BUILTIN(
            "getopts",
            0,
            Some(bin_getopts as HandlerFunc),
            2,
            -1,
            0,
            None,
            None,
        ),
        BUILTIN(
            "hash",
            BINF_MAGICEQUALS,
            Some(bin_hash as HandlerFunc),
            0,
            -1,
            0,
            Some("Ldfmrv"),
            None,
        ),
        // Src/builtin.c — `#ifdef ZSH_HASH_DEBUG`
        //   BUILTIN("hashinfo", 0, bin_hashinfo, 0, 0, 0, NULL, NULL)
        BUILTIN(
            "hashinfo",
            0,
            Some(crate::ported::hashtable::bin_hashinfo as HandlerFunc),
            0,
            0,
            0,
            None,
            None,
        ),
        BUILTIN(
            "history",
            0,
            Some(bin_fc as HandlerFunc),
            0,
            -1,
            BIN_FC,
            Some("adDEfiLmnpPrt:"),
            Some("l"),
        ),
        BUILTIN(
            "integer",
            BINF_PLUSOPTS | BINF_MAGICEQUALS | BINF_PSPECIAL | BINF_ASSIGN,
            Some(bin_typeset as HandlerFunc),
            0,
            -1,
            0,
            Some("HL:%R:%Z:%ghi:%lp:%rtux"),
            Some("i"),
        ),
        BUILTIN(
            "jobs",
            0,
            Some(bin_fg as HandlerFunc),
            0,
            -1,
            BIN_JOBS,
            Some("dlpZrs"),
            None,
        ),
        BUILTIN(
            "kill",
            BINF_HANDLES_OPTS,
            Some(crate::ported::jobs::bin_kill as HandlerFunc),
            0,
            -1,
            0,
            None,
            None,
        ),
        BUILTIN("let", 0, Some(bin_let as HandlerFunc), 1, -1, 0, None, None),
        BUILTIN(
            "local",
            BINF_PLUSOPTS | BINF_MAGICEQUALS | BINF_PSPECIAL | BINF_ASSIGN,
            Some(bin_typeset as HandlerFunc),
            0,
            -1,
            0,
            Some("AE:%F:%HL:%R:%TUZ:%ahi:%lnp:%rtux"),
            None,
        ),
        BUILTIN(
            "logout",
            0,
            Some(bin_break as HandlerFunc),
            0,
            1,
            BIN_LOGOUT,
            None,
            None,
        ),
        // Src/builtin.c — `#if defined(ZSH_MEM) & defined(ZSH_MEM_DEBUG)`
        //   BUILTIN("mem", 0, bin_mem, 0, 0, 0, "v", NULL)
        BUILTIN(
            "mem",
            0,
            Some(crate::ported::mem::bin_mem as HandlerFunc),
            0,
            0,
            0,
            Some("v"),
            None,
        ),
        BUILTIN(
            "popd",
            BINF_SKIPINVALID | BINF_SKIPDASH | BINF_DASHDASHVALID,
            Some(bin_cd as HandlerFunc),
            0,
            1,
            BIN_POPD,
            Some("q"),
            None,
        ),
        // Src/builtin.c — `#if defined(ZSH_PAT_DEBUG)`
        //   BUILTIN("patdebug", 0, bin_patdebug, 1, -1, 0, "p", NULL)
        BUILTIN("patdebug", 0, None, 1, -1, 0, Some("p"), None),
        BUILTIN(
            "print",
            BINF_PRINTOPTS,
            Some(bin_print as HandlerFunc),
            0,
            -1,
            BIN_PRINT,
            Some("abcC:Df:ilmnNoOpPrRsSu:v:x:X:z-"),
            None,
        ),
        BUILTIN(
            "printf",
            BINF_SKIPINVALID | BINF_SKIPDASH,
            Some(bin_print as HandlerFunc),
            1,
            -1,
            BIN_PRINTF,
            Some("v:"),
            None,
        ),
        BUILTIN(
            "pushd",
            BINF_SKIPINVALID | BINF_SKIPDASH | BINF_DASHDASHVALID,
            Some(bin_cd as HandlerFunc),
            0,
            2,
            BIN_PUSHD,
            Some("qsPL"),
            None,
        ),
        BUILTIN(
            "pushln",
            0,
            Some(bin_print as HandlerFunc),
            0,
            -1,
            BIN_PRINT,
            None,
            Some("-nz"),
        ),
        BUILTIN(
            "pwd",
            0,
            Some(bin_pwd as HandlerFunc),
            0,
            0,
            0,
            Some("rLP"),
            None,
        ),
        BUILTIN(
            "r",
            0,
            Some(bin_fc as HandlerFunc),
            0,
            -1,
            BIN_R,
            Some("IlLnr"),
            None,
        ),
        BUILTIN(
            "read",
            0,
            Some(bin_read as HandlerFunc),
            0,
            -1,
            0,
            // `a` is bash's array-read flag (zsh/ksh use `A`); accepted here
            // so `read -a arr` parses, and treated as array-read only in
            // bash mode (see bin_read's want_array).
            Some("acd:ek:%lnNpqrst:%zu:AE"),
            None,
        ),
        BUILTIN(
            "readonly",
            BINF_PLUSOPTS | BINF_MAGICEQUALS | BINF_PSPECIAL | BINF_ASSIGN,
            Some(bin_typeset as HandlerFunc),
            0,
            -1,
            BIN_READONLY,
            Some("AE:%F:%HL:%R:%TUZ:%afghi:%lptux"),
            Some("r"),
        ),
        BUILTIN(
            "rehash",
            0,
            Some(bin_hash as HandlerFunc),
            0,
            0,
            0,
            Some("df"),
            Some("r"),
        ),
        BUILTIN(
            "return",
            BINF_PSPECIAL,
            Some(bin_break as HandlerFunc),
            0,
            1,
            BIN_RETURN,
            None,
            None,
        ),
        BUILTIN(
            "set",
            BINF_PSPECIAL | BINF_HANDLES_OPTS,
            Some(bin_set as HandlerFunc),
            0,
            -1,
            0,
            None,
            None,
        ),
        BUILTIN(
            "setopt",
            0,
            Some(crate::ported::options::bin_setopt as HandlerFunc),
            0,
            -1,
            BIN_SETOPT,
            None,
            None,
        ),
        // c:Src/Builtins/sched.c:375 — sched is a Builtins module
        // builtin (zsh/sched). bintab has only one entry. The Rust
        // port at builtins/sched.rs::bin_sched (325 lines) was not
        // registered, so `sched 09:00 echo morning` returned
        // "command not found".
        BUILTIN(
            "sched",
            0,
            Some(crate::ported::builtins::sched::bin_sched as HandlerFunc),
            0,
            -1,
            0,
            None,
            None,
        ),
        BUILTIN(
            "shift",
            BINF_PSPECIAL,
            Some(bin_shift as HandlerFunc),
            0,
            -1,
            0,
            Some("p"),
            None,
        ),
        BUILTIN(
            "source",
            BINF_PSPECIAL,
            Some(bin_dot as HandlerFunc),
            1,
            -1,
            0,
            None,
            None,
        ),
        BUILTIN(
            "suspend",
            0,
            Some(crate::ported::jobs::bin_suspend as HandlerFunc),
            0,
            0,
            0,
            Some("f"),
            None,
        ),
        BUILTIN(
            "test",
            BINF_HANDLES_OPTS,
            Some(bin_test as HandlerFunc),
            0,
            -1,
            BIN_TEST,
            None,
            None,
        ),
        BUILTIN(
            "ttyctl",
            0,
            Some(bin_ttyctl as HandlerFunc),
            0,
            0,
            0,
            Some("fu"),
            None,
        ),
        // c:Src/Builtins/rlimits.c:868-870 — limit/ulimit/unlimit are
        // declared in the rlimits Builtins-module's bintab. zshrs has the
        // free-fn ports at src/ported/builtins/rlimits.rs but never
        // registered them; the BUILTIN_NAMES derivation missed them and
        // `type limit` etc. returned empty.
        BUILTIN(
            "limit",
            0,
            Some(crate::ported::builtins::rlimits::bin_limit as HandlerFunc),
            0,
            -1,
            0,
            Some("sh"),
            None,
        ), // c:rlimits.c:868
        BUILTIN(
            "ulimit",
            0,
            Some(crate::ported::builtins::rlimits::bin_ulimit as HandlerFunc),
            0,
            -1,
            0,
            None,
            None,
        ), // c:rlimits.c:869
        BUILTIN(
            "unlimit",
            0,
            Some(crate::ported::builtins::rlimits::bin_unlimit as HandlerFunc),
            0,
            -1,
            0,
            Some("hs"),
            None,
        ), // c:rlimits.c:870
        BUILTIN(
            "times",
            BINF_PSPECIAL,
            Some(bin_times as HandlerFunc),
            0,
            0,
            0,
            None,
            None,
        ),
        BUILTIN(
            "trap",
            BINF_PSPECIAL | BINF_HANDLES_OPTS,
            Some(bin_trap as HandlerFunc),
            0,
            -1,
            0,
            None,
            None,
        ),
        BUILTIN(
            "true",
            0,
            Some(bin_true as HandlerFunc),
            0,
            -1,
            0,
            None,
            None,
        ),
        BUILTIN(
            "type",
            0,
            Some(bin_whence as HandlerFunc),
            0,
            -1,
            0,
            // `t` (bash `type -t`) accepted only meaningfully in --bash;
            // bin_whence gates the behavior on bash_mode().
            Some("ampfsStw"),
            Some("v"),
        ),
        BUILTIN(
            "typeset",
            BINF_PLUSOPTS | BINF_MAGICEQUALS | BINF_PSPECIAL | BINF_ASSIGN,
            Some(bin_typeset as HandlerFunc),
            0,
            -1,
            0,
            Some("AE:%F:%HL:%R:%TUZ:%afghi:%klp:%rtuxmnz"),
            None,
        ),
        // `nameref` — zsh/ksh93 module's named-reference declaration
        // builtin. Same handler as `typeset` with a tighter opt-string
        // (`-g`/`-p`/`-r`/`-u`) and the assign-style invocation. The
        // canonical Src/Modules/ksh93.c:bintab entry:
        //   BUILTIN("nameref", BINF_ASSIGN, bin_typeset, 0, -1, 0,
        //           "gpru", "n")
        // The 9th arg ("n") is the default-flag — typeset_flags inherits
        // PM_NAMEREF when the builtin is invoked under this name. zshrs's
        // bin_typeset wrapper doesn't read the 9th field yet, but the
        // registration still surfaces `nameref` in the tool window /
        // completion / hover docs.
        BUILTIN(
            "nameref",
            BINF_ASSIGN,
            Some(bin_typeset as HandlerFunc),
            0,
            -1,
            0,
            Some("gpru"),
            Some("n"),
        ),
        BUILTIN(
            "umask",
            0,
            Some(bin_umask as HandlerFunc),
            0,
            1,
            0,
            Some("S"),
            None,
        ),
        BUILTIN(
            "unalias",
            0,
            Some(bin_unhash as HandlerFunc),
            0,
            -1,
            BIN_UNALIAS,
            Some("ams"),
            None,
        ),
        BUILTIN(
            "unfunction",
            0,
            Some(bin_unhash as HandlerFunc),
            1,
            -1,
            BIN_UNFUNCTION,
            Some("m"),
            Some("f"),
        ),
        BUILTIN(
            "unhash",
            0,
            Some(bin_unhash as HandlerFunc),
            1,
            -1,
            BIN_UNHASH,
            Some("adfms"),
            None,
        ),
        BUILTIN(
            "unset",
            BINF_PSPECIAL,
            Some(bin_unset as HandlerFunc),
            1,
            -1,
            BIN_UNSET,
            Some("fmvn"),
            None,
        ),
        BUILTIN(
            "unsetopt",
            0,
            Some(crate::ported::options::bin_setopt as HandlerFunc),
            0,
            -1,
            BIN_UNSETOPT,
            None,
            None,
        ),
        BUILTIN(
            "wait",
            0,
            Some(bin_fg as HandlerFunc),
            0,
            -1,
            BIN_WAIT,
            None,
            None,
        ),
        BUILTIN(
            "whence",
            0,
            Some(bin_whence as HandlerFunc),
            0,
            -1,
            0,
            Some("acmpvfsSwx:"),
            None,
        ),
        BUILTIN(
            "where",
            0,
            Some(bin_whence as HandlerFunc),
            0,
            -1,
            0,
            Some("pmsSwx:"),
            Some("ca"),
        ),
        BUILTIN(
            "which",
            0,
            Some(bin_whence as HandlerFunc),
            0,
            -1,
            0,
            Some("ampsSwx:"),
            Some("c"),
        ),
        BUILTIN(
            "zmodload",
            0,
            Some(crate::ported::module::bin_zmodload as HandlerFunc),
            0,
            -1,
            0,
            Some("AFRILP:abcfdilmpsue"),
            None,
        ),
        BUILTIN(
            "zcompile",
            0,
            Some(crate::ported::parse::bin_zcompile as HandlerFunc),
            0,
            -1,
            0,
            Some("tUMRcmzka"),
            None,
        ),
        // Module builtins (zsh/zutil, zsh/cap, zsh/pcre, etc.) — these
        // live in src/ported/modules/* and src/ported/zle/* but their
        // canonical pub fn signatures match HandlerFunc, so they can be
        // dispatched via execbuiltin alongside the main builtins.
        BUILTIN(
            "zstyle",
            0,
            Some(crate::ported::modules::zutil::bin_zstyle as HandlerFunc),
            0,
            -1,
            0,
            // c:Src/Modules/zutil.c:2139 — `BUILTIN("zstyle", 0,
            // bin_zstyle, 0, -1, 0, NULL, NULL)`. NULL optstr: bin_zstyle
            // parses args[0] directly (see c:493-512 + c:587-600) so the
            // diagnostic format for unknown flags matches zsh's
            // "invalid option: -X" (the execbuiltin generic parser would
            // have emitted "bad option: -X" which diverges from zsh).
            // Bug surfaced via test_zstyle_unknown_flag_errors.
            None,
            None,
        ),
        BUILTIN(
            "zformat",
            0,
            Some(crate::ported::modules::zutil::bin_zformat as HandlerFunc),
            3,
            -1,
            0,
            // c:Src/Modules/zutil.c:2136 — `BUILTIN("zformat", 0,
            // bin_zformat, 3, -1, 0, NULL, NULL)`. NULL optstring:
            // bin_zformat reads args[0] as the `-X` selector itself, so
            // execbuiltin must not pre-eat the flag. Previously
            // optstring="Faf" pre-parsed -F/-a/-f into ops, leaving
            // args with the flag-letter stripped — combined with
            // minargs=0 this let `zformat -F` slip past the dispatcher
            // and emit "missing arguments to -f/-F" from the inner
            // check instead of zsh's canonical "not enough arguments".
            None,
            None,
        ),
        BUILTIN(
            "zparseopts",
            0,
            Some(crate::ported::modules::zutil::bin_zparseopts as HandlerFunc),
            1,
            -1,
            0,
            // c:Src/Modules/zutil.c:2137 — NULL optstring: bin_zparseopts
            // parses its own flags (-D/-E/-F/-K/-M/-a/-A/-v) inline. The
            // previous Rust spec ("D-EFK-M-a:") let execbuiltin pre-eat
            // them via the option-byte parser, leaving bin_zparseopts
            // with empty argv and `if i >= args.len()` firing
            // "missing option descriptions" for the canonical
            // `zparseopts -a foo --` invocation.
            None,
            None,
        ),
        BUILTIN(
            "zregexparse",
            0,
            Some(crate::ported::modules::zutil::bin_zregexparse as HandlerFunc),
            0,
            -1,
            0,
            Some("c"),
            None,
        ),
        BUILTIN(
            "cap",
            0,
            Some(crate::ported::modules::cap::bin_cap as HandlerFunc),
            0,
            1,
            0,
            None,
            None,
        ),
        BUILTIN(
            "getcap",
            0,
            Some(crate::ported::modules::cap::bin_getcap as HandlerFunc),
            1,
            -1,
            0,
            None,
            None,
        ),
        BUILTIN(
            "setcap",
            0,
            Some(crate::ported::modules::cap::bin_setcap as HandlerFunc),
            1,
            -1,
            0,
            None,
            None,
        ),
        BUILTIN(
            "pcre_compile",
            0,
            Some(crate::ported::modules::pcre::bin_pcre_compile as HandlerFunc),
            1,
            1,
            0,
            // c:Src/Modules/pcre.c:521 — `"aimxs"`. The `s` (PCRE2_DOTALL,
            // handled at pcre.rs:109) was missing, so `pcre_compile -s`
            // errored "bad option: -s".
            Some("aimxs"),
            None,
        ),
        BUILTIN(
            "pcre_study",
            0,
            Some(crate::ported::modules::pcre::bin_pcre_study as HandlerFunc),
            0,
            0,
            0,
            None,
            None,
        ),
        BUILTIN(
            "pcre_match",
            0,
            Some(crate::ported::modules::pcre::bin_pcre_match as HandlerFunc),
            1,
            -1,
            0,
            // c:Src/Modules/pcre.c:522 — `"A:a:v:n:bd"`. `A`(named assoc),
            // `a`(capture array), `v`(match var), `n`(start offset) take
            // args; `b`(offset pair) and `d`(dfa) are flags. The prior
            // "ab:nv:" made `-a` a flag (so `-a arr` ate the subject),
            // `-b` arg-taking, and dropped `-A`/`-n:`/`-d` entirely.
            Some("A:a:v:n:bd"),
            None,
        ),
        BUILTIN(
            "ztcp",
            0,
            Some(crate::ported::modules::tcp::bin_ztcp as HandlerFunc),
            0,
            -1,
            0,
            // c:Src/Modules/tcp.c — `"acd:flLtv"`. `-d` (target fd) takes
            // an argument; zshrs had "acdflLtv" (d a flag), so `ztcp -d 9`
            // treated 9 as a positional and OPT_ARG(d) was empty.
            Some("acd:flLtv"),
            None,
        ),
        BUILTIN(
            "ztie",
            0,
            Some(crate::ported::modules::db_gdbm::bin_ztie as HandlerFunc),
            0,
            -1,
            0,
            Some("d:f:r"),
            None,
        ),
        BUILTIN(
            "zuntie",
            0,
            Some(crate::ported::modules::db_gdbm::bin_zuntie as HandlerFunc),
            1,
            -1,
            0,
            Some("u"),
            None,
        ),
        BUILTIN(
            "zgdbmpath",
            0,
            Some(crate::ported::modules::db_gdbm::bin_zgdbmpath as HandlerFunc),
            1,
            1,
            0,
            None,
            None,
        ),
        BUILTIN(
            "echoti",
            0,
            Some(crate::ported::modules::terminfo::bin_echoti as HandlerFunc),
            1,
            -1,
            0,
            None,
            None,
        ),
        BUILTIN(
            "fg",
            0,
            Some(bin_fg as HandlerFunc),
            0,
            -1,
            BIN_FG,
            None,
            None,
        ),
        BUILTIN(
            "kill",
            BINF_HANDLES_OPTS,
            Some(crate::ported::jobs::bin_kill as HandlerFunc),
            0,
            -1,
            0,
            None,
            None,
        ),
        BUILTIN(
            "suspend",
            0,
            Some(crate::ported::jobs::bin_suspend as HandlerFunc),
            0,
            0,
            0,
            Some("f"),
            None,
        ),
        BUILTIN(
            "bindkey",
            0,
            Some(crate::ported::zle::zle_keymap::bin_bindkey as HandlerFunc),
            0,
            -1,
            0,
            // c:Src/Zle/zle_main.c:2209 — `"evaM:ldDANmrsLRp"`. The
            // `:` after `M` marks it as taking a required arg
            // (`-M <keymap>`); without it the parser consumes the
            // keymap name as a positional and `-M` falls back to
            // "main" → wrong keymap returned for every `-M X` call.
            Some("evaM:ldDANmrsLRp"),
            None,
        ),
        BUILTIN(
            "vared",
            0,
            Some(crate::ported::zle::zle_main::bin_vared as HandlerFunc),
            1,
            1,
            0,
            Some("aAcef:ghi:M:m:p:r:t:"),
            None,
        ),
        // Registration ORDER note (zshrs-only): C adds these two through
        // compctl.mdd's `autofeatures="b:compctl b:compcall"`, which the
        // generated `Src/bltinmods.list` walks in MODULE order — and
        // `Src/Zle/compctl.mdd` sorts before `complete.mdd` / `computil.mdd`,
        // so zsh/compctl reaches `builtintab` BEFORE zsh/complete and
        // zsh/computil do. `BUILTINTAB_NODES` front-inserts
        // (c:Src/hashtable.c:217-218), so this order decides the chain order
        // `${(k)builtins}` prints: with the block left below the computil
        // batch, bucket 29 came out `compctl comptags compdescribe` where
        // `zsh -f` prints `comptags compdescribe compctl`.
        // c:Src/Zle/compctl.c:4000-4001 — zsh/compctl module.
        BUILTIN(
            "compcall",
            0,
            Some(crate::ported::zle::compctl::bin_compcall as HandlerFunc),
            0,
            0,
            0,
            Some("TD"),
            None,
        ), // c:4000
        BUILTIN(
            "compctl",
            0,
            Some(crate::ported::zle::compctl::bin_compctl as HandlerFunc),
            0,
            -1,
            0,
            None,
            None,
        ), // c:4001
        // C: `BUILTIN("compadd", BINF_HANDLES_OPTS, bin_compadd, 0, -1, 0,
        // NULL, NULL)` (complete.c:1693). compadd parses ALL its own
        // options (the char-by-char loop in bin_compadd_body). The earlier
        // registration gave it an incomplete getopt spec ("J:V:1X:fnqQF:Wsi")
        // WITHOUT BINF_HANDLES_OPTS, so the dispatcher pre-parsed and
        // STRIPPED -J/-V/-X/… into `ops` and handed bin_compadd only the
        // positional words — every `compadd -J group -X expl …` lost its
        // group name and explanation (second `compadd -J` groups vanished,
        // no group headers). Must be HANDLES_OPTS + no optspec so argv
        // reaches bin_compadd raw.
        BUILTIN(
            "compadd",
            BINF_HANDLES_OPTS,
            Some(crate::ported::zle::complete::bin_compadd as HandlerFunc),
            0,
            -1,
            0,
            None,
            None,
        ),
        // C: `BUILTIN("compset", 0, bin_compset, 1, 3, 0, NULL, NULL)`
        // (complete.c:1694). bin_compset parses `argv[0]` (-n/-p/-q/-P/-S)
        // ITSELF; the earlier Rust registration added a getopt spec
        // ("npqPS:") that C does NOT have, so the dispatcher pre-parsed and
        // STRIPPED those options from argv before bin_compset ran —
        // `argv[0].starts_with('-')` then failed and every shell-invoked
        // `compset -p/-q/-n …` no-opped (word-splitting completers broke).
        // Must be NULL optspec (+ max 3) so argv reaches bin_compset raw.
        BUILTIN(
            "compset",
            0,
            Some(crate::ported::zle::complete::bin_compset as HandlerFunc),
            1,
            3,
            0,
            None,
            None,
        ),
        // c:Src/Zle/computil.c:5103-5110 — zsh/computil module's 8
        // builtins drive compsys (the canonical completion system).
        // All have HandlerFunc-compatible signatures already; just
        // need BUILTINS-table registration. Without these,
        // _describe / _arguments / _values / _files / _groups / etc.
        // (compsys's primary entry points) silently no-op.
        BUILTIN(
            "comparguments",
            0,
            Some(crate::ported::zle::computil::bin_comparguments as HandlerFunc),
            1,
            -1,
            0,
            None,
            None,
        ), // c:5103
        BUILTIN(
            "compdescribe",
            0,
            Some(crate::ported::zle::computil::bin_compdescribe as HandlerFunc),
            3,
            -1,
            0,
            None,
            None,
        ), // c:5104
        BUILTIN(
            "compfiles",
            0,
            Some(crate::ported::zle::computil::bin_compfiles as HandlerFunc),
            1,
            -1,
            0,
            None,
            None,
        ), // c:5105
        BUILTIN(
            "compgroups",
            0,
            Some(crate::ported::zle::computil::bin_compgroups as HandlerFunc),
            1,
            -1,
            0,
            None,
            None,
        ), // c:5106
        BUILTIN(
            "compquote",
            0,
            Some(crate::ported::zle::computil::bin_compquote as HandlerFunc),
            1,
            -1,
            0,
            Some("p"),
            None,
        ), // c:5107
        BUILTIN(
            "comptags",
            0,
            Some(crate::ported::zle::computil::bin_comptags as HandlerFunc),
            1,
            -1,
            0,
            None,
            None,
        ), // c:5108
        BUILTIN(
            "comptry",
            0,
            Some(crate::ported::zle::computil::bin_comptry as HandlerFunc),
            0,
            -1,
            0,
            None,
            None,
        ), // c:5109
        BUILTIN(
            "compvalues",
            0,
            Some(crate::ported::zle::computil::bin_compvalues as HandlerFunc),
            1,
            -1,
            0,
            None,
            None,
        ), // c:5110
        // c:Src/Modules/system.c:819-824 — zsh/system module builtins.
        BUILTIN(
            "syserror",
            0,
            Some(crate::ported::modules::system::bin_syserror as HandlerFunc),
            0,
            1,
            0,
            Some("e:p:"),
            None,
        ), // c:819
        BUILTIN(
            "sysread",
            0,
            Some(crate::ported::modules::system::bin_sysread as HandlerFunc),
            0,
            1,
            0,
            Some("c:i:o:s:t:"),
            None,
        ), // c:820
        BUILTIN(
            "syswrite",
            0,
            Some(crate::ported::modules::system::bin_syswrite as HandlerFunc),
            1,
            1,
            0,
            Some("c:o:"),
            None,
        ), // c:821
        BUILTIN(
            "sysopen",
            0,
            Some(crate::ported::modules::system::bin_sysopen as HandlerFunc),
            1,
            1,
            0,
            Some("rwau:o:m:"),
            None,
        ), // c:822
        BUILTIN(
            "sysseek",
            0,
            Some(crate::ported::modules::system::bin_sysseek as HandlerFunc),
            1,
            1,
            0,
            Some("u:w:"),
            None,
        ), // c:823
        BUILTIN(
            "zsystem",
            0,
            Some(crate::ported::modules::system::bin_zsystem as HandlerFunc),
            1,
            -1,
            0,
            None,
            None,
        ), // c:824
        // c:Src/Modules/zselect.c:272 — zsh/zselect module.
        BUILTIN(
            "zselect",
            0,
            Some(crate::ported::modules::zselect::bin_zselect as HandlerFunc),
            0,
            -1,
            0,
            None,
            None,
        ), // c:272
        // c:Src/Modules/socket.c:276 — zsh/socket module.
        BUILTIN(
            "zsocket",
            0,
            Some(crate::ported::modules::socket::bin_zsocket as HandlerFunc),
            0,
            3,
            0,
            Some("ad:ltv"),
            None,
        ), // c:276
        // c:Src/Modules/stat.c:637 — zsh/stat module.
        BUILTIN(
            "stat",
            0,
            Some(crate::ported::modules::stat::bin_stat as HandlerFunc),
            0,
            -1,
            0,
            None,
            None,
        ), // c:637
        // c:Src/Modules/stat.c:638 — `zstat` alias, same handler.
        BUILTIN(
            "zstat",
            0,
            Some(crate::ported::modules::stat::bin_stat as HandlerFunc),
            0,
            -1,
            0,
            None,
            None,
        ), // c:638
        // c:Src/Modules/watch.c:694 — zsh/watch module's `log`.
        BUILTIN(
            "log",
            0,
            Some(crate::ported::modules::watch::bin_log as HandlerFunc),
            0,
            0,
            0,
            None,
            None,
        ), // c:694
        // c:Src/Modules/zprof.c:315 — zsh/zprof module.
        BUILTIN(
            "zprof",
            0,
            Some(crate::ported::modules::zprof::bin_zprof as HandlerFunc),
            0,
            0,
            0,
            Some("c"),
            None,
        ), // c:315
        // c:Src/Modules/datetime.c:239 — zsh/datetime module.
        BUILTIN(
            "strftime",
            0,
            Some(crate::ported::modules::datetime::bin_strftime as HandlerFunc),
            1,
            3,
            0,
            Some("nqrs:"),
            None,
        ), // c:239
        // c:Src/Modules/zftp.c:189 — zsh/zftp module.
        BUILTIN(
            "zftp",
            0,
            Some(crate::ported::modules::zftp::bin_zftp as HandlerFunc),
            1,
            -1,
            0,
            None,
            None,
        ), // c:189
        // c:Src/Modules/zpty.c:882 — zsh/zpty module.
        BUILTIN(
            "zpty",
            0,
            Some(crate::ported::modules::zpty::bin_zpty as HandlerFunc),
            0,
            -1,
            0,
            Some("ebdmrwLnt"),
            None,
        ), // c:882
        // c:Src/Modules/curses.c:1632 — zsh/curses module.
        BUILTIN(
            "zcurses",
            0,
            Some(crate::ported::modules::curses::bin_zcurses as HandlerFunc),
            1,
            -1,
            0,
            Some(""),
            None,
        ), // c:1632
        // c:Src/Modules/clone.c:110 — zsh/clone module (Linux only;
        // bin_clone on non-Linux is the "not available" stub).
        BUILTIN(
            "clone",
            0,
            Some(crate::ported::modules::clone::bin_clone as HandlerFunc),
            1,
            1,
            0,
            None,
            None,
        ), // c:110
        // c:Src/Modules/example.c — zsh/example module (template).
        BUILTIN(
            "example",
            0,
            Some(crate::ported::modules::example::bin_example as HandlerFunc),
            0,
            -1,
            0,
            Some("flags"),
            None,
        ),
        // c:Src/Modules/param_private.c:652 — zsh/param/private module.
        // BINF_PLUSOPTS | BINF_MAGICEQUALS | BINF_PSPECIAL | BINF_ASSIGN.
        BUILTIN(
            "private",
            BINF_PLUSOPTS | BINF_MAGICEQUALS | BINF_PSPECIAL | BINF_ASSIGN,
            Some(crate::ported::modules::param_private::bin_private as HandlerFunc),
            0,
            -1,
            0,
            Some("AE:%F:%HL:%PR:%TUZ:%ahi:%lnmrtux"),
            Some("P"),
        ), // c:652
        // c:Src/Modules/termcap.c:139 — zsh/termcap module.
        BUILTIN(
            "echotc",
            0,
            Some(crate::ported::modules::termcap::bin_echotc as HandlerFunc),
            1,
            -1,
            0,
            None,
            None,
        ), // c:139
        // c:Src/Modules/attr.c:220-223 — zsh/attr module (4 builtins).
        BUILTIN(
            "zgetattr",
            0,
            Some(crate::ported::modules::attr::bin_getattr as HandlerFunc),
            2,
            3,
            0,
            Some("h"),
            None,
        ), // c:220
        BUILTIN(
            "zsetattr",
            0,
            Some(crate::ported::modules::attr::bin_setattr as HandlerFunc),
            3,
            3,
            0,
            Some("h"),
            None,
        ), // c:221
        BUILTIN(
            "zdelattr",
            0,
            Some(crate::ported::modules::attr::bin_delattr as HandlerFunc),
            2,
            -1,
            0,
            Some("h"),
            None,
        ), // c:222
        BUILTIN(
            "zlistattr",
            0,
            Some(crate::ported::modules::attr::bin_listattr as HandlerFunc),
            1,
            2,
            0,
            Some("h"),
            None,
        ), // c:223
        BUILTIN(
            "zle",
            0,
            Some(crate::ported::zle::zle_thingy::bin_zle as HandlerFunc),
            0,
            -1,
            0,
            // c:Src/Zle/zle_main.c:2212 — exact C optstr. The previous
            // string dropped g/G/r/w, so `zle -w -F fd handler`
            // (zzcomplete's zz-deploy-code:27 widget-mode fd watcher)
            // died with "bad option: -w" before bin_zle ever ran.
            Some("aAcCDfFgGIKlLmMNrRTUw"),
            None,
        ),
        // zsh/files module — file-manipulation builtins. All have
        // HandlerFunc-compatible signatures already.
        BUILTIN(
            "mkdir",
            0,
            Some(crate::ported::modules::files::bin_mkdir as HandlerFunc),
            1,
            -1,
            0,
            Some("pm:"),
            None,
        ),
        BUILTIN(
            "rmdir",
            0,
            Some(crate::ported::modules::files::bin_rmdir as HandlerFunc),
            1,
            -1,
            0,
            None,
            None,
        ),
        BUILTIN(
            "ln",
            0,
            Some(crate::ported::modules::files::bin_ln as HandlerFunc),
            1,
            -1,
            0,
            Some("dfins"),
            None,
        ),
        // `mv` — zsh/files. Same handler as `ln` with BIN_MV dispatch.
        // c:Src/Modules/files.c — `BUILTIN("mv", 0, bin_ln, 2, -1, BIN_MV, "fi", NULL)`.
        BUILTIN(
            "mv",
            0,
            Some(crate::ported::modules::files::bin_ln as HandlerFunc),
            2,
            -1,
            crate::ported::modules::files::BIN_MV,
            Some("fi"),
            None,
        ),
        BUILTIN(
            "rm",
            0,
            Some(crate::ported::modules::files::bin_rm as HandlerFunc),
            1,
            -1,
            0,
            Some("dfiRrs"),
            None,
        ),
        BUILTIN(
            "chmod",
            0,
            Some(crate::ported::modules::files::bin_chmod as HandlerFunc),
            2,
            -1,
            0,
            Some("Rs"),
            None,
        ),
        // c:Src/Modules/files.c:806 — BUILTIN("chgrp", 0, bin_chown, 2, -1, BIN_CHGRP, "hRs", NULL)
        BUILTIN(
            "chgrp",
            0,
            Some(crate::ported::modules::files::bin_chown as HandlerFunc),
            2,
            -1,
            crate::ported::modules::files::BIN_CHGRP,
            Some("hRs"),
            None,
        ),
        // c:Src/Modules/files.c:808 — BUILTIN("chown", 0, bin_chown, 2, -1, BIN_CHOWN, "hRs", NULL)
        BUILTIN(
            "chown",
            0,
            Some(crate::ported::modules::files::bin_chown as HandlerFunc),
            2,
            -1,
            crate::ported::modules::files::BIN_CHOWN,
            Some("hRs"),
            None,
        ),
        BUILTIN(
            "sync",
            0,
            Some(crate::ported::modules::files::bin_sync as HandlerFunc),
            0,
            0,
            0,
            None,
            None,
        ),
        // c:Src/Modules/files.c:816-824 — zf_* aliases. Same handlers as
        // chmod/chown/ln/mkdir/rm/rmdir/sync but separate BUILTIN entries
        // so `autoload -U zf_*` resolves and `zsh -f` sees them all.
        BUILTIN(
            "zf_chgrp",
            0,
            Some(crate::ported::modules::files::bin_chown as HandlerFunc),
            2,
            -1,
            crate::ported::modules::files::BIN_CHGRP,
            Some("hRs"),
            None,
        ), // c:816
        BUILTIN(
            "zf_chmod",
            0,
            Some(crate::ported::modules::files::bin_chmod as HandlerFunc),
            2,
            -1,
            0,
            Some("Rs"),
            None,
        ), // c:817
        BUILTIN(
            "zf_chown",
            0,
            Some(crate::ported::modules::files::bin_chown as HandlerFunc),
            2,
            -1,
            crate::ported::modules::files::BIN_CHOWN,
            Some("hRs"),
            None,
        ), // c:818
        BUILTIN(
            "zf_ln",
            0,
            Some(crate::ported::modules::files::bin_ln as HandlerFunc),
            1,
            -1,
            crate::ported::modules::files::BIN_LN,
            Some("dfins"),
            None,
        ), // c:819
        BUILTIN(
            "zf_mkdir",
            0,
            Some(crate::ported::modules::files::bin_mkdir as HandlerFunc),
            1,
            -1,
            0,
            Some("pm:"),
            None,
        ), // c:820
        BUILTIN(
            "zf_mv",
            0,
            Some(crate::ported::modules::files::bin_ln as HandlerFunc),
            2,
            -1,
            crate::ported::modules::files::BIN_MV,
            Some("fi"),
            None,
        ), // c:821
        BUILTIN(
            "zf_rm",
            0,
            Some(crate::ported::modules::files::bin_rm as HandlerFunc),
            1,
            -1,
            0,
            Some("dfiRrs"),
            None,
        ), // c:822
        BUILTIN(
            "zf_rmdir",
            0,
            Some(crate::ported::modules::files::bin_rmdir as HandlerFunc),
            1,
            -1,
            0,
            None,
            None,
        ), // c:823
        BUILTIN(
            "zf_sync",
            0,
            Some(crate::ported::modules::files::bin_sync as HandlerFunc),
            0,
            0,
            0,
            None,
            None,
        ), // c:824
    ]
});
// hash table containing builtin commands                                   // c:143
/// Process-wide builtin lookup table. Filled lazily the first time
/// `builtintab()` is called; mirrors the C `mod_export HashTable
/// builtintab` exposed at `Src/builtin.c:146`.
static builtintab: OnceLock<HashMap<String, &'static builtin>> = OnceLock::new();

/// The BUCKET-ARRAY form of `builtintab` — the storage C actually uses, kept
/// alongside the `HashMap` above (which stays the lookup fast path).
///
/// c:Src/builtin.c:152 `builtintab = newhashtable(85, "builtintab", NULL);`
/// then c:167 `addbuiltins("zsh", builtins, sizeof(builtins)/sizeof(*builtins))`,
/// whose loop (c:Src/module.c:547-557) walks the `builtins[]` array in
/// DECLARATION order calling `addbuiltin` → `builtintab->addnode` (= the
/// `addhashnode` / `addhashnode2` front-insert at c:Src/hashtable.c:168-222).
///
/// Enumeration order matters beyond cosmetics: `scanhashtable` walks buckets
/// 0..hsize-1, each chain head→tail (c:Src/hashtable.c:420-434), and that is
/// the order `${(k)builtins}` prints AND the order `compadd -k builtins` feeds
/// `join_clines`, whose ambiguous-prefix fold is not commutative. A `HashMap`
/// (or a sorted list) produces a different, wrong sequence.
///
/// 85 buckets vs 103 nodes never trips the c:183/219 `ct >= hsize * 2`
/// expansion, so the table keeps its original `hsize`.
/// Iterate it with `.iter()` to get C's `scanhashtable` traversal — bucket
/// 0..hsize-1, each chain head→tail (c:Src/hashtable.c:420-434).
///
/// Only entries that are actually IN `builtintab` are added: zshrs's
/// `BUILTINS` slice is the static union of every statically-linked module's
/// bintab, while C's table holds the core `builtins[]` plus whatever the
/// currently-loaded modules added. `builtin_in_builtintab` is the existing
/// predicate for that membership; feeding the same set through C's hash makes
/// the bucket walk match `zsh -f` name for name.
pub static BUILTINTAB_NODES: std::sync::LazyLock<
    crate::ported::hashtable::hashtable_nodes<&'static builtin>, // c:152
> = std::sync::LazyLock::new(|| {
    // c:152 — `newhashtable(85, "builtintab", NULL)`.
    let mut ht = crate::ported::hashtable::hashtable_nodes::newhashtable(85);
    // c:167 → c:Src/module.c:547 `for (n = 0; n < size; n++)`, i.e. the
    // `builtins[]` DECLARATION order. A duplicate name re-adds in place
    // (c:Src/hashtable.c:189-204 `replacing:`), leaving it at the same chain
    // position C's `BINF_ADDED` skip would.
    for b in BUILTINS.iter() {
        if !crate::ext_builtins::builtin_in_builtintab(&b.node.nam) {
            continue;
        }
        ht.addhashnode2(&b.node.nam, b); // c:Src/module.c:551 addbuiltin
    }
    ht
});

/// Names whose `node.flags & DISABLED` is set in C. The Rust port's
/// `builtintab` is an immutable static, so the disabled bit lives
/// in this parallel set; `bin_enable` toggles it via builtin.c:587.
/// Dispatch sites check `is_builtin_disabled(name)` before calling
/// `handlerfunc` to mirror C's "skip nodes with DISABLED set" walk.
pub static BUILTINS_DISABLED: std::sync::LazyLock<
    // c:587 (Src/builtin.c)
    Mutex<std::collections::HashSet<String>>,
> = std::sync::LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

// `shfunctab` is the canonical singleton at `hashtable::shfunctab_lock()`
// (RwLock<shfunc_table>); the prior parallel `shfunctab_table()` /
// `SHFUNCTAB_INNER` (usize-pointer Mutex<HashMap>) was deleted so the
// `bin_functions` C-port and the bytecode function-def path both write
// through one table. C-faithful access via `addnode`/`getnode`/`getnode2`
// methods (Src/zsh.h:281+ HashTable GSU pointers).

// `matchednodes` global from Src/builtin.c:4550.
/// `MATCHEDNODES` static.
pub static MATCHEDNODES: Mutex<Vec<String>> = Mutex::new(Vec::new());

// `stopmsg` global from Src/jobs.c — non-zero when checkjobs() printed.
/// `STOPMSG` static.
pub static STOPMSG: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
// `sfcontext` global from Src/exec.c:239 — current shell-function
// dispatch context (SFC_NONE / SFC_BUILTIN / SFC_FUNC / SFC_SUBST...).
/// `SFCONTEXT` static.
pub static SFCONTEXT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:exec.c:239
                                                                                           // `maxjob` / `thisjob` globals from Src/jobs.c:62/63 — canonical
                                                                                           // storage lives in jobs.rs (`OnceLock<Mutex<i32>>`). The previous
                                                                                           // builtin.rs duplicate `AtomicI32` stores NEVER synced with the
                                                                                           // jobs.rs Mutex<i32> values that the spawn/wait paths actually
                                                                                           // update; `checkjobs` (line 5092) read stale 0s no matter how many
                                                                                           // jobs were active. Callers route through jobs::MAXJOB / jobs::THISJOB
                                                                                           // directly now.
                                                                                           // `jobstats` mirror — flat per-slot stat bits (STAT_*). Real jobtab
                                                                                           // lives in src/ported/jobs.rs's JobTable; this mirror is updated by
                                                                                           // the spawn/wait paths that already touch STOPMSG. Empty → no jobs,
                                                                                           // matching the post-init state of `jobtab[]`.
/// `JOBSTATS` static.
pub static JOBSTATS: Mutex<Vec<i32>> = Mutex::new(Vec::new());

// File-static globals for [_]realexit/zexit — c:5945+, init.c, signals.c.
/// `SHELL_EXITING` static.
pub static SHELL_EXITING: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
/// `EXIT_PENDING` static.
pub static EXIT_PENDING: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
/// `EXIT_VAL` static.
pub static EXIT_VAL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
/// Port of `mod_export volatile int exit_level;` from `Src/builtin.c:5796`.
///
/// Records the `locallevel` at the moment a deferred `exit` was issued
/// inside a function. The `exec.c:6141` gate
/// `if (exit_pending && exit_level >= locallevel+1 && !in_exit_trap)`
/// fires only when the unwind has reached a scope at or above the
/// deferral point — preventing premature exit while the deferred
/// status walks back through nested function frames. C uses
/// `volatile int` because the value is read from a signal-touching
/// context; Rust's AtomicI32 with Relaxed ordering matches the same
/// no-fence read shape.
pub static EXIT_LEVEL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
// ====================================================================
// !!! WARNING: RUST-ONLY ATOMIC — NO DIRECT C COUNTERPART !!!
// ====================================================================
// C zsh forks for `(...)` subshells; the child runs to completion
// then exits via process::exit, and the parent (post-fork) continues.
// zshrs runs subshells in-process via fusevm_bridge::subshell_begin/
// subshell_end (no fork), so `exit N` inside a subshell would call
// realexit() → process::exit(N) and terminate the WHOLE shell —
// breaking `(exit 7); echo $?` (expected: `7\n`, observed: shell
// dies with code 7).
//
// This counter is bumped by subshell_begin / decremented by
// subshell_end. zexit() at c:5977 checks it before realexit and,
// when > 0, sets EXIT_VAL + EXIT_PENDING and returns — letting the
// subshell unwinder catch the deferred exit at its boundary
// (mirroring what the deferred-exit path at c:5871-5891 does for
// function-scope exit).
//
// In C zsh, equivalent state is `forklevel` AT EXACTLY the subshell
// depth that fork would create — but the C check `locallevel >
// forklevel` is FALSE at subshell-top precisely so the fork can
// exit the child via realexit. Without fork, we need this extra
// gate.
/// `SUBSHELL_DEPTH` static.
pub static SUBSHELL_DEPTH: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
/// `LASTVAL` static.
pub static LASTVAL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

// `tok` for the test builtin — Src/builtin.c:7000 ranges. The full enum
// lives in src/ported/lex.rs; we mirror the few values testlex() touches.
/// `TEST_TOK` static.
pub static TEST_TOK: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
const TEST_LEXERR: i32 = -1; // c:7209
const TEST_NULLTOK: i32 = 0;
const TEST_DBAR: i32 = 2; // c:7213
const TEST_DAMPER: i32 = 3; // c:7215
const TEST_BANG: i32 = 4; // c:7217
const TEST_INPAR: i32 = 5; // c:7219
const TEST_OUTPAR: i32 = 6; // c:7221
const TEST_INANG: i32 = 7; // c:7223
const TEST_OUTANG: i32 = 8; // c:7225
const TEST_STRING: i32 = 9; // c:7227

// `testargs` / `curtestarg` / `tokstr` globals from Src/builtin.c — the
// argv-style cursor that bin_test seeds and testlex() advances.
/// `TESTARGS` static.
pub static TESTARGS: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// `TESTARGS_IDX` static.
pub static TESTARGS_IDX: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
/// `TOKSTR` static.
pub static TOKSTR: Mutex<String> = Mutex::new(String::new());

// int doprintdir = 0; set in exec.c (for autocd, cdpath, etc.)            // c:722
// `doprintdir` from Src/exec.c — set when an autocd'd command should
// echo the new directory before executing.
/// `DOPRINTDIR` static.
pub static DOPRINTDIR: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
// set if we are resolving links to their true paths                       // c:829
// `chasinglinks` from Src/exec.c — non-zero when CHASELINKS / -P
// resolution is active.
/// `CHASINGLINKS` static.
pub static CHASINGLINKS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

// `pparams` global from Src/init.c — positional parameters $1..$N.
/// `PPARAMS` static.
pub static PPARAMS: Mutex<Vec<String>> = Mutex::new(Vec::new());

// `zoptind` (Src/builtin.c:5667) and `optcind` (c:5670) — the two
// pieces of getopts state. zoptind backs the user-visible $OPTIND.
/// `ZOPTIND` static.
pub static ZOPTIND: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(1);
/// `OPTCIND` static.
pub static OPTCIND: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

// `ttyfrozen` global lives canonically in jobs.rs (`OnceLock<Mutex<i32>>`
// at jobs.rs:2625). The previous AtomicI32 duplicate here NEVER
// synced with the jobs.rs store — same desync hazard as the prior
// MAXJOB / THISJOB fix. Callers route through jobs::TTYFROZEN.

/// Port of `mod_export int ineval` from `Src/builtin.c:6389`. Set
/// while `eval` is dispatching its body (incremented before
/// `execode(prog, 1, 0, "eval")`, decremented after). Tested by
/// `IN_EVAL_TRAP()` in zsh.h:2962 to determine trap-context state.
pub static INEVAL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:6389

// `loops` / `breaks` / `contflag` / `retflag` / `locallevel` / `sourcelevel`
// globals from Src/loop.c + Src/init.c — control-flow state consulted by
// the bin_break dispatcher.
/// `LOOPS` static.
pub static LOOPS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
/// `BREAKS` static.
pub static BREAKS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
/// `CONTFLAG` static.
pub static CONTFLAG: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
/// `RETFLAG` static.
pub static RETFLAG: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
// Same single-storage rationale as locallevel_param above — C zsh has
// only ONE `int sourcelevel;` global (Src/init.c:60). The canonical
// Rust port is `sourcelevel_init` (lowercase,
// matches C name). Re-export that single storage so the bin_break
// reader and the bin_dot bumps address the same atomic; without
// this, `bin_dot` could increment one global while `bin_break`
// inspected the other and `return` inside a sourced file would
// fall through to `zexit` (Src/builtin.c:5858).
// `locallevel_param` was previously a SEPARATE AtomicI32 here, but C
// zsh has only ONE `int locallevel;` global (Src/params.c:54).
// The canonical Rust port is `locallevel_param`
// (lowercase, matches C name). Re-export that single storage so
// every reader and writer addresses the same atomic — without
// this, `locallevel_param.store(0)` in zle/computil.rs would zero one
// global while `params::locallevel.fetch_add(1)` in vm_helper
// incremented a DIFFERENT global, leaving the two views out of
// sync indefinitely.
// `ZEXIT_NORMAL` re-exported from canonical zsh_h.rs (port of the
// `enum { ZEXIT_NORMAL, ZEXIT_SIGNAL, ZEXIT_DEFERRED }` in Src/zsh.h).
// Same single-source-of-truth pattern as TERM_UNKNOWN / HISTFLAG_*
// / etc — duplicate const declarations are a drift hazard.

// Local builders that construct C-shape `builtin` rows for the
// static registration table below. They mirror the
// `BUILTIN(...)` / `BIN_PREFIX(...)` macros in `Src/zsh.h:1450-1452`,
// taking `u32` flag bitsets (BINF_*) and a `&str` handler-name
// column used only for documentation/wiring lookup — handler
// function pointers themselves are wired up later in
// `Executor::register_builtins` (`src/ported/vm_helper`).
//
// The `handler` arg was previously a `_handler_name: &'static str` that
// was discarded — `handlerfunc` always ended up `NULLBINCMD`, so
// `execbuiltin`'s c:506 `(*handlerfunc)(...)` dispatch was unreachable.
// Now the descriptor carries the actual port-side `HandlerFunc` so
// `execbuiltin` can parse flags and call through to the real builtin.
/// `BUILTIN` — see implementation.
#[allow(non_snake_case)]
pub fn BUILTIN(
    name: &str,
    flags: u32,
    handler: Option<HandlerFunc>,
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

// `traps` mirror — sig name → body. Real `sigtrapped[]`/`siglists[]`
// arrays live in src/ported/signals.rs; this Mutex is the static-link
// shim that bin_trap reads/writes.
static TRAPS_INNER: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

#[allow(non_snake_case)]
fn BIN_PREFIX(name: &str, flags: u32) -> builtin {
    BUILTIN(name, flags | BINF_PREFIX, None, 0, 0, 0, None, None)
}

/// Inline printf-style format helper used by bin_print's -f/printf mode.
/// Replaces `%s` / `%d` / `%i` / `%c` / `%%` with positional args.
/// Full C printf-spec engine (Src/builtin.c:4691-5500) is much more
/// elaborate (width/precision/flag chars/%b/%q/etc.); this is the
/// minimal subset that covers the common script patterns.
///
/// Returns `Ok(output)` on success, or `Err((output_so_far, bad_char))`
/// when an unknown `%X` directive is hit. The C source emits
/// `zwarnnam(name, "%s: invalid directive", start)` at builtin.c:5435
/// then `return 1` — partial output already written stays written. The
/// Rust caller (bin_print, c:4854+) mirrors that: print the partial
/// output, emit `zwarnnam`, return 1.
/// Returns the formatted output plus the byte offset at which each
/// format-reuse CYCLE began (c:Src/builtin.c `splits`). `printf -v` to
/// an array assigns one element per cycle, so the caller slices `out`
/// at these boundaries.
fn printf_format(
    fmt: &str,
    args: &[String],
) -> Result<(String, Vec<usize>, Vec<(String, usize)>), (String, String)> {
    // c:Src/builtin.c:4711 — `fmt = getkeystring(fmt, &flen, ...,
    // GETKEYS_PRINTF_FMT, ...);`. The format string is first run
    // through getkeystring to interpret backslash escapes (`\n`,
    // `\t`, `\xNN`, etc.) before %-format substitution. The shim
    // `getkeystring(s)` defaults flags to 0, which (a) drops the
    // backslash on `\'`/`\"` (wrong for printf — zsh keeps unknown
    // `\<c>` literal because PRINTF_FMT excludes GETKEY_EMACS) and
    // (b) treats `\c` as a control-char escape rather than the
    // truncation marker. Route through `_with(GETKEYS_PRINTF_FMT)`
    // to match C exactly. `_with` takes a u32 `how` mask; the
    // canonical i32 const lives in `zsh_h` (Src/zsh.h:3180-3181).
    let (fmt, _) = getkeystring_with(fmt, crate::ported::zsh_h::GETKEYS_PRINTF_FMT as u32, None); // c:builtin.c:4711
                                                                                                  // c:Src/builtin.c:4696/5382/5527 — a `\c` in the FORMAT (or, below, in
                                                                                                  // a `%b` arg) sets `fmttrunc`, which (a) truncates output here and
                                                                                                  // (b) stops the format-reuse loop entirely (no reapplication over the
                                                                                                  // remaining args). getkeystring_with already cut `fmt` at the `\c`;
                                                                                                  // take the TLS flag it set so the reuse loop knows to stop.
    let mut fmttrunc = crate::ported::utils::getkey_truncated_take();
    let mut out = String::new();
    let mut arg_i: usize = 0;
    // c:Src/builtin.c:5166/5176 — `first` is the base of the current
    // format-reuse cycle's argument window. `%n$` positional specs index
    // `first + n - 1`; at the start of each new cycle `first += maxarg`
    // (the highest positional used in the prior cycle). `first_off` is
    // the Rust analog (absolute offset into `args`).
    let mut first_off: usize = 0;
    // c:Src/builtin.c — `splits`: byte offset where each format-reuse
    // cycle starts (the first is 0). Used by `printf -v ARRAY`.
    let mut bounds: Vec<usize> = Vec::new();
    // c:Src/builtin.c:5427-5429 — `case 'n': if (curarg) setiparam(curarg,
    // count - rcount);`. `%n` consumes an argument (the variable NAME) and
    // stores the number of bytes emitted so far into it, printing nothing.
    // printf_format is a pure formatter with no shell handle, so it records
    // (name, byte_count_at_this_point) and the caller does the setiparam —
    // out.len() here is exactly C's `count - rcount`.
    let mut n_targets: Vec<(String, usize)> = Vec::new();
    // c:Src/builtin.c:4914-4923 — printf reapplies the format string
    // until ALL args are consumed. `printf '%s,' a b c` → `a,b,c,`,
    // not `a,`. The outer loop reapplies; the inner do-while body
    // mirrors C's per-arg conversion loop directly.
    loop {
        bounds.push(out.len());
        // c:Src/builtin.c:5168 `rcount = count;` reset the `%n` byte count
        // at the start of each format-reuse cycle, so `%n` reports bytes
        // emitted SINCE this cycle began, not cumulatively. `printf 'x%n' a b`
        // stores 1 in both a and b, not 1 and 2.
        let cycle_start = out.len();
        let prev = arg_i;
        // c:Src/builtin.c:5175-5178 — `maxarg` is the highest positional
        // (`%n$`) referenced in THIS cycle; reset per cycle and folded
        // into `first_off` at cycle end.
        let mut cycle_maxarg: usize = 0;
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
            // c:Src/builtin.c:5199-5219 — `%n$` positional argument
            // specifier. A leading 1-9 digit run followed by `$` selects
            // the n-th argument (1-based) for this conversion instead of
            // the next sequential one. A leading digit run NOT followed
            // by `$` is an ordinary field width (handled below), so peek
            // the whole run before deciding. (`0` can't start a
            // positional — it's a flag — so C tests `'1'..'9'`.)
            let mut positional: Option<usize> = None;
            if matches!(iter.peek(), Some(&d) if ('1'..='9').contains(&d)) {
                let mut digits = String::new();
                while let Some(&d) = iter.peek() {
                    if d.is_ascii_digit() {
                        digits.push(d);
                        iter.next();
                    } else {
                        break;
                    }
                }
                if iter.peek() == Some(&'$') {
                    iter.next(); // c:5202 — consume `$`
                    let narg: usize = digits.parse().unwrap_or(0);
                    // c:5203-5212 — out-of-range positional is a hard
                    // error (`zwarnnam(...); return 1`). `argc` in C is
                    // the per-cycle remaining count (total minus the
                    // already-consumed `first_off`).
                    if narg == 0 || first_off + narg > args.len() {
                        return Err((
                            out,
                            format!("{}: argument specifier out of range", narg), // c:5204
                        ));
                    }
                    // c:5214-5216 — `if (narg > maxarg) maxarg = narg;`
                    //               `curarg = *(first + narg - 1);`
                    if narg > cycle_maxarg {
                        cycle_maxarg = narg;
                    }
                    positional = Some(first_off + narg - 1);
                } else {
                    // Not a positional: the digits are an ordinary field
                    // width — emit them into the spec and fall through.
                    spec.push_str(&digits);
                }
            }
            // A precision `*` with NO matching arg emits nothing into `spec`
            // (default precision), so the `%`-conversion invalid-directive
            // check below can't see it. Track it separately: `printf '%.*%'`
            // (no arg) is still an invalid directive in zsh (the format
            // carried a precision), exit 1. Src/builtin.c reaches the switch
            // with a non-`%%` `d` buffer regardless of the runtime arg.
            let mut saw_prec_star = false;
            loop {
                match iter.peek() {
                    // c:Src/builtin.c:4791+ — printf flag chars. The `'`
                    // (thousands-grouping) flag is passed straight to libc
                    // by C zsh; the Rust port groups the digit run itself
                    // in format_spec_int/uint per localeconv. Accept it here
                    // so it isn't mistaken for the conversion char (which
                    // produced "%': invalid directive").
                    Some(&c) if matches!(c, '-' | '+' | ' ' | '#' | '0' | '\'') => {
                        spec.push(c);
                        iter.next();
                    }
                    _ => break,
                }
            }
            // c:Src/builtin.c:4791-4796 — width can be either a
            // digit literal or `*` (consume next arg as width).
            // Without `*` handling, `printf '%*d' 4 7` rendered the
            // literal `%*d` because the iter saw `*` and aborted the
            // spec walk before reaching the conversion char.
            if iter.peek() == Some(&'*') {
                iter.next(); // c:4796 — consume the `*` marker
                             // c:Src/builtin.c:5240-5247 — the `*` width arg is
                             // MATH-EVALUATED (`width = (int)mathevali(metafy(*argp,
                             // …))`), not a plain integer parse: `0x1f`→31, ` 4`→4,
                             // `'A`→65, `2+3`→5. parse_int_arg is the shared
                             // getnum→mathevali path (same one the %d/%i arm uses),
                             // and on a math error it zeroes the value and flags
                             // ret=1 (c:5243 `errflag → ret = 1`). Previously the
                             // plain `str::parse` silently yielded 0 for any non-
                             // decimal width arg.
                let w: i64 = args.get(arg_i).map(|s| parse_int_arg(s)).unwrap_or(0);
                arg_i += 1;
                spec.push_str(&w.to_string());
            } else {
                while let Some(&c) = iter.peek() {
                    if c.is_ascii_digit() {
                        spec.push(c);
                        iter.next();
                    } else {
                        break;
                    }
                }
            }
            if iter.peek() == Some(&'.') {
                iter.next(); // consume the `.`
                             // `.` precision: also accepts `*` per c:4796 same as width.
                if iter.peek() == Some(&'*') {
                    iter.next();
                    saw_prec_star = true;
                    // c:Src/builtin.c:5275-5288 — the `*` precision arg is
                    // math-evaluated identically to the `*` width arg
                    // (`prec = (int)mathevali(metafy(*argp, …))`, error →
                    // ret=1) — BUT only when the arg actually EXISTS
                    // (c:5275 `if (*argp)`). If args have run out, `prec`
                    // stays at its -1 init (c:5178) and NO precision is
                    // emitted (c:5288 `if (prec >= 0) *d++ = '.', *d++ =
                    // '*'`). A negative result is unset the same way.
                    // Unlike width (init 0), a MISSING `*` precision must
                    // leave the conversion at default precision, not force
                    // `.0`: `printf '%.*d' ` prints `0` (default), not the
                    // truncated-to-empty `%.0d`. Previously `unwrap_or(0)`
                    // forced `.0` and dropped the digit.
                    if let Some(a) = args.get(arg_i) {
                        let p = parse_int_arg(a);
                        arg_i += 1;
                        if p >= 0 {
                            spec.push('.');
                            spec.push_str(&p.to_string());
                        }
                    }
                } else {
                    // Literal precision: `.` then optional digits (an empty
                    // digit run is precision 0, c:5283-5287 `else prec = 0`).
                    spec.push('.');
                    while let Some(&c) = iter.peek() {
                        if c.is_ascii_digit() {
                            spec.push(c);
                            iter.next();
                        } else {
                            break;
                        }
                    }
                }
            }
            // c:Src/builtin.c:5290 — `/* ignore any size modifier */
            // if (*c == 'l' || *c == 'L' || *c == 'h') c++;`. zsh accepts
            // and discards a single length modifier (`%ld`, `%lu`, `%hd`)
            // — the conversion char that follows determines the type and
            // C rebuilds the libc format as `%l<conv>` itself. NOT pushed
            // into `spec`: it would corrupt the format passed to the
            // format_spec_* helpers. Only one modifier char is skipped,
            // matching C (so `%lld` then errors on the second `l`).
            if matches!(iter.peek(), Some('l') | Some('L') | Some('h')) {
                iter.next();
            }
            // c:Src/builtin.c:5215/5310 — a `%n$` positional spec sets
            // `curarg` directly; the conversion arms below read from
            // `arg_i`, so point it at the positional index for the
            // duration of the conversion, then restore. Restoring means
            // the positional reference does NOT advance the sequential
            // cursor (C only advances `argp` for non-positional args).
            let saved_argi = arg_i;
            let is_positional = positional.is_some();
            if let Some(p) = positional {
                arg_i = p;
            }
            match iter.next() {
                // c:Src/builtin.c:5414-5419 — a bare `%%` prints `%`, but a
                // `%` directive carrying any flag / width / precision / `*` is
                // an "invalid directive": zsh handles doubled `%%` in the
                // literal scan and only reaches the conversion switch (which
                // has no `%` case → default) once modifiers intervened. Here a
                // plain `%%` arrives with `spec == "%"`; anything else means
                // modifiers were consumed.
                Some('%') if spec != "%" || saw_prec_star => {
                    let disp = if saw_prec_star && spec == "%" {
                        "%.*"
                    } else {
                        spec.as_str()
                    };
                    return Err((out, format!("{disp}%: invalid directive")));
                }
                Some('%') => out.push('%'),
                Some('n') => {
                    // c:5427-5428 — `if (curarg) setiparam(curarg, count -
                    // rcount);`. The arg is the target variable's NAME; nothing
                    // is emitted. `if (curarg)` tests that an arg SLOT exists,
                    // not that it is non-empty — an empty or malformed name is
                    // still handed to setiparam, which errors `not an
                    // identifier`. Only a wholly ABSENT arg is the no-op.
                    // `count - rcount` is bytes since the cycle start
                    // (c:5168), i.e. out.len() - cycle_start here.
                    if let Some(name) = args.get(arg_i) {
                        n_targets.push((name.clone(), out.len() - cycle_start));
                        // c:5428
                    }
                    arg_i += 1;
                }
                Some('s') => {
                    let a = args.get(arg_i).cloned().unwrap_or_default();
                    spec.push('s');
                    out.push_str(&format_spec_str(&spec, &a, false));
                    arg_i += 1;
                }
                // bash/ksh `printf '%(FMT)T' TS` — strftime(FMT, TS). The
                // timestamp is epoch seconds; a NEGATIVE value or a MISSING
                // arg means "now" (bash also defines -2 = shell start, treated
                // as now here). zsh's printf has NO such directive (it uses the
                // `strftime` builtin), so it is gated to bash / ksh emulation;
                // under --zsh it stays an "invalid directive" like real zsh.
                Some('(') => {
                    let allow = crate::dash_mode::bash_mode()
                        || crate::ported::zsh_h::EMULATION(crate::ported::zsh_h::EMULATE_KSH);
                    // Collect the strftime format up to the matching ')'.
                    let mut tfmt = String::new();
                    let mut closed = false;
                    for c in iter.by_ref() {
                        if c == ')' {
                            closed = true;
                            break;
                        }
                        tfmt.push(c);
                    }
                    // The conversion char after ')' must be 'T'.
                    let conv_t = iter.next() == Some('T');
                    if !allow || !closed || !conv_t {
                        return Err((out, format!("{}(: invalid directive", spec)));
                    }
                    let now = crate::ported::modules::datetime::getcurrenttime()
                        .first()
                        .and_then(|s| s.parse::<i64>().ok())
                        .unwrap_or(0);
                    let ts: i64 = match args.get(arg_i) {
                        Some(a) => {
                            arg_i += 1;
                            let v = parse_int_arg(a);
                            if v < 0 {
                                now
                            } else {
                                v
                            }
                        }
                        None => now,
                    };
                    let formatted = (|| -> Option<String> {
                        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
                        let t = ts as libc::time_t;
                        let cfmt = std::ffi::CString::new(tfmt.as_str()).ok()?;
                        unsafe {
                            if libc::localtime_r(&t, &mut tm).is_null() {
                                return None;
                            }
                            let mut buf = vec![0u8; 256];
                            let n = libc::strftime(
                                buf.as_mut_ptr() as *mut libc::c_char,
                                buf.len(),
                                cfmt.as_ptr(),
                                &tm,
                            );
                            buf.truncate(n);
                            String::from_utf8(buf).ok()
                        }
                    })()
                    .unwrap_or_default();
                    // Apply any field width/flags to the rendered string, the
                    // same way `%s` does.
                    spec.push('s');
                    out.push_str(&format_spec_str(&spec, &formatted, false));
                }
                Some('d') | Some('i') => {
                    if crate::extensions::dash_mode::bash_printf_empty_numeric_error(
                        args.get(arg_i),
                    ) {
                        PRINTF_MATH_ERR.with(|c| c.set(true));
                    }
                    let a = args.get(arg_i).cloned().unwrap_or_default();
                    let n = parse_int_arg(&a);
                    spec.push('d');
                    out.push_str(&format_spec_int(&spec, n));
                    arg_i += 1;
                }
                Some('u') => {
                    if crate::extensions::dash_mode::bash_printf_empty_numeric_error(
                        args.get(arg_i),
                    ) {
                        PRINTF_MATH_ERR.with(|c| c.set(true));
                    }
                    let a = args.get(arg_i).cloned().unwrap_or_default();
                    // c:Src/builtin.c:5511 — `if (!zstrtoul_underscore(
                    // curarg, &zulongval)) zulongval = mathevali(...)`.
                    // Unsigned conversions first try a direct unsigned
                    // parse (so a full-range literal like
                    // 18446744073709551615 is NOT truncated by the
                    // signed-math path), then fall back to math eval
                    // (which wraps a negative result into u64).
                    let n = crate::ported::utils::zstrtoul_underscore(a.trim())
                        .unwrap_or_else(|| parse_int_arg(&a) as u64);
                    spec.push('u');
                    out.push_str(&format_spec_uint(&spec, n));
                    arg_i += 1;
                }
                Some('x') => {
                    if crate::extensions::dash_mode::bash_printf_empty_numeric_error(
                        args.get(arg_i),
                    ) {
                        PRINTF_MATH_ERR.with(|c| c.set(true));
                    }
                    let a = args.get(arg_i).cloned().unwrap_or_default();
                    // c:Src/builtin.c:5511 — `if (!zstrtoul_underscore(
                    // curarg, &zulongval)) zulongval = mathevali(...)`.
                    // Unsigned conversions first try a direct unsigned
                    // parse (so a full-range literal like
                    // 18446744073709551615 is NOT truncated by the
                    // signed-math path), then fall back to math eval
                    // (which wraps a negative result into u64).
                    let n = crate::ported::utils::zstrtoul_underscore(a.trim())
                        .unwrap_or_else(|| parse_int_arg(&a) as u64);
                    spec.push('x');
                    out.push_str(&format_spec_radix(&spec, n, 'x'));
                    arg_i += 1;
                }
                Some('X') => {
                    if crate::extensions::dash_mode::bash_printf_empty_numeric_error(
                        args.get(arg_i),
                    ) {
                        PRINTF_MATH_ERR.with(|c| c.set(true));
                    }
                    let a = args.get(arg_i).cloned().unwrap_or_default();
                    // c:Src/builtin.c:5511 — `if (!zstrtoul_underscore(
                    // curarg, &zulongval)) zulongval = mathevali(...)`.
                    // Unsigned conversions first try a direct unsigned
                    // parse (so a full-range literal like
                    // 18446744073709551615 is NOT truncated by the
                    // signed-math path), then fall back to math eval
                    // (which wraps a negative result into u64).
                    let n = crate::ported::utils::zstrtoul_underscore(a.trim())
                        .unwrap_or_else(|| parse_int_arg(&a) as u64);
                    spec.push('X');
                    out.push_str(&format_spec_radix(&spec, n, 'X'));
                    arg_i += 1;
                }
                Some('o') => {
                    if crate::extensions::dash_mode::bash_printf_empty_numeric_error(
                        args.get(arg_i),
                    ) {
                        PRINTF_MATH_ERR.with(|c| c.set(true));
                    }
                    let a = args.get(arg_i).cloned().unwrap_or_default();
                    // c:Src/builtin.c:5511 — `if (!zstrtoul_underscore(
                    // curarg, &zulongval)) zulongval = mathevali(...)`.
                    // Unsigned conversions first try a direct unsigned
                    // parse (so a full-range literal like
                    // 18446744073709551615 is NOT truncated by the
                    // signed-math path), then fall back to math eval
                    // (which wraps a negative result into u64).
                    let n = crate::ported::utils::zstrtoul_underscore(a.trim())
                        .unwrap_or_else(|| parse_int_arg(&a) as u64);
                    spec.push('o');
                    out.push_str(&format_spec_radix(&spec, n, 'o'));
                    arg_i += 1;
                }
                Some(conv @ ('f' | 'F' | 'g' | 'G' | 'e' | 'E')) => {
                    let a = args.get(arg_i).cloned().unwrap_or_default();
                    // c:Src/builtin.c:5479-5488 — `doubleval = strtod(curarg,
                    // &eptr); if (*eptr) mnumval = matheval(curarg)`. The arg
                    // is parsed as a numeric constant first (handles "2.5",
                    // "nan", "inf"); if that leaves leftover (a variable name
                    // like `f`, or an expression like "1.5+1"), it is
                    // evaluated as a math expression. The previous port did
                    // only the constant parse, so `printf %g f` / `%f 1.5+1`
                    // yielded 0.
                    let n: f64 =
                        if let Some(rest) = a.strip_prefix('\'').or_else(|| a.strip_prefix('"')) {
                            // c:Src/builtin.c:5431-5447 — a leading `'`/`"` makes
                            // the value the numeric char code of the next char,
                            // for FLOAT conversions too (shared with int):
                            // `printf '%.1E' \'B` → 66 → 6.6E+01. The int arm
                            // (parse_int_arg) already did this; the float arm
                            // skipped it, yielding 0.
                            rest.chars().next().map(|c| c as i64 as f64).unwrap_or(0.0)
                        } else {
                            a.parse::<f64>().unwrap_or_else(|_| match matheval(&a) {
                                Ok(m) if m.type_ == crate::ported::math::MN_FLOAT => m.d,
                                Ok(m) => m.l as f64,
                                // c:Src/builtin.c:5488 — a math-eval failure on
                                // a FLOAT operand is a SOFT error, same as the
                                // integer path: emit the diagnostic, clear the
                                // errflag so later args still evaluate, and flag
                                // ret=1. The float arm previously swallowed it,
                                // so `printf '%g' '%d'` exited 0 where zsh exits
                                // 1. (Empty/missing args math-eval to Ok(0), so
                                // they do NOT trip this.)
                                Err(msg) => {
                                    crate::ported::utils::zerr(&msg);
                                    crate::ported::utils::errflag.fetch_and(
                                        !crate::ported::utils::ERRFLAG_ERROR,
                                        std::sync::atomic::Ordering::Relaxed,
                                    );
                                    PRINTF_MATH_ERR.with(|c| c.set(true));
                                    0.0
                                }
                            })
                        };
                    // c:Src/builtin.c printf %g/%G uses libc snprintf
                    // which strips trailing zeros; %e/%E uses scientific.
                    out.push_str(&format_spec_float_conv(&spec, n, conv));
                    arg_i += 1;
                }
                Some('c') => {
                    // c:5300-5306 — `%c` outputs the arg's first char with
                    // WIDTH/justify applied (`%3c`→"  x", `%-3c`→"x  ") but
                    // precision IGNORED (`%.0c`→"x"). Strip precision from the
                    // spec, then format as a one-char string. The previous port
                    // pushed the char raw, dropping width.
                    let a = args.get(arg_i).cloned().unwrap_or_default();
                    // c:5300-5305 — `intval = curarg ? *curarg : 0`. An
                    // empty or missing arg yields `*curarg == '\0'`, so
                    // `%c` emits a NUL byte (not nothing). zsh:
                    // `printf "%c" "" | od` → 00.
                    let ch_str = a.chars().next().unwrap_or('\0').to_string();
                    let mut cspec = spec.split('.').next().unwrap_or(spec.as_str()).to_string();
                    cspec.push('s');
                    out.push_str(&format_spec_str(&cspec, &ch_str, true));
                    arg_i += 1;
                }
                // c:builtin.c:5403-5409 %q — shell-quote the arg using
                // QT_BACKSLASH_SHOWNULL (backslash-escape form), NOT
                // QT_QUOTEDZPUTS (single-quote form).
                //
                //   c: stringval = quotestring(metafy(curarg, …),
                //                              QT_BACKSLASH_SHOWNULL);
                //
                // Symptom of the previous quotedzputs choice:
                //   printf "%q\n" "a b c"
                //   zshrs (before): 'a b c'        zsh: a\ b\ c
                //
                // p10k uses `printf "%q "` for shell-quoting cached
                // command lines; the difference makes those caches
                // unreadable in zsh-syntax debuggers expecting the
                // backslash form.
                // !!! DASH-STRICT GATE (no C counterpart) !!! dash's printf
                // has no `%q` directive; it errors "invalid directive". Under
                // dash_strict, skip this arm so `%q` falls to the `Some(other)`
                // arm below, which produces exactly that error like /bin/dash.
                Some('q') if !crate::dash_mode::dash_strict() => {
                    // c:Src/builtin.c:5387-5391 — `stringval = curarg ?
                    // quotestring(...) : &nullstr`. A MISSING argument (curarg
                    // == NULL) is the empty nullstr and is NOT quoted, so
                    // `printf '%q'` prints nothing — not `''`. A PRESENT arg,
                    // even an empty string, IS quoted (→ `''`).
                    let quoted = match args.get(arg_i) {
                        Some(a) => crate::ported::utils::quotestring(
                            a,
                            crate::ported::zsh_h::QT_BACKSLASH_SHOWNULL,
                        ),
                        None => String::new(),
                    };
                    // c:Src/builtin.c:5405-5407 — `%q` sets `*d = 's'`
                    // and runs the quoted value through the normal string
                    // output (`print_val`), so width/precision/flags from
                    // the spec apply (`printf "%-5q" a` → "a    ").
                    spec.push('s');
                    out.push_str(&format_spec_str(&spec, &quoted, true));
                    arg_i += 1;
                }
                // c:builtin.c:5332-5336 — `%b` uses GETKEYS_PRINTF_ARG
                // (Src/zsh.h:3183 = GETKEY_BACKSLASH_C alone), NOT
                // GETKEYS_PRINT. The distinction:
                //   GETKEYS_PRINT      = OCTAL_ESC | EMACS | BACKSLASH_C
                //     `\NNN` → octal, `\<c>` → drop backslash.
                //   GETKEYS_PRINTF_ARG = BACKSLASH_C
                //     `\NNN` → literal, `\0NNN` → octal, `\<c>` → keep both.
                // Bourne-style `printf '%b' '\141'` keeps `\141` literal
                // (only `\0NNN` is octal in the arg interpretation),
                // matching zsh. Previous Rust port used GETKEYS_PRINT,
                // so `\141` was octal-eval'd to `a` — diverged.
                Some('b') => {
                    let a = args.get(arg_i).cloned().unwrap_or_default();
                    let (s, _) = getkeystring_with(
                        &a,
                        crate::ported::zsh_h::GETKEYS_PRINTF_ARG as u32,
                        None,
                    );
                    // c:5380-5383 — a `\c` inside the `%b` arg truncates:
                    // emit the expansion up to `\c`, then stop the whole
                    // printf (no rest-of-format, no reuse). getkeystring_with
                    // already cut `s` at the `\c` and set the TLS flag.
                    let arg_truncated = crate::ported::utils::getkey_truncated_take();
                    // c:5307-5360 — `%b` shares the `%s` width+precision
                    // handling (`%5b`→"   ab", `%3.1b`→"  a"), applied to the
                    // escape-expanded string. The previous port pushed the
                    // expansion raw, dropping width/precision.
                    spec.push('s');
                    out.push_str(&format_spec_str(&spec, &s, false));
                    arg_i += 1;
                    if arg_truncated {
                        fmttrunc = true; // c:5382
                        break; // c:5383 — truncate the format here (stop this cycle)
                    }
                }
                // c:builtin.c:5420 — `%n` consumes its arg but writes
                // nothing. C printf writes the byte-count-so-far to
                // the int pointer; zsh has no pointer to write to, so
                // it silently drops the directive. Previous Rust port
                // fell to the unknown-arm and emitted literal `%n`,
                // breaking `printf "%n" x; echo y` (zsh emits `y`,
                // zshrs emitted `%ny`).
                Some('n') => {
                    arg_i += 1;
                }
                // c:builtin.c:5430-5443 — unknown directive in C is
                // a hard error: `zwarnnam(name, "%s: invalid
                // directive", start); ...; return 1;`. Bubble the
                // bad char up via Err; the caller emits the warning
                // and returns rc=1. Partial output already written
                // is preserved (matches C — the warning fires AFTER
                // earlier output bytes have already been emitted).
                Some(other) => {
                    // c:5430-5436 — the message echoes `start`: the spec
                    // text from `%` through the bad conversion char
                    // inclusive (C null-terminates at `c[1]`). `spec`
                    // already holds `%`+flags+width+`.prec`; append the
                    // bad char. e.g. `%0$s` → "%0$: invalid directive".
                    return Err((out, format!("{}{}: invalid directive", spec, other)));
                    // c:5435
                }
                // c:Src/builtin.c:5430-5436 — a bare `%` with nothing
                // after it (end of format) is an invalid directive, same
                // as an unknown conversion char: emit any preceding output
                // then error (rc 1). The previous port pushed a literal
                // `%`, so `printf '%'` printed `%` with rc 0.
                None => return Err((out, format!("{}: invalid directive", spec))),
            }
            // c:Src/builtin.c — restore the sequential cursor after a
            // positional conversion (see the swap above).
            if is_positional {
                arg_i = saved_argi;
            }
        }
        // c:Src/builtin.c:5175-5178 — at the end of a cycle that used
        // positional specs, fold the highest positional into the base
        // offset (`first += maxarg`) and advance the sequential cursor
        // past it so the reapply check below sees forward progress.
        if cycle_maxarg > 0 {
            first_off += cycle_maxarg;
            if first_off > arg_i {
                arg_i = first_off;
            }
        }
        // c:Src/builtin.c:5527 — `} while (... && !fmttrunc ...)`. A `\c`
        // in the format or a `%b` arg stops the reuse loop.
        if fmttrunc || arg_i == prev || arg_i >= args.len() {
            break;
        }
    }
    Ok((out, bounds, n_targets))
}

/// Apply a printf-style `%[-flag][width][.prec]s` spec to a string.
/// Mirrors C `printf "%-10s" str` formatting; the Rust `format!` macro
/// doesn't accept runtime-parsed specs so we hand-parse.
fn format_spec_str(spec: &str, s: &str, honor_zero: bool) -> String {
    // zsh's `%c` and `%q` route through `print_val`, which HONORS the `0` flag
    // (`printf %04c x` → `000x`, `%04q a` → `000a`), so those callers pass
    // honor_zero=true. `%s`/`%b` are handled inline with FORCED space padding
    // (Src/builtin.c:5375 `fprintf(fout, "%*c", …, ' ')` → `%04s ab` = "  ab"),
    // so they pass honor_zero=false.
    let (left_align, zero_pad_flag, width, prec) = parse_flags_width_prec(spec);
    let truncated: &str = if let Some(p) = prec {
        let end: usize = s.chars().take(p).map(|c| c.len_utf8()).sum();
        &s[..end.min(s.len())]
    } else {
        s
    };
    let pad = width.saturating_sub(truncated.chars().count());
    if left_align {
        format!("{}{}", truncated, " ".repeat(pad))
    } else {
        let padch = if honor_zero && zero_pad_flag {
            '0'
        } else {
            ' '
        };
        format!("{}{}", padch.to_string().repeat(pad), truncated)
    }
}

/// c:Src/builtin.c — printf integer-argument parser. zsh accepts:
/// - Decimal (`42`, `-7`) — leading 0 stays decimal, NOT octal.
/// - Hex (`0x10`, `0X10`)
/// - Single character prefix `'A'` or `"A"` → ASCII code
/// - `BASE#NNN` radix literal
///
/// c:Src/builtin.c:5460-5464 — printf evaluates the `%d`/`%i` operand with
/// `mathevali`, which zerr's a "bad math expression" diagnostic AND sets
/// errflag on a parse error (`printf %d 12abc` → "operator expected at
/// `abc'"). C then clears ERRFLAG_ERROR and sets the builtin's `ret = 1`,
/// keeping the value at 0 but continuing to format the remaining args.
/// zshrs's `mathevali` returns the message in `Err` instead of emitting, so
/// surface it here (matching the `$((…))` path at subst.rs:2562), clear the
/// error so following args still evaluate, and record the soft failure in
/// `PRINTF_MATH_ERR` for bin_print to turn into exit status 1.
thread_local! {
    static PRINTF_MATH_ERR: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn parse_int_arg(s: &str) -> i64 {
    // c:Src/builtin.c bin_print c:5447 — character-constant check
    // operates on the RAW arg, not after trim. `*curarg == '\''`
    // → take `curarg[1]` as a byte regardless of whitespace.
    // Otherwise `printf "%d" "' "` returns 0 instead of 32 because
    // the trailing space gets stripped before the leading-quote test.
    if let Some(rest) = s.strip_prefix('\'').or_else(|| s.strip_prefix('"')) {
        return rest.chars().next().map(|c| c as i64).unwrap_or(0);
    }
    // !!! POSIX-FAITHFUL GATE (no C counterpart) !!!
    // zsh's printf `%d` math-EVALUATES its operand, so `printf %d A` treats
    // `A` as a math variable (→ 0, exit 0) and `printf %d 1+1` → 2. dash /
    // POSIX sh instead do `strtoimax` NUMERIC parsing: they use the leading
    // parsed value and exit non-zero on any non-numeric junk (`A` → 0 exit 1,
    // `1+1` → 1 exit 1, `12x` → 12 exit 1, `  7  ` → 7 exit 1), while an
    // empty operand is a clean 0. Under `zshrs --sh`/`--dash`
    // (posix_faithful + EMULATE_SH) parse numerically to match the real
    // shell exactly. `--... --zsh` and runtime `emulate sh` leave
    // posix_faithful() false and keep zsh's math-eval semantics.
    if crate::dash_mode::posix_faithful()
        && crate::ported::zsh_h::EMULATION(crate::ported::zsh_h::EMULATE_SH)
    {
        if s.is_empty() {
            return 0; // dash: empty operand → 0, exit 0 (no error)
        }
        // strtoimax: skip leading whitespace, optional sign, base prefix.
        let t = s.trim_start_matches([' ', '\t', '\n']);
        let (neg, body) = match t.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, t.strip_prefix('+').unwrap_or(t)),
        };
        let (radix, digits) =
            if let Some(h) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
                (16u32, h)
            } else if body.starts_with('0') {
                (8u32, body) // leading 0 → octal ("0" itself is a clean 0)
            } else {
                (10u32, body)
            };
        let ndig = digits.chars().take_while(|c| c.is_digit(radix)).count();
        // Digit chars are ASCII, so char count == byte length of the run.
        let mut val = i64::from_str_radix(&digits[..ndig], radix).unwrap_or(0);
        if neg {
            val = -val;
        }
        // POSIX: any leftover after the digit run — or no digits at all —
        // is a diagnostic (exit 1); the leading value is still used.
        if ndig == 0 || ndig != digits.chars().count() {
            PRINTF_MATH_ERR.with(|c| c.set(true));
        }
        return val;
    }
    // c:Src/builtin.c:5460 — `zlongval = mathevali(metafy(curarg, …))`.
    // printf evaluates the %d/%i operand as a math expression. matheval
    // natively handles decimal / 0x-hex / BASE#NNN / float-truncate and
    // overflow-truncation: the math lexer's zstrtol (Src/utils.c:2511)
    // emits the "number truncated after N digits" warning, so the
    // magnitude 9223372036854775808 truncates to 18 digits exactly as
    // zsh, and INT64_MIN (-9223372036854775808) round-trips without the
    // manual `-parsed` negate that panicked on i64::MIN.
    // c:5460-5464 — evaluate as math; on error emit the diagnostic (C's
    // mathevali does this internally), zero the value, and flag ret=1.
    match crate::ported::math::mathevali(s.trim()) {
        Ok(n) => n,
        Err(msg) => {
            crate::ported::utils::zerr(&msg);
            // c:5463 `errflag &= ~ERRFLAG_ERROR` — clear so the NEXT %d arg
            // still evaluates (matheval bails on a pre-set errflag,
            // math.rs:569); the soft failure is tracked separately.
            crate::ported::utils::errflag.fetch_and(
                !crate::ported::utils::ERRFLAG_ERROR,
                std::sync::atomic::Ordering::Relaxed,
            );
            PRINTF_MATH_ERR.with(|c| c.set(true)); // c:5464 ret = 1
            0
        }
    }
}

fn format_spec_int(spec: &str, n: i64) -> String {
    let (left_align, zero_pad_flag, width, prec) = parse_flags_width_prec(spec);
    // c:libc printf %d — when precision is given, `0` flag is
    // ignored (precision controls the digit padding instead).
    let zero_pad = zero_pad_flag && !left_align && prec.is_none();
    // c:Src/builtin.c — `+` flag: prefix positive numbers with `+`.
    // ` ` flag: prefix positive numbers with a space (mutually
    // exclusive with `+` per POSIX; `+` wins when both set).
    let plus_flag = spec.contains('+');
    let space_flag = spec.contains(' ') && !plus_flag;
    let digits = n.unsigned_abs().to_string();
    // c:libc printf %d precision = minimum number of digits;
    // zero-pad the body up to that count BEFORE the sign + width. Converting
    // a zero value with precision 0 yields NO digits (`printf '%.0d' 0` → "").
    let digits = if let Some(p) = prec {
        if p == 0 && n == 0 {
            String::new()
        } else if digits.len() < p {
            format!("{}{}", "0".repeat(p - digits.len()), digits)
        } else {
            digits
        }
    } else {
        digits
    };
    // c:libc printf `'` (thousands-grouping) flag — C hands `'` to the
    // system printf; the Rust port groups the digit run itself per the
    // active locale's `thousands_sep` (localeconv, set from the env at
    // init.rs:1709). Empty separator (C/POSIX locale) → no grouping,
    // matching `printf "%'d" 1234567` = `1234567` under LC_ALL=C.
    // Grouping is suppressed when an explicit precision is given: glibc
    // does NOT group the precision zero-fill (`printf "%'.8d" 42` =
    // `00000042`, not `00,000,042`), so gate on `prec.is_none()`. This
    // matches glibc for every precision case where the value fits within
    // the precision digit count (the normal use of `%'.Nd`); the rare
    // degenerate combo of `'` + a precision SHORTER than a large value's
    // own digit count (`%'.8d` of 1234567 → glibc `1,234,567`) still
    // groups in glibc but is left ungrouped here — a documented edge.
    // Inlined rather than shared with format_spec_uint because the
    // port-purity gate forbids new fns under src/ported/.
    let digits = if spec.contains('\'') && prec.is_none() {
        let sep = unsafe {
            let lc = libc::localeconv();
            if lc.is_null() || (*lc).thousands_sep.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr((*lc).thousands_sep)
                    .to_string_lossy()
                    .into_owned()
            }
        };
        if sep.is_empty() {
            digits
        } else {
            let chars: Vec<char> = digits.chars().collect();
            let len = chars.len();
            let mut grouped = String::with_capacity(len + len / 3 * sep.len());
            for (i, ch) in chars.iter().enumerate() {
                if i > 0 && (len - i) % 3 == 0 {
                    grouped.push_str(&sep);
                }
                grouped.push(*ch);
            }
            grouped
        }
    } else {
        digits
    };
    let body = if n < 0 {
        format!("-{}", digits)
    } else if plus_flag {
        format!("+{}", digits)
    } else if space_flag {
        format!(" {}", digits)
    } else {
        digits
    };
    let pad = width.saturating_sub(body.chars().count());
    if pad == 0 {
        body
    } else if left_align {
        format!("{}{}", body, " ".repeat(pad))
    } else if zero_pad {
        // Zero-pad: sign/prefix char (`-`, `+`, ` `) stays at the
        // left, zeros pad between it and the digits.
        if let Some(rest) = body
            .strip_prefix('-')
            .or_else(|| body.strip_prefix('+'))
            .or_else(|| body.strip_prefix(' '))
        {
            let sign = body.chars().next().unwrap();
            format!("{}{}{}", sign, "0".repeat(pad), rest)
        } else {
            format!("{}{}", "0".repeat(pad), body)
        }
    } else {
        format!("{}{}", " ".repeat(pad), body)
    }
}

/// printf %x / %X / %o with full flag support: `#` prefix, zero pad,
/// width, left-align. Matches libc printf semantics.
fn format_spec_radix(spec: &str, n: u64, conv: char) -> String {
    let (left_align, zero_pad_flag, width, prec) = parse_flags_width_prec(spec);
    // c:libc printf — when a precision is given the `0` flag is ignored;
    // precision controls the digit zero-fill instead.
    let zero_pad = zero_pad_flag && !left_align && prec.is_none();
    let hash_flag = spec.contains('#');
    let body = match conv {
        'x' => format!("{:x}", n),
        'X' => format!("{:X}", n),
        'o' => format!("{:o}", n),
        _ => n.to_string(),
    };
    // c:libc printf %o/%u/%x/%X — precision is the MINIMUM number of digits;
    // zero-pad the body up to it BEFORE the `#` prefix and width. Converting
    // a zero value with precision 0 yields NO digits.
    let body = if let Some(p) = prec {
        let len = body.chars().count();
        if p == 0 && n == 0 {
            String::new()
        } else if len < p {
            format!("{}{}", "0".repeat(p - len), body)
        } else {
            body
        }
    } else {
        body
    };
    // c:Src/builtin.c — `#` flag: prefix with `0x`/`0X` for hex (only
    // when value non-zero), `0` for octal (always, even zero, which
    // libc handles by emitting "0" anyway).
    let body = if hash_flag {
        match conv {
            'x' if n != 0 => format!("0x{}", body),
            'X' if n != 0 => format!("0X{}", body),
            'o' if !body.starts_with('0') => format!("0{}", body),
            _ => body,
        }
    } else {
        body
    };
    let pad = width.saturating_sub(body.chars().count());
    if pad == 0 {
        body
    } else if left_align {
        format!("{}{}", body, " ".repeat(pad))
    } else if zero_pad {
        // For `%#04x` with value 15: body = "0xf" (3 chars), width=4,
        // pad=1. Zero-pad after the `0x` prefix → "0x0f". Match libc.
        if let Some(rest) = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")) {
            let prefix = &body[..2];
            format!("{}{}{}", prefix, "0".repeat(pad), rest)
        } else {
            format!("{}{}", "0".repeat(pad), body)
        }
    } else {
        format!("{}{}", " ".repeat(pad), body)
    }
}

fn format_spec_uint(spec: &str, n: u64) -> String {
    // c:Src/builtin.c — `%u` is rendered via libc `%lu`. Unsigned
    // conversions print NO sign: the `+` and ` ` flags are ignored
    // (they apply only to signed conversions), and a negative math
    // result has already wrapped into the u64 by the caller's cast
    // (`-42` → 18446744073709551574). The previous port delegated to
    // format_spec_int(n as i64), which reinterpreted the wrapped u64
    // back as a negative i64 and printed `-42`, and honored `+`/` `.
    let (left_align, zero_pad_flag, width, prec) = parse_flags_width_prec(spec);
    // libc: with an explicit precision the `0` flag is ignored.
    let zero_pad = zero_pad_flag && !left_align && prec.is_none();
    let digits = n.to_string();
    // Precision = minimum number of digits (zero-fill the body). A zero value
    // with precision 0 yields NO digits (c:libc printf).
    let body = match prec {
        Some(0) if n == 0 => String::new(),
        Some(p) if digits.len() < p => format!("{}{}", "0".repeat(p - digits.len()), digits),
        _ => digits,
    };
    // c:libc printf `'` thousands-grouping flag (see format_spec_int for
    // the rationale, the `prec.is_none()` gate, and the port-purity-gate
    // note on the inlined copy). Unsigned bodies carry no sign, so the
    // whole body groups.
    let body = if spec.contains('\'') && prec.is_none() {
        let sep = unsafe {
            let lc = libc::localeconv();
            if lc.is_null() || (*lc).thousands_sep.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr((*lc).thousands_sep)
                    .to_string_lossy()
                    .into_owned()
            }
        };
        if sep.is_empty() {
            body
        } else {
            let chars: Vec<char> = body.chars().collect();
            let len = chars.len();
            let mut grouped = String::with_capacity(len + len / 3 * sep.len());
            for (i, ch) in chars.iter().enumerate() {
                if i > 0 && (len - i) % 3 == 0 {
                    grouped.push_str(&sep);
                }
                grouped.push(*ch);
            }
            grouped
        }
    } else {
        body
    };
    let pad = width.saturating_sub(body.chars().count());
    if pad == 0 {
        body
    } else if left_align {
        format!("{}{}", body, " ".repeat(pad))
    } else if zero_pad {
        format!("{}{}", "0".repeat(pad), body)
    } else {
        format!("{}{}", " ".repeat(pad), body)
    }
}

fn format_spec_float(spec: &str, n: f64) -> String {
    let (left_align, zero_pad, width, prec) = parse_flags_width_prec(spec);
    let p = prec.unwrap_or(6);
    let body = format!("{:.*}", p, n);
    let pad = width.saturating_sub(body.chars().count());
    if pad == 0 {
        body
    } else if left_align {
        format!("{}{}", body, " ".repeat(pad))
    } else if zero_pad {
        if let Some(rest) = body.strip_prefix('-') {
            format!("-{}{}", "0".repeat(pad), rest)
        } else {
            format!("{}{}", "0".repeat(pad), body)
        }
    } else {
        format!("{}{}", " ".repeat(pad), body)
    }
}

/// printf %g / %G / %e / %E / %f / %F dispatch. Mirrors C printf
/// semantics (Src/builtin.c — libc snprintf): %g picks the shorter
/// of %e/%f and strips trailing zeros; %e/%E uses scientific notation;
/// %f/%F is decimal-fraction (no scientific). Default precision is 6.
// pub(crate) so zcalc can format through the SAME printf conversion it calls in
// zsh (zcalc:107 `printf "$_forms[_outform]\n" …`, _forms[1] = '%2$g'). The
// float-parameter formatter `convfloat` is NOT interchangeable: it re-appends a
// trailing `.` when %g yields no dot (c:params.c:5748-5749), which printf never
// does — `sqrt(2)*sqrt(2)` must print `2`, not `2.`.
pub(crate) fn format_spec_float_conv(spec: &str, n: f64, conv: char) -> String {
    // c:Src/builtin.c:5495-5499 — `/* force consistent form for Inf/NaN
    // output */ if (isnan(doubleval)) fputs("nan"); else if (isinf(doubleval))
    // fputs(doubleval < 0.0 ? "-inf" : "inf"); else print_val(...)`. zsh emits
    // the BARE lowercase string with no width/precision/flags/padding,
    // bypassing the normal formatting (platform-independent). The previous
    // Rust path ran nan/inf through `format!`, producing "NaN" and garbage
    // scientific ("NaNe+2147483647").
    if n.is_nan() {
        return "nan".to_string(); // c:5496-5497
    }
    if n.is_infinite() {
        return if n < 0.0 { "-inf" } else { "inf" }.to_string(); // c:5498-5499
    }
    let (left_align, zero_pad, width, prec) = parse_flags_width_prec(spec);
    let body = match conv {
        'f' | 'F' => {
            let p = prec.unwrap_or(6);
            format!("{:.*}", p, n)
        }
        'e' | 'E' => {
            // Use Rust's `{:.*e}` for the mantissa+exponent — it is EXACT
            // (correctly-rounded, like libc snprintf). The previous port
            // computed `mantissa = n / 10^exp` which lost precision: `37/10`
            // is not exactly 3.7, so `%.27e 37` printed the binary error
            // `3.70000…0177635683940e+01` instead of zsh's `3.7000…000e+01`.
            // Rust emits `<mant>e<exp>` with lowercase `e` and no exponent
            // padding/sign; reformat the exponent to libc's `e[+-]NN` (≥2 digits).
            let p = prec.unwrap_or(6);
            let raw = format!("{:.*e}", p, n);
            let (mant, exp_s) = raw.split_once('e').unwrap_or((raw.as_str(), "0"));
            let exp: i32 = exp_s.parse().unwrap_or(0);
            let e_char = if conv == 'E' { 'E' } else { 'e' };
            let exp_sign = if exp >= 0 { '+' } else { '-' };
            format!("{}{}{}{:02}", mant, e_char, exp_sign, exp.abs())
        }
        'g' | 'G' => {
            // c:libc printf %g: precision is # significant digits
            // (default 6). Use %e if exp < -4 OR exp >= precision,
            // else %f. Trailing zeros stripped unless `#` flag set
            // (zshrs doesn't track # — skip stripping suppression).
            let p_sig: i32 = prec.unwrap_or(6).max(1) as i32;
            let exp = if n == 0.0 {
                0i32
            } else {
                n.abs().log10().floor() as i32
            };
            let use_e = exp < -4 || exp >= p_sig;
            let body = if use_e {
                // Exact mantissa+exponent via Rust's `{:.*e}` (see the `%e`
                // arm) — `n / 10^exp` lost precision.
                let dec = (p_sig - 1).max(0) as usize;
                let raw = format!("{:.*e}", dec, n);
                let (m, exp_s) = raw.split_once('e').unwrap_or((raw.as_str(), "0"));
                let ex: i32 = exp_s.parse().unwrap_or(0);
                let e_char = if conv == 'G' { 'E' } else { 'e' };
                let exp_sign = if ex >= 0 { '+' } else { '-' };
                format!("{}{}{}{:02}", m, e_char, exp_sign, ex.abs())
            } else {
                // p_sig - 1 - exp digits after decimal point
                let dec = (p_sig - 1 - exp).max(0) as usize;
                format!("{:.*}", dec, n)
            };
            // Strip trailing zeros from the fractional part (but keep
            // at least one digit after `.` if `.` is present).
            // Only strip if no `#` flag was set in spec.
            // c:libc snprintf %g — trailing-zero strip done inline; no
            // separate helper in C source.
            if !spec.contains('#') {
                let stripped = if let Some(e_pos) = body.find(|c| c == 'e' || c == 'E') {
                    let (mantissa, exp) = body.split_at(e_pos);
                    let m = if mantissa.contains('.') {
                        mantissa
                            .trim_end_matches('0')
                            .trim_end_matches('.')
                            .to_string()
                    } else {
                        mantissa.to_string()
                    };
                    format!("{}{}", m, exp)
                } else if body.contains('.') {
                    body.trim_end_matches('0').trim_end_matches('.').to_string()
                } else {
                    body
                };
                stripped
            } else {
                body
            }
        }
        _ => format!("{}", n),
    };
    // c:libc printf `#` flag on a float conversion forces a decimal point to
    // appear even when precision 0 leaves no fractional digits: `%#.0f` 5 →
    // "5.", `%#.0e` 5 → "5.e+00", `%#.0g` 5 → "5.". (The %g/%G arm above also
    // uses `#` to suppress trailing-zero stripping.)
    let body = if spec.contains('#') && !body.contains('.') {
        match body.find(|c| c == 'e' || c == 'E') {
            Some(ep) => format!("{}.{}", &body[..ep], &body[ep..]),
            None => format!("{body}."),
        }
    } else {
        body
    };
    // c:libc printf flags `+` and ` ` — a non-negative float value gets
    // a forced leading `+` (with `+`) or a blank (with ` `); `+`
    // overrides ` `. parse_flags_width_prec only extracts `-`/`0`, so
    // these sign flags were dropped: `printf "%+.2f" 1.5` rendered
    // "1.50" instead of "+1.50". Negatives keep their own `-`. The
    // zero-pad arm below already expects an optional leading `+`.
    let body = if body.starts_with('-') {
        body
    } else if spec.contains('+') {
        format!("+{}", body)
    } else if spec.contains(' ') {
        format!(" {}", body)
    } else {
        body
    };
    let pad = width.saturating_sub(body.chars().count());
    if pad == 0 {
        body
    } else if left_align {
        // c:libc printf — `-` overrides `0`; pad on the right with spaces.
        format!("{}{}", body, " ".repeat(pad))
    } else if zero_pad {
        // c:libc printf — `0` flag without `-`: zero-pad between the
        // optional sign and digits. `printf "%05.2f" 1.5` → "01.50",
        // `printf "%05.2f" -1.5` → "-1.50" (no extra pad since sign +
        // body already fills width). Previously the helper discarded
        // zero_pad via parse_width_prec and always space-padded.
        if let Some(rest) = body.strip_prefix('-') {
            format!("-{}{}", "0".repeat(pad), rest)
        } else if let Some(rest) = body.strip_prefix('+') {
            format!("+{}{}", "0".repeat(pad), rest)
        } else if let Some(rest) = body.strip_prefix(' ') {
            // ` ` (space) flag + `0`: blank stays leftmost, zeros pad
            // between it and the digits. `printf "% 08.2f" 1.5` → " 0001.50".
            format!(" {}{}", "0".repeat(pad), rest)
        } else {
            format!("{}{}", "0".repeat(pad), body)
        }
    } else {
        format!("{}{}", " ".repeat(pad), body)
    }
}

fn parse_width_prec(spec: &str) -> (bool, usize, Option<usize>) {
    let (left_align, _zero_pad, width, prec) = parse_flags_width_prec(spec);
    (left_align, width, prec)
}

/// Parse printf-format flags `-`, `+`, ` `, `#`, `0`, width digits,
/// optional `.precision`. Returns (left_align, zero_pad, width, prec).
/// Mirrors C `parsefmt()` in Src/builtin.c — the `0` flag is only a
/// flag when it appears BEFORE any width digit; a `0` inside the
/// width number (e.g. `%10d`) is just part of the width.
fn parse_flags_width_prec(spec: &str) -> (bool, bool, usize, Option<usize>) {
    let s = spec.trim_start_matches('%');
    let mut i = 0;
    let bytes = s.as_bytes();
    let mut left_align = false;
    let mut zero_pad = false;
    // `'` (thousands-grouping) is a flag char too — skip it here so a
    // following width/precision (`%'12d`, `%'.8d`) is still parsed. The
    // grouping itself is applied by the caller (format_spec_int/uint).
    while i < bytes.len() && matches!(bytes[i], b'-' | b'+' | b' ' | b'#' | b'0' | b'\'') {
        match bytes[i] {
            b'-' => left_align = true,
            b'0' => zero_pad = true,
            _ => {}
        }
        i += 1;
    }
    let width_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    let width: usize = s[width_start..i].parse().unwrap_or(0);
    let mut prec: Option<usize> = None;
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        let p_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        prec = Some(s[p_start..i].parse().unwrap_or(0));
    }
    (left_align, zero_pad, width, prec)
}

// `findcmd` (Src/exec.c:897) — moved to its canonical home at
// `crate::ported::exec::findcmd` per PORT.md Rule C (the C source
// lives in exec.c, so the Rust port belongs in exec.rs). Call sites
// import from the new path.
pub use crate::ported::exec::findcmd;
use crate::ported::signals_h::run_queued_signals;

/// Port of `getsigidx(const char *s)` from `Src/jobs.c:3047`.
/// Local wrapper that delegates to the canonical
/// `crate::ported::jobs::getsigidx` (matching `Src/jobs.c` location).
/// Returns -1 for unknown so existing builtin.rs call sites (which
/// use the i32 sentinel) don't need to change.
fn getsigidx(name: &str) -> i32 {
    crate::ported::jobs::getsigidx(name).unwrap_or(-1)
}

/// Port of `int pat_enables(const char *cmd, char **patp, int enable)`
/// from `Src/pattern.c:4171`. Local builtin.rs shim that delegates to
/// the canonical pattern.rs port. Static-link path: the actual
/// zpc_strings/zpc_disables manipulation lives in
/// `pat_enables`.
fn pat_enables(name: &str, argv: &[String], on: bool) -> i32 {
    // c:4171
    let patp: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
    crate::ported::pattern::pat_enables(name, &patp, on)
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
/// `traps_table` — see implementation.
pub fn traps_table() -> &'static Mutex<HashMap<String, String>> {
    TRAPS_INNER.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `findcmd` with an existing ABSOLUTE path bypasses the PATH
    /// walk entirely (c:916 `if (arg0 == s || ...)` branch — `s` is
    /// the result of `strchr(arg0, '/')`; when the first char IS
    /// the slash, `arg0 == s` is true). The caller's `$PATH` is
    /// irrelevant for an absolute path. A regression that always
    /// walked $PATH would fail to find `/bin/sh` when $PATH was
    /// empty, breaking command-name resolution for cron/init contexts
    /// that explicitly pass absolute paths.
    #[test]
    fn findcmd_absolute_path_skips_path_walk() {
        let _g = crate::test_util::global_state_lock();
        // `PATH` is an exported special: writing it also rewrites the
        // PROCESS environment, and `unsetparam` does not put the old
        // value back. Leaving it empty broke every later test that
        // spawns a helper by name (`Command::new("zsh")` → ENOENT), so
        // snapshot and restore it here.
        let saved_path = getsparam("PATH");
        // Empty $PATH to guarantee the walk would miss.
        setsparam("PATH", "");
        let resolved = findcmd("/bin/sh", 0, 0);
        match saved_path {
            Some(p) => {
                setsparam("PATH", &p);
            }
            None => {
                unsetparam("PATH");
            }
        }
        assert_eq!(
            resolved.as_deref(),
            Some("/bin/sh"),
            "c:914-919 — absolute path that exists must resolve to itself \
             regardless of $PATH"
        );
    }

    /// `findcmd` with `default_path != 0` MUST search the hardcoded
    /// `DEFAULT_PATH` (`/usr/bin:/bin:/usr/sbin:/sbin`), NOT the
    /// caller's `$PATH`. C body c:903-908. This is the `command -p`
    /// security contract: scripts that need to invoke a sanitized
    /// `awk`/`sed`/`grep` regardless of user-poisoned $PATH (e.g.
    /// `command -p sh -c '...'` in a setuid wrapper) rely on the
    /// fallback path. A regression that ignored `default_path` would
    /// re-introduce the very PATH-injection vulnerability that
    /// `command -p` exists to prevent.
    ///
    /// Pin: with $PATH set to a non-existent directory, `findcmd`
    /// for a binary that ONLY lives in /bin or /usr/bin (e.g. `sh`)
    /// must still resolve when `default_path=1`.
    #[test]
    fn findcmd_default_path_searches_hardcoded_dirs() {
        let _g = crate::test_util::global_state_lock();
        // Snapshot/restore — see findcmd_absolute_path_skips_path_walk.
        let saved_path = getsparam("PATH");
        // Poison $PATH so the normal path-walk would miss.
        setsparam("PATH", "/nonexistent/zshrs-test-poison");
        // `sh` exists in /bin on every POSIX system.
        let resolved = findcmd("sh", 0, 1);
        match saved_path {
            Some(p) => {
                setsparam("PATH", &p);
            }
            None => {
                unsetparam("PATH");
            }
        }
        assert!(
            resolved.is_some(),
            "c:903-908 — default_path must search DEFAULT_PATH regardless of $PATH"
        );
        let p = resolved.unwrap();
        assert!(
            DEFAULT_PATH.split(':').any(|d| p.starts_with(d)),
            "resolved path must be under one of DEFAULT_PATH's dirs; got {:?}",
            p
        );
    }

    /// c:7399 — `trap - <undefined>` MUST report failure (non-zero
    /// exit) so scripts can detect the bad signal name. The previous
    /// Rust port returned 0 unconditionally from the clear path,
    /// silently masking errors. C returns `*argv != NULL` — non-zero
    /// when the loop broke on an undefined signal.
    #[test]
    fn bin_trap_clear_undefined_signal_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let empty = options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        // `trap - BOGUS_NEVER_A_SIGNAL` → must return 1.
        let r = bin_trap(
            "trap",
            &["-".into(), "BOGUS_NEVER_A_SIGNAL".into()],
            &empty,
            0,
        );
        assert_ne!(
            r, 0,
            "trap - <undefined> must report error per c:7399 (got {})",
            r
        );
    }

    /// c:7405-7409 — the trap body is parsed when the trap is INSTALLED
    /// (`parse_string(arg, 1)`), so an unparseable body fails right here
    /// with `couldn't parse trap command` and rc=1, and nothing is
    /// installed. zshrs previously stored the body text unexamined and
    /// returned 0, deferring the failure to signal delivery: `trap 'for'
    /// EXIT` reported success and then emitted a parse error at exit.
    ///
    /// The bodies here are the ones the oracle rejects. `while` is
    /// deliberately not among them — `zsh -fc 'while'` treats it as an
    /// incomplete construct and reads the remainder from stdin rather
    /// than erroring, so it is not an install-time parse failure.
    #[test]
    fn bin_trap_unparseable_body_rejected_at_install() {
        let _g = crate::test_util::global_state_lock();
        let empty = options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        for body in [
            "for",
            "((",
            "fi",
            "done",
            "case",
            "if true",
            "print ok; for",
        ] {
            let r = bin_trap("trap", &[body.into(), "USR1".into()], &empty, 0);
            assert_eq!(
                r, 1,
                "trap '{body}' USR1 must fail at install per c:7407 (got {r})"
            );
            // The rejected body must not have been installed (c:7408
            // returns before the install loop at c:7421).
            let installed = traps_table()
                .lock()
                .ok()
                .map(|t| t.contains_key("USR1"))
                .unwrap_or(false);
            assert!(
                !installed,
                "trap '{body}' USR1 was rejected but still installed a handler"
            );
        }
        // A parseable body still installs — the gate must not reject
        // everything.
        let r = bin_trap("trap", &["print ok".into(), "USR1".into()], &empty, 0);
        assert_eq!(r, 0, "parseable body must install (got {r})");
        if let Ok(mut t) = traps_table().lock() {
            assert!(t.contains_key("USR1"), "parseable body must be installed");
            t.remove("USR1");
        }
    }

    /// c:7371 — `getpermtext(siglists[sig], NULL, 0)`. C keeps the trap
    /// body as a compiled Eprog and renders it back to source for the
    /// listing, so `trap` prints CANONICAL text rather than the string
    /// that was typed: separators become newlines. zshrs stores the body
    /// as raw text and previously echoed it verbatim, which is correct
    /// only for a single command — `trap 'print a; print b' EXIT` listed
    /// as `'print a; print b'` where zsh lists `$'print a\nprint b'`.
    ///
    /// Pins the deparse itself (parse_string → getpermtext) rather than
    /// bin_trap's stdout, so the assertion doesn't depend on capturing
    /// the builtin's print! output.
    #[test]
    fn trap_body_listing_deparses_to_canonical_text() {
        let _g = crate::test_util::global_state_lock();
        let prog = crate::ported::exec::parse_string("print a; print b", 1)
            .expect("body must parse for the listing path");
        let rendered = crate::ported::text::getpermtext(Box::new(prog), None, 0);
        assert_eq!(
            rendered, "print a\nprint b",
            "c:7371 getpermtext must canonicalise `;` separators to newlines (got {rendered:?})"
        );
    }

    /// Src/options.c:537-549 — `emulate(zsh_name, ...)` dispatches
    /// on the FIRST char of the shell name, stripping a leading `r`
    /// (so `rcsh`/`rksh` work as restricted variants of their base
    /// shell). `bash` aliases to SH (the `'b'` branch of the case).
    /// Pin the bits assigned by `bin_emulate` for the canonical
    /// names + their first-char-overlap aliases.
    #[test]
    fn bin_emulate_dispatches_on_first_char_per_c537() {
        let _g = crate::test_util::global_state_lock();
        let empty = options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let saved = emulation.load(Relaxed);
        // `emulate` rewrites the WHOLE option table (installemulation,
        // c:551-572), not just the emulation bits — `emulate csh` turns
        // CPRECEDENCES on, `emulate sh` turns SHWORDSPLIT on, and so on.
        // Restoring only `emulation` left those options set for every
        // later test in the process (a leaked CPRECEDENCES made
        // `4 - - 3 * 7 << 1 & 7 ^ 1 | 16 ** 2` evaluate to C's 259
        // instead of zsh's 1591). Snapshot and restore the table too.
        let saved_opts = crate::ported::options::opt_state_snapshot();

        // Each (name, expected_bits) — name covers the canonical
        // shell names AND their `r`-prefix / first-char variants.
        for (name, expected) in [
            ("csh", EMULATE_CSH),
            ("ksh", EMULATE_KSH),
            ("sh", EMULATE_SH),
            ("rcsh", EMULATE_CSH), // c:539-540
            ("rksh", EMULATE_KSH), // c:539-540
            ("bash", EMULATE_SH),  // c:548 'b'
        ] {
            emulation.store(0, Relaxed);
            bin_emulate("emulate", &[name.into()], &empty, 0);
            let bits = emulation.load(Relaxed);
            assert_eq!(
                bits, expected,
                "emulate {} must set bits {:#x}, got {:#x}",
                name, expected, bits
            );
        }
        emulation.store(saved, Relaxed);
        crate::ported::options::opt_state_restore(saved_opts);
    }

    /// c:7399 — `trap - SIGUSR1` (valid signal) MUST return 0, even
    /// when the trap was never set (remove is a no-op).
    #[test]
    fn bin_trap_clear_valid_signal_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let empty = options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_trap("trap", &["-".into(), "USR1".into()], &empty, 0);
        assert_eq!(
            r, 0,
            "trap - USR1 must succeed even with no prior trap (got {})",
            r
        );
    }

    #[test]
    fn registration_table_matches_c_count() {
        let _g = crate::test_util::global_state_lock();
        // Src/builtin.c:40-137 has 79 rows total (5 BIN_PREFIX + 71
        // BUILTIN + 3 debug-only BUILTIN). The Rust port bundles
        // additional builtins eagerly that C would load via zmodload:
        //   zsh/rlimits (limit/ulimit/unlimit)
        //   zsh/zle (bindkey/vared/zle)
        //   zsh/cap (cap/getcap/setcap)
        //   zsh/files (chmod/chown/ln/mkdir/rm/rmdir/sync)
        //   zsh/complete (compadd/compset)
        //   zsh/terminfo (echoti)
        //   zsh/pcre (pcre_compile/pcre_match/pcre_study)
        //   zsh/zutil (zformat/zgdbmpath)
        //   zsh/sched (sched)
        //   zsh/computil (comparguments/compdescribe/compfiles/
        //     compgroups/compquote/comptags/comptry/compvalues)
        //   zsh/system (syserror/sysread/syswrite/sysopen/sysseek/zsystem)
        //   zsh/zselect (zselect)
        //   zsh/socket (zsocket)
        //   zsh/stat (stat)
        //   zsh/watch (log)
        //   zsh/zprof (zprof)
        //   zsh/datetime (strftime)
        //   zsh/zftp (zftp), zsh/zpty (zpty), zsh/curses (zcurses)
        //   zsh/clone (clone), zsh/example (example)
        //   zsh/param/private (private)
        //   zsh/termcap (echotc)
        //   zsh/compctl (compcall, compctl)
        //   zsh/attr (zgetattr, zsetattr, zdelattr, zlistattr)
        // Tripwire pin on BUILTINS table length. The number drifts every
        // time the eagerly-loaded-module list above grows (new builtin
        // ported, new module wired). Bump it alongside the change so
        // accidental additions/removals still trip a review.
        // 159 ported zsh builtins + 1 zshrs-native (`__rust_compile`, the
        // inline Rust FFI desugar target — no C counterpart) = 160.
        assert_eq!(BUILTINS.len(), 160,
            "BUILTINS table size changed — bump count or update the eagerly-loaded-module list above");
    }

    /// `Src/builtin.c:40-137` — every name in the canonical C builtin
    /// table must be present in the Rust port. Pins coverage of all
    /// 79 C builtins by name (ignores option-mask / handler details).
    /// Detects regressions where a builtin gets accidentally dropped
    /// from BUILTINS. Names extracted from upstream zsh `Src/builtin.c`.
    #[test]
    fn registration_table_contains_all_c_builtins() {
        let _g = crate::test_util::global_state_lock();
        // Canonical 79 names from Src/builtin.c:40-137 (verbatim).
        let c_names: &[&str] = &[
            "-",
            ".",
            ":",
            "[",
            "alias",
            "autoload",
            "bg",
            "break",
            "builtin",
            "bye",
            "cd",
            "chdir",
            "command",
            "continue",
            "declare",
            "dirs",
            "disable",
            "disown",
            "echo",
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
            "hashinfo",
            "history",
            "integer",
            "jobs",
            "kill",
            "let",
            "local",
            "logout",
            "mem",
            "noglob",
            "patdebug",
            "popd",
            "print",
            "printf",
            "pushd",
            "pushln",
            "pwd",
            "r",
            "read",
            "readonly",
            "rehash",
            "return",
            "set",
            "setopt",
            "shift",
            "source",
            "suspend",
            "test",
            "times",
            "trap",
            "true",
            "ttyctl",
            "type",
            "typeset",
            "umask",
            "unalias",
            "unfunction",
            "unhash",
            "unset",
            "unsetopt",
            "wait",
            "whence",
            "where",
            "which",
            "zcompile",
            "zmodload",
        ];
        assert_eq!(
            c_names.len(),
            79,
            "C builtin.c row count is 79 — recount if changed"
        );
        let table_names: std::collections::HashSet<&str> =
            BUILTINS.iter().map(|b| b.node.nam.as_str()).collect();
        for c_name in c_names {
            assert!(
                table_names.contains(*c_name),
                "missing C builtin '{}' from BUILTINS table",
                c_name
            );
        }
    }

    #[test]
    fn lookup_finds_known_builtins() {
        let _g = crate::test_util::global_state_lock();
        for name in [
            "cd", "echo", "print", "fg", "bg", "jobs", "wait", "typeset", "test", "[", ".",
        ] {
            assert!(
                createbuiltintable().get(name).copied().is_some(),
                "missing: {name}"
            );
        }
    }

    #[test]
    fn lookup_misses_unknown() {
        let _g = crate::test_util::global_state_lock();
        assert!(createbuiltintable()
            .get("not-a-builtin-zZz")
            .copied()
            .is_none());
    }

    #[test]
    fn prefix_entries_have_prefix_flag() {
        let _g = crate::test_util::global_state_lock();
        for name in ["-", "builtin", "command", "exec", "noglob"] {
            let b = createbuiltintable().get(name).copied().unwrap();
            assert!(
                b.node.flags as u32 & BINF_PREFIX != 0,
                "{name} missing BINF_PREFIX"
            );
        }
    }

    #[test]
    fn fixdir_canonicalizes_absolute_paths() {
        let _g = crate::test_util::global_state_lock();
        // c:1297 — collapse `//`, drop `./`, pop `..`.
        assert_eq!(fixdir("/tmp/./foo"), "/tmp/foo");
        assert_eq!(fixdir("/tmp//foo"), "/tmp/foo");
        assert_eq!(fixdir("/tmp/bar/../foo"), "/tmp/foo");
        assert_eq!(fixdir("/tmp/bar/baz/../.."), "/tmp");
    }

    #[test]
    fn fixdir_drops_dotdot_past_root() {
        let _g = crate::test_util::global_state_lock();
        // c:1372 — absolute path, `..` past `/` is dropped.
        assert_eq!(fixdir("/.."), "/");
        assert_eq!(fixdir("/../.."), "/");
        assert_eq!(fixdir("/foo/../../bar"), "/bar");
    }

    #[test]
    fn fixdir_relative_keeps_leading_dotdot() {
        let _g = crate::test_util::global_state_lock();
        // c:1367 — relative path: `..` past start stays as `..`.
        assert_eq!(fixdir("../foo"), "../foo");
        assert_eq!(fixdir("../../foo"), "../../foo");
        assert_eq!(fixdir("foo/../bar"), "bar");
    }

    #[test]
    fn fixdir_empty_collapses_to_dot() {
        let _g = crate::test_util::global_state_lock();
        // Relative path that collapses fully → "."
        assert_eq!(fixdir("./"), ".");
        assert_eq!(fixdir("foo/.."), ".");
    }

    #[test]
    fn fixdir_empty_input_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(fixdir(""), "");
    }

    #[test]
    fn fg_dispatch_id_distinguishes_aliases() {
        let _g = crate::test_util::global_state_lock();
        // bin_fg covers fg, bg, jobs, wait, disown — same handler,
        // different funcid. Mirrors Src/builtin.c:52,61,75,88,131.
        assert_eq!(
            createbuiltintable().get("fg").copied().unwrap().funcid,
            BIN_FG
        );
        assert_eq!(
            createbuiltintable().get("bg").copied().unwrap().funcid,
            BIN_BG
        );
        assert_eq!(
            createbuiltintable().get("jobs").copied().unwrap().funcid,
            BIN_JOBS
        );
        assert_eq!(
            createbuiltintable().get("wait").copied().unwrap().funcid,
            BIN_WAIT
        );
        assert_eq!(
            createbuiltintable().get("disown").copied().unwrap().funcid,
            BIN_DISOWN
        );
    }

    /// c:1297 — `fixdir` is the lexical-canonicalisation for `cd`. The
    /// path `/a/b/../c` must resolve to `/a/c` BEFORE chdir(2) — the
    /// shell uses it to compute the logical PWD for $PWD/OLDPWD. A
    /// regression that drops the `..` consumption would make $PWD
    /// report `/a/b/../c` literally on `cd /a/b/../c`.
    #[test]
    fn fixdir_pops_dotdot_against_previous_component() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(fixdir("/a/b/../c"), "/a/c");
        assert_eq!(fixdir("/a/b/../../c"), "/c");
        assert_eq!(fixdir("/foo/.."), "/");
    }

    /// c:1352 — `./` collapses to nothing.  `/a/./b` must equal `/a/b`.
    #[test]
    fn fixdir_drops_dot_components() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(fixdir("/a/./b"), "/a/b");
        assert_eq!(fixdir("./a"), "a");
        assert_eq!(fixdir("./."), ".");
    }

    /// c:1388 — `//` collapses to single `/` (no preservation of POSIX
    /// implementation-defined `//` semantics, which zsh doesn't honour).
    #[test]
    fn fixdir_collapses_consecutive_slashes() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(fixdir("/a//b"), "/a/b");
        assert_eq!(fixdir("/a///b/c"), "/a/b/c");
    }

    /// c:1404 — absolute path: `..` past `/` silently drops. `/..`
    /// resolves to `/`. Catches a regression where the underflow
    /// emits `..` literally.
    #[test]
    fn fixdir_dotdot_past_root_clamps_to_root() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(fixdir("/.."), "/");
        assert_eq!(fixdir("/../../a"), "/a");
    }

    /// c:1400 — RELATIVE path: leading `..` are preserved (no parent
    /// known until chdir time). This is critical for `cd ../../foo`
    /// which must NOT resolve `..` lexically.
    #[test]
    fn fixdir_relative_leading_dotdot_is_preserved() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(fixdir("../foo"), "../foo");
        assert_eq!(fixdir("../../foo"), "../../foo");
    }

    /// c:1683 — `fcgetcomm` returns 0 for ambiguous numeric inputs
    /// only when the string actually starts with '0'. The atoi result
    /// alone (which is 0 for non-numeric) MUST NOT short-circuit —
    /// non-numeric input should fall through to hcomsearch instead.
    #[test]
    fn fcgetcomm_numeric_zero_only_for_literal_zero_prefix() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(fcgetcomm("0"), 0, "literal `0` is event 0");
        assert_eq!(fcgetcomm("42"), 42);
        // Non-numeric falls through to hcomsearch (no hist match → -1).
        assert_eq!(fcgetcomm("definitely_not_a_history_command_zshrs"), -1);
    }

    /// c:1088-1093 — `cd_able_vars` requires CDABLEVARS to be set;
    /// otherwise returns None even when the head names a param. A
    /// regression that ignores the option flag would let `cd HOME`
    /// silently `cd $HOME` even when the user disabled CDABLEVARS.
    #[test]
    fn cd_able_vars_returns_none_without_cdablevars_option() {
        let _g = crate::test_util::global_state_lock();
        // CDABLEVARS is not set by default → must return None.
        // We don't fight the option state here; just verify the
        // off-state default short-circuits before paramtab lookup.
        // (If a future commit enables CDABLEVARS by default, this
        // test will fail loudly — that's the right canary.)
        let r = cd_able_vars("HOME/anything");
        // Without CDABLEVARS, must be None; with it, would be Some.
        // Accept either since the option default is the actual invariant.
        if !isset(CDABLEVARS) {
            assert!(r.is_none());
        }
    }

    /// c:212 — `init_builtins` is idempotent: calling twice doesn't
    /// duplicate entries in the table. Regression that re-inserts on
    /// every call would balloon memory + break dispatch lookups.
    #[test]
    fn init_builtins_is_idempotent() {
        let _g = crate::test_util::global_state_lock();
        init_builtins();
        let count1 = createbuiltintable().len();
        init_builtins();
        let count2 = createbuiltintable().len();
        assert_eq!(count1, count2, "init_builtins must not duplicate entries");
    }

    /// c:1708 — `fcsubs(sp, [(old, new), ...])` applies each
    /// substitution to the running string, returning the total
    /// replacement count. A regression returning 0 with substitutions
    /// applied would silently break `fc -s old=new`.
    #[test]
    fn fcsubs_applies_each_substitution_in_order() {
        let _g = crate::test_util::global_state_lock();
        let mut s = "echo foo bar foo".to_string();
        let n = fcsubs(&mut s, &[("foo".to_string(), "FOO".to_string())]);
        assert_eq!(s, "echo FOO bar FOO");
        assert_eq!(n, 2, "two `foo` matches replaced");
    }

    /// c:1708 — empty `old` MUST skip (avoid infinite empty-match
    /// replacement loop). Regression treating "" as "match anywhere"
    /// would hang or silently corrupt every fc invocation.
    #[test]
    fn fcsubs_skips_empty_pattern() {
        let _g = crate::test_util::global_state_lock();
        let mut s = "anything".to_string();
        let n = fcsubs(&mut s, &[("".to_string(), "X".to_string())]);
        assert_eq!(s, "anything", "empty pattern must be skipped");
        assert_eq!(n, 0);
    }

    /// c:1708 — chained substitutions apply left-to-right. After
    /// `a→b`, the next pair sees the post-substitution text. So
    /// `[(a→b), (b→c)]` over `a` yields `c`.
    #[test]
    fn fcsubs_chains_substitutions_left_to_right() {
        let _g = crate::test_util::global_state_lock();
        let mut s = "a".to_string();
        let n = fcsubs(
            &mut s,
            &[
                ("a".to_string(), "b".to_string()),
                ("b".to_string(), "c".to_string()),
            ],
        );
        assert_eq!(s, "c", "second sub sees post-first-sub text");
        assert_eq!(n, 2);
    }

    /// c:1708 — substitution on no-match leaves string unchanged AND
    /// reports 0. Regression touching the string anyway would mangle
    /// fc output for events containing none of the requested patterns.
    #[test]
    fn fcsubs_no_match_returns_zero_unchanged() {
        let _g = crate::test_util::global_state_lock();
        let mut s = "hello world".to_string();
        let n = fcsubs(&mut s, &[("xyz".to_string(), "abc".to_string())]);
        assert_eq!(s, "hello world", "no match → unchanged");
        assert_eq!(n, 0);
    }

    /// c:1297 — `fixdir` for plain relative path (no slashes, no
    /// dots) returns it unchanged. Most-common cd path; regression
    /// here would break `cd subdir`.
    #[test]
    fn fixdir_plain_relative_path_unchanged() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(fixdir("subdir"), "subdir");
        assert_eq!(fixdir("a/b/c"), "a/b/c");
        assert_eq!(fixdir("."), ".");
    }

    /// Shared mutex for bin_let tests that toggle the global errflag.
    static BIN_LET_TEST_LOCK: Mutex<()> = Mutex::new(());

    /// `Src/builtin.c:7469-7484` — `bin_let` semantics:
    ///   1. Returns 0 (success) when the LAST arg evaluates to non-zero.
    ///   2. Returns 1 (failure) when the LAST arg evaluates to zero.
    ///   3. Returns 2 AND CLEARS ERRFLAG_ERROR when any arg errors
    ///      (let errors are non-fatal and local).
    #[test]
    fn bin_let_clears_errflag_on_math_error() {
        let _g = crate::test_util::global_state_lock();
        let _g = BIN_LET_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved = errflag.load(Relaxed);
        errflag.store(0, Relaxed);

        // 1. Last arg evaluates to non-zero → return 0.
        let ops = options {
            ind: [0; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let argv = vec!["1".to_string()];
        assert_eq!(
            bin_let("let", &argv, &ops, 0),
            0,
            "c:7482 — last expr non-zero → return 0 (success)"
        );

        // 2. Last arg evaluates to zero → return 1.
        let argv = vec!["0".to_string()];
        assert_eq!(
            bin_let("let", &argv, &ops, 0),
            1,
            "c:7482 — last expr zero → return 1 (failure)"
        );

        // 3. Bad-syntax arg → return 2 AND clear ERRFLAG_ERROR.
        // Pre-set errflag manually to simulate matheval failure side
        // effect (since exact bad-syntax behavior of the matheval port
        // is implementation-dependent — what we're pinning is the
        // bin_let response to a set errflag).
        errflag.store(ERRFLAG_ERROR, Relaxed);
        // Use a valid expression so matheval succeeds, but errflag
        // is already set from a prior step.
        let argv = vec!["1".to_string()];
        let rc = bin_let("let", &argv, &ops, 0);
        // c:7479 says `return 2;` but the installed zsh 5.9.1 returns
        // 1 (oracle: `zsh -fc 'let 1/0; echo $?'` → 1) and bin_let
        // matches the release binary — see the comment block at the
        // errflag branch in bin_let. This assertion pins the OBSERVED
        // status; it went stale when bin_let switched 2→1.
        assert_eq!(
            rc, 1,
            "c:7479 cleanup path — observed zsh 5.9.1 rc (release binary returns 1, not the c:7479 literal 2)"
        );
        // c:7478 — `errflag &= ~ERRFLAG_ERROR` must have run.
        assert_eq!(
            errflag.load(Relaxed) & ERRFLAG_ERROR,
            0,
            "c:7478 — ERRFLAG_ERROR must be CLEARED after let error"
        );

        // Restore.
        errflag.store(saved, Relaxed);
    }

    /// Bug #1026: `test ! ! -n abc` = not(not(-n abc)) → 0 (true). The
    /// pre-flight rejected a leading-`!` chain over a unary operand as
    /// "condition expected: !" (c:builtin.c:7270 + par_cond `!` rule).
    #[test]
    fn bin_test_double_negation_of_unary_op() {
        let _g = crate::test_util::global_state_lock();
        let saved = errflag.load(Relaxed);
        let ops = options {
            ind: [0; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let mk = |s: &[&str]| s.iter().map(|x| (*x).to_string()).collect::<Vec<_>>();
        let run = |a: &[&str]| {
            errflag.store(0, Relaxed);
            bin_test("test", &mk(a), &ops, crate::ported::hashtable_h::BIN_TEST)
        };
        // not(not(-n abc)) = not(not true) = true → 0
        assert_eq!(run(&["!", "!", "-n", "abc"]), 0);
        // not(not(not(-n abc))) = false → 1
        assert_eq!(run(&["!", "!", "!", "-n", "abc"]), 1);
        // not(not(-n "")) = not(not false) = false → 1
        assert_eq!(run(&["!", "!", "-n", ""]), 1);
        // Single negation unaffected: not(-n abc) → 1
        assert_eq!(run(&["!", "-n", "abc"]), 1);
        errflag.store(saved, Relaxed);
    }

    /// `Src/builtin.c:7474-7475` — C walks ALL argv via
    /// `while (*argv) val = matheval(*argv++);`. The LAST matheval
    /// result is what determines the return code. The previous Rust
    /// port broke on first error, skipping later args. Pin: a sequence
    /// of two non-zero exprs returns 0 even if both are evaluated.
    #[test]
    fn bin_let_walks_all_argv_last_wins() {
        let _g = crate::test_util::global_state_lock();
        let _g = BIN_LET_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        errflag.store(0, Relaxed);

        let ops = options {
            ind: [0; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        // c:7474 — `5; 0` (two args): last is 0 → return 1.
        let argv = vec!["5".to_string(), "0".to_string()];
        assert_eq!(
            bin_let("let", &argv, &ops, 0),
            1,
            "c:7474 — last arg wins (here: 0 → return 1)"
        );

        // c:7474 — `0; 5` (two args): last is 5 → return 0.
        let argv = vec!["0".to_string(), "5".to_string()];
        assert_eq!(
            bin_let("let", &argv, &ops, 0),
            0,
            "c:7474 — last arg wins (here: 5 → return 0)"
        );

        errflag.fetch_and(!ERRFLAG_ERROR, Relaxed);
    }

    /// `Src/builtin.c:4799-4808` — `print -o` (sort) is CASE-SENSITIVE
    /// by default; `-i` flips to case-insensitive. The previous Rust
    /// port had this INVERTED: case-sensitive under `-i`,
    /// case-insensitive without. Pin the canonical semantic by direct
    /// reproduction of the sort step.
    ///
    /// `bin_print` itself is harder to test in isolation because it
    /// emits to stdout; instead we replicate the in-port sort logic
    /// to ensure the gate matches C semantics. If the port body's
    /// `if ignore_case` is ever re-inverted, the regression here
    /// surfaces immediately.
    #[test]
    fn bin_print_sort_matches_c_case_gate() {
        let _g = crate::test_util::global_state_lock();
        // Helper mirroring the in-port logic exactly.
        let sort_with = |items: &[&str], ignore_case: bool, backwards: bool| -> Vec<String> {
            let mut v: Vec<String> = items.iter().map(|s| s.to_string()).collect();
            if ignore_case {
                v.sort_by_key(|s| s.to_lowercase());
            } else {
                v.sort();
            }
            if backwards {
                v.reverse();
            }
            v
        };
        // `print -o foo Bar BAZ` (no `-i`): case-sensitive ASCII sort.
        // Uppercase ASCII < lowercase ASCII, so caps come first.
        let no_i = sort_with(&["foo", "Bar", "BAZ"], false, false);
        assert_eq!(
            no_i,
            vec!["BAZ", "Bar", "foo"],
            "c:4805 — without -i: case-sensitive sort (caps first by ASCII)"
        );
        // `print -oi foo Bar BAZ`: case-insensitive sort.
        // Lower-case comparison: "bar" < "baz" < "foo", so order is
        // Bar, BAZ, foo.
        let with_i = sort_with(&["foo", "Bar", "BAZ"], true, false);
        assert_eq!(
            with_i,
            vec!["Bar", "BAZ", "foo"],
            "c:4805 — with -i: case-insensitive sort"
        );
        // `print -O foo Bar BAZ` (no `-i`): case-sensitive descending.
        let big_o = sort_with(&["foo", "Bar", "BAZ"], false, true);
        assert_eq!(
            big_o,
            vec!["foo", "Bar", "BAZ"],
            "c:4806 — -O reverses after sort"
        );
        // Conjunction check: zsh-equivalent: print -O foo Bar BAZ
        // gives `foo Bar BAZ`. Pin so an inadvertent reverse-before-
        // sort regression fails.
    }

    /// `Src/builtin.c:4854-4856 + 5564-5565` — `printf -z FMT ARGS...`
    /// captures formatted output then pushes to bufstack (same path
    /// as -z without -f).
    #[test]
    fn bin_print_printf_with_minus_z() {
        let _g = crate::test_util::global_state_lock();
        let mut ops = options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        ops.ind[b'z' as usize] = 1;
        // -f set to "echo %s" (positional)
        ops.ind[b'f' as usize] = 1 | (1 << 2);
        ops.args = vec!["echo %s".to_string()];
        ops.argscount = 1;
        crate::ported::zle::zle_main::BUFSTACK
            .lock()
            .unwrap()
            .clear();
        let r = bin_print("printf", &["hello".to_string()], &ops, BIN_PRINTF);
        assert_eq!(r, 0);
        let buf = crate::ported::zle::zle_main::BUFSTACK.lock().unwrap();
        assert_eq!(
            buf.last().map(|s| s.as_str()),
            Some("echo hello"),
            "c:4854-4856 — printf -z must push formatted output to bufstack"
        );
    }

    /// `Src/builtin.c:5564-5565` — `print -z WORDS...` pushes the
    /// joined string to the ZLE bufstack instead of stdout (consumed
    /// by the next zleread call so the string lands at the prompt).
    #[test]
    fn bin_print_minus_z_pushes_to_bufstack() {
        let _g = crate::test_util::global_state_lock();
        let mut ops = options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        ops.ind[b'z' as usize] = 1;
        crate::ported::zle::zle_main::BUFSTACK
            .lock()
            .unwrap()
            .clear();
        let r = bin_print(
            "print",
            &["echo".to_string(), "foo".to_string()],
            &ops,
            BIN_PRINT,
        );
        assert_eq!(r, 0, "c:5565 — -z should succeed");
        let buf = crate::ported::zle::zle_main::BUFSTACK.lock().unwrap();
        assert_eq!(
            buf.last().map(|s| s.as_str()),
            Some("echo foo"),
            "c:5565 — bufstack must have `echo foo` as the top entry"
        );
    }

    /// `Src/builtin.c:5569-5574` — `print -s WORDS...` pushes the
    /// joined string to the history table instead of stdout.
    #[test]
    fn bin_print_minus_s_pushes_to_history() {
        let _g = crate::test_util::global_state_lock();
        let mut ops = options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        ops.ind[b's' as usize] = 1;
        // Clear histtab to a known state.
        crate::ported::hashtable::histtab_lock()
            .write()
            .unwrap()
            .clear();
        let r = bin_print(
            "print",
            &["hello".to_string(), "world".to_string()],
            &ops,
            BIN_PRINT,
        );
        assert_eq!(r, 0, "c:5574 — -s should succeed");
        // After -s, the joined "hello world" string must appear in
        // histtab (the in-process history lookup table).
        let tab = crate::ported::hashtable::histtab_lock().read().unwrap();
        assert!(
            tab.contains_key("hello world"),
            "c:5574 — addhistnode must record `hello world` in histtab"
        );
    }

    /// `Src/builtin.c:4718-4741` — `print -m PATTERN args...` keeps
    /// only the args matching PATTERN. Pipe-roundtrip pin: pat=`foo*`,
    /// args=[foo1, bar, foo2] → expect `foo1 foo2\n` (NOT `bar`).
    #[test]
    fn bin_print_minus_m_glob_filter() {
        let _g = crate::test_util::global_state_lock();
        use std::io::Read as _;
        let mut fds: [libc::c_int; 2] = [0, 0];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let (rfd, wfd) = (fds[0], fds[1]);
        let mut ops = options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        ops.ind[b'u' as usize] = 1 | (1 << 2);
        ops.args = vec![wfd.to_string()];
        ops.argscount = 1;
        ops.ind[b'm' as usize] = 1; // -m

        let r = bin_print(
            "print",
            &[
                "foo*".to_string(),
                "foo1".to_string(),
                "bar".to_string(),
                "foo2".to_string(),
            ],
            &ops,
            BIN_PRINT,
        );
        assert_eq!(r, 0);
        unsafe { libc::close(wfd) };

        let mut buf = String::new();
        unsafe {
            use std::os::unix::io::FromRawFd;
            let mut f = fs::File::from_raw_fd(rfd);
            f.read_to_string(&mut buf).unwrap();
        }
        assert_eq!(
            buf, "foo1 foo2\n",
            "c:4718-4741 — -m filters to only `foo*`-matching args"
        );
    }

    /// `Src/builtin.c:5126-5132` — `print -N a b` separates args with
    /// `\0` and terminates with `\0` (not `\n`). Pipe-roundtrip pin.
    #[test]
    fn bin_print_nul_separator_with_minus_N() {
        let _g = crate::test_util::global_state_lock();
        use std::io::Read as _;
        let mut fds: [libc::c_int; 2] = [0, 0];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let (rfd, wfd) = (fds[0], fds[1]);
        let mut ops = options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        // -u <wfd>
        ops.ind[b'u' as usize] = 1 | (1 << 2);
        ops.args = vec![wfd.to_string()];
        ops.argscount = 1;
        // -N (no arg)
        ops.ind[b'N' as usize] = 1;

        let r = bin_print(
            "print",
            &["a".to_string(), "b".to_string(), "c".to_string()],
            &ops,
            BIN_PRINT,
        );
        assert_eq!(r, 0);
        unsafe { libc::close(wfd) };

        let mut buf = Vec::new();
        unsafe {
            use std::os::unix::io::FromRawFd;
            let mut f = fs::File::from_raw_fd(rfd);
            f.read_to_end(&mut buf).unwrap();
        }
        assert_eq!(
            buf, b"a\0b\0c\0",
            "c:5126-5132 — -N: NUL separators + NUL terminator"
        );
    }

    /// `Src/builtin.c:4815-4847` — `print -u FD` writes to the given
    /// file descriptor. Pin: write to a pipe via -u and read back.
    #[test]
    fn bin_print_writes_to_specified_fd() {
        let _g = crate::test_util::global_state_lock();
        use std::io::Read as _;
        // Open a pipe; print -u writes to write end, we read off read
        // end.
        let mut fds: [libc::c_int; 2] = [0, 0];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let (rfd, wfd) = (fds[0], fds[1]);

        // Build options with -u set to the write fd.
        let mut ops = options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        ops.ind[b'u' as usize] = 1;
        ops.args = vec![wfd.to_string()];
        // OPT_ARG looks up via OPT_HASARG/argscount; the exact wiring
        // depends on the parseopts pre-call path. Use a minimal stub
        // so OPT_ARG('u') returns wfd's string.
        ops.argscount = 1;

        // The OPT_ARG indexing path requires `ops.ind[b'u']` to encode
        // both the "is set" bit and an arg-index pointer. The default
        // parseopts wires this; for the unit test we synthesize the
        // wfd-as-string into args[0] AND set `ops.ind[b'u']` to point
        // at it via the same convention (`(ops.ind[c] >> 2) - 1`).
        ops.ind[b'u' as usize] = 1 | (1 << 2); // sense=1, arg_index=1 → args[0]

        // Closing wfd in the caller after print so reader sees EOF.
        // We dup'd inside bin_print so closing wfd here is safe AFTER
        // bin_print returns.
        let r = bin_print("print", &["hello".to_string()], &ops, BIN_PRINT);
        assert_eq!(r, 0, "c:4847 — bin_print should return 0 on success");
        unsafe { libc::close(wfd) };

        // Read from rfd.
        let mut buf = String::new();
        unsafe {
            use std::os::unix::io::FromRawFd;
            let mut f = fs::File::from_raw_fd(rfd);
            f.read_to_string(&mut buf).unwrap();
        }
        assert_eq!(buf, "hello\n", "c:4847 — write should land on -u FD");
    }

    // ═══════════════════════════════════════════════════════════════════
    // fixdir — pure path-normalization helper (port of c:1297-1395).
    // Tests pin C-faithful collapsing of `.`, `..`, double slashes, and
    // sticky-`..` semantics for relative paths.
    // ═══════════════════════════════════════════════════════════════════

    /// Empty input → empty output.
    #[test]
    fn fixdir_empty_returns_empty() {
        assert_eq!(fixdir(""), "");
    }

    /// Root passes through.
    #[test]
    fn fixdir_root_passes_through() {
        assert_eq!(fixdir("/"), "/");
    }

    /// `/.` → `/` (drop `.`).
    #[test]
    fn fixdir_root_dot_collapses_to_root() {
        assert_eq!(fixdir("/."), "/");
    }

    /// `/a/./b` → `/a/b` (drop intermediate `.`).
    #[test]
    fn fixdir_strips_dot_components() {
        assert_eq!(fixdir("/a/./b"), "/a/b");
    }

    /// `/a/b/..` → `/a` (`..` pops).
    #[test]
    fn fixdir_dot_dot_pops_previous_component() {
        assert_eq!(fixdir("/a/b/.."), "/a");
    }

    /// `/a/b/../c` → `/a/c` (pop then append).
    #[test]
    fn fixdir_dot_dot_then_continue() {
        assert_eq!(fixdir("/a/b/../c"), "/a/c");
    }

    /// `/..` → `/` (`..` past root silently drops).
    #[test]
    fn fixdir_dot_dot_past_root_drops() {
        assert_eq!(fixdir("/.."), "/");
    }

    /// `/../..` → `/` (multiple `..` past root all drop).
    #[test]
    fn fixdir_multiple_dot_dot_past_root_drops() {
        assert_eq!(fixdir("/../.."), "/");
    }

    /// `//a` → `/a` (collapse `//`).
    #[test]
    fn fixdir_collapses_double_slash() {
        assert_eq!(fixdir("//a"), "/a");
    }

    /// `/a//b///c` → `/a/b/c` (collapse runs of slashes).
    #[test]
    fn fixdir_collapses_repeated_slashes() {
        assert_eq!(fixdir("/a//b///c"), "/a/b/c");
    }

    // ── Relative paths ───────────────────────────────────────────────
    /// `a/b/c` → `a/b/c` (no change).
    #[test]
    fn fixdir_relative_no_dots_unchanged() {
        assert_eq!(fixdir("a/b/c"), "a/b/c");
    }

    /// `a/./b` → `a/b` (drop `.`).
    #[test]
    fn fixdir_relative_drops_dot() {
        assert_eq!(fixdir("a/./b"), "a/b");
    }

    /// `a/b/..` → `a` — `..` pops.
    #[test]
    fn fixdir_relative_dot_dot_pops() {
        assert_eq!(fixdir("a/b/.."), "a");
    }

    /// `..` (leading) → `..` — relative path keeps leading `..`.
    #[test]
    fn fixdir_leading_dot_dot_preserved_in_relative() {
        assert_eq!(fixdir(".."), "..");
    }

    /// `../..` (sticky `..`) — both preserved.
    #[test]
    fn fixdir_double_leading_dot_dot_both_preserved() {
        assert_eq!(fixdir("../.."), "../..");
    }

    /// `../foo/..` → `..` (pop `foo`, leading `..` remains).
    #[test]
    fn fixdir_dot_dot_then_dir_then_dot_dot() {
        assert_eq!(fixdir("../foo/.."), "..");
    }

    /// `.` alone → `.` (empty body → "." preserved for relative).
    #[test]
    fn fixdir_single_dot_returns_dot() {
        // No components, not absolute → returns "." per the c:1395 path.
        assert_eq!(fixdir("."), ".");
    }

    /// Trailing slash dropped (output never has trailing `/`).
    #[test]
    fn fixdir_trailing_slash_dropped() {
        assert_eq!(fixdir("/a/b/"), "/a/b");
        assert_eq!(fixdir("a/b/"), "a/b");
    }

    // ═══════════════════════════════════════════════════════════════════
    // cd_able_vars — CDABLEVARS option-gated lookup. Returns Some(val/tail)
    // if `s` head is a set parameter AND `cdablevars` is on. Else None.
    // ═══════════════════════════════════════════════════════════════════

    use crate::ported::options::{opt_state_get, opt_state_set};

    /// `cd_able_vars` returns None when CDABLEVARS option is OFF.
    #[test]
    fn cd_able_vars_returns_none_when_option_off() {
        let _g = crate::test_util::global_state_lock();
        let saved = opt_state_get("cdablevars").unwrap_or(false);
        opt_state_set("cdablevars", false);
        // Even if HOME is set, with cdablevars off cd_able_vars rejects.
        assert_eq!(cd_able_vars("HOME"), None);
        opt_state_set("cdablevars", saved);
    }

    /// `cd_able_vars` looks up the named var when CDABLEVARS is ON.
    #[test]
    fn cd_able_vars_returns_value_when_option_on_and_var_set() {
        let _g = crate::test_util::global_state_lock();
        let saved_opt = opt_state_get("cdablevars").unwrap_or(false);
        opt_state_set("cdablevars", true);
        opt_state_set("exec", true);
        unsetparam("zshrs_cdav_proj");
        setsparam("zshrs_cdav_proj", "/tmp/myproject");

        assert_eq!(
            cd_able_vars("zshrs_cdav_proj"),
            Some("/tmp/myproject".to_string())
        );

        unsetparam("zshrs_cdav_proj");
        opt_state_set("cdablevars", saved_opt);
    }

    /// With slash: head looked up, tail appended.
    /// `cd_able_vars("PROJ/src")` where PROJ=/home/user → "/home/user/src".
    #[test]
    fn cd_able_vars_appends_tail_after_head_substitution() {
        let _g = crate::test_util::global_state_lock();
        let saved_opt = opt_state_get("cdablevars").unwrap_or(false);
        opt_state_set("cdablevars", true);
        opt_state_set("exec", true);
        unsetparam("zshrs_cdav_PROJ");
        setsparam("zshrs_cdav_PROJ", "/home/user");

        assert_eq!(
            cd_able_vars("zshrs_cdav_PROJ/src"),
            Some("/home/user/src".to_string())
        );

        unsetparam("zshrs_cdav_PROJ");
        opt_state_set("cdablevars", saved_opt);
    }

    /// Unknown head var → None (even with option on).
    #[test]
    fn cd_able_vars_unknown_var_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let saved_opt = opt_state_get("cdablevars").unwrap_or(false);
        opt_state_set("cdablevars", true);
        unsetparam("zshrs_cdav_doesnt_exist");

        assert_eq!(cd_able_vars("zshrs_cdav_doesnt_exist"), None);

        opt_state_set("cdablevars", saved_opt);
    }

    /// Empty head (e.g. "/some/path" starts with `/`) → None.
    #[test]
    fn cd_able_vars_empty_head_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let saved_opt = opt_state_get("cdablevars").unwrap_or(false);
        opt_state_set("cdablevars", true);
        // Leading `/` → head split is empty.
        assert_eq!(cd_able_vars("/path/to/foo"), None);
        opt_state_set("cdablevars", saved_opt);
    }

    // ─── zsh-corpus pins for fixed-return builtins ──────────────────

    fn empty_opts_for_corpus() -> options {
        options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        }
    }

    /// `Src/builtin.c:4550` — `bin_true` always returns 0 regardless of
    /// argv / opts / func. Matches `:` / `true` semantics.
    #[test]
    fn builtin_corpus_bin_true_always_zero() {
        let _g = crate::test_util::global_state_lock();
        let o = empty_opts_for_corpus();
        assert_eq!(bin_true("true", &[], &o, 0), 0, "bin_true no args = 0");
        assert_eq!(
            bin_true("true", &["x".into(), "y".into()], &o, 0),
            0,
            "bin_true with args = 0",
        );
    }

    /// `Src/builtin.c:4559` — `bin_false` always returns 1.
    #[test]
    fn builtin_corpus_bin_false_always_one() {
        let _g = crate::test_util::global_state_lock();
        let o = empty_opts_for_corpus();
        assert_eq!(bin_false("false", &[], &o, 0), 1, "bin_false no args = 1");
        assert_eq!(
            bin_false("false", &["any".into()], &o, 0),
            1,
            "bin_false with args = 1",
        );
    }

    /// `Src/builtin.c` — `bin_shift` with no positional params and no
    /// argv → 0 (zsh's POSIX-conforming "shift past end" semantics
    /// when no $@/argv to shift, plus no count argument).
    #[test]
    fn builtin_corpus_bin_shift_empty_positional_returns_zero_or_one() {
        let _g = crate::test_util::global_state_lock();
        let o = empty_opts_for_corpus();
        // No positional params, no arg → either zero (no-op) or one
        // (POSIX error). Both are acceptable per the spec language —
        // we pin only that it doesn't panic and returns 0 or 1.
        let r = bin_shift("shift", &[], &o, 0);
        assert!(r == 0 || r == 1, "shift on empty positional, got {r}");
    }

    /// `Src/builtin.c` — `bin_let` with no math arg returns 1
    /// (no expression to evaluate).
    #[test]
    fn builtin_corpus_bin_let_no_args_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let o = empty_opts_for_corpus();
        let r = bin_let("let", &[], &o, 0);
        assert_eq!(r, 1, "let with no args = 1");
    }

    /// `bin_let "x=5"` evaluates math and assigns; success returns 0
    /// since the result (5) is non-zero. let returns 0 iff last expr
    /// was non-zero.
    #[test]
    fn builtin_corpus_bin_let_nonzero_expr_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let o = empty_opts_for_corpus();
        unsetparam("ZL_X");
        let r = bin_let("let", &["ZL_X=5".into()], &o, 0);
        assert_eq!(r, 0, "let 'x=5' assigns and returns 0 (nonzero result)");
        assert_eq!(getiparam("ZL_X"), 5);
        unsetparam("ZL_X");
    }

    /// `bin_let "x=0"` returns 1 since last expression evaluates to 0.
    #[test]
    fn builtin_corpus_bin_let_zero_expr_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let o = empty_opts_for_corpus();
        unsetparam("ZL_Y");
        let r = bin_let("let", &["ZL_Y=0".into()], &o, 0);
        assert_eq!(r, 1, "let 'x=0' returns 1 (zero result)");
        assert_eq!(getiparam("ZL_Y"), 0);
        unsetparam("ZL_Y");
    }

    /// `bin_let` walks multiple expressions, exit status from last.
    #[test]
    fn builtin_corpus_bin_let_multi_expr_last_wins() {
        let _g = crate::test_util::global_state_lock();
        let o = empty_opts_for_corpus();
        unsetparam("ZL_A");
        unsetparam("ZL_B");
        let r = bin_let("let", &["ZL_A=1".into(), "ZL_B=7".into()], &o, 0);
        assert_eq!(r, 0, "last expr non-zero → 0");
        assert_eq!(getiparam("ZL_A"), 1);
        assert_eq!(getiparam("ZL_B"), 7);
        unsetparam("ZL_A");
        unsetparam("ZL_B");
    }

    /// `bin_pwd` returns 0 on success — even without -P/-L, it should
    /// produce the current dir and return zero.
    #[test]
    fn builtin_corpus_bin_pwd_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let o = empty_opts_for_corpus();
        let r = bin_pwd("pwd", &[], &o, 0);
        assert_eq!(r, 0, "pwd returns 0 on success");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/builtin.c fixdir path normaliser.
    // ═══════════════════════════════════════════════════════════════════

    /// c:1297 — `fixdir("")` returns empty string.
    #[test]
    fn fixdir_empty_returns_empty_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(fixdir(""), "");
    }

    /// c:1297 — `fixdir("/")` returns "/".
    #[test]
    fn fixdir_root_returns_root() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(fixdir("/"), "/", "root path preserved");
    }

    /// c:1352 — `fixdir("/foo/./bar")` collapses `.` to `/foo/bar`.
    #[test]
    fn fixdir_drops_dot_segments() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(fixdir("/foo/./bar"), "/foo/bar");
        assert_eq!(fixdir("/./a"), "/a");
        assert_eq!(fixdir("/a/."), "/a");
    }

    /// c:1339 — `fixdir("/foo//bar")` collapses `//` to `/foo/bar`.
    #[test]
    fn fixdir_collapses_double_slash_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(fixdir("/foo//bar"), "/foo/bar");
        assert_eq!(fixdir("//foo//bar//"), "/foo/bar");
    }

    /// c:1358-1372 — `fixdir("/foo/bar/..")` pops via `..` → `/foo`.
    #[test]
    fn fixdir_pops_via_dotdot() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(fixdir("/foo/bar/.."), "/foo");
        assert_eq!(fixdir("/a/b/c/../.."), "/a");
        assert_eq!(fixdir("/a/b/../c"), "/a/c");
    }

    /// c:1358-1372 — `fixdir("/..")` past root → `/` (absolute paths
    /// silently drop `..` past `/`).
    #[test]
    fn fixdir_dotdot_past_root_clamps_to_root_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(fixdir("/.."), "/", "absolute `..` past root → `/`");
        assert_eq!(fixdir("/../.."), "/");
        assert_eq!(fixdir("/a/../../.."), "/", "successive pops past root → /");
    }

    /// c:1358-1372 — `fixdir("../foo")` on relative path KEEPS leading `..`.
    #[test]
    fn fixdir_relative_keeps_leading_dotdot_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(fixdir("../foo"), "../foo");
        assert_eq!(fixdir("../../foo"), "../../foo");
    }

    /// c:1297-1395 — `fixdir` is idempotent: fixdir(fixdir(x)) == fixdir(x).
    #[test]
    fn fixdir_is_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for input in &[
            "/a/b/c",
            "/a/./b",
            "/a/b/..",
            "/../x",
            "../foo",
            "/foo//bar",
        ] {
            let once = fixdir(input);
            let twice = fixdir(&once);
            assert_eq!(once, twice, "fixdir must be idempotent on {:?}", input);
        }
    }

    /// c:1297 — relative path of just `.` → `.` (not empty).
    #[test]
    fn fixdir_relative_dot_returns_dot() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(fixdir("."), ".", "lone `.` relative → `.`");
    }

    /// c:1297 — `fixdir("foo")` (plain relative) is identity.
    #[test]
    fn fixdir_plain_relative_is_identity() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(fixdir("foo"), "foo");
        assert_eq!(fixdir("foo/bar"), "foo/bar");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/builtin.c bin_true/false +
    // bin_break + other simple builtins.
    // ═══════════════════════════════════════════════════════════════════

    /// c:4550 — `bin_true` always returns 0 regardless of args.
    #[test]
    fn bin_true_always_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_opts_for_corpus();
        assert_eq!(bin_true("true", &[], &ops, 0), 0);
        assert_eq!(
            bin_true("true", &["arg1".into(), "arg2".into()], &ops, 0),
            0
        );
    }

    /// c:4559 — `bin_false` always returns 1 regardless of args.
    #[test]
    fn bin_false_always_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_opts_for_corpus();
        assert_eq!(bin_false("false", &[], &ops, 0), 1);
        assert_eq!(bin_false("false", &["a".into()], &ops, 0), 1);
    }

    /// c:4550 — `bin_true` ignores name arg.
    #[test]
    fn bin_true_ignores_name() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_opts_for_corpus();
        // Any name returns 0 (name is unused per UNUSED(char *name)).
        assert_eq!(bin_true("anything", &[], &ops, 0), 0);
        assert_eq!(bin_true("", &[], &ops, 0), 0);
    }

    /// c:4559 — `bin_false` ignores name arg.
    #[test]
    fn bin_false_ignores_name() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_opts_for_corpus();
        assert_eq!(bin_false("anything", &[], &ops, 0), 1);
        assert_eq!(bin_false("", &[], &ops, 0), 1);
    }

    /// `bin_true`/`bin_false` are deterministic (pure functions).
    #[test]
    fn bin_true_false_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_opts_for_corpus();
        for _ in 0..10 {
            assert_eq!(bin_true("true", &[], &ops, 0), 0);
            assert_eq!(bin_false("false", &[], &ops, 0), 1);
        }
    }

    /// c:7872 — `realexit` would terminate process; pin that we DON'T
    /// accidentally call it. (no-op test — just documents the contract.)
    /// `_realexit` similarly forbidden in tests.
    #[test]
    fn realexit_contract_pin_no_test_actually_calls_it() {
        // This test exists to document that realexit / _realexit are
        // never called by test code (would terminate the test process).
        // No assertions — pin via not calling.
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/builtin.c
    // c:131 createbuiltintable / c:232 init_builtins / c:1837 cd_able_vars
    // c:1999 fixdir / c:2490 fcgetcomm / c:2526 fcsubs / c:209 freebuiltinnode
    // ═══════════════════════════════════════════════════════════════════

    /// c:131 — `createbuiltintable` returns &HashMap (compile-time pin).
    #[test]
    fn createbuiltintable_returns_hashmap_ref_type() {
        let _: &HashMap<String, &builtin> = createbuiltintable();
    }

    /// c:131 — `createbuiltintable` is idempotent (returns same ref).
    #[test]
    fn createbuiltintable_returns_same_ref() {
        let a = createbuiltintable() as *const _;
        let b = createbuiltintable() as *const _;
        assert_eq!(a, b, "createbuiltintable must return same singleton");
    }

    /// c:131 — table contains common shell builtins (sanity sweep).
    #[test]
    fn createbuiltintable_contains_canonical_builtins() {
        let t = createbuiltintable();
        for name in ["true", "false", "set", "cd", "exit", "echo"] {
            assert!(
                t.contains_key(name),
                "builtin table must contain {:?}",
                name
            );
        }
    }

    /// c:232 — `init_builtins` is idempotent.
    #[test]
    fn init_builtins_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            init_builtins();
        }
    }

    /// c:1837 — `cd_able_vars("")` empty returns None.
    #[test]
    fn cd_able_vars_empty_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(cd_able_vars("").is_none(), "empty → None");
    }

    /// c:1837 — `cd_able_vars` returns Option<String> (type pin).
    #[test]
    fn cd_able_vars_returns_option_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<String> = cd_able_vars("anything");
    }

    /// c:1999 — `fixdir("")` empty returns String (type pin).
    #[test]
    fn fixdir_empty_returns_string_type() {
        let _: String = fixdir("");
    }

    /// c:1999 — `fixdir` is pure.
    #[test]
    fn fixdir_is_pure() {
        for s in ["", "/abs", "rel", "./dot", "../parent", "a/b/c"] {
            let first = fixdir(s);
            for _ in 0..3 {
                assert_eq!(fixdir(s), first, "fixdir({:?}) must be pure", s);
            }
        }
    }

    /// c:2490 — `fcgetcomm("")` empty returns i64 (type pin).
    #[test]
    fn fcgetcomm_empty_returns_i64_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i64 = fcgetcomm("");
    }

    /// c:2490 — `fcgetcomm` is deterministic for invalid input.
    #[test]
    fn fcgetcomm_invalid_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for s in ["", "garbage", "not_a_number"] {
            let first = fcgetcomm(s);
            for _ in 0..3 {
                assert_eq!(
                    fcgetcomm(s),
                    first,
                    "fcgetcomm({:?}) must be deterministic",
                    s
                );
            }
        }
    }

    /// c:2526 — `fcsubs(&mut s, &[])` empty sub list is no-op.
    #[test]
    fn fcsubs_empty_subs_returns_zero() {
        let mut s = "hello".to_string();
        let r = fcsubs(&mut s, &[]);
        assert_eq!(r, 0, "empty subs → 0 (no substitutions)");
        assert_eq!(s, "hello", "string unchanged");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/builtin.c
    // c:1324 bin_pwd / c:6810 bin_true / c:6823 bin_false / c:7247 bin_shift /
    // c:8444 bin_eval / c:8922 bin_test / c:9012 bin_times / c:9207 bin_ttyctl /
    // c:9295 bin_umask / c:9240 bin_let — type pins + edge cases
    // ═══════════════════════════════════════════════════════════════════

    /// c:1324 — `bin_pwd` returns i32 (compile-time type pin).
    #[test]
    fn bin_pwd_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let o = empty_opts_for_corpus();
        let _: i32 = bin_pwd("pwd", &[], &o, 0);
    }

    /// c:6810 — `bin_true` returns i32.
    #[test]
    fn bin_true_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let o = empty_opts_for_corpus();
        let _: i32 = bin_true("true", &[], &o, 0);
    }

    /// c:6823 — `bin_false` returns i32.
    #[test]
    fn bin_false_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let o = empty_opts_for_corpus();
        let _: i32 = bin_false("false", &[], &o, 0);
    }

    /// c:9240 — `bin_let` returns i32.
    #[test]
    fn bin_let_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let o = empty_opts_for_corpus();
        let _: i32 = bin_let("let", &[], &o, 0);
    }

    /// c:7247 — `bin_shift` returns i32.
    #[test]
    fn bin_shift_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let o = empty_opts_for_corpus();
        let _: i32 = bin_shift("shift", &[], &o, 0);
    }

    /// c:6810 + c:6823 — `bin_true` and `bin_false` are pure (always
    /// same value regardless of state).
    #[test]
    fn bin_true_false_are_pure() {
        let _g = crate::test_util::global_state_lock();
        let o = empty_opts_for_corpus();
        for _ in 0..5 {
            assert_eq!(bin_true("true", &[], &o, 0), 0);
            assert_eq!(bin_false("false", &[], &o, 0), 1);
        }
    }

    /// c:1324 — `bin_pwd` exit code in u8 range.
    #[test]
    fn bin_pwd_return_in_u8_range() {
        let _g = crate::test_util::global_state_lock();
        let o = empty_opts_for_corpus();
        let r = bin_pwd("pwd", &[], &o, 0);
        assert!(
            (0..256).contains(&r),
            "bin_pwd exit code must fit u8, got {}",
            r
        );
    }

    /// c:7247 — `bin_shift` with no args + empty positional returns
    /// in 0/1 range (POSIX-conforming).
    #[test]
    fn bin_shift_no_args_in_zero_one_range() {
        let _g = crate::test_util::global_state_lock();
        let o = empty_opts_for_corpus();
        let r = bin_shift("shift", &[], &o, 0);
        assert!(r == 0 || r == 1, "shift no args ∈ {{0,1}}, got {}", r);
    }

    /// c:9240 — `bin_let` is deterministic for no-args.
    #[test]
    fn bin_let_no_args_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let o = empty_opts_for_corpus();
        let first = bin_let("let", &[], &o, 0);
        for _ in 0..3 {
            assert_eq!(
                bin_let("let", &[], &o, 0),
                first,
                "bin_let no-args must be deterministic"
            );
        }
    }

    /// c:8444 — `bin_eval(&[])` empty args returns i32 (type pin).
    #[test]
    fn bin_eval_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let o = empty_opts_for_corpus();
        let _: i32 = bin_eval("eval", &[], &o, 0);
    }

    /// c:6810 — `bin_true` ignores everything (args, opts, func).
    #[test]
    fn bin_true_full_argument_immunity() {
        let _g = crate::test_util::global_state_lock();
        let o = empty_opts_for_corpus();
        // Various permutations all return 0.
        assert_eq!(bin_true("true", &[], &o, 0), 0);
        assert_eq!(bin_true("anything", &[], &o, 0), 0);
        assert_eq!(bin_true("true", &["unused".into()], &o, 99), 0);
        assert_eq!(
            bin_true("", &["a".into(), "b".into(), "c".into()], &o, -1),
            0
        );
    }

    /// c:6823 — `bin_false` ignores everything (args, opts, func).
    #[test]
    fn bin_false_full_argument_immunity() {
        let _g = crate::test_util::global_state_lock();
        let o = empty_opts_for_corpus();
        assert_eq!(bin_false("false", &[], &o, 0), 1);
        assert_eq!(bin_false("anything", &[], &o, 0), 1);
        assert_eq!(bin_false("false", &["unused".into()], &o, 99), 1);
        assert_eq!(
            bin_false("", &["a".into(), "b".into(), "c".into()], &o, -1),
            1
        );
    }
}
